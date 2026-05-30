use hom_shared::{RpcError, public_fingerprint, rpc_err};
use serde_json::{Value, json};

use crate::BrainState;
use crate::db::storage::unix_now_s;
use crate::services::permissions;

pub async fn status(state: &BrainState) -> Result<Value, RpcError> {
    let now = unix_now_s();
    let ledger_verification = state.store.cached_ledger_verification();
    let (memory_count, source_counts, active_session_id, session_count) = match state.store.conn() {
        Ok(conn) => (
            count(&conn, "memories").unwrap_or(0),
            source_counts(&conn).unwrap_or_default(),
            active_session_id(&conn),
            count(&conn, "sessions").unwrap_or(0),
        ),
        Err(_) => (0, Vec::new(), Value::Null, 0),
    };
    let capabilities = capabilities(state, now, &ledger_verification).unwrap_or_else(|_| {
        vec![capability_state(
            "runtime_status",
            "Runtime Status",
            false,
            Some("capability scan failed"),
            Some("check diagnostics"),
            now,
        )]
    });
    let degraded_subsystems = capabilities
        .iter()
        .filter(|capability| capability["state"] != "available")
        .filter_map(|capability| capability["id"].as_str().map(ToString::to_string))
        .collect::<Vec<_>>();

    Ok(json!({
        "ok": true,
        "daemon": {
            "status": if degraded_subsystems.is_empty() { "ready" } else { "degraded" },
            "version": env!("CARGO_PKG_VERSION"),
            "base_url": "http://127.0.0.1:9101",
            "uptime_s": state.started_at.elapsed().as_secs()
        },
        "identity": owner_identity(state),
        "ledger": {
            "valid": ledger_verification.get("valid").cloned().unwrap_or_else(|| json!(false)),
            "total_events": ledger_verification.get("total_events").cloned().unwrap_or_else(|| json!(0)),
            "checked_events": ledger_verification.get("checked_events").cloned().unwrap_or_else(|| json!(0)),
            "head_hash": ledger_verification.get("head_hash").cloned().unwrap_or(Value::Null),
            "verified_head_hash": ledger_verification.get("verified_head_hash").cloned().unwrap_or(Value::Null),
            "last_verified_event_id": ledger_verification.get("last_verified_event_id").cloned().unwrap_or(Value::Null),
            "first_invalid": ledger_verification.get("first_invalid").cloned().unwrap_or(Value::Null),
            "algorithm": ledger_verification.get("algorithm").cloned().unwrap_or(Value::Null),
            "checked_at_s": ledger_verification.get("checked_at_s").cloned().unwrap_or(Value::Null)
        },
        "memory": {
            "count": memory_count,
            "source_counts": source_counts,
            "index_status": "ready",
            "last_import_id": Value::Null,
            "last_export_id": Value::Null,
            "last_purge_id": Value::Null
        },
        "permissions": permissions::load_state(state)
            .map(|p| p.to_value())
            .unwrap_or_else(|_| json!({"profile": "restricted", "error": "permissions_unavailable"})),
        "session": {
            "active_session_id": active_session_id,
            "persisted_session_count": session_count
        },
        "diagnostics": {
            "last_failure": Value::Null,
            "last_warning": Value::Null,
            "degraded_subsystems": degraded_subsystems
        },
        "capabilities": capabilities
    }))
}

pub fn capability(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let capability_id = params
        .get("capability_id")
        .or_else(|| params.get("capabilityId"))
        .or_else(|| params.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "capability_id_required"))?;
    let now = unix_now_s();
    let ledger_verification = state.store.cached_ledger_verification();
    capabilities(state, now, &ledger_verification)?
        .into_iter()
        .find(|capability| capability["id"] == capability_id)
        .map(|capability| {
            json!({
                "ok": true,
                "capability_id": capability_id,
                "state": capability["state"],
                "reason": capability["reason"],
                "required_action": capability["required_action"],
                "checks": capability_checks(state, capability_id),
                "last_checked_at": now
            })
        })
        .ok_or_else(|| rpc_err(-32602, "unknown_capability"))
}

fn capabilities(
    state: &BrainState,
    now: i64,
    ledger_verification: &Value,
) -> Result<Vec<Value>, RpcError> {
    let db_ok = state.store.conn().is_ok();
    let ledger_pending = ledger_verification
        .get("pending")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let ledger_valid = ledger_verification
        .get("valid")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let ledger_reason = if ledger_pending {
        Some("ledger verification pending; cache not yet populated".to_string())
    } else {
        ledger_verification
            .get("first_invalid")
            .filter(|value| !value.is_null())
            .map(|value| format!("ledger verification failed: {value}"))
    };
    Ok(vec![
        capability_state(
            "memory_save",
            "Save Memory",
            db_ok,
            (!db_ok).then_some("database unavailable"),
            None,
            now,
        ),
        capability_state(
            "memory_recall",
            "Recall",
            db_ok,
            (!db_ok).then_some("database unavailable"),
            None,
            now,
        ),
        capability_state(
            "memory_open",
            "Open Memory",
            db_ok,
            (!db_ok).then_some("database unavailable"),
            None,
            now,
        ),
        capability_state(
            "ledger_verify",
            "Ledger Verification",
            db_ok && (ledger_valid || ledger_pending),
            if !db_ok {
                Some("database unavailable")
            } else if ledger_pending {
                Some("ledger verification pending; cache not yet populated")
            } else {
                ledger_reason.as_deref()
            },
            Some("inspect ledger hash chain and append paths"),
            now,
        ),
        capability_state(
            "session_compact",
            "Compact Session",
            db_ok,
            (!db_ok).then_some("database unavailable"),
            None,
            now,
        ),
        capability_state(
            "settings_read",
            "Read Settings",
            db_ok,
            (!db_ok).then_some("database unavailable"),
            None,
            now,
        ),
        capability_state(
            "settings_write",
            "Write Settings",
            db_ok,
            (!db_ok).then_some("database unavailable"),
            None,
            now,
        ),
        capability_state(
            "nightly_run",
            "Nightly",
            db_ok,
            (!db_ok).then_some("database unavailable"),
            None,
            now,
        ),
        capability_state(
            "reasoning",
            "Reasoning",
            db_ok,
            (!db_ok).then_some("database unavailable"),
            None,
            now,
        ),
        capability_state(
            "import_export",
            "Import Export",
            db_ok,
            (!db_ok).then_some("database unavailable"),
            None,
            now,
        ),
        capability_state(
            "backup_purge",
            "Backup Purge",
            db_ok,
            (!db_ok).then_some("database unavailable"),
            None,
            now,
        ),
    ])
}

fn capability_state(
    id: &str,
    label: &str,
    available: bool,
    reason: Option<&str>,
    action: Option<&str>,
    now: i64,
) -> Value {
    json!({
        "id": id,
        "label": label,
        "state": if available { "available" } else { "error" },
        "reason": if available { Value::Null } else { json!(reason.unwrap_or("unavailable")) },
        "required_action": if available { Value::Null } else { json!(action.unwrap_or("check diagnostics")) },
        "last_checked_at": now
    })
}

fn capability_checks(state: &BrainState, capability_id: &str) -> Value {
    let db_ok = state.store.conn().is_ok();
    match capability_id {
        "ledger_verify" => {
            let verification = state.store.cached_ledger_verification();
            let valid = verification
                .get("valid")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let pending = verification
                .get("pending")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            json!([
                {"name": "database", "passed": db_ok, "detail": if db_ok { "SQLite connection opened" } else { "SQLite connection failed" }},
                {"name": "hash_chain", "passed": valid || pending, "detail": verification}
            ])
        }
        "session_compact" => json!([
            {"name": "database", "passed": db_ok, "detail": if db_ok { "SQLite connection opened" } else { "SQLite connection failed" }},
            {"name": "memory_save", "passed": db_ok, "detail": "Compaction saves through LocalStore::save_memory"},
            {"name": "memory_recall", "passed": db_ok, "detail": "Compaction artifacts are recalled through memory.recall"}
        ]),
        _ => json!([
            {"name": "database", "passed": db_ok, "detail": if db_ok { "SQLite connection opened" } else { "SQLite connection failed" }},
            {"name": "brain_dispatch", "passed": true, "detail": "Capability is implemented by a brain worker method"}
        ]),
    }
}

fn count(conn: &rusqlite::Connection, table: &str) -> Result<i64, RpcError> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    conn.query_row(&sql, [], |row| row.get(0))
        .map_err(|e| rpc_err(-32603, format!("runtime_count_{table}: {e}")))
}

fn source_counts(conn: &rusqlite::Connection) -> Result<Vec<Value>, RpcError> {
    let mut stmt = conn
        .prepare("SELECT source, COUNT(*) FROM memories GROUP BY source ORDER BY COUNT(*) DESC")
        .map_err(|e| rpc_err(-32603, format!("runtime_source_counts_prepare: {e}")))?;
    stmt.query_map([], |row| {
        Ok(json!({
            "source_kind": row.get::<_, String>(0)?,
            "count": row.get::<_, i64>(1)?
        }))
    })
    .map_err(|e| rpc_err(-32603, format!("runtime_source_counts_query: {e}")))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| rpc_err(-32603, format!("runtime_source_counts_collect: {e}")))
}

fn owner_identity(state: &BrainState) -> Value {
    let owner_key_path = state.hom_dir.join("keys").join("owner.key");
    let public_key = std::fs::read_to_string(&owner_key_path)
        .ok()
        .and_then(|content| {
            if content.starts_with('{') {
                serde_json::from_str::<Value>(&content)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("public_key")
                            .and_then(Value::as_str)
                            .map(ToString::to_string)
                    })
            } else {
                Some(content.trim().to_string())
            }
        });
    let fingerprint = public_key
        .as_deref()
        .and_then(|key| public_fingerprint(key).ok())
        .map(|value| format!("sha256:{value}"));
    json!({
        "configured": public_key.is_some(),
        "unlocked": public_key.is_some(),
        "fingerprint": fingerprint,
        "active_grant_id": if public_key.is_some() { json!("owner.local") } else { Value::Null }
    })
}

fn active_session_id(conn: &rusqlite::Connection) -> Value {
    conn.query_row(
        "SELECT id FROM sessions WHERE active = 1 LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .map(Value::String)
    .unwrap_or(Value::Null)
}
