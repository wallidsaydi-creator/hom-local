use crate::services::entity_extraction;
use crate::services::ranking_service::RankedItem;
use crate::services::recall_lineage::{
    MemoryEntityGraph, PPR_DAMPING, PPR_ITERATIONS, lineage_ppr,
};

pub fn rank(query: &str, graph: &MemoryEntityGraph) -> Vec<RankedItem> {
    let mut seed_entities = entity_extraction::query_entity_candidates(query)
        .into_iter()
        .filter(|entity| graph.contains_entity(entity))
        .collect::<Vec<_>>();
    seed_entities.sort();
    seed_entities.dedup();
    if seed_entities.is_empty() {
        return Vec::new();
    }

    lineage_ppr(&seed_entities, graph, PPR_DAMPING, PPR_ITERATIONS)
        .into_iter()
        .map(|(memory_id, score)| RankedItem::new(memory_id, score))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_walk_rank_returns_ppr_memories() {
        let mut graph = MemoryEntityGraph::new();
        graph.add_memory_entity("m1", "Alpha", 1.0);
        graph.add_memory_entity("m2", "Beta", 1.0);
        graph.add_entity_relationship("Alpha", "Beta", 1.0);

        let ranked = rank("Alpha", &graph);
        let ids = ranked
            .iter()
            .map(|item| item.memory_id.as_str())
            .collect::<Vec<_>>();
        assert!(ids.contains(&"m1"));
        assert!(ids.contains(&"m2"));
    }
}
