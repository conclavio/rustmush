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
fn hulls(w: &World) -> Vec<ObjId> {
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
        // own organizations, or a registered hull whose home is this port.
        let title = crate::port::house_sweep::title_of(w, hull);
        if let Some(PrincipalId::Organization(org)) = &title
            && (map.orgs.values().any(|row| row.org == org.0)
                || crate::port::owned::owned(w, hull).is_some_and(|o| o.home == map.entry))
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

/// An organization's credit account, from the first port map that names it.
fn org_account(w: &World, org: OrgId) -> Option<AccountId> {
    w.module_state::<crate::store::TacsimStore>()?
        .port_maps()
        .flat_map(|map| map.orgs.values())
        .find(|row| row.org == org.0)
        .and_then(|row| row.account)
        .map(AccountId)
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

/// **The lien** (Q-48-54): the berth arrears `seller` owes for `hull` at the port she stands in
/// are paid from the seller's account, which a settled sale of her has just credited. A debt the
/// account cannot cover stands, and is logged.
pub(crate) fn collect_lien(ctx: &mut Ctx<'_>, hull: ObjId, seller: &PrincipalId) {
    let crate::port::ship::Berth::Docked { map, .. } = crate::port::ship::berth_of(ctx.w, hull)
    else {
        return;
    };
    let ns = berth_namespace(map.entry.as_str(), hull);
    for debt in mush_economy::debts_of(ctx.w, seller, &[ns.as_str()]) {
        if let Err(error) = mush_economy::pay_debt(ctx, debt.id, &ns) {
            tracing::warn!(debt = debt.id.0, hull = hull.0, %error, "tacsim: a berth lien was not collected at her sale");
        }
    }
}

/// **What `who` owes a port's berth or locker under `ns`, in arrears**, minor units.
#[must_use]
pub fn arrears_of(w: &World, who: &PrincipalId, ns: &str) -> i64 {
    mush_economy::debts_of(w, who, &[ns])
        .iter()
        .map(|d| d.amount.minor())
        .sum()
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
}
