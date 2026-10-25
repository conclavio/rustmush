//! **The chartered hull** (doc 18 cut IC6; doc 18 §4.2 "The airlock and the
//! berth", "Compartments"; D-18-23, D-18-34, D-18-39; 18a §3; lead rulings L7,
//! L8, L11, L15, L18, L19, L21 of the IC6 brief).
//!
//! - `+ship` and `+ship/hold` read the pilot's chartered hull — where it is,
//!   what its hold vault's pools hold against both limits, the tank and the
//!   locker — anywhere. The hull is the **live lease's** (D-18-23,
//!   [`crate::port::coop::live_charter`]), never the enrolment document's: an
//!   unreadable document darkens only the programme line (L21). Since cut IC13 the
//!   gate is [`crate::port::coop::helm_of`]: the live charter, else the hull the
//!   pilot holds title to (a graduate), whose programme line reads
//!   [`SHIP_GRADUATED`].
//! - `+ship/board` (in the berth gallery where the tenant's hull is docked)
//!   places the tenant `Interior` in the hull's airlock space through
//!   `mush_sim::place_interior` (the berths cap refuses a third), and
//!   `+ship/leave` (in that airlock while docked) unplaces, keeping the room,
//!   and moves them to the gallery — `put_out`'s shape — then stamps the
//!   gallery's port through IC1's `ic_return::stamp` (L15). Board is gated on
//!   the live lease and the placement; leave on the placement alone, so a pilot
//!   whose charter ends aboard is never stranded (L7, an owner item against
//!   D-18-34).
//! - [`ShipLook`] adds a status line to every hull room's `look` (18a §3, with
//!   the mark, L18) and, in the rooms `interior.gauges` lists, the gauge lines
//!   ([`crate::port::gauges`]) read from the hold's pools at look time —
//!   **no pack text carries a load** (D-18-39). Doc 18 cut IC9 added the leg and
//!   the ETA to the status line and `+ship`'s first line, the departing phase,
//!   `+ship/plan` and `+ship/undock` (`crate::port::voyage`), and the refusal of
//!   `+ship/board` and `+ship/leave` while a clearance is filed. Doc 18 cut IC11
//!   added `+ship/orders` and `+ship/order` (`crate::port::orders`) and the
//!   departure hour of a queued depart on the status line and `+ship`'s first
//!   line; the burn and reserve gauge lines go to IC12's content pass (IC11 lead
//!   ruling L21).

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{
    CanonicalId, CmdError, Ctx, EventHook, HookPoint, HookReadContext, ObjId, World, WorldEvent,
    name_span,
};
use mush_economy::{CustodyRef, EconomyStore, VaultId};
use mush_inventory::record::EndpointRef;
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use mush_sim::data::{SpatialParent, SpatialState};

use crate::commands::NOT_ENABLED;
use crate::definitions::content::LANE_PACK;
use crate::definitions::{BayDef, GoodsTable, InteriorDef, LogisticsDef};
use crate::port::capacity::Compartment;
use crate::port::coop::{
    EnrolmentStatus, NO_CHARTER, ProgrammePort, Standing, helm_of, name_of, port_name,
};
use crate::port::def::PortDef;
use crate::port::gauges::{self, DARK, Reading};
use crate::port::map::{HullRow, PortMap};
use crate::port::readiness::report_ports;
use crate::port_tick::PoolReadError;
use crate::store::TacsimStore;

/// The verb.
pub const SHIP_VERB: &str = "+ship";

/// `+ship`'s help (doc 18 §4.2).
pub const SHIP_HELP: &str = "+ship  --  Your chartered Dray: where it is, what its hold carries \
     against both of its limits (tonnes and cubic metres), its tank and its stores locker, and \
     your programme standing. It reads anywhere.\n+ship/board  in the berth gallery where your \
     Dray is cradled, cycle its lock and go aboard.\n+ship/leave  in your Dray's Airlock while it \
     is docked, cycle out into the berth gallery.\n+ship/hold  the hold, the tank and the locker, \
     good by good, in tonnes and cubic metres.\n+ship/plan [<port>]  a dry run of the passage to \
     <port> (or every port this berth clears for): the hold's room by mass and by packed volume, \
     the payload, the burn, the stores and the passage time. It writes nothing.\n+ship/undock \
     <port>  from aboard your Dray, docked, file the passage and take her out now; the burn is \
     committed when the clearance is granted; to leave at a later hour, queue +ship/order \
     depart.\n+ship/lift <port>  the same as +ship/undock: the word for lifting off a pad (or \
     out of a berth).\n+ship/orders  the sailing plan: each order, its state, the next one due (UTC, IC \
     and real time), any hold, and the last eight lines of its log. It reads anywhere.\n\
     +ship/order sell <good> <qty|all> [min=<cr>] [short=partial|skip|hold] | buy <good> \
     <qty|fill> [max=<cr>] [short=…] | depart <port> [at=now|HH:MM|daily HH:MM|+Nh|+Nm] \
     [until=+Nh|+Nm] | stop | del <n> | move <n>=<pos> | clear | pause | resume | skip <n>  aboard \
     your Dray, or in the port where she is docked: file and amend the sailing plan the port \
     runs under your signature while you are off watch.";

/// The programme's one hull class (cut IC8 may read it from the spec).
pub const HULL_CLASS: &str = "Dray";

/// **The refitted hull's class word** (doc 20 cut SM.3a; D-20-7): a programme
/// hull bound to a refit unit reads `Dray-P` wherever `+ship` and the status
/// lines name its class ([`HullView::class_word`]).
pub const HULL_CLASS_PROSPECTOR: &str = "Dray-P";

/// **18a §3's status line, docked** (L18: with the mark): `{name}`, `{mark}`,
/// `{port}` replaced.
pub const STATUS_DOCKED: &str = "Aboard the {name} ({mark}), docked at {port}.";
/// The status line under way (doc 18 cut IC9: the leg and the ETA, 18a §3's
/// form "under way, leg 2 of 2, Kirkwood astern, ETA 15:06 UTC"): `{name}`,
/// `{mark}`, `{n}`, `{m}`, `{astern}` (`, {short} astern` once a boundary is
/// passed), `{hhmm}`; at world rate 0 `ETA unknown.`.
pub const STATUS_UNDER_WAY: &str =
    "Aboard the {name} ({mark}), under way, leg {n} of {m}{astern}, ETA {hhmm} UTC.";
/// The status line while a clearance is filed (doc 18 cut IC9): `{name}`,
/// `{mark}`, `{port}`.
pub const STATUS_CLEARED: &str = "Aboard the {name} ({mark}), cleared for {port}.";
/// The status line docked with a queued departure due (doc 18 cut IC11; §2 step
/// 15): `{name}`, `{mark}`, `{port}`, `{hhmm}`, `{span}`.
pub const STATUS_DOCKED_DEPARTS: &str =
    "Aboard the {name} ({mark}), docked at {port}; departs {hhmm} UTC (in {span}).";
/// The status line anywhere else.
pub const STATUS_ELSEWHERE: &str = "Aboard the {name} ({mark}).";

/// **`+ship`'s first line, docked** (doc 18 §2 step 5's form; L19): `{name}`,
/// `{class}`, `{port}`, `{gallery}` replaced.
pub const SHIP_DOCKED: &str = "{name} ({class}): docked {port}, {gallery}.";
/// `+ship`'s first line under way (doc 18 cut IC9): `{name}`, `{class}`,
/// `{port}`, `{n}`, `{m}`, `{icdays}`, `{hhmm}`, `{day}`; at world rate 0 the
/// line ends `…; arrives in {icdays} IC days.`.
pub const SHIP_UNDER_WAY: &str = "{name} ({class}): under way for {port}, leg {n} of {m}; arrives in {icdays} IC days, about {hhmm} UTC {day}.";
/// `+ship`'s first line while a clearance is filed (doc 18 cut IC9): `{name}`,
/// `{class}`, `{port}`.
pub const SHIP_CLEARED: &str = "{name} ({class}): cleared for {port}; the lock is sealing.";
/// `+ship`'s first line docked with a queued departure due (doc 18 cut IC11):
/// `{name}`, `{class}`, `{port}`, `{gallery}`, `{hhmm}`, `{span}`.
pub const SHIP_DOCKED_DEPARTS: &str =
    "{name} ({class}): docked {port}, {gallery}; departs {hhmm} UTC (in {span}).";
/// `+ship`'s first line anywhere else.
pub const SHIP_ELSEWHERE: &str = "{name} ({class}): not at a berth.";
/// `+ship`'s programme line when the enrolment document does not read (L21).
pub const SHIP_BOOKS_DARK: &str = "Programme: the Co-op's books cannot be read.";
/// `+ship`'s programme line for a graduate (doc 18 cut IC13); `{name}` the hull's.
pub const SHIP_GRADUATED: &str = "Programme: graduated; the {name} is yours.";
/// `+ship`'s hold line when a pool or the goods table does not read.
pub const HOLD_DARK: &str = "Hold: the gauges are dark.";

/// `+ship/board` while placed in a space already.
pub const ALREADY_ABOARD: &str = "You are already aboard.";
/// `+ship/board` away from a berth gallery.
pub const BOARD_WHERE: &str = "You board from the berth gallery where your Dray is cradled.";
/// The hull is cradled at another gallery (`{name}` replaced).
pub const NOT_HERE: &str = "The {name} is not cradled at this gallery.";
/// The hull is under way (`{name}` replaced).
pub const UNDER_WAY_LOCK: &str = "{name} is under way; nothing crosses its lock.";
/// The placement refused.
pub const LOCK_STUCK: &str = "The lock will not cycle right now.";
/// `+ship/leave` while not aboard a programme hull.
pub const LEAVE_NOT_ABOARD: &str = "You are not aboard a Dray.";
/// `+ship/leave` aboard but not in the airlock (`{airlock}` replaced; F3: a
/// refusal that names the room).
pub const LEAVE_WHERE: &str = "You can only leave by the {airlock}.";
/// `+ship/leave` with the hull at no berth (`{name}` replaced).
pub const LEAVE_SEALED: &str = "The {name} is not at a berth; the lock stays sealed.";
/// **Doc 20 cut SM.2**: `+ship`'s first line on a site's pad: `{name}`, `{class}`, `{port}`.
pub const SHIP_SETTLED: &str = "{name} ({class}): settled on the pad at {port}.";
/// The same with a queued departure due: `{name}`, `{class}`, `{port}`, `{hhmm}`, `{span}`.
pub const SHIP_SETTLED_DEPARTS: &str =
    "{name} ({class}): settled on the pad at {port}; departs {hhmm} UTC (in {span}).";
/// The hull room's status line on a pad: `{name}`, `{mark}`, `{port}`.
pub const STATUS_SETTLED: &str = "Aboard the {name} ({mark}), settled on the pad at {port}.";
/// The same with a queued departure due: `{name}`, `{mark}`, `{port}`, `{hhmm}`, `{span}`.
pub const STATUS_SETTLED_DEPARTS: &str =
    "Aboard the {name} ({mark}), settled on the pad at {port}; departs {hhmm} UTC (in {span}).";
/// `+ship/leave` on a pad (decision 10: a soul left on a pad with no exits and no `+ic`
/// route would be stranded when the pilot lifts): `{name}`, `{port}`.
pub const LEAVE_PAD: &str =
    "The {name} is settled on the pad at {port}; the lock stays sealed until she docks.";

/// **One programme hull, resolved.**
#[derive(Clone, Debug)]
pub struct HullView<'a> {
    /// The programme's port.
    pub port: ProgrammePort<'a>,
    /// The hull's map row.
    pub row: &'a HullRow,
    /// **The unit the hull reads as** (doc 20 cut SM.3a; D-20-7): its live
    /// `tactics.unit` record's definition — the programme unit, or a refit unit
    /// `+coop/refit` bound it to — else the programme unit
    /// (`crate::port::hull::resolve_hull_unit`).
    pub unit: CanonicalId,
    /// **A unit record claims the hull but boot held it** (frozen, migrating,
    /// quarantined; doc 20 cut SM.3a, reviewer F6). The view then reads the
    /// programme unit's rooms so `airlock`, `stowage` and `room` still work, and
    /// is **dark** wherever the unit matters: the planner answers
    /// `PlanError::Unreadable`, the capacity check reads `Unreadable`, `+tv`
    /// answers `TV_DARK` and `+coop/refit` `REFIT_FAILED`. Only on a world whose
    /// latched programme authors `refits` (lane v10): on a v9 world a held
    /// record reads as the programme unit, as before.
    pub unit_held: bool,
    /// That unit's interior template.
    pub interior: &'a InteriorDef,
    /// Its logistics block.
    pub logistics: &'a LogisticsDef,
}

impl HullView<'_> {
    /// The hull Thing.
    #[must_use]
    pub fn hull(&self) -> ObjId {
        ObjId(self.row.hull)
    }

    /// The room of template id `id`.
    #[must_use]
    pub fn room(&self, id: &CanonicalId) -> Option<ObjId> {
        self.row.rooms.get(id).map(|obj| ObjId(*obj))
    }

    /// The airlock space.
    #[must_use]
    pub fn airlock(&self) -> Option<ObjId> {
        self.room(&self.interior.space)
    }

    /// The hold stowage (the hold vault's backing).
    #[must_use]
    pub fn stowage(&self) -> Option<ObjId> {
        self.room(&self.interior.store)
    }

    /// **The hull's vehicle bay and its room** (doc 20 cut SM.3a): the first bay
    /// its unit declares, and the room the row records for it. `None` on a
    /// standard Dray (its unit declares no bay) or a bay never laid up.
    #[must_use]
    pub fn bay(&self) -> Option<(&BayDef, ObjId)> {
        let bay = self.interior.bay()?;
        Some((bay, self.room(&bay.room)?))
    }

    /// **Whether the hull is bound to a refit unit** — a Dray-P (doc 20 cut
    /// SM.3a): its unit is not the programme's.
    #[must_use]
    pub fn is_prospector(&self) -> bool {
        self.unit != self.port.programme.unit
    }

    /// **The class word** `+ship` and the status lines print (doc 20 cut SM.3a):
    /// [`HULL_CLASS`], or [`HULL_CLASS_PROSPECTOR`] for a refitted hull.
    #[must_use]
    pub fn class_word(&self) -> &'static str {
        if self.unit.as_str() == "tacsim.unit.halcyon" {
            "Halcyon"
        } else if self.is_prospector() {
            HULL_CLASS_PROSPECTOR
        } else {
            HULL_CLASS
        }
    }
}

/// The view for a programme port, a row: the hull's own unit (doc 20 cut SM.3a,
/// D-20-7; reviewer F6) and that unit's template and logistics.
fn view_of<'a>(w: &'a World, port: ProgrammePort<'a>, row: &'a HullRow) -> Option<HullView<'a>> {
    use crate::port::hull::{HullUnit, live_unit_id, resolve_hull_unit, unit_record_claims};
    let store = w.module_state::<TacsimStore>()?;
    let hull = ObjId(row.hull);
    // The held rule is a v10 world's: a v9 programme authors no refits, so a held
    // record reads as the programme unit there, exactly as before.
    let refits = !port.programme.refits.is_empty();
    let (unit, unit_held) =
        match resolve_hull_unit(live_unit_id(w, hull), unit_record_claims(w, hull)) {
            HullUnit::Live(id) => (id, false),
            HullUnit::Default => (port.programme.unit.clone(), false),
            HullUnit::Held => (port.programme.unit.clone(), refits),
        };
    let (interior, logistics) = crate::port::hull::spec_for(store, &unit)?;
    Some(HullView {
        port,
        row,
        unit,
        unit_held,
        interior,
        logistics,
    })
}

/// **The view of one programme hull** by its Thing.
#[must_use]
pub fn hull_view(w: &World, hull: ObjId) -> Option<HullView<'_>> {
    let port = crate::port::coop::programme_ports(w)
        .into_iter()
        .find(|p| p.map.hulls.iter().any(|r| r.hull == hull.0))?;
    let row = port.map.hulls.iter().find(|row| row.hull == hull.0)?;
    view_of(w, port, row)
}

/// **The programme hull `room` belongs to**, and the room's template id (a
/// scan of the programme port's hull rows).
#[must_use]
pub fn hull_of_room(w: &World, room: ObjId) -> Option<(HullView<'_>, CanonicalId)> {
    let port = crate::port::coop::programme_ports(w)
        .into_iter()
        .find(|p| {
            p.map
                .hulls
                .iter()
                .any(|r| r.rooms.values().any(|id| *id == room.0))
        })?;
    let (row, tid) = port.map.hulls.iter().find_map(|row| {
        row.rooms
            .iter()
            .find(|(_, obj)| **obj == room.0)
            .map(|(tid, _)| (row, tid.clone()))
    })?;
    Some((view_of(w, port, row)?, tid))
}

/// **Where a hull is.**
#[derive(Clone, Copy, Debug)]
pub enum Berth<'a> {
    /// `Direct` on a port's domain.
    Docked {
        /// That port's map.
        map: &'a PortMap,
        /// That port's entry.
        def: &'a PortDef,
    },
    /// On a journey.
    UnderWay,
    /// Anywhere else.
    Elsewhere,
}

/// **Where `hull` is**: `UnderWay` on a journey, `Docked` when `Direct` on a
/// domain some port map records (with that port's entry from the report's lane
/// pack), else `Elsewhere`.
#[must_use]
pub fn berth_of(w: &World, hull: ObjId) -> Berth<'_> {
    if w.module_state::<mush_sim::JourneyBook>()
        .is_some_and(|book| book.under_way(hull))
    {
        return Berth::UnderWay;
    }
    let Some((domain, _)) = mush_sim::pose_of(w, hull) else {
        return Berth::Elsewhere;
    };
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Berth::Elsewhere;
    };
    let Some(map) = store
        .port_maps()
        .find(|map| map.domain == Some(domain.0) && map.pack.as_str() == LANE_PACK)
    else {
        return Berth::Elsewhere;
    };
    let (_, ports) = report_ports(store, w);
    match ports.into_iter().find(|(entry, _)| *entry == map.entry) {
        Some((_, def)) => Berth::Docked { map, def },
        None => Berth::Elsewhere,
    }
}

/// **The hull's hold vault**: the vault backed by its stowage.
#[must_use]
pub fn hold_vault(w: &World, view: &HullView<'_>) -> Option<VaultId> {
    let stowage = view.stowage()?;
    w.module_state::<EconomyStore>()?
        .snapshot()
        .vault_by_backing(&CustodyRef::SpatialInterior(stowage))
        .map(|vault| vault.id)
}

/// **The hold's unreserved room for ore, kilograms** (doc 20 cut SM.4b; the brief's reviewer
/// resolution R6 and decision P11; §19 item 1): [`hold_unreserved`]'s mass entry, read by the
/// `site.map` frame's `hold.room_t` and by a plan's end summary for the tonnes the hold holds.
/// Since doc 20 cut SM.11r the mining plan's compile, its preview and its end summary's room read
/// the room in every measure instead (`crate::surface::plan`'s `room_in_tonnes_of` over
/// [`hold_unreserved`]), never this mass entry alone. `None` as [`hold_unreserved`].
#[must_use]
pub(crate) fn hold_unreserved_kg(
    w: &World,
    view: &HullView<'_>,
    rigs: &[ObjId],
    read: &dyn Fn(EndpointRef, &CanonicalId) -> Result<u64, PoolReadError>,
    read_rig: &dyn Fn(ObjId) -> Result<Option<crate::surface::excursion::RigRecord>, String>,
) -> Option<u64> {
    let mass = CanonicalId::new(crate::definitions::logistics::MASS_KG).ok()?;
    hold_unreserved(w, view, rigs, read, read_rig).map(|room| room.get(&mass).copied().unwrap_or(0))
}

/// **The hold's unreserved room for ore, per measure** (doc 20 §3.5 and §19 item 1: "a hopper
/// never carries what the hold cannot take"; cut SM.4b's function, generalised by cut SM.7 — the
/// brief's decision R8): over the hull's stowage compartments that accept the ore stowage tag
/// (read from the goods table's `metal_ore` row, never a literal), each one's limit less its
/// current load in every measure it limits (`capacity::load` over every good it holds, through
/// `read` at the hold vault), summed per measure — **less each of `rigs`' hopper load** (each good
/// it holds weighed per measure by the goods table; never the hopper's capacity) — **less each of
/// `rigs`' outstanding cycles** (a rig whose record, read through `read_rig`, holds a run: its
/// `Run::outstanding_t` tonnes of the run's good, the cycle in flight included, weighed per
/// measure) — saturating at 0 in each measure. Packed volume counts as mass does: bulky cargo
/// beside the ore leaves less room than its tonnes say. `None` when any pool or record does not
/// read, the goods table is not loaded, a good does not weigh in a measure, or no compartment
/// accepts ore. **Doc 20 cut SM.8** (decision R16): [`hold_unreserved_with`] over the store's
/// goods table — its callers byte-unchanged.
#[must_use]
pub(crate) fn hold_unreserved(
    w: &World,
    view: &HullView<'_>,
    rigs: &[ObjId],
    read: &dyn Fn(EndpointRef, &CanonicalId) -> Result<u64, PoolReadError>,
    read_rig: &dyn Fn(ObjId) -> Result<Option<crate::surface::excursion::RigRecord>, String>,
) -> Option<crate::port::capacity::Load> {
    let goods = w.module_state::<TacsimStore>()?.goods()?;
    hold_unreserved_with(goods, w, view, rigs, read, read_rig)
}

/// **[`hold_unreserved`] over a named goods table** (doc 20 cut SM.8; the brief's decision R16:
/// the unload weighs its preview, its offer, its prepare and its finish with the one table its
/// hold plan carries, `crate::port::market::plan_goods`). The arithmetic is
/// `crate::port::capacity`'s: each ore compartment's room through `room_less` (`limit − load`,
/// summed per measure), then every reserved load — each of `rigs`' hopper contents and its run's
/// outstanding tonnes — off the summed room through `take_room` (a union compartment of the ore
/// compartments' measures and tags), saturating; SM.7's semantics exactly.
#[must_use]
pub(crate) fn hold_unreserved_with(
    goods: &crate::definitions::GoodsTable,
    w: &World,
    view: &HullView<'_>,
    rigs: &[ObjId],
    read: &dyn Fn(EndpointRef, &CanonicalId) -> Result<u64, PoolReadError>,
    read_rig: &dyn Fn(ObjId) -> Result<Option<crate::surface::excursion::RigRecord>, String>,
) -> Option<crate::port::capacity::Load> {
    let ore = CanonicalId::new("metal_ore").ok()?;
    let tag = goods.good(&ore)?.stowage.clone();
    let comps: Vec<(CanonicalId, Compartment)> = compartments(view)
        .into_iter()
        .filter(|(_, comp)| comp.accepts.contains(&tag))
        .collect();
    if comps.is_empty() {
        return None;
    }
    let readings = readings_with(goods, comps, hold_vault(w, view), read).ok()?;
    let mut room = crate::port::capacity::Load::new();
    // The union of the ore compartments, for weighing a reserved load off the summed room: its
    // measures are the room's, its tags every compartment's.
    let mut union = Compartment {
        id: ore,
        accepts: std::collections::BTreeSet::new(),
        limits: BTreeMap::new(),
    };
    for (_, comp, contents) in &readings {
        let left = crate::port::capacity::room_less(goods, comp, contents, &[]).ok()?;
        for (measure, free) in left {
            let slot = room.entry(measure.clone()).or_insert(0);
            *slot = slot.saturating_add(free);
            union.limits.insert(measure, 0);
        }
        union.accepts.extend(comp.accepts.iter().cloned());
    }
    let pool_unit = crate::port::tv::pool_unit(w);
    for rig in rigs {
        let contents =
            crate::port::tv::hopper_contents_with(w, *rig, pool_unit.as_ref(), read).ok()?;
        for (good, qty) in &contents {
            crate::port::capacity::take_room(goods, &union, &mut room, good, *qty).ok()?;
        }
        // Doc 20 cut SM.7: every outstanding, uncompleted cycle of the rig's run.
        if let Some(run) = read_rig(*rig).ok()?.and_then(|record| record.run) {
            crate::port::capacity::take_room(
                goods,
                &union,
                &mut room,
                &run.good,
                run.outstanding_t(),
            )
            .ok()?;
        }
    }
    Some(room)
}

/// The hull's stowage compartments, in stowage order.
pub(crate) fn compartments(view: &HullView<'_>) -> Vec<(CanonicalId, Compartment)> {
    view.logistics
        .stowage
        .iter()
        .map(|(id, def)| (id.clone(), Compartment::from_stowage(id, def)))
        .collect()
}

/// Why a reading failed: a pool that would not read.
fn reading_error(error: &PoolReadError) -> String {
    format!("{error:?}")
}

/// Each compartment's reading: the accepted goods (table order), each read
/// through `read` at the hold vault; no vault → all zeros, no read.
fn readings_with(
    goods: &GoodsTable,
    comps: Vec<(CanonicalId, Compartment)>,
    vault: Option<VaultId>,
    read: &dyn Fn(EndpointRef, &CanonicalId) -> Result<u64, PoolReadError>,
) -> Result<Vec<Reading>, String> {
    let mut out: Vec<Reading> = Vec::with_capacity(comps.len());
    for (id, comp) in comps {
        let mut contents: BTreeMap<CanonicalId, u64> = BTreeMap::new();
        if let Some(vault) = vault {
            for good in goods
                .goods
                .iter()
                .filter(|good| comp.accepts.contains(&good.stowage))
            {
                let qty = read(EndpointRef(vault), &good.id).map_err(|e| reading_error(&e))?;
                if qty > 0 {
                    contents.insert(good.id.clone(), qty);
                }
            }
        }
        out.push((id, comp, contents));
    }
    Ok(out)
}

/// **The hull room's status line** (18a §3; L18): `{name}` is the hull Thing's
/// current name.
#[must_use]
pub fn status_line(w: &World, view: &HullView<'_>) -> String {
    // Doc 18 cut IC9: a hull on passage — departing (a clearance filed) or under
    // way (its leg and ETA) — reads its passage first.
    let template = match (
        crate::port::voyage::passage_of(w, view.hull()),
        berth_of(w, view.hull()),
    ) {
        (Some(passage), _) => crate::port::voyage::status_passage_line(w, &passage),
        (None, Berth::Docked { def, .. }) => {
            // Doc 20 cut SM.2: on a site's pad the hull is settled, not docked.
            let (docked, departs) = if def.site.is_some() {
                (STATUS_SETTLED, STATUS_SETTLED_DEPARTS)
            } else {
                (STATUS_DOCKED, STATUS_DOCKED_DEPARTS)
            };
            // Doc 18 cut IC11: a queued departure due names its hour.
            match crate::port::orders::departs_at(w, view.hull()) {
                Some(due) => departs
                    .replace("{port}", &port_name(def))
                    .replace("{hhmm}", &crate::port::voyage::utc_hhmm(due))
                    .replace(
                        "{span}",
                        &crate::port::voyage::real_span(due.saturating_sub(w.now())),
                    ),
                None => docked.replace("{port}", &port_name(def)),
            }
        }
        (None, Berth::UnderWay) => {
            STATUS_UNDER_WAY.replace(", leg {n} of {m}{astern}, ETA {hhmm} UTC", "")
        }
        (None, Berth::Elsewhere) => STATUS_ELSEWHERE.to_owned(),
    };
    template
        .replace("{name}", &name_of(w, view.hull()))
        .replace("{mark}", &view.row.mark)
}

/// `+ship`'s first line (doc 18 §2 step 5's form; L19).
fn ship_line(w: &World, view: &HullView<'_>) -> String {
    // Doc 18 cut IC9: departing and under way read the passage.
    if let Some(passage) = crate::port::voyage::passage_of(w, view.hull()) {
        return crate::port::voyage::ship_passage_line(w, &passage)
            .replace("{name}", &name_of(w, view.hull()))
            .replace("{class}", view.class_word());
    }
    let template = match berth_of(w, view.hull()) {
        Berth::Docked { map, def } => {
            let gallery = map
                .presentation
                .map_or_else(String::new, |g| name_of(w, ObjId(g)));
            // Doc 20 cut SM.2: on a site's pad the hull is settled, not docked.
            let (docked, departs) = if def.site.is_some() {
                (SHIP_SETTLED, SHIP_SETTLED_DEPARTS)
            } else {
                (SHIP_DOCKED, SHIP_DOCKED_DEPARTS)
            };
            // Doc 18 cut IC11: a queued departure due names its hour.
            let template = match crate::port::orders::departs_at(w, view.hull()) {
                Some(due) => departs
                    .replace("{hhmm}", &crate::port::voyage::utc_hhmm(due))
                    .replace(
                        "{span}",
                        &crate::port::voyage::real_span(due.saturating_sub(w.now())),
                    ),
                None => docked.to_owned(),
            };
            template
                .replace("{port}", &port_name(def))
                .replace("{gallery}", &gallery)
        }
        Berth::UnderWay => SHIP_UNDER_WAY.replace(
            " for {port}, leg {n} of {m}; arrives in {icdays} IC days, about {hhmm} UTC {day}",
            "",
        ),
        Berth::Elsewhere => SHIP_ELSEWHERE.to_owned(),
    };
    template
        .replace("{name}", &name_of(w, view.hull()))
        .replace("{class}", view.class_word())
}

/// Say one line to the actor under the `tacsim` tag.
fn say(ctx: &mut Ctx<'_>, line: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "tacsim", line.into());
}

/// An unattributed narrative line, as the kernel's movement lines are
/// (`ic_return`'s shape).
fn narrative(text: RichText) -> OutputEvent {
    OutputEvent::Narrative {
        kind: NarrativeKind::Emit,
        actor: None,
        text,
    }
}

/// The refusal for an actor with no live charter: ended, when their enrolment
/// was revoked, else [`NO_CHARTER`].
pub(crate) fn no_charter(w: &World, actor: ObjId) -> CmdError {
    let ended = w
        .module_state::<TacsimStore>()
        .and_then(|store| store.enrolment_of(actor))
        .filter(|e| e.status == EnrolmentStatus::Revoked)
        .map(|e| format!("Your charter of the {} ({}) has ended.", e.name, e.mark));
    CmdError::msg(ended.unwrap_or_else(|| NO_CHARTER.to_owned()))
}

/// **`+ship`, `+ship/hold`, `+ship/board`, `+ship/leave`** (doc 18 §4.2).
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor.
pub(crate) fn ship_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let enabled = ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled);
    if !enabled {
        return Err(CmdError::msg(NOT_ENABLED));
    }
    // Doc 18 cut IC11: the sailing plan's read and its edits, ahead of the rest.
    if ctx.switches.has("orders") {
        crate::port::orders::orders_cmd(ctx)
    } else if ctx.switches.has("order") {
        crate::port::orders::order_cmd(ctx)
    } else if ctx.switches.has("board") {
        board(ctx)
    } else if ctx.switches.has("plan") {
        crate::port::voyage::plan_cmd(ctx)
    } else if ctx.switches.has("undock") || ctx.switches.has("lift") {
        // Doc 20 cut SM.2: `+ship/lift` is `+ship/undock`, the word for a pad.
        crate::port::voyage::undock_cmd(ctx)
    } else if ctx.switches.has("leave") {
        leave(ctx)
    } else if ctx.switches.has("hold") {
        hold(ctx)
    } else {
        status(ctx)
    }
}

/// Render `view`'s hold readings, read through the command lens, with
/// `render` (the layer's goods table, never re-parsed).
fn with_readings<T>(
    ctx: &Ctx<'_>,
    view: &HullView<'_>,
    render: impl FnOnce(&GoodsTable, &[Reading]) -> Result<T, String>,
) -> Result<T, String> {
    let goods = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
        .ok_or_else(|| "the goods table is not loaded".to_owned())?;
    let vault = hold_vault(ctx.w, view);
    let read = |endpoint: EndpointRef, good: &CanonicalId| {
        crate::port_tick::read_pool(ctx, endpoint, good)
    };
    let readings = readings_with(goods, compartments(view), vault, &read)?;
    render(goods, &readings)
}

/// **The hull's compartment readings** through the command lens (doc 18 cut IC9:
/// `+ship/plan` reads the same reading the gauges use, IC6's surface note).
///
/// # Errors
///
/// A pool that would not read, or the goods table absent.
pub(crate) fn readings_of(ctx: &Ctx<'_>, view: &HullView<'_>) -> Result<Vec<Reading>, String> {
    with_readings(ctx, view, |_, readings| Ok(readings.to_vec()))
}

/// `+ship`: three lines (doc 18 §2 step 5's form; L19, L21).
fn status(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let Some(charter) = helm_of(ctx.w, actor) else {
        return Err(no_charter(ctx.w, actor));
    };
    let lines = {
        let Some(view) = hull_view(ctx.w, charter.hull) else {
            return Err(no_charter(ctx.w, actor));
        };
        let first = ship_line(ctx.w, &view);
        let hold = match with_readings(ctx, &view, |goods, readings| {
            gauges::ship_hold_line(goods, readings).map_err(|e| e.to_string())
        }) {
            Ok(line) => line,
            Err(error) => {
                tracing::warn!(hull = charter.hull.0, %error, "tacsim: +ship could not read the hold");
                HOLD_DARK.to_owned()
            }
        };
        let programme = match charter.standing {
            Standing::Fleet => {
                "Programme: Concord Fleet; current economic command assignment.".into()
            }
            // Cut IC13: a graduate's line names the hull (the graduated enrolment's
            // name, else the hull's own).
            Standing::Title => {
                let name = ctx
                    .w
                    .module_state::<TacsimStore>()
                    .and_then(|store| store.enrolment_of(actor))
                    .filter(|e| e.status == EnrolmentStatus::Graduated && e.hull == charter.hull)
                    .map_or_else(|| name_of(ctx.w, charter.hull), |e| e.name.clone());
                SHIP_GRADUATED.replace("{name}", &name)
            }
            Standing::Charter(contract) => {
                let enrolment = ctx
                    .w
                    .module_state::<TacsimStore>()
                    .and_then(|store| store.enrolment_for_lease(contract.0))
                    .cloned();
                match enrolment {
                    Some(e) => {
                        let plural = if e.sales == 1 { "" } else { "s" };
                        let buyout = match mush_economy::InstrumentId::new(crate::economy::CREDIT) {
                            Some(instrument) => mush_economy::money(
                                ctx,
                                &instrument,
                                mush_economy::Amount::from_minor(e.buyout_minor),
                            ),
                            None => e.buyout_minor.to_string(),
                        };
                        format!("Programme: {} sale{plural}; buyout {buyout}.", e.sales)
                    }
                    None => {
                        tracing::warn!(
                            lease = contract.0,
                            "tacsim: +ship found a live charter with no readable enrolment"
                        );
                        SHIP_BOOKS_DARK.to_owned()
                    }
                }
            }
        };
        // Doc 20 cut SM.2 (§1 beat 3): on a pad, the stores line after the hold line.
        let stores = crate::port::voyage::settled_stores_line(ctx, &view);
        let mut lines = vec![first, hold];
        // Only an observer aboard can read the hull's current physical lag.
        // An ashore charter holder must not gain instant remote telemetry.
        if crate::port::voyage::aboard(ctx.w, charter.hull, actor) {
            lines.push(crate::relay_metric::lag_line(ctx, charter.hull));
        }
        lines.extend(stores);
        lines.push(programme);
        lines
    };
    for line in lines {
        say(ctx, line);
    }
    Ok(())
}

/// `+ship/hold`: the hold, good by good.
fn hold(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let Some(charter) = helm_of(ctx.w, actor) else {
        return Err(no_charter(ctx.w, actor));
    };
    let lines = {
        let Some(view) = hull_view(ctx.w, charter.hull) else {
            return Err(no_charter(ctx.w, actor));
        };
        let name = name_of(ctx.w, charter.hull);
        match with_readings(ctx, &view, |goods, readings| {
            gauges::hold_lines(&name, &view.row.mark, goods, readings).map_err(|e| e.to_string())
        }) {
            Ok(lines) => lines,
            Err(error) => {
                tracing::warn!(hull = charter.hull.0, %error, "tacsim: +ship/hold could not read the hold");
                vec![HOLD_DARK.to_owned()]
            }
        }
    };
    for line in lines {
        say(ctx, line);
    }
    Ok(())
}

/// **`+ship/board`** (D-18-34): in order — already aboard, at a gallery, a
/// live charter, the hull not under way and cradled at THIS gallery — then the
/// interior placement in the airlock's berths.
fn board(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let aboard = ctx
        .w
        .module_state::<SpatialState>()
        .is_some_and(|st| matches!(st.parent_of(actor), SpatialParent::Interior { .. }));
    if aboard {
        return Err(CmdError::msg(ALREADY_ABOARD));
    }
    let here = ctx.w.get(actor).and_then(|o| o.location);
    let at_gallery = here.is_some_and(|room| {
        ctx.w.module_state::<TacsimStore>().is_some_and(|store| {
            store
                .port_maps()
                .any(|map| map.presentation == Some(room.0))
        })
    });
    let Some(gallery) = here.filter(|_| at_gallery) else {
        return Err(CmdError::msg(BOARD_WHERE));
    };
    // The lease gate (D-18-34): no live charter is the plain refusal, an ended
    // one included (lens 5: "You hold no charter."). Cut IC13: a graduate boards
    // on the title (`helm_of`).
    let Some(charter) = helm_of(ctx.w, actor) else {
        return Err(CmdError::msg(NO_CHARTER));
    };
    let name = name_of(ctx.w, charter.hull);
    // Doc 18 cut IC9 (lead ruling L14; reviewer F10): while a clearance is filed
    // the hull's people are fixed — a person boarding now would be missing from
    // the burn's occupants and the ticket's manifest.
    // Doc 18 cut IC14 row 11 (lead ruling L17): papers that do not read are treated
    // as filed, in their own words.
    if crate::port::voyage::voyage_dark(ctx.w, charter.hull) {
        return Err(CmdError::msg(
            crate::port::voyage::VOYAGE_DARK.replace("{name}", &name),
        ));
    }
    if crate::port::voyage::pending_of(ctx.w, charter.hull).is_some() {
        return Err(CmdError::msg(
            crate::port::voyage::LEAVE_CLEARED.replace("{name}", &name),
        ));
    }
    let (airlock, slot) = {
        let Some(view) = hull_view(ctx.w, charter.hull) else {
            return Err(no_charter(ctx.w, actor));
        };
        match berth_of(ctx.w, charter.hull) {
            Berth::UnderWay => {
                return Err(CmdError::msg(UNDER_WAY_LOCK.replace("{name}", &name)));
            }
            Berth::Docked { map, .. } if map.presentation == Some(gallery.0) => {}
            Berth::Docked { .. } | Berth::Elsewhere => {
                return Err(CmdError::msg(NOT_HERE.replace("{name}", &name)));
            }
        }
        let slot = crate::port::hull::berths_slot(view.port.programme, view.interior);
        match (view.airlock(), slot) {
            (Some(airlock), Ok(slot)) => (airlock, slot),
            _ => {
                tracing::error!(
                    hull = charter.hull.0,
                    "tacsim: +ship/board: the hull has no airlock or berths"
                );
                return Err(CmdError::msg(LOCK_STUCK));
            }
        }
    };
    if let Err(error) = mush_sim::place_interior(ctx, actor, airlock, Some(slot)) {
        tracing::debug!(%error, "tacsim: +ship/board: the placement refused");
        return Err(CmdError::msg(LOCK_STUCK));
    }
    ctx.out
        .info(actor, format!("You cycle the lock into the {name}."));
    let text = RichText::new()
        .span(name_span(ctx.w, actor))
        .text(format!(" cycles through the lock into the {name}."));
    ctx.out
        .to_room_from(gallery, vec![actor], actor, narrative(text));
    let text = RichText::new()
        .span(name_span(ctx.w, actor))
        .text(" comes aboard through the outer door.");
    ctx.out
        .to_room_from(airlock, vec![actor], actor, narrative(text));
    ctx.show_room(actor, airlock);
    ctx.emit_world_event(&WorldEvent::Moved {
        who: actor,
        from: Some(gallery),
        to: airlock,
    });
    Ok(())
}

/// **`+ship/leave`** (D-18-34; L7: placement-gated, not lease-gated): aboard a
/// programme hull's airlock space, standing in the airlock, the hull docked —
/// then unplace (keep), move to the gallery, and stamp its port (L15).
fn leave(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let space = ctx
        .w
        .module_state::<SpatialState>()
        .and_then(|st| match st.parent_of(actor) {
            SpatialParent::Interior { space, .. } => Some(space),
            _ => None,
        });
    let Some(space) = space else {
        return Err(CmdError::msg(LEAVE_NOT_ABOARD));
    };
    let (hull, airlock, airlock_name) = {
        let Some((view, _)) = hull_of_room(ctx.w, space) else {
            return Err(CmdError::msg(LEAVE_NOT_ABOARD));
        };
        if view.airlock() != Some(space) {
            return Err(CmdError::msg(LEAVE_NOT_ABOARD));
        }
        (view.hull(), space, name_of(ctx.w, space))
    };
    let name = name_of(ctx.w, hull);
    // Doc 18 cut IC9 (lead ruling L14): a burned hull never launches with its
    // pilot ashore, so nobody leaves while a clearance is filed.
    // Doc 18 cut IC14 row 11 (lead ruling L17): papers that do not read are treated
    // as filed, in their own words.
    if crate::port::voyage::voyage_dark(ctx.w, hull) {
        return Err(CmdError::msg(
            crate::port::voyage::VOYAGE_DARK.replace("{name}", &name),
        ));
    }
    if crate::port::voyage::pending_of(ctx.w, hull).is_some() {
        return Err(CmdError::msg(
            crate::port::voyage::LEAVE_CLEARED.replace("{name}", &name),
        ));
    }
    if ctx.w.get(actor).and_then(|o| o.location) != Some(airlock) {
        return Err(CmdError::msg(
            LEAVE_WHERE.replace("{airlock}", &airlock_name),
        ));
    }
    let gallery = match berth_of(ctx.w, hull) {
        Berth::UnderWay => return Err(CmdError::msg(UNDER_WAY_LOCK.replace("{name}", &name))),
        Berth::Elsewhere => return Err(CmdError::msg(LEAVE_SEALED.replace("{name}", &name))),
        // Doc 20 cut SM.2 (decision 10): nobody steps out onto a pad.
        Berth::Docked { def, .. } if def.site.is_some() => {
            return Err(CmdError::msg(
                LEAVE_PAD
                    .replace("{name}", &name)
                    .replace("{port}", &port_name(def)),
            ));
        }
        Berth::Docked { map, .. } => map.presentation.map(ObjId),
    };
    let Some(gallery) = gallery else {
        tracing::error!(
            hull = hull.0,
            "tacsim: +ship/leave: the port map records no gallery"
        );
        return Err(CmdError::msg(LOCK_STUCK));
    };
    let ashore = put_ashore(
        ctx,
        actor,
        hull,
        gallery,
        "left the ship by the lock",
        |ctx: &mut Ctx<'_>| {
            let gallery_name = name_of(ctx.w, gallery);
            ctx.out.info(
                actor,
                format!("You cycle the lock out into the {gallery_name}."),
            );
            let text = RichText::new()
                .span(name_span(ctx.w, actor))
                .text(" cycles out through the outer door.");
            ctx.out
                .to_room_from(airlock, vec![actor], actor, narrative(text));
            gallery_line(ctx, actor, gallery, &name);
        },
    );
    if let Err(error) = ashore {
        tracing::error!(%error, "tacsim: +ship/leave: the lock would not cycle");
        return Err(CmdError::msg(LOCK_STUCK));
    }
    Ok(())
}

/// `"{who} comes ashore from the {name}."` to the gallery (`+ship/leave`'s line, and
/// the recall's, cut IC13).
pub(crate) fn gallery_line(ctx: &mut Ctx<'_>, who: ObjId, gallery: ObjId, name: &str) {
    let text = RichText::new()
        .span(name_span(ctx.w, who))
        .text(format!(" comes ashore from the {name}."));
    ctx.out
        .to_room_from(gallery, vec![who], who, narrative(text));
}

/// **Put one person ashore from a docked hull** (doc 18 cut IC13, factored out of
/// `+ship/leave`): the move to the gallery **first**, then `mush_sim::unplace`
/// (keeping the room) with `reason` — doc 18 cut IC14 row 19 (lead ruling L12): a
/// refused move leaves the person aboard and still placed, and a refused unplace moves
/// the person back where they stood (logged) — then `narrate` (the caller's lines,
/// said before the room is shown), `show_room`, `Moved { from: the person's room, to:
/// gallery }` and IC1's last-port stamp (L15: no `Moved` out of a port room happened,
/// so the stamp is this step's own), only once both succeeded. `Err` (nothing
/// narrated, nothing changed) when the move or the unplace refused.
///
/// # Errors
///
/// The refusal, as a sentence for the log.
pub(crate) fn put_ashore(
    ctx: &mut Ctx<'_>,
    person: ObjId,
    hull: ObjId,
    gallery: ObjId,
    reason: &str,
    narrate: impl FnOnce(&mut Ctx<'_>),
) -> Result<(), String> {
    let from = ctx.w.get(person).and_then(|o| o.location);
    ctx.w
        .move_to(person, gallery)
        .map_err(|error| format!("the move of #{} to the gallery refused: {error}", person.0))?;
    if let Err(error) = mush_sim::unplace(ctx, person, reason) {
        match from.map(|back| ctx.w.move_to(person, back)) {
            Some(Ok(())) => tracing::warn!(
                who = person.0,
                hull = hull.0,
                %error,
                "tacsim: a put-ashore's unplace refused; the person was moved back aboard"
            ),
            Some(Err(undo)) => tracing::error!(
                who = person.0,
                hull = hull.0,
                %undo,
                "tacsim: a refused put-ashore could not move the person back aboard"
            ),
            None => tracing::error!(
                who = person.0,
                hull = hull.0,
                %error,
                "tacsim: a put-ashore's unplace refused and the person had no room to go back to"
            ),
        }
        return Err(format!(
            "the unplace of #{} from #{} refused: {error}",
            person.0, hull.0
        ));
    }
    narrate(ctx);
    ctx.show_room(person, gallery);
    ctx.emit_world_event(&WorldEvent::Moved {
        who: person,
        from,
        to: gallery,
    });
    if let Some(port) = crate::ic_return::port_of_room(ctx.w, gallery)
        && let Err(error) = crate::ic_return::stamp(ctx.w, person, port)
    {
        tracing::warn!(who = person.0, port = port.id, %error, "tacsim: the last-port stamp did not write");
    }
    Ok(())
}

// ---- the look hook ------------------------------------------------------------

/// **The burn-and-reserve gauge** (doc 18 cut IC14 row 54; IC11 L21; Q-18-47), after
/// the tank line in the rooms whose instruments show the tank (the rooms
/// `interior.gauges` lists for `propellant`, other than the passage's manifest
/// slate): `{burn}` the filed passage's burn in tonnes, `{reserve}` what the tank
/// holds less that burn — or, once the burn has landed (its ticket exists, the tank
/// already debited), what the tank holds. Nothing is written.
pub const BURN_GAUGE: &str = "Burn for the filed plan: {burn} t; reserve after it: {reserve} t.";

/// [`BURN_GAUGE`] when no passage is filed.
pub const BURN_GAUGE_NONE: &str = "Burn for the filed plan: none filed.";

/// The compartment whose rooms carry the burn gauge.
const BURN_COMPARTMENT: &str = "propellant";

/// The room whose propellant reading is a manifest line, not a tank gauge.
const MANIFEST_ROOM: &str = "passage";

/// Whether the burn of passage `seq` has landed: its ticket reads (doc 18 cut IC14
/// row 54). `None` when the ticket cannot be read or names another key.
fn burn_landed(hook: &HookReadContext<'_>, hull: ObjId, seq: u32) -> Option<bool> {
    let handles = hook
        .world()
        .module_state::<crate::port::families::PortHandles>()?;
    let id = crate::port::families::TicketId { hull, seq };
    match hook.action_state(&handles.families.tickets, &id).ok()? {
        None => Some(false),
        Some(ticket) => (ticket.ticket_id() == id).then_some(true),
    }
}

/// Insert the burn gauge after the tank line of `lines` (doc 18 cut IC14 row 54):
/// one reading renders the tank as the room's only line; several render a heading
/// and one indented line per compartment, in `readings` order. `burn` is the filed
/// passage's burn and whether it has landed; `None` renders [`BURN_GAUGE_NONE`].
fn insert_burn_gauge(
    lines: &mut Vec<String>,
    goods: &GoodsTable,
    readings: &[Reading],
    burn: Option<(u64, bool)>,
) -> Result<(), String> {
    let Some((at, (_, comp, contents))) = readings
        .iter()
        .enumerate()
        .find(|(_, (id, _, _))| id.as_str() == BURN_COMPARTMENT)
    else {
        return Ok(());
    };
    let tank = gauges::single_qty(goods, comp, contents)
        .map_err(|e| e.to_string())?
        .unwrap_or(0);
    let line = match burn {
        Some((burn, landed)) => {
            let reserve = if landed {
                tank
            } else {
                tank.saturating_sub(burn)
            };
            BURN_GAUGE
                .replace("{burn}", &burn.to_string())
                .replace("{reserve}", &reserve.to_string())
        }
        None => BURN_GAUGE_NONE.to_owned(),
    };
    let (index, indent) = if readings.len() == 1 {
        (1, "")
    } else {
        (at.saturating_add(2), "  ")
    };
    let index = index.min(lines.len());
    lines.insert(index, format!("{indent}{line}"));
    Ok(())
}

/// **The hull rooms' status line and gauges** (18a §3; D-18-39): on a `look`
/// in a programme hull room, the status line, then — in a room some
/// `interior.gauges` list names — that room's gauge lines over exactly those
/// compartments, read from the hold's pools at look time (no vault → zeros). A
/// read or capacity error is one `{Instrument}: dark.` line. Nothing while the
/// layer is absent or disabled (a kept world that turns the layer off keeps its
/// hull rooms); nothing is written.
#[derive(Debug)]
pub struct ShipLook;

impl EventHook for ShipLook {
    fn look_rendered(
        &self,
        ctx: &HookReadContext<'_>,
        _looker: ObjId,
        room: ObjId,
    ) -> Vec<RichText> {
        let w = ctx.world();
        let Some(store) = w.module_state::<TacsimStore>() else {
            return Vec::new();
        };
        if !store.enabled() {
            return Vec::new();
        }
        let Some((view, tid)) = hull_of_room(w, room) else {
            return Vec::new();
        };
        let mut lines = vec![status_line(w, &view)];
        // The compartments the gauges map lists for this room, in its order.
        let shown: Vec<(CanonicalId, Compartment)> = view
            .interior
            .gauges
            .iter()
            .filter(|(_, rooms)| rooms.contains(&tid))
            .filter_map(|(comp, _)| {
                view.logistics
                    .stowage
                    .get(comp)
                    .map(|def| (comp.clone(), Compartment::from_stowage(comp, def)))
            })
            .collect();
        if !shown.is_empty() {
            let label = gauges::room_instrument(&tid);
            let read = |endpoint: EndpointRef, good: &CanonicalId| {
                crate::port_tick::read_pool_in(ctx, endpoint, good)
            };
            // Doc 18 cut IC14 row 54: the tank's rooms (not the manifest slate) carry
            // the burn gauge; an unreadable sailing plan shows no burn line at all.
            let burn_room =
                tid.as_str() != MANIFEST_ROOM
                    && view.interior.gauges.iter().any(|(comp, rooms)| {
                        comp.as_str() == BURN_COMPARTMENT && rooms.contains(&tid)
                    });
            let hull = view.hull();
            let burn = if burn_room && !crate::port::voyage::voyage_dark(w, hull) {
                match crate::port::voyage::pending_of(w, hull) {
                    None => Some(None),
                    // The burn has landed when its ticket exists; an unreadable
                    // ticket shows no burn line rather than a wrong reserve.
                    Some(pending) => burn_landed(ctx, hull, pending.seq)
                        .map(|landed| Some((pending.plan.burn_t, landed))),
                }
            } else {
                None
            };
            let rendered = store
                .goods()
                .ok_or_else(|| "the goods table is not loaded".to_owned())
                .and_then(|goods| {
                    let readings = readings_with(goods, shown, hold_vault(w, &view), &read)?;
                    let mut out =
                        gauges::room_lines(&tid, goods, &readings).map_err(|e| e.to_string())?;
                    if let Some(burn) = burn {
                        insert_burn_gauge(&mut out, goods, &readings, burn)?;
                    }
                    Ok(out)
                });
            match rendered {
                Ok(gauge_lines) => lines.extend(gauge_lines),
                Err(error) => {
                    tracing::warn!(room = room.0, %error, "tacsim: a hull gauge could not be read");
                    lines.push(format!("{label}: {DARK}."));
                }
            }
        }
        lines
            .into_iter()
            .map(|line| RichText::new().text(line))
            .collect()
    }
}

/// Register [`ShipLook`] on the `LookRendered` point.
///
/// # Errors
///
/// Never today; the `Result` is the registrar's shape.
pub fn register_hooks(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.hook(HookPoint::LookRendered, Arc::new(ShipLook));
    Ok(())
}
