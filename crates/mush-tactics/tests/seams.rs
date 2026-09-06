//! The standing seam test (design doc 04 §6.3): every reserved core
//! extension point the real tactics engine will need, proven through the toy
//! [`TacticsStub`] over the real [`Engine`] via `mush-testkit`. This file is
//! the acceptance test of treaty ruling 4 — it stays green forever.

use mush_core::{Audience, GOD, LIMBO, ModuleWrite, SceneId};
use mush_proto::{OutputEvent, PanelFrameKind};
use mush_tactics::{
    BATTLE_TICK, HEXLOCK_MSG, MAP_FEED, NS, PULSE_TICK, SCENE_KIND, TacticsState, TacticsStub,
};
use mush_testkit::{TestWorld, assert_actor_saw};

fn tactics_world() -> TestWorld {
    TestWorld::with_modules(vec![Box::new(TacticsStub::new())])
}

fn pulses(tw: &TestWorld) -> u64 {
    tw.world().module_state::<TacticsState>().unwrap().pulses
}

fn active_battle(tw: &TestWorld, who: mush_core::ObjId) -> Option<SceneId> {
    tw.world()
        .module_state::<TacticsState>()
        .unwrap()
        .battles
        .get(&who)
        .copied()
}

/// Panel frames of one kind on `feed`, addressed to `who`.
fn panel_frames(
    events: &[(Audience, OutputEvent)],
    who: mush_core::ObjId,
    kind: PanelFrameKind,
) -> Vec<serde_json::Value> {
    events
        .iter()
        .filter(|(aud, _)| matches!(aud, Audience::Actor(a) if a == &who))
        .filter_map(|(_, ev)| match ev {
            OutputEvent::Panel {
                feed,
                kind: k,
                data,
            } if feed == MAP_FEED && *k == kind => Some(data.clone()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------
// Checklist 1: sub-second Fast tick, scheduler-re-armed
// ---------------------------------------------------------------------

#[test]
fn fast_tick_fires_at_250ms_cadence() {
    let mut tw = tactics_world();
    assert_eq!(pulses(&tw), 0);
    // Armed one period after construction (now = 0).
    tw.fire_due(249);
    assert_eq!(pulses(&tw), 0, "not due yet");
    tw.fire_due(250);
    assert_eq!(pulses(&tw), 1);
    tw.fire_due(499);
    assert_eq!(pulses(&tw), 1, "re-armed one period from the fire");
    tw.fire_due(500);
    assert_eq!(pulses(&tw), 2);
    // Registration is discoverable (tick registry seam).
    let reg = tw
        .registry()
        .tick_reg(NS, PULSE_TICK)
        .expect("pulse registered");
    assert!(matches!(
        reg.cadence,
        mush_core::TickCadence::Fast(d) if d.as_millis() == 250
    ));
}

// ---------------------------------------------------------------------
// Checklist 2+3+4: scene lifecycle, per-scene ticker, RulesetStateStore +
// MapBlob persistence, panel keyframe/delta frames
// ---------------------------------------------------------------------

#[test]
fn battle_scene_ticks_state_persists_and_panels_stream() {
    let mut tw = tactics_world();
    let pilot = tw.spawn_player("Natasha");
    tw.take_module_ops(); // discard fixture noise

    // --- start: scene opens, map blob stored, keyframe emitted. ----------
    let events = tw.run(pilot, "+btest/start");
    let scene = active_battle(&tw, pilot).expect("battle open");
    assert_eq!(tw.world().scene(scene).unwrap().kind, SCENE_KIND);
    assert_actor_saw(&events, pilot, &format!("BTEST: battle {scene} started"));
    let keyframes = panel_frames(&events, pilot, PanelFrameKind::Keyframe);
    assert_eq!(keyframes.len(), 1, "one keyframe at battle start");
    assert!(
        keyframes[0].get("map").is_some(),
        "keyframe carries the map"
    );
    assert!(
        panel_frames(&events, pilot, PanelFrameKind::Delta).is_empty(),
        "no deltas before the first tick"
    );

    // The map blob and the initial state snapshot were persisted as module
    // ops under ns "tactics" — write-through, same channel as everything.
    let ops = tw.take_module_ops();
    assert!(ops.iter().all(|op| op.ns == NS));
    let map_key = format!("scene:{}:map", scene.0);
    let state_key = format!("scene:{}:state", scene.0);
    assert!(
        ops.iter().any(|op| matches!(
            &op.op,
            ModuleWrite::JsonSet { key, value } if key == map_key.as_str()
                && value.get("hexes").is_some()
        )),
        "map blob must persist: {ops:?}"
    );
    assert!(
        ops.iter().any(|op| matches!(
            &op.op,
            ModuleWrite::JsonSet { key, .. } if key == state_key.as_str()
        )),
        "initial scene state must persist: {ops:?}"
    );

    // --- tick: heat bumps in the state store, a delta streams. -----------
    let events = tw.fire_due(250);
    assert_eq!(
        tw.world()
            .scene_state(scene, pilot)
            .and_then(|v| v.get("heat"))
            .and_then(serde_json::Value::as_u64),
        Some(1),
        "battle tick must bump heat in the per-scene state store"
    );
    let deltas = panel_frames(&events, pilot, PanelFrameKind::Delta);
    assert_eq!(deltas.len(), 1, "one delta per tick");
    assert_eq!(
        deltas[0]["heat"][pilot.to_string()],
        serde_json::json!(1),
        "delta carries the new heat"
    );
    // Each change persisted a whole-scene snapshot.
    let ops = tw.take_module_ops();
    assert!(
        ops.iter().any(|op| matches!(
            &op.op,
            ModuleWrite::JsonSet { key, .. } if key == state_key.as_str()
        )),
        "tick must persist the scene snapshot: {ops:?}"
    );

    // A second tick keeps counting (re-armed dynamically).
    tw.fire_due(500);
    assert_eq!(
        tw.world()
            .scene_state(scene, pilot)
            .and_then(|v| v.get("heat"))
            .and_then(serde_json::Value::as_u64),
        Some(2)
    );
    let status = tw.run(pilot, "+btest/status");
    assert_actor_saw(&status, pilot, "heat 2");

    // --- stop: scene ends, ticker dies, persisted blobs are deleted. -----
    tw.take_module_ops();
    let events = tw.run(pilot, "+btest/stop");
    assert_actor_saw(&events, pilot, &format!("BTEST: battle {scene} ended"));
    assert!(
        active_battle(&tw, pilot).is_none(),
        "SceneEnded hook cleaned up"
    );
    assert!(tw.world().scene(scene).is_none(), "scene removed");
    let ops = tw.take_module_ops();
    let deleted: Vec<&str> = ops
        .iter()
        .filter_map(|op| match &op.op {
            ModuleWrite::Del { key } => Some(key.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        deleted.contains(&state_key.as_str()) && deleted.contains(&map_key.as_str()),
        "ending the scene must delete its persisted state and map: {ops:?}"
    );

    // The battle ticker is gone with the scene: later fires change nothing.
    let before = tw.world().module_state::<TacticsState>().unwrap().clone();
    tw.fire_due(750);
    assert!(
        tw.take_module_ops().is_empty(),
        "no scene writes after stop"
    );
    assert_eq!(
        tw.world().module_state::<TacticsState>().unwrap().battles,
        before.battles
    );
}

// ---------------------------------------------------------------------
// Checklist 5: MoveAttempt veto via the registered marker flag
// ---------------------------------------------------------------------

#[test]
fn hexlock_marker_vetoes_movement_and_teleport() {
    let mut tw = tactics_world();
    let pilot = tw.spawn_player("Natasha");
    tw.run(GOD, "@dig Arena=enter;in,out;o");
    let arena = tw
        .world()
        .iter()
        .find(|o| o.name == "Arena")
        .expect("dug room")
        .id;

    // The marker is registered: settable through plain @set.
    let events = tw.run(GOD, &format!("@set #{}=tacseam:hexlock", arena.0));
    assert_actor_saw(&events, GOD, "Set.");
    let (canonical, owner) = tw.registry().marker_owner("TACSEAM:HEXLOCK").unwrap();
    assert_eq!(
        (canonical.as_str(), owner.as_str()),
        ("tacseam:hexlock", NS)
    );

    // Exit traversal is vetoed with the themed message; nobody moves.
    let events = tw.run(pilot, "enter");
    assert_actor_saw(&events, pilot, HEXLOCK_MSG);
    assert_eq!(tw.world().get(pilot).unwrap().location, Some(LIMBO));

    // @teleport is vetoed through the same hook (exit = None).
    let events = tw.run(GOD, &format!("@teleport #{}", arena.0));
    assert_actor_saw(&events, GOD, HEXLOCK_MSG);
    assert_eq!(tw.world().get(GOD).unwrap().location, Some(LIMBO));

    // Clearing the marker restores movement.
    let events = tw.run(GOD, &format!("@set #{}=!tacseam:hexlock", arena.0));
    assert_actor_saw(&events, GOD, "Cleared.");
    tw.run(pilot, "enter");
    assert_eq!(tw.world().get(pilot).unwrap().location, Some(arena));
}

// ---------------------------------------------------------------------
// Checklist 6: registration is the only wiring point
// ---------------------------------------------------------------------

#[test]
fn stub_wires_through_registration_alone() {
    // Constructing the engine with the module is ALL the wiring there is:
    // the feed, marker, ticks, and command all exist afterwards — no core
    // code knows tactics.
    let tw = tactics_world();
    assert!(
        tw.registry()
            .panel_feeds()
            .iter()
            .any(|(f, m)| f == MAP_FEED && m == NS)
    );
    assert!(tw.registry().tick_reg(NS, BATTLE_TICK).is_some());
    assert!(tw.registry().spec_of("+btest").is_some());
    assert!(tw.world().module_state::<TacticsState>().is_some());
}

// ---------------------------------------------------------------------
// Review-fix regressions: destroyed members, boot-time scene purge, the
// reserved WeatherQuery seam
// ---------------------------------------------------------------------

/// `@destroy`ing the starter of a battle must end the scene — otherwise
/// its 250ms ticker (and its persisted snapshots) leak until reboot. The
/// stub's `ObjDestroyed` observer is the documented pattern.
#[test]
fn destroying_the_starter_ends_the_battle_and_its_ticker() {
    let mut tw = tactics_world();
    let pilot = tw.spawn_player("Natasha");
    tw.run(pilot, "+btest/start");
    let scene = active_battle(&tw, pilot).expect("battle open");
    tw.take_module_ops();

    tw.run(GOD, "@destroy Natasha");
    assert!(
        tw.world().scene(scene).is_none(),
        "the dead starter's battle must end"
    );
    assert!(
        tw.world()
            .module_state::<TacticsState>()
            .unwrap()
            .battles
            .is_empty(),
        "no dangling battle entry"
    );
    // The scene's persisted docs were deleted (owner namespace).
    let ops = tw.take_module_ops();
    let state_key = format!("scene:{}:state", scene.0);
    assert!(
        ops.iter().any(|op| op.ns == NS
            && matches!(&op.op, ModuleWrite::Del { key } if key == state_key.as_str())),
        "ending must delete the scene docs: {ops:?}"
    );
    // The ticker is dead: later fires write nothing.
    tw.fire_due(250);
    tw.fire_due(500);
    assert!(
        tw.take_module_ops().is_empty(),
        "no scene ticker survives its starter's destruction"
    );
}

/// The scene boot contract: scene ids restart at 0 each boot, so stale
/// `scene:*` documents left by a crash-without-end_scene are purged in
/// `on_world_loaded`.
#[test]
fn boot_purges_stale_scene_documents() {
    use std::collections::HashMap;

    use mush_core::ModuleBootData;
    use smol_str::SmolStr;

    let mut tw = tactics_world();
    tw.take_module_ops();
    let boot = HashMap::from([(
        SmolStr::new(NS),
        ModuleBootData {
            kv: vec![
                (
                    SmolStr::new("scene:3:state"),
                    serde_json::json!({"#5": {"heat": 9}}),
                ),
                (SmolStr::new("scene:3:map"), serde_json::json!({"w": 2})),
                (SmolStr::new("config"), serde_json::json!({"keep": true})),
            ],
            streams: HashMap::new(),
            ..Default::default()
        },
    )]);
    tw.modules_loaded(boot);
    let dels: Vec<String> = tw
        .take_module_ops()
        .iter()
        .filter_map(|op| match &op.op {
            ModuleWrite::Del { key } if op.ns == NS => Some(key.to_string()),
            _ => None,
        })
        .collect();
    assert!(dels.contains(&"scene:3:state".to_string()), "{dels:?}");
    assert!(dels.contains(&"scene:3:map".to_string()));
    assert!(
        !dels.contains(&"config".to_string()),
        "only scene:* keys are purged"
    );
}

/// The reserved `WeatherQuery` seam (PLAN's tactics seam list): registered
/// through the registrar, discoverable through the registry, readable end
/// to end.
#[test]
fn weather_query_seam_is_registered_and_readable() {
    let tw = tactics_world();
    let (owner, provider) = tw
        .registry()
        .weather_query()
        .expect("the stub registers a weather provider");
    assert_eq!(owner.as_str(), NS);
    let wx = provider
        .current(tw.world(), LIMBO)
        .expect("stub observation");
    assert_eq!(wx.state, "clear");
    assert!(wx.visibility >= 1.0);
}
