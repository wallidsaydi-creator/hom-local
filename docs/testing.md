# Testing

## Overview

HOM Local uses comprehensive integration tests to validate the full stack from brain daemon to HTTP ingress.

## Test structure

```
tests/
├── brain_e2e.rs           # Brain daemon end-to-end tests
└── ingress_e2e.rs         # HTTP ingress end-to-end tests

crates/
├── hom-brain/tests/       # Brain-specific tests
├── hom-ingress/tests/     # Ingress-specific tests
└── .../src/               # Unit tests within source files
```

## Running tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test brain_boots_with_cognitive_runtime_status

# Run with output
cargo test -- --nocapture

# Run tests in parallel
cargo test -- --test-threads=4

# Run specific crate tests
cargo test -p hom-brain
cargo test -p hom-ingress
```

## Brain end-to-end tests

### Basic operations

```rust
#[tokio::test]
async fn brain_boots_with_cognitive_runtime_status() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(dir.path().to_path_buf()).unwrap();
    
    let status = app.dispatch_value("runtime.status", json!({})).await.unwrap();
    assert_eq!(status["ok"], true);
}
```

### Memory operations

```rust
#[tokio::test]
async fn memory_save_and_recall_remain_brain_owned() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(dir.path().to_path_buf()).unwrap();
    
    // Save memory
    let saved = app.dispatch_value("memory.save", json!({
        "key": "test",
        "value": "Test memory",
        "memory_type": "declarative"
    })).await.unwrap();
    
    // Recall memory
    let recalled = app.dispatch_value("memory.recall", json!({
        "query": "test",
        "limit": 5
    })).await.unwrap();
    
    assert!(recalled["result_count"].as_i64().unwrap() >= 1);
}
```

### Vector search tests

```rust
#[tokio::test]
async fn vector_exact_baseline_benchmark() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(dir.path().to_path_buf()).unwrap();
    
    // Save memories
    let left = save_memory(&app, "left", "Left memory", None, None, Some("vector")).await;
    let up = save_memory(&app, "up", "Up memory", None, None, Some("vector")).await;
    
    // Add embeddings
    app.dispatch_value("memory.embedding.upsert", json!({
        "memory_id": left,
        "embedding_model": "test-embedding",
        "vector": [1.0, 0.0, 0.0]
    })).await.unwrap();
    
    // Run benchmark
    let baseline = app.dispatch_value("benchmarks.vector_exact", json!({
        "embedding_model": "test-embedding",
        "query_vector": [0.95, 0.05, 0.0],
        "limit": 2
    })).await.unwrap();
    
    assert_eq!(baseline["ok"], true);
    assert_eq!(baseline["benchmark_kind"], "vector_exact_baseline");
}
```

## Ingress end-to-end tests

### Auth tests

```rust
#[tokio::test]
async fn signed_recall_reaches_brain_over_uds() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("brain.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();
    
    // Mock brain response
    let brain_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        // ... handle request
    });
    
    // Create signed request
    let body = json!({"query": "test", "limit": 3});
    let signature = sign_payload(&app_keys.private_key, &body, nonce, ts).unwrap();
    
    // Send request
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    
    brain_task.await.unwrap();
}
```

### Tool execution tests

```rust
#[tokio::test]
async fn shell_execute_returns_stdout_exit_code() {
    let dir = tempdir().unwrap();
    // ... setup brain mock
    
    let response = app.oneshot(json_request(
        Method::POST,
        "/api/ui/tools/call",
        json!({
            "tool_id": "shell.execute",
            "arguments": {"command": "pwd", "cwd": dir.path()}
        })
    )).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let value = response_json(response).await;
    assert_eq!(value["outcome"], "executed");
    assert_eq!(value["result"]["exit_code"], 0);
}
```

## Test utilities

### Save memory helper

```rust
async fn save_memory(
    app: &App,
    key: &str,
    value: &str,
    session_id: Option<&str>,
    project_id: Option<&str>,
    track: Option<&str>,
) -> String {
    let saved = app.dispatch_value("memory.save", json!({
        "key": key,
        "value": value,
        "memory_type": "declarative",
        "source": "test",
        "session_id": session_id,
        "project_id": project_id,
        "track": track
    })).await.unwrap();
    
    saved["memory_id"].as_str().unwrap().to_string()
}
```

### Mock brain client

```rust
fn brain_client_for_test(socket_path: PathBuf) -> BrainClient {
    let keys = generate_keypair();
    BrainClient::new_for_test(
        socket_path,
        keys.public_key,
        keys.private_key,
    )
}
```

## Coverage

Run coverage reports:

```bash
# Install cargo-tarpaulin
cargo install cargo-tarpaulin

# Generate coverage
cargo tarpaulin --out Html

# View report
open tarpaulin-report.html
```
