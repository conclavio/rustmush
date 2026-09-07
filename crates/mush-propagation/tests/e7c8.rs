//! E7 cut 8 independent verify fold — **D8(a): the propagation-owned detection
//! tick** (10b §3.8 deviation (f)).
//!
//! §3.8 deferred a detection driver, and the deferral left cut 8's whole surface
//! unreachable. `PropagationSensorPolicy` *is* composed in production and it
//! *does* call `detection_pass`, but `SensorPolicy::due` has exactly one caller
//! in the tree — `SensorScheduleAdapter::re_derived`, which runs only for a
//! `SensorJob` that is **already armed**. Nothing armed automatically, so
//! `@prop/pass` (staff) was the only producer of a contact anywhere: on a
//! running server no contact formed, `+fire` had nothing to name, and the whole
//! tactical surface was unreachable end to end *even before* A2.
//!
//! These rows drive the real world tick and assert the observable outcome: a
//! contact **forms in a promoted domain with no staff action at all**.

use std::sync::Arc;

use mush_core::{
    Audience, BehaviorBinding, BehaviorId, ContactLedgerState, GOD, LIMBO, ObjId, ObjKind,
};
use mush_propagation::{
    DETECTION_TICK_TAG, MAX_DOMAINS_PER_DETECTION_FIRE, PropagationModule, PropagationSensorPolicy,
    PropagationStore,
};
use mush_proto::OutputEvent;
use mush_sim::{SimulationModule, SpatialState};
use mush_tactics::TacticsModule;
use mush_testkit::{TestWorld, text_of};

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

fn ok(tw: &mut TestWorld, line: &str) -> String {
    let events = tw.run(GOD, line);
    let text = texts(&events);
    assert!(!has_error(&events), "{line:?} refused: {text}");
    text
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

struct Fx {
    orbit: ObjId,
    alpha: ObjId,
    bravo: ObjId,
    ann: ObjId,
}

/// Two placed platforms in one domain and one crewman at a sensors station.
/// Nothing here promotes the domain: each row decides that for itself, which is
/// the whole point of the comparison.
fn scene() -> (TestWorld, Fx) {
    let mut tw = boot();
    tw.set_now(1_000);
    let orbit = tw
        .world_mut()
        .create(ObjKind::Room, "Orbit", GOD, None)
        .unwrap();
    let alpha = tw
        .world_mut()
        .create(ObjKind::Thing, "Alpha", GOD, Some(LIMBO))
        .unwrap();
    let bravo = tw
        .world_mut()
        .create(ObjKind::Thing, "Bravo", GOD, Some(LIMBO))
        .unwrap();
    let ann = tw
        .world_mut()
        .create(ObjKind::Player, "Ann", GOD, Some(LIMBO))
        .unwrap();
    bind(&mut tw, orbit, "sim.domain");
    for object in [alpha, bravo, ann] {
        bind(&mut tw, object, "sim.participant");
    }
    ok(&mut tw, &format!("@domain/create #{}=sim.passive", orbit.0));
    for object in [alpha, bravo, ann] {
        ok(&mut tw, &format!("@spatial/place/domain #{}=0", object.0));
    }
    ok(
        &mut tw,
        &format!(
            "@spatial/station #{}=sensors,sensors,channels:sensors",
            alpha.0
        ),
    );
    ok(
        &mut tw,
        &format!("@spatial/assign #{}=sensors,#{}", alpha.0, ann.0),
    );
    let _ = tw.take_module_ops();
    (
        tw,
        Fx {
            orbit,
            alpha,
            bravo,
            ann,
        },
    )
}

/// Drive the world clock forward and fire every due tick, draining whatever the
/// ticks submit exactly as the production `CommandSubmitDriver` does.
fn advance(tw: &mut TestWorld, secs: i64) -> usize {
    tw.set_now(secs);
    let _ = tw.fire_due(secs * 1_000);
    let mut submissions = 0_usize;
    while tw.engine_mut().take_authoritative_submit().is_some() {
        submissions += 1;
    }
    submissions
}

fn sightings(tw: &TestWorld) -> usize {
    tw.world()
        .module_state::<PropagationStore>()
        .map(|book| book.sightings().count())
        .unwrap_or(0)
}

/// Promote the domain and **apply** the pulse arm, so `sim.domain_pulse` stops
/// wanting the shared submit slot on every fire.
///
/// This matters: `Ctx::submit_authoritative` is one per ACTOR per deferred
/// drain and every `reg.tick` runs as `GOD`, so a drain carrying the pulse
/// tick's arm AND the detection tick's arm fans out and voids both (cut 2's
/// fold C4). In production the `CommandSubmitDriver` applies the arm within a
/// second and the pulse tick then submits nothing; the rig has to do that
/// itself.
fn promote(tw: &mut TestWorld, host: ObjId) {
    ok(tw, &format!("@domain/promote #{}=contact", host.0));
    ok(tw, &format!("@domain/tick #{}", host.0));
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        let submit = *submit;
        apply_directly(tw, &submit);
    }
}

/// Apply one queued authoritative submission straight through its registered
/// provider. The staff gate on `domain_pulse_arm` is the fixed `armed_by`
/// parameter, which the submission already carries by construction.
fn apply_directly(tw: &mut TestWorld, submit: &mush_core::AuthoritativeSubmit) {
    let mut parameters = submit.parameters.clone();
    parameters["armed_by"] = serde_json::json!(submit.actor);
    let principal = mush_core::PrincipalId::Object { id: submit.actor };
    let invocation = mush_core::ActionInvocation::new(mush_core::ActionInvocationDraft {
        request: mush_core::RequestUid::from_bytes([31; 16]),
        principal: principal.clone(),
        actor: submit.actor,
        offered_action: mush_core::ActionOfferId(mush_core::CanonicalHash::from_bytes([9; 32])),
        subject: mush_core::PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters,
        idempotency: mush_core::ScopedIdempotencyKey::new(
            mush_core::IdempotencyScope {
                principal,
                owner: submit.action.key().owner.clone(),
                namespace: mush_core::CanonicalId::new(submit.action.key().id.as_str()).unwrap(),
            },
            1,
            mush_core::IdempotencyKey::Request(mush_core::RequestUid::from_bytes([31; 16])),
        ),
    })
    .expect("a well-formed invocation");
    if let Ok((prepared, _)) = tw.engine_mut().prepare_registered_authoritative_action(
        &submit.action,
        &invocation,
        mush_core::CommitToken::from_bytes([31; 16]),
    ) {
        let (_ops, installation) = prepared.into_parts();
        tw.engine_mut().install_prepared_action(installation);
    }
}

// ---- E7 cut 8 independent verify fold ----

/// **D8(a), the whole finding.** A **promoted** domain forms observations with
/// **no staff action at all** — no `@prop/pass`, no `@prop/phantom`, nothing.
/// A passive domain does not, which is what makes the tick a *pulse-scoped*
/// driver rather than a second unconditional sweep.
#[test]
fn vf_d8a_a_promoted_domain_detects_with_no_staff_action_and_a_passive_one_does_not() {
    let (mut tw, fx) = scene();
    // Passive: the detection tick fires and finds no promoted domain, so it
    // does nothing at all — `@prop/pass` remains the only producer there.
    let passive_submissions = advance(&mut tw, 1_010);
    assert_eq!(
        sightings(&tw),
        0,
        "a passive domain must not be swept by the pulse-scoped detection tick"
    );
    assert_eq!(
        passive_submissions, 0,
        "a passive domain arms nothing through the tactics ingress"
    );

    // Promoted: the same tick now walks it and arms observations.
    promote(&mut tw, fx.orbit);
    let armed = advance(&mut tw, 1_020);
    assert!(
        sightings(&tw) > 0,
        "a promoted domain must form observations with no staff action; the pulse \
         was unreachable end to end without this"
    );
    assert_eq!(
        armed, 1,
        "exactly ONE Ctx::submit_authoritative per fire (cut 2's fold C4)"
    );
    let _ = (fx.alpha, fx.bravo, fx.ann);
}

/// **D8(a), the ingress half.** What the tick arms is the **tactics-owned**
/// `sensor_arm` action, resolved by definition-registry lookup exactly as
/// `@prop/pass` does — `mush-propagation` registers no contact family, no
/// contact guard validator and no ingress of its own.
#[test]
fn vf_d8b_the_detection_tick_arms_the_tactics_owned_sensor_arm_and_nothing_of_its_own() {
    let (mut tw, fx) = scene();
    promote(&mut tw, fx.orbit);
    tw.set_now(1_020);
    let _ = tw.fire_due(1_020 * 1_000);
    let submit = tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("the detection tick armed an observation");
    assert_eq!(submit.action.key().owner.as_str(), "tactics");
    assert_eq!(submit.action.key().id.as_str(), "sensor_arm");
    assert!(
        submit.parameters["intents"].is_string(),
        "the arm carries the bounded intent batch as its one text parameter"
    );
    assert!(
        tw.engine_mut().take_authoritative_submit().is_none(),
        "one submission per fire"
    );
}

/// **D8(a), the bounds half.** The tick is bounded on promoted domains visited
/// and on observations armed, and both bounds are **paged** into the store's own
/// notice queue rather than logged into the dark (E4c V6) — which is what
/// `@prop/pass` and `@prop/held` drain.
#[test]
fn vf_d8c_the_detection_tick_is_bounded_on_domains_and_on_intents_and_pages_its_bound() {
    const { assert!(MAX_DOMAINS_PER_DETECTION_FIRE == 8) };
    assert_eq!(DETECTION_TICK_TAG, "propagation.detection");
    // Disjoint from every other tick tag and schedule-source id on the tree.
    for other in [
        "propagation.delivery",
        "sim.journey",
        "sim.domain_pulse",
        "route.sweep",
        "domaintick",
        "control_handoff",
        "production",
        "npc",
    ] {
        assert_ne!(DETECTION_TICK_TAG, other);
    }
    let (mut tw, fx) = scene();
    promote(&mut tw, fx.orbit);
    advance(&mut tw, 1_020);
    // Whatever the pass reported, it went into the store's notice queue, which
    // is a real reader — not a `tracing` line.
    let notices = tw
        .world_mut()
        .module_state_mut::<PropagationStore>()
        .map(|book| book.take_notices())
        .unwrap_or_default();
    // A small scene fits inside every bound, so the honest assertion is that
    // nothing was paged and nothing was lost — not that a page appeared.
    assert!(
        notices
            .iter()
            .all(|notice| !notice.contains("stopped at its bound")),
        "a two-platform domain must not hit any bound: {notices:?}"
    );
}

/// **D8(a), the composition half.** Adding the detection tick adds no crate
/// edge and no second contact authority: `mush-propagation` still registers no
/// `AuthorityGuardKind::Contact` validator, and the ledger it reads is
/// `mush-core`'s leak-safe projection.
#[test]
fn vf_d8d_the_detection_tick_adds_no_contact_authority_and_no_new_family() {
    let (tw, _fx) = scene();
    let validators = tw
        .registry()
        .authority_guard_validators()
        .expect("the composition froze the guard registry");
    let guard_for = |owner: &str| mush_core::AuthorityGuard::Contact {
        provider: mush_core::OwnerId::new(owner).unwrap(),
        principal: mush_core::PrincipalId::Object { id: ObjId(1) },
        id: mush_core::ContactId::from_bytes([1; 16]),
        generation: 1,
    };
    assert!(validators.get_for_guard(&guard_for("tactics")).is_ok());
    for owner in ["propagation", "sim"] {
        assert!(
            validators.get_for_guard(&guard_for(owner)).is_err(),
            "{owner} must not register a contact authority"
        );
    }
    assert!(tw.world().module_state::<ContactLedgerState>().is_some());
    assert!(tw.world().module_state::<SpatialState>().is_some());
}
