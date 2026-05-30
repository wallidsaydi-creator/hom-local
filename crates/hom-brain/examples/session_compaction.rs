//! Session compaction example
//!
//! This example demonstrates HOM Local's session compaction system.

use hom_brain::App;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let app = App::new(dir.path().to_path_buf()).map_err(|e| e.to_string())?;

    let session_id = "compaction-example-session";

    // Save memories for the session
    save_memory(
        &app,
        "conversation:design",
        "User asked about session compaction. The goal is to preserve full detail while returning a concise summary.",
        Some(session_id),
        Some("compaction-example"),
        Some("conversation"),
    )
    .await;

    save_memory(
        &app,
        "tool:inspection",
        "Inspected session.compact, backed up touched files, and prepared RED test before implementation.",
        Some(session_id),
        Some("compaction-example"),
        Some("tool_usage"),
    )
    .await;

    save_memory(
        &app,
        "action:implementation",
        "Implemented native model-agnostic compaction with deterministic provider catalog context windows.",
        Some(session_id),
        Some("compaction-example"),
        Some("action_taken"),
    )
    .await;

    // Check context pressure before compaction
    let pressure = app
        .dispatch_value(
            "session.context_pressure",
            json!({
                "provider_id": "openai",
                "model_id": "gpt-4",
                "input_tokens_estimated": 142000,
                "reserved_output_tokens": 8000,
                "model_catalog": {
                    "source": "provider_model_catalog",
                    "verified": true,
                    "context_window_tokens": 200000,
                    "max_output_tokens": 8192
                }
            }),
        )
        .await
        .expect("dispatch failed");

    println!("Context pressure before compaction:");
    println!(
        "  Band: {}",
        pressure["pressure"]["band"].as_str().unwrap_or("unknown")
    );
    println!(
        "  Ratio: {} bps",
        pressure["pressure"]["ratio_bps"].as_i64().unwrap_or(0)
    );

    // Run session compaction
    let compacted = app
        .dispatch_value(
            "session.compact",
            json!({
                "session_id": session_id,
                "provider_id": "openai",
                "model_id": "gpt-4",
                "input_tokens_estimated": 142000,
                "model_catalog": {
                    "source": "provider_model_catalog",
                    "verified": true,
                    "context_window_tokens": 200000,
                    "max_output_tokens": 8192
                }
            }),
        )
        .await
        .expect("dispatch failed");

    println!("\nSession compaction result:");
    println!(
        "  Compaction kind: {}",
        compacted["compaction_kind"].as_str().unwrap_or("unknown")
    );
    println!(
        "  Native compaction: {}",
        compacted["native_compaction"].as_bool().unwrap_or(false)
    );
    println!(
        "  Continuity guaranteed: {}",
        compacted["continuity_guaranteed"]
            .as_bool()
            .unwrap_or(false)
    );

    // Get compaction summary
    let summary = compacted["continuity"]["summary"].as_str().unwrap_or("");
    println!("\nCompaction summary:");
    println!("  {}", summary);

    // List compaction artifacts
    let list = app
        .dispatch_value("session.compactions", json!({"limit": 5}))
        .await
        .expect("dispatch failed");

    println!("\nCompaction artifacts:");
    for item in list["items"].as_array().unwrap() {
        println!(
            "  - {} (native: {})",
            item["memory_id"].as_str().unwrap_or("unknown"),
            item["native_compaction"].as_bool().unwrap_or(false)
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
                "source": "compaction-example",
                "session_id": session_id,
                "project_id": project_id,
                "track": track
            }),
        )
        .await
        .expect("dispatch failed");

    saved["memory_id"].as_str().unwrap().to_string()
}
