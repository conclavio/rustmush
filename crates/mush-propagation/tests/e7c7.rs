//! E7 cut 7 acceptance — one propagation context, two resolvers, false-contact
//! phantoms, delivery envelopes and the tactics-owned sensor ingress (10b §3.7
//! as amended by 08i answer 1) — driven through the real `Engine` via
//! `TestWorld`.
//!
//! Three verify lenses shape the rhythms:
//! - **K (knowledge leak)**: can any viewer, render, error or timing difference
//!   reveal an undetected unit or another observer's picture?
//! - **O (order lifecycle)**: can a stale order become active, a superseded one
//!   execute, or an ack be inferred without arriving?
//! - **B (bounds and restart)**: can contacts or envelopes grow without limit,
//!   or change state across a crash?
//!
//! **Every hazard-axis test asserts the OBSERVABLE outcome** — what a second
//! actor sees, the refusal bytes a non-holder receives, the rendered listing —
//! not merely the document the code just wrote (2026-09-06 addendum).

use mush_core::{
    Audience, BehaviorBinding, BehaviorId, ClockRef, ClockStamp, ContactId, ContactLedgerRow,
    ContactLedgerState, GOD, IdempotencyKeyText, LIMBO, ModuleBootData, ModuleWrite, ObjId,
    ObjKind, PrincipalId, SubjectKey,
};
use mush_propagation::{
    Envelope, EnvelopeState, MAX_ENVELOPE_HOPS, MAX_ENVELOPE_PAYLOAD_BYTES, MAX_ENVELOPES_PER_PASS,
    NO_SUCH_CONTACT, NO_SUCH_ENVELOPE, PropagationModule, PropagationSensorPolicy,
    PropagationStore, active_order, delivery_pass, detection_pass,
};
use mush_proto::OutputEvent;
use mush_sim::{SimulationModule, SpatialState};
use mush_tactics::TacticsModule;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;
use std::collections::HashMap;
use std::sync::Arc;

// ---- harness -----------------------------------------------------------

/// The per-arm bound `@prop/pass` itself passes the detection pass. Named once
/// so a call site reads as "one arm's worth of work" (fold D4).
const ARM: usize = 16;

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(SimulationModule::new()),
        Box::new(TacticsModule::new().with_sensor_policy(Arc::new(PropagationSensorPolicy))),
        Box::new(PropagationModule::new()),
    ])
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}

fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let events = tw.run(actor, line);
    let t = texts(&events);
    assert!(!has_error(&events), "{line:?} refused: {t}");
    t
}

/// Single-step the coarse pass.
///
/// The production world task drains the queued arming submission through
/// `CommandSubmitDriver`; a `TestWorld` has no such loop, so the rig drains it —
/// otherwise a second `@prop/pass` is refused with "your earlier action is still
/// committing", which is the engine's own one-submission-per-actor rule
/// (E7c2 fold C4) doing exactly its job.
fn pass(tw: &mut TestWorld) -> String {
    let seen = ok(tw, GOD, "@prop/pass");
    while tw.engine_mut().take_authoritative_submit().is_some() {}
    seen
}

fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let events = tw.run(actor, line);
    assert!(
        has_error(&events),
        "{line:?} should have been refused: {}",
        texts(&events)
    );
    texts(&events)
}

fn bind(tw: &mut TestWorld, obj: ObjId, behavior: &str) {
    tw.world_mut()
        .attach_behavior(
            obj,
            BehaviorId::new(behavior).unwrap(),
            BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
}

fn stamp(tw: &TestWorld) -> ClockStamp {
    tw.world().world_ic_now()
}

fn state(tw: &TestWorld) -> SpatialState {
    tw.world().module_state::<SpatialState>().cloned().unwrap()
}

fn book(tw: &TestWorld) -> PropagationStore {
    tw.world()
        .module_state::<PropagationStore>()
        .cloned()
        .unwrap()
}

fn set_book(tw: &mut TestWorld, store: PropagationStore) {
    *tw.world_mut()
        .module_state_mut::<PropagationStore>()
        .unwrap() = store;
}

fn fold_kv(tw: &mut TestWorld) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        if op.ns != "propagation" {
            continue;
        }
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}

/// Boot a fresh world from a folded document set — the restart half.
fn reboot(kv: Vec<(SmolStr, serde_json::Value)>) -> TestWorld {
    let mut tw = boot();
    let mut boot_data: HashMap<SmolStr, ModuleBootData> = HashMap::new();
    boot_data.insert(
        SmolStr::new_static("propagation"),
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    );
    tw.modules_loaded(boot_data);
    tw
}

/// Install a known leak-safe ledger projection.
///
/// The projection is published by `mush-tactics` from a bounded scan of the
/// contact family, which no `TestWorld` command path can reach; installing a
/// known one is how the K-lens rendering tests assert what a viewer actually
/// sees rather than what a document says.
fn install_ledger(tw: &mut TestWorld, rows: Vec<ContactLedgerRow>) {
    let at = stamp(tw);
    tw.world_mut()
        .module_state_mut::<ContactLedgerState>()
        .expect("the tactics ledger slot is installed at registration")
        .install(rows, at);
}

/// One ledger row **on the world's own IC clock generation**, because an
/// incomparable stamp renders as age 0 and would make every staleness assertion
/// vacuous.
fn row_at(
    clock: &ClockRef,
    holder: ObjId,
    contact: [u8; 16],
    active: bool,
    observed_at: i64,
    phantom: Option<&str>,
) -> ContactLedgerRow {
    ContactLedgerRow {
        holder: PrincipalId::Object { id: holder },
        contact: ContactId::from_bytes(contact),
        generation: 1,
        revision: 1,
        active,
        observed_at: ClockStamp {
            clock: clock.clone(),
            at: observed_at,
        },
        phantom: phantom.map(|id| IdempotencyKeyText::parse_canonical(id).unwrap()),
    }
}

// ---- the fixture -------------------------------------------------------

/// Two crewed platforms and a third object, all standing in one live domain,
/// with a second (interior) place for the occlusion cases.
struct Fx {
    orbit: ObjId,
    hull: ObjId,
    alpha: ObjId,
    bravo: ObjId,
    target: ObjId,
    ann: ObjId,
    bob: ObjId,
}

fn build(tw: &mut TestWorld) -> Fx {
    let mk_room = |tw: &mut TestWorld, name: &str| {
        tw.world_mut()
            .create(ObjKind::Room, name, GOD, None)
            .unwrap()
    };
    let mk_thing = |tw: &mut TestWorld, name: &str| {
        tw.world_mut()
            .create(ObjKind::Thing, name, GOD, Some(LIMBO))
            .unwrap()
    };
    let orbit = mk_room(tw, "Orbit");
    let hull = mk_room(tw, "Hull");
    let alpha = mk_thing(tw, "Alpha");
    let bravo = mk_thing(tw, "Bravo");
    let target = mk_thing(tw, "Target");
    let ann = tw
        .world_mut()
        .create(ObjKind::Player, "Ann", GOD, Some(LIMBO))
        .unwrap();
    let bob = tw
        .world_mut()
        .create(ObjKind::Player, "Bob", GOD, Some(LIMBO))
        .unwrap();

    bind(tw, orbit, "sim.domain");
    // The two crew stand in the domain too, so every end of a delivery has a
    // place a context can be built between and a declared jamming level applies
    // to them exactly as it does to a ship.
    for platform in [alpha, bravo, target, ann, bob] {
        bind(tw, platform, "sim.participant");
    }
    bind(tw, hull, "sim.interior");
    ok(tw, GOD, &format!("@domain/create #{}=sim.passive", orbit.0));
    for platform in [alpha, bravo, target, ann, bob] {
        ok(tw, GOD, &format!("@spatial/place/domain #{}=0", platform.0));
    }
    ok(
        tw,
        GOD,
        &format!("@spatial/space #{}=moving:#{}", hull.0, alpha.0),
    );
    for (platform, crew, station) in [(alpha, ann, "sensors"), (bravo, bob, "comms")] {
        ok(
            tw,
            GOD,
            &format!(
                "@spatial/station #{}={station},{station},channels:movement",
                platform.0
            ),
        );
        ok(
            tw,
            GOD,
            &format!("@spatial/assign #{}={station},#{}", platform.0, crew.0),
        );
    }
    Fx {
        orbit,
        hull,
        alpha,
        bravo,
        target,
        ann,
        bob,
    }
}

fn scene() -> (TestWorld, Fx) {
    let mut tw = boot();
    tw.set_now(1_000);
    let fx = build(&mut tw);
    let _ = tw.take_module_ops();
    (tw, fx)
}

/// Issue one order and run the pass until it settles.
fn deliver(tw: &mut TestWorld, from: ObjId, to: ObjId, text: &str) -> u64 {
    ok(tw, from, &format!("+order #{}={text}", to.0));
    let id = book(tw)
        .envelopes()
        .filter(|envelope| envelope.issuer() == from && envelope.recipient() == to)
        .map(Envelope::id)
        .max()
        .expect("the order was queued");
    pass(tw);
    id
}

// ======================================================================
// A — the DECIDED probe: one contact authority, and only one
// ======================================================================

/// **The binding constraint, probed on the composed engine.** `mush-tactics` is
/// the sole `ContactState` authority: it is the only owner with a registered
/// `AuthorityGuardKind::Contact` validator, and every other E7 crate — the one
/// this cut ADDS included — has none. The kernel's registry is keyed
/// `(OwnerId, kind)`, so this is the authority question asked directly.
#[test]
fn e7c7_a1_only_mush_tactics_has_a_contact_guard_validator_not_propagation_sim_or_industry() {
    let (tw, _fx) = scene();
    let validators = tw
        .registry()
        .authority_guard_validators()
        .expect("the composition froze the guard registry");
    let guard_for = |owner: &str| mush_core::AuthorityGuard::Contact {
        provider: mush_core::OwnerId::new(owner).unwrap(),
        principal: PrincipalId::Object { id: ObjId(1) },
        id: ContactId::from_bytes([1; 16]),
        generation: 1,
    };
    assert!(
        validators.get_for_guard(&guard_for("tactics")).is_ok(),
        "mush-tactics registers the one contact validator"
    );
    // EVERY arm that could hold a second authority, enumerated.
    for owner in ["propagation", "sim", "industry", "inventory", "economy"] {
        assert!(
            validators.get_for_guard(&guard_for(owner)).is_err(),
            "{owner} must not register a second contact authority"
        );
    }
    // And the family half: `mush-propagation` registers **no typed state family
    // at all**, so there is not even a handle with which it could name the
    // contact family — `ActionTxn::checked_key`'s cross-owner refusal never has
    // to fire. Proved by composing the same world with and without it.
    let without = TestWorld::with_modules(vec![
        Box::new(SimulationModule::new()),
        Box::new(TacticsModule::new().with_sensor_policy(Arc::new(PropagationSensorPolicy))),
    ]);
    let families = |world: &TestWorld| {
        world
            .registry()
            .state_families()
            .expect("the composition froze the family registry")
            .len()
    };
    assert_eq!(
        families(&tw),
        families(&without),
        "mush-propagation registered a typed state family"
    );
}

/// **Propagation writes no `ContactState`, and the observable proof is what
/// `@prop/pass` actually does**: it queues exactly ONE authoritative submission,
/// naming the `mush-tactics`-owned `sensor_arm` action, and every document it
/// persists is its own. Nothing in its op stream touches the contact family.
#[test]
fn e7c7_a2_prop_pass_stages_the_tactics_owned_sensor_arm_and_writes_no_contact_document() {
    let (mut tw, _fx) = scene();
    // Deliberately NOT through `pass()`: this test inspects the submission the
    // rig's helper drains.
    ok(&mut tw, GOD, "@prop/pass");
    assert!(
        tw.engine().has_authoritative_submit(),
        "the pass armed the tactics ingress"
    );
    let submit = tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("exactly one submission");
    assert_eq!(submit.action.key().owner.as_str(), "tactics");
    assert_eq!(submit.action.key().id.as_str(), "sensor_arm");
    assert!(
        !tw.engine().has_authoritative_submit(),
        "one submission per command; a second would be a refusal by design"
    );
    for op in tw.take_module_ops() {
        let key = match &op.op {
            ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key } => key.to_string(),
            ModuleWrite::StreamAppend { .. } => continue,
        };
        assert!(
            op.ns == "propagation" && !key.contains("contact"),
            "propagation wrote {}/{key}",
            op.ns
        );
    }
}

/// A detection pass over one context produces the **same ordered intent set
/// twice** and never more than `MAX_OBSERVATIONS_PER_PASS`.
#[test]
fn e7c7_a3_a_detection_pass_is_deterministic_ordered_and_bounded() {
    let (tw, _fx) = scene();
    let state = state(&tw);
    let ledger = ContactLedgerState::default();
    let at = stamp(&tw);
    let mut left = book(&tw);
    let mut right = book(&tw);
    let first = detection_pass(&mut left, &state, tw.world(), &ledger, &at, None, ARM).intents;
    let second = detection_pass(&mut right, &state, tw.world(), &ledger, &at, None, ARM).intents;
    assert!(!first.is_empty(), "two crewed platforms see each other");
    assert_eq!(
        first.iter().map(|i| i.key().as_str()).collect::<Vec<_>>(),
        second.iter().map(|i| i.key().as_str()).collect::<Vec<_>>(),
        "same context, same set, same order"
    );
    assert!(first.len() <= mush_core::MAX_OBSERVATIONS_PER_PASS);
    // And running it again over the SAME book arms nothing twice: the sighting
    // book is the idempotency key.
    let again = detection_pass(&mut left, &state, tw.world(), &ledger, &at, None, ARM).intents;
    assert!(again.is_empty(), "a second pass re-arms nothing");
}

/// The production composition freezes **eight** schedule sources: cut 5's
/// `control_handoff`, cut 4's `production`, and cut 7's six sensor sources —
/// one per contact kind, all under `mush-tactics`' owner, all disjoint ids.
#[test]
fn e7c7_a4_the_six_sensor_sources_are_registered_beside_handoff_and_production() {
    let (tw, _fx) = scene();
    let sources = tw
        .registry()
        .schedule_sources()
        .expect("the composition froze the source registry");
    // This world carries `mush-sim`'s `control_handoff` and E7 cut 8's
    // `domaintick`, plus the six sensor sources; `production` is
    // `mush-industry`'s and is pinned in `mush-server`.
    //
    // **The count moved 7 → 8 with E7 cut 8**, which registers exactly one new
    // source (`domaintick`, `mush-sim`) — the pin is what makes that visible
    // rather than silent, so it is raised deliberately here.
    assert_eq!(
        sources.len(),
        8,
        "control_handoff + domaintick + six sensor sources"
    );
    let mut ids: Vec<&str> = mush_core::ObservationKind::all()
        .iter()
        .map(|kind| mush_tactics::sensor_source_id(*kind))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 6);
    for id in ids {
        assert!(!matches!(
            id,
            "control_handoff" | "production" | "npc" | "domaintick"
        ));
    }
}

// ======================================================================
// K — hazard axis 1: a phantom laundered into a real identity
// ======================================================================

/// A phantom's `SubjectKey::Scoped` **never resolves to a unit** through any
/// projection, and cannot become an `Object` subject. The observable half: the
/// minted subject, every serialization of the store, and the rendered contact
/// line all carry no object id at all.
#[test]
fn e7c7_k1_a_phantom_subject_never_resolves_to_a_unit_through_any_projection() {
    let (mut tw, fx) = scene();
    ok(
        &mut tw,
        GOD,
        &format!("@prop/phantom #{}=decoy,300", fx.alpha.0),
    );
    let store = book(&tw);
    let phantom = store.phantoms().next().expect("a phantom was minted");
    let subject = phantom.subject();
    assert!(
        matches!(subject, SubjectKey::Scoped { .. }),
        "never an Object subject"
    );
    let SubjectKey::Scoped { owner, class, .. } = &subject else {
        unreachable!()
    };
    assert_eq!(owner.as_str(), "propagation");
    assert_eq!(class.as_str(), mush_propagation::PHANTOM_CLASS);
    // Recursive scan: no `SubjectKey::Object` anywhere in the persisted form.
    let text = serde_json::to_string(&subject).unwrap();
    assert!(!text.contains("\"object\""), "{text}");
    // And the OBSERVABLE half: a holder viewing the phantom-backed contact sees
    // no unit id, only the opaque contact id.
    let clock = stamp(&tw).clock;
    install_ledger(
        &mut tw,
        vec![row_at(
            &clock,
            fx.alpha,
            [9; 16],
            true,
            1_000,
            Some(phantom.id().as_str()),
        )],
    );
    let seen = ok(&mut tw, fx.ann, "+contacts");
    for unit in [fx.alpha, fx.bravo, fx.target] {
        assert!(
            !seen.contains(&format!("#{}", unit.0)),
            "the render leaked #{}: {seen}",
            unit.0
        );
    }
}

/// A `False` contact and a real one **render identically** and cost the same
/// work: the two lines differ only in the opaque id, so neither the text nor
/// the read count is an oracle.
#[test]
fn e7c7_k2_a_false_and_a_real_contact_render_identically_and_cost_the_same_reads() {
    let (mut tw, fx) = scene();
    ok(
        &mut tw,
        GOD,
        &format!("@prop/phantom #{}=spoof,300", fx.alpha.0),
    );
    let phantom_id = book(&tw)
        .phantoms()
        .next()
        .map(|p| p.id().as_str().to_owned())
        .unwrap();
    let clock = stamp(&tw).clock;
    install_ledger(
        &mut tw,
        vec![
            row_at(&clock, fx.alpha, [0xaa; 16], true, 1_000, None),
            row_at(&clock, fx.alpha, [0xbb; 16], true, 1_000, Some(&phantom_id)),
        ],
    );
    let seen = ok(&mut tw, fx.ann, "+contacts");
    let lines: Vec<&str> = seen.lines().filter(|line| line.contains("gen 1")).collect();
    assert_eq!(lines.len(), 2, "{seen}");
    let strip = |line: &str| line.split_once("  ").map(|(_, rest)| rest.to_owned());
    assert_eq!(
        strip(lines[0]),
        strip(lines[1]),
        "a false contact renders exactly as a real one does"
    );
}

/// **One observer's contact is invisible to another on the same side**, and the
/// refusal a non-holder gets is byte-identical to the one a nonexistent id gets.
#[test]
fn e7c7_k3_one_observers_contact_is_invisible_to_another_and_the_two_refusals_are_identical() {
    let (mut tw, fx) = scene();
    let clock = stamp(&tw).clock;
    install_ledger(
        &mut tw,
        vec![row_at(&clock, fx.alpha, [0xaa; 16], true, 1_000, None)],
    );
    let hers = ok(&mut tw, fx.ann, "+contacts");
    assert!(hers.contains("aaaaaaaaaaaa"), "{hers}");
    let his = ok(&mut tw, fx.bob, "+contacts");
    assert!(his.contains("You hold no contacts."), "{his}");
    // The two refusals a viewer may not distinguish.
    let held_by_another = ok(&mut tw, fx.bob, "+contact aaaaaaaaaaaa");
    let nonexistent = ok(&mut tw, fx.bob, "+contact ffffffffffff");
    assert_eq!(held_by_another, nonexistent);
    assert!(
        held_by_another.contains(NO_SUCH_CONTACT),
        "{held_by_another}"
    );
}

// ======================================================================
// K — hazard axis 2: an observation that leaks the observed's truth
// ======================================================================

/// A recursive `ObjId` leak scan over every player-facing serialization the cut
/// produces finds **no undetected unit** (doc 09 Gate 3).
#[test]
fn e7c7_k4_a_leak_scan_over_every_player_facing_render_finds_no_undetected_unit() {
    let (mut tw, fx) = scene();
    // The target is placed but nobody has been told about it in any render.
    let clock = stamp(&tw).clock;
    install_ledger(
        &mut tw,
        vec![row_at(&clock, fx.alpha, [0xaa; 16], true, 1_000, None)],
    );
    let mut rendered = String::new();
    rendered.push_str(&ok(&mut tw, fx.ann, "+contacts"));
    rendered.push_str(&ok(&mut tw, fx.ann, "+contact aaaaaaaaaaaa"));
    rendered.push_str(&ok(&mut tw, fx.ann, "+orders"));
    rendered.push_str(&ok(
        &mut tw,
        fx.ann,
        &format!("@prop/envelopes #{}", fx.ann.0),
    ));
    assert!(
        !rendered.contains(&format!("#{}", fx.target.0)),
        "an undetected unit's id reached a player render: {rendered}"
    );
}

/// A detection **miss** and a **hit** carry the same lag and the same shape, so
/// timing is not an oracle — asserted through the resolver both passes use.
#[test]
fn e7c7_k5_a_detection_miss_and_a_hit_cost_the_same_lag_and_the_same_shape() {
    let (tw, fx) = scene();
    let state = state(&tw);
    let at = stamp(&tw);
    let context =
        mush_propagation::PropagationContext::build(&state, tw.world(), fx.alpha, fx.bravo, at)
            .expect("both ends are placed");
    let hit = mush_propagation::resolve_detection(&context, u32::MAX);
    let miss = mush_propagation::resolve_detection(&context, 0);
    assert!(hit.is_detected() && !miss.is_detected());
    assert_eq!(hit.lag(), miss.lag());
    assert_eq!(
        serde_json::to_value(hit)
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        serde_json::to_value(miss)
            .unwrap()
            .as_object()
            .unwrap()
            .len()
    );
}

/// `@prop/envelopes` on an **unviewable** subject and on a **nonexistent** one
/// are byte-identical (§4 rule 4).
#[test]
fn e7c7_k6_an_unviewable_and_a_nonexistent_envelope_subject_are_byte_identical() {
    let (mut tw, fx) = scene();
    deliver(&mut tw, fx.ann, fx.bob, "hold position");
    let unviewable = ok(&mut tw, fx.bob, &format!("@prop/envelopes #{}", fx.ann.0));
    let nonexistent = ok(&mut tw, fx.bob, "@prop/envelopes #999999");
    assert_eq!(unviewable, nonexistent);
    assert!(unviewable.contains(NO_SUCH_ENVELOPE), "{unviewable}");
}

// ======================================================================
// B — hazard axis 3: knowledge surviving after the subject is gone
// ======================================================================

/// **A phantom's expiry issues `Loss` to every holder.** The observable
/// outcome is the intent set the ingress will actually fire, one per holder.
#[test]
fn e7c7_b1_a_phantoms_expiry_issues_loss_to_every_holder() {
    let (mut tw, fx) = scene();
    ok(
        &mut tw,
        GOD,
        &format!("@prop/phantom #{}=decoy,10", fx.alpha.0),
    );
    let phantom_id = book(&tw)
        .phantoms()
        .next()
        .map(|p| p.id().as_str().to_owned())
        .unwrap();
    // TWO holders of the one phantom-backed contact.
    let clock = stamp(&tw).clock;
    install_ledger(
        &mut tw,
        vec![
            row_at(&clock, fx.alpha, [0xcc; 16], true, 1_000, Some(&phantom_id)),
            row_at(&clock, fx.bravo, [0xcc; 16], true, 1_000, Some(&phantom_id)),
        ],
    );
    tw.set_now(1_100);
    let state = state(&tw);
    let at = stamp(&tw);
    let ledger = tw
        .world()
        .module_state::<ContactLedgerState>()
        .cloned()
        .unwrap();
    let mut store = book(&tw);
    let intents = detection_pass(&mut store, &state, tw.world(), &ledger, &at, None, ARM).intents;
    let losses: Vec<_> = intents
        .iter()
        .filter(|intent| intent.kind() == mush_core::ObservationKind::Loss)
        .collect();
    // **One Loss per CONTACT, not per holder** (E7 cut 7 independent verify
    // fold, O9). The two rows are one shared `ContactState`, and
    // `checked_mark_lost` retires it for its whole audience in one transition;
    // arming a second against the same `expected_revision` was refused `Stale`
    // and left its durable cursor standing for ever. The outcome OPEN 5 names
    // — every holder loses it — is what one Loss on the shared record does.
    assert_eq!(losses.len(), 1, "one Loss per contact: {intents:?}");
    let loss = losses[0];
    assert!(matches!(
        loss.op(),
        mush_core::ObservationOp::Loss { contact, .. }
            if *contact == ContactId::from_bytes([0xcc; 16])
    ));
    assert!(
        [fx.alpha, fx.bravo].contains(&loss.observer().platform()),
        "the loss is armed by one of the two holders: {loss:?}"
    );
}

/// A report from a **destroyed observer** still renders, with its age. The
/// knowledge outlives the platform; only the phantom's expiry or the TTL ends
/// it.
#[test]
fn e7c7_b2_a_report_from_a_destroyed_observer_still_renders_with_its_age() {
    let (mut tw, fx) = scene();
    let clock = stamp(&tw).clock;
    install_ledger(
        &mut tw,
        vec![row_at(&clock, fx.ann, [0xdd; 16], true, 1_000, None)],
    );
    tw.world_mut().destroy(fx.bravo).unwrap();
    tw.set_now(1_045);
    let seen = ok(&mut tw, fx.ann, "+contacts");
    assert!(seen.contains("dddddddddddd"), "{seen}");
    assert!(seen.contains("45s"), "the age is rendered: {seen}");
}

/// An envelope whose recipient is **destroyed mid-flight** `Expire`s and is
/// **attributed**, rather than being retried forever.
#[test]
fn e7c7_b3_an_envelope_whose_recipient_is_destroyed_expires_and_is_attributed() {
    let (mut tw, fx) = scene();
    // Jam the link so the order is still in flight when the recipient dies.
    ok(&mut tw, GOD, &format!("@prop/jam #{}=9", fx.orbit.0));
    ok(&mut tw, fx.ann, &format!("+order #{}=advance", fx.bob.0));
    pass(&mut tw);
    let id = book(&tw).envelopes().next().map(Envelope::id).unwrap();
    assert_eq!(
        book(&tw).envelope(id).unwrap().state(),
        EnvelopeState::Queued
    );
    tw.world_mut().destroy(fx.bob).unwrap();
    let seen = pass(&mut tw);
    assert_eq!(
        book(&tw).envelope(id).unwrap().state(),
        EnvelopeState::Expired
    );
    assert!(
        seen.contains("recipient") && seen.contains(&format!("#{}", fx.bob.0)),
        "the expiry is attributed: {seen}"
    );
    // And it is not retried: a second pass changes nothing.
    let again = pass(&mut tw);
    assert!(again.contains("0 expired"), "{again}");
}

/// A contact past `CONTACT_TTL` is **lost**; one merely stale is **refreshed**.
/// Both arms of the staleness gate, enumerated.
#[test]
fn e7c7_b4_a_contact_past_contact_ttl_is_lost_and_one_merely_stale_is_refreshed() {
    let (mut tw, fx) = scene();
    tw.set_now(2_000);
    let at = stamp(&tw);
    let clock = at.clock.clone();
    install_ledger(
        &mut tw,
        vec![
            // 40 s old: past CONTACT_TTL (30) -> Loss.
            row_at(&clock, fx.alpha, [0x11; 16], true, at.at - 40, None),
            // 20 s old: in the refresh band (>= 10, < 30) -> Refresh.
            row_at(&clock, fx.bravo, [0x22; 16], true, at.at - 20, None),
            // 5 s old: neither arm fires.
            row_at(&clock, fx.alpha, [0x33; 16], true, at.at - 5, None),
        ],
    );
    let ledger = tw
        .world()
        .module_state::<ContactLedgerState>()
        .cloned()
        .unwrap();
    let state = state(&tw);
    let mut store = book(&tw);
    let intents = detection_pass(&mut store, &state, tw.world(), &ledger, &at, None, ARM).intents;
    let kinds: Vec<(u32, mush_core::ObservationKind)> = intents
        .iter()
        .filter(|intent| intent.op().contact().is_some())
        .map(|intent| (intent.observer().platform().0, intent.kind()))
        .collect();
    assert!(
        kinds.contains(&(fx.alpha.0, mush_core::ObservationKind::Loss)),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&(fx.bravo.0, mush_core::ObservationKind::Refresh)),
        "{kinds:?}"
    );
    assert_eq!(kinds.len(), 2, "the fresh contact produced nothing");
}

// ======================================================================
// O — the order lifecycle
// ======================================================================

/// Revision 3 delivered after 2 **supersedes** it, and the recipient obeys the
/// newest revision it has actually received.
#[test]
fn e7c7_o1_revision_3_after_2_supersedes_and_the_recipient_obeys_the_newest_received() {
    let (mut tw, fx) = scene();
    let first = deliver(&mut tw, fx.ann, fx.bob, "hold");
    let second = deliver(&mut tw, fx.ann, fx.bob, "advance");
    let store = book(&tw);
    assert_eq!(
        store.envelope(first).unwrap().state(),
        EnvelopeState::Superseded
    );
    assert_eq!(
        store.envelope(second).unwrap().state(),
        EnvelopeState::Delivered
    );
    // The OBSERVABLE outcome: what the recipient's own listing says is their
    // standing order.
    let seen = ok(&mut tw, fx.bob, "+orders");
    assert!(seen.contains("Standing order: revision 2"), "{seen}");
    assert!(seen.contains("advance"), "{seen}");
    assert_eq!(active_order(&store, fx.bob).map(Envelope::id), Some(second));
}

/// Revision 2 arriving **after** revision 3 is `Superseded` and **never becomes
/// active** at any point.
#[test]
fn e7c7_o2_an_older_revision_arriving_late_is_superseded_and_never_becomes_active() {
    let (mut tw, fx) = scene();
    // Queue the older one behind a jammed link, deliver the newer one, then
    // clear the jam so the older finally arrives.
    ok(&mut tw, GOD, &format!("@prop/jam #{}=9", fx.orbit.0));
    ok(&mut tw, fx.ann, &format!("+order #{}=old plan", fx.bob.0));
    let old = book(&tw).envelopes().next().map(Envelope::id).unwrap();
    pass(&mut tw);
    ok(&mut tw, GOD, &format!("@prop/jam #{}=0", fx.orbit.0));
    let new = deliver(&mut tw, fx.ann, fx.bob, "new plan");
    // The old one is still Queued and now arrives.
    pass(&mut tw);
    let store = book(&tw);
    assert_eq!(
        store.envelope(old).unwrap().state(),
        EnvelopeState::Superseded,
        "the late older revision is superseded ON ARRIVAL"
    );
    assert_eq!(active_order(&store, fx.bob).map(Envelope::id), Some(new));
    let seen = ok(&mut tw, fx.bob, "+orders");
    assert!(seen.contains("Standing order: revision 2"), "{seen}");
    assert!(!seen.contains("Standing order: revision 1"), "{seen}");
}

/// A **jammed recipient keeps its last delivered order** — the new one waits,
/// it is not lost, and the recipient's own listing still names the old one.
#[test]
fn e7c7_o3_a_jammed_recipient_keeps_its_last_delivered_order() {
    let (mut tw, fx) = scene();
    let first = deliver(&mut tw, fx.ann, fx.bob, "hold this line");
    ok(&mut tw, GOD, &format!("@prop/jam #{}=9", fx.orbit.0));
    ok(&mut tw, fx.ann, &format!("+order #{}=fall back", fx.bob.0));
    let seen_pass = pass(&mut tw);
    assert!(
        seen_pass.contains("waiting on a jammed link"),
        "{seen_pass}"
    );
    let store = book(&tw);
    assert_eq!(active_order(&store, fx.bob).map(Envelope::id), Some(first));
    let seen = ok(&mut tw, fx.bob, "+orders");
    assert!(seen.contains("hold this line"), "{seen}");
    assert!(seen.contains("Standing order: revision 1"), "{seen}");
}

/// On restored reachability the recipient gets the **newest valid revision**,
/// not the backlog: every older revision is superseded, and the standing order
/// is the newest.
#[test]
fn e7c7_o4_restored_reachability_delivers_the_newest_valid_revision_not_the_backlog() {
    let (mut tw, fx) = scene();
    ok(&mut tw, GOD, &format!("@prop/jam #{}=9", fx.orbit.0));
    for text in ["plan a", "plan b", "plan c"] {
        ok(&mut tw, fx.ann, &format!("+order #{}={text}", fx.bob.0));
    }
    pass(&mut tw);
    ok(&mut tw, GOD, &format!("@prop/jam #{}=0", fx.orbit.0));
    pass(&mut tw);
    let store = book(&tw);
    let active = active_order(&store, fx.bob).expect("one active order");
    assert_eq!(active.revision(), 3);
    assert_eq!(active.payload(), "plan c");
    let superseded = store
        .envelopes()
        .filter(|envelope| envelope.state() == EnvelopeState::Superseded)
        .count();
    assert_eq!(superseded, 2, "the backlog is superseded, not obeyed");
    let seen = ok(&mut tw, fx.bob, "+orders");
    assert!(seen.contains("Standing order: revision 3"), "{seen}");
}

/// **The issuer sees `Delivered` until the acknowledgement itself arrives**, and
/// a lost ack leaves the order `Delivered` with the issuer uninformed.
#[test]
fn e7c7_o5_an_ack_is_itself_a_delivery_and_a_lost_one_leaves_the_issuer_uninformed() {
    let (mut tw, fx) = scene();
    let order = deliver(&mut tw, fx.ann, fx.bob, "report in");
    assert_eq!(
        book(&tw).envelope(order).unwrap().state(),
        EnvelopeState::Delivered
    );
    // Jam the return path, then acknowledge: the ack cannot arrive.
    ok(&mut tw, GOD, &format!("@prop/jam #{}=9", fx.orbit.0));
    ok(&mut tw, fx.bob, &format!("+orders/ack {order}"));
    pass(&mut tw);
    assert_eq!(
        book(&tw).envelope(order).unwrap().state(),
        EnvelopeState::Delivered,
        "a lost ack leaves the order Delivered"
    );
    let issuer_view = ok(&mut tw, fx.ann, "+orders");
    assert!(
        !issuer_view.contains("Acknowledged"),
        "the issuer is uninformed: {issuer_view}"
    );
    // Clear the jam: the ack arrives, and only NOW does the issuer learn.
    ok(&mut tw, GOD, &format!("@prop/jam #{}=0", fx.orbit.0));
    pass(&mut tw);
    assert_eq!(
        book(&tw).envelope(order).unwrap().state(),
        EnvelopeState::Acknowledged
    );
    let after = ok(&mut tw, fx.ann, "+orders");
    assert!(after.contains("Acknowledged"), "{after}");
}

/// A **duplicate ack is a HIT**: no second envelope, no second write.
#[test]
fn e7c7_o6_a_duplicate_ack_is_a_hit() {
    let (mut tw, fx) = scene();
    let order = deliver(&mut tw, fx.ann, fx.bob, "confirm");
    ok(&mut tw, fx.bob, &format!("+orders/ack {order}"));
    let before = book(&tw).envelopes().count();
    let seen = ok(&mut tw, fx.bob, &format!("+orders/ack {order}"));
    assert!(seen.contains("Already acknowledged"), "{seen}");
    assert_eq!(book(&tw).envelopes().count(), before, "no second envelope");
    // An ack for an order the actor did not receive is the SAME refusal as one
    // for an order that does not exist.
    let not_mine = ok(&mut tw, fx.ann, &format!("+orders/ack {order}"));
    let nonexistent = ok(&mut tw, fx.ann, "+orders/ack 99999");
    assert_eq!(not_mine, nonexistent);
    assert!(not_mine.contains(NO_SUCH_ENVELOPE), "{not_mine}");
}

/// A relay **cycle** terminates at `MAX_ENVELOPE_HOPS` and does not grow age
/// without bound.
#[test]
fn e7c7_o7_a_relay_cycle_terminates_at_max_envelope_hops() {
    let (tw, fx) = scene();
    let at = stamp(&tw);
    let mut envelope = Envelope::issue(1, fx.ann, fx.bob, "loop".into(), 1, at.clone()).unwrap();
    for hop in 0..MAX_ENVELOPE_HOPS {
        let relay = if hop % 2 == 0 { fx.alpha } else { fx.bravo };
        envelope = envelope
            .relayed(
                relay,
                ClockStamp {
                    clock: at.clock.clone(),
                    at: at.at + i64::from(hop) + 1,
                },
            )
            .expect("within the bound");
    }
    assert!(
        envelope
            .relayed(
                fx.alpha,
                ClockStamp {
                    clock: at.clock.clone(),
                    at: at.at + 99
                }
            )
            .is_err(),
        "A->B->A terminates at the hop bound"
    );
    assert_eq!(envelope.path().len(), usize::from(MAX_ENVELOPE_HOPS));
}

/// **`contact_share` is submitted only after the carrying envelope is
/// `Delivered`** — a `Transmitted` (or queued) one shares nothing at all.
#[test]
fn e7c7_o8_a_share_fires_only_after_delivered_and_a_queued_envelope_shares_nothing() {
    let (mut tw, fx) = scene();
    let clock = stamp(&tw).clock;
    install_ledger(
        &mut tw,
        vec![row_at(&clock, fx.ann, [0xee; 16], true, 1_000, None)],
    );
    ok(&mut tw, GOD, &format!("@prop/jam #{}=9", fx.orbit.0));
    ok(
        &mut tw,
        fx.ann,
        &format!("+order/report #{}=eeeeeeeeeeee", fx.bob.0),
    );
    pass(&mut tw);
    let state = state(&tw);
    let at = stamp(&tw);
    let ledger = tw
        .world()
        .module_state::<ContactLedgerState>()
        .cloned()
        .unwrap();
    let mut store = book(&tw);
    let before = detection_pass(&mut store, &state, tw.world(), &ledger, &at, None, ARM).intents;
    assert!(
        !before
            .iter()
            .any(|intent| intent.kind() == mush_core::ObservationKind::Share),
        "a queued report shares nothing: {before:?}"
    );
    // Clear the jam and deliver it.
    ok(&mut tw, GOD, &format!("@prop/jam #{}=0", fx.orbit.0));
    pass(&mut tw);
    let mut store = book(&tw);
    let after = detection_pass(&mut store, &state, tw.world(), &ledger, &at, None, ARM).intents;
    let share = after
        .iter()
        .find(|intent| intent.kind() == mush_core::ObservationKind::Share)
        .expect("a delivered report shares");
    assert_eq!(share.observer().platform(), fx.ann);
}

/// **Expiry is idempotent**: a terminal envelope is not re-stamped and not
/// re-reported.
#[test]
fn e7c7_o9_expiry_is_idempotent() {
    let (mut tw, fx) = scene();
    let at = stamp(&tw);
    let mut store = book(&tw);
    let id = store.alloc_envelope().unwrap();
    let stale = Envelope::issue(
        id,
        fx.ann,
        fx.bob,
        "ancient".into(),
        1,
        ClockStamp {
            clock: at.clock.clone(),
            at: at.at - mush_propagation::ENVELOPE_TTL_SECS - 1,
        },
    )
    .unwrap();
    store.put_envelope(stale).unwrap();
    set_book(&mut tw, store);
    let state = state(&tw);
    let mut store = book(&tw);
    let first = delivery_pass(
        &mut store,
        &state,
        tw.world(),
        &ContactLedgerState::default(),
        &at,
        None,
    );
    assert_eq!(first.expired, vec![id]);
    let advanced = store.envelope(id).unwrap().advanced().at;
    let second = delivery_pass(
        &mut store,
        &state,
        tw.world(),
        &ContactLedgerState::default(),
        &at,
        None,
    );
    assert!(
        second.expired.is_empty(),
        "a terminal envelope is not revisited"
    );
    // …and, past its TTL, it is COMPACTED rather than kept for ever (E7 cut 7
    // independent verify fold, D7): the census caps used to be caps on lifetime
    // totals, so `+order` bricked permanently once 4_096 had ever been issued.
    assert_eq!(second.reaped.envelopes, vec![id]);
    assert!(store.envelope(id).is_none(), "the finished record is gone");
    let _ = advanced;
}

/// An over-size payload is **refused at issue**, not truncated at delivery, and
/// the actor is told.
#[test]
fn e7c7_o10_an_over_size_payload_is_refused_at_issue() {
    let (mut tw, fx) = scene();
    let long = "x".repeat(MAX_ENVELOPE_PAYLOAD_BYTES + 1);
    let seen = err(&mut tw, fx.ann, &format!("+order #{}={long}", fx.bob.0));
    assert!(seen.contains("payload"), "{seen}");
    assert_eq!(book(&tw).envelopes().count(), 0, "nothing was written");
}

// ======================================================================
// B — restart and bounds
// ======================================================================

/// **Every envelope state hydrates** across a reboot, and the allocator does not
/// re-use an id.
#[test]
fn e7c7_b5_every_envelope_state_hydrates_across_a_reboot() {
    let (mut tw, fx) = scene();
    let delivered = deliver(&mut tw, fx.ann, fx.bob, "one");
    let acknowledged = deliver(&mut tw, fx.ann, fx.bob, "two");
    ok(&mut tw, fx.bob, &format!("+orders/ack {acknowledged}"));
    pass(&mut tw);
    let before: Vec<(u64, EnvelopeState)> = book(&tw)
        .envelopes()
        .map(|envelope| (envelope.id(), envelope.state()))
        .collect();
    assert!(before.len() >= 3);
    let kv = fold_kv(&mut tw);
    let rebooted = reboot(kv);
    let after: Vec<(u64, EnvelopeState)> = book(&rebooted)
        .envelopes()
        .map(|envelope| (envelope.id(), envelope.state()))
        .collect();
    assert_eq!(before, after, "every state hydrated exactly once");
    // The allocator never re-uses an id, even from a book alone.
    let mut store = book(&rebooted);
    let next = store.alloc_envelope().unwrap();
    assert!(next > before.iter().map(|(id, _)| *id).max().unwrap());
    let _ = delivered;
}

/// A `Transmitted`/in-flight envelope at restart is **neither delivered twice
/// nor lost**: it hydrates in flight and one further pass delivers it once.
#[test]
fn e7c7_b6_an_in_flight_envelope_at_restart_is_neither_delivered_twice_nor_lost() {
    let (mut tw, fx) = scene();
    ok(&mut tw, GOD, &format!("@prop/jam #{}=9", fx.orbit.0));
    ok(&mut tw, fx.ann, &format!("+order #{}=in flight", fx.bob.0));
    pass(&mut tw);
    let id = book(&tw).envelopes().next().map(Envelope::id).unwrap();
    assert_eq!(
        book(&tw).envelope(id).unwrap().state(),
        EnvelopeState::Queued
    );
    let kv = fold_kv(&mut tw);
    let mut rebooted = reboot(kv);
    rebooted.set_now(1_000);
    assert_eq!(
        book(&rebooted).envelope(id).unwrap().state(),
        EnvelopeState::Queued,
        "in flight at restart, not delivered and not lost"
    );
    // The jam hydrated with it, so it is still waiting rather than silently
    // arriving on a fresh world.
    assert_eq!(book(&rebooted).jamming_at(fx.orbit), 9);
}

/// A corrupt `Envelope`, `Phantom` or `ContactProvenance` is **skipped loudly
/// and HELD, never deleted** — the record is the only place its fact lives.
#[test]
fn e7c7_b7_a_corrupt_record_is_held_loudly_not_deleted() {
    let (mut tw, fx) = scene();
    deliver(&mut tw, fx.ann, fx.bob, "readable");
    ok(
        &mut tw,
        GOD,
        &format!("@prop/phantom #{}=artefact,60", fx.alpha.0),
    );
    let mut kv = fold_kv(&mut tw);
    // Corrupt one of each kind: an envelope past the hop bound, and a phantom
    // whose expiry precedes its creation.
    for (key, value) in kv.iter_mut() {
        if key.starts_with("envelope:") {
            value["path"] = serde_json::json!(
                (0..=MAX_ENVELOPE_HOPS)
                    .map(|n| serde_json::json!({
                        "relay": 1,
                        "at": { "clock": { "kind": "utc" }, "at": n }
                    }))
                    .collect::<Vec<_>>()
            );
        }
        if key.starts_with("phantom:") {
            value["expires"] = value["created"].clone();
        }
    }
    let held_keys: Vec<String> = kv
        .iter()
        .map(|(key, _)| key.to_string())
        .filter(|key| key.starts_with("envelope:") || key.starts_with("phantom:"))
        .collect();
    let rebooted = reboot(kv);
    let store = book(&rebooted);
    assert_eq!(
        store.envelopes().count(),
        0,
        "the corrupt envelope did not load"
    );
    assert_eq!(store.phantoms().count(), 0);
    for key in &held_keys {
        assert!(
            store.held().contains_key(key.as_str()),
            "{key} was deleted rather than held"
        );
    }
}

/// The delivery pass's bound is **paged, not logged**: the operator is told, and
/// nothing is truncated silently.
#[test]
fn e7c7_b8_the_delivery_pass_bound_is_paged_not_logged() {
    let (mut tw, fx) = scene();
    ok(&mut tw, GOD, &format!("@prop/jam #{}=9", fx.orbit.0));
    let mut store = book(&tw);
    let at = stamp(&tw);
    for _ in 0..=MAX_ENVELOPES_PER_PASS {
        let id = store.alloc_envelope().unwrap();
        store
            .put_envelope(
                Envelope::issue(id, fx.ann, fx.bob, "many".into(), 1, at.clone()).unwrap(),
            )
            .unwrap();
    }
    set_book(&mut tw, store);
    let seen = pass(&mut tw);
    assert!(
        seen.contains("stopped at its bound") && seen.contains("run @prop/pass again"),
        "the bound pages with the command that finishes the work: {seen}"
    );
}

// ======================================================================
// P — wiring and projection
// ======================================================================

/// Every verb's recorded compatibility class, in the two registration groups
/// the module declares — the K8.5 lane, pinned locally as well as in
/// `mush-server`'s production-composition test.
#[test]
fn e7c7_p1_every_verbs_recorded_compatibility_class_matches_its_group() {
    let (tw, _fx) = scene();
    let registrations: Vec<_> = tw.registry().command_registrations().collect();
    for (name, class) in [
        ("+contacts", mush_core::CommandCompatibility::ReadOnly),
        ("+contact", mush_core::CommandCompatibility::ReadOnly),
        ("+order", mush_core::CommandCompatibility::LegacyPostImage),
        ("+orders", mush_core::CommandCompatibility::LegacyPostImage),
        ("@prop", mush_core::CommandCompatibility::Authoritative),
    ] {
        assert!(
            registrations.iter().any(|(spec, owner, published)| {
                spec.name == name && *owner == Some("propagation") && *published == class
            }),
            "{name} must publish {class:?}"
        );
    }
}

/// **Every arm of the `@prop` staff gate is covered**: `/pass`, `/phantom` and
/// `/jam` all refuse a non-staff actor, `/envelopes` does not (it is the read),
/// and an unknown switch **refuses rather than falling through to the read**.
#[test]
fn e7c7_p2_the_prop_staff_gate_covers_pass_phantom_and_jam_and_an_unknown_switch_refuses() {
    let (mut tw, fx) = scene();
    for line in [
        "@prop/pass".to_owned(),
        format!("@prop/phantom #{}=decoy", fx.alpha.0),
        format!("@prop/jam #{}=3", fx.orbit.0),
    ] {
        let seen = err(&mut tw, fx.ann, &line);
        assert!(seen.contains("Permission denied"), "{line}: {seen}");
    }
    // The read arm is open to the subject themselves.
    ok(&mut tw, fx.ann, &format!("@prop/envelopes #{}", fx.ann.0));
    // An unknown switch is a REFUSAL, never a silent fall-through to the read:
    // the dispatcher rejects a switch the spec does not declare, and the handler
    // enumerates every arm it does declare rather than defaulting.
    let unknown = err(&mut tw, GOD, "@prop/nonsense");
    assert!(!unknown.contains(NO_SUCH_ENVELOPE), "{unknown}");
}

/// The phantom class constant agrees between the two crates that use it, so the
/// leak-safe ledger recognises exactly the subjects propagation mints.
#[test]
fn e7c7_p3_the_phantom_class_and_owner_agree_between_tactics_and_propagation() {
    assert_eq!(mush_propagation::PHANTOM_CLASS, mush_tactics::PHANTOM_CLASS);
    assert_eq!(mush_propagation::NS, mush_tactics::PHANTOM_OWNER);
}

/// An **unwired** sensor policy arms nothing at all — fail closed.
#[test]
fn e7c7_p4_an_unwired_sensor_policy_arms_nothing() {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(SimulationModule::new()),
        Box::new(TacticsModule::new()),
        Box::new(PropagationModule::new()),
    ]);
    tw.set_now(1_000);
    let fx = build(&mut tw);
    let _ = fx;
    let handle = mush_core::SensorPolicyHandle::unwired();
    assert!(!handle.is_wired());
    assert!(handle.due(tw.world(), &stamp(&tw)).unwrap().is_empty());
}

/// The delivery **tick** is the driver, and it is disjoint from every other tick
/// tag on the tree. A coarse fire advances the same envelopes `@prop/pass` does.
#[test]
fn e7c7_p5_the_coarse_delivery_tick_advances_an_envelope_without_a_staff_verb() {
    let (mut tw, fx) = scene();
    ok(&mut tw, fx.ann, &format!("+order #{}=by tick", fx.bob.0));
    assert_eq!(
        book(&tw).envelopes().next().unwrap().state(),
        EnvelopeState::Queued
    );
    tw.set_now(1_002);
    let _ = tw.fire_due(1_002_000);
    assert_eq!(
        book(&tw).envelopes().next().unwrap().state(),
        EnvelopeState::Delivered,
        "the coarse tick is the driver, not the staff verb"
    );
    // And the OBSERVABLE outcome: the recipient's own listing now names it.
    let seen = ok(&mut tw, fx.bob, "+orders");
    assert!(seen.contains("Standing order: revision 1"), "{seen}");
}

/// The two resolvers answer R4's two cases through the pass, not just in
/// isolation: a **jammed** platform is still detected, and an **occluded** one
/// still receives.
#[test]
fn e7c7_p6_a_jammed_platform_is_still_detected_and_an_occluded_one_still_receives() {
    let (mut tw, fx) = scene();
    ok(&mut tw, GOD, &format!("@prop/jam #{}=9", fx.orbit.0));
    let state = state(&tw);
    let at = stamp(&tw);
    let ledger = ContactLedgerState::default();
    let mut store = book(&tw);
    let intents = detection_pass(&mut store, &state, tw.world(), &ledger, &at, None, ARM).intents;
    assert!(
        intents
            .iter()
            .any(|intent| intent.kind() == mush_core::ObservationKind::Observe),
        "jamming never suppresses detection"
    );
    // Occlusion: move the third platform inside Alpha's hull. Delivery still
    // works across the boundary a sensor cannot see through.
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/interior #{}=#{}", fx.target.0, fx.hull.0),
    );
    ok(&mut tw, GOD, &format!("@prop/jam #{}=0", fx.orbit.0));
    let state = crate::state(&tw);
    let at = stamp(&tw);
    let context = mush_propagation::PropagationContext::build(
        &state,
        tw.world(),
        fx.alpha,
        fx.target,
        at.clone(),
    )
    .expect("both ends placed");
    assert!(
        !mush_propagation::resolve_detection(&context, mush_propagation::DEFAULT_SIGNATURE)
            .is_detected(),
        "the hull occludes the sensor"
    );
    assert!(
        mush_propagation::resolve_delivery(&context).is_reachable(),
        "but a courier still crosses it"
    );
}

// ======================================================================
// ---- E7 cut 7 independent verify fold ----
//
// Every test below asserts the OBSERVABLE outcome — a render, a refusal's
// exact bytes, what a second actor perceives, the recipient's own standing
// order, the document the ingress will actually commit — not merely the
// record the code just wrote. Owner decisions taken: D1(a), D2(a), D3(a) both
// halves, D4(a)+(c), D5(a) strengthened with the kernel change, D6(a), D7(a).
// ======================================================================

use mush_propagation::{
    DetectionReport, MAX_CONTEXT_RECORDS, MAX_ENVELOPES, PropagationContext, Sighting,
    domain_of_place, place_of, resolve_detection, standing_orders,
};
use mush_sim::{SpatialParent, UnplacedInfo};

/// A third player, for the cases that need somebody who is nobody's commander.
fn third_player(tw: &mut TestWorld) -> ObjId {
    let carl = tw
        .world_mut()
        .create(ObjKind::Player, "Carl", GOD, Some(LIMBO))
        .unwrap();
    bind(tw, carl, "sim.participant");
    ok(tw, GOD, &format!("@spatial/place/domain #{}=0", carl.0));
    carl
}

/// `n` further placed things in domain 0 — the volume the bound cases need.
fn extras(tw: &mut TestWorld, n: usize) -> Vec<ObjId> {
    let mut made = Vec::with_capacity(n);
    for index in 0..n {
        let obj = tw
            .world_mut()
            .create(ObjKind::Thing, &format!("Drone {index}"), GOD, Some(LIMBO))
            .unwrap();
        bind(tw, obj, "sim.participant");
        ok(tw, GOD, &format!("@spatial/place/domain #{}=0", obj.0));
        made.push(obj);
    }
    made
}

/// A second live domain, and one placed thing in it.
fn far_domain(tw: &mut TestWorld) -> (ObjId, ObjId) {
    let deep = tw
        .world_mut()
        .create(ObjKind::Room, "Deep Space", GOD, None)
        .unwrap();
    bind(tw, deep, "sim.domain");
    ok(tw, GOD, &format!("@domain/create #{}=sim.passive", deep.0));
    let far = tw
        .world_mut()
        .create(ObjKind::Thing, "Far Ship", GOD, Some(LIMBO))
        .unwrap();
    bind(tw, far, "sim.participant");
    ok(tw, GOD, &format!("@spatial/place/domain #{}=1", far.0));
    (deep, far)
}

/// Force a placement into cut 6's quarantine projection (the recovery room).
fn quarantine(tw: &mut TestWorld, objs: &[ObjId]) {
    let spatial = tw.world_mut().module_state_mut::<SpatialState>().unwrap();
    for obj in objs {
        let record = spatial.placements.get_mut(obj).unwrap();
        record.parent = SpatialParent::Unplaced;
        record.unplaced = Some(UnplacedInfo {
            reason: "fold".to_owned(),
            by: None,
            at: 0,
            previous: None,
            quarantined: true,
        });
    }
    spatial.derived = mush_sim::graph::rebuild(spatial);
}

fn detect(tw: &TestWorld, store: &mut PropagationStore, bound: usize) -> DetectionReport {
    let state = state(tw);
    let at = stamp(tw);
    let ledger = tw
        .world()
        .module_state::<ContactLedgerState>()
        .cloned()
        .unwrap_or_default();
    detection_pass(store, &state, tw.world(), &ledger, &at, None, bound)
}

/// The intent document the last `@prop/pass` actually queued for the ingress.
fn armed_document(tw: &mut TestWorld) -> String {
    ok(tw, GOD, "@prop/pass");
    let mut text = String::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        if let Some(intents) = submit.parameters.get("intents").and_then(|v| v.as_str()) {
            text.push_str(intents);
        }
    }
    text
}

// ---- D1: what bounds detection --------------------------------------

/// **D1(a).** A unit in another domain is not detected at all — and a
/// same-domain control pair still is, so the fix cannot be satisfied by
/// disabling detection.
#[test]
fn vf_k1a_a_unit_in_another_domain_is_not_detected_while_a_same_domain_pair_still_detects() {
    let (mut tw, fx) = scene();
    let (_deep, far) = far_domain(&mut tw);
    let spatial = state(&tw);
    let at = stamp(&tw);
    assert_ne!(
        place_of(&spatial, tw.world(), fx.alpha),
        place_of(&spatial, tw.world(), far),
        "the two ends really are in different places"
    );
    assert_ne!(
        domain_of_place(
            &spatial,
            tw.world(),
            place_of(&spatial, tw.world(), fx.alpha).unwrap()
        ),
        domain_of_place(
            &spatial,
            tw.world(),
            place_of(&spatial, tw.world(), far).unwrap()
        ),
        "and in different domains"
    );
    assert!(
        PropagationContext::build(&spatial, tw.world(), fx.alpha, far, at.clone()).is_none(),
        "a pair the placement graph cannot connect is not a context at all"
    );
    // The control: a same-domain pair still detects.
    let control = PropagationContext::build(&spatial, tw.world(), fx.alpha, fx.bravo, at.clone())
        .expect("a same-domain pair is still a context");
    assert!(resolve_detection(&control, mush_propagation::DEFAULT_SIGNATURE).is_detected());
    // And the pass never arms an Observe on the cross-domain unit.
    let mut store = book(&tw);
    let report = detect(&tw, &mut store, ARM);
    assert!(
        !report.intents.iter().any(|intent| matches!(
            intent.op(),
            mush_core::ObservationOp::Observe { truth: SubjectKey::Object { id }, .. }
                if *id == far
        )),
        "the detection pass observed a unit in another domain: {report:?}"
    );
    assert!(
        report.intents.iter().any(|intent| matches!(
            intent.op(),
            mush_core::ObservationOp::Observe { truth: SubjectKey::Object { id }, .. }
                if *id == fx.bravo
        )),
        "but the same-domain pair is still observed"
    );
}

/// **D1(a), the observable half.** No cross-domain unit reaches the document
/// the tactics ingress will commit — which is where a leak would become a
/// player-visible `ContactState`.
#[test]
fn vf_k1b_no_cross_domain_unit_reaches_the_armed_ingress_document() {
    let (mut tw, fx) = scene();
    let (_deep, far) = far_domain(&mut tw);
    let document = armed_document(&mut tw);
    assert!(!document.is_empty(), "the pass armed something");
    assert!(
        !document.contains(&format!("\"id\":{}", far.0)),
        "a unit in another domain reached the ingress document: {document}"
    );
    assert!(
        document.contains(&format!("\"id\":{}", fx.bravo.0)),
        "the same-domain unit still does: {document}"
    );
}

/// **B15(a).** The declared-but-dead bound now binds: a build over a large
/// domain map still answers, and the walk is capped at `MAX_CONTEXT_RECORDS`.
#[test]
fn vf_b15_a_context_build_visits_at_most_max_context_records() {
    let (mut tw, fx) = scene();
    for index in 0..(MAX_CONTEXT_RECORDS + 8) {
        let room = tw
            .world_mut()
            .create(ObjKind::Room, &format!("Domain {index}"), GOD, None)
            .unwrap();
        bind(&mut tw, room, "sim.domain");
        ok(
            &mut tw,
            GOD,
            &format!("@domain/create #{}=sim.passive", room.0),
        );
    }
    let spatial = state(&tw);
    let at = stamp(&tw);
    assert!(spatial.domains.len() > MAX_CONTEXT_RECORDS);
    assert!(PropagationContext::build(&spatial, tw.world(), fx.alpha, fx.bravo, at).is_some());
}

// ---- D2: where an object that is nowhere is --------------------------

/// **D2(a).** Two quarantined platforms both project to the one nowhere-room
/// and must NOT observe each other; a third, still placed, is unaffected.
#[test]
fn vf_k2a_two_carriers_under_way_do_not_observe_each_other() {
    let (mut tw, fx) = scene();
    {
        let spatial = tw.world().module_state::<SpatialState>().unwrap();
        assert_eq!(
            spatial.recovery_room, spatial.in_transit_room,
            "both projection rooms default to the same nowhere-room, which is the hazard"
        );
    }
    quarantine(&mut tw, &[fx.alpha, fx.bravo]);
    let spatial = state(&tw);
    let at = stamp(&tw);
    assert!(
        place_of(&spatial, tw.world(), fx.alpha).is_none(),
        "a projection room is not a place"
    );
    assert!(
        PropagationContext::build(&spatial, tw.world(), fx.alpha, fx.bravo, at.clone()).is_none(),
        "two platforms that are nowhere are not co-located"
    );
    assert!(
        PropagationContext::build(&spatial, tw.world(), fx.target, fx.ann, at).is_some(),
        "a platform that is really somewhere is unaffected"
    );
    let mut store = book(&tw);
    let report = detect(&tw, &mut store, ARM);
    assert!(
        !report.intents.iter().any(|intent| {
            intent.observer().platform() == fx.alpha || intent.observer().platform() == fx.bravo
        }),
        "nothing is armed from nowhere: {report:?}"
    );
}

/// **D2(a)'s accepted consequence.** An order addressed to a platform that is
/// nowhere **waits** — store-and-forward — and the issuer's own listing says so
/// rather than reporting a delivery.
#[test]
fn vf_k2b_an_order_to_a_ship_under_way_waits_and_says_so() {
    let (mut tw, fx) = scene();
    ok(
        &mut tw,
        fx.ann,
        &format!("+order #{}=make for the lane", fx.bob.0),
    );
    quarantine(&mut tw, &[fx.bob]);
    let seen = pass(&mut tw);
    assert!(seen.contains("waiting on a jammed link"), "{seen}");
    let issuer_view = ok(&mut tw, fx.ann, "+orders");
    assert!(
        issuer_view.contains("Queued"),
        "the order waits: {issuer_view}"
    );
    assert!(
        !issuer_view.contains("Delivered"),
        "and it is not delivered to nowhere: {issuer_view}"
    );
    assert!(
        standing_orders(&book(&tw), fx.bob).is_empty(),
        "the recipient has no standing order it never received"
    );
}

// ---- D5: what a world rate change does to knowledge ------------------

/// **D5(a).** After a world time-rate change the delivery tick re-anchors
/// propagation's own records, so an envelope ages out and a phantom expires
/// again instead of being frozen for ever.
#[test]
fn vf_k3a_a_rate_change_ages_out_an_envelope_and_expires_a_phantom() {
    let (mut tw, fx) = scene();
    ok(
        &mut tw,
        GOD,
        &format!("@prop/phantom #{}=decoy,10", fx.alpha.0),
    );
    ok(&mut tw, GOD, &format!("@prop/jam #{}=9", fx.orbit.0));
    let at = stamp(&tw);
    let mut store = book(&tw);
    let id = store.alloc_envelope().unwrap();
    store
        .put_envelope(
            Envelope::issue(
                id,
                fx.ann,
                fx.bob,
                "ancient".into(),
                1,
                ClockStamp {
                    clock: at.clock.clone(),
                    at: at.at - mush_propagation::ENVELOPE_TTL_SECS - 1,
                },
            )
            .unwrap(),
        )
        .unwrap();
    set_book(&mut tw, store);
    // The kernel's own rate change: a new generation, a CONTINUOUS IC scalar.
    let epoch = tw.world().next_world_timeline_epoch(2).unwrap();
    tw.world_mut().append_world_timeline_epoch(epoch).unwrap();
    let after = stamp(&tw);
    assert_eq!(after.clock, ClockRef::WorldIc { generation: 2 });
    assert_eq!(
        after.at, at.at,
        "the IC scalar is continuous by construction"
    );
    tw.set_now(at.at + 20);
    let _ = tw.fire_due((at.at + 20) * 1_000);
    assert_eq!(
        book(&tw).envelope(id).map(Envelope::state),
        Some(EnvelopeState::Expired),
        "an envelope past its TTL still expires across a rate change"
    );
    let now = stamp(&tw);
    assert!(
        book(&tw).phantoms().all(|phantom| phantom.is_expired(&now)),
        "and a phantom past its expiry is expired again"
    );
}

/// **D5(a)(i).** A contact stamped before the change never renders as freshly
/// observed — the one player-facing view may not look fresher than the truth.
#[test]
fn vf_k3b_a_pre_change_contact_never_renders_as_freshly_observed() {
    let (mut tw, fx) = scene();
    let clock = stamp(&tw).clock;
    install_ledger(
        &mut tw,
        vec![row_at(&clock, fx.ann, [0xab; 16], true, 960, None)],
    );
    let epoch = tw.world().next_world_timeline_epoch(2).unwrap();
    tw.world_mut().append_world_timeline_epoch(epoch).unwrap();
    let seen = ok(&mut tw, fx.ann, "+contacts");
    assert!(
        !seen.contains("held  0s"),
        "a 40-second-old track rendered as observed this second: {seen}"
    );
    assert!(
        seen.contains("stale"),
        "it renders as stale instead: {seen}"
    );
}

/// **D5(a)(iii) and the kernel half.** The stranded contacts are counted, paged
/// with their number, and RETIRED — the pass arms a `clock_generation` Loss
/// that the kernel now admits, so knowledge never freezes for ever.
#[test]
fn vf_k3c_the_stranded_contacts_are_paged_and_retired() {
    let (mut tw, fx) = scene();
    let clock = stamp(&tw).clock;
    install_ledger(
        &mut tw,
        vec![
            row_at(&clock, fx.alpha, [0xab; 16], true, 960, None),
            row_at(&clock, fx.bravo, [0xcd; 16], true, 960, None),
        ],
    );
    let epoch = tw.world().next_world_timeline_epoch(2).unwrap();
    tw.world_mut().append_world_timeline_epoch(epoch).unwrap();
    let mut store = book(&tw);
    let report = detect(&tw, &mut store, ARM);
    assert_eq!(report.stranded, 2, "both stranded contacts are counted");
    let retired: Vec<_> = report
        .intents
        .iter()
        .filter(|intent| intent.kind() == mush_core::ObservationKind::Loss)
        .collect();
    assert_eq!(
        retired.len(),
        2,
        "one Loss per stranded contact: {retired:?}"
    );
    assert!(retired.iter().all(|intent| matches!(
        intent.op(),
        mush_core::ObservationOp::Loss { reason, .. }
            if reason.as_str() == mush_propagation::CLOCK_GENERATION_REASON
    )));
    let seen = pass(&mut tw);
    assert!(
        seen.contains("stranded on an older world time generation"),
        "{seen}"
    );
    assert!(seen.contains('2'), "{seen}");
}

// ---- D3: the cursor's identity ---------------------------------------

/// **D3(a).** The same still-due transition one IC second later arms under the
/// SAME key, so `sensor_arm` replaces one standing cursor instead of inserting
/// a second beside it. Deviation (i)'s stated mitigation is true again.
#[test]
fn vf_k4a_a_second_pass_replaces_the_standing_cursor() {
    let (mut tw, fx) = scene();
    let clock = stamp(&tw).clock;
    let at = stamp(&tw);
    install_ledger(
        &mut tw,
        vec![row_at(&clock, fx.alpha, [0x09; 16], true, at.at - 40, None)],
    );
    let mut store = book(&tw);
    let first = detect(&tw, &mut store, ARM);
    let key_a = first
        .intents
        .iter()
        .find(|intent| intent.kind() == mush_core::ObservationKind::Loss)
        .map(|intent| intent.key().as_str().to_owned())
        .expect("a past-TTL contact is lost");
    tw.set_now(1_001);
    let mut store = book(&tw);
    let second = detect(&tw, &mut store, ARM);
    let key_b = second
        .intents
        .iter()
        .find(|intent| intent.kind() == mush_core::ObservationKind::Loss)
        .map(|intent| intent.key().as_str().to_owned())
        .expect("still due one IC second later");
    assert_eq!(
        key_a, key_b,
        "one IC second later the same due transition must arm the SAME cursor"
    );
    assert_eq!(
        key_a.matches('/').count(),
        2,
        "the key is <kind>/<observer>/c<contact> with NO tick: {key_a}"
    );
}

/// **D3(a), second half.** A transition whose holder is gone or `GOING` is
/// never armed: its cursor could never commit and would stand for ever.
#[test]
fn vf_k4c_a_loss_is_not_armed_for_a_destroyed_holder() {
    let (mut tw, fx) = scene();
    let clock = stamp(&tw).clock;
    let at = stamp(&tw);
    install_ledger(
        &mut tw,
        vec![
            row_at(&clock, fx.target, [0x77; 16], true, at.at - 40, None),
            row_at(&clock, fx.alpha, [0x78; 16], true, at.at - 40, None),
        ],
    );
    tw.world_mut().destroy(fx.target).unwrap();
    let mut store = book(&tw);
    let report = detect(&tw, &mut store, ARM);
    assert!(
        !report
            .intents
            .iter()
            .any(|intent| intent.observer().platform() == fx.target),
        "a transition was armed for a destroyed holder: {report:?}"
    );
    assert!(
        report
            .intents
            .iter()
            .any(|intent| intent.observer().platform() == fx.alpha),
        "while a live holder's is still armed"
    );
}

// ---- D6: orders — addressability, attribution, per-issuer scope -------

/// **D6(a).** An order to a dbref the actor cannot address refuses
/// **byte-identically** to one for an object that does not exist, and neither
/// refusal names the target — the existence-and-identity oracle is closed.
#[test]
fn vf_k5_an_order_to_an_undetected_dbref_refuses_identically_to_a_nonexistent_one() {
    let (mut tw, fx) = scene();
    let vault = tw
        .world_mut()
        .create(ObjKind::Room, "Vault", GOD, None)
        .unwrap();
    let secret = tw
        .world_mut()
        .create(ObjKind::Thing, "Black Prince", GOD, Some(vault))
        .unwrap();
    let undetected = ok(&mut tw, fx.ann, &format!("+order #{}=go", secret.0));
    let nonexistent = ok(&mut tw, fx.ann, "+order #99999=go");
    assert_eq!(
        undetected, nonexistent,
        "an undetected dbref and a nonexistent one must be indistinguishable"
    );
    assert!(undetected.contains(NO_SUCH_CONTACT), "{undetected}");
    assert!(
        !undetected.contains("Black Prince"),
        "a never-detected unit's identity reached a render: {undetected}"
    );
    ok(&mut tw, fx.ann, &format!("+order #{}=hold", fx.bob.0));
}

/// **D6(a).** A stranger cannot displace a commander's standing order, and the
/// recipient's own listing names each order's source.
#[test]
fn vf_k8_a_stranger_cannot_displace_a_commanders_standing_order() {
    let (mut tw, fx) = scene();
    let carl = third_player(&mut tw);
    for text in ["hold the line", "hold again", "hold once more"] {
        ok(&mut tw, fx.ann, &format!("+order #{}={text}", fx.bob.0));
        pass(&mut tw);
    }
    // Carl is nobody's commander and holds no relationship to Bob at all.
    ok(
        &mut tw,
        carl,
        &format!("+order #{}=break formation", fx.bob.0),
    );
    pass(&mut tw);
    let store = book(&tw);
    let standing = standing_orders(&store, fx.bob);
    assert_eq!(
        standing.len(),
        2,
        "one standing order per issuer: {standing:?}"
    );
    let from_ann = standing
        .iter()
        .find(|envelope| envelope.issuer() == fx.ann)
        .expect("the incumbent commander's order survives");
    assert_eq!(from_ann.revision(), 3);
    assert_eq!(from_ann.payload(), "hold once more");
    assert_eq!(from_ann.state(), EnvelopeState::Delivered);
    // The OBSERVABLE half: the recipient's own listing names both sources.
    let seen = ok(&mut tw, fx.bob, "+orders");
    assert!(seen.contains("hold once more"), "{seen}");
    assert!(seen.contains("break formation"), "{seen}");
    assert!(
        seen.contains("(from Ann)"),
        "the issuer is rendered: {seen}"
    );
    assert!(seen.contains("(from Carl)"), "for every issuer: {seen}");
}

// ---- D4: what a bounded pass may durably claim ------------------------

/// **D4(a).** The detection bound PAGES and RESUMES: consecutive passes between
/// them observe every subject, and the first names where it stopped.
#[test]
fn vf_k7a_the_detection_bound_pages_and_resumes() {
    let (mut tw, _fx) = scene();
    let drones = extras(&mut tw, 40);
    let mut store = book(&tw);
    let mut seen: Vec<u32> = Vec::new();
    let mut paged = false;
    for _ in 0..40 {
        let report = detect(&tw, &mut store, ARM);
        paged |= report.bound_hit && report.resume.is_some();
        for intent in &report.intents {
            if let mush_core::ObservationOp::Observe {
                truth: SubjectKey::Object { id },
                ..
            } = intent.op()
            {
                seen.push(id.0);
            }
        }
        if report.intents.is_empty() {
            break;
        }
    }
    assert!(
        paged,
        "the bound bound at least once, and named its resume point"
    );
    for drone in &drones {
        assert!(
            seen.contains(&drone.0),
            "drone #{} was never observed by any pass — the remainder must be reachable",
            drone.0
        );
    }
}

/// **D4(a), the observable half.** The paged remainder really is still due on
/// the next run: the second `@prop/pass` does NOT answer "nothing due", because
/// the book only remembers what was actually armed.
#[test]
fn vf_k7b_the_paged_remainder_is_still_due_on_the_second_run() {
    let (mut tw, _fx) = scene();
    let _ = extras(&mut tw, 12);
    let first = pass(&mut tw);
    assert!(
        first.contains("armed through the tactics ingress"),
        "{first}"
    );
    assert!(
        first.contains("run @prop/pass again"),
        "the bound is paged with the command that continues it: {first}"
    );
    let second = pass(&mut tw);
    assert!(
        !second.contains("Detection pass: nothing due."),
        "the paged remainder was lost for good: {second}"
    );
    assert!(
        second.contains("armed through the tactics ingress"),
        "{second}"
    );
}

/// **D4(a).** A full sighting book REFUSES the observation rather than arming
/// it under a fresh key on every pass: the census error is never swallowed.
#[test]
fn vf_k7c_a_full_sighting_book_refuses_rather_than_rearming() {
    let (tw, _fx) = scene();
    let at = stamp(&tw);
    let mut store = book(&tw);
    for n in 0..mush_propagation::MAX_SIGHTINGS {
        store
            .put_sighting(
                Sighting::new(
                    ObjId(900_000 + u32::try_from(n).unwrap()),
                    Some(ObjId(1)),
                    None,
                    at.clone(),
                    at.clone(),
                )
                .unwrap(),
            )
            .unwrap();
    }
    let report = detect(&tw, &mut store, ARM);
    assert!(
        report.intents.is_empty(),
        "no observation is armed without an idempotency record: {report:?}"
    );
    assert!(
        report
            .refused
            .iter()
            .any(|line| line.contains("was not armed")),
        "and the refusal names the numbers: {:?}",
        report.refused
    );
}

// ---- D7: how long a finished record lives ----------------------------

/// **D7(a).** Finished traffic is compacted, so `+order` still works at the
/// census instead of bricking permanently for every actor.
#[test]
fn vf_b1a_finished_traffic_is_compacted_and_order_still_works_at_the_census() {
    let (mut tw, fx) = scene();
    let at = stamp(&tw);
    let long_ago = ClockStamp {
        clock: at.clock.clone(),
        at: at.at - mush_propagation::ENVELOPE_TTL_SECS - 1,
    };
    let mut store = book(&tw);
    for _ in 0..MAX_ENVELOPES {
        let id = store.alloc_envelope().unwrap();
        let envelope =
            Envelope::issue(id, fx.ann, fx.bob, "done".into(), 1, long_ago.clone()).unwrap();
        store
            .put_envelope(envelope.in_state(EnvelopeState::Acknowledged, long_ago.clone()))
            .unwrap();
    }
    assert!(store.envelopes().all(|e| e.state().is_terminal()));
    set_book(&mut tw, store);
    // Before compaction the book is full and issuing refuses.
    assert!(has_error(
        &tw.run(fx.ann, &format!("+order #{}=go", fx.bob.0))
    ));
    let mut compacted = 0;
    for _ in 0..80 {
        let seen = pass(&mut tw);
        if seen.contains("Compacted") {
            compacted += 1;
        }
        if book(&tw).envelopes().count() + 8 < MAX_ENVELOPES {
            break;
        }
    }
    assert!(compacted > 0, "the pass reported its compaction");
    ok(&mut tw, fx.ann, &format!("+order #{}=go", fx.bob.0));
}

/// **D7(a), the phantom half (K12).** An expired phantom whose Losses have been
/// armed and whose ledger rows are inactive is dropped, with a `Del` in the
/// fold, and no further Loss is armed for it.
#[test]
fn vf_b1b_an_expired_phantom_is_dropped_after_its_losses_are_armed() {
    let (mut tw, fx) = scene();
    ok(
        &mut tw,
        GOD,
        &format!("@prop/phantom #{}=decoy,10", fx.alpha.0),
    );
    let phantom_id = book(&tw)
        .phantoms()
        .next()
        .map(|p| p.id().as_str().to_owned())
        .unwrap();
    let clock = stamp(&tw).clock;
    // The Loss has been applied: the row is inactive.
    install_ledger(
        &mut tw,
        vec![row_at(
            &clock,
            fx.alpha,
            [0xcc; 16],
            false,
            1_000,
            Some(&phantom_id),
        )],
    );
    tw.set_now(1_100);
    let _ = tw.take_module_ops();
    pass(&mut tw);
    assert_eq!(book(&tw).phantoms().count(), 0, "the spent phantom is gone");
    let ops = tw.take_module_ops();
    assert!(
        ops.iter().any(|op| matches!(&op.op,
            ModuleWrite::Del { key } if key.as_str().starts_with("phantom:"))),
        "and the fold carries its Del"
    );
    let mut store = book(&tw);
    let report = detect(&tw, &mut store, ARM);
    assert!(
        !report
            .intents
            .iter()
            .any(|intent| intent.kind() == mush_core::ObservationKind::Loss),
        "no further Loss is re-derived for a dropped phantom: {report:?}"
    );
}

// ---- B3: the delivery pass cursor ------------------------------------

/// **B3.** The delivery pass resumes where it stopped: the union of two passes'
/// jammed lists covers every envelope, including the ones behind the head.
#[test]
fn vf_b3a_the_delivery_pass_resumes_where_it_stopped() {
    let (mut tw, fx) = scene();
    ok(&mut tw, GOD, &format!("@prop/jam #{}=9", fx.orbit.0));
    let at = stamp(&tw);
    let mut store = book(&tw);
    let mut ids = Vec::new();
    for _ in 0..(MAX_ENVELOPES_PER_PASS + 6) {
        let id = store.alloc_envelope().unwrap();
        store
            .put_envelope(
                Envelope::issue(id, fx.ann, fx.bob, "stuck".into(), 1, at.clone()).unwrap(),
            )
            .unwrap();
        ids.push(id);
    }
    set_book(&mut tw, store);
    let state = state(&tw);
    let mut store = book(&tw);
    let ledger = ContactLedgerState::default();
    let first = delivery_pass(&mut store, &state, tw.world(), &ledger, &at, None);
    assert!(first.bound_hit && first.resume.is_some());
    let second = delivery_pass(&mut store, &state, tw.world(), &ledger, &at, None);
    let mut union: Vec<u64> = first.jammed.iter().chain(&second.jammed).copied().collect();
    union.sort_unstable();
    union.dedup();
    for id in &ids {
        assert!(
            union.contains(id),
            "envelope {id} was never visited by either pass — a jammed head starves it"
        );
    }
}

/// **B3.** An envelope behind a permanently jammed head still reaches its TTL.
#[test]
fn vf_b3b_an_envelope_behind_a_jam_still_reaches_its_ttl() {
    let (mut tw, fx) = scene();
    ok(&mut tw, GOD, &format!("@prop/jam #{}=9", fx.orbit.0));
    let now = stamp(&tw);
    let mut store = book(&tw);
    for _ in 0..MAX_ENVELOPES_PER_PASS {
        let id = store.alloc_envelope().unwrap();
        store
            .put_envelope(
                Envelope::issue(id, fx.ann, fx.bob, "stuck".into(), 1, now.clone()).unwrap(),
            )
            .unwrap();
    }
    let old_id = store.alloc_envelope().unwrap();
    let long_ago = ClockStamp {
        clock: now.clock.clone(),
        at: now.at - mush_propagation::ENVELOPE_TTL_SECS - 1,
    };
    store
        .put_envelope(
            Envelope::issue(old_id, fx.ann, fx.bob, "ancient".into(), 1, long_ago).unwrap(),
        )
        .unwrap();
    set_book(&mut tw, store);
    let state = state(&tw);
    let mut store = book(&tw);
    let ledger = ContactLedgerState::default();
    for _ in 0..3 {
        let _ = delivery_pass(&mut store, &state, tw.world(), &ledger, &now, None);
    }
    assert!(
        store
            .envelope(old_id)
            .is_none_or(|envelope| envelope.state() == EnvelopeState::Expired),
        "an envelope past its TTL behind a blocked head must expire"
    );
}

/// **B3.** The production TICK never drops its report: the bound and the
/// anomalies are paged into the store's own queue and the next staff read
/// drains them.
#[test]
fn vf_b3c_the_tick_pages_its_bound_and_its_anomalies() {
    let (mut tw, fx) = scene();
    ok(&mut tw, GOD, &format!("@prop/jam #{}=9", fx.orbit.0));
    let at = stamp(&tw);
    let mut store = book(&tw);
    for _ in 0..(MAX_ENVELOPES_PER_PASS + 4) {
        let id = store.alloc_envelope().unwrap();
        store
            .put_envelope(
                Envelope::issue(id, fx.ann, fx.bob, "stuck".into(), 1, at.clone()).unwrap(),
            )
            .unwrap();
    }
    set_book(&mut tw, store);
    tw.set_now(1_002);
    let _ = tw.fire_due(1_002_000);
    assert!(
        book(&tw)
            .notices()
            .iter()
            .any(|line| line.contains("stopped at its bound")),
        "the tick paged its bound: {:?}",
        book(&tw).notices()
    );
    let seen = pass(&mut tw);
    assert!(
        seen.contains("stopped at its bound"),
        "and staff read it: {seen}"
    );
    assert!(book(&tw).notices().is_empty(), "the queue drains");
}

// ---- the remaining MEDIUMs and LOWs ----------------------------------

/// **K9.** A sealed hull occludes in BOTH directions, and the gate is
/// structural rather than arithmetic.
#[test]
fn vf_k9_a_sealed_hull_occludes_in_both_directions() {
    let (mut tw, fx) = scene();
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/interior #{}=#{}", fx.bravo.0, fx.hull.0),
    );
    let spatial = state(&tw);
    let at = stamp(&tw);
    let inward =
        PropagationContext::build(&spatial, tw.world(), fx.target, fx.bravo, at.clone()).unwrap();
    let outward = PropagationContext::build(&spatial, tw.world(), fx.bravo, fx.target, at).unwrap();
    assert!(
        inward.geometry.occluded,
        "looking INTO the hull is occluded"
    );
    assert!(
        outward.geometry.occluded,
        "and looking OUT of the same sealed hull is too: {outward:?}"
    );
    assert!(
        !resolve_detection(&outward, u32::MAX).is_detected(),
        "the gate is structural, not a matter of the signature constant"
    );
}

/// **Deviation (c), pinned.** The single signature constant is NOT the fog
/// model: an occluded pair and a cross-domain pair both miss at `u32::MAX`, so
/// a later change to `DEFAULT_SIGNATURE` alone can re-open neither gate.
#[test]
fn vf_dev_c_the_signature_constant_is_not_the_fog_model() {
    let (mut tw, fx) = scene();
    let (_deep, far) = far_domain(&mut tw);
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/interior #{}=#{}", fx.bravo.0, fx.hull.0),
    );
    let spatial = state(&tw);
    let at = stamp(&tw);
    assert!(PropagationContext::build(&spatial, tw.world(), fx.alpha, far, at.clone()).is_none());
    let occluded =
        PropagationContext::build(&spatial, tw.world(), fx.target, fx.bravo, at).unwrap();
    assert!(occluded.geometry.occluded);
    assert!(!resolve_detection(&occluded, u32::MAX).is_detected());
}

/// **K6.** A lost phantom-backed contact reacquires when its phantom is still
/// live; a lost REAL contact is observed again once its sighting has aged out,
/// because the leak-safe ledger deliberately carries no object id to reacquire
/// against.
#[test]
fn vf_k6_a_lost_phantom_contact_reacquires_and_a_lost_real_one_is_observed_again() {
    let (mut tw, fx) = scene();
    ok(
        &mut tw,
        GOD,
        &format!("@prop/phantom #{}=decoy,3000", fx.alpha.0),
    );
    let phantom_id = book(&tw)
        .phantoms()
        .next()
        .map(|p| p.id().as_str().to_owned())
        .unwrap();
    let clock = stamp(&tw).clock;
    install_ledger(
        &mut tw,
        vec![row_at(
            &clock,
            fx.alpha,
            [0x55; 16],
            false,
            1_000,
            Some(&phantom_id),
        )],
    );
    let mut store = book(&tw);
    let report = detect(&tw, &mut store, ARM);
    let reacquire = report
        .intents
        .iter()
        .find(|intent| intent.kind() == mush_core::ObservationKind::Reacquire)
        .expect("a lost phantom-backed contact whose phantom is live reacquires");
    assert_eq!(reacquire.observer().platform(), fx.alpha);
    // The real-contact half: the sighting book ages, so a subject standing in
    // the same room is observable again rather than blocked for ever.
    let mut store = book(&tw);
    let names_bravo = |report: &DetectionReport| {
        report.intents.iter().any(|intent| {
            matches!(
                intent.op(),
                mush_core::ObservationOp::Observe { truth: SubjectKey::Object { id }, .. }
                    if *id == fx.bravo
            )
        })
    };
    let first = detect(&tw, &mut store, mush_core::MAX_OBSERVATIONS_PER_PASS);
    assert!(names_bravo(&first));
    let blocked = detect(&tw, &mut store, mush_core::MAX_OBSERVATIONS_PER_PASS);
    assert!(!names_bravo(&blocked));
    tw.set_now(1_000 + mush_propagation::CONTACT_TTL_SECS + 1);
    let again = detect(&tw, &mut store, mush_core::MAX_OBSERVATIONS_PER_PASS);
    assert!(
        names_bravo(&again),
        "an aged sighting no longer blocks re-observation for ever: {again:?}"
    );
}

/// **O7 / deviation (j), discharged.** A minted phantom becomes a contact
/// through the ordinary pass — the phantom→contact path has a producer at last.
#[test]
fn vf_o7_a_minted_phantom_becomes_a_contact_through_the_pass() {
    let (mut tw, fx) = scene();
    ok(
        &mut tw,
        GOD,
        &format!("@prop/phantom #{}=spoof,3000", fx.alpha.0),
    );
    let phantom = book(&tw).phantoms().next().cloned().expect("minted");
    let wanted = mush_core::ObservationOp::Observe {
        truth: phantom.subject(),
        audience: phantom.holders().to_vec(),
    };
    let mut store = book(&tw);
    let report = detect(&tw, &mut store, mush_core::MAX_OBSERVATIONS_PER_PASS);
    let observe = report
        .intents
        .iter()
        .find(|intent| intent.op() == &wanted)
        .expect("the phantom is observed into its holders' picture");
    assert_eq!(observe.observer().platform(), fx.alpha);
    // The subject is scoped, never an object — the no-launder property holds.
    let mush_core::ObservationOp::Observe { truth, .. } = observe.op() else {
        unreachable!()
    };
    let text = serde_json::to_string(truth).unwrap();
    assert!(!text.contains("\"object\""), "{text}");
}

/// **O9.** A shared contact refreshes ONCE, not once per holder.
#[test]
fn vf_o9_a_shared_contact_refreshes_once_not_once_per_holder() {
    let (mut tw, fx) = scene();
    let clock = stamp(&tw).clock;
    let at = stamp(&tw);
    install_ledger(
        &mut tw,
        vec![
            row_at(&clock, fx.alpha, [0x31; 16], true, at.at - 15, None),
            row_at(&clock, fx.bravo, [0x31; 16], true, at.at - 15, None),
        ],
    );
    let mut store = book(&tw);
    let report = detect(&tw, &mut store, ARM);
    let refreshes: Vec<_> = report
        .intents
        .iter()
        .filter(|intent| intent.kind() == mush_core::ObservationKind::Refresh)
        .collect();
    assert_eq!(
        refreshes.len(),
        1,
        "one transition on the shared record, not one per holder: {refreshes:?}"
    );
}

/// **O10.** Two contacts sharing a 32-bit id prefix arm two DISTINCT intents.
#[test]
fn vf_o10_two_contacts_sharing_an_id_prefix_arm_two_distinct_intents() {
    let (mut tw, fx) = scene();
    let clock = stamp(&tw).clock;
    let at = stamp(&tw);
    let mut left = [0x00; 16];
    let mut right = [0x00; 16];
    left[15] = 1;
    right[15] = 2;
    install_ledger(
        &mut tw,
        vec![
            row_at(&clock, fx.alpha, left, true, at.at - 15, None),
            row_at(&clock, fx.alpha, right, true, at.at - 15, None),
        ],
    );
    let mut store = book(&tw);
    let report = detect(&tw, &mut store, ARM);
    let keys: Vec<&str> = report
        .intents
        .iter()
        .filter(|intent| intent.kind() == mush_core::ObservationKind::Refresh)
        .map(|intent| intent.key().as_str())
        .collect();
    assert_eq!(
        keys.len(),
        2,
        "a 32-bit prefix collision silently dropped one contact's transition: {keys:?}"
    );
    assert_ne!(keys[0], keys[1]);
}

/// **O6.** Every holder sees its own contacts past the ledger cap: one holder's
/// volume can no longer starve another's, and a past-TTL contact that sorts
/// after the cut is still lost.
#[test]
fn vf_o6_every_holder_sees_its_own_contacts_past_the_ledger_cap() {
    let (mut tw, fx) = scene();
    let clock = stamp(&tw).clock;
    let at = stamp(&tw);
    let mut rows: Vec<ContactLedgerRow> = Vec::new();
    for index in 0..mush_core::MAX_CONTACT_LEDGER_ROWS {
        let mut id = [0x11; 16];
        id[0] = u8::try_from(index % 251).unwrap();
        id[1] = u8::try_from(index / 251).unwrap();
        rows.push(row_at(&clock, fx.alpha, id, true, at.at, None));
    }
    rows.push(row_at(&clock, fx.ann, [0xfe; 16], true, at.at, None));
    rows.push(row_at(&clock, fx.ann, [0xff; 16], true, at.at - 40, None));
    install_ledger(&mut tw, rows);
    let seen = ok(&mut tw, fx.ann, "+contacts");
    assert!(
        seen.contains("fefefefefefe"),
        "a holder past the cut lost its own picture: {seen}"
    );
    assert!(seen.contains("ffffffffffff"), "{seen}");
    let mut store = book(&tw);
    let report = detect(&tw, &mut store, mush_core::MAX_OBSERVATIONS_PER_PASS);
    assert!(
        report.intents.iter().any(|intent| matches!(
            intent.op(),
            mush_core::ObservationOp::Loss { contact, .. }
                if *contact == ContactId::from_bytes([0xff; 16])
        )),
        "a past-TTL contact that sorts after the cut is still lost: {report:?}"
    );
}

/// **B6.** A pass scoped to one domain names no subject in another, and no
/// observer from another domain.
#[test]
fn vf_b6_a_pass_scoped_to_one_domain_names_no_subject_in_another() {
    let (mut tw, _fx) = scene();
    let (_deep, far) = far_domain(&mut tw);
    let scout = tw
        .world_mut()
        .create(ObjKind::Thing, "Scout", GOD, Some(LIMBO))
        .unwrap();
    bind(&mut tw, scout, "sim.participant");
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/domain #{}=1", scout.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/station #{}=sensors,sensors,channels:movement",
            scout.0
        ),
    );
    let state = state(&tw);
    let at = stamp(&tw);
    let ledger = ContactLedgerState::default();
    let mut store = book(&tw);
    let home = detection_pass(
        &mut store,
        &state,
        tw.world(),
        &ledger,
        &at,
        Some(mush_sim::DomainId(0)),
        mush_core::MAX_OBSERVATIONS_PER_PASS,
    );
    assert!(
        !home.intents.iter().any(|intent| matches!(
            intent.op(),
            mush_core::ObservationOp::Observe { truth: SubjectKey::Object { id }, .. }
                if *id == far || *id == scout
        )),
        "a pass scoped to domain 0 named a subject in domain 1: {home:?}"
    );
    assert!(
        home.intents
            .iter()
            .all(|intent| intent.observer().platform() != scout),
        "and no observer from the other domain"
    );
}

/// **B7.** The held map is bounded, the boot notice names the count, and
/// `@prop/held` is a verb that can actually show a held document.
#[test]
fn vf_b7a_the_held_map_is_bounded_and_a_held_document_is_inspectable() {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for index in 0..(mush_propagation::MAX_HELD_DOCUMENTS + 20) {
        kv.push((
            SmolStr::new(format!("envelope:{index}")),
            serde_json::json!({ "id": index, "not": "an envelope" }),
        ));
    }
    let mut tw = reboot(kv);
    let store = book(&tw);
    assert_eq!(
        store.held().len(),
        mush_propagation::MAX_HELD_DOCUMENTS,
        "the held map is bounded"
    );
    assert_eq!(store.held_overflow(), 20, "and the overflow is counted");
    assert!(
        store
            .notices()
            .iter()
            .any(|line| line.contains("run @prop/held")),
        "the boot pages a verb that can show them: {:?}",
        store.notices()
    );
    let seen = ok(&mut tw, GOD, "@prop/held");
    assert!(seen.contains("envelope:"), "the key is named: {seen}");
    assert!(seen.contains("[envelope]"), "and its kind: {seen}");
    assert!(
        seen.contains("20 further unreadable document(s)"),
        "and the overflow: {seen}"
    );
}

/// **B8.** `@prop` publishes `Authoritative` **because** `/pass` stages the
/// tactics-owned `sensor_arm` — the class and the behaviour asserted together,
/// the shape `@station`'s own pin takes.
#[test]
fn vf_b8_prop_publishes_authoritative_because_pass_stages_sensor_arm() {
    let (mut tw, _fx) = scene();
    let published = tw
        .registry()
        .command_registrations()
        .find(|(spec, owner, _)| spec.name == "@prop" && *owner == Some("propagation"))
        .map(|(_, _, class)| class)
        .expect("@prop is registered");
    assert_eq!(published, mush_core::CommandCompatibility::Authoritative);
    ok(&mut tw, GOD, "@prop/pass");
    let submit = tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("and it really does stage an authoritative action");
    assert_eq!(submit.action.key().owner.as_str(), "tactics");
    assert_eq!(submit.action.key().id.as_str(), "sensor_arm");
}

/// **B10.** A subject-less or time-reversed sighting is refused on load, and a
/// document that will not decode is HELD, never deleted.
#[test]
fn vf_b10_a_subjectless_or_time_reversed_sighting_is_refused_on_load() {
    let subjectless = serde_json::json!({
        "observer": 7, "subject": null, "phantom": null,
        "first": { "clock": { "kind": "utc" }, "at": 1 },
        "last":  { "clock": { "kind": "utc" }, "at": 2 }
    });
    assert!(serde_json::from_value::<Sighting>(subjectless.clone()).is_err());
    let reversed = serde_json::json!({
        "observer": 7, "subject": 9, "phantom": null,
        "first": { "clock": { "kind": "utc" }, "at": 90 },
        "last":  { "clock": { "kind": "utc" }, "at": 1 }
    });
    assert!(serde_json::from_value::<Sighting>(reversed.clone()).is_err());
    let tw = reboot(vec![
        (SmolStr::new("sighting:a"), subjectless),
        (SmolStr::new("sighting:b"), reversed),
    ]);
    let store = book(&tw);
    assert_eq!(store.sightings().count(), 0);
    assert_eq!(store.held().len(), 2, "held, never deleted");
}

/// **B11.** A hydrated phantom set raises the phantom allocator, so a lost
/// allocator document cannot re-mint a live phantom's identity.
#[test]
fn vf_b11_a_hydrated_phantom_set_raises_the_phantom_allocator() {
    let (mut tw, fx) = scene();
    ok(
        &mut tw,
        GOD,
        &format!("@prop/phantom #{}=decoy,300", fx.alpha.0),
    );
    let mut kv = fold_kv(&mut tw);
    // The allocator document is LOST.
    kv.retain(|(key, _)| key.as_str() != mush_propagation::ALLOCATOR_KEY);
    assert!(
        kv.iter()
            .any(|(key, _)| key.as_str().starts_with("phantom:"))
    );
    let mut rebooted = reboot(kv);
    rebooted.set_now(2_000);
    let first = book(&rebooted)
        .phantoms()
        .next()
        .map(|p| p.id().as_str().to_owned())
        .expect("the phantom hydrated");
    // The rebooted world carries the documents, not the fixture's objects, so
    // the second mint names one that exists there.
    ok(
        &mut rebooted,
        GOD,
        &format!("@prop/phantom #{}=spoof,300", GOD.0),
    );
    let ids: Vec<String> = book(&rebooted)
        .phantoms()
        .map(|p| p.id().as_str().to_owned())
        .collect();
    assert_eq!(ids.len(), 2, "two distinct phantoms: {ids:?}");
    assert!(ids.contains(&first), "the live one was not overwritten");
}

/// **B12.** A second pass over an unchanged world writes NO sighting op.
#[test]
fn vf_b12_a_second_pass_with_no_change_writes_no_sighting_op() {
    let (mut tw, _fx) = scene();
    pass(&mut tw);
    let first = tw
        .take_module_ops()
        .iter()
        .filter(
            |op| matches!(&op.op, ModuleWrite::JsonSet { key, .. } if key.starts_with("sighting:")),
        )
        .count();
    assert!(first > 0, "the first pass books what it armed");
    pass(&mut tw);
    let second = tw
        .take_module_ops()
        .iter()
        .filter(
            |op| matches!(&op.op, ModuleWrite::JsonSet { key, .. } if key.starts_with("sighting:")),
        )
        .count();
    assert_eq!(second, 0, "the whole book was re-persisted on every call");
}

/// **B13, recorded not built.** The delivery tick drives DELIVERY only: it is
/// the sole tick propagation registers, and firing it arms no observation.
#[test]
fn vf_b13_the_delivery_tick_drives_delivery_only_and_arms_nothing() {
    let (mut tw, fx) = scene();
    ok(&mut tw, fx.ann, &format!("+order #{}=by tick", fx.bob.0));
    tw.set_now(1_002);
    let _ = tw.fire_due(1_002_000);
    assert_eq!(
        book(&tw).envelopes().next().map(Envelope::state),
        Some(EnvelopeState::Delivered),
        "delivery is driven"
    );
    assert!(
        !tw.engine().has_authoritative_submit(),
        "and detection is NOT: cut 8's pulse owns the detection driver"
    );
    assert_eq!(
        book(&tw).sightings().count(),
        0,
        "the tick books no sighting either"
    );
}

/// **B14.** `+orders` pages rather than printing the whole history.
#[test]
fn vf_b14_plus_orders_pages_rather_than_printing_the_whole_history() {
    let (mut tw, fx) = scene();
    let at = stamp(&tw);
    let mut store = book(&tw);
    for _ in 0..400 {
        let id = store.alloc_envelope().unwrap();
        store
            .put_envelope(
                Envelope::issue(id, fx.ann, fx.bob, "history".into(), 1, at.clone()).unwrap(),
            )
            .unwrap();
    }
    set_book(&mut tw, store);
    let seen = ok(&mut tw, fx.ann, "+orders");
    let rendered = seen.lines().filter(|line| line.contains("hops")).count();
    assert!(rendered <= 32, "the render is bounded: {rendered} lines");
    assert!(
        seen.contains("older record(s) not shown"),
        "and the remainder is named: {seen}"
    );
}

/// **K13.** No persisted key carries an observer→subject truth mapping or an
/// observer's dbref in plaintext.
#[test]
fn vf_k13a_no_sighting_key_or_phantom_id_carries_a_dbref() {
    let (mut tw, fx) = scene();
    ok(
        &mut tw,
        GOD,
        &format!("@prop/phantom #{}=decoy,300", fx.alpha.0),
    );
    pass(&mut tw);
    let kv = fold_kv(&mut tw);
    let subjects = [fx.alpha, fx.bravo, fx.target, fx.ann, fx.bob];
    let mut sightings = 0;
    for (key, _) in &kv {
        if let Some(rest) = key.strip_prefix("sighting:") {
            sightings += 1;
            // A domain-separated digest: 32 hex characters and nothing else, so
            // the key STRUCTURALLY cannot encode an observer/subject pair the
            // way `<observer>/o<subject>` did.
            assert_eq!(rest.len(), 32, "{key}");
            assert!(rest.chars().all(|c| c.is_ascii_hexdigit()), "{key}");
            for near in subjects {
                for far in subjects {
                    assert!(
                        !rest.contains(&format!("{}/o{}", near.0, far.0)),
                        "the key {key} records the observer to subject mapping in plaintext"
                    );
                }
            }
        }
        if let Some(rest) = key.strip_prefix("phantom:") {
            assert!(rest.starts_with("ph"), "{key}");
            assert_eq!(rest.len(), 34, "{key}");
            assert!(rest[2..].chars().all(|c| c.is_ascii_hexdigit()), "{key}");
            for subject in subjects {
                assert!(
                    !rest.starts_with(&format!("{}/", subject.0)),
                    "the phantom id {rest} is derived from a real dbref"
                );
            }
        }
    }
    assert!(sightings > 0, "the pass really did book sightings");
    // Two observers' phantom ids are independent of the object space and of
    // each other.
    ok(
        &mut tw,
        GOD,
        &format!("@prop/phantom #{}=spoof,300", fx.bravo.0),
    );
    let ids: Vec<String> = book(&tw)
        .phantoms()
        .map(|p| p.id().as_str().to_owned())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    for id in &ids {
        for subject in subjects {
            assert!(!id.starts_with(&format!("{}/", subject.0)), "{id}");
        }
    }
}

/// **K15.** A `Provenance::Reported` row names no dbref on any render.
#[test]
fn vf_k15_a_reported_provenance_names_no_dbref() {
    let (mut tw, fx) = scene();
    let clock = stamp(&tw).clock;
    let contact = ContactId::from_bytes([0x5a; 16]);
    install_ledger(
        &mut tw,
        vec![row_at(&clock, fx.ann, [0x5a; 16], true, 1_000, None)],
    );
    let at = stamp(&tw);
    let row = mush_propagation::ContactProvenance::new(
        contact,
        mush_core::CanonicalId::new("radar").unwrap(),
        mush_propagation::Provenance::Reported {
            source: fx.bravo,
            age: 5,
            via: 7,
        },
        1,
        at,
    )
    .unwrap();
    {
        let slot = tw
            .world_mut()
            .module_state_mut::<PropagationStore>()
            .unwrap();
        slot.put_provenance(row).unwrap();
    }
    let seen = ok(&mut tw, fx.ann, "+contacts");
    assert!(
        seen.contains("reported"),
        "the provenance is rendered: {seen}"
    );
    for unit in [fx.alpha, fx.bravo, fx.target] {
        assert!(
            !seen.contains(&format!("#{}", unit.0)),
            "the render leaked #{}: {seen}",
            unit.0
        );
    }
}

/// **B18, pinned.** Every contact renders unattributed and every hop's relay is
/// the issuer, so cut 8's first provenance writer must change it deliberately.
#[test]
fn vf_b18_every_contact_renders_unattributed_and_every_hop_relay_is_the_issuer() {
    let (mut tw, fx) = scene();
    let clock = stamp(&tw).clock;
    install_ledger(
        &mut tw,
        vec![row_at(&clock, fx.ann, [0x1a; 16], true, 1_000, None)],
    );
    let seen = ok(&mut tw, fx.ann, "+contacts");
    assert!(seen.contains("unattributed"), "{seen}");
    let order = deliver(&mut tw, fx.ann, fx.bob, "walk it over");
    let store = book(&tw);
    let envelope = store.envelope(order).unwrap();
    assert!(
        envelope
            .path()
            .iter()
            .all(|hop| hop.relay == envelope.issuer()),
        "every hop's relay is the issuer itself: {envelope:?}"
    );
}

/// **B16, a signature pin.** The delivery pass reads a jamming VIEW, not a
/// clone of the whole store — the type is the assertion.
#[test]
fn vf_b16_the_delivery_pass_takes_a_jamming_view_not_a_store() {
    let (mut tw, fx) = scene();
    ok(&mut tw, GOD, &format!("@prop/jam #{}=9", fx.orbit.0));
    let view: std::collections::BTreeMap<ObjId, u32> = book(&tw).jamming_map();
    assert_eq!(view.len(), 1, "O(jammed places), not O(the whole book)");
    assert_eq!(view.get(&fx.orbit).copied(), Some(9));
}

// ---- O2: the six contact providers check authority -------------------

/// Build one `ActionInvocation` for a tactics contact action, by an actor whose
/// principal is its own object — the shape a projected offer produces.
fn contact_invocation(
    action: &mush_core::DefinitionRef,
    actor: ObjId,
    request: u8,
    parameters: serde_json::Value,
) -> mush_core::ActionInvocation {
    mush_core::ActionInvocation::new(mush_core::ActionInvocationDraft {
        request: mush_core::RequestUid::from_bytes([request; 16]),
        principal: PrincipalId::Object { id: actor },
        actor,
        offered_action: mush_core::ActionOfferId(mush_core::CanonicalHash::from_bytes(
            [request; 32],
        )),
        subject: mush_core::PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters,
        idempotency: mush_core::ScopedIdempotencyKey::new(
            mush_core::IdempotencyScope {
                principal: PrincipalId::Object { id: actor },
                owner: mush_core::OwnerId::new("tactics").unwrap(),
                namespace: mush_core::CanonicalId::new(action.key().id.as_str()).unwrap(),
            },
            1,
            mush_core::IdempotencyKey::Request(mush_core::RequestUid::from_bytes([request; 16])),
        ),
    })
    .unwrap()
}

/// The provider's own refusal code inside the kernel's prepare error.
fn provider_code(error: &mush_core::ActionProviderPrepareError) -> mush_core::ActionErrorCode {
    match error {
        mush_core::ActionProviderPrepareError::Provider { source, .. } => source.code(),
        other => panic!("expected a provider refusal, got {other}"),
    }
}

fn tactics_action(tw: &TestWorld, id: &str) -> mush_core::DefinitionRef {
    tw.registry()
        .definitions()
        .unwrap()
        .current(&mush_core::DefinitionKey {
            owner: mush_core::OwnerId::new("tactics").unwrap(),
            kind: mush_core::DefinitionKind::new("action").unwrap(),
            id: mush_core::DefinitionId::new(id).unwrap(),
        })
        .cloned()
        .unwrap()
}

/// **O2, every arm named.** A `contact_loss`, `contact_refresh`, `contact_share`
/// and `contact_reacquire` by a principal OUTSIDE the audience all refuse
/// `Unauthorized`, an `Observe` credited into another actor's picture refuses,
/// and the legitimate holder's own refresh still commits. Before the fold none
/// of the six compared the invoking principal against the audience at all.
#[test]
fn vf_o2a_loss_refresh_share_reacquire_and_observe_by_a_principal_outside_the_audience_all_refuse()
{
    let (mut tw, fx) = scene();
    let carl = third_player(&mut tw);
    let observe = tactics_action(&tw, "contact_observe");
    // Ann observes Bob into her OWN picture.
    let observe_invocation = contact_invocation(
        &observe,
        fx.ann,
        11,
        serde_json::json!({
            "truth_subject": SubjectKey::Object { id: fx.bob },
            "audience": [PrincipalId::Object { id: fx.ann }],
            "observed_at": { "clock": { "kind": "utc" }, "at": 10 }
        }),
    );
    let (prepared, output) = tw
        .engine_mut()
        .prepare_registered_authoritative_action(
            &observe,
            &observe_invocation,
            mush_core::CommitToken::from_bytes([1; 16]),
        )
        .expect("an observation into one's own picture commits");
    let contact = output.into_parts().0.data()["contact"].clone();
    tw.engine_mut()
        .install_prepared_action(prepared.into_parts().1);
    // Carl is not in the audience, holds no station, and is not the observer.
    for (index, (id, parameters)) in [
        (
            "contact_loss",
            serde_json::json!({
                "contact": contact, "expected_revision": 1,
                "lost_at": { "clock": { "kind": "utc" }, "at": 12 }, "reason": "occluded"
            }),
        ),
        (
            "contact_refresh",
            serde_json::json!({
                "contact": contact, "expected_revision": 1,
                "observed_at": { "clock": { "kind": "utc" }, "at": 12 }
            }),
        ),
        (
            "contact_share",
            serde_json::json!({
                "contact": contact, "expected_revision": 1,
                "principal": PrincipalId::Object { id: carl }
            }),
        ),
        (
            "contact_reacquire",
            serde_json::json!({
                "contact": contact, "expected_revision": 1,
                "observed_at": { "clock": { "kind": "utc" }, "at": 12 }
            }),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let definition = tactics_action(&tw, id);
        let request = 20 + u8::try_from(index).unwrap();
        let error = tw
            .engine_mut()
            .prepare_registered_authoritative_action(
                &definition,
                &contact_invocation(&definition, carl, request, parameters),
                mush_core::CommitToken::from_bytes([request; 16]),
            )
            .expect_err("a stranger's contact transition must refuse");
        assert_eq!(
            provider_code(&error),
            mush_core::ActionErrorCode::Unauthorized,
            "{id} by a stranger must be refused as unauthorized"
        );
    }
    // An observation credited into somebody ELSE's picture refuses too.
    let error = tw
        .engine_mut()
        .prepare_registered_authoritative_action(
            &observe,
            &contact_invocation(
                &observe,
                carl,
                40,
                serde_json::json!({
                    "truth_subject": SubjectKey::Object { id: fx.bob },
                    "audience": [PrincipalId::Object { id: fx.ann }],
                    "observed_at": { "clock": { "kind": "utc" }, "at": 10 }
                }),
            ),
            mush_core::CommitToken::from_bytes([40; 16]),
        )
        .expect_err("an observation into another actor's picture must refuse");
    assert_eq!(
        provider_code(&error),
        mush_core::ActionErrorCode::Unauthorized
    );
    // The holder's own refresh still commits — the gate is not a blanket ban.
    let refresh = tactics_action(&tw, "contact_refresh");
    let (prepared, _) = tw
        .engine_mut()
        .prepare_registered_authoritative_action(
            &refresh,
            &contact_invocation(
                &refresh,
                fx.ann,
                50,
                serde_json::json!({
                    "contact": contact, "expected_revision": 1,
                    "observed_at": { "clock": { "kind": "utc" }, "at": 12 }
                }),
            ),
            mush_core::CommitToken::from_bytes([50; 16]),
        )
        .expect("the holder's own refresh still commits");
    tw.engine_mut()
        .install_prepared_action(prepared.into_parts().1);
}

/// **O2/K16.** A `sensor_arm` that did not come through a projected offer for
/// the acting object refuses **in prepare**, not only in the offer — so the
/// fence is structural rather than compositional.
#[test]
fn vf_o2c_a_sensor_arm_without_a_projected_offer_refuses_in_prepare_not_only_in_the_offer() {
    let (mut tw, fx) = scene();
    let arm = tactics_action(&tw, "sensor_arm");
    let observer = mush_core::ObserverRef::Platform { id: fx.alpha };
    let key = mush_core::ObservationIntent::key_for(
        mush_core::ObservationKind::Observe,
        &observer,
        "o9",
        1,
    )
    .unwrap();
    let intent = mush_core::ObservationIntent::new(
        observer.clone(),
        mush_core::ObservationOp::Observe {
            truth: SubjectKey::Object { id: fx.bravo },
            audience: vec![observer.principal()],
        },
        stamp(&tw),
        key,
    )
    .unwrap();
    let document = serde_json::to_string(&vec![intent]).unwrap();
    let invocation = contact_invocation(
        &arm,
        fx.ann,
        7,
        // No `armed_by`: this invocation did not come from a projected offer.
        serde_json::json!({ "intents": document }),
    );
    let error = tw
        .engine_mut()
        .prepare_registered_authoritative_action(
            &arm,
            &invocation,
            mush_core::CommitToken::from_bytes([3; 16]),
        )
        .expect_err("an arm outside a projected offer must refuse in prepare");
    assert_eq!(
        provider_code(&error),
        mush_core::ActionErrorCode::Unauthorized
    );
}

// ---- O8: the sensor ingress round trip, driven end to end ------------

/// Arm one bounded intent set as staff, exactly as a projected offer would:
/// the `armed_by` fixed parameter is what `project_offer` stamps, and
/// `prepare_authoritative` re-checks it (fold K16).
fn arm(
    tw: &mut TestWorld,
    intents: &[mush_core::ObservationIntent],
    request: u8,
) -> serde_json::Value {
    let action = tactics_action(tw, "sensor_arm");
    let document = serde_json::to_string(intents).unwrap();
    let invocation = contact_invocation(
        &action,
        GOD,
        request,
        serde_json::json!({ "intents": document, "armed_by": GOD }),
    );
    let (prepared, output) = tw
        .engine_mut()
        .prepare_registered_authoritative_action(
            &action,
            &invocation,
            mush_core::CommitToken::from_bytes([request; 16]),
        )
        .expect("a staff arm commits");
    tw.engine_mut()
        .install_prepared_action(prepared.into_parts().1);
    output.into_parts().0.data().clone()
}

/// Discover and fire the one due sensor cursor, returning its outcome document.
fn fire_due_cursor(tw: &mut TestWorld, token: u8) -> Result<serde_json::Value, String> {
    let page = tw
        .engine_mut()
        .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 32)
        .map_err(|error| error.to_string())?;
    let candidate = page
        .candidates()
        .first()
        .cloned()
        .ok_or_else(|| "no due sensor cursor".to_owned())?;
    let ingress =
        mush_core::scheduled_action_ingress(candidate.action(), candidate.advance(), 1).unwrap();
    let plan = tw
        .engine_mut()
        .plan_scheduled_action(&candidate, &ingress)
        .map_err(|error| error.to_string())?;
    let (_, scheduled, _) = plan.into_parts();
    let (prepared, output) = tw
        .engine_mut()
        .prepare_registered_scheduled_authoritative_action(
            &candidate,
            &scheduled,
            mush_core::CommitToken::from_bytes([token; 16]),
        )
        .map_err(|error| error.to_string())?;
    tw.engine_mut()
        .install_prepared_action(prepared.into_parts().1);
    Ok(output.into_parts().0.data().clone())
}

fn observe_intent(tw: &TestWorld, observer: ObjId, subject: ObjId) -> mush_core::ObservationIntent {
    let at = stamp(tw);
    let observer_ref = mush_core::ObserverRef::Platform { id: observer };
    let key = mush_core::ObservationIntent::key_for(
        mush_core::ObservationKind::Observe,
        &observer_ref,
        &format!("o{}", subject.0),
        at.at,
    )
    .unwrap();
    mush_core::ObservationIntent::new(
        observer_ref.clone(),
        mush_core::ObservationOp::Observe {
            truth: SubjectKey::Object { id: subject },
            audience: vec![observer_ref.principal()],
        },
        at,
        key,
    )
    .unwrap()
}

fn refresh_intent(
    tw: &TestWorld,
    observer: ObjId,
    contact: ContactId,
    revision: u64,
) -> mush_core::ObservationIntent {
    let at = stamp(tw);
    let observer_ref = mush_core::ObserverRef::Platform { id: observer };
    let key = mush_core::ObservationIntent::key_for_state(
        mush_core::ObservationKind::Refresh,
        &observer_ref,
        contact,
    )
    .unwrap();
    mush_core::ObservationIntent::new(
        observer_ref,
        mush_core::ObservationOp::Refresh {
            contact,
            expected_revision: revision,
        },
        at,
        key,
    )
    .unwrap()
}

/// **O8, deviation (g) discharged.** The whole ingress runs end to end —
/// `sensor_arm` → `DurableSchedule` → discovery → `SensorScheduleAdapter::plan`
/// → `scheduled_action_ingress` → `ContactProvider` — and the three cases the
/// verify named are asserted together: a duplicate arm at the same tick is a
/// HIT, a re-arm one IC second later **replaces** its standing cursor rather
/// than stacking a second beside it, and an arm naming a contact the observer
/// does not hold **refuses** at the adapter.
///
/// Until this test existed the round trip was asserted nowhere, which is why
/// O2, K4 and O9 all survived the shipped 51-test suite.
#[test]
fn vf_o8_the_sensor_ingress_round_trip_commits_replaces_and_refuses() {
    let (mut tw, fx) = scene();

    // (1) Arm one Observe, fire it, and read the committed contact.
    let observe = observe_intent(&tw, fx.alpha, fx.bravo);
    let armed = arm(&mut tw, &[observe], 1);
    assert_eq!(armed["armed"], serde_json::json!(1));
    let outcome = fire_due_cursor(&mut tw, 2).expect("the armed observation fires and commits");
    assert_eq!(outcome["operation"], serde_json::json!("observed"));
    assert_eq!(outcome["active"], serde_json::json!(true));
    let contact: ContactId = serde_json::from_value(outcome["contact"].clone()).unwrap();

    // (2) A duplicate arm at the SAME tick is a HIT on the same cursor: the
    // fired one is reaped and re-armed rather than joined by a second.
    let refresh = refresh_intent(&tw, fx.alpha, contact, 1);
    let first = arm(&mut tw, std::slice::from_ref(&refresh), 3);
    assert_eq!(first["armed"], serde_json::json!(1));
    let duplicate = arm(&mut tw, std::slice::from_ref(&refresh), 4);
    assert_eq!(
        duplicate["replaced"],
        serde_json::json!(1),
        "a duplicate arm at one deterministic key is a HIT, not a second cursor"
    );

    // (3) A re-arm one IC second later REPLACES the standing cursor, because
    // the key of a state-carrying transition no longer carries the tick.
    tw.set_now(1_001);
    let later = refresh_intent(&tw, fx.alpha, contact, 1);
    assert_eq!(
        later.key().as_str(),
        refresh.key().as_str(),
        "the same due work one IC second later is the same cursor"
    );
    let re_armed = arm(&mut tw, &[later], 5);
    assert_eq!(
        re_armed["replaced"].as_u64().unwrap_or(0) + re_armed["reaped"].as_u64().unwrap_or(0),
        1,
        "the standing cursor was replaced or reaped, never joined: {re_armed}"
    );
    let refreshed = fire_due_cursor(&mut tw, 6).expect("the replaced cursor fires");
    assert_eq!(refreshed["operation"], serde_json::json!("refreshed"));
    assert_eq!(refreshed["revision"], serde_json::json!(2));

    // (4) The negative arm: Bravo does not hold Ann's… Alpha's contact, so the
    // adapter refuses at plan time and nothing commits (fold O2).
    tw.set_now(1_002);
    let stranger = refresh_intent(&tw, fx.bravo, contact, 2);
    let armed = arm(&mut tw, &[stranger], 7);
    assert_eq!(
        armed["armed"],
        serde_json::json!(1),
        "arming is not the gate"
    );
    let refusal = fire_due_cursor(&mut tw, 8)
        .expect_err("a transition on a contact this observer does not hold must refuse");
    assert!(
        refusal.contains("not this observer's to change"),
        "the adapter names the entitlement failure: {refusal}"
    );
}

/// **B9.** The pass fills an arm by **encoded bytes**, so a full one always
/// fits the offer cap — and nothing it trims stays booked.
///
/// 08i answer 15's rule is to MEASURE the bundle rather than assume it fits.
/// The measurement: 16 ordinary `Observe` intents encode to 86 % of the 4 KiB
/// cap, and 16 of the **largest** shape the crate can now produce — a phantom's
/// `SubjectKey::Scoped` truth with a holder audience, which O7's producer added
/// — encode to **over** it, so a fixed count of 16 would have refused the whole
/// batch at `SensorArmProvider::intents` with nothing armed and nothing said.
#[test]
fn vf_b9_the_arm_is_filled_by_bytes_and_a_trimmed_intent_is_not_left_booked() {
    let (mut tw, fx) = scene();
    // The measurement that forces the byte bound.
    let at = stamp(&tw);
    ok(
        &mut tw,
        GOD,
        &format!("@prop/phantom #{}=spoof,3000", fx.alpha.0),
    );
    let phantom = book(&tw).phantoms().next().cloned().expect("minted");
    let mut widest: Vec<mush_core::ObservationIntent> = Vec::new();
    for index in 0..mush_tactics::MAX_INTENTS_PER_ARM {
        let observer = mush_core::ObserverRef::Platform {
            id: ObjId(4_000_000_000 + u32::try_from(index).unwrap()),
        };
        let key = mush_core::ObservationIntent::key_for(
            mush_core::ObservationKind::Observe,
            &observer,
            &format!("f{}", phantom.id().as_str()),
            at.at,
        )
        .unwrap();
        widest.push(
            mush_core::ObservationIntent::new(
                observer.clone(),
                mush_core::ObservationOp::Observe {
                    truth: phantom.subject(),
                    audience: vec![observer.principal()],
                },
                at.clone(),
                key,
            )
            .unwrap(),
        );
    }
    assert!(
        serde_json::to_string(&widest).unwrap().len() > mush_tactics::MAX_SENSOR_ARM_BYTES,
        "a fixed count of {} would not fit the offer cap — which is why the pass counts bytes",
        mush_tactics::MAX_INTENTS_PER_ARM
    );
    // Enough phantoms that the pass wants more than one arm can carry.
    for _ in 0..14 {
        ok(
            &mut tw,
            GOD,
            &format!("@prop/phantom #{}=decoy,3000", fx.alpha.0),
        );
    }
    let mut store = book(&tw);
    let report = detect(&tw, &mut store, ARM);
    let encoded = serde_json::to_string(&report.intents).unwrap();
    assert!(
        encoded.len() <= mush_propagation::MAX_ARM_DOCUMENT_BYTES,
        "the pass emitted an arm document of {} bytes against a cap of {}",
        encoded.len(),
        mush_propagation::MAX_ARM_DOCUMENT_BYTES
    );
    assert!(report.bound_hit, "and it says the bound bound: {report:?}");
    // Nothing trimmed stays booked: the book records exactly what went out, so
    // the remainder is genuinely still due on the next run (the K7 property).
    assert_eq!(
        report.sightings.len(),
        report.intents.len(),
        "a trimmed intent was left booked: {report:?}"
    );
    let armed: Vec<String> = report
        .intents
        .iter()
        .map(|intent| intent.key().as_str().to_owned())
        .collect();
    let second = detect(&tw, &mut store, ARM);
    assert!(
        second
            .intents
            .iter()
            .any(|intent| !armed.contains(&intent.key().as_str().to_owned())),
        "the trimmed remainder is still due on the second run: {second:?}"
    );
}
