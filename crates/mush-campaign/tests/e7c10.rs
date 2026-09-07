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

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use mush_campaign::{CampaignStore, GrantState, OperationId, SeatOfferId};
use mush_core::{
    AccountId, AccountIdentity, Audience, BehaviorBinding, BehaviorId, ChannelId, GOD, LIMBO,
    ModuleBootData, ModuleWrite, ObjId, ObjKind, SeatAudit,
};
use mush_org::CapabilityId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

const NS: &str = "sim";

// ---- rig ------------------------------------------------------------------

/// A fixed in-memory account oracle: character → account id.
#[derive(Debug, Default)]
struct MapOracle(std::sync::Mutex<BTreeMap<ObjId, AccountId>>);

impl AccountIdentity for MapOracle {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.0.lock().ok()?.get(&character).copied()
    }
}

/// The server's own `SeatAuditLog`, re-implemented here so this crate's tests
/// need no `mush-server` edge. Identical contract: process-local, boolean
/// answers, never a world document.
#[derive(Debug, Default)]
struct TestAudit(std::sync::Mutex<BTreeMap<String, AccountId>>);

impl SeatAudit for TestAudit {
    fn record(&self, token: &str, account: AccountId) -> bool {
        match self.0.lock() {
            Ok(mut rows) => {
                rows.insert(token.to_owned(), account);
                true
            }
            Err(_) => false,
        }
    }
    fn knows(&self, token: &str) -> bool {
        self.0
            .lock()
            .map(|r| r.contains_key(token))
            .unwrap_or(false)
    }
    fn account_for(&self, token: &str) -> Option<AccountId> {
        self.0.lock().ok()?.get(token).copied()
    }
    fn forget(&self, token: &str) -> bool {
        self.0
            .lock()
            .map(|mut r| r.remove(token).is_some())
            .unwrap_or(false)
    }
}

/// An audit that refuses every write — the unwired/failed-store arm.
#[derive(Debug, Default)]
struct DeadAudit;

impl SeatAudit for DeadAudit {
    fn record(&self, _token: &str, _account: AccountId) -> bool {
        false
    }
    fn knows(&self, _token: &str) -> bool {
        false
    }
    fn account_for(&self, _token: &str) -> Option<AccountId> {
        None
    }
    fn forget(&self, _token: &str) -> bool {
        false
    }
}

fn vocabulary() -> BTreeSet<CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(mush_campaign::CAPABILITIES.iter().copied())
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| CapabilityId::new(c).expect("well-formed"))
        .collect()
}

fn boot_with(audit: Arc<dyn SeatAudit>) -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(mush_canon::CanonModule::new()),
        Box::new(mush_sim::SimulationModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
        Box::new(mush_campaign::CampaignModule::new().with_seat_audit(audit)),
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
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    texts(&ev)
}

fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(
        has_error(&ev),
        "`{line}` should have been refused, got: {}",
        texts(&ev)
    );
    texts(&ev)
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

fn camp(tw: &TestWorld) -> &CampaignStore {
    tw.world()
        .module_state::<CampaignStore>()
        .expect("campaign store")
}

/// The channels a resolver would actually see on `actor`'s grant.
fn granted(tw: &TestWorld, actor: ObjId, unit: ObjId, station: &str) -> Vec<String> {
    tw.world()
        .command_subjects_of(actor)
        .iter()
        .find(|g| g.subject == unit && g.station.as_str() == station)
        .map(|g| g.channels.iter().map(|c| c.as_str().to_owned()).collect())
        .unwrap_or_default()
}

fn epoch_of(tw: &TestWorld, actor: ObjId, unit: ObjId, station: &str) -> Option<u64> {
    tw.world()
        .command_subjects_of(actor)
        .iter()
        .find(|g| g.subject == unit && g.station.as_str() == station)
        .map(|g| g.epoch)
}

/// The kernel's own stale-intent predicate, per channel.
fn is_current(
    tw: &TestWorld,
    actor: ObjId,
    unit: ObjId,
    station: &str,
    epoch: u64,
    channel: &str,
) -> bool {
    tw.world().channel_grant_is_current(
        actor,
        unit,
        NS,
        station,
        epoch,
        &ChannelId::new(channel).unwrap(),
    )
}

struct Fx {
    field: ObjId,
    atlas: ObjId,
    locust: ObjId,
    /// The AI NPC that normally stands the Atlas pilot watch (also the seat's
    /// declared fallback).
    ace: ObjId,
    /// The AI NPC on the Locust.
    duce: ObjId,
    commander: ObjId,
    driver: ObjId,
    bystander: ObjId,
    oracle: Arc<MapOracle>,
}

/// A domain, two placed units with two-channel `pilot` stations, an AI NPC
/// billeted on each, a commander who owns them, a would-be proxy driver and a
/// bystander with no authority at all.
fn scene(audit: Arc<dyn SeatAudit>) -> (TestWorld, Fx) {
    let mut tw = boot_with(audit);
    let commander = tw.spawn_player("Commander");
    let driver = tw.spawn_player("Driver");
    let bystander = tw.spawn_player("Bystander");
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .unwrap();
    let atlas = tw
        .world_mut()
        .create(ObjKind::Thing, "Atlas", commander, Some(LIMBO))
        .unwrap();
    let locust = tw
        .world_mut()
        .create(ObjKind::Thing, "Locust", commander, Some(LIMBO))
        .unwrap();
    let ace = tw
        .world_mut()
        .create(ObjKind::Npc, "Ace", commander, Some(LIMBO))
        .unwrap();
    let duce = tw
        .world_mut()
        .create(ObjKind::Npc, "Duce", commander, Some(LIMBO))
        .unwrap();
    bind(&mut tw, field, "sim.domain");
    bind(&mut tw, atlas, "sim.participant");
    bind(&mut tw, locust, "sim.participant");
    let oracle = Arc::new(MapOracle::default());
    if let Ok(mut map) = oracle.0.lock() {
        map.insert(commander, AccountId::mint("CommanderAcct", 1).unwrap());
        map.insert(driver, AccountId::mint("DriverAcct", 2).unwrap());
        map.insert(bystander, AccountId::mint("BystanderAcct", 3).unwrap());
    }
    *tw.world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .unwrap() = mush_sim::AccountOracle::new(oracle.clone());
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", field.0),
    );
    for unit in [atlas, locust] {
        ok(
            &mut tw,
            GOD,
            &format!("@spatial/place/domain #{}=0", unit.0),
        );
        ok(
            &mut tw,
            GOD,
            &format!(
                "@spatial/station #{}=pilot,pilot,channels:movement+fire",
                unit.0
            ),
        );
    }
    for (unit, npc) in [(atlas, ace), (locust, duce)] {
        ok(
            &mut tw,
            GOD,
            &format!("@spatial/assign #{}=pilot,#{}", unit.0, npc.0),
        );
        ok(
            &mut tw,
            GOD,
            &format!("@station/fallback #{}/pilot=#{}", unit.0, npc.0),
        );
        tw.world_mut().move_to(npc, field).unwrap();
    }
    for who in [commander, driver, bystander] {
        tw.world_mut().move_to(who, field).unwrap();
    }
    (
        tw,
        Fx {
            field,
            atlas,
            locust,
            ace,
            duce,
            commander,
            driver,
            bystander,
            oracle,
        },
    )
}

/// Declare, make public, and commit both units — the operation every seat hangs
/// off. Returns its id.
fn operation(tw: &mut TestWorld, fx: &Fx) -> OperationId {
    let before: BTreeSet<u64> = camp(tw).operations().map(|d| d.id().0).collect();
    ok(tw, fx.commander, "@op/declare hold the refinery");
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
    ok(
        tw,
        fx.commander,
        &format!("@op/join {}=#{}", id.0, fx.atlas.0),
    );
    ok(
        tw,
        fx.commander,
        &format!("@op/join {}=#{}", id.0, fx.locust.0),
    );
    id
}

/// Publish one seat on `unit`/`pilot` driven by `npc`, granting `channels`.
fn offer(
    tw: &mut TestWorld,
    fx: &Fx,
    op: OperationId,
    unit: ObjId,
    npc: ObjId,
    channels: &str,
) -> SeatOfferId {
    let before: BTreeSet<u64> = camp(tw).seat_offers().map(|o| o.id.0).collect();
    ok(
        tw,
        fx.commander,
        &format!(
            "+seat/offer {}/#{}/pilot=#{},{channels}",
            op.0, unit.0, npc.0
        ),
    );
    camp(tw)
        .seat_offers()
        .map(|o| o.id)
        .find(|id| !before.contains(&id.0))
        .expect("one new seat offer")
}

fn grant_state(tw: &TestWorld, op: OperationId, id: SeatOfferId) -> Option<GrantState> {
    camp(tw).seat_grant(op, id).map(|g| g.state.clone())
}

/// Drain the engine's one-authoritative-submit-per-actor slot, exactly as the
/// world task's `CommandSubmitDriver` would between commands. Without it a
/// second seat act by the same NPC gets back-pressure ("your earlier action is
/// still committing"), which is correct behaviour and not what these tests are
/// about.
fn drain(tw: &mut TestWorld) -> Vec<mush_core::AuthoritativeSubmit> {
    let mut staged = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        staged.push(*submit);
    }
    staged
}

/// Drive the real disconnect ingress: the session layer's `SessionGone` after
/// `mush-session`'s linkdead grace, which is what fires `HookPoint::Disconnected`.
fn disconnect(tw: &mut TestWorld, actor: ObjId, conn: u64) {
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    // The transport drops (the actor goes LINK-DEAD and keeps the seat)...
    tw.engine_mut().step(
        mush_core::Inbound::ConnDown {
            conn,
            actor,
            reason: mush_core::DisconnectKind::Lost,
        },
        &mut out,
        &mut fx,
    );
    // ...and only when `linkdead_grace` expires does the session layer give up,
    // which is the message that fires `HookPoint::Disconnected`.
    tw.engine_mut()
        .step(mush_core::Inbound::SessionGone { actor }, &mut out, &mut fx);
}

fn fold_kv(tw: &mut TestWorld, kv: &mut BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>) {
    for op in tw.take_module_ops() {
        let entry = kv.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                entry.retain(|(k, _)| *k != key);
                entry.push((key, value));
            }
            ModuleWrite::Del { key } => entry.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
}

fn reboot(tw: &mut TestWorld, kv: &BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>) {
    let boot: HashMap<SmolStr, ModuleBootData> = kv
        .iter()
        .map(|(ns, entries)| {
            (
                ns.clone(),
                ModuleBootData {
                    kv: entries.clone(),
                    streams: HashMap::new(),
                    ..Default::default()
                },
            )
        })
        .collect();
    tw.modules_loaded(boot);
}

/// One campaign tick fire (Coarse, so the clock must move a whole period).
fn fire(tw: &mut TestWorld) {
    let period = i64::try_from(mush_campaign::TICK_PERIOD_SECS).unwrap_or(15);
    let t = tw.world().now() + period + 1;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

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
        ok(&mut tw, GOD, &format!("@seat/audit {}", "deadbeef")),
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
    // The staff verb answers a boolean and prints no identity.
    let shown = ok(&mut tw, GOD, &format!("@seat/audit {ta}"));
    assert!(shown.contains("attributed"), "{shown}");
    assert!(!shown.contains(&ta) || shown.contains("attributed"));
    // And it is staff-only.
    err(&mut tw, fx.driver, &format!("@seat/audit {ta}"));
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
    let id = format!("{}/pilot/{}", fx.atlas.0, fx.ace.0);
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
    tw.set_now(tw.world().now() + 600);
    drain(&mut tw);
    ok(&mut tw, fx.driver, "+seat/leave");
    drain(&mut tw);
    let state = provider
        .resolve(tw.world(), "sim.station.served", &id, None)
        .expect("completed service answers");
    assert!(state.satisfied);
    assert_eq!(state.actor, Some(fx.ace));
    // An unknown kind, and an id for service that never happened, both answer
    // `None` — unknown is unsatisfied, everywhere.
    assert_eq!(
        provider.resolve(tw.world(), "sim.station.flown", &id, None),
        None
    );
    assert_eq!(
        provider.resolve(
            tw.world(),
            "sim.station.served",
            &format!("{}/pilot/{}", fx.locust.0, fx.duce.0),
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
            "+seat/offer {}/#{}/pilot=#{},movement,area:#{}",
            op.0, fx.atlas.0, fx.ace.0, elsewhere.0
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
    assert!(refused.contains("not authorized where"), "{refused}");
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
    assert!(!staged.is_empty(), "activation staged the seat's request");
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
