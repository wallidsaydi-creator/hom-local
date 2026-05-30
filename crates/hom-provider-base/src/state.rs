use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{CircuitBreaker, ProviderCapability, RateLimit};

pub const SUCCESS_ALPHA: f64 = 0.3;
pub const LATENCY_ALPHA: f64 = 0.2;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProviderState {
    pub ewma_success: f64,
    pub ewma_latency: f64,
    pub breaker: CircuitBreaker,
    pub capabilities: Vec<ProviderCapability>,
    pub last_probe_at: u64,
    pub rate_limit: RateLimit,
}

impl ProviderState {
    pub fn new(capabilities: Vec<ProviderCapability>) -> Self {
        Self {
            ewma_success: 1.0,
            ewma_latency: 0.0,
            breaker: CircuitBreaker::default(),
            capabilities,
            last_probe_at: 0,
            rate_limit: RateLimit::default(),
        }
    }

    pub fn record_result(&mut self, ok: bool, latency_ms: u64, now_ms: u64) {
        let observation = if ok { 1.0 } else { 0.0 };
        self.ewma_success = ewma(self.ewma_success, observation, SUCCESS_ALPHA);

        let latency = latency_ms as f64;
        self.ewma_latency = if self.ewma_latency == 0.0 {
            latency
        } else {
            ewma(self.ewma_latency, latency, LATENCY_ALPHA)
        };

        self.last_probe_at = now_ms / 1000;
        if ok {
            self.breaker.record_success();
        } else {
            self.breaker.record_failure(now_ms);
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct StateMap(pub HashMap<String, ProviderState>);

impl StateMap {
    pub fn ensure(
        &mut self,
        key: impl Into<String>,
        capabilities: Vec<ProviderCapability>,
    ) -> &mut ProviderState {
        self.0
            .entry(key.into())
            .or_insert_with(|| ProviderState::new(capabilities))
    }
}

pub fn ewma(previous: f64, observation: f64, alpha: f64) -> f64 {
    alpha * observation + (1.0 - alpha) * previous
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ewma_uses_contract_alphas() {
        let mut state = ProviderState::new(vec![ProviderCapability::Chat]);
        state.record_result(false, 100, 1);
        assert!((state.ewma_success - 0.7).abs() < f64::EPSILON);
        assert!((state.ewma_latency - 100.0).abs() < f64::EPSILON);
        state.record_result(true, 200, 2);
        assert!((state.ewma_success - 0.79).abs() < 1e-12);
        assert!((state.ewma_latency - 120.0).abs() < 1e-12);
    }
}
