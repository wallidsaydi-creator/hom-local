use std::collections::HashSet;

use rusqlite::Connection;
use serde_json::{Value, json};

use crate::services::recall_lineage::{MemoryEntityGraph, normalize_entity};

#[derive(Clone, Debug, PartialEq)]
pub struct HippographStats {
    pub entity_count: usize,
    pub memory_entity_edge_count: usize,
    pub relationship_edge_count: usize,
    pub entity_cap: usize,
}

impl HippographStats {
    pub fn to_json(&self) -> Value {
        json!({
            "entity_count": self.entity_count,
            "memory_entity_edge_count": self.memory_entity_edge_count,
            "relationship_edge_count": self.relationship_edge_count,
            "entity_cap": self.entity_cap,
            "source": "memory_entities + memory_relationships",
            "algorithm": "HippoRAG/MAGMA bipartite memory-entity graph"
        })
    }
}

pub fn build_from_conn(
    conn: &Connection,
    entity_cap: usize,
) -> anyhow::Result<(MemoryEntityGraph, HippographStats)> {
    let mut graph = MemoryEntityGraph::new();
    let entity_cap = entity_cap.max(1);
    let mut entity_stmt = conn.prepare(
        "SELECT entity
         FROM memory_entities
         GROUP BY entity
         ORDER BY SUM(frequency) DESC, entity ASC
         LIMIT ?1",
    )?;
    let entities = entity_stmt
        .query_map([entity_cap as i64], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if entities.is_empty() {
        return Ok((
            graph,
            HippographStats {
                entity_count: 0,
                memory_entity_edge_count: 0,
                relationship_edge_count: 0,
                entity_cap,
            },
        ));
    }

    let entity_set: HashSet<_> = entities
        .iter()
        .map(|entity| normalize_entity(entity))
        .collect();
    let mut memory_entity_edge_count = 0usize;
    let mut relationship_edge_count = 0usize;

    let mut memory_stmt = conn.prepare(
        "SELECT memory_id, entity, frequency
         FROM memory_entities
         ORDER BY entity ASC, memory_id ASC",
    )?;
    let memory_edges = memory_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (memory_id, entity, frequency) in memory_edges {
        let normalized = normalize_entity(&entity);
        if entity_set.contains(&normalized) {
            graph.add_memory_entity(memory_id, normalized, frequency as f64);
            memory_entity_edge_count += 1;
        }
    }

    let mut rel_stmt = conn.prepare(
        "SELECT source_entity, target_entity, weight
         FROM memory_relationships
         WHERE decayed_at_s IS NULL
         ORDER BY source_entity ASC, target_entity ASC",
    )?;
    let relationships = rel_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (source, target, weight) in relationships {
        let source = normalize_entity(&source);
        let target = normalize_entity(&target);
        if entity_set.contains(&source) && entity_set.contains(&target) {
            graph.add_entity_relationship(source, target, weight);
            relationship_edge_count += 1;
        }
    }

    Ok((
        graph,
        HippographStats {
            entity_count: entity_set.len(),
            memory_entity_edge_count,
            relationship_edge_count,
            entity_cap,
        },
    ))
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use super::*;
    use crate::db::migrations::run_migrations;

    #[test]
    fn hippograph_builder_loads_memory_entity_relationship_graph() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO memories
             (id, key, value, memory_type, source, created_at_s, updated_at_s, quality_score, metadata_json)
             VALUES ('m1', 'Alpha', 'Alpha links Beta', 'fact', 'test', 1, 1, 0.9, '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memory_entities
             (id, memory_id, entity, entity_type, frequency, created_at_s)
             VALUES ('e1', 'm1', 'Alpha', 'proper_noun', 2, 1), ('e2', 'm1', 'Beta', 'proper_noun', 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memory_relationships
             (id, source_entity, target_entity, relationship_type, weight, memory_id, created_at_s)
             VALUES (?1, 'Alpha', 'Beta', 'co_occurs', 1.0, 'm1', 1)",
            params!["r1"],
        )
        .unwrap();

        let (graph, stats) = build_from_conn(&conn, 10).unwrap();
        assert_eq!(stats.entity_count, 2);
        assert_eq!(stats.memory_entity_edge_count, 2);
        assert_eq!(stats.relationship_edge_count, 1);
        assert!(graph.contains_entity("alpha"));
    }
}
