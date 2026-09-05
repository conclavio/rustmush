//! The composing-hint rate limit (interface plan I4, D-I-11): at most one
//! `active: true` per connection per [`COMPOSING_MIN_INTERVAL_MS`]; an
//! `active: false` always passes and clears the connection's slot, so the
//! table only ever holds connections mid-draft.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The shortest gap between two `active: true` hints from one connection.
pub const COMPOSING_MIN_INTERVAL_MS: u64 = 2000;

/// A hard cap on tracked connections; past it the table is cleared rather
/// than grown (a connection that never sends `false` costs one entry).
const MAX_TRACKED: usize = 10_000;

/// Per-connection last-`true` times.
#[derive(Debug, Default)]
pub struct ComposingLimiter {
    last_start: HashMap<u64, Instant>,
}

impl ComposingLimiter {
    /// Whether a hint from `conn` may go through at `now`.
    pub fn allow(&mut self, conn: u64, active: bool, now: Instant) -> bool {
        if !active {
            self.last_start.remove(&conn);
            return true;
        }
        if let Some(last) = self.last_start.get(&conn)
            && now.duration_since(*last) < Duration::from_millis(COMPOSING_MIN_INTERVAL_MS)
        {
            return false;
        }
        if self.last_start.len() >= MAX_TRACKED {
            self.last_start.clear();
        }
        self.last_start.insert(conn, now);
        true
    }

    /// Drop a connection's slot (it went away).
    pub fn forget(&mut self, conn: u64) {
        self.last_start.remove(&conn);
    }

    /// Connections currently mid-draft.
    #[must_use]
    pub fn tracked(&self) -> usize {
        self.last_start.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_start_per_interval_and_a_stop_always_passes() {
        let mut lim = ComposingLimiter::default();
        let t0 = Instant::now();
        assert!(lim.allow(1, true, t0));
        assert!(!lim.allow(1, true, t0 + Duration::from_millis(500)));
        assert!(!lim.allow(1, true, t0 + Duration::from_millis(1999)));
        assert!(
            lim.allow(1, false, t0 + Duration::from_millis(600)),
            "a stop always passes"
        );
        assert!(
            lim.allow(1, true, t0 + Duration::from_millis(700)),
            "a stop resets the slot"
        );
        assert!(lim.allow(2, true, t0), "connections are independent");
        assert!(lim.allow(1, true, t0 + Duration::from_millis(2800)));
        assert_eq!(lim.tracked(), 2);
        lim.forget(1);
        assert_eq!(lim.tracked(), 1);
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
