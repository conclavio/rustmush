//! S1–S7 review regressions: map visibility, locked movement, and private NPC
//! interface replies. These assert the expected behavior and fail at d713d17.
//! See docs/design/14a-spatial-s1-s7-code-review.md for the reproduced defects.

use std::sync::{Arc, Mutex};

use mush_atlas::{AtlasModule, AtlasState, starter};
use mush_content::{Appliers, ContentModule};
use mush_core::module::{ModError, Registrar};
use mush_core::{
    ArgShape, Audience, CmdError, CommandSpec, Ctx, Effect, Effects, GOD, GameModule, Inbound,
    ObjId, ObjKind, Outbox, WorldCommand,
};
use mush_proto::OutputEvent;
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

#[test]
fn review_map_text_must_hide_private_participants() {
    let mut tw = lot();
    let lobby = room_named(&tw, "Lobby");
    let office = room_named(&tw, "Production Office");
    let forecourt = room_named(&tw, "Forecourt & Bus Stop");
    tw.world_mut().move_to(GOD, office).unwrap();
    let (domain, _) = open_over(&mut tw, GOD, &format!("#{},#{}", lobby.0, office.0));
    with_ctx(&mut tw, GOD, move |ctx| {
        mush_atlas::window::place_in_room(ctx, domain, GOD, office, None)
    })
    .unwrap();
    let viewer = tw.spawn_player("Viewer");
    tw.world_mut().move_to(viewer, forecourt).unwrap();
    tw.run(viewer, "lobby");
    let frame = mush_atlas::frame::frame_for(tw.world(), atlas(&tw), viewer, domain).unwrap();
    assert_eq!(frame.units.len(), 1, "filtered frame correctly hides staff");
    let events = tw.run(viewer, "+map");
    let output = text(&tw, &events, viewer);
    assert!(
        !output.contains("On the map:"),
        "private participant leaked: {output}"
    );
}

#[test]
fn review_tactical_move_must_respect_the_locked_breakroom_exit() {
    let mut tw = lot();
    let lobby = room_named(&tw, "Lobby");
    let breakroom = room_named(&tw, "Breakroom");
    let forecourt = room_named(&tw, "Forecourt & Bus Stop");
    tw.world_mut().move_to(GOD, lobby).unwrap();
    let _ = open_over(&mut tw, GOD, "here");
    let viewer = tw.spawn_player("Viewer");
    tw.world_mut().move_to(viewer, forecourt).unwrap();
    tw.run(viewer, "lobby");
    let exit = tw
        .world()
        .iter()
        .find(|o| {
            o.kind == ObjKind::Exit && o.location == Some(lobby) && o.destination == Some(breakroom)
        })
        .unwrap()
        .id;
    tw.world_mut().get_mut(exit).unwrap().locks.insert(
        mush_core::LockType::Basic,
        mush_core::LockExpr::Const(false),
    );
    let events = tw.run(viewer, "breakroom");
    assert_eq!(tw.world().get(viewer).unwrap().location, Some(lobby));
    println!("Narrative move: {}", text(&tw, &events, viewer));
    tw.run(viewer, "+move 30,23");
    tw.run(viewer, "+move 33,13");
    let events = tw.run(viewer, "+move east 8");
    assert_eq!(
        tw.world().get(viewer).unwrap().location,
        Some(lobby),
        "tactical movement bypasses locked exit: {}",
        text(&tw, &events, viewer)
    );
}

#[test]
fn review_interface_reply_must_keep_request_titles_private() {
    let mut mods = modules();
    mods.insert(1, Box::new(mush_jobs::JobsModule::new()));
    let mut tw = TestWorld::with_modules(mods);
    let lobby = room_named(&tw, "Lobby");
    let alice = tw.spawn_player("Alice");
    let bob = tw.spawn_player("Bob");
    tw.world_mut().move_to(alice, lobby).unwrap();
    tw.world_mut().move_to(bob, lobby).unwrap();
    let npc = tw
        .world_mut()
        .create(ObjKind::Npc, "FrontDesk", GOD, Some(lobby))
        .unwrap();
    let profile = mush_npc::AgentProfile {
        interfaces: vec!["request".into()],
        ..mush_npc::AgentProfile::default()
    };
    tw.world_mut()
        .set_attr(
            npc,
            mush_npc::PROFILE_ATTR,
            mush_core::Attr::json(serde_json::to_value(profile).unwrap(), GOD),
        )
        .unwrap();
    tw.run(
        alice,
        "+request Confidential review of Alice's character=Private notes for staff.",
    );
    assert!(mush_jobs::get_job(tw.world(), 1).is_some());
    let bob_lookup = tw.run(bob, "+job 1");
    assert!(!text(&tw, &bob_lookup, bob).contains("Confidential review"));
    tw.world_mut()
        .module_state_mut::<mush_npc::NpcHostState>()
        .unwrap()
        .begin_turn(npc, 1, 42);
    let payload = serde_json::json!({
        "status": "ok", "npc": npc, "asker": alice,
        "actions": [{ "action": { "action": "use_ability", "name": "request.status", "input": {} } }]
    }).to_string();
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(
        Inbound::Apply(WorldCommand::ModuleAsyncDone {
            module: "npc".into(),
            actor: npc,
            correlation: 42,
            payload,
        }),
        &mut out,
        &mut fx,
    );
    let events = out.take();
    assert!(
        text(&tw, &events, alice).contains("Confidential review"),
        "NPC must answer the owner"
    );
    assert!(
        !text(&tw, &events, bob).contains("Confidential review"),
        "bystander received a private request title: {}",
        text(&tw, &events, bob)
    );
}
