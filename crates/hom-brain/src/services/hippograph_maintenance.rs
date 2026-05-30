use rusqlite::{Connection, params};
use serde_json::{Value, json};

pub fn mark_decayed_edges(
    conn: &Connection,
    now_s: i64,
    stale_after_s: i64,
) -> anyhow::Result<Value> {
    let cutoff = now_s.saturating_sub(stale_after_s.max(0));
    let updated = conn.execute(
        "UPDATE memory_relationships
         SET decayed_at_s = ?1
         WHERE decayed_at_s IS NULL
           AND created_at_s < ?2",
        params![now_s, cutoff],
    )?;
    Ok(json!({
        "ok": true,
        "decayed_edge_count": updated,
        "policy": "mark_decayed_without_delete",
        "cutoff_s": cutoff
    }))
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::db::migrations::run_migrations;

    #[test]
    fn hippograph_maintenance_marks_old_edges_without_deleting() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO memory_relationships
             (id, source_entity, target_entity, relationship_type, weight, memory_id, created_at_s)
             VALUES ('r1', 'a', 'b', 'co_occurs', 1.0, NULL, 10)",
            [],
        )
        .unwrap();

        let result = mark_decayed_edges(&conn, 100, 50).unwrap();
        assert_eq!(result["decayed_edge_count"], 1);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_relationships", [], |row| {
                row.get(0)
            })
            .unwrap();
        let decayed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory_relationships WHERE decayed_at_s IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(decayed, 1);
    }
}
