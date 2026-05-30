//! Basic recall example
//!
//! This example demonstrates how to use HOM Local's memory recall system.

use hom_brain::App;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), String> {
    // Create a new brain instance
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let app = App::new(dir.path().to_path_buf()).map_err(|e| e.to_string())?;

    // Save some memories
    app.dispatch_value(
        "memory.save",
        json!({
            "key": "architecture:boundaries",
            "value": "The brain owns memory, ledger, and diagnostics while provider execution stays outside.",
            "memory_type": "declarative",
            "source": "example"
        }),
    )
    .await
    .expect("dispatch failed");

    app.dispatch_value(
        "memory.save",
        json!({
            "key": "recall:quality",
            "value": "Source-attributed recall ensures every memory has provenance tracking.",
            "memory_type": "declarative",
            "source": "example"
        }),
    )
    .await
    .expect("dispatch failed");

    // Recall memories
    let recalled = app
        .dispatch_value(
            "memory.recall",
            json!({
                "query": "architecture boundaries",
                "limit": 5
            }),
        )
        .await
        .expect("dispatch failed");

    println!("Recall results:");
    println!(
        "  Found {} memories",
        recalled["result_count"].as_i64().unwrap_or(0)
    );

    for memory in recalled["memories"].as_array().unwrap() {
        println!(
            "  - {} (score: {})",
            memory["key"].as_str().unwrap_or("unknown"),
            memory["score"].as_f64().unwrap_or(0.0)
        );
    }

    // Get source-attributed answer
    let answer = app
        .dispatch_value(
            "memory.answer",
            json!({
                "query": "Where do memory and diagnostics stay?",
                "limit": 5
            }),
        )
        .await
        .expect("dispatch failed");

    println!("\nAnswer:");
    println!("  Mode: {}", answer["mode"].as_str().unwrap_or("unknown"));
    println!(
        "  Claims: {}",
        answer["answer"]["claim_count"].as_i64().unwrap_or(0)
    );

    Ok(())
}
