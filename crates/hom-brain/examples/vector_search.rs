//! Vector search example
//!
//! This example demonstrates HOM Local's vector search capabilities.

use hom_brain::App;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let app = App::new(dir.path().to_path_buf()).map_err(|e| e.to_string())?;

    // Save memories
    let left = save_memory(
        &app,
        "vector:left",
        "Vector left memory contains semantic evidence about retrieval.",
        Some("vector-session"),
        None,
        Some("declarative"),
    )
    .await;

    let up = save_memory(
        &app,
        "vector:up",
        "Vector up memory contains evidence about aggregation.",
        Some("vector-session"),
        None,
        Some("declarative"),
    )
    .await;

    // Add vector embeddings
    app.dispatch_value(
        "memory.embedding.upsert",
        json!({
            "memory_id": left,
            "embedding_model": "test-embedding",
            "vector": [1.0, 0.0, 1.0, 0.0]
        }),
    )
    .await
    .expect("dispatch failed");

    app.dispatch_value(
        "memory.embedding.upsert",
        json!({
            "memory_id": up,
            "embedding_model": "test-embedding",
            "vector": [0.0, 1.0, 0.0, 1.0]
        }),
    )
    .await
    .expect("dispatch failed");

    // Run exact vector search benchmark
    let baseline = app
        .dispatch_value(
            "benchmarks.vector_exact",
            json!({
                "embedding_model": "test-embedding",
                "query_vector": [0.95, 0.05, 0.9, 0.1],
                "limit": 2
            }),
        )
        .await
        .expect("dispatch failed");

    println!("Exact vector search benchmark:");
    println!(
        "  Corpus size: {}",
        baseline["corpus_size"].as_i64().unwrap_or(0)
    );
    println!(
        "  Matches: {}",
        baseline["matches"].as_array().unwrap().len()
    );
    println!(
        "  Duration: {}ms",
        baseline["duration_ms"].as_u64().unwrap_or(0)
    );

    // Run PQ candidate generation benchmark
    let candidates = app
        .dispatch_value(
            "benchmarks.vector_pq_candidates",
            json!({
                "embedding_model": "test-embedding",
                "query_vector": [0.95, 0.05, 0.9, 0.1],
                "candidate_pool_size": 2,
                "subquantizers": 2,
                "centroids_per_subquantizer": 2,
                "iterations": 4
            }),
        )
        .await
        .expect("dispatch failed");

    println!("\nPQ candidate generation benchmark:");
    println!(
        "  Candidates: {}",
        candidates["candidates"].as_array().unwrap().len()
    );
    println!(
        "  Approximation: {}",
        candidates["approximation"].as_str().unwrap_or("unknown")
    );

    // Recall with vector search
    let recalled = app
        .dispatch_value(
            "memory.recall",
            json!({
                "query": "retrieval evidence",
                "limit": 2,
                "embedding_model": "test-embedding",
                "query_vector": [0.95, 0.05, 0.9, 0.1]
            }),
        )
        .await
        .expect("dispatch failed");

    println!("\nRecall with vector search:");
    for memory in recalled["memories"].as_array().unwrap() {
        println!(
            "  - {} (score: {})",
            memory["key"].as_str().unwrap_or("unknown"),
            memory["score"].as_f64().unwrap_or(0.0)
        );
    }

    Ok(())
}

async fn save_memory(
    app: &App,
    key: &str,
    value: &str,
    session_id: Option<&str>,
    project_id: Option<&str>,
    track: Option<&str>,
) -> String {
    let saved = app
        .dispatch_value(
            "memory.save",
            json!({
                "key": key,
                "value": value,
                "memory_type": "declarative",
                "source": "vector-example",
                "session_id": session_id,
                "project_id": project_id,
                "track": track
            }),
        )
        .await
        .expect("dispatch failed");

    saved["memory_id"].as_str().unwrap().to_string()
}
