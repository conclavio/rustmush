//! The explicit fixture driver commits the real Space command and preserves
//! both receipt families across an actual Store-backed cold boot.

use mush_core::{BehaviorBinding, BehaviorId, GOD, LIMBO, ObjId, ObjKind};
use mush_sim::{SimulationModule, SpatialState};
use mush_testkit::{authoritative::AcknowledgedWorld, text_of};

fn fixture() -> (AcknowledgedWorld, ObjId, ObjId) {
    let mut world = AcknowledgedWorld::with_modules(vec![Box::new(SimulationModule::new())]);
    world.set_now(100);
    let hull = world
        .world_mut()
        .create(ObjKind::Thing, "Hull", GOD, Some(LIMBO))
        .unwrap();
    let bay = world
        .world_mut()
        .create(ObjKind::Room, "Bay", GOD, None)
        .unwrap();
    world
        .world_mut()
        .attach_behavior(
            bay,
            BehaviorId::new("sim.interior").unwrap(),
            BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 100,
            },
        )
        .unwrap();
    (world, hull, bay)
}

#[test]
fn queued_space_waits_for_exact_ack_and_cold_boot_keeps_both_typed_records() {
    let (mut world, hull, bay) = fixture();
    world.flush_legacy().unwrap();
    let events = world.run(GOD, &format!("@spatial/space {bay}=moving:{hull}"));
    assert!(
        events
            .iter()
            .any(|(_, event)| text_of(event).contains("Queued."))
    );
    assert!(
        !events
            .iter()
            .any(|(_, event)| text_of(event).contains("committed"))
    );
    assert!(
        !world
            .world()
            .module_state::<SpatialState>()
            .unwrap()
            .spaces
            .contains_key(&bay)
    );
    let events = world
        .commit_one(GOD, "sim", mush_core::physical_transition::ACTION)
        .unwrap();
    assert!(
        events
            .iter()
            .any(|(_, event)| text_of(event) == format!("Spatial update for {bay} committed."))
    );
    assert!(
        world
            .world()
            .module_state::<SpatialState>()
            .unwrap()
            .spaces
            .contains_key(&bay)
    );
    let boot = world.persisted_boot();
    let sim = boot.get("sim").unwrap();
    assert!(
        sim.kv
            .iter()
            .any(|(key, _)| key.contains("physical_transitions"))
    );
    assert!(
        sim.kv
            .iter()
            .any(|(key, _)| key.contains("personnel_crossings"))
    );

    let (mut restarted, hull2, bay2) = fixture();
    assert_eq!((hull2, bay2), (hull, bay));
    restarted.adopt_store(&world);
    let mut omitted = boot.clone();
    omitted.remove("sim");
    let refusal = restarted.try_modules_loaded(omitted).unwrap_err();
    assert!(
        refusal
            .to_string()
            .contains("omitted an acknowledged typed namespace")
    );
    assert!(
        !restarted
            .world()
            .module_state::<SpatialState>()
            .unwrap()
            .spaces
            .contains_key(&bay)
    );
    // The refusal occurs before any partial typed hydration: the same fresh
    // engine can still load the intact acknowledged snapshot.
    restarted.modules_loaded(boot);
    assert_eq!(
        restarted.world().module_state::<SpatialState>(),
        world.world().module_state::<SpatialState>()
    );
    let after = restarted.persisted_kv("sim");
    assert!(
        after
            .iter()
            .any(|(key, _)| key.contains("personnel_crossings"))
    );
}

#[test]
fn refused_space_never_gets_a_success_receipt_or_typed_record() {
    let (mut world, hull, bay) = fixture();
    let actor = world.spawn_player("Observer");
    let events = world.run_and_commit_one(
        actor,
        &format!("@spatial/space {bay}=moving:{hull}"),
        "sim",
        mush_core::physical_transition::ACTION,
    );
    assert!(events.iter().any(|(_,event)| matches!(event,mush_proto::OutputEvent::Sys(text) if text.spans().iter().any(|span| span.tag.as_deref()==Some("error")))));
    assert!(
        !events
            .iter()
            .any(|(_, event)| text_of(event).contains("committed"))
    );
    assert!(!world.engine().has_authoritative_submit());
    assert!(
        !world
            .persisted_kv("sim")
            .iter()
            .any(|(key, _)| mush_core::is_action_state_persistence_key(key))
    );
}
