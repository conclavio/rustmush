//! Per-IP abuse protection (design 02 §8.2): a sliding-window auth-failure
//! counter with temporary bans, plus a concurrent-connection cap, plus the
//! in-memory mirror of the **persisted sitebans** (M2).
//!
//! Defaults: **5 auth failures in 10 minutes → 15-minute tempban**; at most
//! **8 concurrent connections** per IP. Clock-free for tempbans: every
//! method takes an explicit `Instant` (the `*_now` wrappers stamp
//! `Instant::now()`), so the window math is deterministic under test.
//!
//! Failure and connection accounting is keyed on the **network**, not the
//! address: a client with a routed /64 would otherwise buy a fresh failure
//! budget and a fresh connection cap for every one of 2^64 addresses. The
//! guard folds each address to [`IpGuardConfig::v4_prefix`] (default /32 — no
//! change from per-address for IPv4) and [`IpGuardConfig::v6_prefix`]
//! (default /64, the smallest unit anyone is routed), and an IPv4-mapped v6
//! address folds as the IPv4 address it is.
//!
//! Sitebans are different: they are staff-set, persisted in the store
//! (`ban:{ip}`, native TTL on Redis), and expressed in **unix seconds** so
//! they survive restarts. The guard only mirrors them: the world task loads
//! the persisted set at boot ([`IpGuard::load_sitebans`]) and updates the
//! mirror after every acknowledged `@siteban` store write. Expiry is
//! checked lazily against the wall clock (mirroring the store's TTL).
//!
//! In the **forked** deployment that `set_siteban` call lands in the SERVER
//! process, while the guard that actually admits connections lives in the
//! GATEWAY process — a different `IpGuard` over the same store. The gateway
//! closes that gap by re-seeding its mirror from `ban:*` every
//! [`SITEBAN_REFRESH`], which is why the constant lives here (both halves
//! quote it: the gateway to schedule the refresh, the server to tell staff
//! how long their `@siteban` takes to bite).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};

use dashmap::DashMap;

/// How often a connection-owning gateway re-reads the persisted siteban set
/// into its [`IpGuard`] mirror, and therefore the worst-case latency between
/// an acknowledged `@siteban` store write and the address being refused.
///
/// A whole-mirror reload is one small store read, so this can be short; it is
/// not so short that a fleet of gateways becomes a poll storm.
pub const SITEBAN_REFRESH: Duration = Duration::from_secs(10);

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
    /// Prefix length an IPv4 address is folded to before it is counted.
    /// Default 32: one address, one budget, exactly as before.
    pub v4_prefix: u8,
    /// Prefix length an IPv6 address is folded to before it is counted.
    /// Default 64: the smallest block anyone is routed, so a client cannot
    /// buy a fresh budget by moving one address to the left.
    pub v6_prefix: u8,
}

impl Default for IpGuardConfig {
    fn default() -> Self {
        IpGuardConfig {
            max_failures: 5,
            window: Duration::from_secs(600),
            ban: Duration::from_secs(900),
            max_conns: 8,
            v4_prefix: 32,
            v6_prefix: 64,
        }
    }
}

/// `addr` with everything below `prefix` bits zeroed (prefix clamped to 32).
fn mask_v4(addr: Ipv4Addr, prefix: u8) -> Ipv4Addr {
    let p = u32::from(prefix.min(32));
    let mask = if p == 0 { 0 } else { u32::MAX << (32 - p) };
    Ipv4Addr::from(u32::from(addr) & mask)
}

/// `addr` with everything below `prefix` bits zeroed (prefix clamped to 128).
fn mask_v6(addr: Ipv6Addr, prefix: u8) -> Ipv6Addr {
    let p = u32::from(prefix.min(128));
    let mask = if p == 0 { 0 } else { u128::MAX << (128 - p) };
    Ipv6Addr::from(u128::from(addr) & mask)
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
    /// Failure/connection accounting, keyed on the **folded network** ([`IpGuard::key`]),
    /// not the exact address.
    map: DashMap<IpAddr, IpEntry>,
    /// Persisted-siteban mirror: ip → expiry (unix seconds; `None` =
    /// permanent). Loaded from the store at boot, updated on `@siteban`.
    sitebans: DashMap<IpAddr, Option<i64>>,
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
            sitebans: DashMap::new(),
        }
    }

    /// The active configuration.
    #[must_use]
    pub fn config(&self) -> IpGuardConfig {
        self.cfg
    }

    /// The accounting key for `ip`: the address folded to its configured
    /// network prefix. An IPv4-mapped v6 address (`::ffff:a.b.c.d`, which is
    /// what a dual-stack listener reports for an IPv4 peer) is canonicalised
    /// to its IPv4 form first, so it folds on `v4_prefix` — folding it on
    /// `v6_prefix` would drop every IPv4 client on earth into one bucket.
    ///
    /// Sitebans are NOT keyed on this: a `ban:{ip}` row means that address.
    fn key(&self, ip: IpAddr) -> IpAddr {
        match ip.to_canonical() {
            IpAddr::V4(v4) => IpAddr::V4(mask_v4(v4, self.cfg.v4_prefix)),
            IpAddr::V6(v6) => IpAddr::V6(mask_v6(v6, self.cfg.v6_prefix)),
        }
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
        if self.siteban_active_at(ip, crate::unix_now_s()) {
            return true;
        }
        let Some(mut e) = self.map.get_mut(&self.key(ip)) else {
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

    /// Record one auth failure against `ip`'s network. Returns `true` when
    /// this failure crossed the threshold and started a tempban — which
    /// blocks every address in that network, not just this one.
    pub fn record_failure_at(&self, ip: IpAddr, now: Instant) -> bool {
        let net = self.key(ip);
        let mut e = self.map.entry(net).or_default();
        let window = self.cfg.window;
        e.failures
            .retain(|&t| now.saturating_duration_since(t) < window);
        e.failures.push(now);
        if e.failures.len() >= self.cfg.max_failures as usize {
            e.failures.clear();
            e.banned_until = Some(now + self.cfg.ban);
            tracing::warn!(%ip, %net, ban_s = self.cfg.ban.as_secs(), "auth failures exceeded; tempban");
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
    /// count it) when `ip`'s network is already at its cap — the cap is
    /// shared by every address behind one prefix.
    pub fn conn_opened(&self, ip: IpAddr) -> bool {
        let mut e = self.map.entry(self.key(ip)).or_default();
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
        let net = self.key(ip);
        let remove = match self.map.get_mut(&net) {
            Some(mut e) => {
                e.conns = e.conns.saturating_sub(1);
                e.is_idle()
            }
            None => false,
        };
        if remove {
            self.map.remove_if(&net, |_, e| e.is_idle());
        }
    }

    /// Number of **networks** currently tracked (diagnostics/tests): one
    /// entry per folded key, not per address.
    #[must_use]
    pub fn tracked(&self) -> usize {
        self.map.len()
    }

    // ------------------------------------------------------------------
    // Persisted sitebans (mirror of the store's `ban:*` namespace)
    // ------------------------------------------------------------------

    /// Replace the siteban mirror wholesale (boot-time load from the
    /// store). Expired entries are dropped on the way in.
    pub fn load_sitebans(&self, bans: impl IntoIterator<Item = (IpAddr, Option<i64>)>) {
        let now = crate::unix_now_s();
        self.sitebans.clear();
        for (ip, until) in bans {
            if until.is_none_or(|u| u > now) {
                self.sitebans.insert(ip, until);
            }
        }
    }

    /// Mirror one siteban (called after the store write is acknowledged).
    /// An already-expired `until` clears instead, matching the store's
    /// instant-expiry semantics.
    pub fn set_siteban(&self, ip: IpAddr, until_unix: Option<i64>) {
        if until_unix.is_some_and(|u| u <= crate::unix_now_s()) {
            self.sitebans.remove(&ip);
        } else {
            self.sitebans.insert(ip, until_unix);
        }
    }

    /// Drop one siteban from the mirror; `true` if it was present.
    pub fn clear_siteban(&self, ip: IpAddr) -> bool {
        self.sitebans.remove(&ip).is_some()
    }

    /// Whether a mirrored siteban is active at `now_unix` (unix seconds).
    /// Expired entries are pruned as a side effect, like the store's TTL.
    #[must_use]
    pub fn siteban_active_at(&self, ip: IpAddr, now_unix: i64) -> bool {
        let Some(entry) = self.sitebans.get(&ip) else {
            return false;
        };
        match *entry {
            None => true,
            Some(until) if until > now_unix => true,
            Some(_) => {
                drop(entry);
                self.sitebans.remove(&ip);
                false
            }
        }
    }

    /// Number of mirrored sitebans (diagnostics/tests).
    #[must_use]
    pub fn siteban_count(&self) -> usize {
        self.sitebans.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(last: u8) -> IpAddr {
        IpAddr::from([10, 0, 0, last])
    }

    /// `2001:db8:0:<net>::<host>` — `net` picks the /64, `host` the address
    /// inside it.
    fn ip6(net: u16, host: u16) -> IpAddr {
        IpAddr::from([0x2001, 0x0db8, 0, net, 0, 0, 0, host])
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
    fn sitebans_block_expire_and_clear() {
        let g = IpGuard::new();
        let now = crate::unix_now_s();
        g.load_sitebans([
            (ip(10), None),             // permanent
            (ip(11), Some(now + 3600)), // active
            (ip(12), Some(now - 10)),   // already expired: dropped on load
        ]);
        assert_eq!(g.siteban_count(), 2);
        assert!(g.is_blocked(ip(10)));
        assert!(g.is_blocked(ip(11)));
        assert!(!g.is_blocked(ip(12)));

        // Expiry is honored lazily (and pruned).
        assert!(!g.siteban_active_at(ip(11), now + 7200));
        assert_eq!(g.siteban_count(), 1);

        // set/clear mirror store writes.
        g.set_siteban(ip(13), Some(now + 60));
        assert!(g.is_blocked(ip(13)));
        assert!(g.clear_siteban(ip(13)));
        assert!(!g.clear_siteban(ip(13)));
        assert!(!g.is_blocked(ip(13)));
        // Setting an already-expired ban clears instead.
        g.set_siteban(ip(10), Some(now - 5));
        assert!(!g.is_blocked(ip(10)));
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
    #[test]
    fn v6_failures_fold_onto_the_64() {
        let g = IpGuard::new();
        let t0 = Instant::now();
        // Five failures from five DIFFERENT addresses inside one /64. Keyed
        // on the exact address each would buy a fresh 5-failure budget and
        // none of them would ever ban.
        for i in 0..4 {
            assert!(!g.record_failure_at(ip6(1, 100 + i), t0 + Duration::from_secs(u64::from(i))));
        }
        assert!(g.record_failure_at(ip6(1, 104), t0 + Duration::from_secs(4)));
        // A sixth address in that /64, never seen before, is already blocked.
        assert!(g.is_blocked_at(ip6(1, 9999), t0 + Duration::from_secs(5)));
        // The whole /64 is one entry.
        assert_eq!(g.tracked(), 1);
        // A different /64 is untouched, and its own budget is intact.
        assert!(!g.is_blocked_at(ip6(2, 100), t0 + Duration::from_secs(5)));
        assert!(!g.record_failure_at(ip6(2, 100), t0 + Duration::from_secs(5)));
        assert_eq!(g.tracked(), 2);
    }

    #[test]
    fn v6_connection_cap_is_shared_across_the_64() {
        let g = IpGuard::new(); // max_conns 8
        // Nine distinct addresses in one /64: the ninth is refused.
        for i in 0..8 {
            assert!(g.conn_opened(ip6(1, 100 + i)));
        }
        assert!(!g.conn_opened(ip6(1, 108)));
        // Closing a DIFFERENT address in the same /64 decrements the shared
        // count, freeing the slot.
        g.conn_closed(ip6(1, 100));
        assert!(g.conn_opened(ip6(1, 108)));
        // A different /64 has its own cap.
        assert!(g.conn_opened(ip6(2, 100)));
        assert_eq!(g.tracked(), 2);
        // Draining the folded entry drops it.
        for i in 0..8 {
            g.conn_closed(ip6(1, 200 + i));
        }
        g.conn_closed(ip6(2, 7));
        assert_eq!(g.tracked(), 0);
    }

    #[test]
    fn v4_stays_per_address_and_mapped_v6_folds_as_v4() {
        let g = IpGuard::new(); // v4_prefix 32
        let t0 = Instant::now();
        // Four failures each from two addresses in the same /24: neither bans.
        for i in 0..4u64 {
            assert!(!g.record_failure_at(ip(1), t0 + Duration::from_secs(i)));
            assert!(!g.record_failure_at(ip(2), t0 + Duration::from_secs(i)));
        }
        assert!(g.record_failure_at(ip(1), t0 + Duration::from_secs(4)));
        assert!(!g.is_blocked_at(ip(2), t0 + Duration::from_secs(4)));
        // `::ffff:10.0.0.1` is 10.0.0.1, not a /64 that would swallow every
        // IPv4 client a dual-stack listener reports.
        let mapped = IpAddr::from([0, 0, 0, 0, 0, 0xffff, 0x0a00, 0x0001]);
        assert!(g.is_blocked_at(mapped, t0 + Duration::from_secs(4)));
        let mapped2 = IpAddr::from([0, 0, 0, 0, 0, 0xffff, 0x0a00, 0x0002]);
        assert!(!g.is_blocked_at(mapped2, t0 + Duration::from_secs(4)));
    }

    #[test]
    fn siteban_stays_keyed_on_the_exact_address() {
        let g = IpGuard::new();
        g.set_siteban(ip6(1, 5), None);
        assert!(g.is_blocked(ip6(1, 5)));
        // A neighbour in the same /64 is not banned: `ban:{ip}` means that
        // address, and a /64 ban would blanket a household.
        assert!(!g.is_blocked(ip6(1, 6)));
        assert!(g.clear_siteban(ip6(1, 5)));
        assert!(!g.is_blocked(ip6(1, 5)));
    }

    #[test]
    fn prefix_zero_and_full_width_do_not_overflow_the_shift() {
        assert_eq!(
            mask_v4(Ipv4Addr::new(10, 1, 2, 3), 0),
            Ipv4Addr::UNSPECIFIED
        );
        assert_eq!(
            mask_v4(Ipv4Addr::new(10, 1, 2, 3), 32),
            Ipv4Addr::new(10, 1, 2, 3)
        );
        assert_eq!(
            mask_v4(Ipv4Addr::new(10, 1, 2, 3), 200),
            Ipv4Addr::new(10, 1, 2, 3)
        );
        let a: Ipv6Addr = "2001:db8::1".parse().unwrap_or(Ipv6Addr::UNSPECIFIED);
        assert_eq!(mask_v6(a, 0), Ipv6Addr::UNSPECIFIED);
        assert_eq!(mask_v6(a, 128), a);
        assert_eq!(mask_v6(a, 200), a);
        assert_eq!(
            mask_v6(a, 64),
            "2001:db8::".parse().unwrap_or(Ipv6Addr::UNSPECIFIED)
        );
    }
}
