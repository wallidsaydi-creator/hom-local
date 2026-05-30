# HOM Local vs HOM Oracle

## What is HOM Local?

HOM Local is the open-source local-first AI memory kernel. It provides persistent, source-attributed recall for AI applications, storing memories in SQLite with tamper-evident ledger integrity.

## What is HOM Oracle?

HOM Oracle is a separate private system. It is not included in this repository.

## Licensing boundary

HOM Local is licensed under Apache-2.0.

HOM Oracle is not included in this repository and is not licensed under Apache-2.0 by this repository.

The public license does not grant rights to Oracle services, Oracle packs, private datasets, commercial intelligence layers, hosted infrastructure, private orchestration, or any repository not explicitly included in this public HOM Local release.

## What is included in this repository

- Brain daemon (memory storage, recall, quality gates)
- Ingress server (HTTP API)
- Provider system (OpenAI-compatible, Anthropic, Google, local models)
- Shared utilities (RPC, auth, types)
- Documentation, tests, and examples

## What is NOT included

- HOM Oracle services
- Oracle packs
- Private orchestration
- Private datasets
- Private commercial intelligence layers
- Private hosted services
- Private roadmap
- User memory data
- Exported brain state
- Internal/private repositories
