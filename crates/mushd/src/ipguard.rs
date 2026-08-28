//! Per-IP abuse protection (design 02 §8.2): a sliding-window auth-failure
//! counter with temporary bans, plus a concurrent-connection cap.
//!
//! Defaults: **5 auth failures in 10 minutes → 15-minute tempban**; at most
//! **8 concurrent connections** per IP. Clock-free: every method takes an
//! explicit `Instant` (the `*_now` wrappers stamp `Instant::now()`), so the
//! window math is deterministic under test.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use dashmap::DashMap;

/// Tunables for [`IpGuard`].
#[derive(Clone, Copy, Debug)]
pub struct IpGuardConfig {
    /// Auth failures within [`IpGuardConfig::window`] that trigger a ban.
    pub max_failures: u32,
    /// Sliding window over which failures are counted.
    pub window: Duration,
    /// How long a triggered ban lasts.
    pub ban: Duration,
    /// Maximum concurrent connections per IP.
    pub max_conns: u32,
}

impl Default for IpGuardConfig {
    fn default() -> Self {
        IpGuardConfig {
            max_failures: 5,
            window: Duration::from_secs(600),
            ban: Duration::from_secs(900),
            max_conns: 8,
        }
    }
}

#[derive(Debug, Default)]
struct IpEntry {
    /// Instants of recent auth failures (pruned to the window).
    failures: Vec<Instant>,
    /// Active tempban, if any.
    banned_until: Option<Instant>,
    /// Live connections from this IP.
    conns: u32,
}

impl IpEntry {
    fn is_idle(&self) -> bool {
        self.failures.is_empty() && self.banned_until.is_none() && self.conns == 0
    }
}

/// The per-IP guard. Shared (`&self` everywhere, DashMap inside); one
/// instance serves every transport.
#[derive(Debug, Default)]
pub struct IpGuard {
    cfg: IpGuardConfig,
    map: DashMap<IpAddr, IpEntry>,
}

impl IpGuard {
    /// Guard with default limits.
    #[must_use]
    pub fn new() -> IpGuard {
        IpGuard::default()
    }

    /// Guard with explicit limits.
    #[must_use]
    pub fn with_config(cfg: IpGuardConfig) -> IpGuard {
        IpGuard {
            cfg,
            map: DashMap::new(),
        }
    }

    /// The active configuration.
    #[must_use]
    pub fn config(&self) -> IpGuardConfig {
        self.cfg
    }

    /// Whether `ip` is currently tempbanned. Expired bans are cleared as a
    /// side effect.
    ///
    /// Deliberately ban-only: the concurrent-connection cap is enforced by
    /// [`IpGuard::conn_opened`] *before* a connection is counted — folding
    /// the cap in here would make an IP's own counted connections block
    /// their later auth checks.
    #[must_use]
    pub fn is_blocked_at(&self, ip: IpAddr, now: Instant) -> bool {
        let Some(mut e) = self.map.get_mut(&ip) else {
            return false;
        };
        if let Some(until) = e.banned_until {
            if now < until {
                return true;
            }
            e.banned_until = None;
        }
        false
    }

    /// [`IpGuard::is_blocked_at`] with `Instant::now()`.
    #[must_use]
    pub fn is_blocked(&self, ip: IpAddr) -> bool {
        self.is_blocked_at(ip, Instant::now())
    }

    /// Record one auth failure. Returns `true` when this failure crossed the
    /// threshold and started a tempban.
    pub fn record_failure_at(&self, ip: IpAddr, now: Instant) -> bool {
        let mut e = self.map.entry(ip).or_default();
        let window = self.cfg.window;
        e.failures
            .retain(|&t| now.saturating_duration_since(t) < window);
        e.failures.push(now);
        if e.failures.len() >= self.cfg.max_failures as usize {
            e.failures.clear();
            e.banned_until = Some(now + self.cfg.ban);
            tracing::warn!(%ip, ban_s = self.cfg.ban.as_secs(), "auth failures exceeded; tempban");
            true
        } else {
            false
        }
    }

    /// [`IpGuard::record_failure_at`] with `Instant::now()`.
    pub fn record_failure(&self, ip: IpAddr) -> bool {
        self.record_failure_at(ip, Instant::now())
    }

    /// Count a new connection from `ip`. Returns `false` (and does not
    /// count it) when the IP is already at its cap.
    pub fn conn_opened(&self, ip: IpAddr) -> bool {
        let mut e = self.map.entry(ip).or_default();
        if e.conns >= self.cfg.max_conns {
            return false;
        }
        e.conns += 1;
        true
    }

    /// Count a connection from `ip` closing. Saturates at zero (a close for
    /// a connection that was never counted — e.g. rejected at the cap — is
    /// harmless). Idle entries are dropped.
    pub fn conn_closed(&self, ip: IpAddr) {
        let remove = match self.map.get_mut(&ip) {
            Some(mut e) => {
                e.conns = e.conns.saturating_sub(1);
                e.is_idle()
            }
            None => false,
        };
        if remove {
            self.map.remove_if(&ip, |_, e| e.is_idle());
        }
    }

    /// Number of IPs currently tracked (diagnostics/tests).
    #[must_use]
    pub fn tracked(&self) -> usize {
        self.map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(last: u8) -> IpAddr {
        IpAddr::from([10, 0, 0, last])
    }

    #[test]
    fn failures_inside_window_trigger_ban_and_ban_expires() {
        let g = IpGuard::new();
        let t0 = Instant::now();
        for i in 0..4 {
            assert!(!g.record_failure_at(ip(1), t0 + Duration::from_secs(i)));
            assert!(!g.is_blocked_at(ip(1), t0 + Duration::from_secs(i)));
        }
        // Fifth failure within 10 minutes: banned.
        assert!(g.record_failure_at(ip(1), t0 + Duration::from_secs(4)));
        assert!(g.is_blocked_at(ip(1), t0 + Duration::from_secs(5)));
        // Still banned just before 15 minutes, clear just after.
        let banned_at = t0 + Duration::from_secs(4);
        assert!(g.is_blocked_at(ip(1), banned_at + Duration::from_secs(899)));
        assert!(!g.is_blocked_at(ip(1), banned_at + Duration::from_secs(901)));
        // Other IPs unaffected.
        assert!(!g.is_blocked_at(ip(2), t0));
    }

    #[test]
    fn old_failures_age_out_of_the_window() {
        let g = IpGuard::new();
        let t0 = Instant::now();
        // 4 failures now; the 5th comes 10+ minutes later — the first four
        // have aged out, so no ban.
        for _ in 0..4 {
            g.record_failure_at(ip(3), t0);
        }
        assert!(!g.record_failure_at(ip(3), t0 + Duration::from_secs(601)));
        // But four more quick ones after it do ban (5 within the window).
        for i in 0..3 {
            assert!(!g.record_failure_at(ip(3), t0 + Duration::from_secs(602 + i)));
        }
        assert!(g.record_failure_at(ip(3), t0 + Duration::from_secs(606)));
    }

    #[test]
    fn concurrent_connection_cap() {
        let g = IpGuard::with_config(IpGuardConfig {
            max_conns: 2,
            ..IpGuardConfig::default()
        });
        let now = Instant::now();
        assert!(g.conn_opened(ip(4)));
        assert!(g.conn_opened(ip(4)));
        // At the cap: further opens refused, but the counted connections'
        // own auth checks are NOT blocked (is_blocked is ban-only).
        assert!(!g.conn_opened(ip(4)));
        assert!(!g.is_blocked_at(ip(4), now));
        // A close frees a slot.
        g.conn_closed(ip(4));
        assert!(g.conn_opened(ip(4)));
        // Closing everything drops the entry.
        g.conn_closed(ip(4));
        g.conn_closed(ip(4));
        g.conn_closed(ip(4)); // extra close saturates, no panic
        assert_eq!(g.tracked(), 0);
    }
}
