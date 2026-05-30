# Provider System

## Overview

HOM Local supports multiple AI providers through a unified provider abstraction with credential management, circuit breakers, and health monitoring.

## Provider architecture

```
ProviderBase (hom-provider-base)
├── HttpProvider trait
├── Circuit breaker
├── EWMA health tracking
├── Credential management
└── Streaming support

Provider implementations:
├── hom-provider-openai-compat
├── hom-provider-anthropic
├── hom-provider-google
├── hom-provider-local-models
└── hom-provider-codex-oauth
```

## Provider trait

All providers implement the `HttpProvider` trait:

```rust
#[async_trait]
pub trait HttpProvider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse>;
    async fn models(&self) -> Result<Vec<ModelInfo>>;
    fn name(&self) -> &str;
    fn supports_streaming(&self) -> bool;
}
```

## Circuit breaker

Providers use circuit breakers to handle failures:

### States

| State | Description |
|-------|-------------|
| Closed | Normal operation |
| Open | Failure threshold exceeded |
| Half-Open | Testing recovery |

### Configuration

```json
{
  "circuit_breaker": {
    "failure_threshold": 5,
    "recovery_timeout_s": 60,
    "half_open_max_calls": 3
  }
}
```

## Health tracking

EWMA (Exponentially Weighted Moving Average) tracks provider health:

```json
{
  "health": {
    "ewma_latency_ms": 120.5,
    "success_rate": 0.98,
    "last_failure_s": 1700000000
  }
}
```

## Credential management

### Keychain integration

API keys are stored in the OS keychain:

```rust
// Store credential
store_credential("nvidia", "api_key", "live-secret-key")?;

// Retrieve credential
let key = resolve_api_key("nvidia", None)?;
```

### OAuth flow

Codex OAuth uses device code flow:

1. Start auth → Get device code
2. User authenticates in browser
3. Poll for completion
4. Store access token

### Credential references

Credentials are referenced, never returned:

```json
{
  "credential_ref": "cred_nvidia_abc123",
  "secret_material_returned": false
}
```

## Provider catalog

### Seed catalog

Providers include seed model catalogs:

```json
{
  "id": "openrouter",
  "models": [
    {"id": "anthropic/claude-sonnet-4-20250514", "can_chat": true},
    {"id": "openai/gpt-4", "can_chat": true}
  ]
}
```

### Live discovery

Providers can discover models dynamically:

```json
{
  "source": "live_provider_catalog",
  "total": 151,
  "items": [...]
}
```

## Streaming support

Providers support SSE streaming for real-time responses:

```rust
async fn stream_complete(&self, request: CompletionRequest) -> Result<Stream<Chunk>> {
    let response = self.client.post(url).json(&body).send().await?;
    let stream = response.bytes_stream().map(parse_sse_line);
    Ok(stream)
}
```

## Error handling

Provider errors are mapped to appropriate HTTP status codes:

| Error | Status | Description |
|-------|--------|-------------|
| `provider_key_missing` | 400 | No API key configured |
| `provider_transport_unavailable` | 503 | Provider offline |
| `provider_quota_exceeded` | 429 | Rate limit hit |
| `provider_model_not_found` | 404 | Unknown model |

## Configuration

### Provider enable/disable

```json
{
  "providers": {
    "openai-compat": {"enabled": true},
    "anthropic": {"enabled": false},
    "local-models": {"enabled": true}
  }
}
```

### Base URL override

```json
{
  "providers": {
    "local-models": {
      "base_url": "http://localhost:11434/v1"
    }
  }
}
```

### Default models

```json
{
  "providers": {
    "openai-compat": {
      "default_model": "gpt-4"
    }
  }
}
```
