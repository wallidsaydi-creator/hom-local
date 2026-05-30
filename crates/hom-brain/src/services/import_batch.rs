use hom_shared::{RpcError, rpc_err};
use serde_json::{Value, json};

use crate::db::storage::ImportBatchRow;

pub fn import_sources(_state: &crate::BrainState) -> Result<Value, RpcError> {
    Ok(json!({
        "ok": true,
        "sources": [
            {
                "id": "generic.json",
                "name": "Generic JSON",
                "description": "Import memories from a JSON file with an items array",
                "supported": true
            },
            {
                "id": "generic.markdown",
                "name": "Generic Markdown",
                "description": "Import memories from a Markdown file (headings become candidates)",
                "supported": true
            },
            {
                "id": "claude.export",
                "name": "Claude Export",
                "description": "Import from Claude Memory export format",
                "supported": false
            },
            {
                "id": "claude.memory_files",
                "name": "Claude Memory Files",
                "description": "Import from Claude Code MEMORY.md files",
                "supported": false
            }
        ]
    }))
}

pub fn create_batch(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let source_type = params
        .get("source_type")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "source_type_required"))?;
    let source_name = params
        .get("source_name")
        .and_then(Value::as_str)
        .unwrap_or(source_type);
    let source_uri = params.get("source_uri").and_then(Value::as_str);
    let metadata = params.get("metadata").cloned().unwrap_or(json!({}));
    let batch = state
        .store
        .create_import_batch(source_type, source_name, source_uri, metadata)?;
    Ok(batch_to_json(&batch))
}

pub fn get_batch(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let batch_id = params
        .get("batch_id")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "batch_id_required"))?;
    let batch = state
        .store
        .get_import_batch(batch_id)?
        .ok_or_else(|| rpc_err(-32602, "batch_not_found"))?;
    let counts = state.store.count_import_candidates_by_status(batch_id)?;
    let mut result = batch_to_json(&batch);
    let totals = result
        .get_mut("totals")
        .and_then(Value::as_object_mut)
        .unwrap();
    for (status, count) in counts {
        totals.insert(status, json!(count));
    }
    Ok(result)
}

pub fn list_batches(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(20) as i64;
    let batches = state.store.list_import_batches(limit)?;
    let items: Vec<Value> = batches.iter().map(|b| batch_to_json(b)).collect();
    Ok(json!({
        "ok": true,
        "batches": items,
        "count": items.len()
    }))
}

pub fn cancel_batch(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let batch_id = params
        .get("batch_id")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "batch_id_required"))?;
    let batch = state
        .store
        .get_import_batch(batch_id)?
        .ok_or_else(|| rpc_err(-32602, "batch_not_found"))?;
    match batch.status.as_str() {
        "committed" | "cancelled" => {
            return Err(rpc_err(-32602, &format!("batch_already_{}", batch.status)));
        }
        _ => {}
    }
    state
        .store
        .update_import_batch_status(batch_id, "cancelled")?;
    Ok(json!({
        "ok": true,
        "batch_id": batch_id,
        "status": "cancelled"
    }))
}

pub fn batch_report(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let batch_id = params
        .get("batch_id")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "batch_id_required"))?;
    let batch = state
        .store
        .get_import_batch(batch_id)?
        .ok_or_else(|| rpc_err(-32602, "batch_not_found"))?;
    let candidates = state
        .store
        .get_import_candidates_by_batch(batch_id, None, 10000)?;
    let mut by_kind: HashMap<&str, i64> = HashMap::new();
    let mut by_status: HashMap<&str, i64> = HashMap::new();
    for c in &candidates {
        *by_kind.entry(&c.memory_kind).or_insert(0) += 1;
        *by_status.entry(&c.candidate_status).or_insert(0) += 1;
    }
    let kinds_json: Value = by_kind
        .iter()
        .map(|(k, v)| ((*k).to_string(), json!(*v)))
        .collect();
    let statuses_json: Value = by_status
        .iter()
        .map(|(k, v)| ((*k).to_string(), json!(*v)))
        .collect();
    Ok(json!({
        "ok": true,
        "batch_id": batch_id,
        "source": {
            "type": batch.source_type,
            "name": batch.source_name,
            "uri": batch.source_uri,
        },
        "status": batch.status,
        "totals": {
            "raw_items": batch.total_raw_items,
            "candidates": batch.total_candidates,
            "ready": batch.total_ready,
            "needs_review": batch.total_needs_review,
            "quarantined": batch.total_quarantined,
            "committed": batch.total_committed,
        },
        "by_kind": kinds_json,
        "by_status": statuses_json,
        "created_at_s": batch.created_at_s,
        "updated_at_s": batch.updated_at_s,
    }))
}

use std::collections::HashMap;

fn batch_to_json(batch: &ImportBatchRow) -> Value {
    json!({
        "ok": true,
        "batch_id": batch.batch_id,
        "source": {
            "type": batch.source_type,
            "name": batch.source_name,
            "uri": batch.source_uri,
        },
        "status": batch.status,
        "totals": {
            "raw_items": batch.total_raw_items,
            "candidates": batch.total_candidates,
            "ready": batch.total_ready,
            "needs_review": batch.total_needs_review,
            "quarantined": batch.total_quarantined,
            "committed": batch.total_committed,
        },
        "created_at_s": batch.created_at_s,
        "updated_at_s": batch.updated_at_s,
    })
}
