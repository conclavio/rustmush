#![cfg(test)]
//! Genuine production Core/Sim/Atlas window commits and original history joins.

use super::dt_locomotion::{Rig, fixture as base_fixture, location, stage_definition_with_nonce};
use super::dt_unit_authority::persist_legacy;
use mush_core::{
    AuthoritativeBundle, BehaviorBinding, BehaviorId, Effects, GOD, ObjKind, Outbox, ReplayWrite,
};
use mush_sim::window_movement::{WINDOW_MOVEMENT_LOG, WindowMovementRecord};
use mush_store::{
    AuthoritativeStore, InMemoryStore, ModuleOp, ModuleWrite, ObjectStore, WriteBatch,
};
use serde_json::json;
use std::sync::atomic::Ordering;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn sim(rig: &Rig) -> TestResult<&mush_sim::SpatialState> {
    rig.world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("Sim unavailable".into())
}
pub(super) async fn fixture(store: &InMemoryStore, kind: ObjKind, placed: bool) -> TestResult<Rig> {
    let mut rig = super::dt_atlas_movement::mapped(store, kind, None).await?;
    rig.world.world_mut().attach_behavior(
        rig.actor,
        BehaviorId::new("sim.participant")?,
        BehaviorBinding {
            schema_version: 1,
            attached_by: GOD,
            attached_at: 100,
        },
    )?;
    if placed {
        rig.world.world_mut().attach_behavior(
            rig.source,
            BehaviorId::new("sim.domain")?,
            BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 100,
            },
        )?;
    }
    persist_legacy(&mut rig.world, store).await?;
    if placed {
        let domain = mush_sim::DomainRecord {
            id: mush_sim::DomainId(sim(&rig)?.next_domain_id),
            host: rig.source,
            driver: mush_sim::passive_definition_ref(),
            status: mush_sim::DomainStatus::Active,
            clock: mush_sim::DomainClock::Passive,
            presentation_location: rig.source,
            revision: 1,
            window: None,
        };
        let placement = mush_sim::PlacementRecord {
            object: rig.actor,
            parent: mush_sim::SpatialParent::Direct {
                domain: domain.id,
                pose: mush_sim::Pose::at(10, 20, 0, mush_sim::passive_rules_value(json!({}))?),
            },
            revision: 7,
            unplaced: None,
        };
        // Authored documents enter through the real Sim boot decoder. No
        // substitute owner, foreign mutable slot or physical receipt is seeded.
        store
            .write(WriteBatch {
                serial: 990,
                durable: true,
                module_ops: vec![
                    ModuleOp {
                        ns: "sim".into(),
                        op: ModuleWrite::JsonSet {
                            key: format!("domain:{}", domain.id.0).into(),
                            value: serde_json::to_value(domain)?,
                        },
                    },
                    ModuleOp {
                        ns: "sim".into(),
                        op: ModuleWrite::JsonSet {
                            key: format!("placement:{}", rig.actor.0).into(),
                            value: serde_json::to_value(placement)?,
                        },
                    },
                ],
                ..Default::default()
            })
            .await?;
        drop(rig);
        rig = base_fixture(store, kind).await?;
        assert!(matches!(
            sim(&rig)?.parent_of(rig.actor),
            mush_sim::SpatialParent::Direct { .. }
        ));
    }
    Ok(rig)
}
pub(super) fn stage(rig: &mut Rig, nonce: u8) -> TestResult {
    stage_line(rig, "north", nonce)
}

fn stage_line(rig: &mut Rig, line: &str, nonce: u8) -> TestResult {
    let definitions = rig
        .world
        .engine()
        .registry
        .definitions()
        .ok_or("definitions")?;
    let entry = definitions
        .iter()
        .find(|entry| {
            let key = entry.reference().key();
            key.owner.as_str() == "sim"
                && key.kind.as_str() == "action"
                && key.id.as_str() == mush_core::locomotion::composed::ACTION
                && entry.reference().version() == 2
        })
        .ok_or("registered window action")?;
    assert!(!entry.is_current());
    assert_eq!(
        definitions
            .current(entry.reference().key())
            .ok_or("v1 default")?
            .version(),
        1
    );
    let action = entry.reference().clone();
    rig.world
        .engine()
        .registry
        .action_providers()
        .ok_or("providers")?
        .get_exact(&action)?;
    stage_definition_with_nonce(rig, line, action, nonce)
}

pub(super) async fn entering(store: &InMemoryStore, kind: ObjKind) -> TestResult<Rig> {
    use mush_atlas::cells::{Cell, Cells};
    use mush_atlas::geom::{PointCm, RectCm};
    use mush_atlas::raster::Grid;
    use mush_atlas::record::SurfaceRef;
    use mush_atlas::terrain::TerrainClass;
    use mush_atlas::window::{Composite, SurfaceComposite, WindowEntry, WindowFrame};
    use mush_sim::WindowOpener;
    let mut rig = fixture(store, kind, false).await?;
    rig.world.world_mut().attach_behavior(
        rig.destination,
        BehaviorId::new("sim.domain")?,
        BehaviorBinding {
            schema_version: 1,
            attached_by: GOD,
            attached_at: 100,
        },
    )?;
    persist_legacy(&mut rig.world, store).await?;
    let id = sim(&rig)?.next_domain_id;
    let region = mush_atlas::RecordKey::new(
        mush_core::CanonicalId::new("dt_window")?,
        mush_core::CanonicalId::new("region")?,
    );
    let frame = WindowFrame {
        region: region.clone(),
        surfaces: vec![SurfaceRef::Ground],
        bbox: RectCm {
            min: PointCm { x: 0, y: 0 },
            max: PointCm { x: 300, y: 300 },
        },
        cell_cm: 100,
    };
    let surface = SurfaceComposite {
        surface: SurfaceRef::Ground,
        grid: Grid {
            origin: PointCm { x: 0, y: 0 },
            resolution_cm: 100,
            width: 3,
            height: 3,
        },
        cells: Cells::filled(
            3,
            3,
            Cell {
                class: TerrainClass::Paved,
                elev_cm: 0,
                cover: 0,
                room: 1,
            },
            9,
        )?,
        rooms: vec![rig.destination],
        features: vec![],
    };
    let entry = WindowEntry {
        domain: id,
        frame: frame.clone(),
        anchors: vec![rig.destination],
        rooms: std::collections::BTreeSet::from([rig.destination]),
        opened_by: WindowOpener::Staff { by: GOD },
        revision: 1,
        composite: Some(Composite {
            frame,
            surfaces: vec![surface],
            pins: vec![],
            features_seen: 0,
            digest: "authored_fixture_not_authority".into(),
        }),
        pending: None,
    };
    let domain = mush_sim::DomainRecord {
        id: mush_sim::DomainId(id),
        host: rig.destination,
        driver: mush_sim::passive_definition_ref(),
        status: mush_sim::DomainStatus::Active,
        clock: mush_sim::DomainClock::Passive,
        presentation_location: rig.destination,
        revision: 1,
        window: Some(mush_sim::WindowRec {
            region: region.to_string().into(),
            surfaces: vec![mush_sim::WindowSurface::Ground],
            bbox: mush_sim::WindowBox {
                x0: 0,
                y0: 0,
                x1: 300,
                y1: 300,
            },
            cell_cm: 100,
            composited_from: vec![],
            features_seen: 0,
            opened_by: mush_sim::WindowOpener::Staff { by: GOD },
            revision: 1,
        }),
    };
    // Both owners hydrate authored documents. The later commit and proof use
    // their actual registered adapters, not fixture owners or receipt bytes.
    store
        .write(WriteBatch {
            serial: 990,
            durable: true,
            module_ops: vec![
                ModuleOp {
                    ns: "sim".into(),
                    op: ModuleWrite::JsonSet {
                        key: format!("domain:{id}").into(),
                        value: serde_json::to_value(domain)?,
                    },
                },
                ModuleOp {
                    ns: "atlas".into(),
                    op: ModuleWrite::JsonSet {
                        key: entry.key(),
                        value: serde_json::to_value(entry)?,
                    },
                },
            ],
            ..Default::default()
        })
        .await?;
    drop(rig);
    let rig = base_fixture(store, kind).await?;
    let plan = mush_atlas::movement_window::plan(
        rig.world.world(),
        rig.actor,
        Some(rig.source),
        rig.destination,
        100,
    )?;
    assert_eq!(
        plan.projection_target()?
            .1
            .ok_or("selected target")?
            .domain(),
        id
    );
    Ok(rig)
}
pub(super) fn record(bundle: &AuthoritativeBundle) -> TestResult<WindowMovementRecord> {
    let value = bundle
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
                    .starts_with("action-state/v1/window_locomotion_crossings/") =>
            {
                Some(&value["value"])
            }
            _ => None,
        })
        .ok_or("window crossing record")?;
    Ok(WindowMovementRecord::decode(value)?)
}
pub(super) async fn commit(
    rig: &mut Rig,
    store: &InMemoryStore,
    nonce: u8,
) -> TestResult<Box<AuthoritativeBundle>> {
    commit_line(rig, store, "north", nonce).await
}

pub(super) async fn commit_line(
    rig: &mut Rig,
    store: &InMemoryStore,
    line: &str,
    nonce: u8,
) -> TestResult<Box<AuthoritativeBundle>> {
    let before_location = location(rig);
    let before_native = sim(rig)?.placements.get(&rig.actor).cloned();
    let before_rows = rig.world.engine().action_state().len();
    let before_journal = sim(rig)?.recovery.next_seq(WINDOW_MOVEMENT_LOG);
    stage_line(rig, line, nonce)?;
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
    // Store acknowledgement alone installs none of the participating owners.
    assert_eq!(location(rig), before_location);
    assert_eq!(sim(rig)?.placements.get(&rig.actor), before_native.as_ref());
    assert_eq!(rig.world.engine().action_state().len(), before_rows);
    assert_eq!(
        sim(rig)?.recovery.next_seq(WINDOW_MOVEMENT_LOG),
        before_journal
    );
    rig.world.engine_mut().install_composed_action_announcing(
        installation,
        &mut Outbox::default(),
        &mut Effects::default(),
    )?;
    Ok(bundle)
}

#[tokio::test]
async fn native_journal_core_receipt_and_actual_atlas_effect_share_one_commit() -> TestResult {
    for (kind, placed) in [
        (ObjKind::Player, false),
        (ObjKind::Npc, false),
        (ObjKind::Player, true),
    ] {
        let store = InMemoryStore::new();
        let mut rig = if placed {
            entering(&store, kind).await?
        } else {
            fixture(&store, kind, false).await?
        };
        let source = rig.source;
        let destination = rig.destination;
        let before = rig.world.engine().action_state().len();
        let recovery = sim(&rig)?.recovery.next_seq("recovery");
        stage(&mut rig, 0xe1)?;
        assert_eq!(location(&rig), Some(source));
        assert_eq!(sim(&rig)?.recovery.next_seq(WINDOW_MOVEMENT_LOG), 0);
        let (bundle, installation) = rig
            .world
            .engine_mut()
            .take_staged_action()?
            .ok_or("staged")?
            .into_parts();
        let record = record(&bundle)?;
        assert_eq!(record.commit(), bundle.token());
        assert_eq!(record.evidence()?.stage_count(), if placed { 2 } else { 1 });
        let journal = bundle
            .writes()
            .iter()
            .find_map(|write| match write {
                ReplayWrite::LedgerChunkSet { write }
                    if write.namespace().as_str() == "sim"
                        && write.log_id() == WINDOW_MOVEMENT_LOG =>
                {
                    write.chunk().entries.last()
                }
                _ => None,
            })
            .ok_or("native journal write")?;
        record.matches_journal_entry(journal.seq, &journal.value)?;
        assert!(bundle.writes().iter().any(|write| matches!(write,
            ReplayWrite::ModuleJsonSet {namespace,key,..} if namespace.as_str()=="sim"
                && key.as_str().starts_with("action-state/v1/window_composed_locomotion/"))));
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await?;
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await?;
        assert_eq!(location(&rig), Some(source));
        let mut out = Outbox::default();
        rig.world.engine_mut().install_composed_action_announcing(
            installation,
            &mut out,
            &mut Effects::default(),
        )?;
        assert!(out.is_empty());
        assert_eq!(location(&rig), Some(destination));
        assert_eq!(rig.moved.load(Ordering::SeqCst), 0);
        assert_eq!(rig.world.engine().action_state().len(), before + 2);
        assert_eq!(sim(&rig)?.recovery.next_seq("recovery"), recovery);
        assert_eq!(sim(&rig)?.recovery.next_seq(WINDOW_MOVEMENT_LOG), 1);
        assert_eq!(
            sim(&rig)?.placements.get(&rig.actor),
            record.evidence()?.final_placement()
        );
        let atlas = rig
            .world
            .world()
            .module_state::<mush_atlas::AtlasState>()
            .ok_or("Atlas")?;
        assert_eq!(
            atlas.travel.known.contains_key(&rig.actor),
            kind == ObjKind::Player
        );
        drop(rig);
        let restarted = base_fixture(&store, kind).await?;
        assert_eq!(location(&restarted), Some(destination));
        assert_eq!(sim(&restarted)?.recovery.next_seq(WINDOW_MOVEMENT_LOG), 1);
        assert_eq!(
            sim(&restarted)?.placements.get(&restarted.actor),
            record.evidence()?.final_placement()
        );
    }
    Ok(())
}

#[tokio::test]
async fn lost_ack_restores_the_genuine_window_pair_and_original_journal_once() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = entering(&store, ObjKind::Player).await?;
    let destination = rig.destination;
    stage(&mut rig, 0xe2)?;
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
    assert_eq!(location(&rig), Some(rig.source));
    assert_eq!(sim(&rig)?.recovery.next_seq(WINDOW_MOVEMENT_LOG), 0);
    drop(installation);
    drop(rig);
    let restarted = base_fixture(&store, ObjKind::Player).await?;
    assert_eq!(location(&restarted), Some(destination));
    let entries = sim(&restarted)?.recovery.open_entries(WINDOW_MOVEMENT_LOG);
    assert_eq!(entries.len(), 1);
    record(&bundle)?.matches_journal_entry(entries[0].seq, &entries[0].value)?;
    let mush_store::IdempotencyStatus::FinalizedExact(retry) = store
        .authoritative_idempotency_status(bundle.idempotency(), bundle.ingress_hash())
        .await?
    else {
        return Err("missing exact retry".into());
    };
    assert_eq!(retry.token(), bundle.token());
    assert_eq!(retry.bundle_hash(), bundle.bundle_hash());
    Ok(())
}

#[tokio::test]
async fn actual_window_repose_departure_and_return_preserve_each_original_native_phase()
-> TestResult {
    for kind in [ObjKind::Player, ObjKind::Npc] {
        let store = InMemoryStore::new();
        let mut rig = entering(&store, kind).await?;
        let around =
            rig.world
                .world_mut()
                .create(ObjKind::Exit, "around", GOD, Some(rig.destination))?;
        rig.world
            .world_mut()
            .link_exit(around, Some(rig.destination))?;
        persist_legacy(&mut rig.world, &store).await?;
        let rows = rig.world.engine().action_state().len();
        let recovery = sim(&rig)?.recovery.next_seq("recovery");
        let entry = commit(&mut rig, &store, 0xf0).await?;
        let entered = record(&entry)?
            .evidence()?
            .final_placement()
            .cloned()
            .ok_or("entry")?;
        assert!(matches!(
            entered.parent,
            mush_sim::SpatialParent::Direct { .. }
        ));

        let repose = commit_line(&mut rig, &store, "around", 0xf1).await?;
        let reposed_record = record(&repose)?;
        let reposed = reposed_record
            .evidence()?
            .final_placement()
            .cloned()
            .ok_or("repose")?;
        assert_eq!(reposed.revision, entered.revision + 1);
        assert!(matches!(
            reposed.parent,
            mush_sim::SpatialParent::Direct { .. }
        ));
        assert_eq!(reposed_record.evidence()?.stage_count(), 2);
        assert_eq!(location(&rig), Some(rig.destination));
        // The real native re-pose and journal survive even though Core's
        // ordinary same-room move has no object write or timestamp change.
        assert!(
            !repose
                .writes()
                .iter()
                .any(|write| matches!(write, ReplayWrite::ObjectSet { .. }))
        );

        let departure = commit_line(&mut rig, &store, "south", 0xf2).await?;
        let departed_record = record(&departure)?;
        let departed = departed_record
            .evidence()?
            .final_placement()
            .cloned()
            .ok_or("release")?;
        assert_eq!(departed.revision, reposed.revision + 1);
        assert_eq!(departed.parent, mush_sim::SpatialParent::Unplaced);
        assert_eq!(
            departed
                .unplaced
                .as_ref()
                .and_then(|info| info.previous.as_ref()),
            Some(&reposed.parent)
        );
        assert_eq!(departed_record.evidence()?.stage_count(), 2);
        assert_eq!(location(&rig), Some(rig.source));
        assert_eq!(sim(&rig)?.placements.get(&rig.actor), Some(&departed));
        drop(rig);
        rig = base_fixture(&store, kind).await?;
        assert_eq!(sim(&rig)?.placements.get(&rig.actor), Some(&departed));
        assert_eq!(location(&rig), Some(rig.source));

        let returned_bundle = commit(&mut rig, &store, 0xf3).await?;
        let returned = record(&returned_bundle)?
            .evidence()?
            .final_placement()
            .cloned()
            .ok_or("return")?;
        assert_eq!(returned.revision, departed.revision + 1);
        assert!(matches!(
            returned.parent,
            mush_sim::SpatialParent::Direct { .. }
        ));
        assert!(returned.unplaced.is_none());
        assert_eq!(rig.world.engine().action_state().len(), rows + 8);
        assert_eq!(rig.moved.load(Ordering::SeqCst), 0);
        assert_eq!(sim(&rig)?.recovery.next_seq("recovery"), recovery);
        drop(rig);
        let restarted = base_fixture(&store, kind).await?;
        assert_eq!(location(&restarted), Some(restarted.destination));
        assert_eq!(
            sim(&restarted)?.placements.get(&restarted.actor),
            Some(&returned)
        );
        let entries = sim(&restarted)?.recovery.open_entries(WINDOW_MOVEMENT_LOG);
        assert_eq!(entries.len(), 4);
        for (bundle, journal) in [entry, repose, departure, returned_bundle]
            .iter()
            .zip(entries)
        {
            record(bundle)?.matches_journal_entry(journal.seq, &journal.value)?;
        }
        assert_eq!(restarted.world.engine().action_state().len(), rows + 8);
    }
    Ok(())
}

#[tokio::test]
async fn lost_ack_of_actual_window_departure_restores_release_once() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = entering(&store, ObjKind::Player).await?;
    commit(&mut rig, &store, 0xf4).await?;
    let before = sim(&rig)?.placements.get(&rig.actor).cloned();
    stage_line(&mut rig, "south", 0xf5)?;
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged")?
        .into_parts();
    let released = record(&bundle)?
        .evidence()?
        .final_placement()
        .cloned()
        .ok_or("release")?;
    assert_eq!(released.parent, mush_sim::SpatialParent::Unplaced);
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    assert_eq!(location(&rig), Some(rig.destination));
    assert_eq!(sim(&rig)?.placements.get(&rig.actor), before.as_ref());
    assert_eq!(sim(&rig)?.recovery.next_seq(WINDOW_MOVEMENT_LOG), 1);
    drop(installation);
    drop(rig);
    for _ in 0..2 {
        let restarted = base_fixture(&store, ObjKind::Player).await?;
        assert_eq!(location(&restarted), Some(restarted.source));
        assert_eq!(
            sim(&restarted)?.placements.get(&restarted.actor),
            Some(&released)
        );
        let entries = sim(&restarted)?.recovery.open_entries(WINDOW_MOVEMENT_LOG);
        assert_eq!(entries.len(), 2);
        record(&bundle)?.matches_journal_entry(entries[1].seq, &entries[1].value)?;
    }
    let mush_store::IdempotencyStatus::FinalizedExact(retry) = store
        .authoritative_idempotency_status(bundle.idempotency(), bundle.ingress_hash())
        .await?
    else {
        return Err("missing exact release retry".into());
    };
    assert_eq!(retry.token(), bundle.token());
    assert_eq!(retry.bundle_hash(), bundle.bundle_hash());
    Ok(())
}

#[tokio::test]
async fn changed_missing_or_gapped_original_window_journals_refuse_before_any_boot_mutation()
-> TestResult {
    for mode in 0..3 {
        let store = InMemoryStore::new();
        let mut rig = entering(&store, ObjKind::Player).await?;
        commit(&mut rig, &store, 0xe3).await?;
        drop(rig);
        let key = mush_core::chunk_key(WINDOW_MOVEMENT_LOG, 0);
        let mut value = store.module_json_get("sim", &key).await?.ok_or("journal")?;
        if mode == 0 {
            value["entries"][0]["value"]["at_utc"] = json!(9999);
        } else if mode == 2 {
            value["entries"][0]["seq"] = json!(1);
        }
        let op = if mode == 1 {
            ModuleWrite::Del { key }
        } else {
            ModuleWrite::JsonSet { key, value }
        };
        store
            .write(WriteBatch {
                serial: 991,
                durable: true,
                module_ops: vec![ModuleOp {
                    ns: "sim".into(),
                    op,
                }],
                ..Default::default()
            })
            .await?;
        super::dt_locomotion::assert_native_refusal_before_any_boot_mutation(&store, "sim").await?;
    }
    Ok(())
}

#[tokio::test]
async fn non_window_placed_departure_preserves_the_real_core_hook_veto_and_installs_nothing()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store, ObjKind::Player, true).await?;
    let before = sim(&rig)?.placements.get(&rig.actor).cloned();
    let rows = rig.world.engine().action_state().len();
    assert!(stage(&mut rig, 0xe4).is_err());
    assert_eq!(location(&rig), Some(rig.source));
    assert_eq!(sim(&rig)?.placements.get(&rig.actor), before.as_ref());
    assert_eq!(sim(&rig)?.recovery.next_seq(WINDOW_MOVEMENT_LOG), 0);
    assert_eq!(rig.world.engine().action_state().len(), rows);
    assert!(rig.world.engine_mut().take_staged_action()?.is_none());
    assert!(
        store
            .module_json_get("sim", &mush_core::chunk_key(WINDOW_MOVEMENT_LOG, 0))
            .await?
            .is_none()
    );
    Ok(())
}

#[tokio::test]
async fn missing_window_pair_members_refuse_the_global_typed_phase_without_world_repair()
-> TestResult {
    async fn audit(
        records: Vec<mush_core::ActionStateBootRecord>,
        original_domain: u64,
        refused: bool,
    ) -> TestResult {
        let store = InMemoryStore::new();
        let mut rig = base_fixture(&store, ObjKind::Player).await?;
        // No current geometry for the original target: typed history must join
        // original proof data instead of reconstructing it from today's slot.
        assert!(
            !sim(&rig)?
                .domains
                .contains_key(&mush_sim::DomainId(original_domain))
        );
        assert!(
            !rig.world
                .world()
                .module_state::<mush_atlas::AtlasState>()
                .ok_or("Atlas")?
                .windows
                .open
                .contains_key(&original_domain)
        );
        rig.world.engine_mut().hydrate_action_state(records)?;
        let _ = rig.world.world_mut().take_dirty();
        let result = rig.world.engine_mut().audit_typed_history();
        if refused {
            assert!(result.is_err());
        } else {
            result?;
        }
        assert!(rig.world.world_mut().take_dirty().is_empty());
        assert!(rig.world.engine_mut().take_module_ops().is_empty());
        Ok(())
    }
    let store = InMemoryStore::new();
    let mut rig = entering(&store, ObjKind::Player).await?;
    let bundle = commit(&mut rig, &store, 0xe5).await?;
    let original_domain = record(&bundle)?.journal_value()["projection"]["target"]["domain"]
        .as_u64()
        .ok_or("original target")?;
    let records: Vec<_> = bundle
        .writes()
        .iter()
        .filter_map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if namespace.as_str() == "sim"
                && (key
                    .as_str()
                    .starts_with("action-state/v1/window_composed_locomotion/")
                    || key
                        .as_str()
                        .starts_with("action-state/v1/window_locomotion_crossings/")) =>
            {
                Some(mush_core::ActionStateBootRecord::new(
                    "sim",
                    key.as_str(),
                    value.clone(),
                ))
            }
            _ => None,
        })
        .collect();
    assert_eq!(records.len(), 2);
    audit(records.clone(), original_domain, false).await?;
    for record in records {
        audit(vec![record], original_domain, true).await?;
    }
    Ok(())
}

#[tokio::test]
async fn changed_core_preimages_after_ack_refuse_every_window_owner_image() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = entering(&store, ObjKind::Player).await?;
    let rows = rig.world.engine().action_state().len();
    stage(&mut rig, 0xe6)?;
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
        .world_mut()
        .get_mut(rig.destination)
        .ok_or("room")?
        .name = "changed after custody".into();
    let _ = rig.world.world_mut().take_dirty();
    let mut out = Outbox::default();
    assert!(
        rig.world
            .engine_mut()
            .install_composed_action_announcing(installation, &mut out, &mut Effects::default())
            .is_err()
    );
    assert!(out.is_empty());
    assert_eq!(location(&rig), Some(rig.source));
    assert!(!sim(&rig)?.placements.contains_key(&rig.actor));
    assert_eq!(sim(&rig)?.recovery.next_seq(WINDOW_MOVEMENT_LOG), 0);
    assert_eq!(rig.world.engine().action_state().len(), rows);
    assert!(
        !rig.world
            .world()
            .module_state::<mush_atlas::AtlasState>()
            .ok_or("Atlas")?
            .travel
            .known
            .contains_key(&rig.actor)
    );
    Ok(())
}
