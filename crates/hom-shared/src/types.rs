use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const ERR_UNKNOWN_CLIENT: i64 = -32001;
pub const ERR_SIGNATURE_INVALID: i64 = -32002;
pub const ERR_NONCE_REPLAY: i64 = -32003;
pub const ERR_CLOCK_SKEW: i64 = -32004;
pub const ERR_BODY_HASH_MISMATCH: i64 = -32005;
pub const ERR_AUTH_MALFORMED: i64 = -32006;
pub const ERR_SCOPE_INSUFFICIENT: i64 = -32010;
pub const ERR_QUEUE_FULL: i64 = -32008;
pub const ERR_BREAKER_OPEN: i64 = -32009;
pub const ERR_QUARANTINED_INPUT: i64 = -32011;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct HomEnvelope {
    pub client_id: String,
    pub client_pub: String,
    pub ts: i64,
    pub nonce: String,
    pub scope: Vec<String>,
    pub body_hash: String,
    pub signature: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub hom: Option<HomEnvelope>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

pub fn json_rpc_success(id: Value, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: Some(result),
        error: None,
    }
}

pub fn json_rpc_error(id: Value, code: i64, message: impl Into<String>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.into(),
            data: None,
        }),
    }
}

pub fn json_rpc_error_data(
    id: Value,
    code: i64,
    message: impl Into<String>,
    data: Value,
) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.into(),
            data: Some(data),
        }),
    }
}
