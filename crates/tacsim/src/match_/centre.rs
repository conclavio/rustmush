//! **The simulator centre: its three rooms, its eight pods and its eight tanks**
//! (16b cut K.1; doc 16 §3.7 "Pods", §4 "Content packs").
//!
//! C.1 authored the centre in the atlas-owned OOC pack — `room.simulator_lobby`,
//! `room.sim_red`, `room.sim_blue`, `item.simulator_selector` and
//! `item.red_pod_1..4` / `item.blue_pod_1..4` — and C.2 authored the arena and
//! its eight tanks in `tacsim.pack.proving_ground`. Both halves are **content**,
//! so nothing here may say "the room called Simulator Lobby": a name is a
//! renameable label and two worlds may not agree on it. Every resolution below
//! starts from the **pack entry id**, which is the one identity a pack fixes.
//!
//! # The two id ladders
//!
//! | subject | from | through |
//! |---|---|---|
//! | the three rooms, the eight pods | the OOC pack's entry ids | the **content latch** ([`mush_atlas::travel::latched_object`]) |
//! | the eight tanks | the arena pack's `tanks[].id` (`red_1` … `blue_4`) | this module's own **roster document** |
//!
//! The first ladder already existed: `mush-content`'s latch records the object
//! every applied entry made, and one room entry makes one room.
//!
//! The second did not, and could not: eight tanks are authored **inside one**
//! arena entry, so the latch — one entry, one object — holds only the arena's
//! muster room. C.2's applier matched a standing tank to its authored slot by
//! **name**, which is the one thing a lifecycle must not do (a staffer renames
//! "Red Tank 3" and slot three of the red side silently becomes a different
//! object, or nothing). So the applier now writes what it made:
//! `tacsim:arena:<entry>` carries `slot id → object`, one document per arena,
//! rewritten on every apply of that arena and read at boot with the rest of the
//! module's state. The name match stays where it belongs — inside the applier,
//! as the rule that makes a **re-apply** idempotent — and stops being the thing
//! a running match's identity rests on.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, Ctx, ModuleWrite, ObjId, World};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::state::Team;
use crate::store::TacsimStore;

/// The atlas pack the centre's rooms and pods live in (C.1).
pub const CENTRE_PACK: &str = mush_atlas::starter::OOC_AREA_PACK;

/// The lobby's entry id in that pack.
pub const LOBBY_ENTRY: &str = "room.simulator_lobby";

/// The red team room's entry id.
pub const RED_ROOM_ENTRY: &str = "room.sim_red";

/// The blue team room's entry id.
pub const BLUE_ROOM_ENTRY: &str = "room.sim_blue";

/// The arena the centre fights on (C.2's `arenas/proving_ground.json`).
pub const CENTRE_ARENA: &str = crate::match_::PROVING_GROUND;

/// Pods per side — four, as C.1 authored them and as doc 16 §3.7 describes the
/// two team rooms.
pub const PODS_PER_SIDE: u8 = 4;

/// The eight pod entry ids, side by side, slot 1 first.
///
/// Written out rather than formatted, so a missing pod is a compile-time fact
/// about this table and never a string that happened not to resolve.
pub const POD_ENTRIES: [(Team, u8, &str); 8] = [
    (Team::Red, 1, "item.red_pod_1"),
    (Team::Red, 2, "item.red_pod_2"),
    (Team::Red, 3, "item.red_pod_3"),
    (Team::Red, 4, "item.red_pod_4"),
    (Team::Blue, 1, "item.blue_pod_1"),
    (Team::Blue, 2, "item.blue_pod_2"),
    (Team::Blue, 3, "item.blue_pod_3"),
    (Team::Blue, 4, "item.blue_pod_4"),
];

/// Which of the centre's three rooms an object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CentreRoom {
    /// The lobby, where the selector stands.
    Lobby,
    /// A team room and its four pods.
    Team(Team),
}

impl CentreRoom {
    /// The word `SIM STATUS` prints.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            CentreRoom::Lobby => "lobby",
            CentreRoom::Team(Team::Red) => "red room",
            CentreRoom::Team(Team::Blue) => "blue room",
        }
    }
}

/// One entry of the centre pack, resolved through the content latch.
///
/// `None` for a world whose OOC pack is older than version 4 (the centre is not
/// in it), for a world with no content module at all, and for an object the
/// latch names that has since been destroyed — all three of which mean the same
/// thing to every caller: **there is no centre here**, so refuse.
#[must_use]
pub fn centre_object(w: &World, entry: &str) -> Option<ObjId> {
    let key = mush_atlas::RecordKey::new(
        CanonicalId::new(CENTRE_PACK).ok()?,
        CanonicalId::new(entry).ok()?,
    );
    mush_atlas::travel::latched_object(w, &key)
}

/// The lobby, the red room and the blue room, in that order, when all three are
/// in this world.
#[must_use]
pub fn centre_rooms(w: &World) -> Option<[ObjId; 3]> {
    Some([
        centre_object(w, LOBBY_ENTRY)?,
        centre_object(w, RED_ROOM_ENTRY)?,
        centre_object(w, BLUE_ROOM_ENTRY)?,
    ])
}

/// Which of the centre's rooms `room` is, if it is one of them.
#[must_use]
pub fn room_role(w: &World, room: ObjId) -> Option<CentreRoom> {
    if centre_object(w, LOBBY_ENTRY) == Some(room) {
        return Some(CentreRoom::Lobby);
    }
    if centre_object(w, RED_ROOM_ENTRY) == Some(room) {
        return Some(CentreRoom::Team(Team::Red));
    }
    if centre_object(w, BLUE_ROOM_ENTRY) == Some(room) {
        return Some(CentreRoom::Team(Team::Blue));
    }
    None
}

/// Where `actor` is standing, as the centre sees it.
///
/// **A pilot inside a pod counts as being in that pod's team room.** Cut K.2's
/// `BOARD` places the character *inside* the pod (`Transition::Place Interior`,
/// doc 16 §3.7), so `location` is the pod Thing and not the room — and doc 16
/// §3.8 gives `SIM ABORT` to *any participant*, which means the one place a
/// pilot is most likely to type it is the one place a room-only test would
/// refuse it from.
#[must_use]
pub fn where_in_centre(w: &World, actor: ObjId) -> Option<CentreRoom> {
    let at = w.get(actor)?.location?;
    if let Some(role) = room_role(w, at) {
        return Some(role);
    }
    let (team, _) = pod_slot(w, at)?;
    Some(CentreRoom::Team(team))
}

/// The pod object for one side's slot `index` (1-based).
#[must_use]
pub fn pod_object(w: &World, team: Team, index: u8) -> Option<ObjId> {
    let entry = POD_ENTRIES
        .iter()
        .find(|(side, slot, _)| *side == team && *slot == index)
        .map(|(_, _, entry)| *entry)?;
    centre_object(w, entry)
}

/// Which side and slot a pod is, if it is one of the eight.
#[must_use]
pub fn pod_slot(w: &World, pod: ObjId) -> Option<(Team, u8)> {
    POD_ENTRIES
        .iter()
        .find(|(_, _, entry)| centre_object(w, entry) == Some(pod))
        .map(|(team, index, _)| (*team, *index))
}

// ---------------------------------------------------------------------------
// The arena's roster: slot id -> object
// ---------------------------------------------------------------------------

/// The document key prefix one arena's roster is written under.
pub const ROSTER_PREFIX: &str = "tacsim:arena:";

/// The roster document's key for one arena entry.
#[must_use]
pub fn roster_key(arena: &CanonicalId) -> SmolStr {
    SmolStr::new(format!("{ROSTER_PREFIX}{arena}"))
}

/// The arena entry a roster key names, for the boot loader.
#[must_use]
pub fn roster_id_of_key(key: &str) -> Option<CanonicalId> {
    CanonicalId::parse_canonical(key.strip_prefix(ROSTER_PREFIX)?).ok()
}

/// **What one arena's applier made** — the muster room and the object behind
/// every authored unit slot (16b cut K.1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArenaRoster {
    /// The arena entry this roster belongs to.
    pub arena: String,
    /// The muster room the applier hosts the domain on.
    pub room: u32,
    /// `tanks[].id` → the object that slot names, in slot-id order.
    pub units: BTreeMap<String, u32>,
}

impl ArenaRoster {
    /// The object one authored slot names.
    #[must_use]
    pub fn unit(&self, slot: &str) -> Option<ObjId> {
        self.units.get(slot).copied().map(ObjId)
    }
}

/// **Record what the arena applier made** (16b cut K.1).
///
/// Called from `crate::content`'s `ArenaApplier` at the end of an apply, with
/// every slot it created or adopted. Two writes in one: the module's own RAM
/// copy, so the very boot that applies the pack can resolve a slot without a
/// reboot, and the document, so every later boot can.
///
/// The op is stamped with **this crate's** namespace by
/// [`crate::store::push_op`], exactly as `mush_atlas::put_site` stamps the
/// atlas's from the same applier context: the applier runs under the *content*
/// module's `Ctx`, but the code, the key and the record are this module's, so
/// the reach is in-family and wants no delegation.
pub(crate) fn record_roster(
    ctx: &mut Ctx<'_>,
    arena: &CanonicalId,
    room: ObjId,
    units: &[(CanonicalId, ObjId)],
) {
    let roster = ArenaRoster {
        arena: arena.to_string(),
        room: room.0,
        units: units
            .iter()
            .map(|(slot, object)| (slot.to_string(), object.0))
            .collect(),
    };
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.put_arena_roster(arena.clone(), roster.clone());
    }
    let Ok(value) = serde_json::to_value(&roster) else {
        tracing::error!(%arena, "tacsim: an arena roster did not serialize; its slots stay unresolvable");
        return;
    };
    crate::store::push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: roster_key(arena),
            value,
        },
    );
}

/// Read one roster out of a boot document, applying the identity rule (DL-07):
/// a body that disagrees with its own key is not this arena's roster.
#[must_use]
pub fn roster_from_document(arena: &CanonicalId, value: &serde_json::Value) -> Option<ArenaRoster> {
    serde_json::from_value::<ArenaRoster>(value.clone())
        .ok()
        .filter(|roster| roster.arena == arena.as_str())
}

/// The object one arena slot names in this world, or `None` when the arena has
/// never been applied here, the roster is lost, or the object is gone.
#[must_use]
pub fn slot_object(w: &World, arena: &str, slot: &str) -> Option<ObjId> {
    let arena = CanonicalId::new(arena).ok()?;
    let object = w
        .module_state::<TacsimStore>()?
        .arena_roster(&arena)?
        .unit(slot)?;
    w.get(object).map(|object| object.id)
}

/// The authored slot id of side `team`'s `index`-th unit on `arena` — the
/// arena's own `tanks` list, filtered by side and taken in authored order, which
/// is the order doc 16 §3.8 means by "slot n of a side".
#[must_use]
pub fn slot_id(w: &World, arena: &str, team: Team, index: u8) -> Option<SmolStr> {
    let arena = CanonicalId::new(arena).ok()?;
    let definitions = w.module_state::<TacsimStore>()?.definitions();
    let def = definitions.arenas.get(&arena)?;
    def.tanks
        .iter()
        .filter(|tank| tank.team == team)
        .nth(usize::from(index.saturating_sub(1)))
        .map(|tank| SmolStr::new(tank.id.as_str()))
}

/// **The tank a pod flies** (16b cut K.1; doc 16 §3.8's "slot n of a side").
///
/// Pod *n* of a team room is slot *n* of that side, and slot *n* of that side is
/// the *n*-th unit the arena pack authors for it — "Red Tank n" and "Blue Tank
/// n" as the shipped proving ground spells them, resolved through the arena's
/// **slot id** and never through that name.
///
/// `None` when the object is not one of the eight pods, when the centre's arena
/// is not applied in this world, or when the slot's object is gone.
#[must_use]
pub fn tank_of(w: &World, pod: ObjId) -> Option<ObjId> {
    let (team, index) = pod_slot(w, pod)?;
    tank_for_slot(w, team, index)
}

/// The tank behind one side's slot, without a pod in hand.
#[must_use]
pub fn tank_for_slot(w: &World, team: Team, index: u8) -> Option<ObjId> {
    let slot = slot_id(w, CENTRE_ARENA, team, index)?;
    slot_object(w, CENTRE_ARENA, &slot)
}

/// **Whether the centre can start a match, for the boot log** (16b cut F.4).
///
/// `(the centre's three rooms resolve, how many of the arena's roster tanks are
/// standing in this world)`. Both halves have to be true before two players can
/// play: the rooms come with the starter area and the tanks come with the game's
/// boot packs, which ride on `[tacsim].enabled`, so a world can easily have one
/// and not the other — and the way that used to surface was `SIM START` refusing
/// hours later, with advice nobody could act on. The composition root says it at
/// boot instead.
#[must_use]
pub fn readiness(w: &World) -> (bool, usize) {
    let rooms = centre_rooms(w).is_some();
    let tanks = [Team::Red, Team::Blue]
        .into_iter()
        .flat_map(|team| (1..=PODS_PER_SIDE).map(move |index| (team, index)))
        .filter(|(team, index)| tank_for_slot(w, *team, *index).is_some())
        .count();
    (rooms, tanks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roster_key_round_trips_and_no_other_tacsim_key_is_one() {
        let arena = CanonicalId::new(CENTRE_ARENA).expect("a canonical id");
        assert_eq!(
            roster_key(&arena).as_str(),
            "tacsim:arena:tacsim.arena.proving_ground"
        );
        assert_eq!(
            roster_id_of_key("tacsim:arena:tacsim.arena.proving_ground"),
            Some(arena)
        );
        assert_eq!(roster_id_of_key("tacsim:match:3"), None);
        assert_eq!(roster_id_of_key("tacsim:arena:"), None);
    }

    #[test]
    fn a_roster_document_that_disagrees_with_its_key_is_not_adopted() {
        let arena = CanonicalId::new(CENTRE_ARENA).expect("a canonical id");
        let good = serde_json::json!({
            "arena": CENTRE_ARENA,
            "room": 7,
            "units": { "red_1": 8 },
        });
        let roster = roster_from_document(&arena, &good).expect("the roster loads");
        assert_eq!(roster.unit("red_1"), Some(ObjId(8)));
        assert_eq!(roster.unit("blue_1"), None);
        let other = serde_json::json!({
            "arena": "tacsim.arena.stand_in",
            "room": 7,
            "units": { "red_1": 8 },
        });
        assert_eq!(roster_from_document(&arena, &other), None);
    }

    #[test]
    fn every_pod_slot_is_declared_once_per_side() {
        for team in Team::ALL {
            for index in 1..=PODS_PER_SIDE {
                assert_eq!(
                    POD_ENTRIES
                        .iter()
                        .filter(|(side, slot, _)| *side == team && *slot == index)
                        .count(),
                    1,
                    "{} pod {index}",
                    team.as_str()
                );
            }
        }
    }
}
