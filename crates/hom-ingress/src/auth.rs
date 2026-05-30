use std::sync::Arc;

use axum::http::StatusCode;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::Value;

use crate::http::AppState;

/// Identity extracted from X-HOM-* headers for macOS app requests.
#[derive(Debug, Clone)]
pub struct UiIdentity {
    pub client_id: String,
    pub verified: bool,
}

/// Verifies X-HOM-* signature headers for `/api/ui/*` requests.
///
/// The macOS app signs requests using Ed25519 (CryptoKit Curve25519.Signing):
/// - `X-HOM-Signature`: base64-encoded Ed25519 signature
/// - `X-HOM-Nonce`: UUID string
/// - `X-HOM-Timestamp`: Unix epoch seconds
///
/// The signing payload is: `"METHOD:PATH:nonce:timestamp:canonical_body"`
/// where canonical_body is the sorted-keys JSON serialization of the request body.
///
/// For local-only access (127.0.0.1), unsigned GET requests are allowed.
/// All POST requests require a valid signature from a known client or the owner key.
pub async fn verify_ui_request(
    state: &Arc<AppState>,
    method: &str,
    path: &str,
    headers: &axum::http::HeaderMap,
    body: &Value,
) -> Result<UiIdentity, (StatusCode, Value)> {
    let signature = match headers.get("X-HOM-Signature").and_then(|v| v.to_str().ok()) {
        Some(sig) => sig.to_string(),
        None => {
            return Ok(UiIdentity {
                client_id: "anonymous.local".to_string(),
                verified: false,
            });
        }
    };

    let nonce = headers
        .get("X-HOM-Nonce")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let timestamp: i64 = headers
        .get("X-HOM-Timestamp")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let canonical_body = canonicalize_body(body);
    let payload = format!("{method}:{path}:{nonce}:{timestamp}:{canonical_body}");

    // Check clock skew (allow 5 minutes for local requests)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let skew = (now - timestamp).abs();
    if skew > 300 {
        return Err((
            StatusCode::UNAUTHORIZED,
            serde_json::json!({
                "ok": false,
                "error": {"code": -32004, "message": "clock_skew"}
            }),
        ));
    }

    // Try the owner key first (most common path for macOS app)
    if let Some(identity) = try_verify_owner(&payload, &signature)? {
        return Ok(identity);
    }

    // Then try known clients
    if let Some(identity) = try_verify_known_clients(state, &payload, &signature).await? {
        return Ok(identity);
    }

    Err((
        StatusCode::UNAUTHORIZED,
        serde_json::json!({
            "ok": false,
            "error": {"code": -32002, "message": "signature_invalid"}
        }),
    ))
}

fn try_verify_owner(
    payload: &str,
    signature: &str,
) -> Result<Option<UiIdentity>, (StatusCode, Value)> {
    let hom_dir = hom_shared::hom_local_dir();
    let owner_key_path = hom_dir.join("keys").join("owner.key");
    if !owner_key_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&owner_key_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"ok": false, "error": {"code": -32603, "message": format!("owner_key_read: {e}")}}),
        )
    })?;

    let owner_pub = if content.starts_with('{') {
        serde_json::from_str::<Value>(&content).ok().and_then(|v| {
            v.get("public_key")
                .and_then(|p| p.as_str().map(String::from))
        })
    } else {
        Some(content.trim().to_string())
    };

    if let Some(pub_key) = owner_pub {
        let sig_bytes = match base64_decode(signature) {
            Ok(bytes) => bytes,
            Err(_) => match base64url_decode(signature) {
                Ok(bytes) => bytes,
                Err(_) => return Ok(None),
            },
        };

        if verify_ed25519_signature(&pub_key, payload.as_bytes(), &sig_bytes) {
            return Ok(Some(UiIdentity {
                client_id: "owner.local".to_string(),
                verified: true,
            }));
        }
    }

    Ok(None)
}

async fn try_verify_known_clients(
    state: &Arc<AppState>,
    payload: &str,
    signature: &str,
) -> Result<Option<UiIdentity>, (StatusCode, Value)> {
    let sig_bytes = match base64_decode(signature) {
        Ok(bytes) => bytes,
        Err(_) => match base64url_decode(signature) {
            Ok(bytes) => bytes,
            Err(_) => return Ok(None),
        },
    };

    for client in &state.ui_clients {
        if verify_ed25519_signature(&client.client_pub, payload.as_bytes(), &sig_bytes) {
            return Ok(Some(UiIdentity {
                client_id: client.client_id.clone(),
                verified: true,
            }));
        }
    }

    let hom_dir = hom_shared::hom_local_dir();
    let clients_path = hom_shared::known_clients_path(&hom_dir);
    if !clients_path.exists() {
        return Ok(None);
    }
    let clients = hom_shared::load_known_clients(&clients_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"ok": false, "error": {"code": -32603, "message": format!("clients_load: {e}")}}),
        )
    })?;
    for client in &clients {
        if verify_ed25519_signature(&client.client_pub, payload.as_bytes(), &sig_bytes) {
            return Ok(Some(UiIdentity {
                client_id: client.client_id.clone(),
                verified: true,
            }));
        }
    }

    Ok(None)
}

fn canonicalize_body(body: &Value) -> String {
    match body {
        Value::Null => String::new(),
        Value::Object(_) | Value::Array(_) => hom_shared::canonical_json(body)
            .unwrap_or_else(|_| serde_json::to_string(body).unwrap_or_default()),
        _ => serde_json::to_string(body).unwrap_or_default(),
    }
}

fn base64_decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    base64::engine::general_purpose::STANDARD.decode(s)
}

fn base64url_decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s)
}

/// Verify an Ed25519 signature against a public key.
/// The public key is stored as base64url-encoded (raw 32-byte or SPKI DER).
fn verify_ed25519_signature(public_key_b64u: &str, message: &[u8], signature: &[u8]) -> bool {
    let pub_bytes: Vec<u8> =
        match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(public_key_b64u) {
            Ok(bytes) => bytes,
            Err(_) => return false,
        };

    // Try to parse as raw 32-byte Ed25519 public key first
    let verifying_key = if pub_bytes.len() == 32 {
        let raw: [u8; 32] = match pub_bytes.try_into() {
            Ok(arr) => arr,
            Err(_) => return false,
        };
        match VerifyingKey::from_bytes(&raw) {
            Ok(key) => key,
            Err(_) => return false,
        }
    } else {
        // Try SPKI DER format (from hom-shared crypto)
        match ed25519_dalek::pkcs8::PublicKeyBytes::try_from(pub_bytes.as_slice()) {
            Ok(pk_bytes) => {
                let raw: [u8; 32] = pk_bytes.to_bytes();
                match VerifyingKey::from_bytes(&raw) {
                    Ok(key) => key,
                    Err(_) => return false,
                }
            }
            Err(_) => return false,
        }
    };

    let sig = match Signature::from_slice(signature) {
        Ok(s) => s,
        Err(_) => return false,
    };

    verifying_key.verify(message, &sig).is_ok()
}
