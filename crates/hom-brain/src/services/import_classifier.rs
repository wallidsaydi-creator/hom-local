use super::import_adapter::RawCandidate;

pub struct ClassifyResult {
    pub memory_kind: String,
    pub confidence: f64,
    pub sensitivity_class: String,
    pub candidate_status: String,
}

pub fn classify(raw: &RawCandidate, source_type: &str) -> ClassifyResult {
    let memory_kind = infer_kind(raw);
    let sensitivity = detect_sensitivity(raw);
    let confidence = compute_confidence(raw, &memory_kind, source_type);
    let candidate_status = determine_status(confidence, &sensitivity);
    ClassifyResult {
        memory_kind,
        confidence,
        sensitivity_class: sensitivity,
        candidate_status,
    }
}

fn infer_kind(raw: &RawCandidate) -> String {
    // Explicit kind from source takes priority if valid
    if let Some(ref kind) = raw.kind {
        if super::import_adapter::valid_memory_kind(kind) {
            return kind.clone();
        }
    }
    let title_lower = raw.title.to_lowercase();
    let body_lower = raw.body.to_lowercase();
    let text = format!("{} {}", title_lower, body_lower);

    if contains_any(
        &text,
        &[
            "decided",
            "decision:",
            "chose to",
            "we chose",
            "trade-off",
            "tradeoff",
        ],
    ) {
        return "decision".to_string();
    }
    if contains_any(
        &text,
        &[
            "incident",
            "outage",
            "post-mortem",
            "postmortem",
            "root cause",
            "break fix",
        ],
    ) {
        return "incident".to_string();
    }
    if contains_any(
        &text,
        &[
            "i prefer",
            "my preference",
            "always use",
            "never use",
            "style:",
            "convention:",
        ],
    ) {
        return "preference".to_string();
    }
    if contains_any(
        &text,
        &["todo", "task:", "fixme", "implement", "migrate to", "plan:"],
    ) {
        return "task".to_string();
    }
    if contains_any(
        &text,
        &[
            "repo:",
            "repository",
            "codebase",
            "convention",
            "coding style",
            "branching strategy",
        ],
    ) {
        return "repo".to_string();
    }
    if contains_any(
        &text,
        &[
            "person:",
            "contact:",
            "team member",
            "colleague",
            "works at",
        ],
    ) {
        return "person".to_string();
    }
    if contains_any(
        &text,
        &[
            "project:",
            "tech stack",
            "architecture",
            "stack:",
            "infrastructure:",
        ],
    ) {
        return "project".to_string();
    }
    "source_note".to_string()
}

fn contains_any(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|k| text.contains(k))
}

fn detect_sensitivity(raw: &RawCandidate) -> String {
    let text = format!("{} {}", raw.title.to_lowercase(), raw.body.to_lowercase());
    if contains_any(
        &text,
        &[
            "password",
            "secret",
            "api_key",
            "apikey",
            "token",
            "credential",
            "private_key",
            "access_key",
            "auth_token",
        ],
    ) {
        return "private".to_string();
    }
    if contains_any(
        &text,
        &[
            "email",
            "phone",
            "address",
            "ssn",
            "social security",
            "credit card",
            "date of birth",
            "personal",
        ],
    ) {
        return "sensitive".to_string();
    }
    "normal".to_string()
}

fn compute_confidence(raw: &RawCandidate, kind: &str, source_type: &str) -> f64 {
    let mut conf: f64 = 0.5;
    // Explicit kind from source is a strong signal
    if raw.kind.is_some() {
        conf += 0.2;
    }
    // Non-empty title helps
    if !raw.title.is_empty() {
        conf += 0.05;
    }
    // Longer body means more content to work with
    if raw.body.len() > 100 {
        conf += 0.05;
    }
    if raw.body.len() > 500 {
        conf += 0.05;
    }
    // Structured source types get a small boost
    if source_type == "generic.json" {
        conf += 0.05;
    }
    // Heuristic classification is less certain than explicit
    if raw.kind.is_none() && kind != "source_note" {
        conf -= 0.05;
    }
    conf.clamp(0.0, 1.0)
}

fn determine_status(confidence: f64, sensitivity: &str) -> String {
    if sensitivity == "private" {
        return "quarantined".to_string();
    }
    if sensitivity == "sensitive" {
        return "needs_review".to_string();
    }
    if confidence >= 0.8 {
        "ready".to_string()
    } else if confidence >= 0.5 {
        "needs_review".to_string()
    } else {
        "low_quality".to_string()
    }
}
