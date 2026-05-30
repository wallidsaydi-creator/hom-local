# HOM Local

**A local brain server for AI agents.**

Most AI agents rebuild context from scratch every run.
Their memory is often hidden, hosted, unverifiable, or trapped inside one product.

HOM Local takes the opposite approach: memory is a local server.

Run the brain daemon on your machine, connect your app over HTTP/IPC, and get durable memory, source-attributed recall, validation gates, context packing, and an auditable ledger.

## Core guarantees

### 1. Recall

HOM Local stores structured memories and retrieves them with source attribution.
Agents can ask the local brain what it knows without relying only on chat history or opaque hosted memory.

### 2. Validation

Before information becomes memory, HOM Local can run quality gates: structure, relevance, substance, and factuality checks.
The goal is not to remember everything. The goal is to remember useful context with evidence.

### 3. Audit

Memory mutations are written to a tamper-evident local ledger.
Developers can inspect what changed, when it changed, and why the brain believes a piece of context exists.

### 4. Compaction

Long sessions can be compressed into durable continuity artifacts.
The compaction does not disappear into a prompt summary — it becomes memory that can be recalled, opened, and audited later.

## How it works

```text
Your app / agent harness
        │
        │ HTTP / IPC
        ▼
HOM Local ingress
        │
        ▼
HOM brain daemon
        │
        ├── SQLite memory store
        ├── source-attributed recall
        ├── quality gates
        ├── context packing
        ├── tamper-evident ledger
        └── compaction artifacts

HOM Local does not decide which model you use.
It gives your application a local memory backend that can be inspected, tested, and extended.
```

## What it does

- Durable local memory in SQLite with WAL mode
- Source-attributed recall with quality scoring
- Quality-gated memory save (four-wall assessment)
- Tamper-evident append-only ledger with hash chain verification
- Context packing with evidence cards and open handles
- Session compaction that becomes durable memory
- Product Quantization approximate vector search with exact rerank fallback
- Ed25519 envelope-based IPC authentication
- JSON-RPC over Unix domain socket + HTTP ingress on `127.0.0.1:9101`
- Synthetic tests and examples

## What HOM Local is not

HOM Local is not:

- a finished consumer app
- a chatbot
- a model provider framework
- a hosted memory service
- a replacement for your agent harness
- a commercial roadmap dump

It is the local backend brain your app can connect to.

## Model/provider boundary

HOM Local does not bundle provider implementations.

Providers belong in the app layer.
HOM Local owns the memory layer.

Your app chooses the model.
HOM Local stores, recalls, validates, packs, and audits memory.

## Why this matters

Agents need more than prompts and tool calls.

They need a memory layer that can answer:

- What do we know?
- Where did that memory come from?
- Was it validated before storage?
- Can the original source be opened?
- What changed in the brain over time?
- Can long sessions become durable continuity instead of disposable summaries?

HOM Local is built as that layer.

## Minimal flow

1. Save a memory from your app.
2. HOM Local validates and stores it.
3. The ledger records the mutation.
4. Later, recall returns source-attributed evidence.
5. Context packing turns recall into model-ready evidence cards.
6. Session compaction becomes durable memory instead of disposable summary text.

## Quick start

```bash
# Clone the repository
git clone https://github.com/wallidsaydi-creator/hom-local.git
cd hom-local

# Build
cargo build --release

# Start the brain daemon
cargo run --release --bin hom-brain &

# Start the ingress server
cargo run --release --bin hom-ingress &

# The ingress listens on http://127.0.0.1:9101
# Connect your app and start saving/recalling memories.
```

## Architecture

```
hom-local/
├── crates/
│   ├── hom-brain/     # Core brain daemon — memory, ledger, recall, quality gates
│   ├── hom-shared/    # Shared types, crypto, envelope, RPC, paths
│   └── hom-ingress/   # HTTP ingress layer with auth
├── docs/              # Architecture and API documentation
└── examples/          # Usage examples
```

## Configuration

The brain daemon reads configuration from:
- `~/.hom/config.json` — Main configuration
- Environment variables — `HOM_*` prefix
- Command line arguments — See `hom-brain --help`

## Development

```bash
# Run tests
cargo test

# Format code
cargo fmt

# Build documentation
cargo doc --open
```

## Documentation

| Document | Description |
|----------|-------------|
| [Architecture](docs/architecture.md) | Crate hierarchy, brain daemon, worker dispatch, IPC protocol |
| [API Reference](docs/api-reference.md) | Brain IPC methods and HTTP API routes |
| [Configuration](docs/configuration.md) | Environment variables, config file, permissions |
| [Security](docs/security.md) | Authentication, security gates, quality gates, audit trail |
| [Memory Model](docs/memory-model.md) | Memory structure, types, source attribution, vector embeddings |
| [Recall System](docs/recall-system.md) | Recall modes, pipeline, scoring, context packing |
| [Quality Gates](docs/quality-gates.md) | Four-wall assessment, quality scoring, benchmarks |
| [Operator Guide](docs/OPERATOR_GUIDE.md) | How an LLM/agent should operate through HOM Local |
| [Testing](docs/testing.md) | Test structure, running tests, coverage |
| [Contributing](docs/contributing.md) | Development setup, contribution process, workflow |
| [Local vs Oracle](docs/local-vs-oracle.md) | Licensing boundary between HOM Local and HOM Oracle |
| [FAQ](docs/faq.md) | General, installation, configuration, memory, recall, troubleshooting |

## License

HOM Local is licensed under the Apache License, Version 2.0.

This license applies only to the code in this repository.

HOM Oracle is a separate private system and is not included in this repository.

See [LICENSE](LICENSE) for the full license text.
