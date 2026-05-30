use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use hom_shared::{
    JsonRpcError, build_request, ensure_helper_identity, hom_local_dir, socket_path, uds,
};
use rand_core::{OsRng, RngCore};
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct BrainClient {
    socket_path: PathBuf,
    client_id: String,
    client_pub: String,
    private_key: String,
}

#[derive(Debug, Error)]
pub enum BrainClientError {
    #[error("brain socket connect failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("brain returned JSON-RPC error {0}: {1}")]
    Rpc(i64, String, Option<Value>),
    #[error("request signing failed: {0}")]
    Sign(String),
}

impl BrainClient {
    pub fn from_env() -> anyhow::Result<Self> {
        let hom_dir = hom_local_dir();
        let keys = ensure_helper_identity(
            &hom_dir,
            "ingress",
            "ingress.local",
            &[
                "system",
                "memory:save",
                "memory:recall",
                "memory:open",
                "memory:answer",
                "events:read",
                "events:write",
                "diagnostics:read",
                "security:read",
                "gates:read",
                "gates:write",
            ],
        )?;
        let socket_path = std::env::var("HOM_BRAIN_SOCK")
            .or_else(|_| std::env::var("HOM_BRAIN_SOCKET"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| socket_path(hom_dir, "brain"));
        Ok(Self {
            socket_path,
            client_id: std::env::var("HOM_INGRESS_CLIENT_ID")
                .unwrap_or_else(|_| "ingress.local".to_string()),
            client_pub: std::env::var("HOM_INGRESS_PUBLIC_KEY").unwrap_or(keys.public_key),
            private_key: std::env::var("HOM_INGRESS_PRIVATE_KEY").unwrap_or(keys.private_key),
        })
    }

    pub fn new_for_test(socket_path: PathBuf, client_pub: String, private_key: String) -> Self {
        Self {
            socket_path,
            client_id: "ingress.local".to_string(),
            client_pub,
            private_key,
        }
    }

    pub async fn call(
        &self,
        method: &str,
        params: Value,
        scope: &str,
    ) -> Result<Value, BrainClientError> {
        let request = build_request(
            json!(next_nonce()),
            method,
            params,
            self.client_id.clone(),
            self.client_pub.clone(),
            &self.private_key,
            vec![scope.to_string(), "system".to_string()],
            next_nonce(),
            now_s(),
        )
        .map_err(|error| BrainClientError::Sign(error.to_string()))?;
        let response = uds::send_request(&self.socket_path, &request).await?;
        match (response.result, response.error) {
            (Some(result), None) => Ok(result),
            (
                _,
                Some(JsonRpcError {
                    code,
                    message,
                    data,
                }),
            ) => Err(BrainClientError::Rpc(code, message, data)),
            _ => Err(BrainClientError::Rpc(
                -32603,
                "empty brain response".to_string(),
                None,
            )),
        }
    }
}

impl BrainClientError {
    pub fn error_payload(&self) -> Value {
        match self {
            Self::Io(error) => json!({
                "code": -32050,
                "message": "brain_socket_connect_failed",
                "data": {"error": error.to_string()}
            }),
            Self::Rpc(code, message, data) => {
                let mut payload = json!({
                    "code": code,
                    "message": message
                });
                if let Some(data) = data {
                    payload["data"] = data.clone();
                }
                payload
            }
            Self::Sign(message) => json!({
                "code": -32603,
                "message": "request_signing_failed",
                "data": {"error": message}
            }),
        }
    }
}

fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn next_nonce() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hom_shared::sha256_b64u(bytes)
}
