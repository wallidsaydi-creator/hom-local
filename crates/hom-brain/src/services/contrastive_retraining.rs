use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub struct ContrastivePair {
    pub memory_id: String,
    pub mode: String,
    pub label: String,
    pub weight: f64,
}

pub fn pairs_from_counts(
    memory_id: &str,
    mode: &str,
    success_count: i64,
    failure_count: i64,
) -> Vec<ContrastivePair> {
    let mut pairs = Vec::new();
    if success_count > 0 {
        pairs.push(ContrastivePair {
            memory_id: memory_id.to_string(),
            mode: mode.to_string(),
            label: "positive".to_string(),
            weight: success_count as f64,
        });
    }
    if failure_count > 0 {
        pairs.push(ContrastivePair {
            memory_id: memory_id.to_string(),
            mode: mode.to_string(),
            label: "negative".to_string(),
            weight: failure_count as f64,
        });
    }
    pairs
}

pub fn pairs_json(pairs: &[ContrastivePair]) -> Value {
    json!({
        "formula_ref": "contrastive_retrieval_pairs_v1",
        "pairs": pairs.iter().map(|pair| {
            json!({
                "memory_id": pair.memory_id,
                "mode": pair.mode,
                "label": pair.label,
                "weight": pair.weight
            })
        }).collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrastive_retraining_emits_positive_and_negative_pairs() {
        let pairs = pairs_from_counts("m1", "text", 3, 2);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].label, "positive");
        assert_eq!(pairs[1].label, "negative");
    }
}
