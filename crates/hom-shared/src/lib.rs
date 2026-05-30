// SPDX-License-Identifier: Apache-2.0
pub mod canonical_json;
pub mod crypto;
pub mod envelope;
pub mod keys;
pub mod paths;
pub mod rpc;
pub mod types;
pub mod uds;

pub use canonical_json::canonical_json;
pub use crypto::{
    CryptoError, GeneratedKeypair, KeypairMaterial, b64u_decode, b64u_encode, generate_keypair,
    public_fingerprint, public_key_from_private, sha256_b64u, sha256_hex, sign_payload,
    verify_payload,
};
pub use envelope::{
    EnvelopeError, EnvelopeVerifier, KnownClient, KnownClientsFile, VerifiedIdentity,
    build_request, load_known_clients, save_known_clients,
};
pub use keys::{HelperIdentity, KeyStoreError, ensure_helper_identity, known_clients_path};
pub use paths::{hom_local_dir, socket_path};
pub use rpc::{RpcError, RpcResult, rpc_err};
pub use types::{
    ERR_AUTH_MALFORMED, ERR_BODY_HASH_MISMATCH, ERR_BREAKER_OPEN, ERR_CLOCK_SKEW, ERR_NONCE_REPLAY,
    ERR_QUARANTINED_INPUT, ERR_QUEUE_FULL, ERR_SCOPE_INSUFFICIENT, ERR_SIGNATURE_INVALID,
    ERR_UNKNOWN_CLIENT, HomEnvelope, JsonRpcError, JsonRpcRequest, JsonRpcResponse, json_rpc_error,
    json_rpc_error_data, json_rpc_success,
};
