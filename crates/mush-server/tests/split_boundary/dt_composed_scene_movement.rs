//! Real composed owner proofs, Store custody and mixed-protocol cold recovery.

use std::sync::atomic::Ordering;

use mush_core::locomotion::composed::ACTION;
use mush_core::{
    AuthoritativeBundle, Effects, ModuleOp, ModuleWrite, ObjKind, Outbox, ReplayWrite,
};
use mush_scenes::{SceneEventKind, SceneStatus};
use mush_store::{AuthoritativeStore, IdempotencyStatus, InMemoryStore, ObjectStore, WriteBatch};
use serde_json::{Value, json};

use super::dt_atomic_scene_movement::{entries, scenes, staff, start_scenes};
use super::dt_locomotion::{Rig, fixture, location, stage_action, stage_action_with_nonce};
use super::dt_unit_authority::persist_legacy;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[tokio::test]
async fn actual_tactics_custody_and_readiness_guard_every_locomotion_profile_after_ack()
-> TestResult {
    for action in [
        mush_core::locomotion::transition::ACTION,
        mush_core::locomotion::scene_transition::ACTION,
        ACTION,
    ] {
        for freeze in [false, true] {
            let store = InMemoryStore::new();
            let mut rig = fixture(&store, ObjKind::Player).await?;
            let (source, destination, _) = start_scenes(&mut rig, &store).await?;
            let _ = rig.world.run(
                mush_core::GOD,
                &format!("@unit/create #{}=tacsim.unit.line_tank", rig.actor.0),
            );
            let snapshot = rig
                .world
                .world()
                .module_state::<mush_tactics::UnitStore>()
                .ok_or("Tactics")?
                .units
                .get(&rig.actor)
                .cloned()
                .ok_or("actual unit creation")?;
            persist_legacy(&mut rig.world, &store).await?;
            let before = [
                entries(&rig, source)?.len(),
                entries(&rig, destination)?.len(),
            ];
            stage_action(&mut rig, "north", action)?;
            let revision = rig.world.engine().action_state().revision();
            let records = rig.world.engine().action_state().len();
            let (bundle, install) = rig
                .world
                .engine_mut()
                .take_staged_action()?
                .ok_or("staged")?
                .into_parts();
            store
                .reserve_authoritative_intent((*bundle).clone())
                .await?;
            store
                .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
                .await?;
            if freeze {
                rig.world.engine().registry.freeze_mechanics_owner(
                    &mush_core::OwnerId::new("tactics")?,
                    mush_core::IncidentId(119),
                    mush_core::RecoveryReason::IntegrityViolation {
                        code: mush_core::CanonicalId::new("custody_test")?,
                    },
                )?;
            } else {
                // Fault injection changes the actual Tactics marker only;
                // Sim's projection and all spatial/Core rows remain fixed.
                rig.world
                    .world_mut()
                    .module_state_mut::<mush_tactics::UnitStore>()
                    .ok_or("Tactics")?
                    .deployed
                    .insert(
                        rig.actor,
                        mush_tactics::custody::DeployedMarker {
                            mark: mush_sim::DeployedMark::new(119, "arena"),
                            domain: None,
                            pose: None,
                            snapshot,
                        },
                    );
                assert!(!mush_sim::deployed::is_deployed(
                    rig.world.world(),
                    rig.actor
                ));
            }
            assert!(
                rig.world
                    .engine_mut()
                    .install_composed_action(install)
                    .is_err(),
                "{action}, freeze {freeze}"
            );
            assert_eq!(location(&rig), Some(rig.source));
            assert_eq!(entries(&rig, source)?.len(), before[0]);
            assert_eq!(entries(&rig, destination)?.len(), before[1]);
            assert_eq!(rig.world.engine().action_state().revision(), revision);
            assert_eq!(rig.world.engine().action_state().len(), records);
            assert!(rig.world.engine().action_state().reservation_lost());
            assert_eq!(rig.moved.load(Ordering::SeqCst), 0);
        }
    }
    Ok(())
}

fn row<'a>(bundle: &'a AuthoritativeBundle, family: &str) -> TestResult<(&'a str, &'a Value)> {
    bundle
        .writes()
        .iter()
        .find_map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if namespace.as_str() == "sim"
                && key
                    .as_str()
                    .starts_with(&format!("action-state/v1/{family}/")) =>
            {
                Some((key.as_str(), value))
            }
            _ => None,
        })
        .ok_or("typed movement row missing".into())
}

async fn commit(
    rig: &mut Rig,
    store: &InMemoryStore,
    line: &str,
    action: &str,
    nonce: u8,
) -> TestResult<Box<AuthoritativeBundle>> {
    stage_action_with_nonce(rig, line, action, nonce)?;
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged")?
        .into_parts();
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    let mut out = Outbox::default();
    rig.world.engine_mut().install_composed_action_announcing(
        installation,
        &mut out,
        &mut Effects::default(),
    )?;
    assert!(out.is_empty());
    assert!(!rig.world.engine().has_legacy_persistence_work());
    Ok(bundle)
}

#[tokio::test]
async fn actual_journey_read_refuses_post_ack_publication_in_every_locomotion_profile() -> TestResult
{
    for action in [
        mush_core::locomotion::transition::ACTION,
        mush_core::locomotion::scene_transition::ACTION,
        ACTION,
    ] {
        let store = InMemoryStore::new();
        let mut rig = fixture(&store, ObjKind::Player).await?;
        let (source, destination, _) = start_scenes(&mut rig, &store).await?;
        let before = [
            entries(&rig, source)?.len(),
            entries(&rig, destination)?.len(),
        ];
        stage_action(&mut rig, "north", action)?;
        let revision = rig.world.engine().action_state().revision();
        let records = rig.world.engine().action_state().len();
        let (bundle, install) = rig
            .world
            .engine_mut()
            .take_staged_action()?
            .ok_or("staged")?
            .into_parts();
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await?;
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await?;
        // Deliberately violate the writer barrier to test the retained actual
        // Sim read. The slot incarnation and every SpatialState row stay fixed.
        rig.world
            .world_mut()
            .module_state_mut::<mush_sim::JourneyBook>()
            .ok_or("journeys")?
            .held
            .insert("cjourney:late".into(), "repair required".into());
        let mut out = Outbox::default();
        assert!(
            rig.world
                .engine_mut()
                .install_composed_action_announcing(install, &mut out, &mut Effects::default())
                .is_err(),
            "{action}"
        );
        assert!(out.is_empty());
        assert_eq!(location(&rig), Some(rig.source));
        assert_eq!(entries(&rig, source)?.len(), before[0]);
        assert_eq!(entries(&rig, destination)?.len(), before[1]);
        assert_eq!(rig.world.engine().action_state().revision(), revision);
        assert_eq!(rig.world.engine().action_state().len(), records);
        assert!(rig.world.engine().action_state().reservation_lost());
        assert_eq!(rig.moved.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[tokio::test]
async fn lost_ack_restores_exact_original_owner_proofs_and_transcripts_without_callbacks()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store, ObjKind::Player).await?;
    let (source, destination, _) = start_scenes(&mut rig, &store).await?;
    let planned = mush_scenes::movement::capture(
        scenes(&rig)?,
        rig.actor,
        Some(rig.source),
        rig.destination,
        100,
    )?;
    let before = [
        entries(&rig, source)?.len(),
        entries(&rig, destination)?.len(),
    ];
    stage_action(&mut rig, "north", ACTION)?;
    rig.world.world_mut().set_now(101);
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged")?
        .into_parts();
    let (_, receipt) = row(&bundle, "composed_locomotion")?;
    let receipt = &receipt["value"];
    assert_eq!(receipt["at_utc"], 100);
    assert_eq!(receipt["effects"].as_array().ok_or("effects")?.len(), 2);
    assert_eq!(
        receipt["effects"][0]["participant"]["key"]["owner"],
        "scenes"
    );
    assert_eq!(
        receipt["effects"][0]["proof"]["appends"]
            .as_array()
            .ok_or("appends")?
            .len(),
        2
    );
    let (_, crossing) = row(&bundle, "composed_locomotion_crossings")?;
    assert_eq!(crossing["value"]["coverage"], "uncertified_legacy_paths");
    assert_eq!(
        crossing["value"]["capture_digest"],
        receipt["capture_digest"]
    );
    assert!(!bundle.writes().iter().any(|write| matches!(write, ReplayWrite::ModuleJsonSet { key, .. }
        if key.as_str().starts_with("action-state/v1/locomotion_scene_transitions/") || key.as_str().starts_with("action-state/v1/locomotion_crossings/"))));
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    let committed = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    assert!(committed.matches_bundle(&bundle));
    assert_eq!(location(&rig), Some(rig.source));
    assert_eq!(entries(&rig, source)?.len(), before[0]);
    assert_eq!(entries(&rig, destination)?.len(), before[1]);
    drop(installation);
    drop(rig);
    let restarted = fixture(&store, ObjKind::Player).await?;
    assert_eq!(location(&restarted), Some(restarted.destination));
    for event in planned.events() {
        let actual = entries(&restarted, event.scene())?;
        assert_eq!(actual.last(), Some(event.event()));
        assert_eq!(actual.iter().filter(|row| *row == event.event()).count(), 1);
    }
    assert_eq!(restarted.moved.load(Ordering::SeqCst), 0);
    let IdempotencyStatus::FinalizedExact(retry) = store
        .authoritative_idempotency_status(bundle.idempotency(), bundle.ingress_hash())
        .await?
    else {
        return Err("retry not exact".into());
    };
    assert_eq!(retry.token(), committed.token());
    assert_eq!(retry.outcome(), committed.outcome());
    Ok(())
}

#[tokio::test]
async fn npc_home_and_empty_or_self_loop_observations_share_real_atomic_install() -> TestResult {
    for (kind, active, self_loop) in [
        (ObjKind::Npc, true, false),
        (ObjKind::Player, false, false),
        (ObjKind::Player, true, true),
    ] {
        let store = InMemoryStore::new();
        let mut rig = fixture(&store, kind).await?;
        if active {
            start_scenes(&mut rig, &store).await?;
        }
        if self_loop {
            let source = rig.source;
            rig.world.world_mut().set_home(rig.actor, Some(source))?;
            let exit = rig.world.world_mut().create(
                ObjKind::Exit,
                "circle",
                mush_core::GOD,
                Some(source),
            )?;
            rig.world.world_mut().link_exit(exit, Some(source))?;
            persist_legacy(&mut rig.world, &store).await?;
        }
        let before: Vec<_> = {
            let state = scenes(&rig)?;
            state
                .scenes
                .keys()
                .map(|id| (*id, state.events.next_seq(&id.to_string())))
                .collect()
        };
        let bundle = commit(
            &mut rig,
            &store,
            if self_loop { "circle" } else { "home" },
            ACTION,
            0xab,
        )
        .await?;
        assert_eq!(
            location(&rig),
            Some(if self_loop {
                rig.source
            } else {
                rig.destination
            })
        );
        let count = bundle.writes().iter().filter(|write| matches!(write, ReplayWrite::LedgerChunkSet { write } if write.namespace().as_str() == "scenes")).count();
        assert_eq!(count, if active && !self_loop { 2 } else { 0 });
        if count == 0 {
            for (id, next) in before {
                assert_eq!(scenes(&rig)?.events.next_seq(&id.to_string()), next);
            }
            let (_, receipt) = row(&bundle, "composed_locomotion")?;
            assert_eq!(
                receipt["value"]["effects"][0]["proof"]["appends"],
                json!([])
            );
        }
        assert_eq!(rig.moved.load(Ordering::SeqCst), 0);
        drop(rig);
        fixture(&store, kind).await?;
    }
    Ok(())
}

#[tokio::test]
async fn new_scene_before_custody_and_changed_second_tail_before_bind_both_refuse() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store, ObjKind::Player).await?;
    staff(&mut rig, &store).await?;
    stage_action(&mut rig, "n", ACTION)?;
    let _ = rig.world.run(rig.actor, "+timestop");
    assert!(rig.world.engine_mut().take_staged_action().is_err());
    assert_eq!(location(&rig), Some(rig.source));

    let store = InMemoryStore::new();
    let mut rig = fixture(&store, ObjKind::Player).await?;
    let (source, destination, _) = start_scenes(&mut rig, &store).await?;
    let destination_before = entries(&rig, destination)?;
    stage_action(&mut rig, "n", ACTION)?;
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged")?
        .into_parts();
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    let _ = rig.world.run(rig.actor, "say late entry");
    let source_after = entries(&rig, source)?;
    assert_eq!(
        source_after.last().ok_or("speech")?.kind,
        SceneEventKind::Said
    );
    assert!(
        rig.world
            .engine_mut()
            .install_composed_action(installation)
            .is_err()
    );
    assert_eq!(location(&rig), Some(rig.source));
    assert_eq!(entries(&rig, destination)?, destination_before);
    assert_eq!(entries(&rig, source)?, source_after);
    Ok(())
}

#[tokio::test]
async fn boot_refuses_missing_or_changed_original_events() -> TestResult {
    for mode in 0..2 {
        let store = InMemoryStore::new();
        let mut rig = fixture(&store, ObjKind::Player).await?;
        start_scenes(&mut rig, &store).await?;
        let bundle = commit(&mut rig, &store, "north", ACTION, 0xac).await?;
        let op = {
            let effect = bundle
                .writes()
                .iter()
                .find_map(|write| match write {
                    ReplayWrite::LedgerChunkSet { write }
                        if write.namespace().as_str() == "scenes" =>
                    {
                        Some(write)
                    }
                    _ => None,
                })
                .ok_or("scene effect")?;
            let key = mush_core::chunk_key(effect.log_id(), effect.chunk().chunk);
            if mode == 0 {
                ModuleWrite::Del { key }
            } else {
                let mut value = serde_json::to_value(effect.chunk())?;
                value["entries"]
                    .as_array_mut()
                    .ok_or("entries")?
                    .last_mut()
                    .ok_or("entry")?["value"]["at"] = json!(999);
                ModuleWrite::JsonSet { key, value }
            }
        };
        store
            .write(WriteBatch {
                serial: 999,
                module_ops: vec![ModuleOp {
                    ns: "scenes".into(),
                    op,
                }],
                durable: true,
                ..WriteBatch::default()
            })
            .await?;
        drop(rig);
        assert!(
            fixture(&store, ObjKind::Player).await.is_err(),
            "boot accepted corruption mode {mode}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn both_protocols_audit_original_positions_after_scene_closure() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store, ObjKind::Player).await?;
    let (source, destination, host) = start_scenes(&mut rig, &store).await?;
    commit(
        &mut rig,
        &store,
        "north",
        mush_core::locomotion::scene_transition::ACTION,
        0xad,
    )
    .await?;
    assert_eq!(location(&rig), Some(rig.destination));
    commit(&mut rig, &store, "south", ACTION, 0xae).await?;
    assert_eq!(location(&rig), Some(rig.source));
    let _ = rig.world.run(rig.actor, "+timestop/end");
    let _ = rig.world.run(host, "+timestop/end");
    for id in [source, destination] {
        assert_eq!(
            scenes(&rig)?.scenes.get(&id).ok_or("scene")?.status,
            SceneStatus::Closed
        );
    }
    persist_legacy(&mut rig.world, &store).await?;
    drop(rig);
    let restarted = fixture(&store, ObjKind::Player).await?;
    assert_eq!(location(&restarted), Some(restarted.source));
    for id in [source, destination] {
        assert_eq!(
            scenes(&restarted)?.scenes.get(&id).ok_or("scene")?.status,
            SceneStatus::Closed
        );
        assert!(entries(&restarted, id)?.is_empty());
    }
    Ok(())
}

#[tokio::test]
async fn sim_cold_audit_requires_both_exact_receipt_and_crossing_family() -> TestResult {
    async fn audit(records: Vec<mush_core::ActionStateBootRecord>) -> TestResult {
        // Hydrate the other production owners through their real boot path;
        // then inject the typed partition to exercise Sim's cold audit alone.
        let store = InMemoryStore::new();
        let mut rig = fixture(&store, ObjKind::Player).await?;
        rig.world.engine_mut().hydrate_action_state(records)?;
        rig.world
            .engine_mut()
            .all_modules_loaded(&mut Outbox::default(), &mut Effects::default())?;
        Ok(())
    }
    let store = InMemoryStore::new();
    let mut rig = fixture(&store, ObjKind::Player).await?;
    let bundle = commit(&mut rig, &store, "north", ACTION, 0xaf).await?;
    let records: Vec<_> = ["composed_locomotion", "composed_locomotion_crossings"]
        .into_iter()
        .map(|family| {
            row(&bundle, family).map(|(key, value)| {
                mush_core::ActionStateBootRecord::new("sim", key, value.clone())
            })
        })
        .collect::<TestResult<_>>()?;
    audit(records.clone()).await?;
    for record in &records {
        assert!(audit(vec![record.clone()]).await.is_err());
    }
    let mut changed = records;
    changed[1].value["value"]["at_utc"] = json!(9999);
    assert!(audit(changed).await.is_err());
    Ok(())
}
