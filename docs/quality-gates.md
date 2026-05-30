# Quality Gates

## Overview

HOM Local implements a four-wall quality assessment system for memories, answers, and reasoning artifacts.

## Four-wall assessment

Every memory passes through four quality walls:

### 1. Form

Structural correctness of the memory:

- Valid UTF-8 encoding
- Appropriate length (not empty, not excessive)
- Correct memory type
- Valid metadata structure

### 2. Filter

Relevance and deduplication:

- Query relevance scoring
- Duplicate detection
- Source attribution verification
- Session/project scoping

### 3. Substance

Evidence strength and citation coverage:

- Evidence card quality
- Citation completeness
- Source authority
- Cross-reference validation

### 4. Factuality

Atomic precision via EvidenceAtom/FActScore-style evaluation:

- Claim decomposition into atomic facts
- Fact-level support scoring
- Unsupported claim penalty
- Confidence computation

## Quality scoring

Quality scores are computed as:

```
quality_score = form_score * filter_score * substance_score * factuality_score
```

Each score ranges from 0.0 to 1.0.

## Answer confidence

Answer confidence combines multiple factors:

```
confidence = clamp(0, 1,
    0.22 * evidence_strength +
    0.16 * query_coverage +
    0.18 * citation_coverage +
    0.18 * atom_precision +
    0.10 * freshness +
    0.08 * mode_trust +
    0.05 * source_authority +
    0.03 * calibration
) * unsupported_penalty
```

### Confidence components

| Component | Weight | Description |
|-----------|--------|-------------|
| evidence_strength | 0.22 | Quality of supporting evidence |
| query_coverage | 0.16 | How well the answer covers the query |
| citation_coverage | 0.18 | Completeness of citations |
| atom_precision | 0.18 | Atomic fact precision |
| freshness | 0.10 | Time-based relevance |
| mode_trust | 0.08 | Search mode reliability |
| source_authority | 0.05 | Origin credibility |
| calibration | 0.03 | Calibration adjustment |

## Gate tasks

Quality gates create audit trails:

```json
{
  "task_id": "uuid",
  "gate_kind": "quality_assessment",
  "subject_id": "memory_id",
  "subject_kind": "memory",
  "status": "pass",
  "evidence": [
    {
      "wall": "factuality",
      "score": 0.95,
      "atoms": ["fact-1", "fact-2"]
    }
  ]
}
```

## Reasoning bridges

Quality gates link reasoning artifacts:

- **Tool invocations**: Every tool call creates a bridge
- **Import events**: Import provenance tracked
- **Compaction artifacts**: Session continuity maintained
- **Nightly operations**: Maintenance audit trail

## Benchmark results

Quality assessments are recorded as benchmarks:

```json
{
  "benchmark_id": "uuid",
  "benchmark_kind": "tool_use",
  "subject_id": "brain.memory.recall",
  "subject_kind": "tool",
  "status": "pass",
  "severity": "production_safe",
  "evidence": [...]
}
```

## Nightly maintenance

Automated nightly runs assess quality:

1. **Tool quality scoring**: Wilson lower bound for reliability
2. **Bridge integrity**: Orphan detection and coverage
3. **Drift detection**: Recall rank stability
4. **Mutation selection**: Automated quality improvements

### Mutation guards

Mutations are blocked when:

- Tool observability is degraded
- Bridge coverage is insufficient
- Recall drift exceeds threshold
- Operator review is required
