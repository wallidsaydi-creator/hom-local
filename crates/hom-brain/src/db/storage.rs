use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use hom_shared::{RpcError, canonical_json, rpc_err, sha256_hex};
use rusqlite::{Connection, OptionalExtension, Row, params, types::Type};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::db::migrations::run_migrations;
use crate::services::product_quantization::{ProductQuantizationConfig, ProductQuantizer};
use crate::services::ranking_service::{
    RRF_K, RankedItem, RecallMode, active_modes, rrf_fuse_weighted,
};
use crate::services::{
    entity_extraction, evidence_atom, hippograph_builder, hippograph_retriever, quality_gate,
    recall_exact, recall_identifier, recall_lineage, recall_temporal, recall_text, vector_index,
};

#[derive(Clone)]
pub struct LocalStore {
    db_path: PathBuf,
    conn: Arc<Mutex<Connection>>,
    ledger_cache: Arc<Mutex<Option<(i64, Value)>>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SaveInput {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default, alias = "text", alias = "content")]
    pub value: Option<String>,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub track: Option<String>,
    #[serde(default)]
    pub metadata: Value,
    #[serde(skip_deserializing)]
    pub trusted_generated_artifact: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryRow {
    pub id: String,
    pub key: String,
    pub value: String,
    pub memory_type: String,
    pub source: String,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub track: Option<String>,
    pub created_at_s: i64,
    pub updated_at_s: i64,
    pub quality_score: f64,
    pub metadata: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct RecallHit {
    pub memory_id: String,
    pub key: String,
    pub value: String,
    pub memory_type: String,
    pub source: String,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub track: Option<String>,
    pub created_at_s: i64,
    pub score: f64,
    pub components: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExecutionCertificate {
    pub call_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub tool_id: String,
    pub namespace: Option<String>,
    pub owner_runtime: String,
    pub descriptor_hash: String,
    pub permission_profile: String,
    pub approval_id: Option<String>,
    pub sandbox_receipt: Option<Value>,
    pub route_certificate_id: Option<String>,
    pub status: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub input_summary: Value,
    pub output_summary: Option<Value>,
    pub error: Option<Value>,
}

fn json_value_to_sql(value: &Value) -> anyhow::Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn optional_json_value_to_sql(value: Option<Value>) -> anyhow::Result<Option<String>> {
    value
        .map(|value| serde_json::to_string(&value))
        .transpose()
        .map_err(Into::into)
}

fn json_value_from_sql(raw: String, column: usize) -> rusqlite::Result<Value> {
    serde_json::from_str(&raw)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(err)))
}

fn optional_json_value_from_sql(
    raw: Option<String>,
    column: usize,
) -> rusqlite::Result<Option<Value>> {
    raw.map(|raw| json_value_from_sql(raw, column)).transpose()
}

fn execution_certificate_from_row(row: &Row<'_>) -> rusqlite::Result<ExecutionCertificate> {
    Ok(ExecutionCertificate {
        call_id: row.get(0)?,
        thread_id: row.get(1)?,
        turn_id: row.get(2)?,
        tool_id: row.get(3)?,
        namespace: row.get(4)?,
        owner_runtime: row.get(5)?,
        descriptor_hash: row.get(6)?,
        permission_profile: row.get(7)?,
        approval_id: row.get(8)?,
        sandbox_receipt: optional_json_value_from_sql(row.get(9)?, 9)?,
        route_certificate_id: row.get(10)?,
        status: row.get(11)?,
        started_at: row.get(12)?,
        completed_at: row.get(13)?,
        input_summary: json_value_from_sql(row.get(14)?, 14)?,
        output_summary: optional_json_value_from_sql(row.get(15)?, 15)?,
        error: optional_json_value_from_sql(row.get(16)?, 16)?,
    })
}

impl LocalStore {
    pub fn open(hom_dir: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(hom_dir)?;
        let db_path = std::env::var("HOM_DB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| hom_dir.join("local.db"));
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        run_migrations(&conn)?;
        Ok(Self {
            db_path,
            conn: Arc::new(Mutex::new(conn)),
            ledger_cache: Arc::new(Mutex::new(None)),
        })
    }

    pub fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, RpcError> {
        self.conn
            .lock()
            .map_err(|_| rpc_err(-32603, "db_lock_poisoned"))
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn upsert_embedding(
        &self,
        memory_id: &str,
        model: &str,
        vector: &[f64],
    ) -> anyhow::Result<Value> {
        let memory_id = memory_id.trim();
        let model = model.trim();
        if memory_id.is_empty() {
            anyhow::bail!("embedding_memory_id_empty");
        }
        if model.is_empty() {
            anyhow::bail!("embedding_model_empty");
        }
        if vector.is_empty() {
            anyhow::bail!("embedding_vector_empty");
        }
        let now = unix_now_s();
        let vector_json = serde_json::to_string(vector)?;
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        conn.execute(
            "INSERT INTO memory_embeddings
             (memory_id, model, dimensions, vector_json, created_at_s, updated_at_s)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(memory_id, model) DO UPDATE SET
                dimensions = excluded.dimensions,
                vector_json = excluded.vector_json,
                updated_at_s = excluded.updated_at_s",
            params![memory_id, model, vector.len() as i64, vector_json, now, now],
        )?;
        Ok(json!({
            "ok": true,
            "memory_id": memory_id,
            "model": model,
            "dimensions": vector.len(),
            "formula_ref": "embedding_store_v1",
            "mutation_permitted": false
        }))
    }

    pub fn embedding_corpus_size(&self, model: &str, dimensions: usize) -> anyhow::Result<usize> {
        let model = model.trim();
        if model.is_empty() {
            anyhow::bail!("embedding_model_empty");
        }
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_embeddings WHERE model = ?1 AND dimensions = ?2",
            params![model, dimensions as i64],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as usize)
    }

    pub fn exact_vector_search(
        &self,
        model: &str,
        query: &[f64],
        limit: usize,
    ) -> anyhow::Result<Value> {
        let matches = self.exact_vector_rank(model, query, limit)?;
        Ok(json!({
            "ok": true,
            "search": {
                "mode": "exact_vector",
                "model": model.trim(),
                "dimensions": query.len(),
                "formula_ref": "exact_cosine_vector_search_v1",
                "mutation_permitted": false,
                "approximation": "none"
            },
            "matches": matches
        }))
    }

    pub fn exact_vector_rank(
        &self,
        model: &str,
        query: &[f64],
        limit: usize,
    ) -> anyhow::Result<Vec<Value>> {
        let model = model.trim();
        if model.is_empty() {
            anyhow::bail!("embedding_model_empty");
        }
        if query.is_empty() {
            anyhow::bail!("embedding_query_empty");
        }
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let mut statement = conn.prepare(
            "SELECT memory_id, model, dimensions, vector_json
             FROM memory_embeddings
             WHERE model = ?1 AND dimensions = ?2",
        )?;
        let rows = statement.query_map(params![model, query.len() as i64], |row| {
            let memory_id: String = row.get(0)?;
            let model: String = row.get(1)?;
            let dimensions: i64 = row.get(2)?;
            let vector_json: String = row.get(3)?;
            Ok((memory_id, model, dimensions, vector_json))
        })?;
        let mut index = vector_index::VectorIndex::new(query.len());
        for row in rows {
            let (memory_id, model, dimensions, vector_json) = row?;
            let vector: Vec<f64> = serde_json::from_str(&vector_json)?;
            index
                .upsert(vector_index::EmbeddingRecord {
                    memory_id,
                    vector,
                    model,
                    dimensions: dimensions as usize,
                })
                .map_err(|error| anyhow::anyhow!(error))?;
        }
        drop(statement);
        drop(conn);
        index
            .exact_search(query, limit)
            .map_err(|error| anyhow::anyhow!(error))
    }

    pub fn load_embedding_vectors(
        &self,
        model: &str,
        dimensions: usize,
        limit: usize,
    ) -> anyhow::Result<Vec<(String, Vec<f64>)>> {
        let model = model.trim();
        if model.is_empty() {
            anyhow::bail!("embedding_model_empty");
        }
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let mut statement = conn.prepare(
            "SELECT memory_id, vector_json
             FROM memory_embeddings
             WHERE model = ?1 AND dimensions = ?2
             ORDER BY updated_at_s DESC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![model, dimensions as i64, limit.max(1) as i64],
            |row| {
                let memory_id: String = row.get(0)?;
                let vector_json: String = row.get(1)?;
                Ok((memory_id, vector_json))
            },
        )?;
        let mut vectors = Vec::new();
        for row in rows {
            let (memory_id, vector_json) = row?;
            let vector: Vec<f64> = serde_json::from_str(&vector_json)?;
            vectors.push((memory_id, vector));
        }
        Ok(vectors)
    }

    pub fn save_memory(&self, input: SaveInput) -> anyhow::Result<Value> {
        let value = input.value.unwrap_or_default().trim().to_string();
        let now = unix_now_s();
        let id = Uuid::new_v4().to_string();
        let key = input
            .key
            .filter(|key| !key.trim().is_empty())
            .unwrap_or_else(|| format!("memory:{id}"));
        let memory_type = input
            .memory_type
            .unwrap_or_else(|| "declarative".to_string());
        let source = input.source.unwrap_or_else(|| "hom-local".to_string());
        let metadata = if input.metadata.is_null() {
            json!({})
        } else {
            input.metadata
        };
        let trusted_generated_artifact = input.trusted_generated_artifact
            && matches!(
                memory_type.as_str(),
                "session_compaction" | "session_reasoning"
            );
        let quality = if trusted_generated_artifact {
            quality_gate::assess_generated_artifact(&key, &value, &memory_type)
        } else {
            quality_gate::assess_quality(&key, &value, &memory_type)
        };
        if !quality.pass {
            anyhow::bail!(
                "quality_gate_rejected: {}",
                quality.reason.as_deref().unwrap_or("unknown")
            );
        }
        let quality_score = quality.score;
        let quality_json = serde_json::to_value(&quality)?;
        let metadata_json = canonical_json(&metadata)?;
        let project_id = input.project_id.filter(|p| !p.trim().is_empty());
        // Track inference: explicit > source-based > key-based > default
        let track = input
            .track
            .filter(|t| !t.trim().is_empty())
            .or_else(|| infer_track_from_source(&source).or_else(|| infer_track_from_key(&key)));
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let tx = conn.transaction()?;
        if has_recent_duplicate_prefix_tx(&tx, &value, now)? {
            anyhow::bail!("quality_gate_rejected: Duplicate value saved within last hour");
        }
        tx.execute(
            "INSERT INTO memories
             (id, key, value, memory_type, source, session_id, project_id, track,
              created_at_s, updated_at_s, quality_score, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                key,
                value,
                memory_type,
                source,
                input.session_id,
                project_id,
                track,
                now,
                now,
                quality_score,
                metadata_json
            ],
        )?;
        persist_entity_graph_tx(&tx, &id, &key, &value, now)?;
        persist_atoms_tx(&tx, &id, &key, &value, &memory_type, quality_score, now)?;
        let ledger = append_ledger_tx(
            &tx,
            "memory.saved",
            "brain.save",
            Some(&id),
            json!({
                "key": key,
                "memory_type": memory_type,
                "quality_score": quality_score,
                "quality_gate": quality_json
            }),
            now,
        )?;
        tx.commit()?;

        Ok(json!({
            "ok": true,
            "memory_id": id,
            "key": key,
            "quality_score": quality_score,
            "quality_gate": quality,
            "ledger": ledger
        }))
    }

    pub fn recall(&self, query: &str, limit: usize) -> anyhow::Result<Value> {
        self.recall_with_options(query, limit, None, None, None, None, None)
    }

    pub fn recall_with_options(
        &self,
        query: &str,
        limit: usize,
        current_session_source: Option<&str>,
        current_project_id: Option<&str>,
        embedding_model: Option<&str>,
        query_vector: Option<&[f64]>,
        vector_policy_params: Option<&Value>,
    ) -> anyhow::Result<Value> {
        let query = query.trim();
        if query.is_empty() {
            anyhow::bail!("recall_query_empty");
        }
        let limit = limit.clamp(1, 50);
        let terms = tokenize(query);
        let rows = self.load_recent_memories(500)?;
        let rows_by_id: HashMap<_, _> = rows.iter().map(|row| (row.id.as_str(), row)).collect();

        // Early exit on exact match
        let exact_ranked = recall_exact::rank(&rows, query);
        if let Some(exact_hit) = exact_ranked.iter().find(|item| item.exact_pin) {
            let row = rows_by_id.get(exact_hit.memory_id.as_str());
            if let Some(row) = row {
                let hits = vec![RecallHit {
                    memory_id: row.id.clone(),
                    key: row.key.clone(),
                    value: row.value.clone(),
                    memory_type: row.memory_type.clone(),
                    source: row.source.clone(),
                    session_id: row.session_id.clone(),
                    project_id: row.project_id.clone(),
                    track: row.track.clone(),
                    created_at_s: row.created_at_s,
                    score: 1.0,
                    components: json!({
                        "early_exit": "exact_artifact",
                        "exact_pin": true,
                        "mode_ranks": [{"mode": "exact_artifact", "rank": 1}]
                    }),
                }];
                return Ok(json!({
                    "ok": true,
                    "query": query,
                    "result_count": 1,
                    "memories": hits,
                    "recall_meta": {
                        "planner": "local_rrf_early_exit",
                        "modes": ["exact_artifact"],
                        "early_exit": true,
                        "formula": "Exact match — skipped remaining 4 modes"
                    }
                }));
            }
        }

        let now = unix_now_s();
        let temporal_ranked = recall_temporal::rank(&rows, &terms, now, current_session_source);
        let temporal_day_buckets = recall_temporal::top_day_buckets(&rows, &temporal_ranked, 3);
        let (entity_graph, graph_stats) =
            self.load_entity_graph(recall_lineage::ENTITY_NODE_CAP)?;
        let lineage_ranked = recall_lineage::rank(query, &entity_graph);
        let graph_walk_ranked = hippograph_retriever::rank(query, &entity_graph);

        // FTS5 BM25 for text mode, fallback to in-memory BM25
        let text_ranked = match self.fts5_bm25_query(query, limit * 5) {
            Ok(fts_results) if !fts_results.is_empty() => fts_results
                .iter()
                .filter_map(|(id, score)| {
                    rows_by_id
                        .get(id.as_str())
                        .map(|_| RankedItem::new(id.clone(), *score))
                })
                .collect(),
            _ => recall_text::rank(&rows, &terms),
        };

        let (vector_ranked, vector_rank_mode, vector_meta) = if let (Some(model), Some(vector)) =
            (embedding_model, query_vector)
        {
            let requested_vector_mode = vector_policy_params
                .and_then(|params| {
                    params
                        .get("vector_recall_mode")
                        .or_else(|| params.get("vector_mode"))
                        .and_then(Value::as_str)
                })
                .unwrap_or("auto");
            let auto_hybrid_requested = requested_vector_mode == "auto"
                && vector_policy_params
                    .and_then(|params| params.get("threshold_passed"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                && self.embedding_corpus_size(model, vector.len())?
                    > vector_policy_params
                        .and_then(|params| params.get("exact_max_corpus_size"))
                        .and_then(Value::as_u64)
                        .map(|value| value as usize)
                        .unwrap_or(999)
                && self.embedding_corpus_size(model, vector.len())?
                    >= vector_policy_params
                        .and_then(|params| params.get("hybrid_min_corpus_size"))
                        .and_then(Value::as_u64)
                        .map(|value| value as usize)
                        .unwrap_or(1_000);
            if requested_vector_mode == "hybrid_candidate_exact_rerank" || auto_hybrid_requested {
                self.guarded_hybrid_vector_rank(
                    model,
                    vector,
                    limit * 5,
                    vector_policy_params,
                    &rows_by_id,
                )?
            } else {
                let ranked = self
                    .exact_vector_rank(model, vector, limit * 5)?
                    .into_iter()
                    .filter_map(|match_value| {
                        let memory_id = match_value.get("memory_id")?.as_str()?.to_string();
                        rows_by_id.contains_key(memory_id.as_str()).then(|| {
                            RankedItem::new(
                                memory_id,
                                match_value
                                    .get("score")
                                    .and_then(Value::as_f64)
                                    .unwrap_or(0.0),
                            )
                        })
                    })
                    .collect::<Vec<_>>();
                (
                    ranked,
                    RecallMode::VectorExact,
                    json!({
                        "enabled": true,
                        "selected_mode": "exact",
                        "model": model,
                        "dimensions": vector.len(),
                        "formula_ref": "exact_cosine_vector_search_v1",
                        "approximation": "none",
                        "native_pipeline_activation": false,
                        "live_ranking_replacement": false,
                        "requires_explicit_query_vector": true,
                        "mutation_permitted": false
                    }),
                )
            }
        } else {
            (
                Vec::new(),
                RecallMode::VectorExact,
                json!({
                    "enabled": false,
                    "model": embedding_model.unwrap_or(""),
                    "dimensions": query_vector.map(|v| v.len()).unwrap_or(0),
                    "formula_ref": "exact_cosine_vector_search_v1",
                    "approximation": "none",
                    "requires_explicit_query_vector": true,
                    "mutation_permitted": false
                }),
            )
        };

        let rank_lists = vec![
            (RecallMode::ExactArtifact, exact_ranked),
            (RecallMode::GraphWalk, graph_walk_ranked),
            (
                RecallMode::Identifier,
                recall_identifier::rank(&rows, query),
            ),
            (RecallMode::Lineage, lineage_ranked),
            (RecallMode::Temporal, temporal_ranked),
            (RecallMode::Text, text_ranked),
            (vector_rank_mode, vector_ranked),
        ];
        let modes = active_modes(&rank_lists);
        let retrieval_weights = self.load_retrieval_weights(&rows)?;
        let fused: Vec<_> = rrf_fuse_weighted(&rank_lists, RRF_K, |memory_id, mode| {
            retrieval_weights
                .get(&(memory_id.to_string(), mode.as_str().to_string()))
                .copied()
                .unwrap_or(0.85)
        })
        .into_iter()
        .take(limit)
        .filter_map(|item| {
            let row = rows_by_id.get(item.memory_id.as_str())?;
            // Lineage boost: λ=0.10 for same-project memories
            let lineage_boost = if let (Some(proj), Some(query_proj)) =
                (row.project_id.as_deref(), current_project_id)
            {
                if proj.eq_ignore_ascii_case(query_proj) {
                    0.10
                } else {
                    0.0
                }
            } else {
                0.0
            };
            Some((
                RecallHit {
                    memory_id: row.id.clone(),
                    key: row.key.clone(),
                    value: row.value.clone(),
                    memory_type: row.memory_type.clone(),
                    source: row.source.clone(),
                    session_id: row.session_id.clone(),
                    project_id: row.project_id.clone(),
                    track: row.track.clone(),
                    created_at_s: row.created_at_s,
                    score: item.score + lineage_boost,
                    components: item.components(),
                },
                lineage_boost,
            ))
        })
        .collect();

        let hits: Vec<RecallHit> = fused.iter().map(|(hit, _)| hit.clone()).collect();
        let lineage_boost_count = fused.iter().filter(|(_, boost)| *boost > 0.0).count();

        Ok(json!({
            "ok": true,
            "query": query,
            "result_count": hits.len(),
            "memories": hits,
            "recall_meta": {
                "planner": "local_rrf_early_exit",
                "modes": modes,
                "formula": "Guarded weighted RRF(k=60): sum_r w(d,mode)/(60 + rank_r(d)); default w=0.85, autonomous bounds [0.50,1.00]",
                "exact_pin": "exact id/key matches sort before non-exact candidates",
                "lineage_boost": {
                    "lambda": 0.10,
                    "boosted_count": lineage_boost_count,
                    "rule": "boosted_score = base_score + 0.10 * same_project(memory, query)"
                },
                "weight_guardrails": {
                    "source": "HOM Local G6/G7 and MATH-A4",
                    "default_weight": 0.85,
                    "min_weight": 0.50,
                    "max_weight": 1.00
                },
                "temporal_scope": {
                    "dayBuckets": temporal_day_buckets,
                    "source_boost": "QMD-modulated: fresh=0.05, aging=0.10, stale=0.20, historical=0.30 distance advantage",
                    "freshness": "QMD priority: max(0.50, 0.97^(age_days*(1-0.50*quality)))"
                },
                "lineage": {
                    "algorithm": "HippoRAG/MAGMA bipartite memory-entity PPR",
                    "formula": "p_next = (1-alpha)*seed + alpha*W^T*p",
                    "alpha": recall_lineage::PPR_DAMPING,
                    "max_iterations": recall_lineage::PPR_ITERATIONS,
                    "entity_cap": recall_lineage::ENTITY_NODE_CAP,
                    "entity_count": entity_graph.entity_count()
                },
                "graph_walk": {
                    "algorithm": "HippographRetriever graph_walk mode over memory/entity/relationship graph",
                    "formula": "PPR seed entities -> memory nodes, fused through weighted RRF",
                    "stats": graph_stats.to_json()
                },
                "vector": vector_meta,
                "limit": limit
            }
        }))
    }

    fn guarded_hybrid_vector_rank(
        &self,
        model: &str,
        query_vector: &[f64],
        limit: usize,
        params: Option<&Value>,
        rows_by_id: &HashMap<&str, &MemoryRow>,
    ) -> anyhow::Result<(Vec<RankedItem>, RecallMode, Value)> {
        let params = params.unwrap_or(&Value::Null);
        let dimensions = query_vector.len();
        let corpus_size = self.embedding_corpus_size(model, dimensions)?;
        let hybrid_min_corpus_size = params
            .get("hybrid_min_corpus_size")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(1_000);
        let exact_max_corpus_size = params
            .get("exact_max_corpus_size")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(999);
        let threshold_passed = params
            .get("threshold_passed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let threshold_status = if threshold_passed {
            "passed"
        } else {
            "missing_or_failed"
        };
        let activation_allowed = corpus_size > exact_max_corpus_size
            && corpus_size >= hybrid_min_corpus_size
            && threshold_passed;
        if !activation_allowed {
            let selected_mode = if corpus_size == 0 {
                "disabled_no_embeddings"
            } else if corpus_size <= exact_max_corpus_size {
                "exact"
            } else if !threshold_passed {
                "blocked_threshold_missing_or_failed"
            } else {
                "guided_policy_gates_not_satisfied"
            };
            return Ok((
                Vec::new(),
                RecallMode::VectorHybridExactRerank,
                json!({
                    "enabled": true,
                    "selected_mode": selected_mode,
                    "model": model,
                    "dimensions": dimensions,
                    "formula_ref": "exact_cosine_vector_search_v1",
                    "approximation": "product_quantization_adc_candidate_generation_only",
                    "native_pipeline_activation": false,
                    "live_ranking_replacement": false,
                    "requires_explicit_query_vector": true,
                    "mutation_permitted": false,
                    "policy": {
                        "policy_kind": "vector_recall_policy_v1",
                        "corpus_trigger": {
                            "corpus_size": corpus_size,
                            "exact_max_corpus_size": exact_max_corpus_size,
                            "hybrid_min_corpus_size": hybrid_min_corpus_size
                        },
                        "threshold_gate": {
                            "status": threshold_status,
                            "threshold_passed": threshold_passed
                        },
                        "activation_gate": {
                            "status": "blocked_policy_gates_not_satisfied",
                            "requires_user_permission": false
                        }
                    }
                }),
            ));
        }

        let candidate_pool_size = params
            .get("candidate_pool_size")
            .or_else(|| params.get("vector_candidate_pool_size"))
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(limit.max(50))
            .clamp(1, 1_000);
        let top_k = params
            .get("top_k")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(limit)
            .clamp(1, candidate_pool_size);
        let vectors = self.load_embedding_vectors(model, dimensions, 10_000)?;
        if vectors.len() < 2 {
            anyhow::bail!("hybrid_vector_recall_requires_at_least_two_embeddings");
        }
        let vector_by_id = vectors
            .iter()
            .map(|(memory_id, vector)| (memory_id.clone(), vector.clone()))
            .collect::<HashMap<_, _>>();
        let config = ProductQuantizationConfig {
            dimensions,
            subquantizers: params
                .get("subquantizers")
                .and_then(Value::as_u64)
                .unwrap_or(2) as usize,
            centroids_per_subquantizer: params
                .get("centroids_per_subquantizer")
                .and_then(Value::as_u64)
                .unwrap_or(2) as usize,
            iterations: params
                .get("iterations")
                .and_then(Value::as_u64)
                .unwrap_or(4) as usize,
        };
        let quantizer = ProductQuantizer::fit(&vectors, config)
            .map_err(|error| anyhow::anyhow!("pq_fit_failed: {error}"))?;
        let codes = quantizer
            .encode_all(&vectors)
            .map_err(|error| anyhow::anyhow!("pq_encode_failed: {error}"))?;
        let candidates = quantizer
            .approximate_search(&codes, query_vector, candidate_pool_size)
            .map_err(|error| anyhow::anyhow!("pq_search_failed: {error}"))?;
        let mut reranked = candidates
            .iter()
            .enumerate()
            .filter_map(|(idx, candidate)| {
                let memory_id = candidate.get("memory_id")?.as_str()?.to_string();
                if !rows_by_id.contains_key(memory_id.as_str()) {
                    return None;
                }
                let vector = vector_by_id.get(&memory_id)?;
                let exact_score = vector_index::cosine_similarity(query_vector, vector);
                Some((memory_id, idx + 1, exact_score))
            })
            .collect::<Vec<_>>();
        reranked.sort_by(|left, right| {
            right
                .2
                .partial_cmp(&left.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.0.cmp(&right.0))
        });
        let ranked = reranked
            .iter()
            .take(top_k)
            .map(|(memory_id, _, exact_score)| RankedItem::new(memory_id.clone(), *exact_score))
            .collect::<Vec<_>>();

        Ok((
            ranked,
            RecallMode::VectorHybridExactRerank,
            json!({
                "enabled": true,
                "selected_mode": "hybrid_candidate_exact_rerank",
                "model": model,
                "dimensions": dimensions,
                "formula_ref": "exact_cosine_vector_search_v1",
                "approximation": "product_quantization_adc_candidate_generation_only",
                "candidate_generator": quantizer.formula_ref(),
                "reranker": "exact_cosine_vector_search_v1",
                "final_ranking_source": "exact_rerank_candidate_pool",
                "global_exact_guarantee": false,
                "native_pipeline_activation": true,
                "live_ranking_replacement": true,
                "requires_explicit_query_vector": true,
                "mutation_permitted": true,
                "mutation_scope": "guarded_native_hybrid_recall_profile",
                "audit_contract": {
                    "enabled": true,
                    "guidance_over_enforcement": true,
                    "guardrails_enabled": true,
                    "context_injection_enabled": true,
                    "blocks_hybrid": false,
                    "user_facing_mode": true
                },
                "guardrails": {
                    "requires_audit": true,
                    "requires_policy_gate": true,
                    "prevents_plan_drift": true,
                    "activation_scope": "native_pipeline_with_audit_guidance",
                    "user_surface": "enabled_with_context_guidance"
                },
                "candidate_pool_size": candidates.len(),
                "reranked_count": reranked.len(),
                "top_k": top_k,
                "policy": {
                    "policy_kind": "vector_recall_policy_v1",
                    "corpus_trigger": {
                        "corpus_size": corpus_size,
                        "exact_max_corpus_size": exact_max_corpus_size,
                        "hybrid_min_corpus_size": hybrid_min_corpus_size,
                        "recommended_mode": "hybrid_candidate_exact_rerank"
                    },
                    "threshold_gate": {
                        "status": "passed",
                        "threshold_passed": true,
                        "recall_at_k": params.get("recall_at_k").and_then(Value::as_f64),
                        "candidate_pool_hit_rate": params.get("candidate_pool_hit_rate").and_then(Value::as_f64),
                        "top1_preserved": params.get("top1_preserved").and_then(Value::as_bool)
                    },
                    "activation_gate": {
                        "status": "dynamic_policy_selected",
                        "requires_user_permission": false
                    }
                }
            }),
        ))
    }

    pub fn open_memory(&self, id: &str) -> anyhow::Result<Value> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let row =
            load_memory_by_id(&conn, id)?.ok_or_else(|| anyhow::anyhow!("memory_not_found"))?;
        Ok(json!({"ok": true, "memory": row}))
    }

    pub fn list_events(&self, limit: usize) -> anyhow::Result<Value> {
        let limit = limit.clamp(1, 100);
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT event_id, event_type, actor, subject_id, payload_json, prev_hash, event_hash, created_at_s
             FROM ledger_events ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit as i64], |row| {
                Ok(json!({
                    "event_id": row.get::<_, String>(0)?,
                    "event_type": row.get::<_, String>(1)?,
                    "actor": row.get::<_, String>(2)?,
                    "subject_id": row.get::<_, Option<String>>(3)?,
                    "payload": serde_json::from_str::<Value>(&row.get::<_, String>(4)?).unwrap_or_else(|_| json!({})),
                    "prev_hash": row.get::<_, Option<String>>(5)?,
                    "event_hash": row.get::<_, String>(6)?,
                    "created_at_s": row.get::<_, i64>(7)?
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({"ok": true, "events": rows}))
    }

    pub fn verify_ledger(&self) -> anyhow::Result<Value> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        verify_ledger_conn(&conn)
    }

    pub fn cached_ledger_verification(&self) -> Value {
        let now = unix_now_s();
        let cache = self.ledger_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_at, ref result)) = *cache {
            if now - cached_at < 300 {
                return result.clone();
            }
        }
        drop(cache);
        json!({"valid": false, "pending": true, "cached": false})
    }

    pub fn refresh_ledger_cache(&self) {
        let now = unix_now_s();
        let cache = self.ledger_cache.lock().unwrap_or_else(|e| e.into_inner());
        let expired = match *cache {
            Some((cached_at, _)) => now - cached_at >= 300,
            None => true,
        };
        drop(cache);
        if !expired {
            return;
        }
        match self.verify_ledger() {
            Ok(result) => {
                let mut cache = self.ledger_cache.lock().unwrap_or_else(|e| e.into_inner());
                *cache = Some((now, result));
            }
            Err(_) => {}
        }
    }

    pub fn repair_segmented_ledger(&self, actor: &str, reason: &str) -> anyhow::Result<Value> {
        let before = self.verify_ledger()?;
        let first_invalid = before
            .get("first_invalid")
            .filter(|value| !value.is_null())
            .cloned();
        if first_invalid.is_none() {
            return Ok(json!({
                "ok": true,
                "repaired": false,
                "reason": "ledger_has_no_invalid_historical_segment",
                "verification": before
            }));
        }
        if before
            .get("repair_certificate")
            .filter(|value| !value.is_null())
            .is_some()
        {
            return Ok(json!({
                "ok": true,
                "repaired": false,
                "reason": "segmented_repair_already_present",
                "verification": before
            }));
        }

        let now = unix_now_s();
        let active_epoch_id = Uuid::new_v4().to_string();
        let repair_certificate_id = Uuid::new_v4().to_string();
        let certificate = json!({
            "kind": "ledger_segmented_repair_certificate",
            "policy": "preserve invalid historical rows; start a new current epoch anchored to this certificate",
            "actor": actor,
            "reason": reason,
            "historical_first_invalid": first_invalid,
            "verified_head_hash": before.get("verified_head_hash").cloned().unwrap_or(Value::Null),
            "observed_head_hash": before.get("head_hash").cloned().unwrap_or(Value::Null),
            "db_total_events": before.get("total_events").cloned().unwrap_or_else(|| json!(0)),
            "algorithm": before.get("algorithm").cloned().unwrap_or(Value::Null),
            "created_at_s": now
        });
        let certificate_json = canonical_json(&certificate)?;
        let certificate_hash = sha256_hex(&certificate_json);
        let db_total_events = before
            .get("total_events")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO ledger_epochs
             (id, status, anchor_hash, repair_certificate_id, created_at_s, closed_at_s, diagnostic_json)
             VALUES (?1, 'active', ?2, ?3, ?4, NULL, ?5)",
            params![
                active_epoch_id,
                certificate_hash,
                repair_certificate_id,
                now,
                canonical_json(&json!({
                    "source": "ledger.repair_segmented",
                    "historical_status": "invalid_preserved",
                    "certificate_hash": certificate_hash
                }))?
            ],
        )?;
        tx.execute(
            "INSERT INTO ledger_repair_certificates
             (id, active_epoch_id, certificate_hash, historical_first_invalid_json,
              verified_head_hash, observed_head_hash, db_total_events, reason, actor,
              ledger_event_id, certificate_json, created_at_s)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11)",
            params![
                repair_certificate_id,
                active_epoch_id,
                certificate_hash,
                canonical_json(first_invalid.as_ref().unwrap())?,
                before
                    .get("verified_head_hash")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                before
                    .get("head_hash")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                db_total_events,
                reason,
                actor,
                certificate_json,
                now
            ],
        )?;
        let ledger = append_ledger_tx(
            &tx,
            "ledger.repair_segmented",
            actor,
            Some(&repair_certificate_id),
            json!({
                "repair_certificate_id": repair_certificate_id,
                "active_epoch_id": active_epoch_id,
                "certificate_hash": certificate_hash,
                "historical_first_invalid": first_invalid,
                "policy": "segmented_repair_no_rewrite"
            }),
            now,
        )?;
        let ledger_event_id = ledger.get("event_id").and_then(Value::as_str);
        tx.execute(
            "UPDATE ledger_repair_certificates SET ledger_event_id = ?1 WHERE id = ?2",
            params![ledger_event_id, repair_certificate_id],
        )?;
        tx.commit()?;
        drop(conn);
        let after = self.verify_ledger()?;
        Ok(json!({
            "ok": true,
            "repaired": true,
            "repair_certificate_id": repair_certificate_id,
            "active_epoch_id": active_epoch_id,
            "certificate_hash": certificate_hash,
            "ledger": ledger,
            "verification": after
        }))
    }

    fn load_recent_memories(&self, limit: usize) -> anyhow::Result<Vec<MemoryRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, key, value, memory_type, source, session_id,
                    COALESCE(project_id, NULL), COALESCE(track, NULL),
                    created_at_s, updated_at_s, quality_score, metadata_json
             FROM memories ORDER BY created_at_s DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit as i64], memory_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn fts5_bm25_query(&self, query: &str, limit: usize) -> anyhow::Result<Vec<(String, f64)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let fts_query = tokenize(query)
            .into_iter()
            .filter(|t| t.len() >= 2)
            .collect::<Vec<_>>()
            .join(" OR ");
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 200) as i64;
        let sql = format!(
            "SELECT m.id, bm25(memories_fts) AS score
             FROM memories_fts f
             JOIN memories m ON m.rowid = f.rowid
             WHERE memories_fts MATCH ?
             ORDER BY score
             LIMIT ?1"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                [
                    &fts_query as &dyn rusqlite::types::ToSql,
                    &limit as &dyn rusqlite::types::ToSql,
                ],
                |row| {
                    let id: String = row.get(0)?;
                    let score: f64 = row.get(1)?;
                    Ok((id, -score)) // bm25() returns negative; negate for ranking
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn load_retrieval_weights(
        &self,
        rows: &[MemoryRow],
    ) -> anyhow::Result<HashMap<(String, String), f64>> {
        if rows.is_empty() {
            return Ok(HashMap::new());
        }
        let ids: Vec<_> = rows.iter().map(|row| row.id.clone()).collect();
        let bind_slots: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "SELECT memory_id, mode, weight FROM retrieval_weights WHERE memory_id IN ({})",
            bind_slots.join(",")
        );
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt
            .query_map(params.as_slice(), |row| {
                Ok((
                    (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                    row.get::<_, f64>(2)?.clamp(0.50, 1.00),
                ))
            })?
            .collect::<Result<HashMap<_, _>, _>>()?;
        Ok(rows)
    }

    fn load_entity_graph(
        &self,
        entity_cap: usize,
    ) -> anyhow::Result<(
        recall_lineage::MemoryEntityGraph,
        hippograph_builder::HippographStats,
    )> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        hippograph_builder::build_from_conn(&conn, entity_cap)
    }

    pub fn load_atoms_for_memories(
        &self,
        memory_ids: &[String],
    ) -> std::collections::HashMap<String, Vec<evidence_atom::Atom>> {
        if memory_ids.is_empty() {
            return std::collections::HashMap::new();
        }
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return std::collections::HashMap::new(),
        };
        let bind_slots: Vec<String> = memory_ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "SELECT memory_id, subject, predicate, object, confidence, atom_type
             FROM memory_atoms WHERE memory_id IN ({})
             ORDER BY memory_id, id",
            bind_slots.join(",")
        );
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return std::collections::HashMap::new(),
        };
        let params: Vec<&dyn rusqlite::types::ToSql> = memory_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        let rows: Vec<(String, evidence_atom::Atom)> =
            match stmt.query_map(params.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    evidence_atom::Atom {
                        subject: row.get::<_, String>(1)?,
                        predicate: row.get::<_, String>(2)?,
                        object: row.get::<_, String>(3)?,
                        confidence: row.get::<_, f64>(4)?,
                        atom_type: row.get::<_, String>(5)?,
                    },
                ))
            }) {
                Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                Err(_) => return std::collections::HashMap::new(),
            };
        let mut map: std::collections::HashMap<String, Vec<evidence_atom::Atom>> =
            std::collections::HashMap::new();
        for (memory_id, atom) in rows {
            map.entry(memory_id).or_default().push(atom);
        }
        map
    }

    pub fn record_retrieval_events(&self, hits: &[RecallHit], now_s: i64) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        for hit in hits {
            let modes = hit
                .components
                .get("mode_ranks")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m.get("mode").and_then(|m| m.as_str()))
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            conn.execute(
                "INSERT INTO retrieval_events (memory_id, mode, outcome, created_at_s)
                 VALUES (?1, ?2, 'top_k', ?3)",
                rusqlite::params![hit.memory_id, modes, now_s],
            )?;
        }
        Ok(())
    }

    pub fn record_security_event(
        &self,
        actor: &str,
        method: &str,
        payload: Value,
    ) -> anyhow::Result<Value> {
        self.record_security_event_type("security.policy.rejected", actor, method, payload)
    }

    pub fn record_security_event_type(
        &self,
        event_type: &str,
        actor: &str,
        method: &str,
        payload: Value,
    ) -> anyhow::Result<Value> {
        if !event_type.starts_with("security.") {
            anyhow::bail!("security_event_type_required");
        }
        let now = unix_now_s();
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let tx = conn.transaction()?;
        let ledger = append_ledger_tx(&tx, event_type, actor, Some(method), payload, now)?;
        tx.commit()?;
        Ok(ledger)
    }

    pub fn record_gate_event(
        &self,
        event_type: &str,
        actor: &str,
        subject_id: Option<&str>,
        payload: Value,
    ) -> anyhow::Result<Value> {
        if !event_type.starts_with("gate.") {
            anyhow::bail!("gate_event_type_required");
        }
        let now = unix_now_s();
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let tx = conn.transaction()?;
        let ledger = append_ledger_tx(&tx, event_type, actor, subject_id, payload, now)?;
        tx.commit()?;
        Ok(ledger)
    }

    pub fn record_tool_execution(&self, cert: ExecutionCertificate) -> anyhow::Result<Value> {
        let now = unix_now_s();
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let tx = conn.transaction()?;
        let sandbox_receipt_json = optional_json_value_to_sql(cert.sandbox_receipt.clone())?;
        let input_json = json_value_to_sql(&cert.input_summary)?;
        let output_json = optional_json_value_to_sql(cert.output_summary.clone())?;
        let error_json = optional_json_value_to_sql(cert.error.clone())?;

        // Store the execution certificate in the tool_execution_events table
        tx.execute(
            "INSERT INTO tool_execution_events (
                id, call_id, thread_id, turn_id, tool_id, namespace, owner_runtime,
                descriptor_hash, permission_profile, approval_id, sandbox_receipt,
                route_certificate_id, status, started_at, completed_at,
                input_json, output_json, error_json, created_at_s
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
            )",
            rusqlite::params![
                Uuid::new_v4().to_string(),
                cert.call_id,
                cert.thread_id,
                cert.turn_id,
                cert.tool_id,
                cert.namespace,
                cert.owner_runtime,
                cert.descriptor_hash,
                cert.permission_profile,
                cert.approval_id,
                sandbox_receipt_json,
                cert.route_certificate_id,
                cert.status,
                cert.started_at,
                cert.completed_at,
                input_json,
                output_json,
                error_json,
                now
            ],
        )?;

        tx.commit()?;

        // Return a success response
        Ok(json!({
            "ok": true,
            "message": "Tool execution recorded",
            "certificate_id": Uuid::new_v4().to_string() // We could return the actual ID if needed
        }))
    }

    pub fn get_tool_execution_by_call_id(
        &self,
        call_id: &str,
    ) -> anyhow::Result<Option<ExecutionCertificate>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;

        let mut stmt = conn.prepare(
            "SELECT call_id, thread_id, turn_id, tool_id, namespace, owner_runtime,
                    descriptor_hash, permission_profile, approval_id, sandbox_receipt,
                    route_certificate_id, status, started_at, completed_at,
                    input_json, output_json, error_json
             FROM tool_execution_events
             WHERE call_id = ?1",
        )?;

        let row = stmt
            .query_row(rusqlite::params![call_id], execution_certificate_from_row)
            .optional()?;

        Ok(row)
    }

    pub fn get_tool_executions_by_thread_id(
        &self,
        thread_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<ExecutionCertificate>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;

        let mut stmt = conn.prepare(
            "SELECT call_id, thread_id, turn_id, tool_id, namespace, owner_runtime,
                    descriptor_hash, permission_profile, approval_id, sandbox_receipt,
                    route_certificate_id, status, started_at, completed_at,
                    input_json, output_json, error_json
             FROM tool_execution_events
             WHERE thread_id = ?1
             ORDER BY started_at DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(
            rusqlite::params![thread_id, limit as i64],
            execution_certificate_from_row,
        )?;

        let mut executions = Vec::new();
        for row in rows {
            executions.push(row?);
        }

        Ok(executions)
    }

    pub fn get_tool_executions_by_tool_id(
        &self,
        tool_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<ExecutionCertificate>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;

        let mut stmt = conn.prepare(
            "SELECT call_id, thread_id, turn_id, tool_id, namespace, owner_runtime,
                    descriptor_hash, permission_profile, approval_id, sandbox_receipt,
                    route_certificate_id, status, started_at, completed_at,
                    input_json, output_json, error_json
             FROM tool_execution_events
             WHERE tool_id = ?1
             ORDER BY started_at DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(
            rusqlite::params![tool_id, limit as i64],
            execution_certificate_from_row,
        )?;

        let mut executions = Vec::new();
        for row in rows {
            executions.push(row?);
        }

        Ok(executions)
    }

    pub fn update_tool_execution_status(
        &self,
        call_id: &str,
        status: &str,
        completed_at: Option<i64>,
        output_summary: Option<Value>,
        error: Option<Value>,
    ) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;

        let output_json = optional_json_value_to_sql(output_summary)?;
        let error_json = optional_json_value_to_sql(error)?;

        conn.execute(
            "UPDATE tool_execution_events
             SET status = ?2, completed_at = ?3, output_json = ?4, error_json = ?5
             WHERE call_id = ?1",
            rusqlite::params![call_id, status, completed_at, output_json, error_json],
        )?;

        Ok(())
    }

    pub fn security_canary_timeline(&self, limit: usize) -> anyhow::Result<Value> {
        let limit = limit.clamp(1, 200);
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT event_type, actor, subject_id, payload_json, created_at_s
             FROM ledger_events
             WHERE event_type LIKE 'security.%'
             ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit as i64], |row| {
                let payload_raw: String = row.get(3)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    serde_json::from_str::<Value>(&payload_raw).unwrap_or_else(|_| json!({})),
                    row.get::<_, i64>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut events = Vec::new();
        let mut stage_counts: HashMap<&'static str, usize> = HashMap::from([
            ("EXPOSED", 0),
            ("PERSISTED", 0),
            ("RELAYED", 0),
            ("EXECUTED", 0),
        ]);
        let mut deepest_stage: Option<&'static str> = None;

        for (event_type, actor, method, payload, created_at_s) in rows {
            let Some(stage) = payload
                .get("canary_stage")
                .and_then(Value::as_str)
                .and_then(normalize_canary_stage)
            else {
                continue;
            };
            *stage_counts.entry(stage).or_default() += 1;
            deepest_stage = deeper_stage(deepest_stage, stage);
            let canaries = payload
                .get("canaries_found")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for canary in canaries {
                let Some(token) = canary.as_str() else {
                    continue;
                };
                events.push(json!({
                    "canary": token,
                    "stage": stage,
                    "event_type": event_type,
                    "actor": actor,
                    "method": method,
                    "rule_id": payload.get("rule_id").cloned().unwrap_or(Value::Null),
                    "attack_class": payload.get("attack_class").cloned().unwrap_or(Value::Null),
                    "created_at_s": created_at_s
                }));
            }
        }

        Ok(json!({
            "ok": true,
            "source_anchor": crate::services::security_policy::SOURCE_SABER,
            "aggregate_score": Value::Null,
            "stage_counts": stage_counts,
            "deepest_stage": deepest_stage.unwrap_or("EXPOSED"),
            "events": events
        }))
    }

    pub fn security_audit_log(&self, limit: usize) -> anyhow::Result<Value> {
        let limit = limit.clamp(1, 200);
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db_lock_poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT event_type, actor, subject_id, payload_json, created_at_s
             FROM ledger_events
             WHERE event_type LIKE 'security.%'
             ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit as i64], |row| {
                let payload_raw: String = row.get(3)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    serde_json::from_str::<Value>(&payload_raw).unwrap_or_else(|_| json!({})),
                    row.get::<_, i64>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut attack_class_counts: HashMap<String, usize> = HashMap::new();
        let mut rule_counts: HashMap<String, usize> = HashMap::new();
        let mut events = Vec::new();

        for (event_type, actor, method, payload, created_at_s) in rows {
            if let Some(attack_class) = payload.get("attack_class").and_then(Value::as_str) {
                *attack_class_counts
                    .entry(attack_class.to_string())
                    .or_default() += 1;
            }
            if let Some(rule_id) = payload.get("rule_id").and_then(Value::as_str) {
                *rule_counts.entry(rule_id.to_string()).or_default() += 1;
            }
            events.push(json!({
                "event_type": event_type,
                "actor": actor,
                "method": method,
                "rule_id": payload.get("rule_id").cloned().unwrap_or(Value::Null),
                "attack_class": payload.get("attack_class").cloned().unwrap_or(Value::Null),
                "reason": payload.get("reason").cloned().unwrap_or(Value::Null),
                "created_at_s": created_at_s
            }));
        }

        Ok(json!({
            "ok": true,
            "total_events": events.len(),
            "attack_class_counts": attack_class_counts,
            "rule_counts": rule_counts,
            "events": events
        }))
    }
}

fn normalize_canary_stage(stage: &str) -> Option<&'static str> {
    match stage {
        "EXPOSED" => Some("EXPOSED"),
        "PERSISTED" => Some("PERSISTED"),
        "RELAYED" => Some("RELAYED"),
        "EXECUTED" => Some("EXECUTED"),
        _ => None,
    }
}

fn deeper_stage(current: Option<&'static str>, candidate: &'static str) -> Option<&'static str> {
    match current {
        None => Some(candidate),
        Some(current) if stage_rank(candidate) > stage_rank(current) => Some(candidate),
        Some(current) => Some(current),
    }
}

fn stage_rank(stage: &str) -> usize {
    match stage {
        "EXPOSED" => 0,
        "PERSISTED" => 1,
        "RELAYED" => 2,
        "EXECUTED" => 3,
        _ => 0,
    }
}

fn persist_entity_graph_tx(
    tx: &rusqlite::Transaction<'_>,
    memory_id: &str,
    key: &str,
    value: &str,
    created_at_s: i64,
) -> anyhow::Result<()> {
    let entities = entity_extraction::extract_entities(&format!("{key}\n{value}"));
    if entities.is_empty() {
        return Ok(());
    }

    for entity in &entities {
        tx.execute(
            "INSERT INTO memory_entities
             (id, memory_id, entity, entity_type, frequency, created_at_s)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(memory_id, entity) DO UPDATE SET
                entity_type = excluded.entity_type,
                frequency = excluded.frequency",
            params![
                Uuid::new_v4().to_string(),
                memory_id,
                entity.entity,
                entity.entity_type,
                entity.frequency as i64,
                created_at_s
            ],
        )?;
    }

    for (index, left) in entities.iter().enumerate() {
        for right in entities.iter().skip(index + 1) {
            let (source, target) = if left.entity <= right.entity {
                (&left.entity, &right.entity)
            } else {
                (&right.entity, &left.entity)
            };
            tx.execute(
                "INSERT INTO memory_relationships
                 (id, source_entity, target_entity, relationship_type, weight, memory_id, created_at_s, decayed_at_s)
                 VALUES (?1, ?2, ?3, 'co_occurs', ?4, ?5, ?6, NULL)
                 ON CONFLICT(source_entity, target_entity, relationship_type, memory_id)
                 DO UPDATE SET weight = excluded.weight",
                params![
                    Uuid::new_v4().to_string(),
                    source,
                    target,
                    (left.frequency.min(right.frequency) as f64).max(1.0),
                    memory_id,
                    created_at_s
                ],
            )?;
        }
    }

    Ok(())
}

fn persist_atoms_tx(
    tx: &rusqlite::Transaction<'_>,
    memory_id: &str,
    key: &str,
    value: &str,
    memory_type: &str,
    quality_score: f64,
    created_at_s: i64,
) -> anyhow::Result<()> {
    let atoms = evidence_atom::extract_atoms(key, value, memory_type, quality_score);
    for atom in &atoms {
        let atom_id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO memory_atoms
             (id, memory_id, subject, predicate, object, confidence, atom_type, created_at_s)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                atom_id,
                memory_id,
                atom.subject,
                atom.predicate,
                atom.object,
                atom.confidence,
                atom.atom_type,
                created_at_s
            ],
        )?;
    }
    Ok(())
}

// ── Import Batch & Candidate CRUD ──────────────────────────────────────

#[derive(Clone, Debug, Serialize)]
pub struct ImportBatchRow {
    pub batch_id: String,
    pub source_type: String,
    pub source_name: String,
    pub source_uri: Option<String>,
    pub status: String,
    pub total_raw_items: i64,
    pub total_candidates: i64,
    pub total_ready: i64,
    pub total_needs_review: i64,
    pub total_quarantined: i64,
    pub total_committed: i64,
    pub created_at_s: i64,
    pub updated_at_s: i64,
    pub metadata: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct ImportCandidateRow {
    pub candidate_id: String,
    pub batch_id: String,
    pub memory_kind: String,
    pub title: Option<String>,
    pub body: String,
    pub confidence: f64,
    pub sensitivity_class: String,
    pub candidate_status: String,
    pub source_type: String,
    pub source_name: Option<String>,
    pub source_uri: Option<String>,
    pub source_hash: Option<String>,
    pub source_timestamp: Option<String>,
    pub provenance: Value,
    pub tags: Value,
    pub evidence: Value,
    pub duplicate_of: Option<String>,
    pub contradiction_with: Option<String>,
    pub committed_memory_id: Option<String>,
    pub created_at_s: i64,
    pub updated_at_s: i64,
    pub metadata: Value,
}

impl LocalStore {
    pub fn create_import_batch(
        &self,
        source_type: &str,
        source_name: &str,
        source_uri: Option<&str>,
        metadata: Value,
    ) -> Result<ImportBatchRow, RpcError> {
        let batch_id = format!("imp_batch_{}", Uuid::new_v4().simple());
        let now = unix_now_s();
        let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO import_batches (batch_id, source_type, source_name, source_uri, status, total_raw_items, total_candidates, total_ready, total_needs_review, total_quarantined, total_committed, created_at_s, updated_at_s, metadata_json)
             VALUES (?1, ?2, ?3, ?4, 'created', 0, 0, 0, 0, 0, 0, ?5, ?6, ?7)",
            params![batch_id, source_type, source_name, source_uri, now, now, metadata_json],
        )
        .map_err(|e| rpc_err(-32603, &format!("import_batch_create: {e}")))?;
        Ok(ImportBatchRow {
            batch_id,
            source_type: source_type.to_string(),
            source_name: source_name.to_string(),
            source_uri: source_uri.map(str::to_string),
            status: "created".to_string(),
            total_raw_items: 0,
            total_candidates: 0,
            total_ready: 0,
            total_needs_review: 0,
            total_quarantined: 0,
            total_committed: 0,
            created_at_s: now,
            updated_at_s: now,
            metadata,
        })
    }

    pub fn get_import_batch(&self, batch_id: &str) -> Result<Option<ImportBatchRow>, RpcError> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT batch_id, source_type, source_name, source_uri, status, total_raw_items, total_candidates, total_ready, total_needs_review, total_quarantined, total_committed, created_at_s, updated_at_s, metadata_json FROM import_batches WHERE batch_id = ?1")
            .map_err(|e| rpc_err(-32603, &format!("import_batch_get: {e}")))?;
        let row = stmt
            .query_row(params![batch_id], |row| {
                Ok(ImportBatchRow {
                    batch_id: row.get(0)?,
                    source_type: row.get(1)?,
                    source_name: row.get(2)?,
                    source_uri: row.get(3)?,
                    status: row.get(4)?,
                    total_raw_items: row.get(5)?,
                    total_candidates: row.get(6)?,
                    total_ready: row.get(7)?,
                    total_needs_review: row.get(8)?,
                    total_quarantined: row.get(9)?,
                    total_committed: row.get(10)?,
                    created_at_s: row.get(11)?,
                    updated_at_s: row.get(12)?,
                    metadata: serde_json::from_str(&row.get::<_, String>(13)?)
                        .unwrap_or(Value::Null),
                })
            })
            .optional()
            .map_err(|e| rpc_err(-32603, &format!("import_batch_get: {e}")))?;
        Ok(row)
    }

    pub fn list_import_batches(&self, limit: i64) -> Result<Vec<ImportBatchRow>, RpcError> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT batch_id, source_type, source_name, source_uri, status, total_raw_items, total_candidates, total_ready, total_needs_review, total_quarantined, total_committed, created_at_s, updated_at_s, metadata_json FROM import_batches ORDER BY created_at_s DESC LIMIT ?1")
            .map_err(|e| rpc_err(-32603, &format!("import_batch_list: {e}")))?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(ImportBatchRow {
                    batch_id: row.get(0)?,
                    source_type: row.get(1)?,
                    source_name: row.get(2)?,
                    source_uri: row.get(3)?,
                    status: row.get(4)?,
                    total_raw_items: row.get(5)?,
                    total_candidates: row.get(6)?,
                    total_ready: row.get(7)?,
                    total_needs_review: row.get(8)?,
                    total_quarantined: row.get(9)?,
                    total_committed: row.get(10)?,
                    created_at_s: row.get(11)?,
                    updated_at_s: row.get(12)?,
                    metadata: serde_json::from_str(&row.get::<_, String>(13)?)
                        .unwrap_or(Value::Null),
                })
            })
            .map_err(|e| rpc_err(-32603, &format!("import_batch_list: {e}")))?;
        let result: Vec<ImportBatchRow> = rows.filter_map(Result::ok).collect();
        Ok(result)
    }

    pub fn update_import_batch_status(&self, batch_id: &str, status: &str) -> Result<(), RpcError> {
        let now = unix_now_s();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE import_batches SET status = ?1, updated_at_s = ?2 WHERE batch_id = ?3",
            params![status, now, batch_id],
        )
        .map_err(|e| rpc_err(-32603, &format!("import_batch_update_status: {e}")))?;
        Ok(())
    }

    pub fn update_import_batch_counts(
        &self,
        batch_id: &str,
        total_raw_items: i64,
        total_candidates: i64,
        total_ready: i64,
        total_needs_review: i64,
        total_quarantined: i64,
    ) -> Result<(), RpcError> {
        let now = unix_now_s();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE import_batches SET total_raw_items = ?1, total_candidates = ?2, total_ready = ?3, total_needs_review = ?4, total_quarantined = ?5, updated_at_s = ?6 WHERE batch_id = ?7",
            params![total_raw_items, total_candidates, total_ready, total_needs_review, total_quarantined, now, batch_id],
        )
        .map_err(|e| rpc_err(-32603, &format!("import_batch_update_counts: {e}")))?;
        Ok(())
    }

    pub fn increment_import_batch_committed(
        &self,
        batch_id: &str,
        count: i64,
    ) -> Result<(), RpcError> {
        let now = unix_now_s();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE import_batches SET total_committed = total_committed + ?1, updated_at_s = ?2 WHERE batch_id = ?3",
            params![count, now, batch_id],
        )
        .map_err(|e| rpc_err(-32603, &format!("import_batch_increment_committed: {e}")))?;
        Ok(())
    }

    pub fn create_import_candidate(
        &self,
        batch_id: &str,
        memory_kind: &str,
        title: Option<&str>,
        body: &str,
        confidence: f64,
        sensitivity_class: &str,
        candidate_status: &str,
        source_type: &str,
        source_name: Option<&str>,
        source_uri: Option<&str>,
        source_hash: Option<&str>,
        source_timestamp: Option<&str>,
        provenance: Value,
        tags: Value,
        evidence: Value,
        metadata: Value,
    ) -> Result<ImportCandidateRow, RpcError> {
        let candidate_id = format!("imp_cand_{}", Uuid::new_v4().simple());
        let now = unix_now_s();
        let provenance_json =
            serde_json::to_string(&provenance).unwrap_or_else(|_| "{}".to_string());
        let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".to_string());
        let evidence_json = serde_json::to_string(&evidence).unwrap_or_else(|_| "[]".to_string());
        let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO import_candidates (candidate_id, batch_id, memory_kind, title, body, confidence, sensitivity_class, candidate_status, source_type, source_name, source_uri, source_hash, source_timestamp, provenance_json, tags_json, evidence_json, created_at_s, updated_at_s, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![candidate_id, batch_id, memory_kind, title, body, confidence, sensitivity_class, candidate_status, source_type, source_name, source_uri, source_hash, source_timestamp, provenance_json, tags_json, evidence_json, now, now, metadata_json],
        )
        .map_err(|e| rpc_err(-32603, &format!("import_candidate_create: {e}")))?;
        Ok(ImportCandidateRow {
            candidate_id,
            batch_id: batch_id.to_string(),
            memory_kind: memory_kind.to_string(),
            title: title.map(str::to_string),
            body: body.to_string(),
            confidence,
            sensitivity_class: sensitivity_class.to_string(),
            candidate_status: candidate_status.to_string(),
            source_type: source_type.to_string(),
            source_name: source_name.map(str::to_string),
            source_uri: source_uri.map(str::to_string),
            source_hash: source_hash.map(str::to_string),
            source_timestamp: source_timestamp.map(str::to_string),
            provenance,
            tags,
            evidence,
            duplicate_of: None,
            contradiction_with: None,
            committed_memory_id: None,
            created_at_s: now,
            updated_at_s: now,
            metadata,
        })
    }

    pub fn get_import_candidates_by_batch(
        &self,
        batch_id: &str,
        status_filter: Option<&str>,
        limit: i64,
    ) -> Result<Vec<ImportCandidateRow>, RpcError> {
        let conn = self.conn()?;
        if let Some(status) = status_filter {
            let mut stmt = conn.prepare(
                "SELECT candidate_id, batch_id, memory_kind, title, body, confidence, sensitivity_class, candidate_status, source_type, source_name, source_uri, source_hash, source_timestamp, provenance_json, tags_json, evidence_json, duplicate_of, contradiction_with, committed_memory_id, created_at_s, updated_at_s, metadata_json FROM import_candidates WHERE batch_id = ?1 AND candidate_status = ?2 ORDER BY created_at_s ASC LIMIT ?3"
            ).map_err(|e| rpc_err(-32603, &format!("import_candidate_list: {e}")))?;
            let rows = stmt
                .query_map(params![batch_id, status, limit], |row| {
                    Self::candidate_from_row(row)
                })
                .map_err(|e| rpc_err(-32603, &format!("import_candidate_list: {e}")))?;
            Ok(rows.filter_map(Result::ok).collect())
        } else {
            let mut stmt = conn.prepare(
                "SELECT candidate_id, batch_id, memory_kind, title, body, confidence, sensitivity_class, candidate_status, source_type, source_name, source_uri, source_hash, source_timestamp, provenance_json, tags_json, evidence_json, duplicate_of, contradiction_with, committed_memory_id, created_at_s, updated_at_s, metadata_json FROM import_candidates WHERE batch_id = ?1 ORDER BY created_at_s ASC LIMIT ?2"
            ).map_err(|e| rpc_err(-32603, &format!("import_candidate_list: {e}")))?;
            let rows = stmt
                .query_map(params![batch_id, limit], |row| {
                    Self::candidate_from_row(row)
                })
                .map_err(|e| rpc_err(-32603, &format!("import_candidate_list: {e}")))?;
            Ok(rows.filter_map(Result::ok).collect())
        }
    }

    pub fn get_import_candidate(
        &self,
        candidate_id: &str,
    ) -> Result<Option<ImportCandidateRow>, RpcError> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT candidate_id, batch_id, memory_kind, title, body, confidence, sensitivity_class, candidate_status, source_type, source_name, source_uri, source_hash, source_timestamp, provenance_json, tags_json, evidence_json, duplicate_of, contradiction_with, committed_memory_id, created_at_s, updated_at_s, metadata_json FROM import_candidates WHERE candidate_id = ?1")
            .map_err(|e| rpc_err(-32603, &format!("import_candidate_get: {e}")))?;
        let row = stmt
            .query_row(params![candidate_id], |row| Self::candidate_from_row(row))
            .optional()
            .map_err(|e| rpc_err(-32603, &format!("import_candidate_get: {e}")))?;
        Ok(row)
    }

    pub fn update_import_candidate_status(
        &self,
        candidate_id: &str,
        candidate_status: &str,
        duplicate_of: Option<&str>,
        contradiction_with: Option<&str>,
    ) -> Result<(), RpcError> {
        let now = unix_now_s();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE import_candidates SET candidate_status = ?1, duplicate_of = ?2, contradiction_with = ?3, updated_at_s = ?4 WHERE candidate_id = ?5",
            params![candidate_status, duplicate_of, contradiction_with, now, candidate_id],
        )
        .map_err(|e| rpc_err(-32603, &format!("import_candidate_update_status: {e}")))?;
        Ok(())
    }

    pub fn set_import_candidate_committed(
        &self,
        candidate_id: &str,
        memory_id: &str,
    ) -> Result<(), RpcError> {
        let now = unix_now_s();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE import_candidates SET candidate_status = 'committed', committed_memory_id = ?1, updated_at_s = ?2 WHERE candidate_id = ?3",
            params![memory_id, now, candidate_id],
        )
        .map_err(|e| rpc_err(-32603, &format!("import_candidate_commit: {e}")))?;
        Ok(())
    }

    pub fn count_import_candidates_by_status(
        &self,
        batch_id: &str,
    ) -> Result<HashMap<String, i64>, RpcError> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT candidate_status, COUNT(*) FROM import_candidates WHERE batch_id = ?1 GROUP BY candidate_status")
            .map_err(|e| rpc_err(-32603, &format!("import_candidate_count: {e}")))?;
        let rows = stmt
            .query_map(params![batch_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| rpc_err(-32603, &format!("import_candidate_count: {e}")))?;
        let mut counts = HashMap::new();
        for row in rows {
            if let Ok((status, count)) = row {
                counts.insert(status, count);
            }
        }
        Ok(counts)
    }

    fn candidate_from_row(row: &Row<'_>) -> rusqlite::Result<ImportCandidateRow> {
        Ok(ImportCandidateRow {
            candidate_id: row.get(0)?,
            batch_id: row.get(1)?,
            memory_kind: row.get(2)?,
            title: row.get(3)?,
            body: row.get(4)?,
            confidence: row.get(5)?,
            sensitivity_class: row.get(6)?,
            candidate_status: row.get(7)?,
            source_type: row.get(8)?,
            source_name: row.get(9)?,
            source_uri: row.get(10)?,
            source_hash: row.get(11)?,
            source_timestamp: row.get(12)?,
            provenance: serde_json::from_str(&row.get::<_, String>(13)?).unwrap_or(Value::Null),
            tags: serde_json::from_str(&row.get::<_, String>(14)?).unwrap_or(Value::Null),
            evidence: serde_json::from_str(&row.get::<_, String>(15)?).unwrap_or(Value::Null),
            duplicate_of: row.get(16)?,
            contradiction_with: row.get(17)?,
            committed_memory_id: row.get(18)?,
            created_at_s: row.get(19)?,
            updated_at_s: row.get(20)?,
            metadata: serde_json::from_str(&row.get::<_, String>(21)?).unwrap_or(Value::Null),
        })
    }
}

pub(crate) fn append_ledger_tx(
    tx: &rusqlite::Transaction<'_>,
    event_type: &str,
    actor: &str,
    subject_id: Option<&str>,
    payload: Value,
    created_at_s: i64,
) -> anyhow::Result<Value> {
    let active_epoch: Option<(String, Option<String>)> = tx
        .query_row(
            "SELECT id, anchor_hash FROM ledger_epochs
             WHERE status = 'active'
             ORDER BY created_at_s DESC
             LIMIT 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let (ledger_epoch_id, prev_hash) = if let Some((epoch_id, anchor_hash)) = active_epoch {
        let prev_hash = tx
            .query_row(
                "SELECT event_hash FROM ledger_events
                 WHERE ledger_epoch_id = ?1
                 ORDER BY id DESC
                 LIMIT 1",
                [&epoch_id],
                |row| row.get(0),
            )
            .optional()?
            .or(anchor_hash);
        (Some(epoch_id), prev_hash)
    } else {
        let prev_hash: Option<String> = tx
            .query_row(
                "SELECT event_hash FROM ledger_events ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        (None, prev_hash)
    };
    let event_id = Uuid::new_v4().to_string();
    let payload_json = canonical_json(&payload)?;
    let hash_input = canonical_json(&json!({
        "event_id": event_id,
        "event_type": event_type,
        "actor": actor,
        "subject_id": subject_id,
        "payload": payload,
        "prev_hash": prev_hash,
        "created_at_s": created_at_s
    }))?;
    let event_hash = sha256_hex(&hash_input);
    tx.execute(
        "INSERT INTO ledger_events
         (event_id, event_type, actor, subject_id, payload_json, prev_hash, event_hash, created_at_s, ledger_epoch_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            event_id,
            event_type,
            actor,
            subject_id,
            payload_json,
            prev_hash,
            event_hash,
            created_at_s,
            ledger_epoch_id
        ],
    )?;
    Ok(json!({"event_id": event_id, "event_hash": event_hash, "ledger_epoch_id": ledger_epoch_id}))
}

pub(crate) fn verify_ledger_conn(conn: &Connection) -> anyhow::Result<Value> {
    let rows = load_ledger_rows(conn)?;
    let db_total_events = rows.len() as i64;
    let observed_head_hash = rows.last().map(|row| row.event_hash.clone());
    let active_epoch = active_ledger_epoch(conn)?;
    let mutations = load_mutation_records(conn)?;
    let historical_rows = rows
        .iter()
        .filter(|row| row.ledger_epoch_id.is_none())
        .cloned()
        .collect::<Vec<_>>();
    let historical = verify_ledger_sequence(&historical_rows, None, &mutations)?;
    let repair_certificate = active_epoch
        .as_ref()
        .and_then(|epoch| load_repair_certificate(conn, &epoch.id).transpose())
        .transpose()?;
    let historical_status = if historical.first_invalid.is_none() {
        "valid"
    } else if repair_certificate.is_some() {
        "invalid_preserved"
    } else {
        "invalid_uncertified"
    };
    let current = if let Some(epoch) = &active_epoch {
        let current_rows = rows
            .iter()
            .filter(|row| row.ledger_epoch_id.as_deref() == Some(epoch.id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        Some(verify_ledger_sequence(
            &current_rows,
            epoch.anchor_hash.clone(),
            &mutations,
        )?)
    } else {
        None
    };
    let current_valid = current
        .as_ref()
        .map(|verification| verification.first_invalid.is_none() && verification.total_events > 0)
        .unwrap_or(false);
    let valid = if active_epoch.is_some() {
        current_valid && historical_status != "invalid_uncertified"
    } else {
        historical.first_invalid.is_none()
    };
    let checked_events = historical.checked_events
        + current
            .as_ref()
            .map(|verification| verification.checked_events)
            .unwrap_or(0);
    let first_invalid = current
        .as_ref()
        .and_then(|verification| verification.first_invalid.clone())
        .or_else(|| historical.first_invalid.clone());
    let verified_head_hash = current
        .as_ref()
        .and_then(|verification| verification.verified_head_hash.clone())
        .or_else(|| historical.verified_head_hash.clone());
    let last_verified_event_id = current
        .as_ref()
        .and_then(|verification| verification.last_verified_event_id.clone())
        .or_else(|| historical.last_verified_event_id.clone());
    let mut self_modified_events = historical.self_modified_events.clone();
    if let Some(cur) = &current {
        self_modified_events.extend(cur.self_modified_events.clone());
    }
    let self_modified_count = self_modified_events.len() as i64;
    let pristine = first_invalid.is_none() && self_modified_count == 0;

    Ok(json!({
        "ok": true,
        "valid": valid,
        "operational_valid": valid,
        "pristine": pristine,
        "self_modified_count": self_modified_count,
        "self_modified_events": self_modified_events,
        "all_history_valid": historical.first_invalid.is_none() && current.as_ref().map_or(true, |verification| verification.first_invalid.is_none()),
        "total_events": db_total_events,
        "checked_events": checked_events,
        "head_hash": observed_head_hash,
        "verified_head_hash": verified_head_hash,
        "last_verified_event_id": last_verified_event_id,
        "first_invalid": first_invalid.clone(),
        "historical_segment": {
            "status": historical_status,
            "total_events": historical.total_events,
            "checked_events": historical.checked_events,
            "verified_head_hash": historical.verified_head_hash,
            "last_verified_event_id": historical.last_verified_event_id,
            "first_invalid": historical.first_invalid
        },
        "current_epoch": active_epoch.as_ref().map(|epoch| {
            let verification = current.as_ref().cloned().unwrap_or_default();
            json!({
                "id": epoch.id,
                "status": epoch.status,
                "valid": verification.first_invalid.is_none() && verification.total_events > 0,
                "anchor_hash": epoch.anchor_hash,
                "repair_certificate_id": epoch.repair_certificate_id,
                "total_events": verification.total_events,
                "checked_events": verification.checked_events,
                "verified_head_hash": verification.verified_head_hash,
                "last_verified_event_id": verification.last_verified_event_id,
                "first_invalid": verification.first_invalid
            })
        }).unwrap_or(Value::Null),
        "repair_certificate": repair_certificate.unwrap_or(Value::Null),
        "algorithm": "sha256(canonical_json({event_id,event_type,actor,subject_id,payload,prev_hash,created_at_s}))",
        "checked_at_s": unix_now_s()
    }))
}

#[derive(Clone)]
struct LedgerEventRow {
    row_id: i64,
    event_id: String,
    event_type: String,
    actor: String,
    subject_id: Option<String>,
    payload_json: String,
    prev_hash: Option<String>,
    event_hash: String,
    created_at_s: i64,
    ledger_epoch_id: Option<String>,
}

#[derive(Clone)]
struct LedgerEpochRow {
    id: String,
    status: String,
    anchor_hash: Option<String>,
    repair_certificate_id: Option<String>,
}

#[derive(Clone, Default)]
struct LedgerSequenceVerification {
    total_events: i64,
    checked_events: i64,
    verified_head_hash: Option<String>,
    last_verified_event_id: Option<String>,
    first_invalid: Option<Value>,
    self_modified_events: Vec<Value>,
}

#[derive(Clone)]
struct MutationRecord {
    mutation_id: String,
    target_ledger_event_id: String,
    mutation_type: String,
    original_event_hash: String,
    post_mutation_payload_hash: String,
}

enum LedgerRowStatus {
    Valid,
    SelfModifiedPreserved { detail: Value },
    Invalid { reason: Value },
}

fn load_ledger_rows(conn: &Connection) -> anyhow::Result<Vec<LedgerEventRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, event_id, event_type, actor, subject_id, payload_json, prev_hash,
                event_hash, created_at_s, ledger_epoch_id
         FROM ledger_events ORDER BY id ASC",
    )?;
    stmt.query_map([], |row| {
        Ok(LedgerEventRow {
            row_id: row.get(0)?,
            event_id: row.get(1)?,
            event_type: row.get(2)?,
            actor: row.get(3)?,
            subject_id: row.get(4)?,
            payload_json: row.get(5)?,
            prev_hash: row.get(6)?,
            event_hash: row.get(7)?,
            created_at_s: row.get(8)?,
            ledger_epoch_id: row.get(9)?,
        })
    })?
    .collect::<Result<Vec<_>, _>>()
    .map_err(Into::into)
}

fn active_ledger_epoch(conn: &Connection) -> anyhow::Result<Option<LedgerEpochRow>> {
    conn.query_row(
        "SELECT id, status, anchor_hash, repair_certificate_id
         FROM ledger_epochs
         WHERE status = 'active'
         ORDER BY created_at_s DESC
         LIMIT 1",
        [],
        |row| {
            Ok(LedgerEpochRow {
                id: row.get(0)?,
                status: row.get(1)?,
                anchor_hash: row.get(2)?,
                repair_certificate_id: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn load_repair_certificate(conn: &Connection, epoch_id: &str) -> anyhow::Result<Option<Value>> {
    conn.query_row(
        "SELECT id, active_epoch_id, certificate_hash, historical_first_invalid_json,
                verified_head_hash, observed_head_hash, db_total_events, reason, actor,
                ledger_event_id, certificate_json, created_at_s
         FROM ledger_repair_certificates
         WHERE active_epoch_id = ?1
         ORDER BY created_at_s DESC
         LIMIT 1",
        [epoch_id],
        |row| {
            let historical_raw: String = row.get(3)?;
            let certificate_raw: String = row.get(10)?;
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "active_epoch_id": row.get::<_, String>(1)?,
                "certificate_hash": row.get::<_, String>(2)?,
                "historical_first_invalid": serde_json::from_str::<Value>(&historical_raw).unwrap_or_else(|_| json!({})),
                "verified_head_hash": row.get::<_, Option<String>>(4)?,
                "observed_head_hash": row.get::<_, Option<String>>(5)?,
                "db_total_events": row.get::<_, i64>(6)?,
                "reason": row.get::<_, String>(7)?,
                "actor": row.get::<_, String>(8)?,
                "ledger_event_id": row.get::<_, Option<String>>(9)?,
                "certificate": serde_json::from_str::<Value>(&certificate_raw).unwrap_or_else(|_| json!({})),
                "created_at_s": row.get::<_, i64>(11)?
            }))
        },
    )
    .optional()
    .map_err(Into::into)
}

fn load_mutation_records(conn: &Connection) -> anyhow::Result<HashMap<String, MutationRecord>> {
    let mut stmt = conn.prepare(
        "SELECT mutation_id, target_ledger_event_id, mutation_type,
                original_event_hash, post_mutation_payload_hash
         FROM ledger_mutation_events ORDER BY id ASC",
    )?;
    let records: Vec<MutationRecord> = stmt
        .query_map([], |row| {
            Ok(MutationRecord {
                mutation_id: row.get(0)?,
                target_ledger_event_id: row.get(1)?,
                mutation_type: row.get(2)?,
                original_event_hash: row.get(3)?,
                post_mutation_payload_hash: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut map = HashMap::new();
    for rec in records {
        map.entry(rec.target_ledger_event_id.clone())
            .or_insert_with(|| rec);
    }
    Ok(map)
}

pub(crate) fn record_mutation_tx(
    tx: &rusqlite::Transaction<'_>,
    target_ledger_event_id: &str,
    target_row_id: i64,
    mutation_type: &str,
    actor: &str,
    cycle_id: Option<&str>,
    allowed_fields: Value,
    payload_delta: Value,
    original_event_hash: &str,
    post_mutation_payload: Value,
    rationale: &str,
    created_at_s: i64,
) -> anyhow::Result<Value> {
    let prev_mutation_hash: Option<String> = tx
        .query_row(
            "SELECT mutation_hash FROM ledger_mutation_events ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let mutation_id = Uuid::new_v4().to_string();
    let post_mutation_payload_hash = sha256_hex(&canonical_json(&post_mutation_payload)?);
    let hash_input = canonical_json(&json!({
        "mutation_id": mutation_id,
        "target_ledger_event_id": target_ledger_event_id,
        "mutation_type": mutation_type,
        "actor": actor,
        "original_event_hash": original_event_hash,
        "payload_delta": payload_delta,
        "prev_mutation_hash": prev_mutation_hash,
        "created_at_s": created_at_s
    }))?;
    let mutation_hash = sha256_hex(&hash_input);
    tx.execute(
        "INSERT INTO ledger_mutation_events
         (mutation_id, target_ledger_event_id, target_row_id, mutation_type, actor, cycle_id,
          allowed_fields_json, payload_delta_json, original_event_hash, post_mutation_payload_hash,
          rationale, prev_mutation_hash, mutation_hash, created_at_s)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            mutation_id,
            target_ledger_event_id,
            target_row_id,
            mutation_type,
            actor,
            cycle_id,
            canonical_json(&allowed_fields)?,
            canonical_json(&payload_delta)?,
            original_event_hash,
            post_mutation_payload_hash,
            rationale,
            prev_mutation_hash,
            mutation_hash,
            created_at_s,
        ],
    )?;
    Ok(json!({
        "mutation_id": mutation_id,
        "mutation_hash": mutation_hash,
        "target_ledger_event_id": target_ledger_event_id,
        "post_mutation_payload_hash": post_mutation_payload_hash
    }))
}

fn verify_ledger_sequence(
    rows: &[LedgerEventRow],
    starting_prev_hash: Option<String>,
    mutations: &HashMap<String, MutationRecord>,
) -> anyhow::Result<LedgerSequenceVerification> {
    let mut expected_prev_hash = starting_prev_hash;
    let mut verification = LedgerSequenceVerification {
        total_events: rows.len() as i64,
        ..LedgerSequenceVerification::default()
    };
    for row in rows {
        match ledger_row_status(row, &expected_prev_hash, mutations)? {
            LedgerRowStatus::Valid => {
                verification.checked_events += 1;
                verification.last_verified_event_id = Some(row.event_id.clone());
                expected_prev_hash = Some(row.event_hash.clone());
                verification.verified_head_hash = expected_prev_hash.clone();
            }
            LedgerRowStatus::SelfModifiedPreserved { detail } => {
                verification.checked_events += 1;
                verification.last_verified_event_id = Some(row.event_id.clone());
                expected_prev_hash = Some(row.event_hash.clone());
                verification.verified_head_hash = expected_prev_hash.clone();
                verification.self_modified_events.push(detail);
            }
            LedgerRowStatus::Invalid { reason } => {
                verification.first_invalid = Some(reason);
                break;
            }
        }
    }
    Ok(verification)
}

fn ledger_row_status(
    row: &LedgerEventRow,
    expected_prev_hash: &Option<String>,
    mutations: &HashMap<String, MutationRecord>,
) -> anyhow::Result<LedgerRowStatus> {
    if row.event_hash.len() != 64 || !row.event_hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Ok(LedgerRowStatus::Invalid {
            reason: json!({
                "row_id": row.row_id,
                "event_id": row.event_id,
                "event_type": row.event_type,
                "ledger_epoch_id": row.ledger_epoch_id,
                "reason": "event_hash_not_64_hex",
                "observed_event_hash": row.event_hash
            }),
        });
    }
    if &row.prev_hash != expected_prev_hash {
        return Ok(LedgerRowStatus::Invalid {
            reason: json!({
                "row_id": row.row_id,
                "event_id": row.event_id,
                "event_type": row.event_type,
                "ledger_epoch_id": row.ledger_epoch_id,
                "reason": "prev_hash_mismatch",
                "observed_prev_hash": row.prev_hash,
                "expected_prev_hash": expected_prev_hash
            }),
        });
    }
    let payload: Value = match serde_json::from_str(&row.payload_json) {
        Ok(payload) => payload,
        Err(error) => {
            return Ok(LedgerRowStatus::Invalid {
                reason: json!({
                    "row_id": row.row_id,
                    "event_id": row.event_id,
                    "event_type": row.event_type,
                    "ledger_epoch_id": row.ledger_epoch_id,
                    "reason": "payload_json_invalid",
                    "error": error.to_string()
                }),
            });
        }
    };
    let hash_input = canonical_json(&json!({
        "event_id": row.event_id,
        "event_type": row.event_type,
        "actor": row.actor,
        "subject_id": row.subject_id,
        "payload": payload,
        "prev_hash": row.prev_hash,
        "created_at_s": row.created_at_s
    }))?;
    let expected_event_hash = sha256_hex(&hash_input);
    if row.event_hash == expected_event_hash {
        return Ok(LedgerRowStatus::Valid);
    }
    let current_payload_hash = sha256_hex(&canonical_json(&payload)?);
    if let Some(mutation) = mutations.get(&row.event_id) {
        if mutation.original_event_hash == row.event_hash
            && mutation.post_mutation_payload_hash == current_payload_hash
        {
            return Ok(LedgerRowStatus::SelfModifiedPreserved {
                detail: json!({
                    "row_id": row.row_id,
                    "event_id": row.event_id,
                    "event_type": row.event_type,
                    "ledger_epoch_id": row.ledger_epoch_id,
                    "reason": "event_hash_mismatch_self_modified",
                    "observed_event_hash": row.event_hash,
                    "expected_event_hash": expected_event_hash,
                    "mutation_id": mutation.mutation_id,
                    "mutation_type": mutation.mutation_type
                }),
            });
        }
    }
    Ok(LedgerRowStatus::Invalid {
        reason: json!({
            "row_id": row.row_id,
            "event_id": row.event_id,
            "event_type": row.event_type,
            "ledger_epoch_id": row.ledger_epoch_id,
            "reason": "event_hash_mismatch",
            "observed_event_hash": row.event_hash,
            "expected_event_hash": expected_event_hash
        }),
    })
}

fn load_memory_by_id(conn: &Connection, id: &str) -> anyhow::Result<Option<MemoryRow>> {
    conn.query_row(
        "SELECT id, key, value, memory_type, source, session_id,
                COALESCE(project_id, NULL), COALESCE(track, NULL),
                created_at_s, updated_at_s, quality_score, metadata_json
         FROM memories WHERE id = ?1 OR key = ?1 LIMIT 1",
        [id],
        memory_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn memory_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRow> {
    let metadata_json: String = row.get(11)?;
    Ok(MemoryRow {
        id: row.get(0)?,
        key: row.get(1)?,
        value: row.get(2)?,
        memory_type: row.get(3)?,
        source: row.get(4)?,
        session_id: row.get(5)?,
        project_id: row.get(6)?,
        track: row.get(7)?,
        created_at_s: row.get(8)?,
        updated_at_s: row.get(9)?,
        quality_score: row.get(10)?,
        metadata: serde_json::from_str(&metadata_json).unwrap_or_else(|_| json!({})),
    })
}

fn has_recent_duplicate_prefix_tx(
    tx: &rusqlite::Transaction<'_>,
    value: &str,
    now_s: i64,
) -> anyhow::Result<bool> {
    let prefix: String = value.chars().take(300).collect();
    let cutoff = now_s - 3600;
    let exists: i64 = tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM memories
             WHERE substr(value, 1, 300) = ?1
               AND created_at_s >= ?2
             LIMIT 1
         )",
        params![prefix, cutoff],
        |row| row.get(0),
    )?;
    Ok(exists == 1)
}

/// Infer track from source prefix convention.
/// brain.reasoning_bridge → "action", brain.tool → "tool",
/// brain.session → "conversation", brain.result → "result"
fn infer_track_from_source(source: &str) -> Option<String> {
    let lower = source.to_lowercase();
    if lower.starts_with("brain.reasoning") || lower.starts_with("brain.action") {
        Some("action".to_string())
    } else if lower.starts_with("brain.tool") || lower.starts_with("tool.") {
        Some("tool".to_string())
    } else if lower.starts_with("brain.session") || lower.starts_with("session.") {
        Some("conversation".to_string())
    } else if lower.starts_with("brain.result") || lower.starts_with("result.") {
        Some("result".to_string())
    } else {
        None
    }
}

/// Infer track from key prefix convention.
/// tool:* → "tool", session:* → "conversation", result:* → "result", action:* → "action"
fn infer_track_from_key(key: &str) -> Option<String> {
    let lower = key.to_lowercase();
    if lower.starts_with("tool:") {
        Some("tool".to_string())
    } else if lower.starts_with("session:") || lower.starts_with("conversation:") {
        Some("conversation".to_string())
    } else if lower.starts_with("result:") {
        Some("result".to_string())
    } else if lower.starts_with("action:") || lower.starts_with("reasoning:") {
        Some("action".to_string())
    } else {
        None
    }
}

pub(crate) fn tokenize(value: &str) -> Vec<String> {
    value
        .split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|term| {
            let term = term.trim().to_lowercase();
            (term.len() >= 2).then_some(term)
        })
        .collect()
}

pub(crate) fn unix_now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn recall_reports_rrf_formula_and_active_modes() {
        let dir = tempdir().unwrap();
        let store = LocalStore::open(dir.path()).unwrap();
        let saved = store
            .save_memory(SaveInput {
                key: Some("session:test:part1".to_string()),
                value: Some(
                    "Part one brain UDS save recall evidence with weighted router.".to_string(),
                ),
                memory_type: Some("session_debrief".to_string()),
                source: Some("test".to_string()),
                session_id: Some("session:test".to_string()),
                metadata: json!({}),
                project_id: None,
                track: None,
                trusted_generated_artifact: false,
            })
            .unwrap();

        let recalled = store.recall("weighted router evidence", 5).unwrap();
        assert_eq!(recalled["recall_meta"]["planner"], "local_rrf_early_exit");
        let formula = recalled["recall_meta"]["formula"].as_str().unwrap();
        assert!(formula.contains("RRF(k=60)"));
        assert!(!formula.contains("0.40*exact_signal"));
        let modes = recalled["recall_meta"]["modes"].as_array().unwrap();
        assert!(modes.iter().any(|mode| mode.as_str() == Some("text")));
        assert!(modes.iter().any(|mode| mode.as_str() == Some("temporal")));
        assert!(
            !recalled["recall_meta"]["temporal_scope"]["dayBuckets"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            recalled["memories"][0]["memory_id"],
            saved["memory_id"].as_str().unwrap()
        );
    }

    #[test]
    fn exact_key_match_is_pinned_above_broader_text_hits() {
        let dir = tempdir().unwrap();
        let store = LocalStore::open(dir.path()).unwrap();
        store
            .save_memory(SaveInput {
                key: Some("broad:hit".to_string()),
                value: Some(
                    "session:test:part1 appears in this broader explanatory memory because the test verifies exact pinning against text recall.".to_string(),
                ),
                memory_type: Some("declarative".to_string()),
                source: Some("test".to_string()),
                session_id: None,
                metadata: json!({}),
                project_id: None,
                track: None,
                trusted_generated_artifact: false,
            })
            .unwrap();
        let exact = store
            .save_memory(SaveInput {
                key: Some("session:test:part1".to_string()),
                value: Some(
                    "Exact artifact memory was implemented and verified because the recall key must pin to this record."
                        .to_string(),
                ),
                memory_type: Some("declarative".to_string()),
                source: Some("test".to_string()),
                session_id: Some("session:test".to_string()),
                metadata: json!({}),
                project_id: None,
                track: None,
                trusted_generated_artifact: false,
            })
            .unwrap();

        let recalled = store.recall("session:test:part1", 5).unwrap();
        assert_eq!(
            recalled["memories"][0]["memory_id"],
            exact["memory_id"].as_str().unwrap()
        );
        assert_eq!(recalled["memories"][0]["components"]["exact_pin"], true);
    }

    #[test]
    fn save_rejects_secret_content() {
        let dir = tempdir().unwrap();
        let store = LocalStore::open(dir.path()).unwrap();
        let error = store
            .save_memory(SaveInput {
                key: Some("secret:test".to_string()),
                value: Some(
                    "The deployment token is sk-abcdefghijklmnopqrstuvwxyz123456 and must be blocked."
                        .to_string(),
                ),
                memory_type: Some("declarative".to_string()),
                source: Some("test".to_string()),
                session_id: None,
                metadata: json!({}),
                project_id: None,
                track: None,
                trusted_generated_artifact: false,
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains("quality_gate_rejected"));
    }

    #[test]
    fn save_allows_bibliographic_citation_identifier_false_positive() {
        let dir = tempdir().unwrap();
        let store = LocalStore::open(dir.path()).unwrap();
        let saved = store
            .save_memory(SaveInput {
                key: Some("paper:citation:false-positive".to_string()),
                value: Some(
                    "BIBLIOGRAPHIC FULL PAPER DIRECT PDF EXTRACT. The reference cites doi: 10.1145/3292500.3330701, ISBN 978-1-4503-6201-6, and URL https://dl.acm.org/doi/10.1145/3292500.3330701."
                        .to_string(),
                ),
                memory_type: Some("bibliographic_reference".to_string()),
                source: Some("hom-local-paper-ingestion".to_string()),
                session_id: None,
                metadata: json!({"_se_bibliographic_ingest": true}),
                project_id: None,
                track: Some("research".to_string()),
                trusted_generated_artifact: false,
            })
            .unwrap();
        assert_eq!(saved["ok"], true);
        assert_eq!(saved["quality_gate"]["pass"], true);
    }

    #[test]
    fn session_reasoning_generated_artifact_allows_uuid_digit_false_positive_not_token() {
        let dir = tempdir().unwrap();
        let store = LocalStore::open(dir.path()).unwrap();
        let value = "Generated reasoning artifact references bridge event 41111111-1111-1111-a111-000000000000 as an identifier, not as payment data.";

        let normal_error = store
            .save_memory(SaveInput {
                key: Some("reasoning:false-positive:normal".to_string()),
                value: Some(value.to_string()),
                memory_type: Some("session_reasoning".to_string()),
                source: Some("brain.reasoning_bridge".to_string()),
                session_id: Some("session:test".to_string()),
                metadata: json!({}),
                project_id: None,
                track: None,
                trusted_generated_artifact: false,
            })
            .unwrap_err()
            .to_string();
        assert!(normal_error.contains("quality_gate_rejected"));

        let saved = store
            .save_memory(SaveInput {
                key: Some("reasoning:false-positive:generated".to_string()),
                value: Some(value.to_string()),
                memory_type: Some("session_reasoning".to_string()),
                source: Some("brain.reasoning_bridge".to_string()),
                session_id: Some("session:test".to_string()),
                metadata: json!({}),
                project_id: None,
                track: None,
                trusted_generated_artifact: true,
            })
            .unwrap();
        assert!(saved["memory_id"].is_string());

        let token_error = store
            .save_memory(SaveInput {
                key: Some("reasoning:secret:generated".to_string()),
                value: Some(
                    "Generated reasoning artifact must still block api_key = sk-abcdefghijklmnopqrstuvwxyz123456."
                        .to_string(),
                ),
                memory_type: Some("session_reasoning".to_string()),
                source: Some("brain.reasoning_bridge".to_string()),
                session_id: Some("session:test".to_string()),
                metadata: json!({}),
                project_id: None,
                track: None,
                trusted_generated_artifact: true,
            })
            .unwrap_err()
            .to_string();
        assert!(token_error.contains("quality_gate_rejected"));
    }

    #[test]
    fn save_rejects_recent_duplicate_prefix() {
        let dir = tempdir().unwrap();
        let store = LocalStore::open(dir.path()).unwrap();
        let value = "Phase 3 duplicate check implemented because repeated saves should not enter memory twice.";
        store
            .save_memory(SaveInput {
                key: Some("dup:test:1".to_string()),
                value: Some(value.to_string()),
                memory_type: Some("declarative".to_string()),
                source: Some("test".to_string()),
                session_id: None,
                metadata: json!({}),
                project_id: None,
                track: None,
                trusted_generated_artifact: false,
            })
            .unwrap();
        let error = store
            .save_memory(SaveInput {
                key: Some("dup:test:2".to_string()),
                value: Some(value.to_string()),
                memory_type: Some("declarative".to_string()),
                source: Some("test".to_string()),
                session_id: None,
                metadata: json!({}),
                project_id: None,
                track: None,
                trusted_generated_artifact: false,
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains("Duplicate value"));
    }

    #[test]
    fn ledger_verification_accepts_shared_append_chain() {
        let dir = tempdir().unwrap();
        let store = LocalStore::open(dir.path()).unwrap();
        store
            .save_memory(SaveInput {
                key: Some("ledger:valid:test".to_string()),
                value: Some(
                    "Ledger verification accepts events written through append_ledger_tx because the row includes canonical payload, previous hash, timestamp, actor, subject, and event id."
                        .to_string(),
                ),
                memory_type: Some("declarative".to_string()),
                source: Some("test".to_string()),
                session_id: None,
                metadata: json!({}),
                project_id: None,
                track: None,
                trusted_generated_artifact: false,
            })
            .unwrap();

        let verification = store.verify_ledger().unwrap();
        assert_eq!(verification["valid"], true);
        assert_eq!(verification["total_events"], 1);
        assert_eq!(verification["checked_events"], 1);
        assert!(verification["first_invalid"].is_null());
    }

    #[test]
    fn ledger_verification_rejects_empty_hash_rows() {
        let dir = tempdir().unwrap();
        let store = LocalStore::open(dir.path()).unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO ledger_events
                 (event_id, event_type, actor, subject_id, payload_json, prev_hash, event_hash, created_at_s)
                 VALUES ('bad-nightly', 'nightly.run', 'system', NULL, '{}', '', '', 1)",
                [],
            )
            .unwrap();
        }

        let verification = store.verify_ledger().unwrap();
        assert_eq!(verification["valid"], false);
        assert_eq!(verification["checked_events"], 0);
        assert_eq!(verification["first_invalid"]["event_id"], "bad-nightly");
        assert_eq!(
            verification["first_invalid"]["reason"],
            "event_hash_not_64_hex"
        );
    }

    #[test]
    fn vector_embedding_storage_persists_and_exact_searches_nearest_neighbor() {
        let dir = tempdir().unwrap();
        let store = LocalStore::open(dir.path()).unwrap();
        let left = store
            .save_memory(SaveInput {
                key: Some("vector:left".to_string()),
                value: Some("Left vector memory stores deterministic embedding evidence for the exact cosine substrate and verifies nearest-neighbor retrieval before approximate quantization is allowed.".to_string()),
                memory_type: Some("declarative".to_string()),
                source: Some("test".to_string()),
                session_id: None,
                project_id: None,
                track: None,
                metadata: json!({}),
                trusted_generated_artifact: false,
            })
            .unwrap();
        let up = store
            .save_memory(SaveInput {
                key: Some("vector:up".to_string()),
                value: Some("Up vector memory stores deterministic embedding evidence for the exact cosine substrate and verifies ranked retrieval before approximate quantization is allowed.".to_string()),
                memory_type: Some("declarative".to_string()),
                source: Some("test".to_string()),
                session_id: None,
                project_id: None,
                track: None,
                metadata: json!({}),
                trusted_generated_artifact: false,
            })
            .unwrap();

        store
            .upsert_embedding(
                left["memory_id"].as_str().unwrap(),
                "unit-test-embedding",
                &[1.0, 0.0, 0.0],
            )
            .unwrap();
        store
            .upsert_embedding(
                up["memory_id"].as_str().unwrap(),
                "unit-test-embedding",
                &[0.0, 1.0, 0.0],
            )
            .unwrap();

        let result = store
            .exact_vector_search("unit-test-embedding", &[0.9, 0.1, 0.0], 2)
            .unwrap();

        assert_eq!(result["ok"], true);
        assert_eq!(
            result["search"]["formula_ref"],
            "exact_cosine_vector_search_v1"
        );
        assert_eq!(result["search"]["mutation_permitted"], false);
        assert_eq!(result["matches"][0]["memory_id"], left["memory_id"]);
        assert!(
            result["matches"][0]["score"].as_f64().unwrap()
                > result["matches"][1]["score"].as_f64().unwrap()
        );
    }

    #[test]
    fn save_seeds_entities_and_recall_uses_lineage_mode() {
        let dir = tempdir().unwrap();
        let store = LocalStore::open(dir.path()).unwrap();
        let saved = store
            .save_memory(SaveInput {
                key: Some("provider:apollo:router".to_string()),
                value: Some(
                    "Apollo Router implemented PPR lineage recall on 2026-05-12 because evidence required graph traversal.".to_string(),
                ),
                memory_type: Some("declarative".to_string()),
                source: Some("test".to_string()),
                session_id: None,
                metadata: json!({}),
                project_id: None,
                track: None,
                trusted_generated_artifact: false,
            })
            .unwrap();

        {
            let conn = store.conn.lock().unwrap();
            let entity_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM memory_entities", [], |row| row.get(0))
                .unwrap();
            let relationship_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM memory_relationships", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert!(entity_count > 0);
            assert!(relationship_count > 0);
        }

        let recalled = store.recall("apollo lineage", 5).unwrap();
        let modes = recalled["recall_meta"]["modes"].as_array().unwrap();
        assert!(modes.iter().any(|mode| mode.as_str() == Some("lineage")));
        assert_eq!(
            recalled["memories"][0]["memory_id"],
            saved["memory_id"].as_str().unwrap()
        );
        assert_eq!(
            recalled["recall_meta"]["lineage"]["alpha"],
            recall_lineage::PPR_DAMPING
        );
    }

    #[test]
    fn test_record_and_retrieve_tool_execution() {
        let dir = tempdir().unwrap();
        let store = LocalStore::open(dir.path()).unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO route_certificates
                 (id, method, provider_id, model_id, capability, permission_scope,
                  descriptor_hash, policy_id, request_context_json, certificate_hash,
                  status, issued_at_s, expires_at_s)
                 VALUES
                 ('route-456', 'tool.execute', NULL, NULL, 'test-tool', 'test-profile',
                  'abc123def456', 'unit-test-policy', '{}', 'route-cert-hash-456',
                  'issued', 999999, NULL)",
                [],
            )
            .unwrap();
        }

        // Create a test execution certificate
        let cert = ExecutionCertificate {
            call_id: "test-call-123".to_string(),
            thread_id: "test-thread-456".to_string(),
            turn_id: "test-turn-789".to_string(),
            tool_id: "test-tool".to_string(),
            namespace: Some("test-ns".to_string()),
            owner_runtime: "test-runtime".to_string(),
            descriptor_hash: "abc123def456".to_string(),
            permission_profile: "test-profile".to_string(),
            approval_id: Some("approval-123".to_string()),
            sandbox_receipt: Some(json!({ "status": "ok" })),
            route_certificate_id: Some("route-456".to_string()),
            status: "executed".to_string(),
            started_at: 1000000,
            completed_at: Some(2000000),
            input_summary: json!({ "param": "value" }),
            output_summary: Some(json!({ "result": "success" })),
            error: None,
        };

        // Record the execution
        let result = store.record_tool_execution(cert.clone()).unwrap();
        assert_eq!(result["ok"], true);
        assert!(result["message"].as_str().unwrap() == "Tool execution recorded");

        // Retrieve by call_id
        let retrieved = store
            .get_tool_execution_by_call_id("test-call-123")
            .unwrap()
            .unwrap();
        assert_eq!(retrieved.call_id, "test-call-123");
        assert_eq!(retrieved.thread_id, "test-thread-456");
        assert_eq!(retrieved.turn_id, "test-turn-789");
        assert_eq!(retrieved.tool_id, "test-tool");
        assert_eq!(retrieved.namespace, Some("test-ns".to_string()));
        assert_eq!(retrieved.owner_runtime, "test-runtime");
        assert_eq!(retrieved.descriptor_hash, "abc123def456");
        assert_eq!(retrieved.permission_profile, "test-profile");
        assert_eq!(retrieved.approval_id, Some("approval-123".to_string()));
        assert_eq!(
            retrieved.route_certificate_id,
            Some("route-456".to_string())
        );
        assert_eq!(retrieved.status, "executed");
        assert_eq!(retrieved.started_at, 1000000);
        assert_eq!(retrieved.completed_at, Some(2000000));
        assert_eq!(retrieved.input_summary, json!({ "param": "value" }));
        assert_eq!(
            retrieved.output_summary,
            Some(json!({ "result": "success" }))
        );
        assert!(retrieved.error.is_none());

        // Update status
        store
            .update_tool_execution_status(
                "test-call-123",
                "completed",
                Some(3000000),
                Some(json!({ "final": "data" })),
                None,
            )
            .unwrap();

        // Retrieve again to verify update
        let updated = store
            .get_tool_execution_by_call_id("test-call-123")
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, "completed");
        assert_eq!(updated.completed_at, Some(3000000));
        assert_eq!(updated.output_summary, Some(json!({ "final": "data" })));

        // Test retrieval by thread_id
        let thread_executions = store
            .get_tool_executions_by_thread_id("test-thread-456", 10)
            .unwrap();
        assert_eq!(thread_executions.len(), 1);
        assert_eq!(thread_executions[0].call_id, "test-call-123");

        // Test retrieval by tool_id
        let tool_executions = store
            .get_tool_executions_by_tool_id("test-tool", 10)
            .unwrap();
        assert_eq!(tool_executions.len(), 1);
        assert_eq!(tool_executions[0].call_id, "test-call-123");

        // Test non-existent call_id returns None
        let none_result = store.get_tool_execution_by_call_id("non-existent").unwrap();
        assert!(none_result.is_none());
    }
}
