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
//! of stale windows, and if it is still full the new key is admitted
//! untracked (failing open on rate, while the global cap still holds).

use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The rate window.
pub const WINDOW: Duration = Duration::from_secs(60);

/// Most keys the rate table tracks at once.
pub const MAX_TRACKED: usize = 65_536;

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
}

/// Why a connection was dropped at accept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DropReason {
    /// Over the per-key rate.
    Rate,
    /// Over the global cap.
    Global,
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
        let mut inner = self.lock();
        // Rate first: a flood is refused before it costs a slot.
        if inner.rate.len() >= MAX_TRACKED && !inner.rate.contains_key(&key) {
            inner
                .rate
                .retain(|_, (start, _)| now.saturating_duration_since(*start) < WINDOW);
        }
        let tracked = inner.rate.len() < MAX_TRACKED || inner.rate.contains_key(&key);
        if tracked {
            let entry = inner.rate.entry(key).or_insert((now, 0));
            if now.saturating_duration_since(entry.0) >= WINDOW {
                *entry = (now, 0);
            }
            entry.1 = entry.1.saturating_add(1);
            if entry.1 > caps.rate {
                *inner.counters.entry("rate").or_default() += 1;
                return Admission::Drop(DropReason::Rate);
            }
        }
        if self.global.load(Ordering::SeqCst) >= caps.global as usize {
            *inner.counters.entry("global").or_default() += 1;
            return Admission::Drop(DropReason::Global);
        }
        self.global.fetch_add(1, Ordering::SeqCst);
        let count = inner.concurrent.entry(key).or_default();
        let counted = *count < caps.per_key;
        if counted {
            *count += 1;
        } else {
            *inner.counters.entry("per-ip").or_default() += 1;
        }
        Admission::Admit(Slot {
            limits: Arc::clone(self),
            key,
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
        if self.counted {
            let mut inner = self.limits.lock();
            if let Some(n) = inner.concurrent.get_mut(&self.key) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    inner.concurrent.remove(&self.key);
                }
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
        assert!(a.counted() && b.counted() && !c.counted());
        assert_eq!(l.concurrent(ip("198.51.100.7")), 2);
        assert_eq!(l.live(), 3);
        drop((a, b, c));
        assert_eq!(l.concurrent(ip("198.51.100.7")), 0);
        assert_eq!(l.live(), 0);
    }

    #[test]
    fn a_rotating_v6_host_shares_one_budget() {
        let l = Limits::new();
        let now = Instant::now();
        let mut held = Vec::new();
        for i in 0..5 {
            held.push(slot(l.admit(ip(&format!("2001:db8::{i:x}")), CAPS, now)));
        }
        assert!(matches!(
            l.admit(ip("2001:db8::ff"), CAPS, now),
            Admission::Drop(DropReason::Rate)
        ));
        // Another /64 is unaffected.
        let _other = slot(l.admit(ip("2001:db8:0:1::1"), CAPS, now));
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
    fn the_rate_table_is_bounded() {
        let l = Limits::new();
        let caps = Caps {
            per_key: 1,
            rate: 1,
            global: u32::MAX,
        };
        let t0 = Instant::now();
        for i in 0..(MAX_TRACKED as u32 + 100) {
            let addr = IpAddr::V4(std::net::Ipv4Addr::from(0x0a00_0000 + i));
            drop(l.admit(addr, caps, t0));
        }
        assert!(l.tracked() <= MAX_TRACKED);
        // Later, the stale windows are swept and new keys are tracked again.
        drop(l.admit(ip("198.18.0.1"), caps, t0 + WINDOW));
        assert!(l.tracked() <= MAX_TRACKED);
        assert!(matches!(
            l.admit(ip("198.18.0.1"), caps, t0 + WINDOW),
            Admission::Drop(DropReason::Rate)
        ));
    }
}
