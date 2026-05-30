use async_trait::async_trait;
use futures_util::StreamExt;
use hom_provider_base::{
    ChatCompleteParams, ChatCompleteResult, ModelInfo, ModelsListParams, ModelsListResult,
    ProgressSink, Provider, ProviderCapability, ProviderError, http_client, join_url,
    serve_over_uds,
};
use serde_json::{Value, json};

struct LocalModelsProvider;

#[async_trait]
impl Provider for LocalModelsProvider {
    fn provider_id(&self) -> &'static str {
        "local-models"
    }

    fn adapter_id(&self) -> &'static str {
        "local-models"
    }

    fn protocol_family(&self) -> &'static str {
        "local-models"
    }

    fn auth_kinds_supported(&self) -> Vec<&'static str> {
        vec!["local_endpoint", "manual_endpoint", "approved_local_scan"]
    }

    fn permissions_required(&self) -> Vec<&'static str> {
        vec!["network"]
    }

    fn fingerprint_rules(&self) -> Value {
        json!({
            "protocols": ["ollama", "openai_compatible"],
            "models": ["/api/tags", "/models"],
            "chat": ["/api/chat", "/chat/completions"],
            "local_endpoint_required": true
        })
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability::Chat,
            ProviderCapability::Streaming,
            ProviderCapability::Tools,
            ProviderCapability::JsonMode,
            ProviderCapability::Local,
        ]
    }

    async fn chat_complete(
        &self,
        params: ChatCompleteParams,
        progress: Option<ProgressSink>,
    ) -> Result<ChatCompleteResult, ProviderError> {
        let base_url = params
            .base_url
            .clone()
            .or_else(|| std::env::var("OLLAMA_BASE_URL").ok())
            .or_else(|| std::env::var("LMSTUDIO_BASE_URL").ok())
            .unwrap_or_else(|| "http://127.0.0.1:11434".to_string());
        if is_openai_compat(&base_url) {
            chat_openai_compat(params, progress, &base_url).await
        } else {
            chat_ollama(params, progress, &base_url).await
        }
    }

    async fn list_models(
        &self,
        params: ModelsListParams,
    ) -> Result<ModelsListResult, ProviderError> {
        let base_url = params
            .base_url
            .or_else(|| std::env::var("OLLAMA_BASE_URL").ok())
            .or_else(|| std::env::var("LMSTUDIO_BASE_URL").ok())
            .unwrap_or_else(|| "http://127.0.0.1:11434".to_string());
        if is_openai_compat(&base_url) {
            let mut request = http_client()?.get(join_url(&base_url, "/models"));
            if let Some(api_key) = params.api_key {
                request = request.bearer_auth(api_key);
            }
            let value = request
                .send()
                .await?
                .error_for_status()
                .map_err(ProviderError::from)?
                .json::<Value>()
                .await?;
            let models = value["data"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|model| {
                    Some(ModelInfo {
                        id: model["id"].as_str()?.to_string(),
                        owned_by: model["owned_by"].as_str().map(ToString::to_string),
                    })
                })
                .collect();
            Ok(ModelsListResult { models })
        } else {
            let value = http_client()?
                .get(join_url(&base_url, "/api/tags"))
                .send()
                .await?
                .error_for_status()
                .map_err(ProviderError::from)?
                .json::<Value>()
                .await?;
            let models = value["models"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|model| {
                    Some(ModelInfo {
                        id: model["name"].as_str()?.to_string(),
                        owned_by: Some("ollama".to_string()),
                    })
                })
                .collect();
            Ok(ModelsListResult { models })
        }
    }
}

async fn chat_ollama(
    params: ChatCompleteParams,
    progress: Option<ProgressSink>,
    base_url: &str,
) -> Result<ChatCompleteResult, ProviderError> {
    let max_tokens = params.max_tokens;
    let mut body = json!({
        "model": params.model,
        "messages": params.messages,
        "stream": params.stream
    });
    if let Some(max_tokens) = max_tokens {
        body["options"] = json!({"num_predict": max_tokens});
    }
    let start = std::time::Instant::now();
    let response = http_client()?
        .post(join_url(base_url, "/api/chat"))
        .json(&body)
        .send()
        .await?
        .error_for_status()
        .map_err(ProviderError::from)?;
    if params.stream {
        let mut content = String::new();
        let mut seq = 0;
        let mut buffer = String::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(ProviderError::from)?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            for line in drain_lines(&mut buffer) {
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if let Some(text) = value["message"]["content"].as_str() {
                        seq += 1;
                        content.push_str(text);
                        if let Some(progress) = &progress {
                            progress.token(seq, text).await;
                        }
                    }
                    if value["done"].as_bool() == Some(true) {
                        break;
                    }
                }
            }
        }
        Ok(ChatCompleteResult {
            ok: true,
            latency_ms: start.elapsed().as_millis() as u64,
            content,
            usage: None,
        })
    } else {
        let value = response.json::<Value>().await?;
        Ok(ChatCompleteResult {
            ok: true,
            latency_ms: start.elapsed().as_millis() as u64,
            content: value["message"]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            usage: None,
        })
    }
}

async fn chat_openai_compat(
    params: ChatCompleteParams,
    progress: Option<ProgressSink>,
    base_url: &str,
) -> Result<ChatCompleteResult, ProviderError> {
    let max_tokens = params.max_tokens;
    let mut body = json!({
        "model": params.model,
        "messages": params.messages,
        "stream": params.stream
    });
    if let Some(max_tokens) = max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    let start = std::time::Instant::now();
    let mut request = http_client()?
        .post(join_url(base_url, "/chat/completions"))
        .json(&body);
    if let Some(api_key) = params.api_key {
        request = request.bearer_auth(api_key);
    }
    let response = request
        .send()
        .await?
        .error_for_status()
        .map_err(ProviderError::from)?;
    if params.stream {
        let mut content = String::new();
        let mut seq = 0;
        let mut buffer = String::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(ProviderError::from)?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            for line in drain_lines(&mut buffer) {
                let data = line
                    .strip_prefix("data:")
                    .map(str::trim)
                    .unwrap_or(line.trim());
                if data == "[DONE]" || data.is_empty() {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<Value>(data) {
                    if let Some(text) = value["choices"][0]["delta"]["content"].as_str() {
                        seq += 1;
                        content.push_str(text);
                        if let Some(progress) = &progress {
                            progress.token(seq, text).await;
                        }
                    }
                }
            }
        }
        Ok(ChatCompleteResult {
            ok: true,
            latency_ms: start.elapsed().as_millis() as u64,
            content,
            usage: None,
        })
    } else {
        let value = response.json::<Value>().await?;
        Ok(ChatCompleteResult {
            ok: true,
            latency_ms: start.elapsed().as_millis() as u64,
            content: value["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            usage: value.get("usage").cloned(),
        })
    }
}

fn is_openai_compat(base_url: &str) -> bool {
    base_url.contains("/v1") || base_url.contains("1234")
}

fn drain_lines(buffer: &mut String) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(pos) = buffer.find('\n') {
        let line = buffer[..pos].trim().to_string();
        buffer.drain(..pos + 1);
        if !line.is_empty() {
            lines.push(line);
        }
    }
    lines
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let helper_name =
        std::env::var("HOM_HELPER_NAME").unwrap_or_else(|_| "provider-local-models".to_string());
    serve_over_uds(LocalModelsProvider, &helper_name).await
}
