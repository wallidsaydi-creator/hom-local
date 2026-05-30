use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};

use crate::services::ranking_service::RRF_K;
use crate::services::recall_temporal::priority_qmd;

#[derive(Clone, Debug, Serialize)]
pub struct AnswerComponents {
    pub evidence_strength: f64,
    pub coverage: f64,
    pub freshness: f64,
    pub mode_trust: f64,
    pub source_authority: f64,
    pub citation_coverage: f64,
    pub atom_precision: f64,
    pub unsupported_penalty: f64,
    pub calibration: f64,
}

pub fn build_answer(query: &str, recall: &Value) -> Value {
    let memories = recall
        .get("memories")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let components = score_components(query, &memories, unix_now_s());
    let confidence = answer_confidence(&components);
    let citations = evidence_cards(&memories);

    json!({
        "ok": true,
        "answer": {
            "type": "extractive_evidence",
            "summary": extractive_summary(&citations),
            "citations": citations
        },
        "confidence": confidence,
        "confidence_components": components,
        "formula": "confidence_v2 = clamp(0,1,0.22*evidence_strength + 0.16*query_coverage + 0.18*citation_coverage + 0.18*atom_precision + 0.10*qmd_freshness + 0.08*mode_trust + 0.05*source_authority + 0.03*calibration) * unsupported_penalty",
        "reference": "HOM-native ALCE/RARR/FActScore confidence envelope pending exact primary anchors; freshness via QMD priority_qmd(t,q)=max(0.50,0.97^(t*(1-0.50*q))); unsupported claims monotonically penalize confidence",
        "mode": "extractive_evidence_only",
        "recall": recall
    })
}

pub fn answer_confidence(components: &AnswerComponents) -> f64 {
    (0.22 * components.evidence_strength
        + 0.16 * components.coverage
        + 0.18 * components.citation_coverage
        + 0.18 * components.atom_precision
        + 0.10 * components.freshness
        + 0.08 * components.mode_trust
        + 0.05 * components.source_authority
        + 0.03 * components.calibration)
        .clamp(0.0, 1.0)
        * components.unsupported_penalty.clamp(0.0, 1.0)
}

pub fn score_components(query: &str, memories: &[Value], now_s: i64) -> AnswerComponents {
    if memories.is_empty() {
        return AnswerComponents {
            evidence_strength: 0.0,
            coverage: 0.0,
            freshness: 0.62,
            mode_trust: 0.0,
            source_authority: 0.0,
            citation_coverage: 0.0,
            atom_precision: 1.0,
            unsupported_penalty: 1.0,
            calibration: 1.0,
        };
    }

    let top_k = memories.iter().take(5).collect::<Vec<_>>();
    let evidence_strength = mean(top_k.iter().map(|memory| normalized_recall_score(memory)));
    let coverage = coverage_score(query, &top_k);
    let freshness = mean(top_k.iter().map(|memory| qmd_freshness(memory, now_s)));
    let mode_trust = mean(top_k.iter().map(|memory| mode_trust_score(memory)));
    let source_authority = mean(top_k.iter().map(|memory| source_authority_score(memory)));
    let citation_coverage = citation_coverage_score(&top_k);
    let atom_precision = mean(top_k.iter().map(|memory| atom_precision_score(memory)));
    let unsupported_penalty = unsupported_penalty_score(&top_k);

    AnswerComponents {
        evidence_strength,
        coverage,
        freshness,
        mode_trust,
        source_authority,
        citation_coverage,
        atom_precision,
        unsupported_penalty,
        calibration: 1.0,
    }
}

fn normalized_recall_score(memory: &Value) -> f64 {
    let raw = memory.get("score").and_then(Value::as_f64).unwrap_or(0.0);
    let mode_count = memory
        .pointer("/components/mode_ranks")
        .and_then(Value::as_array)
        .map(|modes| modes.len().max(1))
        .unwrap_or(1);
    let max_rrf = mode_count as f64 / (RRF_K as f64 + 1.0);
    if max_rrf <= 0.0 {
        return 0.0;
    }
    (raw / max_rrf).clamp(0.0, 1.0)
}

fn coverage_score(query: &str, top_k: &[&Value]) -> f64 {
    let required = query_terms(query);
    if required.is_empty() {
        return 1.0;
    }
    let evidence = top_k
        .iter()
        .map(|memory| {
            format!(
                "{} {}",
                memory.get("key").and_then(Value::as_str).unwrap_or(""),
                memory.get("value").and_then(Value::as_str).unwrap_or("")
            )
            .to_lowercase()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let answered = required
        .iter()
        .filter(|term| evidence.contains(term.as_str()))
        .count();
    answered as f64 / required.len() as f64
}

fn qmd_freshness(memory: &Value, now_s: i64) -> f64 {
    let Some(created_at_s) = memory.get("created_at_s").and_then(Value::as_i64) else {
        return 0.62;
    };
    let quality = memory
        .get("quality_score")
        .and_then(Value::as_f64)
        .unwrap_or(0.5);
    let age_days = ((now_s - created_at_s).max(0) as f64) / 86_400.0;
    priority_qmd(age_days, quality)
}

fn mode_trust_score(memory: &Value) -> f64 {
    memory
        .pointer("/components/mode_ranks")
        .and_then(Value::as_array)
        .map(|modes| {
            modes
                .iter()
                .filter_map(|mode| mode.get("mode").and_then(Value::as_str))
                .map(mode_trust)
                .fold(0.0, f64::max)
        })
        .unwrap_or(0.0)
}

fn mode_trust(mode: &str) -> f64 {
    match mode {
        "exact_artifact" | "identifier" => 1.0,
        "temporal" => 0.86,
        "session" => 0.84,
        "reasoning" => 0.80,
        "lineage" => 0.78,
        "corpus_analytics" => 0.74,
        "text" | "semantic_fallback" => 0.52,
        _ => 0.50,
    }
}

fn source_authority_score(memory: &Value) -> f64 {
    let source = memory
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    if source.contains("inferred") {
        0.6
    } else if source.contains("import") || source.contains("local-pack") {
        0.8
    } else {
        1.0
    }
}

fn citation_coverage_score(top_k: &[&Value]) -> f64 {
    if top_k.is_empty() { 0.0 } else { 1.0 }
}

fn atom_precision_score(memory: &Value) -> f64 {
    memory
        .pointer("/components/atom_precision")
        .or_else(|| memory.get("atom_precision"))
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
        .clamp(0.0, 1.0)
}

fn unsupported_penalty_score(top_k: &[&Value]) -> f64 {
    if top_k.is_empty() {
        return 1.0;
    }
    let unsupported = top_k
        .iter()
        .filter(|memory| {
            memory
                .get("support_status")
                .and_then(Value::as_str)
                .map(|status| status == "unsupported")
                .unwrap_or(false)
        })
        .count();
    (1.0 - unsupported as f64 / top_k.len() as f64).clamp(0.0, 1.0)
}

fn evidence_cards(memories: &[Value]) -> Vec<Value> {
    memories
        .iter()
        .take(5)
        .map(|memory| {
            json!({
                "memory_id": memory.get("memory_id").cloned().unwrap_or(Value::Null),
                "key": memory.get("key").cloned().unwrap_or(Value::Null),
                "excerpt": excerpt(memory.get("value").and_then(Value::as_str).unwrap_or("")),
                "score": memory.get("score").cloned().unwrap_or(Value::Null),
                "source": memory.get("source").cloned().unwrap_or(Value::Null)
            })
        })
        .collect()
}

fn extractive_summary(citations: &[Value]) -> Value {
    if citations.is_empty() {
        return json!("No grounded answer: no recalled evidence matched the query.");
    }
    json!(
        citations
            .iter()
            .filter_map(|card| card.get("excerpt").and_then(Value::as_str))
            .take(3)
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn excerpt(value: &str) -> String {
    let trimmed = value.trim();
    let sentence_end = trimmed
        .char_indices()
        .find_map(|(index, ch)| matches!(ch, '.' | '!' | '?').then_some(index + ch.len_utf8()));
    let end = sentence_end.unwrap_or_else(|| trimmed.len()).min(280);
    trimmed[..end].trim().to_string()
}

fn query_terms(query: &str) -> Vec<String> {
    let stop: HashSet<&str> = [
        "the", "and", "for", "but", "not", "all", "are", "was", "has", "had", "this", "that",
        "with", "from", "into", "onto", "what", "when", "where", "which", "show", "find", "answer",
        "explain",
    ]
    .into_iter()
    .collect();
    let mut terms: Vec<_> = query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|term| {
            let term = term.trim().to_lowercase();
            (term.len() >= 3 && !stop.contains(term.as_str())).then_some(term)
        })
        .collect();
    terms.sort();
    terms.dedup();
    terms
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let mut total = 0.0;
    let mut count = 0usize;
    for value in values {
        total += value;
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

fn unix_now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_formula_matches_weights() {
        let components = AnswerComponents {
            evidence_strength: 1.0,
            coverage: 1.0,
            freshness: 1.0,
            mode_trust: 1.0,
            source_authority: 1.0,
            citation_coverage: 1.0,
            atom_precision: 1.0,
            unsupported_penalty: 1.0,
            calibration: 1.0,
        };
        assert!((answer_confidence(&components) - 1.0).abs() < 0.0001);
    }

    #[test]
    fn unsupported_penalty_monotonically_lowers_confidence() {
        let supported = AnswerComponents {
            evidence_strength: 1.0,
            coverage: 1.0,
            freshness: 1.0,
            mode_trust: 1.0,
            source_authority: 1.0,
            citation_coverage: 1.0,
            atom_precision: 1.0,
            unsupported_penalty: 1.0,
            calibration: 1.0,
        };
        let penalized = AnswerComponents {
            unsupported_penalty: 0.5,
            ..supported.clone()
        };
        assert!(answer_confidence(&penalized) < answer_confidence(&supported));
    }

    #[test]
    fn build_answer_formula_surfaces_citation_and_atom_components() {
        let recall = json!({"memories": [], "confidence": 0.0});
        let answer = build_answer("missing", &recall);
        let formula = answer["formula"].as_str().unwrap();
        assert!(formula.contains("citation_coverage"));
        assert!(formula.contains("atom_precision"));
        assert!(formula.contains("unsupported_penalty"));
    }

    #[test]
    fn empty_recall_returns_zero_evidence_strength() {
        let components = score_components("missing query", &[], unix_now_s());
        assert_eq!(components.evidence_strength, 0.0);
        assert_eq!(components.coverage, 0.0);
        assert_eq!(components.calibration, 1.0);
    }

    #[test]
    fn build_answer_returns_cited_claim_payload() {
        let recall = json!({
            "memories": [{
                "memory_id": "m1",
                "key": "k1",
                "value": "The system was implemented to support evidence-based reasoning.",
                "source": "hom-local",
                "created_at_s": unix_now_s(),
                "score": 1.0 / 61.0,
                "components": {
                    "mode_ranks": [{"mode": "exact_artifact", "rank": 1}]
                }
            }]
        });
        let answer = build_answer("Apollo Router", &recall);
        assert!(answer["confidence"].as_f64().unwrap() > 0.0);
        assert_eq!(answer["answer"]["citations"][0]["memory_id"], "m1");
        assert_ne!(answer["answer"], Value::Null);
    }
}
