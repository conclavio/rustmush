#![cfg(test)]

use super::*;
use crate::{SceneEvent, SceneEventKind, SceneMode, SceneRecord, SceneStatus, SceneUid};
use mush_core::{ClockRef, ClockStamp, GOD, Ledger};
use serde_json::json;

fn scene(id: u64, anchor: ObjId) -> SceneRecord {
    SceneRecord {
        id: SceneUid(id),
        title: "Transcript".into(),
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
    destination.observers = vec![ObjId(22), ObjId(20)];
    state.scenes.insert(SceneUid(2), destination);
    state
}

fn prepare(state: &ScenesState) -> EventsPatch {
    EventsPatch::prepare(state, ObjId(99), Some(ObjId(10)), ObjId(11), 1234)
        .expect("bounded genuine scene events")
}

fn append(ledger: &mut Ledger, id: &str, value: serde_json::Value) {
    let prepared = ledger.prepare_append(id, value).expect("fixture append");
    ledger
        .apply_prepared(prepared)
        .expect("unchanged fixture tail");
}

#[test]
fn two_original_events_and_sequence_refs_bind_without_early_publication() {
    let mut state = state();
    append(&mut state.events, "1", json!("previous source"));
    append(&mut state.events, "2", json!("previous destination"));
    append(&mut state.events, "2", json!("another destination"));
    let before = state.clone();
    let patch = prepare(&state);
    assert_eq!(
        patch.capture.events(),
        movement::legacy_events(&state, ObjId(99), Some(ObjId(10)), ObjId(11), 1234)
    );
    assert_eq!(
        patch
            .references
            .iter()
            .map(|r| (r.scene, r.sequence))
            .collect::<Vec<_>>(),
        vec![(2, 2), (1, 1)]
    );
    let writes = patch.writes().expect("exact original writes");
    assert_eq!(writes.len(), 2);
    for ((planned, reference), write) in patch
        .capture
        .events()
        .iter()
        .zip(&patch.references)
        .zip(&writes)
    {
        let value = serde_json::to_value(planned.event()).expect("event");
        assert_eq!(
            reference.event_digest,
            scene_transition::scene_event_digest(&value).expect("digest")
        );
        assert!(matches!(write, ReplayWrite::LedgerChunkSet { write }
            if write.log_id() == reference.scene.to_string()
                && write.chunk().entries.last().is_some_and(|entry|
                    entry.seq == reference.sequence && entry.value == value)));
    }
    assert_eq!(state, before);
    let bound = patch.bind(&mut state).expect("bind both exact tails");
    drop(bound);
    assert_eq!(state, before);
    let patch = prepare(&state);
    let outputs = patch.bind(&mut state).expect("bind").apply();
    assert_eq!(
        outputs.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(),
        vec![2, 1]
    );
    assert_eq!(state.scenes, before.scenes);
    let next = prepare(&state);
    assert_eq!(
        next.references
            .iter()
            .map(|r| (r.scene, r.sequence))
            .collect::<Vec<_>>(),
        vec![(2, 3), (1, 2)]
    );
}

#[test]
fn empty_capture_has_no_fake_event_or_ledger_dependency() {
    let mut state = ScenesState::default();
    append(
        &mut state.events,
        "999",
        json!("x".repeat(MAX_MOVEMENT_RESIDENT_LOG_BYTES + 1)),
    );
    let patch = prepare(&state);
    assert!(patch.capture.events().is_empty());
    assert!(patch.references.is_empty());
    assert!(patch.writes().expect("empty writes").is_empty());
    let before = state.events.clone();
    assert!(
        patch
            .bind(&mut state)
            .expect("negative observation")
            .apply()
            .is_empty()
    );
    assert_eq!(state.events, before);
    let loop_patch = EventsPatch::prepare(&state, ObjId(99), Some(ObjId(11)), ObjId(11), 1234)
        .expect("self loop");
    assert!(loop_patch.writes().expect("empty writes").is_empty());
}

#[test]
fn negative_capture_refuses_a_new_scene_at_recheck_and_late_bind() {
    let mut state = ScenesState::default();
    let patch = prepare(&state);
    state.scenes.insert(SceneUid(2), scene(2, ObjId(11)));
    assert!(patch.recheck(&state).is_err());
    let before = state.events.clone();
    assert!(patch.bind(&mut state).is_err());
    assert_eq!(state.events, before);
}

#[test]
fn paused_mask_is_a_real_negative_selection_fence() {
    let mut state = ScenesState::default();
    let mut mask = scene(1, ObjId(11));
    mask.status = SceneStatus::Paused;
    state.scenes.insert(SceneUid(1), mask);
    state.scenes.insert(SceneUid(2), scene(2, ObjId(11)));
    let patch = prepare(&state);
    assert!(patch.capture.events().is_empty());
    state.scenes.get_mut(&SceneUid(1)).expect("mask").status = SceneStatus::Closed;
    assert!(patch.recheck(&state).is_err());
    assert!(patch.bind(&mut state).is_err());
    assert_eq!(state.events, Ledger::new());
}

#[test]
fn stale_second_log_cannot_publish_the_first_event() {
    let mut state = state();
    let patch = prepare(&state);
    // Joined is first; a source transcript entry invalidates the second tail.
    append(&mut state.events, "1", json!("intervening source speech"));
    let before = state.events.clone();
    assert!(patch.recheck(&state).is_err());
    assert!(patch.bind(&mut state).is_err());
    assert_eq!(state.events, before);
    assert_eq!(prepare(&state).references[0].sequence, 0);
}

#[test]
fn changed_time_witnesses_or_status_refuse_but_unrelated_fields_do_not() {
    for mutation in 0..3 {
        let mut state = state();
        let patch = prepare(&state);
        let selected = state.scenes.get_mut(&SceneUid(2)).expect("destination");
        match mutation {
            0 => selected.ic_floor += 1,
            1 => selected.observers.push(ObjId(23)),
            _ => selected.status = SceneStatus::Paused,
        }
        assert!(patch.recheck(&state).is_err());
        assert!(patch.bind(&mut state).is_err());
        assert_eq!(state.events, Ledger::new());
    }
    let mut state = state();
    let patch = prepare(&state);
    state
        .scenes
        .get_mut(&SceneUid(2))
        .expect("destination")
        .title = "unrelated".repeat(8192);
    assert!(patch.recheck(&state).is_ok());
    assert_eq!(
        patch
            .bind(&mut state)
            .expect("irrelevant prose does not stale events")
            .apply()
            .len(),
        2
    );
}

#[test]
fn selected_tail_and_capture_limits_refuse_before_any_append() {
    let mut oversized = state();
    // The default threshold would seal this entry and discard its resident
    // payload. Keep the chunk open so the selected-tail bound is exercised.
    oversized.events = Ledger::with_chunk_target(MAX_MOVEMENT_RESIDENT_LOG_BYTES * 2);
    append(
        &mut oversized.events,
        "1",
        json!("x".repeat(MAX_MOVEMENT_RESIDENT_LOG_BYTES + 1)),
    );
    assert!(EventsPatch::prepare(&oversized, ObjId(99), Some(ObjId(10)), ObjId(11), 1234).is_err());
    let mut state = state();
    let patch = prepare(&state);
    state
        .scenes
        .get_mut(&SceneUid(2))
        .expect("destination")
        .observers = vec![ObjId(20); movement::MAX_MOVEMENT_WITNESSES + 1];
    assert!(patch.recheck(&state).is_err());
    assert!(patch.bind(&mut state).is_err());
    assert_eq!(state.events, Ledger::new());
}

#[test]
fn adapter_accepts_only_canonical_scene_ledger_writes() {
    let participant = ScenesMovementParticipant;
    let patch = prepare(&state());
    assert!(
        patch
            .writes()
            .expect("writes")
            .iter()
            .all(|write| participant.accepts_write(write))
    );
    let ledger = Ledger::new();
    for (owner, id) in [("scenes", "01"), ("scenes", "not_a_scene"), ("sim", "1")] {
        let prepared = ledger
            .prepare_append(
                id,
                serde_json::to_value(SceneEvent::new(
                    &scene(1, ObjId(10)),
                    1234,
                    SceneEventKind::Left,
                    Some(ObjId(99)),
                    json!({}),
                ))
                .expect("event"),
            )
            .expect("append");
        let write = prepared
            .authoritative_replay_write(OwnerId::new(owner).expect("owner"))
            .expect("write");
        assert!(!participant.accepts_write(&write));
    }
}
