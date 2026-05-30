use crate::db::storage::MemoryRow;

use super::ranking_service::RankedItem;

pub fn rank(rows: &[MemoryRow], query: &str) -> Vec<RankedItem> {
    let identifiers = extract_identifiers(query);
    if identifiers.is_empty() {
        return Vec::new();
    }

    let mut ranked = Vec::new();
    for row in rows {
        if row.quality_score <= 0.0 {
            continue;
        }
        let key = row.key.to_lowercase();
        let session = row.session_id.as_deref().unwrap_or("").to_lowercase();
        let mut hits = 0usize;
        for identifier in &identifiers {
            if key == *identifier
                || key.starts_with(identifier)
                || key.contains(identifier)
                || session == *identifier
                || session.starts_with(identifier)
            {
                hits += 1;
            }
        }
        if hits > 0 {
            let score = (hits as f64 / identifiers.len() as f64).clamp(0.0, 1.0);
            ranked.push((
                score,
                row.created_at_s,
                row.id.clone(),
                RankedItem::new(row.id.clone(), score),
            ));
        }
    }

    ranked.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    ranked.into_iter().map(|(_, _, _, item)| item).collect()
}

fn extract_identifiers(query: &str) -> Vec<String> {
    let mut identifiers: Vec<String> = query
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| {
                    !(ch.is_alphanumeric() || matches!(ch, ':' | '-' | '_' | '/' | '.'))
                })
                .to_lowercase()
        })
        .filter(|token| is_identifier_like(token))
        .collect();
    identifiers.sort();
    identifiers.dedup();
    identifiers
}

fn is_identifier_like(token: &str) -> bool {
    if token.len() < 3 {
        return false;
    }
    token.contains(':')
        || token.contains('/')
        || token.contains('_')
        || token.contains('-')
        || looks_like_uuid(token)
}

fn looks_like_uuid(token: &str) -> bool {
    token.len() == 36
        && token.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-')
        && token.chars().filter(|ch| *ch == '-').count() == 4
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn structured_key_prefix_matches_identifier() {
        let rows = vec![row(
            "m1",
            "session:test:part1",
            "body",
            Some("session:test"),
        )];
        let ranked = rank(&rows, "session:test");
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].memory_id, "m1");
    }

    fn row(id: &str, key: &str, value: &str, session_id: Option<&str>) -> MemoryRow {
        MemoryRow {
            id: id.to_string(),
            key: key.to_string(),
            value: value.to_string(),
            memory_type: "declarative".to_string(),
            source: "test".to_string(),
            session_id: session_id.map(str::to_string),
            project_id: None,
            track: None,
            created_at_s: 1,
            updated_at_s: 1,
            quality_score: 1.0,
            metadata: json!({}),
        }
    }
}
