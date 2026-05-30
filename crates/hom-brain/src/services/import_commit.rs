use hom_shared::{RpcError, rpc_err};
use serde_json::{Value, json};

use crate::db::storage::SaveInput;

pub fn commit_batch(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let batch_id = params
        .get("batch_id")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "batch_id_required"))?;
    let only_candidate_ids: Option<Vec<String>> = params
        .get("candidate_ids")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        });

    let batch = state
        .store
        .get_import_batch(batch_id)?
        .ok_or_else(|| rpc_err(-32602, "batch_not_found"))?;

    if batch.status != "preview_ready" && batch.status != "classified" {
        return Err(rpc_err(
            -32602,
            &format!("batch_not_preview_ready: {}", batch.status),
        ));
    }

    state
        .store
        .update_import_batch_status(batch_id, "commit_requested")?;

    let candidates = state
        .store
        .get_import_candidates_by_batch(batch_id, None, 10000)?;

    let committable: Vec<_> = candidates
        .iter()
        .filter(|c| c.candidate_status == "ready")
        .filter(|c| match &only_candidate_ids {
            Some(ids) => ids.contains(&c.candidate_id),
            None => true,
        })
        .collect();

    let mut committed = 0_i64;
    let mut failed = 0_i64;
    let mut first_errors: Vec<String> = Vec::new();
    let mut committed_memory_ids: Vec<String> = Vec::new();

    for candidate in &committable {
        match commit_candidate(state, candidate) {
            Ok(memory_id) => {
                state
                    .store
                    .set_import_candidate_committed(&candidate.candidate_id, &memory_id)?;
                committed += 1;
                committed_memory_ids.push(memory_id);
            }
            Err(e) => {
                failed += 1;
                if first_errors.len() < 8 {
                    first_errors.push(format!("{e:?}"));
                }
            }
        }
    }

    state
        .store
        .increment_import_batch_committed(batch_id, committed)?;

    // Reject candidates that were not committed (user chose not to)
    if let Some(ref ids) = only_candidate_ids {
        for candidate in &candidates {
            if candidate.candidate_status == "ready" && !ids.contains(&candidate.candidate_id) {
                state.store.update_import_candidate_status(
                    &candidate.candidate_id,
                    "rejected",
                    None,
                    None,
                )?;
            }
        }
    }

    state
        .store
        .update_import_batch_status(batch_id, "committed")?;

    // Record in ledger
    let payload = json!({
        "batch_id": batch_id,
        "source_type": batch.source_type,
        "source_name": batch.source_name,
        "committed": committed,
        "failed": failed,
        "first_errors": first_errors,
    });
    let conn = &mut state.store.conn()?;
    let event_id = crate::services::admin::append_admin_ledger(
        conn,
        &format!("import.{}", batch.source_type),
        "system",
        Some(batch_id),
        payload,
    )?;

    // Bridge events linking import to each committed memory
    for memory_id in &committed_memory_ids {
        if let Err(e) =
            crate::services::reasoning_bridge::record_import_bridge(state, memory_id, &event_id)
        {
            eprintln!("import_bridge_failed: memory_id={memory_id} error={e:?}");
        }
    }

    Ok(json!({
        "ok": true,
        "batch_id": batch_id,
        "status": "committed",
        "committed": committed,
        "failed": failed,
        "first_errors": first_errors,
    }))
}

fn commit_candidate(
    state: &crate::BrainState,
    candidate: &crate::db::storage::ImportCandidateRow,
) -> Result<String, RpcError> {
    let key = candidate
        .title
        .clone()
        .unwrap_or_else(|| format!("import:{}", candidate.candidate_id));
    let input = SaveInput {
        key: Some(key),
        value: Some(candidate.body.clone()),
        memory_type: Some(candidate.memory_kind.clone()),
        source: Some(format!("import:{}", candidate.source_type)),
        session_id: None,
        project_id: None,
        track: Some("import".to_string()),
        metadata: json!({
            "import_batch_id": candidate.batch_id,
            "import_candidate_id": candidate.candidate_id,
            "source_type": candidate.source_type,
            "source_name": candidate.source_name,
            "source_hash": candidate.source_hash,
            "confidence": candidate.confidence,
            "sensitivity_class": candidate.sensitivity_class,
        }),
        trusted_generated_artifact: false,
    };
    let result = state
        .store
        .save_memory(input)
        .map_err(|e| rpc_err(-32603, &format!("import_commit_save: {e}")))?;
    let memory_id = result
        .get("memory_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok(memory_id)
}
