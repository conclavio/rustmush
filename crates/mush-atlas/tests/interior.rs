//! **C.1 (16b §8 cut C; doc 16 §3.7, §4)** — the simulator centre in the OOC
//! pack: items that are interior spaces, and the pack's own step from version 3
//! to version 4.
//!
//! An `item` entry carrying `interior: true` is applied as a Thing a character
//! can be placed *inside* — the two facts `@spatial/place/interior` looks for,
//! the `sim.interior` behaviour and an interior-space record — which is what a
//! pod is. A plain item is not one, and says so.
//!
//! The upgrade half drives the pack's own latch: a world already at
//! `ooc_area@3` takes version 4 as an update that adds the simulator lobby, the
//! red and blue rooms, their exits, the selector and the eight pods and moves
//! nothing that was already standing; a fresh world lands all of it at once.

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

/// The fifteen entries version 4 adds to version 3: three rooms, three exit
/// rows (six exit objects) and nine items.
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

fn cid(s: &str) -> CanonicalId {
    CanonicalId::new(s).expect("a canonical id")
}

/// The lot's production shape plus the simulation, which is what holds an
/// interior-space record. The shelf is always the shipped (version 4) packs:
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

/// A world with the whole shipped starter area, version 4, shelving `shelf`.
fn lot_shelving(shelf: Vec<String>) -> TestWorld {
    TestWorld::with_modules(modules(shipped(), shelf))
}

/// A world with the whole shipped starter area, version 4.
fn lot() -> TestWorld {
    lot_shelving(shipped())
}

/// **Version 5 of the OOC pack**: nothing but the eight pods repainted, so the
/// latch plans eight *updates* and `create_item` meets a pod that already
/// exists, is already an interior space, and already holds a pilot.
fn ooc_repainted() -> String {
    let mut doc: Value = serde_json::from_str(starter::OOC_AREA_JSON).expect("the pack is json");
    doc["manifest"]["version"] = Value::from(5);
    for (id, _) in PODS {
        doc["entries"][id]["def"]["desc"] =
            Value::String("Repainted, and otherwise the same shell.".to_string());
    }
    doc["manifest"]["digest"] = Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = parse_pack(&unsigned)
        .expect("version 5 parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = Value::String(digest);
    serde_json::to_string(&doc).expect("json")
}

/// **Version 3 of the OOC pack**, rebuilt from the shipped version 4 by taking
/// the simulator centre back out: every entry [`ADDED`] names, the room budget
/// back to 7, the version back to 3, the digest re-derived. (Version 3's bytes
/// are not in the tree any more; this is the world an upgrade meets, built from
/// the one source of truth so it cannot drift. The Welcome Hall's own text
/// therefore matches version 4's, which is why the upgrade below plans one
/// update and not two.)
fn ooc_v3() -> String {
    let mut doc: Value = serde_json::from_str(starter::OOC_AREA_JSON).expect("the pack is json");
    doc["manifest"]["version"] = Value::from(3);
    if let Some(entries) = doc["entries"].as_object_mut() {
        for id in ADDED {
            assert!(entries.remove(id).is_some(), "{id} is in version 4");
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
    // Idempotence: the pack revises the pods (version 5 repaints them), the
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

// ---- C2: the pack from version 3 to version 4 --------------------------------

#[test]
fn version_four_lands_whole_on_a_fresh_world() {
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
fn version_four_upgrades_a_world_at_version_three_and_moves_nothing() {
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
    assert_eq!(latch.version(), 4);
    assert_eq!(latch.anomaly(), None);

    // And the atlas's own records grew by exactly the three rooms.
    let atlas = tw.world().module_state::<AtlasState>().expect("atlas");
    for room in [lobby, red, blue] {
        assert!(atlas.records.roomgeo.contains_key(&room));
    }
}
