use hom_shared::{RpcError, rpc_err};
use serde_json::Value;

use crate::BrainState;
use crate::db::storage::SaveInput;
use crate::services::answer_scorer;
use crate::services::calibration;
use crate::services::context_packer;
use crate::services::security_red_team;
use crate::services::source_attributed_synthesis;

pub fn save(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let input: SaveInput =
        serde_json::from_value(params).map_err(|error| rpc_err(-32602, error.to_string()))?;
    state
        .store
        .save_memory(input)
        .map_err(|error| rpc_err(-32603, error.to_string()))
}

pub fn recall(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let query = params
        .get("query")
        .or_else(|| params.get("q"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "query_required"))?;
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
    let current_session_source = params
        .get("current_session_source")
        .or_else(|| params.get("source"))
        .and_then(Value::as_str);
    let current_project_id = params
        .get("current_project_id")
        .or_else(|| params.get("project_id"))
        .and_then(Value::as_str);
    let embedding_model = params.get("embedding_model").and_then(Value::as_str);
    let query_vector = optional_vector_param(&params, "query_vector")?;
    let mut recall_result = state
        .store
        .recall_with_options(
            query,
            limit,
            current_session_source,
            current_project_id,
            embedding_model,
            query_vector.as_deref(),
            Some(&params),
        )
        .map_err(|error| rpc_err(-32603, error.to_string()))?;
    if let Some(requested_precision) = params
        .get("packed_precision")
        .or_else(|| params.get("precision_level"))
        .cloned()
    {
        if let Value::Object(map) = &mut recall_result {
            map.insert("packed_precision".to_string(), requested_precision);
        }
    }
    let audit =
        crate::services::diagnostics::compute_recall_audit(&recall_result, &recall_result, None);
    if let Value::Object(map) = &mut recall_result {
        let recall_meta = map
            .entry("recall_meta".to_string())
            .or_insert_with(|| serde_json::json!({}));
        if let Value::Object(meta) = recall_meta {
            meta.insert(
                "recall_audit".to_string(),
                serde_json::to_value(audit).unwrap_or(Value::Null),
            );
        }
    }
    context_packer::enrich_recall_result(query, &mut recall_result);
    Ok(recall_result)
}

pub fn embedding_upsert(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let memory_id = params
        .get("memory_id")
        .or_else(|| params.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "memory_id_required"))?;
    let model = params
        .get("model")
        .or_else(|| params.get("embedding_model"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "embedding_model_required"))?;
    let vector = required_vector_param(&params, "vector")?;
    let mut result = state
        .store
        .upsert_embedding(memory_id, model, &vector)
        .map_err(|error| rpc_err(-32603, error.to_string()))?;
    if let Value::Object(map) = &mut result {
        map.insert(
            "exposure_policy".to_string(),
            serde_json::json!({
                "surface": "internal_callable",
                "requires_explicit_vector": true,
                "auto_embedding_generation": false,
                "mutation_permitted": false
            }),
        );
    }
    Ok(result)
}

fn required_vector_param(params: &Value, key: &str) -> Result<Vec<f64>, RpcError> {
    optional_vector_param(params, key)?.ok_or_else(|| rpc_err(-32602, format!("{key}_required")))
}

fn optional_vector_param(params: &Value, key: &str) -> Result<Option<Vec<f64>>, RpcError> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };
    let array = value
        .as_array()
        .ok_or_else(|| rpc_err(-32602, format!("{key}_must_be_array")))?;
    if array.is_empty() {
        return Err(rpc_err(-32602, format!("{key}_empty")));
    }
    let mut vector = Vec::with_capacity(array.len());
    for item in array {
        vector.push(
            item.as_f64()
                .ok_or_else(|| rpc_err(-32602, format!("{key}_must_be_numeric")))?,
        );
    }
    Ok(Some(vector))
}

pub fn open(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let id = params
        .get("id")
        .or_else(|| params.get("memory_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "memory_id_required"))?;
    state
        .store
        .open_memory(id)
        .map_err(|error| rpc_err(-32603, error.to_string()))
}

pub fn answer(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let query = params
        .get("query")
        .or_else(|| params.get("q"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let recall_result = recall(state, params.clone())?;
    let memory_ids: Vec<String> = recall_result
        .get("memories")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("memory_id").and_then(Value::as_str).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let atoms_by_memory = state.store.load_atoms_for_memories(&memory_ids);

    // Record retrieval events for calibration
    let hits: Vec<crate::db::storage::RecallHit> = recall_result
        .get("memories")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let components = m.get("components").cloned().unwrap_or(Value::Null);
                    Some(crate::db::storage::RecallHit {
                        memory_id: m.get("memory_id").and_then(Value::as_str)?.to_string(),
                        key: m.get("key").and_then(Value::as_str)?.to_string(),
                        value: m.get("value").and_then(Value::as_str)?.to_string(),
                        memory_type: m.get("memory_type").and_then(Value::as_str)?.to_string(),
                        source: m.get("source").and_then(Value::as_str)?.to_string(),
                        session_id: m
                            .get("session_id")
                            .and_then(Value::as_str)
                            .map(String::from),
                        project_id: m
                            .get("project_id")
                            .and_then(Value::as_str)
                            .map(String::from),
                        track: m.get("track").and_then(Value::as_str).map(String::from),
                        created_at_s: m.get("created_at_s").and_then(Value::as_i64)?,
                        score: m.get("score").and_then(Value::as_f64)?,
                        components,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let _ = state.store.record_retrieval_events(&hits, now);

    // Compute confidence from answer_scorer and inject into recall result
    let memories = recall_result
        .get("memories")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let components = answer_scorer::score_components(query, &memories, now);
    let confidence = answer_scorer::answer_confidence(&components);

    // Apply calibration boost from recall outcome history
    let calibration_boosts = {
        let conn = state.store.conn()?;
        calibration::load_calibration_for_memories(&memory_ids, &conn)
    };
    let avg_boost = if calibration_boosts.is_empty() {
        1.0
    } else {
        calibration_boosts.values().sum::<f64>() / calibration_boosts.len() as f64
    };
    let confidence = (confidence * avg_boost).clamp(0.0, 1.0);

    // Inject confidence into recall result for build_answer
    let mut recall_with_confidence = recall_result;
    if let Value::Object(ref mut map) = recall_with_confidence {
        map.insert("confidence".to_string(), serde_json::json!(confidence));
        map.insert(
            "confidence_components".to_string(),
            serde_json::to_value(&components).unwrap_or(Value::Null),
        );
    }

    Ok(source_attributed_synthesis::build_answer(
        query,
        &recall_with_confidence,
        &atoms_by_memory,
    ))
}

pub fn events(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(25) as usize;
    state
        .store
        .list_events(limit)
        .map_err(|error| rpc_err(-32603, error.to_string()))
}

pub fn events_security(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let event_type = params
        .get("event_type")
        .and_then(Value::as_str)
        .unwrap_or("security.policy.rejected");
    let actor = params
        .get("actor")
        .and_then(Value::as_str)
        .unwrap_or("unknown.security.actor");
    let method = params
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("unknown.method");
    let payload = params.get("payload").cloned().unwrap_or(Value::Null);
    state
        .store
        .record_security_event_type(event_type, actor, method, payload)
        .map_err(|error| rpc_err(-32603, error.to_string()))
}

pub fn diagnostics_trust(state: &BrainState) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    Ok(crate::services::diagnostics::trust_diagnostics_json(&conn))
}

pub fn diagnostics_recall(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let recall_result = recall(state, params)?;
    let total_candidates = recall_result
        .get("recall_meta")
        .and_then(|m| m.get("limit"))
        .and_then(Value::as_u64)
        .unwrap_or(500) as usize;
    Ok(crate::services::diagnostics::recall_diagnostics_json(
        &recall_result,
        total_candidates,
    ))
}

pub fn diagnostics_recall_drift(_state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let reference = params
        .get("reference")
        .ok_or_else(|| rpc_err(-32602, "reference_required"))?;
    let current = params
        .get("current")
        .or_else(|| params.get("recall_result"))
        .ok_or_else(|| rpc_err(-32602, "current_required"))?;
    let signals = params.get("signals").cloned();
    Ok(crate::services::diagnostics::recall_audit_json(
        reference, current, signals,
    ))
}

pub fn security_saber_dry_run() -> Result<Value, RpcError> {
    serde_json::to_value(security_red_team::run_builtin_dry_run())
        .map_err(|error| rpc_err(-32603, error.to_string()))
}

pub fn security_canary_timeline(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
    state
        .store
        .security_canary_timeline(limit)
        .map_err(|error| rpc_err(-32603, error.to_string()))
}

pub fn security_audit_log(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
    state
        .store
        .security_audit_log(limit)
        .map_err(|error| rpc_err(-32603, error.to_string()))
}
