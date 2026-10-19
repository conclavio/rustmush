use super::*;

#[test]
fn acknowledged_receipt_hydrates_only_with_exact_typed_envelope_and_key() {
    // Codec recovery only: the Industry consumer must separately reconcile
    // actual refit history, paid title and the current Tactics installation.
    let mut r = rig(true, true);
    let prepared = r.prepare(&[]).unwrap();
    let writes = r.install(prepared).unwrap();
    let (key, envelope) = writes
        .into_iter()
        .find_map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if namespace.as_str() == "industry" => Some((key, value)),
            _ => None,
        })
        .unwrap();
    for corrupt in [false, true] {
        let mut fresh = rig(true, true);
        let mut value = envelope.clone();
        if corrupt {
            value["value"]["order"] = json!(8);
        }
        let result = fresh.engine.hydrate_action_state(vec![
            crate::action_state::ActionStateBootRecord::new("industry", key.as_str(), value),
        ]);
        result.unwrap();
        let audit = fresh.engine.all_modules_loaded(
            &mut crate::Outbox::new(),
            &mut crate::engine::Effects::new(),
        );
        if corrupt {
            assert!(
                audit.is_err(),
                "permanent receipt audit admitted a crossed key"
            );
        } else {
            audit.unwrap();
            assert_eq!(
                fresh
                    .engine
                    .action_state()
                    .get(&fresh.handles.service.records, &7)
                    .unwrap(),
                r.engine
                    .action_state()
                    .get(&r.handles.service.records, &7)
                    .unwrap()
            );
        }
    }
}

#[test]
fn actual_empty_install_and_replace_have_unique_merged_images_and_inert_prepare() {
    for (replace, separate, expected) in [(false, false, 3), (true, false, 4), (true, true, 5)] {
        let mut r = rig(replace, separate);
        let before = r.objects();
        let p = r.prepare(&[]).expect("prepare exchange");
        assert_eq!(r.objects(), before);
        r.unchanged_owners();
        assert!(
            r.engine
                .action_state()
                .get(&r.handles.service.records, &7)
                .unwrap()
                .is_none()
        );
        let images: Vec<_> =
            p.0.authoritative_replay_writes()
                .into_iter()
                .filter_map(|w| match w {
                    ReplayWrite::ObjectSet { object } => Some(object),
                    _ => None,
                })
                .collect();
        assert_eq!(images.len(), expected);
        assert_eq!(
            images.iter().map(|o| o.id).collect::<BTreeSet<_>>().len(),
            expected
        );
        r.stage(p, &[]).expect("stage exchange");
        assert_eq!(r.objects(), before);
        r.unchanged_owners();
        let (_, installation) = r.engine.take_staged_action().unwrap().unwrap().into_parts();
        let hook_count = r.handles.hook.seen.lock().unwrap().len();
        r.engine
            .install_composed_action(installation)
            .expect("install exchange");
        assert_eq!(
            r.handles.hook.seen.lock().unwrap().len(),
            hook_count,
            "no hooks after custody"
        );
        assert!(r.handles.hook.seen.lock().unwrap().iter().all(|v| v.2));
        assert_eq!(
            r.engine.world.get(r.incoming).unwrap().location,
            Some(r.hull)
        );
        if let Some(old) = r.outgoing {
            assert_eq!(r.engine.world.get(old).unwrap().location, r.return_to);
        }
        assert_eq!(r.engine.world.module_state::<Cell<1>>(), Some(&Cell(1)));
        assert_eq!(r.engine.world.module_state::<Cell<2>>(), Some(&Cell(1)));
        assert!(
            r.engine
                .action_state()
                .get(&r.handles.service.records, &7)
                .unwrap()
                .is_some()
        );
        for image in images {
            assert_eq!(r.engine.world.get(image.id), Some(&image));
        }
        assert_eq!(r.engine.world.get(r.room), before.get(&r.room));
        assert!(r.engine.world.take_dirty().is_empty());
        assert!(r.prepare(&[]).is_err(), "one refit cannot apply twice");
    }
}

#[test]
fn native_bind_failures_never_apply_objects_or_either_native_owner() {
    for flags in [["refuse_first"], ["refuse_second"]] {
        let mut r = rig(true, true);
        let before = r.objects();
        let p = r.prepare(&flags).unwrap();
        r.stage(p, &flags).unwrap();
        let (_, installation) = r.engine.take_staged_action().unwrap().unwrap().into_parts();
        assert!(r.engine.install_composed_action(installation).is_err());
        assert_eq!(r.objects(), before);
        r.unchanged_owners();
        assert!(
            !r.trace
                .lock()
                .unwrap()
                .iter()
                .any(|(_, phase)| *phase == "apply")
        );
        assert!(
            r.engine
                .action_state()
                .get(&r.handles.service.records, &7)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn missing_caught_and_late_extra_owner_cannot_publish_exchange() {
    for flags in [["missing_owner"], ["caught"], ["extra_owner"]] {
        let mut r = rig(true, false);
        let before = r.objects();
        assert!(r.prepare(&flags).is_err());
        assert_eq!(r.objects(), before);
        r.unchanged_owners();
        assert!(
            r.engine
                .action_state()
                .get(&r.handles.service.records, &7)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn object_ancestor_and_owner_generation_drift_refuse_stage_take_and_install() {
    for stage in 0..3 {
        for target in 0..6 {
            let mut r = rig(true, true);
            let p = r.prepare(&[]).unwrap();
            let mut p = Some(p);
            let mut install = None;
            if stage > 0 {
                r.stage(p.take().unwrap(), &[]).unwrap();
            }
            if stage == 2 {
                install = Some(
                    r.engine
                        .take_staged_action()
                        .unwrap()
                        .unwrap()
                        .into_parts()
                        .1,
                );
            }
            if target == 5 {
                r.data.0.lock().unwrap().0.generation += 1;
            } else {
                let id = [r.incoming, r.hull, r.source, r.outgoing.unwrap(), r.room][target];
                r.engine
                    .world
                    .set_name(id, "changed complete preimage")
                    .unwrap();
                r.engine.world.take_dirty();
            }
            let changed = r.objects();
            match stage {
                0 => assert!(r.stage(p.take().unwrap(), &[]).is_err()),
                1 => assert!(r.engine.take_staged_action().is_err()),
                _ => assert!(r.engine.install_composed_action(install.unwrap()).is_err()),
            }
            assert_eq!(r.objects(), changed);
            r.unchanged_owners();
            assert!(
                r.engine
                    .action_state()
                    .get(&r.handles.service.records, &7)
                    .unwrap()
                    .is_none()
            );
        }
    }
}

#[test]
fn both_carry_hooks_recheck_committed_preimages_until_custody() {
    for stage in 0..3 {
        for old in [false, true] {
            let mut r = rig(true, false);
            let before = r.objects();
            let mut p = None;
            if stage > 0 {
                p = Some(r.prepare(&[]).unwrap());
            }
            if stage == 2 {
                r.stage(p.take().unwrap(), &[]).unwrap();
            }
            r.handles.hook.veto.store(
                if old {
                    r.outgoing.unwrap().0
                } else {
                    r.incoming.0
                },
                Ordering::SeqCst,
            );
            match stage {
                0 => assert!(r.prepare(&[]).is_err()),
                1 => assert!(r.stage(p.take().unwrap(), &[]).is_err()),
                _ => assert!(r.engine.take_staged_action().is_err()),
            }
            assert!(r.handles.hook.seen.lock().unwrap().iter().all(|v| v.2));
            assert_eq!(r.objects(), before);
            r.unchanged_owners();
        }
    }
}

#[test]
fn lifecycle_non_leaf_wrong_source_and_invalid_facts_refuse_inertly() {
    for case in 0..7 {
        let mut r = rig(true, true);
        match case {
            0 => r
                .engine
                .world
                .set_flag(r.incoming, Flags::GOING, true)
                .unwrap(),
            1 => r.engine.world.set_flag(r.hull, Flags::GOING, true).unwrap(),
            2 => r
                .engine
                .world
                .set_flag(r.outgoing.unwrap(), Flags::GOING, true)
                .unwrap(),
            3 => {
                r.engine
                    .world
                    .create(ObjKind::Thing, "nested child", r.actor, Some(r.incoming))
                    .unwrap();
            }
            4 => r.engine.world.move_to(r.incoming, r.room).unwrap(),
            5 => {
                r.data.0.lock().unwrap().1 = false;
            }
            _ => {
                r.data.0.lock().unwrap().0.unit_revision = u64::MAX;
            }
        }
        r.engine.world.take_dirty();
        let before = r.objects();
        assert!(r.prepare(&[]).is_err());
        assert_eq!(r.objects(), before);
        r.unchanged_owners();
    }
}

#[test]
fn complete_exchange_envelope_accepts_exact_limit_and_refuses_one_extra_byte() {
    let mut probe = rig(true, true);
    probe.engine.world.set_name(probe.hull, "").unwrap();
    probe.engine.world.take_dirty();
    let p = probe.prepare(&[]).unwrap();
    let writes: Vec<_> =
        p.0.authoritative_replay_writes()
            .into_iter()
            .filter(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
            .collect();
    let base = canonical_json_bytes_with_limit(&json!(writes), IMAGE_BYTES)
        .unwrap()
        .len();
    probe.engine.abort_prepared_transaction(p.0);
    for extra in [0, 1] {
        let mut r = rig(true, true);
        r.engine
            .world
            .set_name(r.hull, &"x".repeat(IMAGE_BYTES - base + extra))
            .unwrap();
        r.engine.world.take_dirty();
        let before = r.objects();
        let p = r.prepare(&[]);
        if extra == 1 {
            assert!(p.is_err());
            assert_eq!(r.objects(), before);
            r.unchanged_owners();
        } else {
            let p = p.unwrap();
            let writes: Vec<_> =
                p.0.authoritative_replay_writes()
                    .into_iter()
                    .filter(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
                    .collect();
            assert_eq!(
                canonical_json_bytes_with_limit(&json!(writes), IMAGE_BYTES)
                    .unwrap()
                    .len(),
                IMAGE_BYTES
            );
            r.stage(p, &[]).unwrap();
            let (_, install) = r.engine.take_staged_action().unwrap().unwrap().into_parts();
            r.engine.install_composed_action(install).unwrap();
        }
    }
}

#[test]
fn actual_receipt_strictly_binds_action_participants_targets_and_nullable_fields() {
    let mut r = rig(false, false);
    let p = r.prepare(&[]).unwrap();
    r.install(p).unwrap();
    let row = r
        .engine
        .action_state()
        .get(&r.handles.service.records, &7)
        .unwrap()
        .unwrap();
    let value = serde_json::to_value(row).unwrap();
    assert_eq!(decode(&value).unwrap(), *row);
    for case in 0..7 {
        let mut corrupt = value.clone();
        match case {
            0 => {
                corrupt.as_object_mut().unwrap().remove("outgoing");
            }
            1 => {
                corrupt.as_object_mut().unwrap().remove("return_to");
            }
            2 => corrupt["unexpected"] = json!(true),
            3 => corrupt["participants"][1] = corrupt["participants"][0].clone(),
            4 => corrupt["objects"].as_array_mut().unwrap().reverse(),
            5 => corrupt["action"] = json!(owner_ref::<1>()),
            _ => corrupt["objects"][0]["object"] = json!(r.room),
        }
        assert!(decode(&corrupt).is_err());
    }
}

#[test]
fn exact_action_identity_and_registered_owner_readiness_are_required() {
    let mut r = rig(true, false);
    let actual = &r.handles.action;
    let changed = DefinitionRef::new(
        actual.key().clone(),
        actual.version(),
        DefinitionDigest::from_bytes([99; 32]),
    )
    .unwrap();
    let inv = r.invocation(&[]);
    assert!(
        r.engine
            .prepare_registered_composed_authoritative_action(&changed, &inv, token(7))
            .is_err()
    );
    for owner in ["industry", "economy", "tactics"] {
        let mut r = rig(true, false);
        r.engine
            .registry
            .freeze_mechanics_owner(
                &OwnerId::new(owner).unwrap(),
                crate::mechanics::IncidentId(1),
                crate::mechanics::RecoveryReason::IntegrityViolation {
                    code: CanonicalId::new("exchange_fixture").unwrap(),
                },
            )
            .unwrap();
        assert!(r.prepare(&[]).is_err());
        r.unchanged_owners();
    }
}

#[test]
fn actual_work_order_domain_includes_zero_and_maximum_in_canonical_receipts() {
    // Constructor and codec boundary only; native Industry authority is tested
    // by its real consumer. WorkOrderId has the complete u64 domain.
    for order in [0, u64::MAX] {
        let facts =
            ExchangeFacts::new(0, order, ObjId(1), ObjId(2), 0, ObjId(3), ObjId(4), None).unwrap();
        assert_eq!(facts.order(), order);
        let objects = [2, 3, 4]
            .into_iter()
            .map(|id| ExchangeObjectDigest {
                object: ObjId(id),
                before: DefinitionDigest::from_bytes([1; 32]),
                after: DefinitionDigest::from_bytes([2; 32]),
            })
            .collect();
        let row = ExchangeRecord::new(
            &facts,
            action_ref(),
            [owner_ref::<1>(), owner_ref::<2>()],
            token(1),
            0,
            objects,
        );
        let value = bounded(&row).unwrap();
        let decoded = decode(&value).unwrap();
        assert_eq!(decoded.order, order);
        assert_eq!(bounded(&decoded).unwrap(), value);
        let receipt = ExchangedCoreComponent { row: decoded };
        assert_eq!(receipt.order(), order);
        assert!(receipt.digest().is_ok());
    }
}
