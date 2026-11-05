//! **The bay's repair actions, berth and stow** (doc 36 cut GC.8a; Q-36-32, Q-36-33, Q-36-35,
//! Q-36-40, Q-36-53's acts).
//!
//! - **Berth and stow** (Q-36-53): a hull leaves a docked carrier's bay slot for the port's
//!   workshop room (its location becomes the room, its slot frees, the carrier may sail without
//!   it) and goes back the same way. The workshop room is GC.7's `bay_refit` facility, so a
//!   berthed hull is at the facility the built refit bay reads (`bay_refit/authority.rs`).
//! - **The repair order** (Q-36-40): a new authoritative action beside `bay_refit`, for work
//!   that touches cells and conditions only — armour, a rebuilt section, a mended, unjammed or
//!   replaced part (like for like), a recharged beam magazine. It is quoted from GC.7's tariff
//!   at the operator's multiplier ([`crate::port::bay_work::jobs::quote`]), the fee escrowed on acceptance in a
//!   contract the facility's operator accepts when the work is done, and the work runs on the
//!   IC clock, which stops under a W-clock freeze (Q-36-32). The hull is held meanwhile: no
//!   berth, stow, move, carry or launch, and at completion the work is refused (the fee
//!   released) if the hull moved or its unit changed.
//! - **Facility classes** (Q-36-35, §4a): a carrier's bay does minor work at twice the time and
//!   the same cost; a workshop does everything. Each quoted job names the class it needs.
//! - **Who may** (Q-36-33): [`crate::port::bay_work::permits`]. The title holder and a holder of a current
//!   `unit.maintain_refit` grant may repair, rearm and refit; a lessee repairs and rearms; a
//!   salvor with custody repairs at their own risk. None but the title holder scraps. While a
//!   hull is in another's live salvage custody, the custodian alone acts (doc 36a P1-2), and a
//!   hull leaves a carrier's bay only by the hand of one who has the run of that carrier.
//!
//! - **The hold** (GC.8b; Q-36-31, Q-36-36): an order's munitions and `industrial_parts` come
//!   from the carrier's hold — the carrier whose bay the hull stands in, or the carrier it was
//!   berthed from, docked at the facility's port — checked when the order is taken and drawn,
//!   all or nothing, when the work is done (the draw of [`crate::port::bay_work::act`]). A
//!   part may be fitted from a lot in that hold (`+bay/fit`), refused when its provenance is
//!   hot (Q-36-37).
//! - **Strip and scrap** (GC.8b; Q-36-37, Q-36-39; [`crate::port::bay_work::strip`]): a part
//!   taken off becomes a lot in the hold with its provenance; a scrapped hull becomes
//!   `structural_alloys` and a lot per salvageable part.
//!
//! - **Away from port** (doc 41 cut RS.2; Q-41-5..7, Q-38-16): a carrier's bay works wherever
//!   she is not docked at a port with a workshop — on a site, under way, at a port with none —
//!   for the minor jobs at [`crate::port::bay_work::CARRIER_TIME_FACTOR`], from her own hold, and fits a traced part
//!   from that hold (`+bay/fit`, Q-41-6). Nobody is paid there: no fee contract is filed, the
//!   time is the price (Q-41-7, **expensive to reverse**). A rebuild or a replace is refused
//!   with the line naming the workshops, nearest first by the lane. Under a field on her site
//!   an order waits, its time left kept, and resumes when the site settles.
//!
//! Docked at a port with a workshop, a carrier bay's operator is paid; the workshop supplies a
//! replaced part, the hold everything else.

pub mod act;
pub mod command;
pub mod jobs;
pub mod record;
pub mod scrap;
pub mod strip;

pub use jobs::{BayJob, BayQuote, CARRIER_TIME_FACTOR, FacilityClass, QuotedJob};
pub use record::{
    BAY_PREFIX, BAY_SCHEMA, Bale, BaleDraw, BayHull, BayHullState, BayOrder, BerthRow, ClaimLot,
    KG_PER_T, MAX_CLAIM_LOTS, MAX_ORDER_JOBS, OrderStatus, Right, SHOP_BALE_PREFIX, ShopBale,
    VacatedLot, bay_key, bay_key_owner, shop_bale_key,
};

use std::collections::BTreeMap;

use mush_core::{CanonicalId, Ctx, EventHook, HookReadContext, ObjId, World};
use mush_economy::{
    AcceptOutcome, AccountId, Amount, ConsiderationSpec, ContractId, ContractRoles, ContractSpec,
    EconomicClock, EconomicDeadline, EconomicVisibility, EconomyStore, EvidenceKind, Expiry,
    OutcomeTerms, PayoutShare, Share, TypedCondition, VaultId,
};
use mush_inventory::LotId;
use mush_inventory::record::EndpointRef;
use mush_org::unit_authority::{
    UnitCapability,
    reader::{UnitAuthorityRequest, UnitAuthorityResponse},
};
use mush_org::{OrgId, PrincipalId};
use mush_proto::RichText;
use mush_sim::{SpatialParent, SpatialState};
use mush_tactics::TacticalUnit;
use mush_tactics::field_ops::{ComponentCondition, CustodyTitleStatus, SalvageClaimId};
use mush_tactics::unit::ComponentId;
use smol_str::SmolStr;

use act::{GoodUnits, OwedAct, OwedKind};

use crate::detachment::planning::PlanningHandles;
use crate::port::def::PortDef;
use crate::port::map::PortMap;
use crate::store::{TacsimStore, put_bay_hull};

/// **What a right may do** (Q-36-33).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Armour, rebuild, mend, unjam, replace like for like.
    Repair,
    /// Load rounds.
    Rearm,
    /// A part change to another design (`bay_refit`).
    Refit,
    /// Take a part off.
    Strip,
    /// Break the hull up.
    Scrap,
    /// Move the hull between a carrier's bay and a workshop.
    Berth,
}

/// **Q-36-33's table.** Title: everything. A `maintain_refit` grant: everything but scrap,
/// which ends the title. A lessee: repair and rearm (and berth, to get there), never refit,
/// strip or scrap. A salvor with custody: repair at their own risk (Q-S14), never strip or
/// scrap (Q-F5).
#[must_use]
pub const fn permits(right: Right, act: Act) -> bool {
    match right {
        Right::Title => true,
        Right::Grant => !matches!(act, Act::Scrap),
        Right::Lessee => matches!(act, Act::Repair | Act::Rearm | Act::Berth),
        Right::Salvor => matches!(act, Act::Repair | Act::Berth),
        Right::Retrieval => matches!(act, Act::Berth),
    }
}

/// The fee contract's name.
pub const FEE_NAME: &str = "tacsim.bay.repair";
/// How long a fee contract stands before it refunds itself, real seconds (thirty days): far
/// past any order's IC time at any rate the game runs, so it only ever ends by the work or a
/// cancel. Provisional.
pub const FEE_WINDOW_SECS: i64 = 30 * 86_400;

/// Every refusal the acts share.
pub const NO_HULL: &str = "There is no such hull here.";
/// The hull has an unreadable bay record.
pub const HELD_DARK: &str = "That hull's bay record cannot be read; staff repair it first.";
/// An order is open.
pub const HELD_OPEN: &str = "That hull is held for work in progress; +bay/cancel it first.";
/// No right.
pub const NO_RIGHT: &str = "You hold no right to have that hull worked on.";
/// An inventory act is still owed.
pub const SETTLING: &str =
    "That hull's last work is still settling in the bay's stores; try again shortly.";
/// The good a section's points consume (Q-36-31).
pub const PARTS_GOOD: &str = "industrial_parts";
/// **`NO_PART`** (doc 39 cut IN.6, Q-39-11): an empty store refuses a replace, as `NO_HULL`
/// refuses the desk. `{part}` is the part row's label.
pub const NO_PART: &str =
    "The workshop has no {part} in its stores; +market/list the yard, or bring one in the hold.";
/// A replace order fits one part from the stores (doc 39 cut IN.6).
pub const ONE_STORE_PART: &str =
    "The workshop fits one part from its stores an order; order the other replaces after it.";
/// The structural good a repair's bale holds beside the fittings (doc 39 cut IN.1, §4.10).
pub const ALLOYS_GOOD: &str = "structural_alloys";

// ---- where the work is done -------------------------------------------------------------

/// **A facility a hull stands in** (Q-36-35): its class, the room, and the port whose
/// workshop's operator is paid and supplies the parts — none for a carrier's bay away from port
/// (doc 41 cut RS.2).
#[derive(Clone, Debug)]
pub struct Facility {
    /// The class.
    pub class: FacilityClass,
    /// The room the work is done in: the workshop room, or the carrier's bay room.
    pub room: ObjId,
    /// The carrier, for a carrier bay.
    pub carrier: Option<ObjId>,
    /// The port entry whose workshop serves; `None` for a carrier's bay away from port.
    pub port: Option<CanonicalId>,
    /// The operator org and its scoped credit account: who is paid. `None` away from port,
    /// where nobody is (Q-41-7).
    pub operator: Option<(OrgId, AccountId)>,
    /// The operator's multiplier, per mille (1,000 away from port: the tariff's own time).
    pub multiplier_permille: u32,
    /// Doc 39 cut IN.4: the port workshop's cost model and its berths, when authored — the
    /// floor a quote prints.
    pub cost: Option<(mush_industry::FacilityCostDef, u32)>,
    /// Doc 41 cut RS.2: the site her carrier stands on away from port, when she stands on one —
    /// a field open there holds the bay's work (Q-41-5).
    pub site: Option<CanonicalId>,
}

impl Facility {
    /// Whether this is a carrier's bay away from port (doc 41 cut RS.2): no operator, no fee,
    /// no store.
    #[must_use]
    pub const fn away(&self) -> bool {
        self.port.is_none()
    }
}

/// The port workshop's cost model and berths (doc 39 cut IN.4).
fn workshop_cost(def: &PortDef) -> Option<(mush_industry::FacilityCostDef, u32)> {
    def.workshop
        .as_ref()
        .and_then(|workshop| workshop.cost.map(|cost| (cost, workshop.berths)))
}

/// Every port with a workshop, its room on this world and its map.
fn workshops(w: &World) -> Vec<(CanonicalId, &PortDef, &PortMap, ObjId)> {
    crate::port::market::ports(w)
        .into_iter()
        .filter_map(|(id, def, map)| {
            let map = map?;
            let room = crate::port::workshop::room(w, &id, def).ok()??;
            Some((id, def, map, room))
        })
        .collect()
}

fn operator_of(def: &PortDef, map: &PortMap) -> Option<(OrgId, AccountId, u32)> {
    let workshop = def.workshop.as_ref()?;
    let row = map.orgs.get(&workshop.operator)?;
    Some((
        OrgId(row.org),
        AccountId(row.account?),
        workshop.multiplier_permille,
    ))
}

/// **The workshop's stores** (doc 39 cut IN.6, Q-39-11): the operator org's storefront at
/// `facility`'s port, the same vault its stock rows and its bids fill.
fn stores_of<'w>(w: &'w World, facility: &Facility) -> Option<&'w crate::port::map::VendorRow> {
    let port = facility.port.as_ref()?;
    let (_, def, map) = crate::port::market::ports(w)
        .into_iter()
        .find(|(id, _, _)| id == port)?;
    map?.vendors.get(&def.workshop.as_ref()?.operator)
}

/// **The part a slot stocks as and its catalogue price** (doc 39 cut IN.1, IN.6): the good, its
/// label and its reference, minor units.
fn part_price(
    w: &World,
    hull: ObjId,
    component: &ComponentId,
) -> Option<(CanonicalId, String, u64)> {
    let definition = mush_tactics::module::unit_of(w, hull)?
        .component(component)?
        .definition
        .clone();
    let row = crate::hardware::bay::part_of(w, &definition)?;
    let good = w
        .module_state::<TacsimStore>()?
        .goods()?
        .good(&row.commodity)?
        .clone();
    Some((
        row.commodity,
        good.label.clone(),
        good.reference_major.saturating_mul(100),
    ))
}

/// **The lot a replace of `component` takes from the stores** (doc 39 cut IN.6): the
/// lowest-grade clean lot of the slot's row there, with its price.
///
/// # Errors
///
/// [`NO_PART`] naming the part, or the sentence for a workshop with no stores.
pub(crate) fn store_part(
    ctx: &Ctx<'_>,
    hull: ObjId,
    facility: &Facility,
    component: &ComponentId,
) -> Result<(crate::port::stock::StoreLot, u64, u64), String> {
    let (good, label, price) = part_price(ctx.w, hull, component)
        .ok_or_else(|| NO_PART.replace("{part}", component.as_str()))?;
    let row = stores_of(ctx.w, facility).ok_or("That workshop keeps no stores.")?;
    crate::port::stock::store_lots(ctx, row, &good, &claimed_store_lots(ctx.w))
        .first()
        .map(|lot| (*lot, row.vault, price))
        .ok_or_else(|| NO_PART.replace("{part}", &label))
}

/// **The store lots open bay orders have taken** (doc 39 cut IN.6): the clerk lists none of
/// them again, and no second order takes one.
#[must_use]
pub fn claimed_store_lots(w: &World) -> std::collections::BTreeSet<LotId> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return std::collections::BTreeSet::new();
    };
    store
        .bay_open_hulls()
        .into_iter()
        .filter_map(|hull| match store.bay_hull(hull) {
            Some(BayHullState::Present(row)) => row.open().cloned(),
            _ => None,
        })
        .filter(|order| order.part_vault.is_some())
        .flat_map(|order| {
            order.jobs.into_iter().filter_map(|j| match j.job {
                BayJob::Replace { lot, .. } => lot,
                _ => None,
            })
        })
        .collect()
}

/// **Every hull a bay may belong to** (doc 41 cut RS.2, round-2 item 41-16): the hulls the port
/// maps record and the yards' hulls their site rows record — the two sources
/// `crate::surface::recovery::world_hulls` reads, read here off every map without resolving
/// the yards. **The seam for doc 39's IN.9**: its `tacsim.owned_hulls` register joins these two
/// here once it lands, and nothing else in the bay changes.
fn bay_carriers(w: &World) -> Vec<ObjId> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Vec::new();
    };
    store
        .port_maps()
        .flat_map(|map| {
            let yard = map
                .site
                .as_ref()
                .and_then(|site| site.yard.as_ref())
                .map(|yard| yard.hulls.as_slice())
                .unwrap_or_default();
            map.hulls.iter().chain(yard).map(|row| ObjId(row.hull))
        })
        // Doc 39 cut IN.9: then the commissioned ships, after every programme and yard row.
        .chain(crate::port::owned::ships(w))
        .collect()
}

/// **The carrier whose vehicle bay is `room`**: every hull [`bay_carriers`] names, asked for
/// its bay rooms.
fn carrier_of_bay(w: &World, room: ObjId) -> Option<ObjId> {
    bay_carriers(w).into_iter().find(|hull| {
        crate::port::ship::hull_view(w, *hull)
            .is_some_and(|view| crate::port::vehicle::bay_rooms(w, &view).contains(&room))
    })
}

/// **The site `carrier` stands on** (doc 41 cut RS.2): the site port she is docked at, else the
/// site her bay is berthed on (`crate::surface::tick::hull_view_berth`), else none.
fn site_of_carrier(w: &World, carrier: ObjId) -> Option<CanonicalId> {
    match crate::port::ship::berth_of(w, carrier) {
        crate::port::ship::Berth::Docked { map, def } if def.site.is_some() => {
            Some(map.entry.clone())
        }
        crate::port::ship::Berth::Docked { .. } | crate::port::ship::Berth::UnderWay => None,
        crate::port::ship::Berth::Elsewhere => {
            crate::surface::tick::hull_view_berth(w, carrier).map(|(entry, ..)| entry)
        }
    }
}

/// **The workshops that may do what a carrier's bay may not** (doc 41 cut RS.2, Q-36-35): every
/// port with a workshop room, as `the <room> at <port>`, nearest first by the lane — the fewest
/// route hops from `from` (a port entry), then the lane's order; the lane's order alone when
/// `from` is none (a carrier under way).
fn workshop_names(w: &World, from: Option<&CanonicalId>) -> Vec<String> {
    let ports = crate::port::market::ports(w);
    // Hops by venue, breadth first over every port's routes.
    let mut hops: BTreeMap<CanonicalId, u32> = BTreeMap::new();
    let start = from.and_then(|entry| {
        ports
            .iter()
            .find(|(id, _, _)| id == entry)
            .map(|(_, def, _)| def.venue.clone())
    });
    if let Some(start) = start {
        let mut frontier = vec![start.clone()];
        hops.insert(start, 0);
        let mut depth = 0_u32;
        while !frontier.is_empty() && depth < 16 {
            depth = depth.saturating_add(1);
            let mut next = Vec::new();
            for venue in &frontier {
                let Some((_, def, _)) = ports.iter().find(|(_, def, _)| def.venue == *venue) else {
                    continue;
                };
                for to in def.routes.keys() {
                    if !hops.contains_key(to) {
                        hops.insert(to.clone(), depth);
                        next.push(to.clone());
                    }
                }
            }
            frontier = next;
        }
    }
    let mut found: Vec<(u32, usize, String)> = workshops(w)
        .into_iter()
        .enumerate()
        .map(|(at, (_, def, _, room))| {
            let room = w
                .get(room)
                .map_or_else(|| "workshop".to_owned(), |o| o.name.clone());
            let port = crate::port::coop::port_name(def);
            let far = hops.get(&def.venue).copied().unwrap_or(u32::MAX);
            (far, at, format!("the {room} at {port}"))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, _, name)| name).collect()
}

/// **The refusal for work `facility` may not do** (Q-36-35): docked, berth at the port's
/// workshop; away from port (doc 41 cut RS.2), what is wrong and the workshops that would do
/// it, nearest first — `Salt Moth's cannon is destroyed: a workshop must replace it — the
/// <room> at Ceres Freeport, or the <room> at Stickney Yards.`
fn workshop_refusal(w: &World, hull: ObjId, facility: &Facility, job: &BayJob) -> String {
    if !facility.away() {
        return "That work needs a workshop; berth the hull at the port's workshop.".to_owned();
    }
    let name = crate::port::coop::name_of(w, hull);
    let short = job.target().rsplit('.').next().unwrap_or(job.target());
    let what = match job {
        BayJob::Replace { .. } => {
            format!("{name}'s {short} is destroyed: a workshop must replace it")
        }
        BayJob::Rebuild { .. } => {
            format!("{name}'s {short} is wrecked: a workshop must rebuild it")
        }
        _ => format!("{name}'s {} needs a workshop", job.describe()),
    };
    let names = workshop_names(w, facility.site.as_ref());
    match names.split_last() {
        None => format!("{what}; no workshop on the lane takes the work."),
        Some((last, [])) => format!("{what} — {last}."),
        Some((last, rest)) => format!("{what} — {}, or {last}.", rest.join(", ")),
    }
}

/// The bay room and slot a hull stands in, when it stands in a space's interior.
fn interior_of(w: &World, hull: ObjId) -> Option<(ObjId, Option<mush_sim::SlotId>)> {
    match w.module_state::<SpatialState>()?.parent_of(hull) {
        SpatialParent::Interior { space, slot } => Some((space, slot)),
        _ => None,
    }
}

/// **Where `hull` would be worked on now** (Q-35, Q-53): a workshop room it is berthed in, or
/// the bay of a carrier docked at a port with a workshop.
///
/// # Errors
///
/// The sentence for a hull in neither.
pub fn facility_of(w: &World, hull: ObjId) -> Result<Facility, String> {
    let here = w.get(hull).and_then(|o| o.location);
    for (port, def, map, room) in workshops(w) {
        if here == Some(room) {
            let (operator, operator_account, multiplier_permille) =
                operator_of(def, map).ok_or("That workshop has no operator to take the order.")?;
            return Ok(Facility {
                class: FacilityClass::Workshop,
                room,
                carrier: None,
                port: Some(port),
                operator: Some((operator, operator_account)),
                multiplier_permille,
                cost: workshop_cost(def),
                site: None,
            });
        }
    }
    if let crate::port::ship::Berth::Docked { map, def } = crate::port::ship::berth_of(w, hull)
        && let Some(room) = crate::port::workshop::room(w, &map.entry, def)
            .ok()
            .flatten()
        && let Some((operator, operator_account, multiplier_permille)) = operator_of(def, map)
    {
        return Ok(Facility {
            class: FacilityClass::Workshop,
            room,
            carrier: Some(hull),
            port: Some(map.entry.clone()),
            operator: Some((operator, operator_account)),
            multiplier_permille,
            cost: workshop_cost(def),
            site: None,
        });
    }
    let (bay, _) = interior_of(w, hull).ok_or("That hull is in no bay and no workshop.")?;
    let carrier = carrier_of_bay(w, bay).ok_or("That hull is in no carrier's bay.")?;
    if let crate::port::ship::Berth::Docked { map, def } = crate::port::ship::berth_of(w, carrier)
        && let Some((operator, operator_account, multiplier_permille)) = operator_of(def, map)
    {
        return Ok(Facility {
            class: FacilityClass::CarrierBay,
            room: bay,
            carrier: Some(carrier),
            port: Some(map.entry.clone()),
            operator: Some((operator, operator_account)),
            multiplier_permille,
            cost: workshop_cost(def),
            site: None,
        });
    }
    // Doc 41 cut RS.2 (Q-41-5): the fourth case — her carrier on a site, under way or at a port
    // with no workshop. The minor jobs from her own hold, nobody paid (Q-41-7).
    Ok(Facility {
        class: FacilityClass::CarrierBay,
        room: bay,
        carrier: Some(carrier),
        port: None,
        operator: None,
        multiplier_permille: 1_000,
        cost: None,
        site: site_of_carrier(w, carrier),
    })
}

/// **The field open on the site a carrier's bay stands on, away from port** (doc 41 cut RS.2,
/// Q-41-5; Q-CF-30): the bay's work waits under it as a surface act does. Her bay's rigs are
/// not in the field (GC.3's rule); only the clock of the work stops.
fn field_over(ctx: &Ctx<'_>, facility: &Facility) -> Option<crate::surface::site_record::FieldId> {
    let site = facility.site.as_ref()?;
    crate::surface::tow::record_or_new(ctx, site).open_field()
}

// ---- who may ----------------------------------------------------------------------------

/// **A salvor's custody of `hull`** (Q-36-33, Q-S14): the custody record a load into `actor`'s
/// carrier wrote (doc 36 cut GC.5, `crate::surface::salvage`), when `actor` holds it. A lease rig
/// written off since is Occator's alone, and nobody holds it (Q-36-29).
#[must_use]
pub fn salvor_custody(
    ctx: &Ctx<'_>,
    actor: ObjId,
    hull: ObjId,
) -> Option<crate::surface::salvage::SalvageCustody> {
    crate::surface::salvage::custody_of(ctx, hull).filter(|custody| custody.holder == actor)
}

/// Whether `actor` holds the hull's title outright, as the bay reads a title (doc 36a P1-2:
/// `mush_economy::unit_authority::bay_hull_title_stands` — unscoped, unfrozen, unstolen, active
/// and unencumbered; the grant half of that predicate is [`right_of`]'s grant arm).
fn holds_title(w: &World, actor: ObjId, hull: ObjId) -> bool {
    w.module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().asset(hull))
        .is_some_and(|asset| {
            asset.title_owner == PrincipalId::Object(actor)
                && mush_economy::unit_authority::bay_hull_title_stands(w, asset, actor)
        })
}

/// **Whether `actor` has the run of `carrier`** (doc 36a P1-2): they hold her helm (a charter's
/// tenant, the fleet's commander, a titled programme hull's owner) or operate her from a yard.
fn carrier_right(w: &World, actor: ObjId, carrier: ObjId) -> bool {
    crate::port::coop::helm_holder(w, carrier) == Some(actor)
        || crate::port::yard::operates(w, actor, carrier)
        // Doc 39 cut IN.9: a commissioned carrier titled to them or to an org they act for.
        || (crate::port::owned::ship_row(w, carrier).is_some()
            && crate::port::owned::titled_for(w, carrier, actor))
}

/// **The right a third party `actor` holds over `hull`**, as far as the bay can read it for
/// someone other than `ctx.actor`: title, a live lease, a salvor's custody — never a grant,
/// which is Org's owner read and answers for the command's own caller alone, so a berther's
/// grant cannot be read on their behalf (that berther stows her themself).
fn right_held_by(ctx: &Ctx<'_>, actor: ObjId, hull: ObjId) -> Option<Right> {
    let custody = crate::surface::salvage::custody_of(ctx, hull);
    if custody.as_ref().is_some_and(|row| row.holder != actor) {
        return None;
    }
    if holds_title(ctx.w, actor, hull) {
        return Some(Right::Title);
    }
    if leases(ctx.w, actor, hull) {
        return Some(Right::Lessee);
    }
    if custody.is_none() && crate::surface::retrieval::held_by(ctx.w, hull, actor).is_some() {
        return Some(Right::Retrieval);
    }
    custody.map(|_| Right::Salvor)
}

/// **Whether a berthed hull's carrier still serves her** (doc 36a P1-2): the hold a berthed
/// hull draws from and mints into, and the bay a stow returns her to, are the carrier she came
/// off — hers to use only while `actor` has the run of that carrier, or the one who berthed
/// her still holds a right over her that the bay can read for them ([`right_held_by`]: title,
/// lease or custody). A salvor's carrier whose claim has since been vacated serves the prior
/// owner nothing: no stow into her bay, no lot into her hold, no draw from it. A hull not
/// berthed is served by the bay it stands in.
///
/// # Errors
///
/// The sentence for a berth whose carrier is nobody's here to use.
fn berth_stands(ctx: &Ctx<'_>, actor: ObjId, record: &BayHull) -> Result<(), String> {
    let Some(berth) = &record.berth else {
        return Ok(());
    };
    let Some(carrier) = berth.carrier else {
        // A yard berth: no carrier to serve her, nothing to refuse.
        return Ok(());
    };
    if carrier_right(ctx.w, actor, carrier) || right_held_by(ctx, berth.by, record.hull).is_some() {
        return Ok(());
    }
    Err(
        "Her berth's carrier is not yours to use, and whoever berthed her holds no right over \
         her now; a carrier of your own must take her."
            .to_owned(),
    )
}

/// Whether Org's unit authority allows `ctx.actor` `capability` on `hull` now: the grant
/// registry read through its owner (no World mirror exists).
fn unit_grant(ctx: &Ctx<'_>, hull: ObjId, capability: UnitCapability) -> bool {
    let Some(reader) = ctx
        .w
        .module_state::<PlanningHandles>()
        .and_then(|h| h.unit_authority_reader.as_ref())
    else {
        return false;
    };
    matches!(
        ctx.read_owner(
            reader,
            UnitAuthorityRequest::Authorize {
                unit: hull,
                capability,
                delegate: false,
                mission: None,
            }
        ),
        Ok(UnitAuthorityResponse::Authority { allowed: true })
    )
}

/// Whether `actor` is the live lessee of the rig `hull` — or, for a yard's house rig, its
/// dispatcher, whom the house view names its pilot (doc 41 cut RS.2: the yard's rearm through
/// the bay; `crate::port::yard::rig_lease`, the one lessee reader).
fn leases(w: &World, actor: ObjId, hull: ObjId) -> bool {
    let active = |l: &crate::port::tv::TvLease| {
        l.pilot == actor && l.status == crate::port::tv::TvLeaseStatus::Active
    };
    w.module_state::<TacsimStore>()
        .is_some_and(|s| s.tv_leases_on(hull).any(active))
        || (crate::port::yard::is_yard_rig(w, hull)
            && crate::port::yard::rig_lease(w, hull).is_some_and(|l| active(&l)))
}

/// **The right `ctx.actor` holds over `hull`** (Q-36-33), strongest first: title, a
/// `maintain_refit` grant, a live lease, a salvor's custody. **While another holds the hull in
/// live salvage custody** (doc 36a P1-2; doc 36 GC.5 "custody is not title"), nobody else's
/// title, grant or lease reaches the bay: the custodian alone acts, as the salvor (Q-S14), and
/// the prior owner answers for her again only when the claim is vacated or awarded.
#[must_use]
pub fn right_of(ctx: &Ctx<'_>, hull: ObjId) -> Option<Right> {
    let actor = ctx.actor;
    let custody = crate::surface::salvage::custody_of(ctx, hull);
    if custody.as_ref().is_some_and(|row| row.holder != actor) {
        return None;
    }
    if holds_title(ctx.w, actor, hull) {
        return Some(Right::Title);
    }
    // Doc 39 cut IN.9 (§4.13): a commissioned hull titled to an organization the actor acts for
    // is theirs to work as its title holder's officer.
    if crate::port::owned::owned(ctx.w, hull).is_some()
        && crate::port::owned::titled_for(ctx.w, hull, actor)
    {
        return Some(Right::Title);
    }
    if unit_grant(ctx, hull, UnitCapability::MaintainRefit) {
        return Some(Right::Grant);
    }
    if leases(ctx.w, actor, hull)
        || crate::port::coop::helm_of(ctx.w, actor).is_some_and(|h| h.hull == hull)
    {
        return Some(Right::Lessee);
    }
    if custody.is_none() && crate::surface::retrieval::held_by(ctx.w, hull, actor).is_some() {
        return Some(Right::Retrieval);
    }
    custody.map(|_| Right::Salvor)
}

/// The right that lets `right` do every job in `jobs` (repair and rearm separately).
fn allowed(right: Right, jobs: &[QuotedJob]) -> Result<(), String> {
    for job in jobs {
        let act = if job.job.is_repair() {
            Act::Repair
        } else {
            Act::Rearm
        };
        if !permits(right, act) {
            return Err(format!(
                "Your right over that hull does not reach `{}`.",
                job.job.describe()
            ));
        }
    }
    Ok(())
}

// ---- the record -------------------------------------------------------------------------

/// The hull's record, a fresh one when none, or the refusal for an unreadable one.
fn record_of(w: &World, hull: ObjId) -> Result<BayHull, &'static str> {
    match w
        .module_state::<TacsimStore>()
        .and_then(|s| s.bay_hull(hull))
    {
        None => Ok(BayHull::new(hull)),
        Some(BayHullState::Present(row)) => Ok(row.clone()),
        Some(BayHullState::Unreadable) => Err(HELD_DARK),
    }
}

/// **Owe the part commission** (doc 39 cut IN.2, §4.11): the industry lot `source` in `vault`,
/// carrying `lineage`, is consumed and laid back as one identified lot a unit, each with its
/// provenance row (`Manufactured` for a work order's delivery, `Stripped` for a refit's
/// removed part), in one bundle under `ledger`'s ordinal replay — the facility's object for a
/// work order, the refit hull for its removed part. The port tick submits it as `by`, retries
/// a refusal and pages the last. Each lot's id is derived from the ledger, the act and the
/// unit, so a replay names the same lots.
///
/// # Errors
///
/// The refusal, as a sentence: a ledger that owes another act or cannot be read, or a source
/// of no units or more than one act carries.
#[allow(clippy::too_many_arguments)]
pub fn owe_commission(
    ctx: &mut Ctx<'_>,
    ledger: ObjId,
    by: ObjId,
    vault: VaultId,
    source: (LotId, &CanonicalId, u64),
    lineage: act::CommissionSource,
    part: impl Fn(LotId) -> act::PartProvenance,
) -> Result<Vec<LotId>, String> {
    let mut record = record_of(ctx.w, ledger).map_err(str::to_owned)?;
    if record.owed.is_some() {
        return Err(SETTLING.to_owned());
    }
    let (lot, good, units) = source;
    if units == 0 || units > act::MAX_ACT_ROWS as u64 {
        return Err("A commission lays one to 32 parts.".to_owned());
    }
    let act_no = next_act(ctx, &record);
    let mut lots = Vec::new();
    for row in 0..units {
        let id = act::lot_id(ledger, act_no, usize::try_from(row).unwrap_or(usize::MAX))?;
        lots.push(act::LotMint {
            lot: id,
            commodity: good.clone(),
            quantity: 1,
            grade: crate::hardware::bay::part_grade(true),
            part: Some(part(id)),
        });
    }
    let ids = lots.iter().map(|l| l.lot).collect();
    record.acts = act_no;
    record.owed = Some(Box::new(OwedAct {
        act: act_no,
        by,
        vault: Some(vault.0),
        kind: OwedKind::Commission {
            source: lot,
            lineage,
            lots,
        },
        tries: 0,
    }));
    record.check()?;
    put_bay_hull(ctx, &record);
    Ok(ids)
}

/// **The ordinal the next owed act takes** (doc 36a §5.1): one past the hull's ledger when it
/// reads — the ledger, not the record, is what the prepare accepts — else one past the
/// record's own count. A record that ran ahead of its ledger (an act given up) heals here.
fn next_act(ctx: &Ctx<'_>, record: &BayHull) -> u32 {
    act::landed(ctx, record.hull)
        .unwrap_or(record.acts)
        .saturating_add(1)
}

/// **Whether `hull` is held** (Q-36-32): an open order, a berth (it leaves only by stow), a
/// scrapped shell (doc 36a P1-7: the remains are the workshop's, not a thing to carry off), or
/// an unreadable record.
#[must_use]
pub fn held(w: &World, hull: ObjId) -> bool {
    match w
        .module_state::<TacsimStore>()
        .and_then(|s| s.bay_hull(hull))
    {
        None => false,
        Some(BayHullState::Present(row)) => {
            row.open().is_some() || row.berth.is_some() || row.scrapped_ic.is_some()
        }
        Some(BayHullState::Unreadable) => true,
    }
}

/// **Whether `hull` has been scrapped** (doc 36a P1-7): its shell answers to no `+bay` word
/// but its number.
#[must_use]
pub fn scrapped(w: &World, hull: ObjId) -> bool {
    matches!(
        w.module_state::<TacsimStore>().and_then(|s| s.bay_hull(hull)),
        Some(BayHullState::Present(row)) if row.scrapped_ic.is_some()
    )
}

/// **Whether `hull` has an open repair order** (the launch gate reads this).
#[must_use]
pub fn under_work(w: &World, hull: ObjId) -> bool {
    match w
        .module_state::<TacsimStore>()
        .and_then(|s| s.bay_hull(hull))
    {
        None => false,
        Some(BayHullState::Present(row)) => row.open().is_some(),
        Some(BayHullState::Unreadable) => true,
    }
}

/// **A held hull stays put** (Q-36-32): neither carried nor moved by ordinary world moves while
/// it is berthed or worked on; berth and stow are its only doors.
pub(crate) struct BayHold;

impl EventHook for BayHold {
    fn carry_attempt(
        &self,
        ctx: &HookReadContext<'_>,
        _actor: ObjId,
        object: ObjId,
        _destination: ObjId,
    ) -> Result<(), RichText> {
        if held(ctx.world(), object) {
            return Err(RichText::plain("That hull is held in the bay."));
        }
        Ok(())
    }

    fn move_attempt(
        &self,
        ctx: &HookReadContext<'_>,
        actor: ObjId,
        _exit: Option<ObjId>,
        _dest: ObjId,
    ) -> Result<(), RichText> {
        if held(ctx.world(), actor) {
            return Err(RichText::plain("That hull is held in the bay."));
        }
        Ok(())
    }
}

// ---- the quote --------------------------------------------------------------------------

/// The unit, its spec and its anatomy.
fn unit_parts(
    w: &World,
    hull: ObjId,
) -> Result<
    (
        TacticalUnit,
        crate::definitions::UnitSpec,
        mush_tactics::field_ops::UnitAnatomyProfile,
    ),
    String,
> {
    let unit = mush_tactics::module::unit_of(w, hull)
        .cloned()
        .ok_or("That is not a hull the bay can work on.")?;
    let store = w
        .module_state::<TacsimStore>()
        .ok_or("TacSim is not loaded.")?;
    let wanted = unit.schema().key().id.as_str();
    let spec = store
        .definitions()
        .specs
        .iter()
        .find(|(id, _)| id.as_str() == wanted)
        .map(|(_, spec)| spec.clone())
        .ok_or("That hull's design is not in the catalog.")?;
    let anatomy = crate::field_ops::anatomy_for_unit_spec(&spec).map_err(|e| e.to_string())?;
    Ok((unit, spec, anatomy))
}

/// **A quote for `hull` where it stands now** (D-31-13): the tariff at the operator's
/// multiplier, split by what this facility may do.
///
/// # Errors
///
/// The sentence: no facility, no unit, no tariff, past repair.
pub fn quote_here(w: &World, hull: ObjId) -> Result<(Facility, BayQuote), String> {
    let facility = facility_of(w, hull)?;
    let (unit, spec, anatomy) = unit_parts(w, hull)?;
    let tariff = crate::definitions::workshop::workshop_tariff()
        .map_err(|e| e.to_string())?
        .with_multiplier_permille(facility.multiplier_permille)
        .map_err(|e| e.to_string())?;
    let store = w
        .module_state::<TacsimStore>()
        .ok_or("TacSim is not loaded.")?;
    let goods = store.goods().ok_or("The goods table is not loaded.")?;
    let mut quote = jobs::quote(
        &unit,
        &spec,
        &anatomy,
        store.definitions(),
        goods,
        &tariff,
        facility.class,
    )?;
    // Doc 39 cut IN.6: a replaced part comes from the stores at its catalogue price.
    for job in quote.jobs.iter_mut().chain(quote.elsewhere.iter_mut()) {
        if let BayJob::Replace { component, .. } = &job.job {
            job.part_minor = part_price(w, hull, component).map_or(0, |(_, _, price)| price);
        }
    }
    Ok((facility, quote))
}

// ---- the hold ---------------------------------------------------------------------------

/// **A minted lot as the requester reads it** (doc 39 cut IN.1): the good's label, and a part's
/// catalogue mass to the tenth of a tonne — `1 105 mm cannon mount, 2.4 t (lot 7)`; a bulk
/// lot reads by its quantity and id, as it always has.
fn lot_words(w: &World, lot: &act::LotMint) -> String {
    let good = w
        .module_state::<TacsimStore>()
        .and_then(|store| store.goods())
        .and_then(|goods| goods.good(&lot.commodity).cloned());
    match good {
        Some(good) if good.class == crate::definitions::GoodClass::Part => format!(
            "{} {}, {} t ({})",
            lot.quantity,
            good.label,
            crate::port::gauges::part_tonnes(
                good.mass_kg_per_unit()
                    .unwrap_or(0)
                    .saturating_mul(lot.quantity)
            ),
            lot.lot
        ),
        _ => format!("{} {} ({})", lot.quantity, lot.commodity, lot.lot),
    }
}

/// **The goods `jobs` draw on `hull`** (Q-36-31, Q-36-36; doc 39 cuts IN.1, IN.8): each rearm's
/// munitions good, summed by good, and the whole tonnes of `structural_alloys` and
/// `industrial_parts` the hull's `bale` takes in to cover the kilograms their points consume at
/// the design's own figures ([`Bale::draw`]); with the bale once the work is done, `None` when
/// the jobs touch no bale. Beam charge and a replaced part draw nothing.
#[must_use]
pub fn order_goods(
    w: &World,
    hull: ObjId,
    jobs: &[QuotedJob],
    bale: Option<Bale>,
) -> (Vec<GoodUnits>, Option<Bale>) {
    let mut goods: BTreeMap<CanonicalId, u64> = BTreeMap::new();
    for job in jobs {
        if let BayJob::Rearm {
            good: Some(good),
            units,
            ..
        } = &job.job
        {
            let sum = goods.entry(good.clone()).or_default();
            *sum = sum.saturating_add(*units);
        }
    }
    let mut after = None;
    let budget = crate::definitions::mass::hull_budget(w, hull).map(|(_, budget)| budget);
    if let Some(draw) = Bale::draw(bale, BayQuote::bale_need(jobs, budget.as_ref())) {
        for (good, units) in [(ALLOYS_GOOD, draw.alloys_t), (PARTS_GOOD, draw.fittings_t)] {
            if let Ok(good) = CanonicalId::new(good) {
                let sum = goods.entry(good).or_default();
                *sum = sum.saturating_add(units);
            }
        }
        after = Some(draw.after);
    }
    let goods = goods
        .into_iter()
        .filter(|(_, units)| *units > 0)
        .map(|(good, units)| GoodUnits { good, units })
        .collect();
    (goods, after)
}

/// **The carrier whose hold serves `hull` at `facility`**: the carrier whose bay it stands in,
/// wherever she is (doc 41 cut RS.2: GC.8b's rule without the port — her hold is aboard), or
/// the carrier it was berthed from, docked at the facility's port.
///
/// # Errors
///
/// The sentence for a hull with no carrier here.
pub fn source_carrier(w: &World, hull: ObjId, facility: &Facility) -> Result<ObjId, String> {
    let no_carrier = "That hull came off no carrier, so there is no hold to draw from or lay into.";
    if facility.class == FacilityClass::CarrierBay {
        return facility.carrier.ok_or_else(|| no_carrier.to_owned());
    }
    let carrier = facility
        .carrier
        .or_else(|| {
            match w
                .module_state::<TacsimStore>()
                .and_then(|s| s.bay_hull(hull))
            {
                Some(BayHullState::Present(row)) => row.berth.as_ref().and_then(|b| b.carrier),
                _ => None,
            }
        })
        // Doc 39 cut IN.9: a hull the yard built on the floor draws from a carrier her title
        // holder holds docked at this port (the first load is a rearm from the hold, Q-39-36).
        .or_else(|| crate::port::owned::titled_carrier_at(w, hull, facility.port.as_ref()?))
        .ok_or(no_carrier)?;
    match crate::port::ship::berth_of(w, carrier) {
        crate::port::ship::Berth::Docked { map, .. }
            if facility.port.as_ref() == Some(&map.entry) =>
        {
            Ok(carrier)
        }
        _ => Err("Her carrier is not docked at this port, so her hold is not here.".to_owned()),
    }
}

/// **The hold vault that serves `hull` at `facility`** ([`source_carrier`]'s stowage).
///
/// # Errors
///
/// The sentence for no carrier or no hold.
pub fn source_hold(w: &World, hull: ObjId, facility: &Facility) -> Result<VaultId, String> {
    let carrier = source_carrier(w, hull, facility)?;
    crate::port::ship::hull_view(w, carrier)
        .and_then(|view| crate::port::ship::hold_vault(w, &view))
        .ok_or_else(|| "Her carrier has no hold.".to_owned())
}

/// The hold carries every row of `goods`, or the sentence naming the first it lacks.
fn hold_carries(ctx: &Ctx<'_>, vault: VaultId, goods: &[GoodUnits]) -> Result<(), String> {
    for row in goods {
        let have = crate::port_tick::read_pool(ctx, EndpointRef(vault), &row.good)
            .map_err(|_| "The carrier's hold cannot be read.".to_owned())?;
        if have < row.units {
            return Err(format!(
                "The work draws {} {} from the carrier's hold, which carries {have}.",
                row.units, row.good
            ));
        }
    }
    Ok(())
}

// ---- the order --------------------------------------------------------------------------

fn fee_spec(
    payer: AccountId,
    operator: OrgId,
    operator_account: AccountId,
    requester: ObjId,
    fee: Amount,
    now: i64,
) -> ContractSpec {
    let deadline = EconomicDeadline {
        clock: EconomicClock::RealUtc,
        at: now.saturating_add(FEE_WINDOW_SECS),
    };
    let me = PrincipalId::Object(requester);
    ContractSpec {
        name: SmolStr::new_static(FEE_NAME),
        scope: None,
        parties: vec![PrincipalId::Organization(operator)],
        considerations: vec![ConsiderationSpec {
            account: payer,
            amount: fee,
        }],
        outcomes: vec![OutcomeTerms {
            name: SmolStr::new_static("worked"),
            when: vec![0],
            payouts: vec![PayoutShare {
                to: operator_account,
                share: Share::Exact(fee),
            }],
            expense_treatment: mush_economy::ExpenseTreatment::None,
            asset_consequences: Vec::new(),
        }],
        remainder_to: payer,
        conditions: vec![TypedCondition::Before { deadline }],
        adjudicator: None,
        evidence: vec![EvidenceKind::Before],
        deadline: Some(deadline),
        on_expiry: Expiry::Refund,
        visibility: EconomicVisibility::Public,
        roles: ContractRoles {
            author: Some(me.clone()),
            payer: Some(me),
            representative: None,
            facilitator: None,
        },
        deterministic: true,
        deliverables: Vec::new(),
    }
}

/// What a taken order reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Taken {
    /// The order.
    pub order: BayOrder,
}

/// **Take an order** (Q-36-40): `selector` picks jobs from the quote where the hull stands
/// (`all`, a kind word, a section or part id, comma-separated). The requester stands in the
/// facility room, holds a right that reaches every job, has no other order on the hull, and
/// pays the fee into escrow; the hull is then held until the work is done at the order's IC
/// due time.
///
/// # Errors
///
/// The refusal, as a sentence. Nothing is written on any refusal before the escrow; a refused
/// escrow cancels its own contract.
pub fn take_order(ctx: &mut Ctx<'_>, hull: ObjId, selector: &str) -> Result<Taken, String> {
    take(ctx, hull, selector, None)
}

/// **`+bay/fit`** (Q-36-37): an order that fits the part in `lot`, from the carrier's hold,
/// into the slot it fits. The lot must carry the bay's provenance (a part the bay can trace),
/// be clean, unfitted and of the slot's hardware row (doc 39 cut IN.1); it is consumed when the
/// work is done.
///
/// # Errors
///
/// The refusal, as a sentence: a hot part is refused here, and again at the draw.
pub fn fit_order(ctx: &mut Ctx<'_>, hull: ObjId, slot: &str, lot: LotId) -> Result<Taken, String> {
    let part = act::provenance(ctx, lot)?
        .ok_or("The bay fits only parts it can trace; that lot carries no part provenance.")?;
    if let mush_tactics::field_ops::ProvenanceLegalityStatus::ContrabandHotPart {
        rightful_owner,
        ..
    } = &part.legality
    {
        return Err(format!(
            "That part is hot: it came off #{} under a vacated claim, and a lawful workshop will \
             not fit it until #{} clears it.",
            part.parent_hull().map_or(0, |h| h.0),
            rightful_owner.0
        ));
    }
    if let Some(refusal) = disputed_refusal(ctx.w, &part) {
        return Err(refusal);
    }
    if part.fitted_into.is_some() {
        return Err("That part has already been fitted.".to_owned());
    }
    if act::lot_hold(ctx, lot)?.is_some() {
        return Err("That part is held for a sale or another order.".to_owned());
    }
    take(ctx, hull, slot, Some((lot, part)))
}

/// **The claim a disputed part is held under** (doc 39 cut IN.7, §4.11; doc 41 §4.6): a
/// stripped part its parent hull's record lists under a claim that is not vacated and whose
/// filed contract has not ended (an unreadable one counts as live: fail closed). Who owns it is
/// the claim's to decide, so no fit or build takes custody of it meanwhile.
#[must_use]
pub fn disputed(w: &World, part: &act::PartProvenance) -> Option<u64> {
    let parent = part.parent_hull()?;
    let listed = record_of(w, parent)
        .ok()?
        .claim_lots
        .into_iter()
        .find(|listed| listed.lot == part.lot.0 && listed.vacated.is_none())?;
    let ended = w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().contract(ContractId(listed.claim)).cloned())
        .is_some_and(|c| c.state.is_terminal());
    (!ended).then_some(listed.claim)
}

/// The refusal a custody reads for a disputed part (doc 39 cut IN.7): `+bay/fit` here, and a
/// build's delivery of a part into its escrow.
#[must_use]
pub fn disputed_refusal(w: &World, part: &act::PartProvenance) -> Option<String> {
    disputed(w, part).map(|claim| {
        format!(
            "That part is disputed: it came off #{} under claim {claim}, which is still open;              nothing takes custody of it until the claim is settled.",
            part.parent_hull().map_or(0, |h| h.0)
        )
    })
}

/// The `Replace` job for `slot` in `quote`, fitted from `lot`, when `part` fits it.
fn fit_job(
    w: &World,
    hull: ObjId,
    quote: &BayQuote,
    slot: &str,
    lot: LotId,
    part: &act::PartProvenance,
) -> Result<QuotedJob, String> {
    let slot = slot.trim();
    let job = quote
        .jobs
        .iter()
        .chain(&quote.elsewhere)
        .find(|j| {
            matches!(&j.job, BayJob::Replace { component, .. }
                if component.as_str() == slot || component.as_str().ends_with(&format!(".{slot}")))
        })
        .cloned()
        .ok_or("That slot has no part to replace.")?;
    let BayJob::Replace { component, .. } = job.job else {
        return Err("That slot has no part to replace.".to_owned());
    };
    // Doc 39 cut IN.1 (Q-39-3): a part fits a slot that names its hardware row, on any unit.
    let row = mush_tactics::module::unit_of(w, hull)
        .and_then(|u| u.component(&component).map(|c| c.definition.clone()))
        .and_then(|definition| crate::hardware::bay::part_of(w, &definition));
    if row.as_ref().map(|row| &row.row) != Some(&part.definition) {
        return Err("That part does not fit that slot.".to_owned());
    }
    Ok(QuotedJob {
        job: BayJob::Replace {
            component,
            lot: Some(lot),
        },
        part_minor: 0,
        ..job
    })
}

fn take(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    selector: &str,
    fit: Option<(LotId, act::PartProvenance)>,
) -> Result<Taken, String> {
    let mut record = record_of(ctx.w, hull).map_err(str::to_owned)?;
    if record.open().is_some() {
        return Err(HELD_OPEN.to_owned());
    }
    if record.owed.is_some() {
        return Err(SETTLING.to_owned());
    }
    if record.scrapped_ic.is_some() {
        return Err("That hull has been scrapped.".to_owned());
    }
    let (facility, quote) = quote_here(ctx.w, hull)?;
    // Doc 41 cut RS.2 (Q-41-5, Q-CF-30): no bay work starts under a field on her site.
    if let Some(field) = field_over(ctx, &facility)
        && let Some(site) = &facility.site
    {
        return Err(crate::surface::site_record::promoted_refusal(
            &field,
            &crate::surface::act::site_name(ctx.w, site),
        ));
    }
    let in_facility = ctx.w.get(ctx.actor).and_then(|o| o.location) == Some(facility.room)
        // Doc 41 cut RS.2: away from port, the work in her bay is ordered from aboard her.
        || (facility.away()
            && facility.carrier.is_some()
            && crate::space::commands::hull_aboard_for_actor(ctx.w, ctx.actor) == facility.carrier)
        || (facility.carrier == Some(hull)
            && (crate::space::commands::hull_aboard_for_actor(ctx.w, ctx.actor) == Some(hull)
                || crate::space::commands::hull_aboard_for_actor(ctx.w, ctx.actor).is_some_and(
                    |aboard_hull| {
                        matches!(
                            crate::port::ship::berth_of(ctx.w, aboard_hull),
                            crate::port::ship::Berth::Docked { map, .. } if facility.port.as_ref() == Some(&map.entry)
                        )
                    },
                )
                || ctx.w.get(ctx.actor).and_then(|o| o.location)
                    == ctx.w.get(hull).and_then(|o| o.location)));
    if !in_facility {
        return Err(format!(
            "Orders for that hull are taken in the {} it stands in.",
            facility.class.word()
        ));
    }
    let right = right_of(ctx, hull).ok_or(NO_RIGHT)?;
    let picked = match &fit {
        Some((lot, part)) => {
            let job = fit_job(ctx.w, hull, &quote, selector, *lot, part)?;
            // Doc 41 cut RS.2 (Q-41-6): away from port, a traced part from her hold fits in
            // her bay — the one part change that needs no store. Docked, the port's workshop
            // fits it.
            if job.job.needs() > facility.class && !facility.away() {
                return Err(workshop_refusal(ctx.w, hull, &facility, &job.job));
            }
            Some(vec![job])
        }
        None => quote.select(selector),
    };
    let picked = picked.ok_or_else(|| {
        quote
            .elsewhere
            .iter()
            .find(|j| {
                selector.eq_ignore_ascii_case(j.job.kind_word())
                    || selector.eq_ignore_ascii_case(j.job.target())
                    || j.job
                        .target()
                        .rsplit('.')
                        .next()
                        .is_some_and(|short| selector.eq_ignore_ascii_case(short))
            })
            .map_or_else(
                || "Nothing in the quote matches that; +bay/quote lists the work.".to_owned(),
                |j| workshop_refusal(ctx.w, hull, &facility, &j.job),
            )
    })?;
    if picked.len() > MAX_ORDER_JOBS {
        return Err("Too many jobs for one order; order them in parts.".to_owned());
    }
    allowed(right, &picked)?;
    // Doc 36a P0-6: the work is run through the workshop engine on a throwaway book now, as
    // `complete` will run it, so an order the engine would refuse then (a part in a section
    // that does not stand, ordered without its rebuild) is refused before the escrow.
    let mut fitted = BTreeMap::new();
    if let Some((_, part)) = &fit {
        for job in &picked {
            if let BayJob::Replace {
                component,
                lot: Some(_),
            } = &job.job
            {
                fitted.insert(component.clone(), part.condition);
            }
        }
    }
    let custody = custody_for(ctx, ctx.actor, right, hull);
    dry_run(ctx, hull, ctx.actor, &custody, &picked, &fitted)
        .map_err(|error| format!("That work cannot be done as ordered: {error}."))?;
    let mut picked = picked;
    // (After the dry run, which fits a part from the stores intact, as a seeded one is.)
    // Doc 39 cut IN.6 (Q-39-11): a replace takes the lowest-grade clean lot of the slot's row
    // from the workshop's stores, at its catalogue price, held from the take to the draw.
    let mut store_lot = None;
    if fit.is_none() {
        let mut replaces = picked
            .iter()
            .enumerate()
            .filter(|(_, j)| matches!(j.job, BayJob::Replace { lot: None, .. }))
            .map(|(i, _)| i);
        if let Some(at) = replaces.next() {
            if replaces.next().is_some() {
                return Err(ONE_STORE_PART.to_owned());
            }
            let job = &mut picked[at];
            let BayJob::Replace { component, lot } = &mut job.job else {
                return Err("That slot has no part to replace.".to_owned());
            };
            let (found, vault, price) = store_part(ctx, hull, &facility, component)?;
            let part = act::provenance(ctx, found.lot)?
                .ok_or_else(|| NO_PART.replace("{part}", component.as_str()))?;
            *lot = Some(found.lot);
            job.part_minor = price;
            store_lot = Some((found, vault, component.clone(), part.condition));
        }
    }
    // GC.8b: the goods come from the carrier's hold, checked now and drawn at completion.
    let (mut goods, mut bale_after) = order_goods(ctx.w, hull, &picked, record.bale);
    let vault = if goods.is_empty() && fit.is_none() {
        None
    } else if facility.class == FacilityClass::Workshop
        && facility.carrier == Some(hull)
        && fit.is_none()
        && source_hold(ctx.w, hull, &facility)
            .and_then(|v| hold_carries(ctx, v, &goods).map(|()| v))
            .is_err()
    {
        // The workshop supplies the work itself and the hull's bale is not touched.
        goods.clear();
        bale_after = None;
        None
    } else {
        berth_stands(ctx, ctx.actor, &record)?;
        let vault = source_hold(ctx.w, hull, &facility)?;
        hold_carries(ctx, vault, &goods)?;
        if let Some((lot, _)) = &fit
            && act::lot(ctx, *lot)?.is_none_or(|l| l.endpoint() != EndpointRef(vault))
        {
            return Err("That part's lot is not in her carrier's hold.".to_owned());
        }
        Some(vault.0)
    };
    let unit_revision = mush_tactics::module::unit_of(ctx.w, hull)
        .map(TacticalUnit::revision)
        .ok_or(NO_HULL)?;
    // Doc 41 cut RS.2 (Q-41-7, expensive to reverse): away from port nobody is paid — no fee
    // contract is filed and the order's fee is nil; the time (2x) is the price.
    let (fee_minor, contract) = match facility.operator {
        None => (0, None),
        Some((operator, operator_account)) => {
            let fee_minor =
                i64::try_from(BayQuote::fee_minor(&picked)).map_err(|_| "Fee too large.")?;
            let payer = crate::port::credit::own_account(ctx.w, ctx.actor)
                .map(|a| a.id)
                .ok_or("You hold no credit account to pay from.")?;
            let now = ctx.w.now();
            let spec = fee_spec(
                payer,
                operator,
                operator_account,
                ctx.actor,
                Amount::from_minor(fee_minor),
                now,
            );
            let contract = mush_economy::offer_contract(ctx, spec).map_err(|e| {
                tracing::error!(%e, hull = hull.0, "tacsim: +bay/order: the fee contract was not offered");
                "The workshop could not write the order; nothing changed.".to_owned()
            })?;
            match mush_economy::accept_contract(ctx, contract) {
                Ok(AcceptOutcome::Accepted { .. }) => {}
                other => {
                    tracing::warn!(contract = contract.0, outcome = ?other, "tacsim: +bay/order: the escrow was not placed");
                    let _ = mush_economy::cancel_contract(ctx, contract);
                    ctx.out.pin_deferred();
                    return Err(format!(
                        "You cannot cover the fee of {}.",
                        crate::port::coop::money_minor(ctx, fee_minor)
                    ));
                }
            }
            (fee_minor, Some(contract.0))
        }
    };
    let started = ctx.w.world_ic_now().at;
    let secs = BayQuote::secs(&picked, facility.class);
    record.orders = record.orders.saturating_add(1);
    let order = BayOrder {
        seq: record.orders,
        requester: ctx.actor,
        right,
        facility: facility.room,
        class: facility.class,
        jobs: picked,
        fee_minor,
        contract,
        munitions_vault: vault,
        part_vault: store_lot.as_ref().map(|(_, vault, _, _)| *vault),
        goods,
        draw_act: None,
        bale_after,
        unit_revision,
        started_ic: started,
        due_ic: started.saturating_add(secs),
        status: OrderStatus::Open,
        closed_ic: None,
        note: None,
        paused: None,
    };
    record.order = Some(order.clone());
    // Doc 39 cut IN.2 (Q-39-27's "reserved"): an order that fits a part from a lot holds that
    // lot from now to the draw, so no sale, move or split takes it meanwhile. The hold is an
    // act the port tick lands; one refused every try cancels the order.
    // Doc 39 cut IN.6: a part from the stores is held the same way, at the storefront.
    let held = match (&fit, vault, &store_lot) {
        (Some((lot, _)), Some(vault), _) => Some((*lot, vault)),
        (None, _, Some((found, vault, _, _))) => Some((found.lot, *vault)),
        _ => None,
    };
    if let Some((lot, vault)) = held {
        let act = next_act(ctx, &record);
        record.acts = act;
        record.owed = Some(Box::new(OwedAct {
            act,
            by: ctx.actor,
            vault: Some(vault),
            kind: OwedKind::Hold {
                order: order.seq,
                lot,
            },
            tries: 0,
        }));
    }
    if let Err(error) = record.check() {
        tracing::error!(%error, hull = hull.0, "tacsim: +bay/order wrote a record that fails its own check");
    }
    put_bay_hull(ctx, &record);
    // A store lot standing in its own ask is withdrawn by the stock clerk, who listed it, on
    // its next step (`port::stock`'s `claimed_step`); the order's hold waits for that.
    Ok(Taken { order })
}

/// Close the open order as cancelled with `note`, releasing the escrow — and, for an order that
/// fits a part from a lot, owing the release of the lot it held (doc 39 cut IN.2).
fn close_cancelled(ctx: &mut Ctx<'_>, record: BayHull, note: &str) {
    close_order(ctx, record, note, true);
}

fn close_order(ctx: &mut Ctx<'_>, mut record: BayHull, note: &str, release: bool) {
    let held = record.order.as_ref().and_then(|order| {
        order.jobs.iter().find_map(|j| match &j.job {
            BayJob::Replace { lot: Some(lot), .. } => order
                .part_vault
                .or(order.munitions_vault)
                .map(|vault| (order.seq, *lot, vault)),
            _ => None,
        })
    });
    if let (true, Some((seq, lot, vault)), None) = (release, held, &record.owed) {
        let act = next_act(ctx, &record);
        record.acts = act;
        record.owed = Some(Box::new(OwedAct {
            act,
            by: record.order.as_ref().map_or(ctx.actor, |o| o.requester),
            vault: Some(vault),
            kind: OwedKind::Release { order: seq, lot },
            tries: 0,
        }));
    }
    let Some(order) = record.order.as_mut() else {
        return;
    };
    // Doc 41 cut RS.2: an order away from port filed no fee, so there is none to release.
    if let Some(contract) = order.contract
        && !matches!(
            mush_economy::cancel_contract(ctx, ContractId(contract)),
            Ok(true)
        )
    {
        tracing::error!(
            contract = order.contract,
            "tacsim: a bay order's fee contract did not cancel; its refund retries in the \
             economy sweep"
        );
    }
    order.status = OrderStatus::Cancelled;
    order.closed_ic = Some(ctx.w.world_ic_now().at);
    order.note = Some(note.to_owned());
    let requester = order.requester;
    put_bay_hull(ctx, &record);
    ctx.out.sys_to_actor(
        requester,
        "tacsim",
        format!("The bay order on #{} is cancelled: {note}", record.hull.0),
    );
}

/// **`+bay/cancel`**: the requester (or anyone with a right) cancels the open order; the fee
/// is released and nothing is written to the hull.
///
/// # Errors
///
/// No open order, or no right.
pub fn cancel_order(ctx: &mut Ctx<'_>, hull: ObjId) -> Result<(), String> {
    let record = record_of(ctx.w, hull).map_err(str::to_owned)?;
    let order = record.open().ok_or("That hull has no open order.")?;
    if order.requester != ctx.actor && right_of(ctx, hull).is_none() {
        return Err(NO_RIGHT.to_owned());
    }
    // Doc 36a P0-6: an order whose draw is settling is not cancelled under it — the draw
    // would land on a cancelled order and its goods be lost.
    if record.owed.is_some() {
        return Err(SETTLING.to_owned());
    }
    close_cancelled(ctx, record, "cancelled by order");
    Ok(())
}

/// Why a waiting order can no longer be worked, if it cannot: the hull gone, moved off its
/// facility or changed since the order was taken.
fn refused_now(w: &World, hull: ObjId, order: &BayOrder) -> Option<&'static str> {
    let here = match order.class {
        FacilityClass::Workshop => w
            .get(hull)
            .and_then(|o| o.location)
            .filter(|loc| *loc == order.facility)
            .or_else(|| {
                facility_of(w, hull)
                    .ok()
                    .filter(|f| {
                        f.class == order.class
                            && f.room == order.facility
                            && f.carrier == Some(hull)
                    })
                    .map(|f| f.room)
            }),
        FacilityClass::CarrierBay => interior_of(w, hull).map(|(room, _)| room),
    };
    let Some(before) = mush_tactics::module::unit_of(w, hull) else {
        return Some("the hull is gone");
    };
    if here != Some(order.facility) {
        return Some("the hull left the bay");
    }
    if before.revision() != order.unit_revision {
        return Some("the hull changed while it waited");
    }
    None
}

/// **The custody a requester's work runs under** (doc 36a P2-5): a salvor works at their own
/// risk under the pending claim — as filed when it is, else the custody row itself (the claim's
/// number is the filed contract's, else the recovery operation's, else the hull's; the prior
/// owner is the file's, else the one the pin says answers for the title, else the asset's title
/// holder) — and every other right works under the requester's resolved title.
fn custody_for(ctx: &Ctx<'_>, requester: ObjId, right: Right, hull: ObjId) -> CustodyTitleStatus {
    let pending = (right == Right::Salvor)
        .then(|| salvor_custody(ctx, requester, hull))
        .flatten()
        .and_then(|row| {
            let number = match row.claim {
                crate::surface::salvage::ClaimSeam::Filed { contract } => contract,
                crate::surface::salvage::ClaimSeam::Owed => row
                    .file
                    .as_ref()
                    .map(|file| file.id.get())
                    .or_else(|| row.operation.map(|op| op.get()))
                    .unwrap_or(u64::from(hull.0)),
            };
            let title_holder = ctx
                .w
                .module_state::<EconomyStore>()
                .and_then(|e| e.snapshot().asset(hull))
                .and_then(|asset| match asset.title_owner {
                    PrincipalId::Object(owner) => Some(owner),
                    _ => None,
                });
            let prior_owner = row
                .file
                .as_ref()
                .map(|file| file.prior_owner)
                .or_else(|| row.pin.as_ref().and_then(|pin| pin.answering))
                .or(title_holder)?;
            Some(CustodyTitleStatus::PendingSalvageClaim {
                claim_id: SalvageClaimId::new(number).ok()?,
                custodian: requester,
                prior_owner,
                deadline_real_s: row.file.as_ref().map_or(0, |file| file.deadline_real_s),
            })
        });
    pending.unwrap_or(CustodyTitleStatus::ResolvedTitle { owner: requester })
}

/// **The conditions the order's fitted parts come in**, read from their lots' provenance.
fn fitted_of(ctx: &Ctx<'_>, order: &BayOrder) -> BTreeMap<ComponentId, ComponentCondition> {
    let mut fitted = BTreeMap::new();
    for job in &order.jobs {
        if let BayJob::Replace {
            component,
            lot: Some(lot),
        } = &job.job
            && let Ok(Some(part)) = act::provenance(ctx, *lot)
        {
            fitted.insert(component.clone(), part.condition);
        }
    }
    fitted
}

/// **The work, dry-run** (doc 36a P0-6): `jobs` on `hull` as it stands, through the workshop
/// engine on a throwaway book, as [`complete`] runs them. Nothing is written.
///
/// # Errors
///
/// The engine's refusal, as a sentence.
fn dry_run(
    ctx: &Ctx<'_>,
    hull: ObjId,
    requester: ObjId,
    custody: &CustodyTitleStatus,
    jobs: &[QuotedJob],
    fitted: &BTreeMap<ComponentId, ComponentCondition>,
) -> Result<(), String> {
    let (before, _, anatomy) = unit_parts(ctx.w, hull)?;
    jobs::work(
        &before,
        &anatomy,
        jobs,
        &BTreeMap::new(),
        fitted,
        requester,
        custody,
        ctx.w.world_ic_now().at,
    )
    .map(|_| ())
}

/// **Finish one due order** (the port tick): the hull still at the facility and its unit
/// unchanged. An order that draws goods first owes its draw (GC.8b): the hold must still be
/// its carrier's, docked here, the work must still run on the hull (doc 36a P0-6: a dry run,
/// so no draw is owed for work that would be refused), and the draw lands through
/// `tacsim_bay_work` before the work is written ([`settle`]). An order that draws nothing is
/// completed at once ([`complete`]).
fn finish(ctx: &mut Ctx<'_>, hull: ObjId) {
    let Ok(mut record) = record_of(ctx.w, hull) else {
        return;
    };
    let Some(order) = record.open().cloned() else {
        return;
    };
    // An owed act (the part's hold) lands before the work is finished; a field holds it.
    if ctx.w.world_ic_now().at < order.due_ic
        || order.draw_act.is_some()
        || record.owed.is_some()
        || order.paused.is_some()
    {
        return;
    }
    if let Some(note) = refused_now(ctx.w, hull, &order) {
        close_cancelled(ctx, record, note);
        return;
    }
    let fit = order.jobs.iter().find_map(|j| match &j.job {
        BayJob::Replace { lot: Some(lot), .. } => Some(*lot),
        _ => None,
    });
    if order.goods.is_empty() && fit.is_none() {
        complete(ctx, hull);
        return;
    }
    // The hold the order was taken against must still be the one that serves it here.
    let still = facility_of(ctx.w, hull)
        .and_then(|facility| source_hold(ctx.w, hull, &facility))
        .ok()
        .map(|vault| vault.0);
    // Doc 39 cut IN.6: an order whose only draw is a part from the stores has no hold to keep.
    let holdless = order.munitions_vault.is_none() && order.part_vault.is_some();
    if !holdless && (still.is_none() || still != order.munitions_vault) {
        close_cancelled(
            ctx,
            record,
            "her carrier's hold is no longer here to draw from",
        );
        return;
    }
    let custody = custody_for(ctx, order.requester, order.right, hull);
    if let Err(error) = dry_run(
        ctx,
        hull,
        order.requester,
        &custody,
        &order.jobs,
        &fitted_of(ctx, &order),
    ) {
        close_cancelled(
            ctx,
            record,
            &format!("the work could not be done ({error})"),
        );
        return;
    }
    let act = next_act(ctx, &record);
    record.acts = act;
    record.owed = Some(Box::new(OwedAct {
        act,
        by: order.requester,
        vault: order.munitions_vault.or(order.part_vault),
        kind: OwedKind::Draw {
            order: order.seq,
            goods: order.goods.clone(),
            fit,
            fit_at: order.part_vault,
        },
        tries: 0,
    }));
    if let Some(row) = record.order.as_mut() {
        row.draw_act = Some(act);
    }
    put_bay_hull(ctx, &record);
    settle(ctx, hull);
}

/// **Complete one order**: its goods drawn (or none owed), the operator accepts the fee
/// contract (the escrow pays on the economy's sweep), then the work is written through
/// `mush_tactics`' one plan path. A draw is all or nothing, so every rearm loads its full
/// rounds; a part fitted from a lot comes in the condition its provenance records.
fn complete(ctx: &mut Ctx<'_>, hull: ObjId) {
    let Ok(record) = record_of(ctx.w, hull) else {
        return;
    };
    let Some(order) = record.open().cloned() else {
        return;
    };
    if let Some(note) = refused_now(ctx.w, hull, &order) {
        close_cancelled(ctx, record, note);
        return;
    }
    let Some(before) = mush_tactics::module::unit_of(ctx.w, hull).cloned() else {
        close_cancelled(ctx, record, "the hull is gone");
        return;
    };
    let Ok((_, _, anatomy)) = unit_parts(ctx.w, hull) else {
        close_cancelled(ctx, record, "the hull's design cannot be read");
        return;
    };
    // Doc 36a P2-5: the work runs under the custody the order's right gives — a salvor's under
    // the pending claim, which must still stand (a vacated claim works nothing) — at the IC
    // instant the engine stamps. (The right itself is not re-read here: a grant is an owner read
    // the tick has no line to.)
    if order.right == Right::Salvor && salvor_custody(ctx, order.requester, hull).is_none() {
        close_cancelled(ctx, record, "the requester's custody of the hull has ended");
        return;
    }
    let fitted = fitted_of(ctx, &order);
    let custody = custody_for(ctx, order.requester, order.right, hull);
    let at = ctx.w.world_ic_now().at;
    let plan = jobs::work(
        &before,
        &anatomy,
        &order.jobs,
        &BTreeMap::new(),
        &fitted,
        order.requester,
        &custody,
        at,
    )
    .and_then(|after| {
        mush_tactics::field_ops::ic_condition_plan(
            &before,
            &after,
            None,
            Some(SmolStr::new_static("tacsim.bay.repair")),
        )
        .map_err(|e| e.to_string())
    });
    let plan = match plan {
        Ok(plan) => plan,
        Err(error) => {
            close_cancelled(
                ctx,
                record,
                &format!("the work could not be done ({error})"),
            );
            return;
        }
    };
    // Doc 41 cut RS.2 (Q-41-7): an order away from port filed no fee and pays nobody.
    let paid = order.contract.is_none_or(|contract| {
        payee(ctx.w, contract).is_some_and(|operator| {
            matches!(
                mush_economy::accept_contract_as(
                    ctx,
                    ContractId(contract),
                    &PrincipalId::Organization(operator)
                ),
                Ok(AcceptOutcome::Accepted {
                    activated: true,
                    ..
                })
            )
        })
    });
    if !paid {
        close_cancelled(ctx, record, "the facility could not take the fee");
        return;
    }
    if let Some(plan) = plan {
        if let Err(error) = mush_tactics::module::apply_plan(ctx, hull, &plan) {
            tracing::error!(
                hull = hull.0, contract = order.contract, %error,
                "tacsim: a paid bay order's work was refused by the unit; staff refund the fee and \
                 repair by hand"
            );
        } else if let Some(book) = ctx
            .w
            .module_state_mut::<crate::space::tick::SpaceBubbleBook>()
        {
            book.derelicts.remove(&hull);
        }
    }
    let mut record = record;
    if let Some(row) = record.order.as_mut() {
        row.status = OrderStatus::Done;
        row.closed_ic = Some(ctx.w.world_ic_now().at);
    }
    // Doc 39 cut IN.1: the bale the work drew through, its draw landed with the order's.
    if let Some(after) = order.bale_after {
        record.bale = Some(after);
    }
    put_bay_hull(ctx, &record);
    let words: Vec<String> = order.jobs.iter().map(|j| j.job.describe()).collect();
    ctx.out.sys_to_actor(
        order.requester,
        "tacsim",
        format!(
            "The {} has finished on #{}: {}.",
            order.class.word(),
            hull.0,
            words.join(", ")
        ),
    );
}

/// **Settle the act `hull`'s record owes** (the two writes; the
/// [`crate::port::bay_work::act`] module doc): landed on the ledger, the record lets it go and
/// its consequence follows (a draw completes its order; a mint writes the strip or the scrap
/// to the hull, [`strip::land_mint`]); otherwise it is submitted as its actor, at most
/// [`act::MAX_ACT_TRIES`] times, and then given up (a draw cancels its order and releases the
/// fee; a lost mint leaves the hull as it was and pages staff). A given-up act rolls the
/// record's count back to the ledger's (doc 36a §5.1), so the next act takes the ordinal the
/// prepare accepts.
fn settle(ctx: &mut Ctx<'_>, hull: ObjId) {
    let Ok(mut record) = record_of(ctx.w, hull) else {
        return;
    };
    let Some(owed) = record.owed.clone() else {
        return;
    };
    // A draw finishes work, and no work finishes under a W-clock freeze (Q-36-32).
    if matches!(owed.kind, OwedKind::Draw { .. }) && ctx.w.world_timeline().freeze().is_some() {
        return;
    }
    let Some(landed) = act::landed(ctx, hull) else {
        return;
    };
    if landed >= owed.act {
        record.owed = None;
        put_bay_hull(ctx, &record);
        match &owed.kind {
            OwedKind::Draw { .. } => complete(ctx, hull),
            OwedKind::Mint { why, lots, settle } => {
                strip::land_mint(ctx, hull, owed.by, why, lots);
                // Doc 39 cut IN.8: a workshop scrap's or a return's lots are the operator's,
                // and its credit and shop bale land with them.
                let into = if let Some(settle) = settle {
                    scrap::land(ctx, hull, owed.act, owed.by, settle);
                    format!(
                        "the workshop's stores, and {} is paid to you",
                        crate::port::board::money(
                            ctx.w,
                            mush_economy::Amount::from_minor(
                                i64::try_from(settle.credit_minor).unwrap_or(i64::MAX)
                            )
                        )
                    )
                } else {
                    "the carrier's hold".to_owned()
                };
                ctx.out.sys_to_actor(
                    owed.by,
                    "tacsim",
                    format!(
                        "The {why} of #{} is laid into {into}: {}.",
                        hull.0,
                        lots.iter()
                            .map(|l| lot_words(ctx.w, l))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
            OwedKind::Hot { .. }
            | OwedKind::Hold { .. }
            | OwedKind::Release { .. }
            | OwedKind::Commission { .. } => {}
        }
        return;
    }
    // Doc 36a P0-6 (b), the belt under `cancel_order`'s SETTLING: a Draw serves an open order;
    // one whose order has closed under it is dropped before it is submitted, and the record's
    // count follows the ledger.
    if matches!(owed.kind, OwedKind::Draw { .. }) && record.open().is_none() {
        record.owed = None;
        record.acts = landed;
        put_bay_hull(ctx, &record);
        return;
    }
    if ctx.authoritative_pending(owed.by) {
        return;
    }
    // Doc 39 cut IN.6: a store lot still in its own ask waits, without a try, for the clerk to
    // withdraw it.
    if let OwedKind::Hold { lot, .. } = &owed.kind
        && act::lot_hold(ctx, *lot)
            .ok()
            .flatten()
            .is_some_and(|h| h.holder().starts_with("listing/"))
        && record.open().is_some_and(|o| o.part_vault.is_some())
    {
        return;
    }
    if owed.tries >= act::MAX_ACT_TRIES {
        record.owed = None;
        // Doc 36a §5.1: a given-up act never landed, so the record's count follows the
        // ledger; left one ahead, every later act on the hull would be refused Conflict.
        record.acts = landed;
        match &owed.kind {
            OwedKind::Draw { .. } => {
                if let Some(order) = record.order.as_mut() {
                    order.draw_act = None;
                }
                close_cancelled(
                    ctx,
                    record,
                    "the carrier's hold could not supply the goods the work needs",
                );
            }
            OwedKind::Hold { .. } => {
                // Nothing was held, so nothing is released.
                close_order(
                    ctx,
                    record,
                    "the part could not be held for the order (it has left the hold, or a \
                     sale or another order holds it)",
                    false,
                );
            }
            OwedKind::Mint { why, settle, .. } => {
                // The strip or the scrap was never written to the hull (`strip::land_mint`
                // writes it when the mint lands): a lost mint destroys nothing. Doc 39 cut IN.8:
                // the shop bale it held is let go as it stood, and nothing is paid.
                put_bay_hull(ctx, &record);
                if let Some(settle) = settle
                    && let Some(Some(mut shop)) = ctx
                        .w
                        .module_state::<TacsimStore>()
                        .map(|store| store.shop_bale(&settle.shop.port))
                    && shop.pending == Some((hull, owed.act))
                {
                    shop.pending = None;
                    crate::store::put_shop_bale(ctx, &shop);
                }
                tracing::error!(
                    hull = hull.0,
                    act = owed.act,
                    op = why,
                    "tacsim: a bay mint was refused every try; no lot was laid in the hold and \
                     nothing came off the hull"
                );
                ctx.out.sys_to_actor(
                    owed.by,
                    "tacsim",
                    format!(
                        "The {why} of #{} could not be laid into the carrier's hold; nothing \
                         came off her.",
                        hull.0
                    ),
                );
            }
            kind => {
                put_bay_hull(ctx, &record);
                tracing::error!(
                    hull = hull.0,
                    act = owed.act,
                    op = kind.word(),
                    "tacsim: a bay act (a hot flag, a cancelled order's release or a part \
                     commission) was refused every try and was not written; staff write it by \
                     hand"
                );
            }
        }
        return;
    }
    let next = OwedAct {
        tries: owed.tries.saturating_add(1),
        ..*owed
    };
    record.owed = Some(Box::new(next.clone()));
    put_bay_hull(ctx, &record);
    if !act::submit(ctx, hull, &next) {
        tracing::error!(
            hull = hull.0,
            "tacsim: the bay act is not registered on this roster; its owed act waits"
        );
    }
}

/// **The operator org a fee contract pays** (doc 41 cut RS.2): the one organization party
/// [`fee_spec`] bound when the order was taken — read off the contract, so an order taken in a
/// docked carrier's bay is paid to that port's operator wherever she has sailed since.
fn payee(w: &World, contract: u64) -> Option<OrgId> {
    w.module_state::<EconomyStore>()?
        .snapshot()
        .contract(ContractId(contract))?
        .consent
        .parties
        .iter()
        .find_map(|party| match party {
            PrincipalId::Organization(org) => Some(*org),
            _ => None,
        })
}

/// **The bay's arm of the `tacsim.port` fire**: every owed inventory act is settled, then every
/// open order whose IC due time has come is finished. The IC clock stands still under a
/// W-clock freeze, so a frozen world finishes nothing (Q-36-32); an owed mint or flag still
/// settles, an owed draw waits.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let owing = ctx
        .w
        .module_state::<TacsimStore>()
        .map(TacsimStore::bay_owing_hulls)
        .unwrap_or_default();
    for hull in owing {
        settle(ctx, hull);
    }
    if ctx.w.world_timeline().freeze().is_some() {
        return;
    }
    let open = ctx
        .w
        .module_state::<TacsimStore>()
        .map(TacsimStore::bay_open_hulls)
        .unwrap_or_default();
    for hull in open {
        if !wait_under_field(ctx, hull) {
            finish(ctx, hull);
        }
    }
}

/// **A carrier bay's order under a field** (doc 41 cut RS.2, Q-41-5; RS.1's tend precedent,
/// Q-41-3): when a field opens on the site her carrier stands on, the open order is paused, its
/// IC time left kept; on the fire that finds the site settled it resumes, due that much later.
/// An order whose draw is already owed is past the work and is not paused. Whether the order
/// waits now.
fn wait_under_field(ctx: &mut Ctx<'_>, hull: ObjId) -> bool {
    let Ok(mut record) = record_of(ctx.w, hull) else {
        return false;
    };
    let Some(order) = record.open() else {
        return false;
    };
    if order.class != FacilityClass::CarrierBay || order.draw_act.is_some() {
        return false;
    }
    let facility = order.facility;
    let field = facility_of(ctx.w, hull)
        .ok()
        .filter(|f| f.room == facility)
        .and_then(|f| field_over(ctx, &f));
    let now = ctx.w.world_ic_now().at;
    let Some(row) = record.order.as_mut() else {
        return false;
    };
    match (field.is_some(), row.paused) {
        (true, Some(_)) => true,
        (false, None) => false,
        (true, None) => {
            row.paused = Some(row.due_ic.saturating_sub(now).max(1));
            put_bay_hull(ctx, &record);
            true
        }
        (false, Some(left)) => {
            row.due_ic = now.saturating_add(left).max(row.started_ic);
            row.paused = None;
            put_bay_hull(ctx, &record);
            false
        }
    }
}

// ---- berth and stow ---------------------------------------------------------------------

/// **`berth`** (Q-36-53): the hull leaves its docked carrier's bay for the workshop room the
/// actor stands in, at that carrier's port; its slot frees and the carrier may sail without it.
///
/// # Errors
///
/// The refusal, as a sentence; nothing is written on any.
pub fn berth(ctx: &mut Ctx<'_>, hull: ObjId) -> Result<ObjId, String> {
    let mut record = record_of(ctx.w, hull).map_err(str::to_owned)?;
    if record.open().is_some() {
        return Err(HELD_OPEN.to_owned());
    }
    if record.owed.is_some() {
        return Err(SETTLING.to_owned());
    }
    if record.berth.is_some() {
        return Err("That hull is already berthed.".to_owned());
    }
    let here = ctx
        .w
        .get(ctx.actor)
        .and_then(|o| o.location)
        .ok_or(NO_HULL)?;
    let (_, def, map, room) = workshops(ctx.w)
        .into_iter()
        .find(|(_, _, _, room)| *room == here)
        .ok_or("Hulls are berthed from a port's workshop.")?;
    let (bay_room, slot) = interior_of(ctx.w, hull).ok_or(NO_HULL)?;
    let carrier = carrier_of_bay(ctx.w, bay_room).ok_or(NO_HULL)?;
    match crate::port::ship::berth_of(ctx.w, carrier) {
        crate::port::ship::Berth::Docked { map: at, .. } if at.entry == map.entry => {}
        _ => return Err("Her carrier is not docked at this port.".to_owned()),
    }
    let right = right_of(ctx, hull).ok_or(NO_RIGHT)?;
    if !permits(right, Act::Berth) {
        return Err(NO_RIGHT.to_owned());
    }
    // Doc 36a P1-2: a hull leaves a carrier's bay by the hand of one who has the run of that
    // carrier, or holds the hull in custody aboard her.
    if !carrier_right(ctx.w, ctx.actor, carrier)
        && salvor_custody(ctx, ctx.actor, hull).is_none_or(|row| row.carrier != carrier)
    {
        return Err(
            "Hulls are berthed out of your own carrier's bay, or out of the bay that holds your \
             salvage."
                .to_owned(),
        );
    }
    let port = map.entry.clone();
    let berths = def.workshop.as_ref().map_or(0, |w| w.berths);
    let used = ctx
        .w
        .module_state::<TacsimStore>()
        // Doc 41 cut RS.4 (round-2 item 41-15): a hull held for her owner takes no stand.
        .map_or(0, |s| {
            s.bay_berthed_in(room)
                .into_iter()
                .filter(|h| held_for(ctx.w, *h).is_none())
                .count()
        });
    if u32::try_from(used).unwrap_or(u32::MAX) >= berths {
        return Err("Every berth in the workshop is taken.".to_owned());
    }
    mush_sim::unplace(ctx, hull, "tacsim.bay.berth")?;
    ctx.w.move_to(hull, room).map_err(|e| e.to_string())?;
    record.berth = Some(BerthRow {
        workshop: room,
        carrier: Some(carrier),
        bay_room: Some(bay_room),
        slot,
        by: ctx.actor,
        at_ic: ctx.w.world_ic_now().at,
    });
    put_bay_hull(ctx, &record);
    // Doc 41 cut RS.5 (Q-41-8): the first handover — berthed into the destination's workshop
    // by the acceptor's hand.
    let _ = crate::surface::retrieval::deliver(ctx, hull, &port, ctx.actor);
    Ok(room)
}

/// **`stow`** (Q-36-53; doc 39 cut IN.9, §4.13, §8.4: this document's verb): a berthed hull
/// goes back to the bay slot it came from, its carrier still docked at this port — or, with
/// `carrier` named, into a free slot of that carrier's bay, the carrier docked at this port.
///
/// The refusals, in order: an open order or an owed act; a hull in another's live salvage
/// custody; no berth right; a named carrier not docked here; no run of it; a lessee or salvor
/// naming a carrier other than the row's; no slot that takes her (the slot's mass limit read
/// against her catalogue mass). Doc 41's RS.4 widens the run-of-carrier right, not this verb.
///
/// # Errors
///
/// The refusal, as a sentence; nothing is written on any.
pub fn stow(ctx: &mut Ctx<'_>, hull: ObjId, carrier: Option<ObjId>) -> Result<ObjId, String> {
    let mut record = record_of(ctx.w, hull).map_err(str::to_owned)?;
    if record.open().is_some() {
        return Err(HELD_OPEN.to_owned());
    }
    if record.owed.is_some() {
        return Err(SETTLING.to_owned());
    }
    // Doc 41 cut RS.4 (Q-41-18): a vacated hull held for her owner goes bay to bay.
    if record.berth.is_none()
        && let Some(named) = carrier
        && record.held_for.is_some()
    {
        return stow_held(ctx, record, named);
    }
    let row = record.berth.clone().ok_or("That hull is not berthed.")?;
    if ctx.w.get(ctx.actor).and_then(|o| o.location) != Some(row.workshop) {
        return Err("Hulls are stowed from the workshop they stand in.".to_owned());
    }
    if let Some(custody) = crate::surface::salvage::custody_of(ctx, hull)
        && custody.holder != ctx.actor
    {
        return Err(format!(
            "The {} is in {}'s custody; only they may move her.",
            mush_core::output::display_name(ctx.w, hull),
            mush_core::output::display_name(ctx.w, custody.holder)
        ));
    }
    let right = right_of(ctx, hull).ok_or(NO_RIGHT)?;
    if !permits(right, Act::Berth) {
        return Err(NO_RIGHT.to_owned());
    }
    let docked_here = |carrier: ObjId| match crate::port::ship::berth_of(ctx.w, carrier) {
        crate::port::ship::Berth::Docked { map, def } => {
            crate::port::workshop::room(ctx.w, &map.entry, def)
                .ok()
                .flatten()
                == Some(row.workshop)
        }
        _ => false,
    };
    let (target, room, slot) = match carrier.filter(|named| Some(*named) != row.carrier) {
        None => {
            let (Some(carrier), Some(bay_room)) = (row.carrier, row.bay_room) else {
                return Err("That hull came off no carrier; name the one to take her.".to_owned());
            };
            berth_stands(ctx, ctx.actor, &record)?;
            if !docked_here(carrier) {
                return Err("Her carrier is not docked at this port.".to_owned());
            }
            (carrier, bay_room, row.slot.clone())
        }
        Some(named) => {
            let name = mush_core::output::display_name(ctx.w, named);
            if !docked_here(named) {
                return Err(format!("The {name} is not docked at this port."));
            }
            if !carrier_right(ctx.w, ctx.actor, named) {
                return Err(format!("You do not have the run of the {name}'s bay."));
            }
            if !matches!(right, Right::Title | Right::Grant) {
                return Err("Only her title holder may stow her into another carrier.".to_owned());
            }
            let (room, slot) = free_bay_slot(ctx.w, named, hull)?;
            (named, room, Some(slot))
        }
    };
    mush_sim::place_interior(ctx, hull, room, slot)
        .map_err(|e| format!("Her bay will not take her: {e}"))?;
    record.berth = None;
    record.held_for = None;
    put_bay_hull(ctx, &record);
    // Doc 41 cut RS.5 (Q-41-8): the second handover — stowed into the poster's own carrier
    // docked at the destination.
    if let crate::port::ship::Berth::Docked { map, .. } = crate::port::ship::berth_of(ctx.w, target)
        && let Some(order) = crate::surface::retrieval::held_by(ctx.w, hull, ctx.actor)
        && crate::port::coop::helm_holder(ctx.w, target)
            .is_some_and(|helm| crate::surface::retrieval::acts_for(ctx.w, helm, &order.poster))
    {
        let entry = map.entry.clone();
        let _ = crate::surface::retrieval::deliver(ctx, hull, &entry, ctx.actor);
    }
    Ok(target)
}

// ---- the vacated hull's return (doc 41 cut RS.4; Q-41-18, Q-41-19, Q-38-19) ----------------

/// **Whom `hull` is held for**, when a vacated claim left her held ([`BayHull::held_for`]).
#[must_use]
pub fn held_for(w: &World, hull: ObjId) -> Option<PrincipalId> {
    match w
        .module_state::<TacsimStore>()
        .and_then(|s| s.bay_hull(hull))
    {
        Some(BayHullState::Present(row)) => row.held_for.clone(),
        _ => None,
    }
}

/// Whether `actor` answers for `principal`: the character, or one acting for the organization.
fn answers_for(w: &World, actor: ObjId, principal: &PrincipalId) -> bool {
    match principal {
        PrincipalId::Object(who) => *who == actor,
        other => mush_economy::principal_holders(w, other).contains(&actor),
    }
}

/// **Hold a vacated hull for her title holder** (Q-41-18): the claims' tick, at the vacate. A
/// record that will not read is left as it is (logged); the hull waits for staff.
pub(crate) fn hold_for(ctx: &mut Ctx<'_>, hull: ObjId, owner: PrincipalId) {
    let mut record = match record_of(ctx.w, hull) {
        Ok(record) => record,
        Err(why) => {
            tracing::error!(
                hull = hull.0,
                why,
                "tacsim: a vacated hull cannot be held for her owner"
            );
            return;
        }
    };
    if record.held_for.as_ref() == Some(&owner) {
        return;
    }
    record.held_for = Some(owner);
    put_bay_hull(ctx, &record);
}

/// Where a carrier stands, as the return reads it: the port or site entry she is docked at.
fn standing(w: &World, carrier: ObjId) -> Option<CanonicalId> {
    match crate::port::ship::berth_of(w, carrier) {
        crate::port::ship::Berth::Docked { map, .. } => Some(map.entry.clone()),
        _ => None,
    }
}

/// Whether `who` stands aboard `carrier` (in her or one of her rooms).
fn aboard(w: &World, who: ObjId, carrier: ObjId) -> bool {
    let Some(at) = w.get(who).and_then(|o| o.location) else {
        return false;
    };
    crate::port::ship::hull_view(w, carrier)
        .is_some_and(|view| at == carrier || view.row.rooms.values().any(|room| ObjId(*room) == at))
}

/// **The bay-to-bay return** (Q-41-18): a hull held for `ctx.actor` (or an organization they act
/// for, or a `maintain_refit` grantee of hers) goes out of whatever bay holds her into a free slot
/// of `named`, a carrier the actor has the run of and stands aboard, docked where the holding
/// carrier is — one port, or one site's pad. The custodian cannot refuse it.
///
/// # Errors
///
/// The refusal, as a sentence; nothing is written on any.
fn stow_held(ctx: &mut Ctx<'_>, mut record: BayHull, named: ObjId) -> Result<ObjId, String> {
    let hull = record.hull;
    let owner = record.held_for.clone().ok_or(NO_RIGHT)?;
    if !answers_for(ctx.w, ctx.actor, &owner)
        && !unit_grant(ctx, hull, UnitCapability::MaintainRefit)
    {
        return Err(NO_RIGHT.to_owned());
    }
    let crate::surface::act::Where::Bay { hull: holding } =
        crate::surface::act::where_is(ctx.w, hull)
    else {
        return Err("She is in no carrier's bay; she is stowed from where she stands.".to_owned());
    };
    let name = mush_core::output::display_name(ctx.w, named);
    if holding == named {
        return Err(format!("She is in the {name}'s bay already."));
    }
    if !carrier_right(ctx.w, ctx.actor, named) {
        return Err(format!("You do not have the run of the {name}'s bay."));
    }
    if !aboard(ctx.w, ctx.actor, named) {
        return Err(format!(
            "You must be aboard the {name} to take her into her bay."
        ));
    }
    let here = standing(ctx.w, named);
    if here.is_none() || here != standing(ctx.w, holding) {
        return Err(format!(
            "The {name} must stand where the {} does, at one port or on one pad.",
            mush_core::output::display_name(ctx.w, holding)
        ));
    }
    let (room, slot) = free_bay_slot(ctx.w, named, hull)?;
    mush_sim::place_interior(ctx, hull, room, Some(slot))
        .map_err(|e| format!("Her bay will not take her: {e}"))?;
    record.held_for = None;
    put_bay_hull(ctx, &record);
    Ok(named)
}

/// **The port holds her** (Q-41-19; the docking hook): when `carrier` docks at a port with a
/// workshop, every hull in her bay held for someone is berthed into the workshop room with no
/// carrier — outside the stand count — under her owner's name, and the custodian's slot frees;
/// a lease rig goes back to its shed instead. Her owner's carrier `+bay/stow`s her from there.
pub(crate) fn berth_held(ctx: &mut Ctx<'_>, carrier: ObjId) {
    let crate::port::ship::Berth::Docked { map, def } = crate::port::ship::berth_of(ctx.w, carrier)
    else {
        return;
    };
    if def.site.is_some() {
        return;
    }
    let Ok(Some(room)) = crate::port::workshop::room(ctx.w, &map.entry, def) else {
        return;
    };
    let aboard: Vec<ObjId> = ctx
        .w
        .module_state::<TacsimStore>()
        .map(|s| {
            s.bay_hulls
                .iter()
                .filter_map(|(hull, state)| match state {
                    BayHullState::Present(row) if row.held_for.is_some() && row.berth.is_none() => {
                        Some(*hull)
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let by = crate::port::coop::helm_holder(ctx.w, carrier).unwrap_or(mush_core::GOD);
    for hull in aboard {
        if crate::surface::act::where_is(ctx.w, hull)
            != (crate::surface::act::Where::Bay { hull: carrier })
        {
            continue;
        }
        let lease = ctx
            .w
            .module_state::<TacsimStore>()
            .is_some_and(|s| s.tv_lease_on(hull).is_some());
        if lease {
            if crate::port::tv::hand_back(ctx, hull)
                && let Ok(mut record) = record_of(ctx.w, hull)
            {
                record.held_for = None;
                put_bay_hull(ctx, &record);
            }
            continue;
        }
        let Ok(mut record) = record_of(ctx.w, hull) else {
            continue;
        };
        if let Err(error) = mush_sim::unplace(ctx, hull, "tacsim.bay.held") {
            tracing::error!(hull = hull.0, %error, "tacsim: a held hull could not leave the bay");
            continue;
        }
        if let Err(error) = ctx.w.move_to(hull, room) {
            tracing::error!(hull = hull.0, %error, "tacsim: invariant breach: a held hull left the bay and reached no workshop; staff place her by hand");
            continue;
        }
        record.berth = Some(BerthRow {
            workshop: room,
            carrier: None,
            bay_room: None,
            slot: None,
            by,
            at_ic: ctx.w.world_ic_now().at,
        });
        put_bay_hull(ctx, &record);
    }
}

/// **A house's stow** (doc 41 cut RS.6a): a hull berthed on a workshop floor goes into a free
/// slot of `carrier`'s bay, by the house's own hand (no actor's right is read: the caller is the
/// port tick laying up a house it owns). Idempotent: a hull already in that bay is left.
///
/// # Errors
///
/// The refusal, as a sentence; nothing is written on any.
pub(crate) fn stow_house(ctx: &mut Ctx<'_>, hull: ObjId, carrier: ObjId) -> Result<(), String> {
    if crate::surface::act::where_is(ctx.w, hull)
        == (crate::surface::act::Where::Bay { hull: carrier })
    {
        return Ok(());
    }
    let mut record = record_of(ctx.w, hull).map_err(str::to_owned)?;
    let (room, slot) = free_bay_slot(ctx.w, carrier, hull)?;
    mush_sim::place_interior(ctx, hull, room, Some(slot))
        .map_err(|e| format!("Her bay will not take her: {e}"))?;
    record.berth = None;
    put_bay_hull(ctx, &record);
    Ok(())
}

/// **A free slot of `carrier`'s bay that takes `hull`** (doc 39 cut IN.9): the bay's rigs
/// compartment with room for one more, its mass limit at least her catalogue mass.
fn free_bay_slot(
    w: &World,
    carrier: ObjId,
    hull: ObjId,
) -> Result<(ObjId, mush_sim::SlotId), String> {
    let view = crate::port::ship::hull_view(w, carrier)
        .ok_or("Her bay will not take her: that is no carrier.")?;
    let (bay, room) = view
        .bay()
        .ok_or("Her bay will not take her: that carrier has no bay.")?;
    let slot = mush_sim::SlotId::new(bay.slot.id.as_str())
        .ok_or("Her bay will not take her: the bay's slot is misconfigured.")?;
    let maximum = usize::try_from(bay.slot.maximum_items).unwrap_or(0);
    let full = w
        .module_state::<SpatialState>()
        .is_some_and(|state| state.occupants(room, Some(&slot)) >= maximum);
    if full {
        return Err("Her bay will not take her: every slot is taken.".to_owned());
    }
    let limit = crate::port::owned::slot_mass_limit_kg(bay);
    let mass = crate::port::owned::catalogue_mass_kg(w, hull);
    if let (Some(limit), Some(mass)) = (limit, mass)
        && mass > limit
    {
        return Err(format!(
            "Her bay will not take her: she masses {} t and the slot takes {} t.",
            mass.div_ceil(1000),
            limit / 1000
        ));
    }
    Ok((room, slot))
}

/// **The commission's yard berth** (doc 39 cut IN.9, §4.13): a rig the yard built on the
/// workshop floor is berthed there with no carrier, so `held` locks her to the floor and
/// `+bay` quotes her; `+bay/stow <hull>=<carrier>` takes her off it. Idempotent.
///
/// # Errors
///
/// The sentence for a bay record that will not read.
pub(crate) fn berth_built(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    workshop: ObjId,
    by: ObjId,
) -> Result<(), String> {
    let mut record = record_of(ctx.w, hull).map_err(str::to_owned)?;
    if record.berth.is_some() {
        return Ok(());
    }
    record.berth = Some(BerthRow {
        workshop,
        carrier: None,
        bay_room: None,
        slot: None,
        by,
        at_ic: ctx.w.world_ic_now().at,
    });
    put_bay_hull(ctx, &record);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gc8_q36_33_who_may_do_what() {
        for act in [
            Act::Repair,
            Act::Rearm,
            Act::Refit,
            Act::Strip,
            Act::Scrap,
            Act::Berth,
        ] {
            assert!(permits(Right::Title, act), "the title holder may {act:?}");
        }
        // A grant holder: repair, rearm and refit; never scrap.
        assert!(permits(Right::Grant, Act::Repair));
        assert!(permits(Right::Grant, Act::Rearm));
        assert!(permits(Right::Grant, Act::Refit));
        assert!(!permits(Right::Grant, Act::Scrap));
        // A lessee: repair and rearm, no refit, strip or scrap.
        assert!(permits(Right::Lessee, Act::Repair));
        assert!(permits(Right::Lessee, Act::Rearm));
        assert!(!permits(Right::Lessee, Act::Refit));
        assert!(!permits(Right::Lessee, Act::Strip));
        assert!(!permits(Right::Lessee, Act::Scrap));
        // A salvor with custody: repair at their own risk, never strip or scrap.
        assert!(permits(Right::Salvor, Act::Repair));
        assert!(!permits(Right::Salvor, Act::Strip));
        assert!(!permits(Right::Salvor, Act::Scrap));
        assert!(!permits(Right::Salvor, Act::Refit));
    }
}
