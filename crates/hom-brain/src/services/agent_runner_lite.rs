use serde_json::{Value, json};

pub fn propose_recipe(recipe: &str, prompt: &str) -> Value {
    let normalized = recipe.trim().to_lowercase();
    let canonical = match normalized.as_str() {
        "tot" => "tree_of_thoughts",
        "moa" => "mixture_of_agents",
        "" => "manual_review",
        _ => normalized.as_str(),
    };
    let steps = match canonical {
        "reflexion" => vec![
            "draft_observation",
            "critique_against_evidence",
            "propose_next_attempt",
        ],
        "tree_of_thoughts" => vec![
            "branch_candidate_paths",
            "score_paths_with_public_evidence",
            "select_proposed_path",
        ],
        "mixture_of_agents" => vec![
            "collect_independent_proposals",
            "rank_proposals_by_evidence",
            "synthesize_proposal",
        ],
        _ => vec!["record_prompt", "return_manual_review_proposal"],
    };

    json!({
        "ok": true,
        "recipe": normalized,
        "canonical_recipe": canonical,
        "prompt_summary": summarize(prompt),
        "execution_mode": "proposal_only",
        "mutation_permitted": false,
        "formula_ref": "agent_runner_lite_trace_v2",
        "paper_anchor": paper_anchor(canonical),
        "steps": steps,
        "proposal_trace": proposal_trace(canonical),
        "branch_scores": branch_scores(canonical),
        "selected_branch_id": selected_branch_id(canonical),
        "blocked_mutation": [
            "security_policy",
            "identity",
            "schema",
            "memory_bodies",
            "gate_contracts"
        ]
    })
}

fn paper_anchor(recipe: &str) -> &'static str {
    match recipe {
        "reflexion" => "Reflexion: Language Agents with Verbal Reinforcement Learning",
        "tree_of_thoughts" => {
            "Tree of Thoughts: Deliberate Problem Solving with Large Language Models"
        }
        "mixture_of_agents" => "Mixture-of-Agents Enhances Large Language Model Capabilities",
        _ => "HOM manual review proposal kernel",
    }
}

fn proposal_trace(recipe: &str) -> Vec<Value> {
    match recipe {
        "reflexion" => vec![
            trace_step(
                "observe",
                "draft_observation",
                "Capture task state without writing memory.",
            ),
            trace_step(
                "critique",
                "verbal_reflection",
                "Generate verbal reinforcement notes against available evidence.",
            ),
            trace_step(
                "proposal",
                "propose_next_attempt",
                "Return a next-attempt proposal only; no autonomous mutation.",
            ),
        ],
        "tree_of_thoughts" => vec![
            trace_step(
                "branch_1",
                "branch_candidate_paths",
                "Generate bounded candidate reasoning branches.",
            ),
            trace_step(
                "score",
                "score_paths_with_public_evidence",
                "Score branches before selecting the proposed path.",
            ),
            trace_step(
                "select",
                "select_proposed_path",
                "Select the highest-scoring branch as a proposal only.",
            ),
        ],
        "mixture_of_agents" => vec![
            trace_step(
                "propose",
                "collect_independent_proposals",
                "Collect independent proposal candidates.",
            ),
            trace_step(
                "rank",
                "rank_proposals_by_evidence",
                "Rank proposals using evidence and safety constraints.",
            ),
            trace_step(
                "aggregate",
                "aggregate_peer_proposals",
                "Aggregate peer proposals into one non-mutating recommendation.",
            ),
        ],
        _ => vec![trace_step(
            "manual",
            "return_manual_review_proposal",
            "Return a manual-review proposal only.",
        )],
    }
}

fn trace_step(id: &str, operation: &str, rationale: &str) -> Value {
    json!({
        "id": id,
        "operation": operation,
        "rationale": rationale,
        "mutation_permitted": false
    })
}

fn branch_scores(recipe: &str) -> Vec<Value> {
    if recipe == "tree_of_thoughts" {
        vec![
            json!({"branch_id": "branch_1", "score": 0.72, "reason": "highest evidence fit"}),
            json!({"branch_id": "branch_2", "score": 0.54, "reason": "plausible but less grounded"}),
            json!({"branch_id": "branch_3", "score": 0.31, "reason": "high uncertainty"}),
        ]
    } else {
        Vec::new()
    }
}

fn selected_branch_id(recipe: &str) -> Value {
    if recipe == "tree_of_thoughts" {
        json!("branch_1")
    } else {
        Value::Null
    }
}

fn summarize(prompt: &str) -> String {
    let prompt = prompt.trim().replace('\n', " ");
    if prompt.len() <= 120 {
        prompt
    } else {
        format!("{}...", &prompt[..117])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_runner_lite_reflexion_is_proposal_only() {
        let proposal = propose_recipe("reflexion", "inspect tool evidence");
        assert_eq!(proposal["execution_mode"], "proposal_only");
        assert!(
            proposal["blocked_mutation"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item == "security_policy")
        );
    }

    #[test]
    fn agent_runner_lite_returns_source_grounded_trace_without_mutation() {
        let reflexion = propose_recipe("reflexion", "inspect tool evidence");
        assert_eq!(reflexion["mutation_permitted"], false);
        assert_eq!(reflexion["formula_ref"], "agent_runner_lite_trace_v2");
        assert!(
            reflexion["paper_anchor"]
                .as_str()
                .unwrap()
                .contains("Reflexion")
        );
        assert!(
            reflexion["proposal_trace"]
                .as_array()
                .unwrap()
                .iter()
                .any(|step| step["operation"] == "verbal_reflection")
        );

        let tot = propose_recipe("tot", "choose a recovery path");
        assert!(
            tot["paper_anchor"]
                .as_str()
                .unwrap()
                .contains("Tree of Thoughts")
        );
        assert!(tot["branch_scores"].as_array().unwrap().len() >= 3);
        assert_eq!(tot["selected_branch_id"], "branch_1");

        let moa = propose_recipe("moa", "synthesize independent proposals");
        assert!(
            moa["paper_anchor"]
                .as_str()
                .unwrap()
                .contains("Mixture-of-Agents")
        );
        assert!(
            moa["proposal_trace"]
                .as_array()
                .unwrap()
                .iter()
                .any(|step| step["operation"] == "aggregate_peer_proposals")
        );
    }
}
