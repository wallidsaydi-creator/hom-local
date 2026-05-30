use std::collections::{BTreeMap, HashSet};

use rusqlite::Connection;
use serde_json::{Value, json};

use crate::services::ranking_service::RRF_K;

#[derive(Clone, Debug, serde::Serialize)]
pub struct RecallAudit {
    pub audit_kind: String,
    pub decision: String,
    pub drift_score: f64,
    pub component_scores: Value,
    pub formula_ref: String,
    pub mutation_permitted: bool,
    pub user_facing_mode: bool,
    pub guidance: Vec<String>,
    pub context_injection: Value,
    pub allowed_actions: Vec<String>,
    pub forbidden_actions: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RecallDiagnostics {
    pub mode_distribution: Vec<(String, usize)>,
    pub weighted_mode_distribution: Vec<(String, f64)>,
    pub avg_confidence: f64,
    pub fallback_rate: f64,
    pub exact_pin_rate: f64,
    pub top_1_coverage: f64,
    pub graph_walk_contribution: f64,
    pub duplicate_mode_count: usize,
}

pub fn compute_recall_audit(
    reference: &Value,
    current: &Value,
    signals: Option<Value>,
) -> RecallAudit {
    let reference_ids = memory_ids(reference);
    let current_ids = memory_ids(current);
    let union = reference_ids
        .iter()
        .chain(current_ids.iter())
        .collect::<HashSet<_>>()
        .len();
    let intersection = current_ids
        .iter()
        .filter(|id| reference_ids.contains(id))
        .count();
    let jaccard = if union == 0 {
        1.0
    } else {
        intersection as f64 / union as f64
    };
    let rank_drift = (1.0 - jaccard).clamp(0.0, 1.0);
    let rank_displacement = normalized_rank_displacement(&reference_ids, &current_ids);
    let weighted_modes = weighted_mode_distribution(
        &current
            .get("memories")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        &[],
    )
    .0;
    let mode_dominance = weighted_modes
        .iter()
        .map(|(_, weight)| *weight)
        .fold(0.0_f64, f64::max);
    let mode_dominance_excess = (mode_dominance - 0.75).max(0.0);
    let signed_kl = signals
        .as_ref()
        .and_then(|value| value.get("signed_kl"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let kl_drift = (-signed_kl).max(0.0).min(1.0);
    let evidence_precision = average_evidence_precision(current);
    let evidence_drift = (1.0 - evidence_precision).clamp(0.0, 1.0);
    let calibration_boost = signals
        .as_ref()
        .and_then(|value| value.get("avg_calibration_boost"))
        .and_then(Value::as_f64)
        .unwrap_or(1.0);
    let calibration_drift = (1.0 - calibration_boost).max(0.0).min(1.0);
    let drift_score = (0.25 * rank_drift
        + 0.10 * rank_displacement
        + 0.15 * mode_dominance_excess
        + 0.20 * kl_drift
        + 0.20 * evidence_drift
        + 0.10 * calibration_drift)
        .clamp(0.0, 1.0);
    let current_has_hybrid = current
        .get("memories")
        .and_then(Value::as_array)
        .map(|memories| {
            memories.iter().any(|memory| {
                memory
                    .pointer("/components/mode_ranks")
                    .and_then(Value::as_array)
                    .map(|ranks| {
                        ranks.iter().any(|rank| {
                            rank.get("mode").and_then(Value::as_str)
                                == Some("vector_hybrid_exact_rerank")
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    let decision = if drift_score > 0.55 {
        "plan_drift"
    } else if drift_score >= 0.30 {
        "watch"
    } else {
        "aligned"
    };
    let mut guidance = Vec::new();
    if drift_score >= 0.30 {
        guidance.push(
            "compare current recall/action against the validated plan before proceeding"
                .to_string(),
        );
        guidance.push(
            "inject compact plan context if session compaction or long-horizon drift is detected"
                .to_string(),
        );
    }
    if evidence_precision < 0.70 {
        guidance.push("request or retrieve stronger evidence before strong synthesis".to_string());
    }
    if current_has_hybrid {
        guidance.push(
            "hybrid vector recall remains available; audit its contribution instead of blocking it"
                .to_string(),
        );
    }

    RecallAudit {
        audit_kind: "recall_action_audit_v1".to_string(),
        decision: decision.to_string(),
        drift_score,
        component_scores: json!({
            "jaccard_at_k": jaccard,
            "rank_drift": rank_drift,
            "normalized_rank_displacement": rank_displacement,
            "mode_dominance": mode_dominance,
            "mode_dominance_excess": mode_dominance_excess,
            "signed_kl": signed_kl,
            "kl_drift": kl_drift,
            "evidence_precision": evidence_precision,
            "evidence_drift": evidence_drift,
            "avg_calibration_boost": calibration_boost,
            "calibration_drift": calibration_drift
        }),
        formula_ref: "recall_action_audit_score_v1".to_string(),
        mutation_permitted: false,
        user_facing_mode: false,
        guidance,
        context_injection: json!({
            "recommended": drift_score >= 0.30,
            "reason": if drift_score >= 0.30 { "plan_or_recall_drift_risk" } else { "audit_aligned" },
            "payload_kind": "compact_validated_plan_context",
            "user_facing": false
        }),
        allowed_actions: vec![
            "continue_with_audit_metadata".to_string(),
            "warn_operator".to_string(),
            "inject_compact_plan_context".to_string(),
            "request_or_retrieve_more_evidence".to_string(),
            "record_audit_event".to_string(),
        ],
        forbidden_actions: vec![
            "delete_memory".to_string(),
            "mutate_canonical_memory".to_string(),
            "globally_disable_mode".to_string(),
            "prompt_user_for_recall_mode".to_string(),
        ],
    }
}

pub fn recall_audit_json(reference: &Value, current: &Value, signals: Option<Value>) -> Value {
    json!({
        "ok": true,
        "audit": compute_recall_audit(reference, current, signals),
    })
}

fn memory_ids(recall_result: &Value) -> Vec<String> {
    recall_result
        .get("memories")
        .and_then(Value::as_array)
        .map(|memories| {
            memories
                .iter()
                .filter_map(|memory| memory.get("memory_id").and_then(Value::as_str))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn normalized_rank_displacement(reference_ids: &[String], current_ids: &[String]) -> f64 {
    if reference_ids.is_empty() || current_ids.is_empty() {
        return 0.0;
    }
    let mut reference_ranks = BTreeMap::new();
    for (idx, id) in reference_ids.iter().enumerate() {
        reference_ranks.insert(id, idx as f64);
    }
    let mut total = 0.0;
    let mut count = 0.0;
    let denom = reference_ids.len().max(current_ids.len()).max(1) as f64;
    for (idx, id) in current_ids.iter().enumerate() {
        if let Some(reference_rank) = reference_ranks.get(id) {
            total += (*reference_rank - idx as f64).abs() / denom;
            count += 1.0;
        }
    }
    if count == 0.0 { 1.0 } else { total / count }
}

fn average_evidence_precision(recall_result: &Value) -> f64 {
    let Some(memories) = recall_result.get("memories").and_then(Value::as_array) else {
        return 1.0;
    };
    if memories.is_empty() {
        return 1.0;
    }
    let mut total = 0.0;
    for memory in memories {
        total += memory
            .pointer("/components/evidence_precision")
            .or_else(|| memory.get("evidence_precision"))
            .and_then(Value::as_f64)
            .unwrap_or(1.0);
    }
    (total / memories.len() as f64).clamp(0.0, 1.0)
}

pub fn compute_recall_diagnostics(
    recall_result: &Value,
    total_candidates: usize,
) -> RecallDiagnostics {
    let modes = recall_result
        .get("recall_meta")
        .and_then(|m| m.get("modes"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut mode_counts: BTreeMap<String, usize> = BTreeMap::new();
    for mode in modes.iter().filter_map(Value::as_str) {
        *mode_counts.entry(mode.to_string()).or_default() += 1;
    }
    let mode_distribution: Vec<(String, usize)> = mode_counts.into_iter().collect();

    let memories = recall_result
        .get("memories")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let (weighted_mode_distribution, graph_walk_contribution) =
        weighted_mode_distribution(&memories, &modes);
    let duplicate_mode_count = duplicate_memory_count(&memories);

    let confidence = recall_result
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);

    let result_count = memories.len();
    let fallback_rate = if total_candidates > 0 {
        1.0 - (result_count as f64 / total_candidates as f64).min(1.0)
    } else if result_count == 0 {
        1.0
    } else {
        0.0
    };

    let exact_pins = memories
        .iter()
        .filter(|m| {
            m.get("components")
                .and_then(|c| c.get("exact_pin"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count();
    let exact_pin_rate = if result_count > 0 {
        exact_pins as f64 / result_count as f64
    } else {
        0.0
    };

    let top_1_coverage = if result_count > 0 {
        let top = &memories[0];
        let key = top.get("key").and_then(Value::as_str).unwrap_or("");
        let value = top.get("value").and_then(Value::as_str).unwrap_or("");
        let text = format!("{key} {value}").to_lowercase();
        let query_terms = recall_result
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("");
        let terms: Vec<&str> = query_terms
            .split_whitespace()
            .filter(|t| t.len() >= 3)
            .collect();
        if terms.is_empty() {
            1.0
        } else {
            let matched = terms
                .iter()
                .filter(|t| text.contains(&t.to_lowercase()))
                .count();
            matched as f64 / terms.len() as f64
        }
    } else {
        0.0
    };

    RecallDiagnostics {
        mode_distribution,
        weighted_mode_distribution,
        avg_confidence: confidence,
        fallback_rate,
        exact_pin_rate,
        top_1_coverage,
        graph_walk_contribution,
        duplicate_mode_count,
    }
}

fn weighted_mode_distribution(
    memories: &[Value],
    fallback_modes: &[Value],
) -> (Vec<(String, f64)>, f64) {
    let mut weights: BTreeMap<String, f64> = BTreeMap::new();

    for memory in memories {
        if let Some(mode_ranks) = memory
            .pointer("/components/mode_ranks")
            .and_then(Value::as_array)
        {
            for mode_rank in mode_ranks {
                let Some(mode) = mode_rank.get("mode").and_then(Value::as_str) else {
                    continue;
                };
                let rank = mode_rank
                    .get("rank")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .max(1) as f64;
                *weights.entry(mode.to_string()).or_default() += 1.0 / (RRF_K as f64 + rank);
            }
        }
    }

    if weights.is_empty() {
        for mode in fallback_modes.iter().filter_map(Value::as_str) {
            *weights.entry(mode.to_string()).or_default() += 1.0;
        }
    }

    let total: f64 = weights.values().sum();
    if total <= 0.0 {
        return (Vec::new(), 0.0);
    }

    let graph_walk_contribution = weights.get("graph_walk").copied().unwrap_or(0.0) / total;
    let distribution = weights
        .into_iter()
        .map(|(mode, weight)| (mode, weight / total))
        .collect();
    (distribution, graph_walk_contribution)
}

fn duplicate_memory_count(memories: &[Value]) -> usize {
    let mut seen = HashSet::new();
    let mut duplicates = 0;
    for memory_id in memories
        .iter()
        .filter_map(|memory| memory.get("memory_id").and_then(Value::as_str))
    {
        if !seen.insert(memory_id) {
            duplicates += 1;
        }
    }
    duplicates
}

pub fn trust_diagnostics_json(conn: &Connection) -> Value {
    let scores = crate::services::trust_score::compute_trust_scores(conn, 50);
    json!({
        "ok": true,
        "trust_scores": scores,
        "total_sources": scores.len()
    })
}

pub fn recall_diagnostics_json(recall_result: &Value, total_candidates: usize) -> Value {
    let diagnostics = compute_recall_diagnostics(recall_result, total_candidates);
    json!({
        "ok": true,
        "diagnostics": diagnostics
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recall_diagnostics_computes_fallback_rate() {
        let recall = json!({
            "confidence": 0.85,
            "recall_meta": { "modes": ["text", "temporal"] },
            "memories": [{
                "memory_id": "m1",
                "key": "test",
                "value": "test value",
                "components": { "exact_pin": false }
            }]
        });
        let diag = compute_recall_diagnostics(&recall, 100);
        assert!((diag.fallback_rate - 0.99).abs() < 0.01);
        assert!((diag.exact_pin_rate).abs() < 0.01);
        assert!((diag.avg_confidence - 0.85).abs() < 0.01);
    }

    #[test]
    fn recall_diagnostics_with_exact_pin() {
        let recall = json!({
            "confidence": 0.92,
            "recall_meta": { "modes": ["exact_artifact"] },
            "memories": [{
                "memory_id": "m1",
                "key": "session:test",
                "value": "test",
                "components": { "exact_pin": true }
            }]
        });
        let diag = compute_recall_diagnostics(&recall, 10);
        assert!((diag.exact_pin_rate - 1.0).abs() < 0.01);
    }

    #[test]
    fn recall_diagnostics_empty_results() {
        let recall = json!({
            "confidence": 0.0,
            "recall_meta": { "modes": [] },
            "memories": []
        });
        let diag = compute_recall_diagnostics(&recall, 0);
        assert_eq!(diag.fallback_rate, 1.0);
        assert_eq!(diag.exact_pin_rate, 0.0);
    }

    #[test]
    fn recall_diagnostics_aggregates_duplicate_modes() {
        let recall = json!({
            "confidence": 0.7,
            "recall_meta": { "modes": ["text", "temporal", "text"] },
            "memories": []
        });
        let diag = compute_recall_diagnostics(&recall, 0);
        assert_eq!(
            diag.mode_distribution,
            vec![("temporal".to_string(), 1), ("text".to_string(), 2)]
        );
    }

    #[test]
    fn recall_audit_guard_passes_stable_overlap_and_supported_evidence() {
        let reference = json!({
            "memories": [
                {"memory_id": "a", "components": {"mode_ranks": [{"mode": "text", "rank": 1}], "evidence_precision": 1.0}},
                {"memory_id": "b", "components": {"mode_ranks": [{"mode": "vector_exact", "rank": 2}], "evidence_precision": 0.9}}
            ]
        });
        let current = json!({
            "memories": [
                {"memory_id": "a", "components": {"mode_ranks": [{"mode": "text", "rank": 1}], "evidence_precision": 1.0}},
                {"memory_id": "b", "components": {"mode_ranks": [{"mode": "vector_hybrid_exact_rerank", "rank": 2}], "evidence_precision": 0.9}}
            ]
        });
        let audit = compute_recall_audit(&reference, &current, None);
        assert_eq!(audit.decision, "aligned");
        assert!(audit.drift_score < 0.30);
        assert_eq!(audit.mutation_permitted, false);
        assert_eq!(audit.audit_kind, "recall_action_audit_v1");
        assert_eq!(audit.context_injection["recommended"], false);
    }

    #[test]
    fn recall_audit_guides_plan_drift_without_blocking_hybrid() {
        let reference = json!({
            "memories": [
                {"memory_id": "stable-a", "components": {"mode_ranks": [{"mode": "text", "rank": 1}], "evidence_precision": 1.0}},
                {"memory_id": "stable-b", "components": {"mode_ranks": [{"mode": "temporal", "rank": 2}], "evidence_precision": 1.0}}
            ]
        });
        let current = json!({
            "memories": [
                {"memory_id": "drift-x", "components": {"mode_ranks": [{"mode": "vector_hybrid_exact_rerank", "rank": 1}], "evidence_precision": 0.0}},
                {"memory_id": "drift-y", "components": {"mode_ranks": [{"mode": "vector_hybrid_exact_rerank", "rank": 2}], "evidence_precision": 0.0}}
            ]
        });
        let audit = compute_recall_audit(&reference, &current, Some(json!({"signed_kl": -0.7})));
        assert_eq!(audit.decision, "plan_drift");
        assert!(audit.drift_score > 0.55);
        assert!(audit.component_scores["rank_drift"].as_f64().unwrap() > 0.9);
        assert!(
            audit.component_scores["mode_dominance_excess"]
                .as_f64()
                .unwrap()
                > 0.0
        );
        assert_eq!(audit.mutation_permitted, false);
        assert_eq!(audit.context_injection["recommended"], true);
        assert!(
            audit
                .allowed_actions
                .contains(&"inject_compact_plan_context".to_string())
        );
        assert!(
            !audit
                .allowed_actions
                .contains(&"block_hybrid_for_query".to_string())
        );
    }

    #[test]
    fn recall_diagnostics_reports_weighted_graph_walk_contribution() {
        let recall = json!({
            "confidence": 0.81,
            "query": "graph memory",
            "memories": [{
                "memory_id": "m1",
                "key": "graph memory",
                "value": "graph memory supports weighted diagnostics",
                "components": {
                    "mode_ranks": [
                        { "mode": "graph_walk", "rank": 1 },
                        { "mode": "text", "rank": 4 }
                    ]
                }
            }, {
                "memory_id": "m1",
                "key": "graph memory duplicate",
                "value": "same memory from another mode",
                "components": {
                    "mode_ranks": [
                        { "mode": "text", "rank": 2 }
                    ]
                }
            }]
        });

        let diag = compute_recall_diagnostics(&recall, 10);
        assert!(diag.graph_walk_contribution > 0.0);
        assert_eq!(diag.duplicate_mode_count, 1);
        let total_weight: f64 = diag
            .weighted_mode_distribution
            .iter()
            .map(|(_, weight)| *weight)
            .sum();
        assert!((total_weight - 1.0).abs() < 0.000001);
    }
}
