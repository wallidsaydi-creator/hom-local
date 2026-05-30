use async_trait::async_trait;
use futures_util::StreamExt;
use hom_provider_base::{
    ChatCompleteParams, ChatCompleteResult, ModelInfo, ModelsListParams, ModelsListResult,
    ProgressSink, Provider, ProviderCapability, ProviderError, http_client, join_url,
    parse_sse_lines, resolve_api_key, serve_over_uds,
};
use serde_json::{Value, json};

struct AnthropicProvider;

#[async_trait]
impl Provider for AnthropicProvider {
    fn provider_id(&self) -> &'static str {
        "anthropic"
    }

    fn adapter_id(&self) -> &'static str {
        "anthropic-messages"
    }

    fn protocol_family(&self) -> &'static str {
        "anthropic-messages"
    }

    fn auth_kinds_supported(&self) -> Vec<&'static str> {
        vec!["api_key", "oauth"]
    }

    fn fingerprint_rules(&self) -> Value {
        json!({
            "protocol": "anthropic_messages",
            "models": "/models",
            "chat": "/messages"
        })
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability::Chat,
            ProviderCapability::Streaming,
            ProviderCapability::Tools,
            ProviderCapability::Vision,
        ]
    }

    async fn chat_complete(
        &self,
        params: ChatCompleteParams,
        progress: Option<ProgressSink>,
    ) -> Result<ChatCompleteResult, ProviderError> {
        let api_key = resolve_api_key(
            self.provider_id(),
            params.api_key.clone(),
            params.credential_ref.clone(),
            "ANTHROPIC_API_KEY",
        )?;
        let base_url = params
            .base_url
            .or_else(|| std::env::var("ANTHROPIC_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());
        let messages: Vec<_> = params
            .messages
            .into_iter()
            .filter(|message| message.role != "system")
            .map(|message| json!({"role": message.role, "content": message.content}))
            .collect();
        let body = json!({
            "model": params.model,
            "messages": messages,
            "max_tokens": 1024,
            "stream": params.stream
        });
        let start = std::time::Instant::now();
        let response = http_client()?
            .post(join_url(&base_url, "/messages"))
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?
            .error_for_status()
            .map_err(ProviderError::from)?;

        if params.stream {
            let mut content = String::new();
            let mut buffer = String::new();
            let mut seq = 0;
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(ProviderError::from)?;
                let chunk = String::from_utf8_lossy(&chunk);
                for event in parse_sse_lines(&mut buffer, &chunk) {
                    if let Ok(value) = serde_json::from_str::<Value>(&event) {
                        if value["type"] == "content_block_delta" {
                            if let Some(text) = value["delta"]["text"].as_str() {
                                seq += 1;
                                content.push_str(text);
                                if let Some(progress) = &progress {
                                    progress.token(seq, text).await;
                                }
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
            let content = value["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|block| block["text"].as_str())
                .collect::<String>();
            Ok(ChatCompleteResult {
                ok: true,
                latency_ms: start.elapsed().as_millis() as u64,
                content,
                usage: value.get("usage").cloned(),
            })
        }
    }

    async fn list_models(
        &self,
        params: ModelsListParams,
    ) -> Result<ModelsListResult, ProviderError> {
        let api_key = resolve_api_key(
            self.provider_id(),
            params.api_key.clone(),
            params.credential_ref.clone(),
            "ANTHROPIC_API_KEY",
        )?;
        let base_url = params
            .base_url
            .or_else(|| std::env::var("ANTHROPIC_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());
        let value = http_client()?
            .get(join_url(&base_url, "/models"))
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
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
                    owned_by: Some("anthropic".to_string()),
                })
            })
            .collect();
        Ok(ModelsListResult { models })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    serve_over_uds(AnthropicProvider, "provider-anthropic").await
}
