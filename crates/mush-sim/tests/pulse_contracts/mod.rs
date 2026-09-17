//! Regressions from the K-sequence production-adapter review.
use super::*;

fn prepare_tick(
    tw: &mut TestWorld,
    domain: DomainId,
    token: u8,
) -> (mush_core::PreparedTxn, Value) {
    let page = tw
        .engine()
        .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 64)
        .unwrap();
    let candidate = page
        .candidates()
        .iter()
        .find(|c| {
            matches!(
                &c.advance().observed_at().clock, ClockRef::DomainTick { id, .. }
                if domain_id_of(id) == Some(domain)
            )
        })
        .expect("domain has a due tick");
    let ingress =
        mush_core::scheduled_action_ingress(candidate.action(), candidate.advance(), 1).unwrap();
    let plan = tw
        .engine()
        .plan_scheduled_action(candidate, &ingress)
        .unwrap();
    let (prepared, _) = tw
        .engine_mut()
        .prepare_registered_scheduled_authoritative_action(
            candidate,
            plan.invocation(),
            CommitToken::from_bytes([token; 16]),
        )
        .unwrap();
    (prepared, plan.invocation().parameters().clone())
}

#[test]
fn new_windows_pin_current_key_and_retire_the_resolved_windows_key() {
    let (mut tw, fx) = fold_scene();
    issue(&mut tw, GOD, &format!("@domain/tick #{}", fx.field.0), 200).unwrap();
    assert_eq!(
        window_record(&tw, 0)
            .unwrap()
            .window()
            .resolution_secret_version(),
        Some(3)
    );
    tw.engine_mut().set_resolution_keys(
        mush_core::ResolutionKeyRing::new(
            mush_core::ResolutionSecret::new(4, [8; 32]).unwrap(),
            [mush_core::ResolutionSecret::new(3, [7; 32]).unwrap()],
        )
        .unwrap(),
    );
    for (at, token) in [(1_002, 201), (1_004, 202)] {
        tw.set_now(at);
        let (prepared, _) = prepare_tick(&mut tw, DomainId(0), token);
        let (_, install) = prepared.into_parts();
        tw.engine_mut()
            .install_prepared_action(install)
            .expect("the fixture installs");
        assert_eq!(
            window_record(&tw, current_tick(&tw))
                .unwrap()
                .window()
                .resolution_secret_version(),
            Some(4)
        );
        assert_eq!(
            tw.engine().resolution_key_rotation_references(),
            std::collections::BTreeSet::from([4])
        );
    }
    let handles = handles(&tw);
    for (tick, version) in [(0, 3), (1, 4)] {
        let key = TickCommitKey {
            domain: DomainId(0),
            generation: generation(&tw),
            tick,
        };
        assert_eq!(
            tw.engine()
                .action_state()
                .get(&handles.commits, &key)
                .unwrap()
                .unwrap()
                .resolution_secret_version(),
            Some(version)
        );
    }
}

#[test]
fn later_domain_cleanup_resumes_committed_progress_and_does_not_skip_at_removal_cap() {
    let (mut tw, fx) = fold_scene();
    issue(&mut tw, GOD, &format!("@domain/tick #{}", fx.field.0), 210).unwrap();
    for n in 1..=4 {
        let host = tw
            .world_mut()
            .create(ObjKind::Room, &format!("Field {n}"), GOD, None)
            .unwrap();
        bind(&mut tw, host, "sim.domain");
        ok(&mut tw, &format!("@domain/create #{}=sim.passive", host.0));
        ok(&mut tw, &format!("@domain/promote #{}=contact", host.0));
        issue(
            &mut tw,
            GOD,
            &format!("@domain/tick #{}", host.0),
            210 + n as u8,
        )
        .unwrap();
    }
    let owner = mush_core::OwnerId::new("sim").unwrap();
    let families = handles(&tw);
    for domain in 0..=4 {
        let generation = clock_of(&tw, DomainId(domain)).generation().unwrap();
        let clock = pulse_clock(&owner, DomainId(domain), generation).unwrap();
        let mut txn = tw
            .engine_mut()
            .action_transaction(&families.commits)
            .unwrap();
        let ticks = if domain == 4 { 0..100 } else { 136..200 };
        for tick in ticks {
            let window = tick_window_id(&owner, DomainId(domain), generation, tick).unwrap();
            let record = mush_sim::tick::DomainTickCommit::new(
                DomainId(domain),
                tick,
                clock.clone(),
                window,
                vec![],
                vec![],
                vec![],
                mush_core::CanonicalHash::from_bytes([1; 32]),
                vec![],
                Some(3),
            )
            .unwrap();
            txn.insert(&families.commits, &record.key(), record)
                .unwrap();
        }
        let (_, install) = txn.prepare().unwrap().into_parts();
        tw.engine_mut()
            .install_prepared_action(install)
            .expect("the fixture installs");
    }
    tw.set_now(1_400);
    let (prepared, first_plan) = prepare_tick(&mut tw, DomainId(4), 220);
    assert!(first_plan["reap_commits"].as_array().unwrap().is_empty());
    let (_, install) = prepared.into_parts();
    tw.engine_mut()
        .abort_prepared_action(install)
        .expect("the fixture aborts");
    assert!(
        tw.engine()
            .action_state()
            .get(&families.reaping, &DomainId(4))
            .unwrap()
            .is_none()
    );
    let (prepared, retried) = prepare_tick(&mut tw, DomainId(4), 220);
    assert_eq!(first_plan, retried);
    let (ops, install) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(install)
        .expect("the fixture installs");
    // Prove that the committed cursor uses ordinary typed boot hydration.
    let records = ops.into_iter().filter_map(|op| match op.op {
        mush_core::ModuleWrite::JsonSet { key, value }
            if key.contains(mush_sim::tick::REAP_FAMILY) =>
        {
            Some(mush_core::ActionStateBootRecord::new(op.ns, key, value))
        }
        _ => None,
    });
    let mut reboot = boot();
    reboot.engine_mut().hydrate_action_state(records).unwrap();
    let reboot_handles = handles(&reboot);
    assert_eq!(
        reboot
            .engine()
            .action_state()
            .get(&reboot_handles.reaping, &DomainId(4))
            .unwrap(),
        tw.engine()
            .action_state()
            .get(&families.reaping, &DomainId(4))
            .unwrap()
    );
    for (at, token, count) in [(1_402, 221, 64), (1_404, 222, 36)] {
        tw.set_now(at);
        let (prepared, plan) = prepare_tick(&mut tw, DomainId(4), token);
        assert_eq!(plan["reap_commits"].as_array().unwrap().len(), count);
        let (_, install) = prepared.into_parts();
        tw.engine_mut()
            .install_prepared_action(install)
            .expect("the fixture installs");
    }
    let generation = clock_of(&tw, DomainId(4)).generation().unwrap();
    for tick in 0..100 {
        assert!(
            tw.engine()
                .action_state()
                .get(
                    &families.commits,
                    &TickCommitKey {
                        domain: DomainId(4),
                        generation,
                        tick
                    }
                )
                .unwrap()
                .is_none()
        );
    }
}
