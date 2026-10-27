//! The production authoritative-action lane, driven end to end through the
//! server's own [`ActionCoordinator`]: project an offer, preflight it, prepare
//! and stage it, reconcile and checkpoint the bundle, install the prepared
//! action announcing (interface plan I13), and project the committed receipt.
//!
//! 16b cut R-b: the two game rulesets these migrations were written against are
//! gone, so the fixture is the `toy` test ruleset's authoritative pair
//! (`toy-willpower-roll` / `toy-willpower-set`). Every platform property below
//! is the one the deleted rulesets carried: one resource move per accepted
//! offer, a replayable resolution record, the I13 republish to the SUBJECT
//! alone (including when the subject is not the actor), and a typed post-image
//! that rehydrates into a fresh composition beside an untouched legacy sheet.

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
use mush_testkit::toy_ruleset::ToyRuleset;
use serde_json::{Value, json};

/// The toy module id — the idempotency owner every offer below is scoped to.
const OWNER: &str = "toy";

fn install_action_keys(world: &mut TestWorld) {
    world.engine_mut().set_offer_keys(
        OfferKeyRing::new(OfferSecret::new(1, [0x41; 32]).expect("offer secret"), [])
            .expect("offer ring"),
    );
    world.engine_mut().set_resolution_keys(
        ResolutionKeyRing::new(
            ResolutionSecret::new(1, [0x52; 32]).expect("resolution secret"),
            [],
        )
        .expect("resolution ring"),
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
            owner: OwnerId::new(owner).expect("a well-formed owner id"),
            namespace: CanonicalId::new(namespace.replace('-', "_"))
                .expect("a well-formed namespace"),
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

/// The `toy.willpower_current` gauge's current value off a vitals panel.
fn pool_current(panel: &Value) -> Option<i64> {
    let frame = VitalsFrame::from_panel_data(panel).ok()?;
    frame
        .vitals
        .iter()
        .find(|v| v.id == "toy.willpower_current")
        .and_then(|v| match v.value {
            VitalValue::Bounded { current, .. } => Some(current),
            _ => None,
        })
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
        .expect("a projection")
        .projected()
        .into_iter()
        .find(|offer| offer.presentation().verb == verb)
        .unwrap_or_else(|| panic!("missing projected {verb} offer"));
    let (request, principal, idempotency) = request_identity(owner, verb, query.actor(), byte);
    let envelope = IngressEnvelope::new(
        request,
        principal.clone(),
        IngressSurfaceVersion::new(
            CanonicalId::new("test.production_action").expect("a static surface id"),
            1,
        )
        .expect("a static surface version"),
        IngressPayload::structured(json!({
            "offer": offer.offer(),
            "subject": offer.subject(),
            "targets": [],
            "parameters": parameters,
        }))
        .expect("a structured payload"),
        idempotency.clone(),
    )
    .expect("an ingress envelope");
    let status = ActionCoordinator::status_id(&idempotency).expect("a status id");
    let permit = match ActionCoordinator::preflight(store, envelope)
        .await
        .expect("preflight")
    {
        ActionPreflight::Vacant(permit) => permit,
        other => panic!("production action was not vacant: {other:?}"),
    };
    let resolved = world
        .engine()
        .resolve_legal_action(
            &world
                .engine()
                .project_legal_actions(&query)
                .expect("a projection"),
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
        .expect("the offer resolves");
    ActionCoordinator::prepare_and_stage_resolved(permit, world.engine_mut(), resolved)
        .expect("the provider prepares and stages");
    let (bundle, installation) = world
        .engine_mut()
        .take_staged_action()
        .expect("a staged action slot")
        .expect("production action staged")
        .into_parts();
    let bundle = *bundle;
    store
        .reserve_authoritative_intent(bundle.clone())
        .await
        .expect("the intent reserves");
    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .expect("the bundle reconciles");
    assert!(receipt.matches_bundle(&bundle));
    let announced = world.install_composed_action_announcing(installation);
    store
        .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .expect("the bundle checkpoints");
    let projections = world
        .engine()
        .registry
        .action_providers()
        .expect("an action provider table")
        .receipt_projection_table();
    let ProjectedActionStatus::Committed(projected) = ActionCoordinator::projected_status(
        store,
        &projections,
        &PrincipalId::Object { id: query.actor() },
        &status,
    )
    .await
    .expect("a projected status") else {
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

/// Seed a character with a full, valid toy sheet: a five-die pool and a
/// two-point Willpower reserve.
fn seed_roller(world: &mut TestWorld, name: &str) {
    world.run(GOD, &format!("@sheet/set {name}/strength=3"));
    world.run(GOD, &format!("@sheet/set {name}/brawl=2"));
    world.run(GOD, &format!("@sheet/set {name}/willpower=2"));
    // NOTE: the pool is clamped by the permanent rating, so seeding it above 2
    // would simply land on 2 — the seed is the honest value.
    world.run(GOD, &format!("@sheet/set {name}/willpower_current=2"));
}

fn self_query(world: &TestWorld, actor: mush_core::ObjId) -> LegalActionQuery {
    world
        .engine()
        .legal_action_query(
            PrincipalId::Object { id: actor },
            actor,
            PerceivedRef::SelfActor,
        )
        .expect("a legal-action query")
}

fn staff_query(world: &TestWorld, target: mush_core::ObjId) -> LegalActionQuery {
    world
        .engine()
        .legal_action_query(
            PrincipalId::Object { id: GOD },
            GOD,
            PerceivedRef::KnownObject {
                id: target,
                revision: 1,
            },
        )
        .expect("a legal-action query")
}

#[tokio::test]
async fn toy_willpower_roll_spends_once_and_records_resolution() {
    let store = InMemoryStore::new();
    let mut world = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ToyRuleset::with_seed(7)),
    ]);
    let actor = world.spawn_player("Nick");
    seed_roller(&mut world, "Nick");
    let _ = world.world_mut().take_dirty();
    let _ = world.take_module_ops();
    install_action_keys(&mut world);

    let query = self_query(&world, actor);
    let (first, announced) = execute_offer_announcing(
        &mut world,
        &store,
        query,
        OWNER,
        "toy-willpower-roll",
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
        query_panel(&mut world, actor, "toy.sheet_live")["data"]["willpower_current"],
        1
    );
    // Interface plan I13: the commit itself republished the actor's vitals and
    // sheet through the overlay — no poll, no client re-request — to the actor
    // alone. This is the overlay-only mutation the `AttrSet` path never saw.
    let vitals = panels_to(&announced, actor, "toy.vitals");
    assert_eq!(vitals.len(), 1, "{announced:?}");
    assert_eq!(vitals[0].0, PanelFrameKind::Keyframe);
    assert_eq!(
        pool_current(&vitals[0].1),
        Some(1),
        "the spend is live in the frame: {:?}",
        vitals[0].1
    );
    let sheet = panels_to(&announced, actor, "toy.sheet");
    assert_eq!(sheet.len(), 1, "{announced:?}");
    assert_eq!(sheet[0].1["data"]["willpower_current"], 1);
    assert!(!any_to(&announced, GOD), "the republish is subject-only");

    let query = self_query(&world, actor);
    let second = execute_offer(
        &mut world,
        &store,
        query,
        OWNER,
        "toy-willpower-roll",
        json!({ "pool": "strength+brawl", "difficulty": 6 }),
        0x32,
    )
    .await;
    assert_eq!(second.outcome().data()["willpower_remaining"], 0);
    assert_eq!(second.outcome().data()["roll_sequence"], 2);

    let query = self_query(&world, actor);
    assert!(
        world
            .engine()
            .project_legal_actions(&query)
            .expect("a projection")
            .projected()
            .into_iter()
            .all(|offer| offer.presentation().verb != "toy-willpower-roll")
    );

    let query = staff_query(&world, actor);
    let restored = execute_offer(
        &mut world,
        &store,
        query,
        OWNER,
        "toy-willpower-set",
        json!({ "current": 2 }),
        0x33,
    )
    .await;
    assert_eq!(restored.outcome().data()["current"], 2);
    assert_eq!(restored.outcome().data()["roll_sequence"], 2);
    assert_eq!(
        query_panel(&mut world, actor, "toy.sheet_live")["data"]["willpower_current"],
        2
    );
}

/// Interface plan I13, the half no receipt of one's own could cover: a
/// **staffer mutating ANOTHER character** republishes that character's vitals
/// and sheet — to the subject, not to the actor who performed the mutation.
///
/// This is the platform property the deleted Shadowrun condition-monitor pair
/// carried (`shadowrun_condition_damage_and_heal_preserve_old_semantics`); the
/// toy's `toy-willpower-set` is the same shape — a `KnownObject` subject whose
/// typed record keys on the TARGET, not on the actor.
#[tokio::test]
async fn a_staff_mutation_of_another_character_republishes_that_character() {
    let store = InMemoryStore::new();
    let mut world = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ToyRuleset::with_seed(9)),
    ]);
    let target = world.spawn_player("Nick");
    world.run(GOD, "@sheet/set Nick/willpower=4");
    world.run(GOD, "@sheet/set Nick/willpower_current=4");
    let _ = world.world_mut().take_dirty();
    let _ = world.take_module_ops();
    install_action_keys(&mut world);

    let query = staff_query(&world, target);
    let (set, announced) = execute_offer_announcing(
        &mut world,
        &store,
        query,
        OWNER,
        "toy-willpower-set",
        json!({ "current": 1 }),
        0x41,
    )
    .await;
    assert_eq!(set.outcome().data()["actor"], GOD.0);
    assert_eq!(set.outcome().data()["target"], target.0);
    assert_eq!(set.outcome().data()["current"], 1);

    // The republish followed the typed record's SUBJECT, not the actor.
    let vitals = panels_to(&announced, target, "toy.vitals");
    assert_eq!(vitals.len(), 1, "{announced:?}");
    assert_eq!(
        pool_current(&vitals[0].1),
        Some(1),
        "the staffer's write is live in the subject's frame"
    );
    let sheet = panels_to(&announced, target, "toy.sheet");
    assert_eq!(sheet.len(), 1, "{announced:?}");
    assert_eq!(
        sheet[0].1["data"]["willpower_current"], 1,
        "the authoritative overlay rides the republished sheet"
    );
    assert!(
        !any_to(&announced, GOD),
        "the staffer who set the pool is not the subject"
    );

    // And the subject's own live queries agree, while the legacy stored sheet
    // is untouched: the typed record, not the document, owns the field.
    assert_eq!(
        pool_current(&query_panel(&mut world, target, "toy.vitals")),
        Some(1)
    );
    assert_eq!(
        query_panel(&mut world, target, "toy.sheet_live")["data"]["willpower_current"],
        1
    );
    assert_eq!(
        query_panel(&mut world, target, "toy.sheet")["data"]["willpower_current"],
        4,
        "the generic (stored-only) sheet query has no typed-state lens"
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

/// 08j S7 — **the K8.4 non-clobber round trip for the typed roll state.**
///
/// 09b §3 permits unrelated legacy fields beside a typed family "only when the
/// ownership split is explicit **and** round-trip tests prove neither writer
/// clobbers the other". The split is explicit (the overlay re-applies on every
/// read through the typed-state lens) and the implementation is non-clobbering
/// both ways, but no test issued a legacy write to an *unrelated* field after
/// the typed record existed and then re-asserted the typed value, and none
/// carried the typed record across a reboot.
#[tokio::test]
async fn cv_7_toy_a_legacy_sheet_write_beside_the_typed_roll_state_survives_a_reboot() {
    let store = InMemoryStore::new();
    let mut world = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ToyRuleset::with_seed(7)),
    ]);
    let actor = world.spawn_player("Nick");
    seed_roller(&mut world, "Nick");
    world.run(GOD, "@sheet/set Nick/concept=Jaded Detective");
    let _ = world.world_mut().take_dirty();
    let _ = world.take_module_ops();
    install_action_keys(&mut world);

    let query = self_query(&world, actor);
    let bundle = execute_offer(
        &mut world,
        &store,
        query,
        OWNER,
        "toy-willpower-roll",
        json!({ "pool": "strength+brawl", "difficulty": 6 }),
        0x51,
    )
    .await;
    assert_eq!(bundle.outcome().data()["willpower_remaining"], 1);
    assert_eq!(
        query_panel(&mut world, actor, "toy.sheet_live")["data"]["willpower_current"],
        1
    );

    // (a) A legacy write to an **unrelated** field lands and clobbers nothing.
    world.run(GOD, "@sheet/set Nick/concept=Night Courier");
    let panel = query_panel(&mut world, actor, "toy.sheet_live");
    assert_eq!(panel["data"]["concept"], "Night Courier");
    assert_eq!(
        panel["data"]["willpower_current"], 1,
        "an unrelated legacy write must not disturb the typed record"
    );

    // (b) A legacy write **at** the migrated field lands in the document and is
    //     overruled on every overlay read: the typed record owns the field.
    //     (The toy carries no `sheet_set` write fence — the deleted rulesets'
    //     "… is authoritative" refusal has no toy equivalent and its assertions
    //     are gone; what remains is the non-clobber half, which is the property
    //     09b §3 actually asks for.)
    world.run(GOD, "@sheet/set Nick/willpower_current=2");
    assert_eq!(
        query_panel(&mut world, actor, "toy.sheet")["data"]["willpower_current"],
        2,
        "the legacy write did land on the stored document"
    );
    assert_eq!(
        query_panel(&mut world, actor, "toy.sheet_live")["data"]["willpower_current"],
        1,
        "the typed record still wins on every overlay read"
    );
    assert_eq!(
        pool_current(&query_panel(&mut world, actor, "toy.vitals")),
        Some(1),
        "and on the vitals projection"
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
        Box::new(ToyRuleset::with_seed(7)),
    ]);
    let reborn = rebooted.spawn_player("Nick");
    assert_eq!(reborn, actor, "the reboot restores the same object id");
    rebooted.run(GOD, "@sheet/set Nick/willpower=2");
    rebooted.run(GOD, "@sheet/set Nick/willpower_current=2");
    rebooted.run(GOD, "@sheet/set Nick/concept=Night Courier");
    rebooted
        .engine_mut()
        .hydrate_action_state(records)
        .expect("the typed record hydrates into a fresh composition");

    let panel = query_panel(&mut rebooted, reborn, "toy.sheet_live");
    assert_eq!(
        panel["data"]["willpower_current"], 1,
        "the typed record still owns the Willpower pool after the reboot"
    );
    assert_eq!(
        panel["data"]["concept"], "Night Courier",
        "the legacy half of the sheet survived beside it"
    );
    assert_eq!(
        pool_current(&query_panel(&mut rebooted, reborn, "toy.vitals")),
        Some(1),
        "the overlay is live again after the reboot"
    );
}
