#![cfg(test)]
//! Discovery observations and actual legacy policy, not movement authority.

use super::*;
use crate::geom::Point2;
use crate::record::{
    Access, Footprint, MAX_KNOWN_PLACES, RecordKey, RoomGeo, RoomKind, SurfaceRef,
};
use mush_core::{CanonicalId, Effects, GOD, Outbox, WorldEvent};
use mush_testkit::TestWorld;
use serde_json::{Value, json};

struct Rig {
    world: TestWorld,
    actor: ObjId,
    source: ObjId,
    destination: ObjId,
}
fn key(name: &str) -> RecordKey {
    RecordKey::new(
        CanonicalId::new("discovery").expect("pack"),
        CanonicalId::new(name).expect("entry"),
    )
}
fn state(rig: &Rig) -> &AtlasState {
    rig.world
        .world()
        .module_state::<AtlasState>()
        .expect("actual Atlas")
}
fn state_mut(rig: &mut Rig) -> &mut AtlasState {
    rig.world
        .world_mut()
        .module_state_mut::<AtlasState>()
        .expect("actual Atlas")
}
fn fixture(kind: ObjKind, known: Option<Vec<RecordKey>>) -> Rig {
    let mut world = TestWorld::with_modules(vec![
        Box::new(mush_sim::SimulationModule::new()),
        Box::new(crate::AtlasModule::new()),
    ]);
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
        .create(kind, "traveler", GOD, Some(source))
        .expect("actor");
    world.world_mut().set_now(1234);
    let mut rig = Rig {
        world,
        actor,
        source,
        destination,
    };
    for (room, name) in [(source, "source"), (destination, "destination")] {
        state_mut(&mut rig).records.roomgeo.insert(
            room,
            RoomGeo {
                room,
                key: key(&format!("room_{}", room.0)),
                neighborhood: key(name),
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
    }
    if let Some(known) = known {
        state_mut(&mut rig).travel.known.insert(actor, known);
    }
    rig
}
fn planned(rig: &Rig) -> WindowDiscoveryCapture {
    capture(
        rig.world.world(),
        rig.actor,
        Some(rig.source),
        rig.destination,
        rig.source,
        1234,
    )
    .expect("ordered discovery")
}
fn canonical(value: &Value) -> Vec<u8> {
    canonical_json_bytes_with_limit(value, MAX_WINDOW_DISCOVERY_BYTES).expect("bounded JSON")
}

#[test]
fn second_leg_observes_fifo_eviction_from_the_first_even_on_a_return_loop() {
    let before: Vec<_> = std::iter::once(key("source"))
        .chain((0..MAX_KNOWN_PLACES - 1).map(|n| key(&format!("old_{n}"))))
        .collect();
    let rig = fixture(ObjKind::Player, Some(before.clone()));
    let capture = planned(&rig);
    assert_eq!(capture.legs().len(), 2);
    let first = capture
        .legs()
        .first()
        .expect("first")
        .planned_known()
        .expect("first change");
    assert_eq!(first.places.first(), Some(&key("old_0")));
    assert_eq!(first.places.last(), Some(&key("destination")));
    assert_eq!(first.places.len(), MAX_KNOWN_PLACES);
    let second = capture.legs().get(1).expect("second");
    assert!(
        matches!(second.discovery(),Discovery::Mapped {known_before:Some(known),change:movement::LearnChange::Append {evict_oldest:1},..} if known==&first.places)
    );
    let final_doc = capture.planned_known().expect("final change");
    assert_eq!(final_doc.places.first(), Some(&key("old_1")));
    assert_eq!(final_doc.places.last(), Some(&key("source")));
    assert_eq!(final_doc.places.len(), MAX_KNOWN_PLACES);
    assert_eq!(state(&rig).travel.known.get(&rig.actor), Some(&before));
    let first = movement::capture(
        rig.world.world(),
        rig.actor,
        Some(rig.source),
        rig.destination,
        1234,
    )
    .expect("original capture");
    capture.check_first(&first).expect("same first observation");
    let bytes = capture.encode().expect("encode");
    let decoded = WindowDiscoveryCapture::decode(&bytes).expect("strict decode");
    assert_eq!(decoded, capture);
    assert_eq!(
        decoded.digest().expect("digest"),
        capture.digest().expect("digest")
    );
    decoded
        .recheck(rig.world.world())
        .expect("actual unchanged owner");
}

#[test]
fn unchanged_second_visit_keeps_first_postimage_and_does_not_reorder_a_duplicate() {
    let rig = fixture(ObjKind::Player, Some(vec![key("source")]));
    let capture = planned(&rig);
    assert!(matches!(
        capture.legs().get(1).expect("second").discovery(),
        Discovery::Mapped {
            change: movement::LearnChange::Unchanged,
            ..
        }
    ));
    assert_eq!(
        capture
            .planned_known()
            .expect("first change survives")
            .places,
        vec![key("source"), key("destination")]
    );
    let rig = fixture(
        ObjKind::Player,
        Some(vec![key("source"), key("destination")]),
    );
    assert!(planned(&rig).planned_known().is_none());
    let same = capture_same_room(&rig);
    assert_eq!(same.legs().len(), 1);
    assert!(same.planned_known().is_none());
}
fn capture_same_room(rig: &Rig) -> WindowDiscoveryCapture {
    capture(
        rig.world.world(),
        rig.actor,
        Some(rig.source),
        rig.source,
        rig.source,
        1234,
    )
    .expect("same-room discovery")
}

#[test]
fn actual_registered_legacy_observer_matches_both_ordered_discovery_legs() {
    for known in [
        None,
        Some(
            std::iter::once(key("source"))
                .chain((0..MAX_KNOWN_PLACES - 1).map(|n| key(&format!("old_{n}"))))
                .collect(),
        ),
    ] {
        let mut rig = fixture(ObjKind::Player, known);
        let expected = planned(&rig).planned_known().expect("ordered changes");
        for (from, to) in [(rig.source, rig.destination), (rig.destination, rig.source)] {
            rig.world
                .world_mut()
                .move_to(rig.actor, to)
                .expect("actual legacy Core move");
            rig.world.engine_mut().emit_world_event(
                &WorldEvent::Moved {
                    who: rig.actor,
                    from: Some(from),
                    to,
                },
                &mut Outbox::default(),
                &mut Effects::default(),
            );
        }
        assert_eq!(
            state(&rig).travel.known.get(&rig.actor),
            Some(&expected.places)
        );
    }
}

#[test]
fn decoder_refuses_stale_second_preimages_disconnected_legs_and_inconsistent_policy() {
    let rig = fixture(ObjKind::Player, Some(vec![key("source")]));
    let original = serde_json::to_value(planned(&rig)).expect("value");
    for fault in 0..10 {
        let mut value = original.clone();
        match fault {
            0 => value["format"] = json!(2),
            1 => value["legs"][1]["discovery"]["known_before"] = json!([key("source")]),
            2 => value["legs"].as_array_mut().expect("legs").swap(0, 1),
            3 => {
                value["legs"][0]["discovery"]["change"] = json!({"kind":"append","evict_oldest":99})
            }
            4 => {
                value.as_object_mut().expect("object").remove("source");
            }
            5 => {
                value.as_object_mut().expect("object").remove("actor_kind");
            }
            6 => value["actor_kind"] = serde_json::to_value(ObjKind::Npc).expect("kind"),
            7 => value["atlas_slot"] = serde_json::to_value(SlotPresence::Absent).expect("slot"),
            8 => value["extra"] = json!(true),
            _ => {
                value["legs"][0]
                    .as_object_mut()
                    .expect("leg")
                    .remove("from");
            }
        }
        assert!(
            WindowDiscoveryCapture::decode(&canonical(&value)).is_err(),
            "fault {fault}"
        );
    }
    let mut third = original.clone();
    let leg = third["legs"][0].clone();
    third["legs"].as_array_mut().expect("legs").push(leg);
    assert_eq!(
        WindowDiscoveryCapture::decode(&canonical(&third)),
        Err(CaptureError::Limit)
    );
    let mut noncanonical = vec![b' '];
    noncanonical.extend(canonical(&original));
    assert_eq!(
        WindowDiscoveryCapture::decode(&noncanonical),
        Err(CaptureError::Invalid)
    );
    assert_eq!(
        WindowDiscoveryCapture::decode(&vec![b' '; MAX_WINDOW_DISCOVERY_BYTES + 1]),
        Err(CaptureError::Limit)
    );
}

#[test]
fn recheck_reads_the_actual_second_room_and_passive_decoding_never_authenticates_a_proposal() {
    let mut rig = fixture(ObjKind::Player, None);
    let capture = planned(&rig);
    let mut changed = serde_json::to_value(&capture).expect("value");
    changed["legs"][1]["discovery"]["neighborhood"] = json!(key("hypothetical"));
    let decoded = WindowDiscoveryCapture::decode(&canonical(&changed))
        .expect("valid hypothetical observation");
    assert_ne!(
        decoded.digest().expect("digest"),
        capture.digest().expect("digest")
    );
    assert_eq!(
        decoded.recheck(rig.world.world()),
        Err(CaptureError::Invalid)
    );
    state_mut(&mut rig)
        .travel
        .known
        .insert(GOD, vec![key("unrelated")]);
    capture.recheck(rig.world.world()).expect("unrelated actor");
    let source = rig.source;
    state_mut(&mut rig)
        .records
        .roomgeo
        .get_mut(&source)
        .expect("source geometry")
        .neighborhood = key("changed_source");
    assert_eq!(
        capture.recheck(rig.world.world()),
        Err(CaptureError::Invalid)
    );
}

#[test]
fn bounds_apply_to_each_consulted_leg_and_preserve_npc_unmapped_and_absent_observations() {
    let mut rig = fixture(
        ObjKind::Player,
        Some(vec![key("large"); MAX_KNOWN_PLACES + 1]),
    );
    let destination = rig.destination;
    state_mut(&mut rig).records.roomgeo.remove(&destination);
    assert_eq!(
        capture(
            rig.world.world(),
            rig.actor,
            Some(rig.source),
            rig.destination,
            rig.source,
            1234
        ),
        Err(CaptureError::Limit)
    );
    let source = rig.source;
    state_mut(&mut rig).records.roomgeo.remove(&source);
    assert!(
        planned(&rig)
            .legs()
            .iter()
            .all(|leg| matches!(leg.discovery(), Discovery::Unmapped))
    );
    let npc = fixture(ObjKind::Npc, Some(vec![key("large"); MAX_KNOWN_PLACES + 1]));
    assert!(
        planned(&npc)
            .legs()
            .iter()
            .all(|leg| matches!(leg.discovery(), Discovery::NotPlayer))
    );
    let absent =
        capture(&World::new_minimal(), GOD, None, ObjId(10), ObjId(11), 1234).expect("absent data");
    assert_eq!(absent.atlas_slot(), SlotPresence::Absent);
    assert!(
        absent
            .legs()
            .iter()
            .all(|leg| matches!(leg.discovery(), Discovery::AtlasAbsent))
    );
    assert!(absent.planned_known().is_none());
}
