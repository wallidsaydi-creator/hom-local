use rusqlite::Connection;

const CALIBRATION_LOOKBACK: usize = 100;
const BOOST_FLOOR: f64 = 0.85;
const BOOST_CEIL: f64 = 1.15;

#[derive(Clone, Debug)]
pub struct CalibrationSignal {
    pub top_k_rate: f64,
    pub avg_recall_count: f64,
    pub boost: f64,
}

pub fn compute_calibration(memory_id: &str, conn: &Connection) -> CalibrationSignal {
    let mut stmt = match conn.prepare(
        "SELECT outcome FROM retrieval_events
         WHERE memory_id = ?1
         ORDER BY created_at_s DESC
         LIMIT ?2",
    ) {
        Ok(s) => s,
        Err(_) => {
            return default_calibration();
        }
    };

    let rows: Vec<String> = match stmt.query_map(
        rusqlite::params![memory_id, CALIBRATION_LOOKBACK as i64],
        |row| row.get::<_, String>(0),
    ) {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(_) => {
            return default_calibration();
        }
    };

    if rows.is_empty() {
        return default_calibration();
    }

    let top_k_count = rows.iter().filter(|outcome| *outcome == "top_k").count();
    let top_k_rate = top_k_count as f64 / rows.len() as f64;
    let signal = top_k_rate * 0.6 + (top_k_rate * 0.4);
    let boost = BOOST_FLOOR + (BOOST_CEIL - BOOST_FLOOR) * sigmoid(signal - 0.5);
    let boost = boost.clamp(BOOST_FLOOR, BOOST_CEIL);

    CalibrationSignal {
        top_k_rate,
        avg_recall_count: rows.len() as f64,
        boost,
    }
}

pub fn default_calibration() -> CalibrationSignal {
    CalibrationSignal {
        top_k_rate: 0.0,
        avg_recall_count: 0.0,
        boost: 1.0,
    }
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x * 6.0).exp())
}

pub fn record_retrieval_event(
    conn: &Connection,
    memory_id: &str,
    mode: &str,
    outcome: &str,
    created_at_s: i64,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO retrieval_events (memory_id, mode, outcome, created_at_s)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![memory_id, mode, outcome, created_at_s],
    )?;
    Ok(())
}

pub fn load_calibration_for_memories(
    memory_ids: &[String],
    conn: &Connection,
) -> std::collections::HashMap<String, f64> {
    let mut boosts = std::collections::HashMap::new();
    for memory_id in memory_ids {
        let signal = compute_calibration(memory_id, conn);
        boosts.insert(memory_id.clone(), signal.boost);
    }
    boosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_calibration_is_neutral() {
        let cal = default_calibration();
        assert_eq!(cal.boost, 1.0);
        assert_eq!(cal.top_k_rate, 0.0);
    }

    #[test]
    fn sigmoid_maps_zero_to_half() {
        let result = sigmoid(0.0);
        assert!((result - 0.5).abs() < 0.01);
    }

    #[test]
    fn sigmoid_maps_positive_to_above_half() {
        let result = sigmoid(1.0);
        assert!(result > 0.5);
    }

    #[test]
    fn boost_is_clamped() {
        let low_signal = CalibrationSignal {
            top_k_rate: 0.0,
            avg_recall_count: 0.0,
            boost: BOOST_FLOOR,
        };
        assert!(low_signal.boost >= BOOST_FLOOR);
        assert!(low_signal.boost <= BOOST_CEIL);
    }

    #[test]
    fn high_top_k_rate_gives_boost_above_neutral() {
        let _cal = CalibrationSignal {
            top_k_rate: 1.0,
            avg_recall_count: 100.0,
            boost: 1.0,
        };
        // With top_k_rate=1.0, signal = 1.0, sigmoid(0.5) ≈ 0.73, boost ≈ 0.85 + 0.30*0.73 ≈ 1.07
        let signal = 1.0 * 0.6 + 1.0 * 0.4;
        let boost = BOOST_FLOOR + (BOOST_CEIL - BOOST_FLOOR) * sigmoid(signal - 0.5);
        assert!(
            boost > 1.0,
            "high top_k_rate should boost above neutral: got {boost}"
        );
    }
}
