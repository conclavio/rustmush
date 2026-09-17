//! E7 cut 10 acceptance (10b §3.10): **a proxy seat that reaches exactly what it
//! was granted, gives the seat back at a boundary, and can never be traced to an
//! account.**
//!
//! Everything runs through the real `Engine` via `TestWorld`, on the production
//! composition order (org → faction → canon → sim → economy → campaign), and
//! every command goes through the real **command → handler → `mush-sim` seam →
//! world** path. A test that only reads back the document it just wrote does not
//! count: each asserts what a second actor would actually **see** or be **able to
//! do** — the channels the resolver publishes, what
//! `World::channel_grant_is_current` answers per channel, the refusal bytes a
//! non-commander receives, the seat another player finds occupied.
//!
//! The three lenses this cut is verified on:
//!
//! - **G (guardrail escape)** — can a proxy reach any authority outside its
//!   channel set, area or operation?
//! - **R (revocation and boundary)** — can control persist past a revocation, be
//!   lost mid-tick, or be handed off before the boundary?
//! - **A (audit irreversibility)** — can the account behind a driver token be
//!   recovered by anything but the staff verb, or reach a journal, contract or
//!   canon record?

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_campaign::{GrantState, OperationId, SeatOfferId};
use mush_core::{ChannelId, GOD, ObjId, ObjKind, SeatAudit};
use mush_testkit::TestWorld;
use smol_str::SmolStr;

// ---- rig ------------------------------------------------------------------
//
// **E7 cut 11 B0.** The rig moved to `tests/common/mod.rs` VERBATIM (visibility
// only) so cut 11's driving-ingress suite drives the same scene, the same NPCs
// and the same seat offers rather than a second, subtly different world. Every
// test below is byte-unchanged.
mod common;
use common::*;

// ===========================================================================
// Hazard axis 1 — a proxy escaping its scope (G)
// ===========================================================================

/// **G1. A granted set narrower than the station's limits the proxy EXACTLY.**
///
/// The observable is not the record: it is the channel set a resolver publishes
/// for the driven NPC, and what the kernel's own `channel_grant_is_current`
/// answers per channel. An action requiring `fire` finds no grant carrying it,
/// so `select_offer_channel_grant` returns none and the offer is never projected.
#[test]
fn e7c10_g1_a_narrower_granted_set_limits_the_proxy_exactly() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    // Before the seat: the AI NPC holds the WHOLE station.
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()]
    );
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
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["movement".to_owned()],
        "the seat publishes the intersection, not the station's whole set"
    );
    let epoch = epoch_of(&tw, fx.ace, fx.atlas, "pilot").expect("a grant");
    assert!(is_current(
        &tw, fx.ace, fx.atlas, "pilot", epoch, "movement"
    ));
    assert!(
        !is_current(&tw, fx.ace, fx.atlas, "pilot", epoch, "fire"),
        "an ungranted channel is refused by the kernel's own grant check"
    );
    // And the staff inspector — one internal view feeds every projection —
    // renders the same narrowed set.
    let shown = ok(&mut tw, GOD, &format!("@spatial/authority #{}", fx.ace.0));
    assert!(shown.contains("movement"), "{shown}");
    assert!(!shown.contains("fire"), "{shown}");
}

/// **G2. A proxy cannot assume a SECOND station** — cut 5's one-focus rule plus
/// the grant's own `(unit, station)` binding, tested on both arms.
#[test]
fn e7c10_g2_a_proxy_cannot_assume_a_second_station() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let first = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    let second = offer(&mut tw, &fx, op, fx.locust, fx.duce, "movement");
    for seat in [first, second] {
        ok(
            &mut tw,
            fx.driver,
            &format!("+seat/accept {}/{}", op.0, seat.0),
        );
    }
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, first.0),
    );
    drain(&mut tw);
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, second.0),
    );
    drain(&mut tw);
    // Arm 1 — the ACCOUNT holds exactly one foreground focus, so taking the
    // second surrendered the first: the observable is that Ace is no longer
    // driven by anybody and Duce is.
    let focus = tw
        .world()
        .module_state::<mush_sim::FocusState>()
        .expect("focus state");
    assert_eq!(focus.len(), 1, "one account, one foreground focus");
    assert_eq!(
        focus.focus_of_actor(fx.duce).map(|f| f.unit),
        Some(fx.locust)
    );
    assert!(
        focus.focus_of_actor(fx.ace).is_none(),
        "the first seat was surrendered, not held alongside"
    );
    // Arm 2 — and the surrendered station went back to its declared fallback
    // over the STATION's own channels, not the seat's narrowed set.
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()],
        "the released seat's narrowing is cleared, not left behind"
    );
}

/// **G3. `+seat/offer` is EXISTENCE-HIDDEN from a non-commander** (hazard axis 2):
/// the refusal is byte-identical whether the unit and station exist or not.
#[test]
fn e7c10_g3_a_seat_offer_by_a_non_commander_is_existence_hidden() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let real = err(
        &mut tw,
        fx.bystander,
        &format!(
            "+seat/offer {}/#{}/pilot=#{},movement",
            op.0, fx.atlas.0, fx.ace.0
        ),
    );
    let absent = err(
        &mut tw,
        fx.bystander,
        &format!(
            "+seat/offer {}/#{}/nosuchstation=#{},movement",
            op.0, fx.atlas.0, fx.ace.0
        ),
    );
    assert_eq!(
        real, absent,
        "an existing station and a missing one must refuse byte-identically"
    );
    assert!(real.contains(mush_campaign::NO_SUCH_SEAT), "{real}");
    // And the commander really can do it, so the refusal is authority and not a
    // parse failure.
    ok(
        &mut tw,
        fx.commander,
        &format!(
            "+seat/offer {}/#{}/pilot=#{},movement",
            op.0, fx.atlas.0, fx.ace.0
        ),
    );
}

/// **G4. `@crew` is existence-hidden the same way, on every arm.**
#[test]
fn e7c10_g4_the_crew_verbs_are_existence_hidden_on_every_arm() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    for line in [
        format!("@crew/complement #{}", fx.atlas.0),
        format!("@crew/readiness #{}=50", fx.atlas.0),
        format!("@crew/assign #{}/pilot=#{}", fx.atlas.0, fx.ace.0),
    ] {
        let refused = err(&mut tw, fx.bystander, &line);
        assert!(
            refused.contains(mush_campaign::NO_SUCH_CREW),
            "{line}: {refused}"
        );
    }
    // The commander controls the unit and every arm succeeds.
    ok(
        &mut tw,
        fx.commander,
        &format!("@crew/complement #{}", fx.atlas.0),
    );
    ok(
        &mut tw,
        fx.commander,
        &format!("@crew/readiness #{}=50", fx.atlas.0),
    );
    ok(
        &mut tw,
        fx.commander,
        &format!("@crew/assign #{}/pilot=#{}", fx.atlas.0, fx.ace.0),
    );
}

/// **G5. Ordinary tactical loss stays permitted** — the seat is not made safe.
/// A driven unit is still destructible by exactly the paths that could destroy
/// it before, and a source probe pins that cut 10 adds no protection.
#[test]
fn e7c10_g5_ordinary_tactical_loss_is_still_permitted() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
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
    // The unit is still exactly as exposed as it was: staff can unplace it, and
    // the seat's narrowed grant goes with it (cut 2's liveness gate still runs
    // ahead of cut 10's intersection term).
    ok(&mut tw, GOD, &format!("@spatial/unplace #{}", fx.atlas.0));
    assert!(
        granted(&tw, fx.ace, fx.atlas, "pilot").is_empty(),
        "an unplaced unit grants nothing, seat or no seat"
    );
}

/// **G6. The seat grants NO economic or organizational authority.** T11's asset
/// transfer, organization spending and surrender arms are refused not by a new
/// check but because a seat confers control channels and nothing else: the
/// driver's own authority is byte-identical before and after activation.
#[test]
fn e7c10_g6_a_seat_confers_no_economic_or_org_authority() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let before = err(&mut tw, fx.driver, &format!("@op/resolve {}=forfeit", op.0));
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
    let after = err(&mut tw, fx.driver, &format!("@op/resolve {}=forfeit", op.0));
    assert_eq!(
        before, after,
        "taking a seat must not widen what the driver may do to the operation"
    );
    // The same on the roster: a driver cannot mutate the crew of the unit they
    // drive, and the refusal is the existence-hiding one.
    let refused = err(
        &mut tw,
        fx.driver,
        &format!("@crew/assign #{}/pilot=#{}", fx.atlas.0, fx.duce.0),
    );
    assert!(refused.contains(mush_campaign::NO_SUCH_CREW), "{refused}");
    // And a proxy cannot change the station's declared fallback (T11's fallback
    // arm): `@station/fallback` is staff's, and the seat confers no staff role.
    err(
        &mut tw,
        fx.driver,
        &format!("@station/fallback #{}/pilot=#{}", fx.atlas.0, fx.duce.0),
    );
}

// ===========================================================================
// The K6 gate — two policy refusals plus one kernel refusal
// ===========================================================================

/// **K1 (leg 1). A station held by somebody who is not the grant's own declared
/// fallback refuses, and E7 ships no eviction path.** The observable: the
/// incumbent still holds the billet and still publishes its grant afterwards.
#[test]
fn e7c10_k1_a_station_held_by_another_refuses_and_nothing_is_evicted() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    // The offer names Duce as its fallback, but Ace holds the Atlas pilot seat.
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.duce, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let refused = err(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    assert!(refused.contains("held by another controller"), "{refused}");
    // NOTHING was evicted: Ace still holds the seat and still publishes the
    // station's whole channel set.
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()]
    );
    assert!(
        granted(&tw, fx.duce, fx.atlas, "pilot").is_empty(),
        "the refused seat published nothing"
    );
    assert_eq!(
        grant_state(&tw, op, seat),
        Some(GrantState::Dormant),
        "a refused activation leaves the grant dormant, never Active"
    );
}

/// **K2. Accepting seizes NOTHING** — the grant is `Dormant`, no focus is taken,
/// no billet moves and the incumbent's grant is byte-identical.
#[test]
fn e7c10_k2_accepting_a_seat_seizes_nothing() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let before = granted(&tw, fx.ace, fx.atlas, "pilot");
    let epoch = epoch_of(&tw, fx.ace, fx.atlas, "pilot");
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    assert_eq!(grant_state(&tw, op, seat), Some(GrantState::Dormant));
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot"), before);
    assert_eq!(epoch_of(&tw, fx.ace, fx.atlas, "pilot"), epoch);
    assert_eq!(
        tw.world()
            .module_state::<mush_sim::FocusState>()
            .map(mush_sim::FocusState::len),
        Some(0),
        "accepting takes no focus at all"
    );
}

/// **K3. A grant whose audit row is missing refuses activation and pages** — it
/// never activates un-attributed (R3/R7).
#[test]
fn e7c10_k3_a_grant_with_no_audit_row_refuses_activation() {
    // The store refuses every write, so the seat cannot even be taken.
    let (mut tw, fx) = scene(Arc::new(DeadAudit));
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    let refused = err(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    assert!(refused.contains("attribution"), "{refused}");
    assert!(
        camp(&tw).seat_grant(op, seat).is_none(),
        "no grant is created un-attributed"
    );
}

/// **K3b. A dormant grant whose row is forgotten across a reboot refuses
/// activation** — the audit log is server-side and process-local, so a restart
/// is exactly the case the runbook is written for.
#[test]
fn e7c10_k3b_a_dormant_grant_refuses_activation_after_its_row_is_gone() {
    let audit = Arc::new(TestAudit::default());
    let (mut tw, fx) = scene(audit.clone());
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let token = camp(&tw)
        .seat_grant(op, seat)
        .expect("a grant")
        .driver
        .0
        .to_string();
    assert!(audit.knows(&token));
    // Simulate the restart: the row is gone, the document is not.
    assert!(audit.forget(&token));
    let refused = err(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    assert!(refused.contains("no attribution"), "{refused}");
    assert_eq!(grant_state(&tw, op, seat), Some(GrantState::Dormant));
    assert!(
        granted(&tw, fx.ace, fx.atlas, "pilot").len() == 2,
        "nothing moved: the incumbent still holds the whole station"
    );
}

// ===========================================================================
// Hazard axis — revocation and boundary (R)
// ===========================================================================

/// **R1. Revocation bumps the epoch immediately, and every intent captured under
/// the old grant fails stale.** The seat returns to its declared fallback over
/// the station's own channels; the record cannot be re-activated.
#[test]
fn e7c10_r1_revocation_stales_every_captured_intent_and_returns_the_seat() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
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
    let captured = epoch_of(&tw, fx.ace, fx.atlas, "pilot").expect("a grant");
    assert!(is_current(
        &tw, fx.ace, fx.atlas, "pilot", captured, "movement"
    ));
    drain(&mut tw);
    ok(
        &mut tw,
        fx.commander,
        &format!("@seat/revoke {}/{}", op.0, seat.0),
    );
    drain(&mut tw);
    // Every queued intent under the old grant — human, NPC, pathfinder or LLM —
    // now fails the ONE predicate they all consult.
    assert!(
        !is_current(&tw, fx.ace, fx.atlas, "pilot", captured, "movement"),
        "a revocation must stale every intent captured under the old grant"
    );
    // The narrowing is cleared: AI control resumes on the station's own set.
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()]
    );
    assert_eq!(grant_state(&tw, op, seat), Some(GrantState::Revoked));
    // A revoked grant can never be re-activated.
    let refused = err(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    assert!(refused.contains(mush_campaign::NO_SUCH_SEAT), "{refused}");
}

/// **R2. A seat return is staged as a `control_handoff_request`, never a direct
/// lease write** — the seat goes back at a SAFE BOUNDARY through cut 5's path.
#[test]
fn e7c10_r2_a_seat_return_is_staged_as_a_handoff_request() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
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
    // Drain whatever activation staged, then revoke and inspect what the return
    // actually queued.
    drain(&mut tw);
    ok(
        &mut tw,
        fx.commander,
        &format!("@seat/revoke {}/{}", op.0, seat.0),
    );
    let submit = tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("the revocation staged the declared fallback's handoff request");
    assert_eq!(
        submit.actor, fx.ace,
        "the relief is staged under the departing actor's own surviving billet grant"
    );
    assert_eq!(
        submit.action.key().id.as_str(),
        "control_handoff_request",
        "the seat returns through K6's request path, never a direct lease write"
    );
}

/// **R3. `+seat/leave` restores AI control and does not move the character.**
#[test]
fn e7c10_r3_leaving_restores_ai_control_and_moves_nobody() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let where_before = tw.world().get(fx.driver).and_then(|o| o.location);
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    drain(&mut tw);
    // T13: the player's own character never moved and is still where a bystander
    // can see it.
    assert_eq!(
        tw.world().get(fx.driver).and_then(|o| o.location),
        where_before,
        "activating a seat never moves the player's character"
    );
    drain(&mut tw);
    ok(&mut tw, fx.driver, "+seat/leave");
    drain(&mut tw);
    assert_eq!(
        tw.world().get(fx.driver).and_then(|o| o.location),
        where_before,
        "leaving never moves it either"
    );
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()],
        "AI control resumes on the station's own channels"
    );
    assert_eq!(
        tw.world()
            .module_state::<mush_sim::FocusState>()
            .map(mush_sim::FocusState::len),
        Some(0)
    );
}

/// **R4. A DISCONNECT uses the same path** — the tree's first
/// `HookPoint::Disconnected` hook (cut 5's deferred unclean-loss production
/// trigger) returns the seat at a safe boundary and stales every captured intent.
#[test]
fn e7c10_r4_a_disconnect_returns_the_seat_on_the_same_path() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
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
    let captured = epoch_of(&tw, fx.ace, fx.atlas, "pilot").expect("a grant");
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot").len(), 1);
    drain(&mut tw);
    // The production trigger, through the REAL ingress: `Inbound::SessionGone`,
    // which the session layer sends once `linkdead_grace` expires.
    // `TestWorld::spawn_player` connects as it spawns, and `scene` spawns
    // commander, driver, bystander in that order, so the driver's one live
    // connection is the third the rig minted.
    disconnect(&mut tw, fx.driver, 3);
    assert_eq!(
        tw.world()
            .module_state::<mush_sim::FocusState>()
            .map(mush_sim::FocusState::len),
        Some(0),
        "a disconnect releases the account's foreground focus"
    );
    assert!(
        !is_current(&tw, fx.ace, fx.atlas, "pilot", captured, "movement"),
        "a disconnect stales every intent captured under the seat's grant"
    );
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()],
        "the narrowing goes with the driver"
    );
    let submit = tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("the disconnect staged the declared fallback's handoff request");
    assert_eq!(submit.action.key().id.as_str(), "control_handoff_request");
}

// ===========================================================================
// Hazard axis 3 — attribution and leakage (A)
// ===========================================================================

/// **A1. No account id reaches any projection, any record or any refusal.** A
/// recursive byte scan over every module write and every player-facing render.
#[test]
fn e7c10_a1_no_account_id_reaches_a_record_or_a_projection() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let account = fx
        .oracle
        .0
        .lock()
        .ok()
        .and_then(|m| m.get(&fx.driver).copied())
        .expect("the driver is mapped");
    // The id's own serialized form is what a leak would look like.
    let hex = serde_json::to_string(&account).expect("serializes");
    let hex = hex.trim_matches('"').to_owned();
    assert!(hex.len() >= 16, "the probe needs a real id: {hex}");
    let op = operation(&mut tw, &fx);
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
    let rendered = [
        ok(&mut tw, fx.driver, "+seat/list"),
        ok(&mut tw, fx.commander, "+seat/list"),
        ok(&mut tw, fx.commander, &format!("+muster {}", op.0)),
        ok(
            &mut tw,
            fx.commander,
            &format!("@crew/complement #{}", fx.atlas.0),
        ),
        ok(&mut tw, GOD, &format!("@seat/audit {}/{}", op.0, seat.0)),
        ok(
            &mut tw,
            GOD,
            &format!("@seat/audit {}/{}=#{}", op.0, seat.0, fx.driver.0),
        ),
        err(
            &mut tw,
            fx.bystander,
            &format!("@crew/complement #{}", fx.atlas.0),
        ),
        ok(&mut tw, fx.commander, &format!("@op/show/full {}", op.0)),
    ]
    .join("\n");
    assert!(
        !rendered.contains(&hex),
        "an account id reached a player-facing render"
    );
    // Every durable byte this cut wrote, across every namespace.
    let mut kv = BTreeMap::new();
    fold_kv(&mut tw, &mut kv);
    let bytes = serde_json::to_string(&kv.values().collect::<Vec<_>>()).expect("serializes");
    assert!(
        !bytes.contains(&hex),
        "an account id reached a persisted record"
    );
    // ...including the driver token itself, which is what the journal carries.
    let token = camp(&tw)
        .seat_grant(op, seat)
        .expect("a grant")
        .driver
        .0
        .to_string();
    assert!(!token.contains(&hex), "the driver token embeds the account");
    assert!(
        bytes.contains(&token),
        "the token IS what the record carries"
    );
}

/// **A2. The same player accepting two seats produces two tokens no projection
/// correlates**, and `@seat/audit` is staff-only and prints no id.
#[test]
fn e7c10_a2_two_seats_for_one_player_produce_two_uncorrelated_tokens() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let a = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    let b = offer(&mut tw, &fx, op, fx.locust, fx.duce, "movement");
    for seat in [a, b] {
        ok(
            &mut tw,
            fx.driver,
            &format!("+seat/accept {}/{}", op.0, seat.0),
        );
    }
    let ta = camp(&tw).seat_grant(op, a).expect("a").driver.0.to_string();
    let tb = camp(&tw).seat_grant(op, b).expect("b").driver.0.to_string();
    assert_ne!(ta, tb, "one token per grant");
    // Nothing in either record names the other, and nothing names an account.
    let ra = serde_json::to_string(camp(&tw).seat_grant(op, a).expect("a")).expect("ser");
    assert!(!ra.contains(&tb));
    // The staff verb answers a verdict, names the seat by its own public
    // coordinates and prints no identity — and no token, which is why the fold
    // reshaped it (K6): no render, page, log or journal ever printed one, so the
    // old `@seat/audit <token>` had no reachable input at all.
    let shown = ok(&mut tw, GOD, &format!("@seat/audit {}/{}", op.0, a.0));
    assert!(shown.contains("attributed"), "{shown}");
    assert!(!shown.contains(&ta), "the verb prints no token: {shown}");
    assert!(!shown.contains(&tb));
    // And it is staff-only.
    err(&mut tw, fx.driver, &format!("@seat/audit {}/{}", op.0, a.0));
}

/// **A3. `+seat/list` shows nothing about an operation the viewer is not in the
/// audience of, and a DECLINED offer leaves no trace a third party can read.**
#[test]
fn e7c10_a3_a_covert_operations_seats_are_invisible_and_a_decline_leaves_nothing() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    ok(
        &mut tw,
        fx.commander,
        &format!("@op/terms {}/visibility=covert", op.0),
    );
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    let listed = ok(&mut tw, fx.bystander, "+seat/list");
    assert_eq!(
        listed.trim(),
        "No seats are offered to you.",
        "a covert operation's seats must not be listed at all"
    );
    // And the seat itself is byte-identically absent from an id that never was.
    let real = err(
        &mut tw,
        fx.bystander,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let absent = err(
        &mut tw,
        fx.bystander,
        &format!("+seat/accept {}/9999", op.0),
    );
    assert_eq!(real, absent);
    // A decline writes NOTHING, so there is nothing for a projection to leak.
    ok(
        &mut tw,
        fx.commander,
        &format!("@op/terms {}/visibility=public", op.0),
    );
    let mut kv = BTreeMap::new();
    fold_kv(&mut tw, &mut kv);
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/decline {}/{}", op.0, seat.0),
    );
    let mut after = BTreeMap::new();
    fold_kv(&mut tw, &mut after);
    assert!(
        after
            .values()
            .all(|entries| entries.iter().all(|(k, _)| !k.starts_with("seat_grant:"))),
        "a decline must write no grant document"
    );
}

// ===========================================================================
// Musters, distinctness and the tick arm
// ===========================================================================

/// **M1. A slot resolving to vacancy does NOT cancel the operation**, and the
/// report says so in terms a commander can read.
#[test]
fn e7c10_m1_a_vacancy_does_not_cancel_the_operation() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/unassign #{}=pilot", fx.locust.0),
    );
    let shown = ok(&mut tw, fx.commander, &format!("+muster {}", op.0));
    assert!(shown.contains("VACANT"), "{shown}");
    assert!(shown.contains("the operation stands"), "{shown}");
    // The operation is untouched: it is still live and still commits both units.
    let doc = camp(&tw).operation(op).expect("the operation");
    assert!(!doc.runtime.state.is_terminal());
    assert_eq!(doc.runtime.participants.len(), 2);
    let report = camp(&tw).latest_muster(op).expect("a report");
    assert_eq!(report.vacancies(), 1);
}

/// **M2. Roster lock stores ONLY the verdict, and an oracle answering `None`
/// fails closed.**
#[test]
fn e7c10_m2_roster_lock_stores_only_the_verdict_and_fails_closed() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    // Put a PLAYER on the Atlas pilot seat so the lock has an account to ask
    // about, and give the Locust seat to an unmapped player.
    let ghost = tw.spawn_player("Ghost");
    tw.world_mut().move_to(ghost, fx.field).unwrap();
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/unassign #{}=pilot", fx.atlas.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", fx.atlas.0, fx.driver.0),
    );
    ok(&mut tw, fx.commander, &format!("+muster/lock {}", op.0));
    let report = camp(&tw).latest_muster(op).expect("a report");
    assert_eq!(report.accounts_distinct, Some(true));
    assert!(report.locked);
    // Only the verdict: no account id anywhere in the record.
    let bytes = serde_json::to_string(report).expect("serializes");
    let hex = serde_json::to_string(
        &fx.oracle
            .0
            .lock()
            .ok()
            .and_then(|m| m.get(&fx.driver).copied())
            .expect("mapped"),
    )
    .expect("serializes");
    let hex = hex.trim_matches('"').to_owned();
    assert!(
        !bytes.contains(&hex),
        "only the VERDICT is stored, never the id: {bytes}"
    );
    // An UNMAPPED player makes the verdict unknown, and the lock is refused.
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/unassign #{}=pilot", fx.locust.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", fx.locust.0, ghost.0),
    );
    let refused = err(&mut tw, fx.commander, &format!("+muster/lock {}", op.0));
    assert!(refused.contains("cannot answer"), "{refused}");
}

/// **M3. Two characters on ONE account fail the distinctness gate at roster
/// lock.**
#[test]
fn e7c10_m3_two_characters_on_one_account_fail_the_roster_lock() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let alt = tw.spawn_player("Alt");
    tw.world_mut().move_to(alt, fx.field).unwrap();
    // The alt shares the driver's account — exactly what C23A/C24A forbid on one
    // roster.
    let shared = fx
        .oracle
        .0
        .lock()
        .ok()
        .and_then(|m| m.get(&fx.driver).copied())
        .expect("mapped");
    if let Ok(mut map) = fx.oracle.0.lock() {
        map.insert(alt, shared);
    }
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/unassign #{}=pilot", fx.atlas.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/unassign #{}=pilot", fx.locust.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", fx.atlas.0, fx.driver.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", fx.locust.0, alt.0),
    );
    let refused = err(&mut tw, fx.commander, &format!("+muster/lock {}", op.0));
    assert!(refused.contains("one account"), "{refused}");
    let report = camp(&tw).latest_muster(op);
    assert!(
        report.is_none_or(|r| !r.locked),
        "a failed distinctness gate must not lock the roster"
    );
}

/// **T1. The campaign tick fires TWICE in sequence**, its seat arm is a pull, and
/// the second fire stages nothing new rather than repeating a key or freezing.
#[test]
fn e7c10_t1_the_seat_arm_fires_twice_and_the_second_fire_is_a_no_op() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
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
    drain(&mut tw);
    fire(&mut tw);
    let after_one = grant_state(&tw, op, seat);
    assert!(
        !tw.engine().has_authoritative_submit(),
        "the campaign tick makes no authoritative submission at all"
    );
    fire(&mut tw);
    assert_eq!(
        grant_state(&tw, op, seat),
        after_one,
        "the second fire is a no-op past the applied point"
    );
    assert!(!tw.engine().has_authoritative_submit());
}

/// **T2. A reboot re-running the sweep changes nothing**, and every seat record
/// re-enters through its own `TryFrom` wire.
#[test]
fn e7c10_t2_a_reboot_re_running_the_sweep_is_a_no_op() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let before = camp(&tw).seat_offer(op, seat).cloned().expect("an offer");
    let grant = camp(&tw).seat_grant(op, seat).cloned().expect("a grant");
    let mut kv = BTreeMap::new();
    fold_kv(&mut tw, &mut kv);
    reboot(&mut tw, &kv);
    assert_eq!(
        camp(&tw).seat_offer(op, seat),
        Some(&before),
        "the offer re-enters through its wire byte-identically"
    );
    assert_eq!(camp(&tw).seat_grant(op, seat), Some(&grant));
    fire(&mut tw);
    assert_eq!(camp(&tw).seat_grant(op, seat), Some(&grant));
}

/// **T3. An over-cap seat document REFUSES on load and is HELD, never deleted
/// and never truncated** — and the boot still completes.
#[test]
fn e7c10_t3_an_over_cap_seat_document_refuses_on_load_and_is_held() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    let mut kv = BTreeMap::new();
    fold_kv(&mut tw, &mut kv);
    let key = SmolStr::new(format!("seat_offer:{}:{}", op.0, seat.0));
    let entry = kv
        .get_mut("campaign")
        .and_then(|e| e.iter_mut().find(|(k, _)| *k == key))
        .expect("the offer document");
    let channels: Vec<String> = (0..=mush_campaign::MAX_SEAT_CHANNELS)
        .map(|n| format!("c{n}"))
        .collect();
    entry.1["scope"]["channels"] = serde_json::json!(channels);
    reboot(&mut tw, &kv);
    assert!(
        camp(&tw).seat_offer(op, seat).is_none(),
        "an over-cap document refuses on load rather than truncating"
    );
    assert!(
        camp(&tw).held().iter().any(|d| d.key == key),
        "and it is HELD, never deleted"
    );
}

/// **T4. `sim.station.served` is answered from an APPLIED MARKER, never a
/// status** — a sortie that merely started answers unknown.
#[test]
fn e7c10_t4_station_service_answers_only_after_the_seat_is_given_back() {
    use mush_economy::FactProvider;
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let provider = mush_sim::SimFactProvider;
    // K4 (fold): the fact id carries the instant service BEGAN as a fourth term,
    // which is published nowhere — so a party who does not hold the grant cannot
    // name the fact at all. `0` is the "never served" probe.
    let id = format!("{}/pilot/{}/0", fx.atlas.0, fx.ace.0);
    assert_eq!(
        provider.resolve(tw.world(), "sim.station.served", &id, None),
        None,
        "an unstarted sortie is unknown, which is unsatisfied"
    );
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    drain(&mut tw);
    assert_eq!(
        provider.resolve(tw.world(), "sim.station.served", &id, None),
        None,
        "a sortie that merely STARTED must not answer the fact"
    );
    let began = tw
        .world()
        .module_state::<mush_sim::FocusState>()
        .and_then(|state| state.focus_of_actor(fx.ace).map(|f| f.acquired_at.at))
        .expect("the seat is live");
    tw.set_now(tw.world().now() + 600);
    drain(&mut tw);
    ok(&mut tw, fx.driver, "+seat/leave");
    drain(&mut tw);
    assert_eq!(
        provider.resolve(tw.world(), "sim.station.served", &id, None),
        None,
        "the three public coordinates alone still name nothing"
    );
    let named = format!("{}/pilot/{}/{began}", fx.atlas.0, fx.ace.0);
    let state = provider
        .resolve(tw.world(), "sim.station.served", &named, None)
        .expect("completed service answers");
    assert!(state.satisfied);
    assert_eq!(state.actor, Some(fx.ace));
    // An unknown kind, and an id for service that never happened, both answer
    // `None` — unknown is unsatisfied, everywhere.
    assert_eq!(
        provider.resolve(tw.world(), "sim.station.flown", &named, None),
        None
    );
    assert_eq!(
        provider.resolve(
            tw.world(),
            "sim.station.served",
            &format!("{}/pilot/{}/{began}", fx.locust.0, fx.duce.0),
            None
        ),
        None
    );
}

/// **P1. Cut 10 registers no TacSim party type and leaves `mush-party` alone.**
#[test]
fn e7c10_p1_no_tacsim_party_type_exists() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .to_path_buf();
    let mut offenders: Vec<String> = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let name = path.to_string_lossy().replace('\\', "/");
            if !name.contains("mush-campaign/src/") && !name.contains("mush-sim/src/") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            for line in text.lines() {
                let code = line.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                if code.contains("mush_party") || code.contains("MAX_PARTY_SIZE") {
                    offenders.push(format!("{name}: {code}"));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "cut 10 must consume no party type at all: {offenders:?}"
    );
}

/// **K4 (leg 2). A channel under another holder's live lease refuses — and a
/// probe asserts the KERNEL call alone does NOT refuse it** (08j S1).
///
/// `ControlLeaseState::checked_request_handoff` never compares `requested_by` to
/// `holder`, because requesting over a live lease is its ordinary path. The
/// middle leg of the K6 gate is therefore **cut 10's own policy**, and this test
/// proves both halves: the kernel says yes, and cut 10 says no.
#[test]
fn e7c10_k4_a_leased_channel_refuses_and_the_kernel_alone_does_not_refuse_it() {
    use mush_core::{
        ClockRef, ClockStamp, ControlHandoffDraft, ControlLeaseKey, ControlLeaseState, PrincipalId,
        RequestUid,
    };
    // --- the kernel half, in isolation: a NON-holder's request over a live
    // lease is accepted by `checked_request_handoff`.
    let key = ControlLeaseKey::new(ObjId(99), ChannelId::new("movement").unwrap());
    let lease = ControlLeaseState::new(key).expect("a fresh lease");
    let stamp = |at| ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    };
    let seat = lease
        .checked_request_handoff(
            0,
            ControlHandoffDraft {
                request: RequestUid::from_bytes([1; 16]),
                requested_by: PrincipalId::Object { id: ObjId(50) },
                next_holder: Some(PrincipalId::Object { id: ObjId(50) }),
                requested_at: stamp(1),
                boundary: stamp(2),
                reason: mush_core::CanonicalId::new("seat.activate").unwrap(),
            },
        )
        .expect("the kernel accepts a first request");
    let held = seat
        .checked_apply_handoff(seat.revision(), RequestUid::from_bytes([1; 16]), stamp(2))
        .expect("it applies at the boundary");
    assert_eq!(held.holder(), Some(&PrincipalId::Object { id: ObjId(50) }));
    let over_a_live_lease = held.checked_request_handoff(
        held.revision(),
        ControlHandoffDraft {
            request: RequestUid::from_bytes([2; 16]),
            // A DIFFERENT principal, over a lease somebody else holds.
            requested_by: PrincipalId::Object { id: ObjId(60) },
            next_holder: Some(PrincipalId::Object { id: ObjId(60) }),
            requested_at: stamp(3),
            boundary: stamp(4),
            reason: mush_core::CanonicalId::new("seat.activate").unwrap(),
        },
    );
    assert!(
        over_a_live_lease.is_ok(),
        "the kernel call alone does NOT refuse a request over a live lease — \
         which is exactly why leg (2) has to be cut 10's own policy check"
    );
    // ...and the ONLY kernel-answered leg refuses an apply before the boundary.
    let pending = over_a_live_lease.expect("accepted");
    assert!(matches!(
        pending.checked_apply_handoff(
            pending.revision(),
            RequestUid::from_bytes([2; 16]),
            stamp(3)
        ),
        Err(mush_core::ControlLeaseError::BeforeBoundary)
    ));

    // --- cut 10's half, through the real ingress: a third party's live lease on
    // the seat's own channel refuses activation, naming the station and nothing
    // else about the holder.
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    // A third party holds the Atlas `movement` lease.
    tw.world_mut()
        .module_state_mut::<mush_sim::ControlLeaseView>()
        .expect("the lease mirror")
        .note_holder(
            fx.atlas,
            ChannelId::new("movement").unwrap(),
            PrincipalId::Object { id: fx.bystander },
        );
    let refused = err(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    assert!(
        refused.contains("under another controller's lease"),
        "{refused}"
    );
    assert!(
        !refused.contains("Bystander"),
        "the refusal names the station and nothing about the holder: {refused}"
    );
    assert_eq!(
        grant_state(&tw, op, seat),
        Some(GrantState::Dormant),
        "no Active state is ever recorded before `checked_apply_handoff` returns"
    );
    // Nothing moved: Ace still holds the whole station.
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot").len(), 2);
}

/// **P2. No legality check reads a `SeatGrant`.** The live authority is cut 5's
/// billet, the narrowed `ChannelGrant` and K6's lease; the grant is workflow.
#[test]
fn e7c10_p2_no_legality_check_reads_a_seat_grant() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .to_path_buf();
    let mut offenders: Vec<String> = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let name = path.to_string_lossy().replace('\\', "/");
            if !name.contains("/src/") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            // A `SeatGrant` read anywhere but the campaign crate's own seat
            // lane would be a legality path reading workflow.
            if text.contains("SeatGrant")
                && !name.contains("mush-campaign/src/seat.rs")
                && !name.contains("mush-campaign/src/seatcmd.rs")
                && !name.contains("mush-campaign/src/seatstate.rs")
                && !name.contains("mush-campaign/src/store.rs")
                && !name.contains("mush-campaign/src/lib.rs")
            {
                offenders.push(name);
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a SeatGrant is read outside the campaign seat lane: {offenders:?}"
    );
}

/// **G7. An intent outside `permitted_area` refuses** (T11's area arm), and the
/// declared area really is consented content: it has ONE writer (`area:<where>`
/// on `+seat/offer`) and ONE reader (this check), so it is not the hashed field
/// with no writer cut 9's fold B13 struck.
#[test]
fn e7c10_g7_a_seat_outside_its_permitted_area_refuses() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    // A second room the Atlas is NOT in.
    let elsewhere = tw
        .world_mut()
        .create(ObjKind::Room, "Elsewhere", GOD, None)
        .unwrap();
    let before: BTreeSet<u64> = camp(&tw).seat_offers().map(|o| o.id.0).collect();
    ok(
        &mut tw,
        fx.commander,
        &format!(
            "+seat/offer {}/#{}/pilot=#{},movement,to:#{},area:#{}",
            op.0, fx.atlas.0, fx.ace.0, fx.driver.0, elsewhere.0
        ),
    );
    let seat = camp(&tw)
        .seat_offers()
        .map(|o| o.id)
        .find(|id| !before.contains(&id.0))
        .expect("one new offer");
    assert_eq!(
        camp(&tw)
            .seat_offer(op, seat)
            .and_then(|o| o.scope.permitted_area),
        Some(mush_campaign::UnitRef(elsewhere)),
        "the declared area really is stored, and inside the consent hash"
    );
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let refused = err(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    // K7 (fold): the area refusal is byte-identical to the existence-hiding one,
    // so a grant-holder polling it learns nothing about where the unit is.
    assert_eq!(refused.trim(), mush_campaign::NO_SUCH_SEAT, "{refused}");
    // Nothing moved: the incumbent still holds the whole station.
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot").len(), 2);
    assert_eq!(grant_state(&tw, op, seat), Some(GrantState::Dormant));
    // Move the unit INTO the declared area and the same seat activates.
    tw.world_mut().move_to(fx.atlas, elsewhere).unwrap();
    // The crew goes with the ship: reach is proven for the DRIVEN NPC at the
    // console, so a unit that moved without its crew is unreachable anyway.
    tw.world_mut().move_to(fx.ace, elsewhere).unwrap();
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    drain(&mut tw);
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["movement".to_owned()]
    );
}

/// **K5. The K6 MEMO predicate, by name.** 10b §3.10's memo condition retires
/// Draft 0.2's "no live domain" rule: activation on a unit inside a `Pulse`
/// domain requires the rows of `docs/design/09a-k6-look-back.md` cut 10 depends
/// on to read **production**, plus cuts 6 and 8's own migrations, which the cut
/// order guarantees. This is a **passing** pin naming those rows — never an
/// `#[ignore]`, because `ci_gated_test_inventory` requires every ignored test to
/// be owned by a CI job that runs it green (cut 5's deviation (j)).
#[test]
fn e7c10_k5_the_k6_memo_predicate_names_its_production_rows() {
    let memo = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("the repo root")
            .join("docs/design/09a-k6-look-back.md"),
    )
    .expect("the K6 memo exists");
    // The two rows cut 10's path actually depends on, by the memo's own words.
    assert!(
        memo.contains("Production mechanism complete"),
        "the control-lease/handoff row must read production"
    );
    assert!(
        memo.contains("Production lifecycle complete"),
        "the contacts row must read production"
    );
    // The memo's FIXTURE-ONLY rows are deliberately NOT on cut 10's path:
    // atomic damage is cut 8's, unit DTOs are `unit.status`'s projection, and
    // the leak proofs are cut 7/8 probes.
    assert!(
        memo.contains("Fixture"),
        "the memo still distinguishes fixtures"
    );
    // Cuts 6 and 8 are present in this composition: the journey tick and the
    // pulse cadence are both `mush-sim` state the seat path shares a world with.
    let (tw, _fx) = scene(Arc::new(TestAudit::default()));
    assert!(
        tw.world()
            .module_state::<mush_sim::tick::PulseConfig>()
            .is_some(),
        "cut 8's pulse cadence is composed"
    );
    assert!(
        tw.world().module_state::<mush_sim::JourneyBook>().is_some(),
        "cut 6's journey book is composed"
    );
}

/// **K6. An LLM-driven seat is K7-gated, and this is a PASSING pin naming K7.**
///
/// Cut 10 ships deterministic drivers only: the seat's fallback controller is
/// the `sim.station_fallback` record's declared holder and the handoff it stages
/// carries a deterministic key with **no `PendingDecision`** (K7.0's immediate-
/// controller rule). A K7 LLM driver would have to reuse that decision's record
/// and idempotency key; nothing in cut 10 mints one, and this asserts it.
#[test]
fn e7c10_k6_an_llm_driven_seat_is_k7_gated_and_cut_ten_mints_no_decision() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
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
    let staged = drain(&mut tw);
    // **16b cut K.2.** This fixture's station grants TWO channels
    // (`movement+fire`), and a K6 lease is per channel: `handoff::station_channel`
    // has always declined to project an offer for a station whose set is not
    // exactly one, so the request this activation used to stage could only ever
    // be refused — and on a real server that refusal reached the player as "That
    // did not go through. Nothing changed." after a seat that had in fact been
    // taken. `seat.rs` now asks the same question before staging, so a
    // multi-channel station stages nothing. What this test is about is unchanged
    // and is asserted below: whatever IS staged carries a deterministic key, and
    // no `PendingDecision` is minted on any of it.
    for submit in &staged {
        assert!(
            submit.key.as_str().starts_with("seat.activate/")
                || submit.key.as_str().starts_with("station.fallback."),
            "every staged key is deterministic and self-derived: {}",
            submit.key.as_str()
        );
    }
    // No `PendingDecision` is captured anywhere on this path: K7's lifecycle is
    // what an LLM-driven seat would need, and cut 10 does not touch it.
    assert!(
        !tw.engine().has_decision_capture(),
        "cut 10 mints no PendingDecision; an LLM driver stays K7-gated"
    );
}

// ---- E7 cut 10 independent verify fold ----
//
// Every test below asserts the OBSERVABLE outcome through the real command path
// — the channels a resolver publishes for the driven NPC, the bytes a stranger
// receives, whose account the focus names after a contested activation, whether
// a grant is still re-activatable after a reboot, how many `muster:` keys
// survive a fold — and names every arm the guard it exercises covers.

/// Run a line and report which way it went, asserting nothing. For a test whose
/// point is *which* way it went.
fn run(tw: &mut TestWorld, actor: ObjId, line: &str) -> (bool, String) {
    let ev = tw.run(actor, line);
    (!has_error(&ev), texts(&ev))
}

/// A declared, public operation with **no committed units**. R1 forbids a second
/// operation joining a unit the first holds, and `+seat/offer` does not require
/// participation — so this is how a second operation exists at all.
fn operation_bare(tw: &mut TestWorld, fx: &Fx) -> OperationId {
    let before: BTreeSet<u64> = camp(tw).operations().map(|d| d.id().0).collect();
    ok(tw, fx.commander, "@op/declare screen the approach");
    let id = camp(tw)
        .operations()
        .map(|d| d.id())
        .find(|id| !before.contains(&id.0))
        .expect("one new operation");
    ok(tw, fx.commander, &format!("@op/terms {}/notice=24", id.0));
    ok(
        tw,
        fx.commander,
        &format!("@op/terms {}/visibility=public", id.0),
    );
    id
}

/// The token on one grant, as the record carries it.
fn token_of(tw: &TestWorld, op: OperationId, id: SeatOfferId) -> String {
    camp(tw)
        .seat_grant(op, id)
        .expect("a grant")
        .driver
        .0
        .to_string()
}

/// Whether the audit row for `token` names `who`'s account (A2's helper, from
/// the test's own side of the seam).
fn audit_is(audit: &TestAudit, oracle: &MapOracle, token: &str, who: ObjId) -> bool {
    let account = oracle.0.lock().ok().and_then(|m| m.get(&who).copied());
    account.is_some() && audit.account_for(token) == account
}

/// Rewrite one stored `seat_offer:` document and RE-HASH it, so the offer stays
/// self-consistent — the shape a future edit verb, a held-document repair or a
/// hand-edited key produces.
fn patch_offer(
    kv: &mut BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>,
    op: OperationId,
    id: SeatOfferId,
    patch: impl FnOnce(&mut mush_campaign::SeatOffer),
) {
    let key = mush_campaign::seat_offer_key(op, id);
    let entry = kv
        .get_mut("campaign")
        .and_then(|entries| entries.iter_mut().find(|(k, _)| *k == key))
        .expect("the offer document");
    let mut offer: mush_campaign::SeatOffer =
        serde_json::from_value(entry.1.clone()).expect("decodes");
    patch(&mut offer);
    offer.consent_hash = mush_campaign::seat_consent_hash(&offer);
    entry.1 = serde_json::to_value(&offer).expect("serializes");
}

/// Rewrite one stored `seat_grant:` document's state to `Active { epoch }`.
fn patch_grant_state(
    kv: &mut BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>,
    op: OperationId,
    id: SeatOfferId,
    epoch: u64,
) {
    let key = mush_campaign::seat_grant_key(op, id);
    let entry = kv
        .get_mut("campaign")
        .and_then(|entries| entries.iter_mut().find(|(k, _)| *k == key))
        .expect("the grant document");
    entry.1["state"] = serde_json::json!({ "state": "active", "epoch": epoch });
}

/// Name `who` as the operation's declared defending side — a *named side
/// principal*, which `render::lens_for` hands `Lens::Plan`.
fn name_defender(tw: &mut TestWorld, fx: &Fx, op: OperationId, who: ObjId) {
    ok(
        tw,
        fx.commander,
        &format!("@op/terms {}/side=defence:#{}", op.0, who.0),
    );
}

/// **A1a. A seat offer is invisible and untakeable outside its audience — on a
/// PUBLIC operation, which is the whole blast radius.**
///
/// `visible_offer` gated on `render::visible_operation(..) != Lens::Hidden` and
/// nothing else, and `lens_for` hands `Lens::Notice` to EVERY actor in the world
/// for an `EconomicVisibility::Public` operation — which is exactly what the rig
/// builds. `SeatOffer.audience`/`.eligibility` had no reader anywhere, so a
/// stranger with no side, no audience membership, no unit and no capability
/// listed the commander's seat, accepted it and activated it. The arms: list,
/// accept, activate, and the byte-identity of the refusal.
#[test]
fn vf_a1a_a_seat_offer_is_invisible_and_untakeable_outside_its_audience() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    // The bystander is a total stranger: no side, no unit, no capability.
    let listed = ok(&mut tw, fx.bystander, "+seat/list");
    assert_eq!(
        listed.trim(),
        "No seats are offered to you.",
        "a stranger is shown the seat itself, station name included: {listed}"
    );
    let real = err(
        &mut tw,
        fx.bystander,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let absent = err(
        &mut tw,
        fx.bystander,
        &format!("+seat/accept {}/9999", op.0),
    );
    assert_eq!(real, absent, "the refusal must be existence-hiding");
    let (activated, _) = run(
        &mut tw,
        fx.bystander,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    assert!(!activated);
    // Nothing moved: the commander's own station is NOT narrowed and no focus
    // was consumed on the commander's own NPC.
    drain(&mut tw);
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()],
        "a stranger's seat narrowed the commander's own station"
    );
    assert_eq!(
        tw.world()
            .module_state::<mush_sim::FocusState>()
            .map(mush_sim::FocusState::len),
        Some(0)
    );
    // …and the driver, who IS in the audience, still succeeds on every arm.
    let listed = ok(&mut tw, fx.driver, "+seat/list");
    assert!(listed.contains("station `pilot`"), "{listed}");
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
}

/// **A1b. A named defender — a Plan-lens principal — is refused the attacker's
/// seat byte-identically to an id that never existed.**
#[test]
fn vf_a1b_a_named_defender_is_refused_the_attackers_seat_byte_identically() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    name_defender(&mut tw, &fx, op, fx.bystander);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    // The defender sees the PLAN (that is what `lens_for` is for) and still may
    // not take the attacker's seat.
    let plan = ok(&mut tw, fx.bystander, &format!("@op/show/full {}", op.0));
    assert!(plan.contains("Participants"), "{plan}");
    let real = err(
        &mut tw,
        fx.bystander,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let absent = err(
        &mut tw,
        fx.bystander,
        &format!("+seat/accept {}/9999", op.0),
    );
    assert_eq!(real, absent);
    assert!(camp(&tw).seat_grant(op, seat).is_none());
}

/// **A1c. A grant taken before an audience narrowing cannot be driven after
/// it** — the predicate is asked again at activation, not only at acceptance.
#[test]
fn vf_a1c_a_grant_taken_before_an_audience_narrowing_cannot_be_driven() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    // Narrow the audience under the accepted grant, through the stored document
    // and a reboot: the offer now names only the operation's own attacker, and
    // it is re-hashed so it is self-consistent (this is the AUDIENCE arm, not
    // A12's frozen-hash arm).
    let mut kv = BTreeMap::new();
    fold_kv(&mut tw, &mut kv);
    patch_offer(&mut kv, op, seat, |offer| {
        offer
            .audience
            .members
            .retain(|p| *p != mush_org::PrincipalId::Object(fx.driver));
    });
    reboot(&mut tw, &kv);
    assert!(
        camp(&tw)
            .seat_offer(op, seat)
            .is_some_and(mush_campaign::SeatOffer::consent_intact)
    );
    let refused = err(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    assert_eq!(refused.trim(), mush_campaign::NO_SUCH_SEAT, "{refused}");
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()],
        "nothing was narrowed"
    );
}

/// **A2. Only the accepter may activate, and the audit row still names them.**
///
/// `seat_activate`'s attribution check was `audit_knows(token)` — "does ANY row
/// exist" — so account B drove a grant whose only `token -> account` row named
/// account A, and R7/T9's dual attribution resolved to somebody who was not
/// driving. Arms: the accepter succeeds; a second audience member is refused
/// byte-identically; the focus is untouched; and the row is unchanged.
#[test]
fn vf_a2_only_the_accepter_may_activate_and_the_audit_row_still_names_them() {
    let audit = Arc::new(TestAudit::default());
    let (mut tw, fx) = scene(audit.clone());
    let op = operation(&mut tw, &fx);
    // The offer names BOTH would-be drivers, so the audience predicate cannot be
    // what refuses here: only the attribution can.
    ok(
        &mut tw,
        fx.commander,
        &format!(
            "+seat/offer {}/#{}/pilot=#{},movement,to:#{},to:#{}",
            op.0, fx.atlas.0, fx.ace.0, fx.driver.0, fx.bystander.0
        ),
    );
    let seat = camp(&tw)
        .seat_offers()
        .map(|o| o.id)
        .max()
        .expect("an offer");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let token = token_of(&tw, op, seat);
    assert!(audit_is(&audit, &fx.oracle, &token, fx.driver));
    let real = err(
        &mut tw,
        fx.bystander,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    let absent = err(
        &mut tw,
        fx.bystander,
        &format!("+seat/activate {}/9999", op.0),
    );
    assert_eq!(real, absent, "the refusal must be existence-hiding");
    drain(&mut tw);
    assert!(
        tw.world()
            .module_state::<mush_sim::FocusState>()
            .and_then(|s| s.focus_of_actor(fx.ace))
            .is_none(),
        "the seat was driven by an account the one attribution row does not name"
    );
    assert!(
        audit_is(&audit, &fx.oracle, &token, fx.driver),
        "the one attribution row still names the accepter"
    );
    // …and the accepter themselves still activates.
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    drain(&mut tw);
    assert_eq!(
        tw.world()
            .module_state::<mush_sim::FocusState>()
            .and_then(|s| s.focus_of_actor(fx.ace))
            .map(|f| f.account),
        fx.oracle
            .0
            .lock()
            .ok()
            .and_then(|m| m.get(&fx.driver).copied()),
    );
}

/// **A3. A second proxy seat on the same NPC refuses, and the live driver is
/// untouched.**
///
/// K6 leg (1) was `Some(seat.actor) != fallback && seat.actor != npc` — and
/// after ANY activation the billet's actor **is** the npc, so the third conjunct
/// was false for a second seat on the same NPC and `FocusState::claim` silently
/// evicted the first account's focus. §3.10 says "a live human operator is never
/// evicted" and "a test pins that E7 ships no eviction path at all"; `e7c10_k1`
/// exercised only the human-billet arm. Arms: the refusal bytes, the incumbent's
/// focus, their grant state, and their narrowed set.
#[test]
fn vf_a3_a_second_seat_on_the_same_npc_refuses_and_the_live_driver_is_untouched() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let mine = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.commander,
        &format!(
            "+seat/offer {}/#{}/pilot=#{},movement,fire,to:#{}",
            op.0, fx.atlas.0, fx.ace.0, fx.bystander.0
        ),
    );
    let theirs = camp(&tw)
        .seat_offers()
        .map(|o| o.id)
        .max()
        .expect("an offer");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, mine.0),
    );
    ok(
        &mut tw,
        fx.bystander,
        &format!("+seat/accept {}/{}", op.0, theirs.0),
    );
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, mine.0),
    );
    drain(&mut tw);
    let account = tw
        .world()
        .module_state::<mush_sim::FocusState>()
        .and_then(|s| s.focus_of_actor(fx.ace))
        .map(|f| f.account);
    assert!(account.is_some());
    let refused = err(
        &mut tw,
        fx.bystander,
        &format!("+seat/activate {}/{}", op.0, theirs.0),
    );
    assert!(
        refused.contains("seats are never taken by force"),
        "{refused}"
    );
    drain(&mut tw);
    assert_eq!(
        tw.world()
            .module_state::<mush_sim::FocusState>()
            .and_then(|s| s.focus_of_actor(fx.ace))
            .map(|f| f.account),
        account,
        "the live human driver was evicted with no message at all"
    );
    assert_eq!(
        tw.world()
            .module_state::<mush_sim::FocusState>()
            .map(mush_sim::FocusState::len),
        Some(1)
    );
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["movement".to_owned()],
        "the newcomer's wider set replaced the incumbent's"
    );
    assert_eq!(grant_state(&tw, op, theirs), Some(GrantState::Dormant));
}

/// **A4. A terminal operation stands a live seat down BEFORE expiring it.**
///
/// `expire_grants` wrote `Expired` and called `audit_forget` while the billet,
/// the narrowing, the proxy focus and the K6 lease all survived — after which
/// `+seat/leave` answered "You are driving no seat" and `@seat/revoke` refused
/// on `is_terminal()`, so control persisted un-attributed with **no exit but a
/// disconnect**, and the `token -> account` row was deleted while the human was
/// still driving. Arms: the grant is terminal, the focus is gone, the station
/// publishes its whole set again, and the row is forgotten only after release.
#[test]
fn vf_a4_a_terminal_operation_stands_a_live_seat_down_before_expiring_it() {
    let audit = Arc::new(TestAudit::default());
    let (mut tw, fx) = scene(audit.clone());
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let token = token_of(&tw, op, seat);
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    drain(&mut tw);
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot"), vec!["movement"]);
    ok(&mut tw, GOD, &format!("@op/resolve {}=abort", op.0));
    for _ in 0..4 {
        fire(&mut tw);
        drain(&mut tw);
    }
    assert_eq!(grant_state(&tw, op, seat), Some(GrantState::Expired));
    assert!(
        tw.world()
            .module_state::<mush_sim::FocusState>()
            .and_then(|s| s.focus_of_actor(fx.ace))
            .is_none(),
        "the driver kept an un-attributable seat with no exit but a disconnect"
    );
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()],
        "the station goes back to its own channel set"
    );
    assert!(
        !audit.knows(&token),
        "the row is forgotten — but only after the release"
    );
}

/// **A5. Leaving revokes only your OWN grant, and forgets only your own row.**
///
/// `active_grant_of` resolved the caller's proxy focus to `(unit, station)` and
/// then took the first non-terminal grant on that station in key order, never
/// comparing the driver token — so `+seat/leave` revoked the LOWEST-numbered
/// grant, another player's, and forgot *their* row while the leaver's own grant
/// stayed re-activatable.
#[test]
fn vf_a5_leaving_revokes_only_your_own_grant_and_forgets_only_your_own_row() {
    let audit = Arc::new(TestAudit::default());
    let (mut tw, fx) = scene(audit.clone());
    let op = operation(&mut tw, &fx);
    // Offer #1 goes to the bystander (lower id), #2 to the driver — same station.
    ok(
        &mut tw,
        fx.commander,
        &format!(
            "+seat/offer {}/#{}/pilot=#{},movement,to:#{}",
            op.0, fx.atlas.0, fx.ace.0, fx.bystander.0
        ),
    );
    let victim = camp(&tw)
        .seat_offers()
        .map(|o| o.id)
        .max()
        .expect("an offer");
    let mine = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    assert!(victim.0 < mine.0, "the victim's grant must sort first");
    ok(
        &mut tw,
        fx.bystander,
        &format!("+seat/accept {}/{}", op.0, victim.0),
    );
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, mine.0),
    );
    let victim_token = token_of(&tw, op, victim);
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, mine.0),
    );
    drain(&mut tw);
    ok(&mut tw, fx.driver, "+seat/leave");
    drain(&mut tw);
    assert_eq!(grant_state(&tw, op, mine), Some(GrantState::Revoked));
    assert_eq!(
        grant_state(&tw, op, victim),
        Some(GrantState::Dormant),
        "another player's grant was revoked by this leave"
    );
    assert!(
        audit.knows(&victim_token),
        "another player's audit row was forgotten by this leave"
    );
    assert!(audit_is(&audit, &fx.oracle, &victim_token, fx.bystander));
}

/// **A6. A restart returns an orphaned seat to the station's own channels.**
///
/// `granted` is persisted in the exempt `assignment:<unit>` document and
/// re-hydrated; `FocusState` is never hydrated (D-E7-34) and the audit log is
/// process-local, so the AI NPC came back on the SEAT's narrowed set for ever,
/// with no grant anybody could re-activate to clear it. Arms: the republished
/// channel set, `channel_grant_is_current` per channel, and the document's bytes.
#[test]
fn vf_a6_a_restart_returns_an_orphaned_seat_to_the_stations_own_channels() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
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
    let mut kv = BTreeMap::new();
    fold_kv(&mut tw, &mut kv);
    // A restart: `FocusState` is a RAM slot boot never hydrates, so a fresh
    // process is exactly "that slot empty, the documents on disk".
    *tw.world_mut()
        .module_state_mut::<mush_sim::FocusState>()
        .expect("focus state") = mush_sim::FocusState::default();
    reboot(&mut tw, &kv);
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()],
        "the station came back permanently narrowed with nobody driving it"
    );
    let epoch = epoch_of(&tw, fx.ace, fx.atlas, "pilot").expect("a grant");
    assert!(is_current(&tw, fx.ace, fx.atlas, "pilot", epoch, "fire"));
    // …and the persisted document itself carries no narrowing any more.
    let mut after = BTreeMap::new();
    fold_kv(&mut tw, &mut after);
    let doc = after
        .get("sim")
        .and_then(|entries| {
            entries
                .iter()
                .find(|(k, _)| k.as_str() == format!("assignment:{}", fx.atlas.0))
        })
        .map(|(_, v)| v.clone())
        .expect("the assignment document");
    assert!(
        doc["seats"]["pilot"].get("granted").is_none(),
        "the cleared narrowing must be persisted, not only repaired in RAM: {doc}"
    );
    // A further fire changes nothing.
    fire(&mut tw);
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()]
    );
}

/// **A9a. The distinctness oracle answers the muster.**
/// `AccountOracle::all_accounts_distinct` had no production caller at all;
/// `take_muster` re-implemented distinctness over ids it collected itself.
#[test]
fn vf_a9a_two_characters_on_one_account_cannot_lock_a_roster_on_both_sides() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    // Two player-held billets resolving to ONE account: the oracle says `false`.
    let alt = tw.spawn_player("Alt");
    tw.world_mut().move_to(alt, fx.field).unwrap();
    if let Ok(mut map) = fx.oracle.0.lock() {
        let same = map.get(&fx.driver).copied().expect("mapped");
        map.insert(alt, same);
    }
    for (unit, who) in [(fx.atlas, fx.driver), (fx.locust, alt)] {
        ok(
            &mut tw,
            GOD,
            &format!("@spatial/unassign #{}=pilot", unit.0),
        );
        ok(
            &mut tw,
            GOD,
            &format!("@spatial/assign #{}=pilot,#{}", unit.0, who.0),
        );
    }
    let refused = err(&mut tw, fx.commander, &format!("+muster/lock {}", op.0));
    assert!(refused.contains("one account"), "{refused}");
    assert!(
        camp(&tw)
            .latest_muster(op)
            .is_none_or(|report| !report.locked),
        "a failed distinctness gate must not lock the roster"
    );
}

/// **A9b. A locked roster refuses a further seat accept AND activate** —
/// `MusterReport.locked` had no reader anywhere in production, so nothing
/// re-checked after `+muster/lock` and C23A/C24A were enforced nowhere.
#[test]
fn vf_a9b_a_locked_roster_refuses_a_further_seat_accept_and_activate() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    // One live seat (so every human-directed slot resolves to a known account,
    // which is what the distinctness gate needs) and one dormant grant beside it.
    let live = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, live.0),
    );
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, live.0),
    );
    drain(&mut tw);
    // The Locust's billet is vacated, so its slot contributes no account and the
    // distinctness verdict stays answerable with a dormant grant standing on it.
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/unassign #{}=pilot", fx.locust.0),
    );
    let dormant = offer(&mut tw, &fx, op, fx.locust, fx.duce, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, dormant.0),
    );
    ok(&mut tw, fx.commander, &format!("+muster/lock {}", op.0));
    assert!(camp(&tw).latest_muster(op).expect("a report").locked);
    // The ACCEPT arm: a seat offered after the lock cannot be taken.
    let late = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    let refused = err(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, late.0),
    );
    assert!(refused.contains("roster is locked"), "{refused}");
    assert!(camp(&tw).seat_grant(op, late).is_none());
    // The ACTIVATE arm: a grant taken before the lock cannot be driven after it.
    let refused = err(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, dormant.0),
    );
    assert!(refused.contains("roster is locked"), "{refused}");
    assert_eq!(grant_state(&tw, op, dormant), Some(GrantState::Dormant));
}

/// **A12. A grant accepted against older terms refuses to activate.**
///
/// The grant copies the offer's `consent_hash` at acceptance and cut 10 then
/// checked only `offer.consent_intact()` — the offer's self-consistency with
/// itself, which any re-hash satisfies. "All freeze in the accepted invitation"
/// was unenforced.
#[test]
fn vf_a12_a_grant_accepted_against_older_terms_refuses_to_activate() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    // A widened scope in the STORED document, re-hashed so the offer is
    // self-consistent again — exactly what a future edit verb, a held-document
    // repair or a hand-edited `seat_offer:` key produces.
    let mut kv = BTreeMap::new();
    fold_kv(&mut tw, &mut kv);
    patch_offer(&mut kv, op, seat, |offer| {
        offer.scope.channels.insert(SmolStr::new_static("fire"));
    });
    reboot(&mut tw, &kv);
    assert!(
        camp(&tw)
            .seat_offer(op, seat)
            .is_some_and(mush_campaign::SeatOffer::consent_intact),
        "the offer re-hashes to itself, which is all cut 10 ever checked"
    );
    let refused = err(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    assert_eq!(refused.trim(), mush_campaign::NO_SUCH_SEAT, "{refused}");
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()],
        "the widened scope was never applied"
    );
}

/// **A15. An offer naming no station, or no channel the station publishes,
/// refuses at PUBLISH** — both arms. Cut 10 published, consent-hashed and let a
/// driver accept a dead seat, discovering it only at activation where the
/// refusal collapses to the existence-hiding line.
#[test]
fn vf_a15_an_offer_naming_no_station_or_no_published_channel_refuses_at_publish() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let before = camp(&tw).seat_offers().count();
    let (took_station, refused) = run(
        &mut tw,
        fx.commander,
        &format!(
            "+seat/offer {}/#{}/gunner=#{},movement,to:#{}",
            op.0, fx.atlas.0, fx.ace.0, fx.driver.0
        ),
    );
    assert!(
        !took_station,
        "a station the unit does not carry: {refused}"
    );
    assert_eq!(refused.trim(), mush_campaign::NO_SUCH_SEAT);
    let (took_channel, refused) = run(
        &mut tw,
        fx.commander,
        &format!(
            "+seat/offer {}/#{}/pilot=#{},scuttle,to:#{}",
            op.0, fx.atlas.0, fx.ace.0, fx.driver.0
        ),
    );
    assert!(!took_channel, "a channel the station does not publish");
    assert_eq!(refused.trim(), mush_campaign::NO_SUCH_SEAT);
    assert_eq!(
        camp(&tw).seat_offers().count(),
        before,
        "no dead seat was published or consent-hashed"
    );
}

/// **K2a. A defender musters only its OWN units and learns no proxy seat.**
///
/// `cmd_muster` gated on `lens == Lens::Plan` and nothing else, and `lens_for`
/// grants `Plan` to any named side principal — the declared defender — while
/// `take_muster` walks BOTH sides' participants and classifies each slot from
/// `ActiveFocus.proxy`. So cut 10 handed the enemy which of your ships were on a
/// skeleton crew and which stations were empty, at the same lens where
/// `render::render` publishes only `Participants: N of M` and names no unit.
#[test]
fn vf_k2a_a_defender_musters_only_its_own_units_and_learns_no_proxy_seat() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    name_defender(&mut tw, &fx, op, fx.bystander);
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
    // What the operation's own Plan render gives the defender: a count.
    let plan = ok(&mut tw, fx.bystander, &format!("@op/show/full {}", op.0));
    assert!(plan.contains("Participants: 2 of"), "{plan}");
    assert!(!plan.contains("Atlas"), "{plan}");
    // The muster must not widen that by one word.
    let shown = ok(&mut tw, fx.bystander, &format!("+muster {}", op.0));
    for leak in [
        "Atlas",
        "Locust",
        "human-directed",
        "AI-controlled",
        "VACANT",
    ] {
        assert!(
            !shown.contains(leak),
            "the defender learned {leak:?}: {shown}"
        );
    }
    // …while the commander, who may command both, still sees the whole thing.
    let mine = ok(&mut tw, fx.commander, &format!("+muster {}", op.0));
    assert!(mine.contains("Atlas"), "{mine}");
    assert!(mine.contains("a human-directed NPC"), "{mine}");
}

/// **K2b. A defender cannot lock the attacker's roster** — the write gate is a
/// command predicate, not the disclosure lens (cut 9's fold C1, one cut on).
#[test]
fn vf_k2b_a_defender_cannot_lock_the_attackers_roster() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    name_defender(&mut tw, &fx, op, fx.bystander);
    let before = camp(&tw).latest_muster(op).map(|r| r.cycle);
    let refused = err(&mut tw, fx.bystander, &format!("+muster/lock {}", op.0));
    assert_eq!(
        refused.trim(),
        mush_campaign::NO_SUCH_OPERATION,
        "{refused}"
    );
    assert_eq!(camp(&tw).latest_muster(op).map(|r| r.cycle), before);
    assert!(
        camp(&tw)
            .latest_muster(op)
            .is_none_or(|report| !report.locked)
    );
}

/// **K4. `sim.station.served` cannot be named from public coordinates.** The
/// three object ids alone answer `None`; only the grant's own fourth coordinate
/// — the instant service began, published nowhere — names the fact.
#[test]
fn vf_k4_station_served_cannot_be_named_from_public_coordinates() {
    use mush_economy::FactProvider;
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
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
    let began = tw
        .world()
        .module_state::<mush_sim::FocusState>()
        .and_then(|s| s.focus_of_actor(fx.ace).map(|f| f.acquired_at.at))
        .expect("the seat is live");
    tw.set_now(tw.world().now() + 600);
    ok(&mut tw, fx.driver, "+seat/leave");
    drain(&mut tw);
    let provider = mush_sim::SimFactProvider;
    // Three guessable public object ids: nothing.
    assert_eq!(
        provider.resolve(
            tw.world(),
            "sim.station.served",
            &format!("{}/pilot/{}", fx.atlas.0, fx.ace.0),
            None
        ),
        None,
        "an existence-and-timing oracle from three public ids"
    );
    // A guessed fourth term: still nothing.
    assert_eq!(
        provider.resolve(
            tw.world(),
            "sim.station.served",
            &format!("{}/pilot/{}/{}", fx.atlas.0, fx.ace.0, began + 1),
            None
        ),
        None
    );
    // The grant's own coordinates: the fact.
    assert!(
        provider
            .resolve(
                tw.world(),
                "sim.station.served",
                &format!("{}/pilot/{}/{began}", fx.atlas.0, fx.ace.0),
                None
            )
            .is_some_and(|state| state.satisfied)
    );
}

/// **K6. A staff verb can actually attribute a live seat** — SAME for the
/// driver's own character, DIFFERENT for another, UNKNOWN with no row, and no
/// account id and no token in any of the three renders.
#[test]
fn vf_k6_a_staff_verb_can_actually_attribute_a_live_seat() {
    let audit = Arc::new(TestAudit::default());
    let (mut tw, fx) = scene(audit.clone());
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let token = token_of(&tw, op, seat);
    let hex = fx
        .oracle
        .0
        .lock()
        .ok()
        .and_then(|m| m.get(&fx.driver).copied())
        .map(|id| serde_json::to_string(&id).expect("ser"))
        .map(|s| s.trim_matches('"').to_owned())
        .expect("mapped");
    let same = ok(
        &mut tw,
        GOD,
        &format!("@seat/audit {}/{}=#{}", op.0, seat.0, fx.driver.0),
    );
    assert!(same.contains("SAME"), "{same}");
    let different = ok(
        &mut tw,
        GOD,
        &format!("@seat/audit {}/{}=#{}", op.0, seat.0, fx.bystander.0),
    );
    assert!(different.contains("DIFFERENT"), "{different}");
    assert!(audit.forget(&token));
    let unknown = ok(
        &mut tw,
        GOD,
        &format!("@seat/audit {}/{}=#{}", op.0, seat.0, fx.driver.0),
    );
    assert!(unknown.contains("UNKNOWN"), "{unknown}");
    for render in [&same, &different, &unknown] {
        assert!(
            !render.contains(&hex),
            "an account id was printed: {render}"
        );
        assert!(!render.contains(&token), "a token was printed: {render}");
    }
}

/// **K7. The permitted-area refusal is byte-identical to the existence-hiding
/// refusal**, so a grant-holder polling it learns nothing about where the unit
/// is. Cut 10 answered with a distinct line that flipped at the instant of
/// arrival — a fogged-position bit about a unit the driver may not perceive.
#[test]
fn vf_k7_the_area_refusal_is_byte_identical_to_the_existence_hiding_refusal() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let elsewhere = tw
        .world_mut()
        .create(ObjKind::Room, "Elsewhere", GOD, None)
        .unwrap();
    ok(
        &mut tw,
        fx.commander,
        &format!(
            "+seat/offer {}/#{}/pilot=#{},movement,to:#{},area:#{}",
            op.0, fx.atlas.0, fx.ace.0, fx.driver.0, elsewhere.0
        ),
    );
    let seat = camp(&tw)
        .seat_offers()
        .map(|o| o.id)
        .max()
        .expect("an offer");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let area = err(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    let absent = err(&mut tw, fx.driver, &format!("+seat/activate {}/9999", op.0));
    assert_eq!(area, absent, "the area refusal is a position oracle");
}

/// **K8a. `+seat/list` shows only your own audience, and a revoked seat reads as
/// FREE.** `taken` was `grant.is_some()`, true for `Revoked` and `Expired` too,
/// so a freed seat read as taken for ever and a revocation was invisible to the
/// crew call it frees.
#[test]
fn vf_k8a_seat_list_shows_only_your_own_audience_and_a_revoked_seat_reads_as_free() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    assert!(
        ok(&mut tw, fx.driver, "+crewcall/list").contains("station `pilot`"),
        "the crew call is the same listing"
    );
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    assert!(ok(&mut tw, fx.driver, "+seat/list").contains("(taken)"));
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    drain(&mut tw);
    ok(
        &mut tw,
        fx.commander,
        &format!("@seat/revoke {}/{}", op.0, seat.0),
    );
    drain(&mut tw);
    let listed = ok(&mut tw, fx.driver, "+seat/list");
    assert!(listed.contains("station `pilot`"), "{listed}");
    assert!(
        !listed.contains("(taken)"),
        "a revoked seat still reads as taken: {listed}"
    );
    // …and the stranger still sees nothing at all.
    assert_eq!(
        ok(&mut tw, fx.bystander, "+seat/list").trim(),
        "No seats are offered to you."
    );
}

/// **K8b. Offers the viewer may not see do not hide a visible one.**
///
/// `seat_list` took a global 256-row prefix in KEY ORDER and only then asked the
/// lens, so invisible offers consumed a visible viewer's budget and, past the
/// prefix, a viewer's own operation's offers fell outside it entirely and the
/// listing answered "No seats are offered to you." for a seat they held.
#[test]
fn vf_k8b_offers_the_viewer_may_not_see_do_not_hide_a_visible_one() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    // A covert operation whose offers sort FIRST, filling the old prefix.
    let hidden = operation_bare(&mut tw, &fx);
    ok(
        &mut tw,
        fx.commander,
        &format!("@op/terms {}/visibility=covert", hidden.0),
    );
    for _ in 0..mush_campaign::MAX_SEAT_OFFERS_PER_OPERATION {
        ok(
            &mut tw,
            fx.commander,
            &format!(
                "+seat/offer {}/#{}/pilot=#{},movement",
                hidden.0, fx.atlas.0, fx.ace.0
            ),
        );
    }
    // …then the viewer's own operation, whose offers sort after all of them.
    let op = operation(&mut tw, &fx);
    ok(
        &mut tw,
        fx.commander,
        &format!(
            "+seat/offer {}/#{}/pilot=#{},movement,to:#{}",
            op.0, fx.atlas.0, fx.ace.0, fx.driver.0
        ),
    );
    let seat = camp(&tw)
        .seat_offers()
        .filter(|o| o.operation == op)
        .map(|o| o.id)
        .max()
        .expect("an offer");
    let listed = ok(&mut tw, fx.driver, "+seat/list");
    assert!(
        listed.contains(&format!("{seat} — operation {op}")),
        "the viewer's own seat fell outside a prefix of seats they may not see: {listed}"
    );
    assert_eq!(
        listed.lines().filter(|l| l.contains("station")).count(),
        1,
        "and none of the covert operation's own seats are listed: {listed}"
    );
}

/// **B1. The muster book is a RING** (ENCODED through the real command path).
///
/// `install_muster` inserted at `(operation, cycle)` with no cap, no ring and no
/// eviction; `muster_cycle` incremented per call, nothing ever deleted one, and
/// `store::load` hydrated every one it found — so any Plan-lens viewer grew the
/// durable store and boot-time RAM without limit. Only `latest_muster` ever
/// reads the collection.
#[test]
fn vf_b1_the_muster_book_is_a_ring() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    for _ in 0..(mush_campaign::MAX_RETAINED_MUSTERS + 5) {
        ok(&mut tw, fx.commander, &format!("+muster {}", op.0));
    }
    let newest = camp(&tw).latest_muster(op).expect("a report").cycle;
    let mut kv = BTreeMap::new();
    fold_kv(&mut tw, &mut kv);
    let keys: Vec<SmolStr> = kv
        .get("campaign")
        .map(|entries| {
            entries
                .iter()
                .map(|(k, _)| k.clone())
                .filter(|k| k.starts_with("muster:"))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        keys.len(),
        mush_campaign::MAX_RETAINED_MUSTERS,
        "the superseded reports were orphaned rather than Deleted: {keys:?}"
    );
    assert!(keys.contains(&mush_campaign::muster_key(op, newest)));
    // A reboot hydrates exactly those, and the newest still answers.
    reboot(&mut tw, &kv);
    assert_eq!(camp(&tw).latest_muster(op).map(|r| r.cycle), Some(newest));
}

/// **B2. A stale grant past the seat budget still expires** (ENCODED).
///
/// `expire_grants` read `.filter(!is_terminal).take(budget).filter(stale)` with
/// no cursor, so with `MAX_SEATS_PER_FIRE` healthy grants ahead of it a grant on
/// an aborted operation never expired and the reaper re-visited the same prefix
/// for ever — and its audit row was never forgotten either.
#[test]
fn vf_b2_a_stale_grant_past_the_seat_budget_still_expires() {
    let audit = Arc::new(TestAudit::default());
    let (mut tw, fx) = scene(audit.clone());
    // A live operation carrying a full budget of healthy dormant grants…
    let live = operation(&mut tw, &fx);
    for _ in 0..mush_campaign::MAX_SEATS_PER_FIRE {
        let seat = offer(&mut tw, &fx, live, fx.atlas, fx.ace, "movement");
        ok(
            &mut tw,
            fx.driver,
            &format!("+seat/accept {}/{}", live.0, seat.0),
        );
    }
    // …and one grant on an ABORTED operation, sorted last.
    let doomed = operation_bare(&mut tw, &fx);
    assert!(doomed.0 > live.0, "the stale grant must sort last");
    ok(
        &mut tw,
        fx.commander,
        &format!(
            "+seat/offer {}/#{}/pilot=#{},movement,to:#{}",
            doomed.0, fx.locust.0, fx.duce.0, fx.driver.0
        ),
    );
    let seat = camp(&tw)
        .seat_offers()
        .filter(|o| o.operation == doomed)
        .map(|o| o.id)
        .max()
        .expect("an offer");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", doomed.0, seat.0),
    );
    let token = token_of(&tw, doomed, seat);
    ok(&mut tw, GOD, &format!("@op/resolve {}=abort", doomed.0));
    for _ in 0..6 {
        fire(&mut tw);
        drain(&mut tw);
    }
    assert!(
        camp(&tw)
            .operation(doomed)
            .is_some_and(|doc| doc.runtime.state.is_terminal())
    );
    assert_eq!(
        grant_state(&tw, doomed, seat),
        Some(GrantState::Expired),
        "the reaper starved for ever behind a full budget of healthy grants"
    );
    assert!(!audit.knows(&token));
}

/// **B5a. An incomplete lease mirror refuses rather than answering "vacant".**
///
/// `channel_lease_holder` read `ControlLeaseView.holders` and never `complete`
/// (`is_complete()` had ZERO production callers tree-wide), so between boot and
/// the first Coarse fire leg (2) read a mirror that had never run as
/// authoritative ABSENCE — while `control_leases` is durable K state that
/// survived the reboot.
#[test]
fn vf_b5a_an_incomplete_lease_mirror_refuses_rather_than_answering_vacant() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    // A freshly booted process: the mirror has not run yet, though the durable
    // `control_leases` family it mirrors survived the reboot.
    tw.world_mut()
        .module_state_mut::<mush_sim::ControlLeaseView>()
        .expect("the lease mirror")
        .note_unread();
    let refused = err(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    assert!(refused.contains("try again in a moment"), "{refused}");
    assert_eq!(grant_state(&tw, op, seat), Some(GrantState::Dormant));
    assert_eq!(
        granted(&tw, fx.ace, fx.atlas, "pilot").len(),
        2,
        "the command must not mutate the world and then discover it should not have"
    );
    // One Coarse fire and the same activation succeeds.
    fire(&mut tw);
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    drain(&mut tw);
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot"), vec!["movement"]);
}

/// **B5c. The leg-(2) refusal names no holder**, whoever the conflicting holder
/// is — an AI NPC's, another player's, or an unknown principal's.
#[test]
fn vf_b5c_the_leg_two_refusal_names_no_holder() {
    let mut seen: Vec<String> = Vec::new();
    for holder in 0..3usize {
        let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
        let op = operation(&mut tw, &fx);
        let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
        ok(
            &mut tw,
            fx.driver,
            &format!("+seat/accept {}/{}", op.0, seat.0),
        );
        let who = [fx.duce, fx.bystander, ObjId(9999)][holder];
        tw.world_mut()
            .module_state_mut::<mush_sim::ControlLeaseView>()
            .expect("the lease mirror")
            .note_holder(
                fx.atlas,
                ChannelId::new("movement").unwrap(),
                mush_core::PrincipalId::Object { id: who },
            );
        seen.push(err(
            &mut tw,
            fx.driver,
            &format!("+seat/activate {}/{}", op.0, seat.0),
        ));
    }
    assert_eq!(seen[0], seen[1], "the refusal distinguishes holders");
    assert_eq!(seen[1], seen[2]);
    for text in &seen {
        assert!(
            !text.contains("Duce") && !text.contains("Bystander"),
            "{text}"
        );
    }
}

/// **B7. A grant whose audit row is lost can be re-accepted** — the runbook's
/// own first instruction. `seat_accept`'s third check refused whenever a grant
/// existed at all, whatever its state, so the page's `+seat/accept {op}/{id}`
/// could never succeed, and it failed with the existence-hiding line so the
/// driver could not tell why.
#[test]
fn vf_b7_a_grant_whose_audit_row_is_lost_can_be_re_accepted() {
    let audit = Arc::new(TestAudit::default());
    let (mut tw, fx) = scene(audit.clone());
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let old = token_of(&tw, op, seat);
    assert!(audit.forget(&old), "the restart the runbook is written for");
    // The runbook's own instruction, verbatim.
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    let fresh = token_of(&tw, op, seat);
    assert_ne!(fresh, old, "the token is re-minted");
    assert!(!audit.knows(&old));
    assert!(audit_is(&audit, &fx.oracle, &fresh, fx.driver));
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    drain(&mut tw);
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot"), vec!["movement"]);
    // …and an ACTIVE grant with a live row is still refused flatly.
    let refused = err(
        &mut tw,
        fx.driver,
        &format!("+seat/accept {}/{}", op.0, seat.0),
    );
    assert_eq!(refused.trim(), mush_campaign::NO_SUCH_SEAT);
}

/// **B10. A grant past the per-operation cap is refused** (ENCODED through the
/// real command path). `MAX_SEAT_GRANTS_PER_OPERATION` was declared,
/// re-exported and enforced nowhere.
#[test]
fn vf_b10_a_grant_past_the_per_operation_cap_is_refused() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    // The offer cap and the grant cap are the same number, and the offer cap is
    // enforced at mint — so the structural bound and the counted one agree, and
    // the counted one now has its own refusal.
    for _ in 0..mush_campaign::MAX_SEAT_OFFERS_PER_OPERATION {
        let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
        ok(
            &mut tw,
            fx.driver,
            &format!("+seat/accept {}/{}", op.0, seat.0),
        );
    }
    assert_eq!(
        camp(&tw).seat_grant_count(op),
        mush_campaign::MAX_SEAT_GRANTS_PER_OPERATION
    );
    let refused = err(
        &mut tw,
        fx.commander,
        &format!(
            "+seat/offer {}/#{}/pilot=#{},movement,to:#{}",
            op.0, fx.atlas.0, fx.ace.0, fx.driver.0
        ),
    );
    assert!(refused.contains("seat offers"), "{refused}");
}

/// **B11. A grant recorded under a stale epoch is refused.** §3.10 B7 justified
/// `Active { epoch }` as "the exact value `World::channel_grant_is_current`
/// compares, so a revocation's bump makes it stale by construction" — and no
/// such comparison existed anywhere in the tree.
#[test]
fn vf_b11_a_grant_recorded_under_a_stale_epoch_is_refused() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
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
    let now = tw
        .world()
        .module_state::<mush_sim::SpatialState>()
        .map(|state| state.control_generation(fx.atlas))
        .expect("a control generation");
    // The `Dormant -> Active` promotion is the tick's own pull over a live
    // lease; what B11 is about is the RECORD, so it is written through the
    // grant's own strict wire and re-read at boot.
    let mut kv = BTreeMap::new();
    fold_kv(&mut tw, &mut kv);
    patch_grant_state(&mut kv, op, seat, now);
    reboot(&mut tw, &kv);
    assert!(matches!(
        grant_state(&tw, op, seat),
        Some(GrantState::Active { .. })
    ));
    // A grant whose epoch IS the unit's current control generation is live…
    let live = err(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    assert!(live.contains("already driving"), "{live}");
    // …and the same record under a generation the unit has bumped past is
    // STALE, which is exactly what §3.10 B7 claimed and nothing compared.
    patch_grant_state(&mut kv, op, seat, now.saturating_sub(1));
    reboot(&mut tw, &kv);
    let stale = err(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    assert_eq!(stale.trim(), mush_campaign::NO_SUCH_SEAT, "{stale}");
}

/// **B14. A second activate before the tick is a no-op.** `Active` is written
/// only by the 15 s campaign tick, so for up to one whole period a second
/// `+seat/activate` passed every check and re-ran the focus bump — and `cgen` is
/// per UNIT, so each re-run staled every OTHER crew member's captured intents as
/// well as the driver's own.
#[test]
fn vf_b14_a_second_activate_before_the_tick_is_a_no_op() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
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
    let epoch = epoch_of(&tw, fx.ace, fx.atlas, "pilot").expect("a grant");
    // 16b cut K.2: this fixture's station is two-channel and a K6 lease is per
    // channel, so the activation stages nothing to request (see
    // `e7c10_k6_an_llm_driven_seat_is_k7_gated_and_cut_ten_mints_no_decision`).
    // The window this test is about is the one below it: the grant is `Dormant`
    // until the tick, and a second activate inside it must change nothing.
    drain(&mut tw);
    // No fire between: the grant is still `Dormant`, which is the whole window.
    assert_eq!(grant_state(&tw, op, seat), Some(GrantState::Dormant));
    ok(
        &mut tw,
        fx.driver,
        &format!("+seat/activate {}/{}", op.0, seat.0),
    );
    assert_eq!(
        epoch_of(&tw, fx.ace, fx.atlas, "pilot"),
        Some(epoch),
        "the second activate re-bumped the whole unit's control generation"
    );
    assert!(
        drain(&mut tw).is_empty(),
        "the second activate staged a fresh submission"
    );
}

/// **A11. A proxy driver is refused the unit's own channel, and the refusal is
/// recorded as INERTNESS rather than narrowness** (decision E6(b)).
///
/// Every station-gated verb resolves through the **typing** actor
/// (`focused_station_of(w, ctx.actor)`, `holds_billet(ctx, ctx.actor, ..)`,
/// `station::actor_grant_epoch(ctx, ctx.actor, ..)`,
/// `World::command_subjects_of(ctx.actor)`), and a proxy's `ctx.actor` is their
/// own character, which holds no billet and no `ChannelGrant` on the seat's unit
/// — `links::desired_links` keys the grant on `assignment.actor`, i.e. the NPC.
/// Nothing in `mush-session`/`mush-server` maps an account's proxy focus onto its
/// command actor, so **cut 10 ships the seat's consent, attribution, narrowing,
/// gate and give-back, but NOT its driving ingress**. That is what `e7c10_g6`'s
/// byte-identical refusal actually proves, and this test pins the *tell*: after
/// activation the driver's own command-subject set is still **empty**, so the
/// nine T11 refusals fall on an actor that never had the authority.
///
/// **E7 cut 11 B0 — re-run and kept, deliberately.** The driving ingress landed
/// (`mush_sim::driving_actor`, deviation (h)'s narrowing with it), and this
/// assertion does **not** flip: authority stays on the NPC's own `ChannelGrant`
/// per T16, so no grant is ever published to the driver and their own
/// command-subject set is still empty. What changed is the *reading*: after cut
/// 11 the driver genuinely drives, and `crates/mush-campaign/tests/e7c11.rs`
/// proves narrowness rather than inertness (a `fire`-only seat cannot steer and
/// a `movement`-only seat cannot shoot, both through the real command path).
/// `e7c10_g6` was re-run with the ingress live and is unchanged.
#[test]
fn vf_a11_the_driver_is_refused_the_units_own_channel_and_the_refusal_is_inertness() {
    let (mut tw, fx) = scene(Arc::new(TestAudit::default()));
    let op = operation(&mut tw, &fx);
    let seat = offer(&mut tw, &fx, op, fx.atlas, fx.ace, "movement");
    let before = tw.world().command_subjects_of(fx.driver).len();
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
    // The seat is real: the NPC publishes exactly the granted channel.
    assert_eq!(granted(&tw, fx.ace, fx.atlas, "pilot"), vec!["movement"]);
    // …and the human who took it holds nothing at all — not the GRANTED channel
    // and not the withheld one, which is why the two explanations are
    // indistinguishable today.
    assert_eq!(
        tw.world().command_subjects_of(fx.driver).len(),
        before,
        "authority must stay on the NPC's own grant (T16): the E7c11 B0 ingress          resolves the ACTOR, it never publishes a grant to the driver"
    );
    assert!(granted(&tw, fx.driver, fx.atlas, "pilot").is_empty());
    let epoch = epoch_of(&tw, fx.ace, fx.atlas, "pilot").expect("a grant");
    assert!(!is_current(
        &tw, fx.driver, fx.atlas, "pilot", epoch, "movement"
    ));
    assert!(!is_current(
        &tw, fx.driver, fx.atlas, "pilot", epoch, "fire"
    ));
    // The character stays put, present and unable to act (T13).
    assert_eq!(
        tw.world().get(fx.driver).and_then(|o| o.location),
        Some(fx.field)
    );
}
