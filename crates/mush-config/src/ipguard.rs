//! Per-IP abuse protection (design 02 §8.2): a sliding-window auth-failure
//! counter with temporary bans, plus a concurrent-connection cap, plus the
//! in-memory mirror of the **persisted sitebans** (M2).
//!
//! Defaults: **5 auth failures in 10 minutes → 15-minute tempban**; at most
//! **8 concurrent connections** per IP. Clock-free for tempbans: every
//! method takes an explicit `Instant` (the `*_now` wrappers stamp
//! `Instant::now()`), so the window math is deterministic under test.
//!
//! ## Two failure counters, not one
//!
//! A wrong **password** is a guess: nothing but knowing the secret makes the
//! next attempt likelier to work, so five of them in ten minutes buy a
//! fifteen-minute tempban and that number is not negotiable.
//!
//! A refused **bearer token** is not a guess. Web tokens are 256-bit store
//! records (`token:{hash}`, 7-day sliding TTL — see
//! `mush_gateway::web_bridge`), so a browser tab presenting one that the
//! server no longer knows is almost always a tab that outlived the record:
//! the store was flushed and rebuilt (`scripts/reset-instance.sh`), the
//! account's tokens were revoked, or the TTL simply ran out while the tab
//! slept. The tab then reconnects on its own backoff, and on 2026-09-20 that
//! is exactly what happened on dev1. The store had been flushed the day
//! before, so it held no token at all; a browser still carried one from the
//! old world; the refusal was a pre-upgrade HTTP 401, which a browser reports
//! as a dropped network, so the tab never showed a sign-in form and kept
//! trying. Five refused upgrades in about five minutes tempbanned the owner's
//! own address out of SSH, and as each fifteen-minute ban lapsed the same tab
//! re-armed it: five bans in seventy minutes, from one forgotten tab.
//! Guessing a 256-bit token is not a threat model; locking a player's address
//! out of the front door because their browser did what it was built to do is
//! a real one.
//!
//! So refused tokens get [`IpGuard::record_token_failure_at`] — their own,
//! much looser sliding window (**30 in 10 minutes → a 60-second cool-off**,
//! [`IpGuard::is_token_throttled_at`]) that can *delay* an address and can
//! never tempban it. It bounds the cost of token probing without ever
//! touching [`IpGuard::is_blocked_at`], which stays the union of the
//! password tempban and the persisted sitebans.
//!
//! Failure and connection accounting is keyed on the **network**, not the
//! address: a client with a routed /64 would otherwise buy a fresh failure
//! budget and a fresh connection cap for every one of 2^64 addresses. The
//! guard folds each address to [`IpGuardConfig::v4_prefix`] (default /32 — no
//! change from per-address for IPv4) and [`IpGuardConfig::v6_prefix`]
//! (default /64, the smallest unit anyone is routed), and an IPv4-mapped v6
//! address folds as the IPv4 address it is.
//!
//! ## The accept gates (doc 29a §9)
//!
//! The public SSH port asks [`IpGuard::admit_at`] about every accepted TCP
//! connection before any SSH byte is exchanged, because each connection
//! that gets a handshake costs a key exchange. Four answers, in the order
//! they are checked: a network over its **rate**
//! ([`IpGuardConfig::per_ip_new_per_minute`] new connections per
//! [`RATE_WINDOW`]), the process over its **global** cap
//! ([`IpGuardConfig::global_connections`]), an IPv6 **/48** over its
//! aggregate cap ([`IpGuardConfig::per_v6_48_connections`], across its /64s
//! — one customer allocation is exactly 65,536 /64s, each of which would
//! otherwise buy a full budget) are dropped with no handshake; a network over
//! its **concurrency** cap ([`IpGuardConfig::max_conns`]) is admitted
//! [`Admission::OverCap`], to be told and refused in SSH, at most
//! [`MAX_UNCOUNTED_PER_KEY`] of those at once per network so a key cannot
//! hold global capacity in that state. Everything the gates hold is released
//! by [`IpGuard::release`] with the same answer.
//!
//! Every table is **bounded and evicts**; none ever fails closed. The rate
//! table holds at most [`MAX_TRACKED`] networks: past that it is swept of
//! stale windows at most once per [`SWEEP_EVERY`], and if still full one
//! entry is evicted to make room. Refusing new networks instead would let a
//! single /48, connecting once from each of its 65,536 /64s and closing at
//! once, lock every new player out in under a minute (doc 29a §8 F-5). The
//! global and /48 caps are checked before a new network is inserted, so a
//! flood that would be dropped anyway does not fill the table. The
//! failure/connection map is bounded the same way: swept of idle entries,
//! then one idle entry evicted; a live connection or an active ban is never
//! evicted.
//!
//! Networks in [`IpGuardConfig::exempt_nets`] (default: loopback) skip the
//! per-network caps — the concurrency cap, the rate and the /48 aggregate;
//! the owner's bot fleet runs from one address, which trips a per-address
//! cap of eight — but **not** the global cap, which bounds the process
//! rather than the player (the doorway's semantics, so the two halves
//! match), and **never** the password tempban or the sitebans: an exempt
//! address that guesses passwords is banned like any other. Exempt
//! admissions are counted (`exempt`), not logged one by one.
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

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;

/// The accept-rate window ([`IpGuardConfig::per_ip_new_per_minute`]).
pub const RATE_WINDOW: Duration = Duration::from_secs(60);

/// Most networks the rate table — and the failure/connection map — track at
/// once. Exactly the /64s of one IPv6 /48, which is why a full table evicts
/// rather than refuses (module docs).
pub const MAX_TRACKED: usize = 65_536;

/// How often a full table may be swept: the sweep is O(table) under a lock,
/// so it must not run on every accept (doc 29a §7 "Splice and DoS" 8).
pub const SWEEP_EVERY: Duration = Duration::from_secs(1);

/// Most simultaneous over-cap ([`Admission::OverCap`]) connections admitted
/// per network solely to be told "too many connections" in SSH. Further ones
/// are dropped at accept, so one network cannot hold global capacity in the
/// banner state (doc 29a §8 F-1).
pub const MAX_UNCOUNTED_PER_KEY: u32 = 2;

/// A network in CIDR notation (`10.0.0.0/8`, `2001:db8::/32`) or one
/// address (`192.0.2.7`, a /32; `::1`, a /128): the shape of
/// [`IpGuardConfig::exempt_nets`]. Host bits are zeroed on construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IpNet {
    addr: IpAddr,
    prefix: u8,
}

/// Why a string is not an [`IpNet`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct IpNetError(String);

impl IpNet {
    /// `addr/prefix`, or `None` when the prefix is longer than the address
    /// family allows (32 for IPv4, 128 for IPv6).
    #[must_use]
    pub fn new(addr: IpAddr, prefix: u8) -> Option<IpNet> {
        let addr = match addr.to_canonical() {
            IpAddr::V4(v4) if prefix <= 32 => IpAddr::V4(mask_v4(v4, prefix)),
            IpAddr::V6(v6) if prefix <= 128 => IpAddr::V6(mask_v6(v6, prefix)),
            _ => return None,
        };
        Some(IpNet { addr, prefix })
    }

    /// The network address (host bits zero).
    #[must_use]
    pub fn addr(&self) -> IpAddr {
        self.addr
    }

    /// The prefix length.
    #[must_use]
    pub fn prefix(&self) -> u8 {
        self.prefix
    }

    /// Whether `ip` is inside this network. An IPv4-mapped IPv6 address is
    /// the IPv4 address it carries; otherwise the families must match.
    #[must_use]
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip.to_canonical()) {
            (IpAddr::V4(net), IpAddr::V4(v4)) => mask_v4(v4, self.prefix) == net,
            (IpAddr::V6(net), IpAddr::V6(v6)) => mask_v6(v6, self.prefix) == net,
            _ => false,
        }
    }
}

impl FromStr for IpNet {
    type Err = IpNetError;

    fn from_str(s: &str) -> Result<IpNet, IpNetError> {
        let s = s.trim();
        let (addr, prefix) = match s.split_once('/') {
            Some((addr, prefix)) => {
                let prefix: u8 = prefix
                    .parse()
                    .map_err(|_| IpNetError(format!("{s:?}: bad prefix length {prefix:?}")))?;
                (addr, Some(prefix))
            }
            None => (s, None),
        };
        let addr: IpAddr = addr
            .parse()
            .map_err(|_| IpNetError(format!("{s:?}: not an IP address or CIDR network")))?;
        let prefix = prefix.unwrap_or(if addr.to_canonical().is_ipv4() {
            32
        } else {
            128
        });
        IpNet::new(addr, prefix).ok_or_else(|| {
            IpNetError(format!(
                "{s:?}: prefix length {prefix} is longer than the address"
            ))
        })
    }
}

impl std::fmt::Display for IpNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

/// The top 48 bits of a native IPv6 address: one customer's typical
/// allocation, capped in aggregate over its /64s. `None` for IPv4 and for an
/// IPv4-mapped address.
fn v6_48(ip: IpAddr) -> Option<u64> {
    match ip.to_canonical() {
        IpAddr::V6(v6) => Some((u128::from(v6) >> 80) as u64),
        IpAddr::V4(_) => None,
    }
}

/// What the accept gates say about one connection ([`IpGuard::admit_at`]).
/// Every `Admit` and `OverCap` is released once with [`IpGuard::release`]
/// and the same value; a `Drop` holds nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// Serve it: it holds a per-network slot, a global slot and (IPv6) a /48
    /// slot.
    Admit,
    /// Over the network's concurrency cap: serve it only far enough to say
    /// so, then refuse. Holds an uncounted per-network slot, a global slot
    /// and (IPv6) a /48 slot, for the few seconds that takes.
    OverCap,
    /// Close the TCP connection now, with no handshake.
    Drop(DropReason),
}

/// Why a connection is dropped at accept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DropReason {
    /// Over the network's rate of new connections per [`RATE_WINDOW`].
    Rate,
    /// Over the global cap.
    Global,
    /// Over the IPv6 /48 aggregate cap.
    Prefix48,
    /// Over the network's concurrency cap with [`MAX_UNCOUNTED_PER_KEY`]
    /// banner slots already in flight.
    PerIp,
}

impl DropReason {
    /// The counter this reason is tallied under ([`IpGuard::take_counters`]).
    #[must_use]
    pub fn counter(self) -> &'static str {
        match self {
            DropReason::Rate => "rate",
            DropReason::Global => "global",
            DropReason::Prefix48 => "per-48",
            DropReason::PerIp => "per-ip",
        }
    }
}

/// Count a refusal and answer it.
fn refuse(gates: &mut Gates, reason: DropReason) -> Admission {
    *gates.counters.entry(reason.counter()).or_default() += 1;
    Admission::Drop(reason)
}

/// The accept gates' own state, under one lock: the rate table, the /48
/// aggregates, the refusal counters and the sweep clock.
#[derive(Debug, Default)]
struct Gates {
    /// Network → (window start, connections in the window).
    rate: HashMap<IpAddr, (Instant, u32)>,
    /// IPv6 /48 → live connections across its /64s.
    concurrent48: HashMap<u64, u32>,
    /// Refusals since the counters were last taken.
    counters: BTreeMap<&'static str, u64>,
    last_rate_sweep: Option<Instant>,
    last_map_sweep: Option<Instant>,
}

/// How often a connection-owning gateway re-reads the persisted siteban set
/// into its [`IpGuard`] mirror, and therefore the worst-case latency between
/// an acknowledged `@siteban` store write and the address being refused.
///
/// A whole-mirror reload is one small store read, so this can be short; it is
/// not so short that a fleet of gateways becomes a poll storm.
pub const SITEBAN_REFRESH: Duration = Duration::from_secs(10);

/// Tunables for [`IpGuard`]. `[ipguard]` in `mushd.toml`
/// (`mush_config::config::IpGuardCfg`) is its serialised form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpGuardConfig {
    /// Auth failures within [`IpGuardConfig::window`] that trigger a ban.
    pub max_failures: u32,
    /// Sliding window over which failures are counted.
    pub window: Duration,
    /// How long a triggered ban lasts.
    pub ban: Duration,
    /// Refused **bearer tokens** within [`IpGuardConfig::token_window`] that
    /// trigger a cool-off. Deliberately far above `max_failures`: see the
    /// module docs — a refused token is a stale client, not a guess.
    pub max_token_failures: u32,
    /// Sliding window over which refused tokens are counted.
    pub token_window: Duration,
    /// How long a triggered token cool-off lasts. Short on purpose: it is a
    /// brake on probing, not a punishment, and it never bans anything.
    pub token_cooloff: Duration,
    /// Maximum concurrent connections per IP.
    pub max_conns: u32,
    /// Prefix length an IPv4 address is folded to before it is counted.
    /// Default 32: one address, one budget, exactly as before.
    pub v4_prefix: u8,
    /// Prefix length an IPv6 address is folded to before it is counted.
    /// Default 64: the smallest block anyone is routed, so a client cannot
    /// buy a fresh budget by moving one address to the left.
    pub v6_prefix: u8,
    /// New SSH connections one network may open per [`RATE_WINDOW`]; the
    /// rest are dropped at accept with no handshake. Default 20.
    pub per_ip_new_per_minute: u32,
    /// SSH connections live at once in the whole process, admitted and
    /// over-cap together; past it new ones are dropped at accept. Default
    /// 2048.
    pub global_connections: u32,
    /// SSH connections live at once from one IPv6 /48, across all its
    /// /64s. Default 64.
    pub per_v6_48_connections: u32,
    /// Networks exempt from the per-network caps (the concurrency cap, the
    /// rate and the /48 aggregate) — not from the global cap, and never
    /// from the password tempban or the sitebans. Default: `127.0.0.0/8`
    /// and `::1/128`.
    pub exempt_nets: Vec<IpNet>,
}

impl Default for IpGuardConfig {
    fn default() -> Self {
        IpGuardConfig {
            max_failures: 5,
            window: Duration::from_secs(600),
            ban: Duration::from_secs(900),
            max_token_failures: 30,
            token_window: Duration::from_secs(600),
            token_cooloff: Duration::from_secs(60),
            max_conns: 8,
            v4_prefix: 32,
            v6_prefix: 64,
            per_ip_new_per_minute: 20,
            global_connections: 2048,
            per_v6_48_connections: 64,
            exempt_nets: default_exempt_nets(),
        }
    }
}

/// Loopback, both families: the box's own bots and probes.
fn default_exempt_nets() -> Vec<IpNet> {
    [
        IpNet::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8),
        IpNet::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 128),
    ]
    .into_iter()
    .flatten()
    .collect()
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
    /// Start of the ban; an older store lift must not pardon newer failures.
    banned_at: Option<Instant>,
    /// Instants of recent refused bearer tokens (pruned to the token
    /// window). Kept apart from `failures` so a stale browser tab can never
    /// contribute to a tempban.
    token_failures: Vec<Instant>,
    /// Active token cool-off, if any. Never consulted by
    /// [`IpGuard::is_blocked_at`].
    token_cooloff_until: Option<Instant>,
    /// Live connections from this IP.
    conns: u32,
    /// Live over-cap connections from this IP, admitted only to be told
    /// ([`Admission::OverCap`]); at most [`MAX_UNCOUNTED_PER_KEY`].
    uncounted: u32,
}

impl IpEntry {
    fn is_idle(&self) -> bool {
        self.failures.is_empty()
            && self.banned_until.is_none()
            && self.token_failures.is_empty()
            && self.token_cooloff_until.is_none()
            && self.conns == 0
            && self.uncounted == 0
    }

    /// Forget what the windows have outlived, as the recording paths do
    /// lazily, so an untouched entry can be seen to be idle.
    fn prune(&mut self, cfg: &IpGuardConfig, now: Instant) {
        self.failures
            .retain(|&t| now.saturating_duration_since(t) < cfg.window);
        self.token_failures
            .retain(|&t| now.saturating_duration_since(t) < cfg.token_window);
        if self.banned_until.is_some_and(|until| until <= now) {
            self.banned_until = None;
            self.banned_at = None;
        }
        if self.token_cooloff_until.is_some_and(|until| until <= now) {
            self.token_cooloff_until = None;
        }
    }

    /// Whether evicting this entry loses nothing that matters: no live
    /// connection and no active ban.
    fn evictable(&self) -> bool {
        self.conns == 0 && self.uncounted == 0 && self.banned_until.is_none()
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
    /// Lift identifiers retained until their store TTL expires. Never evicted
    /// early: that would permit a refresh to apply an old pardon twice.
    tempban_lifts: std::sync::Mutex<std::collections::HashMap<String, i64>>,
    /// The accept gates (module docs). Locked before `map` wherever both are
    /// touched, so [`IpGuard::admit_at`] and [`IpGuard::release`] cannot
    /// deadlock against each other.
    gates: Mutex<Gates>,
    /// SSH connections live at once, admitted and over-cap together.
    global: AtomicUsize,
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
            tempban_lifts: std::sync::Mutex::default(),
            gates: Mutex::default(),
            global: AtomicUsize::new(0),
        }
    }

    /// The active configuration.
    #[must_use]
    pub fn config(&self) -> &IpGuardConfig {
        &self.cfg
    }

    /// Whether `ip` is in [`IpGuardConfig::exempt_nets`]: no concurrency
    /// cap, no rate, no /48 aggregate. The global cap and the bans still
    /// apply.
    #[must_use]
    pub fn is_exempt(&self, ip: IpAddr) -> bool {
        self.cfg.exempt_nets.iter().any(|net| net.contains(ip))
    }

    fn gates(&self) -> std::sync::MutexGuard<'_, Gates> {
        self.gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Keep the failure/connection map under [`MAX_TRACKED`] entries before
    /// an insert: sweep it of idle entries at most once per [`SWEEP_EVERY`],
    /// and if it is still full evict one entry that holds no live connection
    /// and no active ban. Never refuses (module docs).
    fn bound_map(&self, now: Instant) {
        if self.map.len() < MAX_TRACKED {
            return;
        }
        let mut gates = self.gates();
        self.bound_map_locked(&mut gates, now);
    }

    /// Gate one accepted SSH connection at `now`, before any handshake
    /// (module docs: the order is rate, global, /48, concurrency). The
    /// caller releases an `Admit` or `OverCap` with [`IpGuard::release`].
    pub fn admit_at(&self, ip: IpAddr, now: Instant) -> Admission {
        let global_cap = self.cfg.global_connections as usize;
        if self.is_exempt(ip) {
            // Exempt from every per-network cap, not from the process's:
            // an exempt address holds a global slot like any other, and is
            // refused by the global cap like any other.
            let mut gates = self.gates();
            if self.global.load(Ordering::SeqCst) >= global_cap {
                return refuse(&mut gates, DropReason::Global);
            }
            self.global.fetch_add(1, Ordering::SeqCst);
            *gates.counters.entry("exempt").or_default() += 1;
            return Admission::Admit;
        }
        let net = self.key(ip);
        let prefix = v6_48(ip);
        let mut gates = self.gates();
        if let Some(entry) = gates.rate.get_mut(&net) {
            // A known network: its rate window first, so a flooder is told
            // off as `Rate` before anything else is spent on it.
            if now.saturating_duration_since(entry.0) >= RATE_WINDOW {
                *entry = (now, 0);
            }
            entry.1 = entry.1.saturating_add(1);
            if entry.1 > self.cfg.per_ip_new_per_minute {
                return refuse(&mut gates, DropReason::Rate);
            }
        } else {
            // A new network: the global and /48 caps are checked before it
            // is inserted, so a flood that would be dropped anyway cannot
            // fill the table against everyone else (doc 29a §8 F-5).
            if self.global.load(Ordering::SeqCst) >= global_cap {
                return refuse(&mut gates, DropReason::Global);
            }
            if let Some(p) = prefix
                && gates.concurrent48.get(&p).copied().unwrap_or(0)
                    >= self.cfg.per_v6_48_connections
            {
                return refuse(&mut gates, DropReason::Prefix48);
            }
            if self.cfg.per_ip_new_per_minute == 0 {
                return refuse(&mut gates, DropReason::Rate);
            }
            if gates.rate.len() >= MAX_TRACKED
                && gates
                    .last_rate_sweep
                    .is_none_or(|t| now.saturating_duration_since(t) >= SWEEP_EVERY)
            {
                gates.last_rate_sweep = Some(now);
                gates
                    .rate
                    .retain(|_, (start, _)| now.saturating_duration_since(*start) < RATE_WINDOW);
            }
            if gates.rate.len() >= MAX_TRACKED {
                // Still full of live windows: make room rather than refuse.
                // A flood that fills the table owns almost every entry, so
                // the victim almost always is its own; a legitimate network
                // that is evicted merely starts a fresh window.
                let victim = gates.rate.keys().next().copied();
                if let Some(v) = victim {
                    gates.rate.remove(&v);
                }
                *gates.counters.entry("rate-evict").or_default() += 1;
            }
            gates.rate.insert(net, (now, 1));
        }
        if self.global.load(Ordering::SeqCst) >= global_cap {
            return refuse(&mut gates, DropReason::Global);
        }
        if let Some(p) = prefix
            && gates.concurrent48.get(&p).copied().unwrap_or(0) >= self.cfg.per_v6_48_connections
        {
            return refuse(&mut gates, DropReason::Prefix48);
        }
        self.bound_map_locked(&mut gates, now);
        let admission = {
            let mut e = self.map.entry(net).or_default();
            if e.conns < self.cfg.max_conns {
                e.conns += 1;
                Admission::Admit
            } else if e.uncounted < MAX_UNCOUNTED_PER_KEY {
                e.uncounted += 1;
                *gates.counters.entry("per-ip").or_default() += 1;
                Admission::OverCap
            } else {
                Admission::Drop(DropReason::PerIp)
            }
        };
        if let Admission::Drop(reason) = admission {
            self.map.remove_if(&net, |_, e| e.is_idle());
            return refuse(&mut gates, reason);
        }
        if let Some(p) = prefix {
            *gates.concurrent48.entry(p).or_default() += 1;
        }
        self.global.fetch_add(1, Ordering::SeqCst);
        admission
    }

    /// [`IpGuard::bound_map`] with the gates already locked.
    fn bound_map_locked(&self, gates: &mut Gates, now: Instant) {
        if self.map.len() < MAX_TRACKED {
            return;
        }
        if gates
            .last_map_sweep
            .is_none_or(|t| now.saturating_duration_since(t) >= SWEEP_EVERY)
        {
            gates.last_map_sweep = Some(now);
            self.map.retain(|_, e| {
                e.prune(&self.cfg, now);
                !e.is_idle()
            });
        }
        if self.map.len() < MAX_TRACKED {
            return;
        }
        let victim = self
            .map
            .iter()
            .find(|e| e.value().evictable())
            .map(|e| *e.key());
        if let Some(net) = victim {
            self.map.remove_if(&net, |_, e| e.evictable());
            *gates.counters.entry("map-evict").or_default() += 1;
        }
    }

    /// [`IpGuard::admit_at`] with `Instant::now()`.
    pub fn admit(&self, ip: IpAddr) -> Admission {
        self.admit_at(ip, Instant::now())
    }

    /// Release what [`IpGuard::admit_at`] held for `ip`, given the answer it
    /// gave. A `Drop` held nothing; an exempt address held a global slot
    /// only. Saturates at zero everywhere; idle entries are dropped.
    pub fn release(&self, ip: IpAddr, admission: Admission) {
        if matches!(admission, Admission::Drop(_)) {
            return;
        }
        let mut gates = self.gates();
        let _ = self
            .global
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                Some(n.saturating_sub(1))
            });
        if self.is_exempt(ip) {
            return;
        }
        let net = self.key(ip);
        if let Some(p) = v6_48(ip)
            && let Some(n) = gates.concurrent48.get_mut(&p)
        {
            *n = n.saturating_sub(1);
            if *n == 0 {
                gates.concurrent48.remove(&p);
            }
        }
        let remove = match self.map.get_mut(&net) {
            Some(mut e) => {
                match admission {
                    Admission::Admit => e.conns = e.conns.saturating_sub(1),
                    Admission::OverCap => e.uncounted = e.uncounted.saturating_sub(1),
                    Admission::Drop(_) => {}
                }
                e.is_idle()
            }
            None => false,
        };
        if remove {
            self.map.remove_if(&net, |_, e| e.is_idle());
        }
    }

    /// SSH connections live at once through the accept gates (admitted and
    /// over-cap together, exempt addresses included: the global cap bounds
    /// the process).
    #[must_use]
    pub fn live(&self) -> usize {
        self.global.load(Ordering::SeqCst)
    }

    /// Take the counters (a per-minute summary resets them): the
    /// [`DropReason::counter`] names, `per-ip` for over-cap admissions,
    /// `exempt` for admissions the allow-list waved through, `rate-evict`
    /// and `map-evict` for table evictions.
    pub fn take_counters(&self) -> BTreeMap<&'static str, u64> {
        std::mem::take(&mut self.gates().counters)
    }

    /// Networks in the rate table (tests, diagnostics).
    #[must_use]
    pub fn rate_tracked(&self) -> usize {
        self.gates().rate.len()
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

    /// How much of `ip`'s **tempban** is left at `now`, or `None` when it is
    /// not tempbanned. Sitebans are not counted here: they carry a wall-clock
    /// expiry (or none at all) and answer [`IpGuard::siteban_active_at`].
    ///
    /// This exists so a transport can tell a refused client *when* to come
    /// back instead of only *no* — see `mush_net_ssh`'s authentication
    /// banner. Unlike [`IpGuard::is_blocked_at`] it does not clear a lapsed
    /// ban (it takes `&self` through a read guard); the next `is_blocked_at`
    /// does that.
    #[must_use]
    pub fn tempban_remaining_at(&self, ip: IpAddr, now: Instant) -> Option<Duration> {
        let e = self.map.get(&self.key(ip))?;
        let until = e.banned_until?;
        (until > now).then(|| until.saturating_duration_since(now))
    }

    /// [`IpGuard::tempban_remaining_at`] with `Instant::now()`.
    #[must_use]
    pub fn tempban_remaining(&self, ip: IpAddr) -> Option<Duration> {
        self.tempban_remaining_at(ip, Instant::now())
    }

    /// Record one **refused bearer token** against `ip`'s network. Returns
    /// `true` when this refusal crossed the threshold and started a cool-off.
    ///
    /// This is the whole of the separation described in the module docs: it
    /// writes only `token_failures`/`token_cooloff_until`, so no number of
    /// refused tokens can ever produce a tempban or affect
    /// [`IpGuard::is_blocked_at`].
    pub fn record_token_failure_at(&self, ip: IpAddr, now: Instant) -> bool {
        let net = self.key(ip);
        self.bound_map(now);
        let mut e = self.map.entry(net).or_default();
        let window = self.cfg.token_window;
        e.token_failures
            .retain(|&t| now.saturating_duration_since(t) < window);
        e.token_failures.push(now);
        if e.token_failures.len() >= self.cfg.max_token_failures as usize {
            e.token_failures.clear();
            e.token_cooloff_until = Some(now + self.cfg.token_cooloff);
            tracing::info!(
                %ip,
                %net,
                cooloff_s = self.cfg.token_cooloff.as_secs(),
                "refused tokens exceeded; token cool-off (no tempban)"
            );
            true
        } else {
            false
        }
    }

    /// [`IpGuard::record_token_failure_at`] with `Instant::now()`.
    pub fn record_token_failure(&self, ip: IpAddr) -> bool {
        self.record_token_failure_at(ip, Instant::now())
    }

    /// Whether `ip` is inside a token cool-off at `now` — i.e. whether a
    /// transport should refuse to even look up the next token it presents.
    /// Expired cool-offs are cleared as a side effect. Never a ban: a client
    /// in cool-off is still admitted by [`IpGuard::is_blocked_at`], and SSH
    /// never consults this at all.
    #[must_use]
    pub fn is_token_throttled_at(&self, ip: IpAddr, now: Instant) -> bool {
        let Some(mut e) = self.map.get_mut(&self.key(ip)) else {
            return false;
        };
        if let Some(until) = e.token_cooloff_until {
            if now < until {
                return true;
            }
            e.token_cooloff_until = None;
        }
        false
    }

    /// [`IpGuard::is_token_throttled_at`] with `Instant::now()`.
    #[must_use]
    pub fn is_token_throttled(&self, ip: IpAddr) -> bool {
        self.is_token_throttled_at(ip, Instant::now())
    }

    /// Record one auth failure against `ip`'s network. Returns `true` when
    /// this failure crossed the threshold and started a tempban — which
    /// blocks every address in that network, not just this one.
    pub fn record_failure_at(&self, ip: IpAddr, now: Instant) -> bool {
        let net = self.key(ip);
        self.bound_map(now);
        let mut e = self.map.entry(net).or_default();
        let window = self.cfg.window;
        e.failures
            .retain(|&t| now.saturating_duration_since(t) < window);
        e.failures.push(now);
        if e.failures.len() >= self.cfg.max_failures as usize {
            e.failures.clear();
            e.banned_until = Some(now + self.cfg.ban);
            e.banned_at = Some(now);
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

    /// Apply a short-lived staff pardon once in this guard. It clears only
    /// password failures and a tempban predating the request, across the same
    /// configured network as failure accounting. Sitebans, token cool-offs and
    /// connection counts are untouched. Returns whether an active ban was lifted.
    /// At most 4,096 unexpired request IDs are retained; a full cache fails closed.
    pub fn apply_tempban_lift(
        &self,
        ip: IpAddr,
        id: &str,
        issued_unix_ms: i64,
        expires_unix: i64,
    ) -> bool {
        let now = Instant::now();
        let unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        self.apply_tempban_lift_at(ip, id, issued_unix_ms, expires_unix, now, unix_ms)
    }

    fn apply_tempban_lift_at(
        &self,
        ip: IpAddr,
        id: &str,
        issued_unix_ms: i64,
        expires_unix: i64,
        now: Instant,
        unix_ms: i64,
    ) -> bool {
        let unix_s = unix_ms / 1_000;
        if expires_unix <= unix_s
            || expires_unix > unix_s.saturating_add(300)
            || issued_unix_ms > unix_ms
            || id.is_empty()
            || id.len() > 64
        {
            return false;
        }
        let Ok(mut seen) = self.tempban_lifts.lock() else {
            tracing::error!("tempban lift cache poisoned; refusing pardon");
            return false;
        };
        seen.retain(|_, expires| *expires > unix_s);
        if seen.contains_key(id) || seen.len() >= 4_096 {
            return false;
        }
        seen.insert(id.to_owned(), expires_unix);
        // Both wall stamps are truncated to milliseconds. Exclude the entire
        // ambiguous millisecond rather than pardon a failure just after issue.
        let elapsed = Duration::from_millis(unix_ms.saturating_sub(issued_unix_ms) as u64)
            .saturating_add(Duration::from_millis(1));
        let Some(cutoff) = now.checked_sub(elapsed) else {
            return false;
        };
        let Some(mut entry) = self.map.get_mut(&self.key(ip)) else {
            return false;
        };
        entry.failures.retain(|failure| *failure > cutoff);
        if entry.banned_at.is_some_and(|at| at <= cutoff) {
            entry.banned_at = None;
            return entry.banned_until.take().is_some_and(|until| until > now);
        }
        false
    }

    /// Count a new connection from `ip` against the concurrency cap only
    /// (the web ingress and the trusted ingress, which the rate and global
    /// caps do not apply to). Returns `false` (and does not count it) when
    /// `ip`'s network is already at its cap — the cap is shared by every
    /// address behind one prefix, and by its SSH connections. An exempt
    /// address is always admitted and never counted.
    pub fn conn_opened(&self, ip: IpAddr) -> bool {
        if self.is_exempt(ip) {
            return true;
        }
        self.bound_map(Instant::now());
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
        if self.is_exempt(ip) {
            return;
        }
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
    fn lift_is_once_per_guard_and_preserves_new_failures_and_other_admission_state() {
        let g = IpGuard::with_config(IpGuardConfig {
            max_conns: 1,
            ..IpGuardConfig::default()
        });
        let t0 = Instant::now();
        for _ in 0..5 {
            g.record_failure_at(ip6(1, 7), t0);
        }
        for _ in 0..30 {
            g.record_token_failure_at(ip6(1, 8), t0);
        }
        assert!(g.conn_opened(ip6(1, 9)));
        g.set_siteban(ip6(1, 10), None);
        assert!(g.apply_tempban_lift_at(
            ip6(1, 99),
            "first",
            101_000,
            400,
            t0 + Duration::from_secs(2),
            102_000
        ));
        assert!(!g.is_blocked_at(ip6(1, 7), t0 + Duration::from_secs(2)));
        assert!(
            g.is_blocked_at(ip6(1, 10), t0 + Duration::from_secs(2)),
            "siteban remains"
        );
        assert!(g.is_token_throttled_at(ip6(1, 8), t0 + Duration::from_secs(2)));
        assert!(!g.conn_opened(ip6(1, 11)), "connection cap remains");
        for _ in 0..5 {
            g.record_failure_at(ip6(1, 7), t0 + Duration::from_secs(3));
        }
        assert!(!g.apply_tempban_lift_at(
            ip6(1, 99),
            "first",
            101_000,
            400,
            t0 + Duration::from_secs(4),
            104_000
        ));
        assert!(
            g.is_blocked_at(ip6(1, 7), t0 + Duration::from_secs(4)),
            "refresh never repeats a pardon"
        );
        // Even a never-seen request cannot clear a ban started after its issue time.
        assert!(!g.apply_tempban_lift_at(
            ip6(1, 99),
            "delayed",
            102_000,
            400,
            t0 + Duration::from_secs(4),
            104_000
        ));
        assert!(g.is_blocked_at(ip6(1, 7), t0 + Duration::from_secs(4)));
    }

    #[test]
    fn lift_clears_only_failures_before_issue_and_refuses_expired_commands() {
        let g = IpGuard::new();
        let t0 = Instant::now();
        for _ in 0..4 {
            g.record_failure_at(ip(1), t0);
        }
        assert!(!g.apply_tempban_lift_at(
            ip(1),
            "clear-window",
            101_000,
            400,
            t0 + Duration::from_secs(2),
            102_000
        ));
        for _ in 0..4 {
            assert!(!g.record_failure_at(ip(1), t0 + Duration::from_secs(3)));
        }
        assert!(g.record_failure_at(ip(1), t0 + Duration::from_secs(3)));
        assert!(!g.apply_tempban_lift_at(
            ip(1),
            "expired",
            105_000,
            106,
            t0 + Duration::from_secs(10),
            110_000
        ));
        assert!(g.is_blocked_at(ip(1), t0 + Duration::from_secs(10)));
    }

    #[test]
    fn refused_tokens_cool_off_and_never_tempban() {
        // The 2026-09-20 incident in miniature: one client presenting a
        // token the server no longer knows, over and over. Six times the
        // password budget goes by without a tempban, and the address stays
        // admissible to SSH throughout.
        let g = IpGuard::new();
        let t0 = Instant::now();
        for i in 0..29u64 {
            assert!(!g.record_token_failure_at(ip(1), t0 + Duration::from_secs(i)));
            assert!(!g.is_token_throttled_at(ip(1), t0 + Duration::from_secs(i)));
            assert!(!g.is_blocked_at(ip(1), t0 + Duration::from_secs(i)));
        }
        // The thirtieth inside ten minutes arms the cool-off — and only that.
        assert!(g.record_token_failure_at(ip(1), t0 + Duration::from_secs(29)));
        assert!(g.is_token_throttled_at(ip(1), t0 + Duration::from_secs(30)));
        assert!(
            !g.is_blocked_at(ip(1), t0 + Duration::from_secs(30)),
            "a token cool-off must never close the SSH front door"
        );
        assert_eq!(
            g.tempban_remaining_at(ip(1), t0 + Duration::from_secs(30)),
            None
        );
        // 60 seconds later it lapses, and clears itself on the way out.
        assert!(!g.is_token_throttled_at(ip(1), t0 + Duration::from_secs(91)));
        // The two counters are independent: a password budget is untouched.
        for i in 0..4u64 {
            assert!(!g.record_failure_at(ip(1), t0 + Duration::from_secs(100 + i)));
        }
        assert!(g.record_failure_at(ip(1), t0 + Duration::from_secs(104)));
        assert!(g.is_blocked_at(ip(1), t0 + Duration::from_secs(105)));
    }

    #[test]
    fn old_token_failures_age_out_and_entries_still_drop() {
        let g = IpGuard::new();
        let t0 = Instant::now();
        for _ in 0..29 {
            g.record_token_failure_at(ip(7), t0);
        }
        // The thirtieth arrives after the window: the first 29 have aged out.
        assert!(!g.record_token_failure_at(ip(7), t0 + Duration::from_secs(601)));
        assert!(!g.is_token_throttled_at(ip(7), t0 + Duration::from_secs(601)));
        // An entry holding only token state is still idle-collected once the
        // cool-off is cleared and nothing else is tracked.
        assert_eq!(g.tracked(), 1);
        g.conn_opened(ip(7));
        g.conn_closed(ip(7));
        assert_eq!(g.tracked(), 1, "a live token window keeps the entry");
    }

    #[test]
    fn tempban_remaining_counts_down_and_ignores_sitebans() {
        let g = IpGuard::new();
        let t0 = Instant::now();
        assert_eq!(g.tempban_remaining_at(ip(5), t0), None);
        for i in 0..5u64 {
            g.record_failure_at(ip(5), t0 + Duration::from_secs(i));
        }
        let banned_at = t0 + Duration::from_secs(4);
        assert_eq!(
            g.tempban_remaining_at(ip(5), banned_at),
            Some(Duration::from_secs(900))
        );
        assert_eq!(
            g.tempban_remaining_at(ip(5), banned_at + Duration::from_secs(300)),
            Some(Duration::from_secs(600))
        );
        assert_eq!(
            g.tempban_remaining_at(ip(5), banned_at + Duration::from_secs(900)),
            None
        );
        // A siteban is blocking but has no tempban countdown to report.
        g.set_siteban(ip(6), None);
        assert!(g.is_blocked(ip(6)));
        assert_eq!(g.tempban_remaining_at(ip(6), t0), None);
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

    // ---- the accept gates (doc 29a §9) ----

    fn gated(cfg: IpGuardConfig) -> IpGuard {
        IpGuard::with_config(IpGuardConfig {
            // Nothing in these tests comes from loopback.
            exempt_nets: Vec::new(),
            ..cfg
        })
    }

    fn admitted(a: Admission) -> Admission {
        assert!(
            matches!(a, Admission::Admit | Admission::OverCap),
            "dropped: {a:?}"
        );
        a
    }

    fn net(s: &str) -> IpNet {
        s.parse().unwrap()
    }

    #[test]
    fn ipnet_parses_cidr_and_bare_addresses_and_contains_mapped_v4() {
        let v4 = net("10.0.0.0/8");
        assert_eq!(v4.addr(), IpAddr::from([10, 0, 0, 0]));
        assert_eq!(v4.prefix(), 8);
        assert!(v4.contains(ip(7)));
        assert!(v4.contains("::ffff:10.9.9.9".parse().unwrap()));
        assert!(!v4.contains("11.0.0.1".parse().unwrap()));
        assert!(!v4.contains(ip6(1, 1)));
        // Host bits are dropped, and a bare address is a host route.
        assert_eq!(net("10.1.2.3/8"), v4);
        assert_eq!(net("192.0.2.7"), net("192.0.2.7/32"));
        assert_eq!(net("::1"), net("::1/128"));
        assert_eq!(net("2001:db8:1:2::9/48"), net("2001:db8:1::/48"));
        // `ip6(n, h)` is `2001:db8:0:n::h`: inside the /48 `2001:db8::/48`
        // whichever /64 it is in, outside the next /48 over.
        assert!(net("2001:db8::/48").contains(ip6(1, 1)));
        assert!(net("2001:db8::/48").contains(ip6(2, 1)));
        assert!(!net("2001:db8:1::/48").contains(ip6(1, 1)));
        assert_eq!(net("2001:db8::/32").to_string(), "2001:db8::/32");
        for bad in [
            "",
            "10.0.0.0/33",
            "::1/129",
            "10.0.0.0/x",
            "not an ip",
            "10.0.0/8",
        ] {
            assert!(bad.parse::<IpNet>().is_err(), "{bad:?} parsed");
        }
        let defaults = IpGuardConfig::default().exempt_nets;
        assert!(
            defaults
                .iter()
                .any(|n| n.contains(Ipv4Addr::LOCALHOST.into()))
        );
        assert!(
            defaults
                .iter()
                .any(|n| n.contains(Ipv6Addr::LOCALHOST.into()))
        );
        assert!(!defaults.iter().any(|n| n.contains(ip(1))));
    }

    #[test]
    fn the_rate_window_drops_the_twenty_first_and_resets() {
        let g = gated(IpGuardConfig::default());
        let t0 = Instant::now();
        for _ in 0..20 {
            g.release(ip(1), admitted(g.admit_at(ip(1), t0)));
        }
        assert_eq!(
            g.admit_at(ip(1), t0 + Duration::from_secs(30)),
            Admission::Drop(DropReason::Rate)
        );
        // Another network is unaffected; the window resets after a minute.
        g.release(ip(2), admitted(g.admit_at(ip(2), t0)));
        g.release(ip(1), admitted(g.admit_at(ip(1), t0 + RATE_WINDOW)));
        assert_eq!(g.take_counters().get("rate"), Some(&1));
        assert!(g.take_counters().is_empty());
    }

    #[test]
    fn the_global_cap_counts_admitted_and_over_cap_alike_and_releases() {
        let g = gated(IpGuardConfig {
            max_conns: 1,
            global_connections: 3,
            ..IpGuardConfig::default()
        });
        let t0 = Instant::now();
        assert_eq!(g.admit_at(ip(1), t0), Admission::Admit);
        assert_eq!(g.admit_at(ip(1), t0), Admission::OverCap);
        assert_eq!(g.admit_at(ip(2), t0), Admission::Admit);
        assert_eq!(g.live(), 3);
        assert_eq!(g.admit_at(ip(3), t0), Admission::Drop(DropReason::Global));
        // A drop holds nothing and its release is a no-op.
        g.release(ip(3), Admission::Drop(DropReason::Global));
        assert_eq!(g.live(), 3);
        g.release(ip(1), Admission::OverCap);
        assert_eq!(g.live(), 2);
        assert_eq!(g.admit_at(ip(3), t0), Admission::Admit);
        g.release(ip(1), Admission::Admit);
        g.release(ip(2), Admission::Admit);
        g.release(ip(3), Admission::Admit);
        assert_eq!(g.live(), 0);
        assert_eq!(g.tracked(), 0, "idle entries are dropped on release");
    }

    #[test]
    fn over_cap_banner_slots_are_bounded_per_network() {
        let g = gated(IpGuardConfig {
            max_conns: 1,
            ..IpGuardConfig::default()
        });
        let t0 = Instant::now();
        assert_eq!(g.admit_at(ip(1), t0), Admission::Admit);
        for _ in 0..MAX_UNCOUNTED_PER_KEY {
            assert_eq!(g.admit_at(ip(1), t0), Admission::OverCap);
        }
        assert_eq!(g.admit_at(ip(1), t0), Admission::Drop(DropReason::PerIp));
        assert_eq!(g.live(), 1 + MAX_UNCOUNTED_PER_KEY as usize);
        g.release(ip(1), Admission::OverCap);
        assert_eq!(g.admit_at(ip(1), t0), Admission::OverCap);
        let counters = g.take_counters();
        assert_eq!(counters.get("per-ip"), Some(&4), "{counters:?}");
    }

    #[test]
    fn a_v6_48_is_capped_across_its_64s_and_released() {
        let g = gated(IpGuardConfig {
            per_v6_48_connections: 3,
            ..IpGuardConfig::default()
        });
        let t0 = Instant::now();
        for n in 0..3 {
            assert_eq!(g.admit_at(ip6(n, 1), t0), Admission::Admit);
        }
        assert_eq!(
            g.admit_at(ip6(9, 1), t0),
            Admission::Drop(DropReason::Prefix48)
        );
        // Another /48 is unaffected, and a release frees a place.
        let other: IpAddr = "2001:db8:1::1".parse().unwrap();
        assert_eq!(g.admit_at(other, t0), Admission::Admit);
        g.release(ip6(0, 1), Admission::Admit);
        assert_eq!(g.admit_at(ip6(9, 1), t0), Admission::Admit);
        assert_eq!(g.take_counters().get("per-48"), Some(&1));
    }

    #[test]
    fn the_rate_table_is_bounded_and_evicts_rather_than_refuse() {
        let g = gated(IpGuardConfig {
            per_ip_new_per_minute: 1,
            global_connections: u32::MAX,
            ..IpGuardConfig::default()
        });
        let t0 = Instant::now();
        for i in 0..(MAX_TRACKED as u32) {
            let addr = IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + i));
            g.release(addr, admitted(g.admit_at(addr, t0)));
        }
        assert_eq!(g.rate_tracked(), MAX_TRACKED);
        // Full of live windows: a new network is still admitted (one entry
        // is evicted for it), the table stays bounded, and the new network
        // is tracked — its second connection in the window is over its rate.
        let extra = IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + MAX_TRACKED as u32));
        g.release(extra, admitted(g.admit_at(extra, t0)));
        assert_eq!(g.rate_tracked(), MAX_TRACKED);
        assert_eq!(g.admit_at(extra, t0), Admission::Drop(DropReason::Rate));
        // Later, the stale windows are swept and new networks are tracked
        // again.
        let later = t0 + RATE_WINDOW;
        let fresh: IpAddr = "198.18.0.1".parse().unwrap();
        g.release(fresh, admitted(g.admit_at(fresh, later)));
        assert!(g.rate_tracked() <= MAX_TRACKED);
        assert_eq!(g.admit_at(fresh, later), Admission::Drop(DropReason::Rate));
        assert_eq!(g.take_counters().get("rate-evict"), Some(&1));
    }

    #[test]
    fn one_v6_48_cannot_fill_the_rate_table_against_everyone_else() {
        // One /48 holds exactly MAX_TRACKED /64s. Connecting once from each
        // and closing at once (so the /48 cap never trips) fills the table;
        // a player from anywhere else must still get in (doc 29a §8 F-5).
        let g = gated(IpGuardConfig::default());
        let t0 = Instant::now();
        for i in 0..MAX_TRACKED as u128 {
            let v6 = Ipv6Addr::from((0x2001_0db8_0001u128 << 80) | (i << 64) | 1);
            g.release(IpAddr::V6(v6), admitted(g.admit_at(IpAddr::V6(v6), t0)));
        }
        assert_eq!(g.rate_tracked(), MAX_TRACKED);
        g.release(ip(7), admitted(g.admit_at(ip(7), t0)));
        let other: IpAddr = "2001:db8:2::1".parse().unwrap();
        g.release(other, admitted(g.admit_at(other, t0)));
        assert_eq!(g.rate_tracked(), MAX_TRACKED);
    }

    #[test]
    fn global_and_prefix48_drops_do_not_pollute_the_rate_table() {
        let g = gated(IpGuardConfig {
            global_connections: 1,
            per_v6_48_connections: 1,
            ..IpGuardConfig::default()
        });
        let t0 = Instant::now();
        assert_eq!(g.admit_at(ip6(1, 1), t0), Admission::Admit);
        assert_eq!(g.rate_tracked(), 1);
        assert_eq!(g.admit_at(ip(99), t0), Admission::Drop(DropReason::Global));
        assert_eq!(g.rate_tracked(), 1);
        g.release(ip6(1, 1), Admission::Admit);
    }

    #[test]
    fn the_failure_map_is_bounded_and_evicts_idle_entries_only() {
        let g = gated(IpGuardConfig::default());
        let t0 = Instant::now();
        // A live connection and an active ban must survive the eviction.
        assert_eq!(g.admit_at(ip(1), t0), Admission::Admit);
        for _ in 0..5 {
            g.record_failure_at(ip(2), t0);
        }
        assert!(g.is_blocked_at(ip(2), t0));
        for i in 0..(MAX_TRACKED as u32) {
            let addr = IpAddr::V4(Ipv4Addr::from(0x0b00_0000 + i));
            g.record_failure_at(addr, t0);
        }
        assert!(g.tracked() <= MAX_TRACKED, "{}", g.tracked());
        assert!(g.is_blocked_at(ip(2), t0), "the ban survived the eviction");
        assert!(
            g.tracked() <= MAX_TRACKED,
            "a failure never grows the map past its bound: {}",
            g.tracked()
        );
        g.release(ip(1), Admission::Admit);
        // Two entries were there before the flood (the live connection and
        // the ban), so the last two inserts each had to evict one idle entry
        // to stay under the bound.
        assert_eq!(g.take_counters().get("map-evict"), Some(&2));
        // The sweep is lazy: it runs before an insert that finds the map at
        // its bound (the release above left it one short). A window later
        // the next such insert drops the stale entries wholesale, keeping
        // the ban and the fresh failure.
        let later = t0 + Duration::from_secs(601);
        g.record_failure_at(ip(3), later);
        assert_eq!(g.tracked(), MAX_TRACKED, "one short, so no sweep yet");
        g.record_failure_at(ip(4), later);
        assert_eq!(g.tracked(), 3, "the ban, and the two fresh failures");
        assert!(g.is_blocked_at(ip(2), later));
    }

    #[test]
    fn exempt_networks_skip_the_per_network_caps_but_never_the_global_cap_or_the_bans() {
        let g = IpGuard::with_config(IpGuardConfig {
            max_conns: 1,
            per_ip_new_per_minute: 1,
            per_v6_48_connections: 1,
            global_connections: 6,
            exempt_nets: vec![
                net("127.0.0.0/8"),
                net("203.0.113.0/24"),
                net("2001:db8::/32"),
            ],
            ..IpGuardConfig::default()
        });
        let t0 = Instant::now();
        let home: IpAddr = "203.0.113.9".parse().unwrap();
        let localhost = IpAddr::V4(Ipv4Addr::LOCALHOST);
        // Three at once from one exempt address — past its concurrency cap
        // of one and its rate of one a minute — and two /64s of one exempt
        // /48 past its aggregate of one: all admitted, none counted against
        // any network, every one counted against the process.
        for _ in 0..3 {
            assert_eq!(g.admit_at(home, t0), Admission::Admit);
            assert!(g.conn_opened(home));
        }
        assert_eq!(g.admit_at(ip6(1, 1), t0), Admission::Admit);
        assert_eq!(g.admit_at(ip6(2, 1), t0), Admission::Admit);
        assert_eq!(g.tracked(), 0, "nothing per network");
        assert_eq!(g.rate_tracked(), 0, "nothing in the rate table");
        assert_eq!(g.live(), 5, "every exempt admission holds a global slot");
        // The global cap bounds the process, not the player: the last slot
        // goes to whoever asks, and past it exempt and not are refused
        // alike (the doorway's semantics).
        assert_eq!(g.admit_at(ip(1), t0), Admission::Admit);
        assert_eq!(g.admit_at(home, t0), Admission::Drop(DropReason::Global));
        assert_eq!(
            g.admit_at(localhost, t0),
            Admission::Drop(DropReason::Global)
        );
        assert_eq!(g.admit_at(ip(2), t0), Admission::Drop(DropReason::Global));
        // Exempt admissions are counted, not logged one by one.
        let counters = g.take_counters();
        assert_eq!(counters.get("exempt"), Some(&5));
        assert_eq!(counters.get("global"), Some(&3));
        // A release gives the global slot back; the concurrency-only close
        // for an exempt address touches nothing.
        g.release(home, Admission::Admit);
        g.conn_closed(home);
        assert_eq!(g.live(), 5);
        assert_eq!(g.admit_at(localhost, t0), Admission::Admit);
        assert_eq!(g.live(), 6);
        // But the password tempban bites an exempt address like any other,
        // and so does a siteban.
        for _ in 0..5 {
            g.record_failure_at(home, t0);
        }
        assert!(g.is_blocked_at(home, t0));
        g.set_siteban(localhost, None);
        assert!(g.is_blocked_at(localhost, t0));
    }

    #[test]
    fn the_concurrency_only_pair_is_untouched_by_the_rate_and_global_caps() {
        // The web and trusted ingresses count against the concurrency cap
        // alone: a browser tab's reconnect storm must never meet the SSH
        // rate (the 2026-09-20 lesson in the module docs).
        let g = gated(IpGuardConfig {
            per_ip_new_per_minute: 1,
            global_connections: 1,
            ..IpGuardConfig::default()
        });
        let t0 = Instant::now();
        assert_eq!(g.admit_at(ip(1), t0), Admission::Admit);
        for _ in 0..50 {
            assert!(g.conn_opened(ip(2)));
            g.conn_closed(ip(2));
        }
        // And the two pairs share one concurrency count per network.
        for _ in 0..7 {
            assert!(g.conn_opened(ip(1)));
        }
        assert!(!g.conn_opened(ip(1)));
        g.release(ip(1), Admission::Admit);
        assert!(g.conn_opened(ip(1)));
    }
}
