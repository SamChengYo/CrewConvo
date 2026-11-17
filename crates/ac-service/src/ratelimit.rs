use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use uuid::Uuid;

/// Sliding-window limiter keyed by (member, action).
#[derive(Default)]
pub struct RateLimiter {
    hits: Mutex<HashMap<(Uuid, &'static str), VecDeque<Instant>>>,
}

impl RateLimiter {
    /// Records a hit and returns true if it is within `max` per `window`.
    pub fn check(&self, who: Uuid, action: &'static str, max: usize, window: Duration) -> bool {
        let now = Instant::now();
        let mut hits = self.hits.lock().unwrap();
        let q = hits.entry((who, action)).or_default();
        while let Some(front) = q.front() {
            if now.duration_since(*front) > window {
                q.pop_front();
            } else {
                break;
            }
        }
        if q.len() >= max {
            return false;
        }
        q.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_within_window() {
        let r = RateLimiter::default();
        let id = Uuid::now_v7();
        for _ in 0..3 {
            assert!(r.check(id, "x", 3, Duration::from_secs(60)));
        }
        assert!(!r.check(id, "x", 3, Duration::from_secs(60)));
        assert!(r.check(id, "y", 3, Duration::from_secs(60)));
    }
}
