#![cfg(test)]
//! Selected transcript machinery only; no fake Core or physical owner receipt.

use super::*;
use crate::{SceneEvent, SceneEventKind, SceneMode, SceneRecord, SceneStatus, SceneUid};
use mush_core::{ClockRef, ClockStamp, GOD, ObjId};
use serde_json::json;

fn state() -> ScenesState {
    let mut state = ScenesState::default();
    for (id, anchor) in [(1, 10), (2, 11)] {
        let scene: SceneRecord =
            serde_json::from_value(json!({"id":id,"title":"Authored transcript fixture",
            "kind":"timestop","mode":SceneMode::TurnBased,"status":SceneStatus::Active,
            "exclusive":true,"anchor":anchor,"participants":[20,21],"observers":[22],
            "facilitator":GOD,"created_by":GOD,"created_at":100,
            "ic_started_at":ClockStamp{clock:ClockRef::WorldIc{generation:1},at:500},
            "round_seconds":6,"round":3,"turn_index":0}))
            .expect("authored scene");
        state.scenes.insert(SceneUid(id), scene);
    }
    state
}
fn prepare(state: &ScenesState) -> WindowEvents {
    let capture = movement_window_capture::capture(
        state,
        ObjId(99),
        Some(ObjId(10)),
        ObjId(11),
        ObjId(10),
        1234,
    )
    .expect("two original legs");
    WindowEvents::prepare(state, capture).expect("ordered tails")
}

#[test]
fn four_original_events_bind_without_a_prefix_and_keep_distinct_repeated_positions() {
    let mut state = state();
    let prepared = prepare(&state);
    let value = proof::make(&prepared.capture, &prepared.sequence).expect("proof");
    let claims =
        proof::decode_claims(&value, Some(ObjId(10)), ObjId(11), ObjId(10)).expect("claims");
    assert_eq!(claims.leg_counts, vec![2, 2]);
    assert_eq!(
        claims
            .appends
            .iter()
            .map(|r| (r.scene, r.sequence))
            .collect::<Vec<_>>(),
        vec![(2, 0), (1, 0), (1, 1), (2, 1)]
    );
    assert_eq!(
        prepared
            .sequence
            .authoritative_replay_writes(OwnerId::new(NS).expect("owner"))
            .expect("writes")
            .len(),
        2
    );
    drop(prepared.bind(&mut state).expect("bind"));
    assert_eq!(state.events.next_seq("1"), 0);
    assert_eq!(state.events.next_seq("2"), 0);
    let prepared = prepare(&state);
    let expected: Vec<_> = prepared
        .capture
        .events()
        .map(|event| (event.scene(), event.event().clone()))
        .collect();
    let writes = prepared.bind(&mut state).expect("bind").apply();
    assert_eq!(writes.len(), 2);
    for id in [SceneUid(1), SceneUid(2)] {
        let actual: Vec<SceneEvent> = state
            .events
            .open_entries(&id.to_string())
            .iter()
            .map(|entry| serde_json::from_value(entry.value.clone()).expect("event"))
            .collect();
        assert_eq!(
            actual,
            expected
                .iter()
                .filter(|(scene, _)| *scene == id)
                .map(|(_, event)| event.clone())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn changed_second_tail_or_original_scene_selection_refuses_every_ordered_event() {
    for record in [false, true] {
        let mut state = state();
        let prepared = prepare(&state);
        if record {
            state.scenes.get_mut(&SceneUid(2)).expect("scene").status = SceneStatus::Paused;
        } else {
            let event = SceneEvent::new(
                state.scenes.get(&SceneUid(1)).expect("scene"),
                1235,
                SceneEventKind::Said,
                Some(ObjId(99)),
                json!({}),
            );
            let append = state
                .events
                .prepare_append("1", serde_json::to_value(event).expect("event"))
                .expect("late speech");
            state.events.apply_prepared(append).expect("speech");
        }
        let before = state.events.clone();
        assert!(prepared.bind(&mut state).is_err());
        assert_eq!(state.events, before);
        assert_eq!(state.events.next_seq("2"), 0);
    }
}

#[test]
fn ordered_proof_refuses_lost_legs_repeated_positions_gaps_and_noncanonical_data() {
    let state = state();
    let prepared = prepare(&state);
    let original = proof::make(&prepared.capture, &prepared.sequence).expect("proof");
    for fault in 0..8 {
        let mut value = original.clone();
        match fault {
            0 => value["format"] = json!(1),
            1 => value["leg_counts"] = json!([2]),
            2 => value["leg_counts"] = json!([3, 1]),
            3 => value["appends"][2]["sequence"] = json!(0),
            4 => value["appends"][2]["sequence"] = json!(2),
            5 => value["appends"][1]["scene"] = json!(2),
            6 => value["extra"] = json!(true),
            _ => {
                value.as_object_mut().expect("object").remove("leg_counts");
            }
        }
        assert!(
            proof::decode_claims(&value, Some(ObjId(10)), ObjId(11), ObjId(10)).is_err(),
            "fault {fault}"
        );
    }
    assert!(proof::decode_claims(&original, None, ObjId(11), ObjId(10)).is_err());
    assert!(proof::decode_claims(&original, Some(ObjId(11)), ObjId(11), ObjId(10)).is_err());
    let mut zero = original;
    zero["appends"][0]["event_digest"] =
        serde_json::to_value(mush_core::DefinitionDigest::from_bytes([0; 32])).expect("digest");
    assert!(proof::decode_claims(&zero, Some(ObjId(10)), ObjId(11), ObjId(10)).is_err());
}
