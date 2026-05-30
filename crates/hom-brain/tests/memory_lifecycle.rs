//! Public integration test: Memory lifecycle
//!
//! Tests memory save and recall operations.

use serde_json::json;

mod common;

#[tokio::test]
async fn test_memory_save_and_recall() {
    let (_dir, app) = common::create_test_brain();

    // Save a memory with substantive content
    let saved = app
        .dispatch_value(
            "memory.save",
            json!({
                "key": "lifecycle:save-test",
                "value": "Memory save and retrieval works correctly because the system validates input before persisting to SQLite storage on 2026-01-01.",
                "memory_type": "declarative",
                "source": "test"
            }),
        )
        .await
        .unwrap();

    let memory_id = saved["memory_id"].as_str().expect("missing memory_id");
    assert!(!memory_id.is_empty(), "memory_id should not be empty");

    // Recall memories
    let recalled = app
        .dispatch_value(
            "memory.recall",
            json!({
                "query": "lifecycle save test",
                "limit": 5
            }),
        )
        .await
        .unwrap();

    let memories = recalled["memories"].as_array().unwrap();
    assert!(!memories.is_empty(), "should recall at least one memory");
}

#[tokio::test]
async fn test_memory_recall_multiple() {
    let (_dir, app) = common::create_test_brain();

    // Save multiple memories with substantive content
    for (key, value) in [
        (
            "recall:alpha",
            "Alpha memory contains detailed information about search systems because retrieval accuracy depends on indexing strategies.",
        ),
        (
            "recall:beta",
            "Beta memory contains detailed information about search algorithms because ranking strategies determine result quality.",
        ),
        (
            "recall:gamma",
            "Gamma memory contains detailed information about unrelated topics because weather patterns affect daily planning.",
        ),
    ] {
        app.dispatch_value(
            "memory.save",
            json!({
                "key": key,
                "value": value,
                "memory_type": "declarative",
                "source": "test"
            }),
        )
        .await
        .unwrap();
    }

    // Recall memories
    let recalled = app
        .dispatch_value(
            "memory.recall",
            json!({
                "query": "search systems",
                "limit": 10
            }),
        )
        .await
        .unwrap();

    let memories = recalled["memories"].as_array().unwrap();
    assert!(
        memories.len() >= 2,
        "should recall at least 2 memories, found {}",
        memories.len()
    );
}

#[tokio::test]
async fn test_memory_types() {
    let (_dir, app) = common::create_test_brain();

    // Test all supported memory types with substantive content
    for memory_type in &["declarative", "procedural", "episodic", "semantic"] {
        let saved = app
            .dispatch_value(
                "memory.save",
                json!({
                    "key": format!("type:{}", memory_type),
                    "value": format!("Memory of type {} contains detailed information about the topic because each type serves a specific purpose.", memory_type),
                    "memory_type": memory_type,
                    "source": "test"
                }),
            )
            .await
            .unwrap_or_else(|_| panic!("failed to save {} memory", memory_type));

        let memory_id = saved["memory_id"].as_str().unwrap();
        assert!(
            !memory_id.is_empty(),
            "{} memory_id should not be empty",
            memory_type
        );
    }
}
