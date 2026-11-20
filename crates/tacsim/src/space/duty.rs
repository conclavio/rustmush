//! **The transit duty at the Kirkwood picket** (doc 50 cut KC.2; Q-50-5, Q-50-6, Q-50-10,
//! Q-50-14, Q-50-53, Q-50-54).
//!
//! - **What is assessed** ([`dutiable_value_major`]): every good in the hold that is not a
//!   ship's store, pooled or held as a lot, under a haul's hold or not (the one hold reading,
//!   [`crate::port::ship::hold_reading`]), valued at the good's authored catalog baseline
//!   (`reference_major`, 600 cr a tonne for strategic ore). The duty is on cargo, not on a flag.
//! - **At what rate** ([`quote_duty`]): the terms row of the approach's de facto holder, pinned on
//!   the passage when it was filed or engaged ([`hull_terms`]): `transit_duty_permille` of the
//!   value, never under `transit_floor_major`. An empty hold pays nothing; a holder with no row
//!   levies nothing (the terms as they are today).
//! - **How it is paid** ([`levy_transit_duty`]): a debt to the row's payee (the customs office)
//!   with 28 IC days of grace, paid at once when the account covers it. Lawful cargo is never
//!   seized for a duty, and the hull is cleared either way. A pilot who submits pays it at the
//!   inspection; a hull whose pilot is away is assessed the same way with no submission
//!   ([`assess_absent`]).

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_economy::{AccountId, Amount};
use mush_org::{OrgId, PrincipalId};

use crate::polity::{PlaceTerms, TermsRow};

/// **The terms a hull meets at the picket** (Q-50-10): those pinned on its passage when it was
/// filed or engaged from free flight, else the terms live now.
#[must_use]
pub fn hull_terms(w: &World, hull: ObjId) -> Option<PlaceTerms> {
    let pinned = w
        .module_state::<crate::store::TacsimStore>()
        .and_then(|store| match store.voyage_state(hull) {
            crate::port::voyage::VoyageState::Present(v) => {
                v.departed.as_ref().and_then(|d| d.terms.clone())
            }
            crate::port::voyage::VoyageState::Absent
            | crate::port::voyage::VoyageState::Unreadable => None,
        });
    pinned.or_else(|| live_terms(w))
}

/// **The picket's terms now** (Q-50-10's pin reads this at filing).
#[must_use]
pub fn live_terms(w: &World) -> Option<PlaceTerms> {
    let place = crate::polity::picket_place(w)?;
    crate::polity::terms_at(w, &place)
}

/// **The catalog value of a hull's cargo, major credits** (Q-50-53): each good that is no
/// ship's store, at its catalog baseline per ledger unit, pooled and lots alike. `None` when the
/// goods table is missing; a good whose hold will not read counts nothing (and is logged).
#[must_use]
pub fn dutiable_value_major(ctx: &Ctx<'_>, hull: ObjId) -> Option<(u64, u64)> {
    let goods = ctx
        .w
        .module_state::<crate::store::TacsimStore>()?
        .goods()?
        .goods
        .clone();
    let (mut units, mut value) = (0_u64, 0_u64);
    for good in goods.iter().filter(|g| g.consumable.is_none()) {
        match crate::port::ship::hold_reading(ctx, hull, &good.id) {
            Ok(0) => {}
            Ok(qty) => {
                units = units.saturating_add(qty);
                value = value.saturating_add(qty.saturating_mul(good.reference_major));
            }
            Err(error) => {
                tracing::warn!(hull = hull.0, good = %good.id, ?error, "tacsim: a dutiable good did not read; it counts nothing");
            }
        }
    }
    Some((units, value))
}

/// A duty assessed: the cargo's value, the duty, and who it is owed to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DutyQuote {
    /// The cargo's catalog value, major credits.
    pub value_major: u64,
    /// The duty, minor credits.
    pub duty_minor: i64,
    /// The lane organization it is owed to.
    pub payee: CanonicalId,
}

/// **The duty a hull owes under `row`** (Q-50-6): `transit_duty_permille` of the cargo's value,
/// never under the floor. `None` for an empty hold or a row that levies no duty.
#[must_use]
pub fn quote_duty(ctx: &Ctx<'_>, hull: ObjId, row: &TermsRow) -> Option<DutyQuote> {
    if row.transit_duty_permille == 0 && row.transit_floor_major == 0 {
        return None;
    }
    let (units, value_major) = dutiable_value_major(ctx, hull)?;
    if units == 0 {
        return None;
    }
    let scale = crate::port::apply::credit_scale(ctx.w);
    let value_minor = crate::port::apply::minor_of(value_major, scale)?;
    let share = value_minor.saturating_mul(i64::from(row.transit_duty_permille)) / 1_000;
    let floor = crate::port::apply::minor_of(row.transit_floor_major, scale)?;
    let duty_minor = share.max(floor);
    (duty_minor > 0).then(|| DutyQuote {
        value_major,
        duty_minor,
        payee: row.payee.clone(),
    })
}

/// How a duty was met.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DutyOutcome {
    /// Paid at once.
    Paid,
    /// Owed: a debt (`ObligationId.0`) with 28 IC days of grace (Q-50-54).
    Owed(u64),
}

/// **Levy the transit duty on `hull`, owed by `debtor`** (Q-50-6, Q-50-54): under the hull's
/// terms ([`hull_terms`]), open a debt to the payee due in 28 IC days and pay it at once when
/// the debtor's account covers it. `None` when no duty is owed, or when the debtor or the payee
/// has no account (logged).
pub fn levy_transit_duty(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    debtor: ObjId,
) -> Option<(DutyQuote, DutyOutcome)> {
    let row = hull_terms(ctx.w, hull)?.row?;
    let quote = quote_duty(ctx, hull, &row)?;
    let Some((_, payee_account)) = crate::polity::lane_org_account(ctx.w, &quote.payee) else {
        tracing::warn!(payee = %quote.payee, "tacsim: the transit duty has no payee account; nothing is levied");
        return None;
    };
    let Some(account) = crate::port::credit::own_account(ctx.w, debtor).map(|a| a.id) else {
        tracing::warn!(
            debtor = debtor.0,
            "tacsim: the transit duty meets a debtor with no account"
        );
        return None;
    };
    let saved = ctx.actor;
    ctx.actor = debtor;
    let outcome = open_and_pay(ctx, account, payee_account, quote.duty_minor);
    ctx.actor = saved;
    ctx.out.pin_deferred();
    outcome.map(|o| (quote, o))
}

fn open_and_pay(
    ctx: &mut Ctx<'_>,
    from: AccountId,
    to: AccountId,
    minor: i64,
) -> Option<DutyOutcome> {
    use crate::surface::retrieval::{DEBT_NAMESPACE, debt_due};
    let due = debt_due(ctx.w);
    let id = match mush_economy::create_debt_due(
        ctx,
        (from, to),
        Amount::from_minor(minor),
        DEBT_NAMESPACE,
        due,
    ) {
        Ok(id) => id,
        Err(error) => {
            tracing::error!(%error, "tacsim: the transit duty could not be opened");
            return None;
        }
    };
    let covers = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().account(from).and_then(|a| a.available()))
        .is_some_and(|a| a.minor() >= minor);
    let paid = covers
        && matches!(
            mush_economy::pay_debt(ctx, id, DEBT_NAMESPACE),
            Ok(mush_economy::PostOutcome::Posted(_))
        );
    Some(if paid {
        DutyOutcome::Paid
    } else {
        DutyOutcome::Owed(id.0)
    })
}

/// **The line a duty adds to the picket's report**, or nothing when none was levied.
#[must_use]
pub fn duty_line(ctx: &Ctx<'_>, levied: Option<&(DutyQuote, DutyOutcome)>) -> String {
    let Some((quote, outcome)) = levied else {
        return String::new();
    };
    let duty = crate::port::coop::money_minor(ctx, quote.duty_minor);
    match outcome {
        DutyOutcome::Paid => format!(
            " A transit duty of {duty} on cargo valued at {} cr is paid.",
            quote.value_major
        ),
        DutyOutcome::Owed(_) => format!(
            " A transit duty of {duty} on cargo valued at {} cr is owed, due in 28 IC days (see +debts).",
            quote.value_major
        ),
    }
}

/// **A hull whose pilot is away crosses the picket** (Q-50-54): assessed the same way with no
/// submission, and cleared. The debtor is the hull's customs principal (its helm holder, else
/// the passage's pilot). Answers the line told to the hull's absent tenant.
pub fn assess_absent(ctx: &mut Ctx<'_>, hull: ObjId) -> Option<String> {
    let pilot = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .and_then(|store| match store.voyage_state(hull) {
            crate::port::voyage::VoyageState::Present(v) => v.departed.as_ref().map(|d| d.pilot),
            crate::port::voyage::VoyageState::Absent
            | crate::port::voyage::VoyageState::Unreadable => None,
        })?;
    let debtor = super::picket::resolve_hull_customs_principal(ctx.w, hull, pilot);
    let levied = levy_transit_duty(ctx, hull, debtor)?;
    Some(format!(
        "[PICKET] Republic Customs assessed the hull at Kirkwood in her pilot's absence and cleared her.{}",
        duty_line(ctx, Some(&levied))
    ))
}

/// **The space tick settles the duty of every hull that crossed the picket with nobody aboard**
/// (Q-50-54). The leg's boundary only notes the hull ([`SpaceBubbleBook::absent_duty`]): its hook
/// cannot read a hold, and the tick, which levies the fines, can. Each is assessed once, and her
/// absent tenant is told.
///
/// [`SpaceBubbleBook::absent_duty`]: super::SpaceBubbleBook::absent_duty
pub(crate) fn settle_absent(ctx: &mut Ctx<'_>) {
    let due: Vec<ObjId> = ctx
        .w
        .module_state_mut::<super::SpaceBubbleBook>()
        .map(|book| std::mem::take(&mut book.absent_duty).into_iter().collect())
        .unwrap_or_default();
    for hull in due {
        if let Some(line) = assess_absent(ctx, hull) {
            crate::port::voyage::narrate_hull(ctx, hull, &line);
            let queued = format!("{}{line}", crate::port::voyage::utc_prefix(ctx.w));
            crate::port::voyage::tell_absent_tenant(ctx, hull, &queued);
        }
    }
}

/// **Who a contraband fine at the picket is paid to** (Q-50-14): the terms row's payee when the
/// hull's terms carry one, else the license's issuer, as built.
#[must_use]
pub fn fine_payee(w: &World, hull: ObjId) -> Option<(OrgId, PrincipalId, AccountId)> {
    if let Some(row) = hull_terms(w, hull).and_then(|t| t.row) {
        let (org, account) = crate::polity::lane_org_account(w, &row.payee)?;
        return Some((org, PrincipalId::Organization(org), account));
    }
    super::picket::resolve_registry_principal_and_account(w)
}
