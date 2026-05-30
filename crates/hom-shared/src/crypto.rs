use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::canonical_json::{CanonicalJsonError, canonical_json};

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("invalid base64url")]
    InvalidBase64,
    #[error("invalid ed25519 key")]
    InvalidKey,
    #[error("invalid ed25519 signature")]
    InvalidSignature,
    #[error("canonical JSON failed: {0}")]
    Canonical(#[from] CanonicalJsonError),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GeneratedKeypair {
    pub private_key: String,
    pub public_key: String,
}

pub type KeypairMaterial = GeneratedKeypair;

pub fn b64u_encode(bytes: impl AsRef<[u8]>) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn b64u_decode(value: &str) -> Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| "invalid_base64url".to_string())
}

pub fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn sha256_b64u(bytes: impl AsRef<[u8]>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes.as_ref());
    b64u_encode(hasher.finalize())
}

pub fn public_fingerprint(public_key: &str) -> Result<String, CryptoError> {
    let public_der = b64u_decode(public_key).map_err(|_| CryptoError::InvalidBase64)?;
    let mut hasher = Sha256::new();
    hasher.update(public_der);
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn public_key_from_private(private_key: &str) -> Result<String, CryptoError> {
    let private_der = b64u_decode(private_key).map_err(|_| CryptoError::InvalidBase64)?;
    let signing_key =
        SigningKey::from_pkcs8_der(&private_der).map_err(|_| CryptoError::InvalidKey)?;
    let public_der = signing_key
        .verifying_key()
        .to_public_key_der()
        .map_err(|_| CryptoError::InvalidKey)?;
    Ok(b64u_encode(public_der.as_bytes()))
}

pub fn generate_keypair() -> GeneratedKeypair {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let private_key = signing_key
        .to_pkcs8_der()
        .expect("generated Ed25519 key serializes as PKCS#8");
    let public_key = verifying_key
        .to_public_key_der()
        .expect("generated Ed25519 key serializes as SPKI");

    GeneratedKeypair {
        private_key: b64u_encode(private_key.as_bytes()),
        public_key: b64u_encode(public_key.as_bytes()),
    }
}

pub fn sign_payload(
    private_key: &str,
    body: &Value,
    nonce: &str,
    ts: i64,
) -> Result<String, CryptoError> {
    let private_der = b64u_decode(private_key).map_err(|_| CryptoError::InvalidBase64)?;
    let signing_key =
        SigningKey::from_pkcs8_der(&private_der).map_err(|_| CryptoError::InvalidKey)?;
    let signature = signing_key.sign(signing_payload(body, nonce, ts)?.as_bytes());
    Ok(b64u_encode(signature.to_bytes()))
}

pub fn verify_payload(
    client_pub: &str,
    body: &Value,
    nonce: &str,
    ts: i64,
    signature: &str,
) -> Result<bool, CryptoError> {
    let public_der = b64u_decode(client_pub).map_err(|_| CryptoError::InvalidBase64)?;
    let verifying_key =
        VerifyingKey::from_public_key_der(&public_der).map_err(|_| CryptoError::InvalidKey)?;
    let signature = b64u_decode(signature).map_err(|_| CryptoError::InvalidBase64)?;
    let signature: [u8; 64] = signature
        .try_into()
        .map_err(|_| CryptoError::InvalidSignature)?;
    let signature = Signature::from_bytes(&signature);

    Ok(verifying_key
        .verify(signing_payload(body, nonce, ts)?.as_bytes(), &signature)
        .is_ok())
}

fn signing_payload(body: &Value, nonce: &str, ts: i64) -> Result<String, CanonicalJsonError> {
    Ok(format!("{}\n{}\n{}", canonical_json(body)?, nonce, ts))
}
