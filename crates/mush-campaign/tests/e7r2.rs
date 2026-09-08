//! E7 residue pass 2's campaign half: **P0b, the recovery cooldown's insured-loss
//! marker** (from E7 residue pass 1's fold, F4(i)(a)).
//!
//! The rig is `crates/mush-campaign/tests/e7r1.rs`'s, narrowed to the cooldown:
//! `boot_with` composes org, faction, canon, sim, economy and campaign in production
//! order and everything runs through the real `@recovery/*` command path.
//!
//! **What P0b is for.** Q7A says the cooldown is *"waived when the loss was inside the
//! restrictions"*. Pass 1's fold found that implemented as *"waived by the policy
//! word, for everybody, always"*, which made `cooldown_days` dead on every default
//! deployment and let apply → grant → return → apply farm one stipend per cycle out of
//! a pool nothing in this crate can mint into. The fold's fix was to make the cooldown
//! **always** apply until a per-loss marker existed. This is that marker: the waiver's
//! only gate, written once at grant time from a deterministic predicate over recorded
//! facts, and `false` on every grant written before pass 2.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_campaign::{
    CampaignModule, CampaignStore, Composition, FormationAuthority, FormationKind, RecoveryConfig,
    RecoveryGrant, RoleSpec, TICK_PERIOD_SECS,
};
use mush_core::{Audience, ClockRef, ClockStamp, GOD, LIMBO, ObjId, ObjKind, World};
use mush_economy::{AccountId, EconomySnapshot, EconomyStore};
use mush_org::{CapabilityId, PrincipalId};
use mush_proto::OutputEvent;
use mush_sim::{CarrierJourney, JourneyBook, JourneyId, JourneyState, LegDef, RouteDef};
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

// ---- E7r2: the rig (e7r1's, narrowed) --------------------------------------

fn vocabulary() -> BTreeSet<CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(mush_campaign::CAPABILITIES.iter().copied())
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| CapabilityId::new(c).expect("well-formed"))
        .collect()
}

#[derive(Clone, Copy, Debug)]
struct TestKinds;

impl FormationAuthority for TestKinds {
    fn kinds(&self, _world: &World) -> Vec<FormationKind> {
        vec![FormationKind {
            id: SmolStr::new_static("lance"),
            branch: SmolStr::new_static("mech"),
            echelon: 1,
            composition: Composition {
                minimum: 0,
                maximum: 4,
                preferred: 0,
                allowed: BTreeSet::new(),
                required_command: Vec::new(),
                required_support: Vec::new(),
            },
            command_roles: vec![RoleSpec {
                id: SmolStr::new_static("commander"),
                minimum: 0,
            }],
        }]
    }
}

fn boot_with(recovery: RecoveryConfig) -> TestWorld {
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
        Box::new(
            CampaignModule::new()
                .with_formation_kinds(Arc::new(TestKinds))
                .with_recovery(recovery),
        ),
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

fn snap(tw: &TestWorld) -> &EconomySnapshot {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .snapshot()
}
fn camp(tw: &TestWorld) -> &CampaignStore {
    tw.world()
        .module_state::<CampaignStore>()
        .expect("campaign store")
}
fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw)
        .account(id)
        .expect("account")
        .posted_balance
        .minor()
}
fn account_of(tw: &TestWorld, who: ObjId) -> AccountId {
    snap(tw)
        .accounts()
        .find(|a| a.owner == PrincipalId::Object(who))
        .expect("an opened account")
        .id
}

fn fire(tw: &mut TestWorld) {
    let period = i64::try_from(TICK_PERIOD_SECS).unwrap_or(15);
    let t = tw.world().now() + period + 1;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

fn enabled(pool: AccountId) -> RecoveryConfig {
    RecoveryConfig {
        enabled: true,
        pool_account: pool,
        pool_floor: 0,
        tiers: vec![SmolStr::new_static("scout"), SmolStr::new_static("line")],
        applications_per_principal: 1,
        cooldown_secs: mush_campaign::RECOVERY_COOLDOWN_SECS,
        max_operational_assets: 0,
        insurance: SmolStr::new_static("none"),
        upkeep_per_period: 10,
    }
}

fn probe_pool_account() -> AccountId {
    let mut tw = boot_with(RecoveryConfig::default());
    let yard = tw.spawn_player("Yard");
    let _pat = tw.spawn_player("Pat");
    ok(&mut tw, GOD, "@instrument/register gold=0");
    ok(&mut tw, GOD, "@account/system faucet=gold");
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", yard.0));
    account_of(&tw, yard)
}

/// Bound as a spatial participant, placed in the domain, and a formation member —
/// the three facts beyond title and encumbrance.
fn place_and_form(tw: &mut TestWorld, hull: ObjId, node: mush_campaign::FormationId) {
    tw.world_mut()
        .attach_behavior(
            hull,
            mush_core::BehaviorId::new("sim.participant").expect("well-formed"),
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .expect("bind the participant");
    ok(tw, GOD, &format!("@spatial/place/domain #{}=0", hull.0));
    ok(tw, GOD, &format!("@force/assign {node}=#{}", hull.0));
}

struct Fx {
    yard: ObjId,
    pat: ObjId,
    pool: AccountId,
    node: mush_campaign::FormationId,
    field: ObjId,
}

/// The fixture: the provider's pool with `funds` in it, Pat's account, two hulls
/// titled to the yard, and a formation node to assign into.
fn scene(funds: i64, hulls: usize) -> (TestWorld, Fx) {
    let pool = probe_pool_account();
    let mut tw = boot_with(enabled(pool));
    tw.set_now(1_000);
    let yard = tw.spawn_player("Yard");
    let pat = tw.spawn_player("Pat");
    ok(&mut tw, GOD, "@instrument/register gold=0");
    ok(&mut tw, GOD, "@account/system faucet=gold");
    let faucet = snap(&tw)
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    for who in [yard, pat] {
        ok(&mut tw, GOD, &format!("@account/open #{}=gold", who.0));
    }
    assert_eq!(account_of(&tw, yard), pool, "the frozen pool id");
    if funds > 0 {
        ok(&mut tw, GOD, &format!("@econ/mint {faucet}={pool} {funds}"));
    }
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Yard Field", GOD, None)
        .expect("a field");
    tw.world_mut()
        .attach_behavior(
            field,
            mush_core::BehaviorId::new("sim.domain").expect("well-formed"),
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .expect("bind the domain host");
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", field.0),
    );
    ok(&mut tw, GOD, "@org/create Ironhold");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .next()
        .expect("one org")
        .root_scope;
    ok(&mut tw, GOD, &format!("@force/create lance={}", scope.0));
    let node = camp(&tw).formations().map(|n| n.id).next().expect("a node");
    for n in 0..hulls {
        let hull = tw
            .world_mut()
            .create(ObjKind::Thing, &format!("Loaner{n}"), yard, Some(LIMBO))
            .expect("a hull");
        ok(
            &mut tw,
            GOD,
            &format!("@asset/title #{}=#{}", hull.0, yard.0),
        );
        place_and_form(&mut tw, hull, node);
    }
    (
        tw,
        Fx {
            yard,
            pat,
            pool,
            node,
            field,
        },
    )
}

/// **One recorded approved-operation loss for `who`**: a hull they hold title to,
/// assigned to a formation this world records, whose carrier journey reached the
/// terminal `Failed` state.
///
/// The journey record is written straight into cut 6's own `JourneyBook` because the
/// state it needs is *terminal* — driving `@journey/depart` and then failing it would
/// exercise cut 6's tick rather than P0b's predicate, and the predicate is what this
/// file is about. Every field goes through `CarrierJourney`'s own constructor, so
/// nothing here is a shape the tree would refuse.
fn lose_a_hull(tw: &mut TestWorld, fx: &Fx, who: ObjId, state: JourneyState) -> ObjId {
    let hull = tw
        .world_mut()
        .create(ObjKind::Thing, "Wreck", who, Some(LIMBO))
        .expect("a hull");
    ok(tw, GOD, &format!("@asset/title #{}=#{}", hull.0, who.0));
    place_and_form(tw, hull, fx.node);
    // Then **quarantined and unplaced**, which is what cut 6's `fail` path leaves
    // behind for a `Failed`/`Captured` root and what `journey.rs`'s own invariant
    // requires of a terminal record. It is also why a wreck is *not* an accessible
    // operational asset, and therefore why an applicant who lost one is eligible at
    // all. `is_deployable` — the half `loss_is_covered` reads — is formation
    // membership and nothing else, and quarantine does not touch it.
    {
        let state = tw
            .world_mut()
            .module_state_mut::<mush_sim::SpatialState>()
            .expect("the spatial state");
        let record = state.placements.get_mut(&hull).expect("a placement");
        record.parent = mush_sim::SpatialParent::Unplaced;
        record.unplaced = Some(mush_sim::UnplacedInfo {
            reason: "the journey failed".to_owned(),
            by: None,
            at: 0,
            previous: None,
            quarantined: true,
        });
    }
    let stamp = ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: tw.world().now(),
    };
    let route = RouteDef::new(
        mush_core::CanonicalId::new("wreck_run").expect("route id"),
        (fx.field, LIMBO),
        vec![LegDef::new(LIMBO, 60).expect("one leg")],
        None,
        mush_core::CanonicalId::new("torch").expect("drive class"),
        0,
    )
    .expect("a route");
    let journey = CarrierJourney::new(
        JourneyId(1),
        hull,
        route,
        mush_sim::SpatialParent::Interior {
            space: LIMBO,
            slot: None,
        },
        stamp.clone(),
        stamp,
        0,
        state,
        None,
    )
    .expect("a terminal journey");
    tw.world_mut()
        .module_state_mut::<JourneyBook>()
        .expect("the journey book")
        .journeys
        .insert(hull, journey);
    hull
}

// ---- E7r2: P0b — the insured-loss marker -----------------------------------

/// **The pin, both arms: a grant WITHOUT the marker cannot re-apply inside
/// `cooldown_days`; a grant WITH it can.**
///
/// One test, because the two arms are one claim and asserting them apart would let
/// either drift. Pat loses nothing and is cooled down; Robin's counterpart in the
/// second half loses a hull on an approved operation and is not. Everything is
/// observed the way a player observes it — the `@recovery/status` line, and whether
/// the tick actually grants — and the pool is watched throughout, because the whole
/// reason pass 1 closed the waiver is that a farmable one drains it.
#[test]
fn e7r2_p0b_a_grant_without_the_marker_waits_out_the_cooldown_and_one_with_it_does_not() {
    // ---- arm one: no recorded loss, so no waiver ----
    let (mut tw, fx) = scene(1_000, 2);
    ok(&mut tw, fx.pat, "@recovery/apply scout");
    fire(&mut tw);
    let grant = camp(&tw).recovery_grant(fx.pat).cloned().expect("granted");
    assert!(grant.is_live());
    assert!(
        !grant.loss_covered(),
        "an applicant with no recorded wreck carries no marker — the fail-closed \
         default, and the shape every pre-pass-2 document re-enters with"
    );
    ok(&mut tw, fx.pat, "@recovery/return");
    let status = ok(&mut tw, fx.pat, "@recovery/status");
    assert!(
        status.contains("cooldown runs to"),
        "the cooldown is rendered to its own subject: {status}"
    );
    assert!(
        !status.contains("approved-operation loss"),
        "and nothing claims a cover that was not recorded: {status}"
    );
    ok(&mut tw, fx.pat, "@recovery/apply scout");
    let before = bal(&tw, fx.pool);
    fire(&mut tw);
    assert!(
        camp(&tw)
            .recovery_grant(fx.pat)
            .is_some_and(|g| !g.is_live()),
        "a cooled-down applicant is not re-granted"
    );
    assert_eq!(
        bal(&tw, fx.pool),
        before,
        "and the pool paid nothing — which is the farm pass 1 closed"
    );

    // ---- arm two: a recorded approved-operation loss, so the waiver ----
    let (mut tw, fx) = scene(1_000, 2);
    lose_a_hull(&mut tw, &fx, fx.pat, JourneyState::Failed);
    ok(&mut tw, fx.pat, "@recovery/apply scout");
    fire(&mut tw);
    let grant = camp(&tw).recovery_grant(fx.pat).cloned().expect("granted");
    assert!(
        grant.loss_covered(),
        "the wreck was recorded, so the grant carries the marker"
    );
    ok(&mut tw, fx.pat, "@recovery/return");
    let status = ok(&mut tw, fx.pat, "@recovery/status");
    assert!(
        !status.contains("cooldown runs to"),
        "a covered loss renders no cooldown: {status}"
    );
    assert!(
        status.contains("approved-operation loss"),
        "and says why, to the applicant it belongs to: {status}"
    );
    ok(&mut tw, fx.pat, "@recovery/apply scout");
    let before = bal(&tw, fx.pool);
    fire(&mut tw);
    assert!(
        camp(&tw)
            .recovery_grant(fx.pat)
            .is_some_and(RecoveryGrant::is_live),
        "and the second grant is actually made"
    );
    assert!(
        bal(&tw, fx.pool) < before,
        "the stipend was paid out of the provider's own pool, not minted"
    );
}

/// **Both halves of the predicate are load-bearing**: a wreck that was never in a
/// formation is not an approved-operation loss, and a formation member that is not a
/// wreck is not a loss at all.
///
/// Named this way because it is the arm that keeps the waiver from becoming *"anybody
/// who ever owned anything"*: the marker is a per-loss fact and each half of it is
/// checked.
#[test]
fn e7r2_p0b_a_wreck_outside_a_formation_and_a_formation_member_that_is_no_wreck_are_both_uncovered()
{
    // A hull that IS a wreck but was never formed up.
    let (mut tw, fx) = scene(1_000, 2);
    let loose = tw
        .world_mut()
        .create(ObjKind::Thing, "Loose", fx.pat, Some(LIMBO))
        .expect("a hull");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", loose.0, fx.pat.0),
    );
    let stamp = ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: tw.world().now(),
    };
    let route = RouteDef::new(
        mush_core::CanonicalId::new("loose_run").expect("route id"),
        (fx.field, LIMBO),
        vec![LegDef::new(LIMBO, 60).expect("one leg")],
        None,
        mush_core::CanonicalId::new("torch").expect("drive class"),
        0,
    )
    .expect("a route");
    let journey = CarrierJourney::new(
        JourneyId(2),
        loose,
        route,
        mush_sim::SpatialParent::Interior {
            space: LIMBO,
            slot: None,
        },
        stamp.clone(),
        stamp,
        0,
        JourneyState::Failed,
        None,
    )
    .expect("a terminal journey");
    tw.world_mut()
        .module_state_mut::<JourneyBook>()
        .expect("the journey book")
        .journeys
        .insert(loose, journey);
    assert!(
        !mush_campaign::loss_is_covered(tw.world(), fx.pat),
        "a wreck that was never in a formation was not lost on an operation this \
         campaign approved"
    );

    // …and a hull that is formed up but perfectly intact.
    let (mut tw, fx) = scene(1_000, 2);
    let intact = tw
        .world_mut()
        .create(ObjKind::Thing, "Intact", fx.pat, Some(LIMBO))
        .expect("a hull");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", intact.0, fx.pat.0),
    );
    place_and_form(&mut tw, intact, fx.node);
    assert!(
        !mush_campaign::loss_is_covered(tw.world(), fx.pat),
        "a formation member that never went anywhere is no loss"
    );
    // …and the predicate is about the APPLICANT's own losses: the yard's wrecks are
    // not Pat's cover.
    lose_a_hull(&mut tw, &fx, fx.yard, JourneyState::Failed);
    assert!(
        !mush_campaign::loss_is_covered(tw.world(), fx.pat),
        "somebody else's wreck covers nobody"
    );
    assert!(mush_campaign::loss_is_covered(tw.world(), fx.yard));
}

/// **A captured hull is a covered loss too** — cut 6 records `Captured { by }` and
/// `Failed` as the two terminal loss states, and both are losses to the principal who
/// held the hull. Capture changes custody, not title (doc 10 T3), so the applicant is
/// still the title owner and the predicate still finds it.
#[test]
fn e7r2_p0b_a_captured_hull_is_a_covered_loss_and_title_did_not_move() {
    let (mut tw, fx) = scene(1_000, 2);
    let captor = tw.spawn_player("Captor");
    let hull = lose_a_hull(&mut tw, &fx, fx.pat, JourneyState::Captured { by: captor });
    assert_eq!(
        snap(&tw).asset(hull).expect("the titled hull").title_owner,
        PrincipalId::Object(fx.pat),
        "capture moved custody, never title (doc 10 T3)"
    );
    assert!(mush_campaign::loss_is_covered(tw.world(), fx.pat));
    assert!(
        !mush_campaign::loss_is_covered(tw.world(), captor),
        "and taking a hull is not a loss to the captor"
    );
}

/// **The marker re-enters through the same `TryFrom`, and a pre-pass-2 document
/// re-enters as `false`** — the fail-closed direction, in which the cooldown applies.
#[test]
fn e7r2_p0b_the_marker_re_enters_through_the_same_wire_and_defaults_closed() {
    let covered = RecoveryGrant::with_cover(
        ObjId(5),
        7,
        ObjId(9),
        SmolStr::new_static("scout"),
        100,
        Some(200),
        true,
    )
    .expect("a covered grant");
    let json = serde_json::to_value(&covered).expect("serialize");
    assert_eq!(
        json.get("loss_covered")
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "the marker is on the wire"
    );
    let back: RecoveryGrant = serde_json::from_value(json).expect("re-enter");
    assert_eq!(back, covered, "the wire is lossless");

    // A document written before pass 2 has no such field at all.
    let old = serde_json::json!({
        "applicant": 5,
        "contract": 7,
        "loaner": 9,
        "tier": "scout",
        "granted_at": 100,
        "ended_at": 200
    });
    let back: RecoveryGrant = serde_json::from_value(old).expect("an old document re-enters");
    assert!(
        !back.loss_covered(),
        "and re-enters UNCOVERED, so the cooldown applies exactly as pass 1 left it"
    );

    // The wire is still strict in the other direction.
    let bogus = serde_json::json!({
        "applicant": 5,
        "contract": 7,
        "loaner": 9,
        "tier": "scout",
        "granted_at": 100,
        "ended_at": 200,
        "loss_covered": true,
        "waived_by": "staff"
    });
    assert!(
        serde_json::from_value::<RecoveryGrant>(bogus).is_err(),
        "`deny_unknown_fields` still refuses an invented one"
    );

    // And the marker is re-validated by the SAME constructor: a zero applicant is
    // refused with the marker set exactly as it is without.
    assert!(
        RecoveryGrant::with_cover(
            ObjId(0),
            7,
            ObjId(9),
            SmolStr::new_static("scout"),
            100,
            None,
            true,
        )
        .is_err()
    );
}

/// **The cap is encoded**, and the predicate is bounded on records **visited**.
#[test]
fn e7r2_p0b_the_loss_visit_bound_is_encoded() {
    assert_eq!(mush_campaign::MAX_RECOVERY_LOSS_VISIT, 64);
}
