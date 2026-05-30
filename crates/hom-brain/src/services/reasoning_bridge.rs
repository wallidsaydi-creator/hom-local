use hom_shared::{RpcError, canonical_json, rpc_err, sha256_hex};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::BrainState;
use crate::db::storage::{SaveInput, append_ledger_tx, unix_now_s};
use crate::services::route_certificate;

pub fn record_tool_event_bridge(
    state: &BrainState,
    tool_event_id: &str,
) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let row = conn
        .query_row(
            "SELECT id, tool_id, method, outcome, error_code, latency_ms, linked_memory_id,
                    provider_id, model_id, route_certificate_id, gate_task_id, provenance_json,
                    created_at_s
             FROM tool_events WHERE id = ?1",
            [tool_event_id],
            |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "tool_id": row.get::<_, String>(1)?,
                    "method": row.get::<_, String>(2)?,
                    "outcome": row.get::<_, String>(3)?,
                    "error_code": row.get::<_, Option<i64>>(4)?,
                    "latency_ms": row.get::<_, i64>(5)?,
                    "linked_memory_id": row.get::<_, Option<String>>(6)?,
                    "provider_id": row.get::<_, Option<String>>(7)?,
                    "model_id": row.get::<_, Option<String>>(8)?,
                    "route_certificate_id": row.get::<_, Option<String>>(9)?,
                    "gate_task_id": row.get::<_, Option<String>>(10)?,
                    "provenance": parse_json(row.get::<_, String>(11)?),
                    "created_at_s": row.get::<_, i64>(12)?
                }))
            },
        )
        .optional()
        .map_err(|e| rpc_err(-32603, format!("reasoning_bridge_tool_lookup: {e}")))?
        .ok_or_else(|| rpc_err(-32602, "tool_event_not_found"))?;

    let provenance = row.get("provenance").cloned().unwrap_or_else(|| json!({}));
    let request_id = provenance.get("request_id").and_then(Value::as_str);
    let error_id = provenance.get("error_id").and_then(Value::as_str);
    let fix_id = provenance.get("fix_id").and_then(Value::as_str);
    let broker_ledger_event_id = provenance
        .get("broker_ledger_event_id")
        .and_then(Value::as_str);
    let id = insert_bridge(
        &conn,
        BridgeInsert {
            bridge_kind: "tool_invocation",
            memory_id: row.get("linked_memory_id").and_then(Value::as_str),
            source_memory_id: None,
            tool_event_id: Some(tool_event_id),
            provider_id: row.get("provider_id").and_then(Value::as_str),
            model_id: row.get("model_id").and_then(Value::as_str),
            request_id,
            route_certificate_id: row.get("route_certificate_id").and_then(Value::as_str),
            gate_task_id: row.get("gate_task_id").and_then(Value::as_str),
            gate_evidence_id: None,
            error_id,
            fix_id,
            compaction_artifact_id: None,
            nightly_artifact_id: None,
            reasoning_artifact_id: None,
            ledger_event_id: broker_ledger_event_id,
            relation: json!({
                "source": "tool_events",
                "tool_event": row,
                "broker_ledger_event_id": broker_ledger_event_id,
                "reasoning_contract": "public evidence bridge only; no hidden chain of thought",
                "scoring_boundary": "tool_importance_v1 uses exact events, Wilson lower bound, scale factors, and bridge evidence",
                "error_fix_contract": {
                    "error_id": error_id,
                    "fix_id": fix_id,
                    "provider_model_bridge": row.get("provider_id").is_some() || row.get("model_id").is_some()
                }
            }),
        },
    )?;
    Ok(json!({"ok": true, "bridge_event_id": id}))
}

pub fn record_tool_invocation(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let tool_id = first_param(&params, &["tool_id", "toolId"])
        .ok_or_else(|| rpc_err(-32602, "tool_id_required"))?;
    let method = first_param(&params, &["method"]).unwrap_or_else(|| tool_id.clone());
    let permission_scope = first_param(&params, &["permission_scope", "permissionScope"])
        .unwrap_or_else(|| "unknown".to_string());
    let invocation_source = first_param(&params, &["invocation_source", "invocationSource"])
        .unwrap_or_else(|| "unknown".to_string());
    let params_hash = first_param(&params, &["params_hash", "paramsHash"]).unwrap_or_else(|| {
        sha256_hex(&canonical_json(&params).unwrap_or_else(|_| "{}".to_string()))
    });
    let outcome = first_param(&params, &["outcome"]).unwrap_or_else(|| "unknown".to_string());
    let latency_ms = params
        .get("latency_ms")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let linked_memory_id = first_param(&params, &["linked_memory_id", "linkedMemoryId"]);
    let provider_id = first_param(&params, &["provider_id", "providerId"]);
    let model_id = first_param(&params, &["model_id", "modelId"]);
    let route_certificate_id =
        first_param(&params, &["route_certificate_id", "routeCertificateId"]);
    let gate_task_id = first_param(&params, &["gate_task_id", "gateTaskId"]);
    let error_code = params.get("error_code").and_then(Value::as_i64);
    let provenance = params
        .get("provenance")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let request_id = provenance
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let error_id = provenance
        .get("error_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let fix_id = provenance
        .get("fix_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let broker_ledger_event_id = provenance
        .get("broker_ledger_event_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let now = unix_now_s();
    let mut conn = state.store.conn()?;
    let route_certificate_snapshot = match route_certificate_id.as_deref() {
        Some(id) => Some(
            route_certificate::load_certificate(&conn, id)?
                .ok_or_else(|| rpc_err(-32044, "route_certificate_not_found"))?,
        ),
        None => None,
    };
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("tool_event_record_tx: {e}")))?;
    let tool_event_id = Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO tool_events
            (id, tool_id, method, backend_callable, permission_scope, invocation_source,
             params_hash, outcome, error_code, latency_ms, linked_memory_id, provider_id,
             model_id, route_certificate_id, gate_task_id, provenance_json, created_at_s)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            tool_event_id,
            tool_id,
            method,
            1,
            permission_scope,
            invocation_source,
            params_hash,
            outcome,
            error_code,
            latency_ms,
            linked_memory_id,
            provider_id,
            model_id,
            route_certificate_id,
            gate_task_id,
            canonical_json(&provenance)
                .map_err(|e| rpc_err(-32603, format!("tool_event_record_provenance: {e}")))?,
            now,
        ],
    )
    .map_err(|e| rpc_err(-32603, format!("tool_event_record_insert: {e}")))?;
    let bridge_event_id = Uuid::new_v4().to_string();
    let relation = json!({
        "source": "tool_events.atomic_record",
        "tool_event_id": tool_event_id,
        "reasoning_contract": "public evidence bridge only; no hidden chain of thought",
        "scoring_boundary": "tool_importance_v1 uses exact events, Wilson lower bound, scale factors, and bridge evidence",
        "route_certificate": route_certificate_snapshot,
        "error_fix_contract": {
            "error_id": error_id,
            "fix_id": fix_id,
            "provider_model_bridge": provider_id.is_some() || model_id.is_some()
        }
    });
    tx.execute(
        "INSERT INTO reasoning_bridge_events
            (id, bridge_kind, memory_id, source_memory_id, tool_event_id, provider_id,
             model_id, request_id, route_certificate_id, gate_task_id, gate_evidence_id,
             error_id, fix_id, compaction_artifact_id, nightly_artifact_id,
             reasoning_artifact_id, ledger_event_id, relation_json, created_at_s)
         VALUES (?1, 'tool_invocation', ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10,
                 NULL, NULL, NULL, ?11, ?12, ?13)",
        params![
            bridge_event_id,
            linked_memory_id,
            tool_event_id,
            provider_id,
            model_id,
            request_id,
            route_certificate_id,
            gate_task_id,
            error_id,
            fix_id,
            broker_ledger_event_id,
            canonical_json(&relation)
                .map_err(|e| rpc_err(-32603, format!("tool_event_record_relation: {e}")))?,
            now,
        ],
    )
    .map_err(|e| rpc_err(-32603, format!("tool_event_record_bridge_insert: {e}")))?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("tool_event_record_commit: {e}")))?;
    Ok(json!({
        "ok": true,
        "tool_event_id": tool_event_id,
        "bridge_event_id": bridge_event_id,
        "created_at_s": now
    }))
}

pub fn record_compaction(
    state: &BrainState,
    compaction_memory_id: &str,
    source_memory_ids: &[String],
    ledger_event_id: Option<&str>,
    recall_probe: &Value,
) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let main_id = insert_bridge(
        &conn,
        BridgeInsert {
            bridge_kind: "session_compaction",
            memory_id: Some(compaction_memory_id),
            source_memory_id: None,
            tool_event_id: None,
            provider_id: None,
            model_id: None,
            request_id: None,
            route_certificate_id: None,
            gate_task_id: None,
            gate_evidence_id: None,
            error_id: None,
            fix_id: None,
            compaction_artifact_id: Some(compaction_memory_id),
            nightly_artifact_id: None,
            reasoning_artifact_id: None,
            ledger_event_id,
            relation: json!({
                "strategy": "extractive_full_detail",
                "source_memory_ids": source_memory_ids,
                "source_memory_count": source_memory_ids.len(),
                "recall_probe": recall_probe,
                "paper_boundary": "session compaction remains extractive; mutation is not applied to source memory bodies",
                "feeds": ["reasoning", "nightly_dream"]
            }),
        },
    )?;
    let mut source_bridge_ids = Vec::new();
    for source_memory_id in source_memory_ids {
        source_bridge_ids.push(insert_bridge(
            &conn,
            BridgeInsert {
                bridge_kind: "session_compaction_source",
                memory_id: Some(compaction_memory_id),
                source_memory_id: Some(source_memory_id),
                tool_event_id: None,
                provider_id: None,
                model_id: None,
                request_id: None,
                route_certificate_id: None,
                gate_task_id: None,
                gate_evidence_id: None,
                error_id: None,
                fix_id: None,
                compaction_artifact_id: Some(compaction_memory_id),
                nightly_artifact_id: None,
                reasoning_artifact_id: None,
                ledger_event_id,
                relation: json!({
                    "strategy": "extractive_full_detail",
                    "source_memory_id": source_memory_id,
                    "compaction_memory_id": compaction_memory_id
                }),
            },
        )?);
    }
    Ok(json!({
        "ok": true,
        "bridge_event_id": main_id,
        "source_bridge_event_ids": source_bridge_ids,
        "source_bridge_count": source_bridge_ids.len()
    }))
}

pub fn record_nightly_artifact(
    state: &BrainState,
    nightly_artifact_id: &str,
    ledger_event_id: Option<&str>,
    touched_memory_ids: &[String],
    guardrails: &Value,
) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let main_id = insert_bridge(
        &conn,
        BridgeInsert {
            bridge_kind: "nightly_artifact",
            memory_id: None,
            source_memory_id: None,
            tool_event_id: None,
            provider_id: None,
            model_id: None,
            request_id: None,
            route_certificate_id: None,
            gate_task_id: None,
            gate_evidence_id: None,
            error_id: None,
            fix_id: None,
            compaction_artifact_id: None,
            nightly_artifact_id: Some(nightly_artifact_id),
            reasoning_artifact_id: None,
            ledger_event_id,
            relation: json!({
                "touched_memory_ids": touched_memory_ids,
                "touched_memory_count": touched_memory_ids.len(),
                "guardrails": guardrails,
                "mutation_scope": "retrieval_weights_only"
            }),
        },
    )?;
    let mut touched_bridge_ids = Vec::new();
    for memory_id in touched_memory_ids {
        touched_bridge_ids.push(insert_bridge(
            &conn,
            BridgeInsert {
                bridge_kind: "nightly_memory_touch",
                memory_id: Some(memory_id),
                source_memory_id: None,
                tool_event_id: None,
                provider_id: None,
                model_id: None,
                request_id: None,
                route_certificate_id: None,
                gate_task_id: None,
                gate_evidence_id: None,
                error_id: None,
                fix_id: None,
                compaction_artifact_id: None,
                nightly_artifact_id: Some(nightly_artifact_id),
                reasoning_artifact_id: None,
                ledger_event_id,
                relation: json!({
                    "nightly_artifact_id": nightly_artifact_id,
                    "memory_id": memory_id,
                    "mutation_scope": "retrieval_weights_only"
                }),
            },
        )?);
    }
    Ok(json!({
        "ok": true,
        "bridge_event_id": main_id,
        "touched_bridge_event_ids": touched_bridge_ids,
        "touched_bridge_count": touched_bridge_ids.len()
    }))
}

pub fn record_security_decision(
    state: &BrainState,
    client_id: &str,
    method: &str,
    decision: &Value,
    ledger_event_id: Option<&str>,
) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let allowed = decision
        .get("allowed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let error_id = (!allowed).then(|| {
        decision
            .get("rule_id")
            .and_then(Value::as_str)
            .map(|rule| format!("security-error:{method}:{rule}"))
            .unwrap_or_else(|| format!("security-error:{method}:unknown"))
    });
    let id = insert_bridge(
        &conn,
        BridgeInsert {
            bridge_kind: "security_decision",
            memory_id: None,
            source_memory_id: None,
            tool_event_id: None,
            provider_id: None,
            model_id: None,
            request_id: None,
            route_certificate_id: None,
            gate_task_id: None,
            gate_evidence_id: None,
            error_id: error_id.as_deref(),
            fix_id: None,
            compaction_artifact_id: None,
            nightly_artifact_id: None,
            reasoning_artifact_id: None,
            ledger_event_id,
            relation: json!({
                "source": "security_policy.evaluate_request",
                "client_id": client_id,
                "method": method,
                "decision": decision,
                "bridge_contract": "security denials and permits are public runtime evidence for monitoring and later fix attribution"
            }),
        },
    )?;
    Ok(json!({"ok": true, "bridge_event_id": id, "error_id": error_id}))
}

pub fn list(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let limit = params
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(100)
        .clamp(1, 500);
    let memory_id = first_param(&params, &["memory_id", "memoryId"]);
    let tool_event_id = first_param(&params, &["tool_event_id", "toolEventId"]);
    let compaction_artifact_id = first_param(
        &params,
        &[
            "compaction_artifact_id",
            "compactionArtifactId",
            "compaction_memory_id",
        ],
    );
    let nightly_artifact_id = first_param(&params, &["nightly_artifact_id", "nightlyArtifactId"]);
    let reasoning_artifact_id =
        first_param(&params, &["reasoning_artifact_id", "reasoningArtifactId"]);
    let conn = state.store.conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, bridge_kind, memory_id, source_memory_id, tool_event_id,
                    provider_id, model_id, request_id, route_certificate_id, gate_task_id,
                    gate_evidence_id, error_id, fix_id, compaction_artifact_id,
                    nightly_artifact_id, reasoning_artifact_id, ledger_event_id,
                    relation_json, created_at_s
             FROM reasoning_bridge_events
             WHERE (?1 IS NULL OR memory_id = ?1 OR source_memory_id = ?1)
               AND (?2 IS NULL OR tool_event_id = ?2)
               AND (?3 IS NULL OR compaction_artifact_id = ?3)
               AND (?4 IS NULL OR nightly_artifact_id = ?4)
               AND (?5 IS NULL OR reasoning_artifact_id = ?5)
             ORDER BY created_at_s DESC
             LIMIT ?6",
        )
        .map_err(|e| rpc_err(-32603, format!("reasoning_bridge_list_prepare: {e}")))?;
    let events = stmt
        .query_map(
            params![
                memory_id.as_deref(),
                tool_event_id.as_deref(),
                compaction_artifact_id.as_deref(),
                nightly_artifact_id.as_deref(),
                reasoning_artifact_id.as_deref(),
                limit,
            ],
            bridge_from_row,
        )
        .map_err(|e| rpc_err(-32603, format!("reasoning_bridge_list_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("reasoning_bridge_list_collect: {e}")))?;
    Ok(json!({"ok": true, "events": events, "count": events.len()}))
}

pub fn artifacts(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let limit = params
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(100)
        .clamp(1, 500);
    let conn = state.store.conn()?;
    let mut artifact_rows = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT id, key, memory_type, source, session_id, created_at_s, quality_score,
                        metadata_json
                 FROM memories
                 WHERE memory_type IN ('session_compaction', 'session_reasoning')
                 ORDER BY created_at_s DESC
                 LIMIT ?1",
            )
            .map_err(|e| rpc_err(-32603, format!("reasoning_artifacts_prepare: {e}")))?;
        artifact_rows.extend(
            stmt.query_map([limit], |row| {
                Ok(json!({
                    "kind": "memory_artifact",
                    "id": row.get::<_, String>(0)?,
                    "key": row.get::<_, String>(1)?,
                    "memory_type": row.get::<_, String>(2)?,
                    "source": row.get::<_, String>(3)?,
                    "session_id": row.get::<_, Option<String>>(4)?,
                    "created_at_s": row.get::<_, i64>(5)?,
                    "quality_score": row.get::<_, f64>(6)?,
                    "metadata": parse_json(row.get::<_, String>(7)?)
                }))
            })
            .map_err(|e| rpc_err(-32603, format!("reasoning_artifacts_query: {e}")))?
            .filter_map(|row| row.ok()),
        );
    }
    {
        let mut stmt = conn
            .prepare(
                "SELECT id, cycle_id, mode, status, proposal_count, applied_count,
                        mutation_count, warning_count, insight_count, touched_memory_count,
                        input_signal_hash, output_state_hash, ledger_event_id, created_at_s
                 FROM nightly_artifacts
                 ORDER BY created_at_s DESC
                 LIMIT ?1",
            )
            .map_err(|e| rpc_err(-32603, format!("nightly_artifacts_prepare: {e}")))?;
        artifact_rows.extend(
            stmt.query_map([limit], |row| {
                Ok(json!({
                    "kind": "nightly_artifact",
                    "id": row.get::<_, String>(0)?,
                    "cycle_id": row.get::<_, String>(1)?,
                    "mode": row.get::<_, String>(2)?,
                    "status": row.get::<_, String>(3)?,
                    "proposal_count": row.get::<_, i64>(4)?,
                    "applied_count": row.get::<_, i64>(5)?,
                    "mutation_count": row.get::<_, i64>(6)?,
                    "warning_count": row.get::<_, i64>(7)?,
                    "insight_count": row.get::<_, i64>(8)?,
                    "touched_memory_count": row.get::<_, i64>(9)?,
                    "input_signal_hash": row.get::<_, String>(10)?,
                    "output_state_hash": row.get::<_, String>(11)?,
                    "ledger_event_id": row.get::<_, Option<String>>(12)?,
                    "created_at_s": row.get::<_, i64>(13)?
                }))
            })
            .map_err(|e| rpc_err(-32603, format!("nightly_artifacts_query: {e}")))?
            .filter_map(|row| row.ok()),
        );
    }
    artifact_rows.sort_by(|left, right| {
        right
            .get("created_at_s")
            .and_then(Value::as_i64)
            .cmp(&left.get("created_at_s").and_then(Value::as_i64))
    });
    artifact_rows.truncate(limit as usize);
    Ok(json!({
        "ok": true,
        "artifacts": artifact_rows,
        "count": artifact_rows.len(),
        "diagnostic": {
            "state": if artifact_rows.is_empty() { "empty" } else { "available" },
            "source": "reasoning_bridge_events, session_compaction memories, session_reasoning memories, nightly_artifacts"
        }
    }))
}

pub fn tool_quality(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let limit = params
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(25)
        .clamp(1, 200) as usize;
    let tool_id_filter = first_param(&params, &["tool_id", "toolId"]);
    let conn = state.store.conn()?;

    let mut bridge_stmt = conn
        .prepare(
            "SELECT tool_event_id,
                    COUNT(*) AS bridge_count,
                    SUM(
                        CASE WHEN memory_id IS NOT NULL THEN 1 ELSE 0 END +
                        CASE WHEN source_memory_id IS NOT NULL THEN 1 ELSE 0 END +
                        CASE WHEN provider_id IS NOT NULL THEN 1 ELSE 0 END +
                        CASE WHEN model_id IS NOT NULL THEN 1 ELSE 0 END +
                        CASE WHEN request_id IS NOT NULL THEN 1 ELSE 0 END +
                        CASE WHEN route_certificate_id IS NOT NULL THEN 1 ELSE 0 END +
                        CASE WHEN gate_task_id IS NOT NULL THEN 1 ELSE 0 END +
                        CASE WHEN gate_evidence_id IS NOT NULL THEN 1 ELSE 0 END +
                        CASE WHEN error_id IS NOT NULL THEN 1 ELSE 0 END +
                        CASE WHEN fix_id IS NOT NULL THEN 1 ELSE 0 END +
                        CASE WHEN compaction_artifact_id IS NOT NULL THEN 1 ELSE 0 END +
                        CASE WHEN nightly_artifact_id IS NOT NULL THEN 1 ELSE 0 END +
                        CASE WHEN reasoning_artifact_id IS NOT NULL THEN 1 ELSE 0 END +
                        CASE WHEN ledger_event_id IS NOT NULL THEN 1 ELSE 0 END
                    ) AS bridge_degree
             FROM reasoning_bridge_events
             WHERE tool_event_id IS NOT NULL
             GROUP BY tool_event_id",
        )
        .map_err(|e| rpc_err(-32603, format!("tool_quality_bridge_prepare: {e}")))?;
    let bridge_rows = bridge_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| rpc_err(-32603, format!("tool_quality_bridge_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("tool_quality_bridge_collect: {e}")))?;

    let bridge_map = bridge_rows
        .into_iter()
        .map(|(tool_event_id, bridge_count, bridge_degree)| {
            (tool_event_id, (bridge_count, bridge_degree))
        })
        .collect::<std::collections::HashMap<_, _>>();

    let mut sql = "SELECT id, tool_id, outcome, error_code, latency_ms, linked_memory_id,
                          provider_id, model_id, route_certificate_id, gate_task_id,
                          provenance_json, created_at_s
                   FROM tool_events"
        .to_string();
    if tool_id_filter.is_some() {
        sql.push_str(" WHERE tool_id = ?1");
    }
    sql.push_str(" ORDER BY created_at_s DESC");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| rpc_err(-32603, format!("tool_quality_prepare: {e}")))?;

    let mut events = Vec::new();
    if let Some(tool_id) = tool_id_filter.as_deref() {
        let rows = stmt
            .query_map([tool_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    parse_json(row.get::<_, String>(10)?),
                    row.get::<_, i64>(11)?,
                ))
            })
            .map_err(|e| rpc_err(-32603, format!("tool_quality_query: {e}")))?;
        for row in rows {
            events.push(row.map_err(|e| rpc_err(-32603, format!("tool_quality_row: {e}")))?);
        }
    } else {
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    parse_json(row.get::<_, String>(10)?),
                    row.get::<_, i64>(11)?,
                ))
            })
            .map_err(|e| rpc_err(-32603, format!("tool_quality_query: {e}")))?;
        for row in rows {
            events.push(row.map_err(|e| rpc_err(-32603, format!("tool_quality_row: {e}")))?);
        }
    }

    #[derive(Default)]
    struct ToolAggregate {
        event_count: usize,
        success_count: usize,
        error_count: usize,
        bridged_event_count: usize,
        required_event_count: usize,
        bridged_required_event_count: usize,
        total_bridge_rows: i64,
        total_bridge_degree: i64,
        linked_error_fix_count: usize,
        last_seen_at_s: i64,
        latencies: Vec<i64>,
    }

    let mut per_tool = std::collections::BTreeMap::<String, ToolAggregate>::new();
    let mut orphan_tool_event_count = 0usize;
    let mut total_required_event_count = 0usize;
    let mut bridged_required_event_count = 0usize;

    for (
        event_id,
        tool_id,
        outcome,
        error_code,
        latency_ms,
        linked_memory_id,
        provider_id,
        model_id,
        route_certificate_id,
        gate_task_id,
        provenance,
        created_at_s,
    ) in events
    {
        let (bridge_count, bridge_degree) = bridge_map.get(&event_id).copied().unwrap_or((0, 0));
        let has_bridge = bridge_count > 0;
        let required_bridge = linked_memory_id.is_some()
            || provider_id.is_some()
            || model_id.is_some()
            || route_certificate_id.is_some()
            || gate_task_id.is_some();
        let success = outcome == "ok" && error_code.is_none();
        let aggregate = per_tool.entry(tool_id).or_default();
        aggregate.event_count += 1;
        aggregate.success_count += usize::from(success);
        aggregate.error_count += usize::from(!success);
        aggregate.bridged_event_count += usize::from(has_bridge);
        aggregate.required_event_count += usize::from(required_bridge);
        aggregate.bridged_required_event_count += usize::from(required_bridge && has_bridge);
        aggregate.total_bridge_rows += bridge_count;
        aggregate.total_bridge_degree += bridge_degree;
        aggregate.latencies.push(latency_ms);
        aggregate.last_seen_at_s = aggregate.last_seen_at_s.max(created_at_s);
        if provenance.get("error_id").and_then(Value::as_str).is_some()
            || provenance.get("fix_id").and_then(Value::as_str).is_some()
        {
            aggregate.linked_error_fix_count += 1;
        }
        if !has_bridge {
            orphan_tool_event_count += 1;
        }
        total_required_event_count += usize::from(required_bridge);
        bridged_required_event_count += usize::from(required_bridge && has_bridge);
    }

    let orphan_bridge_count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM reasoning_bridge_events rb
             WHERE rb.tool_event_id IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM tool_events t WHERE t.id = rb.tool_event_id)",
            [],
            |row| row.get(0),
        )
        .map_err(|e| rpc_err(-32603, format!("tool_quality_orphan_bridge_query: {e}")))?;

    let mut tools = per_tool
        .into_iter()
        .map(|(tool_id, mut aggregate)| {
            aggregate.latencies.sort_unstable();
            let mean_latency_ms = if aggregate.latencies.is_empty() {
                0.0
            } else {
                aggregate.latencies.iter().sum::<i64>() as f64 / aggregate.latencies.len() as f64
            };
            let p95_latency_ms = percentile_nearest_rank(&aggregate.latencies, 0.95).unwrap_or(0.0);
            let success_rate = ratio(aggregate.success_count, aggregate.event_count);
            let wilson_lower_bound =
                wilson_lower_bound(aggregate.success_count, aggregate.event_count, 1.96);
            let bridge_coverage = ratio(aggregate.bridged_event_count, aggregate.event_count);
            let bridge_coverage_required = if aggregate.required_event_count == 0 {
                1.0
            } else {
                ratio(
                    aggregate.bridged_required_event_count,
                    aggregate.required_event_count,
                )
            };
            let average_bridge_degree = if aggregate.total_bridge_rows == 0 {
                0.0
            } else {
                aggregate.total_bridge_degree as f64 / aggregate.total_bridge_rows as f64
            };
            let bridge_multiplier = (1.0 + (average_bridge_degree * 0.05)).min(2.0);
            let latency_factor = (-mean_latency_ms / 1_000.0).exp();
            let importance_score = wilson_lower_bound
                * (1.0 + aggregate.event_count as f64).ln()
                * bridge_multiplier
                * latency_factor;
            json!({
                "tool_id": tool_id,
                "event_count": aggregate.event_count,
                "success_count": aggregate.success_count,
                "error_count": aggregate.error_count,
                "success_rate": success_rate,
                "wilson_lower_bound": wilson_lower_bound,
                "mean_latency_ms": mean_latency_ms,
                "p95_latency_ms": p95_latency_ms,
                "bridge_count": aggregate.total_bridge_rows,
                "bridged_event_count": aggregate.bridged_event_count,
                "bridge_coverage": bridge_coverage,
                "required_bridge_event_count": aggregate.required_event_count,
                "bridge_coverage_required": bridge_coverage_required,
                "average_bridge_degree": average_bridge_degree,
                "linked_error_fix_count": aggregate.linked_error_fix_count,
                "importance_score": importance_score,
                "formula_ref": "tool_importance_v1",
                "last_seen_at_s": aggregate.last_seen_at_s,
            })
        })
        .collect::<Vec<_>>();

    tools.sort_by(|left, right| {
        right["importance_score"]
            .as_f64()
            .partial_cmp(&left["importance_score"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    tools.truncate(limit);

    let global_required_coverage = if total_required_event_count == 0 {
        1.0
    } else {
        ratio(bridged_required_event_count, total_required_event_count)
    };

    Ok(json!({
        "ok": true,
        "formula": {
            "formula_ref": "tool_importance_v1",
            "reliability": "wilson_lower_bound(success_count, event_count, z=1.96)",
            "score": "wilson_lower_bound * ln(1 + event_count) * min(2.0, 1 + 0.05 * average_bridge_degree) * exp(-mean_latency_ms / 1000.0)",
            "notes": [
                "bridge_coverage uses all tool events for a tool",
                "bridge_coverage_required only counts events with explicit reasoning/provenance linkage requirements"
            ]
        },
        "integrity": {
            "tool_event_count": tools.iter().map(|tool| tool["event_count"].as_u64().unwrap_or(0)).sum::<u64>(),
            "orphan_tool_event_count": orphan_tool_event_count,
            "orphan_bridge_count": orphan_bridge_count,
            "required_bridge_event_count": total_required_event_count,
            "bridged_required_event_count": bridged_required_event_count,
            "bridge_coverage_required": global_required_coverage,
        },
        "tools": tools,
        "count": tools.len(),
    }))
}

pub fn bridge_integrity(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let limit = params
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(25)
        .clamp(1, 200);
    let tool_quality = tool_quality(
        state,
        json!({
            "limit": limit,
            "tool_id": first_param(&params, &["tool_id", "toolId"])
        }),
    )?;
    let conn = state.store.conn()?;
    let tool_id_filter = first_param(&params, &["tool_id", "toolId"]);
    let mut unbridged_stmt = conn
        .prepare(
            "SELECT id, tool_id, outcome, linked_memory_id, provider_id, model_id,
                    route_certificate_id, gate_task_id, created_at_s
             FROM tool_events
             WHERE (?1 IS NULL OR tool_id = ?1)
               AND (linked_memory_id IS NOT NULL OR provider_id IS NOT NULL OR model_id IS NOT NULL
                    OR route_certificate_id IS NOT NULL OR gate_task_id IS NOT NULL)
               AND NOT EXISTS (
                    SELECT 1 FROM reasoning_bridge_events rb WHERE rb.tool_event_id = tool_events.id
               )
             ORDER BY created_at_s DESC
             LIMIT ?2",
        )
        .map_err(|e| rpc_err(-32603, format!("bridge_integrity_unbridged_prepare: {e}")))?;
    let unbridged_required_events = unbridged_stmt
        .query_map(params![tool_id_filter.as_deref(), limit], |row| {
            Ok(json!({
                "tool_event_id": row.get::<_, String>(0)?,
                "tool_id": row.get::<_, String>(1)?,
                "outcome": row.get::<_, String>(2)?,
                "linked_memory_id": row.get::<_, Option<String>>(3)?,
                "provider_id": row.get::<_, Option<String>>(4)?,
                "model_id": row.get::<_, Option<String>>(5)?,
                "route_certificate_id": row.get::<_, Option<String>>(6)?,
                "gate_task_id": row.get::<_, Option<String>>(7)?,
                "created_at_s": row.get::<_, i64>(8)?
            }))
        })
        .map_err(|e| rpc_err(-32603, format!("bridge_integrity_unbridged_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("bridge_integrity_unbridged_collect: {e}")))?;
    let mut orphan_stmt = conn
        .prepare(
            "SELECT id, bridge_kind, tool_event_id, created_at_s
             FROM reasoning_bridge_events rb
             WHERE rb.tool_event_id IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM tool_events t WHERE t.id = rb.tool_event_id)
             ORDER BY created_at_s DESC
             LIMIT ?1",
        )
        .map_err(|e| rpc_err(-32603, format!("bridge_integrity_orphan_prepare: {e}")))?;
    let orphan_bridges = orphan_stmt
        .query_map([limit], |row| {
            Ok(json!({
                "bridge_event_id": row.get::<_, String>(0)?,
                "bridge_kind": row.get::<_, String>(1)?,
                "tool_event_id": row.get::<_, String>(2)?,
                "created_at_s": row.get::<_, i64>(3)?
            }))
        })
        .map_err(|e| rpc_err(-32603, format!("bridge_integrity_orphan_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("bridge_integrity_orphan_collect: {e}")))?;
    Ok(json!({
        "ok": true,
        "integrity": tool_quality.get("integrity").cloned().unwrap_or_else(|| json!({})),
        "tools": tool_quality.get("tools").cloned().unwrap_or_else(|| json!([])),
        "unbridged_required_events": unbridged_required_events,
        "orphan_bridges": orphan_bridges,
        "count": {
            "unbridged_required_events": unbridged_required_events.len(),
            "orphan_bridges": orphan_bridges.len()
        }
    }))
}

pub fn run(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let target_memory_id = first_param(
        &params,
        &[
            "memory_id",
            "memoryId",
            "compaction_memory_id",
            "compactionMemoryId",
            "target_memory_id",
        ],
    )
    .ok_or_else(|| rpc_err(-32602, "reasoning_target_memory_id_required"))?;
    let memory = state
        .store
        .open_memory(&target_memory_id)
        .map_err(|e| rpc_err(-32603, format!("reasoning_open_target: {e}")))?;
    let bridges = list(state, json!({"memory_id": target_memory_id, "limit": 200}))?;
    let bridge_events = bridges
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let bridge_event_ids = bridge_events
        .iter()
        .filter_map(|event| {
            event
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    let now = unix_now_s();
    let key = format!(
        "reasoning:{}:{}:{}",
        target_memory_id,
        now,
        &Uuid::new_v4().to_string()[..8]
    );
    let artifact = json!({
        "artifact_type": "session_reasoning",
        "target_memory_id": target_memory_id,
        "target_memory": memory.get("memory").cloned().unwrap_or(Value::Null),
        "bridge_events": bridge_events,
        "created_at_s": now,
        "contract": {
            "public_reasoning_only": true,
            "hidden_chain_of_thought": false,
            "memory_body_mutation": false
        }
    });
    let target_key = memory
        .pointer("/memory/key")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let artifact_text = format!(
        "Reasoning bridge artifact for target memory {target_memory_id} ({target_key}). \
It links the saved memory to {} public bridge events so reasoning and nightly review can inspect what was used, why it mattered, and how it connects to future fixes. \
The artifact is evidence-only: it does not edit memory bodies, architecture, schema, identity, provider routing, or gate contracts. \
Bridge event ids: {}. Created at {now}.",
        bridge_events.len(),
        if bridge_event_ids.is_empty() {
            "none".to_string()
        } else {
            bridge_event_ids.join(", ")
        }
    );
    let saved = state
        .store
        .save_memory(SaveInput {
            key: Some(key.clone()),
            value: Some(artifact_text),
            memory_type: Some("session_reasoning".to_string()),
            source: Some("brain.reasoning_bridge".to_string()),
            session_id: memory
                .pointer("/memory/session_id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            metadata: json!({
                "kind": "session_reasoning",
                "target_memory_id": target_memory_id,
                "bridge_event_count": bridge_events.len(),
                "bridge_event_ids": bridge_event_ids,
                "artifact": artifact
            }),
            trusted_generated_artifact: true,
            project_id: None,
            track: None,
        })
        .map_err(|e| rpc_err(-32603, format!("reasoning_artifact_save: {e}")))?;
    let reasoning_memory_id = saved
        .get("memory_id")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32603, "reasoning_artifact_missing_memory_id"))?
        .to_string();
    let ledger = {
        let mut conn = state.store.conn()?;
        let tx = conn
            .transaction()
            .map_err(|e| rpc_err(-32603, format!("reasoning_ledger_tx: {e}")))?;
        let ledger = append_ledger_tx(
            &tx,
            "reasoning.artifact",
            "brain.reasoning_bridge",
            Some(&target_memory_id),
            json!({
                "reasoning_memory_id": reasoning_memory_id,
                "target_memory_id": target_memory_id,
                "bridge_event_count": bridge_events.len()
            }),
            now,
        )
        .map_err(|e| rpc_err(-32603, format!("reasoning_ledger_append: {e}")))?;
        tx.commit()
            .map_err(|e| rpc_err(-32603, format!("reasoning_ledger_commit: {e}")))?;
        ledger
    };
    let ledger_event_id = ledger.get("event_id").and_then(Value::as_str);
    let conn = state.store.conn()?;
    let bridge_event_id = insert_bridge(
        &conn,
        BridgeInsert {
            bridge_kind: "reasoning_artifact",
            memory_id: Some(&target_memory_id),
            source_memory_id: None,
            tool_event_id: None,
            provider_id: None,
            model_id: None,
            request_id: None,
            route_certificate_id: None,
            gate_task_id: None,
            gate_evidence_id: None,
            error_id: None,
            fix_id: None,
            compaction_artifact_id: None,
            nightly_artifact_id: None,
            reasoning_artifact_id: Some(&reasoning_memory_id),
            ledger_event_id,
            relation: json!({
                "target_memory_id": target_memory_id,
                "reasoning_memory_id": reasoning_memory_id,
                "bridge_event_count": bridge_events.len()
            }),
        },
    )?;
    Ok(json!({
        "ok": true,
        "reasoning_memory_id": reasoning_memory_id,
        "key": key,
        "target_memory_id": target_memory_id,
        "bridge_event_count": bridge_events.len(),
        "bridge_event_id": bridge_event_id,
        "save_result": saved,
        "ledger": ledger
    }))
}

/// Trace causal chain within a session: conversation → tool → action → result.
/// Returns compartment-grouped memories with bridge event links.
pub fn causal_chain(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let session_id = first_param(&params, &["session_id", "sessionId"])
        .ok_or_else(|| rpc_err(-32602, "session_id_required"))?;
    let conn = state.store.conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, key, value, memory_type, source, track, project_id, created_at_s, quality_score
             FROM memories
             WHERE session_id = ?1 AND track IS NOT NULL
             ORDER BY track IN ('conversation', 'tool', 'action', 'result'), created_at_s ASC",
        )
        .map_err(|e| rpc_err(-32603, format!("causal_chain_prepare: {e}")))?;
    let rows: Vec<Value> = stmt
        .query_map([session_id.clone()], |row| {
            Ok(json!({
                "memory_id": row.get::<_, String>(0)?,
                "key": row.get::<_, String>(1)?,
                "value": row.get::<_, String>(2)?,
                "memory_type": row.get::<_, String>(3)?,
                "source": row.get::<_, String>(4)?,
                "track": row.get::<_, Option<String>>(5)?,
                "project_id": row.get::<_, Option<String>>(6)?,
                "created_at_s": row.get::<_, i64>(7)?,
                "quality_score": row.get::<_, f64>(8)?
            }))
        })
        .map_err(|e| rpc_err(-32603, format!("causal_chain_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("causal_chain_collect: {e}")))?;
    let mut compartments = serde_json::Map::new();
    for track in &["conversation", "tool", "action", "result"] {
        let track_memories: Vec<Value> = rows
            .iter()
            .filter(|row| row.get("track").and_then(Value::as_str) == Some(track))
            .cloned()
            .collect();
        compartments.insert(
            track.to_string(),
            json!({
                "count": track_memories.len(),
                "memories": track_memories
            }),
        );
    }
    let has_causal = compartments
        .values()
        .all(|v| v.get("count").and_then(Value::as_i64).unwrap_or(0) > 0);
    let bridge_events = conn
        .query_row(
            "SELECT COUNT(*) FROM reasoning_bridge_events
             WHERE memory_id IN (SELECT id FROM memories WHERE session_id = ?1)",
            [session_id.clone()],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0);
    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "has_full_causal_chain": has_causal,
        "total_memories": rows.len(),
        "bridge_event_count": bridge_events,
        "compartments": compartments,
        "chain_order": ["conversation", "tool", "action", "result"]
    }))
}

struct BridgeInsert<'a> {
    bridge_kind: &'a str,
    memory_id: Option<&'a str>,
    source_memory_id: Option<&'a str>,
    tool_event_id: Option<&'a str>,
    provider_id: Option<&'a str>,
    model_id: Option<&'a str>,
    request_id: Option<&'a str>,
    route_certificate_id: Option<&'a str>,
    gate_task_id: Option<&'a str>,
    gate_evidence_id: Option<&'a str>,
    error_id: Option<&'a str>,
    fix_id: Option<&'a str>,
    compaction_artifact_id: Option<&'a str>,
    nightly_artifact_id: Option<&'a str>,
    reasoning_artifact_id: Option<&'a str>,
    ledger_event_id: Option<&'a str>,
    relation: Value,
}

fn insert_bridge(conn: &Connection, input: BridgeInsert<'_>) -> Result<String, RpcError> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO reasoning_bridge_events
            (id, bridge_kind, memory_id, source_memory_id, tool_event_id, provider_id,
             model_id, request_id, route_certificate_id, gate_task_id, gate_evidence_id,
             error_id, fix_id, compaction_artifact_id, nightly_artifact_id,
             reasoning_artifact_id, ledger_event_id, relation_json, created_at_s)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                 ?15, ?16, ?17, ?18, ?19)",
        params![
            id,
            input.bridge_kind,
            input.memory_id,
            input.source_memory_id,
            input.tool_event_id,
            input.provider_id,
            input.model_id,
            input.request_id,
            input.route_certificate_id,
            input.gate_task_id,
            input.gate_evidence_id,
            input.error_id,
            input.fix_id,
            input.compaction_artifact_id,
            input.nightly_artifact_id,
            input.reasoning_artifact_id,
            input.ledger_event_id,
            canonical_json(&input.relation)
                .map_err(|e| rpc_err(-32603, format!("reasoning_bridge_relation_json: {e}")))?,
            unix_now_s(),
        ],
    )
    .map_err(|e| rpc_err(-32603, format!("reasoning_bridge_insert: {e}")))?;
    Ok(id)
}

fn bridge_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "bridge_kind": row.get::<_, String>(1)?,
        "memory_id": row.get::<_, Option<String>>(2)?,
        "source_memory_id": row.get::<_, Option<String>>(3)?,
        "tool_event_id": row.get::<_, Option<String>>(4)?,
        "provider_id": row.get::<_, Option<String>>(5)?,
        "model_id": row.get::<_, Option<String>>(6)?,
        "request_id": row.get::<_, Option<String>>(7)?,
        "route_certificate_id": row.get::<_, Option<String>>(8)?,
        "gate_task_id": row.get::<_, Option<String>>(9)?,
        "gate_evidence_id": row.get::<_, Option<String>>(10)?,
        "error_id": row.get::<_, Option<String>>(11)?,
        "fix_id": row.get::<_, Option<String>>(12)?,
        "compaction_artifact_id": row.get::<_, Option<String>>(13)?,
        "nightly_artifact_id": row.get::<_, Option<String>>(14)?,
        "reasoning_artifact_id": row.get::<_, Option<String>>(15)?,
        "ledger_event_id": row.get::<_, Option<String>>(16)?,
        "relation": parse_json(row.get::<_, String>(17)?),
        "created_at_s": row.get::<_, i64>(18)?
    }))
}

pub fn record_import_bridge(
    state: &BrainState,
    memory_id: &str,
    ledger_event_id: &str,
) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let id = Uuid::new_v4().to_string();
    let relation_json = serde_json::to_string(&json!({
        "source": "import.pack",
        "memory_id": memory_id,
        "bridge_contract": "imported memories are linked to their import ledger event for provenance tracing"
    }))
    .map_err(|e| rpc_err(-32603, format!("import_bridge_relation: {e}")))?;
    conn.execute(
        "INSERT INTO reasoning_bridge_events
         (id, bridge_kind, memory_id, source_memory_id, tool_event_id,
          provider_id, model_id, request_id, route_certificate_id,
          gate_task_id, gate_evidence_id, error_id, fix_id,
          compaction_artifact_id, nightly_artifact_id, reasoning_artifact_id,
          ledger_event_id, relation_json, created_at_s)
         VALUES (?1, 'import', ?2, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, ?3, ?4, ?5)",
        params![id, memory_id, ledger_event_id, relation_json, unix_now_s()],
    )
    .map_err(|e| rpc_err(-32603, format!("import_bridge_insert: {e}")))?;
    Ok(json!({"ok": true, "bridge_event_id": id, "memory_id": memory_id}))
}

pub fn backfill_import_bridges(state: &BrainState) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let existing: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM reasoning_bridge_events WHERE bridge_kind = 'import'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if existing > 0 {
        return Ok(json!({
            "ok": true,
            "status": "already_backfilled",
            "existing_import_bridges": existing
        }));
    }
    let import_events: Vec<String> = conn
        .prepare("SELECT event_id FROM ledger_events WHERE event_type = 'import.pack'")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        })
        .unwrap_or_default();
    let mut total_linked = 0_i64;
    let mut errors = 0_i64;
    for ledger_event_id in &import_events {
        let memory_ids: Vec<String> = conn
            .prepare(
                "SELECT m.id FROM memories m
                 WHERE m.source = 'local-pack'
                 AND NOT EXISTS (
                     SELECT 1 FROM reasoning_bridge_events rb
                     WHERE rb.memory_id = m.id AND rb.bridge_kind = 'import'
                 )",
            )
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get::<_, String>(0))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
            })
            .unwrap_or_default();
        for memory_id in &memory_ids {
            let bridge_id = Uuid::new_v4().to_string();
            let relation_json = serde_json::to_string(&json!({
                "source": "import.pack",
                "memory_id": memory_id,
                "bridge_contract": "backfilled: imported memories linked to their import ledger event for provenance tracing"
            }))
            .unwrap_or_default();
            match conn.execute(
                "INSERT INTO reasoning_bridge_events
                 (id, bridge_kind, memory_id, source_memory_id, tool_event_id,
                  provider_id, model_id, request_id, route_certificate_id,
                  gate_task_id, gate_evidence_id, error_id, fix_id,
                  compaction_artifact_id, nightly_artifact_id, reasoning_artifact_id,
                  ledger_event_id, relation_json, created_at_s)
                 VALUES (?1, 'import', ?2, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, ?3, ?4, ?5)",
                params![bridge_id, memory_id, ledger_event_id, relation_json, unix_now_s()],
            ) {
                Ok(_) => total_linked += 1,
                Err(_) => errors += 1,
            }
        }
    }
    Ok(json!({
        "ok": true,
        "status": "backfilled",
        "import_events": import_events.len(),
        "total_linked": total_linked,
        "errors": errors
    }))
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        1.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn percentile_nearest_rank(values: &[i64], percentile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let rank = ((values.len() as f64) * percentile).ceil().max(1.0) as usize;
    values
        .get(rank.saturating_sub(1))
        .map(|value| *value as f64)
}

fn wilson_lower_bound(successes: usize, total: usize, z: f64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let total = total as f64;
    let successes = successes as f64;
    let p_hat = successes / total;
    let z2 = z * z;
    let denominator = 1.0 + z2 / total;
    let center = p_hat + z2 / (2.0 * total);
    let margin = z * ((p_hat * (1.0 - p_hat) + z2 / (4.0 * total)) / total).sqrt();
    ((center - margin) / denominator).clamp(0.0, 1.0)
}

fn first_param(params: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| params.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn parse_json(raw: String) -> Value {
    serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
}
