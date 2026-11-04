//! **The pools** (doc 39 cut IN.10, §4.7; Q-39-7, Q-39-21 as revised by Q-39-34): the content
//! pools are houses with floors, restocked by production.
//!
//! - **The floor** ([`FLOORS`](crate::port::pools::FLOORS)) — the Trust keeps two free Drays at Ceres. A house whose free
//!   hulls and orders standing are under its floor orders one at the yard **when its account
//!   covers the order** ([`price_minor`](crate::port::pools::price_minor): the bill at the port's asks and the parts' catalogue
//!   price, the yard's fee and booking fee, and the ferry's stores), never on the floor alone.
//!   The Trust's capital is the lane's base and its `raises` (a raise of 250,000 in lane v26), then
//!   the buyouts that come back to it.
//! - **The helm** — a `DARK` Thing of the house's (`"{org} ferry helm"`, the Occator Yard
//!   dispatcher's shape) with **its own credit account**: the industry's budget rule makes an
//!   order's account its requester's own, so the house funds the helm with the order's price in
//!   one post and the helm books, buys and pays as itself. The hull is the house's from the
//!   commission on (the order's beneficiary).
//! - **The order** — `crate::port::works::book` at the yard (Stickney), as the helm, for the
//!   house; the bill bought line by line (`buy`): paid from the helm's account to the seller
//!   under a key that names the escrow's count, the seller's standing ask lowered first (IN.5's
//!   house purchase), then moved into the escrow under the order's own ledger. The works arm runs
//!   it; the helm collects it into the commission (`crate::port::works::commission_as`).
//! - **The ferry** — the commissioned hull's stores bought into her hold the same way (the
//!   passage's burn with a quarter over, and its crew-days), under the works ledger keyed by the
//!   hold; then the helm files `+ship/undock` to the house's port as itself, on `Fleet`'s
//!   precedent of a helm without a charter (`crate::port::coop::helm_of` reads [`ferry_hull`](crate::port::pools::ferry_hull)).
//!   When she docks at home she is a free hull of the pool as a **register row**, never a
//!   programme row, and the helm's unspent purse goes back to the house.
//! - **`NO_HULL`'s two texts** read [`next_due`](crate::port::pools::next_due): on order and due in n IC days, else unfunded.
//!
//! **One record a house** (`tacsim:house_pool:{org}`), written whole; every step is re-derived
//! from durable state on the next fire (the escrow's and the hold's counts, the payment keys, the
//! contract, the commission, the berth), so a restart repeats nothing and loses nothing. One step
//! a house a fire. The figures are provisional (the owner's economy rule of 2026-10-24).
//!
//! **What it leaves**: Occator's floors (two free rigs and one escort) ride a ferry Dray-P's bay,
//! which no house keeps at the yard yet; [`FLOORS`](crate::port::pools::FLOORS) carries the Trust's row alone until one does.

use mush_core::{CanonicalId, Ctx, ModuleWrite, ObjId, World};
use mush_economy::{
    Account, AccountId, AccountStatus, Amount, CreditPolicy, EconomicCause, EconomyPlan,
    EconomyStore, IdempotencyKey, InstrumentId, PostOutcome, RequestedLine, VaultId,
};
use mush_industry::{FacilityRecord, WorkOrderState};
use mush_inventory::LotId;
use mush_inventory::record::EndpointRef;
use mush_market::MarketStore;
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use serde_json::json;
use smol_str::SmolStr;

use crate::port::apply::{credit_scale, minor_of};
use crate::port::def::PortDef;
use crate::port::map::PortMap;
use crate::port::works::{Goods, Then, WorksAct};
use crate::store::TacsimStore;

/// The record's key prefix: `tacsim:house_pool:{org}`.
pub const POOL_PREFIX: &str = "tacsim:house_pool:";
/// The house orders' idempotency scope and economic cause.
pub const ORDER_KEY: &str = "tacsim.house_order";
/// The helm's name: `{org}` the house's.
pub const HELM_NAME: &str = "{org} ferry helm";
/// The yard's port: where the houses order (§4.7: "house orders deliver at Stickney").
pub const YARD_PORT: &str = "stickney_port";
/// The ferry's propellant over the planned burn, per mille (provisional).
pub const FERRY_MARGIN_PERMILLE: u64 = 1_250;
/// The ferry's propellant allowance in the price before her hull exists to plan, tonnes
/// (provisional; the dry run buys what the passage takes).
pub const FERRY_ALLOWANCE_T: u64 = 120;
/// The ferry's passage allowance in `NO_HULL`'s due day before she sails, IC days.
pub const PASSAGE_ALLOWANCE_DAYS: i64 = 4;
/// A day of the IC clock.
const DAY: i64 = 86_400;

/// **One house's floor** (§4.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Floor {
    /// The house: its org id on its port's map.
    pub org: &'static str,
    /// The design the pool holds.
    pub design: &'static str,
    /// The yard recipe that builds it.
    pub recipe: &'static str,
    /// How many free the house keeps.
    pub floor: u32,
    /// The pool's port entry (where the ferry sails to).
    pub home: &'static str,
    /// The home port's venue word (`+ship/undock`'s argument).
    pub venue: &'static str,
}

/// **The floors** (Q-39-34; provisional): the Trust's two free Drays at Ceres.
pub const FLOORS: &[Floor] = &[Floor {
    org: "coop_trust",
    design: "tacsim.unit.dray",
    recipe: "dray",
    floor: 2,
    home: "ceres_port",
    venue: "ceres",
}];

// ---- the record -----------------------------------------------------------------------------

/// **Where one house order stands.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The house funded the helm; the order is not booked yet.
    Funded,
    /// Booked; the bill is being bought into the escrow.
    Supplying,
    /// The bill is covered; the yard builds.
    Building,
    /// Collected into the commission; the hull is being laid up.
    Commissioning,
    /// Her stores are being bought into her hold.
    Provisioning,
    /// The ferry is filed or under way.
    Sailing,
}

/// **One house order** (a row of [`HousePool`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HouseOrder {
    /// Its number in the house's sequence (the keys').
    pub n: u32,
    /// The design.
    pub design: CanonicalId,
    /// What the house funded the helm with, minor units.
    pub funded_minor: u64,
    /// The yard order, once booked.
    pub contract: Option<u64>,
    /// When the house ordered, IC seconds.
    pub ordered_ic: i64,
    /// When she is due home, IC seconds (an estimate until she sails, then her arrival).
    pub due_ic: i64,
    /// The hull, once commissioned.
    pub hull: Option<ObjId>,
    /// The stores move into her hold in flight (the works ledger keyed by the hold).
    pub owed: Option<WorksAct>,
    /// Where it stands.
    pub phase: Phase,
}

/// **One house's pool record** (`tacsim:house_pool:{org}`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HousePool {
    /// The house's org id (the key).
    pub org: String,
    /// The helm Thing, once made.
    pub helm: Option<ObjId>,
    /// The helm's own credit account, once opened.
    pub account: Option<u64>,
    /// Orders ever made (the next is `made + 1`).
    pub made: u32,
    /// The open orders, oldest first.
    pub orders: Vec<HouseOrder>,
}

impl HousePool {
    /// Bounds.
    ///
    /// # Errors
    ///
    /// A sentence naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.org.is_empty() || self.orders.len() > 8 {
            return Err("a house pool names its house and at most eight orders".into());
        }
        Ok(())
    }

    /// The document key.
    #[must_use]
    pub fn key(&self) -> String {
        pool_key(&self.org)
    }
}

/// The record key of `org`.
#[must_use]
pub fn pool_key(org: &str) -> String {
    format!("{POOL_PREFIX}{org}")
}

/// The pools in RAM, by house, each loaded at boot and written whole.
pub type PoolBook = std::collections::BTreeMap<String, crate::port::owned::Held<HousePool>>;

fn book(w: &World) -> Option<&PoolBook> {
    w.module_state::<TacsimStore>().map(|store| &store.pools)
}

/// A house's record, if it read.
#[must_use]
pub fn pool<'a>(w: &'a World, org: &str) -> Option<&'a HousePool> {
    book(w)?.get(org)?.present()
}

/// Install a record and queue its document.
pub(crate) fn put(ctx: &mut Ctx<'_>, record: &HousePool) {
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.pools.insert(
            record.org.clone(),
            crate::port::owned::Held::Present(Box::new(record.clone())),
        );
    }
    let value = serde_json::to_value(record).unwrap_or_else(|error| {
        tracing::error!(
            org = %record.org, %error,
            "tacsim: a house pool did not serialize; a minimal document is written so the key \
             stays claimed. Repair the document by hand."
        );
        json!({ "org": record.org })
    });
    crate::store::push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: record.key().into(),
            value,
        },
    );
}

/// **Boot: one pool document** — `true` when `key` is a pool's (read, or held unreadable).
pub(crate) fn load(store: &mut TacsimStore, key: &str, value: &serde_json::Value) -> bool {
    let Some(org) = key.strip_prefix(POOL_PREFIX) else {
        return false;
    };
    let held = match serde_json::from_value::<HousePool>(value.clone())
        .map_err(|error| error.to_string())
        .and_then(|record| record.check().map(|()| record))
        .and_then(|record| {
            if record.org == org {
                Ok(record)
            } else {
                Err("the record disagrees with its own key".to_owned())
            }
        }) {
        Ok(record) => crate::port::owned::Held::Present(Box::new(record)),
        Err(error) => {
            tracing::error!(
                %key, %error,
                "tacsim: a house pool is unreadable; its house orders nothing until staff \
                 repair the document"
            );
            crate::port::owned::Held::Unreadable
        }
    };
    store.pools.insert(org.to_owned(), held);
    true
}

// ---- the readers ----------------------------------------------------------------------------

/// **The hull a house helm's ferry sails** (Q-39-34): `pilot` is a house's helm and one of its
/// orders is provisioning or sailing that hull. `crate::port::coop::helm_of` reads this on
/// `Fleet`'s precedent; `crate::port::voyage::aboard` holds the helm aboard her.
#[must_use]
pub fn ferry_hull(w: &World, pilot: ObjId) -> Option<ObjId> {
    book(w)?
        .values()
        .filter_map(crate::port::owned::Held::present)
        .filter(|record| record.helm == Some(pilot))
        .flat_map(|record| record.orders.iter())
        .find(|order| matches!(order.phase, Phase::Provisioning | Phase::Sailing))
        .and_then(|order| order.hull)
}

/// Whether `obj` is a house's helm.
#[must_use]
pub fn is_helm(w: &World, obj: ObjId) -> bool {
    book(w).is_some_and(|book| {
        book.values()
            .filter_map(crate::port::owned::Held::present)
            .any(|record| record.helm == Some(obj))
    })
}

/// **The next hull a house has on order** (`NO_HULL`'s second text): the yard's port name and
/// the whole IC days until she is due home, rounded up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Due {
    /// The yard's port, by name.
    pub port: String,
    /// IC days until she is due home.
    pub days: i64,
}

/// The house's earliest-due order, if it has one.
#[must_use]
pub fn next_due(w: &World, org: &CanonicalId) -> Option<Due> {
    let order = pool(w, org.as_str())?
        .orders
        .iter()
        .min_by_key(|o| o.due_ic)?;
    let now = w.world_ic_now().at;
    let port = crate::port::market::ports(w)
        .into_iter()
        .find(|(entry, ..)| entry.as_str() == YARD_PORT)
        .map_or_else(
            || YARD_PORT.to_owned(),
            |(_, def, _)| crate::port::voyage::venue_name(w, &def.venue),
        );
    Some(Due {
        port,
        days: order.due_ic.saturating_sub(now).max(0).div_euclid(DAY)
            + i64::from(order.due_ic.saturating_sub(now).rem_euclid(DAY) > 0),
    })
}

// ---- the arm --------------------------------------------------------------------------------

/// **The pools arm** (the port tick, after the works arm): for each floor, one step — the helm
/// made, the next step of the oldest open order, or a new order when the floor is breached and
/// the house can fund it.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    for floor in FLOORS {
        let mut record = match book(ctx.w).and_then(|book| book.get(floor.org)) {
            Some(crate::port::owned::Held::Present(record)) => (**record).clone(),
            Some(crate::port::owned::Held::Unreadable) => continue,
            None => HousePool {
                org: floor.org.to_owned(),
                ..HousePool::default()
            },
        };
        let Some(home) = home_of(ctx.w, floor) else {
            continue;
        };
        match step(ctx, floor, &home, &mut record) {
            Ok(true) => put(ctx, &record),
            Ok(false) => {}
            Err(error) => {
                tracing::debug!(org = floor.org, %error, "tacsim: a house order waits");
            }
        }
    }
}

/// The house's home: its org row on the pool's port map, its account, and the port's office.
struct Home {
    /// The house's principal.
    house: PrincipalId,
    /// The house's account.
    account: AccountId,
    /// Where the helm stands.
    host: ObjId,
}

fn home_of(w: &World, floor: &Floor) -> Option<Home> {
    let store = w.module_state::<TacsimStore>()?;
    let map = store
        .port_maps()
        .find(|map| map.entry.as_str() == floor.home)?;
    let row = map.orgs.get(&CanonicalId::new(floor.org).ok()?)?;
    let host = map.presentation.map(ObjId)?;
    Some(Home {
        house: PrincipalId::Organization(mush_org::OrgId(row.org)),
        account: AccountId(row.account?),
        host,
    })
}

/// One step for one house: `Ok(true)` when the record changed.
fn step(
    ctx: &mut Ctx<'_>,
    floor: &Floor,
    home: &Home,
    record: &mut HousePool,
) -> Result<bool, String> {
    // 1. The helm and its purse.
    let Some(helm) = record.helm else {
        let thing = crate::port::site::dark_thing(
            ctx,
            &CanonicalId::new(floor.org).map_err(|e| e.to_string())?,
            &HELM_NAME.replace("{org}", floor.org),
            home.host,
        )?;
        record.helm = Some(ObjId(thing));
        return Ok(true);
    };
    let Some(account) = record.account.map(AccountId) else {
        record.account = Some(open_purse(ctx, helm)?.0);
        return Ok(true);
    };
    // 2. The oldest open order's next step.
    if let Some(index) = (!record.orders.is_empty()).then_some(0) {
        let mut order = record.orders[index].clone();
        let step = advance(ctx, floor, home, helm, account, &mut order)?;
        match step {
            Advance::Same => {}
            Advance::Changed => {
                record.orders[index] = order;
                return Ok(true);
            }
            Advance::Home => {
                record.orders.remove(index);
                if record.orders.is_empty() {
                    refund(ctx, home, (helm, account), order.n);
                }
                return Ok(true);
            }
        }
        // An order still building leaves the floor check to run beside it.
        if !matches!(order.phase, Phase::Building) {
            return Ok(false);
        }
    }
    // 3. The floor.
    let free = free_count(ctx.w, floor);
    let standing = u32::try_from(record.orders.len()).unwrap_or(u32::MAX);
    if free.saturating_add(standing) >= floor.floor {
        return Ok(false);
    }
    let price = price_minor(ctx, floor)?;
    let available = available_minor(ctx.w, home.account);
    if available < price {
        return Ok(false);
    }
    let n = record.made.saturating_add(1);
    pay(
        ctx,
        &format!("{}/{n}/fund", floor.org),
        (home.account, account),
        price,
        &home.house,
        format!("{} funds its helm for a {}", floor.org, floor.recipe),
    )?;
    let now = ctx.w.world_ic_now().at;
    record.made = n;
    record.orders.push(HouseOrder {
        n,
        design: CanonicalId::new(floor.design).map_err(|e| e.to_string())?,
        funded_minor: price,
        contract: None,
        ordered_ic: now,
        due_ic: now
            .saturating_add(build_days(ctx.w, floor).saturating_add(PASSAGE_ALLOWANCE_DAYS) * DAY),
        hull: None,
        owed: None,
        phase: Phase::Funded,
    });
    tracing::info!(
        org = floor.org,
        n,
        price,
        "tacsim: a house orders a hull for its pool"
    );
    Ok(true)
}

/// What one order's step did.
enum Advance {
    Same,
    Changed,
    Home,
}

#[allow(clippy::too_many_lines)] // one order's phases, in order
fn advance(
    ctx: &mut Ctx<'_>,
    floor: &Floor,
    home: &Home,
    helm: ObjId,
    account: AccountId,
    order: &mut HouseOrder,
) -> Result<Advance, String> {
    match order.phase {
        Phase::Funded => {
            let facility = yard(ctx.w)?;
            let (recipe, pinned) = crate::port::works::find_recipe(ctx.w, &facility, floor.recipe)
                .ok_or("the yard does not build that")?;
            let saved = ctx.actor;
            ctx.actor = helm;
            let booked = crate::port::works::book(
                ctx,
                crate::port::works::Booking {
                    front: crate::port::works::Front::Yard,
                    facility: &facility,
                    recipe: &recipe,
                    pinned: &pinned,
                    quantity: 1,
                    payer: PrincipalId::Object(helm),
                    beneficiary: home.house.clone(),
                    account,
                },
            );
            ctx.actor = saved;
            let contract = booked?;
            order.contract = Some(contract.id);
            order.due_ic = ctx.w.world_ic_now().at.saturating_add(
                i64::try_from(contract.work_units.div_ceil(24))
                    .unwrap_or(i64::MAX)
                    .saturating_add(PASSAGE_ALLOWANCE_DAYS)
                    .saturating_mul(DAY),
            );
            order.phase = Phase::Supplying;
            Ok(Advance::Changed)
        }
        Phase::Supplying => {
            let id = order.contract.ok_or("no contract")?;
            let c = crate::port::works::open_contract(ctx.w, id).ok_or("the order closed")?;
            if c.owed.is_some() || c.order.is_none() {
                return Ok(Advance::Same);
            }
            let mut lots_held = crate::port::works::lots_in(ctx, c.escrow());
            for line in &c.bill {
                let part = crate::port_tick::is_part(&line.good);
                let held = if part {
                    u64::try_from(lots_held.iter().filter(|(_, g, _)| *g == line.good).count())
                        .unwrap_or(u64::MAX)
                } else {
                    crate::port::works::escrow_units(ctx, &c, &line.good)
                };
                if held >= line.units {
                    continue;
                }
                let want = if part { 1 } else { line.units - held };
                buy(
                    ctx,
                    helm,
                    account,
                    Sink::Escrow(id),
                    (&line.good, want, held),
                )?;
                lots_held.clear();
                return Ok(Advance::Same);
            }
            order.phase = Phase::Building;
            Ok(Advance::Changed)
        }
        Phase::Building => {
            let id = order.contract.ok_or("no contract")?;
            let Some(c) = crate::port::works::open_contract(ctx.w, id) else {
                return Err("the order closed before its collection".to_owned());
            };
            match crate::port::works::order_state(ctx, &c) {
                Some(WorkOrderState::Completed { .. }) if c.owed.is_none() => {
                    crate::port::works::commission_as(ctx, id, helm)?;
                    order.phase = Phase::Commissioning;
                    Ok(Advance::Changed)
                }
                _ => Ok(Advance::Same),
            }
        }
        Phase::Commissioning => {
            let id = order.contract.ok_or("no contract")?;
            let record = crate::port::commission::commission(ctx.w, id).ok_or("no commission")?;
            if record.step != crate::port::commission::Step::Done {
                return Ok(Advance::Same);
            }
            order.hull = record.object();
            order.phase = Phase::Provisioning;
            Ok(Advance::Changed)
        }
        Phase::Provisioning => {
            let hull = order.hull.ok_or("no hull")?;
            if let Some(owed) = order.owed.clone() {
                if crate::port::works::landed(ctx, owed.to) >= owed.act {
                    order.owed = None;
                    return Ok(Advance::Changed);
                }
                if owed.tries >= crate::port::works::MAX_TRIES {
                    order.owed = None;
                    return Ok(Advance::Changed);
                }
                let mut again = owed;
                again.tries = again.tries.saturating_add(1);
                crate::port::works::submit_move(ctx, again.to, &again);
                order.owed = Some(again);
                return Ok(Advance::Changed);
            }
            let view = crate::port::ship::hull_view(ctx.w, hull).ok_or("no hull view")?;
            let hold = crate::port::ship::hold_vault(ctx.w, &view).ok_or("no hold")?;
            let goods = ctx
                .w
                .module_state::<TacsimStore>()
                .and_then(TacsimStore::goods)
                .ok_or("no goods table")?;
            let propellant = crate::port::burn::consumable_good(
                goods,
                crate::definitions::goods::Consumable::Propellant,
            )
            .ok_or("no propellant")?;
            let provisions = crate::port::burn::consumable_good(
                goods,
                crate::definitions::goods::Consumable::Provisions,
            )
            .ok_or("no provisions")?;
            let (burn, days) = crate::port::voyage::passage_needs(ctx, hull, floor.venue, 1)?;
            let wants = [
                (
                    propellant,
                    burn.saturating_mul(FERRY_MARGIN_PERMILLE)
                        .div_ceil(1_000)
                        .max(1),
                ),
                (provisions, u64::from(days).saturating_add(1)),
            ];
            for (good, want) in &wants {
                let held = crate::port_tick::read_pool(ctx, EndpointRef(hold), good).unwrap_or(0);
                if held >= *want {
                    continue;
                }
                if let Some(owed) = buy(
                    ctx,
                    helm,
                    account,
                    Sink::Hold(hold),
                    (good, want - held, held),
                )? {
                    order.owed = Some(owed);
                    return Ok(Advance::Changed);
                }
                return Ok(Advance::Same);
            }
            // The stores are aboard: the helm files the passage home.
            let saved = ctx.actor;
            ctx.actor = helm;
            let filed = crate::port::voyage::undock_as(ctx, floor.venue, false);
            ctx.actor = saved;
            filed.map_err(|e| e.to_string())?;
            if let Some(pending) =
                crate::port::voyage::pending_of(ctx.w, hull).map(|p| p.plan.clone())
            {
                order.due_ic = pending.depart_ic.saturating_add(pending.total_s);
            }
            order.phase = Phase::Sailing;
            Ok(Advance::Changed)
        }
        Phase::Sailing => {
            let hull = order.hull.ok_or("no hull")?;
            match crate::port::ship::berth_of(ctx.w, hull) {
                crate::port::ship::Berth::Docked { map, .. }
                    if map.entry.as_str() == floor.home =>
                {
                    tracing::info!(
                        org = floor.org,
                        hull = hull.0,
                        "tacsim: a house ferry docks home; she joins the pool"
                    );
                    Ok(Advance::Home)
                }
                _ => Ok(Advance::Same),
            }
        }
    }
}

/// Where a purchase lands.
#[derive(Clone, Copy, Debug)]
enum Sink {
    /// An order's escrow, under the order's ledger.
    Escrow(u64),
    /// A hull's hold, under the works ledger keyed by the hold.
    Hold(VaultId),
}

/// **One purchase** (IN.5's house purchase, for a house order): the seller of `good` at the yard's
/// port; paid once under a key naming the sink, the good and the count it holds (`held`), so a
/// restart finds the payment and moves the goods without paying twice; the seller's standing ask
/// lowered first; then the move as the helm. A part is one store lot at its catalogue price.
/// Answers the hold move owed, when it is one.
fn buy(
    ctx: &mut Ctx<'_>,
    helm: ObjId,
    account: AccountId,
    sink: Sink,
    (good, units, held): (&CanonicalId, u64, u64),
) -> Result<Option<WorksAct>, String> {
    let (map, def) = yard_port(ctx.w)?;
    let part = crate::port_tick::is_part(good);
    let (vendor, price_minor) = seller(ctx.w, &def, good, part)?;
    let row = map
        .vendors
        .get(&vendor.org)
        .cloned()
        .ok_or("the seller is not laid up")?;
    let from = EndpointRef(VaultId(row.vault));
    let lot = if part {
        let claimed = crate::port::bay_work::claimed_store_lots(ctx.w);
        let lot = crate::port::stock::store_lots(ctx, &row, good, &claimed)
            .into_iter()
            .find(|lot| lot.listing.is_none())
            .ok_or("the yard's stores hold no such part")?;
        Some(lot.lot)
    } else {
        let pool = crate::port_tick::read_pool(ctx, from, good).unwrap_or(0);
        if pool < units {
            return Err(format!("the seller holds {pool} {good}, not {units}"));
        }
        None
    };
    let sink_id = match sink {
        Sink::Escrow(id) => id,
        Sink::Hold(vault) => vault.0,
    };
    let payee = map
        .orgs
        .get(&vendor.org)
        .and_then(|org| org.account)
        .map(AccountId)
        .ok_or("the seller has no account")?;
    let cost = price_minor.saturating_mul(units);
    let key_text = format!("{sink_id}/{good}/{held}");
    let paid =
        mush_economy::resolve_idempotency(ctx, &IdempotencyKey::new(ORDER_KEY, key_text.clone()));
    match paid {
        Ok(Some(_)) => {}
        Ok(None) => {
            pay(
                ctx,
                &key_text,
                (account, payee),
                cost,
                &PrincipalId::Object(helm),
                format!("a house order buys {units} {good}"),
            )?;
            return Ok(None);
        }
        Err(_) => return Ok(None),
    }
    // The seller's standing ask comes down by the units first (IN.5's rule).
    if !part {
        let reading = ctx
            .w
            .module_state::<MarketStore>()
            .map(|market| crate::port::stock::ask_reading(market, row.vault, good))
            .unwrap_or_default();
        if reading.in_flight {
            return Ok(None);
        }
        if let Some((listing, listed)) = reading.listing {
            let pool = crate::port_tick::read_pool(ctx, from, good).unwrap_or(0);
            let mills = ctx
                .w
                .module_state::<crate::port::families::PortHandles>()
                .map(|handles| handles.families.mills.clone())
                .ok_or("no mills family")?;
            let unlisted = crate::port::mill::unlisted(ctx, &mills, &map, &vendor, &row, good)
                .map_err(|e| format!("{e:?}"))?;
            let excess = pool
                .saturating_sub(unlisted)
                .saturating_sub(listed.saturating_add(reading.undrawn));
            if excess < units {
                let clerk = map
                    .clerk
                    .as_ref()
                    .map(|c| ObjId(c.thing))
                    .ok_or("no clerk")?;
                let spec = mush_market::RestockSpec {
                    listing,
                    good: good.clone(),
                    units: units - excess,
                };
                let saved = ctx.actor;
                ctx.actor = clerk;
                let shrunk = mush_market::shrink_ask(ctx, spec);
                ctx.actor = saved;
                shrunk.map_err(|e| format!("{e:?}"))?;
                return Ok(None);
            }
        }
    }
    let (goods, lots) = match lot {
        Some(lot) => (Vec::new(), vec![(lot, good.clone(), 1)]),
        None => (
            vec![Goods {
                good: good.clone(),
                units,
            }],
            Vec::new(),
        ),
    };
    match sink {
        Sink::Escrow(id) => {
            crate::port::works::supply_from(ctx, id, helm, from, (goods, lots))?;
            Ok(None)
        }
        Sink::Hold(vault) => {
            let owed = WorksAct {
                act: crate::port::works::landed(ctx, vault.0)
                    .checked_add(1)
                    .ok_or("the hold's ledger is full")?,
                by: helm,
                from: row.vault,
                to: vault.0,
                goods,
                lots: lots
                    .into_iter()
                    .map(|(lot, ..)| lot)
                    .collect::<Vec<LotId>>(),
                then: Then::Supplied,
                tries: 1,
            };
            owed.check()?;
            crate::port::works::submit_move(ctx, vault.0, &owed);
            Ok(Some(owed))
        }
    }
}

/// **The moves the helm `actor` owes into a hold** for the works offer: each house order's stores
/// move in flight, keyed by the hold's vault.
pub(crate) fn owed_moves(w: &World, actor: ObjId) -> Vec<(u64, WorksAct)> {
    book(w)
        .into_iter()
        .flat_map(|book| book.values())
        .filter_map(crate::port::owned::Held::present)
        .flat_map(|record| record.orders.iter())
        .filter_map(|order| order.owed.clone())
        .filter(|owed| owed.by == actor)
        .map(|owed| (owed.to, owed))
        .collect()
}

/// The seller of `good` at the yard's port and its unit price, minor: a bulk good's standing ask,
/// a part's catalogue price from the vendor that stocks it.
fn seller(
    w: &World,
    def: &PortDef,
    good: &CanonicalId,
    part: bool,
) -> Result<(crate::port::def::VendorDef, u64), String> {
    let scale = credit_scale(w);
    if part {
        // The yard's operator's stores (IN.7: no part is seeded; what is made, scrapped or sold
        // into them is drawable).
        let operator = def
            .yard
            .as_ref()
            .map(|yard| yard.operator.clone())
            .ok_or("the yard's port authors no yard")?;
        let vendor = def
            .vendors
            .iter()
            .find(|v| v.org == operator)
            .ok_or("the yard's operator keeps no stores")?;
        let major = w
            .module_state::<TacsimStore>()
            .and_then(TacsimStore::goods)
            .and_then(|goods| goods.good(good).map(|g| g.reference_major))
            .ok_or("no catalogue price")?;
        let minor = minor_of(major, scale)
            .and_then(|minor| u64::try_from(minor).ok())
            .ok_or("a price overflows")?;
        return Ok((vendor.clone(), minor));
    }
    def.vendors
        .iter()
        .find_map(|v| {
            v.asks
                .iter()
                .find(|ask| &ask.good == good)
                .map(|ask| (v.clone(), ask.price_major))
        })
        .and_then(|(v, major)| {
            minor_of(major, scale)
                .and_then(|minor| u64::try_from(minor).ok())
                .map(|minor| (v, minor))
        })
        .ok_or_else(|| format!("no ask for {good} at the yard"))
}

/// The yard's port def and map.
fn yard_port(w: &World) -> Result<(PortMap, PortDef), String> {
    let (_, def, map) = crate::port::market::ports(w)
        .into_iter()
        .find(|(entry, ..)| entry.as_str() == YARD_PORT)
        .ok_or("the yard's port is not laid up")?;
    let map = map.ok_or("the yard's port is not laid up")?;
    Ok((map.clone(), def.clone()))
}

/// The yard facility at the yard's port.
fn yard(w: &World) -> Result<FacilityRecord, String> {
    let (_, def) = yard_port(w)?;
    let entry = CanonicalId::new(YARD_PORT).map_err(|e| e.to_string())?;
    // The lane's yard row is a room of the port, which is the facility itself (IN.9).
    let room = crate::port::plant::block_room(w, &entry, &def, "yard")?
        .ok_or("the yard's port authors no yard")?;
    w.module_state::<mush_industry::IndustryStore>()
        .and_then(|store| store.facility(room))
        .cloned()
        .ok_or_else(|| "the yard is not open".to_owned())
}

/// The days the floor's recipe takes at the yard, or two (§4.7's fewest).
fn build_days(w: &World, floor: &Floor) -> i64 {
    yard(w)
        .ok()
        .and_then(|facility| crate::port::works::find_recipe(w, &facility, floor.recipe))
        .map_or(2, |(_, pinned)| {
            pinned
                .work_units()
                .div_ceil(crate::port::bill::WORK_UNITS_A_DAY)
        })
        .try_into()
        .unwrap_or(i64::MAX)
}

/// **What one order costs the house** (Q-39-34: "orders when its account covers the bill"),
/// minor: the bill's bulk at the yard port's asks, its parts at their catalogue prices, the
/// yard's fee and booking fee, and the ferry's stores at an allowance.
///
/// # Errors
///
/// The yard, the recipe or a price cannot be read.
pub fn price_minor(ctx: &Ctx<'_>, floor: &Floor) -> Result<u64, String> {
    let w: &World = ctx.w;
    let facility = yard(w)?;
    let (_, def) = yard_port(w)?;
    let (_, pinned) = crate::port::works::find_recipe(w, &facility, floor.recipe)
        .ok_or("the yard does not build that")?;
    let mut total: u64 = 0;
    for line in crate::port::works::bill_with_parts(w, &pinned, 1) {
        let part = crate::port_tick::is_part(&line.good);
        let (_, unit) = seller(w, &def, &line.good, part)?;
        total = total.saturating_add(unit.saturating_mul(line.units));
    }
    let units = pinned.work_units();
    total = total
        .saturating_add(facility.fee_minor(units).unwrap_or(0))
        .saturating_add(facility.booking_fee_minor().unwrap_or(0));
    let goods = w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
        .ok_or("no goods table")?;
    for (kind, units) in [
        (
            crate::definitions::goods::Consumable::Propellant,
            FERRY_ALLOWANCE_T,
        ),
        (
            crate::definitions::goods::Consumable::Provisions,
            u64::try_from(PASSAGE_ALLOWANCE_DAYS)
                .unwrap_or(0)
                .saturating_add(1),
        ),
    ] {
        let good = crate::port::burn::consumable_good(goods, kind).ok_or("no consumable")?;
        let (_, unit) = seller(w, &def, &good, false)?;
        total = total.saturating_add(unit.saturating_mul(units));
    }
    Ok(total)
}

/// Free hulls of the floor's design in the house's pool.
fn free_count(w: &World, floor: &Floor) -> u32 {
    crate::port::coop::programme_ports(w)
        .into_iter()
        .filter(|port| {
            port.programme.org.as_str() == floor.org && port.programme.unit.as_str() == floor.design
        })
        .map(|port| crate::port::coop::free_hulls(w, &port).len())
        .sum::<usize>()
        .try_into()
        .unwrap_or(u32::MAX)
}

fn available_minor(w: &World, account: AccountId) -> u64 {
    w.module_state::<EconomyStore>()
        .and_then(|economy| economy.snapshot().account(account).cloned())
        .and_then(|account| account.available())
        .map_or(0, |amount| u64::try_from(amount.minor()).unwrap_or(0))
}

/// One post of `minor` from `debit` to `credit` under `tacsim.house_order/{key}`.
fn pay(
    ctx: &mut Ctx<'_>,
    key: &str,
    (debit, credit): (AccountId, AccountId),
    minor: u64,
    initiator: &PrincipalId,
    description: String,
) -> Result<(), String> {
    let signed = i64::try_from(minor).map_err(|_| "an amount overflows")?;
    let (Some(out), into) = (
        Amount::from_minor(signed).checked_neg(),
        Amount::from_minor(signed),
    ) else {
        return Err("an amount overflows".to_owned());
    };
    let plan = EconomyPlan {
        description,
        visibility: mush_economy::EconomicVisibility::Public,
        ..EconomyPlan::new(
            IdempotencyKey::new(ORDER_KEY, key.to_owned()),
            initiator.clone(),
            EconomicCause::Internal {
                kind: SmolStr::new_static(ORDER_KEY),
                id: 0,
            },
            vec![
                RequestedLine {
                    account: debit,
                    delta: out,
                    memo: SmolStr::new_static(ORDER_KEY),
                },
                RequestedLine {
                    account: credit,
                    delta: into,
                    memo: SmolStr::new_static(ORDER_KEY),
                },
            ],
        )
    };
    match mush_economy::post(ctx, &plan) {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => Ok(()),
        other => Err(format!("the post did not land: {other:?}")),
    }
}

/// The helm's unspent purse back to the house once its last order is home.
fn refund(ctx: &mut Ctx<'_>, home: &Home, (helm, purse): (ObjId, AccountId), n: u32) {
    let left = available_minor(ctx.w, purse);
    if left == 0 {
        return;
    }
    if let Err(error) = pay(
        ctx,
        &format!("{purse}/{n}/refund", purse = purse.0),
        (purse, home.account),
        left,
        &PrincipalId::Object(helm),
        "a house helm's unspent purse goes home".to_owned(),
    ) {
        tracing::warn!(%error, "tacsim: a house helm's purse did not go home");
    }
}

/// Open the helm's own credit account.
fn open_purse(ctx: &mut Ctx<'_>, helm: ObjId) -> Result<AccountId, String> {
    let id = ctx
        .w
        .module_state_mut::<EconomyStore>()
        .and_then(EconomyStore::alloc_account)
        .ok_or("the account allocator is spent")?;
    let instrument =
        InstrumentId::new(crate::economy::CREDIT).ok_or("the credit instrument is unknown")?;
    mush_economy::open_account(
        ctx,
        Account {
            id,
            owner: PrincipalId::Object(helm),
            instrument,
            scope: None,
            name: SmolStr::new("house helm purse"),
            status: AccountStatus::Active,
            posted_balance: Amount::ZERO,
            held_amount: Amount::ZERO,
            last_hold_seq: 0,
            credit: CreditPolicy::forbid(),
            last_journal_seq: 0,
            revision: 1,
            approval: None,
        },
    );
    Ok(id)
}
