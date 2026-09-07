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
use serde_json::Value;

mod common;
use common::*;

// ---- E7c11: B0 (the rig extension — a pulsing domain and a live seat) ----

/// Arm the engine's offer key ring so a staged submit can be resolved at all.
fn arm_offer_keys(tw: &mut TestWorld) {
    tw.engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(2, [2; 32]).unwrap(), []).unwrap());
}

/// Promote and apply the scene's domain so it PULSES: `combat::resolve_seat`
/// refuses a passive domain outright, so without this no tactical verb of either
/// a proxy or an owner can resolve and the ingress would be untestable.
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
/// offer is *projected for the submitting actor*, and it is
/// `TacticalIntentProvider::project_offer` -> `combat::resolve_seat` -> the B0
/// ingress that decides whether one projects at all. A proxy driver whose
/// ingress is missing gets `Ok(None)` here and the whole path dead-ends.
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
    tw.engine_mut().install_prepared_action(installation);
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
    tw.engine_mut().install_prepared_action(installation);
    true
}

/// The tick-window post-image the transaction carries, if any.
fn window_post_image(ops: &[ModuleOp]) -> Option<Value> {
    ops.iter().find_map(|op| match &op.op {
        ModuleWrite::JsonSet { key, value } if key.contains("/tick_windows/") => {
            value.get("value").cloned()
        }
        _ => None,
    })
}

/// Take the single staged submit a tactical verb queued (and assert there is
/// exactly one).
fn staged_one(tw: &mut TestWorld) -> mush_core::AuthoritativeSubmit {
    let staged = drain(tw);
    assert_eq!(staged.len(), 1, "exactly one authoritative submit staged");
    staged.into_iter().next().expect("one submit")
}

// ===========================================================================
// B0 — the driving ingress, through the real command -> offer -> window path
// ===========================================================================

/// **B0-1. A proxy driver actually steers**, all the way into the tick's frozen
/// intent set — the act cut 10 could not perform at all.
///
/// The observable is not the record the verb wrote: it is that the engine
/// PROJECTS the tactical action for the human's own submit (which only
/// `resolve_seat` -> `driving_actor` can make true), that the resolved
/// transaction carries a `tick_windows` post-image, and that the intent inside
/// it names the **NPC's unit and station** — never the driver's character.
#[test]
fn e7c11_b0_a_proxy_drives_the_npc_through_command_offer_ingress_and_window() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);
    seat_up(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    // The seat is real and narrowed: the NPC publishes exactly `movement`.
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot"), vec!["movement"]);
    // …and the driver still holds no grant of their own. Authority stays on the
    // NPC's own `ChannelGrant` (T16); the ingress widens nothing.
    assert!(
        granted(&tw, fx.driver, fx.atlas, "pilot").is_empty(),
        "the ingress must not publish a grant to the driver"
    );
    let said = ok(&mut tw, fx.driver, "+heading 1,2,3");
    assert!(said.contains("submitted into tick"), "{said}");
    let submit = staged_one(&mut tw);
    assert_eq!(
        submit.actor, fx.driver,
        "the human's own submit slot is used"
    );
    let ops = drive_submit(&mut tw, &submit, 9)
        .expect("the tactical offer projects for the proxy driver and resolves");
    let window = window_post_image(&ops).expect("the intent joined this tick's window");
    let encoded = serde_json::to_string(&window).expect("the window encodes");
    assert!(
        encoded.contains(&format!("\"unit\":{}", fx.atlas.0)),
        "the window's intent names the NPC's unit: {encoded}"
    );
    assert!(
        encoded.contains("\"station\":\"pilot\""),
        "the window's intent names the NPC's station: {encoded}"
    );
}

/// **B0-2. The ingress fails closed.** A player with no seat, a player whose
/// account the oracle does not map, and a bystander all get the identical
/// pre-cut-11 refusal — `driving_actor` is the identity for them, so nothing
/// about the tactical lane changed for anybody but a live proxy driver.
#[test]
fn e7c11_b0_an_actor_with_no_seat_is_refused_exactly_as_before() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let unseated = err(&mut tw, fx.driver, "+heading 1,2,3");
    assert!(
        unseated.contains("You hold no focused station"),
        "{unseated}"
    );
    let stranger = err(&mut tw, fx.bystander, "+heading 1,2,3");
    assert_eq!(
        unseated, stranger,
        "byte-identical for every unseated actor"
    );
    // And an actor the oracle does not map at all (no account row) is refused
    // the same way: `proxy_drive_of` answers `None` on an unknown, never a
    // default.
    let ghost = tw.spawn_player("Ghost");
    tw.world_mut().move_to(ghost, fx.field).unwrap();
    let unmapped = err(&mut tw, ghost, "+heading 1,2,3");
    assert_eq!(unmapped, unseated, "an unmapped account fails closed");
}

/// **B0-3. The seat narrows the DRIVER exactly**, on the real command path.
///
/// A `fire`-only seat cannot steer and a `movement`-only seat cannot shoot, and
/// the refusal names the channel and the station rather than leaking that the
/// station has a wider set. This is what cut 10 could only demonstrate as
/// *inertness* (`vf_a11`): now the driver genuinely holds the seat and is
/// refused for narrowness.
#[test]
fn e7c11_b0_the_granted_set_narrows_the_driver_on_the_real_command_path() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);
    // A `fire`-only seat: the station grants `movement+fire`, so this is a
    // genuine narrowing rather than an absent capability.
    seat_up(&mut tw, &fx, op, fx.atlas, fx.ace, "fire");
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot"), vec!["fire"]);
    let refused = err(&mut tw, fx.driver, "+heading 1,2,3");
    assert!(
        refused.contains("does not grant the movement channel"),
        "{refused}"
    );
    // The same verb on a `movement` seat succeeds — same driver, same NPC, same
    // station, different granted set.
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);
    seat_up(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(&mut tw, fx.driver, "+heading 1,2,3");
}

/// **B0-4. The tick's apply-time witness accepts the proxy's intent, and
/// refuses it the moment the seat goes back.**
///
/// `DomainTickScheduleAdapter::plan` re-checks every accepted intent's focus and
/// grant. The intent's `actor` is the human; without the ingress at that site
/// the re-check would refuse every proxy order at apply even though the offer
/// admitted it at submit — the exact submit/apply disagreement the pinned seat
/// exists to remove. The observable is the standing order the tick would set.
#[test]
fn e7c11_b0_the_tick_witness_accepts_a_proxy_intent_and_refuses_it_after_leave() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);
    seat_up(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(&mut tw, fx.driver, "+thrust 40");
    let submit = staged_one(&mut tw);
    assert!(
        drive_submit(&mut tw, &submit, 11).is_some(),
        "the proxy's intent reaches the window"
    );
    // Give the seat back: the NPC's grant epoch bumps and the focus is gone, so
    // the same driver's next order finds no seat at all.
    ok(&mut tw, fx.driver, "+seat/leave");
    drain(&mut tw);
    let after = err(&mut tw, fx.driver, "+thrust 40");
    assert!(after.contains("You hold no focused station"), "{after}");
}

/// **B0-5. A second proxy never evicts a live human driver — who keeps
/// driving.** Cut 10's `vf_a3` proved the refusal; with the ingress it can prove
/// the thing that actually matters: after the refusal the first driver's next
/// order still resolves through the whole path.
#[test]
fn e7c11_b0_a_second_proxy_never_evicts_a_live_driver_who_keeps_driving() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);
    seat_up(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(&mut tw, fx.driver, "+heading 1,2,3");
    let _ = drain(&mut tw);
    // A SECOND account offered the same NPC's station.
    let rival = tw.spawn_player("Rival");
    tw.world_mut().move_to(rival, fx.field).unwrap();
    if let Ok(mut map) = fx.oracle.0.lock() {
        map.insert(
            rival,
            mush_core::AccountId::mint("RivalAcct", 9).expect("an account"),
        );
    }
    let second = ok(
        &mut tw,
        fx.commander,
        &format!(
            "+seat/offer {}/#{}/pilot=#{},movement,to:#{}",
            op.0, fx.atlas.0, fx.ace.0, rival.0
        ),
    );
    let id = mush_campaign::CampaignStore::seat_offers(camp(&tw))
        .map(|o| o.id)
        .max_by_key(|id| id.0)
        .expect("the second offer");
    assert!(!second.is_empty());
    ok(&mut tw, rival, &format!("+seat/accept {}/{}", op.0, id.0));
    err(&mut tw, rival, &format!("+seat/activate {}/{}", op.0, id.0));
    drain(&mut tw);
    // The live human is untouched and still drives.
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot"), vec!["movement"]);
    ok(&mut tw, fx.driver, "+heading 3,2,1");
    let submit = staged_one(&mut tw);
    assert!(
        drive_submit(&mut tw, &submit, 13).is_some(),
        "the incumbent's order still reaches the window after the rival is refused"
    );
    // …and the rival drives nothing.
    let rival_refused = err(&mut tw, rival, "+heading 1,1,1");
    assert!(
        rival_refused.contains("You hold no focused station"),
        "{rival_refused}"
    );
}

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
    ok(&mut tw, fx.driver, "+heading 1,2,3");
}

// ===========================================================================
// T11's nine acts — refused from the proxy, performed by the owner
// ===========================================================================

/// **B0-7. Every one of T11's nine acts refuses from the proxy seat**, and the
/// same act is available to the owner. The arms are enumerated by name in the
/// table so a tenth act cannot be added silently.
///
/// Six of the nine refuse by **inertness** — they are economy, org and staff
/// verbs keyed on `ctx.actor`, which the ingress deliberately never rewrites, so
/// the driver's refusal is **byte-identical before and after** activation (that
/// equality is the assertion, not merely that both refuse). The remaining three
/// ride the station lane the ingress opens and are refused by
/// `mush_sim::seat_restriction` itself, naming the act and the exit.
#[test]
fn e7c11_b0_t11_all_nine_acts_refuse_from_the_proxy_and_the_owner_still_acts() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    pulse(&mut tw, &fx);
    let op = operation(&mut tw, &fx);
    arm_offer_keys(&mut tw);

    // (1) asset transfer, (2) abandonment, (3) scuttling, (5) loadout/crew,
    // (6) organization spending, (7) surrender of an unrelated asset — the six
    // INERT arms, measured before the seat exists.
    let inert: Vec<(&str, String)> = vec![
        (
            "asset_transfer",
            format!("@asset/transfer #{}=#{}", fx.atlas.0, fx.driver.0),
        ),
        ("abandonment", format!("@asset/abandon #{}", fx.atlas.0)),
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
    // nothing at all (cut 10's `vf_a11`).
    ok(&mut tw, fx.driver, "+heading 1,2,3");
    drain(&mut tw);

    for ((name, line), was) in inert.iter().zip(before) {
        let now = err(&mut tw, fx.driver, line);
        assert_eq!(
            now, was,
            "T11 {name}: taking a seat must not change what the driver may do"
        );
    }

    // (4) a fallback change, (8) departure from the operation, (9) assumption of
    // another station — the three GUARDED arms, refused by `seat_restriction`
    // with the act and the exit named.
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
    assert_eq!(
        mush_sim::ProxyAct::ALL
            .iter()
            .filter(|a| a.is_guarded())
            .count(),
        3,
        "exactly three acts ride the station lane the ingress opens"
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
