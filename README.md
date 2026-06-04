# HOM Local

[![CI](https://github.com/wallidsaydi-creator/hom-local/actions/workflows/ci.yml/badge.svg)](https://github.com/wallidsaydi-creator/hom-local/actions)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![crates.io](https://img.shields.io/crates/v/hom-brain.svg)](https://crates.io/crates/hom-brain)
[![Star History](https://api.star-history.com/svg?repos=wallidsaydi-creator/hom-local&type=Date)](https://star-history.com/#wallidsaydi-creator/hom-local&Date)

**Local-first memory server for AI agents and harnesses.**

If you build AI systems that must remember work across sessions, model changes, and provider swaps, HOM Local gives you a persistent memory core with provenance, gates, and continuity.

If this saves your team time, [please star the repo](https://github.com/wallidsaydi-creator/hom-local) so more builders can find it.

---

## Table of Contents

- [The problem](#the-problem)
- [Who should use HOM Local](#who-should-use-hom-local)
- [Core guarantees](#core-guarantees)
- [Competitive architecture](#competitive-architecture)
- [What makes it different](#what-makes-it-different)
- [How it maps to real workflows](#how-it-maps-to-real-workflows)
- [What it does](#what-it-does)
- [What HOM Local is not](#what-hom-local-is-not)
- [Provider boundary and model choice](#provider-boundary-and-model-choice)
- [Post-compaction as a memory artifact](#post-compaction-as-a-memory-artifact)
- [Fast start](#fast-start)
- [Install](#install)
- [Architecture](#architecture)
- [Configuration](#configuration)
- [Development](#development)
- [Documentation](#documentation)

---

## The problem

Most AI stacks still lose context every time a session ends, a model changes, or a handler restarts. That creates the **re-briefing tax**: re-stating decisions, constraints, and progress over and over.

HOM Local fixes that at the memory layer, not at the chat surface.

- keep context on your machine
- keep memory tied to provenance
- keep continuity across provider and routing choices

---

## Who should use HOM Local

HOM Local is for teams that care about memory as part of their backend, including:

- AI copilots and harnesses that need context across long workflows.
- Tool-rich agents where actions must remain auditable and recoverable.
- Multi-model stacks that may switch from Claude/GPT/Gemini/etc. without rewriting memory.
- Teams that want local-first deployment and explicit trust boundaries.

---

## Core guarantees

### Recall

Structured memory with source attribution so agents can ask “what do we know?” and get a provenance-aware answer path.

### Validation

Quality gates on memory write path: structure, relevance, substance, and factuality.

### Audit

Mutation ledgering with tamper-evident local history for traceability and change visibility.

### Compaction

Session compaction is not just prompt trimming; it is durable continuity for later runs.

---

## Competitive architecture

HOM Local is designed as dedicated memory service:

- **Three-crate workspace:** `hom-brain`, `hom-ingress`, `hom-shared`.
- **Clear boundaries:** ingress handles HTTP/auth; brain handles memory semantics; shared holds protocol + crypto contracts.
- **Deterministic IPC + HTTP edge:** JSON-RPC over Unix domain sockets in the brain path with an HTTP boundary in ingress.
- **Concurrency design:** weighted priority scheduling for save, recall, cognition, I/O, and main ops.
- **Provenance-first:** append-only ledger plus source-linked memory artifacts.
- **Provider-neutral memory surface:** model provider changes do not require memory rewrites.

```text
Client/Agent → hom-ingress (auth/routing) → hom-brain (memory + gates + ledger) → SQLite WAL + vector index
```

---

## What makes it different

| | HOM Local | Typical memory integrations |
|---|---|---|
| Local-first data control | ✅ | variable |
| Source-attributed recall | ✅ | often opaque |
| Provider-agnostic memory | ✅ | often coupled |
| Tamper-evident memory history | ✅ | often unavailable |
| Compaction-to-continuity pipeline | ✅ | often prompt-only |
| Quality-gated writes | ✅ | often simple or absent |
| Open architecture for app control | ✅ | often limited |

If you need memory that outlives one chat UI, this is the core reason to test it.

---

## How it maps to real workflows

HOM Local gives your app a durable memory API while keeping model providers at the app layer.

- Build agent harnesses with persistent recall state.
- Preserve context when you switch providers or run long sessions.
- Route app outputs, evidence, and model responses into memory records.
- Run continuity-aware workflows from compaction artifacts, not ad-hoc summaries.

---

## What it does

- ✅ Durable local memory in SQLite with WAL mode
- ✅ Source-attributed recall with quality scoring
- ✅ Four-wall quality-gated memory saves (structure, relevance, substance, factuality)
- ✅ Tamper-evident append-only ledger with hash-chain verification
- ✅ Context packing with evidence cards and open handles
- ✅ Session compaction artifacts that remain durable and inspectable
- ✅ Product Quantization vector recall with exact rerank fallback
- ✅ Ed25519 envelope authentication
- ✅ JSON-RPC over Unix domain socket + HTTP ingress on `127.0.0.1:9101`
- ✅ Tests, examples, and operator guidance

### Recall modes

- Exact identifier recall
- Temporal recall with bounded windows
- Session continuity recall
- Semantic (vector) recall
- Procedural reasoning recall

### Diagnostics and security

- Recall quality and coverage diagnostics
- Anti-drift checks and enforcement
- Quality gates + audit trail protections

---

## What HOM Local is not

- a finished consumer chat app  
- a model provider framework  
- a hosted memory service  
- a replacement for your product-specific agent harness

It is a backend memory brain your app can compose with.

---

## Provider boundary and model choice

HOM Local does not own model providers.

Your app decides provider and runtime behavior.
HOM Local owns memory, recall, validation, compaction, and audit.

This is intentionally separated so memory remains stable while provider strategy evolves.

---

## Post-compaction as a memory artifact

Most systems convert long context into a temporary summary and move on.

HOM Local persists post-compaction summaries as continuity artifacts with links back to source sessions and memories.

So continuity is recoverable, inspectable, and reusable instead of being discarded.

---

## Fast start

From a fresh clone:

```bash
git clone https://github.com/wallidsaydi-creator/hom-local.git
cd hom-local
cargo build --release
```

Run each service in its own terminal:

```bash
cargo run --release --bin hom-brain
```

```bash
cargo run --release --bin hom-ingress
```

Your ingress should be available at `http://127.0.0.1:9101`.

---

## Install

```bash
cargo install hom-brain
```

---

## Architecture

```text
hom-local/
├── crates/
│   ├── hom-brain/     # Core brain daemon — memory, ledger, recall, quality gates
│   ├── hom-shared/    # Shared types, crypto, envelope, RPC, paths
│   └── hom-ingress/   # HTTP ingress layer with auth
├── docs/              # Architecture and API documentation
└── examples/          # Usage examples
```

For the full service graph, see the Oracle Architecture Map — HOM Local inherits the proven memory architecture.

---

## Configuration

The brain daemon reads configuration from:

```text
~/.hom/config.json — Main configuration
Environment variables — HOM_* prefix
Command line arguments — See hom-brain --help
```

---

## Development

```bash
# Run tests
cargo test --workspace

# Run example tests
cargo test --workspace --examples

# Format code
cargo fmt

# Build docs
cargo doc --open
```

---

## Documentation

| Document | Description |
|---|---|
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

---

## License

HOM Local is licensed under the Apache License, 2.0.

This license applies only to the code in this repository.

HOM Oracle is a separate private system and is not included in this repository.

See [LICENSE](LICENSE) for the full license text.
