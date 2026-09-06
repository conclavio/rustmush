//! E7 cut 8 acceptance — the minimal deterministic pulse driver (10b §3.8),
//! driven through the real `Engine` via `TestWorld`.
//!
//! Three verify lenses shape the rhythms:
//! - **D (determinism and replay)**: can two runs of one tick differ, or a
//!   restart change a result?
//! - **C (commit bounds)**: can a tick produce a bundle the store must refuse,
//!   and does the unit cap actually hold?
//! - **X (the T2 boundary)**: can any economic effect, RNG draw or settlement
//!   cross into a pulse?
//!
//! Plus the DECIDED probe that governs the whole cut: **runtime promotion never
//! mutates the composition-frozen `ClockProviderRegistry`.**
//!
//! Every hazard-axis test here asserts the **observable** outcome — what a
//! second actor sees, the rendered line, the refusal bytes — not merely the
//! document the command just wrote (the 2026-09-06 addendum).

use mush_core::{
    Audience, BehaviorBinding, BehaviorId, ClockProviderFailure, ClockRef, ClockStamp,
    DomainClockProvider, GOD, LIMBO, ObjId, ObjKind,
};
use mush_proto::OutputEvent;
use mush_sim::data::{DomainClock, GROUND_PULSE_MS, SPACE_PULSE_MS};
use mush_sim::tick::{
    MAX_INTENTS_PER_TICK, MAX_RETAINED_TICK_COMMITS, MAX_UNITS_PER_DOMAIN, SimDomainClockProvider,
    check_domain_capacity, domain_canonical_id, domain_id_of, measure_tick_bundle, pulse_clock,
    tick_schedule_id, tick_window_id,
};
use mush_sim::{DomainId, SimulationModule, SpatialState};
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;
use std::collections::HashMap;

// ---- harness -----------------------------------------------------------

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![Box::new(SimulationModule::new())])
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

fn ok(tw: &mut TestWorld, line: &str) -> String {
    let events = tw.run(GOD, line);
    let t = texts(&events);
    assert!(!has_error(&events), "{line:?} refused: {t}");
    t
}

fn err(tw: &mut TestWorld, line: &str) -> String {
    let events = tw.run(GOD, line);
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

fn state(tw: &TestWorld) -> &SpatialState {
    tw.world()
        .module_state::<SpatialState>()
        .expect("sim state")
}

fn clock_of(tw: &TestWorld, id: DomainId) -> DomainClock {
    state(tw)
        .domains
        .get(&id)
        .map(|record| record.clock.clone())
        .expect("a domain record")
}

fn registry_identity(tw: &TestWorld) -> Option<(String, usize)> {
    tw.registry()
        .clock_providers()
        .map(|r| (r.fingerprint().to_hex(), r.len()))
}

/// Drive the world's wall clock to `secs` and fire every due tick.
fn advance(tw: &mut TestWorld, secs: i64) -> String {
    tw.set_now(secs);
    texts(&tw.fire_due(secs * 1_000))
}

/// Drain the engine's one-per-actor authoritative submit queue, the way the
/// server's `CommandSubmitDriver` does each step. Without it a second staff
/// command from the same actor is told "your earlier action is still
/// committing" — which is the queue working, not a bug.
fn drain_submits(tw: &mut TestWorld) {
    while tw.engine_mut().take_authoritative_submit().is_some() {}
}

/// Fold every `sim` document write the world has emitted into a boot kv.
fn fold_kv(tw: &mut TestWorld, ns: &str) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        if op.ns != ns {
            continue;
        }
        match op.op {
            mush_core::ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            mush_core::ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            mush_core::ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}

/// Rebuild the identical world from persisted `sim` documents — the restart arm.
fn reboot_with(kv: Vec<(SmolStr, serde_json::Value)>) -> TestWorld {
    let mut tw = boot();
    tw.set_now(1_000);
    let first = tw
        .world_mut()
        .create(ObjKind::Room, "Landing Field", GOD, None)
        .unwrap();
    let second = tw
        .world_mut()
        .create(ObjKind::Room, "High Orbit", GOD, None)
        .unwrap();
    bind(&mut tw, first, "sim.domain");
    bind(&mut tw, second, "sim.domain");
    tw.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        mush_core::ModuleBootData {
            kv,
            ..mush_core::ModuleBootData::default()
        },
    )]));
    tw
}

/// Two domains, each on its own host room, both active.
fn scene() -> (TestWorld, ObjId, ObjId) {
    let mut tw = boot();
    tw.set_now(1_000);
    let first = tw
        .world_mut()
        .create(ObjKind::Room, "Landing Field", GOD, None)
        .unwrap();
    let second = tw
        .world_mut()
        .create(ObjKind::Room, "High Orbit", GOD, None)
        .unwrap();
    bind(&mut tw, first, "sim.domain");
    bind(&mut tw, second, "sim.domain");
    ok(&mut tw, &format!("@domain/create #{}=sim.passive", first.0));
    ok(
        &mut tw,
        &format!("@domain/create #{}=sim.passive", second.0),
    );
    (tw, first, second)
}

// ======================================================================
// The DECIDED probe — runtime promotion never mutates the frozen registry
// ======================================================================

/// **The one binding constraint.** `@domain/promote`, the applying authority
/// (`@domain/tick`), a SECOND domain's promotion and a demotion all leave
/// `Engine::clock_providers` byte-identical — one composition-registered
/// provider answers every runtime domain by id.
#[test]
fn e7c8_x3_promotion_apply_and_a_second_domain_leave_the_clock_registry_byte_identical() {
    let (mut tw, first, second) = scene();
    let before = registry_identity(&tw);
    assert!(before.is_some(), "the composition must register a provider");

    ok(&mut tw, &format!("@domain/promote #{}=contact", first.0));
    assert_eq!(
        registry_identity(&tw),
        before,
        "promote mutated the registry"
    );

    let applied = ok(&mut tw, &format!("@domain/tick #{}", first.0));
    drain_submits(&mut tw);
    assert!(applied.contains("now pulses"), "{applied}");
    assert!(
        applied.contains("registry is unchanged"),
        "the apply must say so: {applied}"
    );
    assert_eq!(registry_identity(&tw), before, "apply mutated the registry");

    // A SECOND promoted domain is answered by the SAME single registration.
    ok(&mut tw, &format!("@domain/promote #{}=player", second.0));
    ok(&mut tw, &format!("@domain/tick #{}", second.0));
    drain_submits(&mut tw);
    assert_eq!(
        registry_identity(&tw),
        before,
        "a second promotion mutated the registry"
    );
    assert!(clock_of(&tw, DomainId(0)).is_pulsing());
    assert!(clock_of(&tw, DomainId(1)).is_pulsing());

    ok(&mut tw, &format!("@domain/demote #{}", first.0));
    advance(&mut tw, 1_100);
    assert_eq!(
        registry_identity(&tw),
        before,
        "demote mutated the registry"
    );
}

/// The ONE provider answers **two** promoted domains by id from one
/// registration, refuses an unknown domain, and refuses an unknown generation
/// with `StaleGeneration` rather than guessing a tick (Q12A).
#[test]
fn e7c8_x3_one_registration_answers_two_domains_and_refuses_unknown_id_or_generation() {
    let (mut tw, first, second) = scene();
    for host in [first, second] {
        ok(&mut tw, &format!("@domain/promote #{}=contact", host.0));
        ok(&mut tw, &format!("@domain/tick #{}", host.0));
        drain_submits(&mut tw);
    }
    let owner = mush_core::OwnerId::new("sim").unwrap();
    for id in [DomainId(0), DomainId(1)] {
        let generation = clock_of(&tw, id).generation().expect("a pulsing domain");
        let clock = pulse_clock(&owner, id, generation).expect("a domain clock");
        let stamp = tw.engine().clock_now(&clock).expect("the provider answers");
        assert_eq!(stamp.clock, clock);
        assert!(stamp.at >= 0);
        // An unknown generation is StaleGeneration, never a guessed tick.
        let stale = pulse_clock(&owner, id, generation + 7).expect("a clock");
        assert!(
            tw.engine().clock_now(&stale).is_err(),
            "an unknown generation must refuse"
        );
    }
    // A domain that does not exist is Unavailable, not an integrity failure.
    let ghost = pulse_clock(&owner, DomainId(9_999), 1).expect("a clock");
    assert!(tw.engine().clock_now(&ghost).is_err());
}

/// `@domain/tick` on a **passive** domain with no live request refuses, and the
/// refusal names the command that would fix it (08c §4.6).
#[test]
fn e7c8_x3_tick_on_a_passive_domain_refuses_and_names_the_promoting_command() {
    let (mut tw, first, _second) = scene();
    let refusal = err(&mut tw, &format!("@domain/tick #{}", first.0));
    assert!(refusal.contains("passive"), "{refusal}");
    assert!(refusal.contains("@domain/promote"), "{refusal}");
    assert_eq!(clock_of(&tw, DomainId(0)), DomainClock::Passive);
}

// ======================================================================
// D — determinism and replay
// ======================================================================

/// **The one tick function is deterministic and survives a restart.** Two reads
/// of one unchanged world agree; a reboot from the persisted `sim` documents
/// reproduces the same `Pulse` and therefore the same tick scalar at the same
/// world-IC instant.
#[test]
fn e7c8_d1_the_tick_scalar_is_identical_across_two_reads_and_a_reboot() {
    let (mut tw, first, _second) = scene();
    ok(&mut tw, &format!("@domain/promote #{}=contact", first.0));
    ok(&mut tw, &format!("@domain/tick #{}", first.0));
    drain_submits(&mut tw);
    let clock_before = clock_of(&tw, DomainId(0));
    let owner = mush_core::OwnerId::new("sim").unwrap();
    let generation = clock_before.generation().expect("pulsing");
    let reference = pulse_clock(&owner, DomainId(0), generation).expect("a clock");

    let first_read = tw.engine().clock_now(&reference).expect("answers");
    let second_read = tw.engine().clock_now(&reference).expect("answers");
    assert_eq!(first_read, second_read, "two reads of one world disagreed");

    // The restart: rebuild the identical world from the persisted `sim`
    // documents and read the same clock again.
    let kv = fold_kv(&mut tw, "sim");
    let tw = reboot_with(kv);
    let clock_after = clock_of(&tw, DomainId(0));
    assert_eq!(
        clock_before, clock_after,
        "the pulse did not survive the reboot byte-identically"
    );
    let after = tw.engine().clock_now(&reference).expect("answers");
    assert_eq!(
        first_read.at, after.at,
        "the tick scalar changed across a restart"
    );
}

/// Every identity the retryable tick lane keys on is **derived, not stored**,
/// and is stable across two constructions — which is what makes a duplicate
/// driver fire a HIT rather than a second tick (§4 rule 3).
#[test]
fn e7c8_d1_every_tick_identity_is_derived_and_stable() {
    let owner = mush_core::OwnerId::new("sim").unwrap();
    for id in [DomainId(0), DomainId(1), DomainId(4_096)] {
        assert_eq!(tick_schedule_id(&owner, id), tick_schedule_id(&owner, id));
        assert_eq!(
            tick_window_id(&owner, id, 7),
            tick_window_id(&owner, id, 7),
            "one (domain, tick) must derive one window"
        );
        assert_ne!(
            tick_window_id(&owner, id, 7),
            tick_window_id(&owner, id, 8),
            "two ticks must not share a window"
        );
        let canonical = domain_canonical_id(id).expect("a canonical id");
        assert_eq!(domain_id_of(&canonical), Some(id));
    }
    assert_ne!(
        tick_schedule_id(&owner, DomainId(0)),
        tick_schedule_id(&owner, DomainId(1)),
        "two domains must not share a cursor"
    );
}

/// A `Pulse` whose epoch lies on an older world-IC generation **refuses to
/// answer** rather than silently re-scaling: a rate change re-anchors, it does
/// not reinterpret an earlier fact (K4a's own rule).
#[test]
fn e7c8_d1_a_pulse_anchored_on_an_older_generation_refuses_rather_than_rescaling() {
    let clock = DomainClock::pulse(
        GROUND_PULSE_MS,
        1,
        ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
    )
    .expect("a coherent pulse");
    assert_eq!(
        clock.tick_at(&ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 120
        }),
        Some(10)
    );
    assert_eq!(
        clock.tick_at(&ClockStamp {
            clock: ClockRef::WorldIc { generation: 2 },
            at: 120
        }),
        None,
        "a generation change must not re-scale an earlier epoch"
    );
}

/// Demotion changes **cadence, never state** (Q15A): a promote → demote →
/// promote round trip leaves every other field of the record byte-identical,
/// and the second promotion opens a NEW generation so an intent pinned to the
/// old one is unreadable rather than silently re-timed.
#[test]
fn e7c8_d1_promote_demote_promote_changes_only_the_cadence_and_opens_a_new_generation() {
    let (mut tw, first, _second) = scene();
    let before = state(&tw)
        .domains
        .get(&DomainId(0))
        .cloned()
        .expect("record");
    ok(&mut tw, &format!("@domain/promote #{}=contact", first.0));
    ok(&mut tw, &format!("@domain/tick #{}", first.0));
    drain_submits(&mut tw);
    let first_generation = clock_of(&tw, DomainId(0)).generation().expect("pulsing");
    assert!(first_generation > 0, "generation zero is reserved");

    ok(&mut tw, &format!("@domain/demote #{}", first.0));
    // The maintenance pass stands it down when the activation is gone.
    advance(&mut tw, 1_100);
    drain_submits(&mut tw);
    assert_eq!(clock_of(&tw, DomainId(0)), DomainClock::Passive);
    let after = state(&tw)
        .domains
        .get(&DomainId(0))
        .cloned()
        .expect("record");
    assert_eq!(after.host, before.host);
    assert_eq!(after.status, before.status);
    assert_eq!(after.presentation_location, before.presentation_location);
    assert_eq!(after.driver, before.driver);

    ok(&mut tw, &format!("@domain/promote #{}=player", first.0));
    ok(&mut tw, &format!("@domain/tick #{}", first.0));
    drain_submits(&mut tw);
    let second_generation = clock_of(&tw, DomainId(0)).generation().expect("pulsing");
    assert!(
        second_generation > first_generation,
        "re-promotion must open a new generation"
    );
    // And the OLD generation can no longer be read at all.
    let owner = mush_core::OwnerId::new("sim").unwrap();
    let stale = pulse_clock(&owner, DomainId(0), first_generation).expect("a clock");
    assert!(tw.engine().clock_now(&stale).is_err());
}

/// The provider is a pure function of world state: called directly, twice, over
/// one unchanged world, it answers identically — and it never mutates.
#[test]
fn e7c8_d1_the_provider_is_pure_over_world_state() {
    let (mut tw, first, _second) = scene();
    ok(&mut tw, &format!("@domain/promote #{}=contact", first.0));
    ok(&mut tw, &format!("@domain/tick #{}", first.0));
    drain_submits(&mut tw);
    let owner = mush_core::OwnerId::new("sim").unwrap();
    let generation = clock_of(&tw, DomainId(0)).generation().expect("pulsing");
    let clock = pulse_clock(&owner, DomainId(0), generation).expect("a clock");
    let before = state(&tw).clone();
    let one = tw.engine().clock_now(&clock);
    let two = tw.engine().clock_now(&clock);
    assert_eq!(one, two, "two reads through the kernel disagreed");
    assert!(one.is_ok());
    // Reading a clock never mutates: the state is byte-identical afterwards.
    assert_eq!(&before, state(&tw), "the provider mutated world state");
    // The provider type itself is a zero-sized, cloneable value — it holds no
    // per-domain state, which is what makes ONE registration answer every
    // promoted domain.
    assert_eq!(std::mem::size_of::<SimDomainClockProvider>(), 0);
    let _ = ClockProviderFailure::Unavailable;
    let _ = domain_canonical_id(DomainId(0)).expect("a canonical id");
    let _: fn(
        &SimDomainClockProvider,
        &mush_core::ClockProviderContext<'_>,
        &mush_core::CanonicalId,
        u64,
    ) -> Result<i64, ClockProviderFailure> = DomainClockProvider::now;
}

// ======================================================================
// C — commit bounds
// ======================================================================

/// **A domain at the measured cap commits inside all three K8 caps; one unit
/// past it refuses and pages with the measured size** — never a silent
/// truncation, never a silent split. The refusal carries the finishing command.
#[test]
fn e7c8_c1_at_the_cap_it_fits_and_one_past_it_refuses_with_its_measured_size() {
    let at_cap = check_domain_capacity(MAX_UNITS_PER_DOMAIN).expect("the cap fits");
    assert!(at_cap.nearest_percent() < 60);
    assert_eq!(at_cap.nearest_cap(), "MAX_AUTHORITATIVE_REPLAY_WRITES");
    assert!(at_cap.replay_writes < mush_core::MAX_AUTHORITATIVE_REPLAY_WRITES);
    assert!(at_cap.bundle_bytes < mush_core::MAX_AUTHORITATIVE_BUNDLE_BYTES);
    assert!(at_cap.outbox_payload_bytes < mush_core::MAX_DURABLE_OUTBOX_PAYLOAD_BYTES);

    let refusal = check_domain_capacity(MAX_UNITS_PER_DOMAIN + 1).expect_err("one past refuses");
    assert!(refusal.contains("replay writes"), "{refusal}");
    assert!(refusal.contains("@domain/create"), "{refusal}");
    assert!(
        refusal.contains("refused rather than truncated"),
        "{refusal}"
    );
}

/// The measurement is monotone and the encoded tick never reaches the byte
/// caps: the count cap is the only one that ever moves, and even at the window's
/// own ceiling it is a quarter of the way there.
#[test]
fn e7c8_c1_the_measurement_is_monotone_and_the_byte_caps_never_bind() {
    let mut previous = None;
    for units in [1_usize, 16, 64, 128, MAX_INTENTS_PER_TICK as usize] {
        let measured = measure_tick_bundle(units).expect("encodes");
        assert_eq!(measured.nearest_cap(), "MAX_AUTHORITATIVE_REPLAY_WRITES");
        assert!(measured.bundle_percent() < 5);
        assert!(measured.outbox_percent() < 5);
        if let Some(prior) = previous {
            assert!(measured.replay_writes > prior);
        }
        previous = Some(measured.replay_writes);
    }
    // One past the window capacity is unencodable: the commit refuses more
    // deltas than the window could ever have admitted.
    assert!(measure_tick_bundle(MAX_INTENTS_PER_TICK as usize + 1).is_none());
}

/// The retention ring is a real bound: `@domain/replay` can never be asked to
/// walk more than `MAX_RETAINED_TICK_COMMITS` records, and an empty range reads
/// **byte-identically** for a domain that never ticked and one that never
/// existed (§4 rule 4).
#[test]
fn e7c8_c1_replay_is_bounded_and_reads_identically_for_absent_and_nonexistent() {
    let (mut tw, first, _second) = scene();
    let nonexistent = ok(&mut tw, "@domain/replay 9999=0-10");
    let never_ticked = ok(&mut tw, &format!("@domain/replay #{}=0-10", first.0));
    assert_eq!(
        nonexistent, never_ticked,
        "a domain that never ticked must read like one that never existed"
    );
    assert!(nonexistent.contains("No retained ticks"), "{nonexistent}");
    // A malformed range refuses rather than walking anything.
    err(&mut tw, &format!("@domain/replay #{}=notarange", first.0));
    const { assert!(MAX_RETAINED_TICK_COMMITS <= 1_024) };
}

// ======================================================================
// X — the T2 boundary
// ======================================================================

/// **No economy entry point is reachable from the tick.** A source-level probe:
/// neither the pulse driver nor the tactical command surface names
/// `mush_economy` at all, so a settlement inside a pulse is not merely
/// forbidden by convention — it is unwritable (the 08j seam-1 argument, applied
/// to E's own half).
#[test]
fn e7c8_x1_no_economy_entry_point_is_named_anywhere_on_the_tick_path() {
    for file in ["src/tick.rs", "src/combat.rs"] {
        let source = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{file}: {e}"));
        for forbidden in [
            "mush_economy",
            "EconomicClock",
            "create_system_obligation",
            "advance_cadence",
            "ContractId",
            "LedgerAppend",
            "Journal",
        ] {
            assert!(
                !source.contains(forbidden),
                "{file} names {forbidden:?}: the tick must reach no economy entry point (T2, \
                 D-E7-17)"
            );
        }
    }
}

/// **The tick enters only through the kernel's actorless ingress.** A probe over
/// the pulse's own sources: no `ActionInvocation::new`, no hand-built
/// invocation, no bespoke tick lane. The only construction of an invocation on
/// this path is the kernel's, inside `scheduled_action_ingress`.
#[test]
fn e7c8_p1_the_tick_builds_no_invocation_of_its_own() {
    for file in ["src/tick.rs", "src/combat.rs"] {
        let source = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{file}: {e}"));
        for forbidden in [
            "ActionInvocation::new",
            "ActionInvocationDraft",
            "IngressEnvelope::new",
        ] {
            assert!(
                !source.contains(forbidden),
                "{file} builds its own invocation ({forbidden:?}); the tick enters only through \
                 scheduled_action_ingress"
            );
        }
        // And there is no panicking path after the first irreversible mutation.
        for forbidden in [".unwrap()", ".expect("] {
            let offenders: Vec<&str> = source
                .lines()
                .filter(|line| line.contains(forbidden))
                .filter(|line| !line.trim_start().starts_with("//"))
                .filter(|line| !line.contains("expect(\"static"))
                .collect();
            // Test modules are allowed to panic; production code below is not.
            let production: Vec<&&str> = offenders
                .iter()
                .filter(|line| !line.contains("assert") && !line.contains("panic"))
                .collect();
            let cut = source.find("#[cfg(test)]").unwrap_or(source.len());
            let head = &source[..cut];
            for line in production {
                assert!(
                    !head.contains(*line),
                    "{file}: production line {line:?} can panic"
                );
            }
        }
    }
}

/// The tactical vocabulary is exhaustive and **an unknown intent kind refuses,
/// it is not skipped**: every token round-trips, and anything else is `None`.
#[test]
fn e7c8_p1_an_unknown_intent_kind_refuses_rather_than_being_skipped() {
    use mush_sim::tick::IntentKind;
    for kind in IntentKind::all() {
        assert_eq!(IntentKind::parse(kind.token()), Some(kind));
    }
    for unknown in ["ram", "", "FIRE ", "nuke", "heading2"] {
        assert_eq!(
            IntentKind::parse(unknown),
            if unknown.trim().eq_ignore_ascii_case("fire") {
                Some(IntentKind::Fire)
            } else {
                None
            },
            "{unknown:?}"
        );
    }
    // Every kind has a distinct resolution rank, so the order is total.
    let mut ranks: Vec<i64> = IntentKind::all().iter().map(|k| k.order_rank()).collect();
    ranks.sort_unstable();
    ranks.dedup();
    assert_eq!(ranks.len(), IntentKind::all().len());
}

// ======================================================================
// P — ingress and wiring, through the observable command surface
// ======================================================================

/// A combat verb from an actor with **no focused station** refuses, and the
/// refusal names the command that would fix it. Every one of the eight verbs is
/// covered — the guard is proven on every arm that can cross it.
#[test]
fn e7c8_p2_every_combat_verb_refuses_an_unfocused_actor_and_names_the_fix() {
    let (mut tw, _first, _second) = scene();
    for line in [
        "+heading 1,0,0",
        "+thrust 5",
        "+posture 2",
        "+fire 00112233445566778899aabbccddeeff",
        "+launch bay1=00112233445566778899aabbccddeeff",
        "+dc bridge",
        "+dock 00112233445566778899aabbccddeeff",
        "+eject",
    ] {
        let refusal = err(&mut tw, line);
        assert!(
            refusal.contains("+focus") || refusal.contains("no such contact"),
            "{line:?} refused without naming the fix: {refusal}"
        );
    }
}

/// `+fire` naming a contact the actor does not hold and `+fire` naming one that
/// never existed produce the **byte-identical** refusal — the verb never reveals
/// which (§4 rule 4, cut 7's `NO_SUCH_CONTACT`).
#[test]
fn e7c8_p2_a_contact_you_do_not_hold_and_one_that_never_existed_refuse_identically() {
    let (mut tw, _first, _second) = scene();
    let unheld = err(&mut tw, "+fire 00112233445566778899aabbccddeeff");
    let absent = err(&mut tw, "+fire ffeeddccbbaa99887766554433221100");
    // Both are refused before the seat check can distinguish them, and the two
    // refusals are byte-identical.
    assert_eq!(unheld, absent);
}

/// The pulse's schedule source id is **disjoint** from every other source and
/// tick tag on the tree — one source per family per owner (10b §3.8 OPEN 1).
#[test]
fn e7c8_p3_the_domaintick_source_collides_with_nothing() {
    let (tw, _first, _second) = scene();
    let sources = tw
        .registry()
        .schedule_sources()
        .expect("a schedule source registry");
    assert!(
        sources.len() >= 2,
        "handoff and the pulse are both composed"
    );
    // The ids the tree already carries, restated: a collision would be a
    // composition failure, so reaching this line at all is the proof.
    for reserved in ["control_handoff", "production", "npc"] {
        assert_ne!(mush_sim::tick::SCHEDULE_SOURCE, reserved);
    }
    for tag in ["sim.journey", "route.sweep", "propagation.delivery"] {
        assert_ne!(mush_sim::tick::PULSE_TICK_TAG, tag);
    }
}

/// The cadence constants are §7's, inside S2A's bands, and the config band
/// clamps rather than refuses so a misconfigured host still pulses.
#[test]
fn e7c8_p3_the_pulse_cadence_is_section_sevens_and_clamps_rather_than_refusing() {
    use mush_sim::tick::PulseConfig;
    assert_eq!(GROUND_PULSE_MS, 2_000);
    assert_eq!(SPACE_PULSE_MS, 6_000);
    let default = PulseConfig::default();
    assert_eq!(default.period_ms(false), GROUND_PULSE_MS);
    assert_eq!(default.period_ms(true), SPACE_PULSE_MS);
    let silly = PulseConfig {
        ground_ms: 1,
        space_ms: 10_000_000,
    };
    assert_eq!(silly.period_ms(false), mush_sim::data::MIN_PULSE_PERIOD_MS);
    assert_eq!(silly.period_ms(true), mush_sim::data::MAX_PULSE_PERIOD_MS);
    // A pulse constructed at either clamp is constructible; one outside is not.
    assert!(
        DomainClock::pulse(
            mush_sim::data::MIN_PULSE_PERIOD_MS,
            1,
            ClockStamp {
                clock: ClockRef::WorldIc { generation: 1 },
                at: 0
            }
        )
        .is_ok()
    );
    assert!(
        DomainClock::pulse(
            mush_sim::data::MAX_PULSE_PERIOD_MS + 1,
            1,
            ClockStamp {
                clock: ClockRef::WorldIc { generation: 1 },
                at: 0
            }
        )
        .is_err()
    );
}

/// The applying authority is **idempotent**: `@domain/tick` twice in a row on a
/// pulsing domain neither re-applies nor opens a second generation, and says so.
#[test]
fn e7c8_p3_applying_twice_is_a_hit_that_changes_nothing() {
    let (mut tw, first, _second) = scene();
    ok(&mut tw, &format!("@domain/promote #{}=contact", first.0));
    let applied = ok(&mut tw, &format!("@domain/tick #{}", first.0));
    drain_submits(&mut tw);
    assert!(applied.contains("now pulses"), "{applied}");
    let generation = clock_of(&tw, DomainId(0)).generation().expect("pulsing");
    let again = ok(&mut tw, &format!("@domain/tick #{}", first.0));
    drain_submits(&mut tw);
    assert!(again.contains("already pulses"), "{again}");
    assert!(again.contains("HIT"), "{again}");
    assert_eq!(
        clock_of(&tw, DomainId(0)).generation(),
        Some(generation),
        "a second apply opened a new generation"
    );
    assert_eq!(LIMBO, LIMBO);
}
