use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use hom_shared::{
    ERR_AUTH_MALFORMED, EnvelopeVerifier, HomEnvelope, KnownClient, hom_local_dir,
    known_clients_path, load_known_clients,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::brain_client::{BrainClient, BrainClientError};
use crate::bubble_client::BubbleClients;
use crate::capability_mesh::CapabilityMesh;
use crate::rate_limit::{LimitClass, RateLimiter};

pub struct AppState {
    pub brain: BrainClient,
    pub mesh: CapabilityMesh,
    pub bubbles: BubbleClients,
    pub permissions: Mutex<PermissionPolicy>,
    pub selected_model: Mutex<SelectedModel>,
    pub session_skills: Mutex<Vec<Value>>,
    pub session_skills_store_path: PathBuf,
    pub verifier: Mutex<EnvelopeVerifier>,
    pub ui_clients: Vec<KnownClient>,
    limiter: Mutex<RateLimiter>,
    started_at: Instant,
}

#[derive(Clone, Debug)]
pub struct PermissionPolicy {
    profile: String,
    network_enabled: bool,
    provider_key_access: bool,
    automation_enabled: bool,
    shell_access: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SelectedModel {
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub reasoning_effort: String,
}

impl SelectedModel {
    pub fn snapshot(&self) -> Value {
        json!({
            "selectedProviderId": self.provider_id,
            "selectedModelId": self.model_id,
            "reasoningEffort": self.reasoning_effort,
        })
    }

    pub fn update_from_body(&mut self, body: &Value) {
        if let Some(pid) = body
            .get("provider_id")
            .or_else(|| body.get("providerId"))
            .and_then(Value::as_str)
        {
            self.provider_id = Some(pid.to_string());
        }
        if let Some(mid) = body
            .get("model_id")
            .or_else(|| body.get("modelId"))
            .or_else(|| body.get("model"))
            .and_then(Value::as_str)
        {
            self.model_id = Some(mid.to_string());
        }
        if let Some(effort) = body
            .get("reasoning_effort")
            .or_else(|| body.get("reasoningEffort"))
            .and_then(Value::as_str)
        {
            self.reasoning_effort = effort.to_string();
        }
    }
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self::for_profile("sandbox")
    }
}

impl PermissionPolicy {
    pub fn for_profile(profile: &str) -> Self {
        match normalize_permission_profile(profile).as_str() {
            "full" => Self {
                profile: "full".to_string(),
                network_enabled: true,
                provider_key_access: true,
                automation_enabled: true,
                shell_access: true,
            },
            "review" => Self {
                profile: "review".to_string(),
                network_enabled: true,
                provider_key_access: false,
                automation_enabled: false,
                shell_access: false,
            },
            _ => Self {
                profile: "sandbox".to_string(),
                network_enabled: false,
                provider_key_access: false,
                automation_enabled: false,
                shell_access: false,
            },
        }
    }

    pub fn snapshot(&self) -> Value {
        json!({
            "ok": true,
            "profile": self.profile,
            "network_enabled": self.network_enabled,
            "provider_key_access": self.provider_key_access,
            "automation_enabled": self.automation_enabled,
            "shell_access": self.shell_access,
            "reason": "Backend permission-policy-service policy is the source of truth. This is a policy gate, not a real sandbox runner.",
            "source": "permission-policy-service",
            "security_boundary": "policy_gate_only",
            "sandbox_runner": "separate_service_required",
            "sandbox_runner_state": "not_implemented",
            "policy": {
                "profile": self.profile,
                "network_enabled": self.network_enabled,
                "provider_key_access": self.provider_key_access,
                "automation_enabled": self.automation_enabled,
                "shell_access": self.shell_access,
                "enforced_by": "permission-policy-service",
                "security_boundary": "policy_gate_only",
                "sandbox_runner": "separate_service_required",
                "sandbox_runner_state": "not_implemented"
            }
        })
    }

    pub fn allows_domain(&self, domain: &str) -> bool {
        match normalize_permission_domain(domain).as_str() {
            "brain" | "memory" | "recall" | "skill" | "plugin" | "mcp" => true,
            "network" | "web" => self.network_enabled,
            "filesystem" | "file" => self.profile == "full",
            "browser" | "automation" => self.automation_enabled,
            "shell" => self.shell_access,
            "provider_key" | "provider-key" | "provider" => self.provider_key_access,
            _ => false,
        }
    }
}

fn normalize_permission_profile(profile: &str) -> String {
    let normalized = profile.trim().to_lowercase().replace(['_', '-', ' '], "");
    match normalized.as_str() {
        "full" | "fullaccess" => "full".to_string(),
        "review" | "workspace" | "askfirst" => "review".to_string(),
        "sandbox" | "restricted" => "sandbox".to_string(),
        _ => "sandbox".to_string(),
    }
}

fn normalize_permission_domain(domain: &str) -> String {
    domain.trim().to_lowercase().replace('_', "-")
}

const SESSION_SKILLS_STORE_SCHEMA_VERSION: u64 = 1;
const SESSION_SKILLS_STORE_SCOPE_KIND: &str = "ingress_active_session";

fn default_session_skills_store_path() -> PathBuf {
    hom_local_dir().join("runtime/session-skills.json")
}

fn load_session_skills_from_store(path: &PathBuf) -> Vec<Value> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&content) else {
        return Vec::new();
    };
    if value["artifact_kind"].as_str() != Some("session_skills_store_v1")
        || value["schema_version"].as_u64() != Some(SESSION_SKILLS_STORE_SCHEMA_VERSION)
        || value["scope"]["kind"].as_str() != Some(SESSION_SKILLS_STORE_SCOPE_KIND)
    {
        return Vec::new();
    }
    value["bound_skills"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

pub fn session_skills_persistence_metadata(path: &PathBuf, state: &str) -> Value {
    json!({
        "state": state,
        "path": path.display().to_string(),
        "artifact_kind": "session_skills_store_v1",
        "schema_version": SESSION_SKILLS_STORE_SCHEMA_VERSION,
        "scope": {"kind": SESSION_SKILLS_STORE_SCOPE_KIND}
    })
}

pub fn session_skills_store_read_metadata(path: &PathBuf) -> Value {
    let Ok(content) = std::fs::read_to_string(path) else {
        return session_skills_persistence_metadata(path, "missing");
    };
    let Ok(value) = serde_json::from_str::<Value>(&content) else {
        return json!({
            "state": "corrupt",
            "path": path.display().to_string(),
            "artifact_kind": "session_skills_store_v1",
            "schema_version": SESSION_SKILLS_STORE_SCHEMA_VERSION,
            "error": {"code": "session_skill_store_corrupt_json", "message": "Session skills store is not valid JSON"}
        });
    };
    if value["artifact_kind"].as_str() != Some("session_skills_store_v1")
        || value["schema_version"].as_u64() != Some(SESSION_SKILLS_STORE_SCHEMA_VERSION)
        || value["scope"]["kind"].as_str() != Some(SESSION_SKILLS_STORE_SCOPE_KIND)
    {
        return json!({
            "state": "invalid_schema",
            "path": path.display().to_string(),
            "artifact_kind": "session_skills_store_v1",
            "schema_version": SESSION_SKILLS_STORE_SCHEMA_VERSION,
            "error": {"code": "session_skill_store_invalid_schema", "message": "Session skills store schema is missing or unsupported"}
        });
    }
    session_skills_persistence_metadata(path, "loaded")
}

pub fn persist_session_skills_to_store(
    path: &PathBuf,
    bound_skills: &[Value],
) -> Result<Value, std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = json!({
        "artifact_kind": "session_skills_store_v1",
        "schema_version": SESSION_SKILLS_STORE_SCHEMA_VERSION,
        "scope": {"kind": SESSION_SKILLS_STORE_SCOPE_KIND},
        "bound_skills": bound_skills,
        "updated_at_ms": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .to_string()
    });
    let serialized = serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".to_string());
    let temp_path = path.with_extension(format!(
        "tmp-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::write(&temp_path, serialized)?;
    std::fs::rename(&temp_path, path)?;
    Ok(session_skills_persistence_metadata(path, "persisted"))
}

impl AppState {
    pub fn from_env(brain: BrainClient) -> anyhow::Result<Self> {
        let known_clients_file = std::env::var("HOM_KNOWN_CLIENTS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| known_clients_path(hom_local_dir()));
        let known_clients = if known_clients_file.exists() {
            load_known_clients(known_clients_file)?
        } else {
            Vec::new()
        };
        Ok(Self::new(brain, known_clients))
    }

    pub fn new(brain: BrainClient, known_clients: Vec<KnownClient>) -> Self {
        Self::new_with_mesh_and_bubbles(
            brain,
            known_clients,
            CapabilityMesh::new(),
            BubbleClients::from_env(),
        )
    }

    pub fn new_with_bubbles(
        brain: BrainClient,
        known_clients: Vec<KnownClient>,
        bubbles: BubbleClients,
    ) -> Self {
        Self::new_with_mesh_and_bubbles(brain, known_clients, CapabilityMesh::new(), bubbles)
    }

    pub fn new_with_mesh(
        brain: BrainClient,
        known_clients: Vec<KnownClient>,
        mesh: CapabilityMesh,
    ) -> Self {
        Self::new_with_mesh_and_bubbles(brain, known_clients, mesh, BubbleClients::from_env())
    }

    pub fn new_with_mesh_and_session_skill_store(
        brain: BrainClient,
        known_clients: Vec<KnownClient>,
        mesh: CapabilityMesh,
        session_skills_store_path: PathBuf,
    ) -> Self {
        Self::new_with_mesh_bubbles_and_session_skill_store(
            brain,
            known_clients,
            mesh,
            BubbleClients::from_env(),
            session_skills_store_path,
        )
    }

    pub fn new_with_mesh_and_bubbles(
        brain: BrainClient,
        known_clients: Vec<KnownClient>,
        mesh: CapabilityMesh,
        bubbles: BubbleClients,
    ) -> Self {
        Self::new_with_mesh_bubbles_and_session_skill_store(
            brain,
            known_clients,
            mesh,
            bubbles,
            default_session_skills_store_path(),
        )
    }

    fn new_with_mesh_bubbles_and_session_skill_store(
        brain: BrainClient,
        known_clients: Vec<KnownClient>,
        mesh: CapabilityMesh,
        bubbles: BubbleClients,
        session_skills_store_path: PathBuf,
    ) -> Self {
        let session_skills = load_session_skills_from_store(&session_skills_store_path);
        Self {
            brain,
            mesh,
            bubbles,
            permissions: Mutex::new(PermissionPolicy::default()),
            selected_model: Mutex::new(SelectedModel::default()),
            session_skills: Mutex::new(session_skills),
            session_skills_store_path,
            verifier: Mutex::new(EnvelopeVerifier::new(known_clients.clone())),
            ui_clients: known_clients,
            limiter: Mutex::new(RateLimiter::default()),
            started_at: Instant::now(),
        }
    }

    pub async fn permission_snapshot(&self) -> Value {
        self.permissions.lock().await.snapshot()
    }

    pub async fn selected_model_snapshot(&self) -> SelectedModel {
        self.selected_model.lock().await.clone()
    }

    pub async fn update_selected_model(&self, body: &Value) -> SelectedModel {
        let mut guard = self.selected_model.lock().await;
        guard.update_from_body(body);
        guard.clone()
    }

    pub async fn set_permission_profile(&self, profile: &str) -> Value {
        let policy = PermissionPolicy::for_profile(profile);
        let mut guard = self.permissions.lock().await;
        *guard = policy;
        guard.snapshot()
    }

    pub async fn check_permission_domain(
        &self,
        domain: &str,
        context: Value,
    ) -> (StatusCode, Value) {
        let guard = self.permissions.lock().await;
        let allowed = guard.allows_domain(domain);
        let status = if allowed {
            StatusCode::OK
        } else {
            StatusCode::FORBIDDEN
        };
        (
            status,
            json!({
                "ok": allowed,
                "allowed": allowed,
                "domain": domain,
                "context": context,
                "source": "permission-policy-service",
                "security_boundary": "policy_gate_only",
                "sandbox_runner": "separate_service_required",
                "sandbox_runner_state": "not_implemented",
                "policy": guard.snapshot()["policy"],
                "reason": if allowed { "permission policy allows this domain" } else { "permission policy denies this domain for the active profile" }
            }),
        )
    }

    pub fn log_ready(&self, bind: &str) {
        eprintln!(
            "{{\"level\":\"info\",\"helper\":\"ingress\",\"event\":\"helper.ready\",\"bind\":\"{}\"}}",
            bind
        );
    }
}

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/.well-known/hom-local.json", get(well_known))
        .route(
            "/api/debug/settings-raw",
            get(crate::ui_routes::settings_raw),
        )
        .route("/api/health", get(health))
        .route("/api/ready", get(ready))
        .route("/api/verify/ledger", get(crate::ui_routes::ledger_verify))
        .route("/openapi.json", get(openapi))
        .route("/api/tools", get(crate::ui_routes::tools_list))
        .route(
            "/api/auth/request",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/auth/status",
            get(crate::ui_routes::provider_runtime_offline),
        )
        .route("/api/recall", post(recall))
        // UI routes (macOS app)
        .route("/api/ui/session", get(crate::ui_routes::session))
        .route("/api/ui/login", post(crate::ui_routes::login))
        .route("/api/ui/logout", post(crate::ui_routes::logout))
        .route("/api/ui/status", get(crate::ui_routes::status))
        .route(
            "/api/ui/runtime-status",
            get(crate::ui_routes::runtime_status),
        )
        .route("/api/ui/contract", get(crate::ui_routes::ui_contract))
        .route(
            "/api/ui/actions/chat-composer",
            get(crate::ui_routes::chat_composer_actions),
        )
        .route(
            "/api/ui/ledger/verify",
            get(crate::ui_routes::ledger_verify),
        )
        .route(
            "/api/ui/ledger/repair-segmented",
            post(crate::ui_routes::ledger_repair_segmented),
        )
        .route(
            "/api/ui/ledger/record-mutation",
            post(crate::ui_routes::ledger_record_mutation),
        )
        .route(
            "/api/ui/ledger/reconcile-mismatches",
            post(crate::ui_routes::ledger_reconcile_mismatches),
        )
        .route(
            "/api/ui/monitoring",
            get(crate::ui_routes::monitoring_snapshot),
        )
        .route(
            "/api/ui/security/status",
            get(crate::ui_routes::security_status),
        )
        .route(
            "/api/ui/security/saber-dry-run",
            get(crate::ui_routes::security_saber_dry_run),
        )
        .route(
            "/api/ui/security/canary-timeline",
            get(crate::ui_routes::security_canary_timeline),
        )
        .route(
            "/api/ui/diagnostics/capability/:id",
            get(crate::ui_routes::diagnostics_capability),
        )
        // Six-gate runtime routes
        .route("/api/ui/gates/status", get(crate::ui_routes::gates_status))
        .route(
            "/api/ui/gates/plan/current",
            get(crate::ui_routes::gates_plan_current),
        )
        .route(
            "/api/ui/gates/prompt",
            post(crate::ui_routes::gates_prompt_create),
        )
        .route(
            "/api/ui/gates/plan",
            post(crate::ui_routes::gates_plan_propose),
        )
        .route(
            "/api/ui/gates/plan/:id/approve",
            post(crate::ui_routes::gates_plan_approve),
        )
        .route(
            "/api/ui/gates/tasks/derive",
            post(crate::ui_routes::gates_tasks_derive),
        )
        .route(
            "/api/ui/gates/tasks/preflight",
            post(crate::ui_routes::gates_tasks_preflight),
        )
        .route(
            "/api/ui/gates/amendments",
            post(crate::ui_routes::gates_amendment_request),
        )
        .route(
            "/api/ui/gates/amendments/:id/approve",
            post(crate::ui_routes::gates_amendment_approve),
        )
        .route(
            "/api/ui/gates/runtime/verify",
            post(crate::ui_routes::gates_runtime_verify),
        )
        .route(
            "/api/ui/gates/decisions",
            get(crate::ui_routes::gates_decisions),
        )
        .route(
            "/api/ui/gates/evidence",
            get(crate::ui_routes::gates_evidence_list),
        )
        .route(
            "/api/ui/gates/evidence",
            post(crate::ui_routes::gates_evidence_submit),
        )
        .route(
            "/api/ui/gates/tasks/:id/complete",
            post(crate::ui_routes::gates_tasks_complete),
        )
        .route(
            "/api/ui/gates/argument/build",
            post(crate::ui_routes::gates_argument_build),
        )
        .route(
            "/api/ui/gates/argument/inspect",
            get(crate::ui_routes::gates_argument_inspect),
        )
        .route(
            "/api/ui/gates/argument/:id/accept",
            post(crate::ui_routes::gates_argument_accept),
        )
        .route("/api/ui/chat", post(crate::ui_routes::chat))
        .route(
            "/api/ui/chat/sessions",
            get(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/chat/sessions/:id",
            get(crate::ui_routes::provider_runtime_offline)
                .delete(crate::ui_routes::provider_runtime_offline),
        )
        .route("/api/ui/recall", post(crate::ui_routes::recall))
        .route("/api/ui/search", get(crate::ui_routes::search))
        .route("/api/ui/recall/smart", post(crate::ui_routes::recall_smart))
        .route("/api/ui/recall/:id", get(crate::ui_routes::recall_detail))
        .route(
            "/api/ui/session/compaction-snapshot",
            post(crate::ui_routes::compaction_snapshot),
        )
        .route(
            "/api/ui/session/compact",
            post(crate::ui_routes::session_compact),
        )
        .route("/api/ui/memory/save", post(crate::ui_routes::memory_save))
        .route("/api/ui/benchmarks", get(crate::ui_routes::benchmarks_list))
        .route(
            "/api/ui/benchmarks/:id",
            get(crate::ui_routes::benchmark_detail),
        )
        .route("/api/ui/memory/:id", get(crate::ui_routes::memory_detail))
        .route("/api/ui/events", get(crate::ui_routes::events))
        // Project routes
        .route("/api/ui/projects", get(crate::ui_routes::projects_list))
        .route("/api/ui/projects", post(crate::ui_routes::projects_create))
        .route(
            "/api/ui/projects/hierarchy",
            get(crate::ui_routes::projects_hierarchy),
        )
        // Session/workspace routes
        .route(
            "/api/ui/sessions",
            get(crate::ui_routes::sessions_list).post(crate::ui_routes::sessions_create),
        )
        .route("/api/ui/session/:id", get(crate::ui_routes::session_detail))
        .route(
            "/api/ui/sessions/:id/rename",
            post(crate::ui_routes::session_rename),
        )
        .route(
            "/api/ui/sessions/:id/active",
            post(crate::ui_routes::session_set_active),
        )
        // Nightly routes
        .route("/api/ui/nightly/tree", get(crate::ui_routes::nightly_tree))
        .route(
            "/api/ui/nightly/dry-run",
            post(crate::ui_routes::nightly_dry_run),
        )
        .route("/api/ui/nightly/run", post(crate::ui_routes::nightly_run))
        .route("/api/ui/reasoning", get(crate::ui_routes::reasoning_list))
        .route(
            "/api/ui/reasoning/bridge-integrity",
            get(crate::ui_routes::reasoning_bridge_integrity),
        )
        .route("/api/ui/reasoning", post(crate::ui_routes::reasoning))
        .route(
            "/api/ui/reasoning-policy",
            get(crate::ui_routes::reasoning_policy_get)
                .post(crate::ui_routes::reasoning_policy_set),
        )
        // Settings routes
        .route("/api/ui/settings", get(crate::ui_routes::settings_get))
        .route(
            "/api/ui/settings/config",
            post(crate::ui_routes::settings_set),
        )
        .route(
            "/api/ui/permissions",
            get(crate::ui_routes::permissions_get),
        )
        .route(
            "/api/ui/permissions/profile",
            post(crate::ui_routes::permissions_set_profile),
        )
        .route(
            "/api/ui/permissions/policy",
            get(crate::ui_routes::permissions_policy).post(crate::ui_routes::permissions_policy),
        )
        .route(
            "/api/ui/sandbox/profiles",
            get(crate::ui_routes::sandbox_profiles),
        )
        .route(
            "/api/ui/sandbox/preview",
            post(crate::ui_routes::sandbox_preview),
        )
        .route(
            "/api/ui/sandbox/execute",
            post(crate::ui_routes::sandbox_execute),
        )
        .route(
            "/api/ui/sandbox/denials",
            get(crate::ui_routes::sandbox_denials),
        )
        .route(
            "/api/ui/permissions/grant-root",
            post(crate::ui_routes::permissions_grant_root),
        )
        .route(
            "/api/ui/permissions/revoke-root",
            post(crate::ui_routes::permissions_revoke_root),
        )
        .route(
            "/api/ui/permissions/grants",
            post(crate::ui_routes::permissions_grants_create),
        )
        .route(
            "/api/ui/permissions/grants/revoke",
            post(crate::ui_routes::permissions_grants_revoke),
        )
        .route(
            "/api/ui/tool/check-gate",
            post(crate::ui_routes::tool_check_gate),
        )
        // Registry routes
        .route("/api/ui/skills", get(crate::ui_routes::skills_list))
        .route(
            "/api/ui/skills/reload",
            post(crate::ui_routes::skills_reload),
        )
        .route(
            "/api/ui/session/skills",
            get(crate::ui_routes::session_skills_list),
        )
        .route(
            "/api/ui/session/skills/import/preview",
            post(crate::ui_routes::session_skills_import_preview),
        )
        .route(
            "/api/ui/session/skills/import",
            post(crate::ui_routes::session_skills_import),
        )
        .route(
            "/api/ui/session/skills/remove",
            post(crate::ui_routes::session_skills_remove),
        )
        .route("/api/ui/tools", get(crate::ui_routes::tools_list))
        .route("/api/ui/tools/call", post(crate::ui_routes::tools_call))
        .route("/api/ui/tools/:id", get(crate::ui_routes::tools_detail))
        .route(
            "/api/ui/gate/grants",
            get(crate::ui_routes::gate_grants_list).post(crate::ui_routes::gate_grants_create),
        )
        .route("/api/ui/plugins", get(crate::ui_routes::plugins_list))
        .route(
            "/api/ui/plugins/marketplace",
            get(crate::ui_routes::plugins_marketplace),
        )
        .route(
            "/api/ui/plugins/install",
            post(crate::ui_routes::plugins_install),
        )
        .route(
            "/api/ui/plugins/enable",
            post(crate::ui_routes::plugins_enable),
        )
        .route(
            "/api/ui/plugins/disable",
            post(crate::ui_routes::plugins_disable),
        )
        .route(
            "/api/ui/plugins/uninstall",
            post(crate::ui_routes::plugins_uninstall),
        )
        .route("/api/ui/apps-skills", get(crate::ui_routes::apps_skills))
        .route("/api/ui/mcp-servers", get(crate::ui_routes::mcp_servers))
        .route("/api/ui/mcp/status", get(crate::ui_routes::mcp_status))
        .route("/api/ui/mcp/connect", post(crate::ui_routes::mcp_connect))
        .route("/api/ui/mcp/create", post(crate::ui_routes::mcp_create))
        .route(
            "/api/ui/mcp/servers/:id/enable",
            post(crate::ui_routes::mcp_server_enable),
        )
        .route(
            "/api/ui/mcp/servers/:id/disable",
            post(crate::ui_routes::mcp_server_disable),
        )
        .route(
            "/api/ui/mcp/servers/:id/connect",
            post(crate::ui_routes::mcp_server_connect),
        )
        .route(
            "/api/ui/mcp/servers/:id/tools",
            get(crate::ui_routes::mcp_tools),
        )
        .route("/api/ui/mcp/tools", get(crate::ui_routes::mcp_tools))
        .route(
            "/api/ui/mcp/tools/call",
            post(crate::ui_routes::mcp_call_tool),
        )
        .route(
            "/api/ui/inspector/:kind/:id",
            get(crate::ui_routes::inspector_target),
        )
        // App/grant routes
        .route(
            "/api/ui/apps",
            get(crate::ui_routes::plugins_runtime_offline),
        )
        .route(
            "/api/ui/apps/registry",
            get(crate::ui_routes::plugins_runtime_offline),
        )
        .route(
            "/api/ui/apps/scan-machine",
            post(crate::ui_routes::plugins_runtime_offline),
        )
        .route(
            "/api/ui/apps/:id/grant-request",
            post(crate::ui_routes::plugins_runtime_offline),
        )
        .route(
            "/api/ui/apps/:id/connect-kit",
            post(crate::ui_routes::plugins_runtime_offline),
        )
        .route(
            "/api/ui/apps/approve/:id",
            post(crate::ui_routes::plugins_runtime_offline),
        )
        .route(
            "/api/ui/apps/:id/approve",
            post(crate::ui_routes::plugins_runtime_offline),
        )
        .route(
            "/api/ui/apps/deny/:id",
            post(crate::ui_routes::plugins_runtime_offline),
        )
        .route(
            "/api/ui/apps/:id/deny",
            post(crate::ui_routes::plugins_runtime_offline),
        )
        .route(
            "/api/ui/apps/revoke/:id",
            post(crate::ui_routes::plugins_runtime_offline),
        )
        .route(
            "/api/ui/apps/:id/revoke",
            post(crate::ui_routes::plugins_runtime_offline),
        )
        // Provider management routes
        .route(
            "/api/ui/provider-candidates",
            get(crate::ui_routes::provider_runtime_offline)
                .post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-candidates/:id",
            delete(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-candidates/:id/probes",
            get(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-candidates/:id/probe",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-candidates/:id/classifications",
            get(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-candidates/:id/classify",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-candidates/:id/credentials",
            get(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-credentials",
            get(crate::ui_routes::provider_runtime_offline)
                .post(crate::ui_routes::provider_credentials_save),
        )
        .route(
            "/api/ui/provider-credentials/:id",
            delete(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-credentials/:id/revoke",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-credentials/:id/rotate",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-local/scan-start",
            post(crate::ui_routes::providers_discover),
        )
        .route(
            "/api/ui/provider-local/scan-status",
            get(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-catalog/refresh",
            post(crate::ui_routes::providers_discover),
        )
        .route(
            "/api/ui/provider-model-catalogs",
            get(crate::ui_routes::provider_runtime_offline)
                .post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-model-catalogs/:id/stale",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-candidates/:id/model-catalogs",
            get(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-candidates/:id/model-catalog/refresh",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-candidates/:id/models",
            get(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-routes",
            get(crate::ui_routes::provider_runtime_offline)
                .post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-routes/test-chat",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-routes/:id",
            delete(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-routes/:id/revoke",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-routes/:id/select",
            post(crate::ui_routes::provider_routes_select),
        )
        .route(
            "/api/ui/provider-route/select",
            post(crate::ui_routes::provider_route_select),
        )
        .route(
            "/api/ui/model/select",
            get(crate::ui_routes::model_selected).post(crate::ui_routes::model_select),
        )
        .route(
            "/api/ui/provider-candidates/:id/route-certificate",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-candidates/:id/chat-test",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-helpers",
            get(crate::ui_routes::provider_runtime_offline)
                .post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-helpers/:id",
            delete(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-adapter-manifests",
            get(crate::ui_routes::provider_runtime_offline)
                .post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/provider-adapter-manifests/:id",
            delete(crate::ui_routes::provider_runtime_offline),
        )
        .route("/api/ui/providers", get(crate::ui_routes::providers_list))
        .route(
            "/api/ui/providers/preflight",
            post(crate::ui_routes::providers_preflight),
        )
        .route(
            "/api/ui/providers/discover",
            post(crate::ui_routes::providers_discover),
        )
        .route(
            "/api/ui/providers/register",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/providers/state-reset-dry-run",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/providers/state-reset",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/providers/:id",
            get(crate::ui_routes::providers_detail),
        )
        .route(
            "/api/ui/providers/:id/models",
            get(crate::ui_routes::providers_models),
        )
        .route(
            "/api/ui/providers/:id/probe",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/providers/:id/connect",
            post(crate::ui_routes::providers_connect),
        )
        .route(
            "/api/ui/providers/:id/refresh-models",
            post(crate::ui_routes::providers_connect),
        )
        .route(
            "/api/ui/providers/:id/enable",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/providers/:id/disable",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/providers/:id/auth/start",
            post(crate::ui_routes::providers_auth_start),
        )
        .route(
            "/api/ui/providers/:id/auth/complete",
            post(crate::ui_routes::providers_auth_complete),
        )
        .route(
            "/api/ui/providers/:id/auth/refresh",
            post(crate::ui_routes::providers_auth_refresh),
        )
        .route(
            "/api/ui/providers/:id/entitlement",
            get(crate::ui_routes::providers_entitlement),
        )
        .route(
            "/api/ui/providers/:id/test-chat",
            post(crate::ui_routes::provider_runtime_offline),
        )
        .route(
            "/api/ui/providers/probe-all",
            post(crate::ui_routes::providers_discover),
        )
        // MCP, Import/Export, Brain Admin routes
        .route("/api/ui/mcp/config", get(crate::ui_routes::mcp_config))
        .route("/api/ui/imports", get(crate::ui_routes::imports_list))
        .route(
            "/api/ui/import/seed-pack",
            post(crate::ui_routes::import_pack),
        )
        .route("/api/ui/export", get(crate::ui_routes::export_brain))
        .route("/api/ui/brain/backup", post(crate::ui_routes::brain_backup))
        .route("/api/ui/brain/purge", post(crate::ui_routes::brain_purge))
        .route(
            "/api/ui/maintenance/export",
            get(crate::ui_routes::export_brain),
        )
        .route(
            "/api/ui/maintenance/backup",
            post(crate::ui_routes::brain_backup),
        )
        .route(
            "/api/ui/maintenance/purge-brain",
            post(crate::ui_routes::brain_purge),
        )
        // Memory import routes (feature-flagged at handler level)
        .route(
            "/api/ui/memory/import/sources",
            get(crate::ui_routes::memory_import_sources),
        )
        .route(
            "/api/ui/memory/import/batches",
            post(crate::ui_routes::memory_import_batch_create),
        )
        .route(
            "/api/ui/memory/import/batches/:id",
            get(crate::ui_routes::memory_import_batch_get),
        )
        .route(
            "/api/ui/memory/import/batches/:id/parse",
            post(crate::ui_routes::memory_import_batch_parse),
        )
        .route(
            "/api/ui/memory/import/batches/:id/preview",
            get(crate::ui_routes::memory_import_batch_preview),
        )
        .route(
            "/api/ui/memory/import/batches/:id/commit",
            post(crate::ui_routes::memory_import_batch_commit),
        )
        .route(
            "/api/ui/memory/import/batches/:id/cancel",
            post(crate::ui_routes::memory_import_batch_cancel),
        )
        .route(
            "/api/ui/memory/import/batches/:id/report",
            get(crate::ui_routes::memory_import_batch_report),
        )
        // Workspace filesystem bridge routes
        .route(
            "/api/ui/workspace/files/list",
            post(crate::ui_routes::workspace_files_list),
        )
        .route(
            "/api/ui/workspace/files/read",
            post(crate::ui_routes::workspace_files_read),
        )
        .route(
            "/api/ui/workspace/files/search",
            post(crate::ui_routes::workspace_files_search),
        )
        .route(
            "/api/ui/workspace/files/write-preview",
            post(crate::ui_routes::workspace_files_write_preview),
        )
        .route(
            "/api/ui/workspace/files/write-apply",
            post(crate::ui_routes::workspace_files_write_apply),
        )
        // Attachment inspection route
        .route(
            "/api/ui/attachments/inspect",
            post(crate::ui_routes::attachments_inspect),
        )
        // Code review routes
        .route(
            "/api/ui/code-review/propose",
            post(crate::ui_routes::code_review_propose),
        )
        .route(
            "/api/ui/code-review/accept",
            post(crate::ui_routes::code_review_accept),
        )
        .route(
            "/api/ui/code-review/reject",
            post(crate::ui_routes::code_review_reject),
        )
        .route(
            "/api/ui/code-review/diff",
            post(crate::ui_routes::code_review_diff),
        )
        .route(
            "/api/ui/code-review/list",
            get(crate::ui_routes::code_review_list),
        )
        // Document reader route
        .route(
            "/api/ui/documents/read",
            post(crate::ui_routes::documents_read),
        )
        .with_state(state)
}

async fn well_known() -> Json<Value> {
    Json(json!({
        "name": "HOM Local",
        "version": env!("CARGO_PKG_VERSION"),
        "base_url": "http://127.0.0.1:9101",
        "auth": "HOM signed app grant"
    }))
}

async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "helper": "ingress",
        "uptime_s": state.started_at.elapsed().as_secs(),
        "tcp": "127.0.0.1:9101"
    }))
}

async fn ready(State(state): State<Arc<AppState>>) -> Response {
    match state.brain.call("system.ready", json!({}), "system").await {
        Ok(result) => (StatusCode::OK, Json(json!({"ok": true, "ready": true, "brain": result}))).into_response(),
        Err(error) => (
            brain_error_status(&error),
            Json(json!({"ok": false, "ready": false, "blocking": ["brain"], "error": error.error_payload()})),
        )
            .into_response(),
    }
}

async fn openapi() -> Json<Value> {
    Json(json!({
        "openapi": "3.1.0",
        "info": {"title": "HOM Local Ingress", "version": env!("CARGO_PKG_VERSION")},
        "paths": {
            "/api/health": {"get": {"summary": "Ingress health"}},
            "/api/ready": {"get": {"summary": "Brain readiness"}},
            "/api/recall": {"post": {"summary": "Signed recall through ingress"}}
        }
    }))
}

async fn recall(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let envelope = match envelope_from_headers(&headers) {
        Ok(envelope) => envelope,
        Err((code, message)) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"ok": false, "error": {"code": code, "message": message}})),
            )
                .into_response();
        }
    };

    let identity = {
        let mut verifier = state.verifier.lock().await;
        match verifier.verify_body(&body, &envelope, Some("memory:recall")) {
            Ok(identity) => identity,
            Err(error) => {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"ok": false, "error": {"code": error.code(), "message": error.to_string()}})),
                )
                    .into_response();
            }
        }
    };

    if let Err(retry_after) = state
        .limiter
        .lock()
        .await
        .check(&identity.client_id, LimitClass::Read)
    {
        let mut response = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"ok": false, "error": "rate_limited"})),
        )
            .into_response();
        response.headers_mut().insert(
            "Retry-After",
            HeaderValue::from_str(&retry_after.to_string())
                .unwrap_or(HeaderValue::from_static("60")),
        );
        return response;
    }

    match state
        .brain
        .call("memory.recall", body, "memory:recall")
        .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(error) => (
            brain_error_status(&error),
            Json(json!({"ok": false, "error": error.error_payload()})),
        )
            .into_response(),
    }
}

pub fn brain_error_status(error: &BrainClientError) -> StatusCode {
    match error {
        BrainClientError::Rpc(hom_shared::ERR_QUARANTINED_INPUT, _, _)
        | BrainClientError::Rpc(hom_shared::ERR_SCOPE_INSUFFICIENT, _, _) => StatusCode::FORBIDDEN,
        BrainClientError::Rpc(-32602, _, _) => StatusCode::BAD_REQUEST,
        BrainClientError::Rpc(-32041, _, _) => StatusCode::NOT_FOUND,
        BrainClientError::Rpc(-32042, _, _) => StatusCode::UNAUTHORIZED,
        BrainClientError::Rpc(-32043, _, _) => StatusCode::NOT_FOUND,
        BrainClientError::Rpc(-32044, _, _) => StatusCode::NOT_FOUND,
        BrainClientError::Rpc(-32080, _, _) => StatusCode::FORBIDDEN,
        BrainClientError::Rpc(_, _, _) => StatusCode::BAD_GATEWAY,
        BrainClientError::Io(_) | BrainClientError::Sign(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    fn offline_bubbles() -> BubbleClients {
        let dead = "http://127.0.0.1:1".to_string();
        BubbleClients::from_urls(dead.clone(), dead.clone(), dead.clone(), dead)
    }

    fn test_router() -> Router {
        let brain = BrainClient::new_for_test(
            "/tmp/hom-ingress-test-unused.sock".into(),
            "unused-public-key".to_string(),
            "unused-private-key".to_string(),
        );
        build_router(Arc::new(AppState::new_with_bubbles(
            brain,
            Vec::new(),
            offline_bubbles(),
        )))
    }

    async fn assert_capability_unavailable(method: axum::http::Method, uri: &str, runtime: &str) {
        let method_name = method.as_str().to_string();
        let response = test_router()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"]["message"], "capability_unavailable");
        assert_eq!(payload["error"]["data"]["runtime"], runtime);
        assert_eq!(payload["error"]["data"]["route"], uri);
        assert_eq!(payload["error"]["data"]["method"], method_name);
        assert_eq!(payload["error"]["data"]["state"], "offline");
        assert_eq!(payload["error"]["data"]["retryable"], true);
        assert_eq!(payload["error"]["data"]["brain_forwarded"], false);
    }

    #[tokio::test]
    async fn provider_and_chat_routes_do_not_forward_to_brain() {
        assert_capability_unavailable(
            axum::http::Method::GET,
            "/api/ui/provider-candidates",
            "ProviderRuntime",
        )
        .await;
        assert_capability_unavailable(
            axum::http::Method::GET,
            "/api/ui/chat/sessions",
            "ProviderRuntime",
        )
        .await;
        assert_capability_unavailable(
            axum::http::Method::POST,
            "/api/auth/request",
            "ProviderRuntime",
        )
        .await;
    }

    #[tokio::test]
    async fn tool_and_mcp_routes_do_not_forward_to_brain() {
        for uri in ["/api/tools", "/api/ui/tools", "/api/ui/mcp/config"] {
            let response = test_router()
                .oneshot(
                    Request::builder()
                        .method(axum::http::Method::GET)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            let payload: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(payload["ok"], true, "{uri}");
            assert_eq!(payload["source"], "capability_mesh", "{uri}");
        }
    }
}

fn envelope_from_headers(headers: &HeaderMap) -> Result<HomEnvelope, (i64, String)> {
    let get = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| (ERR_AUTH_MALFORMED, format!("missing header {name}")))
    };
    Ok(HomEnvelope {
        client_id: get("HOM-Client-Id")?,
        client_pub: get("HOM-Client-Pub")?,
        ts: get("HOM-Timestamp")?
            .parse()
            .map_err(|_| (ERR_AUTH_MALFORMED, "invalid HOM-Timestamp".to_string()))?,
        nonce: get("HOM-Nonce")?,
        scope: get("HOM-Scope")?
            .split(',')
            .map(str::trim)
            .filter(|scope| !scope.is_empty())
            .map(ToString::to_string)
            .collect(),
        body_hash: get("HOM-Body-Hash")?,
        signature: get("HOM-Signature")?,
    })
}

#[allow(dead_code)]
fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
