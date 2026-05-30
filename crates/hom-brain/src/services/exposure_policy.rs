use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExposureSurface {
    Callable,
    DiagnosticOnly,
    InternalCallable,
    InternalOnly,
    Deferred,
}

impl ExposureSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            ExposureSurface::Callable => "callable",
            ExposureSurface::DiagnosticOnly => "diagnostic_only",
            ExposureSurface::InternalCallable => "internal_callable",
            ExposureSurface::InternalOnly => "internal_only",
            ExposureSurface::Deferred => "deferred",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExposurePolicyEntry {
    pub kernel: &'static str,
    pub method: &'static str,
    pub surface: ExposureSurface,
    pub mutation_permitted: bool,
    pub requires_preflight: bool,
    pub rationale: &'static str,
}

const ENTRIES: &[ExposurePolicyEntry] = &[
    ExposurePolicyEntry {
        kernel: "memory_save",
        method: "memory.save",
        surface: ExposureSurface::Callable,
        mutation_permitted: true,
        requires_preflight: true,
        rationale: "Durable memory write; protected by quality/security/preflight gates.",
    },
    ExposurePolicyEntry {
        kernel: "memory_recall",
        method: "memory.recall",
        surface: ExposureSurface::Callable,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Default recall is read-only; vector mode only activates with explicit query_vector.",
    },
    ExposurePolicyEntry {
        kernel: "embedding_store",
        method: "memory.embedding.upsert",
        surface: ExposureSurface::InternalCallable,
        mutation_permitted: false,
        requires_preflight: true,
        rationale: "Persists derived embedding vectors but does not mutate memory bodies or architecture.",
    },
    ExposurePolicyEntry {
        kernel: "exact_vector_recall",
        method: "memory.recall",
        surface: ExposureSurface::InternalCallable,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Read-only recall mode enabled only when caller supplies embedding_model and query_vector.",
    },
    ExposurePolicyEntry {
        kernel: "native_hybrid_vector_recall",
        method: "memory.recall",
        surface: ExposureSurface::Callable,
        mutation_permitted: true,
        requires_preflight: true,
        rationale: "Phase E native hybrid vector recall is enabled through memory.recall when policy, threshold, audit, and query-vector guardrails pass.",
    },
    ExposurePolicyEntry {
        kernel: "exact_vector_baseline_benchmark",
        method: "benchmarks.vector_exact",
        surface: ExposureSurface::DiagnosticOnly,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Read-only exact cosine baseline benchmark used as the truth path for ANN/PQ/hybrid promotion decisions.",
    },
    ExposurePolicyEntry {
        kernel: "pq_candidate_generation",
        method: "benchmarks.vector_pq_candidates",
        surface: ExposureSurface::DiagnosticOnly,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Read-only PQ/ADC candidate generation; returns approximate candidate pool only and requires exact rerank before any live ranking use.",
    },
    ExposurePolicyEntry {
        kernel: "pq_exact_rerank",
        method: "benchmarks.vector_pq_exact_rerank",
        surface: ExposureSurface::DiagnosticOnly,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Read-only exact cosine rerank over a PQ/ADC candidate pool; exact rerank is bounded to the candidate set and is used by Phase E native hybrid recall after policy/audit gates pass.",
    },
    ExposurePolicyEntry {
        kernel: "vector_hybrid_threshold_profile",
        method: "benchmarks.vector_thresholds",
        surface: ExposureSurface::DiagnosticOnly,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Read-only benchmark threshold profile for recall@k, candidate hit rate, top1 preservation, and advisory latency before any native pipeline activation.",
    },
    ExposurePolicyEntry {
        kernel: "vector_hybrid_regression_profile",
        method: "benchmarks.vector_regression_profile",
        surface: ExposureSurface::DiagnosticOnly,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Multi-query vector benchmark profile for p50/p95/p99 latency, recall pass rates, and regression dataset summaries; validates the enabled guarded product surface.",
    },
    ExposurePolicyEntry {
        kernel: "vector_recall_policy",
        method: "policy.vector_recall",
        surface: ExposureSurface::DiagnosticOnly,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Inspectable policy object for corpus trigger, benchmark threshold gate, audit guardrails, and native recall activation state.",
    },
    ExposurePolicyEntry {
        kernel: "product_quantization_adc",
        method: "benchmarks.vector_recall",
        surface: ExposureSurface::DiagnosticOnly,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "PQ/ADC is benchmarked against exact vector search and is not a live ranking replacement.",
    },
    ExposurePolicyEntry {
        kernel: "online_turboquant_adaptation",
        method: "none",
        surface: ExposureSurface::Deferred,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Deferred by Walid; no online TurboQuant adaptation is wired in this phase.",
    },
    ExposurePolicyEntry {
        kernel: "route_certificate_registry",
        method: "route.certificate.issue",
        surface: ExposureSurface::InternalCallable,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Issues immutable route certificates for method/provider/model/capability evidence; does not mutate memories or architecture.",
    },
    ExposurePolicyEntry {
        kernel: "route_certificate_registry",
        method: "route.certificate.open",
        surface: ExposureSurface::DiagnosticOnly,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Reads route certificate evidence for audit and bridge verification.",
    },
    ExposurePolicyEntry {
        kernel: "diagnostics_recall",
        method: "diagnostics.recall",
        surface: ExposureSurface::DiagnosticOnly,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Read-only diagnostic envelope over recall results.",
    },
    ExposurePolicyEntry {
        kernel: "paper_registry",
        method: "paper.registry",
        surface: ExposureSurface::DiagnosticOnly,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Read-only source/status registry.",
    },
    ExposurePolicyEntry {
        kernel: "agent_runner_lite",
        method: "none",
        surface: ExposureSurface::InternalOnly,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Proposal trace kernel only; no executor exposure.",
    },
    ExposurePolicyEntry {
        kernel: "dream_feedback",
        method: "none",
        surface: ExposureSurface::InternalOnly,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Bounded feedback scoring kernel; not exposed as autonomous mutation.",
    },
    ExposurePolicyEntry {
        kernel: "spaced_repetition_lite",
        method: "none",
        surface: ExposureSurface::InternalOnly,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Schedule-state kernel remains internal until durable review productization.",
    },
    ExposurePolicyEntry {
        kernel: "meta_ledger_mutation",
        method: "ledger.record_mutation",
        surface: ExposureSurface::InternalCallable,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Records self-mutation events on the meta-ledger; does not mutate memory bodies or architecture.",
    },
    ExposurePolicyEntry {
        kernel: "meta_ledger_reconciliation",
        method: "ledger.reconcile_mismatches",
        surface: ExposureSurface::InternalCallable,
        mutation_permitted: false,
        requires_preflight: false,
        rationale: "Reconciles pre-existing hash mismatches as tracked self-mutations; read-only verification with optional write of reconciliation records.",
    },
];

pub fn list(params: Value) -> Value {
    let surface_filter = params.get("surface").and_then(Value::as_str);
    let items = ENTRIES
        .iter()
        .filter(|entry| surface_filter.is_none_or(|surface| entry.surface.as_str() == surface))
        .map(entry_json)
        .collect::<Vec<_>>();
    json!({
        "ok": true,
        "policy": "hom_kernel_exposure_policy_v1",
        "items": items,
        "guardrails": {
            "no_fake_embedding_generation": true,
            "native_hybrid_recall_enabled_with_guardrails": true,
            "online_turboquant_deferred": true,
            "architecture_mutation_permitted": false
        }
    })
}

fn entry_json(entry: &ExposurePolicyEntry) -> Value {
    json!({
        "kernel": entry.kernel,
        "method": entry.method,
        "surface": entry.surface.as_str(),
        "mutation_permitted": entry.mutation_permitted,
        "requires_preflight": entry.requires_preflight,
        "rationale": entry.rationale
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposure_policy_marks_vector_phase_e_enabled_guardrails_without_live_turboquant_claim() {
        let policy = list(json!({}));
        let items = policy["items"].as_array().unwrap();
        assert!(items.iter().any(|item| {
            item["kernel"] == "exact_vector_recall"
                && item["method"] == "memory.recall"
                && item["surface"] == "internal_callable"
        }));
        assert!(items.iter().any(|item| {
            item["kernel"] == "native_hybrid_vector_recall"
                && item["method"] == "memory.recall"
                && item["surface"] == "callable"
                && item["mutation_permitted"] == true
                && item["requires_preflight"] == true
        }));
        assert!(items.iter().any(|item| {
            item["kernel"] == "exact_vector_baseline_benchmark"
                && item["method"] == "benchmarks.vector_exact"
                && item["surface"] == "diagnostic_only"
        }));
        assert!(items.iter().any(|item| {
            item["kernel"] == "pq_candidate_generation"
                && item["method"] == "benchmarks.vector_pq_candidates"
                && item["surface"] == "diagnostic_only"
        }));
        assert!(items.iter().any(|item| {
            item["kernel"] == "pq_exact_rerank"
                && item["method"] == "benchmarks.vector_pq_exact_rerank"
                && item["surface"] == "diagnostic_only"
        }));
        assert!(items.iter().any(|item| {
            item["kernel"] == "vector_hybrid_threshold_profile"
                && item["method"] == "benchmarks.vector_thresholds"
                && item["surface"] == "diagnostic_only"
        }));
        assert!(items.iter().any(|item| {
            item["kernel"] == "vector_recall_policy"
                && item["method"] == "policy.vector_recall"
                && item["surface"] == "diagnostic_only"
        }));
        assert!(items.iter().any(|item| {
            item["kernel"] == "product_quantization_adc"
                && item["method"] == "benchmarks.vector_recall"
                && item["surface"] == "diagnostic_only"
        }));
        assert!(items.iter().any(|item| {
            item["kernel"] == "online_turboquant_adaptation" && item["surface"] == "deferred"
        }));
        assert_eq!(policy["guardrails"]["online_turboquant_deferred"], true);
        assert_eq!(
            policy["guardrails"]["native_hybrid_recall_enabled_with_guardrails"],
            true
        );
    }
}
