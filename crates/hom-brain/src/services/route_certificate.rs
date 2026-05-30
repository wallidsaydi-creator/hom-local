use hom_shared::{RpcError, canonical_json, rpc_err, sha256_hex};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::BrainState;
use crate::db::storage::unix_now_s;

pub fn issue(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let method = required_string(&params, &["method"])?;
    let provider_id = optional_string(&params, &["provider_id", "providerId"]);
    let model_id = optional_string(&params, &["model_id", "modelId"]);
    let capability = required_string(&params, &["capability"])?;
    let permission_scope = required_string(&params, &["permission_scope", "permissionScope"])?;
    let descriptor_hash = required_string(&params, &["descriptor_hash", "descriptorHash"])?;
    let policy_id = required_string(&params, &["policy_id", "policyId"])?;
    let request_context = params
        .get("request_context")
        .or_else(|| params.get("requestContext"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let expires_at_s = params.get("expires_at_s").and_then(Value::as_i64);
    let now = unix_now_s();
    let certificate_id = Uuid::new_v4().to_string();
    let evidence = json!({
        "method": method,
        "provider_id": provider_id,
        "model_id": model_id,
        "capability": capability,
        "permission_scope": permission_scope,
        "descriptor_hash": descriptor_hash,
        "policy_id": policy_id,
        "request_context": request_context,
        "expires_at_s": expires_at_s,
        "issued_at_s": now,
        "formula_ref": "route_certificate_hash_v1"
    });
    let certificate_hash =
        sha256_hex(&canonical_json(&evidence).map_err(|error| {
            rpc_err(-32603, format!("route_certificate_canonicalize: {error}"))
        })?);
    let conn = state.store.conn()?;
    conn.execute(
        "INSERT INTO route_certificates
         (id, method, provider_id, model_id, capability, permission_scope, descriptor_hash,
          policy_id, request_context_json, certificate_hash, status, issued_at_s, expires_at_s)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active', ?11, ?12)",
        params![
            certificate_id,
            evidence["method"].as_str(),
            evidence["provider_id"].as_str(),
            evidence["model_id"].as_str(),
            evidence["capability"].as_str(),
            evidence["permission_scope"].as_str(),
            evidence["descriptor_hash"].as_str(),
            evidence["policy_id"].as_str(),
            canonical_json(&request_context)
                .map_err(|error| rpc_err(-32603, format!("route_certificate_context: {error}")))?,
            certificate_hash,
            now,
            expires_at_s,
        ],
    )
    .map_err(|error| rpc_err(-32603, format!("route_certificate_insert: {error}")))?;
    let certificate = load_certificate(&conn, &certificate_id)?
        .ok_or_else(|| rpc_err(-32603, "route_certificate_insert_not_visible"))?;
    Ok(json!({
        "ok": true,
        "artifact_kind": "route_certificate_v1",
        "route_certificate_id": certificate_id,
        "certificate": certificate
    }))
}

pub fn open(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let id = required_string(
        &params,
        &["route_certificate_id", "routeCertificateId", "id"],
    )?;
    let conn = state.store.conn()?;
    let certificate = load_certificate(&conn, &id)?
        .ok_or_else(|| rpc_err(-32044, "route_certificate_not_found"))?;
    Ok(json!({
        "ok": true,
        "artifact_kind": "route_certificate_v1",
        "route_certificate_id": id,
        "certificate": certificate
    }))
}

pub fn load_certificate(conn: &Connection, id: &str) -> Result<Option<Value>, RpcError> {
    conn.query_row(
        "SELECT id, method, provider_id, model_id, capability, permission_scope, descriptor_hash,
                policy_id, request_context_json, certificate_hash, status, issued_at_s, expires_at_s
         FROM route_certificates WHERE id = ?1",
        [id],
        |row| {
            let request_context_json: String = row.get(8)?;
            let request_context: Value =
                serde_json::from_str(&request_context_json).unwrap_or_else(|_| json!({}));
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "method": row.get::<_, String>(1)?,
                "provider_id": row.get::<_, Option<String>>(2)?,
                "model_id": row.get::<_, Option<String>>(3)?,
                "capability": row.get::<_, String>(4)?,
                "permission_scope": row.get::<_, String>(5)?,
                "descriptor_hash": row.get::<_, String>(6)?,
                "policy_id": row.get::<_, String>(7)?,
                "request_context": request_context,
                "certificate_hash": row.get::<_, String>(9)?,
                "status": row.get::<_, String>(10)?,
                "issued_at_s": row.get::<_, i64>(11)?,
                "expires_at_s": row.get::<_, Option<i64>>(12)?,
                "formula_ref": "route_certificate_hash_v1",
                "mutation_permitted": false
            }))
        },
    )
    .optional()
    .map_err(|error| rpc_err(-32603, format!("route_certificate_load: {error}")))
}

fn required_string(params: &Value, names: &[&str]) -> Result<String, RpcError> {
    optional_string(params, names).ok_or_else(|| rpc_err(-32602, format!("{}_required", names[0])))
}

fn optional_string(params: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| params.get(*name).and_then(Value::as_str))
        .map(str::to_string)
}
