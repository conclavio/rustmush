//! Pass 2 Tier 2C defensive security regression tests for `mush-sim`:
//! - `P2-MOD-61`: Visibility & existence hiding in `+focus` and `@station`
//! - `P2-MOD-62`: Continuous `ReachProof::Console` revalidation across station, handoff, proxy, and lifecycle
//! - `P2-MOD-63`: Control-generation advance & `ChannelGrant` refresh on `window::*`, `custody::*`, `egress`, `@spatial/egress|detach`
//! - `P2-MOD-64`: Retired `DomainRecord` pruning & pulse maintenance starvation prevention
//! - `P2-MOD-65`: Multi-billet & cross-alt focus release fallback lease revocation
//! - `P2-MOD-82`: `window::close_window` releases `ActivationBook` slots & drains queue
//! - `P2-MOD-83`: `interior_contents`/`interior_holder` symmetry & complete `is_sim_bound`/`claims` coverage
//! - `P2-MOD-84`: Reject actors / non-carriers / self-controlled departures in `@journey/depart` & `journey_ineligible`

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::module::{HookPoint, HookReadContext, Perceive};
use mush_core::{
    AccountId, AccountIdentity, BehaviorBinding, BehaviorId, CanonicalId, ChannelId, CommitToken,
    ControlLeaseState, Ctx, Deferred, DefinitionId, DefinitionKey, DefinitionKind, DefinitionRef,
    Effects, EventHook, Flags, GOD, GameModule, IdempotencyKey, IdempotencyScope, LIMBO, ModError,
    ModuleActionState, ModuleOp, ModuleWrite, ObjId, ObjKind, OfferExecutionContext, OfferKeyRing,
    OfferSecret, OfferSubmission, Outbox, OwnerId, ParsedArgs, PerceivedRef, PrincipalId,
    Registrar, RequestUid, ScheduleDriveCause, ScopedIdempotencyKey, SwitchSet, WorldEvent,
    scheduled_action_ingress,
};
use mush_economy::{CustodyProvider, CustodyRef, EconomyModule};
use mush_org::OrgModule;
use mush_proto::OutputEvent;
use mush_sim::data::{
    HeldPlacement, Pose, SpaceAnchor, WindowBox, WindowOpener, WindowRec, WindowSurface,
    passive_rules_value,
};
use mush_sim::domain::{ActivationBook, MAX_ACTIVE_DOMAINS};
use mush_sim::route::{LegDef, RouteCatalog, RouteDef};
use mush_sim::tick::{
    MAX_DOMAINS_PER_PULSE_FIRE, MaintenanceInputs, PulseConfig, plan_maintenance,
};
use mush_sim::{
    DeployedMark, FocusState, InteriorCustodyProvider, JourneyBook, OpenWindow, SeatRequest,
    SimulationModule, SpatialState, StationId, Transition, request_seat_activation,
};
use mush_testkit::{authoritative::AcknowledgedWorld as TestWorld, text_of};
use serde_json::{Value, json};
use smol_str::SmolStr;

#[path = "support/space_command.rs"]
mod space_command;

const NS: &str = "sim";
const REQUEST_ACTION: &str = "control_handoff_request";
const RELEASE_ACTION: &str = "control_handoff_release";
const APPLY_ACTION: &str = "control_handoff_apply";

struct MapOracle(Arc<Mutex<BTreeMap<ObjId, AccountId>>>);
impl AccountIdentity for MapOracle {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.0.lock().ok()?.get(&character).copied()
    }
}

struct ConcealHook(Arc<Mutex<BTreeSet<ObjId>>>);

impl EventHook for ConcealHook {
    fn perceives(&self, _ctx: &HookReadContext<'_>, _observer: ObjId, subject: ObjId) -> Perceive {
        if self.0.lock().unwrap().contains(&subject) {
            Perceive::Conceal
        } else {
            Perceive::Abstain
        }
    }
}

struct ConcealModule(Arc<Mutex<BTreeSet<ObjId>>>);

impl GameModule for ConcealModule {
    fn name(&self) -> &'static str {
        "test_conceal"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.hook(
            HookPoint::Perceive,
            Arc::new(ConcealHook(Arc::clone(&self.0))),
        );
        Ok(())
    }
}

struct StaticCatalog(Arc<Mutex<Option<RouteDef>>>);
impl RouteCatalog for StaticCatalog {
    fn resolve(&self, _w: &mush_core::World, id: &CanonicalId) -> Option<RouteDef> {
        self.0
            .lock()
            .unwrap()
            .as_ref()
            .filter(|r| r.id() == id)
            .cloned()
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
    let ev = space_command::run(tw, who, line);
    let t = texts(&ev);
    assert!(!has_error(&ev), "{line:?} refused: {t}");
    t
}

fn err(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let ev = space_command::run(tw, who, line);
    assert!(
        has_error(&ev),
        "{line:?} should have been refused: {}",
        texts(&ev)
    );
    texts(&ev)
}

fn bind(tw: &mut TestWorld, obj: ObjId, beh: &str) {
    tw.world_mut()
        .attach_behavior(
            obj,
            BehaviorId::new(beh).unwrap(),
            BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
}

fn arm_offer_keys(tw: &mut TestWorld) {
    tw.engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(2, [2; 32]).unwrap(), []).unwrap());
}

fn action_ref(tw: &TestWorld, id: &str) -> DefinitionRef {
    tw.registry()
        .definitions()
        .expect("frozen definitions")
        .current(&DefinitionKey {
            owner: OwnerId::new(NS).unwrap(),
            kind: DefinitionKind::new("action").unwrap(),
            id: DefinitionId::new(id).unwrap(),
        })
        .expect("registered action")
        .clone()
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
        .expect("transaction carries a control-lease post-image");
    serde_json::from_value(value).expect("valid control-lease post-image")
}

fn drive_submit(
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
            .expect("legal action query");
        let offers = engine
            .project_legal_actions(&query)
            .expect("project legal actions");
        let projected = offers
            .projected_for_action(&submit.action)
            .expect("matching offer projected");
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
            .expect("resolve offer");
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
        .expect("prepare authoritative");
    let (ops, installation) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(installation)
        .expect("install prepared");
    ops
}

fn apply_due(tw: &mut TestWorld, at: i64, token: u8) -> Vec<ModuleOp> {
    tw.world_mut().set_now(at);
    let page = tw
        .engine_mut()
        .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
        .unwrap();
    assert!(
        !page.candidates().is_empty(),
        "expected due handoff candidate"
    );
    let candidate = page.candidates()[0].clone();
    assert_eq!(candidate.action(), &action_ref(tw, APPLY_ACTION));
    let ingress = scheduled_action_ingress(candidate.action(), candidate.advance(), 1).unwrap();
    let plan = tw
        .engine_mut()
        .plan_scheduled_action(&candidate, &ingress)
        .expect("plan scheduled apply");
    let (_, scheduled_inv, _) = plan.into_parts();
    let (prepared, _) = tw
        .engine_mut()
        .prepare_registered_scheduled_authoritative_action(
            &candidate,
            &scheduled_inv,
            CommitToken::from_bytes([token; 16]),
        )
        .expect("prepare scheduled apply");
    let (ops, installation) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(installation)
        .expect("install apply");
    ops
}

fn apply_cancels_stale(tw: &mut TestWorld, at: i64, token: u8) -> ControlLeaseState {
    tw.world_mut().set_now(at);
    let page = tw
        .engine_mut()
        .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
        .unwrap();
    assert_eq!(
        page.candidates().len(),
        1,
        "stale pending handoff should surface once for cleanup"
    );
    let candidate = page.candidates()[0].clone();
    let ingress = scheduled_action_ingress(candidate.action(), candidate.advance(), 1).unwrap();
    let plan = tw
        .engine_mut()
        .plan_scheduled_action(&candidate, &ingress)
        .expect("stale handoff must plan stale-cancel");
    let (_, scheduled_inv, _) = plan.into_parts();
    assert_eq!(
        scheduled_inv.parameters().get("cancel_stale"),
        Some(&Value::Bool(true)),
        "stale handoff must request cancel_stale"
    );
    let (prepared, output) = tw
        .engine_mut()
        .prepare_registered_scheduled_authoritative_action(
            &candidate,
            &scheduled_inv,
            CommitToken::from_bytes([token; 16]),
        )
        .expect("stale-cancel prepares");
    assert_eq!(
        output.outcome().data().get("operation"),
        Some(&json!("cancelled")),
        "stale-cancel emits cancelled outcome"
    );
    let (ops, installation) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(installation)
        .expect("stale-cancel installs");
    lease_post_image(&ops)
}

fn sample_window_rec() -> WindowRec {
    WindowRec {
        region: "region:test".into(),
        surfaces: vec![WindowSurface::Ground],
        bbox: WindowBox {
            x0: 0,
            y0: 0,
            x1: 1_000,
            y1: 1_000,
        },
        cell_cm: 100,
        composited_from: Vec::new(),
        features_seen: 0,
        opened_by: WindowOpener::Staff { by: GOD },
        revision: 1,
    }
}

fn sample_pose() -> Pose {
    Pose::at(100, 100, 0, passive_rules_value(json!({})).unwrap())
}

fn grant_epoch(tw: &TestWorld, actor: ObjId, unit: ObjId, station: &str) -> Option<u64> {
    tw.world()
        .command_subjects_of(actor)
        .iter()
        .find(|g| g.subject == unit && g.station == station)
        .map(|g| g.epoch)
}

fn with_ctx<R>(
    tw: &mut TestWorld,
    actor: ObjId,
    f: impl FnOnce(&mut Ctx<'_>) -> R,
) -> (R, Vec<ModuleOp>) {
    let mut out = Outbox::new();
    let engine = tw.engine_mut();
    let result = {
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(SmolStr::new(NS)),
            typed_state: ModuleActionState::unavailable(),
        };
        f(&mut ctx)
    };
    let mut ops = Vec::new();
    for deferred in out.take_deferred() {
        if let Deferred::Persist(op) = deferred {
            ops.push(op);
        }
    }
    (result, ops)
}

// =========================================================================
// P2-MOD-61: +focus and @station visibility & existence hiding
// =========================================================================

#[test]
fn p2_mod_61_focus_and_station_hide_global_dark_deployed_and_concealed_targets() {
    let concealed_slot: Arc<Mutex<BTreeSet<ObjId>>> = Arc::default();
    let accounts: Arc<Mutex<BTreeMap<ObjId, AccountId>>> = Arc::default();
    let mut tw = TestWorld::with_modules(vec![
        Box::new(SimulationModule::new().with_accounts(Arc::new(MapOracle(Arc::clone(&accounts))))),
        Box::new(ConcealModule(Arc::clone(&concealed_slot))),
    ]);
    let room_a = tw
        .world_mut()
        .create(ObjKind::Room, "RoomA", GOD, None)
        .unwrap();
    let room_b = tw
        .world_mut()
        .create(ObjKind::Room, "SecretBunkerRoom", GOD, None)
        .unwrap();
    let dark_room = tw
        .world_mut()
        .create(ObjKind::Room, "PitchBlackRoom", GOD, None)
        .unwrap();
    tw.world_mut()
        .set_flag(dark_room, Flags::DARK, true)
        .unwrap();

    let attacker = tw.spawn_player("Attacker");
    let secret_player = tw.spawn_player("SecretAdminAlt");
    let remote_hull = tw
        .world_mut()
        .create(ObjKind::Thing, "ClassifiedFlagship", GOD, Some(room_b))
        .unwrap();
    let dark_hull = tw
        .world_mut()
        .create(ObjKind::Thing, "ShadowCraft", GOD, Some(room_a))
        .unwrap();
    tw.world_mut()
        .set_flag(dark_hull, Flags::DARK, true)
        .unwrap();
    let deployed_hull = tw
        .world_mut()
        .create(ObjKind::Thing, "ArenaVanguard", GOD, Some(room_a))
        .unwrap();
    let concealed_target = tw
        .world_mut()
        .create(ObjKind::Thing, "TransitPhantom", GOD, Some(room_a))
        .unwrap();
    let dark_room_hull = tw
        .world_mut()
        .create(ObjKind::Thing, "HiddenInDarkRoom", GOD, Some(dark_room))
        .unwrap();
    // Same-name pair in room_a: one DARK, one visible -> must not leak via Ambiguous.
    let dark_dup = tw
        .world_mut()
        .create(ObjKind::Thing, "Skiff", GOD, Some(room_a))
        .unwrap();
    tw.world_mut()
        .set_flag(dark_dup, Flags::DARK, true)
        .unwrap();
    let vis_dup = tw
        .world_mut()
        .create(ObjKind::Thing, "Skiff", GOD, Some(room_a))
        .unwrap();

    concealed_slot.lock().unwrap().insert(concealed_target);

    // Mark `deployed_hull` as arena-deployed.
    bind(&mut tw, room_a, "sim.domain");
    bind(&mut tw, deployed_hull, "sim.participant");
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", room_a.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/domain #{}=0", deployed_hull.0),
    );
    mush_sim::deployed::mark(
        tw.world_mut(),
        deployed_hull,
        DeployedMark::new(77, "arena.proving"),
    );

    // Map attacker's account so focus reaches unit resolution.
    let acct = AccountId::mint("AttackerAcct", 1).unwrap();
    accounts.lock().unwrap().insert(attacker, acct);

    tw.world_mut().move_to(attacker, room_a).unwrap();
    tw.world_mut().move_to(secret_player, room_b).unwrap();

    let missing_focus = err(&mut tw, attacker, "+focus #999999/pilot");
    let missing_station = err(&mut tw, attacker, "@station/handoff #999999/pilot");
    let missing_station_empty = err(&mut tw, attacker, "@station/handoff #999999/");
    let missing_focus_empty = err(&mut tw, attacker, "+focus #999999/");

    for (label, target_ref, forbidden_name) in [
        (
            "remote #dbref",
            format!("#{}", remote_hull.0),
            "ClassifiedFlagship",
        ),
        (
            "remote room #dbref",
            format!("#{}", room_b.0),
            "SecretBunkerRoom",
        ),
        (
            "global *player",
            "*SecretAdminAlt".to_string(),
            "SecretAdminAlt",
        ),
        (
            "local DARK #dbref",
            format!("#{}", dark_hull.0),
            "ShadowCraft",
        ),
        ("local DARK name", "ShadowCraft".to_string(), "ShadowCraft"),
        (
            "deployed #dbref",
            format!("#{}", deployed_hull.0),
            "ArenaVanguard",
        ),
        (
            "deployed name",
            "ArenaVanguard".to_string(),
            "ArenaVanguard",
        ),
        (
            "transit-concealed #dbref",
            format!("#{}", concealed_target.0),
            "TransitPhantom",
        ),
        (
            "transit-concealed name",
            "TransitPhantom".to_string(),
            "TransitPhantom",
        ),
    ] {
        let f_out = err(&mut tw, attacker, &format!("+focus {target_ref}/pilot"));
        assert_eq!(f_out, missing_focus, "+focus oracle leaked for {label}");
        assert!(
            !f_out.contains(forbidden_name),
            "+focus disclosed name for {label}: {f_out}"
        );
        let s_out = err(
            &mut tw,
            attacker,
            &format!("@station/handoff {target_ref}/pilot"),
        );
        assert_eq!(s_out, missing_station, "@station oracle leaked for {label}");
        let s_empty = err(
            &mut tw,
            attacker,
            &format!("@station/handoff {target_ref}/"),
        );
        assert_eq!(
            s_empty, missing_station_empty,
            "@station empty-station syntax oracle leaked for {label}"
        );
        let f_empty = err(&mut tw, attacker, &format!("+focus {target_ref}/"));
        assert_eq!(
            f_empty, missing_focus_empty,
            "+focus empty-station syntax oracle leaked for {label}"
        );
    }

    // DARK duplicate must not cause Ambiguous when matching the visible Skiff.
    let skiff_out = err(&mut tw, attacker, "+focus Skiff/pilot");
    assert!(
        skiff_out.contains("no billet") && !skiff_out.contains("which one"),
        "visible Skiff must resolve without DARK ambiguity leak: {skiff_out}"
    );
    let _ = vis_dup;

    // Inside a DARK room the attacker does not control, uncarried room things are hidden.
    tw.world_mut().move_to(attacker, dark_room).unwrap();
    let dark_room_focus = err(
        &mut tw,
        attacker,
        &format!("+focus #{}/pilot", dark_room_hull.0),
    );
    assert_eq!(dark_room_focus, missing_focus);
    assert!(!dark_room_focus.contains("HiddenInDarkRoom"));

    // Positive control: a billeted pilot sitting at a deployed unit's live console
    // CAN reclaim `+focus` on that deployed unit, while stepping out of the console
    // hides it again.
    let deployed_pilot = tw.spawn_player("DeployedPilot");
    let deployed_pilot_acct = AccountId::mint("DeployedPilotAcct", 2).unwrap();
    accounts
        .lock()
        .unwrap()
        .insert(deployed_pilot, deployed_pilot_acct);
    let remote_cab = tw
        .world_mut()
        .create(ObjKind::Room, "RemoteCab", GOD, None)
        .unwrap();
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/station #{}=pilot,pilot,channels:movement,space:#{}",
            deployed_hull.0, remote_cab.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/assign #{}=pilot,#{}",
            deployed_hull.0, deployed_pilot.0
        ),
    );
    // Away from console: deployed unit remains hidden even from its billeted pilot.
    tw.world_mut().move_to(deployed_pilot, room_a).unwrap();
    let away_focus = err(
        &mut tw,
        deployed_pilot,
        &format!("+focus #{}/pilot", deployed_hull.0),
    );
    assert_eq!(away_focus, missing_focus);
    // At the live console: billeted pilot reclaims `+focus` cleanly.
    tw.world_mut().move_to(deployed_pilot, remote_cab).unwrap();
    let at_console_focus = ok(
        &mut tw,
        deployed_pilot,
        &format!("+focus #{}/pilot", deployed_hull.0),
    );
    assert!(
        at_console_focus.contains("pilot"),
        "billeted pilot at live console must reclaim +focus on deployed unit: {at_console_focus}"
    );
}

// =========================================================================
// P2-MOD-62: Continuous ReachProof::Console revalidation
// =========================================================================

#[test]
fn p2_mod_62_console_reach_revalidated_across_station_handoff_proxy_and_lifecycle() {
    let accounts: Arc<Mutex<BTreeMap<ObjId, AccountId>>> = Arc::default();
    let mut tw = TestWorld::with_modules(vec![Box::new(
        SimulationModule::new().with_accounts(Arc::new(MapOracle(Arc::clone(&accounts)))),
    )]);
    // Run `modules_loaded` so `ControlLeaseView` completes its initial scan and
    // accepts real `request_seat_activation` calls.
    tw.modules_loaded(HashMap::new());
    arm_offer_keys(&mut tw);
    tw.set_now(1_000);

    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .unwrap();
    let other = tw
        .world_mut()
        .create(ObjKind::Room, "Other", GOD, None)
        .unwrap();
    let cab = tw
        .world_mut()
        .create(ObjKind::Room, "Cab", GOD, None)
        .unwrap();
    let atlas = tw
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, Some(LIMBO))
        .unwrap();
    let pilot = tw.spawn_player("Pilot");
    let driver = tw.spawn_player("Driver");
    let backup = tw.spawn_player("Backup");
    let npc = tw
        .world_mut()
        .create(ObjKind::Npc, "AceNpc", GOD, Some(LIMBO))
        .unwrap();

    bind(&mut tw, field, "sim.domain");
    bind(&mut tw, cab, "sim.interior");
    bind(&mut tw, atlas, "sim.participant");
    bind(&mut tw, npc, "sim.participant");

    let acct_p = AccountId::mint("PilotAcct", 1).unwrap();
    let acct_d = AccountId::mint("DriverAcct", 2).unwrap();
    {
        let mut map = accounts.lock().unwrap();
        map.insert(pilot, acct_p);
        map.insert(driver, acct_d);
    }

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
        &format!("@spatial/space #{}=moving:#{}", cab.0, atlas.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/station #{}=pilot,pilot,channels:movement,space:#{}",
            atlas.0, cab.0
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
        &format!("@station/fallback #{}/pilot=#{}", atlas.0, backup.0),
    );
    tw.world_mut().move_to(pilot, cab).unwrap();
    tw.world_mut().move_to(driver, other).unwrap();

    // 1. Focus at console cab, then walk away via direct move_to -> LegalActions projects NO offer.
    ok(&mut tw, pilot, &format!("+focus #{}/pilot", atlas.0));
    tw.world_mut().move_to(pilot, other).unwrap();
    let epoch = grant_epoch(&tw, pilot, atlas, "pilot").unwrap();
    let query = tw
        .engine()
        .legal_action_query(
            PrincipalId::Object { id: pilot },
            pilot,
            PerceivedRef::KnownObject {
                id: atlas,
                revision: epoch,
            },
        )
        .unwrap();
    let projected = tw.engine().project_legal_actions(&query).unwrap();
    let req_act = action_ref(&tw, REQUEST_ACTION);
    let rel_act = action_ref(&tw, RELEASE_ACTION);
    assert!(
        projected.projected_for_action(&req_act).is_none()
            && projected.projected_for_action(&rel_act).is_none(),
        "stale console proof must not project request or release offers"
    );

    // `@station/handoff` refuses and invalidates the stale ordinary focus.
    let refused = err(
        &mut tw,
        pilot,
        &format!("@station/handoff #{}/pilot", atlas.0),
    );
    assert!(refused.contains("no longer at"), "{refused}");
    assert!(
        tw.world()
            .module_state::<FocusState>()
            .unwrap()
            .focus_of_actor(pilot)
            .is_none(),
        "stale ordinary focus must be relinquished"
    );
    while tw.engine_mut().take_authoritative_submit().is_some() {}

    // 2. Pending self-claim handoff staged at console, then actor leaves console before apply ->
    //    HandoffScheduleAdapter::plan cancels stale instead of transferring the lease!
    tw.world_mut().move_to(pilot, cab).unwrap();
    ok(&mut tw, pilot, &format!("+focus #{}/pilot", atlas.0));
    ok(
        &mut tw,
        pilot,
        &format!("@station/handoff #{}/pilot", atlas.0),
    );
    let submit = *tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("staged handoff");
    let pending = lease_post_image(&drive_submit(&mut tw, &submit, 1));
    assert!(pending.pending().is_some());
    // Pilot leaves console without changing focus (bypassing WorldEvent::Moved).
    tw.world_mut().move_to(pilot, other).unwrap();
    let now = tw.world().world_ic_now().at;
    let after_apply = apply_cancels_stale(&mut tw, now, 2);
    assert_eq!(
        after_apply.holder(),
        None,
        "pending self-claim must be cancelled when console reach breaks before apply"
    );
    assert!(after_apply.pending().is_none());

    // 3. WorldEvent::Moved invalidates ordinary focus immediately.
    tw.world_mut().move_to(pilot, cab).unwrap();
    ok(&mut tw, pilot, &format!("+focus #{}/pilot", atlas.0));
    let pre_move_epoch = grant_epoch(&tw, pilot, atlas, "pilot").unwrap();
    tw.world_mut().move_to(pilot, other).unwrap();
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().emit_world_event(
        &WorldEvent::Moved {
            who: pilot,
            from: Some(cab),
            to: other,
        },
        &mut out,
        &mut fx,
    );
    assert!(
        tw.world()
            .module_state::<FocusState>()
            .unwrap()
            .focus_of_actor(pilot)
            .is_none(),
        "WorldEvent::Moved away from console must relinquish ordinary focus"
    );
    assert!(
        grant_epoch(&tw, pilot, atlas, "pilot").unwrap() > pre_move_epoch,
        "relinquishing on move must advance control generation"
    );
    while tw.engine_mut().take_authoritative_submit().is_some() {}

    // 4. Quarantining the MovingObject host invalidates console reach even while
    //    the actor remains in the cab, whereas a live stationary `SpaceAnchor::Unanchored`
    //    interior (e.g. simulator pod) remains a valid console room until the actor leaves.
    tw.world_mut().move_to(pilot, cab).unwrap();
    ok(&mut tw, pilot, &format!("+focus #{}/pilot", atlas.0));
    with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::run(
            ctx,
            &Transition::Quarantine {
                root: atlas,
                reason: "console host quarantined".into(),
            },
        )
    })
    .0
    .unwrap();
    let show = ok(&mut tw, pilot, "@station/show");
    assert!(
        show.contains("driving no station"),
        "quarantined MovingObject console host must invalidate reach: {show}"
    );
    // Recover & re-place `atlas`; verify a stationary `Unanchored` pod console
    // still admits `+focus` cleanly, then re-anchor `cab` for proxy seat test.
    ok(&mut tw, GOD, &format!("@spatial/recover #{}", atlas.0));
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/domain #{}=0", atlas.0),
    );
    with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::run(
            ctx,
            &Transition::Space {
                space: cab,
                anchor: SpaceAnchor::Unanchored,
            },
        )
    })
    .0
    .unwrap();
    ok(&mut tw, pilot, &format!("+focus #{}/pilot", atlas.0));
    let unanchored_show = ok(&mut tw, pilot, "@station/show");
    assert!(
        unanchored_show.contains("pilot"),
        "live unanchored stationary console (e.g. simulator pod) must stay reachable: {unanchored_show}"
    );
    ok(&mut tw, pilot, "+focus/release");
    with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::run(
            ctx,
            &Transition::Space {
                space: cab,
                anchor: SpaceAnchor::MovingObject { host: atlas },
            },
        )
    })
    .0
    .unwrap();
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/unassign #{}=pilot", atlas.0),
    );

    // 5. Proxy focus via real `request_seat_activation`: moving NPC away from console
    //    immediately revokes `proxy_drive_of` and `driving_actor` while keeping T11
    //    `seat_restriction` active until stand-down.
    tw.world_mut().move_to(npc, cab).unwrap();
    let seat_req = SeatRequest::Activate {
        unit: atlas,
        station: StationId::new("pilot").unwrap(),
        npc,
        granted: BTreeSet::from([ChannelId::new("movement").unwrap()]),
        fallback: Some(backup),
    };
    let (ticket, _) = with_ctx(&mut tw, driver, |ctx| {
        request_seat_activation(ctx, &seat_req)
    });
    ticket.expect("proxy seat activates while NPC stands at console");
    assert_eq!(mush_sim::driving_actor(tw.world(), driver), npc);

    // NPC walks out of cab -> remote proxy drive immediately stops.
    tw.world_mut().move_to(npc, other).unwrap();
    assert!(
        mush_sim::proxy_drive_of(tw.world(), driver).is_none(),
        "proxy_drive_of must refuse when NPC leaves console"
    );
    assert_eq!(
        mush_sim::driving_actor(tw.world(), driver),
        driver,
        "driving_actor must not route to NPC when console reach is broken"
    );
    assert!(
        mush_sim::seat_restriction(tw.world(), driver, mush_sim::ProxyAct::FallbackChange).is_err(),
        "T11 guardrail stays active until driver stands down from the seat"
    );

    // 6. ObjDestroyed on console room tears down proxy focus and clears narrowing.
    tw.world_mut().move_to(npc, cab).unwrap();
    tw.world_mut().destroy(cab).unwrap();
    let mut out2 = Outbox::new();
    let mut fx2 = Effects::new();
    tw.engine_mut()
        .emit_world_event(&WorldEvent::ObjDestroyed { obj: cab }, &mut out2, &mut fx2);
    assert!(
        tw.world()
            .module_state::<FocusState>()
            .unwrap()
            .focus_of_account(acct_d)
            .is_none(),
        "destroying console room must relinquish bound proxy focus"
    );
    let cleared_narrowing = tw
        .world()
        .module_state::<SpatialState>()
        .unwrap()
        .assignments
        .get(&atlas)
        .and_then(|ha| ha.seats.get(&StationId::new("pilot").unwrap()))
        .and_then(|seat| seat.granted.as_ref());
    assert!(
        cleared_narrowing.is_none(),
        "destroying console room must clear seat narrowing on proxy focus teardown"
    );
}

// =========================================================================
// P2-MOD-63: Control-generation advance & ChannelGrant refresh across transitions
// =========================================================================

#[test]
fn p2_mod_63_control_gen_and_links_update_across_window_custody_egress_and_detach() {
    let mut tw = TestWorld::with_modules(vec![Box::new(SimulationModule::new())]);
    let room = tw
        .world_mut()
        .create(ObjKind::Room, "Yard", GOD, None)
        .unwrap();
    let bay = tw
        .world_mut()
        .create(ObjKind::Room, "Bay", GOD, None)
        .unwrap();
    let vault_room = tw
        .world_mut()
        .create(ObjKind::Room, "VaultBay", GOD, None)
        .unwrap();
    let rig = tw
        .world_mut()
        .create(ObjKind::Thing, "Rig", GOD, Some(room))
        .unwrap();
    let carrier = tw
        .world_mut()
        .create(ObjKind::Thing, "Carrier", GOD, Some(room))
        .unwrap();
    let turret = tw
        .world_mut()
        .create(ObjKind::Thing, "Turret", GOD, Some(room))
        .unwrap();
    let pilot = tw.spawn_player("Pilot");
    let gunner = tw.spawn_player("Gunner");

    bind(&mut tw, bay, "sim.interior");
    bind(&mut tw, vault_room, "sim.interior");
    bind(&mut tw, rig, "sim.participant");
    bind(&mut tw, carrier, "sim.participant");
    bind(&mut tw, turret, "sim.participant");

    // Open a window domain and declare stations/assignments on `rig` and `turret`.
    let (dom_res, _) = with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::open_window(
            ctx,
            OpenWindow {
                host: room,
                presentation: room,
                rec: sample_window_rec(),
                driver: None,
            },
        )
    });
    let dom = dom_res.unwrap();
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/space #{}=fixed:{}", bay.0, dom.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/space #{}=fixed:{}", vault_room.0, dom.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/station #{}=pilot,pilot,channels:movement", rig.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", rig.0, pilot.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/station #{}=gun,gunner,channels:fire", turret.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=gun,#{}", turret.0, gunner.0),
    );

    // 1. window::place advances control_generation AND publishes grant at the NEW epoch.
    let gen0 = tw
        .world()
        .module_state::<SpatialState>()
        .unwrap()
        .control_generation(rig);
    assert!(grant_epoch(&tw, pilot, rig, "pilot").is_none());
    with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::place(ctx, rig, dom, sample_pose())
    })
    .0
    .unwrap();
    let gen1 = tw
        .world()
        .module_state::<SpatialState>()
        .unwrap()
        .control_generation(rig);
    assert!(gen1 > gen0, "window::place must advance control_generation");
    assert_eq!(
        grant_epoch(&tw, pilot, rig, "pilot"),
        Some(gen1),
        "window::place must publish ChannelGrant at the bumped epoch"
    );

    // 2. window::place_interior advances control_generation AND republishes grant.
    with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::place_interior(ctx, rig, bay, None)
    })
    .0
    .unwrap();
    let gen2 = tw
        .world()
        .module_state::<SpatialState>()
        .unwrap()
        .control_generation(rig);
    assert!(
        gen2 > gen1,
        "window::place_interior must advance control_generation"
    );
    assert_eq!(grant_epoch(&tw, pilot, rig, "pilot"), Some(gen2));

    // 3. @spatial/egress advances control_generation AND republishes grant.
    ok(&mut tw, GOD, &format!("@spatial/egress #{}", rig.0));
    let gen3 = tw
        .world()
        .module_state::<SpatialState>()
        .unwrap()
        .control_generation(rig);
    assert!(
        gen3 > gen2,
        "@spatial/egress must advance control_generation"
    );
    assert_eq!(grant_epoch(&tw, pilot, rig, "pilot"), Some(gen3));

    // 4. @spatial/place/attached & @spatial/detach advance control_generation BEFORE refreshing links.
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/mount #{}=hardpoint,2,exposed", carrier.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/domain #{}={}", carrier.0, dom.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/place/attached #{}=#{},hardpoint",
            turret.0, carrier.0
        ),
    );
    let t_gen1 = tw
        .world()
        .module_state::<SpatialState>()
        .unwrap()
        .control_generation(turret);
    assert_eq!(
        grant_epoch(&tw, gunner, turret, "gun"),
        Some(t_gen1),
        "@spatial/place must bump BEFORE refreshing links"
    );
    ok(&mut tw, GOD, &format!("@spatial/detach #{}", turret.0));
    let t_gen2 = tw
        .world()
        .module_state::<SpatialState>()
        .unwrap()
        .control_generation(turret);
    assert!(
        t_gen2 > t_gen1,
        "@spatial/detach must advance control_generation"
    );
    assert_eq!(grant_epoch(&tw, gunner, turret, "gun"), Some(t_gen2));

    // 5. window::unplace advances control_generation AND retracts stale ChannelGrant.
    with_ctx(&mut tw, GOD, |ctx| mush_sim::unplace(ctx, rig, "stored"))
        .0
        .unwrap();
    let gen4 = tw
        .world()
        .module_state::<SpatialState>()
        .unwrap()
        .control_generation(rig);
    assert!(
        gen4 > gen3,
        "window::unplace must advance control_generation"
    );
    assert!(
        grant_epoch(&tw, pilot, rig, "pilot").is_none(),
        "window::unplace must revoke stale ChannelGrant"
    );

    // 6. InteriorCustodyProvider::move_into & move_out advance control_generation and sync links.
    let provider = InteriorCustodyProvider;
    let backing = CustodyRef::SpatialInterior(vault_room);
    with_ctx(&mut tw, GOD, |ctx| {
        provider.move_into(ctx, GOD, rig, &backing)
    })
    .0
    .unwrap();
    let gen5 = tw
        .world()
        .module_state::<SpatialState>()
        .unwrap()
        .control_generation(rig);
    assert!(
        gen5 > gen4,
        "custody::move_into must advance control_generation"
    );
    assert_eq!(grant_epoch(&tw, pilot, rig, "pilot"), Some(gen5));

    with_ctx(&mut tw, GOD, |ctx| {
        provider.move_out(ctx, GOD, rig, &backing, room)
    })
    .0
    .unwrap();
    let gen6 = tw
        .world()
        .module_state::<SpatialState>()
        .unwrap()
        .control_generation(rig);
    assert!(
        gen6 > gen5,
        "custody::move_out must advance control_generation"
    );
    assert!(
        grant_epoch(&tw, pilot, rig, "pilot").is_none(),
        "custody::move_out must revoke stale ChannelGrant"
    );

    // 7. window::close_window retracts ChannelGrants for released members.
    with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::place(ctx, rig, dom, sample_pose())
    })
    .0
    .unwrap();
    assert!(grant_epoch(&tw, pilot, rig, "pilot").is_some());
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/space #{}=unanchored", bay.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/space #{}=unanchored", vault_room.0),
    );
    with_ctx(&mut tw, GOD, |ctx| mush_sim::close_window(ctx, dom))
        .0
        .unwrap();
    assert!(
        grant_epoch(&tw, pilot, rig, "pilot").is_none(),
        "window::close_window must revoke ChannelGrant of released members"
    );
    assert!(
        grant_epoch(&tw, gunner, turret, "gun").is_none(),
        "window::close_window must revoke ChannelGrant of all released members"
    );
}

#[test]
fn p2_mod_63_unmanned_rig_omits_cgen_writes_while_live_witnesses_bump() {
    let accounts: Arc<Mutex<BTreeMap<ObjId, AccountId>>> = Arc::default();
    let oracle: Arc<dyn AccountIdentity> = Arc::new(MapOracle(Arc::clone(&accounts)));
    let mut tw = TestWorld::with_modules(vec![Box::new(
        SimulationModule::new().with_accounts(oracle),
    )]);
    tw.modules_loaded(HashMap::new());
    arm_offer_keys(&mut tw);
    tw.set_now(1_000);

    let room = tw
        .world_mut()
        .create(ObjKind::Room, "Yard", GOD, None)
        .unwrap();
    let bay = tw
        .world_mut()
        .create(ObjKind::Room, "Bay", GOD, None)
        .unwrap();
    let shed = tw
        .world_mut()
        .create(ObjKind::Room, "Shed", GOD, None)
        .unwrap();
    let rig = tw
        .world_mut()
        .create(ObjKind::Thing, "UnmannedRig", GOD, Some(room))
        .unwrap();
    let cab = tw
        .world_mut()
        .create(ObjKind::Room, "RigCab", GOD, None)
        .unwrap();
    let npc = tw
        .world_mut()
        .create(ObjKind::Npc, "DronePilot", GOD, Some(room))
        .unwrap();
    let pilot = tw.spawn_player("LeasePilot");
    tw.world_mut().move_to(pilot, cab).unwrap();
    accounts
        .lock()
        .unwrap()
        .insert(pilot, AccountId::mint("LeasePilotAcct", 1).unwrap());
    tw.connect(pilot);

    bind(&mut tw, bay, "sim.interior");
    bind(&mut tw, shed, "sim.interior");
    bind(&mut tw, cab, "sim.interior");
    bind(&mut tw, rig, "sim.participant");

    let (dom_res, _) = with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::open_window(
            ctx,
            OpenWindow {
                host: room,
                presentation: room,
                rec: sample_window_rec(),
                driver: None,
            },
        )
    });
    let dom = dom_res.unwrap();
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/space #{}=fixed:{}", bay.0, dom.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/space #{}=fixed:{}", shed.0, dom.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/space #{}=moving:#{}", cab.0, rig.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/station #{}=pilot,pilot,channels:helm,space:#{}",
            rig.0, cab.0
        ),
    );

    let has_cgen_write = |ops: &[ModuleOp], unit: ObjId| {
        let needle = format!("cgen:{}", unit.0);
        ops.iter()
            .any(|op| matches!(&op.op, ModuleWrite::JsonSet { key, .. } if key.as_str() == needle))
    };

    // 1. Unmanned, unleased, unfocused rig with declared station emits ZERO `cgen:` writes
    //    across `window::place`, `window::place_interior`, and `window::unplace`.
    let gen0 = tw
        .world()
        .module_state::<SpatialState>()
        .unwrap()
        .control_generation(rig);
    let (res, ops) = with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::place(ctx, rig, dom, sample_pose())
    });
    res.unwrap();
    assert!(
        !has_cgen_write(&ops, rig),
        "unmanned window::place wrote cgen"
    );
    let (res, ops) = with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::place_interior(ctx, rig, bay, None)
    });
    res.unwrap();
    assert!(
        !has_cgen_write(&ops, rig),
        "unmanned window::place_interior (bay) wrote cgen"
    );
    let (res, ops) = with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::place_interior(ctx, rig, shed, None)
    });
    res.unwrap();
    assert!(
        !has_cgen_write(&ops, rig),
        "unmanned window::place_interior (shed) wrote cgen"
    );
    let (res, ops) = with_ctx(&mut tw, GOD, |ctx| mush_sim::unplace(ctx, rig, "stored"));
    res.unwrap();
    assert!(
        !has_cgen_write(&ops, rig),
        "unmanned window::unplace wrote cgen"
    );
    assert_eq!(
        tw.world()
            .module_state::<SpatialState>()
            .unwrap()
            .control_generation(rig),
        gen0,
        "unmanned rig control_generation must stay unchanged"
    );

    // 2. A live NPC standing in the anchored cab (even with zero assignments,
    //    leases, or foci) MUST trigger `cgen:` bumps on placement transitions.
    tw.world_mut().move_to(npc, cab).unwrap();
    let (res, ops) = with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::place_interior(ctx, rig, bay, None)
    });
    res.unwrap();
    assert!(
        has_cgen_write(&ops, rig),
        "NPC inside anchored cab must persist cgen on window::place_interior"
    );
    let gen_npc = tw
        .world()
        .module_state::<SpatialState>()
        .unwrap()
        .control_generation(rig);
    assert!(gen_npc > gen0);
    tw.world_mut().move_to(npc, room).unwrap();

    // 3. A pending K6 control lease (with zero assignments, foci, or NPCs)
    //    mirrored in `ControlLeaseView` MUST still trigger `cgen:` bumps.
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", rig.0, pilot.0),
    );
    ok(&mut tw, pilot, &format!("+focus #{}/pilot", rig.0));
    while tw.engine_mut().take_authoritative_submit().is_some() {}
    ok(
        &mut tw,
        pilot,
        &format!("@station/handoff #{}/pilot", rig.0),
    );
    let submit = *tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("staged handoff");
    let pending = lease_post_image(&drive_submit(&mut tw, &submit, 71));
    assert!(pending.pending().is_some());
    // Clear focus & assignment while the pending K6 lease remains in `control_leases`,
    // then fire the coarse tick so `ControlLeaseView` refreshes its mirror.
    ok(&mut tw, pilot, "+focus/release");
    ok(&mut tw, GOD, &format!("@spatial/unassign #{}=pilot", rig.0));
    tw.set_now(1_005);
    let _ = tw.fire_due(1_005 * 1000);

    let gen_pending_before = tw
        .world()
        .module_state::<SpatialState>()
        .unwrap()
        .control_generation(rig);
    let (res, ops) = with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::place_interior(ctx, rig, shed, None)
    });
    res.unwrap();
    assert!(
        has_cgen_write(&ops, rig),
        "pending K6 lease must persist cgen on window::place_interior"
    );
    let gen_pending_after = tw
        .world()
        .module_state::<SpatialState>()
        .unwrap()
        .control_generation(rig);
    assert!(gen_pending_after > gen_pending_before);
}

// =========================================================================
// P2-MOD-64 & P2-MOD-82: Retired DomainRecord pruning, maintenance starvation,
// and ActivationBook slot release on window::close_window
// =========================================================================

#[test]
fn p2_mod_64_retired_windows_pruned_and_pulse_maintenance_never_starves() {
    let mut tw = TestWorld::with_modules(vec![Box::new(SimulationModule::new())]);
    tw.set_now(1_000);
    let room = tw
        .world_mut()
        .create(ObjKind::Room, "Plaza", GOD, None)
        .unwrap();

    // 1. Open and close 45 windows (> MAX_DOMAINS_PER_PULSE_FIRE = 32) on the same room.
    for _ in 0..(MAX_DOMAINS_PER_PULSE_FIRE + 13) {
        let dom = with_ctx(&mut tw, GOD, |ctx| {
            mush_sim::open_window(
                ctx,
                OpenWindow {
                    host: room,
                    presentation: room,
                    rec: sample_window_rec(),
                    driver: None,
                },
            )
        })
        .0
        .unwrap();
        with_ctx(&mut tw, GOD, |ctx| mush_sim::close_window(ctx, dom))
            .0
            .unwrap();
    }
    {
        let st = tw.world().module_state::<SpatialState>().unwrap();
        assert!(
            st.domains.is_empty(),
            "closed window DomainRecords must be pruned immediately (found {})",
            st.domains.len()
        );
        assert_eq!(
            st.next_domain_id,
            (MAX_DOMAINS_PER_PULSE_FIRE + 13) as u64,
            "domain IDs remain strictly monotonic"
        );
    }

    // 2. Open and retire 40 passive DomainRecords ahead of an Active promoted
    //    domain via real commands, then verify `plan_maintenance` never starves
    //    the active domain and `run_maintenance` prunes the retired tombstones.
    for i in 0..40_usize {
        let host = tw
            .world_mut()
            .create(ObjKind::Room, &format!("RetiredHost{i}"), GOD, None)
            .unwrap();
        let retired_id = with_ctx(&mut tw, GOD, |ctx| {
            mush_sim::open_passive_domain(ctx, host, host)
        })
        .0
        .unwrap();
        ok(&mut tw, GOD, &format!("@domain/retire {}", retired_id.0));
    }
    let active_room = tw
        .world_mut()
        .create(ObjKind::Room, "ActiveArena", GOD, None)
        .unwrap();
    let active_id = with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::open_passive_domain(ctx, active_room, active_room)
    })
    .0
    .unwrap();
    ok(&mut tw, GOD, &format!("@domain/promote {}", active_id.0));

    let now = tw.world().world_ic_now();
    {
        let st = tw.world().module_state::<SpatialState>().unwrap();
        let book = tw.world().module_state::<ActivationBook>().unwrap();
        let plan = plan_maintenance(&MaintenanceInputs {
            state: st,
            book,
            now: &now,
            config: PulseConfig::default(),
            space: &BTreeSet::new(),
            cursors: &BTreeMap::new(),
            only: None,
        });
        assert!(
            plan.applied.contains(&active_id),
            "active promoted domain behind 40 retired records must not starve: {plan:?}"
        );
        assert!(
            !plan.bound_hit,
            "retired passive records must not consume maintenance visit budget"
        );
    }

    let (plan, ops) = with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::tick::run_maintenance(ctx, None)
    });
    assert!(plan.applied.contains(&active_id));
    let st = tw.world().module_state::<SpatialState>().unwrap();
    assert_eq!(
        st.domains.len(),
        1,
        "run_maintenance must prune retired empty passive domains (remaining: {})",
        st.domains.len()
    );
    assert!(st.domains.contains_key(&active_id));
    let deleted_domains = ops
        .iter()
        .filter(|op| matches!(&op.op, ModuleWrite::Del { key } if key.starts_with("domain:")))
        .count();
    assert_eq!(deleted_domains, 40, "all 40 retired domain docs deleted");
}

#[test]
fn p2_mod_82_close_window_releases_activation_slots_and_drains_queue() {
    let mut tw = TestWorld::with_modules(vec![Box::new(SimulationModule::new())]);
    tw.set_now(2_000);

    let mut windows = Vec::new();
    for i in 0..MAX_ACTIVE_DOMAINS {
        let r = tw
            .world_mut()
            .create(ObjKind::Room, &format!("Win{i}"), GOD, None)
            .unwrap();
        let dom = with_ctx(&mut tw, GOD, |ctx| {
            mush_sim::open_window(
                ctx,
                OpenWindow {
                    host: r,
                    presentation: r,
                    rec: sample_window_rec(),
                    driver: None,
                },
            )
        })
        .0
        .unwrap();
        ok(&mut tw, GOD, &format!("@domain/promote {}", dom.0));
        windows.push(dom);
    }

    // 9th domain queues behind MAX_ACTIVE_DOMAINS.
    let extra_room = tw
        .world_mut()
        .create(ObjKind::Room, "QueuedDomain", GOD, None)
        .unwrap();
    let queued_dom = with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::open_passive_domain(ctx, extra_room, extra_room)
    })
    .0
    .unwrap();
    ok(&mut tw, GOD, &format!("@domain/promote {}", queued_dom.0));
    {
        let book = tw.world().module_state::<ActivationBook>().unwrap();
        assert_eq!(book.live(), MAX_ACTIVE_DOMAINS);
        assert!(!book.is_promoted(queued_dom));
        assert!(book.activations.get(&queued_dom).unwrap().is_queued());
    }

    // Close the first promoted window: its ActivationBook slot must be released
    // and the queued domain promoted immediately.
    let closed = windows[0];
    let (res, ops) = with_ctx(&mut tw, GOD, |ctx| mush_sim::close_window(ctx, closed));
    res.unwrap();
    {
        let book = tw.world().module_state::<ActivationBook>().unwrap();
        assert!(
            !book.activations.contains_key(&closed),
            "closed window must not retain an ActivationBook entry"
        );
        assert!(
            book.is_promoted(queued_dom),
            "freed MAX_ACTIVE_DOMAINS slot must promote queued domain"
        );
        assert_eq!(book.live(), MAX_ACTIVE_DOMAINS);
    }
    assert!(
        ops.iter().any(|op| {
            matches!(&op.op, ModuleWrite::Del { key } if key.as_str() == format!("activation:{}", closed.0))
        }),
        "activation doc must be deleted on close_window"
    );
}

// =========================================================================
// P2-MOD-65: Multi-billet & cross-alt focus release fallback lease revocation
// =========================================================================

#[test]
fn p2_mod_65_multi_billet_and_cross_alt_release_revokes_exact_station_lease() {
    let accounts: Arc<Mutex<BTreeMap<ObjId, AccountId>>> = Arc::default();
    let mut tw = TestWorld::with_modules(vec![Box::new(
        SimulationModule::new().with_accounts(Arc::new(MapOracle(Arc::clone(&accounts)))),
    )]);
    arm_offer_keys(&mut tw);
    tw.set_now(3_000);

    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .unwrap();
    let atlas = tw
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, Some(LIMBO))
        .unwrap();
    let locust = tw
        .world_mut()
        .create(ObjKind::Thing, "Locust", GOD, Some(LIMBO))
        .unwrap();
    let alt1 = tw.spawn_player("Alt1");
    let alt2 = tw.spawn_player("Alt2");
    let backup_alpha = tw.spawn_player("BackupAlpha");
    let backup_pilot = tw.spawn_player("BackupPilot");

    bind(&mut tw, field, "sim.domain");
    bind(&mut tw, atlas, "sim.participant");
    bind(&mut tw, locust, "sim.participant");

    // Alt1 and Alt2 share ONE account.
    let shared_acct = AccountId::mint("SharedAcct", 1).unwrap();
    {
        let mut map = accounts.lock().unwrap();
        map.insert(alt1, shared_acct);
        map.insert(alt2, shared_acct);
    }

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
    // `alpha` sorts alphabetically BEFORE `pilot` on Atlas!
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/station #{}=alpha,gunner,channels:fire", atlas.0),
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
        &format!(
            "@spatial/station #{}=pilot,pilot,channels:movement",
            locust.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=alpha,#{}", atlas.0, alt1.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", atlas.0, alt1.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", locust.0, alt2.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@station/fallback #{}/alpha=#{}", atlas.0, backup_alpha.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@station/fallback #{}/pilot=#{}", atlas.0, backup_pilot.0),
    );
    for who in [alt1, alt2, backup_alpha, backup_pilot] {
        tw.world_mut().move_to(who, field).unwrap();
    }

    // Case A: Multi-billet `+focus/release` on Atlas/pilot must relieve `movement`
    // to `backup_pilot` (NOT misdirect to `alpha` / `fire`!).
    ok(&mut tw, alt1, &format!("+focus #{}/pilot", atlas.0));
    ok(
        &mut tw,
        alt1,
        &format!("@station/handoff #{}/pilot", atlas.0),
    );
    let sub1 = *tw.engine_mut().take_authoritative_submit().unwrap();
    drive_submit(&mut tw, &sub1, 1);
    let held = lease_post_image(&apply_due(&mut tw, 3_000, 2));
    assert_eq!(held.key().channel().as_str(), "movement");
    assert_eq!(held.holder(), Some(&PrincipalId::Object { id: alt1 }));

    ok(&mut tw, alt1, "+focus/release");
    let sub_rel = *tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("release fires fallback");
    drive_submit(&mut tw, &sub_rel, 3);
    let relieved = lease_post_image(&apply_due(&mut tw, 3_000, 4));
    assert_eq!(
        relieved.key().channel().as_str(),
        "movement",
        "multi-billet release must relieve the exact released `pilot` (movement) channel"
    );
    assert_eq!(
        relieved.holder(),
        Some(&PrincipalId::Object { id: backup_pilot }),
        "pilot's movement lease must transfer to backup_pilot"
    );

    // Case B: Multi-billet SAME-UNIT switch (`Atlas/alpha` -> `Atlas/pilot`) must
    // still relieve `alpha` (`fire` -> `backup_alpha`), not misroute into `pilot`!
    ok(&mut tw, alt1, &format!("+focus #{}/alpha", atlas.0));
    while tw.engine_mut().take_authoritative_submit().is_some() {}
    ok(
        &mut tw,
        alt1,
        &format!("@station/handoff #{}/alpha", atlas.0),
    );
    let sub_fire = *tw.engine_mut().take_authoritative_submit().unwrap();
    drive_submit(&mut tw, &sub_fire, 5);
    let fire_held = lease_post_image(&apply_due(&mut tw, 3_000, 6));
    assert_eq!(fire_held.key().channel().as_str(), "fire");
    assert_eq!(fire_held.holder(), Some(&PrincipalId::Object { id: alt1 }));

    ok(&mut tw, alt1, &format!("+focus #{}/pilot", atlas.0));
    let sub_switch = *tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("same-unit switch fires fallback for alpha");
    drive_submit(&mut tw, &sub_switch, 7);
    let fire_relieved = lease_post_image(&apply_due(&mut tw, 3_000, 8));
    assert_eq!(
        fire_relieved.key().channel().as_str(),
        "fire",
        "same-unit switch must relieve the old `alpha` (fire) lease"
    );
    assert_eq!(
        fire_relieved.holder(),
        Some(&PrincipalId::Object { id: backup_alpha }),
        "old alpha lease must transfer to backup_alpha"
    );

    // Case C: Cross-alt `+focus/release` (Alt1 holds `sensors` on `gamma`,
    // Alt2 types `+focus/release` -> Alt1's lease is revoked!).
    let backup_gamma = tw.spawn_player("BackupGamma");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/station #{}=gamma,sensors,channels:sensors",
            atlas.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=gamma,#{}", atlas.0, alt1.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@station/fallback #{}/gamma=#{}", atlas.0, backup_gamma.0),
    );
    ok(&mut tw, alt1, &format!("+focus #{}/gamma", atlas.0));
    while tw.engine_mut().take_authoritative_submit().is_some() {}
    ok(
        &mut tw,
        alt1,
        &format!("@station/handoff #{}/gamma", atlas.0),
    );
    let sub_g = *tw.engine_mut().take_authoritative_submit().unwrap();
    drive_submit(&mut tw, &sub_g, 9);
    let g_held = lease_post_image(&apply_due(&mut tw, 3_000, 10));
    assert_eq!(g_held.holder(), Some(&PrincipalId::Object { id: alt1 }));

    ok(&mut tw, alt2, "+focus/release");
    let sub_alt_rel = *tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("cross-alt +focus/release must fire fallback as Alt1");
    assert_eq!(sub_alt_rel.actor, alt1);
    drive_submit(&mut tw, &sub_alt_rel, 11);
    let g_relieved = lease_post_image(&apply_due(&mut tw, 3_000, 12));
    assert_eq!(
        g_relieved.holder(),
        Some(&PrincipalId::Object { id: backup_gamma }),
        "cross-alt +focus/release must revoke Alt1's K6 lease"
    );

    // Case D: Cross-alt `+focus` switch (Alt1 holds `comms` on Atlas, Alt2 runs `+focus #Locust/pilot` ->
    // Alt1's `comms` lease is revoked!).
    let backup_comms = tw.spawn_player("BackupComms");
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/station #{}=comms,comms,channels:comms", atlas.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=comms,#{}", atlas.0, alt1.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@station/fallback #{}/comms=#{}", atlas.0, backup_comms.0),
    );
    ok(&mut tw, alt1, &format!("+focus #{}/comms", atlas.0));
    while tw.engine_mut().take_authoritative_submit().is_some() {}
    ok(
        &mut tw,
        alt1,
        &format!("@station/handoff #{}/comms", atlas.0),
    );
    let sub_c = *tw.engine_mut().take_authoritative_submit().unwrap();
    drive_submit(&mut tw, &sub_c, 13);
    let c_held = lease_post_image(&apply_due(&mut tw, 3_000, 14));
    assert_eq!(c_held.holder(), Some(&PrincipalId::Object { id: alt1 }));

    ok(&mut tw, alt2, &format!("+focus #{}/pilot", locust.0));
    let sub_alt_sw = *tw
        .engine_mut()
        .take_authoritative_submit()
        .expect("cross-alt +focus switch must fire fallback as Alt1");
    assert_eq!(sub_alt_sw.actor, alt1);
    drive_submit(&mut tw, &sub_alt_sw, 15);
    let c_relieved = lease_post_image(&apply_due(&mut tw, 3_000, 16));
    assert_eq!(
        c_relieved.holder(),
        Some(&PrincipalId::Object { id: backup_comms }),
        "cross-alt +focus switch must revoke Alt1's displaced K6 lease"
    );
}

// =========================================================================
// P2-MOD-83: interior_contents vs interior_holder symmetry & is_sim_bound coverage
// =========================================================================

#[test]
fn p2_mod_83_interior_custody_symmetry_and_is_sim_bound_coverage() {
    let mut tw = TestWorld::with_modules(vec![Box::new(SimulationModule::new())]);
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .unwrap();
    let pres = tw
        .world_mut()
        .create(ObjKind::Room, "Pres", GOD, None)
        .unwrap();
    let vault_outer = tw
        .world_mut()
        .create(ObjKind::Room, "OuterVault", GOD, None)
        .unwrap();
    let nested_crate_space = tw
        .world_mut()
        .create(ObjKind::Room, "CrateSpace", GOD, None)
        .unwrap();
    let member_room = tw
        .world_mut()
        .create(ObjKind::Room, "BridgeRoom", GOD, None)
        .unwrap();
    let multi_space = tw
        .world_mut()
        .create(ObjKind::Room, "ShipLock", GOD, None)
        .unwrap();
    let gem = tw
        .world_mut()
        .create(ObjKind::Thing, "Gem", GOD, Some(LIMBO))
        .unwrap();
    let parked_carrier = tw
        .world_mut()
        .create(ObjKind::Thing, "ParkedCarrier", GOD, Some(LIMBO))
        .unwrap();
    let riding_bay = tw
        .world_mut()
        .create(ObjKind::Room, "RidingBay", GOD, None)
        .unwrap();
    let held_assign_host = tw
        .world_mut()
        .create(ObjKind::Thing, "HeldAssignHost", GOD, Some(LIMBO))
        .unwrap();
    let deployed_unit = tw
        .world_mut()
        .create(ObjKind::Thing, "DeployedUnit", GOD, Some(LIMBO))
        .unwrap();
    let child_host = tw
        .world_mut()
        .create(ObjKind::Thing, "ChildHost", GOD, Some(LIMBO))
        .unwrap();

    bind(&mut tw, field, "sim.domain");
    bind(&mut tw, vault_outer, "sim.interior");
    bind(&mut tw, nested_crate_space, "sim.interior");
    bind(&mut tw, nested_crate_space, "sim.participant");
    bind(&mut tw, multi_space, "sim.interior");
    bind(&mut tw, riding_bay, "sim.interior");
    bind(&mut tw, gem, "sim.participant");
    bind(&mut tw, deployed_unit, "sim.participant");

    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive,#{}", field.0, pres.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/space #{}=fixed:0", vault_outer.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/space #{}=unanchored", nested_crate_space.0),
    );
    // Place `nested_crate_space` itself directly `Interior` inside `vault_outer`,
    // and place `gem` inside `nested_crate_space`.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/place/interior #{}=#{}",
            nested_crate_space.0, vault_outer.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/place/interior #{}=#{}",
            gem.0, nested_crate_space.0
        ),
    );

    let provider = InteriorCustodyProvider;
    let outer_ref = CustodyRef::SpatialInterior(vault_outer);
    let contents = provider.contents(tw.world(), &outer_ref).unwrap();
    assert!(
        contents.contains(&gem),
        "contents must include gem in self-placed nested space: {contents:?}"
    );
    assert_eq!(
        provider.holds(tw.world(), &outer_ref, gem),
        Some(true),
        "holds MUST be symmetric with contents for self-placed nested spaces (P2-MOD-83)"
    );
    for &item in &contents {
        assert_eq!(
            provider.holds(tw.world(), &outer_ref, item),
            Some(true),
            "every item in contents must satisfy holds == Some(true)"
        );
    }

    // Dead / unspaced target agrees between contents and holds.
    let non_space_ref = CustodyRef::SpatialInterior(pres);
    assert_eq!(provider.contents(tw.world(), &non_space_ref), Some(vec![]));
    assert_eq!(provider.holds(tw.world(), &non_space_ref, gem), Some(false));

    // Verify `SpatialState::is_sim_bound` & `provider.claims` cover all sim-bound states.
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/space #{}=fixed:0", multi_space.0),
    );
    with_ctx(&mut tw, GOD, |ctx| {
        mush_sim::run(
            ctx,
            &Transition::Members {
                space: multi_space,
                rooms: BTreeSet::from([multi_space, member_room]),
            },
        )
    })
    .0
    .unwrap();
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/space #{}=moving:#{}",
            riding_bay.0, parked_carrier.0
        ),
    );
    mush_sim::deployed::mark(
        tw.world_mut(),
        deployed_unit,
        DeployedMark::new(88, "arena.proving"),
    );

    let st = tw.world().module_state::<SpatialState>().unwrap();
    assert!(st.is_sim_bound(field), "domain host must be sim-bound");
    assert!(st.is_sim_bound(pres), "presentation room must be sim-bound");
    assert!(
        st.is_sim_bound(member_room),
        "multi-room member must be sim-bound"
    );
    assert!(
        st.is_sim_bound(parked_carrier),
        "unplaced host with anchored spaces must be sim-bound"
    );
    let mut synthetic = SpatialState::default();
    synthetic.held_assignments.insert(
        held_assign_host,
        HeldPlacement {
            key: "assignment:held".into(),
            reason: "corrupt".into(),
        },
    );
    synthetic
        .derived
        .children
        .entry(child_host)
        .or_default()
        .insert(gem);
    assert!(
        synthetic.is_sim_bound(held_assign_host),
        "held_assignment host must be sim-bound"
    );
    assert!(
        synthetic.is_sim_bound(child_host),
        "object with spatial children must be sim-bound"
    );
    assert!(
        provider.claims(tw.world(), deployed_unit),
        "InteriorCustodyProvider::claims must claim deployed units"
    );
    assert!(
        provider.claims(tw.world(), parked_carrier),
        "InteriorCustodyProvider::claims must claim unplaced carrier with anchored spaces"
    );
}

// =========================================================================
// P2-MOD-84: Non-carrier & actor rejection in @journey/depart & journey_ineligible
// =========================================================================

#[test]
fn p2_mod_84_journey_depart_and_plan_reject_actors_and_non_carriers() {
    let route_slot: Arc<Mutex<Option<RouteDef>>> = Arc::default();
    let sim = SimulationModule::new()
        .with_route_catalog(Arc::new(StaticCatalog(Arc::clone(&route_slot))));
    let mut tw = TestWorld::with_modules(vec![
        Box::new(OrgModule::new()),
        Box::new(sim),
        Box::new(EconomyModule::with_custody(vec![Arc::new(
            InteriorCustodyProvider,
        )])),
    ]);
    tw.set_now(4_000);

    let origin = tw
        .world_mut()
        .create(ObjKind::Room, "Origin", GOD, None)
        .unwrap();
    let dest = tw
        .world_mut()
        .create(ObjKind::Room, "Dest", GOD, None)
        .unwrap();
    let vault_bay = tw
        .world_mut()
        .create(ObjKind::Room, "VaultBay", GOD, None)
        .unwrap();
    let player = tw.spawn_player("Traveler");
    let npc = tw
        .world_mut()
        .create(ObjKind::Npc, "OwnedNpc", player, Some(LIMBO))
        .unwrap();
    let pebble = tw
        .world_mut()
        .create(ObjKind::Thing, "Pebble", player, Some(LIMBO))
        .unwrap();
    let ship = tw
        .world_mut()
        .create(ObjKind::Thing, "OwnedShip", player, Some(LIMBO))
        .unwrap();
    let vaulted_ship = tw
        .world_mut()
        .create(ObjKind::Thing, "VaultedShip", player, Some(LIMBO))
        .unwrap();

    *route_slot.lock().unwrap() = Some(
        RouteDef::new(
            CanonicalId::new("corridor").unwrap(),
            (origin, dest),
            vec![
                LegDef::new(origin, 100).unwrap(),
                LegDef::new(dest, 200).unwrap(),
            ],
            CanonicalId::new("fusion").unwrap(),
            0,
        )
        .unwrap(),
    );

    bind(&mut tw, origin, "sim.domain");
    bind(&mut tw, dest, "sim.domain");
    bind(&mut tw, vault_bay, "sim.interior");
    bind(&mut tw, player, "sim.participant");
    bind(&mut tw, npc, "sim.participant");
    bind(&mut tw, ship, "sim.participant");
    bind(&mut tw, vaulted_ship, "sim.participant");

    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", origin.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", dest.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/domain #{}=0", player.0),
    );
    ok(&mut tw, GOD, &format!("@spatial/place/domain #{}=0", npc.0));
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/domain #{}=0", ship.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/space #{}=fixed:0", vault_bay.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@vault/create bay=interior:#{}", vault_bay.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/place/interior #{}=#{}",
            vaulted_ship.0, vault_bay.0
        ),
    );

    // 1. Player cannot depart themselves (`me`), an owned NPC, or a non-participant Thing.
    for target in [
        "me".to_string(),
        format!("#{}", npc.0),
        format!("#{}", pebble.0),
    ] {
        let out = err(
            &mut tw,
            player,
            &format!("@journey/depart {target}=corridor"),
        );
        assert!(
            out.contains("Permission denied"),
            "non-carrier / actor departure must be denied for {target}: {out}"
        );
    }

    // 2. Staff `@journey/depart` and `@journey/plan` also reject actors, non-participants,
    //    domain hosts, spaces, and ships sitting inside a vault backing.
    for (label, target) in [
        ("player", player),
        ("npc", npc),
        ("non-participant thing", pebble),
        ("domain host", origin),
        ("interior space", vault_bay),
    ] {
        let dep = err(
            &mut tw,
            GOD,
            &format!("@journey/depart #{}=corridor", target.0),
        );
        assert!(
            dep.contains("cannot fly a route"),
            "staff @journey/depart must reject {label}: {dep}"
        );
        let plan = err(
            &mut tw,
            GOD,
            &format!("@journey/plan #{}=corridor", target.0),
        );
        assert!(
            plan.contains("cannot fly a route"),
            "staff @journey/plan must reject {label}: {plan}"
        );
    }

    let vaulted_dep = err(
        &mut tw,
        player,
        &format!("@journey/depart #{}=corridor", vaulted_ship.0),
    );
    assert!(
        vaulted_dep.contains("vault"),
        "ship inside a vault backing must not depart: {vaulted_dep}"
    );

    // 3. Positive control: valid owned Thing carrier with `sim.participant` still departs cleanly.
    ok(
        &mut tw,
        player,
        &format!("@journey/depart #{}=corridor", ship.0),
    );
    assert!(
        tw.world()
            .module_state::<JourneyBook>()
            .unwrap()
            .under_way(ship),
        "valid owned carrier must depart cleanly"
    );
}
