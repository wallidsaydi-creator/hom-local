const EPSILON: f64 = 1.0e-9;

pub fn kl_component(outcome_count: i64, outcome_total: i64, retrieval_probability: f64) -> f64 {
    if outcome_count <= 0 || outcome_total <= 0 {
        return 0.0;
    }
    let p = outcome_count as f64 / outcome_total as f64;
    p * ((p + EPSILON) / (retrieval_probability + EPSILON)).ln()
}

pub fn signed_outcome_kl(
    mode_total: i64,
    sample_total: i64,
    mode_success: i64,
    success_total: i64,
    mode_failure: i64,
    failure_total: i64,
) -> f64 {
    if mode_total <= 0 || sample_total <= 0 {
        return 0.0;
    }
    let retrieval_probability = mode_total as f64 / sample_total as f64;
    kl_component(mode_success, success_total, retrieval_probability)
        - kl_component(mode_failure, failure_total, retrieval_probability)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kl_alignment_positive_when_success_distribution_exceeds_retrieval_distribution() {
        let signal = signed_outcome_kl(10, 100, 20, 50, 0, 50);
        assert!(signal > 0.0);
    }

    #[test]
    fn kl_alignment_negative_when_failure_distribution_exceeds_retrieval_distribution() {
        let signal = signed_outcome_kl(10, 100, 0, 50, 20, 50);
        assert!(signal < 0.0);
    }
}
