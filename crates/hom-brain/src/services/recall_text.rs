use std::collections::HashMap;

use crate::db::storage::{MemoryRow, tokenize};

use super::ranking_service::RankedItem;

/// BM25 parameters (Robertson & Zaragoza, 2009)
const K1: f64 = 1.2;
const B: f64 = 0.75;

pub fn rank(rows: &[MemoryRow], terms: &[String]) -> Vec<RankedItem> {
    if terms.is_empty() {
        return Vec::new();
    }

    let query_terms: Vec<String> = terms.iter().map(|t| t.to_lowercase()).collect();

    // Tokenize each document and compute term frequencies + document length
    let doc_data: Vec<(usize, HashMap<String, usize>)> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.quality_score > 0.0)
        .map(|(idx, row)| {
            let doc_text = format!("{} {}", row.key, row.value).to_lowercase();
            let tokens = tokenize(&doc_text);
            let mut tf: HashMap<String, usize> = HashMap::new();
            for token in &tokens {
                *tf.entry(token.clone()).or_insert(0) += 1;
            }
            (idx, tf)
        })
        .collect();

    if doc_data.is_empty() {
        return Vec::new();
    }

    // Average document length (in tokens)
    let total_len: usize = doc_data
        .iter()
        .map(|(_, tf)| tf.values().sum::<usize>())
        .sum();
    let avgdl = total_len as f64 / doc_data.len() as f64;

    let n_docs = doc_data.len() as f64;

    // Document frequency for each query term
    let df: HashMap<&str, usize> = query_terms
        .iter()
        .map(|qt| {
            let count = doc_data
                .iter()
                .filter(|(_, tf)| tf.contains_key(qt.as_str()))
                .count();
            (qt.as_str(), count)
        })
        .collect();

    let mut scored: Vec<(f64, usize, f64, i64, String, RankedItem)> = Vec::new();

    for (idx, tf) in &doc_data {
        let row = &rows[*idx];
        let doc_len: f64 = tf.values().sum::<usize>() as f64;

        let mut bm25_score: f64 = 0.0;
        let mut matched_count: usize = 0;

        for qt in &query_terms {
            let n_qi = df.get(qt.as_str()).copied().unwrap_or(0) as f64;
            if n_qi == 0.0 {
                continue;
            }
            let f_qi = tf.get(qt.as_str()).copied().unwrap_or(0) as f64;
            if f_qi == 0.0 {
                continue;
            }
            matched_count += 1;

            // IDF(qi) = log((N - n(qi) + 0.5) / (n(qi) + 0.5) + 1)
            let idf = ((n_docs - n_qi + 0.5) / (n_qi + 0.5) + 1.0).ln();

            // TF component: (f * (k1+1)) / (f + k1 * (1 - b + b * dl/avgdl))
            let tf_component = (f_qi * (K1 + 1.0)) / (f_qi + K1 * (1.0 - B + B * doc_len / avgdl));

            bm25_score += idf * tf_component;
        }

        if matched_count == 0 {
            continue;
        }

        scored.push((
            bm25_score,
            matched_count,
            row.quality_score,
            row.created_at_s,
            row.id.clone(),
            RankedItem::new(row.id.clone(), bm25_score),
        ));
    }

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| b.3.cmp(&a.3))
            .then_with(|| a.4.cmp(&b.4))
    });

    scored
        .into_iter()
        .map(|(_, _, _, _, _, item)| item)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn bm25_rare_term_beats_frequent() {
        // "zebra" appears in 1 doc → high IDF; "alpha" in both → lower IDF
        let rows = vec![
            row("m1", "alpha", "common alpha text repeated many times"),
            row("m2", "alpha zebra", "rare term match"),
        ];
        let ranked = rank(&rows, &["alpha".to_string(), "zebra".to_string()]);
        assert_eq!(ranked[0].memory_id, "m2");
    }

    #[test]
    fn empty_query_returns_nothing() {
        let rows = vec![row("m1", "alpha", "beta")];
        let ranked = rank(&rows, &[]);
        assert!(ranked.is_empty());
    }

    #[test]
    fn no_match_returns_nothing() {
        let rows = vec![row("m1", "alpha", "beta")];
        let ranked = rank(&rows, &["xyzzy".to_string()]);
        assert!(ranked.is_empty());
    }

    #[test]
    fn longer_doc_gets_length_penalty() {
        // Same tf, but longer doc should score lower (b > 0)
        let rows = vec![
            row("short", "alpha", "alpha"),
            row(
                "long",
                "alpha",
                "alpha alpha alpha extra padding words here and more",
            ),
        ];
        let ranked = rank(&rows, &["alpha".to_string()]);
        assert_eq!(ranked[0].memory_id, "short");
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
