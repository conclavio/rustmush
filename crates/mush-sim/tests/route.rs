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
    events.iter().map(|(_, e)| text_of(e)).collect::<Vec<_>>().join("\n")
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
    tw.world().module_state::<RouteState>().expect("route state")
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
    let start = tw.world_mut().create(ObjKind::Room, "Start", GOD, None).unwrap();
    let r1 = tw.world_mut().create(ObjKind::Room, "R1", GOD, None).unwrap();
    let r2 = tw.world_mut().create(ObjKind::Room, "R2", GOD, None).unwrap();
    let r3 = tw.world_mut().create(ObjKind::Room, "R3", GOD, None).unwrap();
    let car = tw.world_mut().create(ObjKind::Thing, "Car", GOD, Some(start)).unwrap();
    Fx { start, r1, r2, r3, car }
}
fn route3(tw: &mut TestWorld, fx: &Fx) {
    // A plain object needs no sim binding to travel.
    ok(tw, GOD, &format!("@route #{}=#{}:5 #{}:5 #{}:5", fx.car.0, fx.r1.0, fx.r2.0, fx.r3.0));
}

// ----------------------------------------------------------------------

#[test]
fn car_travels_room_to_room_legibly_and_can_be_intercepted() {
    let mut tw = boot();
    tw.set_now(1000);
    let fx = fixture(&mut tw);
    route3(&mut tw, &fx);
    assert_eq!(loc(&tw, fx.car), Some(fx.start), "not moved until the sweep");
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
    assert!(out.contains("Car arrives"), "watcher in R2 sees the arrival: {out}");
    // Ann intercepts: the journey pauses, the car stays put.
    let t = texts(&tw.run(ann, &format!("+intercept #{}", fx.car.0)));
    assert!(t.contains("waylays") || t.contains("stop"), "{t}");
    assert_eq!(status(&tw, fx.car), Some(JourneyStatus::Paused));
    let _ = sweep_at(&mut tw, 1020);
    assert_eq!(loc(&tw, fx.car), Some(fx.r2), "a paused journey does not advance");
    // Staff resume; the schedule shifts by the pause, then it finishes.
    ok(&mut tw, GOD, &format!("@route/resume #{}", fx.car.0));
    let _ = sweep_at(&mut tw, 1025);
    assert_eq!(loc(&tw, fx.car), Some(fx.r3), "resumed and arrived");
    assert!(status(&tw, fx.car).is_none(), "Arrived journeys are retired");
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
    assert_eq!(trail, vec![Some(fx.r1), Some(fx.r2), Some(fx.r3)], "one hop per period, in order");
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
    assert_eq!(loc(&tw, fx.car), Some(fx.r3), "landed at its correct current position");
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
        .attach_behavior(fx.car, mush_core::BehaviorId::new("sim.participant").unwrap(), mush_core::BehaviorBinding { schema_version: 1, attached_by: GOD, attached_at: 0 })
        .unwrap();
    tw.world_mut()
        .attach_behavior(fx.start, mush_core::BehaviorId::new("sim.domain").unwrap(), mush_core::BehaviorBinding { schema_version: 1, attached_by: GOD, attached_at: 0 })
        .unwrap();
    ok(&mut tw, GOD, &format!("@domain/create #{}=sim.passive", fx.start.0));
    ok(&mut tw, GOD, &format!("@spatial/place/domain #{}=0", fx.car.0));
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
    tw.engine_mut().emit_world_event(&WorldEvent::ObjDestroyed { obj: fx.car }, &mut out, &mut fxx);
    assert!(!rstate(&tw).journeys.contains_key(&fx.car), "journey dropped");
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
    tw2.modules_loaded(std::collections::HashMap::from([(smol_str::SmolStr::new("sim"), mush_core::ModuleBootData { kv, streams: std::collections::HashMap::new() })]));
    assert_eq!(status(&tw2, fx2.car), Some(JourneyStatus::Traveling), "journey restored");
    // It continues on the next sweep.
    sweep_at(&mut tw2, 1010);
    assert_eq!(loc(&tw2, fx2.car), Some(fx2.r2), "resumed from where it was");
}
