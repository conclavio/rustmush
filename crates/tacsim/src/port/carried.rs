//! **A haul through a house's storefront, and its ask** (doc 50 cut OP.3's vault-to-vault hauls;
//! doc 18 §4.3's drift check, lead ruling L19).
//!
//! An organization's haul ships vault to vault (`crate::services::haul`,
//! `+haul/post ... as=<org> [into=<org>]`), and a lane house's vault at a port is its
//! storefront (`crate::services::org_vault`): the pool its standing drawn ask sells. So the load
//! leg takes the goods out from under that ask, and a delivery `into=` a house, or a late return,
//! lands goods beside one. The legs are the job's, not the market's: they move the pool and never
//! the listing. A pool moved from under its listing is the drift the stock clerk pages on, and a
//! listing over its pool refuses every restock of the mills' product.
//!
//! **The clerk squares the ask with the leg**, on its stock arm, after the house purchase and
//! before the drift check and the list step, one act a fire:
//!
//! - **What is owed** is read off durable state every fire: the job's ledger says which legs
//!   landed, and the record says which of them its ask is squared with
//!   (`crate::services::haul::unsquared`). Nothing is kept in RAM, so a restart squares a leg
//!   once and never twice.
//! - **A load** lowers the ask by what the pool is short of it, up to the units loaded
//!   (`mush_market::shrink_ask`), leaving the listing standing with one unit at least. **A
//!   delivery or a return** raises it by what the pool holds over it, up to the units landed
//!   (`mush_market::restock_ask`, whose own backing check stands). The pool is read as the drift
//!   check reads it: less the units the mills made and the ask does not carry yet, the lot a
//!   house purchase is moving, and what a mill or a consumption has drawn. So an ask staff
//!   already re-listed by hand is found square and left alone.
//! - **The record is marked in the same outbox** as the listing (`crate::services::haul::square`).
//!   With no listing standing there is nothing to move, since the list step lists the pool as it
//!   now stands, and the legs are marked. A refused act is warned and marked too: what it left is
//!   real drift, and the drift check pages it.
//! - **The waits** are the restock's: while a Draw quote on the pool is unsettled its units read
//!   twice, and a lapsed listing the sweep has not deleted cannot be written. The legs stay
//!   owed, and meanwhile the drift check and the readiness report read the pool as its ask knows
//!   it (`as_listed`), so the gap between a leg and its squaring never pages and never flags a
//!   Listings line.
//!
//! **A world an older build ran** heals itself: its loaded hauls carry no mark, so the first
//! fire lowers the ask they left standing, and the mills' restock goes through on the next.

use mush_core::{CanonicalId, Ctx, ObjId};
use mush_economy::VaultId;
use mush_inventory::record::EndpointRef;
use mush_market::MarketStore;

use crate::port::def::PortDef;
use crate::port::families::PortHandles;
use crate::port::map::PortMap;
use crate::port::stock::swapped;
use crate::port_tick::{Fire, PortTickState, Step};
use crate::services::haul::{self, Unsquared};

/// **What the clerk does to an ask** to square it with the hauls' landed legs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Adjust {
    /// Lower it by these units: a load took them out of the pool.
    Lower(u64),
    /// Raise it by these units: a delivery or a return landed them in it.
    Raise(u64),
    /// It is square already, or what is left is not the hauls'.
    Nothing,
}

/// **The pool as its ask knows it**: `net` with the units the hauls loaded out of it and the ask
/// still lists put back, and the units they landed in it and the ask does not list yet taken
/// out. What the drift check and the readiness report compare with the listing until the clerk
/// squares it. Pure.
#[must_use]
pub(crate) const fn as_listed(net: u64, out: u64, back: u64) -> u64 {
    net.saturating_add(out).saturating_sub(back)
}

/// **The act that squares an ask** standing at `listed` with `undrawn` units in hand-over over a
/// pool that nets `net`, when the hauls loaded `out` units from it and landed `back` in it:
/// lowered by what the pool is short, raised by what it holds over, and never by more than the
/// hauls moved on balance. A lowered listing keeps one unit (a listing at nothing is not a
/// record the market reads back). Pure.
#[must_use]
pub(crate) const fn adjustment(net: u64, listed: u64, undrawn: u64, out: u64, back: u64) -> Adjust {
    let stands = listed.saturating_add(undrawn);
    if net < stands {
        let short = stands - net;
        let moved = out.saturating_sub(back);
        let room = listed.saturating_sub(1);
        let units = if short < moved { short } else { moved };
        let units = if units < room { units } else { room };
        if units > 0 {
            return Adjust::Lower(units);
        }
    } else if net > stands {
        let over = net - stands;
        let moved = back.saturating_sub(out);
        let units = if over < moved { over } else { moved };
        if units > 0 {
            return Adjust::Raise(units);
        }
    }
    Adjust::Nothing
}

/// **The units of `good` the hauls moved at `vault`'s pool and its ask does not show yet**:
/// `(out, back)`, for the drift check and the readiness report ([`as_listed`]).
pub(crate) fn pending(ctx: &Ctx<'_>, vault: u64, good: &CanonicalId) -> (u64, u64) {
    let Unsquared { out, back, .. } = haul::unsquared(ctx, vault, good);
    (out, back)
}

/// **The clerk's step for the hauls' landed legs** (this module's head): the first ask, in
/// authored order, a landed leg is not squared with. `None` when no ask is owed an act this
/// fire, so the stock arm goes on to its drift check and its list step.
pub(crate) fn carried_step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    clerk: ObjId,
    map: &PortMap,
    def: &PortDef,
) -> Option<Step> {
    let mills = ctx
        .w
        .module_state::<PortHandles>()
        .map(|handles| handles.families.mills.clone())?;
    let now = ctx.w.now();
    for vendor in &def.vendors {
        let Some(row) = map.vendors.get(&vendor.org) else {
            continue;
        };
        for ask in vendor
            .asks
            .iter()
            .filter(|ask| !crate::port_tick::is_part(&ask.good))
        {
            let due = haul::unsquared(ctx, row.vault, &ask.good);
            if due.jobs.is_empty() {
                continue;
            }
            let reading = ctx
                .w
                .module_state::<MarketStore>()
                .map(|market| crate::port::stock::ask_reading(market, row.vault, &ask.good))
                .unwrap_or_default();
            // The restock's wait: a Draw's units read twice until the market mirrors its leg.
            if reading.in_flight {
                continue;
            }
            let Some((listing, listed)) = reading.listing else {
                // Nothing stands to move: the list step lists the pool as it now is.
                haul::square(ctx, &due.jobs);
                continue;
            };
            // A lapsed listing the sweep has not deleted yet: the sweep, then the branch above.
            let live = ctx
                .w
                .module_state::<MarketStore>()
                .and_then(|market| market.listing(listing))
                .is_some_and(|standing| !mush_market::listing_lapsed(standing, now));
            if !live {
                continue;
            }
            // The drift check's reading of the pool; a fault in it is the drift check's to warn.
            let Ok(pool) =
                crate::port_tick::read_pool(ctx, EndpointRef(VaultId(row.vault)), &ask.good)
            else {
                continue;
            };
            let Ok(unlisted) =
                crate::port::mill::unlisted(ctx, &mills, map, vendor, row, &ask.good)
            else {
                continue;
            };
            let Some(net) = pool.checked_sub(unlisted) else {
                continue;
            };
            let net = net.saturating_sub(crate::port::supply::moving(ctx, row.vault, &ask.good));
            let net = net.saturating_sub(crate::port::mill::drawn(
                vendor,
                &ask.good,
                net,
                Some(listed),
                reading.undrawn,
            ));
            let net = net.saturating_sub(crate::port::consume::drawn(
                vendor,
                &ask.good,
                net,
                Some(listed),
                reading.undrawn,
            ));
            let (lower, units) = match adjustment(net, listed, reading.undrawn, due.out, due.back) {
                Adjust::Nothing => {
                    haul::square(ctx, &due.jobs);
                    continue;
                }
                Adjust::Lower(units) => (true, units),
                Adjust::Raise(units) => (false, units),
            };
            let spec = mush_market::RestockSpec {
                listing,
                good: ask.good.clone(),
                units,
            };
            let acted = fire.as_principal(ctx, clerk, clerk, |ctx| {
                if lower {
                    mush_market::shrink_ask(ctx, spec)
                } else {
                    mush_market::restock_ask(ctx, spec)
                }
            });
            return Some(match acted {
                Ok(Ok(_)) => {
                    // One outbox: the listing above, the records here.
                    haul::square(ctx, &due.jobs);
                    Step::Worked
                }
                Ok(Err(error)) => {
                    if state.carried_warned.insert((row.vault, ask.good.clone())) {
                        tracing::warn!(
                            vault = row.vault,
                            good = %ask.good,
                            listing = listing.0,
                            units,
                            lower,
                            ?error,
                            runbook = "@tacsim/port status; @market/vendor withdraw <listing> \
                                       re-lists the whole pool",
                            "tacsim: the stock clerk could not square an ask with a haul's leg; \
                             the drift check pages what is left"
                        );
                    }
                    haul::square(ctx, &due.jobs);
                    Step::Refused
                }
                Err(refused) => swapped(Err(refused)),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{Adjust, adjustment, as_listed};

    /// The live case: 5,000 listed, two 30 t loads gone, 64 t milled and not yet listed already
    /// netted out. The ask comes down by the 60 the hauls took, and no further.
    #[test]
    fn a_load_lowers_the_ask_by_what_the_pool_is_short() {
        assert_eq!(adjustment(4_940, 5_000, 0, 60, 0), Adjust::Lower(60));
        assert_eq!(
            adjustment(4_900, 5_000, 0, 60, 0),
            Adjust::Lower(60),
            "never by more than the hauls loaded: the other 40 is the drift check's"
        );
        assert_eq!(
            adjustment(4_970, 5_000, 0, 60, 0),
            Adjust::Lower(30),
            "nor by more than the pool is short"
        );
        assert_eq!(
            adjustment(4_940, 4_990, 10, 60, 0),
            Adjust::Lower(60),
            "the units in hand-over stand with the listing"
        );
    }

    /// An ask staff re-listed by hand already matches its pool: nothing is lowered twice.
    #[test]
    fn a_square_ask_is_left_alone() {
        assert_eq!(adjustment(4_940, 4_940, 0, 60, 0), Adjust::Nothing);
        assert_eq!(adjustment(5_000, 4_940, 0, 60, 0), Adjust::Nothing);
        assert_eq!(adjustment(4_940, 5_000, 0, 0, 0), Adjust::Nothing);
    }

    /// A lowered listing keeps one unit.
    #[test]
    fn a_lowered_listing_keeps_one_unit() {
        assert_eq!(adjustment(0, 20, 0, 30, 0), Adjust::Lower(19));
        assert_eq!(adjustment(0, 1, 0, 30, 0), Adjust::Nothing);
    }

    /// A return raises the ask by what landed, and a load and a return at one pool cancel.
    #[test]
    fn a_return_raises_the_ask_by_what_the_pool_holds_over() {
        assert_eq!(adjustment(5_000, 4_970, 0, 0, 30), Adjust::Raise(30));
        assert_eq!(adjustment(5_010, 4_970, 0, 0, 30), Adjust::Raise(30));
        assert_eq!(adjustment(4_980, 4_970, 0, 0, 30), Adjust::Raise(10));
        assert_eq!(adjustment(5_000, 5_000, 0, 30, 30), Adjust::Nothing);
        assert_eq!(adjustment(4_970, 5_000, 0, 60, 30), Adjust::Lower(30));
    }

    /// Until the clerk acts, the drift check reads the pool as the ask knows it.
    #[test]
    fn the_pool_reads_as_its_ask_knows_it_until_squared() {
        assert_eq!(as_listed(4_940, 60, 0), 5_000);
        assert_eq!(as_listed(5_000, 0, 30), 4_970);
        assert_eq!(as_listed(5_000, 30, 30), 5_000);
        assert_eq!(as_listed(10, 0, 30), 0);
    }
}
