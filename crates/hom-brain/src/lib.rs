// SPDX-License-Identifier: Apache-2.0
#![recursion_limit = "256"]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde_json::{Value, json};

use hom_shared::{
    JsonRpcRequest, JsonRpcResponse, RpcResult, json_rpc_error, json_rpc_error_data,
    json_rpc_success,
};

pub mod db;
pub mod ipc;
pub mod services;
pub mod workers;

use crate::db::storage::LocalStore;
use crate::services::security_policy::SecurityDecision;
use crate::workers::BrainRuntime;

#[derive(Clone)]
pub struct App {
    state: Arc<BrainState>,
    runtime: BrainRuntime,
}

pub struct BrainState {
    pub hom_dir: PathBuf,
    pub store: LocalStore,
    pub started_at: Instant,
    pub shutting_down: AtomicBool,
}

impl App {
    pub fn new(hom_dir: PathBuf) -> anyhow::Result<Self> {
        let store = LocalStore::open(&hom_dir)?;
        let state = Arc::new(BrainState {
            hom_dir,
            store,
            started_at: Instant::now(),
            shutting_down: AtomicBool::new(false),
        });
        crate::services::permissions::reconcile_system_grants(&state).map_err(|error| {
            anyhow::anyhow!(
                "permission_grant_reconcile_failed: {} ({})",
                error.message,
                error.code
            )
        })?;
        state.store.refresh_ledger_cache();
        let runtime = BrainRuntime::new(Arc::clone(&state));
        Ok(Self { state, runtime })
    }

    pub async fn dispatch(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id.clone();
        match self
            .dispatch_value(&request.method, request.params.clone())
            .await
        {
            Ok(result) => json_rpc_success(id, result),
            Err(error) => match error.data {
                Some(data) => json_rpc_error_data(id, error.code, error.message, data),
                None => json_rpc_error(id, error.code, error.message),
            },
        }
    }

    pub fn record_security_decision(
        &self,
        client_id: &str,
        method: &str,
        decision: &SecurityDecision,
    ) -> Option<Value> {
        let ledger = self
            .state
            .store
            .record_security_event(client_id, method, decision.data())
            .ok();
        if let Some(ledger_value) = &ledger {
            let _ = crate::services::reasoning_bridge::record_security_decision(
                &self.state,
                client_id,
                method,
                &decision.data(),
                ledger_value.get("event_id").and_then(Value::as_str),
            );
        }
        ledger
    }

    pub async fn dispatch_value(&self, method: &str, params: Value) -> RpcResult<Value> {
        match method {
            "system.health" => Ok(self.system_health().await),
            "system.ready" => Ok(json!({
                "ok": true,
                "ready": true,
                "blocking": [],
                "db": self.state.store.db_path().display().to_string()
            })),
            "system.status" => Ok(self.system_status().await),
            "system.shutdown" => {
                self.state.shutting_down.store(true, Ordering::SeqCst);
                Ok(json!({"ok": true, "accepted": true}))
            }
            _ => self.runtime.route(method, params).await,
        }
    }

    pub fn is_shutting_down(&self) -> bool {
        self.state.shutting_down.load(Ordering::SeqCst)
    }

    async fn system_health(&self) -> Value {
        json!({
            "ok": true,
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_s": self.state.started_at.elapsed().as_secs(),
            "mem_rss_mb": 0,
            "queues": self.runtime.queue_snapshot().await,
            "pause_flags": false
        })
    }

    async fn system_status(&self) -> Value {
        let mut memory_count: i64 = 0;
        let mut ledger_count: i64 = 0;
        let mut head_hash: Option<String> = None;
        let mut alpha_pack_count: i64 = 0;
        let mut source_counts: Vec<Value> = Vec::new();

        if let Ok(conn) = self.state.store.conn() {
            memory_count = conn
                .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
                .unwrap_or(0);
            ledger_count = conn
                .query_row("SELECT COUNT(*) FROM ledger_events", [], |row| row.get(0))
                .unwrap_or(0);
            head_hash = conn
                .query_row(
                    "SELECT event_hash FROM ledger_events ORDER BY id DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .ok();
            alpha_pack_count = conn
                .query_row(
                    "SELECT COUNT(*) FROM memories WHERE source = 'local-pack'",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            if let Ok(mut stmt) = conn.prepare(
                "SELECT source, COUNT(*) FROM memories GROUP BY source ORDER BY COUNT(*) DESC",
            ) {
                if let Ok(rows) = stmt.query_map([], |row| {
                    let source: String = row.get(0)?;
                    let count: i64 = row.get(1)?;
                    Ok(json!({
                        "sourceKind": source,
                        "count": count
                    }))
                }) {
                    source_counts = rows.filter_map(Result::ok).collect();
                }
            }
        }
        let ledger_verification = self.state.store.verify_ledger().unwrap_or_else(|error| {
            json!({
                "valid": false,
                "total_events": ledger_count,
                "head_hash": head_hash,
                "last_verified_event_id": Value::Null,
                "first_invalid": {
                    "reason": "ledger_verification_unavailable",
                    "error": error.to_string()
                }
            })
        });

        json!({
            "ok": true,
            "memories": memory_count,
            "ledger": {
                "valid": ledger_verification.get("valid").cloned().unwrap_or_else(|| json!(false)),
                "totalEvents": ledger_verification.get("total_events").cloned().unwrap_or_else(|| json!(ledger_count)),
                "checkedEvents": ledger_verification.get("checked_events").cloned().unwrap_or_else(|| json!(0)),
                "headHash": ledger_verification.get("head_hash").cloned().unwrap_or(Value::Null),
                "verifiedHeadHash": ledger_verification.get("verified_head_hash").cloned().unwrap_or(Value::Null),
                "lastVerifiedEventId": ledger_verification.get("last_verified_event_id").cloned().unwrap_or(Value::Null),
                "firstInvalid": ledger_verification.get("first_invalid").cloned().unwrap_or(Value::Null),
                "algorithm": ledger_verification.get("algorithm").cloned().unwrap_or(Value::Null)
            },
            "daemon": {
                "baseUrl": "http://127.0.0.1:9101"
            },
            "corpus": {
                "memoryCount": memory_count,
                "localPackMemories": alpha_pack_count,
                "alphaPackTotalRows": alpha_pack_count,
                "sourceCounts": source_counts
            },
            "autonomous_loops": {
                "nightly_dream": "paused",
                "kl_alignment": "paused",
                "contrastive_retraining": "paused",
                "auto_weight_update": "paused",
                "hippograph_maintenance": "paused",
                "agent_trust_scoring": "paused"
            },
            "drift_alarms": {"g14": false, "g15": false, "g16": false, "last_triggered_at": null},
            "latest_snapshot": null,
            "agent_trust_summary": {"high": 0, "normal": 0, "quarantined": 0},
            "continuity": {"last_compaction_at": null, "reconstitution_p95_ms": 0}
        })
    }
}
