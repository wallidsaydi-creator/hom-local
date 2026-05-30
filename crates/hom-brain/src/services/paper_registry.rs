use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PaperRegistryEntry {
    service: &'static str,
    paper_anchor: &'static str,
    status: &'static str,
    evidence_path: &'static str,
}

const ENTRIES: &[PaperRegistryEntry] = &[
    PaperRegistryEntry {
        service: "RankingService RRF k=60",
        paper_anchor: "Cormack et al. 2009 / RRF anchor",
        status: "implemented",
        evidence_path: "crates/hom-brain/src/services/ranking_service.rs",
    },
    PaperRegistryEntry {
        service: "TemporalRecall QMD / Chronos-lite",
        paper_anchor: "Chronos / TiMem anchors",
        status: "implemented",
        evidence_path: "crates/hom-brain/src/services/recall_temporal.rs",
    },
    PaperRegistryEntry {
        service: "LineageRecall PPR",
        paper_anchor: "HippoRAG / MAGMA anchors",
        status: "implemented",
        evidence_path: "crates/hom-brain/src/services/recall_lineage.rs",
    },
    PaperRegistryEntry {
        service: "HippographBuilder",
        paper_anchor: "HippoRAG / MAGMA anchors",
        status: "implemented",
        evidence_path: "crates/hom-brain/src/services/hippograph_builder.rs",
    },
    PaperRegistryEntry {
        service: "HippographRetriever graph_walk",
        paper_anchor: "HippoRAG / MAGMA anchors",
        status: "implemented",
        evidence_path: "crates/hom-brain/src/services/hippograph_retriever.rs",
    },
    PaperRegistryEntry {
        service: "HippographMaintenance",
        paper_anchor: "HippoRAG / MAGMA anchors",
        status: "implemented",
        evidence_path: "crates/hom-brain/src/services/hippograph_maintenance.rs",
    },
    PaperRegistryEntry {
        service: "ConceptGraph",
        paper_anchor: "HippoRAG concept-level retrieval family",
        status: "implemented",
        evidence_path: "crates/hom-brain/src/services/concept_graph.rs",
    },
    PaperRegistryEntry {
        service: "AnswerScorer confidence envelope",
        paper_anchor: "ALCE primary recovered: citation recall/precision over cited statements using NLI entailment",
        status: "partial",
        evidence_path: "crates/hom-brain/src/services/answer_scorer.rs;  ALCE Enabling Large Language Models to Generate Text with Citations.pdf",
    },
    PaperRegistryEntry {
        service: "SourceAttributedSynthesis",
        paper_anchor: "RARR + ALCE primaries recovered: research evidence, revise unsupported content, preserve original content, cite supported statements",
        status: "partial",
        evidence_path: "crates/hom-brain/src/services/source_attributed_synthesis.rs; RARR - Researching and Revising What Language Models Say Using Language Models.pdf;  ALCE Enabling Large Language Models to Generate Text with Citations.pdf",
    },
    PaperRegistryEntry {
        service: "EvidenceAtomService",
        paper_anchor: "FActScore primary recovered: factual precision as supported atomic facts / total atomic facts",
        status: "partial",
        evidence_path: "crates/hom-brain/src/services/evidence_atom.rs; FActScore - Fine-grained Atomic Evaluation of Factual Precision in Long Form Text Generation.pdf",
    },
    PaperRegistryEntry {
        service: "QualityGate / ProtectedSave",
        paper_anchor: "FActScore primary recovered: factuality wall can use atomic fact support precision when supplied",
        status: "partial",
        evidence_path: "crates/hom-brain/src/services/quality_gate.rs; FActScore - Fine-grained Atomic Evaluation of Factual Precision in Long Form Text Generation.pdf",
    },
    PaperRegistryEntry {
        service: "TrustScoreService",
        paper_anchor: "FActScore primary recovered: trust formula includes factual precision/evidence support components",
        status: "partial",
        evidence_path: "crates/hom-brain/src/services/trust_score.rs; FActScore - Fine-grained Atomic Evaluation of Factual Precision in Long Form Text Generation.pdf",
    },
    PaperRegistryEntry {
        service: "RecallDiagnostics",
        paper_anchor: "ALCE + FActScore primaries recovered: citation support, citation precision, and atomic factual precision diagnostics",
        status: "partial",
        evidence_path: "crates/hom-brain/src/services/diagnostics.rs;  ALCE Enabling Large Language Models to Generate Text with Citations.pdf; FActScore - Fine-grained Atomic Evaluation of Factual Precision in Long Form Text Generation.pdf",
    },
    PaperRegistryEntry {
        service: "KLDistributionAlignment",
        paper_anchor: "ReAlign anchor recovered",
        status: "implemented",
        evidence_path: "crates/hom-brain/src/services/kl_alignment.rs",
    },
    PaperRegistryEntry {
        service: "ContrastiveRetraining",
        paper_anchor: "ReAlign anchor recovered",
        status: "implemented",
        evidence_path: "crates/hom-brain/src/services/contrastive_retraining.rs",
    },
    PaperRegistryEntry {
        service: "AutoRetrievalWeightUpdate",
        paper_anchor: "ReAlign anchor recovered",
        status: "implemented",
        evidence_path: "crates/hom-brain/src/services/auto_weight_update.rs",
    },
    PaperRegistryEntry {
        service: "SpacedRepetitionLite",
        paper_anchor: "SuperMemo 2 primary source verified: Piotr Wozniak, 1990; interval schedule I(1)=1, I(2)=6, n>2 ceil(I(n-1)*EF), EF update formula and 1.3 floor",
        status: "partial",
        evidence_path: "crates/hom-brain/src/services/spaced_repetition.rs; https://super-memory.com/english/ol/sm2.htm",
    },
    PaperRegistryEntry {
        service: "DreamFeedback",
        paper_anchor: "Self-RAG primary verified: adaptive retrieval, generation critique, and reflection tokens; HOM Reasoning Graphs anchor recovered for evidence-centric feedback",
        status: "partial",
        evidence_path: "crates/hom-brain/src/services/dream_feedback.rs; arXiv:2310.11511; Reasoning Graphs- Self-Improving, Deterministic RAG through Evidence-Centric Feedback.pdf",
    },
    PaperRegistryEntry {
        service: "AgentBDIState",
        paper_anchor: "Rao & Georgeff BDI source listed in architecture map",
        status: "implemented",
        evidence_path: "crates/hom-brain/src/services/agent_bdi_state.rs",
    },
    PaperRegistryEntry {
        service: "AgentRunnerLite",
        paper_anchor: "Reflexion primary recovered; Tree of Thoughts primary recovered; Mixture-of-Agents primary recovered; proposal-only kernel with no autonomous mutation",
        status: "partial",
        evidence_path: "crates/hom-brain/src/services/agent_runner_lite.rs; docs/papers/Reflexion.pdf; docs/papers/Tree_of_Thoughts.pdf; docs/papers/Mixture_of_Agents.pdf",
    },
    PaperRegistryEntry {
        service: "TurboQuant / Product Quantization",
        paper_anchor: "Vector substrate, exact recall integration, and Product Quantization ADC baseline implemented: persisted embeddings, exact cosine reference search, deterministic PQ codebooks/codes, approximate distance search, and exact-vs-PQ diagnostic benchmarking. Online TurboQuant adaptation remains future work.",
        status: "partial",
        evidence_path: "crates/hom-brain/src/services/vector_index.rs; crates/hom-brain/src/services/product_quantization.rs; crates/hom-brain/src/db/migrations.rs; crates/hom-brain/src/db/storage.rs; crates/hom-brain/src/services/exposure_policy.rs",
    },
];

pub fn list(params: Value) -> Value {
    let status_filter = params
        .get("status")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|status| !status.is_empty());
    let items = ENTRIES
        .iter()
        .filter(|entry| status_filter.is_none_or(|status| entry.status == status))
        .map(entry_json)
        .collect::<Vec<_>>();
    json!({
        "ok": true,
        "registry": "paper_source_registry_v1",
        "items": items,
        "count": items.len(),
        "source_doc": "docs/paper-source-registry.md"
    })
}

fn entry_json(entry: &PaperRegistryEntry) -> Value {
    json!({
        "service": entry.service,
        "paper_anchor": entry.paper_anchor,
        "status": entry.status,
        "evidence_path": entry.evidence_path
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_registry_lists_implemented_and_blocked_entries() {
        let registry = list(json!({}));
        assert_eq!(registry["ok"], true);
        let items = registry["items"].as_array().unwrap();
        assert!(
            items
                .iter()
                .any(|item| item["service"] == "HippographRetriever graph_walk")
        );
        assert!(items.iter().any(|item| {
            item["service"] == "TurboQuant / Product Quantization" && item["status"] == "partial"
        }));
    }

    #[test]
    fn paper_registry_filters_by_status() {
        let registry = list(json!({"status": "implemented"}));
        let items = registry["items"].as_array().unwrap();
        assert!(!items.is_empty());
        assert!(items.iter().all(|item| item["status"] == "implemented"));
    }

    #[test]
    fn paper_registry_surfaces_recovered_alce_rarr_factscore_primary_anchors() {
        let registry = list(json!({}));
        let items = registry["items"].as_array().unwrap();
        let joined = items
            .iter()
            .map(|item| format!("{} {}", item["paper_anchor"], item["evidence_path"]))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("ALCE primary recovered"));
        assert!(joined.contains("RARR + ALCE primaries recovered"));
        assert!(joined.contains("FActScore primary recovered"));
        assert!(
            joined.contains(
                "ALCE Enabling Large Language Models to Generate Text with Citations.pdf"
            )
        );
        assert!(joined.contains(
            "RARR - Researching and Revising What Language Models Say Using Language Models.pdf"
        ));
        assert!(joined.contains("FActScore - Fine-grained Atomic Evaluation of Factual Precision in Long Form Text Generation.pdf"));
    }

    #[test]
    fn paper_registry_surfaces_agent_selfrag_and_dream_feedback_anchors() {
        let registry = list(json!({}));
        let items = registry["items"].as_array().unwrap();
        let joined = items
            .iter()
            .map(|item| format!("{} {}", item["paper_anchor"], item["evidence_path"]))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("Reflexion primary recovered"));
        assert!(joined.contains("Tree of Thoughts primary recovered"));
        assert!(joined.contains("Mixture-of-Agents primary recovered"));
        assert!(joined.contains("Self-RAG primary verified"));
        assert!(joined.contains("arXiv:2310.11511"));
        assert!(joined.contains("Reasoning Graphs"));
    }

    #[test]
    fn paper_registry_surfaces_sm2_primary_anchor() {
        let registry = list(json!({}));
        let items = registry["items"].as_array().unwrap();
        let joined = items
            .iter()
            .map(|item| format!("{} {}", item["paper_anchor"], item["evidence_path"]))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("SuperMemo 2 primary source verified"));
        assert!(joined.contains("I(1)=1"));
        assert!(joined.contains("https://super-memory.com/english/ol/sm2.htm"));
    }
}
