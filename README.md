# HOM Local

[![CI](https://github.com/wallidsaydi-creator/hom-local/actions/workflows/ci.yml/badge.svg)](https://github.com/wallidsaydi-creator/hom-local/actions)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![crates.io](https://img.shields.io/crates/v/hom-brain.svg)](https://crates.io/crates/hom-brain)
[![Star History](https://api.star-history.com/svg?repos=wallidsaydi-creator/hom-local&type=Date)](https://star-history.com/#wallidsaydi-creator/hom-local&Date)

**A local brain server for AI agents: durable memory, source-attributed recall, quality gates, audit ledger, context packing, and post-compaction summaries that become memory.**

*Stop paying the re-briefing tax. HOM Local gives your AI models a persistent brain: decisions, evidence, constraints, and project context that survive context windows, session resets, model switches, and handoffs.*

---

## Table of Contents

- [The problem](#the-problem)
- [Core guarantees](#core-guarantees)
- [Who is HOM Local for](#who-is-hom-local-for)
- [Competitive architecture](#competitive-architecture)
- [Why HOM Local vs. alternatives](#why-hom-local-vs-alternatives)
- [What it does](#what-it-does)
- [What HOM Local is not](#what-hom-local-is-not)
- [Model/provider boundary](#modelprovider-boundary)
- [Post-compaction summaries become memory](#post-compaction-summaries-become-memory)
- [Why this matters](#why-this-matters)
- [Why try HOM Local](#why-try-hom-local)
- [Fast start](#fast-start)
- [Quick start](#quick-start)
- [Install](#install)
- [Architecture](#architecture)
- [Configuration](#configuration)
- [Development](#development)
- [Documentation](#documentation)

---

## The Problem

Every serious AI task starts with a manual recap. Re-explaining context. Re-stating decisions. Re-establishing constraints. The "re-briefing tax" eats hours every week and makes AI work feel fragile.

Most agents rebuild context from scratch every run. Their memory is hidden, hosted, unverifiable, or trapped inside a single product with no portability.

**HOM Local takes the opposite approach: memory is a local server.**

Run the brain daemon on your machine. Connect your app over HTTP. Get durable memory, source-attributed recall, quality gates, an auditable ledger, and post-compaction summaries that become durable, source-linked memory.

---

## Core Guarantees

### 1. Recall

HOM Local stores structured memories and retrieves them with source attribution.
Agents can ask the local brain what it knows without relying only on chat history or opaque hosted memory.

### 2. Validation

Before information becomes memory, HOM Local runs quality gates: structure, relevance, substance, and factuality checks.
The goal is not to remember everything. The goal is to remember useful context with evidence.

### 3. Audit

Memory mutations are written to a tamper-evident local ledger.
Developers can inspect what changed, when it changed, and why the brain believes a piece of context exists.

### 4. Compaction

Long sessions can be compressed into durable continuity artifacts.
The compaction does not disappear into a prompt summary — it becomes memory that can be recalled, opened, and audited later.

## Who is HOM Local for

HOM Local is for teams building:

- AI agents and copilots that need durable, session-to-session memory.
- Multi-model stacks where providers and routing can change.
- Tool-augmented workflows where memory provenance and traceability matter.
- Local-first systems where memory logic is separate from chat UX.

---

## Competitive architecture

HOM Local is built as memory infrastructure, not an app wrapper:

- **Three-layer boundary:** `hom-ingress` handles HTTP and auth, `hom-brain` handles memory operations, and `hom-shared` carries protocol types and cryptography.
- **Deterministic service boundaries:** JSON-RPC over Unix domain sockets for brain APIs with an HTTP edge in ingress.
- **Workload isolation:** weighted priority queues separate save, recall, cognition, I/O, and main tasks.
- **Evidence chain:** each mutation is ledgered and linked to provenance.
- **Provider neutrality by design:** model/provider choice is separate from memory persistence.

```text
Client → hom-ingress (auth/routing) → hom-brain (memory/diagnostics/ledger) → SQLite WAL + vector index
```

## Why HOM Local vs. Alternatives

| | HOM Local | Mem0 | Mem.ai | Claude/GPT Memory | Brew / context7 |
|---|---|---|---|---|---|
| Local-first (no cloud required) | ✅ | ❌ | ❌ | ❌ | ❌ |
| Evidence-based recall with citations | ✅ | ❌ | ❌ | ❌ | ❌ |
| Provider-agnostic (switch models, brain survives) | ✅ | ❌ | ❌ | ❌ | ❌ |
| Open source | ✅ | ❌ | ❌ | ❌ | ✅ |
| Tamper-evident ledger with hash chain | ✅ | ❌ | ❌ | ❌ | ❌ |
| Post-compaction as first-class memory artifact | ✅ | ❌ | ❌ | ❌ | ❌ |
| Quality-gated memory (4-wall assessment) | ✅ | ❌ | ❌ | ❌ | ❌ |
| No vendor lock-in | ✅ | ❌ | ❌ | ❌ | ✅ |

**The angle nobody owns yet:** Provider-agnostic persistent memory — your AI's brain survives model switches, not just session resets. Switch from Claude to GPT to Ollama and your project context moves with you.

---

## What it does

- ✅ Durable local memory in SQLite with WAL mode
- ✅ Source-attributed recall with quality scoring
- ✅ Quality-gated memory save (four-wall assessment: structure, relevance, substance, factuality)
- ✅ Tamper-evident append-only ledger with hash chain verification
- ✅ Context packing with evidence cards and open handles
- ✅ Session compaction that becomes durable memory (not a disposable summary)
- ✅ Post-compaction summaries stored as source-linked continuity artifacts
- ✅ Product Quantization approximate vector search with exact rerank fallback
- ✅ Ed25519 envelope-based IPC authentication
- ✅ JSON-RPC over Unix domain socket + HTTP ingress on `127.0.0.1:9101`
- ✅ Synthetic tests and examples

### Recall modes

- Exact identifier recall
- Temporal recall (bounded time windows)
- Session continuity recall
- Semantic (vector) recall
- Procedural reasoning recall

### Compaction

Session compaction is treated as a first-class memory artifact — not a token-saving trick.
The summary is stored locally with links back to the source session and memories.
Future agents can recall it, open the supporting memories, inspect the provenance, and continue from durable continuity.

### Diagnostics

- Recall diagnostics (quality and coverage signals)
- Anti-drift rules (coverage enforcement)

### Security

- Ed25519 envelope-based IPC authentication
- Quality gate system (structure, relevance, substance, factuality)

---

## What HOM Local is not

HOM Local is not:
- a finished consumer app
- a chatbot
- a model provider framework
- a hosted memory service
- a replacement for your agent harness

It is the local backend brain your app can connect to.

---

## Model/Provider Boundary

HOM Local does not bundle provider implementations.

Providers belong in the app layer.
HOM Local owns the memory layer.

Your app chooses the model.
HOM Local stores, recalls, validates, packs, and audits memory.

---

## Post-Compaction Summaries Become Memory

Most agent systems compress long sessions into temporary summaries that vanish into the next prompt.

HOM Local treats the post-compaction summary as a first-class memory artifact.

After compaction, the summary is stored locally with links back to the source session and memories. Future agents can recall it, open the supporting memories, inspect the provenance, and continue from durable continuity instead of rebuilding context from scratch.

This makes compaction part of the memory system, not just a token-saving trick.

---

## Why This Matters

Agents need more than prompts and tool calls.

They need a memory layer that can answer:

- What do we know?
- Where did that memory come from?
- Was it validated before storage?
- Can the original source be opened?
- What changed in the brain over time?
- Can long sessions become durable continuity instead of disposable summaries?

HOM Local is built as that layer.

---

## Why try HOM Local

If your agent stack needs repeatable memory continuity, this is the part worth testing:

- keep memory data and provenance on your machine
- avoid hidden provider coupling in the memory layer
- preserve continuity across sessions and model/provider changes
- start building with auditable, source-linked context instead of prompt-only assumptions

If this direction helps your product, star the repo so it gets seen by more builders.

### Why starring this repo helps

- It keeps this local-first memory layer visible to other builders.
- It helps us attract contributors and early adopters.
- It makes project progress easier to discover for teams evaluating memory infrastructure.

## Fast Start

From a clean checkout:

```bash
git clone https://github.com/wallidsaydi-creator/hom-local.git
cd hom-local
cargo run --release --bin hom-brain &
cargo run --release --bin hom-ingress &
```

The two services should both be running and listening for app traffic.

## Quick Start

```bash
# Start the brain daemon
cargo run --release --bin hom-brain

# Start the ingress server
cargo run --release --bin hom-ingress

# The ingress listens on http://127.0.0.1:9101
# Connect your app and start saving/recalling memories.
```

## Install

```bash
cargo install hom-brain
```

Or clone and build from source:

```bash
git clone https://github.com/wallidsaydi-creator/hom-local.git
cd hom-local
cargo build --release
```

## Architecture

```text
hom-local/
├── crates/
│   ├── hom-brain/     # Core brain daemon — memory, ledger, recall, quality gates
│   ├── hom-shared/    # Shared types, crypto, envelope, RPC, paths
│   └── hom-ingress/   # HTTP ingress layer with auth
├── docs/              # Architecture and API documentation
└── examples/         # Usage examples
```

For the full service graph, see the Oracle Architecture Map — HOM Local inherits the proven memory architecture.

## Configuration

The brain daemon reads configuration from:

```text
~/.hom/config.json — Main configuration
Environment variables — HOM_* prefix
Command line arguments — See hom-brain --help
```

## Development

```bash
# Run tests
cargo test --workspace

# Run example tests
cargo test --workspace --examples

# Format code
cargo fmt

# Build documentation
cargo doc --open
```

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

## License

HOM Local is licensed under the Apache License, 2.0.

This license applies only to the code in this repository.

HOM Oracle is a separate private system and is not included in this repository.

See LICENSE for the full license text.
