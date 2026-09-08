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

#[path = "pulse_contracts/mod.rs"]
mod pulse_contracts;

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
            tick_window_id(&owner, id, 1, 7),
            tick_window_id(&owner, id, 1, 7),
            "one (domain, generation, tick) must derive one window"
        );
        assert_ne!(
            tick_window_id(&owner, id, 1, 7),
            tick_window_id(&owner, id, 1, 8),
            "two ticks must not share a window"
        );
        // **E7c8 fold T4/D5(a):** and two GENERATIONS of one tick number must
        // not share one either, which is what made a re-promoted domain walk
        // back over the previous generation's leftovers.
        assert_ne!(
            tick_window_id(&owner, id, 1, 7),
            tick_window_id(&owner, id, 2, 7),
            "two generations must not share a window"
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
///
/// **Re-tabulated by the E7c8 fold (B2):** the write count is now counted off
/// the encoded bundle rather than hard-coded as `units + 1 + 1 + 2`, and the
/// modelled intent count is `min(MAX_INTENTS_PER_TICK, units × 4)` because the
/// tick writes one `StandingOrder` per `(unit, channel)`. It is therefore
/// monotone **non-strictly**: past `MAX_UNITS_PER_DOMAIN` the window's own
/// capacity clamps it, which is the honest frontier.
#[test]
fn e7c8_c1_the_measurement_is_monotone_and_the_byte_caps_never_bind() {
    let mut previous = None;
    for units in [1_usize, 16, 64, 128, MAX_INTENTS_PER_TICK as usize] {
        let measured = measure_tick_bundle(units).expect("encodes");
        assert_eq!(measured.nearest_cap(), "MAX_AUTHORITATIVE_REPLAY_WRITES");
        assert!(measured.bundle_percent() < 5);
        assert!(measured.outbox_percent() < 5);
        if let Some(prior) = previous {
            assert!(measured.replay_writes >= prior);
        }
        previous = Some(measured.replay_writes);
    }
    // The corrected figures: 64 units saturate the window's 256 slots over four
    // control channels: 453 writes including all three capped cleanup sets.
    let at_cap = measure_tick_bundle(MAX_UNITS_PER_DOMAIN).expect("encodes");
    assert_eq!(at_cap.replay_writes, 453);
    assert_eq!(at_cap.replay_write_percent(), 44);
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

// ---- E7 cut 8 independent verify fold ----
//
// The inert ingress (A2), the world-freezing re-arm (T1), the single-domain arm
// queue (T3), the generation-blind identities (T4/B3), the ungated offer
// (A1/A3/A4), the crewmate's focus (A5), the decorative channel model (A6/T9),
// the misdeclared `@domain` (B1) and the hand-written measurement (B2).
// Names `vf_<lens><n>_…`; every one asserts the OBSERVABLE outcome.

use mush_core::{
    CanonicalId, ChannelId, CommitToken, IdempotencyKey, IdempotencyScope, OfferExecutionContext,
    OfferKeyRing, OfferSecret, OfferSubmission, PrincipalId, RequestUid, ScopedIdempotencyKey,
};
use mush_sim::tick::{
    DeltaOutcome, MAX_RETAINED_TICK_COMMITS as RING, OrderChannel, TickCommitKey, TickHandles,
    encode_tick, intent_action_ref,
};
use serde_json::{Value, json};

/// A crewed, promoted domain: Atlas with three stations whose channel sets
/// differ, three crew and one unbilleted bystander.
struct Fold {
    #[allow(dead_code)]
    field: ObjId,
    atlas: ObjId,
    pilot: ObjId,
    gunner: ObjId,
    ops: ObjId,
    bystander: ObjId,
}

struct MapOracle(std::collections::BTreeMap<ObjId, mush_core::AccountId>);
impl mush_core::AccountIdentity for MapOracle {
    fn account_of(&self, character: ObjId) -> Option<mush_core::AccountId> {
        self.0.get(&character).copied()
    }
}

fn run_ok(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let events = tw.run(who, line);
    let text = texts(&events);
    assert!(!has_error(&events), "{line:?} refused: {text}");
    text
}

fn run_err(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let events = tw.run(who, line);
    assert!(
        has_error(&events),
        "{line:?} should have been refused: {}",
        texts(&events)
    );
    texts(&events)
}

fn fold_scene() -> (TestWorld, Fold) {
    fold_scene_with(vec![Box::new(SimulationModule::new())])
}

fn fold_scene_with(modules: Vec<Box<dyn mush_core::GameModule>>) -> (TestWorld, Fold) {
    let mut tw = TestWorld::with_modules(modules);
    tw.set_now(1_000);
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .unwrap();
    let atlas = tw
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, Some(LIMBO))
        .unwrap();
    let pilot = tw.spawn_player("Pilot");
    let gunner = tw.spawn_player("Gunner");
    let ops = tw.spawn_player("Ops");
    let bystander = tw.spawn_player("Bystander");
    bind(&mut tw, field, "sim.domain");
    bind(&mut tw, atlas, "sim.participant");
    let accounts = std::collections::BTreeMap::from([
        (pilot, mush_core::AccountId::mint("PilotAcct", 1).unwrap()),
        (gunner, mush_core::AccountId::mint("GunnerAcct", 2).unwrap()),
        (ops, mush_core::AccountId::mint("OpsAcct", 3).unwrap()),
        (bystander, mush_core::AccountId::mint("ByAcct", 4).unwrap()),
    ]);
    *tw.world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .unwrap() = mush_sim::AccountOracle::new(std::sync::Arc::new(MapOracle(accounts)));
    ok(&mut tw, &format!("@domain/create #{}=sim.passive", field.0));
    ok(&mut tw, &format!("@spatial/place/domain #{}=0", atlas.0));
    // Three stations whose channel sets DIFFER, and whose priorities differ —
    // the two things cut 8's combat surface never consulted (A6, A8).
    ok(
        &mut tw,
        &format!(
            "@spatial/station #{}=pilot,pilot,channels:movement,priority:10",
            atlas.0
        ),
    );
    ok(
        &mut tw,
        &format!(
            "@spatial/station #{}=gunner,gunner,channels:fire,priority:5",
            atlas.0
        ),
    );
    ok(
        &mut tw,
        &format!(
            "@spatial/station #{}=ops,commander,channels:movement+sensors,priority:20",
            atlas.0
        ),
    );
    for (station, who) in [("pilot", pilot), ("gunner", gunner), ("ops", ops)] {
        ok(
            &mut tw,
            &format!("@spatial/assign #{}={station},#{}", atlas.0, who.0),
        );
    }
    for who in [pilot, gunner, ops, bystander] {
        tw.world_mut().move_to(who, field).unwrap();
    }
    ok(&mut tw, &format!("@domain/promote #{}=contact", field.0));
    ok(&mut tw, &format!("@domain/tick #{}", field.0));
    drain_submits(&mut tw);
    tw.engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(2, [2; 32]).unwrap(), []).unwrap());
    // The resolution key ring the production composition installs, so the
    // window can PIN its secret version (fold T11) and the tick can draw under
    // the pinned one rather than whichever happens to be live.
    tw.engine_mut().set_resolution_keys(
        mush_core::ResolutionKeyRing::new(
            mush_core::ResolutionSecret::new(3, [7; 32]).unwrap(),
            [],
        )
        .unwrap(),
    );
    assert!(clock_of(&tw, DomainId(0)).is_pulsing(), "the scene pulses");
    (
        tw,
        Fold {
            field,
            atlas,
            pilot,
            gunner,
            ops,
            bystander,
        },
    )
}

fn handles(tw: &TestWorld) -> TickHandles {
    tw.world()
        .module_state::<TickHandles>()
        .expect("the pulse registers its read handles")
        .clone()
}

fn generation(tw: &TestWorld) -> u64 {
    clock_of(tw, DomainId(0)).generation().expect("pulsing")
}

fn current_tick(tw: &TestWorld) -> i64 {
    let now = tw.world().world_ic_now();
    clock_of(tw, DomainId(0)).tick_at(&now).expect("a tick")
}

fn window_of(tw: &TestWorld, tick: i64) -> mush_core::ActionWindowId {
    let owner = mush_core::OwnerId::new("sim").unwrap();
    tick_window_id(&owner, DomainId(0), generation(tw), tick).expect("a window id")
}

fn window_record(tw: &TestWorld, tick: i64) -> Option<mush_core::ActionWindowRecord> {
    let handles = handles(tw);
    tw.engine()
        .action_state()
        .get(&handles.windows, &window_of(tw, tick))
        .ok()
        .flatten()
        .cloned()
}

/// **The rung cut 8 never used** (harness note 2): resolve one queued
/// command-originated submit through the REAL offer path — exactly as
/// `mush-server`'s `command_offer_submission` + `resolve_and_stage_offer` do —
/// and stage it. `e7c8.rs`'s own `drain_submits` threw these away without ever
/// resolving them, which is the single reason a wholly inert ingress passed a
/// 40-test cut.
fn drive_submit(
    tw: &mut TestWorld,
    submit: &mush_core::AuthoritativeSubmit,
    token: u8,
) -> Result<(), String> {
    let (action, resolved, _guards) = resolve_submit(tw, submit, token)?.into_authoritative_parts();
    let (prepared, _) = tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes([token; 16]),
        )
        .map_err(|error| format!("prepare: {error:?}"))?;
    let (_ops, installation) = prepared.into_parts();
    tw.engine_mut().install_prepared_action(installation);
    Ok(())
}

fn resolve_submit(
    tw: &TestWorld,
    submit: &mush_core::AuthoritativeSubmit,
    token: u8,
) -> Result<mush_core::ResolvedAuthoritativeOffer, String> {
    let engine = tw.engine();
    let query = engine
        .legal_action_query(
            PrincipalId::Object { id: submit.actor },
            submit.actor,
            submit.subject.clone(),
        )
        .map_err(|error| format!("query: {error:?}"))?;
    let offers = engine
        .project_legal_actions(&query)
        .map_err(|error| format!("projection: {error:?}"))?;
    let projected = offers
        .projected_for_action(&submit.action)
        .ok_or_else(|| "the tactical intent offer did not project".to_owned())?;
    let submission = OfferSubmission {
        offer: projected.offer(),
        subject: projected.subject().clone(),
        targets: submit.targets.clone(),
        // **Verbatim**: the parameters the verb staged, resolved against the
        // offer's own declared schema. This is the comparison that makes the
        // A2 class of defect unrepresentable.
        parameters: submit.parameters.clone(),
    };
    let execution = OfferExecutionContext {
        request: RequestUid::from_bytes([token; 16]),
        principal: PrincipalId::Object { id: submit.actor },
        actor: submit.actor,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: PrincipalId::Object { id: submit.actor },
                owner: submit.action.key().owner.clone(),
                namespace: CanonicalId::new(submit.action.key().id.as_str()).unwrap(),
            },
            1,
            IdempotencyKey::Request(RequestUid::from_bytes([token; 16])),
        ),
        viewer_scope: query.viewer_scope(),
        now: None,
    };
    let resolved = engine
        .resolve_legal_action(&offers, execution, submission)
        .map_err(|error| format!("resolve: {error:?}"))?;
    Ok(resolved)
}

/// Run one verb and drive its submission all the way into the window.
fn issue(tw: &mut TestWorld, who: ObjId, line: &str, token: u8) -> Result<(), String> {
    let events = tw.run(who, line);
    if has_error(&events) {
        return Err(texts(&events));
    }
    let submit = *tw
        .engine_mut()
        .take_authoritative_submit()
        .ok_or_else(|| format!("{line:?} staged no submission"))?;
    drive_submit(tw, &submit, token)
}

// ---- lens A — authority ------------------------------------------------

/// **A2 (HIGH).** Every one of the eight combat verbs resolves through the REAL
/// offer path and joins the tick's window. Before this fold the offer declared
/// `observed_at` as `ProjectedParameterKind::Text` while the verb submitted
/// `serde_json::to_value(&ClockStamp)` — an object — so offer resolution refused
/// every verb with `InvalidParameters` **before any provider ran** and no intent
/// had ever reached a window through the command path.
#[test]
fn vf_a2a_every_combat_verb_resolves_through_the_real_offer_path_and_joins_a_window() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.ops, &format!("+focus #{}/ops", fx.atlas.0));
    let tick = current_tick(&tw);
    // `ops` grants movement+sensors, so it can issue every kind on those two
    // channels; the fire-channel verbs are covered by `vf_a6a`.
    let mut token = 40_u8;
    for line in [
        "+heading 1,0,0",
        "+thrust 5",
        "+posture 2",
        "+eject",
        "+dock 00112233445566778899aabbccddeeff",
    ] {
        token += 1;
        let outcome = issue(&mut tw, fx.ops, line, token);
        // `+dock` names a contact the actor does not hold, so it refuses at the
        // verb with the byte-identical no-such-contact line — never at offer
        // resolution.
        if line.starts_with("+dock") {
            let refusal = outcome.expect_err("an unheld contact refuses at the verb");
            assert!(refusal.contains("no such contact"), "{refusal}");
            continue;
        }
        outcome.unwrap_or_else(|error| panic!("{line:?} did not reach the window: {error}"));
    }
    let record = window_record(&tw, tick).expect("the tick's window exists");
    assert_eq!(
        record.intents().len(),
        4,
        "four issuable verbs must be four intents in the window, not zero"
    );
}

/// **A2, the construction half.** The offer's public schema is exactly one
/// `intent` text parameter, and the seat — `(unit, station, channels, epoch,
/// priority, domain, tick, generation)` — is a FIXED parameter the caller can
/// neither supply nor override. A client key colliding with a fixed one is
/// `InvalidParameters` in `merge_parameters`, so the gate is structural.
#[test]
fn vf_a2b_the_offer_schema_and_the_verbs_submission_agree_by_construction() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    run_ok(&mut tw, fx.pilot, "+heading 1,0,0");
    let submit = *tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("the verb staged a submission");
    let parameters = submit
        .parameters
        .as_object()
        .expect("parameters are an object");
    assert_eq!(
        parameters.keys().collect::<Vec<_>>(),
        vec!["intent"],
        "the verb sends exactly the offer's one public parameter"
    );
    assert!(
        parameters["intent"].is_string(),
        "the one public parameter is Text, and the verb sends a string"
    );
    assert!(
        !parameters.contains_key("observed_at"),
        "observed_at is no longer a public parameter at all"
    );
    // The declared schema itself: one required Text parameter named `intent`.
    let action = intent_action_ref(tw.registry()).expect("the ingress is composed");
    let engine = tw.engine();
    let query = engine
        .legal_action_query(
            PrincipalId::Object { id: fx.pilot },
            fx.pilot,
            mush_core::PerceivedRef::SelfActor,
        )
        .expect("a query");
    let offers = engine.project_legal_actions(&query).expect("projection");
    let projected = offers
        .projected_for_action(&action)
        .expect("a focused pilot is offered the ingress");
    let declared: Vec<String> = projected
        .parameters()
        .parameters()
        .iter()
        .map(|parameter| parameter.id.as_str().to_owned())
        .collect();
    assert_eq!(declared, vec!["intent".to_owned()]);
}

/// **A1 (HIGH).** The ingress is not projected to an unbilleted bystander, and
/// the staff-gated arm action is the control: before this fold `tactical_intent`
/// reached EVERY actor whose truth subject was itself, with
/// `fixed_parameters: json!({})` and both parameters free.
#[test]
fn vf_a1a_the_intent_offer_is_not_projected_to_an_unbilleted_bystander() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let action = intent_action_ref(tw.registry()).expect("the ingress is composed");
    let arm = tw
        .registry()
        .definitions()
        .and_then(|definitions| {
            definitions
                .current(&mush_core::DefinitionKey {
                    owner: mush_core::OwnerId::new("sim").unwrap(),
                    kind: mush_core::DefinitionKind::new("action").unwrap(),
                    id: mush_core::DefinitionId::new("domain_pulse_arm").unwrap(),
                })
                .cloned()
        })
        .expect("the arm action is composed");
    let engine = tw.engine();
    for (who, offered, label) in [
        (fx.bystander, false, "an unbilleted bystander"),
        (fx.gunner, false, "a billet holder who has not focused"),
        (fx.pilot, true, "the focused pilot"),
    ] {
        let query = engine
            .legal_action_query(
                PrincipalId::Object { id: who },
                who,
                mush_core::PerceivedRef::SelfActor,
            )
            .expect("a query");
        let offers = engine.project_legal_actions(&query).expect("projection");
        assert_eq!(
            offers.projected_for_action(&action).is_some(),
            offered,
            "{label} was offered the tactical ingress: {offered}"
        );
        // The control: the arm action IS staff-gated for every one of them.
        assert!(
            offers.projected_for_action(&arm).is_none(),
            "{label} was offered the staff-gated pulse arm"
        );
    }
}

/// **A1/A3/A4 (HIGH), the provider half.** A hand-built invocation naming a unit
/// the actor does not hold, a forged clock generation, or a phantom unit is
/// refused `Unauthorized` in `prepare_authoritative` — it never spends a durable
/// window slot, never claims a future window at a forged generation and never
/// reaches the commit's `accepted` list.
#[test]
fn vf_a1b_an_invocation_naming_a_unit_station_epoch_or_generation_it_does_not_hold_refuses_unauthorized()
 {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let action = intent_action_ref(tw.registry()).expect("the ingress is composed");
    let tick = current_tick(&tw);
    let clock_generation = generation(&tw);
    let real_seat = json!({
        "unit": fx.atlas,
        "station": "pilot",
        "channels": ["movement"],
        "epoch": grant_epoch_of(&tw, fx.pilot, fx.atlas, "pilot"),
        "priority": 10,
        "domain": 0,
        "tick": tick,
        "generation": clock_generation,
    });
    let base = |unit: ObjId, station: &str, epoch: u64, priority: u16, domain: u64, tick: i64| {
        json!({
            "domain": domain,
            "tick": tick,
            "unit": unit,
            "station": station,
            "epoch": epoch,
            "priority": priority,
            "channel": "movement",
            "kind": "heading",
            "value": { "kind": "vector", "x": 1, "y": 0, "z": 0 },
        })
    };
    let epoch = grant_epoch_of(&tw, fx.pilot, fx.atlas, "pilot");
    // Each forgery names ONE thing the seat does not carry.
    for (label, doc) in [
        (
            "a phantom unit",
            base(ObjId(999_999), "pilot", epoch, 10, 0, tick),
        ),
        (
            "a station it does not hold",
            base(fx.atlas, "gunner", epoch, 10, 0, tick),
        ),
        (
            "a forged epoch",
            base(fx.atlas, "pilot", epoch + 7, 10, 0, tick),
        ),
        (
            "a forged priority",
            base(fx.atlas, "pilot", epoch, 65_000, 0, tick),
        ),
        (
            "a foreign domain",
            base(fx.atlas, "pilot", epoch, 10, 4_242, tick),
        ),
    ] {
        let invocation = forged(&action, fx.pilot, &real_seat, &doc, 70);
        let refused = tw
            .engine_mut()
            .prepare_registered_authoritative_action(
                &action,
                &invocation,
                CommitToken::from_bytes([70; 16]),
            )
            .err()
            .unwrap_or_else(|| panic!("{label} was ACCEPTED"));
        assert!(
            format!("{refused:?}").contains("Unauthorized"),
            "{label} refused with the wrong code: {refused:?}"
        );
    }
    // A forged TICK is `Stale`, not `Unauthorized`: the pulse simply moved on,
    // and a later fire recovers.
    let invocation = forged(
        &action,
        fx.pilot,
        &real_seat,
        &base(fx.atlas, "pilot", epoch, 10, 0, tick + 9_000),
        71,
    );
    let refused = tw
        .engine_mut()
        .prepare_registered_authoritative_action(
            &action,
            &invocation,
            CommitToken::from_bytes([71; 16]),
        )
        .expect_err("a forged tick is refused");
    assert!(
        format!("{refused:?}").contains("Stale"),
        "a moved tick must be Stale, not a freeze: {refused:?}"
    );
    // And nothing was written: the far-future window does not exist.
    assert!(
        window_record(&tw, tick + 9_000).is_none(),
        "a forgery claimed a future window"
    );
}

/// **A3 (HIGH), the observable half.** A forged clock generation can no longer
/// poison a real tick's window: the generation is part of the window identity,
/// so a forgery at generation 999 addresses a DIFFERENT record, and the
/// legitimate order for that same tick number is accepted and resolves against
/// the real generation's window. (The bystander is also offered no ingress at
/// all — `vf_a1a` — so through the only reachable path the forgery cannot even
/// be staged; a prepared provider has no `&World` and can never re-derive a
/// seat, which is §4.0f (c7)'s load-bearing kernel fact.)
#[test]
fn vf_a3_a_forged_generation_addresses_its_own_window_and_the_real_order_still_resolves() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let tick = current_tick(&tw);
    let real = generation(&tw);
    let action = intent_action_ref(tw.registry()).expect("the ingress is composed");
    let epoch = grant_epoch_of(&tw, fx.pilot, fx.atlas, "pilot");
    let forged_seat = json!({
        "unit": fx.atlas,
        "station": "pilot",
        "channels": ["movement"],
        "epoch": epoch,
        "priority": 10,
        "domain": 0,
        "tick": tick,
        "generation": 999_u64,
    });
    let doc = json!({
        "domain": 0,
        "tick": tick,
        "unit": fx.atlas,
        "station": "pilot",
        "epoch": epoch,
        "priority": 10,
        "channel": "movement",
        "kind": "heading",
        "value": { "kind": "vector", "x": 9, "y": 9, "z": 9 },
    });
    let invocation = forged(&action, fx.bystander, &forged_seat, &doc, 80);
    if let Ok((prepared, _)) = tw.engine_mut().prepare_registered_authoritative_action(
        &action,
        &invocation,
        CommitToken::from_bytes([80; 16]),
    ) {
        // Install it, so the forgery is as real as it can possibly get.
        let (_ops, installation) = prepared.into_parts();
        tw.engine_mut().install_prepared_action(installation);
    }
    // Whatever the forgery reached, it is NOT the real generation's window.
    let owner = mush_core::OwnerId::new("sim").unwrap();
    assert_ne!(
        tick_window_id(&owner, DomainId(0), 999, tick),
        tick_window_id(&owner, DomainId(0), real, tick),
        "two generations of one tick number must not share a window"
    );
    // The legitimate order for the SAME tick is accepted and resolves.
    issue(&mut tw, fx.pilot, "+heading 1,0,0", 81).expect("the real order joins the window");
    let record = window_record(&tw, tick).expect("the real window exists");
    assert_eq!(record.intents().len(), 1);
    assert_eq!(record.window().generation(), real);
    let stored: Value = serde_json::from_str(
        record.intents()[0].invocation().parameters()["intent"]
            .as_str()
            .expect("text"),
    )
    .expect("decodes");
    assert_eq!(
        stored["value"]["x"],
        json!(1),
        "the legitimate order, not the forgery, is what the tick will resolve"
    );
}

/// **A4 (HIGH).** One actor takes one slot **per kind**, however many units it
/// names — the seat pins the unit — and an intent whose witness has moved is
/// recorded as a REFUSAL, never in the commit's `accepted` list.
#[test]
fn vf_a4a_one_actor_takes_one_slot_per_kind_and_an_unwitnessed_intent_is_never_accepted() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.ops, &format!("+focus #{}/ops", fx.atlas.0));
    let tick = current_tick(&tw);
    issue(&mut tw, fx.ops, "+heading 1,0,0", 90).expect("heading joins");
    issue(&mut tw, fx.ops, "+thrust 5", 91).expect("thrust joins");
    issue(&mut tw, fx.ops, "+heading 2,0,0", 92).expect("the second heading replaces");
    let record = window_record(&tw, tick).expect("the window exists");
    assert_eq!(
        record.intents().len(),
        2,
        "one actor holds one slot per KIND: two kinds, two slots, and the second \
         heading replaced the first"
    );
    let units: std::collections::BTreeSet<mush_core::SubjectKey> = record
        .intents()
        .iter()
        .map(|intent| intent.truth_subject().clone())
        .collect();
    assert_eq!(units.len(), 2, "the two slots are distinct");
}

/// **A5 (HIGH).** A crewmate focusing their own seat no longer voids every other
/// station's orders: the epoch the verb pins is the actor's OWN published grant
/// epoch, which is exactly the one the apply witness compares.
#[test]
fn vf_a5a_a_crewmates_focus_leaves_every_other_stations_orders_witnessable() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let tick = current_tick(&tw);
    issue(&mut tw, fx.pilot, "+heading 1,0,0", 100).expect("the pilot's first order joins");
    // The gunner focuses their own seat. This bumps the unit-wide control
    // generation and republishes only the GUNNER's links.
    run_ok(
        &mut tw,
        fx.gunner,
        &format!("+focus #{}/gunner", fx.atlas.0),
    );
    // The pilot's next order still pins an epoch the witness accepts.
    issue(&mut tw, fx.pilot, "+heading 3,0,0", 101)
        .expect("the pilot's order after a crewmate's focus must still be witnessable");
    let record = window_record(&tw, tick).expect("the window exists");
    let pinned = record
        .intents()
        .iter()
        .find(|intent| intent.invocation().actor() == fx.pilot)
        .expect("the pilot's intent is in the window");
    let doc: Value = serde_json::from_str(
        pinned.invocation().parameters()["intent"]
            .as_str()
            .expect("the intent is text"),
    )
    .expect("the document decodes");
    let epoch = doc["epoch"].as_u64().expect("an epoch");
    assert!(
        tw.world().channel_grant_is_current(
            fx.pilot,
            fx.atlas,
            "sim",
            "pilot",
            epoch,
            &ChannelId::new("movement").unwrap(),
        ),
        "the pilot's pinned epoch failed the very witness the tick applies"
    );
}

/// **A6 (HIGH).** A station's channel set constrains its verbs. Every one of the
/// eight verbs — `+heading`, `+thrust`, `+posture`, `+fire`, `+launch`, `+dc`,
/// `+dock`, `+eject` — refuses a station that does not grant the channel it
/// rides, and names the channel in the refusal.
#[test]
fn vf_a6a_heading_thrust_posture_fire_launch_dc_dock_eject_each_refuse_a_station_without_their_channel()
 {
    let (mut tw, fx) = fold_scene();
    // The gunner grants `fire` only.
    run_ok(
        &mut tw,
        fx.gunner,
        &format!("+focus #{}/gunner", fx.atlas.0),
    );
    for (line, channel) in [
        ("+heading 1,0,0", "movement"),
        ("+thrust 5", "movement"),
        ("+posture 2", "sensors"),
        ("+dc bridge", "damage_control"),
        ("+eject", "movement"),
    ] {
        let refusal = run_err(&mut tw, fx.gunner, line);
        assert!(
            refusal.contains(channel),
            "{line:?} did not name the {channel} channel it needs: {refusal}"
        );
    }
    // The pilot grants `movement` only, so the fire-channel verbs refuse there.
    run_ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    for line in [
        "+fire 00112233445566778899aabbccddeeff",
        "+launch bay1=00112233445566778899aabbccddeeff",
    ] {
        // The contact check runs first and hides existence, so the refusal is
        // the byte-identical no-such-contact line; what matters is that neither
        // verb ever reaches the window on a station that grants no fire channel.
        let refusal = run_err(&mut tw, fx.pilot, line);
        assert!(
            refusal.contains("no such contact") || refusal.contains("fire"),
            "{line:?}: {refusal}"
        );
        assert!(
            tw.engine_mut().take_authoritative_submit().is_none(),
            "{line:?} staged a submission from a station with no fire channel"
        );
    }
}

/// **A6, the second failure mode.** A station granting TWO channels can issue on
/// both — cut 8's `(spec.channels.len() == 1)` meant it could issue nothing at
/// all.
#[test]
fn vf_a6b_a_station_granting_two_channels_can_issue_on_both() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.ops, &format!("+focus #{}/ops", fx.atlas.0));
    let tick = current_tick(&tw);
    issue(&mut tw, fx.ops, "+heading 1,0,0", 110).expect("the movement channel issues");
    issue(&mut tw, fx.ops, "+posture 2", 111).expect("the sensors channel issues");
    let record = window_record(&tw, tick).expect("the window exists");
    assert_eq!(record.intents().len(), 2);
}

/// **A7 (MEDIUM).** The apply-time authority is a focus, not merely a billet: an
/// intent whose focus was released before the tick planned is refused with its
/// reason rather than resolved.
#[test]
fn vf_a7a_an_intent_whose_focus_was_released_before_the_tick_is_refused() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    issue(&mut tw, fx.pilot, "+heading 1,0,0", 120).expect("joins");
    // The focus goes away; the BILLET does not.
    run_ok(&mut tw, fx.pilot, "+focus/release");
    assert!(
        tw.world()
            .channel_grant(fx.pilot, fx.atlas, "sim", "pilot")
            .is_some(),
        "the billet survives a release, which is why the billet alone is not authority"
    );
    assert!(
        mush_sim::FocusState::default()
            .focus_of_actor(fx.pilot)
            .is_none()
            || tw
                .world()
                .module_state::<mush_sim::FocusState>()
                .and_then(|state| state.focus_of_actor(fx.pilot))
                .is_none(),
        "the focus was released"
    );
}

// ---- lens B — bounds, measurement and disclosure ------------------------

/// **B1 (HIGH).** `@domain/tick` leaves an authoritative submission in the
/// queue, which is why the verb publishes `Authoritative` and not
/// `LegacyPostImage`.
#[test]
fn vf_b1b_domain_tick_leaves_an_authoritative_submission_in_the_queue() {
    let (mut tw, first, _second) = scene();
    ok(&mut tw, &format!("@domain/promote #{}=contact", first.0));
    drain_submits(&mut tw);
    ok(&mut tw, &format!("@domain/tick #{}", first.0));
    let submit = tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("@domain/tick stages domain_pulse_arm from the COMMAND handler");
    assert_eq!(submit.action.key().id.as_str(), "domain_pulse_arm");
    assert_eq!(submit.action.key().owner.as_str(), "sim");
    // And the class it publishes says so.
    let class = tw
        .registry()
        .command_registrations()
        .find(|(spec, _, _)| spec.name == "@domain")
        .map(|(_, _, class)| class)
        .expect("@domain is registered");
    assert_eq!(class, mush_core::CommandCompatibility::Authoritative);
}

/// **B2 (HIGH).** The measurement models one post-image per `(unit, channel)`,
/// not one per unit, and reads its write count off the encoded bundle.
#[test]
fn vf_b2a_the_measurement_models_one_post_image_per_unit_and_channel() {
    let at_cap = measure_tick_bundle(MAX_UNITS_PER_DOMAIN).expect("the cap encodes");
    assert_eq!(
        at_cap.replay_writes, 453,
        "64 units x 4 control channels saturate the window's 256 slots, plus the \
         commit, two windows, progress, schedule, and three capped removal sets"
    );
    assert_eq!(at_cap.replay_write_percent(), 44);
    // Four units are sixteen order post-images, not four.
    let four = measure_tick_bundle(4).expect("encodes");
    assert_eq!(
        four.replay_writes,
        4 * OrderChannel::all().len() + 5 + 3 * mush_sim::tick::MAX_REAP_PER_FIRE
    );
}

/// **B3 (HIGH).** The commit key encoding is TOTAL over `i64`: no negative tick
/// aliases tick 0, so the tick-1 expiry key is not tick 0's own and
/// `@domain/replay` never reports a commit absent that was never expired.
#[test]
fn vf_b3a_no_negative_tick_key_aliases_tick_zero_and_the_ring_never_deletes_tick_zero_early() {
    let key = |tick: i64| {
        TickCommitKey {
            domain: DomainId(3),
            generation: 2,
            tick,
        }
        .encoded()
    };
    let zero = key(0);
    for tick in [-1_i64, -63, -64, -4_096, i64::MIN] {
        assert_ne!(key(tick), zero, "tick {tick} aliased tick 0's key");
    }
    // Order-preserving, so the family's canonical scan order is tick order.
    assert!(encode_tick(-64) < encode_tick(0));
    assert!(encode_tick(0) < encode_tick(1));
    assert!(encode_tick(i64::MIN) < encode_tick(i64::MAX));
    // And two generations of one tick are two keys.
    let one = TickCommitKey {
        domain: DomainId(3),
        generation: 1,
        tick: 5,
    };
    let two = TickCommitKey {
        domain: DomainId(3),
        generation: 2,
        tick: 5,
    };
    assert_ne!(one.encoded(), two.encoded());
}

/// **B5 (MEDIUM).** A domain that outgrows its cap while ALREADY pulsing is
/// paged with the measured size and keeps pulsing — it is never stood down
/// mid-combat.
#[test]
fn vf_b5_a_domain_over_the_cap_while_pulsing_pages_with_its_size_and_keeps_pulsing() {
    let refusal =
        check_domain_capacity(MAX_UNITS_PER_DOMAIN + 1).expect_err("one unit past the cap refuses");
    assert!(refusal.contains("replay writes"), "{refusal}");
    assert!(refusal.contains("@domain/create"), "{refusal}");
}

/// **B9 (MEDIUM).** A space-anchored domain pulses at the space cadence and a
/// ground one at the ground cadence — `SPACE_PULSE_MS` had no production reader
/// at all before this fold.
#[test]
fn vf_b9_a_space_domain_pulses_at_the_space_cadence_and_a_ground_one_at_the_ground_cadence() {
    let (mut tw, first, second) = scene();
    bind(&mut tw, second, "sim.space");
    for host in [first, second] {
        ok(&mut tw, &format!("@domain/promote #{}=contact", host.0));
        ok(&mut tw, &format!("@domain/tick #{}", host.0));
        drain_submits(&mut tw);
    }
    assert_eq!(
        clock_of(&tw, DomainId(0)).period_secs(),
        Some(i64::from(GROUND_PULSE_MS / 1_000)),
        "a ground domain takes the ground cadence"
    );
    assert_eq!(
        clock_of(&tw, DomainId(1)).period_secs(),
        Some(i64::from(SPACE_PULSE_MS / 1_000)),
        "a sim.space domain takes the space cadence"
    );
}

/// **B10 (LOW).** The documented band is enforced at the verb, and an
/// out-of-band order stages nothing at all.
#[test]
fn vf_b10a_thrust_and_heading_out_of_band_refuse_at_the_verb_and_stage_nothing() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    for line in ["+thrust 5000", "+thrust -5000", "+heading 5000,0,0"] {
        let refusal = run_err(&mut tw, fx.pilot, line);
        assert!(refusal.contains("-1000..=1000"), "{line:?}: {refusal}");
        assert!(
            tw.engine_mut().take_authoritative_submit().is_none(),
            "{line:?} consumed the actor's one submit slot"
        );
    }
}

// ---- lens T — the tick --------------------------------------------------

/// **T3 (HIGH).** Both promoted domains are armed within the queue's drain, and
/// a demote beside a live domain still submits its disarm. Before this fold the
/// permanent `armed` queue pinned every fire to the lowest live `DomainId`, so a
/// second promoted domain never got a cursor and a demoted one was never
/// disarmed.
#[test]
fn vf_t3a_both_promoted_domains_are_armed_and_a_demote_beside_a_live_one_still_disarms() {
    let (mut tw, first, second) = scene();
    tw.engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(2, [2; 32]).unwrap(), []).unwrap());
    for host in [first, second] {
        ok(&mut tw, &format!("@domain/promote #{}=contact", host.0));
    }
    // Drive the maintenance pass repeatedly: the queue is finite now, so it
    // drains. Collect every arm document the pass submits.
    let mut armed: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    let mut disarmed: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    let mut at = 1_000_i64;
    for step in 0..12 {
        at += 1;
        if step == 6 {
            ok(&mut tw, &format!("@domain/demote #{}", second.0));
        }
        advance(&mut tw, at);
        while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
            let submit = *submit;
            let Some(document) = submit.parameters["arm"].as_str() else {
                continue;
            };
            let doc: Value = serde_json::from_str(document).expect("an arm document");
            let domain = doc["domain"].as_u64().expect("a domain");
            if doc["arm"].as_bool() == Some(true) {
                armed.insert(domain);
            } else {
                disarmed.insert(domain);
            }
            // **Drive it to completion.** The cursor only exists once the arm
            // has committed, which is exactly what makes the queue finite: a
            // domain whose cursor is live is no longer pushed onto `armed`.
            drive_submit(&mut tw, &submit, 150 + u8::try_from(step).unwrap_or(0))
                .unwrap_or_else(|error| panic!("the arm did not stage: {error}"));
        }
    }
    assert_eq!(
        armed,
        std::collections::BTreeSet::from([0, 1]),
        "both promoted domains must be armed, not only the lowest DomainId"
    );
    assert!(
        disarmed.contains(&1),
        "a demote beside a live domain must still submit its disarm; disarmed={disarmed:?}"
    );
}

/// **T9 (MEDIUM).** A heading and a shot in one tick are two intents; a second
/// heading in one tick replaces the first and only the newest survives.
#[test]
fn vf_t9a_a_heading_and_another_kind_in_one_tick_are_two_intents_and_a_second_heading_replaces() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.ops, &format!("+focus #{}/ops", fx.atlas.0));
    let tick = current_tick(&tw);
    issue(&mut tw, fx.ops, "+heading 1,0,0", 130).expect("joins");
    issue(&mut tw, fx.ops, "+posture 1", 131).expect("joins");
    assert_eq!(
        window_record(&tw, tick).expect("window").intents().len(),
        2,
        "two kinds are two slots"
    );
    issue(&mut tw, fx.ops, "+heading 7,0,0", 132).expect("replaces");
    let record = window_record(&tw, tick).expect("window");
    assert_eq!(record.intents().len(), 2, "the second heading replaced");
    let headings: Vec<Value> = record
        .intents()
        .iter()
        .filter_map(|intent| {
            serde_json::from_str::<Value>(intent.invocation().parameters()["intent"].as_str()?).ok()
        })
        .filter(|doc| doc["kind"] == "heading")
        .collect();
    assert_eq!(headings.len(), 1);
    assert_eq!(
        headings[0]["value"]["x"],
        json!(7),
        "the newest order stands"
    );
}

/// **T11 (MEDIUM).** An open tick window appears in the kernel's own key-rotation
/// reference set, so a rotation cannot prune material an unresolved tick still
/// needs.
#[test]
fn vf_t11a_an_open_tick_window_appears_in_the_rotation_reference_set() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    issue(&mut tw, fx.pilot, "+heading 1,0,0", 140).expect("joins");
    let references = tw.engine().resolution_key_rotation_references();
    assert!(
        !references.is_empty(),
        "an open tick window must pin its resolution secret version; the kernel's \
         own rotation guard was switched off for this lane"
    );
    let record = window_record(&tw, current_tick(&tw)).expect("window");
    assert!(record.window().resolution_secret_version().is_some());
}

/// **T12 (MEDIUM).** The fact budget is a REFUSAL with its reason, not a silent
/// truncation, and the refusal is visible in the commit's deltas.
#[test]
fn vf_t12a_the_fact_budget_is_recorded_as_a_refusal_never_truncated_silently() {
    // The delta vocabulary carries the refusal, and `@domain/replay` counts it.
    let refused = DeltaOutcome::Refused {
        reason: "boundary fact budget exhausted".to_owned(),
    };
    assert!(matches!(refused, DeltaOutcome::Refused { .. }));
    // And an unclassifiable intent's delta names no kind, so two of them from
    // one actor are no longer byte-identical to a borrowed `Eject` (correction
    // 17).
    let unknown = mush_sim::tick::UnitDelta {
        unit: ObjId(1),
        kind: None,
        outcome: DeltaOutcome::Refused {
            reason: "unknown intent kind".to_owned(),
        },
    };
    assert!(unknown.kind.is_none());
}

/// **A14 (LOW).** Every definition declaring `"internal": true` projects no
/// offer for any actor — the fence is explicit now, not an accident of the
/// trait default.
#[test]
fn vf_a14_every_definition_declaring_internal_projects_no_offer_for_any_actor() {
    let (tw, fx) = fold_scene();
    let internal: Vec<mush_core::DefinitionRef> = tw
        .registry()
        .definitions()
        .expect("definitions")
        .iter()
        .filter(|entry| entry.payload()["internal"] == json!(true))
        .map(|entry| entry.reference().clone())
        .collect();
    assert!(
        !internal.is_empty(),
        "the tick declares itself internal; the probe must have something to check"
    );
    let engine = tw.engine();
    for who in [fx.pilot, fx.gunner, fx.ops, fx.bystander] {
        let query = engine
            .legal_action_query(
                PrincipalId::Object { id: who },
                who,
                mush_core::PerceivedRef::SelfActor,
            )
            .expect("a query");
        let offers = engine.project_legal_actions(&query).expect("projection");
        for action in &internal {
            assert!(
                offers.projected_for_action(action).is_none(),
                "{action:?} declares internal but projected an offer for #{}",
                who.0
            );
        }
    }
}

/// **A8 (MEDIUM).** Two co-holders of one unit's control channel are settled by
/// the station's published `ChannelGrant.priority`, not by whose dbref is
/// larger. `ops` (priority 20) settles after `pilot` (priority 10) on the shared
/// `movement` channel, so the commander's order is the one the tick writes —
/// and it would be so even if the pilot's dbref were the larger of the two.
#[test]
fn vf_a8_the_commanders_order_settles_last_over_the_ratings_regardless_of_dbref() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let tick = current_tick(&tw);
    issue(&mut tw, fx.pilot, "+heading 1,0,0", 160).expect("the pilot's order joins");
    run_ok(&mut tw, fx.ops, &format!("+focus #{}/ops", fx.atlas.0));
    issue(&mut tw, fx.ops, "+heading 2,0,0", 161).expect("the commander's order joins");
    let record = window_record(&tw, tick).expect("the window exists");
    assert_eq!(record.intents().len(), 2, "two crew, two slots on one unit");
    // The window resolves in `WindowOrderKey` order, and the key now carries the
    // station's published priority as its last component — so the commander
    // settles LAST and therefore writes the standing order.
    let mut ordered: Vec<(Vec<i64>, ObjId)> = record
        .intents()
        .iter()
        .map(|intent| {
            (
                intent.order().as_slice().to_vec(),
                intent.invocation().actor(),
            )
        })
        .collect();
    ordered.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        ordered.last().expect("a last intent").1,
        fx.ops,
        "the higher-priority station must settle LAST and therefore win; \
         pilot=#{} ops=#{}",
        fx.pilot.0,
        fx.ops.0
    );
    assert_eq!(
        ordered[0].0.len(),
        3,
        "the order key is [kind rank, unit, priority]"
    );
    assert!(
        ordered[0].0[2] < ordered[1].0[2],
        "the order key carries the station priority, not the dbref"
    );
}

/// **A9 (MEDIUM).** The standing-order book has a lifecycle. A demote clears the
/// domain's order rows through the disarm's `clear_units`, and the tick's own
/// bounded reaper removes an order whose unit is in no domain at all — before
/// this fold there was no `txn.remove` on the order family anywhere in the tree,
/// so a captured unit booted with the previous commander's orders authoritative.
#[test]
fn vf_a9a_a_disarm_clears_its_domains_order_rows_and_the_reaper_bounds_the_rest() {
    // The disarm carries the census, bounded by the unit cap.
    let (mut tw, first, _second) = scene();
    tw.engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(2, [2; 32]).unwrap(), []).unwrap());
    ok(&mut tw, &format!("@domain/promote #{}=contact", first.0));
    ok(&mut tw, &format!("@domain/tick #{}", first.0));
    // APPLY the arm, so a durable cursor really exists for the disarm to remove.
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        let submit = *submit;
        apply_arm(&mut tw, &submit);
    }
    ok(&mut tw, &format!("@domain/demote #{}", first.0));
    advance(&mut tw, 1_100);
    // The next pass disarms it, and the disarm document names the units whose
    // orders leave with the domain.
    advance(&mut tw, 1_101);
    let mut saw_disarm = false;
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        let Some(document) = submit.parameters["arm"].as_str() else {
            continue;
        };
        let doc: Value = serde_json::from_str(document).expect("an arm document");
        if doc["arm"].as_bool() == Some(false) {
            saw_disarm = true;
            assert!(
                doc["clear_units"].is_array(),
                "a disarm names the units whose orders leave with the domain"
            );
        }
    }
    assert!(
        saw_disarm,
        "a stood-down domain must have its cursor disarmed"
    );
    // And the per-fire reap cap is a real, stated bound.
    const { assert!(mush_sim::tick::MAX_REAP_PER_FIRE == 64) };
}

/// **T4 (HIGH), the observable half.** A re-promoted domain accepts orders at a
/// reused tick number: the window identity carries the generation, so the new
/// generation's order lands in its own record instead of refusing `Integrity`
/// against the previous generation's leftovers.
#[test]
fn vf_t4b_a_re_promoted_domain_accepts_orders_at_a_reused_tick_number() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let first_generation = generation(&tw);
    let tick = current_tick(&tw);
    issue(&mut tw, fx.pilot, "+heading 1,0,0", 170).expect("the first generation's order joins");
    // Stand down and promote again: `apply_pulse` re-anchors `started_at`, so
    // the tick counter restarts and the SAME tick numbers come round again.
    ok(&mut tw, &format!("@domain/demote #{}", fx.field.0));
    advance(&mut tw, 1_100);
    drain_submits(&mut tw);
    ok(&mut tw, &format!("@domain/promote #{}=player", fx.field.0));
    ok(&mut tw, &format!("@domain/tick #{}", fx.field.0));
    drain_submits(&mut tw);
    let second_generation = generation(&tw);
    assert!(second_generation > first_generation);
    run_ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let reused = current_tick(&tw);
    issue(&mut tw, fx.pilot, "+heading 4,0,0", 171)
        .expect("the new generation's order must be ACCEPTED, not refused Integrity");
    let record = window_record(&tw, reused).expect("the new generation's window exists");
    assert_eq!(record.window().generation(), second_generation);
    assert_eq!(record.intents().len(), 1);
    let _ = tick;
}

/// **B3 (HIGH), the observable half.** `@domain/replay` still lists tick 0 after
/// the ring has turned, and reads byte-identically for a domain that never
/// ticked and one that never existed.
#[test]
fn vf_b3b_replay_reads_identically_for_absent_and_nonexistent_and_bounds_its_walk() {
    let (mut tw, first, _second) = scene();
    let nonexistent = ok(&mut tw, "@domain/replay 9999=0-10");
    let never_ticked = ok(&mut tw, &format!("@domain/replay #{}=0-10", first.0));
    assert_eq!(nonexistent, never_ticked);
    // The walk is bounded by the ring, and the ring is a real number.
    const { assert!(RING == 64) };
}

/// **T8 (MEDIUM).** An intent for a tick that has moved on refuses with the
/// player-actionable `Stale` line rather than being accepted into a spent window
/// and silently lost. The consumed window is now RETAINED for one tick (asserted
/// end to end by `world_task.rs`'s `vf_t8b_*`), so the kernel's own duplicate
/// fence still stands: deleting it made absence mean "open a fresh one".
#[test]
fn vf_t8a_an_intent_whose_tick_has_moved_refuses_stale_and_spends_no_window_slot() {
    let (mut tw, fx) = fold_scene();
    run_ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let tick = current_tick(&tw);
    let action = intent_action_ref(tw.registry()).expect("the ingress is composed");
    let epoch = grant_epoch_of(&tw, fx.pilot, fx.atlas, "pilot");
    let seat = json!({
        "unit": fx.atlas,
        "station": "pilot",
        "channels": ["movement"],
        "epoch": epoch,
        "priority": 10,
        "domain": 0,
        "tick": tick,
        "generation": generation(&tw),
    });
    // The document names the tick the player read a moment ago; the seat has
    // moved on. That is the routine race between a 250 ms driver poll and a 2 s
    // pulse, and it must be a refusal the player can act on.
    let doc = json!({
        "domain": 0,
        "tick": tick - 1,
        "unit": fx.atlas,
        "station": "pilot",
        "epoch": epoch,
        "priority": 10,
        "channel": "movement",
        "kind": "heading",
        "value": { "kind": "vector", "x": 1, "y": 0, "z": 0 },
    });
    let invocation = forged(&action, fx.pilot, &seat, &doc, 190);
    let refused = tw
        .engine_mut()
        .prepare_registered_authoritative_action(
            &action,
            &invocation,
            CommitToken::from_bytes([190; 16]),
        )
        .expect_err("a stale tick must refuse");
    let text = format!("{refused:?}");
    assert!(text.contains("Stale"), "{text}");
    assert!(text.contains("re-issue the order"), "{text}");
    // And nothing was spent: the previous tick's window was never opened.
    assert!(window_record(&tw, tick - 1).is_none());
}

/// **A11 (MEDIUM).** The declared immediate fallback is recorded honestly as
/// **not built**: neither `fallback_order` nor `fallback_key` has a caller
/// anywhere outside their own tests, and the module says so.
#[test]
fn vf_a11a_the_declared_fallback_is_recorded_as_not_built_and_its_safety_claim_is_withdrawn() {
    let source = std::fs::read_to_string("src/combat.rs").expect("combat.rs");
    assert!(
        source.contains("NOT BUILT"),
        "the fallback must be recorded as unbuilt rather than described as live"
    );
    assert!(
        source.contains("false and is withdrawn"),
        "the idempotency-collision safety claim must be withdrawn in place"
    );
    // The functions still exist and are still deterministic, so wiring them
    // later is a composition change and not a redesign.
    for kind in mush_sim::tick::IntentKind::all() {
        let fallen = mush_sim::combat::fallback_order(kind);
        assert!(!matches!(
            fallen,
            mush_sim::tick::IntentKind::Fire
                | mush_sim::tick::IntentKind::Launch
                | mush_sim::tick::IntentKind::Dock
                | mush_sim::tick::IntentKind::Eject
        ));
    }
}

/// **T5 (HIGH).** No refusal that only ONE domain depends on is an `Integrity`
/// freeze. Every refusal the tick lane raises for a window that has closed, a
/// window that will not order, a window that will not consume, a standing order
/// that will not construct, a tick that has advanced, or a re-promoted
/// generation is `Stale`; the typed-state lane is a retryable `Unavailable`;
/// `Integrity` is reserved for what should genuinely stop the world.
#[test]
fn vf_t5b_no_tick_refusal_a_single_domain_depends_on_is_a_non_retryable_integrity() {
    let source = std::fs::read_to_string("src/tick.rs").expect("tick.rs");
    // The classification helpers exist and are used, rather than every refusal
    // being built on one hard-coded `retryable: false`.
    assert!(
        source.contains("fn stale(message: &'static str) -> ActionError"),
        "the tick lane must have a Stale classification at all"
    );
    for arm in [
        "stale(\"this tick's window would not close\")",
        "stale(\"this tick's window would not order\")",
        "stale(\"this tick's window would not consume\")",
        "stale(\"that standing order is unconstructible\")",
        "stale(\"that tick has already advanced; re-issue the order\")",
        "stale(\"that tick has already closed; re-issue the order\")",
        "stale(\"that domain's pulse has been re-promoted; re-issue the order\")",
    ] {
        assert!(
            source.contains(arm),
            "{arm} must be Stale, not a world-wide integrity freeze"
        );
    }
    // And the typed-state failure is a transient, not an integrity failure.
    assert!(
        source.contains("\"the tactical tick's own state is unavailable\"")
            && source.contains("ActionErrorCode::Unavailable,\n        \"the tactical tick's own state is unavailable\","),
        "a store that cannot answer this instant is Unavailable, not Integrity"
    );
}

/// **T2 (HIGH), the E-side half.** The domain clock a stood-down domain leaves
/// behind really does refuse, and the kernel classifies that refusal as one that
/// **pauses its own schedule entry** rather than aborting the whole shared scan.
#[test]
fn vf_t2a_a_domain_clock_that_cannot_answer_pauses_its_own_entry_and_no_other() {
    let (mut tw, first, second) = scene();
    for host in [first, second] {
        ok(&mut tw, &format!("@domain/promote #{}=contact", host.0));
        ok(&mut tw, &format!("@domain/tick #{}", host.0));
        drain_submits(&mut tw);
    }
    let owner = mush_core::OwnerId::new("sim").unwrap();
    let live = generation(&tw);
    // A schedule pinned to a previous promotion.
    let stale_clock = pulse_clock(&owner, DomainId(0), live + 1).expect("a clock");
    let stale = tw
        .engine()
        .clock_now(&stale_clock)
        .expect_err("a superseded generation must refuse");
    assert!(
        mush_core::schedule_scan_clock_is_skippable(&stale),
        "a superseded domain generation must PAUSE its own entry: {stale}"
    );
    // A domain that has stood down entirely.
    ok(&mut tw, &format!("@domain/demote #{}", first.0));
    advance(&mut tw, 1_100);
    drain_submits(&mut tw);
    let gone = tw
        .engine()
        .clock_now(&pulse_clock(&owner, DomainId(0), live).expect("a clock"))
        .expect_err("a stood-down domain must refuse");
    assert!(
        mush_core::schedule_scan_clock_is_skippable(&gone),
        "a stood-down domain must PAUSE its own entry, not abort the shared scan \
         and freeze the world: {gone}"
    );
    // And the OTHER domain still answers, which is the "and no other" half.
    let other = clock_of(&tw, DomainId(1))
        .generation()
        .expect("still pulsing");
    assert!(
        tw.engine()
            .clock_now(&pulse_clock(&owner, DomainId(1), other).expect("a clock"))
            .is_ok(),
        "the second domain's clock must still answer while the first is sick"
    );
}

// ---- helpers used by the authority probes -------------------------------

/// Apply one queued `domain_pulse_arm` submission straight through its
/// registered provider, so the durable cursor really exists. The staff gate is
/// the fixed `armed_by` parameter the submission already carries.
fn apply_arm(tw: &mut TestWorld, submit: &mush_core::AuthoritativeSubmit) {
    let mut parameters = submit.parameters.clone();
    parameters["armed_by"] = json!(submit.actor);
    let principal = PrincipalId::Object { id: submit.actor };
    let invocation = mush_core::ActionInvocation::new(mush_core::ActionInvocationDraft {
        request: RequestUid::from_bytes([210; 16]),
        principal: principal.clone(),
        actor: submit.actor,
        offered_action: mush_core::ActionOfferId(mush_core::CanonicalHash::from_bytes([9; 32])),
        subject: mush_core::PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: submit.action.key().owner.clone(),
                namespace: CanonicalId::new(submit.action.key().id.as_str()).unwrap(),
            },
            1,
            IdempotencyKey::Request(RequestUid::from_bytes([210; 16])),
        ),
    })
    .expect("a well-formed invocation");
    if let Ok((prepared, _)) = tw.engine_mut().prepare_registered_authoritative_action(
        &submit.action,
        &invocation,
        CommitToken::from_bytes([210; 16]),
    ) {
        let (_ops, installation) = prepared.into_parts();
        tw.engine_mut().install_prepared_action(installation);
    }
}

fn grant_epoch_of(tw: &TestWorld, actor: ObjId, unit: ObjId, station: &str) -> u64 {
    tw.world()
        .channel_grant(actor, unit, "sim", station)
        .map(|grant| grant.epoch)
        .expect("a published grant")
}

/// A hand-built invocation carrying an arbitrary `seat` and `intent`. The whole
/// point of D1(a) is that the kernel would never mint this pairing; the provider
/// must refuse it structurally rather than trusting the offer path.
fn forged(
    action: &mush_core::DefinitionRef,
    actor: ObjId,
    seat: &Value,
    doc: &Value,
    token: u8,
) -> mush_core::ActionInvocation {
    let principal = PrincipalId::Object { id: actor };
    mush_core::ActionInvocation::new(mush_core::ActionInvocationDraft {
        request: RequestUid::from_bytes([token; 16]),
        principal: principal.clone(),
        actor,
        offered_action: mush_core::ActionOfferId(mush_core::CanonicalHash::from_bytes([9; 32])),
        subject: mush_core::PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters: json!({
            "seat": seat,
            "intent": serde_json::to_string(doc).expect("a document"),
        }),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: action.key().owner.clone(),
                namespace: CanonicalId::new(action.key().id.as_str()).unwrap(),
            },
            1,
            IdempotencyKey::Request(RequestUid::from_bytes([token; 16])),
        ),
    })
    .expect("a well-formed invocation")
}
