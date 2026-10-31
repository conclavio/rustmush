//! S2's travel system (14-spatial-implementation-plan §3.2): never-skip under every
//! clock path (lens N), the clock arithmetic across rates, floors, caps and
//! generations (C), one location per traveler under carrier motion (S), and what a
//! rendering may reveal (K).

#![allow(clippy::too_many_lines)]

use std::collections::HashMap;

use mush_atlas::{AtlasModule, AtlasState, JourneyState, Node, starter};
use mush_content::{Appliers, ContentModule, parse_pack};
use mush_core::clock::ClockRef;
use mush_core::{
    Audience, DetachedTypedState, GOD, GameModule, ModuleBootData, ModuleWrite, ObjId, ObjKind,
    Outbox, TypedStateLens,
};
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use serde_json::{Value, json};
use smol_str::SmolStr;

const T0: i64 = 1_000;

/// The store-less lens these engine-side calls are taken with: this suite holds
/// no live `ActionStateStore`, and WS03-S7 made a call say so rather than let
/// "no typed state" be the shorter spelling.
const DETACHED: TypedStateLens<'static> = TypedStateLens::Detached(DetachedTypedState::Test);

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn signed(mut doc: Value) -> String {
    doc["manifest"]["digest"] = Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = parse_pack(&unsigned)
        .expect("parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = Value::String(digest);
    serde_json::to_string(&doc).expect("json")
}

/// A test region: two neighborhoods joined by a foot road with two checkpoints, a
/// hidden second road to a third neighborhood, and a timed farm road inside the west.
fn region_pack() -> String {
    signed(json!({
        "manifest": { "id": "region.test", "version": 1, "kind": "region", "digest": "", "requires": [] },
        "entries": {
            "region.test": { "kind": "region", "def": {
                "name": "Testland", "kind": "surface", "body": "earth",
                "extent": [[0, 0], [400, 400]], "datum_m": 0.0,
                "background": { "surface": "grass", "mix": [["paved", 4]], "relief_cm": 0,
                                "relief_period_cm": 800, "seed": 7 },
                "ground_resolution_cm": 1000 } },
            "locality.test": { "kind": "locality", "def": {
                "region": "region.test", "name": "The Test Valley", "kind": "district",
                "footprint": [[0, 0], [400, 0], [400, 400], [0, 400]] } },
            "checkpoint.mid1": { "kind": "checkpoint", "def": {
                "region": "region.test", "name": "First Milestone", "point": [100, 100, 0] } },
            "checkpoint.mid2": { "kind": "checkpoint", "def": {
                "region": "region.test", "name": "Second Milestone", "point": [200, 200, 0] } },
            "checkpoint.ford": { "kind": "checkpoint", "def": {
                "region": "region.test", "name": "The Ford", "point": [60, 300, 0] } },
            "connection.road": { "kind": "connection", "def": {
                "region": "region.test", "name": "Valley Road", "kind": "journey",
                "from": { "neighborhood": "neighborhood.west", "point": [30, 30, 0] },
                "to": { "neighborhood": "neighborhood.east", "point": [320, 320, 0] },
                "direction": "both", "modes": ["foot", "car"],
                "corridor": [[30, 30, 0], [100, 100, 0], [200, 200, 0], [320, 320, 0]],
                "legs": [
                    { "to": "checkpoint.mid1", "base_s": 60 },
                    { "to": "checkpoint.mid2", "base_s": 60 },
                    { "to": "connection.road", "base_s": 60 } ],
                "visibility": "public", "status": "active" } },
            "connection.long": { "kind": "connection", "def": {
                "region": "region.test", "name": "Long Drive", "kind": "journey",
                "from": { "neighborhood": "neighborhood.west", "point": [30, 30, 0] },
                "to": { "neighborhood": "neighborhood.east", "point": [320, 320, 0] },
                "direction": "both", "modes": ["car"],
                "corridor": [[30, 30, 0], [320, 320, 0]],
                "legs": [
                    { "to": "checkpoint.mid1", "base_s": 3600 },
                    { "to": "checkpoint.mid2", "base_s": 3600 },
                    { "to": "connection.long", "base_s": 7200 } ],
                "visibility": "public", "status": "active" } },
            "connection.secret": { "kind": "connection", "def": {
                "region": "region.test", "name": "Smugglers' Path", "kind": "journey",
                "from": { "neighborhood": "neighborhood.west", "point": [30, 30, 0] },
                "to": { "neighborhood": "neighborhood.hollow", "point": [60, 340, 0] },
                "direction": "both", "modes": ["foot"],
                "corridor": [[30, 30, 0], [60, 300, 0], [60, 340, 0]],
                "legs": [
                    { "to": "checkpoint.ford", "base_s": 40 },
                    { "to": "connection.secret", "base_s": 40 } ],
                "visibility": "hidden", "status": "active" } }
        }
    }))
}

fn neighborhood(id: &str, name: &str, footprint: Value, rooms: Value) -> String {
    let mut entries = serde_json::Map::new();
    entries.insert(
        "neighborhood".into(),
        json!({ "kind": "neighborhood", "def": {
            "region": "region.test", "locality": "locality.test", "name": name,
            "setting": "rural", "footprint": footprint,
            "room_budget": { "target": 2, "maximum": 8 } } }),
    );
    if let Value::Object(map) = rooms {
        entries.extend(map);
    }
    signed(json!({
        "manifest": { "id": id, "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": "region.test", "version": 1 }] },
        "entries": Value::Object(entries)
    }))
}

fn west() -> String {
    neighborhood(
        "neighborhood.west",
        "West End",
        json!([[0, 0], [80, 0], [80, 120], [0, 120]]),
        json!({
            "room.gate": { "kind": "room", "def": { "name": "West Gate", "surface": "ground",
                "footprint": { "rect": [[10, 10], [50, 50]] }, "desc": "The road east starts here." } },
            "room.farm": { "kind": "room", "def": { "name": "Farmhouse", "surface": "ground",
                "footprint": { "rect": [[10, 80], [50, 110]] }, "desc": "A farmhouse at the end of its road." } },
            "exit.farm_road": { "kind": "exit", "def": { "from": "room.gate", "to": "room.farm",
                "names": ["Farm Road <FARM>;farm;north", "Back to the Gate <GATE>;gate;south"],
                "portal": { "distance_m": 420, "timed": ["foot"] } } },
            "room.shed": { "kind": "room", "def": { "name": "Shed", "surface": "ground",
                "footprint": { "rect": [[50, 10], [70, 50]] } } },
            "exit.gate_shed": { "kind": "exit", "def": { "from": "room.gate", "to": "room.shed",
                "names": ["Shed <SHED>;shed;east", "Gate <GATE>;gate;west"] } }
        }),
    )
}

fn east() -> String {
    neighborhood(
        "neighborhood.east",
        "East End",
        json!([[280, 280], [400, 280], [400, 400], [280, 400]]),
        json!({
            "room.gate": { "kind": "room", "def": { "name": "East Gate", "surface": "ground",
                "footprint": { "rect": [[300, 300], [340, 340]] }, "desc": "The road west starts here." } }
        }),
    )
}

fn hollow() -> String {
    neighborhood(
        "neighborhood.hollow",
        "The Hollow",
        json!([[40, 320], [100, 320], [100, 380], [40, 380]]),
        json!({
            "room.hollow": { "kind": "room", "def": { "name": "The Hollow", "surface": "ground",
                "footprint": { "rect": [[50, 330], [90, 370]] } } }
        }),
    )
}

fn modules(packs: Vec<String>) -> Vec<Box<dyn GameModule>> {
    modules_shelved(packs, Vec::new())
}

/// [`modules`], with `shelf` on the atlas's shelf for `@atlas/pack plan`/`apply`.
fn modules_shelved(packs: Vec<String>, shelf: Vec<String>) -> Vec<Box<dyn GameModule>> {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    let packs = packs
        .iter()
        .map(|p| parse_pack(p).expect("test pack parses"))
        .collect();
    vec![
        Box::new(mush_npc::NpcModule::new()),
        Box::new(
            AtlasModule::new().with_shelf(
                shelf
                    .iter()
                    .map(|p| parse_pack(p).expect("shelf pack parses"))
                    .collect(),
            ),
        ),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(packs),
        ),
    ]
}

/// The test valley, booted at `T0`, with the first tick fired.
fn valley() -> TestWorld {
    let mut tw = TestWorld::with_modules(modules(vec![region_pack(), west(), east(), hollow()]));
    tw.set_now(T0);
    let _ = tw.fire_due(T0 * 1000);
    tw
}

/// The starter lot, booted at `T0`, with the first tick fired (so the bus service
/// stands at the OOC stop).
fn lot() -> TestWorld {
    let packs: Vec<String> = starter::SOURCES
        .iter()
        .map(|(_, j)| (*j).to_string())
        .collect();
    let mut tw = TestWorld::with_modules(modules(packs));
    tw.set_now(T0);
    let _ = tw.fire_due(T0 * 1000);
    tw
}

fn room(tw: &TestWorld, name: &str) -> ObjId {
    tw.world()
        .iter()
        .find(|o| {
            o.kind == ObjKind::Room && o.name == name && !o.flags.contains(mush_core::Flags::GOING)
        })
        .unwrap_or_else(|| panic!("no room named {name}"))
        .id
}

fn player(tw: &mut TestWorld, name: &str, at: ObjId) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Player, name, GOD, Some(at))
        .expect("player")
}

fn thing(tw: &mut TestWorld, name: &str, at: ObjId) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Thing, name, GOD, Some(at))
        .expect("thing")
}

fn loc(tw: &TestWorld, who: ObjId) -> Option<ObjId> {
    tw.world().get(who).and_then(|o| o.location)
}

fn seen(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

fn run(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let events = tw.run(who, line);
    seen(tw, &events, who)
}

/// Advance the world to real second `t` and fire every due tick.
fn tick_at(tw: &mut TestWorld, t: i64) -> Vec<(Audience, OutputEvent)> {
    tw.set_now(t);
    tw.fire_due(t * 1000)
}

fn can_see(tw: &TestWorld, observer: ObjId, subject: ObjId) -> bool {
    mush_core::module::can_perceive(tw.registry(), tw.world(), observer, subject, DETACHED)
}

/// What the engine does on a clean last disconnect, module-side: the hook, then
/// the world event.
fn disconnect(tw: &mut TestWorld, who: ObjId) {
    let mut out = Outbox::new();
    let engine = tw.engine_mut();
    mush_core::module::fire_conn_hook(
        &engine.registry,
        &mut engine.world,
        &mut out,
        who,
        // 16b cut K.2: the dispatcher's `connected: bool` became a word, so that
        // linkdead onset and revival could be published too.
        mush_core::ConnTransition::Disconnected,
        DETACHED,
    );
    mush_core::module::emit_world_event(
        &engine.registry,
        &mut engine.world,
        &mut out,
        &mush_core::WorldEvent::Disconnected { who },
        DETACHED,
    );
}

fn atlas(tw: &TestWorld) -> &AtlasState {
    tw.world().module_state::<AtlasState>().expect("atlas")
}

fn journey_of(tw: &TestWorld, who: ObjId) -> Option<mush_atlas::JourneyRec> {
    atlas(tw).travel.journey_of(who).cloned()
}

fn transit_room(tw: &TestWorld) -> ObjId {
    room(tw, "Valley Road, under way")
}

fn set_rate(tw: &mut TestWorld, rate: u32) {
    let epoch = tw
        .world()
        .next_world_timeline_epoch(rate)
        .expect("a new world-IC rate epoch");
    tw.world_mut()
        .append_world_timeline_epoch(epoch)
        .expect("the epoch appends");
}

/// Fold the persisted ops into boot data and re-hydrate every module (a restart).
fn reboot(tw: &mut TestWorld) {
    let ops = tw.take_module_ops();
    let mut boot: HashMap<SmolStr, ModuleBootData> = HashMap::new();
    for op in ops {
        let data = boot.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                data.kv.retain(|(k, _)| *k != key);
                data.kv.push((key, value));
            }
            ModuleWrite::Del { key } => data.kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    tw.modules_loaded(boot);
}

// ---------------------------------------------------------------------------
// N — never skip
// ---------------------------------------------------------------------------

#[test]
fn n1_a_traveler_visits_every_checkpoint_in_order_at_or_after_its_due_time() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let east_gate = room(&tw, "East Gate");
    let alice = player(&mut tw, "Alice", west_gate);
    let listing = run(&mut tw, alice, "+travel");
    assert!(listing.contains("East End"), "{listing}");
    assert!(listing.contains("Valley Road"), "{listing}");
    let out = run(&mut tw, alice, "+travel East End");
    assert!(
        out.contains("You set off along the Valley Road by foot"),
        "{out}"
    );
    assert_eq!(loc(&tw, alice), Some(transit_room(&tw)));
    let j = journey_of(&tw, alice).expect("a journey");
    assert_eq!(j.legs_ic, vec![60, 60, 60], "rate 1, no floor or cap bites");
    assert_eq!(j.cursor, 0);

    // Second by second: the cursor moves exactly at each due stamp, never before.
    let mut seen_cursors = Vec::new();
    for t in (T0 + 1)..=(T0 + 181) {
        let _ = tick_at(&mut tw, t);
        let cursor = journey_of(&tw, alice).map_or(3, |j| j.cursor);
        if seen_cursors.last() != Some(&cursor) {
            seen_cursors.push(cursor);
            let expected_at = T0 + 60 * i64::from(cursor);
            assert!(
                t >= expected_at,
                "cursor {cursor} moved at {t}, before its due time {expected_at}"
            );
            assert!(t <= expected_at + 1, "cursor {cursor} moved late at {t}");
        }
    }
    assert_eq!(seen_cursors, vec![0, 1, 2, 3], "every checkpoint in order");
    assert_eq!(loc(&tw, alice), Some(east_gate), "arrived");
    assert!(
        journey_of(&tw, alice).is_none(),
        "the record is retired on arrival"
    );
}

#[test]
fn n2_a_catch_up_after_a_clock_jump_lands_in_one_move_with_no_narration() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let east_gate = room(&tw, "East Gate");
    let alice = player(&mut tw, "Alice", west_gate);
    let bob = player(&mut tw, "Bob", east_gate);
    let _ = run(&mut tw, alice, "+travel East End");
    // Downtime: the clock jumps past every boundary; one fire catches up.
    let events = tick_at(&mut tw, T0 + 600);
    assert_eq!(
        loc(&tw, alice),
        Some(east_gate),
        "one move to the right place"
    );
    let bob_saw = seen(&tw, &events, bob);
    assert!(
        bob_saw.is_empty(),
        "a stale arrival is silent to the room: {bob_saw}"
    );
    assert!(journey_of(&tw, alice).is_none());
}

#[test]
fn n3_a_watcher_at_a_checkpoint_sees_the_traveler_even_in_a_catch_up() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let alice = player(&mut tw, "Alice", west_gate);
    let watcher = player(&mut tw, "Watcher", west_gate);
    let out = run(&mut tw, watcher, "+stakeout Second Milestone");
    assert!(out.contains("You settle in at Second Milestone"), "{out}");
    let patch = room(&tw, "Second Milestone");
    assert_eq!(loc(&tw, watcher), Some(patch));
    let _ = run(&mut tw, alice, "+travel East End");
    // Jump past everything: the traveler is still stopped at the watched checkpoint.
    let events = tick_at(&mut tw, T0 + 600);
    assert_eq!(
        loc(&tw, alice),
        Some(patch),
        "held at the watched checkpoint"
    );
    let j = journey_of(&tw, alice).expect("held, not arrived");
    assert!(
        matches!(j.state, JourneyState::Held { .. }),
        "{:?}",
        j.state
    );
    assert_eq!(j.cursor, 2);
    let watcher_saw = seen(&tw, &events, watcher);
    assert!(
        watcher_saw.contains("Watcher stops Alice at Second Milestone"),
        "{watcher_saw}"
    );
    // Release: the schedule shifts by the hold, and the remaining leg runs whole.
    let _ = tick_at(&mut tw, T0 + 700);
    let out = run(&mut tw, watcher, "+stakeout/end");
    assert!(out.contains("You give up the watch"), "{out}");
    assert_eq!(
        loc(&tw, watcher),
        Some(west_gate),
        "the watcher walked back"
    );
    assert_eq!(loc(&tw, alice), Some(transit_room(&tw)), "back on the road");
    let j = journey_of(&tw, alice).expect("under way again");
    assert_eq!(j.state, JourneyState::UnderWay);
    let arrival = j.arrival_at().expect("arrival");
    assert_eq!(
        arrival,
        T0 + 700 + 60,
        "the last leg runs whole after the hold"
    );
    let _ = tick_at(&mut tw, T0 + 759);
    assert!(journey_of(&tw, alice).is_some(), "not yet");
    let _ = tick_at(&mut tw, T0 + 760);
    assert_eq!(loc(&tw, alice), Some(room(&tw, "East Gate")));
    // The patch is gone once nobody watches, nobody is held and nothing stands there.
    assert!(
        tw.world()
            .iter()
            .all(|o| o.name != "Second Milestone" || o.flags.contains(mush_core::Flags::GOING)),
        "the idle patch was retired"
    );
}

// ---------------------------------------------------------------------------
// C — clocks
// ---------------------------------------------------------------------------

#[test]
fn c1_a_rate_of_four_arrives_sooner_but_never_inside_the_dwell_floor() {
    let mut tw = valley();
    set_rate(&mut tw, 4);
    let west_gate = room(&tw, "West Gate");
    let alice = player(&mut tw, "Alice", west_gate);
    let _ = run(&mut tw, alice, "+travel East End");
    let j = journey_of(&tw, alice).expect("journey");
    // 60 IC seconds is 15 real at rate 4; the floor lifts every checkpoint dwell to
    // 30 real = 120 IC. The first leg has no dwell before it.
    assert_eq!(j.legs_ic, vec![60, 120, 120], "{:?}", j.legs_ic);
    let real = (60 + 120 + 120) / 4;
    assert!(real < 180, "sooner than rate 1's 180 s");
    let _ = tick_at(&mut tw, T0 + real - 1);
    assert!(journey_of(&tw, alice).is_some());
    let _ = tick_at(&mut tw, T0 + real);
    assert_eq!(loc(&tw, alice), Some(room(&tw, "East Gate")));
}

#[test]
fn c2_the_cap_holds_a_four_hour_drive_to_real_cap_s() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let alice = player(&mut tw, "Alice", west_gate);
    let listing = run(&mut tw, alice, "+travel");
    assert!(listing.contains("Long Drive"), "{listing}");
    let _ = run(&mut tw, alice, "+travel East End/car");
    let j = journey_of(&tw, alice).expect("journey");
    let total: u32 = j.legs_ic.iter().sum();
    assert!(total <= 180, "capped: {:?}", j.legs_ic);
    assert!(
        j.legs_ic.iter().skip(1).all(|l| *l >= 30),
        "floor: {:?}",
        j.legs_ic
    );
    assert_eq!(j.mode, "car");
    let _ = tick_at(&mut tw, T0 + 181);
    assert_eq!(
        loc(&tw, alice),
        Some(room(&tw, "East Gate")),
        "there within the cap"
    );
}

#[test]
fn c3_a_rate_change_mid_journey_re_anchors_and_never_strands() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let alice = player(&mut tw, "Alice", west_gate);
    let _ = run(&mut tw, alice, "+travel East End");
    let before = journey_of(&tw, alice).expect("journey");
    assert_eq!(before.departed.clock, ClockRef::WorldIc { generation: 1 });
    let _ = tick_at(&mut tw, T0 + 30);
    set_rate(&mut tw, 2);
    let _ = tick_at(&mut tw, T0 + 31);
    let after = journey_of(&tw, alice).expect("journey");
    assert_eq!(
        after.departed.clock,
        ClockRef::WorldIc { generation: 2 },
        "relabelled"
    );
    assert_eq!(after.departed.at, before.departed.at, "not re-based");
    assert_eq!(after.arrival_at(), before.arrival_at(), "no boundary moved");
    assert_eq!(after.cursor, 0);
    // 150 IC seconds remain at 2 IC/s: 75 real seconds, so arrival at T0 + 30 + 75.
    let mut cursors = vec![0];
    for t in (T0 + 32)..=(T0 + 106) {
        let _ = tick_at(&mut tw, t);
        let c = journey_of(&tw, alice).map_or(3, |j| j.cursor);
        if cursors.last() != Some(&c) {
            cursors.push(c);
        }
    }
    assert_eq!(
        cursors,
        vec![0, 1, 2, 3],
        "every checkpoint, none skipped, none stranded"
    );
    assert_eq!(loc(&tw, alice), Some(room(&tw, "East Gate")));
}

// ---------------------------------------------------------------------------
// S — travelers, subtree integrity, holds
// ---------------------------------------------------------------------------

#[test]
fn s1_a_plain_object_traverses_the_road_and_a_destroyed_one_ends_its_journey() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let parcel = thing(&mut tw, "Parcel", west_gate);
    let out = run(
        &mut tw,
        GOD,
        &format!("@atlas/journey send #{}=East End", parcel.0),
    );
    assert!(out.contains("Parcel is on its way"), "{out}");
    assert_eq!(loc(&tw, parcel), Some(transit_room(&tw)));
    let _ = tick_at(&mut tw, T0 + 61);
    assert_eq!(journey_of(&tw, parcel).map(|j| j.cursor), Some(1));
    let listing = run(&mut tw, GOD, "@atlas/journey list");
    assert!(listing.contains("Parcel"), "{listing}");
    let _ = run(&mut tw, GOD, &format!("@destroy #{}", parcel.0));
    assert!(
        journey_of(&tw, parcel).is_none(),
        "a destroyed traveler ends the journey"
    );
    let listing = run(&mut tw, GOD, "@atlas/journey list");
    assert!(!listing.contains("Parcel"), "{listing}");
}

#[test]
fn s2_the_tour_bus_carries_three_players_and_a_prop_with_one_location_each() {
    let mut tw = lot();
    let bus = room(&tw, "Studio Tour Bus");
    let stop = room(&tw, "Bus Stop (OOC)");
    let forecourt = room(&tw, "Forecourt & Bus Stop");
    let riders: Vec<ObjId> = ["Cady", "Janis", "Damian"]
        .iter()
        .map(|n| player(&mut tw, n, stop))
        .collect();
    let prop = thing(&mut tw, "Mounted Camera", bus);
    for r in &riders {
        let out = run(&mut tw, *r, "board");
        assert!(!out.contains("is not here"), "{out}");
        assert_eq!(loc(&tw, *r), Some(bus), "boarded at the OOC stop");
    }
    // The forecourt door is closed while the bus is at the OOC stop.
    let out = run(&mut tw, riders[0], "lot");
    assert!(
        out.contains("is moving") || out.contains("wait for the next stop"),
        "{out}"
    );
    assert_eq!(loc(&tw, riders[0]), Some(bus));
    // The service departs after its dwell (90 s) and runs the loop; every rider and
    // the prop have exactly one location throughout: the bus.
    let mut departed_at = None;
    let mut arrived_at = None;
    for t in (T0 + 1)..=(T0 + 600) {
        let events = tick_at(&mut tw, t);
        let saw = seen(&tw, &events, riders[0]);
        if saw.contains("departing") && departed_at.is_none() {
            departed_at = Some(t);
        }
        if saw.contains("Mind the step") {
            arrived_at = Some(t);
            break;
        }
        for r in &riders {
            assert_eq!(loc(&tw, *r), Some(bus), "rider left the bus at {t}");
        }
        assert_eq!(loc(&tw, prop), Some(bus));
    }
    let departed = departed_at.expect("the bus departed");
    let arrived = arrived_at.expect("the bus arrived");
    assert!(departed >= T0 + 90, "dwell first: {departed}");
    assert!(
        arrived - departed <= 181,
        "within the bus cap: {}",
        arrived - departed
    );
    // At the forecourt the far door opens and the near one is closed.
    let out = run(&mut tw, riders[0], "ooc");
    assert!(out.contains("is moving") || out.contains("wait"), "{out}");
    let out = run(&mut tw, riders[1], "lot");
    assert!(!out.contains("is moving"), "{out}");
    assert_eq!(loc(&tw, riders[1]), Some(forecourt), "off at the forecourt");
    assert_eq!(loc(&tw, riders[0]), Some(bus));
    assert_eq!(loc(&tw, prop), Some(bus));
    // A new passenger boards at the forecourt.
    let newcomer = player(&mut tw, "Newcomer", forecourt);
    let _ = run(&mut tw, newcomer, "board");
    assert_eq!(loc(&tw, newcomer), Some(bus));
}

#[test]
fn s3_a_passenger_boarding_at_a_held_checkpoint_rides_the_rest() {
    let mut tw = lot();
    let bus = room(&tw, "Studio Tour Bus");
    let forecourt = room(&tw, "Forecourt & Bus Stop");
    let watcher = player(&mut tw, "Watcher", forecourt);
    let out = run(&mut tw, watcher, "+stakeout Water Tower");
    assert!(out.contains("You settle in at Water Tower"), "{out}");
    let patch = room(&tw, "Water Tower");
    // Run until the bus is held at the tower.
    let mut held = false;
    for t in (T0 + 1)..=(T0 + 400) {
        let _ = tick_at(&mut tw, t);
        if journey_of(&tw, bus).is_some_and(|j| matches!(j.state, JourneyState::Held { .. })) {
            held = true;
            break;
        }
    }
    assert!(held, "the bus was held at the watched tower");
    // The doors open onto the patch: the watcher boards.
    let out = run(&mut tw, watcher, "board");
    assert!(!out.contains("is not here"), "{out}");
    assert_eq!(loc(&tw, watcher), Some(bus));
    let now = tw.world().now();
    // Their watch ends (they are no longer in the patch), the bus is released and
    // arrives with them aboard.
    let mut arrived = false;
    for t in (now + 1)..=(now + 400) {
        let _ = tick_at(&mut tw, t);
        if journey_of(&tw, bus).is_none() {
            arrived = true;
            break;
        }
        assert_eq!(loc(&tw, watcher), Some(bus));
    }
    assert!(arrived, "the bus arrived");
    assert!(
        atlas(&tw).travel.watches.is_empty(),
        "the watch ended when the watcher boarded"
    );
    let _ = run(&mut tw, watcher, "lot");
    assert_eq!(
        loc(&tw, watcher),
        Some(forecourt),
        "rode the rest and got off"
    );
    assert!(
        tw.world()
            .get(patch)
            .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING)),
        "the patch was retired"
    );
}

#[test]
fn s7_a_passenger_who_steps_off_at_a_held_checkpoint_holds_the_road_and_can_walk_back() {
    let mut tw = lot();
    let bus = room(&tw, "Studio Tour Bus");
    let stop = room(&tw, "Bus Stop (OOC)");
    let forecourt = room(&tw, "Forecourt & Bus Stop");
    let watcher = player(&mut tw, "Watcher", forecourt);
    let rider = player(&mut tw, "Rider", stop);
    let _ = run(&mut tw, rider, "board");
    assert_eq!(loc(&tw, rider), Some(bus));
    let _ = run(&mut tw, watcher, "+stakeout Water Tower");
    let patch = room(&tw, "Water Tower");
    let mut held = false;
    for t in (T0 + 1)..=(T0 + 400) {
        let _ = tick_at(&mut tw, t);
        if journey_of(&tw, bus).is_some_and(|j| matches!(j.state, JourneyState::Held { .. })) {
            held = true;
            break;
        }
    }
    assert!(held);
    // The rider steps off; the watcher gives up and walks home; the bus stays held,
    // because the rider standing in the road is a watcher too.
    // ("off" is also the two stops' door alias; the patch door answers to "out".)
    let out = run(&mut tw, rider, "out");
    assert_eq!(loc(&tw, rider), Some(patch), "{out}");
    let _ = run(&mut tw, watcher, "+stakeout/end");
    assert_eq!(loc(&tw, watcher), Some(forecourt));
    let now = tw.world().now();
    let _ = tick_at(&mut tw, now + 1);
    assert!(
        journey_of(&tw, bus).is_some_and(|j| matches!(j.state, JourneyState::Held { .. })),
        "still held by the rider in the road: {:?} / watches {:?} / rider at {:?} patch {:?}",
        journey_of(&tw, bus).map(|j| j.state),
        atlas(&tw).travel.watches,
        loc(&tw, rider),
        patch
    );
    // The rider walks back to the road's first end; the bus goes on and the patch goes.
    let out = run(&mut tw, rider, "+stakeout/end");
    assert!(out.contains("give up the watch"), "{out}");
    assert_eq!(
        loc(&tw, rider),
        Some(stop),
        "back at the OOC stop, the road's first end"
    );
    let now = tw.world().now();
    let _ = tick_at(&mut tw, now + 1);
    assert!(
        journey_of(&tw, bus).is_some_and(|j| j.state == JourneyState::UnderWay),
        "released"
    );
    let mut arrived = false;
    for t in (now + 2)..=(now + 400) {
        let _ = tick_at(&mut tw, t);
        if journey_of(&tw, bus).is_none() {
            arrived = true;
            break;
        }
    }
    assert!(arrived);
    assert!(
        tw.world()
            .get(patch)
            .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING)),
        "the patch was retired"
    );
}

#[test]
fn s4_cancel_leaves_the_traveler_at_its_last_checkpoint_and_the_road_offers_both_ends() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let alice = player(&mut tw, "Alice", west_gate);
    let _ = run(&mut tw, alice, "+travel East End");
    // Before the first checkpoint: cancel turns back to the gate.
    let out = run(&mut tw, alice, "+travel/cancel");
    assert!(out.contains("Your journey ends"), "{out}");
    assert_eq!(loc(&tw, alice), Some(west_gate), "turned back");
    // Past a checkpoint: cancel stops on the road, at that checkpoint.
    let _ = run(&mut tw, alice, "+travel East End");
    let _ = tick_at(&mut tw, T0 + 61);
    let out = run(&mut tw, alice, "+travel/cancel");
    assert!(out.contains("at First Milestone"), "{out}");
    assert_eq!(
        loc(&tw, alice),
        Some(transit_room(&tw)),
        "stopped on the road"
    );
    let status = run(&mut tw, alice, "+travel/status");
    assert!(status.contains("stopped on the Valley Road"), "{status}");
    let ends = run(&mut tw, alice, "+travel");
    assert!(
        ends.contains("West Gate") && ends.contains("East Gate"),
        "{ends}"
    );
    // Going on: two legs remain, from the first milestone.
    let out = run(&mut tw, alice, "+travel East Gate");
    assert!(out.contains("You set off"), "{out}");
    let j = journey_of(&tw, alice).expect("journey");
    assert_eq!(j.legs_ic.len(), 2);
    assert_eq!(j.nodes.len(), 3);
    assert!(matches!(j.nodes.first(), Some(Node::Checkpoint { .. })));
    let now = tw.world().now();
    let _ = tick_at(&mut tw, now + 120);
    assert_eq!(loc(&tw, alice), Some(room(&tw, "East Gate")));
}

#[test]
fn s5_a_restart_resumes_from_the_cursor() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let alice = player(&mut tw, "Alice", west_gate);
    let _ = run(&mut tw, alice, "+travel East End");
    let _ = tick_at(&mut tw, T0 + 61);
    assert_eq!(journey_of(&tw, alice).map(|j| j.cursor), Some(1));
    reboot(&mut tw);
    let j = journey_of(&tw, alice).expect("the journey came back through its wire");
    assert_eq!(j.cursor, 1);
    assert_eq!(loc(&tw, alice), Some(transit_room(&tw)));
    // It goes on at the same stamps: arrival at T0 + 180, not a second later.
    let _ = tick_at(&mut tw, T0 + 179);
    assert!(journey_of(&tw, alice).is_some());
    let _ = tick_at(&mut tw, T0 + 180);
    assert_eq!(loc(&tw, alice), Some(room(&tw, "East Gate")));
}

#[test]
fn s6_staff_hold_and_release_and_a_traveler_under_way_cannot_walk_off_the_road() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let alice = player(&mut tw, "Alice", west_gate);
    let _ = run(&mut tw, alice, "+travel East End");
    let _ = tick_at(&mut tw, T0 + 61);
    let out = run(&mut tw, GOD, "@atlas/journey hold Alice");
    assert!(out.contains("held"), "{out}");
    assert_eq!(
        loc(&tw, alice),
        Some(room(&tw, "First Milestone")),
        "held at the milestone's patch"
    );
    let out = run(&mut tw, GOD, "@atlas/journey show Alice");
    assert!(
        out.contains("Held") && out.contains("First Milestone"),
        "{out}"
    );
    let _ = tick_at(&mut tw, T0 + 100);
    let out = run(&mut tw, GOD, "@atlas/journey release Alice");
    assert!(out.contains("released"), "{out}");
    assert_eq!(loc(&tw, alice), Some(transit_room(&tw)));
    // The gate: no exit and no teleport moves a traveler off the road.
    let out = run(
        &mut tw,
        GOD,
        &format!("@teleport #{}=#{}", alice.0, west_gate.0),
    );
    assert!(
        out.contains("on your way") || loc(&tw, alice) == Some(transit_room(&tw)),
        "{out}"
    );
    assert_eq!(loc(&tw, alice), Some(transit_room(&tw)));
    let out = run(&mut tw, GOD, "@atlas/journey abort Alice testing");
    assert!(out.contains("ended"), "{out}");
    assert!(journey_of(&tw, alice).is_none());
}

#[test]
fn s8_intercept_holds_a_vehicle_at_the_stop_and_release_lets_it_go() {
    let mut tw = lot();
    let bus = room(&tw, "Studio Tour Bus");
    let stop = room(&tw, "Bus Stop (OOC)");
    let blocker = player(&mut tw, "Blocker", stop);
    let out = run(&mut tw, blocker, "+intercept bus");
    assert!(out.contains("You hold the Studio Tour Bus here"), "{out}");
    // Well past the dwell: the bus stays.
    let _ = tick_at(&mut tw, T0 + 300);
    assert!(
        journey_of(&tw, bus).is_none(),
        "held at the stop, not departed"
    );
    let listing = run(&mut tw, GOD, "@atlas/journey list");
    assert!(listing.contains("held by Blocker"), "{listing}");
    let out = run(&mut tw, blocker, "+intercept/release bus");
    assert!(out.contains("You let the Studio Tour Bus go"), "{out}");
    let _ = tick_at(&mut tw, T0 + 301);
    assert!(journey_of(&tw, bus).is_some(), "departed once released");
    // Aboard, the status and the look name the next stop.
    let rider = player(&mut tw, "Rider", stop);
    let out = run(&mut tw, rider, "board");
    assert!(out.contains("is not here"), "the bus has gone: {out}");
    let status = run(&mut tw, blocker, "+travel/status");
    assert!(status.contains("not travelling"), "{status}");
    let show = run(&mut tw, GOD, "@atlas/journey show bus");
    assert!(
        show.contains("Security Gate") && show.contains("Water Tower"),
        "{show}"
    );
    assert!(show.contains("Carrier"), "{show}");
}

#[test]
fn s9_stakeout_lists_what_a_road_out_of_here_passes() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let alice = player(&mut tw, "Alice", west_gate);
    let out = run(&mut tw, alice, "+stakeout");
    assert!(
        out.contains("First Milestone") && out.contains("Second Milestone"),
        "{out}"
    );
    assert!(out.contains("Valley Road"), "{out}");
    assert!(
        !out.contains("The Ford"),
        "a hidden road's checkpoints are not offered: {out}"
    );
    let out = run(&mut tw, alice, "+stakeout Nowhere");
    assert!(out.contains("No checkpoint called"), "{out}");
    let _ = run(&mut tw, alice, "+stakeout First Milestone");
    // From the patch there is no road out: the road is what passes here.
    let out = run(&mut tw, alice, "+travel East End");
    assert!(out.contains("no way"), "{out}");
}

/// The region at version 2: the Valley Road's last leg is longer, so the road's
/// record is updated (its revision moves up) under any journey along it.
fn region_v2() -> String {
    let mut doc: Value = serde_json::from_str(&region_pack()).expect("the region parses");
    doc["manifest"]["version"] = json!(2);
    doc["entries"]["connection.road"]["def"]["legs"][2]["base_s"] = json!(90);
    signed(doc)
}

/// Doc 18 cut IC1 moved this lens here from `tacsim/tests/s9_corridor.rs`, whose
/// copy flew a corridor by atlas `+travel` (the path Q-18-6 removes): a journey
/// carries the revision of the way it set off on, and a re-applied way ends it
/// where it stands (`travel.rs` "the way changed under it").
#[test]
fn s10_a_journey_under_way_ends_where_it_stands_when_its_road_is_reapplied() {
    let mut tw = TestWorld::with_modules(modules_shelved(
        vec![region_pack(), west(), east(), hollow()],
        vec![region_v2()],
    ));
    tw.set_now(T0);
    let _ = tw.fire_due(T0 * 1000);
    let west_gate = room(&tw, "West Gate");
    let alice = player(&mut tw, "Alice", west_gate);
    let out = run(&mut tw, alice, "+travel East End");
    assert!(out.contains("You set off"), "{out}");
    let revision_of = |tw: &TestWorld| {
        atlas(tw)
            .records
            .connections
            .iter()
            .find(|(k, _)| {
                k.pack.as_str() == "region.test" && k.entry.as_str() == "connection.road"
            })
            .map(|(_, c)| c.revision)
    };
    let before = revision_of(&tw).expect("the road is applied");
    assert_eq!(
        journey_of(&tw, alice).map(|j| j.applied_revision),
        Some(before)
    );
    // Past the first checkpoint, on the road.
    let _ = tick_at(&mut tw, T0 + 61);
    assert_eq!(journey_of(&tw, alice).map(|j| j.cursor), Some(1));

    // The road is re-applied under the journey.
    let events = tw.run(GOD, "@atlas/pack plan region.test");
    let planned = seen(&tw, &events, GOD);
    let fingerprint = planned
        .lines()
        .find_map(|l| l.trim().strip_prefix("fingerprint "))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no fingerprint in:\n{planned}"))
        .to_string();
    let applied = run(&mut tw, GOD, &format!("@atlas/pack apply {fingerprint}"));
    assert!(applied.contains("Applied"), "{planned}\n{applied}");
    assert!(
        revision_of(&tw).is_some_and(|r| r > before),
        "the re-applied road's revision moved up"
    );

    // The next tick ends the journey where it stands: on the road, at the
    // checkpoint it last passed, and says why.
    let events = tick_at(&mut tw, T0 + 62);
    let told = seen(&tw, &events, alice);
    assert!(
        told.contains("Your journey ends at First Milestone (the way changed under it)."),
        "{told}"
    );
    assert!(journey_of(&tw, alice).is_none(), "the journey is retired");
    assert_eq!(
        loc(&tw, alice),
        Some(transit_room(&tw)),
        "stopped on the road"
    );
    // And no later tick walks it on along the stale route.
    let _ = tick_at(&mut tw, T0 + 400);
    assert_eq!(loc(&tw, alice), Some(transit_room(&tw)));
}

// ---------------------------------------------------------------------------
// Portals
// ---------------------------------------------------------------------------

#[test]
fn p1_the_farm_road_is_a_timed_portal_and_a_short_door_stays_instant() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let farm = room(&tw, "Farmhouse");
    let shed = room(&tw, "Shed");
    let alice = player(&mut tw, "Alice", west_gate);
    // The short door: instant.
    let _ = run(&mut tw, alice, "shed");
    assert_eq!(loc(&tw, alice), Some(shed));
    let _ = run(&mut tw, alice, "gate");
    assert_eq!(loc(&tw, alice), Some(west_gate));
    // The farm road: a departure, not a step. 420 m on foot at 1.4 m/s = 300 s IC,
    // capped to 180 real at rate 1.
    let out = run(&mut tw, alice, "farm");
    assert!(out.contains("You set off along Farm Road by foot"), "{out}");
    assert_eq!(
        loc(&tw, alice),
        Some(west_gate),
        "still here until the tick"
    );
    let _ = tick_at(&mut tw, T0 + 1);
    let j = journey_of(&tw, alice).expect("the tick departed");
    assert_eq!(loc(&tw, alice), Some(room(&tw, "Farm Road, on the way")));
    assert_eq!(j.legs_ic.len(), 1);
    assert!(j.legs_ic[0] <= 180, "{:?}", j.legs_ic);
    let arrival = j.arrival_at().expect("arrival");
    let _ = tick_at(&mut tw, arrival - 1);
    assert!(journey_of(&tw, alice).is_some());
    let _ = tick_at(&mut tw, arrival);
    assert_eq!(loc(&tw, alice), Some(farm));
    // And back: the same portal the other way.
    let _ = run(&mut tw, alice, "south");
    let _ = tick_at(&mut tw, arrival + 1);
    let j = journey_of(&tw, alice).expect("back on the road");
    assert_eq!(j.destination(), Some(west_gate));
}

// ---------------------------------------------------------------------------
// K — knowledge
// ---------------------------------------------------------------------------

#[test]
fn k1_travel_lists_only_known_places() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let hollow_room = room(&tw, "The Hollow");
    let alice = player(&mut tw, "Alice", west_gate);
    let listing = run(&mut tw, alice, "+travel");
    assert!(listing.contains("East End"), "{listing}");
    assert!(
        !listing.contains("Hollow"),
        "a hidden road is not offered: {listing}"
    );
    let out = run(&mut tw, alice, "+travel The Hollow");
    assert!(out.contains("You know no way"), "{out}");
    // Staff see everything.
    let staff = run(
        &mut tw,
        GOD,
        &format!("@teleport #{}=#{}", GOD.0, west_gate.0),
    );
    let _ = staff;
    let listing = run(&mut tw, GOD, "+travel");
    assert!(listing.contains("Hollow"), "{listing}");
    // Visiting the hollow teaches it (staff take her there: a visit is a visit).
    let _ = run(
        &mut tw,
        GOD,
        &format!("@teleport #{}=#{}", alice.0, hollow_room.0),
    );
    assert_eq!(loc(&tw, alice), Some(hollow_room));
    let _ = run(
        &mut tw,
        GOD,
        &format!("@teleport #{}=#{}", alice.0, west_gate.0),
    );
    assert_eq!(loc(&tw, alice), Some(west_gate));
    let listing = run(&mut tw, alice, "+travel");
    assert!(listing.contains("Hollow"), "known now: {listing}");
    let out = run(&mut tw, alice, "+travel Hollow");
    assert!(
        out.contains("You set off along the Smugglers' Path"),
        "{out}"
    );
}

#[test]
fn dt_self_loop_exit_still_teaches_its_neighborhood() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let actor = player(&mut tw, "Loop Walker", west_gate);
    let exit = tw
        .world_mut()
        .create(ObjKind::Exit, "circle", GOD, Some(west_gate))
        .expect("self-loop exit");
    tw.world_mut()
        .link_exit(exit, Some(west_gate))
        .expect("self-loop destination");
    let _ = tw.take_module_ops();
    tw.run(actor, "circle");
    assert_eq!(loc(&tw, actor), Some(west_gate));
    let planned = tw
        .world()
        .module_state::<AtlasState>()
        .expect("Atlas")
        .travel
        .known
        .get(&actor)
        .cloned()
        .expect("self-loop discovers the place");
    let key = format!("known:{}", actor.0);
    let writes: Vec<_> = tw
        .take_module_ops()
        .into_iter()
        .filter_map(|op| {
            if op.ns.as_str() != "atlas" {
                return None;
            }
            match op.op {
                ModuleWrite::JsonSet {
                    key: written,
                    value,
                } if written.as_str() == key => Some(value),
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        writes,
        vec![serde_json::json!({ "places": planned })]
    );
}

#[test]
fn k2_intercept_reveals_no_destination_and_no_route() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let east_gate = room(&tw, "East Gate");
    let alice = player(&mut tw, "Alice", west_gate);
    let bob = player(&mut tw, "Bob", west_gate);
    let carol = player(&mut tw, "Carol", east_gate);
    let hollow_room = room(&tw, "The Hollow");
    let dave = player(&mut tw, "Dave", hollow_room);
    let _ = run(&mut tw, alice, "+travel East End");
    let _ = run(&mut tw, bob, "+travel East End");
    // Carol, where they are going, and Dave, where they are not, get the same words.
    let at_dest = run(&mut tw, carol, "+intercept Alice");
    let elsewhere = run(&mut tw, dave, "+intercept Bob");
    assert_eq!(
        at_dest.replace("Alice", "X"),
        elsewhere.replace("Bob", "X"),
        "an intercept confirms nothing about the route"
    );
    assert!(!at_dest.contains("East"), "{at_dest}");
    // Alice's own status names the road and the next milestone, never Bob's; a
    // stranger's look at the road sees no destination.
    let status = run(&mut tw, alice, "+travel/status");
    assert!(
        status.contains("Valley Road") && status.contains("First Milestone"),
        "{status}"
    );
    // Carol's intercept lands when Alice arrives: Alice is held at the East Gate.
    let _ = tick_at(&mut tw, T0 + 400);
    assert_eq!(loc(&tw, alice), Some(east_gate));
    assert!(
        journey_of(&tw, alice).is_none(),
        "arrival is arrival; the hold was at the room"
    );
    // Bob's journey was never touched by Dave.
    assert_eq!(loc(&tw, bob), Some(east_gate));
}

#[test]
fn k3_two_travelers_at_different_points_of_one_road_do_not_see_each_other() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let alice = player(&mut tw, "Alice", west_gate);
    let bob = player(&mut tw, "Bob", west_gate);
    let _ = run(&mut tw, alice, "+travel East End");
    let _ = tick_at(&mut tw, T0 + 61);
    let _ = run(&mut tw, bob, "+travel East End");
    assert_eq!(loc(&tw, alice), loc(&tw, bob), "one transit room");
    assert_ne!(
        journey_of(&tw, alice).map(|j| j.cursor),
        journey_of(&tw, bob).map(|j| j.cursor)
    );
    assert!(!can_see(&tw, alice, bob), "a mile apart on the same road");
    let _ = tick_at(&mut tw, T0 + 122);
    assert_eq!(journey_of(&tw, bob).map(|j| j.cursor), Some(1));
    assert_eq!(journey_of(&tw, alice).map(|j| j.cursor), Some(2));
    assert!(!can_see(&tw, alice, bob));
}

#[test]
fn k4_a_disconnect_leaves_the_traveler_where_it_is() {
    let mut tw = valley();
    let west_gate = room(&tw, "West Gate");
    let alice = player(&mut tw, "Alice", west_gate);
    let _ = run(&mut tw, alice, "+travel East End");
    let _ = tick_at(&mut tw, T0 + 61);
    disconnect(&mut tw, alice);
    assert!(journey_of(&tw, alice).is_none(), "the journey ended");
    assert_eq!(
        loc(&tw, alice),
        Some(transit_room(&tw)),
        "on the road, at the milestone"
    );
    let _ = tick_at(&mut tw, T0 + 400);
    assert_eq!(loc(&tw, alice), Some(transit_room(&tw)), "and stays there");
}
