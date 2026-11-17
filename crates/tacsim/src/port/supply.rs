//! **A works' house purchase** (doc 39 cut IN.5; §4.2's supply table, Q-39-31): the Stickney
//! works buys `structural_alloys` from Kasei's storefront next door at Kasei's ask, paid from
//! Stickney Yards' account — "a receipted in-kind transfer from Kasei's storefront pool to the
//! works' pool at the ask, paid from Stickney Yards' account" — whenever its stores hold less than
//! the row's floor, one lot an IC day a row (the lane's `works.buys`, [`WorksBuyDef`](crate::port::def::WorksBuyDef)). The stores
//! are what a house order at the works draws from (IN.3's house orders); a player's order sources
//! from the player's own hold.
//!
//! **Two durable facts, and nothing else** (the mill's restart argument). A purchase is paid by one
//! economy post under the key `tacsim.works_supply/{stores}/{act}/{good}/{lot}`, and moved by one
//! `tacsim_works` act (the order contract's own goods move, `crate::port::works`) under the works
//! ledger keyed by the stores vault, ordinal `act`. A purchase is **in flight** exactly when the
//! key of the ledger's next ordinal has resolved: paid, not yet moved. Every fire re-derives that
//! from the economy's idempotency index and the ledger, so a restart never pays twice and never
//! loses a paid lot; the RAM copy of the owed move (`PortTickState::supply_owed`) only carries it
//! to the offer the act lands through, and the IC-day rate limit is RAM, as the mill's back-offs
//! are (a restart may buy one lot early, which the floor bounds).
//!
//! **The ask comes down first.** The seller's units back its standing Draw listing, and a pool
//! moved from under its listing is the drift the stock clerk pages on. So the clerk first lowers
//! the seller's ask by the lot (`mush_market::shrink_ask`, the mill restock's mirror; never while a
//! Draw quote on the pool is unsettled, the restock's wait), then moves the lot. The ask stands
//! throughout, so a buyer never finds it gone. Whether the ask is already down is read off durable
//! state: the pool's excess over the listing and its undrawn quotes (net of the mills' unlisted
//! units) is the lot exactly while the move is owed. Meanwhile the drift check and the readiness
//! report count that lot as moving (`moving`), and the clerk's arm does nothing else at that port.
//!
//! **What it never does.** It never buys for a player, never lets the stores pass the floor by more
//! than a lot, and never moves a lot whose payment has not posted. A re-authored row (its good or
//! its lot) while a purchase is in flight leaves that purchase's key unmatched: the payment stands
//! and the lot is not moved until staff repair it by hand.

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_economy::{AccountId, VaultId};
use mush_inventory::record::EndpointRef;
use mush_market::MarketStore;
use smol_str::SmolStr;

use crate::port::def::{PortDef, WorksBuyDef};
use crate::port::map::PortMap;
use crate::port::stock::{IC_DAY, swapped};
use crate::port::works::WorksAct;
use crate::port_tick::{Fire, PortTickState, Step};

/// The purchase's idempotency scope and its economic cause's kind.
pub const SUPPLY_KEY: &str = "tacsim.works_supply";

/// The purse top-up's key for `buy` into `stores` on IC day `day`: one an IC day a row.
fn key(stores: u64, day: i64, buy: &WorksBuyDef) -> mush_economy::IdempotencyKey {
    mush_economy::IdempotencyKey::new(
        SUPPLY_KEY,
        format!("{stores}/{day}/{}/{}", buy.good, buy.lot),
    )
}

/// **The moves the clerk owes as `actor`** for the works' offer (`crate::port::works`): the
/// in-flight purchases this fire derived, by stores vault.
pub(crate) fn owed_moves(w: &World, actor: ObjId) -> Vec<(u64, WorksAct)> {
    w.module_state::<PortTickState>()
        .map(|state| {
            state
                .supply_owed
                .iter()
                .filter(|(_, owed)| owed.by == actor)
                .map(|(stores, owed)| (*stores, owed.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// **The units of `good` a house purchase is moving out of the storefront `vault`** this fire: the
/// lot its ask was lowered by, which the pool still holds until the move lands under the works
/// ledger (the drift check's and the readiness report's reading).
pub(crate) fn moving(ctx: &Ctx<'_>, vault: u64, good: &CanonicalId) -> u64 {
    ctx.w.module_state::<PortTickState>().map_or(0, |state| {
        state
            .supply_owed
            .iter()
            .filter(|(stores, owed)| {
                owed.from == vault && crate::port::works::landed(ctx, **stores) < owed.act
            })
            .map(|(_, owed)| owed)
            .flat_map(|owed| owed.goods.iter())
            .filter(|g| &g.good == good)
            .map(|g| g.units)
            .sum()
    })
}

/// Warn once a boot under `(stores, good)`.
fn warn_once(state: &mut PortTickState, stores: u64, good: &CanonicalId, why: &str) {
    if state.supply_warned.insert((stores, good.clone())) {
        tracing::warn!(stores, %good, why, "tacsim: a works' house purchase is passed over today");
    }
}

/// **The house purchase's step** on the stock clerk's turn (after the seeds, before the list
/// step): start one where a row's stores are under its floor. `None` when the port has no
/// purchase to make, or one is already on the board, so the stock arm goes on to its list step.
pub(crate) fn supply_step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    clerk: ObjId,
    map: &PortMap,
    def: &PortDef,
) -> Option<Step> {
    let works = def
        .works
        .as_deref()
        .filter(|works| !works.buys.is_empty())?;
    let room = crate::port::plant::works_room(ctx.w, &map.entry, def)
        .ok()
        .flatten()?;
    let facility = ctx
        .w
        .module_state::<mush_industry::IndustryStore>()
        .and_then(|store| store.facility(room))
        .cloned()?;
    let stores = match crate::port::works::stores(ctx, &facility) {
        Ok(vault) => vault.0,
        Err(error) => {
            tracing::warn!(%error, "tacsim: a works' stores could not be opened");
            return None;
        }
    };
    // A purchase on the board: the market settles or refunds it, and nothing else is bought
    // meanwhile. The clerk's list step goes on; the market keeps the ask's draw in hand.
    for buy in &works.buys {
        let Some(seller) = map.vendors.get(&buy.vendor) else {
            continue;
        };
        let listing = ctx
            .w
            .module_state::<MarketStore>()
            .and_then(|market| {
                crate::port::stock::ask_reading(market, seller.vault, &buy.good).listing
            })
            .map(|(listing, _)| listing);
        if listing.is_some_and(|listing| on_board(ctx.w, clerk, listing)) {
            return None;
        }
    }
    start(ctx, state, fire, clerk, map, def, stores)
}

/// **The works' buyers** (doc 48 cut EX.3): every laid-up port's stock clerk, whose quotes on the
/// board are the works' house purchases.
#[must_use]
pub fn buyers(w: &World) -> Vec<ObjId> {
    w.module_state::<crate::store::TacsimStore>()
        .into_iter()
        .flat_map(crate::store::TacsimStore::port_maps)
        .filter_map(|map| map.clerk.as_ref().map(|clerk| ObjId(clerk.thing)))
        .collect()
}

/// **Whether any works' house purchase stands on the board** (doc 48 cut EX.3): a buyer's quote
/// whose contract has not yet settled or closed.
#[must_use]
pub fn purchase_on_board(w: &World) -> bool {
    let Some(market) = w.module_state::<MarketStore>() else {
        return false;
    };
    buyers(w).into_iter().any(|buyer| {
        market
            .quotes_of_buyer(&mush_org::PrincipalId::Object(buyer))
            .any(|quote| on_board(w, buyer, quote.listing))
    })
}

/// **Whether `buyer` has a purchase on `listing` on the board**: a quote whose contract is not yet
/// settled, refunded, cancelled or lapsed.
#[must_use]
pub fn on_board(w: &World, buyer: ObjId, listing: mush_market::ListingId) -> bool {
    let (Some(market), Some(economy)) = (
        w.module_state::<MarketStore>(),
        w.module_state::<mush_economy::EconomyStore>(),
    ) else {
        return false;
    };
    market
        .quotes_of_buyer(&mush_org::PrincipalId::Object(buyer))
        .filter(|quote| quote.listing == listing && quote.settled.is_none())
        .filter_map(|quote| quote.contract)
        .filter_map(|contract| economy.snapshot().contract(contract))
        .any(|contract| {
            matches!(
                contract.state,
                mush_economy::ContractState::Draft
                    | mush_economy::ContractState::Offered
                    | mush_economy::ContractState::Active
                    | mush_economy::ContractState::ReadyToSettle
                    | mush_economy::ContractState::Disputed
            )
        })
}

/// **Start a purchase** (doc 48 cut EX.3; Q-48-9): the first row whose stores are under its floor,
/// not bought today, whose seller's ask lists a lot. The operator funds the clerk's purse with
/// what it lacks for the lot at the ask (one post an IC day a row, under the purchase's key), and
/// the clerk buys the lot on the board: a quote of the seller's ask and an acceptance into the
/// works' stores. The market escrows the money, draws the lot and settles; a leg that cannot be
/// staged refunds the escrow to the purse. `None` when no row is due.
fn start(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    clerk: ObjId,
    map: &PortMap,
    def: &PortDef,
    stores: u64,
) -> Option<Step> {
    let works = def.works.as_deref()?;
    let today = ctx.w.world_ic_now().at.div_euclid(IC_DAY);
    let payer = map
        .orgs
        .get(&works.operator)
        .and_then(|row| row.account)
        .map(AccountId)?;
    for buy in &works.buys {
        let day = (stores, buy.good.clone());
        if state.supply_day.get(&day) == Some(&today) {
            continue;
        }
        let held = crate::port_tick::read_pool(ctx, EndpointRef(VaultId(stores)), &buy.good)
            .unwrap_or(u64::MAX);
        if held >= buy.floor {
            continue;
        }
        let Some(seller) = map.vendors.get(&buy.vendor) else {
            continue;
        };
        let reading = ctx
            .w
            .module_state::<MarketStore>()
            .map(|market| crate::port::stock::ask_reading(market, seller.vault, &buy.good))
            .unwrap_or_default();
        let Some((listing, listed)) = reading.listing else {
            continue;
        };
        if listed < buy.lot || reading.in_flight {
            continue;
        }
        let Some(price) = ctx
            .w
            .module_state::<MarketStore>()
            .and_then(|market| market.listing(listing))
            .map(|listing| listing.price)
        else {
            continue;
        };
        let Some(total) = price.minor().checked_mul(i64::try_from(buy.lot).ok()?) else {
            continue;
        };
        // Doc 18 cut IC14 row 2's rule, the sell gate's: a full engine queue would drop the
        // draw after the escrow, so the purchase waits a fire, its day unspent.
        if ctx.authoritative_queue_full() {
            return Some(Step::Deferred);
        }
        // 1. The purse: the clerk's own account, topped up by what it lacks for the lot.
        let purse = match crate::port::pools::purse_of(ctx, clerk, "works buyer purse") {
            Ok(purse) => purse,
            Err(error) => {
                warn_once(state, stores, &buy.good, &error);
                continue;
            }
        };
        let held_minor = ctx
            .w
            .module_state::<mush_economy::EconomyStore>()
            .and_then(|e| {
                e.snapshot()
                    .account(purse)
                    .and_then(mush_economy::Account::available)
            })
            .map_or(0, mush_economy::Amount::minor);
        let lacks = total.saturating_sub(held_minor);
        if lacks > 0 && !fund(ctx, payer, purse, lacks, stores, today, buy) {
            warn_once(
                state,
                stores,
                &buy.good,
                "the operator could not fund the purchase",
            );
            continue;
        }
        // 2. The board: a quote of the seller's ask, accepted into the works' stores.
        let into = EndpointRef(VaultId(stores));
        let bought = fire.as_principal(ctx, clerk, clerk, |ctx| {
            let quote = mush_market::quote(
                ctx,
                &mush_market::QuoteRequest {
                    listing,
                    quantity: buy.lot,
                    window: mush_market::QuoteWindow::Live,
                    ttl_live: mush_market::QUOTE_TTL_LIVE,
                    ttl_remote: mush_market::QUOTE_TTL_REMOTE,
                    presence: mush_market::Presence::Here,
                },
            )?;
            mush_market::accept_at(ctx, quote, Some(into), mush_market::Presence::Here)
        });
        // The day is spent once the market has answered; a clerk busy this fire tries again.
        return Some(match bought {
            Ok(Ok(_)) => {
                state.supply_day.insert(day, today);
                Step::Worked
            }
            Ok(Err(error)) => {
                state.supply_day.insert(day, today);
                warn_once(state, stores, &buy.good, &format!("{error:?}"));
                Step::Refused
            }
            Err(refused) => swapped(Err(refused)),
        });
    }
    None
}

/// The operator's top-up of the clerk's purse, under the purchase's key for the day.
fn fund(
    ctx: &mut Ctx<'_>,
    payer: AccountId,
    purse: AccountId,
    minor: i64,
    stores: u64,
    today: i64,
    buy: &WorksBuyDef,
) -> bool {
    let (Some(debit), credit) = (
        mush_economy::Amount::from_minor(minor).checked_neg(),
        mush_economy::Amount::from_minor(minor),
    ) else {
        return false;
    };
    let plan = mush_economy::EconomyPlan {
        description: format!("the works funds its buyer for {} {}", buy.lot, buy.good),
        visibility: mush_economy::EconomicVisibility::Public,
        ..mush_economy::EconomyPlan::new(
            key(stores, today, buy),
            mush_org::PrincipalId::Object(ctx.actor),
            mush_economy::EconomicCause::Internal {
                kind: SmolStr::new_static(SUPPLY_KEY),
                id: stores,
            },
            vec![
                mush_economy::RequestedLine {
                    account: payer,
                    delta: debit,
                    memo: SmolStr::new_static(SUPPLY_KEY),
                },
                mush_economy::RequestedLine {
                    account: purse,
                    delta: credit,
                    memo: SmolStr::new_static(SUPPLY_KEY),
                },
            ],
        )
    };
    matches!(
        mush_economy::post(ctx, &plan),
        Ok(mush_economy::PostOutcome::Posted(_) | mush_economy::PostOutcome::AlreadyPosted(_))
    )
}
