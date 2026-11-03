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
//! A carrier bay works only while its carrier is docked at a port with a workshop, whose
//! operator is paid; the workshop supplies a replaced part, the hold everything else.

pub mod act;
pub mod command;
pub mod jobs;
pub mod record;
pub mod strip;

pub use jobs::{BayJob, BayQuote, CARRIER_TIME_FACTOR, FacilityClass, QuotedJob};
pub use record::{
    BAY_PREFIX, BAY_SCHEMA, BayHull, BayHullState, BayOrder, BerthRow, MAX_ORDER_JOBS, OrderStatus,
    Right, bay_key, bay_key_owner,
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
use mush_tactics::field_ops::{ComponentCondition, CustodyTitleStatus};
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

// ---- where the work is done -------------------------------------------------------------

/// **A facility a hull stands in** (Q-36-35): its class, the room, and the port whose
/// workshop's operator is paid and supplies the parts.
#[derive(Clone, Debug)]
pub struct Facility {
    /// The class.
    pub class: FacilityClass,
    /// The room the work is done in: the workshop room, or the carrier's bay room.
    pub room: ObjId,
    /// The carrier, for a carrier bay.
    pub carrier: Option<ObjId>,
    /// The port entry.
    pub port: CanonicalId,
    /// The operator org.
    pub operator: OrgId,
    /// Its scoped credit account.
    pub operator_account: AccountId,
    /// The operator's multiplier, per mille.
    pub multiplier_permille: u32,
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

/// **The carrier whose vehicle bay is `room`**, and that bay's room: every programme hull the
/// port maps record, asked for its bay rooms.
fn carrier_of_bay(w: &World, room: ObjId) -> Option<ObjId> {
    let store = w.module_state::<TacsimStore>()?;
    store.port_maps().find_map(|map| {
        map.hulls.iter().find_map(|row| {
            let hull = ObjId(row.hull);
            let view = crate::port::ship::hull_view(w, hull)?;
            crate::port::vehicle::bay_rooms(w, &view)
                .contains(&room)
                .then_some(hull)
        })
    })
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
                port,
                operator,
                operator_account,
                multiplier_permille,
            });
        }
    }
    let (bay, _) = interior_of(w, hull).ok_or("That hull is in no bay and no workshop.")?;
    let carrier = carrier_of_bay(w, bay).ok_or("That hull is in no carrier's bay.")?;
    let crate::port::ship::Berth::Docked { map, def } = crate::port::ship::berth_of(w, carrier)
    else {
        return Err(
            "A carrier's bay works on its hulls only while she is docked at a port with a \
             workshop to supply it."
                .to_owned(),
        );
    };
    let (operator, operator_account, multiplier_permille) = operator_of(def, map).ok_or(
        "A carrier's bay works on its hulls only while she is docked at a port with a workshop \
         to supply it.",
    )?;
    Ok(Facility {
        class: FacilityClass::CarrierBay,
        room: bay,
        carrier: Some(carrier),
        port: map.entry.clone(),
        operator,
        operator_account,
        multiplier_permille,
    })
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
    if carrier_right(ctx.w, actor, berth.carrier)
        || right_held_by(ctx, berth.by, record.hull).is_some()
    {
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

/// Whether `actor` is the live lessee of the rig `hull`.
fn leases(w: &World, actor: ObjId, hull: ObjId) -> bool {
    w.module_state::<TacsimStore>().is_some_and(|s| {
        s.tv_leases_on(hull)
            .any(|l| l.pilot == actor && l.status == crate::port::tv::TvLeaseStatus::Active)
    })
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
    if unit_grant(ctx, hull, UnitCapability::MaintainRefit) {
        return Some(Right::Grant);
    }
    if leases(ctx.w, actor, hull) {
        return Some(Right::Lessee);
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
    let quote = jobs::quote(
        &unit,
        &spec,
        &anatomy,
        store.definitions(),
        goods,
        &tariff,
        facility.class,
    )?;
    Ok((facility, quote))
}

// ---- the hold ---------------------------------------------------------------------------

/// **The goods `jobs` draw** (Q-36-31, Q-36-36): each rearm's munitions good, summed by good,
/// and the `industrial_parts` tonnes their points consume at the tariff's rate. Beam charge
/// and a replaced part draw nothing.
#[must_use]
pub fn order_goods(jobs: &[QuotedJob]) -> Vec<GoodUnits> {
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
    if let Ok(tariff) = crate::definitions::workshop::workshop_tariff() {
        let parts = BayQuote::parts_t(jobs, &tariff);
        if parts > 0
            && let Ok(good) = CanonicalId::new(PARTS_GOOD)
        {
            goods.insert(good, parts);
        }
    }
    goods
        .into_iter()
        .filter(|(_, units)| *units > 0)
        .map(|(good, units)| GoodUnits { good, units })
        .collect()
}

/// **The carrier whose hold serves `hull` at `facility`**: the carrier whose bay it stands in,
/// or the carrier it was berthed from, docked at the facility's port.
///
/// # Errors
///
/// The sentence for a hull with no carrier here.
pub fn source_carrier(w: &World, hull: ObjId, facility: &Facility) -> Result<ObjId, String> {
    let carrier = match facility.class {
        FacilityClass::CarrierBay => facility.carrier,
        FacilityClass::Workshop => match w
            .module_state::<TacsimStore>()
            .and_then(|s| s.bay_hull(hull))
        {
            Some(BayHullState::Present(row)) => row.berth.as_ref().map(|b| b.carrier),
            _ => None,
        },
    }
    .ok_or("That hull came off no carrier, so there is no hold to draw from or lay into.")?;
    match crate::port::ship::berth_of(w, carrier) {
        crate::port::ship::Berth::Docked { map, .. } if map.entry == facility.port => Ok(carrier),
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
/// be clean, unfitted and of the slot's definition; it is consumed when the work is done.
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
            part.parent_hull.0, rightful_owner.0
        ));
    }
    if part.fitted_into.is_some() {
        return Err("That part has already been fitted.".to_owned());
    }
    take(ctx, hull, slot, Some((lot, part)))
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
    let definition = mush_tactics::module::unit_of(w, hull)
        .and_then(|u| u.component(&component).map(|c| c.definition.clone()));
    if definition.as_ref() != Some(&part.definition) {
        return Err("That part does not fit that slot.".to_owned());
    }
    Ok(QuotedJob {
        job: BayJob::Replace {
            component,
            lot: Some(lot),
        },
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
    if ctx.w.get(ctx.actor).and_then(|o| o.location) != Some(facility.room) {
        return Err(format!(
            "Orders for that hull are taken in the {} it stands in.",
            facility.class.word()
        ));
    }
    let right = right_of(ctx, hull).ok_or(NO_RIGHT)?;
    let picked = match &fit {
        Some((lot, part)) => {
            let job = fit_job(ctx.w, hull, &quote, selector, *lot, part)?;
            if job.job.needs() > facility.class {
                return Err(
                    "That work needs a workshop; berth the hull at the port's workshop.".to_owned(),
                );
            }
            Some(vec![job])
        }
        None => quote.select(selector),
    };
    let picked = picked.ok_or_else(|| {
        if quote.elsewhere.iter().any(|j| {
            selector.eq_ignore_ascii_case(j.job.kind_word())
                || selector.eq_ignore_ascii_case(j.job.target())
        }) {
            "That work needs a workshop; berth the hull at the port's workshop.".to_owned()
        } else {
            "Nothing in the quote matches that; +bay/quote lists the work.".to_owned()
        }
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
    dry_run(ctx, hull, ctx.actor, &picked, &fitted)
        .map_err(|error| format!("That work cannot be done as ordered: {error}."))?;
    // GC.8b: the goods come from the carrier's hold, checked now and drawn at completion.
    let goods = order_goods(&picked);
    let vault = if goods.is_empty() && fit.is_none() {
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
    let fee_minor = i64::try_from(BayQuote::cost_minor(&picked)).map_err(|_| "Fee too large.")?;
    let unit_revision = mush_tactics::module::unit_of(ctx.w, hull)
        .map(TacticalUnit::revision)
        .ok_or(NO_HULL)?;
    let payer = crate::port::credit::own_account(ctx.w, ctx.actor)
        .map(|a| a.id)
        .ok_or("You hold no credit account to pay from.")?;
    let now = ctx.w.now();
    let spec = fee_spec(
        payer,
        facility.operator,
        facility.operator_account,
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
        contract: contract.0,
        munitions_vault: vault,
        goods,
        draw_act: None,
        unit_revision,
        started_ic: started,
        due_ic: started.saturating_add(secs),
        status: OrderStatus::Open,
        closed_ic: None,
        note: None,
    };
    record.order = Some(order.clone());
    if let Err(error) = record.check() {
        tracing::error!(%error, hull = hull.0, "tacsim: +bay/order wrote a record that fails its own check");
    }
    put_bay_hull(ctx, &record);
    Ok(Taken { order })
}

/// Close the open order as cancelled with `note`, releasing the escrow.
fn close_cancelled(ctx: &mut Ctx<'_>, mut record: BayHull, note: &str) {
    let Some(order) = record.order.as_mut() else {
        return;
    };
    if !matches!(
        mush_economy::cancel_contract(ctx, ContractId(order.contract)),
        Ok(true)
    ) {
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
        FacilityClass::Workshop => w.get(hull).and_then(|o| o.location),
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
    jobs: &[QuotedJob],
    fitted: &BTreeMap<ComponentId, ComponentCondition>,
) -> Result<(), String> {
    let (before, _, anatomy) = unit_parts(ctx.w, hull)?;
    let custody = CustodyTitleStatus::ResolvedTitle { owner: requester };
    jobs::work(
        &before,
        &anatomy,
        jobs,
        &BTreeMap::new(),
        fitted,
        requester,
        &custody,
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
    if ctx.w.world_ic_now().at < order.due_ic || order.draw_act.is_some() {
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
    if still.is_none() || still != order.munitions_vault {
        close_cancelled(
            ctx,
            record,
            "her carrier's hold is no longer here to draw from",
        );
        return;
    }
    if let Err(error) = dry_run(
        ctx,
        hull,
        order.requester,
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
        vault: order.munitions_vault,
        kind: OwedKind::Draw {
            order: order.seq,
            goods: order.goods.clone(),
            fit,
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
    let fitted = fitted_of(ctx, &order);
    let custody = CustodyTitleStatus::ResolvedTitle {
        owner: order.requester,
    };
    let at = ctx.w.now();
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
    let operator = facility_operator(ctx.w, &order);
    let paid = operator.is_some_and(|operator| {
        matches!(
            mush_economy::accept_contract_as(
                ctx,
                ContractId(order.contract),
                &PrincipalId::Organization(operator)
            ),
            Ok(AcceptOutcome::Accepted {
                activated: true,
                ..
            })
        )
    });
    if !paid {
        close_cancelled(ctx, record, "the facility could not take the fee");
        return;
    }
    if let Some(plan) = plan
        && let Err(error) = mush_tactics::module::apply_plan(ctx, hull, &plan)
    {
        tracing::error!(
            hull = hull.0, contract = order.contract, %error,
            "tacsim: a paid bay order's work was refused by the unit; staff refund the fee and \
             repair by hand"
        );
    }
    let mut record = record;
    if let Some(row) = record.order.as_mut() {
        row.status = OrderStatus::Done;
        row.closed_ic = Some(ctx.w.world_ic_now().at);
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
            OwedKind::Mint { why, lots } => {
                strip::land_mint(ctx, hull, owed.by, why, lots);
                ctx.out.sys_to_actor(
                    owed.by,
                    "tacsim",
                    format!(
                        "The {why} of #{} is laid into the carrier's hold: {}.",
                        hull.0,
                        lots.iter()
                            .map(|l| format!("{} {} ({})", l.quantity, l.commodity, l.lot))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
            OwedKind::Hot { .. } => {}
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
            OwedKind::Mint { why, .. } => {
                // The strip or the scrap was never written to the hull (`strip::land_mint`
                // writes it when the mint lands): a lost mint destroys nothing.
                put_bay_hull(ctx, &record);
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
                    "tacsim: a bay act was refused every try; its flag was not written, staff \
                     write it by hand"
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

/// The operator org of the port an order's facility belongs to.
fn facility_operator(w: &World, order: &BayOrder) -> Option<OrgId> {
    match order.class {
        FacilityClass::Workshop => workshops(w)
            .into_iter()
            .find(|(_, _, _, room)| *room == order.facility)
            .and_then(|(_, def, map, _)| operator_of(def, map))
            .map(|(org, _, _)| org),
        FacilityClass::CarrierBay => {
            let carrier = carrier_of_bay(w, order.facility)?;
            match crate::port::ship::berth_of(w, carrier) {
                crate::port::ship::Berth::Docked { map, def } => {
                    operator_of(def, map).map(|(org, _, _)| org)
                }
                _ => None,
            }
        }
    }
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
        finish(ctx, hull);
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
    let berths = def.workshop.as_ref().map_or(0, |w| w.berths);
    let used = ctx
        .w
        .module_state::<TacsimStore>()
        .map_or(0, |s| s.bay_berthed_in(room).len());
    if u32::try_from(used).unwrap_or(u32::MAX) >= berths {
        return Err("Every berth in the workshop is taken.".to_owned());
    }
    mush_sim::unplace(ctx, hull, "tacsim.bay.berth")?;
    ctx.w.move_to(hull, room).map_err(|e| e.to_string())?;
    record.berth = Some(BerthRow {
        workshop: room,
        carrier,
        bay_room,
        slot,
        by: ctx.actor,
        at_ic: ctx.w.world_ic_now().at,
    });
    put_bay_hull(ctx, &record);
    Ok(room)
}

/// **`stow`** (Q-36-53): a berthed hull goes back to the bay slot it came from, its carrier
/// still docked at this port.
///
/// # Errors
///
/// The refusal, as a sentence; nothing is written on any.
pub fn stow(ctx: &mut Ctx<'_>, hull: ObjId) -> Result<ObjId, String> {
    let mut record = record_of(ctx.w, hull).map_err(str::to_owned)?;
    if record.open().is_some() {
        return Err(HELD_OPEN.to_owned());
    }
    let row = record.berth.clone().ok_or("That hull is not berthed.")?;
    if ctx.w.get(ctx.actor).and_then(|o| o.location) != Some(row.workshop) {
        return Err("Hulls are stowed from the workshop they stand in.".to_owned());
    }
    let right = right_of(ctx, hull).ok_or(NO_RIGHT)?;
    if !permits(right, Act::Berth) {
        return Err(NO_RIGHT.to_owned());
    }
    berth_stands(ctx, ctx.actor, &record)?;
    let docked_here = match crate::port::ship::berth_of(ctx.w, row.carrier) {
        crate::port::ship::Berth::Docked { map, def } => {
            crate::port::workshop::room(ctx.w, &map.entry, def)
                .ok()
                .flatten()
                == Some(row.workshop)
        }
        _ => false,
    };
    if !docked_here {
        return Err("Her carrier is not docked at this port.".to_owned());
    }
    mush_sim::place_interior(ctx, hull, row.bay_room, row.slot.clone())
        .map_err(|e| format!("Her bay will not take her: {e}"))?;
    record.berth = None;
    put_bay_hull(ctx, &record);
    Ok(row.carrier)
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
