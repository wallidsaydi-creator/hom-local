use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitClass {
    Read,
    Write,
}

#[derive(Clone, Debug)]
struct Window {
    start_s: i64,
    count: u32,
}

#[derive(Clone, Debug, Default)]
pub struct RateLimiter {
    windows: HashMap<(String, &'static str), Window>,
}

impl RateLimiter {
    pub fn check(&mut self, client_id: &str, class: LimitClass) -> Result<(), u64> {
        let now = now_s();
        let (name, limit) = match class {
            LimitClass::Read => ("read", 100),
            LimitClass::Write => ("write", 30),
        };
        let key = (client_id.to_string(), name);
        let window = self.windows.entry(key).or_insert(Window {
            start_s: now,
            count: 0,
        });
        if now.saturating_sub(window.start_s) >= 60 {
            window.start_s = now;
            window.count = 0;
        }
        if window.count >= limit {
            return Err((60 - now.saturating_sub(window.start_s)).max(1) as u64);
        }
        window.count += 1;
        Ok(())
    }
}

fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
