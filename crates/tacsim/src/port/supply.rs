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

use mush_core::{CanonicalId, Ctx, IdempotencyKeyText, ObjId, PerceivedRef, World};
use mush_economy::{AccountId, VaultId};
use mush_inventory::record::EndpointRef;
use mush_market::MarketStore;
use serde_json::json;
use smol_str::SmolStr;

use crate::port::apply::{credit_scale, minor_of};
use crate::port::def::{PortDef, WorksBuyDef};
use crate::port::map::PortMap;
use crate::port::stock::{IC_DAY, swapped};
use crate::port::works::{Goods, Then, WorksAct};
use crate::port_tick::{Fire, PortTickState, Step};

/// The purchase's idempotency scope and its economic cause's kind.
pub const SUPPLY_KEY: &str = "tacsim.works_supply";

/// The payment's key for purchase `act` of `buy` into `stores`.
fn key(stores: u64, act: u32, buy: &WorksBuyDef) -> mush_economy::IdempotencyKey {
    mush_economy::IdempotencyKey::new(
        SUPPLY_KEY,
        format!("{stores}/{act}/{}/{}", buy.good, buy.lot),
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
/// step): finish a purchase in flight — withdraw the seller's listing, then move the lot — or start
/// one where a row's stores are under its floor. `None` when the port has no purchase to make, so
/// the stock arm goes on.
#[allow(clippy::too_many_lines)] // one purchase's two phases, in order
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
    let act = crate::port::works::landed(ctx, stores).checked_add(1)?;
    // 1. A purchase in flight: its key resolved, its move not landed.
    let mut flying = None;
    for buy in &works.buys {
        match mush_economy::resolve_idempotency(ctx, &key(stores, act, buy)) {
            Ok(Some(_)) => {
                flying = Some(buy);
                break;
            }
            Ok(None) => {}
            // Outside the RAM window: the key read is requested; wait for it.
            Err(_) => return Some(Step::Idle),
        }
    }
    let Some(buy) = flying else {
        state.supply_owed.remove(&stores);
        return start(ctx, state, map, def, stores, act);
    };
    let seller = map.vendors.get(&buy.vendor)?;
    let from = EndpointRef(VaultId(seller.vault));
    // The owed move, recorded at once: the offer lands it, and the readiness report reads the
    // seller's ask as a purchase in flight, never as stock waiting to be listed.
    let tries = state
        .supply_tries
        .get(&stores)
        .filter(|(at, _)| *at == act)
        .map_or(0, |(_, tries)| *tries);
    let owed = WorksAct {
        act,
        by: clerk,
        from: seller.vault,
        to: stores,
        goods: vec![Goods {
            good: buy.good.clone(),
            units: buy.lot,
        }],
        lots: Vec::new(),
        then: Then::Supplied,
        tries,
    };
    state.supply_owed.insert(stores, owed);
    let reading = ctx
        .w
        .module_state::<MarketStore>()
        .map(|market| crate::port::stock::ask_reading(market, seller.vault, &buy.good))
        .unwrap_or_default();
    // A Draw quote on the pool unsettled: the wait is the mill restock's, a delay never a loss.
    if reading.in_flight {
        return Some(Step::Idle);
    }
    // 2. The seller's ask comes down by the lot, unless it already has.
    if let Some((listing, listed)) = reading.listing {
        let mills = ctx
            .w
            .module_state::<crate::port::families::PortHandles>()
            .map(|handles| handles.families.mills.clone())?;
        let vendor = def.vendors.iter().find(|v| v.org == buy.vendor)?;
        let pool = crate::port_tick::read_pool(ctx, from, &buy.good).ok()?;
        let unlisted =
            crate::port::mill::unlisted(ctx, &mills, map, vendor, seller, &buy.good).ok()?;
        let excess = pool
            .saturating_sub(unlisted)
            .saturating_sub(listed.saturating_add(reading.undrawn));
        if excess < buy.lot {
            let spec = mush_market::RestockSpec {
                listing,
                good: buy.good.clone(),
                units: buy.lot - excess,
            };
            return Some(
                match fire.as_principal(ctx, clerk, clerk, |ctx| mush_market::shrink_ask(ctx, spec))
                {
                    Ok(Ok(_)) => Step::Worked,
                    Ok(Err(error)) => {
                        warn_once(state, stores, &buy.good, &format!("{error:?}"));
                        Step::Refused
                    }
                    Err(refused) => swapped(Err(refused)),
                },
            );
        }
    }
    // 3. The move, under the works ledger's next ordinal.
    let action = ctx
        .w
        .module_state::<crate::port::works::WorksHandles>()
        .and_then(|handles| handles.action.clone())?;
    let key =
        IdempotencyKeyText::parse_canonical(format!("{SUPPLY_KEY}/{stores}/{act}/{tries}")).ok()?;
    let step = swapped(fire.as_principal(ctx, clerk, clerk, |ctx| {
        ctx.submit_authoritative(
            action,
            PerceivedRef::SelfActor,
            Vec::new(),
            json!({"contract": stores, "act": act}),
            key,
        );
    }));
    if step == Step::Submitted {
        state
            .supply_tries
            .insert(stores, (act, tries.saturating_add(1)));
    }
    Some(step)
}

/// **Start a purchase**: the first row whose stores are under its floor, not bought today, whose
/// seller's pool holds a lot — paid from the operator's account to the seller's at the seller's
/// ask, under the purchase's key. `None` when no row is due.
fn start(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    map: &PortMap,
    def: &PortDef,
    stores: u64,
    act: u32,
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
        let (Some(seller), Some(price)) = (
            map.vendors.get(&buy.vendor),
            def.vendors
                .iter()
                .find(|vendor| vendor.org == buy.vendor)
                .and_then(|vendor| vendor.asks.iter().find(|ask| ask.good == buy.good))
                .map(|ask| ask.price_major),
        ) else {
            continue;
        };
        let pool = crate::port_tick::read_pool(ctx, EndpointRef(VaultId(seller.vault)), &buy.good)
            .unwrap_or(0);
        if pool < buy.lot {
            continue;
        }
        let payee = map
            .orgs
            .get(&buy.vendor)
            .and_then(|row| row.account)
            .map(AccountId);
        let minor = buy
            .lot
            .checked_mul(price)
            .and_then(|major| minor_of(major, credit_scale(ctx.w)));
        let (Some(payee), Some(minor)) = (payee, minor) else {
            continue;
        };
        let (Some(debit), credit) = (
            mush_economy::Amount::from_minor(minor).checked_neg(),
            mush_economy::Amount::from_minor(minor),
        ) else {
            continue;
        };
        state.supply_day.insert(day, today);
        let plan = mush_economy::EconomyPlan {
            description: format!(
                "{} buys {} {} for its works stores",
                works.operator, buy.lot, buy.good
            ),
            visibility: mush_economy::EconomicVisibility::Public,
            ..mush_economy::EconomyPlan::new(
                key(stores, act, buy),
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
                        account: payee,
                        delta: credit,
                        memo: SmolStr::new_static(SUPPLY_KEY),
                    },
                ],
            )
        };
        match mush_economy::post(ctx, &plan) {
            Ok(
                mush_economy::PostOutcome::Posted(_) | mush_economy::PostOutcome::AlreadyPosted(_),
            ) => {
                return Some(Step::Worked);
            }
            other => warn_once(state, stores, &buy.good, &format!("{other:?}")),
        }
    }
    None
}
