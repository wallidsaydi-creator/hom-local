use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use hom_shared::{RpcError, load_known_clients, rpc_err, sha256_hex};
use rusqlite::params;
use serde_json::{Map, Value, json};

use crate::services::permissions;

pub async fn get(state: &crate::BrainState) -> Result<Value, RpcError> {
    let rows = settings_rows(state)?;

    let mut quality = json!({"autoScore": true, "minScore": 0.3});
    let mut security = json!({"scanSecrets": true, "canaryDetection": true});
    let mut recall = json!({"defaultMode": "auto", "maxResults": 20});
    let mut storage = json!({"autoCompact": true, "compactionMode": "ask", "maxMemoryMb": 500});
    let mut nightly = json!({"enabled": true, "hour": 3});
    let mut embeddings = json!({"model": "local", "dimensions": 384});
    let mut ledger_settings = json!({"verificationEnabled": true, "hashAlgorithm": "sha256"});
    let mut personalization = json!({"theme": Value::Null, "accentColor": Value::Null});
    let mut retrieval_calibration = json!({"profile": "default", "maxResults": 20});
    let mut provider = json!({"selectedProviderId": Value::Null, "selectedModelId": Value::Null, "reasoningEffort": "medium"});
    let mut git = json!({
        "branchPrefix": Value::Null,
        "commitInstructions": Value::Null,
        "mergeMethod": "merge",
        "draftPr": false,
        "forcePush": false,
        "autoDeleteLimit": Value::Null,
        "deleteOldWorktrees": false,
        "prInstructions": Value::Null
    });

    apply_rows(
        &rows,
        &mut quality,
        &mut security,
        &mut recall,
        &mut storage,
        &mut nightly,
        &mut embeddings,
        &mut ledger_settings,
        &mut personalization,
        &mut retrieval_calibration,
        &mut provider,
        &mut git,
    );

    let (memory_count, seed_pack_count, source_counts) = corpus_state(state)?;
    let ledger_verification = state.store.cached_ledger_verification();
    let permissions_payload = permissions::load_state(state)?.to_value();
    let identity = identity_payload(state);
    let config = rows
        .iter()
        .map(|(key, value, updated_at_s)| {
            json!({
                "key": key,
                "value": value,
                "updatedAt": updated_at_s
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "ok": true,
        "daemon": {
            "baseUrl": "http://127.0.0.1:9101",
            "brainSocket": state.hom_dir.join("brain.sock").to_string_lossy(),
            "ingressSocket": state.hom_dir.join("ingress.sock").to_string_lossy()
        },
        "auth": {
            "configured": identity.get("owner_key_configured").and_then(Value::as_bool).unwrap_or(false),
            "ownerKeyFingerprint": null
        },
        "identity": identity,
        "enrolledAgents": identity.get("enrolled_agents").cloned().unwrap_or_else(|| json!([])),
        "unenrolledAgents": [],
        "osAwareness": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "family": std::env::consts::FAMILY
        },
        "corpus": {
            "memoryCount": memory_count,
            "seedPackMemories": seed_pack_count,
            "seedPackTotalRows": seed_pack_count,
            "sourceCounts": source_counts
        },
        "ledger": {
            "valid": ledger_verification.get("valid").cloned().unwrap_or_else(|| json!(false)),
            "totalEvents": ledger_verification.get("total_events").cloned().unwrap_or_else(|| json!(0)),
            "checkedEvents": ledger_verification.get("checked_events").cloned().unwrap_or_else(|| json!(0)),
            "headHash": ledger_verification.get("head_hash").cloned().unwrap_or(Value::Null),
            "verifiedHeadHash": ledger_verification.get("verified_head_hash").cloned().unwrap_or(Value::Null),
            "lastVerifiedEventId": ledger_verification.get("last_verified_event_id").cloned().unwrap_or(Value::Null),
            "firstInvalid": ledger_verification.get("first_invalid").cloned().unwrap_or(Value::Null),
            "verificationEnabled": ledger_settings.get("verificationEnabled").cloned().unwrap_or_else(|| json!(true)),
            "hashAlgorithm": ledger_settings.get("hashAlgorithm").cloned().unwrap_or_else(|| json!("sha256")),
            "verification": ledger_verification
        },
        "permissions": permissions_payload,
        "personalization": personalization,
        "retrievalCalibration": retrieval_calibration,
        "config": config,
        "quality": quality,
        "security": security,
        "recall": recall,
        "storage": storage,
        "nightly": nightly,
        "embeddings": embeddings,
        "ledgerSettings": ledger_settings,
        "provider": provider,
        "git": git,
    }))
}

pub fn set(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let settings = params
        .get("settings")
        .ok_or_else(|| rpc_err(-32602, "missing settings object"))?;
    let settings = settings
        .as_object()
        .ok_or_else(|| rpc_err(-32602, "settings must be an object"))?;

    validate_settings(state, settings, confirmation_present(&params))?;
    permissions::enforce_settings_write(state, settings)?;

    let conn = state.store.conn()?;
    let now = now_s();
    for (key, value) in settings {
        let value_str = serde_json::to_string(value)
            .map_err(|e| rpc_err(-32603, format!("settings_value_serialize: {e}")))?;
        conn.execute(
            "INSERT INTO settings (key, value, updated_at_s) VALUES (?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at_s = ?3",
            params![key, value_str, now],
        )
        .map_err(|e| rpc_err(-32603, &format!("settings_set: {e}")))?;
    }

    drop(conn);
    let permission_updates = settings
        .iter()
        .filter(|(key, _)| permissions::is_permission_key(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<String, Value>>();
    if !permission_updates.is_empty() {
        permissions::sync_setting_grants(state, &permission_updates)?;
    }

    Ok(json!({
        "ok": true,
        "settings": settings,
        "updatedAt": now
    }))
}

#[allow(clippy::too_many_arguments)]
fn apply_rows(
    rows: &[(String, String, i64)],
    quality: &mut Value,
    security: &mut Value,
    recall: &mut Value,
    storage: &mut Value,
    nightly: &mut Value,
    embeddings: &mut Value,
    ledger_settings: &mut Value,
    personalization: &mut Value,
    retrieval_calibration: &mut Value,
    provider: &mut Value,
    git: &mut Value,
) {
    for (key, value, _) in rows {
        let parsed: Option<Value> = serde_json::from_str(value).ok();
        let Some(v) = parsed else { continue };
        match key.as_str() {
            k if k.starts_with("quality.") => quality[k.strip_prefix("quality.").unwrap_or("")] = v,
            k if k.starts_with("security.") => {
                security[k.strip_prefix("security.").unwrap_or("")] = v
            }
            k if k.starts_with("recall.") => recall[k.strip_prefix("recall.").unwrap_or("")] = v,
            k if k.starts_with("storage.") => storage[k.strip_prefix("storage.").unwrap_or("")] = v,
            k if k.starts_with("nightly.") => nightly[k.strip_prefix("nightly.").unwrap_or("")] = v,
            k if k.starts_with("embeddings.") => {
                embeddings[k.strip_prefix("embeddings.").unwrap_or("")] = v
            }
            k if k.starts_with("ledger.") => {
                ledger_settings[k.strip_prefix("ledger.").unwrap_or("")] = v
            }
            k if k.starts_with("personalization.") => {
                personalization[k.strip_prefix("personalization.").unwrap_or("")] = v
            }
            k if k.starts_with("retrievalCalibration.") => {
                retrieval_calibration[k.strip_prefix("retrievalCalibration.").unwrap_or("")] = v
            }
            k if k.starts_with("provider.") => {
                provider[k.strip_prefix("provider.").unwrap_or("")] = v
            }
            k if k.starts_with("git.") => git[k.strip_prefix("git.").unwrap_or("")] = v,
            _ => {}
        }
    }
}

fn settings_rows(state: &crate::BrainState) -> Result<Vec<(String, String, i64)>, RpcError> {
    let conn = state.store.conn()?;
    let mut stmt = conn
        .prepare("SELECT key, value, updated_at_s FROM settings")
        .map_err(|e| rpc_err(-32603, &format!("settings_get_prepare: {e}")))?;
    stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })
    .map_err(|e| rpc_err(-32603, &format!("settings_get_query: {e}")))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| rpc_err(-32603, &format!("settings_get_collect: {e}")))
}

fn corpus_state(state: &crate::BrainState) -> Result<(i64, i64, Vec<Value>), RpcError> {
    let conn = state.store.conn()?;
    let memory_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .unwrap_or(0);
    let seed_pack_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE source = 'local-pack'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let mut counts = BTreeMap::<String, i64>::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT source, COUNT(*) FROM memories GROUP BY source ORDER BY COUNT(*) DESC")
    {
        if let Ok(mapped) = stmt.query_map([], |row| {
            let source: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((source, count))
        }) {
            for (source, count) in mapped.filter_map(Result::ok) {
                *counts.entry(public_source_kind(&source)).or_insert(0) += count;
            }
        }
    }
    let source_counts = counts
        .into_iter()
        .map(|(source_kind, count)| json!({"sourceKind": source_kind, "count": count}))
        .collect::<Vec<_>>();
    Ok((memory_count, seed_pack_count, source_counts))
}

fn public_source_kind(source: &str) -> String {
    if source == "local-pack" {
        "seed-pack".to_string()
    } else {
        source.to_string()
    }
}

fn validate_settings(
    state: &crate::BrainState,
    settings: &Map<String, Value>,
    confirmed: bool,
) -> Result<(), RpcError> {
    let permission_updates = settings
        .iter()
        .filter(|(key, _)| permissions::is_permission_key(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<String, Value>>();
    if !permission_updates.is_empty() {
        permissions::validate_permission_settings(state, &permission_updates, confirmed)?;
    }
    for (key, value) in settings {
        reject_secret_like(key, value)?;
        match key.as_str() {
            "quality.autoScore"
            | "security.scanSecrets"
            | "security.canaryDetection"
            | "storage.autoCompact"
            | "nightly.enabled"
            | "ledger.verificationEnabled" => require_bool(key, value)?,
            "quality.minScore" => require_number_range(key, value, 0.0, 1.0)?,
            "recall.defaultMode" => require_string_in(
                key,
                value,
                &[
                    "auto",
                    "strict",
                    "smart",
                    "lineage",
                    "temporal",
                    "text",
                    "reasoning",
                ],
            )?,
            "recall.maxResults" | "retrievalCalibration.maxResults" => {
                require_i64_range(key, value, 1, 100)?
            }
            "storage.maxMemoryMb" => require_i64_range(key, value, 64, 32768)?,
            "storage.compactionMode" => require_string_in(key, value, &["always", "ask", "never"])?,
            "nightly.hour" => require_i64_range(key, value, 0, 23)?,
            "embeddings.model"
            | "personalization.theme"
            | "personalization.accentColor"
            | "retrievalCalibration.profile"
            | "provider.selectedProviderId"
            | "provider.selectedModelId" => require_string_or_null(key, value)?,
            "provider.reasoningEffort" => {
                require_string_in(key, value, &["none", "low", "medium", "high"])?
            }
            "git.branchPrefix" | "git.commitInstructions" | "git.prInstructions" => {
                require_string_or_null(key, value)?
            }
            "git.mergeMethod" => require_string_in(key, value, &["merge", "squash"])?,
            "git.draftPr" | "git.forcePush" | "git.deleteOldWorktrees" => require_bool(key, value)?,
            "git.autoDeleteLimit" => require_i64_range(key, value, 0, 1000)?,
            "embeddings.dimensions" => require_i64_range(key, value, 1, 16384)?,
            "ledger.hashAlgorithm" => require_string_in(key, value, &["sha256"])?,
            _ if permissions::is_permission_key(key) => {}
            _ => return Err(rpc_err(-32602, format!("unknown_setting: {key}"))),
        }
    }
    Ok(())
}

fn identity_payload(state: &crate::BrainState) -> Value {
    let owner_key_path = state.hom_dir.join("keys").join("owner.key");
    let known_clients_path = hom_shared::known_clients_path(&state.hom_dir);
    let enrolled_agents = if known_clients_path.exists() {
        load_known_clients(&known_clients_path)
            .map(|clients| {
                clients
                    .into_iter()
                    .map(|client| {
                        json!({
                            "clientId": client.client_id,
                            "scopes": client.scopes,
                            "publicKeySha256": sha256_hex(&client.client_pub),
                            "enrolled": true
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    json!({
        "owner_key_configured": owner_key_path.exists(),
        "ownerKeyConfigured": owner_key_path.exists(),
        "ownerKeyPath": owner_key_path.display().to_string(),
        "knownClientsPath": known_clients_path.display().to_string(),
        "known_clients_file_exists": known_clients_path.exists(),
        "enrolled_agent_count": enrolled_agents.len(),
        "enrolled_agents": enrolled_agents
    })
}

fn reject_secret_like(key: &str, value: &Value) -> Result<(), RpcError> {
    let serialized = serde_json::to_string(value).unwrap_or_default();
    let lower = serialized.to_ascii_lowercase();
    if lower.contains("sk-")
        || lower.contains("access_token")
        || lower.contains("refresh_token")
        || lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("bearer ")
    {
        return Err(rpc_err(
            -32602,
            format!("setting_secret_material_rejected: {key}"),
        ));
    }
    Ok(())
}

fn require_bool(key: &str, value: &Value) -> Result<(), RpcError> {
    value
        .as_bool()
        .map(|_| ())
        .ok_or_else(|| rpc_err(-32602, format!("{key}_must_be_boolean")))
}

fn require_i64_range(key: &str, value: &Value, min: i64, max: i64) -> Result<(), RpcError> {
    let number = value
        .as_i64()
        .ok_or_else(|| rpc_err(-32602, format!("{key}_must_be_integer")))?;
    if number < min || number > max {
        return Err(rpc_err(-32602, format!("{key}_out_of_range")));
    }
    Ok(())
}

fn require_number_range(key: &str, value: &Value, min: f64, max: f64) -> Result<(), RpcError> {
    let number = value
        .as_f64()
        .ok_or_else(|| rpc_err(-32602, format!("{key}_must_be_number")))?;
    if !(min..=max).contains(&number) {
        return Err(rpc_err(-32602, format!("{key}_out_of_range")));
    }
    Ok(())
}

fn require_string_in(key: &str, value: &Value, allowed: &[&str]) -> Result<(), RpcError> {
    let string = value
        .as_str()
        .ok_or_else(|| rpc_err(-32602, format!("{key}_must_be_string")))?;
    if !allowed.iter().any(|candidate| candidate == &string) {
        return Err(rpc_err(-32602, format!("{key}_invalid_value")));
    }
    Ok(())
}

fn require_string_or_null(key: &str, value: &Value) -> Result<(), RpcError> {
    if value.is_null() || value.as_str().is_some() {
        return Ok(());
    }
    Err(rpc_err(-32602, format!("{key}_must_be_string_or_null")))
}

fn confirmation_present(params: &Value) -> bool {
    params
        .get("confirm_broadening")
        .or_else(|| params.get("confirmBroadening"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || params
            .get("confirmation")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
}

fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
