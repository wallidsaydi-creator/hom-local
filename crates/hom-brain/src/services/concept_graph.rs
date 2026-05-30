use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

pub fn concept_for_entity(entity: &str) -> String {
    let normalized = entity
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .to_lowercase();
    normalized
        .split_whitespace()
        .find(|token| token.len() >= 4)
        .unwrap_or("misc")
        .to_string()
}

pub fn build_concepts<I>(entities: I) -> Value
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut concepts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for entity in entities {
        let entity = entity.as_ref().trim();
        if entity.is_empty() {
            continue;
        }
        concepts
            .entry(concept_for_entity(entity))
            .or_default()
            .insert(entity.to_string());
    }

    json!({
        "ok": true,
        "algorithm": "deterministic_entity_token_concept_projection",
        "concepts": concepts.into_iter().map(|(concept, entities)| {
            json!({
                "concept": concept,
                "entities": entities.into_iter().collect::<Vec<_>>()
            })
        }).collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concept_graph_groups_entities_deterministically() {
        let graph = build_concepts(["Alpha Service", "Alpha Runtime", "Beta"]);
        let concepts = graph["concepts"].as_array().unwrap();
        assert_eq!(concepts[0]["concept"], "alpha");
        assert_eq!(concepts[0]["entities"].as_array().unwrap().len(), 2);
    }
}
