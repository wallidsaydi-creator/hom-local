# HOM Local

**Open-source local-first AI memory kernel**

HOM Local is a self-hosted memory server that gives your AI applications persistent, source-attributed recall. It stores memories in SQLite with WAL mode, enforces tamper-evident append-only ledger integrity, and provides quality-gated context packing for downstream models.

## What it does

- **Memory kernel**: Save, recall, and manage structured memories with source attribution
- **Tamper-evident ledger**: Append-only event store with hash chain verification
- **Quality gates**: Four-wall assessment (form, filter, substance, factuality)
- **Context packing**: Evidence cards, open handles, and traceability for LLM context
- **Vector search**: Product Quantization approximate search with exact rerank fallback
- **Multi-provider support**: OpenAI-compatible, Anthropic, Google, local models
- **Ed25519 authentication**: Envelope-based IPC with signed requests
- **JSON-RPC over UDS**: Unix domain socket brain IPC for local-first operation

## Quick start

```bash
# Clone the repository
git clone https://github.com/hom-local/hom-local.git
cd hom-local

# Build the brain daemon
cargo build --release --bin hom-brain

# Start the brain
cargo run --release --bin hom-brain

# The brain listens on a Unix domain socket at ~/.hom/brain.sock
```

## Architecture

```
hom-local/
├── crates/
│   ├── hom-brain/          # Core brain daemon — memory, ledger, recall, quality gates
│   ├── hom-shared/         # Shared types, crypto, envelope, RPC, paths
│   ├── hom-ingress/        # HTTP ingress layer with auth and capability mesh
│   ├── hom-provider-base/  # Provider abstraction and credentials
│   ├── hom-provider-openai-compat/  # OpenAI-compatible provider
│   ├── hom-provider-anthropic/      # Anthropic provider
│   ├── hom-provider-google/         # Google provider
│   ├── hom-provider-local-models/   # Local model provider
│   └── hom-provider-codex-oauth/    # Codex OAuth provider
├── docs/                   # Architecture and API documentation
├── examples/               # Usage examples
└── tests/                  # Integration tests
```

## Key concepts

### Memory kernel

The brain daemon manages a local SQLite database with WAL mode. Memories are stored with:
- Source attribution (where the memory came from)
- Quality scores (computed by the quality gate)
- Metadata (session, project, track information)
- Vector embeddings (for approximate search)

### Tamper-evident ledger

Every mutation is recorded as an append-only ledger event with:
- SHA-256 hash chain linking events
- Event types for audit trail
- Verification on read

### Quality gates

The four-wall quality assessment:
1. **Form**: Structural correctness of the memory
2. **Filter**: Relevance and deduplication
3. **Substance**: Evidence strength and citation coverage
4. **Factuality**: Atomic precision via EvidenceAtom/FActScore-style evaluation

### Context packing

The context packer assembles recall results into evidence cards with:
- Open handles for follow-up queries
- Traceability links back to source memories
- Budget management for LLM context windows
- Mathematical ledger diagnostics

## Provider system

HOM Local supports multiple AI providers through a unified provider abstraction:

| Provider | Status | Notes |
|----------|--------|-------|
| OpenAI-compatible | Stable | Works with any OpenAI-compatible API |
| Anthropic | Stable | Claude models |
| Google | Stable | Gemini models |
| Local models | Stable | Ollama, LM Studio, etc. |
| Codex OAuth | Experimental | OAuth-based authentication |

## Configuration

The brain daemon reads configuration from:
- `~/.hom/config.json` — Main configuration
- Environment variables — `HOM_*` prefix
- Command line arguments — See `hom-brain --help`

## Development

```bash
# Run tests
cargo test

# Run clippy
cargo clippy -- -D warnings

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
| [Configuration](docs/configuration.md) | Environment variables, config file, permissions, providers |
| [Security](docs/security.md) | Authentication, security gates, quality gates, audit trail |
| [Memory Model](docs/memory-model.md) | Memory structure, types, source attribution, vector embeddings |
| [Recall System](docs/recall-system.md) | Recall modes, pipeline, scoring, context packing |
| [Quality Gates](docs/quality-gates.md) | Four-wall assessment, quality scoring, benchmarks |
| [Provider System](docs/provider-system.md) | Provider architecture, circuit breaker, credentials, streaming |
| [Operator Guide](docs/OPERATOR_GUIDE.md) | How an LLM/agent should operate through HOM Local |
| [Testing](docs/testing.md) | Test structure, running tests, coverage |
| [Contributing](docs/contributing.md) | Development setup, contribution process, workflow |
| [Local vs Oracle](docs/local-vs-oracle.md) | Licensing boundary between HOM Local and HOM Oracle |
| [FAQ](docs/faq.md) | General, installation, configuration, memory, recall, providers |

## License

HOM Local is licensed under the Apache License, Version 2.0.

This license applies only to the code in this repository.

HOM Oracle is a separate private system and is not included in this repository.

See [LICENSE](LICENSE) for the full license text.
