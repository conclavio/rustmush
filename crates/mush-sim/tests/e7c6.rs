//! E7 cut 6 acceptance — carrier journeys, the carrier-root invariant,
//! `@domain/promote`, and the arrival scene IC-floor raise (10b §3.6,
//! RESOLUTION A) — driven through the real `Engine` via `TestWorld`.
//!
//! Three verify lenses shape the rhythms:
//! - **T (topology)**: can any leg, promotion or crash orphan an occupant, a
//!   hold, a mount or a station? The carrier-root invariant is the one answer.
//! - **S (state across cadence)**: can an advance across a crash lose, duplicate
//!   or reorder anything? The leg cursor is the idempotency key, so the tick's
//!   re-run at boot applies nothing twice — and repairs a gap if one exists.
//! - **B (boundaries and limits)**: can a scene delay an arrival, or the
//!   active-domain limit be exceeded or a trigger silently swallowed?

use std::collections::HashMap;

use mush_core::{
    Audience, BehaviorBinding, BehaviorId, ClockRef, ClockStamp, CommandCompatibility, Effects,
    Flags, GOD, LIMBO, ModuleBootData, ModuleWrite, ObjId, ObjKind, Outbox, WorldEvent,
};
use mush_proto::OutputEvent;
use mush_scenes::{SceneMode, SceneRecord, SceneStatus, SceneUid, ScenesModule, ScenesState};
use mush_sim::{
    ActivationBook, CarrierJourney, JourneyBook, JourneyState, MAX_ACTIVE_DOMAINS,
    MAX_CARRIER_SUBTREE, PlacementRecord, SimulationModule, SpatialParent, SpatialState,
    Transition, carrier_journey_key, effective_domain,
};
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

// ---- harness -----------------------------------------------------------

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(SimulationModule::new()),
        Box::new(ScenesModule::new()),
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

/// A programmatic kill, the way the world task does it: `World::destroy`
/// bypasses the staff veto by design, then the engine dispatches the
/// `ObjDestroyed` event the module's backstop subscribes to.
fn destroy(tw: &mut TestWorld, obj: ObjId) {
    tw.world_mut().destroy(obj).expect("destroy");
    let mut out = Outbox::new();
    let mut effects = Effects::new();
    tw.engine_mut()
        .emit_world_event(&WorldEvent::ObjDestroyed { obj }, &mut out, &mut effects);
}

fn state(tw: &TestWorld) -> &SpatialState {
    tw.world()
        .module_state::<SpatialState>()
        .expect("sim state")
}

fn book(tw: &TestWorld) -> &JourneyBook {
    tw.world()
        .module_state::<JourneyBook>()
        .expect("journey book")
}

fn activations(tw: &TestWorld) -> &ActivationBook {
    tw.world()
        .module_state::<ActivationBook>()
        .expect("activation book")
}

fn journey(tw: &TestWorld, root: ObjId) -> CarrierJourney {
    book(tw).get(root).cloned().expect("a carrier journey")
}

fn loc(tw: &TestWorld, id: ObjId) -> Option<ObjId> {
    tw.world().get(id).and_then(|o| o.location)
}

fn parent(tw: &TestWorld, id: ObjId) -> SpatialParent {
    state(tw).parent_of(id)
}

fn placement_revision(tw: &TestWorld, id: ObjId) -> Option<u64> {
    state(tw).placements.get(&id).map(|p| p.revision)
}

/// Drive the world's wall clock to `secs` and fire every due tick.
fn advance(tw: &mut TestWorld, secs: i64) -> String {
    tw.set_now(secs);
    texts(&tw.fire_due(secs * 1000))
}

fn fold_kv(tw: &mut TestWorld, ns: &str) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        if op.ns != ns {
            continue;
        }
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}

/// Every `sim` document key the last command touched, sets and deletes alike
/// (a fold would hide a delete, which is exactly what this asserts about).
fn op_keys(tw: &mut TestWorld) -> Vec<String> {
    tw.take_module_ops()
        .into_iter()
        .filter(|op| op.ns == "sim")
        .filter_map(|op| match op.op {
            ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key } => Some(key.to_string()),
            ModuleWrite::StreamAppend { .. } => None,
        })
        .collect()
}

fn doc_mut<'a>(kv: &'a mut [(SmolStr, serde_json::Value)], key: &str) -> &'a mut serde_json::Value {
    &mut kv
        .iter_mut()
        .find(|(candidate, _)| candidate == key)
        .unwrap_or_else(|| panic!("missing sim doc {key}"))
        .1
}

// ---- the fixture -------------------------------------------------------

/// A dropship with three anchored interiors, six occupants, a mounted craft, a
/// loaded hold and a crewed station, standing in domain #0 with domain #1 as
/// its destination.
struct Fx {
    field: ObjId,
    phobos: ObjId,
    carrier: ObjId,
    bay: ObjId,
    hold: ObjId,
    cargo: ObjId,
    aboard: Vec<ObjId>,
    craft: ObjId,
    pilot: ObjId,
}

fn build(tw: &mut TestWorld) -> Fx {
    let mk_room = |tw: &mut TestWorld, name: &str| {
        tw.world_mut()
            .create(ObjKind::Room, name, GOD, None)
            .unwrap()
    };
    let mk_thing = |tw: &mut TestWorld, name: &str| {
        tw.world_mut()
            .create(ObjKind::Thing, name, GOD, Some(LIMBO))
            .unwrap()
    };
    let field = mk_room(tw, "Field");
    let phobos = mk_room(tw, "Phobos");
    let bay = mk_room(tw, "Bay");
    let hold = mk_room(tw, "Hold");
    let cargo = mk_room(tw, "Cargo");
    let carrier = mk_thing(tw, "Leopard");
    let craft = mk_thing(tw, "Locust");
    let aboard: Vec<ObjId> = (0..6)
        .map(|i| mk_thing(tw, &format!("Trooper{i}")))
        .collect();
    let pilot = tw
        .world_mut()
        .create(ObjKind::Player, "Ann", GOD, Some(LIMBO))
        .unwrap();

    bind(tw, field, "sim.domain");
    bind(tw, phobos, "sim.domain");
    for room in [bay, hold, cargo] {
        bind(tw, room, "sim.interior");
    }
    bind(tw, carrier, "sim.participant");
    bind(tw, craft, "sim.participant");
    for a in &aboard {
        bind(tw, *a, "sim.participant");
    }

    ok(tw, &format!("@domain/create #{}=sim.passive", field.0));
    ok(tw, &format!("@domain/create #{}=sim.passive", phobos.0));
    ok(tw, &format!("@spatial/place/domain #{}=0", carrier.0));
    for room in [bay, hold, cargo] {
        ok(
            tw,
            &format!("@spatial/space #{}=moving:#{}", room.0, carrier.0),
        );
    }
    // Two troopers per interior: everything hosted, nothing left over.
    for (i, a) in aboard.iter().enumerate() {
        let room = [bay, hold, cargo][i / 2];
        ok(tw, &format!("@spatial/place/interior #{}=#{}", a.0, room.0));
    }
    ok(
        tw,
        &format!("@spatial/mount #{}=hardpoint,2,exposed", carrier.0),
    );
    ok(
        tw,
        &format!(
            "@spatial/place/attached #{}=#{},hardpoint",
            craft.0, carrier.0
        ),
    );
    ok(
        tw,
        &format!(
            "@spatial/station #{}=pilot,pilot,channels:movement",
            carrier.0
        ),
    );
    ok(
        tw,
        &format!("@spatial/assign #{}=pilot,#{}", carrier.0, pilot.0),
    );
    Fx {
        field,
        phobos,
        carrier,
        bay,
        hold,
        cargo,
        aboard,
        craft,
        pilot,
    }
}

fn scene() -> (TestWorld, Fx) {
    let mut tw = boot();
    tw.set_now(1_000);
    let fx = build(&mut tw);
    (tw, fx)
}

/// Depart on a two-leg corridor: 100 IC seconds to a waypoint, then 200 to
/// Phobos (domain #1's host).
fn depart(tw: &mut TestWorld, fx: &Fx) -> String {
    ok(
        tw,
        &format!(
            "@journey/depart #{}=corridor #{}:100 #{}:200",
            fx.carrier.0, fx.field.0, fx.phobos.0
        ),
    )
}

/// Every `(object, parent)` pair under the carrier — the conservation snapshot.
fn subtree_shape(tw: &TestWorld, root: ObjId) -> Vec<(ObjId, SpatialParent)> {
    let st = state(tw);
    let mut out: Vec<(ObjId, SpatialParent)> = st
        .subtree(root)
        .into_iter()
        .map(|o| (o, st.parent_of(o)))
        .collect();
    out.sort_by_key(|(o, _)| o.0);
    out
}

// ======================================================================
// T — topology: can a leg, a promotion or a crash orphan anything?
// ======================================================================

/// A carrier with three anchored interiors, six occupants, a mounted craft and
/// a loaded hold departs and arrives with **every relationship intact** — the
/// set that moved is conserved exactly, and only the root's own placement
/// changed.
#[test]
fn e7c6_t1_a_carrier_arrives_with_its_whole_subtree_intact() {
    let (mut tw, fx) = scene();
    let before = subtree_shape(&tw, fx.carrier);
    assert_eq!(before.len(), 11, "root + 3 interiors + 6 aboard + 1 craft");
    depart(&mut tw, &fx);
    advance(&mut tw, 1_301);

    assert_eq!(*journey(&tw, fx.carrier).state(), JourneyState::Arrived);
    let after = subtree_shape(&tw, fx.carrier);
    assert_eq!(
        before.iter().map(|(o, _)| *o).collect::<Vec<_>>(),
        after.iter().map(|(o, _)| *o).collect::<Vec<_>>(),
        "the subtree membership is conserved across the passage"
    );
    for ((o, was), (_, now)) in before.iter().zip(after.iter()) {
        if *o == fx.carrier {
            continue;
        }
        assert_eq!(was, now, "{o} changed parent during the passage");
    }
    // Only the root moved, and it moved to the destination domain.
    assert!(
        matches!(parent(&tw, fx.carrier), SpatialParent::Direct { domain, .. } if domain.0 == 1)
    );
    assert_eq!(
        effective_domain(state(&tw), fx.carrier).map(|d| d.0),
        Some(1)
    );
    // Everything aboard projects where it should: occupants in their interior,
    // the exposed craft with the carrier.
    for (i, a) in fx.aboard.iter().enumerate() {
        assert_eq!(loc(&tw, *a), Some([fx.bay, fx.hold, fx.cargo][i / 2]));
    }
    assert_eq!(loc(&tw, fx.carrier), Some(fx.phobos));
    assert_eq!(loc(&tw, fx.craft), Some(fx.phobos));
    mush_sim::verify_indexes(state(&tw)).unwrap();
    assert!(mush_sim::verify_projections(state(&tw), tw.world()).is_empty());
}

/// An occupant who walks between compartments mid-leg is in the **right place**
/// at arrival: the seal permits movement *within* the carrier and the arrival
/// re-derives every projection from the records, never from a snapshot.
#[test]
fn e7c6_t2_an_occupant_who_moves_between_compartments_mid_leg_is_right_at_arrival() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    advance(&mut tw, 1_150);
    let walker = fx.aboard[0];
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", walker.0, fx.cargo.0),
    );
    advance(&mut tw, 1_301);
    assert!(
        matches!(parent(&tw, walker), SpatialParent::Interior { space, .. } if space == fx.cargo)
    );
    assert_eq!(loc(&tw, walker), Some(fx.cargo));
    assert!(state(&tw).subtree(fx.carrier).contains(&walker));
}

/// **The seal.** While a carrier is under way nothing crosses its boundary in
/// either direction — a craft cannot launch mid-leg, and nobody boards
/// mid-passage. Both are permitted again once it has arrived.
#[test]
fn e7c6_t3_the_carrier_subtree_is_sealed_while_under_way() {
    let (mut tw, fx) = scene();
    let outsider = tw
        .world_mut()
        .create(ObjKind::Thing, "Stray", GOD, Some(LIMBO))
        .unwrap();
    bind(&mut tw, outsider, "sim.participant");
    depart(&mut tw, &fx);

    let launch = err(&mut tw, &format!("@spatial/place/domain #{}=1", fx.craft.0));
    assert!(launch.contains("under way"), "{launch}");
    let board = err(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", outsider.0, fx.bay.0),
    );
    assert!(board.contains("under way"), "{board}");

    advance(&mut tw, 1_301);
    // Arrived: the boundary is open again.
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", outsider.0, fx.bay.0),
    );
    ok(&mut tw, &format!("@spatial/place/domain #{}=1", fx.craft.0));
}

/// **A topology cycle is unconstructible.** A carrier cannot come to rest inside
/// its own hold at any depth: the plan refuses before departure, and the ONE
/// checked carrier move refuses the same shape directly.
#[test]
fn e7c6_t4_a_carrier_root_cannot_come_to_rest_inside_its_own_subtree() {
    let (mut tw, fx) = scene();
    let refused = err(
        &mut tw,
        &format!("@journey/depart #{}=loop #{}:100", fx.carrier.0, fx.bay.0),
    );
    assert!(refused.contains("its own hold"), "{refused}");

    for parent in [
        SpatialParent::Interior {
            space: fx.bay,
            slot: None,
        },
        SpatialParent::Interior {
            space: fx.cargo,
            slot: None,
        },
        SpatialParent::Attached {
            host: fx.carrier,
            mount: mush_sim::MountId::new("hardpoint").unwrap(),
            local_pose: None,
        },
    ] {
        let outcome = mush_sim::plan(
            state(&tw),
            tw.world(),
            &Transition::CarrierMove {
                root: fx.carrier,
                parent: Some(parent),
            },
        );
        assert!(
            matches!(outcome, Err(mush_sim::SpatialError::Cycle { .. })),
            "a self-carrying carrier move must be unconstructible, got {outcome:?}"
        );
    }
}

/// The root is **never directly placed in a live domain while cruising** — and
/// its subtree still rides with it, which is the whole point of the invariant.
#[test]
fn e7c6_t5_the_root_is_not_placed_in_a_live_domain_while_cruising() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    assert_eq!(parent(&tw, fx.carrier), SpatialParent::Unplaced);
    assert!(!state(&tw).is_placed(fx.carrier));
    assert!(
        !state(&tw).is_quarantined(fx.carrier),
        "under way is not quarantine"
    );
    assert_eq!(effective_domain(state(&tw), fx.carrier), None);
    assert!(
        state(&tw)
            .derived
            .domain_direct
            .get(&mush_sim::DomainId(0))
            .is_none_or(|d| !d.contains(&fx.carrier))
    );
    assert_eq!(state(&tw).subtree(fx.carrier).len(), 11, "still hosting");
}

/// Destroying an anchored interior mid-leg **orphans no occupant**: they are
/// quarantined into the recovery room, the carrier's remaining subtree stays
/// coherent, and the passage still completes.
#[test]
fn e7c6_t6_destroying_an_anchored_interior_mid_leg_orphans_no_occupant() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    advance(&mut tw, 1_150);
    destroy(&mut tw, fx.cargo);
    for a in &fx.aboard[4..] {
        assert!(
            state(&tw).is_quarantined(*a),
            "{a} was orphaned instead of quarantined"
        );
        assert_eq!(loc(&tw, *a), Some(state(&tw).recovery_room));
    }
    advance(&mut tw, 1_301);
    assert_eq!(*journey(&tw, fx.carrier).state(), JourneyState::Arrived);
    assert!(
        matches!(parent(&tw, fx.carrier), SpatialParent::Direct { domain, .. } if domain.0 == 1)
    );
    mush_sim::verify_indexes(state(&tw)).unwrap();
}

/// **The `controls_are_live` fix** (10b §3.6 contradiction (v)). A cruising
/// carrier root is not directly placed, so the old bare `is_placed` gate dropped
/// every crew grant the moment the ship left. The crew keeps the helm the whole
/// passage, and a journey that ends drops nothing either.
#[test]
fn e7c6_t7_a_cruising_carriers_crew_keeps_the_helm() {
    let (mut tw, fx) = scene();
    let held = |tw: &TestWorld| {
        tw.world()
            .command_subjects_of(fx.pilot)
            .iter()
            .any(|g| g.subject == fx.carrier)
    };
    assert!(held(&tw), "the pilot crews the carrier before departure");
    depart(&mut tw, &fx);
    assert!(
        held(&tw),
        "the crew must not silently lose control mid-journey"
    );
    advance(&mut tw, 1_150);
    assert!(held(&tw), "still crewed mid-leg");
    advance(&mut tw, 1_301);
    assert!(held(&tw), "and after arrival");
}

/// The move/carry gate is the other half of "the ONE checked carrier move": a
/// carrier under way is refused by the ordinary movement path, so
/// `move_carrier_subtree` really is the only way it moves.
#[test]
fn e7c6_t8_a_carrier_under_way_is_refused_by_the_ordinary_movement_path() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    let reason = mush_sim::journey::journeying_reason(book(&tw), tw.world(), fx.carrier);
    assert!(
        reason.is_some_and(|r| r.contains("under way")),
        "the gate must refuse an ordinary move of a carrier under way"
    );
    // An unrelated object is untouched by the journey gate.
    assert!(mush_sim::journey::journeying_reason(book(&tw), tw.world(), fx.craft).is_none());
}

/// The carrier subtree walk is bounded on records **visited** (D-LBK-5), closing
/// §7's missing per-carrier cap: past [`MAX_CARRIER_SUBTREE`] the move is
/// refused rather than building an unbounded arrival bundle.
#[test]
fn e7c6_t9_the_carrier_subtree_walk_is_bounded() {
    let (mut tw, fx) = scene();
    // Load the bay past the cap directly: the point under test is the bound in
    // `plan`, not the authoring path that would take a thousand commands.
    {
        let st = tw
            .world_mut()
            .module_state_mut::<SpatialState>()
            .expect("sim state");
        for i in 0..MAX_CARRIER_SUBTREE {
            let object = ObjId(50_000 + u32::try_from(i).expect("a small index"));
            st.placements.insert(
                object,
                PlacementRecord {
                    object,
                    parent: SpatialParent::Interior {
                        space: fx.bay,
                        slot: None,
                    },
                    revision: 1,
                    unplaced: None,
                },
            );
        }
        st.derived = mush_sim::graph::rebuild(st);
    }
    assert!(state(&tw).subtree(fx.carrier).len() > MAX_CARRIER_SUBTREE);
    let outcome = mush_sim::plan(
        state(&tw),
        tw.world(),
        &Transition::CarrierMove {
            root: fx.carrier,
            parent: None,
        },
    );
    assert!(
        matches!(
            outcome,
            Err(mush_sim::SpatialError::CarrierTooLarge { max, .. }) if max == MAX_CARRIER_SUBTREE
        ),
        "got {outcome:?}"
    );
}

// ======================================================================
// S — state across cadence: the leg cursor is the idempotency key
// ======================================================================

/// **The reboot proof.** An arrival is one atomic batch; a reboot re-runs the
/// same idempotent advance over the same documents, and the placement moved
/// **exactly once** — same revision, same parent — with the state still
/// `Arrived`. Firing the tick again afterwards changes nothing.
#[test]
fn e7c6_s1_a_reboot_re_runs_the_tick_and_the_arrival_applied_exactly_once() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    advance(&mut tw, 1_301);
    let revision = placement_revision(&tw, fx.carrier).expect("a placement");
    let shape = subtree_shape(&tw, fx.carrier);
    let kv = fold_kv(&mut tw, "sim");

    let mut tw2 = boot();
    tw2.set_now(2_000);
    let fx2 = build(&mut tw2);
    assert_eq!(fx2.carrier, fx.carrier, "the fixture is deterministic");
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        // `..ModuleBootData::default()` rather than naming every field: the
        // struct grows (a ledger lane, a fetch timer) and a rig that names them
        // all breaks on every addition.
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    assert_eq!(*journey(&tw2, fx.carrier).state(), JourneyState::Arrived);
    assert_eq!(
        placement_revision(&tw2, fx.carrier),
        Some(revision),
        "the reboot re-ran the tick and applied the arrival a second time"
    );
    assert_eq!(subtree_shape(&tw2, fx.carrier), shape);
    // And the live tick is a no-op past what is applied.
    advance(&mut tw2, 9_000);
    assert_eq!(placement_revision(&tw2, fx.carrier), Some(revision));
}

/// A carrier mid-leg hydrates and finishes its passage: the cursor is durable,
/// so the restart resumes exactly where the boundary left it.
#[test]
fn e7c6_s2_a_carrier_mid_leg_hydrates_and_finishes_the_passage() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    advance(&mut tw, 1_150);
    assert_eq!(journey(&tw, fx.carrier).cursor(), 1);
    assert_eq!(*journey(&tw, fx.carrier).state(), JourneyState::Cruising);
    let kv = fold_kv(&mut tw, "sim");

    let mut tw2 = boot();
    tw2.set_now(1_150);
    let _ = build(&mut tw2);
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        // `..ModuleBootData::default()` rather than naming every field: the
        // struct grows (a ledger lane, a fetch timer) and a rig that names them
        // all breaks on every addition.
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    assert_eq!(
        journey(&tw2, fx.carrier).cursor(),
        1,
        "mid-leg, not restarted"
    );
    assert_eq!(parent(&tw2, fx.carrier), SpatialParent::Unplaced);
    advance(&mut tw2, 1_301);
    assert_eq!(*journey(&tw2, fx.carrier).state(), JourneyState::Arrived);
    assert!(
        matches!(parent(&tw2, fx.carrier), SpatialParent::Direct { domain, .. } if domain.0 == 1)
    );
}

/// **The crash gap heals at boot.** A store whose journey document says
/// `Arrived` but whose placement document still says "under way" is exactly the
/// gap a crash between the two writes would leave. The boot re-run of the tick
/// repairs it — the same code path a live arrival takes, so the two can never
/// disagree.
#[test]
fn e7c6_s3_an_arrived_journey_whose_carrier_never_moved_heals_at_boot() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    advance(&mut tw, 1_301);
    let mut kv = fold_kv(&mut tw, "sim");
    // Rewind ONLY the placement document, keeping the arrived journey: the
    // carrier is durably "under way" while its journey says it landed.
    let placement = doc_mut(&mut kv, &format!("placement:{}", fx.carrier.0));
    *placement = serde_json::json!({
        "object": fx.carrier.0,
        "parent": { "kind": "Unplaced" },
        "revision": 2,
        "unplaced": {
            "reason": "under way on a carrier journey",
            "by": null,
            "at": 1_000,
            "previous": null,
            "quarantined": false
        }
    });
    let arrived: CarrierJourney =
        serde_json::from_value(doc_mut(&mut kv, carrier_journey_key(fx.carrier).as_str()).clone())
            .expect("the arrived journey re-enters");
    assert_eq!(*arrived.state(), JourneyState::Arrived);

    let mut tw2 = boot();
    tw2.set_now(2_000);
    let _ = build(&mut tw2);
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        // `..ModuleBootData::default()` rather than naming every field: the
        // struct grows (a ledger lane, a fetch timer) and a rig that names them
        // all breaks on every addition.
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    assert!(
        matches!(parent(&tw2, fx.carrier), SpatialParent::Direct { domain, .. } if domain.0 == 1),
        "boot did not heal the crash gap: {:?}",
        parent(&tw2, fx.carrier)
    );
    assert_eq!(*journey(&tw2, fx.carrier).state(), JourneyState::Arrived);
    // And healing is itself idempotent.
    let revision = placement_revision(&tw2, fx.carrier);
    advance(&mut tw2, 9_000);
    assert_eq!(placement_revision(&tw2, fx.carrier), revision);
}

/// A duplicate fire is a HIT: the tick re-firing at the same instant, and at a
/// later instant with nothing due, applies nothing.
#[test]
fn e7c6_s4_a_duplicate_fire_is_a_hit() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    advance(&mut tw, 1_150);
    let cursor = journey(&tw, fx.carrier).cursor();
    let revision = placement_revision(&tw, fx.carrier);
    for _ in 0..3 {
        advance(&mut tw, 1_150);
        assert_eq!(journey(&tw, fx.carrier).cursor(), cursor);
        assert_eq!(placement_revision(&tw, fx.carrier), revision);
    }
    advance(&mut tw, 1_299);
    assert_eq!(journey(&tw, fx.carrier).cursor(), cursor, "nothing due yet");
}

/// **Journeys never pause** (D-E7-9) — deliberately the opposite of production's
/// default. A world rate-generation change re-anchors the journey onto the new
/// generation, preserving the IC duration still to run, instead of stranding a
/// stamp nobody may compare.
#[test]
fn e7c6_s5_a_rate_generation_change_re_anchors_and_never_pauses() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    advance(&mut tw, 1_150);
    let before = journey(&tw, fx.carrier);
    assert_eq!(before.departed().clock, ClockRef::WorldIc { generation: 1 });
    let epoch = tw
        .world()
        .next_world_timeline_epoch(2)
        .expect("a new world-IC rate epoch");
    tw.world_mut()
        .append_world_timeline_epoch(epoch)
        .expect("the epoch appends");
    advance(&mut tw, 1_160);
    let after = journey(&tw, fx.carrier);
    assert_eq!(
        after.departed().clock,
        ClockRef::WorldIc { generation: 2 },
        "the journey re-anchored instead of stalling"
    );
    assert_eq!(after.cursor(), before.cursor(), "no leg was skipped");
    // The remaining IC duration is the un-applied legs, exactly.
    let remaining = after.arrival_at().expect("a due stamp") - tw.world().world_ic_now().at;
    assert_eq!(
        remaining, 200,
        "the second leg still has all 200 IC seconds"
    );
    // And it still arrives.
    advance(&mut tw, 10_000);
    assert_eq!(*journey(&tw, fx.carrier).state(), JourneyState::Arrived);
}

/// A `due`/leg-sum overflow **pages** instead of stalling (E5a S2): the journey
/// stops, records the diagnosis with the recommended command already filled in,
/// and stops nothing else.
#[test]
fn e7c6_s6_a_due_overflow_pages_instead_of_stalling() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    let mut kv = fold_kv(&mut tw, "sim");
    let doc = doc_mut(&mut kv, carrier_journey_key(fx.carrier).as_str());
    doc["departed"]["at"] = serde_json::json!(i64::MAX - 10);
    doc["advanced_at"]["at"] = serde_json::json!(i64::MAX - 10);

    let mut tw2 = boot();
    tw2.set_now(2_000);
    let _ = build(&mut tw2);
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        // `..ModuleBootData::default()` rather than naming every field: the
        // struct grows (a ledger lane, a fetch timer) and a rig that names them
        // all breaks on every addition.
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    let stopped = journey(&tw2, fx.carrier);
    assert_eq!(*stopped.state(), JourneyState::Failed);
    let anomaly = stopped.anomaly().expect("a paged diagnosis");
    assert!(anomaly.contains("overflow"), "{anomaly}");
    assert!(
        anomaly.contains("@journey/divert"),
        "the page must carry the recommended command: {anomaly}"
    );
    // Only the subject stopped: the rest of the graph is untouched.
    mush_sim::verify_indexes(state(&tw2)).unwrap();
}

/// A carrier journey record re-enters through its constructor, and a corrupt
/// document is skipped **loudly** rather than silently defaulted (§4 rule 2).
#[test]
fn e7c6_s7_a_corrupt_journey_document_is_skipped_not_defaulted() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    let mut kv = fold_kv(&mut tw, "sim");
    let doc = doc_mut(&mut kv, carrier_journey_key(fx.carrier).as_str());
    doc["cursor"] = serde_json::json!(99); // past the route's two legs

    let mut tw2 = boot();
    tw2.set_now(1_000);
    let _ = build(&mut tw2);
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        // `..ModuleBootData::default()` rather than naming every field: the
        // struct grows (a ledger lane, a fetch timer) and a rig that names them
        // all breaks on every addition.
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    assert!(
        book(&tw2).get(fx.carrier).is_none(),
        "a document past its own cap must be refused, never truncated into service"
    );
}

// ======================================================================
// B — boundaries: the scene floor, and the active-domain limit
// ======================================================================

/// Put an active scene in `room` and return its id.
fn open_scene(tw: &mut TestWorld, room: ObjId, ic_floor: i64, rounds: u32) -> SceneUid {
    let scenes = tw
        .world_mut()
        .module_state_mut::<ScenesState>()
        .expect("scenes state");
    let id = scenes.alloc_uid();
    scenes.scenes.insert(
        id,
        SceneRecord {
            id,
            title: "Aboard".into(),
            kind: SmolStr::new_static("timestop"),
            mode: SceneMode::TurnBased,
            status: SceneStatus::Active,
            pause_reason: None,
            exclusive: false,
            anchor: room,
            participants: Vec::new(),
            observers: Vec::new(),
            facilitator: GOD,
            created_by: GOD,
            created_at: 0,
            ic_started_at: ClockStamp {
                clock: ClockRef::WorldIc { generation: 1 },
                at: 0,
            },
            clock_generation: 1,
            turn: 0,
            round_seconds: 6,
            round: rounds,
            turn_index: 0,
            ended_at: None,
            sealer: None,
            seal_run: None,
            outcome: None,
            ic_floor,
            log_chunks: None,
        },
    );
    id
}

fn scene_of(tw: &TestWorld, id: SceneUid) -> SceneRecord {
    tw.world()
        .module_state::<ScenesState>()
        .expect("scenes state")
        .scenes
        .get(&id)
        .cloned()
        .expect("the scene")
}

/// **An arrival raises each onboard scene's IC floor to the arrival stamp**
/// through E6b's one seam, and leaves `clock_generation` at 1 (08i answer 9).
#[test]
fn e7c6_b1_an_arrival_raises_each_onboard_scene_floor_and_keeps_generation_1() {
    let (mut tw, fx) = scene();
    let aboard = open_scene(&mut tw, fx.bay, 0, 1);
    let also = open_scene(&mut tw, fx.hold, 0, 1);
    let ashore = open_scene(&mut tw, fx.phobos, 0, 1);
    depart(&mut tw, &fx);
    advance(&mut tw, 1_301);

    assert_eq!(scene_of(&tw, aboard).ic_floor, 1_300, "raised to arrival");
    assert_eq!(scene_of(&tw, also).ic_floor, 1_300);
    assert_eq!(
        scene_of(&tw, ashore).ic_floor,
        0,
        "a scene that was not aboard is untouched"
    );
    for id in [aboard, also] {
        assert_eq!(
            scene_of(&tw, id).clock_generation,
            1,
            "an arrival raise must never bump the scene clock generation"
        );
    }
    // The raised record is mirrored in the same batch as the arrival.
    let kv = fold_kv(&mut tw, "scenes");
    assert!(
        kv.iter()
            .any(|(k, _)| k.as_str() == mush_scenes::scene_key(aboard).as_str()),
        "the raised scene document was not persisted: {:?}",
        kv.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>()
    );
}

/// **A scene cannot delay an arrival** (Q6A/S6A). A forty-round scene aboard is
/// still running when the passage is due; the arrival commits first and the
/// floor rises after it.
#[test]
fn e7c6_b2_a_forty_round_scene_does_not_delay_the_arrival() {
    let (mut tw, fx) = scene();
    let aboard = open_scene(&mut tw, fx.bay, 0, 40);
    depart(&mut tw, &fx);
    advance(&mut tw, 1_301);
    assert!(
        matches!(parent(&tw, fx.carrier), SpatialParent::Direct { domain, .. } if domain.0 == 1),
        "the scene held the journey"
    );
    let record = scene_of(&tw, aboard);
    assert_eq!(record.status, SceneStatus::Active, "the scene continues");
    assert_eq!(record.round, 40, "as a new chapter, not a new scene");
    assert_eq!(record.ic_floor, 1_300);
}

/// A raise **at or below** the current floor is refused, and the refusal is
/// paged rather than held: the journey still arrives.
#[test]
fn e7c6_b3_a_backwards_floor_raise_is_refused_and_never_holds_the_journey() {
    let (mut tw, fx) = scene();
    let aboard = open_scene(&mut tw, fx.bay, 9_000, 1);
    depart(&mut tw, &fx);
    advance(&mut tw, 1_301);
    assert_eq!(
        scene_of(&tw, aboard).ic_floor,
        9_000,
        "the floor only ever moves forward"
    );
    assert_eq!(*journey(&tw, fx.carrier).state(), JourneyState::Arrived);
    assert!(
        matches!(parent(&tw, fx.carrier), SpatialParent::Direct { domain, .. } if domain.0 == 1)
    );
}

/// A **sealed** scene refuses the raise (its IC time is canon) and the journey
/// still arrives.
#[test]
fn e7c6_b4_a_sealed_scene_refuses_the_raise_and_the_journey_still_arrives() {
    let (mut tw, fx) = scene();
    let aboard = open_scene(&mut tw, fx.bay, 0, 1);
    {
        let scenes = tw
            .world_mut()
            .module_state_mut::<ScenesState>()
            .expect("scenes state");
        let record = scenes.scenes.get_mut(&aboard).expect("the scene");
        record.sealer = Some(GOD);
        record.outcome = mush_scenes::SceneOutcome::new(
            SmolStr::new_static("success"),
            vec![GOD],
            String::new(),
            GOD,
            0,
        );
        assert!(record.outcome.is_some(), "the fixture seals the scene");
    }
    depart(&mut tw, &fx);
    advance(&mut tw, 1_301);
    assert_eq!(scene_of(&tw, aboard).ic_floor, 0, "canon is immutable");
    assert_eq!(*journey(&tw, fx.carrier).state(), JourneyState::Arrived);
}

// ======================================================================
// Promotion and demotion (B5)
// ======================================================================

/// `@domain/promote` writes a **request** and leaves the composition-frozen
/// `ClockProviderRegistry` byte-identical — a probe snapshots it before and
/// after. No `Pulse`, no provider, no registry mutation.
#[test]
fn e7c6_b5_promote_writes_a_request_and_leaves_the_clock_registry_byte_identical() {
    let (mut tw, _fx) = scene();
    let before = tw
        .registry()
        .clock_providers()
        .map(|r| (r.fingerprint().to_hex(), r.len()));
    let text = ok(&mut tw, "@domain/promote 0=contact");
    assert!(text.contains("REQUEST"), "{text}");
    let after = tw
        .registry()
        .clock_providers()
        .map(|r| (r.fingerprint().to_hex(), r.len()));
    assert_eq!(before, after, "@domain/promote mutated the clock registry");

    let request = activations(&tw)
        .activations
        .get(&mush_sim::DomainId(0))
        .cloned()
        .expect("an activation request");
    assert_eq!(
        request.trigger(),
        mush_sim::PromotionTrigger::CredibleContact
    );
    assert!(!request.is_queued());
    assert_eq!(request.at().clock, ClockRef::WorldIc { generation: 1 });
    // It is a document, at a deterministic key.
    let kv = fold_kv(&mut tw, "sim");
    assert!(
        kv.iter().any(|(k, _)| k.as_str() == "activation:0"),
        "{kv:?}"
    );
}

/// Each of the four triggers promotes exactly once, and a fifth trigger on an
/// already-promoted domain does **not** re-promote.
#[test]
fn e7c6_b5_each_trigger_promotes_once_and_a_fifth_does_not_re_promote() {
    let (mut tw, _fx) = scene();
    for (i, token) in ["player", "contact", "interception", "operation"]
        .into_iter()
        .enumerate()
    {
        let domain = u64::try_from(i % 2).expect("a small index");
        let text = ok(&mut tw, &format!("@domain/promote {domain}={token}"));
        if i < 2 {
            assert!(text.contains("requested promotion"), "{text}");
        } else {
            assert!(text.contains("already promoted"), "{text}");
        }
    }
    assert_eq!(activations(&tw).live(), 2);
    // The standing request keeps its ORIGINAL trigger: a fifth does not rewrite it.
    assert_eq!(
        activations(&tw).activations[&mush_sim::DomainId(0)].trigger(),
        mush_sim::PromotionTrigger::PlayerPresent
    );
    let refused = err(&mut tw, "@domain/promote 99=player");
    assert!(refused.contains("No domain #99"), "{refused}");
}

/// At `MAX_ACTIVE_DOMAINS` the next trigger **queues and re-evaluates**, never
/// drops; a freed slot takes it.
#[test]
fn e7c6_b5_at_the_limit_a_trigger_queues_and_re_evaluates() {
    let (mut tw, _fx) = scene();
    // Two domains exist from the fixture; add enough to pass the limit.
    for i in 0..=MAX_ACTIVE_DOMAINS {
        let room = tw
            .world_mut()
            .create(ObjKind::Room, &format!("Rock{i}"), GOD, None)
            .unwrap();
        bind(&mut tw, room, "sim.domain");
        ok(&mut tw, &format!("@domain/create #{}=sim.passive", room.0));
    }
    for id in 0..MAX_ACTIVE_DOMAINS {
        ok(&mut tw, &format!("@domain/promote {id}=player"));
    }
    assert_eq!(activations(&tw).live(), MAX_ACTIVE_DOMAINS);
    let over = u64::try_from(MAX_ACTIVE_DOMAINS).expect("a small limit");
    let text = ok(&mut tw, &format!("@domain/promote {over}=interception"));
    assert!(text.contains("QUEUED"), "{text}");
    assert!(
        activations(&tw)
            .activations
            .contains_key(&mush_sim::DomainId(over)),
        "a queued trigger must never be dropped"
    );
    // Free a slot: the queued request is re-evaluated and takes it.
    ok(&mut tw, "@domain/demote 0");
    assert!(activations(&tw).is_promoted(mush_sim::DomainId(over)));
    assert_eq!(activations(&tw).live(), MAX_ACTIVE_DOMAINS);
}

/// Demotion writes the durable summary **before** it stands down, and state
/// survives promote → demote → promote byte-identically (Q15A: demotion changes
/// cadence, never state).
#[test]
fn e7c6_b5_demotion_writes_the_summary_first_and_state_survives_the_round_trip() {
    let (mut tw, fx) = scene();
    let before = subtree_shape(&tw, fx.carrier);
    ok(&mut tw, "@domain/promote 0=player");
    let _ = fold_kv(&mut tw, "sim");
    advance(&mut tw, 1_030);
    let text = ok(&mut tw, "@domain/demote 0");
    assert!(text.contains("summary is written first"), "{text}");
    // The ops of THAT command: the summary is queued before the activation is
    // deleted, so a reader can never see a demoted domain with no summary.
    let ops: Vec<String> = tw
        .take_module_ops()
        .into_iter()
        .filter(|o| o.ns == "sim")
        .map(|o| match o.op {
            ModuleWrite::JsonSet { key, .. } => format!("set {key}"),
            ModuleWrite::Del { key } => format!("del {key}"),
            ModuleWrite::StreamAppend { .. } => "stream".into(),
        })
        .collect();
    let set_summary = ops.iter().position(|o| o == "set summary:0");
    let del_activation = ops.iter().position(|o| o == "del activation:0");
    assert!(
        set_summary.is_some() && del_activation.is_some() && set_summary < del_activation,
        "{ops:?}"
    );
    assert!(!activations(&tw).is_promoted(mush_sim::DomainId(0)));
    assert!(
        activations(&tw)
            .summaries
            .contains_key(&mush_sim::DomainId(0))
    );
    assert_eq!(
        subtree_shape(&tw, fx.carrier),
        before,
        "demotion changed state"
    );
    ok(&mut tw, "@domain/promote 0=player");
    assert_eq!(
        subtree_shape(&tw, fx.carrier),
        before,
        "promote -> demote -> promote changed state"
    );
    let refused = err(&mut tw, "@domain/demote 1");
    assert!(refused.contains("not promoted"), "{refused}");
}

/// Demotion refuses while something cut 6 owns is still pending: a carrier under
/// way whose journey ends in that domain.
#[test]
fn e7c6_b5_demotion_refuses_while_a_carrier_is_inbound() {
    let (mut tw, fx) = scene();
    ok(&mut tw, "@domain/promote 1=interception");
    depart(&mut tw, &fx);
    let refused = err(&mut tw, "@domain/demote 1");
    assert!(refused.contains("inbound"), "{refused}");
    advance(&mut tw, 1_301);
    ok(&mut tw, "@domain/demote 1");
}

// ======================================================================
// Projection and wiring (P)
// ======================================================================

/// Two refusals a viewer may not distinguish are **byte-identical** (§4 rule 4):
/// a domain nobody observed and one that never existed answer the same, and so
/// do a carrier with no journey and an object that never existed.
#[test]
fn e7c6_p1_unobserved_and_nonexistent_reads_are_byte_identical() {
    let (mut tw, fx) = scene();
    let unobserved = ok(&mut tw, "@domain/summary 0");
    let nonexistent = ok(&mut tw, "@domain/summary 4242");
    let nonsense = ok(&mut tw, "@domain/summary Nowhere-At-All");
    assert_eq!(unobserved, nonexistent);
    assert_eq!(unobserved, nonsense);
    assert_eq!(unobserved, "No summary for that domain.");

    let no_journey = ok(&mut tw, &format!("@journey/show #{}", fx.carrier.0));
    let no_object = ok(&mut tw, "@journey/show Nowhere-At-All");
    assert_eq!(no_journey, no_object);
    assert_eq!(no_journey, "No journey.");
}

/// `@journey/plan` is the dry run of `@journey/depart`: same arguments, same
/// validation, and it writes **nothing**.
#[test]
fn e7c6_p2_plan_is_a_dry_run_that_writes_nothing() {
    let (mut tw, fx) = scene();
    let _ = fold_kv(&mut tw, "sim");
    let text = ok(
        &mut tw,
        &format!(
            "@journey/plan #{}=corridor #{}:100 #{}:200",
            fx.carrier.0, fx.field.0, fx.phobos.0
        ),
    );
    assert!(text.contains("Planned journey"), "{text}");
    assert!(text.contains("Nothing was written"), "{text}");
    assert_eq!(fold_kv(&mut tw, "sim"), Vec::new(), "the dry run wrote");
    assert!(book(&tw).get(fx.carrier).is_none());
    assert!(state(&tw).is_placed(fx.carrier), "it never left");
}

/// `@journey/divert` re-plans a carrier that is under way; a carrier that is not
/// under way refuses.
#[test]
fn e7c6_p3_divert_re_plans_a_carrier_under_way() {
    let (mut tw, fx) = scene();
    let refused = err(
        &mut tw,
        &format!(
            "@journey/divert #{}=other #{}:50",
            fx.carrier.0, fx.phobos.0
        ),
    );
    assert!(refused.contains("not under way"), "{refused}");
    depart(&mut tw, &fx);
    let id = journey(&tw, fx.carrier).id();
    advance(&mut tw, 1_150);
    ok(
        &mut tw,
        &format!(
            "@journey/divert #{}=shortcut #{}:50",
            fx.carrier.0, fx.phobos.0
        ),
    );
    let after = journey(&tw, fx.carrier);
    assert_eq!(after.id(), id, "a divert keeps the journey's identity");
    assert_eq!(after.cursor(), 0, "the new route starts from here");
    assert!(matches!(after.state(), JourneyState::Diverted { .. }));
    assert!(after.state().is_under_way());
    advance(&mut tw, 1_205);
    assert_eq!(*journey(&tw, fx.carrier).state(), JourneyState::Arrived);
}

/// Each verb publishes its recorded compatibility class (K8.5), and `@journey`
/// takes the CEILING of its four switches.
#[test]
fn e7c6_p4_each_verb_publishes_its_recorded_class() {
    let (tw, _fx) = scene();
    let registrations: Vec<(String, Option<String>, CommandCompatibility)> = tw
        .registry()
        .command_registrations()
        .map(|(spec, owner, class)| (spec.name.to_string(), owner.map(str::to_string), class))
        .collect();
    for (name, class) in [
        ("@journey", CommandCompatibility::LegacyPostImage),
        ("@domain", CommandCompatibility::LegacyPostImage),
        ("@spatial", CommandCompatibility::LegacyPostImage),
        ("+stations", CommandCompatibility::ReadOnly),
        ("@station", CommandCompatibility::Authoritative),
    ] {
        let found = registrations
            .iter()
            .find(|(candidate, _, _)| candidate == name)
            .unwrap_or_else(|| panic!("{name} is not registered"));
        assert_eq!(found.1.as_deref(), Some("sim"), "{name}");
        assert_eq!(found.2, class, "{name} publishes the wrong lane");
    }
}

/// A station-assignment edit never touches a placement document, and a carrier
/// move never touches a station document — the control topology and the
/// placement lane stay separate records (§3.6).
#[test]
fn e7c6_p5_a_station_edit_never_touches_a_placement_document() {
    let (mut tw, fx) = scene();
    let _ = fold_kv(&mut tw, "sim");
    ok(
        &mut tw,
        &format!("@spatial/unassign #{}=pilot", fx.carrier.0),
    );
    let keys = op_keys(&mut tw);
    assert!(
        keys.iter().any(|k| k.starts_with("assignment:")),
        "{keys:?}"
    );
    assert!(
        !keys.iter().any(|k| k.starts_with("placement:")),
        "a station edit touched a placement document: {keys:?}"
    );

    ok(
        &mut tw,
        &format!("@spatial/assign #{}=pilot,#{}", fx.carrier.0, fx.pilot.0),
    );
    let _ = fold_kv(&mut tw, "sim");
    depart(&mut tw, &fx);
    let keys = op_keys(&mut tw);
    assert!(
        keys.iter()
            .any(|k| k == &format!("placement:{}", fx.carrier.0)),
        "{keys:?}"
    );
    assert!(
        !keys
            .iter()
            .any(|k| k.starts_with("stations:") || k.starts_with("assignment:")),
        "a carrier move touched a control-topology document: {keys:?}"
    );
}

/// The legacy `@route` runtime is untouched, and the two never fight over one
/// object: a carrier under way refuses a `@route`, and a traveler refuses a
/// journey.
#[test]
fn e7c6_p6_the_legacy_route_runtime_is_untouched() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    let refused = err(
        &mut tw,
        &format!("@route #{}=#{}:5", fx.carrier.0, fx.phobos.0),
    );
    assert!(refused.contains("simulation"), "{refused}");

    let courier = tw
        .world_mut()
        .create(ObjKind::Thing, "Courier", GOD, Some(fx.field))
        .unwrap();
    ok(
        &mut tw,
        &format!("@route #{}=#{}:5", courier.0, fx.phobos.0),
    );
    bind(&mut tw, courier, "sim.participant");
    let refused = err(
        &mut tw,
        &format!("@journey/depart #{}=x #{}:5", courier.0, fx.phobos.0),
    );
    assert!(refused.contains("legacy @route"), "{refused}");
}

/// A destroyed carrier root drops its journey: no ghost record keeps a dead
/// subtree "under way".
#[test]
fn e7c6_p7_a_destroyed_carrier_root_drops_its_journey() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    assert!(book(&tw).under_way(fx.carrier));
    destroy(&mut tw, fx.carrier);
    assert!(book(&tw).get(fx.carrier).is_none());
    assert!(
        tw.world()
            .get(fx.carrier)
            .is_none_or(|o| o.flags.contains(Flags::GOING))
    );
}
