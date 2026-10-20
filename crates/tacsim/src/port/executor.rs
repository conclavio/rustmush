//! **The sailing plan's planning core and its executor** (doc 18 cut IC11; doc 18
//! §4.4 "The queued-orders model", "Offline execution", "Failure handling";
//! D-18-4, D-18-23, D-18-31, D-18-32, D-18-41; lead rulings L4–L17 and L24–L26 of
//! the IC11 brief).
//!
//! **One module for the dry run and the step** (D-18-41), so `+ship/plan` and the
//! order arm can never disagree about what a buy can take or what `fill` buys:
//!
//! - the venue's prices — [`ask_at`] (the lowest live Draw ask for a good) and
//!   [`bid_at`] (the best standing bid) — read at the port the hull is docked at
//!   now (the implicit venue, doc 18 §4.4);
//! - the clamps — [`clamp_buy`] and [`clamp_sell`] — take the least of every
//!   bound and name **every** bound that attains it ([`Bound`], logged in this
//!   enum's order: doc 18 §4.4 "stock, hold mass, hold volume, funds");
//! - the room — [`room_for`] — is `capacity::max_more`, the arithmetic the market's
//!   capacity gate applies through `capacity::admits`, so a clamp computed here is
//!   never refused at the gate for capacity (lead note 2i);
//! - the projection — [`project_cargo`] — and `fill` — [`fill_targets`], the least
//!   fixed point of "the burn and the stores for the passage, at or above what is
//!   aboard", at the depart order's `not_before`, never now (D-18-41);
//! - the reserve — [`reserve_for`] — prices the next departure's unmet fuel and
//!   stores at this venue's live asks, so a cargo buy never spends the passage
//!   (lead ruling L17).
//!
//! The executor (the order arm on the `tacsim.port` tick) is below the planning
//! core: it runs the cursor order as the document's author, one closed typed call
//! per step inside `Fire::as_principal`, and verifies by pull.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, Ctx, ObjId, PerceivedRef, World};
use mush_economy::{Amount, ContractId, ContractState, EconomyStore};
use mush_market::{
    BidSaleRequest, ListingId, MarketError, MarketStore, Presence, QUOTE_TTL_LIVE,
    QUOTE_TTL_REMOTE, QuoteId, QuoteRequest, QuoteWindow, Supply,
};
use smol_str::SmolStr;

use crate::definitions::GoodsTable;
use crate::definitions::goods::Consumable;
use crate::definitions::logistics::{MASS_KG, VOLUME_L};
use crate::pack::RulesPack;
use crate::port::board::{house_name, label_of, money, unit_of};
use crate::port::burn::{HullSpec, consumable_good, hull_spec_of};
use crate::port::capacity::{self, Compartment};
use crate::port::coop::helm_of;
use crate::port::coop::{name_of, port_name};
use crate::port::def::PortDef;
use crate::port::families::PassagePlan;
use crate::port::families::TicketId;
use crate::port::gauges::{self, Reading};
use crate::port::map::PortMap;
use crate::port::orders::{
    Inflight, OnShort, OrderState, Orders, OrdersState, PlanState, Qty, What, orders_state,
};
use crate::port::passage::{PlanError, choose_route};
use crate::port::ship::{Berth, HullView, berth_of, hull_view};
use crate::port::voyage::PassageOptions;
use crate::port::voyage::{NO_ROUTE, OVERLOADED, narrate_hull, tonnes_one_decimal};
use crate::port::when::{plan_instant, resolve_due};
use crate::port_tick::{Fire, PortTickState, Step, SwapRefused, gone, notify, read_pool};
use crate::sky::Sky;
use crate::store::{TacsimStore, put_orders, put_voyage};

// ---- the constants ---------------------------------------------------------------

/// How many fixed-point passes `fill` makes before it gives up (D-18-41).
pub const FILL_ITERATIONS: u32 = 6;
/// The first re-stage of a market leg (lead ruling L6): since doc 18 cut IC14 row 6
/// (lead ruling L21) one market sweep and a half — `mush_market::SWEEP_PERIOD_SECS` +
/// 30 = 90 s, asserted by a test — not the sweep period itself. At 60 s it equalled the
/// market's 60 s sweep, so a leg staged at the boot instant met its first re-stage in
/// the same fire as the reboot-re-armed sweep that mirrors its landing, and a landed
/// Draw was re-staged before its mirror ran (a wasted replay, IC11's residual). Reading
/// the draw receipt from here instead (the lead's first option) cannot be built: the
/// receipt is market-owned typed state and `Ctx::action_state`'s owner scope refuses
/// it to the TacSim context (IC7's own finding). A literal, because the market's period
/// is a `u64` and this crate's lints refuse the cast.
pub const ORDER_RESTAGE_SECS: i64 = 90;
/// How long past the first re-stage the second deadline waits (reviewer F3): the
/// inventory mirror, the market mirror and the economy's Fact sweep the evidence
/// comes from, plus two fires.
pub const ORDER_SETTLE_GRACE_SECS: i64 = 160;
/// One market sweep lap: a refunded Draw relists within it (lead note 2h); also the
/// pause after a refused closed call.
pub const ORDER_RETRY_SECS: i64 = 60;
/// Proven drops before an order holds (L7).
pub const MAX_ORDER_ATTEMPTS: u32 = 3;
/// Refused closed calls in a row before an order holds (reviewer F6).
pub const MAX_ORDER_REFUSALS: u8 = 3;
/// Most inflight records one fire verifies (the rotation's window).
pub const MAX_ORDER_VERIFY_PER_FIRE: usize = 64;

// ---- the bound words ---------------------------------------------------------------

/// **What limited a clamp** (doc 18 §4.4 "every bound that bit is logged"), in log
/// order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bound {
    /// The price is outside the order's `min=`/`max=`.
    Price,
    /// The listing's stock.
    Stock,
    /// The bid's remaining intake.
    Intake,
    /// The house's funds.
    HouseFunds,
    /// What is aboard to sell.
    Aboard,
    /// The cargo hold's mass limit.
    HoldMass,
    /// The cargo hold's packed-volume limit.
    HoldVolume,
    /// The tank.
    Tank,
    /// The stores locker.
    Locker,
    /// The pilot's funds (less the next departure's reserve).
    Funds,
}

impl Bound {
    /// The bound's word in a log line.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Bound::Price => "price",
            Bound::Stock => "stock",
            Bound::Intake => "intake",
            Bound::HouseFunds => "house funds",
            Bound::Aboard => "aboard",
            Bound::HoldMass => "hold mass",
            Bound::HoldVolume => "hold volume",
            Bound::Tank => "tank",
            Bound::Locker => "locker",
            Bound::Funds => "funds",
        }
    }
}

/// `a`, `a and b`, `a, b and c`.
#[must_use]
pub fn join_bounds(bounds: &[Bound]) -> String {
    let words: Vec<&str> = bounds.iter().map(|bound| bound.word()).collect();
    match words.split_last() {
        None => String::new(),
        Some((last, [])) => (*last).to_owned(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// Sorted into log order, each once.
fn ordered(mut bounds: Vec<Bound>) -> Vec<Bound> {
    bounds.sort_unstable();
    bounds.dedup();
    bounds
}

// ---- the clamps (pure) ---------------------------------------------------------------

/// **A buy's clamp** (doc 18 §4.4): `0` with `[Price]` when the price is over the
/// order's `max`; else `min(want, stock, room, funds_units)`, naming every term that
/// attains the result when it is below `want`.
#[must_use]
pub fn clamp_buy(
    want: u64,
    price_over_max: bool,
    stock: u64,
    room: (u64, Vec<Bound>),
    funds_units: u64,
) -> (u64, Vec<Bound>) {
    if price_over_max {
        return (0, vec![Bound::Price]);
    }
    let result = want.min(stock).min(room.0).min(funds_units);
    if result >= want {
        return (result, Vec::new());
    }
    let mut bounds = Vec::new();
    if stock == result {
        bounds.push(Bound::Stock);
    }
    if room.0 == result {
        bounds.extend(room.1);
    }
    if funds_units == result {
        bounds.push(Bound::Funds);
    }
    (result, ordered(bounds))
}

/// **A sale's clamp**: the same rule over `[Price]`, what is aboard, the bid's
/// remaining intake and the units the house can pay for (`want` = `aboard` for `all`).
#[must_use]
pub fn clamp_sell(
    want: u64,
    price_under_min: bool,
    aboard: u64,
    remaining: u64,
    house_units: u64,
) -> (u64, Vec<Bound>) {
    if price_under_min {
        return (0, vec![Bound::Price]);
    }
    let result = want.min(aboard).min(remaining).min(house_units);
    if result >= want {
        return (result, Vec::new());
    }
    let mut bounds = Vec::new();
    if aboard == result {
        bounds.push(Bound::Aboard);
    }
    if remaining == result {
        bounds.push(Bound::Intake);
    }
    if house_units == result {
        bounds.push(Bound::HouseFunds);
    }
    (result, ordered(bounds))
}

/// **Units affordable** at `price` out of `available` less `reserve`:
/// `floor(max(0, available − reserve) / price)`, checked in `i128`; 0 for a
/// non-positive price.
#[must_use]
pub fn funds_units(available: Amount, reserve: Amount, price: Amount) -> u64 {
    let price = i128::from(price.minor());
    if price <= 0 {
        return 0;
    }
    let left = i128::from(available.minor()).saturating_sub(i128::from(reserve.minor()));
    if left <= 0 {
        return 0;
    }
    u64::try_from(left / price).unwrap_or(u64::MAX)
}

// ---- the compartments ----------------------------------------------------------------

/// Why the planning core could not answer (never guessed at).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    /// The table names no propellant or no provisions good, or a good stows in no
    /// compartment or in more than one.
    #[error("the hull's compartments cannot be told apart: {0}")]
    Compartments(String),
    /// The capacity function refused the question.
    #[error("the capacity function refused: {0}")]
    Capacity(String),
    /// Arithmetic left its type.
    #[error("the arithmetic overflowed")]
    Overflow,
}

/// The tank and the locker, by the compartment each consumable stows in (reviewer
/// F10(3)).
struct Stores {
    propellant: CanonicalId,
    provisions: CanonicalId,
    tank: Compartment,
    locker: Compartment,
}

fn stores(goods: &GoodsTable, readings: &[Reading]) -> Result<Stores, CoreError> {
    let comps: Vec<Compartment> = readings.iter().map(|(_, comp, _)| comp.clone()).collect();
    let missing = |what: &str| CoreError::Compartments(format!("no {what}"));
    let propellant =
        consumable_good(goods, Consumable::Propellant).ok_or_else(|| missing("propellant good"))?;
    let provisions =
        consumable_good(goods, Consumable::Provisions).ok_or_else(|| missing("provisions good"))?;
    let tank = capacity::compartment_for(goods, &comps, &propellant)
        .ok_or_else(|| missing("single tank"))?;
    let locker = capacity::compartment_for(goods, &comps, &provisions)
        .ok_or_else(|| missing("single locker"))?;
    Ok(Stores {
        propellant,
        provisions,
        tank,
        locker,
    })
}

/// A reading's contents by compartment id (empty when absent).
fn contents_of(readings: &[Reading], id: &CanonicalId) -> BTreeMap<CanonicalId, u64> {
    readings
        .iter()
        .find(|(comp, _, _)| comp == id)
        .map(|(_, _, contents)| contents.clone())
        .unwrap_or_default()
}

/// A compartment's binding measures as bound words.
fn measure_bounds(
    stores: &Stores,
    comp: &Compartment,
    measures: &[CanonicalId],
) -> Result<Vec<Bound>, CoreError> {
    if comp.id == stores.tank.id {
        return Ok(vec![Bound::Tank]);
    }
    if comp.id == stores.locker.id {
        return Ok(vec![Bound::Locker]);
    }
    measures
        .iter()
        .map(|measure| match measure.as_str() {
            MASS_KG => Ok(Bound::HoldMass),
            VOLUME_L => Ok(Bound::HoldVolume),
            other => Err(CoreError::Compartments(format!(
                "the hold limits {other}, which no bound word names"
            ))),
        })
        .collect()
}

/// **How many more units of `good` fit, and what binds** (lead note 2i): the
/// compartment `good` stows in, `capacity::max_more` over its contents, the binding
/// measures as bound words.
///
/// # Errors
///
/// [`CoreError`] when the compartments cannot be told apart or the capacity
/// function refuses.
pub fn room_for(
    goods: &GoodsTable,
    readings: &[Reading],
    good: &CanonicalId,
) -> Result<(u64, Vec<Bound>), CoreError> {
    let stores = stores(goods, readings)?;
    let comps: Vec<Compartment> = readings.iter().map(|(_, comp, _)| comp.clone()).collect();
    let comp = capacity::compartment_for(goods, &comps, good)
        .ok_or_else(|| CoreError::Compartments(format!("{good} stows in no single compartment")))?;
    let contents = contents_of(readings, &comp.id);
    let (more, measures) = capacity::max_more(goods, &comp, &contents, good)
        .map_err(|e| CoreError::Capacity(e.to_string()))?;
    Ok((more, ordered(measure_bounds(&stores, &comp, &measures)?)))
}

// ---- the projection and fill (D-18-41) ---------------------------------------------

/// **The cargo a run of orders would load** (D-18-41): from the hold's live contents,
/// in queue order — a sale of all empties that good, a sale of `n` takes `n` off, a
/// cargo buy adds what fits by both limits against the projected contents, an explicit
/// consumable buy adds what its compartment fits as extra payload kilograms; `fill`,
/// departs and stops add nothing. **No funds term and no listing term**: cash and
/// stock can only make the load lighter, so the fuel sized on it is always enough.
/// Returns the projected cargo (every non-consumable good) and the extra kilograms.
///
/// # Errors
///
/// [`CoreError`] when the compartments cannot be told apart or a good is unknown.
pub fn project_cargo(
    goods: &GoodsTable,
    readings: &[Reading],
    orders: &[&What],
) -> Result<(BTreeMap<CanonicalId, u64>, u64), CoreError> {
    let _ = stores(goods, readings)?;
    let comps: Vec<Compartment> = readings.iter().map(|(_, comp, _)| comp.clone()).collect();
    let mut projected: BTreeMap<CanonicalId, BTreeMap<CanonicalId, u64>> = readings
        .iter()
        .map(|(id, _, contents)| (id.clone(), contents.clone()))
        .collect();
    let mut extra_kg: u64 = 0;
    for what in orders {
        match what {
            What::Sell { good, qty, .. } => {
                let Some(comp) = capacity::compartment_for(goods, &comps, good) else {
                    continue;
                };
                let slot = projected.entry(comp.id.clone()).or_default();
                match qty {
                    Qty::All => {
                        slot.remove(good);
                    }
                    Qty::Units(n) => {
                        if let Some(held) = slot.get_mut(good) {
                            *held = held.saturating_sub(*n);
                        }
                    }
                    Qty::Fill => {}
                }
            }
            What::Buy { good, qty, .. } => {
                let Qty::Units(n) = qty else {
                    continue;
                };
                let def = goods
                    .good(good)
                    .ok_or_else(|| CoreError::Compartments(format!("{good} is no good")))?;
                let comp = capacity::compartment_for(goods, &comps, good).ok_or_else(|| {
                    CoreError::Compartments(format!("{good} stows in no single compartment"))
                })?;
                let slot = projected.entry(comp.id.clone()).or_default();
                let (more, _) = capacity::max_more(goods, &comp, slot, good)
                    .map_err(|e| CoreError::Capacity(e.to_string()))?;
                let add = (*n).min(more);
                if def.consumable.is_some() {
                    let kg = def
                        .mass_kg_per_unit()
                        .and_then(|mass| add.checked_mul(mass))
                        .ok_or(CoreError::Overflow)?;
                    extra_kg = extra_kg.checked_add(kg).ok_or(CoreError::Overflow)?;
                }
                let held = slot.entry(good.clone()).or_insert(0);
                *held = held.checked_add(add).ok_or(CoreError::Overflow)?;
            }
            What::Depart { .. } | What::Stop => {}
        }
    }
    let mut cargo: BTreeMap<CanonicalId, u64> = BTreeMap::new();
    for contents in projected.values() {
        for (good, qty) in contents {
            if *qty > 0 && goods.good(good).is_some_and(|def| def.consumable.is_none()) {
                let slot = cargo.entry(good.clone()).or_insert(0);
                *slot = slot.checked_add(*qty).ok_or(CoreError::Overflow)?;
            }
        }
    }
    Ok((cargo, extra_kg))
}

/// **What `fill` buys to** (D-18-41): the tank and locker targets, the projected cargo
/// tonnes they were sized for, the payload and the plan at the fixed point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FillTargets {
    /// The tank target, tonnes (at or above what is aboard).
    pub tank: u64,
    /// The locker target, crew-days (at or above what is aboard).
    pub locker: u64,
    /// The projected cargo, whole tonnes (truncated).
    pub cargo_t: u64,
    /// The payload at the fixed point, kilograms.
    pub payload_kg: u64,
    /// The plan at the fixed point.
    pub plan: PassagePlan,
    /// How many passes it took.
    pub iterations: u32,
}

/// Why `fill` could not size the stores.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FillError {
    /// The planner refused.
    #[error("the passage cannot be planned: {0}")]
    Plan(PlanError),
    /// No fixed point within [`FILL_ITERATIONS`].
    #[error("the burn and the stores did not settle")]
    NoFixedPoint,
    /// The tank cannot carry the target.
    #[error("the tank needs {need} t more and has room for {room}")]
    TankTooSmall {
        /// Tonnes more needed.
        need: u64,
        /// Tonnes of room.
        room: u64,
    },
    /// The locker cannot carry the target.
    #[error("the locker needs {need} crew-days more and has room for {room}")]
    LockerTooSmall {
        /// Crew-days more needed.
        need: u64,
        /// Crew-days of room.
        room: u64,
    },
    /// The compartments or the goods table could not answer.
    #[error("{0}")]
    Core(CoreError),
}

/// A good's kilograms per unit.
fn mass_of(goods: &GoodsTable, good: &CanonicalId) -> Result<u64, CoreError> {
    goods
        .good(good)
        .and_then(crate::definitions::GoodDef::mass_kg_per_unit)
        .ok_or_else(|| CoreError::Compartments(format!("{good} declares no mass")))
}

/// **`fill`: the least fixed point at or above what is aboard** (D-18-41): from
/// `(t, p) = (tank, locker)`, at most [`FILL_ITERATIONS`] times, plan the payload
/// `Σ cargo + extra + t × mass(propellant) + p × mass(provisions) + occupants ×
/// allowance` and take `(max(tank, burn), max(locker, crew-days))`, stopping when it
/// repeats; then the tank and the locker must carry the difference.
///
/// # Errors
///
/// [`FillError`], as its variants say.
#[allow(clippy::too_many_arguments)] // the brief's signature, kept whole
pub fn fill_targets(
    goods: &GoodsTable,
    readings: &[Reading],
    cargo: &BTreeMap<CanonicalId, u64>,
    extra_kg: u64,
    tank: u64,
    locker: u64,
    occupants: u32,
    allowance_kg: u32,
    mut plan: impl FnMut(u64) -> Result<PassagePlan, PlanError>,
) -> Result<FillTargets, FillError> {
    let stores = stores(goods, readings).map_err(FillError::Core)?;
    let mass_p = mass_of(goods, &stores.propellant).map_err(FillError::Core)?;
    let mass_v = mass_of(goods, &stores.provisions).map_err(FillError::Core)?;
    let mut cargo_kg: u64 = 0;
    for (good, qty) in cargo {
        let kg = qty
            .checked_mul(mass_of(goods, good).map_err(FillError::Core)?)
            .ok_or(FillError::Core(CoreError::Overflow))?;
        cargo_kg = cargo_kg
            .checked_add(kg)
            .ok_or(FillError::Core(CoreError::Overflow))?;
    }
    let fixed = cargo_kg
        .checked_add(extra_kg)
        .and_then(|kg| kg.checked_add(u64::from(occupants).checked_mul(u64::from(allowance_kg))?))
        .ok_or(FillError::Core(CoreError::Overflow))?;
    let (mut t, mut p) = (tank, locker);
    let mut found: Option<(u64, PassagePlan, u32)> = None;
    for iteration in 1..=FILL_ITERATIONS {
        let payload = t
            .checked_mul(mass_p)
            .and_then(|kg| kg.checked_add(p.checked_mul(mass_v)?))
            .and_then(|kg| kg.checked_add(fixed))
            .ok_or(FillError::Core(CoreError::Overflow))?;
        let planned = plan(payload).map_err(FillError::Plan)?;
        let next = (
            tank.max(planned.burn_t),
            locker.max(u64::from(planned.crew_days)),
        );
        if next == (t, p) {
            found = Some((payload, planned, iteration));
            break;
        }
        (t, p) = next;
    }
    let Some((payload_kg, planned, iterations)) = found else {
        return Err(FillError::NoFixedPoint);
    };
    let room = |comp: &Compartment, good: &CanonicalId| {
        capacity::max_more(goods, comp, &contents_of(readings, &comp.id), good)
            .map(|(more, _)| more)
            .map_err(|e| FillError::Core(CoreError::Capacity(e.to_string())))
    };
    let tank_room = room(&stores.tank, &stores.propellant)?;
    let need = t.saturating_sub(tank);
    if need > tank_room {
        return Err(FillError::TankTooSmall {
            need,
            room: tank_room,
        });
    }
    let locker_room = room(&stores.locker, &stores.provisions)?;
    let need = p.saturating_sub(locker);
    if need > locker_room {
        return Err(FillError::LockerTooSmall {
            need,
            room: locker_room,
        });
    }
    Ok(FillTargets {
        tank: t,
        locker: p,
        cargo_t: cargo_kg / 1_000,
        payload_kg,
        plan: planned,
        iterations,
    })
}

/// **The reserve a cargo buy leaves for the next departure** (lead ruling L17;
/// reviewer F10(2)): Σ over the two consumables, but the one `excluding` names, of
/// `max(0, target − aboard) × its live ask price here` (an absent ask prices 0),
/// checked in `i128`, saturating at `i64::MAX`.
#[must_use]
pub fn reserve_for(
    targets: &FillTargets,
    aboard_tank: u64,
    aboard_locker: u64,
    ask_tank: Option<Amount>,
    ask_locker: Option<Amount>,
    excluding: Option<Consumable>,
) -> Amount {
    let term = |target: u64, aboard: u64, price: Option<Amount>| -> i128 {
        let short = i128::from(target.saturating_sub(aboard));
        short.saturating_mul(i128::from(price.map_or(0, Amount::minor).max(0)))
    };
    let mut total: i128 = 0;
    if excluding != Some(Consumable::Propellant) {
        total = total.saturating_add(term(targets.tank, aboard_tank, ask_tank));
    }
    if excluding != Some(Consumable::Provisions) {
        total = total.saturating_add(term(targets.locker, aboard_locker, ask_locker));
    }
    Amount::from_minor(i64::try_from(total).unwrap_or(i64::MAX))
}

// ---- the venue (world reads) --------------------------------------------------------

/// **The best ask for a good at a port** (D-18-41's "live listings").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    /// The live Draw listing.
    pub listing: ListingId,
    /// Its price per unit.
    pub price: Amount,
    /// Its quantity (the in-command counter).
    pub quantity: u64,
    /// The house's trading name.
    pub house: String,
}

/// **The lowest live ask for `good` at `def`** (ties to the lower listing id): every
/// vendor that asks it, through its storefront's live Draw listing
/// (`stock::ask_reading`). `None` when it is sold out or not sold here.
#[must_use]
pub fn ask_at(w: &World, def: &PortDef, map: &PortMap, good: &CanonicalId) -> Option<Ask> {
    let market = w.module_state::<MarketStore>()?;
    let mut best: Option<Ask> = None;
    for vendor in &def.vendors {
        if !vendor.asks.iter().any(|ask| &ask.good == good) {
            continue;
        }
        let Some(row) = map.vendors.get(&vendor.org) else {
            continue;
        };
        let Some((listing, quantity)) =
            crate::port::stock::ask_reading(market, row.vault, good).listing
        else {
            continue;
        };
        let Some(price) = market.listing(listing).map(|l| l.price) else {
            continue;
        };
        let better = best.as_ref().is_none_or(|b| {
            price.minor() < b.price.minor() || (price == b.price && listing.0 < b.listing.0)
        });
        if better {
            best = Some(Ask {
                listing,
                price,
                quantity,
                house: house_name(def, &vendor.org, None),
            });
        }
    }
    best
}

/// **The best standing bid for a good at a port.**
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bid {
    /// The Wanted listing.
    pub listing: ListingId,
    /// Its price per unit.
    pub price: Amount,
    /// The intake still wanted (0: exhausted, "not buying").
    pub remaining: u64,
    /// The house's trading name (the bid's `as` name).
    pub house: String,
    /// `floor(the house's available / price)`; 0 when unreadable.
    pub house_units: u64,
}

/// **The best bid for `good` at `def`**: the highest price with intake left (ties to
/// the lower id), else the first exhausted one (the IC10 marker).
#[must_use]
pub fn bid_at(w: &World, def: &PortDef, map: &PortMap, good: &CanonicalId) -> Option<Bid> {
    let market = w.module_state::<MarketStore>()?;
    let economy = w.module_state::<EconomyStore>();
    let mut live: Option<Bid> = None;
    let mut exhausted: Option<Bid> = None;
    for vendor in &def.vendors {
        let Some(bid_def) = vendor.bids.iter().find(|bid| &bid.good == good) else {
            continue;
        };
        let Some(row) = map
            .vendors
            .get(&vendor.org)
            .and_then(|row| row.bids.get(good))
        else {
            continue;
        };
        let id = ListingId(row.listing);
        let Some(listing) = market.listing(id) else {
            continue;
        };
        if !matches!(listing.source, Supply::Wanted { .. }) {
            continue;
        }
        let house_units = market
            .vendor(listing.vendor)
            .and_then(|v| economy?.snapshot().account(v.account).cloned())
            .and_then(|account| account.available())
            .map_or(0, |available| {
                funds_units(available, Amount::from_minor(0), listing.price)
            });
        let bid = Bid {
            listing: id,
            price: listing.price,
            remaining: listing.quantity,
            house: house_name(def, &vendor.org, Some(bid_def)),
            house_units,
        };
        if bid.remaining == 0 {
            if exhausted.is_none() {
                exhausted = Some(bid);
            }
            continue;
        }
        let better = live.as_ref().is_none_or(|b| {
            bid.price.minor() > b.price.minor()
                || (bid.price == b.price && bid.listing.0 < b.listing.0)
        });
        if better {
            live = Some(bid);
        }
    }
    live.or(exhausted)
}

// ---- the planner inputs -------------------------------------------------------------

/// **What a passage is planned on** for one hull: the goods table, the hull spec, the
/// latched lane and its sky (`voyage.rs`'s basis, factored for the queue).
pub(crate) struct Planner<'w> {
    pub(crate) goods: &'w GoodsTable,
    pub(crate) spec: HullSpec,
    pub(crate) pack: &'w RulesPack,
    pub(crate) sky: Sky,
    /// **Doc 20 cut SM.3a**: what the rigs in the hull's bay weigh, kilograms
    /// ([`crate::port::voyage::carried_of`]); [`Planner::plan`] adds it to every
    /// projected payload, so the queue plans a Dray-P with its rigs as the filing
    /// does. Zero from [`planner`]; the callers set it ([`Planner::carrying`]).
    pub(crate) carried_kg: u64,
}

impl Planner<'_> {
    /// The best listed route under `options` for `payload_kg` and `occupants` at
    /// `depart_ic` (doc 19 cut A.2: the order's throttle and waypoint, the
    /// queue's fastest policy).
    pub(crate) fn plan(
        &self,
        routes: &[CanonicalId],
        payload_kg: u64,
        occupants: u32,
        depart_ic: i64,
        options: &PassageOptions,
    ) -> Result<PassagePlan, PlanError> {
        let drive = self
            .spec
            .drive
            .clone()
            .at_throttle(options.throttle_permille);
        // Doc 20 cut SM.3a: the bay's rigs ride every projected payload.
        let payload_kg = payload_kg
            .checked_add(self.carried_kg)
            .ok_or(PlanError::Overflow)?;
        choose_route(
            &self.sky,
            self.pack,
            routes,
            &drive,
            self.spec.dry_kg,
            payload_kg,
            occupants,
            depart_ic,
            options.policy,
            options.via.as_ref(),
        )
    }
}

/// The planner inputs for `view`'s hull.
pub(crate) fn planner<'w>(w: &'w World, view: &HullView<'_>) -> Result<Planner<'w>, String> {
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| "the layer's state is absent".to_owned())?;
    let goods = store
        .goods()
        .ok_or_else(|| "the goods table is not loaded".to_owned())?;
    let lane =
        CanonicalId::new(crate::definitions::content::LANE_PACK).map_err(|e| e.to_string())?;
    let version = store
        .applied_in(w, &lane)
        .ok_or_else(|| "the lane is not latched".to_owned())?;
    let loaded = store
        .pack(&lane, version)
        .ok_or_else(|| format!("the lane at version {version} is not loaded"))?;
    // Doc 20 cut SM.3a (reviewer F6): a hull whose unit record boot held plans
    // nothing; otherwise its own unit (the Dray-P on a refitted hull).
    if view.unit_held {
        return Err(PlanError::Unreadable {
            what: format!("hull #{}'s unit record (held at boot)", view.hull().0),
        }
        .to_string());
    }
    let (_, spec) = hull_spec_of(store.definitions(), goods, &view.unit)?;
    let sky = Sky::from_pack(&loaded.pack, &store.config().time.ephemeris);
    Ok(Planner {
        goods,
        spec,
        pack: &loaded.pack,
        sky,
        carried_kg: 0,
    })
}

impl Planner<'_> {
    /// **The planner with the bay's rigs aboard** (doc 20 cut SM.3a): `view`'s
    /// [`crate::port::voyage::carried_of`] weight added to every plan.
    pub(crate) fn carrying(mut self, ctx: &Ctx<'_>, view: &HullView<'_>) -> Result<Self, String> {
        self.carried_kg = crate::port::voyage::carried_of(ctx, view)?.1;
        Ok(self)
    }
}

/// **The next departure's fill targets** from queue place `from` (inclusive).
pub(crate) struct DepartTargets {
    /// Its destination's venue.
    pub(crate) to: CanonicalId,
    /// The targets at its `not_before`.
    pub(crate) targets: FillTargets,
}

/// Why the next departure's targets could not be sized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TargetError {
    /// No route of this berth serves the depart's destination: the sentence.
    NoRoute(String),
    /// The fill could not be sized: its error and the destination's name.
    Fill(FillError, String),
}

/// The live ask price of a consumable here.
fn consumable_ask(
    w: &World,
    def: &PortDef,
    map: &PortMap,
    goods: &GoodsTable,
    kind: Consumable,
) -> Option<Amount> {
    let good = consumable_good(goods, kind)?;
    ask_at(w, def, map, &good).map(|ask| ask.price)
}

/// `{here}`-and-`{list}`-filled [`NO_ROUTE`] for `def`.
pub(crate) fn no_route_line(w: &World, def: &PortDef) -> String {
    let list: Vec<String> = def
        .routes
        .keys()
        .map(|venue| crate::port::voyage::venue_name(w, venue))
        .collect();
    NO_ROUTE
        .replace("{here}", &port_name(def))
        .replace("{list}", &list.join(", "))
}

/// **What the later orders before the next depart buy of one consumable** (doc 18 cut
/// IC14 row 26; lead ruling L22).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cover {
    /// A later `fill` of it: the next depart's need is met by that order.
    Fill,
    /// The sum of the later `Units(n)` buys of it (saturating).
    Units(u64),
}

/// **The later cover of `good`** (pure; doc 18 cut IC14 row 26): the orders after
/// position `index` up to (not including) the next open depart, open ones only
/// (Pending or Inflight), that buy `good` — a `fill` among them is [`Cover::Fill`],
/// else the sum of their unit counts. `HELD_SHORT_FOR_DEPART` follows doc 18 §4.4's
/// wording ("when the shortfall would fail the next depart") with it: an explicit buy
/// short of the need holds only when these still leave it short.
#[must_use]
pub(crate) fn later_cover(doc: &Orders, index: usize, good: &CanonicalId) -> Cover {
    let mut units = 0_u64;
    for order in doc.orders.iter().skip(index.saturating_add(1)) {
        if !matches!(order.state, OrderState::Pending | OrderState::Inflight) {
            continue;
        }
        match &order.what {
            What::Depart { .. } => break,
            What::Buy { good: g, qty, .. } if g == good => match qty {
                Qty::Fill => return Cover::Fill,
                Qty::Units(n) => units = units.saturating_add(*n),
                Qty::All => {}
            },
            What::Buy { .. } | What::Sell { .. } | What::Stop => {}
        }
    }
    Cover::Units(units)
}

/// **The fill targets of the first depart at or after queue place `from`** over
/// `project_cargo` of the orders from `from` (inclusive) up to it, at its
/// `not_before` (D-18-41, reviewer F10(2)); `Ok(None)` when no depart follows. A buy
/// at `from` is never a depart, so for the step and its reserve this is "the next
/// depart after this order"; when `from` is itself a depart (the `+ship/plan`
/// affordable line with a depart at the cursor) it is that depart, over an empty
/// projection.
#[allow(clippy::too_many_arguments)] // the step's whole context
pub(crate) fn next_depart_targets(
    w: &World,
    planner: &Planner<'_>,
    def: &PortDef,
    orders: &Orders,
    from: usize,
    readings: &[Reading],
    occupants: u32,
) -> Result<Option<DepartTargets>, TargetError> {
    let open = |state: OrderState| {
        matches!(
            state,
            OrderState::Pending | OrderState::Inflight | OrderState::Held
        )
    };
    let Some((offset, depart)) = orders
        .orders
        .iter()
        .enumerate()
        .skip(from)
        .find(|(_, order)| open(order.state) && matches!(order.what, What::Depart { .. }))
    else {
        return Ok(None);
    };
    let What::Depart {
        to,
        not_before,
        not_after_s,
        throttle_permille,
        via,
        ..
    } = &depart.what
    else {
        return Ok(None);
    };
    let options = PassageOptions::for_order(*throttle_permille, via.clone());
    let reached = depart
        .reached
        .as_ref()
        .filter(|reached| reached.venue == def.venue)
        .map_or(w.now(), |reached| reached.utc);
    let due = resolve_due(not_before, *not_after_s, reached);
    let port = crate::port::voyage::venue_name(w, to);
    let routes = def
        .routes
        .get(to)
        .cloned()
        .filter(|routes| !routes.is_empty())
        .ok_or_else(|| TargetError::NoRoute(no_route_line(w, def)))?;
    let between: Vec<&What> = orders
        .orders
        .iter()
        .skip(from)
        .take(offset.saturating_sub(from))
        .filter(|order| open(order.state))
        .map(|order| &order.what)
        .collect();
    let (cargo, extra) = project_cargo(planner.goods, readings, &between)
        .map_err(|e| TargetError::Fill(FillError::Core(e), port.clone()))?;
    let stores = stores(planner.goods, readings)
        .map_err(|e| TargetError::Fill(FillError::Core(e), port.clone()))?;
    let tank = contents_of(readings, &stores.tank.id)
        .get(&stores.propellant)
        .copied()
        .unwrap_or(0);
    let locker = contents_of(readings, &stores.locker.id)
        .get(&stores.provisions)
        .copied()
        .unwrap_or(0);
    let depart_ic = plan_instant(w, due);
    let targets = fill_targets(
        planner.goods,
        readings,
        &cargo,
        extra,
        tank,
        locker,
        occupants.max(1),
        planner.spec.allowance_kg,
        |payload| planner.plan(&routes, payload, occupants.max(1), depart_ic, &options),
    )
    .map_err(|e| TargetError::Fill(e, port.clone()))?;
    Ok(Some(DepartTargets {
        to: to.clone(),
        targets,
    }))
}

/// What is aboard of a consumable, from the readings.
fn aboard_of(goods: &GoodsTable, readings: &[Reading], kind: Consumable) -> u64 {
    let Some(good) = consumable_good(goods, kind) else {
        return 0;
    };
    readings
        .iter()
        .filter_map(|(_, _, contents)| contents.get(&good))
        .sum()
}

/// **The reserve for a buy at queue place `index`** (L17): the next depart's targets
/// from this order inclusive, priced at this venue's asks, `excluding` the order's own
/// consumable; zero when no depart follows.
#[allow(clippy::too_many_arguments)] // the step's whole context
pub(crate) fn reserve_at(
    w: &World,
    planner: &Planner<'_>,
    def: &PortDef,
    map: &PortMap,
    orders: &Orders,
    index: usize,
    readings: &[Reading],
    occupants: u32,
    excluding: Option<Consumable>,
) -> Result<Amount, TargetError> {
    let Some(next) = next_depart_targets(w, planner, def, orders, index, readings, occupants)?
    else {
        return Ok(Amount::from_minor(0));
    };
    let goods = planner.goods;
    Ok(reserve_for(
        &next.targets,
        aboard_of(goods, readings, Consumable::Propellant),
        aboard_of(goods, readings, Consumable::Provisions),
        consumable_ask(w, def, map, goods, Consumable::Propellant),
        consumable_ask(w, def, map, goods, Consumable::Provisions),
        excluding,
    ))
}

// ---- +ship/plan over the queue (IC11 item 5) ----------------------------------------

/// `{n} {unit}` of `good`.
fn qty_text(goods: &GoodsTable, good: &CanonicalId, n: u64) -> String {
    format!("{n} {}", unit_of(goods, good))
}

/// `; short of {want} {unit}: {bounds}` when short.
fn short_text(
    goods: &GoodsTable,
    good: &CanonicalId,
    want: u64,
    got: u64,
    bounds: &[Bound],
) -> String {
    if got >= want || bounds.is_empty() {
        String::new()
    } else {
        format!(
            "; short of {}: {}",
            qty_text(goods, good, want),
            join_bounds(bounds)
        )
    }
}

/// A price per unit (`280.00cr/t`).
fn per_unit(w: &World, goods: &GoodsTable, good: &CanonicalId, price: Amount) -> String {
    format!("{}/{}", money(w, price), unit_of(goods, good))
}

/// Add `n` of `good` to the projected readings.
fn load_into(goods: &GoodsTable, readings: &mut [Reading], good: &CanonicalId, n: i128) {
    let comps: Vec<Compartment> = readings.iter().map(|(_, comp, _)| comp.clone()).collect();
    let Some(comp) = capacity::compartment_for(goods, &comps, good) else {
        return;
    };
    if let Some((_, _, contents)) = readings.iter_mut().find(|(id, _, _)| *id == comp.id) {
        let held = i128::from(contents.get(good).copied().unwrap_or(0));
        let after = u64::try_from(held.saturating_add(n).max(0)).unwrap_or(u64::MAX);
        if after == 0 {
            contents.remove(good);
        } else {
            contents.insert(good.clone(), after);
        }
    }
}

/// The projected payload of the readings, kilograms.
fn payload_of(
    goods: &GoodsTable,
    readings: &[Reading],
    occupants: u32,
    allowance: u32,
) -> Option<u64> {
    let mut total = u64::from(occupants).checked_mul(u64::from(allowance))?;
    for (_, _, contents) in readings {
        for (good, qty) in contents {
            total = total.checked_add(qty.checked_mul(goods.good(good)?.mass_kg_per_unit()?)?)?;
        }
    }
    Some(total)
}

/// **`+ship/plan`'s queue block** (doc 18 §4.4 "`+ship/plan [<port>]` (a dry run of the
/// queue against live listings, pools, funds, burn and ETA; writes nothing)"; the §5
/// row "each buy's clamp and bound, the affordable tonnage at this board"): the orders
/// from the cursor up to and including the first depart, each through this module's
/// functions over a projection of the live readings and the pilot's available funds.
/// Writes nothing.
#[allow(clippy::too_many_lines)] // one simulated walk, one line per order
pub(crate) fn plan_queue(
    ctx: &Ctx<'_>,
    view: &HullView<'_>,
    def: &PortDef,
    map: &PortMap,
    orders: &Orders,
) -> Vec<String> {
    let w: &World = ctx.w;
    let Some(cursor) = orders.cursor else {
        return Vec::new();
    };
    let Some(start) = orders.position(cursor) else {
        return Vec::new();
    };
    let name = name_of(w, view.hull());
    let dark = vec![
        format!("The sailing plan from order {cursor}, as it stands now:"),
        crate::port::voyage::PLAN_DARK
            .replace("{name}", &name)
            .replace(crate::port::voyage::NOTHING_BURNED, ""),
    ];
    let Ok(planner) = planner(w, view).and_then(|planner| planner.carrying(ctx, view)) else {
        return dark;
    };
    let goods = planner.goods;
    let Ok(mut readings) = crate::port::ship::readings_of(ctx, view) else {
        return dark;
    };
    let live_readings = readings.clone();
    let occupants = view
        .airlock()
        .map_or(0, |airlock| {
            u32::try_from(crate::port::voyage::souls_aboard(w, airlock).len()).unwrap_or(u32::MAX)
        })
        .max(1);
    let available = crate::port::credit::own_account(w, orders.author)
        .and_then(mush_economy::Account::available)
        .unwrap_or(Amount::from_minor(0));
    let mut funds = i128::from(available.minor());
    let funds_amount =
        |funds: i128| Amount::from_minor(i64::try_from(funds.max(0)).unwrap_or(i64::MAX));
    let mut lines = vec![format!(
        "The sailing plan from order {cursor}, as it stands now:"
    )];
    let mut drawn: BTreeMap<ListingId, u64> = BTreeMap::new();
    let mut taken: BTreeMap<ListingId, u64> = BTreeMap::new();
    let mut after_depart: Vec<u32> = Vec::new();
    let mut depart_port: Option<String> = None;
    for (index, order) in orders.orders.iter().enumerate().skip(start) {
        if !matches!(
            order.state,
            OrderState::Pending | OrderState::Inflight | OrderState::Held
        ) {
            continue;
        }
        if depart_port.is_some() {
            after_depart.push(order.id);
            continue;
        }
        let id = order.id;
        if order.state == OrderState::Inflight {
            lines.push(format!("  Order {id} is at the counter."));
            continue;
        }
        match &order.what {
            What::Stop => {
                lines.push(format!("  Order {id}, stop: the plan pauses here."));
                break;
            }
            What::Sell {
                good,
                qty,
                min_major,
                ..
            } => {
                let label = label_of(goods, good);
                let Some(bid) = bid_at(w, def, map, good) else {
                    lines.push(format!(
                        "  Order {id}, sell {label}: {}",
                        NO_BID.replace("{label}", &label)
                    ));
                    continue;
                };
                let aboard = readings
                    .iter()
                    .filter_map(|(_, _, contents)| contents.get(good))
                    .sum::<u64>();
                let want = match qty {
                    Qty::Units(n) => *n,
                    Qty::All | Qty::Fill => aboard,
                };
                let remaining = bid
                    .remaining
                    .saturating_sub(taken.get(&bid.listing).copied().unwrap_or(0));
                let under = min_major.is_some_and(|min| {
                    crate::port::apply::minor_of(min, crate::port::apply::credit_scale(w))
                        .is_some_and(|floor| bid.price.minor() < floor)
                });
                let (q, bounds) = clamp_sell(want, under, aboard, remaining, bid.house_units);
                let gross = i128::from(bid.price.minor()).saturating_mul(i128::from(q));
                let net = sale_net(w, ctx, orders, def, good, &bid, q).map_or(gross, i128::from);
                funds = funds.saturating_add(net);
                let slot = taken.entry(bid.listing).or_insert(0);
                *slot = slot.saturating_add(q);
                load_into(goods, &mut readings, good, -i128::from(q));
                lines.push(format!(
                    "  Order {id}, sell {label}: {} to {} at {}{}.",
                    qty_text(goods, good, q),
                    bid.house,
                    per_unit(w, goods, good, bid.price),
                    short_text(goods, good, want, q, &bounds)
                ));
            }
            What::Buy {
                good,
                qty,
                max_major,
                ..
            } => {
                let label = label_of(goods, good);
                let consumable = goods.good(good).and_then(|row| row.consumable);
                let ask = ask_at(w, def, map, good);
                let stock = ask.as_ref().map_or(0, |ask| {
                    ask.quantity
                        .saturating_sub(drawn.get(&ask.listing).copied().unwrap_or(0))
                });
                let price = ask.as_ref().map_or(Amount::from_minor(0), |ask| ask.price);
                let over = max_major.is_some_and(|max| {
                    crate::port::apply::minor_of(max, crate::port::apply::credit_scale(w))
                        .is_some_and(|ceiling| price.minor() > ceiling)
                });
                let room = match room_for(goods, &readings, good) {
                    Ok(room) => room,
                    Err(_) => return dark,
                };
                // The fill targets (and the reserve's), over the projection.
                let reserve = match reserve_at(
                    w, &planner, def, map, orders, index, &readings, occupants, consumable,
                ) {
                    Ok(reserve) => reserve,
                    Err(TargetError::NoRoute(_) | TargetError::Fill(..)) => Amount::from_minor(0),
                };
                let (want, fill_line) = match (qty, consumable) {
                    (Qty::Fill, Some(kind)) => {
                        match next_depart_targets(
                            w, &planner, def, orders, index, &readings, occupants,
                        ) {
                            Ok(Some(next)) => {
                                let target = match kind {
                                    Consumable::Propellant => next.targets.tank,
                                    Consumable::Provisions => next.targets.locker,
                                };
                                let aboard = aboard_of(goods, &readings, kind);
                                (
                                    target.saturating_sub(aboard),
                                    Some((next.targets.cargo_t, target)),
                                )
                            }
                            Ok(None) => {
                                lines.push(format!(
                                    "  Order {id}, buy {label}, fill: it would hold: {}",
                                    HELD_NO_DEPART
                                ));
                                continue;
                            }
                            Err(TargetError::NoRoute(line)) => {
                                lines.push(format!(
                                    "  Order {id}, buy {label}, fill: it would hold: {line}"
                                ));
                                continue;
                            }
                            Err(TargetError::Fill(error, port)) => {
                                lines.push(format!(
                                    "  Order {id}, buy {label}, fill: it would hold: {}",
                                    fill_reason(goods, &error, &port)
                                ));
                                continue;
                            }
                        }
                    }
                    (Qty::Units(n), _) => (*n, None),
                    (Qty::Fill | Qty::All, _) => (0, None),
                };
                // No live ask: stock alone bites (the step's rule).
                let affordable = ask.as_ref().map_or(u64::MAX, |_| {
                    funds_units(funds_amount(funds), reserve, price)
                });
                let (q, bounds) = clamp_buy(want, over, stock, room, affordable);
                if let Some(ask) = &ask {
                    let slot = drawn.entry(ask.listing).or_insert(0);
                    *slot = slot.saturating_add(q);
                }
                funds =
                    funds.saturating_sub(i128::from(price.minor()).saturating_mul(i128::from(q)));
                load_into(goods, &mut readings, good, i128::from(q));
                match fill_line {
                    Some((cargo_t, _)) => {
                        let comps: Vec<Compartment> =
                            readings.iter().map(|(_, comp, _)| comp.clone()).collect();
                        let comp = capacity::compartment_for(goods, &comps, good);
                        let word = comp
                            .as_ref()
                            .map_or_else(String::new, |c| gauges::compartment_words(&c.id).0);
                        let after = consumable.map_or(0, |kind| aboard_of(goods, &readings, kind));
                        let limit = comp
                            .as_ref()
                            .and_then(|c| capacity::max_more(goods, c, &BTreeMap::new(), good).ok())
                            .map_or(0, |(units, _)| units);
                        lines.push(format!(
                            "  Order {id}, buy {label}, fill: {}, {word} {after}/{limit}, for the {cargo_t} t your orders can load{}.",
                            qty_text(goods, good, q),
                            short_text(goods, good, want, q, &bounds)
                        ));
                    }
                    None => lines.push(format!(
                        "  Order {id}, buy {label}: {} at {}{}.",
                        qty_text(goods, good, q),
                        per_unit(w, goods, good, price),
                        short_text(goods, good, want, q, &bounds)
                    )),
                }
            }
            What::Depart {
                to,
                not_before,
                not_after_s,
                throttle_permille,
                via,
                ..
            } => {
                let port = crate::port::voyage::venue_name(w, to);
                depart_port = Some(port.clone());
                let options = PassageOptions::for_order(*throttle_permille, via.clone());
                let reached = order
                    .reached
                    .as_ref()
                    .filter(|reached| reached.venue == def.venue)
                    .map_or(w.now(), |reached| reached.utc);
                let due = resolve_due(not_before, *not_after_s, reached);
                let Some(routes) = def.routes.get(to).filter(|routes| !routes.is_empty()) else {
                    lines.push(format!(
                        "  Order {id}, depart for {port}: it would hold: {}",
                        no_route_line(w, def)
                    ));
                    continue;
                };
                // Doc 20 cut SM.3a: the projected hold plus the bay's rigs, as the filing
                // counts them.
                let Some(payload) =
                    payload_of(goods, &readings, occupants, planner.spec.allowance_kg)
                        .and_then(|payload| payload.checked_add(planner.carried_kg))
                else {
                    return dark;
                };
                // Doc 20 cut SM.2 (the SM.2 fixer, the lead's item 3): the dry run reads the
                // filing's own checks — the site gate (`SITE_CLOSED`), the planner's
                // refusals, the stay's charge, the leg home a site holds back, the tank and
                // the locker (`voyage::site_gate`, `plan_refusal`, `hold_back`) — on the
                // projected pools, so `+ship/orders` reads the refusal the fire would hold
                // with.
                let dest = crate::port::voyage::Destination {
                    venue: to.clone(),
                    port: port.clone(),
                    routes: routes.clone(),
                };
                let site = match crate::port::voyage::site_gate(w, &dest) {
                    Ok(site) => site.map(|(_, site)| site),
                    Err(closed) => {
                        lines.push(format!(
                            "  Order {id}, depart for {port}: it would hold: {}.",
                            inline_reason(&closed.line)
                        ));
                        continue;
                    }
                };
                let Ok(basis) = crate::port::voyage::basis(ctx, view, occupants) else {
                    return dark;
                };
                let mut basis = crate::port::voyage::apply_options(basis, &options);
                basis.payload = payload;
                basis.tank = aboard_of(goods, &readings, Consumable::Propellant);
                basis.locker = aboard_of(goods, &readings, Consumable::Provisions);
                basis.depart_ic = plan_instant(w, due);
                match crate::port::voyage::plan_on(w, &basis, routes) {
                    Ok(mut plan) => {
                        // The refusal's sentence inside the line: a dry run burns
                        // nothing, so "Nothing was burned." goes (`plan_dark`'s rule),
                        // and the line's own full stop ends it.
                        let held = crate::port::voyage::hold_back(
                            w,
                            view.hull(),
                            &name,
                            &basis,
                            &port,
                            def,
                            site,
                            &mut plan,
                        );
                        let hold = match held {
                            Ok(_) => String::new(),
                            Err(refused)
                                if refused.kind
                                    == crate::port::voyage::DepartRefusalKind::PlanDark =>
                            {
                                lines.push(format!(
                                    "  Order {id}, depart for {port}: it would hold: {}",
                                    HELD_FILL_PLAN.replace("{port}", &port)
                                ));
                                continue;
                            }
                            Err(refused) => {
                                format!("; it would hold: {}", inline_reason(&refused.line))
                            }
                        };
                        lines.push(format!(
                            "  Order {id}, depart for {port}: payload {} t, burn {} t, {} crew-days{hold}.",
                            tonnes_one_decimal(payload),
                            plan.burn_t,
                            plan.crew_days
                        ));
                    }
                    Err(PlanError::Overloaded {
                        payload_kg,
                        ceiling_kg,
                    }) => lines.push(format!(
                        "  Order {id}, depart for {port}: payload {} t; it would hold: {}.",
                        crate::port::voyage::tonnes_over(payload_kg, ceiling_kg),
                        inline_reason(
                            &OVERLOADED
                                .replace("{name}", &name)
                                .replace(
                                    "{payload}",
                                    &crate::port::voyage::tonnes_over(payload_kg, ceiling_kg),
                                )
                                .replace("{ceiling}", &(ceiling_kg / 1_000).to_string())
                        )
                    )),
                    // Doc 20 cut SM.2 (D-20-3): the landing law, as the fire holds it.
                    Err(error) => match crate::port::voyage::plan_refusal(w, &name, error) {
                        Ok(refused) => lines.push(format!(
                            "  Order {id}, depart for {port}: it would hold: {}.",
                            inline_reason(&refused.line)
                        )),
                        Err(_) => lines.push(format!(
                            "  Order {id}, depart for {port}: it would hold: {}",
                            HELD_FILL_PLAN.replace("{port}", &port)
                        )),
                    },
                }
            }
        }
    }
    if let (Some(port), Some(first), Some(last)) =
        (depart_port, after_depart.first(), after_depart.last())
    {
        if first == last {
            lines.push(format!(
                "  Order {first} runs at {port}; it is planned there."
            ));
        } else {
            lines.push(format!(
                "  Orders {first} to {last} run at {port}; they are planned there."
            ));
        }
    }
    // The affordable tonnage at this board (the §5 row): the pilot's available funds
    // now, less the reserve of the next depart's targets (no exclusion).
    let reserve = reserve_at(
        w,
        &planner,
        def,
        map,
        orders,
        start,
        &live_readings,
        occupants,
        None,
    )
    .unwrap_or(Amount::from_minor(0));
    let mut asked: Vec<&CanonicalId> = Vec::new();
    for ask in def.vendors.iter().flat_map(|vendor| vendor.asks.iter()) {
        if !asked.contains(&&ask.good) {
            asked.push(&ask.good);
        }
    }
    let parts: Vec<String> = asked
        .into_iter()
        .filter_map(|good| {
            let ask = ask_at(w, def, map, good)?;
            Some(format!(
                "{} {}",
                label_of(goods, good),
                qty_text(goods, good, funds_units(available, reserve, ask.price))
            ))
        })
        .collect();
    if !parts.is_empty() {
        lines.push(format!(
            "Affordable at this board, after the next departure's fuel and stores: {}.",
            parts.join(", ")
        ));
    }
    lines
}

/// A departure refusal's sentence as a dry-run clause: without " Nothing was burned."
/// (a dry run burns nothing; `voyage.rs`'s `plan_dark` rule) and without its own
/// closing full stop (the plan line ends it).
fn inline_reason(sentence: &str) -> String {
    sentence
        .replace(crate::port::voyage::NOTHING_BURNED, "")
        .trim_end_matches('.')
        .to_owned()
}

/// The net a sale would pay the pilot through the market's proceeds policy (gross −
/// Σ cuts); `None` when the policy refuses or cannot be asked.
fn sale_net(
    w: &World,
    ctx: &Ctx<'_>,
    orders: &Orders,
    def: &PortDef,
    good: &CanonicalId,
    bid: &Bid,
    qty: u64,
) -> Option<i64> {
    use mush_market::ProceedsPolicy;
    let _ = ctx;
    let market = w.module_state::<MarketStore>()?;
    let listing = market.listing(bid.listing)?;
    let view = crate::port::ship::hull_view(w, orders.hull)?;
    let vault = crate::port::ship::hold_vault(w, &view)?;
    let venue = mush_market::MarketId::new(def.venue.as_str())?;
    let gross = bid.price.minor().checked_mul(i64::try_from(qty).ok()?)?;
    let proceeds = crate::port::proceeds::CoopProceeds
        .proceeds(
            w,
            &mush_market::SaleTerms {
                seller: orders.author,
                source: mush_inventory::record::EndpointRef(vault),
                house: &listing.seller,
                market: &venue,
                listing: bid.listing,
                commodity: good,
                quantity: qty,
                price: bid.price,
                gross: Amount::from_minor(gross),
            },
        )
        .ok()?;
    let cuts: i64 = proceeds.cuts.iter().map(|cut| cut.amount.minor()).sum();
    gross.checked_sub(cuts)
}

/// The sentence a fill that cannot be met holds with.
pub(crate) fn fill_reason(goods: &GoodsTable, error: &FillError, port: &str) -> String {
    let (comp, need, good) = match error {
        FillError::TankTooSmall { need, .. } => (
            "tank",
            *need,
            consumable_good(goods, Consumable::Propellant),
        ),
        FillError::LockerTooSmall { need, .. } => (
            "locker",
            *need,
            consumable_good(goods, Consumable::Provisions),
        ),
        FillError::Plan(_) | FillError::NoFixedPoint | FillError::Core(_) => {
            return HELD_FILL_PLAN.replace("{port}", port);
        }
    };
    let unit = good.map_or_else(String::new, |good| unit_of(goods, &good));
    HELD_FILL
        .replace("{compartment}", comp)
        .replace("{need}", &need.to_string())
        .replace("{unit}", &unit)
        .replace("{port}", port)
}

// ---- the order arm (the `tacsim.port` tick; IC11 item 8) ----------------------------

/// The market's refusal as a line (a pure local map: never `refuse_market` from the
/// tick, which would count the refusal against the actor; reviewer F6).
#[must_use]
pub(crate) fn market_line(error: &MarketError) -> String {
    match error {
        MarketError::NoStanding(line) => (*line).to_owned(),
        MarketError::Bad(line) => line.clone(),
        MarketError::Contract(error) => error.to_string(),
        MarketError::Economy(error) => error.to_string(),
    }
}

/// **Whether a market refusal is engine back-pressure** (doc 18 cut IC14 row 2;
/// reviewer F3): the sell gate's own-queue line (`SELL_BUSY`) and its full-queue line
/// (`SELL_QUEUE_FULL`). Neither refuses the order: the step waits a fire, exactly as
/// `SwapRefused::Pending` does (which pre-checks only the pilot's own entry), with no
/// log line, no refusal count and no document write.
fn is_back_pressure(line: &str) -> bool {
    line == mush_market::SELL_QUEUE_FULL || line == mush_market::SELL_BUSY
}

/// The kind of act an order stages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActKind {
    Buy,
    Sell,
    Depart,
    Stop,
}

fn act_kind(what: &What) -> ActKind {
    match what {
        What::Buy { .. } => ActKind::Buy,
        What::Sell { .. } => ActKind::Sell,
        What::Depart { .. } => ActKind::Depart,
        What::Stop => ActKind::Stop,
    }
}

/// The kind of the inflight step of `orders`.
fn inflight_kind(orders: &Orders) -> Option<ActKind> {
    let inflight = orders.inflight.as_ref()?;
    orders
        .order(inflight.order)
        .map(|order| act_kind(&order.what))
}

/// **The order agents this fire** (L15; reviewer F12): every queued sailing plan that
/// reads, whose hull has no pending voyage (a filed hull is the launch arm's), and that
/// either has a market leg in flight with the plan not held, or runs with a cursor.
/// Agent = the hull, principal = the document's author.
pub(crate) fn order_agents(w: &World, state: &PortTickState) -> Vec<(ObjId, ObjId)> {
    let _ = state;
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Vec::new();
    };
    store
        .orders_docs()
        .filter(|orders| crate::port::voyage::pending_of(w, orders.hull).is_none())
        .filter(|orders| {
            let leg = matches!(inflight_kind(orders), Some(ActKind::Buy | ActKind::Sell))
                && orders.state != PlanState::Held;
            leg || (orders.state == PlanState::Running && orders.cursor.is_some())
        })
        .map(|orders| (orders.hull, orders.author))
        .collect()
}

/// What one step wants written and told: the next document, the lines told to the
/// author, and the Bridge chime.
struct Outcome {
    doc: Orders,
    notices: Vec<String>,
    narration: Option<String>,
}

impl Outcome {
    fn new(doc: Orders) -> Outcome {
        Outcome {
            doc,
            notices: Vec::new(),
            narration: None,
        }
    }

    /// Log `text` against order `id`, and tell it when `tell`.
    fn log(&mut self, w: &World, id: u32, text: &str, tell: bool) {
        self.doc.push_log(w.now(), w.world_ic_now().at, id, text);
        if tell {
            self.notices.push(crate::port::orders::clamp_line(text));
        }
    }
}

/// Write the outcome (the cursor derived, ONE checked `put_orders`), tell the author,
/// chime; `ok` on a write, `Refused` when the document would not check (paged by
/// `put_orders`).
fn commit(ctx: &mut Ctx<'_>, mut outcome: Outcome, ok: Step) -> Step {
    outcome.doc.derive_cursor();
    let hull = outcome.doc.hull;
    let author = outcome.doc.author;
    if put_orders(ctx, &outcome.doc).is_err() {
        return Step::Refused;
    }
    for line in &outcome.notices {
        notify(ctx, author, line);
    }
    if let Some(chime) = &outcome.narration {
        narrate_hull(ctx, hull, chime);
    }
    ok
}

/// **A staff stand-down recorded on the plan** (reviewer F4, lead ruling L28; called by
/// `@tacsim/port clear`): when the hull's plan reads with order `order` in flight at
/// `attempt`, the order holds (stopped, `HELD_STOOD_DOWN`), its attempt consumed and its
/// step forgotten, one log line written; the pilot hears only the stand-down notice.
/// `false` (logged) when the plan does not match, and today's behaviour stands.
pub(crate) fn stand_down(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    order: u32,
    attempt: u32,
    port: &str,
) -> bool {
    let mut doc = match orders_state(ctx.w, hull) {
        OrdersState::Present(doc)
            if doc
                .inflight
                .as_ref()
                .is_some_and(|i| i.order == order && i.attempt == attempt) =>
        {
            doc.clone()
        }
        OrdersState::Present(_) | OrdersState::Absent | OrdersState::Unreadable => {
            tracing::warn!(
                hull = hull.0,
                order,
                attempt,
                "tacsim: a stood-down queued departure's sailing plan does not name it in flight; nothing is recorded on the plan"
            );
            return false;
        }
    };
    let reason = HELD_STOOD_DOWN.replace("{port}", port);
    doc.inflight = None;
    if let Some(entry) = doc.order_mut(order) {
        entry.state = OrderState::Held;
        entry.attempt = attempt.saturating_add(1);
        entry.refusals = 0;
    }
    let now = ctx.w.now();
    doc.stop_at(order, &reason, now);
    let line = HELD_LINE
        .replace("{n}", &order.to_string())
        .replace("{reason}", &reason);
    doc.push_log(now, ctx.w.world_ic_now().at, order, &line);
    doc.derive_cursor();
    match put_orders(ctx, &doc) {
        Ok(()) => true,
        Err(error) => {
            tracing::error!(hull = hull.0, order, %error, "tacsim: a stand-down could not be recorded on the sailing plan; the stand-down proceeds");
            false
        }
    }
}

/// The verdict on one inflight step, by pull.
enum Verdict {
    Landed,
    Dropped,
    Pending,
    /// A depart's ticket reads with its pending gone and no journey: a stand-down the
    /// staff terminal could not record.
    StoodDown,
    /// A depart's ticket does not read and its pending is gone: hold over the step.
    Unconfirmed,
}

/// A market leg's verdict.
fn leg_verdict(w: &World, kind: ActKind, refs: &crate::port::orders::InflightRefs) -> Verdict {
    let Some(market) = w.module_state::<MarketStore>() else {
        return Verdict::Pending;
    };
    let Some(quote) = refs.quote.and_then(|q| market.quote(QuoteId(q))) else {
        return Verdict::Dropped;
    };
    let contract = quote
        .contract
        .or(refs.contract.map(ContractId))
        .and_then(|c| {
            w.module_state::<EconomyStore>()
                .and_then(|e| e.snapshot().contract(c).map(|c| c.state))
        });
    let settled_contract = matches!(contract, Some(ContractState::Settled { .. }));
    let terminal = contract.is_some_and(|state| state.is_terminal());
    match kind {
        ActKind::Buy => {
            if quote.drawn_at.is_some() || quote.settled.is_some() {
                Verdict::Landed
            } else if terminal {
                Verdict::Dropped
            } else {
                Verdict::Pending
            }
        }
        ActKind::Sell => {
            if quote.settled.is_some() || settled_contract {
                Verdict::Landed
            } else if terminal {
                Verdict::Dropped
            } else {
                Verdict::Pending
            }
        }
        ActKind::Depart | ActKind::Stop => Verdict::Pending,
    }
}

/// A depart's verdict and, when it launched, its ticket.
fn depart_verdict(
    ctx: &Ctx<'_>,
    hull: ObjId,
    refs: &crate::port::orders::InflightRefs,
) -> (Verdict, Option<crate::port::families::Ticket>) {
    let Some(id) = refs.ticket.as_deref().and_then(TicketId::parse) else {
        return (Verdict::Dropped, None);
    };
    let pending = crate::port::voyage::pending_of(ctx.w, hull).is_some_and(|p| p.seq == id.seq);
    match crate::port::voyage::ticket_at(ctx, id) {
        Ok(Some(ticket)) => {
            if crate::port::voyage::launched(ctx.w, hull, &ticket.route, ticket.plan.depart_ic) {
                (Verdict::Landed, Some(ticket))
            } else if pending {
                (Verdict::Pending, None)
            } else {
                (Verdict::StoodDown, None)
            }
        }
        Ok(None) if pending => (Verdict::Pending, None),
        Ok(None) => (Verdict::Dropped, None),
        Err(()) if pending => (Verdict::Pending, None),
        Err(()) => (Verdict::Unconfirmed, None),
    }
}

/// **The order arm's pull verification** (top of every fire; no swap; reviewer F12):
/// every unreadable plan pages once a boot; then the readable plans with a step in
/// flight, in hull order rotating from `order_verify_after`, at most
/// [`MAX_ORDER_VERIFY_PER_FIRE`], each judged Landed, Dropped or Pending.
pub(crate) fn order_verify(ctx: &mut Ctx<'_>, state: &mut PortTickState) {
    let (unreadable, mut inflight): (Vec<ObjId>, Vec<ObjId>) =
        match ctx.w.module_state::<TacsimStore>() {
            Some(store) => (
                store.orders_unreadable().collect(),
                store
                    .orders_docs()
                    .filter(|doc| doc.inflight.is_some())
                    .map(|doc| doc.hull)
                    .collect(),
            ),
            None => return,
        };
    for hull in unreadable {
        if state.orders_warned.insert(hull.0) {
            tracing::error!(
                hull = hull.0,
                runbook = "@tacsim/port status; repair tacsim:orders:<hull> by hand",
                "tacsim: a queued sailing plan is unreadable; the order arm skips that hull"
            );
        }
    }
    inflight.sort_unstable();
    let cursor = state.order_verify_after;
    let (before, mut from): (Vec<ObjId>, Vec<ObjId>) =
        inflight.into_iter().partition(|hull| hull.0 < cursor);
    from.extend(before);
    let mut last: Option<ObjId> = None;
    for hull in from.into_iter().take(MAX_ORDER_VERIFY_PER_FIRE) {
        last = Some(hull);
        verify_one(ctx, hull);
    }
    if let Some(hull) = last {
        state.order_verify_after = hull.0.saturating_add(1);
    }
}

/// One plan's verification.
#[allow(clippy::too_many_lines)] // one verdict table, written out
fn verify_one(ctx: &mut Ctx<'_>, hull: ObjId) {
    let OrdersState::Present(doc) = orders_state(ctx.w, hull) else {
        return;
    };
    let doc = doc.clone();
    let Some(inflight) = doc.inflight.clone() else {
        return;
    };
    let id = inflight.order;
    let Some(order) = doc.order(id).cloned() else {
        return;
    };
    let kind = act_kind(&order.what);
    let (verdict, ticket) = match kind {
        ActKind::Buy | ActKind::Sell => (leg_verdict(ctx.w, kind, &inflight.refs), None),
        ActKind::Depart => depart_verdict(ctx, hull, &inflight.refs),
        ActKind::Stop => (Verdict::Dropped, None),
    };
    let w: &World = ctx.w;
    let now = w.now();
    let mut out = Outcome::new(doc);
    match verdict {
        Verdict::Pending => {}
        Verdict::Landed => {
            let origin = order.reached.as_ref().map(|r| r.venue.clone());
            if let Some(entry) = out.doc.order_mut(id) {
                entry.state = OrderState::Done;
                entry.refusals = 0;
                entry.retry_at = None;
                entry.reached = None;
            }
            out.doc.inflight = None;
            out.doc.clear_hold_naming(id);
            let mut others: Vec<(ObjId, String)> = Vec::new();
            match (kind, ticket) {
                (ActKind::Depart, Some(ticket)) => {
                    let What::Depart { to, crew, .. } = &order.what else {
                        return;
                    };
                    let remaining = w
                        .module_state::<mush_sim::JourneyBook>()
                        .and_then(|book| book.get(hull))
                        .and_then(|journey| journey.arrival_at())
                        .map_or(ticket.plan.total_s, |at| {
                            at.saturating_sub(w.world_ic_now().at)
                        });
                    let name = name_of(w, hull);
                    let dest = crate::port::voyage::venue_name(w, to);
                    let line = crate::port::voyage::CLEARANCE_GRANTED
                        .replace("{burn}", &ticket.plan.burn_t.to_string())
                        .replace("{days}", &ticket.plan.crew_days.to_string())
                        .replace("{name}", &name)
                        .replace("{port}", &dest)
                        .replace("{icdays}", &crate::port::voyage::ic_days(remaining));
                    // Logged, not told: the launch arm's CLEARANCE_GRANTED is the notice.
                    out.log(w, id, &line, false);
                    // The left-behind rule (Q-18-17, L11): a listed passenger ashore at
                    // the launch is left behind, and both are told.
                    let port = origin.as_ref().map_or_else(
                        || "the port".to_owned(),
                        |venue| crate::port::voyage::venue_name(w, venue),
                    );
                    for person in crew {
                        if crate::port::voyage::aboard(w, hull, *person) {
                            continue;
                        }
                        out.notices.push(
                            LEFT_BEHIND_PILOT
                                .replace("{who}", &name_of(w, *person))
                                .replace("{port}", &port),
                        );
                        others.push((
                            *person,
                            LEFT_BEHIND_PASSENGER
                                .replace("{name}", &name)
                                .replace("{dest}", &dest)
                                .replace("{port}", &port),
                        ));
                    }
                }
                (ActKind::Depart | ActKind::Stop, _) => return,
                (ActKind::Buy | ActKind::Sell, _) => {
                    let line = inflight
                        .line
                        .clone()
                        .unwrap_or_else(|| format!("Order {id} is carried out."));
                    out.log(w, id, &line, true);
                }
            }
            let _ = commit(ctx, out, Step::Worked);
            for (person, line) in others {
                notify(ctx, person, &line);
            }
        }
        Verdict::Dropped => {
            let attempt = inflight.attempt.saturating_add(1);
            out.doc.inflight = None;
            out.log(w, id, &NOT_ADMITTED.replace("{n}", &id.to_string()), false);
            let stopped_over = out
                .doc
                .hold
                .as_ref()
                .is_some_and(|note| note.order == id && !note.retry);
            if let Some(entry) = out.doc.order_mut(id) {
                entry.attempt = attempt;
                entry.state = if stopped_over {
                    OrderState::Held
                } else {
                    OrderState::Pending
                };
                entry.retry_at = Some(now.saturating_add(ORDER_RETRY_SECS));
            }
            if !stopped_over && attempt > MAX_ORDER_ATTEMPTS {
                tracing::error!(
                    hull = hull.0,
                    order = id,
                    attempt,
                    "tacsim: a queued order was not taken up three times; it holds"
                );
                hold_with(w, &mut out, id, HELD_UNCONFIRMED);
            }
            let _ = commit(ctx, out, Step::Worked);
        }
        Verdict::StoodDown => {
            out.doc.inflight = None;
            if let Some(entry) = out.doc.order_mut(id) {
                entry.attempt = inflight.attempt.saturating_add(1);
                entry.state = OrderState::Held;
                entry.refusals = 0;
            }
            let port = match &order.what {
                What::Depart { to, .. } => crate::port::voyage::venue_name(w, to),
                What::Buy { .. } | What::Sell { .. } | What::Stop => String::new(),
            };
            let reason = HELD_STOOD_DOWN.replace("{port}", &port);
            out.doc.stop_at(id, &reason, now);
            let line = HELD_LINE
                .replace("{n}", &id.to_string())
                .replace("{reason}", &reason);
            out.log(w, id, &line, false);
            let _ = commit(ctx, out, Step::Worked);
        }
        Verdict::Unconfirmed => {
            let already = out.doc.hold.as_ref().is_some_and(|note| {
                note.order == id && !note.retry && note.reason == HELD_UNCONFIRMED
            });
            if already {
                return;
            }
            tracing::error!(
                hull = hull.0,
                order = id,
                "tacsim: a queued departure's ticket does not read and its clearance is gone; the plan holds over the step"
            );
            if let Some(record) = out.doc.inflight.as_mut() {
                record.restaged = 2;
            }
            hold_with(w, &mut out, id, HELD_UNCONFIRMED);
            let _ = commit(ctx, out, Step::Worked);
        }
    }
}

/// A stopped hold at `id` with `reason`: logged, told, chimed (L10, L12).
fn hold_with(w: &World, out: &mut Outcome, id: u32, reason: &str) {
    out.doc.stop_at(id, reason, w.now());
    let line = HELD_LINE
        .replace("{n}", &id.to_string())
        .replace("{reason}", reason);
    out.log(w, id, &line, true);
    out.narration = Some(HOLD_NARRATION.replace("{n}", &id.to_string()));
}

/// Skip or hold order `id` per `on_short` (`Partial` skips): logged, told, chimed.
fn skip_or_hold(w: &World, out: &mut Outcome, id: u32, on_short: OnShort, reason: &str) {
    match on_short {
        OnShort::Hold => hold_with(w, out, id, reason),
        OnShort::Skip | OnShort::Partial => {
            if let Some(entry) = out.doc.order_mut(id) {
                entry.state = OrderState::Skipped;
                entry.refusals = 0;
                entry.reached = None;
            }
            // A waiting note on the order passed over goes with it (a note always
            // names the cursor order, and the cursor moves on).
            out.doc.clear_hold_naming(id);
            let line = SKIP_LINE
                .replace("{n}", &id.to_string())
                .replace("{reason}", reason);
            out.log(w, id, &line, true);
            out.narration = Some(SKIP_NARRATION.replace("{n}", &id.to_string()));
        }
    }
}

/// A waiting note at `id`: logged and told only when new or its reason changed;
/// `true` when the document changed.
fn wait_with(w: &World, out: &mut Outcome, id: u32, reason: &str) -> bool {
    if !out.doc.wait_at(id, reason, w.now()) {
        return false;
    }
    let line = WAITING_LINE
        .replace("{n}", &id.to_string())
        .replace("{reason}", reason);
    out.log(w, id, &line, true);
    true
}

/// A refused closed call (reviewer F6): counted, retried after [`ORDER_RETRY_SECS`],
/// logged once (`refusals` 0 → 1), and held at [`MAX_ORDER_REFUSALS`].
fn refused_call(w: &World, out: &mut Outcome, id: u32, line: &str) {
    let refusals = out
        .doc
        .order(id)
        .map_or(0, |o| o.refusals)
        .saturating_add(1);
    if let Some(entry) = out.doc.order_mut(id) {
        entry.refusals = refusals.min(MAX_ORDER_REFUSALS);
        entry.retry_at = Some(w.now().saturating_add(ORDER_RETRY_SECS));
    }
    if refusals == 1 {
        let text = REFUSED_RETRY
            .replace("{n}", &id.to_string())
            .replace("{line}", line);
        out.log(w, id, &text, false);
    }
    if refusals >= MAX_ORDER_REFUSALS {
        hold_with(w, out, id, line);
    }
}

/// The document as it would stand with order `id` in flight under `record` — the
/// pre-check before any closed call (reviewer F2).
fn staged(doc: &Orders, id: u32, record: Inflight) -> Option<Orders> {
    let mut next = doc.clone();
    if next.hold.as_ref().is_some_and(|note| note.retry) {
        next.hold = None;
    }
    let entry = next.order_mut(id)?;
    entry.state = OrderState::Inflight;
    entry.retry_at = None;
    next.inflight = Some(record);
    next.derive_cursor();
    match next.check() {
        Ok(()) => Some(next),
        Err(error) => {
            tracing::error!(hull = doc.hull.0, order = id, %error, "tacsim: a queued step's record would not check; nothing was staged");
            None
        }
    }
}

/// An inflight record with every ref empty.
fn record(id: u32, attempt: u32, key: String, now: i64, ic: i64) -> Inflight {
    Inflight {
        order: id,
        attempt,
        key,
        refs: crate::port::orders::InflightRefs::default(),
        staged_utc: now,
        issued_ic: ic,
        restaged: 0,
        want: 0,
        qty: 0,
        bounds: Vec::new(),
        line: None,
    }
}

/// What the step decided, computed under a shared borrow of the world.
enum Plan {
    Idle,
    Refused,
    /// Write and tell; `Worked`.
    Write(Box<Outcome>),
    /// Re-stage the market leg of `quote`.
    Restage {
        quote: QuoteId,
        sell: bool,
        doc: Box<Orders>,
    },
    /// Buy `qty` from `listing`, then record `pre`.
    Buy {
        listing: ListingId,
        qty: u64,
        pre: Box<Orders>,
    },
    /// Sell `qty` into `listing`, then record `pre`.
    Sell {
        listing: ListingId,
        qty: u64,
        pre: Box<Orders>,
        short: Option<String>,
        on_short: OnShort,
        house_line: String,
    },
    /// File the departure, then record `pre` (the reach written first if deferred).
    Depart {
        filing: Box<crate::port::voyage::DepartFiling>,
        pre: Box<Orders>,
        reach: Box<Orders>,
    },
}

/// **The order step for one hull** (the IC11 brief's item 8): the act due this fire,
/// the lease before any act, then ONE closed typed call as the author inside
/// `Fire::as_principal` — or a hold, a skip, a note, or nothing.
pub(crate) fn order_step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    hull: ObjId,
    pilot: ObjId,
) -> Step {
    if gone(ctx.w, hull) {
        return Step::Refused;
    }
    let plan = decide(ctx, state, hull, pilot);
    match plan {
        Plan::Idle => Step::Idle,
        Plan::Refused => Step::Refused,
        Plan::Write(outcome) => commit(ctx, *outcome, Step::Worked),
        Plan::Restage { quote, sell, doc } => {
            let called = fire.as_principal(ctx, hull, pilot, |ctx| {
                if sell {
                    mush_market::restage_deliver(ctx, quote)
                } else {
                    mush_market::restage_draw(ctx, quote)
                }
            });
            let result = match called {
                Ok(result) => result,
                Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => {
                    return Step::Deferred;
                }
                Err(SwapRefused::God | SwapRefused::Gone) => return Step::Refused,
            };
            restaged(ctx, *doc, result)
        }
        Plan::Buy { listing, qty, pre } => {
            let called = fire.as_principal(ctx, hull, pilot, |ctx| {
                let quote = mush_market::quote(
                    ctx,
                    &QuoteRequest {
                        listing,
                        quantity: qty,
                        window: QuoteWindow::Live,
                        ttl_live: QUOTE_TTL_LIVE,
                        ttl_remote: QUOTE_TTL_REMOTE,
                        presence: Presence::Ship(hull),
                    },
                )?;
                mush_market::accept_at(ctx, quote, None, Presence::Ship(hull))
                    .map(|contract| (quote, contract))
            });
            match called {
                Ok(Ok((quote, contract))) => {
                    let mut doc = *pre;
                    if let Some(record) = doc.inflight.as_mut() {
                        record.refs.quote = Some(quote.0);
                        record.refs.contract = Some(contract.0);
                        record.key = format!("market.lot.draw.{:x}", contract.0);
                    }
                    commit(ctx, Outcome::new(doc), Step::Submitted)
                }
                Ok(Err(error)) => refused_after_call(ctx, hull, &pre, &error),
                Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => {
                    Step::Deferred
                }
                Err(SwapRefused::God | SwapRefused::Gone) => Step::Refused,
            }
        }
        Plan::Sell {
            listing,
            qty,
            pre,
            short,
            on_short,
            house_line,
        } => {
            let called = fire.as_principal(ctx, hull, pilot, |ctx| {
                mush_market::sell_into_bid(
                    ctx,
                    &BidSaleRequest {
                        listing,
                        quantity: qty,
                        source: None,
                        presence: Presence::Ship(hull),
                    },
                )
            });
            match called {
                Ok(Ok(sale)) => {
                    let mut doc = *pre;
                    if let Some(record) = doc.inflight.as_mut() {
                        record.refs.quote = Some(sale.quote.0);
                        record.refs.contract = Some(sale.contract.0);
                        record.key = format!("market.lot.deliver.{:x}", sale.contract.0);
                        let mut line = sale.line.clone();
                        if let Some(short) = &short {
                            line.push_str(short);
                        }
                        record.line = Some(crate::port::orders::clamp_line(&line));
                    }
                    commit(ctx, Outcome::new(doc), Step::Submitted)
                }
                Ok(Err(error)) if market_line(&error) == mush_market::HOUSE_CANNOT_PAY => {
                    // Reviewer F13: the house's accept refused (its party cap) is a
                    // skip-or-hold, never the retry path.
                    let Some(id) = pre.inflight.as_ref().map(|i| i.order) else {
                        return Step::Refused;
                    };
                    let OrdersState::Present(doc) = orders_state(ctx.w, hull) else {
                        return Step::Refused;
                    };
                    let mut out = Outcome::new(doc.clone());
                    skip_or_hold(ctx.w, &mut out, id, on_short, &house_line);
                    commit(ctx, out, Step::Worked)
                }
                // Doc 18 cut IC14 row 2 (reviewer F3): a full engine queue (or the
                // pilot's own queued entry) is engine back-pressure, not a refusal of
                // the order — counting it toward `MAX_ORDER_REFUSALS` would hold an
                // offline captain's plan after three busy fires. Wait a fire; write
                // nothing. The Buy arm is unchanged (cut IC14 row 124).
                Ok(Err(error)) if is_back_pressure(&market_line(&error)) => Step::Deferred,
                Ok(Err(error)) => refused_after_call(ctx, hull, &pre, &error),
                Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => {
                    Step::Deferred
                }
                Err(SwapRefused::God | SwapRefused::Gone) => Step::Refused,
            }
        }
        Plan::Depart { filing, pre, reach } => {
            let filing = *filing;
            let seq = filing.seq;
            let called = fire.as_principal(ctx, hull, pilot, |ctx| {
                put_voyage(ctx, &filing.voyage);
                ctx.submit_authoritative(
                    filing.burn.clone(),
                    PerceivedRef::SelfActor,
                    Vec::new(),
                    serde_json::json!({ "seq": seq.to_string() }),
                    filing.key.clone(),
                );
            });
            match called {
                Ok(()) => {
                    let mut doc = *pre;
                    if let Some(record) = doc.inflight.as_mut() {
                        record.refs.ticket = Some(TicketId { hull, seq }.key_text());
                    }
                    commit(ctx, Outcome::new(doc), Step::Submitted)
                }
                Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => {
                    // The reach is durable even when the act waits a fire (L9).
                    if orders_doc(ctx.w, hull).as_ref() != Some(&*reach) {
                        let _ = put_orders(ctx, &reach);
                    }
                    Step::Deferred
                }
                Err(SwapRefused::God | SwapRefused::Gone) => Step::Refused,
            }
        }
    }
}

/// The hull's readable plan, cloned.
fn orders_doc(w: &World, hull: ObjId) -> Option<Orders> {
    match orders_state(w, hull) {
        OrdersState::Present(doc) => Some(doc.clone()),
        OrdersState::Absent | OrdersState::Unreadable => None,
    }
}

/// A closed call refused after the swap: the refusal path on the live document.
fn refused_after_call(ctx: &mut Ctx<'_>, hull: ObjId, pre: &Orders, error: &MarketError) -> Step {
    let Some(id) = pre.inflight.as_ref().map(|i| i.order) else {
        return Step::Refused;
    };
    let Some(doc) = orders_doc(ctx.w, hull) else {
        return Step::Refused;
    };
    let line = market_line(error);
    tracing::info!(hull = hull.0, order = id, %line, "tacsim: a queued order's closed call was refused");
    let mut out = Outcome::new(doc);
    refused_call(ctx.w, &mut out, id, &line);
    commit(ctx, out, Step::Worked)
}

/// The re-stage's answer, at the first or the second deadline (L6, reviewer F3).
fn restaged(
    ctx: &mut Ctx<'_>,
    doc: Orders,
    result: Result<Option<ContractId>, MarketError>,
) -> Step {
    match restage_outcome(ctx.w, doc, result) {
        Some((out, step)) => commit(ctx, out, step),
        None => Step::Idle,
    }
}

/// [`restaged`]'s decision, pure over the world (doc 18 cut IC14 row 3: split out so
/// the `DELIVER_LANDED` and `DELIVER_UNCONFIRMED` answers are unit-tested): the
/// document as it should be written and the step, or `None` with nothing in flight.
fn restage_outcome(
    w: &World,
    doc: Orders,
    result: Result<Option<ContractId>, MarketError>,
) -> Option<(Outcome, Step)> {
    let record = doc.inflight.clone()?;
    let id = record.order;
    let first = record.restaged == 0;
    let mut out = Outcome::new(doc);
    let too_late = |e: &MarketError| {
        let line = market_line(e);
        line == mush_market::DRAW_TOO_LATE || line == mush_market::DELIVER_TOO_LATE
    };
    // The second deadline is ORDER_SETTLE_GRACE_SECS past the FIRST re-stage (the
    // constant's meaning, reviewer F3). A first re-stage that ran late — after a
    // downtime, or deferred by back-pressure — re-arms the re-stage clock so the grace
    // still runs from it: `staged_utc + ORDER_RESTAGE_SECS` is then this fire, and the
    // sweeps the evidence waits on get their whole grace (else both deadlines would
    // fall in consecutive fires and a landed leg would read as a second miss).
    let now = w.now();
    let set = |out: &mut Outcome, n: u8| {
        if let Some(r) = out.doc.inflight.as_mut() {
            r.restaged = n;
            if n == 1 {
                r.staged_utc = r.staged_utc.max(now.saturating_sub(ORDER_RESTAGE_SECS));
            }
        }
    };
    let step = match result {
        Err(error) if too_late(&error) => {
            set(&mut out, 2);
            out.log(
                w,
                id,
                &AWAITING_REFUND.replace("{n}", &id.to_string()),
                false,
            );
            Step::Worked
        }
        Ok(Some(_)) if first => {
            set(&mut out, 1);
            Step::Submitted
        }
        Ok(None) => {
            set(&mut out, if first { 1 } else { 2 });
            Step::Worked
        }
        // Doc 18 cut IC14 row 3: the leg already landed (its marker is written and the
        // mirror has not swept it yet) — neither a refusal nor a miss. Exactly what
        // `Ok(None)` does; the step's pull verification reads the landing.
        // `DELIVER_UNCONFIRMED` gets no arm of its own: it falls into the `Err` arms
        // below, so a re-stage that can never read the marker ends in the existing
        // `HELD_UNCONFIRMED` hold and page at the second deadline (lead ruling L32).
        Err(error) if market_line(&error) == mush_market::DELIVER_LANDED => {
            set(&mut out, if first { 1 } else { 2 });
            Step::Worked
        }
        Err(error) if first => {
            tracing::warn!(hull = out.doc.hull.0, order = id, error = %market_line(&error), "tacsim: a queued leg's first re-stage was refused; its verification resolves it");
            set(&mut out, 1);
            Step::Worked
        }
        second => {
            // The second miss (§4.4 "after a second miss, held and paged"): a stopped
            // hold over the step, which heals itself if the leg then lands.
            tracing::error!(
                hull = out.doc.hull.0,
                order = id,
                quote = record.refs.quote,
                contract = record.refs.contract,
                "tacsim: a queued market leg is still unconfirmed at its second deadline; the plan holds over the step"
            );
            set(&mut out, 2);
            hold_with(w, &mut out, id, HELD_UNCONFIRMED);
            if matches!(second, Ok(Some(_))) {
                Step::Submitted
            } else {
                Step::Worked
            }
        }
    };
    Some((out, step))
}

/// Whether the plan's author is buying this hull out (cut IC13): their governing
/// enrolment for the hull is `BuyoutPending` (owner item O6's window).
fn buying_out(w: &World, hull: ObjId, pilot: ObjId, doc: &Orders) -> bool {
    doc.author == pilot
        && w.module_state::<TacsimStore>()
            .and_then(|store| store.enrolment_for_hull(hull))
            .is_some_and(|e| {
                e.pilot == pilot && e.status == crate::port::coop::EnrolmentStatus::BuyoutPending
            })
}

/// Whether the hull's governing enrolment waits on a re-charter (doc 18 cut IC14 row
/// 12): `BuyoutPending` with its buyout refunded and its old lease terminal
/// (`crate::port::buyout::relet_pending`).
fn reletting(w: &World, hull: ObjId) -> bool {
    w.module_state::<TacsimStore>()
        .and_then(|store| store.enrolment_for_hull(hull))
        .is_some_and(|e| crate::port::buyout::relet_pending(w, e))
}

/// **What is due for one hull this fire**, read under a shared borrow: the re-stage,
/// the lease gate, then the cursor order by kind.
#[allow(clippy::too_many_lines)] // the brief's step list, in order
fn decide(ctx: &mut Ctx<'_>, state: &mut PortTickState, hull: ObjId, pilot: ObjId) -> Plan {
    let Some(doc) = orders_doc(ctx.w, hull) else {
        return Plan::Idle;
    };
    let w: &World = ctx.w;
    let now = w.now();
    // 2. Which act is due.
    let restage = doc.inflight.as_ref().and_then(|record| {
        let kind = doc.order(record.order).map(|o| act_kind(&o.what))?;
        let due = match record.restaged {
            0 => now >= record.staged_utc.saturating_add(ORDER_RESTAGE_SECS),
            1 => {
                now >= record
                    .staged_utc
                    .saturating_add(ORDER_RESTAGE_SECS)
                    .saturating_add(ORDER_SETTLE_GRACE_SECS)
            }
            _ => false,
        };
        (due && doc.state != PlanState::Held && matches!(kind, ActKind::Buy | ActKind::Sell))
            .then_some((record.refs.quote, kind == ActKind::Sell))
    });
    let fresh = doc.inflight.is_none()
        && doc.state == PlanState::Running
        && doc
            .cursor
            .and_then(|id| doc.order(id))
            .is_some_and(|order| order.retry_at.is_none_or(|at| now >= at));
    if restage.is_none() && !fresh {
        return Plan::Idle;
    }
    // 3. The title or the live charter, before any act (D-18-23, D-18-32; reviewer F5;
    // cut IC13's `helm_of`).
    let leased = helm_of(w, pilot).is_some_and(|c| c.hull == hull) && doc.author == pilot;
    let name = name_of(w, hull);
    let charter_note = HELD_CHARTER.replace("{name}", &name);
    let buyout_note = HELD_BUYOUT.replace("{name}", &name);
    // Doc 18 cut IC14 row 12: while a refunded buyout's re-charter retries, the
    // buyout's "she sails on your title" is false; the wait says what is true.
    let relet_note = HELD_RELET.replace("{name}", &name);
    let mut doc = doc;
    if !leased {
        if doc.state == PlanState::Paused {
            return Plan::Idle;
        }
        let Some(id) = doc.cursor else {
            return Plan::Idle;
        };
        // Cut IC13 (owner item O6): between `+coop/buyout` and the settlement the author
        // holds neither charter nor title, so the step waits — under the buyout's own
        // note, not the lapsed charter's — and runs on the title once it passes.
        let reason = if buying_out(w, hull, pilot, &doc) {
            if reletting(w, hull) {
                &relet_note
            } else {
                &buyout_note
            }
        } else {
            &charter_note
        };
        let mut out = Outcome::new(doc);
        if wait_with(w, &mut out, id, reason) {
            return Plan::Write(Box::new(out));
        }
        return Plan::Idle;
    }
    if doc.hold.as_ref().is_some_and(|note| {
        note.retry
            && (note.reason == charter_note
                || note.reason == buyout_note
                || note.reason == relet_note)
    }) {
        // "Re-checked silently each fire": the note drops without a line (the lapsed
        // charter's, or the buyout window's once the title has passed).
        doc.hold = None;
        if put_orders(ctx, &doc).is_err() {
            return Plan::Refused;
        }
    }
    let w: &World = ctx.w;
    // 4. The re-stage.
    if let Some((quote, sell)) = restage {
        let Some(quote) = quote else {
            return Plan::Idle;
        };
        return Plan::Restage {
            quote: QuoteId(quote),
            sell,
            doc: Box::new(doc),
        };
    }
    // 5. The new step: the cursor order, by kind.
    let Some(id) = doc.cursor else {
        return Plan::Idle;
    };
    let Some(index) = doc.position(id) else {
        return Plan::Idle;
    };
    let Some(order) = doc.order(id).cloned() else {
        return Plan::Idle;
    };
    if let What::Stop = order.what {
        let mut out = Outcome::new(doc);
        if let Some(entry) = out.doc.order_mut(id) {
            entry.state = OrderState::Done;
            entry.refusals = 0;
        }
        out.doc.state = PlanState::Paused;
        out.doc.hold = None;
        out.log(w, id, &STOPPED.replace("{n}", &id.to_string()), true);
        return Plan::Write(Box::new(out));
    }
    let Berth::Docked { def, map } = berth_of(w, hull) else {
        return Plan::Idle;
    };
    let Some(view) = hull_view(w, hull) else {
        return Plan::Refused;
    };
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Plan::Refused;
    };
    let Some(goods) = store.goods() else {
        return Plan::Refused;
    };
    let ic = w.world_ic_now().at;
    match order.what.clone() {
        What::Stop => Plan::Idle,
        What::Sell {
            good,
            qty,
            min_major,
            on_short,
        } => {
            let label = label_of(goods, &good);
            let mut out = Outcome::new(doc.clone());
            let Some(bid) = bid_at(w, def, map, &good) else {
                skip_or_hold(
                    w,
                    &mut out,
                    id,
                    on_short,
                    &NO_BID.replace("{label}", &label),
                );
                return Plan::Write(Box::new(out));
            };
            let Some(vault) = crate::port::ship::hold_vault(w, &view) else {
                return Plan::Refused;
            };
            let Ok(aboard) = read_pool(ctx, mush_inventory::record::EndpointRef(vault), &good)
            else {
                return Plan::Idle;
            };
            if aboard == 0 {
                skip_or_hold(
                    w,
                    &mut out,
                    id,
                    on_short,
                    &NOTHING_ABOARD.replace("{label}", &label),
                );
                return Plan::Write(Box::new(out));
            }
            let want = match qty {
                Qty::Units(n) => n,
                Qty::All | Qty::Fill => aboard,
            };
            let floor = min_major
                .and_then(|m| crate::port::apply::minor_of(m, crate::port::apply::credit_scale(w)));
            let under = floor.is_some_and(|floor| bid.price.minor() < floor);
            let (q, bounds) = clamp_sell(want, under, aboard, bid.remaining, bid.house_units);
            let unit = unit_of(goods, &good);
            let house_line = HOUSE_CANNOT_PAY_SKIP
                .replace("{house}", &bid.house)
                .replace("{label}", &label);
            if q == 0 {
                let reason = sale_none_reason(w, &bid, &label, &unit, floor, want, &bounds);
                skip_or_hold(w, &mut out, id, on_short, &reason);
                return Plan::Write(Box::new(out));
            }
            if q < want && on_short != OnShort::Partial {
                skip_or_hold(
                    w,
                    &mut out,
                    id,
                    on_short,
                    &short_clause(want, &unit, &bounds),
                );
                return Plan::Write(Box::new(out));
            }
            let attempt = order.attempt;
            let mut rec = record(id, attempt, String::new(), now, ic);
            rec.want = want;
            rec.qty = q;
            rec.bounds = bounds.iter().map(|b| SmolStr::new(b.word())).collect();
            let Some(pre) = staged(&doc, id, rec) else {
                return Plan::Refused;
            };
            let short = (q < want).then(|| {
                SALE_SHORT
                    .replace("{want}", &want.to_string())
                    .replace("{unit}", &unit)
                    .replace("{bounds}", &join_bounds(&bounds))
            });
            Plan::Sell {
                listing: bid.listing,
                qty: q,
                pre: Box::new(pre),
                short,
                on_short,
                house_line,
            }
        }
        What::Buy {
            good,
            qty,
            max_major,
            on_short,
        } => {
            let label = label_of(goods, &good);
            let unit = unit_of(goods, &good);
            let mut out = Outcome::new(doc.clone());
            let Ok(readings) = crate::port::ship::readings_of(ctx, &view) else {
                return Plan::Idle;
            };
            // The planning core cannot answer (no planner inputs, or compartments it
            // cannot tell apart): the step holds with HELD_FILL's planner sentence and
            // pages once — never guessed, and never re-paged every fire (the IC11
            // brief's item 4). The port named is the next departure's, else here.
            let planning_port = doc
                .orders
                .iter()
                .skip(index)
                .find_map(|o| match (&o.what, o.state) {
                    (
                        What::Depart { to, .. },
                        OrderState::Pending | OrderState::Inflight | OrderState::Held,
                    ) => Some(crate::port::voyage::venue_name(w, to)),
                    _ => None,
                })
                .unwrap_or_else(|| port_name(def));
            let planning_hold = |out: &mut Outcome, error: &str| {
                tracing::error!(hull = hull.0, order = id, %error, "tacsim: a queued buy's planning core cannot answer; the order holds");
                hold_with(
                    w,
                    out,
                    id,
                    &HELD_FILL_PLAN.replace("{port}", &planning_port),
                );
            };
            let planner = match planner(w, &view).and_then(|planner| planner.carrying(ctx, &view)) {
                Ok(planner) => planner,
                Err(error) => {
                    planning_hold(&mut out, &error);
                    return Plan::Write(Box::new(out));
                }
            };
            let occupants = view
                .airlock()
                .map_or(0, |airlock| {
                    u32::try_from(crate::port::voyage::souls_aboard(w, airlock).len())
                        .unwrap_or(u32::MAX)
                })
                .max(1);
            let consumable = goods.good(&good).and_then(|row| row.consumable);
            let next = next_depart_targets(w, &planner, def, &doc, index, &readings, occupants);
            let (want, target, cargo_t) = match (qty, consumable) {
                (Qty::Fill, Some(kind)) => match next {
                    Ok(Some(next)) => {
                        let target = match kind {
                            Consumable::Propellant => next.targets.tank,
                            Consumable::Provisions => next.targets.locker,
                        };
                        let aboard = aboard_of(goods, &readings, kind);
                        let port = crate::port::voyage::venue_name(w, &next.to);
                        if target <= aboard {
                            if let Some(entry) = out.doc.order_mut(id) {
                                entry.state = OrderState::Done;
                                entry.refusals = 0;
                            }
                            out.doc.clear_hold_naming(id);
                            out.log(
                                w,
                                id,
                                &FILL_ENOUGH
                                    .replace("{label}", &label)
                                    .replace("{port}", &port),
                                true,
                            );
                            return Plan::Write(Box::new(out));
                        }
                        (
                            target.saturating_sub(aboard),
                            Some((target, port)),
                            next.targets.cargo_t,
                        )
                    }
                    Ok(None) => {
                        hold_with(w, &mut out, id, HELD_NO_DEPART);
                        return Plan::Write(Box::new(out));
                    }
                    Err(TargetError::NoRoute(line)) => {
                        hold_with(w, &mut out, id, &line);
                        return Plan::Write(Box::new(out));
                    }
                    Err(TargetError::Fill(error, port)) => {
                        if error == FillError::NoFixedPoint {
                            tracing::error!(
                                hull = hull.0,
                                order = id,
                                "tacsim: fill found no fixed point; the order holds"
                            );
                        }
                        hold_with(w, &mut out, id, &fill_reason(goods, &error, &port));
                        return Plan::Write(Box::new(out));
                    }
                },
                (Qty::Units(n), Some(kind)) => match next {
                    Ok(Some(next)) => {
                        let target = match kind {
                            Consumable::Propellant => next.targets.tank,
                            Consumable::Provisions => next.targets.locker,
                        };
                        let port = crate::port::voyage::venue_name(w, &next.to);
                        (n, Some((target, port)), next.targets.cargo_t)
                    }
                    Ok(None) | Err(_) => (n, None, 0),
                },
                (Qty::Units(n), None) => (n, None, 0),
                (Qty::Fill | Qty::All, _) => (0, None, 0),
            };
            let ask = ask_at(w, def, map, &good);
            let stock = ask.as_ref().map_or(0, |a| a.quantity);
            let price = ask.as_ref().map_or(Amount::from_minor(0), |a| a.price);
            let ceiling = max_major
                .and_then(|m| crate::port::apply::minor_of(m, crate::port::apply::credit_scale(w)));
            let over = ceiling.is_some_and(|c| price.minor() > c);
            let room = match room_for(goods, &readings, &good) {
                Ok(room) => room,
                Err(error) => {
                    planning_hold(&mut out, &error.to_string());
                    return Plan::Write(Box::new(out));
                }
            };
            let reserve = reserve_at(
                w, &planner, def, map, &doc, index, &readings, occupants, consumable,
            )
            .unwrap_or(Amount::from_minor(0));
            let available = crate::port::credit::own_account(w, pilot)
                .and_then(mush_economy::Account::available)
                .unwrap_or(Amount::from_minor(0));
            // No live ask: the stock bound alone bites (funds cannot bind a price
            // nobody posts, and a 0 price would read as "funds", every bound that bit
            // and no other).
            let affordable = ask
                .as_ref()
                .map_or(u64::MAX, |a| funds_units(available, reserve, a.price));
            let (q, bounds) = clamp_buy(want, over, stock, room, affordable);
            // A consumable short of the next departure's need holds, buying nothing —
            // doc 18 cut IC14 row 26 (lead ruling L22): only when the later consumable
            // orders before that depart would still leave it short.
            if let (Some(kind), Some((target, port))) = (consumable, &target) {
                let aboard = aboard_of(goods, &readings, kind);
                let short = match later_cover(&doc, index, &good) {
                    Cover::Fill => false,
                    Cover::Units(later) => aboard.saturating_add(q).saturating_add(later) < *target,
                };
                if short {
                    let reason = HELD_SHORT_FOR_DEPART
                        .replace("{bought}", &format!("{q} {unit}"))
                        .replace("{port}", port)
                        .replace("{need}", &target.to_string())
                        .replace("{unit}", &unit);
                    hold_with(w, &mut out, id, &reason);
                    return Plan::Write(Box::new(out));
                }
            }
            if q == 0 {
                if on_short == OnShort::Partial {
                    if let Some(entry) = out.doc.order_mut(id) {
                        entry.state = OrderState::Done;
                        entry.refusals = 0;
                    }
                    out.doc.clear_hold_naming(id);
                    let line = BOUGHT_NONE
                        .replace("{unit}", &unit)
                        .replace("{label}", &label)
                        .replace("{want}", &want.to_string())
                        .replace("{bounds}", &join_bounds(&bounds));
                    out.log(w, id, &line, true);
                } else {
                    skip_or_hold(
                        w,
                        &mut out,
                        id,
                        on_short,
                        &short_clause(want, &unit, &bounds),
                    );
                }
                return Plan::Write(Box::new(out));
            }
            if q < want && on_short != OnShort::Partial {
                skip_or_hold(
                    w,
                    &mut out,
                    id,
                    on_short,
                    &short_clause(want, &unit, &bounds),
                );
                return Plan::Write(Box::new(out));
            }
            let Some(ask) = ask else {
                return Plan::Idle;
            };
            let total = money(
                w,
                Amount::from_minor(
                    price
                        .minor()
                        .saturating_mul(i64::try_from(q).unwrap_or(i64::MAX)),
                ),
            );
            let each = money(w, price);
            let line = match (qty, consumable, &target) {
                (Qty::Fill, Some(Consumable::Propellant), Some(_)) => {
                    let after =
                        aboard_of(goods, &readings, Consumable::Propellant).saturating_add(q);
                    let limit = limit_of(goods, &readings, &good);
                    BOUGHT_FILL_TANK
                        .replace("{qty}", &q.to_string())
                        .replace("{label}", &label)
                        .replace("{price}", &each)
                        .replace("{total}", &total)
                        .replace("{after}", &after.to_string())
                        .replace("{limit}", &limit.to_string())
                        .replace("{cargo}", &cargo_t.to_string())
                }
                (Qty::Fill, Some(Consumable::Provisions), Some(_)) => BOUGHT_FILL_LOCKER
                    .replace("{qty}", &q.to_string())
                    .replace("{total}", &total),
                _ if q < want => BOUGHT_SHORT
                    .replace("{qty}", &q.to_string())
                    .replace("{unit}", &unit)
                    .replace("{label}", &label)
                    .replace("{price}", &each)
                    .replace("{total}", &total)
                    .replace("{want}", &want.to_string())
                    .replace("{bounds}", &join_bounds(&bounds)),
                _ => BOUGHT
                    .replace("{qty}", &q.to_string())
                    .replace("{unit}", &unit)
                    .replace("{label}", &label)
                    .replace("{price}", &each)
                    .replace("{total}", &total),
            };
            let mut rec = record(id, order.attempt, String::new(), now, ic);
            rec.want = want;
            rec.qty = q;
            rec.bounds = bounds.iter().map(|b| SmolStr::new(b.word())).collect();
            rec.line = Some(crate::port::orders::clamp_line(&line));
            let Some(pre) = staged(&doc, id, rec) else {
                return Plan::Refused;
            };
            Plan::Buy {
                listing: ask.listing,
                qty: q,
                pre: Box::new(pre),
            }
        }
        What::Depart {
            to,
            not_before,
            not_after_s,
            on_fail,
            throttle_permille,
            via,
            ..
        } => {
            let port = crate::port::voyage::venue_name(w, &to);
            let here = port_name(def);
            let options = PassageOptions::for_order(throttle_permille, via);
            // The reach (L9; reviewer F8): the first fire the depart is runnable here.
            let mut reach_doc = doc.clone();
            let mut dirty = false;
            let reached = match order.reached.as_ref().filter(|r| r.venue == def.venue) {
                Some(reached) => reached.utc,
                None => {
                    if let Some(entry) = reach_doc.order_mut(id) {
                        entry.reached = Some(crate::port::orders::Reached {
                            utc: now,
                            venue: def.venue.clone(),
                        });
                    }
                    dirty = true;
                    now
                }
            };
            let due = resolve_due(&not_before, not_after_s, reached);
            let mut out = Outcome::new(reach_doc.clone());
            if now < due {
                if !order.announced {
                    if let Some(entry) = out.doc.order_mut(id) {
                        entry.announced = true;
                    }
                    let line = DEPART_SET
                        .replace("{port}", &port)
                        .replace("{hhmm}", &crate::port::voyage::utc_hhmm(due));
                    out.log(w, id, &line, true);
                    return Plan::Write(Box::new(out));
                }
                return if dirty {
                    Plan::Write(Box::new(out))
                } else {
                    Plan::Idle
                };
            }
            let daily = matches!(not_before, crate::port::when::NotBefore::UtcDaily(_));
            if let Some(window) = not_after_s
                && !daily
                && now > due.saturating_add(i64::from(window))
            {
                let reason = HELD_WINDOW
                    .replace("{port}", &port)
                    .replace("{hhmm}", &crate::port::voyage::utc_hhmm(due));
                skip_or_hold(w, &mut out, id, on_fail, &reason);
                return Plan::Write(Box::new(out));
            }
            let attempt = order.attempt;
            let filing = crate::port::voyage::prepare_departure(
                ctx,
                pilot,
                hull,
                to.as_str(),
                &options,
                crate::port::voyage::Queued::Order { order: id, attempt },
            );
            use crate::port::voyage::DepartRefusalKind as K;
            match filing {
                Err(refusal) => {
                    let reason = match refusal.kind {
                        K::NotAboard => HELD_ASHORE
                            .replace("{name}", &name)
                            .replace("{port}", &here),
                        K::GoodsComing
                        | K::ListingHolds
                        | K::GoodsLeaving
                        | K::Overloaded
                        | K::TankShort
                        | K::StoresShort
                        | K::PortClosed
                        | K::Landing
                        | K::SiteClosed => refusal.line.clone(),
                        K::PlanDark | K::VoyageDark | K::Unwired => {
                            if state.depart_dark_warned.insert(hull.0) {
                                tracing::error!(hull = hull.0, order = id, line = %refusal.line, "tacsim: a queued departure meets a dark plan; it waits");
                            }
                            refusal.line.clone()
                        }
                        K::NoRoute => {
                            skip_or_hold(w, &mut out, id, on_fail, &refusal.line);
                            return Plan::Write(Box::new(out));
                        }
                        K::Cleared | K::UnderWay | K::NotDocked => {
                            return if dirty {
                                Plan::Write(Box::new(out))
                            } else {
                                Plan::Idle
                            };
                        }
                    };
                    if wait_with(w, &mut out, id, &reason) || dirty {
                        return Plan::Write(Box::new(out));
                    }
                    Plan::Idle
                }
                Ok(filing) => {
                    let key = filing.key.as_str().to_owned();
                    let rec = record(id, attempt, key, now, ic);
                    let Some(pre) = staged(&reach_doc, id, rec) else {
                        return Plan::Refused;
                    };
                    Plan::Depart {
                        filing: Box::new(filing),
                        pre: Box::new(pre),
                        reach: Box::new(reach_doc),
                    }
                }
            }
        }
    }
}

/// **Why a queued sale takes nothing** (doc 18 cut IC11's skip reasons, factored by
/// cut IC12 so each is pinned without a 4,000 t intake to exhaust): the bid under the
/// order's floor (`UNDER_MIN`), else an exhausted bid (`INTAKE_TAKEN`), else a house
/// that cannot pay (`HOUSE_CANNOT_PAY_SKIP`), else the short clause naming every bound
/// that bit. `floor` is the order's `min=` in minor units, `None` without one.
pub(crate) fn sale_none_reason(
    w: &World,
    bid: &Bid,
    label: &str,
    unit: &str,
    floor: Option<i64>,
    want: u64,
    bounds: &[Bound],
) -> String {
    if floor.is_some_and(|floor| bid.price.minor() < floor) {
        UNDER_MIN
            .replace("{house}", &bid.house)
            .replace("{price}", &money(w, bid.price))
            .replace("{unit}", unit)
            .replace("{min}", &money(w, Amount::from_minor(floor.unwrap_or(0))))
    } else if bid.remaining == 0 {
        INTAKE_TAKEN
            .replace("{house}", &bid.house)
            .replace("{label}", label)
    } else if bid.house_units == 0 {
        HOUSE_CANNOT_PAY_SKIP
            .replace("{house}", &bid.house)
            .replace("{label}", label)
    } else {
        short_clause(want, unit, bounds)
    }
}

/// The short clause (`short of {want} {unit}: {bounds}.`).
fn short_clause(want: u64, unit: &str, bounds: &[Bound]) -> String {
    SHORT_CLAUSE
        .replace("{want}", &want.to_string())
        .replace("{unit}", unit)
        .replace("{bounds}", &join_bounds(bounds))
}

/// A compartment's capacity in units of `good`, from empty.
fn limit_of(goods: &GoodsTable, readings: &[Reading], good: &CanonicalId) -> u64 {
    let comps: Vec<Compartment> = readings.iter().map(|(_, comp, _)| comp.clone()).collect();
    capacity::compartment_for(goods, &comps, good)
        .and_then(|comp| capacity::max_more(goods, &comp, &BTreeMap::new(), good).ok())
        .map_or(0, |(units, _)| units)
}

// ---- the executor texts (the planner's wording, owner item O1) ----------------------

/// A buy: `{qty}`, `{unit}`, `{label}`, `{price}`, `{total}`.
pub const BOUGHT: &str = "Bought {qty} {unit} {label} at {price}/{unit} ({total}).";
/// A short buy: as [`BOUGHT`] plus `{want}`, `{bounds}`.
pub const BOUGHT_SHORT: &str =
    "Bought {qty} {unit} {label} at {price}/{unit} ({total}); short of {want} {unit}: {bounds}.";
/// A buy that took nothing (partial): `{unit}`, `{label}`, `{want}`, `{bounds}`.
pub const BOUGHT_NONE: &str = "Bought 0 {unit} {label}: short of {want} {unit}: {bounds}.";
/// `fill` for the tank (doc 18 §2 step 14): `{qty}`, `{label}`, `{price}`, `{total}`,
/// `{after}`, `{limit}`, `{cargo}`.
pub const BOUGHT_FILL_TANK: &str = "Bought {qty} t {label} at {price}/t ({total}): tank {after}/{limit} t, the burn for the {cargo} t your orders can load.";
/// `fill` for the locker (doc 18 §2 step 14): `{qty}`, `{total}`.
pub const BOUGHT_FILL_LOCKER: &str = "Bought {qty} crew-days ({total}).";
/// `fill` with enough aboard: `{label}`, `{port}`.
pub const FILL_ENOUGH: &str = "No {label} bought: what is aboard covers the passage to {port}.";
/// A partial sale's tail: `{want}`, `{unit}`, `{bounds}`.
pub const SALE_SHORT: &str = " Short of {want} {unit}: {bounds}.";
/// A depart's hour, logged once (doc 18 §2 step 14): `{port}`, `{hhmm}`.
pub const DEPART_SET: &str = "Departure for {port} set for {hhmm} UTC.";
/// A stop reached (owner item O5): `{n}`.
pub const STOPPED: &str = "Stopped at order {n}, as filed; +ship/order resume runs the rest.";
/// An order passed over: `{n}`, `{reason}`.
pub const SKIP_LINE: &str = "Order {n} passed over: {reason}";
/// A stopped hold: `{n}`, `{reason}`.
pub const HELD_LINE: &str = "Order {n} holds: {reason}";
/// A waiting note: `{n}`, `{reason}`.
pub const WAITING_LINE: &str = "Order {n} waits: {reason}";
/// No bid here: `{label}`.
pub const NO_BID: &str = "no house here buys {label}.";
/// Nothing to sell: `{label}`.
pub const NOTHING_ABOARD: &str = "no {label} is aboard.";
/// An exhausted bid: `{house}`, `{label}`.
pub const INTAKE_TAKEN: &str = "{house} is not buying {label}; its intake is taken.";
/// A house that cannot pay: `{house}`, `{label}`.
pub const HOUSE_CANNOT_PAY_SKIP: &str = "{house} cannot pay for {label} right now.";
/// A bid under the order's floor: `{house}`, `{price}`, `{unit}`, `{min}`.
pub const UNDER_MIN: &str = "{house} bids {price}/{unit}, under your {min}.";
/// The short clause: `{want}`, `{unit}`, `{bounds}`.
pub const SHORT_CLAUSE: &str = "short of {want} {unit}: {bounds}.";
/// A consumable buy that leaves the passage short: `{bought}`, `{port}`, `{need}`,
/// `{unit}`.
pub const HELD_SHORT_FOR_DEPART: &str =
    "{bought} would leave the passage to {port} short; it needs {need} {unit}.";
/// `fill` with no depart after it.
pub const HELD_NO_DEPART: &str = "fill sizes the stores for a departure, and none follows it.";
/// `fill` the compartment cannot carry: `{compartment}`, `{need}`, `{unit}`, `{port}`.
pub const HELD_FILL: &str =
    "the {compartment} cannot carry the {need} {unit} the passage to {port} needs.";
/// `fill` the planner cannot answer: `{port}`.
pub const HELD_FILL_PLAN: &str = "the passage to {port} cannot be planned right now.";
/// The pilot ashore (Q-18-17): `{name}`, `{port}`.
pub const HELD_ASHORE: &str = "the {name} waits at {port}: her pilot is ashore, and she sails only with her pilot aboard; nothing was burned.";
/// A fixed hour passed: `{port}`, `{hhmm}`.
pub const HELD_WINDOW: &str =
    "the hour for {port} passed at {hhmm} UTC; +ship/order resume sails now.";
/// The lease lapsed: `{name}`.
pub const HELD_CHARTER: &str = "your charter of the {name} is no longer in force.";
/// The buyout window (cut IC13, owner item O6): `{name}`.
pub const HELD_BUYOUT: &str =
    "your buyout of the {name} is being entered; she sails on your title once it passes.";
/// A refunded buyout whose re-charter is retrying (doc 18 cut IC14 row 12; lead ruling
/// L11; owner voice, cut IC14 O1): the wait's note in place of [`HELD_BUYOUT`], whose
/// "she sails on your title" is false then. `{name}`.
pub const HELD_RELET: &str = "the {name}'s charter is being renewed after your buyout came back; she sails again once it is.";
/// The second unverified miss.
pub const HELD_UNCONFIRMED: &str = "the counter has not confirmed it; staff have been told.";
/// A staff stand-down: `{port}`.
pub const HELD_STOOD_DOWN: &str =
    "the port stood the clearance for {port} down; +ship/order resume files it again.";
/// A proven drop (log only): `{n}`.
pub const NOT_ADMITTED: &str =
    "Order {n} was not taken up at the counter; it is tried again once the counter has it back.";
/// A late hand-over awaiting its refund (log only): `{n}`.
pub const AWAITING_REFUND: &str = "Order {n} awaits the counter: an unlanded hand-over is refunded within the hour, and the order is tried again.";
/// A refused closed call (log only): `{n}`, `{line}`.
pub const REFUSED_RETRY: &str =
    "Order {n} was refused at the counter ({line}); it is tried again in a minute.";
/// The pilot told of a listed passenger left ashore: `{who}`, `{port}`.
pub const LEFT_BEHIND_PILOT: &str = "{who} was ashore and is left at {port}.";
/// The passenger left ashore: `{name}`, `{dest}`, `{port}`.
pub const LEFT_BEHIND_PASSENGER: &str =
    "The {name} sailed for {dest} without you; you are left at {port}.";
/// A hold, narrated to the hull's rooms: `{n}`.
pub const HOLD_NARRATION: &str = "The Bridge chimes: the sailing plan holds at order {n}.";
/// A skip, narrated to the hull's rooms: `{n}`.
pub const SKIP_NARRATION: &str = "The Bridge chimes: the sailing plan passes over order {n}.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::StowageDef;

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).expect("id")
    }

    fn goods() -> GoodsTable {
        crate::definitions::goods::goods_table().expect("the compiled-in goods table")
    }

    /// The Dray's compartments from its shipped pack, as readings of `contents`.
    fn readings(contents: &[(&str, u64)]) -> Vec<Reading> {
        let pack = crate::pack::load_pack(crate::definitions::content::DRAY_JSON)
            .expect("the Dray pack loads");
        let logistics = match pack.entry(&cid(crate::definitions::content::DRAY)) {
            Some(crate::pack::PackEntry::Unit(spec)) => {
                spec.logistics.clone().expect("the Dray's logistics")
            }
            _ => panic!("the Dray unit"),
        };
        let goods = goods();
        logistics
            .stowage
            .iter()
            .map(|(id, def): (&CanonicalId, &StowageDef)| {
                let comp = Compartment::from_stowage(id, def);
                let held: BTreeMap<CanonicalId, u64> = contents
                    .iter()
                    .filter(|(good, _)| {
                        goods
                            .good(&cid(good))
                            .is_some_and(|row| comp.accepts.contains(&row.stowage))
                    })
                    .map(|(good, qty)| (cid(good), *qty))
                    .collect();
                (id.clone(), comp, held)
            })
            .collect()
    }

    fn cr(major: i64) -> Amount {
        Amount::from_minor(major * 100)
    }

    #[test]
    fn ic11_the_clamp_names_every_bound_that_bit() {
        let goods = goods();
        let alloys = cid("structural_alloys");
        let fittings = cid("industrial_parts");
        // (a) The cash-bound return: 49 t of fittings aboard, alloys want 55, the room
        // is volume-bound, the funds buy 20.
        let hold = readings(&[("industrial_parts", 49)]);
        let room = room_for(&goods, &hold, &alloys).unwrap();
        assert_eq!(room.1, vec![Bound::HoldVolume]);
        let funds = funds_units(cr(8_360), cr(0), cr(400));
        assert_eq!(funds, 20);
        assert_eq!(
            clamp_buy(55, false, 7_000, room.clone(), funds),
            (20, vec![Bound::Funds])
        );
        // (b) The same with full funds: the volume binds.
        assert_eq!(
            clamp_buy(55, false, 7_000, room.clone(), 1_000),
            (room.0, vec![Bound::HoldVolume])
        );
        assert_eq!(room.0, 24, "7.3 m³ free over 300 L a tonne");
        // (c) An empty hold, ice want 100: the volume binds at 85.
        let empty = readings(&[]);
        let ice = room_for(&goods, &empty, &cid("volatiles")).unwrap();
        assert_eq!(
            clamp_buy(100, false, 9_600, ice, 1_000),
            (85, vec![Bound::HoldVolume])
        );
        // (d) The both-limits vertex, derived from max_more (never the literals): no
        // room by either measure.
        let cargo = empty
            .iter()
            .find(|(id, _, _)| id.as_str() == "cargo")
            .map(|(_, comp, _)| comp.clone())
            .unwrap();
        let (metals, _) =
            capacity::max_more(&goods, &cargo, &BTreeMap::new(), &cid("metals")).unwrap();
        let mut vertex: Option<(u64, u64)> = None;
        for m in 0..=metals {
            let with = BTreeMap::from([(cid("metals"), m)]);
            let (v, _) = capacity::max_more(&goods, &cargo, &with, &cid("volatiles")).unwrap();
            let both = BTreeMap::from([(cid("metals"), m), (cid("volatiles"), v)]);
            let load = capacity::load(&goods, &cargo, &both).unwrap();
            if load == cargo.limits {
                vertex = Some((m, v));
                break;
            }
        }
        let (m, v) = vertex.expect("a both-limits load exists");
        let full = readings(&[("metals", m), ("volatiles", v)]);
        let room = room_for(&goods, &full, &fittings).unwrap();
        assert_eq!(
            clamp_buy(10, false, 5_400, room, 1_000),
            (0, vec![Bound::HoldMass, Bound::HoldVolume])
        );
        // (e) A 5 t ask, want 10: the stock binds.
        assert_eq!(
            clamp_buy(10, false, 5, (85, vec![Bound::HoldVolume]), 1_000),
            (5, vec![Bound::Stock])
        );
        // (f) A price over max.
        assert_eq!(
            clamp_buy(10, true, 5_000, (85, vec![Bound::HoldVolume]), 1_000),
            (0, vec![Bound::Price])
        );
        assert_eq!(
            join_bounds(&[Bound::HoldMass, Bound::HoldVolume]),
            "hold mass and hold volume"
        );
        assert_eq!(
            join_bounds(&[Bound::Stock, Bound::HoldMass, Bound::Funds]),
            "stock, hold mass and funds"
        );
        assert_eq!(join_bounds(&[Bound::Funds]), "funds");
        // A sale's clamp: the house's funds and the intake.
        assert_eq!(clamp_sell(80, false, 80, 9_600, 1_000), (80, Vec::new()));
        assert_eq!(
            clamp_sell(80, false, 80, 30, 1_000),
            (30, vec![Bound::Intake])
        );
        assert_eq!(
            clamp_sell(80, false, 80, 30, 30),
            (30, vec![Bound::Intake, Bound::HouseFunds])
        );
        assert_eq!(clamp_sell(80, true, 80, 30, 30), (0, vec![Bound::Price]));
        // The tank and the locker bind as themselves.
        let tank = room_for(&goods, &readings(&[("propellant", 79)]), &cid("propellant")).unwrap();
        assert_eq!(tank, (1, vec![Bound::Tank]));
        let locker = room_for(
            &goods,
            &readings(&[("provisions", 120)]),
            &cid("provisions"),
        )
        .unwrap();
        assert_eq!(locker, (0, vec![Bound::Locker]));
    }

    /// A Coast-law fake planner: `ceil((dry + payload) × permille / 1e6)` tonnes and
    /// 2 crew-days per started 50 t of payload.
    fn fake(payload: u64) -> Result<PassagePlan, PlanError> {
        let burn = (240_000 + payload) * 105_u64;
        Ok(plan_with(
            burn.div_ceil(1_000_000),
            u32::try_from(payload / 50_000 + 1).unwrap() * 2,
        ))
    }

    fn plan_with(burn_t: u64, crew_days: u32) -> PassagePlan {
        PassagePlan {
            route: cid("r"),
            law: cid("coast"),
            depart_ic: 0,
            sky_rev: mush_core::DefinitionDigest::from_bytes([0; 32]),
            legs: Vec::new(),
            total_s: 0,
            burn_t,
            crew_days,
            throttle_permille: 1_000,
            landed_crew_days: 0,
        }
    }

    #[test]
    fn ic11_fill_is_the_least_fixed_point_at_or_above_aboard() {
        let goods = goods();
        let empty = readings(&[]);
        let cargo = BTreeMap::from([(cid("metals"), 73)]);
        // Converges from what is aboard, in at most six passes.
        let t = fill_targets(&goods, &empty, &cargo, 0, 1, 0, 1, 100, fake).unwrap();
        assert!(t.iterations <= FILL_ITERATIONS);
        let again = fake(t.payload_kg).unwrap();
        assert_eq!(
            (t.tank, t.locker),
            (again.burn_t, u64::from(again.crew_days))
        );
        assert_eq!(t.cargo_t, 73);
        // A full tank stays: at or above what is aboard.
        let full = fill_targets(
            &goods,
            &readings(&[("propellant", 70)]),
            &cargo,
            0,
            70,
            0,
            1,
            100,
            fake,
        )
        .unwrap();
        assert_eq!(full.tank, 70);
        // The LEAST fixed point: a step planner with two fixed points (10 t below
        // 150 t of payload, 50 t at or above it) settles on the lower one from below.
        // Doc 18 cut IC14 row 41: the step sits inside the reachable range, below the
        // Dray's 181 t payload ceiling (it was at 400 t, a payload no hull carries).
        let step = |payload: u64| Ok(plan_with(if payload >= 150_000 { 50 } else { 10 }, 0));
        let least = fill_targets(
            &goods,
            &empty,
            &BTreeMap::from([(cid("metals"), 100)]),
            0,
            0,
            0,
            1,
            100,
            step,
        )
        .unwrap();
        assert_eq!(
            least.tank, 10,
            "100 t of cargo and 10 t of fuel: under the step"
        );
        // From 50 t aboard the same 100 t of cargo weighs 150 t: the step's upper point.
        let greater = fill_targets(
            &goods,
            &readings(&[("propellant", 50)]),
            &BTreeMap::from([(cid("metals"), 100)]),
            0,
            50,
            0,
            1,
            100,
            step,
        )
        .unwrap();
        assert_eq!(
            greater.tank, 50,
            "only from 50 aboard is the greater point reached"
        );

        // Doc 18 §4.4's case on the shipped lane through the real planner: the 73 t
        // projection (item (a)'s orders through project_cargo) with 1 t and 0
        // crew-days aboard.
        let return_orders = [
            What::Buy {
                good: cid("industrial_parts"),
                qty: Qty::Units(49),
                max_major: Some(400),
                on_short: crate::port::orders::OnShort::Partial,
            },
            What::Buy {
                good: cid("structural_alloys"),
                qty: Qty::Units(55),
                max_major: Some(400),
                on_short: crate::port::orders::OnShort::Partial,
            },
        ];
        let refs: Vec<&What> = return_orders.iter().collect();
        let aboard = readings(&[("propellant", 1)]);
        let (projected, extra) = project_cargo(&goods, &aboard, &refs).unwrap();
        assert_eq!(projected.values().sum::<u64>(), 73);
        let pack = crate::pack::load_pack(crate::definitions::content::LANE_JSON).expect("lane");
        let sky = Sky::from_pack(&pack, &crate::EphemerisConfig::default());
        let catalog = crate::definitions::Catalog::build_fielded(&[], crate::NS).expect("catalog");
        let (_, spec) = hull_spec_of(&catalog, &goods, &cid(crate::definitions::content::DRAY))
            .expect("the Dray's spec");
        let routes = vec![cid(crate::definitions::content::LANE_ROUTE_BACK)];
        let real = fill_targets(
            &goods,
            &aboard,
            &projected,
            extra,
            1,
            0,
            1,
            spec.allowance_kg,
            |payload| {
                choose_route(
                    &sky,
                    &pack,
                    &routes,
                    &spec.drive,
                    spec.dry_kg,
                    payload,
                    1,
                    0,
                    crate::port::passage::RoutePolicy::default(),
                    None,
                )
            },
        )
        .unwrap();
        assert_eq!(
            (real.tank, real.locker),
            (50, 19),
            "the return's fill under the flip law (doc 19 cut A.2): 50 t and 19 crew-days (doc 18 §4.4's coast figures were 37 t and 19, then 20 on lane v4)"
        );
        assert_eq!(real.cargo_t, 73);
    }

    #[test]
    fn ic11_fill_refuses_past_six_iterations() {
        let goods = goods();
        let mut n = 0_u64;
        let never = |_: u64| {
            n += 1;
            Ok(plan_with(n, 0))
        };
        assert_eq!(
            fill_targets(
                &goods,
                &readings(&[]),
                &BTreeMap::new(),
                0,
                0,
                0,
                1,
                100,
                never
            ),
            Err(FillError::NoFixedPoint)
        );
        // A tank too small for the target.
        let huge = |_: u64| Ok(plan_with(90, 0));
        assert_eq!(
            fill_targets(
                &goods,
                &readings(&[]),
                &BTreeMap::new(),
                0,
                0,
                0,
                1,
                100,
                huge
            ),
            Err(FillError::TankTooSmall { need: 90, room: 80 })
        );
    }

    #[test]
    fn ic11_the_projection_takes_sells_then_clamped_buys_in_order() {
        let goods = goods();
        let aboard = readings(&[("metals", 20), ("volatiles", 80), ("propellant", 1)]);
        let orders = [
            What::Sell {
                good: cid("metals"),
                qty: Qty::All,
                min_major: None,
                on_short: crate::port::orders::OnShort::Partial,
            },
            What::Sell {
                good: cid("volatiles"),
                qty: Qty::Units(30),
                min_major: None,
                on_short: crate::port::orders::OnShort::Partial,
            },
            What::Buy {
                good: cid("industrial_parts"),
                qty: Qty::Units(49),
                max_major: None,
                on_short: crate::port::orders::OnShort::Partial,
            },
            What::Buy {
                good: cid("structural_alloys"),
                qty: Qty::Units(55),
                max_major: None,
                on_short: crate::port::orders::OnShort::Partial,
            },
            What::Buy {
                good: cid("provisions"),
                qty: Qty::Units(10),
                max_major: None,
                on_short: crate::port::orders::OnShort::Partial,
            },
        ];
        let refs: Vec<&What> = orders.iter().collect();
        let (cargo, extra) = project_cargo(&goods, &aboard, &refs).unwrap();
        assert_eq!(cargo.get(&cid("metals")), None);
        assert_eq!(cargo.get(&cid("volatiles")), Some(&50));
        // 50 t of ice is 70 m³; 49 t of fittings would be 112.7 m³: they clamp to what
        // fits, 50 m³ / 2.3 = 21 t, and the alloys then fit in what is left.
        let (fit, _) = capacity::max_more(
            &goods,
            &aboard
                .iter()
                .find(|(id, _, _)| id.as_str() == "cargo")
                .unwrap()
                .1,
            &BTreeMap::from([(cid("volatiles"), 50)]),
            &cid("industrial_parts"),
        )
        .unwrap();
        assert_eq!(cargo.get(&cid("industrial_parts")), Some(&fit));
        let cargo_comp = aboard
            .iter()
            .find(|(id, _, _)| id.as_str() == "cargo")
            .unwrap()
            .1
            .clone();
        let (alloys, _) = capacity::max_more(
            &goods,
            &cargo_comp,
            &BTreeMap::from([(cid("volatiles"), 50), (cid("industrial_parts"), fit)]),
            &cid("structural_alloys"),
        )
        .unwrap();
        assert_eq!(cargo.get(&cid("structural_alloys")), Some(&alloys.min(55)));
        assert_eq!(extra, 40, "10 crew-days at 4 kg weigh as payload");
    }

    #[test]
    fn ic11_the_reserve_counts_the_other_consumables_unmet_needs() {
        let targets = FillTargets {
            tank: 37,
            locker: 19,
            cargo_t: 73,
            payload_kg: 0,
            plan: plan_with(37, 19),
            iterations: 1,
        };
        let all = reserve_for(&targets, 1, 0, Some(cr(60)), Some(cr(10)), None);
        assert_eq!(all, cr(36 * 60 + 19 * 10));
        let stores_only = reserve_for(
            &targets,
            1,
            0,
            Some(cr(60)),
            Some(cr(10)),
            Some(Consumable::Propellant),
        );
        assert_eq!(stores_only, cr(190));
        let fuel_only = reserve_for(
            &targets,
            1,
            0,
            Some(cr(60)),
            Some(cr(10)),
            Some(Consumable::Provisions),
        );
        assert_eq!(fuel_only, cr(2_160));
        // Aboard covers it, and an absent ask prices its term 0.
        assert_eq!(
            reserve_for(&targets, 40, 20, Some(cr(60)), Some(cr(10)), None),
            cr(0)
        );
        assert_eq!(
            reserve_for(&targets, 1, 0, None, Some(cr(10)), None),
            cr(190)
        );
    }

    #[test]
    fn ic14_the_first_restage_waits_a_sweep_and_a_half() {
        let market = i64::try_from(mush_market::SWEEP_PERIOD_SECS).expect("a small period");
        assert_eq!(ORDER_RESTAGE_SECS, market + 30);
        assert_ne!(
            ORDER_RESTAGE_SECS % market,
            0,
            "never on a market sweep boundary"
        );
    }

    /// **Doc 18 cut IC14 row 26: `later_cover` reads the open buys of the good between
    /// an order and the next open depart** — a later `fill` is `Fill`, later unit buys
    /// sum, done or skipped orders, other goods and anything past the depart count for
    /// nothing.
    #[test]
    fn ic14_later_cover_reads_up_to_the_next_open_depart() {
        let buy = |id: u32, good: &str, qty: Qty| {
            pending(
                id,
                What::Buy {
                    good: cid(good),
                    qty,
                    max_major: None,
                    on_short: OnShort::Partial,
                },
            )
        };
        let depart = |id: u32| {
            pending(
                id,
                What::Depart {
                    to: cid("stickney"),
                    not_before: crate::port::when::NotBefore::Now,
                    not_after_s: None,
                    on_fail: OnShort::Hold,
                    crew: Vec::new(),
                    throttle_permille: None,
                    via: None,
                },
            )
        };
        let rm = cid("propellant");
        let mut doc = Orders::fresh(ObjId(5011), ObjId(42), 1);
        let mut done = buy(3, "propellant", Qty::Units(50));
        done.state = OrderState::Done;
        doc.orders = vec![
            buy(1, "propellant", Qty::Units(1)),
            buy(2, "propellant", Qty::Units(4)),
            done,
            buy(4, "provisions", Qty::Units(9)),
            buy(5, "propellant", Qty::Units(6)),
            depart(6),
            buy(7, "propellant", Qty::Fill),
        ];
        doc.next_id = 8;
        assert_eq!(later_cover(&doc, 0, &rm), Cover::Units(10), "4 + 6 only");
        assert_eq!(
            later_cover(&doc, 4, &rm),
            Cover::Units(0),
            "nothing after 5"
        );
        assert_eq!(later_cover(&doc, 6, &rm), Cover::Units(0), "the last order");
        doc.orders[1] = buy(2, "propellant", Qty::Fill);
        assert_eq!(later_cover(&doc, 0, &rm), Cover::Fill, "a later fill");
        // A depart that is no longer open is not the boundary.
        doc.orders[5].state = OrderState::Skipped;
        doc.orders[1] = buy(2, "propellant", Qty::Units(4));
        assert_eq!(
            later_cover(&doc, 0, &rm),
            Cover::Fill,
            "past the skipped depart"
        );
    }

    #[test]
    fn ic11_the_settle_grace_covers_the_sweeps() {
        let inventory = i64::try_from(mush_inventory::SWEEP_PERIOD_SECS).unwrap();
        let market = i64::try_from(mush_market::SWEEP_PERIOD_SECS).unwrap();
        let economy = mush_economy::SWEEP_PERIOD_SECS;
        let fire = i64::try_from(crate::port_tick::PORT_TICK_PERIOD.as_secs()).unwrap();
        assert!(ORDER_SETTLE_GRACE_SECS >= inventory + market + economy + 2 * fire);
    }

    /// **Doc 18 cut IC12 (F9): `fill_reason` renders the hold each fill error
    /// produces** — the two compartment errors as `HELD_FILL` with the compartment, the
    /// need and the consumable's unit, the planner's as `HELD_FILL_PLAN` — exact strings
    /// built from the constants (`fill_targets`' own errors are pinned by
    /// `ic11_fill_refuses_past_six_iterations`).
    #[test]
    fn ic12_fill_reason_renders_each_hold() {
        let goods = goods();
        let port = "Ceres Freeport";
        let held = |comp: &str, need: &str, unit: &str| {
            HELD_FILL
                .replace("{compartment}", comp)
                .replace("{need}", need)
                .replace("{unit}", unit)
                .replace("{port}", port)
        };
        assert_eq!(
            fill_reason(
                &goods,
                &FillError::TankTooSmall { need: 90, room: 80 },
                port
            ),
            held("tank", "90", "t")
        );
        assert_eq!(
            fill_reason(
                &goods,
                &FillError::LockerTooSmall {
                    need: 200,
                    room: 120
                },
                port
            ),
            held("locker", "200", "crew-days")
        );
        let plan = HELD_FILL_PLAN.replace("{port}", port);
        assert_eq!(fill_reason(&goods, &FillError::NoFixedPoint, port), plan);
        assert_eq!(
            fill_reason(&goods, &FillError::Plan(PlanError::Overflow), port),
            plan
        );
        assert_eq!(
            held("tank", "90", "t"),
            "the tank cannot carry the 90 t the passage to Ceres Freeport needs."
        );
    }

    /// **Doc 18 cut IC12 (F9): a fill whose crew-days exceed the stores locker refuses
    /// with `LockerTooSmall`** — the third compartment arm `fill_targets` had no case
    /// for: a plan of 0 t and 200 crew-days over an empty Dray (the locker holds 120).
    #[test]
    fn ic12_fill_refuses_a_locker_too_small() {
        let goods = goods();
        let hungry = |_: u64| Ok(plan_with(0, 200));
        assert_eq!(
            fill_targets(
                &goods,
                &readings(&[]),
                &BTreeMap::new(),
                0,
                0,
                0,
                1,
                100,
                hungry
            ),
            Err(FillError::LockerTooSmall {
                need: 200,
                room: 120
            })
        );
    }

    /// **Doc 18 cut IC12 (F14): a refused closed call is logged once and held at the
    /// third** — `refused_call`, which only a market call refused after the step's
    /// pre-check reaches (`refused_after_call`; the planner pre-checks the house's
    /// funds, the stock and the hold, so no step on the shipped composition reaches it
    /// reproducibly, and this unit covers the counter): `REFUSED_RETRY` logged once at
    /// the first refusal, `refusals` 1 → 2 → 3 with `retry_at` set each time, and the
    /// stopped hold (`HELD_LINE`, the plan `Held`, the order `Held`) at
    /// [`MAX_ORDER_REFUSALS`].
    #[test]
    fn ic12_a_refused_call_logs_once_and_holds_at_three() {
        let w = World::new_minimal();
        let mut doc = Orders::fresh(ObjId(5011), ObjId(42), 1);
        doc.orders = vec![pending(
            1,
            What::Buy {
                good: cid("volatiles"),
                qty: Qty::Units(5),
                max_major: None,
                on_short: crate::port::orders::OnShort::Partial,
            },
        )];
        doc.next_id = 2;
        doc.derive_cursor();
        let line = "That listing is not taking quotes.";
        let retry = REFUSED_RETRY.replace("{n}", "1").replace("{line}", line);
        let mut out = Outcome::new(doc);
        for n in 1..=MAX_ORDER_REFUSALS {
            refused_call(&w, &mut out, 1, line);
            let order = out.doc.order(1).expect("the order");
            assert_eq!(order.refusals, n);
            assert_eq!(
                order.retry_at,
                Some(w.now().saturating_add(ORDER_RETRY_SECS))
            );
            let logged = out.doc.log.iter().filter(|l| l.text == retry).count();
            assert_eq!(logged, 1, "logged once, at the first refusal");
            if n < MAX_ORDER_REFUSALS {
                assert_eq!(out.doc.state, PlanState::Running);
                assert!(out.doc.hold.is_none());
            }
        }
        assert_eq!(MAX_ORDER_REFUSALS, 3);
        assert_eq!(out.doc.state, PlanState::Held);
        assert_eq!(out.doc.order(1).map(|o| o.state), Some(OrderState::Held));
        let hold = out.doc.hold.as_ref().expect("the stopped hold");
        assert_eq!(
            (hold.order, hold.reason.as_str(), hold.retry),
            (1, line, false)
        );
        let held = HELD_LINE.replace("{n}", "1").replace("{reason}", line);
        assert!(
            out.doc.log.iter().any(|l| l.text == held),
            "{:?}",
            out.doc.log
        );
        // A fourth refusal keeps the count at its bound and logs nothing new.
        refused_call(&w, &mut out, 1, line);
        assert_eq!(
            out.doc.order(1).map(|o| o.refusals),
            Some(MAX_ORDER_REFUSALS)
        );
        assert_eq!(out.doc.log.iter().filter(|l| l.text == retry).count(), 1);
    }

    /// **Doc 18 cut IC12: a queued sale that takes nothing names its reason**, in
    /// `decide`'s order — the floor first, then an exhausted intake (`INTAKE_TAKEN`,
    /// which no 100 t hull can produce on the shipped 4,000 t and 9,600 t intakes in a
    /// test), then a house that cannot pay, then the short clause — exact strings from
    /// the constants (a world with no economy renders money as bare minor units).
    #[test]
    fn ic12_a_sale_that_takes_nothing_names_its_reason() {
        let w = World::new_minimal();
        let bid = |remaining: u64, house_units: u64| Bid {
            listing: mush_market::ListingId(21),
            price: Amount::from_minor(28_000),
            remaining,
            house: "Kasei Metals".to_owned(),
            house_units,
        };
        let label = "metal concentrate";
        assert_eq!(
            sale_none_reason(&w, &bid(4_000, 900), label, "t", Some(100_000), 20, &[]),
            "Kasei Metals bids 28000/t, under your 100000."
        );
        assert_eq!(
            sale_none_reason(&w, &bid(0, 900), label, "t", None, 20, &[Bound::Intake]),
            INTAKE_TAKEN
                .replace("{house}", "Kasei Metals")
                .replace("{label}", label)
        );
        assert_eq!(
            sale_none_reason(&w, &bid(0, 900), label, "t", None, 20, &[Bound::Intake]),
            "Kasei Metals is not buying metal concentrate; its intake is taken."
        );
        assert_eq!(
            sale_none_reason(
                &w,
                &bid(4_000, 0),
                label,
                "t",
                None,
                20,
                &[Bound::HouseFunds]
            ),
            "Kasei Metals cannot pay for metal concentrate right now."
        );
        assert_eq!(
            sale_none_reason(
                &w,
                &bid(4_000, 900),
                label,
                "t",
                Some(1),
                20,
                &[Bound::Intake]
            ),
            "short of 20 t: intake."
        );
    }

    /// A pending order for the shape tests.
    fn pending(id: u32, what: What) -> crate::port::orders::Order {
        crate::port::orders::Order {
            id,
            state: OrderState::Pending,
            attempt: 1,
            refusals: 0,
            retry_at: None,
            announced: false,
            reached: None,
            what,
        }
    }

    /// A skip (and the `Done` paths, which clear the same way) takes the waiting note
    /// on the order passed over with it, so the document still checks once the cursor
    /// moves on — a stale `HELD_CHARTER` note (the hull renamed while the lease was
    /// out) never wedges the plan.
    #[test]
    fn ic11_a_skip_takes_its_waiting_note_with_it() {
        let w = World::new_minimal();
        let mut doc = Orders::fresh(ObjId(5011), ObjId(42), 1);
        doc.orders = vec![
            pending(
                1,
                What::Sell {
                    good: cid("metals"),
                    qty: Qty::All,
                    min_major: None,
                    on_short: OnShort::Partial,
                },
            ),
            pending(2, What::Stop),
        ];
        doc.next_id = 3;
        doc.derive_cursor();
        assert!(doc.wait_at(1, &HELD_CHARTER.replace("{name}", "Old Name"), 0));
        doc.check().expect("the waiting shape checks");
        let mut out = Outcome::new(doc);
        skip_or_hold(
            &w,
            &mut out,
            1,
            OnShort::Partial,
            &NO_BID.replace("{label}", "metal concentrate"),
        );
        out.doc.derive_cursor();
        assert!(out.doc.hold.is_none(), "the note went with the order");
        assert_eq!(out.doc.cursor, Some(2));
        out.doc.check().expect("the plan still checks");
    }

    /// `+ship/plan` names a departure refusal without "Nothing was burned." and
    /// without a second full stop.
    #[test]
    fn ic11_a_dry_run_hold_reads_as_one_sentence() {
        let line = crate::port::voyage::TANK_SHORT
            .replace("{port}", "Ceres Freeport")
            .replace("{burn}", "37")
            .replace("{tank}", "1");
        let clause = inline_reason(&line);
        assert_eq!(
            format!("burn 37 t; it would hold: {clause}."),
            "burn 37 t; it would hold: The plan for Ceres Freeport burns 37 t of reaction mass; the tank holds 1 t."
        );
    }

    /// A sell order in flight, staged at `staged_utc`, re-staged `restaged` times.
    fn sell_in_flight(restaged: u8) -> Orders {
        let mut doc = Orders::fresh(ObjId(5011), ObjId(42), 1);
        doc.orders = vec![pending(
            1,
            What::Sell {
                good: cid("metals"),
                qty: Qty::All,
                min_major: None,
                on_short: OnShort::Partial,
            },
        )];
        doc.next_id = 2;
        doc.derive_cursor();
        doc.inflight = Some(Inflight {
            order: 1,
            attempt: 1,
            key: "market.lot.deliver.7".to_owned(),
            refs: crate::port::orders::InflightRefs {
                quote: Some(3),
                contract: Some(7),
                ticket: None,
            },
            staged_utc: 0,
            issued_ic: 0,
            restaged,
            want: 20,
            qty: 20,
            bounds: Vec::new(),
            line: None,
        });
        doc
    }

    /// **Doc 18 cut IC14 row 3: a re-stage that finds the leg landed is neither a
    /// refusal nor a miss** — at the first and the second deadline it advances
    /// `restaged` exactly as `Ok(None)` does, holds nothing and counts nothing.
    #[test]
    fn ic14_a_landed_restage_advances_like_a_quiet_one() {
        let w = World::new_minimal();
        for (before, after) in [(0_u8, 1_u8), (1, 2)] {
            let landed = Err(MarketError::bad(mush_market::DELIVER_LANDED));
            let (out, step) =
                restage_outcome(&w, sell_in_flight(before), landed).expect("in flight");
            let (quiet, quiet_step) =
                restage_outcome(&w, sell_in_flight(before), Ok(None)).expect("in flight");
            assert_eq!(step, Step::Worked);
            assert_eq!(step, quiet_step);
            assert_eq!(out.doc, quiet.doc, "exactly what Ok(None) writes");
            assert_eq!(out.doc.inflight.as_ref().map(|r| r.restaged), Some(after));
            assert!(out.doc.hold.is_none(), "no hold");
            assert_eq!(out.doc.state, PlanState::Running);
            assert_eq!(out.doc.order(1).map(|o| o.refusals), Some(0), "no count");
        }
    }

    /// **Doc 18 cut IC14 (lead ruling L32, reviewer F15): a re-stage that cannot read
    /// the marker ends in the stopped hold**, never a silent retry: the first deadline
    /// advances `restaged` to 1, the second holds `HELD_UNCONFIRMED`.
    #[test]
    fn ic14_an_unconfirmed_restage_holds_at_the_second_deadline() {
        let w = World::new_minimal();
        let unconfirmed = || Err(MarketError::bad(mush_market::DELIVER_UNCONFIRMED));
        let (first, step) =
            restage_outcome(&w, sell_in_flight(0), unconfirmed()).expect("in flight");
        assert_eq!(step, Step::Worked);
        assert_eq!(first.doc.inflight.as_ref().map(|r| r.restaged), Some(1));
        assert!(first.doc.hold.is_none());
        let (second, _) = restage_outcome(&w, first.doc, unconfirmed()).expect("in flight");
        assert_eq!(second.doc.inflight.as_ref().map(|r| r.restaged), Some(2));
        assert_eq!(second.doc.state, PlanState::Held);
        let hold = second.doc.hold.as_ref().expect("the stopped hold");
        assert_eq!((hold.order, hold.reason.as_str()), (1, HELD_UNCONFIRMED));
    }

    /// **Doc 18 cut IC14 row 2 (reviewer F3): engine back-pressure is not a refusal.**
    /// Both of the sell gate's queue lines are back-pressure; nothing else is.
    #[test]
    fn ic14_the_queue_lines_are_back_pressure() {
        assert!(is_back_pressure(mush_market::SELL_QUEUE_FULL));
        assert!(is_back_pressure(mush_market::SELL_BUSY));
        assert!(!is_back_pressure(mush_market::HOUSE_CANNOT_PAY));
        assert!(!is_back_pressure(mush_market::NOT_BUYING));
    }

    /// **Doc 18 §9 risk 19, cut IC14 row 1: the marker retention horizon outlives
    /// every window that reads a marker** — the sale's draw window, the executor's
    /// first re-stage and its settle grace, and two market sweeps, each named.
    #[test]
    fn ic14_marker_retention_outlives_every_window_that_reads_a_marker() {
        let sweep = i64::try_from(mush_market::SWEEP_PERIOD_SECS).expect("a small period");
        let window = mush_market::DRAW_WINDOW_SECS
            + ORDER_RESTAGE_SECS
            + ORDER_SETTLE_GRACE_SECS
            + 2 * sweep;
        assert!(
            mush_inventory::MARKER_RETENTION_SECS >= window,
            "{} < {window}",
            mush_inventory::MARKER_RETENTION_SECS
        );
    }
}
