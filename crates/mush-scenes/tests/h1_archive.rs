//! H1 — the scene archive over the module store-read seam
//! (`docs/design/11-h-implementation-plan.md` §3.1).
//!
//! `+timestop/log/archive` and `+recap/archive` request one page of a scene's
//! cold log as an [`Effect::StoreRead`]; the store answers off the world loop
//! and the page renders through the module's `StoreReadCompletion`. These
//! tests drive that round trip through the real `Engine`: the effect is taken
//! from the step, performed against an `InMemoryStore` holding exactly the
//! module ops the scene persisted, and re-entered as the typed completion.

use std::collections::HashMap;

use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::{
    Effect, GOD, Ledger, ModuleBootData, ModuleOp, ModuleWrite, ObjId, ObjKind, Outbox, Registry,
    StoreReadResult, World, WorldCommand, chunk_key, register_core_commands,
    register_engine_commands,
};
use mush_proto::OutputEvent;
use mush_scenes::{NS, SceneStatus, ScenesModule, ScenesState};
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use smol_str::SmolStr;

// ----------------------------------------------------------------------
// Harness (the timestop rig, minus the roll module)
// ----------------------------------------------------------------------

struct Rig {
    engine: Engine,
    nick: ObjId,
    ann: ObjId,
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
    world.set_connected(nick, true);
    world.set_connected(ann, true);
    world.set_live_conns(nick, 1);
    world.set_live_conns(ann, 1);
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
    Rig { engine, nick, ann }
}

/// Dispatch one line and return its output and the effects it deferred.
fn run(engine: &mut Engine, actor: ObjId, line: &str) -> (Outbox, Vec<Effect>) {
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
    (out, fx.drain())
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

/// The one `StoreRead` effect a request produced, as `(module, actor,
/// correlation, keys)`.
fn the_read(effects: &[Effect]) -> (SmolStr, ObjId, u64, Vec<SmolStr>) {
    assert_eq!(effects.len(), 1, "exactly one effect: {effects:?}");
    match &effects[0] {
        Effect::StoreRead {
            module,
            actor,
            correlation,
            keys,
        } => (module.clone(), *actor, *correlation, keys.clone()),
        other => panic!("expected a StoreRead effect: {other:?}"),
    }
}

/// Perform a read against `store` the way the world task's driver does: one
/// `module_json_get` per key, absent keys reported.
fn perform(store: &InMemoryStore, keys: &[SmolStr]) -> StoreReadResult {
    futures::executor::block_on(async {
        let mut docs = Vec::new();
        let mut absent = Vec::new();
        for key in keys {
            match store.module_json_get(NS, key).await.unwrap() {
                Some(value) => docs.push((key.clone(), value)),
                None => absent.push(key.clone()),
            }
        }
        StoreReadResult::Docs { docs, absent }
    })
}

/// Re-enter one completion exactly as the world task would.
fn complete(
    engine: &mut Engine,
    actor: ObjId,
    correlation: u64,
    result: StoreReadResult,
) -> Outbox {
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::Apply(WorldCommand::StoreReadDone {
            module: SmolStr::new(NS),
            actor,
            correlation,
            payload: result.to_payload(),
        }),
        &mut out,
        &mut fx,
    );
    assert!(fx.is_empty(), "a completion never queues further effects");
    out
}

/// Persist `ops` into a fresh store (what the world task's write batch does).
fn store_from(ops: Vec<ModuleOp>) -> InMemoryStore {
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
    store
}

/// Reboot the scenes module from `store` alone, recreating the rig's objects
/// at their dbrefs (deterministic creation order) so the records resolve.
fn reboot(store: &InMemoryStore) -> Engine {
    let kv = futures::executor::block_on(async { store.module_json_list(NS).await.unwrap() });
    let boot = ModuleBootData {
        kv: kv.into_iter().map(|(k, v)| (SmolStr::new(k), v)).collect(),
        streams: HashMap::new(),
        ..Default::default()
    };
    let mut world = World::new_minimal();
    world.set_now(2_000);
    let bar = world
        .create(ObjKind::Room, "The Tap Room", GOD, None)
        .unwrap();
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(bar))
        .unwrap();
    let ann = world
        .create(ObjKind::Player, "Ann", GOD, Some(bar))
        .unwrap();
    world.set_connected(nick, true);
    world.set_connected(ann, true);
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
        .modules_loaded(HashMap::from([(SmolStr::new(NS), boot)]), &mut out, &mut fx)
        .unwrap();
    let _ = engine.take_module_ops();
    engine
}

/// The event lines (`  N. …`) of a rendered log or archive page.
fn event_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| {
            l.trim_start()
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
        })
        .map(|l| l.trim_end().to_string())
        .collect()
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[test]
fn h1_the_archive_of_a_scene_closed_before_a_restart_renders_its_whole_log() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "say hello all");
    run(&mut r.engine, r.nick, ":waves");
    let (live, _) = run(&mut r.engine, r.nick, "+timestop/log");
    let live_lines = event_lines(&texts(&live));
    assert!(live_lines.len() >= 3, "{live_lines:?}");
    run(&mut r.engine, r.nick, "+timestop/end");
    // The close stamped the archive bound: one 16 KiB chunk holds this log.
    let id = scenes(&r.engine).scenes.keys().next().copied().unwrap();
    assert_eq!(scenes(&r.engine).scenes[&id].log_chunks, Some(0));
    let store = store_from(r.engine.take_module_ops());

    let mut engine = reboot(&store);
    let nick = engine.world.resolve_player("Nick").unwrap();
    assert_eq!(scenes(&engine).scenes[&id].status, SceneStatus::Closed);
    // The live path says where to look now.
    let (out, fx) = run(&mut engine, nick, "+timestop/log");
    assert!(
        texts(&out).contains("+timestop/log/archive"),
        "{}",
        texts(&out)
    );
    assert!(fx.is_empty());

    // The archive request: one StoreRead for the scene's chunk 0, in the
    // scenes namespace, for Nick.
    let (out, fx) = run(&mut engine, nick, "+timestop/log/archive");
    assert!(texts(&out).contains("Reading page 0"), "{}", texts(&out));
    let (module, actor, correlation, keys) = the_read(&fx);
    assert_eq!(module.as_str(), NS);
    assert_eq!(actor, nick);
    assert_eq!(keys, vec![chunk_key(&id.to_string(), 0)]);
    assert!(scenes(&engine).pending_reads.contains_key(&nick));

    // The page renders through the same renderer the live log used: the
    // live lines are a prefix of the archive (which also holds the Ended).
    let out = complete(&mut engine, nick, correlation, perform(&store, &keys));
    let t = texts(&out);
    assert!(t.contains("archive page 0"), "{t}");
    assert!(t.contains("hello all"), "{t}");
    let archive_lines = event_lines(&t);
    assert_eq!(&archive_lines[..live_lines.len()], &live_lines[..], "{t}");
    assert!(
        archive_lines.len() > live_lines.len(),
        "the Ended event: {t}"
    );
    // The completion cleared the pending read and persisted nothing.
    assert!(!scenes(&engine).pending_reads.contains_key(&nick));
    assert!(
        engine.take_module_ops().is_empty(),
        "a render queues no persist op"
    );
}

#[test]
fn h1_the_recap_archive_applies_the_live_recap_filter_and_renderer() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "say we should leave");
    run(&mut r.engine, r.ann, ":nods");
    let (live, _) = run(&mut r.engine, r.ann, "+recap");
    let live_text = texts(&live);
    assert!(
        live_text.contains("Nick says, \"we should leave\""),
        "{live_text}"
    );
    run(&mut r.engine, r.nick, "+timestop/end");
    let id = scenes(&r.engine).scenes.keys().next().copied().unwrap();
    let store = store_from(r.engine.take_module_ops());

    let mut engine = reboot(&store);
    let ann = engine.world.resolve_player("Ann").unwrap();
    let (out, _) = run(&mut engine, ann, "+recap");
    assert!(texts(&out).contains("+recap/archive"), "{}", texts(&out));
    let (_, fx) = run(&mut engine, ann, "+recap/archive");
    let (_, _, correlation, keys) = the_read(&fx);
    assert_eq!(keys, vec![chunk_key(&id.to_string(), 0)]);
    let out = complete(&mut engine, ann, correlation, perform(&store, &keys));
    let t = texts(&out);
    assert!(t.contains("as you witnessed it"), "{t}");
    // Every narrative line the live recap showed Ann is in the archive
    // recap, in the same words, plus the scene's end.
    for line in live_text
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("Nick ") || l.starts_with("Ann ") || l.starts_with("The scene"))
    {
        assert!(t.contains(line), "missing {line:?} in {t}");
    }
    assert!(t.contains("The scene ends."), "{t}");
}

#[test]
fn h1_pages_walk_sealed_chunks_newest_first_and_a_page_past_the_oldest_is_refused() {
    let mut r = rig();
    // One entry per chunk: every append seals, so the archive is paged.
    r.engine
        .world
        .module_state_mut::<ScenesState>()
        .unwrap()
        .events = Ledger::with_chunk_target(0);
    run(&mut r.engine, r.nick, "+timestop"); // seq 0: Started
    for n in 1..=5 {
        run(&mut r.engine, r.nick, &format!("say line {n}")); // seq 1..=5
    }
    let id = scenes(&r.engine).scenes.keys().next().copied().unwrap();
    let log = id.to_string();
    assert_eq!(scenes(&r.engine).events.open_chunk(&log), 6);
    let store = store_from(r.engine.take_module_ops());

    // Page 0 = the newest four sealed chunks (2..=5); page 1 = 0..=1.
    let (_, fx) = run(&mut r.engine, r.nick, "+timestop/log/archive");
    let (_, _, c0, keys0) = the_read(&fx);
    assert_eq!(
        keys0,
        (2..=5).map(|n| chunk_key(&log, n)).collect::<Vec<_>>()
    );
    let out = complete(&mut r.engine, r.nick, c0, perform(&store, &keys0));
    let lines = event_lines(&texts(&out));
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines[0].trim_start().starts_with("2."), "{lines:?}");
    assert!(lines[3].contains("line 5"), "{lines:?}");

    let (_, fx) = run(&mut r.engine, r.nick, "+timestop/log/archive 1");
    let (_, _, c1, keys1) = the_read(&fx);
    assert_eq!(
        keys1,
        (0..=1).map(|n| chunk_key(&log, n)).collect::<Vec<_>>()
    );
    let out = complete(&mut r.engine, r.nick, c1, perform(&store, &keys1));
    let lines = event_lines(&texts(&out));
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("The scene began."), "{lines:?}");

    let (out, fx) = run(&mut r.engine, r.nick, "+timestop/log/archive 2");
    assert!(
        texts(&out).contains("No such archive page"),
        "{}",
        texts(&out)
    );
    assert!(fx.is_empty());
    let (out, fx) = run(&mut r.engine, r.nick, "+timestop/log/archive nope");
    assert!(texts(&out).contains("Usage"), "{}", texts(&out));
    assert!(fx.is_empty());
}

#[test]
fn h1_a_hole_in_the_page_reports_a_breach_and_renders_nothing() {
    let mut r = rig();
    r.engine
        .world
        .module_state_mut::<ScenesState>()
        .unwrap()
        .events = Ledger::with_chunk_target(0);
    run(&mut r.engine, r.nick, "+timestop");
    for n in 1..=5 {
        run(&mut r.engine, r.nick, &format!("say line {n}"));
    }
    let id = scenes(&r.engine).scenes.keys().next().copied().unwrap();
    let store = store_from(r.engine.take_module_ops());
    let (_, fx) = run(&mut r.engine, r.nick, "+timestop/log/archive");
    let (_, _, c, keys) = the_read(&fx);
    // Chunk 3 vanished from the store's answer.
    let StoreReadResult::Docs { docs, absent } = perform(&store, &keys) else {
        unreachable!()
    };
    let holed: Vec<_> = docs
        .into_iter()
        .filter(|(k, _)| k != &chunk_key(&id.to_string(), 3))
        .collect();
    let out = complete(
        &mut r.engine,
        r.nick,
        c,
        StoreReadResult::Docs {
            docs: holed,
            absent,
        },
    );
    let t = texts(&out);
    assert!(t.contains("cannot be read"), "{t}");
    assert!(t.contains("chunk 3 is missing"), "{t}");
    assert!(event_lines(&t).is_empty(), "nothing rendered: {t}");
    assert!(!scenes(&r.engine).pending_reads.contains_key(&r.nick));
}

#[test]
fn h1_one_read_in_flight_per_actor_and_every_outcome_clears_it() {
    let mut r = rig();
    r.engine
        .world
        .module_state_mut::<ScenesState>()
        .unwrap()
        .events = Ledger::with_chunk_target(0);
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "say one");
    let (_, fx) = run(&mut r.engine, r.nick, "+timestop/log/archive");
    let (_, _, c, _) = the_read(&fx);
    // A second request while one is pending is refused and queues nothing.
    let (out, fx) = run(&mut r.engine, r.nick, "+timestop/log/archive");
    assert!(texts(&out).contains("already in flight"), "{}", texts(&out));
    assert!(fx.is_empty());
    // A stale completion (a different correlation) is dropped, the pending
    // read stays.
    let out = complete(
        &mut r.engine,
        r.nick,
        c + 100,
        StoreReadResult::Failed("stale".into()),
    );
    assert!(out.is_empty(), "{}", texts(&out));
    assert!(scenes(&r.engine).pending_reads.contains_key(&r.nick));
    // A failed read reports and clears.
    let out = complete(
        &mut r.engine,
        r.nick,
        c,
        StoreReadResult::Failed("boom".into()),
    );
    assert!(
        texts(&out).contains("did not complete (boom)"),
        "{}",
        texts(&out)
    );
    assert!(!scenes(&r.engine).pending_reads.contains_key(&r.nick));
    // And so does an oversized one.
    let (_, fx) = run(&mut r.engine, r.nick, "+timestop/log/archive");
    let (_, _, c, _) = the_read(&fx);
    let out = complete(
        &mut r.engine,
        r.nick,
        c,
        StoreReadResult::Oversized { bytes: 600_000 },
    );
    assert!(texts(&out).contains("too large"), "{}", texts(&out));
    assert!(!scenes(&r.engine).pending_reads.contains_key(&r.nick));
    // A completion with nothing pending is dropped.
    let out = complete(
        &mut r.engine,
        r.nick,
        c + 1,
        StoreReadResult::Failed("nobody asked".into()),
    );
    assert!(out.is_empty());
}

/// Verify fold **B1** (lens B): a read whose effect never launched — refused
/// while the world was booting or recovering — answers never. Without a
/// stale window the actor could not read again until a restart.
#[test]
fn vf_b1_a_wedged_pending_read_is_superseded_after_the_stale_window() {
    let mut r = rig();
    r.engine
        .world
        .module_state_mut::<ScenesState>()
        .unwrap()
        .events = Ledger::with_chunk_target(0);
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "say one");
    let (_, fx) = run(&mut r.engine, r.nick, "+timestop/log/archive");
    let (_, _, old, _) = the_read(&fx);
    // Inside the window: refused.
    r.engine
        .set_now(1_000 + mush_core::STORE_READ_STALE_SECS - 1);
    let (out, fx) = run(&mut r.engine, r.nick, "+timestop/log/archive");
    assert!(texts(&out).contains("already in flight"), "{}", texts(&out));
    assert!(fx.is_empty());
    // Past it: superseded — a fresh read with a fresh correlation.
    r.engine.set_now(1_000 + mush_core::STORE_READ_STALE_SECS);
    let (out, fx) = run(&mut r.engine, r.nick, "+timestop/log/archive");
    assert!(texts(&out).contains("Reading page 0"), "{}", texts(&out));
    let (_, _, fresh, _) = the_read(&fx);
    assert_ne!(fresh, old);
    // The wedged correlation, arriving late, is a stale drop; the fresh one
    // still answers.
    let out = complete(
        &mut r.engine,
        r.nick,
        old,
        StoreReadResult::Failed("late".into()),
    );
    assert!(out.is_empty(), "{}", texts(&out));
    assert!(scenes(&r.engine).pending_reads.contains_key(&r.nick));
    let out = complete(
        &mut r.engine,
        r.nick,
        fresh,
        StoreReadResult::Failed("answered".into()),
    );
    assert!(texts(&out).contains("did not complete"), "{}", texts(&out));
    assert!(!scenes(&r.engine).pending_reads.contains_key(&r.nick));
}

#[test]
fn h1_a_live_scene_with_no_sealed_history_and_a_pre_h1_record_are_both_handled() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "say one");
    // Nothing sealed yet: the whole log is live.
    let (out, fx) = run(&mut r.engine, r.nick, "+timestop/log/archive");
    assert!(texts(&out).contains("still live"), "{}", texts(&out));
    assert!(fx.is_empty());
    run(&mut r.engine, r.nick, "+timestop/end");
    let id = scenes(&r.engine).scenes.keys().next().copied().unwrap();
    let store = store_from(r.engine.take_module_ops());

    // A record written before H1 carries no bound: it pages forward from
    // chunk 0, oldest first, and an all-absent page says so.
    let mut engine = reboot(&store);
    let nick = engine.world.resolve_player("Nick").unwrap();
    engine
        .world
        .module_state_mut::<ScenesState>()
        .unwrap()
        .scenes
        .get_mut(&id)
        .unwrap()
        .log_chunks = None;
    let log = id.to_string();
    let (_, fx) = run(&mut engine, nick, "+timestop/log/archive 1");
    let (_, _, c, keys) = the_read(&fx);
    assert_eq!(
        keys,
        (4..=7).map(|n| chunk_key(&log, n)).collect::<Vec<_>>()
    );
    let out = complete(&mut engine, nick, c, perform(&store, &keys));
    assert!(
        texts(&out).contains("no archived chunks 4–7"),
        "{}",
        texts(&out)
    );
    let (_, fx) = run(&mut engine, nick, "+timestop/log/archive");
    let (_, _, c, keys) = the_read(&fx);
    assert_eq!(
        keys,
        (0..=3).map(|n| chunk_key(&log, n)).collect::<Vec<_>>()
    );
    let out = complete(&mut engine, nick, c, perform(&store, &keys));
    assert!(texts(&out).contains("one"), "{}", texts(&out));
}

#[test]
fn h1_a_seal_after_the_close_keeps_the_archive_bound_exact() {
    let mut r = rig();
    r.engine
        .world
        .module_state_mut::<ScenesState>()
        .unwrap()
        .events = Ledger::with_chunk_target(0);
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "+timestop/end");
    let id = scenes(&r.engine).scenes.keys().next().copied().unwrap();
    let after_close = scenes(&r.engine).scenes[&id].log_chunks.unwrap();
    // Staff confer the seal on Ann and Ann seals: one more append, which
    // with a zero target rolls the chunk.
    let (out, _) = run(
        &mut r.engine,
        GOD,
        &format!("@scene/facilitate {}=#{}", id.0, r.ann.0),
    );
    assert!(!texts(&out).contains("Permission"), "{}", texts(&out));
    let (out, _) = run(&mut r.engine, r.ann, "+scene/outcome success=Nick");
    assert!(texts(&out).contains("sealed"), "{}", texts(&out));
    let after_seal = scenes(&r.engine).scenes[&id].log_chunks.unwrap();
    assert_eq!(after_seal, after_close + 1, "the seal rolled the chunk");
    // The re-stamp persisted: the newest persisted record carries it.
    let ops = r.engine.take_module_ops();
    let key = mush_scenes::scene_key(id);
    let last = ops
        .iter()
        .rev()
        .find_map(|op| match &op.op {
            ModuleWrite::JsonSet { key: k, value } if *k == key => Some(value.clone()),
            _ => None,
        })
        .expect("the record was persisted");
    assert_eq!(last["log_chunks"], serde_json::json!(after_seal));
}

#[test]
fn ext_p2_13_dark_visitors_are_omitted_live_and_hidden_in_legacy_logs_and_archives() {
    let mut r = rig();
    let bar = r.engine.world.get(r.nick).unwrap().location.unwrap();
    let lobby = r
        .engine
        .world
        .create(ObjKind::Room, "Lobby", GOD, None)
        .unwrap();
    let ghost = r
        .engine
        .world
        .create(ObjKind::Player, "GhostMod", GOD, Some(lobby))
        .unwrap();
    r.engine
        .world
        .get_mut(ghost)
        .unwrap()
        .flags
        .insert(mush_core::Flags::DARK | mush_core::Flags::WIZARD);
    let spy = r
        .engine
        .world
        .create(ObjKind::Player, "ShadowMod", GOD, Some(lobby))
        .unwrap();
    r.engine
        .world
        .get_mut(spy)
        .unwrap()
        .flags
        .insert(mush_core::Flags::WIZARD);
    let bystander = r
        .engine
        .world
        .create(ObjKind::Player, "VisibleGuest", GOD, Some(lobby))
        .unwrap();

    // Nick starts a timestop with Ann; Nick is the non-staff facilitator, Ann is participant.
    run(&mut r.engine, r.nick, "+timestop");

    // 1. Live move: already-DARK non-member staff visits and leaves -> NOT recorded at all.
    r.engine.world.move_to(ghost, bar).unwrap();
    r.engine.emit_world_event(
        &mush_core::WorldEvent::Moved {
            who: ghost,
            from: Some(lobby),
            to: bar,
        },
        &mut Outbox::new(),
        &mut Effects::new(),
    );
    r.engine.world.move_to(ghost, lobby).unwrap();
    r.engine.emit_world_event(
        &mush_core::WorldEvent::Moved {
            who: ghost,
            from: Some(bar),
            to: lobby,
        },
        &mut Outbox::new(),
        &mut Effects::new(),
    );

    // Visible non-DARK bystander enters -> still recorded.
    r.engine.world.move_to(bystander, bar).unwrap();
    r.engine.emit_world_event(
        &mush_core::WorldEvent::Moved {
            who: bystander,
            from: Some(lobby),
            to: bar,
        },
        &mut Outbox::new(),
        &mut Effects::new(),
    );

    for viewer in [r.nick, r.ann] {
        let (log_out, _) = run(&mut r.engine, viewer, "+timestop/log");
        let log_txt = texts(&log_out);
        assert!(
            !log_txt.contains("GhostMod"),
            "live DARK visitor must not appear in +timestop/log: {log_txt}"
        );
        assert!(
            log_txt.contains("VisibleGuest arrives."),
            "visible visitor still appears: {log_txt}"
        );
        let (recap_out, _) = run(&mut r.engine, viewer, "+recap");
        let recap_txt = texts(&recap_out);
        assert!(
            !recap_txt.contains("GhostMod"),
            "live DARK visitor must not appear in +recap: {recap_txt}"
        );
        assert!(recap_txt.contains("VisibleGuest arrives."), "{recap_txt}");
    }

    // 2. Legacy persisted events: record real Joined/Left entries for `spy` while visible,
    //    then flag `spy` as `Flags::DARK` (equivalent to legacy pre-fix persisted rows whose
    //    actor is currently DARK and not a scene member), and verify read-side filtering on
    //    all four views (+timestop/log, +recap, +timestop/log/archive, +recap/archive).
    r.engine.world.move_to(spy, bar).unwrap();
    r.engine.emit_world_event(
        &mush_core::WorldEvent::Moved {
            who: spy,
            from: Some(lobby),
            to: bar,
        },
        &mut Outbox::new(),
        &mut Effects::new(),
    );
    r.engine.world.move_to(spy, lobby).unwrap();
    r.engine.emit_world_event(
        &mush_core::WorldEvent::Moved {
            who: spy,
            from: Some(bar),
            to: lobby,
        },
        &mut Outbox::new(),
        &mut Effects::new(),
    );
    r.engine
        .world
        .get_mut(spy)
        .unwrap()
        .flags
        .insert(mush_core::Flags::DARK);

    // Non-staff participant AND non-staff facilitator must not see ShadowMod OR a bogus
    // "earlier are in the archive" count leak!
    for viewer in [r.nick, r.ann] {
        let (log_out, _) = run(&mut r.engine, viewer, "+timestop/log");
        let log_txt = texts(&log_out);
        assert!(
            !log_txt.contains("ShadowMod") && !log_txt.contains("GhostMod"),
            "legacy DARK visitor must be hidden in +timestop/log: {log_txt}"
        );
        assert!(
            !log_txt.contains("earlier are in the archive"),
            "filtered legacy DARK visitor must not leak via archive omission count: {log_txt}"
        );
        let (recap_out, _) = run(&mut r.engine, viewer, "+recap");
        let recap_txt = texts(&recap_out);
        assert!(
            !recap_txt.contains("ShadowMod") && !recap_txt.contains("GhostMod"),
            "legacy DARK visitor must be hidden in +recap: {recap_txt}"
        );
    }
    // Staff (GOD) standing in the room still sees the legacy ShadowMod events (and never
    // GhostMod, which was never recorded).
    r.engine.world.move_to(GOD, bar).unwrap();
    let (staff_log, _) = run(&mut r.engine, GOD, "+timestop/log");
    assert!(
        texts(&staff_log).contains("ShadowMod arrives.")
            && texts(&staff_log).contains("ShadowMod leaves.")
            && !texts(&staff_log).contains("GhostMod"),
        "staff sees legacy events in +timestop/log: {}",
        texts(&staff_log)
    );
    let (staff_recap, _) = run(&mut r.engine, GOD, "+recap");
    assert!(
        texts(&staff_recap).contains("ShadowMod arrives.")
            && texts(&staff_recap).contains("ShadowMod departs.")
            && !texts(&staff_recap).contains("GhostMod"),
        "staff sees legacy events in +recap: {}",
        texts(&staff_recap)
    );

    // 3. Archive views (+timestop/log/archive and +recap/archive) with legacy DARK events.
    run(&mut r.engine, r.nick, "+timestop/end");
    let store = store_from(r.engine.take_module_ops());
    let mut booted = reboot(&store);
    // Recreate lobby, ghost, spy, bystander in the rebooted world with matching ObjIds.
    let booted_nick = booted.world.resolve_player("Nick").unwrap();
    let booted_ann = booted.world.resolve_player("Ann").unwrap();
    let booted_lobby = booted
        .world
        .create(ObjKind::Room, "Lobby", GOD, None)
        .unwrap();
    let booted_ghost = booted
        .world
        .create(ObjKind::Player, "GhostMod", GOD, Some(booted_lobby))
        .unwrap();
    assert_eq!(booted_ghost, ghost);
    booted
        .world
        .get_mut(booted_ghost)
        .unwrap()
        .flags
        .insert(mush_core::Flags::DARK | mush_core::Flags::WIZARD);
    let booted_spy = booted
        .world
        .create(ObjKind::Player, "ShadowMod", GOD, Some(booted_lobby))
        .unwrap();
    assert_eq!(booted_spy, spy);
    booted
        .world
        .get_mut(booted_spy)
        .unwrap()
        .flags
        .insert(mush_core::Flags::DARK | mush_core::Flags::WIZARD);
    let booted_bystander = booted
        .world
        .create(ObjKind::Player, "VisibleGuest", GOD, Some(booted_lobby))
        .unwrap();
    assert_eq!(booted_bystander, bystander);

    // Non-staff facilitator (Nick) and participant (Ann): both archive views hide ShadowMod.
    for viewer in [booted_nick, booted_ann] {
        let (_, fx) = run(&mut booted, viewer, "+timestop/log/archive");
        let (_, _, corr, keys) = the_read(&fx);
        let out = complete(&mut booted, viewer, corr, perform(&store, &keys));
        let txt = texts(&out);
        assert!(
            !txt.contains("ShadowMod") && !txt.contains("GhostMod"),
            "legacy DARK visitor must be hidden in +timestop/log/archive: {txt}"
        );
        assert!(
            txt.contains("VisibleGuest arrives."),
            "visible visitor still shown in +timestop/log/archive: {txt}"
        );

        let (_, fx) = run(&mut booted, viewer, "+recap/archive");
        let (_, _, corr, keys) = the_read(&fx);
        let out = complete(&mut booted, viewer, corr, perform(&store, &keys));
        let txt = texts(&out);
        assert!(
            !txt.contains("ShadowMod") && !txt.contains("GhostMod"),
            "legacy DARK visitor must be hidden in +recap/archive: {txt}"
        );
        assert!(
            txt.contains("VisibleGuest arrives."),
            "visible visitor still shown in +recap/archive: {txt}"
        );
    }

    // Staff sees ShadowMod in both archive views.
    booted
        .world
        .get_mut(booted_ann)
        .unwrap()
        .flags
        .insert(mush_core::Flags::WIZARD);
    let (_, fx) = run(&mut booted, booted_ann, "+timestop/log/archive");
    let (_, _, corr, keys) = the_read(&fx);
    let out = complete(&mut booted, booted_ann, corr, perform(&store, &keys));
    assert!(
        texts(&out).contains("ShadowMod arrives.") && texts(&out).contains("ShadowMod leaves."),
        "staff sees legacy events in +timestop/log/archive: {}",
        texts(&out)
    );
    let (_, fx) = run(&mut booted, booted_ann, "+recap/archive");
    let (_, _, corr, keys) = the_read(&fx);
    let out = complete(&mut booted, booted_ann, corr, perform(&store, &keys));
    assert!(
        texts(&out).contains("ShadowMod arrives.") && texts(&out).contains("ShadowMod departs."),
        "staff sees legacy events in +recap/archive: {}",
        texts(&out)
    );
}

/// Apply an incremental batch of module ops to an existing `InMemoryStore`.
fn apply_ops(store: &InMemoryStore, serial: u64, ops: Vec<ModuleOp>) {
    futures::executor::block_on(async {
        store
            .write(WriteBatch {
                serial,
                module_ops: ops,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
    });
}

/// Reboot the scenes module from `store`, returning the boot result and
/// preserving any deferred boot module ops for callers to inspect/apply.
fn try_reboot_with_ops(
    store: &InMemoryStore,
) -> Result<(Engine, Vec<ModuleOp>), mush_core::ModError> {
    let kv = futures::executor::block_on(async { store.module_json_list(NS).await.unwrap() });
    let boot = ModuleBootData {
        kv: kv.into_iter().map(|(k, v)| (SmolStr::new(k), v)).collect(),
        streams: HashMap::new(),
        ..Default::default()
    };
    let mut world = World::new_minimal();
    world.set_now(2_000);
    let bar = world
        .create(ObjKind::Room, "The Tap Room", GOD, None)
        .unwrap();
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(bar))
        .unwrap();
    let ann = world
        .create(ObjKind::Player, "Ann", GOD, Some(bar))
        .unwrap();
    world.set_connected(nick, true);
    world.set_connected(ann, true);
    world.set_live_conns(nick, 1);
    world.set_live_conns(ann, 1);
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
    )?;
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.modules_loaded(HashMap::from([(SmolStr::new(NS), boot)]), &mut out, &mut fx)?;
    let boot_ops = engine.take_module_ops();
    Ok((engine, boot_ops))
}

/// Extract all `log:*` docs from `store` in the raw `(key, text)` form that
/// `inspect_ledger` validates at boot.
fn ledger_docs_in(store: &InMemoryStore) -> Vec<mush_core::LedgerDoc> {
    futures::executor::block_on(async { store.module_json_list(NS).await.unwrap() })
        .into_iter()
        .filter(|(k, _)| mush_core::is_log_key(k))
        .map(|(k, v)| (SmolStr::new(k), serde_json::to_string(&v).unwrap()))
        .collect()
}

/// P2-CORE-4 / P2-MOD-6: Closing a multi-chunk (>16 KiB) unsealed scene,
/// restarting (`load_boot`), sealing it with `+scene/outcome` (and recording a
/// post-seal consent response), and restarting again must keep chunk 0 intact,
/// append the seal at the true tail, and pass `inspect_ledger` + `load_boot`.
#[test]
fn p2_mod_6_sealing_multi_chunk_closed_scene_post_restart_preserves_chunk_zero_and_reboots_cleanly()
{
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let id = scenes(&r.engine).scenes.keys().next().copied().unwrap();
    let log_id = id.to_string();
    let (out, _) = run(
        &mut r.engine,
        GOD,
        &format!("@scene/facilitate {}=#{}", id.0, r.ann.0),
    );
    assert!(texts(&out).contains("may now seal"), "{}", texts(&out));

    // Exceed the real 16 KiB default chunk target so chunk 0 seals and chunk 1 opens.
    let line = format!("say {}", "a".repeat(512));
    for _ in 0..40 {
        run(&mut r.engine, r.nick, &line);
    }
    assert!(
        scenes(&r.engine).events.open_chunk(&log_id) >= 1,
        "must span multiple chunks (>16 KiB), got open_chunk={}",
        scenes(&r.engine).events.open_chunk(&log_id)
    );
    run(&mut r.engine, r.nick, "+timestop/end");
    let seq_after_end = scenes(&r.engine).events.next_seq(&log_id);
    let chunk_after_end = scenes(&r.engine).events.open_chunk(&log_id);
    assert!(chunk_after_end >= 1);
    assert_eq!(
        scenes(&r.engine).scenes[&id].log_chunks,
        Some(chunk_after_end)
    );

    let store = store_from(r.engine.take_module_ops());
    let original_chunk_0 = futures::executor::block_on(async {
        store
            .module_json_get(NS, &chunk_key(&log_id, 0))
            .await
            .unwrap()
            .expect("chunk 0 persisted")
    });
    assert_eq!(
        original_chunk_0["sealed"],
        serde_json::json!(true),
        "chunk 0 must be sealed before restart"
    );

    // Restart #1: scene is Closed + unsealed, so tail restores for sealing while
    // H1 archive UX (`+timestop/log` -> `+timestop/log/archive`) stays intact.
    let (mut engine1, boot_ops1) = try_reboot_with_ops(&store).expect("restart #1 succeeds");
    assert!(boot_ops1.is_empty());
    let nick1 = engine1.world.resolve_player("Nick").unwrap();
    let ann1 = engine1.world.resolve_player("Ann").unwrap();
    assert_eq!(scenes(&engine1).scenes[&id].status, SceneStatus::Closed);
    assert!(scenes(&engine1).events.has_tail(&log_id));
    assert_eq!(scenes(&engine1).events.next_seq(&log_id), seq_after_end);
    let (log_out, _) = run(&mut engine1, nick1, "+timestop/log");
    assert!(
        texts(&log_out).contains("+timestop/log/archive"),
        "closed-before-restart UX preserved: {}",
        texts(&log_out)
    );

    // Seal post-restart with `+scene/outcome`!
    let (seal_out, _) = run(&mut engine1, ann1, "+scene/outcome success=Nick");
    assert!(
        texts(&seal_out).contains("is sealed"),
        "post-restart seal failed: {}",
        texts(&seal_out)
    );
    // Also respond to the post-seal `CONSENT_OUTCOME` request (a second append
    // onto the closed scene post-restart).
    let req_id = seal_out
        .events()
        .iter()
        .find_map(|(aud, ev)| match (aud, ev) {
            (mush_core::output::Audience::Actor(a), OutputEvent::Request { id, .. })
                if *a == nick1 =>
            {
                Some(*id)
            }
            _ => None,
        })
        .expect("CONSENT_OUTCOME request sent to Nick");
    {
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine1.step(
            Inbound::Respond {
                epoch: engine1.request_epoch().into(),
                conn: 1,
                actor: nick1,
                id: req_id,
                choice: mush_proto::RespondChoice::Confirm(true),
            },
            &mut out,
            &mut fx,
        );
    }

    let seal_ops = engine1.take_module_ops();
    assert!(
        !seal_ops.iter().any(|op| matches!(
            &op.op,
            ModuleWrite::JsonSet { key, .. } if *key == chunk_key(&log_id, 0)
        )),
        "post-restart sealing must NEVER rewrite sealed chunk 0"
    );
    apply_ops(&store, 2, seal_ops);

    let chunk_0_after = futures::executor::block_on(async {
        store
            .module_json_get(NS, &chunk_key(&log_id, 0))
            .await
            .unwrap()
            .expect("chunk 0 still persisted")
    });
    assert_eq!(
        chunk_0_after, original_chunk_0,
        "chunk 0 must remain byte-for-byte intact"
    );

    // Verify `inspect_ledger` passes cleanly across every persisted chunk!
    let history = mush_core::inspect_ledger(&ledger_docs_in(&store), None);
    assert!(
        history.is_clean(),
        "inspect_ledger must be clean after post-restart seal: {:?}",
        history.breaches()
    );
    assert_eq!(
        history.entries_of(&log_id) as u64,
        seq_after_end + 2,
        "Sealed + Consent appended at the true tail"
    );

    // Restart #2: must succeed cleanly, and now that `outcome.is_some()`, the
    // terminal closed scene's tail is NOT retained in RAM and IS closed in Ledger.
    let (mut engine2, _) = try_reboot_with_ops(&store).expect("restart #2 must not boot-brick");
    assert!(scenes(&engine2).scenes[&id].outcome.is_some());
    assert!(
        !scenes(&engine2).events.has_tail(&log_id),
        "already-sealed closed scene does not keep a resident tail"
    );
    assert!(
        scenes(&engine2).events.is_closed(&log_id),
        "already-sealed closed scene is marked closed in Ledger"
    );
    assert_eq!(
        scenes(&engine2)
            .events
            .prepare_append(&log_id, serde_json::json!({"bad": true}))
            .unwrap_err(),
        mush_core::LedgerAppendError::LogClosed
    );

    // Archive read still shows the newly appended Sealed event on page 0.
    let nick2 = engine2.world.resolve_player("Nick").unwrap();
    let (_, fx) = run(&mut engine2, nick2, "+timestop/log/archive");
    let (_, _, corr, keys) = the_read(&fx);
    let out = complete(&mut engine2, nick2, corr, perform(&store, &keys));
    let txt = texts(&out);
    assert!(
        txt.contains("sealed the outcome: success"),
        "archive includes post-restart Sealed event: {txt}"
    );
}

/// P2-MOD-6: Conferring a sealer via `@scene/facilitate` AFTER a restart on a
/// multi-chunk closed scene and sealing it works cleanly, AND `record_scene_event`
/// itself fails closed without writing chunk 0 if a closed/chunked scene ever
/// lacks a resident tail.
#[test]
fn p2_mod_6_post_restart_facilitate_then_seal_and_record_scene_event_fail_closed_guard() {
    let mut r = rig();
    run(&mut r.engine, r.nick, "+timestop");
    let id = scenes(&r.engine).scenes.keys().next().copied().unwrap();
    let log_id = id.to_string();
    let line = format!("say {}", "b".repeat(512));
    for _ in 0..40 {
        run(&mut r.engine, r.nick, &line);
    }
    run(&mut r.engine, r.nick, "+timestop/end");
    let store = store_from(r.engine.take_module_ops());
    let original_chunk_0 = futures::executor::block_on(async {
        store
            .module_json_get(NS, &chunk_key(&log_id, 0))
            .await
            .unwrap()
            .unwrap()
    });

    // Restart #1 with NO sealer conferred beforehand.
    let (mut engine1, _) = try_reboot_with_ops(&store).expect("restart #1 succeeds");
    let ann1 = engine1.world.resolve_player("Ann").unwrap();
    assert!(scenes(&engine1).scenes[&id].sealer.is_none());

    // Staff confers sealer on the closed scene post-restart, then Ann seals.
    let (fac_out, _) = run(
        &mut engine1,
        GOD,
        &format!("@scene/facilitate {}=#{}", id.0, ann1.0),
    );
    assert!(
        texts(&fac_out).contains("may now seal"),
        "{}",
        texts(&fac_out)
    );
    let (seal_out, _) = run(&mut engine1, ann1, "+scene/outcome success=Nick");
    assert!(
        texts(&seal_out).contains("is sealed"),
        "{}",
        texts(&seal_out)
    );
    apply_ops(&store, 2, engine1.take_module_ops());

    let chunk_0_after = futures::executor::block_on(async {
        store
            .module_json_get(NS, &chunk_key(&log_id, 0))
            .await
            .unwrap()
            .unwrap()
    });
    assert_eq!(chunk_0_after, original_chunk_0);
    let (engine2, _) = try_reboot_with_ops(&store).expect("restart #2 succeeds cleanly");
    let saved_bound = scenes(&engine2).scenes[&id].log_chunks;
    assert!(saved_bound.is_some());

    // Isolate the `record_scene_event` + `restamp_log_chunks` fail-closed guard
    // by booting a store that holds only the closed, unsealed, chunked `scene:0`
    // record (with `log_chunks = Some(bound)` and `sealer = Some(ann1)`) and NO
    // `log:0:*` docs — so `Ledger` has neither a tail nor a `closed_logs` marker.
    let mut unsealed_rec = scenes(&engine2).scenes[&id].clone();
    unsealed_rec.outcome = None;
    let isolated_store = store_from(vec![
        ModuleOp {
            ns: SmolStr::new(NS),
            op: ModuleWrite::JsonSet {
                key: mush_scenes::scene_key(id),
                value: serde_json::to_value(&unsealed_rec).unwrap(),
            },
        },
        ModuleOp {
            ns: SmolStr::new(NS),
            op: ModuleWrite::JsonSet {
                key: SmolStr::new_static("meta"),
                value: serde_json::json!({ "next_scene_uid": 1 }),
            },
        },
    ]);
    let (mut isolated_engine, _) =
        try_reboot_with_ops(&isolated_store).expect("isolated boot succeeds");
    assert!(!scenes(&isolated_engine).events.has_tail(&log_id));
    assert!(!scenes(&isolated_engine).events.is_closed(&log_id));

    let ann2 = isolated_engine.world.resolve_player("Ann").unwrap();
    let _ = run(&mut isolated_engine, ann2, "+scene/outcome success=Nick");
    let ops = isolated_engine.take_module_ops();
    assert!(
        !ops.iter().any(|op| op.ns == NS
            && matches!(
                &op.op,
                ModuleWrite::JsonSet { key, .. } if mush_core::is_log_key(key)
            )),
        "record_scene_event must refuse and emit no scenes log:* write when tail is absent: {ops:?}"
    );
    assert_eq!(
        scenes(&isolated_engine).scenes[&id].log_chunks,
        saved_bound,
        "restamp_log_chunks must not reset log_chunks to 0 when tail is absent"
    );
}

/// P2-MOD-31: `prune_closed_scenes` deletes both `scene:{uid}` and all
/// `log:{uid}:0..=hi` KV chunks, `on_obj_destroyed` stamps `log_chunks`, and
/// `load_boot` sweeps any pre-existing orphaned `log:{uid}:*` keys without
/// disturbing retained scenes or breaking archive reads.
#[test]
fn p2_mod_31_prune_deletes_kv_log_chunks_stamps_on_obj_destroyed_and_cleans_boot_orphans() {
    let mut r = rig();
    let bar = r.engine.world.get(r.nick).unwrap().location.unwrap();

    // Make Nick Royalty so we can open/close > MAX_RETAINED_CLOSED_SCENES scenes without
    // hitting the non-staff rate limit.
    r.engine
        .world
        .get_mut(r.nick)
        .unwrap()
        .flags
        .insert(mush_core::Flags::ROYALTY);

    // Scene 0: multi-chunk (>16 KiB) scene closed via `+timestop/end`.
    run(&mut r.engine, r.nick, "+timestop");
    let long_line = format!("say {}", "c".repeat(512));
    for _ in 0..40 {
        run(&mut r.engine, r.nick, &long_line);
    }
    run(&mut r.engine, r.nick, "+timestop/end");
    let scene0 = mush_scenes::SceneUid(0);
    assert!(
        scenes(&r.engine).scenes[&scene0].log_chunks.unwrap() >= 1,
        "scene 0 must span multiple chunks"
    );

    // Scene 1: multi-chunk (>16 KiB) scene closed via `on_obj_destroyed` when its sole participant dies.
    let solo_room = r
        .engine
        .world
        .create(ObjKind::Room, "Solo Room", GOD, None)
        .unwrap();
    let doomed = r
        .engine
        .world
        .create(ObjKind::Player, "Doomed", GOD, Some(solo_room))
        .unwrap();
    r.engine.world.set_connected(doomed, true);
    r.engine.world.set_live_conns(doomed, 1);
    run(&mut r.engine, doomed, "+timestop");
    for _ in 0..40 {
        run(&mut r.engine, doomed, &long_line);
    }
    let scene1 = mush_scenes::SceneUid(1);
    let expected_scene1_chunks = scenes(&r.engine).events.open_chunk("1");
    assert!(
        expected_scene1_chunks >= 1,
        "scene 1 must span multiple chunks"
    );
    r.engine.world.destroy(doomed).unwrap();
    r.engine.emit_world_event(
        &mush_core::WorldEvent::ObjDestroyed { obj: doomed },
        &mut Outbox::new(),
        &mut Effects::new(),
    );
    assert_eq!(
        scenes(&r.engine).scenes[&scene1].status,
        SceneStatus::Closed
    );
    assert_eq!(
        scenes(&r.engine).scenes[&scene1].log_chunks,
        Some(expected_scene1_chunks),
        "on_obj_destroyed must stamp log_chunks on the closed scene"
    );
    assert!(
        scenes(&r.engine).scenes[&scene1].ended_at.is_some(),
        "on_obj_destroyed must stamp ended_at on the closed scene"
    );

    // Create and close enough additional scenes to evict scenes 0, 1, and 2.
    let total = mush_scenes::MAX_RETAINED_CLOSED_SCENES + 3;
    for i in 2..total {
        let (out, _) = run(&mut r.engine, r.nick, "+timestop");
        assert!(
            texts(&out).contains("a timestop begins"),
            "scene {i} start failed: {}",
            texts(&out)
        );
        if i == total - 1 {
            run(&mut r.engine, r.nick, "say retained survivor");
        }
        run(&mut r.engine, r.nick, "+timestop/end");
    }
    let _ = bar;

    // Persist everything into InMemoryStore.
    let store = store_from(r.engine.take_module_ops());
    let stored_keys: std::collections::BTreeSet<String> = futures::executor::block_on(async {
        store
            .module_json_list(NS)
            .await
            .unwrap()
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    });

    // Evicted scenes 0, 1, 2 must have NEITHER `scene:{uid}` NOR any `log:{uid}:*` left in KV!
    for evicted_uid in [0u64, 1, 2] {
        assert!(
            !stored_keys.contains(&format!("scene:{evicted_uid}")),
            "evicted scene:{evicted_uid} leaked in KV"
        );
        assert!(
            !stored_keys
                .iter()
                .any(|k| k.starts_with(&format!("log:{evicted_uid}:"))),
            "evicted log:{evicted_uid}:* chunk(s) leaked in KV"
        );
    }
    // Retained closed scenes (3..total) still have their scene doc and log chunks intact.
    let last_uid = (total - 1) as u64;
    assert!(stored_keys.contains(&format!("scene:{last_uid}")));
    assert!(stored_keys.contains(&format!("log:{last_uid}:0")));

    // Inject a pre-existing orphaned multi-chunk log (no `scene:9999`, simulating PR #750
    // leftovers) and verify `load_boot` validates it, cleans it up, and advances `next_scene_uid`.
    let orphan_chunk_0 = serde_json::json!({
        "chunk": 0,
        "sealed": true,
        "entries": [{"seq": 0, "value": {"kind": "started", "ic_time": 0, "at": 1}}]
    });
    let orphan_chunk_1 = serde_json::json!({
        "chunk": 1,
        "sealed": false,
        "entries": [{"seq": 1, "value": {"kind": "ended", "ic_time": 0, "at": 2}}]
    });
    apply_ops(
        &store,
        2,
        vec![
            ModuleOp {
                ns: SmolStr::new(NS),
                op: ModuleWrite::JsonSet {
                    key: chunk_key("9999", 0),
                    value: orphan_chunk_0,
                },
            },
            ModuleOp {
                ns: SmolStr::new(NS),
                op: ModuleWrite::JsonSet {
                    key: chunk_key("9999", 1),
                    value: orphan_chunk_1,
                },
            },
        ],
    );

    let (mut booted, boot_ops) = try_reboot_with_ops(&store).expect("boot cleans orphans cleanly");
    assert!(
        scenes(&booted).next_scene_uid > 9999,
        "next_scene_uid must stay ahead of any swept orphan uid"
    );
    apply_ops(&store, 3, boot_ops);

    let keys_after_boot: std::collections::BTreeSet<String> = futures::executor::block_on(async {
        store
            .module_json_list(NS)
            .await
            .unwrap()
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    });
    assert!(!keys_after_boot.contains("log:9999:0"));
    assert!(!keys_after_boot.contains("log:9999:1"));

    // Retained surviving scene's archive remains completely readable after reboot!
    let nick_boot = booted.world.resolve_player("Nick").unwrap();
    let (_, fx) = run(&mut booted, nick_boot, "+timestop/log/archive");
    let (_, _, corr, keys) = the_read(&fx);
    let out = complete(&mut booted, nick_boot, corr, perform(&store, &keys));
    assert!(
        texts(&out).contains("retained survivor"),
        "retained closed scene archive intact: {}",
        texts(&out)
    );

    // Final clean reboot check after all deletions.
    let (_booted2, boot_ops2) = try_reboot_with_ops(&store).expect("final reboot passes");
    assert!(boot_ops2.is_empty());
}

/// P2-MOD-31 (boot-time pruning): when KV starts with more than
/// `MAX_RETAINED_CLOSED_SCENES` closed scenes, `load_boot` deletes both
/// `scene:{uid}` and all `log:{uid}:0..=hi` chunks of the evicted scenes, and
/// leaves zero closed-log markers for them in `Ledger`.
#[test]
fn p2_mod_31_boot_prune_deletes_excess_closed_scene_log_chunks_and_purges_ledger() {
    let mut r = rig();
    r.engine
        .world
        .get_mut(r.nick)
        .unwrap()
        .flags
        .insert(mush_core::Flags::ROYALTY);

    // Create 1 real closed scene as a valid template, then clone it into KV at
    // MAX_RETAINED_CLOSED_SCENES + 2 distinct uids with 2 chunks each.
    run(&mut r.engine, r.nick, "+timestop");
    run(&mut r.engine, r.nick, "+timestop/end");
    let template = scenes(&r.engine).scenes[&mush_scenes::SceneUid(0)].clone();

    let total = (mush_scenes::MAX_RETAINED_CLOSED_SCENES + 2) as u64;
    let mut ops = Vec::new();
    for uid in 0..total {
        let mut rec = template.clone();
        rec.id = mush_scenes::SceneUid(uid);
        rec.log_chunks = Some(1);
        ops.push(ModuleOp {
            ns: SmolStr::new(NS),
            op: ModuleWrite::JsonSet {
                key: mush_scenes::scene_key(rec.id),
                value: serde_json::to_value(&rec).unwrap(),
            },
        });
        let log_id = uid.to_string();
        ops.push(ModuleOp {
            ns: SmolStr::new(NS),
            op: ModuleWrite::JsonSet {
                key: chunk_key(&log_id, 0),
                value: serde_json::json!({
                    "chunk": 0,
                    "sealed": true,
                    "entries": [{"seq": 0, "value": {"kind": "started", "ic_time": 0, "at": 1}}]
                }),
            },
        });
        ops.push(ModuleOp {
            ns: SmolStr::new(NS),
            op: ModuleWrite::JsonSet {
                key: chunk_key(&log_id, 1),
                value: serde_json::json!({
                    "chunk": 1,
                    "sealed": false,
                    "entries": [{"seq": 1, "value": {"kind": "ended", "ic_time": 0, "at": 2}}]
                }),
            },
        });
    }
    ops.push(ModuleOp {
        ns: SmolStr::new(NS),
        op: ModuleWrite::JsonSet {
            key: SmolStr::new_static("meta"),
            value: serde_json::json!({ "next_scene_uid": total }),
        },
    });
    let store = store_from(ops);

    let (booted, boot_ops) = try_reboot_with_ops(&store).expect("boot prunes excess scenes");
    assert_eq!(
        scenes(&booted).scenes.closed_count(),
        mush_scenes::MAX_RETAINED_CLOSED_SCENES
    );
    // Evicted scenes 0 and 1 must have neither resident tails nor closed-log guards.
    for evicted in ["0", "1"] {
        assert!(!scenes(&booted).events.has_tail(evicted));
        assert!(!scenes(&booted).events.is_closed(evicted));
    }
    apply_ops(&store, 2, boot_ops);

    let keys: std::collections::BTreeSet<String> = futures::executor::block_on(async {
        store
            .module_json_list(NS)
            .await
            .unwrap()
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    });
    for evicted in [0u64, 1] {
        assert!(!keys.contains(&format!("scene:{evicted}")));
        assert!(!keys.contains(&format!("log:{evicted}:0")));
        assert!(!keys.contains(&format!("log:{evicted}:1")));
    }
    assert!(keys.contains("scene:2"));
    assert!(keys.contains("log:2:0"));
    assert!(keys.contains("log:2:1"));

    let (_booted2, boot_ops2) = try_reboot_with_ops(&store).expect("subsequent boot clean");
    assert!(boot_ops2.is_empty());
}
