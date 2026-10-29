#![cfg(test)]

use super::*;
use crate::geom::{Point2, PointCm, RectCm};
use crate::record::{Access, Footprint, RoomGeo, RoomKind, SurfaceRef};
use crate::window::{Composite, WindowEntry, WindowFrame};
use mush_core::{CanonicalId, GOD, LIMBO};
use mush_sim::{DomainId, Pose, WindowOpener};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const ACTOR: ObjId = ObjId(90);
const FROM: ObjId = ObjId(10);
const TO: ObjId = ObjId(11);

fn key(name: &str) -> RecordKey {
    RecordKey::new(
        CanonicalId::new("test").expect("pack"),
        CanonicalId::new(name).expect("entry"),
    )
}

fn state() -> AtlasState {
    let mut state = AtlasState::default();
    state.records.roomgeo.insert(
        TO,
        RoomGeo {
            room: TO,
            key: key("room"),
            neighborhood: key("district"),
            site: None,
            surface: SurfaceRef::Ground,
            footprint: Footprint::Anchor {
                point: Point2(0.0, 0.0),
                radius_m: 1.0,
            },
            kind: RoomKind::Indoor,
            access: Access::Restricted,
            revision: 1,
        },
    );
    state
}

fn observed(
    state: Option<&AtlasState>,
    kind: Option<ObjKind>,
    placement: SimPlacementObservation,
) -> MovementCapture {
    capture_observed(state, kind, placement, ACTOR, Some(FROM), TO, 1234).expect("bounded capture")
}

fn player(state: &AtlasState) -> MovementCapture {
    observed(
        Some(state),
        Some(ObjKind::Player),
        SimPlacementObservation::NoRecord,
    )
}

fn window(domain: u64, covers: bool, drawn: bool) -> WindowEntry {
    let frame = WindowFrame {
        region: key("region"),
        surfaces: vec![SurfaceRef::Ground],
        bbox: RectCm {
            min: PointCm { x: 0, y: 0 },
            max: PointCm { x: 100, y: 100 },
        },
        cell_cm: 100,
    };
    WindowEntry {
        domain,
        frame: frame.clone(),
        anchors: vec![],
        rooms: if covers {
            BTreeSet::from([TO])
        } else {
            BTreeSet::new()
        },
        opened_by: WindowOpener::Staff { by: GOD },
        revision: 7,
        composite: drawn.then(|| Composite {
            frame,
            surfaces: vec![],
            pins: vec![],
            features_seen: 0,
            digest: "opaque and unused by selection".into(),
        }),
        pending: None,
    }
}

fn canonical(value: &Value) -> Vec<u8> {
    canonical_json_bytes_with_limit(value, MAX_MOVEMENT_CAPTURE_BYTES).expect("canonical fixture")
}

#[test]
fn discovery_proposes_the_original_document_without_mutating_owner_state() {
    let state = state();
    let captured = player(&state);
    assert_eq!(captured.actor(), ACTOR);
    assert_eq!(captured.from(), Some(FROM));
    assert_eq!(captured.to(), TO);
    assert_eq!(captured.at(), 1234);
    assert_eq!(captured.atlas_slot(), SlotPresence::Present);
    assert_eq!(
        captured.planned_known(),
        Some(KnownDoc {
            places: vec![key("district")]
        })
    );
    assert!(matches!(
        captured.discovery(),
        Discovery::Mapped {
            known_before: None,
            change: LearnChange::Append { evict_oldest: 0 },
            ..
        }
    ));
    assert!(state.travel.known.is_empty());
    assert!(state.windows.open.is_empty());
    assert!(
        state.records.neighborhoods.is_empty(),
        "legacy does not require the mapped neighborhood record"
    );
}

#[test]
fn repeated_visits_do_not_reorder_or_write_and_new_visits_evict_oldest() {
    let mut state = state();
    state
        .travel
        .known
        .insert(ACTOR, vec![key("district"), key("later")]);
    assert!(player(&state).planned_known().is_none());
    assert_eq!(
        state.travel.known.get(&ACTOR),
        Some(&vec![key("district"), key("later")])
    );
    let before: Vec<_> = (0..MAX_KNOWN_PLACES)
        .map(|n| key(&format!("place{n}")))
        .collect();
    state.travel.known.insert(ACTOR, before.clone());
    let captured = player(&state);
    let after = captured.planned_known().expect("new document");
    assert_eq!(after.places.len(), MAX_KNOWN_PLACES);
    assert_eq!(after.places.first(), before.get(1));
    assert_eq!(after.places.last(), Some(&key("district")));
    assert!(matches!(
        captured.discovery(),
        Discovery::Mapped {
            change: LearnChange::Append { evict_oldest: 1 },
            ..
        }
    ));
    assert_eq!(state.travel.known.get(&ACTOR), Some(&before));
}

#[test]
fn missing_slots_documents_and_mappings_remain_distinct() {
    let missing = observed(
        None,
        Some(ObjKind::Player),
        SimPlacementObservation::SlotAbsent,
    );
    assert_eq!(missing.discovery(), &Discovery::AtlasAbsent);
    assert_eq!(missing.atlas_slot(), SlotPresence::Absent);
    assert!(matches!(
        missing.window(),
        WindowObservation::Observed {
            placement: SimPlacementObservation::SlotAbsent,
            selection: WindowSelection::AtlasAbsent,
        }
    ));
    let empty = player(&AtlasState::default());
    assert_eq!(empty.discovery(), &Discovery::Unmapped);
    assert_ne!(empty, missing);
    let mut state = state();
    let absent_doc = player(&state);
    state.travel.known.insert(ACTOR, vec![]);
    let empty_doc = player(&state);
    assert_ne!(absent_doc.discovery(), empty_doc.discovery());
    assert_eq!(absent_doc.planned_known(), empty_doc.planned_known());
}

#[test]
fn only_players_learn_but_npcs_still_select_window_work() {
    let mut state = state();
    state.windows.open.insert(5, window(5, true, true));
    for kind in [
        None,
        Some(ObjKind::Room),
        Some(ObjKind::Exit),
        Some(ObjKind::Thing),
        Some(ObjKind::Npc),
    ] {
        let captured = observed(Some(&state), kind, SimPlacementObservation::NoRecord);
        assert_eq!(captured.discovery(), &Discovery::NotPlayer);
        assert!(captured.planned_known().is_none());
        if kind == Some(ObjKind::Npc) {
            assert_eq!(
                captured.window_effect(),
                WindowEffect::NeedsOwnerComposition {
                    release: None,
                    place_or_repose: Some(5)
                }
            );
        } else {
            assert_eq!(captured.window_effect(), WindowEffect::NotApplicable);
        }
    }
}

#[test]
fn self_loop_and_unlocated_event_sources_still_learn() {
    let state = state();
    for from in [None, Some(TO)] {
        let captured = capture_observed(
            Some(&state),
            Some(ObjKind::Player),
            SimPlacementObservation::NoRecord,
            ACTOR,
            from,
            TO,
            -1,
        )
        .expect("capture");
        assert_eq!(captured.from(), from);
        assert!(captured.planned_known().is_some());
        assert_eq!(
            MovementCapture::decode(&captured.encode().expect("encode")).expect("decode"),
            captured
        );
    }
}

#[test]
fn legacy_oversized_vectors_keep_the_old_duplicate_and_eviction_rules() {
    let known: Vec<_> = (0..MAX_KNOWN_PLACES + 7)
        .map(|n| key(&format!("place{n}")))
        .collect();
    let original = known.clone();
    assert!(plan_learn(&known, &key("place0")).is_none());
    assert_eq!(known, original, "duplicates neither trim nor reorder");
    let result = plan_learn(&known, &key("new")).expect("new key");
    assert_eq!(result.places.len(), MAX_KNOWN_PLACES);
    assert_eq!(result.places.first(), known.get(8));
    assert_eq!(result.places.last(), Some(&key("new")));
    let mut state = state();
    state.travel.known.insert(ACTOR, known);
    assert_eq!(
        capture_observed(
            Some(&state),
            Some(ObjKind::Player),
            SimPlacementObservation::NoRecord,
            ACTOR,
            Some(FROM),
            TO,
            0
        ),
        Err(CaptureError::Limit)
    );
}

#[test]
fn exact_key_byte_bound_is_checked_before_copying_the_known_document() {
    let long = RecordKey::new(
        CanonicalId::new("p".repeat(64)).expect("pack"),
        CanonicalId::new("e".repeat(64)).expect("entry"),
    );
    let mut state = state();
    state
        .records
        .roomgeo
        .get_mut(&TO)
        .expect("geo")
        .neighborhood = long.clone();
    state
        .travel
        .known
        .insert(ACTOR, vec![long.clone(); MAX_KNOWN_PLACES - 1]);
    assert!(player(&state).planned_known().is_none());
    state
        .travel
        .known
        .get_mut(&ACTOR)
        .expect("known")
        .push(long);
    assert_eq!(
        capture_observed(
            Some(&state),
            Some(ObjKind::Player),
            SimPlacementObservation::NoRecord,
            ACTOR,
            Some(FROM),
            TO,
            0
        ),
        Err(CaptureError::Limit)
    );
}

#[test]
fn map_release_entry_and_repose_are_pending_owner_work() {
    let mut state = state();
    let direct = SimPlacementObservation::Direct {
        domain: 4,
        revision: 9,
    };
    assert_eq!(
        player(&state).window_effect(),
        WindowEffect::NoSelectedEffect
    );
    assert_eq!(
        observed(Some(&state), Some(ObjKind::Player), direct).window_effect(),
        WindowEffect::NeedsOwnerComposition {
            release: Some(4),
            place_or_repose: None
        }
    );
    assert_eq!(
        observed(None, Some(ObjKind::Player), direct).window_effect(),
        WindowEffect::NeedsOwnerComposition {
            release: Some(4),
            place_or_repose: None
        }
    );
    state.windows.open.insert(4, window(4, true, true));
    assert_eq!(
        observed(Some(&state), Some(ObjKind::Player), direct).window_effect(),
        WindowEffect::NeedsOwnerComposition {
            release: None,
            place_or_repose: Some(4)
        }
    );
    state.windows.open.clear();
    state.windows.open.insert(5, window(5, true, true));
    assert_eq!(
        observed(Some(&state), Some(ObjKind::Player), direct).window_effect(),
        WindowEffect::NeedsOwnerComposition {
            release: Some(4),
            place_or_repose: Some(5)
        }
    );
}

#[test]
fn an_undrawn_first_covering_window_masks_a_later_drawn_match() {
    let mut state = state();
    state.windows.open.insert(1, window(1, false, true));
    state.windows.open.insert(2, window(2, true, false));
    state.windows.open.insert(3, window(3, true, true));
    let captured = player(&state);
    assert_eq!(captured.window_effect(), WindowEffect::NoSelectedEffect);
    assert_eq!(state.windows.drawn_covering_domain(TO), None);
    state.windows.open.remove(&2);
    assert_eq!(
        player(&state).window_effect(),
        WindowEffect::NeedsOwnerComposition {
            release: None,
            place_or_repose: Some(3)
        }
    );
    assert_eq!(state.windows.drawn_covering_domain(TO), Some(DomainId(3)));
}

#[test]
fn placement_observation_uses_the_actual_parent_tag_without_posture_copy() {
    assert_eq!(observe_placement(None), SimPlacementObservation::NoRecord);
    let mut record = PlacementRecord {
        object: ACTOR,
        parent: SpatialParent::Unplaced,
        revision: 13,
        unplaced: None,
    };
    assert_eq!(
        observe_placement(Some(&record)),
        SimPlacementObservation::Unplaced
    );
    record.parent = SpatialParent::Interior {
        space: TO,
        slot: None,
    };
    assert_eq!(
        observe_placement(Some(&record)),
        SimPlacementObservation::Interior
    );
    record.parent = SpatialParent::Attached {
        host: FROM,
        mount: mush_sim::MountId::new("hitch").expect("mount"),
        local_pose: None,
    };
    assert_eq!(
        observe_placement(Some(&record)),
        SimPlacementObservation::Attached
    );
    record.parent = SpatialParent::Direct {
        domain: DomainId(8),
        pose: Pose::at(
            1,
            2,
            3,
            mush_sim::passive_rules_value(json!({})).expect("posture"),
        ),
    };
    assert_eq!(
        observe_placement(Some(&record)),
        SimPlacementObservation::Direct {
            domain: 8,
            revision: 13
        }
    );
    for placement in [
        SimPlacementObservation::SlotAbsent,
        SimPlacementObservation::NoRecord,
        SimPlacementObservation::Unplaced,
        SimPlacementObservation::Interior,
        SimPlacementObservation::Attached,
    ] {
        let captured = observed(Some(&state()), Some(ObjKind::Npc), placement);
        assert_eq!(captured.window_effect(), WindowEffect::NoSelectedEffect);
        assert!(
            matches!(captured.window(), WindowObservation::Observed { placement: seen, .. } if *seen == placement)
        );
    }
}

#[test]
fn window_count_and_membership_bounds_refuse_without_a_partial_selection() {
    let mut state = state();
    for domain in 0..u64::try_from(MAX_MOVEMENT_WINDOWS).expect("count") {
        state
            .windows
            .open
            .insert(domain, window(domain, false, false));
    }
    assert_eq!(
        player(&state).window_effect(),
        WindowEffect::NoSelectedEffect
    );
    state.windows.open.insert(999, window(999, true, true));
    assert_eq!(
        capture_observed(
            Some(&state),
            Some(ObjKind::Player),
            SimPlacementObservation::NoRecord,
            ACTOR,
            None,
            TO,
            0
        ),
        Err(CaptureError::Limit)
    );
    state.windows.open.clear();
    let mut entry = window(1, false, false);
    entry.rooms = (0..=u32::try_from(MAX_MOVEMENT_WINDOW_ROOMS).expect("bound"))
        .map(ObjId)
        .collect();
    state.windows.open.insert(1, entry);
    assert_eq!(
        capture_observed(
            Some(&state),
            Some(ObjKind::Player),
            SimPlacementObservation::NoRecord,
            ACTOR,
            None,
            TO,
            0
        ),
        Err(CaptureError::Limit)
    );
    assert!(check_windows(std::iter::repeat_n(MAX_MOVEMENT_WINDOW_ROOMS, 16), 16).is_ok());
    assert_eq!(
        check_windows(std::iter::repeat_n(MAX_MOVEMENT_WINDOW_ROOMS, 17), 17),
        Err(CaptureError::Limit)
    );
    // A legacy-skipped nonactor never scans the oversized map collection.
    assert_eq!(
        observed(
            Some(&state),
            Some(ObjKind::Thing),
            SimPlacementObservation::NoRecord
        )
        .window_effect(),
        WindowEffect::NotApplicable
    );
}

#[test]
fn unrelated_large_geometry_or_composite_payload_is_not_captured() {
    let mut state = state();
    let mut entry = window(1, true, true);
    entry.composite.as_mut().expect("composite").digest =
        "x".repeat(MAX_MOVEMENT_CAPTURE_BYTES * 2);
    state.windows.open.insert(1, entry);
    let captured = player(&state);
    assert!(captured.encode().expect("encode").len() < 2048);
    assert_eq!(
        captured.window_effect(),
        WindowEffect::NeedsOwnerComposition {
            release: None,
            place_or_repose: Some(1)
        }
    );
}

#[test]
fn canonical_codec_refuses_missing_explicit_nulls_unknown_fields_and_whitespace() {
    let captured = capture_observed(
        None,
        None,
        SimPlacementObservation::SlotAbsent,
        ACTOR,
        None,
        TO,
        1234,
    )
    .expect("capture");
    let bytes = captured.encode().expect("encode");
    assert_eq!(
        MovementCapture::decode(&bytes).expect("round trip"),
        captured
    );
    let base = serde_json::to_value(&captured).expect("value");
    for field in ["from", "actor_kind"] {
        let mut value = base.clone();
        value.as_object_mut().expect("object").remove(field);
        assert!(MovementCapture::decode(&canonical(&value)).is_err());
    }
    let mut value = base.clone();
    value["extra"] = json!(0);
    assert!(MovementCapture::decode(&canonical(&value)).is_err());
    let mut value = base;
    value["format"] = json!(2);
    assert!(MovementCapture::decode(&canonical(&value)).is_err());
    let mut spaced = bytes;
    spaced.push(b' ');
    assert!(MovementCapture::decode(&spaced).is_err());
    assert_eq!(
        MovementCapture::decode(&vec![b' '; MAX_MOVEMENT_CAPTURE_BYTES + 1]),
        Err(CaptureError::Limit)
    );
}

#[test]
fn codec_rejects_inconsistent_discovery_and_nested_missing_fields() {
    let base = serde_json::to_value(player(&state())).expect("value");
    for field in ["known_before", "change", "neighborhood"] {
        let mut value = base.clone();
        value["discovery"]
            .as_object_mut()
            .expect("discovery")
            .remove(field);
        assert!(MovementCapture::decode(&canonical(&value)).is_err());
    }
    let mut value = base.clone();
    value["discovery"]["change"] = json!({"kind":"unchanged"});
    assert!(MovementCapture::decode(&canonical(&value)).is_err());
    let mut value = base.clone();
    value["atlas_slot"] = json!("absent");
    assert!(MovementCapture::decode(&canonical(&value)).is_err());
    let mut value = base;
    value["actor_kind"] = json!("Npc");
    assert!(MovementCapture::decode(&canonical(&value)).is_err());
}

#[test]
fn codec_rejects_forged_window_selection_order_and_counts() {
    let mut state = state();
    state.windows.open.insert(1, window(1, true, false));
    state.windows.open.insert(2, window(2, true, true));
    let base = serde_json::to_value(player(&state)).expect("value");
    let mut value = base.clone();
    value["window"]["selection"]["covering"] = json!(2);
    assert!(
        MovementCapture::decode(&canonical(&value)).is_err(),
        "undrawn first match masks second"
    );
    let mut value = base.clone();
    value["window"]["selection"]
        .as_object_mut()
        .expect("selection")
        .remove("covering");
    assert!(MovementCapture::decode(&canonical(&value)).is_err());
    let mut value = base.clone();
    value["window"]["selection"]["windows"]
        .as_array_mut()
        .expect("rows")
        .reverse();
    assert!(MovementCapture::decode(&canonical(&value)).is_err());
    let mut value = base;
    value["window"]["selection"]["windows"][0]["room_count"] = json!(0);
    assert!(MovementCapture::decode(&canonical(&value)).is_err());
}

#[test]
fn actual_world_capture_observes_missing_owner_slots_without_mutation() {
    let mut world = World::new_minimal();
    let actor = world
        .create(ObjKind::Player, "Walker", GOD, Some(LIMBO))
        .expect("actor");
    let before = world.get(actor).cloned();
    let captured = capture(&world, actor, None, TO, 100).expect("capture");
    assert_eq!(captured.actor_kind(), Some(ObjKind::Player));
    assert_eq!(captured.discovery(), &Discovery::AtlasAbsent);
    assert_eq!(captured.atlas_slot(), SlotPresence::Absent);
    assert!(matches!(
        captured.window(),
        WindowObservation::Observed {
            placement: SimPlacementObservation::SlotAbsent,
            ..
        }
    ));
    assert_eq!(world.get(actor), before.as_ref());
}
