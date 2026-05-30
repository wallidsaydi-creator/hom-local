use hom_shared::{RpcError, canonical_json, rpc_err, sha256_hex};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::BrainState;
use crate::db::storage::unix_now_s;
use crate::services::memory;

pub fn snapshot(state: &BrainState) -> Result<Value, RpcError> {
    let now = unix_now_s();
    let ledger = state
        .store
        .verify_ledger()
        .unwrap_or_else(|_| json!({"valid": false}));
    let security = memory::security_saber_dry_run()?;
    let mut warnings = Vec::new();

    collect_ledger_warnings(&ledger, now, &mut warnings);
    collect_security_warnings(&security, now, &mut warnings);

    persist_warnings(state, &warnings)?;
    Ok(json!({
        "ok": true,
        "state": if warnings.iter().any(|warning| warning["severity"] == "critical") {
            "degraded"
        } else {
            "available"
        },
        "warnings": warnings,
        "warning_count": warnings.len(),
        "sources": {
            "ledger": "ledger.verify",
            "security": "security.saber_dry_run"
        },
        "scanned_at_s": now
    }))
}

fn collect_ledger_warnings(ledger: &Value, now: i64, warnings: &mut Vec<Value>) {
    let current_valid = ledger
        .pointer("/current_epoch/valid")
        .and_then(Value::as_bool);
    let historical_status = ledger
        .pointer("/historical_segment/status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let self_modified_count = ledger
        .get("self_modified_count")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if ledger.get("valid").and_then(Value::as_bool) == Some(false)
        || ledger.get("operational_valid").and_then(Value::as_bool) == Some(false)
    {
        warnings.push(warning(
            "stale_ledger",
            "ledger",
            "stale",
            "critical",
            "ledger verification is not valid".to_string(),
            ledger,
            now,
        ));
    }
    if current_valid == Some(false) {
        warnings.push(warning(
            "current_ledger_epoch_invalid",
            "ledger",
            "invalid",
            "critical",
            "current ledger epoch hash-chain is not valid".to_string(),
            ledger,
            now,
        ));
    }
    if historical_status == "invalid_uncertified" {
        warnings.push(warning(
            "historical_ledger_invalid_uncertified",
            "ledger",
            "unproven",
            "critical",
            "historical ledger invalidity has no segmented repair certificate".to_string(),
            ledger,
            now,
        ));
    } else if historical_status == "invalid_preserved" {
        warnings.push(warning(
            "historical_ledger_invalid_preserved",
            "ledger",
            "degraded",
            "warning",
            "historical ledger invalidity is preserved behind a repair certificate".to_string(),
            ledger,
            now,
        ));
    }
    if self_modified_count > 0 && ledger.get("valid").and_then(Value::as_bool) == Some(true) {
        warnings.push(warning(
            "ledger_self_modified_preserved",
            "ledger",
            "available",
            "warning",
            format!("ledger has {self_modified_count} self-modified event(s) tracked on the meta-ledger; chain integrity is preserved"),
            ledger,
            now,
        ));
    }
}

fn collect_security_warnings(security: &Value, now: i64, warnings: &mut Vec<Value>) {
    let failed = security
        .get("findings")
        .and_then(Value::as_array)
        .map(|findings| {
            findings
                .iter()
                .any(|finding| finding.get("passed").and_then(Value::as_bool) != Some(true))
        })
        .unwrap_or(true);
    if failed {
        warnings.push(warning(
            "security_saber_dry_run_failed",
            "security",
            "degraded",
            "critical",
            "DARPA SABER security dry run has failing or missing vectors".to_string(),
            security,
            now,
        ));
    }
}

fn persist_warnings(state: &BrainState, warnings: &[Value]) -> Result<(), RpcError> {
    let conn = state.store.conn()?;
    for warning in warnings {
        let warning_id = warning
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("unknown_warning");
        let evidence_hash = warning
            .get("evidence_hash")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let now = warning
            .pointer("/diagnostic/last_checked_at_s")
            .and_then(Value::as_i64)
            .unwrap_or_else(unix_now_s);
        let existing_first_seen: Option<i64> = conn
            .query_row(
                "SELECT first_seen_at_s FROM monitoring_events
                 WHERE warning_id = ?1 AND evidence_hash = ?2",
                rusqlite::params![warning_id, evidence_hash],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| rpc_err(-32603, format!("monitoring_lookup: {e}")))?;
        let first_seen = existing_first_seen.unwrap_or(now);
        conn.execute(
            "INSERT INTO monitoring_events
             (id, warning_id, subsystem, state, severity, evidence_hash, payload_json,
              first_seen_at_s, last_seen_at_s)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(warning_id, evidence_hash) DO UPDATE SET
              last_seen_at_s = ?9,
              payload_json = ?7",
            rusqlite::params![
                Uuid::new_v4().to_string(),
                warning_id,
                warning
                    .get("subsystem")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
                warning
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
                warning
                    .get("severity")
                    .and_then(Value::as_str)
                    .unwrap_or("warning"),
                evidence_hash,
                serde_json::to_string(warning)
                    .map_err(|e| rpc_err(-32603, format!("monitoring_payload_json: {e}")))?,
                first_seen,
                now
            ],
        )
        .map_err(|e| rpc_err(-32603, format!("monitoring_upsert: {e}")))?;
    }
    Ok(())
}

fn warning(
    id: &str,
    subsystem: &str,
    state: &str,
    severity: &str,
    message: String,
    evidence: &Value,
    now: i64,
) -> Value {
    let evidence_hash = sha256_hex(
        &canonical_json(evidence)
            .unwrap_or_else(|_| serde_json::to_string(evidence).unwrap_or_default()),
    );
    json!({
        "id": id,
        "subsystem": subsystem,
        "state": state,
        "severity": severity,
        "message": message,
        "evidence_hash": evidence_hash,
        "evidence": evidence,
        "inspectorTarget": {
            "kind": "monitoring_warning",
            "id": id
        },
        "diagnostic": {
            "source": "backend_monitoring_snapshot",
            "last_checked_at_s": now
        }
    })
}

pub fn benchmark_result_artifact(benchmark_id: &str, payload: &Value, created_at_s: i64) -> Value {
    json!({
        "artifact_kind": "benchmark_result_v1",
        "benchmark_id": benchmark_id,
        "benchmark_kind": payload.get("benchmark_kind").cloned().unwrap_or_else(|| json!("unknown")),
        "subject_id": payload.get("subject_id").cloned().unwrap_or(Value::Null),
        "subject_kind": payload.get("subject_kind").cloned().unwrap_or(Value::Null),
        "scenario": payload.get("scenario").cloned().unwrap_or(Value::Null),
        "expected_contract": payload.get("expected_contract").cloned().unwrap_or_else(|| json!({})),
        "actual_contract": payload.get("actual_contract").cloned().unwrap_or_else(|| json!({})),
        "status": payload.get("status").cloned().unwrap_or_else(|| json!("unknown")),
        "severity": payload.get("severity").cloned().unwrap_or_else(|| json!("unknown")),
        "passes": payload.get("passes").cloned().unwrap_or_else(|| json!([])),
        "failures": payload.get("failures").cloned().unwrap_or_else(|| json!([])),
        "evidence": payload.get("evidence").cloned().unwrap_or_else(|| json!([])),
        "started_at_s": payload.get("started_at_s").cloned().unwrap_or(json!(created_at_s)),
        "finished_at_s": payload.get("finished_at_s").cloned().unwrap_or(json!(created_at_s)),
        "duration_ms": payload.get("duration_ms").cloned().unwrap_or(json!(0)),
        "created_by": payload.get("created_by").cloned().unwrap_or_else(|| json!("unknown")),
        "inspector_target": payload.get("inspector_target").cloned().unwrap_or_else(|| json!({"kind": "benchmark_result", "id": benchmark_id})),
        "created_at_s": created_at_s,
        "source": "ledger_events.benchmark.result"
    })
}

pub fn benchmark_registry_entry(artifacts: Vec<Value>) -> Value {
    json!({
        "ok": true,
        "artifact_kind": "benchmark_registry_entry_v1",
        "benchmarks": artifacts,
        "total": artifacts.len(),
        "source": "ledger_events.benchmark.result"
    })
}
