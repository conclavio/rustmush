//! Mobility / route-interception acceptance: a plain object follows a legible,
//! interceptable route of rooms over the Coarse sweep — driven through the real
//! Engine via `TestWorld`.

use mush_core::{Audience, Effects, GOD, ObjId, ObjKind, Outbox, WorldEvent};
use mush_proto::OutputEvent;
use mush_sim::{JourneyStatus, RouteState, SimulationModule};
use mush_testkit::{TestWorld, text_of};

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
fn loc(tw: &TestWorld, id: ObjId) -> Option<ObjId> {
    tw.world().get(id).and_then(|o| o.location)
}
fn rstate(tw: &TestWorld) -> &RouteState {
    tw.world()
        .module_state::<RouteState>()
        .expect("route state")
}
fn status(tw: &TestWorld, t: ObjId) -> Option<JourneyStatus> {
    rstate(tw).journeys.get(&t).map(|j| j.status)
}
fn ok(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let ev = tw.run(who, line);
    let t = texts(&ev);
    assert!(!has_error(&ev), "{line:?} refused: {t}");
    t
}
/// Advance the world clock to `secs` and fire the Coarse route sweep.
fn sweep_at(tw: &mut TestWorld, secs: i64) -> String {
    tw.set_now(secs);
    texts(&tw.fire_due(secs * 1000))
}

struct Fx {
    start: ObjId,
    r1: ObjId,
    r2: ObjId,
    r3: ObjId,
    car: ObjId,
}
fn fixture(tw: &mut TestWorld) -> Fx {
    let start = tw
        .world_mut()
        .create(ObjKind::Room, "Start", GOD, None)
        .unwrap();
    let r1 = tw
        .world_mut()
        .create(ObjKind::Room, "R1", GOD, None)
        .unwrap();
    let r2 = tw
        .world_mut()
        .create(ObjKind::Room, "R2", GOD, None)
        .unwrap();
    let r3 = tw
        .world_mut()
        .create(ObjKind::Room, "R3", GOD, None)
        .unwrap();
    let car = tw
        .world_mut()
        .create(ObjKind::Thing, "Car", GOD, Some(start))
        .unwrap();
    Fx {
        start,
        r1,
        r2,
        r3,
        car,
    }
}
fn route3(tw: &mut TestWorld, fx: &Fx) {
    // A plain object needs no sim binding to travel.
    ok(
        tw,
        GOD,
        &format!(
            "@route #{}=#{}:5 #{}:5 #{}:5",
            fx.car.0, fx.r1.0, fx.r2.0, fx.r3.0
        ),
    );
}

// ----------------------------------------------------------------------

#[test]
fn car_travels_room_to_room_legibly_and_can_be_intercepted() {
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    route3(&mut tw, &fx);
    assert_eq!(
        loc(&tw, fx.car),
        Some(fx.start),
        "not moved until the sweep"
    );
    // A watcher waits in R2.
    let ann = tw.spawn_player("Ann");
    tw.world_mut().move_to(ann, fx.r2).unwrap();
    // Leg 1: -> R1 (arrival is legible in R1).
    let out = sweep_at(&mut tw, 1005);
    assert_eq!(loc(&tw, fx.car), Some(fx.r1));
    assert!(out.contains("Car arrives"), "{out}");
    // Leg 2: -> R2, where Ann is watching (the arrival is legible there).
    let out = sweep_at(&mut tw, 1010);
    assert_eq!(loc(&tw, fx.car), Some(fx.r2));
    assert!(
        out.contains("Car arrives"),
        "watcher in R2 sees the arrival: {out}"
    );
    // Ann intercepts: the journey pauses, the car stays put.
    let t = texts(&tw.run(ann, &format!("+intercept #{}", fx.car.0)));
    assert!(t.contains("waylays") || t.contains("stop"), "{t}");
    assert_eq!(status(&tw, fx.car), Some(JourneyStatus::Paused));
    let _ = sweep_at(&mut tw, 1020);
    assert_eq!(
        loc(&tw, fx.car),
        Some(fx.r2),
        "a paused journey does not advance"
    );
    // Staff resume; the schedule shifts by the pause, then it finishes.
    ok(&mut tw, GOD, &format!("@route/resume #{}", fx.car.0));
    let _ = sweep_at(&mut tw, 1025);
    assert_eq!(loc(&tw, fx.car), Some(fx.r3), "resumed and arrived");
    assert!(
        status(&tw, fx.car).is_none(),
        "Arrived journeys are retired"
    );
}

#[test]
fn each_room_is_visited_once_in_order_during_live_advance() {
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    route3(&mut tw, &fx);
    let mut trail = Vec::new();
    for t in [1005, 1010, 1015] {
        sweep_at(&mut tw, t);
        trail.push(loc(&tw, fx.car));
    }
    assert_eq!(
        trail,
        vec![Some(fx.r1), Some(fx.r2), Some(fx.r3)],
        "one hop per period, in order"
    );
    assert!(status(&tw, fx.car).is_none(), "retired on arrival");
}

#[test]
fn a_catch_up_after_a_gap_jumps_to_the_correct_room_without_replaying_stale_arrivals() {
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    route3(&mut tw, &fx);
    let ann = tw.spawn_player("Ann");
    tw.world_mut().move_to(ann, fx.r2).unwrap();
    // The server was "down" — jump far past the final due (1015).
    let out = sweep_at(&mut tw, 5000);
    assert_eq!(
        loc(&tw, fx.car),
        Some(fx.r3),
        "landed at its correct current position"
    );
    assert!(status(&tw, fx.car).is_none(), "arrived");
    // No stale 'arrives' spam for the rooms it passed while down.
    assert!(!out.contains("Car arrives"), "catch-up is silent: {out}");
}

#[test]
fn a_traveling_object_is_gated_from_ordinary_movement() {
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    route3(&mut tw, &fx);
    // Staff cannot @teleport it.
    let t = texts(&tw.run(GOD, &format!("@teleport #{}=#{}", fx.car.0, fx.start.0)));
    assert!(t.contains("en route"), "{t}");
    // A player cannot pick it up.
    let bob = tw.spawn_player("Bob");
    let carloc = loc(&tw, fx.car).unwrap();
    tw.world_mut().move_to(bob, carloc).unwrap();
    let t = texts(&tw.run(bob, "get Car"));
    assert!(t.contains("en route"), "{t}");
}

#[test]
fn route_refuses_a_placed_simulation_object() {
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    // Make the car a placed sim participant instead.
    tw.world_mut()
        .attach_behavior(
            fx.car,
            mush_core::BehaviorId::new("sim.participant").unwrap(),
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
    tw.world_mut()
        .attach_behavior(
            fx.start,
            mush_core::BehaviorId::new("sim.domain").unwrap(),
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", fx.start.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/domain #{}=0", fx.car.0),
    );
    let t = texts(&tw.run(GOD, &format!("@route #{}=#{}:5", fx.car.0, fx.r1.0)));
    assert!(t.contains("placed in the simulation"), "{t}");
}

#[test]
fn destroying_a_traveler_drops_its_journey() {
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    route3(&mut tw, &fx);
    let _ = tw.take_module_ops();
    tw.world_mut().destroy(fx.car).unwrap();
    let mut out = Outbox::new();
    let mut fxx = Effects::new();
    tw.engine_mut().emit_world_event(
        &WorldEvent::ObjDestroyed { obj: fx.car },
        &mut out,
        &mut fxx,
    );
    assert!(
        !rstate(&tw).journeys.contains_key(&fx.car),
        "journey dropped"
    );
    let ops = tw.take_module_ops();
    assert!(ops.iter().any(|o| matches!(&o.op, mush_core::ModuleWrite::Del { key } if key == &format!("journey:{}", fx.car.0))));
}

#[test]
fn a_route_survives_a_reboot_and_continues() {
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    route3(&mut tw, &fx);
    sweep_at(&mut tw, 1005); // -> R1
    assert_eq!(loc(&tw, fx.car), Some(fx.r1));
    // Fold the sim ops into a boot payload.
    let mut kv: Vec<(smol_str::SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        match op.op {
            mush_core::ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            mush_core::ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            mush_core::ModuleWrite::StreamAppend { .. } => {}
        }
    }
    let mut tw2 = boot();
    tw2.set_now(1005);
    let fx2 = fixture(&mut tw2);
    tw2.world_mut().move_to(fx2.car, fx2.r1).unwrap(); // it was in R1 at snapshot time
    tw2.modules_loaded(std::collections::HashMap::from([(
        smol_str::SmolStr::new("sim"),
        mush_core::ModuleBootData {
            kv,
            streams: std::collections::HashMap::new(),
            ..Default::default()
        },
    )]));
    assert_eq!(
        status(&tw2, fx2.car),
        Some(JourneyStatus::Traveling),
        "journey restored"
    );
    // It continues on the next sweep.
    sweep_at(&mut tw2, 1010);
    assert_eq!(
        loc(&tw2, fx2.car),
        Some(fx2.r2),
        "resumed from where it was"
    );
}

// ---------------------------------------------------------------------------
// M30 hardening regressions (verify workflow wk6x3a1o0)
// ---------------------------------------------------------------------------

fn bind() -> mush_core::BehaviorBinding {
    mush_core::BehaviorBinding {
        schema_version: 1,
        attached_by: GOD,
        attached_at: 0,
    }
}
/// Fold this world's pending sim module writes into a boot kv (last-write-wins).
fn fold_ops(tw: &mut TestWorld) -> Vec<(smol_str::SmolStr, serde_json::Value)> {
    let mut kv: Vec<(smol_str::SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
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
fn boot_data(
    kv: Vec<(smol_str::SmolStr, serde_json::Value)>,
) -> std::collections::HashMap<smol_str::SmolStr, mush_core::ModuleBootData> {
    std::collections::HashMap::from([(
        smol_str::SmolStr::new("sim"),
        mush_core::ModuleBootData {
            kv,
            streams: std::collections::HashMap::new(),
            ..Default::default()
        },
    )])
}

#[test]
fn placing_a_traveling_object_into_the_sim_is_refused() {
    // Cluster A: the placement side mirrors route_ineligible — a live traveler
    // cannot be sim-placed (EITHER journey OR sim-placed, never both).
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    let part = mush_core::BehaviorId::new("sim.participant").unwrap();
    let dom = mush_core::BehaviorId::new("sim.domain").unwrap();
    tw.world_mut()
        .attach_behavior(fx.car, part, bind())
        .unwrap();
    tw.world_mut()
        .attach_behavior(fx.start, dom, bind())
        .unwrap();
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", fx.start.0),
    );
    route3(&mut tw, &fx);
    let t = texts(&tw.run(GOD, &format!("@spatial/place/domain #{}=0", fx.car.0)));
    assert!(t.contains("en route"), "placing a traveler is refused: {t}");
    assert!(status(&tw, fx.car).is_some(), "the journey is untouched");
}

#[test]
fn a_reboot_keeps_a_journey_whose_traveler_has_a_lingering_unplaced_record() {
    // Cluster B regression: create-eligibility and boot-eligibility must agree.
    // An object placed then UNPLACED keeps an `Unplaced` placement record
    // (is_placed()==false, but placements.contains_key()==true), so it is
    // routable — its journey must SURVIVE reboot, not be dropped + doc-deleted.
    let part = mush_core::BehaviorId::new("sim.participant").unwrap();
    let dom = mush_core::BehaviorId::new("sim.domain").unwrap();
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    tw.world_mut()
        .attach_behavior(fx.car, part, bind())
        .unwrap();
    tw.world_mut()
        .attach_behavior(fx.start, dom, bind())
        .unwrap();
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", fx.start.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/domain #{}=0", fx.car.0),
    );
    ok(&mut tw, GOD, &format!("@spatial/unplace #{}", fx.car.0)); // lingering Unplaced record
    route3(&mut tw, &fx);
    assert_eq!(status(&tw, fx.car), Some(JourneyStatus::Traveling));
    let kv = fold_ops(&mut tw);
    let mut tw2 = boot();
    tw2.set_now(1000);
    let fx2 = fixture(&mut tw2);
    tw2.world_mut()
        .attach_behavior(
            fx2.car,
            mush_core::BehaviorId::new("sim.participant").unwrap(),
            bind(),
        )
        .unwrap();
    tw2.world_mut()
        .attach_behavior(
            fx2.start,
            mush_core::BehaviorId::new("sim.domain").unwrap(),
            bind(),
        )
        .unwrap();
    tw2.modules_loaded(boot_data(kv));
    assert_eq!(
        status(&tw2, fx2.car),
        Some(JourneyStatus::Traveling),
        "journey survived despite the traveler's lingering Unplaced record"
    );
}

#[test]
fn a_reboot_round_trips_route_state_including_a_paused_journey() {
    // Findings 0/15: full RouteState field-for-field round-trip, and a Paused
    // journey restores as Paused (never silently promoted to Traveling).
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    route3(&mut tw, &fx); // car: 3 legs
    let van = tw
        .world_mut()
        .create(ObjKind::Thing, "Van", GOD, Some(fx.start))
        .unwrap();
    ok(
        &mut tw,
        GOD,
        &format!("@route #{}=#{}:5 #{}:5", van.0, fx.r1.0, fx.r2.0),
    );
    sweep_at(&mut tw, 1005); // both advance to r1
    ok(&mut tw, GOD, &format!("@route/pause #{}", van.0));
    assert_eq!(status(&tw, van), Some(JourneyStatus::Paused));
    let before = rstate(&tw).clone();
    let kv = fold_ops(&mut tw);
    let mut tw2 = boot();
    tw2.set_now(1005);
    let fx2 = fixture(&mut tw2);
    let van2 = tw2
        .world_mut()
        .create(ObjKind::Thing, "Van", GOD, Some(fx2.r1))
        .unwrap();
    tw2.modules_loaded(boot_data(kv));
    assert_eq!(
        *rstate(&tw2),
        before,
        "RouteState round-trips field-for-field across reboot"
    );
    assert_eq!(
        status(&tw2, van2),
        Some(JourneyStatus::Paused),
        "the paused journey restored as Paused"
    );
}

#[test]
fn resume_grace_blocks_player_reinterception_but_not_staff_pause() {
    // Findings 9/11/16: RESUME_GRACE_S stops a player re-waylaying immediately,
    // but staff @route/pause bypasses the anti-grief window.
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    route3(&mut tw, &fx);
    let ann = tw.spawn_player("Ann");
    tw.world_mut().move_to(ann, fx.r1).unwrap();
    sweep_at(&mut tw, 1005); // car -> r1, where Ann waits
    ok(&mut tw, ann, &format!("+intercept #{}", fx.car.0));
    assert_eq!(status(&tw, fx.car), Some(JourneyStatus::Paused));
    ok(&mut tw, GOD, &format!("@route/resume #{}", fx.car.0)); // grace_until = now + 5
    // Player re-intercept within the grace window is refused.
    let ev = tw.run(ann, &format!("+intercept #{}", fx.car.0));
    assert!(
        has_error(&ev),
        "re-intercept within grace is refused: {}",
        texts(&ev)
    );
    assert_eq!(
        status(&tw, fx.car),
        Some(JourneyStatus::Traveling),
        "still traveling after the refused re-intercept"
    );
    // Staff @route/pause bypasses the grace.
    ok(&mut tw, GOD, &format!("@route/pause #{}", fx.car.0));
    assert_eq!(
        status(&tw, fx.car),
        Some(JourneyStatus::Paused),
        "staff can re-halt within the resume grace"
    );
}

#[test]
fn an_unenterable_leg_aborts_and_retires_the_journey() {
    // Finding 10: the move_to-fails branch (a leg's destination became
    // unenterable mid-route). We force `can_move_to` to fail by making the next
    // leg room a descendant of the traveler (a cycle) — the realistic trigger is
    // a leg room that vanished across a reboot; the failure mode is the same.
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    route3(&mut tw, &fx);
    sweep_at(&mut tw, 1005); // -> r1
    assert_eq!(loc(&tw, fx.car), Some(fx.r1));
    // Put r2 inside the car, so the sweep's move_to(car, r2) is a cycle -> Err.
    tw.world_mut().move_to(fx.r2, fx.car).unwrap();
    let _ = sweep_at(&mut tw, 1010);
    assert!(
        status(&tw, fx.car).is_none(),
        "journey aborted + retired when its next leg was unenterable"
    );
    assert_eq!(loc(&tw, fx.car), Some(fx.r1), "the car stays where it was");
}

#[test]
fn abort_cancels_and_a_second_route_is_refused_while_one_is_active() {
    // Finding 8: the already-routed refusal arm and the @route/abort transition.
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    route3(&mut tw, &fx);
    let t = texts(&tw.run(GOD, &format!("@route #{}=#{}:5", fx.car.0, fx.r1.0)));
    assert!(
        t.contains("already on a route"),
        "a second route on the same traveler is refused: {t}"
    );
    ok(&mut tw, GOD, &format!("@route/abort #{}", fx.car.0));
    assert!(status(&tw, fx.car).is_none(), "aborted journey retired");
    route3(&mut tw, &fx); // a fresh route is now accepted
    assert_eq!(status(&tw, fx.car), Some(JourneyStatus::Traveling));
}

#[test]
fn route_refuses_a_room_or_exit_traveler() {
    // Finding 8: the Room/Exit kind refusal arm.
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    let t = texts(&tw.run(GOD, &format!("@route #{}=#{}:5", fx.r1.0, fx.r2.0)));
    assert!(
        t.contains("cannot travel a route"),
        "a room cannot be routed: {t}"
    );
}
