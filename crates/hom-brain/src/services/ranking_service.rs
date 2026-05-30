use std::cmp::Ordering;
use std::collections::HashMap;

use serde_json::{Value, json};

pub const RRF_K: usize = 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum RecallMode {
    ExactArtifact,
    GraphWalk,
    Identifier,
    Lineage,
    Temporal,
    Text,
    VectorExact,
    VectorHybridExactRerank,
}

impl RecallMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RecallMode::ExactArtifact => "exact_artifact",
            RecallMode::GraphWalk => "graph_walk",
            RecallMode::Identifier => "identifier",
            RecallMode::Lineage => "lineage",
            RecallMode::Temporal => "temporal",
            RecallMode::Text => "text",
            RecallMode::VectorExact => "vector_exact",
            RecallMode::VectorHybridExactRerank => "vector_hybrid_exact_rerank",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RankedItem {
    pub memory_id: String,
    pub mode_score: f64,
    pub exact_pin: bool,
}

impl RankedItem {
    pub fn new(memory_id: impl Into<String>, mode_score: f64) -> Self {
        Self {
            memory_id: memory_id.into(),
            mode_score,
            exact_pin: false,
        }
    }

    pub fn pinned(memory_id: impl Into<String>, mode_score: f64) -> Self {
        Self {
            memory_id: memory_id.into(),
            mode_score,
            exact_pin: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FusedItem {
    pub memory_id: String,
    pub score: f64,
    pub best_rank: usize,
    pub exact_pin: bool,
    pub mode_ranks: Vec<(RecallMode, usize)>,
    pub mode_weights: Vec<(RecallMode, f64)>,
}

impl FusedItem {
    pub fn components(&self) -> Value {
        let mode_ranks: Vec<Value> = self
            .mode_ranks
            .iter()
            .map(|(mode, rank)| json!({"mode": mode.as_str(), "rank": rank}))
            .collect();
        json!({
            "rrf_score": self.score,
            "rrf_k": RRF_K,
            "best_rank": self.best_rank,
            "exact_pin": self.exact_pin,
            "mode_ranks": mode_ranks,
            "mode_weights": self.mode_weights.iter().map(|(mode, weight)| {
                json!({"mode": mode.as_str(), "weight": weight})
            }).collect::<Vec<_>>()
        })
    }
}

#[derive(Clone, Debug)]
struct Accumulator {
    score: f64,
    best_rank: usize,
    exact_pin: bool,
    mode_ranks: Vec<(RecallMode, usize)>,
    mode_weights: Vec<(RecallMode, f64)>,
}

pub fn rrf_fuse(rank_lists: &[(RecallMode, Vec<RankedItem>)], k: usize) -> Vec<FusedItem> {
    rrf_fuse_weighted(rank_lists, k, |_, _| 1.0)
}

pub fn rrf_fuse_weighted<F>(
    rank_lists: &[(RecallMode, Vec<RankedItem>)],
    k: usize,
    weight_for: F,
) -> Vec<FusedItem>
where
    F: Fn(&str, RecallMode) -> f64,
{
    if rank_lists.is_empty() {
        return Vec::new();
    }

    let mut scores: HashMap<String, Accumulator> = HashMap::new();
    for (mode, items) in rank_lists {
        for (index, item) in items.iter().enumerate() {
            let rank = index + 1;
            let entry = scores
                .entry(item.memory_id.clone())
                .or_insert_with(|| Accumulator {
                    score: 0.0,
                    best_rank: rank,
                    exact_pin: false,
                    mode_ranks: Vec::new(),
                    mode_weights: Vec::new(),
                });
            let weight = weight_for(&item.memory_id, *mode).clamp(0.50, 1.00);
            entry.score += weight / (k as f64 + rank as f64);
            entry.best_rank = entry.best_rank.min(rank);
            entry.exact_pin |= item.exact_pin;
            entry.mode_ranks.push((*mode, rank));
            entry.mode_weights.push((*mode, weight));
        }
    }

    let mut fused: Vec<_> = scores
        .into_iter()
        .map(|(memory_id, mut acc)| {
            acc.mode_ranks
                .sort_by(|(mode_a, rank_a), (mode_b, rank_b)| {
                    mode_a.cmp(mode_b).then(rank_a.cmp(rank_b))
                });
            acc.mode_weights
                .sort_by(|(mode_a, weight_a), (mode_b, weight_b)| {
                    mode_a
                        .cmp(mode_b)
                        .then_with(|| weight_a.partial_cmp(weight_b).unwrap_or(Ordering::Equal))
                });
            FusedItem {
                memory_id,
                score: acc.score,
                best_rank: acc.best_rank,
                exact_pin: acc.exact_pin,
                mode_ranks: acc.mode_ranks,
                mode_weights: acc.mode_weights,
            }
        })
        .collect();

    fused.sort_by(compare_fused);
    fused
}

fn compare_fused(a: &FusedItem, b: &FusedItem) -> Ordering {
    b.exact_pin
        .cmp(&a.exact_pin)
        .then_with(|| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal))
        .then_with(|| a.best_rank.cmp(&b.best_rank))
        .then_with(|| a.memory_id.cmp(&b.memory_id))
}

pub fn active_modes(rank_lists: &[(RecallMode, Vec<RankedItem>)]) -> Vec<&'static str> {
    rank_lists
        .iter()
        .filter(|(_, items)| !items.is_empty())
        .map(|(mode, _)| mode.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_mode_list_returns_no_results() {
        assert!(rrf_fuse(&[], RRF_K).is_empty());
    }

    #[test]
    fn single_mode_preserves_that_modes_ranking() {
        let fused = rrf_fuse(
            &[(
                RecallMode::Text,
                vec![
                    RankedItem::new("a", 1.0),
                    RankedItem::new("b", 0.9),
                    RankedItem::new("c", 0.8),
                ],
            )],
            RRF_K,
        );
        let ids: Vec<_> = fused.iter().map(|item| item.memory_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn fused_result_includes_all_contributing_modes() {
        let fused = rrf_fuse(
            &[
                (
                    RecallMode::ExactArtifact,
                    vec![RankedItem::new("a", 0.7), RankedItem::new("b", 0.7)],
                ),
                (
                    RecallMode::Text,
                    vec![RankedItem::new("c", 1.0), RankedItem::new("a", 0.5)],
                ),
            ],
            RRF_K,
        );
        let ids: Vec<_> = fused.iter().map(|item| item.memory_id.as_str()).collect();
        assert!(ids.contains(&"a"));
        assert!(ids.contains(&"b"));
        assert!(ids.contains(&"c"));
        let a = fused.iter().find(|item| item.memory_id == "a").unwrap();
        assert_eq!(a.mode_ranks.len(), 2);
    }

    #[test]
    fn exact_pin_beats_non_exact_multi_mode_candidate() {
        let fused = rrf_fuse(
            &[
                (
                    RecallMode::ExactArtifact,
                    vec![RankedItem::pinned("exact-id", 1.0)],
                ),
                (
                    RecallMode::Identifier,
                    vec![RankedItem::new("broad-hit", 1.0)],
                ),
                (RecallMode::Text, vec![RankedItem::new("broad-hit", 1.0)]),
            ],
            RRF_K,
        );
        assert_eq!(fused[0].memory_id, "exact-id");
        assert!(fused[0].exact_pin);
    }
}
