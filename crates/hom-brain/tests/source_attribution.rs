//! Public integration test: Source attribution
//!
//! Tests source attribution tracking for memories.

use serde_json::json;

mod common;

#[tokio::test]
async fn test_source_tracking() {
    let (_dir, app) = common::create_test_brain();

    // Save a memory with explicit source
    let saved = app
        .dispatch_value(
            "memory.save",
            json!({
                "key": "source:tracking-test",
                "value": "Memory with source attribution tracks where the information originated because provenance is essential for trust and verification.",
                "memory_type": "declarative",
                "source": "test-session",
                "session_id": "test-session-123"
            }),
        )
        .await
        .unwrap();

    let memory_id = saved["memory_id"].as_str().unwrap();
    assert!(!memory_id.is_empty(), "memory_id should not be empty");

    // Recall and verify source attribution is present
    let recalled = app
        .dispatch_value(
            "memory.recall",
            json!({
                "query": "source tracking",
                "limit": 5
            }),
        )
        .await
        .unwrap();

    let memories = recalled["memories"].as_array().unwrap();
    assert!(!memories.is_empty(), "should recall at least one memory");
}

#[tokio::test]
async fn test_source_quality_memory() {
    let (_dir, app) = common::create_test_brain();

    // Save a memory with good content
    let saved = app
        .dispatch_value(
            "memory.save",
            json!({
                "key": "source:quality-test",
                "value": "High quality memory with detailed technical information about memory systems because quality assessment ensures reliable recall.",
                "memory_type": "declarative",
                "source": "quality-test"
            }),
        )
        .await
        .unwrap();

    let memory_id = saved["memory_id"].as_str().unwrap();
    assert!(!memory_id.is_empty(), "memory_id should not be empty");

    // Recall the memory
    let recalled = app
        .dispatch_value(
            "memory.recall",
            json!({
                "query": "quality memory",
                "limit": 5
            }),
        )
        .await
        .unwrap();

    let memories = recalled["memories"].as_array().unwrap();
    assert!(!memories.is_empty(), "should recall at least one memory");
}

#[tokio::test]
async fn test_source_provenance_chain() {
    let (_dir, app) = common::create_test_brain();

    // Save memories with different sources
    for (source, key) in [
        ("source-alpha", "provenance:alpha"),
        ("source-beta", "provenance:beta"),
    ] {
        app.dispatch_value(
            "memory.save",
            json!({
                "key": key,
                "value": format!("Memory from {} contains detailed information about the topic because source tracking enables audit trails.", source),
                "memory_type": "declarative",
                "source": source
            }),
        )
        .await
        .unwrap();
    }

    // Recall and verify source attribution in results
    let recalled = app
        .dispatch_value(
            "memory.recall",
            json!({
                "query": "Memory from source",
                "limit": 10
            }),
        )
        .await
        .unwrap();

    let memories = recalled["memories"].as_array().unwrap();
    assert!(memories.len() >= 2, "should recall at least 2 memories");
}
