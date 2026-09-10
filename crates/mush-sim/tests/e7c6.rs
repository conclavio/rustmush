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
    // Bounded on records VISITED, not on the finished result (E7 cut 6 fold,
    // C8(a)): the walk stops one record past the cap, so the refusal costs the
    // cap and not the graph. The old form reported the whole subtree's size,
    // which was itself the proof that it had walked all of it.
    assert!(
        matches!(
            outcome,
            Err(mush_sim::SpatialError::CarrierTooLarge { count, .. })
                if count <= MAX_CARRIER_SUBTREE + 1
        ),
        "the walk visited every record before applying the cap: {outcome:?}"
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
    // **The IC duration still to run is PRESERVED, not restarted** (E7 cut 6
    // fold, R2). The world IC axis is continuous across a generation, so a
    // re-anchor relabels the departure stamp and moves no boundary at all: the
    // arrival lands at the same absolute IC position it always would have. The
    // shipped form of this assertion pinned `remaining == 200` — the WHOLE
    // second leg — which encoded the bug (a rate change extended the passage)
    // as intent.
    assert_eq!(
        after.departed().at,
        before.departed().at,
        "a re-anchor relabels the generation; it must not re-base the departure"
    );
    assert_eq!(
        after.arrival_at(),
        before.arrival_at(),
        "a re-anchor moved a leg boundary"
    );
    let remaining = after.arrival_at().expect("a due stamp") - tw.world().world_ic_now().at;
    let elapsed_on_leg = tw.world().world_ic_now().at - 1_100;
    assert_eq!(
        remaining,
        200 - elapsed_on_leg,
        "the seconds already flown on the in-flight leg must not be handed back"
    );
    assert!(remaining < 200, "the leg was restarted: {remaining}");
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
    // And refused is not the same as gone (E7 cut 6 fold, R3): the document is
    // HELD, its key is recorded, and the carrier it names — whose whole crewed
    // subtree is otherwise nowhere — is quarantined where staff can find it.
    let key = carrier_journey_key(fx.carrier).to_string();
    assert!(
        book(&tw2).held.contains_key(key.as_str()),
        "the refused document was not held: {:?}",
        book(&tw2).held
    );
    assert!(state(&tw2).is_quarantined(fx.carrier));
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
            consent: Vec::new(),
            nudge_after_s: None,
            turn_started_at: 0,
            turn_visit: 0,
            nudged: None,
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
        // **E7 cut 8 independent verify fold B1:** `@domain` publishes
        // `Authoritative`. It is one `CommandSpec` and therefore takes the
        // CEILING class of its switches; `/tick` reaches
        // `Ctx::submit_authoritative(domain_pulse_arm, ..)` from the handler.
        ("@domain", CommandCompatibility::Authoritative),
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

// ======================================================================
// ---- E7 cut 6 independent verify fold ----
//
// One regression test per confirmed finding, named `vf_<lens><n>_…`. The
// owner decisions taken are D1(a) (a stopped journey quarantines its root and
// the gate widens to any journey record on an unplaced root), D2(a) (a new
// `SpatialState.in_transit_room`), D3(a) + filter-before-take (a
// revision-scoped `applied_revision`), D4(ii) (the promotion row is recorded as
// deviation (i) and pinned in all four cells) and D5(b) (the reads open to a
// billet-holder, the writes stay staff-only).
// ======================================================================

/// The production module order — `ScenesModule` BEFORE `SimulationModule`, as
/// `mush-server::modules::server_modules_with` composes them — for the cases
/// that turn on the scenes seam.
fn scenes_first() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(ScenesModule::new()),
        Box::new(SimulationModule::new()),
    ])
}

/// Every `sim` op the last command produced, as `("set"|"del", key)` — a fold
/// would hide a delete, and R3 is entirely about a delete.
fn op_pairs(tw: &mut TestWorld) -> Vec<(&'static str, String)> {
    tw.take_module_ops()
        .into_iter()
        .filter(|op| op.ns == "sim")
        .filter_map(|op| match op.op {
            ModuleWrite::JsonSet { key, .. } => Some(("set", key.to_string())),
            ModuleWrite::Del { key } => Some(("del", key.to_string())),
            ModuleWrite::StreamAppend { .. } => None,
        })
        .collect()
}

fn run_as(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    texts(&tw.run(actor, line))
}

fn thing(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Thing, name, GOD, Some(LIMBO))
        .unwrap()
}

/// Re-write a live journey record with a different state, the way a later cut's
/// authority (or a hand-edited document) would.
fn set_state(tw: &mut TestWorld, root: ObjId, state: JourneyState) {
    let j = journey(tw, root);
    let next = CarrierJourney::new(
        j.id(),
        j.root(),
        j.route().clone(),
        j.destination().clone(),
        j.departed().clone(),
        j.advanced_at().clone(),
        j.cursor(),
        state,
        None,
    )
    .expect("a coherent journey");
    tw.world_mut()
        .module_state_mut::<JourneyBook>()
        .expect("journey book")
        .journeys
        .insert(root, next);
}

// ---- C1: the seal is no longer one-armed --------------------------------

/// **C1(a).** A craft aboard a cruising mothership cannot be given a journey of
/// its own: the `CarrierMove` arm is sealed, so it cannot leave the sealed
/// subtree mid-leg the way `@spatial/place` was already refused from doing.
#[test]
fn vf_c1a_a_craft_cannot_launch_from_a_sealed_mothership() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    assert!(book(&tw).under_way(fx.carrier));
    let before = subtree_shape(&tw, fx.carrier);

    let refused = err(
        &mut tw,
        &format!("@journey/depart #{}=escape #{}:10", fx.craft.0, fx.phobos.0),
    );
    assert!(refused.contains("under way"), "{refused}");
    assert!(book(&tw).get(fx.craft).is_none(), "a record was written");
    assert_eq!(
        subtree_shape(&tw, fx.carrier),
        before,
        "the sealed subtree lost a record"
    );
    // The PROJECTION, not just the record: the craft is still bolted to a ship
    // in passage, so it is exactly where the ship is.
    assert_eq!(loc(&tw, fx.craft), loc(&tw, fx.carrier));
    assert_eq!(loc(&tw, fx.craft), Some(state(&tw).in_transit_room));

    // The transition itself refuses, not merely the command wrapper.
    let outcome = mush_sim::plan(
        state(&tw),
        tw.world(),
        &Transition::CarrierMove {
            root: fx.craft,
            parent: None,
        },
    );
    assert!(
        matches!(outcome, Err(mush_sim::SpatialError::CarrierSealed { root, .. }) if root == fx.carrier),
        "got {outcome:?}"
    );
}

/// **C1(b).** A journey may not END inside a carrier that is under way — at
/// plan time, and (for a passage already flown) at arrival, where it stops and
/// pages instead of boarding eleven records onto a moving ship.
#[test]
fn vf_c1b_an_arriving_carrier_cannot_board_an_under_way_one() {
    let (mut tw, fx) = scene();
    let hauler = thing(&mut tw, "Hauler");
    let dock = tw
        .world_mut()
        .create(ObjKind::Room, "Dock", GOD, None)
        .unwrap();
    bind(&mut tw, hauler, "sim.participant");
    bind(&mut tw, dock, "sim.interior");
    ok(&mut tw, &format!("@spatial/place/domain #{}=0", hauler.0));
    ok(
        &mut tw,
        &format!("@spatial/space #{}=moving:#{}", dock.0, hauler.0),
    );

    // The carrier sets out for the hauler's dock while the hauler is still
    // stationary — legal at the moment it departs.
    ok(
        &mut tw,
        &format!(
            "@journey/depart #{}=ferry #{}:100 #{}:200",
            fx.carrier.0, fx.field.0, dock.0
        ),
    );
    ok(
        &mut tw,
        &format!("@journey/depart #{}=haul #{}:900", hauler.0, fx.phobos.0),
    );
    assert!(book(&tw).under_way(hauler));

    // Plan time: nobody else may aim at that dock now.
    let stray = thing(&mut tw, "Stray");
    bind(&mut tw, stray, "sim.participant");
    ok(&mut tw, &format!("@spatial/place/domain #{}=0", stray.0));
    let plan_refusal = err(
        &mut tw,
        &format!("@journey/plan #{}=late #{}:10", stray.0, dock.0),
    );
    assert!(plan_refusal.contains("under way"), "{plan_refusal}");

    // Arrival time: the passage was already flown, so it stops and pages
    // rather than crossing the boundary.
    advance(&mut tw, 1_301);
    let stopped = journey(&tw, fx.carrier);
    assert_eq!(*stopped.state(), JourneyState::Failed);
    assert!(
        stopped.anomaly().is_some_and(|a| a.contains("under way")),
        "{:?}",
        stopped.anomaly()
    );
    assert!(
        !state(&tw).subtree(hauler).contains(&fx.carrier),
        "eleven records boarded a ship that is under way"
    );
    // And D1 held the stopped ship rather than leaving it loose — projected
    // into the recovery room, not into the moving hauler's dock.
    assert!(state(&tw).is_quarantined(fx.carrier));
    assert_eq!(loc(&tw, fx.carrier), Some(state(&tw).recovery_room));
    assert!(!tw.world().contents_of(dock).contains(&fx.carrier));
    for a in &fx.aboard {
        assert!(!tw.world().contents_of(dock).contains(a));
    }
}

/// **C1(c).** `Unplace` is sealed too — which is the only place the economy's
/// `InteriorCustodyProvider::move_out` can be refused, because it deliberately
/// skips this module's own `CarryAttempt` hook.
#[test]
fn vf_c1c_unplace_is_sealed_too() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    let trooper = fx.aboard[0];

    let refusal = err(
        &mut tw,
        &format!("@spatial/unplace #{}=pulled mid-passage", trooper.0),
    );
    assert!(refusal.contains("under way"), "{refusal}");
    assert!(state(&tw).subtree(fx.carrier).contains(&trooper));
    // The projection is untouched too: the trooper is still in the bay aboard
    // the ship, not standing wherever the withdrawal aimed him.
    assert_eq!(loc(&tw, trooper), Some(fx.bay));
    assert!(tw.world().contents_of(fx.bay).contains(&trooper));

    // The check lives in the TRANSITION, so every caller is covered — the
    // custody withdrawal path included.
    let outcome = mush_sim::plan(
        state(&tw),
        tw.world(),
        &Transition::Unplace {
            obj: trooper,
            reason: "withdrawn".into(),
            by: Some(GOD),
            keep: true,
        },
    );
    assert!(
        matches!(outcome, Err(mush_sim::SpatialError::CarrierSealed { obj, root }) if obj == trooper && root == fx.carrier),
        "got {outcome:?}"
    );
}

/// **D4's four-cell pin.** `Place` and `@journey/depart` of an aboard craft,
/// once with the root `Cruising` and once `Promoted` — all four cells refuse.
/// Cut 6's seal treats `Promoted` exactly as `Cruising` (it is `is_under_way`,
/// the ONE predicate), and nothing in cut 6 writes `Promoted` at all: the
/// launch-once-promoted rule arrives with cut 8's tactical domain, which is its
/// first writer. Recorded as deviation (i); the row can never again be neither
/// built nor recorded.
#[test]
fn vf_c1e_the_promoted_cell_is_pinned() {
    for promoted in [false, true] {
        let (mut tw, fx) = scene();
        depart(&mut tw, &fx);
        if promoted {
            set_state(
                &mut tw,
                fx.carrier,
                JourneyState::Promoted {
                    domain: mush_sim::DomainId(1),
                },
            );
            assert!(book(&tw).under_way(fx.carrier));
        }
        let placed = err(&mut tw, &format!("@spatial/place/domain #{}=1", fx.craft.0));
        assert!(
            placed.contains("under way"),
            "promoted={promoted}: {placed}"
        );
        let launched = err(
            &mut tw,
            &format!("@journey/depart #{}=escape #{}:10", fx.craft.0, fx.phobos.0),
        );
        assert!(
            launched.contains("under way"),
            "promoted={promoted}: {launched}"
        );
    }
}

// ---- B1/D2: a departed carrier actually leaves ---------------------------

/// **B1 (D2(a)).** A departed carrier and its exposed mounts LEAVE the room they
/// departed from: the projection is the new `in_transit_room`, not "left where
/// it is", and the projection audit agrees at every phase.
#[test]
fn vf_b1_a_departed_carrier_leaves_the_room() {
    let (mut tw, fx) = scene();
    assert_eq!(loc(&tw, fx.carrier), Some(fx.field));
    assert_eq!(loc(&tw, fx.craft), Some(fx.field), "an exposed mount");
    assert!(mush_sim::verify_projections(state(&tw), tw.world()).is_empty());

    depart(&mut tw, &fx);
    let transit = state(&tw).in_transit_room;
    assert_eq!(parent(&tw, fx.carrier), SpatialParent::Unplaced);
    assert_eq!(
        loc(&tw, fx.carrier),
        Some(transit),
        "a departed carrier must not still be in the room it left"
    );
    assert_eq!(
        loc(&tw, fx.craft),
        Some(transit),
        "an exposed mount rides with the ship it is bolted to"
    );
    assert!(
        !tw.world().contents_of(fx.field).contains(&fx.carrier),
        "the origin room still contains the departed carrier"
    );
    assert!(!tw.world().contents_of(fx.field).contains(&fx.craft));
    assert!(mush_sim::verify_projections(state(&tw), tw.world()).is_empty());

    advance(&mut tw, 1_150);
    assert_eq!(loc(&tw, fx.carrier), Some(transit), "still in passage");
    assert!(mush_sim::verify_projections(state(&tw), tw.world()).is_empty());

    advance(&mut tw, 1_301);
    assert_eq!(loc(&tw, fx.carrier), Some(fx.phobos));
    assert_eq!(loc(&tw, fx.craft), Some(fx.phobos));
    assert!(mush_sim::verify_projections(state(&tw), tw.world()).is_empty());
}

// ---- C4/D1: a stopped journey never un-gates its carrier -----------------

/// **C4 (D1(a)).** Pausing the destination domain mid-leg stops the journey —
/// and the stopped ship is HELD, not left as an ordinary takeable object a
/// bystander can pick up with its crew and mounted craft aboard.
#[test]
fn vf_c4a_a_paused_destination_leaves_a_gated_carrier() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    let thief = tw
        .world_mut()
        .create(ObjKind::Player, "Bystander", GOD, Some(LIMBO))
        .unwrap();
    ok(&mut tw, "@domain/pause 1");
    advance(&mut tw, 1_301);

    let stopped = journey(&tw, fx.carrier);
    assert_eq!(*stopped.state(), JourneyState::Failed);
    assert!(
        stopped
            .anomaly()
            .is_some_and(|a| a.contains("could not be placed")),
        "{:?}",
        stopped.anomaly()
    );
    assert!(state(&tw).is_quarantined(fx.carrier), "the ship is loose");
    assert_eq!(loc(&tw, fx.carrier), Some(state(&tw).recovery_room));
    assert_eq!(state(&tw).subtree(fx.carrier).len(), 11, "still hosting");

    // The real hook, not just the predicate: an ordinary player standing where
    // the ship is projected cannot take it.
    let events = tw.run(thief, &format!("get #{}", fx.carrier.0));
    assert!(
        has_error(&events),
        "a non-staff player picked up a dropship: {}",
        texts(&events)
    );
    assert_ne!(loc(&tw, fx.carrier), Some(thief));
}

/// **C4, the `Captured` arm (deviation (f)).** A terminal state a LATER cut
/// writes lands in the same place, and the gate covers it by construction —
/// the widened `journeying_reason` is phrased over record presence and
/// unplacedness, not over a second under-way predicate.
#[test]
fn vf_c4b_a_captured_carrier_is_still_gated() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    let thief = tw
        .world_mut()
        .create(ObjKind::Player, "Bystander", GOD, Some(LIMBO))
        .unwrap();
    set_state(&mut tw, fx.carrier, JourneyState::Captured { by: GOD });
    assert!(!book(&tw).under_way(fx.carrier));
    advance(&mut tw, 1_150);

    let st = state(&tw);
    assert!(
        st.is_placed(fx.carrier) || st.is_quarantined(fx.carrier),
        "a captured carrier is neither placed nor held"
    );
    assert!(
        mush_sim::journey::journeying_reason(book(&tw), tw.world(), fx.carrier).is_some(),
        "the gate went silent for a terminal journey"
    );
    let events = tw.run(thief, &format!("get #{}", fx.carrier.0));
    assert!(has_error(&events), "{}", texts(&events));
}

/// **R7 (with B4).** The command a stopped journey's page RECOMMENDS actually
/// runs — before the fold `@journey/divert` refused exactly the journey whose
/// anomaly named it, and the obvious second try (`@journey/depart`) refused too
/// because the stopped root is unplaced.
#[test]
fn vf_c4c_the_anomalys_command_actually_recovers() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    let mut kv = fold_kv(&mut tw, "sim");
    let doc = doc_mut(&mut kv, carrier_journey_key(fx.carrier).as_str());
    doc["departed"]["at"] = serde_json::json!(i64::MAX - 10);
    doc["advanced_at"]["at"] = serde_json::json!(i64::MAX - 10);

    let mut tw2 = boot();
    tw2.set_now(2_000);
    let fx2 = build(&mut tw2);
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    let stopped = journey(&tw2, fx2.carrier);
    assert_eq!(*stopped.state(), JourneyState::Failed);
    let anomaly = stopped.anomaly().expect("a paged diagnosis").to_string();
    assert!(anomaly.contains("@journey/divert"), "{anomaly}");
    assert!(state(&tw2).is_quarantined(fx2.carrier), "and it is held");

    // The exact shape the page names.
    ok(
        &mut tw2,
        &format!(
            "@journey/divert #{}=recover #{}:10",
            fx2.carrier.0, fx2.phobos.0
        ),
    );
    assert!(book(&tw2).under_way(fx2.carrier));
    assert!(
        !state(&tw2).is_quarantined(fx2.carrier),
        "a ship under way again must leave the recovery room"
    );
    advance(&mut tw2, 2_011);
    assert_eq!(*journey(&tw2, fx2.carrier).state(), JourneyState::Arrived);
}

/// **C5.** Every writer of a `controls_are_live` input refreshes: a stopped
/// journey re-derives the crew's links in the same edit, instead of leaving a
/// live grant standing until some unrelated edit silently drops it.
#[test]
fn vf_c5_a_stopped_journey_refreshes_its_links() {
    let (mut tw, fx) = scene();
    let held = |tw: &TestWorld| {
        tw.world()
            .command_subjects_of(fx.pilot)
            .iter()
            .any(|g| g.subject == fx.carrier)
    };
    depart(&mut tw, &fx);
    assert!(held(&tw), "the crew has the helm while cruising");
    ok(&mut tw, "@domain/pause 1");
    advance(&mut tw, 1_301);
    assert_eq!(*journey(&tw, fx.carrier).state(), JourneyState::Failed);
    let after_fail = held(&tw);
    // An unrelated edit must not change the answer — that is the whole point.
    ok(
        &mut tw,
        &format!("@spatial/station #{}=gunner,gunner", fx.carrier.0),
    );
    assert_eq!(
        held(&tw),
        after_fail,
        "an unrelated station edit changed the published links, so `fail` had left them stale"
    );
}

// ---- C3/D3: the arrival heal expires ------------------------------------

/// **C3 (D3(a)).** An `Arrived` journey's repair claim is scoped to the
/// placement revision it landed at, so a deliberate later relocation stands
/// instead of being reverted — whole subtree included, with a revision bump, a
/// duplicate narrative and a false scene page — on the next 1 s fire, forever.
#[test]
fn vf_c3_a_later_placement_of_an_arrived_carrier_is_not_undone() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    advance(&mut tw, 1_301);
    assert!(
        matches!(parent(&tw, fx.carrier), SpatialParent::Direct { domain, .. } if domain.0 == 1)
    );
    ok(
        &mut tw,
        &format!("@spatial/place/domain #{}=0", fx.carrier.0),
    );
    let revision = placement_revision(&tw, fx.carrier).expect("a placement");
    let _ = fold_kv(&mut tw, "sim");

    for t in 1..=3 {
        advance(&mut tw, 1_400 + t);
        assert!(
            matches!(parent(&tw, fx.carrier), SpatialParent::Direct { domain, .. } if domain.0 == 0),
            "the arrived journey dragged the carrier back on fire {t}"
        );
        assert_eq!(placement_revision(&tw, fx.carrier), Some(revision));
    }
    let keys = op_keys(&mut tw);
    assert!(
        !keys
            .iter()
            .any(|k| k == &format!("placement:{}", fx.carrier.0)),
        "the heal kept rewriting the placement: {keys:?}"
    );
    for a in &fx.aboard {
        assert!(state(&tw).subtree(fx.carrier).contains(a));
    }
}

// ---- R2: a re-anchor preserves the remaining duration --------------------

/// **R2.** A world-IC rate generation change moves NO boundary: the IC axis is
/// continuous, so re-anchoring relabels the departure stamp and the arrival
/// lands at the same absolute IC position. Repeated bumps therefore cannot
/// defer an arrival without limit.
#[test]
fn vf_r2_a_rate_generation_change_preserves_the_exact_remaining_duration() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    let arrival = journey(&tw, fx.carrier).arrival_at().expect("a due stamp");
    advance(&mut tw, 1_290);
    let remaining_before = arrival - tw.world().world_ic_now().at;
    assert_eq!(remaining_before, 10);

    let epoch = tw
        .world()
        .next_world_timeline_epoch(1)
        .expect("a new world-IC epoch");
    tw.world_mut()
        .append_world_timeline_epoch(epoch)
        .expect("the epoch appends");
    advance(&mut tw, 1_291);
    let after = journey(&tw, fx.carrier);
    assert_eq!(
        after.arrival_at(),
        Some(arrival),
        "the re-anchor moved the arrival stamp"
    );
    assert_eq!(
        arrival - tw.world().world_ic_now().at,
        9,
        "the re-anchor handed back seconds that had already been flown"
    );

    // A bump cadence shorter than the leg must not defer the arrival forever.
    for step in 0..5 {
        let epoch = tw
            .world()
            .next_world_timeline_epoch(1)
            .expect("a new world-IC epoch");
        tw.world_mut()
            .append_world_timeline_epoch(epoch)
            .expect("the epoch appends");
        advance(&mut tw, 1_292 + step);
    }
    advance(&mut tw, 1_400);
    assert_eq!(
        *journey(&tw, fx.carrier).state(),
        JourneyState::Arrived,
        "repeated rate changes deferred the arrival past IC {}",
        tw.world().world_ic_now().at
    );
}

// ---- R3: a refused journey document is held, never deleted ---------------

/// **R3.** An under-way carrier is deliberately not placed, so its `cjourney:`
/// document is the ONLY record of where the ship and its whole crewed subtree
/// are. A wire refusal must therefore never be a durable delete.
#[test]
fn vf_r3_an_unreadable_carrier_journey_is_held_not_deleted() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    let mut kv = fold_kv(&mut tw, "sim");
    let key = carrier_journey_key(fx.carrier).to_string();
    doc_mut(&mut kv, &key)["cursor"] = serde_json::json!(99); // past the route's two legs

    let mut tw2 = boot();
    tw2.set_now(1_150);
    let fx2 = build(&mut tw2);
    let _ = tw2.take_module_ops();
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    assert!(
        book(&tw2).get(fx2.carrier).is_none(),
        "refused, as designed"
    );
    let ops = op_pairs(&mut tw2);
    assert!(
        !ops.iter().any(|(kind, k)| *kind == "del" && *k == key),
        "boot issued a durable delete for the only record of an under-way carrier: {ops:?}"
    );
    assert!(
        book(&tw2).held.contains_key(key.as_str()),
        "the refused document was not recorded as held: {:?}",
        book(&tw2).held
    );
    assert!(
        state(&tw2).is_quarantined(fx2.carrier),
        "the stranded carrier is neither placed nor findable"
    );
    assert_eq!(loc(&tw2, fx2.carrier), Some(state(&tw2).recovery_room));
}

// ---- R4: a rate change must not wedge a promoted domain ------------------

/// **R4.** `DomainSummary::new` demanded one clock generation, so after ANY
/// world rate change every standing activation became un-demotable — forever,
/// across every reboot, accumulating against `MAX_ACTIVE_DOMAINS`.
#[test]
fn vf_r4_a_promoted_domain_demotes_across_a_rate_generation_change() {
    let (mut tw, _fx) = scene();
    ok(&mut tw, "@domain/promote 0=player");
    let epoch = tw
        .world()
        .next_world_timeline_epoch(2)
        .expect("a new world-IC epoch");
    tw.world_mut()
        .append_world_timeline_epoch(epoch)
        .expect("the epoch appends");
    tw.set_now(2_000);

    let text = ok(&mut tw, "@domain/demote 0");
    assert!(text.contains("summary is written first"), "{text}");
    assert!(!activations(&tw).is_promoted(mush_sim::DomainId(0)));
    let summary = activations(&tw)
        .summaries
        .get(&mush_sim::DomainId(0))
        .cloned()
        .expect("a summary");
    assert_eq!(
        summary.elapsed(),
        tw.world().world_ic_now().at - 1_000,
        "the elapsed IC window must stay exact across the generation"
    );
    assert_eq!(summary.to().clock, tw.world().world_ic_now().clock);
    // And the slot really is free again.
    ok(&mut tw, "@domain/promote 0=player");
    assert_eq!(activations(&tw).live(), 1);
}

// ---- C7: a journey cannot dock into a custody backing --------------------

/// **C7.** An arrival is not `@spatial/place`: it is `apply_arrival` →
/// `move_carrier_subtree`, and `InteriorCustodyProvider::contents` derives vault
/// stock purely from the placement graph. Docking into a vault backing would
/// therefore be an unjournaled deposit of a ship and everyone aboard.
#[test]
fn vf_c7_a_journey_cannot_dock_into_a_vault_backing() {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(SimulationModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![
            std::sync::Arc::new(mush_sim::InteriorCustodyProvider),
        ])),
    ]);
    tw.set_now(1_000);
    let fx = build(&mut tw);
    let warehouse = tw
        .world_mut()
        .create(ObjKind::Room, "Bonded Warehouse", GOD, None)
        .unwrap();
    bind(&mut tw, warehouse, "sim.interior");
    ok(&mut tw, &format!("@spatial/space #{}=fixed:1", warehouse.0));
    let created = ok(
        &mut tw,
        &format!("@vault/create bay=interior:#{}", warehouse.0),
    );
    assert!(created.contains("Created vault"), "{created}");

    for verb in ["plan", "depart"] {
        let refusal = err(
            &mut tw,
            &format!(
                "@journey/{verb} #{}=corridor #{}:100",
                fx.carrier.0, warehouse.0
            ),
        );
        assert!(refusal.contains("custody vault"), "{verb}: {refusal}");
    }
    assert!(book(&tw).get(fx.carrier).is_none());
    assert!(state(&tw).is_placed(fx.carrier), "it never left");
}

// ---- C8: the walk is bounded, and Place is capped too --------------------

/// **C8(b).** `Transition::Place` builds its projection bundle from the same
/// subtree walk a carrier move does, so ONE constant caps both — before the
/// fold, `@spatial/place/domain` on an over-cap root built exactly the bundle
/// `MAX_CARRIER_SUBTREE` exists to refuse.
#[test]
fn vf_c8_place_is_capped_too() {
    let (mut tw, fx) = scene();
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
    let outcome = mush_sim::plan(
        state(&tw),
        tw.world(),
        &Transition::Place {
            obj: fx.carrier,
            parent: SpatialParent::Direct {
                domain: mush_sim::DomainId(1),
                pose: mush_sim::Pose::passive(
                    mush_sim::passive_rules_value(serde_json::json!({})).expect("a pose"),
                ),
            },
        },
    );
    assert!(
        matches!(
            outcome,
            Err(mush_sim::SpatialError::CarrierTooLarge { count, max, .. })
                if max == MAX_CARRIER_SUBTREE && count <= MAX_CARRIER_SUBTREE + 1
        ),
        "got {outcome:?}"
    );
}

// ---- C9/D5: who may read a journey ---------------------------------------

/// **C9 (D5(b)).** The reads open to an actor who crews the carrier; the writes
/// stay staff-only; a bystander gets the byte-identical rule-4 line, so the read
/// leaks nothing.
#[test]
fn vf_c9_a_billeted_pilot_may_read_its_own_journey() {
    let (mut tw, fx) = scene();
    let bystander = tw
        .world_mut()
        .create(ObjKind::Player, "Passer", GOD, Some(LIMBO))
        .unwrap();
    depart(&mut tw, &fx);

    let crewed = run_as(
        &mut tw,
        fx.pilot,
        &format!("@journey/show #{}", fx.carrier.0),
    );
    assert!(
        crewed.contains("Cruising"),
        "the ship's own pilot: {crewed}"
    );
    let stranger = run_as(
        &mut tw,
        bystander,
        &format!("@journey/show #{}", fx.carrier.0),
    );
    assert_eq!(stranger, "No journey.");
    let absent = run_as(&mut tw, bystander, "@journey/show Nowhere-At-All");
    assert_eq!(stranger, absent, "rule 4: byte-identical");

    // **Amended by E7 residue pass 1, R6** (10b §15.1 item 10; cut 6 D5). Cut 6
    // recorded `@journey/depart` as staff-only *because crew authority was cut
    // 10's*, and cut 11 B0 landed the refusing half of it. The permitting half is
    // now here: the ship's own **billeted pilot** may depart it, and gets past the
    // authority gate to fail on the route spec instead. A **bystander** is still
    // refused with the byte-identical `Permission denied.`, and `/divert` is still
    // staff-only for everybody.
    let crewed = run_as(
        &mut tw,
        fx.pilot,
        &format!("@journey/depart #{}=x #{}:10", fx.carrier.0, fx.phobos.0),
    );
    assert!(
        !crewed.contains("Permission denied"),
        "the ship's own pilot is past the authority gate: {crewed}"
    );
    let refused = run_as(
        &mut tw,
        bystander,
        &format!("@journey/depart #{}=x #{}:10", fx.carrier.0, fx.phobos.0),
    );
    assert!(refused.contains("Permission denied"), "{refused}");
    let diverted = run_as(
        &mut tw,
        fx.pilot,
        &format!("@journey/divert #{}=x #{}:10", fx.carrier.0, fx.phobos.0),
    );
    assert!(
        diverted.contains("Permission denied"),
        "a divert re-writes a committed route and stays staff's: {diverted}"
    );
}

// ---- C10: an incoherent hydrated record is repaired ----------------------

/// **C10.** A stored record saying `Cruising` while the placement document says
/// the root is in a hangar survived boot intact and left a ship permanently
/// sealed and permanently un-movable, with no anomaly and no page.
#[test]
fn vf_c10_a_placed_cruising_root_is_repaired_at_boot() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    let mut kv = fold_kv(&mut tw, "sim");
    // Rewind ONLY the placement document to its pre-departure value, leaving a
    // `Cruising` journey against a placed root.
    *doc_mut(&mut kv, &format!("placement:{}", fx.carrier.0)) = serde_json::json!({
        "object": fx.carrier.0,
        "parent": { "kind": "Direct", "domain": 0, "pose": { "definition": mush_sim::passive_definition_ref(), "data": {} } },
        "revision": 1,
        "unplaced": null
    });

    let mut tw2 = boot();
    tw2.set_now(1_050);
    let fx2 = build(&mut tw2);
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    let placed = state(&tw2).is_placed(fx2.carrier);
    let stopped = book(&tw2)
        .get(fx2.carrier)
        .is_some_and(|j| !j.state().is_under_way());
    assert!(
        !placed || stopped,
        "boot left a placed root permanently sealed by a cruising record"
    );
}

// ---- R5/B3: the eight tactical slots ------------------------------------

/// Create `n` extra domains deterministically, so the reboot world matches.
fn extra_domains(tw: &mut TestWorld, n: usize) {
    for i in 0..n {
        let room = tw
            .world_mut()
            .create(ObjKind::Room, &format!("Rock{i}"), GOD, None)
            .unwrap();
        bind(tw, room, "sim.domain");
        ok(tw, &format!("@domain/create #{}=sim.passive", room.0));
    }
}

/// **R5.** Boot installs the activation book and now RE-EVALUATES the queue:
/// a slot freed out of band (an activation dropped, deleted or belonging to a
/// domain that is no longer active) must not leave a queued request stranded
/// until the next `@domain/promote|demote`.
#[test]
fn vf_r5_boot_re_evaluates_the_activation_queue() {
    let (mut tw, _fx) = scene();
    extra_domains(&mut tw, MAX_ACTIVE_DOMAINS + 1);
    for id in 0..MAX_ACTIVE_DOMAINS {
        ok(&mut tw, &format!("@domain/promote {id}=player"));
    }
    let over = u64::try_from(MAX_ACTIVE_DOMAINS).expect("a small limit");
    let queued = ok(&mut tw, &format!("@domain/promote {over}=interception"));
    assert!(queued.contains("QUEUED"), "{queued}");

    let mut kv = fold_kv(&mut tw, "sim");
    // A slot frees out of band: one live activation document is gone.
    kv.retain(|(k, _)| k.as_str() != "activation:0");

    let mut tw2 = boot();
    tw2.set_now(1_000);
    let _fx2 = build(&mut tw2);
    extra_domains(&mut tw2, MAX_ACTIVE_DOMAINS + 1);
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    assert!(
        activations(&tw2).is_promoted(mush_sim::DomainId(over)),
        "a slot is free ({} live of {MAX_ACTIVE_DOMAINS}) and a request is still queued",
        activations(&tw2).live()
    );
    assert_eq!(activations(&tw2).next_queued(), None);
}

/// **B3.** Retiring or pausing a promoted domain releases its tactical slot,
/// writing the summary FIRST — before the fold, eight retired domains starved
/// every live one and could not even be re-promoted to clear the leak.
#[test]
fn vf_b3_retire_and_pause_release_the_tactical_slot() {
    let (mut tw, _fx) = scene();
    ok(&mut tw, "@domain/promote 1=interception");
    assert_eq!(activations(&tw).live(), 1);
    let _ = tw.take_module_ops();
    ok(&mut tw, "@domain/retire 1");
    let ops = op_pairs(&mut tw);
    let set_summary = ops
        .iter()
        .position(|(k, key)| *k == "set" && key == "summary:1");
    let del_activation = ops
        .iter()
        .position(|(k, key)| *k == "del" && key == "activation:1");
    assert!(
        set_summary.is_some() && del_activation.is_some() && set_summary < del_activation,
        "{ops:?}"
    );
    assert_eq!(activations(&tw).live(), 0, "a retired domain kept its slot");

    // Pause does the same.
    ok(&mut tw, "@domain/promote 0=player");
    assert_eq!(activations(&tw).live(), 1);
    ok(&mut tw, "@domain/pause 0");
    assert_eq!(activations(&tw).live(), 0, "a paused domain kept its slot");

    // And a hydrated activation for a domain that is no longer active is
    // dropped rather than holding a slot across every reboot.
    ok(&mut tw, "@domain/resume 0");
    ok(&mut tw, "@domain/promote 0=player");
    let mut kv = fold_kv(&mut tw, "sim");
    for (key, value) in kv.iter_mut() {
        if key.as_str() == "domain:0" {
            value["status"] = serde_json::json!("Retired");
        }
    }
    let mut tw2 = boot();
    tw2.set_now(1_000);
    let _ = build(&mut tw2);
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    assert_eq!(activations(&tw2).live(), 0);
}

// ---- R6/R9/R10: the arrival's post-move steps ---------------------------

/// **R6.** A replayed arrival — a journey document older than its placement
/// document — used to skip the event, the narrative AND the onboard IC-floor
/// raise entirely, and nothing else ever raised it. The floor raise is
/// monotonic, so it belongs outside the `already` short-circuit.
#[test]
fn vf_r6_a_re_derived_arrival_still_raises_the_onboard_floors() {
    let mut tw = scenes_first();
    tw.set_now(1_000);
    let fx = build(&mut tw);
    let aboard = open_scene(&mut tw, fx.bay, 0, 1);
    depart(&mut tw, &fx);
    advance(&mut tw, 1_301);
    assert_eq!(scene_of(&tw, aboard).ic_floor, 1_300);

    let ops = tw.take_module_ops();
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    let mut scenes_kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in ops {
        let target = if op.ns == "sim" {
            &mut kv
        } else if op.ns == "scenes" {
            &mut scenes_kv
        } else {
            continue;
        };
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                target.retain(|(k, _)| *k != key);
                target.push((key, value));
            }
            ModuleWrite::Del { key } => target.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    // The crash: the placement landed; the journey doc and the scene doc did not.
    let doc = doc_mut(&mut kv, carrier_journey_key(fx.carrier).as_str());
    doc["cursor"] = serde_json::json!(1);
    doc["state"] = serde_json::json!({ "state": "Cruising" });
    doc["advanced_at"]["at"] = serde_json::json!(1_150);
    doc["applied_revision"] = serde_json::json!(0);
    for (_, v) in scenes_kv.iter_mut() {
        if v.get("ic_floor").is_some() {
            v["ic_floor"] = serde_json::json!(0);
        }
    }

    let mut tw2 = scenes_first();
    tw2.set_now(1_400);
    let fx2 = build(&mut tw2);
    let aboard2 = open_scene(&mut tw2, fx2.bay, 0, 1);
    tw2.modules_loaded(HashMap::from([
        (
            SmolStr::new("sim"),
            ModuleBootData {
                kv,
                ..ModuleBootData::default()
            },
        ),
        (
            SmolStr::new("scenes"),
            ModuleBootData {
                kv: scenes_kv,
                ..ModuleBootData::default()
            },
        ),
    ]));
    assert_eq!(*journey(&tw2, fx2.carrier).state(), JourneyState::Arrived);
    let floors: Vec<i64> = tw2
        .world()
        .module_state::<ScenesState>()
        .expect("scenes state")
        .scenes
        .values()
        .filter(|s| s.anchor == fx2.bay)
        .map(|s| s.ic_floor)
        .collect();
    assert!(
        floors.contains(&1_300),
        "`already` short-circuited before the onboard floor raise: {floors:?} (scene {aboard2:?})"
    );
}

/// **R10.** A repeated arrival raise at the identical stamp is a HIT, not an
/// operator page: the floor is monotonic, so `NotForward` with a floor already
/// at or past the stamp means the repair worked. Only `AheadOfWorld` and
/// `AlreadySealed` need a human.
#[test]
fn vf_r10_a_repeated_arrival_floor_raise_is_a_hit_not_a_page() {
    // The classification is a log level, so it is pinned at the source: the
    // `NotForward`-with-`floor >= at` arm must `continue` at `debug` before the
    // `error!` page, and the two arms that need a decision must still reach it.
    const SRC: &str = include_str!("../src/journey.rs");
    let hit = SRC
        .find("ArrivalFloorRefusal::NotForward { at: refused, floor }")
        .expect("the NotForward hit arm");
    let page = SRC
        .find("an onboard scene refused the arrival IC-floor raise")
        .expect("the page");
    assert!(hit < page, "the hit arm must be classified before the page");
    assert!(SRC[hit..page].contains("tracing::debug!"));
    assert!(SRC[hit..page].contains("continue;"));

    // And behaviourally: a re-derived arrival at the same stamp leaves the floor
    // exactly where it is and never holds the journey.
    let mut tw = scenes_first();
    tw.set_now(1_000);
    let fx = build(&mut tw);
    let aboard = open_scene(&mut tw, fx.bay, 0, 1);
    depart(&mut tw, &fx);
    advance(&mut tw, 1_301);
    assert_eq!(scene_of(&tw, aboard).ic_floor, 1_300);
    for t in 1..=3 {
        advance(&mut tw, 1_400 + t);
        assert_eq!(scene_of(&tw, aboard).ic_floor, 1_300);
    }
    assert_eq!(*journey(&tw, fx.carrier).state(), JourneyState::Arrived);
}

// ---- R8/R11/R13: the tick's budget, rotation and catch-up ----------------

/// **R8.** A terminal, coherent journey costs no `visited` budget, so a world
/// with a long passage history still works a live boundary on the FIRST fire
/// instead of `ceil(N/64)` seconds later.
#[test]
fn vf_r8_a_terminal_journey_stops_costing_the_budget() {
    let (mut tw, fx) = scene();
    let mut old: Vec<ObjId> = Vec::new();
    for i in 0..70 {
        let hauler = thing(&mut tw, &format!("Hauler{i}"));
        bind(&mut tw, hauler, "sim.participant");
        ok(&mut tw, &format!("@spatial/place/domain #{}=0", hauler.0));
        ok(
            &mut tw,
            &format!("@journey/depart #{}=hop #{}:10", hauler.0, fx.phobos.0),
        );
        old.push(hauler);
    }
    // Several fires to work them all off (16 per fire), then they are terminal.
    for t in 0..12 {
        advance(&mut tw, 1_011 + t);
    }
    for h in &old {
        assert_eq!(*journey(&tw, *h).state(), JourneyState::Arrived, "{h}");
    }

    // The newest root sorts last in the book, i.e. worst case for the rotation.
    let late = thing(&mut tw, "Latecomer");
    bind(&mut tw, late, "sim.participant");
    ok(&mut tw, &format!("@spatial/place/domain #{}=0", late.0));
    ok(
        &mut tw,
        &format!("@journey/depart #{}=hop #{}:10", late.0, fx.phobos.0),
    );
    assert!(book(&tw).journeys.len() > mush_sim::MAX_JOURNEYS_VISITED_PER_FIRE);
    advance(&mut tw, 1_040);
    assert_eq!(
        *journey(&tw, late).state(),
        JourneyState::Arrived,
        "a live boundary waited behind {} terminal records",
        old.len()
    );
}

/// **R11.** The rotation cursor rides the allocator document, so a restart
/// resumes the rotation instead of re-walking the lowest root ids and starving
/// the tail of the book after every reboot.
#[test]
fn vf_r11_the_rotation_cursor_survives_a_reboot() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    advance(&mut tw, 1_301);
    assert_eq!(*journey(&tw, fx.carrier).state(), JourneyState::Arrived);
    let cursor = book(&tw).cursor;
    assert_ne!(cursor, 0, "the rotation never moved");

    let kv = fold_kv(&mut tw, "sim");
    let meta = kv
        .iter()
        .find(|(k, _)| k.as_str() == "journey_meta")
        .map(|(_, v)| v.clone())
        .expect("the allocator document");
    assert_eq!(
        meta.get("rotation_cursor")
            .and_then(serde_json::Value::as_u64),
        Some(u64::from(cursor)),
        "the cursor is not carried in the document that already exists: {meta}"
    );

    let mut tw2 = boot();
    tw2.set_now(2_000);
    let _ = build(&mut tw2);
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    assert_eq!(
        book(&tw2).cursor,
        cursor,
        "the reboot restarted the rotation at the head of the book"
    );
}

/// **R12.** The module's own no-panic contract, stated as code: the persistence
/// helpers page instead of panicking, because they run in the world task from
/// the tick and from boot.
#[test]
fn vf_r12_the_persistence_helpers_do_not_panic() {
    let sources = [
        include_str!("../src/journey.rs"),
        include_str!("../src/domain.rs"),
    ]
    .join(
        "
",
    );
    for panicking in [
        "expect(\"carrier journey serializes\")",
        "expect(\"domain activation serializes\")",
        "expect(\"domain summary serializes\")",
    ] {
        assert!(
            !sources.contains(panicking),
            "a persistence helper still panics on a serialization failure: {panicking}"
        );
    }
    // And the pages exist in their place.
    assert!(sources.contains("would not serialize"));
}

/// **R13.** A long downtime is caught up ONE leg boundary at a time, each
/// observed exactly once, so a later cut can hang an interception or an
/// alignment-band advance on a boundary without the catch-up collapsing past it.
#[test]
fn vf_r13_a_downtime_observes_every_leg_boundary_once() {
    let (mut tw, fx) = scene();
    ok(
        &mut tw,
        &format!(
            "@journey/depart #{}=four #{}:10 #{}:10 #{}:10 #{}:10",
            fx.carrier.0, fx.field.0, fx.bay.0, fx.hold.0, fx.phobos.0
        ),
    );
    let _ = tw.take_module_ops();
    // One fire, well past the last boundary.
    advance(&mut tw, 1_100);
    let key = carrier_journey_key(fx.carrier).to_string();
    let writes = op_pairs(&mut tw)
        .into_iter()
        .filter(|(kind, k)| *kind == "set" && *k == key)
        .count();
    assert_eq!(
        writes, 4,
        "the four boundaries collapsed into {writes} advance(s)"
    );
    let arrived = journey(&tw, fx.carrier);
    assert_eq!(*arrived.state(), JourneyState::Arrived);
    assert_eq!(arrived.cursor(), 4);
}

// ---- B8: the journey census --------------------------------------------

/// **B8.** The `cjourney:` key space is bounded by a declared census, refused
/// at the producer rather than admitted and then truncated.
#[test]
fn vf_b8_the_journey_census_is_bounded() {
    let (mut tw, fx) = scene();
    {
        // Fill the book to the cap directly: the point under test is the bound
        // in `journey_ineligible`, not the authoring path.
        let route = {
            depart(&mut tw, &fx);
            let j = journey(&tw, fx.carrier);
            (
                j.route().clone(),
                j.destination().clone(),
                j.departed().clone(),
            )
        };
        let book = tw
            .world_mut()
            .module_state_mut::<JourneyBook>()
            .expect("journey book");
        for i in 0..mush_sim::MAX_CARRIER_JOURNEYS {
            let root = ObjId(60_000 + u32::try_from(i).expect("a small index"));
            let record = CarrierJourney::new(
                mush_sim::JourneyId(u64::try_from(i).expect("a small index")),
                root,
                route.0.clone(),
                route.1.clone(),
                route.2.clone(),
                route.2.clone(),
                0,
                JourneyState::Arrived,
                None,
            )
            .expect("a coherent journey");
            book.journeys.insert(root, record);
        }
    }
    let hauler = thing(&mut tw, "Overflow");
    bind(&mut tw, hauler, "sim.participant");
    ok(&mut tw, &format!("@spatial/place/domain #{}=0", hauler.0));
    let refused = err(
        &mut tw,
        &format!("@journey/depart #{}=hop #{}:10", hauler.0, fx.phobos.0),
    );
    assert!(
        refused.contains(&mush_sim::MAX_CARRIER_JOURNEYS.to_string()),
        "the census must name its cap: {refused}"
    );
    assert!(book(&tw).get(hauler).is_none());
}

// ---- The deviations, pinned so their inertness cannot drift --------------

/// **Deviation (b).** The alignment-band advance is structurally inert: nothing
/// authors a band, and `mush-sim` names `advance_cadence_manual` nowhere, so it
/// writes no economy document. Pinned so the inertness cannot drift silently.
#[test]
fn vf_db_the_alignment_band_is_inert() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    assert!(journey(&tw, fx.carrier).route().alignment_band().is_none());
    for src in [
        include_str!("../src/journey.rs"),
        include_str!("../src/commands.rs"),
        include_str!("../src/route.rs"),
        include_str!("../src/transition.rs"),
    ] {
        assert!(
            !src.contains("advance_cadence_manual"),
            "mush-sim now writes an economy cadence document"
        );
    }
}

/// **Deviation (c).** `@journey/plan` writes nothing — no document, and no
/// allocator bump either, so a future durable step in the dry run fails loudly.
#[test]
fn vf_dc_plan_bumps_no_allocator() {
    let (mut tw, fx) = scene();
    let before = book(&tw).next_id;
    let _ = fold_kv(&mut tw, "sim");
    ok(
        &mut tw,
        &format!(
            "@journey/plan #{}=corridor #{}:100 #{}:200",
            fx.carrier.0, fx.field.0, fx.phobos.0
        ),
    );
    assert_eq!(fold_kv(&mut tw, "sim"), Vec::new(), "the dry run wrote");
    assert_eq!(
        book(&tw).next_id,
        before,
        "the dry run bumped the allocator"
    );
}

/// **Deviation (d).** Leg boundaries are a pure function of the STORED
/// document, which is what makes the leg cursor an idempotency key a reboot can
/// re-derive identically. When a drive profile lands, its resolved seconds must
/// be frozen into the record at `/depart`, never looked up at tick time.
#[test]
fn vf_dd_boundaries_are_a_function_of_the_stored_document_only() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    let before = journey(&tw, fx.carrier);
    let kv = fold_kv(&mut tw, "sim");

    let mut tw2 = boot();
    tw2.set_now(1_050);
    let fx2 = build(&mut tw2);
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    let after = journey(&tw2, fx2.carrier);
    assert_eq!(after.leg_due_at(0), before.leg_due_at(0));
    assert_eq!(after.leg_due_at(1), before.leg_due_at(1));
    assert_eq!(after.arrival_at(), before.arrival_at());
}

/// **Deviation (e).** The order holds and a corrupt summary is dropped without
/// deleting its key: a demotion can never leave a domain demoted-without-summary.
#[test]
fn vf_de_the_summary_is_written_before_the_activation_delete() {
    let (mut tw, _fx) = scene();
    ok(&mut tw, "@domain/promote 0=player");
    let _ = tw.take_module_ops();
    advance(&mut tw, 1_030);
    ok(&mut tw, "@domain/demote 0");
    let ops = op_pairs(&mut tw);
    let set_summary = ops
        .iter()
        .position(|(k, key)| *k == "set" && key == "summary:0");
    let del_activation = ops
        .iter()
        .position(|(k, key)| *k == "del" && key == "activation:0");
    assert!(
        set_summary.is_some() && set_summary < del_activation,
        "{ops:?}"
    );

    // A corrupt summary document is dropped at boot, never deleted.
    let mut kv = fold_kv(&mut tw, "sim");
    for (key, value) in kv.iter_mut() {
        if key.as_str() == "summary:0" {
            value["contacts"] = serde_json::json!("not a number");
        }
    }
    let mut tw2 = boot();
    tw2.set_now(2_000);
    let _ = build(&mut tw2);
    let _ = tw2.take_module_ops();
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    assert!(
        !activations(&tw2)
            .summaries
            .contains_key(&mush_sim::DomainId(0))
    );
    let ops = op_pairs(&mut tw2);
    assert!(
        !ops.iter().any(|(k, key)| *k == "del" && key == "summary:0"),
        "a corrupt summary document was deleted: {ops:?}"
    );
}

/// **Deviation (h).** Once the arrival is confirmed the record is a PASSIVE
/// marker: three further fires write nothing at all, which is what makes it
/// safe for cut 7/8 to poll `JourneyBook` for a `sim.journey.arrived` fact
/// rather than re-deriving "arrived" from placement.
#[test]
fn vf_dh_an_arrived_journey_emits_no_further_sim_op() {
    let (mut tw, fx) = scene();
    depart(&mut tw, &fx);
    advance(&mut tw, 1_301);
    assert_eq!(*journey(&tw, fx.carrier).state(), JourneyState::Arrived);
    let _ = tw.take_module_ops();
    for t in 1..=3 {
        advance(&mut tw, 1_400 + t);
    }
    assert_eq!(
        op_pairs(&mut tw),
        Vec::new(),
        "an arrived journey kept writing"
    );
}

/// **B6, DISCHARGED by E7 cut 7.** Cut 6 shipped `@journey/depart`'s
/// `<route id>` as a LABEL: `build_route` fabricated a fresh `RouteDef` from the
/// command line at the `manual` drive class with zero declared propellant and no
/// alignment band, so `RouteDef::propellant()`, `alignment_band()` and
/// `route_duration` had no caller. The verify's own fix — "resolve the id
/// against cut 1's authored pack" — could not be an edge (`ruleset-tacsim`
/// depends on `mush-sim`, so the lookup is a cycle), so it needed a
/// composition-installed catalog in the `SimulationModule::with_accounts` shape.
/// That catalog is `mush_sim::RouteCatalog` /
/// `SimulationModule::with_route_catalog`, implemented over the tacsim store in
/// `mush_server::sim_routes`.
///
/// Both arms are pinned, because a resolution that swallowed the fallback would
/// break every hand-typed corridor the cut-6 tests fly:
/// - an **authored** id resolves to its declared legs, drive class, propellant
///   and alignment band, and the journey the world can actually be observed
///   flying is that route (the carrier's projected arrival is the authored
///   destination, not a typed one);
/// - a **hand-typed** id that names no authored route still builds the manual
///   fixture route, unchanged.
#[test]
fn vf_b6_an_authored_route_id_resolves_and_an_unknown_one_still_falls_back() {
    // --- the authored arm ---
    let cell: std::sync::Arc<std::sync::Mutex<Option<mush_sim::RouteDef>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));

    struct TestCatalog(std::sync::Arc<std::sync::Mutex<Option<mush_sim::RouteDef>>>);
    impl mush_sim::RouteCatalog for TestCatalog {
        fn resolve(
            &self,
            _world: &mush_core::World,
            id: &mush_core::CanonicalId,
        ) -> Option<mush_sim::RouteDef> {
            self.0.lock().ok()?.clone().filter(|route| route.id() == id)
        }
    }

    let mut tw = TestWorld::with_modules(vec![
        Box::new(
            SimulationModule::new().with_route_catalog(std::sync::Arc::new(TestCatalog(
                std::sync::Arc::clone(&cell),
            ))),
        ),
        Box::new(ScenesModule::new()),
    ]);
    tw.set_now(1_000);
    let fx = build(&mut tw);
    *cell.lock().unwrap() = Some(
        mush_sim::RouteDef::new(
            mush_core::CanonicalId::new("mars_corridor").unwrap(),
            (fx.field, fx.phobos),
            vec![mush_sim::LegDef::new(fx.phobos, 1).unwrap()],
            Some(mush_sim::CadenceId(7)),
            mush_core::CanonicalId::new("fusion_torch").unwrap(),
            900,
        )
        .unwrap(),
    );
    // No place tokens at all: the id alone must resolve.
    ok(
        &mut tw,
        &format!("@journey/depart #{}=mars_corridor", fx.carrier.0),
    );
    let route = journey(&tw, fx.carrier).route().clone();
    assert_eq!(route.propellant(), 900, "the authored propellant resolved");
    assert_eq!(
        route.alignment_band().copied(),
        Some(mush_sim::CadenceId(7))
    );
    assert_eq!(route.drive_class().as_str(), "fusion_torch");
    advance(&mut tw, 1_002);
    assert_eq!(*journey(&tw, fx.carrier).state(), JourneyState::Arrived);
    // The OBSERVABLE outcome, not the record: the carrier is projected at the
    // authored destination, and the destination room lists it.
    assert_eq!(
        mush_sim::graph::project(state(&tw), tw.world(), fx.carrier),
        Some(fx.phobos),
        "the authored route actually flew the carrier to its declared endpoint"
    );

    // --- the fallback arm, unchanged ---
    let (mut tw, fx) = scene();
    ok(
        &mut tw,
        &format!(
            "@journey/depart #{}=not_a_real_route_id #{}:1",
            fx.carrier.0, fx.phobos.0
        ),
    );
    let route = journey(&tw, fx.carrier).route().clone();
    assert_eq!(route.propellant(), 0, "no declared propellant to reserve");
    assert!(route.alignment_band().is_none());
    assert_eq!(route.drive_class().as_str(), "manual");
    advance(&mut tw, 1_002);
    assert_eq!(*journey(&tw, fx.carrier).state(), JourneyState::Arrived);
}

// ---- R9: the arrival event ----------------------------------------------

/// A module composed AFTER `mush-sim` — standing in for `mush-script`,
/// `mush-npc` and `mush-weather`, which all subscribe to `Moved` and are all
/// composed after it in `server_modules_with`. It records every `Moved` it sees
/// and whether its own `on_world_loaded` had run at the time.
#[derive(Default)]
struct LateProbeState {
    hydrated: std::sync::atomic::AtomicBool,
    before_hydration: std::sync::atomic::AtomicUsize,
    moves: std::sync::Mutex<Vec<(ObjId, Option<ObjId>, ObjId)>>,
}

struct LateModule(std::sync::Arc<LateProbeState>);

impl mush_core::GameModule for LateModule {
    fn name(&self) -> &'static str {
        "lateprobe"
    }
    fn register(&mut self, reg: &mut mush_core::Registrar<'_>) -> Result<(), mush_core::ModError> {
        let probe = std::sync::Arc::clone(&self.0);
        reg.subscribe(
            mush_core::WorldEventKind::Moved,
            std::sync::Arc::new(
                move |_ctx: &mut mush_core::Ctx<'_>, ev: &mush_core::WorldEvent| {
                    if let mush_core::WorldEvent::Moved { who, from, to } = ev {
                        if !probe.hydrated.load(std::sync::atomic::Ordering::SeqCst) {
                            probe
                                .before_hydration
                                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        }
                        if let Ok(mut moves) = probe.moves.lock() {
                            moves.push((*who, *from, *to));
                        }
                    }
                },
            ),
        );
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        _ctx: &mut mush_core::ModuleCtx<'_>,
        _data: ModuleBootData,
    ) -> Result<(), mush_core::ModError> {
        self.0
            .hydrated
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

fn with_late_probe(probe: &std::sync::Arc<LateProbeState>) -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(ScenesModule::new()),
        Box::new(SimulationModule::new()),
        Box::new(LateModule(std::sync::Arc::clone(probe))),
    ])
}

/// **R9(1).** `Ctx::emit_world_event` dispatches INLINE, and the modules that
/// subscribe to `Moved` are composed after `mush-sim`, so a boot-time arrival
/// used to wake observers whose own `on_world_loaded` had not run. The durable
/// truth is unchanged either way; only the audience is.
#[test]
fn vf_r9a_the_boot_arrival_emits_no_world_event() {
    let probe = std::sync::Arc::new(LateProbeState::default());
    let mut tw = with_late_probe(&probe);
    tw.set_now(1_000);
    let fx = build(&mut tw);
    depart(&mut tw, &fx);
    advance(&mut tw, 1_301);
    let mut kv = fold_kv(&mut tw, "sim");
    // The s3 crash gap: the journey says Arrived, the placement document does not.
    *doc_mut(&mut kv, &format!("placement:{}", fx.carrier.0)) = serde_json::json!({
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

    let probe2 = std::sync::Arc::new(LateProbeState::default());
    let mut tw2 = with_late_probe(&probe2);
    tw2.set_now(2_000);
    let fx2 = build(&mut tw2);
    probe2
        .hydrated
        .store(false, std::sync::atomic::Ordering::SeqCst);
    probe2
        .before_hydration
        .store(0, std::sync::atomic::Ordering::SeqCst);
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            ..ModuleBootData::default()
        },
    )]));
    assert!(
        matches!(parent(&tw2, fx2.carrier), SpatialParent::Direct { domain, .. } if domain.0 == 1),
        "the heal did not run"
    );
    assert_eq!(
        probe2
            .before_hydration
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the boot heal woke a module whose `on_world_loaded` had not yet run"
    );
}

/// **R9(2)/(3).** The arrival event names where the ship came FROM, and the
/// departure emits its own half — before the fold `from: None` left every
/// `Joined` without a matching `Left`, and a departure emitted nothing at all.
#[test]
fn vf_r9b_the_arrival_event_names_where_it_came_from() {
    let probe = std::sync::Arc::new(LateProbeState::default());
    let mut tw = with_late_probe(&probe);
    tw.set_now(1_000);
    let fx = build(&mut tw);
    if let Ok(mut moves) = probe.moves.lock() {
        moves.clear();
    }
    depart(&mut tw, &fx);
    advance(&mut tw, 1_301);

    let moves: Vec<(ObjId, Option<ObjId>, ObjId)> = probe
        .moves
        .lock()
        .map(|m| {
            m.iter()
                .filter(|(who, _, _)| *who == fx.carrier)
                .copied()
                .collect()
        })
        .unwrap_or_default();
    let transit = state(&tw).in_transit_room;
    assert_eq!(
        moves,
        vec![
            (fx.carrier, Some(fx.field), transit),
            (fx.carrier, Some(transit), fx.phobos),
        ],
        "the departure and the arrival must each name both ends exactly once"
    );
}

// ---- E7 cut 7 independent verify fold ----

/// A catalog holding one authored route, for the fold's own cases.
struct FoldCatalog(std::sync::Arc<std::sync::Mutex<Option<mush_sim::RouteDef>>>);
impl mush_sim::RouteCatalog for FoldCatalog {
    fn resolve(
        &self,
        _world: &mush_core::World,
        id: &mush_core::CanonicalId,
    ) -> Option<mush_sim::RouteDef> {
        self.0.lock().ok()?.clone().filter(|route| route.id() == id)
    }
}

/// A world whose catalog authors `mars_corridor` **from Phobos**, while the
/// carrier is standing in Field.
fn authored_elsewhere() -> (TestWorld, Fx) {
    let cell: std::sync::Arc<std::sync::Mutex<Option<mush_sim::RouteDef>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut tw = TestWorld::with_modules(vec![
        Box::new(
            SimulationModule::new().with_route_catalog(std::sync::Arc::new(FoldCatalog(
                std::sync::Arc::clone(&cell),
            ))),
        ),
        Box::new(ScenesModule::new()),
    ]);
    tw.set_now(1_000);
    let fx = build(&mut tw);
    *cell.lock().unwrap() = Some(
        mush_sim::RouteDef::new(
            mush_core::CanonicalId::new("mars_corridor").unwrap(),
            (fx.phobos, fx.field),
            vec![mush_sim::LegDef::new(fx.field, 1).unwrap()],
            Some(mush_sim::CadenceId(7)),
            mush_core::CanonicalId::new("fusion_torch").unwrap(),
            900,
        )
        .unwrap(),
    );
    (tw, fx)
}

/// **O5.** One extra place token cannot bypass the authored route.
///
/// Before the fold the catalog was consulted only when no place tokens
/// followed, so a trailing token skipped the authored arm entirely — including
/// its `from != origin` refusal — and fell into the manual parser, which minted
/// a `RouteDef` carrying the **authored id** from the wrong origin with zero
/// propellant, no alignment band and the manual drive class.
#[test]
fn vf_o5_one_extra_token_cannot_bypass_the_authored_route() {
    let (mut tw, fx) = authored_elsewhere();
    // The bare form refuses, because the route sets out from somewhere else.
    let refusal = err(
        &mut tw,
        &format!("@journey/depart #{}=mars_corridor", fx.carrier.0),
    );
    assert!(refusal.contains("sets out from"), "{refusal}");
    // …and so does the form with one extra place token, which used to succeed.
    let bypass = err(
        &mut tw,
        &format!(
            "@journey/depart #{}=mars_corridor #{}:1",
            fx.carrier.0, fx.phobos.0
        ),
    );
    assert!(
        bypass.contains("is authored; drop the place list"),
        "an authored id may not degrade to an unconstrained manual route: {bypass}"
    );
    // No journey was created under an id the carrier never flew.
    assert!(
        tw.world()
            .module_state::<mush_sim::JourneyBook>()
            .is_none_or(|book| !book.under_way(fx.carrier)),
        "the carrier did not depart on a bypassed authored route"
    );
}

/// **O12.** A crew route probe refuses identically for a real authored id and a
/// nonsense one, while staff still get the informative line (§4 rule 4).
#[test]
fn vf_o12_a_crew_route_probe_refuses_identically_for_a_real_and_a_nonsense_id() {
    let (mut tw, fx) = authored_elsewhere();
    let real = texts(&tw.run(
        fx.pilot,
        &format!("@journey/plan #{}=mars_corridor", fx.carrier.0),
    ));
    let nonsense = texts(&tw.run(
        fx.pilot,
        &format!("@journey/plan #{}=not_a_real_route_at_all", fx.carrier.0),
    ));
    assert_eq!(
        real, nonsense,
        "a crew member must not learn which route ids exist or where they begin"
    );
    assert!(real.contains(mush_sim::ROUTE_NOT_FROM_HERE), "{real}");
    assert!(!real.contains("Phobos"), "and it names no place: {real}");
    // Staff still get the informative form.
    let staff = err(
        &mut tw,
        &format!("@journey/plan #{}=mars_corridor", fx.carrier.0),
    );
    assert!(staff.contains("sets out from"), "{staff}");
}
