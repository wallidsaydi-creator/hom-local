use std::collections::{HashMap, HashSet};

use crate::services::entity_extraction;

use super::ranking_service::RankedItem;

pub const PPR_DAMPING: f64 = 0.85;
pub const PPR_ITERATIONS: usize = 10;
pub const ENTITY_NODE_CAP: usize = 500;

#[derive(Clone, Debug, Default)]
pub struct MemoryEntityGraph {
    entity_to_memories: HashMap<String, Vec<WeightedEdge>>,
    memory_to_entities: HashMap<String, Vec<WeightedEdge>>,
    entity_to_entities: HashMap<String, Vec<WeightedEdge>>,
}

#[derive(Clone, Debug)]
struct WeightedEdge {
    target: String,
    weight: f64,
}

impl MemoryEntityGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_memory_entity(
        &mut self,
        memory_id: impl Into<String>,
        entity: impl Into<String>,
        weight: f64,
    ) {
        let memory_id = memory_id.into();
        let entity = normalize_entity(&entity.into());
        let weight = weight.max(0.01);
        self.entity_to_memories
            .entry(entity.clone())
            .or_default()
            .push(WeightedEdge {
                target: memory_id.clone(),
                weight,
            });
        self.memory_to_entities
            .entry(memory_id)
            .or_default()
            .push(WeightedEdge {
                target: entity,
                weight,
            });
    }

    pub fn add_entity_relationship(
        &mut self,
        source: impl Into<String>,
        target: impl Into<String>,
        weight: f64,
    ) {
        let source = normalize_entity(&source.into());
        let target = normalize_entity(&target.into());
        if source == target {
            return;
        }
        let weight = weight.max(0.01);
        self.entity_to_entities
            .entry(source.clone())
            .or_default()
            .push(WeightedEdge {
                target: target.clone(),
                weight,
            });
        self.entity_to_entities
            .entry(target)
            .or_default()
            .push(WeightedEdge {
                target: source,
                weight,
            });
    }

    pub fn contains_entity(&self, entity: &str) -> bool {
        self.entity_to_memories
            .contains_key(&normalize_entity(entity))
    }

    pub fn entity_count(&self) -> usize {
        self.entity_to_memories.len()
    }

    fn neighbors(&self, node: &GraphNode) -> Vec<(GraphNode, f64)> {
        match node {
            GraphNode::Entity(entity) => {
                let mut neighbors = Vec::new();
                if let Some(memories) = self.entity_to_memories.get(entity) {
                    neighbors.extend(
                        memories
                            .iter()
                            .map(|edge| (GraphNode::Memory(edge.target.clone()), edge.weight)),
                    );
                }
                if let Some(entities) = self.entity_to_entities.get(entity) {
                    neighbors.extend(
                        entities
                            .iter()
                            .map(|edge| (GraphNode::Entity(edge.target.clone()), edge.weight)),
                    );
                }
                neighbors
            }
            GraphNode::Memory(memory_id) => self
                .memory_to_entities
                .get(memory_id)
                .map(|edges| {
                    edges
                        .iter()
                        .map(|edge| (GraphNode::Entity(edge.target.clone()), edge.weight))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
enum GraphNode {
    Entity(String),
    Memory(String),
}

pub fn rank(query: &str, graph: &MemoryEntityGraph) -> Vec<RankedItem> {
    let seed_entities: Vec<String> = entity_extraction::query_entity_candidates(query)
        .into_iter()
        .filter(|entity| graph.contains_entity(entity))
        .collect();
    if seed_entities.is_empty() {
        return Vec::new();
    }

    lineage_ppr(&seed_entities, graph, PPR_DAMPING, PPR_ITERATIONS)
        .into_iter()
        .map(|(memory_id, score)| RankedItem::new(memory_id, score))
        .collect()
}

pub fn lineage_ppr(
    query_entities: &[String],
    graph: &MemoryEntityGraph,
    alpha: f64,
    max_iterations: usize,
) -> Vec<(String, f64)> {
    let seeds: Vec<_> = query_entities
        .iter()
        .map(|entity| normalize_entity(entity))
        .filter(|entity| graph.contains_entity(entity))
        .collect();
    if seeds.is_empty() {
        return Vec::new();
    }

    let mut nodes = HashSet::new();
    for seed in &seeds {
        collect_reachable(graph, GraphNode::Entity(seed.clone()), 4, &mut nodes);
    }

    let seed_weight = 1.0 / seeds.len() as f64;
    let mut scores: HashMap<GraphNode, f64> =
        nodes.iter().cloned().map(|node| (node, 0.0)).collect();
    for seed in &seeds {
        scores.insert(GraphNode::Entity(seed.clone()), seed_weight);
    }

    for _ in 0..max_iterations {
        let mut next: HashMap<GraphNode, f64> =
            nodes.iter().cloned().map(|node| (node, 0.0)).collect();

        for seed in &seeds {
            *next.entry(GraphNode::Entity(seed.clone())).or_default() +=
                (1.0 - alpha) * seed_weight;
        }

        for (node, score) in &scores {
            if *score <= 0.0 {
                continue;
            }
            let neighbors = graph.neighbors(node);
            if neighbors.is_empty() {
                continue;
            }
            let total_weight: f64 = neighbors.iter().map(|(_, weight)| *weight).sum();
            if total_weight <= 0.0 {
                continue;
            }
            for (neighbor, weight) in neighbors {
                if nodes.contains(&neighbor) {
                    *next.entry(neighbor).or_default() += alpha * score * (weight / total_weight);
                }
            }
        }

        scores = next;
    }

    let mut ranked: Vec<_> = scores
        .into_iter()
        .filter_map(|(node, score)| match node {
            GraphNode::Memory(memory_id) if score > 0.0 && score.is_finite() => {
                Some((memory_id, score))
            }
            _ => None,
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked
}

fn collect_reachable(
    graph: &MemoryEntityGraph,
    node: GraphNode,
    remaining_hops: usize,
    nodes: &mut HashSet<GraphNode>,
) {
    if !nodes.insert(node.clone()) || remaining_hops == 0 {
        return;
    }
    for (neighbor, _) in graph.neighbors(&node) {
        collect_reachable(graph, neighbor, remaining_hops - 1, nodes);
    }
}

pub fn normalize_entity(entity: &str) -> String {
    entity.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ppr_with_single_query_entity_returns_related_memories() {
        let mut graph = MemoryEntityGraph::new();
        graph.add_memory_entity("m1", "apollo", 1.0);
        graph.add_memory_entity("m2", "apollo", 1.0);
        graph.add_memory_entity("m2", "router", 1.0);

        let ranked = lineage_ppr(&["apollo".to_string()], &graph, PPR_DAMPING, PPR_ITERATIONS);
        let ids: Vec<_> = ranked.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"m1"));
        assert!(ids.contains(&"m2"));
    }

    #[test]
    fn disconnected_query_entity_returns_empty() {
        let mut graph = MemoryEntityGraph::new();
        graph.add_memory_entity("m1", "apollo", 1.0);
        assert!(lineage_ppr(&["zeus".to_string()], &graph, PPR_DAMPING, PPR_ITERATIONS).is_empty());
    }

    #[test]
    fn ppr_scores_are_finite_on_large_capped_graph() {
        let mut graph = MemoryEntityGraph::new();
        for i in 0..ENTITY_NODE_CAP {
            graph.add_memory_entity(format!("m{i}"), format!("entity{i}"), 1.0);
            if i > 0 {
                graph.add_entity_relationship(
                    format!("entity{}", i - 1),
                    format!("entity{i}"),
                    1.0,
                );
            }
        }

        let ranked = lineage_ppr(
            &["entity0".to_string()],
            &graph,
            PPR_DAMPING,
            PPR_ITERATIONS,
        );
        assert!(!ranked.is_empty());
        assert!(ranked.iter().all(|(_, score)| score.is_finite()));
    }

    #[test]
    fn rank_uses_query_entity_candidates() {
        let mut graph = MemoryEntityGraph::new();
        graph.add_memory_entity("m1", "apollo", 1.0);
        let ranked = rank("apollo lineage", &graph);
        assert_eq!(ranked[0].memory_id, "m1");
    }
}
