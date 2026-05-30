use serde::Serialize;

use crate::services::security_gate;

pub const SUBSTANCE_REFERENCE: &str = "Four-wall deterministic gate: form, filter, substance, factuality; factuality uses EvidenceAtom/FActScore-style atomic precision when supplied";

#[derive(Clone, Debug, Serialize)]
pub struct QualityAssessment {
    pub pass: bool,
    pub score: f64,
    pub reason: Option<String>,
    pub walls: QualityWalls,
    pub reference: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct QualityWalls {
    pub form: WallReport,
    pub filter: Option<WallReport>,
    pub substance: Option<WallReport>,
    pub factuality: Option<WallReport>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WallReport {
    pub pass: bool,
    pub score: Option<f64>,
    pub reason: Option<String>,
}

pub fn assess_quality(key: &str, value: &str, memory_type: &str) -> QualityAssessment {
    assess_quality_with_options(key, value, memory_type, None, false)
}

pub fn assess_generated_artifact(key: &str, value: &str, memory_type: &str) -> QualityAssessment {
    assess_quality_with_options(key, value, memory_type, None, true)
}

pub fn assess_quality_with_factuality(
    key: &str,
    value: &str,
    memory_type: &str,
    factuality_score: Option<f64>,
) -> QualityAssessment {
    assess_quality_with_options(key, value, memory_type, factuality_score, false)
}

fn assess_quality_with_options(
    key: &str,
    value: &str,
    memory_type: &str,
    factuality_score: Option<f64>,
    generated_artifact: bool,
) -> QualityAssessment {
    let form = wall1_form(value);
    if !form.pass {
        return QualityAssessment {
            pass: false,
            score: 0.0,
            reason: form.reason.clone(),
            walls: QualityWalls {
                form,
                filter: None,
                substance: None,
                factuality: None,
            },
            reference: SUBSTANCE_REFERENCE,
        };
    }

    let filter = wall2_filter_with_options(value, memory_type, generated_artifact);
    if !filter.pass {
        return QualityAssessment {
            pass: false,
            score: 0.0,
            reason: filter.reason.clone(),
            walls: QualityWalls {
                form,
                filter: Some(filter),
                substance: None,
                factuality: None,
            },
            reference: SUBSTANCE_REFERENCE,
        };
    }

    let substance = wall3_substance(key, value, memory_type);
    if !substance.pass {
        return QualityAssessment {
            pass: false,
            score: substance.score.unwrap_or(0.0),
            reason: substance.reason.clone(),
            walls: QualityWalls {
                form,
                filter: Some(filter),
                substance: Some(substance),
                factuality: None,
            },
            reference: SUBSTANCE_REFERENCE,
        };
    }

    let factuality = factuality_score.map(wall4_factuality);
    if let Some(factuality_wall) = factuality.clone() {
        if !factuality_wall.pass {
            return QualityAssessment {
                pass: false,
                score: factuality_wall.score.unwrap_or(0.0),
                reason: factuality_wall.reason.clone(),
                walls: QualityWalls {
                    form,
                    filter: Some(filter),
                    substance: Some(substance),
                    factuality: Some(factuality_wall),
                },
                reference: SUBSTANCE_REFERENCE,
            };
        }
    }

    QualityAssessment {
        pass: true,
        score: substance.score.unwrap_or(1.0),
        reason: None,
        walls: QualityWalls {
            form,
            filter: Some(filter),
            substance: Some(substance),
            factuality,
        },
        reference: SUBSTANCE_REFERENCE,
    }
}

pub fn wall1_form(value: &str) -> WallReport {
    let value = value.trim();
    if value.is_empty() {
        return fail("Empty value");
    }
    if value.chars().count() < 20 {
        return fail(format!(
            "Too short ({} chars, minimum 20)",
            value.chars().count()
        ));
    }
    if is_kill_pattern(value) {
        return fail("Kill pattern");
    }
    pass(None, None)
}

pub fn wall2_filter(value: &str) -> WallReport {
    wall2_filter_with_options(value, "declarative", false)
}

fn wall2_filter_with_options(
    value: &str,
    memory_type: &str,
    generated_artifact: bool,
) -> WallReport {
    let secret_matches = if generated_artifact {
        security_gate::generated_artifact_secret_matches(value)
    } else {
        security_gate::memory_secret_matches(value, memory_type)
    };
    if !secret_matches.is_empty() {
        return fail("Secret or credential pattern detected");
    }
    if heartbeat_spam(value) {
        return fail("Heartbeat spam pattern");
    }
    if !generated_artifact && has_excessive_repetition(value) {
        return fail("Repetition above 50% threshold");
    }
    pass(None, None)
}

pub fn wall3_substance(key: &str, value: &str, memory_type: &str) -> WallReport {
    if is_exempt_type(memory_type) || is_exempt_key(key) {
        return pass(Some(1.0), None);
    }

    let score = substance_score(value);
    if score < 0.30 {
        return WallReport {
            pass: false,
            score: Some(score),
            reason: Some(format!(
                "Insufficient substance ({score:.2} < 0.30 threshold)"
            )),
        };
    }

    pass(Some(score), None)
}

pub fn wall4_factuality(score: f64) -> WallReport {
    let score = score.clamp(0.0, 1.0);
    if score < 0.70 {
        return WallReport {
            pass: false,
            score: Some(score),
            reason: Some(format!("Factuality below threshold ({score:.2} < 0.70)")),
        };
    }
    pass(Some(score), None)
}

pub fn substance_score(value: &str) -> f64 {
    let value = value.trim();
    let mut score: f64 = 0.25;

    if has_specifics(value) {
        score += 0.15;
    }
    if has_file_path(value) {
        score += 0.10;
    }

    let len = value.chars().count();
    if len > 500 {
        score += 0.20;
    } else if len > 200 {
        score += 0.15;
    } else if len > 100 {
        score += 0.10;
    }

    if has_reasoning(value) {
        score += 0.20;
    }
    if has_action(value) {
        score += 0.10;
    }
    if has_structure(value) {
        score += 0.10;
    }

    score.min(1.0)
}

fn pass(score: Option<f64>, reason: Option<String>) -> WallReport {
    WallReport {
        pass: true,
        score,
        reason,
    }
}

fn fail(reason: impl Into<String>) -> WallReport {
    WallReport {
        pass: false,
        score: Some(0.0),
        reason: Some(reason.into()),
    }
}

fn is_kill_pattern(value: &str) -> bool {
    let lower = value.trim().to_lowercase();
    if matches!(
        lower.as_str(),
        "undefined"
            | "null"
            | "none"
            | "n/a"
            | "ok"
            | "yes"
            | "no"
            | "true"
            | "false"
            | "done"
            | "test"
            | "ping"
            | "pong"
            | "error"
            | "{}"
            | "[]"
            | "\"\""
            | "''"
    ) {
        return true;
    }
    if lower.chars().all(|ch| ch.is_ascii_digit()) {
        return true;
    }
    if lower
        .split(',')
        .all(|segment| segment.trim() == "mixed events")
    {
        return true;
    }

    let words: Vec<_> = lower.split_whitespace().collect();
    words.len() >= 3 && words.iter().all(|word| *word == words[0])
}

fn heartbeat_spam(value: &str) -> bool {
    let lower = value.to_lowercase();
    lower.contains("knowledge gate blocked")
        || (lower.contains("system heartbeat") && lower.contains("30 minutes"))
}

fn has_excessive_repetition(value: &str) -> bool {
    if max_run_len(value) >= 10 {
        return true;
    }

    let segments: Vec<_> = value
        .split(['\n', '.'])
        .map(|segment| segment.trim().to_lowercase())
        .filter(|segment| segment.len() > 5)
        .collect();
    if segments.len() <= 2 {
        return false;
    }
    let unique: std::collections::HashSet<_> = segments.iter().collect();
    let repetition = 1.0 - (unique.len() as f64 / segments.len() as f64);
    repetition > 0.50
}

fn max_run_len(value: &str) -> usize {
    let mut max_run = 0;
    let mut current_run = 0;
    let mut previous = '\0';
    for ch in value.chars() {
        if ch == previous && !ch.is_whitespace() {
            current_run += 1;
        } else {
            current_run = 1;
            previous = ch;
        }
        max_run = max_run.max(current_run);
    }
    max_run
}

fn is_exempt_type(memory_type: &str) -> bool {
    matches!(
        memory_type.trim(),
        "session_debrief"
            | "strategic_directive"
            | "operational_rule"
            | "constitution"
            | "procedural"
            | "action_trace"
            | "session_compaction"
            | "session_reasoning"
    )
}

fn is_exempt_key(key: &str) -> bool {
    ["paper:", "book:", "heartbeat:pulse", "heartbeat:latest"]
        .iter()
        .any(|prefix| key == *prefix || key.starts_with(prefix))
}

fn has_specifics(value: &str) -> bool {
    has_iso_date(value)
        || has_time(value)
        || has_absolute_path(value)
        || has_version(value)
        || has_proper_name(value)
        || value.contains("0x")
}

fn has_iso_date(value: &str) -> bool {
    value.as_bytes().windows(10).any(|w| {
        w[4] == b'-'
            && w[7] == b'-'
            && w[0..4].iter().all(u8::is_ascii_digit)
            && w[5..7].iter().all(u8::is_ascii_digit)
            && w[8..10].iter().all(u8::is_ascii_digit)
    })
}

fn has_time(value: &str) -> bool {
    value.as_bytes().windows(5).any(|w| {
        w[2] == b':'
            && w[0..2].iter().all(u8::is_ascii_digit)
            && w[3..5].iter().all(u8::is_ascii_digit)
    })
}

fn has_absolute_path(value: &str) -> bool {
    value.split_whitespace().any(|token| {
        token.starts_with('/')
            && token
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/'))
                .count()
                >= 4
    })
}

fn has_version(value: &str) -> bool {
    value.split_whitespace().any(|token| {
        let parts: Vec<_> = token
            .trim_matches(|ch: char| !ch.is_ascii_digit() && ch != '.')
            .split('.')
            .collect();
        parts.len() >= 3
            && parts
                .iter()
                .all(|part| part.chars().all(|ch| ch.is_ascii_digit()))
    })
}

fn has_proper_name(value: &str) -> bool {
    let words: Vec<_> = value
        .split_whitespace()
        .map(|word| word.trim_matches(|ch: char| !ch.is_alphabetic()))
        .filter(|word| !word.is_empty())
        .collect();
    words.windows(2).any(|pair| {
        pair.iter().all(|word| {
            let mut chars = word.chars();
            chars.next().is_some_and(|ch| ch.is_uppercase()) && chars.any(|ch| ch.is_lowercase())
        })
    })
}

fn has_file_path(value: &str) -> bool {
    const EXTENSIONS: [&str; 12] = [
        ".js", ".ts", ".json", ".md", ".py", ".sh", ".yml", ".yaml", ".sql", ".html", ".css", ".rs",
    ];
    value.split_whitespace().any(|token| {
        let token = token.trim_matches(|ch: char| matches!(ch, ',' | ';' | ':' | ')' | '('));
        EXTENSIONS
            .iter()
            .any(|extension| token.ends_with(extension))
    })
}

fn has_reasoning(value: &str) -> bool {
    let lower = value.to_lowercase();
    [
        "because",
        "therefore",
        "root cause",
        "decision",
        "why",
        "since",
        "as a result",
        "consequence",
        "hence",
        "thus",
        "determined",
        "concluded",
        "analysis shows",
        "evidence",
        "verified",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn has_action(value: &str) -> bool {
    let lower = value.to_lowercase();
    [
        "fixed",
        "implemented",
        "created",
        "deployed",
        "tested",
        "resolved",
        "added",
        "removed",
        "updated",
        "refactored",
        "migrated",
        "configured",
        "debugged",
        "shipped",
        "merged",
        "committed",
        "reverted",
        "discovered",
        "identified",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn has_structure(value: &str) -> bool {
    let lower = value.to_lowercase();
    lower.contains("step ")
        || lower.contains("phase ")
        || lower.contains("key:")
        || lower.contains("value:")
        || lower.contains("result:")
        || lower.contains("status:")
        || lower.contains("error:")
        || lower.contains("warning:")
        || value.contains("->")
        || value.contains("==>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_fails_form_wall() {
        let assessment = assess_quality("memory:test", "   ", "declarative");
        assert!(!assessment.pass);
        assert_eq!(assessment.score, 0.0);
        assert!(!assessment.walls.form.pass);
    }

    #[test]
    fn secret_pattern_fails_filter_wall() {
        let assessment = assess_quality(
            "memory:test",
            "The API token is sk-abcdefghijklmnopqrstuvwxyz123456 and must not enter memory.",
            "declarative",
        );
        assert!(!assessment.pass);
        assert!(assessment.walls.form.pass);
        assert!(!assessment.walls.filter.unwrap().pass);
    }

    #[test]
    fn bibliographic_reference_allows_citation_numbers_after_filter_wall() {
        let assessment = assess_quality(
            "paper:citation",
            "BIBLIOGRAPHIC FULL PAPER DIRECT PDF EXTRACT. The reference cites doi: 10.1145/3292500.3330701, ISBN 978-1-4503-6201-6, and URL https://dl.acm.org/doi/10.1145/3292500.3330701.",
            "bibliographic_reference",
        );
        assert!(assessment.pass);
        assert!(assessment.walls.form.pass);
        assert!(assessment.walls.filter.unwrap().pass);
    }

    #[test]
    fn bibliographic_reference_still_blocks_real_secret_after_filter_wall() {
        let assessment = assess_quality(
            "paper:secret",
            "BIBLIOGRAPHIC FULL PAPER DIRECT PDF EXTRACT. api_key = sk-abcdefghijklmnopqrstuvwxyz123456",
            "bibliographic_reference",
        );
        assert!(!assessment.pass);
        assert!(assessment.walls.form.pass);
        assert!(!assessment.walls.filter.unwrap().pass);
    }

    #[test]
    fn low_substance_text_fails_substance_wall() {
        let assessment = assess_quality(
            "memory:test",
            "This sentence is long enough but generic filler",
            "declarative",
        );
        assert!(!assessment.pass);
        assert!(assessment.walls.substance.unwrap().score.unwrap() < 0.30);
    }

    #[test]
    fn well_formed_paragraph_passes_all_walls() {
        let assessment = assess_quality(
            "memory:test",
            "Phase 3 implemented storage.rs quality gate on 2026-05-12 because the prior length heuristic created fake green status.",
            "declarative",
        );
        assert!(assessment.pass);
        assert!(assessment.score >= 0.30);
        assert!(assessment.walls.form.pass);
        assert!(assessment.walls.filter.unwrap().pass);
        assert!(assessment.walls.substance.unwrap().pass);
    }

    #[test]
    fn exempt_types_skip_substance_scoring_after_form_and_filter() {
        let assessment = assess_quality(
            "memory:test",
            "Session debrief text passes structural checks.",
            "session_debrief",
        );
        assert!(assessment.pass);
        assert_eq!(assessment.score, 1.0);
    }

    #[test]
    fn generated_artifact_skips_repetition_but_keeps_secret_filter() {
        let repetitive = (0..12)
            .map(|_| "same generated manifest segment.")
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!assess_quality("artifact:test", &repetitive, "session_compaction").pass);
        assert!(assess_generated_artifact("artifact:test", &repetitive, "session_compaction").pass);

        let secret = format!(
            "Session compaction artifact api_key = sk-{}",
            "abcdefghijklmnopqrstuvwxyz123456"
        );
        assert!(!assess_generated_artifact("artifact:test", &secret, "session_compaction").pass);
    }

    #[test]
    fn factuality_wall_is_distinct_from_filter_wall() {
        let assessment = assess_quality_with_factuality(
            "memory:test",
            "Phase 3 implemented storage.rs quality gate on 2026-05-12 because evidence verified the behavior.",
            "declarative",
            Some(0.40),
        );
        assert!(!assessment.pass);
        assert!(assessment.walls.form.pass);
        assert!(assessment.walls.filter.unwrap().pass);
        assert!(assessment.walls.substance.unwrap().pass);
        assert!(!assessment.walls.factuality.unwrap().pass);
        assert_eq!(
            assessment.reason.unwrap(),
            "Factuality below threshold (0.40 < 0.70)"
        );
    }

    #[test]
    fn secret_failure_short_circuits_before_factuality_wall() {
        let assessment = assess_quality_with_factuality(
            "memory:test",
            "The API token is sk-abcdefghijklmnopqrstuvwxyz123456 and must not enter memory.",
            "declarative",
            Some(1.0),
        );
        assert!(!assessment.pass);
        assert!(assessment.walls.form.pass);
        assert!(!assessment.walls.filter.unwrap().pass);
        assert!(assessment.walls.factuality.is_none());
    }

    #[test]
    fn factuality_not_applicable_keeps_existing_save_behavior() {
        let assessment = assess_quality_with_factuality(
            "memory:test",
            "Phase 3 implemented storage.rs quality gate on 2026-05-12 because the prior length heuristic created fake green status.",
            "declarative",
            None,
        );
        assert!(assessment.pass);
        assert!(assessment.walls.factuality.is_none());
    }
}
