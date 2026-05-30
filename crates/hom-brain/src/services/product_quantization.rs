use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductQuantizationConfig {
    pub dimensions: usize,
    pub subquantizers: usize,
    pub centroids_per_subquantizer: usize,
    pub iterations: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuantizedCode {
    pub memory_id: String,
    pub codes: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductQuantizer {
    config: ProductQuantizationConfig,
    subvector_dimensions: usize,
    codebooks: Vec<Vec<Vec<f64>>>,
}

impl ProductQuantizer {
    pub fn fit(
        vectors: &[(String, Vec<f64>)],
        config: ProductQuantizationConfig,
    ) -> Result<Self, String> {
        validate_config(config)?;
        if vectors.is_empty() {
            return Err("pq_training_vectors_empty".to_string());
        }
        if vectors
            .iter()
            .any(|(_, vector)| vector.len() != config.dimensions)
        {
            return Err("pq_training_dimension_mismatch".to_string());
        }
        let subvector_dimensions = config.dimensions / config.subquantizers;
        let mut codebooks = Vec::with_capacity(config.subquantizers);
        for subquantizer in 0..config.subquantizers {
            let start = subquantizer * subvector_dimensions;
            let end = start + subvector_dimensions;
            let samples = vectors
                .iter()
                .map(|(_, vector)| vector[start..end].to_vec())
                .collect::<Vec<_>>();
            codebooks.push(fit_codebook(
                &samples,
                config.centroids_per_subquantizer,
                config.iterations,
            ));
        }
        Ok(Self {
            config,
            subvector_dimensions,
            codebooks,
        })
    }

    pub fn formula_ref(&self) -> &'static str {
        "product_quantization_adc_v1"
    }

    pub fn mutation_permitted(&self) -> bool {
        false
    }

    pub fn codebooks(&self) -> &[Vec<Vec<f64>>] {
        &self.codebooks
    }

    pub fn encode_all(&self, vectors: &[(String, Vec<f64>)]) -> Result<Vec<QuantizedCode>, String> {
        vectors
            .iter()
            .map(|(memory_id, vector)| self.encode(memory_id, vector))
            .collect()
    }

    pub fn encode(&self, memory_id: &str, vector: &[f64]) -> Result<QuantizedCode, String> {
        if vector.len() != self.config.dimensions {
            return Err("pq_encode_dimension_mismatch".to_string());
        }
        let mut codes = Vec::with_capacity(self.config.subquantizers);
        for subquantizer in 0..self.config.subquantizers {
            let start = subquantizer * self.subvector_dimensions;
            let end = start + self.subvector_dimensions;
            let centroid = nearest_centroid(&vector[start..end], &self.codebooks[subquantizer]);
            codes.push(centroid);
        }
        Ok(QuantizedCode {
            memory_id: memory_id.to_string(),
            codes,
        })
    }

    pub fn approximate_search(
        &self,
        codes: &[QuantizedCode],
        query: &[f64],
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        if query.len() != self.config.dimensions {
            return Err("pq_query_dimension_mismatch".to_string());
        }
        let limit = limit.max(1);
        let mut scored = codes
            .iter()
            .map(|code| {
                let distance = self.adc_distance(code, query);
                json!({
                    "memory_id": code.memory_id,
                    "approximate_distance": distance,
                    "score": 1.0 / (1.0 + distance),
                    "codes": code.codes,
                    "formula_ref": self.formula_ref(),
                    "mutation_permitted": self.mutation_permitted(),
                    "paper_anchor": "Product Quantization for Nearest Neighbor Search; asymmetric distance computation over subvector codebooks."
                })
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            left["approximate_distance"]
                .as_f64()
                .unwrap_or(f64::INFINITY)
                .partial_cmp(
                    &right["approximate_distance"]
                        .as_f64()
                        .unwrap_or(f64::INFINITY),
                )
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit.min(scored.len()));
        Ok(scored)
    }

    fn adc_distance(&self, code: &QuantizedCode, query: &[f64]) -> f64 {
        code.codes
            .iter()
            .enumerate()
            .map(|(subquantizer, centroid_index)| {
                let start = subquantizer * self.subvector_dimensions;
                let end = start + self.subvector_dimensions;
                squared_l2(
                    &query[start..end],
                    &self.codebooks[subquantizer][*centroid_index],
                )
            })
            .sum()
    }
}

fn validate_config(config: ProductQuantizationConfig) -> Result<(), String> {
    if config.dimensions == 0 || config.subquantizers == 0 || config.centroids_per_subquantizer == 0
    {
        return Err("pq_config_zero_value".to_string());
    }
    if !config.dimensions.is_multiple_of(config.subquantizers) {
        return Err("pq_dimensions_not_divisible_by_subquantizers".to_string());
    }
    Ok(())
}

fn fit_codebook(samples: &[Vec<f64>], centroids: usize, iterations: usize) -> Vec<Vec<f64>> {
    let mut codebook = samples.iter().take(centroids).cloned().collect::<Vec<_>>();
    while codebook.len() < centroids {
        codebook.push(vec![0.0; samples.first().map_or(0, Vec::len)]);
    }
    for _ in 0..iterations.max(1) {
        let mut assigned = vec![Vec::<Vec<f64>>::new(); centroids];
        for sample in samples {
            assigned[nearest_centroid(sample, &codebook)].push(sample.clone());
        }
        for (idx, bucket) in assigned.into_iter().enumerate() {
            if !bucket.is_empty() {
                codebook[idx] = mean_vector(&bucket);
            }
        }
    }
    codebook
}

fn nearest_centroid(sample: &[f64], codebook: &[Vec<f64>]) -> usize {
    codebook
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            squared_l2(sample, left)
                .partial_cmp(&squared_l2(sample, right))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(idx, _)| idx)
        .unwrap_or(0)
}

fn mean_vector(bucket: &[Vec<f64>]) -> Vec<f64> {
    let dimensions = bucket.first().map_or(0, Vec::len);
    let mut mean = vec![0.0; dimensions];
    for vector in bucket {
        for (idx, value) in vector.iter().enumerate() {
            mean[idx] += value;
        }
    }
    for value in &mut mean {
        *value /= bucket.len() as f64;
    }
    mean
}

fn squared_l2(left: &[f64], right: &[f64]) -> f64 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| {
            let delta = left - right;
            delta * delta
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_quantizer_fits_codebooks_encodes_and_preserves_nearest_neighbor() {
        let vectors = vec![
            ("left".to_string(), vec![1.0, 0.0, 1.0, 0.0]),
            ("up".to_string(), vec![0.0, 1.0, 0.0, 1.0]),
            ("left-soft".to_string(), vec![0.9, 0.1, 0.8, 0.2]),
        ];
        let quantizer = ProductQuantizer::fit(
            &vectors,
            ProductQuantizationConfig {
                dimensions: 4,
                subquantizers: 2,
                centroids_per_subquantizer: 2,
                iterations: 4,
            },
        )
        .unwrap();
        let codes = quantizer.encode_all(&vectors).unwrap();
        let matches = quantizer
            .approximate_search(&codes, &[0.95, 0.05, 0.9, 0.1], 2)
            .unwrap();

        assert_eq!(quantizer.formula_ref(), "product_quantization_adc_v1");
        assert_eq!(quantizer.mutation_permitted(), false);
        assert_eq!(quantizer.codebooks().len(), 2);
        assert_eq!(codes[0].codes.len(), 2);
        assert_eq!(matches[0]["memory_id"], "left");
        assert_eq!(matches[0]["formula_ref"], "product_quantization_adc_v1");
        assert_eq!(matches[0]["mutation_permitted"], false);
    }

    #[test]
    fn product_quantizer_rejects_invalid_subvector_layout() {
        let error = ProductQuantizer::fit(
            &[],
            ProductQuantizationConfig {
                dimensions: 5,
                subquantizers: 2,
                centroids_per_subquantizer: 2,
                iterations: 1,
            },
        )
        .unwrap_err();

        assert!(error.contains("pq_dimensions_not_divisible_by_subquantizers"));
    }
}
