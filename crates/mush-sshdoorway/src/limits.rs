//! The doorway's own abuse gates (doc 29 §8, 29a A-4/A-5).
//!
//! These run at TCP accept, before any SSH work, and are separate from each
//! instance's gates (which see the player's real address through `Hello`).
//!
//! - **Key.** IPv4 is limited per address; IPv6 per **/64**, because one host
//!   commonly holds a whole /64 and can rotate through it freely. An
//!   IPv4-mapped IPv6 address is treated as the IPv4 address.
//! - **Rate** (new connections per key per minute) and the **global** cap are
//!   answered by dropping the TCP connection: a flood gets no handshake.
//! - **Concurrency** per key is answered in SSH (a banner, then refusal), so
//!   a household behind one NAT is told why.
//!
//! The rate table is bounded: when it grows past [`MAX_TRACKED`] it is swept
//! of stale windows, and if it is still full one arbitrary entry is evicted
//! to make room. It never fails closed: 65,536 keys is exactly the /64s of
//! one IPv6 /48, so refusing new keys on a full table would let one customer
//! allocation lock every new player out (adversarial review, round 2). The
//! global and per-/48 caps are checked before a new key is inserted, so
//! connections that would be dropped anyway do not fill the table (F-5).

use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The rate window.
pub const WINDOW: Duration = Duration::from_secs(60);

/// Most keys the rate table tracks at once.
pub const MAX_TRACKED: usize = 65_536;

/// How often a full rate table may be swept (the sweep is O(table) under the
/// lock, so it must not run on every accept — adversarial review).
pub const SWEEP_EVERY: Duration = Duration::from_secs(1);

/// Most simultaneous over-cap (`counted == false`) connections admitted per
/// key solely to deliver the "too many connections" SSH banner. Further
/// concurrent connections from the same key are dropped at TCP accept so one
/// key cannot hold `rate` global slots in banner/half-close state (F-1).
pub const MAX_UNCOUNTED_PER_KEY: u32 = 2;

/// The top 48 bits of an IPv6 address: one customer's typical allocation.
/// IPv6 connections are also capped per /48 (in aggregate over its /64s), or
/// one /48's 65,536 /64s would each get a full per-key budget.
fn v6_48(ip: IpAddr) -> Option<u64> {
    match ip {
        IpAddr::V6(v6) if v6.to_ipv4_mapped().is_none() => Some((u128::from(v6) >> 80) as u64),
        _ => None,
    }
}

/// What the limits key an address on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IpKey {
    /// An IPv4 address.
    V4(u32),
    /// The top 64 bits of an IPv6 address.
    V6(u64),
}

/// The key for one address.
#[must_use]
pub fn ip_key(ip: IpAddr) -> IpKey {
    match ip {
        IpAddr::V4(v4) => IpKey::V4(u32::from(v4)),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpKey::V4(u32::from(v4)),
            None => IpKey::V6((u128::from(v6) >> 64) as u64),
        },
    }
}

/// The numbers the gates apply (from `[limits]`, reloadable).
#[derive(Clone, Copy, Debug)]
pub struct Caps {
    /// Concurrent connections per key.
    pub per_key: u32,
    /// New connections per key per [`WINDOW`].
    pub rate: u32,
    /// Concurrent connections in total.
    pub global: u32,
    /// Concurrent connections per IPv6 /48, across its /64s.
    pub per_v6_48: u32,
}

/// Why a connection was dropped at accept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DropReason {
    /// Over the per-key rate (or the rate table is full).
    Rate,
    /// Over the global cap.
    Global,
    /// Over the per-/48 cap.
    Prefix48,
    /// Over the per-key concurrency cap and [`MAX_UNCOUNTED_PER_KEY`] banner
    /// slots are already in flight for this key.
    PerIp,
}

/// The outcome of [`Limits::admit`].
#[derive(Debug)]
pub enum Admission {
    /// Run SSH. The slot is released when dropped.
    Admit(Slot),
    /// Close the TCP connection now.
    Drop(DropReason),
}

/// The shared gate state.
#[derive(Debug, Default)]
pub struct Limits {
    inner: Mutex<Inner>,
    global: AtomicUsize,
}

#[derive(Debug, Default)]
struct Inner {
    concurrent: HashMap<IpKey, u32>,
    uncounted: HashMap<IpKey, u32>,
    concurrent48: HashMap<u64, u32>,
    last_sweep: Option<Instant>,
    rate: HashMap<IpKey, (Instant, u32)>,
    counters: BTreeMap<&'static str, u64>,
}

impl Limits {
    /// A fresh gate set.
    #[must_use]
    pub fn new() -> Arc<Limits> {
        Arc::new(Limits::default())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Gate one accepted connection.
    pub fn admit(self: &Arc<Self>, ip: IpAddr, caps: Caps, now: Instant) -> Admission {
        let key = ip_key(ip);
        let prefix = v6_48(ip);
        let mut inner = self.lock();
        // If this key is already tracked in the rate table, enforce its rate
        // window first so a known flooder is dropped as `Rate`.
        if let Some(entry) = inner.rate.get_mut(&key) {
            if now.saturating_duration_since(entry.0) >= WINDOW {
                *entry = (now, 0);
            }
            entry.1 = entry.1.saturating_add(1);
            if entry.1 > caps.rate {
                *inner.counters.entry("rate").or_default() += 1;
                return Admission::Drop(DropReason::Rate);
            }
        } else {
            // Before inserting a *new* key into the rate table, check the
            // global and /48 concurrency caps so rejected floods cannot fill
            // `MAX_TRACKED` and lock out new IPs (adversarial review F-5).
            if self.global.load(Ordering::SeqCst) >= caps.global as usize {
                *inner.counters.entry("global").or_default() += 1;
                return Admission::Drop(DropReason::Global);
            }
            if let Some(p) = prefix
                && inner.concurrent48.get(&p).copied().unwrap_or(0) >= caps.per_v6_48
            {
                *inner.counters.entry("per-48").or_default() += 1;
                return Admission::Drop(DropReason::Prefix48);
            }
            if caps.rate == 0 {
                *inner.counters.entry("rate").or_default() += 1;
                return Admission::Drop(DropReason::Rate);
            }
            if inner.rate.len() >= MAX_TRACKED
                && inner
                    .last_sweep
                    .is_none_or(|t| now.saturating_duration_since(t) >= SWEEP_EVERY)
            {
                inner.last_sweep = Some(now);
                inner
                    .rate
                    .retain(|_, (start, _)| now.saturating_duration_since(*start) < WINDOW);
            }
            if inner.rate.len() >= MAX_TRACKED {
                // Still full of live windows: make room rather than refuse.
                // A flood that fills the table owns almost every entry, so
                // the victim almost always is its own; a legitimate key that
                // is evicted merely starts a fresh window.
                let victim = inner.rate.keys().next().copied();
                if let Some(v) = victim {
                    inner.rate.remove(&v);
                }
                *inner.counters.entry("rate-evict").or_default() += 1;
            }
            inner.rate.insert(key, (now, 1));
        }
        if self.global.load(Ordering::SeqCst) >= caps.global as usize {
            *inner.counters.entry("global").or_default() += 1;
            return Admission::Drop(DropReason::Global);
        }
        if let Some(p) = prefix
            && inner.concurrent48.get(&p).copied().unwrap_or(0) >= caps.per_v6_48
        {
            *inner.counters.entry("per-48").or_default() += 1;
            return Admission::Drop(DropReason::Prefix48);
        }
        let count = inner.concurrent.get(&key).copied().unwrap_or(0);
        let counted = count < caps.per_key;
        if counted {
            *inner.concurrent.entry(key).or_default() += 1;
        } else {
            let uncounted = inner.uncounted.get(&key).copied().unwrap_or(0);
            if uncounted >= MAX_UNCOUNTED_PER_KEY {
                *inner.counters.entry("per-ip").or_default() += 1;
                return Admission::Drop(DropReason::PerIp);
            }
            *inner.uncounted.entry(key).or_default() += 1;
            *inner.counters.entry("per-ip").or_default() += 1;
        }
        if let Some(p) = prefix {
            *inner.concurrent48.entry(p).or_default() += 1;
        }
        self.global.fetch_add(1, Ordering::SeqCst);
        Admission::Admit(Slot {
            limits: Arc::clone(self),
            key,
            prefix,
            counted,
        })
    }

    /// Count one refusal that happened after accept (`invalid-user`,
    /// `unknown-method`, …) for the per-minute summary.
    pub fn count(&self, reason: &'static str) {
        *self.lock().counters.entry(reason).or_default() += 1;
    }

    /// Take the counters (the summary line resets them).
    pub fn take_counters(&self) -> BTreeMap<&'static str, u64> {
        std::mem::take(&mut self.lock().counters)
    }

    /// Connections currently admitted.
    #[must_use]
    pub fn live(&self) -> usize {
        self.global.load(Ordering::SeqCst)
    }

    /// Concurrent connections for one address's key (tests, diagnostics).
    #[must_use]
    pub fn concurrent(&self, ip: IpAddr) -> u32 {
        self.lock()
            .concurrent
            .get(&ip_key(ip))
            .copied()
            .unwrap_or(0)
    }

    /// Keys in the rate table (tests).
    #[must_use]
    pub fn tracked(&self) -> usize {
        self.lock().rate.len()
    }
}

/// One admitted connection's claim. `counted == false` means the key was at
/// its concurrency cap: the connection is told and refused in SSH.
#[derive(Debug)]
pub struct Slot {
    limits: Arc<Limits>,
    key: IpKey,
    prefix: Option<u64>,
    counted: bool,
}

impl Slot {
    /// Whether this connection holds a per-key slot.
    #[must_use]
    pub fn counted(&self) -> bool {
        self.counted
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.limits.global.fetch_sub(1, Ordering::SeqCst);
        let mut inner = self.limits.lock();
        if let Some(p) = self.prefix
            && let Some(n) = inner.concurrent48.get_mut(&p)
        {
            *n = n.saturating_sub(1);
            if *n == 0 {
                inner.concurrent48.remove(&p);
            }
        }
        if self.counted {
            if let Some(n) = inner.concurrent.get_mut(&self.key) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    inner.concurrent.remove(&self.key);
                }
            }
        } else if let Some(n) = inner.uncounted.get_mut(&self.key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                inner.uncounted.remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPS: Caps = Caps {
        per_key: 2,
        rate: 5,
        global: 10,
        per_v6_48: 3,
    };

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn slot(a: Admission) -> Slot {
        match a {
            Admission::Admit(s) => s,
            Admission::Drop(r) => panic!("dropped: {r:?}"),
        }
    }

    #[test]
    fn ipv6_is_keyed_per_64_and_mapped_v4_as_v4() {
        assert_eq!(
            ip_key(ip("2001:db8:1:2::1")),
            ip_key(ip("2001:db8:1:2:ffff::9"))
        );
        assert_ne!(ip_key(ip("2001:db8:1:2::1")), ip_key(ip("2001:db8:1:3::1")));
        assert_eq!(
            ip_key(ip("::ffff:198.51.100.7")),
            ip_key(ip("198.51.100.7"))
        );
    }

    #[test]
    fn concurrency_is_counted_and_released() {
        let l = Limits::new();
        let now = Instant::now();
        let a = slot(l.admit(ip("198.51.100.7"), CAPS, now));
        let b = slot(l.admit(ip("198.51.100.7"), CAPS, now));
        let c = slot(l.admit(ip("198.51.100.7"), CAPS, now));
        let d = slot(l.admit(ip("198.51.100.7"), CAPS, now));
        assert!(a.counted() && b.counted() && !c.counted() && !d.counted());
        // Beyond MAX_UNCOUNTED_PER_KEY (2) simultaneous banner slots, further
        // connections from the same key are dropped at TCP accept without
        // claiming global slots (F-1).
        assert!(matches!(
            l.admit(ip("198.51.100.7"), CAPS, now),
            Admission::Drop(DropReason::PerIp)
        ));
        assert_eq!(l.concurrent(ip("198.51.100.7")), 2);
        assert_eq!(l.live(), 4);
        drop((a, b, c, d));
        assert_eq!(l.concurrent(ip("198.51.100.7")), 0);
        assert_eq!(l.live(), 0);
    }

    #[test]
    fn a_v6_48_is_capped_across_its_64s_and_released() {
        let l = Limits::new();
        let now = Instant::now();
        let held: Vec<Slot> = (0..3)
            .map(|i| slot(l.admit(ip(&format!("2001:db8:0:{i:x}::1")), CAPS, now)))
            .collect();
        assert!(matches!(
            l.admit(ip("2001:db8:0:ff::1"), CAPS, now),
            Admission::Drop(DropReason::Prefix48)
        ));
        // Another /48 is unaffected, and dropping one frees a place.
        let _other = slot(l.admit(ip("2001:db8:1::1"), CAPS, now));
        drop(held);
        let _again = slot(l.admit(ip("2001:db8:0:ff::1"), CAPS, now));
    }

    #[test]
    fn a_rotating_v6_host_shares_one_budget() {
        let l = Limits::new();
        let now = Instant::now();
        let mut held = Vec::new();
        let caps = Caps {
            per_v6_48: 100,
            ..CAPS
        };
        for i in 0..5 {
            let s = slot(l.admit(ip(&format!("2001:db8::{i:x}")), caps, now));
            assert_eq!(s.counted(), i < 2);
            if s.counted() {
                held.push(s);
            }
        }
        assert!(matches!(
            l.admit(ip("2001:db8::ff"), caps, now),
            Admission::Drop(DropReason::Rate)
        ));
        // Another /64 is unaffected.
        let _other = slot(l.admit(ip("2001:db8:0:1::1"), caps, now));
    }

    #[test]
    fn the_rate_window_resets_and_the_global_cap_holds() {
        let l = Limits::new();
        let t0 = Instant::now();
        for _ in 0..5 {
            drop(slot(l.admit(ip("203.0.113.1"), CAPS, t0)));
        }
        assert!(matches!(
            l.admit(ip("203.0.113.1"), CAPS, t0),
            Admission::Drop(DropReason::Rate)
        ));
        drop(slot(l.admit(ip("203.0.113.1"), CAPS, t0 + WINDOW)));

        let mut held = Vec::new();
        for i in 0..10 {
            held.push(slot(l.admit(ip(&format!("192.0.2.{i}")), CAPS, t0)));
        }
        assert!(matches!(
            l.admit(ip("192.0.2.200"), CAPS, t0),
            Admission::Drop(DropReason::Global)
        ));
        held.clear();
        drop(slot(l.admit(ip("192.0.2.200"), CAPS, t0)));
        let counters = l.take_counters();
        assert_eq!(counters.get("rate"), Some(&1));
        assert_eq!(counters.get("global"), Some(&1));
        assert!(l.take_counters().is_empty());
    }

    #[test]
    fn the_rate_table_is_bounded_and_evicts_rather_than_refuse() {
        let l = Limits::new();
        let caps = Caps {
            per_key: 1,
            rate: 1,
            global: u32::MAX,
            per_v6_48: u32::MAX,
        };
        let t0 = Instant::now();
        for i in 0..(MAX_TRACKED as u32) {
            let addr = IpAddr::V4(std::net::Ipv4Addr::from(0x0a00_0000 + i));
            drop(slot(l.admit(addr, caps, t0)));
        }
        assert_eq!(l.tracked(), MAX_TRACKED);
        // Full of live windows: a new key is still admitted (one entry is
        // evicted for it), the table stays bounded, and the new key is
        // tracked — its second connection in the window is over its rate.
        let extra = IpAddr::V4(std::net::Ipv4Addr::from(0x0a00_0000 + MAX_TRACKED as u32));
        drop(slot(l.admit(extra, caps, t0)));
        assert_eq!(l.tracked(), MAX_TRACKED);
        assert!(matches!(
            l.admit(extra, caps, t0),
            Admission::Drop(DropReason::Rate)
        ));
        // Later, the stale windows are swept and new keys are tracked again.
        drop(slot(l.admit(ip("198.18.0.1"), caps, t0 + WINDOW)));
        assert!(l.tracked() <= MAX_TRACKED);
        assert!(matches!(
            l.admit(ip("198.18.0.1"), caps, t0 + WINDOW),
            Admission::Drop(DropReason::Rate)
        ));
        assert_eq!(l.take_counters().get("rate-evict"), Some(&1));
    }

    #[test]
    fn one_v6_48_cannot_fill_the_rate_table_against_everyone_else() {
        // One /48 holds exactly MAX_TRACKED /64s. Connecting once from each
        // and closing at once (so the /48 concurrency cap never trips) fills
        // the table; a player from anywhere else must still get in.
        let l = Limits::new();
        let caps = Caps {
            per_key: 8,
            rate: 20,
            global: 2048,
            per_v6_48: 64,
        };
        let t0 = Instant::now();
        for i in 0..MAX_TRACKED as u128 {
            let v6 = std::net::Ipv6Addr::from((0x2001_0db8_0001u128 << 80) | (i << 64) | 1);
            drop(slot(l.admit(IpAddr::V6(v6), caps, t0)));
        }
        assert_eq!(l.tracked(), MAX_TRACKED);
        drop(slot(l.admit(ip("198.51.100.7"), caps, t0)));
        drop(slot(l.admit(ip("2001:db8:2::1"), caps, t0)));
        assert_eq!(l.tracked(), MAX_TRACKED);
    }

    #[test]
    fn global_and_prefix48_drops_do_not_pollute_the_rate_table() {
        let l = Limits::new();
        let t0 = Instant::now();
        let caps = Caps {
            per_key: 2,
            rate: 5,
            global: 1,
            per_v6_48: 1,
        };
        let held = slot(l.admit(ip("2001:db8:0:1::1"), caps, t0));
        assert_eq!(l.tracked(), 1);
        // While global (and /48) is full, new IPs are dropped before inserting
        // into the rate table.
        assert!(matches!(
            l.admit(ip("198.51.100.99"), caps, t0),
            Admission::Drop(DropReason::Global)
        ));
        assert_eq!(l.tracked(), 1);
        drop(held);
    }
}
