//! **C.1 (16b §8 cut C; doc 16 §3.7, §4)** — the simulator centre in the OOC
//! pack: items that are interior spaces, and the pack's own steps from version 3
//! to the shipped one.
//!
//! An `item` entry carrying `interior: true` is applied as a Thing a character
//! can be placed *inside* — the two facts `@spatial/place/interior` looks for,
//! the `sim.interior` behaviour and an interior-space record — which is what a
//! pod is. A plain item is not one, and says so.
//!
//! The upgrade half drives the pack's own latch: a world already at
//! `ooc_area@3` takes the shipped version as an update that adds the simulator
//! lobby, the red and blue rooms, their exits, the selector and the eight pods
//! and moves nothing that was already standing; a fresh world lands all of it at
//! once.
//!
//! **16b cut F.4** adds the second step, and it is a different shape: version 5
//! creates nothing at all. It rewrites the eleven strings that told the players
//! the simulator was not built, corrects the Welcome Hall's sign, which sent
//! them at the one word the `SIM` verb shadows, and takes `sim` off three exits
//! so that verb can be typed. A world at version 4 has the old strings standing
//! on real objects, so the upgrade has to *update* them in place — which is the
//! one thing a create-only upgrade never proves, and a room `detail:` attribute
//! (the sign) is a different kind of update from an item's `desc`.

use std::collections::BTreeMap;

use mush_atlas::{AtlasModule, AtlasState, starter};
use mush_content::{Appliers, ContentModule, ContentState, parse_pack};
use mush_core::{
    Audience, BehaviorBinding, BehaviorId, CanonicalId, Flags, GOD, GameModule, ObjId, ObjKind,
};
use mush_proto::OutputEvent;
use mush_sim::{SimulationModule, SpatialParent, SpatialState};
use mush_testkit::TestWorld;
use serde_json::Value;

/// The eight pods, as C.2 references them: entry id and the object name.
const PODS: [(&str, &str); 8] = [
    ("item.red_pod_1", "Red Pod 1"),
    ("item.red_pod_2", "Red Pod 2"),
    ("item.red_pod_3", "Red Pod 3"),
    ("item.red_pod_4", "Red Pod 4"),
    ("item.blue_pod_1", "Blue Pod 1"),
    ("item.blue_pod_2", "Blue Pod 2"),
    ("item.blue_pod_3", "Blue Pod 3"),
    ("item.blue_pod_4", "Blue Pod 4"),
];

/// The fifteen entries version 4 added to version 3, and the shipped pack still
/// carries: three rooms, three exit rows (six exit objects) and nine items.
const ADDED: [&str; 15] = [
    "room.simulator_lobby",
    "room.sim_red",
    "room.sim_blue",
    "exit.hall_simulator_lobby",
    "exit.simulator_lobby_red",
    "exit.simulator_lobby_blue",
    "item.simulator_selector",
    "item.red_pod_1",
    "item.red_pod_2",
    "item.red_pod_3",
    "item.red_pod_4",
    "item.blue_pod_1",
    "item.blue_pod_2",
    "item.blue_pod_3",
    "item.blue_pod_4",
];

/// The version of `neighborhood.ooc_area` this tree ships. Read off the pack so
/// the fixtures below cannot drift from it (16b cut F.4 moved it 4 -> 5).
fn shipped_version() -> u32 {
    parse_pack(starter::OOC_AREA_JSON)
        .expect("the shipped pack parses")
        .manifest()
        .version()
}

/// The same number as a constant, for the fixtures that build a neighbouring
/// version. Asserted against [`shipped_version`] by
/// [`the_fixtures_track_the_shipped_version`]. (16b cut D.5 moved it 5 -> 6.)
const SHIPPED_VERSION: u32 = 6;

fn cid(s: &str) -> CanonicalId {
    CanonicalId::new(s).expect("a canonical id")
}

/// The lot's production shape plus the simulation, which is what holds an
/// interior-space record. The shelf is always the shipped packs:
/// that is what `@atlas/pack plan` reads.
fn modules(boot_packs: Vec<String>, shelf: Vec<String>) -> Vec<Box<dyn GameModule>> {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    let content = ContentModule::new()
        .with_appliers(appliers)
        .with_boot_packs(
            boot_packs
                .iter()
                .map(|p| parse_pack(p).expect("a boot pack parses"))
                .collect(),
        );
    let shelf = shelf
        .iter()
        .map(|p| parse_pack(p).expect("a shelved pack parses"))
        .collect();
    vec![
        Box::new(mush_npc::NpcModule::new()),
        Box::new(SimulationModule::new()),
        Box::new(AtlasModule::new().with_shelf(shelf)),
        Box::new(content),
    ]
}

/// The shipped packs as bytes, in boot order.
fn shipped() -> Vec<String> {
    starter::SOURCES
        .iter()
        .map(|(_, j)| (*j).to_string())
        .collect()
}

/// A world with the whole shipped starter area, shelving `shelf`.
fn lot_shelving(shelf: Vec<String>) -> TestWorld {
    TestWorld::with_modules(modules(shipped(), shelf))
}

/// A world with the whole shipped starter area.
fn lot() -> TestWorld {
    lot_shelving(shipped())
}

/// **One version past the shipped one**: nothing but the eight pods repainted,
/// so the latch plans eight *updates* and `create_item` meets a pod that already
/// exists, is already an interior space, and already holds a pilot.
fn ooc_repainted() -> String {
    let mut doc: Value = serde_json::from_str(starter::OOC_AREA_JSON).expect("the pack is json");
    doc["manifest"]["version"] = Value::from(SHIPPED_VERSION + 1);
    for (id, _) in PODS {
        doc["entries"][id]["def"]["desc"] =
            Value::String("Repainted, and otherwise the same shell.".to_string());
    }
    doc["manifest"]["digest"] = Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = parse_pack(&unsigned)
        .expect("the repainted pack parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = Value::String(digest);
    serde_json::to_string(&doc).expect("json")
}

/// **Version 3 of the OOC pack**, rebuilt from the shipped one by taking the
/// simulator centre back out: every entry [`ADDED`] names, the room budget back
/// to 7, the version back to 3, the digest re-derived. (Version 3's bytes are
/// not in the tree any more; this is the world an upgrade meets, built from the
/// one source of truth so it cannot drift. The Welcome Hall's own text therefore
/// matches the shipped one's, which is why the upgrade below plans one update
/// and not two.)
fn ooc_v3() -> String {
    let mut doc: Value = serde_json::from_str(starter::OOC_AREA_JSON).expect("the pack is json");
    doc["manifest"]["version"] = Value::from(3);
    if let Some(entries) = doc["entries"].as_object_mut() {
        for id in ADDED {
            assert!(entries.remove(id).is_some(), "{id} is in the shipped pack");
        }
    }
    doc["entries"]["neighborhood"]["def"]["room_budget"]["maximum"] = Value::from(7);
    doc["manifest"]["digest"] = Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = parse_pack(&unsigned)
        .expect("version 3 parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = Value::String(digest);
    serde_json::to_string(&doc).expect("json")
}

fn texts(tw: &TestWorld, events: &[(Audience, OutputEvent)]) -> String {
    tw.seen_by(events, GOD).join("\n")
}

fn say(tw: &mut TestWorld, line: &str) -> String {
    let events = tw.run(GOD, line);
    texts(tw, &events)
}

fn live(tw: &TestWorld) -> impl Iterator<Item = &mush_core::Obj> {
    tw.world()
        .iter()
        .filter(|o| !o.flags.contains(Flags::GOING))
}

fn named(tw: &TestWorld, kind: ObjKind, name: &str) -> ObjId {
    live(tw)
        .find(|o| o.kind == kind && o.name == name)
        .unwrap_or_else(|| panic!("no {kind:?} named {name}"))
        .id
}

fn find(tw: &TestWorld, kind: ObjKind, name: &str) -> Option<ObjId> {
    live(tw)
        .find(|o| o.kind == kind && o.name == name)
        .map(|o| o.id)
}

fn state(tw: &TestWorld) -> &SpatialState {
    tw.world()
        .module_state::<SpatialState>()
        .expect("the simulation is composed")
}

/// Every live object as `(id, kind, name, location)` — the shape of "nothing
/// that existed moved".
fn census(tw: &TestWorld) -> BTreeMap<u32, (ObjKind, String, Option<ObjId>)> {
    live(tw)
        .map(|o| (o.id.0, (o.kind, o.name.to_string(), o.location)))
        .collect()
}

/// Give `obj` the behaviour a placement needs, as staff would with
/// `@behavior/add`.
fn participant(tw: &mut TestWorld, obj: ObjId) {
    tw.world_mut()
        .attach_behavior(
            obj,
            BehaviorId::new("sim.participant").expect("a behavior id"),
            BehaviorBinding {
                schema_version: mush_core::BINDING_SCHEMA_VERSION,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .expect("the behaviour attaches");
}

/// Plan and apply the shelved pack, returning what staff were told by each.
fn replan(tw: &mut TestWorld, pack: &str) -> (String, String) {
    let planned = say(tw, &format!("@atlas/pack plan {pack}"));
    let fingerprint = planned
        .lines()
        .find_map(|l| l.trim().strip_prefix("fingerprint "))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no fingerprint in:\n{planned}"))
        .to_string();
    let applied = say(tw, &format!("@atlas/pack apply {fingerprint}"));
    (planned, applied)
}

// ---- C0: an item that is an interior space -----------------------------------

#[test]
fn a_pod_is_an_interior_a_character_can_be_placed_into_and_a_plain_item_is_not() {
    let mut tw = lot_shelving(vec![
        shipped().first().cloned().expect("the region pack"),
        ooc_repainted(),
    ]);
    let alice = tw.spawn_player("Alice");
    participant(&mut tw, alice);

    let pod = named(&tw, ObjKind::Thing, "Red Pod 1");
    let board = named(&tw, ObjKind::Thing, "notice board");

    // The two facts `@spatial/place/interior` looks for, both made at apply.
    assert!(
        tw.world()
            .has_behavior(pod, &BehaviorId::new("sim.interior").expect("id")),
        "the pod carries sim.interior"
    );
    assert!(
        state(&tw).spaces.contains_key(&pod),
        "the pod has an interior-space record"
    );
    assert!(!state(&tw).spaces.contains_key(&board));

    let said = say(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", alice.0, pod.0),
    );
    assert!(said.contains("Placed Alice"), "{said}");
    assert_eq!(
        tw.world().get(alice).and_then(|o| o.location),
        Some(pod),
        "the pilot stands inside the pod"
    );
    assert!(
        matches!(state(&tw).parent_of(alice), SpatialParent::Interior { space, .. } if space == pod)
    );

    // A plain item of the same pack is not a place: same command, one refusal.
    let said = say(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", alice.0, board.0),
    );
    assert!(said.contains("is not an interior space"), "{said}");
    assert_eq!(
        tw.world().get(alice).and_then(|o| o.location),
        Some(pod),
        "the refusal left the pilot where they were"
    );

    // All eight pods, and re-applying the pack disturbs none of it.
    for (_, name) in PODS {
        let pod = named(&tw, ObjKind::Thing, name);
        assert!(state(&tw).spaces.contains_key(&pod), "{name} is a space");
    }
    // Idempotence: the pack revises the pods (`ooc_repainted` repaints them), the
    // applier meets objects that are already interior spaces, and nothing about
    // the space, its record or its occupant moves.
    let before = census(&tw);
    let (planned, applied) = replan(&mut tw, starter::OOC_AREA_PACK);
    assert!(
        planned.contains("0 create, 8 update, 0 retire"),
        "the eight pods are updates, so the applier really meets them again:\n{planned}"
    );
    assert!(
        applied.contains("Applied neighborhood.ooc_area"),
        "{applied}"
    );
    assert!(!applied.contains("STOPPED"), "{applied}");
    assert_eq!(census(&tw), before, "the revision moved nothing");
    for (_, name) in PODS {
        let pod = named(&tw, ObjKind::Thing, name);
        assert!(
            state(&tw).spaces.contains_key(&pod),
            "{name} is still a space"
        );
        assert!(
            tw.world()
                .has_behavior(pod, &BehaviorId::new("sim.interior").expect("id")),
            "{name} still carries sim.interior"
        );
    }
    assert_eq!(
        tw.world().get(alice).and_then(|o| o.location),
        Some(pod),
        "the pilot rode the revision"
    );
    assert!(
        matches!(state(&tw).parent_of(alice), SpatialParent::Interior { space, .. } if space == pod)
    );
}

// ---- C2: the pack from version 3 to the shipped one --------------------------

#[test]
fn the_shipped_version_lands_whole_on_a_fresh_world() {
    let tw = lot();
    for room in ["Simulator Lobby", "Red Room", "Blue Room"] {
        named(&tw, ObjKind::Room, room);
    }
    let lobby = named(&tw, ObjKind::Room, "Simulator Lobby");
    let red = named(&tw, ObjKind::Room, "Red Room");
    let blue = named(&tw, ObjKind::Room, "Blue Room");
    let hall = named(&tw, ObjKind::Room, "Welcome Hall");

    // The exits, both ways.
    let leads = |from: ObjId, to: ObjId| {
        tw.world()
            .exits_of(from)
            .iter()
            .filter_map(|e| tw.world().get(*e))
            .any(|e| e.destination == Some(to))
    };
    assert!(leads(hall, lobby) && leads(lobby, hall), "hall <-> lobby");
    assert!(leads(lobby, red) && leads(red, lobby), "lobby <-> red");
    assert!(leads(lobby, blue) && leads(blue, lobby), "lobby <-> blue");

    // The selector in the lobby; four pods a side, every one an interior space.
    assert_eq!(
        tw.world()
            .get(named(&tw, ObjKind::Thing, "Simulator Selector"))
            .and_then(|o| o.location),
        Some(lobby)
    );
    for (_, name) in PODS {
        let pod = named(&tw, ObjKind::Thing, name);
        let room = if name.starts_with("Red") { red } else { blue };
        assert_eq!(
            tw.world().get(pod).and_then(|o| o.location),
            Some(room),
            "{name} stands in its team room"
        );
        assert!(state(&tw).spaces.contains_key(&pod), "{name} is a space");
    }

    let content = tw.world().module_state::<ContentState>().expect("content");
    let latch = content
        .latches
        .applied(&cid(starter::OOC_AREA_PACK))
        .expect("latched");
    assert_eq!(latch.anomaly(), None, "the pack applied whole");
    for id in ADDED {
        assert!(latch.entries().contains_key(&cid(id)), "{id} is latched");
    }
}

#[test]
fn the_shipped_version_upgrades_a_world_at_version_three_and_moves_nothing() {
    // Region and the OOC area at version 3 only: the staff lobby requires
    // `ooc_area@4`, so it has no place on a world still at 3.
    let region = starter::SOURCES
        .first()
        .map(|(_, j)| (*j).to_string())
        .expect("the region pack");
    let mut tw = TestWorld::with_modules(modules(vec![region, ooc_v3()], shipped()));

    assert!(find(&tw, ObjKind::Room, "Simulator Lobby").is_none());
    assert!(find(&tw, ObjKind::Thing, "Red Pod 1").is_none());
    let hall = named(&tw, ObjKind::Room, "Welcome Hall");
    let content = tw.world().module_state::<ContentState>().expect("content");
    assert_eq!(
        content
            .latches
            .applied(&cid(starter::OOC_AREA_PACK))
            .map(mush_content::Applied::version),
        Some(3)
    );

    // Somebody is standing in the hall, and a prop has been moved into it.
    let alice = tw.spawn_player("Alice");
    tw.world_mut().move_to(alice, hall).expect("Alice walks in");
    let pamphlet = named(&tw, ObjKind::Thing, "tutorial pamphlet");
    let before = census(&tw);

    let (planned, applied) = replan(&mut tw, starter::OOC_AREA_PACK);
    assert!(
        planned.contains("15 create, 1 update, 0 retire"),
        "the upgrade creates the simulator centre, updates only the neighborhood          (its raised room budget), and retires nothing:
{planned}"
    );
    assert!(
        applied.contains("Applied neighborhood.ooc_area"),
        "{applied}"
    );
    assert!(!applied.contains("STOPPED"), "{applied}");

    // The three rooms, their six exit objects, the selector and the eight pods.
    let lobby = named(&tw, ObjKind::Room, "Simulator Lobby");
    let red = named(&tw, ObjKind::Room, "Red Room");
    let blue = named(&tw, ObjKind::Room, "Blue Room");
    named(&tw, ObjKind::Thing, "Simulator Selector");
    for (_, name) in PODS {
        let pod = named(&tw, ObjKind::Thing, name);
        assert!(
            state(&tw).spaces.contains_key(&pod),
            "{name} became an interior space on the upgrade"
        );
    }
    let after = census(&tw);
    let fresh: Vec<&str> = after
        .iter()
        .filter(|(id, _)| !before.contains_key(id))
        .map(|(_, (_, name, _))| name.as_str())
        .collect();
    assert_eq!(fresh.len(), 18, "3 rooms, 6 exits, 9 items: {fresh:?}");

    // Nothing that was already standing moved, was renamed, or was replaced.
    for (id, was) in &before {
        assert_eq!(after.get(id), Some(was), "object #{id} moved or changed");
    }
    assert_eq!(tw.world().get(alice).and_then(|o| o.location), Some(hall));
    assert_eq!(
        tw.world().get(pamphlet).and_then(|o| o.location),
        Some(hall),
        "the pamphlet kept its id and its room"
    );

    let content = tw.world().module_state::<ContentState>().expect("content");
    let latch = content
        .latches
        .applied(&cid(starter::OOC_AREA_PACK))
        .expect("latched");
    assert_eq!(latch.version(), SHIPPED_VERSION);
    assert_eq!(latch.anomaly(), None);

    // And the atlas's own records grew by exactly the three rooms.
    let atlas = tw.world().module_state::<AtlasState>().expect("atlas");
    for room in [lobby, red, blue] {
        assert!(atlas.records.roomgeo.contains_key(&room));
    }
}

// ---- F.4: the pack from version 4 onwards ------------------------------------

/// The strings the shipped pack rewrote after version 4, as version 4 carried
/// them: `(shipped, version 4)`. Ten pairs, because the pod description moved in
/// two places and each pair has to be a substring `replace` can find on its own.
///
/// Built from the shipped bytes by putting the old text back, for the same
/// reason [`ooc_v3`] is: version 4's file is not in the tree any more, and a
/// fixture that is a copy drifts. Every string a later version touched belongs
/// here: one it omits is one the upgrade test silently stops proving, because
/// the "version 4" world it starts from already carries the corrected text.
///
/// **One set is deliberately missing, and cannot be added** (16b cut D.5, 16c
/// `B-11`): version 6 took the segment `l` off the three `Lounge <L>` exit
/// names, and the same cut taught pack apply to refuse an exit whose name
/// carries a word the kernel owns. A fixture pack spelling
/// `Lounge <L>;l;lounge;west;w` is refused before its first write, so the world
/// it would build never boots and the pre-D.5 shape of those three entries is
/// not constructible through apply at all. What that leaves untested is the
/// five exits' own remake; the applier path it runs down is the one the three
/// `sim` exits below take on this very upgrade, and
/// [`the_shipped_version_corrects_the_centres_texts_on_a_world_at_version_four`]
/// also puts an alias on a standing exit object by hand and proves the remake
/// drops it. The five exits' own fresh-boot shape is pinned in
/// `tests/lot.rs`.
const V4_TEXTS: [(&str, &str); 10] = [
    (
        "above the door. Once a match is boarding, the pilot holding this slot climbs in with BOARD and the panel wakes; EXIT gets you out again.",
        "above the door. The hatch stands open. Boarding is not built yet; when it is, a pilot who holds a slot on this side climbs in here.",
    ),
    (
        "one hatch, one seat, a panel, and",
        "one hatch, one seat, a blank panel, and",
    ),
    (
        "WELCOME TO THE LOT. Tutorial: east. Lounge: south. Simulator: 'simulator', down the east corridor. The bus: through the lounge and out. Type 'help getting-started' any time, or 'help simulator' to fly.",
        "WELCOME TO THE LOT. Tutorial: east. Lounge: south. Simulator: SIM, down the east corridor. The bus: through the lounge and out. Type 'help getting-started' any time.",
    ),
    (
        "The size of a place mat. Dark until the match starts, and then it is the tank: STATUS, VIEW, CONTACTS, and the orders that answer them.",
        "Dark, and the size of a place mat. It wakes for a pilot, and for nobody else.",
    ),
    (
        "A console bolted to the lobby counter: two lit buttons, RED and BLUE, and a small screen. 'SIM TEAM red' or 'SIM TEAM blue' claims a slot on that side, 'SIM STATUS' says who holds one, and 'SIM START' opens the pods. 'help simulator' is the rest of it.",
        "A console bolted to the lobby counter: two lit buttons, RED and BLUE, and a small screen reading STANDING BY. When the simulator opens, 'SIM TEAM red' or 'SIM TEAM blue' will claim a slot on that side and 'SIM STATUS' will say who holds one. The verbs are not built yet, so for now the screen is the whole of it.",
    ),
    (
        "SIM TEAM red|blue claims a slot. SIM LEAVE gives one back. SIM START opens the pods; SIM ABORT calls the match off; SIM CONCEDE hands it to the other side. SIM STATUS is this screen, in words.",
        "STANDING BY -- no match scheduled. SIM TEAM red|blue claims a slot when the verbs land; SIM LEAVE gives one back.",
    ),
    (
        "Both lit. Pressing one does nothing but click: the sides are taken with SIM TEAM, not with a thumb.",
        "Both lit, neither wired. Pressing one does nothing but click.",
    ),
    (
        "Simulator Lobby <SIMULATOR>;simulator;annexe",
        "Simulator Lobby <SIM>;sim;simulator",
    ),
    (
        "Simulator Lobby <LOBBY>;lobby;out;south;s",
        "Simulator Lobby <SIM>;sim;lobby;south;s",
    ),
    (
        "Simulator Lobby <LOBBY>;lobby;out;north;n",
        "Simulator Lobby <SIM>;sim;lobby;north;n",
    ),
];

/// **Version 4 of the OOC pack**: the shipped bytes with the eleven strings cut
/// F.4 rewrote put back, the version set to 4 and the digest re-derived.
///
/// The version is the literal 4 and not `SHIPPED_VERSION - 1`: what this fixture
/// carries is version 4's *text*, and the shipped version has moved past 5 (16b
/// cut D.5). The upgrade under test is therefore 4 to the shipped version, which
/// is what a world left standing since cut C.1 would really take.
fn ooc_v4() -> String {
    let mut text = starter::OOC_AREA_JSON.to_string();
    for (shipped, old) in V4_TEXTS {
        assert!(
            text.contains(shipped),
            "the shipped pack no longer carries {shipped:?}; this fixture is stale"
        );
        text = text.replace(shipped, old);
    }
    let mut doc: Value = serde_json::from_str(&text).expect("the pack is json");
    doc["manifest"]["version"] = Value::from(4);
    doc["manifest"]["digest"] = Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = parse_pack(&unsigned)
        .expect("version 4 parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = Value::String(digest);
    serde_json::to_string(&doc).expect("json")
}

/// The exit object whose `destination` is `to`, out of `from`'s exits.
fn exit_from(tw: &TestWorld, from: ObjId, to: ObjId) -> ObjId {
    tw.world()
        .exits_of(from)
        .iter()
        .filter_map(|e| tw.world().get(*e))
        .find(|e| e.destination == Some(to))
        .unwrap_or_else(|| panic!("no exit from #{} to #{}", from.0, to.0))
        .id
}

/// Whether an exit's name carries `alias` as one of its `;`-separated segments —
/// which is exactly what `mush_core::commands::movement` matches a typed line
/// against.
fn has_alias(tw: &TestWorld, exit: ObjId, alias: &str) -> bool {
    tw.world().get(exit).is_some_and(|o| {
        o.name
            .split(';')
            .map(str::trim)
            .any(|seg| seg.eq_ignore_ascii_case(alias))
            || o.aliases.iter().any(|a| a.eq_ignore_ascii_case(alias))
    })
}

/// The description one object carries, as `look` would read it.
fn desc_of(tw: &TestWorld, obj: ObjId) -> String {
    tw.world()
        .attr(obj, "desc")
        .and_then(|a| a.as_text().map(str::to_owned))
        .unwrap_or_default()
}

/// One of a pack entry's `details`, under the namespace the applier writes them
/// in (`atlas:detail:<keyword>`). A room's detail is the one kind of string this
/// upgrade rewrites that is not a `desc`.
fn detail_of(tw: &TestWorld, obj: ObjId, keyword: &str) -> String {
    tw.world()
        .attr(obj, &format!("atlas:detail:{keyword}"))
        .and_then(|a| a.as_text().map(str::to_owned))
        .unwrap_or_default()
}

#[test]
fn the_fixtures_track_the_shipped_version() {
    assert_eq!(
        shipped_version(),
        SHIPPED_VERSION,
        "the OOC pack's version moved; the version-3 and version-4 fixtures in this \
         file are built relative to it, so read them before re-pinning"
    );
}

/// **The upgrade off version 4 rewrites what is standing and creates nothing**
/// (16b cut F.4; still true at cut D.5's version 6, which only edits strings).
#[test]
fn the_shipped_version_corrects_the_centres_texts_on_a_world_at_version_four() {
    let region = starter::SOURCES
        .first()
        .map(|(_, j)| (*j).to_string())
        .expect("the region pack");
    let mut tw = TestWorld::with_modules(modules(vec![region, ooc_v4()], shipped()));

    // The world at version 4: the centre is here, and it tells the players it is
    // not. This is what the readiness review found in the first minute.
    let hall = named(&tw, ObjKind::Room, "Welcome Hall");
    let lobby = named(&tw, ObjKind::Room, "Simulator Lobby");
    let red = named(&tw, ObjKind::Room, "Red Room");
    let pod = named(&tw, ObjKind::Thing, "Red Pod 1");
    assert!(
        desc_of(&tw, pod).contains("not built yet"),
        "the version-4 fixture must carry the stale text, or this proves nothing"
    );
    // The hall's sign is the room-`detail:` half, and it is the K5 text: it
    // sends the player at the one word the `SIM` verb shadows.
    assert!(
        detail_of(&tw, hall, "sign").contains("Simulator: SIM,"),
        "the version-4 fixture must carry the stale sign, or this proves nothing"
    );
    // The three exits that carried `sim`: into the lobby from the hall, and back
    // into the lobby from each team room.
    let blue = named(&tw, ObjKind::Room, "Blue Room");
    for (from, to) in [(hall, lobby), (red, lobby), (blue, lobby)] {
        let exit = exit_from(&tw, from, to);
        assert!(
            has_alias(&tw, exit, "sim"),
            "version 4 shadowed the verb on this exit"
        );
    }

    // 16b cut D.5 (16c `B-11`): a standing exit can carry an alias that is in no
    // version of the pack — version 6 takes `l` off this pack's three
    // `Lounge <L>` exits, and a fixture cannot author that shape any more, since
    // apply now refuses a pack that spells it (see [`V4_TEXTS`]). So the
    // alias goes on the object instead, on an exit this upgrade does remake. An
    // exit entry is remade rather than edited, and this is the assertion that
    // says the remake carries nothing off the object that stood there — not
    // merely nothing the new text omits.
    let hall_to_lobby = exit_from(&tw, hall, lobby);
    tw.world_mut()
        .add_alias(hall_to_lobby, "l")
        .expect("a standing exit takes the alias");
    assert!(
        has_alias(&tw, hall_to_lobby, "l"),
        "the standing exit must carry the stale alias, or this proves nothing"
    );

    // Somebody is standing in the lobby, and the census is taken with them in it.
    let alice = tw.spawn_player("Alice");
    tw.world_mut()
        .move_to(alice, lobby)
        .expect("Alice walks in");
    let before = census(&tw);

    let (planned, applied) = replan(&mut tw, starter::OOC_AREA_PACK);
    assert!(
        planned.contains("0 create,"),
        "the upgrade off version 4 adds no entry; it rewrites what is here:\n{planned}"
    );
    assert!(planned.contains("0 retire"), "{planned}");
    assert!(
        applied.contains("Applied neighborhood.ooc_area"),
        "{applied}"
    );
    assert!(!applied.contains("STOPPED"), "{applied}");

    // Every one of the eight pods now says what boarding really is, and none of
    // them says the feature is missing.
    for (_, name) in PODS {
        let pod = named(&tw, ObjKind::Thing, name);
        let desc = desc_of(&tw, pod);
        assert!(!desc.contains("not built"), "{name}: {desc}");
        assert!(desc.contains("BOARD"), "{name}: {desc}");
        // A pilot inside a running match is located in the pod, so `look` reads
        // this string with the hatch shut and the panel drawing contacts.
        // Neither clause may say otherwise.
        assert!(!desc.contains("blank panel"), "{name}: {desc}");
        assert!(!desc.contains("hatch stands open"), "{name}: {desc}");
    }
    let selector = desc_of(&tw, named(&tw, ObjKind::Thing, "Simulator Selector"));
    assert!(!selector.contains("not built"), "{selector}");
    assert!(selector.contains("SIM START"), "{selector}");

    // The room detail moved too, in place, on a room that kept its id.
    let sign = detail_of(&tw, hall, "sign");
    assert!(!sign.contains("Simulator: SIM,"), "the stale sign: {sign}");
    assert!(sign.contains("'simulator'"), "{sign}");

    // And `sim` is off all three exits, so the verb of that name can be typed in
    // the hall and in both team rooms. The walking words are still there. The
    // exits are re-resolved because an `exit` entry is remade, not edited.
    let to_lobby = exit_from(&tw, hall, lobby);
    let to_red = exit_from(&tw, lobby, red);
    let back_to_lobby = exit_from(&tw, red, lobby);
    for exit in [to_lobby, to_red, back_to_lobby] {
        assert!(!has_alias(&tw, exit, "sim"), "an exit still eats `sim`");
    }
    // And the hand-placed alias is gone with the object that carried it: a
    // remade exit starts from the entry's names and nothing else.
    assert_ne!(
        to_lobby, hall_to_lobby,
        "the exit was edited in place, not remade"
    );
    assert!(
        !has_alias(&tw, to_lobby, "l"),
        "a remade exit kept an alias off the object that stood there, so a world \
         standing at version 5 would keep `l` and go on eating `look`"
    );
    assert!(has_alias(&tw, to_lobby, "simulator"));
    assert!(has_alias(&tw, back_to_lobby, "lobby"));
    assert!(has_alias(&tw, back_to_lobby, "out"));
    assert!(has_alias(&tw, to_red, "red"));

    // Nothing but the three exit entries changed identity. An `exit` entry is
    // **remade** on an update (`AtlasApplier::update`: "their live state is
    // nothing"), so its six objects get new ids; every room, item and character
    // keeps its id, its name and where it is standing.
    let after = census(&tw);
    let is_exit = |row: &(ObjKind, String, Option<ObjId>)| row.0 == ObjKind::Exit;
    for (id, was) in &before {
        if is_exit(was) {
            continue;
        }
        assert_eq!(
            after.get(id),
            Some(was),
            "object #{id} moved or was renamed"
        );
    }
    let exits_before = before.values().filter(|row| is_exit(row)).count();
    let exits_after = after.values().filter(|row| is_exit(row)).count();
    assert_eq!(exits_after, exits_before, "an exit was gained or lost");
    assert_eq!(
        after.values().filter(|row| !is_exit(row)).count(),
        before.values().filter(|row| !is_exit(row)).count(),
        "the upgrade created or retired a room, an item or a character"
    );
    assert_eq!(tw.world().get(alice).and_then(|o| o.location), Some(lobby));

    let content = tw.world().module_state::<ContentState>().expect("content");
    let latch = content
        .latches
        .applied(&cid(starter::OOC_AREA_PACK))
        .expect("latched");
    assert_eq!(latch.version(), SHIPPED_VERSION);
    assert_eq!(latch.anomaly(), None);
}

/// A fresh world lands the shipped texts whole, with no staff command at all.
#[test]
fn a_fresh_world_gets_the_corrected_texts_at_first_boot() {
    let tw = lot();
    for (_, name) in PODS {
        let desc = desc_of(&tw, named(&tw, ObjKind::Thing, name));
        assert!(!desc.contains("not built"), "{name}: {desc}");
        assert!(!desc.contains("blank panel"), "{name}: {desc}");
        assert!(!desc.contains("hatch stands open"), "{name}: {desc}");
    }
    let hall = named(&tw, ObjKind::Room, "Welcome Hall");
    assert!(
        !detail_of(&tw, hall, "sign").contains("Simulator: SIM,"),
        "a fresh world's sign sends the player at the shadowed word"
    );
    let lobby = named(&tw, ObjKind::Room, "Simulator Lobby");
    let red = named(&tw, ObjKind::Room, "Red Room");
    let blue = named(&tw, ObjKind::Room, "Blue Room");
    for (from, to) in [(hall, lobby), (red, lobby), (blue, lobby)] {
        let exit = exit_from(&tw, from, to);
        assert!(
            !has_alias(&tw, exit, "sim"),
            "a fresh world's exit eats `sim`"
        );
    }
    let content = tw.world().module_state::<ContentState>().expect("content");
    assert_eq!(
        content
            .latches
            .applied(&cid(starter::OOC_AREA_PACK))
            .map(mush_content::Applied::version),
        Some(SHIPPED_VERSION)
    );
}
