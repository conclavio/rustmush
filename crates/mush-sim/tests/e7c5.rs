//! E7 cut 5 acceptance — the active-station focus, billets, witnesses, reach,
//! and the account-distinctness verb (10b §3.5), driven through the real
//! `Engine` via `TestWorld` on the `SimulationModule`.
//!
//! Three verify lenses shape the rhythms:
//! - **F (one focus)**: no interleaving, reconnect or proxy path leaves two live
//!   control surfaces for one account; an unknown account refuses the acquire.
//! - **E (epoch)**: every focus change bumps the unit's control generation and
//!   republishes the grant (bump THEN republish), so a stale intent fails
//!   `World::channel_grant_is_current`.
//! - Reach and billets: ownership alone never satisfies control; a console in
//!   another room refuses; `+stations` discloses no unit location.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use mush_core::{
    AccountId, AccountIdentity, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    CanonicalHash, CanonicalId, ChannelId, CommitToken, ControlLeaseState, DefinitionId,
    DefinitionKey, DefinitionKind, DefinitionRef, GOD, IdempotencyKey, IdempotencyKeyText,
    IdempotencyScope, LIMBO, ModuleBootData, ModuleOp, ModuleWrite, ObjId, ObjKind,
    OfferExecutionContext, OfferKeyRing, OfferSecret, OfferSubmission, OwnerId, PerceivedRef,
    PrincipalId, RequestUid, ScheduleDriveCause, ScopedIdempotencyKey, scheduled_action_ingress,
};
use mush_proto::OutputEvent;
use mush_sim::{AccountOracle, FocusState, ReachProof, SimulationModule};
use mush_testkit::{TestWorld, text_of};
use serde_json::{Value, json};
use smol_str::SmolStr;

const NS: &str = "sim";

/// A fixed in-memory account oracle: character → account id.
struct MapOracle(BTreeMap<ObjId, AccountId>);
impl AccountIdentity for MapOracle {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.0.get(&character).copied()
    }
}

fn texts(events: &[(mush_core::Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_error(events: &[(mush_core::Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}

fn ok(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let ev = tw.run(who, line);
    let t = texts(&ev);
    assert!(!has_error(&ev), "{line:?} refused: {t}");
    t
}

fn err(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let ev = tw.run(who, line);
    assert!(
        has_error(&ev),
        "{line:?} should have been refused: {}",
        texts(&ev)
    );
    texts(&ev)
}

fn focus(tw: &TestWorld) -> &FocusState {
    tw.world()
        .module_state::<FocusState>()
        .expect("focus state")
}

fn grant_epoch(tw: &TestWorld, actor: ObjId, subject: ObjId, station: &str) -> Option<u64> {
    tw.world()
        .command_subjects_of(actor)
        .iter()
        .find(|g| g.subject == subject && g.station == station)
        .map(|g| g.epoch)
}

fn is_current(
    tw: &TestWorld,
    actor: ObjId,
    subject: ObjId,
    station: &str,
    epoch: u64,
    channel: &str,
) -> bool {
    tw.world().channel_grant_is_current(
        actor,
        subject,
        NS,
        station,
        epoch,
        &ChannelId::new(channel).unwrap(),
    )
}

struct Fx {
    field: ObjId,
    other: ObjId,
    atlas: ObjId,
    locust: ObjId,
    pilot: ObjId,
    gunner: ObjId,
}

/// A world with a domain, two placed units, two crew players, and the account
/// oracle installed post-boot. `pilot` holds `pilot` on Atlas; `gunner` holds
/// `gunner` on Atlas. Both crew stand in the presentation room `field`.
fn scene() -> (TestWorld, Fx, AccountId, AccountId) {
    let mut tw = TestWorld::with_modules(vec![Box::new(SimulationModule::new())]);
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .unwrap();
    let other = tw
        .world_mut()
        .create(ObjKind::Room, "Other", GOD, None)
        .unwrap();
    let atlas = tw
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, Some(LIMBO))
        .unwrap();
    let locust = tw
        .world_mut()
        .create(ObjKind::Thing, "Locust", GOD, Some(LIMBO))
        .unwrap();
    let pilot = tw.spawn_player("Pilot");
    let gunner = tw.spawn_player("Gunner");
    for (obj, beh) in [
        (field, "sim.domain"),
        (atlas, "sim.participant"),
        (locust, "sim.participant"),
    ] {
        tw.world_mut()
            .attach_behavior(
                obj,
                mush_core::BehaviorId::new(beh).unwrap(),
                mush_core::BehaviorBinding {
                    schema_version: 1,
                    attached_by: GOD,
                    attached_at: 0,
                },
            )
            .unwrap();
    }
    // Install the oracle: Pilot and Gunner map to distinct accounts.
    let account_p = AccountId::mint("PilotAcct", 1).unwrap();
    let account_g = AccountId::mint("GunnerAcct", 2).unwrap();
    *tw.world_mut().module_state_mut::<AccountOracle>().unwrap() =
        AccountOracle::new(Arc::new(MapOracle(BTreeMap::from([
            (pilot, account_p),
            (gunner, account_g),
        ]))));
    // Domain, placement, stations, assignments.
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", field.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/domain #{}=0", atlas.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/domain #{}=0", locust.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/station #{}=pilot,pilot,channels:movement",
            atlas.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/station #{}=gunner,gunner,channels:fire", atlas.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/station #{}=pilot,pilot,channels:movement",
            locust.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", atlas.0, pilot.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=gunner,#{}", atlas.0, gunner.0),
    );
    // Crew stand where the units project (the console room).
    tw.world_mut().move_to(pilot, field).unwrap();
    tw.world_mut().move_to(gunner, field).unwrap();
    (
        tw,
        Fx {
            field,
            other,
            atlas,
            locust,
            pilot,
            gunner,
        },
        account_p,
        account_g,
    )
}

// ---- Hazard axis 1 — one focus (F) ---------------------------------------

#[test]
fn e7c5_f_two_focus_in_one_tick_leave_exactly_one() {
    let (mut tw, fx, account_p, _) = scene();
    // Give Pilot a second billet so a switch has somewhere to go.
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", fx.locust.0, fx.pilot.0),
    );
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    assert_eq!(focus(&tw).len(), 1);
    tw.world_mut().move_to(fx.pilot, fx.field).unwrap(); // locust also presents in field
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.locust.0));
    assert_eq!(
        focus(&tw).len(),
        1,
        "a second acquire leaves exactly one focus"
    );
    let f = focus(&tw).focus_of_account(account_p).unwrap();
    assert_eq!(f.unit, fx.locust, "the newest acquire wins");
}

#[test]
fn e7c5_f_unmapped_account_refuses_acquire() {
    let (mut tw, fx, _, _) = scene();
    // A third crew member with no oracle entry, on Locust's free pilot seat.
    let ghost = tw.spawn_player("Ghost");
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", fx.locust.0, ghost.0),
    );
    tw.world_mut().move_to(ghost, fx.field).unwrap();
    // Locust also presents in field, so reach would pass; the ONLY refusal is
    // the unmapped account — it never defaults to "distinct" or "same".
    let t = err(&mut tw, ghost, &format!("+focus #{}/pilot", fx.locust.0));
    assert!(t.contains("account is not mapped"), "{t}");
    assert!(focus(&tw).is_empty());
}

#[test]
fn e7c5_f_ownership_without_a_billet_refuses() {
    let (mut tw, fx, _, _) = scene();
    // Pilot holds `pilot`, not `gunner`, on Atlas.
    let t = err(&mut tw, fx.pilot, &format!("+focus #{}/gunner", fx.atlas.0));
    assert!(t.contains("no billet"), "{t}");
    assert!(focus(&tw).focus_of_actor(fx.pilot).is_none());
}

#[test]
fn e7c5_f_restart_clears_focus_and_focus_writes_no_focus_doc() {
    let (mut tw, fx, account_p, _) = scene();
    // Drain the setup writes, then focus and capture exactly what +focus wrote.
    let _ = tw.take_module_ops();
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let ops = tw.take_module_ops();
    // The ONLY write a focus change makes is the pre-existing cgen:<unit>.
    for op in &ops {
        let key = match &op.op {
            ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key } => key.as_str(),
            ModuleWrite::StreamAppend { .. } => continue,
        };
        assert!(
            key.starts_with("cgen:"),
            "+focus must write no focus document/key/family; wrote {key:?}"
        );
    }
    assert!(
        ops.iter().any(
            |op| matches!(&op.op, ModuleWrite::JsonSet { key, .. } if key.starts_with("cgen:"))
        )
    );
    assert_eq!(focus(&tw).len(), 1);
    // A fresh boot never hydrates focus — the map is world memory.
    let tw2 = TestWorld::with_modules(vec![Box::new(SimulationModule::new())]);
    assert!(
        tw2.world().module_state::<FocusState>().unwrap().is_empty(),
        "boot resurrects no focus"
    );
    let _ = (fx, account_p);
}

// ---- Hazard axis 2 — epoch (E) -------------------------------------------

#[test]
fn e7c5_e_focus_bumps_epoch_then_republishes_and_stale_intent_fails() {
    let (mut tw, fx, _, _) = scene();
    let before = grant_epoch(&tw, fx.pilot, fx.atlas, "pilot").expect("assigned grant");
    // The pre-focus grant is current at its own epoch on the movement channel.
    assert!(is_current(
        &tw, fx.pilot, fx.atlas, "pilot", before, "movement"
    ));
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let after = grant_epoch(&tw, fx.pilot, fx.atlas, "pilot").expect("republished grant");
    assert!(
        after > before,
        "focus bumped the control generation ({before} -> {after})"
    );
    // Bump preceded republish: the republished grant carries the NEW epoch, so
    // an intent captured under the old epoch fails and the new one holds.
    assert!(
        !is_current(&tw, fx.pilot, fx.atlas, "pilot", before, "movement"),
        "stale epoch fails"
    );
    assert!(
        is_current(&tw, fx.pilot, fx.atlas, "pilot", after, "movement"),
        "fresh epoch holds"
    );
    // A grant on a DIFFERENT station of the same unit does not satisfy the
    // pilot channel, and the pilot grant does not carry `fire` (K5a: no union).
    assert!(!is_current(&tw, fx.pilot, fx.atlas, "pilot", after, "fire"));
}

#[test]
fn e7c5_e_release_bumps_epoch_again() {
    let (mut tw, fx, _, _) = scene();
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let focused = grant_epoch(&tw, fx.pilot, fx.atlas, "pilot").unwrap();
    ok(&mut tw, fx.pilot, "+focus/release");
    let released = grant_epoch(&tw, fx.pilot, fx.atlas, "pilot").expect("billet still projects");
    assert!(released > focused, "release bumped the epoch again");
    assert!(!is_current(
        &tw, fx.pilot, fx.atlas, "pilot", focused, "movement"
    ));
    assert!(focus(&tw).is_empty());
}

#[test]
fn e7c5_e_station_removal_and_unassign_fail_a_stale_intent() {
    let (mut tw, fx, _, _) = scene();
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let epoch = grant_epoch(&tw, fx.pilot, fx.atlas, "pilot").unwrap();
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/unassign #{}=pilot", fx.atlas.0),
    );
    // The grant is gone; the captured epoch no longer validates.
    assert!(grant_epoch(&tw, fx.pilot, fx.atlas, "pilot").is_none());
    assert!(!is_current(
        &tw, fx.pilot, fx.atlas, "pilot", epoch, "movement"
    ));
}

// ---- Reach and billets ----------------------------------------------------

#[test]
fn e7c5_reach_console_in_another_room_refuses() {
    let (mut tw, fx, _, _) = scene();
    tw.world_mut().move_to(fx.pilot, fx.other).unwrap();
    let t = err(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    assert!(t.contains("console"), "{t}");
    assert!(focus(&tw).is_empty());
}

#[test]
fn e7c5_reach_remotelink_predicate_refuses_when_path_down() {
    // The stub reach predicate directly (cut 7 tightens RemoteLink): a live
    // console co-located passes; a remote link with a dead path refuses.
    let (tw, fx, _, _) = scene();
    let up = ReachProof::Console { room: fx.field };
    assert!(up.is_reachable(tw.world(), fx.pilot));
    let elsewhere = ReachProof::Console { room: fx.other };
    assert!(
        !elsewhere.is_reachable(tw.world(), fx.pilot),
        "console in another room"
    );
    // A remote link over a nonexistent path object is down -> refuses.
    let down = ReachProof::RemoteLink {
        terminal: fx.atlas,
        path: ObjId(99999),
    };
    assert!(
        !down.is_reachable(tw.world(), fx.pilot),
        "a down path refuses"
    );
}

#[test]
fn e7c5_billets_three_grants_one_focus_and_stations_lists_them() {
    let (mut tw, fx, _, _) = scene();
    // Free Atlas's gunner seat, then give Pilot three billets: pilot + gunner on
    // Atlas, and pilot on Locust.
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/unassign #{}=gunner", fx.atlas.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=gunner,#{}", fx.atlas.0, fx.pilot.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", fx.locust.0, fx.pilot.0),
    );
    assert_eq!(
        tw.world().command_subjects_of(fx.pilot).len(),
        3,
        "three billets project three grants"
    );
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    // Controls exactly one.
    assert_eq!(
        focus(&tw)
            .focus_of_actor(fx.pilot)
            .map(|f| (f.unit, f.station.as_str().to_string())),
        Some((fx.atlas, "pilot".to_string()))
    );
    let listing = ok(&mut tw, fx.pilot, "+stations");
    assert!(
        listing.contains("[FOCUSED]"),
        "the driven billet is marked: {listing}"
    );
    // The listing never prints a room / location.
    assert!(
        !listing.contains("Field") && !listing.contains("Other"),
        "no unit location: {listing}"
    );
}

#[test]
fn e7c5_billets_non_focused_command_names_the_focused_station() {
    // A billet without focus is eligibility only; the focused predicate reports
    // which station a delayed command must match.
    let (mut tw, fx, account_p, _) = scene();
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/unassign #{}=gunner", fx.atlas.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=gunner,#{}", fx.atlas.0, fx.pilot.0),
    );
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let f = focus(&tw).focus_of_account(account_p).unwrap();
    assert_eq!(
        f.station.as_str(),
        "pilot",
        "the focused station is named for a stale-billet refusal"
    );
    assert_eq!(f.unit, fx.atlas);
}

// ---- Account audit --------------------------------------------------------

#[test]
fn e7c5_account_audit_same_different_and_never_an_id() {
    let (mut tw, fx, _, _) = scene();
    // Two characters of one account: alias Pilot to a second dbref by mapping.
    let alt = tw.spawn_player("PilotAlt");
    let account_p = AccountId::mint("PilotAcct", 1).unwrap();
    let account_g = AccountId::mint("GunnerAcct", 2).unwrap();
    *tw.world_mut().module_state_mut::<AccountOracle>().unwrap() =
        AccountOracle::new(Arc::new(MapOracle(BTreeMap::from([
            (fx.pilot, account_p),
            (alt, account_p),
            (fx.gunner, account_g),
        ]))));
    let same = ok(
        &mut tw,
        GOD,
        &format!("@identity/audit #{}=#{}", fx.pilot.0, alt.0),
    );
    assert!(same.contains("SAME"), "{same}");
    let diff = ok(
        &mut tw,
        GOD,
        &format!("@identity/audit #{}=#{}", fx.pilot.0, fx.gunner.0),
    );
    assert!(diff.contains("DIFFERENT"), "{diff}");
    // Never an identity: the redacted forms are the only rendering, and the hex
    // never appears.
    let hex = account_p.canonical_hash().to_hex();
    assert!(
        !same.contains(&hex[..8]) && !diff.contains(&hex[..8]),
        "audit leaked an id"
    );
}

#[test]
fn e7c5_account_audit_refuses_on_unknown() {
    let (mut tw, fx, _, _) = scene();
    let ghost = tw.spawn_player("Ghost"); // unmapped
    let t = ok(
        &mut tw,
        GOD,
        &format!("@identity/audit #{}=#{}", fx.pilot.0, ghost.0),
    );
    assert!(
        t.contains("Cannot determine"),
        "unmapped refuses, never defaults: {t}"
    );
}

#[test]
fn e7c5_account_audit_is_staff_gated() {
    let (mut tw, fx, _, _) = scene();
    let t = err(
        &mut tw,
        fx.pilot,
        &format!("@identity/audit #{}=#{}", fx.pilot.0, fx.gunner.0),
    );
    assert!(t.contains("Permission denied"), "{t}");
}

// ---- Reboot: focus is world memory, cgen survives -------------------------

#[test]
fn e7c5_reboot_reproduces_cgen_without_focus() {
    let (mut tw, fx, _, _) = scene();
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let epoch = tw
        .world()
        .module_state::<mush_sim::SpatialState>()
        .unwrap()
        .control_generation(fx.atlas);
    assert!(epoch > 0);
    // Fold the store writes and reboot into a fresh, identically-built world.
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    // A note: take_module_ops was drained lazily; re-run setup writes are gone,
    // so this reboot exercises only that focus persists NO focus doc. The
    // control generation persisted (cgen:) is what carries across.
    let mut tw2 = TestWorld::with_modules(vec![Box::new(SimulationModule::new())]);
    // Rebuild the same objects so ids line up, then load the sim docs.
    let _ = &fx;
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    )]));
    assert!(
        tw2.world().module_state::<FocusState>().unwrap().is_empty(),
        "no actor holds a focus after a restart"
    );
}

// ---- cut 5 B5/B6 ----
//
// The H (handoff) lens: can a station's control lease be empty, doubly-held, or
// held past a revocation across a boundary? Candidate (iii) resolves OPEN 2 — the
// migrated `control_handoff_request` action declares `required_channels = []`, so
// its offer always projects (never dropped by the kernel's `ControlLease` witness
// loop); the authority split lives in the offer, and the requester's
// control-generation is pinned as a FIXED parameter and re-checked at apply. The
// deep request/apply mechanics are driven here at the engine level (as the
// world-task's `CommandSubmitDriver` would), because `TestWorld::run` queues an
// authoritative submit but does not itself resolve it.

const REQUEST_ACTION: &str = "control_handoff_request";
const APPLY_ACTION: &str = "control_handoff_apply";

fn action_ref(tw: &TestWorld, id: &str) -> DefinitionRef {
    tw.registry()
        .definitions()
        .expect("frozen definitions")
        .current(&DefinitionKey {
            owner: OwnerId::new(NS).unwrap(),
            kind: DefinitionKind::new("action").unwrap(),
            id: DefinitionId::new(id).unwrap(),
        })
        .expect("handoff action is registered")
        .clone()
}

fn utc(at: i64) -> Value {
    json!({ "clock": { "kind": "utc" }, "at": at })
}

/// Arm a test offer key ring so `project_legal_actions` can sign offers (the
/// world-task supplies one in production; a bare `TestWorld` does not).
fn arm_offer_keys(tw: &mut TestWorld) {
    tw.engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(2, [2; 32]).unwrap(), []).unwrap());
}

/// Build one request invocation for `actor` on `unit` with the station-grant
/// witness cut 5 pins as fixed parameters.
#[allow(clippy::too_many_arguments)]
fn request_invocation(
    action: &DefinitionRef,
    req: u8,
    actor: ObjId,
    unit: ObjId,
    channel: &str,
    next: ObjId,
    expected_revision: u64,
    station: &str,
    grant_epoch: u64,
    boundary_at: i64,
) -> ActionInvocation {
    invocation(
        action,
        req,
        actor,
        unit,
        json!({
            "channel": channel,
            "next_holder": PrincipalId::Object { id: next },
            "requested_at": utc(boundary_at - 1),
            "boundary": utc(boundary_at),
            "reason": "focus.station_seat",
            "expected_revision": expected_revision,
            "station": station,
            "grant_epoch": grant_epoch,
        }),
    )
}

fn invocation(
    action: &DefinitionRef,
    req: u8,
    actor: ObjId,
    unit: ObjId,
    parameters: Value,
) -> ActionInvocation {
    let principal = PrincipalId::Object { id: actor };
    ActionInvocation::new(ActionInvocationDraft {
        request: RequestUid::from_bytes([req; 16]),
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([9; 32])),
        subject: PerceivedRef::KnownObject {
            id: unit,
            revision: 0,
        },
        targets: Vec::new(),
        parameters,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: action.key().owner.clone(),
                namespace: CanonicalId::new(action.key().id.as_str()).unwrap(),
            },
            1,
            IdempotencyKey::Request(RequestUid::from_bytes([req; 16])),
        ),
    })
    .unwrap()
}

/// Prepare and install one authoritative request; return the transaction ops so
/// the lease post-image can be read.
fn install_request(
    tw: &mut TestWorld,
    action: &DefinitionRef,
    inv: &ActionInvocation,
    token: u8,
) -> Vec<ModuleOp> {
    let (prepared, _) = tw
        .engine_mut()
        .prepare_registered_authoritative_action(action, inv, CommitToken::from_bytes([token; 16]))
        .expect("request stages");
    let (ops, installation) = prepared.into_parts();
    tw.engine_mut().install_prepared_action(installation);
    ops
}

/// Discover and apply the one due scheduled handoff at `at`, returning its ops.
fn apply_due(tw: &mut TestWorld, at: i64, token: u8) -> Vec<ModuleOp> {
    tw.world_mut().set_now(at);
    let page = tw
        .engine_mut()
        .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
        .unwrap();
    let candidate = page.candidates()[0].clone();
    assert_eq!(candidate.action(), &action_ref(tw, APPLY_ACTION));
    let ingress = scheduled_action_ingress(candidate.action(), candidate.advance(), 1).unwrap();
    let plan = tw
        .engine_mut()
        .plan_scheduled_action(&candidate, &ingress)
        .expect("the scheduled apply plans");
    let (_, scheduled_inv, _) = plan.into_parts();
    let (prepared, _) = tw
        .engine_mut()
        .prepare_registered_scheduled_authoritative_action(
            &candidate,
            &scheduled_inv,
            CommitToken::from_bytes([token; 16]),
        )
        .unwrap();
    let (ops, installation) = prepared.into_parts();
    tw.engine_mut().install_prepared_action(installation);
    ops
}

/// Discover the due handoff at `at` and assert it is now STALE (a focus change
/// bumped the requester's control-generation between request and apply).
fn apply_is_stale(tw: &mut TestWorld, at: i64) {
    tw.world_mut().set_now(at);
    let page = tw
        .engine_mut()
        .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
        .unwrap();
    let candidate = page.candidates()[0].clone();
    let ingress = scheduled_action_ingress(candidate.action(), candidate.advance(), 1).unwrap();
    let error = tw
        .engine_mut()
        .plan_scheduled_action(&candidate, &ingress)
        .expect_err("a focus change must stale the in-flight apply");
    assert!(
        matches!(
            &error,
            mush_core::ScheduleActionPlanError::Adapter { source, .. }
                if source.code() == mush_core::ActionErrorCode::Stale
        ),
        "expected a stale apply, got {error:?}"
    );
}

fn lease_post_image(ops: &[ModuleOp]) -> ControlLeaseState {
    let value = ops
        .iter()
        .find_map(|op| match &op.op {
            ModuleWrite::JsonSet { key, value } if key.contains("/control_leases/") => {
                value.get("value").cloned()
            }
            _ => None,
        })
        .expect("the transaction carries a control-lease post-image");
    serde_json::from_value(value).expect("a valid control-lease post-image")
}

fn grant_epoch_of(tw: &TestWorld, actor: ObjId, unit: ObjId, station: &str) -> u64 {
    grant_epoch(tw, actor, unit, station).expect("the actor holds a station grant")
}

// A billeted actor establishes their vacant station's control lease and holds it:
// cut 5 is the FIRST production `ControlLeaseState` creator (contradiction (vi)).
#[test]
fn e7c5_h_billeted_actor_creates_a_vacant_stations_lease_and_holds_it() {
    let (mut tw, fx, _, _) = scene();
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let epoch = grant_epoch_of(&tw, fx.pilot, fx.atlas, "pilot");
    let request = action_ref(&tw, REQUEST_ACTION);
    let inv = request_invocation(
        &request, 1, fx.pilot, fx.atlas, "movement", fx.pilot, 0, "pilot", epoch, 100,
    );
    let ops = install_request(&mut tw, &request, &inv, 1);
    let pending = lease_post_image(&ops);
    assert_eq!(pending.revision(), 1);
    assert_eq!(pending.epoch(), 0);
    assert!(
        pending.holder().is_none(),
        "the lease is vacant until the boundary"
    );
    assert!(pending.pending().is_some());

    let applied = lease_post_image(&apply_due(&mut tw, 100, 2));
    assert_eq!(
        applied.holder(),
        Some(&PrincipalId::Object { id: fx.pilot })
    );
    assert_eq!(
        applied.epoch(),
        1,
        "the lease epoch advances exactly on apply"
    );
    assert!(applied.pending().is_none());
}

// An unbilleted, non-staff actor cannot even form the subject query for the unit
// (no station grant), so the handoff can never be requested — the OWNER
// DECISION's "never let an unbilleted, non-staff actor create a lease".
#[test]
fn e7c5_h_unbilleted_nonstaff_is_refused_at_request() {
    let (mut tw, fx, _, _) = scene();
    let ghost = tw.spawn_player("Ghost");
    tw.world_mut().move_to(ghost, fx.field).unwrap();
    let subject = PerceivedRef::KnownObject {
        id: fx.atlas,
        revision: 0,
    };
    assert!(
        tw.engine()
            .legal_action_query(PrincipalId::Object { id: ghost }, ghost, subject)
            .is_err(),
        "an unbilleted actor holds no grant on the unit and cannot query it"
    );
}

// The current holder relieves the lease to a billeted next holder at the boundary.
#[test]
fn e7c5_h_current_holder_relieves_to_a_billeted_next_holder() {
    let (mut tw, fx, _, _) = scene();
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let epoch = grant_epoch_of(&tw, fx.pilot, fx.atlas, "pilot");
    let request = action_ref(&tw, REQUEST_ACTION);
    // Pilot establishes the lease.
    let inv = request_invocation(
        &request, 1, fx.pilot, fx.atlas, "movement", fx.pilot, 0, "pilot", epoch, 100,
    );
    install_request(&mut tw, &request, &inv, 1);
    let held = lease_post_image(&apply_due(&mut tw, 100, 2));
    assert_eq!(held.holder(), Some(&PrincipalId::Object { id: fx.pilot }));

    // The current holder relieves to Gunner (a billeted crew member).
    let relief = request_invocation(
        &request,
        3,
        fx.pilot,
        fx.atlas,
        "movement",
        fx.gunner,
        held.revision(),
        "pilot",
        epoch,
        200,
    );
    install_request(&mut tw, &request, &relief, 3);
    let relieved = lease_post_image(&apply_due(&mut tw, 200, 4));
    assert_eq!(
        relieved.holder(),
        Some(&PrincipalId::Object { id: fx.gunner })
    );
    assert_eq!(relieved.epoch(), 2, "relief advances the lease epoch again");
}

// A non-holder (here a billet-holder who does not hold the lease) is not offered
// a relief: the authority split refuses it at projection.
#[test]
fn e7c5_h_a_non_holder_cannot_relieve() {
    let (mut tw, fx, _, _) = scene();
    // Pilot establishes and holds the lease.
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let epoch = grant_epoch_of(&tw, fx.pilot, fx.atlas, "pilot");
    let request = action_ref(&tw, REQUEST_ACTION);
    let inv = request_invocation(
        &request, 1, fx.pilot, fx.atlas, "movement", fx.pilot, 0, "pilot", epoch, 100,
    );
    install_request(&mut tw, &request, &inv, 1);
    apply_due(&mut tw, 100, 2);
    // Reassign the seat to Gunner and let Gunner focus it: Gunner now holds the
    // billet but NOT the lease (still Pilot's). Gunner is offered no handoff.
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/unassign #{}=pilot", fx.atlas.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", fx.atlas.0, fx.gunner.0),
    );
    ok(&mut tw, fx.gunner, &format!("+focus #{}/pilot", fx.atlas.0));
    arm_offer_keys(&mut tw);
    let subject = PerceivedRef::KnownObject {
        id: fx.atlas,
        revision: grant_epoch_of(&tw, fx.gunner, fx.atlas, "pilot"),
    };
    let query = tw
        .engine()
        .legal_action_query(PrincipalId::Object { id: fx.gunner }, fx.gunner, subject)
        .expect("the billet-holder can query the unit");
    let offers = tw.engine().project_legal_actions(&query).unwrap();
    assert!(
        offers.projected_for_action(&request).is_none(),
        "a non-holder is offered no handoff on a lease held by another"
    );
}

// The epoch binding (candidate (iii)): a focus change between request and apply
// bumps the requester's control generation, so the in-flight apply is STALE.
#[test]
fn e7c5_h_focus_change_between_request_and_apply_invalidates_it() {
    let (mut tw, fx, _, _) = scene();
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let epoch = grant_epoch_of(&tw, fx.pilot, fx.atlas, "pilot");
    let request = action_ref(&tw, REQUEST_ACTION);
    let inv = request_invocation(
        &request, 1, fx.pilot, fx.atlas, "movement", fx.pilot, 0, "pilot", epoch, 500,
    );
    install_request(&mut tw, &request, &inv, 1);
    // Pilot changes focus (here, releases it) — B4 bumps the unit's control
    // generation and republishes the grant, so the pinned witness epoch is stale.
    ok(&mut tw, fx.pilot, "+focus/release");
    assert!(
        grant_epoch(&tw, fx.pilot, fx.atlas, "pilot") != Some(epoch),
        "the focus change bumped the requester's grant epoch"
    );
    apply_is_stale(&mut tw, 500);
}

// Apply advances the lease epoch, so the OLD holder's captured control-lease
// authority no longer matches and the new holder's does — the witness at apply.
#[test]
fn e7c5_h_apply_bumps_the_lease_epoch_so_the_old_holders_intent_fails() {
    let (mut tw, fx, _, _) = scene();
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    let epoch = grant_epoch_of(&tw, fx.pilot, fx.atlas, "pilot");
    let request = action_ref(&tw, REQUEST_ACTION);
    let inv = request_invocation(
        &request, 1, fx.pilot, fx.atlas, "movement", fx.pilot, 0, "pilot", epoch, 100,
    );
    install_request(&mut tw, &request, &inv, 1);
    let held = lease_post_image(&apply_due(&mut tw, 100, 2));
    let owner = OwnerId::new(NS).unwrap();
    let old_guard = held
        .authority_guard(owner.clone())
        .expect("the held lease has a guard");
    assert!(held.matches_guard(&owner, &old_guard));

    let relief = request_invocation(
        &request,
        3,
        fx.pilot,
        fx.atlas,
        "movement",
        fx.gunner,
        held.revision(),
        "pilot",
        epoch,
        200,
    );
    install_request(&mut tw, &request, &relief, 3);
    let relieved = lease_post_image(&apply_due(&mut tw, 200, 4));
    assert!(
        !relieved.matches_guard(&owner, &old_guard),
        "the old holder's captured authority no longer matches after apply"
    );
    let new_guard = relieved
        .authority_guard(owner.clone())
        .expect("the new holder has a guard");
    assert!(
        relieved.matches_guard(&owner, &new_guard),
        "the new holder's authority is current"
    );
}

// `@station/handoff` is offerable (candidate (iii): the offer is never dropped by
// the kernel's ControlLease witness loop) for BOTH the vacant-create case and the
// held-relief case.
#[test]
fn e7c5_h_handoff_is_offerable_for_vacant_create_and_held_relief() {
    let (mut tw, fx, _, _) = scene();
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    arm_offer_keys(&mut tw);
    let request = action_ref(&tw, REQUEST_ACTION);
    let subject = |tw: &TestWorld| PerceivedRef::KnownObject {
        id: fx.atlas,
        revision: grant_epoch_of(tw, fx.pilot, fx.atlas, "pilot"),
    };
    // Vacant: the create offer projects for the focused billet-holder.
    let query = tw
        .engine()
        .legal_action_query(PrincipalId::Object { id: fx.pilot }, fx.pilot, subject(&tw))
        .unwrap();
    let offers = tw.engine().project_legal_actions(&query).unwrap();
    assert!(
        offers.projected_for_action(&request).is_some(),
        "the vacant-create handoff projects (required_channels = [])"
    );

    // Establish the lease, then declare a fallback: the held lease now projects a
    // relief to the declared controller.
    let epoch = grant_epoch_of(&tw, fx.pilot, fx.atlas, "pilot");
    let inv = request_invocation(
        &request, 1, fx.pilot, fx.atlas, "movement", fx.pilot, 0, "pilot", epoch, 100,
    );
    install_request(&mut tw, &request, &inv, 1);
    apply_due(&mut tw, 100, 2);
    ok(
        &mut tw,
        GOD,
        &format!("@station/fallback #{}/pilot=#{}", fx.atlas.0, fx.gunner.0),
    );
    let query = tw
        .engine()
        .legal_action_query(PrincipalId::Object { id: fx.pilot }, fx.pilot, subject(&tw))
        .unwrap();
    let offers = tw.engine().project_legal_actions(&query).unwrap();
    assert!(
        offers.projected_for_action(&request).is_some(),
        "the held-relief handoff to the declared fallback also projects"
    );
}

// B6: on a clean switch the released station's declared fallback fires
// immediately — a direct authoritative submit with NO PendingDecision (the K7.0
// immediate-controller seam).
#[test]
fn e7c5_h_fallback_controller_submits_with_no_pending_decision() {
    let (mut tw, fx, _, _) = scene();
    ok(
        &mut tw,
        GOD,
        &format!("@station/fallback #{}/pilot=#{}", fx.atlas.0, fx.gunner.0),
    );
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    assert!(!tw.engine().has_authoritative_submit(), "no submit yet");
    // A clean release fires the fallback controller.
    ok(&mut tw, fx.pilot, "+focus/release");
    assert!(
        tw.engine().has_authoritative_submit_for(fx.pilot),
        "the released station's fallback submitted through the authoritative seam"
    );
    // No PendingDecision is captured (fire_station_fallback never captures a
    // decision) — the immediate deterministic controller under K7.0's rule.
}

// K7 pin: cut 5 registers no decision source and no LLM path, so the fallback's
// immediate controller submits under a DETERMINISTIC self-derived idempotency key
// with no PendingDecision. The competing-LLM-decision contract — "a fallback
// competing with an existing LLM decision reuses that decision's record and
// idempotency key" — is inherently K7 territory (there is no decision to reuse).
// This pins the current key so K7's change of it is a deliberate, visible edit.
//
// (The plan's "#[ignore] K7 pin" form (§4.0c C) is realized as a PASSING pin
// instead: the landed `ci_gated_test_inventory` census requires every `#[ignore]`
// test to be owned by a CI job that runs it, which a genuinely unrunnable K7 pin
// cannot be — so it asserts the true cut-5 state rather than skipping. Deviation
// recorded in §3.5's as-built.)
#[test]
fn e7c5_h_fallback_key_is_deterministic_not_an_llm_decision_k7() {
    let (mut tw, fx, _, _) = scene();
    ok(
        &mut tw,
        GOD,
        &format!("@station/fallback #{}/pilot=#{}", fx.atlas.0, fx.gunner.0),
    );
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    ok(&mut tw, fx.pilot, "+focus/release");
    let submit = tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("the clean switch fired the fallback controller");
    assert_eq!(submit.actor, fx.pilot);
    // A self-derived deterministic key over (unit, station, holder) — NOT an LLM
    // decision's captured action-idempotency. With a K7 decision source, the
    // controller must reuse the pending decision's key here instead.
    assert_eq!(
        submit.key.as_str(),
        format!("station.fallback.{}.pilot.{}", fx.atlas.0, fx.gunner.0)
    );
}

// Existence hiding: an unknown unit and a station that does not exist on a unit
// are refused with the SAME line, so probing reveals nothing about which units or
// stations exist. (`resolve_visible_unit` also routes an unperceivable unit to
// this same line; the sim test rig registers no concealment hook to exercise it,
// so the byte-identical station-existence path stands in.)
#[test]
fn e7c5_h_station_existence_hiding_unknown_vs_unviewable() {
    let (mut tw, fx, _, _) = scene();
    let unknown_unit = err(&mut tw, fx.pilot, "@station/handoff #999999/pilot");
    let unknown_station = err(
        &mut tw,
        fx.pilot,
        &format!("@station/handoff #{}/ghostseat", fx.atlas.0),
    );
    assert_eq!(
        unknown_unit, unknown_station,
        "an unknown unit and an unknown station refuse byte-identically"
    );
    assert!(unknown_unit.contains("No such station."), "{unknown_unit}");
    let _ = fx.other;
}

// ---- E7 cut 5 independent verify fold ----
//
// The clean-release / switch fallback that never seated (C1/C2), the
// station-existence leak (B1), the boot-hydration lapse (D2/R2), the fallback
// census (B2), the bounded `+stations` scan (B3), and reach's acquire-time-only
// stance (C3/D3). Named `vf_<lens><n>_…`.

/// Resolve one queued command-originated authoritative submit through the REAL
/// offer path (as `mush-server`'s `command_offer_submission` +
/// `resolve_and_stage_offer` do) and stage it, returning the transaction ops so
/// the lease post-image can be read. This is what the shipped H tests never did:
/// it drives the fallback's DEFERRED submit to a staged request, proving the
/// relief offer both projects and resolves AFTER the release tore down the focus.
fn drive_fire_submit(
    tw: &mut TestWorld,
    submit: &mush_core::AuthoritativeSubmit,
    token: u8,
) -> Vec<ModuleOp> {
    let (action, resolved) = {
        let engine = tw.engine();
        let query = engine
            .legal_action_query(
                PrincipalId::Object { id: submit.actor },
                submit.actor,
                submit.subject.clone(),
            )
            .expect("the fire's subject still queries under the fresh grant epoch");
        let offers = engine.project_legal_actions(&query).expect("projection");
        let projected = offers
            .projected_for_action(&submit.action)
            .expect("the released station's relief offer projects after focus is gone");
        assert_eq!(projected.subject(), &submit.subject);
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
            .expect("the fire resolves against the projected relief offer");
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
        .expect("the resolved fire stages");
    let (ops, installation) = prepared.into_parts();
    tw.engine_mut().install_prepared_action(installation);
    ops
}

// C1 (HIGH): a clean `+focus/release` fires the released station's declared
// fallback AND it actually SEATS — the deferred submit is no longer self-staled
// by the same release. Drives resolution to completion and asserts the lease
// holder becomes the fallback.
#[test]
fn vf_c1_a_clean_release_fallback_actually_seats() {
    let (mut tw, fx, _, _) = scene();
    let backup = tw.spawn_player("Backup");
    tw.world_mut().move_to(backup, fx.field).unwrap();
    ok(
        &mut tw,
        GOD,
        &format!("@station/fallback #{}/pilot=#{}", fx.atlas.0, backup.0),
    );
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    arm_offer_keys(&mut tw);
    ok(&mut tw, fx.pilot, "+focus/release");
    let submit = *tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("the clean release fired the fallback controller");
    // Failure (a) fixed: the fire pins the POST-bump epoch, matching the pilot's
    // still-projecting grant, so it is not refused as stale at resolution.
    let PerceivedRef::KnownObject { revision, .. } = submit.subject else {
        panic!("the fire names an exact object subject")
    };
    assert_eq!(
        revision,
        grant_epoch_of(&tw, fx.pilot, fx.atlas, "pilot"),
        "the fire pins the new grant epoch, not the pre-release one"
    );
    let boundary_at = tw.world().world_ic_now().at;
    // Failure (b) fixed: the relief offer re-sources from the surviving billet
    // grant, so it still projects after the focus is gone; drive it and apply.
    let pending = lease_post_image(&drive_fire_submit(&mut tw, &submit, 7));
    assert!(pending.pending().is_some(), "the relief is pending");
    let applied = lease_post_image(&apply_due(&mut tw, boundary_at, 8));
    assert_eq!(
        applied.holder(),
        Some(&PrincipalId::Object { id: backup }),
        "the declared fallback actually seats"
    );
    assert!(applied.pending().is_none());
}

/// A scheduler/system-principal control-handoff request (D1 unclean-loss): NO
/// station-grant witness, so the apply is authorized by the declared record
/// alone and never gated on a departed pilot's grant epoch.
#[allow(clippy::too_many_arguments)]
fn system_request_invocation(
    action: &DefinitionRef,
    req: u8,
    unit: ObjId,
    channel: &str,
    next: ObjId,
    expected_revision: u64,
    boundary_at: i64,
) -> ActionInvocation {
    let principal = PrincipalId::Scoped {
        owner: OwnerId::new(NS).unwrap(),
        class: CanonicalId::new("scheduler").unwrap(),
        id: IdempotencyKeyText::parse_canonical("station.fallback.system").unwrap(),
    };
    ActionInvocation::new(ActionInvocationDraft {
        request: RequestUid::from_bytes([req; 16]),
        principal: principal.clone(),
        actor: unit,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([9; 32])),
        subject: PerceivedRef::KnownObject {
            id: unit,
            revision: 0,
        },
        targets: Vec::new(),
        parameters: json!({
            "channel": channel,
            "next_holder": PrincipalId::Object { id: next },
            "requested_at": utc(boundary_at - 1),
            "boundary": utc(boundary_at),
            "reason": "focus.station_seat",
            "expected_revision": expected_revision,
            // No station / grant_epoch: witness None -> apply is not grant-gated.
        }),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: action.key().owner.clone(),
                namespace: CanonicalId::new(action.key().id.as_str()).unwrap(),
            },
            1,
            IdempotencyKey::Request(RequestUid::from_bytes([req; 16])),
        ),
    })
    .unwrap()
}

// C1 (unclean loss): a linkdead pilot cannot issue the release, so the
// system/scheduler principal fires the relief, authorized by the declared
// `sim.station_fallback` record with no station witness. Drives to completion
// and asserts the seat fills with the declared fallback.
#[test]
fn vf_c1_b_unclean_loss_system_principal_seats() {
    let (mut tw, fx, _, _) = scene();
    let backup = tw.spawn_player("Backup");
    tw.world_mut().move_to(backup, fx.field).unwrap();
    ok(
        &mut tw,
        GOD,
        &format!("@station/fallback #{}/pilot=#{}", fx.atlas.0, backup.0),
    );
    // The pilot never focuses (gone); the system fires the relief on its behalf.
    let request = action_ref(&tw, REQUEST_ACTION);
    let inv = system_request_invocation(&request, 1, fx.atlas, "movement", backup, 0, 100);
    let pending = lease_post_image(&install_request(&mut tw, &request, &inv, 1));
    assert!(pending.pending().is_some(), "the system relief is pending");
    let applied = lease_post_image(&apply_due(&mut tw, 100, 2));
    assert_eq!(
        applied.holder(),
        Some(&PrincipalId::Object { id: backup }),
        "the declared fallback seats under the system principal, no witness needed"
    );
    assert_eq!(applied.epoch(), 1);
}

// C2 (MEDIUM): a focus SWITCH (`+focus <other>`) also fires the released
// station's declared fallback — the same mechanism, not only `+focus/release`.
#[test]
fn vf_c2_a_switch_fires_the_released_stations_fallback() {
    let (mut tw, fx, _, _) = scene();
    let backup = tw.spawn_player("Backup");
    tw.world_mut().move_to(backup, fx.field).unwrap();
    // Pilot is billeted on BOTH atlas/pilot and locust/pilot; a fallback on each.
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", fx.locust.0, fx.pilot.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@station/fallback #{}/pilot=#{}", fx.atlas.0, backup.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@station/fallback #{}/pilot=#{}", fx.locust.0, backup.0),
    );
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    assert!(
        !tw.engine().has_authoritative_submit(),
        "an acquire fires no fallback"
    );
    arm_offer_keys(&mut tw);
    // SWITCH atlas -> locust: this releases atlas/pilot and must fire its fallback.
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.locust.0));
    let submit = *tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("the switch fired the released atlas/pilot fallback");
    let PerceivedRef::KnownObject { id, .. } = submit.subject else {
        panic!()
    };
    assert_eq!(id, fx.atlas, "the fired relief is for the RELEASED unit");
    let boundary_at = tw.world().world_ic_now().at;
    let _ = drive_fire_submit(&mut tw, &submit, 7);
    let applied = lease_post_image(&apply_due(&mut tw, boundary_at, 8));
    assert_eq!(
        applied.holder(),
        Some(&PrincipalId::Object { id: backup }),
        "the switched-away station seats its fallback"
    );
}

// B1 (MEDIUM): `@station/handoff`/`/cancel` reveals nothing about station
// existence to a co-located NON-holder — an existing station (not held) and a
// non-existent one refuse byte-identically, so the unit's station complement
// cannot be enumerated.
#[test]
fn vf_b1_station_existence_is_hidden_from_a_nonholder() {
    let (mut tw, fx, _, _) = scene();
    // A co-located bystander with no billet on Atlas.
    let bystander = tw.spawn_player("Bystander");
    tw.world_mut().move_to(bystander, fx.field).unwrap();
    let existing = err(
        &mut tw,
        bystander,
        &format!("@station/handoff #{}/pilot", fx.atlas.0),
    );
    let missing = err(
        &mut tw,
        bystander,
        &format!("@station/handoff #{}/ghostseat", fx.atlas.0),
    );
    assert_eq!(
        existing, missing,
        "an existing-but-not-held station and a missing one refuse byte-identically"
    );
    assert!(existing.contains("No such station."), "{existing}");
    // A crew member holding a DIFFERENT seat on the same unit is equally blind to
    // a seat they do not hold.
    let existing_g = err(
        &mut tw,
        fx.gunner,
        &format!("@station/handoff #{}/pilot", fx.atlas.0),
    );
    let missing_g = err(
        &mut tw,
        fx.gunner,
        &format!("@station/handoff #{}/ghostseat", fx.atlas.0),
    );
    assert_eq!(existing_g, missing_g);
    assert!(existing_g.contains("No such station."));
    // The holder still gets the discoverable "+focus it first" hint (they already
    // know the seat exists — it is their billet).
    let held = err(
        &mut tw,
        fx.pilot,
        &format!("@station/handoff #{}/pilot", fx.atlas.0),
    );
    assert!(held.contains("+focus it first"), "{held}");
}

// B2 (LOW, folded into D2): the declared-fallback family is census-bounded. A
// pathological station name is refused (it cannot exist on the unit), and the
// count is capped by MAX_STATION_FALLBACKS.
#[test]
fn vf_b2_station_fallbacks_are_bounded() {
    let (mut tw, fx, _, _) = scene();
    // A pathological station name is refused: it is not a station on the unit.
    let long = "z".repeat(4096);
    let refused = err(
        &mut tw,
        GOD,
        &format!(
            "@station/fallback #{}/{}=#{}",
            fx.atlas.0, long, fx.gunner.0
        ),
    );
    assert!(refused.contains("No such station."), "{refused}");
    // The RAM census refuses a NEW key past the cap but never a re-declare.
    let mut slot = mush_sim::StationFallbacks::default();
    let station = mush_sim::StationId::new("pilot").unwrap();
    for n in 0..mush_sim::MAX_STATION_FALLBACKS {
        assert!(slot.declare(ObjId(n as u32 + 1), station.clone(), fx.gunner));
    }
    assert!(
        !slot.declare(ObjId(u32::MAX), station.clone(), fx.gunner),
        "a NEW key past the cap is refused"
    );
    assert!(
        slot.declare(ObjId(1), station, fx.pilot),
        "re-declaring an existing key still succeeds at the cap"
    );
    assert_eq!(slot.len(), mush_sim::MAX_STATION_FALLBACKS);
}

// B3 (LOW): `+stations` iterates the actor's OWN seats, so a unit crowded with
// many other actors' seats does not inflate the scan — the actor still sees
// their single billet regardless of where it sorts among the crowd.
#[test]
fn vf_b3_stations_scan_is_bounded() {
    let (mut tw, fx, _, _) = scene();
    // Crowd Locust with many other actors' seats, then give Pilot a billet whose
    // station name sorts AFTER all of them.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/station #{}=zpilot,pilot,channels:movement",
            fx.locust.0
        ),
    );
    for n in 0..80u32 {
        let crew = tw.spawn_player(&format!("Crew{n}"));
        let sid = format!("seat{n:03}");
        ok(
            &mut tw,
            GOD,
            &format!(
                "@spatial/station #{}={sid},gunner,channels:fire",
                fx.locust.0
            ),
        );
        ok(
            &mut tw,
            GOD,
            &format!("@spatial/assign #{}={sid},#{}", fx.locust.0, crew.0),
        );
    }
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=zpilot,#{}", fx.locust.0, fx.pilot.0),
    );
    let out = ok(&mut tw, fx.pilot, "+stations");
    // Pilot sees their Atlas pilot billet and the Locust zpilot billet, and none
    // of the 80 other-actor seats — the scan is bounded by Pilot's own billets.
    assert!(out.contains("`pilot`"), "{out}");
    assert!(out.contains("`zpilot`"), "{out}");
    assert!(
        !out.contains("seat0"),
        "no other actor's seat is listed: {out}"
    );
}

// C3 / D3(b): reach is an ACQUIRE-TIME gate only in cut 5. Focusing at a console
// and then leaving the room does NOT lose foreground control; continuous reach
// is tracked to cut 7 (`RemoteLink`/`EnvelopeId`). This pins the documented
// stance so a cut-7 change is deliberate.
#[test]
fn vf_c3_reach_is_acquire_time_only_in_cut5() {
    let (mut tw, fx, _, _) = scene();
    ok(&mut tw, fx.pilot, &format!("+focus #{}/pilot", fx.atlas.0));
    // Walk off the console into another room.
    tw.world_mut().move_to(fx.pilot, fx.other).unwrap();
    // Foreground control is retained (reach is not re-evaluated); `@station/show`
    // and a handoff still proceed for the still-focused billet-holder.
    let shown = ok(&mut tw, fx.pilot, "@station/show");
    assert!(shown.contains("driving"), "{shown}");
    assert_eq!(
        focused_station_of(&tw, fx.pilot),
        Some(fx.atlas),
        "focus survives leaving the console (acquire-time reach only)"
    );
}

fn focused_station_of(tw: &TestWorld, actor: ObjId) -> Option<ObjId> {
    focus(tw).focus_of_actor(actor).map(|f| f.unit)
}

// D2 / R2: the declared fallback survives a reboot — `load_boot` hydrates the
// `StationFallbacks` mirror from the persisted `station_fallback:` docs, so a
// clean switch fires a fallback after a restart without staff re-declaring.
#[test]
fn vf_d2_station_fallbacks_hydrate_at_boot() {
    let (mut tw, fx, _, _) = scene();
    ok(
        &mut tw,
        GOD,
        &format!("@station/fallback #{}/pilot=#{}", fx.atlas.0, fx.gunner.0),
    );
    // Fold the store writes into a KV map and reboot into a fresh world.
    let mut kv: Vec<(SmolStr, Value)> = Vec::new();
    for op in tw.take_module_ops() {
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    assert!(
        kv.iter().any(|(k, _)| k.starts_with("station_fallback:")),
        "the declaration persisted a durable doc"
    );
    let mut tw2 = TestWorld::with_modules(vec![Box::new(SimulationModule::new())]);
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    )]));
    let restored = tw2
        .world()
        .module_state::<mush_sim::StationFallbacks>()
        .expect("station-fallback slot")
        .holder(fx.atlas, &mush_sim::StationId::new("pilot").unwrap());
    assert_eq!(
        restored,
        Some(fx.gunner),
        "the declared fallback is live in the RAM mirror after a reboot"
    );
}
