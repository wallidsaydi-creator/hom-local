use async_trait::async_trait;
use futures_util::StreamExt;
use hom_provider_base::{
    ChatCompleteParams, ChatCompleteResult, ModelInfo, ModelsListParams, ModelsListResult,
    ProgressSink, Provider, ProviderCapability, ProviderError, bearer_headers, http_client,
    join_url, parse_sse_lines, resolve_oauth_access_token, serve_over_uds,
};
use serde_json::{Value, json};

struct CodexOAuthProvider;

#[async_trait]
impl Provider for CodexOAuthProvider {
    fn provider_id(&self) -> &'static str {
        "codex-oauth"
    }

    fn adapter_id(&self) -> &'static str {
        "codex-oauth"
    }

    fn protocol_family(&self) -> &'static str {
        "openai-compatible-oauth"
    }

    fn auth_kinds_supported(&self) -> Vec<&'static str> {
        vec!["oauth"]
    }

    fn fingerprint_rules(&self) -> Value {
        json!({
            "protocol": "openai_compatible_oauth",
            "models": "/models",
            "chat": "/chat/completions",
            "credential_kind": "oauth"
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
        let access_token =
            resolve_oauth_access_token(self.provider_id(), params.credential_ref.clone())?;
        let base_url = codex_base_url(params.base_url);
        let client = http_client()?;
        let mut body = json!({
            "model": params.model,
            "messages": params.messages,
            "stream": params.stream
        });
        if let Some(tools) = params.tools {
            body["tools"] = tools;
        }
        if let Some(response_format) = params.response_format {
            body["response_format"] = response_format;
        }

        let start = std::time::Instant::now();
        let response = client
            .post(join_url(&base_url, "/chat/completions"))
            .headers(bearer_headers(&access_token)?)
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
                let chunk = String::from_utf8_lossy(&chunk);
                for event in parse_sse_lines(&mut buffer, &chunk) {
                    if event == "[DONE]" {
                        break;
                    }
                    if let Ok(value) = serde_json::from_str::<Value>(&event) {
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

    async fn list_models(
        &self,
        params: ModelsListParams,
    ) -> Result<ModelsListResult, ProviderError> {
        let access_token =
            resolve_oauth_access_token(self.provider_id(), params.credential_ref.clone())?;
        let base_url = codex_base_url(params.base_url);
        let value = http_client()?
            .get(join_url(&base_url, "/models"))
            .headers(bearer_headers(&access_token)?)
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
    }
}

fn codex_base_url(base_url: Option<String>) -> String {
    base_url
        .or_else(|| std::env::var("CODEX_OAUTH_BASE_URL").ok())
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    serve_over_uds(CodexOAuthProvider, "provider-codex-oauth").await
}
