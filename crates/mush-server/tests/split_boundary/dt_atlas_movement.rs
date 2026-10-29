//! Real Atlas owner hydration, captured actions and durable knowledge recovery.

use std::sync::atomic::Ordering;

use mush_atlas::geom::Point2;
use mush_atlas::record::{Access, Footprint, RoomGeo, RoomKind, SurfaceRef};
use mush_atlas::travel::KnownDoc;
use mush_atlas::{AtlasState, RecordKey};
use mush_core::locomotion::composed::ACTION;
use mush_core::{
    AuthoritativeBundle, CanonicalId, Effects, ModuleBootData, ModuleOp, ModuleWrite, ObjId,
    ObjKind, Outbox, ReplayWrite,
};
use mush_store::{AuthoritativeStore, InMemoryStore, ObjectStore, WriteBatch};
use serde_json::{Value, json};

use super::dt_locomotion::{Rig, fixture, location, stage_action_with_nonce};
use super::dt_unit_authority::persist_legacy;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn key(name: &str) -> TestResult<RecordKey> {
    Ok(RecordKey::new(
        CanonicalId::new("dt_movement")?,
        CanonicalId::new(name)?,
    ))
}
fn geo(room: ObjId, neighborhood: &str) -> TestResult<RoomGeo> {
    Ok(RoomGeo {
        room,
        key: key(&format!("room_{}", room.0))?,
        neighborhood: key(neighborhood)?,
        site: None,
        surface: SurfaceRef::Ground,
        footprint: Footprint::Anchor {
            point: Point2(0.0, 0.0),
            radius_m: 1.0,
        },
        kind: RoomKind::Indoor,
        access: Access::Restricted,
        revision: 1,
    })
}
async fn write(store: &InMemoryStore, ops: Vec<ModuleWrite>) -> TestResult {
    store
        .write(WriteBatch {
            serial: 999,
            durable: true,
            module_ops: ops
                .into_iter()
                .map(|op| ModuleOp {
                    ns: "atlas".into(),
                    op,
                })
                .collect(),
            ..Default::default()
        })
        .await?;
    Ok(())
}
async fn mapped(
    store: &InMemoryStore,
    kind: ObjKind,
    before: Option<Vec<RecordKey>>,
) -> TestResult<Rig> {
    let rig = fixture(store, kind).await?;
    // Authored fixture documents enter through the real Atlas boot decoder,
    // rather than a cross-owner mutable World slot or substitute adapter.
    let mut ops = Vec::new();
    for (room, name) in [(rig.source, "source"), (rig.destination, "destination")] {
        ops.push(ModuleWrite::JsonSet {
            key: format!("roomgeo:{}", room.0).into(),
            value: serde_json::to_value(geo(room, name)?)?,
        });
    }
    if let Some(places) = before {
        ops.push(ModuleWrite::JsonSet {
            key: format!("known:{}", rig.actor.0).into(),
            value: serde_json::to_value(KnownDoc { places })?,
        });
    }
    write(store, ops).await?;
    drop(rig);
    let rig = fixture(store, kind).await?;
    assert_eq!(
        atlas(&rig)?
            .records
            .roomgeo
            .get(&rig.destination)
            .ok_or("geo")?
            .neighborhood,
        key("destination")?
    );
    Ok(rig)
}
fn atlas(rig: &Rig) -> TestResult<&AtlasState> {
    rig.world
        .world()
        .module_state::<AtlasState>()
        .ok_or("Atlas absent".into())
}
fn known(rig: &Rig) -> TestResult<Option<Vec<RecordKey>>> {
    Ok(atlas(rig)?.travel.known.get(&rig.actor).cloned())
}
fn receipt(bundle: &AuthoritativeBundle) -> TestResult<&Value> {
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
                    .starts_with("action-state/v1/composed_locomotion/") =>
            {
                Some(&value["value"])
            }
            _ => None,
        })
        .ok_or("composed receipt missing".into())
}
fn atlas_effect(bundle: &AuthoritativeBundle) -> TestResult<&Value> {
    receipt(bundle)?["effects"]
        .as_array()
        .ok_or("effects")?
        .iter()
        .find(|effect| effect["participant"]["key"]["owner"] == "atlas")
        .ok_or("Atlas proof missing".into())
}
fn atlas_writes(bundle: &AuthoritativeBundle) -> usize {
    bundle
        .writes()
        .iter()
        .filter(|write| match write {
            ReplayWrite::ModuleJsonSet { namespace, .. }
            | ReplayWrite::ModuleDelete { namespace, .. } => namespace.as_str() == "atlas",
            ReplayWrite::LedgerChunkSet { write } => write.namespace().as_str() == "atlas",
            _ => false,
        })
        .count()
}
async fn commit(
    rig: &mut Rig,
    store: &InMemoryStore,
    line: &str,
    nonce: u8,
) -> TestResult<Box<AuthoritativeBundle>> {
    stage_action_with_nonce(rig, line, ACTION, nonce)?;
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
    assert_eq!(rig.moved.load(Ordering::SeqCst), 0);
    assert!(!rig.world.engine().has_legacy_persistence_work());
    Ok(bundle)
}

#[tokio::test]
async fn lost_ack_recovers_actual_known_document_and_original_atlas_proof_in_the_same_commit()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = mapped(&store, ObjKind::Player, None).await?;
    stage_action_with_nonce(&mut rig, "north", ACTION, 0xc1)?;
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged")?
        .into_parts();
    assert_eq!(atlas_writes(&bundle), 2);
    assert!(known(&rig)?.is_none());
    assert_eq!(location(&rig), Some(rig.source));
    let effect = atlas_effect(&bundle)?;
    assert_eq!(effect["proof"]["history_next"], 0);
    assert_eq!(effect["proof"]["event"]["sequence"], 0);
    assert_eq!(
        effect["proof"]["capture"]["discovery"]["known_before"],
        Value::Null
    );
    let proof = effect.clone();
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    assert!(known(&rig)?.is_none());
    assert_eq!(location(&rig), Some(rig.source));
    assert_eq!(
        store
            .module_json_get("atlas", &format!("known:{}", rig.actor.0))
            .await?,
        Some(serde_json::to_value(KnownDoc {
            places: vec![key("destination")?]
        })?)
    );
    drop(installation);
    drop(rig);
    let restarted = fixture(&store, ObjKind::Player).await?;
    assert_eq!(location(&restarted), Some(restarted.destination));
    assert_eq!(known(&restarted)?, Some(vec![key("destination")?]));
    assert_eq!(restarted.moved.load(Ordering::SeqCst), 0);
    let stored = store
        .module_json_list("sim")
        .await?
        .into_iter()
        .find(|(key, _)| key.starts_with("action-state/v1/composed_locomotion/"))
        .ok_or("receipt")?
        .1;
    assert_eq!(stored["value"]["effects"][1], proof);
    Ok(())
}

#[tokio::test]
async fn fifo_eviction_and_duplicate_no_write_match_actual_knowledge_policy() -> TestResult {
    let before: Vec<_> = (0..512)
        .map(|n| key(&format!("place_{n}")))
        .collect::<TestResult<_>>()?;
    let store = InMemoryStore::new();
    let mut rig = mapped(&store, ObjKind::Player, Some(before.clone())).await?;
    let bundle = commit(&mut rig, &store, "n", 0xc2).await?;
    let expected: Vec<_> = before
        .into_iter()
        .skip(1)
        .chain([key("destination")?])
        .collect();
    assert_eq!(known(&rig)?, Some(expected.clone()));
    assert_eq!(atlas_writes(&bundle), 2);
    assert_eq!(
        atlas_effect(&bundle)?["proof"]["capture"]["discovery"]["change"]["evict_oldest"],
        1
    );
    drop(rig);
    let restarted = fixture(&store, ObjKind::Player).await?;
    assert_eq!(known(&restarted)?, Some(expected));

    let store = InMemoryStore::new();
    let before = vec![key("destination")?, key("earlier")?];
    let mut rig = mapped(&store, ObjKind::Player, Some(before.clone())).await?;
    let bundle = commit(&mut rig, &store, "north", 0xc3).await?;
    assert_eq!(atlas_writes(&bundle), 0);
    assert_eq!(atlas_effect(&bundle)?["proof"]["event"], Value::Null);
    assert_eq!(known(&rig)?, Some(before.clone()));
    drop(rig);
    assert_eq!(
        known(&fixture(&store, ObjKind::Player).await?)?,
        Some(before)
    );
    Ok(())
}

#[tokio::test]
async fn self_loop_still_learns_and_npc_or_unmapped_arrivals_have_checked_empty_images()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = mapped(&store, ObjKind::Player, Some(vec![])).await?;
    let exit =
        rig.world
            .world_mut()
            .create(ObjKind::Exit, "circle", mush_core::GOD, Some(rig.source))?;
    rig.world.world_mut().link_exit(exit, Some(rig.source))?;
    persist_legacy(&mut rig.world, &store).await?;
    let bundle = commit(&mut rig, &store, "circle", 0xc4).await?;
    assert_eq!(location(&rig), Some(rig.source));
    assert_eq!(known(&rig)?, Some(vec![key("source")?]));
    assert_eq!(receipt(&bundle)?["objects"], json!([]));
    assert_eq!(
        atlas_effect(&bundle)?["proof"]["capture"]["discovery"]["known_before"],
        json!([])
    );
    assert_eq!(atlas_writes(&bundle), 2);
    drop(rig);
    fixture(&store, ObjKind::Player).await?;
    for kind in [ObjKind::Npc, ObjKind::Player] {
        let store = InMemoryStore::new();
        let mut rig = if kind == ObjKind::Npc {
            mapped(&store, kind, None).await?
        } else {
            fixture(&store, kind).await?
        };
        let bundle = commit(&mut rig, &store, "home", 0xc5).await?;
        assert_eq!(atlas_writes(&bundle), 0);
        assert_eq!(atlas_effect(&bundle)?["proof"]["event"], Value::Null);
        assert!(known(&rig)?.is_none());
        drop(rig);
        fixture(&store, kind).await?;
    }
    Ok(())
}

#[tokio::test]
async fn later_real_legacy_visits_preserve_old_receipts_and_advance_the_no_op_boundary()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = mapped(&store, ObjKind::Player, None).await?;
    let original = commit(&mut rig, &store, "north", 0xc6).await?;
    let original_proof = atlas_effect(&original)?.clone();
    let _ = rig.world.run(rig.actor, "south");
    assert_eq!(location(&rig), Some(rig.source));
    assert_eq!(
        known(&rig)?,
        Some(vec![key("destination")?, key("source")?])
    );
    persist_legacy(&mut rig.world, &store).await?;
    drop(rig);
    let mut rig = fixture(&store, ObjKind::Player).await?;
    let second = commit(&mut rig, &store, "north", 0xc7).await?;
    assert_eq!(atlas_writes(&second), 0);
    assert_eq!(atlas_effect(&second)?["proof"]["history_next"], 2);
    assert_eq!(atlas_effect(&second)?["proof"]["event"], Value::Null);
    assert_eq!(atlas_effect(&original)?, &original_proof);
    drop(rig);
    assert_eq!(
        known(&fixture(&store, ObjKind::Player).await?)?,
        Some(vec![key("destination")?, key("source")?])
    );
    Ok(())
}

#[tokio::test]
async fn final_owner_bind_refuses_changed_geometry_or_knowledge_including_an_empty_image()
-> TestResult {
    for (duplicate, change_geo) in [(false, false), (false, true), (true, false)] {
        let store = InMemoryStore::new();
        let before = if duplicate {
            Some(vec![key("destination")?])
        } else {
            None
        };
        let mut rig = mapped(&store, ObjKind::Player, before).await?;
        let mut data = ModuleBootData {
            kv: store
                .module_json_list("atlas")
                .await?
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
            ..Default::default()
        };
        if change_geo {
            let (_, geo) = data
                .kv
                .iter_mut()
                .find(|(key, _)| key.as_str() == format!("roomgeo:{}", rig.destination.0))
                .ok_or("geo")?;
            geo["neighborhood"] = serde_json::to_value(key("changed")?)?;
        } else {
            data.kv
                .retain(|(key, _)| key.as_str() != format!("known:{}", rig.actor.0));
            data.kv.push((
                format!("known:{}", rig.actor.0).into(),
                serde_json::to_value(KnownDoc {
                    places: vec![key("changed")?],
                })?,
            ));
        }
        stage_action_with_nonce(&mut rig, "n", ACTION, 0xc8)?;
        let (bundle, installation) = rig
            .world
            .engine_mut()
            .take_staged_action()?
            .ok_or("staged")?
            .into_parts();
        assert_eq!(atlas_writes(&bundle), if duplicate { 0 } else { 2 });
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await?;
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await?;
        // Deliberately simulate forbidden late drift through the actual owner
        // loader, preserving the World slot identity and every Core object.
        rig.world.engine_mut().module_loaded(
            "atlas",
            data,
            &mut Outbox::default(),
            &mut Effects::default(),
        )?;
        let observed = known(&rig)?;
        assert!(
            rig.world
                .engine_mut()
                .install_composed_action(installation)
                .is_err()
        );
        assert_eq!(location(&rig), Some(rig.source));
        assert_eq!(known(&rig)?, observed);
        drop(rig);
        let restarted = fixture(&store, ObjKind::Player).await?;
        assert_eq!(location(&restarted), Some(restarted.destination));
        assert_eq!(known(&restarted)?, Some(vec![key("destination")?]));
    }
    Ok(())
}

#[tokio::test]
async fn cold_boot_refuses_missing_or_changed_actual_known_documents_and_original_journal_events()
-> TestResult {
    for mode in 0..4 {
        let store = InMemoryStore::new();
        let mut rig = mapped(&store, ObjKind::Player, None).await?;
        let bundle = commit(&mut rig, &store, "north", 0xc9).await?;
        let op = if mode < 2 {
            let key = format!("known:{}", rig.actor.0).into();
            if mode == 0 {
                ModuleWrite::Del { key }
            } else {
                ModuleWrite::JsonSet {
                    key,
                    value: serde_json::to_value(KnownDoc {
                        places: vec![self::key("wrong")?],
                    })?,
                }
            }
        } else {
            let effect = bundle
                .writes()
                .iter()
                .find_map(|write| match write {
                    ReplayWrite::LedgerChunkSet { write }
                        if write.namespace().as_str() == "atlas" =>
                    {
                        Some(write)
                    }
                    _ => None,
                })
                .ok_or("journal")?;
            let key = mush_core::chunk_key(effect.log_id(), effect.chunk().chunk);
            if mode == 2 {
                ModuleWrite::Del { key }
            } else {
                let mut value = serde_json::to_value(effect.chunk())?;
                value["entries"]
                    .as_array_mut()
                    .ok_or("entries")?
                    .last_mut()
                    .ok_or("entry")?["value"]["at_utc"] = json!(9999);
                ModuleWrite::JsonSet { key, value }
            }
        };
        write(&store, vec![op]).await?;
        drop(rig);
        assert!(
            fixture(&store, ObjKind::Player).await.is_err(),
            "boot accepted Atlas corruption mode {mode}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn genuine_older_partial_receipts_remain_partial_after_atlas_joins_the_composition()
-> TestResult {
    use mush_core::locomotion::scene_transition;
    use mush_core::{DefinitionRegistryBuilder, Engine, OwnerId};
    use mush_scenes::movement_composed::ComposedScenesMovementParticipant;
    use std::sync::{Arc, OnceLock};

    let store = InMemoryStore::new();
    let mut world = mush_server::persist::load_or_seed_world(&store).await?;
    let source = world.create(
        ObjKind::Room,
        "Captured locomotion source",
        mush_core::GOD,
        None,
    )?;
    let destination = world.create(
        ObjKind::Room,
        "Captured locomotion destination",
        mush_core::GOD,
        None,
    )?;
    let actor = world.create(
        ObjKind::Player,
        "Captured locomotion actor",
        mush_core::GOD,
        Some(source),
    )?;
    for (name, from, to) in [
        ("north;n", source, destination),
        ("south", destination, source),
    ] {
        let exit = world.create(ObjKind::Exit, name, mush_core::GOD, Some(from))?;
        world.link_exit(exit, Some(to))?;
    }
    world.set_home(actor, Some(destination))?;
    let scene_reader = Arc::new(OnceLock::new());
    let publish = Arc::clone(&scene_reader);
    // The old scope is produced by genuine owner registrations, requests,
    // grants and a captured command. Atlas is the real legacy-only module.
    let factory: mush_sim::ComposedMovementFactory = Arc::new(move |reg, builder| {
        let bad = |error: mush_core::participant::ParticipantError| {
            mush_core::ModError::msg(error.to_string())
        };
        let mut definitions = DefinitionRegistryBuilder::new();
        let owner = OwnerId::new("scenes").map_err(|e| mush_core::ModError::msg(e.to_string()))?;
        let participant = definitions
            .register(
                owner.clone(),
                mush_scenes::movement_composed::participant_descriptor()?,
            )
            .map_err(|e| mush_core::ModError::msg(e.to_string()))?;
        let observer = definitions
            .register(owner, scene_transition::observer_descriptor()?)
            .map_err(|e| mush_core::ModError::msg(e.to_string()))?;
        let service = builder
            .owner::<ComposedScenesMovementParticipant>(reg, participant.clone(), vec![observer])?
            .finish(reg)?;
        publish
            .set(service.owner_reader(&participant).map_err(bad)?)
            .map_err(|_| mush_core::ModError::msg("reader reused"))?;
        Ok(service)
    });
    let mut engine = Engine::with_modules(
        world,
        mush_server::world_task::base_registry(),
        vec![
            Box::new(mush_scenes::ScenesModule::new().with_composed_movement(scene_reader)),
            Box::new(mush_sim::SimulationModule::new().with_composed_movement(factory)),
            Box::new(mush_atlas::AtlasModule::new()),
        ],
    )?;
    mush_server::modules::boot_modules(&mut engine, &store).await?;
    engine.world.set_now(100);
    engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x6a; 32])?,
        [],
    )?);
    let mut old = Rig {
        world: mush_testkit::TestWorld::from_engine(engine),
        actor,
        source,
        destination,
        moved: Arc::default(),
    };
    persist_legacy(&mut old.world, &store).await?;
    let older = commit(&mut old, &store, "north", 0xcb).await?;
    assert_eq!(
        receipt(&older)?["effects"]
            .as_array()
            .ok_or("effects")?
            .len(),
        1
    );
    assert_eq!(
        receipt(&older)?["manifest"]["groups"]
            .as_array()
            .ok_or("groups")?
            .len(),
        1
    );
    assert_eq!(
        receipt(&older)?["manifest"]["entries"]
            .as_array()
            .ok_or("entries")?
            .len(),
        3
    );
    let (old_key, old_value) = older
        .writes()
        .iter()
        .find_map(|write| match write {
            ReplayWrite::ModuleJsonSet { key, value, .. }
                if key
                    .as_str()
                    .starts_with("action-state/v1/composed_locomotion/") =>
            {
                Some((key.to_string(), value.clone()))
            }
            _ => None,
        })
        .ok_or("old row")?;
    drop(old);
    let mut expanded = fixture(&store, ObjKind::Player).await?;
    assert_eq!(location(&expanded), Some(destination));
    let newer = commit(&mut expanded, &store, "south", 0xcc).await?;
    assert_eq!(
        receipt(&newer)?["effects"]
            .as_array()
            .ok_or("effects")?
            .len(),
        2
    );
    assert_eq!(
        receipt(&newer)?["manifest"]["groups"]
            .as_array()
            .ok_or("groups")?
            .len(),
        2
    );
    assert_ne!(
        receipt(&newer)?["manifest_digest"],
        receipt(&older)?["manifest_digest"]
    );
    assert_eq!(
        store.module_json_get("sim", &old_key).await?,
        Some(old_value)
    );
    drop(expanded);
    fixture(&store, ObjKind::Player).await?;
    Ok(())
}
