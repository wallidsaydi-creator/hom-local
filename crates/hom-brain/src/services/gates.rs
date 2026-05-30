use std::collections::BTreeSet;

use hom_shared::{RpcError, canonical_json, rpc_err, sha256_hex};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::BrainState;
use crate::db::storage::{append_ledger_tx, unix_now_s};

const STATE_ALLOWED: &str = "allowed";
const STATE_BLOCKED_OUT_OF_SCOPE: &str = "blocked_out_of_scope";
const STATE_NEEDS_EVIDENCE: &str = "needs_evidence";
const STATE_NEEDS_RUNTIME_TRUTH: &str = "needs_runtime_truth";
const STATE_REQUIRES_AMENDMENT: &str = "requires_plan_amendment";
const STATE_COMPLETED_WITH_EVIDENCE: &str = "completed_with_evidence";

const POLICY_PLAN_REQUIRED: &str = "plan.required";
const POLICY_SCOPE_MATCH: &str = "task.scope_match_required";
const POLICY_AMENDMENT_REQUIRED: &str = "task.amendment_required";
const POLICY_RUNTIME_TRUTH: &str = "runtime.truth_required";
const POLICY_EVIDENCE_REQUIRED: &str = "task.evidence_required";
const POLICY_COMPLETION_PROOF: &str = "completion.proof_required";
const POLICY_UI_BACKEND_TRUTH: &str = "ui.backend_truth_required";
const POLICY_ARGUMENT_REQUIRED: &str = "completion.safety_argument_required";

#[derive(Clone, Debug)]
struct PlanVersionRow {
    id: String,
    plan_id: String,
    version: i64,
    payload: Value,
}

#[derive(Clone, Debug)]
struct TaskRow {
    id: String,
    plan_version_id: String,
    title: String,
    target_method: String,
    status: String,
    payload: Value,
}

pub fn prompt_create(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let prompt_text = string_field(&params, &["prompt_text", "prompt", "text"])
        .ok_or_else(|| rpc_err(-32602, "prompt_text_required"))?;
    let now = unix_now_s();
    let id = Uuid::new_v4().to_string();
    let payload = json!({
        "prompt_text": prompt_text,
        "origin": string_field(&params, &["origin"]).unwrap_or_else(|| "hom-local".to_string()),
        "preconditions": array_field(&params, &["preconditions"]),
        "postconditions": array_field(&params, &["postconditions"]),
        "invariants": array_field(&params, &["invariants"]),
        "failure_states": array_field(&params, &["failure_states"])
    });

    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("gate_prompt_tx: {e}")))?;
    tx.execute(
        "INSERT INTO gate_prompt_contracts
         (id, prompt_text, status, payload_json, created_at_s, updated_at_s)
         VALUES (?1, ?2, 'draft', ?3, ?4, ?4)",
        params![id, prompt_text, canonical(&payload)?, now],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_prompt_insert: {e}")))?;
    let ledger = append_gate_ledger(
        &tx,
        "gate.prompt.created",
        "brain.gates",
        Some(&id),
        json!({"prompt_id": id, "status": "draft"}),
        now,
    )?;
    tx.execute(
        "UPDATE gate_prompt_contracts SET ledger_event_id = ?1 WHERE id = ?2",
        params![ledger["event_id"].as_str(), id],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_prompt_ledger_update: {e}")))?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("gate_prompt_commit: {e}")))?;

    Ok(json!({
        "ok": true,
        "prompt_contract": {
            "id": id,
            "prompt_text": prompt_text,
            "status": "draft",
            "payload": payload,
            "created_at_s": now,
            "updated_at_s": now,
            "ledger": ledger
        }
    }))
}

pub fn plan_propose(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let title =
        string_field(&params, &["title", "name"]).unwrap_or_else(|| "Gate Plan".to_string());
    let prompt_id = string_field(&params, &["prompt_id", "promptId"]);
    let now = unix_now_s();
    let id = Uuid::new_v4().to_string();
    let payload = json!({
        "title": title,
        "prompt_id": prompt_id,
        "body": string_field(&params, &["body", "plan", "description"]).unwrap_or_default(),
        "allowed_scopes": string_array_field(&params, &["allowed_scopes", "allowedScopes", "scopes"]),
        "forbidden_scopes": string_array_field(&params, &["forbidden_scopes", "forbiddenScopes"]),
        "tasks": params.get("tasks").cloned().unwrap_or_else(|| json!([])),
        "claims": params.get("claims").cloned().unwrap_or_else(|| json!([])),
        "preconditions": array_field(&params, &["preconditions"]),
        "postconditions": array_field(&params, &["postconditions"]),
        "invariants": array_field(&params, &["invariants"]),
        "failure_states": array_field(&params, &["failure_states"])
    });

    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("gate_plan_tx: {e}")))?;
    tx.execute(
        "INSERT INTO gate_plan_contracts
         (id, prompt_id, title, status, payload_json, created_at_s, updated_at_s)
         VALUES (?1, ?2, ?3, 'proposed', ?4, ?5, ?5)",
        params![id, prompt_id, title, canonical(&payload)?, now],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_plan_insert: {e}")))?;
    let ledger = append_gate_ledger(
        &tx,
        "gate.plan.proposed",
        "brain.gates",
        Some(&id),
        json!({"plan_id": id, "status": "proposed"}),
        now,
    )?;
    tx.execute(
        "UPDATE gate_plan_contracts SET ledger_event_id = ?1 WHERE id = ?2",
        params![ledger["event_id"].as_str(), id],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_plan_ledger_update: {e}")))?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("gate_plan_commit: {e}")))?;

    Ok(json!({
        "ok": true,
        "plan_contract": {
            "id": id,
            "prompt_id": prompt_id,
            "title": title,
            "status": "proposed",
            "payload": payload,
            "created_at_s": now,
            "updated_at_s": now,
            "ledger": ledger
        }
    }))
}

pub fn plan_approve(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let plan_id = string_field(&params, &["plan_id", "planId", "id"])
        .ok_or_else(|| rpc_err(-32602, "plan_id_required"))?;
    let approved_by = string_field(&params, &["approved_by", "approvedBy", "actor"])
        .ok_or_else(|| rpc_err(-32602, "approved_by_required"))?;
    let now = unix_now_s();
    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("gate_plan_approve_tx: {e}")))?;
    let plan_payload_raw: String = tx
        .query_row(
            "SELECT payload_json FROM gate_plan_contracts WHERE id = ?1",
            params![plan_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| rpc_err(-32603, format!("gate_plan_lookup: {e}")))?
        .ok_or_else(|| rpc_err(-32602, "plan_not_found"))?;
    let plan_payload: Value = serde_json::from_str(&plan_payload_raw).unwrap_or_else(|_| json!({}));
    let next_version = tx
        .query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM gate_plan_versions WHERE plan_id = ?1",
            params![plan_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| rpc_err(-32603, format!("gate_plan_version_lookup: {e}")))?;
    let version_id = Uuid::new_v4().to_string();
    let version_payload = json!({
        "plan_id": plan_id,
        "version": next_version,
        "approved_by": approved_by,
        "approved_at_s": now,
        "contract": plan_payload
    });
    tx.execute(
        "INSERT INTO gate_plan_versions
         (id, plan_id, version, status, approved_by, approved_at_s, payload_json, created_at_s)
         VALUES (?1, ?2, ?3, 'approved_locked', ?4, ?5, ?6, ?5)",
        params![
            version_id,
            plan_id,
            next_version,
            approved_by,
            now,
            canonical(&version_payload)?
        ],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_plan_version_insert: {e}")))?;
    tx.execute(
        "UPDATE gate_plan_contracts SET status = 'approved', updated_at_s = ?1 WHERE id = ?2",
        params![now, plan_id],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_plan_status_update: {e}")))?;
    let ledger = append_gate_ledger(
        &tx,
        "gate.plan.approved",
        &approved_by,
        Some(&version_id),
        json!({"plan_id": plan_id, "plan_version_id": version_id, "version": next_version}),
        now,
    )?;
    tx.execute(
        "UPDATE gate_plan_versions SET ledger_event_id = ?1 WHERE id = ?2",
        params![ledger["event_id"].as_str(), version_id],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_plan_version_ledger_update: {e}")))?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("gate_plan_approve_commit: {e}")))?;

    Ok(json!({
        "ok": true,
        "plan_version": {
            "id": version_id,
            "plan_id": plan_id,
            "version": next_version,
            "status": "approved_locked",
            "approved_by": approved_by,
            "approved_at_s": now,
            "payload": version_payload,
            "ledger": ledger
        }
    }))
}

pub fn plan_current(state: &BrainState) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let Some(plan_version) = current_plan_version(&conn)? else {
        return Ok(json!({"ok": true, "plan_version": Value::Null}));
    };
    Ok(json!({"ok": true, "plan_version": plan_version_json(&plan_version)}))
}

pub fn tasks_derive(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("gate_tasks_tx: {e}")))?;
    let plan_version = plan_version_from_params(&tx, &params)?;
    let task_inputs = if let Some(tasks) = params.get("tasks").and_then(Value::as_array) {
        tasks.clone()
    } else {
        plan_version
            .payload
            .get("contract")
            .and_then(|v| v.get("tasks"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    if task_inputs.is_empty() {
        return Err(rpc_err(-32602, "tasks_required"));
    }

    let now = unix_now_s();
    let mut tasks = Vec::new();
    for task in task_inputs {
        let title =
            string_field(&task, &["title", "name"]).unwrap_or_else(|| "Gate Task".to_string());
        let target_method = string_field(&task, &["target_method", "targetMethod", "method"])
            .ok_or_else(|| rpc_err(-32602, "task_target_method_required"))?;
        if !known_target_method(&target_method) {
            return Err(rpc_err(
                -32602,
                format!("unknown_target_method: {target_method}"),
            ));
        }
        let allowed_scopes = scopes_for_task(&task, &target_method);
        if allowed_scopes.is_empty() {
            return Err(rpc_err(-32602, "task_allowed_scope_required"));
        }
        let task_id = Uuid::new_v4().to_string();
        let payload = json!({
            "title": title,
            "target_method": target_method,
            "allowed_scopes": allowed_scopes,
            "forbidden_scopes": string_array_field(&task, &["forbidden_scopes", "forbiddenScopes"]),
            "preconditions": array_field(&task, &["preconditions"]),
            "postconditions": array_field(&task, &["postconditions"]),
            "invariants": array_field(&task, &["invariants"]),
            "failure_states": array_field(&task, &["failure_states"]),
            "required_evidence_types": required_evidence_types(&task),
            "acceptance": task.get("acceptance").cloned().unwrap_or(Value::Null)
        });
        tx.execute(
            "INSERT INTO gate_task_contracts
             (id, plan_version_id, title, target_method, status, payload_json, created_at_s, updated_at_s)
             VALUES (?1, ?2, ?3, ?4, 'ready', ?5, ?6, ?6)",
            params![
                task_id,
                plan_version.id,
                title,
                target_method,
                canonical(&payload)?,
                now
            ],
        )
        .map_err(|e| rpc_err(-32603, format!("gate_task_insert: {e}")))?;
        let ledger = append_gate_ledger(
            &tx,
            "gate.task.derived",
            "brain.gates",
            Some(&task_id),
            json!({"task_id": task_id, "plan_version_id": plan_version.id}),
            now,
        )?;
        tx.execute(
            "UPDATE gate_task_contracts SET ledger_event_id = ?1 WHERE id = ?2",
            params![ledger["event_id"].as_str(), task_id],
        )
        .map_err(|e| rpc_err(-32603, format!("gate_task_ledger_update: {e}")))?;
        tasks.push(task_json(&TaskRow {
            id: task_id,
            plan_version_id: plan_version.id.clone(),
            title,
            target_method,
            status: "ready".to_string(),
            payload,
        }));
    }
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("gate_tasks_commit: {e}")))?;
    Ok(json!({"ok": true, "plan_version": plan_version_json(&plan_version), "tasks": tasks}))
}

pub fn tasks_preflight(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let task_id = string_field(&params, &["task_id", "taskId"]);
    let target_method =
        string_field(&params, &["target_method", "targetMethod", "method"]).unwrap_or_default();
    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("gate_preflight_tx: {e}")))?;
    let now = unix_now_s();

    let result = if !known_target_method(&target_method) {
        insert_decision(
            &tx,
            DecisionInput {
                plan_version_id: None,
                task_id: task_id.clone(),
                state: STATE_BLOCKED_OUT_OF_SCOPE,
                policy_id: POLICY_PLAN_REQUIRED,
                target_method: Some(target_method.clone()),
                reason: "unknown target method is default-denied".to_string(),
                next_action: Some("create_or_amend_plan".to_string()),
                observed_facts: json!({"known_target_method": false}),
                payload: params.clone(),
                actor: "brain.gates",
            },
            now,
        )?
    } else if let Some(task_id) = task_id {
        match load_task(&tx, &task_id)? {
            None => insert_decision(
                &tx,
                DecisionInput {
                    plan_version_id: None,
                    task_id: None,
                    state: STATE_BLOCKED_OUT_OF_SCOPE,
                    policy_id: POLICY_PLAN_REQUIRED,
                    target_method: Some(target_method.clone()),
                    reason: "task does not trace to a locked plan version".to_string(),
                    next_action: Some("derive_task_from_locked_plan".to_string()),
                    observed_facts: json!({"task_found": false, "requested_task_id": task_id}),
                    payload: params.clone(),
                    actor: "brain.gates",
                },
                now,
            )?,
            Some(task) => preflight_existing_task(state, &tx, &task, &target_method, &params, now)?,
        }
    } else {
        insert_decision(
            &tx,
            DecisionInput {
                plan_version_id: None,
                task_id: None,
                state: STATE_BLOCKED_OUT_OF_SCOPE,
                policy_id: POLICY_PLAN_REQUIRED,
                target_method: Some(target_method.clone()),
                reason: "mutating gate preflight requires a task id".to_string(),
                next_action: Some("derive_task_from_locked_plan".to_string()),
                observed_facts: json!({"task_id_present": false}),
                payload: params.clone(),
                actor: "brain.gates",
            },
            now,
        )?
    };
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("gate_preflight_commit: {e}")))?;
    Ok(json!({"ok": true, "decision": result}))
}

pub fn amendment_request(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let task_id = string_field(&params, &["task_id", "taskId"]);
    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("gate_amendment_tx: {e}")))?;
    let plan_version = if let Some(task_id) = task_id.as_deref() {
        let task = load_task(&tx, task_id)?.ok_or_else(|| rpc_err(-32602, "task_not_found"))?;
        load_plan_version(&tx, &task.plan_version_id)?
            .ok_or_else(|| rpc_err(-32602, "plan_version_not_found"))?
    } else {
        plan_version_from_params(&tx, &params)?
    };
    let requested_scopes = string_array_field(
        &params,
        &["requested_scopes", "requestedScopes", "scopes", "scope"],
    );
    if requested_scopes.is_empty() {
        return Err(rpc_err(-32602, "requested_scope_required"));
    }
    let amendment = insert_amendment(
        &tx,
        &plan_version.id,
        task_id.as_deref(),
        requested_scopes,
        &params,
    )?;
    let decision = insert_decision(
        &tx,
        DecisionInput {
            plan_version_id: Some(plan_version.id.clone()),
            task_id: task_id.clone(),
            state: STATE_REQUIRES_AMENDMENT,
            policy_id: POLICY_AMENDMENT_REQUIRED,
            target_method: string_field(&params, &["target_method", "targetMethod", "method"]),
            reason: "requested scope is outside the locked task contract".to_string(),
            next_action: Some("approve_amendment".to_string()),
            observed_facts: json!({"amendment_id": amendment["id"]}),
            payload: params.clone(),
            actor: "brain.gates",
        },
        unix_now_s(),
    )?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("gate_amendment_commit: {e}")))?;
    Ok(json!({"ok": true, "amendment": amendment, "decision": decision}))
}

pub fn amendment_approve(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let id = string_field(&params, &["amendment_id", "amendmentId", "id"])
        .ok_or_else(|| rpc_err(-32602, "amendment_id_required"))?;
    let approved_by = string_field(&params, &["approved_by", "approvedBy", "actor"])
        .ok_or_else(|| rpc_err(-32602, "approved_by_required"))?;
    let now = unix_now_s();
    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("gate_amendment_approve_tx: {e}")))?;
    let rows = tx
        .execute(
            "UPDATE gate_amendments SET status = 'approved', updated_at_s = ?1 WHERE id = ?2 AND status = 'pending'",
            params![now, id],
        )
        .map_err(|e| rpc_err(-32603, format!("gate_amendment_approve_update: {e}")))?;
    if rows == 0 {
        return Err(rpc_err(-32602, "pending_amendment_not_found"));
    }
    let ledger = append_gate_ledger(
        &tx,
        "gate.amendment.approved",
        &approved_by,
        Some(&id),
        json!({"amendment_id": id, "status": "approved"}),
        now,
    )?;
    tx.execute(
        "UPDATE gate_amendments SET ledger_event_id = ?1 WHERE id = ?2",
        params![ledger["event_id"].as_str(), id],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_amendment_ledger_update: {e}")))?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("gate_amendment_approve_commit: {e}")))?;
    Ok(
        json!({"ok": true, "amendment": {"id": id, "status": "approved", "approved_by": approved_by, "updated_at_s": now, "ledger": ledger}}),
    )
}

pub fn runtime_snapshot(state: &BrainState) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let memory_count = count_table(&conn, "memories")?;
    let ledger_count = count_table(&conn, "ledger_events")?;
    let task_count = count_table(&conn, "gate_task_contracts")?;
    let evidence_count = count_table(&conn, "gate_evidence_records")?;
    let decision_count = count_table(&conn, "gate_decisions")?;
    let security_events: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM ledger_events WHERE event_type LIKE 'security.%'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| rpc_err(-32603, format!("gate_security_count: {e}")))?;
    let last_decision = latest_decision(&conn)?;
    Ok(json!({
        "ok": true,
        "snapshot": {
            "observed_at_s": unix_now_s(),
            "memory": {"memory_count": memory_count, "ledger_count": ledger_count},
            "gates": {"task_count": task_count, "evidence_count": evidence_count, "decision_count": decision_count},
            "security": {"security_event_count": security_events},
            "last_decision": last_decision
        }
    }))
}

pub fn runtime_verify(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let claim_state = string_field(&params, &["claim_state", "claimState", "state"])
        .ok_or_else(|| rpc_err(-32602, "claim_state_required"))?;
    let task_id = string_field(&params, &["task_id", "taskId"]);
    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("gate_runtime_verify_tx: {e}")))?;
    let last = if let Some(task_id) = task_id.as_deref() {
        latest_decision_for_task(&tx, task_id)?
    } else {
        latest_decision_tx(&tx)?
    };
    let accepted = match claim_state.as_str() {
        "complete" | "completed" => {
            last.as_ref()
                .and_then(|d| d.get("decision_state").and_then(Value::as_str))
                == Some(STATE_COMPLETED_WITH_EVIDENCE)
        }
        "ready" | "passed" | "allowed" => last
            .as_ref()
            .and_then(|d| d.get("decision_state").and_then(Value::as_str))
            .is_some_and(|state| state == STATE_ALLOWED || state == STATE_COMPLETED_WITH_EVIDENCE),
        _ => false,
    };
    let decision = insert_decision(
        &tx,
        DecisionInput {
            plan_version_id: last
                .as_ref()
                .and_then(|d| string_field(d, &["plan_version_id"])),
            task_id: task_id.clone(),
            state: if accepted {
                STATE_ALLOWED
            } else {
                STATE_NEEDS_RUNTIME_TRUTH
            },
            policy_id: POLICY_UI_BACKEND_TRUTH,
            target_method: Some("ui.claim".to_string()),
            reason: if accepted {
                "ui claim is backed by the latest backend gate decision".to_string()
            } else {
                "ui claim has no supporting backend gate decision".to_string()
            },
            next_action: (!accepted).then(|| "request_runtime_truth".to_string()),
            observed_facts: json!({"claim_state": claim_state, "latest_decision": last}),
            payload: params,
            actor: "brain.gates",
        },
        unix_now_s(),
    )?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("gate_runtime_verify_commit: {e}")))?;
    Ok(json!({"ok": true, "decision": decision}))
}

pub fn decisions_list(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let limit = params
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(50)
        .clamp(1, 200);
    let task_id = string_field(&params, &["task_id", "taskId"]);
    let conn = state.store.conn()?;
    let decisions = if let Some(task_id) = task_id {
        let mut stmt = conn
            .prepare(
                "SELECT id, plan_version_id, task_id, decision_state, policy_id, target_method, reason, next_action, observed_facts_json, payload_json, created_at_s, ledger_event_id
                 FROM gate_decisions WHERE task_id = ?1 ORDER BY created_at_s DESC LIMIT ?2",
            )
            .map_err(|e| rpc_err(-32603, format!("gate_decisions_prepare: {e}")))?;
        stmt.query_map(params![task_id, limit], decision_from_row)
            .map_err(|e| rpc_err(-32603, format!("gate_decisions_query: {e}")))?
            .filter_map(Result::ok)
            .collect::<Vec<_>>()
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT id, plan_version_id, task_id, decision_state, policy_id, target_method, reason, next_action, observed_facts_json, payload_json, created_at_s, ledger_event_id
                 FROM gate_decisions ORDER BY created_at_s DESC LIMIT ?1",
            )
            .map_err(|e| rpc_err(-32603, format!("gate_decisions_prepare: {e}")))?;
        stmt.query_map(params![limit], decision_from_row)
            .map_err(|e| rpc_err(-32603, format!("gate_decisions_query: {e}")))?
            .filter_map(Result::ok)
            .collect::<Vec<_>>()
    };
    Ok(json!({"ok": true, "decisions": decisions}))
}

pub fn evidence_submit(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let task_id = string_field(&params, &["task_id", "taskId"])
        .ok_or_else(|| rpc_err(-32602, "task_id_required"))?;
    let source_type = string_field(&params, &["source_type", "sourceType", "type"])
        .ok_or_else(|| rpc_err(-32602, "source_type_required"))?;
    if !allowed_evidence_type(&source_type) {
        return Err(rpc_err(
            -32602,
            format!("unknown_evidence_type: {source_type}"),
        ));
    }
    let source = string_field(&params, &["source"]).unwrap_or_else(|| source_type.clone());
    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("gate_evidence_tx: {e}")))?;
    let task = load_task(&tx, &task_id)?.ok_or_else(|| rpc_err(-32602, "task_not_found"))?;
    let plan_version_id = task.plan_version_id.clone();
    let payload = params
        .get("payload")
        .cloned()
        .unwrap_or_else(|| params.clone());
    let artifact_hash = string_field(&params, &["artifact_hash", "artifactHash"])
        .unwrap_or_else(|| sha256_hex(&canonical(&payload).unwrap_or_default()));
    let custody = params.get("custody").cloned().unwrap_or_else(|| json!({
        "submitted_by": string_field(&params, &["submitted_by", "submittedBy"]).unwrap_or_else(|| "local-user".to_string()),
        "recorded_by": "hom-brain"
    }));
    let status = if evidence_success(&source_type, &payload) {
        "passed"
    } else {
        "failed"
    };
    let now = unix_now_s();
    let id = Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO gate_evidence_records
         (id, plan_version_id, task_id, source_type, source, artifact_hash, custody_json, payload_json, status, created_at_s)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            plan_version_id,
            task_id,
            source_type,
            source,
            artifact_hash,
            canonical(&custody)?,
            canonical(&payload)?,
            status,
            now
        ],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_evidence_insert: {e}")))?;
    let ledger = append_gate_ledger(
        &tx,
        "gate.evidence.submitted",
        "brain.gates",
        Some(&id),
        json!({"evidence_id": id, "task_id": task.id, "source_type": source_type, "status": status, "artifact_hash": artifact_hash}),
        now,
    )?;
    tx.execute(
        "UPDATE gate_evidence_records SET ledger_event_id = ?1 WHERE id = ?2",
        params![ledger["event_id"].as_str(), id],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_evidence_ledger_update: {e}")))?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("gate_evidence_commit: {e}")))?;
    Ok(json!({
        "ok": true,
        "evidence": {
            "id": id,
            "plan_version_id": plan_version_id,
            "task_id": task.id,
            "source_type": source_type,
            "source": source,
            "artifact_hash": artifact_hash,
            "custody": custody,
            "payload": payload,
            "status": status,
            "created_at_s": now,
            "ledger": ledger
        }
    }))
}

pub fn evidence_list(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let task_id = string_field(&params, &["task_id", "taskId"]);
    let conn = state.store.conn()?;
    let evidence = if let Some(task_id) = task_id {
        let mut stmt = conn
            .prepare(
                "SELECT id, plan_version_id, task_id, source_type, source, artifact_hash, custody_json, payload_json, status, created_at_s, ledger_event_id
                 FROM gate_evidence_records WHERE task_id = ?1 ORDER BY created_at_s DESC",
            )
            .map_err(|e| rpc_err(-32603, format!("gate_evidence_prepare: {e}")))?;
        stmt.query_map(params![task_id], evidence_from_row)
            .map_err(|e| rpc_err(-32603, format!("gate_evidence_query: {e}")))?
            .filter_map(Result::ok)
            .collect::<Vec<_>>()
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT id, plan_version_id, task_id, source_type, source, artifact_hash, custody_json, payload_json, status, created_at_s, ledger_event_id
                 FROM gate_evidence_records ORDER BY created_at_s DESC LIMIT 200",
            )
            .map_err(|e| rpc_err(-32603, format!("gate_evidence_prepare: {e}")))?;
        stmt.query_map([], evidence_from_row)
            .map_err(|e| rpc_err(-32603, format!("gate_evidence_query: {e}")))?
            .filter_map(Result::ok)
            .collect::<Vec<_>>()
    };
    Ok(json!({"ok": true, "evidence": evidence}))
}

pub fn tasks_complete(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let task_id = string_field(&params, &["task_id", "taskId"])
        .ok_or_else(|| rpc_err(-32602, "task_id_required"))?;
    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("gate_complete_tx: {e}")))?;
    let task = load_task(&tx, &task_id)?.ok_or_else(|| rpc_err(-32602, "task_not_found"))?;
    let evidence_ids = string_array_field(&params, &["evidence_ids", "evidenceIds"]);
    let evidence = matching_evidence(&tx, &task, &evidence_ids)?;
    let required = evidence_required_for_task(&task);
    let missing = missing_evidence_types(&required, &evidence);
    let now = unix_now_s();
    if !missing.is_empty() || evidence.is_empty() {
        let decision = insert_decision(
            &tx,
            DecisionInput {
                plan_version_id: Some(task.plan_version_id.clone()),
                task_id: Some(task.id.clone()),
                state: STATE_NEEDS_EVIDENCE,
                policy_id: POLICY_EVIDENCE_REQUIRED,
                target_method: Some(task.target_method.clone()),
                reason: "completion claim does not carry the required task-bound evidence"
                    .to_string(),
                next_action: Some("submit_matching_evidence".to_string()),
                observed_facts: json!({"required_evidence_types": required, "missing_evidence_types": missing, "evidence_count": evidence.len()}),
                payload: params,
                actor: "brain.gates",
            },
            now,
        )?;
        tx.commit()
            .map_err(|e| rpc_err(-32603, format!("gate_complete_needs_evidence_commit: {e}")))?;
        return Ok(json!({"ok": true, "decision": decision}));
    }
    if let Some(failed) = evidence.iter().find(|item| item["status"] != "passed") {
        let decision = insert_decision(
            &tx,
            DecisionInput {
                plan_version_id: Some(task.plan_version_id.clone()),
                task_id: Some(task.id.clone()),
                state: STATE_NEEDS_RUNTIME_TRUTH,
                policy_id: POLICY_RUNTIME_TRUTH,
                target_method: Some(task.target_method.clone()),
                reason: "completion evidence exists but one or more artifacts failed".to_string(),
                next_action: Some("rerun_runtime_probe_or_command".to_string()),
                observed_facts: json!({"failed_evidence_id": failed["id"]}),
                payload: params,
                actor: "brain.gates",
            },
            now,
        )?;
        tx.commit()
            .map_err(|e| rpc_err(-32603, format!("gate_complete_failed_evidence_commit: {e}")))?;
        return Ok(json!({"ok": true, "decision": decision}));
    }
    tx.execute(
        "UPDATE gate_task_contracts SET status = 'completed', updated_at_s = ?1 WHERE id = ?2",
        params![now, task.id],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_task_complete_update: {e}")))?;
    let argument = build_safety_argument_tx(&tx, &task.plan_version_id, Some(&task.id), now)?;
    let decision = insert_decision(
        &tx,
        DecisionInput {
            plan_version_id: Some(task.plan_version_id.clone()),
            task_id: Some(task.id.clone()),
            state: STATE_COMPLETED_WITH_EVIDENCE,
            policy_id: POLICY_COMPLETION_PROOF,
            target_method: Some(task.target_method.clone()),
            reason:
                "completion claim is backed by task-bound successful evidence and a safety argument"
                    .to_string(),
            next_action: None,
            observed_facts: json!({"evidence_count": evidence.len(), "safety_argument_id": argument["id"]}),
            payload: json!({"completion": params, "evidence": evidence, "safety_argument": argument}),
            actor: "brain.gates",
        },
        now,
    )?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("gate_complete_commit: {e}")))?;
    Ok(json!({"ok": true, "decision": decision, "safety_argument": argument}))
}

pub fn argument_build(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("gate_argument_tx: {e}")))?;
    let task_id = string_field(&params, &["task_id", "taskId"]);
    let plan_version_id = if let Some(task_id) = task_id.as_deref() {
        load_task(&tx, task_id)?
            .ok_or_else(|| rpc_err(-32602, "task_not_found"))?
            .plan_version_id
    } else {
        plan_version_from_params(&tx, &params)?.id
    };
    let argument =
        build_safety_argument_tx(&tx, &plan_version_id, task_id.as_deref(), unix_now_s())?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("gate_argument_commit: {e}")))?;
    Ok(json!({"ok": true, "argument": argument}))
}

pub fn argument_inspect(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let task_id = string_field(&params, &["task_id", "taskId"]);
    let plan_version_id = string_field(&params, &["plan_version_id", "planVersionId"]);
    let conn = state.store.conn()?;
    let argument = if let Some(task_id) = task_id {
        conn.query_row(
            "SELECT id, plan_version_id, task_id, status, argument_json, created_at_s, updated_at_s, ledger_event_id
             FROM gate_safety_arguments WHERE task_id = ?1 ORDER BY created_at_s DESC LIMIT 1",
            params![task_id],
            safety_argument_from_row,
        )
        .optional()
        .map_err(|e| rpc_err(-32603, format!("gate_argument_query: {e}")))?
    } else if let Some(plan_version_id) = plan_version_id {
        conn.query_row(
            "SELECT id, plan_version_id, task_id, status, argument_json, created_at_s, updated_at_s, ledger_event_id
             FROM gate_safety_arguments WHERE plan_version_id = ?1 ORDER BY created_at_s DESC LIMIT 1",
            params![plan_version_id],
            safety_argument_from_row,
        )
        .optional()
        .map_err(|e| rpc_err(-32603, format!("gate_argument_query: {e}")))?
    } else {
        None
    };
    Ok(json!({"ok": true, "argument": argument}))
}

pub fn argument_accept(state: &BrainState, params: Value) -> Result<Value, RpcError> {
    let id = string_field(&params, &["argument_id", "argumentId", "id"])
        .ok_or_else(|| rpc_err(-32602, "argument_id_required"))?;
    let accepted_by = string_field(&params, &["accepted_by", "acceptedBy", "actor"])
        .ok_or_else(|| rpc_err(-32602, "accepted_by_required"))?;
    let now = unix_now_s();
    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("gate_argument_accept_tx: {e}")))?;
    let argument = tx
        .query_row(
            "SELECT id, plan_version_id, task_id, status, argument_json, created_at_s, updated_at_s, ledger_event_id
             FROM gate_safety_arguments WHERE id = ?1",
            params![id],
            safety_argument_from_row,
        )
        .optional()
        .map_err(|e| rpc_err(-32603, format!("gate_argument_lookup: {e}")))?
        .ok_or_else(|| rpc_err(-32602, "argument_not_found"))?;
    if argument["status"] != "satisfied" {
        let decision = insert_decision(
            &tx,
            DecisionInput {
                plan_version_id: string_field(&argument, &["plan_version_id"]),
                task_id: string_field(&argument, &["task_id"]),
                state: STATE_NEEDS_EVIDENCE,
                policy_id: POLICY_ARGUMENT_REQUIRED,
                target_method: Some("gates.argument.accept".to_string()),
                reason: "safety argument cannot be accepted while subclaims are unsatisfied"
                    .to_string(),
                next_action: Some("satisfy_argument_evidence".to_string()),
                observed_facts: json!({"argument_id": id, "argument_status": argument["status"]}),
                payload: params,
                actor: "brain.gates",
            },
            now,
        )?;
        tx.commit()
            .map_err(|e| rpc_err(-32603, format!("gate_argument_accept_block_commit: {e}")))?;
        return Ok(json!({"ok": true, "decision": decision, "argument": argument}));
    }
    let ledger = append_gate_ledger(
        &tx,
        "gate.argument.accepted",
        &accepted_by,
        Some(&id),
        json!({"argument_id": id, "accepted_by": accepted_by}),
        now,
    )?;
    tx.execute(
        "UPDATE gate_safety_arguments SET status = 'accepted', updated_at_s = ?1, ledger_event_id = ?2 WHERE id = ?3",
        params![now, ledger["event_id"].as_str(), id],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_argument_accept_update: {e}")))?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("gate_argument_accept_commit: {e}")))?;
    Ok(
        json!({"ok": true, "argument": {"id": id, "status": "accepted", "accepted_by": accepted_by, "ledger": ledger}}),
    )
}

pub fn enforce_attached_preflight(
    state: &BrainState,
    params: &Value,
    target_method: &str,
) -> Result<(), RpcError> {
    let Some(task_id) = string_field(params, &["task_id", "taskId", "gate_task_id", "gateTaskId"])
    else {
        return Ok(());
    };
    let decision_id = string_field(
        params,
        &[
            "gate_decision_id",
            "gateDecisionId",
            "decision_id",
            "decisionId",
        ],
    )
    .ok_or_else(|| gate_required_error(&task_id, target_method, "gate_decision_id_required"))?;
    let conn = state.store.conn()?;
    let decision = conn
        .query_row(
            "SELECT id, plan_version_id, task_id, decision_state, policy_id, target_method, reason, next_action, observed_facts_json, payload_json, created_at_s, ledger_event_id
             FROM gate_decisions WHERE id = ?1",
            params![decision_id],
            decision_from_row,
        )
        .optional()
        .map_err(|e| rpc_err(-32603, format!("gate_decision_lookup: {e}")))?
        .ok_or_else(|| gate_required_error(&task_id, target_method, "gate_decision_not_found"))?;
    let matches_task = decision["task_id"].as_str() == Some(task_id.as_str());
    let matches_target = decision["target_method"].as_str() == Some(target_method);
    let allowed = decision["decision_state"].as_str() == Some(STATE_ALLOWED);
    if allowed && matches_task && matches_target {
        Ok(())
    } else {
        Err(gate_required_error(
            &task_id,
            target_method,
            "gate_decision_not_allowed_for_action",
        ))
    }
}

fn preflight_existing_task(
    _state: &BrainState,
    tx: &Transaction<'_>,
    task: &TaskRow,
    target_method: &str,
    params: &Value,
    now: i64,
) -> Result<Value, RpcError> {
    let target_match = task.target_method == target_method;
    if !target_match {
        return insert_decision(
            tx,
            DecisionInput {
                plan_version_id: Some(task.plan_version_id.clone()),
                task_id: Some(task.id.clone()),
                state: STATE_BLOCKED_OUT_OF_SCOPE,
                policy_id: POLICY_SCOPE_MATCH,
                target_method: Some(target_method.to_string()),
                reason: "target method is outside the task contract".to_string(),
                next_action: Some("derive_or_amend_task".to_string()),
                observed_facts: json!({"task_target_method": task.target_method, "requested_target_method": target_method}),
                payload: params.clone(),
                actor: "brain.gates",
            },
            now,
        );
    }
    let requested_scopes = requested_scopes_for(params, target_method);
    let allowed_scopes = value_string_set(&task.payload["allowed_scopes"]);
    let requested_set: BTreeSet<_> = requested_scopes.iter().cloned().collect();
    let scope_match = requested_set.is_subset(&allowed_scopes);
    let amendment_match = if scope_match {
        false
    } else {
        approved_amendment_covers(tx, &task.id, &requested_scopes)?
    };
    if !scope_match && !amendment_match {
        let amendment = insert_amendment(
            tx,
            &task.plan_version_id,
            Some(&task.id),
            requested_scopes.clone(),
            params,
        )?;
        return insert_decision(
            tx,
            DecisionInput {
                plan_version_id: Some(task.plan_version_id.clone()),
                task_id: Some(task.id.clone()),
                state: STATE_REQUIRES_AMENDMENT,
                policy_id: POLICY_AMENDMENT_REQUIRED,
                target_method: Some(target_method.to_string()),
                reason: "requested scope is not included in the locked task contract".to_string(),
                next_action: Some("approve_amendment".to_string()),
                observed_facts: json!({
                    "requested_scopes": requested_scopes,
                    "allowed_scopes": allowed_scopes.into_iter().collect::<Vec<_>>(),
                    "amendment_id": amendment["id"]
                }),
                payload: params.clone(),
                actor: "brain.gates",
            },
            now,
        );
    }
    insert_decision(
        tx,
        DecisionInput {
            plan_version_id: Some(task.plan_version_id.clone()),
            task_id: Some(task.id.clone()),
            state: STATE_ALLOWED,
            policy_id: POLICY_SCOPE_MATCH,
            target_method: Some(target_method.to_string()),
            reason: "target method and scope match the locked task contract".to_string(),
            next_action: None,
            observed_facts: json!({
                "requested_scopes": requested_scopes,
                "scope_match": scope_match,
                "approved_amendment_match": amendment_match,
                "task_status": task.status
            }),
            payload: params.clone(),
            actor: "brain.gates",
        },
        now,
    )
}

struct DecisionInput<'a> {
    plan_version_id: Option<String>,
    task_id: Option<String>,
    state: &'a str,
    policy_id: &'a str,
    target_method: Option<String>,
    reason: String,
    next_action: Option<String>,
    observed_facts: Value,
    payload: Value,
    actor: &'a str,
}

fn insert_decision(
    tx: &Transaction<'_>,
    input: DecisionInput<'_>,
    now: i64,
) -> Result<Value, RpcError> {
    let id = Uuid::new_v4().to_string();
    let decision_payload = json!({
        "id": id,
        "plan_version_id": input.plan_version_id,
        "task_id": input.task_id,
        "decision_state": input.state,
        "policy_id": input.policy_id,
        "target_method": input.target_method,
        "reason": input.reason,
        "next_action": input.next_action,
        "observed_facts": input.observed_facts,
        "payload": input.payload,
        "created_at_s": now
    });
    let ledger = append_gate_ledger(
        tx,
        "gate.decision.recorded",
        input.actor,
        Some(&id),
        decision_payload.clone(),
        now,
    )?;
    tx.execute(
        "INSERT INTO gate_decisions
         (id, plan_version_id, task_id, decision_state, policy_id, target_method, reason, next_action, observed_facts_json, payload_json, created_at_s, ledger_event_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            id,
            decision_payload["plan_version_id"].as_str(),
            decision_payload["task_id"].as_str(),
            input.state,
            input.policy_id,
            decision_payload["target_method"].as_str(),
            decision_payload["reason"].as_str(),
            decision_payload["next_action"].as_str(),
            canonical(&decision_payload["observed_facts"])?,
            canonical(&decision_payload["payload"])?,
            now,
            ledger["event_id"].as_str()
        ],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_decision_insert: {e}")))?;
    Ok(json!({
        "id": id,
        "plan_version_id": decision_payload["plan_version_id"],
        "task_id": decision_payload["task_id"],
        "decision_state": input.state,
        "policy_id": input.policy_id,
        "target_method": decision_payload["target_method"],
        "reason": decision_payload["reason"],
        "next_action": decision_payload["next_action"],
        "observed_facts": decision_payload["observed_facts"],
        "payload": decision_payload["payload"],
        "created_at_s": now,
        "ledger": ledger
    }))
}

fn insert_amendment(
    tx: &Transaction<'_>,
    plan_version_id: &str,
    task_id: Option<&str>,
    requested_scopes: Vec<String>,
    payload: &Value,
) -> Result<Value, RpcError> {
    let now = unix_now_s();
    let id = Uuid::new_v4().to_string();
    let amendment_payload = json!({
        "requested_scopes": requested_scopes,
        "target_method": string_field(payload, &["target_method", "targetMethod", "method"]),
        "reason": string_field(payload, &["reason"]).unwrap_or_else(|| "scope expansion requested".to_string())
    });
    tx.execute(
        "INSERT INTO gate_amendments
         (id, plan_version_id, task_id, status, requested_scope_json, payload_json, created_at_s, updated_at_s)
         VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6, ?6)",
        params![
            id,
            plan_version_id,
            task_id,
            canonical(&amendment_payload["requested_scopes"])?,
            canonical(&amendment_payload)?,
            now
        ],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_amendment_insert: {e}")))?;
    let ledger = append_gate_ledger(
        tx,
        "gate.amendment.requested",
        "brain.gates",
        Some(&id),
        json!({"amendment_id": id, "plan_version_id": plan_version_id, "task_id": task_id, "requested_scopes": amendment_payload["requested_scopes"]}),
        now,
    )?;
    tx.execute(
        "UPDATE gate_amendments SET ledger_event_id = ?1 WHERE id = ?2",
        params![ledger["event_id"].as_str(), id],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_amendment_ledger_update: {e}")))?;
    Ok(json!({
        "id": id,
        "plan_version_id": plan_version_id,
        "task_id": task_id,
        "status": "pending",
        "requested_scopes": amendment_payload["requested_scopes"],
        "payload": amendment_payload,
        "created_at_s": now,
        "updated_at_s": now,
        "ledger": ledger
    }))
}

fn build_safety_argument_tx(
    tx: &Transaction<'_>,
    plan_version_id: &str,
    task_id: Option<&str>,
    now: i64,
) -> Result<Value, RpcError> {
    let tasks = load_tasks_for_argument(tx, plan_version_id, task_id)?;
    let mut subclaims = Vec::new();
    let mut unresolved = Vec::new();
    for task in tasks {
        let evidence = matching_evidence(tx, &task, &Vec::new())?;
        let required = evidence_required_for_task(&task);
        let missing = missing_evidence_types(&required, &evidence);
        let satisfied = missing.is_empty()
            && !evidence.is_empty()
            && evidence.iter().all(|e| e["status"] == "passed");
        if !satisfied {
            unresolved.push(json!({"task_id": task.id, "missing_evidence_types": missing}));
        }
        subclaims.push(json!({
            "task_id": task.id,
            "claim": task.title,
            "strategy": "task contract evidence satisfaction",
            "required_evidence_types": required,
            "evidence_links": evidence.iter().map(|e| e["id"].clone()).collect::<Vec<_>>(),
            "status": if satisfied { "satisfied" } else { "unsatisfied" }
        }));
    }
    let status = if unresolved.is_empty() && !subclaims.is_empty() {
        "satisfied"
    } else {
        "unsatisfied"
    };
    let id = Uuid::new_v4().to_string();
    let argument = json!({
        "id": id,
        "plan_version_id": plan_version_id,
        "task_id": task_id,
        "claim": "HOM Local task completion is supported by locked plan, preflight decision, runtime truth, and proof evidence.",
        "strategy": "Goal Structuring Notation: plan claim -> task subclaim -> evidence artifact",
        "subclaims": subclaims,
        "open_assumptions": [],
        "unresolved_risks": unresolved,
        "status": status
    });
    tx.execute(
        "INSERT INTO gate_safety_arguments
         (id, plan_version_id, task_id, status, argument_json, created_at_s, updated_at_s)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![
            id,
            plan_version_id,
            task_id,
            status,
            canonical(&argument)?,
            now
        ],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_argument_insert: {e}")))?;
    let ledger = append_gate_ledger(
        tx,
        "gate.argument.built",
        "brain.gates",
        Some(&id),
        json!({"argument_id": id, "plan_version_id": plan_version_id, "task_id": task_id, "status": status}),
        now,
    )?;
    tx.execute(
        "UPDATE gate_safety_arguments SET ledger_event_id = ?1 WHERE id = ?2",
        params![ledger["event_id"].as_str(), id],
    )
    .map_err(|e| rpc_err(-32603, format!("gate_argument_ledger_update: {e}")))?;
    let mut out = argument;
    out["ledger"] = ledger;
    Ok(out)
}

fn plan_version_from_params(
    tx: &Transaction<'_>,
    params: &Value,
) -> Result<PlanVersionRow, RpcError> {
    if let Some(id) = string_field(params, &["plan_version_id", "planVersionId"]) {
        load_plan_version(tx, &id)?.ok_or_else(|| rpc_err(-32602, "plan_version_not_found"))
    } else {
        current_plan_version_tx(tx)?
            .ok_or_else(|| rpc_err(-32602, "approved_plan_version_required"))
    }
}

fn current_plan_version(conn: &rusqlite::Connection) -> Result<Option<PlanVersionRow>, RpcError> {
    conn.query_row(
        "SELECT id, plan_id, version, payload_json FROM gate_plan_versions
         WHERE status = 'approved_locked' ORDER BY approved_at_s DESC, version DESC LIMIT 1",
        [],
        plan_version_from_row,
    )
    .optional()
    .map_err(|e| rpc_err(-32603, format!("gate_current_plan_query: {e}")))
}

fn current_plan_version_tx(tx: &Transaction<'_>) -> Result<Option<PlanVersionRow>, RpcError> {
    tx.query_row(
        "SELECT id, plan_id, version, payload_json FROM gate_plan_versions
         WHERE status = 'approved_locked' ORDER BY approved_at_s DESC, version DESC LIMIT 1",
        [],
        plan_version_from_row,
    )
    .optional()
    .map_err(|e| rpc_err(-32603, format!("gate_current_plan_query: {e}")))
}

fn load_plan_version(tx: &Transaction<'_>, id: &str) -> Result<Option<PlanVersionRow>, RpcError> {
    tx.query_row(
        "SELECT id, plan_id, version, payload_json FROM gate_plan_versions WHERE id = ?1 AND status = 'approved_locked'",
        params![id],
        plan_version_from_row,
    )
    .optional()
    .map_err(|e| rpc_err(-32603, format!("gate_plan_version_query: {e}")))
}

fn plan_version_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlanVersionRow> {
    let payload_raw: String = row.get(3)?;
    Ok(PlanVersionRow {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        version: row.get(2)?,
        payload: serde_json::from_str(&payload_raw).unwrap_or_else(|_| json!({})),
    })
}

fn load_task(tx: &Transaction<'_>, id: &str) -> Result<Option<TaskRow>, RpcError> {
    tx.query_row(
        "SELECT id, plan_version_id, title, target_method, status, payload_json FROM gate_task_contracts WHERE id = ?1",
        params![id],
        task_from_row,
    )
    .optional()
    .map_err(|e| rpc_err(-32603, format!("gate_task_query: {e}")))
}

fn load_tasks_for_argument(
    tx: &Transaction<'_>,
    plan_version_id: &str,
    task_id: Option<&str>,
) -> Result<Vec<TaskRow>, RpcError> {
    if let Some(task_id) = task_id {
        Ok(load_task(tx, task_id)?.into_iter().collect())
    } else {
        let mut stmt = tx
            .prepare(
                "SELECT id, plan_version_id, title, target_method, status, payload_json
                 FROM gate_task_contracts WHERE plan_version_id = ?1 ORDER BY created_at_s ASC",
            )
            .map_err(|e| rpc_err(-32603, format!("gate_tasks_argument_prepare: {e}")))?;
        stmt.query_map(params![plan_version_id], task_from_row)
            .map_err(|e| rpc_err(-32603, format!("gate_tasks_argument_query: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| rpc_err(-32603, format!("gate_tasks_argument_collect: {e}")))
    }
}

fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRow> {
    let payload_raw: String = row.get(5)?;
    Ok(TaskRow {
        id: row.get(0)?,
        plan_version_id: row.get(1)?,
        title: row.get(2)?,
        target_method: row.get(3)?,
        status: row.get(4)?,
        payload: serde_json::from_str(&payload_raw).unwrap_or_else(|_| json!({})),
    })
}

fn matching_evidence(
    tx: &Transaction<'_>,
    task: &TaskRow,
    evidence_ids: &[String],
) -> Result<Vec<Value>, RpcError> {
    if evidence_ids.is_empty() {
        let mut stmt = tx
            .prepare(
                "SELECT id, plan_version_id, task_id, source_type, source, artifact_hash, custody_json, payload_json, status, created_at_s, ledger_event_id
                 FROM gate_evidence_records WHERE task_id = ?1 AND plan_version_id = ?2 ORDER BY created_at_s ASC",
            )
            .map_err(|e| rpc_err(-32603, format!("gate_evidence_match_prepare: {e}")))?;
        return stmt
            .query_map(params![task.id, task.plan_version_id], evidence_from_row)
            .map_err(|e| rpc_err(-32603, format!("gate_evidence_match_query: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| rpc_err(-32603, format!("gate_evidence_match_collect: {e}")));
    }
    let mut out = Vec::new();
    for evidence_id in evidence_ids {
        let item = tx
            .query_row(
                "SELECT id, plan_version_id, task_id, source_type, source, artifact_hash, custody_json, payload_json, status, created_at_s, ledger_event_id
                 FROM gate_evidence_records WHERE id = ?1 AND task_id = ?2 AND plan_version_id = ?3",
                params![evidence_id, task.id, task.plan_version_id],
                evidence_from_row,
            )
            .optional()
            .map_err(|e| rpc_err(-32603, format!("gate_evidence_id_query: {e}")))?
            .ok_or_else(|| rpc_err(-32602, "evidence_not_bound_to_task"))?;
        out.push(item);
    }
    Ok(out)
}

fn missing_evidence_types(required: &[String], evidence: &[Value]) -> Vec<String> {
    let present: BTreeSet<String> = evidence
        .iter()
        .filter_map(|e| {
            e.get("source_type")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect();
    required
        .iter()
        .filter(|source_type| !present.contains(*source_type))
        .cloned()
        .collect()
}

fn evidence_required_for_task(task: &TaskRow) -> Vec<String> {
    let required = value_string_vec(&task.payload["required_evidence_types"]);
    if required.is_empty() {
        vec!["command_result".to_string()]
    } else {
        required
    }
}

fn evidence_success(source_type: &str, payload: &Value) -> bool {
    match source_type {
        "command_result" => payload.get("exit_code").and_then(Value::as_i64) == Some(0),
        "manual_user_approval" => payload.get("approved").and_then(Value::as_bool) == Some(true),
        _ => {
            payload.get("ok").and_then(Value::as_bool) == Some(true)
                || payload.get("passed").and_then(Value::as_bool) == Some(true)
                || payload
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s == "ok" || s == "passed")
        }
    }
}

fn allowed_evidence_type(source_type: &str) -> bool {
    matches!(
        source_type,
        "command_result"
            | "api_response"
            | "db_query"
            | "ui_snapshot"
            | "runtime_probe"
            | "diagnostic_report"
            | "manual_user_approval"
            | "recall_result"
            | "ledger_event"
            | "artifact_hash"
    )
}

fn approved_amendment_covers(
    tx: &Transaction<'_>,
    task_id: &str,
    requested_scopes: &[String],
) -> Result<bool, RpcError> {
    let mut stmt = tx
        .prepare(
            "SELECT requested_scope_json FROM gate_amendments WHERE task_id = ?1 AND status = 'approved'",
        )
        .map_err(|e| rpc_err(-32603, format!("gate_amendment_scope_prepare: {e}")))?;
    let rows = stmt
        .query_map(params![task_id], |row| row.get::<_, String>(0))
        .map_err(|e| rpc_err(-32603, format!("gate_amendment_scope_query: {e}")))?;
    let requested: BTreeSet<_> = requested_scopes.iter().cloned().collect();
    for raw in rows.filter_map(Result::ok) {
        let value: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!([]));
        let approved = value_string_set(&value);
        if requested.is_subset(&approved) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn scopes_for_task(task: &Value, target_method: &str) -> Vec<String> {
    let mut scopes = string_array_field(
        task,
        &["allowed_scopes", "allowedScopes", "scopes", "scope"],
    );
    if scopes.is_empty() {
        scopes.extend(
            scope_for_target(target_method)
                .into_iter()
                .map(ToString::to_string),
        );
    }
    scopes.sort();
    scopes.dedup();
    scopes
}

fn requested_scopes_for(params: &Value, target_method: &str) -> Vec<String> {
    let mut scopes = string_array_field(
        params,
        &["scope", "scopes", "requested_scopes", "requestedScopes"],
    );
    if scopes.is_empty() {
        scopes.extend(
            scope_for_target(target_method)
                .into_iter()
                .map(ToString::to_string),
        );
    }
    scopes.sort();
    scopes.dedup();
    scopes
}

fn scope_for_target(target_method: &str) -> Vec<&'static str> {
    match target_method {
        "memory.save" => vec!["memory:save"],
        "memory.open" => vec!["memory:open"],
        "memory.answer" => vec!["memory:answer"],
        "memory.recall" | "memory.recall.smart" => vec!["memory:recall"],
        "settings.set" => vec!["system"],
        "nightly.dry_run" | "nightly.run" => vec!["system"],
        "reasoning.run" => vec!["memory:save"],
        method if method.starts_with("reasoning.") => vec!["diagnostics:read"],
        method
            if method.starts_with("import.")
                || method.starts_with("export.")
                || method.starts_with("brain.") =>
        {
            vec!["system"]
        }
        "ledger.verify"
        | "ledger.repair_segmented"
        | "monitoring.snapshot"
        | "ui.contract.snapshot"
        | "ui.inspector.target" => vec!["system"],
        _ => Vec::new(),
    }
}

fn known_target_method(target_method: &str) -> bool {
    matches!(
        target_method,
        "memory.save"
            | "memory.open"
            | "memory.answer"
            | "memory.recall"
            | "memory.recall.smart"
            | "ledger.verify"
            | "ledger.repair_segmented"
            | "monitoring.snapshot"
            | "ui.contract.snapshot"
            | "ui.inspector.target"
            | "settings.set"
            | "nightly.dry_run"
            | "nightly.run"
            | "reasoning.artifacts"
            | "reasoning.bridge.list"
            | "reasoning.tool_quality"
            | "reasoning.run"
            | "import.pack"
            | "export.brain"
            | "brain.backup"
            | "brain.purge"
            | "gates.tasks.complete"
            | "gates.argument.accept"
    )
}

fn required_evidence_types(task: &Value) -> Vec<String> {
    let mut out = string_array_field(
        task,
        &[
            "required_evidence_types",
            "requiredEvidenceTypes",
            "evidence",
        ],
    );
    if out.is_empty() {
        out.push("command_result".to_string());
    }
    out.sort();
    out.dedup();
    out
}

fn latest_decision(conn: &rusqlite::Connection) -> Result<Value, RpcError> {
    conn.query_row(
        "SELECT id, plan_version_id, task_id, decision_state, policy_id, target_method, reason, next_action, observed_facts_json, payload_json, created_at_s, ledger_event_id
         FROM gate_decisions ORDER BY created_at_s DESC, rowid DESC LIMIT 1",
        [],
        decision_from_row,
    )
    .optional()
    .map(|opt| opt.unwrap_or(Value::Null))
    .map_err(|e| rpc_err(-32603, format!("gate_latest_decision_query: {e}")))
}

fn latest_decision_tx(tx: &Transaction<'_>) -> Result<Option<Value>, RpcError> {
    tx.query_row(
        "SELECT id, plan_version_id, task_id, decision_state, policy_id, target_method, reason, next_action, observed_facts_json, payload_json, created_at_s, ledger_event_id
         FROM gate_decisions ORDER BY created_at_s DESC, rowid DESC LIMIT 1",
        [],
        decision_from_row,
    )
    .optional()
    .map_err(|e| rpc_err(-32603, format!("gate_latest_decision_query: {e}")))
}

fn latest_decision_for_task(
    tx: &Transaction<'_>,
    task_id: &str,
) -> Result<Option<Value>, RpcError> {
    tx.query_row(
        "SELECT id, plan_version_id, task_id, decision_state, policy_id, target_method, reason, next_action, observed_facts_json, payload_json, created_at_s, ledger_event_id
         FROM gate_decisions WHERE task_id = ?1 ORDER BY created_at_s DESC, rowid DESC LIMIT 1",
        params![task_id],
        decision_from_row,
    )
    .optional()
    .map_err(|e| rpc_err(-32603, format!("gate_latest_task_decision_query: {e}")))
}

fn decision_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let observed: String = row.get(8)?;
    let payload: String = row.get(9)?;
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "plan_version_id": row.get::<_, Option<String>>(1)?,
        "task_id": row.get::<_, Option<String>>(2)?,
        "decision_state": row.get::<_, String>(3)?,
        "policy_id": row.get::<_, String>(4)?,
        "target_method": row.get::<_, Option<String>>(5)?,
        "reason": row.get::<_, String>(6)?,
        "next_action": row.get::<_, Option<String>>(7)?,
        "observed_facts": serde_json::from_str::<Value>(&observed).unwrap_or_else(|_| json!({})),
        "payload": serde_json::from_str::<Value>(&payload).unwrap_or_else(|_| json!({})),
        "created_at_s": row.get::<_, i64>(10)?,
        "ledger_event_id": row.get::<_, Option<String>>(11)?
    }))
}

fn evidence_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let custody: String = row.get(6)?;
    let payload: String = row.get(7)?;
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "plan_version_id": row.get::<_, String>(1)?,
        "task_id": row.get::<_, String>(2)?,
        "source_type": row.get::<_, String>(3)?,
        "source": row.get::<_, String>(4)?,
        "artifact_hash": row.get::<_, String>(5)?,
        "custody": serde_json::from_str::<Value>(&custody).unwrap_or_else(|_| json!({})),
        "payload": serde_json::from_str::<Value>(&payload).unwrap_or_else(|_| json!({})),
        "status": row.get::<_, String>(8)?,
        "created_at_s": row.get::<_, i64>(9)?,
        "ledger_event_id": row.get::<_, Option<String>>(10)?
    }))
}

fn safety_argument_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let argument_raw: String = row.get(4)?;
    let mut argument = serde_json::from_str::<Value>(&argument_raw).unwrap_or_else(|_| json!({}));
    argument["id"] = json!(row.get::<_, String>(0)?);
    argument["plan_version_id"] = json!(row.get::<_, String>(1)?);
    argument["task_id"] = match row.get::<_, Option<String>>(2)? {
        Some(value) => json!(value),
        None => Value::Null,
    };
    argument["status"] = json!(row.get::<_, String>(3)?);
    argument["created_at_s"] = json!(row.get::<_, i64>(5)?);
    argument["updated_at_s"] = json!(row.get::<_, i64>(6)?);
    argument["ledger_event_id"] = match row.get::<_, Option<String>>(7)? {
        Some(value) => json!(value),
        None => Value::Null,
    };
    Ok(argument)
}

fn count_table(conn: &rusqlite::Connection, table: &str) -> Result<i64, RpcError> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    conn.query_row(&sql, [], |row| row.get(0))
        .map_err(|e| rpc_err(-32603, format!("gate_count_{table}: {e}")))
}

fn append_gate_ledger(
    tx: &Transaction<'_>,
    event_type: &str,
    actor: &str,
    subject_id: Option<&str>,
    payload: Value,
    now: i64,
) -> Result<Value, RpcError> {
    append_ledger_tx(tx, event_type, actor, subject_id, payload, now)
        .map_err(|e| rpc_err(-32603, format!("gate_ledger_append: {e}")))
}

fn plan_version_json(row: &PlanVersionRow) -> Value {
    json!({
        "id": row.id,
        "plan_id": row.plan_id,
        "version": row.version,
        "status": "approved_locked",
        "payload": row.payload
    })
}

fn task_json(row: &TaskRow) -> Value {
    json!({
        "id": row.id,
        "plan_version_id": row.plan_version_id,
        "title": row.title,
        "target_method": row.target_method,
        "status": row.status,
        "payload": row.payload
    })
}

fn gate_required_error(task_id: &str, target_method: &str, reason: &str) -> RpcError {
    RpcError {
        code: -32080,
        message: "gate_preflight_required".to_string(),
        data: Some(json!({
            "task_id": task_id,
            "target_method": target_method,
            "reason": reason,
            "policy_id": POLICY_SCOPE_MATCH
        })),
    }
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn array_field(value: &Value, keys: &[&str]) -> Value {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_array))
        .cloned()
        .map(Value::Array)
        .unwrap_or_else(|| json!([]))
}

fn string_array_field(value: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .map(value_string_vec)
        .unwrap_or_default()
}

fn value_string_vec(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect(),
        Value::String(scope) => scope
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn value_string_set(value: &Value) -> BTreeSet<String> {
    value_string_vec(value).into_iter().collect()
}

fn canonical(value: &Value) -> Result<String, RpcError> {
    canonical_json(value).map_err(|e| rpc_err(-32603, format!("canonical_json: {e}")))
}
