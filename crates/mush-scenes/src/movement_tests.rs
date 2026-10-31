#![cfg(test)]

use super::*;
use crate::{PauseReason, SceneMode, ScenesModule};
use mush_core::{ClockRef, ClockStamp, Effects, GOD, ObjKind, Outbox, WorldEvent};
use serde_json::json;

fn scene(id: u64, anchor: ObjId) -> SceneRecord {
    SceneRecord {
        id: SceneUid(id),
        title: "Movement scene".into(),
        kind: "timestop".into(),
        mode: SceneMode::TurnBased,
        status: SceneStatus::Active,
        pause_reason: None,
        exclusive: true,
        anchor,
        participants: vec![ObjId(20), ObjId(21)],
        observers: vec![ObjId(22)],
        facilitator: GOD,
        created_by: GOD,
        created_at: 100,
        ic_started_at: ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 500,
        },
        clock_generation: 1,
        turn: 0,
        round_seconds: 6,
        round: 3,
        turn_index: 0,
        ended_at: None,
        sealer: None,
        seal_run: None,
        outcome: None,
        ic_floor: 0,
        log_chunks: None,
        consent: vec![],
        nudge_after_s: None,
        turn_started_at: 0,
        turn_visit: 0,
        nudged: None,
    }
}

fn state() -> ScenesState {
    let mut state = ScenesState::default();
    state.scenes.insert(SceneUid(1), scene(1, ObjId(10)));
    let mut destination = scene(2, ObjId(11));
    destination.ic_floor = 700;
    destination.participants = vec![ObjId(31), ObjId(30)];
    // Keep original order and duplicates: SceneEvent::new does not deduplicate.
    destination.observers = vec![ObjId(32), ObjId(31)];
    state.scenes.insert(SceneUid(2), destination);
    state
}

fn planned(state: &ScenesState) -> Vec<PlannedSceneEvent> {
    legacy_events(state, ObjId(99), Some(ObjId(10)), ObjId(11), 1234)
}

#[test]
fn events_preserve_original_times_witnesses_and_order() {
    let state = state();
    let events = planned(&state);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].scene(), SceneUid(2));
    assert_eq!(
        events[0].event(),
        &SceneEvent::new(
            state.scenes.get(&SceneUid(2)).expect("destination"),
            1234,
            SceneEventKind::Joined,
            Some(ObjId(99)),
            json!({}),
        )
    );
    assert_eq!(events[1].scene(), SceneUid(1));
    assert_eq!(events[1].event().kind, SceneEventKind::Left);
    assert_eq!(events[0].event().ic_time, 700);
    assert_eq!(events[1].event().ic_time, 512);
    assert_eq!(
        events[0].event().witnesses,
        vec![ObjId(31), ObjId(30), ObjId(32), ObjId(31)]
    );
}

#[test]
fn real_registered_observer_appends_each_original_event_once_after_movement() {
    let mut world = mush_testkit::TestWorld::with_modules(vec![Box::new(ScenesModule::new())]);
    let source = world
        .world_mut()
        .create(ObjKind::Room, "source", GOD, None)
        .expect("source");
    let destination = world
        .world_mut()
        .create(ObjKind::Room, "destination", GOD, None)
        .expect("destination");
    let actor = world
        .world_mut()
        .create(ObjKind::Player, "walker", GOD, Some(source))
        .expect("actor");
    world.world_mut().set_now(1234);
    let mut source_scene = scene(1, source);
    source_scene.participants = vec![actor];
    let destination_scene = scene(2, destination);
    let state = world
        .world_mut()
        .module_state_mut::<ScenesState>()
        .expect("scene state");
    state.scenes.insert(SceneUid(1), source_scene);
    state.scenes.insert(SceneUid(2), destination_scene);
    let expected = legacy_events(state, actor, Some(source), destination, 1234);
    assert!(state.events.open_entries("1").is_empty());
    assert!(state.events.open_entries("2").is_empty());
    world.take_module_ops();
    world
        .world_mut()
        .move_to(actor, destination)
        .expect("actual move");
    world.engine_mut().emit_world_event(
        &WorldEvent::Moved {
            who: actor,
            from: Some(source),
            to: destination,
        },
        &mut Outbox::default(),
        &mut Effects::default(),
    );
    assert_eq!(
        world.world().get(actor).and_then(|object| object.location),
        Some(destination)
    );
    let state = world
        .world()
        .module_state::<ScenesState>()
        .expect("scene state");
    for planned in &expected {
        let entries = state.events.open_entries(&planned.scene().0.to_string());
        assert_eq!(entries.len(), 1);
        let event: SceneEvent = serde_json::from_value(entries[0].value.clone()).expect("event");
        assert_eq!(&event, planned.event());
    }
    assert_eq!(world.take_module_ops().len(), 2);
}

#[test]
fn self_loop_and_absent_or_inactive_scenes_produce_no_events() {
    let mut state = state();
    assert!(legacy_events(&state, ObjId(99), Some(ObjId(10)), ObjId(10), 42).is_empty());
    assert!(legacy_events(&state, ObjId(99), None, ObjId(999), 42).is_empty());
    for status in [
        SceneStatus::Preparing,
        SceneStatus::Paused,
        SceneStatus::Closed,
    ] {
        for scene in state.scenes.values_mut() {
            scene.status = status;
        }
        assert!(planned(&state).is_empty());
    }
}

#[test]
fn first_paused_holding_scene_masks_later_active_scene_as_before() {
    let mut state = state();
    let first = state.scenes.get_mut(&SceneUid(1)).expect("first");
    first.status = SceneStatus::Paused;
    first.pause_reason = Some(PauseReason::ServerRestart);
    state.scenes.insert(SceneUid(3), scene(3, ObjId(10)));
    let events = planned(&state);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event().kind, SceneEventKind::Joined);
    state.scenes.get_mut(&SceneUid(1)).expect("first").status = SceneStatus::Closed;
    assert_eq!(planned(&state)[1].scene(), SceneUid(3));
}

#[test]
fn witnesses_are_unbounded_on_the_observer_path() {
    let mut state = state();
    let destination = state.scenes.get_mut(&SceneUid(2)).expect("destination");
    destination.participants = (0..128).map(ObjId).collect();
    destination.observers = (128..257).map(ObjId).collect();
    assert_eq!(planned(&state)[0].event().witnesses.len(), 257);
}
