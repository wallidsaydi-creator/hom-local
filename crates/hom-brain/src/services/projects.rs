use std::time::{SystemTime, UNIX_EPOCH};

use hom_shared::{RpcError, rpc_err};
use serde_json::{Value, json};

pub fn list(state: &crate::BrainState) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT p.id, p.name, p.description, p.created_at_s, \
             (SELECT COUNT(*) FROM memories WHERE session_id IN \
              (SELECT id FROM sessions WHERE project_id = p.id)) as memory_count \
             FROM projects p ORDER BY p.created_at_s DESC",
        )
        .map_err(|e| rpc_err(-32603, &format!("projects_list_prepare: {e}")))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(json!({
                "projectId": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "description": row.get::<_, Option<String>>(2)?,
                "createdAtS": row.get::<_, i64>(3)?,
                "memoryCount": row.get::<_, i64>(4)?,
            }))
        })
        .map_err(|e| rpc_err(-32603, &format!("projects_list_query: {e}")))?;

    let projects: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
    Ok(json!({"ok": true, "projects": projects}))
}

pub fn create(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| rpc_err(-32602, "missing name"))?;
    let description = params.get("description").and_then(|v| v.as_str());

    let id = format!("proj_{}", random_hex(16));
    let now = now_s();

    let conn = state.store.conn()?;
    conn.execute(
        "INSERT INTO projects (id, name, description, created_at_s, updated_at_s) VALUES (?1, ?2, ?3, ?4, ?4)",
        rusqlite::params![id, name, description, now],
    )
    .map_err(|e| rpc_err(-32603, &format!("project_create: {e}")))?;

    let project = json!({
        "projectId": id,
        "name": name,
        "description": description,
        "createdAtS": now,
        "memoryCount": 0,
    });

    Ok(json!({
        "ok": true,
        "projectId": project.get("projectId").cloned().unwrap_or(Value::Null),
        "project_id": project.get("projectId").cloned().unwrap_or(Value::Null),
        "name": project.get("name").cloned().unwrap_or(Value::Null),
        "description": project.get("description").cloned().unwrap_or(Value::Null),
        "createdAtS": project.get("createdAtS").cloned().unwrap_or(Value::Null),
        "memoryCount": project.get("memoryCount").cloned().unwrap_or(Value::Null),
        "project": project
    }))
}

fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn random_hex(len: usize) -> String {
    // Use uuid v4 as portable random source — each UUID provides 16 random bytes
    let mut result = String::new();
    while result.len() < len * 2 {
        result.push_str(&uuid::Uuid::new_v4().simple().to_string());
    }
    result.truncate(len * 2);
    result
}
