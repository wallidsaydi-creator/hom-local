use serde_json::Value;

pub const MAX_ATOMS_PER_MEMORY: usize = 10;

#[derive(Clone, Debug)]
pub struct Atom {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f64,
    pub atom_type: String,
}

pub fn extract_atoms(key: &str, value: &str, memory_type: &str, quality_score: f64) -> Vec<Atom> {
    let combined = format!("{key}\n{value}");
    let sentences = split_sentences(&combined);
    let mut atoms = Vec::new();
    for sentence in &sentences {
        if atoms.len() >= MAX_ATOMS_PER_MEMORY {
            break;
        }
        if let Some(atom) = extract_spo(sentence, memory_type, quality_score) {
            atoms.push(atom);
        }
    }
    atoms
}

pub fn atom_precision(atoms: &[Atom], evidence: &[Value]) -> f64 {
    if atoms.is_empty() {
        return 1.0;
    }
    let evidence_text: String = evidence
        .iter()
        .filter_map(|e| {
            let key = e.get("key").and_then(Value::as_str).unwrap_or("");
            let val = e.get("value").and_then(Value::as_str).unwrap_or("");
            Some(format!("{key} {val}").to_lowercase())
        })
        .collect::<Vec<_>>()
        .join(" ");
    if evidence_text.is_empty() {
        return 0.0;
    }
    let evidence_tokens = token_set(&evidence_text);
    let supported = atoms
        .iter()
        .filter(|atom| atom_support_score(atom, &evidence_tokens) >= 0.60)
        .count();
    supported as f64 / atoms.len() as f64
}

fn atom_support_score(atom: &Atom, evidence_tokens: &std::collections::HashSet<String>) -> f64 {
    let subject_match = phrase_tokens_supported(&atom.subject, evidence_tokens);
    let predicate_match = phrase_tokens_supported(&atom.predicate, evidence_tokens);
    let object_match = phrase_tokens_supported(&atom.object, evidence_tokens);

    let score = (if subject_match { 0.30 } else { 0.0 })
        + (if predicate_match { 0.20 } else { 0.0 })
        + (if object_match { 0.40 } else { 0.0 });

    if predicate_match && (subject_match || object_match) {
        score
    } else {
        0.0
    }
}

fn phrase_tokens_supported(
    phrase: &str,
    evidence_tokens: &std::collections::HashSet<String>,
) -> bool {
    let tokens = phrase_tokens(phrase);
    !tokens.is_empty() && tokens.iter().all(|token| evidence_tokens.contains(token))
}

fn token_set(text: &str) -> std::collections::HashSet<String> {
    phrase_tokens(text).into_iter().collect()
}

fn phrase_tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|token| {
            let token = token.trim().to_lowercase();
            (token.len() >= 2).then_some(token)
        })
        .collect()
}

fn split_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;
    for (i, ch) in text.char_indices() {
        if matches!(ch, '.' | '!' | '?') {
            let end = i + ch.len_utf8();
            let sentence = text[start..end].trim();
            if !sentence.is_empty() && sentence.len() >= 10 {
                sentences.push(sentence);
            }
            start = end;
        }
    }
    if start < text.len() {
        let remaining = text[start..].trim();
        if !remaining.is_empty() && remaining.len() >= 10 {
            sentences.push(remaining);
        }
    }
    sentences
}

fn extract_spo(sentence: &str, memory_type: &str, quality_score: f64) -> Option<Atom> {
    let tokens: Vec<&str> = sentence
        .split_whitespace()
        .filter(|t| t.len() >= 2)
        .collect();
    if tokens.len() < 3 {
        return None;
    }
    let (subject_end, predicate_end) = find_spo_split(&tokens)?;
    let subject = tokens[..subject_end].join(" ");
    let predicate = tokens[subject_end..predicate_end].join(" ");
    let object = tokens[predicate_end..].join(" ");
    if subject.len() < 2 || predicate.len() < 2 || object.len() < 2 {
        return None;
    }
    Some(Atom {
        subject,
        predicate,
        object,
        confidence: quality_score,
        atom_type: atom_type_from_memory(memory_type),
    })
}

fn find_spo_split(tokens: &[&str]) -> Option<(usize, usize)> {
    let verbs = [
        "is",
        "are",
        "was",
        "were",
        "has",
        "have",
        "had",
        "does",
        "did",
        "will",
        "would",
        "can",
        "could",
        "should",
        "shall",
        "may",
        "might",
        "must",
        "implemented",
        "implemented",
        "uses",
        "used",
        "requires",
        "required",
        "provides",
        "provided",
        "supports",
        "supported",
        "contains",
        "contained",
        "generates",
        "generated",
        "returns",
        "returned",
        "creates",
        "created",
        "updates",
        "updated",
        "removes",
        "removed",
        "adds",
        "added",
        "enables",
        "enabled",
        "allows",
        "allowed",
        "prevents",
        "prevented",
        "runs",
        "ran",
        "executes",
        "executed",
        "processes",
        "processed",
        "computes",
        "computed",
        "stores",
        "stored",
        "loads",
        "loaded",
        "sends",
        "sent",
        "receives",
        "received",
        "connects",
        "connected",
        "links",
        "linked",
        "maps",
        "mapped",
        "converts",
        "converted",
        "validates",
        "validated",
        "checks",
        "checked",
        "verifies",
        "verified",
        "ensures",
        "ensured",
        "triggers",
        "triggered",
        "invokes",
        "invoked",
        "calls",
        "called",
        "needs",
        "needed",
        "depends",
        "depended",
        "references",
        "referenced",
        "because",
        "since",
        "therefore",
        "however",
        "although",
        "while",
        "when",
        "where",
        "which",
        "that",
    ];
    let verbs_set: std::collections::HashSet<&str> = verbs.iter().copied().collect();
    for (i, token) in tokens.iter().enumerate() {
        let lower = token.to_lowercase();
        if verbs_set.contains(lower.as_str())
            || lower.ends_with("ed")
            || lower.ends_with("es")
            || lower.ends_with("ing")
        {
            if i >= 1 && tokens.len() > i + 1 {
                return Some((i, i + 1));
            }
        }
    }
    if tokens.len() >= 4 {
        Some((1, 2))
    } else {
        None
    }
}

fn atom_type_from_memory(memory_type: &str) -> String {
    match memory_type {
        "procedural" | "how_to" => "procedural".to_string(),
        "relational" | "relationship" => "relational".to_string(),
        _ => "declarative".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_atoms_from_multi_sentence_value() {
        let atoms = extract_atoms(
            "session:test",
            "The system was implemented to support evidence-based reasoning. The system uses PPR for graph traversal.",
            "declarative",
            0.9,
        );
        assert!(!atoms.is_empty());
        assert!(atoms.len() <= MAX_ATOMS_PER_MEMORY);
        for atom in &atoms {
            assert!(!atom.subject.is_empty());
            assert!(!atom.predicate.is_empty());
            assert!(!atom.object.is_empty());
            assert!((atom.confidence - 0.9).abs() < 0.01);
        }
    }

    #[test]
    fn caps_at_max_atoms() {
        let long_value: Vec<String> = (0..20)
            .map(|i| format!("Memory {i} was created because system required it."))
            .collect();
        let atoms = extract_atoms("key", &long_value.join(" "), "declarative", 1.0);
        assert_eq!(atoms.len(), MAX_ATOMS_PER_MEMORY);
    }

    #[test]
    fn atom_precision_returns_fraction_supported() {
        let atoms = vec![
            Atom {
                subject: "Apollo".into(),
                predicate: "uses".into(),
                object: "PPR".into(),
                confidence: 1.0,
                atom_type: "declarative".into(),
            },
            Atom {
                subject: "System".into(),
                predicate: "requires".into(),
                object: "evidence".into(),
                confidence: 1.0,
                atom_type: "declarative".into(),
            },
        ];
        let evidence = vec![serde_json::json!({
            "key": "apollo",
            "value": "Apollo uses PPR for graph traversal"
        })];
        let precision = atom_precision(&atoms, &evidence);
        assert!(precision >= 0.5, "At least one atom should be supported");
    }

    #[test]
    fn atom_precision_returns_zero_with_no_evidence() {
        let atoms = vec![Atom {
            subject: "test".into(),
            predicate: "is".into(),
            object: "value".into(),
            confidence: 1.0,
            atom_type: "declarative".into(),
        }];
        let precision = atom_precision(&atoms, &[]);
        assert_eq!(precision, 0.0);
    }

    #[test]
    fn atom_precision_returns_one_with_empty_atoms() {
        let precision = atom_precision(&[], &[]);
        assert_eq!(precision, 1.0);
    }

    #[test]
    fn procedural_memory_type() {
        let atoms = extract_atoms(
            "key",
            "Step was executed because tool required it.",
            "procedural",
            0.8,
        );
        assert!(atoms.iter().any(|a| a.atom_type == "procedural"));
    }

    #[test]
    fn subject_only_overlap_is_not_enough_for_support() {
        let atoms = vec![Atom {
            subject: "Apollo".into(),
            predicate: "uses".into(),
            object: "PPR".into(),
            confidence: 1.0,
            atom_type: "declarative".into(),
        }];
        let evidence = vec![serde_json::json!({
            "key": "apollo",
            "value": "Apollo was discussed in the meeting but no graph traversal method was recorded."
        })];
        assert_eq!(atom_precision(&atoms, &evidence), 0.0);
    }

    #[test]
    fn object_only_overlap_is_not_enough_for_support() {
        let atoms = vec![Atom {
            subject: "Apollo".into(),
            predicate: "uses".into(),
            object: "PPR".into(),
            confidence: 1.0,
            atom_type: "declarative".into(),
        }];
        let evidence = vec![serde_json::json!({
            "key": "ppr",
            "value": "PPR appeared in unrelated benchmark notes without the named system or usage evidence."
        })];
        assert_eq!(atom_precision(&atoms, &evidence), 0.0);
    }

    #[test]
    fn weighted_subject_predicate_object_support_accepts_grounded_atom() {
        let atoms = vec![Atom {
            subject: "Apollo".into(),
            predicate: "uses".into(),
            object: "PPR".into(),
            confidence: 1.0,
            atom_type: "declarative".into(),
        }];
        let evidence = vec![serde_json::json!({
            "key": "apollo_graph",
            "value": "Apollo uses PPR for graph traversal."
        })];
        assert_eq!(atom_precision(&atoms, &evidence), 1.0);
    }
}
