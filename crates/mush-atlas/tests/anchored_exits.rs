//! **Doc 18 cut IC9 — no authored exit leads into a room aboard a moving hull**
//! (lead ruling L16; D-18-22; `mush_sim::anchored_space_of`).
//!
//! The atlas applier's `create_exit` asks the simulation, before any geometry or
//! object is touched, whether either end of an exit row (the destination, and the
//! origin when the row names the way back too) is a room of an interior space
//! anchored to a moving object. If so the row is refused with
//! `"{id}: {room} is a room aboard a moving hull; no exit leads into one"` and
//! nothing is created; an exit into an unanchored interior's room is applied as
//! it always was.
//!
//! The rig: a region and a neighbourhood with three touching rooms applied at
//! boot; one room made a member of a space riding a hull (and another of an
//! unanchored space) through `mush_sim::run(Transition::Space/Members)`; then the
//! neighbourhood's next version, adding one exit row, planned and applied from
//! the shelf as staff do.

use mush_atlas::AtlasModule;
use mush_content::{Appliers, ContentModule, Pack, parse_pack};
use mush_core::{
    BehaviorBinding, BehaviorId, Ctx, Flags, GOD, GameModule, LIMBO, ModuleActionState, ObjId,
    ObjKind, Outbox, ParsedArgs, SwitchSet,
};
use mush_sim::{SimulationModule, SpaceAnchor, Transition};
use mush_testkit::TestWorld;
use serde_json::{Value, json};
use smol_str::SmolStr;

fn signed(mut doc: Value) -> Pack {
    doc["manifest"]["digest"] = Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = parse_pack(&unsigned)
        .expect("parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = Value::String(digest);
    parse_pack(&serde_json::to_string(&doc).expect("json")).expect("parses")
}

fn region() -> Pack {
    signed(json!({
        "manifest": { "id": "region.town", "version": 1, "kind": "region", "digest": "", "requires": [] },
        "entries": {
            "region.town": { "kind": "region", "def": {
                "name": "The Town", "kind": "surface", "body": "earth",
                "extent": [[0, 0], [400, 400]], "datum_m": 0.0,
                "background": { "surface": "paved", "mix": [["grass", 8]], "relief_cm": 0,
                                "relief_period_cm": 800, "seed": 1 },
                "ground_resolution_cm": 1000 } },
            "locality.town": { "kind": "locality", "def": {
                "region": "region.town", "name": "Town", "kind": "district",
                "footprint": [[0, 0], [400, 0], [400, 400], [0, 400]] } }
        }
    }))
}

/// The Block at `version`: three touching rooms (Ay, Bee, Cee), each joined to
/// Limbo (the validator refuses an isolated or unreachable room), plus `extra`
/// exit rows.
fn block(version: u32, extra: &[(&str, Value)]) -> Pack {
    let mut doc = json!({
        "manifest": { "id": "neighborhood.block", "version": version, "kind": "neighborhood",
                      "digest": "", "requires": [{ "id": "region.town", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.town", "locality": "locality.town", "name": "The Block",
                "setting": "suburban", "footprint": [[0, 0], [200, 0], [200, 200], [0, 200]],
                "room_budget": { "target": 3, "maximum": 12 } } },
            "room.a": { "kind": "room", "def": { "name": "Ay", "surface": "ground",
                "footprint": { "rect": [[10, 10], [60, 60]] }, "desc": "Room Ay." } },
            "room.b": { "kind": "room", "def": { "name": "Bee", "surface": "ground",
                "footprint": { "rect": [[60, 10], [110, 60]] }, "desc": "Room Bee." } },
            "room.c": { "kind": "room", "def": { "name": "Cee", "surface": "ground",
                "footprint": { "rect": [[10, 60], [60, 110]] }, "desc": "Room Cee." } },
            "exit.limbo_a": { "kind": "exit", "def": { "from": "limbo", "to": "room.a",
                "names": ["Out <O>;o;out", "Limbo <L>;limbo"] } },
            "exit.limbo_b": { "kind": "exit", "def": { "from": "limbo", "to": "room.b",
                "names": ["Bee Door;beedoor", "Limbo <L>;limbo"] } },
            "exit.limbo_c": { "kind": "exit", "def": { "from": "limbo", "to": "room.c",
                "names": ["Cee Door;ceedoor", "Limbo <L>;limbo"] } }
        }
    });
    for (id, row) in extra {
        doc["entries"][*id] = row.clone();
    }
    signed(doc)
}

fn modules(shelf: Pack) -> Vec<Box<dyn GameModule>> {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    vec![
        Box::new(SimulationModule::new()),
        Box::new(AtlasModule::new().with_shelf(vec![shelf])),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(vec![region(), block(1, &[])]),
        ),
    ]
}

fn say(tw: &mut TestWorld, line: &str) -> String {
    let events = tw.run(GOD, line);
    tw.seen_by(&events, GOD).join("\n")
}

/// Plan and apply the shelved Block, returning what staff were told by each.
fn replan(tw: &mut TestWorld) -> (String, String) {
    let planned = say(tw, "@atlas/pack plan neighborhood.block");
    let fingerprint = planned
        .lines()
        .find_map(|l| l.trim().strip_prefix("fingerprint "))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no fingerprint in:\n{planned}"))
        .to_string();
    let applied = say(tw, &format!("@atlas/pack apply {fingerprint}"));
    (planned, applied)
}

fn named(tw: &TestWorld, kind: ObjKind, name: &str) -> Option<ObjId> {
    tw.world()
        .iter()
        .filter(|o| !o.flags.contains(Flags::GOING))
        .find(|o| o.kind == kind && o.name == name)
        .map(|o| o.id)
}

fn exits(tw: &TestWorld) -> usize {
    tw.world()
        .iter()
        .filter(|o| o.kind == ObjKind::Exit && !o.flags.contains(Flags::GOING))
        .count()
}

fn bind(tw: &mut TestWorld, obj: ObjId, behavior: &str) {
    tw.world_mut()
        .attach_behavior(
            obj,
            BehaviorId::new(behavior).expect("id"),
            BehaviorBinding {
                schema_version: mush_core::BINDING_SCHEMA_VERSION,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .expect("binds");
}

/// Run one spatial transition as staff in the simulation's own context.
fn transition(tw: &mut TestWorld, t: &Transition) {
    let mut out = Outbox::new();
    let engine = tw.engine_mut();
    let mut ctx = Ctx {
        w: &mut engine.world,
        actor: GOD,
        switches: SwitchSet::default(),
        args: ParsedArgs::default(),
        out: &mut out,
        reg: &engine.registry,
        module: Some(SmolStr::new("sim")),
        typed_state: ModuleActionState::unavailable(),
    };
    mush_sim::run(&mut ctx, t).expect("the transition applies");
}

/// A space room `name` with `anchor`, whose members are itself and `member`.
fn space_with_member(tw: &mut TestWorld, name: &str, anchor: SpaceAnchor, member: ObjId) -> ObjId {
    let space = tw
        .world_mut()
        .create(ObjKind::Room, name, GOD, None)
        .expect("a room");
    bind(tw, space, "sim.interior");
    transition(tw, &Transition::Space { space, anchor });
    transition(
        tw,
        &Transition::Members {
            space,
            rooms: [space, member].into_iter().collect(),
        },
    );
    space
}

/// A world at Block v1 with Bee a member of a space riding a hull and Cee a
/// member of an unanchored space; `shelf` is the next version.
fn world(shelf: Pack) -> TestWorld {
    let mut tw = TestWorld::with_modules(modules(shelf));
    let bee = named(&tw, ObjKind::Room, "Bee").expect("Bee applied");
    let cee = named(&tw, ObjKind::Room, "Cee").expect("Cee applied");
    let hull = tw
        .world_mut()
        .create(ObjKind::Thing, "Hull", GOD, Some(LIMBO))
        .expect("a hull");
    bind(&mut tw, hull, "sim.participant");
    let lock = space_with_member(
        &mut tw,
        "Lock",
        SpaceAnchor::MovingObject { host: hull },
        bee,
    );
    let pod = space_with_member(&mut tw, "Pod", SpaceAnchor::Unanchored, cee);
    assert_eq!(mush_sim::anchored_space_of(tw.world(), bee), Some(lock));
    assert_eq!(mush_sim::anchored_space_of(tw.world(), cee), None);
    assert_eq!(mush_sim::anchored_space_of(tw.world(), pod), None);
    tw
}

fn exit_row(from: &str, to: &str, names: &[&str]) -> Value {
    json!({ "kind": "exit", "def": { "from": from, "to": to, "names": names } })
}

/// **An exit into a room aboard a moving hull is refused at apply**, with the
/// sentence naming the row and the reference as authored, and nothing is
/// created — one-way into it, and a pair whose way back starts there.
#[test]
fn ic9_an_exit_into_a_room_aboard_a_moving_hull_is_refused_at_apply() {
    // One way, into Bee.
    let mut tw = world(block(
        2,
        &[("exit.a_b", exit_row("room.a", "room.b", &["Bee <B>;b;bee"]))],
    ));
    let before = exits(&tw);
    let (_, applied) = replan(&mut tw);
    assert!(
        applied.contains("exit.a_b: room.b is a room aboard a moving hull; no exit leads into one"),
        "{applied}"
    );
    assert_eq!(exits(&tw), before, "nothing was created");
    assert!(named(&tw, ObjKind::Exit, "Bee <B>").is_none());

    // A pair whose way back leads into Bee: the origin is asked too.
    let mut tw = world(block(
        2,
        &[(
            "exit.b_a",
            exit_row("room.b", "room.a", &["Ay <A>;a;ay", "Bee <B>;b;bee"]),
        )],
    ));
    let before = exits(&tw);
    let (_, applied) = replan(&mut tw);
    assert!(
        applied.contains("exit.b_a: room.b is a room aboard a moving hull; no exit leads into one"),
        "{applied}"
    );
    assert_eq!(exits(&tw), before, "nothing was created");
}

/// **An exit into an unanchored interior's room applies as before.**
#[test]
fn ic9_an_exit_into_an_unanchored_interior_room_applies_as_before() {
    let mut tw = world(block(
        2,
        &[(
            "exit.a_c",
            exit_row("room.a", "room.c", &["Cee <C>;c;cee", "Ay <A>;a;ay"]),
        )],
    ));
    let before = exits(&tw);
    let (_, applied) = replan(&mut tw);
    assert!(applied.contains("Applied neighborhood.block"), "{applied}");
    assert_eq!(exits(&tw), before + 2, "the pair was created");
    let a = named(&tw, ObjKind::Room, "Ay").expect("Ay");
    let forward = named(&tw, ObjKind::Exit, "Cee <C>").expect("the exit");
    assert_eq!(tw.world().get(forward).and_then(|o| o.location), Some(a));
}
