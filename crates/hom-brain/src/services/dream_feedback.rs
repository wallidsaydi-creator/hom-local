use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DreamFeedbackCounts {
    pub held: usize,
    pub superseded: usize,
    pub contradicted: usize,
}

pub fn rates(counts: DreamFeedbackCounts) -> Value {
    let total = counts.held + counts.superseded + counts.contradicted;
    let ratio = |count: usize| {
        if total == 0 {
            0.0
        } else {
            count as f64 / total as f64
        }
    };
    json!({
        "ok": true,
        "formula_ref": "dream_feedback_rates_v1",
        "total": total,
        "held_rate": ratio(counts.held),
        "superseded_rate": ratio(counts.superseded),
        "contradicted_rate": ratio(counts.contradicted)
    })
}

pub fn apply_to_score(base_score: f64, counts: DreamFeedbackCounts) -> Value {
    let rate_payload = rates(counts);
    let held_rate = rate_payload["held_rate"].as_f64().unwrap_or(0.0);
    let superseded_rate = rate_payload["superseded_rate"].as_f64().unwrap_or(0.0);
    let contradicted_rate = rate_payload["contradicted_rate"].as_f64().unwrap_or(0.0);
    let raw_adjustment = (0.10 * held_rate) - (0.15 * superseded_rate) - (0.25 * contradicted_rate);
    let adjustment = raw_adjustment.clamp(-0.25, 0.10);
    let adjusted_score = (base_score + adjustment).clamp(0.0, 1.0);

    json!({
        "ok": true,
        "formula_ref": "self_rag_dream_feedback_v2",
        "paper_anchor": "Self-RAG: Learning to Retrieve, Generate, and Critique through Self-Reflection (arXiv:2310.11511); HOM Reasoning Graphs evidence-centric feedback anchor",
        "base_score": base_score.clamp(0.0, 1.0),
        "adjustment": adjustment,
        "adjusted_score": adjusted_score,
        "bounds": { "min_adjustment": -0.25, "max_adjustment": 0.10 },
        "reflection_tokens": ["Retrieve", "IsRel", "IsSup", "Utility"],
        "formula_components": {
            "held_rate": held_rate,
            "superseded_rate": superseded_rate,
            "contradicted_rate": contradicted_rate,
            "raw_adjustment": raw_adjustment,
            "adjustment_formula": "clamp(-0.25, 0.10, 0.10*held_rate - 0.15*superseded_rate - 0.25*contradicted_rate)"
        },
        "downstream_effect": "bounded_score_adjustment",
        "mutation_permitted": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dream_feedback_rates_are_normalized() {
        let result = rates(DreamFeedbackCounts {
            held: 2,
            superseded: 1,
            contradicted: 1,
        });
        assert_eq!(result["total"], 4);
        assert_eq!(result["held_rate"], 0.5);
    }

    #[test]
    fn dream_feedback_applies_self_rag_reflection_adjustment_to_downstream_score() {
        let contradicted = apply_to_score(
            0.70,
            DreamFeedbackCounts {
                held: 0,
                superseded: 1,
                contradicted: 3,
            },
        );
        assert_eq!(contradicted["formula_ref"], "self_rag_dream_feedback_v2");
        assert!(
            contradicted["paper_anchor"]
                .as_str()
                .unwrap()
                .contains("Self-RAG")
        );
        assert_eq!(contradicted["reflection_tokens"][0], "Retrieve");
        assert!(contradicted["adjusted_score"].as_f64().unwrap() < 0.70);
        assert!(contradicted["adjustment"].as_f64().unwrap() >= -0.25);

        let held = apply_to_score(
            0.70,
            DreamFeedbackCounts {
                held: 4,
                superseded: 0,
                contradicted: 0,
            },
        );
        assert!(held["adjusted_score"].as_f64().unwrap() > 0.70);
        assert!(held["adjustment"].as_f64().unwrap() <= 0.10);
    }
}
