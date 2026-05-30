# Changelog

All notable changes to HOM Local will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-05-30

### Added

- Core memory system with SQLite WAL storage
- Tamper-evident append-only ledger with hash chain
- Source-attributed recall with quality gates
- Context packing with evidence cards and open handles
- Product Quantization for approximate vector search
- Weighted priority queue system for worker dispatch
- JSON-RPC over Unix domain socket (brain IPC)
- Ed25519 envelope authentication for IPC
- Provider system with circuit breaker and streaming
- Session compaction with continuity guarantees
- Memory import/export for seed packs
- Comprehensive test suite
- Full API documentation

### Supported Providers

- OpenAI-compatible (any OpenAI API)
- Anthropic (Claude)
- Google (Gemini)
- Local models (Ollama, LM Studio)
- Codex OAuth (experimental)
