# Frequently Asked Questions

## General

### What is HOM Local?

HOM Local is an open-source local-first AI memory kernel. It provides persistent, source-attributed recall for AI applications, storing memories in SQLite with tamper-evident ledger integrity.

### How is HOM Local different from other memory systems?

- **Local-first**: All data stays on your machine
- **Source-attributed**: Every memory tracks its origin
- **Quality-gated**: Four-wall assessment ensures memory quality
- **Tamper-evident**: Append-only ledger with hash chain verification
- **Vector search**: Product Quantization for approximate search

### Is HOM Local production-ready?

HOM Local is in active development. The core memory and recall systems are stable, but some features (like vector search activation) require threshold profiles.

## Installation

### What are the system requirements?

- Rust 1.94+ (edition 2024)
- SQLite (bundled via rusqlite)
- 64-bit Linux, macOS, or Windows

### How do I install from source?

```bash
git clone https://github.com/hom-local/hom-local.git
cd hom-local
cargo build --release
```

### Can I run HOM Local without Rust?

Currently, building from source is the primary installation method. Pre-built binaries may be available in future releases.

## Configuration

### Where is the configuration stored?

Configuration is stored in `~/.hom/config.json` with environment variable overrides.

### How do I change the data directory?

Set the `HOM_DIR` environment variable:

```bash
export HOM_DIR=/path/to/data
cargo run --bin hom-brain
```

### How do I connect my app to the brain server?

Start the brain daemon and ingress server, then connect over HTTP:

```bash
cargo run --release --bin hom-brain &
cargo run --release --bin hom-ingress &
# Your app connects to http://127.0.0.1:9101
```

## Memory

### How are memories stored?

Memories are stored in SQLite with WAL mode. Each memory has:
- Unique ID
- Key and value
- Memory type
- Source attribution
- Quality score
- Metadata

### What memory types are supported?

- `declarative`: Factual knowledge
- `procedural`: How-to instructions
- `episodic`: Event sequences
- `semantic`: Conceptual relationships
- `action_trace`: Tool execution traces
- `reasoning`: Reasoning artifacts

### How is memory quality assessed?

Memories pass through four quality walls:
1. **Form**: Structural correctness
2. **Filter**: Relevance and deduplication
3. **Substance**: Evidence strength
4. **Factuality**: Atomic precision

## Recall

### What recall modes are available?

- **Text**: Full-text search
- **Vector**: Approximate vector search
- **Hybrid**: Combined text and vector
- **Smart**: Intelligent multi-strategy

### How does vector search work?

HOM Local uses Product Quantization:
1. Codebook training via K-means
2. Vector encoding as indices
3. Asymmetric distance computation
4. Candidate generation
5. Exact rerank on candidates

### How do I activate vector search?

Vector search requires a threshold profile:

```json
{
  "min_recall_at_k": 0.95,
  "min_candidate_pool_hit_rate": 0.95,
  "require_top1_preserved": true
}
```

## Providers

### What is the provider model in HOM Local?

Provider implementations are **app-layer concerns**. The brain daemon exposes provider catalog metadata (what providers the app has configured), but the actual provider runtime lives in the application layer, not in this repository.

The brain tracks provider metadata through `providers.list` and `providers.model_catalog`. The ingress server's provider routes return "provider runtime not configured" in the public release.

### How do I connect a provider runtime?

Build your own provider adapter in your application layer. The brain provides the memory, recall, quality, and ledger primitives. Your app handles provider communication and exposes it through the brain's catalog metadata.

## Security

### How are API keys stored?

API keys are stored in the OS keychain (macOS Keychain, Linux Secret Service, Windows Credential Locker).

### How is authentication handled?

HOM Local uses Ed25519 signed envelopes for IPC authentication.

### Can HOM Local be exposed to the internet?

HOM Local is designed for local-first operation. If you need external access, use a reverse proxy with TLS.

## Development

### How do I run tests?

```bash
cargo test
```

### How do I contribute?

See [contributing.md](contributing.md) for the full contribution guide.

### Where do I report bugs?

Report bugs via [GitHub Issues](https://github.com/hom-local/hom-local/issues).

## Troubleshooting

### Brain daemon won't start

1. Check if port is in use
2. Verify configuration file
3. Check logs for errors
4. Ensure SQLite is accessible

### Recall returns no results

1. Verify memories exist
2. Check query syntax
3. Ensure source attribution is correct
4. Review quality scores

### Provider connection fails

1. Verify API key is configured
2. Check network connectivity
3. Review provider status
4. Check rate limits

### Vector search not working

1. Ensure embeddings exist
2. Check threshold profile
3. Verify embedding model
4. Review corpus size
