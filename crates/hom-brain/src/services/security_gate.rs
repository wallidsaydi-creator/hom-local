#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretMatch {
    pub pattern: &'static str,
}

pub fn contains_secret_pattern(value: &str) -> bool {
    !secret_matches(value).is_empty()
}

pub fn contains_memory_secret_pattern(value: &str, memory_type: &str) -> bool {
    !memory_secret_matches(value, memory_type).is_empty()
}

pub fn key_value_contains_secret_pattern(key: &str, value: &str) -> bool {
    is_assignment_secret_key(key) && secret_assignment_token(value).is_some()
}

pub fn memory_secret_matches(value: &str, memory_type: &str) -> Vec<SecretMatch> {
    let matches = secret_matches(value);
    if !is_bibliographic_memory_type(memory_type) {
        return matches;
    }

    matches
        .into_iter()
        .filter(|item| !is_bibliographic_false_positive(item, value))
        .collect()
}

/// Post-structural check for trusted generated artifacts.
/// Raw leaf/object-key values must be sanitized before this is used.
pub fn generated_artifact_secret_matches(value: &str) -> Vec<SecretMatch> {
    secret_matches(value)
        .into_iter()
        .filter(|item| !matches!(item.pattern, "credit_card" | "ssn"))
        .collect()
}

fn is_bibliographic_memory_type(memory_type: &str) -> bool {
    matches!(
        memory_type.trim().to_ascii_lowercase().as_str(),
        "bibliographic_reference" | "academic_reference" | "paper_extract" | "book_extract"
    )
}

fn is_bibliographic_false_positive(item: &SecretMatch, value: &str) -> bool {
    if item.pattern != "credit_card" {
        return false;
    }

    let lower = value.to_ascii_lowercase();
    let has_citation_cue = [
        "academic_paper_full_ingestion",
        "bibliographic",
        "doi:",
        "isbn",
        "arxiv:",
        "proceedings",
        "conference",
        "journal",
        "references",
        "url http",
        "dl.acm.org",
        "springer",
        "pages ",
    ]
    .iter()
    .any(|cue| lower.contains(cue));

    let has_payment_cue = [
        "credit card",
        "card number",
        "payment card",
        "visa",
        "mastercard",
        "amex",
        "cvv",
        "cvc",
        "expiration date",
    ]
    .iter()
    .any(|cue| lower.contains(cue));

    has_citation_cue && !has_payment_cue
}

pub fn secret_matches(value: &str) -> Vec<SecretMatch> {
    let mut matches = Vec::new();
    let lower = value.to_lowercase();

    if lower.contains("-----begin ")
        && (lower.contains(" private key-----") || lower.contains(" secret key-----"))
    {
        matches.push(SecretMatch {
            pattern: "private_key_block",
        });
    }

    if lower.contains("authorization: bearer ") || lower.contains("bearer eyj") {
        matches.push(SecretMatch {
            pattern: "bearer_token",
        });
    }

    if contains_assignment_secret(&lower) {
        matches.push(SecretMatch {
            pattern: "credential_assignment",
        });
    }

    if contains_aws_access_key(value) {
        matches.push(SecretMatch {
            pattern: "aws_access_key",
        });
    }

    if contains_prefixed_token(value, &["sk-", "ghp_", "github_pat_", "xoxb-", "xoxp-"]) {
        matches.push(SecretMatch {
            pattern: "api_token",
        });
    }

    if contains_ssn(value) {
        matches.push(SecretMatch { pattern: "ssn" });
    }

    if contains_luhn_card(value) {
        matches.push(SecretMatch {
            pattern: "credit_card",
        });
    }

    matches
}

fn contains_assignment_secret(lower: &str) -> bool {
    for key in ASSIGNMENT_SECRET_KEYS {
        if let Some(index) = lower.find(key) {
            let trimmed = lower[index + key.len()..].trim_start();
            let value = if let Some(rest) = trimmed.strip_prefix("=>") {
                rest
            } else if let Some(rest) = trimmed.strip_prefix('=') {
                rest
            } else if let Some(rest) = trimmed.strip_prefix(':') {
                rest
            } else if let Some(rest) = trimmed.strip_prefix("is ") {
                rest
            } else {
                continue;
            };
            if secret_assignment_token(value).is_some() {
                return true;
            }
        }
    }

    false
}

const ASSIGNMENT_SECRET_KEYS: [&str; 11] = [
    "api_key",
    "apikey",
    "api key",
    "secret_key",
    "secret key",
    "token",
    "password",
    "private_key",
    "private key",
    "client_secret",
    "client secret",
];

fn is_assignment_secret_key(key: &str) -> bool {
    let lower = key.trim().to_lowercase();
    let normalized = lower.replace([' ', '-'], "_");
    let compact: String = lower
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    ASSIGNMENT_SECRET_KEYS.iter().any(|candidate| {
        let candidate_normalized = candidate.replace(' ', "_");
        let candidate_compact: String = candidate
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect();
        normalized == candidate_normalized
            || normalized.ends_with(&format!("_{candidate_normalized}"))
            || compact == candidate_compact
            || compact.ends_with(&candidate_compact)
    })
}

fn secret_assignment_token(value: &str) -> Option<&str> {
    let token = value
        .trim_start_matches(|ch| matches!(ch, '"' | '\'' | '`' | ' '))
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')))
        .next()
        .unwrap_or("");
    is_secret_assignment_token(token).then_some(token)
}

fn is_secret_assignment_token(token: &str) -> bool {
    token.len() >= 16
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn contains_aws_access_key(value: &str) -> bool {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| {
            token.len() == 20
                && (token.starts_with("AKIA") || token.starts_with("ASIA"))
                && token
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
        })
}

fn contains_prefixed_token(value: &str, prefixes: &[&str]) -> bool {
    value
        .split(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | '`' | ',' | ';'))
        .any(|token| {
            prefixes.iter().any(|prefix| {
                token.starts_with(prefix)
                    && token.len() >= prefix.len() + 16
                    && token[prefix.len()..]
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
            })
        })
}

fn contains_ssn(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.windows(11).any(|w| {
        w[3] == b'-'
            && w[6] == b'-'
            && w[0..3].iter().all(u8::is_ascii_digit)
            && w[4..6].iter().all(u8::is_ascii_digit)
            && w[7..11].iter().all(u8::is_ascii_digit)
    })
}

fn contains_luhn_card(value: &str) -> bool {
    let mut digits = Vec::new();
    for ch in value.chars().chain(std::iter::once('\0')) {
        if ch.is_ascii_digit() {
            digits.push(ch.to_digit(10).unwrap() as u8);
            continue;
        }

        if matches!(ch, ' ' | '-' | '.') && !digits.is_empty() {
            continue;
        }

        if (13..=19).contains(&digits.len()) && luhn_valid(&digits) {
            return true;
        }
        digits.clear();
    }
    false
}

fn luhn_valid(digits: &[u8]) -> bool {
    let mut sum = 0u32;
    let mut double = false;
    for digit in digits.iter().rev() {
        let mut value = *digit as u32;
        if double {
            value *= 2;
            if value > 9 {
                value -= 9;
            }
        }
        sum += value;
        double = !double;
    }
    sum % 10 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_secret_patterns() {
        assert!(contains_secret_pattern(
            "api_key = sk-abcdefghijklmnopqrstuvwxyz123456"
        ));
        assert!(contains_secret_pattern("operator ssn 123-45-6789"));
        assert!(contains_secret_pattern(
            "test card 4111 1111 1111 1111 should be blocked"
        ));
    }

    #[test]
    fn ignores_normal_text() {
        assert!(!contains_secret_pattern(
            "Implemented the recall router with deterministic ranking tests."
        ));
    }

    #[test]
    fn detects_secret_key_value_pairs() {
        assert!(key_value_contains_secret_pattern(
            "refresh_token",
            "abcdefghijklmnopqrstuvwxyz123456"
        ));
        assert!(key_value_contains_secret_pattern(
            "accessToken",
            "abcdefghijklmnopqrstuvwxyz123456"
        ));
        assert!(key_value_contains_secret_pattern(
            "clientSecret",
            "abcdefghijklmnopqrstuvwxyz123456"
        ));
        assert!(!key_value_contains_secret_pattern("token_count", "42"));
    }

    #[test]
    fn generated_artifact_filter_ignores_serialized_identifier_pii_false_positive() {
        assert!(contains_secret_pattern("operator ssn 123-45-6789"));
        assert!(contains_secret_pattern(
            "test card 4111 1111 1111 1111 should be blocked"
        ));
        assert!(generated_artifact_secret_matches("operator ssn 123-45-6789").is_empty());
        assert!(
            generated_artifact_secret_matches("test card 4111 1111 1111 1111 should be blocked")
                .is_empty()
        );
        assert!(
            !generated_artifact_secret_matches("api_key = sk-abcdefghijklmnopqrstuvwxyz123456")
                .is_empty()
        );
    }

    #[test]
    fn bibliographic_memory_allows_citation_identifier_false_positive() {
        let citation = "BIBLIOGRAPHIC FULL PAPER DIRECT PDF EXTRACT\n\
            doi: 10.1145/3292500.3330701. ISBN 978-1-4503-6201-6. \
            URL https://dl.acm.org/doi/10.1145/3292500.3330701.";
        assert!(contains_secret_pattern(citation));
        assert!(memory_secret_matches(citation, "bibliographic_reference").is_empty());
    }

    #[test]
    fn bibliographic_memory_still_blocks_real_secret_tokens() {
        let secret = "BIBLIOGRAPHIC FULL PAPER DIRECT PDF EXTRACT\n\
            api_key = sk-abcdefghijklmnopqrstuvwxyz123456";
        assert!(contains_memory_secret_pattern(
            secret,
            "bibliographic_reference"
        ));
    }

    #[test]
    fn bibliographic_memory_does_not_whitelist_payment_card_cues() {
        let payment = "BIBLIOGRAPHIC note mentions credit card 4111 1111 1111 1111.";
        assert!(contains_memory_secret_pattern(
            payment,
            "bibliographic_reference"
        ));
    }
}
