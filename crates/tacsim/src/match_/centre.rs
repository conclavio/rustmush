//! **The simulator centre: its three rooms, its twelve pods and its twelve
//! tanks** (16b cut K.1; doc 16 §3.7 "Pods", §4 "Content packs"; six a side
//! since 16b cut Z.3).
//!
//! C.1 authored the centre in the atlas-owned OOC pack — `room.simulator_lobby`,
//! `room.sim_red`, `room.sim_blue`, `item.simulator_selector` and
//! `item.red_pod_1..4` / `item.blue_pod_1..4` — and C.2 authored the arena and
//! its eight tanks in `tacsim.pack.proving_ground`. **16b cut Z.3** added pods
//! and slots 5 and 6 a side to both packs (`neighborhood.ooc_area` 7, the arena
//! pack 3), which a standing world takes by hand from the `@atlas/pack` shelf;
//! until it does, [`fieldable_per_side`] holds the lobby to the four it has.
//!
//! Both halves are **content**, so nothing here may say "the room called
//! Simulator Lobby": a name is a
//! renameable label and two worlds may not agree on it. Every resolution below
//! starts from the **pack entry id**, which is the one identity a pack fixes.
//!
//! # The two id ladders
//!
//! | subject | from | through |
//! |---|---|---|
//! | the three rooms, the twelve pods | the OOC pack's entry ids | the **content latch** ([`mush_atlas::travel::latched_object`]) |
//! | the twelve tanks | the arena pack's `tanks[].id` (`red_1` … `blue_6`) | this module's own **roster document** |
//!
//! The first ladder already existed: `mush-content`'s latch records the object
//! every applied entry made, and one room entry makes one room.
//!
//! The second did not, and could not: a side's tanks are authored **inside one**
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

/// Pods per side — six since 16b cut Z.3 (16c N7 at Q39's default: six first);
/// C.1 authored four, and doc 16 §3.7 describes the two team rooms.
///
/// This is what the **binary** knows how to seat. What a given **world** can
/// seat is [`fieldable_per_side`], which is never more than this and is less on
/// a world that has not taken the Z.3 packs.
pub const PODS_PER_SIDE: u8 = 6;

/// The twelve pod entry ids, side by side, slot 1 first: red 1 to 6, then blue
/// 1 to 6.
///
/// Written out rather than formatted, so a missing pod is a compile-time fact
/// about this table and never a string that happened not to resolve. **The
/// order within a side is load-bearing** — [`pod_slot`], the cockpit's pod pair,
/// the lobby's fill loop and the callsign all read the slot index — so 16b cut
/// Z.3 appended slots 5 and 6 after slot 4 of each side and moved nothing.
pub const POD_ENTRIES: [(Team, u8, &str); 12] = [
    (Team::Red, 1, "item.red_pod_1"),
    (Team::Red, 2, "item.red_pod_2"),
    (Team::Red, 3, "item.red_pod_3"),
    (Team::Red, 4, "item.red_pod_4"),
    (Team::Red, 5, "item.red_pod_5"),
    (Team::Red, 6, "item.red_pod_6"),
    (Team::Blue, 1, "item.blue_pod_1"),
    (Team::Blue, 2, "item.blue_pod_2"),
    (Team::Blue, 3, "item.blue_pod_3"),
    (Team::Blue, 4, "item.blue_pod_4"),
    (Team::Blue, 5, "item.blue_pod_5"),
    (Team::Blue, 6, "item.blue_pod_6"),
];

/// Which of the centre's three rooms an object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CentreRoom {
    /// The lobby, where the selector stands.
    Lobby,
    /// A team room and its pods.
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

/// Which side and slot a pod is, if it is one of the twelve.
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
/// `None` when the object is not one of the twelve pods, when the centre's arena
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

/// **How many slots a side this world can really seat** (16b cut Z.3; 16c N7).
///
/// The largest `k` in `1..=`[`PODS_PER_SIDE`] such that, on **both** sides,
/// every slot `1..=k` has a pod ([`pod_object`]) and a tank
/// ([`tank_for_slot`]); never below one.
///
/// [`PODS_PER_SIDE`] is what the binary knows how to seat, and the two packs
/// that make the pods and the tanks are content a standing world takes by hand
/// (`deploy/README.md`). A Z.3 binary on a world still at `ooc_area@6` and the
/// version-2 arena has eight pods and eight tanks, and before this cap the lobby
/// handed out slot 5 anyway — and then every `SIM START` refused with "no
/// simulator centre", because the pod lookup runs before the unit check. So the
/// lobby's cap follows what exists, and an un-migrated world stays four a side
/// exactly as it was.
///
/// Both sides, and a prefix: a side that has slot 6 but not slot 5 fields four,
/// not five (the fill and the callsigns read the index, so a hole cannot be
/// skipped: counting whole slots would hand out slot 5 with nothing behind it),
/// and a centre whose red side has six and whose blue side has four fields four
/// a side, because the lobby never lets one side ask for more than the other.
///
/// One, not zero, on a world with no centre at all: every caller is a lobby
/// transition that has already refused a missing centre in its own words, and a
/// cap of zero would be a second, less helpful refusal of the same fact.
#[must_use]
pub fn fieldable_per_side(w: &World) -> u8 {
    let whole = |index: u8| {
        Team::ALL.iter().all(|team| {
            pod_object(w, *team, index).is_some() && tank_for_slot(w, *team, index).is_some()
        })
    };
    (1..=PODS_PER_SIDE)
        .take_while(|index| whole(*index))
        .last()
        .unwrap_or(1)
}

/// **What the centre has, for the boot log** (16b cut F.4; counted against
/// twelve since 16b cut Z.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Readiness {
    /// The centre's three rooms resolve.
    pub rooms: bool,
    /// How many of the [`POD_ENTRIES`] resolve to a pod in this world.
    pub pods: usize,
    /// How many of the arena's roster slots `1..=`[`PODS_PER_SIDE`] a side
    /// resolve to a standing tank.
    pub tanks: usize,
}

impl Readiness {
    /// The count a whole centre has of each: [`PODS_PER_SIDE`] a side, both
    /// sides.
    pub const WHOLE: usize = 2 * PODS_PER_SIDE as usize;

    /// Every pod and every tank the binary can seat is here.
    #[must_use]
    pub const fn is_whole(&self) -> bool {
        self.pods == Self::WHOLE && self.tanks == Self::WHOLE
    }

    /// **The boot's warning for this reading** (16b cut Z.3): `None` for a
    /// whole centre, which the boot logs as ready; otherwise the one sentence
    /// `mush_server::boot` warns with, chosen here so the choice and the words
    /// are one pure function a test can pin.
    ///
    /// * The rooms and some tanks, not the whole count: **partly ready** — a
    ///   world that has not taken the Z.3 packs by hand (eight pods, eight
    ///   tanks), which plays the smaller count a side ([`fieldable_per_side`]).
    /// * The rooms and no tanks: **not ready**, the arena's roster is empty.
    /// * No rooms: **not ready**, the OOC pack is latched below the shipped
    ///   version.
    #[must_use]
    pub fn warning(&self) -> Option<String> {
        let whole = Self::WHOLE;
        if self.rooms && self.is_whole() {
            None
        } else if self.rooms && self.tanks > 0 {
            Some(format!(
                "simulator centre partly ready: {} of {whole} pods, {} of {whole} tanks; the \
                 lobby fields the smaller count a side; take the packs by hand \
                 (deploy/README.md)",
                self.pods, self.tanks
            ))
        } else if self.rooms {
            Some(
                "simulator centre NOT ready: its rooms and pods are here but the arena's roster \
                 is empty, so `SIM START` will refuse. Take the arena pack by hand with \
                 `@atlas/pack plan tacsim.pack.proving_ground` and `@atlas/pack apply`, with the \
                 centre idle (deploy/README.md)."
                    .to_owned(),
            )
        } else {
            Some(
                "simulator centre NOT ready: this world has no centre rooms. An existing world \
                 latched `neighborhood.ooc_area` below the shipped version; take the upgrade with \
                 `@atlas/pack plan` and `@atlas/pack apply` (deploy/README.md)."
                    .to_owned(),
            )
        }
    }
}

/// **Whether the centre can start a match, for the boot log** (16b cut F.4).
///
/// The rooms, the pods and the arena's roster tanks, counted. All three have to
/// be there before two players can play: the rooms and the pods come with the
/// starter area and the tanks come with the game's boot packs, which ride on
/// `[tacsim].enabled`, so a world can easily have one and not the other — and
/// the way that used to surface was `SIM START` refusing hours later, with
/// advice nobody could act on. The composition root says it at boot instead.
///
/// **16b cut Z.3** counts the pods as well as the tanks, both against
/// [`Readiness::WHOLE`], because a world that has not taken the Z.3 packs by
/// hand has eight of each and plays four a side ([`fieldable_per_side`]): the
/// boot says so, and names the route, rather than calling that world ready.
#[must_use]
pub fn readiness(w: &World) -> Readiness {
    let rooms = centre_rooms(w).is_some();
    let pods = POD_ENTRIES
        .iter()
        .filter(|(team, index, _)| pod_object(w, *team, *index).is_some())
        .count();
    let tanks = Team::ALL
        .into_iter()
        .flat_map(|team| (1..=PODS_PER_SIDE).map(move |index| (team, index)))
        .filter(|(team, index)| tank_for_slot(w, *team, *index).is_some())
        .count();
    Readiness { rooms, pods, tanks }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// **A whole centre, composed as production composes it** (16b cut Z.3): the
    /// starter area on (the rooms and the twelve pods of `ooc_area@7`) and the
    /// game's boot packs on (the version-3 arena and its twelve tanks).
    ///
    /// The character sheet is left out, as `tests/match_lifecycle.rs` leaves it
    /// out: it registers a job payload handler once per process, and this test
    /// binary composes more than one world.
    pub(crate) fn whole_centre() -> mush_testkit::TestWorld {
        use crate::composition as comp;

        let llm = std::sync::Arc::new(
            mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
                .expect("a disabled LLM service composes"),
        );
        let cfg = crate::TacsimConfig {
            enabled: true,
            ..crate::TacsimConfig::default()
        };
        let onboarding = mush_core::OnboardingConfig::default();
        let game = crate::GameEconomyConfig::default();
        let (mut mods, inventory) = comp::server_modules_with_tacsim(
            mush_weather::WeatherConfig::default(),
            mush_economy::EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            &game,
            comp::recovery_policy(&cfg.recovery),
        );
        mods.push(comp::worldgen_module_with(
            std::sync::Arc::clone(&llm),
            mush_worldgen::WorldgenConfig::default(),
        ));
        mods.push(comp::npc_module_with(
            comp::npc_agent_adapter(),
            &mush_config::config::NpcCfg::default(),
        ));
        // Keep the lane definitions and applied content together, as production
        // does: the full relay star needs the Fleet/Postmaster programme.
        let game_content = cfg.enabled;
        mods.push(comp::tacsim_module_wired(cfg, game_content, inventory));
        mods.push(
            comp::content_module_with(true, game_content).expect("the content module composes"),
        );
        mush_testkit::TestWorld::with_modules(mods)
    }

    /// **The world a Z.3 binary meets before the packs are taken** (16b cut
    /// Z.3): twelve pods, because this test composes the shipped OOC pack, and a
    /// roster that names only the eight slots the version-2 arena made — the
    /// shape an un-migrated world's roster document has.
    pub(crate) fn twelve_pods_and_eight_tanks() -> mush_testkit::TestWorld {
        let mut tw = whole_centre();
        let arena = CanonicalId::new(CENTRE_ARENA).expect("a canonical id");
        let store = tw
            .world_mut()
            .module_state_mut::<TacsimStore>()
            .expect("the layer is composed");
        let mut roster = store
            .arena_roster(&arena)
            .cloned()
            .expect("the arena applied at boot");
        for slot in ["red_5", "red_6", "blue_5", "blue_6"] {
            assert!(roster.units.remove(slot).is_some(), "{slot} was applied");
        }
        store.put_arena_roster(arena, roster);
        tw
    }

    #[test]
    fn the_twelve_pod_entries_are_red_one_to_six_then_blue_one_to_six() {
        assert_eq!(POD_ENTRIES.len(), 12);
        assert_eq!(usize::from(PODS_PER_SIDE) * 2, POD_ENTRIES.len());
        let mut want = Vec::new();
        for (team, word) in [(Team::Red, "red"), (Team::Blue, "blue")] {
            for index in 1..=PODS_PER_SIDE {
                want.push((team, index, format!("item.{word}_pod_{index}")));
            }
        }
        let have: Vec<(Team, u8, String)> = POD_ENTRIES
            .iter()
            .map(|(team, index, entry)| (*team, *index, (*entry).to_owned()))
            .collect();
        assert_eq!(have, want, "the slot index is read in this order");
    }

    #[test]
    fn the_fieldable_count_follows_the_pods_and_tanks_that_exist() {
        let whole = whole_centre();
        assert_eq!(
            fieldable_per_side(whole.world()),
            6,
            "twelve pods and twelve tanks field six a side"
        );
        let whole_reading = readiness(whole.world());
        assert!(whole_reading.rooms);
        assert_eq!((whole_reading.pods, whole_reading.tanks), (12, 12));
        assert!(whole_reading.is_whole());

        let old = twelve_pods_and_eight_tanks();
        assert_eq!(
            fieldable_per_side(old.world()),
            4,
            "an arena that has not taken version 3 fields four a side"
        );
        let old_reading = readiness(old.world());
        assert_eq!((old_reading.pods, old_reading.tanks), (12, 8));
        assert!(!old_reading.is_whole());

        // One side short is both sides short: the lobby never lets one side
        // field more than the other.
        let mut lopsided = whole_centre();
        let arena = CanonicalId::new(CENTRE_ARENA).expect("a canonical id");
        let store = lopsided
            .world_mut()
            .module_state_mut::<TacsimStore>()
            .expect("the layer is composed");
        let mut roster = store.arena_roster(&arena).cloned().expect("applied");
        roster.units.remove("blue_6");
        store.put_arena_roster(arena.clone(), roster);
        assert_eq!(fieldable_per_side(lopsided.world()), 5);

        // A hole is a prefix's end, not a gap to skip: slot 6 standing behind a
        // missing slot 5 fields four, never five.
        let mut holed = whole_centre();
        let store = holed
            .world_mut()
            .module_state_mut::<TacsimStore>()
            .expect("the layer is composed");
        let mut roster = store.arena_roster(&arena).cloned().expect("applied");
        roster.units.remove("red_5");
        store.put_arena_roster(arena, roster);
        assert!(tank_for_slot(holed.world(), Team::Red, 6).is_some());
        assert_eq!(
            fieldable_per_side(holed.world()),
            4,
            "the hole at 5 ends it"
        );

        // The pods count as the tanks do: a pod that is gone ends the prefix
        // at the slot before it, with every tank standing.
        let mut podless = whole_centre();
        let pod = pod_object(podless.world(), Team::Blue, 3).expect("blue pod 3");
        podless
            .world_mut()
            .set_flag(pod, mush_core::Flags::GOING, true)
            .expect("the pod is going");
        assert_eq!(pod_object(podless.world(), Team::Blue, 3), None);
        assert_eq!(
            fieldable_per_side(podless.world()),
            2,
            "a missing pod holds the cap down however many tanks stand"
        );
        let podless_reading = readiness(podless.world());
        assert_eq!((podless_reading.pods, podless_reading.tanks), (11, 12));
        assert!(!podless_reading.is_whole());

        // And a world with no centre at all answers one, never zero.
        let bare = mush_testkit::TestWorld::new();
        assert_eq!(fieldable_per_side(bare.world()), 1);
        let bare_reading = readiness(bare.world());
        assert_eq!(
            bare_reading,
            Readiness {
                rooms: false,
                pods: 0,
                tanks: 0
            }
        );
    }

    /// **The boot's three warnings, verbatim** (16b cut Z.3): the words
    /// `mush_server::boot` logs, and which reading gets which. Eight and eight —
    /// dev1 before the packs are taken — is partly ready, never ready.
    #[test]
    fn the_boot_warning_is_chosen_by_the_reading_and_says_the_route() {
        let reading = |rooms: bool, pods: usize, tanks: usize| Readiness { rooms, pods, tanks };
        assert_eq!(reading(true, 12, 12).warning(), None, "whole is ready");
        assert_eq!(
            reading(true, 8, 8).warning().as_deref(),
            Some(
                "simulator centre partly ready: 8 of 12 pods, 8 of 12 tanks; the lobby fields \
                 the smaller count a side; take the packs by hand (deploy/README.md)"
            )
        );
        assert_eq!(
            reading(true, 12, 8).warning().as_deref(),
            Some(
                "simulator centre partly ready: 12 of 12 pods, 8 of 12 tanks; the lobby fields \
                 the smaller count a side; take the packs by hand (deploy/README.md)"
            ),
            "the pods first, the tanks second"
        );
        assert!(
            reading(true, 8, 12)
                .warning()
                .is_some_and(|line| line.contains("8 of 12 pods, 12 of 12 tanks")),
            "twelve tanks behind eight pods is not whole"
        );
        assert_eq!(
            reading(true, 12, 0).warning().as_deref(),
            Some(
                "simulator centre NOT ready: its rooms and pods are here but the arena's roster \
                 is empty, so `SIM START` will refuse. Take the arena pack by hand with \
                 `@atlas/pack plan tacsim.pack.proving_ground` and `@atlas/pack apply`, with the \
                 centre idle (deploy/README.md)."
            )
        );
        for (pods, tanks) in [(0, 0), (12, 12)] {
            assert_eq!(
                reading(false, pods, tanks).warning().as_deref(),
                Some(
                    "simulator centre NOT ready: this world has no centre rooms. An existing \
                     world latched `neighborhood.ooc_area` below the shipped version; take the \
                     upgrade with `@atlas/pack plan` and `@atlas/pack apply` \
                     (deploy/README.md)."
                ),
                "no rooms outranks every count"
            );
        }
        // And the worlds the other test composes read as the boot would say.
        assert_eq!(readiness(whole_centre().world()).warning(), None);
        assert!(
            readiness(twelve_pods_and_eight_tanks().world())
                .warning()
                .is_some_and(|line| line.starts_with(
                    "simulator centre partly ready: 12 of 12 \
                                                       pods, 8 of 12 tanks;"
                ))
        );
    }

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
