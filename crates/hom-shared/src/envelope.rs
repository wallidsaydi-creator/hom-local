use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::canonical_json::canonical_json;
use crate::crypto::{CryptoError, sha256_hex, sign_payload, verify_payload};
use crate::types::{
    ERR_AUTH_MALFORMED, ERR_BODY_HASH_MISMATCH, ERR_CLOCK_SKEW, ERR_NONCE_REPLAY,
    ERR_SCOPE_INSUFFICIENT, ERR_SIGNATURE_INVALID, ERR_UNKNOWN_CLIENT, HomEnvelope, JsonRpcRequest,
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct KnownClient {
    pub client_id: String,
    #[serde(alias = "pub", alias = "pub_key")]
    pub client_pub: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct KnownClientsFile {
    pub clients: Vec<KnownClient>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum KnownClientsWire {
    Vec {
        clients: Vec<KnownClient>,
    },
    Map {
        clients: HashMap<String, KnownClientEntry>,
    },
}

#[derive(Clone, Debug, Deserialize)]
struct KnownClientEntry {
    #[serde(alias = "client_pub", alias = "pub_key")]
    pub r#pub: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedIdentity {
    pub client_id: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Error)]
pub enum EnvelopeError {
    #[error("unknown client")]
    UnknownClient,
    #[error("signature invalid")]
    SignatureInvalid,
    #[error("nonce replay")]
    NonceReplay,
    #[error("clock skew")]
    ClockSkew,
    #[error("body hash mismatch")]
    BodyHashMismatch,
    #[error("malformed auth envelope")]
    Malformed,
    #[error("scope insufficient")]
    ScopeInsufficient,
    #[error("crypto error: {0}")]
    Crypto(#[from] CryptoError),
    #[error("canonical JSON failed: {0}")]
    Canonical(#[from] crate::canonical_json::CanonicalJsonError),
    #[error("known client file read failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("known client file parse failed: {0}")]
    Json(#[from] serde_json::Error),
}

impl EnvelopeError {
    pub fn code(&self) -> i64 {
        match self {
            EnvelopeError::UnknownClient => ERR_UNKNOWN_CLIENT,
            EnvelopeError::SignatureInvalid => ERR_SIGNATURE_INVALID,
            EnvelopeError::NonceReplay => ERR_NONCE_REPLAY,
            EnvelopeError::ClockSkew => ERR_CLOCK_SKEW,
            EnvelopeError::BodyHashMismatch => ERR_BODY_HASH_MISMATCH,
            EnvelopeError::ScopeInsufficient => ERR_SCOPE_INSUFFICIENT,
            EnvelopeError::Malformed
            | EnvelopeError::Crypto(_)
            | EnvelopeError::Canonical(_)
            | EnvelopeError::Io(_)
            | EnvelopeError::Json(_) => ERR_AUTH_MALFORMED,
        }
    }
}

pub struct EnvelopeVerifier {
    clients: HashMap<String, KnownClient>,
    seen_nonces: HashSet<String>,
    skew_seconds: i64,
    nonce_ttl_seconds: i64,
    now_fn: Box<dyn Fn() -> i64 + Send + Sync>,
}

impl EnvelopeVerifier {
    pub fn new(clients: Vec<KnownClient>) -> Self {
        Self::with_clock(clients, 30, 60, || {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        })
    }

    pub fn with_clock(
        clients: Vec<KnownClient>,
        skew_seconds: i64,
        nonce_ttl_seconds: i64,
        now_fn: impl Fn() -> i64 + Send + Sync + 'static,
    ) -> Self {
        Self {
            clients: clients
                .into_iter()
                .map(|client| (client.client_id.clone(), client))
                .collect(),
            seen_nonces: HashSet::new(),
            skew_seconds,
            nonce_ttl_seconds,
            now_fn: Box::new(now_fn),
        }
    }

    pub fn verify_request(
        &mut self,
        request: &JsonRpcRequest,
        required_scope: Option<&str>,
    ) -> Result<VerifiedIdentity, EnvelopeError> {
        let hom = request.hom.as_ref().ok_or(EnvelopeError::Malformed)?;
        self.verify_body(&request.params, hom, required_scope)
    }

    pub fn verify_body(
        &mut self,
        body: &Value,
        hom: &HomEnvelope,
        required_scope: Option<&str>,
    ) -> Result<VerifiedIdentity, EnvelopeError> {
        if hom.client_id.is_empty()
            || hom.client_pub.is_empty()
            || hom.nonce.is_empty()
            || hom.signature.is_empty()
        {
            return Err(EnvelopeError::Malformed);
        }

        let client = self
            .clients
            .get(&hom.client_id)
            .cloned()
            .ok_or(EnvelopeError::UnknownClient)?;
        if client.client_pub != hom.client_pub {
            return Err(EnvelopeError::UnknownClient);
        }

        let now = (self.now_fn)();
        if (now - hom.ts).abs() > self.skew_seconds {
            return Err(EnvelopeError::ClockSkew);
        }

        self.prune_nonces(now);
        let nonce_key = format!("{}:{}:{}", hom.client_id, hom.nonce, hom.ts);
        if !self.seen_nonces.insert(nonce_key) {
            return Err(EnvelopeError::NonceReplay);
        }

        let body_canonical = canonical_json(body)?;
        if sha256_hex(&body_canonical) != hom.body_hash {
            return Err(EnvelopeError::BodyHashMismatch);
        }

        let granted: HashSet<_> = client.scopes.iter().map(String::as_str).collect();
        if !hom
            .scope
            .iter()
            .all(|scope| granted.contains(scope.as_str()))
        {
            return Err(EnvelopeError::ScopeInsufficient);
        }
        if let Some(required) = required_scope {
            if !hom.scope.iter().any(|scope| scope == required) {
                return Err(EnvelopeError::ScopeInsufficient);
            }
        }

        let ok = verify_payload(&client.client_pub, body, &hom.nonce, hom.ts, &hom.signature)?;
        if !ok {
            return Err(EnvelopeError::SignatureInvalid);
        }

        Ok(VerifiedIdentity {
            client_id: hom.client_id.clone(),
            scopes: hom.scope.clone(),
        })
    }

    fn prune_nonces(&mut self, now: i64) {
        let ttl = self.nonce_ttl_seconds;
        self.seen_nonces.retain(|key| {
            key.rsplit(':')
                .next()
                .and_then(|ts| ts.parse::<i64>().ok())
                .is_some_and(|ts| now - ts <= ttl)
        });
    }
}

pub fn build_request(
    id: Value,
    method: impl Into<String>,
    params: Value,
    client_id: impl Into<String>,
    client_pub: impl Into<String>,
    private_key: &str,
    scopes: Vec<String>,
    nonce: impl Into<String>,
    ts: i64,
) -> Result<JsonRpcRequest, EnvelopeError> {
    let nonce = nonce.into();
    let body_hash = sha256_hex(&canonical_json(&params)?);
    let signature = sign_payload(private_key, &params, &nonce, ts)?;
    Ok(JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id,
        method: method.into(),
        params,
        hom: Some(HomEnvelope {
            client_id: client_id.into(),
            client_pub: client_pub.into(),
            ts,
            nonce,
            scope: scopes,
            body_hash,
            signature,
        }),
    })
}

pub fn load_known_clients(path: impl AsRef<Path>) -> Result<Vec<KnownClient>, EnvelopeError> {
    let raw = fs::read_to_string(path)?;
    let wire = serde_json::from_str::<KnownClientsWire>(&raw)?;
    Ok(match wire {
        KnownClientsWire::Vec { clients } => clients,
        KnownClientsWire::Map { clients } => clients
            .into_iter()
            .map(|(client_id, entry)| KnownClient {
                client_id,
                client_pub: entry.r#pub,
                scopes: entry.scopes,
            })
            .collect(),
    })
}

pub fn save_known_clients(
    path: impl AsRef<Path>,
    clients: &[KnownClient],
) -> Result<(), EnvelopeError> {
    let file = KnownClientsFile {
        clients: clients.to_vec(),
    };
    let json = serde_json::to_string_pretty(&file)?;
    let tmp_path = path.as_ref().with_extension("tmp");
    fs::write(&tmp_path, &json)?;
    fs::rename(&tmp_path, path.as_ref())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::generate_keypair;
    use serde_json::json;

    fn verifier_for(now: i64) -> (EnvelopeVerifier, KeypairFixture) {
        let keys = generate_keypair();
        let fixture = KeypairFixture {
            client_id: "test.client".to_string(),
            private: keys.private_key.clone(),
            public: keys.public_key.clone(),
        };
        let verifier = EnvelopeVerifier::with_clock(
            vec![KnownClient {
                client_id: fixture.client_id.clone(),
                client_pub: fixture.public.clone(),
                scopes: vec!["memory:recall".to_string()],
            }],
            30,
            60,
            move || now,
        );
        (verifier, fixture)
    }

    #[test]
    fn loads_clients_array_file() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        fs::write(
            temp.path(),
            r#"{
              "clients": [
                {
                  "client_id": "ingress.local",
                  "client_pub": "pub-array",
                  "scopes": ["system"]
                }
              ]
            }"#,
        )
        .unwrap();

        let clients = load_known_clients(temp.path()).unwrap();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].client_id, "ingress.local");
        assert_eq!(clients[0].client_pub, "pub-array");
        assert_eq!(clients[0].scopes, vec!["system"]);
    }

    #[test]
    fn loads_legacy_clients_map_file() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        fs::write(
            temp.path(),
            r#"{
              "version": 1,
              "clients": {
                "ingress.local": {
                  "pub": "pub-map",
                  "scopes": ["system", "memory:recall"]
                }
              }
            }"#,
        )
        .unwrap();

        let clients = load_known_clients(temp.path()).unwrap();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].client_id, "ingress.local");
        assert_eq!(clients[0].client_pub, "pub-map");
        assert_eq!(clients[0].scopes, vec!["system", "memory:recall"]);
    }

    struct KeypairFixture {
        client_id: String,
        private: String,
        public: String,
    }

    #[test]
    fn verifies_and_rejects_replay() {
        let (mut verifier, keys) = verifier_for(100);
        let request = build_request(
            json!(1),
            "memory.recall",
            json!({"q": "alpha"}),
            keys.client_id,
            keys.public,
            &keys.private,
            vec!["memory:recall".to_string()],
            "nonce-1",
            100,
        )
        .unwrap();
        assert!(
            verifier
                .verify_request(&request, Some("memory:recall"))
                .is_ok()
        );
        assert_eq!(
            verifier
                .verify_request(&request, Some("memory:recall"))
                .unwrap_err()
                .code(),
            ERR_NONCE_REPLAY
        );
    }

    #[test]
    fn rejects_tampered_body() {
        let (mut verifier, keys) = verifier_for(100);
        let mut request = build_request(
            json!(1),
            "memory.recall",
            json!({"q": "alpha"}),
            keys.client_id,
            keys.public,
            &keys.private,
            vec!["memory:recall".to_string()],
            "nonce-1",
            100,
        )
        .unwrap();
        request.params = json!({"q": "beta"});
        assert_eq!(
            verifier
                .verify_request(&request, Some("memory:recall"))
                .unwrap_err()
                .code(),
            ERR_BODY_HASH_MISMATCH
        );
    }

    #[test]
    fn rejects_stale_timestamp() {
        let (mut verifier, keys) = verifier_for(200);
        let request = build_request(
            json!(1),
            "memory.recall",
            json!({}),
            keys.client_id,
            keys.public,
            &keys.private,
            vec!["memory:recall".to_string()],
            "nonce-1",
            100,
        )
        .unwrap();
        assert_eq!(
            verifier
                .verify_request(&request, Some("memory:recall"))
                .unwrap_err()
                .code(),
            ERR_CLOCK_SKEW
        );
    }
}
