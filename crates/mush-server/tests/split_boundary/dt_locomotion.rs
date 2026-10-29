//! Actual captured commands and Sim participant through Store ACK and cold boot.
//! This explicit test adapter does not add a live route or a legal-action offer.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use mush_core::action::{ActionInvocationDraft, ActionOfferId, PreparedCausalRef};
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_core::{
    ActionInvocation, ActionStateBootRecord, CanonicalHash, CanonicalId, CommitToken, DefinitionId,
    DefinitionKey, DefinitionKind, Effects, Engine, GOD, IdempotencyKey, IdempotencyScope, ObjId,
    ObjKind, Outbox, OwnerId, PerceivedRef, PrincipalId, ReplayWrite, RequestUid, RevisionSet,
    ScopedIdempotencyKey,
};
use mush_store::{AuthoritativeStore, IdempotencyStatus, InMemoryStore, ObjectStore};
use mush_testkit::TestWorld;
use serde_json::{Value, json};

use super::dt_unit_authority::persist_legacy;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub(super) struct Rig {
    pub(super) world: TestWorld,
    pub(super) actor: ObjId,
    pub(super) source: ObjId,
    pub(super) destination: ObjId,
    pub(super) moved: Arc<AtomicUsize>,
}

struct ObserveMoved(Arc<AtomicUsize>);
impl mush_core::GameModule for ObserveMoved {
    fn name(&self) -> &'static str {
        "dt_locomotion_test"
    }
    fn register(&mut self, reg: &mut mush_core::Registrar<'_>) -> Result<(), mush_core::ModError> {
        let calls = self.0.clone();
        reg.subscribe(
            mush_core::WorldEventKind::Moved,
            Arc::new(
                move |_: &mut mush_core::Ctx<'_>, _: &mush_core::WorldEvent| {
                    calls.fetch_add(1, Ordering::SeqCst);
                },
            ),
        );
        Ok(())
    }
}

pub(super) async fn fixture(store: &InMemoryStore, kind: ObjKind) -> TestResult<Rig> {
    let mut config = mush_config::config::Config::default();
    config.tacsim.enabled = true;
    let llm = Arc::new(mush_llm::LlmService::from_config(
        &mush_llm::LlmConfig::default(),
    )?);
    let mut modules = mush_server::boot::game_modules(&config, &llm)?;
    let moved = Arc::new(AtomicUsize::new(0));
    modules.push(Box::new(ObserveMoved(moved.clone())));
    let mut world = mush_server::persist::load_or_seed_world(store).await?;
    let existing = world
        .iter()
        .find(|object| object.name == "Captured locomotion actor")
        .map(|object| object.id);
    let (actor, source, destination) = if let Some(actor) = existing {
        let find = |name| {
            world
                .iter()
                .find(|object| object.name == name)
                .map(|object| object.id)
                .ok_or("fixture object missing")
        };
        (
            actor,
            find("Captured locomotion source")?,
            find("Captured locomotion destination")?,
        )
    } else {
        let source = world.create(ObjKind::Room, "Captured locomotion source", GOD, None)?;
        let destination =
            world.create(ObjKind::Room, "Captured locomotion destination", GOD, None)?;
        let actor = world.create(kind, "Captured locomotion actor", GOD, Some(source))?;
        let exit = world.create(ObjKind::Exit, "north;n", GOD, Some(source))?;
        world.link_exit(exit, Some(destination))?;
        let back = world.create(ObjKind::Exit, "south", GOD, Some(destination))?;
        world.link_exit(back, Some(source))?;
        world.set_home(actor, Some(destination))?;
        (actor, source, destination)
    };
    let mut engine =
        Engine::with_modules(world, mush_server::world_task::base_registry(), modules)?;
    mush_server::modules::boot_modules(&mut engine, store).await?;
    engine.world.set_now(100);
    engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x6a; 32])?,
        [],
    )?);
    let mut world = TestWorld::from_engine(engine);
    persist_legacy(&mut world, store).await?;
    Ok(Rig {
        world,
        actor,
        source,
        destination,
        moved,
    })
}

fn stage(rig: &mut Rig, line: &str) -> TestResult {
    stage_action(rig, line, mush_core::locomotion::transition::ACTION)
}

pub(super) fn stage_action(rig: &mut Rig, line: &str, action_id: &str) -> TestResult {
    stage_action_with_nonce(rig, line, action_id, 0xa9)
}

pub(super) fn stage_action_with_nonce(
    rig: &mut Rig,
    line: &str,
    action_id: &str,
    nonce: u8,
) -> TestResult {
    let token = CommitToken::from_bytes([nonce; 16]);
    let action = rig
        .world
        .engine()
        .registry
        .definitions()
        .ok_or("definitions")?
        .current(&DefinitionKey {
            owner: OwnerId::new("sim")?,
            kind: DefinitionKind::new("action")?,
            id: DefinitionId::new(action_id)?,
        })
        .cloned()
        .ok_or("locomotion action")?;
    let request = RequestUid::from_bytes([nonce; 16]);
    let principal = PrincipalId::Object { id: rig.actor };
    let invocation = ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: rig.actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([0xa9; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("sim")?,
                namespace: CanonicalId::new("locomotion_store_test")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?;
    let mut out = Outbox::default();
    let captured = rig
        .world
        .engine_mut()
        .capture_locomotion_command(rig.actor, line, &mut out, &mut Effects::default())
        .ok_or_else(|| format!("capture refused: {out:?}"))?;
    assert!(out.is_empty());
    let (prepared, output) = rig.world.engine_mut().prepare_captured_locomotion_action(
        captured,
        &action,
        &invocation,
        token,
    )?;
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        output.into_parts();
    definitions.push(action.clone());
    definitions.sort();
    definitions.dedup();
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token,
        request,
        idempotency: invocation.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([0xaa; 32]),
        request_hash: invocation.request_hash(),
        plan_hash: CanonicalHash::from_bytes([0xab; 32]),
        action,
        provider: OwnerId::new("sim")?,
        cause: PreparedCausalRef::new(request, None, 0)?,
        definitions,
        writes: prepared.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|resolution| resolution.bind(token)),
        semantic_events,
        eligible_witnesses,
        durable_outbox,
    })?;
    let guards = rig
        .world
        .engine()
        .registry
        .server_guard_set_builder(RevisionSet::default())?
        .finish()?;
    rig.world
        .engine_mut()
        .stage_authoritative_action(prepared, guards, bundle)?;
    Ok(())
}

pub(super) fn location(rig: &Rig) -> Option<ObjId> {
    rig.world
        .world()
        .get(rig.actor)
        .and_then(|actor| actor.location)
}

fn rows(bundle: &AuthoritativeBundle) -> Vec<ActionStateBootRecord> {
    bundle
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
                    .starts_with("action-state/v1/locomotion_transitions/")
                    || key
                        .as_str()
                        .starts_with("action-state/v1/locomotion_crossings/")) =>
            {
                Some(ActionStateBootRecord::new(
                    "sim",
                    key.to_string(),
                    value.clone(),
                ))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn lost_ack_restores_exact_movement_and_uncertified_pair_without_replaying_command()
-> TestResult {
    for (kind, line) in [(ObjKind::Player, "n"), (ObjKind::Npc, "home")] {
        let store = InMemoryStore::new();
        let mut rig = fixture(&store, kind).await?;
        let stamp = rig.world.world().world_ic_now();
        let before = rig.world.engine().action_state().len();
        stage(&mut rig, line)?;
        rig.world.world_mut().set_now(101);
        let (bundle, installation) = rig
            .world
            .engine_mut()
            .take_staged_action()?
            .ok_or("staged")?
            .into_parts();
        let records = rows(&bundle);
        assert_eq!(records.len(), 2);
        for record in &records {
            assert_eq!(
                record.value["value"]["prepared_at"],
                serde_json::to_value(&stamp)?
            );
            assert_eq!(record.value["value"]["at_utc"], json!(100));
        }
        let crossing = records
            .iter()
            .find(|record| record.key.contains("/locomotion_crossings/"))
            .ok_or("crossing")?;
        assert_eq!(
            crossing.value["value"]["coverage"],
            json!("uncertified_legacy_paths")
        );
        assert_eq!(crossing.value["value"]["crossings"], json!([]));
        assert_eq!(location(&rig), Some(rig.source));
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await?;
        let receipt = store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await?;
        assert!(receipt.matches_bundle(&bundle));
        assert_eq!(location(&rig), Some(rig.source));
        assert_eq!(rig.world.engine().action_state().len(), before);
        for record in &records {
            assert_eq!(
                store.module_json_get("sim", &record.key).await?,
                Some(record.value.clone())
            );
        }
        drop(installation);
        drop(rig);
        let restarted = fixture(&store, kind).await?;
        assert_eq!(location(&restarted), Some(restarted.destination));
        assert_eq!(restarted.world.engine().action_state().len(), before + 2);
        let IdempotencyStatus::FinalizedExact(retry) = store
            .authoritative_idempotency_status(bundle.idempotency(), bundle.ingress_hash())
            .await?
        else {
            return Err("retry did not return exact receipt".into());
        };
        assert_eq!(retry.token(), receipt.token());
        assert_eq!(retry.bundle_hash(), receipt.bundle_hash());
        assert_eq!(retry.outcome(), receipt.outcome());
        let sim = restarted
            .world
            .world()
            .module_state::<mush_sim::SpatialState>()
            .ok_or("Sim state")?;
        assert_eq!(
            sim.recovery
                .open_entries("recovery")
                .iter()
                .filter(|entry| entry.value["op"] == "locomotion_v1")
                .count(),
            1
        );
    }
    Ok(())
}

#[tokio::test]
async fn acknowledged_install_is_atomic_and_has_no_legacy_moved_callbacks() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store, ObjKind::Player).await?;
    let before = rig.world.engine().action_state().len();
    stage(&mut rig, "north")?;
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
    let mut fx = Effects::default();
    rig.world
        .engine_mut()
        .install_composed_action_announcing(installation, &mut out, &mut fx)?;
    assert_eq!(location(&rig), Some(rig.destination));
    assert_eq!(rig.world.engine().action_state().len(), before + 2);
    assert!(out.is_empty());
    assert!(!rig.world.engine().has_legacy_persistence_work());
    assert_eq!(rig.moved.load(Ordering::SeqCst), 0);
    let sim = rig
        .world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("Sim state")?;
    assert_eq!(
        sim.recovery
            .open_entries("recovery")
            .iter()
            .filter(|entry| entry.value["op"] == "locomotion_v1")
            .count(),
        1
    );
    let _ = rig.world.run(rig.actor, "south");
    assert_eq!(
        rig.moved.load(Ordering::SeqCst),
        1,
        "the observer is registered on the real legacy route"
    );
    Ok(())
}

#[tokio::test]
async fn carrier_transfer_records_real_crossings_without_creating_actor_placement() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store, ObjKind::Player).await?;
    let mut carriers = Vec::new();
    for (name, room) in [
        ("Departure carrier", rig.source),
        ("Arrival carrier", rig.destination),
    ] {
        let carrier =
            rig.world
                .world_mut()
                .create(ObjKind::Thing, name, GOD, Some(mush_core::LIMBO))?;
        for (object, behavior) in [(carrier, "sim.participant"), (room, "sim.interior")] {
            rig.world.world_mut().attach_behavior(
                object,
                mush_core::BehaviorId::new(behavior)?,
                mush_core::BehaviorBinding {
                    schema_version: 1,
                    attached_by: GOD,
                    attached_at: 100,
                },
            )?;
        }
        persist_legacy(&mut rig.world, &store).await?;
        let submission = super::dt_unit_authority::submit(
            &mut rig.world,
            GOD,
            &format!("@spatial/space #{}=moving:#{}", room.0, carrier.0),
        )?;
        super::dt_unit_authority::stage(&mut rig.world, &store, &submission).await?;
        let (bundle, installation) = rig
            .world
            .engine_mut()
            .take_staged_action()?
            .ok_or("space staged")?
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
        carriers.push(carrier);
    }
    stage(&mut rig, "n")?;
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("movement staged")?
        .into_parts();
    let records = rows(&bundle);
    let crossing = records
        .iter()
        .find(|record| record.key.contains("/locomotion_crossings/"))
        .ok_or("crossing")?;
    assert_eq!(
        crossing.value["value"]["crossings"],
        json!([
            {"character":rig.actor,"carrier":carriers[0],"direction":"exit"},
            {"character":rig.actor,"carrier":carriers[1],"direction":"enter"},
        ])
    );
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    rig.world
        .engine_mut()
        .install_composed_action(installation)?;
    assert_eq!(location(&rig), Some(rig.destination));
    let sim = rig
        .world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("Sim state")?;
    assert!(!sim.placements.contains_key(&rig.actor));
    assert!(!bundle.writes().iter().any(|write| matches!(write, ReplayWrite::ModuleJsonSet { key, .. } if key.as_str().starts_with("placement:"))));
    drop(rig);
    let restarted = fixture(&store, ObjKind::Player).await?;
    assert_eq!(location(&restarted), Some(restarted.destination));
    let sim = restarted
        .world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("Sim state")?;
    assert!(!sim.placements.contains_key(&restarted.actor));
    assert_eq!(
        store.module_json_get("sim", &crossing.key).await?,
        Some(crossing.value.clone())
    );
    Ok(())
}

fn audit(records: Vec<ActionStateBootRecord>) -> TestResult {
    let mut engine = Engine::with_modules(
        mush_core::World::new_minimal(),
        mush_core::Registry::new(),
        vec![Box::new(mush_sim::SimulationModule::new())],
    )?;
    engine.hydrate_action_state(records)?;
    engine.all_modules_loaded(&mut Outbox::default(), &mut Effects::default())?;
    Ok(())
}

pub(super) fn audit_before_projection_repair(
    records: Vec<ActionStateBootRecord>,
    refused: bool,
) -> TestResult {
    use mush_sim::{
        DomainClock, DomainId, DomainRecord, DomainStatus, PlacementRecord, Pose, SpatialParent,
    };
    let mut world = mush_core::World::new_minimal();
    let from = world.create(ObjKind::Room, "unrepaired source", GOD, None)?;
    let to = world.create(ObjKind::Room, "repair destination", GOD, None)?;
    let object = world.create(
        ObjKind::Thing,
        "unrelated repair candidate",
        GOD,
        Some(from),
    )?;
    for (id, behavior) in [(object, "sim.participant"), (to, "sim.domain")] {
        world.attach_behavior(
            id,
            mush_core::BehaviorId::new(behavior)?,
            mush_core::BehaviorBinding {
                schema_version: mush_core::BINDING_SCHEMA_VERSION,
                attached_by: GOD,
                attached_at: 0,
            },
        )?;
    }
    let domain = DomainRecord {
        id: DomainId(1),
        host: to,
        driver: mush_sim::passive_definition_ref(),
        status: DomainStatus::Active,
        clock: DomainClock::Passive,
        presentation_location: to,
        revision: 1,
        window: None,
    };
    let placement = PlacementRecord {
        object,
        parent: SpatialParent::Direct {
            domain: domain.id,
            pose: Pose::passive(mush_sim::passive_rules_value(json!({}))?),
        },
        revision: 1,
        unplaced: None,
    };
    let data = mush_core::ModuleBootData {
        kv: vec![
            ("domain:1".into(), serde_json::to_value(domain)?),
            (
                format!("placement:{}", object.0).into(),
                serde_json::to_value(placement)?,
            ),
        ],
        ..Default::default()
    };
    let mut config = mush_config::config::Config::default();
    config.tacsim.enabled = true;
    let llm = Arc::new(mush_llm::LlmService::from_config(
        &mush_llm::LlmConfig::default(),
    )?);
    let mut modules = mush_server::boot::game_modules(&config, &llm)?;
    modules.push(Box::new(ObserveMoved(Arc::new(AtomicUsize::new(0)))));
    let mut engine = Box::new(Engine::with_modules(
        world,
        mush_server::world_task::base_registry(),
        modules,
    )?);
    engine.hydrate_action_state(records)?;
    let _ = engine.world.take_dirty();
    let preflight = engine.audit_typed_history();
    if refused {
        assert!(
            preflight.is_err(),
            "global typed-history phase must reject the real Sim pair"
        );
    } else {
        preflight?;
    }
    let mut out = Outbox::new();
    let result = engine.module_loaded("sim", data, &mut out, &mut Effects::new());
    if refused {
        assert!(
            result.is_err(),
            "broken history must refuse in Sim's first load hook"
        );
        assert_eq!(
            engine.world.get(object).ok_or("object")?.location,
            Some(from)
        );
        assert!(engine.world.take_dirty().is_empty());
        assert!(out.is_empty());
        assert!(engine.take_module_ops().is_empty());
        assert!(
            engine
                .world
                .module_state::<mush_sim::SpatialState>()
                .ok_or("actual Sim")?
                .placements
                .is_empty()
        );
    } else {
        result?;
        assert_eq!(engine.world.get(object).ok_or("object")?.location, Some(to));
        assert!(
            engine
                .world
                .module_state::<mush_sim::SpatialState>()
                .ok_or("actual Sim")?
                .placements
                .contains_key(&object)
        );
    }
    Ok(())
}

#[tokio::test]
async fn actual_boot_requires_both_exact_receipt_and_crossing_records() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store, ObjKind::Player).await?;
    stage(&mut rig, "n")?;
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
    let records = rows(&bundle);
    assert_eq!(records.len(), 2);
    audit(records.clone())?;
    audit_before_projection_repair(records.clone(), false)?;
    for record in &records {
        assert!(audit(vec![record.clone()]).is_err());
        audit_before_projection_repair(vec![record.clone()], true)?;
    }
    let crossing = records
        .iter()
        .position(|record| record.key.contains("/locomotion_crossings/"))
        .ok_or("crossing")?;
    for field in ["actor", "source", "exit", "destination", "at_utc"] {
        let mut changed = records.clone();
        changed[crossing].value["value"][field] = json!(999999);
        audit_before_projection_repair(changed.clone(), true)?;
        assert!(audit(changed).is_err(), "accepted mismatched {field}");
    }
    for field in ["source", "exit"] {
        let mut changed = records.clone();
        changed[crossing].value["value"]
            .as_object_mut()
            .ok_or("record")?
            .remove(field);
        assert!(audit(changed).is_err(), "accepted missing {field}");
    }
    let mut wrong_key = records;
    let other = CommitToken::from_bytes([0xb0; 16]).to_string();
    wrong_key[crossing].key = format!("action-state/v1/locomotion_crossings/{other}").into();
    wrong_key[crossing].value["key"] = Value::String(other);
    assert!(audit(wrong_key).is_err());
    Ok(())
}
