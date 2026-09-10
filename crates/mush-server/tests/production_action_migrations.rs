use mush_core::{
    ActionStateBootRecord, Audience, AuthoritativeBundle, CanonicalId, Effects, GOD,
    IdempotencyKey, IdempotencyScope, Inbound, IngressEnvelope, IngressPayload,
    IngressSurfaceVersion, LegalActionQuery, OfferExecutionContext, OfferKeyRing, OfferSecret,
    OfferSubmission, Outbox, OwnerId, PerceivedRef, PrincipalId, ReplayWrite, RequestUid,
    ResolutionKeyRing, ResolutionSecret, ScopedIdempotencyKey, is_action_state_persistence_key,
};
use mush_jobs::JobsModule;
use mush_proto::{OutputEvent, PanelFrameKind, ProjectedActionStatus, VitalValue, VitalsFrame};
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

/// Every `Panel` frame on `feed` addressed to `who`, as `(kind, data)`.
fn panels_to(
    events: &[(Audience, OutputEvent)],
    who: mush_core::ObjId,
    feed: &str,
) -> Vec<(PanelFrameKind, Value)> {
    events
        .iter()
        .filter_map(|(aud, ev)| match (aud, ev) {
            (
                Audience::Actor(a),
                OutputEvent::Panel {
                    feed: f,
                    kind,
                    data,
                },
            ) if *a == who && f.as_str() == feed => Some((*kind, data.clone())),
            _ => None,
        })
        .collect()
}

fn any_to(events: &[(Audience, OutputEvent)], who: mush_core::ObjId) -> bool {
    events
        .iter()
        .any(|(aud, _)| matches!(aud, Audience::Actor(a) if *a == who))
}

/// [`execute_offer_announcing`] for the tests that assert on the receipt alone.
async fn execute_offer(
    world: &mut TestWorld,
    store: &InMemoryStore,
    query: LegalActionQuery,
    owner: &str,
    verb: &str,
    parameters: Value,
    byte: u8,
) -> AuthoritativeBundle {
    execute_offer_announcing(world, store, query, owner, verb, parameters, byte)
        .await
        .0
}

/// Drive one projected offer through the coordinator to its exact receipt and
/// install it the way the world task does (interface plan I13: announcing),
/// returning the bundle and everything the announcement made observers say.
async fn execute_offer_announcing(
    world: &mut TestWorld,
    store: &InMemoryStore,
    query: LegalActionQuery,
    owner: &str,
    verb: &str,
    parameters: Value,
    byte: u8,
) -> (AuthoritativeBundle, Vec<(Audience, OutputEvent)>) {
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
    let (bundle, installation) = world
        .engine_mut()
        .take_staged_action()
        .unwrap()
        .expect("production action staged")
        .into_parts();
    let bundle = *bundle;
    store
        .reserve_authoritative_intent(bundle.clone())
        .await
        .unwrap();
    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    assert!(receipt.matches_bundle(&bundle));
    let announced = world.install_prepared_action_announcing(installation);
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
    (bundle, announced)
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
    let (first, announced) = execute_offer_announcing(
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
    // Interface plan I13: the commit itself republished the actor's vitals and
    // sheet through the overlay — no poll, no client re-request — to the actor
    // alone. This is the overlay-only mutation the `AttrSet` path never saw.
    let vitals = panels_to(&announced, actor, "wod.vitals");
    assert_eq!(vitals.len(), 1, "{announced:?}");
    assert_eq!(vitals[0].0, PanelFrameKind::Keyframe);
    let frame = VitalsFrame::from_panel_data(&vitals[0].1).expect("a vitals frame");
    let temp = frame
        .vitals
        .iter()
        .find(|v| v.id == "wod.willpower_temp")
        .expect("temporary Willpower is a declared vital");
    assert!(
        matches!(temp.value, VitalValue::Bounded { current: 1, .. }),
        "the spend is live in the frame: {temp:?}"
    );
    let sheet = panels_to(&announced, actor, "wod.sheet");
    assert_eq!(sheet.len(), 1, "{announced:?}");
    assert_eq!(sheet[0].1["data"]["willpower_temp"], 1);
    assert!(!any_to(&announced, GOD), "the republish is subject-only");

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
    let (damage, announced) = execute_offer_announcing(
        &mut world,
        &store,
        query,
        "shadowrun",
        "condition-damage",
        json!({ "amount": 12, "kind": "stun" }),
        0x41,
    )
    .await;
    // Interface plan I13: a staffer's damage on another character republishes
    // that character's vitals and sheet — the case no client-side re-request
    // after one's own receipt could cover.
    let vitals = panels_to(&announced, target, "shadowrun.vitals");
    assert_eq!(vitals.len(), 1, "{announced:?}");
    let sheet = panels_to(&announced, target, "shadowrun.sheet");
    assert_eq!(sheet.len(), 1, "{announced:?}");
    assert_eq!(
        sheet[0].1["data"]["condition_stun"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|mark| !mark.is_null())
            .count(),
        10,
        "the Condition Monitor overlay rides the republished sheet"
    );
    assert!(
        !any_to(&announced, GOD),
        "the staffer who dealt the damage is not the subject"
    );
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

// ----------------------------------------------------------------------
// ---- convergence fold ----
// ----------------------------------------------------------------------

/// The typed post-images an authoritative bundle carries, in the shape the
/// server's boot partition hands `Engine::hydrate_action_state` — the same
/// `is_action_state_persistence_key` split `mush_server::modules`'
/// `load_one_namespace` performs against a live store.
fn typed_boot_records(bundle: &AuthoritativeBundle) -> Vec<ActionStateBootRecord> {
    bundle
        .writes()
        .iter()
        .filter_map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if is_action_state_persistence_key(key.as_str()) => Some(ActionStateBootRecord::new(
                namespace.as_str(),
                key.as_str(),
                value.clone(),
            )),
            _ => None,
        })
        .collect()
}

/// 08j S7 — **the K8.4 non-clobber round trip for `WodRollState`.**
///
/// 09b §3 permits unrelated legacy fields beside a typed family "only when the
/// ownership split is explicit **and** round-trip tests prove neither writer
/// clobbers the other". The split is explicit (`sheet_set` fences exactly
/// `willpower_temp`; `WodCmd::read_ctx` re-overlays on every read) and the
/// implementation is non-clobbering both ways, but no test issued a legacy
/// write to an *unrelated* field after the typed record existed and then
/// re-asserted the typed value, and none carried the typed record across a
/// reboot.
#[tokio::test]
async fn cv_7_wod_a_legacy_sheet_write_beside_the_typed_roll_state_survives_a_reboot() {
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
    world.run(GOD, "@sheet/set Nick/concept=Jaded Detective");
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
    let bundle = execute_offer(
        &mut world,
        &store,
        query,
        "wod",
        "willpower-roll",
        json!({ "pool": "strength+brawl", "difficulty": 6 }),
        0x51,
    )
    .await;
    assert_eq!(bundle.outcome().data()["willpower_remaining"], 1);
    assert_eq!(
        query_panel(&mut world, actor, "wod.sheet")["data"]["willpower_temp"],
        1
    );

    // (a) A legacy write to an **unrelated** field lands and clobbers nothing.
    world.run(GOD, "@sheet/set Nick/concept=Night Courier");
    let panel = query_panel(&mut world, actor, "wod.sheet");
    assert_eq!(panel["data"]["concept"], "Night Courier");
    assert_eq!(
        panel["data"]["willpower_temp"], 1,
        "an unrelated legacy write must not disturb the typed record"
    );

    // (b) A legacy write **at** the migrated field is refused, and the refusal
    //     leaves the typed record exactly where it was.
    let clobber = world.run(GOD, "@sheet/set Nick/willpower_temp=7");
    assert!(
        world
            .seen_by(&clobber, GOD)
            .iter()
            .any(|line| line.contains("Temporary Willpower is authoritative"))
    );
    assert_eq!(
        query_panel(&mut world, actor, "wod.sheet")["data"]["willpower_temp"],
        1,
        "a refused legacy write must not half-apply"
    );

    // (c) The record survives a reboot: the bundle's typed post-image hydrates
    //     into a fresh composition and still owns the field, over a legacy
    //     sheet that came back from the store with its own stale value.
    let records = typed_boot_records(&bundle);
    assert!(
        !records.is_empty(),
        "the migrated record must be in the authoritative bundle"
    );
    let mut rebooted = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(7)),
    ]);
    let reborn = rebooted.spawn_player("Nick");
    assert_eq!(reborn, actor, "the reboot restores the same object id");
    rebooted.run(GOD, "@sheet/set Nick/willpower=2");
    rebooted.run(GOD, "@sheet/set Nick/willpower_temp=9");
    rebooted.run(GOD, "@sheet/set Nick/concept=Night Courier");
    rebooted
        .engine_mut()
        .hydrate_action_state(records)
        .expect("the typed record hydrates into a fresh composition");

    let panel = query_panel(&mut rebooted, reborn, "wod.sheet");
    assert_eq!(
        panel["data"]["willpower_temp"], 1,
        "the typed record still owns temporary Willpower after the reboot"
    );
    assert_eq!(
        panel["data"]["concept"], "Night Courier",
        "the legacy half of the sheet survived beside it"
    );
    let refused = rebooted.run(GOD, "@sheet/set Nick/willpower_temp=7");
    assert!(
        rebooted
            .seen_by(&refused, GOD)
            .iter()
            .any(|line| line.contains("Temporary Willpower is authoritative")),
        "the fence is live again after the reboot"
    );
}

/// 08j S7 — **the K8.4 non-clobber round trip for `ConditionState`.**
///
/// The Shadowrun half of the same duty. `sheet_set` fences exactly
/// `physical_overflow` (a string asserted by no test before this one) and
/// `ConditionState::apply_to_sheet` re-overlays the monitors on every read.
#[tokio::test]
async fn cv_7_shadowrun_a_legacy_sheet_write_beside_the_typed_condition_state_survives_a_reboot() {
    let store = InMemoryStore::new();
    let mut world = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ShadowrunRuleset::with_seed(9)),
    ]);
    let target = world.spawn_player("Nick");
    world.run(GOD, "@sheet/set Nick/body=6");
    world.run(GOD, "@sheet/set Nick/willpower=4");
    world.run(GOD, "@sheet/set Nick/concept=Street Samurai");
    let _ = world.world_mut().take_dirty();
    let _ = world.take_module_ops();
    install_action_keys(&mut world);

    let perceived = PerceivedRef::KnownObject {
        id: target,
        revision: 1,
    };
    let query = world
        .engine()
        .legal_action_query(PrincipalId::Object { id: GOD }, GOD, perceived)
        .unwrap();
    let bundle = execute_offer(
        &mut world,
        &store,
        query,
        "shadowrun",
        "condition-damage",
        json!({ "amount": 12, "kind": "stun" }),
        0x61,
    )
    .await;
    assert_eq!(bundle.outcome().data()["report"]["stun_filled"], 10);

    let filled = |panel: &Value, track: &str| -> usize {
        panel["data"][track]
            .as_array()
            .expect("a condition track")
            .iter()
            .filter(|mark| !mark.is_null())
            .count()
    };

    // (a) A legacy write to an **unrelated** field lands and clobbers nothing.
    world.run(GOD, "@sheet/set Nick/concept=Rigger");
    let panel = query_panel(&mut world, target, "shadowrun.sheet");
    assert_eq!(panel["data"]["concept"], "Rigger");
    assert_eq!(
        filled(&panel, "condition_stun"),
        10,
        "an unrelated legacy write must not disturb the typed monitors"
    );

    // (b) A legacy write **at** the migrated field is refused.
    let clobber = world.run(GOD, "@sheet/set Nick/physical_overflow=5");
    assert!(
        world
            .seen_by(&clobber, GOD)
            .iter()
            .any(|line| line.contains("Condition Monitor is authoritative")),
        "the `physical_overflow` fence refuses the clobber"
    );
    assert_eq!(
        filled(
            &query_panel(&mut world, target, "shadowrun.sheet"),
            "condition_stun"
        ),
        10,
        "a refused legacy write must not half-apply"
    );

    // (c) The record survives a reboot.
    let records = typed_boot_records(&bundle);
    assert!(
        !records.is_empty(),
        "the migrated record must be in the authoritative bundle"
    );
    let mut rebooted = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ShadowrunRuleset::with_seed(9)),
    ]);
    let reborn = rebooted.spawn_player("Nick");
    assert_eq!(reborn, target, "the reboot restores the same object id");
    rebooted.run(GOD, "@sheet/set Nick/body=6");
    rebooted.run(GOD, "@sheet/set Nick/willpower=4");
    rebooted.run(GOD, "@sheet/set Nick/concept=Rigger");
    rebooted
        .engine_mut()
        .hydrate_action_state(records)
        .expect("the typed record hydrates into a fresh composition");

    let panel = query_panel(&mut rebooted, reborn, "shadowrun.sheet");
    assert_eq!(
        filled(&panel, "condition_stun"),
        10,
        "the typed record still owns the monitors after the reboot"
    );
    assert_eq!(
        panel["data"]["concept"], "Rigger",
        "the legacy half of the sheet survived beside it"
    );
    let refused = rebooted.run(GOD, "@sheet/set Nick/physical_overflow=5");
    assert!(
        rebooted
            .seen_by(&refused, GOD)
            .iter()
            .any(|line| line.contains("Condition Monitor is authoritative")),
        "the fence is live again after the reboot"
    );
}
