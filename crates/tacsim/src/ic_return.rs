//! **The IC-return rule** (doc 18 §4.5, D-18-3; cut IC1): the Limbo door into the
//! 2350 corridor, and the one verb that puts a pilot back where they left it.
//!
//! The corridor has five IC ports (doc 19 cut A.8 added the three moon ports).
//! Ceres Freeport has a foot route — the unlocked Limbo door `exit.limbo_arrivals`
//! into Port Watch Arrivals — and the other four (Stickney Yards, Aitken Highport,
//! Voltaire Anchorage, Ligeia Terminal) have none: they are reached by ship.
//! Limbo's exits are global (`mush-core/src/commands/movement.rs:24-42`: a line
//! that names no local exit is tried against Limbo's from anywhere), so without a
//! rule a pilot who stepped out to the OOC area at Stickney (or any port reached
//! by ship) could walk straight back in at Ceres and skip the passage. Three
//! pieces close that:
//!
//! - **the stamp** ([`crate::ic_return::IcReturnStamp`]): a `Moved` observer that records the port a
//!   character last stood in whenever they move from a port room to a room outside
//!   every port — C03's `x`, Limbo's global `o`/`s`/`c` typed in a station room,
//!   `home`, a staff `@teleport`. The lead's "the Limbo-bound OOC exit from an IC
//!   room" is read that broadly on purpose: `home` from Stickney and then `c` is
//!   exactly the skip the rule exists to close. The stamp is one **unregistered**
//!   object marker per port (`tacsim.last_port.<port>`), so `@set` cannot forge it
//!   (doc 18 §3-A correction 2) and it needs no state slot, namespace or document.
//! - **the door** ([`crate::ic_return::IcDoorHook`]): a `HookPoint::MoveAttempt` veto on the door
//!   alone — doc 18 §4.5's "tacsim `MoveAttempt` hook", the kernel seam the atlas's
//!   `TravelHook` and the sim's `PlacementGate` use. It refuses a pilot stamped at a
//!   port with no foot route, and anyone typing Limbo's global `c` while standing in
//!   one. No staff exemption: the rule is about where the character left the
//!   corridor, and staff have `@teleport`.
//! - **`+ic`** ([`crate::ic_return::ic_command`]): back to the stamped port's arrival room through the
//!   kernel's non-exit move shape (`go_home`'s: the veto chain, the move, the
//!   narration, the room, then `Moved`). It writes no document.
//!
//! - **`+ooc`** ([`crate::ic_return::ooc_command`], the 2026-11-24 playtest): the one way out of
//!   character. From a port room it steps the character out to Limbo through the same move
//!   shape, and remembers the exact room they left as a second unregistered marker
//!   (`tacsim.ooc_room.<room entry>`, the room's atlas entry id, never its object number), so
//!   `+ic` can put them back in that room. It is refused in a simulator pod, in a running
//!   field, and wherever the `MoveAttempt` chain refuses an exit-less move (aboard a hull, a
//!   carrier under way, custody, a journey).
//! - **no IC exit into Limbo**: Limbo's global exits are not offered from a port room
//!   ([`crate::ic_return::IcDoorHook`] refuses them with [`crate::ic_return::LIMBO_WAY`]), so
//!   `o`, `s` and `c` typed in a station room no longer leave the corridor; `+ooc` does.
//!
//! The port table is local to this module ([`crate::ic_return::PORTS`]). Cut IC3 kept it beside the
//! lane's `port` pack entries rather than reading them (lead ruling L10), and pins
//! its agreement with them — venue, name, area and arrival room — by
//! `crates/tacsim/tests/ports.rs`'s `ic3_the_ic_return_table_agrees_with_the_port_entries`.
//! The onboarding gate
//! (`sheet/onboarding.rs`) is a `CommandGate` and is untouched: it already refuses
//! the door and `+ic` to an unapproved character, the one through its exit rule and
//! the other because the verb is off its allowlist.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{
    ArgShape, CanonicalId, CmdError, CommandSpec, Ctx, EventHook, HookPoint, HookReadContext,
    LIMBO, ObjId, ObjKind, World, WorldError, WorldEvent, WorldObserver, name_span,
};
use mush_proto::{NarrativeKind, OutputEvent, RichText};

use crate::commands::NOT_ENABLED;
use crate::store::TacsimStore;

/// One IC port the IC-return rule knows (doc 18 §4.5, D-18-3). Cut IC3 kept this
/// table (lead ruling L10) and pins its agreement with the lane's `port` entries
/// (venue, name, area, arrival) by test.
#[derive(Debug, PartialEq, Eq)]
pub struct IcPort {
    /// The port entry's venue id: `ceres`, `stickney`, `luna`, `deimos` or `titan`.
    /// The stamp's marker is
    /// [`LAST_PORT_MARKER`] followed by it.
    pub id: &'static str,
    /// The port's name in the player-facing lines.
    pub name: &'static str,
    /// The neighborhood pack whose rooms are the port: a room is "in the port" when
    /// its `RoomGeo` belongs to this pack.
    pub area: &'static str,
    /// Where `+ic` lands: an entry of [`IcPort::area`], resolved through the
    /// content latch.
    pub arrival: &'static str,
    /// Whether a door from Limbo leads here: true for Ceres (the Limbo door), false
    /// for the four ports reached by ship.
    pub foot_route: bool,
}

/// The five IC ports, Ceres first: `+ic` with no stamp goes to the first (where
/// the Limbo door goes), and a character carrying two stamps — which [`stamp`]
/// never writes — reads as the first of its own stamped ports with no foot route,
/// which fails closed (see [`last_port`]). Doc 19 cut A.8 appended the three moon
/// ports, reached by ship; doc 50 cut OP.1 the Isidis Concession, reached from Stickney.
pub const PORTS: [IcPort; 6] = [
    IcPort {
        id: "ceres",
        name: "Ceres Freeport",
        area: crate::definitions::content::CERES_PACK,
        arrival: "room.c03_arrivals",
        foot_route: true,
    },
    IcPort {
        id: "stickney",
        name: "Stickney Yards",
        area: crate::definitions::content::STICKNEY_PACK,
        arrival: "room.p01_berth_gallery",
        foot_route: false,
    },
    IcPort {
        id: "luna",
        name: "Aitken Highport",
        area: crate::definitions::content::AITKEN_PACK,
        arrival: "room.l03_arrivals",
        foot_route: false,
    },
    IcPort {
        id: "deimos",
        name: "Voltaire Anchorage",
        area: crate::definitions::content::VOLTAIRE_PACK,
        arrival: "room.d01_berth_gallery",
        foot_route: false,
    },
    IcPort {
        id: "titan",
        name: "Ligeia Terminal",
        area: crate::definitions::content::LIGEIA_PACK,
        arrival: "room.t03_arrivals",
        foot_route: false,
    },
    IcPort {
        id: "isidis",
        name: "Isidis Concession",
        area: crate::definitions::content::ISIDIS_PACK,
        arrival: "room.i01_landing_field",
        foot_route: false,
    },
];

/// The IC door's pack: the forward exit the content latch records for
/// [`DOOR_ENTRY`] (Limbo → Port Watch Arrivals, D-18-3) is the door.
pub const DOOR_PACK: &str = crate::definitions::content::CERES_PACK;
/// The IC door's entry.
pub const DOOR_ENTRY: &str = "exit.limbo_arrivals";
/// The stamp's marker prefix: unregistered object markers, one per port —
/// `tacsim.last_port.ceres`, `tacsim.last_port.stickney`, … — at most one present.
pub const LAST_PORT_MARKER: &str = "tacsim.last_port.";

/// The exact-room marker's prefix (`+ooc`): an unregistered object marker naming the
/// atlas entry of the room the character stepped out of, `tacsim.ooc_room.room.c05_concourse`
/// — at most one present. Like the port stamp, `@set` cannot forge it.
pub const OOC_ROOM_MARKER: &str = "tacsim.ooc_room.";

/// The verb.
pub const IC_VERB: &str = "+ic";
/// The way out.
pub const OOC_VERB: &str = "+ooc";

/// `+ic`'s help (T-45 checks every backticked verb in `docs/help` resolves).
pub const IC_HELP: &str = "+ic  --  Go back into character, to the room you left with +ooc. If \
     that room is gone or closed to you now, or you left some other way, +ic takes you to the \
     arrivals of the port you last stood in: Ceres Freeport's arrivals hall, Stickney Yards' \
     berth gallery, Aitken Highport's arrivals, Voltaire Anchorage's berth gallery, Ligeia \
     Terminal's arrivals or the Isidis Concession's landing field gallery. With no port \
     remembered it takes you to Ceres Freeport, where the Limbo door goes. A port you can only \
     reach by ship has no door from Limbo, so +ic is the way back to it.";

/// `+ooc`'s help.
pub const OOC_HELP: &str = "+ooc  --  Step out of character. From any room of a port, +ooc \
     takes you to Limbo, out of the story, and remembers the room you left; +ic puts you back \
     there. No exit of a port leads out of character: +ooc is the way. It is refused while you \
     are in a simulator pod, out in a field, aboard a ship or another vehicle, or held in \
     custody.";

/// `+ic` from inside a port.
pub const ALREADY_IN: &str = "You are already in the corridor.";
/// `+ooc` from outside every port.
pub const ALREADY_OUT: &str = "You are already out of character.";
/// `+ooc` from a simulator pod.
pub const IN_A_POD: &str = "You are in a simulator pod; EXIT leaves it.";
/// `+ooc` while a running field holds the character.
pub const IN_THE_FIELD: &str = "You are out in the field; +ooc waits until you are back in port.";
/// Limbo's global exits, typed in a port room.
pub const LIMBO_WAY: &str =
    "That way leads out of character; +ooc steps out, and +ic brings you back.";
/// `+ic` from something that is not a character.
pub const NOT_A_CHARACTER: &str = "Only a character can do that.";
/// `+ic` on a world whose station packs are not applied.
pub const NO_CORRIDOR: &str = "There is no corridor on this world to return to.";

/// The door's refusal for a pilot stamped at a port with no foot route — doc 18
/// §4.5's line, the port named from the table.
#[must_use]
pub fn left_at_line(port: &IcPort) -> String {
    format!(
        "You left the corridor at {}; +ic returns you there.",
        port.name
    )
}

/// `+ic`'s answer on arrival.
#[must_use]
pub fn return_line(port: &IcPort) -> String {
    format!("You return to the corridor at {}.", port.name)
}

/// `+ic`'s first line when the room `+ooc` remembered cannot take the character back.
#[must_use]
pub fn fell_back_line(port: &IcPort) -> String {
    format!(
        "The room you left is closed to you now; you return to the corridor at {}'s arrivals.",
        port.name
    )
}

/// `+ooc`'s answer.
#[must_use]
pub fn ooc_line(port: &IcPort) -> String {
    format!(
        "You step out of character, leaving the corridor at {}. +ic brings you back.",
        port.name
    )
}

/// The marker naming `port`.
fn marker_of(port: &IcPort) -> String {
    format!("{LAST_PORT_MARKER}{}", port.id)
}

/// The exact-room markers `who` carries (at most one is ever written).
fn room_markers(w: &World, who: ObjId) -> Vec<smol_str::SmolStr> {
    w.get(who)
        .map(|object| {
            object
                .markers
                .iter()
                .filter(|marker| marker.starts_with(OOC_ROOM_MARKER))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Clear every exact-room marker on `who`.
fn clear_room(w: &mut World, who: ObjId) -> Result<(), WorldError> {
    for marker in room_markers(w, who) {
        w.set_marker(who, &marker, false)?;
    }
    Ok(())
}

/// The atlas entry id of `room`, when the atlas made it (`room.c05_concourse`).
fn room_entry(w: &World, room: ObjId) -> Option<CanonicalId> {
    let atlas = w.module_state::<mush_atlas::AtlasState>()?;
    Some(atlas.records.roomgeo.get(&room)?.key.entry.clone())
}

/// Remember `room` as the room `who` stepped out of: one exact-room marker, the others
/// cleared. A room the atlas did not make is not remembered (and `+ic` falls back).
///
/// # Errors
///
/// [`WorldError`] when `who` is not an object.
pub fn remember_room(w: &mut World, who: ObjId, room: ObjId) -> Result<(), WorldError> {
    clear_room(w, who)?;
    if let Some(entry) = room_entry(w, room) {
        w.set_marker(who, &format!("{OOC_ROOM_MARKER}{}", entry.as_str()), true)?;
    }
    Ok(())
}

/// The room `+ooc` remembered for `who`, as an object, when it lies in `port`: `None`
/// when nothing is remembered, or the remembered entry no longer resolves to one of
/// `port`'s rooms.
#[must_use]
pub fn remembered_room(w: &World, who: ObjId, port: &IcPort) -> Option<ObjId> {
    room_markers(w, who).iter().find_map(|marker| {
        let entry = marker.strip_prefix(OOC_ROOM_MARKER)?;
        latched(w, port.area, entry).filter(|room| {
            w.get(*room).is_some_and(|object| {
                object.kind == ObjKind::Room && !object.flags.contains(mush_core::Flags::GOING)
            }) && port_of_room(w, *room).is_some_and(|here| here.id == port.id)
        })
    })
}

/// Whether `who` may stand in `room` again (`+ic`'s exact return): a public room, a room
/// `who` controls, or a restricted one some exit into it from the same neighborhood lets
/// `who` through by its lock. A staff-locked office the character has since lost the key
/// to is not.
#[must_use]
pub fn may_stand(w: &World, who: ObjId, room: ObjId) -> bool {
    let Some(atlas) = w.module_state::<mush_atlas::AtlasState>() else {
        return false;
    };
    let Some(geo) = atlas.records.roomgeo.get(&room) else {
        return false;
    };
    if geo.access == mush_atlas::Access::Public || mush_core::perm::controls(w, who, room) {
        return true;
    }
    atlas
        .records
        .rooms_in(&geo.neighborhood)
        .filter_map(|other| w.get(other.room))
        .flat_map(|other| other.exits.iter())
        .filter_map(|exit| w.get(*exit))
        .filter(|exit| exit.destination == Some(room))
        .any(|exit| {
            exit.locks
                .get(&mush_core::LockType::Basic)
                .is_none_or(|lock| {
                    mush_core::eval_lock(w, lock, who, mush_core::lock::MAX_LOCK_DEPTH)
                })
        })
}

/// The port whose neighborhood `room` belongs to, by its `RoomGeo` (the atlas's
/// record of the room): `None` for Limbo, the OOC area, a hull interior, or any
/// room outside the five ports' neighborhood packs.
#[must_use]
pub fn port_of_room(w: &World, room: ObjId) -> Option<&'static IcPort> {
    let atlas = w.module_state::<mush_atlas::AtlasState>()?;
    let geo = atlas.records.roomgeo.get(&room)?;
    PORTS
        .iter()
        .find(|port| geo.neighborhood.pack.as_str() == port.area)
}

/// The port `who` last stood in, by the stamp. Two or more markers — which
/// [`stamp`] never writes — read as the first **stamped** port in [`PORTS`] with no
/// foot route (else the first stamped): the door then refuses, which is the closed
/// answer, and the answer is always one of the character's own stamps, so `+ic`
/// never lands them at a port neither marker names.
#[must_use]
pub fn last_port(w: &World, who: ObjId) -> Option<&'static IcPort> {
    let mut stamped = PORTS
        .iter()
        .filter(|port| w.has_marker(who, &marker_of(port)));
    let first = stamped.next()?;
    if stamped.next().is_none() {
        return Some(first);
    }
    PORTS
        .iter()
        .filter(|port| w.has_marker(who, &marker_of(port)))
        .find(|port| !port.foot_route)
        .or(Some(first))
}

/// Stamp `who` at `port`: set its marker and clear every other port's, so a
/// character carries at most one.
///
/// # Errors
///
/// [`WorldError`] when `who` is not an object.
pub fn stamp(w: &mut World, who: ObjId, port: &IcPort) -> Result<(), WorldError> {
    for other in &PORTS {
        w.set_marker(who, &marker_of(other), other.id == port.id)?;
    }
    Ok(())
}

/// A pack entry resolved through the content latch.
fn latched(w: &World, pack: &str, entry: &str) -> Option<ObjId> {
    let key =
        mush_atlas::RecordKey::new(CanonicalId::new(pack).ok()?, CanonicalId::new(entry).ok()?);
    mush_atlas::travel::latched_object(w, &key)
}

/// The IC door: the forward exit (Limbo → Port Watch Arrivals) the content latch
/// records for [`DOOR_ENTRY`]. `None` on a world without the Ceres pack.
#[must_use]
pub fn door(w: &World) -> Option<ObjId> {
    latched(w, DOOR_PACK, DOOR_ENTRY)
}

/// Where `+ic` lands for `port`: its arrival entry through the latch, when that is
/// a room.
#[must_use]
pub fn arrival_room(w: &World, port: &IcPort) -> Option<ObjId> {
    latched(w, port.area, port.arrival).filter(|room| {
        w.get(*room)
            .is_some_and(|object| object.kind == ObjKind::Room)
    })
}

/// **The stamp** (doc 18 §4.5): on `Moved { who, from, to }` for a Player, when
/// `from` is a port room and `to` lies in no port, record the port `who` left.
/// Logs nothing per move; a failed marker write logs once at `warn` and changes
/// nothing (the observer is infallible from dispatch's point of view).
pub struct IcReturnStamp;

impl WorldObserver for IcReturnStamp {
    fn on_event(&self, ctx: &mut Ctx<'_>, ev: &WorldEvent) {
        let WorldEvent::Moved {
            who,
            from: Some(from),
            to,
        } = ev
        else {
            return;
        };
        if ctx.w.get(*who).map(|object| object.kind) != Some(ObjKind::Player) {
            return;
        }
        let Some(port) = port_of_room(ctx.w, *from) else {
            return;
        };
        if port_of_room(ctx.w, *to).is_some() {
            return;
        }
        // Any way out forgets the exact room; `+ooc` writes it again after this move.
        if let Err(error) = stamp(ctx.w, *who, port).and_then(|()| clear_room(ctx.w, *who)) {
            tracing::warn!(who = who.0, port = port.id, %error, "ic_return: the last-port stamp did not write");
        }
    }
}

/// **The door** (doc 18 §4.5, D-18-3): a `MoveAttempt` veto on Limbo's exits. Typed in
/// a port room, every Limbo exit — the global `o`, `s` and `c` — is refused with
/// [`LIMBO_WAY`]: no IC exit leads out of character, `+ooc` does (the 2026-11-24
/// playtest). From anywhere else only the IC door is looked at: a pilot stamped at a
/// port with no foot route is refused with doc 18 §4.5's line. No stamp, or a Ceres
/// stamp, passes.
pub struct IcDoorHook;

impl EventHook for IcDoorHook {
    fn move_attempt(
        &self,
        ctx: &HookReadContext<'_>,
        actor: ObjId,
        exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), RichText> {
        let Some(exit) = exit else {
            return Ok(());
        };
        let w = ctx.world();
        let here = w.get(actor).and_then(|object| object.location);
        if here != Some(LIMBO)
            && here.and_then(|room| port_of_room(w, room)).is_some()
            && (dest == LIMBO
                || w.get(LIMBO)
                    .is_some_and(|limbo| limbo.exits.contains(&exit)))
        {
            return Err(RichText::tag("error", LIMBO_WAY));
        }
        if door(w) != Some(exit) {
            return Ok(());
        }
        if let Some(left) = last_port(w, actor)
            && !left.foot_route
        {
            return Err(RichText::tag("error", left_at_line(left)));
        }
        Ok(())
    }
}

/// **`+ic`** (doc 18 §4.5; the 2026-11-24 playtest): back into the corridor at the room
/// `+ooc` remembered, when it still stands in the stamped port and the character
/// [`may_stand`] there; otherwise at the stamped port's arrival room (Ceres when there is
/// no stamp). In order: the layer, a character, not already in a port, the room, then
/// `go_home`'s shape through `Ctx` — the `MoveAttempt` chain (which is what makes the
/// sim's `PlacementGate` refuse a hull occupant from cut IC4 on), the move, the lines,
/// the room, and `Moved`. A return clears the remembered room. It writes no document.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor; nothing is written.
pub fn ic_command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    enabled(ctx)?;
    let actor = ctx.actor;
    let from = ctx.w.get(actor).and_then(|object| object.location);
    if from.and_then(|here| port_of_room(ctx.w, here)).is_some() {
        return Err(CmdError::msg(ALREADY_IN));
    }
    let Some(port) = last_port(ctx.w, actor).or_else(|| PORTS.first()) else {
        return Err(CmdError::msg(NO_CORRIDOR));
    };
    let Some(arrival) = arrival_room(ctx.w, port) else {
        return Err(CmdError::msg(NO_CORRIDOR));
    };
    let remembered = !room_markers(ctx.w, actor).is_empty();
    let exact = remembered_room(ctx.w, actor, port).filter(|room| may_stand(ctx.w, actor, *room));
    let dest = exact.unwrap_or(arrival);
    if let Err(reason) = ctx.check_move_attempt(actor, None, dest) {
        ctx.out.to_actor(actor, OutputEvent::Sys(reason));
        return Ok(());
    }
    if ctx.w.move_to(actor, dest).is_err() {
        return Err(CmdError::msg(NO_CORRIDOR));
    }
    if let Err(error) = clear_room(ctx.w, actor) {
        tracing::warn!(who = actor.0, %error, "ic_return: the remembered room did not clear");
    }
    if remembered && exact.is_none() {
        ctx.out.info(actor, fell_back_line(port));
    } else {
        ctx.out.info(actor, return_line(port));
    }
    narrate(ctx, actor, from, dest);
    Ok(())
}

/// **`+ooc`** (the 2026-11-24 playtest): out of character, to Limbo, from a port room,
/// remembering the exact room for `+ic`. In order: the layer, a character, not in a
/// simulator pod, not held by a running field, then the `MoveAttempt` chain with no
/// exit (aboard a hull, a carrier under way, custody and a journey all refuse there,
/// with their own lines), then "already out" for anyone standing outside every port.
/// The move fires `Moved`, whose stamp records the port; the room is remembered after.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor (a gate's reason is said as
/// `Sys`, as `+ic` says it); nothing is written.
pub fn ooc_command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    enabled(ctx)?;
    let actor = ctx.actor;
    if crate::match_::cockpit::in_a_pod(ctx.w, actor) {
        return Err(CmdError::msg(IN_A_POD));
    }
    if crate::field::crew::in_a_running_field(ctx.w, actor) {
        return Err(CmdError::msg(IN_THE_FIELD));
    }
    let from = ctx.w.get(actor).and_then(|object| object.location);
    if let Err(reason) = ctx.check_move_attempt(actor, None, LIMBO) {
        ctx.out.to_actor(actor, OutputEvent::Sys(reason));
        return Ok(());
    }
    let Some((here, port)) = from
        .filter(|here| *here != LIMBO)
        .and_then(|here| port_of_room(ctx.w, here).map(|port| (here, port)))
    else {
        return Err(CmdError::msg(ALREADY_OUT));
    };
    if ctx.w.move_to(actor, LIMBO).is_err() {
        return Err(CmdError::msg(NO_CORRIDOR));
    }
    ctx.out.info(actor, ooc_line(port));
    narrate(ctx, actor, Some(here), LIMBO);
    if let Err(error) = remember_room(ctx.w, actor, here) {
        tracing::warn!(who = actor.0, %error, "ic_return: the room left did not write");
    }
    Ok(())
}

/// The layer and the character, both verbs' first two refusals.
fn enabled(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if !ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled)
    {
        return Err(CmdError::msg(NOT_ENABLED));
    }
    if ctx.w.get(ctx.actor).map(|object| object.kind) != Some(ObjKind::Player) {
        return Err(CmdError::msg(NOT_A_CHARACTER));
    }
    Ok(())
}

/// After the move: the leave and arrive lines, the room, and `Moved`.
fn narrate(ctx: &mut Ctx<'_>, actor: ObjId, from: Option<ObjId>, dest: ObjId) {
    if let Some(left) = from {
        let text = RichText::new()
            .span(name_span(ctx.w, actor))
            .text(" leaves.");
        ctx.out
            .to_room_from(left, vec![actor], actor, narrative(text));
    }
    let text = RichText::new()
        .span(name_span(ctx.w, actor))
        .text(" has arrived.");
    ctx.out
        .to_room_from(dest, vec![actor], actor, narrative(text));
    ctx.show_room(actor, dest);
    ctx.emit_world_event(&WorldEvent::Moved {
        who: actor,
        from,
        to: dest,
    });
}

/// An unattributed narrative line, as the kernel's movement lines are.
fn narrative(text: RichText) -> OutputEvent {
    OutputEvent::Narrative {
        kind: NarrativeKind::Emit,
        actor: None,
        text,
    }
}

/// Register `+ic`, `+ooc`, the door hook and the stamp on the **tacsim** module, under the
/// module's `LegacyPostImage` classification (the verb writes no document).
///
/// # Errors
///
/// [`ModError`] if the verb name is already taken.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: IC_VERB.into(),
            shape: ArgShape::None,
            help: IC_HELP.into(),
            ..CommandSpec::default()
        },
        Arc::new(ic_command),
    )?;
    reg.command(
        CommandSpec {
            name: OOC_VERB.into(),
            shape: ArgShape::None,
            help: OOC_HELP.into(),
            ..CommandSpec::default()
        },
        Arc::new(ooc_command),
    )?;
    reg.hook(HookPoint::MoveAttempt, Arc::new(IcDoorHook));
    // Doc 36 cut GC.8a (Q-36-32): a hull berthed or under a repair order stays put.
    reg.hook(
        HookPoint::MoveAttempt,
        Arc::new(crate::port::bay_work::BayHold),
    );
    reg.hook(
        HookPoint::CarryAttempt,
        Arc::new(crate::port::bay_work::BayHold),
    );
    reg.subscribe_moved("ic_return_stamp", 1, Arc::new(IcReturnStamp))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::GOD;

    fn pilot(w: &mut World) -> ObjId {
        w.create(ObjKind::Player, "Pilot", GOD, Some(LIMBO))
            .expect("a player")
    }

    #[test]
    fn ic1_the_stamp_writes_one_marker_and_clears_the_other() {
        let (mut w, _reg) = mush_testkit::boot_world();
        let who = pilot(&mut w);
        assert_eq!(last_port(&w, who), None);
        stamp(&mut w, who, &PORTS[1]).expect("stamped");
        assert!(w.has_marker(who, "tacsim.last_port.stickney"));
        assert!(!w.has_marker(who, "tacsim.last_port.ceres"));
        assert_eq!(last_port(&w, who).map(|p| p.id), Some("stickney"));
        stamp(&mut w, who, &PORTS[0]).expect("stamped");
        assert!(w.has_marker(who, "tacsim.last_port.ceres"));
        assert!(!w.has_marker(who, "tacsim.last_port.stickney"));
        assert_eq!(last_port(&w, who).map(|p| p.id), Some("ceres"));
    }

    #[test]
    fn ic1_last_port_fails_closed_on_two_markers() {
        let (mut w, _reg) = mush_testkit::boot_world();
        let who = pilot(&mut w);
        w.set_marker(who, "tacsim.last_port.ceres", true)
            .expect("marker");
        w.set_marker(who, "tacsim.last_port.stickney", true)
            .expect("marker");
        let read = last_port(&w, who).expect("a port");
        assert_eq!(read.id, "stickney");
        assert!(!read.foot_route);
    }

    /// Doc 19 cut A.8 (review): with five ports the two-marker fallback is one of
    /// the character's own stamps, never a port neither marker names.
    #[test]
    fn a8_two_markers_read_as_one_of_the_characters_own_stamps() {
        let (mut w, _reg) = mush_testkit::boot_world();
        // Ceres and Titan: Titan (the stamped port with no foot route), not Stickney.
        let who = pilot(&mut w);
        w.set_marker(who, "tacsim.last_port.ceres", true)
            .expect("marker");
        w.set_marker(who, "tacsim.last_port.titan", true)
            .expect("marker");
        let read = last_port(&w, who).expect("a port");
        assert_eq!(read.id, "titan");
        assert!(!read.foot_route);
        // Luna and Deimos, both by ship: the first stamped in table order.
        let (mut w, _reg) = mush_testkit::boot_world();
        let who = pilot(&mut w);
        w.set_marker(who, "tacsim.last_port.deimos", true)
            .expect("marker");
        w.set_marker(who, "tacsim.last_port.luna", true)
            .expect("marker");
        assert_eq!(last_port(&w, who).map(|p| p.id), Some("luna"));
    }

    #[test]
    fn ic1_the_help_names_ic() {
        assert!(IC_HELP.starts_with("+ic  --  "));
        assert!(IC_HELP.contains("to the room you left with +ooc"));
        assert!(OOC_HELP.starts_with("+ooc  --  "));
        assert!(OOC_HELP.contains("+ic puts you back there"));
        assert_eq!(
            left_at_line(&PORTS[1]),
            "You left the corridor at Stickney Yards; +ic returns you there."
        );
        assert_eq!(
            return_line(&PORTS[1]),
            "You return to the corridor at Stickney Yards."
        );
        assert_eq!(
            ooc_line(&PORTS[0]),
            "You step out of character, leaving the corridor at Ceres Freeport. +ic brings you back."
        );
        assert_eq!(
            fell_back_line(&PORTS[0]),
            "The room you left is closed to you now; you return to the corridor at Ceres Freeport's arrivals."
        );
    }

    /// The exact-room marker is one entry id, cleared on the next write, and resolves only
    /// through the latch (a bare world has no atlas, so nothing resolves).
    #[test]
    fn ooc_the_room_marker_is_one_and_resolves_only_through_the_atlas() {
        let (mut w, _reg) = mush_testkit::boot_world();
        let who = pilot(&mut w);
        w.set_marker(who, "tacsim.ooc_room.room.a", true)
            .expect("marker");
        w.set_marker(who, "tacsim.ooc_room.room.b", true)
            .expect("marker");
        assert_eq!(room_markers(&w, who).len(), 2);
        assert_eq!(remembered_room(&w, who, &PORTS[0]), None);
        // A room the atlas did not make clears and writes nothing.
        remember_room(&mut w, who, LIMBO).expect("written");
        assert!(room_markers(&w, who).is_empty());
        assert!(!may_stand(&w, who, LIMBO));
    }
}
