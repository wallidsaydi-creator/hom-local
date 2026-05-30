#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ewma {
    alpha: f64,
    value: Option<f64>,
}

impl Ewma {
    pub fn new(alpha: f64) -> Self {
        assert!((0.0..=1.0).contains(&alpha), "EWMA alpha must be in [0, 1]");
        Self { alpha, value: None }
    }

    pub fn update(&mut self, sample: f64) -> f64 {
        let next = match self.value {
            Some(prev) => self.alpha * sample + (1.0 - self.alpha) * prev,
            None => sample,
        };
        self.value = Some(next);
        next
    }

    pub fn value(&self) -> Option<f64> {
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_seeds_then_smooths() {
        let mut ewma = Ewma::new(0.3);
        assert_eq!(ewma.update(1.0), 1.0);
        assert!((ewma.update(0.0) - 0.7).abs() < 1e-12);
        assert!((ewma.update(1.0) - 0.79).abs() < 1e-12);
    }
}
