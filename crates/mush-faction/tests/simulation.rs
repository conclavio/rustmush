//! Faction simulation: the fronts/clocks loop through the real `Engine` — a
//! canon event advances a matching clock (the scene-outcome → clock "missing
//! middle"), completion fires a consequence (a canon event + a territory flip),
//! staff drive clocks, visibility gates the player view, and it all round-trips.

use std::collections::HashMap;

use mush_canon::{CanonModule, CanonState};
use mush_comsys::ComsysModule;
use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::module::emit_world_event;
use mush_core::{
    GOD, LIMBO, ModuleBootData, ModuleOp, ObjId, ObjKind, Outbox, Registry, World, WorldEvent,
    register_core_commands, register_engine_commands,
};
use mush_faction::{ControlKind, FactionModule, FactionState, SimState, Visibility, NS};
use mush_proto::OutputEvent;
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use smol_str::SmolStr;

fn boot() -> (Engine, ObjId, ObjId) {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let nick = world.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
    let bob = world.create(ObjKind::Player, "Bob", GOD, Some(LIMBO)).unwrap();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine = Engine::with_modules(
        world,
        registry,
        vec![
            Box::new(ComsysModule::new()),
            Box::new(CanonModule::new()),
            Box::new(FactionModule::new()),
        ],
    )
    .unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.modules_loaded(HashMap::new(), &mut out, &mut fx).unwrap();
    let _ = engine.take_module_ops();
    (engine, nick, bob)
}

fn run(engine: &mut Engine, actor: ObjId, line: &str) -> Outbox {
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(Inbound::Cmd { conn: 1, actor, line: line.into() }, &mut out, &mut fx);
    out
}

fn texts(out: &Outbox) -> String {
    out.events()
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Sys(t) => Some(t.to_plain_text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn sim(engine: &Engine) -> &SimState {
    engine.world.module_state::<SimState>().unwrap()
}

fn emit_canon(engine: &mut Engine, kind: &str, entities: &[ObjId]) {
    let mut out = Outbox::new();
    emit_world_event(
        &engine.registry,
        &mut engine.world,
        &mut out,
        &WorldEvent::CanonRecorded {
            canon_id: 0,
            kind: kind.into(),
            entities: entities.to_vec(),
        },
    );
}

/// Set up a faction (Undertow) with Nick a member, a region, a front, and a
/// clock that watches `scene.concluded` and flips the waterfront on completion.
fn setup_loop(engine: &mut Engine, nick: ObjId) {
    run(engine, GOD, "@faction/create Undertow");
    run(engine, GOD, "@faction/add Undertow=Nick");
    run(engine, GOD, "@region/create The Waterfront");
    run(engine, GOD, "@front/create Undertow=Seize the waterfront");
    run(engine, GOD, "@clock/create 0=Waterfront falls/2");
    run(engine, GOD, "@clock/set 0=on:scene.concluded");
    run(engine, GOD, "@clock/set 0=control:The Waterfront/defacto");
    run(engine, GOD, "@clock/set 0=says:The Undertow seize the waterfront.");
    let _ = nick;
}

#[test]
fn a_canon_event_advances_a_watching_clock_and_completion_fires_the_consequence() {
    let (mut engine, nick, bob) = boot();
    setup_loop(&mut engine, nick);
    let clock0 = mush_faction::ClockId(0);

    // An unrelated member's scene does NOT advance it (relevance gate).
    emit_canon(&mut engine, "scene.concluded", &[bob]);
    assert_eq!(sim(&engine).clocks[&clock0].filled, 0, "irrelevant scene ignored");

    // A scene involving the faction's member advances it by one.
    emit_canon(&mut engine, "scene.concluded", &[nick]);
    assert_eq!(sim(&engine).clocks[&clock0].filled, 1);
    assert!(!sim(&engine).clocks[&clock0].completed);

    // The second advance completes it: the consequence fires.
    emit_canon(&mut engine, "scene.concluded", &[nick]);
    let clock = &sim(&engine).clocks[&clock0];
    assert!(clock.completed, "clock filled and completed");

    // Completion flipped the territory: Undertow now holds the waterfront.
    let fac = engine.world.module_state::<FactionState>().unwrap();
    let region = fac.region_by_name("The Waterfront").unwrap();
    let recs = fac.control.get(&region.id).unwrap();
    assert!(
        recs.iter().any(|r| r.kind == ControlKind::DeFacto),
        "control flipped on completion: {recs:?}"
    );

    // And recorded a canon consequence (front.progress), so the chronicle knows.
    let canon = engine.world.module_state::<CanonState>().unwrap();
    assert!(
        canon.recent().iter().any(|e| e.kind == "front.progress"
            && e.summary.contains("seize the waterfront")),
        "consequence recorded in canon"
    );

    // A further canon event does not re-complete or re-advance it.
    emit_canon(&mut engine, "scene.concluded", &[nick]);
    assert_eq!(sim(&engine).clocks[&clock0].filled, 2, "completed clock frozen");
}

#[test]
fn a_covert_clock_flip_stays_covert_not_public() {
    // Regression: fire_consequence used to hardcode the flipped control record
    // to Public, leaking a covert operation's territorial result to everyone.
    let (mut engine, nick, bob) = boot();
    setup_loop(&mut engine, nick);
    run(&mut engine, GOD, "@clock/set 0=vis:covert");
    run(&mut engine, GOD, "@clock/advance 0=2"); // completes → flips territory

    // The control record inherits the clock's covert visibility, not Public.
    let fac = engine.world.module_state::<FactionState>().unwrap();
    let region = fac.region_by_name("The Waterfront").unwrap();
    let rec = fac.control[&region.id]
        .iter()
        .find(|r| r.kind == ControlKind::DeFacto)
        .expect("flip happened");
    assert_eq!(rec.visibility, Visibility::Covert, "covert op → covert hold");

    // And a non-member bystander sees no holder on +territory (the leak closed).
    let out = run(&mut engine, bob, "+territory");
    assert!(
        !texts(&out).contains("Undertow"),
        "covert hold leaked its holder to a bystander: {}",
        texts(&out)
    );
}

#[test]
fn the_consequence_kind_is_reserved_from_watching() {
    // Regression: a clock watching the consequence kind could cascade. @clock/set
    // must refuse it (which is what actually bounds the canon→clock chain depth).
    let (mut engine, nick, _bob) = boot();
    setup_loop(&mut engine, nick);
    let out = run(&mut engine, GOD, "@clock/set 0=on:front.progress");
    assert!(texts(&out).to_lowercase().contains("reserved"), "{}", texts(&out));
    // The watch was not changed.
    assert_eq!(
        sim(&engine).clocks[&mush_faction::ClockId(0)].advance_on.as_deref(),
        Some("scene.concluded"),
    );
}

#[test]
fn destroying_a_faction_prunes_its_fronts_and_clocks() {
    // Regression: faction-destroy cascaded FactionState but left SimState orphans.
    let (mut engine, nick, _bob) = boot();
    setup_loop(&mut engine, nick);
    assert_eq!(sim(&engine).fronts.len(), 1);
    assert_eq!(sim(&engine).clocks.len(), 1);
    run(&mut engine, GOD, "@faction/destroy Undertow");
    assert!(sim(&engine).fronts.is_empty(), "fronts pruned with the faction");
    assert!(sim(&engine).clocks.is_empty(), "clocks pruned with the faction");
}

#[test]
fn destroying_a_region_detaches_its_clock_flip() {
    // Regression: a destroyed region left clocks pointing at a gone region, whose
    // flip then silently no-ops. Destroy should null the consequence instead.
    let (mut engine, nick, _bob) = boot();
    setup_loop(&mut engine, nick);
    assert!(sim(&engine).clocks[&mush_faction::ClockId(0)].on_complete_control.is_some());
    run(&mut engine, GOD, "@region/destroy The Waterfront");
    assert!(
        sim(&engine).clocks[&mush_faction::ClockId(0)].on_complete_control.is_none(),
        "clock flip detached from the destroyed region",
    );
}

#[test]
fn staff_can_advance_a_clock_to_completion() {
    let (mut engine, nick, _bob) = boot();
    setup_loop(&mut engine, nick);
    let out = run(&mut engine, GOD, "@clock/advance 0=2");
    assert!(texts(&out).contains("COMPLETE"), "{}", texts(&out));
    assert!(sim(&engine).clocks[&mush_faction::ClockId(0)].completed);
}

#[test]
fn clocks_are_visibility_gated_for_players() {
    let (mut engine, nick, bob) = boot();
    setup_loop(&mut engine, nick); // clock defaults to OfficersKnow
    // Nick is a member but level 1 (not an officer, default officer_level 2),
    // so an OfficersKnow clock is hidden from him.
    let out = run(&mut engine, nick, "+clocks");
    assert!(texts(&out).contains("no active faction clocks"), "member sees officer clock? {}", texts(&out));
    // Make it Public: now Nick (and Bob) see it.
    run(&mut engine, GOD, "@clock/set 0=vis:public");
    assert!(texts(&run(&mut engine, nick, "+clocks")).contains("Waterfront falls"));
    assert!(texts(&run(&mut engine, bob, "+clocks")).contains("Waterfront falls"));
}

#[test]
fn fronts_and_clocks_survive_a_restart() {
    let (mut engine, nick, _bob) = boot();
    let mut ops: Vec<ModuleOp> = Vec::new();
    setup_loop(&mut engine, nick);
    run(&mut engine, GOD, "@clock/advance 0=1");
    ops.extend(engine.take_module_ops());

    let store = InMemoryStore::new();
    futures::executor::block_on(async {
        store
            .write(WriteBatch { serial: 1, module_ops: ops, ..WriteBatch::default() })
            .await
            .unwrap();
    });
    let kv = futures::executor::block_on(async { store.module_json_list(NS).await.unwrap() });
    let boot_data = ModuleBootData {
        kv: kv.into_iter().map(|(k, v)| (SmolStr::new(k), v)).collect(),
        streams: HashMap::new(),
    };
    let mut world = World::new_minimal();
    world.set_now(2_000);
    world.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
    world.create(ObjKind::Player, "Bob", GOD, Some(LIMBO)).unwrap();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine2 = Engine::with_modules(
        world,
        registry,
        vec![
            Box::new(ComsysModule::new()),
            Box::new(CanonModule::new()),
            Box::new(FactionModule::new()),
        ],
    )
    .unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine2
        .modules_loaded(HashMap::from([(SmolStr::new(NS), boot_data)]), &mut out, &mut fx)
        .unwrap();

    let s = sim(&engine2);
    assert_eq!(s.fronts.len(), 1, "front rehydrated");
    let clock = &s.clocks[&mush_faction::ClockId(0)];
    assert_eq!(clock.filled, 1, "clock progress preserved");
    assert_eq!(clock.advance_on.as_deref(), Some("scene.concluded"));
    assert_eq!(s.next_clock_id, 1, "allocator resumes");
}
