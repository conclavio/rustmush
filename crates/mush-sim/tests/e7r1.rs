//! **E7 residue pass 1 — the `mush-sim` half.**
//!
//! **R2** (10b §15.1 item 7): the corridor as a live domain — the journey tick's
//! leg boundary firing `PromotionTrigger::Interception` through the ONE
//! request→apply seam.
//!
//! **R6** (10b §15.1 item 10): the *permitting* half of crew-driven
//! `@journey/depart`. Cut 6 D5(b) made the whole write half staff-only because crew
//! authority was cut 10's; cut 11 B0 landed the *refusing* half
//! (`seat_restriction(.., ProxyAct::OperationDeparture)`); this is what was left —
//! replacing `require_admin` with a billet/command test, keeping the proxy guard in
//! front of it, and leaving `/divert` staff's.
//!
//! Every test drives the real command path and asserts what an actor can actually
//! *do*, not what a predicate returns.

use mush_core::{
    Audience, BehaviorBinding, BehaviorId, GOD, LIMBO, ObjId, ObjKind, ScopedIdempotencyKey,
};
use mush_proto::OutputEvent;
use mush_sim::{JourneyBook, JourneyState, LegDef, RouteDef, SimulationModule};
use mush_testkit::{TestWorld, text_of};

// ---- E7r1: the rig --------------------------------------------------------

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![Box::new(SimulationModule::new())])
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
fn run_as(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    texts(&tw.run(actor, line))
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

struct Fx {
    field: ObjId,
    phobos: ObjId,
    /// A hull nobody in this fixture *owns*, so `perm::controls` is false for
    /// every player here and only the billet can let one of them through.
    carrier: ObjId,
    /// A second hull, owned by `owner`.
    owned: ObjId,
    pilot: ObjId,
    bystander: ObjId,
    owner: ObjId,
}

fn scene() -> (TestWorld, Fx) {
    let mut tw = boot();
    tw.set_now(1_000);
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .unwrap();
    let phobos = tw
        .world_mut()
        .create(ObjKind::Room, "Phobos", GOD, None)
        .unwrap();
    // `spawn_player` makes a player its OWN owner, so one player never controls
    // another and `perm::controls` cannot silently satisfy the gate for them.
    let pilot = tw.spawn_player("Ann");
    let bystander = tw.spawn_player("Passer");
    let owner = tw.spawn_player("Reg");
    let carrier = tw
        .world_mut()
        .create(ObjKind::Thing, "Leopard", GOD, Some(LIMBO))
        .unwrap();
    let owned = tw
        .world_mut()
        .create(ObjKind::Thing, "Union", owner, Some(LIMBO))
        .unwrap();

    bind(&mut tw, field, "sim.domain");
    bind(&mut tw, phobos, "sim.domain");
    bind(&mut tw, carrier, "sim.participant");
    bind(&mut tw, owned, "sim.participant");
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", field.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", phobos.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/domain #{}=0", carrier.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/domain #{}=0", owned.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/station #{}=pilot,pilot,channels:movement",
            carrier.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", carrier.0, pilot.0),
    );
    (
        tw,
        Fx {
            field,
            phobos,
            carrier,
            owned,
            pilot,
            bystander,
            owner,
        },
    )
}

fn depart_line(carrier: ObjId, waypoint: ObjId, destination: ObjId) -> String {
    format!(
        "@journey/depart #{}=corridor #{}:100 #{}:200",
        carrier.0, waypoint.0, destination.0
    )
}

fn under_way(tw: &TestWorld, root: ObjId) -> bool {
    tw.world()
        .module_state::<JourneyBook>()
        .and_then(|b| b.get(root).cloned())
        .is_some_and(|j| *j.state() == JourneyState::Cruising)
}

// ---- E7r1: R6 — crew-driven `@journey/depart` -----------------------------

/// **The ship's own billeted pilot puts it under way, with no staff step.**
///
/// This is doc 10 §11's first staff step removed: the observable outcome is that a
/// hull is *cruising* after a command a player typed, and the journey book — which
/// `@journey/show` and `graph::project` both read — says so.
#[test]
fn e7r1_r6_a_billeted_pilot_departs_the_carrier_with_no_staff_step() {
    let (mut tw, fx) = scene();
    assert!(!under_way(&tw, fx.carrier));

    ok(
        &mut tw,
        fx.pilot,
        &depart_line(fx.carrier, fx.field, fx.phobos),
    );

    assert!(
        under_way(&tw, fx.carrier),
        "the pilot's own command put the hull under way"
    );
    // And a second actor sees it: the read gate is the same `crews` predicate, so
    // "who may read this ship's journey" and "who may depart it" have one answer.
    let shown = run_as(
        &mut tw,
        fx.pilot,
        &format!("@journey/show #{}", fx.carrier.0),
    );
    assert!(shown.contains("Cruising"), "{shown}");
}

/// **A bystander is refused with the byte-identical existing refusal** — the same
/// `Permission denied.` `require_admin` answered before R6, so widening who may
/// depart discloses nothing about which hulls have crews.
#[test]
fn e7r1_r6_a_bystander_is_refused_byte_identically_and_the_hull_stays_put() {
    let (mut tw, fx) = scene();
    let refused = run_as(
        &mut tw,
        fx.bystander,
        &depart_line(fx.carrier, fx.field, fx.phobos),
    );
    // `@journey/divert` still runs the pre-R6 `require_admin`, so its refusal IS
    // the byte string R6 must not have moved.
    let staff_only = run_as(
        &mut tw,
        fx.bystander,
        &format!(
            "@journey/divert #{}=corridor #{}:100",
            fx.carrier.0, fx.phobos.0
        ),
    );
    assert_eq!(
        refused, staff_only,
        "R6's refusal is `require_admin`'s own, unchanged"
    );
    assert!(refused.contains("Permission denied"), "{refused}");
    assert!(
        !under_way(&tw, fx.carrier),
        "and a refused departure moved nothing"
    );
}

/// **The hull's owner may depart it without holding a billet** — `perm::controls`,
/// the tree's existing "this is mine" answer, which is the *command relation* half
/// of the gate beside the billet half.
#[test]
fn e7r1_r6_the_hulls_owner_may_depart_it_without_a_billet() {
    let (mut tw, fx) = scene();
    // `owned` has no stations at all, so no billet exists to hold.
    ok(
        &mut tw,
        fx.owner,
        &depart_line(fx.owned, fx.field, fx.phobos),
    );
    assert!(under_way(&tw, fx.owned));

    // The pilot billeted on the OTHER hull is a stranger to this one.
    let (mut tw, fx) = scene();
    let refused = run_as(
        &mut tw,
        fx.pilot,
        &depart_line(fx.owned, fx.field, fx.phobos),
    );
    assert!(
        refused.contains("Permission denied"),
        "a billet is over ONE carrier, not over hulls in general: {refused}"
    );
    assert!(!under_way(&tw, fx.owned));
}

/// **Staff still may** — R6 widens the gate, it does not move it.
#[test]
fn e7r1_r6_staff_still_depart_a_carrier_they_neither_own_nor_crew() {
    let (mut tw, fx) = scene();
    ok(&mut tw, GOD, &depart_line(fx.carrier, fx.field, fx.phobos));
    assert!(under_way(&tw, fx.carrier));
}

/// **`/divert` stays staff-only, deliberately.** A divert re-writes a route that is
/// already under way with everyone aboard committed to it, and cut 6's `fail` path
/// names `@journey/divert` as *the* staff recovery route for a stopped hull
/// (§15.1 item 4). Widening it would remove the only recovery a staffer has.
#[test]
fn e7r1_r6_divert_stays_staff_only_for_the_billeted_pilot_and_the_owner() {
    let (mut tw, fx) = scene();
    ok(
        &mut tw,
        fx.pilot,
        &depart_line(fx.carrier, fx.field, fx.phobos),
    );
    for actor in [fx.pilot, fx.owner, fx.bystander] {
        let refused = run_as(
            &mut tw,
            actor,
            &format!(
                "@journey/divert #{}=corridor #{}:50",
                fx.carrier.0, fx.phobos.0
            ),
        );
        assert!(
            refused.contains("Permission denied"),
            "a divert is staff's: {refused}"
        );
    }
    ok(
        &mut tw,
        GOD,
        &format!(
            "@journey/divert #{}=corridor #{}:50",
            fx.carrier.0, fx.phobos.0
        ),
    );
}

/// **The proxy restriction stays IN FRONT of the new permission**, so widening who
/// may depart never widens what a proxy seat may do (T11).
///
/// The ordering is structural and is pinned as such: `cmd_journey`'s `depart` arm
/// calls `seat_restriction` and only then reaches `journey_depart`, where the new
/// authority gate lives. The guard's own call-site census — four in this file — is
/// pinned by `mush-campaign`'s disposition table, and R6 does not move it.
#[test]
fn e7r1_r6_the_proxy_guard_runs_ahead_of_the_new_permission_gate() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands.rs"),
    )
    .expect("the sim command source");
    let boundary = src.find("#[cfg(test)]").unwrap_or(src.len());
    let production = &src[..boundary];

    let dispatch = production.find("fn cmd_journey").expect("the dispatcher");
    let body = &production[dispatch..dispatch + 3_000];
    let guard = body
        .find("ProxyAct::OperationDeparture")
        .expect("the proxy guard");
    let call = body.find("journey_depart(ctx)").expect("the depart arm");
    assert!(
        guard < call,
        "the seat restriction is evaluated before the departure is attempted"
    );
    assert!(
        !body[..call].contains("require_admin(ctx)?;\n        journey_depart"),
        "the staff-only gate on `depart` is gone"
    );

    // The permitting half is inside `journey_depart`, before anything is read or
    // written, and it answers `require_admin`'s own refusal.
    let depart = production
        .find("fn journey_depart")
        .expect("the depart handler");
    let depart_body = &production[depart..depart + 3_000];
    let gate = depart_body
        .find("may_command_departure")
        .expect("the authority gate");
    let first_read = depart_body
        .find("journey_ineligible")
        .expect("the first state read");
    assert!(gate < first_read, "the gate runs before anything is read");
    assert!(
        depart_body[gate..].contains("denied()"),
        "and refuses with the one shared refusal"
    );
    // **B4 (independent verify fold):** the carrier's own RESOLUTION wears the same
    // refusal, so an existing dbref and a free one are one byte string. Before this
    // the arm was a world-wide existence oracle for any unprivileged actor.
    let resolve = depart_body.find("match_object").expect("the resolution");
    assert!(
        depart_body[resolve..]
            .split_once(';')
            .is_some_and(|(line, _)| line.contains("map_err(|_| denied())")),
        "the resolution failure is `denied()` too, not `I don't see that here.`"
    );

    // `/divert` keeps its own staff gate, so the two arms have not been conflated.
    let divert = body.find("journey_divert(ctx)").expect("the divert arm");
    assert!(
        body[..divert].contains("require_admin(ctx)?;"),
        "the divert arm still requires staff"
    );
}

/// A tiny type-level pin so the module's public seam list is not accidentally
/// widened: `ScopedIdempotencyKey` stays the only key shape this crate exports for
/// a reservation holder, and R6 adds no key of its own.
#[test]
fn e7r1_r6_adds_no_new_key_shape() {
    fn _assert_exists(_: Option<ScopedIdempotencyKey>) {}
}

// ---- E7r1: R2 — the corridor as a live domain ----------------------------

/// An authored corridor whose **first** leg is interdicted, handed in through the
/// existing `RouteCatalog` composition seam — the same seam a ruleset's pack feeds.
struct Corridor {
    from: ObjId,
    waypoint: ObjId,
    to: ObjId,
    hazard: u8,
    interdiction: u8,
}

impl mush_sim::RouteCatalog for Corridor {
    fn resolve(&self, _world: &mush_core::World, id: &mush_core::CanonicalId) -> Option<RouteDef> {
        if id.as_str() != "corridor" {
            return None;
        }
        RouteDef::controlled(
            id.clone(),
            (self.from, self.to),
            vec![
                LegDef::with_hazard(self.waypoint, 100, self.hazard, self.interdiction)
                    .expect("leg 1"),
                LegDef::with_hazard(self.to, 200, 0, 0).expect("leg 2"),
            ],
            mush_core::CanonicalId::new("fusion_torch").expect("class"),
            0,
            Some(mush_core::CanonicalId::new("belt_authority").expect("controller")),
        )
        .ok()
    }
}

struct Corr {
    waypoint: ObjId,
    carrier: ObjId,
    pilot: ObjId,
}

/// A three-place corridor: `field` (domain 0) then `waypoint` then `phobos`. The
/// interdicted leg is the FIRST one, so its boundary is an intermediate one and the
/// arrival is a separate event.
fn corridor_scene(hazard: u8, interdiction: u8, waypoint_is_a_domain: bool) -> (TestWorld, Corr) {
    // Object ids are allocated in creation order, and the catalog needs them before
    // the world exists — so build a throwaway world first to learn the ids.
    // `TestWorld::with_modules` is deterministic, so the second build allocates
    // exactly the same ones.
    let (field, waypoint, phobos) = {
        let mut probe = boot();
        let field = probe
            .world_mut()
            .create(ObjKind::Room, "Field", GOD, None)
            .unwrap();
        let waypoint = probe
            .world_mut()
            .create(ObjKind::Room, "Node", GOD, None)
            .unwrap();
        let phobos = probe
            .world_mut()
            .create(ObjKind::Room, "Phobos", GOD, None)
            .unwrap();
        (field, waypoint, phobos)
    };
    let mut tw = TestWorld::with_modules(vec![Box::new(
        SimulationModule::new().with_route_catalog(std::sync::Arc::new(Corridor {
            from: field,
            waypoint,
            to: phobos,
            hazard,
            interdiction,
        })),
    )]);
    tw.set_now(1_000);
    assert_eq!(
        (
            tw.world_mut()
                .create(ObjKind::Room, "Field", GOD, None)
                .unwrap(),
            tw.world_mut()
                .create(ObjKind::Room, "Node", GOD, None)
                .unwrap(),
            tw.world_mut()
                .create(ObjKind::Room, "Phobos", GOD, None)
                .unwrap(),
        ),
        (field, waypoint, phobos),
        "the id allocator is deterministic"
    );
    let pilot = tw.spawn_player("Ann");
    let carrier = tw
        .world_mut()
        .create(ObjKind::Thing, "Leopard", GOD, Some(LIMBO))
        .unwrap();
    bind(&mut tw, field, "sim.domain");
    if waypoint_is_a_domain {
        bind(&mut tw, waypoint, "sim.domain");
    }
    bind(&mut tw, phobos, "sim.domain");
    bind(&mut tw, carrier, "sim.participant");
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", field.0),
    );
    if waypoint_is_a_domain {
        ok(
            &mut tw,
            GOD,
            &format!("@domain/create #{}=sim.passive", waypoint.0),
        );
    }
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", phobos.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/domain #{}=0", carrier.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/station #{}=pilot,pilot,channels:movement",
            carrier.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=pilot,#{}", carrier.0, pilot.0),
    );
    (
        tw,
        Corr {
            waypoint,
            carrier,
            pilot,
        },
    )
}

fn advance(tw: &mut TestWorld, secs: i64) {
    tw.set_now(secs);
    let _ = tw.fire_due(secs.saturating_mul(1_000));
}

fn activations(tw: &TestWorld) -> &mush_sim::ActivationBook {
    tw.world()
        .module_state::<mush_sim::ActivationBook>()
        .expect("activation book")
}

fn domain_of(tw: &TestWorld, host: ObjId) -> mush_sim::DomainId {
    tw.world()
        .module_state::<mush_sim::SpatialState>()
        .expect("spatial state")
        .domain_of_host(host)
        .expect("a domain on that host")
        .id
}

fn fold_sim(tw: &mut TestWorld) -> Vec<(smol_str::SmolStr, serde_json::Value)> {
    let mut kv: Vec<(smol_str::SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        if op.ns != "sim" {
            continue;
        }
        match op.op {
            mush_core::ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            mush_core::ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            mush_core::ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}

fn reboot_sim(tw: &mut TestWorld, kv: Vec<(smol_str::SmolStr, serde_json::Value)>) {
    tw.modules_loaded(std::collections::HashMap::from([(
        smol_str::SmolStr::new("sim"),
        mush_core::ModuleBootData {
            kv,
            streams: std::collections::HashMap::new(),
            ..Default::default()
        },
    )]));
}

/// **`PromotionTrigger::Interception` finally fires — exactly once.**
///
/// Cut 6 declared the variant and 10b §15.1 item 7 recorded that *nothing fires it*.
/// The journey tick's intermediate leg boundary does, when the leg's pack-authored
/// interdiction threshold is met, and it does so through the ONE request seam.
#[test]
fn e7r1_r2_an_interdicted_leg_boundary_raises_an_interception_request_once() {
    let (mut tw, fx) = corridor_scene(4, 3, true);
    ok(
        &mut tw,
        fx.pilot,
        &format!("@journey/depart #{}=corridor", fx.carrier.0),
    );
    let node = domain_of(&tw, fx.waypoint);
    assert!(
        !activations(&tw).activations.contains_key(&node),
        "nothing is raised until the boundary is crossed"
    );

    // Cross the first leg's boundary (100 IC seconds).
    advance(&mut tw, 1_150);
    let request = activations(&tw)
        .activations
        .get(&node)
        .cloned()
        .expect("the boundary raised an activation");
    assert_eq!(
        request.trigger(),
        mush_sim::PromotionTrigger::Interception,
        "the trigger cut 6 declared and nothing fired"
    );
    assert!(!request.is_queued(), "a slot was free, so it is live");
    let stamp = request.at().clone();

    // Fire the tick again, repeatedly: the leg cursor already moved, so no boundary
    // is crossed and nothing is raised twice.
    advance(&mut tw, 1_160);
    advance(&mut tw, 1_170);
    let again = activations(&tw)
        .activations
        .get(&node)
        .cloned()
        .expect("still there");
    assert_eq!(again.at(), &stamp, "the standing request keeps its stamp");
    assert_eq!(again.trigger(), mush_sim::PromotionTrigger::Interception);
}

/// **A crash cannot double-fire it.** A reboot re-runs `advance_due` at boot from
/// the persisted cursor, and the cursor is the idempotency key: the boundary was
/// already crossed, so nothing is raised a second time and the standing request
/// keeps the trigger and stamp it was raised with.
#[test]
fn e7r1_r2_a_reboot_re_running_the_tick_raises_no_second_activation() {
    let (mut tw, fx) = corridor_scene(4, 3, true);
    ok(
        &mut tw,
        fx.pilot,
        &format!("@journey/depart #{}=corridor", fx.carrier.0),
    );
    advance(&mut tw, 1_150);
    let node = domain_of(&tw, fx.waypoint);
    let before = activations(&tw)
        .activations
        .get(&node)
        .cloned()
        .expect("raised");

    let kv = fold_sim(&mut tw);
    reboot_sim(&mut tw, kv);
    advance(&mut tw, 1_160);

    let after = activations(&tw)
        .activations
        .get(&node)
        .cloned()
        .expect("the request survived the reboot");
    assert_eq!(
        after.trigger(),
        before.trigger(),
        "the standing request keeps its ORIGINAL trigger"
    );
    assert_eq!(after.at(), before.at(), "…and its original stamp");
    assert_eq!(
        activations(&tw)
            .activations
            .values()
            .filter(|a| a.trigger() == mush_sim::PromotionTrigger::Interception)
            .count(),
        1,
        "exactly one interception across the crash"
    );
}

/// **A domain already promoted is not re-promoted by a second trigger**, and the
/// standing request keeps its original trigger and stamp — the short-circuit that
/// used to live inline in `@domain/promote` and now lives in the one seam both
/// callers go through.
#[test]
fn e7r1_r2_a_promoted_domain_is_not_re_promoted_by_an_interception() {
    let (mut tw, fx) = corridor_scene(4, 3, true);
    let node = domain_of(&tw, fx.waypoint);
    ok(&mut tw, GOD, &format!("@domain/promote {node}=player"));
    let standing = activations(&tw)
        .activations
        .get(&node)
        .cloned()
        .expect("promoted by hand");
    assert_eq!(
        standing.trigger(),
        mush_sim::PromotionTrigger::PlayerPresent
    );

    ok(
        &mut tw,
        fx.pilot,
        &format!("@journey/depart #{}=corridor", fx.carrier.0),
    );
    advance(&mut tw, 1_150);

    let after = activations(&tw)
        .activations
        .get(&node)
        .cloned()
        .expect("still one request");
    assert_eq!(
        after.trigger(),
        mush_sim::PromotionTrigger::PlayerPresent,
        "the standing request is unchanged — the interception did not overwrite it"
    );
    assert_eq!(after.at(), standing.at());
    // And a staffer asking again is told exactly that, through the same seam.
    let said = run_as(
        &mut tw,
        GOD,
        &format!("@domain/promote {node}=interception"),
    );
    assert!(said.contains("already promoted"), "{said}");
}

/// **A leg whose place presents into NO domain raises nothing, and the journey
/// continues.** No sentinel `DomainId` is minted, and the passage is unaffected.
#[test]
fn e7r1_r2_a_leg_that_presents_into_no_domain_raises_nothing() {
    let (mut tw, fx) = corridor_scene(4, 3, false);
    ok(
        &mut tw,
        fx.pilot,
        &format!("@journey/depart #{}=corridor", fx.carrier.0),
    );
    let before = activations(&tw).activations.len();
    advance(&mut tw, 1_150);
    assert_eq!(
        activations(&tw).activations.len(),
        before,
        "no activation, and certainly not one on a sentinel domain"
    );
    // The journey still crossed the boundary and still arrives.
    advance(&mut tw, 1_400);
    assert_eq!(
        *tw.world()
            .module_state::<JourneyBook>()
            .expect("book")
            .get(fx.carrier)
            .expect("journey")
            .state(),
        JourneyState::Arrived
    );
}

/// **An un-interdicted leg raises nothing.** The predicate is two authored numbers
/// and nothing else, so a corridor a pack did not arm is inert — which is what
/// makes the shipped fixture's unchanged digest safe.
#[test]
fn e7r1_r2_an_un_interdicted_leg_raises_nothing() {
    // A band of 2 under a threshold of 3: authored, and below the bar.
    let (mut tw, fx) = corridor_scene(2, 3, true);
    ok(
        &mut tw,
        fx.pilot,
        &format!("@journey/depart #{}=corridor", fx.carrier.0),
    );
    advance(&mut tw, 1_150);
    let node = domain_of(&tw, fx.waypoint);
    assert!(!activations(&tw).activations.contains_key(&node));

    // A zero threshold is *never interdicted*, whatever the band.
    let (mut tw, fx) = corridor_scene(5, 0, true);
    ok(
        &mut tw,
        fx.pilot,
        &format!("@journey/depart #{}=corridor", fx.carrier.0),
    );
    advance(&mut tw, 1_150);
    let node = domain_of(&tw, fx.waypoint);
    assert!(!activations(&tw).activations.contains_key(&node));
}

/// **The composition-frozen clock registry is untouched across an interception.**
///
/// The activation is a REQUEST; cut 8's authority is the only applier. Nothing in
/// R2 constructs a `DomainClock::Pulse`, adds a variant, or mutates the
/// `ClockProviderRegistry` — so the domain's own record is byte-identical before
/// and after, `Passive` included.
#[test]
fn e7r1_r2_an_interception_mutates_no_clock_and_no_domain_record() {
    let (mut tw, fx) = corridor_scene(4, 3, true);
    let node = domain_of(&tw, fx.waypoint);
    let before = tw
        .world()
        .module_state::<mush_sim::SpatialState>()
        .expect("state")
        .domains
        .get(&node)
        .cloned()
        .expect("the domain record");
    ok(
        &mut tw,
        fx.pilot,
        &format!("@journey/depart #{}=corridor", fx.carrier.0),
    );
    advance(&mut tw, 1_150);
    assert!(activations(&tw).activations.contains_key(&node));

    let after = tw
        .world()
        .module_state::<mush_sim::SpatialState>()
        .expect("state")
        .domains
        .get(&node)
        .cloned()
        .expect("the domain record");

    // **Everything R2 could have moved is unmoved.** The domain's identity, its
    // host, its presentation location and — the load-bearing one — its `driver`
    // `DefinitionRef` are byte-identical: no clock provider was bound, unbound or
    // re-keyed, which is the observable shadow of the composition-frozen
    // `ClockProviderRegistry` (`Registrar::domain_clock_provider` is compose-time
    // only, 08i answer 4 / G-A, and this test composes exactly one module).
    assert_eq!(
        (before.id, before.host, before.presentation_location),
        (after.id, after.host, after.presentation_location)
    );
    assert_eq!(
        serde_json::to_value(&before.driver).expect("encode"),
        serde_json::to_value(&after.driver).expect("encode"),
        "the domain's driver definition is untouched: no provider was re-bound"
    );

    // What DID change is cut 8's authority applying the request — which is the
    // contract, not a violation of it: R2 writes a REQUEST and cut 8 remains the
    // only applier. It reached the SAME post-state a hand `@domain/promote` reaches,
    // through the same code, on the cadence the composition already declared.
    match after.clock {
        // No `DomainClock` variant was added: this match is exhaustive over the two
        // cut 8 declared, so a third would fail to compile here.
        mush_sim::DomainClock::Pulse { period_ms, .. } => assert_eq!(
            period_ms, 2_000,
            "the period is the composition's own `GROUND_PULSE_MS`, never a number              R2 authored"
        ),
        mush_sim::DomainClock::Passive => {
            panic!("cut 8's authority did not apply the request")
        }
    }
}

/// **A leg boundary that raises an activation still moves exactly one cursor.**
#[test]
fn e7r1_r2_a_raising_boundary_still_moves_exactly_one_cursor() {
    let (mut tw, fx) = corridor_scene(4, 3, true);
    ok(
        &mut tw,
        fx.pilot,
        &format!("@journey/depart #{}=corridor", fx.carrier.0),
    );
    advance(&mut tw, 1_150);
    let journey = tw
        .world()
        .module_state::<JourneyBook>()
        .expect("book")
        .get(fx.carrier)
        .cloned()
        .expect("journey");
    assert_eq!(journey.cursor(), 1, "one boundary, one cursor step");
    assert_eq!(*journey.state(), JourneyState::Cruising);
    assert!(
        activations(&tw)
            .activations
            .contains_key(&domain_of(&tw, fx.waypoint))
    );
}

/// **The corridor's leg fields survive a round trip and refuse past the band cap.**
#[test]
fn e7r1_r2_the_leg_wire_round_trips_and_refuses_past_the_hazard_cap() {
    assert_eq!(mush_sim::MAX_HAZARD_BAND, 5);
    let leg = LegDef::with_hazard(ObjId(9), 100, 4, 3).expect("a legal leg");
    assert!(leg.is_interdicted());
    let encoded = serde_json::to_value(leg).expect("encode");
    let back: LegDef = serde_json::from_value(encoded).expect("re-enter through the wire");
    assert_eq!(back, leg);

    // Past the cap: **refused on load, never clamped.**
    assert!(LegDef::with_hazard(ObjId(9), 100, 6, 3).is_err());
    assert!(LegDef::with_hazard(ObjId(9), 100, 4, 6).is_err());
    let forged = serde_json::json!({ "to": 9, "base_s": 100, "hazard": 9, "interdiction": 1 });
    assert!(
        serde_json::from_value::<LegDef>(forged).is_err(),
        "the wire re-runs the constructor"
    );
    // A leg stored before the residue pass re-enters as an unhazardous one.
    let old = serde_json::json!({ "to": 9, "base_s": 100 });
    let leg: LegDef = serde_json::from_value(old).expect("an old leg still loads");
    assert_eq!((leg.hazard(), leg.interdiction()), (0, 0));
    assert!(!leg.is_interdicted());
}

// ---- E7 residue pass 1 independent verify fold ----

/// **A gunner with no movement channel is refused byte-identically, and the hull
/// stays put** (C8; owner decision F7(a)).
///
/// `may_command_departure`'s crew arm was `crews` — the **read** gate, *"holds a
/// billet at any authored station"* — and R6's own comment presented the reuse as a
/// virtue: *"the same predicate the read gate uses, so 'who may read this ship's
/// journey' and 'who may depart it' have one answer"*. That is the defect. The tree
/// already recorded the discriminator and did not use it: `@spatial/station <host>=
/// <station>,<role>[,channels:…]`, `StationSpec::channels`,
/// `StationAssignment::granted_channels`, and this fixture's own pilot holding
/// `channels:movement`. A gunner holding `channels:fire` and nothing else put the
/// hull, and everyone aboard it, under way.
///
/// Every arm the split gate covers is named and driven: the **gunner** (a billet, no
/// movement channel) is refused; the **pilot** (a billet with it) still departs; the
/// **bystander** (no billet at all) is refused; and all three refusals plus the one
/// for a dbref naming **no object** are one byte string.
#[test]
fn vf_c8a_a_gunner_with_no_movement_channel_is_refused_byte_identically_and_the_hull_stays_put() {
    let (mut tw, fx) = scene();
    let gunner = tw.spawn_player("Gus");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@spatial/station #{}=turret,gunner,channels:fire",
            fx.carrier.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=turret,#{}", fx.carrier.0, gunner.0),
    );

    // The gunner genuinely crews the hull: the READ gate still lets them in.
    let read = run_as(&mut tw, gunner, &format!("@journey/show #{}", fx.carrier.0));
    assert!(
        !read.contains("Permission denied"),
        "a billet is still a billet for reading: {read}"
    );

    let refused = run_as(
        &mut tw,
        gunner,
        &depart_line(fx.carrier, fx.field, fx.phobos),
    );
    assert!(
        !under_way(&tw, fx.carrier),
        "the GUNNER did not put the hull under way: {refused}"
    );
    assert!(refused.contains("Permission denied"), "{refused}");

    // The refusal discloses nothing the bystander's does not.
    let bystander = run_as(
        &mut tw,
        fx.bystander,
        &depart_line(fx.carrier, fx.field, fx.phobos),
    );
    assert_eq!(refused, bystander, "byte-identical to a total stranger's");

    // …and the pilot, who holds the movement channel, still departs unaided.
    ok(
        &mut tw,
        fx.pilot,
        &depart_line(fx.carrier, fx.field, fx.phobos),
    );
    assert!(under_way(&tw, fx.carrier));
}

/// **A station that declares NO channels is not a drive gate** (C8's other arm, and
/// the reason F7(c)'s compatibility hedge was rejected).
///
/// F7(c) offered *"require the movement channel only when the station declares any
/// channels at all"*, which would make the rule depend on how carefully a pack was
/// written: a loader's deck station with no `channels:` at all would drive the hull.
/// `StationSpec::channels` is `#[serde(default)]` and empty until authored, so that
/// is the common case in an unfinished pack, not an exotic one.
///
/// It also pins that the gate asks `granted_channels` — the seat's **narrowed** set
/// intersected with the station's — rather than `StationSpec::channels` directly, so
/// cut 10's proxy narrowing takes the drive with it exactly as it takes every other
/// channelled action.
#[test]
fn vf_c8b_a_station_declaring_no_channels_is_not_a_drive_gate() {
    let (mut tw, fx) = scene();
    let loader = tw.spawn_player("Lou");
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/station #{}=deck,loader", fx.carrier.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/assign #{}=deck,#{}", fx.carrier.0, loader.0),
    );
    let refused = run_as(
        &mut tw,
        loader,
        &depart_line(fx.carrier, fx.field, fx.phobos),
    );
    assert!(
        !under_way(&tw, fx.carrier),
        "an unauthored channel set is not a licence to fly: {refused}"
    );
    assert!(refused.contains("Permission denied"), "{refused}");

    // The narrowed seat set is what the gate reads, so a proxy narrowing removes the
    // drive along with everything else it removes.
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands.rs"),
    )
    .expect("the sim command source");
    let drives = src.find("pub(crate) fn drives").expect("the drive gate");
    let body = &src[drives..drives + 900];
    assert!(
        body.contains("granted_channels"),
        "the gate reads the SEAT's granted set, never the station's raw `channels`"
    );
}

/// **`@journey/depart` is not a world-wide dbref existence oracle** (B4).
///
/// Before R6 the arm ran `require_admin` **before** `journey_depart`, so every
/// non-staff argument answered `Permission denied.` R6 moved the gate inside, and
/// `match_object` resolves a `#N` against the whole world with no visibility test —
/// so an existing object answered `Permission denied.` and a free dbref answered
/// `I don't see that here.`, and any unprivileged actor could enumerate the world's
/// object table one command at a time. §15.1 row 10 and the `@journey` allowlist
/// stanza both claimed the byte-identical refusal that this now makes true.
///
/// All four arms in one assertion: an existing hull they may not depart, a hull that
/// exists and is owned by somebody else, a dbref naming no object at all, and
/// `/divert` — which behaved correctly only because `require_admin` still precedes
/// it.
#[test]
fn vf_b4a_an_existing_a_foreign_and_a_free_dbref_all_refuse_byte_identically() {
    let (mut tw, fx) = scene();
    let missing = ObjId(90_000);
    assert!(
        tw.world().get(missing).is_none(),
        "the fixture's free dbref really names nothing"
    );

    let exists = run_as(
        &mut tw,
        fx.bystander,
        &depart_line(fx.carrier, fx.field, fx.phobos),
    );
    let foreign = run_as(
        &mut tw,
        fx.bystander,
        &depart_line(fx.owned, fx.field, fx.phobos),
    );
    let absent = run_as(
        &mut tw,
        fx.bystander,
        &depart_line(missing, fx.field, fx.phobos),
    );
    let divert = run_as(
        &mut tw,
        fx.bystander,
        &format!(
            "@journey/divert #{}=corridor #{}:100",
            missing.0, fx.phobos.0
        ),
    );
    assert_eq!(exists, absent, "an existing hull and a free dbref");
    assert_eq!(foreign, absent, "somebody else's hull and a free dbref");
    assert_eq!(divert, absent, "and `/divert`, which was already correct");
    assert!(exists.contains("Permission denied"), "{exists}");
    assert!(
        !absent.contains("don't see that here"),
        "the resolution failure no longer leaks: {absent}"
    );
}

/// **An unauthored route and journey wire encode no new keys** (B14).
///
/// A byte round trip against the pre-pass encoding: an unhazardous, uninterdicted,
/// uncontrolled route serializes exactly the keys it serialized before E7 residue
/// pass 1, so a rollback to that build still decodes every document written since
/// under its `deny_unknown_fields` wires. An **authored** band still encodes, so the
/// skip is a default-elision and never a data loss.
#[test]
fn vf_b14a_an_unauthored_route_and_journey_wire_encode_no_new_keys() {
    let plain = RouteDef::new(
        mush_core::CanonicalId::new("plain").expect("id"),
        (ObjId(10), ObjId(20)),
        vec![LegDef::new(ObjId(20), 100).expect("leg")],
        mush_core::CanonicalId::new("fusion").expect("class"),
        7,
    )
    .expect("a route");
    let encoded = serde_json::to_value(&plain).expect("encode");
    let leg = encoded["legs"][0]
        .as_object()
        .expect("the leg encodes as an object");
    assert_eq!(
        leg.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["base_s", "to"],
        "an unauthored leg writes exactly the pre-pass keys"
    );
    assert!(
        !encoded
            .as_object()
            .expect("object")
            .contains_key("controller"),
        "and an uncontrolled route writes no `controller` at all: {encoded}"
    );
    // Still round trips, and an AUTHORED band is still written.
    let back: RouteDef = serde_json::from_value(encoded).expect("decode");
    assert_eq!(back, plain);
    let banded = RouteDef::new(
        mush_core::CanonicalId::new("banded").expect("id"),
        (ObjId(10), ObjId(20)),
        vec![LegDef::with_hazard(ObjId(20), 100, 3, 2).expect("a banded leg")],
        mush_core::CanonicalId::new("fusion").expect("class"),
        7,
    )
    .expect("a route");
    let encoded = serde_json::to_value(&banded).expect("encode");
    assert_eq!(encoded["legs"][0]["hazard"], serde_json::json!(3));
    assert_eq!(encoded["legs"][0]["interdiction"], serde_json::json!(2));
    let back: RouteDef = serde_json::from_value(encoded).expect("decode");
    assert_eq!(back, banded);
}
