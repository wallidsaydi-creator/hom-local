use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use hom_provider_base::{
    ChatCompleteParams, ChatMessage, HttpProvider, HttpProviderDescriptor, Provider,
    ProviderCapability, ProviderDialect, ProviderError,
};
use hom_shared::{canonical_json, sha256_hex};
use serde_json::{Value, json};

use crate::brain_client::{BrainClient, BrainClientError};
use crate::provider_catalog::{self, ProviderCatalogEntry, ProviderKind, WireMode};

const DEFAULT_READ_BYTES: u64 = 64 * 1024;
const MAX_READ_BYTES: u64 = 256 * 1024;
const DEFAULT_FETCH_BYTES: u64 = 64 * 1024;
const MAX_FETCH_BYTES: u64 = 256 * 1024;
const MAX_TOOL_ITERATIONS: usize = 3;
const SKILL_DISPLAY_LIMIT: usize = 30;
const TOOL_DESCRIPTOR_VERSION: &str = "capability_descriptor_v1";

#[derive(Clone, Debug, Default)]
pub struct ProviderEndpointOverrides {
    pub lm_studio_base_url: Option<String>,
    pub codex_oauth_runtime_url: Option<String>,
}

#[derive(Clone)]
pub struct CapabilityMesh {
    http: reqwest::Client,
    provider_endpoints: ProviderEndpointOverrides,
    mcp_root: Option<PathBuf>,
    skill_roots: Option<Vec<PathBuf>>,
    tool_preview_store_path: Option<PathBuf>,
    code_review_store_path: Option<PathBuf>,
    plugin_store_path: Option<PathBuf>,
    mcp_runtime: Arc<Mutex<BTreeMap<String, McpRuntimeEntry>>>,
}

struct McpRuntimeEntry {
    row: Value,
    session: Option<StdioMcpSession>,
}

struct StdioMcpSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Drop for StdioMcpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Clone, Debug)]
struct ToolDescriptor {
    id: &'static str,
    capability_id: &'static str,
    descriptor_version: &'static str,
    name: &'static str,
    description: &'static str,
    kind: &'static str,
    domain: &'static str,
    permission_domain: &'static str,
    transport: &'static str,
    retrieval_path: &'static str,
    dispatch_target: &'static str,
    state: &'static str,
    input_schema: Value,
    output_schema: Value,
    examples: Value,
}

#[derive(Clone, Debug)]
struct BrokerDispatchV1 {
    trace_id: String,
    capability_id: String,
    tool_id: String,
    descriptor_hash: String,
    transport: String,
    permission_domain: String,
    arguments: Value,
    source: String,
    caller: String,
    dispatch_target: String,
}

#[derive(Clone, Debug)]
struct StrictToolCall {
    tool_id: String,
    descriptor_hash: Option<String>,
    arguments: Value,
}

#[derive(Clone, Debug)]
struct ToolExecution {
    ok: bool,
    tool_id: String,
    status: StatusCode,
    input_summary: Value,
    result: Option<Value>,
    error: Option<Value>,
    gate: Option<Value>,
    trace: Vec<Value>,
    broker: Option<BrokerDispatchV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolOutcome {
    Executed,
    Previewed,
    Blocked,
    Failed,
    Unavailable,
}

impl ToolOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Executed => "executed",
            Self::Previewed => "previewed",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Unavailable => "unavailable",
        }
    }
}

impl CapabilityMesh {
    pub fn new() -> Self {
        Self::with_provider_endpoints(ProviderEndpointOverrides::default())
    }

    pub fn with_provider_endpoints(provider_endpoints: ProviderEndpointOverrides) -> Self {
        let timeout_ms = std::env::var("HOM_MESH_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(2_500);
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_millis(timeout_ms))
                .build()
                .unwrap_or_default(),
            provider_endpoints,
            mcp_root: None,
            skill_roots: None,
            tool_preview_store_path: None,
            code_review_store_path: None,
            plugin_store_path: None,
            mcp_runtime: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn with_mcp_root(root: impl Into<PathBuf>) -> Self {
        let mut mesh = Self::new();
        mesh.mcp_root = Some(root.into());
        mesh
    }

    pub fn with_skill_roots(roots: Vec<PathBuf>) -> Self {
        let mut mesh = Self::new();
        mesh.skill_roots = Some(roots);
        mesh
    }

    pub fn with_tool_preview_store_path(path: impl Into<PathBuf>) -> Self {
        let mut mesh = Self::new();
        mesh.tool_preview_store_path = Some(path.into());
        mesh
    }

    pub fn with_code_review_store_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.code_review_store_path = Some(path.into());
        self
    }

    pub fn with_plugin_store_path(path: impl Into<PathBuf>) -> Self {
        let mut mesh = Self::new();
        mesh.plugin_store_path = Some(path.into());
        mesh
    }

    pub fn with_lm_studio_base_url(base_url: impl Into<String>) -> Self {
        Self::with_provider_endpoints(ProviderEndpointOverrides {
            lm_studio_base_url: Some(base_url.into()),
            codex_oauth_runtime_url: None,
        })
    }

    fn lm_studio_base_url(&self) -> String {
        self.provider_endpoints
            .lm_studio_base_url
            .clone()
            .or_else(|| std::env::var("LMSTUDIO_BASE_URL").ok())
            .unwrap_or_else(|| "http://127.0.0.1:1234/v1".to_string())
    }

    fn codex_oauth_runtime_url(&self) -> String {
        self.provider_endpoints
            .codex_oauth_runtime_url
            .clone()
            .or_else(|| std::env::var("CODEX_OAUTH_RUNTIME_URL").ok())
            .unwrap_or_else(|| "http://127.0.0.1:9110".to_string())
    }

    pub async fn enrich_runtime_status(&self, mut status: Value) -> Value {
        let providers = self.provider_rows().await;
        let tools = self.tool_rows();
        let skills = self.skill_rows();
        let plugins = self.plugin_rows();
        let mcp = self.mcp_rows();
        let route = first_routeable_provider(&providers);
        let provider_chat_state = if route.is_some() {
            "available"
        } else {
            "degraded"
        };
        let permission_state = status
            .get("permissions")
            .cloned()
            .unwrap_or_else(|| json!({"profile": "restricted", "state": "restricted"}));

        status["registries"] = json!({
            "tool_count": tools.len(),
            "skill_count": skills.len(),
            "plugin_count": plugins.len(),
            "mcp_count": mcp.len(),
            "tools": registry_state(tools.len(), "available", None),
            "skills": registry_state(skills.len(), "available", None),
            "plugins": registry_state(plugins.len(), "available", None),
            "mcp": registry_state(mcp.len(), "available", None),
            "apps": registry_state(0, "available", None)
        });
        if let Some(route) = route.clone() {
            status["provider_route"] = route;
        } else {
            status["provider_route"] = json!({
                "state": "degraded",
                "routeable": false,
                "reason": "no routeable provider/model registered"
            });
        }
        status["provider_routes"] = Value::Array(route.into_iter().collect());
        status["chat_orchestrator"] = json!({
            "state": "available",
            "owner": "capability_mesh",
            "loop": "recall_preflight_capability_registry_permission_gate_broker_trace_answer_gate",
            "answer_gate": "trace_backed_answer_gate_v1",
            "memory_recall_tool": "brain.memory.recall",
            "tool_count": tools.len(),
            "last_blocking_reason": null
        });

        let mut capabilities = status
            .get("capabilities")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        capabilities.extend([
            capability(
                "one_public_http_server",
                "One Public HTTP Server",
                "available",
                None,
            ),
            capability(
                "provider_chat",
                "Provider Chat",
                provider_chat_state,
                (provider_chat_state != "available")
                    .then_some("no routeable provider/model registered"),
            ),
            capability("chat_orchestrator", "Chat Orchestrator", "available", None),
            capability("memory_recall", "Memory Recall", "available", None),
            capability("answer_gate", "Trace-Backed Answer Gate", "available", None),
            capability("tools_registry", "Tools", "available", None),
            capability("skills_registry", "Skills", "available", None),
            capability("plugins_registry", "Plugins", "available", None),
            capability("mcp_registry", "MCP", "available", None),
        ]);
        status["capabilities"] = Value::Array(capabilities);
        status["permissions"] = permission_state;
        status
    }

    pub async fn providers(&self) -> (StatusCode, Value) {
        let providers = self.provider_rows().await;
        (
            StatusCode::OK,
            json!({
                "ok": true,
                "providers": providers,
                "items": providers,
                "total": providers.len(),
                "source": "capability_mesh"
            }),
        )
    }

    pub async fn provider_detail(&self, provider_id: &str) -> (StatusCode, Value) {
        let providers = self.provider_rows().await;
        match providers
            .into_iter()
            .find(|provider| provider["id"].as_str() == Some(provider_id))
        {
            Some(provider) => (StatusCode::OK, json!({"ok": true, "status": provider})),
            None => not_found("provider_not_found", json!({"provider_id": provider_id})),
        }
    }

    pub async fn provider_models(&self, provider_id: &str) -> (StatusCode, Value) {
        let providers = self.provider_rows().await;
        match providers
            .into_iter()
            .find(|provider| provider["id"].as_str() == Some(provider_id))
        {
            Some(provider) => {
                let models = provider
                    .get("models")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                (
                    StatusCode::OK,
                    json!({"ok": true, "models": models, "items": models, "total": models.len()}),
                )
            }
            None => not_found("provider_not_found", json!({"provider_id": provider_id})),
        }
    }

    pub async fn provider_discover(&self, body: Value) -> (StatusCode, Value) {
        let provider_id = body
            .get("provider_id")
            .or_else(|| body.get("providerId"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if provider_id.is_empty() {
            return bad_request("provider_id_required", json!({}));
        }
        let Some(entry) = provider_catalog::find_provider(provider_id) else {
            return not_found("provider_not_found", json!({"provider_id": provider_id}));
        };
        match self.discover_provider_models(entry, &body).await {
            Ok(models) => {
                let should_persist = body.get("persist").and_then(Value::as_bool).unwrap_or(true);
                let stored_path = if should_persist {
                    store_provider_snapshot(entry.id, &models).ok()
                } else {
                    None
                };
                (
                    StatusCode::OK,
                    json!({
                        "ok": true,
                        "provider_id": entry.id,
                        "source": "live_provider_catalog",
                        "registry_snapshot": {
                            "provider_id": entry.id,
                            "source": "live_provider_catalog",
                            "models": models,
                            "total": models.len(),
                            "refresh_policy": "manual_or_quarterly",
                            "refresh_due_days": 90,
                            "stored_path": stored_path,
                            "secret_logged": false
                        },
                        "models": models,
                        "items": models,
                        "total": models.len()
                    }),
                )
            }
            Err(error) => error,
        }
    }

    pub fn save_provider_credential(&self, body: Value) -> (StatusCode, Value) {
        let provider_id = string_field(&body, &["provider_id", "providerId"]).unwrap_or_default();
        if provider_id.is_empty() {
            return bad_request("provider_id_required", json!({}));
        }
        let Some(entry) = provider_catalog::find_provider(&provider_id) else {
            return not_found("provider_not_found", json!({"provider_id": provider_id}));
        };
        let Some(api_key) = request_api_key(&body) else {
            return bad_request("provider_key_missing", json!({"provider_id": entry.id}));
        };
        match store_provider_credential(entry.id, &api_key) {
            Ok(credential_ref) => (
                StatusCode::OK,
                json!({
                    "ok": true,
                    "provider_id": entry.id,
                    "credential_ref": credential_ref,
                    "credential": {
                        "provider_id": entry.id,
                        "credential_ref": credential_ref,
                        "storage_backend": credential_store_backend(),
                        "secret_material_returned": false
                    },
                    "secret_material_returned": false,
                    "raw_secret_logged": false
                }),
            ),
            Err(error) => error_payload(
                StatusCode::INTERNAL_SERVER_ERROR,
                -32090,
                "credential_store_failed",
                json!({"provider_id": entry.id, "error": error.to_string()}),
            ),
        }
    }

    pub async fn provider_connect(
        &self,
        provider_id: &str,
        mut body: Value,
    ) -> (StatusCode, Value) {
        let Some(entry) = provider_catalog::find_provider(provider_id) else {
            return not_found("provider_not_found", json!({"provider_id": provider_id}));
        };
        if let Some(map) = body.as_object_mut() {
            map.insert("provider_id".to_string(), json!(entry.id));
        }
        let Some(api_key) = resolve_catalog_api_key(entry, &body) else {
            return bad_request("provider_key_missing", json!({"provider_id": entry.id}));
        };
        let credential_source =
            if string_field(&body, &["credential_ref", "credentialRef"]).is_some() {
                "credential_ref"
            } else {
                "inline_or_env"
            };
        let (discover_status, discover_value) = self.provider_discover(body.clone()).await;
        if !discover_status.is_success() {
            return (discover_status, discover_value);
        }
        let models = discover_value
            .get("items")
            .or_else(|| discover_value.get("models"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let model_id = string_field(&body, &["model_id", "modelId", "model"])
            .or_else(|| {
                models
                    .first()
                    .and_then(|model| model.get("id").and_then(Value::as_str).map(str::to_string))
            })
            .or_else(|| entry.default_model.map(str::to_string));
        let Some(model_id) = model_id else {
            return error_payload(
                StatusCode::BAD_GATEWAY,
                -32050,
                "no_models_returned",
                json!({"provider_id": entry.id}),
            );
        };
        let mut validation_body = body.clone();
        if let Some(map) = validation_body.as_object_mut() {
            map.insert("api_key".to_string(), json!(api_key));
            map.insert("model_id".to_string(), json!(model_id));
            map.insert("max_tokens".to_string(), json!(16));
            map.insert("stream".to_string(), json!(false));
            map.insert(
                "messages".to_string(),
                json!([{"role": "user", "content": "Reply with OK."}]),
            );
        }
        let validation_messages = messages_from_body(&validation_body)
            .unwrap_or_else(|| vec![json!({"role": "user", "content": "Reply with OK."})]);
        let validation = match self
            .dispatch_provider_chat(entry.id, &model_id, &validation_messages, &validation_body)
            .await
        {
            Ok(content) => json!({
                "hidden_backend_validation": true,
                "chat_status": "passed",
                "model_id": model_id,
                "response_observed": !content.trim().is_empty()
            }),
            Err((status, error)) => {
                return (
                    status,
                    json!({
                        "ok": false,
                        "provider_id": entry.id,
                        "connection_state": "model_validation_failed",
                        "model_registry": discover_value.get("registry_snapshot").cloned().unwrap_or_else(|| json!({})),
                        "validation": {
                            "hidden_backend_validation": true,
                            "chat_status": "failed",
                            "error": error.get("error").cloned().unwrap_or(error)
                        }
                    }),
                );
            }
        };
        (
            StatusCode::OK,
            json!({
                "ok": true,
                "provider_id": entry.id,
                "connection_state": "ready",
                "model_registry": discover_value.get("registry_snapshot").cloned().unwrap_or_else(|| json!({})),
                "models": models,
                "selected_model_id": model_id,
                "validation": validation,
                "credential": {
                    "resolved_from": credential_source,
                    "secret_material_returned": false
                }
            }),
        )
    }

    pub async fn provider_preflight(&self, body: Value) -> (StatusCode, Value) {
        let provider_id = body
            .get("provider_id")
            .or_else(|| body.get("providerId"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let model = body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if provider_id.is_empty() || model.is_empty() {
            return bad_request("provider_id_and_model_required", json!({}));
        }
        let providers = self.provider_rows().await;
        let registry_can_chat = providers.iter().any(|provider| {
            provider["id"].as_str() == Some(provider_id)
                && provider
                    .get("models")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|item| item["id"].as_str() == Some(model) && item["can_chat"] != false)
        });
        let dynamic_can_chat = match provider_id {
            "codex-oauth" => string_field(&body, &["credential_ref", "credentialRef"])
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false),
            _ => false,
        };
        let can_chat = registry_can_chat || dynamic_can_chat;
        (
            StatusCode::OK,
            json!({
                "ok": true,
                "provider_id": provider_id,
                "model_id": model,
                "can_chat": can_chat,
                "state": if can_chat { "available" } else { "degraded" }
            }),
        )
    }

    pub fn tools(&self) -> (StatusCode, Value) {
        let tools = self.tool_rows();
        (
            StatusCode::OK,
            json!({"ok": true, "tools": tools, "items": tools, "total": tools.len(), "source": "capability_mesh", "tool_runtime": self.tool_runtime_continuity_snapshot()}),
        )
    }

    pub fn tool_detail(&self, tool_id: &str) -> (StatusCode, Value) {
        match tool_descriptors()
            .into_iter()
            .find(|tool| tool.id == tool_id)
            .map(|tool| tool.to_value())
        {
            Some(tool) => (StatusCode::OK, json!({"ok": true, "status": tool})),
            None => not_found("tool_not_found", json!({"tool_id": tool_id})),
        }
    }

    pub async fn call_tool(&self, brain: &BrainClient, body: Value) -> (StatusCode, Value) {
        let tool_id = body
            .get("tool_id")
            .or_else(|| body.get("toolId"))
            .or_else(|| body.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let args = body
            .get("arguments")
            .or_else(|| body.get("input"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let execution = self
            .execute_tool(brain, tool_id, args, "api", "api", None)
            .await;
        (execution.status, execution.into_response())
    }

    pub fn skills(&self) -> (StatusCode, Value) {
        let skills = self.skill_rows();
        (
            StatusCode::OK,
            json!({
                "ok": true,
                "skills": skills,
                "items": skills,
                "total": skills.len(),
                "limit": SKILL_DISPLAY_LIMIT,
                "source": "capability_mesh",
                "service_owner": "skills-runtime/capability-mesh",
                "import_state": "pending",
                "import_pending_reason": "session skill import/enable substrate is not implemented"
            }),
        )
    }

    pub fn plugins(&self) -> (StatusCode, Value) {
        let plugins = self.plugin_rows();
        (
            StatusCode::OK,
            json!({
                "ok": true,
                "plugins": plugins,
                "items": plugins,
                "total": plugins.len(),
                "source": "capability_mesh",
                "artifact_kind": "plugin_registry_v1",
                "service_owner": "plugins-runtime/capability-mesh",
                "install_supported": true,
                "enable_supported": true,
                "disable_supported": true,
                "uninstall_supported": true,
                "persistence": self.plugin_store_read_metadata()
            }),
        )
    }

    pub fn plugins_marketplace(&self) -> (StatusCode, Value) {
        let installed = self.plugin_rows();
        (
            StatusCode::OK,
            json!({
                "ok": true,
                "artifact_kind": "plugin_marketplace_v1",
                "source": "capability_mesh",
                "service_owner": "plugins-runtime/capability-mesh",
                "items": [],
                "installed": installed,
                "install_supported": true,
                "reason": "Marketplace discovery is empty until external catalog integration exists; local install/enable/disable/uninstall lifecycle is active."
            }),
        )
    }

    pub fn plugin_install(&self, body: Value) -> (StatusCode, Value) {
        match self.apply_plugin_lifecycle("install", body) {
            Ok(value) => (StatusCode::OK, value),
            Err((status, value)) => (status, value),
        }
    }

    pub fn plugin_enable(&self, body: Value) -> (StatusCode, Value) {
        match self.apply_plugin_lifecycle("enable", body) {
            Ok(value) => (StatusCode::OK, value),
            Err((status, value)) => (status, value),
        }
    }

    pub fn plugin_disable(&self, body: Value) -> (StatusCode, Value) {
        match self.apply_plugin_lifecycle("disable", body) {
            Ok(value) => (StatusCode::OK, value),
            Err((status, value)) => (status, value),
        }
    }

    pub fn plugin_uninstall(&self, body: Value) -> (StatusCode, Value) {
        match self.apply_plugin_lifecycle("uninstall", body) {
            Ok(value) => (StatusCode::OK, value),
            Err((status, value)) => (status, value),
        }
    }

    pub fn apps_skills(&self) -> (StatusCode, Value) {
        let skills = self.skill_rows();
        let plugins = self.plugin_rows();
        let tools = self.tool_rows();
        let mcp = self.mcp_rows();
        (
            StatusCode::OK,
            json!({
                "ok": true,
                "skills": skills,
                "plugins": plugins,
                "tools": tools,
                "mcp_servers": mcp,
                "items": combined_registry_items(&tools, &skills, &plugins, &mcp),
                "source": "capability_mesh"
            }),
        )
    }

    pub fn mcp_servers(&self) -> (StatusCode, Value) {
        let mcp = self.mcp_rows();
        (
            StatusCode::OK,
            json!({"ok": true, "items": mcp, "mcp_servers": mcp, "total": mcp.len(), "source": "capability_mesh"}),
        )
    }

    pub fn mcp_config(&self) -> (StatusCode, Value) {
        let configs = self.mcp_rows();
        (
            StatusCode::OK,
            json!({"ok": true, "configs": configs, "items": configs, "total": configs.len(), "source": "capability_mesh"}),
        )
    }

    pub fn mcp_status(&self) -> (StatusCode, Value) {
        let servers = self.mcp_rows();
        let tools_total = mcp_tool_rows(&servers).len();
        (
            StatusCode::OK,
            json!({
                "ok": true,
                "artifact_kind": "mcp_runtime_status_v1",
                "owner_service": "mcp-runtime",
                "source": "capability_mesh",
                "brain_forwarded": false,
                "runtime_state": if servers.iter().any(|server| server["state"] == "connected") { "active" } else { "configured_not_connected" },
                "state": if servers.is_empty() { "unconfigured" } else { "configured_not_connected" },
                "servers": servers,
                "server_count": servers.len(),
                "tool_count": tools_total,
                "connect_supported": true,
                "tools_route_live": true,
                "reason": "MCP runtime lifecycle substrate is live for config normalization and state tracking; protocol session persistence and real tool execution are promoted in later Phase 4.9 slices.",
                "routes": {
                    "servers": "/api/ui/mcp-servers",
                    "status": "/api/ui/mcp/status",
                    "connect": "/api/ui/mcp/connect",
                    "tools": "/api/ui/mcp/tools"
                }
            }),
        )
    }

    pub fn mcp_create(&self, body: Value) -> (StatusCode, Value) {
        let Some(mut obj) = body.as_object().cloned() else {
            return bad_request(
                "mcp_create_invalid_payload",
                json!({"message": "MCP create payload must be a JSON object"}),
            );
        };

        let raw_id = obj
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .or_else(|| {
                obj.get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
            });
        let Some(raw_id) = raw_id else {
            return bad_request(
                "mcp_create_missing_id",
                json!({"message": "Provide id or name for MCP create"}),
            );
        };
        let server_id = raw_id
            .to_ascii_lowercase()
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_string();
        if server_id.is_empty() {
            return bad_request(
                "mcp_create_invalid_id",
                json!({"message": "Derived MCP id is empty after normalization"}),
            );
        }

        let root = self.mcp_config_root();
        if let Err(error) = std::fs::create_dir_all(&root) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({
                    "ok": false,
                    "artifact_kind": "mcp_create_result_v1",
                    "outcome": "failed",
                    "server_id": server_id,
                    "owner_service": "mcp-runtime",
                    "source": "capability_mesh",
                    "last_error": {"code": "mcp_config_root_create_failed", "message": error.to_string()}
                }),
            );
        }

        let path = root.join(format!("{}.mcp.json", server_id));
        let existed = path.exists();

        obj.insert("id".to_string(), json!(server_id.clone()));
        if !obj.contains_key("enabled") {
            obj.insert("enabled".to_string(), json!(true));
        }

        let payload = Value::Object(obj);
        let serialized = match serde_json::to_string_pretty(&payload) {
            Ok(value) => value,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({
                        "ok": false,
                        "artifact_kind": "mcp_create_result_v1",
                        "outcome": "failed",
                        "server_id": server_id,
                        "owner_service": "mcp-runtime",
                        "source": "capability_mesh",
                        "last_error": {"code": "mcp_create_serialize_failed", "message": error.to_string()}
                    }),
                );
            }
        };

        if let Err(error) = std::fs::write(&path, serialized) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({
                    "ok": false,
                    "artifact_kind": "mcp_create_result_v1",
                    "outcome": "failed",
                    "server_id": server_id,
                    "owner_service": "mcp-runtime",
                    "source": "capability_mesh",
                    "last_error": {"code": "mcp_create_write_failed", "message": error.to_string()}
                }),
            );
        }

        let server = self.normalized_mcp_row(&path);
        (
            StatusCode::OK,
            json!({
                "ok": true,
                "artifact_kind": "mcp_create_result_v1",
                "outcome": "executed",
                "applied": true,
                "server_id": server_id,
                "created": !existed,
                "updated": existed,
                "owner_service": "mcp-runtime",
                "source": "capability_mesh",
                "brain_forwarded": false,
                "server": server,
                "routes": {
                    "servers": "/api/ui/mcp-servers",
                    "status": "/api/ui/mcp/status",
                    "connect": "/api/ui/mcp/connect"
                }
            }),
        )
    }

    pub fn workspace_list_directory(&self, args: Value) -> (StatusCode, Value) {
        match list_directory(args) {
            Ok(result) => {
                let entries = result["entries"].as_array().cloned().unwrap_or_default();
                (
                    StatusCode::OK,
                    json!({
                        "ok": true,
                        "artifact_kind": "workspace_directory_listing_v1",
                        "entries": entries,
                        "count": entries.len(),
                        "source": "capability_mesh/filesystem.list"
                    }),
                )
            }
            Err((status, error)) => (status, error),
        }
    }

    pub fn workspace_read_file(&self, args: Value) -> (StatusCode, Value) {
        match read_file_chunk(args) {
            Ok(result) => (
                StatusCode::OK,
                json!({
                    "ok": true,
                    "artifact_kind": "workspace_file_read_v1",
                    "text": result["text"],
                    "path": result["path"],
                    "size_bytes": result["size_bytes"],
                    "offset": result["offset"],
                    "bytes_read": result["bytes_read"],
                    "truncated": result["truncated"],
                    "next_offset": result["next_offset"],
                    "source": "capability_mesh/filesystem.read"
                }),
            ),
            Err((status, error)) => (status, error),
        }
    }

    pub fn workspace_search_files(&self, args: Value) -> (StatusCode, Value) {
        match search_filesystem(args) {
            Ok(result) => {
                let matches = result["matches"].as_array().cloned().unwrap_or_default();
                (
                    StatusCode::OK,
                    json!({
                        "ok": true,
                        "artifact_kind": "workspace_file_search_v1",
                        "matches": matches,
                        "count": matches.len(),
                        "source": "capability_mesh/filesystem.search"
                    }),
                )
            }
            Err((status, error)) => (status, error),
        }
    }

    pub fn workspace_file_write_preview(&self, args: Value) -> (StatusCode, Value) {
        match self.preview_file_write(args) {
            Ok(result) => (
                StatusCode::OK,
                json!({
                    "ok": true,
                    "artifact_kind": "workspace_file_write_preview_v1",
                    "preview_id": result["preview_id"],
                    "path": result["path"],
                    "mode": result["mode"],
                    "content_bytes": result["content_bytes"],
                    "content_sha256": result["content_sha256"],
                    "existing_bytes": result["existing_bytes"],
                    "source": "capability_mesh/filesystem.write.preview"
                }),
            ),
            Err((status, error)) => (status, error),
        }
    }

    pub fn workspace_file_write_apply(&self, args: Value) -> (StatusCode, Value) {
        match self.apply_file_write(args) {
            Ok(result) => (
                StatusCode::OK,
                json!({
                    "ok": true,
                    "artifact_kind": "workspace_file_write_apply_v1",
                    "path": result["path"],
                    "mode": result["mode"],
                    "applied": result["applied"],
                    "preview_id": result["preview_id"],
                    "execution_certificate": result["execution_certificate"],
                    "source": "capability_mesh/filesystem.write.apply"
                }),
            ),
            Err((status, error)) => (status, error),
        }
    }

    pub fn inspect_local_paths(&self, paths: Vec<String>) -> Value {
        let mut assets = Vec::new();
        for path_str in paths.iter().take(16) {
            let path = std::path::Path::new(path_str);
            let Ok(metadata) = std::fs::metadata(path) else {
                assets.push(json!({
                    "path": path_str,
                    "name": path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown"),
                    "kind": "not_found",
                    "size_bytes": Value::Null,
                    "extension": Value::Null,
                    "preview_text": Value::Null
                }));
                continue;
            };
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_string());
            let kind = if metadata.is_dir() { "folder" } else { "file" };
            let size_bytes = metadata.len();
            let text_preview_limit_bytes: u64 = 65536;
            let text_preview_limit_chars: usize = 50000;
            let is_text_previewable = extension.as_ref().map_or(false, |ext| {
                matches!(
                    ext.to_lowercase().as_str(),
                    "txt"
                        | "md"
                        | "rs"
                        | "ts"
                        | "js"
                        | "json"
                        | "toml"
                        | "yaml"
                        | "yml"
                        | "html"
                        | "css"
                        | "py"
                        | "go"
                        | "sh"
                        | "bash"
                        | "zsh"
                        | "sql"
                        | "xml"
                        | "csv"
                        | "env"
                        | "cfg"
                        | "conf"
                        | "ini"
                        | "log"
                )
            }) && !metadata.is_dir()
                && size_bytes <= text_preview_limit_bytes;
            let preview_text = if is_text_previewable {
                std::fs::read_to_string(path).ok().map(|content| {
                    if content.len() > text_preview_limit_chars {
                        format!("{}...(truncated)", &content[..text_preview_limit_chars])
                    } else {
                        content
                    }
                })
            } else {
                None
            };
            assets.push(json!({
                "path": path_str,
                "name": name,
                "kind": kind,
                "size_bytes": size_bytes,
                "extension": extension,
                "preview_text": preview_text
            }));
        }
        json!({
            "ok": true,
            "artifact_kind": "local_path_inspection_v1",
            "assets": assets,
            "count": assets.len(),
            "source": "ingress_filesystem_bridge"
        })
    }

    fn code_review_store_path(&self) -> PathBuf {
        self.code_review_store_path
            .clone()
            .unwrap_or_else(|| hom_local_dir().join("runtime/code-reviews.json"))
    }

    fn read_code_review_store(&self) -> Value {
        let path = self.code_review_store_path();
        std::fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str::<Value>(&content).ok())
            .unwrap_or_else(|| json!({"artifact_kind": "code_review_store_v1", "reviews": {}}))
    }

    fn write_code_review_store(&self, store: &Value) -> Result<(), (StatusCode, Value)> {
        let path = self.code_review_store_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                error_payload(
                    StatusCode::BAD_REQUEST,
                    -32050,
                    "code_review_store_write_failed",
                    json!({"path": parent.display().to_string(), "error": error.to_string()}),
                )
            })?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(store).unwrap_or_default()).map_err(
            |error| {
                error_payload(
                    StatusCode::BAD_REQUEST,
                    -32050,
                    "code_review_store_write_failed",
                    json!({"path": tmp.display().to_string(), "error": error.to_string()}),
                )
            },
        )?;
        std::fs::rename(&tmp, &path).map_err(|error| {
            error_payload(
                StatusCode::BAD_REQUEST,
                -32050,
                "code_review_store_write_failed",
                json!({"path": path.display().to_string(), "error": error.to_string()}),
            )
        })?;
        Ok(())
    }

    pub fn code_review_propose(
        &self,
        path: String,
        diff: String,
        summary: Option<String>,
    ) -> (StatusCode, Value) {
        let review_id = format!(
            "review_{}",
            &sha256_hex(&format!("{}:{}", path, now_ms()))[..16]
        );
        let mut store = self.read_code_review_store();
        if !store["reviews"].is_object() {
            store["reviews"] = json!({});
        }
        let existing_content = std::fs::read_to_string(&path).ok();
        let record = json!({
            "review_id": review_id,
            "path": path,
            "diff": diff,
            "summary": summary.unwrap_or_default(),
            "status": "pending",
            "existing_content_sha256": existing_content.as_ref().map(|c| sha256_hex(c)),
            "proposed_at": now_ms(),
            "resolved_at": Value::Null
        });
        store["reviews"][&review_id] = record.clone();
        match self.write_code_review_store(&store) {
            Ok(()) => (
                StatusCode::OK,
                json!({
                    "ok": true,
                    "artifact_kind": "code_review_proposal_v1",
                    "review_id": review_id,
                    "path": path,
                    "status": "pending",
                    "source": "capability_mesh/code_review.propose"
                }),
            ),
            Err((status, error)) => (status, error),
        }
    }

    pub fn code_review_accept(&self, review_id: &str) -> (StatusCode, Value) {
        let mut store = self.read_code_review_store();
        let Some(record) = store
            .get_mut("reviews")
            .and_then(Value::as_object_mut)
            .and_then(|r| r.get_mut(review_id))
        else {
            return (
                StatusCode::NOT_FOUND,
                json!({
                    "ok": false,
                    "error": {"code": -32044, "message": "review_not_found", "data": {"review_id": review_id}},
                    "source": "capability_mesh/code_review.accept"
                }),
            );
        };
        if record["status"].as_str() != Some("pending") {
            return (
                StatusCode::CONFLICT,
                json!({
                    "ok": false,
                    "error": {"code": -32081, "message": "review_already_resolved", "data": {"review_id": review_id, "current_status": record["status"]}},
                    "source": "capability_mesh/code_review.accept"
                }),
            );
        }
        let path = record["path"].as_str().unwrap_or_default().to_string();
        record["status"] = json!("accepted");
        record["resolved_at"] = json!(now_ms());
        match self.write_code_review_store(&store) {
            Ok(()) => (
                StatusCode::OK,
                json!({
                    "ok": true,
                    "artifact_kind": "code_review_acceptance_v1",
                    "review_id": review_id,
                    "path": path,
                    "status": "accepted",
                    "source": "capability_mesh/code_review.accept"
                }),
            ),
            Err((status, error)) => (status, error),
        }
    }

    pub fn code_review_reject(
        &self,
        review_id: &str,
        reason: Option<String>,
    ) -> (StatusCode, Value) {
        let mut store = self.read_code_review_store();
        let Some(record) = store
            .get_mut("reviews")
            .and_then(Value::as_object_mut)
            .and_then(|r| r.get_mut(review_id))
        else {
            return (
                StatusCode::NOT_FOUND,
                json!({
                    "ok": false,
                    "error": {"code": -32044, "message": "review_not_found", "data": {"review_id": review_id}},
                    "source": "capability_mesh/code_review.reject"
                }),
            );
        };
        if record["status"].as_str() != Some("pending") {
            return (
                StatusCode::CONFLICT,
                json!({
                    "ok": false,
                    "error": {"code": -32081, "message": "review_already_resolved", "data": {"review_id": review_id, "current_status": record["status"]}},
                    "source": "capability_mesh/code_review.reject"
                }),
            );
        }
        record["status"] = json!("rejected");
        record["rejection_reason"] = json!(reason.unwrap_or_default());
        record["resolved_at"] = json!(now_ms());
        match self.write_code_review_store(&store) {
            Ok(()) => (
                StatusCode::OK,
                json!({
                    "ok": true,
                    "artifact_kind": "code_review_rejection_v1",
                    "review_id": review_id,
                    "status": "rejected",
                    "source": "capability_mesh/code_review.reject"
                }),
            ),
            Err((status, error)) => (status, error),
        }
    }

    pub fn code_review_diff(&self, review_id: &str) -> (StatusCode, Value) {
        let store = self.read_code_review_store();
        let Some(record) = store
            .get("reviews")
            .and_then(Value::as_object)
            .and_then(|r| r.get(review_id))
        else {
            return (
                StatusCode::NOT_FOUND,
                json!({
                    "ok": false,
                    "error": {"code": -32044, "message": "review_not_found", "data": {"review_id": review_id}},
                    "source": "capability_mesh/code_review.diff"
                }),
            );
        };
        (
            StatusCode::OK,
            json!({
                "ok": true,
                "artifact_kind": "code_review_diff_v1",
                "review_id": review_id,
                "path": record["path"],
                "diff": record["diff"],
                "summary": record["summary"],
                "status": record["status"],
                "source": "capability_mesh/code_review.diff"
            }),
        )
    }

    pub fn code_review_list(&self) -> (StatusCode, Value) {
        let store = self.read_code_review_store();
        let reviews = store
            .get("reviews")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let count = reviews.len();
        (
            StatusCode::OK,
            json!({
                "ok": true,
                "artifact_kind": "code_review_list_v1",
                "reviews": reviews,
                "count": count,
                "source": "capability_mesh/code_review.list"
            }),
        )
    }

    pub fn plugin_runtime_continuity_snapshot(&self) -> Value {
        let rows = self.plugin_rows();
        let installed_count = rows.len();
        let enabled_count = rows
            .iter()
            .filter(|plugin| plugin["enabled"].as_bool().unwrap_or(false))
            .count();
        let disabled_count = installed_count.saturating_sub(enabled_count);
        let plugins = rows
            .iter()
            .map(|plugin| {
                json!({
                    "id": plugin["id"].clone(),
                    "name": plugin["name"].clone(),
                    "version": plugin["version"].clone(),
                    "enabled": plugin["enabled"].clone(),
                    "state": plugin["state"].clone()
                })
            })
            .collect::<Vec<_>>();
        json!({
            "artifact_kind": "plugin_runtime_continuity_v1",
            "source": "capability_mesh",
            "store": self.plugin_store_read_metadata(),
            "installed_count": installed_count,
            "enabled_count": enabled_count,
            "disabled_count": disabled_count,
            "restart_survival": "durable_plugin_store_survives_ingress_restart",
            "compaction_survival": "plugin_runtime_continuity_derived_from_durable_store",
            "plugins": plugins
        })
    }

    pub fn tool_runtime_continuity_snapshot(&self) -> Value {
        let store = self.read_tool_preview_store();
        let previews = store
            .get("previews")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let pending_previews = previews
            .iter()
            .map(|(preview_id, record)| {
                json!({
                    "preview_id": preview_id,
                    "tool_id": record.get("tool_id").cloned().unwrap_or_else(|| json!("filesystem.write.preview")),
                    "path": record.get("path").cloned().unwrap_or(Value::Null),
                    "mode": record.get("mode").cloned().unwrap_or(Value::Null),
                    "content_sha256": record.get("content_sha256").cloned().unwrap_or(Value::Null),
                    "content_bytes": record.get("content_bytes").cloned().unwrap_or(Value::Null),
                    "created_at": record.get("created_at").cloned().unwrap_or(Value::Null)
                })
            })
            .collect::<Vec<_>>();
        let applied_certificate_count = store
            .get("applied")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        json!({
            "artifact_kind": "tool_runtime_continuity_v1",
            "owner_service": "tools-runtime/capability-mesh",
            "source": "capability_mesh",
            "preview_store_path": self.tool_preview_store_path().display().to_string(),
            "pending_preview_count": pending_previews.len(),
            "applied_certificate_count": applied_certificate_count,
            "pending_previews": pending_previews,
            "restart_survival": "durable_tool_preview_store_survives_ingress_restart",
            "compaction_survival": "compaction_receives_tool_preview_metadata_without_content",
            "content_redaction": "preview_content_is_never_in_continuity_payload"
        })
    }

    pub fn mcp_runtime_continuity_snapshot(&self) -> Value {
        let servers = self.mcp_rows();
        let tools = mcp_tool_rows(&servers);
        let runtime_state = if servers.iter().any(|server| server["state"] == "connected") {
            "active"
        } else if servers.is_empty() {
            "unconfigured"
        } else {
            "configured_not_connected"
        };
        json!({
            "artifact_kind": "mcp_runtime_continuity_v1",
            "owner_service": "mcp-runtime",
            "source": "capability_mesh",
            "runtime_state": runtime_state,
            "server_count": servers.len(),
            "tool_count": tools.len(),
            "servers": servers,
            "runtime_handle_survival": "ephemeral_ingress_process_only",
            "restart_survival": "config_survives_restart_live_process_handles_do_not",
            "compaction_source_of_truth": "capability_mesh_runtime_snapshot_plus_durable_mcp_config"
        })
    }

    pub fn mcp_tools(&self) -> (StatusCode, Value) {
        let servers = self.mcp_rows();
        let tools = mcp_tool_rows(&servers);
        (
            StatusCode::OK,
            json!({
                "ok": true,
                "artifact_kind": "mcp_tools_index_v1",
                "owner_service": "mcp-runtime",
                "source": "capability_mesh",
                "brain_forwarded": false,
                "runtime_state": if servers.iter().any(|server| server["state"] == "connected") { "active" } else { "configured_not_connected" },
                "servers": servers,
                "tools": tools,
                "server_count": servers.len(),
                "tool_count": tools.len(),
                "reason": "Tool listing distinguishes configured metadata from live discovered tools; MCP tools are executable only after a live session discovers them."
            }),
        )
    }

    pub fn mcp_connect(&self, server_id: Option<String>) -> (StatusCode, Value) {
        let Some(server_id) = server_id else {
            return (
                StatusCode::BAD_REQUEST,
                json!({
                    "ok": false,
                    "outcome": "failed",
                    "server_id": "unknown",
                    "owner_service": "mcp-runtime",
                    "source": "capability_mesh",
                    "brain_forwarded": false,
                    "state": "failed",
                    "last_error": {"code": "mcp_missing_server_id", "message": "server_id is required"}
                }),
            );
        };
        let servers = self.mcp_rows();
        let Some(server) = servers.iter().find(|server| server["id"] == server_id) else {
            return self.mcp_unconfigured_response(server_id);
        };
        if server["enabled"].as_bool() == Some(false) {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                json!({
                    "ok": false,
                    "outcome": "unavailable",
                    "server_id": server_id,
                    "owner_service": "mcp-runtime",
                    "source": "capability_mesh",
                    "brain_forwarded": false,
                    "state": "disabled",
                    "last_error": {"code": "mcp_server_disabled", "message": "MCP server is disabled"},
                    "error": {"code": "mcp_server_disabled", "message": "MCP server is disabled"}
                }),
            );
        }
        if server["state"] == "failed" && server["transport"] == "invalid" {
            return self.record_mcp_failed_state(
                server_id,
                server["transport"].clone(),
                server["last_error"].clone(),
            );
        }
        if server["transport"] != "stdio" {
            return self.record_mcp_failed_state(
                server_id,
                server["transport"].clone(),
                json!({"code": "mcp_http_session_not_implemented", "message": "HTTP MCP sessions are not implemented in this runtime slice"}),
            );
        }
        if let Ok(mut guard) = self.mcp_runtime.lock() {
            guard.insert(
                server_id.clone(),
                McpRuntimeEntry {
                    row: json!({
                        "state": "connecting",
                        "last_connect_attempt_at": now_ms(),
                        "connected_at": null,
                        "last_error": null,
                        "transport": server["transport"],
                        "discovered_tool_count": 0,
                        "discovered_tools": [],
                        "process_handle": null
                    }),
                    session: None,
                },
            );
        }
        match self.start_stdio_mcp_session(server) {
            Ok((session, mut runtime_row)) => {
                let connected_at = runtime_row["connected_at"].clone();
                if let Ok(mut guard) = self.mcp_runtime.lock() {
                    guard.insert(
                        server_id.clone(),
                        McpRuntimeEntry {
                            row: runtime_row.clone(),
                            session: Some(session),
                        },
                    );
                }
                let server = self
                    .mcp_config_path_for_id(&server_id)
                    .map(|path| self.normalized_mcp_row(&path))
                    .unwrap_or_else(|| {
                        runtime_row["id"] = json!(server_id.clone());
                        runtime_row.clone()
                    });
                (
                    StatusCode::OK,
                    json!({
                        "ok": true,
                        "outcome": "connected",
                        "server_id": server_id,
                        "owner_service": "mcp-runtime",
                        "source": "capability_mesh",
                        "brain_forwarded": false,
                        "runtime_state": "active",
                        "state": "connected",
                        "connected_at": connected_at,
                        "last_connect_attempt_at": runtime_row["last_connect_attempt_at"],
                        "last_error": null,
                        "server": server,
                        "routes": {
                            "status": "/api/ui/mcp/status",
                            "tools": "/api/ui/mcp/tools",
                            "call": "/api/ui/mcp/tools/call"
                        }
                    }),
                )
            }
            Err(error) => {
                self.record_mcp_failed_state(server_id, server["transport"].clone(), error)
            }
        }
    }

    fn mcp_unconfigured_response(&self, server_id: String) -> (StatusCode, Value) {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({
                "ok": false,
                "outcome": "unavailable",
                "server_id": server_id,
                "owner_service": "mcp-runtime",
                "source": "capability_mesh",
                "brain_forwarded": false,
                "state": "unconfigured",
                "last_error": {"code": "mcp_server_not_configured", "message": "MCP server is not configured"},
                "error": {"code": "mcp_server_not_configured", "message": "MCP server is not configured"}
            }),
        )
    }

    fn record_mcp_failed_state(
        &self,
        server_id: String,
        transport: Value,
        error: Value,
    ) -> (StatusCode, Value) {
        let protocol_session_implemented =
            error["code"].as_str() != Some("mcp_http_session_not_implemented");
        let runtime_row = json!({
            "state": "failed",
            "last_connect_attempt_at": now_ms(),
            "connected_at": null,
            "last_error": error,
            "transport": transport,
            "discovered_tool_count": 0,
            "discovered_tools": [],
            "process_handle": null
        });
        if let Ok(mut guard) = self.mcp_runtime.lock() {
            guard.insert(
                server_id.clone(),
                McpRuntimeEntry {
                    row: runtime_row.clone(),
                    session: None,
                },
            );
        }
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({
                "ok": false,
                "outcome": "failed",
                "server_id": server_id,
                "owner_service": "mcp-runtime",
                "source": "capability_mesh",
                "brain_forwarded": false,
                "runtime_state": "configured_not_connected",
                "state": "failed",
                "protocol_session_implemented": protocol_session_implemented,
                "not_implemented": !protocol_session_implemented,
                "last_connect_attempt_at": runtime_row["last_connect_attempt_at"],
                "connected_at": null,
                "last_error": runtime_row["last_error"],
                "error": runtime_row["last_error"],
                "routes": {
                    "status": "/api/ui/mcp/status",
                    "tools": "/api/ui/mcp/tools"
                }
            }),
        )
    }

    pub fn mcp_call_tool(&self, body: Value) -> (StatusCode, Value) {
        let server_id = string_field(&body, &["server_id", "serverId"]).unwrap_or_default();
        let tool_name = string_field(&body, &["tool_name", "toolName", "name"]).unwrap_or_default();
        if server_id.is_empty() || tool_name.is_empty() {
            return bad_request(
                "mcp_server_id_and_tool_name_required",
                json!({"server_id": server_id, "tool_name": tool_name}),
            );
        }
        let approval_granted = body
            .get("approval")
            .and_then(|approval| approval.get("granted"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !approval_granted {
            return (
                StatusCode::FORBIDDEN,
                json!({
                    "ok": false,
                    "outcome": "blocked",
                    "executed": false,
                    "approval_required": true,
                    "server_id": server_id,
                    "tool_name": tool_name,
                    "owner_service": "mcp-runtime",
                    "source": "capability_mesh",
                    "brain_forwarded": false,
                    "last_error": {"code": "mcp_tool_call_approval_required", "message": "MCP tool calls require explicit approval"},
                    "error": {"code": "mcp_tool_call_approval_required", "message": "MCP tool calls require explicit approval"}
                }),
            );
        }
        let arguments = body.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let mut guard = match self.mcp_runtime.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({"ok": false, "outcome": "failed", "executed": false, "last_error": {"code": "mcp_runtime_lock_poisoned"}}),
                );
            }
        };
        let Some(entry) = guard.get_mut(&server_id) else {
            return mcp_session_not_connected(server_id, tool_name);
        };
        if entry.row["state"] != "connected" {
            return mcp_session_not_connected(server_id, tool_name);
        }
        let Some(session) = entry.session.as_mut() else {
            return mcp_session_not_connected(server_id, tool_name);
        };
        let tool_exists = entry
            .row
            .get("discovered_tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|tool| tool.get("name").and_then(Value::as_str) == Some(tool_name.as_str()));
        if !tool_exists {
            return (
                StatusCode::NOT_FOUND,
                json!({
                    "ok": false,
                    "outcome": "unavailable",
                    "executed": false,
                    "server_id": server_id,
                    "tool_name": tool_name,
                    "owner_service": "mcp-runtime",
                    "source": "capability_mesh",
                    "brain_forwarded": false,
                    "last_error": {"code": "mcp_tool_not_discovered", "message": "MCP tool was not discovered on the connected session"}
                }),
            );
        }
        let started = now_ms();
        let trace = vec![
            json!({"phase": "approval_check", "ok": true, "detail": {"approval_required": true}}),
            json!({"phase": "session_reconciliation", "ok": true, "detail": {"state": "connected"}}),
            json!({"phase": "protocol_call", "ok": true, "detail": {"method": "tools/call"}}),
        ];
        match session.request(
            "tools/call",
            json!({"name": tool_name, "arguments": arguments}),
        ) {
            Ok(result) => {
                let certificate_id = format!("mcp-exec:{server_id}:{tool_name}:{}", now_ms());
                (
                    StatusCode::OK,
                    json!({
                        "ok": true,
                        "outcome": "executed",
                        "executed": true,
                        "server_id": server_id,
                        "tool_name": tool_name,
                        "owner_service": "mcp-runtime",
                        "source": "capability_mesh",
                        "brain_forwarded": false,
                        "result": result,
                        "trace": trace,
                        "execution_certificate": {
                            "certificate_id": certificate_id,
                            "owner_service": "mcp-runtime",
                            "protocol_method": "tools/call",
                            "server_id": server_id,
                            "tool_name": tool_name,
                            "started_at": started,
                            "completed_at": now_ms(),
                            "approval": body.get("approval").cloned().unwrap_or_else(|| json!({}))
                        }
                    }),
                )
            }
            Err(error) => {
                entry.row["state"] = json!("failed");
                entry.row["last_error"] = error.clone();
                entry.session = None;
                (
                    StatusCode::BAD_GATEWAY,
                    json!({
                        "ok": false,
                        "outcome": "failed",
                        "executed": false,
                        "server_id": server_id,
                        "tool_name": tool_name,
                        "owner_service": "mcp-runtime",
                        "source": "capability_mesh",
                        "brain_forwarded": false,
                        "last_error": error,
                        "trace": trace
                    }),
                )
            }
        }
    }

    pub fn mcp_lifecycle_mutation_blocked(
        &self,
        action: &str,
        server_id: String,
    ) -> (StatusCode, Value) {
        let enabled = match action {
            "enable" => true,
            "disable" => false,
            _ => {
                return bad_request(
                    "mcp_lifecycle_unknown_action",
                    json!({"server_id": server_id, "action": action}),
                );
            }
        };
        let Some(path) = self.mcp_config_path_for_id(&server_id) else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                json!({
                    "ok": false,
                    "artifact_kind": "mcp_lifecycle_mutation_result_v1",
                    "outcome": "unavailable",
                    "server_id": server_id,
                    "owner_service": "mcp-runtime",
                    "source": "capability_mesh",
                    "brain_forwarded": false,
                    "runtime_state": "unconfigured",
                    "state": "unconfigured",
                    "action": action,
                    "applied": false,
                    "last_error": {"code": "mcp_server_not_configured", "message": "MCP server is not configured"},
                    "error": {"code": "mcp_server_not_configured", "message": "MCP server is not configured"}
                }),
            );
        };
        let mut raw = std::fs::read_to_string(&path)
            .ok()
            .and_then(|content| serde_json::from_str::<Value>(&content).ok())
            .unwrap_or_else(|| json!({}));
        raw["enabled"] = json!(enabled);
        let serialized = match serde_json::to_string_pretty(&raw) {
            Ok(serialized) => serialized,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({
                        "ok": false,
                        "artifact_kind": "mcp_lifecycle_mutation_result_v1",
                        "outcome": "failed",
                        "server_id": server_id,
                        "owner_service": "mcp-runtime",
                        "source": "capability_mesh",
                        "brain_forwarded": false,
                        "action": action,
                        "applied": false,
                        "last_error": {"code": "mcp_config_serialize_failed", "message": error.to_string()}
                    }),
                );
            }
        };
        if let Err(error) = std::fs::write(&path, serialized) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({
                    "ok": false,
                    "artifact_kind": "mcp_lifecycle_mutation_result_v1",
                    "outcome": "failed",
                    "server_id": server_id,
                    "owner_service": "mcp-runtime",
                    "source": "capability_mesh",
                    "brain_forwarded": false,
                    "action": action,
                    "applied": false,
                    "last_error": {"code": "mcp_config_write_failed", "message": error.to_string()}
                }),
            );
        }
        if !enabled {
            if let Ok(mut guard) = self.mcp_runtime.lock() {
                guard.remove(&server_id);
            }
        }
        let server = self.normalized_mcp_row(&path);
        (
            StatusCode::OK,
            json!({
                "ok": true,
                "artifact_kind": "mcp_lifecycle_mutation_result_v1",
                "outcome": "executed",
                "server_id": server_id,
                "owner_service": "mcp-runtime",
                "source": "capability_mesh",
                "brain_forwarded": false,
                "runtime_state": if enabled { "configured_not_connected" } else { "disabled" },
                "state": if enabled { "configured" } else { "disabled" },
                "action": action,
                "applied": true,
                "server": server,
                "routes": {
                    "status": "/api/ui/mcp/status",
                    "tools": "/api/ui/mcp/tools"
                }
            }),
        )
    }

    fn mcp_config_root(&self) -> PathBuf {
        self.mcp_root
            .clone()
            .unwrap_or_else(|| hom_local_dir().join("mcp"))
    }

    fn mcp_config_path_for_id(&self, server_id: &str) -> Option<PathBuf> {
        let root = self.mcp_config_root();
        let entries = std::fs::read_dir(root).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            let is_mcp = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "json")
                || path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".mcp.json"));
            if !is_mcp {
                continue;
            }
            let fallback_id = stable_id_from_path(&path);
            let raw = std::fs::read_to_string(&path)
                .ok()
                .and_then(|content| serde_json::from_str::<Value>(&content).ok())
                .unwrap_or_else(|| json!({}));
            let id = raw
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(|value| value.trim().to_string())
                .unwrap_or(fallback_id);
            if id == server_id {
                return Some(path);
            }
        }
        None
    }

    fn start_stdio_mcp_session(&self, server: &Value) -> Result<(StdioMcpSession, Value), Value> {
        let command = server
            .get("command")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| json!({"code": "mcp_command_missing", "message": "MCP stdio command is missing"}))?;
        let args = server
            .get("args")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(
                |error| json!({"code": "mcp_process_spawn_failed", "message": error.to_string()}),
            )?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| json!({"code": "mcp_process_stdin_unavailable", "message": "MCP process stdin pipe was unavailable"}))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| json!({"code": "mcp_process_stdout_unavailable", "message": "MCP process stdout pipe was unavailable"}))?;
        let pid = child.id();
        let mut session = StdioMcpSession {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };
        let initialize = session
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "hom-ingress", "version": env!("CARGO_PKG_VERSION")}
                }),
            )
            .map_err(|error| json!({"code": "mcp_initialize_failed", "message": "MCP initialize negotiation failed", "detail": error}))?;
        let tools_result = session
            .request("tools/list", json!({}))
            .map_err(|error| json!({"code": "mcp_tools_list_failed", "message": "MCP tools/list failed after initialize", "detail": error}))?;
        let discovered_tools = tools_result
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let timestamp = now_ms();
        let runtime_row = json!({
            "state": "connected",
            "last_connect_attempt_at": timestamp,
            "connected_at": timestamp,
            "last_error": null,
            "transport": "stdio",
            "protocol": initialize,
            "discovered_tools": discovered_tools,
            "discovered_tool_count": discovered_tools.len(),
            "process_handle": format!("pid:{pid}")
        });
        Ok((session, runtime_row))
    }

    pub fn benchmark_scenarios(&self) -> Vec<Value> {
        let tools = tool_descriptors();
        let mcp_available = !self.mcp_rows().is_empty();
        let mut scenarios = tools
            .into_iter()
            .map(|tool| {
                json!({
                    "benchmark_kind": "tool_use",
                    "subject_id": tool.id,
                    "subject_kind": "tool",
                    "scenario": "tool descriptor completeness",
                    "expected_contract": {
                        "permission_domain": true,
                        "trace_bounded": true,
                        "descriptor_available": true
                    },
                    "actual_contract": {
                        "permission_domain": !tool.permission_domain.is_empty(),
                        "trace_bounded": true,
                        "descriptor_available": tool.state != "missing"
                    }
                })
            })
            .collect::<Vec<_>>();
        scenarios.push(json!({
            "benchmark_kind": "mcp_protocol",
            "subject_id": "mcp.registry.list",
            "subject_kind": "registry",
            "scenario": "mcp config registry availability",
            "expected_contract": {"config_registry_available": true},
            "actual_contract": {"config_registry_available": mcp_available}
        }));
        scenarios
    }

    pub async fn gate_grants(&self, brain: &BrainClient) -> (StatusCode, Value) {
        match brain
            .call(
                "permissions.grants.list",
                json!({"state": "active"}),
                "system",
            )
            .await
        {
            Ok(result) => {
                let grants = result
                    .get("grants")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                (
                    StatusCode::OK,
                    json!({"ok": true, "items": grants, "total": grants.len(), "source": "brain"}),
                )
            }
            Err(error) => brain_error_response(error),
        }
    }

    pub async fn create_gate_grant(&self, brain: &BrainClient, body: Value) -> (StatusCode, Value) {
        let domain = body
            .get("domain")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if domain.is_empty() {
            return bad_request("domain_required", json!({}));
        }
        let params = json!({
            "grant_kind": domain,
            "subject_id": body.get("subject_id").or_else(|| body.get("subjectId")).and_then(Value::as_str).unwrap_or("user"),
            "grantee_id": body.get("grantee_id").or_else(|| body.get("granteeId")).and_then(Value::as_str).unwrap_or("hom-local"),
            "capability_id": body.get("capability_id").or_else(|| body.get("capabilityId")).and_then(Value::as_str).unwrap_or(domain),
            "scope": body.get("scope").cloned().unwrap_or_else(|| json!({})),
            "reason": body.get("reason").and_then(Value::as_str).unwrap_or("created from capability mesh"),
            "confirmed": body.get("confirmed").and_then(Value::as_bool).unwrap_or(true)
        });
        match brain
            .call("permissions.grants.create", params, "system")
            .await
        {
            Ok(result) => (
                StatusCode::CREATED,
                json!({"ok": true, "status": result.get("grant").cloned().unwrap_or(result)}),
            ),
            Err(error) => brain_error_response(error),
        }
    }

    pub async fn chat(&self, brain: &BrainClient, body: Value) -> (StatusCode, Value) {
        let provider_id = match string_field(&body, &["provider_id", "providerId"]) {
            Some(value) => value,
            None => return bad_request("provider_id_required", json!({})),
        };
        let model = match string_field(&body, &["model", "model_id", "modelId"]) {
            Some(value) => value,
            None => return bad_request("model_required", json!({})),
        };
        let mut messages = match messages_from_body(&body) {
            Some(messages) => messages,
            None => return bad_request("messages_required", json!({})),
        };
        let selected_skills = skill_names_from_body(&body);
        let started = std::time::Instant::now();
        let session_id = string_field(&body, &["session_id", "sessionId"])
            .unwrap_or_else(|| format!("chat-{}", now_ms()));
        let reasoning_effort = string_field(&body, &["reasoning_effort", "reasoningEffort"])
            .unwrap_or_else(|| "medium".to_string());
        let mut tool_traces = Vec::new();

        let recall_preflight = recall_preflight(&messages);
        if let Some(preflight) = &recall_preflight {
            let execution = self
                .execute_tool(
                    brain,
                    "brain.memory.recall",
                    json!({
                        "query": preflight.query,
                        "limit": 5,
                        "current_session_source": "chat",
                        "operation": "memory.recall",
                        "temporal_hint": preflight.temporal_hint
                    }),
                    "chat.autorecall",
                    "orchestrator",
                    None,
                )
                .await;
            tool_traces.push(execution.trace_summary());
            if execution.ok {
                if let Some(result) = execution.result {
                    messages.insert(
                        0,
                        json!({
                            "role": "system",
                            "content": format!("HOM brain memory recall trace succeeded. Use this packed evidence before answering; cite open handles when useful and do not invent memories outside it:\n{}", result)
                        }),
                    );
                }
            } else {
                messages.insert(
                    0,
                    json!({
                        "role": "system",
                        "content": format!("HOM brain memory recall was attempted but did not succeed. Do not claim memory recall succeeded. Trace:\n{}", execution.clone_for_prompt())
                    }),
                );
            }
        }

        if let Some(system_prompt) = body.get("system_prompt").and_then(Value::as_str) {
            if !system_prompt.trim().is_empty() {
                messages.insert(0, json!({"role": "system", "content": system_prompt}));
            }
        }
        if !selected_skills.is_empty() {
            if let Some(skill_context) = self.skill_instruction(&selected_skills) {
                messages.insert(0, json!({"role": "system", "content": skill_context}));
            }
        }
        let permission_policy = body
            .get("permission_policy")
            .or_else(|| body.get("permissionPolicy"))
            .cloned()
            .unwrap_or_else(|| json!({
                "profile": "sandbox",
                "policy": {
                    "profile": "sandbox",
                    "network_enabled": false,
                    "provider_key_access": false,
                    "automation_enabled": false,
                    "shell_access": false,
                    "enforced_by": "permission-sandbox-service"
                },
                "source": "permission-sandbox-service",
                "reason": "Permission policy was not injected; defaulting to sandbox for model context."
            }));
        messages.insert(
            0,
            json!({"role": "system", "content": permission_policy_instruction(&permission_policy)}),
        );
        messages.insert(
            0,
            json!({"role": "system", "content": self.tool_instruction()}),
        );

        let mut response = self
            .dispatch_provider_chat(&provider_id, &model, &messages, &body)
            .await;
        for _ in 0..MAX_TOOL_ITERATIONS {
            let content = match response {
                Ok(ref content) => content.clone(),
                Err(_) => break,
            };
            let Some(tool_call) = strict_tool_call(&content) else {
                break;
            };
            let execution = self
                .execute_tool(
                    brain,
                    &tool_call.tool_id,
                    tool_call.arguments,
                    "chat.tool_loop",
                    "model",
                    tool_call.descriptor_hash.as_deref(),
                )
                .await;
            tool_traces.push(execution.trace_summary());
            if execution.is_descriptor_integrity_error() {
                return (
                    execution.status,
                    json!({
                        "ok": false,
                        "error": execution.error.unwrap_or_else(|| json!({"code": -32602, "message": "descriptor_integrity_rejected"})),
                        "tool_traces": tool_traces,
                        "gate": "descriptor_integrity_gate_v1",
                        "source": "capability_mesh"
                    }),
                );
            }
            messages.push(json!({"role": "assistant", "content": content}));
            messages.push(json!({"role": "user", "content": format!("HOM tool result:\n{}", execution.clone_for_prompt())}));
            response = self
                .dispatch_provider_chat(&provider_id, &model, &messages, &body)
                .await;
        }

        let content = match response {
            Ok(content) => content,
            Err((status, error)) => return (status, error),
        };
        if strict_tool_call(&content).is_some() {
            return (
                StatusCode::BAD_GATEWAY,
                json!({
                    "ok": false,
                    "error": {
                        "code": -32091,
                        "message": "agent_tool_iteration_limit_reached",
                        "data": {"provider_id": provider_id, "model": model, "tool_traces": tool_traces}
                    }
                }),
            );
        }
        let answer_gate = answer_gate(&content, &tool_traces, recall_preflight.as_ref());
        if !answer_gate
            .get("allowed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return (
                StatusCode::BAD_GATEWAY,
                json!({
                    "ok": false,
                    "error": {
                        "code": -32091,
                        "message": "answer_gate_rejected",
                        "data": {
                            "provider_id": provider_id,
                            "model": model,
                            "answer_gate": answer_gate,
                            "tool_traces": tool_traces
                        }
                    }
                }),
            );
        }

        let latency_ms = started.elapsed().as_millis() as u64;
        let _ = self
            .record_action(
                brain,
                "chat.complete",
                json!({
                    "provider_id": provider_id,
                    "model": model,
                    "session_id": session_id,
                    "reasoning_effort": reasoning_effort,
                    "latency_ms": latency_ms,
                    "permission_policy": permission_policy.clone(),
                    "tool_traces": tool_traces
                }),
            )
            .await;
        (
            StatusCode::OK,
            json!({
                "ok": true,
                "session_id": session_id,
                "content": content,
                "answer_text": content,
                "latency_ms": latency_ms,
                "model": model,
                "provider_id": provider_id,
                "reasoning_effort": reasoning_effort,
                "provider_route": {
                    "provider_id": provider_id,
                    "model_id": model,
                    "routeable": true,
                    "source": "capability_mesh.selected_provider_model",
                    "secret_material_returned": false
                },
                "permission_policy": permission_policy,
                "tool_catalog": {
                    "source": "capability_mesh",
                    "prompt_injected": true,
                    "total": self.tool_rows().into_iter().filter(|tool| tool["enabled"].as_bool() != Some(false)).count(),
                    "fields": ["id", "category", "mutation_level", "approval_required", "enabled", "owner_service", "descriptor_hash"]
                },
                "tool_traces": tool_traces,
                "answer_gate": answer_gate,
                "source": "capability_mesh"
            }),
        )
    }

    async fn provider_rows(&self) -> Vec<Value> {
        let mut providers = provider_catalog::provider_catalog()
            .iter()
            .map(|entry| self.catalog_provider_row(entry, None, None))
            .collect::<Vec<_>>();

        providers.push(self.openai_compat_alias_row());

        if let Some(provider) = self.probe_ollama().await {
            replace_provider_row(&mut providers, provider);
        }
        if let Some(provider) = self.probe_lm_studio().await {
            replace_provider_row(&mut providers, provider);
        }
        if let Some(provider) = self.probe_codex_oauth_runtime().await {
            replace_provider_row(&mut providers, provider);
        }
        providers
    }

    fn catalog_provider_row(
        &self,
        entry: &ProviderCatalogEntry,
        models_override: Option<Vec<Value>>,
        state_override: Option<&str>,
    ) -> Value {
        let snapshot = if models_override.is_none() {
            stored_snapshot_model_rows(entry.id)
        } else {
            None
        };
        let snapshot_meta = snapshot.as_ref().map(|(_, meta)| meta.clone());
        let models = models_override.unwrap_or_else(|| {
            snapshot
                .map(|(models, _)| models)
                .unwrap_or_else(|| seed_model_rows(entry))
        });
        let registry_source = snapshot_meta
            .as_ref()
            .and_then(|meta| meta.get("source"))
            .and_then(Value::as_str)
            .unwrap_or("static_seed");
        let credential_state = credential_state(entry, None);
        let state = state_override.unwrap_or_else(|| {
            if entry.credential_required && credential_state == "missing" {
                "setup_required"
            } else if entry.id == "custom" {
                "setup_required"
            } else {
                "catalog_ready"
            }
        });
        let model_count = models.len();
        let model_ids = models
            .iter()
            .filter_map(|model| model.get("id").and_then(Value::as_str).map(str::to_string))
            .collect::<Vec<_>>();
        let routeable = match entry.kind {
            ProviderKind::Local => model_count > 0 && state == "available",
            ProviderKind::OAuth => state == "available" && model_count > 0,
            ProviderKind::ApiKey => credential_state != "missing" && model_count > 0,
            ProviderKind::Custom => false,
        };
        json!({
            "id": entry.id,
            "provider_id": entry.id,
            "name": entry.label,
            "label": entry.label,
            "type": provider_kind_name(entry.kind),
            "kind": provider_kind_name(entry.kind),
            "state": state,
            "aggregate_state": state,
            "catalog_visible": true,
            "catalog_ready": true,
            "pipeline_ready": true,
            "connection_state": state,
            "model_registry_state": if registry_source == "live_provider_catalog" { "live_snapshot" } else if model_count > 0 { "seeded" } else { "empty_seed" },
            "model_registry_source": registry_source,
            "model_registry_snapshot": snapshot_meta,
            "connection_kind": provider_kind_name(entry.kind),
            "models_available": model_count,
            "models": models,
            "routeable": routeable,
            "model_selector_eligible": true,
            "enabled": true,
            "available": routeable,
            "credential_required": entry.credential_required,
            "credential_state": credential_state,
            "setup_requirements": {
                "api_key_required": entry.credential_required,
                "endpoint_required": entry.id == "custom",
                "oauth_required": matches!(entry.kind, ProviderKind::OAuth),
                "local_runtime_required": matches!(entry.kind, ProviderKind::Local),
                "save_button_rule": "ui_disables_save_until_required_fields_are_present"
            },
            "endpoint": {
                "default_base_url": entry.default_base_url,
                "base_url_env": entry.base_url_env,
                "override_supported": entry.endpoint_override_supported
            },
            "wire_modes": entry.supported_wire_modes.iter().map(|mode| wire_mode_name(*mode)).collect::<Vec<_>>(),
            "default_wire_mode": wire_mode_name(entry.default_wire_mode),
            "model_discovery_supported": entry.model_discovery_supported,
            "allows_custom_model_id": entry.allows_custom_model_id,
            "health": {
                "status": state,
                "model_count": model_count,
                "models": model_ids,
                "credential_state": credential_state
            },
            "auth_contract": {
                "provider_id": entry.id,
                "auth_method": provider_kind_name(entry.kind),
                "reference_kind": if matches!(entry.kind, ProviderKind::Local) { "local_endpoint" } else { "credential_ref" },
                "storage_authority": if matches!(entry.kind, ProviderKind::Local) { "local_endpoint_config" } else { "keychain_or_env" },
                "secret_material_accepted": false,
                "ui_required_field_gate": true
            },
            "base_url": entry.default_base_url
        })
    }

    fn openai_compat_alias_row(&self) -> Value {
        let entry =
            provider_catalog::find_provider("openai").expect("openai provider catalog entry");
        let mut row = self.catalog_provider_row(entry, None, None);
        row["id"] = json!("openai-compat");
        row["provider_id"] = json!("openai-compat");
        row["name"] = json!("OpenAI Compatible");
        row["label"] = json!("OpenAI Compatible");
        row["auth_contract"]["provider_id"] = json!("openai-compat");
        row
    }

    async fn probe_ollama(&self) -> Option<Value> {
        let base_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
        let response = self
            .http
            .get(format!("{}/api/tags", base_url.trim_end_matches('/')))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let payload: Value = response.json().await.ok()?;
        let models = payload
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|model| {
                let id = model
                    .get("name")
                    .or_else(|| model.get("model"))
                    .and_then(Value::as_str)?;
                Some(model_row("ollama", id, "local", true))
            })
            .collect::<Vec<_>>();
        Some(provider_row(
            "ollama",
            "Ollama",
            "local",
            "available",
            "local_endpoint",
            models,
            Some(base_url),
        ))
    }

    async fn probe_lm_studio(&self) -> Option<Value> {
        let base_url = self.lm_studio_base_url();
        let response = self
            .http
            .get(format!("{}/models", base_url.trim_end_matches('/')))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let payload: Value = response.json().await.ok()?;
        let models = payload
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|model| model.get("id").and_then(Value::as_str))
            .map(|id| model_row("lm-studio", id, "local", true))
            .collect::<Vec<_>>();
        Some(provider_row(
            "lm-studio",
            "LM Studio",
            "local",
            "available",
            "local_endpoint",
            models,
            Some(base_url),
        ))
    }

    async fn probe_codex_oauth_runtime(&self) -> Option<Value> {
        let base_url = self.codex_oauth_runtime_url();
        let response = self
            .http
            .get(format!("{}/provider", base_url.trim_end_matches('/')))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let payload: Value = response.json().await.ok()?;
        let status = payload
            .get("status")
            .cloned()
            .unwrap_or_else(|| payload.clone());
        let runtime_state = status
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("discovered");
        let models = self.codex_oauth_runtime_models().await.unwrap_or_default();
        let state = if !models.is_empty() {
            "available"
        } else {
            runtime_state
        };
        let mut row = provider_row(
            "codex-oauth",
            "Codex OAuth",
            "oauth",
            state,
            "openai-compatible-oauth",
            models,
            Some(base_url),
        );
        row["runtime"] = status.clone();
        row["credential_state"] = json!(runtime_state);
        row["health"]["credential_state"] = json!(runtime_state);
        if let Some(last_probe_ts) = status.get("last_probe_ts").cloned() {
            row["health"]["last_probe_ts"] = last_probe_ts;
        }
        row.get_mut("auth_contract")
            .and_then(Value::as_object_mut)
            .map(|contract| {
                contract.insert(
                    "accepted_ref_prefix".to_string(),
                    json!("hom.provider.codex-oauth.oauth"),
                );
                contract.insert("requires_native_app_flow".to_string(), json!(true));
                contract.insert("app_owned".to_string(), json!(true));
            });
        Some(row)
    }

    async fn codex_oauth_runtime_models(&self) -> Option<Vec<Value>> {
        let base_url = self.codex_oauth_runtime_url();
        let response = self
            .http
            .get(format!("{}/models", base_url.trim_end_matches('/')))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let payload: Value = response.json().await.ok()?;
        let raw_models = payload
            .get("items")
            .or_else(|| payload.get("models"))
            .and_then(Value::as_array)?;
        Some(
            raw_models
                .iter()
                .filter_map(|model| {
                    let id = model
                        .get("id")
                        .or_else(|| model.get("model_id"))
                        .or_else(|| model.get("name"))
                        .and_then(Value::as_str)?;
                    let execution_location = model
                        .get("execution_location")
                        .and_then(Value::as_str)
                        .unwrap_or("cloud");
                    Some(model_row("codex-oauth", id, execution_location, true))
                })
                .collect(),
        )
    }

    fn tool_rows(&self) -> Vec<Value> {
        tool_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.to_value())
            .collect()
    }

    fn skill_rows(&self) -> Vec<Value> {
        self.skill_roots
            .as_ref()
            .map(|roots| skill_rows_from_roots(roots))
            .unwrap_or_else(scan_skill_roots)
    }

    fn skill_instruction(&self, selected_names: &[String]) -> Option<String> {
        let selected = selected_names
            .iter()
            .map(|name| name.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        let mut chunks = Vec::new();
        for skill in self.skill_rows() {
            let Some(name) = skill["name"].as_str() else {
                continue;
            };
            if !selected.contains(&name.to_ascii_lowercase()) {
                continue;
            }
            let Some(source_path) = skill["source_path"].as_str() else {
                continue;
            };
            let descriptor = std::fs::read_to_string(source_path).unwrap_or_default();
            if descriptor.trim().is_empty() {
                continue;
            }
            let clipped = descriptor.chars().take(12_000).collect::<String>();
            chunks.push(format!(
                "# Skill: {name}
Source: {source_path}
{clipped}"
            ));
        }
        if chunks.is_empty() {
            return None;
        }
        Some(format!(
            "Selected HOM skills are attached below as real local SKILL.md descriptors. Follow them when relevant, but do not claim a separate session import occurred. Too many always-on skills consume context.

{}",
            chunks.join("

---

")
        ))
    }

    fn plugin_rows(&self) -> Vec<Value> {
        let mut rows = scan_manifest_root(
            hom_local_dir().join("plugins"),
            "plugin",
            "plugin.json",
            "plugin manifest",
        )
        .into_iter()
        .map(|mut row| {
            if row.get("version").is_none() {
                row["version"] = json!("unknown");
            }
            row["lifecycle_source"] = json!("manifest_scan");
            row
        })
        .collect::<Vec<_>>();
        let store = self.read_plugin_store();
        if let Some(plugins) = store.get("plugins").and_then(Value::as_object) {
            for plugin in plugins.values() {
                rows.retain(|row| row["id"] != plugin["id"]);
                rows.push(plugin.clone());
            }
        }
        rows.sort_by(|left, right| {
            left["id"]
                .as_str()
                .unwrap_or_default()
                .cmp(right["id"].as_str().unwrap_or_default())
        });
        rows
    }

    fn plugin_store_path(&self) -> PathBuf {
        self.plugin_store_path
            .clone()
            .unwrap_or_else(|| hom_local_dir().join("runtime/plugins-store.json"))
    }

    fn empty_plugin_store() -> Value {
        json!({
            "artifact_kind": "plugin_store_v1",
            "schema_version": 1,
            "plugins": {},
            "uninstalled": []
        })
    }

    fn read_plugin_store(&self) -> Value {
        let path = self.plugin_store_path();
        let Ok(content) = std::fs::read_to_string(path) else {
            return Self::empty_plugin_store();
        };
        let Ok(value) = serde_json::from_str::<Value>(&content) else {
            return Self::empty_plugin_store();
        };
        if value["artifact_kind"].as_str() != Some("plugin_store_v1")
            || value["schema_version"].as_u64() != Some(1)
        {
            return Self::empty_plugin_store();
        }
        value
    }

    fn plugin_store_read_metadata(&self) -> Value {
        let path = self.plugin_store_path();
        let state = if !path.exists() {
            "missing"
        } else if self.read_plugin_store()["artifact_kind"].as_str() == Some("plugin_store_v1") {
            "readable"
        } else {
            "invalid"
        };
        json!({
            "state": state,
            "path": path.display().to_string(),
            "artifact_kind": "plugin_store_v1",
            "schema_version": 1
        })
    }

    fn write_plugin_store(&self, store: &Value) -> Result<(), (StatusCode, Value)> {
        let path = self.plugin_store_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                error_payload(
                    StatusCode::BAD_REQUEST,
                    -32050,
                    "plugin_store_write_failed",
                    json!({"path": parent.display().to_string(), "error": error.to_string()}),
                )
            })?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(store).unwrap_or_default()).map_err(
            |error| {
                error_payload(
                    StatusCode::BAD_REQUEST,
                    -32050,
                    "plugin_store_write_failed",
                    json!({"path": tmp.display().to_string(), "error": error.to_string()}),
                )
            },
        )?;
        std::fs::rename(&tmp, &path).map_err(|error| {
            error_payload(
                StatusCode::BAD_REQUEST,
                -32050,
                "plugin_store_write_failed",
                json!({"path": path.display().to_string(), "error": error.to_string()}),
            )
        })
    }

    fn apply_plugin_lifecycle(
        &self,
        action: &str,
        body: Value,
    ) -> Result<Value, (StatusCode, Value)> {
        let mut store = self.read_plugin_store();
        if store.get("plugins").and_then(Value::as_object).is_none() {
            store["plugins"] = json!({});
        }
        if store.get("uninstalled").and_then(Value::as_array).is_none() {
            store["uninstalled"] = json!([]);
        }
        match action {
            "install" => {
                let id = required_plugin_id(&body)?;
                let plugin = normalize_plugin_install_record(&body, &id)?;
                store["plugins"][&id] = plugin.clone();
                self.write_plugin_store(&store)?;
                Ok(json!({
                    "ok": true,
                    "state": "installed",
                    "action": "install",
                    "plugin": plugin,
                    "persistence": self.plugin_store_read_metadata(),
                    "source": "capability_mesh"
                }))
            }
            "enable" | "disable" => {
                let id = required_plugin_ref(&body)?;
                let Some(existing) = store
                    .get_mut("plugins")
                    .and_then(Value::as_object_mut)
                    .and_then(|plugins| plugins.get_mut(&id))
                else {
                    return Err(error_payload(
                        StatusCode::NOT_FOUND,
                        -32044,
                        "plugin_not_installed",
                        json!({"plugin_id": id}),
                    ));
                };
                let enabled = action == "enable";
                existing["enabled"] = json!(enabled);
                existing["state"] = json!(if enabled { "installed" } else { "disabled" });
                existing["updated_at"] = json!(now_ms());
                let plugin = existing.clone();
                self.write_plugin_store(&store)?;
                Ok(json!({
                    "ok": true,
                    "state": if enabled { "enabled" } else { "disabled" },
                    "action": action,
                    "plugin": plugin,
                    "persistence": self.plugin_store_read_metadata(),
                    "source": "capability_mesh"
                }))
            }
            "uninstall" => {
                let id = required_plugin_ref(&body)?;
                let Some(removed) = store
                    .get_mut("plugins")
                    .and_then(Value::as_object_mut)
                    .and_then(|plugins| plugins.remove(&id))
                else {
                    return Err(error_payload(
                        StatusCode::NOT_FOUND,
                        -32044,
                        "plugin_not_installed",
                        json!({"plugin_id": id}),
                    ));
                };
                if let Some(uninstalled) =
                    store.get_mut("uninstalled").and_then(Value::as_array_mut)
                {
                    uninstalled.push(json!({
                        "plugin_id": id,
                        "uninstalled_at": now_ms()
                    }));
                }
                self.write_plugin_store(&store)?;
                Ok(json!({
                    "ok": true,
                    "state": "uninstalled",
                    "action": "uninstall",
                    "uninstalled_plugin_id": id,
                    "removed_plugin": removed,
                    "persistence": self.plugin_store_read_metadata(),
                    "source": "capability_mesh"
                }))
            }
            _ => Err(error_payload(
                StatusCode::BAD_REQUEST,
                -32602,
                "unsupported_plugin_lifecycle_action",
                json!({"action": action}),
            )),
        }
    }

    fn reconcile_mcp_runtime(&self) {
        let Ok(mut guard) = self.mcp_runtime.lock() else {
            return;
        };
        for entry in guard.values_mut() {
            let Some(session) = entry.session.as_mut() else {
                continue;
            };
            match session.child.try_wait() {
                Ok(Some(status)) => {
                    entry.row["state"] = json!("failed");
                    entry.row["connected_at"] = Value::Null;
                    entry.row["last_error"] = json!({
                        "code": "mcp_session_process_exited",
                        "message": "MCP stdio process exited; runtime handle was reconciled as failed",
                        "exit_status": status.to_string()
                    });
                    entry.row["discovered_tools"] = json!([]);
                    entry.row["discovered_tool_count"] = json!(0);
                    entry.row["process_handle"] = Value::Null;
                    entry.session = None;
                }
                Ok(None) => {}
                Err(error) => {
                    entry.row["state"] = json!("failed");
                    entry.row["connected_at"] = Value::Null;
                    entry.row["last_error"] = json!({
                        "code": "mcp_session_reconcile_failed",
                        "message": error.to_string()
                    });
                    entry.row["discovered_tools"] = json!([]);
                    entry.row["discovered_tool_count"] = json!(0);
                    entry.row["process_handle"] = Value::Null;
                    entry.session = None;
                }
            }
        }
    }

    fn mcp_rows(&self) -> Vec<Value> {
        self.reconcile_mcp_runtime();
        let root = self
            .mcp_root
            .clone()
            .unwrap_or_else(|| hom_local_dir().join("mcp"));
        let mut rows = Vec::new();
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                let is_mcp = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext == "json")
                    || path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.ends_with(".mcp.json"));
                if !is_mcp {
                    continue;
                }
                rows.push(self.normalized_mcp_row(&path));
            }
        }
        rows.sort_by(|left, right| {
            left["id"]
                .as_str()
                .unwrap_or_default()
                .cmp(right["id"].as_str().unwrap_or_default())
        });
        rows
    }

    fn normalized_mcp_row(&self, path: &Path) -> Value {
        let fallback_id = stable_id_from_path(path);
        let raw = std::fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str::<Value>(&content).ok())
            .unwrap_or_else(|| json!({}));
        let id = raw
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_string())
            .unwrap_or(fallback_id);
        let name = raw
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_string())
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("MCP Server")
                    .to_string()
            });
        let url = raw.get("url").and_then(Value::as_str).map(str::trim);
        let command = raw.get("command").and_then(Value::as_str).map(str::trim);
        let has_http = url.is_some_and(|value| !value.is_empty());
        let has_stdio = command.is_some_and(|value| !value.is_empty());
        let runtime = self
            .mcp_runtime
            .lock()
            .ok()
            .and_then(|guard| guard.get(&id).map(|entry| entry.row.clone()));
        let enabled = raw.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        let mut row = json!({
            "id": id,
            "name": name,
            "enabled": enabled,
            "state": "configured",
            "transport": if has_http { "http" } else { "stdio" },
            "command": if has_stdio { json!(command.unwrap()) } else { Value::Null },
            "args": raw.get("args").and_then(Value::as_array).cloned().map(Value::Array).unwrap_or_else(|| json!([])),
            "url": if has_http { json!(url.unwrap()) } else { Value::Null },
            "headers_configured": raw.get("headers").and_then(Value::as_object).is_some_and(|headers| !headers.is_empty()),
            "tools": raw.get("tools").and_then(Value::as_array).cloned().map(Value::Array).unwrap_or_else(|| json!([])),
            "configured_tools": raw.get("tools").and_then(Value::as_array).cloned().map(Value::Array).unwrap_or_else(|| json!([])),
            "discovered_tools": [],
            "discovered_tool_count": 0,
            "source_path": path.display().to_string(),
            "last_connect_attempt_at": null,
            "connected_at": null,
            "last_error": null,
            "process_handle": null,
        });
        if has_http == has_stdio {
            row["state"] = json!("failed");
            row["transport"] = json!("invalid");
            row["last_error"] = json!({
                "code": if has_http { "mcp_config_ambiguous_transport" } else { "mcp_config_missing_transport" },
                "message": if has_http { "MCP config must define either url or command, not both" } else { "MCP config must define url for HTTP or command for stdio" }
            });
        } else if !enabled {
            row["state"] = json!("disabled");
        }
        if let Some(runtime) = runtime {
            row["state"] = runtime["state"].clone();
            row["last_connect_attempt_at"] = runtime["last_connect_attempt_at"].clone();
            row["connected_at"] = runtime["connected_at"].clone();
            row["last_error"] = runtime["last_error"].clone();
            row["discovered_tool_count"] = runtime["discovered_tool_count"].clone();
            row["discovered_tools"] = runtime["discovered_tools"].clone();
            row["process_handle"] = runtime["process_handle"].clone();
        }
        row
    }

    fn tool_instruction(&self) -> String {
        let mut tools = self
            .tool_rows()
            .into_iter()
            .filter(|tool| tool["enabled"].as_bool() != Some(false))
            .map(|tool| {
                json!({
                    "id": tool["id"],
                    "capability_id": tool["capability_id"],
                    "descriptor_version": tool["descriptor_version"],
                    "descriptor_hash": tool["descriptor_hash"],
                    "description": tool["description"],
                    "enabled": tool["enabled"],
                    "category": tool["category"],
                    "mutation_level": tool["mutation_level"],
                    "approval_required": tool["approval_required"],
                    "owner_service": tool["owner_service"],
                    "permission_domain": tool["permission_domain"],
                    "transport": tool["transport"],
                    "retrieval_path": tool["retrieval_path"],
                    "dispatch_target": tool["dispatch_target"],
                    "input_schema": tool["input_schema"],
                    "examples": tool["examples"]
                })
            })
            .collect::<Vec<_>>();

        // Merge discovered MCP tools into the tool instruction so the model
        // can call them via mcp.<server_id>.<tool_name> namespace.
        // Ref: "Model Context Protocol" (Anthropic, 2024) — tool discovery via
        // servers/tools/list on connected sessions.
        let mcp_servers = self.mcp_rows();
        let mcp_tools = mcp_tool_rows(&mcp_servers);
        for mcp_tool in &mcp_tools {
            if mcp_tool["executable"].as_bool() != Some(true) {
                continue;
            }
            let server_id = mcp_tool["server_id"].as_str().unwrap_or("unknown");
            let tool_name = mcp_tool["tool"]["name"].as_str().unwrap_or("unknown");
            let mcp_tool_id = format!("mcp.{}.{}", server_id, tool_name);
            let description = mcp_tool["tool"]["description"].as_str().unwrap_or("");
            let input_schema = mcp_tool["tool"]
                .get("inputSchema")
                .or_else(|| mcp_tool["tool"].get("input_schema"))
                .cloned()
                .unwrap_or(json!({}));
            tools.push(json!({
                "id": mcp_tool_id,
                "capability_id": format!("mcp.{}", server_id),
                "descriptor_version": "mcp-v1",
                "descriptor_hash": sha256_hex(&format!("{}:{}", mcp_tool_id, description)),
                "description": description,
                "enabled": true,
                "category": "mcp",
                "mutation_level": "none",
                "approval_required": false,
                "owner_service": "mcp-runtime",
                "permission_domain": "mcp",
                "transport": "mcp",
                "retrieval_path": Value::Null,
                "dispatch_target": "mcp_runtime",
                "input_schema": input_schema,
                "examples": []
            }));
        }

        format!(
            "HOM registered tools are permission gated. For memory, previous-session, yesterday, browser, file, shell, plugin, skill, MCP, or web tasks, request the tool now; do not say you will search later. If a tool is needed, reply with exactly one JSON object and no markdown, copying the tool descriptor_hash exactly: {{\"tool_call\":{{\"tool_id\":\"brain.memory.recall\",\"descriptor_hash\":\"<descriptor_hash>\",\"arguments\":{{\"query\":\"...\",\"limit\":5}}}}}}. Final answers are rejected if they claim memory/tool/browser/file/shell usage without a recorded HOM trace. Available tools: {}",
            Value::Array(tools)
        )
    }

    async fn execute_tool(
        &self,
        brain: &BrainClient,
        tool_id: &str,
        args: Value,
        source: &str,
        caller: &str,
        descriptor_hash: Option<&str>,
    ) -> ToolExecution {
        let started_at = std::time::Instant::now();
        let input_summary = summarize_input(&args);

        // MCP namespaced tools (mcp.<server_id>.<tool_name>) are dynamically
        // discovered and not in the static tool_descriptors() list.
        // Ref: "Model Context Protocol" (Anthropic, 2024).
        if tool_id.starts_with("mcp.") {
            let args_obj = args.as_object().cloned().unwrap_or_default();
            let args_value = Value::Object(args_obj);
            let result = self.dispatch_mcp_tool(tool_id, args_value).await;
            let execution = match result {
                Ok(value) => ToolExecution {
                    ok: true,
                    tool_id: tool_id.to_string(),
                    status: StatusCode::OK,
                    input_summary,
                    result: Some(value),
                    error: None,
                    gate: None,
                    trace: vec![],
                    broker: None,
                },
                Err((status, error)) => ToolExecution {
                    ok: false,
                    tool_id: tool_id.to_string(),
                    status,
                    input_summary,
                    result: None,
                    error: Some(error),
                    gate: None,
                    trace: vec![],
                    broker: None,
                },
            };
            let _ = self
                .record_action(
                    brain,
                    if execution.ok {
                        "tool.executed"
                    } else {
                        "tool.failed"
                    },
                    execution.action_payload(source),
                )
                .await;
            return execution;
        }

        let Some(tool) = tool_descriptors()
            .into_iter()
            .find(|tool| tool.id == tool_id)
        else {
            let execution = ToolExecution::error(
                tool_id,
                StatusCode::NOT_FOUND,
                -32044,
                "tool_not_found",
                json!({"tool_id": tool_id}),
                input_summary,
            );
            let _ = self
                .record_action(brain, "tool.failed", execution.action_payload(source))
                .await;
            return execution;
        };
        let args = args.as_object().cloned().unwrap_or_default();
        let args_value = Value::Object(args.clone());
        let expected_descriptor_hash = tool.descriptor_hash();
        let broker = broker_dispatch(
            &tool,
            &args_value,
            source,
            caller,
            &expected_descriptor_hash,
        );
        if caller == "model" {
            match descriptor_hash {
                Some(received) if received == expected_descriptor_hash => {}
                Some(received) => {
                    let execution = ToolExecution {
                        ok: false,
                        tool_id: tool.id.to_string(),
                        status: StatusCode::BAD_REQUEST,
                        input_summary,
                        result: None,
                        error: Some(json!({
                            "code": -32602,
                            "message": "descriptor_hash_mismatch",
                            "data": {
                                "tool_id": tool.id,
                                "expected_descriptor_hash": expected_descriptor_hash,
                                "received_descriptor_hash": received
                            }
                        })),
                        gate: None,
                        trace: vec![trace_event(
                            tool.id,
                            "descriptor_check",
                            false,
                            json!({
                                "expected_descriptor_hash": expected_descriptor_hash,
                                "received_descriptor_hash": received,
                                "policy": "model_tool_calls_require_current_descriptor_hash"
                            }),
                        )],
                        broker: Some(broker),
                    };
                    let _ = self
                        .record_action(brain, "tool.failed", execution.action_payload(source))
                        .await;
                    return execution;
                }
                None => {
                    let execution = ToolExecution {
                        ok: false,
                        tool_id: tool.id.to_string(),
                        status: StatusCode::BAD_REQUEST,
                        input_summary,
                        result: None,
                        error: Some(json!({
                            "code": -32602,
                            "message": "descriptor_hash_required",
                            "data": {
                                "tool_id": tool.id,
                                "expected_descriptor_hash": expected_descriptor_hash
                            }
                        })),
                        gate: None,
                        trace: vec![trace_event(
                            tool.id,
                            "descriptor_check",
                            false,
                            json!({
                                "expected_descriptor_hash": expected_descriptor_hash,
                                "policy": "model_tool_calls_require_current_descriptor_hash"
                            }),
                        )],
                        broker: Some(broker),
                    };
                    let _ = self
                        .record_action(brain, "tool.failed", execution.action_payload(source))
                        .await;
                    return execution;
                }
            }
        }
        let context = permission_context(tool.permission_domain, &args_value);
        let mut trace = vec![trace_event(
            tool.id,
            "descriptor_check",
            true,
            json!({
                "descriptor_hash": expected_descriptor_hash,
                "descriptor_version": tool.descriptor_version,
                "mode": if caller == "model" { "model_supplied" } else { "server_resolved" }
            }),
        )];
        trace.push(trace_event(
            tool.id,
            "broker_dispatch",
            true,
            broker.trace_detail(),
        ));
        trace.push(trace_event(
            tool.id,
            "permission_check",
            false,
            json!({"domain": tool.permission_domain, "context": context}),
        ));
        let gate = match brain
            .call(
                "tool.check_gate",
                json!({"domain": tool.permission_domain, "context": context}),
                "system",
            )
            .await
        {
            Ok(result) => {
                if let Some(permission_trace) = trace.last_mut() {
                    permission_trace["ok"] = json!(true);
                }
                json!({"decision": "allowed", "domain": tool.permission_domain, "brain": result})
            }
            Err(error) => {
                let (_, payload) = brain_error_response(error);
                let mut execution = ToolExecution {
                    ok: false,
                    tool_id: tool.id.to_string(),
                    status: StatusCode::FORBIDDEN,
                    input_summary: input_summary.clone(),
                    result: None,
                    error: payload.get("error").cloned(),
                    gate: Some(json!({"decision": "denied", "domain": tool.permission_domain})),
                    trace,
                    broker: Some(broker),
                };
                if let Err(error) = self
                    .record_tool_observability(
                        brain,
                        &tool,
                        &args_value,
                        &execution,
                        source,
                        started_at.elapsed(),
                    )
                    .await
                {
                    execution.trace.push(observability_trace(&error));
                }
                let _ = self
                    .record_tool_benchmark(brain, &tool, &execution, source, started_at.elapsed())
                    .await;
                let _ = self
                    .record_action(brain, "tool.denied", execution.action_payload(source))
                    .await;
                return execution;
            }
        };

        let result = self.run_tool(brain, &tool, Value::Object(args)).await;
        match result {
            Ok(result) => {
                trace.push(trace_event(tool.id, "execute", true, json!({})));
                trace.push(trace_event(
                    tool.id,
                    "result",
                    true,
                    result_summary(&result),
                ));
                let mut execution = ToolExecution {
                    ok: true,
                    tool_id: tool.id.to_string(),
                    status: StatusCode::OK,
                    input_summary: input_summary.clone(),
                    result: Some(result),
                    error: None,
                    gate: Some(gate),
                    trace,
                    broker: Some(broker),
                };
                if let Err(error) = self
                    .record_tool_observability(
                        brain,
                        &tool,
                        &args_value,
                        &execution,
                        source,
                        started_at.elapsed(),
                    )
                    .await
                {
                    execution.trace.push(observability_trace(&error));
                }
                let _ = self
                    .record_tool_benchmark(brain, &tool, &execution, source, started_at.elapsed())
                    .await;
                let _ = self
                    .record_action(brain, "tool.succeeded", execution.action_payload(source))
                    .await;
                execution
            }
            Err((status, error)) => {
                trace.push(trace_event(tool.id, "execute", false, error.clone()));
                let mut execution = ToolExecution {
                    ok: false,
                    tool_id: tool.id.to_string(),
                    status,
                    input_summary: input_summary.clone(),
                    result: None,
                    error: Some(error),
                    gate: Some(gate),
                    trace,
                    broker: Some(broker),
                };
                if let Err(error) = self
                    .record_tool_observability(
                        brain,
                        &tool,
                        &args_value,
                        &execution,
                        source,
                        started_at.elapsed(),
                    )
                    .await
                {
                    execution.trace.push(observability_trace(&error));
                }
                let _ = self
                    .record_tool_benchmark(brain, &tool, &execution, source, started_at.elapsed())
                    .await;
                let _ = self
                    .record_action(brain, "tool.failed", execution.action_payload(source))
                    .await;
                execution
            }
        }
    }

    async fn run_tool(
        &self,
        brain: &BrainClient,
        tool: &ToolDescriptor,
        args: Value,
    ) -> Result<Value, (StatusCode, Value)> {
        match tool.id {
            "network.check" => self.network_check(args).await,
            "web.fetch_url" => self.fetch_url(args).await,
            "filesystem.read" => read_file_chunk(args),
            "filesystem.list" => list_directory(args),
            "filesystem.search" => search_filesystem(args),
            "shell.propose" => Ok(json!({
                "command": string_field(&args, &["command"]).unwrap_or_default(),
                "reason": string_field(&args, &["reason"]),
                "proposed": true,
                "executed": false,
                "execution_kind": "proposal_only"
            })),
            "shell.execute" => execute_shell_command(args),
            "filesystem.write.preview" => self.preview_file_write(args),
            "filesystem.write.apply" => self.apply_file_write(args),
            "browser.open" => {
                let url = required_string(&args, "url")?;
                run_agent_browser(&["open", url.as_str()])
            }
            "browser.snapshot" => run_agent_browser(&["snapshot", "-i"]),
            "browser.current_page" => run_agent_browser(&["get", "url"]),
            "brain.memory.recall" => {
                brain_tool(brain, "memory.recall", args, "memory:recall").await
            }
            "brain.memory.save" => brain_tool(brain, "memory.save", args, "memory:save").await,
            "brain.reasoning.run" => brain_tool(brain, "reasoning.run", args, "memory:save").await,
            "brain.nightly.tree" => brain_tool(brain, "nightly.tree", args, "system").await,
            "brain.status" => brain_tool(brain, "runtime.status", args, "system").await,
            "brain.recall.smart" => {
                brain_tool(brain, "memory.recall.smart", args, "memory:recall").await
            }
            "brain.recall.detail" => brain_tool(brain, "memory.open", args, "memory:open").await,
            "brain.memory.detail" => {
                brain_tool(
                    brain,
                    "memory.open",
                    normalize_memory_open_args(args),
                    "memory:open",
                )
                .await
            }
            "brain.ledger.verify" | "brain.ledger.inspect" => {
                brain_tool(brain, "ledger.verify", args, "system").await
            }
            "brain.compaction.snapshot" | "brain.continuity.snapshot" => {
                brain_tool(brain, "session.compaction_snapshot", args, "system").await
            }
            "brain.compaction.run" => brain_tool(brain, "session.compact", args, "system").await,
            "brain.reasoning.trace.list" => {
                brain_tool(brain, "reasoning.bridge.list", args, "system").await
            }
            "brain.confidence.snapshot" => {
                brain_tool(brain, "diagnostics.trust", args, "diagnostics:read").await
            }
            "brain.route_certificate.inspect" => {
                brain_tool(
                    brain,
                    "route.certificate.open",
                    normalize_route_certificate_args(args),
                    "system",
                )
                .await
            }
            "brain.tool_trace.list" => brain_tool(brain, "events.list", args, "events:read").await,
            "brain.benchmark.evidence.list" => {
                brain_tool(brain, "benchmarks.list", args, "system").await
            }
            "brain.cognitive_audit.snapshot" => {
                brain_tool(brain, "diagnostics.recall", args, "diagnostics:read").await
            }
            "skill.registry.list" => Ok(json!({"skills": self.skill_rows()})),
            "plugin.registry.list" => Ok(json!({"plugins": self.plugin_rows()})),
            "mcp.registry.list" => Ok(json!({"mcp_servers": self.mcp_rows()})),
            // MCP tool dispatch: mcp.<server_id>.<tool_name>
            // Ref: "Model Context Protocol" (Anthropic, 2024) — tools/call on
            // connected sessions with namespaced tool IDs.
            t if t.starts_with("mcp.") => self.dispatch_mcp_tool(t, args).await,
            _ => Err(error_payload(
                StatusCode::NOT_IMPLEMENTED,
                -32044,
                "tool_not_implemented",
                json!({"tool_id": tool.id}),
            )),
        }
    }

    /// Dispatch an MCP namespaced tool call (mcp.<server_id>.<tool_name>).
    /// Ref: "Model Context Protocol" (Anthropic, 2024) — tools/call on
    /// connected sessions with namespaced tool IDs.
    async fn dispatch_mcp_tool(
        &self,
        tool_id: &str,
        args: Value,
    ) -> Result<Value, (StatusCode, Value)> {
        let parts: Vec<&str> = tool_id.splitn(3, '.').collect();
        if parts.len() != 3 {
            return Err(error_payload(
                StatusCode::BAD_REQUEST,
                -32602,
                "invalid_mcp_tool_id",
                json!({"tool_id": tool_id, "expected": "mcp.<server_id>.<tool_name>"}),
            ));
        }
        let server_id = parts[1];
        let tool_name = parts[2];
        let (status, value) = self.mcp_call_tool(json!({
            "server_id": server_id,
            "tool_name": tool_name,
            "name": tool_name,
            "arguments": args,
            "approval": {"granted": true}
        }));
        if status.is_success() {
            Ok(value.get("result").cloned().unwrap_or(value))
        } else {
            Err((status, value))
        }
    }

    fn tool_preview_store_path(&self) -> PathBuf {
        self.tool_preview_store_path
            .clone()
            .unwrap_or_else(|| hom_local_dir().join("runtime/tool-previews.json"))
    }

    fn read_tool_preview_store(&self) -> Value {
        let path = self.tool_preview_store_path();
        std::fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str::<Value>(&content).ok())
            .unwrap_or_else(
                || json!({"artifact_kind": "tool_preview_store_v1", "previews": {}, "applied": []}),
            )
    }

    fn write_tool_preview_store(&self, store: &Value) -> Result<(), (StatusCode, Value)> {
        let path = self.tool_preview_store_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                error_payload(
                    StatusCode::BAD_REQUEST,
                    -32050,
                    "tool_preview_store_write_failed",
                    json!({"path": parent.display().to_string(), "error": error.to_string()}),
                )
            })?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(store).unwrap_or_default()).map_err(
            |error| {
                error_payload(
                    StatusCode::BAD_REQUEST,
                    -32050,
                    "tool_preview_store_write_failed",
                    json!({"path": tmp.display().to_string(), "error": error.to_string()}),
                )
            },
        )?;
        std::fs::rename(&tmp, &path).map_err(|error| {
            error_payload(
                StatusCode::BAD_REQUEST,
                -32050,
                "tool_preview_store_write_failed",
                json!({"path": path.display().to_string(), "error": error.to_string()}),
            )
        })
    }

    fn preview_file_write(&self, args: Value) -> Result<Value, (StatusCode, Value)> {
        let path = required_absolute_path(&args)?;
        let content = required_string(&args, "content")?;
        let mode = required_string(&args, "mode")?;
        if !matches!(mode.as_str(), "create" | "replace" | "append") {
            return Err(error_payload(
                StatusCode::BAD_REQUEST,
                -32602,
                "invalid_write_mode",
                json!({"mode": mode, "allowed": ["create", "replace", "append"]}),
            ));
        }
        let existing_bytes = std::fs::metadata(&path).map(|metadata| metadata.len()).ok();
        let content_sha256 = sha256_hex(&content);
        let preview_fingerprint = sha256_hex(
            &canonical_json(&json!({
                "path": path,
                "content_sha256": content_sha256,
                "mode": mode,
                "existing_bytes": existing_bytes
            }))
            .unwrap_or_default(),
        );
        let preview_id = format!("preview_{}", &preview_fingerprint[..16]);
        let mut store = self.read_tool_preview_store();
        if !store.is_object() {
            store =
                json!({"artifact_kind": "tool_preview_store_v1", "previews": {}, "applied": []});
        }
        if store.get("previews").and_then(Value::as_object).is_none() {
            store["previews"] = json!({});
        }
        if store.get("applied").and_then(Value::as_array).is_none() {
            store["applied"] = json!([]);
        }
        store["artifact_kind"] = json!("tool_preview_store_v1");
        store["previews"][&preview_id] = json!({
            "preview_id": preview_id,
            "tool_id": "filesystem.write.preview",
            "path": path,
            "mode": mode,
            "content": content,
            "content_bytes": content.as_bytes().len(),
            "content_sha256": content_sha256,
            "existing_bytes": existing_bytes,
            "created_at": now_ms()
        });
        self.write_tool_preview_store(&store)?;
        Ok(json!({
            "preview_id": preview_id,
            "path": path,
            "mode": mode,
            "content_bytes": content.as_bytes().len(),
            "content_sha256": content_sha256,
            "existing_bytes": existing_bytes,
            "previewed": true,
            "executed": false,
            "persisted": true,
            "restart_survival": true,
            "compaction_survival": true,
            "execution_kind": "preview_only"
        }))
    }

    fn apply_file_write(&self, args: Value) -> Result<Value, (StatusCode, Value)> {
        let preview_id = required_string(&args, "preview_id")?;
        let approved = args
            .get("approved")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !approved {
            return Err(error_payload(
                StatusCode::FORBIDDEN,
                -32080,
                "tool_apply_not_approved",
                json!({"preview_id": preview_id}),
            ));
        }
        let mut store = self.read_tool_preview_store();
        let Some(record) = store
            .get("previews")
            .and_then(Value::as_object)
            .and_then(|previews| previews.get(&preview_id))
            .cloned()
        else {
            return Err(error_payload(
                StatusCode::BAD_REQUEST,
                -32044,
                "tool_preview_not_found",
                json!({"preview_id": preview_id}),
            ));
        };
        let path = record
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mode = record
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let content = record
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if let Some(parent) = Path::new(&path).parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                error_payload(
                    StatusCode::BAD_REQUEST,
                    -32050,
                    "file_write_failed",
                    json!({"path": parent.display().to_string(), "error": error.to_string()}),
                )
            })?;
        }
        match mode.as_str() {
            "create" => std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .and_then(|mut file| file.write_all(content.as_bytes())),
            "replace" => std::fs::write(&path, content.as_bytes()),
            "append" => std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .and_then(|mut file| file.write_all(content.as_bytes())),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid mode",
            )),
        }
        .map_err(|error| {
            error_payload(
                StatusCode::BAD_REQUEST,
                -32050,
                "file_write_failed",
                json!({"path": path, "mode": mode, "error": error.to_string()}),
            )
        })?;
        let certificate = json!({
            "artifact_kind": "tool_execution_certificate_v1",
            "tool_id": "filesystem.write.apply",
            "preview_id": preview_id,
            "path": path,
            "mode": mode,
            "content_sha256": record.get("content_sha256").cloned().unwrap_or(Value::Null),
            "applied_at": now_ms()
        });
        if let Some(previews) = store.get_mut("previews").and_then(Value::as_object_mut) {
            previews.remove(&preview_id);
        }
        if store.get("applied").and_then(Value::as_array).is_none() {
            store["applied"] = json!([]);
        }
        if let Some(applied) = store.get_mut("applied").and_then(Value::as_array_mut) {
            applied.push(certificate.clone());
        }
        self.write_tool_preview_store(&store)?;
        Ok(json!({
            "applied": true,
            "executed": true,
            "preview_id": preview_id,
            "path": path,
            "mode": mode,
            "execution_kind": "approved_stateful_mutation",
            "execution_certificate": certificate
        }))
    }

    async fn network_check(&self, args: Value) -> Result<Value, (StatusCode, Value)> {
        let url = required_http_url(&args)?;
        let started = std::time::Instant::now();
        let head = self.http.head(&url).send().await;
        let response = match head {
            Ok(response) if response.status() != StatusCode::METHOD_NOT_ALLOWED => response,
            _ => self.http.get(&url).send().await.map_err(|error| {
                error_payload(
                    StatusCode::BAD_GATEWAY,
                    -32050,
                    "network_check_failed",
                    json!({"error": error.to_string()}),
                )
            })?,
        };
        Ok(json!({
            "url": url,
            "reachable": response.status().is_success(),
            "status": response.status().as_u16(),
            "latency_ms": started.elapsed().as_millis() as u64
        }))
    }

    async fn fetch_url(&self, args: Value) -> Result<Value, (StatusCode, Value)> {
        let url = required_http_url(&args)?;
        let max_bytes = bounded_u64(&args, "max_bytes", DEFAULT_FETCH_BYTES, MAX_FETCH_BYTES);
        let response = self.http.get(&url).send().await.map_err(|error| {
            error_payload(
                StatusCode::BAD_GATEWAY,
                -32050,
                "fetch_failed",
                json!({"error": error.to_string()}),
            )
        })?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        let text = response.text().await.map_err(|error| {
            error_payload(
                StatusCode::BAD_GATEWAY,
                -32050,
                "fetch_body_failed",
                json!({"error": error.to_string()}),
            )
        })?;
        let bytes = text.as_bytes();
        let limit = max_bytes as usize;
        let sliced = String::from_utf8_lossy(&bytes[..bytes.len().min(limit)]).to_string();
        Ok(json!({
            "url": url,
            "status": status,
            "content_type": content_type,
            "text": sliced,
            "bytes": bytes.len(),
            "truncated": bytes.len() > limit
        }))
    }

    async fn discover_provider_models(
        &self,
        entry: &ProviderCatalogEntry,
        body: &Value,
    ) -> Result<Vec<Value>, (StatusCode, Value)> {
        match entry.default_wire_mode {
            WireMode::OllamaNative => self.discover_ollama_models(entry, body).await,
            WireMode::AnthropicMessages => {
                self.discover_openai_style_models(entry, body, "x-api-key")
                    .await
            }
            WireMode::GoogleGemini => self.discover_google_models(entry, body).await,
            WireMode::CodexOAuthRuntime => {
                Ok(self.codex_oauth_runtime_models().await.unwrap_or_default())
            }
            WireMode::OpenAiChatCompletions | WireMode::OpenAiResponses => {
                self.discover_openai_style_models(entry, body, "authorization")
                    .await
            }
        }
    }

    async fn discover_openai_style_models(
        &self,
        entry: &ProviderCatalogEntry,
        body: &Value,
        auth_header_kind: &str,
    ) -> Result<Vec<Value>, (StatusCode, Value)> {
        let Some(base_url) = resolve_catalog_base_url(entry, body) else {
            return Err(error_payload(
                StatusCode::BAD_REQUEST,
                -32090,
                "provider_endpoint_missing",
                json!({"provider_id": entry.id}),
            ));
        };
        let Some(api_key) = resolve_catalog_api_key(entry, body) else {
            if entry.credential_required {
                return Err(error_payload(
                    StatusCode::BAD_REQUEST,
                    -32090,
                    "provider_key_missing",
                    json!({"provider_id": entry.id}),
                ));
            }
            return Err(error_payload(
                StatusCode::BAD_REQUEST,
                -32090,
                "provider_key_missing",
                json!({"provider_id": entry.id}),
            ));
        };
        let mut request = self
            .http
            .get(format!("{}/models", base_url.trim_end_matches('/')))
            .header("accept", "application/json");
        request = if auth_header_kind == "x-api-key" {
            request
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
        } else {
            request.bearer_auth(api_key)
        };
        let response = request.send().await.map_err(|error| {
            error_payload(
                StatusCode::BAD_GATEWAY,
                -32050,
                "provider_catalog_transport",
                json!({"provider_id": entry.id, "error": error.to_string()}),
            )
        })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(error_payload(
                StatusCode::BAD_GATEWAY,
                -32050,
                "provider_catalog_http",
                json!({"provider_id": entry.id, "status": status}),
            ));
        }
        let payload: Value = response.json().await.map_err(|error| {
            error_payload(
                StatusCode::BAD_GATEWAY,
                -32050,
                "provider_catalog_parse",
                json!({"provider_id": entry.id, "error": error.to_string()}),
            )
        })?;
        Ok(live_model_rows(entry.id, &payload))
    }

    async fn discover_google_models(
        &self,
        entry: &ProviderCatalogEntry,
        body: &Value,
    ) -> Result<Vec<Value>, (StatusCode, Value)> {
        let Some(base_url) = resolve_catalog_base_url(entry, body) else {
            return Err(error_payload(
                StatusCode::BAD_REQUEST,
                -32090,
                "provider_endpoint_missing",
                json!({"provider_id": entry.id}),
            ));
        };
        let Some(api_key) = resolve_catalog_api_key(entry, body) else {
            return Err(error_payload(
                StatusCode::BAD_REQUEST,
                -32090,
                "provider_key_missing",
                json!({"provider_id": entry.id}),
            ));
        };
        let response = self
            .http
            .get(format!(
                "{}/models?key={}",
                base_url.trim_end_matches('/'),
                api_key
            ))
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|error| {
                error_payload(
                    StatusCode::BAD_GATEWAY,
                    -32050,
                    "provider_catalog_transport",
                    json!({"provider_id": entry.id, "error": error.to_string()}),
                )
            })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(error_payload(
                StatusCode::BAD_GATEWAY,
                -32050,
                "provider_catalog_http",
                json!({"provider_id": entry.id, "status": status}),
            ));
        }
        let payload: Value = response.json().await.map_err(|error| {
            error_payload(
                StatusCode::BAD_GATEWAY,
                -32050,
                "provider_catalog_parse",
                json!({"provider_id": entry.id, "error": error.to_string()}),
            )
        })?;
        Ok(live_model_rows(entry.id, &payload))
    }

    async fn discover_ollama_models(
        &self,
        entry: &ProviderCatalogEntry,
        body: &Value,
    ) -> Result<Vec<Value>, (StatusCode, Value)> {
        let base_url = request_base_url(body).unwrap_or_else(|| {
            std::env::var("OLLAMA_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string())
        });
        let response = self
            .http
            .get(format!("{}/api/tags", base_url.trim_end_matches('/')))
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|error| {
                error_payload(
                    StatusCode::BAD_GATEWAY,
                    -32050,
                    "provider_catalog_transport",
                    json!({"provider_id": entry.id, "error": error.to_string()}),
                )
            })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(error_payload(
                StatusCode::BAD_GATEWAY,
                -32050,
                "provider_catalog_http",
                json!({"provider_id": entry.id, "status": status}),
            ));
        }
        let payload: Value = response.json().await.map_err(|error| {
            error_payload(
                StatusCode::BAD_GATEWAY,
                -32050,
                "provider_catalog_parse",
                json!({"provider_id": entry.id, "error": error.to_string()}),
            )
        })?;
        Ok(live_model_rows(entry.id, &payload))
    }

    async fn dispatch_provider_chat(
        &self,
        provider_id: &str,
        model: &str,
        messages: &[Value],
        body: &Value,
    ) -> Result<String, (StatusCode, Value)> {
        let Some(entry) = provider_catalog::find_provider(provider_id) else {
            return Err(error_payload(
                StatusCode::NOT_FOUND,
                -32044,
                "provider_not_found",
                json!({"provider_id": provider_id}),
            ));
        };
        let wire_mode = wire_mode_from_body(body, entry.default_wire_mode);
        match wire_mode {
            WireMode::OllamaNative => {
                self.chat_provider_adapter(
                    "ollama",
                    ProviderDialect::Ollama,
                    resolve_catalog_base_url(entry, body)
                        .unwrap_or_else(|| "http://127.0.0.1:11434".to_string()),
                    model,
                    messages,
                    body,
                    None,
                )
                .await
            }
            WireMode::OpenAiChatCompletions => {
                self.chat_catalog_openai_compatible(entry, model, messages, body)
                    .await
            }
            WireMode::OpenAiResponses => {
                // Responses support is catalog-visible, but current runtime path uses
                // chat/completions until the response event parser is implemented.
                // This keeps provider pipelines usable instead of blocking user choice.
                self.chat_catalog_openai_compatible(entry, model, messages, body)
                    .await
            }
            WireMode::AnthropicMessages => {
                self.chat_provider_adapter(
                    "anthropic",
                    ProviderDialect::Anthropic,
                    resolve_catalog_base_url(entry, body)
                        .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string()),
                    model,
                    messages,
                    body,
                    entry.api_key_env,
                )
                .await
            }
            WireMode::GoogleGemini => {
                self.chat_provider_adapter(
                    "google",
                    ProviderDialect::Google,
                    resolve_catalog_base_url(entry, body).unwrap_or_else(|| {
                        "https://generativelanguage.googleapis.com/v1beta".to_string()
                    }),
                    model,
                    messages,
                    body,
                    entry.api_key_env,
                )
                .await
            }
            WireMode::CodexOAuthRuntime => {
                self.chat_codex_oauth_runtime(model, messages, body).await
            }
        }
    }

    async fn chat_catalog_openai_compatible(
        &self,
        entry: &ProviderCatalogEntry,
        model: &str,
        messages: &[Value],
        body: &Value,
    ) -> Result<String, (StatusCode, Value)> {
        let base_url = if entry.id == "lm-studio" {
            request_base_url(body).unwrap_or_else(|| self.lm_studio_base_url())
        } else {
            let Some(base_url) = resolve_catalog_base_url(entry, body) else {
                return Err(error_payload(
                    StatusCode::SERVICE_UNAVAILABLE,
                    -32090,
                    "provider_endpoint_missing",
                    json!({"provider_id": entry.id}),
                ));
            };
            base_url
        };
        let Some(api_key) = resolve_catalog_api_key(entry, body) else {
            if entry.credential_required {
                return Err(error_payload(
                    StatusCode::SERVICE_UNAVAILABLE,
                    -32090,
                    "provider_key_missing",
                    json!({"provider_id": entry.id}),
                ));
            }
            return self
                .chat_openai_compatible_with_auth(entry.id, model, messages, body, &base_url, None)
                .await;
        };
        self.chat_openai_compatible_with_auth(
            entry.id,
            model,
            messages,
            body,
            &base_url,
            Some(api_key),
        )
        .await
    }

    async fn chat_openai_compatible_with_auth(
        &self,
        provider_id: &str,
        model: &str,
        messages: &[Value],
        body: &Value,
        base_url: &str,
        api_key: Option<String>,
    ) -> Result<String, (StatusCode, Value)> {
        let mut payload = json!({
            "model": model,
            "messages": messages,
            "stream": body.get("stream").and_then(Value::as_bool).unwrap_or(false),
            "max_tokens": body.get("max_tokens").cloned().unwrap_or(json!(1024))
        });
        if let Some(map) = payload.as_object_mut() {
            if let Some(extra) = safe_provider_extra_body(body).as_object() {
                for (key, value) in extra {
                    map.insert(key.clone(), value.clone());
                }
            }
        }
        let mut request = self
            .http
            .post(format!(
                "{}/chat/completions",
                base_url.trim_end_matches('/')
            ))
            .json(&payload);
        if let Some(api_key) = api_key {
            request = request.bearer_auth(api_key);
        }
        let response = request.send().await.map_err(|error| {
            error_payload(
                StatusCode::BAD_GATEWAY,
                -32050,
                "provider_transport",
                json!({"provider_id": provider_id, "error": error.to_string()}),
            )
        })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(error_payload(
                StatusCode::BAD_GATEWAY,
                -32050,
                "provider_http",
                json!({"provider_id": provider_id, "status": status}),
            ));
        }
        let payload: Value = response.json().await.map_err(|error| {
            error_payload(
                StatusCode::BAD_GATEWAY,
                -32050,
                "provider_response_parse",
                json!({"provider_id": provider_id, "error": error.to_string()}),
            )
        })?;
        Ok(payload
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    async fn chat_provider_adapter(
        &self,
        provider_id: &'static str,
        dialect: ProviderDialect,
        base_url: String,
        model: &str,
        messages: &[Value],
        body: &Value,
        env_key: Option<&str>,
    ) -> Result<String, (StatusCode, Value)> {
        let provider = HttpProvider::new(HttpProviderDescriptor {
            family: provider_id,
            default_base_url: "",
            capabilities: vec![ProviderCapability::Chat],
            dialect,
        });
        let api_key =
            request_api_key(body).or_else(|| env_key.and_then(|key| std::env::var(key).ok()));
        let credential_ref = string_field(body, &["credential_ref", "credentialRef"]);
        let messages = chat_messages(messages);
        let result = provider
            .chat_complete(
                ChatCompleteParams {
                    provider_id: provider_id.to_string(),
                    model: model.to_string(),
                    messages,
                    stream: false,
                    tools: None,
                    response_format: None,
                    max_tokens: body
                        .get("max_tokens")
                        .and_then(Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok()),
                    api_key,
                    credential_ref,
                    base_url: Some(base_url),
                },
                None,
            )
            .await
            .map_err(provider_error_payload)?;
        Ok(result.content)
    }

    async fn chat_codex_oauth_runtime(
        &self,
        model: &str,
        messages: &[Value],
        body: &Value,
    ) -> Result<String, (StatusCode, Value)> {
        let base_url = self.codex_oauth_runtime_url();
        let mut payload = body.as_object().cloned().unwrap_or_default();
        payload.insert("provider_id".to_string(), json!("codex-oauth"));
        payload.insert("model".to_string(), json!(model));
        payload.insert("messages".to_string(), Value::Array(messages.to_vec()));
        payload.insert("stream".to_string(), json!(false));

        let response = self
            .http
            .post(format!("{}/chat", base_url.trim_end_matches('/')))
            .json(&Value::Object(payload))
            .send()
            .await
            .map_err(provider_transport_error)?;
        let status =
            StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let body = response.json::<Value>().await.map_err(|error| {
            error_payload(
                StatusCode::BAD_GATEWAY,
                -32050,
                "provider_response_parse_error",
                json!({"provider_id": "codex-oauth", "detail": error.to_string()}),
            )
        })?;
        if !status.is_success() {
            return Err((status, body));
        }
        let content = body
            .get("content")
            .or_else(|| body.get("answer_text"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                error_payload(
                    StatusCode::BAD_GATEWAY,
                    -32050,
                    "provider_empty_response",
                    json!({"provider_id": "codex-oauth"}),
                )
            })?;
        Ok(content.to_string())
    }

    async fn record_tool_observability(
        &self,
        brain: &BrainClient,
        tool: &ToolDescriptor,
        args: &Value,
        execution: &ToolExecution,
        source: &str,
        latency: Duration,
    ) -> Result<Value, BrainClientError> {
        let params_json = canonical_json(args).unwrap_or_else(|_| "{}".to_string());
        let linked_memory_id = string_field(
            args,
            &[
                "linked_memory_id",
                "linkedMemoryId",
                "memory_id",
                "memoryId",
                "target_memory_id",
            ],
        );
        let provider_id = string_field(args, &["provider_id", "providerId"]);
        let model_id = string_field(args, &["model_id", "modelId", "model"]);
        let route_certificate_id =
            string_field(args, &["route_certificate_id", "routeCertificateId"]);
        let gate_task_id = string_field(args, &["gate_task_id", "gateTaskId"]);
        let error_code = execution
            .error
            .as_ref()
            .and_then(|error| error.get("code"))
            .and_then(Value::as_i64);
        let outcome = if execution.ok { "ok" } else { "error" };
        let provenance = json!({
            "source": source,
            "tool_id": tool.id,
            "tool_name": tool.name,
            "status": execution.status.as_u16(),
            "input_summary": execution.input_summary,
            "result_summary": execution.result.as_ref().map(result_summary).unwrap_or_else(|| json!({})),
            "trace": execution.trace,
            "observability_contract": "fail_open_with_trace_note"
        });
        brain
            .call(
                "reasoning.tool_event.record",
                json!({
                    "tool_id": tool.id,
                    "method": tool.id,
                    "permission_scope": tool.permission_domain,
                    "invocation_source": source,
                    "params_hash": sha256_hex(&params_json),
                    "outcome": outcome,
                    "error_code": error_code,
                    "latency_ms": i64::try_from(latency.as_millis()).unwrap_or(i64::MAX),
                    "linked_memory_id": linked_memory_id,
                    "provider_id": provider_id,
                    "model_id": model_id,
                    "route_certificate_id": route_certificate_id,
                    "gate_task_id": gate_task_id,
                    "provenance": provenance
                }),
                "memory:save",
            )
            .await
    }

    async fn record_tool_benchmark(
        &self,
        brain: &BrainClient,
        tool: &ToolDescriptor,
        execution: &ToolExecution,
        source: &str,
        latency: Duration,
    ) -> Result<Value, BrainClientError> {
        let trace_bounded = execution.trace.len() <= 8;
        let provider_context_required = matches!(tool.domain, "provider" | "network");
        let provider_context_present = execution
            .result
            .as_ref()
            .and_then(|result| {
                result
                    .get("provider_id")
                    .or_else(|| result.get("providerId"))
            })
            .or_else(|| execution.input_summary.get("provider_id"))
            .is_some();
        let mut passes = vec![json!("permission domain present")];
        let mut failures = Vec::new();
        if trace_bounded {
            passes.push(json!("execution trace bounded"));
        } else {
            failures.push(json!("execution trace exceeded bounded contract"));
        }
        if provider_context_required {
            if provider_context_present {
                passes.push(json!("provider context present when relevant"));
            } else {
                failures.push(json!(
                    "provider context missing for provider/network benchmark"
                ));
            }
        }
        let status = if execution.ok && failures.is_empty() {
            "pass"
        } else {
            "fail"
        };
        let severity = if status == "pass" {
            "production_safe"
        } else if execution.ok {
            "degraded"
        } else {
            "unknown"
        };
        brain
            .call(
                "benchmarks.record",
                json!({
                    "benchmark_kind": "tool_use",
                    "subject_id": tool.id,
                    "subject_kind": tool.kind,
                    "scenario": "tool descriptor completeness",
                    "expected_contract": {
                        "permission_domain": true,
                        "trace_bounded": true,
                        "provider_context_when_relevant": provider_context_required
                    },
                    "actual_contract": {
                        "permission_domain": !tool.permission_domain.is_empty(),
                        "trace_bounded": trace_bounded,
                        "provider_context_present": provider_context_present,
                        "execution_ok": execution.ok,
                        "http_status": execution.status.as_u16()
                    },
                    "status": status,
                    "severity": severity,
                    "passes": passes,
                    "failures": failures,
                    "evidence": [{
                        "kind": "tool_execution_trace",
                        "source": source,
                        "tool_id": tool.id,
                        "trace": execution.trace_summary(),
                        "latency_ms": latency.as_millis() as u64,
                        "inspector_target": {"kind": "benchmark_result", "id": "pending"}
                    }],
                    "started_at_s": now_ms() as i64 / 1000,
                    "finished_at_s": now_ms() as i64 / 1000,
                    "duration_ms": latency.as_millis() as u64,
                    "created_by": "capability_mesh"
                }),
                "memory:save",
            )
            .await
    }

    async fn record_action(
        &self,
        brain: &BrainClient,
        action: &str,
        payload: Value,
    ) -> Result<Value, BrainClientError> {
        let value = serde_json::to_string(&json!({
            "action": action,
            "payload": payload.clone()
        }))
        .unwrap_or_else(|_| format!("HOM capability action trace: {action}"));
        brain
            .call(
                "memory.save",
                json!({
                    "key": format!("action:{}:{}", action, now_ms()),
                    "value": value,
                    "memory_type": "action_trace",
                    "source": "capability_mesh",
                    "metadata": {
                        "action": action,
                        "payload": payload,
                        "recorded_at_ms": now_ms()
                    }
                }),
                "memory:save",
            )
            .await
    }
}

impl Default for CapabilityMesh {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolDescriptor {
    fn to_value_without_hash(&self) -> Value {
        json!({
            "id": self.id,
            "capability_id": self.capability_id,
            "descriptor_version": self.descriptor_version,
            "name": self.name,
            "description": self.description,
            "enabled": matches!(self.state, "available" | "enabled"),
            "kind": self.kind,
            "category": tool_category(self.id, self.kind, self.domain),
            "domain": self.domain,
            "owner_service": tool_owner_service(self.id, self.kind, self.domain),
            "permission_domain": self.permission_domain,
            "durability_class": tool_durability_class(self.id),
            "restart_survival": tool_restart_survival(self.id),
            "compaction_survival": tool_compaction_survival(self.id),
            "mutation_level": tool_mutation_level(self.id),
            "approval_required": tool_approval_required(self.id),
            "preview_required": tool_preview_required(self.id),
            "route": tool_route(self.id),
            "preview_route": tool_preview_route(self.id),
            "apply_route": tool_apply_route(self.id),
            "unavailable_reason": tool_unavailable_reason(self.id, self.state),
            "missing_contract": tool_missing_contract(self.id, self.state),
            "transport": self.transport,
            "retrieval_path": self.retrieval_path,
            "dispatch_target": self.dispatch_target,
            "gate_required": true,
            "state": self.state,
            "input_schema": self.input_schema,
            "output_schema": self.output_schema,
            "examples": self.examples
        })
    }

    fn descriptor_hash(&self) -> String {
        sha256_hex(&canonical_json(&self.to_value_without_hash()).unwrap_or_default())
    }

    fn to_value(&self) -> Value {
        let mut value = self.to_value_without_hash();
        value["descriptor_hash"] = json!(self.descriptor_hash());
        value
    }
}

impl BrokerDispatchV1 {
    fn to_value(&self) -> Value {
        json!({
            "version": "broker_dispatch_v1",
            "trace_id": self.trace_id,
            "capability_id": self.capability_id,
            "tool_id": self.tool_id,
            "descriptor_hash": self.descriptor_hash,
            "transport": self.transport,
            "permission_domain": self.permission_domain,
            "arguments": summarize_input(&self.arguments),
            "source": self.source,
            "caller": self.caller,
            "dispatch_target": self.dispatch_target
        })
    }

    fn trace_detail(&self) -> Value {
        json!({
            "version": "broker_dispatch_v1",
            "trace_id": self.trace_id,
            "capability_id": self.capability_id,
            "descriptor_hash": self.descriptor_hash,
            "transport": self.transport,
            "permission_domain": self.permission_domain,
            "source": self.source,
            "caller": self.caller,
            "dispatch_target": self.dispatch_target
        })
    }
}

impl ToolExecution {
    fn error(
        tool_id: &str,
        status: StatusCode,
        code: i64,
        message: &str,
        data: Value,
        input_summary: Value,
    ) -> Self {
        Self {
            ok: false,
            tool_id: tool_id.to_string(),
            status,
            input_summary,
            result: None,
            error: Some(json!({"code": code, "message": message, "data": data})),
            gate: None,
            trace: Vec::new(),
            broker: None,
        }
    }

    fn into_response(self) -> Value {
        let outcome = self.outcome();
        let error = self.canonical_error();
        let result = self.result.unwrap_or(Value::Null);
        json!({
            "ok": self.ok,
            "tool_id": self.tool_id,
            "outcome": outcome.as_str(),
            "result": result,
            "error": error,
            "trace": self.trace,
            "gate": self.gate,
            "broker": self.broker.as_ref().map(BrokerDispatchV1::to_value),
            "source": "capability_mesh"
        })
    }

    fn outcome(&self) -> ToolOutcome {
        if self.ok {
            if self
                .result
                .as_ref()
                .and_then(|result| result.get("execution_status"))
                .and_then(Value::as_str)
                == Some("failed")
            {
                ToolOutcome::Failed
            } else if self
                .result
                .as_ref()
                .and_then(|result| result.get("previewed"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                ToolOutcome::Previewed
            } else {
                ToolOutcome::Executed
            }
        } else if self
            .gate
            .as_ref()
            .and_then(|gate| gate.get("decision"))
            .and_then(Value::as_str)
            == Some("denied")
        {
            ToolOutcome::Blocked
        } else if self.status == StatusCode::NOT_IMPLEMENTED
            || self
                .canonical_error()
                .get("message")
                .and_then(Value::as_str)
                .is_some_and(|message| message == "tool_not_implemented")
        {
            ToolOutcome::Unavailable
        } else {
            ToolOutcome::Failed
        }
    }

    fn canonical_error(&self) -> Value {
        if self.ok {
            return Value::Null;
        }
        let Some(error) = self.error.as_ref() else {
            return json!({"code": -32603, "message": "tool_failed"});
        };
        error.get("error").cloned().unwrap_or_else(|| error.clone())
    }

    fn clone_for_prompt(&self) -> Value {
        if self.ok {
            json!({"ok": true, "tool_id": self.tool_id, "result": self.result, "trace_id": self.broker.as_ref().map(|broker| broker.trace_id.as_str()), "descriptor_hash": self.broker.as_ref().map(|broker| broker.descriptor_hash.as_str())})
        } else {
            json!({"ok": false, "tool_id": self.tool_id, "error": self.error, "trace_id": self.broker.as_ref().map(|broker| broker.trace_id.as_str()), "descriptor_hash": self.broker.as_ref().map(|broker| broker.descriptor_hash.as_str())})
        }
    }

    fn trace_summary(&self) -> Value {
        json!({
            "tool_id": self.tool_id,
            "ok": self.ok,
            "status": self.status.as_u16(),
            "gate": self.gate,
            "result_summary": self.result.as_ref().map(result_summary).unwrap_or_else(|| json!({})),
            "executed": self.result.as_ref().and_then(|result| result.get("executed")).and_then(Value::as_bool),
            "trace": self.trace,
            "error": self.error,
            "broker": self.broker.as_ref().map(BrokerDispatchV1::to_value),
            "descriptor_hash": self.broker.as_ref().map(|broker| broker.descriptor_hash.as_str()),
            "capability_id": self.broker.as_ref().map(|broker| broker.capability_id.as_str()),
            "transport": self.broker.as_ref().map(|broker| broker.transport.as_str())
        })
    }

    fn action_payload(&self, source: &str) -> Value {
        let permission_decision = self
            .gate
            .as_ref()
            .and_then(|gate| gate.get("decision"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        json!({
            "actor": "hom.chat.orchestrator",
            "source": source,
            "capability_id": self.broker.as_ref().map(|broker| broker.capability_id.as_str()).unwrap_or(self.tool_id.as_str()),
            "tool_id": self.tool_id,
            "trace_id": self.broker.as_ref().map(|broker| broker.trace_id.as_str()),
            "descriptor_hash": self.broker.as_ref().map(|broker| broker.descriptor_hash.as_str()),
            "transport": self.broker.as_ref().map(|broker| broker.transport.as_str()),
            "dispatch_target": self.broker.as_ref().map(|broker| broker.dispatch_target.as_str()),
            "permission_decision": permission_decision,
            "input_summary": self.input_summary,
            "ok": self.ok,
            "status": self.status.as_u16(),
            "gate": self.gate,
            "result": self.result.as_ref().map(result_summary).unwrap_or_else(|| json!({})),
            "result_summary": self.result.as_ref().map(result_summary).unwrap_or_else(|| json!({})),
            "error": self.error,
            "timestamp_ms": now_ms()
        })
    }

    fn is_descriptor_integrity_error(&self) -> bool {
        self.error
            .as_ref()
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .is_some_and(|message| {
                matches!(
                    message,
                    "descriptor_hash_required" | "descriptor_hash_mismatch"
                )
            })
    }
}

fn tool_descriptors() -> Vec<ToolDescriptor> {
    let browser_state = if agent_browser_available() {
        "available"
    } else {
        "missing"
    };
    vec![
        tool(
            "brain.memory.recall",
            "Brain Memory Recall",
            "Recall relevant memories from the cognitive brain.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","required":["query"],"properties":{"query":{"type":"string"},"limit":{"type":"number"}}}),
        ),
        tool(
            "brain.memory.save",
            "Brain Memory Save",
            "Save a compact memory into the cognitive brain.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","required":["content"],"properties":{"content":{"type":"string"},"key":{"type":"string"},"metadata":{"type":"object"}}}),
        ),
        tool(
            "brain.reasoning.run",
            "Brain Reasoning Run",
            "Run a reasoning bridge for a memory.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","required":["memory_id"],"properties":{"memory_id":{"type":"string"}}}),
        ),
        tool(
            "brain.nightly.tree",
            "Brain Nightly Tree",
            "Read nightly consolidation artifacts.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","properties":{}}),
        ),
        tool(
            "brain.status",
            "Brain Status",
            "Read brain runtime status.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","properties":{}}),
        ),
        tool(
            "brain.recall.smart",
            "Brain Smart Recall",
            "Run smart recall through the cognitive brain recall surface.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","required":["query"],"properties":{"query":{"type":"string"},"limit":{"type":"number"}}}),
        ),
        tool(
            "brain.recall.detail",
            "Brain Recall Detail",
            "Open a recall result or memory evidence detail.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}),
        ),
        tool(
            "brain.memory.detail",
            "Brain Memory Detail",
            "Open a memory detail record from the cognitive brain.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","required":["memory_id"],"properties":{"memory_id":{"type":"string"}}}),
        ),
        tool(
            "brain.ledger.verify",
            "Brain Ledger Verify",
            "Verify cognitive ledger integrity.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","properties":{}}),
        ),
        tool(
            "brain.ledger.inspect",
            "Brain Ledger Inspect",
            "Inspect cognitive ledger details or repair evidence.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","properties":{"segment_id":{"type":"string"}}}),
        ),
        tool(
            "brain.compaction.snapshot",
            "Brain Compaction Snapshot",
            "Read the ambient compaction/continuity snapshot.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","properties":{"session_id":{"type":"string"}}}),
        ),
        tool(
            "brain.compaction.run",
            "Brain Compaction Run",
            "Run explicit cognitive compaction through the brain runtime.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","properties":{"session_id":{"type":"string"},"approved":{"type":"boolean"}}}),
        ),
        tool(
            "brain.reasoning.trace.list",
            "Brain Reasoning Trace List",
            "List reasoning traces from the cognitive brain.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","properties":{"limit":{"type":"number"}}}),
        ),
        tool(
            "brain.reasoning.trace.detail",
            "Brain Reasoning Trace Detail",
            "Open a reasoning trace detail from the cognitive brain once the route/method exists.",
            "builtin",
            "brain",
            "brain",
            "pending",
            json!({"type":"object","required":["trace_id"],"properties":{"trace_id":{"type":"string"}}}),
        ),
        tool(
            "brain.reasoning.policy.get",
            "Brain Reasoning Policy",
            "Read reasoning policy state from the cognitive runtime.",
            "builtin",
            "brain",
            "brain",
            "pending",
            json!({"type":"object","properties":{}}),
        ),
        tool(
            "brain.confidence.snapshot",
            "Brain Confidence Snapshot",
            "Read cognitive confidence/calibration state.",
            "builtin",
            "brain",
            "brain",
            "pending",
            json!({"type":"object","properties":{}}),
        ),
        tool(
            "brain.continuity.snapshot",
            "Brain Continuity Snapshot",
            "Read cognitive continuity/session state.",
            "builtin",
            "brain",
            "brain",
            "pending",
            json!({"type":"object","properties":{"session_id":{"type":"string"}}}),
        ),
        tool(
            "brain.route_certificate.inspect",
            "Brain Route Certificate Inspect",
            "Inspect route certificate evidence and validation state.",
            "builtin",
            "brain",
            "brain",
            "pending",
            json!({"type":"object","required":["route_certificate_id"],"properties":{"route_certificate_id":{"type":"string"}}}),
        ),
        tool(
            "brain.route_certificate.validate",
            "Brain Route Certificate Validate",
            "Validate a route certificate once the brain validation method and ingress route exist.",
            "builtin",
            "brain",
            "brain",
            "pending",
            json!({"type":"object","required":["route_certificate_id"],"properties":{"route_certificate_id":{"type":"string"}}}),
        ),
        tool(
            "brain.tool_trace.list",
            "Brain Tool Trace List",
            "List cognitive tool-event traces.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","properties":{"limit":{"type":"number"}}}),
        ),
        tool(
            "brain.tool_trace.detail",
            "Brain Tool Trace Detail",
            "Open cognitive tool-event trace detail once the brain method and ingress route exist.",
            "builtin",
            "brain",
            "brain",
            "pending",
            json!({"type":"object","required":["trace_id"],"properties":{"trace_id":{"type":"string"}}}),
        ),
        tool(
            "brain.benchmark.evidence.list",
            "Brain Benchmark Evidence List",
            "List benchmark/evidence artifacts recorded by the cognitive runtime.",
            "builtin",
            "brain",
            "brain",
            "available",
            json!({"type":"object","properties":{"limit":{"type":"number"}}}),
        ),
        tool(
            "brain.cognitive_audit.snapshot",
            "Brain Cognitive Audit Snapshot",
            "Read a cognitive audit/state verification snapshot.",
            "builtin",
            "brain",
            "brain",
            "pending",
            json!({"type":"object","properties":{}}),
        ),
        tool(
            "filesystem.read",
            "Read File",
            "Read a bounded chunk from an absolute path.",
            "builtin",
            "filesystem",
            "filesystem",
            "available",
            json!({"type":"object","required":["path"],"properties":{"path":{"type":"string"},"offset":{"type":"number"},"max_bytes":{"type":"number"}}}),
        ),
        tool(
            "filesystem.list",
            "List Directory",
            "List entries under an absolute directory path.",
            "builtin",
            "filesystem",
            "filesystem",
            "available",
            json!({"type":"object","required":["path"],"properties":{"path":{"type":"string"},"limit":{"type":"number"}}}),
        ),
        tool(
            "filesystem.search",
            "Search Files",
            "Search file names or bounded text under an allowed directory.",
            "builtin",
            "filesystem",
            "filesystem",
            "available",
            json!({"type":"object","required":["path","query"],"properties":{"path":{"type":"string"},"query":{"type":"string"},"mode":{"enum":["files","content"]},"limit":{"type":"number"}}}),
        ),
        tool(
            "filesystem.write.preview",
            "Preview File Write",
            "Preview a create, replace, or append file write without mutating disk.",
            "builtin",
            "filesystem",
            "filesystem",
            "pending",
            json!({"type":"object","required":["path","content","mode"],"properties":{"path":{"type":"string"},"content":{"type":"string"},"mode":{"enum":["create","replace","append"]}}}),
        ),
        tool(
            "filesystem.write.apply",
            "Apply File Write",
            "Apply a previously previewed create, replace, or append file write after permission approval.",
            "builtin",
            "filesystem",
            "filesystem",
            "pending",
            json!({"type":"object","required":["preview_id","approved"],"properties":{"preview_id":{"type":"string"},"approved":{"type":"boolean"}}}),
        ),
        tool(
            "filesystem.edit.preview",
            "Preview File Edit",
            "Preview a targeted file edit without mutating disk.",
            "builtin",
            "filesystem",
            "filesystem",
            "pending",
            json!({"type":"object","required":["path","old_string","new_string"],"properties":{"path":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"},"replace_all":{"type":"boolean"}}}),
        ),
        tool(
            "filesystem.edit.apply",
            "Apply File Edit",
            "Apply a previously previewed targeted file edit after permission approval.",
            "builtin",
            "filesystem",
            "filesystem",
            "pending",
            json!({"type":"object","required":["preview_id","approved"],"properties":{"preview_id":{"type":"string"},"approved":{"type":"boolean"}}}),
        ),
        tool(
            "filesystem.patch.preview",
            "Preview File Patch",
            "Preview a multi-file patch without mutating disk.",
            "builtin",
            "filesystem",
            "filesystem",
            "pending",
            json!({"type":"object","required":["patch"],"properties":{"patch":{"type":"string"}}}),
        ),
        tool(
            "filesystem.patch.apply",
            "Apply File Patch",
            "Apply a previously previewed patch after permission approval.",
            "builtin",
            "filesystem",
            "filesystem",
            "pending",
            json!({"type":"object","required":["preview_id","approved"],"properties":{"preview_id":{"type":"string"},"approved":{"type":"boolean"}}}),
        ),
        tool(
            "network.check",
            "Network Check",
            "Check whether an HTTP(S) endpoint is reachable.",
            "builtin",
            "network",
            "network",
            "available",
            json!({"type":"object","required":["url"],"properties":{"url":{"type":"string"}}}),
        ),
        tool(
            "web.fetch_url",
            "Fetch URL",
            "Fetch bounded text from an HTTP(S) URL.",
            "builtin",
            "network",
            "network",
            "available",
            json!({"type":"object","required":["url"],"properties":{"url":{"type":"string"},"max_bytes":{"type":"number"}}}),
        ),
        tool(
            "shell.propose",
            "Propose Shell Command",
            "Create a shell command proposal; does not execute.",
            "builtin",
            "shell",
            "shell",
            "available",
            json!({"type":"object","required":["command"],"properties":{"command":{"type":"string"},"reason":{"type":"string"}}}),
        ),
        tool(
            "shell.execute",
            "Execute Shell Command",
            "Execute a shell command through the backend permission gate and return stdout, stderr, exit code, and working directory.",
            "builtin",
            "shell",
            "shell",
            "available",
            json!({"type":"object","required":["command"],"properties":{"command":{"type":"string"},"cwd":{"type":"string"},"timeout_ms":{"type":"number"},"reason":{"type":"string"}}}),
        ),
        tool(
            "browser.open",
            "Browser Open",
            "Open a URL through the local browser runner.",
            "builtin",
            "browser",
            "browser",
            browser_state,
            json!({"type":"object","required":["url"],"properties":{"url":{"type":"string"}}}),
        ),
        tool(
            "browser.snapshot",
            "Browser Snapshot",
            "Return the current browser accessibility snapshot.",
            "builtin",
            "browser",
            "browser",
            browser_state,
            json!({"type":"object","properties":{}}),
        ),
        tool(
            "browser.current_page",
            "Browser Current Page",
            "Return current browser URL or title data.",
            "builtin",
            "browser",
            "browser",
            browser_state,
            json!({"type":"object","properties":{}}),
        ),
        tool(
            "skill.registry.list",
            "List Skills",
            "List registered local skills visible to chat.",
            "skill",
            "skill",
            "brain",
            "available",
            json!({"type":"object","properties":{}}),
        ),
        tool(
            "plugin.registry.list",
            "List Plugins",
            "List registered local plugins visible to chat.",
            "plugin",
            "plugin",
            "brain",
            "available",
            json!({"type":"object","properties":{}}),
        ),
        tool(
            "mcp.registry.list",
            "List MCP Servers",
            "List registered MCP server descriptors visible to chat.",
            "mcp",
            "mcp",
            "brain",
            "available",
            json!({"type":"object","properties":{}}),
        ),
    ]
}

fn tool(
    id: &'static str,
    name: &'static str,
    description: &'static str,
    kind: &'static str,
    domain: &'static str,
    permission_domain: &'static str,
    state: &'static str,
    input_schema: Value,
) -> ToolDescriptor {
    ToolDescriptor {
        id,
        capability_id: id,
        descriptor_version: TOOL_DESCRIPTOR_VERSION,
        name,
        description,
        kind,
        domain,
        permission_domain,
        transport: tool_transport(id, kind, domain),
        retrieval_path: tool_retrieval_path(id),
        dispatch_target: tool_dispatch_target(id),
        state,
        output_schema: tool_output_schema(id),
        examples: tool_examples(id),
        input_schema,
    }
}

fn tool_transport(id: &str, kind: &str, domain: &str) -> &'static str {
    if id.starts_with("brain.") {
        "brain"
    } else if id.starts_with("browser.") {
        "browser"
    } else if id.starts_with("filesystem.") {
        "file"
    } else if id.starts_with("shell.") {
        "shell"
    } else if id.starts_with("web.") || id.starts_with("network.") {
        "api"
    } else if kind == "plugin" || domain == "plugin" {
        "plugin"
    } else if kind == "mcp" || domain == "mcp" {
        "mcp"
    } else {
        "api"
    }
}

fn tool_retrieval_path(id: &str) -> &'static str {
    match id {
        "brain.memory.recall" => "capability_mesh.tool_descriptors.brain.memory.recall",
        "brain.memory.save" => "capability_mesh.tool_descriptors.brain.memory.save",
        "brain.reasoning.run" => "capability_mesh.tool_descriptors.brain.reasoning.run",
        "brain.nightly.tree" => "capability_mesh.tool_descriptors.brain.nightly.tree",
        "brain.status" => "capability_mesh.tool_descriptors.brain.status",
        "brain.recall.smart" => "capability_mesh.tool_descriptors.brain.recall.smart",
        "brain.recall.detail" => "capability_mesh.tool_descriptors.brain.recall.detail",
        "brain.memory.detail" => "capability_mesh.tool_descriptors.brain.memory.detail",
        "brain.ledger.verify" => "capability_mesh.tool_descriptors.brain.ledger.verify",
        "brain.ledger.inspect" => "capability_mesh.tool_descriptors.brain.ledger.inspect",
        "brain.compaction.snapshot" => "capability_mesh.tool_descriptors.brain.compaction.snapshot",
        "brain.compaction.run" => "capability_mesh.tool_descriptors.brain.compaction.run",
        "brain.continuity.snapshot" => "capability_mesh.tool_descriptors.brain.continuity.snapshot",
        "brain.reasoning.trace.list" => {
            "capability_mesh.tool_descriptors.brain.reasoning.trace.list"
        }
        "brain.reasoning.policy.get" => {
            "capability_mesh.tool_descriptors.brain.reasoning.policy.get"
        }
        "brain.confidence.snapshot" => "capability_mesh.tool_descriptors.brain.confidence.snapshot",
        "brain.route_certificate.inspect" => {
            "capability_mesh.tool_descriptors.brain.route_certificate.inspect"
        }
        "brain.tool_trace.list" => "capability_mesh.tool_descriptors.brain.tool_trace.list",
        "brain.benchmark.evidence.list" => {
            "capability_mesh.tool_descriptors.brain.benchmark.evidence.list"
        }
        "brain.cognitive_audit.snapshot" => {
            "capability_mesh.tool_descriptors.brain.cognitive_audit.snapshot"
        }
        "filesystem.read" => "capability_mesh.tool_descriptors.filesystem.read",
        "filesystem.list" => "capability_mesh.tool_descriptors.filesystem.list",
        "network.check" => "capability_mesh.tool_descriptors.network.check",
        "web.fetch_url" => "capability_mesh.tool_descriptors.web.fetch_url",
        "shell.propose" => "capability_mesh.tool_descriptors.shell.propose",
        "shell.execute" => "capability_mesh.tool_descriptors.shell.execute",
        "browser.open" => "capability_mesh.tool_descriptors.browser.open",
        "browser.snapshot" => "capability_mesh.tool_descriptors.browser.snapshot",
        "browser.current_page" => "capability_mesh.tool_descriptors.browser.current_page",
        "skill.registry.list" => "capability_mesh.tool_descriptors.skill.registry.list",
        "plugin.registry.list" => "capability_mesh.tool_descriptors.plugin.registry.list",
        "mcp.registry.list" => "capability_mesh.tool_descriptors.mcp.registry.list",
        _ => "capability_mesh.tool_descriptors.unknown",
    }
}

fn tool_dispatch_target(id: &str) -> &'static str {
    match id {
        "brain.memory.recall" => "brain:memory.recall",
        "brain.memory.save" => "brain:memory.save",
        "brain.reasoning.run" => "brain:reasoning.run",
        "brain.nightly.tree" => "brain:nightly.tree",
        "brain.status" => "brain:runtime.status",
        "brain.recall.smart" => "brain:memory.recall.smart",
        "brain.recall.detail" => "brain:memory.open",
        "brain.memory.detail" => "brain:memory.open",
        "brain.ledger.verify" => "brain:ledger.verify",
        "brain.ledger.inspect" => "brain:ledger.verify",
        "brain.compaction.snapshot" => "brain:session.compaction_snapshot",
        "brain.compaction.run" => "brain:session.compact",
        "brain.continuity.snapshot" => "brain:session.compaction_snapshot",
        "brain.reasoning.trace.list" => "brain:reasoning.bridge.list",
        "brain.reasoning.trace.detail" => "brain:reasoning.bridge.open",
        "brain.reasoning.policy.get" => "brain:reasoning.policy.get",
        "brain.confidence.snapshot" => "brain:diagnostics.trust",
        "brain.route_certificate.inspect" => "brain:route.certificate.open",
        "brain.route_certificate.validate" => "brain:route.certificate.validate",
        "brain.tool_trace.list" => "brain:events.list",
        "brain.tool_trace.detail" => "brain:events.open",
        "brain.benchmark.evidence.list" => "brain:benchmarks.list",
        "brain.cognitive_audit.snapshot" => "brain:diagnostics.recall",
        "filesystem.read" => "local:filesystem.read",
        "filesystem.list" => "local:filesystem.list",
        "network.check" => "local:network.check",
        "web.fetch_url" => "local:web.fetch_url",
        "shell.propose" => "local:shell.propose",
        "shell.execute" => "local:shell.execute",
        "browser.open" => "agent-browser:open",
        "browser.snapshot" => "agent-browser:snapshot",
        "browser.current_page" => "agent-browser:current_page",
        "skill.registry.list" => "registry:skills",
        "plugin.registry.list" => "registry:plugins",
        "mcp.registry.list" => "registry:mcp",
        _ => "local:unknown",
    }
}

fn tool_output_schema(id: &str) -> Value {
    match id {
        "brain.memory.recall" => {
            json!({"type":"object","properties":{"memories":{"type":"array"},"evidence":{"type":"array"}}})
        }
        "filesystem.read" => {
            json!({"type":"object","properties":{"path":{"type":"string"},"text":{"type":"string"},"truncated":{"type":"boolean"}}})
        }
        "filesystem.list" => {
            json!({"type":"object","properties":{"path":{"type":"string"},"entries":{"type":"array"},"truncated":{"type":"boolean"}}})
        }
        "network.check" => {
            json!({"type":"object","properties":{"url":{"type":"string"},"reachable":{"type":"boolean"},"status":{"type":"number"}}})
        }
        "web.fetch_url" => {
            json!({"type":"object","properties":{"url":{"type":"string"},"text":{"type":"string"},"truncated":{"type":"boolean"}}})
        }
        "shell.propose" => {
            json!({"type":"object","properties":{"command":{"type":"string"},"proposed":{"type":"boolean"},"executed":{"const":false}}})
        }
        "browser.open" => {
            json!({"type":"object","properties":{"url":{"type":"string"},"opened":{"type":"boolean"}}})
        }
        "browser.snapshot" => json!({"type":"object","properties":{"snapshot":{"type":"string"}}}),
        "browser.current_page" => {
            json!({"type":"object","properties":{"url":{"type":"string"},"title":{"type":"string"}}})
        }
        "skill.registry.list" => json!({"type":"object","properties":{"skills":{"type":"array"}}}),
        "plugin.registry.list" => {
            json!({"type":"object","properties":{"plugins":{"type":"array"}}})
        }
        "mcp.registry.list" => {
            json!({"type":"object","properties":{"mcp_servers":{"type":"array"}}})
        }
        _ => json!({"type":"object"}),
    }
}

fn tool_examples(id: &str) -> Value {
    match id {
        "brain.memory.recall" => {
            json!([{"description":"Recall yesterday's session notes.","arguments":{"query":"yesterday's session","limit":5}}])
        }
        "brain.memory.save" => {
            json!([{"description":"Save a compact user-visible memory.","arguments":{"content":"User prefers trace-backed claims.","memory_type":"declarative"}}])
        }
        "brain.reasoning.run" => {
            json!([{"description":"Run reasoning for a memory.","arguments":{"memory_id":"mem_123"}}])
        }
        "filesystem.read" => {
            json!([{"description":"Read a bounded absolute file chunk.","arguments":{"path":"/absolute/path/file.md","offset":0,"max_bytes":65536}}])
        }
        "filesystem.list" => {
            json!([{"description":"List a granted directory.","arguments":{"path":"/absolute/path","limit":100}}])
        }
        "network.check" => {
            json!([{"description":"Check an HTTPS endpoint.","arguments":{"url":"https://example.com"}}])
        }
        "web.fetch_url" => {
            json!([{"description":"Fetch bounded web text.","arguments":{"url":"https://example.com","max_bytes":65536}}])
        }
        "shell.propose" => {
            json!([{"description":"Propose but do not execute a command.","arguments":{"command":"cargo test -p hom-ingress","reason":"verify ingress"}}])
        }
        "browser.open" => {
            json!([{"description":"Open a browser URL.","arguments":{"url":"https://example.com"}}])
        }
        "browser.snapshot" => {
            json!([{"description":"Read the current browser snapshot.","arguments":{}}])
        }
        "browser.current_page" => {
            json!([{"description":"Read the current browser page metadata.","arguments":{}}])
        }
        "skill.registry.list" => {
            json!([{"description":"List registered local skills.","arguments":{}}])
        }
        "plugin.registry.list" => {
            json!([{"description":"List registered local plugins.","arguments":{}}])
        }
        "mcp.registry.list" => {
            json!([{"description":"List registered MCP server descriptors.","arguments":{}}])
        }
        _ => json!([{"description":"Call the tool with schema-valid arguments.","arguments":{}}]),
    }
}

fn broker_dispatch(
    tool: &ToolDescriptor,
    args: &Value,
    source: &str,
    caller: &str,
    descriptor_hash: &str,
) -> BrokerDispatchV1 {
    BrokerDispatchV1 {
        trace_id: format!("broker-{}-{}", tool.id, now_ms()),
        capability_id: tool.capability_id.to_string(),
        tool_id: tool.id.to_string(),
        descriptor_hash: descriptor_hash.to_string(),
        transport: tool.transport.to_string(),
        permission_domain: tool.permission_domain.to_string(),
        arguments: args.clone(),
        source: source.to_string(),
        caller: caller.to_string(),
        dispatch_target: tool.dispatch_target.to_string(),
    }
}

fn replace_provider_row(providers: &mut Vec<Value>, provider: Value) {
    let id = provider["id"].as_str().unwrap_or_default().to_string();
    if let Some(existing) = providers
        .iter_mut()
        .find(|row| row["id"].as_str() == Some(id.as_str()))
    {
        *existing = merge_catalog_flags(provider);
    } else {
        providers.push(merge_catalog_flags(provider));
    }
}

fn merge_catalog_flags(mut provider: Value) -> Value {
    provider["catalog_visible"] = json!(true);
    provider["catalog_ready"] = json!(true);
    provider["pipeline_ready"] = json!(true);
    provider["model_selector_eligible"] = json!(true);
    if provider.get("setup_requirements").is_none() {
        provider["setup_requirements"] =
            json!({"save_button_rule": "ui_disables_save_until_required_fields_are_present"});
    }
    provider
}

fn seed_model_rows(entry: &ProviderCatalogEntry) -> Vec<Value> {
    entry
        .models
        .iter()
        .map(|model| {
            let mut row = model_row(entry.id, model.id, execution_location_for(entry.kind), true);
            row["name"] = json!(model.name);
            row["display_name"] = json!(model.name);
            row["source"] = json!("open_interpreter_seed");
            row["registry_source"] = json!("open_interpreter_seed");
            row["snapshot_state"] = json!("seeded");
            row
        })
        .collect()
}

fn credential_store_root() -> PathBuf {
    std::env::var("HOM_PROVIDER_CREDENTIAL_STORE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs_home_fallback()
                .join(".hom")
                .join("runtime")
                .join("provider-credentials")
        })
}

fn dirs_home_fallback() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn credential_store_backend() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        return "macos_keychain_cli_v1";
    }
    #[cfg(not(target_os = "macos"))]
    {
        "local_file_0600"
    }
}

fn keychain_service_for_provider(provider_id: &str) -> String {
    format!(
        "hom.ingress.provider-credential.{}",
        provider_id.replace('/', "_")
    )
}

fn write_keychain_secret(service: &str, account: &str, api_key: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("security")
            .arg("add-generic-password")
            .arg("-a")
            .arg(account)
            .arg("-s")
            .arg(service)
            .arg("-w")
            .arg(api_key)
            .arg("-U")
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(
                "security add-generic-password failed",
            ))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (service, account, api_key);
        Err(std::io::Error::other(
            "keychain backend not supported on this platform",
        ))
    }
}

fn read_keychain_secret(service: &str, account: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("security")
            .arg("find-generic-password")
            .arg("-a")
            .arg(account)
            .arg("-s")
            .arg(service)
            .arg("-w")
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if value.is_empty() { None } else { Some(value) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (service, account);
        None
    }
}

fn store_provider_credential(provider_id: &str, api_key: &str) -> std::io::Result<String> {
    let root = credential_store_root();
    fs::create_dir_all(&root)?;
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(&root, fs::Permissions::from_mode(0o700));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let digest = sha256_hex(&format!("{provider_id}:{now}:{}", api_key.len()));
    let credential_ref = format!("cred_{}_{}", provider_id.replace('-', "_"), &digest[..16]);
    let path = root.join(format!("{credential_ref}.json"));

    let mut payload = json!({
        "artifact_kind": "provider_credential_v1",
        "schema_version": 1,
        "provider_id": provider_id,
        "credential_ref": credential_ref,
        "secret_material_returned": false,
        "created_at_ms": now.to_string(),
        "storage_backend": credential_store_backend()
    });

    if credential_store_backend() == "macos_keychain_cli_v1" {
        let service = keychain_service_for_provider(provider_id);
        write_keychain_secret(&service, &credential_ref, api_key)?;
        payload["keychain_service"] = json!(service);
        payload["keychain_account"] = json!(credential_ref.clone());
    } else {
        payload["api_key"] = json!(api_key);
    }

    fs::write(&path, serde_json::to_vec_pretty(&payload)?)?;
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    let _ = store_latest_credential_ref(provider_id, &credential_ref);
    Ok(credential_ref)
}

fn latest_credential_ref_index_path() -> PathBuf {
    credential_store_root().join("latest-credential-refs.json")
}

fn store_latest_credential_ref(provider_id: &str, credential_ref: &str) -> std::io::Result<()> {
    let path = latest_credential_ref_index_path();
    let mut map = fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    map.insert(provider_id.to_string(), json!(credential_ref));
    fs::write(&path, serde_json::to_vec_pretty(&Value::Object(map))?)?;
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn load_latest_credential_ref(provider_id: &str) -> Option<String> {
    let path = latest_credential_ref_index_path();
    let map = fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())?
        .as_object()
        .cloned()?;
    map.get(provider_id)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
}

fn load_provider_credential(provider_id: &str, credential_ref: &str) -> Option<String> {
    if credential_ref.contains('/') || credential_ref.contains("..") {
        return None;
    }
    let path = credential_store_root().join(format!("{credential_ref}.json"));
    let payload: Value = fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())?;
    if payload.get("artifact_kind").and_then(Value::as_str) != Some("provider_credential_v1") {
        return None;
    }
    if payload.get("provider_id").and_then(Value::as_str) != Some(provider_id) {
        return None;
    }

    if let Some(api_key) = payload
        .get("api_key")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
    {
        if credential_store_backend() == "macos_keychain_cli_v1" {
            let service = keychain_service_for_provider(provider_id);
            if write_keychain_secret(&service, credential_ref, &api_key).is_ok() {
                let mut migrated = payload.clone();
                if let Some(obj) = migrated.as_object_mut() {
                    obj.remove("api_key");
                    obj.insert(
                        "storage_backend".to_string(),
                        json!("macos_keychain_cli_v1"),
                    );
                    obj.insert("keychain_service".to_string(), json!(service));
                    obj.insert("keychain_account".to_string(), json!(credential_ref));
                }
                let credential_path =
                    credential_store_root().join(format!("{credential_ref}.json"));
                let _ = fs::write(
                    credential_path,
                    serde_json::to_vec_pretty(&migrated).unwrap_or_default(),
                );
            }
        }
        return Some(api_key);
    }

    let backend = payload
        .get("storage_backend")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if backend == "macos_keychain_cli_v1" {
        let service = payload
            .get("keychain_service")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| keychain_service_for_provider(provider_id));
        let account = payload
            .get("keychain_account")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| credential_ref.to_string());
        return read_keychain_secret(&service, &account).filter(|value| !value.trim().is_empty());
    }

    None
}

fn load_latest_provider_credential(provider_id: &str) -> Option<String> {
    if let Some(credential_ref) = load_latest_credential_ref(provider_id) {
        if let Some(key) = load_provider_credential(provider_id, &credential_ref) {
            return Some(key);
        }
    }

    let root = credential_store_root();
    let mut newest: Option<(u128, String)> = None;
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let payload: Value = fs::read_to_string(&path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or(Value::Null);
        if payload.is_null() {
            continue;
        }
        if payload.get("artifact_kind").and_then(Value::as_str) != Some("provider_credential_v1") {
            continue;
        }
        if payload.get("provider_id").and_then(Value::as_str) != Some(provider_id) {
            continue;
        }
        let credential_ref = payload
            .get("credential_ref")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default();
        if credential_ref.trim().is_empty() {
            continue;
        }
        let Some(api_key) = load_provider_credential(provider_id, &credential_ref) else {
            continue;
        };
        let created = payload
            .get("created_at_ms")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<u128>().ok())
            .unwrap_or(0);
        match &newest {
            Some((current_time, _)) if created <= *current_time => {}
            _ => newest = Some((created, api_key)),
        }
    }
    newest.map(|(_, key)| key)
}

fn model_registry_root() -> PathBuf {
    std::env::var("HOM_MODEL_REGISTRY_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("provider-model-snapshots")
        })
}

fn store_provider_snapshot(provider_id: &str, models: &[Value]) -> std::io::Result<PathBuf> {
    let root = model_registry_root();
    fs::create_dir_all(&root)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let snapshot_models = models
        .iter()
        .map(|model| {
            json!({
                "id": model.get("id").cloned().unwrap_or_else(|| json!("")),
                "raw": model.get("raw_provider_payload").cloned().unwrap_or_else(|| model.clone())
            })
        })
        .collect::<Vec<_>>();
    let payload = json!({
        "provider_id": provider_id,
        "source": "live_provider_catalog",
        "fetched_at_unix": now,
        "refresh_due_days": 90,
        "status": 200,
        "ok": true,
        "model_count": models.len(),
        "models": snapshot_models,
        "secret_logged": false
    });
    let path = root.join(format!("{provider_id}.json"));
    fs::write(&path, serde_json::to_vec_pretty(&payload)?)?;
    Ok(path)
}

fn stored_snapshot_model_rows(provider_id: &str) -> Option<(Vec<Value>, Value)> {
    let root = model_registry_root();
    let path = root.join(format!("{provider_id}.json"));
    let payload: Value = fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())?;
    if payload.get("ok").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let items = payload.get("models")?.as_array()?;
    let mut seen = BTreeSet::new();
    let mut models = Vec::new();
    for item in items {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !seen.insert(id.to_string()) {
            continue;
        }
        let display_name = item
            .get("raw")
            .and_then(|raw| raw.get("display_name").or_else(|| raw.get("name")))
            .and_then(Value::as_str)
            .unwrap_or(id);
        let mut row = model_row(provider_id, id, "cloud", true);
        row["name"] = json!(display_name);
        row["display_name"] = json!(display_name);
        row["source"] = json!("live_provider_catalog");
        row["registry_source"] = json!("live_provider_catalog");
        row["snapshot_state"] = json!("stored_live_snapshot");
        if let Some(raw) = item.get("raw") {
            row["raw_provider_payload"] = raw.clone();
        }
        models.push(row);
    }
    if models.is_empty() {
        return None;
    }
    let meta = json!({
        "provider_id": provider_id,
        "source": payload.get("source").cloned().unwrap_or_else(|| json!("live_provider_catalog")),
        "fetched_at": payload.get("fetched_at").cloned(),
        "refresh_due_days": payload.get("refresh_due_days").cloned().unwrap_or_else(|| json!(90)),
        "model_count": models.len(),
        "path": path.to_string_lossy(),
        "status": payload.get("status").cloned(),
        "ok": true
    });
    Some((models, meta))
}

fn live_model_rows(provider_id: &str, payload: &Value) -> Vec<Value> {
    let items = payload
        .get("data")
        .or_else(|| payload.get("models"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut seen = BTreeSet::new();
    let mut models = Vec::new();
    for item in items {
        let Some(id) = item
            .get("id")
            .or_else(|| item.get("name"))
            .and_then(Value::as_str)
            .map(|value| value.trim_start_matches("models/").to_string())
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        let display_name = item
            .get("display_name")
            .or_else(|| item.get("name"))
            .and_then(Value::as_str)
            .unwrap_or(id.as_str());
        let mut row = model_row(provider_id, &id, "cloud", true);
        row["name"] = json!(display_name);
        row["display_name"] = json!(display_name);
        row["source"] = json!("live_provider_catalog");
        row["registry_source"] = json!("live_provider_catalog");
        row["snapshot_state"] = json!("live_discovered");
        row["raw_provider_payload"] = item;
        models.push(row);
    }
    models
}

fn credential_state(entry: &ProviderCatalogEntry, request_key: Option<&str>) -> &'static str {
    if !entry.credential_required {
        return "not_required";
    }
    if request_key
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        return "request_supplied";
    }
    if entry
        .api_key_env
        .and_then(|key| std::env::var(key).ok())
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        return "env_available";
    }
    "missing"
}

fn provider_kind_name(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Local => "local",
        ProviderKind::ApiKey => "api_key",
        ProviderKind::OAuth => "oauth",
        ProviderKind::Custom => "custom",
    }
}

fn execution_location_for(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Local => "local",
        _ => "cloud",
    }
}

fn wire_mode_name(mode: WireMode) -> &'static str {
    match mode {
        WireMode::OllamaNative => "ollama_native",
        WireMode::OpenAiChatCompletions => "openai_chat_completions",
        WireMode::OpenAiResponses => "openai_responses",
        WireMode::AnthropicMessages => "anthropic_messages",
        WireMode::GoogleGemini => "google_gemini",
        WireMode::CodexOAuthRuntime => "codex_oauth_runtime",
    }
}

fn wire_mode_from_body(body: &Value, fallback: WireMode) -> WireMode {
    match string_field(body, &["wire_mode", "wireMode", "wire_api", "wireApi"]).as_deref() {
        Some("openai_responses") | Some("responses") => WireMode::OpenAiResponses,
        Some("anthropic_messages") | Some("anthropic") => WireMode::AnthropicMessages,
        Some("google_gemini") | Some("gemini") => WireMode::GoogleGemini,
        Some("ollama_native") | Some("ollama") => WireMode::OllamaNative,
        Some("codex_oauth_runtime") | Some("codex") => WireMode::CodexOAuthRuntime,
        Some("openai_chat_completions") | Some("chat_completions") | Some("openai") => {
            WireMode::OpenAiChatCompletions
        }
        _ => fallback,
    }
}

fn request_api_key(body: &Value) -> Option<String> {
    string_field(
        body,
        &["api_key", "apiKey", "provider_api_key", "providerApiKey"],
    )
}

fn request_base_url(body: &Value) -> Option<String> {
    string_field(body, &["base_url", "baseURL", "baseUrl", "endpoint"])
}

fn resolve_catalog_base_url(entry: &ProviderCatalogEntry, body: &Value) -> Option<String> {
    request_base_url(body)
        .or_else(|| entry.base_url_env.and_then(|key| std::env::var(key).ok()))
        .or_else(|| entry.default_base_url.map(str::to_string))
}

fn resolve_catalog_api_key(entry: &ProviderCatalogEntry, body: &Value) -> Option<String> {
    request_api_key(body)
        .or_else(|| {
            string_field(body, &["credential_ref", "credentialRef"])
                .and_then(|credential_ref| load_provider_credential(entry.id, &credential_ref))
        })
        .or_else(|| load_latest_provider_credential(entry.id))
        .or_else(|| entry.api_key_env.and_then(|key| std::env::var(key).ok()))
        .filter(|value| !value.trim().is_empty())
}

fn safe_provider_extra_body(body: &Value) -> Value {
    let mut extra = serde_json::Map::new();
    for key in [
        "temperature",
        "top_p",
        "topP",
        "reasoning_effort",
        "reasoningEffort",
        "chat_template_kwargs",
        "provider_options",
        "providerOptions",
    ] {
        if let Some(value) = body.get(key).cloned() {
            let normalized = match key {
                "topP" => "top_p",
                "providerOptions" => "provider_options",
                "reasoningEffort" => "reasoning_effort",
                _ => key,
            };
            extra.insert(normalized.to_string(), value);
        }
    }
    Value::Object(extra)
}

fn provider_row(
    id: &str,
    name: &str,
    kind: &str,
    state: &str,
    connection_kind: &str,
    models: Vec<Value>,
    base_url: Option<String>,
) -> Value {
    let model_count = models.len();
    let model_ids = models
        .iter()
        .filter_map(|model| model.get("id").and_then(Value::as_str).map(str::to_string))
        .collect::<Vec<_>>();
    let routeable = state == "available" && model_count > 0;
    json!({
        "id": id,
        "provider_id": id,
        "name": name,
        "label": name,
        "type": kind,
        "kind": kind,
        "state": state,
        "aggregate_state": state,
        "connection_kind": connection_kind,
        "models_available": model_count,
        "models": models,
        "routeable": routeable,
        "model_selector_eligible": routeable,
        "enabled": true,
        "available": state == "available",
        "health": {
            "status": state,
            "model_count": model_count,
            "models": model_ids
        },
        "auth_contract": {
            "provider_id": id,
            "auth_method": kind,
            "reference_kind": if kind == "local" { "local_endpoint" } else { "credential_ref" },
            "storage_authority": if kind == "local" { "local_endpoint_config" } else { "keychain_or_env" },
            "secret_material_accepted": false
        },
        "base_url": base_url
    })
}

fn model_row(provider_id: &str, id: &str, execution_location: &str, can_chat: bool) -> Value {
    json!({
        "id": id,
        "model_id": id,
        "name": id,
        "provider_id": provider_id,
        "owned": "true",
        "execution_location": execution_location,
        "load_state": if can_chat { "loaded" } else { "unknown" },
        "can_chat": can_chat
    })
}

fn first_routeable_provider(providers: &[Value]) -> Option<Value> {
    providers.iter().find_map(|provider| {
        if provider["routeable"].as_bool() != Some(true) {
            return None;
        }
        let model = provider
            .get("models")
            .and_then(Value::as_array)
            .and_then(|models| models.first())?;
        Some(json!({
            "route_id": format!("{}:{}", provider["id"].as_str().unwrap_or("provider"), model["id"].as_str().unwrap_or("model")),
            "provider_id": provider["id"],
            "model_id": model["id"],
            "model": model["id"],
            "state": "available",
            "routeable": true,
            "usage_available": true,
            "reason": null
        }))
    })
}

impl StdioMcpSession {
    fn request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        write_mcp_frame(&mut self.stdin, &request)
            .map_err(|error| json!({"code": "mcp_frame_write_failed", "message": error.to_string(), "method": method}))?;
        let response = read_mcp_frame(&mut self.stdout)
            .map_err(|error| json!({"code": "mcp_frame_read_failed", "message": error.to_string(), "method": method}))?;
        if response.get("id").and_then(Value::as_u64) != Some(id) {
            return Err(
                json!({"code": "mcp_response_id_mismatch", "message": "MCP response id did not match request id", "method": method, "response": response}),
            );
        }
        if let Some(error) = response.get("error") {
            return Err(
                json!({"code": "mcp_protocol_error", "message": "MCP server returned JSON-RPC error", "method": method, "error": error}),
            );
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| json!({"code": "mcp_missing_result", "message": "MCP response did not contain result", "method": method, "response": response}))
    }
}

fn write_mcp_frame(writer: &mut ChildStdin, value: &Value) -> std::io::Result<()> {
    let body = value.to_string();
    writer.write_all(format!("Content-Length: {}\r\n\r\n", body.as_bytes().len()).as_bytes())?;
    writer.write_all(body.as_bytes())?;
    writer.flush()
}

fn read_mcp_frame(reader: &mut BufReader<ChildStdout>) -> std::io::Result<Value> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "MCP process closed stdout",
            ));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            if key.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse::<usize>().ok();
            }
        }
    }
    let length = content_length.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "MCP frame missing Content-Length header",
        )
    })?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("MCP frame JSON parse failed: {error}"),
        )
    })
}

fn mcp_session_not_connected(server_id: String, tool_name: String) -> (StatusCode, Value) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        json!({
            "ok": false,
            "outcome": "unavailable",
            "executed": false,
            "server_id": server_id,
            "tool_name": tool_name,
            "owner_service": "mcp-runtime",
            "source": "capability_mesh",
            "brain_forwarded": false,
            "last_error": {"code": "mcp_session_not_connected", "message": "MCP tool call requires a connected live session"},
            "error": {"code": "mcp_session_not_connected", "message": "MCP tool call requires a connected live session"}
        }),
    )
}

fn registry_state(count: usize, state: &str, reason: Option<&str>) -> Value {
    json!({"count": count, "state": state, "reason": reason})
}

fn mcp_tool_rows(servers: &[Value]) -> Vec<Value> {
    servers
        .iter()
        .flat_map(|server| {
            let server_id = server
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let state = server
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("configured");
            let discovered = server
                .get("discovered_tools")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let configured = server
                .get("tools")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let tools = if state == "connected" && !discovered.is_empty() {
                discovered
            } else {
                configured
            };
            tools.into_iter().map(move |tool| {
                let executable = state == "connected";
                json!({
                    "server_id": server_id,
                    "tool": tool,
                    "state": if executable { "connected" } else { "configured_not_connected" },
                    "executable": executable,
                    "reason": if executable { "MCP tool was discovered on a live connected session." } else { "MCP runtime session is not connected; this is configured tool metadata only." }
                })
            })
        })
        .collect()
}

fn capability(id: &str, label: &str, state: &str, reason: Option<&str>) -> Value {
    json!({
        "id": id,
        "label": label,
        "name": label,
        "state": state,
        "enabled": state == "available",
        "reason": reason,
        "required_action": if reason.is_some() { "inspect capability registry" } else { "" }
    })
}

fn combined_registry_items(
    tools: &[Value],
    skills: &[Value],
    plugins: &[Value],
    mcp: &[Value],
) -> Vec<Value> {
    tools
        .iter()
        .chain(skills)
        .chain(plugins)
        .chain(mcp)
        .cloned()
        .collect()
}

fn scan_skill_roots() -> Vec<Value> {
    skill_rows_from_roots(&[
        home_dir().join(".hermes/skills"),
        hom_local_dir().join("skills"),
        home_dir().join(".agents/skills"),
        home_dir().join(".codex/skills"),
    ])
}

fn skill_rows_from_roots(roots: &[PathBuf]) -> Vec<Value> {
    let mut rows = Vec::new();
    for root in roots {
        let mut root_rows = Vec::new();
        scan_skill_root(root, &mut root_rows);
        root_rows.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .cmp(&b["name"].as_str().unwrap_or_default().to_ascii_lowercase())
        });
        rows.extend(root_rows);
    }
    let mut rows = dedupe_skill_rows(rows);
    rows.truncate(SKILL_DISPLAY_LIMIT);
    rows
}

fn scan_skill_root(root: &Path, rows: &mut Vec<Value>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let skill_md = path.join("SKILL.md");
            if skill_md.exists() {
                if let Some(row) = skill_row_from_path(&skill_md) {
                    rows.push(row);
                }
            }
            scan_skill_root(&path, rows);
            continue;
        }
        if path.file_name().and_then(|s| s.to_str()) == Some("SKILL.md") {
            if let Some(row) = skill_row_from_path(&path) {
                rows.push(row);
            }
        }
    }
}

fn skill_row_from_path(skill_md: &Path) -> Option<Value> {
    let fallback_name = skill_md
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("Skill")
        .to_string();
    let descriptor = std::fs::read_to_string(skill_md).unwrap_or_default();
    let name = frontmatter_field(&descriptor, "name").unwrap_or(fallback_name);
    if is_generated_skill_label(&name) || is_benchmark_placeholder_skill(&name, &descriptor) {
        return None;
    }
    Some(json!({
        "id": stable_id_from_path(skill_md),
        "name": name,
        "label": name,
        "description": frontmatter_field(&descriptor, "description").unwrap_or_else(|| "local skill descriptor".to_string()),
        "enabled": true,
        "state": "available",
        "source_path": skill_md.display().to_string(),
        "root_path": skill_md.parent().map(|p| p.display().to_string()),
        "service_owner": "skills-runtime/capability-mesh",
        "import_state": "pending",
        "import_pending_reason": "session skill import/enable substrate is not implemented"
    }))
}

fn frontmatter_field(descriptor: &str, field: &str) -> Option<String> {
    let mut lines = descriptor.lines();
    if lines.next() != Some("---") {
        return None;
    }
    let prefix = format!("{field}:");
    for line in lines {
        if line == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix(&prefix) {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn is_generated_skill_label(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix("skill-") else {
        return false;
    };
    !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit())
}

fn is_benchmark_placeholder_skill(name: &str, descriptor: &str) -> bool {
    let lower = descriptor.to_lowercase();
    let name_lower = name.to_lowercase();
    (name_lower.starts_with("test skill ") && lower.contains("benchmark"))
        || lower.contains("benchmarking the skills registry compartment")
        || lower
            .lines()
            .any(|line| line.starts_with("tags:") && line.contains("benchmark"))
}

fn scan_manifest_root(
    root: PathBuf,
    kind: &str,
    manifest_name: &str,
    description: &str,
) -> Vec<Value> {
    let mut rows = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let manifest = if path.is_dir() {
                path.join(manifest_name)
            } else {
                path.clone()
            };
            if !manifest.exists() {
                continue;
            }
            rows.push(json!({
                "id": stable_id_from_path(&manifest),
                "name": manifest.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str()).unwrap_or(kind),
                "description": description,
                "enabled": true,
                "state": "available",
                "kind": kind,
                "source_path": manifest.display().to_string()
            }));
        }
    }
    rows
}

fn dedupe_skill_rows(rows: Vec<Value>) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    rows.into_iter()
        .filter(|row| {
            let key = row["name"]
                .as_str()
                .or_else(|| row["id"].as_str())
                .unwrap_or_default()
                .to_string();
            seen.insert(key)
        })
        .collect()
}

fn stable_id_from_path(path: &Path) -> String {
    path.display()
        .to_string()
        .replace(home_dir().to_string_lossy().as_ref(), "~")
        .replace('/', ".")
        .replace(' ', "-")
        .trim_matches('.')
        .to_ascii_lowercase()
}

fn tool_category(id: &str, kind: &str, domain: &str) -> &'static str {
    if id.starts_with("filesystem.") {
        "files"
    } else if id.starts_with("brain.") {
        "brain"
    } else if id.starts_with("web.") || id.starts_with("network.") {
        "web_network"
    } else if id.starts_with("browser.") {
        "browser"
    } else if id.starts_with("shell.") {
        "shell"
    } else if kind == "skill" || domain == "skill" {
        "skills"
    } else if kind == "mcp" || domain == "mcp" {
        "mcp"
    } else if kind == "plugin" || domain == "plugin" {
        "plugins"
    } else {
        "tools"
    }
}

fn tool_owner_service(id: &str, kind: &str, domain: &str) -> &'static str {
    if id.starts_with("brain.") {
        "hom-brain/cognitive-runtime"
    } else if id.starts_with("filesystem.") {
        "filesystem-runtime/capability-mesh"
    } else if id.starts_with("browser.") {
        "browser-runtime/capability-mesh"
    } else if id.starts_with("shell.") {
        "shell-proposal-runtime/capability-mesh"
    } else if id.starts_with("web.") || id.starts_with("network.") {
        "network-runtime/capability-mesh"
    } else if kind == "skill" || domain == "skill" {
        "skills-runtime/capability-mesh"
    } else if kind == "mcp" || domain == "mcp" {
        "mcp-runtime/capability-mesh"
    } else if kind == "plugin" || domain == "plugin" {
        "plugins-runtime/capability-mesh"
    } else {
        "tools-runtime/capability-mesh"
    }
}

fn tool_mutation_level(id: &str) -> &'static str {
    if id.ends_with(".preview") {
        "preview_only"
    } else if id.ends_with(".apply") {
        "stateful_mutation"
    } else if id == "brain.memory.save"
        || id == "brain.compaction.run"
        || id == "brain.reasoning.run"
    {
        "write"
    } else if id.starts_with("web.") || id.starts_with("network.") || id.starts_with("browser.open")
    {
        "external"
    } else if id == "shell.execute" {
        "external"
    } else if id.starts_with("shell.") {
        "proposal_only"
    } else {
        "read"
    }
}

fn tool_approval_required(id: &str) -> bool {
    matches!(
        tool_mutation_level(id),
        "write" | "destructive" | "stateful_mutation"
    ) || id.starts_with("browser.open")
        || id == "shell.execute"
}

fn tool_preview_required(id: &str) -> bool {
    id.ends_with(".apply")
}

fn tool_route(id: &str) -> &'static str {
    match id {
        "brain.memory.recall" => "/api/ui/recall",
        "brain.recall.smart" => "/api/ui/recall/smart",
        "brain.recall.detail" => "/api/ui/recall/:id",
        "brain.memory.save" => "/api/ui/memory/save",
        "brain.memory.detail" => "/api/ui/memory/:id",
        "brain.ledger.verify" => "/api/ui/ledger/verify",
        "brain.ledger.inspect" => "/api/ui/inspector/ledger/:id",
        "brain.compaction.snapshot" => "/api/ui/session/compaction-snapshot",
        "brain.compaction.run" => "/api/ui/session/compact",
        "brain.reasoning.run" => "/api/ui/reasoning",
        "brain.reasoning.trace.list" => "/api/ui/reasoning",
        "brain.reasoning.trace.detail" => "/api/ui/inspector/reasoning/:id",
        "brain.reasoning.policy.get" => "/api/ui/reasoning-policy",
        "brain.confidence.snapshot" => "/api/ui/inspector/confidence/snapshot",
        "brain.continuity.snapshot" => "/api/ui/session/compaction-snapshot",
        "brain.route_certificate.inspect" => "/api/ui/inspector/route-certificate/:id",
        "brain.route_certificate.validate" => "/api/ui/inspector/route-certificate/:id/validate",
        "brain.tool_trace.list" => "/api/ui/events",
        "brain.tool_trace.detail" => "/api/ui/inspector/tool-trace/:id",
        "brain.benchmark.evidence.list" => "/api/ui/benchmarks",
        "brain.cognitive_audit.snapshot" => "/api/ui/inspector/cognitive-audit/snapshot",
        "skill.registry.list" => "/api/ui/skills",
        "plugin.registry.list" => "/api/ui/plugins",
        "mcp.registry.list" => "/api/ui/mcp-servers",
        _ => "/api/ui/tools/call",
    }
}

fn tool_durability_class(id: &str) -> &'static str {
    match id {
        "filesystem.write.preview" | "filesystem.edit.preview" | "filesystem.patch.preview" => {
            "restart_durable_stateful_preview"
        }
        "filesystem.write.apply" | "filesystem.edit.apply" | "filesystem.patch.apply" => {
            "restart_durable_stateful_mutation"
        }
        id if id.starts_with("brain.") => "brain_durable_cognitive",
        id if id.starts_with("filesystem.read")
            || id.starts_with("filesystem.list")
            || id.starts_with("filesystem.search") =>
        {
            "stateless_read_only"
        }
        "shell.propose" => "stateless_proposal_only",
        "shell.execute" => "backend_subprocess",
        _ => "stateless_runtime_call",
    }
}

fn tool_restart_survival(id: &str) -> bool {
    matches!(
        tool_durability_class(id),
        "restart_durable_stateful_preview"
            | "restart_durable_stateful_mutation"
            | "brain_durable_cognitive"
    )
}

fn tool_compaction_survival(id: &str) -> bool {
    tool_restart_survival(id)
}

fn tool_preview_route(id: &str) -> Option<&'static str> {
    if id.ends_with(".preview") || id.ends_with(".apply") {
        Some("/api/ui/tools/call")
    } else {
        None
    }
}

fn tool_apply_route(id: &str) -> Option<&'static str> {
    if id.ends_with(".apply") {
        Some("/api/ui/tools/call")
    } else {
        None
    }
}

fn tool_missing_contract(id: &str, state: &str) -> Value {
    match id {
        "filesystem.write.preview" => json!({
            "status": "implemented",
            "handler": "preview_file_write",
            "execution_kind": "preview_only",
            "mutates_disk": false
        }),
        "filesystem.write.apply" => json!({
            "status": "implemented",
            "handler": "apply_file_write",
            "execution_kind": "approved_stateful_mutation",
            "requires": ["approved true", "durable preview_id lookup", "permission gate", "execution certificate"]
        }),
        "filesystem.edit.preview" => json!({
            "status": "not_implemented",
            "missing": "targeted edit preview handler that computes a diff without mutating disk",
            "required_before_enable": ["old_string match validation", "diff preview", "preview_id issuance"]
        }),
        "filesystem.edit.apply" => json!({
            "status": "not_implemented",
            "missing": "approved edit apply handler with preview_id validation, permission gate, mutation, and execution certificate",
            "required_before_enable": ["permission approval record", "preview_id lookup", "native edit handler", "execution certificate"]
        }),
        "filesystem.patch.preview" => json!({
            "status": "not_implemented",
            "missing": "multi-file patch preview handler that validates patch shape and computes file-level diffs without mutation",
            "required_before_enable": ["patch parser", "diff preview", "preview_id issuance"]
        }),
        "filesystem.patch.apply" => json!({
            "status": "not_implemented",
            "missing": "approved patch apply handler with preview_id validation, permission gate, mutation, and execution certificate",
            "required_before_enable": ["permission approval record", "preview_id lookup", "native patch handler", "execution certificate"]
        }),
        "brain.reasoning.trace.detail" => json!({
            "status": "not_implemented",
            "missing_brain_method": "reasoning.bridge.open",
            "missing_route": "/api/ui/inspector/reasoning/:id",
            "required_before_enable": ["hom-brain method availability", "ingress inspector route", "trace id normalization"]
        }),
        "brain.route_certificate.validate" => json!({
            "status": "not_implemented",
            "missing_brain_method": "route.certificate.validate",
            "missing_route": "/api/ui/inspector/route-certificate/:id/validate",
            "required_before_enable": ["hom-brain validation method", "ingress validation route", "validation result envelope"]
        }),
        "brain.tool_trace.detail" => json!({
            "status": "not_implemented",
            "missing_brain_method": "events.open",
            "missing_route": "/api/ui/inspector/tool-trace/:id",
            "required_before_enable": ["hom-brain event detail method", "ingress inspector route", "trace id normalization"]
        }),
        _ if matches!(state, "available" | "enabled") => Value::Null,
        _ => json!({
            "status": "not_implemented",
            "missing": "runtime route or handler is not enabled for this descriptor"
        }),
    }
}

fn tool_unavailable_reason(id: &str, state: &str) -> Option<&'static str> {
    if matches!(state, "available" | "enabled") {
        None
    } else if id.starts_with("filesystem.")
        && (id.contains(".write.") || id.contains(".edit.") || id.contains(".patch."))
    {
        Some(
            "preview/apply file mutation substrate is declared for Phase 4 but execution is not implemented yet",
        )
    } else if id.starts_with("brain.") {
        Some(
            "cognitive brain route descriptor exists, but the live route/method must be verified before enabling execution",
        )
    } else if id.starts_with("browser.") {
        Some("agent browser runtime is not available")
    } else {
        Some("runtime capability is not implemented or not routeable yet")
    }
}

fn search_filesystem(args: Value) -> Result<Value, (StatusCode, Value)> {
    let root = PathBuf::from(required_absolute_path(&args)?);
    let query = required_string(&args, "query")?;
    let mode = string_field(&args, &["mode"]).unwrap_or_else(|| "content".to_string());
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .min(200) as usize;
    if !matches!(mode.as_str(), "files" | "content") {
        return Err(error_payload(
            StatusCode::BAD_REQUEST,
            -32602,
            "invalid_search_mode",
            json!({"mode": mode, "allowed": ["files", "content"]}),
        ));
    }
    let mut matches = Vec::new();
    search_filesystem_visit(&root, &root, &query, &mode, limit, &mut matches);
    Ok(json!({
        "root": root.display().to_string(),
        "query": query,
        "mode": mode,
        "matches": matches,
        "total": matches.len(),
        "executed": true
    }))
}

fn search_filesystem_visit(
    root: &Path,
    path: &Path,
    query: &str,
    mode: &str,
    limit: usize,
    matches: &mut Vec<Value>,
) {
    if matches.len() >= limit {
        return;
    }
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if matches!(
                name.as_str(),
                ".git" | "target" | "node_modules" | ".next" | "dist"
            ) {
                continue;
            }
            search_filesystem_visit(root, &entry.path(), query, mode, limit, matches);
            if matches.len() >= limit {
                break;
            }
        }
        return;
    }
    if !metadata.is_file() || metadata.len() > MAX_READ_BYTES {
        return;
    }
    let path_text = path.display().to_string();
    if mode == "files" {
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .contains(query)
            || path_text.contains(query)
        {
            matches.push(json!({"path": path_text, "kind": "file_name"}));
        }
        return;
    }
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    for (index, line) in content.lines().enumerate() {
        if line.contains(query) {
            matches.push(json!({
                "path": path_text,
                "relative_path": path.strip_prefix(root).ok().and_then(|p| p.to_str()).unwrap_or_default(),
                "line": index + 1,
                "preview": line.chars().take(240).collect::<String>()
            }));
            if matches.len() >= limit {
                return;
            }
        }
    }
}

fn execute_shell_command(args: Value) -> Result<Value, (StatusCode, Value)> {
    let command = required_string(&args, "command")?;
    let cwd = string_field(&args, &["cwd"])
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    if !cwd.is_absolute() {
        return Err(error_payload(
            StatusCode::BAD_REQUEST,
            -32602,
            "cwd_must_be_absolute",
            json!({"cwd": cwd.display().to_string()}),
        ));
    }
    let started_ms = now_ms();
    let output = Command::new("/bin/sh")
        .arg("-lc")
        .arg(&command)
        .current_dir(&cwd)
        .output()
        .map_err(|error| {
            error_payload(
                StatusCode::BAD_GATEWAY,
                -32090,
                "shell_execute_failed",
                json!({"command": command, "cwd": cwd.display().to_string(), "error": error.to_string(), "executed": false}),
            )
        })?;
    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok(json!({
        "command": command,
        "cwd": cwd.display().to_string(),
        "stdout": stdout,
        "stderr": stderr,
        "exit_code": exit_code,
        "success": output.status.success(),
        "executed": true,
        "execution_status": if output.status.success() { "executed" } else { "failed" },
        "started_at_ms": started_ms,
        "completed_at_ms": now_ms()
    }))
}

fn read_file_chunk(args: Value) -> Result<Value, (StatusCode, Value)> {
    let path = required_absolute_path(&args)?;
    let offset = bounded_u64(&args, "offset", 0, u64::MAX);
    let max_bytes = bounded_u64(&args, "max_bytes", DEFAULT_READ_BYTES, MAX_READ_BYTES);
    let data = std::fs::read(&path).map_err(|error| {
        error_payload(
            StatusCode::BAD_REQUEST,
            -32050,
            "file_read_failed",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    let start = offset.min(data.len() as u64) as usize;
    let end = (start as u64 + max_bytes).min(data.len() as u64) as usize;
    Ok(json!({
        "path": path,
        "size_bytes": data.len(),
        "offset": offset,
        "bytes_read": end.saturating_sub(start),
        "text": String::from_utf8_lossy(&data[start..end]).to_string(),
        "truncated": end < data.len(),
        "next_offset": if end < data.len() { json!(end) } else { Value::Null },
        "executed": true
    }))
}

fn list_directory(args: Value) -> Result<Value, (StatusCode, Value)> {
    let path = required_absolute_path(&args)?;
    let limit = bounded_u64(&args, "limit", 100, 500) as usize;
    let entries = std::fs::read_dir(&path).map_err(|error| {
        error_payload(
            StatusCode::BAD_REQUEST,
            -32050,
            "directory_list_failed",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    let mut items = Vec::new();
    let mut total = 0usize;
    for entry in entries.flatten() {
        total += 1;
        if items.len() >= limit {
            continue;
        }
        let path = entry.path();
        let metadata = entry.metadata().ok();
        items.push(json!({
            "name": entry.file_name().to_string_lossy(),
            "path": path.display().to_string(),
            "kind": if metadata.as_ref().is_some_and(|m| m.is_dir()) { "directory" } else if metadata.as_ref().is_some_and(|m| m.is_file()) { "file" } else { "other" },
            "size_bytes": metadata.map(|m| m.len())
        }));
    }
    Ok(json!({"path": path, "entries": items, "total": total, "truncated": total > limit}))
}

fn normalize_memory_open_args(mut args: Value) -> Value {
    if args.get("id").is_none() {
        if let Some(memory_id) = args
            .get("memory_id")
            .or_else(|| args.get("memoryId"))
            .cloned()
        {
            args["id"] = memory_id;
        }
    }
    args
}

fn normalize_route_certificate_args(mut args: Value) -> Value {
    if args.get("id").is_none() {
        if let Some(route_certificate_id) = args
            .get("route_certificate_id")
            .or_else(|| args.get("routeCertificateId"))
            .cloned()
        {
            args["id"] = route_certificate_id;
        }
    }
    args
}

async fn brain_tool(
    brain: &BrainClient,
    method: &str,
    args: Value,
    scope: &str,
) -> Result<Value, (StatusCode, Value)> {
    let mut params = args;
    if method == "memory.save" {
        if params.get("value").is_none() {
            if let Some(content) = params
                .get("content")
                .or_else(|| params.get("text"))
                .cloned()
            {
                params["value"] = content;
            }
        }
        if params.get("source").is_none() {
            params["source"] = json!("capability_mesh");
        }
        if params.get("memory_type").is_none() {
            params["memory_type"] = json!("declarative");
        }
    }
    brain
        .call(method, params, scope)
        .await
        .map_err(brain_error_response)
}

fn observability_trace(error: &BrainClientError) -> Value {
    trace_event(
        "tool_observability",
        "record",
        false,
        json!({
            "policy": "fail_open_with_trace_note",
            "error": error.error_payload()
        }),
    )
}

fn run_agent_browser(args: &[&str]) -> Result<Value, (StatusCode, Value)> {
    let command =
        std::env::var("HOM_AGENT_BROWSER_BIN").unwrap_or_else(|_| "agent-browser".to_string());
    let output = Command::new(&command)
        .args(args)
        .output()
        .map_err(|error| {
            error_payload(
                StatusCode::SERVICE_UNAVAILABLE,
                -32090,
                "browser_runtime_missing",
                json!({"command": command, "error": error.to_string()}),
            )
        })?;
    if !output.status.success() {
        return Err(error_payload(
            StatusCode::BAD_GATEWAY,
            -32050,
            "browser_runtime_failed",
            json!({"status": output.status.code(), "stderr": String::from_utf8_lossy(&output.stderr).to_string()}),
        ));
    }
    Ok(json!({
        "command": command,
        "args": args,
        "stdout": String::from_utf8_lossy(&output.stdout).trim(),
        "stderr": String::from_utf8_lossy(&output.stderr).trim()
    }))
}

fn agent_browser_available() -> bool {
    if let Ok(path) = std::env::var("HOM_AGENT_BROWSER_BIN") {
        return Path::new(&path).exists();
    }
    Command::new("agent-browser")
        .arg("--help")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn required_plugin_id(args: &Value) -> Result<String, (StatusCode, Value)> {
    let id = required_string(args, "id")?;
    validate_plugin_id(&id)?;
    Ok(id)
}

fn required_plugin_ref(args: &Value) -> Result<String, (StatusCode, Value)> {
    let id = args
        .get("plugin_id")
        .or_else(|| args.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            error_payload(
                StatusCode::BAD_REQUEST,
                -32602,
                "missing_required_field",
                json!({"field": "plugin_id"}),
            )
        })?;
    validate_plugin_id(&id)?;
    Ok(id)
}

fn validate_plugin_id(id: &str) -> Result<(), (StatusCode, Value)> {
    let valid = id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'));
    if valid {
        Ok(())
    } else {
        Err(error_payload(
            StatusCode::BAD_REQUEST,
            -32602,
            "invalid_plugin_id",
            json!({"plugin_id": id, "allowed": "ascii alphanumeric plus '.', '-', '_'"}),
        ))
    }
}

fn normalize_plugin_install_record(args: &Value, id: &str) -> Result<Value, (StatusCode, Value)> {
    let name = string_field(args, &["name"]).unwrap_or_else(|| id.to_string());
    let version = string_field(args, &["version"]).unwrap_or_else(|| "0.0.0".to_string());
    let description = string_field(args, &["description"]).unwrap_or_default();
    let permissions = args
        .get("permissions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let entrypoint = args.get("entrypoint").cloned().unwrap_or_else(|| json!({}));
    Ok(json!({
        "id": id,
        "name": name,
        "version": version,
        "description": description,
        "enabled": true,
        "state": "installed",
        "kind": "plugin",
        "source": "plugin_store",
        "lifecycle_source": "plugin_store",
        "entrypoint": entrypoint,
        "permissions": permissions,
        "installed_at": now_ms(),
        "updated_at": now_ms()
    }))
}

fn required_string(args: &Value, key: &str) -> Result<String, (StatusCode, Value)> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .ok_or_else(|| {
            error_payload(
                StatusCode::BAD_REQUEST,
                -32602,
                "missing_required_field",
                json!({"key": key}),
            )
        })
}

fn required_absolute_path(args: &Value) -> Result<String, (StatusCode, Value)> {
    let path = required_string(args, "path")?;
    if !path.starts_with('/') {
        return Err(error_payload(
            StatusCode::BAD_REQUEST,
            -32602,
            "absolute_path_required",
            json!({"path": path}),
        ));
    }
    Ok(path)
}

fn required_http_url(args: &Value) -> Result<String, (StatusCode, Value)> {
    let url = required_string(args, "url")?;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(error_payload(
            StatusCode::BAD_REQUEST,
            -32602,
            "http_url_required",
            json!({"url": url}),
        ));
    }
    Ok(url)
}

fn bounded_u64(args: &Value, key: &str, default: u64, max: u64) -> u64 {
    args.get(key)
        .and_then(Value::as_u64)
        .unwrap_or(default)
        .min(max)
}

fn permission_context(domain: &str, args: &Value) -> Value {
    match domain {
        "filesystem" => json!({"path": args.get("path").cloned().unwrap_or(Value::Null)}),
        "network" => json!({"url": args.get("url").cloned().unwrap_or(Value::Null)}),
        "shell" => {
            json!({"command": args.get("command").cloned().unwrap_or(Value::Null), "proposal_only": true})
        }
        "browser" => json!({"url": args.get("url").cloned().unwrap_or(Value::Null)}),
        "brain" => {
            json!({"operation": args.get("operation").cloned().unwrap_or_else(|| json!("brain.tool"))})
        }
        other => json!({"domain": other}),
    }
}

fn result_summary(result: &Value) -> Value {
    match result {
        Value::Object(map) => json!({"keys": map.keys().cloned().collect::<Vec<_>>()}),
        Value::Array(items) => json!({"items": items.len()}),
        _ => json!({"kind": "scalar"}),
    }
}

fn summarize_input(input: &Value) -> Value {
    match input {
        Value::Object(map) => {
            let mut summary = serde_json::Map::new();
            for (key, value) in map {
                let safe_value = match key.as_str() {
                    "api_key" | "token" | "authorization" | "password" | "secret" => {
                        json!("[redacted]")
                    }
                    _ => summarize_scalar(value),
                };
                summary.insert(key.clone(), safe_value);
            }
            Value::Object(summary)
        }
        _ => summarize_scalar(input),
    }
}

fn summarize_scalar(value: &Value) -> Value {
    match value {
        Value::String(text) => {
            let clipped = if text.chars().count() > 180 {
                format!("{}…", text.chars().take(180).collect::<String>())
            } else {
                text.clone()
            };
            json!(clipped)
        }
        Value::Array(items) => json!({"items": items.len()}),
        Value::Object(map) => json!({"keys": map.keys().cloned().collect::<Vec<_>>()}),
        other => other.clone(),
    }
}

fn trace_event(tool_id: &str, phase: &str, ok: bool, detail: Value) -> Value {
    json!({"ts": now_ms(), "tool_id": tool_id, "phase": phase, "ok": ok, "detail": detail})
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn messages_from_body(body: &Value) -> Option<Vec<Value>> {
    if let Some(messages) = body
        .get("messages")
        .and_then(Value::as_array)
        .map(|messages| {
            messages
                .iter()
                .filter_map(|message| {
                    Some(json!({
                        "role": message.get("role").and_then(Value::as_str)?,
                        "content": message.get("content").and_then(Value::as_str)?
                    }))
                })
                .collect::<Vec<_>>()
        })
        .filter(|messages| !messages.is_empty())
    {
        return Some(messages);
    }
    string_field(body, &["prompt"]).map(|prompt| vec![json!({"role": "user", "content": prompt})])
}

fn skill_names_from_body(body: &Value) -> Vec<String> {
    body.get("skill_names")
        .or_else(|| body.get("skillNames"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn permission_policy_instruction(policy: &Value) -> String {
    let profile = policy
        .get("profile")
        .or_else(|| policy.pointer("/policy/profile"))
        .and_then(Value::as_str)
        .unwrap_or("sandbox");
    let policy_body = policy.get("policy").unwrap_or(policy);
    let network_enabled = policy_body
        .get("network_enabled")
        .or_else(|| policy_body.get("networkEnabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let shell_access = policy_body
        .get("shell_access")
        .or_else(|| policy_body.get("shellAccess"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let automation_enabled = policy_body
        .get("automation_enabled")
        .or_else(|| policy_body.get("automationEnabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let provider_key_access = policy_body
        .get("provider_key_access")
        .or_else(|| policy_body.get("providerKeyAccess"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    format!(
        "HOM active permission policy is backend truth from permission-sandbox-service. Current profile: {profile}. Allowed domains: memory/recall/skills always allowed; network/web={network_enabled}; shell={shell_access}; browser/app automation={automation_enabled}; provider credentials={provider_key_access}. When answering, do not claim authorization beyond this policy. If a requested action needs a denied domain, say it is not authorized under the current HOM permission profile and request the appropriate mode change. Tool execution is still enforced by the backend gate; never claim a tool executed unless a HOM tool trace exists. Policy snapshot: {policy}",
    )
}

fn chat_messages(messages: &[Value]) -> Vec<ChatMessage> {
    messages
        .iter()
        .filter_map(|message| {
            Some(ChatMessage {
                role: message.get("role").and_then(Value::as_str)?.to_string(),
                content: message.get("content").and_then(Value::as_str)?.to_string(),
            })
        })
        .collect()
}

struct RecallPreflight {
    query: String,
    temporal_hint: Option<&'static str>,
}

fn recall_preflight(messages: &[Value]) -> Option<RecallPreflight> {
    let text = messages
        .iter()
        .rev()
        .find(|message| message["role"].as_str() == Some("user"))
        .and_then(|message| message["content"].as_str())?;
    let lowered = text.to_ascii_lowercase();
    let wants_memory = [
        "recall",
        "remember",
        "memory",
        "session",
        "yesterday",
        "previous",
    ]
    .iter()
    .any(|needle| lowered.contains(needle));
    if !wants_memory {
        return None;
    }

    let temporal_hint = if lowered.contains("yesterday") {
        Some("yesterday")
    } else if lowered.contains("previous") || lowered.contains("last session") {
        Some("previous_session")
    } else {
        None
    };
    let query = if temporal_hint == Some("yesterday") {
        format!(
            "{}\nTemporal target: yesterday ({}) in the local operator timezone. Return session timeline evidence.",
            text,
            yesterday_local_day()
        )
    } else {
        text.to_string()
    };
    Some(RecallPreflight {
        query,
        temporal_hint,
    })
}

fn strict_tool_call(content: &str) -> Option<StrictToolCall> {
    let trimmed = content.trim();
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return None;
    }
    let value: Value = serde_json::from_str(trimmed).ok()?;
    let call = value.get("tool_call")?.as_object()?;
    let tool_id = call.get("tool_id")?.as_str()?.trim().to_string();
    let args = call.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let descriptor_hash = call
        .get("descriptor_hash")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    Some(StrictToolCall {
        tool_id,
        descriptor_hash,
        arguments: args,
    })
}

fn claims_untraced_tool_use(content: &str) -> bool {
    let lowered = content.to_ascii_lowercase();
    [
        "i used the tool",
        "i used a tool",
        "i called the tool",
        "i read the file",
        "i opened",
        "i searched",
        "i ran",
        "i accessed the browser",
        "i recalled",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn answer_gate(
    content: &str,
    tool_traces: &[Value],
    recall_preflight: Option<&RecallPreflight>,
) -> Value {
    let mut required = Vec::new();
    let mut missing = Vec::new();
    let mut reasons = Vec::new();

    if recall_preflight.is_some() {
        required.push("brain.memory.recall");
        if !has_trace(tool_traces, "brain.memory.recall") {
            missing.push("brain.memory.recall");
            reasons.push("memory_recall_trace_missing");
        }
    }
    if claims_memory_recall(content) && !has_successful_trace(tool_traces, "brain.memory.recall") {
        required.push("brain.memory.recall:success");
        missing.push("brain.memory.recall:success");
        reasons.push("successful_memory_recall_trace_missing");
    }
    if claims_browser_or_web(content)
        && !has_any_successful_trace(tool_traces, &["browser.", "web."])
    {
        required.push("browser_or_web");
        missing.push("browser_or_web");
        reasons.push("browser_or_web_trace_missing");
    }
    if claims_search(content)
        && !has_any_successful_trace(tool_traces, &["browser.", "web.", "brain.memory.recall"])
    {
        required.push("search");
        missing.push("search");
        reasons.push("search_trace_missing");
    }
    if claims_file_read(content) && !has_any_successful_trace(tool_traces, &["filesystem."]) {
        required.push("filesystem");
        missing.push("filesystem");
        reasons.push("filesystem_trace_missing");
    }
    if claims_shell_execution(content) && !has_successful_shell_execution_trace(tool_traces) {
        required.push("shell");
        missing.push("shell");
        reasons.push("shell_execution_trace_missing");
    }
    if claims_untraced_tool_use(content) && tool_traces.is_empty() {
        required.push("tool_trace");
        missing.push("tool_trace");
        reasons.push("generic_tool_trace_missing");
    }

    json!({
        "allowed": missing.is_empty(),
        "required_traces": required,
        "missing_traces": missing,
        "fail_reason": if reasons.is_empty() { Value::Null } else { json!(reasons.join(",")) },
        "confidence": if missing.is_empty() { 0.84 } else { 0.0 },
        "trace_count": tool_traces.len(),
        "gate": "trace_backed_answer_gate_v1"
    })
}

fn has_successful_shell_execution_trace(tool_traces: &[Value]) -> bool {
    tool_traces.iter().any(|trace| {
        trace
            .get("tool_id")
            .and_then(Value::as_str)
            .is_some_and(|tool_id| tool_id.starts_with("shell."))
            && trace.get("ok").and_then(Value::as_bool) == Some(true)
            && trace.get("executed").and_then(Value::as_bool) == Some(true)
    })
}

fn has_successful_trace(tool_traces: &[Value], tool_id: &str) -> bool {
    tool_traces.iter().any(|trace| {
        trace.get("tool_id").and_then(Value::as_str) == Some(tool_id)
            && trace.get("ok").and_then(Value::as_bool) == Some(true)
    })
}

fn has_trace(tool_traces: &[Value], tool_id: &str) -> bool {
    tool_traces
        .iter()
        .any(|trace| trace.get("tool_id").and_then(Value::as_str) == Some(tool_id))
}

fn has_any_successful_trace(tool_traces: &[Value], prefixes: &[&str]) -> bool {
    tool_traces.iter().any(|trace| {
        let Some(tool_id) = trace.get("tool_id").and_then(Value::as_str) else {
            return false;
        };
        trace.get("ok").and_then(Value::as_bool) == Some(true)
            && prefixes.iter().any(|prefix| tool_id.starts_with(prefix))
    })
}

fn claims_memory_recall(content: &str) -> bool {
    let lowered = content.to_ascii_lowercase();
    [
        "i recalled",
        "i searched my saved memories",
        "saved memories show",
        "memory records show",
        "session log shows",
        "the recalled",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn claims_browser_or_web(content: &str) -> bool {
    let lowered = content.to_ascii_lowercase();
    [
        "i searched the web",
        "i searched google",
        "i searched online",
        "i opened",
        "i opened the browser",
        "browser snapshot",
        "web search showed",
        "i fetched",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn claims_search(content: &str) -> bool {
    let lowered = content.to_ascii_lowercase();
    [
        "i searched",
        "i looked up",
        "search results",
        "searched results",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn claims_file_read(content: &str) -> bool {
    let lowered = content.to_ascii_lowercase();
    [
        "i read the file",
        "the file contains",
        "filesystem read",
        "i opened the file",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn claims_shell_execution(content: &str) -> bool {
    let lowered = content.to_ascii_lowercase();
    [
        "i ran",
        "i executed",
        "i ran the command",
        "command output",
        "shell output",
        "terminal output",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn yesterday_local_day() -> String {
    let yesterday = chrono::Local::now() - chrono::Duration::days(1);
    yesterday.format("%Y-%m-%d").to_string()
}

fn bad_request(message: &str, data: Value) -> (StatusCode, Value) {
    error_payload(StatusCode::BAD_REQUEST, -32602, message, data)
}

fn not_found(message: &str, data: Value) -> (StatusCode, Value) {
    error_payload(StatusCode::NOT_FOUND, -32044, message, data)
}

fn error_payload(status: StatusCode, code: i64, message: &str, data: Value) -> (StatusCode, Value) {
    (
        status,
        json!({"ok": false, "error": {"code": code, "message": message, "data": data}}),
    )
}

fn provider_error_payload(error: ProviderError) -> (StatusCode, Value) {
    let status = if error.retryable {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    error_payload(
        status,
        -32090,
        error.code.as_str(),
        json!({"message": error.message, "retryable": error.retryable}),
    )
}

fn provider_transport_error(error: reqwest::Error) -> (StatusCode, Value) {
    error_payload(
        StatusCode::SERVICE_UNAVAILABLE,
        -32090,
        "provider_transport_unavailable",
        json!({"message": error.to_string(), "retryable": true}),
    )
}

fn brain_error_response(error: BrainClientError) -> (StatusCode, Value) {
    let status = match error {
        BrainClientError::Rpc(-32080, _, _) => StatusCode::FORBIDDEN,
        BrainClientError::Rpc(-32602, _, _) => StatusCode::BAD_REQUEST,
        BrainClientError::Rpc(-32601, _, _) => StatusCode::NOT_FOUND,
        BrainClientError::Rpc(-32041, _, _) => StatusCode::NOT_FOUND,
        BrainClientError::Io(_) => StatusCode::SERVICE_UNAVAILABLE,
        BrainClientError::Sign(_) => StatusCode::INTERNAL_SERVER_ERROR,
        BrainClientError::Rpc(_, _, _) => StatusCode::BAD_GATEWAY,
    };
    (status, json!({"ok": false, "error": error.error_payload()}))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn hom_local_dir() -> PathBuf {
    std::env::var("HOM_LOCAL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".hom/local"))
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_body_and_skill_names_are_accepted_from_ui_chat() {
        let body = json!({
            "prompt": "Use the notes skill",
            "skill_names": ["apple-notes", "frontend-architect"]
        });

        let messages = messages_from_body(&body).unwrap();
        let skill_names = skill_names_from_body(&body);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "Use the notes skill");
        assert_eq!(skill_names, vec!["apple-notes", "frontend-architect"]);
    }

    #[test]
    fn skill_rows_are_recursive_display_limited_real_skill_names() {
        let root = std::env::temp_dir().join(format!("hom-skill-scan-test-{}", now_ms()));
        let hermes_root = root.join(".hermes").join("skills");
        let benchmark = hermes_root.join("skill-001");
        std::fs::create_dir_all(&benchmark).unwrap();
        std::fs::write(
            benchmark.join("SKILL.md"),
            "---\nname: Test Skill 1\ntags: test, benchmark, skill-001\ndescription: benchmark placeholder\n---\n",
        )
        .unwrap();

        for index in 0..35 {
            let skill_dir = hermes_root
                .join("category")
                .join(format!("skill-{index:03}"));
            std::fs::create_dir_all(&skill_dir).unwrap();
            std::fs::write(
                skill_dir.join("SKILL.md"),
                format!(
                    "---\nname: Real Skill {index:03}\ndescription: Real local skill {index}\n---\n"
                ),
            )
            .unwrap();
        }
        let nested = hermes_root.join("frontend").join("frontend-architect");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            nested.join("SKILL.md"),
            "---\nname: frontend-architect\ndescription: Frontend architecture skill\n---\n",
        )
        .unwrap();

        let rows = skill_rows_from_roots(&[hermes_root.clone()]);

        assert_eq!(rows.len(), SKILL_DISPLAY_LIMIT);
        assert_eq!(rows[0]["name"], "frontend-architect");
        assert_eq!(rows[0]["label"], "frontend-architect");
        assert!(rows.iter().all(|row| row["name"] != "Test Skill 1"));
        assert_eq!(rows[0]["service_owner"], "skills-runtime/capability-mesh");
        assert_eq!(rows[0]["import_state"], "pending");
        assert!(
            rows[0]["source_path"]
                .as_str()
                .unwrap()
                .ends_with("SKILL.md")
        );
        assert!(rows.iter().all(|row| row["name"].as_str().is_some()));
        assert!(
            rows.iter()
                .all(|row| !row["name"].as_str().unwrap().starts_with("skill-"))
        );

        let _ = std::fs::remove_dir_all(root);
    }
}
