//! Input flood control: a plain token bucket.
//!
//! No clock of its own — every operation takes an explicit `now: Instant`,
//! so the math is testable with a fake clock and the manager can share one
//! `Instant::now()` across a batch. [`SessionManager`](crate::SessionManager)
//! keeps one bucket per attached connection (checked via
//! [`check_input`](crate::SessionManager::check_input)); transports can run
//! their own standalone bucket for pre-auth input (welcome screen, login).

use std::time::{Duration, Instant};

/// Result of a flood-control check for one input line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloodVerdict {
    /// Within budget; process the line.
    Ok,
    /// Over budget; the caller should drop the line (and may warn or, on
    /// sustained abuse, disconnect).
    Throttled,
}

/// Token bucket: `rate` tokens per second, capacity `burst`, one token per
/// input line. Starts full so a fresh connection gets its full burst.
#[derive(Clone, Debug)]
pub struct FloodBucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    last: Instant,
}

impl FloodBucket {
    /// New bucket, full at `now`. `rate` is tokens/second (sustained lines
    /// per second), `burst` the bucket capacity (clamped to at least 1).
    pub fn new(rate: f64, burst: f64, now: Instant) -> Self {
        let burst = burst.max(1.0);
        Self {
            rate: rate.max(0.0),
            burst,
            tokens: burst,
            last: now,
        }
    }

    /// Charge one input line at `now`.
    pub fn check(&mut self, now: Instant) -> FloodVerdict {
        let elapsed = now.saturating_duration_since(self.last);
        if elapsed > Duration::ZERO {
            self.tokens = (self.tokens + elapsed.as_secs_f64() * self.rate).min(self.burst);
            self.last = now;
        }
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            FloodVerdict::Ok
        } else {
            FloodVerdict::Throttled
        }
    }

    /// Tokens currently available (diagnostics/tests).
    pub fn tokens(&self) -> f64 {
        self.tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_then_throttle() {
        let t0 = Instant::now();
        let mut b = FloodBucket::new(10.0, 30.0, t0);
        for i in 0..30 {
            assert_eq!(b.check(t0), FloodVerdict::Ok, "burst line {i}");
        }
        assert_eq!(
            b.check(t0),
            FloodVerdict::Throttled,
            "31st line in one instant"
        );
    }

    #[test]
    fn refills_at_rate() {
        let t0 = Instant::now();
        let mut b = FloodBucket::new(10.0, 30.0, t0);
        for _ in 0..30 {
            assert_eq!(b.check(t0), FloodVerdict::Ok);
        }
        // 500 ms later: 5 tokens accrued.
        let t1 = t0 + Duration::from_millis(500);
        for i in 0..5 {
            assert_eq!(b.check(t1), FloodVerdict::Ok, "refilled line {i}");
        }
        assert_eq!(b.check(t1), FloodVerdict::Throttled);
        // A long quiet period refills to burst, no further.
        let t2 = t1 + Duration::from_secs(3600);
        for i in 0..30 {
            assert_eq!(b.check(t2), FloodVerdict::Ok, "after refill line {i}");
        }
        assert_eq!(b.check(t2), FloodVerdict::Throttled);
    }

    #[test]
    fn sustained_rate_passes() {
        let t0 = Instant::now();
        let mut b = FloodBucket::new(10.0, 30.0, t0);
        // Drain the burst, then send exactly at the sustained rate: always Ok.
        for _ in 0..30 {
            assert_eq!(b.check(t0), FloodVerdict::Ok);
        }
        for i in 1..=50u64 {
            let t = t0 + Duration::from_millis(100 * i);
            assert_eq!(b.check(t), FloodVerdict::Ok, "paced line {i}");
        }
    }

    #[test]
    fn clock_going_backward_is_harmless() {
        let t0 = Instant::now();
        let mut b = FloodBucket::new(10.0, 2.0, t0);
        let t1 = t0 + Duration::from_secs(5);
        assert_eq!(b.check(t1), FloodVerdict::Ok);
        assert_eq!(b.check(t1), FloodVerdict::Ok);
        // An earlier `now` neither panics nor grants free tokens.
        assert_eq!(b.check(t0), FloodVerdict::Throttled);
        assert_eq!(b.check(t0), FloodVerdict::Throttled);
    }

    #[test]
    fn burst_clamped_to_one() {
        let t0 = Instant::now();
        let mut b = FloodBucket::new(1.0, 0.0, t0);
        assert_eq!(b.check(t0), FloodVerdict::Ok);
        assert_eq!(b.check(t0), FloodVerdict::Throttled);
    }
}
