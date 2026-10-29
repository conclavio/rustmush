#![cfg(test)]

use super::*;
use crate::{PauseReason, SceneMode, ScenesModule};
use mush_core::{ClockRef, ClockStamp, Effects, GOD, ObjKind, Outbox, WorldEvent};
use serde_json::{Value, json};

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

fn planned(state: &ScenesState) -> MovementCapture {
    capture(state, ObjId(99), Some(ObjId(10)), ObjId(11), 1234).expect("bounded capture")
}

fn canonical(value: &Value) -> Vec<u8> {
    canonical_json_bytes_with_limit(value, MAX_MOVEMENT_CAPTURE_BYTES).expect("bounded JSON")
}

#[test]
fn capture_preserves_original_events_times_witnesses_and_legacy_order() {
    let state = state();
    let capture = planned(&state);
    assert_eq!(
        capture.events(),
        legacy_events(&state, ObjId(99), Some(ObjId(10)), ObjId(11), 1234)
    );
    assert_eq!(capture.actor(), ObjId(99));
    assert_eq!(capture.from(), Some(ObjId(10)));
    assert_eq!(capture.to(), ObjId(11));
    assert_eq!(capture.at(), 1234);
    assert_eq!(capture.events().len(), 2);
    assert_eq!(capture.events()[0].scene(), SceneUid(2));
    assert_eq!(
        capture.events()[0].event(),
        &SceneEvent::new(
            state.scenes.get(&SceneUid(2)).expect("destination"),
            1234,
            SceneEventKind::Joined,
            Some(ObjId(99)),
            json!({}),
        )
    );
    assert_eq!(capture.events()[1].scene(), SceneUid(1));
    assert_eq!(capture.events()[1].event().kind, SceneEventKind::Left);
    assert_eq!(capture.events()[0].event().ic_time, 700);
    assert_eq!(capture.events()[1].event().ic_time, 512);
    assert_eq!(
        capture.events()[0].event().witnesses,
        vec![ObjId(31), ObjId(30), ObjId(32), ObjId(31)]
    );
    let bytes = capture.encode().expect("encode");
    assert_eq!(MovementCapture::decode(&bytes).expect("decode"), capture);
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
    let expected = capture(state, actor, Some(source), destination, 1234).expect("capture");
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
    for planned in expected.events() {
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
    assert!(
        capture(&state, ObjId(99), Some(ObjId(10)), ObjId(10), 42)
            .expect("self loop")
            .events()
            .is_empty()
    );
    assert!(legacy_events(&state, ObjId(99), Some(ObjId(10)), ObjId(10), 42).is_empty());
    assert!(
        capture(&state, ObjId(99), None, ObjId(999), 42)
            .expect("missing scene")
            .events()
            .is_empty()
    );
    for status in [
        SceneStatus::Preparing,
        SceneStatus::Paused,
        SceneStatus::Closed,
    ] {
        for scene in state.scenes.values_mut() {
            scene.status = status;
        }
        assert!(planned(&state).events().is_empty());
        assert!(legacy_events(&state, ObjId(99), Some(ObjId(10)), ObjId(11), 1234).is_empty());
    }
}

#[test]
fn first_paused_holding_scene_masks_later_active_scene_as_before() {
    let mut state = state();
    let first = state.scenes.get_mut(&SceneUid(1)).expect("first");
    first.status = SceneStatus::Paused;
    first.pause_reason = Some(PauseReason::ServerRestart);
    state.scenes.insert(SceneUid(3), scene(3, ObjId(10)));
    let capture = planned(&state);
    assert_eq!(capture.events().len(), 1);
    assert_eq!(capture.events()[0].event().kind, SceneEventKind::Joined);
    state.scenes.get_mut(&SceneUid(1)).expect("first").status = SceneStatus::Closed;
    assert_eq!(planned(&state).events()[1].scene(), SceneUid(3));
}

#[test]
fn witness_bounds_are_inclusive_and_do_not_change_legacy_observers() {
    let mut state = state();
    let destination = state.scenes.get_mut(&SceneUid(2)).expect("destination");
    destination.participants = (0..128).map(ObjId).collect();
    destination.observers = (128..256).map(ObjId).collect();
    // Large unrelated scene fields are never cloned into a movement event.
    destination.title = "x".repeat(MAX_MOVEMENT_CAPTURE_BYTES * 2);
    let exact = planned(&state);
    assert_eq!(
        exact.events()[0].event().witnesses.len(),
        MAX_MOVEMENT_WITNESSES
    );
    assert_eq!(
        MovementCapture::decode(&exact.encode().expect("encode")).expect("decode"),
        exact
    );
    state
        .scenes
        .get_mut(&SceneUid(2))
        .expect("destination")
        .observers
        .push(ObjId(999));
    assert_eq!(
        capture(&state, ObjId(99), Some(ObjId(10)), ObjId(11), 1234),
        Err(CaptureError::Limit)
    );
    assert_eq!(
        legacy_events(&state, ObjId(99), Some(ObjId(10)), ObjId(11), 1234)[0]
            .event()
            .witnesses
            .len(),
        257
    );
}

#[test]
fn scene_map_is_bounded_before_lookup_but_legacy_and_self_loop_are_unchanged() {
    let mut state = state();
    for id in 3..=MAX_MOVEMENT_CAPTURE_SCENES as u64 {
        state.scenes.insert(SceneUid(id), scene(id, ObjId(1000)));
    }
    assert_eq!(state.scenes.len(), MAX_MOVEMENT_CAPTURE_SCENES);
    assert_eq!(planned(&state).events().len(), 2);
    let id = MAX_MOVEMENT_CAPTURE_SCENES as u64 + 1;
    state.scenes.insert(SceneUid(id), scene(id, ObjId(1000)));
    assert_eq!(
        capture(&state, ObjId(99), Some(ObjId(10)), ObjId(11), 1234),
        Err(CaptureError::Limit)
    );
    assert_eq!(
        legacy_events(&state, ObjId(99), Some(ObjId(10)), ObjId(11), 1234).len(),
        2
    );
    assert!(
        capture(&state, ObjId(99), Some(ObjId(10)), ObjId(10), 1234)
            .expect("self loop")
            .events()
            .is_empty()
    );
}

#[test]
fn captured_facts_survive_later_scene_and_clock_changes() {
    let mut state = state();
    let capture = planned(&state);
    let original = capture.encode().expect("capture");
    for scene in state.scenes.values_mut() {
        scene.participants.clear();
        scene.observers.clear();
        scene.round = 50;
        scene.ic_floor = 999999;
        scene.status = SceneStatus::Closed;
    }
    assert_eq!(capture.encode().expect("unchanged"), original);
    assert_eq!(
        MovementCapture::decode(&original).expect("decode").events()[0]
            .event()
            .at,
        1234
    );
    assert!(planned(&state).events().is_empty());
}

#[test]
fn strict_codec_requires_all_fields_and_explicit_null_source() {
    let capture = capture(&state(), ObjId(99), None, ObjId(11), 1234).expect("unlocated");
    let value = serde_json::to_value(&capture).expect("JSON");
    assert_eq!(value["from"], Value::Null);
    assert_eq!(
        MovementCapture::decode(&canonical(&value)).expect("decode"),
        capture
    );
    for key in ["format", "actor", "from", "to", "at", "events"] {
        let mut changed = value.clone();
        changed.as_object_mut().expect("object").remove(key);
        assert!(
            MovementCapture::decode(&canonical(&changed)).is_err(),
            "missing {key}"
        );
    }
    for key in ["kind", "actor", "ic_time", "at", "witnesses", "detail"] {
        let mut changed = value.clone();
        changed["events"][0]["event"]
            .as_object_mut()
            .expect("event")
            .remove(key);
        assert!(
            MovementCapture::decode(&canonical(&changed)).is_err(),
            "missing event {key}"
        );
    }
    for key in ["scene", "event"] {
        let mut changed = value.clone();
        changed["events"][0]
            .as_object_mut()
            .expect("row")
            .remove(key);
        assert!(
            MovementCapture::decode(&canonical(&changed)).is_err(),
            "missing row {key}"
        );
    }
    let mut changed = value;
    changed["events"][0]["event"]["actor"] = Value::Null;
    assert!(MovementCapture::decode(&canonical(&changed)).is_err());
}

#[test]
fn strict_codec_rejects_unknown_data_and_forged_event_shape() {
    let original = serde_json::to_value(planned(&state())).expect("JSON");
    for target in ["root", "row", "event", "detail"] {
        let mut changed = original.clone();
        let object = match target {
            "root" => &mut changed,
            "row" => &mut changed["events"][0],
            "event" => &mut changed["events"][0]["event"],
            _ => &mut changed["events"][0]["event"]["detail"],
        };
        object["unexpected"] = json!(true);
        assert!(
            MovementCapture::decode(&canonical(&changed)).is_err(),
            "unknown {target}"
        );
    }
    for (field, value) in [
        ("kind", json!("said")),
        ("actor", json!(98)),
        ("at", json!(1235)),
        ("detail", Value::Null),
    ] {
        let mut changed = original.clone();
        changed["events"][0]["event"][field] = value;
        assert!(
            MovementCapture::decode(&canonical(&changed)).is_err(),
            "changed {field}"
        );
    }
    let mut reversed = original.clone();
    reversed["events"].as_array_mut().expect("events").reverse();
    assert!(MovementCapture::decode(&canonical(&reversed)).is_err());
    let mut self_loop = original.clone();
    self_loop["from"] = self_loop["to"].clone();
    assert!(MovementCapture::decode(&canonical(&self_loop)).is_err());
    let mut no_source = original.clone();
    no_source["from"] = Value::Null;
    assert!(MovementCapture::decode(&canonical(&no_source)).is_err());
    let mut duplicate_scene = original.clone();
    duplicate_scene["events"][1]["scene"] = duplicate_scene["events"][0]["scene"].clone();
    assert!(MovementCapture::decode(&canonical(&duplicate_scene)).is_err());
    let mut wrong_version = original;
    wrong_version["format"] = json!(2);
    assert!(MovementCapture::decode(&canonical(&wrong_version)).is_err());
}

#[test]
fn codec_enforces_event_witness_byte_and_canonical_bounds() {
    let original = serde_json::to_value(planned(&state())).expect("JSON");
    let mut too_many = original.clone();
    let row = too_many["events"][0].clone();
    too_many["events"].as_array_mut().expect("events").push(row);
    assert_eq!(
        MovementCapture::decode(&canonical(&too_many)),
        Err(CaptureError::Limit)
    );
    let mut witnesses = original.clone();
    witnesses["events"][0]["event"]["witnesses"] = json!(vec![42; MAX_MOVEMENT_WITNESSES + 1]);
    assert_eq!(
        MovementCapture::decode(&canonical(&witnesses)),
        Err(CaptureError::Limit)
    );
    let bytes = canonical(&original);
    let mut padded = bytes.clone();
    padded.resize(MAX_MOVEMENT_CAPTURE_BYTES, b' ');
    assert_eq!(MovementCapture::decode(&padded), Err(CaptureError::Invalid));
    padded.push(b' ');
    assert_eq!(MovementCapture::decode(&padded), Err(CaptureError::Limit));
    let mut noncanonical = bytes;
    noncanonical.push(b'\n');
    assert_eq!(
        MovementCapture::decode(&noncanonical),
        Err(CaptureError::Invalid)
    );
    let duplicate = br#"{"format":1,"format":1,"actor":1,"from":null,"to":2,"at":3,"events":[]}"#;
    assert!(MovementCapture::decode(duplicate).is_err());
}
