//! Canon read/retcon commands through the real `Engine`. The commit +
//! persistence round trip is exercised end to end via scene emission in
//! mush-scenes; here we pin the chronicle scope filtering and retcon marking.

use std::collections::HashMap;

use mush_canon::{CanonEvent, CanonId, CanonModule, CanonScope, CanonState, LOG};
use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::{
    Flags, GOD, LIMBO, ObjId, ObjKind, Outbox, Registry, World, register_core_commands,
    register_engine_commands,
};
use mush_proto::OutputEvent;

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
    (engine, alice, bob)
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

/// Push a canon event directly into the ledger (simulating a module `commit`).
fn push(engine: &mut Engine, summary: &str, scope: CanonScope, entities: &[ObjId]) -> CanonId {
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
        supersedes: None,
        reason: String::new(),
        detail: serde_json::Value::Null,
    };
    let prepared = st
        .ledger
        .prepare_append(LOG, serde_json::to_value(&e).unwrap())
        .unwrap();
    st.ledger.apply_prepared(prepared).unwrap();
    id
}

#[test]
fn chronicle_filters_by_scope() {
    let (mut engine, alice, bob) = rig();
    push(
        &mut engine,
        "The waterfront changed hands",
        CanonScope::Public,
        &[],
    );
    push(
        &mut engine,
        "Alice swore a boon to Bob",
        CanonScope::Restricted,
        &[alice, bob],
    );
    push(
        &mut engine,
        "The Prince is secretly dead",
        CanonScope::Secret,
        &[alice],
    );

    // Alice: public + the restricted one she's in; never the secret.
    let t = texts(&run(&mut engine, alice, "+chronicle"));
    assert!(t.contains("waterfront changed hands"), "{t}");
    assert!(t.contains("swore a boon"), "{t}");
    assert!(
        !t.contains("secretly dead"),
        "secret leaked to a player: {t}"
    );

    // A bystander (Bob is an entity of the boon, so use a third party).
    let eve = engine
        .world
        .create(ObjKind::Player, "Eve", GOD, Some(LIMBO))
        .unwrap();
    engine.world.set_connected(eve, true);
    let t = texts(&run(&mut engine, eve, "+chronicle"));
    assert!(t.contains("waterfront"), "{t}");
    assert!(
        !t.contains("swore a boon"),
        "restricted leaked to a non-entity: {t}"
    );

    // Staff (@canon) see everything including the secret.
    let god_t = texts(&run(&mut engine, GOD, "@canon"));
    assert!(
        god_t.contains("secretly dead"),
        "staff must see secret: {god_t}"
    );
    assert!(
        god_t.contains("<secret>"),
        "staff view shows scope: {god_t}"
    );
}

#[test]
fn retcon_appends_and_marks_the_target() {
    let (mut engine, _alice, _bob) = rig();
    let id = push(
        &mut engine,
        "The warehouse burned down",
        CanonScope::Public,
        &[],
    );

    // A player cannot retcon.
    let out = run(&mut engine, _alice, "@canon/retcon 0=nope");
    assert!(texts(&out).contains("Permission denied"), "{}", texts(&out));

    // Staff retcons it.
    let out = run(
        &mut engine,
        GOD,
        &format!("@canon/retcon {id}=it was only damaged"),
    );
    assert!(
        texts(&out).contains("Retconned canon #0"),
        "{}",
        texts(&out)
    );

    // The chronicle now marks the original as retconned, and hides the retcon
    // meta-event itself.
    let t = texts(&run(&mut engine, GOD, "@canon"));
    assert!(t.contains("(retconned)"), "target marked: {t}");
    // The player chronicle marks it too and does not show the raw retcon line.
    let pt = texts(&run(&mut engine, _alice, "+chronicle"));
    assert!(pt.contains("warehouse burned down"), "{pt}");
    assert!(pt.contains("(retconned)"), "{pt}");
    assert!(
        !pt.contains("Retcon of canon"),
        "retcon meta-event hidden from players: {pt}"
    );

    // Retconning an unknown id fails clearly.
    let out = run(&mut engine, GOD, "@canon/retcon 999=huh");
    assert!(
        texts(&out).contains("No such canon event"),
        "{}",
        texts(&out)
    );
}

#[test]
fn player_chronicle_hides_canon_ids() {
    let (mut engine, alice, _bob) = rig();
    push(&mut engine, "The bridge collapsed", CanonScope::Public, &[]);
    push(&mut engine, "The gate reopened", CanonScope::Public, &[]);
    // A player never sees raw ids — the dense global counter's gaps would leak
    // the existence/timing of hidden events.
    let pt = texts(&run(&mut engine, alice, "+chronicle"));
    assert!(pt.contains("bridge collapsed"), "{pt}");
    assert!(
        !pt.contains("#0") && !pt.contains("#1"),
        "player chronicle leaked ids: {pt}"
    );
    // Staff (@canon) do see ids, for retcon targeting.
    let st = texts(&run(&mut engine, GOD, "@canon"));
    assert!(st.contains("#0"), "staff view shows ids: {st}");
}

#[test]
fn retcon_rejects_retcon_records_and_double_supersede() {
    let (mut engine, _alice, _bob) = rig();
    push(&mut engine, "the tower fell", CanonScope::Public, &[]); // #0
    run(&mut engine, GOD, "@canon/retcon 0=it stood"); // #1 supersedes 0
    // Retconning the retcon record itself is refused.
    let out = run(&mut engine, GOD, "@canon/retcon 1=undo");
    assert!(texts(&out).contains("retcon record"), "{}", texts(&out));
    // Double-retconning the same original is refused.
    let out = run(&mut engine, GOD, "@canon/retcon 0=again");
    assert!(
        texts(&out).contains("already been retconned"),
        "{}",
        texts(&out)
    );
}

#[test]
fn retcon_inherits_the_targets_scope() {
    let (mut engine, alice, _bob) = rig();
    push(&mut engine, "a secret death", CanonScope::Secret, &[alice]); // #0
    run(&mut engine, GOD, "@canon/retcon 0=undone"); // #1 must not be Public
    let st = engine.world.module_state::<CanonState>().unwrap();
    let retcon = st
        .recent()
        .into_iter()
        .find(|e| e.supersedes == Some(CanonId(0)))
        .expect("retcon recorded");
    assert_eq!(
        retcon.scope,
        CanonScope::Secret,
        "a retcon of a secret event must not be broadcast as public"
    );
}

#[test]
fn empty_chronicle_is_graceful() {
    let (mut engine, alice, _bob) = rig();
    assert!(texts(&run(&mut engine, alice, "+chronicle")).contains("empty"));
    // A non-staff @canon is denied.
    let out = run(&mut engine, alice, "@canon");
    assert!(texts(&out).contains("Permission denied"), "{}", texts(&out));
    // Make Alice royalty → allowed.
    engine
        .world
        .get_mut(alice)
        .unwrap()
        .flags
        .insert(Flags::ROYALTY);
    assert!(texts(&run(&mut engine, alice, "@canon")).contains("empty"));
}

// ---- E7 cut 9 independent verify fold ----

/// Push a canon event naming `source`, so a bounded per-source read can be pinned.
fn push_source(engine: &mut Engine, summary: &str, source: &str) -> CanonId {
    let st = engine.world.module_state_mut::<CanonState>().unwrap();
    let id = st.alloc_id();
    let e = CanonEvent {
        id,
        kind: "campaign.operation.resolved".into(),
        summary: summary.into(),
        entities: Vec::new(),
        scope: CanonScope::Public,
        ic_time: 100,
        at: 1000,
        source: source.to_owned(),
        supersedes: None,
        reason: String::new(),
        detail: serde_json::Value::Null,
    };
    let prepared = st
        .ledger
        .prepare_append(LOG, serde_json::to_value(&e).unwrap())
        .unwrap();
    st.ledger.apply_prepared(prepared).unwrap();
    id
}

/// **D1d (E7 cut 9 fold). `records_for_source` is a WINDOW, and says so.**
///
/// E7 cut 9's `settle::root_recorded` used it as "canon's own durable ledger" for a
/// commit-once check, and `tick::objective_met` asked it for 16 records. It is
/// neither: it answers from the ledger's open tail plus a boot-time sealed tail, it
/// is clamped to `MAX_RECORDS_PER_SOURCE` **8** however large a limit is passed, and
/// it is newest-first — so a record behind the eighth is invisible to it. Pinned here
/// so no other crate mistakes the seam for the ledger again.
#[test]
fn vf_d1d_records_for_source_is_windowed_and_clamped_to_max_records_per_source() {
    let (mut engine, _alice, _bob) = rig();
    let mut ids = Vec::new();
    for n in 0..12 {
        ids.push(push_source(&mut engine, &format!("op {n}"), "op:1"));
    }
    push_source(&mut engine, "another", "op:2");
    let state = engine.world.module_state::<CanonState>().unwrap();

    // However large the limit, the answer is clamped.
    for limit in [1, 8, 16, 1_000] {
        let got = mush_canon::records_for_source(state, "op:1", limit);
        assert!(
            got.len() <= mush_canon::MAX_RECORDS_PER_SOURCE.min(limit),
            "limit {limit} answered {} records",
            got.len()
        );
    }
    let got = mush_canon::records_for_source(state, "op:1", 16);
    assert_eq!(got.len(), mush_canon::MAX_RECORDS_PER_SOURCE);
    // Newest first, so the OLDEST records are the ones that fall out of the answer.
    assert_eq!(got[0].id, ids[11]);
    let visible: Vec<CanonId> = got.iter().map(|e| e.id).collect();
    for old in &ids[..4] {
        assert!(
            !visible.contains(old),
            "{old:?} is behind the window and was still answered"
        );
    }
    // …and the seam is per-source, so another source's traffic is not the reason.
    assert_eq!(mush_canon::records_for_source(state, "op:2", 16).len(), 1);
    assert!(mush_canon::records_for_source(state, "op:3", 16).is_empty());
}
