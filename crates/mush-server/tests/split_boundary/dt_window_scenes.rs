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
use super::dt_window_movement::{
    commit, commit_line, entering, fixture, projected_loop, record, stage,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[tokio::test]
async fn actual_two_leg_window_geometry_joins_both_discoveries_and_four_scene_events() -> TestResult
{
    for (kind, lost_ack) in [
        (ObjKind::Player, false),
        (ObjKind::Player, true),
        (ObjKind::Npc, false),
    ] {
        let store = InMemoryStore::new();
        let mut rig = projected_loop(&store, kind).await?;
        let (source, destination, _) = start_scenes(&mut rig, &store).await?;
        let before = [entries(&rig, source)?, entries(&rig, destination)?];
        let expected_events = mush_scenes::movement_window_capture::capture(
            scenes(&rig)?,
            rig.actor,
            Some(rig.source),
            rig.destination,
            rig.source,
            100,
        )?;
        assert_eq!(expected_events.events().count(), 4);
        let discovery = mush_atlas::movement_window_discovery::capture(
            rig.world.world(),
            rig.actor,
            Some(rig.source),
            rig.destination,
            rig.source,
            100,
        )?;
        let known = discovery.planned_known().map(|doc| doc.places);
        let mutations = discovery
            .legs()
            .iter()
            .filter(|leg| leg.planned_known().is_some())
            .count();
        assert_eq!(mutations, if kind == ObjKind::Player { 2 } else { 0 });
        let rows = rig.world.engine().action_state().len();
        stage(&mut rig, 0xb6)?;
        let (bundle, installation) = rig
            .world
            .engine_mut()
            .take_staged_action()?
            .ok_or("staged")?
            .into_parts();
        assert_eq!(
            record(&bundle)?.journal_value()["projected_room"],
            json!(rig.source),
            "actual projected return"
        );
        assert_eq!(
            bundle.writes().iter().find_map(|write| match write {
                ReplayWrite::ObjectSet { object } if object.id == rig.actor =>
                    Some(object.location),
                _ => None,
            }),
            Some(Some(rig.source)),
            "durable Core return image"
        );
        let writes: Vec<_> = bundle
            .writes()
            .iter()
            .filter_map(|write| match write {
                ReplayWrite::LedgerChunkSet { write } if write.namespace().as_str() == "atlas" => {
                    Some(write)
                }
                _ => None,
            })
            .collect();
        assert_eq!(writes.len(), usize::from(mutations > 0));
        if let Some(write) = writes.first() {
            assert_eq!(write.chunk().entries.len(), mutations);
            for (index, entry) in write.chunk().entries.iter().enumerate() {
                assert_eq!(entry.seq, index as u64);
                assert_eq!(
                    entry.value["source"]["commit"],
                    serde_json::to_value(bundle.token())?
                );
                assert_eq!(
                    entry.value["source"]["capture_digest"],
                    serde_json::to_value(discovery.digest()?)?
                );
            }
        }
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await?;
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await?;
        // Acknowledgement alone publishes none of the live owner images.
        assert_eq!(location(&rig), Some(rig.source));
        assert_eq!(entries(&rig, source)?, before[0]);
        assert_eq!(entries(&rig, destination)?, before[1]);
        let native = rig
            .world
            .world()
            .module_state::<mush_sim::SpatialState>()
            .ok_or("Sim")?;
        assert!(!native.placements.contains_key(&rig.actor));
        assert_eq!(native.recovery.next_seq(WINDOW_MOVEMENT_LOG), 0);
        assert!(atlas_known(&rig)?.is_none());
        assert_eq!(rig.world.engine().action_state().len(), rows);
        assert_eq!(rig.moved.load(Ordering::SeqCst), 0);
        let final_placement = record(&bundle)?
            .evidence()?
            .final_placement()
            .cloned()
            .ok_or("placement")?;
        if lost_ack {
            drop(installation);
        } else {
            rig.world
                .engine_mut()
                .install_composed_action(installation)?;
            assert_eq!(atlas_known(&rig)?, known);
            assert_eq!(location(&rig), Some(rig.source));
        }
        drop(rig);
        for _ in 0..2 {
            let restarted = restart(&store, kind).await?;
            assert_eq!(location(&restarted), Some(restarted.source));
            assert_eq!(atlas_known(&restarted)?, known);
            assert_eq!(
                restarted
                    .world
                    .world()
                    .module_state::<mush_sim::SpatialState>()
                    .ok_or("Sim")?
                    .placements
                    .get(&restarted.actor),
                Some(&final_placement)
            );
            assert_eq!(restarted.moved.load(Ordering::SeqCst), 0);
            for (scene, original) in [(source, &before[0]), (destination, &before[1])] {
                let events = entries(&restarted, scene)?;
                let expected: Vec<_> = expected_events
                    .events()
                    .filter(|event| event.scene() == scene)
                    .map(|event| event.event().clone())
                    .collect();
                assert_eq!(events.len(), original.len() + 2);
                assert_eq!(&events[original.len()..], expected.as_slice());
            }
        }
    }
    Ok(())
}

fn atlas_known(rig: &super::dt_locomotion::Rig) -> TestResult<Option<Vec<mush_atlas::RecordKey>>> {
    Ok(rig
        .world
        .world()
        .module_state::<mush_atlas::AtlasState>()
        .ok_or("Atlas")?
        .travel
        .known
        .get(&rig.actor)
        .cloned())
}

#[tokio::test]
async fn ordered_atlas_typed_audit_refuses_missing_disconnected_and_changed_second_leg_claims()
-> TestResult {
    async fn preflight(
        rows: Vec<mush_core::ActionStateBootRecord>,
    ) -> TestResult<Result<(), mush_core::ModError>> {
        let store = InMemoryStore::new();
        let mut rig = restart(&store, ObjKind::Player).await?;
        rig.world.engine_mut().hydrate_action_state(rows)?;
        Ok(rig.world.engine_mut().audit_typed_history())
    }
    let store = InMemoryStore::new();
    let mut rig = projected_loop(&store, ObjKind::Player).await?;
    let bundle = commit(&mut rig, &store, 0xb9).await?;
    let rows: Vec<_> = bundle
        .writes()
        .iter()
        .filter_map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if namespace.as_str() == "sim" && key.as_str().starts_with("action-state/") => Some(
                mush_core::ActionStateBootRecord::new("sim", key.as_str(), value.clone()),
            ),
            _ => None,
        })
        .collect();
    assert_eq!(rows.len(), 2);
    preflight(rows.clone()).await??;
    for fault in 0..8 {
        let mut changed = rows.clone();
        let row = changed
            .iter_mut()
            .find(|row| row.key.contains("/window_composed_locomotion/"))
            .ok_or("movement")?;
        let effect = row.value["value"]["movement"]["effects"]
            .as_array_mut()
            .ok_or("effects")?
            .iter_mut()
            .find(|effect| effect["participant"]["key"]["owner"] == "atlas")
            .ok_or("Atlas")?;
        match fault {
            0 => {
                effect["proof"]["legs"].as_array_mut().ok_or("legs")?.pop();
            }
            1 => effect["proof"]["legs"][1]["history_next"] = json!(0),
            2 => effect["proof"]["legs"][1]["event"] = serde_json::Value::Null,
            3 => effect["proof"]["legs"][1]["event"]["sequence"] = json!(0),
            4 => {
                effect["proof"]["legs"][1]
                    .as_object_mut()
                    .ok_or("leg")?
                    .remove("event");
            }
            5 => effect["proof"]["discovery"]["legs"][1]["to"] = json!(99999),
            6 => effect["proof"]["window"]["capture"]["at"] = json!(999),
            _ => effect["proof"]["extra"] = json!(true),
        }
        // Rebind only the generic proof hash so the actual Atlas grammar,
        // rather than a generic Core checksum, must reject the corruption.
        let digest = mush_core::DefinitionDigest::for_canonical_value_with_limit(
            "core.composed_locomotion.owner_proof.v1",
            &json!({"binding":effect["binding"],"capture_digest":effect["capture_digest"],"proof":effect["proof"]}),
            mush_core::locomotion::composed::MAX_MOVEMENT_PROOF_BYTES + 4096,
        )?;
        effect["proof_digest"] = serde_json::to_value(digest)?;
        let error = preflight(changed)
            .await?
            .err()
            .ok_or("malformed second leg accepted")?;
        assert!(
            error.to_string().contains("Atlas typed movement proof"),
            "{error}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn late_second_room_discovery_drift_refuses_all_prepared_window_owner_images() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = projected_loop(&store, ObjKind::Player).await?;
    let (source, destination, _) = start_scenes(&mut rig, &store).await?;
    let before = [entries(&rig, source)?, entries(&rig, destination)?];
    let rows = rig.world.engine().action_state().len();
    let mut data = mush_core::ModuleBootData {
        kv: store
            .module_json_list("atlas")
            .await?
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
        ..Default::default()
    };
    let (_, geo) = data
        .kv
        .iter_mut()
        .find(|(key, _)| key.as_str() == format!("roomgeo:{}", rig.source.0))
        .ok_or("source geometry")?;
    geo["neighborhood"] = json!({"pack":"dt_movement","entry":"late_second_room"});
    stage(&mut rig, 0xba)?;
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
    // Deliberate forbidden drift through the actual owner loader, preserving
    // the owner slot and every Core image. Only the second neighborhood moves.
    rig.world.engine_mut().module_loaded(
        "atlas",
        data,
        &mut Outbox::default(),
        &mut Effects::default(),
    )?;
    assert!(
        rig.world
            .engine_mut()
            .install_composed_action(installation)
            .is_err()
    );
    assert_eq!(location(&rig), Some(rig.source));
    assert!(atlas_known(&rig)?.is_none());
    assert_eq!(entries(&rig, source)?, before[0]);
    assert_eq!(entries(&rig, destination)?, before[1]);
    assert_eq!(rig.world.engine().action_state().len(), rows);
    let native = rig
        .world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("Sim")?;
    assert!(!native.placements.contains_key(&rig.actor));
    assert_eq!(native.recovery.next_seq(WINDOW_MOVEMENT_LOG), 0);
    Ok(())
}

#[tokio::test]
async fn ordered_window_discovery_recovers_fifo_and_duplicate_no_op_boundaries() -> TestResult {
    for mode in 0..4 {
        let store = InMemoryStore::new();
        let rig = projected_loop(&store, ObjKind::Player).await?;
        let atlas = rig
            .world
            .world()
            .module_state::<mush_atlas::AtlasState>()
            .ok_or("Atlas")?;
        let source = atlas
            .records
            .roomgeo
            .get(&rig.source)
            .ok_or("source")?
            .neighborhood
            .clone();
        let destination = atlas
            .records
            .roomgeo
            .get(&rig.destination)
            .ok_or("destination")?
            .neighborhood
            .clone();
        let before = match mode {
            0 => std::iter::once(source.clone())
                .chain(
                    (0..511)
                        .map(|index| {
                            Ok(mush_atlas::RecordKey::new(
                                mush_core::CanonicalId::new("dt_fifo")?,
                                mush_core::CanonicalId::new(format!("old_{index}"))?,
                            ))
                        })
                        .collect::<TestResult<Vec<_>>>()?,
                )
                .collect(),
            1 => vec![source.clone()],
            2 => vec![source, destination],
            _ => vec![destination],
        };
        let actor = rig.actor;
        store
            .write(WriteBatch {
                serial: 999,
                durable: true,
                module_ops: vec![ModuleOp {
                    ns: "atlas".into(),
                    op: ModuleWrite::JsonSet {
                        key: format!("known:{}", actor.0).into(),
                        value: serde_json::to_value(mush_atlas::travel::KnownDoc {
                            places: before.clone(),
                        })?,
                    },
                }],
                ..Default::default()
            })
            .await?;
        drop(rig);
        let mut rig = restart(&store, ObjKind::Player).await?;
        let capture = mush_atlas::movement_window_discovery::capture(
            rig.world.world(),
            actor,
            Some(rig.source),
            rig.destination,
            rig.source,
            100,
        )?;
        let changes = capture
            .legs()
            .iter()
            .filter(|leg| leg.planned_known().is_some())
            .count();
        assert_eq!(
            changes,
            match mode {
                0 => 2,
                2 => 0,
                _ => 1,
            }
        );
        let expected = capture.planned_known().map_or(before, |doc| doc.places);
        let bundle = commit(&mut rig, &store, 0xb7).await?;
        assert_eq!(bundle.writes().iter().filter(|write| matches!(write, ReplayWrite::ModuleJsonSet {namespace, key, ..} if namespace.as_str()=="atlas" && key.as_str().starts_with("known:"))).count(), usize::from(changes > 0));
        assert_eq!(atlas_known(&rig)?, Some(expected.clone()));
        drop(rig);
        for _ in 0..2 {
            assert_eq!(
                atlas_known(&restart(&store, ObjKind::Player).await?)?,
                Some(expected.clone())
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn ordered_window_history_requires_each_original_discovery_mutation() -> TestResult {
    for missing in [false, true] {
        let store = InMemoryStore::new();
        let mut rig = projected_loop(&store, ObjKind::Player).await?;
        start_scenes(&mut rig, &store).await?;
        let bundle = commit(&mut rig, &store, 0xb8).await?;
        let write = bundle
            .writes()
            .iter()
            .find_map(|write| match write {
                ReplayWrite::LedgerChunkSet { write } if write.namespace().as_str() == "atlas" => {
                    Some(write)
                }
                _ => None,
            })
            .ok_or("knowledge journal")?;
        assert_eq!(write.chunk().entries.len(), 2);
        let mut value = serde_json::to_value(write.chunk())?;
        if missing {
            value["entries"].as_array_mut().ok_or("entries")?.pop();
        } else {
            value["entries"][1]["value"]["at_utc"] = json!(999);
        }
        store
            .write(WriteBatch {
                serial: 999,
                durable: true,
                module_ops: vec![ModuleOp {
                    ns: "atlas".into(),
                    op: ModuleWrite::JsonSet {
                        key: mush_core::chunk_key(write.log_id(), write.chunk().chunk),
                        value,
                    },
                }],
                ..Default::default()
            })
            .await?;
        drop(rig);
        super::dt_locomotion::assert_native_refusal_before_any_boot_mutation(&store, "atlas")
            .await?;
    }
    Ok(())
}

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
