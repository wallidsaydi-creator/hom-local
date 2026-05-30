use rusqlite::Connection;

#[derive(Clone, Debug, serde::Serialize)]
pub struct TrustScore {
    pub source: String,
    pub total_saves: usize,
    pub passed_quality: usize,
    pub avg_quality_score: f64,
    pub avg_calibration_boost: f64,
    pub quality_pass_rate: f64,
    pub evidence_coverage: f64,
    pub factual_precision: f64,
    pub recency: f64,
    pub diagnostic_health: f64,
    pub low_quality_penalty: f64,
    pub missing_evidence_penalty: f64,
    pub formula_ref: &'static str,
    pub trust: f64,
}

pub fn compute_trust_scores(conn: &Connection, limit: usize) -> Vec<TrustScore> {
    let limit = limit.clamp(1, 100);
    let mut stmt = match conn.prepare(
        "SELECT source,
                COUNT(*) as total,
                SUM(CASE WHEN quality_score > 0.0 THEN 1 ELSE 0 END) as passed,
                AVG(quality_score) as avg_quality
         FROM memories
         GROUP BY source
         ORDER BY total DESC
         LIMIT ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows: Vec<(String, usize, usize, f64)> = stmt
        .query_map([limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, usize>(1)?,
                row.get::<_, usize>(2)?,
                row.get::<_, f64>(3).unwrap_or(0.0),
            ))
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();

    rows.into_iter()
        .map(|(source, total, passed, avg_quality)| {
            let quality_pass_rate = smoothed_rate(passed, total);
            let low_quality_penalty = if total > 0 {
                (total.saturating_sub(passed)) as f64 / total as f64
            } else {
                0.0
            };
            let evidence_coverage = 1.0;
            let factual_precision = avg_quality.clamp(0.0, 1.0);
            let recency = 1.0;
            let diagnostic_health = (1.0 - low_quality_penalty).clamp(0.0, 1.0);
            let missing_evidence_penalty = 0.0;
            let trust = (0.25 * quality_pass_rate
                + 0.20 * avg_quality.clamp(0.0, 1.0)
                + 0.20 * evidence_coverage
                + 0.15 * factual_precision
                + 0.10 * recency
                + 0.10 * diagnostic_health
                - 0.15 * low_quality_penalty
                - 0.15 * missing_evidence_penalty)
                .clamp(0.0, 1.0);
            TrustScore {
                source,
                total_saves: total,
                passed_quality: passed,
                avg_quality_score: avg_quality,
                avg_calibration_boost: 1.0,
                quality_pass_rate,
                evidence_coverage,
                factual_precision,
                recency,
                diagnostic_health,
                low_quality_penalty,
                missing_evidence_penalty,
                formula_ref: "source_trust_v2",
                trust,
            }
        })
        .collect()
}

fn smoothed_rate(passed: usize, total: usize) -> f64 {
    (passed as f64 + 1.0) / (total as f64 + 2.0)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::db::migrations::run_migrations;

    #[test]
    fn trust_score_combines_quality_pass_rate_and_average_quality() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO memories
             (id, key, value, memory_type, source, created_at_s, updated_at_s, quality_score, metadata_json)
             VALUES
             ('m1', 'a', 'alpha memory', 'fact', 'agent.a', 1, 1, 1.0, '{}'),
             ('m2', 'b', 'beta memory', 'fact', 'agent.a', 1, 1, 0.5, '{}')",
            [],
        )
        .unwrap();

        let scores = compute_trust_scores(&conn, 10);
        assert_eq!(scores.len(), 1);
        assert_eq!(scores[0].source, "agent.a");
        assert!(scores[0].trust > 0.0);
    }

    #[test]
    fn trust_score_reports_formula_components_and_version() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO memories
             (id, key, value, memory_type, source, created_at_s, updated_at_s, quality_score, metadata_json)
             VALUES ('m1', 'a', 'alpha memory', 'fact', 'agent.a', 1, 1, 1.0, '{}')",
            [],
        )
        .unwrap();

        let scores = compute_trust_scores(&conn, 10);
        assert_eq!(scores[0].formula_ref, "source_trust_v2");
        assert!(scores[0].quality_pass_rate > 0.0);
        assert!(scores[0].low_quality_penalty >= 0.0);
    }

    #[test]
    fn low_quality_penalty_lowers_trust() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO memories
             (id, key, value, memory_type, source, created_at_s, updated_at_s, quality_score, metadata_json)
             VALUES
             ('good', 'a', 'alpha memory', 'fact', 'agent.good', 1, 1, 1.0, '{}'),
             ('bad1', 'b', 'beta memory', 'fact', 'agent.bad', 1, 1, 0.0, '{}'),
             ('bad2', 'c', 'gamma memory', 'fact', 'agent.bad', 1, 1, 0.0, '{}')",
            [],
        )
        .unwrap();

        let scores = compute_trust_scores(&conn, 10);
        let good = scores
            .iter()
            .find(|score| score.source == "agent.good")
            .unwrap();
        let bad = scores
            .iter()
            .find(|score| score.source == "agent.bad")
            .unwrap();
        assert!(bad.low_quality_penalty > good.low_quality_penalty);
        assert!(bad.trust < good.trust);
    }
}
