use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use hom_shared::{RpcError, rpc_err};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

use crate::db::storage::append_ledger_tx;

pub fn create(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let title = params
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("New Session");
    if title.chars().count() > 120 {
        return Err(rpc_err(-32602, "session_title_too_long"));
    }
    let project_id = params.get("project_id").and_then(|v| v.as_str());
    let activate = params
        .get("activate")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let session_id = uuid::Uuid::new_v4().to_string();
    let now = now_s();

    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("session_create_begin: {e}")))?;

    if activate {
        tx.execute("UPDATE sessions SET active = 0 WHERE active = 1", [])
            .map_err(|e| rpc_err(-32603, format!("session_create_deactivate: {e}")))?;
    }

    tx.execute(
        "INSERT INTO sessions (id, project_id, title, created_at_s, updated_at_s, memory_count, active)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
        rusqlite::params![session_id, project_id, title, now, now, activate],
    )
    .map_err(|e| rpc_err(-32603, format!("session_create_insert: {e}")))?;

    let ledger = append_ledger_tx(
        &tx,
        "session.created",
        "brain.sessions",
        Some(&session_id),
        json!({
            "title": title,
            "project_id": project_id,
            "activate": activate,
            "command": "sessions.create"
        }),
        now,
    )
    .map_err(|e| rpc_err(-32603, format!("session_create_ledger: {e}")))?;

    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("session_create_commit: {e}")))?;

    Ok(json!({
        "ok": true,
        "sessionId": session_id,
        "title": title,
        "projectId": project_id,
        "createdAtS": now,
        "updatedAtS": now,
        "memoryCount": 0,
        "active": activate,
        "ledger": ledger
    }))
}

pub fn list(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;

    let project_filter = params.get("project_id").and_then(|v| v.as_str());
    let (sql, params_vec): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) =
        if let Some(pid) = project_filter {
            (
                "SELECT s.id, s.project_id, s.title, s.created_at_s, s.memory_count, s.active \
             FROM sessions s WHERE s.project_id = ?1 ORDER BY s.created_at_s DESC",
                vec![Box::new(pid.to_string())],
            )
        } else {
            (
                "SELECT s.id, s.project_id, s.title, s.created_at_s, s.memory_count, s.active \
             FROM sessions s ORDER BY s.created_at_s DESC",
                vec![],
            )
        };

    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| rpc_err(-32603, &format!("sessions_list_prepare: {e}")))?;

    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(params_refs.as_slice(), |row| {
            Ok(json!({
                "sessionId": row.get::<_, String>(0)?,
                "projectId": row.get::<_, Option<String>>(1)?,
                "title": row.get::<_, String>(2)?,
                "createdAtS": row.get::<_, i64>(3)?,
                "memoryCount": row.get::<_, i64>(4)?,
                "active": row.get::<_, bool>(5)?,
            }))
        })
        .map_err(|e| rpc_err(-32603, &format!("sessions_list_query: {e}")))?;

    let sessions: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
    Ok(json!({"ok": true, "sessions": sessions}))
}

pub fn detail(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let session_id = params
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| rpc_err(-32602, "missing id"))?;

    let conn = state.store.conn()?;

    let session: Option<Value> = conn
        .query_row(
            "SELECT s.id, s.project_id, s.title, s.created_at_s, s.memory_count, s.active \
             FROM sessions s WHERE s.id = ?1",
            rusqlite::params![session_id],
            |row| {
                Ok(json!({
                    "sessionId": row.get::<_, String>(0)?,
                    "projectId": row.get::<_, Option<String>>(1)?,
                    "title": row.get::<_, String>(2)?,
                    "createdAtS": row.get::<_, i64>(3)?,
                    "memoryCount": row.get::<_, i64>(4)?,
                    "active": row.get::<_, bool>(5)?,
                }))
            },
        )
        .ok();

    let session = session.ok_or_else(|| rpc_err(-32602, "session_not_found"))?;

    // Fetch memories for this session
    let mut mem_stmt = conn
        .prepare(
            "SELECT id, key, value, memory_type, source, quality_score, created_at_s \
             FROM memories WHERE session_id = ?1 ORDER BY created_at_s DESC LIMIT 100",
        )
        .map_err(|e| rpc_err(-32603, &format!("session_memories_prepare: {e}")))?;

    let memories: Vec<Value> = mem_stmt
        .query_map(rusqlite::params![session_id], |row| {
            Ok(json!({
                "memoryId": row.get::<_, String>(0)?,
                "key": row.get::<_, String>(1)?,
                "value": row.get::<_, String>(2)?,
                "memoryType": row.get::<_, String>(3)?,
                "source": row.get::<_, String>(4)?,
                "score": row.get::<_, f64>(5)?,
                "createdAtS": row.get::<_, i64>(6)?,
            }))
        })
        .map_err(|e| rpc_err(-32603, &format!("session_memories_query: {e}")))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(json!({
        "ok": true,
        "session": session,
        "memories": memories,
    }))
}

pub fn set_active(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let session_id = params
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| rpc_err(-32602, "missing id"))?;

    let active = params
        .get("active")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let conn = state.store.conn()?;

    // Deactivate all other sessions
    conn.execute("UPDATE sessions SET active = 0 WHERE active = 1", [])
        .map_err(|e| rpc_err(-32603, &format!("session_deactivate: {e}")))?;

    // Set the requested session active
    let rows = conn
        .execute(
            "UPDATE sessions SET active = ?1, updated_at_s = ?2 WHERE id = ?3",
            rusqlite::params![active, now_s(), session_id],
        )
        .map_err(|e| rpc_err(-32603, &format!("session_set_active: {e}")))?;

    if rows == 0 {
        return Err(rpc_err(-32602, "session_not_found"));
    }

    Ok(json!({"ok": true, "sessionId": session_id, "active": active}))
}

pub fn rename(state: &crate::BrainState, params: Value) -> Result<Value, RpcError> {
    let session_id = params
        .get("id")
        .or_else(|| params.get("session_id"))
        .or_else(|| params.get("sessionId"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| rpc_err(-32602, "missing id"))?;
    let title = params
        .get("title")
        .or_else(|| params.get("name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| rpc_err(-32602, "session_title_required"))?;
    if title.chars().count() > 120 {
        return Err(rpc_err(-32602, "session_title_too_long"));
    }

    let mut conn = state.store.conn()?;
    let tx = conn
        .transaction()
        .map_err(|e| rpc_err(-32603, format!("session_rename_begin: {e}")))?;
    let old_title: Option<String> = tx
        .query_row(
            "SELECT title FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| rpc_err(-32603, format!("session_rename_lookup: {e}")))?;
    let Some(old_title) = old_title else {
        return Err(rpc_err(-32602, "session_not_found"));
    };
    let now = now_s();
    tx.execute(
        "UPDATE sessions SET title = ?1, updated_at_s = ?2 WHERE id = ?3",
        rusqlite::params![title, now, session_id],
    )
    .map_err(|e| rpc_err(-32603, format!("session_rename_update: {e}")))?;
    let ledger = append_ledger_tx(
        &tx,
        "session.renamed",
        "brain.sessions",
        Some(session_id),
        json!({
            "old_title": old_title,
            "new_title": title,
            "command": "sessions.rename"
        }),
        now,
    )
    .map_err(|e| rpc_err(-32603, format!("session_rename_ledger: {e}")))?;
    tx.commit()
        .map_err(|e| rpc_err(-32603, format!("session_rename_commit: {e}")))?;

    Ok(json!({
        "ok": true,
        "sessionId": session_id,
        "title": title,
        "updatedAtS": now,
        "ledger": ledger
    }))
}

pub fn hierarchy(state: &crate::BrainState, _params: Value) -> Result<Value, RpcError> {
    let conn = state.store.conn()?;
    let now = now_s();
    let mut project_stmt = conn
        .prepare(
            "SELECT p.id, p.name, p.description, p.created_at_s, p.updated_at_s,
                    COUNT(DISTINCT s.id) AS session_count,
                    COUNT(m.id) AS memory_count,
                    MAX(COALESCE(m.created_at_s, s.updated_at_s, s.created_at_s, p.updated_at_s)) AS latest_at_s
             FROM projects p
             LEFT JOIN sessions s ON s.project_id = p.id
             LEFT JOIN memories m ON m.session_id = s.id
             GROUP BY p.id, p.name, p.description, p.created_at_s, p.updated_at_s
             ORDER BY COALESCE(latest_at_s, p.updated_at_s, p.created_at_s) DESC",
        )
        .map_err(|e| rpc_err(-32603, format!("sessions_hierarchy_projects_prepare: {e}")))?;
    let project_rows = project_stmt
        .query_map([], |row| {
            Ok(ProjectHierarchy {
                project_id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at_s: row.get(3)?,
                updated_at_s: row.get(4)?,
                session_count: row.get(5)?,
                memory_count: row.get(6)?,
                latest_at_s: row.get(7)?,
                days: Vec::new(),
            })
        })
        .map_err(|e| rpc_err(-32603, format!("sessions_hierarchy_projects_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("sessions_hierarchy_projects_collect: {e}")))?;
    let mut projects = project_rows;

    let mut session_stmt = conn
        .prepare(
            "SELECT s.id, s.project_id, s.title, s.created_at_s, s.updated_at_s,
                    COUNT(m.id) AS memory_count,
                    s.active,
                    MAX(m.created_at_s) AS latest_memory_at_s
             FROM sessions s
             LEFT JOIN memories m ON m.session_id = s.id
             GROUP BY s.id, s.project_id, s.title, s.created_at_s, s.updated_at_s, s.active
             ORDER BY COALESCE(latest_memory_at_s, s.updated_at_s, s.created_at_s) DESC",
        )
        .map_err(|e| rpc_err(-32603, format!("sessions_hierarchy_sessions_prepare: {e}")))?;
    let session_rows = session_stmt
        .query_map([], |row| {
            Ok(SessionHierarchy {
                session_id: row.get(0)?,
                project_id: row.get(1)?,
                title: row.get(2)?,
                created_at_s: row.get(3)?,
                updated_at_s: row.get(4)?,
                memory_count: row.get(5)?,
                active: row.get::<_, bool>(6)?,
                latest_memory_at_s: row.get(7)?,
            })
        })
        .map_err(|e| rpc_err(-32603, format!("sessions_hierarchy_sessions_query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rpc_err(-32603, format!("sessions_hierarchy_sessions_collect: {e}")))?;

    let mut unassigned_days: Vec<DayHierarchy> = Vec::new();
    for session in session_rows {
        if let Some(project_id) = session.project_id.as_deref() {
            if let Some(project) = projects
                .iter_mut()
                .find(|project| project.project_id == project_id)
            {
                push_session_day(&mut project.days, session);
                continue;
            }
        }
        push_session_day(&mut unassigned_days, session);
    }

    Ok(json!({
        "ok": true,
        "projects": projects.into_iter().map(ProjectHierarchy::to_value).collect::<Vec<_>>(),
        "unassignedSessionsByDay": unassigned_days.into_iter().map(DayHierarchy::to_value).collect::<Vec<_>>(),
        "diagnostic": {
            "state": "available",
            "source": "sqlite.projects_sessions_memories",
            "proof": "projects, sessions, and memory counts are loaded from SQLite; unassigned sessions are not emitted as fake project rows",
            "last_checked_at_s": now
        }
    }))
}

struct ProjectHierarchy {
    project_id: String,
    name: String,
    description: Option<String>,
    created_at_s: i64,
    updated_at_s: i64,
    session_count: i64,
    memory_count: i64,
    latest_at_s: Option<i64>,
    days: Vec<DayHierarchy>,
}

impl ProjectHierarchy {
    fn to_value(self) -> Value {
        json!({
            "projectId": self.project_id,
            "name": self.name,
            "description": self.description,
            "createdAtS": self.created_at_s,
            "updatedAtS": self.updated_at_s,
            "sessionCount": self.session_count,
            "memoryCount": self.memory_count,
            "latestAtS": self.latest_at_s,
            "days": self.days.into_iter().map(DayHierarchy::to_value).collect::<Vec<_>>(),
            "diagnostic": {
                "source": "sqlite.projects_sessions_memories",
                "row_kind": "project"
            }
        })
    }
}

struct DayHierarchy {
    day: String,
    latest_at_s: i64,
    sessions: Vec<Value>,
}

impl DayHierarchy {
    fn to_value(self) -> Value {
        json!({
            "day": self.day,
            "latestAtS": self.latest_at_s,
            "sessions": self.sessions,
            "diagnostic": {
                "source": "sqlite.sessions_memories",
                "row_kind": "day"
            }
        })
    }
}

struct SessionHierarchy {
    session_id: String,
    project_id: Option<String>,
    title: String,
    created_at_s: i64,
    updated_at_s: i64,
    memory_count: i64,
    active: bool,
    latest_memory_at_s: Option<i64>,
}

fn push_session_day(days: &mut Vec<DayHierarchy>, session: SessionHierarchy) {
    let latest_at_s = session
        .latest_memory_at_s
        .unwrap_or(session.updated_at_s)
        .max(session.created_at_s);
    let day = day_key(latest_at_s);
    let session_id = session.session_id;
    let rename_endpoint = format!("/api/ui/sessions/{session_id}/rename");
    let row = json!({
        "sessionId": session_id,
        "projectId": session.project_id,
        "title": session.title,
        "createdAtS": session.created_at_s,
        "updatedAtS": session.updated_at_s,
        "memoryCount": session.memory_count,
        "active": session.active,
        "latestMemoryAtS": session.latest_memory_at_s,
        "latestAtS": latest_at_s,
        "renameCommand": {
            "method": "sessions.rename",
            "endpoint": rename_endpoint,
            "required_scope": "system"
        },
        "inspectorTarget": {
            "kind": "session",
            "id": session_id
        },
        "diagnostic": {
            "source": "sqlite.sessions_memories",
            "row_kind": "session"
        }
    });
    if let Some(day_row) = days.iter_mut().find(|existing| existing.day == day) {
        day_row.latest_at_s = day_row.latest_at_s.max(latest_at_s);
        day_row.sessions.push(row);
    } else {
        days.push(DayHierarchy {
            day,
            latest_at_s,
            sessions: vec![row],
        });
    }
    days.sort_by(|left, right| right.latest_at_s.cmp(&left.latest_at_s));
}

fn day_key(timestamp_s: i64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp_s, 0)
        .map(|value| value.date_naive().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_state() -> crate::BrainState {
        let dir = tempdir().unwrap();
        let store = crate::db::storage::LocalStore::open(dir.path()).unwrap();
        crate::BrainState {
            hom_dir: dir.path().to_path_buf(),
            store,
            started_at: std::time::Instant::now(),
            shutting_down: std::sync::atomic::AtomicBool::new(false),
        }
    }

    #[test]
    fn create_returns_session_id_and_defaults() {
        let state = test_state();
        let result = create(&state, json!({})).unwrap();
        assert_eq!(result["ok"], true);
        assert!(result["sessionId"].as_str().unwrap().len() > 0);
        assert_eq!(result["title"], "New Session");
        assert_eq!(result["active"], true);
        assert_eq!(result["memoryCount"], 0);
    }

    #[test]
    fn create_with_custom_title_and_no_activate() {
        let state = test_state();
        let result = create(
            &state,
            json!({
                "title": "Custom Title",
                "activate": false
            }),
        )
        .unwrap();
        assert_eq!(result["title"], "Custom Title");
        assert_eq!(result["projectId"], Value::Null);
        assert_eq!(result["active"], false);
    }

    #[test]
    fn create_activates_deactivates_previous() {
        let state = test_state();
        let a = create(&state, json!({"title": "A", "activate": true})).unwrap();
        let a_id = a["sessionId"].as_str().unwrap().to_string();
        let b = create(&state, json!({"title": "B", "activate": true})).unwrap();
        let b_id = b["sessionId"].as_str().unwrap().to_string();

        let list = crate::services::sessions::list(&state, json!({})).unwrap();
        let sessions = list["sessions"].as_array().unwrap();
        let a_session = sessions.iter().find(|s| s["sessionId"] == a_id).unwrap();
        let b_session = sessions.iter().find(|s| s["sessionId"] == b_id).unwrap();
        assert_eq!(a_session["active"], false);
        assert_eq!(b_session["active"], true);
    }

    #[test]
    fn create_then_set_active_preserves_identity() {
        let state = test_state();
        let a = create(&state, json!({"title": "A", "activate": true})).unwrap();
        let a_id = a["sessionId"].as_str().unwrap().to_string();
        let b = create(&state, json!({"title": "B", "activate": false})).unwrap();
        let b_id = b["sessionId"].as_str().unwrap().to_string();

        // Activate B
        set_active(&state, json!({"id": b_id, "active": true})).unwrap();

        // Rename B
        rename(&state, json!({"id": b_id, "title": "B Renamed"})).unwrap();

        // Verify B still has same session id
        let detail = crate::services::sessions::detail(&state, json!({"id": b_id})).unwrap();
        assert_eq!(detail["session"]["sessionId"], b_id);
        assert_eq!(detail["session"]["title"], "B Renamed");
    }

    #[test]
    fn create_title_too_long_rejected() {
        let state = test_state();
        let long_title: String = "x".repeat(121);
        let result = create(&state, json!({"title": long_title}));
        assert!(result.is_err());
    }

    #[test]
    fn create_appears_in_list() {
        let state = test_state();
        create(&state, json!({"title": "S1"})).unwrap();
        create(&state, json!({"title": "S2"})).unwrap();

        let list = crate::services::sessions::list(&state, json!({})).unwrap();
        let sessions = list["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 2);
        let titles: Vec<&str> = sessions
            .iter()
            .filter_map(|s| s["title"].as_str())
            .collect();
        assert!(titles.contains(&"S1"));
        assert!(titles.contains(&"S2"));
    }

    #[test]
    fn create_records_ledger_event() {
        let state = test_state();
        let result = create(&state, json!({"title": "Ledger Test"})).unwrap();
        assert!(result["ledger"].is_object());
        assert!(result["ledger"]["event_id"].as_str().unwrap().len() > 0);
    }
}
