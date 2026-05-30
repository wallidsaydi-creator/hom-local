use hom_shared::{RpcError, rpc_err};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

use crate::BrainState;
use crate::db::storage::unix_now_s;
use crate::services::{monitoring, runtime, sessions};

pub async fn snapshot(state: &BrainState, _params: Value) -> Result<Value, RpcError> {
    let now = unix_now_s();
    let runtime_status = runtime::status(state).await?;
    let hierarchy = sessions::hierarchy(state, json!({}))?;
    let active = active_context(state)?;
    let session_compact = capability(&runtime_status, "session_compact");
    let benchmark_plane = benchmark_plane_summary(state)?;
    let monitoring_snapshot = monitoring::snapshot(state).unwrap_or_else(|error| {
        json!({
            "ok": false,
            "warnings": [{
                "id": "monitoring_snapshot_unavailable",
                "message": error.message,
                "diagnostic": {"source": "monitoring.snapshot"}
            }]
        })
    });
    let ledger_valid = runtime_status
        .pointer("/ledger/valid")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let rows = vec![
        contract_row(
            "U01",
            "Runtime Status",
            if ledger_valid {
                "available"
            } else {
                "degraded"
            },
            "runtime.status",
            "runtime",
            json!({
                "daemon": runtime_status.get("daemon").cloned().unwrap_or_else(|| json!({})),
                "ledger": runtime_status.get("ledger").cloned().unwrap_or_else(|| json!({})),
                "memory": runtime_status.get("memory").cloned().unwrap_or_else(|| json!({})),
                "oneLine": runtime_line(&runtime_status)
            }),
            now,
        ),
        contract_row(
            "U02",
            "Project And Session Context",
            if active["session"].is_null() {
                "unavailable"
            } else {
                "available"
            },
            "sqlite.sessions_projects",
            "session",
            json!({
                "active": active,
                "renameCommand": {
                    "method": "sessions.rename",
                    "endpoint": active["session"]["sessionId"].as_str().map(|id| format!("/api/ui/sessions/{id}/rename")),
                    "required_scope": "system"
                }
            }),
            now,
        ),
        contract_row(
            "U03",
            "Agnostic Chat Capability Mesh",
            "available",
            "hom_ingress.capability_mesh",
            "chat",
            json!({
                "enabled": true,
                "disabledReason": null,
                "brainOwnsDispatch": false,
                "meshOwnsDispatch": true,
                "brainOwnsDurableTruth": ["memory", "ledger", "permissions", "reasoning", "nightly", "sessions"],
                "chatCanUse": ["brain.memory.recall", "filesystem", "browser", "network", "shell", "skills", "plugins", "mcp"],
                "traceContract": "external actions append compact action_trace memories back into brain"
            }),
            now,
        ),
        contract_row(
            "U04",
            "Registered Capability Mesh",
            "available",
            "hom_ingress.capability_registry",
            "runtimes",
            json!({
                "brainOwns": ["memory", "ledger", "sessions", "reasoning", "nightly", "permissions", "diagnostics"],
                "externalOwns": ["provider adapters", "tool executors", "skills", "plugins", "mcp", "browser", "filesystem", "shell"],
                "singleAppFacingServer": "127.0.0.1:9101",
                "publicRoutes": [
                    {"method": "POST", "route": "/api/ui/chat", "owner": "capability_mesh"},
                    {"method": "GET", "route": "/api/ui/providers", "owner": "capability_mesh"},
                    {"method": "GET", "route": "/api/ui/tools", "owner": "capability_mesh"},
                    {"method": "GET", "route": "/api/ui/skills", "owner": "capability_mesh"},
                    {"method": "GET", "route": "/api/ui/plugins", "owner": "capability_mesh"},
                    {"method": "GET", "route": "/api/ui/mcp/config", "owner": "capability_mesh"}
                ],
                "stateVocabulary": ["available", "degraded", "denied", "missing", "offline"],
                "handoff": "external executors stay outside the cognitive brain, register descriptors through ingress, and write action traces back into brain memory"
            }),
            now,
        ),
        contract_row(
            "U05",
            "Compaction Payload",
            compact_state(&active, &session_compact),
            "session.compact_and_reasoning_bridge",
            "compaction",
            json!({
                "activeSessionId": active["session"]["sessionId"].clone(),
                "memoryCount": active["session"]["memoryCount"].clone(),
                "command": {
                    "method": "session.compact",
                    "endpoint": "/api/ui/session/compact",
                    "required_scope": "system"
                },
                "inspectorTarget": active["session"]["sessionId"].as_str().map(|id| json!({"kind": "session", "id": id})).unwrap_or(Value::Null)
            }),
            now,
        ),
        contract_row(
            "U06",
            "Project Day Session Hierarchy",
            "available",
            "sessions.hierarchy",
            "projects",
            hierarchy,
            now,
        ),
        contract_row(
            "U07",
            "Diagnostics Availability",
            "available",
            "gates_reasoning_nightly_services",
            "diagnostics",
            diagnostics_payload(now),
            now,
        ),
        contract_row(
            "U08",
            "Settings Fields",
            "available",
            "settings.get",
            "settings",
            settings_field_contract(),
            now,
        ),
        contract_row(
            "U09",
            "Control States",
            "available",
            "runtime.capabilities_and_command_contracts",
            "controls",
            json!({"controls": controls(&active, &session_compact, now)}),
            now,
        ),
        contract_row(
            "U10",
            "Diagnostic Sources",
            "available",
            "ui.contract.snapshot.rows",
            "diagnostics",
            json!({
                "requirement": "every visible brain row, count, badge, and warning carries a source",
                "sources": diagnostic_sources(),
                "negativeStates": [
                    "degraded",
                    "stale_ledger",
                    "brain_unavailable"
                ],
                "negativeStateSource": "monitoring.snapshot"
            }),
            now,
        ),
        contract_row(
            "U11",
            "Benchmark Registry",
            if benchmark_plane["available"].as_bool() == Some(true) {
                "available"
            } else {
                "degraded"
            },
            "ledger_events.benchmark.result",
            "benchmarks",
            json!({
                "artifact_kind": "benchmark_registry_entry_v1",
                "endpoint": "/api/ui/benchmarks",
                "count": benchmark_plane["count"].clone(),
                "available": benchmark_plane["available"].clone(),
                "sources": ["benchmarks.list", "benchmarks.detail"],
                "inspectorTarget": {"kind": "benchmarks", "id": "registry"}
            }),
            now,
        ),
        contract_row(
            "U12",
            "Protocol Conformance Evidence",
            if benchmark_plane["available"].as_bool() == Some(true) {
                "available"
            } else {
                "degraded"
            },
            "ledger_events.benchmark.result",
            "benchmarks",
            json!({
                "artifact_kind": "protocol_conformance_report_v1",
                "negativeStateSource": "ledger_events.benchmark.result",
                "reason": if benchmark_plane["available"].as_bool() == Some(true) {
                    Value::Null
                } else {
                    json!("no benchmark results recorded yet")
                },
                "count": benchmark_plane["count"].clone(),
                "sources": ["benchmarks.list", "benchmarks.detail"]
            }),
            now,
        ),
    ];

    Ok(json!({
        "ok": true,
        "contractId": "hom-local-brain-contract-v2",
        "generatedAtS": now,
        "normalNavigation": [
            {"id": "chat", "label": "Chat", "source": "U03"},
            {"id": "projects", "label": "Projects", "source": "U06"},
            {"id": "settings", "label": "Settings", "source": "U08"}
        ],
        "diagnosticNavigation": [
            {"id": "gates", "label": "Gates", "source": "U07", "normalNavigation": false},
            {"id": "reasoning", "label": "Reasoning", "source": "U07", "normalNavigation": false},
            {"id": "nightly", "label": "Nightly", "source": "U07", "normalNavigation": false}
        ],
        "runtimeLine": runtime_line(&runtime_status),
        "rows": rows,
        "warnings": warnings(&monitoring_snapshot, now),
        "sourcePolicy": {
            "noFakeRows": true,
            "noFallbackRows": true,
            "countsMustCarrySource": true,
            "brainOwnsDurableTruth": true,
            "singleAppFacingServer": true,
            "executorsRegisterToBrain": true
        }
    }))
}

pub fn inspector_target(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let kind = string_param(&params, &["kind", "target_kind", "targetKind"])
        .ok_or_else(|| rpc_err(-32602, "inspector_kind_required"))?;
    let id = string_param(&params, &["id", "target_id", "targetId"])
        .ok_or_else(|| rpc_err(-32602, "inspector_id_required"))?;
    let now = unix_now_s();
    let object = match kind.as_str() {
        "session" => inspect_session(state, &id)?,
        "memory" => inspect_memory(state, &id)?,
        "compaction_artifact" => crate::services::session::open_compaction_artifact(state, &id)?,
        "benchmarks" => crate::services::admin::benchmarks_list(state, json!({}))?,
        "benchmark_result" => {
            crate::services::admin::benchmarks_detail(state, json!({"benchmark_id": id}))?
        }
        "diagnostic" | "runtime" | "chat" | "runtimes" | "settings" | "projects" | "compaction" => {
            json!({
                "id": id,
                "kind": kind,
                "state": "contract_row",
                "source": "ui.contract.snapshot"
            })
        }
        _ => return Err(rpc_err(-32602, "inspector_kind_unsupported")),
    };
    Ok(json!({
        "ok": true,
        "target": {"kind": kind, "id": id},
        "object": object,
        "diagnostic": {
            "source": "ui.inspector.target",
            "backend_backed": true,
            "last_checked_at_s": now
        }
    }))
}

fn active_context(state: &BrainState) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.project_id, s.title, s.created_at_s, s.updated_at_s,
                    COUNT(m.id) AS memory_count,
                    p.name
             FROM sessions s
             LEFT JOIN memories m ON m.session_id = s.id
             LEFT JOIN projects p ON p.id = s.project_id
             WHERE s.active = 1
             GROUP BY s.id, s.project_id, s.title, s.created_at_s, s.updated_at_s, p.name
             LIMIT 1",
        )
        .map_err(|e| rpc_err(-32603, format!("ui_active_session_prepare: {e}")))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| rpc_err(-32603, format!("ui_active_session_query: {e}")))?;
    if let Some(row) = rows
        .next()
        .map_err(|e| rpc_err(-32603, format!("ui_active_session_next: {e}")))?
    {
        let session_id: String = row
            .get(0)
            .map_err(|e| rpc_err(-32603, format!("ui_active_session_id: {e}")))?;
        let project_id: Option<String> = row
            .get(1)
            .map_err(|e| rpc_err(-32603, format!("ui_active_project_id: {e}")))?;
        let title: String = row
            .get(2)
            .map_err(|e| rpc_err(-32603, format!("ui_active_session_title: {e}")))?;
        let created_at_s: i64 = row
            .get(3)
            .map_err(|e| rpc_err(-32603, format!("ui_active_session_created: {e}")))?;
        let updated_at_s: i64 = row
            .get(4)
            .map_err(|e| rpc_err(-32603, format!("ui_active_session_updated: {e}")))?;
        let memory_count: i64 = row
            .get(5)
            .map_err(|e| rpc_err(-32603, format!("ui_active_session_memories: {e}")))?;
        let project_name: Option<String> = row
            .get(6)
            .map_err(|e| rpc_err(-32603, format!("ui_active_project_name: {e}")))?;
        Ok(json!({
            "project": project_id.as_ref().map(|id| json!({
                "projectId": id,
                "name": project_name,
                "diagnostic": {"source": "sqlite.projects"}
            })).unwrap_or(Value::Null),
            "session": {
                "sessionId": session_id,
                "projectId": project_id,
                "title": title,
                "createdAtS": created_at_s,
                "updatedAtS": updated_at_s,
                "memoryCount": memory_count,
                "diagnostic": {"source": "sqlite.sessions_memories"}
            }
        }))
    } else {
        Ok(json!({
            "project": Value::Null,
            "session": Value::Null,
            "diagnostic": {
                "source": "sqlite.sessions",
                "reason": "no active session"
            }
        }))
    }
}

fn benchmark_plane_summary(state: &BrainState) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM ledger_events WHERE event_type = 'benchmark.result'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| rpc_err(-32603, format!("benchmark_plane_summary: {e}")))?;
    Ok(json!({
        "available": count > 0,
        "count": count,
        "source": "ledger_events.benchmark.result"
    }))
}

fn contract_row(
    id: &str,
    label: &str,
    state: &str,
    source: &str,
    inspector_kind: &str,
    payload: Value,
    now: i64,
) -> Value {
    json!({
        "id": id,
        "label": label,
        "title": label,
        "state": state,
        "payload": payload,
        "inspectorTarget": {
            "kind": inspector_kind,
            "id": id
        },
        "diagnostic": {
            "source": source,
            "last_checked_at_s": now,
            "proof": "brain contract row emitted by ui.contract.snapshot"
        }
    })
}

fn capability(runtime_status: &Value, capability_id: &str) -> Option<Value> {
    runtime_status
        .get("capabilities")
        .and_then(Value::as_array)
        .and_then(|capabilities| {
            capabilities
                .iter()
                .find(|capability| capability["id"] == capability_id)
                .cloned()
        })
}

fn runtime_line(runtime_status: &Value) -> Value {
    json!({
        "daemonStatus": runtime_status.pointer("/daemon/status").cloned().unwrap_or(Value::Null),
        "memoryCount": runtime_status.pointer("/memory/count").cloned().unwrap_or_else(|| json!(0)),
        "ledgerValid": runtime_status.pointer("/ledger/valid").cloned().unwrap_or_else(|| json!(false)),
        "source": "runtime.status"
    })
}

fn compact_state(active: &Value, session_compact: &Option<Value>) -> &'static str {
    if active["session"].is_null() {
        return "unavailable";
    }
    if active["session"]["memoryCount"].as_i64().unwrap_or(0) <= 0 {
        return "disabled";
    }
    match session_compact
        .as_ref()
        .and_then(|capability| capability.get("state"))
        .and_then(Value::as_str)
    {
        Some("available") => "available",
        _ => "unavailable",
    }
}

fn diagnostics_payload(now: i64) -> Value {
    json!({
        "normalNavigation": false,
        "entries": [
            {
                "id": "gates",
                "label": "Gates",
                "source": "gates.status/gates.decisions",
                "endpoint": "/api/ui/gates/status",
                "inspectorTarget": {"kind": "diagnostic", "id": "gates"}
            },
            {
                "id": "reasoning",
                "label": "Reasoning",
                "source": "reasoning.artifacts/reasoning.bridge.list",
                "endpoint": "/api/ui/reasoning",
                "inspectorTarget": {"kind": "diagnostic", "id": "reasoning"}
            },
            {
                "id": "nightly",
                "label": "Nightly",
                "source": "nightly.tree/nightly.dry_run",
                "endpoint": "/api/ui/nightly/tree",
                "inspectorTarget": {"kind": "diagnostic", "id": "nightly"}
            }
        ],
        "diagnostic": {
            "source": "brain_diagnostic_services",
            "last_checked_at_s": now
        }
    })
}

fn settings_field_contract() -> Value {
    json!({
        "groups": [
            {"id": "identity", "source": "settings.get.identity"},
            {"id": "enrolledAgents", "source": "settings.get.enrolledAgents"},
            {"id": "osAwareness", "source": "settings.get.osAwareness"},
            {"id": "maintenance", "source": "brain.backup/export.brain/brain.purge reports"},
            {"id": "personalization", "source": "settings.get.personalization"},
            {"id": "retrievalCalibration", "source": "settings.get.retrievalCalibration"}
        ],
        "endpoint": "/api/ui/settings",
        "writeEndpoint": "/api/ui/settings/config"
    })
}

fn controls(active: &Value, session_compact: &Option<Value>, now: i64) -> Vec<Value> {
    vec![
        control(
            "session.rename",
            "Rename Session",
            "sessions.rename",
            active["session"]["sessionId"]
                .as_str()
                .map(|id| format!("/api/ui/sessions/{id}/rename"))
                .unwrap_or_else(|| "/api/ui/sessions/:id/rename".to_string()),
            if active["session"].is_null() {
                "disabled"
            } else {
                "enabled"
            },
            if active["session"].is_null() {
                json!("no active session")
            } else {
                Value::Null
            },
            "sqlite.sessions",
            now,
        ),
        control(
            "session.compact",
            "Compact",
            "session.compact",
            "/api/ui/session/compact",
            compact_state(active, session_compact),
            if active["session"].is_null() {
                json!("no active session")
            } else if active["session"]["memoryCount"].as_i64().unwrap_or(0) <= 0 {
                json!("session has no memories to compact")
            } else {
                session_compact
                    .as_ref()
                    .and_then(|capability| capability.get("reason"))
                    .cloned()
                    .unwrap_or(Value::Null)
            },
            "runtime.status.capabilities.session_compact",
            now,
        ),
        control(
            "nightly.dry_run",
            "Nightly Dry Run",
            "nightly.dry_run",
            "/api/ui/nightly/dry-run",
            "enabled",
            Value::Null,
            "nightly.dry_run",
            now,
        ),
    ]
}

fn control(
    id: &str,
    label: &str,
    method: &str,
    endpoint: impl Into<String>,
    state: &str,
    reason: Value,
    source: &str,
    now: i64,
) -> Value {
    json!({
        "id": id,
        "label": label,
        "method": method,
        "endpoint": endpoint.into(),
        "state": state,
        "enabled": state == "enabled" || state == "available",
        "unavailableReason": if reason.is_null() { Value::Null } else { reason },
        "loading": {
            "current": Value::Null,
            "owner": "client_action_lifecycle",
            "backendSupportsLoadingState": true
        },
        "diagnostic": {
            "source": source,
            "last_checked_at_s": now
        }
    })
}

fn diagnostic_sources() -> Vec<Value> {
    vec![
        json!({"id": "runtime", "source": "runtime.status"}),
        json!({"id": "memory_counts", "source": "sqlite.memories"}),
        json!({"id": "ledger_badge", "source": "ledger_events hash-chain verification"}),
        json!({"id": "sessions", "source": "sqlite.sessions_memories"}),
        json!({"id": "settings", "source": "settings table and typed services"}),
    ]
}

fn warnings(monitoring_snapshot: &Value, now: i64) -> Vec<Value> {
    let mut warnings = monitoring_snapshot
        .get("warnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    if warnings.is_empty() && monitoring_snapshot.get("ok").and_then(Value::as_bool) != Some(true) {
        warnings.push(warning(
            "monitoring_snapshot_unavailable",
            "monitoring snapshot is unavailable",
            "monitoring.snapshot",
            now,
        ));
    }
    warnings
}

fn warning(id: &str, message: &str, source: &str, now: i64) -> Value {
    json!({
        "id": id,
        "message": message,
        "diagnostic": {
            "source": source,
            "last_checked_at_s": now
        }
    })
}

fn string_param(params: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| params.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn inspect_session(state: &BrainState, id: &str) -> Result<Value, RpcError> {
    sessions::detail(state, json!({"id": id}))
}

fn inspect_memory(state: &BrainState, id: &str) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    conn.query_row(
        "SELECT id, key, value, memory_type, source, session_id, project_id,
                track, quality_score, metadata_json, created_at_s, updated_at_s
         FROM memories WHERE id = ?1",
        [id],
        |row| {
            let metadata: String = row.get(9)?;
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "key": row.get::<_, String>(1)?,
                "value": row.get::<_, String>(2)?,
                "memoryType": row.get::<_, String>(3)?,
                "source": row.get::<_, String>(4)?,
                "sessionId": row.get::<_, Option<String>>(5)?,
                "projectId": row.get::<_, Option<String>>(6)?,
                "track": row.get::<_, Option<String>>(7)?,
                "qualityScore": row.get::<_, f64>(8)?,
                "metadata": serde_json::from_str::<Value>(&metadata).unwrap_or_else(|_| json!({})),
                "createdAtS": row.get::<_, i64>(10)?,
                "updatedAtS": row.get::<_, i64>(11)?
            }))
        },
    )
    .optional()
    .map_err(|e| rpc_err(-32603, format!("inspector_memory_query: {e}")))?
    .ok_or_else(|| rpc_err(-32004, "inspector_target_not_found"))
}
