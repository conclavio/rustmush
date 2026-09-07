//! E6b (B6/B7): **the conferred sealer, the sealed outcome, and the IC floor.**
//!
//! The rhythms here are the ones 08f §4.5 asks for:
//!
//! - **fail closed.** A scene with no conferred sealer produces no outcome at all —
//!   for staff too — because the alternative is that anyone who can start a scene can
//!   end somebody's run.
//! - **written once, terminal.** A second seal is refused; the record is never edited.
//! - **rule 2 (08i §4).** Nothing may let an NPC's completion seal a settlement, at
//!   conferral or at seal.
//! - **the clock never re-clocks.** An `ic_floor` raise is monotonic and never writes
//!   `clock_generation`; generation one lasts the record's life (08i §9 answer 9).

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::module::{EventHook, HookPoint, HookReadContext};
use mush_core::output::Audience;
use mush_core::{
    ClockProviderFailure, ClockReadError, ClockRef, Flags, GOD, GameModule, ModError, ObjId,
    ObjKind, Outbox, Perceive, Registrar, Registry, World, register_core_commands,
    register_engine_commands,
};
use mush_proto::{OutputEvent, RequestKind, RespondChoice};
use mush_scenes::{CONSENT_DEADLINE_S, PauseReason, SceneStatus, ScenesModule, ScenesState};

// ---- rig ------------------------------------------------------------------

struct Rig {
    engine: Engine,
    bar: ObjId,
    nick: ObjId,
    ann: ObjId,
    npc: ObjId,
}

/// A `Perceive` hook that hides one object from everyone — the scene tests'
/// stand-in for a concealment ruleset.
struct Veil(ObjId);

impl EventHook for Veil {
    fn perceives(&self, _ctx: &HookReadContext<'_>, _observer: ObjId, subject: ObjId) -> Perceive {
        if subject == self.0 {
            Perceive::Conceal
        } else {
            Perceive::Abstain
        }
    }
}

struct VeilModule(ObjId);

impl GameModule for VeilModule {
    fn name(&self) -> &'static str {
        "veil"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.hook(HookPoint::Perceive, Arc::new(Veil(self.0)));
        Ok(())
    }
}

fn rig() -> Rig {
    rig_with(|_nick, _ann| Vec::new())
}

/// [`rig`] with extra modules composed beside canon and scenes; the closure
/// receives Nick's and Ann's ids so a module can be pointed at one of them.
fn rig_with(extra: impl FnOnce(ObjId, ObjId) -> Vec<Box<dyn GameModule>>) -> Rig {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let bar = world
        .create(ObjKind::Room, "The Tap Room", GOD, None)
        .unwrap();
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(bar))
        .unwrap();
    let ann = world
        .create(ObjKind::Player, "Ann", GOD, Some(bar))
        .unwrap();
    let npc = world
        .create(ObjKind::Npc, "Barkeep", GOD, Some(bar))
        .unwrap();
    world.set_connected(nick, true);
    world.set_connected(ann, true);
    world.set_live_conns(nick, 1);
    world.set_live_conns(ann, 1);
    // Staff stand in the room so the staff verbs resolve `here`.
    world.move_to(GOD, bar).unwrap();

    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(mush_canon::CanonModule::new()),
        Box::new(ScenesModule::new()),
    ];
    modules.extend(extra(nick, ann));
    let mut engine = Engine::with_modules(world, registry, modules).unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .unwrap();
    let _ = engine.take_module_ops();
    Rig {
        engine,
        bar,
        nick,
        ann,
        npc,
    }
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
            OutputEvent::Narrative { text, .. } => Some(text.to_plain_text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn scenes(engine: &Engine) -> &ScenesState {
    engine
        .world
        .module_state::<ScenesState>()
        .expect("scenes state")
}

/// The one live scene in the bar.
fn scene(r: &Rig) -> mush_scenes::SceneRecord {
    scenes(&r.engine)
        .latest_scene_at(r.bar)
        .cloned()
        .expect("a scene")
}

/// Every canon record the world holds right now.
fn canon(engine: &Engine) -> Vec<mush_canon::CanonEvent> {
    engine
        .world
        .module_state::<mush_canon::CanonState>()
        .map(mush_canon::CanonState::recent)
        .unwrap_or_default()
}

/// Fold every scenes write since the last drain into one **last-write-wins** kv map
/// and hand it back to the module — a reboot, exactly as the store would replay it.
fn reboot(r: &mut Rig) {
    let mut kv: Vec<(smol_str::SmolStr, serde_json::Value)> = Vec::new();
    for op in r.engine.take_module_ops() {
        if op.ns.as_str() != mush_scenes::NS {
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
    let mut boot: HashMap<smol_str::SmolStr, mush_core::ModuleBootData> = HashMap::new();
    boot.insert(
        smol_str::SmolStr::new(mush_scenes::NS),
        mush_core::ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    );
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    r.engine.modules_loaded(boot, &mut out, &mut fx).unwrap();
}

/// Start a scene and confer the seal on `who`, bound to run 7 (E6b verify fold,
/// M2/R12: staff bind **which run** a scene may seal, and the seal may name no
/// other).
fn scened(r: &mut Rig, sealer: ObjId) {
    run(&mut r.engine, r.nick, "+timestop");
    let id = scene(r).id;
    let out = run(
        &mut r.engine,
        GOD,
        &format!("@scene/facilitate {}=#{} run=7", id.0, sealer.0),
    );
    assert!(
        texts(&out).contains("may now seal"),
        "conferral refused: {}",
        texts(&out)
    );
}

// ---------------------------------------------------------------------------
// E6b: B6 — the conferred sealer
// ---------------------------------------------------------------------------

/// §4.1: a scene with **no conferred sealer cannot be sealed** — by anyone, staff
/// included. Fail-closed exactly like the empty canon-kind table it replaces, so a
/// contract gated on the seal simply never settles.
#[test]
fn e6b_b6_a_scene_with_no_conferred_sealer_cannot_be_sealed_by_anyone() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    for who in [r.nick, GOD] {
        let out = run(&mut r.engine, who, "+scene/outcome success=Nick run=7");
        assert!(
            texts(&out).contains("cannot seal this scene")
                || texts(&out).contains("No one has been conferred the seal"),
            "the seal was not refused for #{}: {}",
            who.0,
            texts(&out)
        );
    }
    assert!(scene(&r).outcome.is_none());
    // And nothing reached canon at all.
    assert!(canon(&r.engine).iter().all(|e| e.kind != "scene.outcome"));
}

/// Only the conferred sealer (or staff) may seal: a participant who was not
/// conferred it is refused, even though they can drive the scene.
#[test]
fn e6b_b6_a_non_sealer_non_staff_player_is_refused() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick);
    let out = run(&mut r.engine, r.ann, "+scene/outcome success=Nick run=7");
    // E6b verify fold (D9): one refusal, the same one an unconferred scene gives, so
    // a participant cannot read a staff conferral off the difference.
    assert!(
        texts(&out).contains("cannot seal this scene"),
        "{}",
        texts(&out)
    );
    assert!(scene(&r).outcome.is_none());
}

/// **Written once, terminal, never edited.** A second seal is refused and the first
/// record stands unchanged.
#[test]
fn e6b_b6_a_second_seal_is_refused_and_the_first_stands() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick);
    run(
        &mut r.engine,
        r.nick,
        "+scene/outcome success=Nick run=7 note=they got the paydata",
    );
    let first = scene(&r).outcome.clone().expect("a sealed outcome");
    assert_eq!(first.name.as_str(), "success");
    let out = run(&mut r.engine, r.nick, "+scene/outcome failure=Nick run=7");
    assert!(texts(&out).contains("already sealed"), "{}", texts(&out));
    assert_eq!(scene(&r).outcome.as_ref(), Some(&first));
    // Exactly one `scene.outcome` record.
    assert_eq!(
        canon(&r.engine)
            .iter()
            .filter(|e| e.kind == "scene.outcome")
            .count(),
        1
    );
}

/// **Rule 2 (08i §4): nothing may let an NPC's completion seal a settlement.**
/// Refused at conferral *and* at seal — the second is not redundant, because an
/// object could in principle change kind between the two.
#[test]
fn e6b_b6_an_npc_sealer_is_refused_at_conferral_and_at_seal() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let id = scene(&r).id;
    let out = run(
        &mut r.engine,
        GOD,
        &format!("@scene/facilitate {}=#{}", id.0, r.npc.0),
    );
    assert!(
        texts(&out).contains("Only a player may be conferred"),
        "{}",
        texts(&out)
    );
    assert!(scene(&r).sealer.is_none());

    // Now force the record into the shape conferral refuses and prove the seal
    // refuses it too: the gate is asked twice on purpose.
    let npc = r.npc;
    {
        let st = r
            .engine
            .world
            .module_state_mut::<ScenesState>()
            .expect("scenes state");
        let rec = st.scenes.get_mut(&id).expect("the scene");
        rec.sealer = Some(npc);
    }
    let out = run(&mut r.engine, GOD, "+scene/outcome success=Nick run=7");
    assert!(
        texts(&out).contains("Only a player may seal"),
        "{}",
        texts(&out)
    );
    assert!(scene(&r).outcome.is_none());
}

/// §4.1: `kind`, `source` and `entities` are **exactly** what the economy matches,
/// and `entities` is `subjects ∪ {sealed_by}` **named at seal time** — never copied
/// from `participants`, which is frozen at scene creation.
#[test]
fn e6b_b7_the_seal_records_the_kind_the_run_source_and_the_named_subjects() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick);
    // Ann is a participant; the seal names only Nick, so Ann is NOT in `entities`.
    run(&mut r.engine, r.nick, "+scene/outcome success=Nick run=7");
    let ev = canon(&r.engine)
        .into_iter()
        .find(|e| e.kind == "scene.outcome")
        .expect("a sealed outcome reached canon");
    assert_eq!(ev.source, "run:7#success");
    assert_eq!(ev.scope, mush_canon::CanonScope::Restricted);
    assert!(ev.entities.contains(&r.nick), "the subject is named");
    assert!(
        !ev.entities.contains(&r.ann),
        "a participant who is not a subject is NOT named: {:?}",
        ev.entities
    );
    assert_eq!(ev.detail["outcome"], "success");
    assert_eq!(ev.detail["sealed_by"], r.nick.0);
}

/// A runner who joined **mid-scene** is named in `subjects` and matches; one the
/// seal omits does not. This is the whole reason `entities` is named at seal time:
/// `participants` is frozen at creation with no add path.
#[test]
fn e6b_b7_a_mid_scene_arrival_is_named_by_the_seal_and_an_omitted_one_is_not() {
    let mut r = rig();
    // Zed arrives after the scene starts, so he is not a participant.
    let zed = r
        .engine
        .world
        .create(ObjKind::Player, "Zed", GOD, None)
        .unwrap();
    let nick = r.nick;
    scened(&mut r, nick);
    assert!(!scene(&r).participants.contains(&zed));
    r.engine.world.move_to(zed, r.bar).unwrap();

    run(
        &mut r.engine,
        r.nick,
        "+scene/outcome success=Nick, Zed run=7",
    );
    let ev = canon(&r.engine)
        .into_iter()
        .find(|e| e.kind == "scene.outcome")
        .expect("a sealed outcome");
    assert!(ev.entities.contains(&zed), "the arrival is named");
    assert!(!ev.entities.contains(&r.ann), "the omitted one is not");
}

/// `scene.concluded` is a **lifecycle** record any participant can produce by ending
/// a scene. It is never a `scene.outcome`, and the hazard test says so out loud: a
/// payee who ends the scene manufactures no seal.
#[test]
fn e6b_b7_a_payee_ending_the_scene_manufactures_no_seal() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "+timestop/end");
    let kinds: Vec<String> = canon(&r.engine)
        .iter()
        .map(|e| e.kind.to_string())
        .collect();
    assert!(
        kinds.iter().any(|k| k == "scene.concluded"),
        "the lifecycle record is still produced: {kinds:?}"
    );
    assert!(
        !kinds.iter().any(|k| k == "scene.outcome"),
        "ending a scene sealed an outcome: {kinds:?}"
    );
}

/// A **closed** scene can still be sealed (the run's paperwork often follows the
/// play), but a scene that never began cannot.
#[test]
fn e6b_b6_a_closed_scene_seals_and_a_preparing_one_does_not() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick);
    run(&mut r.engine, r.nick, "+timestop/end");
    assert_eq!(scene(&r).status, SceneStatus::Closed);
    let out = run(&mut r.engine, r.nick, "+scene/outcome success=Nick run=7");
    assert!(!texts(&out).contains("has not begun"), "{}", texts(&out));
    assert!(scene(&r).outcome.is_some());
}

// ---------------------------------------------------------------------------
// E6b: B6 — the IC floor (08i §9 answer 9, doc 10 §15/6A)
// ---------------------------------------------------------------------------

/// **OPEN 3, as decided.** The seal raises `ic_floor` to the world IC clock *first*
/// and then stamps `scene_ic_time()`, so the chronicle's `ic_time` is the world IC
/// clock **by construction** rather than by a second rule — and never the frozen
/// scene clock, which advances only `round_seconds × (rounds − 1)`.
#[test]
fn e6b_b6_the_seals_ic_time_is_the_world_ic_clock_not_the_frozen_scene_clock() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick);
    let frozen = scene(&r).scene_ic_time();
    // The world's IC clock runs on while the timestop holds its seconds.
    r.engine.world.set_now(1_000_000);
    let world_ic = r.engine.world.game_now_unix();
    assert!(
        world_ic > frozen,
        "the fixture must actually diverge: {world_ic} vs {frozen}"
    );
    run(&mut r.engine, r.nick, "+scene/outcome success=Nick run=7");
    let ev = canon(&r.engine)
        .into_iter()
        .find(|e| e.kind == "scene.outcome")
        .expect("a sealed outcome");
    assert_eq!(ev.ic_time, world_ic);
    assert_eq!(scene(&r).ic_floor, world_ic);
}

/// The floor is **monotonic**: a lower value is refused, and the record is untouched.
#[test]
fn e6b_b6_a_lower_ic_floor_is_refused() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let id = scene(&r).id;
    run(&mut r.engine, GOD, &format!("@scene/icfloor {}=5000", id.0));
    assert_eq!(scene(&r).ic_floor, 5_000);
    let out = run(&mut r.engine, GOD, &format!("@scene/icfloor {}=4000", id.0));
    assert!(
        texts(&out).contains("only moves forward"),
        "{}",
        texts(&out)
    );
    assert_eq!(scene(&r).ic_floor, 5_000);
    let out = run(&mut r.engine, GOD, &format!("@scene/icfloor {}=5000", id.0));
    assert!(
        texts(&out).contains("only moves forward"),
        "{}",
        texts(&out)
    );
}

/// 08i §9 answer 9: an `ic_floor` raise **never** writes `clock_generation`, which
/// lasts the record's lifetime, and no E6b verb or boot path writes it either.
#[test]
fn e6b_b6_no_e6b_path_ever_writes_the_clock_generation() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick);
    let id = scene(&r).id;
    let before = scene(&r).clock_generation;
    assert_eq!(before, 1);
    run(&mut r.engine, GOD, &format!("@scene/icfloor {}=9000", id.0));
    run(&mut r.engine, r.nick, "+timestop/next");
    run(&mut r.engine, r.nick, "+scene/outcome success=Nick run=7");
    run(&mut r.engine, GOD, &format!("@scene/icfloor {}=9500", id.0));
    assert_eq!(
        scene(&r).clock_generation,
        before,
        "an E6b path re-clocked the scene"
    );
}

/// 08i §9's explicit monotonicity test: a `TurnBased` scene driven through
/// `+timestop` start → `/next` → `/pause` (and the boot-time `ServerRestart` pause)
/// → `/resume` → `/next`, asserting `DurableSceneClock::now` is **non-decreasing**
/// throughout within `clock_generation` 1 — and that a read at generation 2 is
/// `StaleGeneration`.
#[test]
fn e6b_b6_the_scene_turn_clock_is_monotonic_across_the_whole_lifecycle() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let id = scene(&r).id;
    let generation = scene(&r).clock_generation;
    let read = |engine: &Engine| -> i64 {
        engine
            .clock_now(&ClockRef::SceneTurn {
                scene: id.0,
                generation,
            })
            .expect("the scene clock reads")
            .at
    };
    let mut seen = vec![read(&r.engine)];
    run(&mut r.engine, r.nick, "+timestop/next");
    seen.push(read(&r.engine));
    run(&mut r.engine, r.nick, "+timestop/pause");
    seen.push(read(&r.engine));

    // The boot-time pause: a reboot rehydrates every live scene as
    // `Paused(ServerRestart)`, and the clock must not move backwards through it.
    reboot(&mut r);
    assert_eq!(scene(&r).pause_reason, Some(PauseReason::ServerRestart));
    seen.push(read(&r.engine));

    run(&mut r.engine, r.nick, "+timestop/resume");
    seen.push(read(&r.engine));
    run(&mut r.engine, r.nick, "+timestop/next");
    seen.push(read(&r.engine));

    assert!(
        seen.windows(2).all(|w| w[1] >= w[0]),
        "the scene turn clock went backwards: {seen:?}"
    );
    assert_eq!(scene(&r).clock_generation, generation, "generation moved");
    // Any implicit generation mutation is rejected rather than silently answered.
    let stale = r.engine.clock_now(&ClockRef::SceneTurn {
        scene: id.0,
        generation: generation + 1,
    });
    assert!(
        matches!(
            stale,
            Err(ClockReadError::SceneProvider {
                source: ClockProviderFailure::StaleGeneration { .. },
                ..
            })
        ),
        "a read at the next generation should be StaleGeneration, got {stale:?}"
    );
}

/// The record's own invariants: a document claiming a sealed outcome with **no
/// sealer** is a forged seal and never loads, and a negative floor never loads.
#[test]
fn e6b_b6_validate_clock_refuses_a_forged_seal_and_a_negative_floor() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick);
    run(&mut r.engine, r.nick, "+scene/outcome success=Nick run=7");
    let mut rec = scene(&r);
    assert!(rec.validate_clock().is_ok());
    rec.sealer = None;
    assert!(
        rec.validate_clock().is_err(),
        "an outcome with no conferred sealer must not validate"
    );
    rec.sealer = Some(r.nick);
    rec.ic_floor = -1;
    assert!(rec.validate_clock().is_err());
}

/// The bounds a stored outcome re-runs on load (the E5a fold rule).
#[test]
fn e6b_b6_a_sealed_outcome_re_runs_its_bounds_on_load() {
    let too_long = "x".repeat(mush_scenes::MAX_OUTCOME_NOTE_BYTES + 1);
    let bad = serde_json::json!({
        "name": "success", "subjects": [2], "note": too_long,
        "sealed_by": 2, "sealed_at": 1,
    });
    assert!(serde_json::from_value::<mush_scenes::SceneOutcome>(bad).is_err());
    let empty = serde_json::json!({
        "name": "", "subjects": [2], "note": "", "sealed_by": 2, "sealed_at": 1,
    });
    assert!(serde_json::from_value::<mush_scenes::SceneOutcome>(empty).is_err());
    let nobody = serde_json::json!({
        "name": "success", "subjects": [], "note": "", "sealed_by": 2, "sealed_at": 1,
    });
    assert!(serde_json::from_value::<mush_scenes::SceneOutcome>(nobody).is_err());
    let good = serde_json::json!({
        "name": "success", "subjects": [2, 3], "note": "ok", "sealed_by": 2, "sealed_at": 1,
    });
    let outcome: mush_scenes::SceneOutcome =
        serde_json::from_value(good).expect("a well-formed seal loads");
    assert_eq!(outcome.subjects, vec![ObjId(2), ObjId(3)]);
}

/// The seal survives a reboot on its own document, and the floor with it.
#[test]
fn e6b_b6_the_seal_and_the_floor_survive_a_reboot() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick);
    r.engine.world.set_now(500_000);
    run(&mut r.engine, r.nick, "+scene/outcome success=Nick run=7");
    let before = scene(&r);
    reboot(&mut r);
    let after = scene(&r);
    assert_eq!(after.outcome, before.outcome);
    assert_eq!(after.sealer, before.sealer);
    assert_eq!(after.ic_floor, before.ic_floor);
    assert_eq!(after.clock_generation, before.clock_generation);
}

/// Conferral is **staff's**: a player cannot confer the seal on themselves, which is
/// the whole difference between a `sealer` and the `facilitator` any player becomes
/// by starting a scene.
#[test]
fn e6b_b6_a_runner_confers_nothing_on_themselves() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let id = scene(&r).id;
    let out = run(
        &mut r.engine,
        r.nick,
        &format!("@scene/facilitate {}=#{}", id.0, r.nick.0),
    );
    assert!(
        texts(&out).contains("Permission denied") || texts(&out).contains("Huh"),
        "a player conferred the seal on themselves: {}",
        texts(&out)
    );
    assert!(scene(&r).sealer.is_none());
    let _ = Arc::new(());
}

// ---- E6b verify fold ------------------------------------------------------

/// **M2/R12.** The seal names **the run staff bound to this scene**, and no other.
/// `run=` was free input, so a sealer conferred on any scene could name any contract
/// id, pick that contract's own parties as subjects, and settle a run they had
/// nothing to do with — the economy's three gates all pass on that.
#[test]
fn vf_m2_a_seal_names_only_the_run_it_was_bound_to() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick); // bound to run 7
    let out = run(&mut r.engine, r.nick, "+scene/outcome success=Nick run=99");
    assert!(texts(&out).contains("not for that run"), "{}", texts(&out));
    assert!(scene(&r).outcome.is_none());
    // The bound run seals, and the source names it.
    run(&mut r.engine, r.nick, "+scene/outcome success=Nick run=7");
    let ev = canon(&r.engine)
        .into_iter()
        .find(|e| e.kind == "scene.outcome")
        .expect("a sealed outcome");
    assert_eq!(ev.source, "run:7#success");
}

/// **M2, the chronicle half.** A scene bound to **no** run still seals — that is
/// what §4.1's "staff conferral remains for scenes no contract names" asks for —
/// and its record names a `scene:{uid}` source, which no contract can ever cite,
/// because a `scene:{uid}` is unknowable when a run is offered.
#[test]
fn vf_m2_an_unbound_scene_seals_for_the_chronicle_and_settles_nothing() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let id = scene(&r).id;
    run(
        &mut r.engine,
        GOD,
        &format!("@scene/facilitate {}=#{}", id.0, r.nick.0),
    );
    run(&mut r.engine, r.nick, "+scene/outcome success=Nick");
    let ev = canon(&r.engine)
        .into_iter()
        .find(|e| e.kind == "scene.outcome")
        .expect("a sealed outcome");
    assert_eq!(ev.source, format!("scene:{}", id.0));
}

/// **M5.** The row is in the source, so a seal selects the outcome it names rather
/// than every canon row at once.
#[test]
fn vf_m5_the_seals_source_names_the_row_it_answers() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick);
    run(&mut r.engine, r.nick, "+scene/outcome failure=Nick run=7");
    let ev = canon(&r.engine)
        .into_iter()
        .find(|e| e.kind == "scene.outcome")
        .expect("a sealed outcome");
    assert_eq!(
        ev.source, "run:7#failure",
        "the row the sealer named is not in the source, so it decides nothing"
    );
}

/// **R4.** The floor raise is monotonic and has **no inverse**, so it is bounded:
/// one pasted epoch-millis value would otherwise stamp every future record from this
/// scene at the year 55 000 for ever. And it is refused once the outcome is canon.
#[test]
fn vf_r4_the_ic_floor_cannot_be_raised_past_the_world_or_after_the_seal() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick);
    let id = scene(&r).id;
    let absurd = r.engine.world.game_now_unix() + mush_scenes::MAX_IC_FLOOR_LEAD_SECS + 1;
    let out = run(
        &mut r.engine,
        GOD,
        &format!("@scene/icfloor {}={absurd}", id.0),
    );
    assert!(texts(&out).contains("further ahead"), "{}", texts(&out));
    assert_eq!(scene(&r).ic_floor, 0);

    run(&mut r.engine, r.nick, "+scene/outcome success=Nick run=7");
    let sealed = scene(&r).ic_floor;
    let out = run(
        &mut r.engine,
        GOD,
        &format!("@scene/icfloor {}={}", id.0, sealed + 10),
    );
    assert!(texts(&out).contains("already sealed"), "{}", texts(&out));
    assert_eq!(scene(&r).ic_floor, sealed);
}

/// **D9.** One refusal. "Nobody was conferred the seal here" and "somebody was, and
/// it is not you" were two messages, and conferral is told only to the staffer who
/// made it — so any participant could read a staff conferral off the difference.
#[test]
fn vf_d9_the_seal_refusal_does_not_disclose_whether_one_was_conferred() {
    let unconferred = {
        let mut r = rig();
        run(&mut r.engine, r.nick, "+timestop");
        texts(&run(
            &mut r.engine,
            r.ann,
            "+scene/outcome success=Nick run=7",
        ))
    };
    let conferred = {
        let mut r = rig();
        let nick = r.nick;
        scened(&mut r, nick);
        texts(&run(
            &mut r.engine,
            r.ann,
            "+scene/outcome success=Nick run=7",
        ))
    };
    assert_eq!(
        unconferred, conferred,
        "the refusal tells a participant whether staff conferred the seal"
    );
}

/// **R3.** The three new fields are `#[serde(default)]`, so forward they need no
/// migration — but the version stamp is what makes a **rollback** refuse rather than
/// boot happily and erase a terminal sealed outcome on its first write-through.
#[test]
fn vf_r3_the_namespace_is_stamped_so_a_rollback_refuses() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick);
    run(&mut r.engine, r.nick, "+scene/outcome success=Nick run=7");
    // The record round-trips, and an **old** document (no E6b fields at all) still
    // loads with the pre-E6b behaviour — the forward half.
    let old = serde_json::json!({
        "id": 0, "title": "T", "kind": "timestop", "mode": "turn_based",
        "status": "active", "exclusive": true, "anchor": 10,
        "participants": [2], "facilitator": 2, "created_by": 2, "created_at": 1,
        "ic_started_at": { "clock": { "kind": "world_ic", "generation": 1 }, "at": 5 },
        "round_seconds": 3, "round": 1, "turn_index": 0,
    });
    let rec: mush_scenes::SceneRecord =
        serde_json::from_value(old).expect("a pre-E6b document still loads");
    assert!(rec.sealer.is_none());
    assert!(rec.outcome.is_none());
    assert_eq!(rec.ic_floor, 0);
    assert!(rec.validate_clock().is_ok());
    assert_eq!(rec.scene_ic_time(), 5, "the floor changes nothing at zero");
}

// ---------------------------------------------------------------------------
// E6b independent verify fold
// ---------------------------------------------------------------------------

/// **D3.** The chronicle line named the contract to a non-party. `scene.outcome` is
/// `Restricted`, and a `Restricted` record is shown to every entity it names — but
/// the entities are the seal's **subjects**, who need not be parties to the run. So
/// interpolating `run:{id}#{row}` into `summary` told a subject which contract paid
/// for the work and which row of its table was selected, off a chronicle read. The
/// source stays where it is load-bearing: `detail`, and the `CanonRecorded` event.
#[test]
fn vf2_d3_the_chronicle_summary_does_not_name_the_contract() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick);
    let out = run(&mut r.engine, r.nick, "+scene/outcome success=Ann run=7");
    assert!(texts(&out).contains("sealed"), "{}", texts(&out));
    let ev = canon(&r.engine)
        .into_iter()
        .find(|e| e.kind == mush_scenes::KIND_SCENE_OUTCOME)
        .expect("the seal reached canon");
    assert!(
        !ev.summary.contains("run:"),
        "the chronicle line names the run: {}",
        ev.summary
    );
    assert!(
        ev.summary.contains("success"),
        "the chronicle line lost the outcome it is for: {}",
        ev.summary
    );
    // The source itself is unchanged — it is the economy's whole match key.
    assert_eq!(ev.source, "run:7#success");
    assert_eq!(
        ev.detail.get("outcome").and_then(|v| v.as_str()),
        Some("success"),
        "the staff-facing detail lost the outcome: {}",
        ev.detail
    );
}

/// **R6.** A `SceneOutcome` that will not decode used to fail the whole
/// `SceneRecord`, and `load_boot` turned that into a `ModError` — so one over-long
/// note in one persisted outcome was a server that could not start. The outcome is
/// decoded leniently instead: the scene boots **unsealed**, keeps its conferred
/// sealer and its run binding, and the sealer can simply seal it again (the
/// economy's write-once slots make a second canon record for a filled slot inert).
#[test]
fn vf2_r6_a_corrupt_persisted_outcome_does_not_brick_the_boot() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick);
    let id = scene(&r).id;
    run(&mut r.engine, r.nick, "+scene/outcome success=Ann run=7");
    assert!(scene(&r).outcome.is_some(), "the seal did not land");

    // Corrupt exactly the outcome, the way an older build or a hand edit would: a
    // note past `MAX_OUTCOME_NOTE_BYTES`, which the wire refuses.
    let mut kv: Vec<(smol_str::SmolStr, serde_json::Value)> = Vec::new();
    for op in r.engine.take_module_ops() {
        if op.ns.as_str() != mush_scenes::NS {
            continue;
        }
        match op.op {
            mush_core::ModuleWrite::JsonSet { key, mut value } => {
                if key.as_str() == mush_scenes::scene_key(id).as_str()
                    && let Some(outcome) = value.get_mut("outcome").and_then(|o| o.as_object_mut())
                {
                    outcome.insert(
                        "note".to_owned(),
                        serde_json::Value::String(
                            "x".repeat(mush_scenes::MAX_OUTCOME_NOTE_BYTES + 1),
                        ),
                    );
                }
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            mush_core::ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            mush_core::ModuleWrite::StreamAppend { .. } => {}
        }
    }
    let mut boot: HashMap<smol_str::SmolStr, mush_core::ModuleBootData> = HashMap::new();
    boot.insert(
        smol_str::SmolStr::new(mush_scenes::NS),
        mush_core::ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    );
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    r.engine
        .modules_loaded(boot, &mut out, &mut fx)
        .expect("an unreadable outcome must not fail the boot");

    let rec = scenes(&r.engine)
        .scenes
        .get(&id)
        .cloned()
        .expect("the scene still loaded");
    assert!(
        rec.outcome.is_none(),
        "the unreadable outcome loaded anyway"
    );
    assert_eq!(
        rec.sealer,
        Some(r.nick),
        "the conferred sealer was lost with the outcome"
    );
    assert_eq!(rec.seal_run, Some(7), "the run binding was lost with it");
    // And the remedy really works: the sealer seals again.
    let again = run(&mut r.engine, r.nick, "+scene/outcome success=Ann run=7");
    assert!(texts(&again).contains("sealed"), "{}", texts(&again));
}

/// Programme D3 (interface plan I11): the player whose turn it becomes gets a
/// `scene` `Notice` — on `/next` and on `/resume`, never on `/pause` — the
/// client-neutral signal a Discord follow, a web toast or the SSH notice rail
/// all render. Only players are told.
#[test]
fn d3_the_player_whose_turn_it_is_gets_a_notice_on_next_and_resume_not_pause() {
    fn turn_notices(out: &Outbox) -> Vec<ObjId> {
        out.events()
            .iter()
            .filter_map(|(aud, e)| match (aud, e) {
                (
                    mush_core::output::Audience::Actor(a),
                    OutputEvent::Notice { category, text, .. },
                ) if category == "scene" && text.to_plain_text().starts_with("It is your turn") => {
                    Some(*a)
                }
                _ => None,
            })
            .collect()
    }
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let out = run(&mut r.engine, r.nick, "+timestop/next");
    let current = scene(&r).current_actor().expect("a turn order");
    assert!(
        r.engine
            .world
            .get(current)
            .is_some_and(|o| o.kind == ObjKind::Player),
        "the fixture's turn goes to a player"
    );
    assert_eq!(turn_notices(&out), vec![current], "{}", texts(&out));
    let out = run(&mut r.engine, r.nick, "+timestop/pause");
    assert!(turn_notices(&out).is_empty(), "a pause is not a turn");
    let out = run(&mut r.engine, r.nick, "+timestop/resume");
    assert_eq!(
        turn_notices(&out),
        vec![current],
        "resume re-announces the turn"
    );
}

// ---------------------------------------------------------------------------
// D6 — consent: the requests a scene opens and what their answers do
// ---------------------------------------------------------------------------

/// Every `Request` in `out`: (subject, id, kind).
fn requests(out: &Outbox) -> Vec<(ObjId, u64, RequestKind)> {
    out.events()
        .iter()
        .filter_map(|(aud, e)| match (aud, e) {
            (Audience::Actor(a), OutputEvent::Request { id, kind, .. }) => Some((*a, *id, *kind)),
            _ => None,
        })
        .collect()
}

fn respond(engine: &mut Engine, actor: ObjId, id: u64, choice: RespondChoice) -> Outbox {
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::Respond {
            conn: 1,
            actor,
            id,
            choice,
        },
        &mut out,
        &mut fx,
    );
    out
}

/// The recorded votes as (about, participant, choice).
fn votes(r: &Rig) -> Vec<(String, ObjId, String)> {
    scene(r)
        .consent
        .iter()
        .map(|v| (v.about.to_string(), v.participant, v.choice.to_string()))
        .collect()
}

/// Drive the scheduler to `unix_seconds` (the world clock first, so handlers
/// read the same time the timers fire at).
fn advance_to(r: &mut Rig, unix_seconds: i64) -> Outbox {
    r.engine.set_now(unix_seconds);
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    r.engine.fire_due(unix_seconds * 1000, &mut out, &mut fx);
    out
}

/// The `Sys` lines addressed to `who`.
fn sys_to(out: &Outbox, who: ObjId) -> String {
    out.events()
        .iter()
        .filter_map(|(aud, e)| match (aud, e) {
            (Audience::Actor(a), OutputEvent::Sys(t)) if *a == who => Some(t.to_plain_text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Ending asks each **player participant** exactly once — never the
/// facilitator whose command it is, never the NPC, never an observer.
#[test]
fn d6_ending_asks_each_player_participant_once_never_the_facilitator_npc_or_observer() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let uid = scene(&r).id;
    let watcher = r
        .engine
        .world
        .create(ObjKind::Player, "Watcher", GOD, Some(r.bar))
        .unwrap();
    r.engine.world.set_connected(watcher, true);
    r.engine
        .world
        .module_state_mut::<ScenesState>()
        .unwrap()
        .scenes
        .get_mut(&uid)
        .unwrap()
        .observers
        .push(watcher);
    r.engine.world.set_exclusive_scene_member(watcher);

    let out = run(&mut r.engine, r.nick, "+timestop/end");
    let asked = requests(&out);
    assert_eq!(
        asked.len(),
        1,
        "one request per player participant: {asked:?}"
    );
    assert_eq!(asked[0].0, r.ann, "Ann, the only other player participant");
    assert_eq!(asked[0].2, RequestKind::Confirm);
    assert_eq!(
        scene(&r).status,
        SceneStatus::Closed,
        "the end waited for no one"
    );
    assert!(
        votes(&r).is_empty(),
        "nothing is recorded until someone answers"
    );
}

/// The Confirm prompts `out` delivered to `who`, as plain text.
fn prompts_to(out: &Outbox, who: ObjId) -> Vec<String> {
    out.events()
        .iter()
        .filter_map(|(aud, e)| match (aud, e) {
            (Audience::Actor(a), OutputEvent::Request { prompt, .. }) if *a == who => {
                Some(prompt.to_plain_text())
            }
            _ => None,
        })
        .collect()
}

/// I10 hardening (D-I-11): a consent prompt names the asker only as the
/// subject perceives them — a concealed facilitator ends the scene as
/// "Someone", and an ordinary one by name.
#[test]
fn d6_a_consent_prompt_names_a_concealed_asker_as_someone() {
    let mut r = rig_with(|nick, _ann| vec![Box::new(VeilModule(nick))]);
    run(&mut r.engine, r.nick, "+timestop");
    let out = run(&mut r.engine, r.nick, "+timestop/end");
    let prompts = prompts_to(&out, r.ann);
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(
        prompts[0].starts_with("Someone ended scene #"),
        "{prompts:?}"
    );
    assert!(
        !prompts[0].contains("Nick"),
        "a concealed asker's name never reaches the prompt: {prompts:?}"
    );

    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let out = run(&mut r.engine, r.nick, "+timestop/end");
    let prompts = prompts_to(&out, r.ann);
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(
        prompts[0].starts_with("Nick ended scene #"),
        "a perceivable asker is named: {prompts:?}"
    );
}

/// The rewind prompt names two objects — the asker and the turn's actor —
/// and both go through the subject's own view (verify fold, C1).
#[test]
fn d6_a_rewind_prompt_names_the_turns_actor_as_the_subject_sees_them() {
    let mut r = rig_with(|nick, _ann| vec![Box::new(VeilModule(nick))]);
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "+timestop/next");
    let out = run(&mut r.engine, r.nick, "+timestop/rewind");
    let prompts = prompts_to(&out, r.ann);
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(
        prompts[0].starts_with("Someone rewound scene #")
            && prompts[0].contains("to Someone's turn (round 1)"),
        "{prompts:?}"
    );
    assert!(!prompts[0].contains("Nick"), "{prompts:?}");

    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "+timestop/next");
    let out = run(&mut r.engine, r.nick, "+timestop/rewind");
    let prompts = prompts_to(&out, r.ann);
    assert!(
        prompts[0].starts_with("Nick rewound scene #")
            && prompts[0].contains("to Nick's turn (round 1)"),
        "{prompts:?}"
    );
}

/// DARK is part of the gate, as it is for `look` (verify fold): a DARK asker
/// is "Someone" to a subject without `SEE_HIDDEN`.
#[test]
fn d6_a_dark_asker_is_someone_in_the_prompt() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    r.engine.world.set_flag(r.nick, Flags::DARK, true).unwrap();
    let out = run(&mut r.engine, r.nick, "+timestop/end");
    let prompts = prompts_to(&out, r.ann);
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(
        prompts[0].starts_with("Someone ended scene #"),
        "{prompts:?}"
    );
}

/// Silence for `CONSENT_DEADLINE_S` is recorded as an abstention, in the record
/// and in the scene's log.
#[test]
fn d6_silence_abstains_at_the_deadline() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let out = run(&mut r.engine, r.nick, "+timestop/end");
    assert_eq!(requests(&out).len(), 1);
    // Not yet.
    advance_to(&mut r, 1_000 + CONSENT_DEADLINE_S - 1);
    assert!(votes(&r).is_empty(), "the deadline has not passed");
    advance_to(&mut r, 1_000 + CONSENT_DEADLINE_S + 1);
    assert_eq!(
        votes(&r),
        vec![("end".to_string(), r.ann, "abstain".to_string())]
    );
    let log = texts(&run(&mut r.engine, r.nick, "+timestop/log"));
    assert!(log.contains("Ann on end: abstain."), "{log}");
}

/// A disagreement is recorded and blocks nothing; the facilitator is told the
/// answer and the running tally.
#[test]
fn d6_a_disagree_is_recorded_and_blocks_nothing() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let out = run(&mut r.engine, r.nick, "+timestop/end");
    let (_, id, _) = requests(&out)[0];
    let out = respond(&mut r.engine, r.ann, id, RespondChoice::Confirm(false));
    assert_eq!(
        votes(&r),
        vec![("end".to_string(), r.ann, "disagree".to_string())]
    );
    assert_eq!(scene(&r).status, SceneStatus::Closed, "a veto this is not");
    let to_nick = sys_to(&out, r.nick);
    assert!(
        to_nick.contains("Ann on end")
            && to_nick.contains("disagree")
            && to_nick.contains("1 disagree"),
        "the facilitator hears the answer and the tally: {to_nick}"
    );
    assert!(
        sys_to(&out, r.ann).contains("Recorded: disagree"),
        "the participant sees their answer land: {}",
        sys_to(&out, r.ann)
    );
    // A second answer to the same request is ignored by the engine (resolved).
    let out = respond(&mut r.engine, r.ann, id, RespondChoice::Confirm(true));
    assert!(out.is_empty());
    assert_eq!(votes(&r).len(), 1);
}

/// The seal asks the participants whether they agree with the outcome; the
/// `scenes` keyframe carries the tally as counts.
#[test]
fn d6_the_seal_asks_the_participants_and_the_keyframe_counts_the_answers() {
    let mut r = rig();
    let nick = r.nick;
    scened(&mut r, nick);
    run(&mut r.engine, r.nick, "+timestop/end");
    let out = run(&mut r.engine, r.nick, "+scene/outcome success=Nick run=7");
    assert!(texts(&out).contains("is sealed"), "{}", texts(&out));
    let asked = requests(&out);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!((asked[0].0, asked[0].2), (r.ann, RequestKind::Confirm));
    let out = respond(
        &mut r.engine,
        r.ann,
        asked[0].1,
        RespondChoice::Confirm(true),
    );
    assert_eq!(
        votes(&r),
        vec![("outcome".to_string(), r.ann, "agree".to_string())]
    );
    let keyframe = out
        .events()
        .iter()
        .find_map(|(aud, e)| match (aud, e) {
            (Audience::Actor(a), OutputEvent::Panel { feed, data, .. })
                if *a == r.nick && feed == "scenes" =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .expect("a keyframe to the facilitator");
    assert_eq!(keyframe["consent"]["outcome"]["agree"], 1, "{keyframe}");
    assert!(
        keyframe["consent"]["end"].is_null(),
        "an unanswered question has no tally yet: {keyframe}"
    );
    assert!(
        !keyframe["consent"].to_string().contains("Ann"),
        "the keyframe counts, it never names: {keyframe}"
    );
}

/// `+timestop/rewind` steps the turn back (into the previous round when
/// needed), asks the participants, and the tally names no one but to the
/// facilitator and staff (lens P).
#[test]
fn d6_rewind_steps_the_turn_back_asks_and_the_tally_names_no_one_but_to_the_facilitator() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    assert_eq!(scene(&r).current_actor(), Some(r.nick));
    let out = run(&mut r.engine, r.nick, "+timestop/rewind");
    assert!(
        texts(&out).contains("no earlier turn"),
        "the first turn of round one has nothing before it: {}",
        texts(&out)
    );
    run(&mut r.engine, r.nick, "+timestop/next");
    assert_eq!(scene(&r).current_actor(), Some(r.ann));
    let out = run(&mut r.engine, r.nick, "+timestop/rewind");
    assert!(
        texts(&out).contains("The turn goes back"),
        "{}",
        texts(&out)
    );
    assert_eq!(scene(&r).current_actor(), Some(r.nick));
    assert_eq!(scene(&r).round, 1);
    let asked = requests(&out);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!((asked[0].0, asked[0].2), (r.ann, RequestKind::Confirm));
    respond(
        &mut r.engine,
        r.ann,
        asked[0].1,
        RespondChoice::Confirm(true),
    );
    assert_eq!(
        votes(&r),
        vec![("rewind".to_string(), r.ann, "agree".to_string())]
    );

    // The facilitator sees who; a participant sees only counts; staff see who.
    let who = texts(&run(&mut r.engine, r.nick, "+timestop/who"));
    assert!(
        who.contains("rewind: 1 agree") && who.contains("Ann — agree"),
        "{who}"
    );
    let who = texts(&run(&mut r.engine, r.ann, "+timestop/who"));
    assert!(
        who.contains("rewind: 1 agree") && !who.contains("Ann — agree"),
        "{who}"
    );
    let who = texts(&run(&mut r.engine, GOD, "+timestop/who"));
    assert!(who.contains("Ann — agree"), "{who}");

    // Across a round boundary: three advances reach round 2, a rewind returns
    // to the last participant of round 1.
    run(&mut r.engine, r.nick, "+timestop/next");
    run(&mut r.engine, r.nick, "+timestop/next");
    run(&mut r.engine, r.nick, "+timestop/next");
    assert_eq!(
        (scene(&r).round, scene(&r).current_actor()),
        (2, Some(r.nick))
    );
    run(&mut r.engine, r.nick, "+timestop/rewind");
    assert_eq!(
        (scene(&r).round, scene(&r).current_actor()),
        (1, Some(r.npc))
    );
    let log = texts(&run(&mut r.engine, r.nick, "+timestop/log"));
    assert!(log.contains("rewound the turn"), "{log}");
}

/// A nudge asks the idle current actor once per turn; `pass` advances the
/// turn; a pass answered after the facilitator already advanced is ignored
/// (lens S).
#[test]
fn d6_a_nudge_asks_the_idle_actor_once_a_pass_advances_and_a_late_pass_is_ignored() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let out = run(&mut r.engine, r.nick, "+timestop/nudge 99999");
    assert!(texts(&out).contains("1 to 1440"), "{}", texts(&out));
    let out = run(&mut r.engine, r.nick, "+timestop/nudge 5");
    assert!(texts(&out).contains("Nudging on"), "{}", texts(&out));
    assert_eq!(scene(&r).nudge_after_s, Some(300));

    // Four minutes in: nothing. Five minutes in: Nick is asked, once.
    let out = advance_to(&mut r, 1_000 + 240);
    assert!(requests(&out).is_empty(), "{:?}", requests(&out));
    let out = advance_to(&mut r, 1_000 + 301);
    let asked = requests(&out);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!((asked[0].0, asked[0].2), (r.nick, RequestKind::Choice));
    let out = advance_to(&mut r, 1_000 + 340);
    assert!(
        requests(&out).is_empty(),
        "a turn is nudged once: {:?}",
        requests(&out)
    );

    // Pass: the turn moves to Ann and the pass is on the record.
    respond(
        &mut r.engine,
        r.nick,
        asked[0].1,
        RespondChoice::Choose("pass".into()),
    );
    assert_eq!(scene(&r).current_actor(), Some(r.ann));
    assert_eq!(
        votes(&r),
        vec![("nudge".to_string(), r.nick, "pass".to_string())]
    );

    // Ann idles into a nudge; the facilitator advances by hand first; Ann's
    // late pass is ignored — the turn does not skip the Barkeep.
    let out = advance_to(&mut r, 1_000 + 340 + 301);
    let asked = requests(&out);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].0, r.ann);
    run(&mut r.engine, r.nick, "+timestop/next");
    assert_eq!(scene(&r).current_actor(), Some(r.npc));
    respond(
        &mut r.engine,
        r.ann,
        asked[0].1,
        RespondChoice::Choose("pass".into()),
    );
    assert_eq!(
        scene(&r).current_actor(),
        Some(r.npc),
        "a late pass moves nothing"
    );
    assert_eq!(votes(&r).len(), 1, "and records nothing: {:?}", votes(&r));

    // Off: no further nudges.
    run(&mut r.engine, r.nick, "+timestop/next");
    assert_eq!(scene(&r).current_actor(), Some(r.nick));
    let out = run(&mut r.engine, r.nick, "+timestop/nudge off");
    assert!(texts(&out).contains("Nudging off"), "{}", texts(&out));
    let out = advance_to(&mut r, 1_000 + 340 + 301 + 600);
    assert!(requests(&out).is_empty(), "{:?}", requests(&out));
}
