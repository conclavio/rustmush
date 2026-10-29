#![cfg(test)]
//! Actual Scenes transcripts share the genuine Core/Sim/Atlas window commit.

use std::sync::atomic::Ordering;

use mush_core::{Effects, ObjKind, Outbox, ReplayWrite};
use mush_scenes::SceneEventKind;
use mush_sim::window_movement::WINDOW_MOVEMENT_LOG;
use mush_store::{
    AuthoritativeStore, InMemoryStore, ModuleOp, ModuleWrite, ObjectStore, WriteBatch,
};
use serde_json::json;

use super::dt_atomic_scene_movement::{entries, scenes, start_scenes};
use super::dt_locomotion::{fixture as restart, location};
use super::dt_window_movement::{commit, commit_line, entering, fixture, record, stage};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[tokio::test]
async fn actual_window_entry_and_original_scene_events_commit_and_recover_together() -> TestResult {
    for (kind, lost_ack) in [
        (ObjKind::Player, false),
        (ObjKind::Npc, false),
        (ObjKind::Player, true),
    ] {
        let store = InMemoryStore::new();
        let mut rig = entering(&store, kind).await?;
        let (source, destination, _) = start_scenes(&mut rig, &store).await?;
        let expected = mush_scenes::movement::capture(
            scenes(&rig)?,
            rig.actor,
            Some(rig.source),
            rig.destination,
            100,
        )?;
        assert_eq!(expected.events().len(), 2);
        let before = [entries(&rig, source)?, entries(&rig, destination)?];
        stage(&mut rig, 0xb1)?;
        let (bundle, installation) = rig
            .world
            .engine_mut()
            .take_staged_action()?
            .ok_or("staged")?
            .into_parts();
        assert_eq!(
            bundle
                .writes()
                .iter()
                .filter(|write| matches!(write,
            ReplayWrite::LedgerChunkSet {write} if write.namespace().as_str()=="scenes"))
                .count(),
            2
        );
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await?;
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await?;
        assert_eq!(location(&rig), Some(rig.source));
        assert_eq!(entries(&rig, source)?, before[0]);
        assert_eq!(entries(&rig, destination)?, before[1]);
        assert!(matches!(
            rig.world
                .world()
                .module_state::<mush_sim::SpatialState>()
                .ok_or("Sim")?
                .parent_of(rig.actor),
            mush_sim::SpatialParent::Unplaced
        ));
        if lost_ack {
            drop(installation);
        } else {
            let mut out = Outbox::default();
            rig.world.engine_mut().install_composed_action_announcing(
                installation,
                &mut out,
                &mut Effects::default(),
            )?;
            assert!(out.is_empty());
            assert_eq!(rig.moved.load(Ordering::SeqCst), 0);
        }
        let final_placement = record(&bundle)?
            .evidence()?
            .final_placement()
            .cloned()
            .ok_or("placement")?;
        drop(rig);
        for _ in 0..2 {
            let restarted = restart(&store, kind).await?;
            assert_eq!(location(&restarted), Some(restarted.destination));
            let native = restarted
                .world
                .world()
                .module_state::<mush_sim::SpatialState>()
                .ok_or("Sim")?;
            assert_eq!(
                native.placements.get(&restarted.actor),
                Some(&final_placement)
            );
            assert_eq!(native.recovery.next_seq(WINDOW_MOVEMENT_LOG), 1);
            assert_eq!(entries(&restarted, source)?.len(), before[0].len() + 1);
            assert_eq!(entries(&restarted, destination)?.len(), before[1].len() + 1);
            for event in expected.events() {
                assert_eq!(
                    entries(&restarted, event.scene())?.last(),
                    Some(event.event())
                );
            }
        }
        let mush_store::IdempotencyStatus::FinalizedExact(retry) = store
            .authoritative_idempotency_status(bundle.idempotency(), bundle.ingress_hash())
            .await?
        else {
            return Err("missing exact movement retry".into());
        };
        assert_eq!(retry.token(), bundle.token());
        assert_eq!(retry.bundle_hash(), bundle.bundle_hash());
    }
    Ok(())
}

#[tokio::test]
async fn ordinary_and_window_profiles_join_distinct_original_scene_positions() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store, ObjKind::Player, false).await?;
    let (source, destination, _) = start_scenes(&mut rig, &store).await?;
    let before = [
        entries(&rig, source)?.len(),
        entries(&rig, destination)?.len(),
    ];
    let rows = rig.world.engine().action_state().len();
    super::dt_locomotion::stage_action_with_nonce(
        &mut rig,
        "north",
        mush_core::locomotion::composed::ACTION,
        0xb4,
    )?;
    let (ordinary, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged")?
        .into_parts();
    store
        .reserve_authoritative_intent((*ordinary).clone())
        .await?;
    store
        .reconcile_authoritative(ordinary.token(), ordinary.bundle_hash())
        .await?;
    rig.world
        .engine_mut()
        .install_composed_action(installation)?;
    let window = commit_line(&mut rig, &store, "south", 0xb5).await?;
    assert_ne!(ordinary.token(), window.token());
    assert_eq!(location(&rig), Some(rig.source));
    assert_eq!(rig.world.engine().action_state().len(), rows + 4);
    drop(rig);
    for _ in 0..2 {
        let restarted = restart(&store, ObjKind::Player).await?;
        assert_eq!(location(&restarted), Some(restarted.source));
        assert_eq!(restarted.world.engine().action_state().len(), rows + 4);
        for (id, count, kinds) in [
            (
                source,
                before[0],
                [SceneEventKind::Left, SceneEventKind::Joined],
            ),
            (
                destination,
                before[1],
                [SceneEventKind::Joined, SceneEventKind::Left],
            ),
        ] {
            let events = entries(&restarted, id)?;
            assert_eq!(events.len(), count + 2);
            assert_eq!(events[count].kind, kinds[0]);
            assert_eq!(events[count + 1].kind, kinds[1]);
        }
    }
    Ok(())
}

#[tokio::test]
async fn changed_second_scene_tail_refuses_core_native_and_atlas_window_images() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = entering(&store, ObjKind::Player).await?;
    let (source, destination, _) = start_scenes(&mut rig, &store).await?;
    let destination_before = entries(&rig, destination)?;
    let rows = rig.world.engine().action_state().len();
    let known = rig
        .world
        .world()
        .module_state::<mush_atlas::AtlasState>()
        .ok_or("Atlas")?
        .travel
        .known
        .clone();
    stage(&mut rig, 0xb2)?;
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
    // Deliberate forbidden drift after custody: real speech advances the
    // second transcript tail, through the actual Scenes owner callback.
    let _ = rig.world.run(rig.actor, "say late window entry");
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
    let native = rig
        .world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("Sim")?;
    assert!(!native.placements.contains_key(&rig.actor));
    assert_eq!(native.recovery.next_seq(WINDOW_MOVEMENT_LOG), 0);
    assert_eq!(rig.world.engine().action_state().len(), rows);
    assert_eq!(
        rig.world
            .world()
            .module_state::<mush_atlas::AtlasState>()
            .ok_or("Atlas")?
            .travel
            .known,
        known
    );
    assert_eq!(entries(&rig, source)?, source_after);
    assert_eq!(entries(&rig, destination)?, destination_before);
    Ok(())
}

#[tokio::test]
async fn window_scene_history_refuses_missing_or_changed_events_before_any_boot_mutation()
-> TestResult {
    for missing in [false, true] {
        let store = InMemoryStore::new();
        let mut rig = entering(&store, ObjKind::Player).await?;
        start_scenes(&mut rig, &store).await?;
        let bundle = commit(&mut rig, &store, 0xb3).await?;
        let effect = bundle
            .writes()
            .iter()
            .find_map(|write| match write {
                ReplayWrite::LedgerChunkSet { write } if write.namespace().as_str() == "scenes" => {
                    Some(write)
                }
                _ => None,
            })
            .ok_or("actual scene append")?;
        let key = mush_core::chunk_key(effect.log_id(), effect.chunk().chunk);
        let op = if missing {
            ModuleWrite::Del { key }
        } else {
            let mut changed = serde_json::to_value(effect.chunk())?;
            changed["entries"]
                .as_array_mut()
                .ok_or("entries")?
                .last_mut()
                .ok_or("entry")?["value"]["at"] = json!(999);
            ModuleWrite::JsonSet {
                key,
                value: changed,
            }
        };
        store
            .write(WriteBatch {
                serial: 999,
                durable: true,
                module_ops: vec![ModuleOp {
                    ns: "scenes".into(),
                    op,
                }],
                ..Default::default()
            })
            .await?;
        drop(rig);
        super::dt_locomotion::assert_native_refusal_before_any_boot_mutation(&store, "scenes")
            .await?;
    }
    Ok(())
}
