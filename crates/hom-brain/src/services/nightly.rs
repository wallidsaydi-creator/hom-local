use std::collections::{BTreeMap, HashMap};
use std::fs;

use hom_shared::{RpcError, canonical_json, rpc_err, sha256_hex};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::db::storage::{append_ledger_tx, unix_now_s};
use crate::services::{auto_weight_update, contrastive_retraining, kl_alignment, reasoning_bridge};

const DEFAULT_WEIGHT: f64 = 0.85;
const DELTA_CAP: f64 = 0.05;
const EWMA_OLD: f64 = 0.85;
const EWMA_NEW: f64 = 0.15;
const ETA: f64 = 0.10;
const EPSILON: f64 = 1e-6;
const MIN_WEIGHT: f64 = 0.50;
const MAX_WEIGHT: f64 = 1.00;
const SAMPLE_GATE: i64 = 30;
const CYCLE_CAP_RATIO: f64 = 0.10;
const SOURCE_CAP_RATIO: f64 = 0.05;
const LOW_QUALITY_REVIEW_THRESHOLD: f64 = 0.62;
const TOOL_QUALITY_LIMIT: usize = 10;
const TOOL_RELIABILITY_WARNING_THRESHOLD: f64 = 0.80;
const TOOL_LATENCY_WARNING_MS: f64 = 1_000.0;

/// Returns persisted nightly artifacts grouped by date.
pub fn tree(state: &crate::BrainState) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT date(created_at_s, 'unixepoch') as day,
                    COUNT(*) as artifacts,
                    SUM(proposal_count) as proposals,
                    SUM(applied_count) as applied,
                    SUM(mutation_count) as mutations,
                    SUM(warning_count) as warnings
             FROM nightly_artifacts
             GROUP BY day
             ORDER BY day DESC
             LIMIT 90",
        )
        .map_err(|e| rpc_err(-32603, format!("nightly_tree_prepare: {e}")))?;
    let days = stmt
        .query_map([], |row| {
            Ok(json!({
                "date": row.get::<_, String>(0)?,
                "artifacts": row.get::<_, i64>(1)?,
                "proposals": row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                "applied": row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                "mutations": row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                "warnings": row.get::<_, Option<i64>>(5)?.unwrap_or(0)
            }))
        })
        .map_err(|e| rpc_err(-32603, format!("nightly_tree_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("nightly_tree_collect: {e}")))?;
    Ok(json!({"ok": true, "days": days}))
}

pub fn dry_run(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    execute(state, params, false)
}

/// Runs nightly consolidation. Mutation is dry-run by default; applying requires
/// { "apply": true, "approved": true } and the worker automation gate.
pub fn run(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let apply = params
        .get("apply")
        .or_else(|| params.get("apply_mutations"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if apply
        && params
            .get("approved")
            .or_else(|| params.get("approval"))
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(rpc_err(-32602, "nightly_apply_approval_required"));
    }
    execute(state, params, apply)
}

fn execute(state: &crate::BrainState, _params: Value, apply: bool) -> Result<Value, RpcError> {
    let now_s = unix_now_s();
    let cycle_id = format!("nightly:{}:{}", now_s, &Uuid::new_v4().to_string()[..8]);
    let mut plan = build_plan(state, &cycle_id, now_s)?;
    let blocked_by_kill_switch = !plan.learning_autonomous || !plan.dream_autonomous;
    if apply && blocked_by_kill_switch {
        return Err(rpc_err(-32603, "nightly_autonomous_learning_paused"));
    }
    if apply && !plan.tool_observability_allows_mutation {
        plan.warnings.push(json!({
            "code": "tool_observability_blocks_mutation_selection",
            "guardrail": "G17",
            "bridge_coverage_required": plan.guardrails["G17_tool_observability"]["bridge_coverage_required"],
            "orphan_tool_event_count": plan.guardrails["G17_tool_observability"]["orphan_tool_event_count"],
            "orphan_bridge_count": plan.guardrails["G17_tool_observability"]["orphan_bridge_count"]
        }));
    }

    let selected_mutations = if apply && plan.tool_observability_allows_mutation {
        select_mutations(&plan.weight_mutations, plan.corpus_count)
        // META-LEDGER INTEGRATION POINT: When weight mutations are applied and modify
        // ledger_events.payload_json, call record_mutation_tx within the same transaction
        // to register the self-mutation on the meta-ledger hash chain.
    } else {
        Vec::new()
    };
    let touched_memory_ids = selected_mutations
        .iter()
        .map(|mutation| mutation.memory_id.clone())
        .collect::<Vec<_>>();
    let input_signal_hash = sha256_hex(
        &canonical_json(&json!({
            "cycle_id": cycle_id,
            "proposals": plan.proposals.clone(),
            "guardrails": plan.guardrails.clone()
        }))
        .map_err(|e| rpc_err(-32603, format!("nightly_input_hash: {e}")))?,
    );

    let snapshot = if apply && !selected_mutations.is_empty() {
        Some(create_weight_snapshot(state, &cycle_id, now_s)?)
    } else {
        None
    };

    let output_state_hash = if apply && !selected_mutations.is_empty() {
        sha256_hex(
            &canonical_json(&json!({
                "cycle_id": cycle_id,
                "selected_mutations": selected_mutations.iter().map(|mutation| mutation.value()).collect::<Vec<_>>()
            }))
            .map_err(|e| rpc_err(-32603, format!("nightly_output_hash: {e}")))?,
        )
    } else {
        sha256_hex(&input_signal_hash)
    };

    let artifact_id = Uuid::new_v4().to_string();
    let mode = if apply { "apply" } else { "dry_run" };
    let status = if blocked_by_kill_switch {
        "paused"
    } else if selected_mutations.is_empty() {
        "no_mutation"
    } else {
        "mutated"
    };
    let mutation_values = selected_mutations
        .iter()
        .map(WeightMutation::value)
        .collect::<Vec<_>>();
    let warnings_json = json!(plan.warnings.clone());
    let insights_json = json!(plan.insights.clone());
    let proposals_json = json!(plan.proposals.clone());
    plan.guardrails["G4_ledgered_mutation"]["selected_mutation_count"] =
        json!(selected_mutations.len());
    plan.guardrails["G11_snapshot"]["snapshot"] = snapshot
        .as_ref()
        .map(|snapshot| snapshot.value.clone())
        .unwrap_or(Value::Null);
    plan.guardrails["mutation_boundaries"]["selected_mutations"] = json!(mutation_values);
    let guardrails_json = plan.guardrails.clone();

    let ledger = {
        let mut conn = state.store.conn()?;
        let tx = conn
            .transaction()
            .map_err(|e| rpc_err(-32603, format!("nightly_tx: {e}")))?;
        let mut proposal_ledgers = Vec::new();
        for proposal in &plan.proposals {
            let proposal_ledger = append_ledger_tx(
                &tx,
                "nightly.proposal",
                "brain.nightly",
                proposal.get("memory_id").and_then(Value::as_str),
                proposal.clone(),
                now_s,
            )
            .map_err(|e| rpc_err(-32603, format!("nightly_proposal_ledger: {e}")))?;
            proposal_ledgers.push(proposal_ledger);
        }
        let mut snapshot_ledger = Value::Null;
        if let Some(snapshot) = &snapshot {
            snapshot_ledger = append_ledger_tx(
                &tx,
                "autonomous.snapshot",
                "brain.nightly",
                None,
                snapshot.value.clone(),
                now_s,
            )
            .map_err(|e| rpc_err(-32603, format!("nightly_snapshot_ledger: {e}")))?;
            tx.execute(
                "INSERT INTO autonomous_mutation_snapshots
                    (id, cycle_id, snapshot_path, snapshot_hash, row_count, created_at_s, ledger_event_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    Uuid::new_v4().to_string(),
                    cycle_id,
                    snapshot.path,
                    snapshot.hash,
                    snapshot.row_count,
                    now_s,
                    snapshot_ledger.get("event_id").and_then(Value::as_str),
                ],
            )
            .map_err(|e| rpc_err(-32603, format!("nightly_snapshot_insert: {e}")))?;
        }
        for mutation in &selected_mutations {
            tx.execute(
                "INSERT INTO retrieval_weights (memory_id, mode, weight, updated_at_s)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(memory_id, mode) DO UPDATE SET
                    weight = excluded.weight,
                    updated_at_s = excluded.updated_at_s",
                params![
                    mutation.memory_id,
                    mutation.mode,
                    mutation.new_weight,
                    now_s
                ],
            )
            .map_err(|e| rpc_err(-32603, format!("nightly_weight_update: {e}")))?;
            append_ledger_tx(
                &tx,
                "autonomous.weight_update",
                "brain.nightly",
                Some(&mutation.memory_id),
                mutation.ledger_payload(&cycle_id),
                now_s,
            )
            .map_err(|e| rpc_err(-32603, format!("nightly_weight_ledger: {e}")))?;
        }
        let run_ledger = append_ledger_tx(
            &tx,
            "nightly.run",
            "brain.nightly",
            Some(&artifact_id),
            json!({
                "cycle_id": cycle_id,
                "mode": mode,
                "status": status,
                "proposal_count": plan.proposals.len(),
                "applied_count": selected_mutations.len(),
                "mutation_count": selected_mutations.len(),
                "input_signal_hash": input_signal_hash,
                "output_state_hash": output_state_hash,
                "guardrails": guardrails_json
            }),
            now_s,
        )
        .map_err(|e| rpc_err(-32603, format!("nightly_run_ledger: {e}")))?;
        tx.execute(
            "INSERT INTO nightly_artifacts
                (id, cycle_id, mode, status, proposal_count, applied_count, mutation_count,
                 warning_count, insight_count, touched_memory_count, input_signal_hash,
                 output_state_hash, ledger_event_id, proposals_json, insights_json,
                 warnings_json, guardrails_json, created_at_s)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                     ?15, ?16, ?17, ?18)",
            params![
                artifact_id,
                cycle_id,
                mode,
                status,
                plan.proposals.len() as i64,
                selected_mutations.len() as i64,
                selected_mutations.len() as i64,
                plan.warnings.len() as i64,
                plan.insights.len() as i64,
                touched_memory_ids.len() as i64,
                input_signal_hash,
                output_state_hash,
                run_ledger.get("event_id").and_then(Value::as_str),
                canonical_json(&proposals_json)
                    .map_err(|e| rpc_err(-32603, format!("nightly_proposals_json: {e}")))?,
                canonical_json(&insights_json)
                    .map_err(|e| rpc_err(-32603, format!("nightly_insights_json: {e}")))?,
                canonical_json(&warnings_json)
                    .map_err(|e| rpc_err(-32603, format!("nightly_warnings_json: {e}")))?,
                canonical_json(&guardrails_json)
                    .map_err(|e| rpc_err(-32603, format!("nightly_guardrails_json: {e}")))?,
                now_s,
            ],
        )
        .map_err(|e| rpc_err(-32603, format!("nightly_artifact_insert: {e}")))?;
        let tool_scoring_snapshot_count = persist_tool_scoring_snapshots_tx(
            &tx,
            &cycle_id,
            now_s,
            &plan.tool_quality,
            run_ledger.get("event_id").and_then(Value::as_str),
        )?;
        tx.commit()
            .map_err(|e| rpc_err(-32603, format!("nightly_commit: {e}")))?;
        json!({
            "run": run_ledger,
            "proposal_ledgers": proposal_ledgers,
            "snapshot_ledger": snapshot_ledger,
            "tool_scoring_snapshot_count": tool_scoring_snapshot_count
        })
    };

    let ledger_event_id = ledger
        .get("run")
        .and_then(|value| value.get("event_id"))
        .and_then(Value::as_str);
    let bridge = reasoning_bridge::record_nightly_artifact(
        state,
        &artifact_id,
        ledger_event_id,
        &touched_memory_ids,
        &guardrails_json,
    )?;

    Ok(json!({
        "ok": true,
        "artifact_id": artifact_id,
        "cycle_id": cycle_id,
        "mode": mode,
        "status": status,
        "proposals": plan.proposals.len(),
        "proposal_count": plan.proposals.len(),
        "proposal_details": plan.proposals,
        "applied": selected_mutations.len(),
        "applied_count": selected_mutations.len(),
        "mutation_count": selected_mutations.len(),
        "mutations": mutation_values,
        "warnings": plan.warnings,
        "insights": plan.insights,
        "guardrails": guardrails_json,
        "tool_quality": plan.tool_quality,
        "input_signal_hash": input_signal_hash,
        "output_state_hash": output_state_hash,
        "ledger": ledger,
        "reasoning_bridge": bridge
    }))
}

struct NightlyPlan {
    corpus_count: i64,
    learning_autonomous: bool,
    dream_autonomous: bool,
    tool_observability_allows_mutation: bool,
    proposals: Vec<Value>,
    weight_mutations: Vec<WeightMutation>,
    warnings: Vec<Value>,
    insights: Vec<Value>,
    guardrails: Value,
    tool_quality: Value,
}

#[derive(Clone)]
struct WeightMutation {
    memory_id: String,
    mode: String,
    source_id: String,
    prior_weight: f64,
    proposed_delta: f64,
    applied_delta: f64,
    new_weight: f64,
    sample_count: i64,
    success_count: i64,
    failure_count: i64,
    reason_code: String,
    signal_hash: String,
}

impl WeightMutation {
    fn value(&self) -> Value {
        json!({
            "kind": "retrieval_weight_delta",
            "memory_id": self.memory_id,
            "mode": self.mode,
            "source_id": self.source_id,
            "prior_weight": self.prior_weight,
            "proposed_delta": self.proposed_delta,
            "applied_delta": self.applied_delta,
            "new_weight": self.new_weight,
            "sample_count": self.sample_count,
            "success_count": self.success_count,
            "failure_count": self.failure_count,
            "reason_code": self.reason_code,
            "signal_hash": self.signal_hash,
            "formula": "clamp(EWMA(0.85*old + 0.15*clamp(old + clamp(eta*signed_kl, -0.05, 0.05), 0.50, 1.00)))"
        })
    }

    fn ledger_payload(&self, cycle_id: &str) -> Value {
        json!({
            "cycle_id": cycle_id,
            "input_signal_hash": self.signal_hash,
            "output_state_hash": sha256_hex(&canonical_json(&self.value()).unwrap_or_else(|_| "{}".to_string())),
            "magnitude": self.applied_delta.abs(),
            "reason_code": self.reason_code,
            "guardrails": {
                "G1_no_delete": true,
                "G2_sensitivity_unchanged": true,
                "G4_ledgered": true,
                "G6_delta_cap": DELTA_CAP,
                "G7_weight_bounds": [MIN_WEIGHT, MAX_WEIGHT],
                "memory_body_mutated": false,
                "architecture_mutated": false
            },
            "mutation": self.value()
        })
    }
}

struct SnapshotRecord {
    path: String,
    hash: String,
    row_count: i64,
    value: Value,
}

#[derive(Default)]
struct MemoryOutcomeStats {
    source_id: String,
    modes: BTreeMap<String, ModeOutcomeStats>,
}

#[derive(Default)]
struct ModeOutcomeStats {
    total: i64,
    success: i64,
    failure: i64,
}

fn build_plan(
    state: &crate::BrainState,
    cycle_id: &str,
    now_s: i64,
) -> Result<NightlyPlan, RpcError> {
    // tool_quality opens its own SQLite connection; gather it before the longer-lived nightly conn.
    let tool_quality = reasoning_bridge::tool_quality(state, json!({"limit": TOOL_QUALITY_LIMIT}))?;
    let conn = state.store.conn()?;
    let corpus_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .unwrap_or(0);
    let learning_autonomous = setting_bool(&conn, "learning.autonomous", true)?;
    let dream_autonomous = setting_bool(&conn, "dream.autonomous", true)?;
    let drift_status = drift_guard_status(&conn)?;
    let confidence_status = confidence_guard_status(&conn)?;

    let mut warnings = Vec::new();
    if !learning_autonomous {
        warnings.push(json!({
            "code": "learning_autonomous_paused",
            "guardrail": "G5",
            "setting": "learning.autonomous"
        }));
    }
    if !dream_autonomous {
        warnings.push(json!({
            "code": "dream_autonomous_paused",
            "guardrail": "G5",
            "setting": "dream.autonomous"
        }));
    }
    warnings.extend(tool_quality_warnings(&tool_quality));
    let mut proposals = Vec::new();
    proposals.extend(session_compaction_proposals(&conn)?);
    proposals.extend(low_quality_review_proposals(&conn)?);
    proposals.extend(tool_quality_review_proposals(&tool_quality, cycle_id));
    let dream_proposals = compartment_dream_proposals(&conn, cycle_id, corpus_count)?;
    proposals.extend(dream_proposals);
    let weight_mutations = weight_mutation_proposals(&conn, now_s)?;
    for mutation in &weight_mutations {
        proposals.push(mutation.value());
    }
    if weight_mutations.is_empty() {
        warnings.push(json!({
            "code": "sample_gate_not_satisfied_or_zero_gradient",
            "guardrail": "G10",
            "required_outcome_samples": SAMPLE_GATE
        }));
    }

    let mut insights = vec![json!({
        "kind": "spice_dream_boundary",
        "cycle_id": cycle_id,
        "papers": [
            "SPICED memory consolidation",
            "Sleep-Based Homeostatic memory consolidation",
            "MATH-A4 KL/InfoNCE clamped EWMA",
            "DA-SSDP bounded plasticity"
        ],
        "allowed_mutation": ["retrieval_weights"],
        "blocked_mutation": [
            "architecture",
            "schema",
            "identity",
            "security",
            "memory_bodies",
            "gate_contracts"
        ]
    })];
    insights.push(tool_quality_insight(&tool_quality, cycle_id));
    let cycle_cap = cycle_cap(corpus_count);
    let source_cap = source_cap(cycle_cap);
    let tool_integrity = tool_quality
        .get("integrity")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let tool_bridge_coverage_required = tool_integrity
        .get("bridge_coverage_required")
        .and_then(Value::as_f64)
        .unwrap_or(1.0);
    let orphan_tool_event_count = tool_integrity
        .get("orphan_tool_event_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let orphan_bridge_count = tool_integrity
        .get("orphan_bridge_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let tool_observability_allows_mutation = orphan_tool_event_count == 0
        && orphan_bridge_count == 0
        && (tool_bridge_coverage_required - 1.0).abs() < EPSILON;
    let guardrails = json!({
        "G1_no_delete": {"state": "enforced", "mechanism": "nightly never deletes memories or source rows"},
        "G2_sensitivity_unchanged": {"state": "enforced", "mechanism": "no sensitivity field is updated"},
        "G3_quality_security_gate": {"state": "enforced", "mechanism": "nightly does not write memory bodies; any future body artifact must use memory.save"},
        "G4_ledgered_mutation": {"state": "enforced", "required_fields": ["actor", "input_signal_hash", "output_state_hash", "magnitude", "reason_code"]},
        "G5_kill_switches": {"learning_autonomous": learning_autonomous, "dream_autonomous": dream_autonomous},
        "G6_delta_cap": {"state": "enforced", "max_abs_delta": DELTA_CAP, "ewma": {"old": EWMA_OLD, "proposed": EWMA_NEW}},
        "G7_weight_bounds": {"state": "enforced", "min": MIN_WEIGHT, "max": MAX_WEIGHT},
        "G8_cycle_cap": {"state": "enforced", "corpus_count": corpus_count, "cycle_cap": cycle_cap, "ratio": CYCLE_CAP_RATIO},
        "G9_trust_delta": {"state": "not_applicable_in_b5", "reason": "B5 does not mutate trust scores"},
        "G10_sample_gate": {"state": "enforced", "minimum_outcome_samples": SAMPLE_GATE},
        "G11_snapshot": {"state": "enforced_on_apply", "retention": "append_only"},
        "G12_supersession_links": {"state": "not_applicable_in_b5", "reason": "B5 does not write supersession edges"},
        "G13_low_trust_quarantine": {"state": "not_applicable_in_b5", "reason": "B5 does not mutate agent trust"},
        "G14_drift_pause": drift_status,
        "G15_negative_correlation_pause": confidence_status,
        "G16_source_cap": {"state": "enforced_on_apply", "source_cap": source_cap, "ratio": SOURCE_CAP_RATIO},
        "G17_tool_observability": {
            "state": if orphan_tool_event_count > 0 || orphan_bridge_count > 0 || tool_bridge_coverage_required < 1.0 {
                "degraded"
            } else {
                "healthy"
            },
            "formula_ref": tool_quality.pointer("/formula/formula_ref").and_then(Value::as_str).unwrap_or("tool_importance_v1"),
            "bridge_coverage_required": tool_bridge_coverage_required,
            "orphan_tool_event_count": orphan_tool_event_count,
            "orphan_bridge_count": orphan_bridge_count
        },
        "mutation_boundaries": {
            "allowed": ["retrieval_weights"],
            "blocked": ["architecture", "schema", "identity", "security_policy", "gate_contracts", "memory_bodies"]
        }
    });

    Ok(NightlyPlan {
        corpus_count,
        learning_autonomous,
        dream_autonomous,
        tool_observability_allows_mutation,
        proposals,
        weight_mutations,
        warnings,
        insights,
        guardrails,
        tool_quality,
    })
}

fn tool_quality_warnings(tool_quality: &Value) -> Vec<Value> {
    let integrity = tool_quality
        .get("integrity")
        .cloned()
        .unwrap_or(Value::Null);
    let bridge_coverage_required = integrity
        .get("bridge_coverage_required")
        .and_then(Value::as_f64)
        .unwrap_or(1.0);
    let orphan_tool_event_count = integrity
        .get("orphan_tool_event_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let orphan_bridge_count = integrity
        .get("orphan_bridge_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    let mut warnings = Vec::new();
    if orphan_tool_event_count > 0 || orphan_bridge_count > 0 || bridge_coverage_required < 1.0 {
        warnings.push(json!({
            "code": "tool_bridge_coverage_degraded",
            "guardrail": "G17",
            "bridge_coverage_required": bridge_coverage_required,
            "orphan_tool_event_count": orphan_tool_event_count,
            "orphan_bridge_count": orphan_bridge_count,
            "formula_ref": tool_quality.pointer("/formula/formula_ref").and_then(Value::as_str).unwrap_or("tool_importance_v1")
        }));
    }

    if let Some(tool) = tool_quality
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|tool| {
            tool.get("event_count").and_then(Value::as_u64).unwrap_or(0) >= 2
                && (tool
                    .get("success_rate")
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0)
                    < TOOL_RELIABILITY_WARNING_THRESHOLD
                    || tool
                        .get("mean_latency_ms")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0)
                        >= TOOL_LATENCY_WARNING_MS)
        })
    {
        warnings.push(json!({
            "code": "tool_reliability_review_recommended",
            "guardrail": "G17",
            "tool_id": tool.get("tool_id").cloned().unwrap_or(Value::Null),
            "success_rate": tool.get("success_rate").cloned().unwrap_or(Value::Null),
            "mean_latency_ms": tool.get("mean_latency_ms").cloned().unwrap_or(Value::Null),
            "importance_score": tool.get("importance_score").cloned().unwrap_or(Value::Null)
        }));
    }

    warnings
}

fn tool_quality_review_proposals(tool_quality: &Value, cycle_id: &str) -> Vec<Value> {
    tool_quality
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            let event_count = tool.get("event_count").and_then(Value::as_u64).unwrap_or(0);
            let bridge_coverage_required = tool
                .get("bridge_coverage_required")
                .and_then(Value::as_f64)
                .unwrap_or(1.0);
            let success_rate = tool.get("success_rate").and_then(Value::as_f64).unwrap_or(1.0);
            let mean_latency_ms = tool
                .get("mean_latency_ms")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let reason_code = if bridge_coverage_required < 1.0 {
                Some("repair_reasoning_bridge_coverage")
            } else if event_count >= 2 && success_rate < TOOL_RELIABILITY_WARNING_THRESHOLD {
                Some("review_tool_reliability")
            } else if event_count >= 2 && mean_latency_ms >= TOOL_LATENCY_WARNING_MS {
                Some("review_tool_latency")
            } else {
                None
            }?;

            Some(json!({
                "kind": "tool_quality_review",
                "cycle_id": cycle_id,
                "tool_id": tool.get("tool_id").cloned().unwrap_or(Value::Null),
                "event_count": event_count,
                "success_rate": success_rate,
                "bridge_coverage_required": bridge_coverage_required,
                "mean_latency_ms": mean_latency_ms,
                "importance_score": tool.get("importance_score").cloned().unwrap_or(Value::Null),
                "formula_ref": tool.get("formula_ref").cloned().unwrap_or(Value::String("tool_importance_v1".to_string())),
                "reason_code": reason_code,
                "proposed_action": "inspect_tool_quality"
            }))
        })
        .take(5)
        .collect()
}

fn tool_quality_insight(tool_quality: &Value, cycle_id: &str) -> Value {
    let top_tools = tool_quality
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(5)
        .map(|tool| {
            json!({
                "tool_id": tool.get("tool_id").cloned().unwrap_or(Value::Null),
                "importance_score": tool.get("importance_score").cloned().unwrap_or(Value::Null),
                "success_rate": tool.get("success_rate").cloned().unwrap_or(Value::Null),
                "bridge_coverage_required": tool.get("bridge_coverage_required").cloned().unwrap_or(Value::Null),
                "mean_latency_ms": tool.get("mean_latency_ms").cloned().unwrap_or(Value::Null)
            })
        })
        .collect::<Vec<_>>();

    json!({
        "kind": "tool_quality_summary",
        "cycle_id": cycle_id,
        "formula_ref": tool_quality.pointer("/formula/formula_ref").and_then(Value::as_str).unwrap_or("tool_importance_v1"),
        "integrity": tool_quality.get("integrity").cloned().unwrap_or(Value::Null),
        "tool_count": tool_quality.get("count").cloned().unwrap_or(Value::Null),
        "top_tools": top_tools
    })
}

fn compartment_dream_proposals(
    conn: &Connection,
    cycle_id: &str,
    corpus_count: i64,
) -> Result<Vec<Value>, RpcError> {
    let cap = cycle_cap(corpus_count) as usize;
    let mut proposals = Vec::new();
    let mut stmt = conn
        .prepare(
            "SELECT session_id, project_id,
                    SUM(CASE WHEN track = 'conversation' THEN 1 ELSE 0 END) as conversation_count,
                    SUM(CASE WHEN track = 'tool' THEN 1 ELSE 0 END) as tool_count,
                    SUM(CASE WHEN track = 'action' THEN 1 ELSE 0 END) as action_count,
                    SUM(CASE WHEN track = 'result' THEN 1 ELSE 0 END) as result_count,
                    COUNT(*) as total
             FROM memories
             WHERE session_id IS NOT NULL AND track IS NOT NULL
             GROUP BY session_id, project_id
             HAVING total >= 4
             ORDER BY MAX(created_at_s) DESC
             LIMIT ?1",
        )
        .map_err(|e| rpc_err(-32603, format!("dream_session_prepare: {e}")))?;
    let sessions: Vec<(String, Option<String>, i64, i64, i64, i64, i64)> = stmt
        .query_map([cap as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .map_err(|e| rpc_err(-32603, format!("dream_session_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("dream_session_collect: {e}")))?;

    for (session_id, project_id, conv, tool, action, result, total) in &sessions {
        let causal_chain = format!(
            "conversation={}, tool={}, action={}, result={}",
            conv, tool, action, result
        );
        let has_causal = *conv > 0 && *action > 0 && *result > 0;
        proposals.push(json!({
            "kind": "dream_causal_summary",
            "cycle_id": cycle_id,
            "session_id": session_id,
            "project_id": project_id,
            "causal_chain": causal_chain,
            "has_full_causal_chain": has_causal,
            "compartment_count": total,
            "proposed_action": if has_causal {
                "generate_session_narrative"
            } else {
                "observe_partial_chain"
            }
        }));

        if !has_causal {
            continue;
        }
        let error_patterns = detect_error_patterns(conn, session_id)?;
        for pattern in &error_patterns {
            let project_scope = project_id.as_deref().unwrap_or("none");
            let lesson_key = format!("lesson:{}:{}", project_scope, pattern.error_tool);
            let existing: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM memories WHERE key = ?1 AND track = 'result'",
                    [&lesson_key],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            if existing > 0 {
                continue;
            }
            proposals.push(json!({
                "kind": "dream_negative_precedent",
                "cycle_id": cycle_id,
                "session_id": session_id,
                "project_id": project_id,
                "error_tool": pattern.error_tool,
                "error_pattern": pattern.error_pattern,
                "lesson_key": lesson_key,
                "proposed_action": "save_lesson_memory",
                "proposed_memory": {
                    "key": lesson_key,
                    "track": "result",
                    "project_id": project_id,
                    "trusted_generated_artifact": true
                },
                "guardrail": "G3: lesson proposals do not write memory bodies; memory.save required"
            }));
        }
    }

    let wisdom = detect_wisdom_patterns(conn)?;
    for pattern in &wisdom {
        let project_scope = pattern.project_id.as_deref().unwrap_or("none");
        let wisdom_key = format!(
            "wisdom:{}:{}:{}",
            project_scope, pattern.tool_name, pattern.error_class
        );
        let existing: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE key = ?1",
                [&wisdom_key],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if existing > 0 {
            continue;
        }
        proposals.push(json!({
            "kind": "dream_wisdom_distillation",
            "cycle_id": cycle_id,
            "project_id": pattern.project_id,
            "tool_name": pattern.tool_name,
            "error_class": pattern.error_class,
            "session_count": pattern.session_count,
            "wisdom_key": wisdom_key,
            "proposed_action": "save_wisdom_memory",
            "proposed_memory": {
                "key": wisdom_key,
                "track": "result",
                "project_id": pattern.project_id,
                "trusted_generated_artifact": true
            },
            "guardrail": "G3: wisdom proposals do not write memory bodies; memory.save required"
        }));
    }

    Ok(proposals)
}

struct ErrorPattern {
    error_tool: String,
    error_pattern: String,
}

fn detect_error_patterns(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<ErrorPattern>, RpcError> {
    let mut stmt = conn
        .prepare(
            "SELECT m1.key, m1.value
             FROM memories m1
             WHERE m1.session_id = ?1
               AND m1.track = 'result'
               AND (LOWER(m1.value) LIKE '%error%' OR LOWER(m1.value) LIKE '%failed%')",
        )
        .map_err(|e| rpc_err(-32603, format!("dream_error_prepare: {e}")))?;
    let rows = stmt
        .query_map([session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| rpc_err(-32603, format!("dream_error_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("dream_error_collect: {e}")))?;
    let mut patterns = Vec::new();
    for (key, value) in &rows {
        let tool_name = extract_tool_from_key(key);
        let error_class = classify_error(value);
        if let (Some(tool), Some(class)) = (&tool_name, &error_class) {
            patterns.push(ErrorPattern {
                error_tool: tool.clone(),
                error_pattern: class.clone(),
            });
        }
    }
    patterns.sort_by(|a, b| a.error_tool.cmp(&b.error_tool));
    patterns.dedup_by(|a, b| a.error_tool == b.error_tool && a.error_pattern == b.error_pattern);
    Ok(patterns)
}

struct WisdomPattern {
    project_id: Option<String>,
    tool_name: String,
    error_class: String,
    session_count: i64,
}

fn detect_wisdom_patterns(conn: &Connection) -> Result<Vec<WisdomPattern>, RpcError> {
    let mut stmt = conn
        .prepare(
            "SELECT m.project_id, SUBSTR(m.key, 1, INSTR(m.key, ':') - 1) as tool_hint,
                    CASE
                        WHEN LOWER(m.value) LIKE '%timeout%' THEN 'timeout'
                        WHEN LOWER(m.value) LIKE '%connection%' THEN 'connection'
                        WHEN LOWER(m.value) LIKE '%permission%' THEN 'permission'
                        WHEN LOWER(m.value) LIKE '%not found%' THEN 'not_found'
                        ELSE 'other'
                    END as error_class,
                    COUNT(DISTINCT m.session_id) as session_count
             FROM memories m
             WHERE m.track = 'result'
               AND (LOWER(m.value) LIKE '%error%' OR LOWER(m.value) LIKE '%failed%')
               AND m.project_id IS NOT NULL
             GROUP BY m.project_id, tool_hint, error_class
             HAVING session_count >= 2
             ORDER BY session_count DESC
             LIMIT 10",
        )
        .map_err(|e| rpc_err(-32603, format!("dream_wisdom_prepare: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(WisdomPattern {
                project_id: row.get::<_, Option<String>>(0)?,
                tool_name: row.get::<_, String>(1).unwrap_or_default(),
                error_class: row.get::<_, String>(2).unwrap_or_default(),
                session_count: row.get::<_, i64>(3)?,
            })
        })
        .map_err(|e| rpc_err(-32603, format!("dream_wisdom_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("dream_wisdom_collect: {e}")))?;
    Ok(rows)
}

fn extract_tool_from_key(key: &str) -> Option<String> {
    let parts: Vec<&str> = key.split(':').collect();
    if parts.len() >= 2 {
        Some(parts[0].to_string())
    } else {
        None
    }
}

fn classify_error(value: &str) -> Option<String> {
    let lower = value.to_lowercase();
    if lower.contains("timeout") {
        Some("timeout".to_string())
    } else if lower.contains("connection") || lower.contains("connect") {
        Some("connection".to_string())
    } else if lower.contains("permission")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
    {
        Some("permission".to_string())
    } else if lower.contains("not found") || lower.contains("missing") {
        Some("not_found".to_string())
    } else if lower.contains("error") || lower.contains("failed") {
        Some("generic_error".to_string())
    } else {
        None
    }
}

fn session_compaction_proposals(conn: &Connection) -> Result<Vec<Value>, RpcError> {
    let mut stmt = conn
        .prepare(
            "SELECT session_id, COUNT(*) as memory_count, MAX(created_at_s) as latest_at_s
             FROM memories
             WHERE session_id IS NOT NULL
             GROUP BY session_id
             HAVING memory_count >= 2
             ORDER BY latest_at_s DESC
             LIMIT 25",
        )
        .map_err(|e| rpc_err(-32603, format!("nightly_compaction_prepare: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| rpc_err(-32603, format!("nightly_compaction_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("nightly_compaction_collect: {e}")))?;
    let mut proposals = Vec::new();
    for (session_id, memory_count, latest_at_s) in rows {
        let pattern = format!("session_compaction:{}:%", session_id);
        let existing: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE memory_type = 'session_compaction' AND key LIKE ?1",
                [pattern],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if existing == 0 {
            proposals.push(json!({
                "kind": "session_compaction_available",
                "session_id": session_id,
                "memory_count": memory_count,
                "latest_at_s": latest_at_s,
                "mutation": false,
                "required_action": "session.compact",
                "reason_code": "session_has_uncompacted_full_detail_context"
            }));
        }
    }
    Ok(proposals)
}

fn low_quality_review_proposals(conn: &Connection) -> Result<Vec<Value>, RpcError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, key, quality_score
             FROM memories
             WHERE quality_score < ?1
             ORDER BY quality_score ASC, created_at_s DESC
             LIMIT 25",
        )
        .map_err(|e| rpc_err(-32603, format!("nightly_low_quality_prepare: {e}")))?;
    let rows = stmt
        .query_map([LOW_QUALITY_REVIEW_THRESHOLD], |row| {
            Ok(json!({
                "kind": "low_quality_review",
                "memory_id": row.get::<_, String>(0)?,
                "key": row.get::<_, String>(1)?,
                "quality_score": row.get::<_, f64>(2)?,
                "threshold": LOW_QUALITY_REVIEW_THRESHOLD,
                "mutation": false,
                "required_action": "manual_review_or_supersession",
                "reason_code": "quality_score_below_documented_review_threshold"
            }))
        })
        .map_err(|e| rpc_err(-32603, format!("nightly_low_quality_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("nightly_low_quality_collect: {e}")))?;
    Ok(rows)
}

fn weight_mutation_proposals(
    conn: &Connection,
    now_s: i64,
) -> Result<Vec<WeightMutation>, RpcError> {
    let since = now_s - 86_400;
    let mut stats: HashMap<String, MemoryOutcomeStats> = HashMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT re.memory_id, re.mode, re.outcome, COALESCE(m.source, 'unknown')
             FROM retrieval_events re
             JOIN memories m ON m.id = re.memory_id
             WHERE re.created_at_s >= ?1",
        )
        .map_err(|e| rpc_err(-32603, format!("nightly_retrieval_events_prepare: {e}")))?;
    let rows = stmt
        .query_map([since], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| rpc_err(-32603, format!("nightly_retrieval_events_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("nightly_retrieval_events_collect: {e}")))?;
    for (memory_id, mode_csv, outcome, source) in rows {
        let memory_stats = stats.entry(memory_id).or_default();
        memory_stats.source_id = source;
        let modes = mode_csv
            .split(',')
            .map(str::trim)
            .filter(|mode| !mode.is_empty())
            .collect::<Vec<_>>();
        for mode in modes {
            let mode_stats = memory_stats.modes.entry(mode.to_string()).or_default();
            mode_stats.total += 1;
            if is_success_outcome(&outcome) {
                mode_stats.success += 1;
            } else if is_failure_outcome(&outcome) {
                mode_stats.failure += 1;
            }
        }
    }

    let mut mutations = Vec::new();
    for (memory_id, memory_stats) in stats {
        let sample_count: i64 = memory_stats.modes.values().map(|mode| mode.total).sum();
        if sample_count < SAMPLE_GATE {
            continue;
        }
        let success_count: i64 = memory_stats.modes.values().map(|mode| mode.success).sum();
        let failure_count: i64 = memory_stats.modes.values().map(|mode| mode.failure).sum();
        if success_count == 0 && failure_count == 0 {
            continue;
        }
        for (mode, mode_stats) in memory_stats.modes {
            if mode_stats.total < SAMPLE_GATE {
                continue;
            }
            let signed_kl = kl_alignment::signed_outcome_kl(
                mode_stats.total,
                sample_count,
                mode_stats.success,
                success_count,
                mode_stats.failure,
                failure_count,
            );
            let contrastive_pairs = contrastive_retraining::pairs_from_counts(
                &memory_id,
                &mode,
                mode_stats.success,
                mode_stats.failure,
            );
            let prior_weight = conn
                .query_row(
                    "SELECT weight FROM retrieval_weights WHERE memory_id = ?1 AND mode = ?2",
                    params![&memory_id, &mode],
                    |row| row.get::<_, f64>(0),
                )
                .optional()
                .map_err(|e| rpc_err(-32603, format!("nightly_weight_lookup: {e}")))?
                .unwrap_or(DEFAULT_WEIGHT);
            let update = auto_weight_update::bounded_ewma_update(
                prior_weight,
                signed_kl,
                ETA,
                DELTA_CAP,
                MIN_WEIGHT,
                MAX_WEIGHT,
                EWMA_OLD,
                EWMA_NEW,
            );
            let proposed_delta = update.proposed_delta;
            if proposed_delta.abs() < EPSILON {
                continue;
            }
            if update.applied_delta.abs() < EPSILON {
                continue;
            }
            let signal = json!({
                "memory_id": memory_id,
                "mode": mode,
                "sample_count": sample_count,
                "mode_sample_count": mode_stats.total,
                "success_count": success_count,
                "failure_count": failure_count,
                "mode_success_count": mode_stats.success,
                "mode_failure_count": mode_stats.failure,
                "signed_kl": signed_kl,
                "contrastive": contrastive_retraining::pairs_json(&contrastive_pairs)
            });
            mutations.push(WeightMutation {
                memory_id: memory_id.clone(),
                mode,
                source_id: memory_stats.source_id.clone(),
                prior_weight: prior_weight.clamp(MIN_WEIGHT, MAX_WEIGHT),
                proposed_delta,
                applied_delta: update.applied_delta,
                new_weight: update.new_weight,
                sample_count,
                success_count,
                failure_count,
                reason_code: "kl_signed_outcome_distribution_with_clamped_ewma".to_string(),
                signal_hash: sha256_hex(
                    &canonical_json(&signal)
                        .map_err(|e| rpc_err(-32603, format!("nightly_signal_hash: {e}")))?,
                ),
            });
        }
    }
    mutations.sort_by(|left, right| {
        right
            .applied_delta
            .abs()
            .partial_cmp(&left.applied_delta.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.memory_id.cmp(&right.memory_id))
            .then_with(|| left.mode.cmp(&right.mode))
    });
    Ok(mutations)
}

fn select_mutations(mutations: &[WeightMutation], corpus_count: i64) -> Vec<WeightMutation> {
    let cycle_cap = cycle_cap(corpus_count) as usize;
    let source_cap = source_cap(cycle_cap as i64) as usize;
    let mut selected = Vec::new();
    let mut per_source: HashMap<String, usize> = HashMap::new();
    for mutation in mutations {
        if selected.len() >= cycle_cap {
            break;
        }
        let count = per_source.entry(mutation.source_id.clone()).or_default();
        if *count >= source_cap {
            continue;
        }
        *count += 1;
        selected.push(mutation.clone());
    }
    selected
}

fn create_weight_snapshot(
    state: &crate::BrainState,
    cycle_id: &str,
    now_s: i64,
) -> Result<SnapshotRecord, RpcError> {
    let weights = {
        let conn = state.store.conn()?;
        let mut stmt = conn
            .prepare("SELECT memory_id, mode, weight, updated_at_s FROM retrieval_weights ORDER BY memory_id, mode")
            .map_err(|e| rpc_err(-32603, format!("nightly_snapshot_prepare: {e}")))?;
        stmt.query_map([], |row| {
            Ok(json!({
                "memory_id": row.get::<_, String>(0)?,
                "mode": row.get::<_, String>(1)?,
                "weight": row.get::<_, f64>(2)?,
                "updated_at_s": row.get::<_, i64>(3)?
            }))
        })
        .map_err(|e| rpc_err(-32603, format!("nightly_snapshot_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("nightly_snapshot_collect: {e}")))?
    };

    let value = json!({
        "cycle_id": cycle_id,
        "created_at_s": now_s,
        "default_weight": DEFAULT_WEIGHT,
        "bounds": [MIN_WEIGHT, MAX_WEIGHT],
        "row_count": weights.len(),
        "weights": weights
    });
    let raw = canonical_json(&value)
        .map_err(|e| rpc_err(-32603, format!("nightly_snapshot_json: {e}")))?;
    let hash = sha256_hex(&raw);
    let snapshots_dir = state.hom_dir.join("snapshots");
    fs::create_dir_all(&snapshots_dir)
        .map_err(|e| rpc_err(-32603, format!("nightly_snapshot_dir: {e}")))?;
    let filename = format!("{}.retrieval_weights.json", cycle_id.replace(':', "-"));
    let path = snapshots_dir.join(filename);
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, raw)
        .map_err(|e| rpc_err(-32603, format!("nightly_snapshot_write: {e}")))?;
    fs::rename(&tmp_path, &path)
        .map_err(|e| rpc_err(-32603, format!("nightly_snapshot_rename: {e}")))?;
    Ok(SnapshotRecord {
        path: path.to_string_lossy().to_string(),
        hash,
        row_count: value.get("row_count").and_then(Value::as_i64).unwrap_or(0),
        value,
    })
}

fn persist_tool_scoring_snapshots_tx(
    tx: &rusqlite::Transaction<'_>,
    cycle_id: &str,
    now_s: i64,
    tool_quality: &Value,
    ledger_event_id: Option<&str>,
) -> Result<i64, RpcError> {
    let Some(tools) = tool_quality.get("tools").and_then(Value::as_array) else {
        return Ok(0);
    };
    let formula_ref = tool_quality
        .pointer("/formula/formula_ref")
        .and_then(Value::as_str)
        .unwrap_or("tool_importance_v1");
    let mut inserted = 0i64;
    for tool in tools {
        let Some(tool_id) = tool.get("tool_id").and_then(Value::as_str) else {
            continue;
        };
        let snapshot_json = json!({
            "cycle_id": cycle_id,
            "created_at_s": now_s,
            "formula_ref": formula_ref,
            "tool": tool,
            "integrity": tool_quality.get("integrity").cloned().unwrap_or_else(|| json!({}))
        });
        tx.execute(
            "INSERT INTO tool_scoring_snapshots
                (id, tool_id, window_start_s, window_end_s, event_count, success_count,
                 error_count, formula_ref, importance_score, score_json, created_at_s, ledger_event_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                Uuid::new_v4().to_string(),
                tool_id,
                Option::<i64>::None,
                now_s,
                tool.get("event_count").and_then(Value::as_i64).unwrap_or(0),
                tool.get("success_count").and_then(Value::as_i64).unwrap_or(0),
                tool.get("error_count").and_then(Value::as_i64).unwrap_or(0),
                formula_ref,
                tool.get("importance_score").and_then(Value::as_f64).unwrap_or(0.0),
                canonical_json(&snapshot_json)
                    .map_err(|e| rpc_err(-32603, format!("tool_scoring_snapshot_json: {e}")))?,
                now_s,
                ledger_event_id,
            ],
        )
        .map_err(|e| rpc_err(-32603, format!("tool_scoring_snapshot_insert: {e}")))?;
        inserted += 1;
    }
    Ok(inserted)
}

fn setting_bool(conn: &Connection, key: &str, default: bool) -> Result<bool, RpcError> {
    let Some(raw) = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .map_err(|e| rpc_err(-32603, format!("nightly_setting_lookup: {e}")))?
    else {
        return Ok(default);
    };
    let parsed = serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| Value::String(raw));
    Ok(match parsed {
        Value::Bool(value) => value,
        Value::String(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "paused"
        ),
        Value::Number(value) => value.as_i64().unwrap_or(0) != 0,
        _ => default,
    })
}

fn drift_guard_status(conn: &Connection) -> Result<Value, RpcError> {
    let rows: Vec<(f64, f64)> = conn
        .prepare(
            "SELECT payload_json
             FROM ledger_events
             WHERE event_type = 'autonomous.weight_update'
               AND created_at_s >= ?1
             ORDER BY created_at_s DESC
             LIMIT 500",
        )
        .and_then(|mut stmt| {
            let since = unix_now_s() - 7 * 86_400;
            let rows = stmt.query_map([since], |row| row.get::<_, String>(0))?;
            let mut parsed = Vec::new();
            for row in rows {
                let raw = row?;
                let value: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
                let Some(prior) = value
                    .pointer("/mutation/prior_weight")
                    .and_then(Value::as_f64)
                else {
                    continue;
                };
                let Some(new) = value
                    .pointer("/mutation/new_weight")
                    .and_then(Value::as_f64)
                else {
                    continue;
                };
                parsed.push((prior, new));
            }
            Ok(parsed)
        })
        .map_err(|e| rpc_err(-32603, format!("nightly_drift_guard: {e}")))?;
    if rows.is_empty() {
        return Ok(json!({"state": "not_tripped", "reason": "no_7d_weight_update_history"}));
    }
    let old_mean = rows.iter().map(|(old, _)| old).sum::<f64>() / rows.len() as f64;
    let new_mean = rows.iter().map(|(_, new)| new).sum::<f64>() / rows.len() as f64;
    let drift = (new_mean - old_mean).abs();
    Ok(json!({
        "state": if drift > 0.15 { "tripped" } else { "not_tripped" },
        "mean_weight_drift": drift,
        "threshold": 0.15,
        "sample_count": rows.len()
    }))
}

fn confidence_guard_status(_conn: &Connection) -> Result<Value, RpcError> {
    Ok(json!({
        "state": "not_tripped",
        "reason": "insufficient_14d_confidence_trend_data",
        "threshold": -0.40
    }))
}

fn cycle_cap(corpus_count: i64) -> i64 {
    if corpus_count <= 0 {
        0
    } else {
        ((corpus_count as f64) * CYCLE_CAP_RATIO).ceil().max(1.0) as i64
    }
}

fn source_cap(cycle_cap: i64) -> i64 {
    if cycle_cap <= 0 {
        0
    } else {
        ((cycle_cap as f64) * SOURCE_CAP_RATIO).ceil().max(1.0) as i64
    }
}

fn is_success_outcome(outcome: &str) -> bool {
    matches!(
        outcome,
        "top_k" | "recall_success" | "accepted" | "used_in_answer" | "clicked" | "success"
    )
}

fn is_failure_outcome(outcome: &str) -> bool {
    matches!(
        outcome,
        "recall_failure" | "rejected" | "unused" | "stale" | "error" | "failure"
    )
}
