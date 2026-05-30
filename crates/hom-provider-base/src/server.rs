use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::{SystemTime, UNIX_EPOCH};

use hom_shared::{
    ERR_BREAKER_OPEN, ERR_QUARANTINED_INPUT, EnvelopeVerifier, JsonRpcRequest, KnownClient,
    build_request, ensure_helper_identity, hom_local_dir, json_rpc_error, json_rpc_error_data,
    json_rpc_success, known_clients_path, load_known_clients, sha256_b64u, uds,
};
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{Mutex, mpsc};

use crate::{
    ChatCompleteParams, ModelsListParams, ProbeParams, ProgressEvent, ProgressSink, Provider,
    StateMap,
};

static STARTED_AT_MS: LazyLock<u64> = LazyLock::new(now_ms);

#[derive(Clone, Debug, Default)]
pub struct ProviderServerConfig {
    pub socket_path: Option<PathBuf>,
    pub known_clients: Vec<KnownClient>,
    pub require_signed: bool,
    pub security_ledger: Option<ProviderSecurityLedgerConfig>,
}

#[derive(Clone, Debug)]
pub struct ProviderSecurityLedgerConfig {
    pub brain_socket_path: PathBuf,
    pub client_id: String,
    pub client_pub: String,
    pub private_key: String,
}

impl ProviderServerConfig {
    pub fn from_env(helper_name: &str) -> Self {
        let socket_path = std::env::var("HOM_SOCKET_PATH")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| default_socket_path(helper_name));
        let hom_dir = hom_local_dir();
        let known_clients_file = std::env::var("HOM_KNOWN_CLIENTS")
            .map(PathBuf::from)
            .unwrap_or_else(|_| known_clients_path(&hom_dir));
        let known_clients = load_known_clients(known_clients_file)
            .ok()
            .unwrap_or_default();
        let security_ledger = std::env::var("HOM_BRAIN_SOCKET")
            .or_else(|_| std::env::var("HOM_BRAIN_SOCK"))
            .ok()
            .and_then(|brain_socket_path| {
                let client_id = std::env::var("HOM_PROVIDER_CLIENT_ID")
                    .unwrap_or_else(|_| format!("{helper_name}.local"));
                let identity = ensure_helper_identity(
                    &hom_dir,
                    helper_name,
                    &client_id,
                    &["system", "events:write"],
                )
                .ok()?;
                Some(ProviderSecurityLedgerConfig {
                    brain_socket_path: PathBuf::from(brain_socket_path),
                    client_id,
                    client_pub: std::env::var("HOM_PROVIDER_PUBLIC_KEY")
                        .unwrap_or(identity.public_key),
                    private_key: std::env::var("HOM_PROVIDER_PRIVATE_KEY")
                        .unwrap_or(identity.private_key),
                })
            });

        Self {
            socket_path: Some(socket_path),
            known_clients,
            require_signed: std::env::var("HOM_REQUIRE_SIGNED")
                .map(|value| value != "0")
                .unwrap_or(true),
            security_ledger,
        }
    }
}

pub async fn serve_over_uds<P: Provider>(provider: P, helper_name: &str) -> anyhow::Result<()> {
    let config = ProviderServerConfig::from_env(helper_name);
    serve_over_uds_with_config(provider, helper_name, config).await
}

pub async fn serve_over_uds_with_config<P: Provider>(
    provider: P,
    helper_name: &str,
    config: ProviderServerConfig,
) -> anyhow::Result<()> {
    let socket_path = config
        .socket_path
        .unwrap_or_else(|| default_socket_path(helper_name));
    prepare_socket(&socket_path)?;
    let listener = UnixListener::bind(&socket_path)?;
    chmod_socket(&socket_path)?;

    let provider = Arc::new(provider);
    let state = Arc::new(Mutex::new(StateMap::default()));
    let helper_name = Arc::new(helper_name.to_string());
    loop {
        let (stream, _) = listener.accept().await?;
        let provider = Arc::clone(&provider);
        let state = Arc::clone(&state);
        let known_clients = config.known_clients.clone();
        let require_signed = config.require_signed;
        let security_ledger = config.security_ledger.clone();
        let helper_name = Arc::clone(&helper_name);
        tokio::spawn(async move {
            let _ = handle_client(
                provider,
                state,
                known_clients,
                require_signed,
                security_ledger,
                helper_name,
                stream,
            )
            .await;
        });
    }
}

async fn handle_client<P: Provider>(
    provider: Arc<P>,
    state: Arc<Mutex<StateMap>>,
    known_clients: Vec<KnownClient>,
    require_signed: bool,
    security_ledger: Option<ProviderSecurityLedgerConfig>,
    helper_name: Arc<String>,
    stream: tokio::net::UnixStream,
) -> anyhow::Result<()> {
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let writer = Arc::new(Mutex::new(write_half));
    let mut verifier = EnvelopeVerifier::new(known_clients);

    while let Some(request) = uds::read_json_line::<JsonRpcRequest>(&mut reader).await? {
        let required_scope = scope_for_method(&request.method);
        if require_signed {
            if let Err(error) = verifier.verify_request(&request, required_scope) {
                let response = json_rpc_error(request.id.clone(), error.code(), error.to_string());
                write_response(&writer, &response).await?;
                continue;
            }
        }

        let response = dispatch(
            request,
            Arc::clone(&provider),
            Arc::clone(&state),
            Arc::clone(&writer),
            security_ledger.as_ref(),
            helper_name.as_str(),
        )
        .await;
        write_response(&writer, &response).await?;
    }
    Ok(())
}

async fn dispatch<P: Provider>(
    request: JsonRpcRequest,
    provider: Arc<P>,
    state: Arc<Mutex<StateMap>>,
    writer: Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
    security_ledger: Option<&ProviderSecurityLedgerConfig>,
    helper_name: &str,
) -> hom_shared::JsonRpcResponse {
    match request.method.as_str() {
        "system.health" => json_rpc_success(request.id, {
            let snapshot = state.lock().await.clone();
            json!({
            "ok": true,
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_s": now_ms().saturating_sub(*STARTED_AT_MS) / 1000,
            "mem_rss_mb": resident_memory_mb(),
            "queues": {
                "tracked_provider_states": snapshot.0.len()
            },
            "pause_flags": false,
            "metrics_observed": true
            })
        }),
        "system.manifest" => json_rpc_success(
            request.id,
            json!({
                "ok": true,
                "helper_name": helper_name,
                "provider_id": provider.provider_id(),
                "adapter_id": provider.adapter_id(),
                "protocol_family": provider.protocol_family(),
                "binary_path": std::env::current_exe()
                    .ok()
                    .map(|path| path.display().to_string()),
                "version": env!("CARGO_PKG_VERSION"),
                "capabilities": provider.capabilities(),
                "auth_kinds_supported": provider.auth_kinds_supported(),
                "permissions_required": provider.permissions_required(),
                "adapter_manifest": provider.adapter_manifest(),
                "helper_only": true,
                "capability_only": true,
                "creates_provider_rows": false,
                "creates_candidate_rows": false,
                "stores_raw_secret": false
            }),
        ),
        "system.ready" => json_rpc_success(
            request.id,
            json!({"ok": true, "ready": true, "blocking": []}),
        ),
        "system.shutdown" => {
            let id = request.id;
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                std::process::exit(0);
            });
            json_rpc_success(id, json!({"ok": true}))
        }
        "health.snapshot" => {
            let snapshot = state.lock().await.clone();
            json_rpc_success(
                request.id,
                serde_json::to_value(snapshot).unwrap_or_else(|_| json!({})),
            )
        }
        "health.probe" => {
            let params = match serde_json::from_value::<ProbeParams>(request.params.clone()) {
                Ok(params) => params,
                Err(error) => return json_rpc_error(request.id, -32602, error.to_string()),
            };
            let start = std::time::Instant::now();
            let result = provider.health_probe(params.clone()).await;
            let latency_ms = start.elapsed().as_millis() as u64;
            {
                let mut states = state.lock().await;
                states
                    .ensure(
                        format!("{}:__probe", params.provider_id),
                        provider.capabilities(),
                    )
                    .record_result(result.as_ref().is_ok_and(|r| r.ok), latency_ms, now_ms());
            }
            match result {
                Ok(result) => json_rpc_success(
                    request.id,
                    serde_json::to_value(result).unwrap_or_else(|_| json!({})),
                ),
                Err(error) => json_rpc_error(request.id, -32050, error.to_string()),
            }
        }
        "models.list" => {
            let params = match serde_json::from_value::<ModelsListParams>(request.params.clone()) {
                Ok(params) => params,
                Err(error) => return json_rpc_error(request.id, -32602, error.to_string()),
            };
            match provider.list_models(params).await {
                Ok(result) => json_rpc_success(
                    request.id,
                    serde_json::to_value(result).unwrap_or_else(|_| json!({})),
                ),
                Err(error) => json_rpc_error(request.id, -32050, error.to_string()),
            }
        }
        "chat.cancel" => json_rpc_success(request.id, json!({"ok": true})),
        "chat.complete" => {
            let params = match serde_json::from_value::<ChatCompleteParams>(request.params.clone())
            {
                Ok(params) => params,
                Err(error) => return json_rpc_error(request.id, -32602, error.to_string()),
            };
            if let Some(data) = provider_request_denial(&params) {
                let data =
                    with_security_ledger(data, security_ledger, helper_name, "chat.complete").await;
                return json_rpc_error_data(
                    request.id,
                    ERR_QUARANTINED_INPUT,
                    "provider_security_policy_rejected",
                    data,
                );
            }
            let key = format!("{}:{}", params.provider_id, params.model);
            {
                let mut states = state.lock().await;
                let provider_state = states.ensure(key.clone(), provider.capabilities());
                if !provider_state.breaker.can_call(now_ms()) {
                    return json_rpc_error(
                        request.id,
                        ERR_BREAKER_OPEN,
                        "provider breaker is open",
                    );
                }
            }

            let (progress, progress_task) = if params.stream {
                let (tx, rx) = mpsc::channel(256);
                (
                    Some(ProgressSink::new(tx)),
                    Some(tokio::spawn(write_progress(
                        Arc::clone(&writer),
                        request.id.clone(),
                        rx,
                    ))),
                )
            } else {
                (None, None)
            };

            let start = std::time::Instant::now();
            let result = provider.chat_complete(params, progress).await;
            let latency_ms = start.elapsed().as_millis() as u64;
            if let Some(task) = progress_task {
                let _ = task.await;
            }

            {
                let mut states = state.lock().await;
                states.ensure(key, provider.capabilities()).record_result(
                    result.is_ok(),
                    latency_ms,
                    now_ms(),
                );
            }

            match result {
                Ok(result) => {
                    if let Some(data) = provider_output_denial(&result.content) {
                        let data = with_security_ledger(
                            data,
                            security_ledger,
                            helper_name,
                            "chat.complete",
                        )
                        .await;
                        return json_rpc_error_data(
                            request.id,
                            ERR_QUARANTINED_INPUT,
                            "provider_security_policy_rejected",
                            data,
                        );
                    }
                    json_rpc_success(
                        request.id,
                        serde_json::to_value(result).unwrap_or_else(|_| json!({})),
                    )
                }
                Err(error) => json_rpc_error(request.id, -32050, error.to_string()),
            }
        }
        other => json_rpc_error(request.id, -32601, format!("unknown method: {other}")),
    }
}

async fn write_progress(
    writer: Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
    request_id: Value,
    mut rx: mpsc::Receiver<ProgressEvent>,
) {
    while let Some(event) = rx.recv().await {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "$/progress",
            "params": {
                "request_id": request_id,
                "event": event
            }
        });
        let _ = write_raw(&writer, &notification).await;
    }
}

async fn write_response(
    writer: &Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
    response: &hom_shared::JsonRpcResponse,
) -> std::io::Result<()> {
    write_raw(writer, response).await
}

async fn write_raw<T: serde::Serialize>(
    writer: &Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
    value: &T,
) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    line.push(b'\n');
    let mut writer = writer.lock().await;
    writer.write_all(&line).await?;
    writer.flush().await
}

fn scope_for_method(method: &str) -> Option<&'static str> {
    match method {
        "chat.complete" | "chat.cancel" => Some("providers:use"),
        "models.list" | "health.probe" | "health.snapshot" | "system.health"
        | "system.manifest" | "system.ready" => Some("providers:read"),
        "system.shutdown" => Some("admin:shutdown"),
        _ => None,
    }
}

fn default_socket_path(helper_name: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let path = PathBuf::from(home)
        .join(".hom")
        .join("local")
        .join("sockets")
        .join(format!("{helper_name}.sock"));
    if path.to_string_lossy().len() <= 100 {
        return path;
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
    PathBuf::from("/tmp")
        .join(format!("hom-{user}"))
        .join(format!("{helper_name}.sock"))
}

fn prepare_socket(socket_path: &PathBuf) -> std::io::Result<()> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }
    Ok(())
}

fn chmod_socket(socket_path: &PathBuf) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn resident_memory_mb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
        let page_size_kb = 4u64;
        return Some((pages * page_size_kb).div_ceil(1024));
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/bin/ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let kb = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u64>()
            .ok()?;
        return Some(kb.div_ceil(1024));
    }
    #[allow(unreachable_code)]
    None
}

async fn with_security_ledger(
    mut data: Value,
    config: Option<&ProviderSecurityLedgerConfig>,
    helper_name: &str,
    method: &str,
) -> Value {
    if let Some(ledger) = record_provider_security_event(config, helper_name, method, &data).await {
        if let Value::Object(ref mut map) = data {
            map.insert("ledger".to_string(), ledger);
        }
    }
    data
}

async fn record_provider_security_event(
    config: Option<&ProviderSecurityLedgerConfig>,
    helper_name: &str,
    method: &str,
    payload: &Value,
) -> Option<Value> {
    let config = config?;
    let nonce = next_security_nonce(helper_name);
    let request = build_request(
        json!(nonce.clone()),
        "events.security",
        json!({
            "event_type": "security.provider.rejected",
            "actor": helper_name,
            "method": method,
            "payload": payload
        }),
        config.client_id.clone(),
        config.client_pub.clone(),
        &config.private_key,
        vec!["events:write".to_string(), "system".to_string()],
        nonce,
        now_s(),
    )
    .ok()?;
    let response = uds::send_request(&config.brain_socket_path, &request)
        .await
        .ok()?;
    response.result
}

fn next_security_nonce(helper_name: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let input = format!(
        "{helper_name}:{}:{}",
        now_ms(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    sha256_b64u(input.as_bytes())
}

fn provider_request_denial(params: &ChatCompleteParams) -> Option<Value> {
    let mut text = params
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(tools) = &params.tools {
        text.push('\n');
        text.push_str(&serde_json::to_string(tools).unwrap_or_default());
    }

    if let Some(canaries) = detect_canaries(&text).into_non_empty() {
        return Some(json!({
            "rule_id": "F_PROVIDER_CANARY_EXECUTION",
            "operator": "F",
            "reason": "canary token reached provider execution boundary",
            "assurance_region": "provider_boundary",
            "source_anchor": "DARPA SABER: AI kill-chain red-team evaluation",
            "attack_class": "canary_propagation",
            "canary_stage": "EXECUTED",
            "canaries_found": canaries
        }));
    }

    if contains_secret_like(&text) {
        return Some(json!({
            "rule_id": "F_PROVIDER_SECRET_RELAY",
            "operator": "F",
            "reason": "secret-like prompt or tool payload cannot be relayed to a provider",
            "assurance_region": "provider_boundary",
            "source_anchor": "NIST SP 800-207: no implicit trust for non-person service calls",
            "attack_class": "sensitive_information_disclosure"
        }));
    }

    if params
        .tools
        .as_ref()
        .is_some_and(|tools| contains_unmanaged_execution_tool(tools))
    {
        return Some(json!({
            "rule_id": "F_UNMANAGED_PROVIDER_TOOL",
            "operator": "F",
            "reason": "provider tool payload requests unmanaged local execution capability",
            "assurance_region": "provider_boundary",
            "source_anchor": "DARPA CODORD: prohibited actions override permissions",
            "attack_class": "excessive_agency"
        }));
    }

    None
}

fn provider_output_denial(content: &str) -> Option<Value> {
    if let Some(canaries) = detect_canaries(content).into_non_empty() {
        return Some(json!({
            "rule_id": "F_PROVIDER_CANARY_OUTPUT",
            "operator": "F",
            "reason": "provider output attempted to relay a kill-chain canary",
            "assurance_region": "provider_boundary",
            "source_anchor": "DARPA SABER: AI kill-chain red-team evaluation",
            "attack_class": "canary_propagation",
            "canary_stage": "RELAYED",
            "canaries_found": canaries
        }));
    }
    if contains_secret_like(content) {
        return Some(json!({
            "rule_id": "F_PROVIDER_SECRET_OUTPUT",
            "operator": "F",
            "reason": "provider output contains secret-like material",
            "assurance_region": "provider_boundary",
            "source_anchor": "NIST SP 800-207: prevent sensitive data relay",
            "attack_class": "sensitive_information_disclosure"
        }));
    }
    None
}

trait NonEmptyVec<T> {
    fn into_non_empty(self) -> Option<Vec<T>>;
}

impl<T> NonEmptyVec<T> for Vec<T> {
    fn into_non_empty(self) -> Option<Vec<T>> {
        (!self.is_empty()).then_some(self)
    }
}

fn detect_canaries(value: &str) -> Vec<String> {
    let mut canaries = Vec::new();
    for token in value.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-')) {
        let Some(rest) = token.strip_prefix("SECRET-") else {
            continue;
        };
        if rest.len() == 8 && rest.chars().all(|ch| ch.is_ascii_hexdigit()) {
            let token = token.to_string();
            if !canaries.contains(&token) {
                canaries.push(token);
            }
        }
    }
    canaries
}

fn contains_secret_like(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("authorization: bearer ")
        || lower.contains("bearer eyj")
        || lower.contains("-----begin private key-----")
        || lower.contains("-----begin secret key-----")
        || contains_assignment_secret(&lower)
        || value
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .any(|token| {
                token.len() == 20 && (token.starts_with("AKIA") || token.starts_with("ASIA"))
            })
        || contains_prefixed_token(value)
}

fn contains_assignment_secret(lower: &str) -> bool {
    for key in [
        "api_key",
        "apikey",
        "api key",
        "secret_key",
        "secret key",
        "token",
        "password",
        "private_key",
        "private key",
        "client_secret",
        "client secret",
    ] {
        let Some(index) = lower.find(key) else {
            continue;
        };
        let tail = lower[index + key.len()..].trim_start();
        let value = if let Some(rest) = tail.strip_prefix('=') {
            rest
        } else if let Some(rest) = tail.strip_prefix(':') {
            rest
        } else if let Some(rest) = tail.strip_prefix("is ") {
            rest
        } else {
            continue;
        };
        let token = value
            .trim_start_matches(|ch| matches!(ch, '"' | '\'' | '`' | ' '))
            .split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')))
            .next()
            .unwrap_or("");
        if token.len() >= 16
            && token
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        {
            return true;
        }
    }
    false
}

fn contains_prefixed_token(value: &str) -> bool {
    value
        .split(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | '`' | ',' | ';'))
        .any(|token| {
            ["sk-", "ghp_", "github_pat_", "xoxb-", "xoxp-"]
                .iter()
                .any(|prefix| token.starts_with(prefix) && token.len() >= prefix.len() + 16)
        })
}

fn contains_unmanaged_execution_tool(tools: &Value) -> bool {
    let text = serde_json::to_string(tools)
        .unwrap_or_default()
        .to_ascii_lowercase();
    [
        "execute_shell",
        "shell_exec",
        "bash",
        "zsh",
        "powershell",
        "python_eval",
        "file_write",
        "filesystem",
        "rm -rf",
        "subprocess",
        "spawn_process",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ChatCompleteResult, ChatMessage, ModelsListParams, ModelsListResult, ProviderCapability,
        ProviderError,
    };
    use async_trait::async_trait;
    use hom_shared::{KnownClient, build_request, generate_keypair, uds};
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::time::{Duration, sleep};

    struct MockProvider {
        content: &'static str,
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn provider_id(&self) -> &'static str {
            "mock"
        }

        fn capabilities(&self) -> Vec<ProviderCapability> {
            vec![ProviderCapability::Chat]
        }

        async fn chat_complete(
            &self,
            params: ChatCompleteParams,
            _progress: Option<ProgressSink>,
        ) -> Result<ChatCompleteResult, ProviderError> {
            assert_eq!(params.messages[0].content, "ping");
            Ok(ChatCompleteResult {
                ok: true,
                latency_ms: 1,
                content: self.content.to_string(),
                usage: None,
            })
        }

        async fn list_models(
            &self,
            _params: ModelsListParams,
        ) -> Result<ModelsListResult, ProviderError> {
            Ok(ModelsListResult { models: vec![] })
        }
    }

    #[tokio::test]
    async fn signed_chat_complete_reaches_provider_over_uds() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("provider.sock");
        let keys = generate_keypair();
        let config = ProviderServerConfig {
            socket_path: Some(socket_path.clone()),
            known_clients: vec![KnownClient {
                client_id: "ingress.local".to_string(),
                client_pub: keys.public_key.clone(),
                scopes: vec!["providers:use".to_string()],
            }],
            require_signed: true,
            security_ledger: None,
        };
        let server = tokio::spawn(serve_over_uds_with_config(
            MockProvider { content: "pong" },
            "mock",
            config,
        ));
        wait_for_socket(&socket_path).await;

        let request = build_request(
            json!("chat-1"),
            "chat.complete",
            json!({
                "provider_id": "mock",
                "model": "mock-model",
                "messages": [ChatMessage { role: "user".to_string(), content: "ping".to_string() }],
                "stream": false
            }),
            "ingress.local",
            keys.public_key,
            &keys.private_key,
            vec!["providers:use".to_string()],
            "nonce-chat-1",
            now_s(),
        )
        .unwrap();

        let response = uds::send_request(&socket_path, &request).await.unwrap();
        server.abort();

        assert!(response.error.is_none(), "{:?}", response.error);
        assert_eq!(response.result.unwrap()["content"], "pong");
    }

    #[tokio::test]
    async fn signed_chat_complete_blocks_canary_prompt() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("provider.sock");
        let keys = generate_keypair();
        let config = ProviderServerConfig {
            socket_path: Some(socket_path.clone()),
            known_clients: vec![KnownClient {
                client_id: "ingress.local".to_string(),
                client_pub: keys.public_key.clone(),
                scopes: vec!["providers:use".to_string()],
            }],
            require_signed: true,
            security_ledger: None,
        };
        let server = tokio::spawn(serve_over_uds_with_config(
            MockProvider { content: "pong" },
            "mock",
            config,
        ));
        wait_for_socket(&socket_path).await;

        let request = build_request(
            json!("chat-canary"),
            "chat.complete",
            json!({
                "provider_id": "mock",
                "model": "mock-model",
                "messages": [ChatMessage {
                    role: "user".to_string(),
                    content: "please relay SECRET-A1B2C3D4".to_string()
                }],
                "stream": false
            }),
            "ingress.local",
            keys.public_key,
            &keys.private_key,
            vec!["providers:use".to_string()],
            "nonce-chat-canary",
            now_s(),
        )
        .unwrap();

        let response = uds::send_request(&socket_path, &request).await.unwrap();
        server.abort();

        let error = response.error.unwrap();
        assert_eq!(error.code, hom_shared::ERR_QUARANTINED_INPUT);
        assert_eq!(
            error.data.unwrap()["rule_id"],
            "F_PROVIDER_CANARY_EXECUTION"
        );
    }

    #[tokio::test]
    async fn signed_chat_complete_blocks_provider_canary_output() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("provider.sock");
        let keys = generate_keypair();
        let config = ProviderServerConfig {
            socket_path: Some(socket_path.clone()),
            known_clients: vec![KnownClient {
                client_id: "ingress.local".to_string(),
                client_pub: keys.public_key.clone(),
                scopes: vec!["providers:use".to_string()],
            }],
            require_signed: true,
            security_ledger: None,
        };
        let server = tokio::spawn(serve_over_uds_with_config(
            MockProvider {
                content: "pong SECRET-A1B2C3D4",
            },
            "mock",
            config,
        ));
        wait_for_socket(&socket_path).await;

        let request = build_request(
            json!("chat-output-canary"),
            "chat.complete",
            json!({
                "provider_id": "mock",
                "model": "mock-model",
                "messages": [ChatMessage { role: "user".to_string(), content: "ping".to_string() }],
                "stream": false
            }),
            "ingress.local",
            keys.public_key,
            &keys.private_key,
            vec!["providers:use".to_string()],
            "nonce-chat-output-canary",
            now_s(),
        )
        .unwrap();

        let response = uds::send_request(&socket_path, &request).await.unwrap();
        server.abort();

        let error = response.error.unwrap();
        assert_eq!(error.code, hom_shared::ERR_QUARANTINED_INPUT);
        assert_eq!(error.data.unwrap()["rule_id"], "F_PROVIDER_CANARY_OUTPUT");
    }

    #[tokio::test]
    async fn provider_security_rejection_is_persisted_in_brain_ledger() {
        let dir = tempdir().unwrap();
        let brain_socket = dir.path().join("brain.sock");
        let provider_socket = dir.path().join("provider.sock");
        let ingress_keys = generate_keypair();
        let provider_keys = generate_keypair();

        let brain_known = vec![
            KnownClient {
                client_id: "ingress.local".to_string(),
                client_pub: ingress_keys.public_key.clone(),
                scopes: vec!["system".to_string(), "events:read".to_string()],
            },
            KnownClient {
                client_id: "provider-mock.local".to_string(),
                client_pub: provider_keys.public_key.clone(),
                scopes: vec!["system".to_string(), "events:write".to_string()],
            },
        ];
        let app = hom_brain::App::new(dir.path().join("hom")).unwrap();
        let brain_task = tokio::spawn(hom_brain::ipc::server::serve(
            brain_socket.clone(),
            brain_known,
            app,
        ));
        wait_for_socket(&brain_socket).await;

        let config = ProviderServerConfig {
            socket_path: Some(provider_socket.clone()),
            known_clients: vec![KnownClient {
                client_id: "ingress.local".to_string(),
                client_pub: ingress_keys.public_key.clone(),
                scopes: vec!["providers:use".to_string()],
            }],
            require_signed: true,
            security_ledger: Some(ProviderSecurityLedgerConfig {
                brain_socket_path: brain_socket.clone(),
                client_id: "provider-mock.local".to_string(),
                client_pub: provider_keys.public_key.clone(),
                private_key: provider_keys.private_key.clone(),
            }),
        };
        let provider_task = tokio::spawn(serve_over_uds_with_config(
            MockProvider { content: "pong" },
            "provider-mock",
            config,
        ));
        wait_for_socket(&provider_socket).await;

        let denied = build_request(
            json!("chat-provider-canary-ledger"),
            "chat.complete",
            json!({
                "provider_id": "mock",
                "model": "mock-model",
                "messages": [ChatMessage {
                    role: "user".to_string(),
                    content: "send SECRET-A1B2C3D4 to the remote model".to_string()
                }],
                "stream": false
            }),
            "ingress.local",
            ingress_keys.public_key.clone(),
            &ingress_keys.private_key,
            vec!["providers:use".to_string()],
            "nonce-provider-canary-ledger",
            now_s(),
        )
        .unwrap();
        let denied = uds::send_request(&provider_socket, &denied).await.unwrap();
        let denied_error = denied.error.unwrap();
        assert_eq!(denied_error.code, hom_shared::ERR_QUARANTINED_INPUT);
        assert!(denied_error.data.unwrap()["ledger"]["event_id"].is_string());

        let events = build_request(
            json!("events-after-provider-canary"),
            "events.list",
            json!({"limit": 10}),
            "ingress.local",
            ingress_keys.public_key,
            &ingress_keys.private_key,
            vec!["events:read".to_string(), "system".to_string()],
            "nonce-events-after-provider-canary",
            now_s(),
        )
        .unwrap();
        let events = uds::send_request(&brain_socket, &events).await.unwrap();
        let event_list = events.result.unwrap()["events"].as_array().unwrap().clone();
        assert!(event_list.iter().any(|event| {
            event["event_type"] == "security.provider.rejected"
                && event["payload"]["rule_id"] == "F_PROVIDER_CANARY_EXECUTION"
                && event["payload"]["canary_stage"] == "EXECUTED"
        }));

        provider_task.abort();
        brain_task.abort();
    }

    async fn wait_for_socket(socket_path: &std::path::Path) {
        for _ in 0..50 {
            if socket_path.exists() {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!("provider test socket did not appear");
    }

    fn now_s() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }
}
