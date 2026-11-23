//! Faction simulation: the fronts/clocks loop through the real `Engine` — a
//! canon event advances a matching clock (the scene-outcome → clock "missing
//! middle"), completion fires a consequence (a canon event + a territory flip),
//! staff drive clocks, visibility gates the player view, and it all round-trips.

use std::collections::HashMap;
use std::sync::Arc;

use mush_canon::{CanonModule, CanonState};
use mush_comsys::ComsysModule;
use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::module::emit_world_event;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, GOD, GameModule, LIMBO, ModError, ModuleBootData,
    ModuleOp, ObjId, ObjKind, Outbox, Registrar, Registry, World, WorldEvent,
    register_core_commands, register_engine_commands,
};
use mush_faction::{ControlKind, FactionModule, FactionState, NS, SimState, Visibility};
use mush_proto::OutputEvent;
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use smol_str::SmolStr;

/// A store-less lens for this harness: it drives the engine's public
/// event fan-out with no live typed-state snapshot and says so.
const DETACHED: mush_core::TypedStateLens<'static> =
    mush_core::TypedStateLens::Detached(mush_core::DetachedTypedState::Test);

fn boot() -> (Engine, ObjId, ObjId) {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let bob = world
        .create(ObjKind::Player, "Bob", GOD, Some(LIMBO))
        .unwrap();
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
            Box::new(Owner),
        ],
    )
    .unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .unwrap();
    let _ = engine.take_module_ops();
    (engine, nick, bob)
}

fn run(engine: &mut Engine, actor: ObjId, line: &str) -> Outbox {
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::Cmd {
            conn: 1,
            actor,
            line: line.into(),
        },
        &mut out,
        &mut fx,
    );
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
            source: String::new(),
        },
        DETACHED,
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
    run(
        engine,
        GOD,
        "@clock/set 0=says:The Undertow seize the waterfront.",
    );
    let _ = nick;
}

#[test]
fn a_canon_event_advances_a_watching_clock_and_completion_fires_the_consequence() {
    let (mut engine, nick, bob) = boot();
    setup_loop(&mut engine, nick);
    let clock0 = mush_faction::ClockId(0);

    // An unrelated member's scene does NOT advance it (relevance gate).
    emit_canon(&mut engine, "scene.concluded", &[bob]);
    assert_eq!(
        sim(&engine).clocks[&clock0].filled,
        0,
        "irrelevant scene ignored"
    );

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
        canon
            .recent()
            .iter()
            .any(|e| e.kind == "front.progress" && e.summary.contains("seize the waterfront")),
        "consequence recorded in canon"
    );

    // A further canon event does not re-complete or re-advance it.
    emit_canon(&mut engine, "scene.concluded", &[nick]);
    assert_eq!(
        sim(&engine).clocks[&clock0].filled,
        2,
        "completed clock frozen"
    );
}

/// Doc 17h cut M8d D7: a clock watching the Red Dies kind (`minds.unwarranted`) advances on
/// a line that names nobody — the member gate is skipped for a world kind — while a clock on
/// an ordinary kind stays gated.
#[test]
fn m8d_a_world_kind_advances_its_clock_without_naming_a_member() {
    let (mut engine, nick, _bob) = boot();
    setup_loop(&mut engine, nick);
    run(&mut engine, GOD, "@faction/create Republic");
    run(&mut engine, GOD, "@front/create Republic=The Red Dies");
    run(&mut engine, GOD, "@clock/create 1=Ungoverned cores found/6");
    run(&mut engine, GOD, "@clock/set 1=on:minds.unwarranted");
    let gated = mush_faction::ClockId(0);
    let red = mush_faction::ClockId(1);
    assert_eq!(
        sim(&engine).clocks[&red].advance_on.as_deref(),
        Some(mush_faction::RED_DIES_KIND)
    );
    emit_canon(&mut engine, mush_faction::RED_DIES_KIND, &[]);
    assert_eq!(sim(&engine).clocks[&red].filled, 1, "one step on");
    assert!(!sim(&engine).clocks[&red].completed);
    emit_canon(&mut engine, "scene.concluded", &[]);
    assert_eq!(
        sim(&engine).clocks[&gated].filled,
        0,
        "ordinary kinds stay gated"
    );
    assert_eq!(sim(&engine).clocks[&red].filled, 1);
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
    assert_eq!(
        rec.visibility,
        Visibility::Covert,
        "covert op → covert hold"
    );

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
    assert!(
        texts(&out).to_lowercase().contains("reserved"),
        "{}",
        texts(&out)
    );
    // The watch was not changed.
    assert_eq!(
        sim(&engine).clocks[&mush_faction::ClockId(0)]
            .advance_on
            .as_deref(),
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
    assert!(
        sim(&engine).fronts.is_empty(),
        "fronts pruned with the faction"
    );
    assert!(
        sim(&engine).clocks.is_empty(),
        "clocks pruned with the faction"
    );
}

#[test]
fn destroying_a_region_detaches_its_clock_flip() {
    // Regression: a destroyed region left clocks pointing at a gone region, whose
    // flip then silently no-ops. Destroy should null the consequence instead.
    let (mut engine, nick, _bob) = boot();
    setup_loop(&mut engine, nick);
    assert!(
        sim(&engine).clocks[&mush_faction::ClockId(0)]
            .on_complete_control
            .is_some()
    );
    run(&mut engine, GOD, "@region/destroy The Waterfront");
    assert!(
        sim(&engine).clocks[&mush_faction::ClockId(0)]
            .on_complete_control
            .is_none(),
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
    assert!(
        texts(&out).contains("no active faction clocks"),
        "member sees officer clock? {}",
        texts(&out)
    );
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
            .write(WriteBatch {
                serial: 1,
                module_ops: ops,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
    });
    let kv = futures::executor::block_on(async { store.module_json_list(NS).await.unwrap() });
    let boot_data = ModuleBootData {
        kv: kv.into_iter().map(|(k, v)| (SmolStr::new(k), v)).collect(),
        streams: HashMap::new(),
        ..Default::default()
    };
    let mut world = World::new_minimal();
    world.set_now(2_000);
    world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    world
        .create(ObjKind::Player, "Bob", GOD, Some(LIMBO))
        .unwrap();
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
        .modules_loaded(
            HashMap::from([(SmolStr::new(NS), boot_data)]),
            &mut out,
            &mut fx,
        )
        .unwrap();

    let s = sim(&engine2);
    assert_eq!(s.fronts.len(), 1, "front rehydrated");
    let clock = &s.clocks[&mush_faction::ClockId(0)];
    assert_eq!(clock.filled, 1, "clock progress preserved");
    assert_eq!(clock.advance_on.as_deref(), Some("scene.concluded"));
    assert_eq!(s.next_clock_id, 1, "allocator resumes");
}

// ----------------------------------------------------------------------
// Doc 50 cut KC.0a (Q-50-50, Q-50-51): one de facto holder, the IC date of a hold, the public
// control write, one completion path, a front reopened and a clock set, and a polity's clock
// moved by a typed owner call that carries a cause id.
// ----------------------------------------------------------------------

/// A compiled owner standing in for TacSim: it reaches the faction engine only through its
/// public calls.
struct Owner;

fn arg_u64(s: &str) -> Result<u64, CmdError> {
    s.trim().parse().map_err(|_| CmdError::msg("a number"))
}

fn kc_control(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let rhs = ctx.args.rhs().unwrap_or_default().to_owned();
    let (fname, kind) = rhs.split_once('/').ok_or_else(|| CmdError::msg("spec"))?;
    let (region, faction) = {
        let st = ctx
            .w
            .module_state::<FactionState>()
            .ok_or_else(|| CmdError::msg("state"))?;
        let region = st
            .region_by_name(ctx.args.lhs())
            .ok_or_else(|| CmdError::msg("region"))?
            .id;
        let faction = st
            .faction_by_name(fname)
            .ok_or_else(|| CmdError::msg("faction"))?
            .id;
        (region, faction)
    };
    let kind = ControlKind::parse(kind).ok_or_else(|| CmdError::msg("kind"))?;
    let change =
        mush_faction::write_control(ctx, mush_faction::ControlWrite::new(region, faction, kind))
            .map_err(CmdError::msg)?;
    ctx.out.sys_to_actor(
        ctx.actor,
        "test",
        format!("began={} displaced={:?}", change.began, change.displaced),
    );
    Ok(())
}

fn kc_cause(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let clock = mush_faction::ClockId(arg_u64(ctx.args.lhs())?);
    let rhs = ctx.args.rhs().unwrap_or_default().to_owned();
    let (by, cause) = rhs.split_once('/').ok_or_else(|| CmdError::msg("spec"))?;
    let by = u8::try_from(arg_u64(by)?).map_err(|_| CmdError::msg("by"))?;
    let draft = mush_canon::CanonDraft {
        kind: "kc.cause".into(),
        summary: format!("cause {cause}"),
        entities: Vec::new(),
        scope: mush_canon::CanonScope::Public,
        ic_time: ctx.w.game_now_unix(),
        source: cause.to_owned(),
        detail: serde_json::Value::Null,
    };
    let r = mush_faction::advance_clock_by_cause(ctx, clock, by, draft).map_err(CmdError::msg)?;
    ctx.out.sys_to_actor(ctx.actor, "test", format!("{r:?}"));
    Ok(())
}

fn kc_reopen(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let front = mush_faction::FrontId(arg_u64(ctx.args.lhs())?);
    mush_faction::reopen_front(ctx, front).map_err(CmdError::msg)?;
    Ok(())
}

fn kc_set(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let clock = mush_faction::ClockId(arg_u64(ctx.args.lhs())?);
    let n = arg_u64(ctx.args.rhs().unwrap_or_default())?;
    let n = u8::try_from(n).map_err(|_| CmdError::msg("n"))?;
    mush_faction::set_clock(ctx, clock, n).map_err(CmdError::msg)?;
    Ok(())
}

impl GameModule for Owner {
    fn name(&self) -> &'static str {
        "kc0a_owner"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        let spec = |name: &str| CommandSpec {
            name: name.into(),
            shape: ArgShape::EqPair,
            ..CommandSpec::default()
        };
        reg.command(spec("kc-control"), Arc::new(kc_control))?;
        reg.command(spec("kc-cause"), Arc::new(kc_cause))?;
        reg.command(spec("kc-reopen"), Arc::new(kc_reopen))?;
        reg.command(spec("kc-set"), Arc::new(kc_set))?;
        Ok(())
    }
}

fn faction_state(engine: &Engine) -> &FactionState {
    engine.world.module_state::<FactionState>().unwrap()
}

fn waterfront(engine: &Engine) -> mush_faction::RegionId {
    faction_state(engine)
        .region_by_name("The Waterfront")
        .unwrap()
        .id
}

fn holds(engine: &Engine, kind: ControlKind) -> Vec<String> {
    let fac = faction_state(engine);
    fac.control
        .get(&waterfront(engine))
        .map(|recs| {
            recs.iter()
                .filter(|r| r.kind == kind)
                .map(|r| fac.factions[&r.faction].name.to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn canon_count(engine: &Engine, kind: &str) -> usize {
    engine
        .world
        .module_state::<CanonState>()
        .unwrap()
        .recent()
        .iter()
        .filter(|e| e.kind == kind)
        .count()
}

/// Q-50-50: a filled clock's flip moves the prior de facto holder out. Its de jure claim stays.
#[test]
fn kc0a_a_clock_flip_moves_the_prior_de_facto_holder_out() {
    let (mut engine, nick, _bob) = boot();
    setup_loop(&mut engine, nick);
    run(&mut engine, GOD, "@faction/create Freeport");
    run(
        &mut engine,
        GOD,
        "@faction/control The Waterfront=Freeport/defacto",
    );
    run(
        &mut engine,
        GOD,
        "@faction/control The Waterfront=Freeport/dejure",
    );
    run(&mut engine, GOD, "@clock/advance 0=2");
    assert_eq!(holds(&engine, ControlKind::DeFacto), vec!["Undertow"]);
    assert_eq!(holds(&engine, ControlKind::DeJure), vec!["Freeport"]);
    assert_eq!(
        faction_state(&engine)
            .de_facto(waterfront(&engine))
            .map(|r| r.faction),
        faction_state(&engine)
            .faction_by_name("Undertow")
            .map(|f| f.id)
    );
}

/// Q-50-50: a staff flip moves the prior de facto holder out too, and says so; de jure stays
/// plural.
#[test]
fn kc0a_a_staff_flip_moves_the_prior_de_facto_holder_out() {
    let (mut engine, nick, _bob) = boot();
    setup_loop(&mut engine, nick);
    run(&mut engine, GOD, "@faction/create Freeport");
    run(
        &mut engine,
        GOD,
        "@faction/control The Waterfront=Freeport/defacto",
    );
    run(
        &mut engine,
        GOD,
        "@faction/control The Waterfront=Freeport/dejure",
    );
    run(
        &mut engine,
        GOD,
        "@faction/control The Waterfront=Undertow/dejure",
    );
    let out = run(
        &mut engine,
        GOD,
        "@faction/control The Waterfront=Undertow/defacto",
    );
    assert!(
        texts(&out).contains("Freeport no longer holds it de facto"),
        "{}",
        texts(&out)
    );
    assert_eq!(holds(&engine, ControlKind::DeFacto), vec!["Undertow"]);
    assert_eq!(holds(&engine, ControlKind::DeJure).len(), 2);
}

/// Q-50-50 and Q-50-9: a hold carries the IC date it began, from the world's clock; a holder
/// that goes on holding keeps its date; `+territory` shows it.
#[test]
fn kc0a_a_hold_is_dated_in_ic_time_and_territory_shows_it() {
    let (mut engine, nick, bob) = boot();
    // The live 24:1 rate, phased to 2350: IC time is nowhere near wall time.
    engine.world.set_game_clock(mush_core::GameClock {
        ic_ratio: 24,
        epoch_offset: 2_350 * 365 * 86_400,
    });
    setup_loop(&mut engine, nick);
    run(&mut engine, GOD, "@faction/create Freeport");
    run(
        &mut engine,
        GOD,
        "@faction/control The Waterfront=Freeport/defacto",
    );
    let first_ic = engine.world.game_now_unix();
    assert_ne!(first_ic, engine.world.now());
    let rec = faction_state(&engine)
        .de_facto(waterfront(&engine))
        .cloned()
        .unwrap();
    assert_eq!(rec.since_ic, first_ic, "the IC clock, not the wall clock");
    assert!(rec.since_date().ends_with(", 2350"), "{}", rec.since_date());
    let out = texts(&run(&mut engine, bob, "+territory"));
    assert!(
        out.contains(&format!("Freeport (de facto since {})", rec.since_date())),
        "{out}"
    );

    // Wall time moves thirty days; the world's clock maps it to IC time.
    let later = engine.world.now() + 30 * 86_400;
    engine.world.set_now(later);
    assert!(engine.world.game_now_unix() > first_ic);
    run(
        &mut engine,
        GOD,
        "@faction/control The Waterfront=Freeport/defacto/strength:4",
    );
    assert_eq!(
        faction_state(&engine)
            .de_facto(waterfront(&engine))
            .unwrap()
            .since_ic,
        first_ic,
        "the same holder keeps the date its hold began"
    );
    run(&mut engine, GOD, "@clock/advance 0=2");
    let flipped = faction_state(&engine)
        .de_facto(waterfront(&engine))
        .cloned()
        .unwrap();
    assert_eq!(flipped.since_ic, engine.world.game_now_unix());
    assert!(flipped.since_ic > first_ic, "the flip dates the new hold");
}

/// Q-50-50: the one public control write seeds and flips.
#[test]
fn kc0a_the_public_control_write_seeds_and_flips() {
    let (mut engine, nick, _bob) = boot();
    setup_loop(&mut engine, nick);
    run(&mut engine, GOD, "@faction/create Freeport");
    let _ = engine.take_module_ops();
    let out = texts(&run(
        &mut engine,
        GOD,
        "kc-control The Waterfront=Freeport/defacto",
    ));
    assert!(out.contains("began=true displaced=[]"), "{out}");
    let out = texts(&run(
        &mut engine,
        GOD,
        "kc-control The Waterfront=Undertow/defacto",
    ));
    assert!(out.contains("began=true displaced=[FactionId(1)]"), "{out}");
    let out = texts(&run(
        &mut engine,
        GOD,
        "kc-control The Waterfront=Undertow/defacto",
    ));
    assert!(out.contains("began=false displaced=[]"), "{out}");
    assert_eq!(holds(&engine, ControlKind::DeFacto), vec!["Undertow"]);
    let ops = engine.take_module_ops();
    assert!(
        ops.iter().any(|op| matches!(
            &op.op,
            mush_core::ModuleWrite::JsonSet { key, .. } if key.starts_with("control:")
        )),
        "the write persisted the region's control document"
    );
}

/// Q-50-51: the engine's path marks the front resolved when its LAST clock fills, not before.
#[test]
fn kc0a_the_engine_path_resolves_the_front_when_its_last_clock_fills() {
    let (mut engine, nick, _bob) = boot();
    setup_loop(&mut engine, nick);
    run(&mut engine, GOD, "@clock/create 0=Docks secured/1");
    let front = mush_faction::FrontId(0);
    emit_canon(&mut engine, "scene.concluded", &[nick]);
    emit_canon(&mut engine, "scene.concluded", &[nick]);
    assert!(sim(&engine).clocks[&mush_faction::ClockId(0)].completed);
    assert!(!sim(&engine).fronts[&front].resolved, "a clock still runs");
    run(&mut engine, GOD, "@clock/advance 1");
    assert!(
        sim(&engine).fronts[&front].resolved,
        "the last clock filled"
    );
}

/// Q-50-51 and Q-50-31: a resolved front reopens, a filled clock is set back, and it climbs and
/// fires again, by the owner calls and by the staff verbs.
#[test]
fn kc0a_a_front_reopens_and_a_clock_set_back_fills_and_fires_again() {
    let (mut engine, nick, _bob) = boot();
    setup_loop(&mut engine, nick);
    let (front, clock) = (mush_faction::FrontId(0), mush_faction::ClockId(0));
    run(&mut engine, GOD, "@clock/advance 0=2");
    assert!(sim(&engine).fronts[&front].resolved);
    assert_eq!(canon_count(&engine, "front.progress"), 1);

    // The owner calls: set back to 1 of 2. The front is still resolved, so canon moves nothing.
    run(&mut engine, GOD, "kc-set 0=1");
    assert!(!sim(&engine).clocks[&clock].completed);
    assert_eq!(sim(&engine).clocks[&clock].filled, 1);
    emit_canon(&mut engine, "scene.concluded", &[nick]);
    assert_eq!(
        sim(&engine).clocks[&clock].filled,
        1,
        "a resolved front stays frozen"
    );
    run(&mut engine, GOD, "kc-reopen 0");
    assert!(!sim(&engine).fronts[&front].resolved);
    emit_canon(&mut engine, "scene.concluded", &[nick]);
    assert!(sim(&engine).clocks[&clock].completed, "it climbed again");
    assert_eq!(canon_count(&engine, "front.progress"), 2, "and fired again");
    assert!(sim(&engine).fronts[&front].resolved);

    // The staff verbs do the same.
    let out = texts(&run(&mut engine, GOD, "@clock/set 0=filled:2"));
    assert!(out.contains("advance it to fill it"), "{out}");
    let out = texts(&run(&mut engine, GOD, "@clock/set 0=filled:0"));
    assert!(out.contains("set to 0/2"), "{out}");
    let out = texts(&run(&mut engine, GOD, "@front/reopen 0"));
    assert!(out.contains("is open"), "{out}");
    assert!(!sim(&engine).fronts[&front].resolved);
    assert!(!sim(&engine).clocks[&clock].completed);
}

/// Q-50-51: a polity's clock moves by a typed owner call carrying a cause id, through the one
/// completion path; a cause counted once is not counted again; a clock bound to a canon kind
/// refuses the call; and canon alone does not move the polity's clock.
#[test]
fn kc0a_a_polity_clock_moves_by_a_typed_cause() {
    let (mut engine, nick, _bob) = boot();
    setup_loop(&mut engine, nick);
    run(&mut engine, GOD, "@faction/create Freeport");
    run(
        &mut engine,
        GOD,
        "@faction/control The Waterfront=Freeport/defacto",
    );
    run(
        &mut engine,
        GOD,
        "@front/create Undertow=Terms at the docks",
    );
    run(&mut engine, GOD, "@clock/create 1=Dock terms/2");
    run(
        &mut engine,
        GOD,
        "@clock/set 1=control:The Waterfront/defacto",
    );
    let (front, clock) = (mush_faction::FrontId(1), mush_faction::ClockId(1));

    let out = texts(&run(&mut engine, GOD, "kc-cause 1=1/seizure-7"));
    assert!(out.contains("filled: 1, completed: false"), "{out}");
    let out = texts(&run(&mut engine, GOD, "kc-cause 1=1/seizure-7"));
    assert!(out.contains("AlreadyCounted"), "{out}");
    assert_eq!(sim(&engine).clocks[&clock].filled, 1);
    assert_eq!(
        canon_count(&engine, "kc.cause"),
        1,
        "a retried cause writes no second line"
    );

    // Canon naming a member does not move it: it watches no kind.
    emit_canon(&mut engine, "kc.cause", &[nick]);
    emit_canon(&mut engine, "scene.concluded", &[nick]);
    assert_eq!(sim(&engine).clocks[&clock].filled, 1);

    let out = texts(&run(&mut engine, GOD, "kc-cause 1=1/seizure-8"));
    assert!(out.contains("filled: 2, completed: true"), "{out}");
    assert_eq!(holds(&engine, ControlKind::DeFacto), vec!["Undertow"]);
    assert!(sim(&engine).fronts[&front].resolved);
    assert_eq!(
        sim(&engine).clocks[&clock].causes,
        vec![SmolStr::new("seizure-7"), SmolStr::new("seizure-8")]
    );

    // A completed clock refuses; a clock bound to a canon kind refuses.
    let out = texts(&run(&mut engine, GOD, "kc-cause 1=1/seizure-9"));
    assert!(out.contains("completed"), "{out}");
    let before = sim(&engine).clocks[&mush_faction::ClockId(0)].filled;
    let out = texts(&run(&mut engine, GOD, "kc-cause 0=1/scene-1"));
    assert!(out.contains("canon kind"), "{out}");
    assert_eq!(
        sim(&engine).clocks[&mush_faction::ClockId(0)].filled,
        before
    );
}

/// P2-MOD-39: `+clocks` filters hidden factions out for non-staff non-members,
/// even when the clock's own visibility is `Visibility::Public`.
#[test]
fn p2_mod_39_clocks_hides_hidden_factions_from_non_members() {
    let (mut engine, nick, bob) = boot();
    run(&mut engine, GOD, "@faction/create ShadowCabal");
    run(&mut engine, GOD, "@faction/set ShadowCabal=hidden:yes");
    run(&mut engine, GOD, "@faction/add ShadowCabal=Nick");
    run(&mut engine, GOD, "@front/create ShadowCabal=Covert Plot");
    run(&mut engine, GOD, "@clock/create 0=Secret Coup/4");
    run(&mut engine, GOD, "@clock/set 0=vis:public");
    run(&mut engine, GOD, "@clock/advance 0=2");

    // Non-member Bob must NOT see the hidden faction's name, clock, or progress.
    let bob_out = texts(&run(&mut engine, bob, "+clocks"));
    assert!(
        !bob_out.contains("ShadowCabal") && !bob_out.contains("Secret Coup"),
        "hidden faction clock leaked to non-member: {bob_out}"
    );
    assert!(bob_out.contains("no active faction clocks"), "{bob_out}");

    // Member Nick and staff GOD still see the clock.
    let nick_out = texts(&run(&mut engine, nick, "+clocks"));
    assert!(
        nick_out.contains("Secret Coup") && nick_out.contains("ShadowCabal"),
        "member should see hidden faction clock: {nick_out}"
    );
    let staff_out = texts(&run(&mut engine, GOD, "+clocks"));
    assert!(
        staff_out.contains("Secret Coup"),
        "staff should see hidden faction clock: {staff_out}"
    );
}
