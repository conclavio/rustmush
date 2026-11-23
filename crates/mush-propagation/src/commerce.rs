//! R8a: bounded, quoted per-recipient relay sales. Money stays in the injected
//! economy owner. Capture and the provisional transmission share one engine batch.
use crate::{
    Envelope, PropagationStore,
    service::{self, Class},
};
use mush_core::{CanonicalId, CmdError, Ctx, ModuleOp, ModuleWrite, ObjId, World};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_PURCHASES: usize = 256;
pub const MAX_SETTLEMENTS: usize = 8;
/// Maximum live (`Phase::Quoted | Phase::Reserved`) purchase rows per actor.
pub const MAX_ACTOR_LIVE_PURCHASES: usize = 32;
/// Maximum unreserved (`Phase::Quoted`) purchase rows per actor (`P2-CORE-18`).
/// Kept well below [`MAX_PURCHASES`] so zero-balance actors cannot saturate the
/// global purchase table with free quotes.
pub const MAX_ACTOR_UNRESERVED_QUOTES: usize = 4;
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tariff {
    pub version: u16,
    pub standard_minor: i64,
    pub priority_minor: i64,
    pub quote_secs: i64,
    pub reservation_secs: i64,
}
impl Tariff {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || !(1..=1_000_000).contains(&self.standard_minor)
            || self.priority_minor <= self.standard_minor
            || self.priority_minor > 1_000_000
            || !(1..=3600).contains(&self.quote_secs)
            || !(3600..=604800).contains(&self.reservation_secs)
        {
            return Err("invalid relay retail tariff".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Quoted,
    Reserved,
    Consumed,
    Released,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Purchase {
    pub id: u64,
    pub envelope: u64,
    pub issuer: ObjId,
    pub recipient: ObjId,
    pub operator: u64,
    pub payee: u64,
    pub payer: u64,
    pub class: Class,
    pub bytes: u32,
    pub units: u32,
    pub tariff: u16,
    pub unit_minor: i64,
    pub total_minor: i64,
    pub quoted_at: i64,
    pub quote_until: i64,
    pub service_until: i64,
    pub phase: Phase,
    pub hold: Option<u64>,
    pub transaction: Option<u64>,
}
impl Purchase {
    fn validate(&self) -> bool {
        self.id > 0
            && self.tariff == 1
            && matches!(self.class, Class::Standard | Class::Priority)
            && (1..=1024).contains(&self.bytes)
            && self.units == self.bytes.div_ceil(128)
            && (1..=1_000_000).contains(&self.unit_minor)
            && self.unit_minor.checked_mul(i64::from(self.units)) == Some(self.total_minor)
            && self
                .quote_until
                .checked_sub(self.quoted_at)
                .is_some_and(|n| (1..=3600).contains(&n))
            && self
                .service_until
                .checked_sub(self.quoted_at)
                .is_some_and(|n| (3600..=604800).contains(&n))
            && self.quoted_at < self.quote_until
            && self.quote_until < self.service_until
            && match self.phase {
                Phase::Quoted => self.hold.is_none() && self.transaction.is_none(),
                Phase::Reserved => self.hold.is_some() && self.transaction.is_none(),
                Phase::Consumed => self.hold.is_some() && self.transaction.is_some(),
                Phase::Released => self.hold.is_some() == self.transaction.is_some(),
            }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Market {
    pub operator: u64,
    pub payee: u64,
    pub tariff: Tariff,
    /// Accepted pre-retail envelopes keep their noncommercial class semantics.
    pub legacy_before: u64,
    pub capital_transaction: Option<u64>,
    pub next: u64,
    pub purchases: BTreeMap<u64, Purchase>,
}
impl Market {
    pub fn validate(&self) -> Result<(), String> {
        self.tariff.validate()?;
        let mut active = BTreeSet::new();
        if self.next == 0
            || self.purchases.len() > MAX_PURCHASES
            || self.purchases.iter().any(|(id, p)| {
                *id != p.id
                    || *id >= self.next
                    || !p.validate()
                    || p.unit_minor
                        != if p.class == Class::Standard {
                            self.tariff.standard_minor
                        } else {
                            self.tariff.priority_minor
                        }
                    || (self.capital_transaction.is_none()
                        && p.phase != Phase::Quoted
                        && p.phase != Phase::Released)
                    || p.operator != self.operator
                    || p.payee != self.payee
                    || (matches!(p.phase, Phase::Reserved | Phase::Consumed)
                        && !active.insert(p.envelope))
            })
        {
            return Err("invalid relay purchases".into());
        }
        Ok(())
    }
    pub fn active(&self, envelope: u64) -> Option<&Purchase> {
        self.purchases.values().find(|p| {
            p.envelope == envelope && matches!(p.phase, Phase::Reserved | Phase::Consumed)
        })
    }
}
pub fn current(w: &World) -> Option<&Market> {
    w.module_state::<PropagationStore>()?
        .transport
        .service
        .as_ref()?
        .commerce
        .as_ref()
}
fn bytes(e: &Envelope) -> u32 {
    e.dispatch_ref()
        .map_or(e.payload().len() as u32, |d| d.bytes)
}
fn pending(e: &Envelope) -> bool {
    matches!(
        e.state(),
        crate::EnvelopeState::Queued | crate::EnvelopeState::Transmitted
    )
}
fn unspent(e: &Envelope) -> bool {
    pending(e)
        && e.path().is_empty()
        && e.transit()
            .is_none_or(|r| r.serialization.is_none() && r.admitted_operators.is_empty())
}
/// Whether an unreserved `Phase::Quoted` row has expired or its envelope is no
/// longer pending (`Queued | Transmitted`), so it holds no money and can be
/// evicted inline without economy settlement (`P2-CORE-18`).
fn quote_evictable(w: &World, p: &Purchase, now: i64) -> bool {
    p.phase == Phase::Quoted
        && (now >= p.quote_until
            || w.module_state::<PropagationStore>()
                .and_then(|s| s.envelope(p.envelope))
                .is_none_or(|e| !pending(e)))
}
pub(crate) fn persist(ctx: &mut Ctx<'_>) {
    let Some(book) = ctx.w.module_state::<PropagationStore>() else {
        tracing::error!("relay persistence requires the propagation store");
        return;
    };
    let value = book.transport.to_value();
    ctx.persist(ModuleOp {
        ns: crate::NS.into(),
        op: ModuleWrite::JsonSet {
            key: crate::routing::STATE_KEY.into(),
            value,
        },
    });
    ctx.out.pin_deferred();
    ctx.gate_on_durable_ack();
}
/// Boot/content owner handoff; no implicit replacement of a committed market.
pub fn install(ctx: &mut Ctx<'_>, operator: u64, payee: u64, tariff: Tariff) -> Result<(), String> {
    tariff.validate()?;
    if let Some(old) = current(ctx.w) {
        old.validate()?;
        if old.operator != operator || old.payee != payee || old.tariff != tariff {
            return Err("relay market identity/tariff drift; explicit migration required".into());
        }
        return Ok(());
    }
    let book = ctx
        .w
        .module_state_mut::<PropagationStore>()
        .ok_or("propagation unavailable")?;
    let legacy_before = book
        .allocator()
        .get("next_envelope")
        .and_then(|v| v.as_u64())
        .and_then(|last| last.checked_add(1))
        .ok_or("envelope allocator unavailable or exhausted")?;
    let state = book
        .transport
        .service
        .as_mut()
        .ok_or("service policy unavailable")?;
    state.commerce = Some(Market {
        operator,
        payee,
        tariff,
        legacy_before,
        capital_transaction: None,
        next: 1,
        purchases: BTreeMap::new(),
    });
    persist(ctx);
    Ok(())
}
/// Complete the once-only capitalization handoff in the money post's batch.
pub fn capitalized(ctx: &mut Ctx<'_>, transaction: u64) {
    let Some(market) = ctx
        .w
        .module_state_mut::<PropagationStore>()
        .and_then(|s| s.transport.service.as_mut())
        .and_then(|s| s.commerce.as_mut())
    else {
        tracing::error!("relay capitalization requires the installed market");
        return;
    };
    market.capital_transaction = Some(transaction);
    persist(ctx);
}
/// A purchased class never substitutes a payer identity for either endpoint.
pub fn base(state: &service::State, w: &World, pair: &CanonicalId, e: &Envelope) -> Class {
    if let Some(m) = &state.commerce {
        let exchange =
            service::integration(w).and_then(|api| api.operator(w, pair)) == Some(m.operator);
        if exchange {
            if let Some(p) = m.active(e.id()) {
                let valid = p.phase == Phase::Consumed
                    || (w.world_ic_now().at < p.service_until
                        && service::integration(w).is_some_and(|api| api.reserved(w, p)));
                return if valid { p.class } else { Class::Deferred };
            }
            if e.id() >= m.legacy_before {
                return crate::billing::class_as(w, m.operator, e.issuer(), e.representation())
                    .map_or_else(
                        || state.class(pair, e.issuer()),
                        |class| class.min(state.class(pair, e.issuer())),
                    );
            }
            return e.transit().and_then(|r| r.service.as_ref()).map_or_else(
                || {
                    state
                        .grants
                        .iter()
                        .find(|g| &g.pair == pair && g.principal == e.issuer())
                        .map_or(Class::Standard, |g| g.class)
                },
                |t| t.base_class.unwrap_or(t.class),
            );
        }
    }
    // A different operator keeps its own noncommercial default. Installing the
    // Exchange market cannot impose its free-tier schedule on a private link.
    e.transit().and_then(|r| r.service.as_ref()).map_or_else(
        || {
            state
                .grants
                .iter()
                .find(|g| &g.pair == pair && g.principal == e.issuer())
                .map_or(Class::Standard, |g| g.class)
        },
        |t| t.base_class.unwrap_or(t.class),
    )
}
/// The pure scheduler marks at most one new capture per pass. No money moves here.
pub fn consuming(
    book: &mut PropagationStore,
    w: &World,
    pair: &CanonicalId,
    envelope: u64,
) -> bool {
    let Some(m) = book
        .transport
        .service
        .as_mut()
        .and_then(|s| s.commerce.as_mut())
    else {
        return true;
    };
    if service::integration(w).and_then(|api| api.operator(w, pair)) != Some(m.operator) {
        return true;
    }
    let Some(p) = m
        .purchases
        .values_mut()
        .find(|p| p.envelope == envelope && p.phase == Phase::Reserved)
    else {
        return true;
    };
    if w.world_ic_now().at >= p.service_until
        || !service::integration(w).is_some_and(|api| api.reserved(w, p))
    {
        return false;
    }
    // transaction is filled only by settle_pass before this provisional book installs.
    p.phase = Phase::Consumed;
    true
}
/// One newly consumed purchase; all its ledger effects and the caller's route,
/// capacity and receipt post-images ride the same strict-durable WriteBatch.
pub fn settle_pass(ctx: &mut Ctx<'_>, book: &mut PropagationStore) -> bool {
    let pending: Vec<_> = book
        .transport
        .service
        .as_ref()
        .and_then(|s| s.commerce.as_ref())
        .into_iter()
        .flat_map(|m| m.purchases.values())
        .filter(|p| p.phase == Phase::Consumed && p.transaction.is_none())
        .cloned()
        .collect();
    if pending.is_empty() {
        return true;
    }
    if pending.len() != 1 {
        tracing::error!("relay retail pass exceeded settlement budget");
        return false;
    }
    let Some(api) = service::integration(ctx.w) else {
        return false;
    };
    let p = &pending[0];
    // Resolve the provisional destination before money moves. This mutable
    // borrow belongs to the local post-image, independent of the world adapter.
    let Some(purchase) = book
        .transport
        .service
        .as_mut()
        .and_then(|s| s.commerce.as_mut())
        .and_then(|m| m.purchases.get_mut(&p.id))
    else {
        tracing::error!(purchase = p.id, "relay settlement destination missing");
        return false;
    };
    match api.settle(ctx, p, true) {
        Ok(tx) => {
            purchase.transaction = Some(tx);
            true
        }
        Err(error) => {
            tracing::warn!(%error,purchase=p.id,"relay transmission waits for settlement");
            false
        }
    }
}
/// Bounded release work before both real delivery ingresses, including boot's
/// first fire. No expiry sweep in the economy may race consumed service.
pub fn sweep(ctx: &mut Ctx<'_>) {
    let Some(api) = service::integration(ctx.w) else {
        return;
    };
    let now = ctx.w.world_ic_now().at;
    let todo: Vec<_> = current(ctx.w)
        .into_iter()
        .flat_map(|m| m.purchases.values())
        .filter(|p| {
            matches!(p.phase, Phase::Quoted | Phase::Reserved)
                && (now
                    >= if p.phase == Phase::Quoted {
                        p.quote_until
                    } else {
                        p.service_until
                    }
                    || ctx
                        .w
                        .module_state::<PropagationStore>()
                        .and_then(|s| s.envelope(p.envelope))
                        .is_none_or(|e| !pending(e)))
        })
        .take(MAX_SETTLEMENTS)
        .cloned()
        .collect();
    for p in todo {
        let tx = if p.phase == Phase::Reserved {
            match api.settle(ctx, &p, false) {
                Ok(tx) => Some(tx),
                Err(error) => {
                    tracing::warn!(%error,purchase=p.id,"relay reservation release waits");
                    continue;
                }
            }
        } else {
            None
        };
        // The economy adapter cannot remove this owner's installed state.
        // Preserve the identified post-image without a second purchase lookup.
        let Some(market) = ctx
            .w
            .module_state_mut::<PropagationStore>()
            .and_then(|s| s.transport.service.as_mut())
            .and_then(|s| s.commerce.as_mut())
        else {
            tracing::error!(purchase = p.id, "relay release lost its installed market");
            return;
        };
        let mut released = p;
        released.phase = Phase::Released;
        released.transaction = tx;
        market.purchases.insert(released.id, released);
        persist(ctx);
    }
}
fn say(ctx: &mut Ctx<'_>, s: impl Into<String>) {
    ctx.out.sys_to_actor(ctx.actor, crate::NS, s.into());
}
fn line(p: &Purchase) -> String {
    format!(
        "Quote {}: envelope {} to {}; {} content bytes / {} units; {} at {}.{:02} cr/unit = {}.{:02} cr from account {}; tariff v{}. {:?}; quote until IC {}, unsent release IC {}. First Exchange transmission consumes the whole fee; no later refund.",
        p.id,
        p.envelope,
        p.recipient,
        p.bytes,
        p.units,
        p.class.label(),
        p.unit_minor / 100,
        p.unit_minor % 100,
        p.total_minor / 100,
        p.total_minor % 100,
        p.payer,
        p.tariff,
        p.phase,
        p.quote_until,
        p.service_until
    )
}
pub fn handles(ctx: &Ctx<'_>) -> bool {
    ["outbox", "quote", "buy", "cancel", "purchases"]
        .iter()
        .any(|s| ctx.switches.has(s))
}
pub fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let fail = |s: &str| CmdError::msg(s.to_string());
    let api = service::integration(ctx.w).ok_or_else(|| fail("Relay commerce unavailable."))?;
    let mut market = current(ctx.w)
        .cloned()
        .ok_or_else(|| fail("Relay commerce unavailable."))?;
    let now = ctx.w.world_ic_now().at;
    let actor = ctx.actor;
    if ctx.switches.has("outbox") {
        let rows: Vec<_> = ctx
            .w
            .module_state::<PropagationStore>()
            .into_iter()
            .flat_map(|s| s.envelopes())
            .filter(|e| e.issuer() == ctx.actor && unspent(e))
            .take(64)
            .map(|e| {
                format!(
                    "Envelope {} {:?} to {}: {} content bytes, {} units. {}",
                    e.id(),
                    e.kind(),
                    e.recipient(),
                    bytes(e),
                    service::units(e),
                    e.dispatch_ref().map_or("", |d| d.key.as_str())
                )
            })
            .collect();
        if rows.is_empty() {
            say(ctx, "No queued, unconsumed copies.");
        }
        for row in rows {
            say(ctx, row);
        }
        return Ok(());
    }
    if ctx.switches.has("purchases") {
        for p in market
            .purchases
            .values()
            .filter(|p| p.issuer == actor)
            .rev()
            .take(32)
        {
            say(ctx, line(p));
        }
        return Ok(());
    }
    let id = ctx
        .args
        .lhs()
        .trim()
        .trim_start_matches('#')
        .parse::<u64>()
        .map_err(|_| fail("Supply an envelope or quote number."))?;
    if ctx.switches.has("quote") {
        if market.capital_transaction.is_none() {
            return Err(fail(
                "The Exchange is awaiting its charter capital; paid quotes are unavailable.",
            ));
        }
        let e = ctx
            .w
            .module_state::<PropagationStore>()
            .and_then(|s| s.envelope(id))
            .filter(|e| e.issuer() == ctx.actor && unspent(e))
            .cloned()
            .ok_or_else(|| fail("No such unconsumed outgoing copy."))?;
        if market.active(id).is_some() {
            return Err(fail(
                "Cancel the unconsumed reservation before changing its terms.",
            ));
        }
        if !(1..=1024).contains(&bytes(&e)) {
            return Err(fail(
                "This copy uses conventional transport; the relay short limit is 1024 content bytes.",
            ));
        }
        let args = ctx.args.rhs().unwrap_or("");
        let (class, account) = args
            .split_once(',')
            .ok_or_else(|| fail("Usage: +relay/quote envelope=standard|priority,account"))?;
        let class = match class.trim() {
            "standard" => Class::Standard,
            "priority" => Class::Priority,
            _ => return Err(fail("Choose standard or priority.")),
        };
        let payer = account
            .trim()
            .trim_start_matches('#')
            .parse()
            .map_err(|_| fail("Supply an authorized account."))?;
        api.payer(ctx.w, ctx.actor, payer).map_err(CmdError::msg)?;
        // Enforce at most 1 live `Phase::Quoted` row per `(issuer, envelope)`
        // by replacing any prior quote for this copy in place, and retire this
        // actor's own expired/stale quotes before checking per-actor caps
        // (`P2-CORE-18`).
        market.purchases.retain(|_, p| {
            !(p.issuer == actor
                && p.phase == Phase::Quoted
                && (p.envelope == id || quote_evictable(ctx.w, p, now)))
        });
        if market
            .purchases
            .values()
            .filter(|p| p.issuer == actor && p.phase == Phase::Quoted)
            .count()
            >= MAX_ACTOR_UNRESERVED_QUOTES
        {
            return Err(fail(
                "Your unreserved quote slots are full; buy or cancel unused quotes.",
            ));
        }
        if market
            .purchases
            .values()
            .filter(|p| p.issuer == actor && matches!(p.phase, Phase::Quoted | Phase::Reserved))
            .count()
            >= MAX_ACTOR_LIVE_PURCHASES
        {
            return Err(fail(
                "Your 32 live quote/reservation slots are full; cancel unused quotes.",
            ));
        }
        // Retire terminal rows and expired/stale unreserved quotes inline when
        // at capacity. Live reservations are never evicted for room.
        if market.purchases.len() >= MAX_PURCHASES {
            let old = market
                .purchases
                .iter()
                .find(|(_, p)| {
                    p.phase == Phase::Released
                        || quote_evictable(ctx.w, p, now)
                        || (p.phase == Phase::Consumed
                            && ctx
                                .w
                                .module_state::<PropagationStore>()
                                .and_then(|s| s.envelope(p.envelope))
                                .is_none_or(|e| !pending(e)))
                })
                .map(|(qid, _)| *qid);
            if let Some(old) = old {
                market.purchases.remove(&old);
            } else {
                return Err(fail("Relay purchase capacity is full; retry later."));
            }
        }
        let quote = market.next;
        market.next = quote
            .checked_add(1)
            .ok_or_else(|| fail("Quote identities exhausted."))?;
        let unit_minor = if class == Class::Standard {
            market.tariff.standard_minor
        } else {
            market.tariff.priority_minor
        };
        let p = Purchase {
            id: quote,
            envelope: id,
            issuer: ctx.actor,
            recipient: e.recipient(),
            operator: market.operator,
            payee: market.payee,
            payer,
            class,
            bytes: bytes(&e),
            units: service::units(&e),
            tariff: market.tariff.version,
            unit_minor,
            total_minor: unit_minor
                .checked_mul(i64::from(service::units(&e)))
                .ok_or_else(|| fail("Quote exceeds money bounds."))?,
            quoted_at: now,
            quote_until: now
                .checked_add(market.tariff.quote_secs)
                .ok_or_else(|| fail("IC time exhausted."))?,
            service_until: now
                .checked_add(market.tariff.reservation_secs)
                .ok_or_else(|| fail("IC time exhausted."))?,
            phase: Phase::Quoted,
            hold: None,
            transaction: None,
        };
        market.purchases.insert(quote, p.clone());
        market.validate().map_err(CmdError::msg)?;
        ctx.w
            .module_state_mut::<PropagationStore>()
            .and_then(|s| s.transport.service.as_mut())
            .ok_or_else(|| fail("Relay service unavailable."))?
            .commerce = Some(market);
        persist(ctx);
        say(ctx, line(&p));
        return Ok(());
    }
    let mut p = market
        .purchases
        .get(&id)
        .filter(|p| p.issuer == ctx.actor)
        .cloned()
        .ok_or_else(|| fail("No such quote."))?;
    if ctx.switches.has("buy") {
        if matches!(p.phase, Phase::Reserved | Phase::Consumed) {
            say(ctx, line(&p));
            return Ok(());
        }
        if p.phase != Phase::Quoted || now >= p.quote_until {
            return Err(fail("Quote expired or canceled; obtain a fresh quote."));
        }
        if market.active(p.envelope).is_some() {
            return Err(fail("This copy already has a reservation."));
        }
        let e = ctx
            .w
            .module_state::<PropagationStore>()
            .and_then(|s| s.envelope(p.envelope))
            .filter(|e| {
                unspent(e)
                    && e.issuer() == p.issuer
                    && e.recipient() == p.recipient
                    && bytes(e) == p.bytes
            })
            .ok_or_else(|| fail("Copy no longer eligible; nothing reserved."))?;
        // Policy is applied independently again at transmission. This check does
        // not reveal a remote principal's restrictions or current location.
        let _ = e;
        let hold = api.reserve(ctx, &p).map_err(CmdError::msg)?;
        // No fallible work after the owner post. Same batch as its hold/journal.
        p.hold = Some(hold);
        p.phase = Phase::Reserved;
    } else {
        if p.phase == Phase::Consumed {
            return Err(fail(
                "Service already consumed; the tariff has no refund after transmission.",
            ));
        }
        if p.phase == Phase::Released {
            say(ctx, line(&p));
            return Ok(());
        }
        if p.phase == Phase::Reserved {
            p.transaction = Some(api.settle(ctx, &p, false).map_err(CmdError::msg)?);
        }
        p.phase = Phase::Released;
    }
    if let Some(installed) = ctx
        .w
        .module_state_mut::<PropagationStore>()
        .and_then(|s| s.transport.service.as_mut())
        .and_then(|s| s.commerce.as_mut())
    {
        installed.purchases.insert(id, p.clone());
    } else {
        // An installed owner slot cannot disappear during the synchronous
        // economy call; retain a diagnostic if composition violates that seam.
        tracing::error!(purchase = id, "relay purchase lost its installed market");
    }
    persist(ctx);
    say(ctx, line(&p));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EnvelopeState;
    use crate::routing::{Measurement, Metric, MetricProvider, RoutingServices};
    use crate::service::{
        Config as ServiceConfig, Evidence, Integration, Principal, Receipt,
        State as ServiceStateStore,
    };
    use mush_core::{
        ClockStamp, Engine, GOD, GameModule, ModError, ModuleActionState, ObjKind, Outbox,
        ParsedArgs, Registrar, Registry, SwitchSet,
    };
    use smol_str::SmolStr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Debug)]
    struct NoopMetric;
    impl Metric for NoopMetric {
        fn between(&self, _w: &World, _from: ObjId, _to: ObjId) -> Measurement {
            Measurement::Unknown
        }
    }

    #[derive(Debug)]
    struct NoopMetricProvider;
    impl MetricProvider for NoopMetricProvider {
        fn snapshot(&self, _w: &World, _at: &ClockStamp) -> Box<dyn Metric> {
            Box::new(NoopMetric)
        }
    }

    #[derive(Debug)]
    struct StubIntegration {
        next_hold: AtomicU64,
        next_tx: AtomicU64,
    }

    impl StubIntegration {
        fn new() -> Self {
            Self {
                next_hold: AtomicU64::new(100),
                next_tx: AtomicU64::new(500),
            }
        }
    }

    impl Integration for StubIntegration {
        fn payer(&self, _w: &World, _actor: ObjId, _account: u64) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _ctx: &mut Ctx<'_>, _p: &Purchase) -> Result<u64, String> {
            Ok(self.next_hold.fetch_add(1, Ordering::Relaxed))
        }
        fn reserved(&self, _w: &World, _p: &Purchase) -> bool {
            true
        }
        fn settle(&self, _ctx: &mut Ctx<'_>, _p: &Purchase, _capture: bool) -> Result<u64, String> {
            Ok(self.next_tx.fetch_add(1, Ordering::Relaxed))
        }
        fn seed(&self) -> Result<ServiceConfig, String> {
            Ok(ServiceConfig {
                version: 1,
                bytes_per_unit: 128,
                short_message_bytes: 1024,
                burst_units: 8,
                standard_delay_s: 0,
                deferred_min_s: 0,
                window_period_s: 86400,
                window_start_s: 0,
                window_duration_s: 3600,
                aging_s: 300,
            })
        }
        fn operator(&self, _w: &World, _pair: &CanonicalId) -> Option<u64> {
            Some(1)
        }
        fn principals(&self, _w: &World, endpoint: ObjId) -> Vec<Principal> {
            vec![Principal::Object(endpoint)]
        }
        fn valid_principal(&self, _w: &World, _p: &Principal) -> bool {
            true
        }
        fn visible(&self, _w: &World, _actor: ObjId, _receipt: &Receipt) -> bool {
            true
        }
        fn authorized(
            &self,
            _w: &World,
            _actor: ObjId,
            _pair: &CanonicalId,
            _action: &str,
        ) -> bool {
            true
        }
        fn discover(&self, _ctx: &mut Ctx<'_>, _evidence: &Evidence) -> Result<(), String> {
            Ok(())
        }
        fn publish(&self, _ctx: &mut Ctx<'_>, _evidence: &Evidence) -> Result<u64, String> {
            Ok(1)
        }
    }

    struct CommerceTestModule(Option<(PropagationStore, RoutingServices)>);
    impl GameModule for CommerceTestModule {
        fn name(&self) -> &'static str {
            crate::NS
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            if let Some((store, services)) = self.0.take() {
                reg.state(store);
                reg.state(services);
            }
            Ok(())
        }
    }

    fn setup_engine() -> (Engine, ObjId, ObjId, ObjId) {
        let mut store = PropagationStore::default();
        let cfg = StubIntegration::new().seed().expect("seed");
        let mut service = ServiceStateStore::new(cfg);
        service.commerce = Some(Market {
            operator: 1,
            payee: 2,
            tariff: Tariff {
                version: 1,
                standard_minor: 100,
                priority_minor: 500,
                quote_secs: 600,
                reservation_secs: 3600,
            },
            legacy_before: 1,
            capital_transaction: Some(1),
            next: 1,
            purchases: BTreeMap::new(),
        });
        store.transport.service = Some(service);
        let services = RoutingServices(
            Arc::new(NoopMetricProvider),
            Some(Arc::new(StubIntegration::new())),
        );
        let mut world = World::new_minimal();
        world.set_now(10_000);
        let alice = world
            .create(ObjKind::Player, "Alice", GOD, Some(mush_core::LIMBO))
            .expect("alice");
        let bob = world
            .create(ObjKind::Player, "Bob", GOD, Some(mush_core::LIMBO))
            .expect("bob");
        let carol = world
            .create(ObjKind::Player, "Carol", GOD, Some(mush_core::LIMBO))
            .expect("carol");
        let engine = Engine::with_modules(
            world,
            Registry::new(),
            vec![Box::new(CommerceTestModule(Some((store, services))))],
        )
        .expect("engine");
        (engine, alice, bob, carol)
    }

    fn add_envelope(
        engine: &mut Engine,
        issuer: ObjId,
        recipient: ObjId,
        state: EnvelopeState,
    ) -> u64 {
        let now = engine.world.world_ic_now();
        let store = engine
            .world
            .module_state_mut::<PropagationStore>()
            .expect("store");
        let id = store.alloc_envelope().expect("alloc");
        let mut env = Envelope::issue(id, issuer, recipient, "hello".into(), 1, now.clone())
            .expect("envelope");
        if state != EnvelopeState::Queued {
            env = env.in_state(state, now);
        }
        store.put_envelope(env).expect("put");
        id
    }

    fn run_commerce(
        engine: &mut Engine,
        actor: ObjId,
        switch: &str,
        raw: &str,
    ) -> Result<String, CmdError> {
        let mut out = Outbox::new();
        let mut ctx = Ctx {
            w: &mut engine.world,
            reg: &engine.registry,
            actor,
            switches: SwitchSet::from_names([switch]),
            args: ParsedArgs::parse(mush_core::ArgShape::EqPair, raw),
            out: &mut out,
            module: Some(SmolStr::new_static(crate::NS)),
            typed_state: ModuleActionState::unavailable(),
        };
        let res = command(&mut ctx);
        let text = out
            .take()
            .into_iter()
            .map(|(_, ev)| match ev {
                mush_proto::OutputEvent::Sys(rt) => rt.to_plain_text(),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        res.map(|()| text)
    }

    #[test]
    fn p2_core_18_repeated_quotes_replace_in_place_and_cap_unreserved() {
        let (mut engine, alice, bob, _) = setup_engine();
        let mut envs = Vec::new();
        for _ in 0..=MAX_ACTOR_UNRESERVED_QUOTES {
            envs.push(add_envelope(&mut engine, alice, bob, EnvelopeState::Queued));
        }

        // Repeatedly quoting the same envelope replaces the existing `Quoted`
        // row in place so `purchases.len()` stays 1 instead of exhausting slots.
        for class in ["standard", "priority", "standard", "priority"] {
            run_commerce(
                &mut engine,
                alice,
                "quote",
                &format!("{}= {class},1", envs[0]),
            )
            .expect("re-quote replaces in place");
            let m = current(&engine.world).expect("market");
            assert_eq!(m.purchases.len(), 1);
            let only = m.purchases.values().next().expect("quote");
            assert_eq!(only.envelope, envs[0]);
            assert_eq!(
                only.class,
                if class == "priority" {
                    Class::Priority
                } else {
                    Class::Standard
                }
            );
        }

        // Fill up to MAX_ACTOR_UNRESERVED_QUOTES distinct envelopes.
        for &env in &envs[1..MAX_ACTOR_UNRESERVED_QUOTES] {
            run_commerce(&mut engine, alice, "quote", &format!("{env}=standard,1"))
                .expect("within unreserved quote cap");
        }
        assert_eq!(
            current(&engine.world).expect("market").purchases.len(),
            MAX_ACTOR_UNRESERVED_QUOTES
        );

        // An additional distinct unreserved quote is refused.
        let extra = envs[MAX_ACTOR_UNRESERVED_QUOTES];
        let err = run_commerce(&mut engine, alice, "quote", &format!("{extra}=standard,1"))
            .expect_err("must enforce per-actor unreserved quote cap");
        assert!(err.to_string().contains("unreserved quote slots are full"));

        // Re-quoting an already-quoted envelope still succeeds in place at the cap.
        run_commerce(
            &mut engine,
            alice,
            "quote",
            &format!("{}=priority,1", envs[0]),
        )
        .expect("in-place replacement works even at unreserved cap");
        assert_eq!(
            current(&engine.world).expect("market").purchases.len(),
            MAX_ACTOR_UNRESERVED_QUOTES
        );
    }

    #[test]
    fn p2_core_18_expired_quotes_evicted_inline_at_max_purchases() {
        let (mut engine, alice, bob, carol) = setup_engine();
        let env_carol = add_envelope(&mut engine, carol, bob, EnvelopeState::Queued);
        let env_alice = add_envelope(&mut engine, alice, bob, EnvelopeState::Queued);

        // Saturate `market.purchases` to MAX_PURCHASES with expired `Phase::Quoted`
        // rows without running `sweep`.
        {
            let m = engine
                .world
                .module_state_mut::<PropagationStore>()
                .expect("store")
                .transport
                .service
                .as_mut()
                .expect("service")
                .commerce
                .as_mut()
                .expect("market");
            for i in 1..=(MAX_PURCHASES as u64) {
                m.purchases.insert(
                    i,
                    Purchase {
                        id: i,
                        envelope: env_alice,
                        issuer: alice,
                        recipient: bob,
                        operator: 1,
                        payee: 2,
                        payer: 1,
                        class: Class::Standard,
                        bytes: 5,
                        units: 1,
                        tariff: 1,
                        unit_minor: 100,
                        total_minor: 100,
                        quoted_at: 1_000,
                        quote_until: 1_600,
                        service_until: 4_600,
                        phase: Phase::Quoted,
                        hold: None,
                        transaction: None,
                    },
                );
            }
            m.next = (MAX_PURCHASES as u64) + 1;
        }

        // Now is 10_000 > quote_until (1_600): Carol's fresh quote evicts one
        // expired `Phase::Quoted` row inline and succeeds immediately.
        run_commerce(
            &mut engine,
            carol,
            "quote",
            &format!("{env_carol}=standard,1"),
        )
        .expect("expired Quoted row must be evicted inline at MAX_PURCHASES");
        let m = current(&engine.world).expect("market");
        assert_eq!(m.purchases.len(), MAX_PURCHASES);
        assert!(m.purchases.values().any(|p| p.issuer == carol));

        // By contrast, live `Phase::Reserved` rows are never evicted inline.
        let env_carol2 = add_envelope(&mut engine, carol, bob, EnvelopeState::Queued);
        {
            let m = engine
                .world
                .module_state_mut::<PropagationStore>()
                .expect("store")
                .transport
                .service
                .as_mut()
                .expect("service")
                .commerce
                .as_mut()
                .expect("market");
            m.purchases.clear();
            for i in 1..=(MAX_PURCHASES as u64) {
                m.purchases.insert(
                    i,
                    Purchase {
                        id: i,
                        envelope: 10_000 + i,
                        issuer: alice,
                        recipient: bob,
                        operator: 1,
                        payee: 2,
                        payer: 1,
                        class: Class::Standard,
                        bytes: 5,
                        units: 1,
                        tariff: 1,
                        unit_minor: 100,
                        total_minor: 100,
                        quoted_at: 9_900,
                        quote_until: 10_500,
                        service_until: 13_500,
                        phase: Phase::Reserved,
                        hold: Some(i),
                        transaction: None,
                    },
                );
            }
            m.next = (MAX_PURCHASES as u64) + 1;
        }
        let err = run_commerce(
            &mut engine,
            carol,
            "quote",
            &format!("{env_carol2}=standard,1"),
        )
        .expect_err("live Reserved rows must never be evicted inline");
        assert!(err.to_string().contains("capacity is full"));
    }

    #[test]
    fn p2_core_18_delivered_envelope_cannot_be_quoted_or_bought() {
        let (mut engine, alice, bob, _) = setup_engine();
        let delivered = add_envelope(&mut engine, alice, bob, EnvelopeState::Delivered);

        // 1. Delivered envelope fails `unspent` / `pending` and cannot be quoted or listed.
        let outbox = run_commerce(&mut engine, alice, "outbox", "").expect("outbox");
        assert!(
            !outbox.contains(&format!("Envelope {delivered}")),
            "{outbox}"
        );
        let err = run_commerce(
            &mut engine,
            alice,
            "quote",
            &format!("{delivered}=priority,1"),
        )
        .expect_err("Delivered envelope must not be quotable");
        assert!(err.to_string().contains("No such unconsumed outgoing copy"));

        // 2. An envelope quoted while `Queued` and then delivered locally before
        //    `+relay/buy` cannot be reserved, and `sweep` releases the quote.
        let env = add_envelope(&mut engine, alice, bob, EnvelopeState::Queued);
        run_commerce(&mut engine, alice, "quote", &format!("{env}=priority,1"))
            .expect("quote while Queued");
        let qid = current(&engine.world)
            .expect("market")
            .purchases
            .values()
            .find(|p| p.envelope == env)
            .expect("quote row")
            .id;

        let now = engine.world.world_ic_now();
        let delivered_copy = engine
            .world
            .module_state::<PropagationStore>()
            .expect("store")
            .envelope(env)
            .expect("env")
            .clone()
            .in_state(EnvelopeState::Delivered, now);
        engine
            .world
            .module_state_mut::<PropagationStore>()
            .expect("store")
            .put_envelope(delivered_copy)
            .expect("update");

        let buy_err = run_commerce(&mut engine, alice, "buy", &qid.to_string())
            .expect_err("Delivered envelope must not be buyable");
        assert!(buy_err.to_string().contains("Copy no longer eligible"));

        // Sweep releases the now-stale quote without waiting for `quote_until`.
        let mut out = Outbox::new();
        let mut ctx = Ctx {
            w: &mut engine.world,
            reg: &engine.registry,
            actor: GOD,
            switches: SwitchSet::default(),
            args: ParsedArgs::parse(mush_core::ArgShape::Raw, ""),
            out: &mut out,
            module: Some(SmolStr::new_static(crate::NS)),
            typed_state: ModuleActionState::unavailable(),
        };
        sweep(&mut ctx);
        assert_eq!(
            current(&engine.world).expect("market").purchases[&qid].phase,
            Phase::Released
        );
    }
}
