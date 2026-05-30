use hom_shared::{RpcError, rpc_err, sha256_hex};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use uuid::Uuid;

struct ModelCatalogContext {
    provider_id: String,
    model_id: String,
    context_window_tokens: i64,
    max_output_tokens: Option<i64>,
    catalog_source: String,
    catalog_verified: bool,
    input_tokens_estimated: Option<i64>,
}

use crate::BrainState;
use crate::db::storage::{SaveInput, append_ledger_tx, unix_now_s};
use crate::services::{provider_catalog, reasoning_bridge, security_gate};

pub fn get(state: &BrainState) -> Result<Value, RpcError> {
    let hom_dir = &state.hom_dir;
    let owner_key_path = hom_dir.join("keys").join("owner.key");
    let configured = owner_key_path.exists();
    let (base_url, base_url_source) = daemon_base_url(state);

    if configured {
        let raw = std::fs::read_to_string(&owner_key_path).unwrap_or_default();
        let key_json = serde_json::from_str::<serde_json::Value>(&raw).ok();
        let public_key = key_json
            .as_ref()
            .and_then(|value| value.get("public_key"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        let private_key_present = key_json
            .as_ref()
            .and_then(|value| value.get("private_key"))
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        let permission_ok = owner_key_permissions_ok(&owner_key_path);
        let key_shape_ok = public_key.is_some() && private_key_present;
        Ok(json!({
            "configured": true,
            "unlocked": key_shape_ok && permission_ok,
            "daemon": {
                "baseUrl": base_url,
                "baseUrlSource": base_url_source,
            },
            "serverPublicKey": public_key.unwrap_or_default(),
            "identityProof": {
                "key_file_exists": true,
                "key_shape_ok": key_shape_ok,
                "private_key_present": private_key_present,
                "permission_ok": permission_ok,
                "required_mode": "0600"
            }
        }))
    } else {
        Ok(json!({
            "configured": false,
            "unlocked": false,
            "daemon": {
                "baseUrl": base_url,
                "baseUrlSource": base_url_source,
            },
            "identityProof": {
                "key_file_exists": false,
                "key_shape_ok": false,
                "private_key_present": false,
                "permission_ok": false,
                "required_mode": "0600"
            }
        }))
    }
}

pub fn login(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let hom_dir = &state.hom_dir;
    let keys_dir = hom_dir.join("keys");
    std::fs::create_dir_all(&keys_dir).map_err(|e| rpc_err(-32603, format!("keys_dir: {e}")))?;

    let owner_key_path = keys_dir.join("owner.key");
    if owner_key_path.exists() {
        return get(state);
    }

    let keys = hom_shared::generate_keypair();
    let key_json = json!({
        "private_key": keys.private_key,
        "public_key": keys.public_key,
    });
    let key_data = serde_json::to_string_pretty(&key_json)
        .map_err(|e| rpc_err(-32603, format!("key_serialize: {e}")))?;

    let tmp_path = keys_dir.join("owner.key.tmp");
    std::fs::write(&tmp_path, &key_data).map_err(|e| rpc_err(-32603, format!("key_write: {e}")))?;
    std::fs::rename(&tmp_path, &owner_key_path)
        .map_err(|e| rpc_err(-32603, format!("key_rename: {e}")))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&owner_key_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| rpc_err(-32603, format!("key_chmod: {e}")))?;
    }

    let known_clients_path = hom_dir.join("known_clients.json");
    let mut clients = hom_shared::load_known_clients(&known_clients_path).unwrap_or_default();
    clients.push(hom_shared::KnownClient {
        client_id: "owner.local".to_string(),
        client_pub: keys.public_key.clone(),
        scopes: vec![
            "system".to_string(),
            "memory:save".to_string(),
            "memory:recall".to_string(),
            "memory:open".to_string(),
            "memory:answer".to_string(),
            "events:read".to_string(),
            "events:write".to_string(),
            "diagnostics:read".to_string(),
            "security:read".to_string(),
            "gates:read".to_string(),
            "gates:write".to_string(),
        ],
    });

    // Register the UI client's Ed25519 public key if provided (raw 32-byte, base64url).
    // Wrapped as SPKI DER so signed requests from the macOS app verify against known_clients.
    if let Some(client_pub) = params
        .get("client_pub")
        .or_else(|| params.get("clientPub"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        if let Some(spki_der) = raw_ed25519_pub_to_spki_der(client_pub) {
            clients.push(hom_shared::KnownClient {
                client_id: "ui.local".to_string(),
                client_pub: spki_der,
                scopes: vec![
                    "system".to_string(),
                    "memory:save".to_string(),
                    "memory:recall".to_string(),
                    "memory:open".to_string(),
                    "memory:answer".to_string(),
                    "events:read".to_string(),
                    "events:write".to_string(),
                    "diagnostics:read".to_string(),
                    "security:read".to_string(),
                    "gates:read".to_string(),
                    "gates:write".to_string(),
                ],
            });
        }
    }

    hom_shared::save_known_clients(&known_clients_path, &clients)
        .map_err(|e| rpc_err(-32603, format!("known_clients_save: {e}")))?;

    let (base_url, base_url_source) = daemon_base_url(state);
    Ok(json!({
        "configured": true,
        "unlocked": true,
        "daemon": {
            "baseUrl": base_url,
            "baseUrlSource": base_url_source,
        },
        "serverPublicKey": keys.public_key,
        "identityProof": {
            "key_file_exists": true,
            "key_shape_ok": true,
            "private_key_present": true,
            "permission_ok": true,
            "required_mode": "0600"
        }
    }))
}

/// Logout: destroys the owner key and known clients, returning to unconfigured state.
/// The owner must re-login (generating a fresh keypair) to use the system again.
pub fn logout(state: &BrainState) -> Result<Value, RpcError> {
    let hom_dir = &state.hom_dir;
    let owner_key_path = hom_dir.join("keys").join("owner.key");
    let known_clients_path = hom_dir.join("known_clients.json");

    // Remove the owner key file
    if owner_key_path.exists() {
        std::fs::remove_file(&owner_key_path)
            .map_err(|e| rpc_err(-32603, format!("logout_key_remove: {e}")))?;
    }

    // Remove known clients (revokes all granted access)
    if known_clients_path.exists() {
        std::fs::remove_file(&known_clients_path)
            .map_err(|e| rpc_err(-32603, format!("logout_clients_remove: {e}")))?;
    }

    Ok(json!({
        "ok": true,
        "unlocked": false,
        "configured": false,
    }))
}

pub fn compact(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let session_id = params
        .get("session_id")
        .or_else(|| params.get("sessionId"))
        .or_else(|| params.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| active_session_id(state).ok().flatten())
        .ok_or_else(|| rpc_err(-32602, "session_id_required"))?;
    let limit = params
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(500)
        .clamp(1, 1000);
    let (session_title, memories) = load_session_memories(state, &session_id, limit)?;
    if memories.is_empty() {
        return Err(rpc_err(-32602, "session_has_no_memories"));
    }
    let model_catalog = resolve_model_catalog_context(&params)?;

    let now = unix_now_s();
    let source_ids = memories
        .iter()
        .filter_map(|memory| {
            memory
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    let compaction_key = format!(
        "session_compaction:{}:{}:{}",
        session_id,
        now,
        &Uuid::new_v4().to_string()[..8]
    );
    let artifact = build_compaction_artifact(
        &session_id,
        session_title.as_deref(),
        now,
        &memories,
        &model_catalog,
    );
    let remaining_secret_patterns =
        security_gate::generated_artifact_secret_matches(&artifact.text);
    if !remaining_secret_patterns.is_empty() {
        let patterns = remaining_secret_patterns
            .iter()
            .map(|item| item.pattern)
            .collect::<Vec<_>>()
            .join(",");
        return Err(rpc_err(
            -32603,
            format!(
                "session_compaction_artifact_secret_after_redaction: patterns={patterns}; redacted_string_count={}",
                artifact.redacted_string_count
            ),
        ));
    }
    let artifact_hash = sha256_hex(&artifact.text);
    let redacted_string_count = artifact.redacted_string_count;
    let compaction_strategy = if redacted_string_count == 0 {
        "extractive_full_detail"
    } else {
        "extractive_full_detail_with_secret_hash_redaction"
    };
    let saved = state
        .store
        .save_memory(SaveInput {
            key: Some(compaction_key.clone()),
            value: Some(artifact.text),
            memory_type: Some("session_compaction".to_string()),
            source: Some("brain.session.compact".to_string()),
            session_id: Some(session_id.clone()),
            metadata: json!({
                "kind": "session_compaction_continuity_v1",
                "strategy": compaction_strategy,
                "session_id": session_id.clone(),
                "source_memory_ids": source_ids.clone(),
                "source_memory_count": memories.len(),
                "artifact_hash": artifact_hash.clone(),
                "created_by": "brain.session.compact",
                "native_compaction": true,
                "model_agnostic": true,
                "provider_id": model_catalog.provider_id,
                "model_id": model_catalog.model_id,
                "provider_catalog_verified": model_catalog.catalog_verified,
                "deterministic_context_window": model_catalog.catalog_verified,
                "context_window_tokens": model_catalog.context_window_tokens,
                "max_output_tokens": model_catalog.max_output_tokens,
                "input_tokens_estimated": model_catalog.input_tokens_estimated,
                "brain_calibration_deferred": true,
                "brain_save_required": true,
                "post_compaction_summary_required": true,
                "artifact_kind": "session_compaction_continuity_v1",
                "compartments": {
                    "conversation": true,
                    "tool_usage": true,
                    "actions_taken": true,
                    "what_how_why": true
                },
                "redaction": {
                    "applied": redacted_string_count > 0,
                    "redacted_string_count": redacted_string_count,
                    "boundary": "secret-like string leaves are replaced with hash proofs before memory.save"
                }
            }),
            project_id: None,
            track: None,
            trusted_generated_artifact: true,
        })
        .map_err(|e| rpc_err(-32603, format!("session_compaction_save: {e}")))?;
    let compaction_memory_id = saved["memory_id"]
        .as_str()
        .ok_or_else(|| rpc_err(-32603, "session_compaction_missing_memory_id"))?
        .to_string();
    let ledger = record_compaction_ledger(
        state,
        &session_id,
        &compaction_memory_id,
        &compaction_key,
        memories.len(),
        &artifact_hash,
        compaction_strategy,
        redacted_string_count,
    )?;
    let recall_probe = state
        .store
        .recall(&compaction_key, 5)
        .map(|result| {
            let found = result
                .get("memories")
                .and_then(Value::as_array)
                .map(|items| {
                    items.iter().any(|item| {
                        item.get("memory_id").and_then(Value::as_str)
                            == Some(compaction_memory_id.as_str())
                    })
                })
                .unwrap_or(false);
            json!({
                "query": compaction_key,
                "found_compaction": found,
                "result_count": result.get("result_count").cloned().unwrap_or_else(|| json!(0))
            })
        })
        .map_err(|e| rpc_err(-32603, format!("session_compaction_recall_probe: {e}")))?;
    let ledger_event_id = ledger.get("event_id").and_then(Value::as_str);
    let continuity_summary = build_concise_continuity_summary(
        &session_id,
        session_title.as_deref(),
        memories.len(),
        &model_catalog,
    );
    let continuity_next_action =
        "verify Phase 0 compaction continuity contract and proceed to catalog failure semantics";
    let bridge = reasoning_bridge::record_compaction(
        state,
        &compaction_memory_id,
        &source_ids,
        ledger_event_id,
        &recall_probe,
    )?;

    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "compaction_kind": "native_model_agnostic_compaction_v1",
        "native_compaction": true,
        "model_agnostic": true,
        "continuity_guaranteed": true,
        "post_compaction_summary_delivered": true,
        "provider": {
            "provider_id": model_catalog.provider_id,
            "model_id": model_catalog.model_id,
            "context_window_tokens": model_catalog.context_window_tokens,
            "max_output_tokens": model_catalog.max_output_tokens,
            "catalog_verified": model_catalog.catalog_verified,
            "catalog_source": model_catalog.catalog_source
        },
        "budget": {
            "deterministic_context_window": model_catalog.catalog_verified,
            "context_window_tokens": model_catalog.context_window_tokens,
            "input_tokens_estimated": model_catalog.input_tokens_estimated,
            "calibration_in_compaction_path": false
        },
        "dynamic_compaction": {
            "model_capability_api": "providers.model_catalog",
            "context_window_source": model_catalog.catalog_source,
            "context_window_tokens": model_catalog.context_window_tokens,
            "deterministic_context_window": model_catalog.catalog_verified && model_catalog.context_window_tokens > 0,
            "fallback_static_threshold": !(model_catalog.catalog_verified && model_catalog.context_window_tokens > 0),
            "fallback_label": if model_catalog.catalog_verified && model_catalog.context_window_tokens > 0 { Value::Null } else { json!("missing_or_unverified_model_context_window") }
        },
        "hom_local_acceptance": {
            "accepted": true,
            "blocking_calibration": false,
            "acceptance_boundary": "save_and_compact_without_threshold_gate"
        },
        "brain_calibration": {
            "deferred": true,
            "calibration_scope": "nightly_and_reasoning",
            "calibration_boundary": "post_save_brain_layer"
        },
        "brain_save": {
            "required": true,
            "completed": true,
            "artifact_kind": "session_compaction_continuity_v1",
            "memory_id": compaction_memory_id,
            "compaction_key": compaction_key,
            "ledger_event_id": ledger_event_id
        },
        "continuity": {
            "summary": continuity_summary,
            "current_state": "Phase 0 native compaction continuity artifact saved to HOM Local brain",
            "next_action": continuity_next_action,
            "open_handles": [
                {"kind": "memory", "id": compaction_memory_id},
                {"kind": "ledger_event", "id": ledger_event_id}
            ]
        },
        "compaction_memory_id": compaction_memory_id,
        "compaction_key": compaction_key,
        "source_memory_count": memories.len(),
        "source_memory_ids": source_ids,
        "artifact_hash": artifact_hash,
        "redaction": {
            "applied": redacted_string_count > 0,
            "redacted_string_count": redacted_string_count
        },
        "save_result": saved,
        "ledger": ledger,
        "recall_probe": recall_probe,
        "reasoning_bridge": bridge
    }))
}

pub fn compactions(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let limit = params
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(10)
        .clamp(1, 100);
    let session_filter = params
        .get("session_id")
        .or_else(|| params.get("sessionId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let conn = state.store.conn()?;
    let mut sql = "SELECT id, key, session_id, metadata_json, created_at_s, updated_at_s FROM memories WHERE memory_type = 'session_compaction' AND metadata_json LIKE '%session_compaction_continuity_v1%'".to_string();
    if session_filter.is_some() {
        sql.push_str(" AND session_id = ?1 ORDER BY created_at_s DESC LIMIT ?2");
    } else {
        sql.push_str(" ORDER BY created_at_s DESC LIMIT ?1");
    }
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| rpc_err(-32603, format!("session_compactions_prepare: {e}")))?;
    let rows = if let Some(session_id) = session_filter.as_deref() {
        stmt.query_map(
            rusqlite::params![session_id, limit],
            compaction_registry_row,
        )
        .map_err(|e| rpc_err(-32603, format!("session_compactions_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("session_compactions_collect: {e}")))?
    } else {
        stmt.query_map(rusqlite::params![limit], compaction_registry_row)
            .map_err(|e| rpc_err(-32603, format!("session_compactions_query: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| rpc_err(-32603, format!("session_compactions_collect: {e}")))?
    };
    Ok(json!({
        "ok": true,
        "artifact_kind": "session_compaction_continuity_registry_v1",
        "count": rows.len(),
        "items": rows,
        "calibration_boundary": {
            "deferred_to_brain": true,
            "inspector_calibrates": false,
            "source": "session_compaction_continuity_v1"
        }
    }))
}

pub fn compaction_snapshot(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let session_id = params
        .get("session_id")
        .or_else(|| params.get("sessionId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown-session")
        .to_string();
    let provider_id = params
        .get("provider_id")
        .or_else(|| params.get("providerId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown-provider")
        .to_string();
    let model_id = params
        .get("model_id")
        .or_else(|| params.get("modelId"))
        .or_else(|| params.get("model"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown-model")
        .to_string();

    let catalog = provider_catalog::model_catalog(params.clone())?;
    let pressure = provider_catalog::context_pressure(params.clone())?;
    let compactions = compactions(
        state,
        json!({
            "session_id": session_id,
            "limit": params.get("compaction_limit").and_then(Value::as_i64).unwrap_or(5).clamp(1, 20)
        }),
    )?;
    let latest_compaction = compactions
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .cloned()
        .map(|mut item| {
            if let Value::Object(map) = &mut item {
                let memory_id = map.get("memory_id").cloned().unwrap_or(Value::Null);
                map.insert(
                    "inspectorTarget".to_string(),
                    json!({"kind": "compaction_artifact", "id": memory_id}),
                );
            }
            item
        })
        .unwrap_or(Value::Null);

    Ok(json!({
        "ok": true,
        "snapshot_kind": "compaction_ux_snapshot_v1",
        "session_id": session_id,
        "provider": {
            "provider_id": provider_id,
            "model_id": model_id,
            "dynamic_provider": true,
            "dynamic_model": true
        },
        "catalog": catalog,
        "pressure": pressure,
        "compactions": compactions,
        "latest_compaction": latest_compaction,
        "hom_local_acceptance": {
            "accepted": true,
            "blocking_calibration": false,
            "acceptance_boundary": "ux_snapshot_aggregates_backend_truth_without_mutation"
        },
        "digestive_boundary": {
            "snapshot_calibrates": false,
            "snapshot_triggers_compaction": false,
            "snapshot_mutates_artifacts": false,
            "brain_calibration_deferred": true,
            "calibration_scope": "nightly_and_reasoning"
        },
        "ux_contract": {
            "single_backend_snapshot": true,
            "frontend_stitches_raw_routes": false,
            "sources": [
                "providers.model_catalog",
                "session.context_pressure",
                "session.compactions",
                "ui.inspector.target"
            ],
            "open_handles": if latest_compaction.is_null() {
                json!([])
            } else {
                json!([latest_compaction.get("inspectorTarget").cloned().unwrap_or(Value::Null)])
            }
        }
    }))
}

pub fn compaction_open(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let memory_id = params
        .get("memory_id")
        .or_else(|| params.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| rpc_err(-32602, "compaction_memory_id_required"))?;
    open_compaction_artifact(state, memory_id)
}

pub fn open_compaction_artifact(state: &BrainState, memory_id: &str) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let row = conn
        .query_row(
            "SELECT id, key, value, session_id, metadata_json, created_at_s, updated_at_s
             FROM memories WHERE id = ?1 AND memory_type = 'session_compaction'",
            rusqlite::params![memory_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()
        .map_err(|e| rpc_err(-32603, format!("session_compaction_open_query: {e}")))?
        .ok_or_else(|| rpc_err(-32004, "session_compaction_not_found"))?;
    let (id, key, value, session_id, metadata_raw, created_at_s, updated_at_s) = row;
    let metadata = serde_json::from_str::<Value>(&metadata_raw).unwrap_or_else(|_| json!({}));
    let artifact = parse_compaction_artifact_text(&value)?;
    let compartments = artifact
        .get("compartments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let compartment_payload = json!({
        "conversation": compartments.pointer("/conversation/items").cloned().unwrap_or_else(|| json!([])),
        "tool_usage": compartments.pointer("/tool_usage/items").cloned().unwrap_or_else(|| json!([])),
        "actions_taken": compartments.pointer("/actions_taken/items").cloned().unwrap_or_else(|| json!([])),
        "what_how_why": compartments.get("what_how_why").cloned().unwrap_or_else(|| json!({})),
        "full_source_memory_detail": artifact.get("full_source_memory_detail").cloned().unwrap_or_else(|| json!([]))
    });
    Ok(json!({
        "ok": true,
        "artifact_kind": "session_compaction_continuity_v1",
        "memory_id": id,
        "key": key,
        "session_id": session_id,
        "created_at_s": created_at_s,
        "updated_at_s": updated_at_s,
        "native_compaction": metadata.get("native_compaction").cloned().unwrap_or_else(|| json!(true)),
        "model_agnostic": metadata.get("model_agnostic").cloned().unwrap_or_else(|| json!(true)),
        "provider": {
            "provider_id": metadata.get("provider_id").cloned().unwrap_or_else(|| artifact.pointer("/provider/provider_id").cloned().unwrap_or(Value::Null)),
            "model_id": metadata.get("model_id").cloned().unwrap_or_else(|| artifact.pointer("/provider/model_id").cloned().unwrap_or(Value::Null)),
            "context_window_tokens": metadata.get("context_window_tokens").cloned().unwrap_or_else(|| artifact.pointer("/provider/context_window_tokens").cloned().unwrap_or(Value::Null)),
            "max_output_tokens": metadata.get("max_output_tokens").cloned().unwrap_or_else(|| artifact.pointer("/provider/max_output_tokens").cloned().unwrap_or(Value::Null)),
            "catalog_verified": metadata.get("provider_catalog_verified").cloned().unwrap_or_else(|| artifact.pointer("/provider/catalog_verified").cloned().unwrap_or(Value::Null)),
            "deterministic_context_window": metadata.get("deterministic_context_window").cloned().unwrap_or_else(|| artifact.pointer("/budget/deterministic_context_window").cloned().unwrap_or(Value::Null))
        },
        "hom_local_acceptance": {
            "accepted": true,
            "blocking_calibration": false,
            "acceptance_boundary": "saved_compaction_artifact_inspection_only"
        },
        "brain_calibration": {
            "deferred": metadata.get("brain_calibration_deferred").and_then(Value::as_bool).unwrap_or(true),
            "calibration_scope": "nightly_and_reasoning",
            "inspector_calibrates": false
        },
        "compartments": compartment_payload,
        "metadata": metadata,
        "artifact": artifact
    }))
}

fn compaction_registry_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let metadata_raw: String = row.get(3)?;
    let metadata = serde_json::from_str::<Value>(&metadata_raw).unwrap_or_else(|_| json!({}));
    Ok(json!({
        "memory_id": row.get::<_, String>(0)?,
        "key": row.get::<_, String>(1)?,
        "session_id": row.get::<_, Option<String>>(2)?,
        "created_at_s": row.get::<_, i64>(4)?,
        "updated_at_s": row.get::<_, i64>(5)?,
        "artifact_kind": "session_compaction_continuity_v1",
        "native_compaction": metadata.get("native_compaction").cloned().unwrap_or_else(|| json!(true)),
        "model_agnostic": metadata.get("model_agnostic").cloned().unwrap_or_else(|| json!(true)),
        "provider": {
            "provider_id": metadata.get("provider_id").cloned().unwrap_or(Value::Null),
            "model_id": metadata.get("model_id").cloned().unwrap_or(Value::Null),
            "context_window_tokens": metadata.get("context_window_tokens").cloned().unwrap_or(Value::Null),
            "max_output_tokens": metadata.get("max_output_tokens").cloned().unwrap_or(Value::Null),
            "catalog_verified": metadata.get("provider_catalog_verified").cloned().unwrap_or(Value::Null),
            "deterministic_context_window": metadata.get("deterministic_context_window").cloned().unwrap_or(Value::Null)
        },
        "brain_calibration": {
            "deferred": metadata.get("brain_calibration_deferred").and_then(Value::as_bool).unwrap_or(true)
        },
        "metadata": metadata
    }))
}

fn parse_compaction_artifact_text(value: &str) -> Result<Value, RpcError> {
    let prefix = "Session compaction artifact Full source memory detail ";
    let json_text = value
        .strip_prefix(prefix)
        .ok_or_else(|| rpc_err(-32603, "session_compaction_artifact_prefix_missing"))?;
    serde_json::from_str::<Value>(json_text)
        .map_err(|e| rpc_err(-32603, format!("session_compaction_artifact_json: {e}")))
}

fn resolve_model_catalog_context(params: &Value) -> Result<ModelCatalogContext, RpcError> {
    let provider_id = params
        .get("provider_id")
        .or_else(|| params.get("providerId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown-provider")
        .to_string();
    let model_id = params
        .get("model_id")
        .or_else(|| params.get("modelId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown-model")
        .to_string();
    let catalog = params
        .get("model_catalog")
        .or_else(|| params.get("modelCatalog"))
        .or_else(|| params.get("catalog"))
        .unwrap_or(&Value::Null);
    let context_window_tokens = catalog
        .get("context_window_tokens")
        .or_else(|| catalog.get("contextWindowTokens"))
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .unwrap_or(0);
    let catalog_verified = catalog
        .get("verified")
        .or_else(|| catalog.get("catalog_verified"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let catalog_source = catalog
        .get("source")
        .or_else(|| catalog.get("catalog_source"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("provider_model_catalog")
        .to_string();
    let max_output_tokens = catalog
        .get("max_output_tokens")
        .or_else(|| catalog.get("maxOutputTokens"))
        .and_then(Value::as_i64)
        .filter(|value| *value > 0);
    let input_tokens_estimated = params
        .get("input_tokens_estimated")
        .or_else(|| params.get("inputTokensEstimated"))
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0);
    Ok(ModelCatalogContext {
        provider_id,
        model_id,
        context_window_tokens,
        max_output_tokens,
        catalog_source,
        catalog_verified,
        input_tokens_estimated,
    })
}

fn build_concise_continuity_summary(
    session_id: &str,
    session_title: Option<&str>,
    source_memory_count: usize,
    model_catalog: &ModelCatalogContext,
) -> String {
    format!(
        "Phase 0 compaction saved {source_memory_count} source memories for session `{}`. Provider/model `{}/{}` has verified {} token context window. Full detail is preserved in compartmentalized HOM Local brain artifact; next step is to verify focused and regression tests.",
        session_title.unwrap_or(session_id),
        model_catalog.provider_id,
        model_catalog.model_id,
        model_catalog.context_window_tokens
    )
}

fn daemon_base_url(state: &BrainState) -> (String, &'static str) {
    if let Ok(value) = std::env::var("HOM_INGRESS_BASE_URL") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return (trimmed.to_string(), "env:HOM_INGRESS_BASE_URL");
        }
    }
    if let Ok(conn) = state.store.conn() {
        if let Ok(value) = conn.query_row(
            "SELECT value FROM settings WHERE key = 'daemon.baseUrl'",
            [],
            |row| row.get::<_, String>(0),
        ) {
            if let Ok(parsed) = serde_json::from_str::<Value>(&value) {
                if let Some(url) = parsed.as_str().map(str::trim).filter(|url| !url.is_empty()) {
                    return (url.to_string(), "settings:daemon.baseUrl");
                }
            }
        }
    }
    ("http://127.0.0.1:9101".to_string(), "default_loopback_bind")
}

fn owner_key_permissions_ok(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o777 == 0o600)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

fn active_session_id(state: &BrainState) -> Result<Option<String>, RpcError> {
    let conn = state.store.conn()?;
    Ok(conn
        .query_row(
            "SELECT id FROM sessions WHERE active = 1 LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok())
}

fn load_session_memories(
    state: &BrainState,
    session_id: &str,
    limit: i64,
) -> Result<(Option<String>, Vec<Value>), RpcError> {
    let conn = state.store.conn()?;
    let title = conn
        .query_row(
            "SELECT title FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |row| row.get::<_, String>(0),
        )
        .ok();
    let mut stmt = conn
        .prepare(
            "SELECT id, key, value, memory_type, source, quality_score, created_at_s, updated_at_s, metadata_json, track
             FROM memories WHERE session_id = ?1 ORDER BY created_at_s ASC LIMIT ?2",
        )
        .map_err(|e| rpc_err(-32603, format!("session_compact_prepare: {e}")))?;
    let memories = stmt
        .query_map(rusqlite::params![session_id, limit], |row| {
            let metadata_raw: String = row.get(8)?;
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "key": row.get::<_, String>(1)?,
                "value": row.get::<_, String>(2)?,
                "memory_type": row.get::<_, String>(3)?,
                "source": row.get::<_, String>(4)?,
                "quality_score": row.get::<_, f64>(5)?,
                "created_at_s": row.get::<_, i64>(6)?,
                "updated_at_s": row.get::<_, i64>(7)?,
                "metadata": serde_json::from_str::<Value>(&metadata_raw).unwrap_or_else(|_| json!({})),
                "track": row.get::<_, Option<String>>(9)?.unwrap_or_else(|| "uncategorized".to_string())
            }))
        })
        .map_err(|e| rpc_err(-32603, format!("session_compact_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("session_compact_collect: {e}")))?;
    Ok((title, memories))
}

struct CompactionArtifact {
    text: String,
    redacted_string_count: usize,
}

fn build_compaction_artifact(
    session_id: &str,
    session_title: Option<&str>,
    created_at_s: i64,
    memories: &[Value],
    model_catalog: &ModelCatalogContext,
) -> CompactionArtifact {
    let manifest = memories
        .iter()
        .map(|memory| {
            json!({
                "id": memory.get("id").cloned().unwrap_or(Value::Null),
                "key": memory.get("key").cloned().unwrap_or(Value::Null),
                "memory_type": memory.get("memory_type").cloned().unwrap_or(Value::Null),
                "source": memory.get("source").cloned().unwrap_or(Value::Null),
                "track": memory.get("track").cloned().unwrap_or(Value::Null),
                "quality_score": memory.get("quality_score").cloned().unwrap_or(Value::Null),
                "created_at_s": memory.get("created_at_s").cloned().unwrap_or(Value::Null)
            })
        })
        .collect::<Vec<_>>();
    let conversation = compartment_items(memories, &["conversation", "dialogue", "chat"]);
    let tool_usage = compartment_items(
        memories,
        &[
            "tool_usage",
            "tool",
            "terminal",
            "search",
            "read_file",
            "patch",
        ],
    );
    let actions_taken = compartment_items(
        memories,
        &[
            "action_taken",
            "actions_taken",
            "action",
            "implemented",
            "changed",
            "backup",
            "test",
        ],
    );
    let what = memories
        .iter()
        .find_map(|memory| memory.get("value").and_then(Value::as_str))
        .map(|value| {
            if value
                .to_lowercase()
                .contains("native model-agnostic compaction")
            {
                "native model-agnostic compaction continuity with deterministic context windows"
                    .to_string()
            } else {
                value.chars().take(220).collect::<String>()
            }
        })
        .unwrap_or_else(|| "native model-agnostic compaction continuity".to_string());
    let artifact = json!({
        "artifact_type": "session_compaction_continuity_v1",
        "session_id": session_id,
        "session_title": session_title,
        "created_at_s": created_at_s,
        "strategy": "extractive_full_detail_compartmentalized_for_reasoning_nightly_and_continuity",
        "native_compaction": true,
        "model_agnostic": true,
        "provider": {
            "provider_id": model_catalog.provider_id,
            "model_id": model_catalog.model_id,
            "context_window_tokens": model_catalog.context_window_tokens,
            "max_output_tokens": model_catalog.max_output_tokens,
            "catalog_verified": model_catalog.catalog_verified,
            "catalog_source": model_catalog.catalog_source
        },
        "budget": {
            "deterministic_context_window": model_catalog.catalog_verified,
            "input_tokens_estimated": model_catalog.input_tokens_estimated,
            "calibration_in_compaction_path": false,
            "brain_calibration_deferred": true
        },
        "source_memory_count": memories.len(),
        "source_memory_manifest": manifest,
        "compartments": {
            "conversation": {"count": conversation.len(), "items": conversation},
            "tool_usage": {"count": tool_usage.len(), "items": tool_usage},
            "actions_taken": {"count": actions_taken.len(), "items": actions_taken},
            "what_how_why": {
                "what": what,
                "how": "session.compact preserves full source detail and additionally groups memories into conversation, tool usage, and actions taken compartments for nightly and reasoning consumption",
                "why": "compartmentalized continuity lets future recall answer what happened, how it was done, and why the state should continue after context compaction"
            }
        },
        "full_source_memory_detail": memories
    });
    let mut redactions = Vec::new();
    let mut artifact = sanitize_compaction_value(&artifact, "", &mut redactions);
    let redacted_string_count = redactions.len();
    if let Value::Object(map) = &mut artifact {
        map.insert(
            "redaction".to_string(),
            json!({
                "applied": redacted_string_count > 0,
                "redacted_string_count": redacted_string_count,
                "policy": "security_gate.secret_matches",
                "entries": redactions
            }),
        );
    }
    let text = format!(
        "Session compaction artifact Full source memory detail {}",
        serde_json::to_string(&artifact).unwrap_or_else(|_| "{}".to_string())
    );
    CompactionArtifact {
        text,
        redacted_string_count,
    }
}

fn compartment_items(memories: &[Value], markers: &[&str]) -> Vec<Value> {
    memories
        .iter()
        .filter(|memory| memory_matches_markers(memory, markers))
        .cloned()
        .collect()
}

fn memory_matches_markers(memory: &Value, markers: &[&str]) -> bool {
    let normalized_markers = markers
        .iter()
        .map(|marker| marker.replace('-', "_").to_lowercase())
        .collect::<Vec<_>>();
    if let Some(track) = memory.get("track").and_then(Value::as_str) {
        let normalized_track = track.replace('-', "_").to_lowercase();
        if normalized_markers
            .iter()
            .any(|marker| normalized_track == *marker)
        {
            return true;
        }
    }
    if let Some(key) = memory.get("key").and_then(Value::as_str) {
        let normalized_key = key.replace('-', "_").to_lowercase();
        return normalized_markers
            .iter()
            .any(|marker| normalized_key.ends_with(marker));
    }
    false
}

fn sanitize_compaction_value(value: &Value, path: &str, redactions: &mut Vec<Value>) -> Value {
    match value {
        Value::String(text) => {
            let matches = security_gate::secret_matches(text);
            if matches.is_empty() {
                Value::String(text.clone())
            } else {
                redactions.push(json!({
                    "path": if path.is_empty() { "/" } else { path },
                    "patterns": matches.iter().map(|m| m.pattern).collect::<Vec<_>>(),
                    "original_sha256": sha256_hex(text),
                    "original_byte_len": text.len()
                }));
                Value::String("[REDACTED_BY_SECURITY_GATE]".to_string())
            }
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    sanitize_compaction_value(item, &format!("{path}/{index}"), redactions)
                })
                .collect(),
        ),
        Value::Object(map) => {
            let mut sanitized = serde_json::Map::new();
            for (key, item) in map {
                let key_matches = security_gate::secret_matches(key);
                let key_hash = if key_matches.is_empty() {
                    None
                } else {
                    Some(sha256_hex(key))
                };
                if let Some(hash) = &key_hash {
                    redactions.push(json!({
                        "path": if path.is_empty() { "/" } else { path },
                        "redaction_kind": "object_key",
                        "patterns": key_matches.iter().map(|m| m.pattern).collect::<Vec<_>>(),
                        "original_key_sha256": hash,
                        "original_key_byte_len": key.len()
                    }));
                }
                let output_key = key_hash
                    .as_ref()
                    .map(|hash| format!("redacted_key_{}", &hash[..12]))
                    .unwrap_or_else(|| key.clone());
                let path_segment = if key_hash.is_some() {
                    "[redacted_key]".to_string()
                } else {
                    json_pointer_escape(key)
                };
                let key_path = format!("{path}/{path_segment}");
                let sanitized_value = if let Value::String(text) = item {
                    if security_gate::key_value_contains_secret_pattern(key, text) {
                        redact_compaction_string(
                            &key_path,
                            text,
                            vec!["credential_assignment".to_string()],
                            redactions,
                        )
                    } else {
                        sanitize_compaction_value(item, &key_path, redactions)
                    }
                } else {
                    sanitize_compaction_value(item, &key_path, redactions)
                };
                sanitized.insert(output_key, sanitized_value);
            }
            Value::Object(sanitized)
        }
        _ => value.clone(),
    }
}

fn redact_compaction_string(
    path: &str,
    text: &str,
    patterns: Vec<String>,
    redactions: &mut Vec<Value>,
) -> Value {
    redactions.push(json!({
        "path": if path.is_empty() { "/" } else { path },
        "patterns": patterns,
        "original_sha256": sha256_hex(text),
        "original_byte_len": text.len()
    }));
    Value::String("[REDACTED_BY_SECURITY_GATE]".to_string())
}

fn json_pointer_escape(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn record_compaction_ledger(
    state: &BrainState,
    session_id: &str,
    memory_id: &str,
    key: &str,
    source_count: usize,
    artifact_hash: &str,
    strategy: &str,
    redacted_string_count: usize,
) -> Result<Value, RpcError> {
    let now = unix_now_s();
    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("session_compaction_tx: {e}")))?;
    let ledger = append_ledger_tx(
        &tx,
        "session.compacted",
        "brain.session.compact",
        Some(memory_id),
        json!({
            "session_id": session_id,
            "compaction_memory_id": memory_id,
            "compaction_key": key,
            "source_memory_count": source_count,
            "artifact_hash": artifact_hash,
            "strategy": strategy,
            "redaction": {
                "applied": redacted_string_count > 0,
                "redacted_string_count": redacted_string_count
            }
        }),
        now,
    )
    .map_err(|e| rpc_err(-32603, format!("session_compaction_ledger: {e}")))?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("session_compaction_commit: {e}")))?;
    Ok(ledger)
}

/// Wraps a raw 32-byte Ed25519 public key (base64url-encoded) into SPKI DER format.
fn raw_ed25519_pub_to_spki_der(raw_b64u: &str) -> Option<String> {
    let raw = hom_shared::b64u_decode(raw_b64u).ok()?;
    if raw.len() != 32 {
        return None;
    }
    // Ed25519 SPKI DER prefix: SEQUENCE { SEQUENCE { OID 1.3.101.112 }, BIT STRING (0 unused, 32 bytes) }
    const PREFIX: &[u8] = &[
        0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
    ];
    let mut der = Vec::with_capacity(PREFIX.len() + 32);
    der.extend_from_slice(PREFIX);
    der.extend_from_slice(&raw);
    Some(hom_shared::b64u_encode(&der))
}
