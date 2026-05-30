# Recall System

## Overview

HOM Local provides source-attributed recall with multiple search modes, quality gates, and audit guidance.

## Recall modes

### Text recall

Full-text search across memory keys and values:

```
memory.recall(query="architecture boundaries", limit=5)
```

### Vector recall

Approximate vector search using Product Quantization:

```
memory.recall(
  query="architecture boundaries",
  embedding_model="text-embedding-3-small",
  query_vector=[0.1, 0.2, ...],
  limit=5
)
```

### Hybrid recall

Combined text and vector search with Reciprocal Rank Fusion:

```
memory.recall(
  query="architecture boundaries",
  embedding_model="text-embedding-3-small",
  query_vector=[0.1, 0.2, ...],
  limit=5,
  candidate_pool_size=10,
  top_k=5
)
```

### Smart recall

Intelligent recall with planner and multi-strategy:

```
memory.recall.smart(
  query="architecture boundaries",
  limit=5,
  current_session_source="ingress",
  project_id="project-alpha"
)
```

## Recall pipeline

```
Query → Parse → Mode Select → Retrieve → Score → Rank → Pack → Audit
```

### Mode selection

The recall planner selects the optimal mode:

1. **Small corpus (< 10K)**: Use exact text search
2. **Large corpus + embeddings**: Use hybrid PQ + exact rerank
3. **No embeddings**: Fall back to text search
4. **Threshold missing**: Block hybrid until profile exists

### Scoring

Memories are scored using multiple factors:

- **Recall score**: Initial retrieval relevance
- **Quality score**: Memory quality assessment
- **Freshness**: Time-based decay (QMD)
- **Mode trust**: Search mode reliability
- **Source authority**: Origin credibility
- **Citation coverage**: Evidence support

### Ranking

Results are ranked using Reciprocal Rank Fusion:

```
RRF_score = 1/(k + rank_text) + 1/(k + rank_vector)
```

Where `k = 60` (default constant).

## Context packing

The context packer assembles recall results into evidence cards:

### Evidence cards

```json
{
  "memory_id": "uuid",
  "key": "architecture:boundaries",
  "value": "The brain owns memory...",
  "score": 0.92,
  "components": {
    "mode_ranks": [{"mode": "text", "rank": 1}],
    "evidence_precision": 0.95
  },
  "open_handles": ["memory:recall:0", "memory:recall:1"]
}
```

### Budget management

Context packing respects token budgets:

```json
{
  "context_window_tokens": 200000,
  "reserved_output_tokens": 8000,
  "input_tokens_estimated": 142000,
  "available_input_tokens": 192000
}
```

## Vector search

### Product Quantization

HOM Local uses Product Quantization for approximate vector search:

1. **Codebook training**: K-means clustering on embedding vectors
2. **Encoding**: Vectors encoded as codebook indices
3. **Distance estimation**: Asymmetric distance computation (ADC)
4. **Candidate generation**: PQ-based approximate nearest neighbors
5. **Exact rerank**: Cosine similarity on candidate pool

### Threshold profiles

Vector search activation requires threshold profiles:

```json
{
  "min_recall_at_k": 0.95,
  "min_candidate_pool_hit_rate": 0.95,
  "require_top1_preserved": true
}
```

### Regression profiles

Multi-query regression testing ensures quality:

```json
{
  "dataset": {"dataset_id": "test-fixture-v1"},
  "queries": [
    {"query_id": "left-family", "query_vector": [0.95, 0.05, 0.9, 0.1]},
    {"query_id": "up-family", "query_vector": [0.05, 0.95, 0.1, 0.9]}
  ]
}
```

## Audit guidance

Every recall attaches audit metadata:

```json
{
  "recall_audit": {
    "audit_kind": "recall_action_audit_v1",
    "decision": "aligned",
    "mutation_permitted": false,
    "user_facing_mode": false,
    "drift_score": 0.12
  }
}
```

### Drift detection

The recall system detects rank drift:

- **Aligned**: No significant drift detected
- **Plan drift**: Rank changes exceed threshold
- **Context injection recommended**: Drift requires attention
