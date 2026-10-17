//! **The chartered hull** (doc 18 cut IC6; doc 18 §4.2 "The airlock and the
//! berth", "Compartments"; D-18-23, D-18-34, D-18-39; 18a §3; lead rulings L7,
//! L8, L11, L15, L18, L19, L21 of the IC6 brief).
//!
//! - `+ship` and `+ship/hold` read the pilot's chartered hull — where it is,
//!   what its hold vault's pools hold against both limits, the tank and the
//!   locker — anywhere. The hull is the **live lease's** (D-18-23,
//!   [`crate::port::coop::live_charter`]), never the enrolment document's: an
//!   unreadable document darkens only the programme line (L21).
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
//!   `+ship/board` and `+ship/leave` while a clearance is filed; the burn and
//!   reserve gauge lines are cut IC11's (IC9 lead ruling L17).

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
use crate::definitions::{GoodsTable, InteriorDef, LogisticsDef};
use crate::port::capacity::Compartment;
use crate::port::coop::{
    EnrolmentStatus, NO_CHARTER, ProgrammePort, live_charter, name_of, port_name, programme_port,
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
     committed when the clearance is granted.";

/// The programme's one hull class (cut IC8 may read it from the spec).
pub const HULL_CLASS: &str = "Dray";

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
/// `+ship`'s first line anywhere else.
pub const SHIP_ELSEWHERE: &str = "{name} ({class}): not at a berth.";
/// `+ship`'s programme line when the enrolment document does not read (L21).
pub const SHIP_BOOKS_DARK: &str = "Programme: the Co-op's books cannot be read.";
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

/// **One programme hull, resolved.**
#[derive(Clone, Debug)]
pub struct HullView<'a> {
    /// The programme's port.
    pub port: ProgrammePort<'a>,
    /// The hull's map row.
    pub row: &'a HullRow,
    /// The programme unit's interior template.
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
}

/// The view for a programme port, a row.
fn view_of<'a>(w: &'a World, port: ProgrammePort<'a>, row: &'a HullRow) -> Option<HullView<'a>> {
    let store = w.module_state::<TacsimStore>()?;
    let (interior, logistics) = crate::port::hull::hull_spec(store, port.programme)?;
    Some(HullView {
        port,
        row,
        interior,
        logistics,
    })
}

/// **The view of one programme hull** by its Thing.
#[must_use]
pub fn hull_view(w: &World, hull: ObjId) -> Option<HullView<'_>> {
    let port = programme_port(w)?;
    let row = port.map.hulls.iter().find(|row| row.hull == hull.0)?;
    view_of(w, port, row)
}

/// **The programme hull `room` belongs to**, and the room's template id (a
/// scan of the programme port's hull rows).
#[must_use]
pub fn hull_of_room(w: &World, room: ObjId) -> Option<(HullView<'_>, CanonicalId)> {
    let port = programme_port(w)?;
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

/// The hull's stowage compartments, in stowage order.
fn compartments(view: &HullView<'_>) -> Vec<(CanonicalId, Compartment)> {
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
        (None, Berth::Docked { def, .. }) => STATUS_DOCKED.replace("{port}", &port_name(def)),
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
            .replace("{class}", HULL_CLASS);
    }
    let template = match berth_of(w, view.hull()) {
        Berth::Docked { map, def } => {
            let gallery = map
                .presentation
                .map_or_else(String::new, |g| name_of(w, ObjId(g)));
            SHIP_DOCKED
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
        .replace("{class}", HULL_CLASS)
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
    if ctx.switches.has("board") {
        board(ctx)
    } else if ctx.switches.has("plan") {
        crate::port::voyage::plan_cmd(ctx)
    } else if ctx.switches.has("undock") {
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
    let Some(charter) = live_charter(ctx.w, actor) else {
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
        let enrolment = ctx
            .w
            .module_state::<TacsimStore>()
            .and_then(|store| store.enrolment_for_lease(charter.contract.0))
            .cloned();
        let programme = match enrolment {
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
                    lease = charter.contract.0,
                    "tacsim: +ship found a live charter with no readable enrolment"
                );
                SHIP_BOOKS_DARK.to_owned()
            }
        };
        [first, hold, programme]
    };
    for line in lines {
        say(ctx, line);
    }
    Ok(())
}

/// `+ship/hold`: the hold, good by good.
fn hold(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let Some(charter) = live_charter(ctx.w, actor) else {
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
    // one included (lens 5: "You hold no charter.").
    let Some(charter) = live_charter(ctx.w, actor) else {
        return Err(CmdError::msg(NO_CHARTER));
    };
    let name = name_of(ctx.w, charter.hull);
    // Doc 18 cut IC9 (lead ruling L14; reviewer F10): while a clearance is filed
    // the hull's people are fixed — a person boarding now would be missing from
    // the burn's occupants and the ticket's manifest.
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
        Berth::Docked { map, .. } => map.presentation.map(ObjId),
    };
    let Some(gallery) = gallery else {
        tracing::error!(
            hull = hull.0,
            "tacsim: +ship/leave: the port map records no gallery"
        );
        return Err(CmdError::msg(LOCK_STUCK));
    };
    if let Err(error) = mush_sim::unplace(ctx, actor, "left the ship by the lock") {
        tracing::error!(%error, "tacsim: +ship/leave: the unplace refused");
        return Err(CmdError::msg(LOCK_STUCK));
    }
    if let Err(error) = ctx.w.move_to(actor, gallery) {
        tracing::error!(%error, "tacsim: +ship/leave: the move to the gallery refused");
        return Err(CmdError::msg(LOCK_STUCK));
    }
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
    let text = RichText::new()
        .span(name_span(ctx.w, actor))
        .text(format!(" comes ashore from the {name}."));
    ctx.out
        .to_room_from(gallery, vec![actor], actor, narrative(text));
    ctx.show_room(actor, gallery);
    ctx.emit_world_event(&WorldEvent::Moved {
        who: actor,
        from: Some(airlock),
        to: gallery,
    });
    // L15 (IC1's carry-forward): no `Moved` out of a port room happened here
    // (the `from` is the hull's airlock), so the stamp is this verb's own.
    if let Some(port) = crate::ic_return::port_of_room(ctx.w, gallery)
        && let Err(error) = crate::ic_return::stamp(ctx.w, actor, port)
    {
        tracing::warn!(who = actor.0, port = port.id, %error, "tacsim: +ship/leave: the last-port stamp did not write");
    }
    Ok(())
}

// ---- the look hook ------------------------------------------------------------

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
            let rendered = store
                .goods()
                .ok_or_else(|| "the goods table is not loaded".to_owned())
                .and_then(|goods| {
                    let readings = readings_with(goods, shown, hold_vault(w, &view), &read)?;
                    gauges::room_lines(&tid, goods, &readings).map_err(|e| e.to_string())
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
