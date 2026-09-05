use mush_core::{
    AuthoritativeBundle, CanonicalId, Effects, GOD, IdempotencyKey, IdempotencyScope, Inbound,
    IngressEnvelope, IngressPayload, IngressSurfaceVersion, LegalActionQuery,
    OfferExecutionContext, OfferKeyRing, OfferSecret, OfferSubmission, Outbox, OwnerId,
    PerceivedRef, PrincipalId, RequestUid, ResolutionKeyRing, ResolutionSecret,
    ScopedIdempotencyKey, StagedActionPersistence,
};
use mush_jobs::JobsModule;
use mush_proto::ProjectedActionStatus;
use mush_server::action_coordinator::{ActionCoordinator, ActionPreflight};
use mush_store::{AuthoritativeStore, InMemoryStore};
use mush_testkit::TestWorld;
use ruleset_shadowrun::ShadowrunRuleset;
use ruleset_wod::WodRuleset;
use serde_json::{Value, json};

fn install_action_keys(world: &mut TestWorld) {
    world
        .engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x41; 32]).unwrap(), []).unwrap());
    world.engine_mut().set_resolution_keys(
        ResolutionKeyRing::new(ResolutionSecret::new(1, [0x52; 32]).unwrap(), []).unwrap(),
    );
}

fn request_identity(
    owner: &str,
    namespace: &str,
    actor: mush_core::ObjId,
    byte: u8,
) -> (RequestUid, PrincipalId, ScopedIdempotencyKey) {
    let request = RequestUid::from_bytes([byte; 16]);
    let principal = PrincipalId::Object { id: actor };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: OwnerId::new(owner).unwrap(),
            namespace: CanonicalId::new(namespace.replace('-', "_")).unwrap(),
        },
        0,
        IdempotencyKey::Request(request),
    );
    (request, principal, idempotency)
}

fn query_panel(world: &mut TestWorld, actor: mush_core::ObjId, name: &str) -> Value {
    let mut out = Outbox::new();
    let mut effects = Effects::new();
    world.engine_mut().step(
        Inbound::Query {
            conn: 1,
            actor,
            id: 91,
            name: name.into(),
            args: json!({}),
        },
        &mut out,
        &mut effects,
    );
    out.events()
        .iter()
        .find_map(|(_, event)| match event {
            mush_proto::OutputEvent::Reply { id: 91, event } => match event.as_ref() {
                mush_proto::OutputEvent::Panel { data, .. } => Some(data.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("query returned a panel keyframe")
}

async fn execute_offer(
    world: &mut TestWorld,
    store: &InMemoryStore,
    query: LegalActionQuery,
    owner: &str,
    verb: &str,
    parameters: Value,
    byte: u8,
) -> AuthoritativeBundle {
    let offer = world
        .engine()
        .project_legal_actions(&query)
        .unwrap()
        .projected()
        .into_iter()
        .find(|offer| offer.presentation().verb == verb)
        .unwrap_or_else(|| panic!("missing projected {verb} offer"));
    let (request, principal, idempotency) = request_identity(owner, verb, query.actor(), byte);
    let envelope = IngressEnvelope::new(
        request,
        principal.clone(),
        IngressSurfaceVersion::new(CanonicalId::new("test.production_action").unwrap(), 1).unwrap(),
        IngressPayload::structured(json!({
            "offer": offer.offer(),
            "subject": offer.subject(),
            "targets": [],
            "parameters": parameters,
        }))
        .unwrap(),
        idempotency.clone(),
    )
    .unwrap();
    let status = ActionCoordinator::status_id(&idempotency).unwrap();
    let permit = match ActionCoordinator::preflight(store, envelope).await.unwrap() {
        ActionPreflight::Vacant(permit) => permit,
        other => panic!("production action was not vacant: {other:?}"),
    };
    let resolved = world
        .engine()
        .resolve_legal_action(
            &world.engine().project_legal_actions(&query).unwrap(),
            OfferExecutionContext {
                request,
                principal,
                actor: query.actor(),
                idempotency,
                viewer_scope: query.viewer_scope(),
                now: None,
            },
            OfferSubmission {
                offer: offer.offer(),
                subject: offer.subject().clone(),
                targets: Vec::new(),
                parameters,
            },
        )
        .unwrap();
    ActionCoordinator::prepare_and_stage_resolved(permit, world.engine_mut(), resolved).unwrap();
    let (persistence, installation) = world
        .engine_mut()
        .take_staged_action()
        .unwrap()
        .expect("production action staged")
        .into_parts();
    let bundle = match persistence {
        StagedActionPersistence::Authoritative(bundle) => *bundle,
        StagedActionPersistence::Legacy(_) => panic!("production action used the legacy lane"),
    };
    store
        .reserve_authoritative_intent(bundle.clone())
        .await
        .unwrap();
    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    assert!(receipt.matches_bundle(&bundle));
    world.engine_mut().install_prepared_action(installation);
    store
        .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    let projections = world
        .engine()
        .registry
        .action_providers()
        .unwrap()
        .receipt_projection_table();
    let ProjectedActionStatus::Committed(projected) = ActionCoordinator::projected_status(
        store,
        &projections,
        &PrincipalId::Object { id: query.actor() },
        &status,
    )
    .await
    .unwrap() else {
        panic!("checkpointed production action did not project a committed receipt");
    };
    assert_eq!(
        projected
            .result()
            .expect("provider result projection")
            .kind(),
        verb.replace('-', "_")
    );
    bundle
}

#[tokio::test]
async fn wod_willpower_roll_spends_once_and_records_resolution() {
    let store = InMemoryStore::new();
    let mut world = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(7)),
    ]);
    let actor = world.spawn_player("Nick");
    world.run(GOD, "@sheet/set Nick/strength=3");
    world.run(GOD, "@sheet/set Nick/brawl=2");
    world.run(GOD, "@sheet/set Nick/willpower=2");
    world.run(GOD, "@sheet/set Nick/willpower_temp=9");
    let _ = world.world_mut().take_dirty();
    let _ = world.take_module_ops();
    install_action_keys(&mut world);

    let query = world
        .engine()
        .legal_action_query(
            PrincipalId::Object { id: actor },
            actor,
            PerceivedRef::SelfActor,
        )
        .unwrap();
    let first = execute_offer(
        &mut world,
        &store,
        query,
        "wod",
        "willpower-roll",
        json!({ "pool": "strength+brawl", "difficulty": 6 }),
        0x31,
    )
    .await;
    assert_eq!(first.outcome().data()["pool"], 5);
    assert_eq!(first.outcome().data()["willpower_remaining"], 1);
    assert_eq!(first.outcome().data()["roll_sequence"], 1);
    let resolution = first.resolution().expect("roll records deterministic RNG");
    assert_eq!(resolution.outcome(), first.outcome());
    assert!(!resolution.draws().is_empty());
    assert_eq!(
        query_panel(&mut world, actor, "wod.sheet")["data"]["willpower_temp"],
        1
    );

    let query = world
        .engine()
        .legal_action_query(
            PrincipalId::Object { id: actor },
            actor,
            PerceivedRef::SelfActor,
        )
        .unwrap();
    let second = execute_offer(
        &mut world,
        &store,
        query,
        "wod",
        "willpower-roll",
        json!({ "pool": "strength+brawl", "difficulty": 6 }),
        0x32,
    )
    .await;
    assert_eq!(second.outcome().data()["willpower_remaining"], 0);
    assert_eq!(second.outcome().data()["roll_sequence"], 2);

    let query = world
        .engine()
        .legal_action_query(
            PrincipalId::Object { id: actor },
            actor,
            PerceivedRef::SelfActor,
        )
        .unwrap();
    assert!(
        world
            .engine()
            .project_legal_actions(&query)
            .unwrap()
            .projected()
            .into_iter()
            .all(|offer| offer.presentation().verb != "willpower-roll")
    );

    let staff_query = world
        .engine()
        .legal_action_query(
            PrincipalId::Object { id: GOD },
            GOD,
            PerceivedRef::KnownObject {
                id: actor,
                revision: 1,
            },
        )
        .unwrap();
    let restored = execute_offer(
        &mut world,
        &store,
        staff_query,
        "wod",
        "willpower-set",
        json!({ "current": 2 }),
        0x33,
    )
    .await;
    assert_eq!(restored.outcome().data()["current"], 2);
    assert_eq!(restored.outcome().data()["roll_sequence"], 2);
    assert_eq!(
        query_panel(&mut world, actor, "wod.sheet")["data"]["willpower_temp"],
        2
    );
    let legacy_set = world.run(GOD, "@sheet/set Nick/willpower_temp=1");
    assert!(
        world
            .seen_by(&legacy_set, GOD)
            .iter()
            .any(|line| line.contains("Temporary Willpower is authoritative"))
    );
}

#[tokio::test]
async fn shadowrun_condition_damage_and_heal_preserve_old_semantics() {
    let store = InMemoryStore::new();
    let mut world = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ShadowrunRuleset::with_seed(9)),
    ]);
    let target = world.spawn_player("Nick");
    world.run(GOD, "@sheet/set Nick/body=6");
    world.run(GOD, "@sheet/set Nick/willpower=4");
    let _ = world.world_mut().take_dirty();
    let _ = world.take_module_ops();
    install_action_keys(&mut world);

    let perceived = PerceivedRef::KnownObject {
        id: target,
        revision: 1,
    };
    let query = world
        .engine()
        .legal_action_query(PrincipalId::Object { id: GOD }, GOD, perceived.clone())
        .unwrap();
    let damage = execute_offer(
        &mut world,
        &store,
        query,
        "shadowrun",
        "condition-damage",
        json!({ "amount": 12, "kind": "stun" }),
        0x41,
    )
    .await;
    assert_eq!(damage.outcome().data()["revision"], 1);
    assert_eq!(damage.outcome().data()["report"]["stun_filled"], 10);
    assert_eq!(damage.outcome().data()["report"]["physical_filled"], 2);
    assert_eq!(
        damage.outcome().data()["summary"],
        json!([
            "Physical  [XX.........] 2/11",
            "Stun      [XXXXXXXXXX] 10/10",
            "Wound modifier -3"
        ])
    );
    let visible = world.run(GOD, "+condition Nick");
    assert!(
        world
            .seen_by(&visible, GOD)
            .iter()
            .any(|line| line.contains("Stun      [XXXXXXXXXX] 10/10"))
    );
    let panel = query_panel(&mut world, target, "shadowrun.sheet");
    assert_eq!(
        panel["data"]["condition_stun"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|mark| !mark.is_null())
            .count(),
        10
    );
    let fenced = world.run(target, "+cast Manabolt");
    assert!(
        world
            .seen_by(&fenced, target)
            .iter()
            .any(|line| line.contains("Condition Monitor is authoritative"))
    );

    let query = world
        .engine()
        .legal_action_query(PrincipalId::Object { id: GOD }, GOD, perceived)
        .unwrap();
    let heal = execute_offer(
        &mut world,
        &store,
        query,
        "shadowrun",
        "condition-heal",
        json!({ "amount": 5, "kind": "stun" }),
        0x42,
    )
    .await;
    assert_eq!(heal.outcome().data()["revision"], 2);
    assert_eq!(heal.outcome().data()["cleared"], 5);
    assert_eq!(
        heal.outcome().data()["summary"],
        json!([
            "Physical  [XX.........] 2/11",
            "Stun      [XXXXX.....] 5/10",
            "Wound modifier -1"
        ])
    );
}
