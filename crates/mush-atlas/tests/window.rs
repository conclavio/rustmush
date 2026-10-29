//! Windows (plan §3.5, S4a): a window over the lot composites off-loop from a
//! snapshot, re-enters through the atlas's async completion, places who waited
//! on it, and reads back whole across a restart. *Purity* (P5): the same pins
//! and feature states composite byte-identically twice, after a restart, and
//! after a close and reopen. *Elevation* (P2): a pose in the Production Office
//! stands at `z_cm = 360`; the stair portal climbs `3.6` m. *Placement* (P1): entry
//! poses are free cells of the room; the resolver puts a pose back in its room.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mush_atlas::{AtlasModule, AtlasState, starter};
use mush_content::{Appliers, ContentModule};
use mush_core::module::{ModError, Registrar};
use mush_core::{
    ArgShape, Audience, CmdError, CommandSpec, Ctx, Effect, Effects, GOD, GameModule, Inbound,
    ModuleBootData, ModuleWrite, ObjId, ObjKind, Outbox, WorldCommand,
};
use mush_proto::OutputEvent;
use mush_sim::{DomainId, SimulationModule};
use mush_testkit::TestWorld;
use smol_str::SmolStr;

type ProbeJob = Box<dyn FnOnce(&mut Ctx<'_>) + Send>;

/// A test-only module whose `+probe` command runs a queued closure in a command
/// context, so the tests can call the atlas's placement API as a scene would.
#[derive(Clone, Default)]
struct ProbeQueue(Arc<Mutex<Vec<ProbeJob>>>);

struct Probe;

impl GameModule for Probe {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(ProbeQueue::default());
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            CommandSpec {
                name: "+probe".into(),
                shape: ArgShape::Raw,
                help: "+probe -- test: run the queued closure in a command context.".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let job = ctx
                    .state::<ProbeQueue>()
                    .map(|q| Arc::clone(&q.0))
                    .and_then(|q| q.lock().expect("lock").pop());
                if let Some(job) = job {
                    job(ctx);
                }
                Ok(())
            }),
        )
    }
}

fn with_ctx<T: Send + 'static>(
    tw: &mut TestWorld,
    actor: ObjId,
    f: impl FnOnce(&mut Ctx<'_>) -> T + Send + 'static,
) -> T {
    let slot: Arc<Mutex<Option<T>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&slot);
    let queue = tw
        .world()
        .module_state::<ProbeQueue>()
        .expect("the probe module")
        .clone();
    queue.0.lock().expect("lock").push(Box::new(move |ctx| {
        *sink.lock().expect("lock") = Some(f(ctx));
    }));
    tw.run(actor, "+probe");
    let value = slot.lock().expect("lock").take();
    value.expect("the probe ran")
}

fn modules() -> Vec<Box<dyn GameModule>> {
    modules_with_shelf(starter::packs().expect("shelf"))
}

fn modules_with_shelf(shelf: Vec<mush_content::Pack>) -> Vec<Box<dyn GameModule>> {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    let content = ContentModule::new()
        .with_appliers(appliers)
        .with_boot_packs(starter::packs().expect("the shipped packs load"));
    vec![
        Box::new(Probe),
        Box::new(mush_npc::NpcModule::new()),
        Box::new(SimulationModule::new()),
        Box::new(AtlasModule::new().with_shelf(shelf)),
        Box::new(content),
    ]
}

fn lot() -> TestWorld {
    TestWorld::with_modules(modules())
}

fn room_named(tw: &TestWorld, name: &str) -> ObjId {
    tw.world()
        .iter()
        .find(|o| o.kind == ObjKind::Room && o.name == name)
        .map(|o| o.id)
        .unwrap_or_else(|| panic!("no room named {name}"))
}

fn step(
    tw: &mut TestWorld,
    actor: ObjId,
    line: &str,
) -> (Vec<(Audience, OutputEvent)>, Vec<Effect>) {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(
        Inbound::Cmd {
            conn: 0,
            actor,
            line: line.to_string(),
        },
        &mut out,
        &mut fx,
    );
    (out.take(), fx.drain())
}

fn text(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

fn module_async(effects: &[Effect]) -> (u64, String) {
    let mut found = effects.iter().filter_map(|e| match e {
        Effect::ModuleAsync {
            module,
            correlation,
            payload,
            ..
        } if module.as_str() == mush_atlas::NS => Some((*correlation, payload.clone())),
        _ => None,
    });
    let one = found.next().expect("one deferred composite");
    assert!(found.next().is_none(), "exactly one composite was deferred");
    one
}

fn apply_done(
    tw: &mut TestWorld,
    actor: ObjId,
    correlation: u64,
    payload: String,
) -> Vec<(Audience, OutputEvent)> {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(
        Inbound::Apply(WorldCommand::ModuleAsyncDone {
            module: mush_atlas::NS.into(),
            actor,
            correlation,
            payload,
        }),
        &mut out,
        &mut fx,
    );
    out.take()
}

/// Run the deferred composite off-loop and re-enter it: the outcome's digest and
/// the events the completion printed.
fn finish(
    tw: &mut TestWorld,
    actor: ObjId,
    effects: &[Effect],
) -> (String, Vec<(Audience, OutputEvent)>) {
    let (correlation, payload) = module_async(effects);
    let outcome = mush_atlas::run_async(&payload);
    let parsed: mush_atlas::CompositeOutcome = serde_json::from_str(&outcome).expect("an outcome");
    let digest = parsed
        .result
        .as_ref()
        .map(|c| c.digest.clone())
        .unwrap_or_else(|e| panic!("the composite failed: {e}"));
    (digest, apply_done(tw, actor, correlation, outcome))
}

fn atlas(tw: &TestWorld) -> &AtlasState {
    tw.world()
        .module_state::<AtlasState>()
        .expect("atlas state")
}

fn open_over(tw: &mut TestWorld, actor: ObjId, rooms: &str) -> (DomainId, String) {
    let (events, effects) = step(tw, actor, &format!("@atlas/window open {rooms}"));
    let t = text(tw, &events, actor);
    assert!(t.contains("opens; its map is being drawn"), "{t}");
    let domain = atlas(tw)
        .windows
        .open
        .keys()
        .max()
        .copied()
        .map(DomainId)
        .expect("a window entry");
    let (digest, done) = finish(tw, actor, &effects);
    let t = text(tw, &done, actor);
    assert!(t.contains("The map is drawn"), "{t}");
    (domain, digest)
}

fn reboot(mut tw: TestWorld) -> TestWorld {
    let boot = boot_data(&mut tw);
    // Keep objects and persisted documents while installing this composition's
    // fresh runtime handles; copying old module slots would cross registries.
    tw.reboot_into(modules(), boot).expect("atlas restart")
}

#[test]
fn a_window_over_the_coffee_shop_composites_the_same_bytes_twice_and_after_a_restart() {
    let mut tw = lot();
    let staff = GOD;
    let coffee = room_named(&tw, "Coffee Shop");
    tw.world_mut()
        .move_to(staff, coffee)
        .expect("staff to the coffee shop");
    let (domain, first) = open_over(&mut tw, staff, "here");
    let entry = atlas(&tw)
        .windows
        .open
        .get(&domain.0)
        .cloned()
        .expect("open");
    let composite = entry.composite.as_ref().expect("composited");
    assert_eq!(composite.digest, first);
    // The frame: the coffee shop's footprint plus ten metres, at the street cell.
    assert_eq!(entry.frame.cell_cm, 100);
    assert!(
        entry.rooms.contains(&coffee),
        "the coffee shop is on its own map"
    );
    assert!(
        entry.rooms.contains(&room_named(&tw, "Backlot")),
        "the margin reaches the backlot: {:?}",
        entry.rooms
    );
    // Pins: at least the coffee shop's floor tile and the backlot's ground.
    assert!(composite.pins.len() >= 2, "{:?}", composite.pins);
    // The same input composites to the same bytes (P5).
    let input = mush_atlas::capture(&atlas(&tw).records, &entry.frame).expect("capture");
    let again = mush_atlas::composite(&input).expect("composite");
    assert_eq!(again.digest, first);
    assert_eq!(again, *composite);
    // A restart reads the document back, whole.
    let tw2 = reboot(tw);
    let entry2 = atlas(&tw2)
        .windows
        .open
        .get(&domain.0)
        .cloned()
        .expect("open after boot");
    assert_eq!(
        entry2.composite.as_ref().map(|c| c.digest.as_str()),
        Some(first.as_str())
    );
    assert!(
        mush_sim::window_of(tw2.world(), domain).is_some(),
        "the simulation's window survives"
    );
    // Close, reopen: the same cells.
    let mut tw2 = tw2;
    let (events, _) = step(
        &mut tw2,
        staff,
        &format!("@atlas/window close {}", domain.0),
    );
    assert!(
        text(&tw2, &events, staff).contains("closed"),
        "{}",
        text(&tw2, &events, staff)
    );
    assert!(atlas(&tw2).windows.open.is_empty());
    let (_, second) = open_over(&mut tw2, staff, "here");
    assert_eq!(second, first, "closing and reopening changes no cell");
}

#[test]
fn a_participant_is_placed_on_a_free_cell_and_the_resolver_returns_their_room() {
    let mut tw = lot();
    let staff = GOD;
    let coffee = room_named(&tw, "Coffee Shop");
    tw.world_mut()
        .move_to(staff, coffee)
        .expect("to the coffee shop");
    let rae = tw.spawn_player("Rae");
    tw.world_mut()
        .move_to(rae, coffee)
        .expect("Rae to the coffee shop");
    // Staff open the window over the room they stand in; the participants are
    // placed by the entry path.
    let (domain, _) = open_over(&mut tw, staff, "here");
    let records = atlas(&tw).records.clone();
    let entry = atlas(&tw)
        .windows
        .open
        .get(&domain.0)
        .cloned()
        .expect("open");
    let composite = entry.composite.clone().expect("composited");
    // Place Rae through the API the scene path uses.
    with_ctx(&mut tw, staff, move |ctx| {
        mush_atlas::window::place_in_room(ctx, domain, rae, coffee, None)
    })
    .expect("Rae stands on the map");
    let (d, pose) = mush_sim::pose_of(tw.world(), rae).expect("a pose");
    assert_eq!(d, domain);
    assert_eq!(
        pose.z_cm, 0,
        "the coffee shop floor is at the datum: {pose:?}"
    );
    let geo = records.roomgeo.get(&coffee).expect("geometry");
    let shape = geo.footprint.shape().expect("shape");
    let p = mush_atlas::PointCm {
        x: pose.x_cm,
        y: pose.y_cm,
    };
    assert!(
        shape.contains(p),
        "the pose is inside the footprint: {pose:?}"
    );
    assert_eq!(
        tw.world().get(rae).and_then(|o| o.location),
        Some(coffee),
        "P1: one room"
    );
    assert_eq!(
        mush_atlas::resolve_room(tw.world(), domain, &pose),
        Some(coffee)
    );
    // A pose just off the footprint resolves to the nearest room within reach.
    let mut off = pose.clone();
    off.x_cm = 31_350; // the coffee shop's west wall is at x = 312..314 m
    assert_eq!(
        mush_atlas::resolve_room(tw.world(), domain, &off),
        Some(coffee)
    );
    let _ = composite;
}

#[test]
fn the_mezzanine_pose_stands_at_three_point_six_and_the_stair_climbs_it() {
    let mut tw = lot();
    let staff = GOD;
    let office = room_named(&tw, "Production Office");
    let lobby = room_named(&tw, "Lobby");
    tw.world_mut()
        .move_to(staff, office)
        .expect("staff up to the office");
    // Both floors of the lobby building: the window's surfaces are its anchors'
    // (the ground floor and the mezzanine) plus the ground.
    let (domain, _) = open_over(&mut tw, staff, &format!("#{},#{}", lobby.0, office.0));
    let entry = atlas(&tw)
        .windows
        .open
        .get(&domain.0)
        .cloned()
        .expect("open");
    assert!(
        entry.rooms.contains(&lobby),
        "the lobby is on the map: {:?}",
        entry.rooms
    );
    assert!(
        entry.rooms.contains(&office),
        "the office is on the map: {:?}",
        entry.rooms
    );
    assert_eq!(entry.frame.surfaces.len(), 3, "{:?}", entry.frame.surfaces);
    with_ctx(&mut tw, staff, move |ctx| {
        mush_atlas::window::place_in_room(ctx, domain, staff, office, None)
    })
    .expect("staff stand on the mezzanine");
    let (_, pose) = mush_sim::pose_of(tw.world(), staff).expect("a pose");
    assert_eq!(pose.z_cm, 360, "P2: the mezzanine is 3.6 m up: {pose:?}");
    assert_eq!(
        mush_atlas::resolve_room(tw.world(), domain, &pose),
        Some(office)
    );
    // The same (x, y) at the datum is the lobby, one floor down.
    let mut down = pose.clone();
    down.z_cm = 0;
    assert_eq!(
        mush_atlas::resolve_room(tw.world(), domain, &down),
        Some(lobby)
    );
    let stairs = atlas(&tw)
        .records
        .portals
        .values()
        .find(|p| p.key.entry.as_str() == "exit.stairs")
        .expect("the stair portal");
    assert!(
        (stairs.dz_m - 3.6).abs() < 1e-9,
        "the stair's dz: {}",
        stairs.dz_m
    );
}

#[test]
fn a_window_over_the_whole_lot_coarsens_and_a_second_over_the_same_room_is_refused() {
    let mut tw = lot();
    let staff = GOD;
    let forecourt = room_named(&tw, "Forecourt & Bus Stop");
    tw.world_mut()
        .move_to(staff, forecourt)
        .expect("to the forecourt");
    // The lot is 400 m square: at 100 cm that is 400 cells a side, over the
    // 256-cell side a grid allows, so the window coarsens to 200 cm (D-14-19).
    let (domain, _) = open_over(&mut tw, staff, "0,0,400,400");
    let entry = atlas(&tw)
        .windows
        .open
        .get(&domain.0)
        .cloned()
        .expect("open");
    assert_eq!(entry.frame.cell_cm, 200, "coarsened, not refused");
    let grid = entry.frame.grid().expect("grid");
    assert_eq!((grid.width, grid.height), (200, 200));
    // Every room on the lot is on this map.
    assert!(entry.rooms.len() >= 12, "{:?}", entry.rooms);
    // Another window over a room on this map is refused, naming it.
    let (events, effects) = step(&mut tw, staff, "@atlas/window open here");
    let t = text(&tw, &events, staff);
    assert!(
        t.contains("on window #") && t.contains("map already"),
        "{t}"
    );
    assert!(effects.is_empty(), "nothing deferred: {effects:?}");
    // The listing.
    let (events, _) = step(&mut tw, staff, "@atlas/window");
    let t = text(&tw, &events, staff);
    assert!(t.contains("200x200 at 200 cm"), "{t}");
}

// ---------------------------------------------------------------------------
// S4b: the exits, walking, growth, write-back, and a scene's map
// ---------------------------------------------------------------------------

fn pose_of(tw: &TestWorld, who: ObjId) -> Option<mush_sim::Pose> {
    mush_sim::pose_of(tw.world(), who).map(|(_, p)| p)
}

fn location(tw: &TestWorld, who: ObjId) -> Option<ObjId> {
    tw.world().get(who).and_then(|o| o.location)
}

#[test]
fn walking_through_exits_places_by_the_portal_re_poses_across_rooms_and_releases_off_the_map() {
    use mush_atlas::movement::{WindowEffect, capture};
    let mut tw = lot();
    let staff = GOD;
    let coffee = room_named(&tw, "Coffee Shop");
    let backlot = room_named(&tw, "Backlot");
    let forecourt = room_named(&tw, "Forecourt & Bus Stop");
    tw.world_mut()
        .move_to(staff, coffee)
        .expect("staff to the coffee shop");
    let (domain, _) = open_over(&mut tw, staff, "here");
    // Rae walks in from the backlot: placed on a free cell by the door.
    let rae = tw.spawn_player("Rae");
    tw.world_mut()
        .move_to(rae, backlot)
        .expect("Rae to the backlot");
    let arrival =
        capture(tw.world(), rae, Some(backlot), coffee, tw.world().now()).expect("arrival capture");
    assert_eq!(
        arrival.window_effect(),
        WindowEffect::NeedsOwnerComposition {
            release: None,
            place_or_repose: Some(domain.0),
        }
    );
    assert!(
        pose_of(&tw, rae).is_none(),
        "capture does not place the actor"
    );
    let events = tw.run(rae, "coffee");
    let t = text(&tw, &events, rae);
    assert!(t.contains("Coffee Shop"), "{t}");
    assert_eq!(location(&tw, rae), Some(coffee));
    let pose = pose_of(&tw, rae).expect("placed on entry");
    assert!(
        (pose.x_cm - 32_300).abs() < 160,
        "by the door at x 323 m: {pose:?}"
    );
    assert!(
        pose.y_cm > 26_300 && pose.y_cm < 26_600,
        "just inside the south wall: {pose:?}"
    );
    assert_eq!(
        mush_atlas::resolve_room(tw.world(), domain, &pose),
        Some(coffee)
    );
    // Out to the backlot, which the margin covers: re-posed, still on the map.
    let crossing = capture(tw.world(), rae, Some(coffee), backlot, tw.world().now())
        .expect("same-map capture");
    assert_eq!(
        crossing.window_effect(),
        WindowEffect::NeedsOwnerComposition {
            release: None,
            place_or_repose: Some(domain.0),
        }
    );
    assert_eq!(
        pose_of(&tw, rae).as_ref(),
        Some(&pose),
        "capture does not re-pose"
    );
    let events = tw.run(rae, "backlot");
    assert!(text(&tw, &events, rae).contains("Backlot"));
    assert_eq!(location(&tw, rae), Some(backlot));
    let pose = pose_of(&tw, rae).expect("re-posed");
    assert!(pose.y_cm >= 27_000, "on the backlot: {pose:?}");
    // South-west to the forecourt (covered too), then to Limbo: released.
    tw.run(rae, "forecourt");
    assert_eq!(location(&tw, rae), Some(forecourt));
    assert!(pose_of(&tw, rae).is_some());
    let departure = capture(
        tw.world(),
        rae,
        Some(forecourt),
        mush_core::LIMBO,
        tw.world().now(),
    )
    .expect("departure capture");
    assert_eq!(
        departure.window_effect(),
        WindowEffect::NeedsOwnerComposition {
            release: Some(domain.0),
            place_or_repose: None,
        }
    );
    assert!(
        pose_of(&tw, rae).is_some(),
        "capture does not release the actor"
    );
    tw.run(rae, "limbo");
    assert_eq!(location(&tw, rae), Some(mush_core::LIMBO));
    assert!(pose_of(&tw, rae).is_none(), "released where they stand");
    let no_map = capture(
        tw.world(),
        rae,
        Some(mush_core::LIMBO),
        mush_core::LIMBO,
        tw.world().now(),
    )
    .expect("off-map capture");
    assert_eq!(no_map.window_effect(), WindowEffect::NoSelectedEffect);
}

#[test]
fn plus_move_walks_cells_and_walls_counters_and_closed_rooms_stop_it() {
    let mut tw = lot();
    let staff = GOD;
    let lobby = room_named(&tw, "Lobby");
    let breakroom = room_named(&tw, "Breakroom");
    let forecourt = room_named(&tw, "Forecourt & Bus Stop");
    tw.world_mut()
        .move_to(staff, lobby)
        .expect("staff to the lobby");
    let (domain, _) = open_over(&mut tw, staff, "here");
    let rae = tw.spawn_player("Rae");
    tw.world_mut()
        .move_to(rae, forecourt)
        .expect("Rae to the forecourt");
    tw.run(rae, "lobby");
    assert_eq!(location(&tw, rae), Some(lobby));
    let start = pose_of(&tw, rae).expect("placed by the lobby door");
    assert!(
        start.y_cm > 23_600,
        "by the door on the north wall: {start:?}"
    );
    // Two cells south: the pose moves two metres.
    let events = tw.run(rae, "+move south 2");
    let t = text(&tw, &events, rae);
    assert!(t.contains("You move 2 cells south"), "{t}");
    let pose = pose_of(&tw, rae).expect("still placed");
    assert_eq!(start.y_cm - pose.y_cm, 200, "{start:?} -> {pose:?}");
    assert_eq!(pose.heading_mdeg, 180_000);
    assert_eq!(location(&tw, rae), Some(lobby));
    // West until the wall stops the walk short.
    let events = tw.run(rae, "+move west 40");
    let t = text(&tw, &events, rae);
    assert!(t.contains("wall is in the way"), "{t}");
    assert!(!t.contains("You move 40 cells"), "{t}");
    let pose = pose_of(&tw, rae).expect("placed");
    assert!(
        pose.x_cm > 25_050,
        "never through the west wall at x 250 m: {pose:?}"
    );
    // The reception desk (a counter across y = 222, x 256..270 — cells 14..28 of
    // row 20 on this frame, whose origin is (242, 202)) blocks the way south
    // from just north of it.
    let events = tw.run(rae, "+move 20,21");
    assert!(
        text(&tw, &events, rae).contains("You move"),
        "{}",
        text(&tw, &events, rae)
    );
    let events = tw.run(rae, "+move south 3");
    let t = text(&tw, &events, rae);
    assert!(t.contains("reception_desk is in the way"), "{t}");
    // The breakroom, made restricted, refuses the walk through its door.
    tw.world_mut()
        .module_state_mut::<AtlasState>()
        .expect("atlas")
        .records
        .roomgeo
        .get_mut(&breakroom)
        .expect("breakroom geometry")
        .access = mush_atlas::Access::Restricted;
    // Around the desk's east end, then down beside the breakroom's edge (its
    // cells start at column 34).
    tw.run(rae, "+move 30,21");
    let events = tw.run(rae, "+move 33,13");
    assert!(
        text(&tw, &events, rae).contains("(33, 13)"),
        "{}",
        text(&tw, &events, rae)
    );
    let events = tw.run(rae, "+move east 6");
    let t = text(&tw, &events, rae);
    assert!(t.contains("Breakroom is not open to you"), "{t}");
    assert_eq!(location(&tw, rae), Some(lobby), "P1: never in two rooms");
    // Staff walk anywhere: through the door into the breakroom, and the room
    // follows the pose (D-14-18).
    with_ctx(&mut tw, staff, move |ctx| {
        mush_atlas::window::place_in_room(ctx, domain, staff, lobby, None)
    })
    .expect("staff on the map");
    tw.run(staff, "+move 30,23");
    tw.run(staff, "+move 33,13");
    let events = tw.run(staff, "+move east 8");
    let t = text(&tw, &events, staff);
    assert!(t.contains("in Breakroom"), "{t}");
    assert_eq!(location(&tw, staff), Some(breakroom));
}

#[test]
fn stepping_off_the_edge_grows_the_window_into_the_backlot() {
    let mut tw = lot();
    let staff = GOD;
    let coffee = room_named(&tw, "Coffee Shop");
    let backlot = room_named(&tw, "Backlot");
    tw.world_mut()
        .move_to(staff, coffee)
        .expect("staff to the coffee shop");
    // A tight extent: the coffee shop and two metres of the backlot's edge.
    let (domain, _) = open_over(&mut tw, staff, "312,240,334,272");
    let entry = atlas(&tw)
        .windows
        .open
        .get(&domain.0)
        .cloned()
        .expect("open");
    assert!(!entry.rooms.contains(&backlot) || entry.rooms.contains(&backlot));
    let rae = tw.spawn_player("Rae");
    tw.world_mut()
        .move_to(rae, backlot)
        .expect("Rae to the backlot");
    tw.run(rae, "coffee");
    assert!(pose_of(&tw, rae).is_some(), "placed by the door");
    // North: through the door cell onto the strip, then the edge — the map
    // grows to take in the backlot, and the walk waits for the new composite.
    let (events, effects) = step(&mut tw, rae, "+move north 12");
    let t = text(&tw, &events, rae);
    assert!(t.contains("the map grows to take in Backlot"), "{t}");
    let (digest, done) = finish(&mut tw, staff, &effects);
    assert!(!digest.is_empty());
    let _ = done;
    let entry = atlas(&tw)
        .windows
        .open
        .get(&domain.0)
        .cloned()
        .expect("open");
    assert!(entry.rooms.contains(&backlot), "{:?}", entry.rooms);
    assert!(
        entry.frame.bbox.max.y >= 290 * 100,
        "{:?}",
        entry.frame.bbox
    );
    assert!(
        pose_of(&tw, rae).is_some(),
        "the walker keeps their pose across the growth"
    );
    let events = tw.run(rae, "+move north 3");
    let t = text(&tw, &events, rae);
    assert!(t.contains("in Backlot"), "{t}");
    assert_eq!(
        location(&tw, rae),
        Some(backlot),
        "the room follows the pose"
    );
}

#[test]
fn a_destroyed_counter_survives_a_pack_update_and_shows_in_the_next_composite() {
    let mut tw = lot();
    let staff = GOD;
    let lobby = room_named(&tw, "Lobby");
    tw.world_mut()
        .move_to(staff, lobby)
        .expect("staff to the lobby");
    let (domain, before) = open_over(&mut tw, staff, "here");
    // The desk is destroyed by hand: the window over it redraws.
    let (events, effects) = step(
        &mut tw,
        staff,
        "@atlas/feature feature.reception_desk=destroyed",
    );
    let t = text(&tw, &events, staff);
    assert!(t.contains("is now Destroyed; 1 window(s) redraw"), "{t}");
    let (after, _) = finish(&mut tw, staff, &effects);
    assert_ne!(after, before, "the wreckage changes the cells");
    let entry = atlas(&tw)
        .windows
        .open
        .get(&domain.0)
        .cloned()
        .expect("open");
    let composite = entry.composite.expect("redrawn");
    let floor = composite
        .surfaces
        .iter()
        .find(|s| matches!(s.surface, mush_atlas::SurfaceRef::Floor { .. }))
        .expect("the lobby floor");
    let desk = floor
        .features
        .iter()
        .find(|f| f.key.entry.as_str() == "feature.reception_desk")
        .expect("the desk is overlaid");
    assert_eq!(desk.state, mush_atlas::record::FeatureState::Destroyed);
    let (x, y) = desk.cells[0];
    assert_eq!(
        floor.cells.get(x, y).map(|c| c.class),
        Some(mush_atlas::TerrainClass::Rubble)
    );
    // A description regeneration: the pack moves to its next version with a new
    // lobby desc; the applier's update keeps the wreckage.
    let mut doc: serde_json::Value = serde_json::from_str(starter::STAFF_LOBBY_JSON).expect("json");
    // "The next version" is read off the shipped bytes rather than written as a
    // literal: 16b cut D.5 moved the staff lobby pack 3 -> 4, and a literal 3 was
    // a downgrade the plan refused.
    let next = doc["manifest"]["version"]
        .as_u64()
        .expect("the shipped pack declares a version")
        + 1;
    doc["manifest"]["version"] = serde_json::json!(next);
    doc["entries"]["room.lobby"]["def"]["desc"] = serde_json::json!("Reception, repainted.");
    let v3 = signed(doc);
    // Configure the new binary's shelf before composition, as an operator's
    // `[atlas].pack_paths` would, then reload persisted state with fresh handles.
    let mut shelf = starter::packs().expect("shelf");
    shelf.retain(|pack| pack.manifest().id() != v3.manifest().id());
    shelf.push(v3);
    let boot = boot_data(&mut tw);
    let mut tw2 = tw
        .reboot_into(modules_with_shelf(shelf), boot)
        .expect("atlas restart with updated shelf");
    let lobby = room_named(&tw2, "Lobby");
    tw2.world_mut()
        .move_to(staff, lobby)
        .expect("staff to the lobby");
    let shelved = atlas(&tw2)
        .shelf
        .get(&mush_core::CanonicalId::new("neighborhood.staff_lobby").expect("id"))
        .map(|p| p.manifest().version())
        .expect("on the shelf");
    assert_eq!(
        u64::from(shelved),
        next,
        "the shelf carries the new version"
    );
    let events = tw2.run(staff, "@atlas/pack plan neighborhood.staff_lobby");
    let t = text(&tw2, &events, staff);
    let fingerprint = t
        .lines()
        .find_map(|l| l.trim().strip_prefix("fingerprint "))
        .and_then(|f| f.split_whitespace().next())
        .map(str::to_string)
        .unwrap_or_else(|| panic!("no fingerprint in:\n{t}"));
    let events = tw2.run(staff, &format!("@atlas/pack apply {fingerprint}"));
    let t = text(&tw2, &events, staff);
    assert!(t.contains("Applied") && t.contains("1 updated"), "{t}");
    let desk = atlas(&tw2)
        .records
        .features
        .values()
        .find(|f| f.key.entry.as_str() == "feature.reception_desk")
        .expect("the desk survives the update");
    assert_eq!(
        desk.state,
        mush_atlas::record::FeatureState::Destroyed,
        "rule 8: live state kept"
    );
    let events = tw2.run(staff, "look");
    let t = text(&tw2, &events, staff);
    assert!(
        t.contains("Reception, repainted."),
        "the regeneration landed: {t}"
    );
}

/// Sign a pack document: its digest is its content's.
fn signed(mut doc: serde_json::Value) -> mush_content::Pack {
    doc["manifest"]["digest"] = serde_json::Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = mush_content::parse_pack(&unsigned)
        .expect("parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = serde_json::Value::String(digest);
    mush_content::parse_pack(&serde_json::to_string(&doc).expect("json")).expect("parses")
}

fn boot_data(tw: &mut TestWorld) -> HashMap<SmolStr, ModuleBootData> {
    let ops = tw.take_module_ops();
    let mut boot: HashMap<SmolStr, ModuleBootData> = HashMap::new();
    for op in ops {
        let data = boot.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                data.kv.retain(|(k, _)| *k != key);
                data.kv.push((key, value));
            }
            ModuleWrite::Del { key } => data.kv.retain(|(k, _)| *k != key),
            _ => {}
        }
    }
    boot
}

#[test]
fn a_timestop_opens_a_map_over_its_room_and_closes_it_with_the_scene() {
    let mut mods = modules();
    mods.insert(1, Box::new(mush_scenes::ScenesModule::new()));
    let mut tw = TestWorld::with_modules(mods);
    let coffee = room_named(&tw, "Coffee Shop");
    let rae = tw.spawn_player("Rae");
    tw.world_mut()
        .move_to(rae, coffee)
        .expect("Rae to the coffee shop");
    let (events, effects) = step(&mut tw, rae, "+timestop");
    let t = text(&tw, &events, rae);
    assert!(t.contains("timestop begins"), "{t}");
    assert!(t.contains("opens over the scene"), "{t}");
    let domain = atlas(&tw)
        .windows
        .open
        .values()
        .next()
        .map(|e| e.domain)
        .expect("a window for the scene");
    let (_, done) = finish(&mut tw, rae, &effects);
    assert!(text(&tw, &done, rae).contains("The map is drawn"));
    assert_eq!(
        pose_of(&tw, rae).map(|_| ()),
        Some(()),
        "the facilitator stands on it"
    );
    let events = tw.run(rae, "+map");
    let t = text(&tw, &events, rae);
    assert!(
        t.contains(&format!("#{domain}")) && t.contains("You stand at"),
        "{t}"
    );
    // +move is a Turn command: on Rae's turn it runs.
    let events = tw.run(rae, "+move north");
    let t = text(&tw, &events, rae);
    assert!(t.contains("You move") || t.contains("in the way"), "{t}");
    let events = tw.run(rae, "+timestop/end");
    assert!(text(&tw, &events, rae).contains("ends"));
    assert!(atlas(&tw).windows.open.is_empty(), "closed with the scene");
    assert!(pose_of(&tw, rae).is_none());
    assert_eq!(location(&tw, rae), Some(coffee));
}

// ---------------------------------------------------------------------------
// S4c: the `atlas.map` frames — text first (P10), filtered for the viewer (V)
// ---------------------------------------------------------------------------

fn panel_frames(
    events: &[(Audience, OutputEvent)],
    who: ObjId,
    kind: mush_proto::PanelFrameKind,
) -> Vec<serde_json::Value> {
    events
        .iter()
        .filter(|(aud, _)| matches!(aud, Audience::Actor(a) if *a == who))
        .filter_map(|(_, e)| match e {
            OutputEvent::Panel {
                feed,
                kind: k,
                data,
            } if feed.as_str() == mush_atlas::MAP_FEED && *k == kind => Some(data.clone()),
            _ => None,
        })
        .collect()
}

fn text_frame(data: &serde_json::Value) -> Vec<String> {
    mush_ansi::grid_lines(data, mush_proto::PanelFrameKind::Keyframe, 78, 20)
}

/// The lines a frame prints, right-trimmed (P10 is about the facts, not the
/// padding).
fn trimmed(lines: &[String]) -> Vec<String> {
    lines.iter().map(|l| l.trim_end().to_string()).collect()
}

fn expected(block: &str) -> Vec<String> {
    block
        .lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.is_empty() || true)
        .collect()
}

#[test]
fn the_coffee_shop_and_the_backlot_render_as_text_at_eighty_by_twenty_four() {
    let mut tw = lot();
    let staff = GOD;
    let coffee = room_named(&tw, "Coffee Shop");
    let backlot = room_named(&tw, "Backlot");
    tw.world_mut()
        .move_to(staff, coffee)
        .expect("staff to the coffee shop");
    let (domain, _) = open_over(&mut tw, staff, "here");
    let rae = tw.spawn_player("Rae");
    tw.world_mut()
        .move_to(rae, backlot)
        .expect("Rae to the backlot");
    tw.run(rae, "coffee");
    let events = tw.run(rae, "+map");
    let frames = panel_frames(&events, rae, mush_proto::PanelFrameKind::Keyframe);
    assert_eq!(frames.len(), 1, "one keyframe: {}", text(&tw, &events, rae));
    let coffee_frame: mush_proto::AtlasMapFrame =
        serde_json::from_value(frames[0].clone()).expect("a typed frame");
    assert_eq!(coffee_frame.window, domain.0);
    assert_eq!(coffee_frame.surface, "floor 0");
    assert_eq!(
        coffee_frame.floors,
        vec!["ground".to_string(), "floor 0".to_string()]
    );
    assert_eq!(
        coffee_frame.units.len(),
        1,
        "Rae alone: {:?}",
        coffee_frame.units
    );
    assert!(coffee_frame.units[0].you);
    let golden = "\
Coffee Shop (floor 0, 100 cm cells)  floors: ground, floor 0
legend: _ floor  # wall  + door  @ you  o other






        ###########+###########
        #_____________________#
        #_____________________#
        #__________@__________#
        #_____________________#
        #_____________________#
        #_____________________#
        #_____________________#
        #_____________________#
        #_____________________#
        #_____________________#
        #_____________________#
        #_____________________#
        #_____________________#
x 0..38  y 23..42  of 39x45  you at (19, 33)";
    let lines = text_frame(&frames[0]);
    assert_eq!(lines.len(), 23, "two headers, twenty rows, one footer");
    assert!(
        lines.iter().all(|l| l.chars().count() <= 78),
        "fits eighty columns"
    );
    assert_eq!(trimmed(&lines), expected(golden));
    // Out to the backlot: the ground surface, the dock, the buildings as walls
    // with their insides void, the grass in the paving.
    tw.run(rae, "backlot");
    let events = tw.run(rae, "+map");
    let frames = panel_frames(&events, rae, mush_proto::PanelFrameKind::Keyframe);
    let golden = "\
Backlot (ground, 100 cm cells)  floors: ground, floor 0
legend: . paved  \" grass  # wall  + door  D platform  @ you  o other
..................DDDDDDDDDDDDDDDDDDDDD
..................DDDDDDDDDDDDDDDDDDDDD
..................DDDDDDDDDDDDDDDDDDDDD
..................DDDDDDDDDDDDDDDDDDDDD
..................DDDDDDDDDDDDDDDDDDDDD
.......................................
...................@...................
.......................................
........###########+###########...#####
........#                     #...#
........#                     #...#
........#                     #...#
........#                     #...#
........#                     #.\"\"#
........#                     #.\"\"#
........#                     #.\"\"#
........#                     #.\"\"#
........#                     #.\"\"#
........#                     #.\"\"#
........#                     #.\"\"#
x 0..38  y 25..44  of 39x45  you at (19, 38)";
    let lines = text_frame(&frames[0]);
    assert_eq!(trimmed(&lines), expected(golden));
    // The query handler answers the same frame the command printed.
    let events = tw.run(rae, "+map");
    let printed = panel_frames(&events, rae, mush_proto::PanelFrameKind::Keyframe);
    let answered = {
        let (_, handler) = tw
            .registry()
            .query_handler(mush_atlas::MAP_FEED)
            .expect("the atlas.map query");
        handler
            .query(tw.world(), rae, &serde_json::json!({}))
            .expect("an answer")
    };
    match answered {
        OutputEvent::Panel { feed, kind, data } => {
            assert_eq!(feed.as_str(), mush_atlas::MAP_FEED);
            assert_eq!(kind, mush_proto::PanelFrameKind::Keyframe);
            assert_eq!(data, printed[0]);
        }
        other => panic!("not a panel: {other:?}"),
    }
}

#[test]
fn a_viewer_receives_no_cell_or_unit_outside_their_visibility() {
    let mut tw = lot();
    let staff = GOD;
    let lobby = room_named(&tw, "Lobby");
    let office = room_named(&tw, "Production Office");
    let forecourt = room_named(&tw, "Forecourt & Bus Stop");
    tw.world_mut()
        .move_to(staff, office)
        .expect("staff to the office");
    let (domain, _) = open_over(&mut tw, staff, &format!("#{},#{}", lobby.0, office.0));
    with_ctx(&mut tw, staff, move |ctx| {
        mush_atlas::window::place_in_room(ctx, domain, staff, office, None)
    })
    .expect("staff on the mezzanine");
    let rae = tw.spawn_player("Rae");
    tw.world_mut()
        .move_to(rae, forecourt)
        .expect("Rae to the forecourt");
    tw.run(rae, "lobby");
    assert!(pose_of(&tw, rae).is_some());
    // Rae's floor-0 frame: the lobby, herself, and no sign of the staffer
    // upstairs.
    let events = tw.run(rae, "+map");
    let frame: mush_proto::AtlasMapFrame = serde_json::from_value(
        panel_frames(&events, rae, mush_proto::PanelFrameKind::Keyframe)[0].clone(),
    )
    .expect("frame");
    assert_eq!(frame.surface, "floor 0");
    assert_eq!(frame.units.len(), 1);
    assert!(frame.units[0].you);
    // Her floor-1 frame: the office is restricted, so its cells are void and
    // the staffer standing in it is absent — only the building's walls show.
    let events = tw.run(rae, "+map/floor 1");
    let frame: mush_proto::AtlasMapFrame = serde_json::from_value(
        panel_frames(&events, rae, mush_proto::PanelFrameKind::Keyframe)[0].clone(),
    )
    .expect("frame");
    assert_eq!(frame.surface, "floor 1");
    assert!(
        frame.units.is_empty(),
        "no unit outside her visibility: {:?}",
        frame.units
    );
    let lines = text_frame(&serde_json::to_value(&frame).expect("json"));
    let grid_rows = &lines[2..lines.len() - 1];
    assert!(
        grid_rows.iter().all(|l| l.trim().is_empty()),
        "nothing of a floor she may see no room of reaches her:\n{}",
        lines.join("\n")
    );
    // The staffer's own floor-1 frame has the office and both of them? No —
    // Rae is a floor below; the staffer sees the office and themself.
    let events = tw.run(staff, "+map");
    let frame: mush_proto::AtlasMapFrame = serde_json::from_value(
        panel_frames(&events, staff, mush_proto::PanelFrameKind::Keyframe)[0].clone(),
    )
    .expect("frame");
    assert_eq!(frame.surface, "floor 1");
    assert_eq!(frame.units.len(), 1);
    let lines = text_frame(&serde_json::to_value(&frame).expect("json"));
    assert!(
        lines.iter().any(|l| l.contains('_')),
        "the office floor for staff"
    );
}

#[test]
fn a_move_sends_a_delta_whose_units_match_the_next_keyframe() {
    let mut tw = lot();
    let staff = GOD;
    let coffee = room_named(&tw, "Coffee Shop");
    let backlot = room_named(&tw, "Backlot");
    tw.world_mut()
        .move_to(staff, coffee)
        .expect("staff to the coffee shop");
    let (_, _) = open_over(&mut tw, staff, "here");
    let rae = tw.spawn_player("Rae");
    let kit = tw.spawn_player("Kit");
    tw.world_mut()
        .move_to(rae, backlot)
        .expect("Rae to the backlot");
    tw.world_mut()
        .move_to(kit, backlot)
        .expect("Kit to the backlot");
    tw.run(rae, "coffee");
    let events = tw.run(kit, "coffee");
    // Kit's arrival: Rae gets a delta naming it, and a keyframe of her own.
    let deltas = panel_frames(&events, rae, mush_proto::PanelFrameKind::Delta);
    assert_eq!(deltas.len(), 1, "{}", text(&tw, &events, rae));
    let delta: mush_proto::AtlasMapDelta =
        serde_json::from_value(deltas[0].clone()).expect("delta");
    assert_eq!(delta.note.as_deref(), Some("Kit arrives on the map."));
    assert_eq!(delta.units.len(), 2);
    assert!(!panel_frames(&events, kit, mush_proto::PanelFrameKind::Keyframe).is_empty());
    // Kit moves: both get a delta; the delta's units are the next keyframe's.
    let events = tw.run(kit, "+move north 2");
    let delta: mush_proto::AtlasMapDelta = serde_json::from_value(
        panel_frames(&events, rae, mush_proto::PanelFrameKind::Delta)[0].clone(),
    )
    .expect("delta");
    assert_eq!(
        delta.note.as_deref(),
        Some(&*format!(
            "Kit moves north to ({}, {}).",
            delta.units[1].cell.0, delta.units[1].cell.1
        ))
    );
    let events = tw.run(rae, "+map");
    let frame: mush_proto::AtlasMapFrame = serde_json::from_value(
        panel_frames(&events, rae, mush_proto::PanelFrameKind::Keyframe)[0].clone(),
    )
    .expect("frame");
    assert_eq!(
        frame.units, delta.units,
        "a keyframe after a delta shows the same units"
    );
    assert_eq!(frame.you, delta.you);
    // Line mode prints the delta's note alone.
    let lines = mush_ansi::grid_lines(
        &serde_json::to_value(&delta).expect("json"),
        mush_proto::PanelFrameKind::Delta,
        78,
        20,
    );
    assert_eq!(lines, vec![delta.note.clone().expect("note")]);
}

// ---------------------------------------------------------------------------
// S5: the walk is priced, the engine's geometry reaches propagation
// ---------------------------------------------------------------------------

#[test]
fn a_walk_costs_the_tables_price_and_stops_at_what_it_cannot_cross() {
    let mut tw = lot();
    let staff = GOD;
    let coffee = room_named(&tw, "Coffee Shop");
    let backlot = room_named(&tw, "Backlot");
    tw.world_mut()
        .move_to(staff, coffee)
        .expect("staff to the coffee shop");
    let (_, _) = open_over(&mut tw, staff, "here");
    let rae = tw.spawn_player("Rae");
    tw.world_mut()
        .move_to(rae, backlot)
        .expect("Rae to the backlot");
    tw.run(rae, "coffee");
    // Two cells of floor: one each.
    let ev = tw.run(rae, "+move west 2");
    let out = text(&tw, &ev, rae);
    assert!(out.contains("(cost 2.00)"), "{out}");
    // A diagonal costs its length.
    let ev = tw.run(rae, "+move ne");
    let out = text(&tw, &ev, rae);
    assert!(out.contains("(cost 1.41)"), "{out}");
    // The wall is the table's refusal, in the table's words.
    let ev = tw.run(rae, "+move west 40");
    let out = text(&tw, &ev, rae);
    assert!(out.contains("wall is in the way"), "{out}");
}

#[test]
fn propagation_over_the_lot_takes_the_metric_and_the_line_of_sight() {
    use mush_core::{ClockRef, ClockStamp};
    use mush_propagation::PropagationContext;
    let mut tw = lot();
    let staff = GOD;
    let coffee = room_named(&tw, "Coffee Shop");
    let backlot = room_named(&tw, "Backlot");
    tw.world_mut()
        .move_to(staff, coffee)
        .expect("staff to the coffee shop");
    let (domain, _) = open_over(&mut tw, staff, "here");
    let rae = tw.spawn_player("Rae");
    let kit = tw.spawn_player("Kit");
    tw.world_mut()
        .move_to(rae, backlot)
        .expect("Rae to the backlot");
    tw.world_mut()
        .move_to(kit, backlot)
        .expect("Kit to the backlot");
    tw.run(rae, "coffee");
    tw.run(kit, "coffee");
    tw.run(kit, "+move north 3");
    let at = ClockStamp {
        clock: ClockRef::Utc,
        at: 0,
    };
    // Both in the shop, nothing between them: the metric is the Chebyshev
    // distance in cells and the sight is clear.
    let (rae_cell, kit_cell) = {
        let w = tw.world();
        let frame = mush_atlas::frame_for(
            w,
            w.module_state::<mush_atlas::AtlasState>().expect("atlas"),
            rae,
            domain,
        )
        .expect("frame");
        let cell_of = |id: ObjId| {
            frame
                .units
                .iter()
                .find(|u| u.id == id.to_string())
                .map(|u| u.cell)
                .expect("placed")
        };
        (cell_of(rae), cell_of(kit))
    };
    let expected_range = rae_cell
        .0
        .abs_diff(kit_cell.0)
        .max(rae_cell.1.abs_diff(kit_cell.1));
    let metric = mush_sim::metric_between(tw.world(), rae, kit).expect("a shared window");
    assert_eq!(metric.range_cells, expected_range);
    assert!(!metric.occluded);
    assert_eq!(
        mush_atlas::window::sight_between(tw.world(), rae, kit),
        Some(mush_atlas::Sight::Clear)
    );
    let ctx = {
        let w = tw.world();
        let spatial = w.module_state::<mush_sim::SpatialState>().expect("sim");
        PropagationContext::build(spatial, w, rae, kit, at.clone()).expect("a context")
    };
    assert_eq!(
        ctx.geometry.range, expected_range,
        "propagation's range is the metric"
    );
    assert!(
        !ctx.geometry.occluded,
        "propagation's occlusion is the line of sight"
    );
    // Hearing and the direct trace read the same surface.
    let hearing = mush_atlas::window::hearing_between(tw.world(), rae, kit).expect("hearing");
    assert_eq!(
        hearing.attenuation,
        expected_range * mush_atlas::trace::HEARING_PER_CELL
    );
    let trace = mush_atlas::window::direct_fire_between(tw.world(), rae, kit).expect("trace");
    assert!(trace.reaches());
    assert_eq!(trace.range_cells, expected_range);
    // Rae steps out to the lot: a different surface of one window. Before S9.4
    // that was occluded at the grid's range with no sight answered; now the line
    // runs through the doorway the two surfaces share, so a sight answers and
    // the metric is the two-legged one (S9.4, S5's note).
    tw.run(rae, "backlot");
    assert!(pose_of(&tw, rae).is_some(), "the lot is on the map");
    let metric = mush_sim::metric_between(tw.world(), rae, kit).expect("still one window");
    assert!(metric.range_cells >= 1);
    let sight = mush_atlas::window::sight_between(tw.world(), rae, kit)
        .expect("S9.4: the doorway joins the floor to the lot");
    assert_eq!(metric.occluded, !sight.is_clear_or_partial());
    let ctx = {
        let w = tw.world();
        let spatial = w.module_state::<mush_sim::SpatialState>().expect("sim");
        PropagationContext::build(spatial, w, rae, kit, at.clone()).expect("a context")
    };
    assert_eq!(ctx.geometry.occluded, metric.occluded);
    assert_eq!(ctx.geometry.range, metric.range_cells);
    // Staff read the same answers by hand.
    // Staff name them by number: the two are not in the staffer's room.
    let los = format!("@atlas/los #{}=#{}", rae.0, kit.0);
    let ev = tw.run(staff, &los);
    let out = text(&tw, &ev, staff);
    assert!(
        out.contains("cells apart") || out.contains("cell apart"),
        "{out}"
    );
    assert!(
        out.contains("Sight:") && !out.contains("no doorway joins"),
        "S9.4: the doorway carries the line: {out}"
    );
    tw.run(rae, "coffee");
    let ev = tw.run(staff, &los);
    let out = text(&tw, &ev, staff);
    assert!(
        out.contains("in sight") && out.contains("Sight: clear") && out.contains("Direct fire:"),
        "{out}"
    );
    assert!(out.contains("Hearing: attenuation"), "{out}");
    // A player may not.
    let ev = tw.run(rae, &los);
    let out = text(&tw, &ev, rae);
    assert!(out.contains("staff's"), "{out}");
}

#[test]
fn the_geometry_slot_is_filled_once_at_boot() {
    let mut tw = lot();
    let err =
        mush_sim::register_window_geometry(tw.world_mut(), std::sync::Arc::new(|_, _, _| None))
            .expect_err("the atlas filled it at boot");
    assert!(err.contains("registered already"), "{err}");
    // Two people on no window have no metric.
    let rae = tw.spawn_player("Rae");
    let kit = tw.spawn_player("Kit");
    assert_eq!(mush_sim::metric_between(tw.world(), rae, kit), None);
}

// ---------------------------------------------------------------------------
// 16b cut S.2: doc 16 §3.6's world-side exclusion, on the map surface
// ---------------------------------------------------------------------------

/// **A domain a simulation authority holds is no map, for anybody** (doc 16 §3.6
/// "World-side exclusion": *its domain is excluded from `visible_rooms`,
/// `units_for`, the atlas `+map` frame … for every viewer including staff*).
///
/// The exclusion is keyed on the **domain**, not on the unit: a deployed unit is
/// already absent from `mush_sim::participants`, and this is the other half — the
/// window itself stops answering, so a viewer standing on an arena's ground is told
/// exactly what a viewer whose window is shut is told, and staff are not exempt.
///
/// `mush_sim::deployed::mark` is the world-side index `mush_tactics`' check-out
/// publishes into; marking Rae's own object is the shortest honest way to put a
/// deployed unit in this domain, because a deployed unit's world placement is left
/// frozen exactly where it stood.
#[test]
fn a_domain_an_arena_holds_is_no_map_for_anybody() {
    let mut tw = lot();
    let staff = GOD;
    let coffee = room_named(&tw, "Coffee Shop");
    tw.world_mut()
        .move_to(staff, coffee)
        .expect("staff to the coffee shop");
    let (domain, _) = open_over(&mut tw, staff, "here");
    let backlot = room_named(&tw, "Backlot");
    let rae = tw.spawn_player("Rae");
    tw.world_mut()
        .move_to(rae, backlot)
        .expect("Rae to the backlot");
    // Walking **in** is what places somebody on a window; `move_to` alone does not.
    tw.run(rae, "coffee");
    tw.run(staff, "backlot");
    tw.run(staff, "coffee");

    // The control: the window is there, for the participant and for staff.
    let events = tw.run(rae, "+map");
    let shown = text(&tw, &events, rae);
    assert!(shown.contains("You stand at"), "{shown}");
    assert!(
        mush_atlas::frame::frame_for(tw.world(), atlas(&tw), rae, domain).is_some(),
        "the frame answers before the arena takes the ground"
    );

    // One unit under custody, standing where it stood.
    mush_sim::deployed::mark(
        tw.world_mut(),
        rae,
        mush_sim::DeployedMark::new(7, "tacsim.arena.proving_ground"),
    );
    assert!(
        mush_sim::deployed::excluded(tw.world(), mush_sim::deployed::Shown::Domain(domain)),
        "the one predicate answers for the domain the deployed unit stands in"
    );

    // And the map is shut, in the words a shut map already had.
    for who in [rae, staff] {
        let events = tw.run(who, "+map");
        let seen = text(&tw, &events, who);
        assert!(
            seen.contains("Your map is closed."),
            "#{} was answered something else: {seen}",
            who.0
        );
        assert!(
            !seen.contains("You stand at"),
            "and never a pose on an arena's ground: {seen}"
        );
        assert!(
            panel_frames(&events, who, mush_proto::PanelFrameKind::Keyframe).is_empty(),
            "#{} was sent a frame of a held domain",
            who.0
        );
        assert!(
            mush_atlas::frame::frame_for(tw.world(), atlas(&tw), who, domain).is_none(),
            "#{} could still build the frame",
            who.0
        );
        assert!(
            mush_atlas::frame::delta_for(tw.world(), atlas(&tw), who, domain, None).is_none(),
            "#{} could still build a delta",
            who.0
        );
    }
    // `+map/floor` is the other arm of the same verb, and answers the same way.
    let events = tw.run(rae, "+map/floor 0");
    let floored = text(&tw, &events, rae);
    assert!(floored.contains("Your map is closed."), "{floored}");

    // The check-in gives the ground back.
    mush_sim::deployed::clear(tw.world_mut(), rae);
    let events = tw.run(rae, "+map");
    let back = text(&tw, &events, rae);
    assert!(back.contains("You stand at"), "{back}");
}
