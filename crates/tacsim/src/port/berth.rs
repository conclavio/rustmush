//! **What a port charges a player** (doc 48 cut EX.4; Q-48-8, -15, -54, -55, -56).
//!
//! - **The exchange's fees** ([`PortFees`], the market's `VenueFees` seam): a port's venue is
//!   operated by its governing authority (the port def's `authority` org), which takes the listing
//!   fee and the sale cut; the levy goes to the port's treasury, a system account the money
//!   census counts as a sink. The least listing fee is [`LISTING_FLOOR_CR`]. A port with no
//!   authority charges nothing.
//! - **The berth** ([`BERTH_CR_PER_DAY`] an IC day per hull of any mass, Q-48-15, provisional):
//!   every hull docked at a port with an authority owes it, from her title holder's account (her
//!   charterer's, for a chartered Co-op hull) to the authority's, as one player-owed obligation (`mush_economy::create_player_obligation`, its grace
//!   the machine's largest, about ten real days at 24:1), kept under
//!   `tacsim.berth.<port>.<hull>` ([`berth_namespace`]). A period the account cannot pay accrues
//!   as arrears, a one-term debt under the same name, payable at any time and paid when the account
//!   is funded. **A House's hulls at their home port owe nothing** (Q-48-55): a hull titled to one
//!   of the port's own organizations, or a registered hull whose home is that port. The obligation
//!   ends when she leaves the berth or changes hands; her arrears stand. **Nothing here holds her
//!   at the berth**: undocking never reads a berth or its arrears.
//! - **The lien** (`collect_lien`): the arrears a seller owes for a hull's berth at the port she
//!   stands in are paid out of her next sale there, in the settlement's own batch
//!   (`crate::port::hull_sale::HullSaleObserver`).
//! - **The locker** (doc 48 cut EX.1b's charge, [`crate::port::locker::LOCKER_MINOR_PER_T_DAY`]
//!   an IC day per started tonne held): the same class, kept under
//!   `tacsim.locker.<venue>.<player>`, repriced to the tonnes held as they change and ended when the
//!   locker is empty.
//!
//! `tick` keeps every obligation in step with the world on each port fire.

use std::collections::BTreeMap;

use mush_core::{Ctx, ObjId, World};
use mush_economy::{AccountId, Amount, EconomyStore, Obligation, ObligationStatus};
use mush_market::{MarketId, VenueFeeAccounts};
use mush_org::{OrgId, PrincipalId};
use smol_str::SmolStr;

use crate::port::apply::{credit_scale, minor_of};

/// The berth fee, whole credits an IC day (Q-48-15, provisional).
pub const BERTH_CR_PER_DAY: u64 = 5;
/// The least listing fee at a port's venue, whole credits (Q-48-8, provisional).
pub const LISTING_FLOOR_CR: u64 = 10;
/// A berth's namespace prefix.
pub const BERTH_NS_PREFIX: &str = "tacsim.berth.";
/// A locker's namespace prefix.
pub const LOCKER_NS_PREFIX: &str = "tacsim.locker.";
/// IC seconds in an IC day.
const IC_DAY: i64 = 86_400;

/// **The namespace a hull's berth at `port` is kept under**: its obligation and its arrears.
#[must_use]
pub fn berth_namespace(port: &str, hull: ObjId) -> String {
    format!("{BERTH_NS_PREFIX}{port}.{}", hull.0)
}

/// **The namespace a player's locker at `venue` is kept under.**
#[must_use]
pub fn locker_namespace(venue: &str, player: ObjId) -> String {
    format!("{LOCKER_NS_PREFIX}{venue}.{}", player.0)
}

/// **TacSim's fee seam** (doc 48 cut EX.4): a port's authority operates its venue.
#[derive(Debug, Default)]
pub struct PortFees;

impl mush_market::VenueFees for PortFees {
    fn accounts(&self, w: &World, venue: &MarketId) -> Option<VenueFeeAccounts> {
        let (def, map) = crate::port::market::port_of_venue(w, venue)?;
        let map = map?;
        let authority = map.orgs.get(def.authority.as_ref()?)?;
        let floor = minor_of(LISTING_FLOOR_CR, credit_scale(w))?;
        Some(VenueFeeAccounts {
            operator: AccountId(authority.account?),
            operator_name: crate::surface::retrieval::principal_name(
                w,
                &PrincipalId::Organization(OrgId(authority.org)),
            ),
            sink: AccountId(map.treasury?),
            listing_floor: Amount::from_minor(floor),
        })
    }

    /// **The port's buy-back desk lists as its house** (doc 48 cut EX.3b): the clerk's Thing bids
    /// for a scrap's tonnes and parts on the house's behalf, through a personal bid only because
    /// the market refuses one GOD-owned NPC's listing to another, so its bids pay no listing fee.
    fn house_lists(&self, w: &World, venue: &MarketId, payer: &PrincipalId) -> bool {
        let Some((_, Some(map))) = crate::port::market::port_of_venue(w, venue) else {
            return false;
        };
        map.desk
            .as_ref()
            .is_some_and(|desk| *payer == PrincipalId::Object(ObjId(desk.thing)))
    }
}

/// One obligation the world wants: who pays whom how much, under which name.
struct Want {
    debtor: PrincipalId,
    debit: AccountId,
    credit: AccountId,
    amount: Amount,
}

/// **Keep every berth and locker obligation in step** (doc 48 cut EX.4): open what the world now
/// wants, reprice a locker whose tonnes moved, and end what it no longer wants. Never panics; a
/// refusal is logged and retried next fire.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    collect_sold_liens(ctx);
    let mut wanted = berths_wanted(ctx.w);
    wanted.extend(lockers_wanted(ctx));
    let live = live_obligations(ctx.w);
    for (ns, o) in &live {
        match wanted.get(ns) {
            Some(want) if want.debtor == o.debtor && want.debit == o.debit_account => {
                if want.amount != o.amount
                    && let Err(error) =
                        mush_economy::reprice_player_obligation(ctx, o.id, ns, want.amount)
                {
                    tracing::warn!(obligation = o.id.0, %error, "tacsim: a charge was not repriced");
                }
            }
            _ => {
                if let Err(error) = mush_economy::cancel_obligation(ctx, o.id, ns) {
                    tracing::warn!(obligation = o.id.0, %error, "tacsim: a charge did not end");
                }
            }
        }
    }
    let now_ic = ctx.w.world_ic_now().at;
    for (ns, want) in wanted {
        if live
            .get(&ns)
            .is_some_and(|o| o.debtor == want.debtor && o.debit_account == want.debit)
        {
            continue;
        }
        let spec = mush_economy::ObligationSpec {
            debit_account: want.debit,
            credit_account: want.credit,
            amount: want.amount,
            schedule: mush_economy::EconomicSchedule::new(
                mush_economy::EconomicClock::WorldIc,
                now_ic.saturating_add(IC_DAY),
                Some(IC_DAY),
                mush_economy::MAX_CATCH_UP_CAP,
            ),
            grace: mush_economy::GracePolicy::new(mush_economy::MAX_GRACE_PERIODS),
            notify_on_success: false,
            cause: mush_economy::ObligationCause::Ruleset {
                namespace: SmolStr::new(&ns),
            },
        };
        if let Err(error) = mush_economy::create_player_obligation(ctx, spec, &ns) {
            tracing::warn!(%ns, %error, "tacsim: a charge was not opened");
        }
    }
}

/// Every live recurring berth or locker obligation, by its namespace.
fn live_obligations(w: &World) -> BTreeMap<String, Obligation> {
    let Some(store) = w.module_state::<EconomyStore>() else {
        return BTreeMap::new();
    };
    store
        .snapshot()
        .obligations()
        .filter(|o| o.schedule.cadence.is_some())
        .filter(|o| {
            !matches!(
                o.status,
                ObligationStatus::Cancelled | ObligationStatus::Satisfied
            )
        })
        .filter_map(|o| {
            let ns = o.arrears.as_ref()?.namespace.to_string();
            (ns.starts_with(BERTH_NS_PREFIX) || ns.starts_with(LOCKER_NS_PREFIX))
                .then(|| (ns, o.clone()))
        })
        .collect()
}

/// The hulls a berth fee may apply to: every programme hull and every registered one.
pub(crate) fn hulls(w: &World) -> Vec<ObjId> {
    let mut out: Vec<ObjId> = crate::port::coop::programme_ports(w)
        .iter()
        .flat_map(|port| port.map.hulls.iter().map(|row| ObjId(row.hull)))
        .collect();
    out.extend(crate::port::owned::rows(w).map(|row| row.hull));
    out.sort_unstable();
    out.dedup();
    out
}

/// **The berths the world wants charged**: each docked hull at a port with an authority, owed by
/// her title holder unless she is a House's at its home port.
fn berths_wanted(w: &World) -> BTreeMap<String, Want> {
    let mut out = BTreeMap::new();
    let Some(amount) = minor_of(BERTH_CR_PER_DAY, credit_scale(w)).map(Amount::from_minor) else {
        return out;
    };
    for hull in hulls(w) {
        let crate::port::ship::Berth::Docked { map, def } = crate::port::ship::berth_of(w, hull)
        else {
            continue;
        };
        let Some(credit) = def
            .authority
            .as_ref()
            .and_then(|a| map.orgs.get(a))
            .and_then(|row| row.account)
            .map(AccountId)
        else {
            continue;
        };
        // A House's hull at its home port owes nothing (Q-48-55): one titled to one of the port's
        // own organizations, or a House's registered hull whose home is this port. Doc 48e (48c#13,
        // 48d H-14): the home waiver is a House's only, never any organization's own-yard hull.
        let title = crate::port::house_sweep::title_of(w, hull);
        if let Some(PrincipalId::Organization(org)) = &title
            && (map.orgs.values().any(|row| row.org == org.0)
                || (is_house(w, *org)
                    && crate::port::owned::owned(w, hull).is_some_and(|o| o.home == map.entry)))
        {
            continue;
        }
        // A chartered hull's berth is her charterer's, never the House that lets her: the
        // pilot berths her (doc 48b G-4(3): a House's treasury must not drain for its charters).
        let charterer = w
            .module_state::<crate::store::TacsimStore>()
            .and_then(|s| s.enrolment_for_hull(hull))
            .filter(|e| {
                matches!(
                    e.status,
                    crate::port::coop::EnrolmentStatus::Active
                        | crate::port::coop::EnrolmentStatus::BuyoutPending
                )
            })
            .map(|e| PrincipalId::Object(e.pilot));
        let Some(debtor) = charterer.or(title) else {
            continue;
        };
        let debit = match &debtor {
            PrincipalId::Object(player) => {
                crate::port::credit::own_account(w, *player).map(|a| a.id)
            }
            PrincipalId::Organization(org) => org_account(w, *org),
            PrincipalId::System(_) => None,
        };
        let Some(debit) = debit.filter(|d| *d != credit) else {
            continue;
        };
        out.insert(
            berth_namespace(map.entry.as_str(), hull),
            Want {
                debtor,
                debit,
                credit,
                amount,
            },
        );
    }
    out
}

/// An organization's credit account: a lane institution's from the first port map that names it,
/// else its root-scope treasury (doc 48e, 48c#13 and 48d H-14: a launch House or a player's
/// organization berths too, as upkeep already charges them).
pub(crate) fn org_account(w: &World, org: OrgId) -> Option<AccountId> {
    w.module_state::<crate::store::TacsimStore>()
        .and_then(|store| {
            store
                .port_maps()
                .flat_map(|map| map.orgs.values())
                .find(|row| row.org == org.0)
                .and_then(|row| row.account)
        })
        .map(AccountId)
        .or_else(|| crate::port::pools::treasury_of(w, org))
        .or_else(|| {
            w.module_state::<EconomyStore>()?
                .snapshot()
                .accounts_of(&PrincipalId::Organization(org))
                .filter(|a| {
                    a.instrument.as_str() == crate::economy::CREDIT
                        && a.status == mush_economy::AccountStatus::Active
                })
                .map(|a| a.id)
                .min()
        })
}

/// **Whether `org` is a House** (Q-48-55's waiver): a lane institution some port map names, or an
/// organization with a House record of its own (doc 48 unit PL.5's officers' pool).
fn is_house(w: &World, org: OrgId) -> bool {
    w.module_state::<crate::store::TacsimStore>()
        .is_some_and(|store| {
            store
                .port_maps()
                .flat_map(|map| map.orgs.values())
                .any(|row| row.org == org.0)
        })
        || crate::port::pools::pool(w, &crate::port::pools::house_key(org.0)).is_some()
}

/// **The lockers the world wants charged**: each locker holding goods, at its started tonnes.
fn lockers_wanted(ctx: &Ctx<'_>) -> BTreeMap<String, Want> {
    let mut out = BTreeMap::new();
    let lockers: Vec<crate::port::locker::Locker> = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .map(|s| s.lockers.values().cloned().collect())
        .unwrap_or_default();
    for locker in lockers {
        let tonnes =
            crate::port::locker::held_kg(ctx, mush_economy::VaultId(locker.vault)).div_ceil(1_000);
        if tonnes == 0 {
            continue;
        }
        let Some(venue) = MarketId::new(locker.venue.as_str()) else {
            continue;
        };
        let Some(fees) = mush_market::VenueFees::accounts(&PortFees, ctx.w, &venue) else {
            continue;
        };
        let Some(debit) = crate::port::credit::own_account(ctx.w, locker.player).map(|a| a.id)
        else {
            continue;
        };
        let amount = i64::try_from(tonnes)
            .ok()
            .and_then(|t| t.checked_mul(crate::port::locker::LOCKER_MINOR_PER_T_DAY))
            .map(Amount::from_minor);
        let Some(amount) = amount else { continue };
        out.insert(
            locker_namespace(locker.venue.as_str(), locker.player),
            Want {
                debtor: PrincipalId::Object(locker.player),
                debit,
                credit: fees.operator,
                amount,
            },
        );
    }
    out
}

/// Parse a berth obligation namespace `tacsim.berth.<port>.<hull>` into `(port, hull)`.
fn parse_berth_namespace(ns: &str) -> Option<(&str, ObjId)> {
    let (entry, hull) = ns.strip_prefix(BERTH_NS_PREFIX)?.rsplit_once('.')?;
    let hull = ObjId(hull.parse::<u32>().ok()?);
    (!entry.is_empty() && hull.0 > 0).then_some((entry, hull))
}

/// Whether `principal` currently holds an active Co-op charter on `hull`.
fn is_active_charterer(w: &World, hull: ObjId, principal: &PrincipalId) -> bool {
    let PrincipalId::Object(pilot) = *principal else {
        return false;
    };
    w.module_state::<crate::store::TacsimStore>()
        .and_then(|s| s.enrolment_for_hull(hull))
        .is_some_and(|e| {
            e.pilot == pilot
                && matches!(
                    e.status,
                    crate::port::coop::EnrolmentStatus::Active
                        | crate::port::coop::EnrolmentStatus::BuyoutPending
                )
        })
}

/// **The lien** (Q-48-54; M6): every berth arrears debt `seller` owes for `hull` across all ports
/// is paid from the seller's account, which a settled sale of her has just credited (paying the
/// port where she currently lies docked first, then any other port's arrears). A debt the account
/// cannot cover stands, and is logged.
pub(crate) fn collect_lien(ctx: &mut Ctx<'_>, hull: ObjId, seller: &PrincipalId) {
    let current_ns = match crate::port::ship::berth_of(ctx.w, hull) {
        crate::port::ship::Berth::Docked { map, .. } => {
            Some(berth_namespace(map.entry.as_str(), hull))
        }
        crate::port::ship::Berth::UnderWay | crate::port::ship::Berth::Elsewhere => None,
    };
    let mut namespaces: Vec<String> = mush_economy::debts_of(ctx.w, seller, &[])
        .into_iter()
        .filter_map(|debt| match debt.cause {
            mush_economy::ObligationCause::Ruleset { namespace }
                if parse_berth_namespace(namespace.as_str())
                    .is_some_and(|(_, debt_hull)| debt_hull == hull) =>
            {
                Some(namespace.to_string())
            }
            _ => None,
        })
        .collect();
    namespaces.sort();
    namespaces.dedup();
    if let Some(ref here) = current_ns
        && let Some(pos) = namespaces.iter().position(|ns| ns == here)
    {
        let ns = namespaces.remove(pos);
        namespaces.insert(0, ns);
    }
    if namespaces.is_empty() {
        return;
    }
    // Doc 48e (48d H-13 (a)): the lien is the port's, collected under system authority whatever
    // ctx settled the sale (the sweep, or a player's command that promoted it).
    let saved = ctx.actor;
    ctx.actor = mush_core::GOD;
    for ns in &namespaces {
        for debt in mush_economy::debts_of(ctx.w, seller, &[ns.as_str()]) {
            if let Err(error) = mush_economy::pay_debt(ctx, debt.id, ns) {
                tracing::warn!(debt = debt.id.0, hull = hull.0, %ns, %error, "tacsim: a berth lien was not collected at her sale");
            }
        }
    }
    ctx.actor = saved;
}

/// **The liens of hulls sold another way** (doc 48e, 48c#11 and 48d H-07; M6): a cored hull sells
/// through EX.2b's private resale, a union with no economy contract, so the sale observer never
/// sees it. Each fire, every berth arrears debt whose hull is now titled to someone other than
/// the debtor (and is not still chartered by the debtor) is collected across all ports from the
/// debtor.
fn collect_sold_liens(ctx: &mut Ctx<'_>) {
    let Some(store) = ctx.w.module_state::<EconomyStore>() else {
        return;
    };
    let mut sold: Vec<(ObjId, PrincipalId)> = Vec::new();
    for o in store.snapshot().obligations().filter(|o| {
        o.schedule.cadence.is_none()
            && matches!(
                o.status,
                ObligationStatus::Active
                    | ObligationStatus::Grace { .. }
                    | ObligationStatus::Delinquent
            )
    }) {
        let mush_economy::ObligationCause::Ruleset { namespace } = &o.cause else {
            continue;
        };
        let Some((_, hull)) = parse_berth_namespace(namespace.as_str()) else {
            continue;
        };
        let passed = crate::port::house_sweep::title_of(ctx.w, hull).as_ref() != Some(&o.debtor)
            && !is_active_charterer(ctx.w, hull, &o.debtor);
        if passed && !sold.iter().any(|(h, p)| *h == hull && *p == o.debtor) {
            sold.push((hull, o.debtor.clone()));
        }
    }
    for (hull, seller) in sold {
        collect_lien(ctx, hull, &seller);
    }
}

/// **What `who` owes a port's berth or locker under `ns`, in arrears**, minor units.
#[must_use]
pub fn arrears_of(w: &World, who: &PrincipalId, ns: &str) -> i64 {
    mush_economy::debts_of(w, who, &[ns])
        .iter()
        .fold(0_i64, |sum, d| sum.saturating_add(d.amount.minor()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ex4_a_berth_and_a_locker_are_named_by_port_and_owner() {
        assert_eq!(
            berth_namespace("ceres_port", ObjId(12)),
            "tacsim.berth.ceres_port.12"
        );
        assert_eq!(locker_namespace("ceres", ObjId(7)), "tacsim.locker.ceres.7");
    }

    #[test]
    fn m6_parse_berth_namespace_and_cross_port_lien_sweep() {
        assert_eq!(
            parse_berth_namespace("tacsim.berth.ceres_port.12"),
            Some(("ceres_port", ObjId(12)))
        );
        assert_eq!(
            parse_berth_namespace("tacsim.berth.stickney_port.12"),
            Some(("stickney_port", ObjId(12)))
        );
        assert_eq!(parse_berth_namespace("tacsim.locker.ceres.12"), None);
        assert_eq!(parse_berth_namespace("tacsim.berth.ceres_port.0"), None);
        assert_eq!(parse_berth_namespace("tacsim.berth..12"), None);
    }
}
