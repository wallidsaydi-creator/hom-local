use rusqlite::Connection;

pub const LATEST_SCHEMA_VERSION: i32 = 9;

struct Migration {
    version: i32,
    name: &'static str,
    sql: &'static str,
}

pub fn run_migrations(conn: &Connection) -> anyhow::Result<()> {
    let current = schema_version(conn)?;
    for migration in migrations() {
        if current < migration.version || migration_required(conn, migration.version)? {
            conn.execute_batch(migration.sql)
                .map_err(|error| anyhow::anyhow!("migration {} failed: {error}", migration.name))?;
            if current < migration.version {
                conn.pragma_update(None, "user_version", migration.version)?;
            }
        }
    }
    Ok(())
}

fn migration_required(conn: &Connection, version: i32) -> anyhow::Result<bool> {
    Ok(match version {
        1 => !object_exists(conn, "table", "memories") || !object_exists(conn, "table", "settings"),
        2 => {
            !object_exists(conn, "table", "tool_events")
                || !object_exists(conn, "table", "reasoning_bridge_events")
                || !object_exists(conn, "table", "tool_scoring_snapshots")
        }
        3 => {
            !column_exists(conn, "tool_scoring_snapshots", "importance_score")
                || !column_exists(conn, "tool_scoring_snapshots", "ledger_event_id")
                || !object_exists(conn, "index", "idx_tool_scoring_ledger")
        }
        4 => !object_exists(conn, "table", "agent_bdi_states"),
        5 => {
            !object_exists(conn, "table", "memory_embeddings")
                || !object_exists(conn, "index", "idx_memory_embeddings_model")
        }
        6 => {
            !object_exists(conn, "table", "route_certificates")
                || !object_exists(conn, "index", "idx_route_certificates_method")
                || !object_exists(conn, "index", "idx_route_certificates_hash")
        }
        7 => {
            !object_exists(conn, "table", "tool_execution_events")
                || !object_exists(conn, "index", "idx_tool_execution_call")
                || !object_exists(conn, "index", "idx_tool_execution_thread")
                || !object_exists(conn, "index", "idx_tool_execution_tool")
        }
        8 => {
            !object_exists(conn, "table", "import_batches")
                || !object_exists(conn, "table", "import_candidates")
                || !object_exists(conn, "index", "idx_import_candidates_batch")
                || !object_exists(conn, "index", "idx_import_batches_status")
        }
        9 => {
            !object_exists(conn, "table", "ledger_mutation_events")
                || !object_exists(conn, "index", "idx_mutation_target")
                || !object_exists(conn, "index", "idx_mutation_hash")
        }
        _ => false,
    })
}

pub fn schema_version(conn: &Connection) -> anyhow::Result<i32> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(Into::into)
}

fn object_exists(conn: &Connection, kind: &str, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2",
        [kind, name],
        |_| Ok(()),
    )
    .is_ok()
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    let pragma = format!("PRAGMA table_info({table})");
    let mut statement = match conn.prepare(&pragma) {
        Ok(statement) => statement,
        Err(_) => return false,
    };
    let rows = match statement.query_map([], |row| row.get::<_, String>(1)) {
        Ok(rows) => rows,
        Err(_) => return false,
    };
    rows.flatten().any(|name| name == column)
}

fn migrations() -> [Migration; 9] {
    [
        Migration {
        version: 1,
        name: "cognitive_brain_core",
        sql: "
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                key TEXT NOT NULL UNIQUE,
                value TEXT NOT NULL,
                memory_type TEXT NOT NULL,
                source TEXT NOT NULL,
                session_id TEXT,
                project_id TEXT,
                track TEXT,
                created_at_s INTEGER NOT NULL,
                updated_at_s INTEGER NOT NULL,
                quality_score REAL NOT NULL,
                metadata_json TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_memories_created_at ON memories(created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_memories_session ON memories(session_id);
            CREATE INDEX IF NOT EXISTS idx_memories_project ON memories(project_id, created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_memories_track ON memories(track, created_at_s DESC);

            CREATE TABLE IF NOT EXISTS ledger_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL UNIQUE,
                event_type TEXT NOT NULL,
                actor TEXT NOT NULL,
                subject_id TEXT,
                payload_json TEXT NOT NULL,
                prev_hash TEXT,
                event_hash TEXT NOT NULL,
                created_at_s INTEGER NOT NULL,
                ledger_epoch_id TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_ledger_created_at ON ledger_events(created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_ledger_epoch ON ledger_events(ledger_epoch_id, id);

            CREATE TABLE IF NOT EXISTS memory_entities (
                id TEXT PRIMARY KEY,
                memory_id TEXT NOT NULL REFERENCES memories(id),
                entity TEXT NOT NULL,
                entity_type TEXT NOT NULL DEFAULT 'unknown',
                frequency INTEGER NOT NULL DEFAULT 1,
                created_at_s INTEGER NOT NULL,
                UNIQUE(memory_id, entity)
            );
            CREATE INDEX IF NOT EXISTS idx_memory_entities_memory ON memory_entities(memory_id);
            CREATE INDEX IF NOT EXISTS idx_memory_entities_entity ON memory_entities(entity);

            CREATE TABLE IF NOT EXISTS memory_relationships (
                id TEXT PRIMARY KEY,
                source_entity TEXT NOT NULL,
                target_entity TEXT NOT NULL,
                relationship_type TEXT NOT NULL DEFAULT 'related_to',
                weight REAL NOT NULL DEFAULT 1.0,
                memory_id TEXT REFERENCES memories(id),
                created_at_s INTEGER NOT NULL,
                decayed_at_s INTEGER,
                UNIQUE(source_entity, target_entity, relationship_type, memory_id)
            );
            CREATE INDEX IF NOT EXISTS idx_rel_source ON memory_relationships(source_entity);
            CREATE INDEX IF NOT EXISTS idx_rel_target ON memory_relationships(target_entity);

            CREATE TABLE IF NOT EXISTS retrieval_weights (
                memory_id TEXT NOT NULL REFERENCES memories(id),
                mode TEXT NOT NULL,
                weight REAL NOT NULL DEFAULT 0.85,
                updated_at_s INTEGER NOT NULL,
                PRIMARY KEY (memory_id, mode)
            );

            CREATE TABLE IF NOT EXISTS memory_atoms (
                id TEXT PRIMARY KEY,
                memory_id TEXT NOT NULL REFERENCES memories(id),
                subject TEXT NOT NULL,
                predicate TEXT NOT NULL,
                object TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 0.5,
                atom_type TEXT NOT NULL DEFAULT 'declarative',
                created_at_s INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_atoms_memory ON memory_atoms(memory_id);

            CREATE TABLE IF NOT EXISTS retrieval_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id TEXT NOT NULL,
                mode TEXT NOT NULL,
                outcome TEXT NOT NULL,
                created_at_s INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_re_events_memory ON retrieval_events(memory_id);
            CREATE INDEX IF NOT EXISTS idx_re_events_created ON retrieval_events(created_at_s);

            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT,
                created_at_s INTEGER NOT NULL,
                updated_at_s INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                project_id TEXT,
                title TEXT NOT NULL,
                created_at_s INTEGER NOT NULL,
                updated_at_s INTEGER NOT NULL,
                memory_count INTEGER NOT NULL DEFAULT 0,
                active INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (project_id) REFERENCES projects(id)
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_active ON sessions(active);

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at_s INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS gate_prompt_contracts (
                id TEXT PRIMARY KEY,
                prompt_text TEXT NOT NULL,
                status TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at_s INTEGER NOT NULL,
                updated_at_s INTEGER NOT NULL,
                ledger_event_id TEXT
            );

            CREATE TABLE IF NOT EXISTS gate_plan_contracts (
                id TEXT PRIMARY KEY,
                prompt_id TEXT,
                title TEXT NOT NULL,
                status TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at_s INTEGER NOT NULL,
                updated_at_s INTEGER NOT NULL,
                ledger_event_id TEXT,
                FOREIGN KEY (prompt_id) REFERENCES gate_prompt_contracts(id)
            );

            CREATE TABLE IF NOT EXISTS gate_plan_versions (
                id TEXT PRIMARY KEY,
                plan_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                status TEXT NOT NULL,
                approved_by TEXT NOT NULL,
                approved_at_s INTEGER NOT NULL,
                payload_json TEXT NOT NULL,
                created_at_s INTEGER NOT NULL,
                ledger_event_id TEXT,
                UNIQUE(plan_id, version),
                FOREIGN KEY (plan_id) REFERENCES gate_plan_contracts(id)
            );
            CREATE INDEX IF NOT EXISTS idx_gate_plan_versions_plan ON gate_plan_versions(plan_id, version DESC);

            CREATE TABLE IF NOT EXISTS gate_task_contracts (
                id TEXT PRIMARY KEY,
                plan_version_id TEXT NOT NULL,
                title TEXT NOT NULL,
                target_method TEXT NOT NULL,
                status TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at_s INTEGER NOT NULL,
                updated_at_s INTEGER NOT NULL,
                ledger_event_id TEXT,
                FOREIGN KEY (plan_version_id) REFERENCES gate_plan_versions(id)
            );
            CREATE INDEX IF NOT EXISTS idx_gate_tasks_plan_version ON gate_task_contracts(plan_version_id);

            CREATE TABLE IF NOT EXISTS gate_decisions (
                id TEXT PRIMARY KEY,
                plan_version_id TEXT,
                task_id TEXT,
                decision_state TEXT NOT NULL,
                policy_id TEXT NOT NULL,
                target_method TEXT,
                reason TEXT NOT NULL,
                next_action TEXT,
                observed_facts_json TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at_s INTEGER NOT NULL,
                ledger_event_id TEXT,
                FOREIGN KEY (plan_version_id) REFERENCES gate_plan_versions(id),
                FOREIGN KEY (task_id) REFERENCES gate_task_contracts(id)
            );
            CREATE INDEX IF NOT EXISTS idx_gate_decisions_task ON gate_decisions(task_id, created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_gate_decisions_state ON gate_decisions(decision_state);

            CREATE TABLE IF NOT EXISTS gate_evidence_records (
                id TEXT PRIMARY KEY,
                plan_version_id TEXT NOT NULL,
                task_id TEXT NOT NULL,
                source_type TEXT NOT NULL,
                source TEXT NOT NULL,
                artifact_hash TEXT NOT NULL,
                custody_json TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at_s INTEGER NOT NULL,
                ledger_event_id TEXT,
                UNIQUE(task_id, artifact_hash),
                FOREIGN KEY (plan_version_id) REFERENCES gate_plan_versions(id),
                FOREIGN KEY (task_id) REFERENCES gate_task_contracts(id)
            );
            CREATE INDEX IF NOT EXISTS idx_gate_evidence_task ON gate_evidence_records(task_id, source_type);

            CREATE TABLE IF NOT EXISTS gate_amendments (
                id TEXT PRIMARY KEY,
                plan_version_id TEXT NOT NULL,
                task_id TEXT,
                status TEXT NOT NULL,
                requested_scope_json TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at_s INTEGER NOT NULL,
                updated_at_s INTEGER NOT NULL,
                ledger_event_id TEXT,
                FOREIGN KEY (plan_version_id) REFERENCES gate_plan_versions(id),
                FOREIGN KEY (task_id) REFERENCES gate_task_contracts(id)
            );
            CREATE INDEX IF NOT EXISTS idx_gate_amendments_task ON gate_amendments(task_id, status);

            CREATE TABLE IF NOT EXISTS gate_safety_arguments (
                id TEXT PRIMARY KEY,
                plan_version_id TEXT NOT NULL,
                task_id TEXT,
                status TEXT NOT NULL,
                argument_json TEXT NOT NULL,
                created_at_s INTEGER NOT NULL,
                updated_at_s INTEGER NOT NULL,
                ledger_event_id TEXT,
                FOREIGN KEY (plan_version_id) REFERENCES gate_plan_versions(id),
                FOREIGN KEY (task_id) REFERENCES gate_task_contracts(id)
            );
            CREATE INDEX IF NOT EXISTS idx_gate_arguments_task ON gate_safety_arguments(task_id, created_at_s DESC);

            CREATE TABLE IF NOT EXISTS reasoning_bridge_events (
                id TEXT PRIMARY KEY,
                bridge_kind TEXT NOT NULL,
                memory_id TEXT,
                source_memory_id TEXT,
                tool_event_id TEXT,
                provider_id TEXT,
                model_id TEXT,
                request_id TEXT,
                route_certificate_id TEXT,
                gate_task_id TEXT,
                gate_evidence_id TEXT,
                error_id TEXT,
                fix_id TEXT,
                compaction_artifact_id TEXT,
                nightly_artifact_id TEXT,
                reasoning_artifact_id TEXT,
                ledger_event_id TEXT,
                relation_json TEXT NOT NULL,
                created_at_s INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_reasoning_bridge_memory ON reasoning_bridge_events(memory_id, created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_reasoning_bridge_source_memory ON reasoning_bridge_events(source_memory_id, created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_reasoning_bridge_tool ON reasoning_bridge_events(tool_event_id, created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_reasoning_bridge_gate ON reasoning_bridge_events(gate_task_id, gate_evidence_id);
            CREATE INDEX IF NOT EXISTS idx_reasoning_bridge_compaction ON reasoning_bridge_events(compaction_artifact_id, created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_reasoning_bridge_nightly ON reasoning_bridge_events(nightly_artifact_id, created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_reasoning_bridge_reasoning ON reasoning_bridge_events(reasoning_artifact_id, created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_reasoning_bridge_created ON reasoning_bridge_events(created_at_s DESC);

            CREATE TABLE IF NOT EXISTS nightly_artifacts (
                id TEXT PRIMARY KEY,
                cycle_id TEXT NOT NULL,
                mode TEXT NOT NULL,
                status TEXT NOT NULL,
                proposal_count INTEGER NOT NULL,
                applied_count INTEGER NOT NULL,
                mutation_count INTEGER NOT NULL DEFAULT 0,
                warning_count INTEGER NOT NULL,
                insight_count INTEGER NOT NULL,
                touched_memory_count INTEGER NOT NULL DEFAULT 0,
                input_signal_hash TEXT NOT NULL,
                output_state_hash TEXT NOT NULL,
                ledger_event_id TEXT,
                proposals_json TEXT NOT NULL,
                insights_json TEXT NOT NULL,
                warnings_json TEXT NOT NULL,
                guardrails_json TEXT NOT NULL,
                created_at_s INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_nightly_artifacts_created ON nightly_artifacts(created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_nightly_artifacts_mode ON nightly_artifacts(mode, created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_nightly_artifacts_status ON nightly_artifacts(status, created_at_s DESC);

            CREATE TABLE IF NOT EXISTS autonomous_mutation_snapshots (
                id TEXT PRIMARY KEY,
                cycle_id TEXT NOT NULL,
                snapshot_path TEXT NOT NULL,
                snapshot_hash TEXT NOT NULL,
                row_count INTEGER NOT NULL,
                created_at_s INTEGER NOT NULL,
                ledger_event_id TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_mutation_snapshots_cycle ON autonomous_mutation_snapshots(cycle_id, created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_mutation_snapshots_created ON autonomous_mutation_snapshots(created_at_s DESC);

            CREATE TABLE IF NOT EXISTS ledger_epochs (
                id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                anchor_hash TEXT,
                repair_certificate_id TEXT,
                created_at_s INTEGER NOT NULL,
                closed_at_s INTEGER,
                diagnostic_json TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ledger_epochs_status ON ledger_epochs(status, created_at_s DESC);

            CREATE TABLE IF NOT EXISTS ledger_repair_certificates (
                id TEXT PRIMARY KEY,
                active_epoch_id TEXT NOT NULL,
                certificate_hash TEXT NOT NULL UNIQUE,
                historical_first_invalid_json TEXT NOT NULL,
                verified_head_hash TEXT,
                observed_head_hash TEXT,
                db_total_events INTEGER NOT NULL,
                reason TEXT NOT NULL,
                actor TEXT NOT NULL,
                ledger_event_id TEXT,
                certificate_json TEXT NOT NULL,
                created_at_s INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ledger_repair_epoch ON ledger_repair_certificates(active_epoch_id);

            CREATE TABLE IF NOT EXISTS monitoring_events (
                id TEXT PRIMARY KEY,
                warning_id TEXT NOT NULL,
                subsystem TEXT NOT NULL,
                state TEXT NOT NULL,
                severity TEXT NOT NULL,
                evidence_hash TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                first_seen_at_s INTEGER NOT NULL,
                last_seen_at_s INTEGER NOT NULL,
                UNIQUE(warning_id, evidence_hash)
            );
            CREATE INDEX IF NOT EXISTS idx_monitoring_events_state ON monitoring_events(state, last_seen_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_monitoring_events_subsystem ON monitoring_events(subsystem, last_seen_at_s DESC);

            CREATE TABLE IF NOT EXISTS grant_ledger_entries (
                id TEXT PRIMARY KEY,
                grant_kind TEXT NOT NULL,
                subject_id TEXT NOT NULL,
                grantee_id TEXT NOT NULL,
                capability_id TEXT,
                scope_json TEXT NOT NULL,
                state TEXT NOT NULL,
                issuer TEXT NOT NULL,
                reason TEXT,
                expires_at_s INTEGER,
                revoked_at_s INTEGER,
                ledger_event_id TEXT,
                created_at_s INTEGER NOT NULL,
                updated_at_s INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_grant_ledger_subject ON grant_ledger_entries(subject_id, grant_kind, state);
            CREATE INDEX IF NOT EXISTS idx_grant_ledger_grantee ON grant_ledger_entries(grantee_id, state);
            CREATE INDEX IF NOT EXISTS idx_grant_ledger_capability ON grant_ledger_entries(capability_id, state);
            CREATE INDEX IF NOT EXISTS idx_grant_ledger_ledger ON grant_ledger_entries(ledger_event_id);
        ",
    },
    Migration {
        version: 2,
        name: "tool_evidence_scoring",
        sql: "
            CREATE TABLE IF NOT EXISTS tool_events (
                id TEXT PRIMARY KEY,
                tool_id TEXT NOT NULL,
                method TEXT NOT NULL,
                backend_callable INTEGER NOT NULL,
                permission_scope TEXT NOT NULL,
                invocation_source TEXT NOT NULL,
                params_hash TEXT NOT NULL,
                outcome TEXT NOT NULL,
                error_code INTEGER,
                latency_ms INTEGER NOT NULL,
                linked_memory_id TEXT,
                provider_id TEXT,
                model_id TEXT,
                route_certificate_id TEXT,
                gate_task_id TEXT,
                provenance_json TEXT NOT NULL,
                created_at_s INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_tool_events_created ON tool_events(created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_tool_events_memory ON tool_events(linked_memory_id);
            CREATE INDEX IF NOT EXISTS idx_tool_events_method ON tool_events(method, created_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_tool_events_provider ON tool_events(provider_id, model_id);
            CREATE INDEX IF NOT EXISTS idx_tool_events_tool ON tool_events(tool_id, created_at_s DESC);

            CREATE TABLE IF NOT EXISTS tool_scoring_snapshots (
                id TEXT PRIMARY KEY,
                tool_id TEXT NOT NULL,
                window_start_s INTEGER,
                window_end_s INTEGER NOT NULL,
                event_count INTEGER NOT NULL,
                success_count INTEGER NOT NULL,
                error_count INTEGER NOT NULL,
                formula_ref TEXT NOT NULL,
                score_json TEXT NOT NULL,
                created_at_s INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_tool_scoring_tool ON tool_scoring_snapshots(tool_id, created_at_s DESC);
        ",
    },
    Migration {
        version: 3,
        name: "tool_scoring_snapshot_expansion",
        sql: "
            ALTER TABLE tool_scoring_snapshots ADD COLUMN importance_score REAL NOT NULL DEFAULT 0.0;
            ALTER TABLE tool_scoring_snapshots ADD COLUMN ledger_event_id TEXT;
            CREATE INDEX IF NOT EXISTS idx_tool_scoring_ledger ON tool_scoring_snapshots(ledger_event_id);
        ",
    },
    Migration {
        version: 4,
        name: "agent_bdi_state",
        sql: "
            CREATE TABLE IF NOT EXISTS agent_bdi_states (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL UNIQUE,
                beliefs_json TEXT NOT NULL,
                desires_json TEXT NOT NULL,
                intentions_json TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                updated_at_s INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_agent_bdi_updated ON agent_bdi_states(updated_at_s DESC);
        ",
    },
    Migration {
        version: 5,
        name: "memory_embedding_substrate",
        sql: "
            CREATE TABLE IF NOT EXISTS memory_embeddings (
                memory_id TEXT NOT NULL REFERENCES memories(id),
                model TEXT NOT NULL,
                dimensions INTEGER NOT NULL,
                vector_json TEXT NOT NULL,
                created_at_s INTEGER NOT NULL,
                updated_at_s INTEGER NOT NULL,
                PRIMARY KEY (memory_id, model)
            );
            CREATE INDEX IF NOT EXISTS idx_memory_embeddings_model ON memory_embeddings(model, dimensions);
        ",
    },
    Migration {
        version: 6,
        name: "route_certificates",
        sql: "
            CREATE TABLE IF NOT EXISTS route_certificates (
                id TEXT PRIMARY KEY,
                method TEXT NOT NULL,
                provider_id TEXT,
                model_id TEXT,
                capability TEXT NOT NULL,
                permission_scope TEXT NOT NULL,
                descriptor_hash TEXT NOT NULL,
                policy_id TEXT NOT NULL,
                request_context_json TEXT NOT NULL,
                certificate_hash TEXT NOT NULL UNIQUE,
                status TEXT NOT NULL,
                issued_at_s INTEGER NOT NULL,
                expires_at_s INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_route_certificates_method ON route_certificates(method, issued_at_s DESC);
            CREATE INDEX IF NOT EXISTS idx_route_certificates_hash ON route_certificates(certificate_hash);
        ",
    },
    Migration {
        version: 7,
        name: "tool_execution_events",
        sql: "
            CREATE TABLE IF NOT EXISTS tool_execution_events (
                id TEXT PRIMARY KEY,
                call_id TEXT NOT NULL UNIQUE,
                thread_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                tool_id TEXT NOT NULL,
                namespace TEXT,
                owner_runtime TEXT NOT NULL,
                descriptor_hash TEXT NOT NULL,
                permission_profile TEXT NOT NULL,
                approval_id TEXT,
                sandbox_receipt TEXT,
                route_certificate_id TEXT,
                status TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                completed_at INTEGER,
                input_json TEXT NOT NULL,
                output_json TEXT,
                error_json TEXT,
                created_at_s INTEGER NOT NULL,
                FOREIGN KEY (route_certificate_id) REFERENCES route_certificates(id)
            );
            CREATE INDEX IF NOT EXISTS idx_tool_execution_call ON tool_execution_events(call_id);
            CREATE INDEX IF NOT EXISTS idx_tool_execution_thread ON tool_execution_events(thread_id, started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_tool_execution_tool ON tool_execution_events(tool_id, started_at DESC);
        ",
    },
    Migration {
        version: 8,
        name: "memory_import_bridge",
        sql: "
            CREATE TABLE IF NOT EXISTS import_batches (
                batch_id TEXT PRIMARY KEY,
                source_type TEXT NOT NULL,
                source_name TEXT NOT NULL,
                source_uri TEXT,
                status TEXT NOT NULL DEFAULT 'created',
                total_raw_items INTEGER NOT NULL DEFAULT 0,
                total_candidates INTEGER NOT NULL DEFAULT 0,
                total_ready INTEGER NOT NULL DEFAULT 0,
                total_needs_review INTEGER NOT NULL DEFAULT 0,
                total_quarantined INTEGER NOT NULL DEFAULT 0,
                total_committed INTEGER NOT NULL DEFAULT 0,
                created_at_s INTEGER NOT NULL,
                updated_at_s INTEGER NOT NULL,
                metadata_json TEXT DEFAULT '{}'
            );
            CREATE INDEX IF NOT EXISTS idx_import_batches_status ON import_batches(status, created_at_s DESC);

            CREATE TABLE IF NOT EXISTS import_candidates (
                candidate_id TEXT PRIMARY KEY,
                batch_id TEXT NOT NULL REFERENCES import_batches(batch_id),
                memory_kind TEXT NOT NULL DEFAULT 'source_note',
                title TEXT,
                body TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 0.5,
                sensitivity_class TEXT NOT NULL DEFAULT 'normal',
                candidate_status TEXT NOT NULL DEFAULT 'ready',
                source_type TEXT NOT NULL,
                source_name TEXT,
                source_uri TEXT,
                source_hash TEXT,
                source_timestamp TEXT,
                provenance_json TEXT DEFAULT '{}',
                tags_json TEXT DEFAULT '[]',
                evidence_json TEXT DEFAULT '[]',
                duplicate_of TEXT,
                contradiction_with TEXT,
                committed_memory_id TEXT,
                created_at_s INTEGER NOT NULL,
                updated_at_s INTEGER NOT NULL,
                metadata_json TEXT DEFAULT '{}'
            );
            CREATE INDEX IF NOT EXISTS idx_import_candidates_batch ON import_candidates(batch_id, candidate_status);
        ",
    },
    Migration {
        version: 9,
        name: "meta_ledger_mutation_tracking",
        sql: "
            CREATE TABLE IF NOT EXISTS ledger_mutation_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                mutation_id TEXT NOT NULL UNIQUE,
                target_ledger_event_id TEXT NOT NULL,
                target_row_id INTEGER NOT NULL,
                mutation_type TEXT NOT NULL,
                actor TEXT NOT NULL,
                cycle_id TEXT,
                allowed_fields_json TEXT NOT NULL,
                payload_delta_json TEXT NOT NULL,
                original_event_hash TEXT NOT NULL,
                post_mutation_payload_hash TEXT NOT NULL,
                rationale TEXT NOT NULL,
                prev_mutation_hash TEXT,
                mutation_hash TEXT NOT NULL,
                created_at_s INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_mutation_target
                ON ledger_mutation_events(target_ledger_event_id);
            CREATE INDEX IF NOT EXISTS idx_mutation_hash
                ON ledger_mutation_events(mutation_hash);
        ",
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_cognitive_brain_tables_and_indexes() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), LATEST_SCHEMA_VERSION);

        for table in [
            "memories",
            "ledger_events",
            "memory_entities",
            "memory_relationships",
            "retrieval_weights",
            "memory_atoms",
            "retrieval_events",
            "projects",
            "sessions",
            "settings",
            "gate_prompt_contracts",
            "gate_plan_contracts",
            "gate_plan_versions",
            "gate_task_contracts",
            "gate_decisions",
            "gate_evidence_records",
            "gate_amendments",
            "gate_safety_arguments",
            "reasoning_bridge_events",
            "nightly_artifacts",
            "autonomous_mutation_snapshots",
            "ledger_epochs",
            "ledger_repair_certificates",
            "monitoring_events",
            "grant_ledger_entries",
            "tool_events",
            "tool_scoring_snapshots",
            "agent_bdi_states",
            "memory_embeddings",
            "route_certificates",
            "tool_execution_events",
            "import_batches",
            "import_candidates",
            "ledger_mutation_events",
        ] {
            assert!(
                object_exists(&conn, "table", table),
                "missing table {table}"
            );
        }

        for index in [
            "idx_memories_created_at",
            "idx_memories_session",
            "idx_memories_project",
            "idx_memories_track",
            "idx_ledger_created_at",
            "idx_ledger_epoch",
            "idx_memory_entities_memory",
            "idx_memory_entities_entity",
            "idx_rel_source",
            "idx_rel_target",
            "idx_atoms_memory",
            "idx_re_events_memory",
            "idx_re_events_created",
            "idx_gate_plan_versions_plan",
            "idx_gate_tasks_plan_version",
            "idx_gate_decisions_task",
            "idx_gate_decisions_state",
            "idx_gate_evidence_task",
            "idx_gate_amendments_task",
            "idx_gate_arguments_task",
            "idx_reasoning_bridge_memory",
            "idx_reasoning_bridge_source_memory",
            "idx_reasoning_bridge_tool",
            "idx_reasoning_bridge_gate",
            "idx_reasoning_bridge_compaction",
            "idx_reasoning_bridge_nightly",
            "idx_reasoning_bridge_reasoning",
            "idx_reasoning_bridge_created",
            "idx_nightly_artifacts_created",
            "idx_nightly_artifacts_mode",
            "idx_nightly_artifacts_status",
            "idx_mutation_snapshots_cycle",
            "idx_mutation_snapshots_created",
            "idx_ledger_epochs_status",
            "idx_ledger_repair_epoch",
            "idx_monitoring_events_state",
            "idx_monitoring_events_subsystem",
            "idx_grant_ledger_subject",
            "idx_grant_ledger_grantee",
            "idx_grant_ledger_capability",
            "idx_grant_ledger_ledger",
            "idx_tool_events_created",
            "idx_tool_events_memory",
            "idx_tool_events_method",
            "idx_tool_events_provider",
            "idx_tool_events_tool",
            "idx_tool_scoring_tool",
            "idx_agent_bdi_updated",
            "idx_memory_embeddings_model",
            "idx_route_certificates_method",
            "idx_route_certificates_hash",
            "idx_tool_execution_call",
            "idx_tool_execution_thread",
            "idx_tool_execution_tool",
            "idx_import_batches_status",
            "idx_import_candidates_batch",
            "idx_mutation_target",
            "idx_mutation_hash",
        ] {
            assert!(
                object_exists(&conn, "index", index),
                "missing index {index}"
            );
        }
    }

    #[test]
    fn migrations_are_idempotent_and_preserve_data() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO memories
             (id, key, value, memory_type, source, session_id, project_id, track,
              created_at_s, updated_at_s, quality_score, metadata_json)
             VALUES ('m1', 'k1', 'value', 'declarative', 'test', NULL, NULL, NULL, 1, 1, 0.9, '{}')",
            [],
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories WHERE id = 'm1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(schema_version(&conn).unwrap(), LATEST_SCHEMA_VERSION);
    }

    #[test]
    fn migrations_repair_shared_db_when_user_version_is_ahead_but_columns_are_missing() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 31).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE tool_scoring_snapshots (
                id TEXT PRIMARY KEY,
                tool_id TEXT NOT NULL,
                window_start_s INTEGER,
                window_end_s INTEGER NOT NULL,
                event_count INTEGER NOT NULL,
                success_count INTEGER NOT NULL,
                error_count INTEGER NOT NULL,
                formula_ref TEXT NOT NULL,
                score_json TEXT NOT NULL,
                created_at_s INTEGER NOT NULL
            );
            CREATE INDEX idx_tool_scoring_tool ON tool_scoring_snapshots(tool_id, created_at_s DESC);
            ",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        assert!(column_exists(
            &conn,
            "tool_scoring_snapshots",
            "importance_score"
        ));
        assert!(column_exists(
            &conn,
            "tool_scoring_snapshots",
            "ledger_event_id"
        ));
        assert!(object_exists(&conn, "index", "idx_tool_scoring_ledger"));
        assert_eq!(schema_version(&conn).unwrap(), 31);
    }

    fn object_exists(conn: &Connection, kind: &str, name: &str) -> bool {
        conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2",
            [kind, name],
            |_| Ok(()),
        )
        .is_ok()
    }
}
