use std::path::Path;

use hom_shared::{RpcError, canonical_json, rpc_err};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::BrainState;
use crate::db::storage::{append_ledger_tx, unix_now_s};

pub const ERR_PERMISSION_DENIED: i64 = -32080;

const PROFILE_RESTRICTED: &str = "restricted";
const PROFILE_WORKSPACE: &str = "workspace";
const PROFILE_FULL_ACCESS: &str = "full_access";
const SYSTEM_GRANTEE: &str = "hom-local";

const GRANT_KINDS: [&str; 7] = [
    "shell",
    "filesystem",
    "browser",
    "app_automation",
    "network",
    "provider_key",
    "cloud_drive",
];

#[derive(Clone, Debug)]
pub struct PermissionState {
    pub profile: String,
    pub network_enabled: bool,
    pub filesystem_enabled: bool,
    pub browser_enabled: bool,
    pub automation_enabled: bool,
    pub shell_access: bool,
    pub provider_key_access: bool,
    pub local_roots: Vec<String>,
    pub google_drive_grants: Vec<String>,
    pub local_filesystem_grants: Vec<String>,
}

impl PermissionState {
    pub fn to_value(&self) -> Value {
        json!({
            "profile": self.profile,
            "network_enabled": self.network_enabled,
            "networkEnabled": self.network_enabled,
            "filesystem_enabled": self.filesystem_enabled,
            "filesystemEnabled": self.filesystem_enabled,
            "browser_enabled": self.browser_enabled,
            "browserEnabled": self.browser_enabled,
            "automation_enabled": self.automation_enabled,
            "automationEnabled": self.automation_enabled,
            "shell_access": self.shell_access,
            "shellAccess": self.shell_access,
            "provider_key_access": self.provider_key_access,
            "providerKeyAccess": self.provider_key_access,
            "local_roots": self.local_roots,
            "localRoots": self.local_roots,
            "google_drive_grants": self.google_drive_grants,
            "googleDriveGrants": self.google_drive_grants,
            "local_filesystem_grants": self.local_filesystem_grants,
            "localFilesystemGrants": self.local_filesystem_grants,
            "state": self.profile,
            "capabilities": {
                "settings_write": profile_rank(&self.profile) >= profile_rank(PROFILE_WORKSPACE),
                "app_approval": self.profile == PROFILE_FULL_ACCESS,
                "network": self.network_enabled,
                "filesystem": self.filesystem_enabled,
                "browser": self.browser_enabled,
                "automation": self.automation_enabled,
                "shell": self.shell_access
            }
        })
    }
}

pub fn get(state: &BrainState) -> Result<Value, RpcError> {
    Ok(json!({
        "ok": true,
        "permissions": load_state(state)?.to_value(),
        "grants": list_grant_rows(state, "all")?,
        "grantKinds": GRANT_KINDS
    }))
}

pub fn set_profile(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let mut updates = Map::new();
    let profile =
        string_field(&params, &["profile"]).ok_or_else(|| rpc_err(-32602, "profile_required"))?;
    updates.insert("permissions.profile".to_string(), json!(profile));
    for (key, aliases) in [
        (
            "permissions.network_enabled",
            ["network_enabled", "networkEnabled"],
        ),
        (
            "permissions.filesystem_enabled",
            ["filesystem_enabled", "filesystemEnabled"],
        ),
        (
            "permissions.browser_enabled",
            ["browser_enabled", "browserEnabled"],
        ),
        (
            "permissions.automation_enabled",
            ["automation_enabled", "automationEnabled"],
        ),
        ("permissions.shell_access", ["shell_access", "shellAccess"]),
        (
            "permissions.provider_key_access",
            ["provider_key_access", "providerKeyAccess"],
        ),
    ] {
        if let Some(value) = first(&params, &aliases) {
            updates.insert(key.to_string(), value.clone());
        }
    }
    for (key, aliases) in [
        ("permissions.local_roots", ["local_roots", "localRoots"]),
        (
            "permissions.google_drive_grants",
            ["google_drive_grants", "googleDriveGrants"],
        ),
        (
            "permissions.local_filesystem_grants",
            ["local_filesystem_grants", "localFilesystemGrants"],
        ),
    ] {
        if let Some(value) = first(&params, &aliases) {
            updates.insert(key.to_string(), value.clone());
        }
    }
    validate_permission_settings(state, &updates, confirmation_present(&params))?;
    persist_settings(state, &updates)?;
    sync_setting_grants(state, &updates)?;
    Ok(json!({"ok": true, "permissions": load_state(state)?.to_value()}))
}

pub fn grant_root(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let path = string_field(&params, &["path", "root"])
        .ok_or_else(|| rpc_err(-32602, "root_path_required"))?;
    validate_absolute_path(&path, "root_path")?;
    validate_permission_settings(
        state,
        &Map::from_iter([("permissions.local_roots".to_string(), json!([path.clone()]))]),
        confirmation_present(&params),
    )?;
    let mut current = load_state(state)?.local_roots;
    if !current.iter().any(|existing| existing == &path) {
        current.push(path.clone());
        current.sort();
    }
    persist_settings(
        state,
        &Map::from_iter([("permissions.local_roots".to_string(), json!(current))]),
    )?;
    ensure_system_grant(
        state,
        "filesystem",
        &path,
        "filesystem:root",
        json!({"root": path, "source": "permissions.grant_root"}),
        "local root granted",
    )?;
    Ok(json!({"ok": true, "permissions": load_state(state)?.to_value()}))
}

pub fn revoke_root(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let path = string_field(&params, &["path", "root"])
        .ok_or_else(|| rpc_err(-32602, "root_path_required"))?;
    let mut current = load_state(state)?.local_roots;
    current.retain(|existing| existing != &path);
    persist_settings(
        state,
        &Map::from_iter([("permissions.local_roots".to_string(), json!(current))]),
    )?;
    revoke_system_grant_if_present(
        state,
        "filesystem",
        &path,
        "filesystem:root",
        "local root revoked",
    )?;
    Ok(json!({"ok": true, "permissions": load_state(state)?.to_value()}))
}

pub fn grants_list(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let state_filter = string_field(&params, &["state"]).unwrap_or_else(|| "all".to_string());
    Ok(json!({
        "ok": true,
        "grants": list_grant_rows(state, &state_filter)?,
        "grantKinds": GRANT_KINDS
    }))
}

pub fn grants_create(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let grant = grant_input(&params)?;
    if grant.reason.trim().is_empty() {
        return Err(rpc_err(-32602, "grant_reason_required"));
    }
    if grant_is_broadening(state, &grant)? && !confirmation_present(&params) {
        return Err(permission_error(
            "permission_broadening_confirmation_required",
            json!({
                "grant_kind": grant.grant_kind,
                "subject_id": grant.subject_id,
                "grantee_id": grant.grantee_id,
                "capability_id": grant.capability_id,
                "scope": grant.scope
            }),
        ));
    }
    let grant = create_or_refresh_grant(state, &grant, "permissions.grants.create")?;
    Ok(json!({"ok": true, "grant": grant}))
}

pub fn grants_revoke(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let reason = string_field(&params, &["reason"])
        .ok_or_else(|| rpc_err(-32602, "grant_revoke_reason_required"))?;
    let revoked = revoke_grants(state, &params, &reason, "permissions.grants.revoke")?;
    if revoked.is_empty() {
        return Err(rpc_err(-32602, "grant_not_found_or_inactive"));
    }
    Ok(json!({"ok": true, "revoked": revoked, "count": revoked.len()}))
}

pub fn grants_check(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let grant = grant_input(&params)?;
    let row = require_active_grant(
        state,
        &grant.grant_kind,
        &grant.subject_id,
        &grant.grantee_id,
        grant.capability_id.as_deref(),
        "permission_grant_required",
    )?;
    Ok(json!({"ok": true, "grant": row}))
}

pub fn validate_permission_settings(
    state: &BrainState,
    settings: &Map<String, Value>,
    confirmed: bool,
) -> Result<(), RpcError> {
    let current = load_state(state)?;
    let mut broadening = Vec::new();
    for (key, value) in settings {
        match key.as_str() {
            "permissions.profile" => {
                let profile = value
                    .as_str()
                    .ok_or_else(|| rpc_err(-32602, "permissions.profile_must_be_string"))?;
                validate_profile(profile)?;
                if profile_rank(profile) > profile_rank(&current.profile) {
                    broadening.push(key.clone());
                }
            }
            "permissions.network_enabled" => {
                let next = bool_value(value, key)?;
                if next && !current.network_enabled {
                    broadening.push(key.clone());
                }
            }
            "permissions.filesystem_enabled" => {
                let next = bool_value(value, key)?;
                if next && !current.filesystem_enabled {
                    broadening.push(key.clone());
                }
            }
            "permissions.browser_enabled" => {
                let next = bool_value(value, key)?;
                if next && !current.browser_enabled {
                    broadening.push(key.clone());
                }
            }
            "permissions.automation_enabled" => {
                let next = bool_value(value, key)?;
                if next && !current.automation_enabled {
                    broadening.push(key.clone());
                }
            }
            "permissions.shell_access" => {
                let next = bool_value(value, key)?;
                if next && !current.shell_access {
                    broadening.push(key.clone());
                }
            }
            "permissions.provider_key_access" => {
                let next = bool_value(value, key)?;
                if next && !current.provider_key_access {
                    broadening.push(key.clone());
                }
            }
            "permissions.local_roots" => {
                let roots = string_array_value(value, key)?;
                for root in &roots {
                    validate_absolute_path(root, key)?;
                }
                if roots
                    .iter()
                    .any(|root| !current.local_roots.iter().any(|existing| existing == root))
                {
                    broadening.push(key.clone());
                }
            }
            "permissions.google_drive_grants" => {
                let grants = string_array_value(value, key)?;
                if grants.iter().any(|grant| grant.trim().is_empty()) {
                    return Err(rpc_err(-32602, "permissions.google_drive_grants_invalid"));
                }
                if grants.iter().any(|grant| {
                    !current
                        .google_drive_grants
                        .iter()
                        .any(|existing| existing == grant)
                }) {
                    broadening.push(key.clone());
                }
            }
            "permissions.local_filesystem_grants" => {
                let grants = string_array_value(value, key)?;
                for grant in &grants {
                    validate_absolute_path(grant, key)?;
                }
                if grants.iter().any(|grant| {
                    !current
                        .local_filesystem_grants
                        .iter()
                        .any(|existing| existing == grant)
                }) {
                    broadening.push(key.clone());
                }
            }
            _ => {
                return Err(rpc_err(
                    -32602,
                    format!("unknown_permission_setting: {key}"),
                ));
            }
        }
    }
    if !broadening.is_empty() && !confirmed {
        return Err(permission_error(
            "permission_broadening_confirmation_required",
            json!({"settings": broadening}),
        ));
    }
    Ok(())
}

pub fn enforce_settings_write(
    state: &BrainState,
    settings: &Map<String, Value>,
) -> Result<(), RpcError> {
    if settings.keys().all(|key| key.starts_with("permissions.")) {
        return Ok(());
    }
    let permissions = load_state(state)?;
    if profile_rank(&permissions.profile) < profile_rank(PROFILE_WORKSPACE) {
        return Err(permission_error(
            "settings_write_requires_workspace_or_full_access",
            permissions.to_value(),
        ));
    }
    Ok(())
}

pub fn enforce_app_approval(state: &BrainState) -> Result<(), RpcError> {
    let permissions = load_state(state)?;
    if permissions.profile != PROFILE_FULL_ACCESS {
        return Err(permission_error(
            "app_approval_requires_full_access",
            permissions.to_value(),
        ));
    }
    Ok(())
}

pub fn enforce_automation(state: &BrainState) -> Result<(), RpcError> {
    let permissions = load_state(state)?;
    if !permissions.automation_enabled {
        return Err(permission_error(
            "automation_disabled_by_permission_policy",
            permissions.to_value(),
        ));
    }
    Ok(())
}

/// Tool gate enforcement: check that the permission profile and grant ledger
/// allow the requested tool action. Each tool domain (shell, filesystem,
/// network, browser, app_connector) maps to a permission toggle and/or grant.
pub fn enforce_tool_gate(
    state: &BrainState,
    tool_domain: &str,
    action_context: Option<&Value>,
) -> Result<(), RpcError> {
    let permissions = load_state(state)?;
    match tool_domain {
        "brain" => {
            if profile_rank(&permissions.profile) < profile_rank(PROFILE_RESTRICTED) {
                return Err(permission_error(
                    "tool_gate_brain_denied",
                    json!({
                        "tool_domain": "brain",
                        "required": "valid permission profile",
                        "current": permissions.to_value()
                    }),
                ));
            }
        }
        "shell" => {
            if !permissions.shell_access {
                return Err(permission_error(
                    "tool_gate_shell_denied",
                    json!({
                        "tool_domain": "shell",
                        "required": "shell_access=true",
                        "current": permissions.to_value()
                    }),
                ));
            }
        }
        "filesystem" => {
            if !permissions.filesystem_enabled {
                return Err(permission_error(
                    "tool_gate_filesystem_denied",
                    json!({
                        "tool_domain": "filesystem",
                        "required": "filesystem_enabled=true",
                        "current": permissions.to_value()
                    }),
                ));
            }
        }
        "network" => {
            if !permissions.network_enabled {
                return Err(permission_error(
                    "tool_gate_network_denied",
                    json!({
                        "tool_domain": "network",
                        "required": "network_enabled=true",
                        "current": permissions.to_value()
                    }),
                ));
            }
        }
        "browser" => {
            if !permissions.browser_enabled {
                return Err(permission_error(
                    "tool_gate_browser_denied",
                    json!({
                        "tool_domain": "browser",
                        "required": "browser_enabled=true",
                        "current": permissions.to_value()
                    }),
                ));
            }
        }
        "app_connector" => {
            if !permissions.automation_enabled {
                return Err(permission_error(
                    "tool_gate_app_connector_denied",
                    json!({
                        "tool_domain": "app_connector",
                        "required": "automation_enabled=true",
                        "current": permissions.to_value()
                    }),
                ));
            }
        }
        "provider_key" => {
            if !permissions.provider_key_access {
                return Err(permission_error(
                    "tool_gate_provider_key_denied",
                    json!({
                        "tool_domain": "provider_key",
                        "required": "provider_key_access=true",
                        "current": permissions.to_value()
                    }),
                ));
            }
        }
        _ => {
            // Unknown tool domains are denied by default
            return Err(permission_error(
                "tool_gate_unknown_denied",
                json!({
                    "tool_domain": tool_domain,
                    "reason": "unknown tool domain, denied by default"
                }),
            ));
        }
    }

    // Check grant ledger for scoped access if action context is provided
    if let Some(context) = action_context {
        if let Some(path) = context.get("path").and_then(Value::as_str) {
            let path_owned = path.to_string();
            let has_grant = has_filesystem_grant(state, &path_owned)?;
            if !has_grant {
                return Err(permission_error(
                    "tool_gate_filesystem_path_denied",
                    json!({
                        "tool_domain": "filesystem",
                        "path": path,
                        "required": "active filesystem grant for path",
                        "current": permissions.to_value()
                    }),
                ));
            }
        }
    }

    Ok(())
}

/// Check if there is an active filesystem grant covering the given path.
fn has_filesystem_grant(state: &BrainState, path: &str) -> Result<bool, RpcError> {
    let conn = state
        .store
        .conn()
        .map_err(|e| rpc_err(-32603, format!("db_error: {e:?}")))?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM grant_ledger_entries
             WHERE grant_kind = 'filesystem'
               AND state = 'active'
               AND (expires_at_s IS NULL OR expires_at_s > ?1)",
            params![unix_now_s()],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if count == 0 {
        return Ok(false);
    }
    // Check if any grant root is a prefix of the requested path
    let mut stmt = conn
        .prepare("SELECT subject_id FROM grant_ledger_entries WHERE grant_kind = 'filesystem' AND state = 'active' AND (expires_at_s IS NULL OR expires_at_s > ?1)")
        .map_err(|e| rpc_err(-32603, format!("db_error: {e:?}")))?;
    let roots: Vec<String> = stmt
        .query_map(params![unix_now_s()], |row| row.get(0))
        .map_err(|e| rpc_err(-32603, format!("db_error: {e}")))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(roots.iter().any(|root| path.starts_with(root.as_str())))
}

pub fn enforce_purge(state: &BrainState, params: &Value) -> Result<(), RpcError> {
    let permissions = load_state(state)?;
    if permissions.profile != PROFILE_FULL_ACCESS {
        return Err(permission_error(
            "brain_purge_requires_full_access",
            permissions.to_value(),
        ));
    }
    if !confirmation_present(params) {
        return Err(permission_error(
            "brain_purge_confirmation_required",
            json!({"required": "confirm_broadening=true or confirmation present"}),
        ));
    }
    Ok(())
}

pub fn load_state(state: &BrainState) -> Result<PermissionState, RpcError> {
    let profile = read_string(state, "permissions.profile")?
        .unwrap_or_else(|| PROFILE_RESTRICTED.to_string());
    validate_profile(&profile)?;
    Ok(PermissionState {
        profile,
        network_enabled: read_bool(state, "permissions.network_enabled")?.unwrap_or(false),
        filesystem_enabled: read_bool(state, "permissions.filesystem_enabled")?.unwrap_or(false),
        browser_enabled: read_bool(state, "permissions.browser_enabled")?.unwrap_or(false),
        automation_enabled: read_bool(state, "permissions.automation_enabled")?.unwrap_or(false),
        shell_access: read_bool(state, "permissions.shell_access")?.unwrap_or(false),
        provider_key_access: read_bool(state, "permissions.provider_key_access")?.unwrap_or(false),
        local_roots: read_string_array(state, "permissions.local_roots")?,
        google_drive_grants: read_string_array(state, "permissions.google_drive_grants")?,
        local_filesystem_grants: read_string_array(state, "permissions.local_filesystem_grants")?,
    })
}

pub fn is_permission_key(key: &str) -> bool {
    key.starts_with("permissions.")
}

pub(crate) fn reconcile_system_grants(state: &BrainState) -> Result<(), RpcError> {
    let permissions = load_state(state)?;
    reconcile_boolean_grant(
        state,
        permissions.network_enabled,
        "network",
        "network",
        "network:use",
        json!({"network": "all", "source": "permissions.reconcile"}),
        "network permission reconciled from settings",
        "network permission disabled",
    )?;
    reconcile_boolean_grant(
        state,
        permissions.filesystem_enabled,
        "filesystem",
        "filesystem",
        "filesystem:use",
        json!({"filesystem": "enabled", "source": "permissions.reconcile"}),
        "filesystem permission reconciled from settings",
        "filesystem permission disabled",
    )?;
    reconcile_boolean_grant(
        state,
        permissions.browser_enabled,
        "browser",
        "browser",
        "browser:use",
        json!({"browser": "enabled", "source": "permissions.reconcile"}),
        "browser permission reconciled from settings",
        "browser permission disabled",
    )?;
    reconcile_boolean_grant(
        state,
        permissions.automation_enabled,
        "app_automation",
        "automation",
        "automation:run",
        json!({"automation": true, "source": "permissions.reconcile"}),
        "automation permission reconciled from settings",
        "automation permission disabled",
    )?;
    reconcile_boolean_grant(
        state,
        permissions.shell_access,
        "shell",
        "shell",
        "shell:execute",
        json!({"shell": "local", "source": "permissions.reconcile"}),
        "shell permission reconciled from settings",
        "shell permission disabled",
    )?;
    reconcile_boolean_grant(
        state,
        permissions.provider_key_access,
        "provider_key",
        "provider_key",
        "provider-key:use",
        json!({"provider_key": true, "source": "permissions.reconcile"}),
        "provider key permission reconciled from settings",
        "provider key permission disabled",
    )?;
    for root in permissions.local_roots {
        if !grant_history_exists(state, "filesystem", &root, "filesystem:root")? {
            ensure_system_grant(
                state,
                "filesystem",
                &root,
                "filesystem:root",
                json!({"root": root, "source": "permissions.reconcile"}),
                "local root reconciled from settings",
            )?;
        }
    }
    for root in permissions.local_filesystem_grants {
        if !grant_history_exists(state, "filesystem", &root, "filesystem:access")? {
            ensure_system_grant(
                state,
                "filesystem",
                &root,
                "filesystem:access",
                json!({"root": root, "source": "permissions.reconcile"}),
                "local filesystem grant reconciled from settings",
            )?;
        }
    }
    for drive_scope in permissions.google_drive_grants {
        if !grant_history_exists(state, "cloud_drive", &drive_scope, "cloud-drive:access")? {
            ensure_system_grant(
                state,
                "cloud_drive",
                &drive_scope,
                "cloud-drive:access",
                json!({"drive_scope": drive_scope, "source": "permissions.reconcile"}),
                "cloud drive grant reconciled from settings",
            )?;
        }
    }
    Ok(())
}

fn persist_settings(state: &BrainState, settings: &Map<String, Value>) -> Result<(), RpcError> {
    let conn = state.store.conn()?;
    let now = unix_now_s();
    for (key, value) in settings {
        let value_str = serde_json::to_string(value)
            .map_err(|e| rpc_err(-32603, format!("permission_setting_serialize: {e}")))?;
        conn.execute(
            "INSERT INTO settings (key, value, updated_at_s) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at_s = ?3",
            params![key, value_str, now],
        )
        .map_err(|e| rpc_err(-32603, format!("permission_setting_persist: {e}")))?;
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct GrantInput {
    grant_kind: String,
    subject_id: String,
    grantee_id: String,
    capability_id: Option<String>,
    scope: Value,
    issuer: String,
    reason: String,
    expires_at_s: Option<i64>,
}

fn grant_input(params: &Value) -> Result<GrantInput, RpcError> {
    let grant_kind = string_field(params, &["grant_kind", "grantKind", "kind"])
        .ok_or_else(|| rpc_err(-32602, "grant_kind_required"))?;
    let grant_kind = normalize_grant_kind(&grant_kind)?;
    let subject_id = string_field(params, &["subject_id", "subjectId", "subject"])
        .ok_or_else(|| rpc_err(-32602, "grant_subject_required"))?;
    let grantee_id = string_field(params, &["grantee_id", "granteeId", "grantee"])
        .unwrap_or_else(|| SYSTEM_GRANTEE.to_string());
    let capability_id = string_field(params, &["capability_id", "capabilityId", "capability"]);
    let scope = params
        .get("scope")
        .or_else(|| params.get("scope_json"))
        .or_else(|| params.get("scopeJson"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let issuer = string_field(params, &["issuer"]).unwrap_or_else(|| "hom-local.user".to_string());
    let reason = string_field(params, &["reason"]).unwrap_or_default();
    let expires_at_s = params
        .get("expires_at_s")
        .or_else(|| params.get("expiresAtS"))
        .and_then(Value::as_i64);
    Ok(GrantInput {
        grant_kind,
        subject_id,
        grantee_id,
        capability_id,
        scope,
        issuer,
        reason,
        expires_at_s,
    })
}

fn normalize_grant_kind(kind: &str) -> Result<String, RpcError> {
    let normalized = kind.trim().replace('-', "_").to_ascii_lowercase();
    let normalized = match normalized.as_str() {
        "app" | "apps" | "app_automation" | "automation" => "app_automation",
        "file" | "files" | "filesystem" | "local_filesystem" | "local_root" => "filesystem",
        "browser" | "web_browser" => "browser",
        "google_drive" | "cloud_drive" | "cloud" => "cloud_drive",
        "network" => "network",
        "provider_key" | "provider_keys" | "provider" => "provider_key",
        "shell" => "shell",
        _ => {
            return Err(rpc_err(-32602, format!("unknown_grant_kind: {kind}")));
        }
    };
    Ok(normalized.to_string())
}

fn grant_is_broadening(state: &BrainState, grant: &GrantInput) -> Result<bool, RpcError> {
    let conn = state.store.conn()?;
    let scope_json = canonical_json(&grant.scope)
        .map_err(|e| rpc_err(-32603, format!("grant_scope_canonicalize: {e}")))?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM grant_ledger_entries
             WHERE grant_kind = ?1
               AND subject_id = ?2
               AND grantee_id = ?3
               AND COALESCE(capability_id, '') = COALESCE(?4, '')
               AND scope_json = ?5
               AND state = 'active'
               AND (expires_at_s IS NULL OR expires_at_s > ?6)",
            params![
                grant.grant_kind,
                grant.subject_id,
                grant.grantee_id,
                grant.capability_id,
                scope_json,
                unix_now_s()
            ],
            |row| row.get(0),
        )
        .map_err(|e| rpc_err(-32603, format!("grant_broadening_lookup: {e}")))?;
    Ok(count == 0)
}

fn active_matching_grant(
    state: &BrainState,
    grant: &GrantInput,
) -> Result<Option<Value>, RpcError> {
    let conn = state.store.conn()?;
    let scope_json = canonical_json(&grant.scope)
        .map_err(|e| rpc_err(-32603, format!("grant_scope_canonicalize: {e}")))?;
    conn.query_row(
        "SELECT id, grant_kind, subject_id, grantee_id, capability_id, scope_json,
                state, issuer, reason, expires_at_s, revoked_at_s, ledger_event_id,
                created_at_s, updated_at_s
         FROM grant_ledger_entries
         WHERE grant_kind = ?1
           AND subject_id = ?2
           AND grantee_id = ?3
           AND COALESCE(capability_id, '') = COALESCE(?4, '')
           AND scope_json = ?5
           AND state = 'active'
           AND (expires_at_s IS NULL OR expires_at_s > ?6)
         ORDER BY created_at_s DESC
         LIMIT 1",
        params![
            grant.grant_kind,
            grant.subject_id,
            grant.grantee_id,
            grant.capability_id,
            scope_json,
            unix_now_s()
        ],
        grant_row,
    )
    .optional()
    .map_err(|e| rpc_err(-32603, format!("grant_active_matching_lookup: {e}")))
}

fn create_or_refresh_grant(
    state: &BrainState,
    grant: &GrantInput,
    actor: &str,
) -> Result<Value, RpcError> {
    let scope_json = canonical_json(&grant.scope)
        .map_err(|e| rpc_err(-32603, format!("grant_scope_canonicalize: {e}")))?;
    let now = unix_now_s();
    let id = Uuid::new_v4().to_string();
    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("grant_tx_begin: {e}")))?;
    tx.execute(
        "UPDATE grant_ledger_entries
         SET state = 'superseded', updated_at_s = ?1
         WHERE grant_kind = ?2
           AND subject_id = ?3
           AND grantee_id = ?4
           AND COALESCE(capability_id, '') = COALESCE(?5, '')
           AND state = 'active'
           AND scope_json != ?6",
        params![
            now,
            grant.grant_kind,
            grant.subject_id,
            grant.grantee_id,
            grant.capability_id,
            scope_json
        ],
    )
    .map_err(|e| rpc_err(-32603, format!("grant_supersede: {e}")))?;
    let existing_id: Option<String> = tx
        .query_row(
            "SELECT id FROM grant_ledger_entries
             WHERE grant_kind = ?1
               AND subject_id = ?2
               AND grantee_id = ?3
               AND COALESCE(capability_id, '') = COALESCE(?4, '')
               AND scope_json = ?5
               AND state = 'active'
             ORDER BY created_at_s DESC
             LIMIT 1",
            params![
                grant.grant_kind,
                grant.subject_id,
                grant.grantee_id,
                grant.capability_id,
                scope_json
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| rpc_err(-32603, format!("grant_existing_lookup: {e}")))?;
    let grant_id = existing_id.unwrap_or(id);
    let ledger = append_ledger_tx(
        &tx,
        "permission.grant.created",
        actor,
        Some(&grant.subject_id),
        json!({
            "grant_id": grant_id,
            "grant_kind": grant.grant_kind,
            "subject_id": grant.subject_id,
            "grantee_id": grant.grantee_id,
            "capability_id": grant.capability_id,
            "scope": grant.scope,
            "issuer": grant.issuer,
            "reason": grant.reason,
            "expires_at_s": grant.expires_at_s
        }),
        now,
    )
    .map_err(|e| rpc_err(-32603, format!("grant_ledger_append: {e}")))?;
    let ledger_event_id = ledger.get("event_id").and_then(Value::as_str);
    tx.execute(
        "INSERT INTO grant_ledger_entries
            (id, grant_kind, subject_id, grantee_id, capability_id, scope_json,
             state, issuer, reason, expires_at_s, revoked_at_s, ledger_event_id,
             created_at_s, updated_at_s)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?9, NULL, ?10, ?11, ?11)
         ON CONFLICT(id) DO UPDATE SET
            scope_json = ?6,
            state = 'active',
            issuer = ?7,
            reason = ?8,
            expires_at_s = ?9,
            revoked_at_s = NULL,
            ledger_event_id = ?10,
            updated_at_s = ?11",
        params![
            grant_id,
            grant.grant_kind,
            grant.subject_id,
            grant.grantee_id,
            grant.capability_id,
            scope_json,
            grant.issuer,
            grant.reason,
            grant.expires_at_s,
            ledger_event_id,
            now,
        ],
    )
    .map_err(|e| rpc_err(-32603, format!("grant_persist: {e}")))?;
    let row = grant_row_by_id_tx(&tx, &grant_id)?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("grant_tx_commit: {e}")))?;
    Ok(row)
}

fn revoke_grants(
    state: &BrainState,
    params: &Value,
    reason: &str,
    actor: &str,
) -> Result<Vec<Value>, RpcError> {
    let now = unix_now_s();
    let id = string_field(params, &["id", "grant_id", "grantId"]);
    let grant_kind = string_field(params, &["grant_kind", "grantKind", "kind"])
        .map(|kind| normalize_grant_kind(&kind))
        .transpose()?;
    let subject_id = string_field(params, &["subject_id", "subjectId", "subject"]);
    let grantee_id = string_field(params, &["grantee_id", "granteeId", "grantee"]);
    let capability_id = string_field(params, &["capability_id", "capabilityId", "capability"]);

    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("grant_revoke_tx_begin: {e}")))?;
    let active_ids = active_grant_ids_tx(
        &tx,
        id.as_deref(),
        grant_kind.as_deref(),
        subject_id.as_deref(),
        grantee_id.as_deref(),
        capability_id.as_deref(),
    )?;
    let mut revoked = Vec::new();
    for grant_id in active_ids {
        let before = grant_row_by_id_tx(&tx, &grant_id)?;
        let ledger = append_ledger_tx(
            &tx,
            "permission.grant.revoked",
            actor,
            before.get("subject_id").and_then(Value::as_str),
            json!({
                "grant_id": grant_id,
                "grant_kind": before.get("grant_kind").cloned().unwrap_or(Value::Null),
                "subject_id": before.get("subject_id").cloned().unwrap_or(Value::Null),
                "grantee_id": before.get("grantee_id").cloned().unwrap_or(Value::Null),
                "capability_id": before.get("capability_id").cloned().unwrap_or(Value::Null),
                "reason": reason
            }),
            now,
        )
        .map_err(|e| rpc_err(-32603, format!("grant_revoke_ledger_append: {e}")))?;
        let ledger_event_id = ledger.get("event_id").and_then(Value::as_str);
        tx.execute(
            "UPDATE grant_ledger_entries
             SET state = 'revoked',
                 revoked_at_s = ?1,
                 reason = ?2,
                 ledger_event_id = ?3,
                 updated_at_s = ?1
             WHERE id = ?4",
            params![now, reason, ledger_event_id, grant_id],
        )
        .map_err(|e| rpc_err(-32603, format!("grant_revoke_update: {e}")))?;
        revoked.push(grant_row_by_id_tx(&tx, &grant_id)?);
    }
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("grant_revoke_tx_commit: {e}")))?;
    Ok(revoked)
}

fn active_grant_ids_tx(
    tx: &Transaction<'_>,
    id: Option<&str>,
    grant_kind: Option<&str>,
    subject_id: Option<&str>,
    grantee_id: Option<&str>,
    capability_id: Option<&str>,
) -> Result<Vec<String>, RpcError> {
    let mut stmt = tx
        .prepare(
            "SELECT id, grant_kind, subject_id, grantee_id, capability_id
             FROM grant_ledger_entries
             WHERE state = 'active'
             ORDER BY created_at_s DESC",
        )
        .map_err(|e| rpc_err(-32603, format!("grant_revoke_prepare: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|e| rpc_err(-32603, format!("grant_revoke_query: {e}")))?;
    Ok(rows
        .filter_map(Result::ok)
        .filter(
            |(row_id, row_kind, row_subject, row_grantee, row_capability)| {
                id.is_none_or(|value| value == row_id)
                    && grant_kind.is_none_or(|value| value == row_kind)
                    && subject_id.is_none_or(|value| value == row_subject)
                    && grantee_id.is_none_or(|value| value == row_grantee)
                    && capability_id.is_none_or(|value| Some(value) == row_capability.as_deref())
            },
        )
        .map(|(row_id, _, _, _, _)| row_id)
        .collect())
}

fn require_active_grant(
    state: &BrainState,
    grant_kind: &str,
    subject_id: &str,
    grantee_id: &str,
    capability_id: Option<&str>,
    error_message: &str,
) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let now = unix_now_s();
    let mut stmt = conn
        .prepare(
            "SELECT id, grant_kind, subject_id, grantee_id, capability_id, scope_json,
                    state, issuer, reason, expires_at_s, revoked_at_s, ledger_event_id,
                    created_at_s, updated_at_s
             FROM grant_ledger_entries
             WHERE grant_kind = ?1
               AND subject_id = ?2
               AND grantee_id = ?3
               AND COALESCE(capability_id, '') = COALESCE(?4, '')
               AND state = 'active'
               AND (expires_at_s IS NULL OR expires_at_s > ?5)
             ORDER BY created_at_s DESC
             LIMIT 1",
        )
        .map_err(|e| rpc_err(-32603, format!("grant_require_prepare: {e}")))?;
    let row = stmt
        .query_row(
            params![grant_kind, subject_id, grantee_id, capability_id, now],
            grant_row,
        )
        .optional()
        .map_err(|e| rpc_err(-32603, format!("grant_require_query: {e}")))?;
    row.ok_or_else(|| {
        permission_error(
            error_message,
            json!({
                "grant_kind": grant_kind,
                "subject_id": subject_id,
                "grantee_id": grantee_id,
                "capability_id": capability_id,
                "required_state": "active"
            }),
        )
    })
}

fn list_grant_rows(state: &BrainState, state_filter: &str) -> Result<Vec<Value>, RpcError> {
    let conn = state.store.conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, grant_kind, subject_id, grantee_id, capability_id, scope_json,
                    state, issuer, reason, expires_at_s, revoked_at_s, ledger_event_id,
                    created_at_s, updated_at_s
             FROM grant_ledger_entries
             ORDER BY updated_at_s DESC, created_at_s DESC",
        )
        .map_err(|e| rpc_err(-32603, format!("grant_list_prepare: {e}")))?;
    let rows = stmt
        .query_map([], grant_row)
        .map_err(|e| rpc_err(-32603, format!("grant_list_query: {e}")))?;
    Ok(rows
        .filter_map(Result::ok)
        .filter(|row| {
            state_filter == "all" || row.get("state").and_then(Value::as_str) == Some(state_filter)
        })
        .collect())
}

fn grant_row_by_id_tx(tx: &Transaction<'_>, id: &str) -> Result<Value, RpcError> {
    tx.query_row(
        "SELECT id, grant_kind, subject_id, grantee_id, capability_id, scope_json,
                state, issuer, reason, expires_at_s, revoked_at_s, ledger_event_id,
                created_at_s, updated_at_s
         FROM grant_ledger_entries
         WHERE id = ?1",
        params![id],
        grant_row,
    )
    .map_err(|e| rpc_err(-32603, format!("grant_row_lookup: {e}")))
}

fn grant_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let scope_json: String = row.get(5)?;
    let scope = serde_json::from_str::<Value>(&scope_json).unwrap_or_else(|_| json!({}));
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "grant_id": row.get::<_, String>(0)?,
        "grant_kind": row.get::<_, String>(1)?,
        "grantKind": row.get::<_, String>(1)?,
        "subject_id": row.get::<_, String>(2)?,
        "subjectId": row.get::<_, String>(2)?,
        "grantee_id": row.get::<_, String>(3)?,
        "granteeId": row.get::<_, String>(3)?,
        "capability_id": row.get::<_, Option<String>>(4)?,
        "capabilityId": row.get::<_, Option<String>>(4)?,
        "scope": scope,
        "state": row.get::<_, String>(6)?,
        "issuer": row.get::<_, String>(7)?,
        "reason": row.get::<_, Option<String>>(8)?,
        "expires_at_s": row.get::<_, Option<i64>>(9)?,
        "expiresAtS": row.get::<_, Option<i64>>(9)?,
        "revoked_at_s": row.get::<_, Option<i64>>(10)?,
        "revokedAtS": row.get::<_, Option<i64>>(10)?,
        "ledger_event_id": row.get::<_, Option<String>>(11)?,
        "ledgerEventId": row.get::<_, Option<String>>(11)?,
        "created_at_s": row.get::<_, i64>(12)?,
        "createdAtS": row.get::<_, i64>(12)?,
        "updated_at_s": row.get::<_, i64>(13)?,
        "updatedAtS": row.get::<_, i64>(13)?
    }))
}

pub(crate) fn sync_setting_grants(
    state: &BrainState,
    updates: &Map<String, Value>,
) -> Result<(), RpcError> {
    if updates
        .get("permissions.network_enabled")
        .and_then(Value::as_bool)
        == Some(true)
    {
        ensure_system_grant(
            state,
            "network",
            "network",
            "network:use",
            json!({"network": "all", "source": "permissions.set_profile"}),
            "network permission enabled",
        )?;
    } else if updates
        .get("permissions.network_enabled")
        .and_then(Value::as_bool)
        == Some(false)
    {
        revoke_system_grant_if_present(
            state,
            "network",
            "network",
            "network:use",
            "network permission disabled",
        )?;
    }
    if updates
        .get("permissions.automation_enabled")
        .and_then(Value::as_bool)
        == Some(true)
    {
        ensure_system_grant(
            state,
            "app_automation",
            "automation",
            "automation:run",
            json!({"automation": true, "source": "permissions.set_profile"}),
            "automation permission enabled",
        )?;
    } else if updates
        .get("permissions.automation_enabled")
        .and_then(Value::as_bool)
        == Some(false)
    {
        revoke_system_grant_if_present(
            state,
            "app_automation",
            "automation",
            "automation:run",
            "automation permission disabled",
        )?;
    }
    if updates
        .get("permissions.shell_access")
        .and_then(Value::as_bool)
        == Some(true)
    {
        ensure_system_grant(
            state,
            "shell",
            "shell",
            "shell:execute",
            json!({"shell": "local", "source": "permissions.set_profile"}),
            "shell permission enabled",
        )?;
    } else if updates
        .get("permissions.shell_access")
        .and_then(Value::as_bool)
        == Some(false)
    {
        revoke_system_grant_if_present(
            state,
            "shell",
            "shell",
            "shell:execute",
            "shell permission disabled",
        )?;
    }
    if let Some(roots) = updates.get("permissions.local_roots") {
        for root in string_array_value(roots, "permissions.local_roots")? {
            ensure_system_grant(
                state,
                "filesystem",
                &root,
                "filesystem:root",
                json!({"root": root, "source": "permissions.set_profile"}),
                "local root granted",
            )?;
        }
    }
    if let Some(grants) = updates.get("permissions.local_filesystem_grants") {
        for root in string_array_value(grants, "permissions.local_filesystem_grants")? {
            ensure_system_grant(
                state,
                "filesystem",
                &root,
                "filesystem:access",
                json!({"root": root, "source": "permissions.set_profile"}),
                "local filesystem grant enabled",
            )?;
        }
    }
    if let Some(grants) = updates.get("permissions.google_drive_grants") {
        for drive_scope in string_array_value(grants, "permissions.google_drive_grants")? {
            ensure_system_grant(
                state,
                "cloud_drive",
                &drive_scope,
                "cloud-drive:access",
                json!({"drive_scope": drive_scope, "source": "permissions.set_profile"}),
                "cloud drive grant enabled",
            )?;
        }
    }
    Ok(())
}

fn ensure_system_grant(
    state: &BrainState,
    grant_kind: &str,
    subject_id: &str,
    capability_id: &str,
    scope: Value,
    reason: &str,
) -> Result<Value, RpcError> {
    let input = GrantInput {
        grant_kind: grant_kind.to_string(),
        subject_id: subject_id.to_string(),
        grantee_id: SYSTEM_GRANTEE.to_string(),
        capability_id: Some(capability_id.to_string()),
        scope,
        issuer: "permissions.set_profile".to_string(),
        reason: reason.to_string(),
        expires_at_s: None,
    };
    if let Some(row) = active_matching_grant(state, &input)? {
        return Ok(row);
    }
    create_or_refresh_grant(state, &input, "permissions.set_profile")
}

fn reconcile_boolean_grant(
    state: &BrainState,
    enabled: bool,
    grant_kind: &str,
    subject_id: &str,
    capability_id: &str,
    scope: Value,
    enabled_reason: &str,
    disabled_reason: &str,
) -> Result<(), RpcError> {
    if enabled {
        if !grant_history_exists(state, grant_kind, subject_id, capability_id)? {
            ensure_system_grant(
                state,
                grant_kind,
                subject_id,
                capability_id,
                scope,
                enabled_reason,
            )?;
        }
    } else {
        revoke_system_grant_if_present(
            state,
            grant_kind,
            subject_id,
            capability_id,
            disabled_reason,
        )?;
    }
    Ok(())
}

fn grant_history_exists(
    state: &BrainState,
    grant_kind: &str,
    subject_id: &str,
    capability_id: &str,
) -> Result<bool, RpcError> {
    let conn = state.store.conn()?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM grant_ledger_entries
             WHERE grant_kind = ?1
               AND subject_id = ?2
               AND grantee_id = ?3
               AND COALESCE(capability_id, '') = ?4",
            params![grant_kind, subject_id, SYSTEM_GRANTEE, capability_id],
            |row| row.get(0),
        )
        .map_err(|e| rpc_err(-32603, format!("grant_history_lookup: {e}")))?;
    Ok(count > 0)
}

fn revoke_system_grant(
    state: &BrainState,
    grant_kind: &str,
    subject_id: &str,
    capability_id: &str,
    reason: &str,
) -> Result<Vec<Value>, RpcError> {
    revoke_grants(
        state,
        &json!({
            "grant_kind": grant_kind,
            "subject_id": subject_id,
            "grantee_id": SYSTEM_GRANTEE,
            "capability_id": capability_id
        }),
        reason,
        "permissions.revoke_root",
    )
}

fn revoke_system_grant_if_present(
    state: &BrainState,
    grant_kind: &str,
    subject_id: &str,
    capability_id: &str,
    reason: &str,
) -> Result<Vec<Value>, RpcError> {
    match revoke_system_grant(state, grant_kind, subject_id, capability_id, reason) {
        Ok(rows) => Ok(rows),
        Err(error) if error.message == "grant_not_found_or_inactive" => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn read_string(state: &BrainState, key: &str) -> Result<Option<String>, RpcError> {
    read_setting(state, key).map(|value| {
        value.and_then(|raw| {
            serde_json::from_str::<String>(&raw)
                .ok()
                .or_else(|| Some(raw.trim_matches('"').to_string()))
                .filter(|value| !value.trim().is_empty())
        })
    })
}

fn read_bool(state: &BrainState, key: &str) -> Result<Option<bool>, RpcError> {
    read_setting(state, key).and_then(|value| {
        value
            .map(|raw| {
                serde_json::from_str::<bool>(&raw)
                    .map_err(|e| rpc_err(-32603, format!("{key}_parse_bool: {e}")))
            })
            .transpose()
    })
}

fn read_string_array(state: &BrainState, key: &str) -> Result<Vec<String>, RpcError> {
    read_setting(state, key).and_then(|value| {
        value
            .map(|raw| {
                serde_json::from_str::<Vec<String>>(&raw)
                    .map_err(|e| rpc_err(-32603, format!("{key}_parse_array: {e}")))
            })
            .transpose()
            .map(|value| value.unwrap_or_default())
    })
}

fn read_setting(state: &BrainState, key: &str) -> Result<Option<String>, RpcError> {
    let conn = state.store.conn()?;
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| rpc_err(-32603, format!("permission_setting_lookup: {e}")))
}

fn profile_rank(profile: &str) -> i64 {
    match profile {
        PROFILE_RESTRICTED => 0,
        PROFILE_WORKSPACE => 1,
        PROFILE_FULL_ACCESS => 2,
        _ => -1,
    }
}

fn validate_profile(profile: &str) -> Result<(), RpcError> {
    if profile_rank(profile) < 0 {
        return Err(rpc_err(
            -32602,
            format!("invalid_permission_profile: {profile}"),
        ));
    }
    Ok(())
}

fn bool_value(value: &Value, key: &str) -> Result<bool, RpcError> {
    value
        .as_bool()
        .ok_or_else(|| rpc_err(-32602, format!("{key}_must_be_boolean")))
}

fn string_array_value(value: &Value, key: &str) -> Result<Vec<String>, RpcError> {
    let values = value
        .as_array()
        .ok_or_else(|| rpc_err(-32602, format!("{key}_must_be_array")))?;
    let mut out = Vec::with_capacity(values.len());
    for item in values {
        let value = item
            .as_str()
            .ok_or_else(|| rpc_err(-32602, format!("{key}_must_contain_strings")))?;
        if value.trim().is_empty() {
            return Err(rpc_err(-32602, format!("{key}_contains_empty_value")));
        }
        out.push(value.to_string());
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn validate_absolute_path(path: &str, field: &str) -> Result<(), RpcError> {
    if !Path::new(path).is_absolute() {
        return Err(rpc_err(-32602, format!("{field}_must_be_absolute")));
    }
    Ok(())
}

fn confirmation_present(params: &Value) -> bool {
    params
        .get("confirm_broadening")
        .or_else(|| params.get("confirmBroadening"))
        .or_else(|| params.get("confirm_purge"))
        .or_else(|| params.get("confirmPurge"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || params
            .get("confirmation")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
}

fn string_field(params: &Value, keys: &[&str]) -> Option<String> {
    first(params, keys)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
}

fn first<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| value.get(*key))
}

fn permission_error(message: &str, data: Value) -> RpcError {
    RpcError {
        code: ERR_PERMISSION_DENIED,
        message: message.to_string(),
        data: Some(data),
    }
}
