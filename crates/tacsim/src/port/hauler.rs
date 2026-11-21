//! **The NPC hauling house** (doc 48 unit EX.10; Q-48-20, Q-48-41).
//!
//! A port entry's `hauler` block ([`HaulerDef`]) names a house and the marks of its hulls. On the
//! port tick, for each hull, one step a fire:
//!
//! 1. **Its hull**: seeded at the port for the house (`crate::port::commission::seed`, the staff
//!    seed's path), laid up, docked and titled by the commission arm; then found by its mark.
//! 2. **Its helm**: a `DARK` Thing of the house's with its own credit account, the pools' ferry
//!    helm's shape. `crate::port::coop::helm_of` reads the hull it sails ([`hauler_hull`]), and
//!    the voyage counts it aboard as the house's one soul.
//! 3. **A trip, when a gap pays** (`choose`): docked at a lane port with no plan running, the
//!    house looks at every good it carries that this port's houses ask, and every port one route
//!    away whose houses bid for it. A trip's profit is the lot at the bid less the lot at the ask
//!    less the passage's fuel and stores at this port's asks; it must clear `margin_permille` of
//!    what the trip lays out. The best trip wins. With none, an empty hop to the neighbor whose
//!    own best trip pays for the hop, and otherwise she waits for the next IC day.
//! 4. **The plan**: the helm's purse is topped up from the house for the trip's outlay, and a
//!    sailing plan is written for the hull (`crate::port::orders::Orders`): buy the lot within the
//!    ask, fill the tank and the locker, depart, sell everything into the bid at no less than the
//!    ask. The IC11 executor flies it as the helm, exactly as it flies a captain's plan: the market
//!    legs, the fill, the departure, the re-stage and the restart rules are its.
//! 5. **Home**: once the plan is done and she is docked, what the helm holds past its float goes
//!    back to the house.
//!
//! **She can be intercepted.** Under way, the house's mind holds her helm: her link-loss record is
//! opened with the backup mind at the helm (`crate::space::tick::cover_house_hull`), so a crewed leg's
//! rules are hers. An interdicted leg (Ceres to Stickney, at Kirkwood) opens its picket's bubble on
//! her, where she heaves to under a customs hail and is inspected, and where the Belt Raider may
//! take her; she is on her passage again when the bubble lets her go, and the record ends when she
//! docks.
//!
//! **The restart argument.** The record ([`HaulerHouse`], `tacsim:hauler:{entry}`) holds the hull,
//! the helm, its purse and the trip in hand; the plan is its own document; every payment is keyed
//! by mark and trip. A fire re-derives where each hull stands from them.

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_economy::{
    Account, AccountId, Amount, EconomicCause, EconomyPlan, EconomyStore, IdempotencyKey,
    PostOutcome, RequestedLine,
};
use mush_inventory::record::EndpointRef;
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::definitions::goods::Consumable;
use crate::port::def::{HaulerDef, PortDef};
use crate::port::map::PortMap;
use crate::port::orders::{OnShort, Order, OrderState, Orders, OrdersState, Qty, What};
use crate::port::ship::Berth;
use crate::port::stock::IC_DAY;
use crate::port::when::NotBefore;
use crate::store::TacsimStore;

/// The document prefix of a port's hauling house: `tacsim:hauler:{entry}`.
pub const HAULER_PREFIX: &str = "tacsim:hauler:";
/// A hauler payment's idempotency scope.
pub const HAULER_KEY: &str = "tacsim.hauler";
/// The fuel bought over the passage's burn, per mille (the pools' ferry margin).
const FUEL_MARGIN_PERMILLE: u64 = 1_250;
/// The purse a trip is funded to, per mille of its outlay, so a moved price does not strand it.
const FUND_PERMILLE: u64 = 1_200;
/// A sister hull's trip in hand: its good, the port it left and the port it makes for.
type Taken = (Option<CanonicalId>, CanonicalId, CanonicalId);

/// `tacsim:hauler:{entry}`.
#[must_use]
pub fn hauler_key(entry: &CanonicalId) -> SmolStr {
    SmolStr::new(format!("{HAULER_PREFIX}{entry}"))
}

/// **A port's hauling house**, written whole when it changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HaulerHouse {
    /// The port entry.
    pub entry: CanonicalId,
    /// The first IC day it trades: the day after the tick first saw it, so a lane laid up today
    /// stands as authored until the day turns.
    pub opens_day: i64,
    /// Laid up by staff (`@tacsim/port haulers <entry> off`): its hulls finish what is under way
    /// and dock, and no new trip is planned until staff turn it back on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub laid_up: bool,
    /// One row a mark, in the block's order.
    pub rows: Vec<HaulerRow>,
}

/// **One hauler hull's row.**
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HaulerRow {
    /// Its registry mark.
    pub mark: String,
    /// The hull, once laid up and titled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull: Option<u32>,
    /// Its `DARK` helm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helm: Option<u32>,
    /// The helm's grant on the house's root scope (`economy.account.post`, with `vault.withdraw`
    /// beside it), `GrantId.0`: it acts for the house, and its buys land in the house's hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant: Option<u64>,
    /// The helm's purse, `AccountId.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<u64>,
    /// The trip in hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trip: Option<Trip>,
    /// Trips made.
    #[serde(default)]
    pub trips: u32,
    /// The IC day it last found nothing worth a trip, so it looks once a day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_day: Option<i64>,
    /// The IC day it last tried to sell what is aboard where it lay, so it tries once a day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sold_day: Option<i64>,
    /// Why it waits, for staff: what its last look found (at most [`MAX_NOTE`] bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// **The house lost this mark's hull** (48e services:G18, G19): its title passed to
    /// another, or it was wrecked or is gone. The row keeps its history and sails nothing again;
    /// the mark is neither re-adopted nor seeded afresh.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lost: bool,
}

/// The longest wait note a row keeps, bytes.
pub const MAX_NOTE: usize = 480;

/// **A trip in hand.**
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trip {
    /// The trip's number for this hull, from 1.
    pub n: u32,
    /// The good carried; absent for an empty hop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub good: Option<CanonicalId>,
    /// The venue it leaves from.
    pub from: CanonicalId,
    /// The venue it sails to.
    pub to: CanonicalId,
    /// Units planned.
    pub units: u64,
    /// The ask it buys within, major units a unit.
    pub ask_major: u64,
    /// The bid it was planned against, major units a unit.
    pub bid_major: u64,
    /// The profit it was planned to clear, minor units.
    pub profit_minor: i64,
    /// **The haul it carries** (doc 50 cut OP.4, Q-50-24), `ContractId.0`: an arm's haul no
    /// player took, carried hop by hop to its destination and delivered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub haul: Option<u64>,
}

// ---- readings ----------------------------------------------------------------------

/// **The hull `helm` sails** for its hauling house, when it is one's helm.
#[must_use]
pub fn hauler_hull(w: &World, helm: ObjId) -> Option<ObjId> {
    w.module_state::<TacsimStore>()?
        .haulers()
        .flat_map(|house| house.rows.iter())
        .find(|row| row.helm == Some(helm.0))
        .and_then(|row| row.hull)
        .map(ObjId)
}

/// **Every hauling house's helm**: the actors that file its market buys and sales.
#[must_use]
pub fn helms(w: &World) -> Vec<ObjId> {
    w.module_state::<TacsimStore>()
        .map_or_else(Vec::new, |store| {
            store
                .haulers()
                .flat_map(|house| house.rows.iter())
                .filter_map(|row| row.helm.map(ObjId))
                .collect()
        })
}

/// **The house account a hauling house's helm acts for** (doc 50 cut OP.4, Q-50-56): what the
/// house pays a picket's duty from when its hull crosses.
#[must_use]
pub fn house_account_of_helm(w: &World, helm: ObjId) -> Option<AccountId> {
    let store = w.module_state::<TacsimStore>()?;
    let house = store
        .haulers()
        .find(|house| house.rows.iter().any(|row| row.helm == Some(helm.0)))?;
    let (def, map) = ports_by_venue(w)
        .into_iter()
        .find(|(_, map)| map.entry == house.entry)?;
    let org = &def.hauler.as_deref()?.org;
    map.orgs.get(org)?.account.map(AccountId)
}

/// **Whether `hull` is a hauling house's.**
#[must_use]
pub fn is_hauler(w: &World, hull: ObjId) -> bool {
    w.module_state::<TacsimStore>().is_some_and(|store| {
        store
            .haulers()
            .flat_map(|house| house.rows.iter())
            .any(|row| row.hull == Some(hull.0))
    })
}

/// `minor` of the credit as whole credits, rounded down.
fn major_of(w: &World, minor: i64) -> u64 {
    let scale = crate::port::apply::credit_scale(w);
    let per = 10_i64.pow(u32::from(scale)).max(1);
    u64::try_from(minor.max(0) / per).unwrap_or(0)
}

/// `major` whole credits in minor units.
fn minor_of(w: &World, major: u64) -> i64 {
    crate::port::apply::minor_of(major, crate::port::apply::credit_scale(w)).unwrap_or(i64::MAX)
}

/// What an account can pay now, minor units.
fn available(w: &World, account: AccountId) -> i64 {
    w.module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().account(account).and_then(Account::available))
        .map_or(0, Amount::minor)
}

// ---- the choice ----------------------------------------------------------------------

/// **A trip the house could make**, priced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The good, absent for an empty hop.
    pub good: Option<CanonicalId>,
    /// The destination venue.
    pub to: CanonicalId,
    /// Units.
    pub units: u64,
    /// The ask, major units a unit.
    pub ask_major: u64,
    /// The bid, major units a unit.
    pub bid_major: u64,
    /// The passage's fuel and stores, minor units.
    pub stores_minor: i64,
    /// The profit, minor units (negative for an empty hop: its cost).
    pub profit_minor: i64,
    /// Carried already: the lot is aboard, so nothing is bought.
    pub carried: bool,
}

/// **Whether a trip pays** (Q-48-20): its profit clears `margin_permille` of what it lays out (the
/// lot at the ask and the stores), and is positive. Pure.
#[must_use]
pub fn pays(
    units: u64,
    ask_minor: i64,
    bid_minor: i64,
    stores_minor: i64,
    margin_permille: u32,
) -> Option<i64> {
    let units = i128::from(units);
    let cargo = units * i128::from(ask_minor);
    let profit = units * i128::from(bid_minor) - cargo - i128::from(stores_minor);
    let outlay = cargo + i128::from(stores_minor);
    let wanted = outlay * i128::from(margin_permille) / 1_000;
    (profit > 0 && profit >= wanted).then(|| i64::try_from(profit).unwrap_or(i64::MAX))
}

/// The lane's port defs and maps, by venue.
fn ports_by_venue(w: &World) -> Vec<(PortDef, PortMap)> {
    crate::port::market::ports(w)
        .into_iter()
        .filter(|(entry, _, _)| crate::port::stock::entry_latched(w, entry))
        .filter_map(|(_, def, map)| Some((def.clone(), map?.clone())))
        .collect()
}

/// **How many more units of `good` her stowage takes now**, `0` when it cannot be read.
fn room(ctx: &Ctx<'_>, hull: ObjId, good: &CanonicalId) -> u64 {
    let Some(goods) = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
    else {
        return 0;
    };
    let Some(view) = crate::port::ship::hull_view(ctx.w, hull) else {
        return 0;
    };
    crate::port::ship::readings_of(ctx, &view)
        .ok()
        .and_then(|readings| crate::port::executor::room_for(goods, &readings, good).ok())
        .map_or(0, |(more, _)| more)
}

/// **What her tank holds when full**, tonnes: its room now and what is aboard.
fn tank_t(ctx: &Ctx<'_>, hull: ObjId, propellant: &CanonicalId) -> u64 {
    let aboard = crate::port::ship::hull_view(ctx.w, hull)
        .and_then(|view| crate::port::ship::hold_vault(ctx.w, &view))
        .map_or(0, |hold| {
            crate::port_tick::read_pool(ctx, EndpointRef(hold), propellant).unwrap_or(0)
        });
    room(ctx, hull, propellant).saturating_add(aboard)
}

/// The passage's fuel and stores from where `hull` is docked to `to`, priced at this port's asks,
/// minor units; `None` when no route or price reads.
fn stores_minor(
    ctx: &Ctx<'_>,
    hull: ObjId,
    here: (&PortDef, &PortMap),
    to: &CanonicalId,
) -> Option<i64> {
    stores_priced_empty(ctx, hull, here, to).ok()
}

/// [`stores_priced`] with the passage's crew-days, its length in IC days.
fn stores_and_days(
    ctx: &Ctx<'_>,
    hull: ObjId,
    here: (&PortDef, &PortMap),
    to: &CanonicalId,
    extra_kg: u64,
) -> Result<(i64, u32), String> {
    stores_priced(ctx, hull, here, to, extra_kg)
}

/// [`stores_minor`] for an empty hold past what is aboard: the price, or why it is none.
fn stores_priced_empty(
    ctx: &Ctx<'_>,
    hull: ObjId,
    here: (&PortDef, &PortMap),
    to: &CanonicalId,
) -> Result<i64, String> {
    stores_priced(ctx, hull, here, to, 0).map(|(stores, _)| stores)
}

/// **The passage's fuel and stores** with `extra_kg` more aboard than she holds, priced at this
/// port's asks, minor units, and its crew-days; or why it cannot be flown. The burn is the least
/// fixed point of "the burn with its own fuel aboard" (the executor's `fill`), three passes, and
/// it must fit her tank.
fn stores_priced(
    ctx: &Ctx<'_>,
    hull: ObjId,
    (def, map): (&PortDef, &PortMap),
    to: &CanonicalId,
    extra_kg: u64,
) -> Result<(i64, u32), String> {
    let goods = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
        .ok_or("no goods table")?;
    let propellant = crate::port::burn::consumable_good(goods, Consumable::Propellant)
        .ok_or("no propellant good")?;
    let provisions = crate::port::burn::consumable_good(goods, Consumable::Provisions)
        .ok_or("no provisions good")?;
    let fuel_kg = goods
        .good(&propellant)
        .and_then(|g| g.mass_kg_per_unit())
        .unwrap_or(crate::port::burn::PROPELLANT_KG_PER_UNIT);
    let mut burn = 0_u64;
    let mut days = 0_u32;
    for _ in 0..3 {
        let (b, d) = crate::port::voyage::passage_needs_laden(
            ctx,
            hull,
            to.as_str(),
            1,
            extra_kg.saturating_add(burn.saturating_mul(fuel_kg)),
        )?;
        burn = b;
        days = d;
    }
    let tank = tank_t(ctx, hull, &propellant);
    if burn > tank {
        return Err(format!(
            "the tank carries {tank} t, the passage burns {burn} t with the lot aboard"
        ));
    }
    let fuel = burn
        .saturating_mul(FUEL_MARGIN_PERMILLE)
        .div_ceil(1_000)
        .max(1)
        .min(tank);
    let stores = u64::from(days).saturating_add(1);
    let fuel_minor = stores_price(ctx, hull, (def, map), &propellant, fuel)?;
    let stores_minor = stores_price(ctx, hull, (def, map), &provisions, stores)?;
    Ok((fuel_minor.saturating_add(stores_minor), days))
}

/// **What `need` units of a ship's store cost her here**, minor units: at this port's ask; or,
/// where no house here sells it (Isidis, doc 50b item 1), out of what is aboard, valued at the
/// lowest ask the lane posts for it, since she buys it back there. Why it is none: no ask here
/// while she lacks it, or none anywhere.
fn stores_price(
    ctx: &Ctx<'_>,
    hull: ObjId,
    (def, map): (&PortDef, &PortMap),
    good: &CanonicalId,
    need: u64,
) -> Result<i64, String> {
    let units = i64::try_from(need).unwrap_or(i64::MAX);
    if let Some(ask) = crate::port::executor::ask_at(ctx.w, def, map, good) {
        return Ok(ask.price.minor().saturating_mul(units));
    }
    if def.authors_ask(good) {
        // A house here sells it, out of stock for now: look again later.
        return Err(format!("no {good} ask here"));
    }
    let aboard = aboard(ctx, hull, good);
    if aboard < need {
        return Err(format!(
            "no {good} ask here, and {aboard} aboard of the {need} the passage needs"
        ));
    }
    ports_by_venue(ctx.w)
        .iter()
        .filter_map(|(d, m)| crate::port::executor::ask_at(ctx.w, d, m, good))
        .map(|ask| ask.price.minor())
        .min()
        .map(|price| price.saturating_mul(units))
        .ok_or_else(|| format!("no {good} ask on the lane"))
}

/// **How much of `good` is aboard her**, every compartment; `0` when it cannot be read.
fn aboard(ctx: &Ctx<'_>, hull: ObjId, good: &CanonicalId) -> u64 {
    crate::port::ship::hull_view(ctx.w, hull)
        .and_then(|view| crate::port::ship::readings_of(ctx, &view).ok())
        .map_or(0, |readings| {
            readings
                .iter()
                .filter_map(|(_, _, contents)| contents.get(good))
                .sum()
        })
}

/// The best trip carrying one of `hauler`'s goods from this port to one port away, if one pays.
/// **The best trip one port on from `there`**, its profit, priced with `stores` as the onward
/// passage's (this hull's passage from here stands in for it: the hull is not there to plan from).
fn onward(
    ctx: &Ctx<'_>,
    hauler: &HaulerDef,
    (there_def, there_map): (&PortDef, &PortMap),
    ports: &[(PortDef, PortMap)],
    stores: i64,
) -> Option<i64> {
    there_def
        .routes
        .keys()
        .filter_map(|next| ports.iter().find(|(def, _)| &def.venue == next))
        .flat_map(|(next_def, next_map)| {
            hauler.goods.iter().filter_map(move |good| {
                let ask = crate::port::executor::ask_at(ctx.w, there_def, there_map, good)?;
                let bid = crate::port::executor::bid_at(ctx.w, next_def, next_map, good)?;
                let units = hauler
                    .lot
                    .min(ask.quantity)
                    .min(bid.remaining)
                    .min(bid.house_units);
                pays(
                    units,
                    ask.price.minor(),
                    bid.price.minor(),
                    stores,
                    hauler.margin_permille,
                )
            })
        })
        .max()
}

/// **The best paying trip from here**: of every carried good asked here and bid one port on, the
/// one that pays, ranked by its own profit and the best trip from where it lands (one look ahead,
/// so a house does not shuttle between two ports while a richer circuit stands).
fn best_trip(
    ctx: &Ctx<'_>,
    hauler: &HaulerDef,
    hull: ObjId,
    here: (&PortDef, &PortMap),
    ports: &[(PortDef, PortMap)],
    taken: &[Taken],
) -> Option<Choice> {
    let mut best: Option<(i64, Choice)> = None;
    for to in here.0.routes.keys() {
        let Some((there_def, there_map)) = ports.iter().find(|(def, _)| &def.venue == to) else {
            continue;
        };
        let Ok((empty, _)) = stores_and_days(ctx, hull, here, to, 0) else {
            continue;
        };
        let ahead = onward(ctx, hauler, (there_def, there_map), ports, empty).unwrap_or(0);
        for good in &hauler.goods {
            // A sister hull already carries this lot from here to there.
            if taken
                .iter()
                .any(|(g, from, t)| g.as_ref() == Some(good) && from == &here.0.venue && t == to)
            {
                continue;
            }
            let Some(ask) = crate::port::executor::ask_at(ctx.w, here.0, here.1, good) else {
                continue;
            };
            let Some(bid) = crate::port::executor::bid_at(ctx.w, there_def, there_map, good) else {
                continue;
            };
            let mut units = hauler
                .lot
                .min(ask.quantity)
                .min(bid.remaining)
                .min(bid.house_units)
                .min(room(ctx, hull, good));
            // The lot aboard, halved until her tank carries the passage (at most four times).
            let unit_kg = ctx
                .w
                .module_state::<TacsimStore>()
                .and_then(TacsimStore::goods)
                .and_then(|goods| goods.good(good).and_then(|g| g.mass_kg_per_unit()))
                .unwrap_or(1_000);
            let mut priced = None;
            for _ in 0..5 {
                if units == 0 {
                    break;
                }
                if let Ok(found) =
                    stores_and_days(ctx, hull, here, to, units.saturating_mul(unit_kg))
                {
                    priced = Some(found);
                    break;
                }
                units /= 2;
            }
            let Some((stores, days)) = priced else {
                continue;
            };
            let Some(profit) = pays(
                units,
                ask.price.minor(),
                bid.price.minor(),
                stores,
                hauler.margin_permille,
            ) else {
                continue;
            };
            // Profit an IC day of passage: a short paying trip beats a long one.
            let rank = profit.saturating_add(ahead).saturating_mul(1_000)
                / i64::from(days).saturating_add(1);
            if best.as_ref().is_none_or(|(b, _)| rank > *b) {
                best = Some((
                    rank,
                    Choice {
                        good: Some(good.clone()),
                        to: to.clone(),
                        units,
                        ask_major: major_of(ctx.w, ask.price.minor()),
                        bid_major: major_of(ctx.w, bid.price.minor()),
                        stores_minor: stores,
                        profit_minor: profit,
                        carried: false,
                    },
                ));
            }
        }
    }
    best.map(|(_, choice)| choice)
}

/// **The best trip for a lot already aboard** that no house here takes: to the best bid one route
/// on, its cost the passage alone (what it cost is sunk), so long as the sale clears the margin of
/// the stores.
fn carry_trip(
    ctx: &Ctx<'_>,
    hauler: &HaulerDef,
    hull: ObjId,
    here: (&PortDef, &PortMap),
    ports: &[(PortDef, PortMap)],
) -> Option<Choice> {
    let view = crate::port::ship::hull_view(ctx.w, hull)?;
    let hold = crate::port::ship::hold_vault(ctx.w, &view)?;
    let mut best: Option<Choice> = None;
    for good in &hauler.goods {
        let aboard = crate::port_tick::read_pool(ctx, EndpointRef(hold), good).unwrap_or(0);
        if aboard == 0 {
            continue;
        }
        for to in here.0.routes.keys() {
            let Some((there_def, there_map)) = ports.iter().find(|(def, _)| &def.venue == to)
            else {
                continue;
            };
            let Some(stores) = stores_minor(ctx, hull, here, to) else {
                continue;
            };
            let Some(bid) = crate::port::executor::bid_at(ctx.w, there_def, there_map, good) else {
                continue;
            };
            let units = aboard.min(bid.remaining).min(bid.house_units);
            let Some(profit) = pays(units, 0, bid.price.minor(), stores, hauler.margin_permille)
            else {
                continue;
            };
            if best.as_ref().is_none_or(|b| profit > b.profit_minor) {
                best = Some(Choice {
                    good: Some(good.clone()),
                    to: to.clone(),
                    units,
                    ask_major: 0,
                    bid_major: major_of(ctx.w, bid.price.minor()),
                    stores_minor: stores,
                    profit_minor: profit,
                    carried: true,
                });
            }
        }
    }
    best
}

/// **The trip the house makes from here**, if any: the best paying trip; else an empty hop to the
/// neighbor whose best onward trip (read with this hull's passage from here as a stand-in) pays
/// for the hop and more.
fn choose(
    ctx: &Ctx<'_>,
    hauler: &HaulerDef,
    hull: ObjId,
    here: (&PortDef, &PortMap),
    taken: &[Taken],
) -> Option<Choice> {
    let ports = ports_by_venue(ctx.w);
    if let Some(trip) = carry_trip(ctx, hauler, hull, here, &ports) {
        return Some(trip);
    }
    if let Some(trip) = best_trip(ctx, hauler, hull, here, &ports, taken) {
        return Some(trip);
    }
    let mut hop: Option<Choice> = None;
    for to in here.0.routes.keys() {
        let Some((there_def, there_map)) = ports.iter().find(|(def, _)| &def.venue == to) else {
            continue;
        };
        let Some(cost) = stores_minor(ctx, hull, here, to) else {
            continue;
        };
        let onward = onward(ctx, hauler, (there_def, there_map), &ports, cost);
        let Some(onward) = onward else {
            continue;
        };
        if onward > cost && hop.as_ref().is_none_or(|h| onward - cost > h.profit_minor) {
            hop = Some(Choice {
                good: None,
                to: to.clone(),
                units: 0,
                ask_major: 0,
                bid_major: 0,
                stores_minor: cost,
                profit_minor: onward - cost,
                carried: false,
            });
        }
    }
    hop
}

/// **What a look found, for staff**: each route's passage cost and each carried good's ask, bid,
/// lot and net, in one line of at most [`MAX_NOTE`] bytes.
fn explain(ctx: &Ctx<'_>, hauler: &HaulerDef, hull: ObjId, here: (&PortDef, &PortMap)) -> String {
    let ports = ports_by_venue(ctx.w);
    let mut parts = Vec::new();
    for to in here.0.routes.keys() {
        let Some((there_def, there_map)) = ports.iter().find(|(def, _)| &def.venue == to) else {
            continue;
        };
        let stores = match stores_priced_empty(ctx, hull, here, to) {
            Ok(stores) => stores,
            Err(why) => {
                parts.push(format!("{to}: {why}"));
                continue;
            }
        };
        for good in &hauler.goods {
            let ask = crate::port::executor::ask_at(ctx.w, here.0, here.1, good);
            let bid = crate::port::executor::bid_at(ctx.w, there_def, there_map, good);
            if let (Some(ask), Some(bid)) = (ask, bid) {
                let units = hauler
                    .lot
                    .min(ask.quantity)
                    .min(bid.remaining)
                    .min(bid.house_units);
                let net = i64::try_from(units)
                    .unwrap_or(0)
                    .saturating_mul(bid.price.minor().saturating_sub(ask.price.minor()))
                    .saturating_sub(stores);
                parts.push(format!(
                    "{to} {good}: {units} at {}->{}, stores {stores}, net {net}",
                    ask.price.minor(),
                    bid.price.minor()
                ));
            }
        }
    }
    let mut note = if parts.is_empty() {
        "nothing carried is asked here and bid one route on".to_owned()
    } else {
        parts.join("; ")
    };
    if note.len() > MAX_NOTE {
        let mut cut = MAX_NOTE;
        while !note.is_char_boundary(cut) {
            cut -= 1;
        }
        note.truncate(cut);
    }
    note
}

// ---- the plan ------------------------------------------------------------------------

/// **The sailing plan for a choice**: the lot within the ask, the fill, the departure, the sale
/// at no less than the ask (an empty hop: the fill and the departure). Reaction mass held for a
/// carried haul is the executor's fill's to count (doc 50b item 7), so nothing is bought back.
fn plan(ctx: &Ctx<'_>, hull: ObjId, helm: ObjId, choice: &Choice) -> Option<Orders> {
    let goods = ctx.w.module_state::<TacsimStore>()?.goods()?;
    let propellant = crate::port::burn::consumable_good(goods, Consumable::Propellant)?;
    let provisions = crate::port::burn::consumable_good(goods, Consumable::Provisions)?;
    let mut whats = Vec::new();
    if let Some(good) = choice.good.as_ref().filter(|_| !choice.carried) {
        whats.push(What::Buy {
            good: good.clone(),
            qty: Qty::Units(choice.units),
            max_major: Some(choice.ask_major.max(1)),
            on_short: OnShort::Partial,
        });
    }
    for good in [propellant, provisions] {
        whats.push(What::Buy {
            good,
            qty: Qty::Fill,
            max_major: None,
            on_short: OnShort::Partial,
        });
    }
    whats.push(What::Depart {
        to: choice.to.clone(),
        not_before: NotBefore::Now,
        not_after_s: None,
        on_fail: OnShort::Hold,
        crew: Vec::new(),
        throttle_permille: None,
        via: None,
    });
    if let Some(good) = &choice.good {
        whats.push(What::Sell {
            good: good.clone(),
            qty: Qty::All,
            min_major: (!choice.carried).then_some(choice.ask_major.max(1)),
            on_short: OnShort::Partial,
        });
    }
    let first = crate::port::orders::seed_next_id(ctx, hull);
    let mut doc = Orders::fresh(hull, helm, first);
    for (n, what) in whats.into_iter().enumerate() {
        let id = first.saturating_add(u32::try_from(n).ok()?);
        doc.orders.push(Order {
            id,
            state: OrderState::Pending,
            attempt: 1,
            refusals: 0,
            retry_at: None,
            announced: false,
            reached: None,
            what,
            awaited: None,
        });
        doc.next_id = id.saturating_add(1);
    }
    doc.derive_cursor();
    Some(doc)
}

/// **A sale plan for what is aboard**: every carried good the hold still holds, into this port's
/// bids at any price.
fn sell_what_is_aboard(
    ctx: &Ctx<'_>,
    hauler: &HaulerDef,
    hull: ObjId,
    helm: ObjId,
) -> Option<Orders> {
    let view = crate::port::ship::hull_view(ctx.w, hull)?;
    let hold = crate::port::ship::hold_vault(ctx.w, &view)?;
    let (here_def, here_map) = match crate::port::ship::berth_of(ctx.w, hull) {
        Berth::Docked { def, map } => (def, map),
        _ => return None,
    };
    // Only what a house here bids for, with intake left and the money to pay.
    let aboard: Vec<CanonicalId> = hauler
        .goods
        .iter()
        .filter(|good| crate::port_tick::read_pool(ctx, EndpointRef(hold), good).unwrap_or(0) > 0)
        .filter(|good| {
            crate::port::executor::bid_at(ctx.w, here_def, here_map, good)
                .is_some_and(|bid| bid.remaining > 0 && bid.house_units > 0)
        })
        .cloned()
        .collect();
    if aboard.is_empty() {
        return None;
    }
    let first = crate::port::orders::seed_next_id(ctx, hull);
    let mut doc = Orders::fresh(hull, helm, first);
    for (n, good) in aboard.into_iter().enumerate() {
        let id = first.saturating_add(u32::try_from(n).ok()?);
        doc.orders.push(Order {
            id,
            state: OrderState::Pending,
            attempt: 1,
            refusals: 0,
            retry_at: None,
            announced: false,
            reached: None,
            what: What::Sell {
                good,
                qty: Qty::All,
                min_major: None,
                on_short: OnShort::Skip,
            },
            awaited: None,
        });
        doc.next_id = id.saturating_add(1);
    }
    doc.derive_cursor();
    Some(doc)
}

// ---- money ---------------------------------------------------------------------------

/// One keyed transfer of `minor` from `from` to `to`, as the tick.
fn pay(
    ctx: &mut Ctx<'_>,
    key: String,
    (from, to): (AccountId, AccountId),
    minor: i64,
    why: String,
) -> bool {
    pay_in(ctx, HAULER_KEY, key, (from, to), minor, why)
}

/// **A transfer of `minor` from one account to another**, under the idempotency key `key` in
/// namespace `ns` (doc 50b item 8: the Freeport arm's own, for the tank farm's fills). A
/// repeated key is answered as posted. Answers whether it posted.
pub(crate) fn pay_in(
    ctx: &mut Ctx<'_>,
    ns: &'static str,
    key: String,
    (from, to): (AccountId, AccountId),
    minor: i64,
    why: String,
) -> bool {
    if minor <= 0 {
        return true;
    }
    let (Some(debit), credit) = (
        Amount::from_minor(minor).checked_neg(),
        Amount::from_minor(minor),
    ) else {
        return false;
    };
    let plan = EconomyPlan {
        description: why,
        visibility: mush_economy::EconomicVisibility::Public,
        ..EconomyPlan::new(
            IdempotencyKey::new(ns, key),
            PrincipalId::Object(ctx.actor),
            EconomicCause::Internal {
                kind: SmolStr::new_static(ns),
                id: from.0,
            },
            vec![
                RequestedLine {
                    account: from,
                    delta: debit,
                    memo: SmolStr::new_static(ns),
                },
                RequestedLine {
                    account: to,
                    delta: credit,
                    memo: SmolStr::new_static(ns),
                },
            ],
        )
    };
    matches!(
        mush_economy::post(ctx, &plan),
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_))
    )
}

/// **The helm's grants on the house's root scope**: `economy.account.post` and `vault.withdraw`,
/// in one org plan. The first grant's id.
fn grant_helm(
    ctx: &mut Ctx<'_>,
    org: OrgId,
    root: mush_org::OrgScopeId,
    helm: ObjId,
) -> Result<u64, String> {
    let post = crate::port::apply::alloc_grant_id(ctx)?;
    let hold = crate::port::apply::alloc_grant_id(ctx)?;
    let put = |id: mush_org::GrantId, capability: &str| -> Result<_, String> {
        Ok(mush_org::OrgMutationOp::PutGrant {
            grant: mush_org::CapabilityGrant {
                id,
                org,
                grantee: helm,
                capability: mush_org::CapabilityId::new(capability)
                    .ok_or_else(|| format!("{capability} is not a capability id"))?,
                scope: mush_org::ScopeSelector::Exact(root),
                limit: None,
                delegable: false,
                valid_from: None,
                valid_until: None,
                revision: 1,
            },
        })
    };
    let plan = mush_org::OrgMutationPlan::of(vec![
        put(post, crate::port::house::POST_CAPABILITY)?,
        put(hold, crate::port::house::HOLD_CAPABILITY)?,
    ]);
    mush_org::submit(
        ctx,
        &plan,
        mush_org::OrgAction {
            action: "tacsim.hauler.helm".into(),
            org: Some(org),
            detail: serde_json::json!({ "helm": helm.0 }),
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(post.0)
}

// ---- an untaken haul (doc 50 cut OP.4) -------------------------------------------------

/// What a carried haul's purse is funded to past the bond, minor units: the first hop's stores.
const CARRY_FLOAT_MINOR: i64 = 100_000;

/// **Take an arm's untaken haul at this port** (Q-50-24): the purse funded for the bond and the
/// first hop, then `+haul/accept` as the helm, so the goods load aboard under the haul's hold.
/// Answers whether a haul was taken.
fn take_haul(
    ctx: &mut Ctx<'_>,
    hauler: &HaulerDef,
    (hull, helm): (ObjId, ObjId),
    (purse, house_account): (AccountId, AccountId),
    here: &PortDef,
    row: &mut HaulerRow,
) -> bool {
    let now = ctx.w.world_ic_now().at;
    let Some(contract) = crate::stockpile::untaken(ctx.w, here.venue.as_str(), now, &[]) else {
        return false;
    };
    let Some(job) = crate::services::haul::haul_of(ctx.w, contract) else {
        return false;
    };
    let n = row.trips.saturating_add(1);
    let want = job.bond_minor.saturating_add(CARRY_FLOAT_MINOR);
    let short = want.saturating_sub(available(ctx.w, purse));
    if short > available(ctx.w, house_account) {
        return false;
    }
    if !pay(
        ctx,
        format!("{}/{n}/haul-fund", row.mark),
        (house_account, purse),
        short,
        format!("{} funds hauler {} to carry a haul", hauler.org, row.mark),
    ) {
        return false;
    }
    let saved = ctx.actor;
    ctx.actor = helm;
    let taken = crate::services::haul::accept_as(ctx, &format!("#{contract}"));
    ctx.actor = saved;
    match taken {
        Ok(_) => {
            tracing::info!(
                mark = %row.mark, contract, hull = hull.0,
                "tacsim: a hauling house takes an untaken haul"
            );
            row.note = None;
            row.trip = Some(Trip {
                n,
                good: Some(job.good.clone()),
                from: here.venue.clone(),
                to: CanonicalId::new(job.destination.as_str())
                    .unwrap_or_else(|_| here.venue.clone()),
                units: job.quantity,
                ask_major: 0,
                bid_major: 0,
                profit_minor: job.fee_minor,
                haul: Some(contract),
            });
            true
        }
        Err(why) => {
            let mut note = format!("could not take haul {contract}: {why}");
            note.truncate(MAX_NOTE);
            row.note = Some(note);
            false
        }
    }
}

/// **The next port toward `to`** from `here`: `to` itself when a route joins them, else a
/// neighbor with a route to it.
fn next_hop(w: &World, here: &PortDef, to: &CanonicalId) -> Option<CanonicalId> {
    if here.routes.contains_key(to) {
        return Some(to.clone());
    }
    let ports = ports_by_venue(w);
    here.routes
        .keys()
        .find(|via| {
            ports
                .iter()
                .any(|(def, _)| &def.venue == *via && def.routes.contains_key(to))
        })
        .cloned()
}

/// **One step of a carried haul** (Q-50-24): loading; a hop's plan (the fill and the departure)
/// at each port short of the destination; the delivery there, as the helm; and the trip closed
/// once the haul has ended, the purse's surplus (the fee among it) sent home.
fn carry(
    ctx: &mut Ctx<'_>,
    hauler: &HaulerDef,
    (hull, helm): (ObjId, ObjId),
    (purse, house_account): (AccountId, AccountId),
    (here_def, here_map): (&PortDef, &PortMap),
    state: Option<&Orders>,
    row: &mut HaulerRow,
) {
    use crate::services::haul::HaulStage;
    let Some(trip) = row.trip.clone() else {
        return;
    };
    let Some(contract) = trip.haul else {
        return;
    };
    let job = crate::services::haul::haul_of(ctx.w, contract);
    let running = state.is_some_and(|doc| doc.author == helm && plan_end(doc).is_none());
    match job {
        Some(job) if job.stage == HaulStage::Taken => {}
        Some(job) if job.stage == HaulStage::Aboard => {
            // A plan under way, or a leg owed (the delivery on its way), is someone else's step.
            if running || job.owed.is_some() {
                return;
            }
            if here_def.venue.as_str() == job.destination.as_str() {
                let saved = ctx.actor;
                ctx.actor = helm;
                let delivered = crate::services::haul::deliver_as(ctx, contract);
                ctx.actor = saved;
                if let Err(why) = delivered {
                    tracing::warn!(
                        mark = %row.mark, contract, %why,
                        "tacsim: a carried haul could not be delivered"
                    );
                }
                return;
            }
            let Ok(to) = CanonicalId::new(job.destination.as_str()) else {
                return;
            };
            hop_toward(
                ctx,
                hauler,
                (hull, helm),
                (purse, house_account),
                (here_def, here_map),
                (&to, "haul-hop"),
                row,
            );
        }
        // Doc 50b item 1: the haul ended away from the port she took it at (delivered at Isidis,
        // where nothing is bid that she carries): she sails back there hop by hop, where the arm
        // posts its hauls, before the trip ends. A hop whose plan held ends it where she lies.
        _ if here_def.venue != trip.from
            && !state.is_some_and(|doc| doc.author == helm && plan_end(doc) == Some(true)) =>
        {
            if running {
                return;
            }
            hop_toward(
                ctx,
                hauler,
                (hull, helm),
                (purse, house_account),
                (here_def, here_map),
                (&trip.from, "haul-home"),
                row,
            );
        }
        // Delivered, lapsed, lost, withdrawn or gone: the trip ends, and what it earned goes home.
        _ => {
            row.trip = None;
            row.trips = row.trips.saturating_add(1);
            let float = minor_of(ctx.w, hauler.float_major);
            let surplus = available(ctx.w, purse).saturating_sub(float);
            let _ = pay(
                ctx,
                format!("{}/{}/home", row.mark, trip.n),
                (purse, house_account),
                surplus,
                format!("{} hauler {} pays its haul home", hauler.org, row.mark),
            );
        }
    }
}

/// **A carried trip's next hop toward `to`** (doc 50 cut OP.4, doc 50b item 1): the purse funded
/// for the hop's stores under a ledger key of `leg` and the hop's port, then the plan (the fill and
/// the departure). A port with no route on toward `to` notes it for staff. Answers whether the
/// hop's plan was written.
fn hop_toward(
    ctx: &mut Ctx<'_>,
    hauler: &HaulerDef,
    (hull, helm): (ObjId, ObjId),
    (purse, house_account): (AccountId, AccountId),
    (here_def, here_map): (&PortDef, &PortMap),
    (to, leg): (&CanonicalId, &str),
    row: &mut HaulerRow,
) -> bool {
    let Some(n) = row.trip.as_ref().map(|trip| trip.n) else {
        return false;
    };
    let Some(next) = next_hop(ctx.w, here_def, to) else {
        let mut note = format!("trip {n}: no route toward {to}");
        note.truncate(MAX_NOTE);
        row.note = Some(note);
        return false;
    };
    let stores = stores_minor(ctx, hull, (here_def, here_map), &next).unwrap_or(0);
    let want = i64::try_from(
        u64::try_from(stores)
            .unwrap_or(0)
            .saturating_mul(FUND_PERMILLE)
            / 1_000,
    )
    .unwrap_or(i64::MAX);
    let short = want.saturating_sub(available(ctx.w, purse));
    if short > 0
        && (short > available(ctx.w, house_account)
            || !pay(
                ctx,
                format!("{}/{n}/{leg}/{next}", row.mark),
                (house_account, purse),
                short,
                format!("{} funds hauler {} for a hop", hauler.org, row.mark),
            ))
    {
        return false;
    }
    let choice = Choice {
        good: None,
        to: next,
        units: 0,
        ask_major: 0,
        bid_major: 0,
        stores_minor: stores,
        profit_minor: 0,
        carried: true,
    };
    let Some(doc) = plan(ctx, hull, helm, &choice) else {
        return false;
    };
    if let Err(error) = crate::store::put_orders(ctx, &doc) {
        tracing::warn!(
            mark = %row.mark, %error,
            "tacsim: a carried trip's hop could not be planned"
        );
        return false;
    }
    true
}

/// **Called home for the Freeport arm's work** (doc 50 cut OP.8; Q-50-24): a hull lying idle
/// away from her home port, with an arm's haul on the board there ([`crate::stockpile::awaiting`]),
/// sets out for home empty, one hop at a time, before she looks for a trade. Each hop is a trip of
/// its own that ends where it docks. Answers whether she set out.
fn called_home(
    ctx: &mut Ctx<'_>,
    hauler: &HaulerDef,
    (hull, helm): (ObjId, ObjId),
    (purse, house_account): (AccountId, AccountId),
    (here_def, here_map): (&PortDef, &PortMap),
    home: &CanonicalId,
    row: &mut HaulerRow,
) -> bool {
    let Some(home_venue) = ports_by_venue(ctx.w)
        .into_iter()
        .find(|(_, map)| &map.entry == home)
        .map(|(def, _)| def.venue)
    else {
        return false;
    };
    let now = ctx.w.world_ic_now().at;
    if here_def.venue == home_venue
        || crate::stockpile::awaiting(ctx.w, home_venue.as_str(), now).is_none()
    {
        return false;
    }
    row.trip = Some(Trip {
        n: row.trips.saturating_add(1),
        good: None,
        from: here_def.venue.clone(),
        to: home_venue.clone(),
        units: 0,
        ask_major: 0,
        bid_major: 0,
        profit_minor: 0,
        haul: None,
    });
    if hop_toward(
        ctx,
        hauler,
        (hull, helm),
        (purse, house_account),
        (here_def, here_map),
        (&home_venue, "called-home"),
        row,
    ) {
        tracing::info!(
            mark = %row.mark, from = %here_def.venue, home = %home_venue,
            "tacsim: a hauler is called home for the Freeport arm's haul"
        );
        row.note = None;
        return true;
    }
    row.trip = None;
    false
}

// ---- the tick --------------------------------------------------------------------------

/// A house the tick has not seen yet: it opens the IC day after `today`.
fn fresh_house(entry: &CanonicalId, today: i64) -> HaulerHouse {
    HaulerHouse {
        entry: entry.clone(),
        opens_day: today + 1,
        laid_up: false,
        rows: Vec::new(),
    }
}

/// **`@tacsim/port haulers <entry> [on|off]`** (staff): the house's rows, one line a hull, or
/// lay it up (`off`) or open it again (`on`). A house the tick has not seen yet is written
/// fresh, so a world laid up at boot never seeds a hull or trades. The reply, or why there is none.
///
/// # Errors
///
/// A port entry that has no hauling house.
pub(crate) fn haulers_cmd(
    ctx: &mut Ctx<'_>,
    entry: &CanonicalId,
    set: Option<bool>,
) -> Result<Vec<String>, String> {
    let has_block = ports_by_venue(ctx.w)
        .iter()
        .any(|(def, map)| &map.entry == entry && def.hauler.is_some());
    if !has_block {
        return Err(format!("{entry} has no hauling house."));
    }
    let today = ctx.w.world_ic_now().at.div_euclid(IC_DAY);
    let mut house = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(|store| store.hauler(entry).cloned())
        .unwrap_or_else(|| fresh_house(entry, today));
    if let Some(open) = set {
        house.laid_up = !open;
        crate::store::put_hauler(ctx, &house);
    }
    let mut out = vec![format!(
        "{entry} hauling house: {}, trading from IC day {}.",
        if house.laid_up { "laid up" } else { "open" },
        house.opens_day
    )];
    for row in &house.rows {
        let trip = row.trip.as_ref().map_or_else(
            || "no trip".to_owned(),
            |trip| {
                format!(
                    "trip {}: {} x{} {} to {}",
                    trip.n,
                    trip.good.as_ref().map_or("empty", CanonicalId::as_str),
                    trip.units,
                    trip.from,
                    trip.to
                )
            },
        );
        out.push(format!(
            "  {}: hull {}, {trip}, {} trips{}",
            row.mark,
            row.hull
                .map_or_else(|| "unseeded".to_owned(), |hull| format!("#{hull}")),
            row.trips,
            row.note
                .as_ref()
                .map_or_else(String::new, |note| format!("; {note}"))
        ));
    }
    Ok(out)
}

/// **Every hauling house, one step a hull a fire** (on the port tick, after the pools).
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    for (def, map) in ports_by_venue(ctx.w) {
        let Some(hauler) = def.hauler.as_deref().cloned() else {
            continue;
        };
        let kept = ctx
            .w
            .module_state::<TacsimStore>()
            .and_then(|store| store.hauler(&map.entry).cloned());
        let today = ctx.w.world_ic_now().at.div_euclid(IC_DAY);
        let mut house = kept
            .clone()
            .unwrap_or_else(|| fresh_house(&map.entry, today));
        for hull in &hauler.hulls {
            if !house.rows.iter().any(|row| row.mark == hull.mark) {
                house.rows.push(HaulerRow {
                    mark: hull.mark.clone(),
                    ..HaulerRow::default()
                });
            }
        }
        let open = !house.laid_up && today >= house.opens_day;
        let Some(org_row) = map.orgs.get(&hauler.org).cloned() else {
            continue;
        };
        let root = mush_org::OrgScopeId(org_row.root_scope);
        let Some(house_account) = org_row.account.map(AccountId) else {
            continue;
        };
        for i in 0..house.rows.len() {
            let mut row = house.rows[i].clone();
            // A laid-up house seeds no hull: one laid up before it ever traded stays on paper.
            if house.laid_up && row.hull.is_none() {
                continue;
            }
            let home = hauler
                .hulls
                .iter()
                .find(|hull| hull.mark == row.mark)
                .map_or_else(|| map.entry.clone(), |hull| hull.home.clone());
            let taken: Vec<Taken> = house
                .rows
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .filter_map(|(_, other)| other.trip.as_ref())
                .map(|trip| (trip.good.clone(), trip.from.clone(), trip.to.clone()))
                .collect();
            step(
                ctx,
                &hauler,
                (&home, open, &taken),
                (OrgId(org_row.org), root, house_account),
                &mut row,
            );
            house.rows[i] = row;
        }
        if kept.as_ref() != Some(&house) {
            crate::store::put_hauler(ctx, &house);
        }
    }
}

/// **Whether the helm's plan has ended**: `Some(false)` finished, `Some(true)` held, `None` still
/// running. A held plan with a step still in flight has not ended (48e services:G20): its
/// landing step is outstanding, and a plan written over it would orphan that leg.
fn plan_end(doc: &Orders) -> Option<bool> {
    if doc.inflight.is_some() {
        return None;
    }
    if doc.state == crate::port::orders::PlanState::Held {
        return Some(true);
    }
    doc.cursor.is_none().then_some(false)
}

/// **Why the house no longer has `hull` to sail**, if it does not (48e services:G18, G19): it is
/// gone, wrecked, or titled to anyone but the house.
fn lost_why(w: &World, hull: ObjId, org: OrgId) -> Option<&'static str> {
    if w.get(hull)
        .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING))
    {
        return Some("hull lost: gone");
    }
    if crate::services::escort::wrecked(w, hull) {
        return Some("hull lost: wrecked");
    }
    if crate::port::house_sweep::title_of(w, hull) != Some(PrincipalId::Organization(org)) {
        return Some("hull lost: its title is not the house's");
    }
    None
}

/// **The house loses a row's hull**: the trip dropped, every credit in the purse sent home, the
/// hull let go, and the row marked lost for staff.
fn lose(
    ctx: &mut Ctx<'_>,
    hauler: &HaulerDef,
    row: &mut HaulerRow,
    (purse, house_account): (AccountId, AccountId),
    why: &str,
) {
    let all = available(ctx.w, purse);
    if !pay(
        ctx,
        format!("{}/lost", row.mark),
        (purse, house_account),
        all,
        format!(
            "{} hauler {} lost; its purse goes home",
            hauler.org, row.mark
        ),
    ) {
        // The purse did not move: try again at the next fire.
        return;
    }
    tracing::warn!(mark = %row.mark, why, "tacsim: a hauling house lost a hull");
    row.trip = None;
    row.hull = None;
    row.lost = true;
    row.note = Some(why.to_owned());
}

/// **One hull's step.**
#[allow(clippy::too_many_lines)] // one hull's life, in order
fn step(
    ctx: &mut Ctx<'_>,
    hauler: &HaulerDef,
    (home, open, taken): (&CanonicalId, bool, &[Taken]),
    (org, root, house_account): (OrgId, mush_org::OrgScopeId, AccountId),
    row: &mut HaulerRow,
) {
    // A lost hull's row sails nothing again (48e services:G18, G19).
    if row.lost {
        return;
    }
    // 1. The hull: seeded for the house, then found by its mark once laid up.
    let Some(hull) = row.hull.map(ObjId) else {
        if let Some(found) = crate::port::owned::rows(ctx.w)
            .find(|owned| owned.mark == row.mark)
            .map(|owned| owned.hull)
        {
            row.hull = Some(found.0);
            return;
        }
        let seeded = crate::port::commission::commissions(ctx.w)
            .iter()
            .any(|record| record.mark == row.mark);
        if !seeded
            && let Err(error) = crate::port::commission::seed(
                ctx,
                home,
                &hauler.design,
                &row.mark,
                PrincipalId::Organization(org),
            )
        {
            tracing::warn!(mark = %row.mark, %error, "tacsim: a hauler hull could not be seeded");
        }
        return;
    };
    // 2. The helm and its purse.
    let Some(helm) = row.helm.map(ObjId) else {
        let Some(host) = ports_by_venue(ctx.w)
            .into_iter()
            .find(|(_, map)| &map.entry == home)
            .and_then(|(_, map)| map.presentation)
            .map(ObjId)
        else {
            return;
        };
        match crate::port::site::dark_thing(
            ctx,
            &hauler.org,
            &format!("{} helm {}", hauler.org, row.mark),
            host,
        ) {
            Ok(thing) => row.helm = Some(thing),
            Err(error) => tracing::warn!(%error, "tacsim: a hauler's helm could not be made"),
        }
        return;
    };
    // The helm acts for the house, and may take into its hold (the recovery house's dispatcher's
    // grants, `crate::port::house`).
    if row.grant.is_none() {
        match grant_helm(ctx, org, root, helm) {
            Ok(grant) => row.grant = Some(grant),
            Err(error) => tracing::warn!(%error, "tacsim: a hauler's helm could not be granted"),
        }
        return;
    }
    let Some(purse) = row.account.map(AccountId) else {
        match crate::port::pools::purse_of(ctx, helm, "hauler purse") {
            Ok(account) => row.account = Some(account.0),
            Err(error) => tracing::warn!(%error, "tacsim: a hauler's purse could not be opened"),
        }
        return;
    };
    // The hull is still the house's to sail (48e services:G18, G19): titled to the house, and
    // neither wrecked nor gone. One lost drops its trip and sends the purse home.
    if let Some(why) = lost_why(ctx.w, hull, org) {
        lose(ctx, hauler, row, (purse, house_account), why);
        return;
    }
    // 3. Under way: the house's mind holds her helm.
    let berth = crate::port::ship::berth_of(ctx.w, hull);
    if matches!(berth, Berth::UnderWay) {
        crate::space::tick::cover_house_hull(ctx, hull);
        return;
    }
    let Berth::Docked {
        map: here_map,
        def: here_def,
    } = berth
    else {
        return;
    };
    let (here_map, here_def) = (here_map.clone(), here_def.clone());
    // 4. A plan in hand runs; a finished one closes the trip and sends the purse home.
    let state = match crate::port::orders::orders_state(ctx.w, hull) {
        OrdersState::Present(doc) => Some(doc.clone()),
        OrdersState::Absent => None,
        OrdersState::Unreadable => return,
    };
    // Doc 50 cut OP.4 (Q-50-24): a haul in hand is carried hop by hop, then delivered.
    if row.trip.as_ref().and_then(|trip| trip.haul).is_some() {
        carry(
            ctx,
            hauler,
            (hull, helm),
            (purse, house_account),
            (&here_def, &here_map),
            state.as_ref(),
            row,
        );
        return;
    }
    if let Some(doc) = &state
        && doc.author == helm
    {
        let Some(held) = plan_end(doc) else {
            return;
        };
        if let Some(trip) = row.trip.take() {
            row.trips = row.trips.saturating_add(1);
            if held {
                tracing::warn!(
                    mark = %row.mark, trip = trip.n,
                    "tacsim: a hauler's plan held; the house drops it and looks again tomorrow"
                );
                row.idle_day = Some(ctx.w.world_ic_now().at.div_euclid(IC_DAY));
                row.note = doc
                    .hold
                    .as_ref()
                    .map(|hold| format!("plan held: {hold:?}"))
                    .map(|mut note| {
                        note.truncate(MAX_NOTE);
                        note
                    });
            }
            let float = minor_of(ctx.w, hauler.float_major);
            let surplus = available(ctx.w, purse).saturating_sub(float);
            let _ = pay(
                ctx,
                format!("{}/{}/home", row.mark, trip.n),
                (purse, house_account),
                surplus,
                format!("{} hauler {} pays its trip home", hauler.org, row.mark),
            );
            return;
        }
    }
    let today = ctx.w.world_ic_now().at.div_euclid(IC_DAY);
    if !open {
        return;
    }
    // Doc 50 cut OP.4 (Q-50-24): an arm's haul no player took in its days is the house's to carry.
    if take_haul(
        ctx,
        hauler,
        (hull, helm),
        (purse, house_account),
        &here_def,
        row,
    ) {
        return;
    }
    if row.idle_day == Some(today) {
        return;
    }
    // 5. Anything still aboard is sold here first, once an IC day.
    if row.sold_day != Some(today)
        && let Some(doc) = sell_what_is_aboard(ctx, hauler, hull, helm)
    {
        row.sold_day = Some(today);
        if crate::store::put_orders(ctx, &doc).is_ok() {
            row.trip = Some(Trip {
                n: row.trips.saturating_add(1),
                good: None,
                from: here_def.venue.clone(),
                to: here_def.venue.clone(),
                units: 0,
                ask_major: 0,
                bid_major: 0,
                profit_minor: 0,
                haul: None,
            });
        }
        return;
    }
    // Doc 50 cut OP.8 (Q-50-24): away from home with the arm's work on the board there, she
    // sails home for it before she trades.
    if called_home(
        ctx,
        hauler,
        (hull, helm),
        (purse, house_account),
        (&here_def, &here_map),
        home,
        row,
    ) {
        return;
    }
    // 6. A trip, when one pays.
    let Some(choice) = choose(ctx, hauler, hull, (&here_def, &here_map), taken) else {
        // A look that priced no passage (a port still stocking its asks) is tried again; one
        // that priced them and found nothing paying waits for the next IC day.
        // Doc 50b item 1: a port that sells her no stores never will, so its look waits a day too.
        let priced = here_def
            .routes
            .keys()
            .any(|to| stores_minor(ctx, hull, (&here_def, &here_map), to).is_some())
            || ctx
                .w
                .module_state::<TacsimStore>()
                .and_then(TacsimStore::goods)
                .and_then(|g| crate::port::burn::consumable_good(g, Consumable::Propellant))
                .is_some_and(|propellant| !here_def.authors_ask(&propellant));
        if priced {
            row.idle_day = Some(today);
        }
        row.note = Some(explain(ctx, hauler, hull, (&here_def, &here_map)));
        return;
    };
    row.note = None;
    let n = row.trips.saturating_add(1);
    let cargo = if choice.carried {
        0
    } else {
        i64::try_from(choice.units)
            .unwrap_or(i64::MAX)
            .saturating_mul(minor_of(ctx.w, choice.ask_major))
    };
    let outlay = cargo.saturating_add(choice.stores_minor);
    let want = i64::try_from(
        u64::try_from(outlay)
            .unwrap_or(0)
            .saturating_mul(FUND_PERMILLE)
            / 1_000,
    )
    .unwrap_or(i64::MAX);
    let short = want.saturating_sub(available(ctx.w, purse));
    if short > available(ctx.w, house_account) {
        row.idle_day = Some(today);
        return;
    }
    if !pay(
        ctx,
        format!("{}/{n}/fund", row.mark),
        (house_account, purse),
        short,
        format!("{} funds hauler {} for a trip", hauler.org, row.mark),
    ) {
        return;
    }
    let Some(doc) = plan(ctx, hull, helm, &choice) else {
        return;
    };
    if let Err(error) = crate::store::put_orders(ctx, &doc) {
        tracing::warn!(mark = %row.mark, %error, "tacsim: a hauler's plan did not write");
        return;
    }
    tracing::info!(
        mark = %row.mark,
        good = choice.good.as_ref().map_or("(empty)", CanonicalId::as_str),
        to = %choice.to,
        units = choice.units,
        profit = choice.profit_minor,
        "tacsim: a hauler sets out on a trip"
    );
    row.trip = Some(Trip {
        n,
        good: choice.good,
        from: here_def.venue.clone(),
        to: choice.to,
        units: choice.units,
        ask_major: choice.ask_major,
        bid_major: choice.bid_major,
        profit_minor: choice.profit_minor,
        haul: None,
    });
    row.idle_day = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A trip pays when its profit clears the margin of its outlay** (Q-48-20).
    #[test]
    fn ex10_a_trip_pays_only_past_its_margin() {
        // 40 t of water at 100 cr, sold at 195, with 1,200 cr of stores: 3,800 − 1,200 = 2,600
        // on an outlay of 5,200.
        assert_eq!(pays(40, 10_000, 19_500, 120_000, 200), Some(260_000));
        assert_eq!(pays(40, 10_000, 19_500, 120_000, 500), Some(260_000));
        assert_eq!(pays(40, 10_000, 19_500, 120_000, 600), None, "short of 60%");
        // No gap, or a gap the stores eat: nothing.
        assert_eq!(pays(40, 10_000, 10_000, 0, 0), None);
        assert_eq!(pays(10, 10_000, 11_000, 20_000, 0), None);
        assert_eq!(pays(0, 10_000, 19_500, 0, 0), None);
    }

    /// The record re-encodes with its defaults skipped.
    #[test]
    fn ex10_the_hauler_record_round_trips() {
        let house = HaulerHouse {
            entry: CanonicalId::new("ceres_port").unwrap(),
            opens_day: 1,
            laid_up: true,
            rows: vec![HaulerRow {
                mark: "VCC-01".to_owned(),
                hull: Some(7),
                trip: Some(Trip {
                    n: 1,
                    good: Some(CanonicalId::new("volatiles").unwrap()),
                    from: CanonicalId::new("ceres").unwrap(),
                    to: CanonicalId::new("luna").unwrap(),
                    units: 40,
                    ask_major: 100,
                    bid_major: 195,
                    profit_minor: 260_000,
                    haul: None,
                }),
                ..HaulerRow::default()
            }],
        };
        let wire = serde_json::to_value(&house).unwrap();
        assert!(wire["rows"][0].get("helm").is_none());
        let back: HaulerHouse = serde_json::from_value(wire).unwrap();
        assert_eq!(back, house);
    }

    /// **48e services:G20**: a held plan with a step in flight has not ended; held with none
    /// has, and so has a running plan with nothing left.
    #[test]
    fn a_held_plan_with_a_step_in_flight_has_not_ended() {
        use crate::port::orders::{Inflight, InflightRefs, PlanState};
        let mut doc = Orders {
            schema: crate::port::orders::ORDERS_SCHEMA,
            hull: ObjId(7),
            author: ObjId(8),
            rev: 1,
            state: PlanState::Held,
            hold: None,
            cursor: Some(1),
            next_id: 2,
            inflight: Some(Inflight {
                order: 1,
                attempt: 1,
                key: "burn".to_owned(),
                refs: InflightRefs::default(),
                staged_utc: 0,
                issued_ic: 0,
                restaged: 0,
                want: 1,
                qty: 1,
                bounds: Vec::new(),
                line: None,
            }),
            orders: Vec::new(),
            log: Vec::new(),
        };
        assert_eq!(plan_end(&doc), None, "held, its leg still in flight");
        doc.inflight = None;
        assert_eq!(plan_end(&doc), Some(true), "held, nothing in flight");
        doc.state = PlanState::Running;
        assert_eq!(plan_end(&doc), None, "running, an order left");
        doc.cursor = None;
        assert_eq!(plan_end(&doc), Some(false), "finished");
    }
}
