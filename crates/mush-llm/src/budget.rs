//! Daily budget ledger (design 03 §4.2). Layered per-scope counters over
//! tokens **and** cost, with UTC day rollover, a conservative pre-check, and an
//! authoritative settle.
//!
//! **Persistence seam (M7):** counters live in memory. The documented path to
//! durable budgets is to snapshot [`BudgetLedger::snapshot`] through a module
//! `ModuleOp::JsonSet` (the worldgen module or `mushd` owns the store handle)
//! and reload via [`BudgetLedger::restore`] at boot — mush-llm stays game-side
//! I/O-free. Nothing here blocks; critical sections are a `Mutex` with no
//! `.await` held.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

use crate::provider::BudgetScope;
use crate::types::Usage;

/// Validate a USD cost value (non-negative and finite). Returns `None` for
/// `NaN`, `±inf`, or negative values so malformed provider output can never
/// poison or neutralize budget ceilings.
#[must_use]
pub(crate) fn sanitize_cost_usd(cost: f64) -> Option<f64> {
    if !cost.is_finite() || cost < 0.0 {
        return None;
    }
    Some(if cost == 0.0 { 0.0 } else { cost })
}

/// A single budget ceiling. Either field may be `None` (unlimited on that
/// axis); both `None` is an unlimited budget.
#[derive(Copy, Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct BudgetLimit {
    /// Daily total-token ceiling (input + output).
    pub tokens: Option<u64>,
    /// Daily cost ceiling, USD.
    pub cost_usd: Option<f64>,
}

impl BudgetLimit {
    /// A token-only ceiling.
    #[must_use]
    pub fn tokens(n: u64) -> BudgetLimit {
        BudgetLimit {
            tokens: Some(n),
            cost_usd: None,
        }
    }
}

/// Running counters for one scope on one day.
#[derive(Copy, Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct Counters {
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens consumed.
    pub output_tokens: u64,
    /// Cost consumed, USD.
    pub cost_usd: f64,
}

impl Counters {
    /// Input + output tokens.
    #[must_use]
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    fn add(&mut self, usage: &Usage) {
        self.input_tokens = self.input_tokens.saturating_add(usage.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(usage.output_tokens);
        if let Some(c) = usage.cost_usd.and_then(sanitize_cost_usd) {
            self.cost_usd += c;
            if !self.cost_usd.is_finite() {
                self.cost_usd = f64::MAX;
            }
        }
    }
}

/// A scope key: which counter bucket a call touches.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ScopeKey {
    Global,
    Feature(String),
    Actor(ObjId),
    Npc(ObjId),
}

impl ScopeKey {
    fn scope(&self) -> BudgetScope {
        match self {
            ScopeKey::Global => BudgetScope::Global,
            ScopeKey::Feature(_) => BudgetScope::Feature,
            ScopeKey::Actor(_) => BudgetScope::Actor,
            ScopeKey::Npc(_) => BudgetScope::Npc,
        }
    }

    fn wire(&self) -> String {
        match self {
            ScopeKey::Global => "global".to_string(),
            ScopeKey::Feature(f) => format!("feature:{f}"),
            ScopeKey::Actor(a) => format!("actor:{}", a.0),
            ScopeKey::Npc(n) => format!("npc:{}", n.0),
        }
    }
}

/// The limits that apply to one call, assembled by the service from config +
/// ctx. A `None` limit means "no ceiling at this scope".
#[derive(Clone, Debug, Default)]
pub struct ScopeLimits {
    /// Global ceiling.
    pub global: Option<BudgetLimit>,
    /// Per-feature ceiling (with the feature's route name).
    pub feature: Option<(String, BudgetLimit)>,
    /// Per-actor ceiling (with the actor id).
    pub actor: Option<(ObjId, BudgetLimit)>,
    /// Per-NPC ceiling (with the NPC id).
    pub npc: Option<(ObjId, BudgetLimit)>,
}

impl ScopeLimits {
    fn entries(&self) -> Vec<(ScopeKey, BudgetLimit)> {
        let mut v = Vec::new();
        if let Some(l) = self.global {
            v.push((ScopeKey::Global, l));
        }
        if let Some((f, l)) = &self.feature {
            v.push((ScopeKey::Feature(f.clone()), *l));
        }
        if let Some((a, l)) = &self.actor {
            v.push((ScopeKey::Actor(*a), *l));
        }
        if let Some((n, l)) = &self.npc {
            v.push((ScopeKey::Npc(*n), *l));
        }
        v
    }
}

/// The daily budget ledger. Cheap `Clone` is deliberately not derived — the
/// service holds it behind an `Arc`.
pub struct BudgetLedger {
    inner: Mutex<Inner>,
    /// Injected day source (UTC day number). Overridable in tests to exercise
    /// rollover without waiting for midnight.
    day_of: Box<dyn Fn() -> u64 + Send + Sync>,
}

struct Inner {
    day: u64,
    counters: HashMap<ScopeKey, Counters>,
    /// Tokens reserved by currently in-flight calls (not yet settled), keyed
    /// by scope. Cleared on UTC day rollover alongside `counters`.
    reserved_tokens: HashMap<ScopeKey, u64>,
    /// Worst-case USD cost reserved by currently in-flight calls, keyed by scope.
    reserved_cost_usd: HashMap<ScopeKey, f64>,
    /// Active reservation count per scope so floating-point cost sums reset to
    /// exact zero when the last in-flight call on a scope drains.
    reserved_count: HashMap<ScopeKey, usize>,
}

/// RAII guard holding an atomic in-flight token and estimated-cost reservation
/// inside a [`BudgetLedger`].
///
/// Calling [`BudgetReservation::settle`] atomically releases the reservation
/// and adds authoritative [`Usage`] under a single lock acquisition. Dropping
/// the guard without settling (on transport error, cancellation, or abandoned
/// stream) releases the reservation automatically so budget capacity never leaks.
#[derive(Debug)]
pub struct BudgetReservation {
    ledger: Arc<BudgetLedger>,
    limits: ScopeLimits,
    reserved_day: u64,
    tokens: u64,
    cost_usd: f64,
    active: bool,
}

impl BudgetReservation {
    /// Atomically release this reservation and record the call's authoritative
    /// `usage` in one lock acquisition.
    pub fn settle(mut self, usage: &Usage) {
        let mut g = self.ledger.lock_inner();
        self.ledger.roll(&mut g);
        if self.active {
            self.active = false;
            if g.day == self.reserved_day {
                release_locked(&mut g, &self.limits, self.tokens, self.cost_usd);
            }
        }
        for (key, _limit) in self.limits.entries() {
            g.counters.entry(key).or_default().add(usage);
        }
    }
}

impl Drop for BudgetReservation {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        if let Ok(mut g) = self.ledger.inner.lock() {
            self.ledger.roll(&mut g);
            if g.day == self.reserved_day {
                release_locked(&mut g, &self.limits, self.tokens, self.cost_usd);
            }
        }
    }
}

fn release_locked(g: &mut Inner, limits: &ScopeLimits, tokens: u64, cost_usd: f64) {
    for (key, _limit) in limits.entries() {
        let next_count = g
            .reserved_count
            .get(&key)
            .copied()
            .unwrap_or(0)
            .saturating_sub(1);
        if next_count == 0 {
            g.reserved_count.remove(&key);
            g.reserved_tokens.remove(&key);
            g.reserved_cost_usd.remove(&key);
        } else {
            g.reserved_count.insert(key.clone(), next_count);
            if let Some(t) = g.reserved_tokens.get_mut(&key) {
                *t = t.saturating_sub(tokens);
            }
            if cost_usd > 0.0
                && let Some(c) = g.reserved_cost_usd.get_mut(&key)
            {
                *c = (*c - cost_usd).max(0.0);
            }
        }
    }
}

fn check_limits_locked(
    g: &Inner,
    entries: &[(ScopeKey, BudgetLimit)],
    estimated_tokens: u64,
    estimated_cost_usd: f64,
) -> Result<(), BudgetScope> {
    if !estimated_cost_usd.is_finite() || estimated_cost_usd < 0.0 {
        return Err(BudgetScope::Global);
    }
    for (key, limit) in entries {
        let used = g.counters.get(key).copied().unwrap_or_default();
        let res_tokens = g.reserved_tokens.get(key).copied().unwrap_or(0);
        let res_cost = g.reserved_cost_usd.get(key).copied().unwrap_or(0.0);
        if let Some(cap) = limit.tokens
            && used
                .total_tokens()
                .saturating_add(res_tokens)
                .saturating_add(estimated_tokens)
                > cap
        {
            return Err(key.scope());
        }
        if let Some(cap) = limit.cost_usd {
            let committed = used.cost_usd + res_cost;
            if !cap.is_finite()
                || cap < 0.0
                || !used.cost_usd.is_finite()
                || !res_cost.is_finite()
                || !committed.is_finite()
                || committed >= cap
                || (estimated_cost_usd > 0.0 && committed + estimated_cost_usd > cap)
            {
                return Err(key.scope());
            }
        }
    }
    Ok(())
}

impl std::fmt::Debug for BudgetLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.inner.lock().expect("budget lock");
        write!(
            f,
            "BudgetLedger(day={}, scopes={})",
            g.day,
            g.counters.len()
        )
    }
}

impl Default for BudgetLedger {
    fn default() -> BudgetLedger {
        BudgetLedger::new()
    }
}

impl BudgetLedger {
    /// A fresh ledger with real UTC-day rollover.
    #[must_use]
    pub fn new() -> BudgetLedger {
        BudgetLedger::with_day_source(Box::new(utc_day))
    }

    /// A ledger with an injected day source (tests).
    #[must_use]
    pub fn with_day_source(day_of: Box<dyn Fn() -> u64 + Send + Sync>) -> BudgetLedger {
        let day = day_of();
        BudgetLedger {
            inner: Mutex::new(Inner {
                day,
                counters: HashMap::new(),
                reserved_tokens: HashMap::new(),
                reserved_cost_usd: HashMap::new(),
                reserved_count: HashMap::new(),
            }),
            day_of,
        }
    }

    fn lock_inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn roll(&self, g: &mut Inner) {
        let today = (self.day_of)();
        if today != g.day {
            g.day = today;
            g.counters.clear();
            g.reserved_tokens.clear();
            g.reserved_cost_usd.clear();
            g.reserved_count.clear();
        }
    }

    /// Pre-flight check: would adding `estimated_tokens` push any applicable
    /// scope past its **token** ceiling (counting both settled spend and
    /// in-flight reservations), or is any scope already at/over its **cost**
    /// ceiling? Returns the tripped scope on refusal.
    pub fn pre_check(
        &self,
        limits: &ScopeLimits,
        estimated_tokens: u64,
    ) -> Result<(), BudgetScope> {
        self.pre_check_with_cost(limits, estimated_tokens, 0.0)
    }

    /// Pre-flight check including an optional worst-case `estimated_cost_usd`
    /// (0.0 when per-token pricing is not configured for the provider).
    pub fn pre_check_with_cost(
        &self,
        limits: &ScopeLimits,
        estimated_tokens: u64,
        estimated_cost_usd: f64,
    ) -> Result<(), BudgetScope> {
        let mut g = self.inner.lock().expect("budget lock");
        self.roll(&mut g);
        let entries = limits.entries();
        check_limits_locked(&g, &entries, estimated_tokens, estimated_cost_usd)
    }

    /// Atomically verify ceilings (counting settled spend + all active
    /// in-flight reservations) and reserve `estimated_tokens` and
    /// `estimated_cost_usd` across every applicable scope. Returns an RAII
    /// [`BudgetReservation`] guard that releases the reservation on settle or
    /// drop, eliminating concurrent TOCTOU budget overspend.
    pub fn reserve(
        self: &Arc<Self>,
        limits: ScopeLimits,
        estimated_tokens: u64,
        estimated_cost_usd: f64,
    ) -> Result<BudgetReservation, BudgetScope> {
        let mut g = self.lock_inner();
        self.roll(&mut g);
        let entries = limits.entries();
        check_limits_locked(&g, &entries, estimated_tokens, estimated_cost_usd)?;
        let clean_cost = sanitize_cost_usd(estimated_cost_usd).unwrap_or(0.0);
        for (key, _limit) in entries {
            let count = g.reserved_count.entry(key.clone()).or_insert(0);
            *count = count.saturating_add(1);
            let tok = g.reserved_tokens.entry(key.clone()).or_insert(0);
            *tok = tok.saturating_add(estimated_tokens);
            if clean_cost > 0.0 {
                let cost = g.reserved_cost_usd.entry(key).or_insert(0.0);
                *cost += clean_cost;
            }
        }
        Ok(BudgetReservation {
            ledger: Arc::clone(self),
            limits,
            reserved_day: g.day,
            tokens: estimated_tokens,
            cost_usd: clean_cost,
            active: true,
        })
    }

    /// Settle a completed call: add authoritative `usage` to every applicable
    /// scope. Called after a successful response (refusals/errors return no
    /// usage and are not settled).
    pub fn settle(&self, limits: &ScopeLimits, usage: &Usage) {
        let mut g = self.inner.lock().expect("budget lock");
        self.roll(&mut g);
        for (key, _limit) in limits.entries() {
            g.counters.entry(key).or_default().add(usage);
        }
    }

    /// Read the counters for the global scope (staff `@llm/budget`).
    #[must_use]
    pub fn global(&self) -> Counters {
        let mut g = self.inner.lock().expect("budget lock");
        self.roll(&mut g);
        g.counters
            .get(&ScopeKey::Global)
            .copied()
            .unwrap_or_default()
    }

    /// Read the counters for one feature.
    #[must_use]
    pub fn feature(&self, route: &str) -> Counters {
        let mut g = self.inner.lock().expect("budget lock");
        self.roll(&mut g);
        g.counters
            .get(&ScopeKey::Feature(route.to_string()))
            .copied()
            .unwrap_or_default()
    }

    /// Read the counters for one actor (builder).
    #[must_use]
    pub fn actor(&self, actor: ObjId) -> Counters {
        let mut g = self.inner.lock().expect("budget lock");
        self.roll(&mut g);
        g.counters
            .get(&ScopeKey::Actor(actor))
            .copied()
            .unwrap_or_default()
    }

    /// Read the counters for one NPC.
    #[must_use]
    pub fn npc(&self, npc: ObjId) -> Counters {
        let mut g = self.inner.lock().expect("budget lock");
        self.roll(&mut g);
        g.counters
            .get(&ScopeKey::Npc(npc))
            .copied()
            .unwrap_or_default()
    }

    /// Every feature-scoped counter for today (UTC), keyed by route name —
    /// the per-feature half of the cost dashboard's budget-vs-cap rows.
    /// Unlike [`BudgetLedger::feature`] (one named route), this returns every
    /// feature scope that has a recorded counter. The service records a counter
    /// for every feature it calls, capped or not (see
    /// [`LlmService::call`](crate::LlmService::call)'s `scope_limits`), so a
    /// feature with no configured cap still shows up once it has spent.
    #[must_use]
    pub fn feature_totals(&self) -> Vec<(String, Counters)> {
        let mut g = self.inner.lock().expect("budget lock");
        self.roll(&mut g);
        g.counters
            .iter()
            .filter_map(|(k, v)| match k {
                ScopeKey::Feature(f) => Some((f.clone(), *v)),
                _ => None,
            })
            .collect()
    }

    /// The top `n` actor (builder) scopes by cost today (UTC), descending
    /// (ties broken by total tokens) — the dashboard's "top builders by
    /// spend" rows. Aggregates only: counters, never prompt data.
    #[must_use]
    pub fn top_actors(&self, n: usize) -> Vec<(ObjId, Counters)> {
        self.top_scope(n, |k| match k {
            ScopeKey::Actor(a) => Some(*a),
            _ => None,
        })
    }

    /// The top `n` NPC scopes by cost today (UTC), descending — the
    /// dashboard's "top NPCs by spend" rows.
    #[must_use]
    pub fn top_npcs(&self, n: usize) -> Vec<(ObjId, Counters)> {
        self.top_scope(n, |k| match k {
            ScopeKey::Npc(a) => Some(*a),
            _ => None,
        })
    }

    fn top_scope(
        &self,
        n: usize,
        extract: impl Fn(&ScopeKey) -> Option<ObjId>,
    ) -> Vec<(ObjId, Counters)> {
        let mut g = self.inner.lock().expect("budget lock");
        self.roll(&mut g);
        let mut rows: Vec<(ObjId, Counters)> = g
            .counters
            .iter()
            .filter_map(|(k, v)| extract(k).map(|id| (id, *v)))
            .collect();
        rows.sort_by(|a, b| {
            b.1.cost_usd
                .partial_cmp(&a.1.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.1.total_tokens().cmp(&a.1.total_tokens()))
        });
        rows.truncate(n);
        rows
    }

    /// Snapshot every scope's counters as `(wire-key, counters)` pairs for
    /// persistence (the documented store seam).
    #[must_use]
    pub fn snapshot(&self) -> (u64, Vec<(String, Counters)>) {
        let mut g = self.inner.lock().expect("budget lock");
        self.roll(&mut g);
        let rows = g.counters.iter().map(|(k, v)| (k.wire(), *v)).collect();
        (g.day, rows)
    }

    /// Restore a snapshot taken by [`BudgetLedger::snapshot`] for the given
    /// day. Rows from a stale day are dropped (rollover already happened).
    pub fn restore(&self, day: u64, rows: Vec<(String, Counters)>) {
        let mut g = self.inner.lock().expect("budget lock");
        self.roll(&mut g);
        if g.day != day {
            return;
        }
        for (wire, mut counters) in rows {
            if let Some(key) = parse_wire(&wire) {
                if counters.cost_usd.is_nan() {
                    // Fail closed if a corrupted snapshot carried NaN.
                    counters.cost_usd = f64::INFINITY;
                } else if counters.cost_usd < 0.0 {
                    counters.cost_usd = 0.0;
                }
                g.counters.insert(key, counters);
            }
        }
    }
}

fn parse_wire(wire: &str) -> Option<ScopeKey> {
    if wire == "global" {
        return Some(ScopeKey::Global);
    }
    let (kind, rest) = wire.split_once(':')?;
    match kind {
        "feature" => Some(ScopeKey::Feature(rest.to_string())),
        "actor" => Some(ScopeKey::Actor(ObjId(rest.parse().ok()?))),
        "npc" => Some(ScopeKey::Npc(ObjId(rest.parse().ok()?))),
        _ => None,
    }
}

/// Days since the Unix epoch, UTC (86_400 s per day; leap seconds ignored,
/// which is correct for a calendar-day budget rollover).
fn utc_day() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn usage(input: u64, output: u64, cost: Option<f64>) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
            reasoning_tokens: 0,
            cost_usd: cost,
        }
    }

    #[test]
    fn pre_check_and_settle_accumulate() {
        let led = BudgetLedger::new();
        let limits = ScopeLimits {
            global: Some(BudgetLimit::tokens(300)),
            ..Default::default()
        };
        assert!(led.pre_check(&limits, 100).is_ok());
        led.settle(&limits, &usage(100, 100, Some(0.01)));
        assert_eq!(led.global().total_tokens(), 200);
        // 200 used + 150 estimate > 300 → refuse (global).
        assert_eq!(led.pre_check(&limits, 150), Err(BudgetScope::Global));
    }

    #[test]
    fn cost_ceiling_refuses_when_reached() {
        let led = BudgetLedger::new();
        let limits = ScopeLimits {
            global: Some(BudgetLimit {
                tokens: None,
                cost_usd: Some(1.0),
            }),
            ..Default::default()
        };
        led.settle(&limits, &usage(10, 10, Some(1.5)));
        assert_eq!(led.pre_check(&limits, 1), Err(BudgetScope::Global));
    }

    #[test]
    fn utc_rollover_clears_counters() {
        let day = Arc::new(AtomicU64::new(100));
        let d = Arc::clone(&day);
        let led = BudgetLedger::with_day_source(Box::new(move || d.load(Ordering::Relaxed)));
        let limits = ScopeLimits {
            global: Some(BudgetLimit::tokens(1_000_000)),
            ..Default::default()
        };
        led.settle(&limits, &usage(500, 0, None));
        assert_eq!(led.global().input_tokens, 500);
        // Next UTC day: counters reset.
        day.store(101, Ordering::Relaxed);
        assert_eq!(led.global().input_tokens, 0);
    }

    #[test]
    fn feature_totals_and_top_scopes_report_todays_spend() {
        let led = BudgetLedger::new();
        let alice = ObjId(10);
        let bob = ObjId(11);
        let npc = ObjId(20);

        // worldgen_room: global + feature + actor(alice).
        led.settle(
            &ScopeLimits {
                global: Some(BudgetLimit::tokens(1_000_000)),
                feature: Some(("worldgen_room".to_string(), BudgetLimit::tokens(1_000_000))),
                actor: Some((alice, BudgetLimit::tokens(1_000_000))),
                ..Default::default()
            },
            &usage(100, 50, Some(0.05)),
        );
        // npc_dialogue_major: feature + actor(bob, smaller spend) + npc.
        led.settle(
            &ScopeLimits {
                feature: Some((
                    "npc_dialogue_major".to_string(),
                    BudgetLimit::tokens(1_000_000),
                )),
                actor: Some((bob, BudgetLimit::tokens(1_000_000))),
                npc: Some((npc, BudgetLimit::tokens(1_000_000))),
                ..Default::default()
            },
            &usage(10, 10, Some(0.01)),
        );

        let features = led.feature_totals();
        assert_eq!(features.len(), 2);
        let wg = features
            .iter()
            .find(|(f, _)| f == "worldgen_room")
            .expect("worldgen_room feature scope");
        assert_eq!(wg.1.input_tokens, 100);

        // Alice outspent Bob → sorted first.
        let top_actors = led.top_actors(10);
        assert_eq!(top_actors.len(), 2);
        assert_eq!(top_actors[0].0, alice);
        assert!((top_actors[0].1.cost_usd - 0.05).abs() < 1e-9);
        assert_eq!(top_actors[1].0, bob);

        let top_npcs = led.top_npcs(10);
        assert_eq!(top_npcs, vec![(npc, led.npc(npc))]);

        // `n` truncates.
        assert_eq!(led.top_actors(1).len(), 1);
    }

    #[test]
    fn p2_core_11_reservation_raii_rollover_and_nan_cost_hardening() {
        let day = Arc::new(AtomicU64::new(10));
        let d = Arc::clone(&day);
        let led = Arc::new(BudgetLedger::with_day_source(Box::new(move || {
            d.load(Ordering::Relaxed)
        })));
        let limits = ScopeLimits {
            global: Some(BudgetLimit {
                tokens: Some(300),
                cost_usd: Some(1.0),
            }),
            ..Default::default()
        };

        // Two reservations consume 250 tokens / $0.80 -> third refused.
        let r1 = led.reserve(limits.clone(), 150, 0.40).unwrap();
        let r2 = led.reserve(limits.clone(), 100, 0.40).unwrap();
        assert_eq!(
            led.reserve(limits.clone(), 100, 0.10).unwrap_err(),
            BudgetScope::Global
        );
        assert_eq!(
            led.reserve(limits.clone(), 10, 0.30).unwrap_err(),
            BudgetScope::Global
        );

        // Dropping r2 releases its reservation cleanly.
        drop(r2);
        let r3 = led.reserve(limits.clone(), 100, 0.40).unwrap();

        // Settling r1 replaces its reservation with authoritative usage.
        r1.settle(&usage(25, 25, Some(0.10)));
        assert_eq!(led.global().total_tokens(), 50);

        // UTC day rollover clears old reservations; dropping r3 from the old
        // day must not corrupt the new day's counters or reservations.
        day.store(11, Ordering::Relaxed);
        let r_next_day = led.reserve(limits.clone(), 200, 0.50).unwrap();
        drop(r3);
        assert_eq!(
            led.reserve(limits.clone(), 150, 0.10).unwrap_err(),
            BudgetScope::Global
        );
        drop(r_next_day);

        // NaN and negative costs passed directly to settle never poison cost_usd.
        led.settle(&limits, &usage(10, 10, Some(f64::NAN)));
        led.settle(&limits, &usage(10, 10, Some(-10.0)));
        assert_eq!(led.global().cost_usd, 0.0);
        led.settle(&limits, &usage(10, 10, Some(1.0)));
        assert_eq!(led.pre_check(&limits, 1), Err(BudgetScope::Global));

        // Restoring a NaN snapshot fails closed on cost checks.
        let led2 = BudgetLedger::with_day_source(Box::new(|| 42));
        led2.restore(
            42,
            vec![(
                "global".to_string(),
                Counters {
                    input_tokens: 0,
                    output_tokens: 0,
                    cost_usd: f64::NAN,
                },
            )],
        );
        assert_eq!(led2.pre_check(&limits, 1), Err(BudgetScope::Global));
    }
}
