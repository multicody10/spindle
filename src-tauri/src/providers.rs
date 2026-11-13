use serde::Serialize;
use serde_json::Value;
use std::time::{Duration, Instant};

const MAX_MODELS: usize = 100;
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalProviderModel {
    pub id: String,
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalProviderStatus {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    pub available: bool,
    pub latency_ms: Option<u64>,
    pub models: Vec<LocalProviderModel>,
    pub message: String,
}

async fn get_json(client: &reqwest::Client, endpoint: &str) -> Result<Value, String> {
    let response = client.get(endpoint).send().await.map_err(|error| {
        if error.is_timeout() {
            "Connection timed out. Check that the local provider is running.".to_string()
        } else {
            "Could not reach the local provider. Check that it is running.".to_string()
        }
    })?;

    if !response.status().is_success() {
        return Err(format!(
            "Local provider returned HTTP {}.",
            response.status().as_u16()
        ));
    }

    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES)
    {
        return Err("Local provider response exceeded the 1 MiB discovery limit.".into());
    }

    let mut response = response;
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or(0)
            .min(MAX_RESPONSE_BYTES) as usize,
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read the local provider response.".to_string())?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES as usize {
            return Err("Local provider response exceeded the 1 MiB discovery limit.".into());
        }
        body.extend_from_slice(&chunk);
    }

    serde_json::from_slice(&body).map_err(|_| "Local provider returned invalid JSON.".to_string())
}

fn bounded_string(value: Option<&Value>, max_length: usize) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= max_length)
        .map(str::to_owned)
}

fn parse_ollama_models(document: &Value) -> Result<Vec<LocalProviderModel>, String> {
    let entries = document
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| "Ollama returned an unexpected model list.".to_string())?;

    Ok(entries
        .iter()
        .take(MAX_MODELS)
        .filter_map(|entry| {
            let id = bounded_string(entry.get("name").or_else(|| entry.get("model")), 256)?;
            let details = entry.get("details");
            Some(LocalProviderModel {
                id,
                parameter_size: bounded_string(
                    details.and_then(|value| value.get("parameter_size")),
                    64,
                ),
                quantization: bounded_string(
                    details.and_then(|value| value.get("quantization_level")),
                    64,
                ),
            })
        })
        .collect())
}

fn parse_openai_models(document: &Value) -> Result<Vec<LocalProviderModel>, String> {
    let entries = document
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "LM Studio returned an unexpected model list.".to_string())?;

    Ok(entries
        .iter()
        .take(MAX_MODELS)
        .filter_map(|entry| {
            Some(LocalProviderModel {
                id: bounded_string(entry.get("id"), 256)?,
                parameter_size: None,
                quantization: None,
            })
        })
        .collect())
}

async fn probe(
    client: &reqwest::Client,
    id: &str,
    name: &str,
    base_url: &str,
    path: &str,
    parse_models: fn(&Value) -> Result<Vec<LocalProviderModel>, String>,
) -> LocalProviderStatus {
    let started = Instant::now();
    let endpoint = format!("{base_url}{path}");
    let result = get_json(client, &endpoint)
        .await
        .and_then(|document| parse_models(&document));
    let latency_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;

    match result {
        Ok(models) => {
            let count = models.len();
            LocalProviderStatus {
                id: id.into(),
                name: name.into(),
                endpoint: base_url.into(),
                available: true,
                latency_ms: Some(latency_ms),
                models,
                message: if count == 0 {
                    "Connected. No models are currently available.".into()
                } else {
                    format!("Connected. {count} model(s) available.")
                },
            }
        }
        Err(message) => LocalProviderStatus {
            id: id.into(),
            name: name.into(),
            endpoint: base_url.into(),
            available: false,
            latency_ms: None,
            models: Vec::new(),
            message,
        },
    }
}

pub async fn discover() -> Result<Vec<LocalProviderStatus>, String> {
    // Discovery is intentionally restricted to fixed loopback endpoints. No user URL,
    // proxy, authentication token, or redirect can turn this into an external request.
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(750))
        .timeout(Duration::from_secs(3))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not initialize the local provider discovery client.".to_string())?;

    let (ollama, lm_studio) = tokio::join!(
        probe(
            &client,
            "ollama",
            "Ollama",
            "http://127.0.0.1:11434",
            "/api/tags",
            parse_ollama_models,
        ),
        probe(
            &client,
            "lm-studio",
            "LM Studio",
            "http://127.0.0.1:1234",
            "/v1/models",
            parse_openai_models,
        )
    );

    Ok(vec![ollama, lm_studio])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ollama_model_metadata() {
        let document = serde_json::json!({
            "models": [{
                "name": "qwen3.5:9b",
                "details": {
                    "parameter_size": "9.1B",
                    "quantization_level": "Q4_K_M"
                }
            }]
        });
        let models = parse_ollama_models(&document).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "qwen3.5:9b");
        assert_eq!(models[0].parameter_size.as_deref(), Some("9.1B"));
        assert_eq!(models[0].quantization.as_deref(), Some("Q4_K_M"));
    }

    #[test]
    fn parses_openai_compatible_model_ids() {
        let document = serde_json::json!({
            "data": [
                {"id": "local-model", "object": "model"},
                {"id": "another-model", "object": "model"}
            ]
        });
        let models = parse_openai_models(&document).unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["local-model", "another-model"]
        );
    }

    #[test]
    fn rejects_unexpected_provider_payloads() {
        assert!(parse_ollama_models(&serde_json::json!({"items": []})).is_err());
        assert!(parse_openai_models(&serde_json::json!({"models": []})).is_err());
    }

    #[test]
    fn bounds_untrusted_model_names() {
        let document = serde_json::json!({
            "data": [{"id": "x".repeat(257)}]
        });
        assert!(parse_openai_models(&document).unwrap().is_empty());
    }
}
