use crate::{domain::GenerationSettings, inference::InferenceRuntimeManager};
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
const LLAMA_CPP_BASE: &str = "http://127.0.0.1:12345";
const MAX_MODEL_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
const MAX_STREAM_LINE_BYTES: usize = 1024 * 1024;
const MAX_STREAM_TOTAL_BYTES: usize = 64 * 1024 * 1024;
const MAX_THINKING_BYTES: usize = 256 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ChatProvider {
    Ollama,
    LmStudio,
    LlamaCpp,
}

impl ChatProvider {
    fn id(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::LmStudio => "lm-studio",
            Self::LlamaCpp => "llama-cpp",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub request_id: String,
    pub session_id: String,
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
pub struct ChatThinkingDelta {
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
    pub content: String,
}

#[derive(Default)]
struct ChatRuntimeState {
    active: HashMap<String, Arc<Notify>>,
    cancelled_before_start: HashMap<String, Instant>,
}

#[derive(Default)]
pub struct ChatRuntime {
    state: Mutex<ChatRuntimeState>,
}

impl ChatRuntime {
    pub fn begin(&self, request_id: &str) -> Result<Arc<Notify>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "The chat runtime is unavailable.".to_string())?;
        state
            .cancelled_before_start
            .retain(|_, created| created.elapsed() < Duration::from_secs(120));
        if state.active.contains_key(request_id) {
            return Err("A chat request with this identifier is already active.".into());
        }
        let signal = Arc::new(Notify::new());
        if state.cancelled_before_start.remove(request_id).is_some() {
            signal.notify_one();
        }
        state.active.insert(request_id.to_owned(), signal.clone());
        Ok(signal)
    }

    pub fn cancel(&self, request_id: &str) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if let Some(signal) = state.active.get(request_id) {
            signal.notify_one();
            return true;
        }
        state
            .cancelled_before_start
            .retain(|_, created| created.elapsed() < Duration::from_secs(120));
        if state.cancelled_before_start.len() >= 64 {
            return false;
        }
        state
            .cancelled_before_start
            .insert(request_id.to_owned(), Instant::now());
        true
    }

    pub fn finish(&self, request_id: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.active.remove(request_id);
            state.cancelled_before_start.remove(request_id);
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
    thinking: Option<String>,
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
    let mut body = Vec::with_capacity(response.content_length().unwrap_or(0).min(limit) as usize);
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

    serde_json::from_slice(&body).map_err(|_| "Local provider returned invalid JSON.".to_string())
}

async fn verify_model(
    client: &Client,
    provider: ChatProvider,
    model: &str,
    cancellation: &Notify,
) -> Result<(), String> {
    let (endpoint, list_path, list_key, id_key) = match provider {
        ChatProvider::Ollama => (OLLAMA_BASE, "/api/tags", "models", "name"),
        ChatProvider::LmStudio => (LM_STUDIO_BASE, "/v1/models", "data", "id"),
        ChatProvider::LlamaCpp => (LLAMA_CPP_BASE, "/v1/models", "data", "id"),
    };
    let response = tokio::select! {
        _ = cancellation.notified() => return Err("Generation cancelled.".into()),
        result = client.get(format!("{endpoint}{list_path}")).send() => result
            .map_err(|_| "Could not reach the selected local provider. Check that its server is running.".to_string())?,
    };
    let document = tokio::select! {
        _ = cancellation.notified() => return Err("Generation cancelled.".into()),
        result = read_bounded_json(response, MAX_MODEL_RESPONSE_BYTES) => result?,
    };
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
        "top_k": generation.top_k,
        "min_p": generation.min_p,
        "typical_p": generation.typical_p,
        "repeat_penalty": generation.repeat_penalty,
        "repeat_last_n": generation.repeat_last_n,
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

fn llama_cpp_body(request: &ChatRequest, slot_id: usize) -> Value {
    let mut body = lm_studio_body(request);
    body["id_slot"] = json!(slot_id);
    body["cache_prompt"] = json!(true);
    body
}

async fn erase_llama_slot(
    client: &Client,
    slot_id: usize,
    cancellation: &Notify,
) -> Result<(), String> {
    let response = tokio::select! {
        _ = cancellation.notified() => return Err("Generation cancelled.".into()),
        result = client
            .post(format!("{LLAMA_CPP_BASE}/slots/{slot_id}?action=erase"))
            .send() => result
                .map_err(|_| "Could not clear an evicted llama.cpp session cache slot.".to_string())?,
    };
    let result = tokio::select! {
        _ = cancellation.notified() => return Err("Generation cancelled.".into()),
        result = read_bounded_json(response, MAX_MODEL_RESPONSE_BYTES) => result?,
    };
    if result.get("id_slot").and_then(Value::as_u64) != Some(slot_id as u64)
        || result.get("n_erased").and_then(Value::as_u64).is_none()
    {
        return Err("llama.cpp returned an unexpected KV cache erase response.".into());
    }
    Ok(())
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

    let message = document.get("message");
    let delta = message
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .filter(|content| !content.is_empty())
        .map(str::to_owned);
    let thinking = message
        .and_then(|message| message.get("thinking").or_else(|| message.get("reasoning")))
        .and_then(Value::as_str)
        .filter(|content| !content.is_empty())
        .map(str::to_owned);

    Ok(Some(ParsedFrame {
        delta,
        thinking,
        done: document
            .get("done")
            .and_then(Value::as_bool)
            .unwrap_or(false),
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
    let provider_delta = choice.and_then(|choice| choice.get("delta"));
    let delta = provider_delta
        .and_then(|delta| delta.get("content"))
        .and_then(Value::as_str)
        .filter(|content| !content.is_empty())
        .map(str::to_owned);
    let thinking = provider_delta
        .and_then(|delta| {
            delta.get("reasoning_content")
                .or_else(|| delta.get("reasoning"))
                .or_else(|| delta.get("thinking"))
        })
        .and_then(Value::as_str)
        .filter(|content| !content.is_empty())
        .map(str::to_owned);
    let usage = document.get("usage");

    Ok(Some(ParsedFrame {
        delta,
        thinking,
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

fn emit_thinking(
    app: &AppHandle,
    request_id: &str,
    thinking: &str,
    emitted_bytes: &mut usize,
) -> Result<(), String> {
    let remaining = MAX_THINKING_BYTES.saturating_sub(*emitted_bytes);
    let mut end = thinking.len().min(remaining);
    while !thinking.is_char_boundary(end) {
        end -= 1;
    }
    if end == 0 {
        return Ok(());
    }
    let delta = thinking[..end].to_owned();
    *emitted_bytes += delta.len();
    app.emit(
        "chat-thinking",
        ChatThinkingDelta {
            request_id: request_id.to_owned(),
            delta,
        },
    )
    .map_err(|_| "Could not deliver provider reasoning to Spindle.".to_string())
}

async fn stream_chat(
    app: &AppHandle,
    request_id: &str,
    provider: ChatProvider,
    mut response: Response,
    cancellation: &Notify,
) -> Result<(StreamMetrics, String), String> {
    let mut buffer = Vec::<u8>::new();
    let mut total_bytes = 0usize;
    let mut metrics = StreamMetrics::default();
    let mut content = String::new();
    let mut thinking_bytes = 0usize;
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
                ChatProvider::LmStudio | ChatProvider::LlamaCpp => parse_lm_studio_line(&line)?,
            };
            if let Some(frame) = frame {
                if let Some(thinking) = frame.thinking {
                    emit_thinking(app, request_id, &thinking, &mut thinking_bytes)?;
                }
                if let Some(delta) = frame.delta {
                    content.push_str(&delta);
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
            ChatProvider::LmStudio | ChatProvider::LlamaCpp => parse_lm_studio_line(&buffer)?,
        };
        if let Some(frame) = frame {
            if let Some(thinking) = frame.thinking {
                emit_thinking(app, request_id, &thinking, &mut thinking_bytes)?;
            }
            if let Some(delta) = frame.delta {
                content.push_str(&delta);
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
            finished = frame.done;
        }
    }

    if !finished {
        return Err("The local provider stream ended before it sent a completion marker.".into());
    }

    Ok((metrics, content))
}

async fn read_error_body(mut response: Response) -> String {
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

pub async fn run(
    app: AppHandle,
    request: ChatRequest,
    cancellation: Arc<Notify>,
    inference: &InferenceRuntimeManager,
) -> Result<ChatResult, String> {
    let session_lease = if request.provider_id == ChatProvider::LlamaCpp {
        Some(
            inference
                .acquire_session(&request.session_id, &request.request_id, &cancellation)
                .await?,
        )
    } else {
        None
    };
    let started = Instant::now();
    let client = provider_client()?;
    verify_model(&client, request.provider_id, &request.model, &cancellation).await?;

    let (endpoint, body) = match request.provider_id {
        ChatProvider::Ollama => (format!("{OLLAMA_BASE}/api/chat"), ollama_body(&request)),
        ChatProvider::LmStudio => (
            format!("{LM_STUDIO_BASE}/v1/chat/completions"),
            lm_studio_body(&request),
        ),
        ChatProvider::LlamaCpp => {
            let lease = session_lease.as_ref().ok_or_else(|| {
                "The managed inference session did not receive a KV cache slot.".to_string()
            })?;
            if lease.needs_clear() {
                erase_llama_slot(&client, lease.slot_id(), &cancellation).await?;
                inference.mark_slot_cleared(&request.session_id, lease.slot_id())?;
            }
            (
                format!("{LLAMA_CPP_BASE}/v1/chat/completions"),
                llama_cpp_body(&request, lease.slot_id()),
            )
        }
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
    let (metrics, content) = stream_chat(
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
            .unwrap_or(elapsed.as_secs_f64());
        (seconds > 0.0).then_some(tokens as f64 / seconds)
    });

    Ok(ChatResult {
        provider_id: request.provider_id.id().to_owned(),
        model: request.model,
        duration_ms,
        prompt_tokens: metrics.prompt_tokens,
        output_tokens: metrics.output_tokens,
        tokens_per_second,
        content,
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

        let reasoning = parse_ollama_line(
            br#"{"message":{"thinking":"Let me reason through this."},"done":false}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(reasoning.thinking.as_deref(), Some("Let me reason through this."));

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

        let reasoning = parse_lm_studio_line(
            br#"data: {"choices":[{"delta":{"reasoning_content":"Inspecting the problem."}}]}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(reasoning.thinking.as_deref(), Some("Inspecting the problem."));

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
    fn llama_cpp_body_pins_the_session_slot_and_enables_prompt_cache_reuse() {
        let configuration = crate::domain::ModelConfiguration::new(
            "sample.gguf".into(),
            crate::domain::OptimizationProfile::Automatic,
        );
        let request = ChatRequest {
            request_id: "123e4567-e89b-12d3-a456-426614174000".into(),
            session_id: "session-one".into(),
            provider_id: ChatProvider::LlamaCpp,
            model: "sample.gguf".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hello".into(),
            }],
            context_tokens: 4096,
            generation: configuration.generation,
        };
        let body = llama_cpp_body(&request, 3);
        assert_eq!(body["id_slot"], 3);
        assert_eq!(body["cache_prompt"], true);
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn provider_identifiers_match_the_frontend_contract() {
        assert_eq!(ChatProvider::Ollama.id(), "ollama");
        assert_eq!(ChatProvider::LmStudio.id(), "lm-studio");
        assert_eq!(ChatProvider::LlamaCpp.id(), "llama-cpp");
    }
}
