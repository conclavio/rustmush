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
