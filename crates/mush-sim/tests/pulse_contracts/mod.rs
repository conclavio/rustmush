//! Regressions from the K-sequence production-adapter review.
use super::*;

type Contacts = mush_core::StateFamilyHandle<ContactId, mush_core::ContactState>;
use mush_core::ContactId;

struct ContactOwner;
impl mush_core::GameModule for ContactOwner {
    fn name(&self) -> &'static str {
        "tactics"
    }
    fn register(&mut self, reg: &mut mush_core::Registrar<'_>) -> Result<(), mush_core::ModError> {
        let codec = mush_core::ContactStateCodec;
        let handle = reg.state_family(
            mush_core::contact_state_family_descriptor(
                mush_core::StateFamilyId::new("contacts").unwrap(),
            )
            .unwrap(),
            codec,
            codec,
            codec,
            codec,
        )?;
        reg.authority_guard_validator(
            mush_core::AuthorityGuardKind::Contact,
            mush_core::ContactGuardValidator::new(handle.clone()),
        )?;
        reg.state(handle);
        reg.state(mush_core::ContactLedgerState::default());
        Ok(())
    }
}

fn contact_scene() -> (TestWorld, Fold) {
    fold_scene_with(vec![
        Box::new(SimulationModule::new()),
        Box::new(ContactOwner),
    ])
}

fn seed_contact(tw: &mut TestWorld, holder: ObjId) -> ContactId {
    let id = ContactId::from_hex("00112233445566778899aabbccddeeff").unwrap();
    let now = tw.world().world_ic_now();
    let contact = mush_core::ContactState::new(mush_core::ContactStateDraft {
        truth_subject: mush_core::SubjectKey::Object { id: ObjId(987_654) },
        audience: vec![PrincipalId::Object { id: holder }],
        observed_at: now.clone(),
    })
    .unwrap();
    let handle = tw.world().module_state::<Contacts>().unwrap().clone();
    let mut txn = tw.engine_mut().action_transaction(&handle).unwrap();
    txn.insert(&handle, &id, contact.clone()).unwrap();
    let (_, install) = txn.prepare().unwrap().into_parts();
    tw.engine_mut()
        .install_prepared_action(install)
        .expect("the fixture installs");
    tw.world_mut()
        .module_state_mut::<mush_core::ContactLedgerState>()
        .unwrap()
        .install(
            vec![mush_core::ContactLedgerRow {
                holder: PrincipalId::Object { id: holder },
                contact: id,
                generation: contact.generation(),
                revision: contact.revision(),
                active: true,
                observed_at: now.clone(),
                phantom: None,
            }],
            now,
        );
    id
}

fn fire_submit(tw: &TestWorld, fx: &Fold, contact: ContactId) -> mush_core::AuthoritativeSubmit {
    let grant = tw
        .world()
        .channel_grant(fx.gunner, fx.atlas, "sim", "gunner")
        .unwrap();
    mush_core::AuthoritativeSubmit {
        actor: fx.gunner,
        action: intent_action_ref(tw.registry()).unwrap(),
        subject: mush_core::PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({ "intent": serde_json::to_string(&json!({
            "domain": 0, "tick": current_tick(tw), "unit": fx.atlas,
            "station": "gunner", "epoch": grant.epoch, "priority": grant.priority,
            "channel": "fire", "kind": "fire", "contact": contact,
            "contact_generation": 1,
        })).unwrap() }),
        key: mush_core::IdempotencyKeyText::parse_canonical("regression.fire").unwrap(),
    }
}

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
fn structured_fire_refuses_foreign_missing_and_missing_generation_contacts() {
    let (mut tw, fx) = contact_scene();
    run_ok(
        &mut tw,
        fx.gunner,
        &format!("+focus #{}/gunner", fx.atlas.0),
    );
    let foreign = seed_contact(&mut tw, fx.bystander);
    run_err(&mut tw, fx.gunner, &format!("+fire {foreign}"));
    let mut submit = fire_submit(&tw, &fx, foreign);
    let foreign_error = resolve_submit(&tw, &submit, 190).unwrap_err();
    let absent = ContactId::from_hex("11112233445566778899aabbccddeeff").unwrap();
    submit = fire_submit(&tw, &fx, absent);
    assert_eq!(
        resolve_submit(&tw, &submit, 190).unwrap_err(),
        foreign_error
    );
    let mut doc: Value =
        serde_json::from_str(submit.parameters["intent"].as_str().unwrap()).unwrap();
    doc.as_object_mut().unwrap().remove("contact_generation");
    submit.parameters["intent"] = json!(serde_json::to_string(&doc).unwrap());
    assert!(drive_submit(&mut tw, &submit, 191).is_err());
    assert!(window_record(&tw, 0).is_none());
}

#[test]
fn structured_eject_cannot_bypass_proxy_abandonment_policy() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.ops, &format!("+focus #{}/ops", fx.atlas.0));
    let account = tw
        .world()
        .module_state::<mush_sim::focus::AccountOracle>()
        .unwrap()
        .account_of(fx.gunner)
        .unwrap();
    let focus = tw
        .world_mut()
        .module_state_mut::<mush_sim::focus::FocusState>()
        .unwrap();
    let mut proxy = focus.focus_of_actor(fx.ops).unwrap().clone();
    proxy.account = account;
    proxy.driver = fx.gunner;
    proxy.proxy = true;
    focus.claim_for_test(proxy);
    // A real live movement offer, through the driving ingress, still resolves.
    run_ok(&mut tw, fx.gunner, "+heading 1,0,0");
    let mut submit = *tw.engine_mut().take_authoritative_submit().unwrap();
    assert!(resolve_submit(&tw, &submit, 192).is_ok());
    run_err(&mut tw, fx.gunner, "+eject");
    let mut doc: Value =
        serde_json::from_str(submit.parameters["intent"].as_str().unwrap()).unwrap();
    doc["kind"] = json!("eject");
    doc.as_object_mut().unwrap().remove("value");
    submit.parameters["intent"] = json!(serde_json::to_string(&doc).unwrap());
    assert!(
        drive_submit(&mut tw, &submit, 193)
            .unwrap_err()
            .contains("abandonment")
    );
    assert!(window_record(&tw, 0).is_none());
}

#[test]
fn platform_contact_is_guarded_at_resolution_and_rechecked_when_tick_fires() {
    let (mut tw, fx) = contact_scene();
    issue(&mut tw, GOD, &format!("@domain/tick #{}", fx.field.0), 180).unwrap();
    run_ok(
        &mut tw,
        fx.gunner,
        &format!("+focus #{}/gunner", fx.atlas.0),
    );
    let id = seed_contact(&mut tw, fx.atlas);
    run_ok(&mut tw, fx.gunner, &format!("+fire {id}"));
    let line_submit = *tw.engine_mut().take_authoritative_submit().unwrap();
    let resolved = resolve_submit(&tw, &line_submit, 181).unwrap();
    let (_, _, guards) = resolved.into_authoritative_parts();
    assert!(guards.authority().iter().any(|guard| matches!(guard,
        mush_core::AuthorityGuard::Contact { principal: PrincipalId::Object { id: holder }, id: contact, generation: 1, .. }
        if *holder == fx.atlas && *contact == id
    )));
    drive_submit(&mut tw, &line_submit, 181).unwrap();
    // Lose and reacquire truth but leave the display at generation 1. Even an
    // active contact with the same id must not resurrect the queued intent.
    let handle = tw.world().module_state::<Contacts>().unwrap().clone();
    let observed = tw.world().world_ic_now();
    let mut txn = tw.engine_mut().action_transaction(&handle).unwrap();
    let before = txn.get(&handle, &id).unwrap().unwrap();
    let lost = before
        .checked_mark_lost(
            before.revision(),
            observed.clone(),
            mush_core::CanonicalId::new("occluded").unwrap(),
        )
        .unwrap();
    let reacquired = lost.checked_reacquire(lost.revision(), observed).unwrap();
    assert_eq!(reacquired.generation(), 3);
    txn.replace(&handle, &id, reacquired).unwrap();
    let (_, install) = txn.prepare().unwrap().into_parts();
    tw.engine_mut()
        .install_prepared_action(install)
        .expect("the fixture installs");
    let e = tw.engine();
    assert!(
        !guards
            .revalidate_with_action_state(
                &e.world,
                e.action_state(),
                e.registry.revision_validators().unwrap(),
                e.registry.authority_guard_validators().unwrap(),
                e.registry.definitions().unwrap()
            )
            .unwrap()
            .is_current()
    );
    assert!(resolve_submit(&tw, &line_submit, 182).is_err());
    tw.set_now(1_002);
    let (prepared, plan) = prepare_tick(&mut tw, DomainId(0), 183);
    assert!(plan["current"].as_array().unwrap().is_empty());
    let (_, install) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(install)
        .expect("the fixture installs");
    let handles = handles(&tw);
    let key = TickCommitKey {
        domain: DomainId(0),
        generation: generation(&tw),
        tick: 0,
    };
    let commit = tw
        .engine()
        .action_state()
        .get(&handles.commits, &key)
        .unwrap()
        .unwrap();
    assert!(commit.accepted().is_empty());
    assert!(matches!(
        commit.deltas()[0].outcome,
        DeltaOutcome::Refused { .. }
    ));
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
