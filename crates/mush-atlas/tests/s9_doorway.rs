//! S9.4 (S5's note): a line between two surfaces of one window runs through
//! the doorway they share — a building's ground floor and the lot outside —
//! where before it was answered `None`. A floor with no way down still is.

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

fn finish(tw: &mut TestWorld, actor: ObjId, effects: &[Effect]) {
    let (correlation, payload) = effects
        .iter()
        .find_map(|e| match e {
            Effect::ModuleAsync {
                module,
                correlation,
                payload,
                ..
            } if module.as_str() == mush_atlas::NS => Some((*correlation, payload.clone())),
            _ => None,
        })
        .expect("one deferred composite");
    let outcome = mush_atlas::run_async(&payload);
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(
        Inbound::Apply(WorldCommand::ModuleAsyncDone {
            module: mush_atlas::NS.into(),
            actor,
            correlation,
            payload: outcome,
        }),
        &mut out,
        &mut fx,
    );
}

fn atlas(tw: &TestWorld) -> &AtlasState {
    tw.world()
        .module_state::<AtlasState>()
        .expect("atlas state")
}

/// A window over the Lobby (a building's ground floor) and the Forecourt (the
/// lot's ground): God inside, a player outside.
fn inside_and_outside(tw: &mut TestWorld) -> (DomainId, ObjId, ObjId) {
    let lobby = room_named(tw, "Lobby");
    let forecourt = room_named(tw, "Forecourt & Bus Stop");
    tw.world_mut().move_to(GOD, lobby).unwrap();
    let (events, effects) = step(
        tw,
        GOD,
        &format!("@atlas/window open here,#{}", forecourt.0),
    );
    let t = text(tw, &events, GOD);
    assert!(t.contains("opens; its map is being drawn"), "{t}");
    let domain = atlas(tw)
        .windows
        .open
        .keys()
        .max()
        .copied()
        .map(DomainId)
        .expect("a window entry");
    finish(tw, GOD, &effects);
    let outside = tw.spawn_player("Outside");
    tw.world_mut().move_to(outside, forecourt).unwrap();
    with_ctx(tw, GOD, move |ctx| {
        mush_atlas::window::place_in_room(ctx, domain, GOD, lobby, None)?;
        mush_atlas::window::place_in_room(ctx, domain, outside, forecourt, None)
    })
    .unwrap();
    (domain, GOD, outside)
}

#[test]
fn a_line_runs_from_the_lobby_floor_through_its_door_onto_the_lot() {
    let mut tw = TestWorld::with_modules(modules());
    let (_, inside, outside) = inside_and_outside(&mut tw);
    // Before S9.4 every cross-surface answer was `None` or "occluded at the
    // grid's range"; now the doorway carries the line.
    let sight = mush_atlas::window::sight_between(tw.world(), inside, outside);
    assert!(sight.is_some(), "the door joins the floor to the lot");
    let metric = mush_sim::metric_between(tw.world(), inside, outside).expect("a metric");
    assert!(metric.range_cells >= 1, "{metric:?}");
    let hearing = mush_atlas::window::hearing_between(tw.world(), inside, outside);
    assert!(hearing.is_some());
    let fire = mush_atlas::window::direct_fire_between(tw.world(), inside, outside);
    assert!(fire.is_some());
    // An arc across surfaces is still no arc.
    assert_eq!(
        mush_sim::arc_between(tw.world(), inside, outside),
        Some(false)
    );
}

#[test]
fn the_line_and_the_metric_agree_with_each_other() {
    let mut tw = TestWorld::with_modules(modules());
    let (_, inside, outside) = inside_and_outside(&mut tw);
    let sight = mush_atlas::window::sight_between(tw.world(), inside, outside).expect("a sight");
    let metric = mush_sim::metric_between(tw.world(), inside, outside).expect("a metric");
    assert_eq!(metric.occluded, !sight.is_clear_or_partial());
    // The same answer both ways round.
    let back = mush_atlas::window::sight_between(tw.world(), outside, inside).expect("a sight");
    assert_eq!(back.is_clear_or_partial(), sight.is_clear_or_partial());
}
