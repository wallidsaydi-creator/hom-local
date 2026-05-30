use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use hom_shared::{
    ERR_QUARANTINED_INPUT, EnvelopeVerifier, JsonRpcRequest, JsonRpcResponse, KnownClient,
    json_rpc_error, json_rpc_error_data,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

use crate::App;
use crate::services::security_policy;

const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

pub async fn serve(
    socket_path: PathBuf,
    known_clients: Vec<KnownClient>,
    app: App,
) -> anyhow::Result<()> {
    prepare_socket(&socket_path)?;
    let listener = UnixListener::bind(&socket_path)?;
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
    log_event(
        "info",
        "helper.ready",
        json!({"helper": "brain", "socket": socket_path.display().to_string()}),
    );

    let verifier = Arc::new(Mutex::new(EnvelopeVerifier::new(known_clients)));
    loop {
        if app.is_shutting_down() {
            break;
        }
        let (stream, _) = listener.accept().await?;
        let app = app.clone();
        let verifier = Arc::clone(&verifier);
        tokio::spawn(async move {
            if let Err(error) = handle_connection(stream, verifier, app).await {
                log_event(
                    "warn",
                    "ipc.connection.error",
                    json!({"helper": "brain", "err": error.kind().to_string()}),
                );
            }
        });
    }
    Ok(())
}

async fn handle_connection(
    stream: UnixStream,
    verifier: Arc<Mutex<EnvelopeVerifier>>,
    app: App,
) -> std::io::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            return Ok(());
        }
        let started = Instant::now();
        let method = extract_method_name(line.trim_end());
        let response = if line.len() > MAX_FRAME_BYTES {
            json_rpc_error(json!(null), -32700, "frame_too_large")
        } else {
            process_frame(line.trim_end(), Arc::clone(&verifier), &app).await
        };
        log_method(method.as_deref(), &response, started.elapsed().as_millis());
        let mut bytes = serde_json::to_vec(&response).map_err(std::io::Error::other)?;
        bytes.push(b'\n');
        write_half.write_all(&bytes).await?;
        write_half.flush().await?;
    }
}

async fn process_frame(
    line: &str,
    verifier: Arc<Mutex<EnvelopeVerifier>>,
    app: &App,
) -> JsonRpcResponse {
    let request: JsonRpcRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(_) => return json_rpc_error(json!(null), -32700, "parse_error"),
    };
    let required_scope = match required_scope(&request.method) {
        Some(scope) => scope,
        None => return json_rpc_error(request.id, -32601, "method_not_found"),
    };
    let verified = {
        let mut verifier = verifier.lock().await;
        verifier.verify_request(&request, Some(required_scope))
    };
    let identity = match verified {
        Ok(identity) => identity,
        Err(error) => {
            log_event(
                "warn",
                "ipc.envelope.reject",
                json!({"helper": "brain", "code": error.code()}),
            );
            return json_rpc_error(request.id, error.code(), error.to_string());
        }
    };

    let decision = security_policy::evaluate_request(&request.method, &request.params);
    if !decision.allowed {
        let mut data = decision.data();
        let ledger = app.record_security_decision(&identity.client_id, &request.method, &decision);
        if let Value::Object(ref mut map) = data {
            map.insert("client_id".to_string(), json!(identity.client_id));
            map.insert("method".to_string(), json!(request.method));
            if let Some(ledger) = ledger {
                map.insert("ledger".to_string(), ledger);
            }
        }
        log_event(
            "warn",
            "ipc.security.reject",
            json!({"helper": "brain", "rule_id": decision.rule_id}),
        );
        return json_rpc_error_data(
            request.id,
            ERR_QUARANTINED_INPUT,
            "security_policy_rejected",
            data,
        );
    }
    app.dispatch(&request).await
}

fn required_scope(method: &str) -> Option<&'static str> {
    match method {
        "system.health" | "system.ready" | "system.status" | "system.shutdown" => Some("system"),
        "memory.save" => Some("memory:save"),
        "memory.recall" | "memory.recall.smart" => Some("memory:recall"),
        "memory.open" => Some("memory:open"),
        "memory.answer" => Some("memory:answer"),
        "events.list" => Some("events:read"),
        "events.security" => Some("events:write"),
        "runtime.status" => Some("system"),
        "ledger.verify"
        | "ledger.repair_segmented"
        | "ledger.record_mutation"
        | "ledger.reconcile_mismatches" => Some("system"),
        "monitoring.snapshot" => Some("diagnostics:read"),
        "diagnostics.trust"
        | "diagnostics.recall"
        | "diagnostics.capability"
        | "paper.registry" => Some("diagnostics:read"),
        "security.saber_dry_run" | "security.canary_timeline" | "security.audit_log" => {
            Some("security:read")
        }
        "session.get" | "session.login" | "session.logout" | "session.compact" => Some("system"),
        "projects.list"
        | "projects.create"
        | "sessions.list"
        | "sessions.create"
        | "sessions.detail"
        | "sessions.set_active"
        | "sessions.rename"
        | "sessions.hierarchy"
        | "ui.contract.snapshot"
        | "ui.inspector.target" => Some("system"),
        "settings.get"
        | "settings.set"
        | "permissions.get"
        | "permissions.set_profile"
        | "permissions.grant_root"
        | "permissions.revoke_root"
        | "permissions.grants.list"
        | "permissions.grants.create"
        | "permissions.grants.revoke"
        | "permissions.grants.check"
        | "tool.check_gate" => Some("system"),
        "nightly.tree" | "nightly.dry_run" | "nightly.run" => Some("system"),
        "reasoning.artifacts"
        | "reasoning.bridge.list"
        | "reasoning.bridge.integrity"
        | "reasoning.causal_chain"
        | "reasoning.tool_quality" => Some("diagnostics:read"),
        "reasoning.run" | "reasoning.bridge.backfill_imports" | "reasoning.tool_event.record" => {
            Some("memory:save")
        }
        "gates.plan.current"
        | "gates.runtime.snapshot"
        | "gates.decisions.list"
        | "gates.evidence.list"
        | "gates.argument.inspect" => Some("gates:read"),
        "gates.prompt.create"
        | "gates.plan.propose"
        | "gates.plan.approve"
        | "gates.tasks.derive"
        | "gates.tasks.preflight"
        | "gates.amendment.request"
        | "gates.amendment.approve"
        | "gates.runtime.verify"
        | "gates.evidence.submit"
        | "gates.tasks.complete"
        | "gates.argument.build"
        | "gates.argument.accept" => Some("gates:write"),
        "import.list" | "import.pack" | "export.brain" | "brain.backup" | "brain.purge" => {
            Some("system")
        }
        _ => None,
    }
}

fn prepare_socket(socket_path: &PathBuf) -> std::io::Result<()> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }
    Ok(())
}

fn log_method(method: Option<&str>, response: &JsonRpcResponse, duration_ms: u128) {
    let ok = response.error.is_none();
    let mut extra = json!({
        "helper": "brain",
        "duration_ms": duration_ms,
        "ok": ok,
        "method": method.unwrap_or("unknown")
    });
    if let Some(error) = &response.error {
        extra["error_code"] = json!(error.code);
        extra["error_message"] = json!(error.message);
        extra["outcome"] = json!(classify_error(error));
        if let Some(rule_id) = error
            .data
            .as_ref()
            .and_then(|data| data.get("rule_id"))
            .and_then(Value::as_str)
        {
            extra["rule_id"] = json!(rule_id);
        }
    } else {
        extra["outcome"] = json!("success");
    }
    log_event("info", "ipc.method", extra);
}

fn extract_method_name(frame: &str) -> Option<String> {
    serde_json::from_str::<Value>(frame).ok().and_then(|value| {
        value
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

fn classify_error(error: &hom_shared::JsonRpcError) -> &'static str {
    match error.code {
        ERR_QUARANTINED_INPUT => "policy_reject",
        -32700 => "parse_error",
        -32601 => "method_not_found",
        -32603 => "internal_error",
        _ => "rpc_error",
    }
}

fn log_event(level: &str, event: &str, extra: Value) {
    let mut object = serde_json::Map::new();
    object.insert("ts".to_string(), json!(chrono::Utc::now().to_rfc3339()));
    object.insert("level".to_string(), json!(level));
    object.insert("event".to_string(), json!(event));
    if let Value::Object(extra) = extra {
        for (key, value) in extra {
            object.insert(key, value);
        }
    }
    eprintln!("{}", Value::Object(object));
}

#[cfg(test)]
mod tests {
    use super::*;
    use hom_shared::JsonRpcError;

    #[test]
    fn cognitive_methods_have_required_scope() {
        for method in [
            "system.health",
            "system.ready",
            "system.status",
            "system.shutdown",
            "memory.save",
            "memory.recall",
            "memory.recall.smart",
            "memory.open",
            "memory.answer",
            "events.list",
            "events.security",
            "diagnostics.trust",
            "diagnostics.recall",
            "diagnostics.capability",
            "paper.registry",
            "security.saber_dry_run",
            "security.canary_timeline",
            "security.audit_log",
            "session.get",
            "session.login",
            "session.logout",
            "session.compact",
            "runtime.status",
            "projects.list",
            "projects.create",
            "sessions.list",
            "sessions.create",
            "sessions.detail",
            "sessions.set_active",
            "sessions.rename",
            "sessions.hierarchy",
            "ui.contract.snapshot",
            "ui.inspector.target",
            "settings.get",
            "settings.set",
            "permissions.get",
            "permissions.set_profile",
            "permissions.grant_root",
            "permissions.revoke_root",
            "permissions.grants.list",
            "permissions.grants.create",
            "permissions.grants.revoke",
            "permissions.grants.check",
            "tool.check_gate",
            "nightly.tree",
            "nightly.dry_run",
            "nightly.run",
            "reasoning.artifacts",
            "reasoning.bridge.list",
            "reasoning.bridge.backfill_imports",
            "reasoning.tool_quality",
            "reasoning.run",
            "reasoning.causal_chain",
            "gates.prompt.create",
            "gates.plan.propose",
            "gates.plan.approve",
            "gates.plan.current",
            "gates.tasks.derive",
            "gates.tasks.preflight",
            "gates.amendment.request",
            "gates.amendment.approve",
            "gates.runtime.snapshot",
            "gates.runtime.verify",
            "gates.decisions.list",
            "gates.evidence.submit",
            "gates.evidence.list",
            "gates.tasks.complete",
            "gates.argument.build",
            "gates.argument.inspect",
            "gates.argument.accept",
            "import.list",
            "import.pack",
            "export.brain",
            "brain.backup",
            "brain.purge",
        ] {
            assert!(required_scope(method).is_some(), "{method}");
        }
    }

    #[test]
    fn non_cognitive_runtime_methods_are_not_brain_methods() {
        for method in [
            "chat.send",
            "providers.list",
            "skills.list",
            "tools.list",
            "plugins.list",
            "auth.request",
        ] {
            assert_eq!(required_scope(method), None, "{method}");
        }
    }

    #[test]
    fn extracts_method_name_from_json_rpc_frame() {
        let method = extract_method_name(
            r#"{"jsonrpc":"2.0","id":"1","method":"runtime.status","params":{}}"#,
        );
        assert_eq!(method.as_deref(), Some("runtime.status"));
    }

    #[test]
    fn classifies_quarantined_input_as_policy_reject() {
        let error = JsonRpcError {
            code: ERR_QUARANTINED_INPUT,
            message: "security_policy_rejected".to_string(),
            data: Some(json!({"rule_id": "F_SECRET_EXFILTRATION"})),
        };
        assert_eq!(classify_error(&error), "policy_reject");
    }
}
