use std::collections::HashMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntityMention {
    pub entity: String,
    pub entity_type: String,
    pub frequency: usize,
}

pub fn extract_entities(text: &str) -> Vec<EntityMention> {
    let mut counts: HashMap<(String, String), usize> = HashMap::new();

    for phrase in title_case_phrases(text) {
        add(&mut counts, phrase, "proper_noun");
    }
    for token in tokens(text) {
        if is_symbol(&token) {
            add(&mut counts, token, "symbol");
        } else if is_camel_or_pascal(&token) {
            add(&mut counts, token, "proper_noun");
        }
    }
    for date in iso_dates(text) {
        add(&mut counts, date, "date");
    }
    for amount in amounts(text) {
        add(&mut counts, amount, "amount");
    }

    let mut entities: Vec<_> = counts
        .into_iter()
        .map(|((entity, entity_type), frequency)| EntityMention {
            entity,
            entity_type,
            frequency,
        })
        .collect();
    entities.sort_by(|a, b| {
        b.frequency
            .cmp(&a.frequency)
            .then_with(|| a.entity.cmp(&b.entity))
            .then_with(|| a.entity_type.cmp(&b.entity_type))
    });
    entities.truncate(20);
    entities
}

pub fn query_entity_candidates(query: &str) -> Vec<String> {
    let mut candidates: Vec<String> = extract_entities(query)
        .into_iter()
        .map(|entity| entity.entity)
        .collect();
    for token in tokens(query) {
        let normalized = normalize(&token);
        if normalized.len() >= 3 && !STOP_WORDS.contains(&normalized.as_str()) {
            candidates.push(normalized);
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn add(counts: &mut HashMap<(String, String), usize>, value: String, entity_type: &str) {
    let entity = normalize(&value);
    if entity.len() < 2 || entity.len() > 80 || STOP_WORDS.contains(&entity.as_str()) {
        return;
    }
    *counts.entry((entity, entity_type.to_string())).or_insert(0) += 1;
}

fn title_case_phrases(text: &str) -> Vec<String> {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '-' && ch != '_')
                .to_string()
        })
        .filter(|word| !word.is_empty())
        .collect();
    let mut phrases = Vec::new();
    let mut current = Vec::new();

    for word in words {
        if is_title_word(&word) && !TITLE_STOP_WORDS.contains(&word.as_str()) {
            current.push(word);
            if current.len() == 4 {
                phrases.push(current.join(" "));
                current.clear();
            }
        } else {
            if !current.is_empty() {
                phrases.push(current.join(" "));
                current.clear();
            }
        }
    }
    if !current.is_empty() {
        phrases.push(current.join(" "));
    }

    phrases
}

fn tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric() && ch != '-' && ch != '_')
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn iso_dates(text: &str) -> Vec<String> {
    text.as_bytes()
        .windows(10)
        .filter_map(|w| {
            let is_date = w[4] == b'-'
                && w[7] == b'-'
                && w[0..4].iter().all(u8::is_ascii_digit)
                && w[5..7].iter().all(u8::is_ascii_digit)
                && w[8..10].iter().all(u8::is_ascii_digit);
            is_date.then(|| String::from_utf8_lossy(w).to_string())
        })
        .collect()
}

fn amounts(text: &str) -> Vec<String> {
    let mut amounts = Vec::new();
    for token in text.split_whitespace() {
        let trimmed = token.trim_matches(|ch: char| matches!(ch, ',' | '.' | ';' | ')'));
        if trimmed.starts_with('$') || trimmed.starts_with("EUR") || trimmed.starts_with("USD") {
            if trimmed.chars().any(|ch| ch.is_ascii_digit()) {
                amounts.push(trimmed.to_string());
            }
        }
    }
    amounts
}

fn normalize(value: &str) -> String {
    value
        .trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '-' && ch != '_' && ch != ' ')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn is_title_word(word: &str) -> bool {
    let mut chars = word.chars();
    chars.next().is_some_and(|ch| ch.is_uppercase()) && chars.any(|ch| ch.is_lowercase())
}

fn is_symbol(token: &str) -> bool {
    (2..=8).contains(&token.len())
        && token
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
        && token.chars().any(|ch| ch.is_ascii_uppercase())
        && !SYMBOL_STOP_WORDS.contains(&token)
}

fn is_camel_or_pascal(token: &str) -> bool {
    token.len() >= 4
        && token.chars().any(|ch| ch.is_lowercase())
        && token.chars().any(|ch| ch.is_uppercase())
        && !TITLE_STOP_WORDS.contains(&token)
}

const TITLE_STOP_WORDS: &[&str] = &[
    "The", "This", "That", "These", "Those", "When", "What", "Where", "Which", "Here", "There",
    "After", "Before", "During", "About", "Also", "Just", "Some", "Each", "Every", "Most", "Many",
    "Phase", "Step",
];

const SYMBOL_STOP_WORDS: &[&str] = &[
    "THE", "AND", "FOR", "BUT", "NOT", "ALL", "ARE", "WAS", "HAS", "HAD", "GET", "SET", "PUT",
    "RUN", "API", "SQL", "URL", "CSS", "DNS", "SSH", "LLM", "NLP", "AAR",
];

const STOP_WORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "but",
    "not",
    "all",
    "are",
    "was",
    "has",
    "had",
    "this",
    "that",
    "with",
    "from",
    "into",
    "onto",
    "memory",
    "recall",
    "query",
    "what",
    "when",
    "where",
    "which",
    "show",
    "find",
    "related",
    "connected",
    "lineage",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_proper_nouns_symbols_and_dates() {
        let entities =
            extract_entities("Apollo Router used PPR on 2026-05-12 because Codex linked evidence.");
        let names: Vec<_> = entities
            .iter()
            .map(|entity| entity.entity.as_str())
            .collect();
        assert!(names.contains(&"apollo router"));
        assert!(names.contains(&"ppr"));
        assert!(names.contains(&"2026-05-12"));
        assert!(names.contains(&"codex"));
    }

    #[test]
    fn query_candidates_include_lowercase_terms() {
        let candidates = query_entity_candidates("apollo lineage");
        assert!(candidates.contains(&"apollo".to_string()));
    }
}
