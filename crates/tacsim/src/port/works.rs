//! **The order contract** (doc 39 cut IN.3, §4.12; Q-39-7, Q-39-28, Q-39-29, Q-39-30): `+works`
//! and `+yard` front `@wo` at a port's works or yard, the way `+bay` fronts the bay.
//!
//! - **Quoted** — `+works/quote <recipe> [qty]`: the bill, the days, the fee and the booking fee
//!   from the recipe and the facility's `FacilityCostDef`; nothing is held.
//! - **Booked** — `+works/order <recipe> [qty][=<org>]`: an **escrow** endpoint for the order (a
//!   vault of the requester's on a crate of its own in the facility, so one vault a backing
//!   holds), the **booking fee** (one slot-day) held on the payer's account, and one `@wo/draft`
//!   whose source and destination are the escrow. The contract records the requester, the payer
//!   and the beneficiary (Q-39-30): the requester pays, and the beneficiary is the requester
//!   unless an officer names their organization.
//! - **Supplied** — `+works/deliver <order>=<good> <qty>` from the requester's hold docked at the
//!   port, in as many deliveries as the bill takes (one goods move a delivery, this module's
//!   `tacsim_works` act); for a house order, from the house's stores.
//! - **Running** — the arm's submit (§4.12: "on the fire after the bill is covered"), in booking
//!   order while the facility has a slot free, so two orders for one slot queue. The provider
//!   stamps the materials and the slot at the order's duration plus the TTL and holds
//!   `fee(order)`.
//! - **AwaitingDelivery → Delivered** — the arm delivers on its next fire, a player's order
//!   included, so nothing waits on a hand; the output lands in the escrow.
//! - **Collected** — `+works/collect <order>`: the escrow's goods and lots into the requester's
//!   docked hold (a house order's into its stores, by the arm). After 28 IC days uncollected, one
//!   slot-day a day of storage is charged until the account runs dry, when the facility takes
//!   the goods into its stores.
//! - **Cancelled** — `+works/cancel <order>`, the requester's alone (the facility cancels only an
//!   unfunded order): the booking fee forfeit, and from Running the elapsed work units' fee too
//!   (`mush_industry::money::cancel_forfeit_minor`), the whole bill back to the hold; refused
//!   past half the work units and once the work is done (`mush_industry::provider::cancel_cutoff`).
//!
//! The contract is a `tacsim:works:{id}` document (the order's number is its escrow vault's);
//! the one goods move a step owes is an authoritative act under the contract's ledger ordinal
//! (`tacsim.works_ledger`), the bay act's shape: the command or the arm records it, the offer
//! projects it from the record, the prepare moves the goods once, and the tick settles it.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{
    ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn, AuthoritativeActionOutput,
    CanonicalId, CmdError, Ctx, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    ModError, ObjId, PerceivedRef, ProjectedParameter, ProjectedParameterKind,
    ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft, Registrar, RevisionSet,
    RulesValue, StateFamilyHandle, SubjectKey, World,
};
use mush_economy::{AccountId, CustodyRef, EconomyStore, VaultId};
use mush_industry::{
    FacilityRecord, IndustryHandles, IndustryVerb, RecipeAuthority, WorkOrder, WorkOrderId,
    WorkOrderState,
};
use mush_inventory::record::EndpointRef;
use mush_inventory::{InventoryFamilies, LotId};
use mush_market::{CapacityPlan, CapacityProvider, CapacityRefusal, Presence, VenueGate};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use smol_str::SmolStr;

use crate::port::coop::{money_minor, say};
use crate::port::market::{PortCapacity, PortVenueGate};
use crate::port_tick::port_error;
use crate::store::TacsimStore;

/// The contract documents' key prefix: `tacsim:works:{id}`.
pub const WORKS_PREFIX: &str = "tacsim:works:";
/// The goods move's action.
pub const WORKS_ACTION: &str = "tacsim_works";
/// Its outcome.
pub const WORKS_OUTCOME: &str = "tacsim_works_result";
/// The contracts' ledger family: the last landed act a contract.
pub const WORKS_FAMILY: &str = "tacsim.works_ledger";
/// The capability a works' facility carries.
pub const WORKS_CAPABILITY: &str = "tacsim.works";
/// The capability a yard's facility carries.
pub const YARD_CAPABILITY: &str = "tacsim.yard";
/// IC seconds an uncollected delivery waits before storage is charged (§4.12: 28 IC days).
pub const STORAGE_GRACE_SECS: i64 = 28 * 86_400;
/// IC seconds a storage day.
pub const DAY_SECS: i64 = 86_400;
/// Tries an owed move is resubmitted before the arm gives it up and tells the requester.
pub const MAX_TRIES: u8 = 4;
/// Fires a booked contract waits for its draft to land before it is closed unbound.
pub const MAX_UNBOUND_FIRES: u8 = 8;
/// Lines a bill may carry (`mush_industry::MAX_WORK_ORDER_INPUTS`).
pub const MAX_BILL_LINES: usize = 10;
/// Lots one move may carry.
pub const MAX_MOVE_LOTS: usize = 32;
/// Owed moves one offer projects.
const MAX_OFFER_ROWS: usize = 16;
/// Lot pages a collection reads looking for the escrow's lots.
const MAX_LOT_PAGES: usize = 16;
/// Lots a lot page reads.
const LOT_PAGE: usize = 256;

/// The usage line.
pub const WORKS_USAGE: &str = "Usage: +works · +works/quote <recipe> [qty] · \
                               +works/order <recipe> [qty][=<org>] · \
                               +works/deliver <order>=<good> <qty> · +works/collect <order> · \
                               +works/cancel <order>";
/// The help text (`+yard` reads the same with its own word).
pub const WORKS_HELP: &str = "+works                          the works here: what it makes, its \
                              slots, your orders.\n\
                              +works/quote <recipe> [qty]     the bill, the days, the fee and the \
                              booking fee.\n\
                              +works/order <recipe> [qty][=<org>]  book it: an escrow and the \
                              booking fee (one slot-day, forfeit on a cancel).\n\
                              +works/deliver <order>=<good> <qty>  move bill goods from your docked \
                              hold into the escrow (a part as its lots); the order starts when the \
                              bill is covered.\n\
                              +works/collect <order>          the output (or a cancelled bill) into \
                              your docked hold; at a yard, the hull's commission: she is handed \
                              over docked here (a ship) or on the workshop floor (a rig), titled to \
                              you, her magazines empty.\n\
                              +works/cancel <order>           before half the work is done: the \
                              bill back, the booking fee and the work done so far forfeit.";
/// The switches.
pub const WORKS_SWITCHES: &[&str] = &["quote", "order", "deliver", "collect", "cancel"];

const NO_ORDER: &str = "You have no such order here.";

// ---- the sentences the bot reads (doc 39 cut IN.5; `mush-bot`'s `lane::works`) ----------------

/// A booking's middle, before the front's word: "Order {n} booked at the works: …".
pub const BOOKED_AT: &str = " booked at the ";
/// The delivery's tell, before the front's word: "Order {n} is delivered at the works: …".
pub const DELIVERED_AT: &str = " is delivered at the ";
/// A delivery into the escrow's tail: "{units} {good} moves into order {n}'s escrow.".
pub const INTO_ESCROW: &str = "'s escrow.";
/// A collection's tail: "Order {n}: {goods} move into your hold.".
pub const INTO_HOLD: &str = " move into your hold.";
/// The quote's fee clause: "…; fee {cr}; booking fee {cr}.".
pub const FEE_CLAUSE: &str = "; fee ";
/// A collection or delivery asked for while the order's last move settles.
pub const SETTLING: &str = "That order is still settling.";

// ---- the records --------------------------------------------------------------------------

/// **Which front** an order was booked through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Front {
    /// A works: fittings and part recipes.
    Works,
    /// A yard: hull recipes (IN.9's row authors the yard).
    Yard,
}

impl Front {
    /// The verb.
    #[must_use]
    pub const fn verb(self) -> &'static str {
        match self {
            Self::Works => "+works",
            Self::Yard => "+yard",
        }
    }
    /// The facility's word.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Works => "works",
            Self::Yard => "yard",
        }
    }
    /// The capability the facility carries.
    #[must_use]
    pub const fn capability(self) -> &'static str {
        match self {
            Self::Works => WORKS_CAPABILITY,
            Self::Yard => YARD_CAPABILITY,
        }
    }
}

/// **How a contract closed.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Close {
    /// The goods went to the requester's hold (a house order's to its stores).
    Collected,
    /// Cancelled, and the escrow's contents returned.
    Cancelled,
    /// Storage went unpaid; the facility took the goods.
    Seized,
    /// The draft never landed, or the order failed; the escrow's contents returned.
    Failed,
}

/// One bill line: a good and its units.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Goods {
    /// The good.
    pub good: CanonicalId,
    /// Its units (the goods table's measure: tonnes for bulk, each for a part).
    pub units: u64,
}

/// **What a landed move settles.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Then {
    /// A delivery into the escrow: nothing more.
    Supplied,
    /// The collection: the contract closes `Collected`.
    Collected,
    /// The bill returned after a cancel or a failure: it closes as its WO ended.
    Returned,
    /// Storage unpaid: the facility's stores take the goods, and it closes `Seized`.
    Seized,
    /// **Doc 39 cut IN.9 (§4.13)**: a yard order's kit and part lots **consumed** in the escrow
    /// (never moved), the commission's first half; it closes `Collected` and the commission
    /// (`crate::port::commission`) lays the hull up.
    Commissioned,
    /// **Doc 39 cut IN.5** (§4.3's `output_lots: pooled`): the delivery's bulk lot dissolved into
    /// the escrow's pool where it lies, the arm's act on the fire after the delivery, so the
    /// collection moves pools. Nothing more.
    Pooled,
}

/// **One goods move a contract owes** — the bay's `OwedAct` shape: the act ordinal (the ledger's
/// next), the actor it is submitted as, the endpoints and what moves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorksAct {
    /// The ledger ordinal it lands as.
    pub act: u32,
    /// Who submits it.
    pub by: ObjId,
    /// The vault it moves from.
    pub from: u64,
    /// The vault it moves to.
    pub to: u64,
    /// Pool goods moved.
    pub goods: Vec<Goods>,
    /// Lots moved whole; a bulk lot is dissolved into the destination's pool.
    pub lots: Vec<LotId>,
    /// What landing it settles.
    pub then: Then,
    /// Submissions so far.
    pub tries: u8,
}

impl WorksAct {
    /// Bounds.
    ///
    /// # Errors
    ///
    /// A sentence naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.then == Then::Commissioned {
            // The consumption names the escrow twice and lots only.
            if self.act == 0
                || self.from != self.to
                || !self.goods.is_empty()
                || self.lots.is_empty()
                || self.lots.len() > MAX_MOVE_LOTS
            {
                return Err("a commission consumes lots in one escrow".into());
            }
            return Ok(());
        }
        // Doc 39 cut IN.5: a pooling act names one endpoint, its lots and no pool goods.
        let pooling = self.then == Then::Pooled;
        if pooling && (self.from != self.to || !self.goods.is_empty() || self.lots.is_empty()) {
            return Err("a pooling act names one endpoint and the lots it dissolves".into());
        }
        if self.act == 0
            || (self.from == self.to && !pooling)
            || self.goods.len() > MAX_BILL_LINES
            || self.lots.len() > MAX_MOVE_LOTS
            || (self.goods.is_empty() && self.lots.is_empty())
            || self.goods.iter().any(|g| g.units == 0)
        {
            return Err("a works move names a landed ordinal, two endpoints and something".into());
        }
        Ok(())
    }
}

/// **One order contract** (`tacsim:works:{id}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorksContract {
    /// The order's number: its escrow vault's id.
    pub id: u64,
    /// The front it was booked through.
    pub front: Front,
    /// The facility.
    pub facility: ObjId,
    /// The escrow's crate (the vault's backing).
    pub escrow_crate: ObjId,
    /// The character who booked it.
    pub requester: ObjId,
    /// Who pays (the budget account's owner).
    pub payer: PrincipalId,
    /// Who the output is for.
    pub beneficiary: PrincipalId,
    /// The budget account the fee and the booking are held on.
    pub account: u64,
    /// A house order: the facility's owner orders from its own stores (Q-39-7).
    pub house: bool,
    /// The recipe's pack.
    pub pack: CanonicalId,
    /// Its version.
    pub version: u32,
    /// The recipe.
    pub recipe: CanonicalId,
    /// How many.
    pub quantity: u32,
    /// The bill: every input line times the quantity.
    pub bill: Vec<Goods>,
    /// The work units (the days are these over 24).
    pub work_units: u64,
    /// When it was booked, IC seconds (the queue's order).
    pub booked_ic: i64,
    /// The booking fee held, minor units (0 for a house order).
    pub booking_minor: u64,
    /// The work order, once its draft landed.
    pub order: Option<u64>,
    /// Fires spent waiting for the draft.
    pub unbound: u8,
    /// When the output landed in the escrow, IC seconds.
    pub delivered_ic: Option<i64>,
    /// Storage days charged.
    pub storage_days: u32,
    /// The landed act ordinal (the ledger's, mirrored).
    pub acts: u32,
    /// The move owed, if any.
    pub owed: Option<WorksAct>,
    /// How it closed.
    pub closed: Option<Close>,
}

impl WorksContract {
    /// Bounds.
    ///
    /// # Errors
    ///
    /// A sentence naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.bill.is_empty() || self.bill.len() > MAX_BILL_LINES || self.quantity == 0 {
            return Err("a contract carries one to ten bill lines and a quantity".into());
        }
        if let Some(owed) = &self.owed {
            owed.check()?;
        }
        Ok(())
    }

    /// The escrow endpoint.
    #[must_use]
    pub const fn escrow(&self) -> EndpointRef {
        EndpointRef(VaultId(self.id))
    }

    /// The document key.
    #[must_use]
    pub fn key(&self) -> String {
        works_key(self.id)
    }
}

/// One contract's ledger: the last landed move.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorksLedger {
    /// The contract: the key.
    pub contract: u64,
    /// The last landed act.
    pub acts: u32,
}

/// The document key of contract `id`.
#[must_use]
pub fn works_key(id: u64) -> String {
    format!("{WORKS_PREFIX}{id}")
}

/// A stored contract: read, or unreadable (which holds it until staff repair the document).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorksState {
    /// The record.
    Present(Box<WorksContract>),
    /// The document would not read.
    Unreadable,
}

// ---- the store ------------------------------------------------------------------------------

/// Install a contract and queue its document.
pub(crate) fn put(ctx: &mut Ctx<'_>, contract: &WorksContract) {
    crate::store::put_works(ctx, contract);
}

fn contracts(w: &World) -> Vec<WorksContract> {
    w.module_state::<TacsimStore>()
        .map(TacsimStore::works_contracts)
        .unwrap_or_default()
}

fn contract(w: &World, id: u64) -> Option<WorksContract> {
    match w.module_state::<TacsimStore>()?.works_contract(id)? {
        WorksState::Present(c) => Some((**c).clone()),
        WorksState::Unreadable => None,
    }
}

// ---- reads ----------------------------------------------------------------------------------

fn handles(w: &World) -> Option<(mush_industry::IndustryFamilies, InventoryFamilies)> {
    let industry = w.module_state::<IndustryHandles>()?.0.clone();
    let inventory = industry.inventory.clone();
    Some((industry, inventory))
}

fn work_order(ctx: &Ctx<'_>, id: u64) -> Option<WorkOrder> {
    let (industry, _) = handles(ctx.w)?;
    ctx.action_state(&industry.work_orders, &WorkOrderId(id))
        .ok()
        .flatten()
        .cloned()
}

fn facility_record(w: &World, facility: ObjId) -> Option<FacilityRecord> {
    w.module_state::<mush_industry::IndustryStore>()?
        .facility(facility)
        .cloned()
}

/// Slots free at `facility` now (the capacity book's own count, as `@facility/show` reads it).
fn slots_free(ctx: &Ctx<'_>, facility: &FacilityRecord) -> u64 {
    let Some((industry, _)) = handles(ctx.w) else {
        return 0;
    };
    if ctx
        .w
        .module_state::<mush_industry::IndustryStore>()
        .is_some_and(|store| store.is_facility_quarantined(facility.object()))
    {
        return 0;
    }
    let Ok(key) = mush_industry::facility_capacity_key(facility.object()) else {
        return 0;
    };
    let scalar = ctx
        .action_state(&industry.facility_slots, &key)
        .ok()
        .flatten()
        .copied();
    let book = ctx
        .action_state(&industry.facility_books, &key)
        .ok()
        .flatten();
    match (scalar, book) {
        (Some(scalar), Some(book)) => book.checked_available(scalar).unwrap_or(0),
        _ => u64::from(facility.maximum_slots()),
    }
}

fn pooled(ctx: &Ctx<'_>, endpoint: EndpointRef, good: &CanonicalId) -> u64 {
    crate::port_tick::read_pool(ctx, endpoint, good).unwrap_or(0)
}

/// **The front's facility at the actor's port**: the port whose rooms hold the actor, and the
/// facility there carrying the front's capability.
fn here(ctx: &Ctx<'_>, front: Front) -> Result<(CanonicalId, FacilityRecord), String> {
    let room = ctx
        .w
        .get(ctx.actor)
        .and_then(|o| o.location)
        .ok_or_else(|| format!("There is no {} here.", front.word()))?;
    let capability =
        CanonicalId::new(front.capability()).map_err(|_| "The front is misconfigured.")?;
    for (entry, def, _) in crate::port::market::ports(ctx.w) {
        let rooms = crate::port::market::port_rooms(ctx.w, def);
        if !rooms.contains(&room) {
            continue;
        }
        // Doc 39 cut IN.5: the lane's works row is a room of the port (its spine, at both
        // stations), which is the facility itself.
        let authored = crate::port::plant::works_room(ctx.w, &entry, def)
            .ok()
            .flatten();
        let found = ctx
            .w
            .module_state::<mush_industry::IndustryStore>()
            .into_iter()
            .flat_map(|store| store.facilities())
            .find(|f| {
                f.capabilities().contains(&capability)
                    && (Some(f.object()) == authored
                        || ctx
                            .w
                            .get(f.object())
                            .and_then(|o| o.location)
                            .is_some_and(|at| rooms.contains(&at)))
            })
            .cloned();
        if let Some(found) = found {
            return Ok((entry, found));
        }
    }
    Err(format!("There is no {} at this port.", front.word()))
}

fn port_def<'a>(w: &'a World, entry: &CanonicalId) -> Option<&'a crate::port::def::PortDef> {
    crate::port::market::ports(w)
        .into_iter()
        .find(|(id, _, _)| id == entry)
        .map(|(_, def, _)| def)
}

fn venue_of(def: &crate::port::def::PortDef) -> Option<mush_market::MarketId> {
    mush_market::MarketId::new(def.venue.as_str())
}

/// **The requester's hold docked at the port**, as the market reads it (the charter hull's hold,
/// admitted by the same gate: docked here, no passage filed). `present` asks that the actor
/// stand in the port too.
fn docked_hold(
    w: &World,
    who: ObjId,
    entry: &CanonicalId,
    present: bool,
) -> Result<EndpointRef, String> {
    let def = port_def(w, entry).ok_or("That port is not open.")?;
    let venue = venue_of(def).ok_or("That port has no market.")?;
    let hold = PortVenueGate
        .endpoint_for(w, who, &venue, Presence::Here)
        .ok_or("You have no ship's hold docked here.")?;
    let presence = if present {
        Presence::Here
    } else {
        Presence::Ship(
            crate::port::coop::helm_of(w, who)
                .map(|h| h.hull)
                .ok_or("You have no ship's hold docked here.")?,
        )
    };
    PortVenueGate
        .admit(w, who, &venue, Some(hold), presence)
        .map_err(|refusal| refusal.line)?;
    Ok(hold)
}

/// **The facility's stores**: the vault on the facility object itself, owned by its owner —
/// found, or opened on first need (a house order's source and sink; a seizure's destination).
pub(crate) fn stores(ctx: &mut Ctx<'_>, facility: &FacilityRecord) -> Result<VaultId, String> {
    let backing = CustodyRef::Container(facility.object());
    if let Some(vault) = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().vault_by_backing(&backing).map(|v| v.id))
    {
        return Ok(vault);
    }
    let id = crate::port::apply::alloc_vault_id(ctx)?;
    mush_economy::create_vault(
        ctx,
        mush_economy::Vault {
            id,
            owner: facility.owner().clone(),
            scope: None,
            name: SmolStr::new("Works stores"),
            backing,
            status: mush_economy::VaultStatus::Active,
            revision: 1,
        },
    );
    Ok(id)
}

/// The lots at the escrow (a bounded walk of the lot family; the escrow's lots are its outputs
/// and any part lot delivered into it).
fn escrow_lots(ctx: &Ctx<'_>, escrow: EndpointRef) -> Vec<(LotId, CanonicalId, u64)> {
    let Some((_, inventory)) = handles(ctx.w) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_LOT_PAGES {
        let Ok(page) = ctx.scan_action_state(&inventory.lots, cursor.as_deref(), LOT_PAGE) else {
            break;
        };
        for entry in page.entries() {
            let lot = entry.value();
            if lot.endpoint() == escrow && out.len() < MAX_MOVE_LOTS {
                out.push((lot.id(), lot.commodity().clone(), lot.quantity()));
            }
        }
        match page.next_after() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }
    out
}

fn recipes() -> Arc<dyn RecipeAuthority> {
    Arc::new(crate::composition::industry_recipes::TacsimRecipeAuthority)
}

/// The recipes `facility` runs: `(pack, version, id, pinned)` from the published catalogue.
fn menu(
    w: &World,
    facility: &FacilityRecord,
) -> Vec<(mush_industry::RecipeRef, mush_industry::PinnedRecipe)> {
    recipes()
        .catalog(w)
        .into_iter()
        .filter(|(_, pinned)| facility.offers(pinned.requires()))
        .collect()
}

pub(crate) fn find_recipe(
    w: &World,
    facility: &FacilityRecord,
    word: &str,
) -> Option<(mush_industry::RecipeRef, mush_industry::PinnedRecipe)> {
    menu(w, facility)
        .into_iter()
        .find(|(r, _)| r.id().as_str() == word)
}

fn days_of(units: u64) -> u64 {
    units.div_ceil(mush_industry::cost::WORK_UNITS_PER_DAY)
}

/// A yard or works order's whole bill: the recipe's lines and, for a hull, the design's parts.
pub(crate) fn bill_with_parts(
    w: &World,
    pinned: &mush_industry::PinnedRecipe,
    quantity: u32,
) -> Vec<Goods> {
    let mut bill = bill_of(pinned, quantity);
    bill.extend(design_parts(w, pinned, quantity));
    bill
}

fn bill_of(pinned: &mush_industry::PinnedRecipe, quantity: u32) -> Vec<Goods> {
    pinned
        .inputs()
        .iter()
        .map(|line| Goods {
            good: line.commodity().clone(),
            units: line.units().saturating_mul(u64::from(quantity)),
        })
        .collect()
}

/// **A hull recipe's part lines** (doc 39 cut IN.9): the design its kit builds, one part a slot
/// from the design's bill (`crate::port::bill`), times the quantity. None for a recipe whose
/// output is no kit.
fn design_parts(w: &World, pinned: &mush_industry::PinnedRecipe, quantity: u32) -> Vec<Goods> {
    let Some(design) = pinned
        .outputs()
        .iter()
        .find_map(|line| crate::port::commission::design_of_kit(line.commodity()))
    else {
        return Vec::new();
    };
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Vec::new();
    };
    let Some(goods) = store.goods() else {
        return Vec::new();
    };
    crate::port::bill::design_bill(store.definitions(), goods, &design)
        .map(|bill| {
            bill.part_lines()
                .into_iter()
                .map(|(good, units)| Goods {
                    good,
                    units: units.saturating_mul(u64::from(quantity)),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn bill_text(bill: &[Goods]) -> String {
    bill.iter()
        .map(|g| format!("{} {}", g.units, g.good.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn cr(ctx: &Ctx<'_>, minor: u64) -> String {
    money_minor(ctx, i64::try_from(minor).unwrap_or(i64::MAX))
}

// ---- the command ----------------------------------------------------------------------------

/// `+works`.
///
/// # Errors
///
/// The refusal the actor reads.
pub(crate) fn works_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    front_cmd(ctx, Front::Works)
}

/// `+yard`.
///
/// # Errors
///
/// The refusal the actor reads.
pub(crate) fn yard_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    front_cmd(ctx, Front::Yard)
}

fn front_cmd(ctx: &mut Ctx<'_>, front: Front) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    let raw = ctx.args.raw().trim().to_owned();
    let run = |ctx: &mut Ctx<'_>| -> Result<(), String> {
        if ctx.switches.has("quote") {
            return quote(ctx, front, &raw);
        }
        if ctx.switches.has("order") {
            return order(ctx, front, &raw);
        }
        if ctx.switches.has("deliver") {
            return deliver(ctx, front, &raw);
        }
        if ctx.switches.has("collect") {
            return collect(ctx, front, &raw);
        }
        if ctx.switches.has("cancel") {
            return cancel(ctx, front, &raw);
        }
        show(ctx, front)
    };
    run(ctx).map_err(CmdError::msg)
}

fn show(ctx: &mut Ctx<'_>, front: Front) -> Result<(), String> {
    let (entry, facility) = here(ctx, front)?;
    let free = slots_free(ctx, &facility);
    let mut lines = vec![format!(
        "The {} here: {} slots, {free} free.",
        front.word(),
        facility.maximum_slots()
    )];
    for (recipe, pinned) in menu(ctx.w, &facility) {
        let units = pinned.work_units();
        lines.push(format!(
            "  {}: {} -> {}, {} IC days.",
            recipe.id().as_str(),
            bill_text(&bill_of(&pinned, 1)),
            pinned
                .outputs()
                .iter()
                .map(|l| format!("{} {}", l.units(), l.commodity().as_str()))
                .collect::<Vec<_>>()
                .join(", "),
            days_of(units)
        ));
    }
    for c in contracts(ctx.w).into_iter().filter(|c| {
        c.facility == facility.object() && c.requester == ctx.actor && c.closed.is_none()
    }) {
        let state = c
            .order
            .and_then(|o| work_order(ctx, o))
            .map_or_else(|| "booked".to_owned(), |o| phase_word(ctx, &c, &o));
        lines.push(format!(
            "  Order {}: {} x{}, {state}.",
            c.id,
            c.recipe.as_str(),
            c.quantity
        ));
    }
    let _ = entry;
    for line in lines {
        say(ctx, line);
    }
    Ok(())
}

fn phase_word(ctx: &Ctx<'_>, c: &WorksContract, order: &WorkOrder) -> String {
    match order.state() {
        WorkOrderState::Draft => {
            let short: Vec<String> = c
                .bill
                .iter()
                .filter_map(|g| {
                    let have = pooled(ctx, c.escrow(), &g.good);
                    (have < g.units).then(|| format!("{} {}", g.units - have, g.good.as_str()))
                })
                .collect();
            if short.is_empty() {
                "supplied, waiting for a slot".to_owned()
            } else {
                format!("booked, short {}", short.join(", "))
            }
        }
        WorkOrderState::Running | WorkOrderState::Interrupted { .. } => {
            let (done, total) = order
                .schedule()
                .map_or((0, 0), |s| (s.progress_units(), s.total_units()));
            format!("running, {done} of {total} work units")
        }
        WorkOrderState::AwaitingDelivery => "done, delivering".to_owned(),
        WorkOrderState::Completed { .. } => "delivered, ready to collect".to_owned(),
        WorkOrderState::Cancelled => "cancelled".to_owned(),
        WorkOrderState::Failed => "failed".to_owned(),
        WorkOrderState::Ready | WorkOrderState::AwaitingApproval { .. } => "starting".to_owned(),
    }
}

fn parse_qty(word: Option<&str>) -> Result<u32, String> {
    match word {
        None => Ok(1),
        Some(n) => n
            .parse::<u32>()
            .ok()
            .filter(|n| *n >= 1 && *n <= 100)
            .ok_or_else(|| "How many? One to a hundred.".to_owned()),
    }
}

fn quote(ctx: &mut Ctx<'_>, front: Front, raw: &str) -> Result<(), String> {
    let (_, facility) = here(ctx, front)?;
    let mut words = raw.split_whitespace();
    let word = words.next().ok_or(WORKS_USAGE)?;
    let quantity = parse_qty(words.next())?;
    let (_, pinned) = find_recipe(ctx.w, &facility, word)
        .ok_or_else(|| format!("The {} here does not make that.", front.word()))?;
    let units = pinned.work_units().saturating_mul(u64::from(quantity));
    let fee = facility.fee_minor(units);
    let booking = facility.booking_fee_minor();
    let line = format!(
        "{word} x{quantity}: bill {}; {} IC days ({units} work units){FEE_CLAUSE}{}; booking fee {}.",
        bill_text(&bill_of(&pinned, quantity)),
        days_of(units),
        fee.map_or_else(|| "none".to_owned(), |f| cr(ctx, f)),
        booking.map_or_else(|| "none".to_owned(), |f| cr(ctx, f)),
    );
    say(ctx, line);
    Ok(())
}

fn order(ctx: &mut Ctx<'_>, front: Front, raw: &str) -> Result<(), String> {
    let (entry, facility) = here(ctx, front)?;
    let (terms, org) = match raw.split_once('=') {
        Some((terms, org)) => (terms.trim(), Some(org.trim())),
        None => (raw, None),
    };
    let mut words = terms.split_whitespace();
    let word = words.next().ok_or(WORKS_USAGE)?;
    let quantity = parse_qty(words.next())?;
    if !mush_industry::may_order(ctx.w, ctx.actor, &facility) {
        return Err(format!(
            "The {} here takes no orders from you.",
            front.word()
        ));
    }
    let (recipe, pinned) = find_recipe(ctx.w, &facility, word)
        .ok_or_else(|| format!("The {} here does not make that.", front.word()))?;
    // The triple (Q-39-30): the requester pays; the beneficiary is the requester unless an
    // officer names their organization.
    let payer = PrincipalId::Object(ctx.actor);
    let beneficiary = match org {
        None | Some("") => payer.clone(),
        Some(text) => {
            let named = mush_economy::parse_principal(ctx, text)
                .filter(|p| matches!(p, PrincipalId::Organization(_)))
                .ok_or("No such organization.")?;
            if !mush_economy::acts_for(ctx.w, ctx.actor, &named) {
                return Err("You do not act for that organization.".to_owned());
            }
            named
        }
    };
    let account = crate::port::credit::own_account(ctx.w, ctx.actor)
        .map(|a| a.id)
        .ok_or("You hold no credit account to pay from.")?;
    let contract = book(
        ctx,
        Booking {
            front,
            facility: &facility,
            recipe: &recipe,
            pinned: &pinned,
            quantity,
            payer,
            beneficiary,
            account,
        },
    )?;
    let _ = entry;
    let line = format!(
        "Order {}{BOOKED_AT}{}: {word} x{quantity}, {} IC days. Deliver the bill: {}. \
         Booking fee {} held{}.",
        contract.id,
        front.word(),
        days_of(contract.work_units),
        bill_text(&contract.bill),
        cr(ctx, contract.booking_minor),
        if contract.booking_minor > 0 {
            " (forfeit on a cancel)"
        } else {
            ""
        }
    );
    say(ctx, line);
    Ok(())
}

/// **What one booking names** (`book`'s terms): the front and its facility, the recipe, the
/// quantity, and the triple's payer and beneficiary with the payer's own account.
pub(crate) struct Booking<'a> {
    /// The front.
    pub front: Front,
    /// The facility.
    pub facility: &'a FacilityRecord,
    /// The recipe.
    pub recipe: &'a mush_industry::RecipeRef,
    /// Its pin.
    pub pinned: &'a mush_industry::PinnedRecipe,
    /// How many.
    pub quantity: u32,
    /// Who pays: `ctx.actor`'s own principal (the industry's budget rule).
    pub payer: PrincipalId,
    /// Who the output is for.
    pub beneficiary: PrincipalId,
    /// The payer's account.
    pub account: AccountId,
}

/// **Book one order as `ctx.actor`** (§4.12): the escrow, the booking fee held, the draft, and
/// the contract written. `+works/order` and `+yard/order` front it; doc 39 cut IN.10's house arm
/// (`crate::port::pools`) books a house's yard order through it as the house's helm.
///
/// # Errors
///
/// The refusal the actor reads; nothing stands after one but a closed escrow vault.
pub(crate) fn book(ctx: &mut Ctx<'_>, terms: Booking<'_>) -> Result<WorksContract, String> {
    let Booking {
        front,
        facility,
        recipe,
        pinned,
        quantity,
        payer,
        beneficiary,
        account,
    } = terms;
    let house = &beneficiary == facility.owner();
    let units = pinned.work_units().saturating_mul(u64::from(quantity));
    let booking = if house {
        0
    } else {
        facility.booking_fee_minor().unwrap_or(0)
    };
    // The escrow: a crate of its own in the facility (one vault a backing), the vault the
    // requester's (§4.12: custody is the requester's throughout).
    let escrow_crate = ctx
        .w
        .create(
            mush_core::ObjKind::Thing,
            &format!("{} order crate", front.word()),
            mush_core::GOD,
            Some(facility.object()),
        )
        .map_err(|e| e.to_string())?;
    let vault = crate::port::apply::alloc_vault_id(ctx)?;
    mush_economy::create_vault(
        ctx,
        mush_economy::Vault {
            id: vault,
            owner: payer.clone(),
            scope: None,
            name: SmolStr::new(format!("Order {} escrow", vault.0)),
            backing: CustodyRef::Container(escrow_crate),
            status: mush_economy::VaultStatus::Active,
            revision: 1,
        },
    );
    if booking > 0
        && let Err(error) = mush_industry::money::place_booking(
            ctx,
            vault.0,
            account,
            &payer,
            &beneficiary,
            booking,
        )
    {
        let _ = mush_economy::set_vault_status(ctx, vault, mush_economy::VaultStatus::Closed);
        return Err(if error.is_transient() {
            mush_industry::money::LEDGER_BUSY.to_owned()
        } else {
            "Your account cannot cover the booking fee.".to_owned()
        });
    }
    let industry = handles(ctx.w).ok_or("Industry is not available here.")?.0;
    mush_industry::commands::draft(
        ctx,
        &industry,
        &recipes(),
        &mush_industry::commands::Draft {
            pack: recipe.pack().clone(),
            recipe: recipe.id().clone(),
            version: recipe.version(),
            facility: facility.object(),
            quantity,
            account: account.0,
            source: vault.0,
            destination: vault.0,
            beneficiary: mush_economy::principal_token(&beneficiary).to_string(),
        },
    )
    .map_err(|e| {
        let _ = mush_industry::money::settle_booking(ctx, vault.0, None);
        e.to_string()
    })?;
    let mut bill = bill_of(pinned, quantity);
    // Doc 39 cut IN.9 (§4.7): a hull's parts are the design's, one a slot, delivered as lots.
    if front == Front::Yard {
        bill.extend(design_parts(ctx.w, pinned, quantity));
    }
    let contract = WorksContract {
        id: vault.0,
        front,
        facility: facility.object(),
        escrow_crate,
        requester: ctx.actor,
        payer,
        beneficiary,
        account: account.0,
        house,
        pack: recipe.pack().clone(),
        version: recipe.version(),
        recipe: recipe.id().clone(),
        quantity,
        bill: bill.clone(),
        work_units: units,
        booked_ic: ctx.w.world_ic_now().at,
        booking_minor: booking,
        order: None,
        unbound: 0,
        delivered_ic: None,
        storage_days: 0,
        acts: 0,
        owed: None,
        closed: None,
    };
    put(ctx, &contract);
    Ok(contract)
}

/// The actor's open contract `raw` names at this front's facility here.
fn mine(ctx: &Ctx<'_>, front: Front, raw: &str) -> Result<(FacilityRecord, WorksContract), String> {
    let (_, facility) = here(ctx, front)?;
    let id = raw
        .trim()
        .trim_start_matches('#')
        .parse::<u64>()
        .map_err(|_| NO_ORDER)?;
    let c = contract(ctx.w, id)
        .filter(|c| c.facility == facility.object() && c.closed.is_none())
        .ok_or(NO_ORDER)?;
    Ok((facility, c))
}

fn deliver(ctx: &mut Ctx<'_>, front: Front, raw: &str) -> Result<(), String> {
    let (left, right) = raw.split_once('=').ok_or(WORKS_USAGE)?;
    let (facility, mut c) = mine(ctx, front, left)?;
    if c.requester != ctx.actor && !mush_economy::acts_for(ctx.w, ctx.actor, &c.beneficiary) {
        return Err(NO_ORDER.to_owned());
    }
    let mut words = right.split_whitespace();
    let good = words
        .next()
        .and_then(|w| CanonicalId::new(w).ok())
        .ok_or(WORKS_USAGE)?;
    let units = words
        .next()
        .and_then(|w| w.parse::<u64>().ok())
        .filter(|u| *u > 0)
        .ok_or(WORKS_USAGE)?;
    let line = c
        .bill
        .iter()
        .find(|g| g.good == good)
        .ok_or("That good is not on the order's bill.")?;
    let have = pooled(ctx, c.escrow(), &good);
    if have.saturating_add(units) > line.units {
        return Err(format!(
            "The bill takes {} {}; the escrow holds {have}.",
            line.units,
            good.as_str()
        ));
    }
    if c.order
        .and_then(|o| work_order(ctx, o))
        .is_some_and(|o| *o.state() != WorkOrderState::Draft)
    {
        return Err("That order is no longer taking goods.".to_owned());
    }
    if c.owed.is_some() {
        return Err("That order's last delivery is still settling.".to_owned());
    }
    let from = if c.house {
        EndpointRef(stores(ctx, &facility)?)
    } else {
        let entry = here(ctx, front)?.0;
        docked_hold(ctx.w, ctx.actor, &entry, true)?
    };
    let carried = pooled(ctx, from, &good);
    if carried < units {
        return Err(format!(
            "That hold carries {carried} {}, not {units}.",
            good.as_str()
        ));
    }
    // Doc 39 cut IN.9: a part moves as its lots, one each, never as a pool.
    let moved = if crate::port_tick::is_part(&good) {
        let lots: Vec<_> = escrow_lots(ctx, from)
            .into_iter()
            .filter(|(_, held, _)| *held == good)
            .take(usize::try_from(units).unwrap_or(usize::MAX))
            .collect();
        if u64::try_from(lots.len()).unwrap_or(0) < units {
            return Err(format!(
                "That hold carries no {units} loose {} lots.",
                good.as_str()
            ));
        }
        (Vec::new(), lots)
    } else {
        (
            vec![Goods {
                good: good.clone(),
                units,
            }],
            Vec::new(),
        )
    };
    let (by, escrow) = (ctx.actor, c.escrow());
    owe(ctx, &mut c, by, (from, escrow), moved, Then::Supplied)?;
    say(
        ctx,
        format!(
            "{units} {} moves into order {}{INTO_ESCROW}",
            good.as_str(),
            c.id
        ),
    );
    Ok(())
}

fn collect(ctx: &mut Ctx<'_>, front: Front, raw: &str) -> Result<(), String> {
    let (_, mut c) = mine(ctx, front, raw)?;
    if c.requester != ctx.actor && !mush_economy::acts_for(ctx.w, ctx.actor, &c.beneficiary) {
        return Err(NO_ORDER.to_owned());
    }
    let state = c.order.and_then(|o| work_order(ctx, o)).map(|o| *o.state());
    let then = match state {
        Some(WorkOrderState::Completed { .. }) => Then::Collected,
        Some(WorkOrderState::Cancelled | WorkOrderState::Failed) => Then::Returned,
        _ => return Err("That order has nothing to collect yet.".to_owned()),
    };
    if c.owed.is_some() {
        return Err(SETTLING.to_owned());
    }
    let entry = here(ctx, front)?.0;
    // Doc 39 cut IN.9: a delivered hull order is collected by its commission, never into a hold.
    if then == Then::Collected
        && let Some(lots) = kit_lots(ctx, &c)
    {
        return commission(ctx, &mut c, &entry, lots);
    }
    let hold = docked_hold(ctx.w, ctx.actor, &entry, true)?;
    let (goods, lots) = escrow_contents(ctx, &c);
    // Doc 39 cut IN.5: a bulk output is pooled by the arm before it moves.
    if !bulk_lots(&lots).is_empty() {
        return Err(SETTLING.to_owned());
    }
    if goods.is_empty() && lots.is_empty() {
        close(
            ctx,
            &mut c,
            if then == Then::Collected {
                Close::Collected
            } else {
                Close::Cancelled
            },
        );
        say(
            ctx,
            format!("Order {} is closed; its escrow was empty.", c.id),
        );
        return Ok(());
    }
    let words = describe(&goods, &lots);
    let (by, escrow) = (ctx.actor, c.escrow());
    owe(ctx, &mut c, by, (escrow, hold), (goods, lots), then)?;
    say(ctx, format!("Order {}: {words}{INTO_HOLD}", c.id));
    Ok(())
}

/// **A hull order's lots** (doc 39 cut IN.9): the escrow's lots when one of them is a hull kit,
/// the kit first; `None` for any other order.
fn kit_lots(ctx: &Ctx<'_>, c: &WorksContract) -> Option<Vec<(LotId, CanonicalId, u64)>> {
    let mut lots = escrow_lots(ctx, c.escrow());
    let kit = lots
        .iter()
        .position(|(_, good, _)| crate::port::commission::is_kit(good))?;
    let first = lots.remove(kit);
    lots.insert(0, first);
    Some(lots)
}

/// **`+yard/collect` on a delivered hull order** (doc 39 cut IN.9, §4.13): the commission
/// record first, then the one move that consumes the kit and the parts in the escrow.
fn commission(
    ctx: &mut Ctx<'_>,
    c: &mut WorksContract,
    entry: &CanonicalId,
    lots: Vec<(LotId, CanonicalId, u64)>,
) -> Result<(), String> {
    use crate::port::commission::{Step, YardCommission, design_of_kit, next_mark};
    let (kit, kit_good, _) = lots.first().cloned().ok_or(NO_ORDER)?;
    let design = design_of_kit(&kit_good).ok_or("That order's kit names no design.")?;
    let ship = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(|store| store.definitions().specs.get(&design))
        .and_then(|spec| spec.logistics.as_ref())
        .ok_or("The yard has no plans for that design.")?
        .drive
        .is_some();
    if crate::port::commission::commission(ctx.w, c.id).is_some() {
        return Err("That order is already being commissioned.".to_owned());
    }
    let parts: Vec<LotId> = lots.iter().skip(1).map(|(lot, ..)| *lot).collect();
    let (n, mark) = next_mark(ctx.w);
    let record = YardCommission {
        order: c.id,
        work_order: c.order.ok_or(NO_ORDER)?,
        facility: c.facility,
        home: entry.clone(),
        design,
        requester: ctx.actor,
        beneficiary: c.beneficiary.clone(),
        mark: mark.clone(),
        n,
        ship,
        kit,
        parts,
        step: Step::Consuming,
        row: None,
        hold: None,
        at_ic: ctx.w.world_ic_now().at,
    };
    record.check()?;
    crate::port::commission::put(ctx, &record);
    let (by, escrow) = (ctx.actor, c.escrow());
    owe(
        ctx,
        c,
        by,
        (escrow, escrow),
        (Vec::new(), lots),
        Then::Commissioned,
    )?;
    say(
        ctx,
        format!(
            "Order {}: the yard commissions her as {mark}; she is handed over when the work is \
             done.",
            c.id
        ),
    );
    Ok(())
}

/// The bulk lots among `lots`, the first of each good (doc 39 cut IN.5: one dissolve a pool an
/// act, since a bundle mutates a pool's key once).
fn bulk_lots(lots: &[(LotId, CanonicalId, u64)]) -> Vec<(LotId, CanonicalId, u64)> {
    let mut seen = std::collections::BTreeSet::new();
    lots.iter()
        // Doc 39 cut IN.9: a hull kit is never pooled; the commission consumes it whole.
        .filter(|(_, good, _)| {
            !crate::port_tick::is_part(good)
                && !crate::port::commission::is_kit(good)
                && seen.insert(good.clone())
        })
        .cloned()
        .collect()
}

// ---- the house arm's seams (doc 39 cut IN.10) -----------------------------------------------

/// An open contract by its number.
pub(crate) fn open_contract(w: &World, id: u64) -> Option<WorksContract> {
    contract(w, id).filter(|c| c.closed.is_none())
}

/// The state of a contract's work order, once its draft landed.
pub(crate) fn order_state(ctx: &Ctx<'_>, c: &WorksContract) -> Option<WorkOrderState> {
    c.order.and_then(|o| work_order(ctx, o)).map(|o| *o.state())
}

/// The units of `good` pooled in a contract's escrow.
pub(crate) fn escrow_units(ctx: &Ctx<'_>, c: &WorksContract, good: &CanonicalId) -> u64 {
    pooled(ctx, c.escrow(), good)
}

/// The lots in an endpoint (an escrow, a storefront), at most a move's worth.
pub(crate) fn lots_in(ctx: &Ctx<'_>, endpoint: EndpointRef) -> Vec<(LotId, CanonicalId, u64)> {
    escrow_lots(ctx, endpoint)
}

/// **A house order's supply** (doc 39 cut IN.10): `goods` and `lots` from `from` into contract
/// `id`'s escrow, owed as `by` — the `+yard/deliver` move with the source the arm bought from.
///
/// # Errors
///
/// The order is gone, settling, past its draft, or the move does not check.
pub(crate) fn supply_from(
    ctx: &mut Ctx<'_>,
    id: u64,
    by: ObjId,
    from: EndpointRef,
    (goods, lots): (Vec<Goods>, Vec<(LotId, CanonicalId, u64)>),
) -> Result<(), String> {
    let mut c = open_contract(ctx.w, id).ok_or(NO_ORDER)?;
    if c.owed.is_some() {
        return Err(SETTLING.to_owned());
    }
    if order_state(ctx, &c).is_some_and(|state| state != WorkOrderState::Draft) {
        return Err("That order is no longer taking goods.".to_owned());
    }
    let escrow = c.escrow();
    owe(
        ctx,
        &mut c,
        by,
        (from, escrow),
        (goods, lots),
        Then::Supplied,
    )
}

/// **A house order's collection** (doc 39 cut IN.10): a delivered hull order commissioned as
/// `requester` (the house's helm), the `+yard/collect` body; the hull is titled to the order's
/// beneficiary, the house.
///
/// # Errors
///
/// Nothing to collect yet, the order still settling, or the commission refused.
pub(crate) fn commission_as(ctx: &mut Ctx<'_>, id: u64, requester: ObjId) -> Result<(), String> {
    let mut c = open_contract(ctx.w, id).ok_or(NO_ORDER)?;
    if !matches!(order_state(ctx, &c), Some(WorkOrderState::Completed { .. })) {
        return Err("That order has nothing to collect yet.".to_owned());
    }
    if c.owed.is_some() {
        return Err(SETTLING.to_owned());
    }
    let facility = facility_record(ctx.w, c.facility).ok_or(NO_ORDER)?;
    let entry = port_of(ctx.w, &facility).ok_or(NO_ORDER)?;
    let lots = kit_lots(ctx, &c).ok_or("That order delivered no kit.")?;
    let saved = ctx.actor;
    ctx.actor = requester;
    let done = commission(ctx, &mut c, &entry, lots);
    ctx.actor = saved;
    done
}

fn describe(goods: &[Goods], lots: &[(LotId, CanonicalId, u64)]) -> String {
    let mut parts: Vec<String> = goods
        .iter()
        .map(|g| format!("{} {}", g.units, g.good.as_str()))
        .collect();
    parts.extend(
        lots.iter()
            .map(|(_, good, n)| format!("{n} {} (lot)", good.as_str())),
    );
    parts.join(", ")
}

fn escrow_contents(
    ctx: &Ctx<'_>,
    c: &WorksContract,
) -> (Vec<Goods>, Vec<(LotId, CanonicalId, u64)>) {
    // The bill's goods, and (doc 39 cut IN.5) the order's pooled bulk outputs; a part output
    // stays a lot.
    let mut named: Vec<CanonicalId> = c.bill.iter().map(|g| g.good.clone()).collect();
    if let Some(order) = c.order.and_then(|o| work_order(ctx, o)) {
        for line in order.pinned().outputs() {
            if !crate::port_tick::is_part(line.commodity()) && !named.contains(line.commodity()) {
                named.push(line.commodity().clone());
            }
        }
    }
    let goods = named
        .into_iter()
        .filter_map(|good| {
            let units = pooled(ctx, c.escrow(), &good);
            (units > 0).then_some(Goods { good, units })
        })
        .collect();
    (goods, escrow_lots(ctx, c.escrow()))
}

fn cancel(ctx: &mut Ctx<'_>, front: Front, raw: &str) -> Result<(), String> {
    let (_, c) = mine(ctx, front, raw)?;
    // The cancel authority is the requester's (Q-39-29, Q-39-30); the facility cancels only an
    // unfunded order, which is the arm's.
    if c.requester != ctx.actor {
        return Err(
            "Only the one who booked an order may cancel it; the works cancels only an \
             unfunded one."
                .to_owned(),
        );
    }
    let Some(id) = c.order else {
        return Err("That order is still being drafted; try again shortly.".to_owned());
    };
    let order = work_order(ctx, id).ok_or(NO_ORDER)?;
    if order.state().is_terminal() {
        return Err("That order is already finished.".to_owned());
    }
    if let Some(refusal) = mush_industry::provider::cancel_cutoff(&order) {
        return Err(refusal.to_owned());
    }
    mush_industry::commands::submit(
        ctx,
        IndustryVerb::WoCancel,
        json!({ "order": WorkOrderId(id).to_string() }),
        "works.cancel",
    )
    .map_err(|e| e.to_string())?;
    let forfeit = mush_industry::money::live_hold(ctx, WorkOrderId(id))
        .map(|hold| {
            let held = u64::try_from(hold.amount.minor()).unwrap_or(0);
            mush_industry::money::cancel_forfeit_minor(ctx.w, &order, held)
        })
        .unwrap_or(0);
    let line = format!(
        "Order {} cancelled: the bill comes back to your hold; the booking fee {} and the work \
         done, {}, are forfeit.",
        c.id,
        cr(ctx, c.booking_minor),
        cr(ctx, forfeit)
    );
    say(ctx, line);
    Ok(())
}

/// Record a move as owed and submit it as `by`.
fn owe(
    ctx: &mut Ctx<'_>,
    c: &mut WorksContract,
    by: ObjId,
    (from, to): (EndpointRef, EndpointRef),
    (goods, lots): (Vec<Goods>, Vec<(LotId, CanonicalId, u64)>),
    then: Then,
) -> Result<(), String> {
    let act = WorksAct {
        act: c
            .acts
            .checked_add(1)
            .ok_or("That order's ledger is full.")?,
        by,
        from: from.vault().0,
        to: to.vault().0,
        goods,
        lots: lots.into_iter().map(|(lot, ..)| lot).collect(),
        then,
        tries: 1,
    };
    act.check()?;
    c.owed = Some(act.clone());
    put(ctx, c);
    submit(ctx, c.id, &act);
    Ok(())
}

fn close(ctx: &mut Ctx<'_>, c: &mut WorksContract, how: Close) {
    c.closed = Some(how);
    c.owed = None;
    put(ctx, c);
    let _ = mush_economy::set_vault_status(ctx, VaultId(c.id), mush_economy::VaultStatus::Closed);
}

/// Submit `owed` as its actor (the bay's swap).
/// Submit an owed move under `contract`'s ledger as its `by` (the house arm's hold moves too).
pub(crate) fn submit_move(ctx: &mut Ctx<'_>, contract: u64, owed: &WorksAct) -> bool {
    submit(ctx, contract, owed)
}

fn submit(ctx: &mut Ctx<'_>, contract: u64, owed: &WorksAct) -> bool {
    let Some(action) = ctx
        .w
        .module_state::<WorksHandles>()
        .and_then(|h| h.action.clone())
    else {
        return false;
    };
    let Ok(key) = mush_core::IdempotencyKeyText::parse_canonical(format!(
        "tacsim.works/{contract}/{}/{}",
        owed.act, owed.tries
    )) else {
        return false;
    };
    let saved = ctx.actor;
    ctx.actor = owed.by;
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        vec![],
        json!({"contract": contract, "act": owed.act}),
        key,
    );
    ctx.actor = saved;
    true
}

/// Submit one industry verb as `who` (the arm's swap).
fn submit_as(ctx: &mut Ctx<'_>, who: ObjId, verb: IndustryVerb, params: Value, tag: &str) {
    let saved = ctx.actor;
    ctx.actor = who;
    if let Err(error) = mush_industry::commands::submit(ctx, verb, params, tag) {
        tracing::warn!(%error, "tacsim: the works arm could not submit");
    }
    ctx.actor = saved;
}

// ---- the arm --------------------------------------------------------------------------------

/// **The works arm** (Q-39-7, Q-39-28; the port tick): for every open contract — settle a
/// landed move or resubmit it, bind a landed draft, submit a covered order when the facility
/// has a slot (in booking order, so orders queue), deliver a finished one, settle the booking
/// fee, return a cancelled bill, collect a house order, and charge storage after 28 IC days.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let mut open: Vec<WorksContract> = contracts(ctx.w)
        .into_iter()
        .filter(|c| c.closed.is_none())
        .collect();
    if open.is_empty() {
        return;
    }
    open.sort_by_key(|c| (c.booked_ic, c.id));
    let mut free: BTreeMap<ObjId, u64> = BTreeMap::new();
    for mut c in open {
        if settle(ctx, &mut c) {
            continue;
        }
        step(ctx, &mut c, &mut free);
    }
}

/// A move owed: settled when its ledger shows it landed, else resubmitted or given up.
fn settle(ctx: &mut Ctx<'_>, c: &mut WorksContract) -> bool {
    let Some(owed) = c.owed.clone() else {
        return false;
    };
    if landed(ctx, c.id) >= owed.act {
        c.acts = owed.act;
        c.owed = None;
        match owed.then {
            Then::Supplied | Then::Pooled => put(ctx, c),
            Then::Collected => close(ctx, c, Close::Collected),
            Then::Seized => close(ctx, c, Close::Seized),
            Then::Commissioned => {
                crate::port::commission::consumed(ctx, c.id);
                close(ctx, c, Close::Collected);
            }
            Then::Returned => {
                let failed = c
                    .order
                    .and_then(|o| work_order(ctx, o))
                    .is_none_or(|o| *o.state() == WorkOrderState::Failed);
                close(
                    ctx,
                    c,
                    if failed {
                        Close::Failed
                    } else {
                        Close::Cancelled
                    },
                );
            }
        }
        return true;
    }
    if owed.tries >= MAX_TRIES {
        c.owed = None;
        put(ctx, c);
        ctx.out.info(
            c.requester,
            format!(
                "Order {}: the goods could not be moved; nothing moved.",
                c.id
            ),
        );
        return true;
    }
    let mut again = owed;
    again.tries = again.tries.saturating_add(1);
    c.owed = Some(again.clone());
    put(ctx, c);
    submit(ctx, c.id, &again);
    true
}

pub(crate) fn landed(ctx: &Ctx<'_>, contract: u64) -> u32 {
    let Some(handles) = ctx.w.module_state::<WorksHandles>() else {
        return 0;
    };
    match ctx.action_state(&handles.ledgers, &contract) {
        Ok(Some(ledger)) if ledger.contract == contract => ledger.acts,
        _ => 0,
    }
}

#[allow(clippy::too_many_lines)] // one contract's step through its states, in order
fn step(ctx: &mut Ctx<'_>, c: &mut WorksContract, free: &mut BTreeMap<ObjId, u64>) {
    let Some(facility) = facility_record(ctx.w, c.facility) else {
        return;
    };
    // Bind the draft once it landed.
    let Some(id) = c.order.or_else(|| bind(ctx, c)) else {
        c.unbound = c.unbound.saturating_add(1);
        if c.unbound >= MAX_UNBOUND_FIRES {
            let _ = mush_industry::money::settle_booking(ctx, c.id, None);
            fail_back(ctx, c, &facility);
        } else {
            put(ctx, c);
        }
        return;
    };
    if c.order.is_none() {
        c.order = Some(id);
        put(ctx, c);
    }
    let Some(order) = work_order(ctx, id) else {
        return;
    };
    let payee = mush_industry::money::facility_payee_account(ctx, c.facility, AccountId(c.account));
    match order.state() {
        WorkOrderState::Draft => {
            let covered = c
                .bill
                .iter()
                .all(|g| pooled(ctx, c.escrow(), &g.good) >= g.units);
            if !covered {
                return;
            }
            let slots = free
                .entry(c.facility)
                .or_insert_with(|| slots_free(ctx, &facility));
            if *slots == 0 {
                // Queued behind an earlier booking: nothing this fire.
                return;
            }
            let hold = match mush_industry::money::live_hold(ctx, WorkOrderId(id)) {
                Some(hold) => Ok(hold.id),
                None => mush_industry::money::place(ctx, &order),
            };
            match hold {
                Ok(hold) => {
                    *slots -= 1;
                    submit_as(
                        ctx,
                        c.requester,
                        IndustryVerb::WoSubmit,
                        json!({ "order": WorkOrderId(id).to_string(), "hold": hold.0 }),
                        "works.submit",
                    );
                }
                Err(error) if error.is_transient() => {}
                Err(_) => {
                    // Unfunded: the facility cancels it (§4.12), the booking fee forfeit.
                    submit_as(
                        ctx,
                        c.requester,
                        IndustryVerb::WoCancel,
                        json!({ "order": WorkOrderId(id).to_string() }),
                        "works.unfunded",
                    );
                    ctx.out.info(
                        c.requester,
                        format!(
                            "Order {}: your account cannot cover the fee, so the {} cancels it.",
                            c.id,
                            c.front.word()
                        ),
                    );
                }
            }
        }
        WorkOrderState::AwaitingDelivery => {
            submit_as(
                ctx,
                c.requester,
                IndustryVerb::WoDeliver,
                json!({ "order": WorkOrderId(id).to_string() }),
                "works.deliver",
            );
        }
        WorkOrderState::Completed { .. } => {
            let now = ctx.w.world_ic_now().at;
            if c.delivered_ic.is_none() {
                c.delivered_ic = Some(now);
                let _ = mush_industry::money::settle_booking(ctx, c.id, None);
                put(ctx, c);
                ctx.out.info(
                    c.requester,
                    format!(
                        "Order {}{DELIVERED_AT}{}: `{} /collect {}` takes it.",
                        c.id,
                        c.front.word(),
                        c.front.verb(),
                        c.id
                    ),
                );
            }
            // Doc 39 cut IN.5 (§4.3): a pooled output is dissolved in the escrow first.
            let bulk = bulk_lots(&escrow_lots(ctx, c.escrow()));
            if c.owed.is_none() && !bulk.is_empty() {
                let (requester, escrow) = (c.requester, c.escrow());
                if let Err(error) = owe(
                    ctx,
                    c,
                    requester,
                    (escrow, escrow),
                    (Vec::new(), bulk),
                    Then::Pooled,
                ) {
                    tracing::warn!(contract = c.id, %error, "tacsim: a works output was not pooled");
                }
                return;
            }
            if c.house {
                house_collect(ctx, c, &facility);
                return;
            }
            charge_storage(ctx, c, &facility, payee, now);
        }
        WorkOrderState::Cancelled | WorkOrderState::Failed => {
            let forfeit = *order.state() == WorkOrderState::Cancelled;
            let _ = mush_industry::money::settle_booking(ctx, c.id, payee.filter(|_| forfeit));
            return_bill(ctx, c, &facility);
        }
        WorkOrderState::Running
        | WorkOrderState::Interrupted { .. }
        | WorkOrderState::Ready
        | WorkOrderState::AwaitingApproval { .. } => {}
    }
}

/// The contract's work order: the requester's order whose source is the escrow (a bounded walk
/// of the work-order family; a contract binds once).
fn bind(ctx: &Ctx<'_>, c: &WorksContract) -> Option<u64> {
    let (industry, _) = handles(ctx.w)?;
    let requester = PrincipalId::Object(c.requester);
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_LOT_PAGES {
        let page = ctx
            .scan_action_state(&industry.work_orders, cursor.as_deref(), LOT_PAGE)
            .ok()?;
        for entry in page.entries() {
            let order = entry.value();
            if order.requester() == &requester && order.material_source() == Some(c.escrow()) {
                return Some(order.id().0);
            }
        }
        cursor = Some(page.next_after()?.to_string());
    }
    None
}

/// A cancelled or failed order's escrow back to the requester's docked hold (the house's to its
/// stores); with nothing in it, the contract closes.
fn return_bill(ctx: &mut Ctx<'_>, c: &mut WorksContract, facility: &FacilityRecord) {
    let (goods, lots) = escrow_contents(ctx, c);
    let failed = c
        .order
        .and_then(|o| work_order(ctx, o))
        .is_some_and(|o| *o.state() == WorkOrderState::Failed);
    if goods.is_empty() && lots.is_empty() {
        close(
            ctx,
            c,
            if failed {
                Close::Failed
            } else {
                Close::Cancelled
            },
        );
        return;
    }
    let to = if c.house {
        stores(ctx, facility).ok().map(EndpointRef)
    } else {
        port_of(ctx.w, facility)
            .and_then(|entry| docked_hold(ctx.w, c.requester, &entry, false).ok())
    };
    // No hold docked: the bill waits in the escrow for `/collect`.
    let Some(to) = to else {
        return;
    };
    let (by, escrow) = (c.requester, c.escrow());
    let _ = owe(ctx, c, by, (escrow, to), (goods, lots), Then::Returned);
}

fn fail_back(ctx: &mut Ctx<'_>, c: &mut WorksContract, facility: &FacilityRecord) {
    let (goods, lots) = escrow_contents(ctx, c);
    if goods.is_empty() && lots.is_empty() {
        close(ctx, c, Close::Failed);
        return;
    }
    let to = port_of(ctx.w, facility)
        .and_then(|entry| docked_hold(ctx.w, c.requester, &entry, false).ok());
    match to {
        Some(to) => {
            let (by, escrow) = (c.requester, c.escrow());
            let _ = owe(ctx, c, by, (escrow, to), (goods, lots), Then::Returned);
        }
        None => put(ctx, c),
    }
}

fn port_of(w: &World, facility: &FacilityRecord) -> Option<CanonicalId> {
    let at = w.get(facility.object()).and_then(|o| o.location);
    crate::port::market::ports(w)
        .into_iter()
        .find(|(entry, def, _)| {
            // Doc 39 cut IN.5: or the port's authored works room is the facility itself.
            at.is_some_and(|at| crate::port::market::port_rooms(w, def).contains(&at))
                || crate::port::plant::works_room(w, entry, def).ok().flatten()
                    == Some(facility.object())
        })
        .map(|(entry, ..)| entry)
}

/// A house order's output into the house's stores (the arm collects its own, §4.12).
fn house_collect(ctx: &mut Ctx<'_>, c: &mut WorksContract, facility: &FacilityRecord) {
    let Ok(stores) = stores(ctx, facility) else {
        return;
    };
    let (goods, lots) = escrow_contents(ctx, c);
    if goods.is_empty() && lots.is_empty() {
        close(ctx, c, Close::Collected);
        return;
    }
    let (by, escrow) = (c.requester, c.escrow());
    let _ = owe(
        ctx,
        c,
        by,
        (escrow, EndpointRef(stores)),
        (goods, lots),
        Then::Collected,
    );
}

/// **Storage** (§4.12): after 28 IC days uncollected, a slot-day a day from the payer's account
/// to the facility; an account that cannot pay loses the goods to the facility's stores.
fn charge_storage(
    ctx: &mut Ctx<'_>,
    c: &mut WorksContract,
    facility: &FacilityRecord,
    payee: Option<AccountId>,
    now: i64,
) {
    let (Some(delivered), Some(payee), Some(day_fee)) =
        (c.delivered_ic, payee, facility.booking_fee_minor())
    else {
        return;
    };
    let due = now
        .saturating_sub(delivered)
        .saturating_sub(STORAGE_GRACE_SECS);
    if due < DAY_SECS {
        return;
    }
    let days = u32::try_from(due / DAY_SECS).unwrap_or(u32::MAX);
    let mut charged = false;
    while c.storage_days < days {
        let day = c.storage_days.saturating_add(1);
        match mush_industry::money::charge_storage(
            ctx,
            (c.id, day),
            &c.payer,
            AccountId(c.account),
            payee,
            day_fee,
        ) {
            Ok(()) => {
                c.storage_days = day;
                charged = true;
            }
            Err(error) if error.is_transient() => break,
            Err(_) => {
                // The account ran dry: the facility takes the goods.
                let Ok(stores) = stores(ctx, facility) else {
                    break;
                };
                let (goods, lots) = escrow_contents(ctx, c);
                ctx.out.info(
                    c.requester,
                    format!(
                        "Order {}: storage went unpaid, and the {} has taken the goods.",
                        c.id,
                        c.front.word()
                    ),
                );
                if goods.is_empty() && lots.is_empty() {
                    close(ctx, c, Close::Seized);
                } else {
                    let (by, escrow) = (c.requester, c.escrow());
                    let _ = owe(
                        ctx,
                        c,
                        by,
                        (escrow, EndpointRef(stores)),
                        (goods, lots),
                        Then::Seized,
                    );
                }
                return;
            }
        }
    }
    if charged {
        put(ctx, c);
    }
}

// ---- registration -------------------------------------------------------------------------

/// The capabilities the works reads its ledger through, and the action when it registered.
#[derive(Clone, Debug)]
pub struct WorksHandles {
    /// The registered action; `None` on a roster without the inventory cell.
    pub action: Option<DefinitionRef>,
    /// `tacsim.works_ledger`.
    pub ledgers: StateFamilyHandle<u64, WorksLedger>,
}

fn mod_error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(mod_error)?,
        DefinitionId::new(id).map_err(mod_error)?,
        1,
        payload,
    )
    .map_err(mod_error)
}

/// The outcome's descriptor.
///
/// # Errors
///
/// A descriptor that does not build.
pub fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "outcome",
        WORKS_OUTCOME,
        json!({"format":1,"fields":["actor","contract","act"]}),
    )
}

/// The action's descriptor.
///
/// # Errors
///
/// The outcome unregistered, or a descriptor that does not build.
pub fn action_descriptor(
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<DefinitionDescriptor, ModError> {
    let outcome = outcomes
        .get(WORKS_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the works move's outcome did not register"))?;
    descriptor(
        "action",
        WORKS_ACTION,
        json!({"format":1,"subject":"self_actor","targets":0,"family":WORKS_FAMILY}),
    )
    .map(|d| d.with_references(vec![outcome]))
}

/// **Register the ledger family, the provider and the handles** — the family unconditionally,
/// the provider only with the inventory cell (the mill's rule).
///
/// # Errors
///
/// Any registration the registrar refuses.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: &BTreeMap<String, DefinitionRef>,
    inventory: Option<&InventoryFamilies>,
) -> Result<(), ModError> {
    use crate::port::families::family_descriptor;
    let ledgers = reg.state_family::<u64, WorksLedger, _, _, _, _>(
        family_descriptor(
            WORKS_FAMILY,
            "tacsim.works_ledger_key.v1",
            "tacsim.works_ledger",
        )?,
        |key: &u64| Ok(key.to_string()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &WorksLedger| {
            if value.acts == 0 {
                Err("a works ledger records one landed move".to_owned())
            } else {
                Ok(())
            }
        },
        |value: &WorksLedger| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let action = match (actions.get(WORKS_ACTION), inventory) {
        (Some(action), Some(inventory)) => {
            let outcome = outcomes
                .get(WORKS_OUTCOME)
                .cloned()
                .ok_or_else(|| ModError::msg("the works move's outcome did not register"))?;
            reg.action_provider(
                action.clone(),
                WorksProvider {
                    action: action.clone(),
                    outcome,
                    ledgers: ledgers.clone(),
                    inventory: inventory.clone(),
                },
            )?;
            Some(action.clone())
        }
        _ => None,
    };
    reg.state(WorksHandles { action, ledgers });
    Ok(())
}

// ---- the provider ---------------------------------------------------------------------------

/// **`tacsim_works`**: one owed goods move of one contract, landed once under its ordinal.
struct WorksProvider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    ledgers: StateFamilyHandle<u64, WorksLedger>,
    inventory: InventoryFamilies,
}

fn invalid() -> mush_core::ActionError {
    port_error(
        ActionErrorCode::InvalidRequest,
        "the works move is self-scoped, takes no targets and names one contract and its act",
    )
}

fn parameters(
    invocation: &ActionInvocation,
) -> Result<(&Map<String, Value>, u64, u32), mush_core::ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid());
    }
    let parameters = invocation.parameters().as_object().ok_or_else(invalid)?;
    let contract = parameters
        .get("contract")
        .and_then(Value::as_u64)
        .ok_or_else(invalid)?;
    let act = parameters
        .get("act")
        .and_then(Value::as_u64)
        .and_then(|a| u32::try_from(a).ok())
        .filter(|a| *a > 0)
        .ok_or_else(invalid)?;
    Ok((parameters, contract, act))
}

/// The moves `actor` owes, keyed by contract, and the hold plans of their destinations —
/// derived from the store at projection, never from the submitter.
fn owed_rows(w: &World, actor: ObjId) -> (Map<String, Value>, Map<String, Value>) {
    let mut rows = Map::new();
    let mut plans = Map::new();
    for c in contracts(w) {
        if rows.len() >= MAX_OFFER_ROWS {
            break;
        }
        let Some(owed) = c.owed.as_ref().filter(|o| o.by == actor) else {
            continue;
        };
        if let Ok(value) = serde_json::to_value(owed) {
            if let Some(plan) = PortCapacity.plan(w, EndpointRef(VaultId(owed.to))) {
                plans.insert(c.id.to_string(), plan.0);
            }
            rows.insert(c.id.to_string(), value);
        }
    }
    // Doc 39 cut IN.5: a works' house purchase in flight, keyed by its stores vault (a vault id,
    // so never an escrow's), its destination the ungoverned stores.
    for (stores, owed) in crate::port::supply::owed_moves(w, actor) {
        if rows.len() >= MAX_OFFER_ROWS {
            break;
        }
        if let Ok(value) = serde_json::to_value(&owed) {
            rows.insert(stores.to_string(), value);
        }
    }
    // Doc 39 cut IN.10: a house ferry's stores move into her hold, keyed by the hold's vault,
    // governed by the hold's capacity.
    for (hold, owed) in crate::port::pools::owed_moves(w, actor) {
        if rows.len() >= MAX_OFFER_ROWS {
            break;
        }
        if let Ok(value) = serde_json::to_value(&owed) {
            if let Some(plan) = PortCapacity.plan(w, EndpointRef(VaultId(hold))) {
                plans.insert(hold.to_string(), plan.0);
            }
            rows.insert(hold.to_string(), value);
        }
    }
    (rows, plans)
}

impl WorksProvider {
    /// The destination's room, by the plan the offer carried (a hold the offer carried none for
    /// is ungoverned).
    fn room_for(
        &self,
        txn: &mut ActionTxn<'_>,
        plan: Option<&Value>,
        to: EndpointRef,
        incoming: &BTreeMap<CanonicalId, u64>,
    ) -> Result<(), mush_core::ActionError> {
        let Some(plan) = plan else {
            return Ok(());
        };
        let plan = CapacityPlan(plan.clone());
        for (good, quantity) in incoming {
            let mut read = |g: &CanonicalId| {
                let pooled = self
                    .inventory
                    .pool(txn, to, g)
                    .map(|pool| pool.map_or(0, |pool| pool.quantity))
                    .map_err(|_| mush_market::CapacityReadError)?;
                if !crate::port_tick::is_part(g) {
                    return Ok(pooled);
                }
                self.inventory
                    .lot_quantity(txn, to, g)
                    .map(|held| pooled.saturating_add(held))
                    .map_err(|_| mush_market::CapacityReadError)
            };
            PortCapacity
                .check(&plan, good, *quantity, &mut read)
                .map_err(|refusal| match refusal {
                    CapacityRefusal::Refused { .. } => port_error(
                        ActionErrorCode::ProviderRejected,
                        "the hold has no room for the goods",
                    ),
                    CapacityRefusal::Unreadable => port_error(
                        ActionErrorCode::ProviderRejected,
                        "the hold's room cannot be read",
                    ),
                })?;
        }
        Ok(())
    }
}

impl ActionProvider for WorksProvider {
    fn validate_invocation(
        &self,
        invocation: &ActionInvocation,
    ) -> Result<(), mush_core::ActionError> {
        parameters(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, mush_core::ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let (rows, plans) = owed_rows(&engine.world, query.actor());
        if rows.is_empty() {
            return Ok(None);
        }
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| integrity("the works move's definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| integrity("the works move's guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| integrity("the works move's definition guard is unavailable"))?;
        let integer =
            |id: &str, maximum: i64| -> Result<ProjectedParameter, mush_core::ActionError> {
                Ok(ProjectedParameter {
                    id: CanonicalId::new(id)
                        .map_err(|_| integrity("the works move's parameter id is invalid"))?,
                    required: true,
                    kind: ProjectedParameterKind::Integer {
                        minimum: 1,
                        maximum,
                    },
                })
            };
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({
                "rows": Value::Object(rows),
                "plans": Value::Object(plans),
                "now": engine.world.world_ic_now().at,
            }),
            parameters: ProjectedParameterSchema::new(vec![
                integer("contract", i64::MAX)?,
                integer("act", i64::from(u32::MAX))?,
            ])
            .map_err(|_| integrity("the works move's parameter schema is invalid"))?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "tacsim-works",
                "Works escrow",
                "The works moves an order's goods between its escrow and a hold or its stores.",
            )
            .map_err(|_| integrity("the works move's presentation is invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| integrity("the works move's offer guards are invalid"))?,
        }))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, mush_core::ActionError> {
        // 1. The row the offer carried.
        let (parameters, contract, act) = parameters(invocation)?;
        let owed: WorksAct = parameters
            .get("rows")
            .and_then(Value::as_object)
            .and_then(|rows| rows.get(&contract.to_string()))
            .cloned()
            .and_then(|row| serde_json::from_value(row).ok())
            .ok_or_else(invalid)?;
        owed.check().map_err(|_| invalid())?;
        let actor = invocation.actor();
        if owed.by != actor || owed.act != act {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that works move is not yours to submit",
            ));
        }
        // 2. The replay guard: only the ledger's next ordinal.
        let prior = txn
            .get(&self.ledgers, &contract)
            .map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the works ledgers are unavailable",
                )
            })?
            .cloned();
        if prior.as_ref().is_some_and(|l| l.contract != contract) {
            return Err(port_error(
                ActionErrorCode::Integrity,
                "a works ledger is stored under another contract's key",
            ));
        }
        if prior.as_ref().map_or(Some(1), |l| l.acts.checked_add(1)) != Some(act) {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that works move has already landed",
            ));
        }
        // 3. The move.
        let refused =
            |message: &'static str| port_error(ActionErrorCode::ProviderRejected, message);
        if owed.then == Then::Commissioned {
            self.consume(txn, &owed)?;
            return self.landed(invocation, txn, (contract, act), prior.is_some());
        }
        let from = EndpointRef(VaultId(owed.from));
        let to = EndpointRef(VaultId(owed.to));
        // Doc 39 cut IN.5: a pooling act dissolves each bulk lot where it lies; nothing moves.
        if owed.then == Then::Pooled {
            let mut goods = std::collections::BTreeSet::new();
            for lot in &owed.lots {
                let record = txn
                    .get(&self.inventory.lots, lot)
                    .map_err(|_| refused("a lot cannot be read"))?
                    .cloned()
                    .filter(|l| l.endpoint() == from)
                    .ok_or_else(|| refused("a lot is no longer where the act names it"))?;
                if crate::port_tick::is_part(record.commodity())
                    || !goods.insert(record.commodity().clone())
                {
                    return Err(refused("only one bulk lot a good is pooled in one act"));
                }
                self.inventory
                    .dissolve(txn, from, *lot)
                    .map_err(|_| refused("a lot could not be pooled"))?;
            }
        }
        let mut incoming: BTreeMap<CanonicalId, u64> = BTreeMap::new();
        let mut lots = Vec::with_capacity(owed.lots.len());
        let moving = if owed.then == Then::Pooled {
            &[][..]
        } else {
            &owed.lots[..]
        };
        for lot in moving {
            let record = txn
                .get(&self.inventory.lots, lot)
                .map_err(|_| refused("a lot cannot be read"))?
                .cloned()
                .filter(|l| l.endpoint() == from)
                .ok_or_else(|| refused("a lot is no longer where the move names it"))?;
            let slot = incoming.entry(record.commodity().clone()).or_insert(0);
            *slot = slot.saturating_add(record.quantity());
            lots.push(record);
        }
        for row in &owed.goods {
            let slot = incoming.entry(row.good.clone()).or_insert(0);
            *slot = slot.saturating_add(row.units);
        }
        self.room_for(
            txn,
            parameters
                .get("plans")
                .and_then(Value::as_object)
                .and_then(|plans| plans.get(&contract.to_string())),
            to,
            &incoming,
        )?;
        for row in &owed.goods {
            let available = self
                .inventory
                .available(txn, from, &row.good)
                .map_err(|_| refused("the goods cannot be read"))?;
            if available < row.units {
                return Err(refused("the goods are no longer there to move"));
            }
            self.inventory
                .transfer(txn, from, to, &row.good, row.units)
                .map_err(|_| refused("the goods could not be moved"))?;
        }
        for lot in lots {
            // A part moves as its lot; a bulk lot is pooled first (`Then::Pooled`: a bundle
            // mutates a pool's tally once, so it cannot be moved and dissolved in one act).
            if !crate::port_tick::is_part(lot.commodity()) {
                return Err(refused("a bulk lot is pooled before it moves"));
            }
            self.inventory
                .relocate(txn, lot.id(), from, to)
                .map_err(|_| refused("a lot could not be moved"))?;
        }
        self.landed(invocation, txn, (contract, act), prior.is_some())
    }
}

impl WorksProvider {
    /// **Doc 39 cut IN.9**: the commission's consumption — every lot still in the escrow, the
    /// kit by its `workorder.output` lineage (the relay foundry's check), consumed in one bundle.
    fn consume(
        &self,
        txn: &mut ActionTxn<'_>,
        owed: &WorksAct,
    ) -> Result<(), mush_core::ActionError> {
        let refused =
            |message: &'static str| port_error(ActionErrorCode::ProviderRejected, message);
        let escrow = EndpointRef(VaultId(owed.from));
        let mut lots = Vec::with_capacity(owed.lots.len());
        for lot in &owed.lots {
            let record = txn
                .get(&self.inventory.lots, lot)
                .map_err(|_| refused("a lot cannot be read"))?
                .cloned()
                .filter(|l| l.endpoint() == escrow)
                .ok_or_else(|| refused("a lot is no longer in the escrow"))?;
            if crate::port::commission::is_kit(record.commodity())
                && !matches!(record.lineage(), mush_inventory::Lineage::Origin { source }
                    if source.provider == "industry" && source.kind == "workorder.output")
            {
                return Err(refused("the kit did not come from a work order"));
            }
            lots.push(record);
        }
        self.inventory
            .consume_products(txn, &lots)
            .map_err(|_| refused("a lot could not be consumed"))?;
        Ok(())
    }

    /// The ledger and the outcome of a landed move.
    fn landed(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (contract, act): (u64, u32),
        prior: bool,
    ) -> Result<AuthoritativeActionOutput, mush_core::ActionError> {
        let actor = invocation.actor();
        // 4. The ledger.
        let ledger = WorksLedger {
            contract,
            acts: act,
        };
        let staged = if prior {
            txn.replace(&self.ledgers, &contract, ledger)
        } else {
            txn.insert(&self.ledgers, &contract, ledger)
        };
        staged.map_err(|_| {
            port_error(ActionErrorCode::Integrity, "the works ledger did not stage")
        })?;
        // 5. The outcome.
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({"actor": actor.0, "contract": contract, "act": act}),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the works move's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn act() -> WorksAct {
        WorksAct {
            act: 1,
            by: ObjId(5),
            from: 10,
            to: 11,
            goods: vec![Goods {
                good: CanonicalId::new("structural_alloys").expect("an id"),
                units: 3,
            }],
            lots: Vec::new(),
            then: Then::Supplied,
            tries: 1,
        }
    }

    #[test]
    fn in3_a_move_names_two_endpoints_an_ordinal_and_something() {
        assert!(act().check().is_ok());
        assert!(WorksAct { act: 0, ..act() }.check().is_err());
        assert!(WorksAct { to: 10, ..act() }.check().is_err());
        assert!(
            WorksAct {
                goods: Vec::new(),
                ..act()
            }
            .check()
            .is_err()
        );
        let wire = serde_json::to_value(act()).expect("serializes");
        assert_eq!(
            serde_json::from_value::<WorksAct>(wire).expect("reads back"),
            act()
        );
    }

    #[test]
    fn in3_storage_is_free_for_28_days_then_a_day_at_a_time() {
        assert_eq!(STORAGE_GRACE_SECS, 28 * DAY_SECS);
        assert_eq!(days_of(480), 20);
        assert_eq!(days_of(6), 1);
    }
}
