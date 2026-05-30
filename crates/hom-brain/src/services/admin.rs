use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use hom_shared::{RpcError, rpc_err};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::db::storage::{SaveInput, append_ledger_tx, unix_now_s, verify_ledger_conn};
use crate::services::monitoring;
use crate::services::product_quantization::{ProductQuantizationConfig, ProductQuantizer};
use crate::services::vector_index::cosine_similarity;

/// List imported packs and their status.
pub fn import_list(state: &crate::BrainState) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;

    let mut stmt = conn
        .prepare(
            "SELECT event_id, payload_json, created_at_s FROM ledger_events \
             WHERE event_type = 'import.pack' ORDER BY created_at_s DESC LIMIT 50",
        )
        .map_err(|e| rpc_err(-32603, &format!("import_list_prepare: {e}")))?;

    let imports: Vec<Value> = stmt
        .query_map([], |row| {
            let event_id: String = row.get(0)?;
            let payload: String = row.get::<_, String>(1)?;
            let created_at_s: i64 = row.get(2)?;
            let payload_val: Value = serde_json::from_str(&payload).unwrap_or(json!({}));
            Ok(json!({
                "id": event_id,
                "status": payload_val.get("status").and_then(|v| v.as_str()).unwrap_or("completed"),
                "memoriesImported": payload_val.get("memories_imported").and_then(|v| v.as_i64()).unwrap_or(0),
                "memoriesSkipped": payload_val.get("memories_skipped").and_then(|v| v.as_i64()).unwrap_or(0),
                "createdAtS": created_at_s,
            }))
        })
        .map_err(|e| rpc_err(-32603, &format!("import_list_query: {e}")))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(json!({"ok": true, "imports": imports}))
}

pub fn benchmarks_list(state: &crate::BrainState, _params: Value) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT event_id, payload_json, created_at_s
             FROM ledger_events
             WHERE event_type = 'benchmark.result'
             ORDER BY created_at_s DESC
             LIMIT 100",
        )
        .map_err(|e| rpc_err(-32603, &format!("benchmarks_list_prepare: {e}")))?;

    let benchmarks: Vec<Value> = stmt
        .query_map([], |row| {
            let event_id: String = row.get(0)?;
            let payload: String = row.get(1)?;
            let created_at_s: i64 = row.get(2)?;
            let payload_val: Value = serde_json::from_str(&payload).unwrap_or_else(|_| json!({}));
            Ok(monitoring::benchmark_result_artifact(
                &event_id,
                &payload_val,
                created_at_s,
            ))
        })
        .map_err(|e| rpc_err(-32603, &format!("benchmarks_list_query: {e}")))?
        .filter_map(|row| row.ok())
        .collect();

    Ok(monitoring::benchmark_registry_entry(benchmarks))
}

pub fn benchmarks_detail(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let benchmark_id = params
        .get("benchmark_id")
        .or_else(|| params.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "benchmark_id_required"))?;
    let conn = state.store.conn()?;
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT payload_json, created_at_s FROM ledger_events
             WHERE event_type = 'benchmark.result' AND event_id = ?1",
            [benchmark_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| rpc_err(-32603, &format!("benchmarks_detail_query: {e}")))?;

    let Some((payload, created_at_s)) = row else {
        return Err(rpc_err(-32041, "benchmark_not_found"));
    };
    let payload_val: Value = serde_json::from_str(&payload).unwrap_or_else(|_| json!({}));
    let artifact = monitoring::benchmark_result_artifact(benchmark_id, &payload_val, created_at_s);
    Ok(json!({
        "ok": true,
        "artifact_kind": "benchmark_result_v1",
        "benchmark_id": benchmark_id,
        "created_at_s": created_at_s,
        "result": artifact,
        "inspector_target": {"kind": "benchmark_result", "id": benchmark_id}
    }))
}

pub fn vector_exact_benchmark(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let model = params
        .get("embedding_model")
        .or_else(|| params.get("model"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "embedding_model_required"))?;
    let query_vector = vector_param(&params, "query_vector")?;
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
    let limit = limit.clamp(1, 100);
    let dimensions = query_vector.len();
    let corpus_size = state
        .store
        .embedding_corpus_size(model, dimensions)
        .map_err(|error| rpc_err(-32603, error.to_string()))?;
    let started = Instant::now();
    let matches = state
        .store
        .exact_vector_rank(model, &query_vector, limit)
        .map_err(|error| rpc_err(-32603, error.to_string()))?;
    let duration_ms = started.elapsed().as_millis() as u64;

    Ok(json!({
        "ok": true,
        "benchmark_kind": "vector_exact_baseline",
        "embedding_model": model.trim(),
        "dimensions": dimensions,
        "corpus_size": corpus_size,
        "limit": limit,
        "duration_ms": duration_ms,
        "formula_ref": "exact_cosine_vector_search_v1",
        "approximation": "none",
        "mutation_permitted": false,
        "matches": matches,
        "search": {
            "mode": "exact_vector",
            "model": model.trim(),
            "dimensions": dimensions,
            "formula_ref": "exact_cosine_vector_search_v1",
            "approximation": "none",
            "mutation_permitted": false,
            "paper_anchor": "Exact cosine baseline; ANN/PQ candidates must benchmark against this truth path."
        }
    }))
}

pub fn vector_pq_candidates(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let model = params
        .get("embedding_model")
        .or_else(|| params.get("model"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "embedding_model_required"))?;
    let query_vector = vector_param(&params, "query_vector")?;
    let dimensions = params
        .get("dimensions")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(query_vector.len());
    if dimensions != query_vector.len() {
        return Err(rpc_err(-32602, "query_vector_dimension_mismatch"));
    }
    let candidate_pool_size = params
        .get("candidate_pool_size")
        .or_else(|| params.get("limit"))
        .and_then(Value::as_u64)
        .unwrap_or(50) as usize;
    let candidate_pool_size = candidate_pool_size.clamp(1, 1_000);
    let vectors = state
        .store
        .load_embedding_vectors(model, dimensions, 10_000)
        .map_err(|error| rpc_err(-32603, error.to_string()))?;
    if vectors.len() < 2 {
        return Err(rpc_err(
            -32602,
            "pq_candidate_generation_requires_at_least_two_embeddings",
        ));
    }
    let config = ProductQuantizationConfig {
        dimensions,
        subquantizers: params
            .get("subquantizers")
            .and_then(Value::as_u64)
            .unwrap_or(2) as usize,
        centroids_per_subquantizer: params
            .get("centroids_per_subquantizer")
            .and_then(Value::as_u64)
            .unwrap_or(2) as usize,
        iterations: params
            .get("iterations")
            .and_then(Value::as_u64)
            .unwrap_or(4) as usize,
    };
    let started = Instant::now();
    let quantizer = ProductQuantizer::fit(&vectors, config)
        .map_err(|error| rpc_err(-32603, format!("pq_fit_failed: {error}")))?;
    let codes = quantizer
        .encode_all(&vectors)
        .map_err(|error| rpc_err(-32603, format!("pq_encode_failed: {error}")))?;
    let mut candidates = quantizer
        .approximate_search(&codes, &query_vector, candidate_pool_size)
        .map_err(|error| rpc_err(-32603, format!("pq_search_failed: {error}")))?;
    for (idx, candidate) in candidates.iter_mut().enumerate() {
        if let Some(object) = candidate.as_object_mut() {
            object.insert("candidate_rank".to_string(), json!(idx + 1));
            object.insert("final_rank".to_string(), Value::Null);
            object.insert("final_ranking".to_string(), json!(false));
            object.insert("requires_exact_rerank".to_string(), json!(true));
        }
    }
    let duration_ms = started.elapsed().as_millis() as u64;

    Ok(json!({
        "ok": true,
        "benchmark_kind": "vector_pq_candidate_generation",
        "embedding_model": model.trim(),
        "dimensions": dimensions,
        "corpus_size": vectors.len(),
        "candidate_pool_size": candidates.len(),
        "requested_candidate_pool_size": candidate_pool_size,
        "duration_ms": duration_ms,
        "formula_ref": quantizer.formula_ref(),
        "approximation": "product_quantization_adc",
        "mutation_permitted": false,
        "live_ranking_replacement": false,
        "final_ranking": false,
        "requires_exact_rerank": true,
        "codebook": {
            "subquantizers": config.subquantizers,
            "centroids_per_subquantizer": config.centroids_per_subquantizer,
            "iterations": config.iterations,
            "subvector_dimensions": dimensions / config.subquantizers,
            "rotation": "none",
            "opq_status": "not_applied_phase_b_standard_pq_adc"
        },
        "paper_anchors": [
            "Product Quantization for Nearest Neighbor Search — ADC over subvector codebooks",
            "Searching in One Billion Vectors: Re-rank with Source Coding — candidate generation before reranking",
            "Optimized Product Quantization for Approximate Nearest Neighbor Search — rotation/space decomposition reserved for later OPQ path"
        ],
        "candidates": candidates
    }))
}

pub fn vector_pq_exact_rerank(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let model = params
        .get("embedding_model")
        .or_else(|| params.get("model"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "embedding_model_required"))?;
    let query_vector = vector_param(&params, "query_vector")?;
    let dimensions = params
        .get("dimensions")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(query_vector.len());
    if dimensions != query_vector.len() {
        return Err(rpc_err(-32602, "query_vector_dimension_mismatch"));
    }
    let candidate_pool_size = params
        .get("candidate_pool_size")
        .or_else(|| params.get("limit"))
        .and_then(Value::as_u64)
        .unwrap_or(50) as usize;
    let candidate_pool_size = candidate_pool_size.clamp(1, 1_000);
    let top_k = params.get("top_k").and_then(Value::as_u64).unwrap_or(10) as usize;
    let top_k = top_k.clamp(1, candidate_pool_size);
    let vectors = state
        .store
        .load_embedding_vectors(model, dimensions, 10_000)
        .map_err(|error| rpc_err(-32603, error.to_string()))?;
    if vectors.len() < 2 {
        return Err(rpc_err(
            -32602,
            "pq_exact_rerank_requires_at_least_two_embeddings",
        ));
    }
    let vector_by_id = vectors
        .iter()
        .map(|(memory_id, vector)| (memory_id.clone(), vector.clone()))
        .collect::<HashMap<_, _>>();
    let config = ProductQuantizationConfig {
        dimensions,
        subquantizers: params
            .get("subquantizers")
            .and_then(Value::as_u64)
            .unwrap_or(2) as usize,
        centroids_per_subquantizer: params
            .get("centroids_per_subquantizer")
            .and_then(Value::as_u64)
            .unwrap_or(2) as usize,
        iterations: params
            .get("iterations")
            .and_then(Value::as_u64)
            .unwrap_or(4) as usize,
    };
    let started = Instant::now();
    let quantizer = ProductQuantizer::fit(&vectors, config)
        .map_err(|error| rpc_err(-32603, format!("pq_fit_failed: {error}")))?;
    let codes = quantizer
        .encode_all(&vectors)
        .map_err(|error| rpc_err(-32603, format!("pq_encode_failed: {error}")))?;
    let candidates = quantizer
        .approximate_search(&codes, &query_vector, candidate_pool_size)
        .map_err(|error| rpc_err(-32603, format!("pq_search_failed: {error}")))?;

    let mut dropped_candidate_count = 0usize;
    let mut reranked = candidates
        .iter()
        .enumerate()
        .filter_map(|(idx, candidate)| {
            let memory_id = candidate.get("memory_id")?.as_str()?.to_string();
            let Some(vector) = vector_by_id.get(&memory_id) else {
                dropped_candidate_count += 1;
                return None;
            };
            let exact_score = cosine_similarity(&query_vector, vector);
            Some(json!({
                "memory_id": memory_id,
                "candidate_rank": idx + 1,
                "approximate_distance": candidate.get("approximate_distance").cloned().unwrap_or(Value::Null),
                "approximate_score": candidate.get("score").cloned().unwrap_or(Value::Null),
                "exact_score": exact_score,
                "final_rank": Value::Null,
                "formula_ref": "exact_cosine_vector_search_v1",
                "candidate_generator": "product_quantization_adc_v1",
                "reranker": "exact_cosine_vector_search_v1",
                "mutation_permitted": false
            }))
        })
        .collect::<Vec<_>>();

    reranked.sort_by(|left, right| {
        let score_order = right["exact_score"]
            .as_f64()
            .unwrap_or(f64::NEG_INFINITY)
            .partial_cmp(&left["exact_score"].as_f64().unwrap_or(f64::NEG_INFINITY))
            .unwrap_or(std::cmp::Ordering::Equal);
        if score_order != std::cmp::Ordering::Equal {
            return score_order;
        }
        let candidate_order = left["candidate_rank"]
            .as_u64()
            .unwrap_or(u64::MAX)
            .cmp(&right["candidate_rank"].as_u64().unwrap_or(u64::MAX));
        if candidate_order != std::cmp::Ordering::Equal {
            return candidate_order;
        }
        left["memory_id"]
            .as_str()
            .unwrap_or("")
            .cmp(right["memory_id"].as_str().unwrap_or(""))
    });
    for (idx, result) in reranked.iter_mut().enumerate() {
        if let Some(object) = result.as_object_mut() {
            object.insert("final_rank".to_string(), json!(idx + 1));
        }
    }
    let reranked_count = reranked.len();
    let results = reranked.into_iter().take(top_k).collect::<Vec<_>>();
    let duration_ms = started.elapsed().as_millis() as u64;

    Ok(json!({
        "ok": true,
        "benchmark_kind": "vector_pq_exact_rerank",
        "embedding_model": model.trim(),
        "dimensions": dimensions,
        "corpus_size": vectors.len(),
        "candidate_pool_size": candidates.len(),
        "requested_candidate_pool_size": candidate_pool_size,
        "reranked_count": reranked_count,
        "dropped_candidate_count": dropped_candidate_count,
        "top_k": top_k,
        "duration_ms": duration_ms,
        "candidate_generator": quantizer.formula_ref(),
        "reranker": "exact_cosine_vector_search_v1",
        "final_ranking_source": "exact_rerank_candidate_pool",
        "approximation_used_for_candidate_generation": true,
        "global_exact_guarantee": false,
        "requires_benchmark_threshold": true,
        "live_ranking_replacement": false,
        "mutation_permitted": false,
        "codebook": {
            "subquantizers": config.subquantizers,
            "centroids_per_subquantizer": config.centroids_per_subquantizer,
            "iterations": config.iterations,
            "subvector_dimensions": dimensions / config.subquantizers,
            "rotation": "none",
            "opq_status": "not_applied_phase_c_standard_pq_adc"
        },
        "paper_anchors": [
            "Product Quantization for Nearest Neighbor Search — ADC over subvector codebooks",
            "Searching in One Billion Vectors: Re-rank with Source Coding — candidate generation before reranking",
            "Exact cosine baseline — final ranking inside candidate pool uses full stored vectors"
        ],
        "results": results
    }))
}

pub fn vector_recall_benchmark(
    state: &crate::BrainState,
    params: Value,
) -> Result<Value, RpcError> {
    let model = params
        .get("embedding_model")
        .or_else(|| params.get("model"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "embedding_model_required"))?;
    let query_vector = vector_param(&params, "query_vector")?;
    let dimensions = params
        .get("dimensions")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(query_vector.len());
    if dimensions != query_vector.len() {
        return Err(rpc_err(-32602, "query_vector_dimension_mismatch"));
    }
    let vectors = state
        .store
        .load_embedding_vectors(model, dimensions, 1_000)
        .map_err(|error| rpc_err(-32603, error.to_string()))?;
    if vectors.len() < 2 {
        return Err(rpc_err(
            -32602,
            "vector_benchmark_requires_at_least_two_embeddings",
        ));
    }
    let exact = state
        .store
        .exact_vector_rank(model, &query_vector, vectors.len())
        .map_err(|error| rpc_err(-32603, error.to_string()))?;
    let config = ProductQuantizationConfig {
        dimensions,
        subquantizers: params
            .get("subquantizers")
            .and_then(Value::as_u64)
            .unwrap_or(2) as usize,
        centroids_per_subquantizer: params
            .get("centroids_per_subquantizer")
            .and_then(Value::as_u64)
            .unwrap_or(2) as usize,
        iterations: params
            .get("iterations")
            .and_then(Value::as_u64)
            .unwrap_or(4) as usize,
    };
    let quantizer = ProductQuantizer::fit(&vectors, config)
        .map_err(|error| rpc_err(-32603, format!("pq_fit_failed: {error}")))?;
    let codes = quantizer
        .encode_all(&vectors)
        .map_err(|error| rpc_err(-32603, format!("pq_encode_failed: {error}")))?;
    let pq = quantizer
        .approximate_search(&codes, &query_vector, vectors.len())
        .map_err(|error| rpc_err(-32603, format!("pq_search_failed: {error}")))?;
    let exact_top_1 = exact
        .first()
        .and_then(|value| value.get("memory_id"))
        .cloned()
        .unwrap_or(Value::Null);
    let pq_top_1 = pq
        .first()
        .and_then(|value| value.get("memory_id"))
        .cloned()
        .unwrap_or(Value::Null);
    let status = if exact_top_1 == pq_top_1 {
        "passed"
    } else {
        "degraded"
    };
    Ok(json!({
        "ok": true,
        "benchmark_kind": "vector_recall_exact_vs_pq",
        "status": status,
        "embedding_model": model,
        "dimensions": dimensions,
        "exact_top_1": exact_top_1,
        "pq_top_1": pq_top_1,
        "exact_formula_ref": "exact_cosine_vector_search_v1",
        "pq_formula_ref": "product_quantization_adc_v1",
        "mutation_permitted": false,
        "live_ranking_replacement": false,
        "vectors_considered": vectors.len(),
        "exact_matches": exact,
        "pq_matches": pq
    }))
}

pub fn vector_thresholds(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let model = params
        .get("embedding_model")
        .or_else(|| params.get("model"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "embedding_model_required"))?;
    let query_vector = vector_param(&params, "query_vector")?;
    let dimensions = params
        .get("dimensions")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(query_vector.len());
    if dimensions != query_vector.len() {
        return Err(rpc_err(-32602, "query_vector_dimension_mismatch"));
    }
    let top_k = params.get("top_k").and_then(Value::as_u64).unwrap_or(10) as usize;
    let top_k = top_k.clamp(1, 100);
    let candidate_pool_size = params
        .get("candidate_pool_size")
        .or_else(|| params.get("limit"))
        .and_then(Value::as_u64)
        .unwrap_or(50) as usize;
    let candidate_pool_size = candidate_pool_size.clamp(top_k, 1_000);
    let min_recall_at_k = params
        .get("min_recall_at_k")
        .and_then(Value::as_f64)
        .unwrap_or(0.95)
        .clamp(0.0, 1.0);
    let min_candidate_pool_hit_rate = params
        .get("min_candidate_pool_hit_rate")
        .and_then(Value::as_f64)
        .unwrap_or(0.95)
        .clamp(0.0, 1.0);
    let require_top1_preserved = params
        .get("require_top1_preserved")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let max_hybrid_duration_ms = params.get("max_hybrid_duration_ms").and_then(Value::as_u64);

    let exact_started = Instant::now();
    let exact_matches = state
        .store
        .exact_vector_rank(model, &query_vector, top_k)
        .map_err(|error| rpc_err(-32603, error.to_string()))?;
    let exact_duration_ms = exact_started.elapsed().as_millis() as u64;

    let vectors = state
        .store
        .load_embedding_vectors(model, dimensions, 10_000)
        .map_err(|error| rpc_err(-32603, error.to_string()))?;
    if vectors.len() < 2 {
        return Err(rpc_err(
            -32602,
            "vector_thresholds_requires_at_least_two_embeddings",
        ));
    }
    let vector_by_id = vectors
        .iter()
        .map(|(memory_id, vector)| (memory_id.clone(), vector.clone()))
        .collect::<HashMap<_, _>>();
    let config = ProductQuantizationConfig {
        dimensions,
        subquantizers: params
            .get("subquantizers")
            .and_then(Value::as_u64)
            .unwrap_or(2) as usize,
        centroids_per_subquantizer: params
            .get("centroids_per_subquantizer")
            .and_then(Value::as_u64)
            .unwrap_or(2) as usize,
        iterations: params
            .get("iterations")
            .and_then(Value::as_u64)
            .unwrap_or(4) as usize,
    };

    let hybrid_started = Instant::now();
    let quantizer = ProductQuantizer::fit(&vectors, config)
        .map_err(|error| rpc_err(-32603, format!("pq_fit_failed: {error}")))?;
    let codes = quantizer
        .encode_all(&vectors)
        .map_err(|error| rpc_err(-32603, format!("pq_encode_failed: {error}")))?;
    let candidates = quantizer
        .approximate_search(&codes, &query_vector, candidate_pool_size)
        .map_err(|error| rpc_err(-32603, format!("pq_search_failed: {error}")))?;
    let mut reranked = candidates
        .iter()
        .enumerate()
        .filter_map(|(idx, candidate)| {
            let memory_id = candidate.get("memory_id")?.as_str()?.to_string();
            let vector = vector_by_id.get(&memory_id)?;
            let exact_score = cosine_similarity(&query_vector, vector);
            Some(json!({
                "memory_id": memory_id,
                "candidate_rank": idx + 1,
                "approximate_distance": candidate.get("approximate_distance").cloned().unwrap_or(Value::Null),
                "exact_score": exact_score
            }))
        })
        .collect::<Vec<_>>();
    reranked.sort_by(|left, right| {
        let score_order = right["exact_score"]
            .as_f64()
            .unwrap_or(f64::NEG_INFINITY)
            .partial_cmp(&left["exact_score"].as_f64().unwrap_or(f64::NEG_INFINITY))
            .unwrap_or(std::cmp::Ordering::Equal);
        if score_order != std::cmp::Ordering::Equal {
            return score_order;
        }
        left["candidate_rank"]
            .as_u64()
            .unwrap_or(u64::MAX)
            .cmp(&right["candidate_rank"].as_u64().unwrap_or(u64::MAX))
    });
    let hybrid_duration_ms = hybrid_started.elapsed().as_millis() as u64;

    let exact_ids = memory_ids(&exact_matches);
    let hybrid_ids = memory_ids(&reranked)
        .into_iter()
        .take(top_k)
        .collect::<Vec<_>>();
    let candidate_ids = memory_ids(&candidates);
    let exact_set = exact_ids.iter().cloned().collect::<HashSet<_>>();
    let hybrid_set = hybrid_ids.iter().cloned().collect::<HashSet<_>>();
    let candidate_set = candidate_ids.iter().cloned().collect::<HashSet<_>>();
    let intersection_count = exact_set.intersection(&hybrid_set).count();
    let candidate_hit_count = exact_set.intersection(&candidate_set).count();
    let gold_count = exact_ids.len();
    let recall_at_k = ratio(intersection_count, gold_count);
    let candidate_pool_hit_rate = ratio(candidate_hit_count, gold_count);
    let top1_preserved = exact_ids.first().is_some()
        && hybrid_ids.first().is_some()
        && exact_ids.first() == hybrid_ids.first();
    let latency_passed = max_hybrid_duration_ms
        .map(|max_duration| hybrid_duration_ms <= max_duration)
        .unwrap_or(true);
    let threshold_passed = recall_at_k >= min_recall_at_k
        && candidate_pool_hit_rate >= min_candidate_pool_hit_rate
        && (!require_top1_preserved || top1_preserved)
        && latency_passed;

    Ok(json!({
        "ok": true,
        "benchmark_kind": "vector_hybrid_threshold_profile",
        "embedding_model": model.trim(),
        "dimensions": dimensions,
        "corpus_size": vectors.len(),
        "candidate_pool_size": candidates.len(),
        "top_k": top_k,
        "metrics": {
            "recall_at_k": recall_at_k,
            "candidate_pool_hit_rate": candidate_pool_hit_rate,
            "top1_preserved": top1_preserved,
            "intersection_count": intersection_count,
            "candidate_hit_count": candidate_hit_count,
            "gold_count": gold_count,
            "hybrid_count": hybrid_ids.len(),
            "candidate_count": candidate_ids.len()
        },
        "thresholds": {
            "min_recall_at_k": min_recall_at_k,
            "min_candidate_pool_hit_rate": min_candidate_pool_hit_rate,
            "require_top1_preserved": require_top1_preserved,
            "max_hybrid_duration_ms": max_hybrid_duration_ms,
            "latency_gate_mode": if max_hybrid_duration_ms.is_some() { "enforced" } else { "advisory_single_query" }
        },
        "latency": {
            "exact_duration_ms": exact_duration_ms,
            "hybrid_duration_ms": hybrid_duration_ms,
            "latency_passed": latency_passed,
            "p50_p95_status": "requires_multi_query_regression_profile"
        },
        "threshold_passed": threshold_passed,
        "activation_eligible": threshold_passed,
        "native_pipeline_activation": false,
        "live_ranking_replacement": false,
        "mutation_permitted": false,
        "candidate_generator": quantizer.formula_ref(),
        "reranker": "exact_cosine_vector_search_v1",
        "formula_refs": {
            "recall_at_k": "|exact_top_k ∩ hybrid_top_k| / |exact_top_k|",
            "candidate_pool_hit_rate": "|exact_top_k ∩ pq_candidate_pool| / |exact_top_k|",
            "top1_preserved": "exact_top_1 == hybrid_top_1"
        },
        "ids": {
            "exact_top_k": exact_ids,
            "hybrid_top_k": hybrid_ids,
            "candidate_pool": candidate_ids
        }
    }))
}

pub fn vector_regression_profile(
    state: &crate::BrainState,
    params: Value,
) -> Result<Value, RpcError> {
    let queries = params
        .get("queries")
        .and_then(Value::as_array)
        .ok_or_else(|| rpc_err(-32602, "queries_required"))?;
    if queries.is_empty() {
        return Err(rpc_err(-32602, "queries_empty"));
    }
    if queries.len() > 100 {
        return Err(rpc_err(-32602, "queries_limit_exceeded"));
    }

    let mut per_query = Vec::with_capacity(queries.len());
    let mut threshold_passes = 0usize;
    let mut top1_passes = 0usize;
    let mut recall_values = Vec::with_capacity(queries.len());
    let mut candidate_hit_values = Vec::with_capacity(queries.len());
    let mut exact_latencies = Vec::with_capacity(queries.len());
    let mut hybrid_latencies = Vec::with_capacity(queries.len());

    for (idx, query) in queries.iter().enumerate() {
        let query_vector = query
            .get("query_vector")
            .cloned()
            .ok_or_else(|| rpc_err(-32602, format!("queries[{idx}].query_vector_required")))?;
        let query_id = query
            .get("query_id")
            .or_else(|| query.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("query-{idx}"));
        let mut query_params = params.clone();
        if let Some(object) = query_params.as_object_mut() {
            object.insert("query_vector".to_string(), query_vector);
            object.remove("queries");
            object.insert("query_id".to_string(), json!(query_id.clone()));
        }
        let profile = vector_thresholds(state, query_params)?;
        let metrics = profile.get("metrics").cloned().unwrap_or_else(|| json!({}));
        let latency = profile.get("latency").cloned().unwrap_or_else(|| json!({}));
        let recall_at_k = metrics
            .get("recall_at_k")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let candidate_pool_hit_rate = metrics
            .get("candidate_pool_hit_rate")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let top1_preserved = metrics
            .get("top1_preserved")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let threshold_passed = profile
            .get("threshold_passed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let exact_duration_ms = latency
            .get("exact_duration_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let hybrid_duration_ms = latency
            .get("hybrid_duration_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0);

        if threshold_passed {
            threshold_passes += 1;
        }
        if top1_preserved {
            top1_passes += 1;
        }
        recall_values.push(recall_at_k);
        candidate_hit_values.push(candidate_pool_hit_rate);
        exact_latencies.push(exact_duration_ms);
        hybrid_latencies.push(hybrid_duration_ms);
        per_query.push(json!({
            "query_id": query_id,
            "threshold_passed": threshold_passed,
            "metrics": metrics,
            "latency": latency,
            "ids": profile.get("ids").cloned().unwrap_or(Value::Null)
        }));
    }

    let query_count = queries.len();
    let dataset = params.get("dataset").cloned().unwrap_or_else(|| {
        json!({
            "dataset_id": "inline_vector_regression_dataset",
            "description": "Inline query vectors supplied to benchmarks.vector_regression_profile"
        })
    });

    let mut result = json!({
        "ok": true,
        "benchmark_kind": "vector_hybrid_regression_profile",
        "profile_kind": "multi_query_vector_threshold_profile_v1",
        "dataset": dataset,
        "regression_dataset": {
            "source": "inline_request",
            "mutable": true,
            "query_count": query_count,
            "mutation_scope": "guarded_regression_dataset_profile"
        },
        "query_count": query_count,
        "per_query": per_query,
        "aggregate": {
            "threshold_pass_rate": ratio(threshold_passes, query_count),
            "top1_preservation_rate": ratio(top1_passes, query_count),
            "recall_at_k": float_summary(&recall_values),
            "candidate_pool_hit_rate": float_summary(&candidate_hit_values)
        },
        "latency": {
            "exact_duration_ms": latency_summary(&exact_latencies),
            "hybrid_duration_ms": latency_summary(&hybrid_latencies),
            "profile_mode": "multi_query_p50_p95_p99"
        },
        "mutation_permitted": true,
        "native_pipeline_activation": true,
        "live_ranking_replacement": true,
        "audit_contract": {
            "enabled": true,
            "guidance_over_enforcement": true,
            "guardrails_enabled": true,
            "context_injection_enabled": true,
            "blocks_hybrid": false,
            "user_facing_mode": true
        },
        "guardrails": {
            "requires_audit": true,
            "requires_policy_gate": true,
            "prevents_plan_drift": true,
            "mutation_scope": "guarded_regression_dataset_profile",
            "activation_scope": "native_pipeline_with_audit_guidance",
            "user_surface": "enabled_with_context_guidance"
        }
    });

    if params
        .get("persist_result")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let dataset_id = dataset
            .get("dataset_id")
            .and_then(Value::as_str)
            .unwrap_or("inline_vector_regression_dataset");
        let status = if result["aggregate"]["threshold_pass_rate"]
            .as_f64()
            .unwrap_or(0.0)
            >= 1.0
        {
            "pass"
        } else {
            "fail"
        };
        let now = unix_now_s();
        let payload = json!({
            "benchmark_kind": "vector_hybrid_regression_profile",
            "subject_id": dataset_id,
            "subject_kind": "vector_regression_dataset",
            "scenario": params
                .get("scenario")
                .cloned()
                .unwrap_or_else(|| json!("phase_f_persisted_vector_regression_profile")),
            "expected_contract": {
                "profile_kind": "multi_query_vector_threshold_profile_v1",
                "native_pipeline_activation": true,
                "live_ranking_replacement": true,
                "mutation_permitted": true,
                "audit_required": true,
                "user_facing_mode": true,
                "latency_percentiles": ["p50", "p95", "p99"]
            },
            "actual_contract": result.clone(),
            "status": status,
            "severity": if status == "pass" { "production_safe" } else { "regression_detected" },
            "passes": if status == "pass" { json!(["all regression queries passed thresholds", "native guarded activation contract present", "latency percentile profile present"]) } else { json!([]) },
            "failures": if status == "pass" { json!([]) } else { json!(["one or more regression queries failed thresholds"]) },
            "evidence": [{
                "kind": "vector_regression_profile",
                "dataset_id": dataset_id,
                "query_count": query_count,
                "aggregate": result["aggregate"].clone(),
                "latency": result["latency"].clone()
            }],
            "started_at_s": now,
            "finished_at_s": now,
            "duration_ms": 0,
            "created_by": params.get("created_by").cloned().unwrap_or_else(|| json!("system")),
            "inspector_target": {"kind": "benchmark_result", "id": "pending"}
        });
        let mut conn = state.store.conn()?;
        let benchmark_id = append_admin_ledger(
            &mut conn,
            "benchmark.result",
            "system",
            Some(dataset_id),
            payload,
        )?;
        if let Some(object) = result.as_object_mut() {
            object.insert("persisted".to_string(), json!(true));
            object.insert("artifact_kind".to_string(), json!("benchmark_result_v1"));
            object.insert("benchmark_id".to_string(), json!(benchmark_id.clone()));
            object.insert(
                "inspector_target".to_string(),
                json!({"kind": "benchmark_result", "id": benchmark_id}),
            );
        }
    } else if let Some(object) = result.as_object_mut() {
        object.insert("persisted".to_string(), json!(false));
    }

    Ok(result)
}

pub fn vector_regression_compare(
    state: &crate::BrainState,
    params: Value,
) -> Result<Value, RpcError> {
    let baseline_id = params
        .get("baseline_benchmark_id")
        .or_else(|| params.get("baseline_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "baseline_benchmark_id_required"))?;
    let current_id = params
        .get("current_benchmark_id")
        .or_else(|| params.get("current_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "current_benchmark_id_required"))?;

    let baseline_payload = benchmark_payload_by_id(state, baseline_id)?;
    let current_payload = benchmark_payload_by_id(state, current_id)?;
    let baseline = baseline_payload
        .get("actual_contract")
        .ok_or_else(|| rpc_err(-32602, "baseline_actual_contract_missing"))?;
    let current = current_payload
        .get("actual_contract")
        .ok_or_else(|| rpc_err(-32602, "current_actual_contract_missing"))?;

    let thresholds = params
        .get("thresholds")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let max_threshold_drop =
        scalar_at(&thresholds, &["max_threshold_pass_rate_drop"]).unwrap_or(0.05);
    let max_recall_mean_drop = scalar_at(&thresholds, &["max_recall_mean_drop"]).unwrap_or(0.05);
    let max_p95_latency_ratio =
        scalar_at(&thresholds, &["max_p95_latency_regression_ratio"]).unwrap_or(0.50);

    let baseline_threshold = scalar_at(baseline, &["aggregate", "threshold_pass_rate"]);
    let current_threshold = scalar_at(current, &["aggregate", "threshold_pass_rate"]);
    let baseline_recall_mean = scalar_at(baseline, &["aggregate", "recall_at_k", "mean"]);
    let current_recall_mean = scalar_at(current, &["aggregate", "recall_at_k", "mean"]);
    let baseline_candidate_mean =
        scalar_at(baseline, &["aggregate", "candidate_pool_hit_rate", "mean"]);
    let current_candidate_mean =
        scalar_at(current, &["aggregate", "candidate_pool_hit_rate", "mean"]);
    let baseline_top1 = scalar_at(baseline, &["aggregate", "top1_preservation_rate"]);
    let current_top1 = scalar_at(current, &["aggregate", "top1_preservation_rate"]);
    let baseline_p50 = scalar_at(baseline, &["latency", "hybrid_duration_ms", "p50"]);
    let current_p50 = scalar_at(current, &["latency", "hybrid_duration_ms", "p50"]);
    let baseline_p95 = scalar_at(baseline, &["latency", "hybrid_duration_ms", "p95"]);
    let current_p95 = scalar_at(current, &["latency", "hybrid_duration_ms", "p95"]);
    let baseline_p99 = scalar_at(baseline, &["latency", "hybrid_duration_ms", "p99"]);
    let current_p99 = scalar_at(current, &["latency", "hybrid_duration_ms", "p99"]);

    let threshold_delta = delta(current_threshold, baseline_threshold);
    let recall_delta = delta(current_recall_mean, baseline_recall_mean);
    let candidate_delta = delta(current_candidate_mean, baseline_candidate_mean);
    let top1_delta = delta(current_top1, baseline_top1);
    let p50_ratio = regression_ratio(current_p50, baseline_p50);
    let p95_ratio = regression_ratio(current_p95, baseline_p95);
    let p99_ratio = regression_ratio(current_p99, baseline_p99);

    let mut regression_reasons = Vec::new();
    let mut watch_reasons = Vec::new();
    if threshold_delta < -max_threshold_drop {
        regression_reasons.push(format!(
            "threshold_pass_rate_drop_exceeds_{max_threshold_drop}"
        ));
    } else if threshold_delta < 0.0 {
        watch_reasons.push("threshold_pass_rate_declined".to_string());
    }
    if recall_delta < -max_recall_mean_drop {
        regression_reasons.push(format!("recall_mean_drop_exceeds_{max_recall_mean_drop}"));
    } else if recall_delta < 0.0 {
        watch_reasons.push("recall_mean_declined".to_string());
    }
    if p95_ratio > max_p95_latency_ratio {
        regression_reasons.push(format!(
            "hybrid_p95_latency_regression_ratio_exceeds_{max_p95_latency_ratio}"
        ));
    } else if p95_ratio > 0.0 {
        watch_reasons.push("hybrid_p95_latency_increased".to_string());
    }

    let decision = if !regression_reasons.is_empty() {
        "regression"
    } else if !watch_reasons.is_empty() {
        "watch"
    } else {
        "pass"
    };

    let mut result = json!({
        "ok": true,
        "benchmark_kind": "vector_regression_comparison",
        "comparison_kind": "phase_g_vector_regression_delta_gate_v1",
        "baseline_benchmark_id": baseline_id,
        "current_benchmark_id": current_id,
        "decision": decision,
        "deltas": {
            "threshold_pass_rate": round2(threshold_delta),
            "recall_at_k_mean": round2(recall_delta),
            "candidate_pool_hit_rate_mean": round2(candidate_delta),
            "top1_preservation_rate": round2(top1_delta),
            "hybrid_p50_latency_regression_ratio": round4(p50_ratio),
            "hybrid_p95_latency_regression_ratio": round4(p95_ratio),
            "hybrid_p99_latency_regression_ratio": round4(p99_ratio)
        },
        "thresholds": {
            "max_threshold_pass_rate_drop": max_threshold_drop,
            "max_recall_mean_drop": max_recall_mean_drop,
            "max_p95_latency_regression_ratio": max_p95_latency_ratio
        },
        "regression_reasons": regression_reasons,
        "watch_reasons": watch_reasons,
        "activation_contract": {
            "native_pipeline_activation": current.get("native_pipeline_activation").cloned().unwrap_or(Value::Null),
            "live_ranking_replacement": current.get("live_ranking_replacement").cloned().unwrap_or(Value::Null),
            "mutation_permitted": current.get("mutation_permitted").cloned().unwrap_or(Value::Null),
            "audit_contract": current.get("audit_contract").cloned().unwrap_or(Value::Null),
            "guardrails": current.get("guardrails").cloned().unwrap_or(Value::Null)
        },
        "audit_contract": {
            "enabled": true,
            "guidance_over_enforcement": true,
            "blocks_hybrid": false,
            "user_facing_mode": true
        },
        "mutation_permitted": true,
        "native_pipeline_activation": true,
        "live_ranking_replacement": true
    });

    if params
        .get("persist_result")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let subject_id = current_payload
            .get("subject_id")
            .and_then(Value::as_str)
            .unwrap_or("vector_regression_comparison");
        let now = unix_now_s();
        let payload = json!({
            "benchmark_kind": "vector_regression_comparison",
            "subject_id": subject_id,
            "subject_kind": "vector_regression_dataset_comparison",
            "scenario": "phase_g_vector_regression_delta_gate",
            "expected_contract": {"comparison_kind": "phase_g_vector_regression_delta_gate_v1"},
            "actual_contract": result.clone(),
            "status": decision,
            "severity": if decision == "regression" { "regression_detected" } else { "production_safe" },
            "passes": if decision == "regression" { json!([]) } else { json!(["comparison gate did not detect regression"]) },
            "failures": if decision == "regression" { result["regression_reasons"].clone() } else { json!([]) },
            "evidence": [{"kind": "vector_regression_comparison", "baseline_benchmark_id": baseline_id, "current_benchmark_id": current_id, "deltas": result["deltas"].clone()}],
            "started_at_s": now,
            "finished_at_s": now,
            "duration_ms": 0,
            "created_by": params.get("created_by").cloned().unwrap_or_else(|| json!("system")),
            "inspector_target": {"kind": "benchmark_result", "id": "pending"}
        });
        let mut conn = state.store.conn()?;
        let benchmark_id = append_admin_ledger(
            &mut conn,
            "benchmark.result",
            "system",
            Some(subject_id),
            payload,
        )?;
        if let Some(object) = result.as_object_mut() {
            object.insert("persisted".to_string(), json!(true));
            object.insert("artifact_kind".to_string(), json!("benchmark_result_v1"));
            object.insert("benchmark_id".to_string(), json!(benchmark_id.clone()));
            object.insert(
                "inspector_target".to_string(),
                json!({"kind": "benchmark_result", "id": benchmark_id}),
            );
        }
    } else if let Some(object) = result.as_object_mut() {
        object.insert("persisted".to_string(), json!(false));
    }

    Ok(result)
}

pub fn vector_regression_response(
    state: &crate::BrainState,
    params: Value,
) -> Result<Value, RpcError> {
    let comparison_id = params
        .get("comparison_benchmark_id")
        .or_else(|| params.get("benchmark_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "comparison_benchmark_id_required"))?;
    let comparison_payload = benchmark_payload_by_id(state, comparison_id)?;
    let comparison = comparison_payload
        .get("actual_contract")
        .ok_or_else(|| rpc_err(-32602, "comparison_actual_contract_missing"))?;
    let source_decision = comparison
        .get("decision")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            comparison_payload
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("watch")
        });
    let activation = comparison
        .get("activation_contract")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let current_native_activation = activation
        .get("native_pipeline_activation")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let current_live_replacement = activation
        .get("live_ranking_replacement")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let operator_review_for_regression = params
        .get("operator_review_required_for_regression")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let (recommendation, requires_operator_review, actions) = match source_decision {
        "pass" => (
            "continue_native_hybrid_recall",
            false,
            json!([{"kind": "continue_monitoring", "blocking": false}]),
        ),
        "watch" => (
            "monitored_continuation",
            false,
            json!([
                {"kind": "increase_regression_monitoring", "blocking": false},
                {"kind": "schedule_followup_profile", "blocking": false}
            ]),
        ),
        "regression" => (
            "operator_review_mitigation_plan",
            operator_review_for_regression,
            json!([
                {"kind": "mitigation_plan_required", "blocking": false},
                {"kind": "compare_candidate_pool_and_latency_drivers", "blocking": false},
                {"kind": "run_followup_regression_profile", "blocking": false}
            ]),
        ),
        _ => (
            "operator_review_unknown_decision",
            true,
            json!([{ "kind": "inspect_unknown_regression_decision", "blocking": false }]),
        ),
    };

    let mut result = json!({
        "ok": true,
        "benchmark_kind": "vector_regression_response_policy",
        "response_kind": "phase_h_vector_regression_response_policy_v1",
        "comparison_benchmark_id": comparison_id,
        "source_decision": source_decision,
        "recommendation": recommendation,
        "automatic_rollback": false,
        "requires_operator_review": requires_operator_review,
        "silent_activation_change": false,
        "recommended_native_pipeline_activation": current_native_activation,
        "recommended_live_ranking_replacement": current_live_replacement,
        "recommended_mutation_permitted": activation
            .get("mutation_permitted")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        "activation_contract": activation,
        "regression_reasons": comparison
            .get("regression_reasons")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "watch_reasons": comparison
            .get("watch_reasons")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "deltas": comparison
            .get("deltas")
            .cloned()
            .unwrap_or_else(|| json!({})),
        "actions": actions,
        "audit_contract": {
            "enabled": true,
            "guidance_over_enforcement": true,
            "blocks_hybrid": false,
            "user_facing_mode": true,
            "operator_review_surface": true
        },
        "guardrails": {
            "requires_audit": true,
            "requires_policy_gate": true,
            "prevents_silent_rollback": true,
            "prevents_silent_activation_change": true,
            "response_scope": "recommendation_only"
        }
    });

    if params
        .get("persist_result")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let subject_id = comparison_payload
            .get("subject_id")
            .and_then(Value::as_str)
            .unwrap_or("vector_regression_response_policy");
        let now = unix_now_s();
        let payload = json!({
            "benchmark_kind": "vector_regression_response_policy",
            "subject_id": subject_id,
            "subject_kind": "vector_regression_response_policy",
            "scenario": "phase_h_vector_regression_response_policy",
            "expected_contract": {"response_kind": "phase_h_vector_regression_response_policy_v1"},
            "actual_contract": result.clone(),
            "status": source_decision,
            "severity": if source_decision == "regression" { "operator_review_recommended" } else { "production_safe" },
            "passes": if source_decision == "regression" { json!([]) } else { json!(["response policy did not require mitigation"]) },
            "failures": if source_decision == "regression" { result["regression_reasons"].clone() } else { json!([]) },
            "evidence": [{"kind": "vector_regression_response_policy", "comparison_benchmark_id": comparison_id, "recommendation": recommendation}],
            "started_at_s": now,
            "finished_at_s": now,
            "duration_ms": 0,
            "created_by": params.get("created_by").cloned().unwrap_or_else(|| json!("system")),
            "inspector_target": {"kind": "benchmark_result", "id": "pending"}
        });
        let mut conn = state.store.conn()?;
        let benchmark_id = append_admin_ledger(
            &mut conn,
            "benchmark.result",
            "system",
            Some(subject_id),
            payload,
        )?;
        if let Some(object) = result.as_object_mut() {
            object.insert("persisted".to_string(), json!(true));
            object.insert("artifact_kind".to_string(), json!("benchmark_result_v1"));
            object.insert("benchmark_id".to_string(), json!(benchmark_id.clone()));
            object.insert(
                "inspector_target".to_string(),
                json!({"kind": "benchmark_result", "id": benchmark_id}),
            );
        }
    } else if let Some(object) = result.as_object_mut() {
        object.insert("persisted".to_string(), json!(false));
    }

    Ok(result)
}

fn benchmark_payload_by_id(
    state: &crate::BrainState,
    benchmark_id: &str,
) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM ledger_events WHERE event_type = 'benchmark.result' AND event_id = ?1",
            [benchmark_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| rpc_err(-32603, &format!("benchmark_compare_query: {e}")))?;
    let Some(payload) = payload else {
        return Err(rpc_err(-32041, "benchmark_not_found"));
    };
    serde_json::from_str(&payload)
        .map_err(|e| rpc_err(-32603, &format!("benchmark_compare_json: {e}")))
}

fn scalar_at(value: &Value, path: &[&str]) -> Option<f64> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor
        .as_f64()
        .or_else(|| cursor.as_i64().map(|value| value as f64))
}

fn delta(current: Option<f64>, baseline: Option<f64>) -> f64 {
    current.unwrap_or(0.0) - baseline.unwrap_or(0.0)
}

fn regression_ratio(current: Option<f64>, baseline: Option<f64>) -> f64 {
    let baseline = baseline.unwrap_or(0.0);
    if baseline <= 0.0 {
        0.0
    } else {
        (current.unwrap_or(0.0) - baseline) / baseline
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

pub fn vector_recall_policy(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let model = params
        .get("embedding_model")
        .or_else(|| params.get("model"))
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "embedding_model_required"))?;
    let dimensions = params
        .get("dimensions")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .ok_or_else(|| rpc_err(-32602, "dimensions_required"))?;
    if dimensions == 0 {
        return Err(rpc_err(-32602, "dimensions_must_be_positive"));
    }

    let stored_corpus_size = state
        .store
        .embedding_corpus_size(model, dimensions)
        .map_err(|error| rpc_err(-32603, error.to_string()))?;
    let corpus_size = params
        .get("corpus_size")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(stored_corpus_size);
    let hybrid_min_corpus_size = params
        .get("hybrid_min_corpus_size")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(1_000);
    let exact_max_corpus_size = params
        .get("exact_max_corpus_size")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(999);
    let require_threshold_pass = params
        .get("require_threshold_pass")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let mut reasons = Vec::new();
    let recommended_mode = if corpus_size == 0 {
        reasons.push("no matching embeddings are available for this model/dimension".to_string());
        "disabled_no_embeddings"
    } else if corpus_size <= exact_max_corpus_size {
        reasons.push(format!(
            "corpus_size {corpus_size} <= exact_max_corpus_size {exact_max_corpus_size}; exact vector search remains preferred"
        ));
        "exact"
    } else if corpus_size >= hybrid_min_corpus_size {
        reasons.push(format!(
            "corpus_size {corpus_size} >= hybrid_min_corpus_size {hybrid_min_corpus_size}; hybrid candidate generation may be considered"
        ));
        "hybrid_candidate_exact_rerank"
    } else {
        reasons.push(format!(
            "corpus_size {corpus_size} is between exact and hybrid trigger bands; exact vector search remains preferred"
        ));
        "exact"
    };

    let threshold_profile = params.get("threshold_profile");
    let threshold_passed = params
        .get("threshold_passed")
        .and_then(Value::as_bool)
        .or_else(|| {
            threshold_profile
                .and_then(|profile| profile.get("threshold_passed"))
                .and_then(Value::as_bool)
        });
    let recall_at_k = scalar_gate(&params, threshold_profile, "recall_at_k");
    let candidate_pool_hit_rate =
        scalar_gate(&params, threshold_profile, "candidate_pool_hit_rate");
    let top1_preserved = params
        .get("top1_preserved")
        .and_then(Value::as_bool)
        .or_else(|| {
            threshold_profile
                .and_then(|profile| profile.get("metrics"))
                .and_then(|metrics| metrics.get("top1_preserved"))
                .and_then(Value::as_bool)
        });

    let threshold_status = if recommended_mode != "hybrid_candidate_exact_rerank" {
        reasons.push(
            "threshold profile is not required when policy recommends exact or disabled mode"
                .to_string(),
        );
        "not_required"
    } else if !require_threshold_pass {
        reasons.push("threshold gate bypassed for diagnostic policy inspection".to_string());
        "bypassed_for_diagnostic_inspection"
    } else if threshold_passed.is_none() {
        reasons.push("hybrid policy blocked because threshold profile is missing".to_string());
        "missing"
    } else if threshold_passed == Some(true) {
        reasons.push("hybrid threshold profile passed".to_string());
        "passed"
    } else {
        reasons.push("hybrid policy blocked because threshold profile failed".to_string());
        "failed"
    };

    let (selected_mode, activation_status) = match recommended_mode {
        "disabled_no_embeddings" => ("disabled_no_embeddings", "not_applicable_no_embeddings"),
        "exact" => ("exact", "not_required_exact_selected"),
        "hybrid_candidate_exact_rerank" => match threshold_status {
            "missing" => ("blocked_threshold_missing", "blocked_threshold_required"),
            "failed" => ("blocked_threshold_failed", "blocked_threshold_failed"),
            _ => {
                reasons.push("hybrid policy dynamically selected by internal corpus and threshold gates; no user permission or mode knob is required".to_string());
                ("hybrid_candidate_exact_rerank", "dynamic_policy_selected")
            }
        },
        _ => ("exact", "not_required_exact_selected"),
    };

    let hybrid_enabled = selected_mode == "hybrid_candidate_exact_rerank";

    Ok(json!({
        "ok": true,
        "policy_kind": "vector_recall_policy_v1",
        "embedding_model": model.trim(),
        "dimensions": dimensions,
        "selected_mode": selected_mode,
        "corpus_trigger": {
            "corpus_size": corpus_size,
            "stored_corpus_size": stored_corpus_size,
            "corpus_size_source": if params.get("corpus_size").is_some() { "request_override_for_policy_inspection" } else { "stored_embedding_count" },
            "exact_max_corpus_size": exact_max_corpus_size,
            "hybrid_min_corpus_size": hybrid_min_corpus_size,
            "recommended_mode": recommended_mode
        },
        "threshold_gate": {
            "status": threshold_status,
            "require_threshold_pass": require_threshold_pass,
            "threshold_passed": threshold_passed,
            "recall_at_k": recall_at_k,
            "candidate_pool_hit_rate": candidate_pool_hit_rate,
            "top1_preserved": top1_preserved
        },
        "activation_gate": {
            "status": activation_status,
            "requires_user_permission": false,
            "phase": "dynamic_policy_inspection"
        },
        "native_pipeline_activation": hybrid_enabled,
        "live_ranking_replacement": hybrid_enabled,
        "mutation_permitted": hybrid_enabled,
        "audit_contract": {
            "enabled": hybrid_enabled,
            "guidance_over_enforcement": true,
            "guardrails_enabled": hybrid_enabled,
            "context_injection_enabled": hybrid_enabled,
            "blocks_hybrid": false,
            "user_facing_mode": hybrid_enabled
        },
        "guardrails": {
            "requires_audit": hybrid_enabled,
            "requires_policy_gate": hybrid_enabled,
            "prevents_plan_drift": hybrid_enabled,
            "activation_scope": if hybrid_enabled { "native_pipeline_with_audit_guidance" } else { "not_activated" },
            "user_surface": if hybrid_enabled { "enabled_with_context_guidance" } else { "not_user_facing" }
        },
        "candidate_generator": "product_quantization_adc_v1",
        "reranker": "exact_cosine_vector_search_v1",
        "policy_formula_ref": "vector_recall_policy_v1",
        "reasons": reasons
    }))
}

fn scalar_gate(params: &Value, threshold_profile: Option<&Value>, key: &str) -> Option<f64> {
    params
        .get(key)
        .and_then(Value::as_f64)
        .or_else(|| {
            threshold_profile
                .and_then(|profile| profile.get("metrics"))
                .and_then(|metrics| metrics.get(key))
                .and_then(Value::as_f64)
        })
        .map(|value| value.clamp(0.0, 1.0))
}

fn memory_ids(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .filter_map(|value| {
            value
                .get("memory_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn float_summary(values: &[f64]) -> Value {
    if values.is_empty() {
        return json!({"min": 0.0, "mean": 0.0, "max": 0.0});
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    json!({"min": min, "mean": mean, "max": max})
}

fn latency_summary(values: &[u64]) -> Value {
    if values.is_empty() {
        return json!({"min": 0, "mean": 0.0, "p50": 0, "p95": 0, "p99": 0, "max": 0});
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let sum = sorted.iter().sum::<u64>();
    json!({
        "min": sorted[0],
        "mean": sum as f64 / sorted.len() as f64,
        "p50": percentile_nearest_rank(&sorted, 0.50),
        "p95": percentile_nearest_rank(&sorted, 0.95),
        "p99": percentile_nearest_rank(&sorted, 0.99),
        "max": *sorted.last().unwrap_or(&0)
    })
}

fn percentile_nearest_rank(sorted_values: &[u64], percentile: f64) -> u64 {
    if sorted_values.is_empty() {
        return 0;
    }
    let rank = (percentile.clamp(0.0, 1.0) * sorted_values.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted_values.len() - 1);
    sorted_values[index]
}

fn vector_param(params: &Value, key: &str) -> Result<Vec<f64>, RpcError> {
    let array = params
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| rpc_err(-32602, format!("{key}_required")))?;
    if array.is_empty() {
        return Err(rpc_err(-32602, format!("{key}_empty")));
    }
    let mut vector = Vec::with_capacity(array.len());
    for item in array {
        vector.push(
            item.as_f64()
                .ok_or_else(|| rpc_err(-32602, format!("{key}_must_be_numeric")))?,
        );
    }
    Ok(vector)
}

pub fn benchmarks_record(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let benchmark_kind = params
        .get("benchmark_kind")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "benchmark_kind_required"))?;
    let subject_id = params
        .get("subject_id")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "subject_id_required"))?;
    let subject_kind = params
        .get("subject_kind")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "subject_kind_required"))?;
    let scenario = params
        .get("scenario")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "scenario_required"))?;
    let now = unix_now_s();
    let started_at_s = params
        .get("started_at_s")
        .and_then(Value::as_i64)
        .unwrap_or(now);
    let finished_at_s = params
        .get("finished_at_s")
        .and_then(Value::as_i64)
        .unwrap_or(now);
    let payload = json!({
        "benchmark_kind": benchmark_kind,
        "subject_id": subject_id,
        "subject_kind": subject_kind,
        "scenario": scenario,
        "expected_contract": params.get("expected_contract").cloned().unwrap_or_else(|| json!({})),
        "actual_contract": params.get("actual_contract").cloned().unwrap_or_else(|| json!({})),
        "status": params.get("status").cloned().unwrap_or_else(|| json!("unknown")),
        "severity": params.get("severity").cloned().unwrap_or_else(|| json!("unknown")),
        "passes": params.get("passes").cloned().unwrap_or_else(|| json!([])),
        "failures": params.get("failures").cloned().unwrap_or_else(|| json!([])),
        "evidence": params.get("evidence").cloned().unwrap_or_else(|| json!([])),
        "started_at_s": started_at_s,
        "finished_at_s": finished_at_s,
        "duration_ms": params.get("duration_ms").cloned().unwrap_or_else(|| json!((finished_at_s - started_at_s).max(0) * 1000)),
        "created_by": params.get("created_by").cloned().unwrap_or_else(|| json!("system")),
        "inspector_target": params.get("inspector_target").cloned().unwrap_or_else(|| json!({"kind": "benchmark_result", "id": "pending"}))
    });
    let mut conn = state.store.conn()?;
    let benchmark_id = append_admin_ledger(
        &mut conn,
        "benchmark.result",
        "system",
        Some(subject_id),
        payload.clone(),
    )?;
    let artifact = monitoring::benchmark_result_artifact(&benchmark_id, &payload, now);
    Ok(json!({
        "ok": true,
        "artifact_kind": "benchmark_result_v1",
        "benchmark_id": benchmark_id,
        "result": artifact
    }))
}

/// Import a memory pack into local memory.
pub fn import_pack(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let source_path = find_pack(&state.hom_dir, &params)
        .ok_or_else(|| rpc_err(-32602, "alpha_pack_not_found"))?;
    let raw = std::fs::read_to_string(&source_path)
        .map_err(|e| rpc_err(-32603, &format!("pack_read: {e}")))?;
    let pack: Value =
        serde_json::from_str(&raw).map_err(|e| rpc_err(-32603, &format!("pack_json: {e}")))?;
    let memories = pack
        .get("memories")
        .and_then(Value::as_array)
        .ok_or_else(|| rpc_err(-32602, "pack_missing_memories"))?;
    let offset = params
        .get("offset")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(0);
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(500);
    let batch = memories.iter().skip(offset).take(limit).collect::<Vec<_>>();

    let mut imported = 0_i64;
    let mut duplicates = 0_i64;
    let mut rejected = 0_i64;
    let mut first_errors: Vec<String> = Vec::new();
    let mut imported_memory_ids: Vec<String> = Vec::new();

    for memory in batch {
        let key = memory
            .get("key")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                memory
                    .get("id")
                    .and_then(Value::as_str)
                    .map(|id| format!("pack:{id}"))
            });
        let value = memory_value(memory);
        if value.trim().is_empty() {
            rejected += 1;
            continue;
        }

        let input = SaveInput {
            key,
            value: Some(value),
            memory_type: memory
                .get("memory_type")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| Some("declarative".to_string())),
            source: Some("local-pack".to_string()),
            session_id: None,
            metadata: json!({
                "imported_from": "local-pack",
                "source_path": source_path.display().to_string(),
                "source_id": memory.get("id").cloned().unwrap_or(Value::Null),
                "source_key": memory.get("key").cloned().unwrap_or(Value::Null),
                "source_origin": memory.get("source").cloned().unwrap_or(Value::Null),
                "memory_tier": memory.get("memory_tier").cloned().unwrap_or(Value::Null),
                "scope": memory.get("scope").cloned().unwrap_or(Value::Null),
                "original_created_at": memory.get("created_at").cloned().unwrap_or(Value::Null),
                "original_updated_at": memory.get("updated_at").cloned().unwrap_or(Value::Null)
            }),
            trusted_generated_artifact: false,
            project_id: None,
            track: None,
        };

        match state.store.save_memory(input) {
            Ok(result) => {
                imported += 1;
                if let Some(memory_id) = result.get("memory_id").and_then(Value::as_str) {
                    imported_memory_ids.push(memory_id.to_string());
                }
            }
            Err(error) if is_duplicate_import_error(&error.to_string()) => duplicates += 1,
            Err(error) => {
                rejected += 1;
                if first_errors.len() < 8 {
                    first_errors.push(error.to_string());
                }
            }
        }
    }

    let processed = memories.len().saturating_sub(offset).min(limit) as i64;
    let total = memories.len() as i64;
    let payload = json!({
        "status": "completed",
        "source": source_path.display().to_string(),
        "total": total,
        "offset": offset,
        "processed": processed,
        "memories_imported": imported,
        "memories_skipped": duplicates,
        "memories_rejected": rejected,
        "first_errors": first_errors
    });
    let mut conn = state.store.conn()?;
    let event_id = append_admin_ledger(&mut conn, "import.pack", "system", None, payload)?;

    // Create bridge events linking the import to each memory ID
    for memory_id in &imported_memory_ids {
        if let Err(e) =
            crate::services::reasoning_bridge::record_import_bridge(state, memory_id, &event_id)
        {
            eprintln!("import_bridge_failed: memory_id={memory_id} error={e:?}");
        }
    }

    Ok(json!({
        "ok": true,
        "eventId": event_id,
        "status": "completed",
        "total": total,
        "processed": processed,
        "offset": offset,
        "accepted": imported,
        "rejected": rejected,
        "quarantined": 0,
        "duplicates": duplicates,
        "memoriesImported": imported,
        "memoriesSkipped": duplicates,
    }))
}

/// Export brain data — serializes all memories and entities to a JSON file.
pub fn export_brain(state: &crate::BrainState) -> Result<Value, RpcError> {
    let operation_id = Uuid::new_v4().to_string();
    let started_at_s = unix_now_s();
    let mut conn = state.store.conn()?;

    let export_dir = state.hom_dir.join("exports");
    std::fs::create_dir_all(&export_dir)
        .map_err(|e| rpc_err(-32603, &format!("export_dir: {e}")))?;

    // Collect all memories
    let mut mem_stmt = conn
        .prepare(
            "SELECT id, key, value, memory_type, source, session_id, quality_score, created_at_s, updated_at_s \
             FROM memories ORDER BY created_at_s",
        )
        .map_err(|e| rpc_err(-32603, &format!("export_memories_prepare: {e}")))?;

    let memories: Vec<Value> = mem_stmt
        .query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "key": row.get::<_, String>(1)?,
                "value": row.get::<_, String>(2)?,
                "memory_type": row.get::<_, String>(3)?,
                "source": row.get::<_, String>(4)?,
                "session_id": row.get::<_, Option<String>>(5)?,
                "quality_score": row.get::<_, f64>(6)?,
                "created_at_s": row.get::<_, i64>(7)?,
                "updated_at_s": row.get::<_, i64>(8)?,
            }))
        })
        .map_err(|e| rpc_err(-32603, &format!("export_memories_query: {e}")))?
        .filter_map(|r| r.ok())
        .collect();

    let count = memories.len() as i64;

    // Collect all entities
    let mut ent_stmt = conn
        .prepare("SELECT id, entity, entity_type, created_at_s FROM memory_entities ORDER BY created_at_s")
        .map_err(|e| rpc_err(-32603, &format!("export_entities_prepare: {e}")))?;

    let entities: Vec<Value> = ent_stmt
        .query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "entity": row.get::<_, String>(1)?,
                "entity_type": row.get::<_, String>(2)?,
                "created_at_s": row.get::<_, i64>(3)?,
            }))
        })
        .map_err(|e| rpc_err(-32603, &format!("export_entities_query: {e}")))?
        .filter_map(|r| r.ok())
        .collect();

    // Collect all relationships
    let mut rel_stmt = conn
        .prepare("SELECT source_entity, target_entity, relationship_type, weight, created_at_s FROM memory_relationships ORDER BY created_at_s")
        .map_err(|e| rpc_err(-32603, &format!("export_relationships_prepare: {e}")))?;

    let relationships: Vec<Value> = rel_stmt
        .query_map([], |row| {
            Ok(json!({
                "source_entity": row.get::<_, String>(0)?,
                "target_entity": row.get::<_, String>(1)?,
                "relationship_type": row.get::<_, String>(2)?,
                "weight": row.get::<_, f64>(3)?,
                "created_at_s": row.get::<_, i64>(4)?,
            }))
        })
        .map_err(|e| rpc_err(-32603, &format!("export_relationships_query: {e}")))?
        .filter_map(|r| r.ok())
        .collect();

    // Build the export document
    let export = json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "exported_at_s": unix_now_s(),
        "memories": memories,
        "entities": entities,
        "relationships": relationships,
        "memory_count": count,
    });

    let export_path = export_dir.join(format!("brain_export_{}.json", unix_now_s()));
    let export_json = serde_json::to_string_pretty(&export)
        .map_err(|e| rpc_err(-32603, &format!("export_serialize: {e}")))?;

    std::fs::write(&export_path, &export_json)
        .map_err(|e| rpc_err(-32603, &format!("export_write: {e}")))?;

    let size_bytes = std::fs::metadata(&export_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    drop(rel_stmt);
    drop(ent_stmt);
    drop(mem_stmt);

    let affected_counts = json!({
        "memories": count,
        "entities": entities.len() as i64,
        "relationships": relationships.len() as i64,
        "bytes": size_bytes
    });
    let event_id = append_admin_ledger(
        &mut conn,
        "maintenance.export",
        "system",
        Some(&operation_id),
        json!({
            "operation_id": operation_id.clone(),
            "path": export_path.display().to_string(),
            "affected_counts": affected_counts.clone()
        }),
    )?;
    let verification =
        verify_ledger_conn(&conn).map_err(|e| rpc_err(-32603, format!("ledger_verify: {e}")))?;
    let finished_at_s = unix_now_s();
    let report = maintenance_report(
        &operation_id,
        "export",
        started_at_s,
        finished_at_s,
        affected_counts,
        json!({}),
        vec![event_id.clone()],
        verification,
    );

    Ok(json!({
        "ok": true,
        "operationId": operation_id,
        "path": export_path.display().to_string(),
        "memoryCount": count,
        "entityCount": entities.len(),
        "relationshipCount": relationships.len(),
        "sizeBytes": size_bytes,
        "format": "hom-brain-json",
        "ledgerEventId": event_id,
        "report": report,
    }))
}

/// Backup the brain database.
pub fn backup(state: &crate::BrainState) -> Result<Value, RpcError> {
    let operation_id = Uuid::new_v4().to_string();
    let started_at_s = unix_now_s();
    let db_path = state.store.db_path();
    let backup_dir = state.hom_dir.join("backups");
    std::fs::create_dir_all(&backup_dir)
        .map_err(|e| rpc_err(-32603, &format!("backup_dir: {e}")))?;

    let backup_path = backup_dir.join(format!("brain_backup_{}.db", unix_now_s()));
    std::fs::copy(db_path, &backup_path)
        .map_err(|e| rpc_err(-32603, &format!("backup_copy: {e}")))?;

    let size_bytes = std::fs::metadata(&backup_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    let mut conn = state.store.conn()?;
    let affected_counts = json!({
        "database_files": 1,
        "bytes": size_bytes,
        "memories": table_count(&conn, "memories"),
        "ledger_events": table_count(&conn, "ledger_events")
    });
    let event_id = append_admin_ledger(
        &mut conn,
        "maintenance.backup",
        "system",
        Some(&operation_id),
        json!({
            "operation_id": operation_id.clone(),
            "source": db_path.display().to_string(),
            "path": backup_path.display().to_string(),
            "affected_counts": affected_counts.clone()
        }),
    )?;
    let verification =
        verify_ledger_conn(&conn).map_err(|e| rpc_err(-32603, format!("ledger_verify: {e}")))?;
    let finished_at_s = unix_now_s();
    let report = maintenance_report(
        &operation_id,
        "backup",
        started_at_s,
        finished_at_s,
        affected_counts,
        json!({}),
        vec![event_id.clone()],
        verification,
    );

    Ok(json!({
        "ok": true,
        "operationId": operation_id,
        "path": backup_path.display().to_string(),
        "sizeBytes": size_bytes,
        "ledgerEventId": event_id,
        "report": report,
    }))
}

/// Purge all memories from the brain.
pub fn purge(state: &crate::BrainState) -> Result<Value, RpcError> {
    let operation_id = Uuid::new_v4().to_string();
    let started_at_s = unix_now_s();
    let mut conn = state.store.conn()?;

    let memories_before = table_count(&conn, "memories");
    let affected_counts = json!({
        "reasoning_bridge_events": delete_table(&conn, "reasoning_bridge_events")?,
        "nightly_artifacts": delete_table(&conn, "nightly_artifacts")?,
        "autonomous_mutation_snapshots": delete_table(&conn, "autonomous_mutation_snapshots")?,
        "retrieval_weights": delete_table(&conn, "retrieval_weights")?,
        "retrieval_events": delete_table(&conn, "retrieval_events")?,
        "memory_atoms": delete_table(&conn, "memory_atoms")?,
        "memory_entities": delete_table(&conn, "memory_entities")?,
        "memory_relationships": delete_table(&conn, "memory_relationships")?,
        "memories": delete_table(&conn, "memories")?
    });
    let preserved_counts = preserved_counts(&conn);
    let event_id = append_admin_ledger(
        &mut conn,
        "maintenance.purge_brain",
        "system",
        Some(&operation_id),
        json!({
            "operation_id": operation_id.clone(),
            "affected_counts": affected_counts.clone(),
            "preserved_counts": preserved_counts.clone()
        }),
    )?;
    let verification =
        verify_ledger_conn(&conn).map_err(|e| rpc_err(-32603, format!("ledger_verify: {e}")))?;
    let finished_at_s = unix_now_s();
    let report = maintenance_report(
        &operation_id,
        "purge_brain",
        started_at_s,
        finished_at_s,
        affected_counts,
        preserved_counts,
        vec![event_id.clone()],
        verification,
    );

    Ok(json!({
        "ok": true,
        "operationId": operation_id,
        "memoriesPurged": memories_before,
        "ledgerEventId": event_id,
        "report": report,
    }))
}

fn maintenance_report(
    operation_id: &str,
    operation_type: &str,
    started_at_s: i64,
    finished_at_s: i64,
    affected_counts: Value,
    preserved_counts: Value,
    ledger_event_ids: Vec<String>,
    verification: Value,
) -> Value {
    json!({
        "operation_id": operation_id,
        "operationId": operation_id,
        "operation_type": operation_type,
        "operationType": operation_type,
        "started_at_s": started_at_s,
        "startedAtS": started_at_s,
        "finished_at_s": finished_at_s,
        "finishedAtS": finished_at_s,
        "status": "recorded",
        "affected_counts": affected_counts.clone(),
        "affectedCounts": affected_counts,
        "preserved_counts": preserved_counts.clone(),
        "preservedCounts": preserved_counts,
        "ledger_event_ids": ledger_event_ids.clone(),
        "ledgerEventIds": ledger_event_ids,
        "verification": verification
    })
}

fn table_count(conn: &rusqlite::Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap_or(0)
}

fn delete_table(conn: &rusqlite::Connection, table: &str) -> Result<i64, RpcError> {
    conn.execute(&format!("DELETE FROM {table}"), [])
        .map(|count| count as i64)
        .map_err(|e| rpc_err(-32603, format!("purge_{table}: {e}")))
}

fn preserved_counts(conn: &rusqlite::Connection) -> Value {
    json!({
        "settings": table_count(conn, "settings"),
        "ledger_events": table_count(conn, "ledger_events")
    })
}

fn find_pack(hom_dir: &Path, params: &Value) -> Option<PathBuf> {
    let explicit = params
        .get("source")
        .or_else(|| params.get("path"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let current_dir = std::env::current_dir().ok();
    let source_tree_pack = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(|root| root.join("alpha-packs").join("memory-pack.json"));
    let candidates = explicit
        .into_iter()
        .chain([hom_dir.join("alpha-packs").join("memory-pack.json")])
        .chain([hom_dir
            .join("alpha")
            .join("daemon")
            .join("alpha-packs")
            .join("memory-pack.json")])
        .chain(
            current_dir
                .into_iter()
                .map(|cwd| cwd.join("alpha-packs").join("memory-pack.json")),
        )
        .chain(source_tree_pack)
        .collect::<Vec<_>>();

    candidates.into_iter().find(|path| path.exists())
}

fn memory_value(memory: &Value) -> String {
    match memory.get("value") {
        Some(Value::String(value)) => value.clone(),
        Some(value) => serde_json::to_string(value).unwrap_or_default(),
        None => memory
            .get("content")
            .or_else(|| memory.get("text"))
            .map(|value| match value {
                Value::String(text) => text.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            })
            .unwrap_or_default(),
    }
}

fn is_duplicate_import_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("duplicate")
        || lower.contains("unique constraint failed")
        || lower.contains("constraint violation")
}

pub fn append_admin_ledger(
    conn: &mut rusqlite::Connection,
    event_type: &str,
    actor: &str,
    subject_id: Option<&str>,
    payload: Value,
) -> Result<String, RpcError> {
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("admin_ledger_tx: {e}")))?;
    let ledger = append_ledger_tx(&tx, event_type, actor, subject_id, payload, unix_now_s())
        .map_err(|e| rpc_err(-32603, format!("admin_ledger_append: {e}")))?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("admin_ledger_commit: {e}")))?;
    ledger
        .get("event_id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| rpc_err(-32603, "admin_ledger_missing_event_id"))
}
