//! S8.1 and S8.3 (14a #1, #3): a tactical step crosses a room boundary through
//! the movement gate — the portal's exit lock, the `MoveAttempt` veto chain —
//! and `+map`'s text is the frame's view. The review's own reproductions are in
//! `review_window.rs` and pass unchanged; these are the cases they do not reach.

use std::sync::{Arc, Mutex};

use mush_atlas::{AtlasModule, AtlasState, starter};
use mush_content::{Appliers, ContentModule};
use mush_core::module::{EventHook, HookPoint, HookReadContext, ModError, Registrar};
use mush_core::{
    ArgShape, Audience, CmdError, CommandSpec, Ctx, Effect, Effects, GOD, GameModule, Inbound,
    ObjId, ObjKind, Outbox, WorldCommand,
};
use mush_proto::{OutputEvent, RichText};
use mush_sim::{DomainId, SimulationModule};
use mush_testkit::TestWorld;

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
        Box::new(AtlasModule::new().with_shelf(starter::packs().expect("shelf"))),
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

/// A test module whose `MoveAttempt` hook vetoes every move into one room —
/// the scenes freeze and the housing ward, in miniature.
#[derive(Clone, Default)]
struct WetFloor(Arc<Mutex<Option<ObjId>>>);

impl EventHook for WetFloor {
    fn move_attempt(
        &self,
        _ctx: &HookReadContext<'_>,
        _actor: ObjId,
        _exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), RichText> {
        if *self.0.lock().expect("lock") == Some(dest) {
            return Err(RichText::plain("The floor is wet; wait for it to dry."));
        }
        Ok(())
    }
}

struct Veto(WetFloor);

impl GameModule for Veto {
    fn name(&self) -> &'static str {
        "veto"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.hook(HookPoint::MoveAttempt, Arc::new(self.0.clone()));
        Ok(())
    }
}

fn exit_between(tw: &TestWorld, from: ObjId, to: ObjId) -> ObjId {
    tw.world()
        .iter()
        .find(|o| o.kind == ObjKind::Exit && o.location == Some(from) && o.destination == Some(to))
        .map(|o| o.id)
        .expect("an exit between the rooms")
}

/// God opens the Lobby's window; a player comes in from the forecourt and
/// walks to the cell by the Breakroom door.
fn player_at_the_breakroom_door(tw: &mut TestWorld) -> ObjId {
    let lobby = room_named(tw, "Lobby");
    let forecourt = room_named(tw, "Forecourt & Bus Stop");
    tw.world_mut().move_to(GOD, lobby).unwrap();
    let _ = open_over(tw, GOD, "here");
    let viewer = tw.spawn_player("Viewer");
    tw.world_mut().move_to(viewer, forecourt).unwrap();
    tw.run(viewer, "lobby");
    tw.run(viewer, "+move 30,23");
    tw.run(viewer, "+move 33,13");
    viewer
}

#[test]
fn an_unlocked_crossing_still_walks() {
    let mut tw = lot();
    let breakroom = room_named(&tw, "Breakroom");
    let viewer = player_at_the_breakroom_door(&mut tw);
    let events = tw.run(viewer, "+move east 8");
    assert_eq!(
        tw.world().get(viewer).unwrap().location,
        Some(breakroom),
        "an open doorway is walked through: {}",
        text(&tw, &events, viewer)
    );
}

#[test]
fn a_module_veto_stops_the_step_at_the_boundary() {
    let wet = WetFloor::default();
    let mut mods = modules();
    mods.insert(0, Box::new(Veto(wet.clone())));
    let mut tw = TestWorld::with_modules(mods);
    let lobby = room_named(&tw, "Lobby");
    let breakroom = room_named(&tw, "Breakroom");
    *wet.0.lock().expect("lock") = Some(breakroom);
    let viewer = player_at_the_breakroom_door(&mut tw);
    let events = tw.run(viewer, "+move east 8");
    let output = text(&tw, &events, viewer);
    assert_eq!(
        tw.world().get(viewer).unwrap().location,
        Some(lobby),
        "the veto chain gates the step: {output}"
    );
    assert!(output.contains("The floor is wet"), "{output}");
    // The narrative move is refused by the same hook, in the same words.
    let events = tw.run(viewer, "breakroom");
    assert!(text(&tw, &events, viewer).contains("The floor is wet"));
}

#[test]
fn the_coordinate_form_stops_short_of_a_locked_boundary() {
    let mut tw = lot();
    let lobby = room_named(&tw, "Lobby");
    let breakroom = room_named(&tw, "Breakroom");
    let viewer = player_at_the_breakroom_door(&mut tw);
    let exit = exit_between(&tw, lobby, breakroom);
    tw.world_mut().get_mut(exit).unwrap().locks.insert(
        mush_core::LockType::Basic,
        mush_core::LockExpr::Const(false),
    );
    let events = tw.run(viewer, "+move 41,13");
    let output = text(&tw, &events, viewer);
    assert_eq!(
        tw.world().get(viewer).unwrap().location,
        Some(lobby),
        "a walk toward a cell stops at the locked doorway: {output}"
    );
    assert!(output.contains("You can't go that way."), "{output}");
}

#[test]
fn a_locked_doorway_refuses_in_the_exits_own_words() {
    let mut tw = lot();
    let lobby = room_named(&tw, "Lobby");
    let breakroom = room_named(&tw, "Breakroom");
    let viewer = player_at_the_breakroom_door(&mut tw);
    let exit = exit_between(&tw, lobby, breakroom);
    tw.world_mut().get_mut(exit).unwrap().locks.insert(
        mush_core::LockType::Basic,
        mush_core::LockExpr::Const(false),
    );
    tw.world_mut()
        .set_attr(
            exit,
            "fail",
            mush_core::Attr::text("The breakroom door is bolted from inside.", GOD),
        )
        .unwrap();
    let events = tw.run(viewer, "+move east 8");
    let output = text(&tw, &events, viewer);
    assert_eq!(tw.world().get(viewer).unwrap().location, Some(lobby));
    assert!(output.contains("bolted from inside"), "{output}");
}

/// God in the restricted Production Office, a window over it and the Lobby, a
/// player in the Lobby.
fn office_and_lobby(tw: &mut TestWorld) -> (DomainId, ObjId) {
    let lobby = room_named(tw, "Lobby");
    let office = room_named(tw, "Production Office");
    let forecourt = room_named(tw, "Forecourt & Bus Stop");
    tw.world_mut().move_to(GOD, office).unwrap();
    let (domain, _) = open_over(tw, GOD, &format!("#{},#{}", lobby.0, office.0));
    with_ctx(tw, GOD, move |ctx| {
        mush_atlas::window::place_in_room(ctx, domain, GOD, office, None)
    })
    .unwrap();
    let viewer = tw.spawn_player("Viewer");
    tw.world_mut().move_to(viewer, forecourt).unwrap();
    tw.run(viewer, "lobby");
    (domain, viewer)
}

#[test]
fn the_header_names_only_what_the_viewer_may_see() {
    let mut tw = lot();
    let (_, viewer) = office_and_lobby(&mut tw);
    let events = tw.run(viewer, "+map");
    let output = text(&tw, &events, viewer);
    assert!(output.contains("over Lobby"), "{output}");
    assert!(
        !output.contains("Production Office"),
        "the header named a room the frame hides: {output}"
    );
    let events = tw.run(GOD, "+map");
    let output = text(&tw, &events, GOD);
    assert!(output.contains("Lobby, Production Office"), "{output}");
}

#[test]
fn the_text_units_are_the_frames() {
    let mut tw = lot();
    let (domain, viewer) = office_and_lobby(&mut tw);
    // The player: one unit on their frame, so no list at all.
    let frame = mush_atlas::frame::frame_for(tw.world(), atlas(&tw), viewer, domain).unwrap();
    assert_eq!(frame.units.len(), 1);
    let events = tw.run(viewer, "+map");
    assert!(!text(&tw, &events, viewer).contains("On the map:"));
    // God, stepping down into the Lobby (a frame lists the units on the
    // viewer's own surface), sees the player on the map, at the cell the frame
    // gives — the text and the frame agree name for name.
    let lobby = room_named(&tw, "Lobby");
    with_ctx(&mut tw, GOD, move |ctx| {
        mush_atlas::window::place_in_room(ctx, domain, GOD, lobby, None)
    })
    .unwrap();
    let frame = mush_atlas::frame::frame_for(tw.world(), atlas(&tw), GOD, domain).unwrap();
    let unit = frame
        .units
        .iter()
        .find(|u| !u.you)
        .expect("God's frame shows the player");
    let events = tw.run(GOD, "+map");
    let output = text(&tw, &events, GOD);
    assert!(
        output.contains(&format!(
            "On the map: {} (cell {}, {}).",
            unit.name, unit.cell.0, unit.cell.1
        )),
        "{output}"
    );
}

/// Sign a test content pack JSON with its real digest.
fn signed_pack(mut doc: serde_json::Value) -> mush_content::Pack {
    doc["manifest"]["digest"] = serde_json::Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = mush_content::parse_pack(&unsigned)
        .expect("parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = serde_json::Value::String(digest);
    let text = serde_json::to_string(&doc).expect("json");
    mush_content::parse_pack(&text).expect("signed pack parses")
}

/// Authored test region + neighborhood with real `room == 0` courtyard gaps
/// between `Lobby`, `Vault`, `WetRoom`, and `RestrictedRoom` (`access: restricted`),
/// loaded through `ContentModule` with zero `module_state_mut` fiat writes.
fn gap_world(extra: Option<Box<dyn GameModule>>) -> TestWorld {
    let region = signed_pack(serde_json::json!({
        "manifest": { "id": "region.crit16", "version": 1, "kind": "region", "digest": "", "requires": [] },
        "entries": {
            "region.crit16": { "kind": "region", "def": {
                "name": "Crit16 Yard", "kind": "surface", "body": "earth",
                "extent": [[0, 0], [100, 100]], "datum_m": 0.0,
                "background": { "surface": "grass", "mix": [], "relief_cm": 0,
                                "relief_period_cm": 800, "seed": 1 },
                "ground_resolution_cm": 100 } },
            "locality.crit16": { "kind": "locality", "def": {
                "region": "region.crit16", "name": "Crit16 Quarter", "kind": "district",
                "footprint": [[0, 0], [60, 0], [60, 60], [0, 60]] } }
        }
    }));
    let hood = signed_pack(serde_json::json!({
        "manifest": { "id": "neighborhood.crit16", "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": "region.crit16", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.crit16", "locality": "locality.crit16", "name": "Courtyard",
                "setting": "rural", "footprint": [[0, 0], [60, 0], [60, 60], [0, 60]],
                "room_budget": { "target": 4, "maximum": 8 } } },
            "room.lobby": { "kind": "room", "def": {
                "name": "Lobby", "surface": "ground",
                "footprint": { "rect": [[10, 10], [20, 20]] },
                "access": "public" } },
            "room.vault": { "kind": "room", "def": {
                "name": "Vault", "surface": "ground",
                "footprint": { "rect": [[22, 10], [32, 20]] },
                "access": "public" } },
            "exit.vault": { "kind": "exit", "def": {
                "from": "room.lobby", "to": "room.vault",
                "names": ["Vault <V>;vault;east", "Lobby <L>;lobby;west"],
                "portal": { "distance_m": 2 } } },
            "room.wet": { "kind": "room", "def": {
                "name": "WetRoom", "surface": "ground",
                "footprint": { "rect": [[10, 24], [20, 34]] },
                "access": "public" } },
            "room.restricted": { "kind": "room", "def": {
                "name": "RestrictedRoom", "surface": "ground",
                "footprint": { "rect": [[22, 24], [32, 34]] },
                "access": "restricted" } }
        }
    }));
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    let content = ContentModule::new()
        .with_appliers(appliers)
        .with_boot_packs(vec![region, hood]);
    let mut mods: Vec<Box<dyn GameModule>> = Vec::new();
    if let Some(m) = extra {
        mods.push(m);
    }
    mods.push(Box::new(Probe));
    mods.push(Box::new(mush_npc::NpcModule::new()));
    mods.push(Box::new(SimulationModule::new()));
    mods.push(Box::new(AtlasModule::new()));
    mods.push(Box::new(content));
    TestWorld::with_modules(mods)
}

/// CRIT-16: crossing a `room == 0` gap between two rooms must retain `last_room` so
/// exit `Basic` locks on the crossing between the prior room and the destination room
/// cannot be skipped.
#[test]
fn crit16_locked_exit_cannot_be_skipped_across_room_zero_gap() {
    let mut tw = gap_world(None);
    let lobby = room_named(&tw, "Lobby");
    let vault = room_named(&tw, "Vault");
    let viewer = tw.spawn_player("Tester");
    tw.world_mut().move_to(GOD, lobby).expect("move god");
    let (domain, _) = open_over(&mut tw, GOD, "here");
    tw.run(GOD, &format!("@teleport #{}=#{}", viewer.0, lobby.0));
    let (_, base_pose) = mush_sim::pose_of(tw.world(), viewer).expect("viewer pose");

    // Place viewer near Lobby's east edge (x = 19.5 m, y = 15.5 m) inside Lobby.
    let start_pose = mush_sim::Pose {
        x_cm: 1_950,
        y_cm: 1_550,
        z_cm: 0,
        heading_mdeg: 90_000,
        ..base_pose
    };
    with_ctx(&mut tw, viewer, move |ctx| {
        mush_sim::place(ctx, viewer, domain, start_pose).expect("place viewer");
    });

    // Lock the exit from Lobby -> Vault with `Basic = false`.
    let exit = exit_between(&tw, lobby, vault);
    tw.world_mut().get_mut(exit).unwrap().locks.insert(
        mush_core::LockType::Basic,
        mush_core::LockExpr::Const(false),
    );
    tw.world_mut()
        .set_attr(
            exit,
            "fail",
            mush_core::Attr::text("The vault door is bolted tight.", GOD),
        )
        .unwrap();

    // Attempt to cross Lobby (x=10..20) -> authored room==0 gap (x=20..22) -> Vault (x=22..32).
    let events = tw.run(viewer, "+move east 6");
    let output = text(&tw, &events, viewer);
    assert_eq!(
        tw.world().get(viewer).unwrap().location,
        Some(lobby),
        "must NOT enter Vault across room == 0 gap: {output}"
    );
    assert!(
        output.contains("bolted tight"),
        "exit Basic lock must still fire across room == 0 gap: {output}"
    );
}

/// CRIT-16: `resolve_room` excludes `Access::Restricted` and locked rooms from the
/// nearest-room fallback on `room == 0` cells, and `step()` authorizes the resolved
/// room (blocking hook-denied fallback rooms) BEFORE calling `mush_sim::place`.
#[test]
fn crit16_resolve_room_excludes_restricted_and_locked_and_step_blocks_hook_denied_fallback() {
    let wet = WetFloor::default();
    let mut tw = gap_world(Some(Box::new(Veto(wet.clone()))));
    let lobby = room_named(&tw, "Lobby");
    let vault = room_named(&tw, "Vault");
    let wet_room = room_named(&tw, "WetRoom");
    let restricted = room_named(&tw, "RestrictedRoom");
    let viewer = tw.spawn_player("Tester");
    tw.world_mut().move_to(GOD, lobby).expect("move god");
    let (domain, _) = open_over(&mut tw, GOD, "here");
    tw.run(GOD, &format!("@teleport #{}=#{}", viewer.0, lobby.0));
    let (_, base_pose) = mush_sim::pose_of(tw.world(), viewer).expect("viewer pose");

    // (1) On `room == 0` cell (27.5 m, 23.5 m), nearest room is `RestrictedRoom` (y=24..34,
    // 0.5 m away vs Vault at y=20, 3.5 m away). `Access::Restricted` MUST be excluded.
    let near_restricted = mush_sim::Pose {
        x_cm: 2_750,
        y_cm: 2_350,
        z_cm: 0,
        heading_mdeg: 0,
        ..base_pose.clone()
    };
    assert_ne!(
        mush_atlas::resolve_room(tw.world(), domain, &near_restricted),
        Some(restricted),
        "Restricted room must be excluded from resolve_room fallback"
    );

    // (2) Lock Lobby -> Vault; on `room == 0` cell (21.5 m, 15.5 m), nearest room is
    // `Vault` (x=22, 0.5 m away vs Lobby at x=20, 1.5 m away). Locked-entry `Vault`
    // MUST be excluded from nearest-room fallback.
    let exit = exit_between(&tw, lobby, vault);
    tw.world_mut().get_mut(exit).unwrap().locks.insert(
        mush_core::LockType::Basic,
        mush_core::LockExpr::Const(false),
    );
    let near_locked_vault = mush_sim::Pose {
        x_cm: 2_150,
        y_cm: 1_550,
        z_cm: 0,
        heading_mdeg: 0,
        ..base_pose.clone()
    };
    assert_ne!(
        mush_atlas::resolve_room(tw.world(), domain, &near_locked_vault),
        Some(vault),
        "locked-entry room must be excluded from resolve_room fallback"
    );

    // (3) Hook-denied room: place viewer near Lobby's north edge (15.5 m, 19.5 m) and
    // veto `WetRoom` via `MoveAttempt`. Stepping north 3.5 m lands on `room == 0` cell
    // (15.5 m, 23.0 m) whose fallback resolves to `WetRoom` (y=24, 1 m away vs Lobby at
    // y=20, 3 m away); `step()` MUST refuse before `mush_sim::place` moves the viewer!
    let north_edge = mush_sim::Pose {
        x_cm: 1_550,
        y_cm: 1_950,
        z_cm: 0,
        heading_mdeg: 0,
        ..base_pose
    };
    with_ctx(&mut tw, viewer, move |ctx| {
        mush_sim::place(ctx, viewer, domain, north_edge).expect("place viewer");
    });
    *wet.0.lock().expect("lock") = Some(wet_room);
    let events = tw.run(viewer, "+move north 4");
    let output = text(&tw, &events, viewer);
    assert_eq!(
        tw.world().get(viewer).unwrap().location,
        Some(lobby),
        "step() must NOT project viewer into hook-denied fallback room: {output}"
    );
    assert!(
        output.contains("The floor is wet"),
        "MoveAttempt hook refusal must surface when fallback resolves to denied room: {output}"
    );
}
