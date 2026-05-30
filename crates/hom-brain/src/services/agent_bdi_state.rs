use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentBdiState {
    pub agent_id: String,
    pub beliefs: Vec<String>,
    pub desires: Vec<String>,
    pub intentions: Vec<String>,
    pub updated_at_s: i64,
}

pub fn save_state(conn: &Connection, state: &AgentBdiState) -> anyhow::Result<String> {
    let id = conn
        .query_row(
            "SELECT id FROM agent_bdi_states WHERE agent_id = ?1",
            params![state.agent_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let payload = serde_json::to_string(state)?;
    conn.execute(
        "INSERT INTO agent_bdi_states
         (id, agent_id, beliefs_json, desires_json, intentions_json, payload_json, updated_at_s)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(agent_id) DO UPDATE SET
            beliefs_json = excluded.beliefs_json,
            desires_json = excluded.desires_json,
            intentions_json = excluded.intentions_json,
            payload_json = excluded.payload_json,
            updated_at_s = excluded.updated_at_s",
        params![
            id,
            state.agent_id,
            serde_json::to_string(&state.beliefs)?,
            serde_json::to_string(&state.desires)?,
            serde_json::to_string(&state.intentions)?,
            payload,
            state.updated_at_s
        ],
    )?;
    Ok(id)
}

pub fn load_state(conn: &Connection, agent_id: &str) -> anyhow::Result<Option<AgentBdiState>> {
    conn.query_row(
        "SELECT payload_json FROM agent_bdi_states WHERE agent_id = ?1",
        params![agent_id],
        |row| row.get::<_, String>(0),
    )
    .optional()?
    .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
    .transpose()
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::db::migrations::run_migrations;

    #[test]
    fn agent_bdi_state_persists_and_loads_state() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let state = AgentBdiState {
            agent_id: "agent.codex".to_string(),
            beliefs: vec!["tools require traces".to_string()],
            desires: vec!["verify evidence".to_string()],
            intentions: vec!["propose only".to_string()],
            updated_at_s: 42,
        };
        let id = save_state(&conn, &state).unwrap();
        assert!(!id.is_empty());
        assert_eq!(load_state(&conn, "agent.codex").unwrap(), Some(state));
    }
}
