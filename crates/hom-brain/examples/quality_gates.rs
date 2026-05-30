//! Quality gates example
//!
//! This example demonstrates HOM Local's quality assessment system.

use hom_brain::App;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let app = App::new(dir.path().to_path_buf()).map_err(|e| e.to_string())?;

    // Save a high-quality memory
    let high_quality = save_memory(
        &app,
        "quality:high",
        "The brain daemon manages SQLite with WAL mode for concurrent reads while maintaining hash chain integrity.",
        None,
        Some("quality-example"),
        Some("declarative"),
    )
    .await;

    // Save a low-quality memory (empty value)
    let low_quality = save_memory(
        &app,
        "quality:low",
        "",
        None,
        Some("quality-example"),
        Some("declarative"),
    )
    .await;

    // Run quality assessment
    let quality = app
        .dispatch_value(
            "quality.assess",
            json!({
                "memory_id": high_quality,
                "walls": ["form", "filter", "substance", "factuality"]
            }),
        )
        .await
        .expect("dispatch failed");

    println!("Quality assessment for high-quality memory:");
    println!("  Pass: {}", quality["pass"].as_bool().unwrap_or(false));
    println!("  Score: {}", quality["score"].as_f64().unwrap_or(0.0));
    println!(
        "  Form: {}",
        quality["walls"]["form"].as_f64().unwrap_or(0.0)
    );
    println!(
        "  Filter: {}",
        quality["walls"]["filter"].as_f64().unwrap_or(0.0)
    );
    println!(
        "  Substance: {}",
        quality["walls"]["substance"].as_f64().unwrap_or(0.0)
    );
    println!(
        "  Factuality: {}",
        quality["walls"]["factuality"].as_f64().unwrap_or(0.0)
    );

    // Run quality assessment on low-quality memory
    let low_quality_result = app
        .dispatch_value(
            "quality.assess",
            json!({
                "memory_id": low_quality,
                "walls": ["form", "filter", "substance", "factuality"]
            }),
        )
        .await
        .expect("dispatch failed");

    println!("\nQuality assessment for low-quality memory:");
    println!(
        "  Pass: {}",
        low_quality_result["pass"].as_bool().unwrap_or(false)
    );
    println!(
        "  Score: {}",
        low_quality_result["score"].as_f64().unwrap_or(0.0)
    );

    // Get quality gate status
    let gate_status = app
        .dispatch_value("quality.gate.status", json!({}))
        .await
        .expect("dispatch failed");

    println!("\nQuality gate status:");
    println!(
        "  Total assessed: {}",
        gate_status["total_assessed"].as_i64().unwrap_or(0)
    );
    println!(
        "  Pass rate: {}",
        gate_status["pass_rate"].as_f64().unwrap_or(0.0)
    );

    // Run nightly quality assessment
    let nightly = app
        .dispatch_value("nightly.dry_run", json!({}))
        .await
        .expect("dispatch failed");

    println!("\nNightly quality dry run:");
    println!("  Mode: {}", nightly["mode"].as_str().unwrap_or("unknown"));
    println!(
        "  Proposals: {}",
        nightly["proposal_count"].as_i64().unwrap_or(0)
    );

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
                "source": "quality-example",
                "session_id": session_id,
                "project_id": project_id,
                "track": track
            }),
        )
        .await
        .expect("dispatch failed");

    saved["memory_id"].as_str().unwrap().to_string()
}
