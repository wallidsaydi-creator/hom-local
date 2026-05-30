# Architecture

## Overview

HOM Local is a local-first AI memory kernel built as a Rust workspace with 9 crates. The brain daemon runs as a single process with weighted priority worker queues, serving JSON-RPC requests over Unix domain sockets.

## Crate hierarchy

```
hom-shared          (types, crypto, envelope, RPC)
    ↑
hom-brain           (core daemon, memory, ledger, recall)
    ↑
hom-ingress         (HTTP layer, auth, capability mesh)
    ↑
hom-provider-*      (provider implementations)
```

## Brain daemon

The brain daemon (`hom-brain`) is the central component. It manages:

- **Memory store**: SQLite with WAL mode for concurrent reads
- **Ledger**: Append-only event store with hash chain integrity
- **Worker pools**: Weighted priority queues for Save, Recall, Cognition, Io, and Main operations
- **IPC server**: JSON-RPC over Unix domain socket
- **Services**: 55+ service modules covering memory, recall, diagnostics, quality gates, and more

### Worker dispatch

The brain uses weighted priority queues to dispatch work:

- **Save**: Memory persistence and ledger operations
- **Recall**: Memory retrieval and search
- **Cognition**: Quality gates, nightly runs, dream feedback
- **Io**: Import/export and backup operations
- **Main**: System status and admin operations

### IPC protocol

All communication uses JSON-RPC over Unix domain sockets:

```json
{
  "jsonrpc": "2.0",
  "method": "memory.recall",
  "params": {"query": "architecture boundaries", "limit": 5},
  "id": 1
}
```

## Ingress layer

The ingress layer (`hom-ingress`) provides:

- **HTTP server**: Axum-based HTTP API
- **Ed25519 authentication**: Signed request envelopes
- **Capability mesh**: Tool routing and provider management
- **Brain client**: UDS client for brain IPC

## Provider system

Provider crates implement the `HttpProvider` trait for different AI backends:

- **hom-provider-base**: Core provider abstractions and credentials
- **hom-provider-openai-compat**: OpenAI-compatible APIs
- **hom-provider-anthropic**: Anthropic Claude models
- **hom-provider-google**: Google Gemini models
- **hom-provider-local-models**: Local model servers (Ollama, LM Studio)
- **hom-provider-codex-oauth**: OAuth-based authentication

## Data flow

```
Client → HTTP Request → Ingress (auth, routing) → Brain (JSON-RPC) → Services → SQLite
                                                                    ↓
                                                              Ledger (append-only)
                                                                    ↓
                                                              Vector Index (PQ)
```
