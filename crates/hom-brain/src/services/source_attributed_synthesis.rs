use serde_json::{Value, json};

use super::evidence_atom::Atom;
use super::security_policy;

const MAX_CLAIM_CARDS: usize = 5;

#[derive(Clone, Debug, serde::Serialize)]
pub struct ClaimCard {
    pub claim: String,
    pub source_memory_id: String,
    pub source_key: String,
    pub source_excerpt: String,
    pub atom_ids: Vec<String>,
    pub confidence: f64,
    pub support_status: String,
    pub support_score: f64,
    pub unsupported_reason: String,
}

pub fn synthesize(
    query: &str,
    recall: &Value,
    atoms_by_memory: &std::collections::HashMap<String, Vec<Atom>>,
) -> Vec<ClaimCard> {
    let memories: Vec<Value> = recall
        .get("memories")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|memory| {
            let key = memory.get("key").and_then(Value::as_str).unwrap_or("");
            let value = memory.get("value").and_then(Value::as_str).unwrap_or("");
            security_policy::evidence_is_safe(key, value)
        })
        .collect();

    if memories.is_empty() {
        return vec![ClaimCard {
            claim: "No grounded answer: no recalled evidence matched the query.".into(),
            source_memory_id: String::new(),
            source_key: String::new(),
            source_excerpt: String::new(),
            atom_ids: Vec::new(),
            confidence: 0.0,
            support_status: "abstained".into(),
            support_score: 0.0,
            unsupported_reason: "no_safe_recalled_evidence".into(),
        }];
    }

    let query_terms = tokenize_query(query);
    let mut cards: Vec<ClaimCard> = Vec::new();

    for memory in memories.iter().take(5) {
        if cards.len() >= MAX_CLAIM_CARDS {
            break;
        }
        let memory_id = memory
            .get("memory_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        let key = memory.get("key").and_then(Value::as_str).unwrap_or("");
        let value = memory.get("value").and_then(Value::as_str).unwrap_or("");
        let score = memory.get("score").and_then(Value::as_f64).unwrap_or(0.0);
        let excerpt = excerpt(value);

        let memory_atoms = atoms_by_memory.get(memory_id).cloned().unwrap_or_default();
        let matching_atoms: Vec<&Atom> = memory_atoms
            .iter()
            .filter(|atom| {
                let atom_text =
                    format!("{} {} {}", atom.subject, atom.predicate, atom.object).to_lowercase();
                query_terms
                    .iter()
                    .any(|term| atom_text.contains(term.as_str()))
            })
            .collect();

        if matching_atoms.is_empty() && !query_terms.is_empty() {
            let claim_text = if excerpt.is_empty() {
                format!("{key}: {value}").chars().take(280).collect()
            } else {
                excerpt.clone()
            };
            cards.push(ClaimCard {
                claim: claim_text,
                source_memory_id: memory_id.to_string(),
                source_key: key.to_string(),
                source_excerpt: excerpt.clone(),
                atom_ids: Vec::new(),
                confidence: score,
                support_status: "supported".into(),
                support_score: extractive_support_score(&query_terms, &excerpt),
                unsupported_reason: String::new(),
            });
            continue;
        }

        for atom in matching_atoms {
            if cards.len() >= MAX_CLAIM_CARDS {
                break;
            }
            let claim_text = format!("{} {} {}", atom.subject, atom.predicate, atom.object);
            cards.push(ClaimCard {
                claim: claim_text,
                source_memory_id: memory_id.to_string(),
                source_key: key.to_string(),
                source_excerpt: excerpt.clone(),
                atom_ids: vec![format!("atom:{}:{}", memory_id, atom.subject.len())],
                confidence: (score * atom.confidence).clamp(0.0, 1.0),
                support_status: "supported".into(),
                support_score: 1.0,
                unsupported_reason: String::new(),
            });
        }
    }

    cards.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    cards.dedup_by(|a, b| a.claim == b.claim);
    cards.truncate(MAX_CLAIM_CARDS);
    cards
}

pub fn build_answer(
    query: &str,
    recall: &Value,
    atoms_by_memory: &std::collections::HashMap<String, Vec<Atom>>,
) -> Value {
    let cards = synthesize(query, recall, atoms_by_memory);
    let confidence = recall
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let components = recall.get("confidence_components").cloned();
    let packed_recall = recall.get("packed_recall").cloned().unwrap_or(Value::Null);
    let packed_confidence = packed_recall
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(confidence);
    let packed_open_handles = packed_recall
        .get("open_handles")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let packed_frames = packed_recall
        .get("frames")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let evidence_budget = packed_frames
        .as_array()
        .and_then(|frames| frames.first())
        .and_then(|frame| frame.get("diagnostics"))
        .and_then(|diagnostics| diagnostics.get("evidence_budget"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let packed_phase = packed_recall
        .get("diagnostics")
        .and_then(|diagnostics| diagnostics.get("phase"))
        .cloned()
        .unwrap_or(Value::Null);
    let recall_audit = recall
        .get("recall_meta")
        .and_then(|meta| meta.get("recall_audit"))
        .cloned()
        .unwrap_or(Value::Null);

    json!({
        "ok": true,
        "answer": {
            "type": "source_attributed_evidence",
            "claims": cards.iter().map(|card| json!({
                "claim": card.claim,
                "source_memory_id": card.source_memory_id,
                "source_key": card.source_key,
                "source_excerpt": card.source_excerpt,
                "atom_ids": card.atom_ids,
                "confidence": card.confidence,
                "support_status": card.support_status,
                "support_score": card.support_score,
                "unsupported_reason": card.unsupported_reason
            })).collect::<Vec<_>>(),
            "claim_count": cards.len(),
            "contract": {
                "mode": "source_attributed_evidence",
                "packed_alignment": "packed_recall_v1",
                "query": query,
                "phase": packed_phase,
                "recall_audit": {
                    "attached": !recall_audit.is_null(),
                    "user_facing": false,
                    "decision": recall_audit.get("decision").cloned().unwrap_or(Value::Null),
                    "context_injection": recall_audit.get("context_injection").cloned().unwrap_or(Value::Null)
                }
            },
            "support": {
                "open_handles": packed_open_handles,
                "evidence_budget": evidence_budget,
                "frames": packed_frames,
                "packed_confidence": packed_confidence
            }
        },
        "confidence": confidence,
        "confidence_components": components,
        "mode": "source_attributed_evidence",
        "recall": recall
    })
}

fn excerpt(value: &str) -> String {
    let trimmed = value.trim();
    let sentence_end = trimmed
        .char_indices()
        .find_map(|(index, ch)| matches!(ch, '.' | '!' | '?').then_some(index + ch.len_utf8()));
    let end = sentence_end.unwrap_or_else(|| trimmed.len()).min(280);
    trimmed[..end].trim().to_string()
}

fn extractive_support_score(query_terms: &[String], excerpt: &str) -> f64 {
    if query_terms.is_empty() {
        return 1.0;
    }
    let excerpt = excerpt.to_lowercase();
    let matched = query_terms
        .iter()
        .filter(|term| excerpt.contains(term.as_str()))
        .count();
    (matched as f64 / query_terms.len() as f64).clamp(0.0, 1.0)
}

fn tokenize_query(query: &str) -> Vec<String> {
    let stop: std::collections::HashSet<&str> = [
        "the", "and", "for", "but", "not", "all", "are", "was", "has", "had", "this", "that",
        "with", "from", "into", "onto", "what", "when", "where", "which", "show", "find", "answer",
        "explain", "how", "why", "does", "did", "can", "could", "would", "should",
    ]
    .into_iter()
    .collect();
    query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|term| {
            let term = term.trim().to_lowercase();
            (term.len() >= 3 && !stop.contains(term.as_str())).then_some(term)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesize_with_matching_atoms_produces_claim_cards() {
        let recall = json!({
            "memories": [{
                "memory_id": "m1",
                "key": "session:test",
                "value": "The system was implemented to support evidence-based reasoning.",
                "score": 0.85,
                "confidence": 0.85
            }]
        });
        let atoms = vec![Atom {
            subject: "Apollo Router".into(),
            predicate: "was implemented".into(),
            object: "to support evidence-based reasoning".into(),
            confidence: 0.9,
            atom_type: "declarative".into(),
        }];
        let mut atoms_by_memory = std::collections::HashMap::new();
        atoms_by_memory.insert("m1".into(), atoms);

        let cards = synthesize("Apollo Router evidence", &recall, &atoms_by_memory);
        assert!(!cards.is_empty());
        assert_eq!(cards[0].source_memory_id, "m1");
        assert!(!cards[0].atom_ids.is_empty());
    }

    #[test]
    fn synthesize_with_no_evidence_returns_grounded_empty_claim() {
        let recall = json!({"memories": [], "confidence": 0.0});
        let cards = synthesize("missing query", &recall, &std::collections::HashMap::new());
        assert_eq!(cards.len(), 1);
        assert!(cards[0].claim.contains("No grounded answer"));
        assert!(cards[0].source_memory_id.is_empty());
    }

    #[test]
    fn synthesize_caps_at_max_claim_cards() {
        let memories: Vec<Value> = (0..10)
            .map(|i| {
                json!({
                    "memory_id": format!("m{i}"),
                    "key": format!("key{i}"),
                    "value": format!("Memory {i} was created because system required it for testing purposes."),
                    "score": 0.8
                })
            })
            .collect();
        let recall = json!({"memories": memories, "confidence": 0.8});
        let cards = synthesize("memory system", &recall, &std::collections::HashMap::new());
        assert!(cards.len() <= MAX_CLAIM_CARDS);
    }

    #[test]
    fn build_answer_surfaces_unified_packed_contract() {
        let recall = json!({
            "memories": [{
                "memory_id": "m1",
                "key": "session:test",
                "value": "The system was implemented to support evidence-based reasoning.",
                "score": 0.85,
                "source": "test"
            }],
            "confidence": 0.82,
            "packed_recall": {
                "query": "why router",
                "precision_level": "claim_evidence",
                "confidence": 0.81,
                "frames": [{
                    "mode": "claim_evidence",
                    "diagnostics": {
                        "evidence_budget": {
                            "estimated_chars": 144,
                            "pressure_band": "medium"
                        }
                    }
                }],
                "open_handles": [{
                    "method": "memory.open",
                    "memory_id": "m1"
                }],
                "diagnostics": {
                    "phase": "phase_5_grounded_pressure"
                }
            }
        });

        let answer = build_answer("why router", &recall, &std::collections::HashMap::new());
        assert_eq!(answer["answer"]["type"], "source_attributed_evidence");
        assert_eq!(
            answer["answer"]["contract"]["mode"],
            "source_attributed_evidence"
        );
        assert_eq!(
            answer["answer"]["contract"]["packed_alignment"],
            "packed_recall_v1"
        );
        assert_eq!(
            answer["answer"]["support"]["open_handles"][0]["memory_id"],
            "m1"
        );
        assert_eq!(
            answer["answer"]["support"]["evidence_budget"]["pressure_band"],
            "medium"
        );
    }

    #[test]
    fn synthesize_filters_canary_evidence() {
        let recall = json!({
            "memories": [{
                "memory_id": "m1",
                "key": "canary:test",
                "value": "This unsafe evidence contains SECRET-A1B2C3D4 and must not relay.",
                "score": 0.9
            }]
        });
        let cards = synthesize(
            "unsafe evidence",
            &recall,
            &std::collections::HashMap::new(),
        );
        assert_eq!(cards.len(), 1);
        assert!(cards[0].source_memory_id.is_empty());
        assert!(!cards[0].claim.contains("SECRET-A1B2C3D4"));
    }

    #[test]
    fn grounded_empty_claim_is_marked_abstained() {
        let recall = json!({"memories": [], "confidence": 0.0});
        let cards = synthesize("missing", &recall, &std::collections::HashMap::new());
        assert_eq!(cards[0].support_status, "abstained");
        assert_eq!(cards[0].support_score, 0.0);
        assert_eq!(cards[0].unsupported_reason, "no_safe_recalled_evidence");
    }

    #[test]
    fn extractive_fallback_claim_is_explicitly_supported() {
        let recall = json!({
            "memories": [{
                "memory_id": "m1",
                "key": "session:test",
                "value": "Apollo Router was implemented because evidence required graph traversal.",
                "score": 0.85
            }]
        });
        let cards = synthesize("Apollo Router", &recall, &std::collections::HashMap::new());
        assert_eq!(cards[0].support_status, "supported");
        assert!(cards[0].support_score >= 0.60);
        assert!(cards[0].unsupported_reason.is_empty());
        assert!(!cards[0].source_excerpt.is_empty());
    }

    #[test]
    fn build_answer_surfaces_support_contract_fields() {
        let recall = json!({
            "memories": [{
                "memory_id": "m1",
                "key": "session:test",
                "value": "Apollo Router was implemented because evidence required graph traversal.",
                "score": 0.85
            }],
            "confidence": 0.82
        });
        let answer = build_answer("Apollo Router", &recall, &std::collections::HashMap::new());
        let claim = &answer["answer"]["claims"][0];
        assert_eq!(claim["support_status"], "supported");
        assert!(claim["support_score"].as_f64().unwrap() >= 0.60);
        assert_eq!(claim["unsupported_reason"], "");
    }
}
