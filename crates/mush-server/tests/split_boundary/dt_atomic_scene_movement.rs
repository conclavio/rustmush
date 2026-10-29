//! Real Scenes commands and the production joint provider through Store custody.
use std::sync::atomic::Ordering;

use mush_core::{Effects, Flags, GOD, ModuleOp, ModuleWrite, ObjId, ObjKind, Outbox, ReplayWrite};
use mush_scenes::{SceneEvent, SceneEventKind, SceneStatus, SceneUid, ScenesState};
use mush_store::{AuthoritativeStore, IdempotencyStatus, InMemoryStore, ObjectStore, WriteBatch};

use super::dt_locomotion::{Rig, fixture, location, stage_action};
use super::dt_unit_authority::persist_legacy;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const ACTION: &str = "sim_locomotion_with_scenes";

pub(super) fn scenes(rig: &Rig) -> TestResult<&ScenesState> {
    rig.world
        .world()
        .module_state::<ScenesState>()
        .ok_or("Scenes state missing".into())
}

pub(super) fn entries(rig: &Rig, id: SceneUid) -> TestResult<Vec<SceneEvent>> {
    scenes(rig)?
        .events
        .open_entries(&id.to_string())
        .iter()
        .map(|entry| serde_json::from_value(entry.value.clone()).map_err(Into::into))
        .collect()
}

pub(super) async fn staff(rig: &mut Rig, store: &InMemoryStore) -> TestResult {
    rig.world
        .world_mut()
        .get_mut(rig.actor)
        .ok_or("actor")?
        .flags
        .insert(Flags::WIZARD);
    rig.world.world_mut().set_connected(rig.actor, true);
    persist_legacy(&mut rig.world, store).await?;
    Ok(())
}

pub(super) async fn start_scenes(
    rig: &mut Rig,
    store: &InMemoryStore,
) -> TestResult<(SceneUid, SceneUid, ObjId)> {
    staff(rig, store).await?;
    let destination_host = rig.world.world_mut().create(
        ObjKind::Player,
        "Scene destination facilitator",
        GOD,
        Some(rig.destination),
    )?;
    rig.world.world_mut().set_connected(destination_host, true);
    let source_host = if rig
        .world
        .world()
        .get(rig.actor)
        .is_some_and(|actor| actor.kind == ObjKind::Player)
    {
        rig.actor
    } else {
        let host = rig.world.world_mut().create(
            ObjKind::Player,
            "Scene source facilitator",
            GOD,
            Some(rig.source),
        )?;
        rig.world.world_mut().set_connected(host, true);
        host
    };
    let _ = rig.world.run(source_host, "+timestop");
    let _ = rig.world.run(destination_host, "+timestop");
    let _ = rig.world.run(destination_host, "+timestop/next");
    let source = scenes(rig)?
        .active_scene_at(rig.source)
        .ok_or("source scene did not start")?
        .id;
    let destination = scenes(rig)?
        .active_scene_at(rig.destination)
        .ok_or("destination scene did not start")?
        .id;
    assert_ne!(source, destination);
    persist_legacy(&mut rig.world, store).await?;
    Ok((source, destination, destination_host))
}

fn planned(rig: &Rig) -> TestResult<mush_scenes::movement::MovementCapture> {
    Ok(mush_scenes::movement::capture(
        scenes(rig)?,
        rig.actor,
        Some(rig.source),
        rig.destination,
        100,
    )?)
}

#[tokio::test]
async fn lost_ack_restores_movement_and_original_transcripts_in_one_commit() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store, ObjKind::Player).await?;
    let (source, destination, _) = start_scenes(&mut rig, &store).await?;
    let capture = planned(&rig)?;
    assert_eq!(capture.events().len(), 2);
    assert_eq!(capture.events()[0].event().kind, SceneEventKind::Joined);
    assert_eq!(capture.events()[1].event().kind, SceneEventKind::Left);
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
    assert_eq!(location(&rig), Some(rig.source));
    assert_eq!(entries(&rig, source)?.len(), before[0]);
    assert_eq!(entries(&rig, destination)?.len(), before[1]);
    assert_eq!(
        bundle
            .writes()
            .iter()
            .filter(|write| matches!(write,
                ReplayWrite::LedgerChunkSet { write } if write.namespace().as_str() == "scenes"
            ))
            .count(),
        2
    );
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    assert!(receipt.matches_bundle(&bundle));
    // The acknowledgement is lost: neither the move nor transcript reaches RAM.
    assert_eq!(location(&rig), Some(rig.source));
    assert_eq!(entries(&rig, source)?.len(), before[0]);
    drop(installation);
    drop(rig);

    let restarted = fixture(&store, ObjKind::Player).await?;
    assert_eq!(location(&restarted), Some(restarted.destination));
    for event in capture.events() {
        let actual = entries(&restarted, event.scene())?;
        assert_eq!(actual.last(), Some(event.event()));
        assert_eq!(actual.iter().filter(|row| *row == event.event()).count(), 1);
        assert_eq!(
            scenes(&restarted)?
                .scenes
                .get(&event.scene())
                .ok_or("scene")?
                .status,
            SceneStatus::Paused
        );
    }
    assert_eq!(entries(&restarted, source)?.len(), before[0] + 1);
    assert_eq!(entries(&restarted, destination)?.len(), before[1] + 1);
    assert_eq!(restarted.moved.load(Ordering::SeqCst), 0);
    let IdempotencyStatus::FinalizedExact(retry) = store
        .authoritative_idempotency_status(bundle.idempotency(), bundle.ingress_hash())
        .await?
    else {
        return Err("retry did not return the exact movement receipt".into());
    };
    assert_eq!(retry.token(), receipt.token());
    assert_eq!(retry.bundle_hash(), receipt.bundle_hash());
    assert_eq!(retry.outcome(), receipt.outcome());
    Ok(())
}

#[tokio::test]
async fn acknowledged_install_publishes_both_logs_without_legacy_callback_replay() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store, ObjKind::Player).await?;
    start_scenes(&mut rig, &store).await?;
    let capture = planned(&rig)?;
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
    let mut out = Outbox::default();
    rig.world.engine_mut().install_composed_action_announcing(
        installation,
        &mut out,
        &mut Effects::default(),
    )?;
    assert_eq!(location(&rig), Some(rig.destination));
    for event in capture.events() {
        assert_eq!(entries(&rig, event.scene())?.last(), Some(event.event()));
    }
    assert_eq!(rig.moved.load(Ordering::SeqCst), 0);
    assert!(out.is_empty());
    assert!(!rig.world.engine().has_legacy_persistence_work());
    Ok(())
}

#[tokio::test]
async fn no_scene_and_self_loop_commit_without_fabricating_scene_writes() -> TestResult {
    for self_loop in [false, true] {
        let store = InMemoryStore::new();
        let mut rig = fixture(&store, ObjKind::Player).await?;
        let line = if self_loop {
            start_scenes(&mut rig, &store).await?;
            let exit =
                rig.world
                    .world_mut()
                    .create(ObjKind::Exit, "circle", GOD, Some(rig.source))?;
            rig.world.world_mut().link_exit(exit, Some(rig.source))?;
            persist_legacy(&mut rig.world, &store).await?;
            "circle"
        } else {
            "north"
        };
        let state = scenes(&rig)?;
        let before: Vec<_> = state
            .scenes
            .keys()
            .map(|id| (*id, state.events.next_seq(&id.to_string())))
            .collect();
        stage_action(&mut rig, line, ACTION)?;
        let (bundle, installation) = rig
            .world
            .engine_mut()
            .take_staged_action()?
            .ok_or("staged")?
            .into_parts();
        assert!(!bundle.writes().iter().any(|write| match write {
            ReplayWrite::LedgerChunkSet { write } => write.namespace().as_str() == "scenes",
            ReplayWrite::ModuleJsonSet { namespace, .. }
            | ReplayWrite::ModuleDelete { namespace, .. } => namespace.as_str() == "scenes",
            _ => false,
        }));
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await?;
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await?;
        rig.world
            .engine_mut()
            .install_composed_action(installation)?;
        assert_eq!(
            location(&rig),
            Some(if self_loop {
                rig.source
            } else {
                rig.destination
            })
        );
        for (id, count) in before {
            assert_eq!(scenes(&rig)?.events.next_seq(&id.to_string()), count);
        }
    }
    Ok(())
}

#[tokio::test]
async fn empty_capture_rechecks_new_scene_before_writer_custody() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store, ObjKind::Player).await?;
    staff(&mut rig, &store).await?;
    stage_action(&mut rig, "n", ACTION)?;
    let _ = rig.world.run(rig.actor, "+timestop");
    assert!(scenes(&rig)?.active_scene_at(rig.source).is_some());
    assert!(rig.world.engine_mut().take_staged_action().is_err());
    assert_eq!(location(&rig), Some(rig.source));
    assert_eq!(rig.moved.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn changed_second_ledger_tail_refuses_bind_before_any_owner_publishes() -> TestResult {
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
    // Simulate forbidden late state drift after writer custody to exercise the
    // install integrity fence. The destination append is first in capture order.
    let _ = rig.world.run(rig.actor, "say late scene entry");
    assert_eq!(
        entries(&rig, source)?.last().ok_or("speech")?.kind,
        SceneEventKind::Said
    );
    let source_after_speech = entries(&rig, source)?;
    assert!(
        rig.world
            .engine_mut()
            .install_composed_action(installation)
            .is_err()
    );
    assert_eq!(location(&rig), Some(rig.source));
    assert_eq!(entries(&rig, destination)?, destination_before);
    assert_eq!(entries(&rig, source)?, source_after_speech);
    Ok(())
}

#[tokio::test]
async fn cold_boot_refuses_missing_or_changed_committed_scene_events() -> TestResult {
    for missing in [false, true] {
        let store = InMemoryStore::new();
        let mut rig = fixture(&store, ObjKind::Player).await?;
        start_scenes(&mut rig, &store).await?;
        stage_action(&mut rig, "north", ACTION)?;
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
        let effect = bundle
            .writes()
            .iter()
            .find_map(|write| match write {
                ReplayWrite::LedgerChunkSet { write } if write.namespace().as_str() == "scenes" => {
                    Some(write)
                }
                _ => None,
            })
            .ok_or("Scenes write")?;
        let key = mush_core::chunk_key(effect.log_id(), effect.chunk().chunk);
        let op = if missing {
            ModuleWrite::Del { key }
        } else {
            let mut changed = serde_json::to_value(effect.chunk())?;
            let event = changed["entries"]
                .as_array_mut()
                .ok_or("entries")?
                .last_mut()
                .ok_or("entry")?;
            event["value"]["at"] = serde_json::json!(999);
            ModuleWrite::JsonSet {
                key,
                value: changed,
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
        drop(installation);
        drop(rig);
        assert!(
            fixture(&store, ObjKind::Player).await.is_err(),
            "boot accepted missing={missing} Scenes completion"
        );
    }
    Ok(())
}

#[tokio::test]
async fn cold_audit_accepts_original_events_in_later_closed_scene_history() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store, ObjKind::Player).await?;
    let (source, destination, host) = start_scenes(&mut rig, &store).await?;
    stage_action(&mut rig, "north", ACTION)?;
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
    rig.world
        .engine_mut()
        .install_composed_action(installation)?;
    let _ = rig.world.run(rig.actor, "+timestop/end");
    let _ = rig.world.run(host, "+timestop/end");
    for id in [source, destination] {
        assert_eq!(
            scenes(&rig)?.scenes.get(&id).ok_or("closed scene")?.status,
            SceneStatus::Closed
        );
        assert_eq!(
            entries(&rig, id)?.last().ok_or("terminal event")?.kind,
            SceneEventKind::Ended
        );
    }
    persist_legacy(&mut rig.world, &store).await?;
    drop(rig);
    let restarted = fixture(&store, ObjKind::Player).await?;
    assert_eq!(location(&restarted), Some(restarted.destination));
    for id in [source, destination] {
        assert_eq!(
            scenes(&restarted)?
                .scenes
                .get(&id)
                .ok_or("closed scene")?
                .status,
            SceneStatus::Closed
        );
        assert!(
            entries(&restarted, id)?.is_empty(),
            "closed log stays cold after its boot audit"
        );
    }
    Ok(())
}
