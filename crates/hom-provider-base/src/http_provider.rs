use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use std::time::Instant;

use crate::credentials::{resolve_api_key, resolve_oauth_access_token};
use crate::streaming::ProgressSink;
use crate::types::{
    ChatCompleteParams, ChatCompleteResult, ModelInfo, ModelsListParams, ModelsListResult,
    Provider, ProviderCapability, ProviderError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderDialect {
    OpenAiCompat,
    OpenAiCompatOAuth,
    Anthropic,
    Google,
    Ollama,
}

#[derive(Debug, Clone)]
pub struct HttpProviderDescriptor {
    pub family: &'static str,
    pub default_base_url: &'static str,
    pub capabilities: Vec<ProviderCapability>,
    pub dialect: ProviderDialect,
}

#[derive(Debug, Clone)]
pub struct HttpProvider {
    descriptor: HttpProviderDescriptor,
    client: reqwest::Client,
}

impl HttpProvider {
    pub fn new(descriptor: HttpProviderDescriptor) -> Self {
        Self {
            descriptor,
            client: crate::http::http_client().unwrap_or_else(|_| reqwest::Client::new()),
        }
    }
}

#[async_trait]
impl Provider for HttpProvider {
    fn provider_id(&self) -> &'static str {
        self.descriptor.family
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        self.descriptor.capabilities.clone()
    }

    async fn chat_complete(
        &self,
        params: ChatCompleteParams,
        _progress: Option<ProgressSink>,
    ) -> Result<ChatCompleteResult, ProviderError> {
        let started = Instant::now();
        let max_tokens = params.max_tokens;
        let base_url = params
            .base_url
            .as_deref()
            .unwrap_or(self.descriptor.default_base_url);
        let request = match self.descriptor.dialect {
            ProviderDialect::OpenAiCompat | ProviderDialect::OpenAiCompatOAuth => {
                let bearer_token = match self.descriptor.dialect {
                    ProviderDialect::OpenAiCompat => resolve_api_key(
                        self.provider_id(),
                        params.api_key.clone(),
                        params.credential_ref.clone(),
                        "OPENAI_API_KEY",
                    )?,
                    ProviderDialect::OpenAiCompatOAuth => resolve_oauth_access_token(
                        self.provider_id(),
                        params.credential_ref.clone(),
                    )?,
                    _ => unreachable!(),
                };
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
                if let Some(max_tokens) = max_tokens {
                    body["max_tokens"] = json!(max_tokens);
                }
                self.client
                    .post(format!("{}/chat/completions", trim_slash(base_url)))
                    .headers(bearer_headers(Some(bearer_token.as_str()))?)
                    .json(&body)
            }
            ProviderDialect::Anthropic => {
                let api_key = resolve_api_key(
                    self.provider_id(),
                    params.api_key.clone(),
                    params.credential_ref.clone(),
                    "ANTHROPIC_API_KEY",
                )?;
                let messages: Vec<_> = params
                    .messages
                    .into_iter()
                    .filter(|message| message.role != "system")
                    .map(|message| json!({"role": message.role, "content": message.content}))
                    .collect();
                let body = json!({
                    "model": params.model,
                    "messages": messages,
                    "stream": params.stream,
                    "max_tokens": max_tokens.unwrap_or(1024)
                });
                self.client
                    .post(format!("{}/messages", trim_slash(base_url)))
                    .headers(anthropic_headers(api_key.as_str())?)
                    .json(&body)
            }
            ProviderDialect::Google => {
                let api_key = resolve_api_key(
                    self.provider_id(),
                    params.api_key.clone(),
                    params.credential_ref.clone(),
                    "GOOGLE_API_KEY",
                )?;
                let model = params.model.trim_start_matches("models/");
                let body = json!({
                    "contents": params.messages.into_iter().map(|message| json!({
                        "role": if message.role == "assistant" { "model" } else { "user" },
                        "parts": [{ "text": message.content }]
                    })).collect::<Vec<Value>>(),
                    "generationConfig": {
                        "maxOutputTokens": max_tokens.unwrap_or(1024)
                    }
                });
                self.client
                    .post(format!(
                        "{}/models/{}:generateContent?key={}",
                        trim_slash(base_url),
                        model,
                        api_key
                    ))
                    .json(&body)
            }
            ProviderDialect::Ollama => {
                let mut body = json!({
                    "model": params.model,
                    "messages": params.messages,
                    "stream": params.stream
                });
                if let Some(max_tokens) = max_tokens {
                    body["options"] = json!({"num_predict": max_tokens});
                }
                self.client
                    .post(format!("{}/api/chat", trim_slash(base_url)))
                    .json(&body)
            }
        };

        let response = request
            .send()
            .await
            .map_err(ProviderError::redacted_transport)?;
        let status = response.status();
        let body = response
            .json::<Value>()
            .await
            .unwrap_or_else(|_| json!({ "status": status.as_u16() }));
        if !status.is_success() {
            return Err(ProviderError::new(
                "provider_http_error",
                format!("provider returned http {}", status.as_u16()),
                status.is_server_error(),
            ));
        }

        Ok(ChatCompleteResult {
            ok: true,
            latency_ms: started.elapsed().as_millis() as u64,
            content: extract_content(self.descriptor.dialect, &body),
            usage: body
                .get("usage")
                .or_else(|| body.get("usageMetadata"))
                .cloned(),
        })
    }

    async fn list_models(
        &self,
        params: ModelsListParams,
    ) -> Result<ModelsListResult, ProviderError> {
        let base_url = params
            .base_url
            .as_deref()
            .unwrap_or(self.descriptor.default_base_url);
        let request = match self.descriptor.dialect {
            ProviderDialect::OpenAiCompat | ProviderDialect::OpenAiCompatOAuth => {
                let bearer_token = match self.descriptor.dialect {
                    ProviderDialect::OpenAiCompat => resolve_api_key(
                        self.provider_id(),
                        params.api_key.clone(),
                        params.credential_ref.clone(),
                        "OPENAI_API_KEY",
                    )?,
                    ProviderDialect::OpenAiCompatOAuth => resolve_oauth_access_token(
                        self.provider_id(),
                        params.credential_ref.clone(),
                    )?,
                    _ => unreachable!(),
                };
                self.client
                    .get(format!("{}/models", trim_slash(base_url)))
                    .headers(bearer_headers(Some(bearer_token.as_str()))?)
            }
            ProviderDialect::Anthropic => {
                let api_key = resolve_api_key(
                    self.provider_id(),
                    params.api_key.clone(),
                    params.credential_ref.clone(),
                    "ANTHROPIC_API_KEY",
                )?;
                self.client
                    .get(format!("{}/models", trim_slash(base_url)))
                    .headers(anthropic_headers(api_key.as_str())?)
            }
            ProviderDialect::Google => {
                let api_key = resolve_api_key(
                    self.provider_id(),
                    params.api_key.clone(),
                    params.credential_ref.clone(),
                    "GOOGLE_API_KEY",
                )?;
                self.client
                    .get(format!("{}/models?key={}", trim_slash(base_url), api_key))
            }
            ProviderDialect::Ollama => self
                .client
                .get(format!("{}/api/tags", trim_slash(base_url))),
        };

        let response = request
            .send()
            .await
            .map_err(ProviderError::redacted_transport)?;
        let status = response.status();
        let body = response
            .json::<Value>()
            .await
            .unwrap_or_else(|_| json!({ "status": status.as_u16() }));
        if !status.is_success() {
            return Err(ProviderError::new(
                "provider_http_error",
                format!("provider returned http {}", status.as_u16()),
                status.is_server_error(),
            ));
        }

        Ok(ModelsListResult {
            models: extract_models(self.descriptor.dialect, &body),
        })
    }
}

fn trim_slash(value: &str) -> &str {
    value.trim_end_matches('/')
}

fn bearer_headers(api_key: Option<&str>) -> Result<HeaderMap, ProviderError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(key) = api_key.filter(|key| !key.is_empty()) {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|_| ProviderError::new("bad_api_key", "invalid api key header", false))?,
        );
    }
    Ok(headers)
}

fn anthropic_headers(api_key: &str) -> Result<HeaderMap, ProviderError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    headers.insert(
        "x-api-key",
        HeaderValue::from_str(api_key)
            .map_err(|_| ProviderError::new("bad_api_key", "invalid api key header", false))?,
    );
    Ok(headers)
}

fn extract_content(dialect: ProviderDialect, body: &Value) -> String {
    match dialect {
        ProviderDialect::OpenAiCompat | ProviderDialect::OpenAiCompatOAuth => {
            body["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        }
        ProviderDialect::Anthropic => body["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|block| block["text"].as_str())
            .collect(),
        ProviderDialect::Google => body["candidates"][0]["content"]["parts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|part| part["text"].as_str())
            .collect(),
        ProviderDialect::Ollama => body["message"]["content"]
            .as_str()
            .or_else(|| body["response"].as_str())
            .unwrap_or_default()
            .to_string(),
    }
}

fn extract_models(dialect: ProviderDialect, body: &Value) -> Vec<ModelInfo> {
    let items = match dialect {
        ProviderDialect::OpenAiCompat
        | ProviderDialect::OpenAiCompatOAuth
        | ProviderDialect::Anthropic => body["data"]
            .as_array()
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>(),
        ProviderDialect::Google => body["models"]
            .as_array()
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>(),
        ProviderDialect::Ollama => body["models"]
            .as_array()
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>(),
    };

    items
        .into_iter()
        .filter_map(|item| {
            let id = item
                .get("id")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)?
                .trim_start_matches("models/")
                .to_string();
            Some(ModelInfo {
                id,
                owned_by: item
                    .get("owned_by")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            })
        })
        .collect()
}
