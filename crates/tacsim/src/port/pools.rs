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
//! - **A part the stores lack** (the owner's ruling of 2026-11-05: "letting the house order take
//!   longer is a more real test than adding a house restock") — the helm orders it at the yard
//!   port's works for the house (`HouseOrder::part`), buys that order's bill the same way, and
//!   once the works has made and traced it, moves it from that escrow into the hull's
//!   (`crate::port::works::collect_into`). The hull waits the part's days.
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
//! **A House's own order** (doc 48 unit PL.5; Q-48-28 as corrected by doc 48b W-48-4): a House's
//! officer orders a hull at the yard for it (`+yard/fund <recipe>=<org>`, `request`); the House
//! keeps no floor and no hold. Its record (`tacsim:house_pool:org.{id}`,
//! [`HousePool::house`](crate::port::pools::HousePool::house)) runs
//! the same arm: its **yard agent** (`"{org} yard agent"`,
//! [`AGENT_NAME`](crate::port::pools::AGENT_NAME)) is made at the yard's
//! port and funded from the House's treasury (its credit account on its root scope), books the
//! order for the House, buys the bill on the board, and collects the commission. She is handed
//! over docked at the yard, titled to the House: no ferry, and the agent's unspent purse goes back
//! to the treasury.
//!
//! **Any buyer** (PL.5): the board's seller and price (`board`) are the listing's, whoever buys.
//! `+works/buy` and `+yard/buy` (`buy_into`) buy a player's own order's bill through the same
//! seam, straight into its escrow with no hold.
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
/// **Doc 48 unit PL.5**: a House's yard agent's name, `{org}` the House's name.
pub const AGENT_NAME: &str = "{org} yard agent";
/// **Doc 48 unit PL.5**: the venue a House's yard agent buys at (the yard's port's).
pub const YARD_VENUE: &str = "stickney";
/// **Doc 48 unit PL.5**: a House record's key word, `org.{id}`.
pub const HOUSE_KEY: &str = "org.";
/// **Doc 48 unit PL.5**: the real seconds a private ask to a player stands before it lapses
/// (provisional): long enough to buy again, short enough that another buyer of the same pool
/// waits minutes, not days (the market keeps one live draw listing a pool).
pub const PLAYER_ASK_TTL_SECS: i64 = 600;
/// **Doc 48 unit PL.5**: the most hulls a House has on order at once (the record's bound).
pub const MAX_HOUSE_ORDERS: usize = 8;

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
    /// **Doc 48 cut EX.3**: the venue the house's helm buys its bill and stores at (the yard's,
    /// [`YARD_PORT`]'s): the one venue the gate admits the helm at.
    pub buys_at: &'static str,
}

/// **The floors** (Q-39-34; provisional): the Trust's two free Drays at Ceres.
pub const FLOORS: &[Floor] = &[Floor {
    org: "coop_trust",
    design: "tacsim.unit.dray",
    recipe: "dray",
    floor: 2,
    home: "ceres_port",
    venue: "ceres",
    buys_at: "stickney",
}];

// ---- the record -----------------------------------------------------------------------------

/// **Where one house order stands.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// **Doc 48 unit PL.5**: a House's officer ordered it; the House funds its yard agent once
    /// the agent and its purse stand.
    Requested,
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
    /// **The works order for a part the yard's stores did not hold** (the owner's IN.11 ruling,
    /// 2026-11-05), once booked; cleared when it closes (its part is then in the hull order's
    /// escrow). Omitted while `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<u64>,
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
    /// **Doc 48 unit PL.5**: a House order's yard recipe (a floor's order builds its floor's).
    /// Omitted while `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<CanonicalId>,
    /// **Doc 48 unit PL.5**: the officer who ordered it for the House. Omitted while `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<ObjId>,
    /// **Doc 48e, production G10**: what the house's account has topped the purse up by for
    /// this order since its funding, minor units, because the board's live asks rose past the
    /// prices it was funded at (`top_up`). Omitted while 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub topped_minor: u64,
    /// **Doc 48e, production G11**: a yard order of this one that ended before its hull (a
    /// cancel, a failure, a draft that never bound) with its bill in its escrow. The order is
    /// booked again and this escrow is carried into the new one before anything more is bought.
    /// Omitted while `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carry: Option<u64>,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's `skip_serializing_if` passes a reference
const fn is_zero(n: &u64) -> bool {
    *n == 0
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
    /// **Doc 48 unit PL.5**: the House this record buys for, by its org id, when it is a House's
    /// own (its officers order, with no floor and no ferry; `org` is then `org.{id}`). `None` for
    /// a floor's record. Omitted while `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub house: Option<u64>,
}

impl HousePool {
    /// Bounds.
    ///
    /// # Errors
    ///
    /// A sentence naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.org.is_empty() || self.orders.len() > MAX_HOUSE_ORDERS {
            return Err("a house pool names its house and at most eight orders".into());
        }
        if let Some(house) = self.house {
            if self.org != house_key(house) {
                return Err("a House's record is keyed by its org id".into());
            }
            if self.orders.iter().any(|order| order.recipe.is_none()) {
                return Err("a House's order names its recipe".into());
            }
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

/// **Doc 48 unit PL.5**: a House record's `org` word, `org.{id}`.
#[must_use]
pub fn house_key(org: u64) -> String {
    format!("{HOUSE_KEY}{org}")
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

/// **The venue a house's helm buys at** (doc 48 cut EX.3): its floor's `buys_at`, `None` for
/// anyone who is not a registered helm.
#[must_use]
pub fn helm_buys_at(w: &World, obj: ObjId) -> Option<&'static str> {
    let book = book(w)?;
    let record = book
        .values()
        .filter_map(crate::port::owned::Held::present)
        .find(|record| record.helm == Some(obj))?;
    // Doc 48 unit PL.5: a House's yard agent buys at the yard's venue.
    if record.house.is_some() {
        return Some(YARD_VENUE);
    }
    let org = record.org.clone();
    FLOORS
        .iter()
        .find(|floor| floor.org == org.as_str())
        .map(|floor| floor.buys_at)
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
    // Doc 48 unit PL.5: each House's own orders, one step a House a fire.
    let houses: Vec<HousePool> = book(ctx.w)
        .into_iter()
        .flat_map(|book| book.values())
        .filter_map(crate::port::owned::Held::present)
        .filter(|record| record.house.is_some() && !record.orders.is_empty())
        .cloned()
        .collect();
    for mut record in houses {
        let Some(home) = house_home(ctx.w, &record) else {
            continue;
        };
        match house_step(ctx, &home, &mut record) {
            Ok(true) => put(ctx, &record),
            Ok(false) => {}
            Err(error) => {
                tracing::debug!(org = %record.org, %error, "tacsim: a House's order waits");
            }
        }
    }
}

/// **What one order builds and where she goes** (doc 48 unit PL.5): a floor's, or a House's.
#[derive(Clone, Copy, Debug)]
struct Spec<'a> {
    /// The record's word (the logs').
    org: &'a str,
    /// The yard recipe.
    recipe: &'a str,
    /// The ferry home, a floor's: its home port's entry and venue word. `None` for a House's
    /// order: she is handed over docked at the yard.
    ferry: Option<(&'a str, &'a str)>,
}

impl Floor {
    const fn spec(&self) -> Spec<'static> {
        Spec {
            org: self.org,
            recipe: self.recipe,
            ferry: Some((self.home, self.venue)),
        }
    }
}

/// **A House's treasury** (doc 48 unit PL.5): its Active credit account on its root scope, the
/// lowest id (the procurement tick's treasury rule).
#[must_use]
pub fn treasury_of(w: &World, org: mush_org::OrgId) -> Option<AccountId> {
    let root = w
        .module_state::<mush_org::OrgStore>()?
        .state()
        .org(org)?
        .root_scope;
    let instrument = InstrumentId::new(crate::economy::CREDIT)?;
    w.module_state::<EconomyStore>()?
        .snapshot()
        .accounts_of(&PrincipalId::Organization(org))
        .find(|account| {
            account.instrument == instrument
                && account.scope == Some(root)
                && account.status == AccountStatus::Active
        })
        .map(|account| account.id)
}

/// A House record's home: the House, its treasury, and the yard's port's presentation room,
/// where its agent stands.
fn house_home(w: &World, record: &HousePool) -> Option<Home> {
    let org = mush_org::OrgId(record.house?);
    let account = treasury_of(w, org)?;
    let (map, _) = yard_port(w).ok()?;
    Some(Home {
        house: PrincipalId::Organization(org),
        account,
        host: map.presentation.map(ObjId)?,
    })
}

/// **One step for one House's record** (doc 48 unit PL.5): the agent made, its purse opened, the
/// oldest order funded from the treasury, or that order's next step. `Ok(true)` when the record
/// changed.
fn house_step(ctx: &mut Ctx<'_>, home: &Home, record: &mut HousePool) -> Result<bool, String> {
    let Some(helm) = record.helm else {
        let PrincipalId::Organization(org) = home.house else {
            return Err("a House record's home is an organization".to_owned());
        };
        let name = ctx
            .w
            .module_state::<mush_org::OrgStore>()
            .and_then(|store| store.state().org(org).map(|o| o.name.to_string()))
            .ok_or("the House is not on the org store")?;
        let thing = crate::port::site::dark_thing(
            ctx,
            &CanonicalId::new(&record.org).map_err(|e| e.to_string())?,
            &AGENT_NAME.replace("{org}", &name),
            home.host,
        )?;
        record.helm = Some(ObjId(thing));
        return Ok(true);
    };
    let Some(account) = record.account.map(AccountId) else {
        record.account = Some(open_named_purse(ctx, helm, "house yard agent purse")?.0);
        return Ok(true);
    };
    let mut order = record.orders[0].clone();
    let recipe = order
        .recipe
        .clone()
        .ok_or("a House's order names its recipe")?;
    if order.phase == Phase::Requested {
        // The treasury is read again: the officer's check was at the order.
        if available_minor(ctx.w, home.account) < order.funded_minor {
            return Err("the House's treasury no longer covers its order".to_owned());
        }
        pay(
            ctx,
            &format!("{}/{}/fund", record.org, order.n),
            (home.account, account),
            order.funded_minor,
            &home.house,
            format!("{} funds its yard agent for a {recipe}", record.org),
        )?;
        order.phase = Phase::Funded;
        record.orders[0] = order;
        return Ok(true);
    }
    let spec = Spec {
        org: &record.org,
        recipe: recipe.as_str(),
        ferry: None,
    };
    match advance(ctx, &spec, home, helm, account, &mut order)? {
        Advance::Same => Ok(false),
        Advance::Changed => {
            record.orders[0] = order;
            Ok(true)
        }
        Advance::Home => {
            record.orders.remove(0);
            if record.orders.is_empty() {
                refund(ctx, home, (helm, account), order.n);
            }
            Ok(true)
        }
    }
}

/// **What a House's order answers** (doc 48 unit PL.5): its number in the House's sequence, its
/// price, and the IC days it is due in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requested {
    /// The order's number in the House's sequence.
    pub n: u32,
    /// What the House funds its agent with, minor units.
    pub price_minor: u64,
    /// The IC days until the yard hands her over, rounded up.
    pub days: i64,
}

/// **A House's officer orders a hull for it** (doc 48 unit PL.5; Q-48-28): `recipe` at the yard,
/// the House's treasury paying the bill at the yard's asks, the fee and the booking fee. The
/// record takes the order as [`Phase::Requested`]; the arm funds the agent, books, buys and
/// collects on later fires. The caller has checked that `by` may spend the House's money.
///
/// # Errors
///
/// A sentence: no treasury, no such recipe, the House's orders full, or the treasury short of
/// the price (with what it holds and what the order costs).
pub(crate) fn request(
    ctx: &mut Ctx<'_>,
    org: mush_org::OrgId,
    recipe: &str,
    by: ObjId,
) -> Result<Requested, String> {
    let treasury = treasury_of(ctx.w, org).ok_or(NO_TREASURY)?;
    let facility = yard(ctx.w)?;
    let (_, pinned) = crate::port::works::find_recipe(ctx.w, &facility, recipe)
        .ok_or("The yard here does not make that.")?;
    let design = pinned
        .outputs()
        .iter()
        .find_map(|line| crate::port::commission::design_of_kit(line.commodity()))
        .ok_or("The yard here does not make that.")?;
    let key = house_key(org.0);
    let mut record = match book(ctx.w).and_then(|book| book.get(&key)) {
        Some(crate::port::owned::Held::Present(record)) => (**record).clone(),
        Some(crate::port::owned::Held::Unreadable) => {
            return Err(
                "That organization's yard orders cannot be read; staff must repair them.".into(),
            );
        }
        None => HousePool {
            org: key,
            house: Some(org.0),
            ..HousePool::default()
        },
    };
    if record.orders.len() >= MAX_HOUSE_ORDERS {
        return Err(HOUSE_ORDERS_FULL.to_owned());
    }
    let price = order_price_minor(ctx, recipe, false)?;
    // What the treasury has promised its earlier orders and not yet paid its agent.
    let promised: u64 = record
        .orders
        .iter()
        .filter(|order| order.phase == Phase::Requested)
        .map(|order| order.funded_minor)
        .sum();
    let available = available_minor(ctx.w, treasury);
    if available < promised.saturating_add(price) {
        return Err(TREASURY_SHORT
            .replace(
                "{holds}",
                &crate::port::coop::money_minor(
                    ctx,
                    i64::try_from(available.saturating_sub(promised)).unwrap_or(i64::MAX),
                ),
            )
            .replace("{recipe}", recipe)
            .replace(
                "{price}",
                &crate::port::coop::money_minor(ctx, i64::try_from(price).unwrap_or(i64::MAX)),
            ));
    }
    let now = ctx.w.world_ic_now().at;
    let days = build_days(ctx.w, recipe).saturating_add(part_days(ctx, recipe));
    let n = record.made.saturating_add(1);
    record.made = n;
    record.orders.push(HouseOrder {
        n,
        design,
        funded_minor: price,
        contract: None,
        part: None,
        ordered_ic: now,
        due_ic: now.saturating_add(days.saturating_mul(DAY)),
        hull: None,
        owed: None,
        phase: Phase::Requested,
        recipe: Some(CanonicalId::new(recipe).map_err(|e| e.to_string())?),
        by: Some(by),
        topped_minor: 0,
        carry: None,
    });
    record.check()?;
    put(ctx, &record);
    tracing::info!(
        org = %record.org,
        n,
        price,
        "tacsim: a House orders a hull at the yard"
    );
    Ok(Requested {
        n,
        price_minor: price,
        days,
    })
}

/// **Doc 48 unit PL.5**: `+yard/fund` for an organization with no treasury to pay from.
pub const NO_TREASURY: &str = "That organization holds no treasury to pay from.";
/// **Doc 48 unit PL.5**: `+yard/fund` past [`MAX_HOUSE_ORDERS`].
pub const HOUSE_ORDERS_FULL: &str =
    "That organization has eight hulls on order already; wait for one to be handed over.";
/// **Doc 48 unit PL.5**: `+yard/fund` with the treasury short, `{holds}` what it has free,
/// `{recipe}` and `{price}` the order's.
pub const TREASURY_SHORT: &str = "The treasury has {holds} free; a {recipe} costs {price} bought \
                                  at the yard's asks.";

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
    // 1. The oldest open order's next step (an order stands only once the helm and its purse do).
    if let (Some(helm), Some(account), false) = (
        record.helm,
        record.account.map(AccountId),
        record.orders.is_empty(),
    ) {
        let index = 0;
        let mut order = record.orders[index].clone();
        let step = advance(ctx, &floor.spec(), home, helm, account, &mut order)?;
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
    // 2. The floor: breached and fundable, or nothing (the helm is made only when needed).
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
    // 3. The helm and its purse, a fire each, then the funding.
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
        part: None,
        ordered_ic: now,
        due_ic: now.saturating_add(
            build_days(ctx.w, floor.recipe)
                .saturating_add(part_days(ctx, floor.recipe))
                .saturating_add(PASSAGE_ALLOWANCE_DAYS)
                * DAY,
        ),
        hull: None,
        owed: None,
        phase: Phase::Funded,
        recipe: None,
        by: None,
        topped_minor: 0,
        carry: None,
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
    spec: &Spec<'_>,
    home: &Home,
    helm: ObjId,
    account: AccountId,
    order: &mut HouseOrder,
) -> Result<Advance, String> {
    match order.phase {
        // A House's order is funded before it advances (`house_step`).
        Phase::Requested => Ok(Advance::Same),
        Phase::Funded => {
            let facility = yard(ctx.w)?;
            let (recipe, pinned) = crate::port::works::find_recipe(ctx.w, &facility, spec.recipe)
                .ok_or("the yard does not build that")?;
            // Doc 48e, production G10: the booking fee is held at the booking and the fee at the
            // submit, both from the purse, so it holds them before the yard is asked.
            let booking = if &home.house == facility.owner() {
                0
            } else {
                facility.booking_fee_minor().unwrap_or(0)
            };
            let fee = facility.fee_minor(pinned.work_units()).unwrap_or(0);
            if top_up(ctx, spec, home, order, account, booking.saturating_add(fee))? {
                return Ok(Advance::Changed);
            }
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
                    .saturating_add(part_days(ctx, spec.recipe))
                    .saturating_add(if spec.ferry.is_some() {
                        PASSAGE_ALLOWANCE_DAYS
                    } else {
                        0
                    })
                    .saturating_mul(DAY),
            );
            order.phase = Phase::Supplying;
            Ok(Advance::Changed)
        }
        Phase::Supplying => {
            let id = order.contract.ok_or("no contract")?;
            if let Some(again) = rebook(ctx, spec, order, id) {
                return Ok(again);
            }
            // Doc 48e, production G11: an earlier booking's bill comes into this one before
            // anything more is bought, so nothing is bought twice.
            if let Some(old) = order.carry {
                if crate::port::works::carry_into(ctx, old, EndpointRef(VaultId(id)))? {
                    order.carry = None;
                    return Ok(Advance::Changed);
                }
                return Ok(Advance::Same);
            }
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
                // The owner's IN.11 ruling: a part the stores lack is made at the works.
                if part && (order.part.is_some() || !stores_hold(ctx, &line.good)) {
                    return supply_part(ctx, spec, home, (helm, account), order, (id, &line.good));
                }
                let want = if part { 1 } else { line.units - held };
                // Doc 48e, production G10: the purse holds this purchase at the board's live
                // price, and the fee the submit holds, before it buys.
                let (map, def) = yard_port(ctx.w)?;
                let unit = board::unit_minor(ctx, Some(helm), (&map, &def), &line.good)?;
                let need = unit
                    .saturating_mul(want)
                    .saturating_add(c.fee_minor.unwrap_or(0));
                if top_up(ctx, spec, home, order, account, need)? {
                    return Ok(Advance::Changed);
                }
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
            if let Some(again) = rebook(ctx, spec, order, id) {
                return Ok(again);
            }
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
            // Doc 48 unit PL.5: a House's hull is handed over docked at the yard, titled to it.
            if spec.ferry.is_none() {
                tracing::info!(
                    org = spec.org,
                    hull = ?order.hull,
                    "tacsim: the yard hands a House its hull"
                );
                return Ok(Advance::Home);
            }
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
            let (_, venue) = spec.ferry.ok_or("a House's hull sails nowhere")?;
            let (burn, days) = crate::port::voyage::passage_needs(ctx, hull, venue, 1)?;
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
            let filed = crate::port::voyage::undock_as(ctx, venue, false);
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
                    if spec
                        .ferry
                        .is_some_and(|(home, _)| map.entry.as_str() == home) =>
                {
                    tracing::info!(
                        org = spec.org,
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

/// **A yard order that ended before its hull is booked again** (doc 48e, production G11).
///
/// The arm steps the oldest order alone, so an order whose contract closed early (the yard's
/// cancel when its fee rose after the booking, an unfunded submit, a draft that never bound)
/// stood at the head of the queue for good, and every order behind it waited forever. It is
/// booked again instead (back to [`Phase::Funded`]); a contract still open with its bill in its
/// escrow, which the agent's missing hold leaves there, is noted as [`HouseOrder::carry`] and its
/// bill is carried into the new booking. The new booking's fee and the asks are the live ones,
/// and [`top_up`] covers any rise. `None` while the contract runs.
fn rebook(ctx: &Ctx<'_>, spec: &Spec<'_>, order: &mut HouseOrder, id: u64) -> Option<Advance> {
    let open = crate::port::works::ended_early(ctx, id)?;
    // An escrow still open holds the bill, and any earlier carry has landed in it: an earlier
    // carry still open means nothing was carried or bought into this one yet.
    if open
        && order
            .carry
            .is_none_or(|old| crate::port::works::open_contract(ctx.w, old).is_none())
    {
        order.carry = Some(id);
    }
    tracing::info!(
        org = spec.org,
        n = order.n,
        contract = id,
        "tacsim: a house's yard order ended before its hull; its agent books it again"
    );
    order.contract = None;
    order.phase = Phase::Funded;
    Some(Advance::Changed)
}

/// **The purse topped up from the house's account** (doc 48e, production G10): when `purse`
/// holds less than `need` minor, the house pays it the difference under a key naming the order
/// and what it has topped up so far, and the order records it ([`HouseOrder::topped_minor`]).
/// An order is funded at the board's prices when it is ordered and bought at its live ones, which
/// EX.8's weekly step moves, so a rise in between left the purse short mid-bill and the order
/// failing every fire. `Ok(true)` when it paid: the purchase waits a fire for the post.
///
/// # Errors
///
/// The house's account cannot cover the difference, or the post did not land.
fn top_up(
    ctx: &mut Ctx<'_>,
    spec: &Spec<'_>,
    home: &Home,
    order: &mut HouseOrder,
    purse: AccountId,
    need: u64,
) -> Result<bool, String> {
    let have = available_minor(ctx.w, purse);
    if have >= need {
        return Ok(false);
    }
    let short = need - have;
    if available_minor(ctx.w, home.account) < short {
        return Err(format!(
            "the house's account cannot cover the {short} the yard's asks have risen by"
        ));
    }
    pay(
        ctx,
        &format!("{}/{}/top/{}", spec.org, order.n, order.topped_minor),
        (home.account, purse),
        short,
        &home.house,
        format!(
            "{} tops up its order {} at the yard's asks",
            spec.org, order.n
        ),
    )?;
    order.topped_minor = order.topped_minor.saturating_add(short);
    tracing::info!(
        org = spec.org,
        n = order.n,
        short,
        "tacsim: a house tops its yard purse up at the board's live asks"
    );
    Ok(true)
}

/// **The works order for a part the stores lack** (the owner's IN.11 ruling, 2026-11-05): booked
/// at the yard port's works as the helm for the house, its bill bought into its escrow, and once
/// made and traced, moved into the hull order `into`'s escrow. One step a fire.
fn supply_part(
    ctx: &mut Ctx<'_>,
    spec: &Spec<'_>,
    home: &Home,
    (helm, account): (ObjId, AccountId),
    order: &mut HouseOrder,
    (into, good): (u64, &CanonicalId),
) -> Result<Advance, String> {
    let Some(id) = order.part else {
        let facility = works(ctx.w)?;
        let (recipe, pinned) = crate::port::works::recipe_making(ctx.w, &facility, good)
            .ok_or_else(|| format!("the works makes no {good}"))?;
        let saved = ctx.actor;
        ctx.actor = helm;
        let booked = crate::port::works::book(
            ctx,
            crate::port::works::Booking {
                front: crate::port::works::Front::Works,
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
        order.part = Some(booked?.id);
        return Ok(Advance::Changed);
    };
    // Closed: collected into the hull's escrow (or failed, and the next fire books again).
    let Some(c) = crate::port::works::open_contract(ctx.w, id) else {
        order.part = None;
        return Ok(Advance::Changed);
    };
    if c.owed.is_some() || c.order.is_none() {
        return Ok(Advance::Same);
    }
    match crate::port::works::order_state(ctx, &c) {
        Some(WorkOrderState::Draft) => {
            for line in &c.bill {
                let held = crate::port::works::escrow_units(ctx, &c, &line.good);
                if held < line.units {
                    // Doc 48e, production G10: as the hull order's purchases.
                    let (map, def) = yard_port(ctx.w)?;
                    let unit = board::unit_minor(ctx, Some(helm), (&map, &def), &line.good)?;
                    let need = unit
                        .saturating_mul(line.units - held)
                        .saturating_add(c.fee_minor.unwrap_or(0));
                    if top_up(ctx, spec, home, order, account, need)? {
                        return Ok(Advance::Changed);
                    }
                    buy(
                        ctx,
                        helm,
                        account,
                        Sink::Escrow(id),
                        (&line.good, line.units - held, held),
                    )?;
                    return Ok(Advance::Same);
                }
            }
            Ok(Advance::Same)
        }
        Some(WorkOrderState::Completed { .. }) => {
            let to = EndpointRef(VaultId(into));
            // While the part is still being traced (IN.7's commission) this waits for the next
            // fire; once the move lands the order closes, and the next step clears it.
            if let Err(error) = crate::port::works::collect_into(ctx, id, helm, to) {
                tracing::debug!(order = id, %error, "tacsim: a house part order waits to move");
            }
            Ok(Advance::Same)
        }
        Some(WorkOrderState::Cancelled | WorkOrderState::Failed) => {
            Err(format!("the house's part order {id} did not finish"))
        }
        _ => Ok(Advance::Same),
    }
}

/// Whether the port's parts stores ([`parts_operator`]) hold a drawable `good` lot, free of any
/// listing.
fn stores_hold(ctx: &Ctx<'_>, good: &CanonicalId) -> bool {
    let Ok((map, def)) = yard_port(ctx.w) else {
        return false;
    };
    let Some(row) = parts_operator(&def).and_then(|operator| map.vendors.get(&operator)) else {
        return false;
    };
    let claimed = crate::port::bay_work::claimed_store_lots(ctx.w);
    // Doc 48 cut EX.3: a lot standing in its own lot ask is for sale all the same.
    !crate::port::stock::store_lots(ctx, row, good, &claimed).is_empty()
}

/// The works at the yard's port.
fn works(w: &World) -> Result<FacilityRecord, String> {
    let (_, def) = yard_port(w)?;
    let entry = CanonicalId::new(YARD_PORT).map_err(|e| e.to_string())?;
    let room = crate::port::plant::block_room(w, &entry, &def, "works")?
        .ok_or("the yard's port authors no works")?;
    w.module_state::<mush_industry::IndustryStore>()
        .and_then(|store| store.facility(room))
        .cloned()
        .ok_or_else(|| "the works is not open".to_owned())
}

/// **The days a yard recipe's parts add** when the stores lack them: each missing part's recipe at
/// the works, one after another (the owner's IN.11 ruling).
fn part_days(ctx: &Ctx<'_>, recipe: &str) -> i64 {
    let (Ok(yard), Ok(works)) = (yard(ctx.w), works(ctx.w)) else {
        return 0;
    };
    let Some((_, pinned)) = crate::port::works::find_recipe(ctx.w, &yard, recipe) else {
        return 0;
    };
    crate::port::works::bill_with_parts(ctx.w, &pinned, 1)
        .iter()
        .filter(|line| crate::port_tick::is_part(&line.good) && !stores_hold(ctx, &line.good))
        .filter_map(|line| crate::port::works::recipe_making(ctx.w, &works, &line.good))
        .map(|(_, made)| {
            i64::try_from(
                made.work_units()
                    .div_ceil(crate::port::bill::WORK_UNITS_A_DAY),
            )
            .unwrap_or(i64::MAX)
        })
        .sum()
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
    // Doc 48 cut EX.3 (Q-48-9): an order's bill is bought on the board.
    if let Sink::Escrow(id) = sink {
        return board::buy(ctx, helm, (&map, &def), id, (good, units))
            .map(|_| None)
            .map_err(String::from);
    }
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

/// **The vendor whose stores hold a port's parts** (doc 48e, production G13): the workshop's
/// operator, whose storefront the parts market's bids and the scrap desk buy into (doc 48 unit
/// PL.8), or the yard's operator at a port that authors no workshop. At Stickney both are
/// Stickney Yards; at Ceres the yard is the Freeport Works' and the parts are Occator's, so a
/// part line read from the yard's operator found none there.
fn parts_operator(def: &PortDef) -> Option<CanonicalId> {
    def.workshop
        .as_ref()
        .map(|workshop| workshop.operator.clone())
        .or_else(|| def.yard.as_ref().map(|yard| yard.operator.clone()))
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
        // The port's parts stores (IN.7: what is made, scrapped or sold into them is drawable;
        // doc 48e, production G13: the workshop operator's).
        let operator = parts_operator(def).ok_or("the yard's port authors no yard")?;
        let vendor = def
            .vendors
            .iter()
            .find(|v| v.org == operator)
            .ok_or("the yard's operator keeps no stores")?;
        let major = w
            .module_state::<TacsimStore>()
            .and_then(TacsimStore::goods)
            .and_then(|goods| goods.good(good).map(|g| g.reference_major))
            .ok_or("no catalog price")?;
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
        // The owner's IN.11 ruling (lane v33): a vendor that stocks a good and bids for it, with
        // no ask (a vendor never both asks and bids one good), sells the house its stock at its
        // bid — Stickney Yards' carbon, which the works' plants take.
        .or_else(|| {
            def.vendors.iter().find_map(|v| {
                let stocked = v.stock.iter().any(|row| &row.good == good);
                v.bids
                    .iter()
                    .find(|bid| stocked && &bid.good == good)
                    .map(|bid| (v.clone(), bid.price_major))
            })
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

/// The days a yard recipe takes at the yard, or two (§4.7's fewest).
fn build_days(w: &World, recipe: &str) -> i64 {
    yard(w)
        .ok()
        .and_then(|facility| crate::port::works::find_recipe(w, &facility, recipe))
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
    order_price_minor(ctx, floor.recipe, true)
}

/// **What one order of `recipe` costs** (doc 48 unit PL.5): [`price_minor`]'s sum, the ferry's
/// stores in it only when she sails home (`ferry`); a House's hull is handed over at the yard.
///
/// # Errors
///
/// The yard, the recipe or a price cannot be read.
pub fn order_price_minor(ctx: &Ctx<'_>, recipe: &str, ferry: bool) -> Result<u64, String> {
    let w: &World = ctx.w;
    let facility = yard(w)?;
    let (map, def) = yard_port(w)?;
    // Doc 48e, production G10: each good at the board's live price, the price the agent pays.
    let unit = |good: &CanonicalId| board::unit_minor(ctx, None, (&map, &def), good);
    let (_, pinned) = crate::port::works::find_recipe(w, &facility, recipe)
        .ok_or("the yard does not build that")?;
    let mut total: u64 = 0;
    for line in crate::port::works::bill_with_parts(w, &pinned, 1) {
        let part = crate::port_tick::is_part(&line.good);
        // The owner's IN.11 ruling: a part the stores lack costs its works order — the bill at
        // the asks, the works' fee and booking fee.
        if part && !stores_hold(ctx, &line.good) {
            let works = works(w)?;
            let (_, made) = crate::port::works::recipe_making(w, &works, &line.good)
                .ok_or_else(|| format!("the works makes no {}", line.good))?;
            for input in crate::port::works::bill_with_parts(w, &made, 1) {
                total = total.saturating_add(unit(&input.good)?.saturating_mul(input.units));
            }
            total = total
                .saturating_add(works.fee_minor(made.work_units()).unwrap_or(0))
                .saturating_add(works.booking_fee_minor().unwrap_or(0));
            continue;
        }
        total = total.saturating_add(unit(&line.good)?.saturating_mul(line.units));
    }
    let units = pinned.work_units();
    total = total
        .saturating_add(facility.fee_minor(units).unwrap_or(0))
        .saturating_add(facility.booking_fee_minor().unwrap_or(0));
    if !ferry {
        return Ok(total);
    }
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
        total = total.saturating_add(unit(&good)?.saturating_mul(units));
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

/// **An NPC buyer's purse** (doc 48 cut EX.3): its own unscoped credit account, opened on first
/// use under `name`. The market's buyer is always a person's account, so a house's or a works'
/// buyer holds one.
///
/// # Errors
///
/// The economy cannot open one.
pub(crate) fn purse_of(ctx: &mut Ctx<'_>, buyer: ObjId, name: &str) -> Result<AccountId, String> {
    if let Some(account) = crate::port::credit::own_account(ctx.w, buyer) {
        return Ok(account.id);
    }
    open_named_purse(ctx, buyer, name)
}

/// Open the helm's own credit account.
fn open_purse(ctx: &mut Ctx<'_>, helm: ObjId) -> Result<AccountId, String> {
    open_named_purse(ctx, helm, "house helm purse")
}

/// Open `owner`'s own credit account, named `name`.
fn open_named_purse(ctx: &mut Ctx<'_>, owner: ObjId, name: &str) -> Result<AccountId, String> {
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
            owner: PrincipalId::Object(owner),
            instrument,
            scope: None,
            name: SmolStr::new(name),
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

pub use board::on_board as helm_on_board;
pub(crate) use board::{Bought, BuyError, Wait};

/// **Buy into an order's escrow on the board as `buyer`** (doc 48 unit PL.5; Q-48-28): up to
/// `want` of `good` at the port `entry`'s listings, V7's seller and price, whoever buys.
/// `+works/buy` and `+yard/buy` front it for a player's own order.
///
/// # Errors
///
/// No one at the port sells the good, or the market refuses the purchase.
pub(crate) fn buy_into(
    ctx: &mut Ctx<'_>,
    buyer: ObjId,
    entry: &CanonicalId,
    escrow: u64,
    (good, want): (&CanonicalId, u64),
) -> Result<Bought, BuyError> {
    let (map, def) = {
        let (_, def, map) = crate::port::market::ports(ctx.w)
            .into_iter()
            .find(|(port, ..)| port == entry)
            .ok_or_else(|| BuyError::Plain("That port is not laid up.".into()))?;
        let map = map
            .ok_or_else(|| BuyError::Plain("That port is not laid up.".into()))?
            .clone();
        (map, def.clone())
    };
    board::buy(ctx, buyer, (&map, &def), escrow, (good, want))
}

/// **A bill line's shortfall** (doc 48 unit PL.5): the first line of contract `c`'s bill its
/// escrow does not yet cover (only `only`'s, when named), with how many more it takes (a part's
/// lots counted one each).
#[must_use]
pub(crate) fn short_line(
    ctx: &Ctx<'_>,
    c: &crate::port::works::WorksContract,
    only: Option<&CanonicalId>,
) -> Option<(CanonicalId, u64)> {
    let lots = crate::port::works::lots_in(ctx, c.escrow());
    c.bill
        .iter()
        .filter(|line| only.is_none_or(|good| *good == line.good))
        .find_map(|line| {
            let held = if crate::port_tick::is_part(&line.good) {
                u64::try_from(lots.iter().filter(|(_, g, _)| *g == line.good).count())
                    .unwrap_or(u64::MAX)
            } else {
                crate::port::works::escrow_units(ctx, c, &line.good)
            };
            (held < line.units).then(|| (line.good.clone(), line.units - held))
        })
}

/// **A bill bought on the board** (doc 48 cut EX.3; Q-48-9, doc 48a A-3; doc 48 unit PL.5): V7's
/// purchases, each a quote and an acceptance by the buyer into the order's escrow, paid from the
/// buyer's own account. The buyer is a house's helm or yard agent on the arm, or a player buying
/// their own order's bill (`+works/buy`, `+yard/buy`).
///
/// - **A bulk line** takes a seller's standing ask at the port, at most what it lists. A good
///   some vendor at the port asks is bought only from that ask: while it is not listed, the buyer
///   waits.
/// - **A good stocked and bid for with no ask** (the owner's IN.11 ruling: Stickney Yards'
///   carbon): the port's clerk lists the buyer a **private** draw ask of what the line wants, at
///   the bid, and the buyer takes it on a later fire or command. Private, so the vendor still
///   never both asks and bids one good on the board. The market keeps one live draw listing a
///   pool, so while another buyer's private ask stands the buyer waits; a player's lapses after
///   [`PLAYER_ASK_TTL_SECS`].
/// - **A part** takes one lot of the yard operator's stores. A lot already standing in a lot ask
///   the buyer may take (public, or private to them) is quoted as it stands; otherwise the
///   port's clerk lists the buyer the lowest-grade unlisted lot as a private exact-lot ask at its
///   catalogue price.
///
/// **The guarantee** (A-3): the market escrows the price before the lot is drawn, and a leg that
/// cannot be staged refunds the escrow to the purse; nothing moves without escrowed money, and
/// each purchase is a trade on the tape. One purchase stands at a time: while the buyer's quote
/// is on the board nothing else is bought, and the next try reads the escrow again. The seller
/// and the price are the listing's, so the seam serves any buyer.
mod board {
    use mush_core::{CanonicalId, Ctx, ObjId, World};
    use mush_economy::{Amount, CatalogRef, VaultId};
    use mush_inventory::record::EndpointRef;
    use mush_market::{ListingId, MarketStore};
    use mush_org::PrincipalId;
    use smol_str::SmolStr;

    use crate::port::def::PortDef;
    use crate::port::map::PortMap;

    /// **What one purchase step did.**
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) enum Bought {
        /// Quoted and accepted: `quantity` escrowed at `price` minor a unit; the goods land when
        /// the market's draw settles.
        Accepted {
            /// The vendor's name.
            seller: String,
            /// How many.
            quantity: u64,
            /// Minor a unit.
            price: u64,
        },
        /// The port's clerk lists the buyer a private ask of `quantity` at `price` minor a unit;
        /// the buyer takes it on a later try.
        Listed {
            /// The vendor's name.
            seller: String,
            /// How many.
            quantity: u64,
            /// Minor a unit.
            price: u64,
        },
        /// Nothing now, and why.
        Wait(Wait),
    }

    /// **Why a purchase was refused**: the market's own refusal (a player reads its sentence),
    /// or the seam's.
    #[derive(Debug)]
    pub(crate) enum BuyError {
        /// The market refused the quote or the acceptance.
        Market(mush_market::MarketError),
        /// No seller, no part, no clerk: the seam's sentence.
        Plain(String),
    }

    impl From<String> for BuyError {
        fn from(why: String) -> Self {
            Self::Plain(why)
        }
    }

    impl From<&str> for BuyError {
        fn from(why: &str) -> Self {
            Self::Plain(why.to_owned())
        }
    }

    impl From<BuyError> for String {
        fn from(error: BuyError) -> Self {
            match error {
                BuyError::Market(error) => format!("the market refused the purchase: {error:?}"),
                BuyError::Plain(why) => why,
            }
        }
    }

    /// **Why a purchase step waits.**
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Wait {
        /// One of the buyer's purchases is still on the board.
        OnBoard,
        /// The engine's queue is full, or the buyer or the port's clerk has a submission queued.
        Busy,
        /// A draw on the seller's pool has not settled, or its ask is not listed just now.
        Pool,
        /// Another buyer's private ask holds the pool, or every stores lot of the part.
        Held,
    }

    /// Buy up to `want` of `good` into order `escrow`'s escrow, as `buyer`.
    ///
    /// # Errors
    ///
    /// No seller lists the good, or the market refuses the purchase.
    pub(crate) fn buy(
        ctx: &mut Ctx<'_>,
        buyer: ObjId,
        (map, def): (&PortMap, &PortDef),
        escrow: u64,
        (good, want): (&CanonicalId, u64),
    ) -> Result<Bought, BuyError> {
        if on_board(ctx.w, buyer) {
            return Ok(Bought::Wait(Wait::OnBoard));
        }
        if ctx.authoritative_queue_full() || ctx.authoritative_pending(buyer) {
            return Ok(Bought::Wait(Wait::Busy));
        }
        let pick = if crate::port_tick::is_part(good) {
            part_listing(ctx, buyer, (map, def), good)?
        } else {
            bulk_listing(ctx, buyer, (map, def), good, want)?
        };
        let (listing, quantity) = match pick {
            Pick::Take(listing, quantity) => (listing, quantity),
            Pick::Done(done) => return Ok(done),
        };
        if quantity == 0 {
            return Ok(Bought::Wait(Wait::Pool));
        }
        let (seller, price) = ctx
            .w
            .module_state::<MarketStore>()
            .and_then(|market| {
                let row = market.listing(listing)?;
                let name = market.vendor(row.vendor).map(|v| v.name.to_string())?;
                Some((name, u64::try_from(row.price.minor()).unwrap_or(0)))
            })
            .unwrap_or_default();
        let into = EndpointRef(VaultId(escrow));
        let saved = ctx.actor;
        ctx.actor = buyer;
        let bought = quote_and_accept(ctx, listing, quantity, into);
        ctx.actor = saved;
        bought.map_err(BuyError::Market)?;
        Ok(Bought::Accepted {
            seller,
            quantity,
            price,
        })
    }

    /// What a listing search found.
    enum Pick {
        /// A listing to take, and how many from it.
        Take(ListingId, u64),
        /// Nothing to take this try.
        Done(Bought),
    }

    /// The quote and the acceptance, as the actor.
    fn quote_and_accept(
        ctx: &mut Ctx<'_>,
        listing: ListingId,
        quantity: u64,
        into: EndpointRef,
    ) -> Result<(), mush_market::MarketError> {
        let quote = mush_market::quote(
            ctx,
            &mush_market::QuoteRequest {
                listing,
                quantity,
                window: mush_market::QuoteWindow::Live,
                ttl_live: mush_market::QUOTE_TTL_LIVE,
                ttl_remote: mush_market::QUOTE_TTL_REMOTE,
                presence: mush_market::Presence::Here,
            },
        )?;
        let _allow = crate::port::works::allow_escrow_buy(ctx.actor, into.vault().0);
        mush_market::accept_at(ctx, quote, Some(into), mush_market::Presence::Here).map(|_| ())
    }

    /// **Whether `buyer` has a purchase on the board**: a quote whose contract has not settled
    /// or closed.
    #[must_use]
    pub fn on_board(w: &World, buyer: ObjId) -> bool {
        w.module_state::<MarketStore>().is_some_and(|market| {
            market
                .quotes_of_buyer(&PrincipalId::Object(buyer))
                .any(|quote| crate::port::supply::on_board(w, buyer, quote.listing))
        })
    }

    /// Whether `buyer` may take `listing`: public, or private to them.
    fn open_to(listing: &mush_market::Listing, buyer: ObjId) -> bool {
        listing
            .private_to
            .as_ref()
            .is_none_or(|to| *to == PrincipalId::Object(buyer))
    }

    /// **What one `good` costs on the board now**, minor (doc 48e, production G10): the listing
    /// [`buy`] would take, at its live price, which EX.8's weekly step moves — a seller's standing
    /// ask, a private ask already listed to `buyer`, or a part's lot ask open to them (`None`:
    /// open to anyone, a public one) — else the price the port's clerk would list it at (the
    /// authored ask or bid, a part's catalogue price). A read: it lists nothing.
    ///
    /// # Errors
    ///
    /// No one at the port sells the good, or its price cannot be read.
    pub(crate) fn unit_minor(
        ctx: &Ctx<'_>,
        buyer: Option<ObjId>,
        (map, def): (&PortMap, &PortDef),
        good: &CanonicalId,
    ) -> Result<u64, String> {
        let market = ctx.w.module_state::<MarketStore>().ok_or("no market")?;
        let open = |listing: &mush_market::Listing| match buyer {
            Some(buyer) => open_to(listing, buyer),
            None => listing.private_to.is_none(),
        };
        let price = |listing: &mush_market::Listing| u64::try_from(listing.price.minor()).ok();
        let part = crate::port_tick::is_part(good);
        let live = if part {
            let row = super::parts_operator(def).and_then(|operator| map.vendors.get(&operator));
            let claimed = crate::port::bay_work::claimed_store_lots(ctx.w);
            row.map(|row| crate::port::stock::store_lots(ctx, row, good, &claimed))
                .unwrap_or_default()
                .iter()
                .filter_map(|lot| lot.listing)
                .filter_map(|id| market.listing(id))
                .find(|listing| open(listing))
                .and_then(price)
        } else {
            let askers: Vec<u64> = def
                .vendors
                .iter()
                .filter(|vendor| vendor.asks.iter().any(|ask| &ask.good == good))
                .filter_map(|vendor| map.vendors.get(&vendor.org).map(|row| row.vault))
                .collect();
            if askers.is_empty() {
                let (vendor, _) = super::seller(ctx.w, def, good, false)?;
                map.vendors.get(&vendor.org).and_then(|row| {
                    market
                        .lot_listings_of(EndpointRef(VaultId(row.vault)), good)
                        .find(|listing| open(listing))
                        .and_then(price)
                })
            } else {
                askers.iter().find_map(|vault| {
                    market
                        .lot_listings_of(EndpointRef(VaultId(*vault)), good)
                        .find(|listing| listing.private_to.is_none())
                        .and_then(price)
                })
            }
        };
        match live {
            Some(unit) => Ok(unit),
            None => super::seller(ctx.w, def, good, part).map(|(_, unit)| unit),
        }
    }

    /// The listing a bulk `good` is bought from at the port and how much of `want` to take from
    /// it now (0 while a draw on the pool is unsettled). A seller's standing ask, when a vendor
    /// here asks the good; else the vendor that stocks it and bids for it with no ask (the
    /// owner's IN.11 ruling, lane v33: Stickney Yards' carbon) lists the buyer a private draw
    /// ask of what it wants, at its bid.
    fn bulk_listing(
        ctx: &mut Ctx<'_>,
        buyer: ObjId,
        (map, def): (&PortMap, &PortDef),
        good: &CanonicalId,
        want: u64,
    ) -> Result<Pick, String> {
        let market = ctx.w.module_state::<MarketStore>().ok_or("no market")?;
        let take = |vault: u64, listing: ListingId, listed: u64| {
            let (_, in_flight) = market.draw_reading(EndpointRef(VaultId(vault)), good);
            Pick::Take(listing, if in_flight { 0 } else { want.min(listed) })
        };
        let askers: Vec<u64> = def
            .vendors
            .iter()
            .filter(|vendor| vendor.asks.iter().any(|ask| &ask.good == good))
            .filter_map(|vendor| map.vendors.get(&vendor.org).map(|row| row.vault))
            .collect();
        if !askers.is_empty() {
            // Doc 48 unit PL.5: a good asked here is bought from its standing ask alone.
            let standing = askers.iter().find_map(|vault| {
                market
                    .lot_listings_of(EndpointRef(VaultId(*vault)), good)
                    .find(|listing| listing.private_to.is_none())
                    .map(|listing| take(*vault, listing.id, listing.quantity))
            });
            return Ok(standing.unwrap_or(Pick::Done(Bought::Wait(Wait::Pool))));
        }
        let (vendor, price) = super::seller(ctx.w, def, good, false)?;
        let row = map
            .vendors
            .get(&vendor.org)
            .cloned()
            .ok_or("the seller is not laid up")?;
        let on_pool = market
            .lot_listings_of(EndpointRef(VaultId(row.vault)), good)
            .next()
            .map(|listing| (open_to(listing, buyer), listing.id, listing.quantity));
        if let Some((open, listing, listed)) = on_pool {
            if !open {
                return Ok(Pick::Done(Bought::Wait(Wait::Held)));
            }
            return Ok(take(row.vault, listing, listed));
        }
        let pool =
            crate::port_tick::read_pool(ctx, EndpointRef(VaultId(row.vault)), good).unwrap_or(0);
        let quantity = want.min(pool).min(mush_market::MAX_QUANTITY);
        if quantity == 0 {
            return Err(format!("the port holds no {good} to sell"));
        }
        let vendor_id = row.vendor.ok_or("the seller is not a vendor")?;
        let source = mush_market::Supply::Stock {
            vault: VaultId(row.vault),
        };
        let listed = list_for(
            ctx,
            map,
            buyer,
            (vendor_id, good, price),
            (source, quantity),
        )?;
        Ok(Pick::Done(if listed {
            Bought::Listed {
                seller: vendor_name(ctx.w, vendor_id),
                quantity,
                price,
            }
        } else {
            Bought::Wait(Wait::Busy)
        }))
    }

    /// The vendor's name on the market, or empty.
    fn vendor_name(w: &World, vendor: u64) -> String {
        w.module_state::<MarketStore>()
            .and_then(|market| market.vendor(mush_market::VendorId(vendor)))
            .map(|v| v.name.to_string())
            .unwrap_or_default()
    }

    /// **A private ask to `buyer`**, listed by the port's clerk for `vendor`: `quantity` of
    /// `good` from `source` at `price` minor a unit. `Ok(false)` while the clerk is busy.
    fn list_for(
        ctx: &mut Ctx<'_>,
        map: &PortMap,
        buyer: ObjId,
        (vendor, good, price): (u64, &CanonicalId, u64),
        (source, quantity): (mush_market::Supply, u64),
    ) -> Result<bool, String> {
        let clerk = map
            .clerk
            .as_ref()
            .map(|clerk| ObjId(clerk.thing))
            .ok_or("the port has no clerk")?;
        if ctx.authoritative_pending(clerk) {
            return Ok(false);
        }
        let price = i64::try_from(price).map_err(|_| "a price overflows")?;
        // Doc 48 unit PL.5: a house's agent takes its ask on the next fire; a player's lapses in
        // minutes, so another buyer of the pool waits minutes, not days.
        let ttl_secs = if super::is_helm(ctx.w, buyer) {
            mush_market::MAX_LISTING_TTL_SECS
        } else {
            super::PLAYER_ASK_TTL_SECS
        };
        let spec = mush_market::ListingSpec {
            vendor: mush_market::VendorId(vendor),
            catalog: CatalogRef {
                ruleset: SmolStr::new_static(crate::NS),
                code: SmolStr::new(good.as_str()),
            },
            price: Amount::from_minor(price),
            quantity,
            source,
            object: None,
            visibility: mush_market::MarketVisibility::Hidden,
            ttl_secs,
            private_to: Some(PrincipalId::Object(buyer)),
        };
        let saved = ctx.actor;
        ctx.actor = clerk;
        let listed = mush_market::list_goods(ctx, spec);
        ctx.actor = saved;
        listed
            .map(|_| true)
            .map_err(|error| format!("the port could not list its {good}: {error:?}"))
    }

    /// **The part's lot ask**: a lot of `good` in the yard operator's stores standing in a lot
    /// ask the buyer may take, or the clerk lists the buyer the lowest-grade unlisted lot at its
    /// catalogue price.
    fn part_listing(
        ctx: &mut Ctx<'_>,
        buyer: ObjId,
        (map, def): (&PortMap, &PortDef),
        good: &CanonicalId,
    ) -> Result<Pick, String> {
        let operator = super::parts_operator(def).ok_or("the port authors no yard")?;
        let row = map
            .vendors
            .get(&operator)
            .cloned()
            .ok_or("the yard's operator keeps no stores")?;
        let claimed = crate::port::bay_work::claimed_store_lots(ctx.w);
        let lots = crate::port::stock::store_lots(ctx, &row, good, &claimed);
        let market = ctx.w.module_state::<MarketStore>().ok_or("no market")?;
        let standing = lots.iter().filter_map(|lot| lot.listing).find(|id| {
            market
                .listing(*id)
                .is_some_and(|listing| open_to(listing, buyer))
        });
        if let Some(listing) = standing {
            return Ok(Pick::Take(listing, 1));
        }
        let Some(lot) = lots.iter().find(|lot| lot.listing.is_none()) else {
            return if lots.is_empty() {
                Err("the yard's stores hold no such part".to_owned())
            } else {
                Ok(Pick::Done(Bought::Wait(Wait::Held)))
            };
        };
        let (lot, grade) = (lot.lot, lot.grade);
        let vendor = row.vendor.ok_or("the yard's operator is not a vendor")?;
        let (_, price) = super::seller(ctx.w, def, good, true)?;
        let source = mush_market::Supply::Lot {
            endpoint: mush_market::LotEndpoint(EndpointRef(VaultId(row.vault))),
            commodity: mush_market::LotCommodity(good.clone()),
            grade,
            lot: Some(lot),
        };
        let listed = list_for(ctx, map, buyer, (vendor, good, price), (source, 1))?;
        Ok(Pick::Done(if listed {
            Bought::Listed {
                seller: vendor_name(ctx.w, vendor),
                quantity: 1,
                price,
            }
        } else {
            Bought::Wait(Wait::Busy)
        }))
    }
}
