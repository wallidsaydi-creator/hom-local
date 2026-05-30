use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReviewState {
    pub interval_days: u32,
    pub repetition_count: u32,
    pub ease_factor: f64,
}

impl Default for ReviewState {
    fn default() -> Self {
        Self {
            interval_days: 0,
            repetition_count: 0,
            ease_factor: 2.5,
        }
    }
}

pub fn sm2_next(state: ReviewState, quality: u8) -> ReviewState {
    let quality = quality.min(5);
    let ease_factor = sm2_ease_factor(state.ease_factor, quality);
    if quality < 3 {
        return ReviewState {
            interval_days: 1,
            repetition_count: 0,
            ease_factor,
        };
    }

    let repetition_count = state.repetition_count + 1;
    let interval_days = match repetition_count {
        1 => 1,
        2 => 6,
        _ => ((state.interval_days as f64) * state.ease_factor)
            .ceil()
            .max(1.0) as u32,
    };

    ReviewState {
        interval_days,
        repetition_count,
        ease_factor,
    }
}

pub fn schedule_review(state: ReviewState, reviewed_on_day: u32, quality: u8) -> Value {
    let quality = quality.min(5);
    let next = sm2_next(state, quality);
    let next_review_day = reviewed_on_day.saturating_add(next.interval_days);
    json!({
        "ok": true,
        "formula_ref": "sm2_schedule_v2",
        "paper_anchor": "SuperMemo 2: Algorithm, Piotr Wozniak, 1990; I(1)=1, I(2)=6, for n>2 I(n)=I(n-1)*EF; EF'=EF+(0.1-(5-q)*(0.08+(5-q)*0.02)); EF floor 1.3; intervals rounded up",
        "quality": quality,
        "reviewed_on_day": reviewed_on_day,
        "next_review_day": next_review_day,
        "state": state_json(next),
        "previous_state": state_json(state),
        "formula_components": {
            "interval_formula": "I(1)=1; I(2)=6; n>2: ceil(I(n-1)*EF)",
            "ease_factor_formula": "max(1.3, EF + (0.1 - (5-q) * (0.08 + (5-q) * 0.02)))",
            "failure_rule": "if q < 3 then repetition_count=0 and interval_days=1",
            "quality_scale": "0..5"
        },
        "mutation_permitted": false,
        "allowed_mutation_domain": "review_schedule_state",
        "forbidden_mutation_domain": "architecture"
    })
}

fn sm2_ease_factor(ease_factor: f64, quality: u8) -> f64 {
    let q = quality.min(5) as f64;
    (ease_factor + (0.1 - (5.0 - q) * (0.08 + (5.0 - q) * 0.02))).max(1.3)
}

fn state_json(state: ReviewState) -> Value {
    json!({
        "interval_days": state.interval_days,
        "repetition_count": state.repetition_count,
        "ease_factor": state.ease_factor
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaced_repetition_sm2_advances_successful_reviews() {
        let first = sm2_next(ReviewState::default(), 5);
        let second = sm2_next(first, 5);
        assert_eq!(first.interval_days, 1);
        assert_eq!(second.interval_days, 6);
        assert_eq!(second.repetition_count, 2);
    }

    #[test]
    fn spaced_repetition_sm2_resets_failed_reviews() {
        let state = ReviewState {
            interval_days: 20,
            repetition_count: 4,
            ease_factor: 2.3,
        };
        let next = sm2_next(state, 2);
        assert_eq!(next.interval_days, 1);
        assert_eq!(next.repetition_count, 0);
    }

    #[test]
    fn spaced_repetition_schedules_next_review_with_source_metadata() {
        let scheduled = schedule_review(ReviewState::default(), 0, 5);
        assert_eq!(scheduled["formula_ref"], "sm2_schedule_v2");
        assert!(
            scheduled["paper_anchor"]
                .as_str()
                .unwrap()
                .contains("SuperMemo 2")
        );
        assert_eq!(scheduled["reviewed_on_day"], 0);
        assert_eq!(scheduled["next_review_day"], 1);
        assert_eq!(scheduled["state"]["interval_days"], 1);
        assert_eq!(scheduled["mutation_permitted"], false);
        assert_eq!(
            scheduled["allowed_mutation_domain"],
            "review_schedule_state"
        );
    }

    #[test]
    fn spaced_repetition_uses_sm2_round_up_interval_and_min_ease() {
        let state = ReviewState {
            interval_days: 7,
            repetition_count: 2,
            ease_factor: 1.31,
        };
        let scheduled = schedule_review(state, 10, 0);
        assert_eq!(scheduled["state"]["ease_factor"].as_f64().unwrap(), 1.3);
        assert_eq!(scheduled["state"]["repetition_count"], 0);
        assert_eq!(scheduled["next_review_day"], 11);

        let successful = schedule_review(
            ReviewState {
                interval_days: 7,
                repetition_count: 2,
                ease_factor: 2.5,
            },
            10,
            5,
        );
        assert_eq!(successful["state"]["interval_days"], 18);
        assert_eq!(successful["next_review_day"], 28);
    }
}
