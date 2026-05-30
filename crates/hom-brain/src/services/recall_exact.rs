use crate::db::storage::MemoryRow;

use super::ranking_service::RankedItem;

pub fn rank(rows: &[MemoryRow], query: &str) -> Vec<RankedItem> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let query_lower = query.to_lowercase();
    let mut direct = Vec::new();
    let mut substring = Vec::new();

    for row in rows {
        if row.quality_score <= 0.0 {
            continue;
        }
        if row.id == query || row.key.eq_ignore_ascii_case(query) {
            direct.push((
                row.created_at_s,
                row.id.clone(),
                RankedItem::pinned(row.id.clone(), 1.0),
            ));
            continue;
        }
        let haystack = format!("{} {}", row.key, row.value).to_lowercase();
        if haystack.contains(&query_lower) {
            substring.push((
                row.created_at_s,
                row.id.clone(),
                RankedItem::new(row.id.clone(), 0.7),
            ));
        }
    }

    direct.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    substring.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    direct
        .into_iter()
        .chain(substring)
        .map(|(_, _, item)| item)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn exact_key_match_is_pinned() {
        let rows = vec![row("m1", "session:test", "body")];
        let ranked = rank(&rows, "session:test");
        assert_eq!(ranked.len(), 1);
        assert!(ranked[0].exact_pin);
    }

    fn row(id: &str, key: &str, value: &str) -> MemoryRow {
        MemoryRow {
            id: id.to_string(),
            key: key.to_string(),
            value: value.to_string(),
            memory_type: "declarative".to_string(),
            source: "test".to_string(),
            session_id: None,
            project_id: None,
            track: None,
            created_at_s: 1,
            updated_at_s: 1,
            quality_score: 1.0,
            metadata: json!({}),
        }
    }
}
