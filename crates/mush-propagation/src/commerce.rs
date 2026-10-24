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
    !e.state().is_terminal()
}
fn unspent(e: &Envelope) -> bool {
    pending(e) && e.path().is_empty()
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
                return crate::billing::class(w, m.operator, e.issuer()).map_or_else(
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
        if market
            .purchases
            .values()
            .filter(|p| p.issuer == actor && matches!(p.phase, Phase::Quoted | Phase::Reserved))
            .count()
            >= 32
        {
            return Err(fail(
                "Your 32 live quote/reservation slots are full; cancel unused quotes.",
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
        // Retire only terminal rows. Live reservations are never evicted for room.
        if market.purchases.len() >= MAX_PURCHASES {
            let old = market
                .purchases
                .iter()
                .find(|(_, p)| {
                    p.phase == Phase::Released
                        || (p.phase == Phase::Consumed
                            && ctx
                                .w
                                .module_state::<PropagationStore>()
                                .and_then(|s| s.envelope(p.envelope))
                                .is_none_or(|e| !pending(e)))
                })
                .map(|(id, _)| *id);
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
            return Err(fail("Quote expired or cancelled; obtain a fresh quote."));
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
