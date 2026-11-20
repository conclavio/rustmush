//! The composing-hint rate limit (interface plan I4, D-I-11, HIGH-1):
//! tracks per-connection active state and last transition timestamp, ignores
//! redundant identical transitions (`false` when inactive, `true` when already
//! active), and enforces [`COMPOSING_MIN_INTERVAL_MS`] cooldown on BOTH `true`
//! and `false` transitions without clearing the cooldown on `active: false`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The shortest gap between two composing state transitions from one connection.
pub const COMPOSING_MIN_INTERVAL_MS: u64 = 2000;

/// A hard cap on tracked connections; past it the table is cleared rather
/// than grown (a connection that never disconnects costs one entry).
const MAX_TRACKED: usize = 10_000;

#[derive(Clone, Copy, Debug)]
struct ConnComposingState {
    active: bool,
    last_transition: Instant,
}

/// Per-connection composing state and transition cooldowns.
#[derive(Debug, Default)]
pub struct ComposingLimiter {
    states: HashMap<u64, ConnComposingState>,
}

impl ComposingLimiter {
    /// Whether a composing hint from `conn` may go through at `now`.
    pub fn allow(&mut self, conn: u64, active: bool, now: Instant) -> bool {
        let min_interval = Duration::from_millis(COMPOSING_MIN_INTERVAL_MS);
        if let Some(state) = self.states.get_mut(&conn) {
            if state.active == active {
                return false;
            }
            if now.saturating_duration_since(state.last_transition) < min_interval {
                return false;
            }
            state.active = active;
            state.last_transition = now;
            return true;
        }
        // An untracked connection is inactive; ignore redundant `false` spam.
        if !active {
            return false;
        }
        if self.states.len() >= MAX_TRACKED {
            self.states.clear();
        }
        self.states.insert(
            conn,
            ConnComposingState {
                active: true,
                last_transition: now,
            },
        );
        true
    }

    /// Drop a connection's slot (it went away).
    pub fn forget(&mut self, conn: u64) {
        self.states.remove(&conn);
    }

    /// Connections currently tracked by the limiter.
    #[must_use]
    pub fn tracked(&self) -> usize {
        self.states.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_cooldown_on_both_transitions_and_ignores_redundant_hints() {
        let mut lim = ComposingLimiter::default();
        let t0 = Instant::now();

        // Initial redundant `false` when inactive is ignored and does not allocate a slot.
        assert!(!lim.allow(1, false, t0));
        assert_eq!(lim.tracked(), 0);

        // First `true` passes; redundant `true` is always ignored even after cooldown.
        assert!(lim.allow(1, true, t0));
        assert!(!lim.allow(1, true, t0 + Duration::from_millis(500)));
        assert!(!lim.allow(1, true, t0 + Duration::from_millis(5_000)));

        // `false` inside the cooldown is rejected (cannot bypass via rapid toggle).
        assert!(!lim.allow(1, false, t0 + Duration::from_millis(600)));
        assert!(!lim.allow(1, false, t0 + Duration::from_millis(1999)));

        // After the cooldown, `false` passes and starts a fresh cooldown (does NOT clear slot).
        let t1 = t0 + Duration::from_millis(COMPOSING_MIN_INTERVAL_MS);
        assert!(lim.allow(1, false, t1));
        assert_eq!(lim.tracked(), 1);
        assert!(!lim.allow(1, false, t1 + Duration::from_millis(5_000)));
        assert!(
            !lim.allow(1, true, t1 + Duration::from_millis(100)),
            "stopping must not reset the cooldown slot"
        );
        assert!(
            !lim.allow(1, true, t1 + Duration::from_millis(1999)),
            "restart still blocked until full cooldown elapses"
        );

        // Connections are independent.
        assert!(lim.allow(2, true, t0), "connections are independent");
        assert!(lim.allow(
            1,
            true,
            t1 + Duration::from_millis(COMPOSING_MIN_INTERVAL_MS)
        ));
        assert_eq!(lim.tracked(), 2);
        lim.forget(1);
        assert_eq!(lim.tracked(), 1);
    }

    #[test]
    fn alternating_true_false_and_repeated_false_spam_are_rate_limited() {
        let mut lim = ComposingLimiter::default();
        let t0 = Instant::now();

        // Repeated `false` spam from an inactive connection never passes.
        for i in 0..100 {
            assert!(!lim.allow(7, false, t0 + Duration::from_millis(i * 10)));
        }

        // Rapid alternating `true`/`false` inside one cooldown window only allows
        // the initial `true`.
        let mut allowed = 0usize;
        for i in 0..100 {
            let active = i % 2 == 0;
            if lim.allow(7, active, t0 + Duration::from_millis(i * 10)) {
                allowed += 1;
            }
        }
        assert_eq!(
            allowed, 1,
            "alternating true/false spam must not bypass cooldown"
        );
    }

    #[test]
    fn the_table_is_bounded() {
        let mut lim = ComposingLimiter::default();
        let t0 = Instant::now();
        for conn in 0..(MAX_TRACKED as u64 + 10) {
            assert!(lim.allow(conn, true, t0));
        }
        assert!(lim.tracked() <= MAX_TRACKED);
    }
}
