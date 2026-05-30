use hom_shared::{RpcError, rpc_err};
use serde_json::{Value, json};

use crate::BrainState;
use crate::db::storage::{record_mutation_tx, unix_now_s};

pub fn verify(state: &BrainState) -> Result<Value, RpcError> {
    let result = state
        .store
        .verify_ledger()
        .map_err(|error| rpc_err(-32603, format!("ledger_verify: {error}")))?;
    state.store.refresh_ledger_cache();
    Ok(result)
}

pub fn repair_segmented(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let actor = params
        .get("actor")
        .and_then(Value::as_str)
        .unwrap_or("brain.ledger_repair");
    let reason = params
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("segmented repair requested for preserved historical hash-chain invalidity");
    let confirmation = params
        .get("confirm_segmented_repair")
        .or_else(|| params.get("confirmSegmentedRepair"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !confirmation {
        return Err(RpcError {
            code: -32602,
            message: "segmented_repair_confirmation_required".to_string(),
            data: Some(json!({
                "required": {"confirm_segmented_repair": true},
                "policy": "historical invalid rows are preserved and a new current epoch is anchored by repair certificate"
            })),
        });
    }
    state
        .store
        .repair_segmented_ledger(actor, reason)
        .map_err(|error| rpc_err(-32603, format!("ledger_repair_segmented: {error}")))
}

pub fn record_mutation(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let target_event_id = params
        .get("target_ledger_event_id")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "target_ledger_event_id_required"))?;
    let mutation_type = params
        .get("mutation_type")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "mutation_type_required"))?;
    let actor = params
        .get("actor")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "actor_required"))?;
    let allowed_fields = params.get("allowed_fields").cloned().unwrap_or(json!([]));
    let payload_delta = params.get("payload_delta").cloned().unwrap_or(json!({}));
    let rationale = params
        .get("rationale")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "rationale_required"))?;
    let cycle_id = params.get("cycle_id").and_then(Value::as_str);
    let confirmation = params
        .get("confirm_mutation")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !confirmation {
        return Err(RpcError {
            code: -32602,
            message: "mutation_confirmation_required".to_string(),
            data: Some(json!({
                "required": {"confirm_mutation": true},
                "policy": "self-mutation events must be explicitly confirmed to prevent accidental registration"
            })),
        });
    }
    let conn = state
        .store
        .conn()
        .map_err(|_| rpc_err(-32603, "db_connection_failed"))?;
    let (target_row_id, original_event_hash, current_payload) = conn
        .query_row(
            "SELECT id, event_hash, payload_json FROM ledger_events WHERE event_id = ?1",
            [target_event_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|_| rpc_err(-32602, "target_ledger_event_not_found"))?;
    let current_payload: Value = serde_json::from_str(&current_payload)
        .map_err(|_| rpc_err(-32603, "payload_parse_failed"))?;
    let now = unix_now_s();
    let mut tx = conn
        .unchecked_transaction()
        .map_err(|e| rpc_err(-32603, format!("tx_begin: {e}")))?;
    let result = record_mutation_tx(
        &mut tx,
        target_event_id,
        target_row_id,
        mutation_type,
        actor,
        cycle_id,
        allowed_fields,
        payload_delta,
        &original_event_hash,
        current_payload,
        rationale,
        now,
    )
    .map_err(|e| rpc_err(-32603, format!("record_mutation: {e}")))?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("tx_commit: {e}")))?;
    state.store.refresh_ledger_cache();
    Ok(result)
}

pub fn reconcile_mismatches(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let confirmation = params
        .get("confirm_legacy_mutation")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !confirmation {
        return Err(RpcError {
            code: -32602,
            message: "legacy_mutation_confirmation_required".to_string(),
            data: Some(json!({
                "required": {"confirm_legacy_mutation": true},
                "policy": "reconciliation of pre-existing hash mismatches as self-mutations must be explicitly confirmed"
            })),
        });
    }
    let verification = state
        .store
        .verify_ledger()
        .map_err(|e| rpc_err(-32603, format!("verify_before_reconcile: {e}")))?;
    let self_modified = verification
        .get("self_modified_events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !self_modified.is_empty() {
        return Ok(json!({
            "ok": true,
            "already_reconciled": true,
            "reconciled_count": self_modified.len(),
            "mutations": self_modified
        }));
    }
    let first_invalid = verification.get("first_invalid");
    if first_invalid.is_none() || first_invalid == Some(&Value::Null) {
        return Ok(json!({
            "ok": true,
            "no_mismatches_found": true,
            "reconciled_count": 0
        }));
    }
    let invalid = first_invalid.unwrap();
    let reason = invalid
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if reason != "event_hash_mismatch" {
        return Ok(json!({
            "ok": true,
            "mismatch_not_hash": true,
            "reason": reason,
            "reconciled_count": 0
        }));
    }
    let event_id = invalid
        .get("event_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let conn = state
        .store
        .conn()
        .map_err(|_| rpc_err(-32603, "db_connection_failed"))?;
    let (target_row_id, original_event_hash, current_payload) = conn
        .query_row(
            "SELECT id, event_hash, payload_json FROM ledger_events WHERE event_id = ?1",
            [&event_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|_| rpc_err(-32603, "target_event_not_found_for_reconciliation"))?;
    let current_payload: Value = serde_json::from_str(&current_payload)
        .map_err(|_| rpc_err(-32603, "payload_parse_failed"))?;
    let now = unix_now_s();
    let mut tx = conn
        .unchecked_transaction()
        .map_err(|e| rpc_err(-32603, format!("tx_begin: {e}")))?;
    let result = record_mutation_tx(
        &mut tx,
        &event_id,
        target_row_id,
        "reconciliation.legacy",
        "brain.operator",
        None,
        json!(["retrieval_weights"]),
        json!({"field": "payload", "reason": "legacy nightly self-mutation predating meta-ledger tracking"}),
        &original_event_hash,
        current_payload,
        "Legacy nightly.proposal weight modification predating meta-ledger tracking",
        now,
    )
    .map_err(|e| rpc_err(-32603, format!("reconcile_record: {e}")))?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("tx_commit: {e}")))?;
    state.store.refresh_ledger_cache();
    Ok(json!({
        "ok": true,
        "already_reconciled": false,
        "reconciled_count": 1,
        "mutations": [result]
    }))
}
