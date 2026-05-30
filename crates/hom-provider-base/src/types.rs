use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::streaming::ProgressSink;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapability {
    Chat,
    Streaming,
    Tools,
    Vision,
    JsonMode,
    Local,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChatCompleteParams {
    pub provider_id: String,
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub tools: Option<Value>,
    #[serde(default)]
    pub response_format: Option<Value>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChatCompleteResult {
    pub ok: bool,
    pub latency_ms: u64,
    pub content: String,
    #[serde(default)]
    pub usage: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelsListParams {
    pub provider_id: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub owned_by: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelsListResult {
    pub models: Vec<ModelInfo>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProbeParams {
    pub provider_id: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProbeResult {
    pub ok: bool,
    pub latency_ms: u64,
    #[serde(default)]
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RateLimit {
    pub read_per_min: u32,
    pub write_per_min: u32,
}

impl Default for RateLimit {
    fn default() -> Self {
        Self {
            read_per_min: 100,
            write_per_min: 30,
        }
    }
}

#[derive(Debug, Error)]
#[error("{code}: {message}")]
pub struct ProviderError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl ProviderError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }

    pub fn missing_key(provider_id: &str) -> Self {
        Self::new(
            "missing_api_key",
            format!("{provider_id} requires a credential reference for this request"),
            false,
        )
    }

    pub fn redacted_transport(error: impl std::fmt::Display) -> Self {
        Self::new(
            "provider_transport",
            redact_secret_like(&error.to_string()),
            true,
        )
    }
}

pub fn redact_secret_like(value: &str) -> String {
    value
        .split_whitespace()
        .map(|token| {
            let lowered = token.to_ascii_lowercase();
            if lowered.contains("sk-")
                || lowered.contains("api_key")
                || lowered.contains("apikey")
                || lowered.contains("authorization")
                || token.len() > 80
            {
                "[redacted]"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl From<reqwest::Error> for ProviderError {
    fn from(error: reqwest::Error) -> Self {
        let retryable = error.is_timeout()
            || error.is_connect()
            || error.status().is_some_and(|s| s.is_server_error());
        Self::new(
            if error.is_timeout() {
                "provider_timeout"
            } else if error.is_connect() {
                "provider_connect"
            } else {
                "provider_http"
            },
            redact_secret_like(&error.to_string()),
            retryable,
        )
    }
}

#[async_trait]
pub trait Provider: Send + Sync + 'static {
    fn provider_id(&self) -> &'static str;
    fn capabilities(&self) -> Vec<ProviderCapability>;

    fn adapter_id(&self) -> &'static str {
        self.provider_id()
    }

    fn protocol_family(&self) -> &'static str {
        self.provider_id()
    }

    fn auth_kinds_supported(&self) -> Vec<&'static str> {
        vec!["api_key"]
    }

    fn permissions_required(&self) -> Vec<&'static str> {
        vec!["network", "provider_credentials"]
    }

    fn fingerprint_rules(&self) -> Value {
        json!({
            "provider_id": self.provider_id(),
            "protocol_family": self.protocol_family(),
            "models_result": "models[]",
            "chat_result": "content"
        })
    }

    fn secret_policy(&self) -> Value {
        json!({
            "stores_raw_secret": false,
            "credential_refs_only": true,
            "raw_secret_in_logs": false,
            "raw_secret_in_urls": false
        })
    }

    fn known_error_shapes(&self) -> Value {
        json!({
            "missing_auth": ["missing_api_key", "missing_oauth_token"],
            "transport": ["provider_transport", "provider_timeout", "provider_connect", "provider_http"],
            "security": ["provider_security_policy_rejected"]
        })
    }

    fn adapter_manifest(&self) -> Value {
        json!({
            "adapter_id": self.adapter_id(),
            "protocol_family": self.protocol_family(),
            "fingerprint_rules": self.fingerprint_rules(),
            "probe_method": "health.probe",
            "models_method": "models.list",
            "chat_method": "chat.complete",
            "auth_methods": self.auth_kinds_supported(),
            "secret_policy": self.secret_policy(),
            "known_error_shapes": self.known_error_shapes(),
            "model_id_extractor": "models[].id",
            "display_name_extractor": "models[].id"
        })
    }

    async fn chat_complete(
        &self,
        params: ChatCompleteParams,
        progress: Option<ProgressSink>,
    ) -> Result<ChatCompleteResult, ProviderError>;

    async fn list_models(
        &self,
        params: ModelsListParams,
    ) -> Result<ModelsListResult, ProviderError>;

    async fn health_probe(&self, params: ProbeParams) -> Result<ProbeResult, ProviderError> {
        let start = std::time::Instant::now();
        let models = self
            .list_models(ModelsListParams {
                provider_id: params.provider_id,
                api_key: params.api_key,
                credential_ref: params.credential_ref,
                base_url: params.base_url,
            })
            .await;
        match models {
            Ok(_) => Ok(ProbeResult {
                ok: true,
                latency_ms: start.elapsed().as_millis() as u64,
                error_code: None,
            }),
            Err(error) => Ok(ProbeResult {
                ok: false,
                latency_ms: start.elapsed().as_millis() as u64,
                error_code: Some(error.code),
            }),
        }
    }
}
