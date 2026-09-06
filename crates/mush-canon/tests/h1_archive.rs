//! H1 — the chronicle archive over the module store-read seam
//! (`docs/design/11-h-implementation-plan.md` §3.1): `+chronicle/archive`
//! and `@canon/archive` page the sealed chunks older than the RAM tail, with
//! the live scope filter and retcon marking.

use std::collections::HashMap;

use mush_canon::{CanonEvent, CanonId, CanonModule, CanonScope, CanonState, LOG, NS};
use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::{
    Effect, GOD, LIMBO, Ledger, ModuleWrite, ObjId, ObjKind, Outbox, Registry, StoreReadResult,
    World, WorldCommand, chunk_key, register_core_commands, register_engine_commands,
};
use mush_proto::OutputEvent;
use smol_str::SmolStr;

fn rig() -> (Engine, ObjId, ObjId) {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let alice = world
        .create(ObjKind::Player, "Alice", GOD, Some(LIMBO))
        .unwrap();
    let bob = world
        .create(ObjKind::Player, "Bob", GOD, Some(LIMBO))
        .unwrap();
    world.set_connected(alice, true);
    world.set_connected(bob, true);
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine =
        Engine::with_modules(world, registry, vec![Box::new(CanonModule::new())]).unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .unwrap();
    // One entry per chunk, so the ledger is paged from the first event.
    engine
        .world
        .module_state_mut::<CanonState>()
        .unwrap()
        .ledger = Ledger::with_chunk_target(0);
    (engine, alice, bob)
}

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
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Append one canon event straight into the ledger (as a module `commit`
/// would), returning its id and the persisted chunk document the append
/// produced.
fn push(
    engine: &mut Engine,
    summary: &str,
    scope: CanonScope,
    entities: &[ObjId],
    supersedes: Option<CanonId>,
) -> (CanonId, (SmolStr, serde_json::Value)) {
    let st = engine.world.module_state_mut::<CanonState>().unwrap();
    let id = st.alloc_id();
    let e = CanonEvent {
        id,
        kind: "test".into(),
        summary: summary.into(),
        entities: entities.to_vec(),
        scope,
        ic_time: 100,
        at: 1000,
        source: String::new(),
        supersedes,
        reason: String::new(),
        detail: serde_json::Value::Null,
    };
    let prepared = st
        .ledger
        .prepare_append(LOG, serde_json::to_value(&e).unwrap())
        .unwrap();
    let (_, op) = st.ledger.apply_prepared(prepared).unwrap();
    let doc = match op {
        ModuleWrite::JsonSet { key, value } => (key, value),
        other => panic!("a chunk write is a JsonSet: {other:?}"),
    };
    (id, doc)
}

fn the_read(effects: &[Effect]) -> (u64, Vec<SmolStr>) {
    assert_eq!(effects.len(), 1, "exactly one effect: {effects:?}");
    match &effects[0] {
        Effect::StoreRead {
            module,
            correlation,
            keys,
            ..
        } => {
            assert_eq!(module.as_str(), NS);
            (*correlation, keys.clone())
        }
        other => panic!("expected a StoreRead effect: {other:?}"),
    }
}

fn answer(docs: &[(SmolStr, serde_json::Value)], keys: &[SmolStr]) -> StoreReadResult {
    let present: Vec<_> = keys
        .iter()
        .filter_map(|k| docs.iter().find(|(dk, _)| dk == k).cloned())
        .collect();
    let absent: Vec<_> = keys
        .iter()
        .filter(|k| !docs.iter().any(|(dk, _)| dk == *k))
        .cloned()
        .collect();
    StoreReadResult::Docs {
        docs: present,
        absent,
    }
}

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
    out
}

#[test]
fn h1_the_chronicle_archive_pages_sealed_history_with_the_live_scope_filter() {
    let (mut engine, alice, bob) = rig();
    let mut docs = Vec::new();
    // Six events → chunks 0..=5 sealed, an empty open chunk 6.
    for (summary, scope, ents) in [
        ("The waterfront changed hands", CanonScope::Public, vec![]),
        (
            "Alice swore a boon to Bob",
            CanonScope::Restricted,
            vec![alice, bob],
        ),
        (
            "The Prince is secretly dead",
            CanonScope::Secret,
            vec![alice],
        ),
        ("The sewers flooded", CanonScope::Public, vec![]),
        ("Bob owes the Harpy", CanonScope::Restricted, vec![bob]),
        ("A new Sheriff", CanonScope::Public, vec![]),
    ] {
        let (_, doc) = push(&mut engine, summary, scope, &ents, None);
        docs.push(doc);
    }
    let st = engine.world.module_state::<CanonState>().unwrap();
    assert_eq!(st.ledger.open_chunk(LOG), 6);
    assert!(
        st.recent().is_empty(),
        "the tail is empty: everything sealed"
    );

    // The live view has nothing; it points at the archive.
    let (out, fx) = run(&mut engine, alice, "+chronicle");
    assert!(texts(&out).contains("empty so far"), "{}", texts(&out));
    assert!(fx.is_empty());

    // Page 0 = chunks 2..=5. Alice sees the public ones; not the secret, not
    // Bob's restricted one.
    let (out, fx) = run(&mut engine, alice, "+chronicle/archive");
    assert!(
        texts(&out).contains("Reading archive page 0"),
        "{}",
        texts(&out)
    );
    let (c, keys) = the_read(&fx);
    assert_eq!(keys, (2..=5).map(|n| chunk_key(LOG, n)).collect::<Vec<_>>());
    let out = complete(&mut engine, alice, c, answer(&docs, &keys));
    let t = texts(&out);
    assert!(t.contains("archive page 0"), "{t}");
    assert!(t.contains("The sewers flooded"), "{t}");
    assert!(t.contains("A new Sheriff"), "{t}");
    assert!(!t.contains("secretly dead"), "a Secret event leaked: {t}");
    assert!(
        !t.contains("Bob owes"),
        "another's Restricted event leaked: {t}"
    );
    assert!(!t.contains('#'), "ids are staff-only: {t}");
    // Newest first.
    assert!(
        t.find("A new Sheriff").unwrap() < t.find("The sewers flooded").unwrap(),
        "{t}"
    );

    // Page 1 = chunks 0..=1: Alice sees the public one and her own boon.
    let (_, fx) = run(&mut engine, alice, "+chronicle/archive 1");
    let (c, keys) = the_read(&fx);
    assert_eq!(keys, (0..=1).map(|n| chunk_key(LOG, n)).collect::<Vec<_>>());
    let out = complete(&mut engine, alice, c, answer(&docs, &keys));
    let t = texts(&out);
    assert!(
        t.contains("waterfront") && t.contains("swore a boon"),
        "{t}"
    );

    // Past the oldest chunk.
    let (out, fx) = run(&mut engine, alice, "+chronicle/archive 2");
    assert!(
        texts(&out).contains("No such archive page"),
        "{}",
        texts(&out)
    );
    assert!(fx.is_empty());
    assert!(engine.take_module_ops().is_empty(), "reads persist nothing");
}

/// Verify fold **B1** (lens B), the chronicle half: a wedged pending read is
/// superseded after the stale window, never before.
#[test]
fn vf_b1_a_wedged_chronicle_read_is_superseded_after_the_stale_window() {
    let (mut engine, alice, _bob) = rig();
    let (_, doc) = push(&mut engine, "One", CanonScope::Public, &[], None);
    let docs = vec![doc];
    let (_, fx) = run(&mut engine, alice, "+chronicle/archive");
    let (old, keys) = the_read(&fx);
    engine.set_now(1_000 + mush_core::STORE_READ_STALE_SECS - 1);
    let (out, fx) = run(&mut engine, alice, "+chronicle/archive");
    assert!(texts(&out).contains("already in flight"), "{}", texts(&out));
    assert!(fx.is_empty());
    engine.set_now(1_000 + mush_core::STORE_READ_STALE_SECS);
    let (_, fx) = run(&mut engine, alice, "+chronicle/archive");
    let (fresh, _) = the_read(&fx);
    assert_ne!(fresh, old);
    let out = complete(&mut engine, alice, old, answer(&docs, &keys));
    assert!(out.is_empty(), "a stale completion renders nothing");
    let out = complete(&mut engine, alice, fresh, answer(&docs, &keys));
    assert!(texts(&out).contains("One"), "{}", texts(&out));
}

#[test]
fn h1_the_staff_archive_shows_everything_with_ids_and_marks_retcons_on_the_page() {
    let (mut engine, alice, _bob) = rig();
    let mut docs = Vec::new();
    let (first, d) = push(&mut engine, "One", CanonScope::Public, &[], None);
    docs.push(d);
    let (second, d) = push(&mut engine, "Two", CanonScope::Secret, &[], None);
    docs.push(d);
    let (_, d) = push(&mut engine, "Three", CanonScope::Public, &[], None);
    docs.push(d);
    // A retcon of `second` sits in chunk 3 — the same page as its target.
    let (_, d) = push(
        &mut engine,
        "Retcon of canon #1: it was a rumour",
        CanonScope::Secret,
        &[],
        Some(second),
    );
    docs.push(d);
    let _ = first;

    // A player is refused the staff view outright.
    let (out, fx) = run(&mut engine, alice, "@canon/archive");
    assert!(texts(&out).contains("Permission denied"), "{}", texts(&out));
    assert!(fx.is_empty());

    let (_, fx) = run(&mut engine, GOD, "@canon/archive");
    let (c, keys) = the_read(&fx);
    assert_eq!(keys, (0..=3).map(|n| chunk_key(LOG, n)).collect::<Vec<_>>());
    let out = complete(&mut engine, GOD, c, answer(&docs, &keys));
    let t = texts(&out);
    assert!(t.contains("Canon ledger — archive page 0"), "{t}");
    assert!(t.contains("#1 <secret>  Two  (retconned)"), "{t}");
    assert!(t.contains("#0 <public>  One"), "{t}");
    assert!(
        t.contains("Retcon of canon #1"),
        "staff see the retcon record: {t}"
    );

    // A page with an interior hole is refused with the breach, not rendered.
    let (_, fx) = run(&mut engine, GOD, "@canon/archive");
    let (c, keys) = the_read(&fx);
    let holed: Vec<_> = docs
        .iter()
        .filter(|(k, _)| k != &chunk_key(LOG, 2))
        .cloned()
        .collect();
    let out = complete(&mut engine, GOD, c, answer(&holed, &keys));
    let t = texts(&out);
    assert!(
        t.contains("cannot be read") && t.contains("chunk 2 is missing"),
        "{t}"
    );
    assert!(!t.contains("#0 <public>"), "{t}");
}
