#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeightUpdate {
    pub proposed_delta: f64,
    pub proposed_weight: f64,
    pub new_weight: f64,
    pub applied_delta: f64,
}

pub fn bounded_ewma_update(
    prior_weight: f64,
    signed_signal: f64,
    eta: f64,
    delta_cap: f64,
    min_weight: f64,
    max_weight: f64,
    ewma_old: f64,
    ewma_new: f64,
) -> WeightUpdate {
    let prior_weight = prior_weight.clamp(min_weight, max_weight);
    let proposed_delta = (eta * signed_signal).clamp(-delta_cap, delta_cap);
    let proposed_weight = (prior_weight + proposed_delta).clamp(min_weight, max_weight);
    let new_weight =
        (ewma_old * prior_weight + ewma_new * proposed_weight).clamp(min_weight, max_weight);
    WeightUpdate {
        proposed_delta,
        proposed_weight,
        new_weight,
        applied_delta: new_weight - prior_weight,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_weight_update_clamps_delta_and_weight() {
        let update = bounded_ewma_update(0.95, 10.0, 0.1, 0.05, 0.5, 1.0, 0.8, 0.2);
        assert_eq!(update.proposed_delta, 0.05);
        assert!(update.new_weight <= 1.0);
        assert!(update.applied_delta > 0.0);
    }
}
