use mush_contract_tests::{TOY_OWNER, install_toy_schedule, toy_definitions, toy_world_with_state};
use mush_core::{
    CanonicalId, ClockRef, ClockStamp, DurableSchedule, DurableScheduleDraft, IncidentId, OwnerId,
    RecoveryReason, ScheduleCatchUpOrder, ScheduleDriveCause, ScheduleId, ScheduleRestartPolicy,
    ScheduleSourceScanError,
};

fn schedule(local_id: &str, due: i64, restart: ScheduleRestartPolicy) -> DurableSchedule {
    let owner = OwnerId::new(TOY_OWNER).expect("static owner");
    DurableSchedule::new(DurableScheduleDraft {
        id: ScheduleId::derive(
            &owner,
            &CanonicalId::new(local_id).expect("canonical schedule id"),
        )
        .expect("schedule identity"),
        owner,
        clock: ClockRef::Utc,
        next_due: ClockStamp {
            clock: ClockRef::Utc,
            at: due,
        },
        cadence: Some(10),
        generation: 1,
        next_ordinal: 0,
        restart,
        catch_up_order: ScheduleCatchUpOrder::OldestFirst,
        max_catch_up: 4,
    })
    .expect("valid schedule")
}

#[test]
fn discovery_pages_canonical_durable_state_without_a_private_index() {
    let (mut world, families) = toy_world_with_state();
    world.world_mut().set_now(100);
    let mut expected = Vec::new();
    for local_id in ["charlie", "alpha", "bravo"] {
        let schedule = schedule(local_id, 90, ScheduleRestartPolicy::Resume);
        expected.push((schedule.id().canonical_hash().to_hex(), schedule.id()));
        install_toy_schedule(world.engine_mut(), &families, schedule);
    }
    expected.sort_by(|left, right| left.0.cmp(&right.0));

    let first = world
        .engine()
        .discover_scheduled_actions(ScheduleDriveCause::Live, None, 2)
        .expect("first discovery page");
    assert_eq!(first.records_scanned(), 2);
    assert_eq!(first.candidates().len(), 2);
    assert_eq!(
        first
            .candidates()
            .iter()
            .map(|candidate| candidate.advance().schedule())
            .collect::<Vec<_>>(),
        expected[..2]
            .iter()
            .map(|(_, schedule)| *schedule)
            .collect::<Vec<_>>()
    );
    assert!(
        first
            .candidates()
            .iter()
            .all(|candidate| candidate.action() == &toy_definitions().guarded_expire)
    );

    let second = world
        .engine()
        .discover_scheduled_actions(ScheduleDriveCause::Live, first.next_cursor(), 2)
        .expect("second discovery page");
    assert_eq!(second.records_scanned(), 1);
    assert_eq!(second.candidates().len(), 1);
    assert_eq!(second.candidates()[0].advance().schedule(), expected[2].1);
    assert!(second.next_cursor().is_none());
}

#[test]
fn restart_skip_is_still_actionable_and_owner_freeze_fails_closed() {
    let (mut world, families) = toy_world_with_state();
    world.world_mut().set_now(100);
    install_toy_schedule(
        world.engine_mut(),
        &families,
        schedule("restart_skip", 10, ScheduleRestartPolicy::SkipElapsed),
    );

    let page = world
        .engine()
        .discover_scheduled_actions(ScheduleDriveCause::Restart, None, 1)
        .expect("restart discovery");
    let advance = page.candidates()[0].advance();
    assert!(advance.occurrences().is_empty());
    assert!(advance.skipped().is_some());

    world
        .engine()
        .registry
        .freeze_mechanics_owner(
            &OwnerId::new(TOY_OWNER).expect("static owner"),
            IncidentId(77),
            RecoveryReason::IntegrityViolation {
                code: CanonicalId::new("schedule_source_test").expect("static reason"),
            },
        )
        .expect("freeze owner");
    assert!(matches!(
        world
            .engine()
            .discover_scheduled_actions(ScheduleDriveCause::Restart, None, 1),
        Err(ScheduleSourceScanError::OwnerNotReady(_))
    ));
}
