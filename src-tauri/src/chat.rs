use crate::domain::GenerationSettings;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

const OLLAMA_BASE: &str = "http://127.0.0.1:11434";
const LM_STUDIO_BASE: &str = "http://127.0.0.1:1234";
const MAX_MODEL_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
const MAX_STREAM_LINE_BYTES: usize = 1024 * 1024;
const MAX_STREAM_TOTAL_BYTES: usize = 64 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ChatProvider {
    Ollama,
    LmStudio,
}

impl ChatProvider {
    fn id(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::LmStudio => "lm-studio",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub request_id: String,
    pub provider_id: ChatProvider,
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub context_tokens: usize,
    pub generation: GenerationSettings,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatDelta {
    pub request_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatResult {
    pub provider_id: String,
    pub model: String,
    pub duration_ms: u64,
    pub prompt_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub tokens_per_second: Option<f64>,
}

#[derive(Default)]
pub struct ChatRuntime {
    active: Mutex<HashMap<String, Arc<Notify>>>,
}

impl ChatRuntime {
    pub fn begin(&self, request_id: &str) -> Result<Arc<Notify>, String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "The chat runtime is unavailable.".to_string())?;
        if active.contains_key(request_id) {
            return Err("A chat request with this identifier is already active.".into());
        }
        let signal = Arc::new(Notify::new());
        active.insert(request_id.to_owned(), signal.clone());
        Ok(signal)
    }

    pub fn cancel(&self, request_id: &str) -> bool {
        self.active
            .lock()
            .ok()
            .and_then(|active| active.get(request_id).cloned())
            .map(|signal| {
                signal.notify_one();
                true
            })
            .unwrap_or(false)
    }

    pub fn finish(&self, request_id: &str) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(request_id);
        }
    }
}

#[derive(Debug, Default)]
struct StreamMetrics {
    prompt_tokens: Option<u64>,
    output_tokens: Option<u64>,
    evaluation_duration_ns: Option<u64>,
}

#[derive(Debug, Default)]
struct ParsedFrame {
    delta: Option<String>,
    done: bool,
    metrics: StreamMetrics,
}

fn provider_client() -> Result<Client, String> {
    Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(REQUEST_TIMEOUT)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not initialize the local inference client.".to_string())
}

async fn read_bounded_json(response: Response, limit: u64) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err(format!(
            "Local provider returned HTTP {}.",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err("Local provider response exceeded the configured size limit.".into());
    }

    let mut response = response;
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or(0)
            .min(limit) as usize,
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read the local provider response.".to_string())?
    {
        if body.len().saturating_add(chunk.len()) > limit as usize {
            return Err("Local provider response exceeded the configured size limit.".into());
        }
        body.extend_from_slice(&chunk);
    }

    serde_json::from_slice(&body)
        .map_err(|_| "Local provider returned invalid JSON.".to_string())
}

async fn verify_model(
    client: &Client,
    provider: ChatProvider,
    model: &str,
) -> Result<(), String> {
    let (endpoint, list_path, list_key, id_key) = match provider {
        ChatProvider::Ollama => (OLLAMA_BASE, "/api/tags", "models", "name"),
        ChatProvider::LmStudio => (LM_STUDIO_BASE, "/v1/models", "data", "id"),
    };
    let response = client
        .get(format!("{endpoint}{list_path}"))
        .send()
        .await
        .map_err(|_| "Could not reach the selected local provider. Check that its server is running.".to_string())?;
    let document = read_bounded_json(response, MAX_MODEL_RESPONSE_BYTES).await?;
    let models = document
        .get(list_key)
        .and_then(Value::as_array)
        .ok_or_else(|| "The selected provider returned an unexpected model list.".to_string())?;

    if !models.iter().any(|entry| {
        entry
            .get(id_key)
            .and_then(Value::as_str)
            .is_some_and(|id| id == model)
    }) {
        return Err(format!(
            "Model '{model}' is not currently listed by {}. Refresh provider discovery and select an available model.",
            provider.id()
        ));
    }
    Ok(())
}

fn ollama_body(request: &ChatRequest) -> Value {
    let generation = &request.generation;
    let mut options = json!({
        "num_ctx": request.context_tokens,
        "num_predict": generation.max_output_tokens,
        "temperature": generation.temperature,
        "top_p": generation.top_p,
        "top_k": generation.top_k,
        "min_p": generation.min_p,
        "typical_p": generation.typical_p,
        "repeat_penalty": generation.repeat_penalty,
        "repeat_last_n": generation.repeat_last_n,
        "presence_penalty": generation.presence_penalty,
        "frequency_penalty": generation.frequency_penalty,
        "stop": generation.stop_sequences,
    });
    if let Some(seed) = generation.seed {
        options["seed"] = json!(seed);
    }
    json!({
        "model": request.model,
        "messages": request.messages,
        "stream": true,
        "options": options,
    })
}

fn lm_studio_body(request: &ChatRequest) -> Value {
    let generation = &request.generation;
    let mut body = json!({
        "model": request.model,
        "messages": request.messages,
        "stream": true,
        "stream_options": {"include_usage": true},
        "temperature": generation.temperature,
        "top_p": generation.top_p,
        "max_tokens": generation.max_output_tokens,
        "presence_penalty": generation.presence_penalty,
        "frequency_penalty": generation.frequency_penalty,
        "stop": generation.stop_sequences,
    });
    if let Some(seed) = generation.seed {
        body["seed"] = json!(seed);
    }
    body
}

fn bounded_provider_error(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(500).collect())
}

fn parse_ollama_line(line: &[u8]) -> Result<Option<ParsedFrame>, String> {
    let line = std::str::from_utf8(line)
        .map_err(|_| "Ollama returned a non-UTF-8 stream frame.".to_string())?
        .trim();
    if line.is_empty() {
        return Ok(None);
    }
    let document: Value = serde_json::from_str(line)
        .map_err(|_| "Ollama returned an invalid streaming JSON frame.".to_string())?;
    if let Some(message) = bounded_provider_error(document.get("error")) {
        return Err(format!("Ollama: {message}"));
    }

    let delta = document
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .filter(|content| !content.is_empty())
        .map(str::to_owned);

    Ok(Some(ParsedFrame {
        delta,
        done: document.get("done").and_then(Value::as_bool).unwrap_or(false),
        metrics: StreamMetrics {
            prompt_tokens: document.get("prompt_eval_count").and_then(Value::as_u64),
            output_tokens: document.get("eval_count").and_then(Value::as_u64),
            evaluation_duration_ns: document.get("eval_duration").and_then(Value::as_u64),
        },
    }))
}

fn parse_lm_studio_line(line: &[u8]) -> Result<Option<ParsedFrame>, String> {
    let line = std::str::from_utf8(line)
        .map_err(|_| "LM Studio returned a non-UTF-8 stream frame.".to_string())?
        .trim();
    if !line.starts_with("data:") {
        return Ok(None);
    }

    let data = line["data:".len()..].trim();
    if data.is_empty() {
        return Ok(None);
    }
    if data == "[DONE]" {
        return Ok(Some(ParsedFrame {
            done: true,
            ..ParsedFrame::default()
        }));
    }

    let document: Value = serde_json::from_str(data)
        .map_err(|_| "LM Studio returned an invalid SSE data frame.".to_string())?;
    if let Some(message) = bounded_provider_error(document.get("error")) {
        return Err(format!("LM Studio: {message}"));
    }

    let choice = document
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first());
    let delta = choice
        .and_then(|choice| choice.get("delta"))
        .and_then(|delta| delta.get("content"))
        .and_then(Value::as_str)
        .filter(|content| !content.is_empty())
        .map(str::to_owned);
    let usage = document.get("usage");

    Ok(Some(ParsedFrame {
        delta,
        done: false,
        metrics: StreamMetrics {
            prompt_tokens: usage
                .and_then(|usage| usage.get("prompt_tokens"))
                .and_then(Value::as_u64),
            output_tokens: usage
                .and_then(|usage| usage.get("completion_tokens"))
                .and_then(Value::as_u64),
            evaluation_duration_ns: None,
        },
    }))
}

async fn stream_chat(
    app: &AppHandle,
    request_id: &str,
    provider: ChatProvider,
    mut response: Response,
    cancellation: &Notify,
) -> Result<StreamMetrics, String> {
    let mut buffer = Vec::<u8>::new();
    let mut total_bytes = 0usize;
    let mut metrics = StreamMetrics::default();
    let mut finished = false;

    loop {
        let chunk = tokio::select! {
            _ = cancellation.notified() => return Err("Generation cancelled.".into()),
            result = response.chunk() => result
                .map_err(|_| "The local provider stream ended unexpectedly.".to_string())?,
        };
        let Some(chunk) = chunk else {
            break;
        };

        total_bytes = total_bytes.saturating_add(chunk.len());
        if total_bytes > MAX_STREAM_TOTAL_BYTES {
            return Err("The inference response exceeded the 64 MiB output limit.".into());
        }
        buffer.extend_from_slice(&chunk);

        while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = buffer.drain(..=newline).collect();
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.len() > MAX_STREAM_LINE_BYTES {
                return Err("The local provider sent an oversized stream frame.".into());
            }

            let frame = match provider {
                ChatProvider::Ollama => parse_ollama_line(&line)?,
                ChatProvider::LmStudio => parse_lm_studio_line(&line)?,
            };
            if let Some(frame) = frame {
                if let Some(delta) = frame.delta {
                    app.emit(
                        "chat-delta",
                        ChatDelta {
                            request_id: request_id.to_owned(),
                            delta,
                        },
                    )
                    .map_err(|_| "Could not deliver the model response to Spindle.".to_string())?;
                }
                if frame.metrics.prompt_tokens.is_some() {
                    metrics.prompt_tokens = frame.metrics.prompt_tokens;
                }
                if frame.metrics.output_tokens.is_some() {
                    metrics.output_tokens = frame.metrics.output_tokens;
                }
                if frame.metrics.evaluation_duration_ns.is_some() {
                    metrics.evaluation_duration_ns = frame.metrics.evaluation_duration_ns;
                }
                if frame.done {
                    finished = true;
                }
            }
        }

        if buffer.len() > MAX_STREAM_LINE_BYTES {
            return Err("The local provider sent an oversized stream frame.".into());
        }
        if finished {
            break;
        }
    }

    if !buffer.is_empty() && !finished {
        let frame = match provider {
            ChatProvider::Ollama => parse_ollama_line(&buffer)?,
            ChatProvider::LmStudio => parse_lm_studio_line(&buffer)?,
        };
        if let Some(frame) = frame {
            if let Some(delta) = frame.delta {
                app.emit(
                    "chat-delta",
                    ChatDelta {
                        request_id: request_id.to_owned(),
                        delta,
                    },
                )
                .map_err(|_| "Could not deliver the model response to Spindle.".to_string())?;
            }
            if frame.metrics.prompt_tokens.is_some() {
                metrics.prompt_tokens = frame.metrics.prompt_tokens;
            }
            if frame.metrics.output_tokens.is_some() {
                metrics.output_tokens = frame.metrics.output_tokens;
            }
            if frame.metrics.evaluation_duration_ns.is_some() {
                metrics.evaluation_duration_ns = frame.metrics.evaluation_duration_ns;
            }
        }
    }

    Ok(metrics)
}

fn read_error_body(response: Response) -> impl std::future::Future<Output = String> {
    async move {
        let mut response = response;
        let mut body = Vec::new();
        while let Ok(Some(chunk)) = response.chunk().await {
            let remaining = MAX_ERROR_BODY_BYTES.saturating_sub(body.len());
            if remaining == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            if body.len() >= MAX_ERROR_BODY_BYTES {
                break;
            }
        }
        let text = String::from_utf8_lossy(&body);
        text.chars().take(500).collect()
    }
}

pub async fn run(
    app: AppHandle,
    request: ChatRequest,
    cancellation: Arc<Notify>,
) -> Result<ChatResult, String> {
    let started = Instant::now();
    let client = provider_client()?;
    verify_model(&client, request.provider_id, &request.model).await?;

    let (endpoint, body) = match request.provider_id {
        ChatProvider::Ollama => (format!("{OLLAMA_BASE}/api/chat"), ollama_body(&request)),
        ChatProvider::LmStudio => (
            format!("{LM_STUDIO_BASE}/v1/chat/completions"),
            lm_studio_body(&request),
        ),
    };

    let response = tokio::select! {
        _ = cancellation.notified() => return Err("Generation cancelled.".into()),
        result = client.post(endpoint).json(&body).send() => result
            .map_err(|_| "The selected local provider could not start the chat request.".to_string())?,
    };

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = read_error_body(response).await;
        return Err(if body.is_empty() {
            format!("Local provider returned HTTP {status}.")
        } else {
            format!("Local provider returned HTTP {status}: {body}")
        });
    }

    let request_id = request.request_id.clone();
    let metrics = stream_chat(
        &app,
        &request_id,
        request.provider_id,
        response,
        &cancellation,
    )
    .await?;

    let elapsed = started.elapsed();
    let duration_ms = elapsed.as_millis().min(u64::MAX as u128) as u64;
    let tokens_per_second = metrics.output_tokens.and_then(|tokens| {
        let seconds = metrics
            .evaluation_duration_ns
            .map(|duration| duration as f64 / 1_000_000_000.0)
            .filter(|duration| *duration > 0.0)
            .unwrap_or_else(|| elapsed.as_secs_f64());
        (seconds > 0.0).then_some(tokens as f64 / seconds)
    });

    Ok(ChatResult {
        provider_id: request.provider_id.id().to_owned(),
        model: request.model,
        duration_ms,
        prompt_tokens: metrics.prompt_tokens,
        output_tokens: metrics.output_tokens,
        tokens_per_second,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ollama_token_and_final_metrics_frames() {
        let token = parse_ollama_line(
            br#"{"message":{"role":"assistant","content":"Hello"},"done":false}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(token.delta.as_deref(), Some("Hello"));
        assert!(!token.done);

        let final_frame = parse_ollama_line(
            br#"{"done":true,"prompt_eval_count":12,"eval_count":8,"eval_duration":2000000000}"#,
        )
        .unwrap()
        .unwrap();
        assert!(final_frame.done);
        assert_eq!(final_frame.metrics.prompt_tokens, Some(12));
        assert_eq!(final_frame.metrics.output_tokens, Some(8));
        assert_eq!(
            final_frame.metrics.evaluation_duration_ns,
            Some(2_000_000_000)
        );
    }

    #[test]
    fn parses_lm_studio_sse_content_and_usage() {
        let token = parse_lm_studio_line(
            br#"data: {"choices":[{"delta":{"role":"assistant","content":"world"}}]}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(token.delta.as_deref(), Some("world"));

        let usage = parse_lm_studio_line(
            br#"data: {"choices":[],"usage":{"prompt_tokens":21,"completion_tokens":13}}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(usage.metrics.prompt_tokens, Some(21));
        assert_eq!(usage.metrics.output_tokens, Some(13));

        let done = parse_lm_studio_line(b"data: [DONE]").unwrap().unwrap();
        assert!(done.done);
    }

    #[test]
    fn ignores_sse_metadata_lines_and_blank_lines() {
        assert!(parse_lm_studio_line(b"event: message").unwrap().is_none());
        assert!(parse_lm_studio_line(b"").unwrap().is_none());
    }

    #[test]
    fn rejects_malformed_stream_frames_and_provider_errors() {
        assert!(parse_ollama_line(b"{").is_err());
        assert!(parse_lm_studio_line(b"data: {").is_err());
        assert!(parse_ollama_line(br#"{"error":"model not loaded"}"#).is_err());
        assert!(parse_lm_studio_line(br#"data: {"error":"server failed"}"#).is_err());
    }

    #[test]
    fn provider_identifiers_match_the_frontend_contract() {
        assert_eq!(ChatProvider::Ollama.id(), "ollama");
        assert_eq!(ChatProvider::LmStudio.id(), "lm-studio");
    }
}
