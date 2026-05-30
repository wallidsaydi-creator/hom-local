use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CircuitBreaker {
    pub state: BreakerState,
    pub failure_count: u32,
    pub failure_threshold: u32,
    pub cooldown_ms: u64,
    pub opened_at_ms: Option<u64>,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self {
            state: BreakerState::Closed,
            failure_count: 0,
            failure_threshold: 5,
            cooldown_ms: 30_000,
            opened_at_ms: None,
        }
    }
}

impl CircuitBreaker {
    pub fn can_call(&mut self, now_ms: u64) -> bool {
        match self.state {
            BreakerState::Closed | BreakerState::HalfOpen => true,
            BreakerState::Open => {
                let opened_at = self.opened_at_ms.unwrap_or(now_ms);
                if now_ms.saturating_sub(opened_at) >= self.cooldown_ms {
                    self.state = BreakerState::HalfOpen;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn record_success(&mut self) {
        self.state = BreakerState::Closed;
        self.failure_count = 0;
        self.opened_at_ms = None;
    }

    pub fn record_failure(&mut self, now_ms: u64) {
        match self.state {
            BreakerState::HalfOpen => self.open(now_ms),
            BreakerState::Closed | BreakerState::Open => {
                self.failure_count = self.failure_count.saturating_add(1);
                if self.failure_count >= self.failure_threshold {
                    self.open(now_ms);
                }
            }
        }
    }

    fn open(&mut self, now_ms: u64) {
        self.state = BreakerState::Open;
        self.opened_at_ms = Some(now_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_after_five_failures_and_half_opens_after_cooldown() {
        let mut breaker = CircuitBreaker::default();
        for i in 0..4 {
            breaker.record_failure(i);
            assert_eq!(breaker.state, BreakerState::Closed);
        }
        breaker.record_failure(5);
        assert_eq!(breaker.state, BreakerState::Open);
        assert!(!breaker.can_call(10_000));
        assert!(breaker.can_call(35_001));
        assert_eq!(breaker.state, BreakerState::HalfOpen);
        breaker.record_success();
        assert_eq!(breaker.state, BreakerState::Closed);
        assert_eq!(breaker.failure_count, 0);
    }
}
