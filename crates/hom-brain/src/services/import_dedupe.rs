use hom_shared::RpcError;

pub fn scan_for_duplicates(state: &crate::BrainState, batch_id: &str) -> Result<(), RpcError> {
    let candidates = state
        .store
        .get_import_candidates_by_batch(batch_id, None, 10000)?;
    let ready_candidates: Vec<_> = candidates
        .iter()
        .filter(|c| c.candidate_status == "ready" || c.candidate_status == "needs_review")
        .collect();

    for candidate in &ready_candidates {
        let search_key = candidate.title.as_deref().unwrap_or(&candidate.body);
        let search_term = if search_key.len() > 80 {
            &search_key[..80]
        } else {
            search_key
        };

        let recall_result = state.store.recall(search_term, 5);

        if let Ok(result) = recall_result {
            if let Some(hits) = result.get("hits").and_then(|h| h.as_array()) {
                for hit in hits {
                    let hit_value = hit.get("value").and_then(|v| v.as_str()).unwrap_or("");
                    let hit_key = hit.get("key").and_then(|v| v.as_str()).unwrap_or("");
                    let hit_id = hit.get("memory_id").and_then(|v| v.as_str()).unwrap_or("");

                    if similar_enough(&candidate.body, hit_value) {
                        state.store.update_import_candidate_status(
                            &candidate.candidate_id,
                            "duplicate_candidate",
                            Some(hit_id),
                            None,
                        )?;
                        break;
                    }

                    if same_topic(&candidate.title, hit_key)
                        && different_content(&candidate.body, hit_value)
                    {
                        state.store.update_import_candidate_status(
                            &candidate.candidate_id,
                            "contradiction_candidate",
                            None,
                            Some(hit_id),
                        )?;
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

fn similar_enough(a: &str, b: &str) -> bool {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();
    if a_lower == b_lower {
        return true;
    }
    let a_words: std::collections::HashSet<&str> = a_lower.split_whitespace().collect();
    let b_words: std::collections::HashSet<&str> = b_lower.split_whitespace().collect();
    if a_words.is_empty() || b_words.is_empty() {
        return false;
    }
    let intersection = a_words.intersection(&b_words).count() as f64;
    let union = a_words.union(&b_words).count() as f64;
    let jaccard = intersection / union;
    jaccard > 0.85
}

fn same_topic(title: &Option<String>, key: &str) -> bool {
    match title {
        Some(t) => {
            let t_lower = t.to_lowercase();
            let k_lower = key.to_lowercase();
            t_lower == k_lower || t_lower.contains(&k_lower) || k_lower.contains(&t_lower)
        }
        None => false,
    }
}

fn different_content(a: &str, b: &str) -> bool {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();
    if a_lower == b_lower {
        return false;
    }
    let a_words: std::collections::HashSet<&str> = a_lower.split_whitespace().collect();
    let b_words: std::collections::HashSet<&str> = b_lower.split_whitespace().collect();
    if a_words.is_empty() || b_words.is_empty() {
        return true;
    }
    let intersection = a_words.intersection(&b_words).count() as f64;
    let union = a_words.union(&b_words).count() as f64;
    let jaccard = intersection / union;
    jaccard < 0.3
}
