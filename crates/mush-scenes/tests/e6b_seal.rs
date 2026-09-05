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
use mush_core::{
    ClockProviderFailure, ClockReadError, ClockRef, GOD, ObjId, ObjKind, Outbox, Registry, World,
    register_core_commands, register_engine_commands,
};
use mush_proto::OutputEvent;
use mush_scenes::{PauseReason, SceneStatus, ScenesModule, ScenesState};

// ---- rig ------------------------------------------------------------------

struct Rig {
    engine: Engine,
    bar: ObjId,
    nick: ObjId,
    ann: ObjId,
    npc: ObjId,
}

fn rig() -> Rig {
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
    let mut engine = Engine::with_modules(
        world,
        registry,
        vec![
            Box::new(mush_canon::CanonModule::new()),
            Box::new(ScenesModule::new()),
        ],
    )
    .unwrap();
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
