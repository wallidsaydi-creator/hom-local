use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::adapter;
use crate::http::AppState;
use hom_shared::sha256_hex;

#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
    intent: Option<String>,
    min_confidence: Option<String>,
    hide_contradicted: Option<bool>,
    hide_superseded: Option<bool>,
    show_low_confidence: Option<bool>,
    limit: Option<u64>,
}

#[derive(Deserialize)]
pub struct SessionsQuery {
    project_id: Option<String>,
}

#[derive(Deserialize)]
pub struct GateListQuery {
    task_id: Option<String>,
    plan_version_id: Option<String>,
    limit: Option<i64>,
}

fn runtime_bubble_response(runtime: &str, method: &str, route: &str) -> Response {
    (
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "ok": false,
            "error": {
                "code": -32090,
                "message": "capability_unavailable",
                "data": {
                    "runtime": runtime,
                    "route": route,
                    "method": method,
                    "retryable": true,
                    "state": "offline",
                    "reason": format!("{runtime} capability is not routeable in the single-server mesh yet"),
                    "required_action": "Register the capability with the HOM capability mesh.",
                    "brain_forwarded": false
                }
            }
        })),
    )
        .into_response()
}

fn runtime_bubble_offline(runtime: &str, method: Method, uri: axum::http::Uri) -> Response {
    runtime_bubble_response(runtime, method.as_str(), uri.path())
}

pub async fn provider_runtime_offline(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("ProviderRuntime", method, uri)
}

pub async fn tools_runtime_offline(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("ToolsRuntime", method, uri)
}

pub async fn skills_runtime_offline(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("SkillsRuntime", method, uri)
}

pub async fn plugins_runtime_offline(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("PluginsRuntime", method, uri)
}

pub async fn mcp_runtime_offline(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("MCPRuntime", method, uri)
}

pub async fn mcp_status(State(state): State<Arc<AppState>>) -> Response {
    let (status, value) = state.mesh.mcp_status();
    (status, Json(value)).into_response()
}

pub async fn mcp_tools(State(state): State<Arc<AppState>>) -> Response {
    let (status, value) = state.mesh.mcp_tools();
    (status, Json(value)).into_response()
}

pub async fn mcp_connect(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let server_id = body
        .get("server_id")
        .or_else(|| body.get("serverId"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string());
    let (status, value) = state.mesh.mcp_connect(server_id);
    (status, Json(value)).into_response()
}

pub async fn mcp_create(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let (status, value) = state.mesh.mcp_create(body);
    (status, Json(value)).into_response()
}

pub async fn mcp_server_connect(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let (status, value) = state.mesh.mcp_connect(Some(id));
    (status, Json(value)).into_response()
}

pub async fn mcp_server_enable(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let (status, value) = state.mesh.mcp_lifecycle_mutation_blocked("enable", id);
    (status, Json(value)).into_response()
}

pub async fn mcp_server_disable(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let (status, value) = state.mesh.mcp_lifecycle_mutation_blocked("disable", id);
    (status, Json(value)).into_response()
}

pub async fn mcp_call_tool(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let (status, value) = state.mesh.mcp_call_tool(body);
    (status, Json(value)).into_response()
}

pub async fn workspace_session_runtime_offline(
    method: Method,
    OriginalUri(uri): OriginalUri,
) -> Response {
    runtime_bubble_offline("WorkspaceSessionRuntime", method, uri)
}

pub async fn sessions_create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    match state.brain.call("sessions.create", body, "system").await {
        Ok(result) => {
            let session = adapter::session_summary_payload(json!({
                "session_id": result.get("sessionId").cloned().unwrap_or(Value::Null),
                "project_id": result.get("projectId").cloned().unwrap_or(Value::Null),
                "title": result.get("title").cloned().unwrap_or(Value::Null),
                "memory_count": result.get("memoryCount").cloned().unwrap_or(json!(0)),
                "created_at_s": result.get("createdAtS").cloned().unwrap_or(Value::Null),
                "updated_at_s": result.get("updatedAtS").cloned().unwrap_or(Value::Null),
                "active": result.get("active").cloned().unwrap_or(json!(false)),
            }));
            (
                axum::http::StatusCode::CREATED,
                Json(json!({
                    "ok": true,
                    "session": session,
                    "ledger": result.get("ledger").cloned().unwrap_or(Value::Null),
                })),
            )
                .into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn permission_sandbox_runtime_offline(
    method: Method,
    OriginalUri(uri): OriginalUri,
) -> Response {
    runtime_bubble_offline("PermissionPolicyRuntime", method, uri)
}

pub async fn permissions_policy(State(state): State<Arc<AppState>>) -> Response {
    match state
        .brain
        .call("permissions.get", json!({}), "system")
        .await
    {
        Ok(result) => {
            let adapted = adapter::adapt_response("permissions.get", &result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn sandbox_profiles() -> Response {
    (
        axum::http::StatusCode::OK,
        Json(json!({
            "ok": true,
            "owner": "sandbox-runner-service",
            "platform": "macos",
            "runner_state": "not_implemented",
            "security_boundary": "native_runner_required",
            "electron_boundary": false,
            "profiles": sandbox_profiles_payload(),
            "reason": "HOM does not yet have a real sandbox runner; these are backend contracts for the macOS Seatbelt runner boundary."
        })),
    )
        .into_response()
}

pub async fn sandbox_preview(Json(body): Json<Value>) -> Response {
    let profile = sandbox_profile_from_body(&body);
    let session_id = string_field(&body, "session_id").unwrap_or("unknown");
    let tool_id = string_field(&body, "tool_id").unwrap_or("unknown");
    let command = string_field(&body, "command");
    let cwd = string_field(&body, "cwd").unwrap_or(".");
    let workspace_root = string_field(&body, "workspace_root").unwrap_or(cwd);

    (
        axum::http::StatusCode::OK,
        Json(json!({
            "ok": true,
            "status": "previewed",
            "executed": false,
            "owner": "sandbox-runner-service",
            "runner_state": "not_implemented",
            "stages": [
                {
                    "name": "permission_policy_decision",
                    "owner": "permission-policy-service",
                    "security_boundary": "policy_gate_only"
                },
                {
                    "name": "sandbox_runner_execution",
                    "owner": "sandbox-runner-service",
                    "security_boundary": "native_runner_required",
                    "state": "not_implemented"
                }
            ],
            "certificate_preview": {
                "request_id": format!("sandbox_preview:{session_id}:{tool_id}"),
                "session_id": session_id,
                "tool_id": tool_id,
                "command": command,
                "cwd": cwd,
                "workspace_root": workspace_root,
                "sandbox_profile": profile,
                "allowed_roots": [workspace_root],
                "allowed_unix_sockets": [],
                "denied_operations": [],
                "stdout": null,
                "stderr": null,
                "exit_code": null,
                "status": "previewed",
                "route_certificate_id": null
            }
        })),
    )
        .into_response()
}

pub async fn sandbox_execute(Json(body): Json<Value>) -> Response {
    let profile = sandbox_profile_from_body(&body);
    (
        axum::http::StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "ok": false,
            "status": "blocked",
            "executed": false,
            "owner": "sandbox-runner-service",
            "runner_state": "not_implemented",
            "sandbox_profile": profile,
            "error": {
                "code": "sandbox_runner_not_implemented",
                "message": "HOM does not yet have a native sandbox runner; command and mutation execution remain blocked."
            }
        })),
    )
        .into_response()
}

pub async fn sandbox_denials() -> Response {
    (
        axum::http::StatusCode::OK,
        Json(json!({
            "ok": true,
            "source": "sandbox-runner-service",
            "runner_state": "not_implemented",
            "denials": [],
            "reason": "No denial log exists because the native sandbox runner is not implemented."
        })),
    )
        .into_response()
}

fn sandbox_profiles_payload() -> Value {
    json!([
        {
            "id": "read-only",
            "platform": "macos",
            "seatbelt_profile": "read_only",
            "filesystem_write": false,
            "dangerous": false,
            "bypasses_sandbox": false
        },
        {
            "id": "workspace-write",
            "platform": "macos",
            "seatbelt_profile": "workspace_write",
            "filesystem_write": "workspace_root_and_additional_writable_roots_only",
            "dangerous": false,
            "bypasses_sandbox": false
        },
        {
            "id": "danger-full-access",
            "platform": "macos",
            "seatbelt_profile": "full_access",
            "filesystem_write": true,
            "dangerous": true,
            "bypasses_sandbox": false
        },
        {
            "id": "dangerously-bypass-approvals-and-sandbox",
            "platform": "macos",
            "seatbelt_profile": null,
            "filesystem_write": true,
            "dangerous": true,
            "bypasses_sandbox": true
        }
    ])
}

fn sandbox_profile_from_body(body: &Value) -> &str {
    string_field(body, "profile").unwrap_or("read-only")
}

fn string_field<'a>(body: &'a Value, key: &str) -> Option<&'a str> {
    body.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

pub async fn chat_composer_policy_runtime_offline(
    method: Method,
    OriginalUri(uri): OriginalUri,
) -> Response {
    runtime_bubble_offline("ChatComposerPolicyRuntime", method, uri)
}

pub async fn chat_composer_actions(State(state): State<Arc<AppState>>) -> Response {
    let (_, tools_payload) = state.mesh.tools();
    let (_, skills_payload) = state.mesh.skills();
    let (_, plugins_payload) = state.mesh.plugins();
    let (_, mcp_payload) = state.mesh.mcp_servers();
    let permissions = state.permission_snapshot().await;

    let tools = tools_payload["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut category_counts = serde_json::Map::new();
    for tool in &tools {
        let category = tool
            .get("category")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let count = category_counts
            .get(&category)
            .and_then(Value::as_i64)
            .unwrap_or(0)
            + 1;
        category_counts.insert(category, json!(count));
    }
    let tool_total = tools.len();
    let skill_total = skills_payload["total"].as_i64().unwrap_or(0);
    let plugin_total = plugins_payload["total"].as_i64().unwrap_or(0);
    let mcp_total = mcp_payload["total"].as_i64().unwrap_or(0);

    (
        axum::http::StatusCode::OK,
        Json(json!({
            "ok": true,
            "artifact_kind": "chat_composer_dynamic_action_registry_v1",
            "registry_version": "phase4.4",
            "read_only": true,
            "generated_from": {
                "tools_route": "/api/ui/tools",
                "permissions_route": "/api/ui/permissions",
                "skills_route": "/api/ui/skills",
                "plugins_route": "/api/ui/plugins",
                "mcp_route": "/api/ui/mcp-servers",
                "provider_route": "/api/ui/providers",
                "model_route": "/api/ui/providers/:id/models",
                "source": "capability_mesh_and_permission_sandbox"
            },
            "actions": {
                "tools": {
                    "state": if tool_total > 0 { "enabled" } else { "unavailable" },
                    "owner": "tools-runtime/capability-mesh",
                    "route": "/api/ui/tools",
                    "clickable": tool_total > 0,
                    "tool_count": tool_total,
                    "category_counts": Value::Object(category_counts),
                    "reason": "Tool inventory is derived from the runtime capability mesh; composer UI may inspect real tools but must not fake execution success.",
                    "missing_contract": if tool_total > 0 { Value::Null } else { json!("Capability mesh tool descriptors") },
                    "owner_boundary_risk": false
                },
                "permissions": {
                    "state": "enabled",
                    "owner": "permission-policy-service",
                    "route": "/api/ui/permissions",
                    "clickable": true,
                    "active_profile": permissions["profile"],
                    "policy": permissions["policy"],
                    "security_boundary": "policy_gate_only",
                    "sandbox_runner": "separate_service_required",
                    "sandbox_runner_state": "not_implemented",
                    "reason": "Permission action state is derived from the backend permission-policy-service policy snapshot. This is not a real sandbox runner.",
                    "missing_contract": null,
                    "owner_boundary_risk": false
                },
                "access": {
                    "state": "available",
                    "owner": "permission-policy-service",
                    "route": "/api/ui/permissions/policy",
                    "clickable": true,
                    "active_profile": permissions["profile"],
                    "profiles": [
                        {
                            "id": "sandbox",
                            "label": "Sandbox",
                            "brain_profile": "restricted",
                            "network_enabled": false,
                            "filesystem_write": false,
                            "dangerous": false
                        },
                        {
                            "id": "review",
                            "label": "Review",
                            "brain_profile": "workspace",
                            "network_enabled": true,
                            "filesystem_write": false,
                            "dangerous": false
                        },
                        {
                            "id": "full",
                            "label": "Full Access",
                            "brain_profile": "full_access",
                            "network_enabled": true,
                            "filesystem_write": true,
                            "dangerous": true
                        }
                    ],
                    "set_route": "/api/ui/permissions/profile",
                    "reason": "Access tray profiles are backed by the brain permission-policy-service. Profile changes persist through brain settings.",
                    "missing_contract": Value::Null,
                    "owner_boundary_risk": false
                },
                "skills": {
                    "state": "available",
                    "owner": "skills-runtime/capability-mesh",
                    "route": "/api/ui/skills",
                    "clickable": true,
                    "skill_count": skill_total,
                    "import_route": "/api/ui/session/skills/import",
                    "remove_route": "/api/ui/session/skills/remove",
                    "reason": "Skill listing, session-bound import/remove, and compaction continuity are wired. No per-skill enable/disable toggle yet.",
                    "missing_contract": Value::Null,
                    "owner_boundary_risk": false
                },
                "plugins": {
                    "state": "available",
                    "owner": "plugins-runtime",
                    "route": "/api/ui/plugins",
                    "clickable": true,
                    "plugin_count": plugin_total,
                    "install_route": "/api/ui/plugins/install",
                    "enable_route": "/api/ui/plugins/enable",
                    "disable_route": "/api/ui/plugins/disable",
                    "uninstall_route": "/api/ui/plugins/uninstall",
                    "reason": "Plugin install/uninstall/enable/disable are wired with durable store. Marketplace catalog is empty. No brain methods for plugins.",
                    "missing_contract": Value::Null,
                    "owner_boundary_risk": false
                },
                "mcp": {
                    "state": "available",
                    "owner": "mcp-runtime",
                    "route": "/api/ui/mcp-servers",
                    "clickable": true,
                    "server_count": mcp_total,
                    "connect_route": "/api/ui/mcp/connect",
                    "create_route": "/api/ui/mcp/create",
                    "status_route": "/api/ui/mcp/status",
                    "tools_route": "/api/ui/mcp/tools",
                    "call_route": "/api/ui/mcp/tools/call",
                    "reason": "MCP server discovery, connect (stdio), enable/disable, live status, and tool-list/call are wired. HTTP-transport sessions not implemented.",
                    "missing_contract": Value::Null,
                    "owner_boundary_risk": false
                },
                "provider": {
                    "state": "available",
                    "owner": "provider-runtime/capability-mesh",
                    "route": "/api/ui/providers",
                    "clickable": true,
                    "reason": "Provider catalog and model selection are live; POST /api/ui/model/select persists authoritative provider/model route.",
                    "missing_contract": Value::Null,
                    "owner_boundary_risk": false
                },
                "model": {
                    "state": "available",
                    "owner": "provider-runtime/capability-mesh",
                    "route": "/api/ui/model/select",
                    "clickable": true,
                    "reason": "Model selection is wired: POST persists provider_id/model_id/reasoning_effort to brain settings; GET returns current selection.",
                    "missing_contract": Value::Null,
                    "owner_boundary_risk": false
                },
                "reasoning": {
                    "state": "available",
                    "owner": "chat-composer-policy-runtime",
                    "route": "/api/ui/reasoning-policy",
                    "clickable": true,
                    "reason": "Reasoning effort policy is wired: GET/POST /api/ui/reasoning-policy reads/writes effort level backed by brain settings.",
                    "missing_contract": Value::Null,
                    "owner_boundary_risk": false
                },
                "side_chat_new": {
                    "state": "available",
                    "owner": "workspace-session-runtime",
                    "route": "/api/ui/sessions",
                    "method": "POST",
                    "clickable": true,
                    "reason": "POST /api/ui/sessions creates a real backend session with ledger event and optional activation",
                    "owner_boundary_risk": false
                },
                "workspace": {
                    "state": "available",
                    "owner": "capability-mesh/filesystem-bridge",
                    "route": "/api/ui/workspace/files/list",
                    "clickable": true,
                    "read_profile": "review",
                    "write_profile": "full",
                    "routes": {
                        "list": "/api/ui/workspace/files/list",
                        "read": "/api/ui/workspace/files/read",
                        "search": "/api/ui/workspace/files/search",
                        "write_preview": "/api/ui/workspace/files/write-preview",
                        "write_apply": "/api/ui/workspace/files/write-apply"
                    },
                    "reason": "Workspace filesystem bridge delegates to capability_mesh filesystem tools with permission gate: read ops require review or full; write ops require full.",
                    "missing_contract": Value::Null,
                    "owner_boundary_risk": false
                },
                "attachments": {
                    "state": "available",
                    "owner": "ingress-filesystem-bridge",
                    "route": "/api/ui/attachments/inspect",
                    "clickable": true,
                    "reason": "Attachment inspection resolves local file metadata (kind, size, extension, text preview) for chat composer drag-drop. Ingress-native; no brain round-trip.",
                    "missing_contract": Value::Null,
                    "owner_boundary_risk": false
                }
            }
        })),
    )
        .into_response()
}

pub async fn session(State(state): State<Arc<AppState>>) -> Response {
    match state.brain.call("session.get", json!({}), "system").await {
        Ok(result) => {
            let adapted = adapter::adapt_response("session.get", &result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub async fn login(State(state): State<Arc<AppState>>) -> Response {
    match state.brain.call("session.login", json!({}), "system").await {
        Ok(result) => {
            let adapted = adapter::adapt_response("session.login", &result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub async fn logout(State(state): State<Arc<AppState>>) -> Response {
    match state
        .brain
        .call("session.logout", json!({}), "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub async fn status(State(state): State<Arc<AppState>>) -> Response {
    let status = state.brain.call("system.status", json!({}), "system").await;
    let health = state.brain.call("system.health", json!({}), "system").await;

    match (status, health) {
        (Ok(s), Ok(h)) => {
            let mut result = adapter::adapt_response("system.status", &s);
            if let Some(daemon) = result.get_mut("daemon").and_then(Value::as_object_mut) {
                daemon.insert(
                    "version".to_string(),
                    h.get("version").cloned().unwrap_or(json!("0.1.0")),
                );
            }
            (axum::http::StatusCode::OK, Json(result)).into_response()
        }
        (Err(error), _) | (_, Err(error)) => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"ok": false, "ready": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub async fn runtime_status(State(state): State<Arc<AppState>>) -> Response {
    match tokio::time::timeout(
        Duration::from_secs(3),
        state.brain.call("runtime.status", json!({}), "system"),
    )
    .await
    {
        Ok(Ok(result)) => {
            let result = state.mesh.enrich_runtime_status(result).await;
            (axum::http::StatusCode::OK, Json(result)).into_response()
        }
        Ok(Err(error)) => (
            crate::http::brain_error_status(&error),
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
        Err(_) => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "ok": false,
                "timeout": true,
                "daemon": {"status": "degraded", "reason": "runtime_status_timeout"},
                "memory": {"count": 0},
                "ledger": {"valid": false, "pending": true},
                "capabilities": [],
                "error": "runtime.status did not respond within 3 seconds"
            })),
        )
            .into_response(),
    }
}

pub async fn ui_contract(State(state): State<Arc<AppState>>) -> Response {
    match state
        .brain
        .call("ui.contract.snapshot", json!({}), "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn ledger_verify(State(state): State<Arc<AppState>>) -> Response {
    match state.brain.call("ledger.verify", json!({}), "system").await {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => (
            crate::http::brain_error_status(&error),
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub async fn ledger_repair_segmented(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    match state
        .brain
        .call("ledger.repair_segmented", body, "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => (
            crate::http::brain_error_status(&error),
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub async fn ledger_record_mutation(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    match state
        .brain
        .call("ledger.record_mutation", body, "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => (
            crate::http::brain_error_status(&error),
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub async fn ledger_reconcile_mismatches(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    match state
        .brain
        .call("ledger.reconcile_mismatches", body, "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => (
            crate::http::brain_error_status(&error),
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub async fn monitoring_snapshot(State(state): State<Arc<AppState>>) -> Response {
    match state
        .brain
        .call("monitoring.snapshot", json!({}), "diagnostics:read")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => (
            crate::http::brain_error_status(&error),
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub async fn security_saber_dry_run(State(state): State<Arc<AppState>>) -> Response {
    match state
        .brain
        .call("security.saber_dry_run", json!({}), "security:read")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => (
            crate::http::brain_error_status(&error),
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub async fn security_canary_timeline(State(state): State<Arc<AppState>>) -> Response {
    match state
        .brain
        .call(
            "security.canary_timeline",
            json!({"limit": 50}),
            "security:read",
        )
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => (
            crate::http::brain_error_status(&error),
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub async fn security_status(State(state): State<Arc<AppState>>) -> Response {
    let dry_run = state
        .brain
        .call("security.saber_dry_run", json!({}), "security:read")
        .await;
    let timeline = state
        .brain
        .call(
            "security.canary_timeline",
            json!({"limit": 50}),
            "security:read",
        )
        .await;
    match (dry_run, timeline) {
        (Ok(dry_run), Ok(timeline)) => (
            axum::http::StatusCode::OK,
            Json(json!({
                "ok": true,
                "saber_dry_run": dry_run,
                "canary_timeline": timeline
            })),
        )
            .into_response(),
        (Err(error), _) | (_, Err(error)) => (
            crate::http::brain_error_status(&error),
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub async fn diagnostics_capability(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state
        .brain
        .call(
            "diagnostics.capability",
            json!({"capability_id": id}),
            "diagnostics:read",
        )
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => (
            crate::http::brain_error_status(&error),
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub async fn gates_status(State(state): State<Arc<AppState>>) -> Response {
    gate_read(state, "gates.runtime.snapshot", json!({})).await
}

pub async fn gates_plan_current(State(state): State<Arc<AppState>>) -> Response {
    gate_read(state, "gates.plan.current", json!({})).await
}

pub async fn gates_prompt_create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    gate_write(
        state,
        headers,
        "/api/ui/gates/prompt",
        "gates.prompt.create",
        body,
    )
    .await
}

pub async fn gates_plan_propose(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    gate_write(
        state,
        headers,
        "/api/ui/gates/plan",
        "gates.plan.propose",
        body,
    )
    .await
}

pub async fn gates_plan_approve(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Response {
    let signed_body = body.clone();
    body["plan_id"] = json!(id);
    gate_write_with_brain_body(
        state,
        headers,
        &format!(
            "/api/ui/gates/plan/{}/approve",
            body["plan_id"].as_str().unwrap_or("")
        ),
        "gates.plan.approve",
        signed_body,
        body,
    )
    .await
}

pub async fn gates_tasks_derive(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    gate_write(
        state,
        headers,
        "/api/ui/gates/tasks/derive",
        "gates.tasks.derive",
        body,
    )
    .await
}

pub async fn gates_tasks_preflight(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    gate_write(
        state,
        headers,
        "/api/ui/gates/tasks/preflight",
        "gates.tasks.preflight",
        body,
    )
    .await
}

pub async fn gates_amendment_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    gate_write(
        state,
        headers,
        "/api/ui/gates/amendments",
        "gates.amendment.request",
        body,
    )
    .await
}

pub async fn gates_amendment_approve(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Response {
    let signed_body = body.clone();
    body["amendment_id"] = json!(id);
    gate_write_with_brain_body(
        state,
        headers,
        &format!(
            "/api/ui/gates/amendments/{}/approve",
            body["amendment_id"].as_str().unwrap_or("")
        ),
        "gates.amendment.approve",
        signed_body,
        body,
    )
    .await
}

pub async fn gates_runtime_verify(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    gate_write(
        state,
        headers,
        "/api/ui/gates/runtime/verify",
        "gates.runtime.verify",
        body,
    )
    .await
}

pub async fn gates_decisions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GateListQuery>,
) -> Response {
    let params = gate_query_params(query);
    gate_read(state, "gates.decisions.list", params).await
}

pub async fn gates_evidence_list(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GateListQuery>,
) -> Response {
    let params = gate_query_params(query);
    gate_read(state, "gates.evidence.list", params).await
}

pub async fn gates_evidence_submit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    gate_write(
        state,
        headers,
        "/api/ui/gates/evidence",
        "gates.evidence.submit",
        body,
    )
    .await
}

pub async fn gates_tasks_complete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Response {
    let signed_body = body.clone();
    body["task_id"] = json!(id);
    gate_write_with_brain_body(
        state,
        headers,
        &format!(
            "/api/ui/gates/tasks/{}/complete",
            body["task_id"].as_str().unwrap_or("")
        ),
        "gates.tasks.complete",
        signed_body,
        body,
    )
    .await
}

pub async fn gates_argument_build(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    gate_write(
        state,
        headers,
        "/api/ui/gates/argument/build",
        "gates.argument.build",
        body,
    )
    .await
}

pub async fn gates_argument_inspect(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    gate_read(state, "gates.argument.inspect", json!(query)).await
}

pub async fn gates_argument_accept(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Response {
    let signed_body = body.clone();
    body["argument_id"] = json!(id);
    gate_write_with_brain_body(
        state,
        headers,
        &format!(
            "/api/ui/gates/argument/{}/accept",
            body["argument_id"].as_str().unwrap_or("")
        ),
        "gates.argument.accept",
        signed_body,
        body,
    )
    .await
}

pub async fn search(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> Response {
    let q = query.q.unwrap_or_default();
    let limit = query.limit.unwrap_or(12).min(50);
    let params = json!({
        "query": q,
        "limit": limit,
        "intent": query.intent.unwrap_or_else(|| "project".to_string()),
        "min_confidence": query.min_confidence,
        "filters": {
            "hide_contradicted": query.hide_contradicted.unwrap_or(false),
            "hide_superseded": query.hide_superseded.unwrap_or(false),
            "show_low_confidence": query.show_low_confidence.unwrap_or(false)
        },
        "surface": "tauri_search_tab"
    });

    match state
        .brain
        .call("memory.recall", params.clone(), "memory:recall")
        .await
    {
        Ok(result) => {
            let adapted = adapter::adapt_response("memory.recall", &result);
            let results = normalize_search_results(&adapted);
            (
                axum::http::StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "source": "brain-recall-runtime",
                    "owner_service": "hom-brain",
                    "route": "/api/ui/search",
                    "query": params["query"],
                    "results": results,
                    "raw": adapted,
                    "meta": {"brain_forwarded": true, "static_sample": false}
                })),
            )
                .into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({
                    "ok": false,
                    "source": "brain-recall-runtime",
                    "owner_service": "hom-brain",
                    "route": "/api/ui/search",
                    "meta": {"brain_forwarded": true, "static_sample": false},
                    "error": error.error_payload()
                })),
            )
                .into_response()
        }
    }
}

fn normalize_search_results(adapted: &Value) -> Vec<Value> {
    let source = adapted
        .get("results")
        .or_else(|| adapted.get("evidence"))
        .or_else(|| adapted.get("memories"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    source
        .into_iter()
        .enumerate()
        .map(|(idx, item)| {
            let id = item
                .get("id")
                .or_else(|| item.get("memory_id"))
                .or_else(|| item.get("session_id"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("search.result.{idx}"));
            let title = item
                .get("title")
                .or_else(|| item.get("key"))
                .or_else(|| item.get("memory_key"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| id.clone());
            let excerpt = item
                .get("excerpt")
                .or_else(|| item.get("preview"))
                .or_else(|| item.get("text"))
                .or_else(|| item.get("value"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let score = item
                .get("confidence_score")
                .or_else(|| item.get("quality_score"))
                .or_else(|| item.get("score"))
                .and_then(Value::as_f64)
                .unwrap_or(0.5);
            let confidence = item
                .get("confidence")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if score >= 0.75 {
                        "high".to_string()
                    } else if score >= 0.4 {
                        "medium".to_string()
                    } else {
                        "low".to_string()
                    }
                });
            json!({
                "id": id,
                "title": title,
                "excerpt": excerpt,
                "source_type": item.get("source_type").or_else(|| item.get("type")).and_then(Value::as_str).unwrap_or("memory"),
                "confidence": confidence,
                "confidence_score": score,
                "intent_matches": item.get("intent_matches").cloned().unwrap_or_else(|| json!(["project", "words", "context"])),
                "contradicted": item.get("contradicted").and_then(Value::as_bool).unwrap_or(false),
                "superseded": item.get("superseded").and_then(Value::as_bool).unwrap_or(false),
                "updated_at": item.get("updated_at").or_else(|| item.get("created_at")).cloned().unwrap_or(Value::Null),
                "route": "/api/ui/search"
            })
        })
        .collect()
}

pub async fn recall(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let _identity =
        match crate::auth::verify_ui_request(&state, "POST", "/api/ui/recall", &headers, &body)
            .await
        {
            Ok(id) => id,
            Err((status, error)) => return (status, Json(error)).into_response(),
        };

    match state
        .brain
        .call("memory.recall", body, "memory:recall")
        .await
    {
        Ok(result) => {
            let adapted = adapter::adapt_response("memory.recall", &result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn recall_smart(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let _identity = match crate::auth::verify_ui_request(
        &state,
        "POST",
        "/api/ui/recall/smart",
        &headers,
        &body,
    )
    .await
    {
        Ok(id) => id,
        Err((status, error)) => return (status, Json(error)).into_response(),
    };

    let mut params = body;
    if params.get("mode").is_none() {
        params["mode"] = json!("auto");
    }

    match state
        .brain
        .call("memory.recall", params, "memory:recall")
        .await
    {
        Ok(result) => {
            let adapted = adapter::adapt_response("memory.recall.smart", &result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn chat(State(state): State<Arc<AppState>>, Json(mut body): Json<Value>) -> Response {
    // Check what the client provided before mutating the body.
    let has_provider_id = body.get("provider_id").and_then(Value::as_str).is_some()
        || body.get("providerId").and_then(Value::as_str).is_some();
    let has_model = body.get("model").and_then(Value::as_str).is_some()
        || body.get("model_id").and_then(Value::as_str).is_some()
        || body.get("modelId").and_then(Value::as_str).is_some();
    let has_reasoning_effort = body
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .is_some()
        || body
            .get("reasoningEffort")
            .and_then(Value::as_str)
            .is_some();

    // Load cached model selection as defaults for fields the client didn't provide.
    let selected = state.selected_model_snapshot().await;

    // Collect session-bound skills and enabled plugins for context injection.
    let bound_skills = state.session_skills.lock().await.clone();
    let (_, plugins_payload) = state.mesh.plugins();
    let enabled_plugins: Vec<Value> = plugins_payload["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p["enabled"].as_bool().unwrap_or(false))
        .collect();
    let (_, mcp_payload) = state.mesh.mcp_servers();
    let connected_mcp: Vec<Value> = mcp_payload["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s["state"] == "connected")
        .collect();
    let (_, mcp_tools_payload) = state.mesh.mcp_tools();
    let available_mcp_tools = mcp_tools_payload["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    if let Some(map) = body.as_object_mut() {
        map.insert(
            "permission_policy".to_string(),
            state.permission_snapshot().await,
        );

        // Inject persisted model selection as defaults when the client doesn't specify.
        if !has_provider_id {
            if let Some(ref pid) = selected.provider_id {
                map.insert("provider_id".to_string(), json!(pid));
            }
        }
        if !has_model {
            if let Some(ref mid) = selected.model_id {
                map.insert("model".to_string(), json!(mid));
            }
        }
        if !has_reasoning_effort {
            map.insert(
                "reasoning_effort".to_string(),
                json!(selected.reasoning_effort),
            );
        }

        // Inject session-bound skills when client doesn't provide them.
        if !map.contains_key("session_skills") && !bound_skills.is_empty() {
            map.insert("session_skills".to_string(), json!(bound_skills));
        }
        // Inject enabled plugins context.
        if !map.contains_key("enabled_plugins") && !enabled_plugins.is_empty() {
            map.insert("enabled_plugins".to_string(), json!(enabled_plugins));
        }
        // Inject connected MCP servers and available tools.
        if !map.contains_key("mcp_servers") && !connected_mcp.is_empty() {
            map.insert("mcp_servers".to_string(), json!(connected_mcp));
        }
        if !map.contains_key("mcp_tools") && !available_mcp_tools.is_empty() {
            map.insert("mcp_tools".to_string(), json!(available_mcp_tools));
        }
    }
    let (status, body) = state.mesh.chat(&state.brain, body).await;
    (status, Json(body)).into_response()
}

pub async fn chat_sessions(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("ProviderRuntime", method, uri)
}

pub async fn chat_session_detail(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("ProviderRuntime", method, uri)
}

pub async fn chat_session_delete(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("ProviderRuntime", method, uri)
}

pub async fn recall_detail(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state
        .brain
        .call("memory.open", json!({"id": id}), "memory:open")
        .await
    {
        Ok(result) => {
            let adapted = adapter::adapt_response("memory.open", &result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn compaction_snapshot(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    match state
        .brain
        .call("session.compaction_snapshot", body, "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn session_compact(
    State(state): State<Arc<AppState>>,
    Json(mut body): Json<Value>,
) -> Response {
    let bound_skills = state.session_skills.lock().await.clone();
    let skill_continuity = json!({
        "artifact_kind": "session_bound_skills_continuity_v1",
        "source": "skills-runtime/capability-mesh",
        "persistence": crate::http::session_skills_store_read_metadata(&state.session_skills_store_path),
        "bound_skills": bound_skills,
        "bound_skill_count": bound_skills.len()
    });
    let mcp_continuity = state.mesh.mcp_runtime_continuity_snapshot();
    let tool_continuity = state.mesh.tool_runtime_continuity_snapshot();
    let plugin_continuity = state.mesh.plugin_runtime_continuity_snapshot();
    if let Some(object) = body.as_object_mut() {
        object.insert("session_bound_skills".to_string(), skill_continuity);
        object.insert("mcp_runtime".to_string(), mcp_continuity);
        object.insert("tool_runtime".to_string(), tool_continuity);
        object.insert("plugin_runtime".to_string(), plugin_continuity);
    } else {
        body = json!({
            "input": body,
            "session_bound_skills": skill_continuity,
            "mcp_runtime": mcp_continuity,
            "tool_runtime": tool_continuity,
            "plugin_runtime": plugin_continuity
        });
    }

    match state.brain.call("session.compact", body, "system").await {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn memory_detail(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state
        .brain
        .call("memory.open", json!({"id": id}), "memory:open")
        .await
    {
        Ok(result) => {
            let adapted = adapter::adapt_response("memory.open", &result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn memory_save(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    match state.brain.call("memory.save", body, "memory:save").await {
        Ok(result) => {
            let adapted = adapter::adapt_response("memory.save", &result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn benchmarks_list(State(state): State<Arc<AppState>>) -> Response {
    match state
        .brain
        .call("benchmarks.list", json!({}), "system")
        .await
    {
        Ok(result) => {
            let mut adapted = adapter::adapt_response("benchmarks.list", &result);
            adapted["scenario_templates"] = json!(state.mesh.benchmark_scenarios());
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn benchmark_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state
        .brain
        .call("benchmarks.detail", json!({"benchmark_id": id}), "system")
        .await
    {
        Ok(result) => {
            let adapted = adapter::adapt_response("benchmarks.detail", &result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn events(State(state): State<Arc<AppState>>) -> Response {
    match state
        .brain
        .call("events.list", json!({"limit": 50}), "events:read")
        .await
    {
        Ok(result) => {
            let adapted = adapter::adapt_response("events.list", &result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn projects_list(State(state): State<Arc<AppState>>) -> Response {
    match state.brain.call("projects.list", json!({}), "system").await {
        Ok(result) => {
            let projects = result
                .get("projects")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(adapter::project_summary_payload)
                .collect::<Vec<_>>();
            (axum::http::StatusCode::OK, Json(json!(projects))).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn projects_create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    match state.brain.call("projects.create", body, "system").await {
        Ok(result) => {
            let project = result
                .get("project")
                .cloned()
                .unwrap_or_else(|| result.clone());
            let adapted = adapter::project_summary_payload(project);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn projects_hierarchy(State(state): State<Arc<AppState>>) -> Response {
    match state
        .brain
        .call("sessions.hierarchy", json!({}), "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn sessions_list(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SessionsQuery>,
) -> Response {
    let params = match query.project_id {
        Some(project_id) => json!({"project_id": project_id}),
        None => json!({}),
    };
    match state.brain.call("sessions.list", params, "system").await {
        Ok(result) => {
            let sessions = result
                .get("sessions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(adapter::session_summary_payload)
                .collect::<Vec<_>>();
            (axum::http::StatusCode::OK, Json(json!(sessions))).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn session_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state
        .brain
        .call("sessions.detail", json!({"id": id}), "system")
        .await
    {
        Ok(result) => {
            let adapted = adapter::session_detail_payload(&result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn session_rename(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let mut params = body;
    params["id"] = json!(id);
    match state.brain.call("sessions.rename", params, "system").await {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn session_set_active(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let mut params = body;
    params["id"] = json!(id);
    match state
        .brain
        .call("sessions.set_active", params, "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn nightly_tree(State(state): State<Arc<AppState>>) -> Response {
    match state.brain.call("nightly.tree", json!({}), "system").await {
        Ok(result) => {
            let days = result
                .get("days")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(adapter::nightly_day_payload)
                .collect::<Vec<_>>();
            (axum::http::StatusCode::OK, Json(days)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn nightly_run(State(state): State<Arc<AppState>>) -> Response {
    match state.brain.call("nightly.run", json!({}), "system").await {
        Ok(result) => {
            let adapted = adapter::nightly_result_payload(&result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn nightly_dry_run(State(state): State<Arc<AppState>>) -> Response {
    match state
        .brain
        .call("nightly.dry_run", json!({}), "system")
        .await
    {
        Ok(result) => {
            let adapted = adapter::nightly_result_payload(&result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn reasoning_list(State(state): State<Arc<AppState>>) -> Response {
    match state
        .brain
        .call(
            "reasoning.artifacts",
            json!({"limit": 50}),
            "diagnostics:read",
        )
        .await
    {
        Ok(result) => {
            let summaries = result
                .get("artifacts")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            (axum::http::StatusCode::OK, Json(json!(summaries))).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn reasoning_bridge_integrity(State(state): State<Arc<AppState>>) -> Response {
    match state
        .brain
        .call(
            "reasoning.bridge.integrity",
            json!({"limit": 100}),
            "diagnostics:read",
        )
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn reasoning(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    match state.brain.call("reasoning.run", body, "memory:save").await {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn settings_get(State(state): State<Arc<AppState>>) -> Response {
    let settings = state.brain.call("settings.get", json!({}), "system").await;
    let session = state
        .brain
        .call("session.get", json!({}), "system")
        .await
        .ok();
    let session_configured = session
        .as_ref()
        .and_then(|s| s.get("configured").and_then(Value::as_bool))
        .unwrap_or(false);

    match settings {
        Ok(settings_result) => {
            // Debug: return raw brain response for inspection
            // Remove after debugging
            let adapted =
                adapter::settings_payload(&settings_result, None, None, session_configured);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn settings_set(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let params = if body.get("settings").is_some() {
        body
    } else if let (Some(key), Some(value)) =
        (body.get("key").and_then(Value::as_str), body.get("value"))
    {
        let mut settings = serde_json::Map::new();
        settings.insert(key.to_string(), value.clone());
        json!({"settings": settings})
    } else {
        body
    };
    match state.brain.call("settings.set", params, "system").await {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn model_select(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let provider_id = body
        .get("provider_id")
        .or_else(|| body.get("providerId"))
        .and_then(Value::as_str);
    let model_id = body
        .get("model_id")
        .or_else(|| body.get("modelId"))
        .or_else(|| body.get("model"))
        .and_then(Value::as_str);
    let reasoning_effort = body
        .get("reasoning_effort")
        .or_else(|| body.get("reasoningEffort"))
        .and_then(Value::as_str);

    let mut settings = serde_json::Map::new();
    if let Some(pid) = provider_id {
        settings.insert("provider.selectedProviderId".to_string(), json!(pid));
    }
    if let Some(mid) = model_id {
        settings.insert("provider.selectedModelId".to_string(), json!(mid));
    }
    if let Some(re) = reasoning_effort {
        settings.insert("provider.reasoningEffort".to_string(), json!(re));
    }

    if settings.is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "error": "at least one of provider_id, model_id, or reasoning_effort is required"
            })),
        )
            .into_response();
    }

    match state
        .brain
        .call("settings.set", json!({"settings": settings}), "system")
        .await
    {
        Ok(result) => {
            // Update in-memory cache so chat handler can use the new selection immediately.
            let mut update_body = json!({});
            if let Some(map) = update_body.as_object_mut() {
                if let Some(pid) = provider_id {
                    map.insert("provider_id".to_string(), json!(pid));
                }
                if let Some(mid) = model_id {
                    map.insert("model_id".to_string(), json!(mid));
                }
                if let Some(re) = reasoning_effort {
                    map.insert("reasoning_effort".to_string(), json!(re));
                }
            }
            state.update_selected_model(&update_body).await;

            let confirmed_provider_id = provider_id.map(|v| json!(v)).unwrap_or_else(|| {
                result
                    .get("settings")
                    .and_then(|s| s.get("provider.selectedProviderId"))
                    .cloned()
                    .unwrap_or(Value::Null)
            });
            let confirmed_model_id = model_id.map(|v| json!(v)).unwrap_or_else(|| {
                result
                    .get("settings")
                    .and_then(|s| s.get("provider.selectedModelId"))
                    .cloned()
                    .unwrap_or(Value::Null)
            });
            let default_reasoning = json!("medium");
            let confirmed_reasoning = reasoning_effort.map(|v| json!(v)).unwrap_or_else(|| {
                result
                    .get("settings")
                    .and_then(|s| s.get("provider.reasoningEffort"))
                    .cloned()
                    .unwrap_or(default_reasoning)
            });

            (
                axum::http::StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "artifact_kind": "model_selection_v1",
                    "provider_id": confirmed_provider_id,
                    "model_id": confirmed_model_id,
                    "reasoning_effort": confirmed_reasoning,
                    "routeable": provider_id.is_some() && model_id.is_some(),
                    "updated_at": result.get("updatedAt").cloned().unwrap_or(Value::Null)
                })),
            )
                .into_response()
        }
        Err(error) => {
            let fallback = state.update_selected_model(&body).await;
            (
                axum::http::StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "artifact_kind": "model_selection_v1",
                    "provider_id": fallback.provider_id,
                    "model_id": fallback.model_id,
                    "reasoning_effort": fallback.reasoning_effort,
                    "routeable": fallback.provider_id.is_some() && fallback.model_id.is_some(),
                    "local_only": true,
                    "warning": "brain_settings_unavailable_using_ingress_cache",
                    "error": error.error_payload()
                })),
            )
                .into_response()
        }
    }
}

pub async fn model_selected(State(state): State<Arc<AppState>>) -> Response {
    match state.brain.call("settings.get", json!({}), "system").await {
        Ok(result) => {
            let provider_section = result.get("provider").cloned().unwrap_or_else(|| {
                json!({
                    "selectedProviderId": Value::Null,
                    "selectedModelId": Value::Null,
                    "reasoningEffort": "medium"
                })
            });
            let from_brain_provider = provider_section
                .get("selectedProviderId")
                .and_then(Value::as_str);
            let from_brain_model = provider_section
                .get("selectedModelId")
                .and_then(Value::as_str);
            let from_brain_reasoning = provider_section
                .get("reasoningEffort")
                .and_then(Value::as_str);
            if from_brain_provider.is_none() || from_brain_model.is_none() {
                let selected = state.selected_model_snapshot().await;
                return (axum::http::StatusCode::OK, Json(json!({
                    "ok": true,
                    "artifact_kind": "model_selection_v1",
                    "provider_id": selected.provider_id,
                    "model_id": selected.model_id,
                    "reasoning_effort": if selected.reasoning_effort.is_empty() { "medium" } else { selected.reasoning_effort.as_str() },
                    "routeable": selected.provider_id.is_some() && selected.model_id.is_some(),
                    "source": "ingress_cache"
                }))).into_response();
            }
            (axum::http::StatusCode::OK, Json(json!({
                "ok": true,
                "artifact_kind": "model_selection_v1",
                "provider_id": provider_section.get("selectedProviderId").cloned().unwrap_or(Value::Null),
                "model_id": provider_section.get("selectedModelId").cloned().unwrap_or(Value::Null),
                "reasoning_effort": from_brain_reasoning.unwrap_or("medium"),
                "routeable": provider_section.get("selectedProviderId").and_then(Value::as_str).is_some()
                    && provider_section.get("selectedModelId").and_then(Value::as_str).is_some(),
                "source": "brain_settings"
            })))
            .into_response()
        }
        Err(error) => {
            let selected = state.selected_model_snapshot().await;
            (
                axum::http::StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "artifact_kind": "model_selection_v1",
                    "provider_id": selected.provider_id,
                    "model_id": selected.model_id,
                    "reasoning_effort": if selected.reasoning_effort.is_empty() { "medium" } else { selected.reasoning_effort.as_str() },
                    "routeable": selected.provider_id.is_some() && selected.model_id.is_some(),
                    "local_only": true,
                    "source": "ingress_cache",
                    "error": error.error_payload()
                })),
            )
                .into_response()
        }
    }
}

pub async fn reasoning_policy_get(State(state): State<Arc<AppState>>) -> Response {
    match state.brain.call("settings.get", json!({}), "system").await {
        Ok(result) => {
            let provider_section = result.get("provider").cloned().unwrap_or_else(|| {
                json!({
                    "selectedProviderId": Value::Null,
                    "selectedModelId": Value::Null,
                    "reasoningEffort": "medium"
                })
            });
            let effort = provider_section
                .get("reasoningEffort")
                .and_then(Value::as_str)
                .unwrap_or("medium");
            (
                axum::http::StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "artifact_kind": "reasoning_policy_v1",
                    "effort": effort,
                    "allowed_values": ["none", "low", "medium", "high"],
                    "default": "medium"
                })),
            )
                .into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn reasoning_policy_set(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let effort = body
        .get("effort")
        .or_else(|| body.get("reasoning_effort"))
        .or_else(|| body.get("reasoningEffort"))
        .and_then(Value::as_str)
        .unwrap_or("medium");

    let allowed = ["none", "low", "medium", "high"];
    if !allowed.contains(&effort) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "error": format!("invalid reasoning effort: {effort}, allowed: {:?}", allowed)
            })),
        )
            .into_response();
    }

    match state
        .brain
        .call(
            "settings.set",
            json!({"settings": {"provider.reasoningEffort": effort}}),
            "system",
        )
        .await
    {
        Ok(result) => {
            // Update in-memory cache so chat handler uses the new effort immediately.
            state
                .update_selected_model(&json!({"reasoning_effort": effort}))
                .await;

            (
                axum::http::StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "artifact_kind": "reasoning_policy_v1",
                    "effort": effort,
                    "allowed_values": ["none", "low", "medium", "high"],
                    "default": "medium",
                    "updated_at": result.get("updatedAt").cloned().unwrap_or(Value::Null)
                })),
            )
                .into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn permissions_get(State(state): State<Arc<AppState>>) -> Response {
    (
        axum::http::StatusCode::OK,
        Json(state.permission_snapshot().await),
    )
        .into_response()
}

pub async fn permissions_set_profile(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let profile = body
        .get("profile")
        .or_else(|| body.get("mode"))
        .and_then(Value::as_str)
        .unwrap_or("sandbox");
    (
        axum::http::StatusCode::OK,
        Json(state.set_permission_profile(profile).await),
    )
        .into_response()
}

pub async fn permissions_grant_root(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    match state
        .brain
        .call("permissions.grant_root", body, "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn permissions_revoke_root(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    match state
        .brain
        .call("permissions.revoke_root", body, "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn tool_check_gate(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let domain = body
        .get("domain")
        .or_else(|| body.get("permission_domain"))
        .or_else(|| body.get("permissionDomain"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let context = body.get("context").cloned().unwrap_or_else(|| json!({}));
    let (status, payload) = state.check_permission_domain(&domain, context).await;
    (status, Json(payload)).into_response()
}

pub async fn permissions_grants_create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    match state
        .brain
        .call("permissions.grants.create", body, "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn permissions_grants_revoke(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    match state
        .brain
        .call("permissions.grants.revoke", body, "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn skills_list(State(state): State<Arc<AppState>>) -> Response {
    let (status, body) = state.mesh.skills();
    (status, Json(body)).into_response()
}

pub async fn skills_reload(State(state): State<Arc<AppState>>) -> Response {
    let (status, body) = state.mesh.skills();
    (status, Json(body)).into_response()
}

pub async fn session_skills_list(State(state): State<Arc<AppState>>) -> Response {
    let (_, skills) = state.mesh.skills();
    let total = skills["total"].as_i64().unwrap_or(0);
    let bound_skills = state.session_skills.lock().await.clone();
    let bound_skill_count = bound_skills.len();
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "artifact_kind": "session_skills_state_v1",
            "service_owner": "skills-runtime/capability-mesh",
            "source": "capability_mesh",
            "brain_forwarded": false,
            "import_state": "active",
            "state": "active",
            "skill_registry_state": if total > 0 { "live" } else { "empty" },
            "available_skills": skills["items"].clone(),
            "available_skill_count": total,
            "bound_skills": bound_skills,
            "bound_skill_count": bound_skill_count,
            "persistence": crate::http::session_skills_store_read_metadata(&state.session_skills_store_path),
            "missing_substrate": {},
            "reason": "Session skill binding is backed by the ingress skills-runtime substrate for this backend slice; imported skills are explicit session context attachments and are traced to HOM brain."
        })),
    )
        .into_response()
}

pub async fn session_skills_import_preview(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Response {
    let body = parse_optional_json_body(&body);
    let requested = requested_skill_ids(&body);
    let (_, skills) = state.mesh.skills();
    let rows = skills["items"].as_array().cloned().unwrap_or_default();
    let mut total_estimated_tokens: u64 = 0;
    let mut previews = Vec::new();

    for requested_id in requested.iter() {
        let row = rows.iter().find(|skill| {
            skill["id"].as_str() == Some(requested_id.as_str())
                || skill["name"].as_str() == Some(requested_id.as_str())
                || skill["label"].as_str() == Some(requested_id.as_str())
        });
        let Some(skill) = row else {
            previews.push(json!({
                "skill_id": requested_id,
                "state": "missing",
                "estimated_tokens": null,
                "context_bytes": null,
                "source_path": null,
                "warning": "Skill descriptor was not found; no synthetic context cost was invented."
            }));
            continue;
        };
        let source_path = skill["source_path"].as_str().unwrap_or_default();
        let descriptor = std::fs::read_to_string(source_path).unwrap_or_default();
        let context_bytes = descriptor.len() as u64;
        let estimated_tokens = estimate_tokens_from_descriptor(&descriptor);
        total_estimated_tokens += estimated_tokens;
        previews.push(json!({
            "skill_id": requested_id,
            "matched_skill_id": skill["id"],
            "name": skill["name"],
            "state": "available",
            "estimated_tokens": estimated_tokens,
            "context_bytes": context_bytes,
            "source_path": source_path,
            "warning": if estimated_tokens > 4_000 { "This skill consumes significant context; avoid always-on import unless needed." } else { "" }
        }));
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "artifact_kind": "session_skill_import_preview_v1",
            "service_owner": "skills-runtime/capability-mesh",
            "source": "capability_mesh",
            "brain_forwarded": false,
            "estimate_source": "skill_descriptor_content",
            "estimate_formula": "ceil(utf8_descriptor_bytes / 4)",
            "total_estimated_tokens": total_estimated_tokens,
            "skill_previews": previews,
            "warning": if total_estimated_tokens > 12_000 { "Importing these skills may consume excessive context window." } else { "" },
            "apply_state": "active",
            "missing_substrate": {}
        })),
    )
        .into_response()
}

pub async fn session_skills_import(
    OriginalUri(uri): OriginalUri,
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Response {
    let body = parse_optional_json_body(&body);
    apply_session_skill_import(state, uri.path(), requested_skill_ids(&body)).await
}

pub async fn session_skills_remove(
    OriginalUri(uri): OriginalUri,
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Response {
    let body = parse_optional_json_body(&body);
    apply_session_skill_remove(state, uri.path(), requested_skill_ids(&body)).await
}

fn parse_optional_json_body(body: &Bytes) -> Value {
    if body.is_empty() {
        return json!({});
    }
    serde_json::from_slice::<Value>(body).unwrap_or_else(|_| json!({}))
}

fn requested_skill_ids(body: &Value) -> Vec<String> {
    body.get("skill_ids")
        .or_else(|| body.get("skillIds"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn estimate_tokens_from_descriptor(descriptor: &str) -> u64 {
    ((descriptor.len() as u64) + 3) / 4
}

async fn apply_session_skill_import(
    state: Arc<AppState>,
    route: &str,
    requested_skills: Vec<String>,
) -> Response {
    let (_, skills) = state.mesh.skills();
    let rows = skills["items"].as_array().cloned().unwrap_or_default();
    let mut matched = Vec::new();
    let mut missing = Vec::new();

    for requested in requested_skills.iter() {
        if let Some(skill) = find_requested_skill(&rows, requested) {
            matched.push(skill);
        } else {
            missing.push(requested.clone());
        }
    }

    if matched.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "artifact_kind": "session_skill_mutation_result_v1",
                "service_owner": "skills-runtime/capability-mesh",
                "source": "capability_mesh",
                "brain_forwarded": false,
                "state": "failed",
                "action": "import",
                "applied": false,
                "requested_skills": requested_skills,
                "missing_skills": missing,
                "error": {"code": "skill_not_found", "message": "No requested skill descriptor was found", "data": {"route": route}}
            })),
        )
            .into_response();
    }

    let persisted_bound_skills = {
        let mut bound = state.session_skills.lock().await;
        for skill in matched.iter() {
            let id = skill["id"].as_str().unwrap_or_default();
            if !bound
                .iter()
                .any(|existing| existing["id"].as_str() == Some(id))
            {
                let mut bound_skill = skill.clone();
                bound_skill["bound_at"] = json!(now_millis_string());
                bound_skill["binding_state"] = json!("bound");
                bound.push(bound_skill);
            }
        }
        bound.clone()
    };
    let persistence = match crate::http::persist_session_skills_to_store(
        &state.session_skills_store_path,
        &persisted_bound_skills,
    ) {
        Ok(metadata) => metadata,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "ok": false,
                    "artifact_kind": "session_skill_mutation_result_v1",
                    "service_owner": "skills-runtime/capability-mesh",
                    "source": "capability_mesh",
                    "brain_forwarded": false,
                    "state": "failed",
                    "action": "import",
                    "applied": false,
                    "requested_skills": requested_skills,
                    "bound_skills": matched,
                    "error": {"code": "session_skill_store_write_failed", "message": error.to_string()},
                    "persistence": crate::http::session_skills_persistence_metadata(&state.session_skills_store_path, "failed")
                })),
            )
                .into_response();
        }
    };

    let primary = matched.first().cloned().unwrap_or_else(|| json!({}));
    let event_payload = json!({
        "kind": "skill_import",
        "skill_name": primary["name"],
        "skill_id": primary["id"],
        "skill_count": matched.len(),
        "dependencies_resolved": true,
        "status": "executed",
        "owner_runtime": "skills-runtime/capability-mesh",
        "route": route,
        "source_paths": matched.iter().map(|skill| skill["source_path"].clone()).collect::<Vec<_>>()
    });
    let brain_result = state
        .brain
        .call(
            "reasoning.tool_event.record",
            event_payload,
            "skills:import",
        )
        .await;
    let brain_event = match brain_result {
        Ok(result) => {
            json!({"method": "reasoning.tool_event.record", "recorded": result["ok"].as_bool().unwrap_or(true), "result": result})
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            return (
                status,
                Json(json!({
                    "ok": false,
                    "artifact_kind": "session_skill_mutation_result_v1",
                    "service_owner": "skills-runtime/capability-mesh",
                    "source": "capability_mesh",
                    "brain_forwarded": true,
                    "state": "failed",
                    "action": "import",
                    "applied": false,
                    "requested_skills": requested_skills,
                    "bound_skills": matched,
                    "error": error.error_payload()
                })),
            )
                .into_response();
        }
    };

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "artifact_kind": "session_skill_mutation_result_v1",
            "service_owner": "skills-runtime/capability-mesh",
            "source": "capability_mesh",
            "brain_forwarded": true,
            "state": "applied",
            "action": "import",
            "applied": true,
            "requested_skills": requested_skills,
            "bound_skills": matched,
            "bound_skill_count": matched.len(),
            "missing_skills": missing,
            "dependencies_resolved": true,
            "persistence": persistence,
            "brain_event": brain_event
        })),
    )
        .into_response()
}

async fn apply_session_skill_remove(
    state: Arc<AppState>,
    route: &str,
    requested_skills: Vec<String>,
) -> Response {
    let mut removed = Vec::new();
    let kept_bound_skills = {
        let mut bound = state.session_skills.lock().await;
        let mut kept = Vec::new();
        for skill in bound.drain(..) {
            let matches = requested_skills
                .iter()
                .any(|requested| skill_matches_request(&skill, requested));
            if matches {
                removed.push(skill);
            } else {
                kept.push(skill);
            }
        }
        *bound = kept;
        bound.clone()
    };
    let persistence = match crate::http::persist_session_skills_to_store(
        &state.session_skills_store_path,
        &kept_bound_skills,
    ) {
        Ok(metadata) => metadata,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "ok": false,
                    "artifact_kind": "session_skill_mutation_result_v1",
                    "service_owner": "skills-runtime/capability-mesh",
                    "source": "capability_mesh",
                    "brain_forwarded": false,
                    "state": "failed",
                    "action": "remove",
                    "applied": false,
                    "requested_skills": requested_skills,
                    "removed_skills": removed,
                    "error": {"code": "session_skill_store_write_failed", "message": error.to_string()},
                    "persistence": crate::http::session_skills_persistence_metadata(&state.session_skills_store_path, "failed")
                })),
            )
                .into_response();
        }
    };

    let primary = removed.first().cloned().unwrap_or_else(|| json!({}));
    let event_payload = json!({
        "kind": "skill_remove",
        "skill_name": primary["name"],
        "skill_id": primary["id"],
        "skill_count": removed.len(),
        "status": "executed",
        "owner_runtime": "skills-runtime/capability-mesh",
        "route": route
    });
    let brain_result = state
        .brain
        .call(
            "reasoning.tool_event.record",
            event_payload,
            "skills:remove",
        )
        .await;
    let brain_event = match brain_result {
        Ok(result) => {
            json!({"method": "reasoning.tool_event.record", "recorded": result["ok"].as_bool().unwrap_or(true), "result": result})
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            return (
                status,
                Json(json!({
                    "ok": false,
                    "artifact_kind": "session_skill_mutation_result_v1",
                    "service_owner": "skills-runtime/capability-mesh",
                    "source": "capability_mesh",
                    "brain_forwarded": true,
                    "state": "failed",
                    "action": "remove",
                    "applied": false,
                    "requested_skills": requested_skills,
                    "removed_skills": removed,
                    "error": error.error_payload()
                })),
            )
                .into_response();
        }
    };

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "artifact_kind": "session_skill_mutation_result_v1",
            "service_owner": "skills-runtime/capability-mesh",
            "source": "capability_mesh",
            "brain_forwarded": true,
            "state": "applied",
            "action": "remove",
            "applied": true,
            "requested_skills": requested_skills,
            "removed_skills": removed,
            "removed_skill_count": removed.len(),
            "persistence": persistence,
            "brain_event": brain_event
        })),
    )
        .into_response()
}

fn find_requested_skill(rows: &[Value], requested: &str) -> Option<Value> {
    rows.iter()
        .find(|skill| skill_matches_request(skill, requested))
        .cloned()
}

fn skill_matches_request(skill: &Value, requested: &str) -> bool {
    skill["id"].as_str() == Some(requested)
        || skill["name"].as_str() == Some(requested)
        || skill["label"].as_str() == Some(requested)
}

fn now_millis_string() -> String {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

pub async fn tools_list(State(state): State<Arc<AppState>>) -> Response {
    let (status, body) = state.mesh.tools();
    (status, Json(body)).into_response()
}

pub async fn tools_detail(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let (status, body) = state.mesh.tool_detail(&id);
    (status, Json(body)).into_response()
}

pub async fn tools_call(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    let (status, body) = state.mesh.call_tool(&state.brain, body).await;
    (status, Json(body)).into_response()
}

pub async fn gate_grants_list(State(state): State<Arc<AppState>>) -> Response {
    let (status, body) = state.mesh.gate_grants(&state.brain).await;
    (status, Json(body)).into_response()
}

pub async fn gate_grants_create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let (status, body) = state.mesh.create_gate_grant(&state.brain, body).await;
    (status, Json(body)).into_response()
}

pub async fn plugins_list(State(state): State<Arc<AppState>>) -> Response {
    let (status, body) = state.mesh.plugins();
    (status, Json(body)).into_response()
}

pub async fn plugins_marketplace(State(state): State<Arc<AppState>>) -> Response {
    let (status, body) = state.mesh.plugins_marketplace();
    (status, Json(body)).into_response()
}

pub async fn plugins_install(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let (status, body) = state.mesh.plugin_install(body);
    (status, Json(body)).into_response()
}

pub async fn plugins_enable(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let (status, body) = state.mesh.plugin_enable(body);
    (status, Json(body)).into_response()
}

pub async fn plugins_disable(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let (status, body) = state.mesh.plugin_disable(body);
    (status, Json(body)).into_response()
}

pub async fn plugins_uninstall(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let (status, body) = state.mesh.plugin_uninstall(body);
    (status, Json(body)).into_response()
}

pub async fn apps_skills(State(state): State<Arc<AppState>>) -> Response {
    let (status, body) = state.mesh.apps_skills();
    (status, Json(body)).into_response()
}

pub async fn mcp_servers(State(state): State<Arc<AppState>>) -> Response {
    let (status, body) = state.mesh.mcp_servers();
    (status, Json(body)).into_response()
}

pub async fn inspector_target(
    State(state): State<Arc<AppState>>,
    Path((kind, id)): Path<(String, String)>,
) -> Response {
    match state
        .brain
        .call(
            "ui.inspector.target",
            json!({"kind": kind, "id": id}),
            "system",
        )
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn apps_list(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("PluginsRuntime", method, uri)
}

pub async fn apps_registry(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("PluginsRuntime", method, uri)
}

pub async fn apps_scan_machine(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("PluginsRuntime", method, uri)
}

pub async fn apps_grant_request(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("PluginsRuntime", method, uri)
}

pub async fn apps_connect_kit(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("PluginsRuntime", method, uri)
}

pub async fn apps_approve(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("PluginsRuntime", method, uri)
}

pub async fn apps_deny(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("PluginsRuntime", method, uri)
}

pub async fn apps_revoke(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("PluginsRuntime", method, uri)
}

pub async fn providers_register(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("ProviderRuntime", method, uri)
}

pub async fn legacy_provider_singleton_disabled(
    method: Method,
    OriginalUri(uri): OriginalUri,
) -> Response {
    runtime_bubble_offline("ProviderRuntime", method, uri)
}

pub async fn provider_credentials_save(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let (status, body) = state.mesh.save_provider_credential(body);
    (status, Json(body)).into_response()
}

pub async fn providers_connect(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let (status, body) = state.mesh.provider_connect(&id, body).await;
    (status, Json(body)).into_response()
}

pub async fn providers_discover(
    State(state): State<Arc<AppState>>,
    OriginalUri(_uri): OriginalUri,
    body: Option<Json<Value>>,
) -> Response {
    let body = body.map(|Json(value)| value).unwrap_or_else(|| json!({}));
    if body
        .get("provider_id")
        .or_else(|| body.get("providerId"))
        .is_none()
    {
        let (status, body) = state.mesh.providers().await;
        return (status, Json(body)).into_response();
    }
    let (status, body) = state.mesh.provider_discover(body).await;
    (status, Json(body)).into_response()
}

pub async fn providers_list(State(state): State<Arc<AppState>>) -> Response {
    let (status, body) = state.mesh.providers().await;
    (status, Json(body)).into_response()
}

pub async fn providers_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let (status, body) = state.mesh.provider_detail(&id).await;
    (status, Json(body)).into_response()
}

pub async fn providers_models(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let (status, body) = state.mesh.provider_models(&id).await;
    (status, Json(body)).into_response()
}

pub async fn providers_preflight(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let (status, body) = state.mesh.provider_preflight(body).await;
    (status, Json(body)).into_response()
}

pub async fn providers_auth_start(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    OriginalUri(uri): OriginalUri,
    Json(body): Json<Value>,
) -> Response {
    if id == "codex-oauth" {
        return state
            .bubbles
            .provider
            .post("/auth/start", body)
            .await
            .into_response();
    }
    runtime_bubble_response("ProviderAuthAdapter", "POST", uri.path())
}

pub async fn providers_auth_complete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    OriginalUri(uri): OriginalUri,
    Json(body): Json<Value>,
) -> Response {
    if id == "codex-oauth" {
        return state
            .bubbles
            .provider
            .post("/auth/complete", body)
            .await
            .into_response();
    }
    runtime_bubble_response("ProviderAuthAdapter", "POST", uri.path())
}

pub async fn providers_auth_refresh(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    OriginalUri(uri): OriginalUri,
    Json(body): Json<Value>,
) -> Response {
    if id == "codex-oauth" {
        return state
            .bubbles
            .provider
            .post("/auth/refresh", body)
            .await
            .into_response();
    }
    runtime_bubble_response("ProviderAuthAdapter", "POST", uri.path())
}

pub async fn providers_entitlement(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    OriginalUri(uri): OriginalUri,
) -> Response {
    if id == "codex-oauth" {
        return state
            .bubbles
            .provider
            .get("/entitlement")
            .await
            .into_response();
    }
    runtime_bubble_response("ProviderAuthAdapter", "GET", uri.path())
}

pub async fn providers_state_reset_dry_run(
    method: Method,
    OriginalUri(uri): OriginalUri,
) -> Response {
    runtime_bubble_offline("ProviderRuntime", method, uri)
}

pub async fn providers_state_reset(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("ProviderRuntime", method, uri)
}

pub async fn providers_probe(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("ProviderRuntime", method, uri)
}

pub async fn providers_enable(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("ProviderRuntime", method, uri)
}

pub async fn providers_disable(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("ProviderRuntime", method, uri)
}

pub async fn providers_set_default(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("ProviderRuntime", method, uri)
}

pub async fn providers_test_chat(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("ProviderRuntime", method, uri)
}

pub async fn providers_probe_all(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    runtime_bubble_offline("ProviderRuntime", method, uri)
}

pub async fn mcp_config(State(state): State<Arc<AppState>>) -> Response {
    let (status, body) = state.mesh.mcp_config();
    (status, Json(body)).into_response()
}

pub async fn imports_list(State(state): State<Arc<AppState>>) -> Response {
    match state.brain.call("import.list", json!({}), "system").await {
        Ok(result) => {
            let settings = state
                .brain
                .call("settings.get", json!({}), "system")
                .await
                .ok();
            let adapted = adapter::imports_payload(&result, settings.as_ref());
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn import_pack(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    match state.brain.call("import.pack", body, "system").await {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn export_brain(State(state): State<Arc<AppState>>) -> Response {
    match state.brain.call("export.brain", json!({}), "system").await {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

// ── Memory Import (feature-flagged) ───────────────────────────────────

fn memory_import_enabled() -> bool {
    std::env::var("HOM_FEATURE_MEMORY_IMPORT").unwrap_or_default() == "1"
}

fn feature_disabled_response() -> Response {
    (
        axum::http::StatusCode::NOT_FOUND,
        Json(json!({"ok": false, "error": "feature_disabled"})),
    )
        .into_response()
}

pub async fn memory_import_sources(State(state): State<Arc<AppState>>) -> Response {
    if !memory_import_enabled() {
        return feature_disabled_response();
    }
    match state
        .brain
        .call("import.memory.sources", json!({}), "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn memory_import_batch_create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    if !memory_import_enabled() {
        return feature_disabled_response();
    }
    match state
        .brain
        .call("import.memory.batch.create", body, "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn memory_import_batch_get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if !memory_import_enabled() {
        return feature_disabled_response();
    }
    match state
        .brain
        .call("import.memory.batch.get", json!({"batch_id": id}), "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn memory_import_batch_parse(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if !memory_import_enabled() {
        return feature_disabled_response();
    }
    let mut params = body;
    params["batch_id"] = json!(id);
    match state
        .brain
        .call("import.memory.batch.parse", params, "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn memory_import_batch_preview(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if !memory_import_enabled() {
        return feature_disabled_response();
    }
    match state
        .brain
        .call(
            "import.memory.batch.preview",
            json!({"batch_id": id}),
            "system",
        )
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn memory_import_batch_commit(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if !memory_import_enabled() {
        return feature_disabled_response();
    }
    let mut params = body;
    params["batch_id"] = json!(id);
    match state
        .brain
        .call("import.memory.batch.commit", params, "system")
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn memory_import_batch_cancel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if !memory_import_enabled() {
        return feature_disabled_response();
    }
    match state
        .brain
        .call(
            "import.memory.batch.cancel",
            json!({"batch_id": id}),
            "system",
        )
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn memory_import_batch_report(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if !memory_import_enabled() {
        return feature_disabled_response();
    }
    match state
        .brain
        .call(
            "import.memory.batch.report",
            json!({"batch_id": id}),
            "system",
        )
        .await
    {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn brain_backup(State(state): State<Arc<AppState>>) -> Response {
    match state.brain.call("brain.backup", json!({}), "system").await {
        Ok(result) => {
            let adapted = adapter::backup_payload(&result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn brain_purge(
    State(state): State<Arc<AppState>>,
    body: Option<Json<Value>>,
) -> Response {
    let params = body.map(|Json(value)| value).unwrap_or_else(|| json!({}));
    let confirm_purge = params
        .get("confirm_purge")
        .or_else(|| params.get("confirmPurge"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !confirm_purge {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "error": {
                    "code": -32080,
                    "message": "brain_purge_confirmation_required",
                    "data": {"required": "confirm_purge=true"}
                }
            })),
        )
            .into_response();
    }
    // Escalate to full_access first so purge can proceed
    let escalate_result = state
        .brain
        .call(
            "permissions.set_profile",
            json!({
                "profile": "full_access",
                "confirm_broadening": true,
                "automation_enabled": true,
                "network_enabled": true,
                "provider_key_access": true,
                "shell_access": true
            }),
            "system",
        )
        .await;
    if let Err(error) = escalate_result {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "ok": false,
                "error": {"message": format!("permission_escalation_failed: {error}")}
            })),
        )
            .into_response();
    }
    match state
        .brain
        .call(
            "brain.purge",
            json!({"confirm_purge": true, "confirm_broadening": true}),
            "system",
        )
        .await
    {
        Ok(result) => {
            let adapted = adapter::purge_payload(&result);
            (axum::http::StatusCode::OK, Json(adapted)).into_response()
        }
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

async fn gate_read(state: Arc<AppState>, method: &str, params: Value) -> Response {
    match state.brain.call(method, params, "gates:read").await {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => (
            crate::http::brain_error_status(&error),
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

async fn gate_write(
    state: Arc<AppState>,
    headers: HeaderMap,
    path: &str,
    method: &str,
    body: Value,
) -> Response {
    gate_write_with_brain_body(state, headers, path, method, body.clone(), body).await
}

async fn gate_write_with_brain_body(
    state: Arc<AppState>,
    headers: HeaderMap,
    path: &str,
    method: &str,
    signed_body: Value,
    brain_body: Value,
) -> Response {
    let identity =
        match crate::auth::verify_ui_request(&state, "POST", path, &headers, &signed_body).await {
            Ok(identity) => identity,
            Err((status, error)) => return (status, Json(error)).into_response(),
        };
    if !identity.verified {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(json!({"ok": false, "error": {"code": -32002, "message": "signed_gate_request_required"}})),
        )
            .into_response();
    }
    match state.brain.call(method, brain_body, "gates:write").await {
        Ok(result) => (axum::http::StatusCode::OK, Json(result)).into_response(),
        Err(error) => (
            crate::http::brain_error_status(&error),
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

fn gate_query_params(query: GateListQuery) -> Value {
    let mut params = serde_json::Map::new();
    if let Some(task_id) = query.task_id {
        params.insert("task_id".to_string(), json!(task_id));
    }
    if let Some(plan_version_id) = query.plan_version_id {
        params.insert("plan_version_id".to_string(), json!(plan_version_id));
    }
    if let Some(limit) = query.limit {
        params.insert("limit".to_string(), json!(limit));
    }
    Value::Object(params)
}

/// Debug endpoint: returns raw brain settings.get response
pub async fn settings_raw(State(state): State<Arc<AppState>>) -> Response {
    match state.brain.call("settings.get", json!({}), "system").await {
        Ok(result) => Json(result).into_response(),
        Err(error) => (
            crate::http::brain_error_status(&error),
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub async fn workspace_files_list(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let policy = state.permission_snapshot().await;
    let profile = policy["profile"].as_str().unwrap_or("sandbox");
    if profile == "sandbox" {
        return (StatusCode::FORBIDDEN, Json(json!({
            "ok": false,
            "error": {"code": -32080, "message": "filesystem_access_denied", "data": {"profile": profile, "reason": "sandbox profile denies filesystem listing"}},
            "source": "permission-policy-gate"
        }))).into_response();
    }
    let (status, result) = state.mesh.workspace_list_directory(body);
    (status, Json(result)).into_response()
}

pub async fn workspace_files_read(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let policy = state.permission_snapshot().await;
    let profile = policy["profile"].as_str().unwrap_or("sandbox");
    if profile == "sandbox" {
        return (StatusCode::FORBIDDEN, Json(json!({
            "ok": false,
            "error": {"code": -32080, "message": "filesystem_access_denied", "data": {"profile": profile, "reason": "sandbox profile denies filesystem read"}},
            "source": "permission-policy-gate"
        }))).into_response();
    }
    let (status, result) = state.mesh.workspace_read_file(body);
    (status, Json(result)).into_response()
}

pub async fn workspace_files_search(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let policy = state.permission_snapshot().await;
    let profile = policy["profile"].as_str().unwrap_or("sandbox");
    if profile == "sandbox" {
        return (StatusCode::FORBIDDEN, Json(json!({
            "ok": false,
            "error": {"code": -32080, "message": "filesystem_access_denied", "data": {"profile": profile, "reason": "sandbox profile denies filesystem search"}},
            "source": "permission-policy-gate"
        }))).into_response();
    }
    let (status, result) = state.mesh.workspace_search_files(body);
    (status, Json(result)).into_response()
}

pub async fn workspace_files_write_preview(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let policy = state.permission_snapshot().await;
    let profile = policy["profile"].as_str().unwrap_or("sandbox");
    let write_allowed = profile == "full";
    if !write_allowed {
        return (StatusCode::FORBIDDEN, Json(json!({
            "ok": false,
            "error": {"code": -32080, "message": "filesystem_write_denied", "data": {"profile": profile, "reason": "only full profile allows filesystem write preview"}},
            "source": "permission-policy-gate"
        }))).into_response();
    }
    let (status, result) = state.mesh.workspace_file_write_preview(body);
    (status, Json(result)).into_response()
}

pub async fn workspace_files_write_apply(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let policy = state.permission_snapshot().await;
    let profile = policy["profile"].as_str().unwrap_or("sandbox");
    let write_allowed = profile == "full";
    if !write_allowed {
        return (StatusCode::FORBIDDEN, Json(json!({
            "ok": false,
            "error": {"code": -32080, "message": "filesystem_write_denied", "data": {"profile": profile, "reason": "only full profile allows filesystem write apply"}},
            "source": "permission-policy-gate"
        }))).into_response();
    }
    let (status, result) = state.mesh.workspace_file_write_apply(body);
    (status, Json(result)).into_response()
}

pub async fn attachments_inspect(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let paths = body
        .get("paths")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();
    if paths.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({
            "ok": false,
            "error": {"code": -32602, "message": "missing_paths", "data": {"required": "paths array with at least one string"}}
        }))).into_response();
    }
    let result = state.mesh.inspect_local_paths(paths);
    (StatusCode::OK, Json(result)).into_response()
}

pub async fn code_review_propose(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let policy = state.permission_snapshot().await;
    let profile = policy["profile"].as_str().unwrap_or("sandbox");
    if profile == "sandbox" {
        return (StatusCode::FORBIDDEN, Json(json!({
            "ok": false,
            "error": {"code": -32080, "message": "filesystem_access_denied", "data": {"profile": profile, "reason": "sandbox profile denies code review proposal"}},
            "source": "permission-policy-gate"
        }))).into_response();
    }
    let path = match body.get("path").and_then(Value::as_str) {
        Some(p) => p.to_string(),
        None => return (StatusCode::BAD_REQUEST, Json(json!({
            "ok": false,
            "error": {"code": -32602, "message": "missing_path", "data": {"required": "path string"}}
        }))).into_response(),
    };
    let diff = body
        .get("diff")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let summary = body
        .get("summary")
        .and_then(Value::as_str)
        .map(String::from);
    let (status, result) = state.mesh.code_review_propose(path, diff, summary);
    (status, Json(result)).into_response()
}

pub async fn code_review_accept(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let policy = state.permission_snapshot().await;
    let profile = policy["profile"].as_str().unwrap_or("sandbox");
    if profile != "full" {
        return (StatusCode::FORBIDDEN, Json(json!({
            "ok": false,
            "error": {"code": -32080, "message": "filesystem_write_denied", "data": {"profile": profile, "reason": "only full profile allows code review acceptance"}},
            "source": "permission-policy-gate"
        }))).into_response();
    }
    let review_id = match body.get("review_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return (StatusCode::BAD_REQUEST, Json(json!({
            "ok": false,
            "error": {"code": -32602, "message": "missing_review_id", "data": {"required": "review_id string"}}
        }))).into_response(),
    };
    let (status, result) = state.mesh.code_review_accept(&review_id);
    (status, Json(result)).into_response()
}

pub async fn code_review_reject(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let policy = state.permission_snapshot().await;
    let profile = policy["profile"].as_str().unwrap_or("sandbox");
    if profile != "full" {
        return (StatusCode::FORBIDDEN, Json(json!({
            "ok": false,
            "error": {"code": -32080, "message": "filesystem_write_denied", "data": {"profile": profile, "reason": "only full profile allows code review rejection"}},
            "source": "permission-policy-gate"
        }))).into_response();
    }
    let review_id = match body.get("review_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return (StatusCode::BAD_REQUEST, Json(json!({
            "ok": false,
            "error": {"code": -32602, "message": "missing_review_id", "data": {"required": "review_id string"}}
        }))).into_response(),
    };
    let reason = body.get("reason").and_then(Value::as_str).map(String::from);
    let (status, result) = state.mesh.code_review_reject(&review_id, reason);
    (status, Json(result)).into_response()
}

pub async fn code_review_diff(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let review_id = match body.get("review_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return (StatusCode::BAD_REQUEST, Json(json!({
            "ok": false,
            "error": {"code": -32602, "message": "missing_review_id", "data": {"required": "review_id string"}}
        }))).into_response(),
    };
    let (status, result) = state.mesh.code_review_diff(&review_id);
    (status, Json(result)).into_response()
}

pub async fn code_review_list(State(state): State<Arc<AppState>>) -> Response {
    let (status, result) = state.mesh.code_review_list();
    (status, Json(result)).into_response()
}

pub async fn documents_read(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let policy = state.permission_snapshot().await;
    let profile = policy["profile"].as_str().unwrap_or("sandbox");
    if profile == "sandbox" {
        return (StatusCode::FORBIDDEN, Json(json!({
            "ok": false,
            "error": {"code": -32080, "message": "filesystem_access_denied", "data": {"profile": profile, "reason": "sandbox profile denies document reading"}},
            "source": "permission-policy-gate"
        }))).into_response();
    }
    let path = match body.get("path").and_then(Value::as_str) {
        Some(p) => p.to_string(),
        None => return (StatusCode::BAD_REQUEST, Json(json!({
            "ok": false,
            "error": {"code": -32602, "message": "missing_path", "data": {"required": "path string"}}
        }))).into_response(),
    };
    let offset = body.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let max_bytes = body
        .get("max_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(65536);
    let (status, result) = state.mesh.workspace_read_file(json!({
        "path": path,
        "offset": offset,
        "max_bytes": max_bytes
    }));
    (
        status,
        Json(json!({
            "ok": result["ok"],
            "artifact_kind": "document_read_v1",
            "text": result["text"],
            "path": result["path"],
            "size_bytes": result["size_bytes"],
            "offset": result["offset"],
            "bytes_read": result["bytes_read"],
            "truncated": result["truncated"],
            "next_offset": result["next_offset"],
            "source": "capability_mesh/document_reader"
        })),
    )
        .into_response()
}

pub async fn provider_route_select(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let provider_id = body
        .get("provider_id")
        .or_else(|| body.get("providerId"))
        .and_then(Value::as_str);
    let model_id = body
        .get("model_id")
        .or_else(|| body.get("modelId"))
        .and_then(Value::as_str);
    let reasoning_effort = body
        .get("reasoning_effort")
        .or_else(|| body.get("reasoningEffort"))
        .and_then(Value::as_str);

    if provider_id.is_none() && model_id.is_none() {
        return (StatusCode::BAD_REQUEST, Json(json!({
            "ok": false,
            "error": {"code": -32602, "message": "missing_selection", "data": {"required": "provider_id or model_id"}}
        }))).into_response();
    }

    // Persist to brain settings
    let mut settings_map = serde_json::Map::new();
    if let Some(pid) = provider_id {
        settings_map.insert("provider.selectedProviderId".to_string(), json!(pid));
    }
    if let Some(mid) = model_id {
        settings_map.insert("provider.selectedModelId".to_string(), json!(mid));
    }
    if let Some(re) = reasoning_effort {
        settings_map.insert("provider.reasoningEffort".to_string(), json!(re));
    }

    match state
        .brain
        .call("settings.set", json!({"settings": settings_map}), "system")
        .await
    {
        Ok(result) if result["ok"].as_bool().unwrap_or(false) => {
            // Update in-memory cache
            state
                .update_selected_model(&json!({
                    "provider_id": provider_id,
                    "model_id": model_id,
                    "reasoning_effort": reasoning_effort
                }))
                .await;

            let route_id = format!(
                "route_{}",
                &sha256_hex(&format!(
                    "{}:{}",
                    provider_id.unwrap_or("default"),
                    model_id.unwrap_or("default")
                ))[..16]
            );

            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "artifact_kind": "provider_route_selection_v1",
                    "route_id": route_id,
                    "provider_id": provider_id,
                    "model_id": model_id,
                    "reasoning_effort": reasoning_effort,
                    "route_certificate_id": Value::Null,
                    "state": "active",
                    "source": "ingress/provider-route-bridge"
                })),
            )
                .into_response()
        }
        Ok(result) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "error": result.get("error").cloned().unwrap_or_else(|| json!("unknown")),
                "source": "brain/settings.set"
            })),
        )
            .into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}

pub async fn provider_routes_select(
    State(state): State<Arc<AppState>>,
    Path(route_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    // Extract provider_id and model_id from body, or use the route_id as a lookup
    let provider_id = body
        .get("provider_id")
        .or_else(|| body.get("providerId"))
        .and_then(Value::as_str);
    let model_id = body
        .get("model_id")
        .or_else(|| body.get("modelId"))
        .and_then(Value::as_str);

    if provider_id.is_none() && model_id.is_none() {
        return (StatusCode::BAD_REQUEST, Json(json!({
            "ok": false,
            "error": {"code": -32602, "message": "missing_selection", "data": {"required": "provider_id or model_id in body"}}
        }))).into_response();
    }

    // Persist to brain settings (same as provider_route_select)
    let mut settings_map = serde_json::Map::new();
    if let Some(pid) = provider_id {
        settings_map.insert("provider.selectedProviderId".to_string(), json!(pid));
    }
    if let Some(mid) = model_id {
        settings_map.insert("provider.selectedModelId".to_string(), json!(mid));
    }

    match state
        .brain
        .call("settings.set", json!({"settings": settings_map}), "system")
        .await
    {
        Ok(result) if result["ok"].as_bool().unwrap_or(false) => {
            state
                .update_selected_model(&json!({
                    "provider_id": provider_id,
                    "model_id": model_id
                }))
                .await;

            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "artifact_kind": "provider_route_selection_v1",
                    "route_id": route_id,
                    "provider_id": provider_id,
                    "model_id": model_id,
                    "route_certificate_id": Value::Null,
                    "state": "active",
                    "source": "ingress/provider-route-bridge"
                })),
            )
                .into_response()
        }
        Ok(result) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "error": result.get("error").cloned().unwrap_or_else(|| json!("unknown")),
                "source": "brain/settings.set"
            })),
        )
            .into_response(),
        Err(error) => {
            let status = crate::http::brain_error_status(&error);
            (
                status,
                Json(json!({"ok": false, "error": error.error_payload()})),
            )
                .into_response()
        }
    }
}
