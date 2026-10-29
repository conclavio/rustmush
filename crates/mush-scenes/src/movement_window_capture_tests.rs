#![cfg(test)]
//! Transcript data and actual legacy observer equivalence, not physical proof.

use super::*;
use crate::{
    SceneEvent, SceneEventKind, SceneMode, SceneRecord, SceneStatus, SceneUid, ScenesModule,
};
use mush_core::{ClockRef, ClockStamp, Effects, GOD, ObjKind, Outbox, WorldEvent};
use serde_json::{Value, json};

fn scene(id: u64, anchor: ObjId) -> SceneRecord {
    serde_json::from_value(json!({"id":id,"title":"Authored transcript fixture",
        "kind":"timestop","mode":SceneMode::TurnBased,"status":SceneStatus::Active,
        "exclusive":true,"anchor":anchor,"participants":[20,21],"observers":[22],
        "facilitator":GOD,"created_by":GOD,"created_at":100,
        "ic_started_at":ClockStamp{clock:ClockRef::WorldIc{generation:1},at:500},
        "round_seconds":6,"round":3,"turn_index":0,"ic_floor":id*200}))
    .expect("authored scene record")
}
fn state() -> ScenesState {
    let mut state = ScenesState::default();
    for (id, room) in [(1, 10), (2, 11), (3, 12)] {
        state.scenes.insert(SceneUid(id), scene(id, ObjId(room)));
    }
    let middle = state.scenes.get_mut(&SceneUid(2)).expect("middle scene");
    middle.participants = vec![ObjId(31), ObjId(30)];
    middle.observers = vec![ObjId(32), ObjId(31)];
    state
}
fn planned(state: &ScenesState, to: ObjId) -> WindowSceneCapture {
    capture(state, ObjId(99), Some(ObjId(10)), ObjId(11), to, 1234).expect("ordered capture")
}
fn canonical(value: &Value) -> Vec<u8> {
    canonical_json_bytes_with_limit(value, MAX_WINDOW_SCENE_CAPTURE_BYTES).expect("bounded JSON")
}

#[test]
fn original_legs_keep_intermediate_events_clocks_witnesses_and_loop_order() {
    let state = state();
    for to in [ObjId(12), ObjId(10)] {
        let capture = planned(&state, to);
        assert_eq!(capture.legs().len(), 2);
        let expected: Vec<_> =
            crate::movement::legacy_events(&state, ObjId(99), Some(ObjId(10)), ObjId(11), 1234)
                .into_iter()
                .chain(crate::movement::legacy_events(
                    &state,
                    ObjId(99),
                    Some(ObjId(11)),
                    to,
                    1234,
                ))
                .collect();
        assert_eq!(
            capture.events().collect::<Vec<_>>(),
            expected.iter().collect::<Vec<_>>()
        );
        assert_eq!(expected.len(), 4);
        assert_eq!(
            expected[0].event().witnesses,
            vec![ObjId(31), ObjId(30), ObjId(32), ObjId(31)]
        );
        assert_eq!(expected[0].event().ic_time, 512);
        assert_eq!(expected[3].event().ic_time, 512);
        let bytes = capture.encode().expect("encode");
        let decoded = WindowSceneCapture::decode(&bytes).expect("strict decode");
        assert_eq!(decoded, capture);
        assert_eq!(
            decoded.digest().expect("digest"),
            capture.digest().expect("digest")
        );
        decoded.recheck(&state).expect("original owner data");
    }
    assert!(state.events.open_entries("1").is_empty());
    assert!(state.events.open_entries("2").is_empty());
}

#[test]
fn actual_registered_observer_matches_both_legs_even_when_returning_to_source() {
    let mut world = mush_testkit::TestWorld::with_modules(vec![Box::new(ScenesModule::new())]);
    let source = world
        .world_mut()
        .create(ObjKind::Room, "source", GOD, None)
        .expect("source");
    let middle = world
        .world_mut()
        .create(ObjKind::Room, "middle", GOD, None)
        .expect("middle");
    let actor = world
        .world_mut()
        .create(ObjKind::Player, "walker", GOD, Some(source))
        .expect("actor");
    world.world_mut().set_now(1234);
    let state = world
        .world_mut()
        .module_state_mut::<ScenesState>()
        .expect("actual Scenes slot");
    state.scenes.insert(SceneUid(1), scene(1, source));
    state.scenes.insert(SceneUid(2), scene(2, middle));
    let expected = capture(state, actor, Some(source), middle, source, 1234).expect("loop capture");
    world.take_module_ops();
    // Actual legacy movement and the registered Scenes observer establish
    // transcript equivalence. They mint no closed movement/personnel receipt.
    for (from, to) in [(source, middle), (middle, source)] {
        world
            .world_mut()
            .move_to(actor, to)
            .expect("legacy Core move");
        world.engine_mut().emit_world_event(
            &WorldEvent::Moved {
                who: actor,
                from: Some(from),
                to,
            },
            &mut Outbox::default(),
            &mut Effects::default(),
        );
    }
    let state = world
        .world()
        .module_state::<ScenesState>()
        .expect("actual Scenes slot");
    for id in [SceneUid(1), SceneUid(2)] {
        let actual: Vec<SceneEvent> = state
            .events
            .open_entries(&id.to_string())
            .iter()
            .map(|entry| serde_json::from_value(entry.value.clone()).expect("original event"))
            .collect();
        let planned: Vec<_> = expected
            .events()
            .filter(|row| row.scene() == id)
            .map(|row| row.event().clone())
            .collect();
        assert_eq!(actual, planned);
        assert_eq!(actual.len(), 2);
    }
    assert_eq!(world.take_module_ops().len(), 4);
}

#[test]
fn strict_decoder_refuses_disconnected_missing_extra_or_reordered_legs() {
    let original = serde_json::to_value(planned(&state(), ObjId(12))).expect("value");
    for fault in 0..10 {
        let mut value = original.clone();
        match fault {
            0 => value["format"] = json!(2),
            1 => value["actor"] = json!(100),
            2 => {
                value.as_object_mut().expect("object").remove("source");
            }
            3 => value["admitted_room"] = json!(42),
            4 => value["projected_room"] = json!(11),
            5 => value["legs"] = json!([]),
            6 => value["legs"].as_array_mut().expect("legs").swap(0, 1),
            7 => value["legs"][1]["at"] = json!(999),
            8 => value["extra"] = json!(true),
            _ => value["legs"][0]["extra"] = json!(true),
        }
        assert!(
            WindowSceneCapture::decode(&canonical(&value)).is_err(),
            "fault {fault}"
        );
    }
    let mut too_many = original.clone();
    let extra = too_many["legs"][0].clone();
    too_many["legs"].as_array_mut().expect("legs").push(extra);
    assert_eq!(
        WindowSceneCapture::decode(&canonical(&too_many)),
        Err(CaptureError::Limit)
    );
    let mut noncanonical = vec![b' '];
    noncanonical.extend(canonical(&original));
    assert_eq!(
        WindowSceneCapture::decode(&noncanonical),
        Err(CaptureError::Invalid)
    );
    assert_eq!(
        WindowSceneCapture::decode(&vec![b' '; MAX_WINDOW_SCENE_CAPTURE_BYTES + 1]),
        Err(CaptureError::Limit)
    );
}

#[test]
fn valid_passive_data_is_not_authentication_and_real_second_leg_drift_refuses() {
    let mut state = state();
    let capture = planned(&state, ObjId(12));
    let mut changed = serde_json::to_value(&capture).expect("value");
    changed["legs"][1]["events"][0]["event"]["witnesses"] = json!([123]);
    let decoded =
        WindowSceneCapture::decode(&canonical(&changed)).expect("valid hypothetical data");
    assert_ne!(
        decoded.digest().expect("digest"),
        capture.digest().expect("digest")
    );
    assert_eq!(decoded.recheck(&state), Err(CaptureError::Invalid));
    state
        .scenes
        .get_mut(&SceneUid(3))
        .expect("final scene")
        .ic_floor += 1;
    assert_eq!(capture.recheck(&state), Err(CaptureError::Invalid));
}

#[test]
fn bounded_second_leg_and_noop_leg_preserve_existing_capture_rules() {
    let mut state = state();
    let same = capture(
        &state,
        ObjId(99),
        Some(ObjId(10)),
        ObjId(11),
        ObjId(11),
        1234,
    )
    .expect("one leg");
    assert_eq!(same.legs().len(), 1);
    assert_eq!(same.events().count(), 2);
    let noop = capture(
        &state,
        ObjId(99),
        Some(ObjId(10)),
        ObjId(10),
        ObjId(11),
        1234,
    )
    .expect("noop then move");
    assert!(noop.legs()[0].events().is_empty());
    assert_eq!(noop.legs()[1].events().len(), 2);
    state
        .scenes
        .get_mut(&SceneUid(3))
        .expect("final")
        .participants = vec![ObjId(1); 257];
    assert_eq!(
        capture(
            &state,
            ObjId(99),
            Some(ObjId(10)),
            ObjId(11),
            ObjId(12),
            1234
        ),
        Err(CaptureError::Limit)
    );
    state.scenes.get_mut(&SceneUid(3)).expect("final").status = SceneStatus::Paused;
    let held = planned(&state, ObjId(12));
    assert_eq!(held.events().count(), 3);
    assert!(state.events.open_entries("1").is_empty());
    assert_eq!(
        held.events().last().expect("middle departure").event().kind,
        SceneEventKind::Left
    );
}
