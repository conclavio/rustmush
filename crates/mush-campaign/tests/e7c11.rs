//! E7 cut 11 **B0** acceptance (10b §3.11 / §3.10 deviations (c) and (h)): the
//! proxy seat's **driving ingress**, and T11's guardrail on the acts it opens.
//!
//! Cut 10 shipped the seat's consent, attribution, narrowing, gate and give-back
//! and **not** its driving ingress: `ActiveFocus` for a proxy seat carries
//! `actor = npc`, so `FocusState::focus_of_actor(driver)` answered `None` and
//! every station-gated verb the driver typed refused with "you hold no focused
//! station". `vf_a11` pinned the tell. This suite is the flip.
//!
//! Everything runs on the **shared** cut-9/10 rig (`tests/common/mod.rs`) — the
//! same scene, the same two NPCs, the same commander, driver and bystander — and
//! every drive goes through the REAL `command -> offer -> ingress -> window`
//! path: the verb stages an authoritative submit, the engine projects the
//! action's own offer (which resolves the seat through the ingress), the offer
//! is resolved and prepared, and the tick window is read back from the
//! transaction. A provider-level call does not count.

use std::sync::Arc;

use mush_campaign::OperationId;
use mush_core::{
    CanonicalId, ChannelId, CommitToken, GOD, IdempotencyKey, IdempotencyKeyText, IdempotencyScope,
    LIMBO, ModuleOp, ModuleWrite, ObjId, ObjKind, OfferExecutionContext, OfferKeyRing, OfferSecret,
    OfferSubmission, PrincipalId, RequestUid, ScopedIdempotencyKey, SeatAudit,
};
use mush_testkit::TestWorld;

mod common;
use common::*;

// ---- E7c11: B0 (the rig extension — a pulsing domain and a live seat) ----

/// Arm the engine's offer key ring so a staged submit can be resolved at all.
fn arm_offer_keys(tw: &mut TestWorld) {
    tw.engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(2, [2; 32]).unwrap(), []).unwrap());
}

/// Promote and apply the scene's domain so it PULSES: the campaign tick promotes
/// a seat's grant to `Active { epoch }` only on a pulsing domain, so without this
/// `vf_d1b`'s hazard is unreachable and the K6 lease never lands.
fn pulse(tw: &mut TestWorld, fx: &Fx) {
    ok(tw, GOD, &format!("@domain/promote #{}=contact", fx.field.0));
    ok(tw, GOD, &format!("@domain/tick #{}", fx.field.0));
}

/// Offer, accept and activate one seat, draining the engine's one-submit slot.
fn seat_up(tw: &mut TestWorld, fx: &Fx, op: OperationId, unit: ObjId, npc: ObjId, channels: &str) {
    let seat = offer(tw, fx, op, unit, npc, channels);
    ok(tw, fx.driver, &format!("+seat/accept {}/{}", op.0, seat.0));
    ok(
        tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    drain(tw);
}

/// A unit whose `pilot` station declares exactly ONE control channel, plus its
/// own AI NPC and declared fallback, joined to `op`.
///
/// The K6 control lease is **per channel** and `handoff::station_channel`
/// deliberately refuses to project an offer for a station that declares more
/// than one, so the shared scene's two-channel `pilot` can never reach the lease
/// path at all. A10's arm lives on that path, so it needs a single-channel unit.
fn single_channel_unit(tw: &mut TestWorld, fx: &Fx, op: OperationId, name: &str) -> (ObjId, ObjId) {
    let unit = tw
        .world_mut()
        .create(ObjKind::Thing, name, fx.commander, Some(LIMBO))
        .expect("a unit");
    let npc = tw
        .world_mut()
        .create(
            ObjKind::Npc,
            &format!("{name} Watch"),
            fx.commander,
            Some(LIMBO),
        )
        .expect("an npc");
    bind(tw, unit, "sim.participant");
    ok(tw, GOD, &format!("@spatial/place/domain #{}=0", unit.0));
    ok(
        tw,
        GOD,
        &format!("@spatial/station #{}=pilot,pilot,channels:movement", unit.0),
    );
    ok(
        tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", unit.0, npc.0),
    );
    ok(
        tw,
        GOD,
        &format!("@station/fallback #{}/pilot=#{}", unit.0, npc.0),
    );
    tw.world_mut()
        .move_to(npc, fx.field)
        .expect("the npc stands its watch");
    ok(tw, fx.commander, &format!("@op/join {}=#{}", op.0, unit.0));
    (unit, npc)
}

/// Resolve one queued command-originated authoritative submit through the REAL
/// offer path — exactly as `mush-server`'s `command_offer_submission` +
/// `resolve_and_stage_offer` do — and stage it, returning the transaction ops.
///
/// This is the "through the ingress" harness cut 8's verify made binding: the
/// offer is *projected for the submitting actor*, so a provider that refuses to
/// project for that actor dead-ends the whole path here with `Ok(None)`. The
/// tactical half of the harness went with the retired combat lane (16b cut T,
/// D-16-20); what drives it now is the seat's own `control_handoff_request`.
fn drive_submit(
    tw: &mut TestWorld,
    submit: &mush_core::AuthoritativeSubmit,
    token: u8,
) -> Option<Vec<ModuleOp>> {
    let (action, resolved) = {
        let engine = tw.engine();
        let query = engine
            .legal_action_query(
                PrincipalId::Object { id: submit.actor },
                submit.actor,
                submit.subject.clone(),
            )
            .ok()?;
        let offers = engine.project_legal_actions(&query).ok()?;
        let projected = offers.projected_for_action(&submit.action)?;
        let submission = OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: submit.targets.clone(),
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
            .ok()?;
        let (action, resolved, _guards) = resolved.into_authoritative_parts();
        (action, resolved)
    };
    let (prepared, _) = tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes([token; 16]),
        )
        .ok()?;
    let (ops, installation) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(installation)
        .expect("the fixture installs");
    Some(ops)
}

/// Discover and apply the one due scheduled `control_handoff_apply`, so the
/// lease actually changes HANDS rather than merely carrying a pending request.
/// Cut 5's `apply_due` shape, filtered by action id because a promoted domain
/// puts a `domaintick` candidate on the same page.
fn apply_due_handoff(tw: &mut TestWorld, token: u8) -> bool {
    let at = tw.world().world_ic_now().at;
    tw.world_mut().set_now(at);
    let Ok(page) =
        tw.engine_mut()
            .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 32)
    else {
        return false;
    };
    let Some(candidate) = page
        .candidates()
        .iter()
        .find(|c| c.action().key().id.as_str() == "control_handoff_apply")
        .cloned()
    else {
        return false;
    };
    let Ok(ingress) =
        mush_core::scheduled_action_ingress(candidate.action(), candidate.advance(), 1)
    else {
        return false;
    };
    let Ok(plan) = tw.engine_mut().plan_scheduled_action(&candidate, &ingress) else {
        return false;
    };
    let (_, scheduled_inv, _) = plan.into_parts();
    let Ok((prepared, _)) = tw
        .engine_mut()
        .prepare_registered_scheduled_authoritative_action(
            &candidate,
            &scheduled_inv,
            CommitToken::from_bytes([token; 16]),
        )
    else {
        return false;
    };
    let (_, installation) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(installation)
        .expect("the fixture installs");
    true
}

// ===========================================================================
// B0 — the driving ingress, through the real command -> offer -> window path
// ===========================================================================

/// **B0-6. A10's proxy arm in `handoff.rs` is exercised.** Cut 10's fold fixed
/// the branch and recorded honestly that it had **no test**: reproducing it
/// needs a lease the NPC already holds, which needs the whole scheduled apply
/// driven from a crate that can construct a proxy focus. This is that test.
///
/// The first activation's staged handoff is driven to completion AND applied, so
/// the NPC really holds the `control_leases` row. The station's declared
/// fallback is then pointed at a DIFFERENT NPC and the seat is re-taken. Before
/// A10 that projected a **relief to the fallback**, moving control away from the
/// seat the human had just taken; the arm keeps the acting NPC as `next_holder`,
/// so the kernel answers `NoControlChange` and the lease does not move.
///
/// *Recorded honestly (a correction to the fold's own wording):* the arm gives
/// "no relief", but it is **not** "no refusal" — `ControlLeaseError::NoControlChange`
/// maps to `ActionErrorCode::Stale` in `HandoffProvider::transition_error`, so
/// the staged request resolves and then refuses as a benign no-op. What A10
/// actually buys is that control **stays with the seat**, which is what the
/// assertions below measure.
#[test]
fn e7c11_b0_a10_activating_over_a_lease_the_npc_holds_keeps_control_on_the_seat() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);
    let (scout, watch) = single_channel_unit(&mut tw, &fx, op, "Scout");
    let movement = ChannelId::new("movement").expect("a channel");
    let seat = offer(&mut tw, &fx, op, scout, watch, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    let staged = drain(&mut tw);
    let handoff = staged
        .into_iter()
        .find(|s| s.actor == watch)
        .expect("the activation staged the NPC's own handoff request");
    let ops = drive_submit(&mut tw, &handoff, 21)
        .expect("the seat's handoff request projects and resolves");
    assert!(
        ops.iter().any(|op| matches!(
            &op.op,
            ModuleWrite::JsonSet { key, .. } if key.contains("/control_leases/")
        )),
        "the activation actually created the control lease"
    );
    // …and the scheduled apply moves it into the NPC's hands, which is the
    // precondition A10's arm needs: a lease the acting NPC ALREADY HOLDS.
    assert!(
        apply_due_handoff(&mut tw, 23),
        "the due control-handoff apply must run"
    );
    fire(&mut tw);
    assert_eq!(
        mush_sim::channel_lease_holder(tw.world(), scout, &movement),
        Some(PrincipalId::Object { id: watch }),
        "the NPC holds the lease before the second activation"
    );
    // Point the declared fallback somewhere ELSE, so a relief would be visible.
    ok(&mut tw, fx.driver, "+seat/leave");
    drain(&mut tw);
    ok(
        &mut tw,
        GOD,
        &format!("@station/fallback #{}/pilot=#{}", scout.0, fx.duce.0),
    );
    let again = offer(&mut tw, &fx, op, scout, watch, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, again.0),
    );
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, again.0),
    );
    let staged = drain(&mut tw);
    let handoff = staged
        .into_iter()
        .find(|s| s.actor == watch)
        .expect("the second activation staged the NPC's handoff too");
    // A10: the offer PROJECTS (before the fix this branch returned `Ok(None)`
    // whenever the fallback was the NPC, and a relief to the fallback whenever
    // it was not) …
    {
        let engine = tw.engine();
        let query = engine
            .legal_action_query(
                PrincipalId::Object { id: watch },
                watch,
                handoff.subject.clone(),
            )
            .expect("the NPC's own subject queries");
        let offers = engine.project_legal_actions(&query).expect("a projection");
        assert!(
            offers.projected_for_action(&handoff.action).is_some(),
            "A10: the activation's own offer must project over a lease the NPC holds"
        );
    }
    // … and driving it moves NOTHING: the lease stays with the seat rather than
    // being relieved to the declared fallback.
    let _ = drive_submit(&mut tw, &handoff, 22);
    let _ = apply_due_handoff(&mut tw, 24);
    fire(&mut tw);
    assert_eq!(
        mush_sim::channel_lease_holder(tw.world(), scout, &movement),
        Some(PrincipalId::Object { id: watch }),
        "A10: control must stay on the seat, never move to the declared fallback"
    );
    // The driver drives, which is the whole point of the arm.
    assert_eq!(granted(&tw, watch, scout, "pilot"), vec!["movement"]);
    assert_eq!(mush_sim::driving_actor(tw.world(), fx.driver), watch);
}

// ===========================================================================
// T11's nine acts — refused from the proxy, performed by the owner
// ===========================================================================

/// **B0-7. Every one of T11's nine acts refuses from the proxy seat**, and the
/// same act is available to the owner. The arms are enumerated by name in the
/// table so a tenth act cannot be added silently.
///
/// **Five** of the nine refuse by **inertness** — they are economy, org and staff
/// verbs keyed on `ctx.actor`, which the ingress deliberately never rewrites, so
/// the driver's refusal is **byte-identical before and after** activation (that
/// equality is the assertion, not merely that both refuse). The remaining
/// **four** ride the station lane the ingress opens and are refused by
/// `mush_sim::seat_restriction` itself, naming the act and the exit.
///
/// The split was six/three, then four/five after cut 11's independent verify
/// fold (D2) gave abandonment a station-lane face in `+eject` beside its economy
/// face `@asset/abandon`. 16b cut T retired `+eject` with the rest of the combat
/// lane (D-16-20) and the split went back to six/three; **16b cut K.2 gave
/// abandonment a new station-lane face, `EXIT`** — climbing out of a simulator
/// pod, which releases the seat and leaves the unit idling in a live match — so
/// it is five/four again. `EXIT`'s own arm is exercised in `tacsim`
/// (`crates/tacsim/tests/cockpit.rs` and `crates/mushd/tests/sim_lenses/sim_cockpit.rs`),
/// which is where the verb and its pod live; here the vocabulary's shape is what
/// is pinned. Abandonment keeps its economy face in the inert table below, and
/// that face is still inert: `@asset/abandon` is keyed on `ctx.actor`.
#[test]
fn e7c11_b0_t11_all_nine_acts_refuse_from_the_proxy_and_the_owner_still_acts() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);

    // (1) asset transfer, (2) abandonment's ECONOMY face, (3) scuttling,
    // (5) loadout/crew, (6) organization spending, (7) surrender of an unrelated
    // asset — the inert arms, measured before the seat exists. Abandonment is in
    // both halves since 16b cut K.2: `@asset/abandon` is keyed on `ctx.actor`
    // and is inert here, and `EXIT` is the station-lane face the guard refuses,
    // in `tacsim`.
    let inert: Vec<(&str, String)> = vec![
        (
            "asset_transfer",
            format!("@asset/transfer #{}=#{}", fx.atlas.0, fx.driver.0),
        ),
        (
            "abandonment_economy_face",
            format!("@asset/abandon #{}", fx.atlas.0),
        ),
        ("scuttling", format!("@asset/writeoff #{}", fx.atlas.0)),
        (
            "loadout_or_crew",
            format!("@crew/assign #{}/pilot=#{}", fx.atlas.0, fx.duce.0),
        ),
        (
            "organization_spending",
            format!("@vault/withdraw #{}=1", fx.atlas.0),
        ),
        (
            "unrelated_surrender",
            format!("@op/withdraw {}=#{}", op.0, fx.locust.0),
        ),
    ];
    let before: Vec<String> = inert
        .iter()
        .map(|(_, line)| err(&mut tw, fx.driver, line))
        .collect();

    seat_up(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    // The driver is genuinely driving — otherwise the nine refusals below prove
    // nothing at all (cut 10's `vf_a11`). The station-lane verb that used to
    // demonstrate it went with the combat lane (16b cut T), so the ingress's own
    // resolution is the observable: the driver's acts resolve as the NPC.
    assert_eq!(mush_sim::driving_actor(tw.world(), fx.driver), fx.ace);
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot"), vec!["movement"]);

    for ((name, line), was) in inert.iter().zip(before) {
        let now = err(&mut tw, fx.driver, line);
        assert_eq!(
            now, was,
            "T11 {name}: taking a seat must not change what the driver may do"
        );
    }

    // (4) a fallback change, (8) departure from the operation, (9) assumption of
    // another station — three of the four GUARDED arms, refused by
    // `seat_restriction` with the act and the exit named. The fourth,
    // (2) abandonment through `EXIT`, is `tacsim`'s verb on `tacsim`'s pod and
    // is exercised there.
    for (act, line) in [
        (
            mush_sim::ProxyAct::FallbackChange,
            format!("@station/fallback #{}/pilot=#{}", fx.atlas.0, fx.duce.0),
        ),
        (
            mush_sim::ProxyAct::OperationDeparture,
            format!("@journey/depart #{}=r1 #{}", fx.atlas.0, fx.field.0),
        ),
        (
            mush_sim::ProxyAct::StationAssumption,
            format!("+focus #{}/pilot", fx.locust.0),
        ),
    ] {
        let refused = err(&mut tw, fx.driver, &line);
        assert!(
            refused.contains(act.token()),
            "T11 {}: the refusal must name the act — got {refused}",
            act.token()
        );
        assert!(
            refused.contains("+seat/leave"),
            "T11 {}: the refusal must name the exit — got {refused}",
            act.token()
        );
    }

    // The three guarded acts belong to the OWNER, and the guard does not touch
    // them: staff still declares a fallback and departs, and an ordinary
    // billet-holder still focuses their own station.
    ok(
        &mut tw,
        GOD,
        &format!("@station/fallback #{}/pilot=#{}", fx.locust.0, fx.duce.0),
    );
    // …and after the seat goes back, the driver's own `+focus` is permitted
    // again: the guard narrows the SEAT, never the person.
    ok(&mut tw, fx.driver, "+seat/leave");
    drain(&mut tw);
    let unguarded = err(
        &mut tw,
        fx.driver,
        &format!("+focus #{}/pilot", fx.locust.0),
    );
    assert!(
        unguarded.contains("You hold no billet"),
        "the guard is gone and the ordinary billet refusal is back: {unguarded}"
    );

    // The vocabulary is closed and every arm above is covered.
    assert_eq!(mush_sim::ProxyAct::ALL.len(), 9);
    assert_eq!(mush_sim::ProxyAct::COUNT, 9);
    assert_eq!(
        mush_sim::ProxyAct::ALL
            .iter()
            .filter(|a| a.is_guarded())
            .count(),
        4,
        "exactly four acts ride the station lane the ingress opens; 16b cut K.2 gave abandonment \
         a station-lane face again in the arena's `EXIT`"
    );
}

/// **B0-8. The guard is asked on every arm it covers**, and on nobody else.
///
/// `seat_restriction` is `Ok(())` for an ordinary billet-holder, for staff at
/// their own console and for an NPC acting for itself — so a predicate that
/// accidentally refused everybody would fail here rather than merely look
/// strict.
#[test]
fn e7c11_b0_the_guard_refuses_only_a_live_proxy_driver() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    for act in mush_sim::ProxyAct::ALL {
        for who in [fx.commander, fx.driver, fx.bystander, fx.ace, GOD] {
            assert!(
                mush_sim::seat_restriction(tw.world(), who, act).is_ok(),
                "nobody is driving a proxy yet: {act:?} refused {who:?}"
            );
        }
    }
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);
    seat_up(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    for act in mush_sim::ProxyAct::ALL {
        assert!(
            mush_sim::seat_restriction(tw.world(), fx.driver, act).is_err(),
            "{act:?} must be refused from a live proxy seat"
        );
        // The NPC itself, the commander and a bystander are untouched: the
        // predicate narrows the DRIVER, never the crew or the world.
        for who in [fx.ace, fx.commander, fx.bystander, GOD] {
            assert!(
                mush_sim::seat_restriction(tw.world(), who, act).is_ok(),
                "{act:?} must not refuse {who:?}"
            );
        }
    }
    // The ingress is idempotent: resolving the NPC again is a fixed point, so no
    // chain of seats can hop from one NPC to another.
    let once = mush_sim::driving_actor(tw.world(), fx.driver);
    assert_eq!(once, fx.ace);
    assert_eq!(mush_sim::driving_actor(tw.world(), once), once);
}

/// **B0-9. The ingress publishes no key, mints no grant and adds no consumer.**
/// A source probe: `focus.rs` reaches no `ChannelGrant` API at all, so the C12
/// census (one writer, nine consumer files) cannot have moved, and the ingress
/// cannot have widened authority by construction.
#[test]
fn e7c11_b0_the_ingress_reaches_no_channel_grant_api() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("mush-sim/src/focus.rs");
    let text = std::fs::read_to_string(path).expect("focus.rs");
    let boundary = text.find("#[cfg(test)]").unwrap_or(text.len());
    for needle in [
        "command_subjects_of(",
        ".channel_grant(",
        "channel_grant_is_current(",
        "desired_links(",
    ] {
        assert!(
            !text[..boundary].contains(needle),
            "the ingress must reach no grant API: {needle}"
        );
    }
}

// A `SeatAudit` whose store refuses every write is the rig's `DeadAudit`: the
// seat cannot even be ACCEPTED, so no ingress can ever exist for it. Naming it
// here keeps the shared rig's whole surface exercised by this binary too.
#[test]
fn e7c11_b0_a_dead_audit_never_produces_a_drivable_seat() {
    let (mut tw, fx) = scene(Arc::new(DeadAudit));
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    let refused = err(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    assert!(
        refused.contains("Seat attribution is unavailable"),
        "{refused}"
    );
    assert!(mush_sim::proxy_drive_of(tw.world(), fx.driver).is_none());
    assert_eq!(mush_sim::driving_actor(tw.world(), fx.driver), fx.driver);
    let _ = IdempotencyKeyText::parse_canonical("e7c11.b0").expect("a canonical key");
    let _ = ChannelId::new("movement").expect("a channel");
    let _: Option<&dyn SeatAudit> = None;
}

// ===========================================================================
// ---- E7 cut 11 independent verify fold ----
//
// The seat-path half. Every test below drives the REAL command path — the verb,
// the offer, the ingress, the window — and asserts the observable outcome: what
// a resolver publishes for the driven NPC after a release, whether a grant is
// still reachable from `+seat/leave`, what `@station/show` renders. Owner
// decisions taken: F2=(c), F3=(a), F4=(a), F5=(a), F6=(a).
// ===========================================================================

/// **D1(a). `+focus/release` from a live seat is refused, and nothing of the
/// seat's give-back is skipped.**
///
/// `+focus/release` is the sibling switch of the very verb that carries guard
/// site 1, and cut 11 asked it nothing. Because a proxy driver's account has
/// exactly one foreground focus and it *is* the seat, the release dropped the
/// seat while doing none of a stand-down's work: `granted` was never cleared, so
/// the NPC's station stayed narrowed with nobody driving it, no
/// `sim.station.served` marker was written, and `+seat/leave` — the exit every
/// guard refusal names — then answered "You are driving no seat."
#[test]
fn vf_d1a_focus_release_from_a_seat_is_refused_and_the_seat_gives_back_whole() {
    use mush_economy::FactProvider;
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    drain(&mut tw);
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot"), vec!["movement"]);
    let began = tw
        .world()
        .module_state::<mush_sim::FocusState>()
        .and_then(|state| state.focus_of_actor(fx.ace).map(|f| f.acquired_at.at))
        .expect("the seat is live");
    // The guard: the act is named, and so is the exit.
    let refused = err(&mut tw, fx.driver, "+focus/release");
    assert!(
        refused.contains(mush_sim::ProxyAct::StationAssumption.token()),
        "the refusal must name the act: {refused}"
    );
    assert!(
        refused.contains("+seat/leave"),
        "the refusal must name the exit: {refused}"
    );
    // Nothing moved: the seat is still live and still narrowed.
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot"), vec!["movement"]);
    assert_eq!(mush_sim::driving_actor(tw.world(), fx.driver), fx.ace);

    // …and the sanctioned exit does the whole give-back the release skipped.
    let later = tw.world().now() + 600;
    tw.set_now(later);
    drain(&mut tw);
    ok(&mut tw, fx.driver, "+seat/leave");
    drain(&mut tw);
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()],
        "the narrowing is cleared: the station is back on its own channel set"
    );
    let served = mush_sim::SimFactProvider
        .resolve(
            tw.world(),
            "sim.station.served",
            &format!("{}/pilot/{}/{began}", fx.atlas.0, fx.ace.0),
            None,
        )
        .expect("the interval was recorded");
    assert!(
        served.satisfied,
        "the service marker is written on give-back"
    );
    assert!(matches!(
        grant_state(&tw, op, seat),
        Some(mush_campaign::GrantState::Revoked)
    ));
}

/// **D1(b). A released `Active` seat is never stranded behind staff.**
///
/// On a SINGLE-channel unit the K6 lease really lands and the campaign tick
/// promotes the grant to `Active { epoch }`. `+focus/release`'s
/// `prepare_focus_bump` then made that epoch stale, so `+seat/activate` answered
/// `no_seat()` and `+seat/leave` answered "You are driving no seat.": the seat
/// was reachable only by staff `@seat/revoke`. A test that asserts on `Dormant`
/// has not reached this hazard.
#[test]
fn vf_d1b_a_released_active_seat_is_never_stranded_behind_staff() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);
    let (scout, watch) = single_channel_unit(&mut tw, &fx, op, "Scout");
    let seat = offer(&mut tw, &fx, op, scout, watch, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    let handoff = drain(&mut tw)
        .into_iter()
        .find(|s| s.actor == watch)
        .expect("the activation staged the NPC's handoff request");
    drive_submit(&mut tw, &handoff, 51).expect("the handoff request resolves");
    assert!(apply_due_handoff(&mut tw, 53), "the due apply must run");
    fire(&mut tw);
    assert!(
        matches!(
            grant_state(&tw, op, seat),
            Some(mush_campaign::GrantState::Active { .. })
        ),
        "the hazard needs a PROMOTED grant, not a dormant one: {:?}",
        grant_state(&tw, op, seat)
    );

    let refused = err(&mut tw, fx.driver, "+focus/release");
    assert!(refused.contains("+seat/leave"), "{refused}");
    assert_eq!(granted(&tw, watch, scout, "pilot"), vec!["movement"]);
    // The exit every refusal names still works on a PROMOTED grant.
    ok(&mut tw, fx.driver, "+seat/leave");
    drain(&mut tw);
    assert!(matches!(
        grant_state(&tw, op, seat),
        Some(mush_campaign::GrantState::Revoked)
    ));
    assert!(mush_sim::proxy_drive_of(tw.world(), fx.driver).is_none());
}

/// **D5 / F6. Taking a second seat gives the first one back through the one
/// seam.**
///
/// `seat_activate`'s B14 short-circuit fires only for the SAME `(unit,
/// station)`, so a second activation displaced the incumbent focus while the
/// campaign side was never told: grant 1 stayed non-terminal with a live
/// `SeatAudit` row and `active_grant_of` resolved only grant 2, so grant 1 was
/// unreachable from `+seat/leave` for ever.
#[test]
fn vf_d5_taking_a_second_seat_gives_the_first_one_back_and_leaves_no_orphan() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);
    let first = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    let second = offer(&mut tw, &fx, op, fx.locust, fx.duce, "fire");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, first.0),
    );
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, first.0),
    );
    drain(&mut tw);
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot"), vec!["movement"]);
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, second.0),
    );
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, second.0),
    );
    drain(&mut tw);
    // Grant 1 is TERMINAL, not merely displaced…
    assert!(
        matches!(
            grant_state(&tw, op, first),
            Some(mush_campaign::GrantState::Revoked)
        ),
        "grant 1 must be stood down, not orphaned: {:?}",
        grant_state(&tw, op, first)
    );
    // …its narrowing is cleared…
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()]
    );
    // …it is unreachable as a seat again…
    let again = err(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, first.0),
    );
    assert!(again.contains("No such seat"), "{again}");
    // …and the second seat is the one the driver is on, and gives back too.
    assert_eq!(mush_sim::driving_actor(tw.world(), fx.driver), fx.duce);
    ok(&mut tw, fx.driver, "+seat/leave");
    drain(&mut tw);
    assert!(matches!(
        grant_state(&tw, op, second),
        Some(mush_campaign::GrantState::Revoked)
    ));
    assert_eq!(
        granted(&tw, fx.duce, fx.locust, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()]
    );
    assert!(mush_sim::proxy_drive_of(tw.world(), fx.driver).is_none());
}

/// **B4 / F4. An alt of the driver's account drives nothing, is guarded by
/// nothing, and cannot strand the seat either.**
///
/// The ingress was ACCOUNT-scoped: every character the oracle mapped to the
/// driver's account drove the seat, though none of them consented, appeared in
/// the offer's audience or held a `SeatAudit` token — and each was refused their
/// own unrelated `+focus` with the seat line. `ActiveFocus.driver` binds the
/// drive to the character that took the seat. The account's ONE focus slot is
/// still the account's, so the alt's `+focus` is refused — but for the slot,
/// naming no T11 act, and only until the driver stands down.
#[test]
fn vf_b4_an_alt_of_the_drivers_account_drives_nothing_and_is_guarded_by_nothing() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);
    let alt = tw.spawn_player("Alt");
    tw.world_mut().move_to(alt, fx.field).expect("the alt");
    fx.oracle.0.lock().unwrap().insert(
        alt,
        mush_core::AccountId::mint("DriverAcct", 2).expect("the driver's account"),
    );
    // The alt holds a billet of their own, on the OTHER unit.
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/unassign #{}=pilot", fx.locust.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", fx.locust.0, alt.0),
    );
    seat_up(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");

    // (a) the alt drives nothing at all, and every T11 act is open to them.
    assert!(mush_sim::proxy_drive_of(tw.world(), alt).is_none());
    assert_eq!(mush_sim::driving_actor(tw.world(), alt), alt);
    for act in mush_sim::ProxyAct::ALL {
        assert!(
            mush_sim::seat_restriction(tw.world(), alt, act).is_ok(),
            "{act:?} must not narrow a character who took no seat"
        );
    }
    // (b) the account's one focus slot is still occupied by the live seat, so
    // the alt's `+focus` is refused — naming the slot, never a T11 act — and the
    // seat survives untouched (this is D1's harm through a second door).
    let refused = err(&mut tw, alt, &format!("+focus #{}/pilot", fx.locust.0));
    assert!(
        refused.contains("is driving a seat"),
        "the refusal is about the account's focus slot: {refused}"
    );
    assert!(
        !refused.contains(mush_sim::ProxyAct::StationAssumption.token()),
        "…and never about a T11 act the alt was not granted: {refused}"
    );
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot"), vec!["movement"]);
    assert_eq!(mush_sim::driving_actor(tw.world(), fx.driver), fx.ace);

    // (c) once the seat goes back, the alt's own billet is theirs to focus.
    ok(&mut tw, fx.driver, "+seat/leave");
    drain(&mut tw);
    let taken = ok(&mut tw, alt, &format!("+focus #{}/pilot", fx.locust.0));
    assert!(taken.contains("You take foreground control"), "{taken}");
}

/// **B6 / F4. An account-mapped NPC is its own fixed point — by construction.**
///
/// "Idempotent by construction: an NPC maps to no account" was a property of the
/// account map's *contents*: `AccountMap` is hydrated from `list_accounts()`'s
/// characters with no `ObjKind` filter, so an NPC a builder links to an account
/// made `driving_actor(npcA)` answer npcB and `seat_restriction` refuse npcA's
/// own station acts. `proxy_drive_of` now refuses any caller that is not a
/// player.
#[test]
fn vf_b6_an_account_mapped_npc_is_its_own_fixed_point() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);
    seat_up(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    // Duce is the Locust's own NPC pilot; a builder links it to the account
    // that is currently driving Ace.
    fx.oracle.0.lock().unwrap().insert(
        fx.duce,
        mush_core::AccountId::mint("DriverAcct", 2).expect("the driver's account"),
    );
    assert_ne!(fx.duce, fx.ace);
    assert!(mush_sim::proxy_drive_of(tw.world(), fx.duce).is_none());
    assert_eq!(
        mush_sim::driving_actor(tw.world(), fx.duce),
        fx.duce,
        "an account-mapped NPC must not hop to another NPC"
    );
    for act in mush_sim::ProxyAct::ALL {
        assert!(
            mush_sim::seat_restriction(tw.world(), fx.duce, act).is_ok(),
            "{act:?} must not be refused to an NPC acting for itself"
        );
    }
    // The real driver is unaffected.
    assert_eq!(mush_sim::driving_actor(tw.world(), fx.driver), fx.ace);
}

/// **B5 / F4. A proxy focus cannot be installed from outside `mush-sim`.**
///
/// Cut 11 made `ActiveFocus { proxy: true }` an authority root — six readers
/// across three crates — while `FocusState::claim` was `pub` on a `pub`
/// re-export and `World::module_state_mut::<T>()` is generic and not
/// owner-scoped, so any composed module could install a focus that drove WIDER
/// than a real seat: the station's full channel set, no offer, no acceptance, no
/// audit token, no distinctness check. It is fenced the way its sibling
/// `ControlLeaseView::note_holder` already was, one file away.
#[test]
fn vf_b5_the_proxy_focus_claim_is_fenced_and_a_real_seat_narrows() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("mush-sim/src/focus.rs");
    let text = std::fs::read_to_string(path).expect("focus.rs");
    let boundary = text.find("#[cfg(test)]").unwrap_or(text.len());
    let production = &text[..boundary];
    assert!(
        production.contains("pub(crate) fn claim(&mut self, focus: ActiveFocus)"),
        "`FocusState::claim` must not be in the production surface"
    );
    assert!(
        !production.contains("pub fn claim(&mut self, focus: ActiveFocus)"),
        "…and must not be reachable unfenced"
    );
    assert!(
        production.contains("pub fn claim_for_test"),
        "the seeding door is the testkit shim, as `note_holder`'s is"
    );
    assert!(
        production.contains("#[cfg(any(test, feature = \"testkit\"))]"),
        "…and it is fenced behind the same feature"
    );

    // The runtime half: a REAL seat is narrowed to the offer's channels, which
    // is exactly what a forged focus bypassed.
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);
    seat_up(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["movement"],
        "the station declares two channels; the seat grants one"
    );
}

/// **B7. The authority probe covers the files that actually carry the
/// ingress.**
///
/// `e7c11_b0_the_ingress_reaches_no_channel_grant_api` scans only `focus.rs`,
/// which contained zero of its four needles before the cut — so it passed
/// byte-identically on the pre-cut tree and proved nothing. This pins the COUNT
/// of the grant reads in every file the ingress runs through rather than their
/// absence, and pins that `focus.rs` names the ingress it is supposed to carry.
///
/// The two reads the tactical lane carried — `combat.rs`'s `resolve_seat` and
/// `tick.rs`'s apply-time witness — went with it (16b cut T, D-16-20), so both
/// files leave the census: `combat.rs` does not exist and `tick.rs` reaches no
/// grant API at all.
#[test]
fn vf_b7_the_ingress_grant_reads_are_counted_where_they_actually_live() {
    let crates_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .to_path_buf();
    let read = |rel: &str| -> String {
        let text = std::fs::read_to_string(crates_dir.join(rel)).expect("a source file");
        let boundary = text.find("#[cfg(test)]").unwrap_or(text.len());
        text[..boundary].to_owned()
    };
    let focus = read("mush-sim/src/focus.rs");
    // Fails on the pre-cut tree: neither name existed in this file.
    assert!(focus.contains("fn driving_actor") && focus.contains("fn proxy_drive_of"));
    for needle in [
        "command_subjects_of(",
        ".channel_grant(",
        "channel_grant_is_current(",
        "desired_links(",
    ] {
        assert!(
            !focus.contains(needle),
            "the ingress itself must reach no grant API: {needle}"
        );
    }
    // The four files the ingress actually runs through, and the grant reads they
    // carry. A fifth reader breaks this.
    let counts: Vec<(&str, usize)> = [
        "mush-sim/src/tick.rs",
        "mush-sim/src/commands.rs",
        "mush-sim/src/station.rs",
        "mush-propagation/src/commands.rs",
    ]
    .into_iter()
    .map(|rel| {
        let text = read(rel);
        let n: usize = [
            "command_subjects_of(",
            ".channel_grant(",
            "channel_grant_is_current(",
        ]
        .into_iter()
        .map(|needle| text.matches(needle).count())
        .sum();
        (rel, n)
    })
    .collect();
    assert_eq!(
        counts,
        vec![
            ("mush-sim/src/tick.rs", 0),
            ("mush-sim/src/commands.rs", 1),
            ("mush-sim/src/station.rs", 1),
            ("mush-propagation/src/commands.rs", 0),
        ],
        "the ingress files' grant-API reads are pinned by COUNT, not by absence"
    );
}

/// **B8 / F2(c). Every station-lane verb has a recorded proxy disposition.**
///
/// `seat_restriction` is opt-in per call site: the tree carried four sites and
/// ZERO in `combat.rs` or `tick.rs`, so every verb the ingress opened was
/// unguarded unless somebody remembered — and two verbs shipped in cut 11 itself
/// (`+focus/release`, `+eject`) were exactly that hazard realized. This table is
/// the tripwire: a verb added to the lane without a decision fails here rather
/// than defaulting to unguarded.
///
/// 16b cut T (D-16-20) retired the eight combat verbs; **16b cut K.2 put eleven
/// back** — the arena's cockpit — so the table is the focus pair, the read-only
/// renders, the station/journey administration, and the cockpit's own ten
/// (`BOARD` is not on the lane: it is what puts somebody on it).
#[test]
fn vf_b8_every_station_lane_verb_has_a_recorded_proxy_disposition() {
    /// Guarded, and by which act — or reached and permitted, or inert.
    enum Disposition {
        /// Refused by `seat_restriction` with this act.
        Guarded(mush_sim::ProxyAct),
        /// A read-only render: it resolves through the ingress and writes
        /// nothing, so there is no act to guard.
        ReadOnly,
        /// **Reached from the seat and deliberately permitted** — the tactical
        /// discretion a seat IS (doc 10 §19 T11). It went with the eight combat
        /// verbs (16b cut T, D-16-20) and came back with the arena's cockpit
        /// (16b cut K.2): steering and shooting the unit you were lent is the
        /// whole of what a seat confers.
        Discretion,
    }
    use Disposition::{Discretion, Guarded, ReadOnly};

    let table: Vec<(&str, Disposition)> = vec![
        // The focus pair: both arms of `cmd_focus`'s switch.
        ("+focus", Guarded(mush_sim::ProxyAct::StationAssumption)),
        (
            "+focus/release",
            Guarded(mush_sim::ProxyAct::StationAssumption),
        ),
        // Read-only surfaces: they render through the ingress and write nothing.
        ("+stations", ReadOnly),
        ("@station/show", ReadOnly),
        // **16b cut K.2, the arena's cockpit.** Five stage one arena intent each
        // through the seat's own channel grant, and four render the last frame;
        // none of them writes a document, and every one of them is exactly the
        // tactical discretion the seat was lent. `EXIT` is the one that is not:
        // climbing out leaves the unit idling in a live match, which is T11's
        // "abandonment" word for word, so it carries the guard site — in
        // `tacsim`, which is why it is absent from the mush-sim census below.
        ("STARTUP", Discretion),
        ("HEADING", Discretion),
        ("SPEED", Discretion),
        ("LOCK", Discretion),
        ("FIRE", Discretion),
        ("VIEW", Discretion),
        ("CONTACTS", Discretion),
        ("WEAPONSTATUS", Discretion),
        ("STATUS", Discretion),
        ("EXIT", Guarded(mush_sim::ProxyAct::Abandonment)),
        // The station and journey administration.
        (
            "@station/fallback",
            Guarded(mush_sim::ProxyAct::FallbackChange),
        ),
        // **E7 residue pass 1, R6** (10b §15.1 item 10). `@journey/depart` is no
        // longer staff-only: the *permitting* half is now a billet/command test
        // (`mush_sim::commands::may_command_departure`), so the pilot billeted on a
        // hull departs it with no staff step. The disposition below is unchanged and
        // that is the point — the proxy guard runs **in front of** the new
        // permission, so widening who may depart never widens what a proxy seat may
        // do. `@journey/divert` keeps `require_admin` deliberately: it re-writes a
        // route already under way, and cut 6's `fail` path names it as the staff
        // recovery route for a stopped hull.
        (
            "@journey/depart",
            Guarded(mush_sim::ProxyAct::OperationDeparture),
        ),
        (
            "@journey/divert",
            Guarded(mush_sim::ProxyAct::OperationDeparture),
        ),
    ];

    // (1) Every act the table claims is guarded must actually BE guarded, and
    // every guarded act must appear at least once in the table.
    let mut named: Vec<mush_sim::ProxyAct> = Vec::new();
    let mut permitted = 0usize;
    for (verb, disposition) in &table {
        match disposition {
            Guarded(act) => {
                assert!(
                    act.is_guarded(),
                    "{verb} claims {act:?}, which `is_guarded()` denies"
                );
                if !named.contains(act) {
                    named.push(*act);
                }
            }
            ReadOnly | Discretion => permitted += 1,
        }
    }
    assert_eq!(
        permitted, 11,
        "eleven station-lane verbs are reached and permitted: two renders and the cockpit's nine"
    );
    for act in mush_sim::ProxyAct::ALL.iter().filter(|a| a.is_guarded()) {
        assert!(
            named.contains(act),
            "{act:?} is guarded but no station-lane verb in the table carries it"
        );
    }
    assert_eq!(named.len(), 4, "four guarded acts ride the station lane");
    assert_eq!(
        mush_sim::ProxyAct::ALL
            .iter()
            .filter(|a| !a.is_guarded())
            .count(),
        5,
        "five refuse by inertness"
    );

    // (2) The source census the table stands for: the guard's call sites, by
    // file. A new `seat_restriction(` anywhere — or one that disappears — is a
    // decision that has to be recorded here and in 10b §3.11.
    let crates_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .to_path_buf();
    let mut sites: Vec<(String, usize)> = Vec::new();
    for rel in [
        "mush-sim/src/commands.rs",
        "mush-sim/src/station.rs",
        "mush-sim/src/tick.rs",
        "mush-sim/src/seat.rs",
        "mush-propagation/src/commands.rs",
        // 16b cut K.2: the guard travelled out of `mush-sim` for the first time,
        // because the verb that carries it did — `EXIT` is the arena's.
        "tacsim/src/match_/cockpit/notices.rs",
    ] {
        let text = std::fs::read_to_string(crates_dir.join(rel)).expect("a source file");
        let boundary = text.find("#[cfg(test)]").unwrap_or(text.len());
        let n = text[..boundary]
            .lines()
            .filter(|line| {
                let code = line.trim_start();
                !code.starts_with("//") && code.contains("seat_restriction(")
            })
            .count();
        sites.push((rel.to_owned(), n));
    }
    assert_eq!(
        sites,
        vec![
            // `+focus` (1) and `+focus/release` (1), plus each arm's own
            // account-slot precondition (`account_proxy_seat`) is a separate
            // predicate and not counted here.
            ("mush-sim/src/commands.rs".to_owned(), 4),
            // `@station/fallback`.
            ("mush-sim/src/station.rs".to_owned(), 1),
            // The two tick-time rechecks of `+eject`'s abandonment went with the
            // retired combat lane (16b cut T, D-16-20), which took `combat.rs`
            // out of this census entirely.
            ("mush-sim/src/tick.rs".to_owned(), 0),
            ("mush-sim/src/seat.rs".to_owned(), 0),
            ("mush-propagation/src/commands.rs".to_owned(), 0),
            // `EXIT` (16b cut K.2): `ProxyAct::Abandonment`'s station-lane face,
            // back after one cut without one.
            ("tacsim/src/match_/cockpit/notices.rs".to_owned(), 1),
        ],
        "the guard's call sites, by file — a new station-lane verb must join the \
         table above and this census in the same commit"
    );
}

/// **D8. A driver can see what they drive.**
///
/// `@station/show` and `+stations` read `FocusState`'s ACTOR-keyed accessor, and
/// a proxy focus is keyed on the NPC — so the two surfaces answered "You are
/// driving no station." and rendered `[FOCUSED]` on nothing, while `+focus`
/// reported the seat. Three surfaces, three answers. An unseated actor's output
/// is byte-unchanged.
#[test]
fn vf_d8_a_driver_can_see_what_they_drive_and_an_unseated_actor_cannot() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);
    let before_show = ok(&mut tw, fx.driver, "@station/show");
    let before_list = ok(&mut tw, fx.driver, "+stations");
    seat_up(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");

    let show = ok(&mut tw, fx.driver, "@station/show");
    assert!(
        show.contains("Ace") && show.contains("pilot") && show.contains("Atlas"),
        "the seat, and the represented NPC, are named: {show}"
    );
    let list = ok(&mut tw, fx.driver, "+stations");
    assert!(
        list.contains("[FOCUSED]") && list.contains("Ace"),
        "the driven seat appears in the list: {list}"
    );
    let focus = ok(&mut tw, fx.driver, "+focus");
    assert!(focus.contains("Atlas"), "{focus}");

    // The bystander drives nothing, and nothing about their render moved.
    assert_eq!(ok(&mut tw, fx.bystander, "@station/show"), before_show);
    assert_eq!(ok(&mut tw, fx.bystander, "+stations"), before_list);
    ok(&mut tw, fx.driver, "+seat/leave");
    drain(&mut tw);
    assert_eq!(ok(&mut tw, fx.driver, "@station/show"), before_show);
    assert_eq!(ok(&mut tw, fx.driver, "+stations"), before_list);
}

/// **G7. `seatstate::seat_holder` is a RAW map read, not a derived-index read.**
///
/// §14.1 said `mush-campaign`'s two L2 sites go "through the derived
/// `Derived::seats_by_actor` index rather than the map itself, which is what a
/// projection-shaped migration preserves for free". Both halves are wrong: the
/// two sites are the direct `SpatialState.assignments[host].seats[station]`
/// walk, and it is the one cross-crate read a projection-shaped migration does
/// NOT get for free. (The `mush-propagation` site reads
/// `Derived::assignments_by_actor`, a different index from the one §14 named.)
#[test]
fn vf_g7_seat_holder_is_a_raw_map_read_the_migration_must_re_point() {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/seatstate.rs"),
    )
    .expect("seatstate.rs");
    let boundary = text.find("#[cfg(test)]").unwrap_or(text.len());
    let production = &text[..boundary];
    assert!(
        !production.contains(".derived."),
        "seatstate.rs reaches NO derived index — §14.1's L2 note is wrong"
    );
    assert!(
        production.contains(".assignments") && production.contains(".seats.get("),
        "the L2 read is the raw two-level map walk in `seat_holder`"
    );
    assert_eq!(
        production.matches(".assignments").count(),
        1,
        "exactly one raw `SpatialState.assignments` walk in this file"
    );

    // …and it answers the same seat the derived index does, today. The day the
    // map moves, these two diverge and the test names the read that was missed.
    let (tw, fx) = scene(Arc::new(TestAudit::default()));
    let station = mush_sim::StationId::new("pilot").expect("a station");
    let derived = tw
        .world()
        .module_state::<mush_sim::SpatialState>()
        .and_then(|state| state.derived.seats_by_actor.get(&fx.ace).cloned())
        .unwrap_or_default();
    assert!(
        derived.contains(&(fx.atlas, station)),
        "the derived index seats Ace on Atlas/pilot"
    );
}
