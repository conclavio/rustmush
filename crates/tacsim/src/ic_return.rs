//! **The IC-return rule** (doc 18 §4.5, D-18-3; cut IC1): the Limbo door into the
//! 2350 corridor, and the one verb that puts a pilot back where they left it.
//!
//! The corridor has two IC ports in the slice. Ceres Freeport has a foot route —
//! the unlocked Limbo door `exit.limbo_arrivals` into Port Watch Arrivals — and
//! Stickney Yards has none: it is reached by ship. Limbo's exits are global
//! (`mush-core/src/commands/movement.rs:24-42`: a line that names no local exit
//! is tried against Limbo's from anywhere), so without a rule a pilot who stepped
//! out to the OOC area at Stickney could walk straight back in at Ceres and skip a
//! two-to-three-week passage. Three pieces close that:
//!
//! - **the stamp** ([`IcReturnStamp`]): a `Moved` observer that records the port a
//!   character last stood in whenever they move from a port room to a room outside
//!   every port — C03's `x`, Limbo's global `o`/`s`/`c` typed in a station room,
//!   `home`, a staff `@teleport`. The lead's "the Limbo-bound OOC exit from an IC
//!   room" is read that broadly on purpose: `home` from Stickney and then `c` is
//!   exactly the skip the rule exists to close. The stamp is one **unregistered**
//!   object marker per port (`tacsim.last_port.<port>`), so `@set` cannot forge it
//!   (doc 18 §3-A correction 2) and it needs no state slot, namespace or document.
//! - **the door** ([`IcDoorHook`]): a `HookPoint::MoveAttempt` veto on the door
//!   alone — doc 18 §4.5's "tacsim `MoveAttempt` hook", the kernel seam the atlas's
//!   `TravelHook` and the sim's `PlacementGate` use. It refuses a pilot stamped at a
//!   port with no foot route, and anyone typing Limbo's global `c` while standing in
//!   one. No staff exemption: the rule is about where the character left the
//!   corridor, and staff have `@teleport`.
//! - **`+ic`** ([`ic_command`]): back to the stamped port's arrival room through the
//!   kernel's non-exit move shape (`go_home`'s: the veto chain, the move, the
//!   narration, the room, then `Moved`). It writes no document.
//!
//! The port table is local to this cut ([`PORTS`]); cut IC3's `port` pack entries
//! (their `area`, `rooms.arrivals` and `gallery`) replace it. The onboarding gate
//! (`sheet/onboarding.rs`) is a `CommandGate` and is untouched: it already refuses
//! the door and `+ic` to an unapproved character, the one through its exit rule and
//! the other because the verb is off its allowlist.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{
    ArgShape, CanonicalId, CmdError, CommandSpec, Ctx, EventHook, HookPoint, HookReadContext,
    ObjId, ObjKind, World, WorldError, WorldEvent, WorldEventKind, WorldObserver, name_span,
};
use mush_proto::{NarrativeKind, OutputEvent, RichText};

use crate::commands::NOT_ENABLED;
use crate::store::TacsimStore;

/// One IC port the IC-return rule knows (doc 18 §4.5, D-18-3). Cut IC3's `port`
/// entries replace this table (their `area`, `rooms.arrivals` and `gallery`).
#[derive(Debug, PartialEq, Eq)]
pub struct IcPort {
    /// The port entry's venue id: `ceres` or `stickney`. The stamp's marker is
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
    /// for Stickney, which is reached by ship.
    pub foot_route: bool,
}

/// The two IC ports of the slice, Ceres first: `+ic` with no stamp goes to the
/// first (where the Limbo door goes), and a character carrying two stamps — which
/// [`stamp`] never writes — reads as stamped at the first port with no foot route,
/// which fails closed.
pub const PORTS: [IcPort; 2] = [
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
];

/// The IC door's pack: the forward exit the content latch records for
/// [`DOOR_ENTRY`] (Limbo → Port Watch Arrivals, D-18-3) is the door.
pub const DOOR_PACK: &str = crate::definitions::content::CERES_PACK;
/// The IC door's entry.
pub const DOOR_ENTRY: &str = "exit.limbo_arrivals";
/// The stamp's marker prefix: unregistered object markers, one per port —
/// `tacsim.last_port.ceres`, `tacsim.last_port.stickney` — at most one present.
pub const LAST_PORT_MARKER: &str = "tacsim.last_port.";

/// The verb.
pub const IC_VERB: &str = "+ic";

/// `+ic`'s help (T-45 checks every backticked verb in `docs/help` resolves).
pub const IC_HELP: &str = "+ic  --  Go back into the corridor where you left it. The game \
     remembers the port you last stood in when you stepped out to Limbo or the OOC area, and \
     +ic puts you back there: Ceres Freeport's arrivals hall, or Stickney Yards' berth \
     gallery. With no port remembered it takes you to Ceres Freeport, where the Limbo door \
     goes. A port you can only reach by ship has no door from Limbo, so +ic is the way back \
     to it.";

/// `+ic` from inside a port.
pub const ALREADY_IN: &str = "You are already in the corridor.";
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

/// The door's refusal for someone typing Limbo's global door word while standing
/// in a port with no foot route.
#[must_use]
pub fn no_door_line(standing: &IcPort, door_port: &IcPort) -> String {
    format!(
        "There is no door from {} to {}; the way there is by ship.",
        standing.name, door_port.name
    )
}

/// `+ic`'s answer on arrival.
#[must_use]
pub fn return_line(port: &IcPort) -> String {
    format!("You return to the corridor at {}.", port.name)
}

/// The marker naming `port`.
fn marker_of(port: &IcPort) -> String {
    format!("{LAST_PORT_MARKER}{}", port.id)
}

/// The port whose neighborhood `room` belongs to, by its `RoomGeo` (the atlas's
/// record of the room): `None` for Limbo, the OOC area, a hull interior, or any
/// room outside the two station packs.
#[must_use]
pub fn port_of_room(w: &World, room: ObjId) -> Option<&'static IcPort> {
    let atlas = w.module_state::<mush_atlas::AtlasState>()?;
    let geo = atlas.records.roomgeo.get(&room)?;
    PORTS
        .iter()
        .find(|port| geo.neighborhood.pack.as_str() == port.area)
}

/// The port `who` last stood in, by the stamp. Two markers — which [`stamp`] never
/// writes — read as the first port in [`PORTS`] with no foot route: the door then
/// refuses, which is the closed answer.
#[must_use]
pub fn last_port(w: &World, who: ObjId) -> Option<&'static IcPort> {
    let mut stamped = PORTS
        .iter()
        .filter(|port| w.has_marker(who, &marker_of(port)));
    let first = stamped.next()?;
    if stamped.next().is_none() {
        return Some(first);
    }
    PORTS.iter().find(|port| !port.foot_route).or(Some(first))
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

/// The port the door leads to (the table's port whose area is the door's pack).
fn door_port() -> Option<&'static IcPort> {
    PORTS.iter().find(|port| port.area == DOOR_PACK)
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
        if let Err(error) = stamp(ctx.w, *who, port) {
            tracing::warn!(who = who.0, port = port.id, %error, "ic_return: the last-port stamp did not write");
        }
    }
}

/// **The door** (doc 18 §4.5, D-18-3): a `MoveAttempt` veto that looks only at the
/// IC door. Standing in a port with no foot route, the door is refused with the
/// standing line (Limbo's global `c` typed at Stickney); otherwise a pilot stamped
/// at such a port is refused with doc 18 §4.5's line. No stamp, or a Ceres stamp,
/// passes.
pub struct IcDoorHook;

impl EventHook for IcDoorHook {
    fn move_attempt(
        &self,
        ctx: &HookReadContext<'_>,
        actor: ObjId,
        exit: Option<ObjId>,
        _dest: ObjId,
    ) -> Result<(), RichText> {
        let Some(exit) = exit else {
            return Ok(());
        };
        let w = ctx.world();
        if door(w) != Some(exit) {
            return Ok(());
        }
        let Some(door_port) = door_port() else {
            return Ok(());
        };
        let standing = w
            .get(actor)
            .and_then(|object| object.location)
            .and_then(|here| port_of_room(w, here));
        if let Some(standing) = standing
            && !standing.foot_route
        {
            return Err(RichText::tag("error", no_door_line(standing, door_port)));
        }
        if let Some(left) = last_port(w, actor)
            && !left.foot_route
        {
            return Err(RichText::tag("error", left_at_line(left)));
        }
        Ok(())
    }
}

/// **`+ic`** (doc 18 §4.5): back into the corridor at the stamped port's arrival
/// room (Ceres when there is no stamp). In order: the layer, a character, not
/// already in a port, the arrival room, then `go_home`'s shape through `Ctx` — the
/// `MoveAttempt` chain (which is what makes the sim's `PlacementGate` refuse a hull
/// occupant from cut IC4 on), the move, the lines, the room, and `Moved`.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor; nothing is written.
pub fn ic_command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled)
    {
        return Err(CmdError::msg(NOT_ENABLED));
    }
    let actor = ctx.actor;
    if ctx.w.get(actor).map(|object| object.kind) != Some(ObjKind::Player) {
        return Err(CmdError::msg(NOT_A_CHARACTER));
    }
    let from = ctx.w.get(actor).and_then(|object| object.location);
    if from.and_then(|here| port_of_room(ctx.w, here)).is_some() {
        return Err(CmdError::msg(ALREADY_IN));
    }
    let Some(port) = last_port(ctx.w, actor).or_else(|| PORTS.first()) else {
        return Err(CmdError::msg(NO_CORRIDOR));
    };
    let Some(dest) = arrival_room(ctx.w, port) else {
        return Err(CmdError::msg(NO_CORRIDOR));
    };
    if let Err(reason) = ctx.check_move_attempt(actor, None, dest) {
        ctx.out.to_actor(actor, OutputEvent::Sys(reason));
        return Ok(());
    }
    if ctx.w.move_to(actor, dest).is_err() {
        return Err(CmdError::msg(NO_CORRIDOR));
    }
    ctx.out.info(actor, return_line(port));
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
    Ok(())
}

/// An unattributed narrative line, as the kernel's movement lines are.
fn narrative(text: RichText) -> OutputEvent {
    OutputEvent::Narrative {
        kind: NarrativeKind::Emit,
        actor: None,
        text,
    }
}

/// Register `+ic`, the door hook and the stamp on the **tacsim** module, under the
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
    reg.hook(HookPoint::MoveAttempt, Arc::new(IcDoorHook));
    reg.subscribe(WorldEventKind::Moved, Arc::new(IcReturnStamp));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{GOD, LIMBO};

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

    #[test]
    fn ic1_the_help_names_ic() {
        assert!(IC_HELP.starts_with("+ic  --  "));
        assert!(IC_HELP.contains("+ic puts you back there"));
        assert_eq!(
            left_at_line(&PORTS[1]),
            "You left the corridor at Stickney Yards; +ic returns you there."
        );
        assert_eq!(
            no_door_line(&PORTS[1], &PORTS[0]),
            "There is no door from Stickney Yards to Ceres Freeport; the way there is by ship."
        );
        assert_eq!(
            return_line(&PORTS[1]),
            "You return to the corridor at Stickney Yards."
        );
    }
}
