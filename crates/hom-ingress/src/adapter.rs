//! Response adapter layer: transforms brain JSON-RPC responses into the exact
//! shapes expected by Swift's Decodable models.

use serde_json::{Value, json};

pub fn adapt_response(method: &str, result: &Value) -> Value {
    match method {
        "memory.recall" | "memory.recall.smart" => recall_payload(result),
        "memory.open" => memory_detail_payload(result),
        "memory.answer" => chat_payload(result),
        "events.list" => events_payload(result),
        "session.get" | "session.login" => session_info_payload(result),
        "system.status" => status_payload(result),
        "benchmarks.list" => benchmark_list_payload(result),
        "benchmarks.detail" => benchmark_detail_payload(result),
        _ => ensure_ok(result),
    }
}

pub fn memory_item_payload(mut memory: Value) -> Value {
    if memory.get("id").is_none() {
        if let Some(id) = first(&memory, &["memory_id", "memoryId"]) {
            memory["id"] = id.clone();
        }
    }
    if memory.get("memory_id").is_none() {
        if let Some(id) = first(&memory, &["id", "memoryId"]) {
            memory["memory_id"] = id.clone();
        }
    }
    if memory.get("created_at").is_none() {
        memory["created_at"] = first(
            &memory,
            &["created_at_s", "createdAtS", "createdAt", "created_at"],
        )
        .cloned()
        .unwrap_or_else(|| json!(0));
    }
    if memory.get("updated_at").is_none() {
        memory["updated_at"] = first(
            &memory,
            &[
                "updated_at_s",
                "updatedAtS",
                "updatedAt",
                "updated_at",
                "created_at_s",
                "createdAtS",
                "created_at",
            ],
        )
        .cloned()
        .unwrap_or_else(|| json!(0));
    }
    if memory.get("type").is_none() {
        memory["type"] = first(&memory, &["memory_type", "memoryType"])
            .cloned()
            .unwrap_or_else(|| json!("memory"));
    }
    if memory.get("memory_type").is_none() {
        memory["memory_type"] = memory
            .get("type")
            .cloned()
            .unwrap_or_else(|| json!("memory"));
    }
    if memory.get("title").is_none() {
        memory["title"] = first(&memory, &["key", "title"])
            .cloned()
            .unwrap_or_else(|| json!("memory"));
    }
    if memory.get("body_redacted").is_none() {
        memory["body_redacted"] = first(&memory, &["value", "bodyRedacted", "body_preview"])
            .cloned()
            .unwrap_or_else(|| json!(""));
    }
    if memory.get("body_preview").is_none() {
        let preview = first(&memory, &["value", "bodyPreview", "body_redacted"])
            .and_then(Value::as_str)
            .map(excerpt)
            .unwrap_or_default();
        memory["body_preview"] = json!(preview);
    }
    if memory.get("display_content").is_none() {
        memory["display_content"] = first(&memory, &["value", "displayContent", "body_redacted"])
            .cloned()
            .unwrap_or_else(|| json!(""));
    }
    if memory.get("source_kind").is_none() {
        memory["source_kind"] = first(&memory, &["source", "sourceKind"])
            .cloned()
            .unwrap_or_else(|| json!("hom-local"));
    }
    if memory.get("source_ref").is_none() {
        memory["source_ref"] = first(&memory, &["source_ref", "sourceRef", "key"])
            .cloned()
            .unwrap_or(Value::Null);
    }
    if memory.get("sensitivity").is_none() {
        memory["sensitivity"] = json!("normal");
    }
    if memory.get("quality_score").is_none() {
        memory["quality_score"] = first(&memory, &["score", "qualityScore"])
            .cloned()
            .unwrap_or(Value::Null);
    }
    if memory.get("security_score").is_none() {
        memory["security_score"] = Value::Null;
    }
    if memory.get("freshness_state").is_none() {
        memory["freshness_state"] = json!("active");
    }
    if memory.get("recall_weight").is_none() {
        memory["recall_weight"] = first(&memory, &["score", "recallWeight"])
            .cloned()
            .unwrap_or(Value::Null);
    }
    if memory.get("session_id").is_none() {
        memory["session_id"] = first(&memory, &["sessionId"])
            .cloned()
            .unwrap_or(Value::Null);
    }
    if memory.get("deleted_at").is_none() {
        memory["deleted_at"] = Value::Null;
    }
    memory
}

pub fn memory_summary_payload(memory: Value) -> Value {
    let memory = memory_item_payload(memory);
    json!({
        "id": first(&memory, &["id", "memory_id"]).cloned().unwrap_or_else(|| json!("")),
        "type": first(&memory, &["type", "memory_type"]).cloned().unwrap_or_else(|| json!("memory")),
        "title": memory.get("title").cloned().unwrap_or(Value::Null),
        "source_kind": memory.get("source_kind").cloned().unwrap_or_else(|| json!("hom-local")),
        "created_at": memory.get("created_at").cloned().unwrap_or(Value::Null),
        "updated_at": memory.get("updated_at").cloned().unwrap_or_else(|| json!(0)),
        "evidence_preview": memory.get("body_preview").cloned().unwrap_or(Value::Null)
    })
}

pub fn recall_payload(result: &Value) -> Value {
    let mut payload = ensure_ok(result);
    let hits = first(&payload, &["results", "memories"])
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let results = hits
        .iter()
        .enumerate()
        .map(|(index, hit)| recall_result_payload(hit, index + 1))
        .collect::<Vec<_>>();
    let memories = results
        .iter()
        .filter_map(|hit| hit.get("memory").cloned())
        .collect::<Vec<_>>();

    payload["results"] = json!(results);
    payload["memories"] = json!(memories);
    if payload.get("result_count").is_none() {
        payload["result_count"] = json!(payload["results"].as_array().map_or(0, Vec::len));
    }
    if payload.get("mode").is_none() {
        payload["mode"] = payload
            .get("recall_meta")
            .and_then(|meta| meta.get("planner"))
            .cloned()
            .unwrap_or_else(|| json!("local_rrf"));
    }
    if payload.get("resolved_modes").is_none() {
        if let Some(modes) = payload
            .get("recall_meta")
            .and_then(|meta| meta.get("modes"))
        {
            payload["resolved_modes"] = modes.clone();
        }
    }
    payload
}

pub fn recall_detail_payload(query_id: &str, result: Option<&Value>) -> Value {
    result.map_or_else(
        || {
            json!({
                "found": false,
                "query_id": query_id,
                "query": Value::Null,
                "mode": Value::Null,
                "fallback_used": false,
                "result_count": 0,
                "results": []
            })
        },
        |value| {
            let recall = recall_payload(value);
            json!({
                "found": true,
                "query_id": first(&recall, &["query_id", "queryId"]).cloned().unwrap_or_else(|| json!(query_id)),
                "query": recall.get("query").cloned().unwrap_or(Value::Null),
                "mode": recall.get("mode").cloned().unwrap_or(Value::Null),
                "fallback_used": first(&recall, &["fallback_used", "fallbackUsed"]).cloned().unwrap_or(Value::Null),
                "result_count": first(&recall, &["result_count", "resultCount"]).cloned().unwrap_or(Value::Null),
                "results": recall.get("results").cloned().unwrap_or_else(|| json!([]))
            })
        },
    )
}

pub fn memory_detail_payload(result: &Value) -> Value {
    let memory = result.get("memory").cloned().or_else(|| {
        first(result, &["id", "memory_id", "memoryId"])
            .is_some()
            .then(|| result.clone())
    });
    let found = memory.is_some()
        || result
            .get("found")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    json!({
        "ok": result.get("ok").cloned().unwrap_or_else(|| json!(true)),
        "found": found,
        "memory": memory.map(memory_item_payload).unwrap_or(Value::Null),
        "entities": result.get("entities").cloned().unwrap_or_else(|| json!([])),
        "related": result.get("related").cloned().unwrap_or_else(|| json!([])),
        "recall_history": first(result, &["recall_history", "recallHistory"]).cloned().unwrap_or_else(|| json!([]))
    })
}

pub fn benchmark_list_payload(result: &Value) -> Value {
    let mut payload = ensure_ok(result);
    if payload.get("scenario_templates").is_none() {
        payload["scenario_templates"] = json!([]);
    }
    payload
}

pub fn benchmark_detail_payload(result: &Value) -> Value {
    let mut payload = ensure_ok(result);
    if payload.get("result").is_none() {
        payload["result"] = json!({});
    }
    payload
}

pub fn project_summary_payload(project: Value) -> Value {
    json!({
        "id": first(&project, &["id", "project_id", "projectId"]).cloned().unwrap_or_else(|| json!("")),
        "workspace_id": first(&project, &["workspace_id", "workspaceId"]).cloned().unwrap_or(Value::Null),
        "name": project.get("name").cloned().unwrap_or_else(|| json!("Untitled Project")),
        "description": project.get("description").cloned().unwrap_or(Value::Null),
        "session_count": first(&project, &["session_count", "sessionCount"]).cloned().unwrap_or(Value::Null),
        "created_at": first(&project, &["created_at", "createdAt", "createdAtS", "created_at_s"]).cloned().unwrap_or(Value::Null),
        "updated_at": first(&project, &["updated_at", "updatedAt", "updatedAtS", "updated_at_s"]).cloned().unwrap_or(Value::Null)
    })
}

pub fn session_summary_payload(session: Value) -> Value {
    let created = first(
        &session,
        &["created_at", "createdAt", "createdAtS", "created_at_s"],
    )
    .cloned()
    .unwrap_or(Value::Null);
    json!({
        "session_id": first(&session, &["session_id", "sessionId", "id"]).cloned().unwrap_or_else(|| json!("")),
        "project_id": first(&session, &["project_id", "projectId"]).cloned().unwrap_or(Value::Null),
        "name": first(&session, &["name", "title"]).cloned().unwrap_or(Value::Null),
        "memory_count": first(&session, &["memory_count", "memoryCount"]).cloned().unwrap_or_else(|| json!(0)),
        "updated_at": first(&session, &["updated_at", "updatedAt", "updatedAtS", "updated_at_s"]).cloned().unwrap_or_else(|| created.clone()),
        "created_at": created,
        "latest_memory_at": first(&session, &["latest_memory_at", "latestMemoryAt"]).cloned().unwrap_or(Value::Null)
    })
}

pub fn session_detail_payload(result: &Value) -> Value {
    let session = result.get("session").unwrap_or(result);
    let memories = result
        .get("memories")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(memory_item_payload)
        .collect::<Vec<_>>();
    let source_kinds = memories
        .iter()
        .filter_map(|memory| memory.get("source_kind").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    json!({
        "ok": result.get("ok").cloned().unwrap_or_else(|| json!(true)),
        "found": result.get("found").cloned().unwrap_or_else(|| json!(true)),
        "session_id": first(session, &["session_id", "sessionId", "id"]).cloned().unwrap_or(Value::Null),
        "memory_count": first(session, &["memory_count", "memoryCount"]).cloned().unwrap_or_else(|| json!(memories.len())),
        "started_at": first(session, &["started_at", "startedAt", "createdAtS", "created_at_s", "created_at"]).cloned().unwrap_or(Value::Null),
        "updated_at": first(session, &["updated_at", "updatedAt", "updatedAtS", "updated_at_s", "createdAtS", "created_at_s"]).cloned().unwrap_or(Value::Null),
        "source_kinds": source_kinds,
        "memories": memories,
        "events": result.get("events").and_then(Value::as_array).map(|events| events.iter().map(ledger_event_payload).collect::<Vec<_>>()).unwrap_or_default()
    })
}

pub fn nightly_day_payload(day: Value) -> Value {
    if day.get("reports").is_some() {
        return day;
    }
    let day_id = first(&day, &["day", "date"])
        .and_then(Value::as_str)
        .unwrap_or("undated")
        .to_string();
    let proposals = day.get("proposals").and_then(Value::as_i64).unwrap_or(0);
    let applied = day.get("applied").and_then(Value::as_i64).unwrap_or(0);
    json!({
        "day": day_id,
        "reports": [{
            "report_id": format!("nightly:{day_id}"),
            "started_at": Value::Null,
            "completed_at": Value::Null,
            "summary_redacted": format!("{proposals} proposals, {applied} applied"),
            "health": Value::Null,
            "proposed_actions": [],
            "mutation_count": applied
        }]
    })
}

pub fn nightly_result_payload(result: &Value) -> Value {
    let applied = first(result, &["applied", "applied_count", "appliedCount"])
        .and_then(|value| value.as_bool().or_else(|| value.as_i64().map(|n| n > 0)))
        .unwrap_or(false);
    let proposals = first(result, &["proposal_count", "proposalCount", "proposals"])
        .cloned()
        .unwrap_or_else(|| json!(0));
    let mutation_count = first(result, &["mutation_count", "mutationCount"])
        .cloned()
        .unwrap_or_else(|| json!(0));
    json!({
        "summary_redacted": first(result, &["summary_redacted", "summaryRedacted"]).cloned().unwrap_or_else(|| {
            json!(format!(
                "{} proposals, {} mutations",
                proposals.as_i64().unwrap_or(0),
                mutation_count.as_i64().unwrap_or(0)
            ))
        }),
        "health": result.get("health").cloned().unwrap_or(Value::Null),
        "proposal_count": proposals,
        "mutation_count": mutation_count,
        "applied": applied
    })
}

pub fn provider_info_payload(provider: Value) -> Value {
    let provider_id = first(&provider, &["provider_id", "providerId", "id"])
        .cloned()
        .unwrap_or_else(|| json!("unknown"));
    let provider_id_str = provider_id.as_str().unwrap_or("unknown");
    let base_url = first(&provider, &["base_url", "baseUrl"])
        .cloned()
        .unwrap_or(Value::Null);
    let detected = provider
        .get("detected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let available = provider
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let enabled = provider
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let health = provider
        .get("health")
        .cloned()
        .or_else(|| provider.get("probe").cloned())
        .unwrap_or_else(|| json!({}));
    let model_count = first(&health, &["model_count", "modelCount"])
        .cloned()
        .unwrap_or_else(|| json!(0));
    let models = health.get("models").cloned().unwrap_or_else(|| json!([]));
    let circuit = health
        .get("circuit")
        .cloned()
        .unwrap_or_else(|| json!("closed"));
    let api_key_env = first(&health, &["api_key_env", "apiKeyEnv"])
        .cloned()
        .unwrap_or(Value::Null);
    let credential_redacted = first(&health, &["credential_redacted", "credentialRedacted"])
        .cloned()
        .unwrap_or(Value::Null);
    let auth_contract = provider
        .get("auth_contract")
        .or_else(|| provider.pointer("/route_certificate/diagnostics/auth_contract"))
        .cloned()
        .unwrap_or(Value::Null);
    let auth_method = provider
        .get("auth_method")
        .or_else(|| provider.pointer("/route_certificate/diagnostics/auth_method"))
        .cloned()
        .unwrap_or(Value::Null);
    let auth_lanes = provider
        .get("auth_lanes")
        .or_else(|| provider.get("authLanes"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    let route_certificate = provider
        .get("route_certificate")
        .cloned()
        .unwrap_or(Value::Null);
    let routeable = provider
        .get("routeable")
        .or_else(|| route_certificate.get("routeable"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let route_certified_models = if routeable
        && route_certificate
            .get("model_catalog_ok")
            .or_else(|| route_certificate.get("modelCatalogOk"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        route_certificate
            .get("models")
            .cloned()
            .unwrap_or_else(|| json!([]))
    } else {
        json!([])
    };
    json!({
        "providerId": provider_id,
        "provider_id": provider_id,
        "label": provider.get("label").or_else(|| provider.get("name")).or_else(|| provider.get("display_name")).cloned().unwrap_or(provider_id.clone()),
        "name": provider.get("label").or_else(|| provider.get("name")).or_else(|| provider.get("display_name")).cloned().unwrap_or(provider_id.clone()),
        "display_name": provider.get("label").or_else(|| provider.get("name")).or_else(|| provider.get("display_name")).cloned().unwrap_or(provider_id.clone()),
        "kind": provider.get("kind").cloned().unwrap_or_else(|| json!(provider_kind(provider_id_str))),
        "baseUrl": base_url,
        "base_url": base_url,
        "authMethod": auth_method,
        "auth_method": auth_method,
        "authContract": auth_contract,
        "auth_contract": auth_contract,
        "authLanes": auth_lanes.clone(),
        "auth_lanes": auth_lanes,
        "detected": detected,
        "available": available,
        "enabled": enabled,
        "routeable": routeable,
        "selectedModel": first(&route_certificate, &["model_id", "modelId"]).cloned().unwrap_or(Value::Null),
        "selected_model": first(&route_certificate, &["model_id", "modelId"]).cloned().unwrap_or(Value::Null),
        "routeCertificate": route_certificate.clone(),
        "route_certificate": route_certificate,
        "routeCertifiedModels": route_certified_models.clone(),
        "route_certified_models": route_certified_models,
        "modelSelectorEligible": routeable,
        "model_selector_eligible": routeable,
        "health": {
            "modelCount": model_count,
            "model_count": model_count,
            "models": models,
            "circuit": circuit,
            "credentialState": first(&health, &["credential_state", "credentialState"]).cloned().unwrap_or(Value::Null),
            "credential_state": first(&health, &["credential_state", "credentialState"]).cloned().unwrap_or(Value::Null),
            "apiKeyEnv": api_key_env,
            "api_key_env": api_key_env,
            "credentialRedacted": credential_redacted,
            "credential_redacted": credential_redacted
        }
    })
}

pub fn provider_action_payload(provider_id: &str, result: &Value, enabled: Option<bool>) -> Value {
    let mut provider = result.clone();
    provider["provider_id"] = json!(provider_id);
    provider["kind"] = json!(provider_kind(provider_id));
    provider["enabled"] = json!(enabled.unwrap_or_else(|| {
        provider
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }));
    if provider.get("detected").is_none() {
        provider["detected"] = json!(
            provider.get("probe").is_some()
                || provider.get("ok").and_then(Value::as_bool).unwrap_or(false)
        );
    }
    provider_info_payload(provider)
}

pub fn probe_all_payload(result: &Value) -> Value {
    let providers = result
        .get("providers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let detected = providers
        .iter()
        .filter(|provider| {
            first(provider, &["detected", "available"])
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count();
    let available_models: i64 = providers
        .iter()
        .filter_map(|provider| {
            provider
                .get("health")
                .and_then(|health| first(health, &["model_count", "modelCount"]))
                .and_then(Value::as_i64)
        })
        .sum();
    json!({
        "total": providers.len(),
        "detected": detected,
        "availableModels": available_models,
        "available_models": available_models
    })
}

pub fn connect_kit_payload(app_id: &str, result: &Value) -> Value {
    let mcp_payload = result.get("mcp");
    let request = result.get("request").cloned().unwrap_or_else(|| {
        json!({
            "request_id": app_id,
            "app_name": app_id,
            "app_type": "app",
            "requested_scopes": [],
            "state": "pending"
        })
    });
    json!({
        "app_id": app_id,
        "request": request,
        "mcp": {
            "transport": mcp_payload.and_then(|payload| payload.get("transport")).cloned().unwrap_or_else(|| json!("stdio")),
            "command": mcp_payload.and_then(|payload| payload.get("command")).cloned().unwrap_or_else(|| json!("hom")),
            "args": mcp_payload.and_then(|payload| payload.get("args")).cloned().unwrap_or_else(|| json!(["mcp"])),
            "discovery_url": mcp_payload.and_then(|payload| first(payload, &["discovery_url", "discoveryUrl"])).cloned().unwrap_or(Value::Null),
            "tools": mcp_payload.and_then(|payload| payload.get("tools")).cloned().unwrap_or_else(|| json!([])),
            "env": mcp_payload.and_then(|payload| payload.get("env")).cloned().unwrap_or_else(|| json!({})),
            "note": mcp_payload.and_then(|payload| payload.get("note")).cloned().unwrap_or(Value::Null)
        },
        "requires_client_keypair": first(result, &["requires_client_keypair", "requiresClientKeypair"]).and_then(Value::as_bool).unwrap_or(false),
        "instructions": result.get("instructions").cloned().unwrap_or_else(|| json!("Connected"))
    })
}

pub fn imports_payload(imports_result: &Value, settings_result: Option<&Value>) -> Value {
    let imports = imports_result
        .get("imports")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let total_rows: i64 = imports
        .iter()
        .map(|item| {
            first(item, &["memoriesImported", "memories_imported"])
                .and_then(Value::as_i64)
                .unwrap_or(0)
                + first(item, &["memoriesSkipped", "memories_skipped"])
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
        })
        .sum();
    let memory_count = settings_result
        .and_then(|settings| settings.get("corpus"))
        .and_then(|corpus| first(corpus, &["memory_count", "memoryCount"]))
        .and_then(Value::as_i64);
    json!({
        "pack": {"total_rows": total_rows},
        "corpus": {"memory_count": memory_count},
        "imports": imports
    })
}

pub fn purge_payload(result: &Value) -> Value {
    let count = first(result, &["memories_purged", "memoriesPurged"])
        .and_then(Value::as_i64)
        .unwrap_or(0);
    json!({
        "ok": result.get("ok").and_then(Value::as_bool).unwrap_or(true),
        "operation_id": first(result, &["operation_id", "operationId"]).cloned().unwrap_or(Value::Null),
        "purged": result.get("ok").and_then(Value::as_bool).unwrap_or(true),
        "before": {"memory_count": count},
        "after": {"memory_count": 0},
        "ledger_event_id": first(result, &["ledger_event_id", "ledgerEventId"]).cloned().unwrap_or(Value::Null),
        "report": result.get("report").cloned().unwrap_or(Value::Null)
    })
}

pub fn backup_payload(result: &Value) -> Value {
    json!({
        "ok": result.get("ok").and_then(Value::as_bool).unwrap_or(true),
        "operation_id": first(result, &["operation_id", "operationId"]).cloned().unwrap_or(Value::Null),
        "backup_id": first(result, &["backup_id", "backupId", "path"]).cloned().unwrap_or(Value::Null),
        "ledger_event_id": first(result, &["ledger_event_id", "ledgerEventId"]).cloned().unwrap_or(Value::Null),
        "report": result.get("report").cloned().unwrap_or(Value::Null)
    })
}

fn session_info_payload(result: &Value) -> Value {
    let mut payload = ensure_ok(result);
    if payload.get("daemon").is_none() {
        payload["daemon"] = json!({
            "baseUrl": "http://127.0.0.1:9101",
            "port": 9101,
            "version": env!("CARGO_PKG_VERSION")
        });
    } else if let Some(daemon) = payload.get_mut("daemon").and_then(Value::as_object_mut) {
        if !daemon.contains_key("baseUrl") {
            let base_url = daemon
                .get("base_url")
                .cloned()
                .unwrap_or_else(|| json!("http://127.0.0.1:9101"));
            daemon.insert("baseUrl".to_string(), base_url);
        }
        daemon.entry("port").or_insert_with(|| json!(9101));
        daemon
            .entry("version")
            .or_insert_with(|| json!(env!("CARGO_PKG_VERSION")));
    }
    if payload.get("serverPublicKey").is_none() {
        if let Some(value) = payload.get("server_public_key") {
            payload["serverPublicKey"] = value.clone();
        }
    }
    payload
}

fn status_payload(result: &Value) -> Value {
    let mut payload = ensure_ok(result);
    payload["memories"] = payload.get("memories").cloned().unwrap_or_else(|| json!(0));
    if payload.get("ledger").is_none() {
        payload["ledger"] = json!({"valid": false, "total_events": 0, "head_hash": Value::Null});
    }
    if payload.get("daemon").is_none() {
        payload["daemon"] = json!({"baseUrl": "http://127.0.0.1:9101"});
    }
    if payload.get("corpus").is_none() {
        payload["corpus"] = json!({
            "memory_count": payload.get("memories").cloned().unwrap_or_else(|| json!(0)),
            "seed_pack_memories": 0,
            "seed_pack_total_rows": 0,
            "source_counts": []
        });
    }
    payload
}

fn chat_payload(result: &Value) -> Value {
    let mut payload = ensure_ok(result);
    if let Some(recall) = payload.get("recall").cloned() {
        payload["recall"] = recall_payload(&recall);
    }
    payload
}

fn events_payload(result: &Value) -> Value {
    let events = result
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(ledger_event_payload)
        .collect::<Vec<_>>();
    json!({"ok": true, "events": events})
}

fn recall_result_payload(hit: &Value, rank: usize) -> Value {
    if hit.get("memory").is_some() && first(hit, &["memory_id", "memoryId"]).is_some() {
        return hit.clone();
    }
    let memory = memory_item_payload(hit.clone());
    json!({
        "memory_id": first(&memory, &["memory_id", "id"]).cloned().unwrap_or_else(|| json!("")),
        "memory": memory,
        "score": hit.get("score").cloned().unwrap_or_else(|| json!(0.0)),
        "rank": hit.get("rank").cloned().unwrap_or_else(|| json!(rank)),
        "score_components": first(hit, &["score_components", "scoreComponents", "components"]).cloned().unwrap_or(Value::Null),
        "retrieval_mode": first(hit, &["retrieval_mode", "retrievalMode"]).cloned().unwrap_or_else(|| json!("rrf")),
        "evidence_span_redacted": first(hit, &["evidence_span_redacted", "evidenceSpanRedacted", "value", "body_preview"]).cloned().unwrap_or(Value::Null)
    })
}

fn ledger_event_payload(event: &Value) -> Value {
    let payload = event.get("payload").unwrap_or(&Value::Null);
    json!({
        "event_id": first(event, &["event_id", "eventId", "id"]).cloned().unwrap_or_else(|| json!("")),
        "event_type": first(event, &["event_type", "eventType"]).cloned().unwrap_or_else(|| json!("event")),
        "actor_id": first(event, &["actor_id", "actorId", "actor"]).cloned().unwrap_or(Value::Null),
        "operation": first(event, &["operation"]).or_else(|| first(payload, &["operation"])).or_else(|| first(event, &["event_type", "eventType"])).cloned().unwrap_or_else(|| json!("event")),
        "target_type": first(event, &["target_type", "targetType"]).or_else(|| first(payload, &["target_type", "targetType"])).cloned().unwrap_or(Value::Null),
        "target_id": first(event, &["target_id", "targetId", "subject_id", "subjectId"]).cloned().unwrap_or(Value::Null),
        "reason": first(event, &["reason"]).or_else(|| first(payload, &["reason"])).cloned().unwrap_or(Value::Null),
        "summary_redacted": first(event, &["summary_redacted", "summaryRedacted"]).or_else(|| first(payload, &["summary_redacted", "summaryRedacted", "summary"])).or_else(|| first(event, &["event_type", "eventType"])).cloned().unwrap_or_else(|| json!("event")),
        "created_at": first(event, &["created_at", "createdAt", "created_at_s", "createdAtS"]).cloned().unwrap_or_else(|| json!(0))
    })
}

fn ensure_ok(result: &Value) -> Value {
    let mut payload = result.clone();
    if payload.get("ok").is_none() {
        payload["ok"] = json!(true);
    }
    payload
}

fn first<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| value.get(*key))
}

fn provider_kind(provider_id: &str) -> &'static str {
    match provider_id {
        "openai" => "openai",
        "codex-oauth" => "codex_oauth",
        "anthropic" => "anthropic",
        "google-gemini" => "google_gemini",
        "openrouter" => "openrouter",
        "ollama" => "local",
        "venice" => "venice",
        "aixai" => "aixai",
        "local-models" => "local",
        _ => "unknown",
    }
}

pub fn mcp_config_payload(result: &Value) -> Value {
    let server = result
        .get("mcpServers")
        .and_then(|ms| ms.get("hom-local"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let transport = server
        .get("transport")
        .cloned()
        .unwrap_or_else(|| json!("stdio"));
    let command = server
        .get("command")
        .cloned()
        .unwrap_or_else(|| json!("hom"));
    let args = server
        .get("args")
        .cloned()
        .unwrap_or_else(|| json!(["mcp"]));
    let discovery_url = first(&server, &["discovery_url", "discoveryUrl", "baseUrl"])
        .cloned()
        .unwrap_or(Value::Null);
    let tools = server
        .get("tools")
        .or_else(|| result.get("tools"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    let env = server.get("env").cloned().unwrap_or_else(|| json!({}));
    let note = server
        .get("description")
        .or_else(|| server.get("note"))
        .cloned()
        .unwrap_or_else(|| json!("HOM Local — sovereign cognitive memory"));
    json!({
        "transport": transport,
        "command": command,
        "args": args,
        "discovery_url": discovery_url,
        "tools": tools,
        "env": env,
        "note": note
    })
}

pub fn settings_payload(
    settings: &Value,
    providers_result: Option<&Value>,
    apps_result: Option<&Value>,
    session_configured: bool,
) -> Value {
    // Memory count: prefer corpus.memory_count, then top-level, then system status
    let memory_count = settings
        .get("corpus")
        .and_then(|c| first(c, &["memory_count", "memoryCount"]))
        .and_then(Value::as_i64)
        .or_else(|| first(settings, &["memory_count", "memoryCount"]).and_then(Value::as_i64))
        .or_else(|| providers_result.and_then(|r| r.get("memories").and_then(Value::as_i64)));

    let providers = providers_result
        .and_then(|r| r.get("providers").and_then(Value::as_array).cloned())
        .or_else(|| settings.get("providers").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .into_iter()
        .map(provider_info_payload)
        .collect::<Vec<_>>();

    let apps_from_settings = settings.get("apps").cloned().unwrap_or_else(|| json!({}));
    let (requests, grants) = if let Some(apps) = apps_result
        .and_then(|r| r.get("apps").and_then(Value::as_array))
        .or_else(|| apps_from_settings.get("requests").and_then(Value::as_array))
    {
        let req = apps
            .iter()
            .filter(|a| a.get("status").and_then(Value::as_str) != Some("approved"))
            .map(app_request_payload_for_settings)
            .collect::<Vec<_>>();
        let grnt = apps
            .iter()
            .filter(|a| a.get("status").and_then(Value::as_str) == Some("approved"))
            .map(app_grant_payload_for_settings)
            .collect::<Vec<_>>();
        (req, grnt)
    } else {
        (vec![], vec![])
    };

    let config_rows = build_config_rows(settings);

    // Extract corpus details from nested settings objects
    let corpus = settings.get("corpus").cloned().unwrap_or_else(|| json!({}));
    let seed_pack = first(
        &corpus,
        &[
            "seed_pack_memories",
            "seedPackMemories",
            "alpha_pack_memories",
        ],
    )
    .and_then(Value::as_i64)
    .or_else(|| first(settings, &["seed_pack_memories"]).and_then(Value::as_i64));
    let seed_total = first(
        &corpus,
        &["seed_pack_total_rows", "seedPackTotalRows", "total_rows"],
    )
    .and_then(Value::as_i64)
    .or_else(|| first(settings, &["seed_pack_total_rows"]).and_then(Value::as_i64));
    let source_counts = first(&corpus, &["source_counts", "sourceCounts"])
        .or_else(|| first(settings, &["source_counts", "sourceCounts"]))
        .cloned()
        .unwrap_or(Value::Null);

    // Extract ledger info from settings if available
    let ledger = settings.get("ledger").cloned().unwrap_or_else(|| json!({}));
    let ledger_valid = ledger.get("valid").and_then(Value::as_bool).unwrap_or(true);
    let ledger_events =
        first(&ledger, &["total_events", "totalEvents", "event_count"]).and_then(Value::as_i64);
    let ledger_hash = first(&ledger, &["head_hash", "headHash"])
        .cloned()
        .unwrap_or(Value::Null);

    json!({
        "daemon": {"base_url": "http://127.0.0.1:9101"},
        "auth": {"configured": session_configured},
        "corpus": {
            "memory_count": memory_count,
            "seed_pack_memories": seed_pack,
            "seed_pack_total_rows": seed_total,
            "source_counts": source_counts
        },
        "ledger": {
            "valid": ledger_valid,
            "total_events": ledger_events,
            "head_hash": ledger_hash
        },
        "providers": providers,
        "apps": {"requests": requests, "grants": grants},
        "permissions": settings.get("permissions").cloned().unwrap_or(Value::Null),
        "registries": settings.get("registries").cloned().unwrap_or(Value::Null),
        "identity": settings.get("identity").cloned().unwrap_or(Value::Null),
        "enrolled_agents": settings.get("enrolledAgents").cloned().unwrap_or_else(|| json!([])),
        "unenrolled_agents": settings.get("unenrolledAgents").cloned().unwrap_or_else(|| json!([])),
        "os_awareness": settings.get("osAwareness").cloned().unwrap_or(Value::Null),
        "personalization": settings.get("personalization").cloned().unwrap_or(Value::Null),
        "provider": settings.get("provider").cloned().unwrap_or(Value::Null),
        "git": settings.get("git").cloned().unwrap_or(Value::Null),
        "retrieval_calibration": settings.get("retrievalCalibration").cloned().unwrap_or(Value::Null),
        "config": config_rows
    })
}

fn app_request_payload_for_settings(app: &Value) -> Value {
    json!({
        "request_id": first(app, &["appId", "app_id"]).cloned().unwrap_or(Value::Null),
        "app_name": first(app, &["name", "appName"]).cloned().unwrap_or(Value::Null),
        "app_type": first(app, &["appType", "app_type"]).cloned().unwrap_or_else(|| json!("desktop")),
        "requested_scopes": app.get("scopes").cloned().unwrap_or_else(|| json!([])),
        "state": app.get("status").cloned().unwrap_or_else(|| json!("pending"))
    })
}

fn app_grant_payload_for_settings(app: &Value) -> Value {
    json!({
        "client_id": first(app, &["appId", "client_id"]).cloned().unwrap_or(Value::Null),
        "app_name": first(app, &["name", "appName"]).cloned().unwrap_or(Value::Null),
        "app_type": first(app, &["appType", "app_type"]).cloned().unwrap_or_else(|| json!("desktop")),
        "scopes": app.get("scopes").cloned().unwrap_or_else(|| json!([])),
        "active": true
    })
}

fn build_config_rows(settings: &Value) -> Vec<Value> {
    // Prefer the brain's own config array (from settings table rows)
    if let Some(arr) = settings.get("config").and_then(Value::as_array) {
        return arr
            .iter()
            .map(|row| {
                json!({
                    "key": first(row, &["key"]).cloned().unwrap_or(Value::Null),
                    "value": first(row, &["value"]).cloned().unwrap_or(Value::Null),
                    "updated_at": first(row, &["updated_at", "updatedAt", "updatedAtS", "updated_at_s"])
                        .cloned()
                        .unwrap_or_else(|| json!(0))
                })
            })
            .collect();
    }

    // Fallback: flatten nested settings objects into dotted keys
    let mut rows = Vec::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let skip_keys = [
        "ok",
        "daemon",
        "auth",
        "providers",
        "apps",
        "corpus",
        "ledger",
        "config",
        "defaultProvider",
    ];
    if let Some(obj) = settings.as_object() {
        for (key, value) in obj {
            if skip_keys.contains(&key.as_str()) {
                continue;
            }
            flatten_config(key, value, &mut rows, now);
        }
    }
    rows
}

fn flatten_config(prefix: &str, value: &Value, rows: &mut Vec<Value>, now: i64) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let dotted = format!("{prefix}.{k}");
                flatten_config(&dotted, v, rows, now);
            }
        }
        Value::Array(arr) => {
            rows.push(json!({
                "key": prefix,
                "value": Value::Array(arr.clone()),
                "updated_at": now
            }));
        }
        _ => {
            rows.push(json!({
                "key": prefix,
                "value": value.clone(),
                "updated_at": now
            }));
        }
    }
}

fn excerpt(value: &str) -> String {
    value.chars().take(280).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recall_maps_backend_memories_to_swift_results() {
        let output = adapt_response(
            "memory.recall",
            &json!({
                "query": "memory recall",
                "memories": [{"memory_id": "mem_1", "key": "test", "value": "hello", "score": 0.7, "created_at_s": 1700000000}]
            }),
        );
        assert_eq!(output["results"][0]["memory_id"], "mem_1");
        assert_eq!(output["results"][0]["memory"]["id"], "mem_1");
        assert_eq!(output["results"][0]["memory"]["source_kind"], "hom-local");
        assert_eq!(output["result_count"], 1);
    }

    #[test]
    fn memory_detail_wraps_memory_row() {
        let output = adapt_response(
            "memory.open",
            &json!({"ok": true, "memory": {"id": "mem_2", "key": "k", "value": "v", "created_at_s": 1}}),
        );
        assert_eq!(output["found"], true);
        assert_eq!(output["memory"]["id"], "mem_2");
        assert_eq!(output["entities"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn nightly_result_turns_numeric_applied_into_bool() {
        let output = nightly_result_payload(&json!({"proposals": 3, "applied": 0}));
        assert_eq!(output["proposal_count"], 3);
        assert_eq!(output["mutation_count"], 0);
        assert_eq!(output["applied"], false);
    }

    #[test]
    fn provider_probe_all_returns_counts() {
        let output = probe_all_payload(&json!({
            "providers": [
                {"detected": true, "health": {"modelCount": 2}},
                {"detected": false, "health": {"modelCount": 0}}
            ]
        }));
        assert_eq!(output["total"], 2);
        assert_eq!(output["detected"], 1);
        assert_eq!(output["available_models"], 2);
    }

    #[test]
    fn connect_kit_supplies_required_mcp_command() {
        let output = connect_kit_payload("obsidian", &json!({"instructions": "ok"}));
        assert_eq!(output["app_id"], "obsidian");
        assert_eq!(output["mcp"]["command"], "hom");
        assert!(output["mcp"]["args"].is_array());
    }
}
