use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::db::storage::MemoryRow;

use super::ranking_service::RankedItem;

/// Quality-Modulated Decay (QMD) parameters
/// priority_qmd(t, q) = max(FLOOR, BASE^(t * (1 - BETA*q)))
/// Paper: Quality-modulated priority — high-quality memories decay slower.
const BASE: f64 = 0.97;
const BETA: f64 = 0.50;
const FLOOR: f64 = 0.50;

/// Quality-Modulated Priority: replaces discrete freshness_score.
/// t = age_days, q = quality_score ∈ [0,1].
/// High-quality memories retain priority longer because decay exponent
/// is scaled by (1 - β*q): q=1 → exponent halved, q=0 → full decay.
pub fn priority_qmd(age_days: f64, quality_score: f64) -> f64 {
    let q = quality_score.clamp(0.0, 1.0);
    let exponent = age_days * (1.0 - BETA * q);
    (BASE.powf(exponent)).max(FLOOR)
}

/// Display label derived from QMD score range (P0-5: discrete band is
/// display-only, not a scoring input).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreshnessBand {
    Fresh,
    Aging,
    Stale,
    Historical,
}

impl FreshnessBand {
    pub fn as_str(self) -> &'static str {
        match self {
            FreshnessBand::Fresh => "fresh",
            FreshnessBand::Aging => "aging",
            FreshnessBand::Stale => "stale",
            FreshnessBand::Historical => "historical",
        }
    }
}

/// Derive display band from QMD score (not used in scoring).
pub fn freshness_band_from_qmd(qmd: f64) -> FreshnessBand {
    if qmd >= 0.90 {
        FreshnessBand::Fresh
    } else if qmd >= 0.70 {
        FreshnessBand::Aging
    } else if qmd >= 0.55 {
        FreshnessBand::Stale
    } else {
        FreshnessBand::Historical
    }
}

/// Legacy band-from-age kept for backward compat in display contexts.
pub fn freshness_band(age_days: f64) -> FreshnessBand {
    if age_days < 1.0 {
        FreshnessBand::Fresh
    } else if age_days < 7.0 {
        FreshnessBand::Aging
    } else if age_days < 30.0 {
        FreshnessBand::Stale
    } else {
        FreshnessBand::Historical
    }
}

/// Source boost by QMD band — staler memories get larger boost to compensate
/// for lower temporal score (distance advantage principle).
pub fn source_boost_from_qmd(qmd: f64) -> f64 {
    match freshness_band_from_qmd(qmd) {
        FreshnessBand::Fresh => 0.05,
        FreshnessBand::Aging => 0.10,
        FreshnessBand::Stale => 0.20,
        FreshnessBand::Historical => 0.30,
    }
}

/// Legacy wrapper kept for any callers that only have age_days.
pub fn source_boost_distance_advantage(age_days: f64) -> f64 {
    source_boost_from_qmd(priority_qmd(age_days, 0.0))
}

pub fn rank(
    rows: &[MemoryRow],
    terms: &[String],
    now_s: i64,
    current_session_source: Option<&str>,
) -> Vec<RankedItem> {
    if terms.is_empty() {
        return Vec::new();
    }

    let current_source = current_session_source.map(|source| source.to_lowercase());
    let mut ranked = Vec::new();
    for row in rows {
        if row.quality_score <= 0.0 {
            continue;
        }
        let haystack = format!(
            "{} {} {} {}",
            row.key,
            row.value,
            row.source,
            row.session_id.as_deref().unwrap_or("")
        )
        .to_lowercase();
        let matched_terms = terms
            .iter()
            .filter(|term| haystack.contains(term.as_str()))
            .count();
        if matched_terms == 0 {
            continue;
        }

        let age_days = ((now_s - row.created_at_s).max(0) as f64) / 86_400.0;
        let coverage = matched_terms as f64 / terms.len() as f64;
        let qmd = priority_qmd(age_days, row.quality_score);
        let source_match = current_source
            .as_deref()
            .is_some_and(|source| source == row.source.to_lowercase());
        let mode_score = qmd;
        let boost = source_boost_from_qmd(qmd);
        ranked.push((
            coverage,
            source_match,
            qmd,
            boost,
            row.created_at_s,
            row.id.clone(),
            RankedItem::new(row.id.clone(), mode_score),
        ));
    }

    ranked.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| b.4.cmp(&a.4))
            .then_with(|| a.5.cmp(&b.5))
    });

    ranked
        .into_iter()
        .map(|(_, _, _, _, _, _, item)| item)
        .collect()
}

pub fn top_day_buckets(
    rows: &[MemoryRow],
    temporal_ranked: &[RankedItem],
    max_buckets: usize,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut buckets = Vec::new();
    for item in temporal_ranked {
        let Some(row) = rows.iter().find(|row| row.id == item.memory_id) else {
            continue;
        };
        let bucket = day_bucket(row.created_at_s);
        if seen.insert(bucket.clone()) {
            buckets.push(bucket);
        }
        if buckets.len() >= max_buckets {
            break;
        }
    }
    buckets
}

pub fn day_bucket(timestamp_s: i64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp_s, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "1970-01-01".to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn qmd_high_quality_decays_slower() {
        let low_q = priority_qmd(30.0, 0.2);
        let high_q = priority_qmd(30.0, 0.9);
        assert!(
            high_q > low_q,
            "high-quality should retain more priority: high_q={high_q}, low_q={low_q}"
        );
    }

    #[test]
    fn qmd_respects_floor() {
        let very_old = priority_qmd(365.0, 0.0);
        assert!(
            very_old >= FLOOR,
            "should never drop below floor: got {very_old}"
        );
    }

    #[test]
    fn qmd_perfect_quality_halves_decay() {
        // q=1 → exponent = t*(1-0.5*1) = t*0.5, so 60-day q=1 ≈ 30-day q=0
        let q1_60 = priority_qmd(60.0, 1.0);
        let q0_30 = priority_qmd(30.0, 0.0);
        let diff = (q1_60 - q0_30).abs();
        assert!(
            diff < 0.01,
            "q=1 at 60d ≈ q=0 at 30d: q1_60={q1_60}, q0_30={q0_30}, diff={diff}"
        );
    }

    #[test]
    fn qmd_fresh_is_near_one() {
        let fresh = priority_qmd(0.1, 1.0);
        assert!(fresh > 0.99, "fresh memory should be near 1.0: got {fresh}");
    }

    #[test]
    fn source_boost_from_qmd_matches_bands() {
        assert_eq!(source_boost_from_qmd(0.95), 0.05); // Fresh
        assert_eq!(source_boost_from_qmd(0.75), 0.10); // Aging
        assert_eq!(source_boost_from_qmd(0.58), 0.20); // Stale
        assert_eq!(source_boost_from_qmd(0.50), 0.30); // Historical
    }

    #[test]
    fn same_source_wins_when_coverage_matches() {
        let now = 1_700_000_000;
        let rows = vec![
            row("other", "topic", "alpha beta", "other", now),
            row("same", "topic", "alpha beta", "current", now),
        ];
        let ranked = rank(&rows, &["alpha".to_string()], now, Some("current"));
        assert_eq!(ranked[0].memory_id, "same");
    }

    #[test]
    fn day_buckets_are_deduplicated_in_temporal_rank_order() {
        let rows = vec![
            row("a", "topic", "alpha", "test", 1_700_000_000),
            row("b", "topic", "alpha", "test", 1_700_000_000),
        ];
        let ranked = vec![RankedItem::new("a", 1.0), RankedItem::new("b", 1.0)];
        assert_eq!(top_day_buckets(&rows, &ranked, 3).len(), 1);
    }

    fn row(id: &str, key: &str, value: &str, source: &str, created_at_s: i64) -> MemoryRow {
        MemoryRow {
            id: id.to_string(),
            key: key.to_string(),
            value: value.to_string(),
            memory_type: "declarative".to_string(),
            source: source.to_string(),
            session_id: None,
            project_id: None,
            track: None,
            created_at_s,
            updated_at_s: created_at_s,
            quality_score: 1.0,
            metadata: json!({}),
        }
    }
}
