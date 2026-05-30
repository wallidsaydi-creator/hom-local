# Content Queue

Pre-written social content for HOM Local. Post manually or schedule via automation.

---

## Launch Week

### Hacker News - Show HN

**Title**: `Show HN: HOM Local – open-source local-first AI memory server (Rust)`

**Body**:
```
Hey HN – I built HOM Local, a self-hosted memory server for AI applications.

The idea: your AI app connects to a local brain daemon over HTTP/IPC.
It stores memories in SQLite with WAL mode, enforces tamper-evident
ledger integrity, and provides quality-gated context packing for
downstream models.

Key features:
- Source-attributed recall with quality scoring
- Append-only ledger with hash chain verification
- Product Quantization for approximate vector search
- Four-wall quality gates (form, filter, substance, factuality)
- JSON-RPC over Unix domain socket + HTTP ingress

It's Apache-2.0, written in Rust, runs locally. No cloud, no API keys,
no vendor lock-in.

GitHub: https://github.com/wallidsaydi-creator/hom-local
Docs: https://wallidsaydi-creator.github.io/hom-local/
```

**Timing**: Tuesday or Wednesday, 9-11am EST

---

### Reddit r/rust

**Title**: `Show r/rust: HOM Local — open-source local-first AI memory server`

**Body**:
```
I built HOM Local, a self-hosted memory server for AI applications.

Architecture: 3 Rust crates in a workspace (resolver 3, edition 2024):
- hom-brain: core daemon with SQLite WAL, tamper-evident ledger, quality gates, vector search
- hom-shared: Ed25519 crypto, envelope auth, RPC types
- hom-ingress: HTTP server connecting apps to the brain over IPC

188 tests, CI/CD, cross-platform releases. Apache-2.0.

GitHub: https://github.com/wallidsaydi-creator/hom-local
Docs: https://wallidsaydi-creator.github.io/hom-local/
```

**Timing**: Wednesday

---

### Reddit r/LocalLLaMA

**Title**: `HOM Local: open-source local memory server for AI agents`

**Body**:
```
HOM Local is a local brain server for AI agents. Your app connects to a brain daemon over HTTP, and it stores memories in SQLite with WAL mode.

No cloud. No API keys. No vendor lock-in. Runs on your machine.

Key features:
- Source-attributed recall with quality scoring
- Tamper-evident append-only ledger
- Context packing for model-ready evidence cards
- Post-compaction summaries that become durable memory

GitHub: https://github.com/wallidsaydi-creator/hom-local
```

**Timing**: Thursday

---

### Reddit r/MachineLearning

**Title**: `P: HOM Local — self-hosted memory server with quality-gated recall`

**Body**:
```
HOM Local is a self-hosted memory server for AI applications. It provides:

- Quality-gated memory save: every memory is assessed for form, filter, substance, and factuality before storage
- Source-attributed recall: every memory hit includes provenance information
- Tamper-evident ledger: append-only with hash chain verification
- Product Quantization for approximate vector search with exact rerank fallback
- Context packing: recall results become model-ready evidence cards

Apache-2.0, Rust, runs locally.

GitHub: https://github.com/wallidsaydi-creator/hom-local
Docs: https://wallidsaydi-creator.github.io/hom-local/
```

**Timing**: Friday

---

### Twitter/X Thread

**Tweet 1**:
```
I built HOM Local — an open-source local AI memory server. Your app connects to a brain daemon over HTTP. It stores memories in SQLite, enforces tamper-evident ledger integrity, and provides quality-gated recall for LLMs. Apache-2.0. [link]
```

**Tweet 2**:
```
The architecture: 3 Rust crates. hom-brain (core daemon), hom-shared (crypto/types), hom-ingress (HTTP server). 188 tests, CI/CD, cross-platform releases. No cloud, no API keys.
```

**Tweet 3**:
```
What makes it different: four-wall quality gates. Every memory is assessed for form, filter, substance, and factuality before it enters the recall pipeline. Source attribution on every hit.
```

**Tweet 4**:
```
Try it: git clone, cargo build --release, run the brain + ingress, connect your app to localhost:9101. That's it. [link to quick start]
```

**Timing**: Friday

---

## Weekly Content (rotate)

### Feature Highlight 1: Quality Gates
Post about the four-wall assessment system. Explain form, filter, substance, factuality. Show how it prevents low-quality memories from entering the recall pipeline.

### Feature Highlight 2: Tamper-Evident Ledger
Post about the append-only ledger with hash chain verification. Explain how developers can audit what changed, when, and why.

### Feature Highlight 3: Context Packing
Post about how recall results become model-ready evidence cards with open handles for follow-up.

### Feature Highlight 4: Post-Compaction Memory
Post about how compaction summaries become durable, source-linked memory artifacts instead of disposable prompt text.

### Feature Highlight 5: Vector Search
Post about Product Quantization for approximate vector search with exact rerank fallback.

---

## Metrics to Track

| Metric | Source | Target |
|--------|--------|--------|
| GitHub stars | `gh api` | 100 in first month |
| crates.io downloads | crates.io API | Growing weekly |
| HN upvotes | Manual check | 50+ |
| Reddit upvotes | Manual check | 20+ per post |
| Twitter impressions | Twitter analytics | Growing |
| Issues opened | GitHub | Community engagement |
| PRs opened | GitHub | Community contributions |
