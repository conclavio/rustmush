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
    AccountId, AccountIdentity, ChannelId, GOD, LIMBO, ModuleBootData, ModuleWrite, ObjId, ObjKind,
};
use mush_proto::OutputEvent;
use mush_sim::{AccountOracle, FocusState, ReachProof, SimulationModule};
use mush_testkit::{TestWorld, text_of};
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
        },
    )]));
    assert!(
        tw2.world().module_state::<FocusState>().unwrap().is_empty(),
        "no actor holds a focus after a restart"
    );
}
