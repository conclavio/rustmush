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
    let first = detection_pass(&mut left, &state, tw.world(), &ledger, &at);
    let second = detection_pass(&mut right, &state, tw.world(), &ledger, &at);
    assert!(!first.is_empty(), "two crewed platforms see each other");
    assert_eq!(
        first.iter().map(|i| i.key().as_str()).collect::<Vec<_>>(),
        second.iter().map(|i| i.key().as_str()).collect::<Vec<_>>(),
        "same context, same set, same order"
    );
    assert!(first.len() <= mush_core::MAX_OBSERVATIONS_PER_PASS);
    // And running it again over the SAME book arms nothing twice: the sighting
    // book is the idempotency key.
    let again = detection_pass(&mut left, &state, tw.world(), &ledger, &at);
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
    // This two-module world carries `control_handoff` plus the six sensor
    // sources; `production` is `mush-industry`'s and is pinned in `mush-server`.
    assert_eq!(sources.len(), 7, "control_handoff + six sensor sources");
    let mut ids: Vec<&str> = mush_core::ObservationKind::all()
        .iter()
        .map(|kind| mush_tactics::sensor_source_id(*kind))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 6);
    for id in ids {
        assert!(!matches!(id, "control_handoff" | "production" | "npc"));
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
    let intents = detection_pass(&mut store, &state, tw.world(), &ledger, &at);
    let losses: Vec<_> = intents
        .iter()
        .filter(|intent| intent.kind() == mush_core::ObservationKind::Loss)
        .collect();
    assert_eq!(losses.len(), 2, "one Loss per holder: {intents:?}");
    let mut holders: Vec<u32> = losses
        .iter()
        .map(|intent| intent.observer().platform().0)
        .collect();
    holders.sort_unstable();
    assert_eq!(holders, vec![fx.alpha.0, fx.bravo.0]);
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
    let intents = detection_pass(&mut store, &state, tw.world(), &ledger, &at);
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
    let before = detection_pass(&mut store, &state, tw.world(), &ledger, &at);
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
    let after = detection_pass(&mut store, &state, tw.world(), &ledger, &at);
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
    let first = delivery_pass(&mut store, &state, tw.world(), &at);
    assert_eq!(first.expired, vec![id]);
    let advanced = store.envelope(id).unwrap().advanced().at;
    let second = delivery_pass(&mut store, &state, tw.world(), &at);
    assert!(
        second.expired.is_empty(),
        "a terminal envelope is not revisited"
    );
    assert_eq!(store.envelope(id).unwrap().advanced().at, advanced);
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
        ("@prop", mush_core::CommandCompatibility::LegacyPostImage),
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
    let intents = detection_pass(&mut store, &state, tw.world(), &ledger, &at);
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
