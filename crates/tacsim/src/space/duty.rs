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

use std::collections::BTreeMap;

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
    let duty_minor = duty_on_value(ctx.w, value_major, row)?;
    Some(DutyQuote {
        value_major,
        duty_minor,
        payee: row.payee.clone(),
    })
}

/// **The duty on cargo of `value_major`** (doc 50 cut KC.3: the estimate a job board shows for
/// cargo not yet aboard): `transit_duty_permille` of the value, never under the floor, minor
/// units. `None` for a row that levies no duty.
#[must_use]
pub fn duty_on_value(w: &World, value_major: u64, row: &TermsRow) -> Option<i64> {
    if row.transit_duty_permille == 0 && row.transit_floor_major == 0 {
        return None;
    }
    let scale = crate::port::apply::credit_scale(w);
    let value_minor = crate::port::apply::minor_of(value_major, scale)?;
    let share = value_minor.saturating_mul(i64::from(row.transit_duty_permille)) / 1_000;
    let floor = crate::port::apply::minor_of(row.transit_floor_major, scale)?;
    let duty_minor = share.max(floor);
    (duty_minor > 0).then_some(duty_minor)
}

/// **Whether a lane route crosses the Kirkwood picket** (doc 50 cut KC.3): one of its legs is
/// interdicted and ends at the picket's host.
#[must_use]
pub fn route_crosses_picket(w: &World, route: &CanonicalId) -> bool {
    w.module_state::<crate::store::TacsimStore>()
        .and_then(|store| store.route_in_world(w, route))
        .is_some_and(|route| {
            route.legs().iter().any(|leg| {
                leg.is_interdicted() && super::picket::is_kirkwood_picket_host(w, leg.to())
            })
        })
}

/// **The reason clause** (Q-50-9): who holds the Kirkwood approach and the IC date its hold
/// began, as `Republic hold since March 1, 2350`. `None` when the terms name no holder.
#[must_use]
pub fn reason_clause(w: &World, terms: &PlaceTerms) -> Option<String> {
    let holder = terms.holder?;
    let name = w
        .module_state::<mush_faction::FactionState>()?
        .factions
        .get(&mush_faction::FactionId(holder))?
        .name
        .to_string();
    let since = terms.since_ic.map(|ic| {
        let d = mush_core::clock::CivilDate::from_ic_seconds(ic);
        format!(" since {} {}, {}", d.month_name(), d.day, d.year)
    });
    Some(format!("{name} hold{}", since.unwrap_or_default()))
}

/// **The duty estimate a pilot reads before the picket** (Q-50-8, Q-50-9): on `+ship/plan` and at
/// the filing, for a route that crosses the picket, under `terms` (live for the plan, pinned for
/// the filing). `None` when the route does not cross, the terms levy nothing, or the hold is
/// empty. `pinned` adds the filing's note.
#[must_use]
pub fn estimate_line(
    ctx: &Ctx<'_>,
    hull: ObjId,
    route: &CanonicalId,
    terms: &PlaceTerms,
    pinned: bool,
) -> Option<String> {
    if !route_crosses_picket(ctx.w, route) {
        return None;
    }
    let quote = quote_duty(ctx, hull, terms.row.as_ref()?)?;
    let duty = crate::port::coop::money_minor(ctx, quote.duty_minor);
    let reason = reason_clause(ctx.w, terms).map_or_else(String::new, |r| format!(" ({r})"));
    let note = if pinned { ", pinned at filing" } else { "" };
    Some(format!(
        "Kirkwood duty ~{duty} on {} cr of cargo{note}{reason}.",
        quote.value_major
    ))
}

/// **The duty clause a haul row on the job board carries** (Q-50-8): for cargo of `value_major`
/// carried from `origin` to `destination` on a route that crosses the picket, under the live
/// terms: ` (Kirkwood duty ~400 cr)`. Empty otherwise.
#[must_use]
pub fn haul_clause(w: &World, origin: &str, destination: &str, value_major: u64) -> String {
    let crosses = haul_crosses_picket(w, origin, destination);
    let duty = crosses
        .then(|| live_terms(w).and_then(|t| t.row))
        .flatten()
        .and_then(|row| duty_on_value(w, value_major, &row));
    let Some(minor) = duty else {
        return String::new();
    };
    let unit = 10_i64.pow(u32::from(crate::port::apply::credit_scale(w)));
    format!(" (Kirkwood duty ~{} cr)", (minor + unit - 1) / unit)
}

/// **Whether a haul from `origin` to `destination` crosses the picket** (Q-50-8; doc 50b): the
/// fewest hops of listed lane routes that join the two venues, as a hauler flies them port to
/// port (Ceres to Isidis by Stickney), any hop of which may cross. A venue pair no chain of
/// listed routes joins crosses nothing.
fn haul_crosses_picket(w: &World, origin: &str, destination: &str) -> bool {
    let ports = crate::port::voyage::lane_ports(w);
    let routes_from = |venue: &str| {
        ports
            .iter()
            .find(|(def, _)| def.venue.as_str() == venue)
            .map(|(def, _)| &def.routes)
    };
    // Breadth first over venues: each reached venue keeps whether its path so far crossed.
    let mut reached: BTreeMap<String, bool> = BTreeMap::new();
    reached.insert(origin.to_owned(), false);
    let mut frontier = vec![origin.to_owned()];
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for venue in &frontier {
            let crossed = reached.get(venue).copied().unwrap_or(false);
            let Some(routes) = routes_from(venue) else {
                continue;
            };
            for (to, ids) in routes {
                if reached.contains_key(to.as_str()) {
                    continue;
                }
                let hop = ids.iter().any(|id| route_crosses_picket(w, id));
                reached.insert(to.to_string(), crossed || hop);
                next.push(to.to_string());
            }
        }
        if let Some(&crossed) = reached.get(destination) {
            return crossed;
        }
        frontier = next;
    }
    false
}

/// How a duty was met.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DutyOutcome {
    /// Paid at once.
    Paid,
    /// Owed: a debt (`ObligationId.0`) with 28 IC days of grace (Q-50-54).
    Owed(u64),
}

/// **Resolve the debit account and acting principal for a hull's transit duty or customs fine
/// debt** (`P2-TSW-5`):
/// 1. A hauling house's hull pays from the house's own account (acted by its helm Thing).
/// 2. An organization's commissioned hull (`answerer_for_hull == Offender::Organization(org)`)
///    pays from the organization's own account (acted with staff authority `GOD`, matching
///    `surface/security.rs`), aligning `Obligation.debtor` with `OffenseKey.offender`.
/// 3. Any other hull (or an organization without a credit account) falls back to `debtor`'s own
///    account.
fn resolve_duty_account(w: &World, hull: ObjId, debtor: ObjId) -> Option<(AccountId, ObjId)> {
    if let Some(account) = crate::port::hauler::house_account_of_helm(w, debtor) {
        return Some((account, debtor));
    }
    if let Some(crate::offense::Offender::Organization(org)) =
        crate::offense::answerer_for_hull(w, hull)
        && let Some(account) = crate::port::berth::org_account(w, org)
    {
        return Some((account, mush_core::GOD));
    }
    crate::port::credit::own_account(w, debtor).map(|a| (a.id, debtor))
}

/// Resolve the [`crate::offense::Offender`] matching the actual debtor of obligation `debt`,
/// falling back to [`crate::offense::answerer_for_hull`] (`P2-TSW-5`).
fn offender_of_debt(w: &World, hull: ObjId, debt: Option<u64>) -> Option<crate::offense::Offender> {
    debt.and_then(|id| {
        w.module_state::<mush_economy::EconomyStore>()
            .and_then(|e| {
                e.snapshot()
                    .obligation(mush_economy::ObligationId(id))
                    .map(|o| o.debtor.clone())
            })
            .and_then(|principal| match principal {
                PrincipalId::Object(who) => Some(crate::offense::Offender::Character(who)),
                PrincipalId::Organization(org) => Some(crate::offense::Offender::Organization(org)),
                PrincipalId::System(_) => None,
            })
    })
    .or_else(|| crate::offense::answerer_for_hull(w, hull))
}

/// **Levy the transit duty on `hull`, owed by `debtor`** (Q-50-6, Q-50-54, `P2-TSW-5`): under the
/// hull's terms ([`hull_terms`]), open a debt to the payee due in 28 IC days and pay it at once
/// when the debtor's account covers it beyond `reserve_minor` (doc 50b: a contraband fine found on
/// the same crossing, which is settled first when drawn from the same account). `None` when no
/// duty is owed, or when the debtor or the payee has no account (logged).
pub fn levy_transit_duty(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    debtor: ObjId,
    reserve_minor: i64,
) -> Option<(DutyQuote, DutyOutcome)> {
    let row = hull_terms(ctx.w, hull)?.row?;
    let quote = quote_duty(ctx, hull, &row)?;
    let Some((_, payee_account)) = crate::polity::lane_org_account(ctx.w, &quote.payee) else {
        tracing::warn!(payee = %quote.payee, "tacsim: the transit duty has no payee account; nothing is levied");
        return None;
    };
    let Some((account, acting)) = resolve_duty_account(ctx.w, hull, debtor) else {
        tracing::warn!(
            debtor = debtor.0,
            "tacsim: the transit duty meets a debtor with no account"
        );
        return None;
    };
    // Only withhold `reserve_minor` when the contraband fine draws from the same account.
    let effective_reserve = if super::picket::fine_account(ctx.w, debtor) == Some(account) {
        reserve_minor
    } else {
        0
    };
    let saved = ctx.actor;
    ctx.actor = acting;
    let outcome = open_and_pay(
        ctx,
        account,
        payee_account,
        quote.duty_minor,
        effective_reserve,
    );
    ctx.actor = saved;
    ctx.out.pin_deferred();
    outcome.map(|o| (quote, o))
}

/// **[`levy_transit_duty`] once a crossing** (doc 50b): `crossing`, the hull and the journey
/// cursor of the Kirkwood leg she crossed, is recorded once a duty is levied on it, and a crossing
/// already recorded is charged nothing. With no crossing to key it to, the duty is levied.
pub fn levy_once(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    debtor: ObjId,
    crossing: Option<(ObjId, u32)>,
    reserve_minor: i64,
) -> Option<(DutyQuote, DutyOutcome)> {
    if let Some(key) = crossing
        && ctx
            .w
            .module_state::<super::SpaceBubbleBook>()
            .is_some_and(|book| book.duty_crossings.contains(&key))
    {
        return None;
    }
    let levied = levy_transit_duty(ctx, hull, debtor, reserve_minor)?;
    if let Some(key) = crossing
        && let Some(book) = ctx.w.module_state_mut::<super::SpaceBubbleBook>()
    {
        book.duty_crossings.insert(key);
    }
    if let (Some((_, cursor)), DutyOutcome::Owed(debt)) = (crossing, levied.1) {
        record_owed_duty(ctx, hull, cursor, &levied.0, debt);
    }
    Some(levied)
}

/// **An owed duty on the hull's record** (doc 50b; Q-50-54, `P2-TSW-4`, `P2-TSW-5`): an offense
/// row of the payee's polity against the debt's actual debtor principal (aligned with
/// `Obligation.debtor`), carrying the duty's debt, so once the debt is past its grace the
/// departure hold and the trade restriction read it as they read a fine.
fn record_owed_duty(ctx: &mut Ctx<'_>, hull: ObjId, cursor: u32, quote: &DutyQuote, debt: u64) {
    let Some(polity) = crate::polity::lane_org_account(ctx.w, &quote.payee)
        .and_then(|(org, _)| crate::polity::polity_of_org(ctx.w, org))
    else {
        tracing::warn!(payee = %quote.payee, "tacsim: an owed duty's payee answers to no polity; no record is kept");
        return;
    };
    let Some(offender) = offender_of_debt(ctx.w, hull, Some(debt)) else {
        return;
    };
    let at_ic = ctx.w.world_ic_now().at;
    let offense = crate::offense::ProvenOffense {
        kind: crate::offense::OffenseKind::UnpaidDuty,
        evidence: crate::offense::Evidence::Duty {
            hull,
            cursor,
            at_ic,
        },
        proof: crate::offense::Proof::Audit,
        proven_ic: at_ic,
        debt: Some(debt),
        payout: None,
    };
    let key = crate::offense::OffenseKey { offender, polity };
    let _ = crate::offense::record_offense(ctx, key, offense, crate::offense::is_settled);
}

/// **Fail-closed contraband enforcement when an unattended fine cannot be collected or an
/// unattended customs ruling/seizure lapses** (`P2-TSW-7`): opens a one-term fine debt on the
/// hull's accountable principal (paying it immediately if covered) and records a proven
/// [`crate::offense::OffenseKind::UnlicensedPossession`] offense carrying that debt (or `None`
/// if no account exists, so docking security fines it), plus standing loss. Never clears
/// silently. Answers whether an offense row or debt was recorded.
pub(crate) fn record_uncollected_contraband(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    principal: ObjId,
    unlicensed_kg: u64,
    explicit_fine_minor: Option<i64>,
) -> bool {
    let Some((org, _, payee_account)) = fine_payee(ctx.w, hull) else {
        tracing::warn!(
            hull = hull.0,
            "tacsim: uncollected contraband has no payee; cannot record debt"
        );
        return false;
    };
    let Some(polity) = crate::polity::polity_of_org(ctx.w, org) else {
        tracing::warn!(
            hull = hull.0,
            org = org.0,
            "tacsim: uncollected contraband payee has no polity; cannot record offense"
        );
        return false;
    };
    let fine_minor = explicit_fine_minor
        .filter(|&m| m > 0)
        .or_else(|| super::picket::customs_fine_minor(ctx.w, unlicensed_kg));
    let debt = fine_minor.and_then(|minor| {
        let (account, acting) = resolve_duty_account(ctx.w, hull, principal)?;
        let saved = ctx.actor;
        ctx.actor = acting;
        let opened = open_and_pay_with_id(ctx, account, payee_account, minor, 0);
        ctx.actor = saved;
        ctx.out.pin_deferred();
        opened.map(|(id, _)| id)
    });
    let offender = offender_of_debt(ctx.w, hull, debt)
        .unwrap_or(crate::offense::Offender::Character(principal));
    let entry =
        crate::polity::picket_place(ctx.w).or_else(|| CanonicalId::new("kirkwood_depot").ok());
    let good = CanonicalId::new("strategic_ore").ok();
    let (Some(entry), Some(good)) = (entry, good) else {
        return debt.is_some();
    };
    let units = unlicensed_kg.div_ceil(1_000).max(1);
    let at_ic = ctx.w.world_ic_now().at;
    let offense = crate::offense::ProvenOffense {
        kind: crate::offense::OffenseKind::UnlicensedPossession,
        evidence: crate::offense::Evidence::Audit {
            hull,
            entry,
            good,
            units,
            at_ic,
        },
        proof: crate::offense::Proof::Audit,
        proven_ic: at_ic,
        debt,
        payout: None,
    };
    let key = crate::offense::OffenseKey { offender, polity };
    let recorded = crate::offense::record_offense(ctx, key, offense, crate::offense::is_settled);
    if recorded {
        crate::port::record::lose_standing(ctx, &key, |row| {
            matches!(
                &row.evidence,
                crate::offense::Evidence::Audit {
                    hull: h,
                    at_ic: t,
                    ..
                } if *h == hull && *t == at_ic
            )
        });
    }
    recorded || debt.is_some()
}

fn open_and_pay_with_id(
    ctx: &mut Ctx<'_>,
    from: AccountId,
    to: AccountId,
    minor: i64,
    reserve_minor: i64,
) -> Option<(u64, bool)> {
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
        .is_some_and(|a| a.minor().saturating_sub(reserve_minor.max(0)) >= minor);
    let paid = covers
        && matches!(
            mush_economy::pay_debt(ctx, id, DEBT_NAMESPACE),
            Ok(mush_economy::PostOutcome::Posted(_))
        );
    Some((id.0, paid))
}

fn open_and_pay(
    ctx: &mut Ctx<'_>,
    from: AccountId,
    to: AccountId,
    minor: i64,
    reserve_minor: i64,
) -> Option<DutyOutcome> {
    let (id, paid) = open_and_pay_with_id(ctx, from, to, minor, reserve_minor)?;
    Some(if paid {
        DutyOutcome::Paid
    } else {
        DutyOutcome::Owed(id)
    })
}

/// **The line a duty adds to the picket's report**, or nothing when none was levied.
#[must_use]
pub fn duty_line(ctx: &Ctx<'_>, hull: ObjId, levied: Option<&(DutyQuote, DutyOutcome)>) -> String {
    let Some((quote, outcome)) = levied else {
        return String::new();
    };
    let duty = crate::port::coop::money_minor(ctx, quote.duty_minor);
    // Doc 50 cut KC.3 (Q-50-9): the charge names who holds the approach and since when.
    let reason = hull_terms(ctx.w, hull)
        .and_then(|terms| reason_clause(ctx.w, &terms))
        .unwrap_or_default();
    match outcome {
        DutyOutcome::Paid => format!(
            " A transit duty of {duty} on cargo valued at {} cr is paid ({reason}).",
            quote.value_major
        ),
        DutyOutcome::Owed(_) => format!(
            " A transit duty of {duty} on cargo valued at {} cr is owed, due in 28 IC days (see +debts; {reason}).",
            quote.value_major
        ),
    }
}

/// The picket's name in an absent hull's ruling lines, where no corvette is in a bubble.
const ABSENT_PICKET: &str = "Kirkwood picket";

/// **A hull whose pilot is away crosses the picket** (Q-50-54): assessed the same way with no
/// submission. The debtor is the hull's customs principal (its helm holder, else the passage's
/// pilot). Doc 50b: the manifest is read as at a submission, and a fine or a seizure is filed as
/// a ruling with the principal as its officer (a clean hold is cleared at once); the transit
/// duty, once for `cursor`'s crossing, is paid only from what a fine leaves. Answers the line told
/// to the hull's absent tenant, or nothing when there is nothing to tell.
pub fn assess_absent(ctx: &mut Ctx<'_>, hull: ObjId, cursor: u32) -> Option<String> {
    let pilot = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .and_then(|store| match store.voyage_state(hull) {
            crate::port::voyage::VoyageState::Present(v) => v.departed.as_ref().map(|d| d.pilot),
            crate::port::voyage::VoyageState::Absent
            | crate::port::voyage::VoyageState::Unreadable => None,
        })?;
    let debtor = super::picket::resolve_hull_customs_principal(ctx.w, hull, pilot);
    let (verdict, reserve) = match super::commands::customs_finding(ctx, hull, debtor) {
        Ok((super::customs::CustomsFinding::Cleared { .. }, _)) => (None, 0),
        Ok((finding, found)) => {
            let reserve = match &finding {
                super::customs::CustomsFinding::Fined { fine_minor, .. } => *fine_minor,
                _ => 0,
            };
            let names = (
                ABSENT_PICKET.to_owned(),
                crate::port::coop::name_of(ctx.w, hull),
            );
            match super::customs::file_ruling_as(
                ctx,
                debtor,
                hull,
                debtor,
                finding.clone(),
                names,
                0,
            ) {
                Ok(seq) => super::customs::mark_unattended(ctx, hull, seq),
                Err(error) => {
                    tracing::warn!(hull = hull.0, %error, "tacsim: an absent hull's customs ruling was not filed; recording contraband offense fail-closed");
                    // P2-TSW-7: never fail open on an absent contraband crossing when the
                    // authoritative ruling queue rejects or is busy.
                    match finding {
                        super::customs::CustomsFinding::Fined {
                            unlicensed_strategic_ore_kg,
                            fine_minor,
                        } => {
                            let _ = record_uncollected_contraband(
                                ctx,
                                hull,
                                debtor,
                                unlicensed_strategic_ore_kg,
                                Some(fine_minor),
                            );
                        }
                        super::customs::CustomsFinding::Seized {
                            unlicensed_strategic_ore_kg,
                            ..
                        } => {
                            let _ = record_uncollected_contraband(
                                ctx,
                                hull,
                                debtor,
                                unlicensed_strategic_ore_kg,
                                None,
                            );
                        }
                        super::customs::CustomsFinding::Cleared { .. }
                        | super::customs::CustomsFinding::Ran => {}
                    }
                }
            }
            (Some(found), reserve)
        }
        Err(error) => {
            tracing::warn!(hull = hull.0, %error, "tacsim: an absent hull's manifest could not be ruled on");
            (None, 0)
        }
    };
    let levied = levy_once(ctx, hull, debtor, Some((hull, cursor)), reserve);
    let duty = duty_line(ctx, hull, levied.as_ref());
    Some(match verdict {
        Some(found) => format!(
            "[PICKET] Republic Customs assessed the hull at Kirkwood in her pilot's absence ({found}); the ruling follows once it is recorded.{duty}"
        ),
        None if levied.is_some() => format!(
            "[PICKET] Republic Customs assessed the hull at Kirkwood in her pilot's absence and cleared her.{duty}"
        ),
        None => return None,
    })
}

/// **The space tick settles the duty of every hull that crossed the picket with nobody aboard**
/// (Q-50-54). The leg's boundary only notes the hull ([`SpaceBubbleBook::absent_duty`]): its hook
/// cannot read a hold, and the tick, which levies the fines, can. Each is assessed once, and her
/// absent tenant is told.
///
/// [`SpaceBubbleBook::absent_duty`]: super::SpaceBubbleBook::absent_duty
pub(crate) fn settle_absent(ctx: &mut Ctx<'_>) {
    let due: Vec<(ObjId, u32)> = ctx
        .w
        .module_state_mut::<super::SpaceBubbleBook>()
        .map(|book| std::mem::take(&mut book.absent_duty).into_iter().collect())
        .unwrap_or_default();
    for (hull, cursor) in due {
        if let Some(line) = assess_absent(ctx, hull, cursor) {
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

/// **The Kirkwood crossings a restart caught up** (doc 50b): boot recovery advances an overdue
/// journey's cursor past its boundaries without the live hook, so a crossing of the picket's leg
/// there is neither noted nor assessed. Every leg a journey's cursor has passed that ends at the
/// picket, that the live hook never saw, and that no bubble holds and no clearance names, is
/// queued as an absent crossing and its leg marked cleared. A passage that arrived during the
/// outage has no journey left to read, and is not assessed.
pub(crate) fn catch_up_crossings(ctx: &mut Ctx<'_>) {
    let Some(journeys) = ctx.w.module_state::<mush_sim::JourneyBook>() else {
        return;
    };
    let Some(book) = ctx.w.module_state::<super::SpaceBubbleBook>() else {
        return;
    };
    let mut missed: Vec<(ObjId, u32)> = Vec::new();
    for (&hull, journey) in &journeys.journeys {
        if book.active_for_hull(hull).is_some() {
            continue;
        }
        let passed = usize::try_from(journey.cursor()).unwrap_or(0);
        for (index, leg) in journey.route().legs().iter().enumerate().take(passed) {
            let Ok(reached) = u32::try_from(index + 1) else {
                continue;
            };
            let key = (hull, reached);
            if leg.is_interdicted()
                && super::picket::is_kirkwood_picket_host(ctx.w, leg.to())
                && !book.picket_crossings.contains(&key)
                && !book.cleared_interdiction_legs.contains(&key)
            {
                missed.push(key);
            }
        }
    }
    if missed.is_empty() {
        return;
    }
    if let Some(book) = ctx.w.module_state_mut::<super::SpaceBubbleBook>() {
        for key in missed {
            book.picket_crossings.insert(key);
            book.absent_duty.insert(key);
            book.cleared_interdiction_legs.insert(key);
        }
    }
}
