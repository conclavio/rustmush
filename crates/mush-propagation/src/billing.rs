//! R8b: bounded recurring relay products. The injected economy owns obligations,
//! clocked collection and cure receipts. Payer identity never becomes an endpoint.
use crate::{
    PropagationStore,
    service::{self, Class, Principal, ServiceState},
};
use mush_core::{CmdError, Ctx, ObjId, World};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SUBSCRIPTIONS: usize = 128;
pub const MAX_TREATIES: usize = 32;
fn money(minor: i64) -> String {
    format!("{}.{:02} cr", minor / 100, minor % 100)
}
fn period(secs: i64) -> String {
    if secs % 86400 == 0 {
        format!("{} IC day(s)", secs / 86400)
    } else if secs % 3600 == 0 {
        format!("{} IC hour(s)", secs / 3600)
    } else {
        format!("{secs} IC seconds")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tariff {
    pub version: u16,
    pub period_secs: i64,
    pub standard_minor: i64,
    pub priority_minor: i64,
    pub sovereign_minor: i64,
    pub quote_secs: i64,
}
impl Tariff {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || !(3600..=2592000).contains(&self.period_secs)
            || !(1..=1_000_000).contains(&self.standard_minor)
            || self.priority_minor <= self.standard_minor
            || self.sovereign_minor <= self.priority_minor
            || self.sovereign_minor > 100_000_000
            || !(1..=3600).contains(&self.quote_secs)
        {
            return Err("invalid relay subscription tariff".into());
        }
        Ok(())
    }
    fn price(&self, class: Class) -> Option<i64> {
        match class {
            Class::Standard => Some(self.standard_minor),
            Class::Priority => Some(self.priority_minor),
            Class::Sovereign => Some(self.sovereign_minor),
            Class::Deferred => None,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Quoted,
    Active,
    Cancelled,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub paid: bool,
    pub missed: u32,
    pub cured: u32,
    pub paused: bool,
    pub terminal: bool,
}
impl View {
    pub fn state(&self) -> ServiceState {
        if self.cured > self.missed || self.paused || self.missed - self.cured >= 2 {
            ServiceState::Suspended
        } else if self.missed > self.cured {
            ServiceState::Throttled
        } else {
            ServiceState::Served
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subscription {
    pub id: u64,
    pub issuer: ObjId,
    pub principal: Principal,
    pub payer: u64,
    pub class: Class,
    pub amount_minor: i64,
    pub quoted_at: i64,
    pub quote_until: i64,
    pub until_ic: Option<i64>,
    pub phase: Phase,
    pub obligation: Option<u64>,
    pub observed: Option<View>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Treaty {
    pub organization: u64,
    pub actor: ObjId,
    pub at: i64,
    pub until_ic: i64,
    pub reason: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Book {
    pub tariff: Tariff,
    pub next: u64,
    pub cursor: u64,
    pub subscriptions: BTreeMap<u64, Subscription>,
    pub treaties: BTreeMap<u64, Treaty>,
}
impl Book {
    pub fn validate(&self) -> Result<(), String> {
        self.tariff.validate()?;
        let mut active = BTreeSet::new();
        let mut obligations = BTreeSet::new();
        if self.next == 0
            || self.subscriptions.len() > MAX_SUBSCRIPTIONS
            || self.treaties.len() > MAX_TREATIES
            || self.subscriptions.iter().any(|(id, s)| {
                *id != s.id
                    || *id >= self.next
                    || s.obligation.is_some_and(|id| !obligations.insert(id))
                    || self.tariff.price(s.class) != Some(s.amount_minor)
                    || s.quote_until.checked_sub(s.quoted_at) != Some(self.tariff.quote_secs)
                    || (s.class == Class::Sovereign) != s.until_ic.is_some()
                    || s.until_ic.is_some_and(|end| end <= s.quoted_at)
                    || (s.class == Class::Sovereign
                        && !matches!(s.principal, Principal::Organization(_)))
                    || (s.phase == Phase::Quoted && s.obligation.is_some())
                    || (s.phase == Phase::Active
                        && (s.obligation.is_none() || !active.insert(&s.principal)))
                    || s.observed.as_ref().is_some_and(|v| v.cured > v.missed)
            })
            || self.treaties.iter().any(|(id, t)| {
                *id != t.organization
                    || t.until_ic <= t.at
                    || t.reason.trim().is_empty()
                    || t.reason.len() > 160
                    || t.reason.chars().any(char::is_control)
            })
        {
            return Err("invalid relay subscription/treaty records".into());
        }
        Ok(())
    }
}
pub fn current(w: &World) -> Option<&Book> {
    w.module_state::<PropagationStore>()?
        .transport
        .service
        .as_ref()?
        .billing
        .as_ref()
}
pub fn install(ctx: &mut Ctx<'_>, tariff: Tariff) -> Result<(), String> {
    tariff.validate()?;
    if let Some(old) = current(ctx.w) {
        old.validate()?;
        if old.tariff != tariff {
            return Err("relay subscription tariff drift; explicit migration required".into());
        }
        return Ok(());
    }
    save(
        ctx,
        Book {
            tariff,
            next: 1,
            cursor: 0,
            subscriptions: BTreeMap::new(),
            treaties: BTreeMap::new(),
        },
    );
    Ok(())
}
fn save(ctx: &mut Ctx<'_>, book: Book) {
    let Some(service) = ctx
        .w
        .module_state_mut::<PropagationStore>()
        .and_then(|s| s.transport.service.as_mut())
    else {
        tracing::error!("relay billing requires the installed service");
        return;
    };
    service.billing = Some(book);
    crate::commerce::persist(ctx);
}
fn view(w: &World, s: &Subscription) -> View {
    if s.obligation.is_none() {
        return View::default();
    }
    service::integration(w)
        .and_then(|api| api.subscription(w, s).ok())
        .unwrap_or(View {
            paused: true,
            ..View::default()
        })
}
fn exchange(w: &World, operator: u64) -> bool {
    crate::commerce::current(w).is_some_and(|m| m.operator == operator)
}
pub fn status(w: &World, operator: u64, principals: &[Principal]) -> ServiceState {
    if !exchange(w, operator) {
        return ServiceState::Served;
    }
    current(w)
        .into_iter()
        .flat_map(|b| b.subscriptions.values())
        .filter(|s| s.obligation.is_some() && principals.contains(&s.principal))
        .map(|s| view(w, s).state())
        .max()
        .unwrap_or_default()
}
pub fn class(w: &World, operator: u64, endpoint: ObjId) -> Option<Class> {
    class_as(w, operator, endpoint, None)
}
pub fn class_as(
    w: &World,
    operator: u64,
    endpoint: ObjId,
    representation: Option<mush_core::RelayRepresentation>,
) -> Option<Class> {
    if !exchange(w, operator) {
        return None;
    }
    let principals = crate::access::principals(w, endpoint, representation)?;
    current(w)?
        .subscriptions
        .values()
        .filter(|s| {
            s.phase == Phase::Active
                && principals.contains(&s.principal)
                && s.until_ic.is_none_or(|end| w.world_ic_now().at < end)
        })
        .filter(|s| {
            let v = view(w, s);
            v.paid && !v.terminal && !v.paused
        })
        .map(|s| s.class)
        .min()
}
pub fn status_lines(w: &World, operator: u64, principals: &[Principal]) -> Vec<String> {
    if !exchange(w, operator) {
        return vec![];
    }
    current(w).into_iter().flat_map(|b|b.subscriptions.values())
        .filter(|s|s.obligation.is_some() && principals.contains(&s.principal))
        .take(32).map(|s|{let v=view(w,s);format!("billing.subscription.{} {:?}: {} unpaid installment(s); +relay/cure {}. Other sanctions are independent.",s.id,v.state(),v.missed.saturating_sub(v.cured),s.id)}).collect()
}
pub fn board(w: &World) -> Option<String> {
    let t = &current(w)?.tariff;
    Some(format!(
        "Subscriptions v{} per {}: Standard {} / Priority {} / Sovereign treaty {}. Short outgoing copies only; capacity and both endpoint policies still apply. First actual payment enables service; one unpaid installment throttles, two suspend. +relay/subscribe #person|org:id=class,account; +relay/enroll quote; +relay/subscriptions; +relay/cure id; +relay/unsubscribe id. Cancellation ends future bills, never old debt. Sovereign requires an Exchange treaty and expires with its term.",
        t.version,
        period(t.period_secs),
        money(t.standard_minor),
        money(t.priority_minor),
        money(t.sovereign_minor)
    ))
}
pub fn handles(ctx: &Ctx<'_>) -> bool {
    [
        "subscribe",
        "enroll",
        "subscriptions",
        "unsubscribe",
        "cure",
        "treaty",
    ]
    .iter()
    .any(|s| ctx.switches.has(s))
}
fn say(ctx: &mut Ctx<'_>, s: impl Into<String>) {
    ctx.out.sys_to_actor(ctx.actor, crate::NS, s.into());
}
fn audit(ctx: &mut Ctx<'_>, text: String) {
    let actor = ctx.actor;
    let now = ctx.w.world_ic_now().at;
    if let Some(state) = ctx
        .w
        .module_state_mut::<PropagationStore>()
        .and_then(|s| s.transport.service.as_mut())
    {
        state.audit(actor, now, text);
    }
}
fn line(w: &World, s: &Subscription, t: &Tariff) -> String {
    let v = view(w, s);
    format!(
        "Subscription {} for {}: {:?}, {} per {} from account {}; {}, obligation {:?}, paid={}, {:?}, missed={}, cured={}, term end {:?}. Quote expires IC {}. First collection is due at enrollment; later collections follow full IC periods. Unpaid periods remain owed after cancellation. Downtime catch-up is bounded by the economy; no charge after treaty expiry.",
        s.id,
        s.principal.label(),
        s.class,
        money(s.amount_minor),
        period(t.period_secs),
        s.payer,
        if s.phase == Phase::Active && v.terminal {
            "Ended".to_owned()
        } else {
            format!("{:?}", s.phase)
        },
        s.obligation,
        v.paid,
        v.state(),
        v.missed,
        v.cured,
        s.until_ic,
        s.quote_until
    )
}
pub fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let fail = |s: &str| CmdError::msg(s.to_owned());
    let api =
        service::integration(ctx.w).ok_or_else(|| fail("Relay subscriptions unavailable."))?;
    let mut book = current(ctx.w)
        .cloned()
        .ok_or_else(|| fail("Relay subscriptions unavailable."))?;
    let now = ctx.w.world_ic_now().at;
    if ctx.switches.has("subscriptions") {
        let rows: Vec<_> = book
            .subscriptions
            .values()
            .filter(|s| s.issuer == ctx.actor || api.represents(ctx.w, ctx.actor, &s.principal))
            .rev()
            .take(32)
            .map(|s| line(ctx.w, s, &book.tariff))
            .collect();
        if rows.is_empty() {
            say(ctx, "No relay subscriptions or quotes.");
        }
        for row in rows {
            say(ctx, row);
        }
        return Ok(());
    }
    if ctx.switches.has("treaty") {
        let principal = Principal::parse(ctx.args.lhs().trim())
            .ok_or_else(|| fail("Usage: +relay/treaty org:id=days,reason"))?;
        let Principal::Organization(org) = principal else {
            return Err(fail(
                "A Sovereign treaty names a recognized government institution.",
            ));
        };
        let pair = crate::routing::registry(ctx.w)
            .and_then(|r| {
                r.pairs
                    .keys()
                    .find(|id| api.operator(ctx.w, id) == Some(r.operator.0))
            })
            .cloned()
            .ok_or_else(|| fail("Exchange unavailable."))?;
        if ctx.args.rhs().is_none() {
            if !api.represents(ctx.w, ctx.actor, &Principal::Organization(org))
                && !api.authorized(ctx.w, ctx.actor, &pair, "relay.manage")
            {
                return Err(fail("Permission denied."));
            }
            let treaty = book
                .treaties
                .get(&org)
                .ok_or_else(|| fail("No treaty offer for this principal."))?;
            say(
                ctx,
                format!(
                    "Treaty offer org:{org}, IC {}..{}: {}. {}",
                    treaty.at,
                    treaty.until_ic,
                    treaty.reason,
                    if now < treaty.until_ic {
                        "Current"
                    } else {
                        "Expired"
                    }
                ),
            );
            return Ok(());
        }
        if !api.authorized(ctx.w, ctx.actor, &pair, "relay.manage") {
            return Err(fail("Permission denied."));
        }
        if !api.government(ctx.w, org) {
            return Err(fail(
                "No recognized government authority for Sovereign service.",
            ));
        }
        let (days, reason) = ctx
            .args
            .rhs()
            .unwrap_or("")
            .split_once(',')
            .ok_or_else(|| fail("Usage: +relay/treaty org:id=days,reason"))?;
        let days = days
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|d| (2..=365).contains(d))
            .ok_or_else(|| fail("Treaty term must be 2..365 IC days, leaving time to accept a full billing period."))?;
        let reason = reason.trim().to_owned();
        if reason.is_empty() || reason.len() > 160 || reason.chars().any(char::is_control) {
            return Err(fail("Supply a short, single-line treaty reason."));
        }
        if book.treaties.get(&org).is_some_and(|t| t.until_ic > now) {
            return Err(fail(
                "A current treaty is immutable; use operator policy for a separate sanction.",
            ));
        }
        if !book.treaties.contains_key(&org) && book.treaties.len() >= MAX_TREATIES {
            return Err(fail("Treaty capacity is full."));
        }
        let until_ic = now
            .checked_add(days * 86400)
            .ok_or_else(|| fail("IC time exhausted."))?;
        book.treaties.insert(
            org,
            Treaty {
                organization: org,
                actor: ctx.actor,
                at: now,
                until_ic,
                reason,
            },
        );
        audit(
            ctx,
            format!("Sovereign treaty offer org:{org} until IC {until_ic}"),
        );
        save(ctx, book);
        say(
            ctx,
            format!(
                "Treaty offered to org:{org} until IC {until_ic}; its authorized representative must accept quoted Sovereign subscription terms."
            ),
        );
        return Ok(());
    }
    if ctx.switches.has("subscribe") {
        if crate::commerce::current(ctx.w).is_none_or(|m| m.capital_transaction.is_none()) {
            return Err(fail("Exchange charter capital pending."));
        }
        let principal = Principal::parse(ctx.args.lhs().trim())
            .filter(|p| api.represents(ctx.w, ctx.actor, p))
            .ok_or_else(|| fail("No such represented IC principal."))?;
        let (class, payer) = ctx
            .args
            .rhs()
            .unwrap_or("")
            .split_once(',')
            .ok_or_else(|| {
                fail("Usage: +relay/subscribe #person|org:id=standard|priority|sovereign,account")
            })?;
        let class = Class::parse(class.trim())
            .filter(|c| *c != Class::Deferred)
            .ok_or_else(|| fail("Choose a paid subscription class."))?;
        let payer = payer
            .trim()
            .trim_start_matches('#')
            .parse()
            .map_err(|_| fail("Supply an authorized payer account."))?;
        api.payer(ctx.w, ctx.actor, payer).map_err(CmdError::msg)?;
        if book.subscriptions.values().any(|s| {
            s.principal == principal
                && (s.phase == Phase::Active
                    || (s.obligation.is_some() && view(ctx.w, s).state() != ServiceState::Served))
        }) {
            return Err(fail(
                "Cancel or cure this principal's existing subscription first.",
            ));
        }
        if book
            .subscriptions
            .values()
            .filter(|s| s.issuer == ctx.actor && s.phase == Phase::Quoted && s.quote_until > now)
            .count()
            >= 8
        {
            return Err(fail("Your eight live subscription quotes are full."));
        }
        let until_ic = if class == Class::Sovereign {
            let Principal::Organization(org) = principal else {
                return Err(fail("Sovereign is only available by government treaty."));
            };
            if !api.government(ctx.w, org) {
                return Err(fail("No recognized government authority."));
            }
            Some(
                book.treaties
                    .get(&org)
                    .filter(|t| {
                        now.checked_add(book.tariff.period_secs)
                            .is_some_and(|start| t.until_ic > start)
                    })
                    .ok_or_else(|| {
                        fail("No treaty with at least one full billing period remaining.")
                    })?
                    .until_ic,
            )
        } else {
            None
        };
        if book.subscriptions.len() >= MAX_SUBSCRIPTIONS {
            let old = book
                .subscriptions
                .iter()
                .find(|(_, s)| {
                    (s.phase == Phase::Quoted && s.quote_until <= now)
                        || (s.phase == Phase::Cancelled
                            && view(ctx.w, s).state() == ServiceState::Served)
                })
                .map(|(id, _)| *id);
            if let Some(id) = old {
                book.subscriptions.remove(&id);
            } else {
                return Err(fail("Subscription capacity is full."));
            }
        }
        let id = book.next;
        book.next = id
            .checked_add(1)
            .ok_or_else(|| fail("Subscription identities exhausted."))?;
        let s = Subscription {
            id,
            issuer: ctx.actor,
            principal,
            payer,
            class,
            amount_minor: book
                .tariff
                .price(class)
                .ok_or_else(|| fail("Choose a paid subscription class."))?,
            quoted_at: now,
            quote_until: now
                .checked_add(book.tariff.quote_secs)
                .ok_or_else(|| fail("IC time exhausted."))?,
            until_ic,
            phase: Phase::Quoted,
            obligation: None,
            observed: None,
        };
        let reply = line(ctx.w, &s, &book.tariff);
        book.subscriptions.insert(id, s);
        book.validate().map_err(CmdError::msg)?;
        save(ctx, book);
        say(ctx, reply);
        return Ok(());
    }
    let id = ctx
        .args
        .lhs()
        .trim()
        .parse::<u64>()
        .map_err(|_| fail("Supply a subscription quote/id."))?;
    let mut s = book
        .subscriptions
        .get(&id)
        .filter(|s| api.represents(ctx.w, ctx.actor, &s.principal))
        .cloned()
        .ok_or_else(|| fail("No such represented subscription."))?;
    if ctx.switches.has("enroll") {
        if s.phase == Phase::Active {
            say(ctx, line(ctx.w, &s, &book.tariff));
            return Ok(());
        }
        if s.phase != Phase::Quoted || now >= s.quote_until {
            return Err(fail("Subscription quote expired or cancelled."));
        }
        if book.subscriptions.values().any(|other| {
            other.id != s.id
                && other.principal == s.principal
                && (other.phase == Phase::Active
                    || (other.obligation.is_some()
                        && view(ctx.w, other).state() != ServiceState::Served))
        }) {
            return Err(fail(
                "An existing subscription or arrears must be resolved first.",
            ));
        }
        if s.class == Class::Sovereign
            && s.until_ic.is_none_or(|end| {
                now.checked_add(book.tariff.period_secs)
                    .is_none_or(|start| end <= start)
            })
        {
            return Err(fail(
                "Treaty has less than a full period remaining; requote.",
            ));
        }
        let obligation = api.enroll(ctx, &s, &book.tariff).map_err(CmdError::msg)?;
        s.obligation = Some(obligation);
        s.phase = Phase::Active;
        audit(
            ctx,
            format!(
                "subscription {id} enrolled for {}; obligation {obligation}",
                s.principal.label()
            ),
        );
    } else if ctx.switches.has("unsubscribe") {
        if s.phase != Phase::Cancelled && s.obligation.is_some() {
            api.cancel_subscription(ctx, &s).map_err(CmdError::msg)?;
        }
        s.phase = Phase::Cancelled;
        audit(
            ctx,
            format!("subscription {id} cancelled; existing arrears remain"),
        );
    } else if ctx.switches.has("cure") {
        let v = api.subscription(ctx.w, &s).map_err(CmdError::msg)?;
        if let Some(rhs) = ctx.args.rhs() {
            let (missed, cured) = rhs
                .split_once(',')
                .ok_or_else(|| fail("Use the confirmation printed by +relay/cure id."))?;
            let missed = missed
                .trim()
                .parse()
                .map_err(|_| fail("Invalid cure quote."))?;
            let cured = cured
                .trim()
                .parse()
                .map_err(|_| fail("Invalid cure quote."))?;
            let receipt = api
                .cure_subscription(ctx, &s, missed, cured)
                .map_err(CmdError::msg)?;
            audit(
                ctx,
                format!("subscription {id} arrears paid: transaction {receipt}"),
            );
            save(ctx, book);
            say(
                ctx,
                format!(
                    "Arrears paid; transaction {receipt}. Incident sanctions and anathema remain independent."
                ),
            );
            return Ok(());
        }
        let amount = i64::from(v.missed.saturating_sub(v.cured))
            .checked_mul(s.amount_minor)
            .ok_or_else(|| fail("Arrears exceed money bounds."))?;
        say(
            ctx,
            format!(
                "Subscription {id}: {} owed for {} missed installments from account {}. Confirm +relay/cure {id}={},{}. A changed debt refuses this quote; cancellation/expiry is not reversed and other sanctions are not cleared.",
                money(amount),
                v.missed.saturating_sub(v.cured),
                s.payer,
                v.missed,
                v.cured
            ),
        );
        return Ok(());
    }
    let reply = line(ctx.w, &s, &book.tariff);
    book.subscriptions.insert(id, s);
    save(ctx, book);
    say(ctx, reply);
    Ok(())
}
/// At most 32 reads and eight transition notices per fire; routing reads the live
/// money-owned status directly, so the observer cursor never delays a restriction.
pub fn maintain(ctx: &mut Ctx<'_>) {
    let Some(mut book) = current(ctx.w).cloned() else {
        return;
    };
    let Some(api) = service::integration(ctx.w) else {
        return;
    };
    if book.subscriptions.is_empty() {
        return;
    }
    let ids: Vec<_> = book
        .subscriptions
        .range((
            std::ops::Bound::Excluded(book.cursor),
            std::ops::Bound::Unbounded,
        ))
        .chain(book.subscriptions.range(..=book.cursor))
        .take(32)
        .map(|(id, _)| *id)
        .collect();
    let mut changes = 0;
    for id in ids {
        book.cursor = id;
        let Some(s) = book.subscriptions.get_mut(&id) else {
            continue;
        };
        if s.obligation.is_none() {
            continue;
        }
        let v = match api.subscription(ctx.w, s) {
            Ok(v) => v,
            Err(error) => {
                tracing::warn!(%error, subscription=id, "relay billing facts unavailable; observer will retry");
                continue;
            }
        };
        if let Err(error) = api.subscription_changed(ctx, s, &v) {
            tracing::warn!(%error, subscription=id, "relay subscription observer will retry");
            continue;
        }
        if s.observed.as_ref() == Some(&v) {
            continue;
        }
        let notice = format!(
            "Relay subscription {id} for {}: {:?}; {} unpaid installment(s), paid={}. +relay/subscriptions; +relay/cure {id}. Other restrictions remain independent.",
            s.principal.label(),
            v.state(),
            v.missed.saturating_sub(v.cured),
            v.paid
        );
        api.subscription_notice(ctx, &s.principal, notice);
        audit(
            ctx,
            format!(
                "billing.subscription.{id} {:?}: missed {}, cured {}, paid {}",
                v.state(),
                v.missed,
                v.cured,
                v.paid
            ),
        );
        s.observed = Some(v);
        changes += 1;
        if changes == 8 {
            break;
        }
    }
    save(ctx, book);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tariff() -> Tariff {
        Tariff {
            version: 1,
            period_secs: 86400,
            standard_minor: 2500,
            priority_minor: 10000,
            sovereign_minor: 50000,
            quote_secs: 600,
        }
    }
    #[test]
    fn arrears_counts_cures_without_erasing_independent_pause() {
        assert_eq!(
            View {
                missed: 1,
                ..View::default()
            }
            .state(),
            ServiceState::Throttled
        );
        assert_eq!(
            View {
                missed: 2,
                ..View::default()
            }
            .state(),
            ServiceState::Suspended
        );
        assert_eq!(
            View {
                missed: 3,
                cured: 2,
                ..View::default()
            }
            .state(),
            ServiceState::Throttled
        );
        assert_eq!(
            View {
                missed: 3,
                cured: 3,
                paused: true,
                ..View::default()
            }
            .state(),
            ServiceState::Suspended
        );
        assert_eq!(
            View {
                missed: 3,
                cured: 3,
                ..View::default()
            }
            .state(),
            ServiceState::Served
        );
        assert_eq!(
            View {
                missed: 0,
                cured: 1,
                ..View::default()
            }
            .state(),
            ServiceState::Suspended
        );
    }
    #[test]
    fn subscription_books_reject_duplicate_claims_corrupt_prices_and_unbounded_records() {
        let s = Subscription {
            id: 1,
            issuer: ObjId(2),
            principal: Principal::Object(ObjId(2)),
            payer: 1,
            class: Class::Standard,
            amount_minor: 2500,
            quoted_at: 1000,
            quote_until: 1600,
            until_ic: None,
            phase: Phase::Active,
            obligation: Some(9),
            observed: None,
        };
        let mut b = Book {
            tariff: tariff(),
            next: 3,
            cursor: 0,
            subscriptions: BTreeMap::from([(1, s.clone())]),
            treaties: BTreeMap::new(),
        };
        assert!(b.validate().is_ok());
        let mut second = s.clone();
        second.id = 2;
        second.principal = Principal::Object(ObjId(3));
        b.subscriptions.insert(2, second);
        assert!(b.validate().is_err());
        b.subscriptions.remove(&2);
        b.subscriptions.get_mut(&1).expect("test row").amount_minor = 1;
        assert!(b.validate().is_err());
        b.subscriptions.clear();
        b.next = 1000;
        for id in 1..=(MAX_SUBSCRIPTIONS as u64 + 1) {
            let mut row = s.clone();
            row.id = id;
            row.phase = Phase::Quoted;
            row.obligation = None;
            b.subscriptions.insert(id, row);
        }
        assert!(b.validate().is_err());
    }
}
