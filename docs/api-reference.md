# API Reference

## Brain IPC (JSON-RPC over UDS)

### Memory operations

| Method | Description |
|--------|-------------|
| `memory.save` | Save a new memory |
| `memory.recall` | Recall memories by query |
| `memory.recall.smart` | Smart recall with planner |
| `memory.open` | Open a specific memory by ID |
| `memory.answer` | Source-attributed answer generation |
| `memory.embedding.upsert` | Upsert vector embedding |

### Session operations

| Method | Description |
|--------|-------------|
| `session.compact` | Compact session context |
| `session.compaction_snapshot` | Get compaction snapshot for UX |
| `session.context_pressure` | Report context pressure diagnostics |
| `session.compactions` | List compaction artifacts |
| `session.compaction.open` | Open a compaction artifact |

### Provider operations

| Method | Description |
|--------|-------------|
| `providers.model_catalog` | Get provider model catalog |
| `providers.list` | List available providers |

### Import/Export operations

| Method | Description |
|--------|-------------|
| `import.pack` | Import a memory pack |
| `import.list` | List import history |
| `import.memory.sources` | List import sources |
| `import.memory.batch.create` | Create import batch |
| `export.brain` | Export all brain data |
| `brain.backup` | Create brain backup |
| `brain.purge` | Purge brain data |

### Diagnostics operations

| Method | Description |
|--------|-------------|
| `runtime.status` | Get runtime status |
| `diagnostics.recall` | Recall diagnostics |
| `diagnostics.recall_drift` | Report recall drift |
| `ui.contract.snapshot` | Get UI contract snapshot |
| `ui.inspector.target` | Open inspector target |

### Reasoning operations

| Method | Description |
|--------|-------------|
| `reasoning.run` | Run reasoning on a memory |
| `reasoning.causal_chain` | Get causal chain for session |
| `reasoning.bridge.list` | List reasoning bridges |
| `reasoning.bridge.integrity` | Check bridge integrity |
| `reasoning.tool_event.record` | Record tool event |
| `reasoning.tool_quality` | Get tool quality metrics |

### Benchmark operations

| Method | Description |
|--------|-------------|
| `benchmarks.list` | List benchmarks |
| `benchmarks.detail` | Get benchmark detail |
| `benchmarks.record` | Record benchmark result |
| `benchmarks.vector_recall` | Vector recall benchmark |
| `benchmarks.vector_exact` | Exact vector search benchmark |
| `benchmarks.vector_pq_candidates` | PQ candidate generation benchmark |
| `benchmarks.vector_pq_exact_rerank` | PQ exact rerank benchmark |
| `benchmarks.vector_thresholds` | Vector threshold profile |
| `benchmarks.vector_regression_profile` | Regression profile |
| `benchmarks.vector_regression_compare` | Compare regression profiles |
| `benchmarks.vector_regression_response` | Regression response policy |

### Gate operations

| Method | Description |
|--------|-------------|
| `gates.evidence.submit` | Submit gate evidence |
| `gates.tasks.complete` | Complete gate task |
| `gates.argument.build` | Build argument |
| `gates.argument.inspect` | Inspect argument |
| `gates.argument.accept` | Accept argument |

### Permission operations

| Method | Description |
|--------|-------------|
| `permissions.set_profile` | Set permission profile |
| `permissions.grants.revoke` | Revoke permissions |

### Nightly operations

| Method | Description |
|--------|-------------|
| `nightly.dry_run` | Dry run nightly maintenance |
| `nightly.run` | Run nightly maintenance |
| `nightly.tree` | Get nightly tree |

### Policy operations

| Method | Description |
|--------|-------------|
| `exposure.policy` | Get exposure policy |
| `policy.vector_recall` | Get vector recall policy |

### Route certificate operations

| Method | Description |
|--------|-------------|
| `route.certificate.issue` | Issue route certificate |
| `route.certificate.open` | Open route certificate |

## HTTP API (Ingress)

### Brain routes (forwarded to brain IPC)

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/recall` | Memory recall |
| POST | `/api/save` | Save memory |
| GET | `/api/ui/runtime-status` | Runtime status |
| GET | `/api/ui/contract` | UI contract |
| GET | `/api/ui/recall/:id` | Open memory |
| GET | `/api/ui/reasoning/bridge-integrity` | Bridge integrity |
| POST | `/api/ui/session/compact` | Compact session |
| POST | `/api/ui/session/compaction-snapshot` | Compaction snapshot |
| GET | `/api/ui/benchmarks` | List benchmarks |
| GET | `/api/ui/benchmarks/:id` | Benchmark detail |

### Mesh routes (capability mesh)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/ui/providers` | List providers |
| GET | `/api/ui/providers/:id` | Get provider |
| GET | `/api/ui/providers/:id/models` | Get provider models |
| POST | `/api/ui/providers/discover` | Discover models |
| POST | `/api/ui/providers/:id/connect` | Connect provider |
| POST | `/api/ui/providers/preflight` | Provider preflight |
| GET | `/api/ui/tools` | List tools |
| POST | `/api/ui/tools/call` | Call tool |
| GET | `/api/ui/skills` | List skills |
| GET | `/api/ui/plugins` | List plugins |
| POST | `/api/ui/chat` | Chat completion |
