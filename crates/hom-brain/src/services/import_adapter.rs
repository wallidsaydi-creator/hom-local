use hom_shared::{RpcError, rpc_err};
use serde_json::{Value, json};

pub const MEMORY_KINDS: &[&str] = &[
    "project",
    "person",
    "repo",
    "decision",
    "preference",
    "task",
    "incident",
    "source_note",
    "session_summary",
    "tool_event",
];

pub fn valid_memory_kind(kind: &str) -> bool {
    MEMORY_KINDS.contains(&kind)
}

pub fn normalize_kind(kind: &str) -> &str {
    if valid_memory_kind(kind) {
        kind
    } else {
        "source_note"
    }
}

pub struct RawCandidate {
    pub title: String,
    pub body: String,
    pub kind: Option<String>,
    pub tags: Vec<String>,
    pub source_timestamp: Option<String>,
    pub metadata: Value,
}

pub fn parse_source(
    source_type: &str,
    content: &str,
    _source_uri: Option<&str>,
) -> Result<Vec<RawCandidate>, RpcError> {
    match source_type {
        "generic.json" => parse_generic_json(content),
        "generic.markdown" => parse_generic_markdown(content),
        _ => Err(rpc_err(
            -32602,
            &format!("unsupported_source_type: {source_type}"),
        )),
    }
}

fn parse_generic_json(content: &str) -> Result<Vec<RawCandidate>, RpcError> {
    let parsed: Value = serde_json::from_str(content)
        .map_err(|e| rpc_err(-32602, &format!("json_parse_error: {e}")))?;
    let items = parsed
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| rpc_err(-32602, "json_missing_items_array"))?;
    let mut candidates = Vec::new();
    for item in items {
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let body = item
            .get("body")
            .or_else(|| item.get("content"))
            .or_else(|| item.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if body.trim().is_empty() {
            continue;
        }
        let kind = item.get("kind").and_then(Value::as_str).map(str::to_string);
        let tags = item
            .get("tags")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let source_timestamp = item
            .get("created_at")
            .and_then(Value::as_str)
            .map(str::to_string);
        let metadata = item.get("metadata").cloned().unwrap_or(json!({}));
        candidates.push(RawCandidate {
            title,
            body,
            kind,
            tags,
            source_timestamp,
            metadata,
        });
    }
    Ok(candidates)
}

fn parse_generic_markdown(content: &str) -> Result<Vec<RawCandidate>, RpcError> {
    let mut candidates = Vec::new();
    let mut in_front_matter = false;
    let mut front_matter_lines: Vec<String> = Vec::new();
    let mut current_title = String::new();
    let mut current_body_lines: Vec<String> = Vec::new();
    let mut front_matter_consumed = false;
    let lines: Vec<&str> = content.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed == "---" && (i == 0 || in_front_matter) {
            if in_front_matter {
                in_front_matter = false;
                front_matter_consumed = true;
                continue;
            }
            in_front_matter = true;
            continue;
        }
        if in_front_matter {
            front_matter_lines.push(trimmed.to_string());
            continue;
        }
        if trimmed.starts_with("## ") {
            if !current_title.is_empty()
                || (current_title.is_empty() && !current_body_lines.is_empty())
            {
                let body = current_body_lines.join("\n").trim().to_string();
                if !body.is_empty() {
                    let (kind, tags, metadata) = parse_front_matter(&front_matter_lines);
                    candidates.push(RawCandidate {
                        title: current_title.clone(),
                        body,
                        kind,
                        tags,
                        source_timestamp: None,
                        metadata,
                    });
                }
            }
            current_title = trimmed.trim_start_matches("## ").trim().to_string();
            current_body_lines.clear();
            front_matter_lines.clear();
        } else {
            current_body_lines.push(line.to_string());
        }
    }

    // Last section
    let body = current_body_lines.join("\n").trim().to_string();
    if !body.is_empty() {
        let (kind, tags, metadata) = parse_front_matter(&front_matter_lines);
        candidates.push(RawCandidate {
            title: current_title,
            body,
            kind,
            tags,
            source_timestamp: None,
            metadata,
        });
    }

    // If no headings found, treat entire document as one candidate
    if candidates.is_empty() && !content.trim().is_empty() {
        let (kind, tags, metadata) = if front_matter_consumed {
            parse_front_matter(&front_matter_lines)
        } else {
            (None, Vec::new(), json!({}))
        };
        candidates.push(RawCandidate {
            title: String::new(),
            body: content.trim().to_string(),
            kind,
            tags,
            source_timestamp: None,
            metadata,
        });
    }

    Ok(candidates)
}

fn parse_front_matter(lines: &[String]) -> (Option<String>, Vec<String>, Value) {
    let mut kind = None;
    let mut tags = Vec::new();
    let mut meta = serde_json::Map::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "kind" | "type" => {
                    kind = Some(value.to_string());
                }
                "tags" => {
                    tags = value
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect();
                }
                _ => {
                    meta.insert(key.to_string(), json!(value));
                }
            }
        }
    }
    (kind, tags, Value::Object(meta))
}

pub fn parse_batch(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let batch_id = params
        .get("batch_id")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "batch_id_required"))?;
    let batch = state
        .store
        .get_import_batch(batch_id)?
        .ok_or_else(|| rpc_err(-32602, "batch_not_found"))?;
    if batch.status != "created" {
        return Err(rpc_err(
            -32602,
            &format!("batch_status_not_created: {}", batch.status),
        ));
    }
    state
        .store
        .update_import_batch_status(batch_id, "parsing")?;

    let content = params
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "content_required"))?;
    let raw_candidates = parse_source(&batch.source_type, content, batch.source_uri.as_deref())?;
    let total_raw = raw_candidates.len() as i64;

    state
        .store
        .update_import_batch_status(batch_id, "classified")?;

    let mut total_ready = 0_i64;
    let mut total_needs_review = 0_i64;
    let mut total_quarantined = 0_i64;
    let mut created_count = 0_i64;

    for raw in &raw_candidates {
        let classified = super::import_classifier::classify(raw, &batch.source_type);
        let status = classified.candidate_status.clone();
        match status.as_str() {
            "ready" => total_ready += 1,
            "needs_review" => total_needs_review += 1,
            _ => total_quarantined += 1,
        }
        let source_hash = sha256_hex(&raw.body);
        state.store.create_import_candidate(
            batch_id,
            &classified.memory_kind,
            if raw.title.is_empty() {
                None
            } else {
                Some(&raw.title)
            },
            &raw.body,
            classified.confidence,
            &classified.sensitivity_class,
            &status,
            &batch.source_type,
            Some(&batch.source_name),
            batch.source_uri.as_deref(),
            Some(&source_hash),
            raw.source_timestamp.as_deref(),
            json!({
                "source_type": batch.source_type,
            }),
            json!(raw.tags),
            json!([]),
            raw.metadata.clone(),
        )?;
        created_count += 1;
    }

    state.store.update_import_batch_counts(
        batch_id,
        total_raw,
        created_count,
        total_ready,
        total_needs_review,
        total_quarantined,
    )?;
    state
        .store
        .update_import_batch_status(batch_id, "preview_ready")?;

    Ok(json!({
        "ok": true,
        "batch_id": batch_id,
        "status": "preview_ready",
        "totals": {
            "raw_items": total_raw,
            "candidates": created_count,
            "ready": total_ready,
            "needs_review": total_needs_review,
            "quarantined": total_quarantined,
        }
    }))
}

fn sha256_hex(text: &str) -> String {
    use hom_shared::sha256_hex;
    sha256_hex(text)
}

pub fn preview_batch(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let batch_id = params
        .get("batch_id")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_err(-32602, "batch_id_required"))?;
    let batch = state
        .store
        .get_import_batch(batch_id)?
        .ok_or_else(|| rpc_err(-32602, "batch_not_found"))?;

    // Run dedupe scanner before returning preview
    super::import_dedupe::scan_for_duplicates(state, batch_id)?;

    let candidates = state
        .store
        .get_import_candidates_by_batch(batch_id, None, 10000)?;
    let mut by_kind: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for c in &candidates {
        *by_kind.entry(c.memory_kind.clone()).or_insert(0) += 1;
    }
    let kinds_json: Value = by_kind
        .iter()
        .map(|(k, v)| (k.clone(), json!(*v)))
        .collect();

    let needs_review: Vec<Value> = candidates
        .iter()
        .filter(|c| c.candidate_status == "needs_review" || c.candidate_status == "duplicate_candidate" || c.candidate_status == "contradiction_candidate")
        .map(|c| {
            json!({
                "candidate_id": c.candidate_id,
                "memory_kind": c.memory_kind,
                "title": c.title,
                "body": if c.body.len() > 200 { format!("{}...", &c.body[..200]) } else { c.body.clone() },
                "confidence": c.confidence,
                "sensitivity_class": c.sensitivity_class,
                "candidate_status": c.candidate_status,
                "duplicate_of": c.duplicate_of,
                "contradiction_with": c.contradiction_with,
            })
        })
        .collect();

    // Refresh counts after dedupe may have changed statuses
    let counts = state.store.count_import_candidates_by_status(batch_id)?;
    let mut totals_map = serde_json::Map::new();
    for (status, count) in counts {
        totals_map.insert(status, json!(count));
    }

    Ok(json!({
        "ok": true,
        "batch_id": batch_id,
        "source": {
            "type": batch.source_type,
            "name": batch.source_name,
            "uri": batch.source_uri,
        },
        "status": batch.status,
        "totals": Value::Object(totals_map),
        "by_kind": kinds_json,
        "needs_review": needs_review,
    }))
}
