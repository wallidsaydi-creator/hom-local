use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::Timelike;
use futures_util::FutureExt;
use hom_shared::{RpcError, RpcResult, rpc_err};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, oneshot};

use crate::BrainState;
use crate::ipc::router::{
    QueueCaps, QueueSnapshot, WeightedQueue, WorkerKind, priority_for_method, worker_for_method,
};
use crate::services::{
    admin, exposure_policy, gates, import_adapter, import_batch, import_commit, ledger, memory,
    monitoring, nightly, paper_registry, permissions, projects, provider_catalog, reasoning_bridge,
    route_certificate, runtime, session, sessions, settings, ui_contract,
};

#[derive(Clone)]
pub struct BrainRuntime {
    mailboxes: Arc<HashMap<WorkerKind, Arc<WorkerMailbox>>>,
}

struct WorkerMailbox {
    queue: Mutex<WeightedQueue<WorkerJob>>,
    notify: Notify,
}

struct WorkerJob {
    method: String,
    params: Value,
    response: oneshot::Sender<RpcResult<Value>>,
}

impl BrainRuntime {
    pub fn new(state: Arc<BrainState>) -> Self {
        let mut mailboxes = HashMap::new();
        for kind in [
            WorkerKind::Save,
            WorkerKind::Recall,
            WorkerKind::Cognition,
            WorkerKind::Io,
            WorkerKind::Main,
        ] {
            let mailbox = Arc::new(WorkerMailbox {
                queue: Mutex::new(WeightedQueue::new(QueueCaps::default())),
                notify: Notify::new(),
            });
            tokio::spawn(worker_loop(kind, Arc::clone(&state), Arc::clone(&mailbox)));
            mailboxes.insert(kind, mailbox);
        }

        let refresh_state = Arc::clone(&state);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            refresh_state.store.refresh_ledger_cache();
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                interval.tick().await;
                if refresh_state.shutting_down.load(Ordering::SeqCst) {
                    break;
                }
                refresh_state.store.refresh_ledger_cache();
            }
        });

        let nightly_state = Arc::clone(&state);
        tokio::spawn(async move {
            loop {
                let enabled = read_nightly_setting_bool(&nightly_state, "nightly.enabled", true);
                if !enabled {
                    tokio::time::sleep(Duration::from_secs(300)).await;
                    continue;
                }
                let hour = read_nightly_setting_i64(&nightly_state, "nightly.hour", 0).clamp(0, 23);
                let now = chrono::Local::now();
                let target = dst_safe_hour(hour as u32, now).unwrap_or(now);
                let target = if target <= now {
                    let next_day = now + chrono::Duration::days(1);
                    dst_safe_hour(hour as u32, next_day)
                        .unwrap_or_else(|| next_day + chrono::Duration::hours(1))
                } else {
                    target
                };
                let sleep_duration = (target - now).to_std().unwrap_or(Duration::from_secs(3600));
                tokio::time::sleep(sleep_duration).await;
                if nightly_state.shutting_down.load(Ordering::SeqCst) {
                    break;
                }
                match nightly::run(&nightly_state, json!({"apply": true, "approved": true})) {
                    Ok(value) => eprintln!(
                        "nightly_auto_run_ok: status={} proposals={} applied={}",
                        value.get("status").and_then(Value::as_str).unwrap_or("?"),
                        value
                            .get("proposal_count")
                            .and_then(Value::as_i64)
                            .unwrap_or(0),
                        value
                            .get("applied_count")
                            .and_then(Value::as_i64)
                            .unwrap_or(0),
                    ),
                    Err(error) => eprintln!("nightly_auto_run_error: {:?}", error),
                }
            }
        });

        Self {
            mailboxes: Arc::new(mailboxes),
        }
    }

    pub async fn route(&self, method: &str, params: Value) -> RpcResult<Value> {
        let kind = worker_for_method(method).ok_or_else(|| rpc_err(-32601, "method_not_found"))?;
        let priority = priority_for_method(method, &params);
        let mailbox = self
            .mailboxes
            .get(&kind)
            .ok_or_else(|| rpc_err(-32601, "worker_not_found"))?;
        let (tx, rx) = oneshot::channel();

        {
            let mut queue = mailbox.queue.lock().await;
            let retry_after_ms = queue.estimate_wait_ms(priority, 64.0);
            if let Err(error) = queue.push(
                priority,
                WorkerJob {
                    method: method.to_string(),
                    params,
                    response: tx,
                },
            ) {
                return Err(error.into_rpc(retry_after_ms));
            }
        }

        mailbox.notify.notify_one();
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(rpc_err(-32603, "worker_channel_closed")),
            Err(_) => Err(rpc_err(-32603, "worker_timeout")),
        }
    }

    pub async fn queue_snapshot(&self) -> Value {
        let mut out = serde_json::Map::new();
        for (kind, mailbox) in self.mailboxes.iter() {
            let snapshot: QueueSnapshot = mailbox.queue.lock().await.snapshot();
            out.insert(format!("{kind:?}").to_lowercase(), json!(snapshot));
        }
        Value::Object(out)
    }
}

async fn worker_loop(kind: WorkerKind, state: Arc<BrainState>, mailbox: Arc<WorkerMailbox>) {
    loop {
        mailbox.notify.notified().await;
        loop {
            let job = {
                let mut queue = mailbox.queue.lock().await;
                queue.pop()
            };
            let Some(job) = job else { break };
            let result = AssertUnwindSafe(execute_worker_method(
                kind,
                Arc::clone(&state),
                &job.method,
                job.params,
            ))
            .catch_unwind()
            .await
            .unwrap_or_else(|_| Err(rpc_err(-32603, "worker_panic")));
            let _ = job.response.send(result);
        }
    }
}

async fn execute_worker_method(
    _kind: WorkerKind,
    state: Arc<BrainState>,
    method: &str,
    params: Value,
) -> Result<Value, RpcError> {
    match method {
        "memory.save" => {
            gates::enforce_attached_preflight(&state, &params, "memory.save")?;
            memory::save(&state, params)
        }
        "memory.embedding.upsert" => {
            gates::enforce_attached_preflight(&state, &params, "memory.embedding.upsert")?;
            memory::embedding_upsert(&state, params)
        }
        "memory.recall" | "memory.recall.smart" => memory::recall(&state, params),
        "memory.open" => memory::open(&state, params),
        "memory.answer" => memory::answer(&state, params),
        "events.list" => memory::events(&state, params),
        "events.security" => memory::events_security(&state, params),
        "diagnostics.trust" => memory::diagnostics_trust(&state),
        "diagnostics.recall" => memory::diagnostics_recall(&state, params),
        "diagnostics.recall_drift" => memory::diagnostics_recall_drift(&state, params),
        "diagnostics.capability" => runtime::capability(&state, params),
        "paper.registry" => Ok(paper_registry::list(params)),
        "route.certificate.issue" => route_certificate::issue(&state, params),
        "route.certificate.open" => route_certificate::open(&state, params),
        "exposure.policy" => Ok(exposure_policy::list(params)),
        "security.saber_dry_run" => memory::security_saber_dry_run(),
        "security.canary_timeline" => memory::security_canary_timeline(&state, params),
        "security.audit_log" => memory::security_audit_log(&state, params),
        "session.get" => session::get(&state),
        "session.login" => session::login(&state, params),
        "session.logout" => session::logout(&state),
        "session.compact" => session::compact(&state, params),
        "session.compactions" => session::compactions(&state, params),
        "session.compaction.open" => session::compaction_open(&state, params),
        "session.compaction_snapshot" => session::compaction_snapshot(&state, params),
        "session.context_pressure" => provider_catalog::context_pressure(params),
        "providers.model_catalog" => provider_catalog::model_catalog(params),
        "runtime.status" => runtime::status(&state).await,
        "ledger.verify" => ledger::verify(&state),
        "ledger.repair_segmented" => ledger::repair_segmented(&state, params),
        "ledger.record_mutation" => ledger::record_mutation(&state, params),
        "ledger.reconcile_mismatches" => ledger::reconcile_mismatches(&state, params),
        "monitoring.snapshot" => monitoring::snapshot(&state),
        "benchmarks.list" => admin::benchmarks_list(&state, params),
        "benchmarks.detail" => admin::benchmarks_detail(&state, params),
        "benchmarks.record" => admin::benchmarks_record(&state, params),
        "benchmarks.vector_exact" => admin::vector_exact_benchmark(&state, params),
        "benchmarks.vector_pq_candidates" => admin::vector_pq_candidates(&state, params),
        "benchmarks.vector_pq_exact_rerank" => admin::vector_pq_exact_rerank(&state, params),
        "benchmarks.vector_recall" => admin::vector_recall_benchmark(&state, params),
        "benchmarks.vector_thresholds" => admin::vector_thresholds(&state, params),
        "benchmarks.vector_regression_profile" => admin::vector_regression_profile(&state, params),
        "benchmarks.vector_regression_compare" => admin::vector_regression_compare(&state, params),
        "benchmarks.vector_regression_response" => {
            admin::vector_regression_response(&state, params)
        }
        "policy.vector_recall" => admin::vector_recall_policy(&state, params),
        "projects.list" => projects::list(&state),
        "projects.create" => projects::create(&state, params),
        "sessions.list" => sessions::list(&state, params),
        "sessions.create" => sessions::create(&state, params),
        "sessions.detail" => sessions::detail(&state, params),
        "sessions.set_active" => sessions::set_active(&state, params),
        "sessions.rename" => sessions::rename(&state, params),
        "sessions.hierarchy" => sessions::hierarchy(&state, params),
        "ui.contract.snapshot" => ui_contract::snapshot(&state, params).await,
        "ui.inspector.target" => ui_contract::inspector_target(&state, params),
        "nightly.tree" => nightly::tree(&state),
        "nightly.dry_run" => nightly::dry_run(&state, params),
        "nightly.run" => {
            gates::enforce_attached_preflight(&state, &params, "nightly.run")?;
            permissions::enforce_automation(&state)?;
            nightly::run(&state, params)
        }
        "reasoning.artifacts" => reasoning_bridge::artifacts(&state, params),
        "reasoning.bridge.list" => reasoning_bridge::list(&state, params),
        "reasoning.bridge.integrity" => reasoning_bridge::bridge_integrity(&state, params),
        "reasoning.bridge.backfill_imports" => reasoning_bridge::backfill_import_bridges(&state),
        "reasoning.tool_quality" => reasoning_bridge::tool_quality(&state, params),
        "reasoning.tool_event.record" => reasoning_bridge::record_tool_invocation(&state, params),
        "reasoning.run" => reasoning_bridge::run(&state, params),
        "reasoning.causal_chain" => reasoning_bridge::causal_chain(&state, params),
        "settings.get" => settings::get(&state).await,
        "settings.set" => {
            gates::enforce_attached_preflight(&state, &params, "settings.set")?;
            settings::set(&state, params)
        }
        "permissions.get" => permissions::get(&state),
        "permissions.set_profile" => permissions::set_profile(&state, params),
        "permissions.grant_root" => permissions::grant_root(&state, params),
        "permissions.revoke_root" => permissions::revoke_root(&state, params),
        "permissions.grants.list" => permissions::grants_list(&state, params),
        "permissions.grants.create" => permissions::grants_create(&state, params),
        "permissions.grants.revoke" => permissions::grants_revoke(&state, params),
        "permissions.grants.check" => permissions::grants_check(&state, params),
        "tool.check_gate" => {
            let domain = params
                .get("domain")
                .and_then(|v| v.as_str())
                .ok_or_else(|| rpc_err(-32602, "domain_required"))?;
            let context = params.get("context");
            permissions::enforce_tool_gate(&state, domain, context)?;
            Ok(json!({"ok": true, "domain": domain, "allowed": true}))
        }
        "gates.prompt.create" => gates::prompt_create(&state, params),
        "gates.plan.propose" => gates::plan_propose(&state, params),
        "gates.plan.approve" => gates::plan_approve(&state, params),
        "gates.plan.current" => gates::plan_current(&state),
        "gates.tasks.derive" => gates::tasks_derive(&state, params),
        "gates.tasks.preflight" => gates::tasks_preflight(&state, params),
        "gates.amendment.request" => gates::amendment_request(&state, params),
        "gates.amendment.approve" => gates::amendment_approve(&state, params),
        "gates.runtime.snapshot" => gates::runtime_snapshot(&state),
        "gates.runtime.verify" => gates::runtime_verify(&state, params),
        "gates.decisions.list" => gates::decisions_list(&state, params),
        "gates.evidence.submit" => gates::evidence_submit(&state, params),
        "gates.evidence.list" => gates::evidence_list(&state, params),
        "gates.tasks.complete" => gates::tasks_complete(&state, params),
        "gates.argument.build" => gates::argument_build(&state, params),
        "gates.argument.inspect" => gates::argument_inspect(&state, params),
        "gates.argument.accept" => gates::argument_accept(&state, params),
        "import.list" => admin::import_list(&state),
        "import.pack" => {
            gates::enforce_attached_preflight(&state, &params, "import.pack")?;
            admin::import_pack(&state, params)
        }
        "import.memory.sources" => import_batch::import_sources(&state),
        "import.memory.batch.create" => {
            gates::enforce_attached_preflight(&state, &params, "import.memory.batch.create")?;
            import_batch::create_batch(&state, params)
        }
        "import.memory.batch.get" => import_batch::get_batch(&state, params),
        "import.memory.batch.list" => import_batch::list_batches(&state, params),
        "import.memory.batch.parse" => {
            gates::enforce_attached_preflight(&state, &params, "import.memory.batch.parse")?;
            import_adapter::parse_batch(&state, params)
        }
        "import.memory.batch.preview" => import_adapter::preview_batch(&state, params),
        "import.memory.batch.commit" => {
            gates::enforce_attached_preflight(&state, &params, "import.memory.batch.commit")?;
            import_commit::commit_batch(&state, params)
        }
        "import.memory.batch.cancel" => import_batch::cancel_batch(&state, params),
        "import.memory.batch.report" => import_batch::batch_report(&state, params),
        "export.brain" => admin::export_brain(&state),
        "brain.backup" => {
            gates::enforce_attached_preflight(&state, &params, "brain.backup")?;
            admin::backup(&state)
        }
        "brain.purge" => {
            gates::enforce_attached_preflight(&state, &params, "brain.purge")?;
            permissions::enforce_purge(&state, &params)?;
            admin::purge(&state)
        }
        _ => Err(rpc_err(-32601, "method_not_found")),
    }
}

/// Returns a DateTime at the given hour (0-23) on the same day as `base`,
/// falling back to neighboring hours if the target hour is skipped by DST.
fn dst_safe_hour(
    hour: u32,
    base: chrono::DateTime<chrono::Local>,
) -> Option<chrono::DateTime<chrono::Local>> {
    for offset in 0..3 {
        let h = (hour + offset).min(23);
        if let Some(t) = base
            .with_hour(h)
            .and_then(|t| t.with_minute(0))
            .and_then(|t| t.with_second(0))
        {
            return Some(t);
        }
    }
    None
}

fn read_nightly_setting_bool(state: &BrainState, key: &str, default: bool) -> bool {
    state
        .store
        .conn()
        .ok()
        .and_then(|conn| {
            conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .ok()
            .flatten()
        })
        .and_then(|raw| serde_json::from_str::<bool>(&raw).ok())
        .unwrap_or(default)
}

fn read_nightly_setting_i64(state: &BrainState, key: &str, default: i64) -> i64 {
    state
        .store
        .conn()
        .ok()
        .and_then(|conn| {
            conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .ok()
            .flatten()
        })
        .and_then(|raw| serde_json::from_str::<i64>(&raw).ok())
        .unwrap_or(default)
}
