use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub struct EmbeddingRecord {
    pub memory_id: String,
    pub vector: Vec<f64>,
    pub model: String,
    pub dimensions: usize,
}

#[derive(Clone, Debug, Default)]
pub struct VectorIndex {
    dimensions: usize,
    records: Vec<EmbeddingRecord>,
}

impl VectorIndex {
    pub fn new(dimensions: usize) -> Self {
        Self {
            dimensions,
            records: Vec::new(),
        }
    }

    pub fn upsert(&mut self, record: EmbeddingRecord) -> Result<(), String> {
        if record.dimensions != self.dimensions || record.vector.len() != self.dimensions {
            return Err(format!(
                "embedding_dimension_mismatch: expected {}, got declared {} len {}",
                self.dimensions,
                record.dimensions,
                record.vector.len()
            ));
        }
        if record.memory_id.trim().is_empty() {
            return Err("embedding_memory_id_empty".to_string());
        }
        if let Some(existing) = self
            .records
            .iter_mut()
            .find(|existing| existing.memory_id == record.memory_id)
        {
            *existing = record;
        } else {
            self.records.push(record);
        }
        Ok(())
    }

    pub fn exact_search(&self, query: &[f64], limit: usize) -> Result<Vec<Value>, String> {
        if query.len() != self.dimensions {
            return Err(format!(
                "embedding_query_dimension_mismatch: expected {}, got {}",
                self.dimensions,
                query.len()
            ));
        }
        let limit = limit.max(1);
        let mut scored = self
            .records
            .iter()
            .map(|record| {
                let score = cosine_similarity(query, &record.vector);
                json!({
                    "memory_id": record.memory_id,
                    "score": score,
                    "model": record.model,
                    "dimensions": record.dimensions,
                    "formula_ref": "exact_cosine_vector_search_v1",
                    "mutation_permitted": false,
                    "paper_anchor": "Vector substrate baseline before TurboQuant/Product Quantization; exact cosine search is the reference path for approximate-distance regression."
                })
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right["score"]
                .as_f64()
                .unwrap_or(f64::NEG_INFINITY)
                .partial_cmp(&left["score"].as_f64().unwrap_or(f64::NEG_INFINITY))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit.min(scored.len()));
        Ok(scored)
    }
}

pub fn cosine_similarity(left: &[f64], right: &[f64]) -> f64 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let dot = left
        .iter()
        .zip(right.iter())
        .map(|(a, b)| a * b)
        .sum::<f64>();
    let left_norm = left.iter().map(|value| value * value).sum::<f64>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f64>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm * right_norm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_index_exact_search_returns_nearest_neighbor_with_source_metadata() {
        let mut index = VectorIndex::new(3);
        index
            .upsert(EmbeddingRecord {
                memory_id: "m-left".to_string(),
                vector: vec![1.0, 0.0, 0.0],
                model: "unit-test-embedding".to_string(),
                dimensions: 3,
            })
            .unwrap();
        index
            .upsert(EmbeddingRecord {
                memory_id: "m-up".to_string(),
                vector: vec![0.0, 1.0, 0.0],
                model: "unit-test-embedding".to_string(),
                dimensions: 3,
            })
            .unwrap();

        let result = index.exact_search(&[0.9, 0.1, 0.0], 2).unwrap();

        assert_eq!(result[0]["memory_id"], "m-left");
        assert_eq!(result[0]["formula_ref"], "exact_cosine_vector_search_v1");
        assert_eq!(result[0]["mutation_permitted"], false);
        assert_eq!(result[0]["dimensions"], 3);
        assert!(result[0]["score"].as_f64().unwrap() > result[1]["score"].as_f64().unwrap());
    }

    #[test]
    fn vector_index_rejects_dimension_mismatches() {
        let mut index = VectorIndex::new(3);
        let error = index
            .upsert(EmbeddingRecord {
                memory_id: "bad".to_string(),
                vector: vec![1.0, 0.0],
                model: "unit-test-embedding".to_string(),
                dimensions: 2,
            })
            .unwrap_err();

        assert!(error.contains("embedding_dimension_mismatch"));
    }
}
