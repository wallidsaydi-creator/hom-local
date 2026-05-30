use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::crypto::{GeneratedKeypair, generate_keypair, public_key_from_private};
use crate::envelope::{KnownClient, load_known_clients, save_known_clients};

#[derive(Debug, Error)]
pub enum KeyStoreError {
    #[error("key store io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("key store JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("key store envelope error: {0}")]
    Envelope(#[from] crate::envelope::EnvelopeError),
    #[error("key store missing public key in {0}")]
    MissingPublicKey(PathBuf),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HelperIdentity {
    pub private_key: String,
    pub public_key: String,
}

pub fn ensure_helper_identity(
    hom_dir: impl AsRef<Path>,
    helper_id: &str,
    client_id: &str,
    scopes: &[&str],
) -> Result<HelperIdentity, KeyStoreError> {
    let hom_dir = hom_dir.as_ref();
    let keys_dir = hom_dir.join("keys");
    ensure_private_dir(&keys_dir)?;

    let key_path = keys_dir.join(format!("{helper_id}.key"));
    let identity = match fs::read_to_string(&key_path) {
        Ok(raw) => parse_identity(&raw, &key_path)?,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let GeneratedKeypair {
                private_key,
                public_key,
            } = generate_keypair();
            let identity = HelperIdentity {
                private_key,
                public_key,
            };
            write_private_file(&key_path, &serde_json::to_vec_pretty(&identity)?)?;
            identity
        }
        Err(error) => return Err(error.into()),
    };

    upsert_known_client(hom_dir, client_id, &identity.public_key, scopes)?;
    Ok(identity)
}

pub fn known_clients_path(hom_dir: impl AsRef<Path>) -> PathBuf {
    hom_dir.as_ref().join("known_clients.json")
}

fn parse_identity(raw: &str, path: &Path) -> Result<HelperIdentity, KeyStoreError> {
    if raw.trim_start().starts_with('{') {
        return Ok(serde_json::from_str(raw)?);
    }
    let private_key = raw.trim().to_string();
    let public_key = public_key_from_private(&private_key)
        .map_err(|_| KeyStoreError::MissingPublicKey(path.to_path_buf()))?;
    Ok(HelperIdentity {
        private_key,
        public_key,
    })
}

fn upsert_known_client(
    hom_dir: &Path,
    client_id: &str,
    public_key: &str,
    scopes: &[&str],
) -> Result<(), KeyStoreError> {
    ensure_private_dir(hom_dir)?;
    let path = known_clients_path(hom_dir);
    let mut clients = load_known_clients(&path).unwrap_or_default();
    let scope_vec: Vec<String> = scopes.iter().map(|s| s.to_string()).collect();
    if let Some(existing) = clients.iter_mut().find(|c| c.client_id == client_id) {
        existing.client_pub = public_key.to_string();
        existing.scopes = scope_vec;
    } else {
        clients.push(KnownClient {
            client_id: client_id.to_string(),
            client_pub: public_key.to_string(),
            scopes: scope_vec,
        });
    }
    save_known_clients(&path, &clients)?;
    Ok(())
}

fn ensure_private_dir(path: &Path) -> Result<(), std::io::Error> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp, path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}
