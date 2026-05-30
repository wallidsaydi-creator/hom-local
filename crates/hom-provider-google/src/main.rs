use async_trait::async_trait;
use futures_util::StreamExt;
use hom_provider_base::{
    ChatCompleteParams, ChatCompleteResult, ModelInfo, ModelsListParams, ModelsListResult,
    ProgressSink, Provider, ProviderCapability, ProviderError, http_client, join_url,
    parse_sse_lines, resolve_api_key, serve_over_uds,
};
use serde_json::{Value, json};

struct GoogleProvider;

#[async_trait]
impl Provider for GoogleProvider {
    fn provider_id(&self) -> &'static str {
        "google"
    }

    fn adapter_id(&self) -> &'static str {
        "google-gemini"
    }

    fn protocol_family(&self) -> &'static str {
        "google-gemini"
    }

    fn auth_kinds_supported(&self) -> Vec<&'static str> {
        vec!["api_key", "oauth"]
    }

    fn fingerprint_rules(&self) -> Value {
        json!({
            "protocol": "google_gemini",
            "models": "/models",
            "chat": "/models/{model}:generateContent",
            "stream_chat": "/models/{model}:streamGenerateContent"
        })
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability::Chat,
            ProviderCapability::Streaming,
            ProviderCapability::Tools,
            ProviderCapability::Vision,
            ProviderCapability::JsonMode,
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
            "GOOGLE_API_KEY",
        )?;
        let base_url = params
            .base_url
            .or_else(|| std::env::var("GOOGLE_BASE_URL").ok())
            .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta".to_string());
        let model = params.model.trim_start_matches("models/").to_string();
        let contents: Vec<_> = params
            .messages
            .into_iter()
            .map(|message| {
                json!({
                    "role": if message.role == "assistant" { "model" } else { "user" },
                    "parts": [{"text": message.content}]
                })
            })
            .collect();
        let body = json!({ "contents": contents });
        let path = if params.stream {
            format!("/models/{model}:streamGenerateContent?alt=sse")
        } else {
            format!("/models/{model}:generateContent")
        };
        let start = std::time::Instant::now();
        let response = http_client()?
            .post(join_url(&base_url, &path))
            .header("content-type", "application/json")
            .header("x-goog-api-key", api_key)
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
                        if let Some(text) =
                            value["candidates"][0]["content"]["parts"][0]["text"].as_str()
                        {
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
                content: value["candidates"][0]["content"]["parts"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                usage: value.get("usageMetadata").cloned(),
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
            "GOOGLE_API_KEY",
        )?;
        let base_url = params
            .base_url
            .or_else(|| std::env::var("GOOGLE_BASE_URL").ok())
            .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta".to_string());
        let value = http_client()?
            .get(join_url(&base_url, "/models"))
            .header("x-goog-api-key", api_key)
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
                    id: model["name"]
                        .as_str()?
                        .trim_start_matches("models/")
                        .to_string(),
                    owned_by: Some("google".to_string()),
                })
            })
            .collect();
        Ok(ModelsListResult { models })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let helper_name =
        std::env::var("HOM_HELPER_NAME").unwrap_or_else(|_| "provider-google".to_string());
    serve_over_uds(GoogleProvider, &helper_name).await
}
