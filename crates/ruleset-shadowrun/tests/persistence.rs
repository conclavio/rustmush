//! M17 — the SR5 scene-state hot-restart proof (the WoD parity case).
//!
//! Shadowrun has **no** fiction-critical scene state (no Obfuscate concealment,
//! no ST pending actions — initiative + Edge/Drain live on the persisted sheet),
//! so for parity with `ruleset-wod` it persists exactly **one** subsystem: the
//! in-progress extended rolls. A mid-scene hot restart (M16: the gateway holds
//! connections while `mush-server` restarts and reloads the world from Redis)
//! must keep an accumulating extended test's successes. M17 mirrors the extended
//! map to the `mod:shadowrun:extended` doc on every mutation and re-hydrates it in
//! [`ShadowrunRuleset::on_world_loaded`].
//!
//! This drives the **real** command path (`+roll … extended N`), captures the
//! persistence ops it emits, round-trips them through a real [`InMemoryStore`],
//! then REBUILDS a fresh engine and runs the single `modules_loaded` boot path
//! (the same one that runs `on_world_loaded`), and asserts:
//!
//! - an in-progress extended roll survives with its accumulated successes and
//!   its shrinking pool, and the next `+roll … extended N` CONTINUES it (it is
//!   not reset);
//! - an ORPHANED entry (its object destroyed while down) is DROPPED on reload,
//!   not resurrected, and the corrected doc is re-persisted;
//! - the RNG is NOT persisted (a fresh roll reproduces the seed-0 stream) and
//!   the transient roll-log ring is NOT persisted (it comes back empty).

use std::collections::HashMap;

use mush_core::{
    Audience, Effects, Engine, GOD, GameModule, Inbound, LIMBO, ModuleBootData, ModuleOp, ObjId,
    ObjKind, Outbox,
};
use mush_dice::sr5::{Sr5Extended, Sr5Opts, Sr5Outcome, Sr5Pool};
use mush_jobs::JobsModule;
use mush_proto::OutputEvent;
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use mush_testkit::{TestWorld, boot_world};
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use ruleset_shadowrun::ShadowrunRuleset;
use ruleset_shadowrun::state::Sr5State;
use smol_str::SmolStr;

/// A fixed RNG seed shared by both engine instances (register-time), so the
/// reboot's fresh RNG is at the same seed-0 position as a pristine world — the
/// basis of the "RNG was not persisted" check.
const S: u64 = 4242;

/// The modules under test: SR needs the jobs module for its approval queues.
fn sr_modules() -> Vec<Box<dyn GameModule>> {
    vec![
        Box::new(JobsModule::new()),
        Box::new(ShadowrunRuleset::with_seed(S)),
    ]
}

// ----------------------------------------------------------------------
// Raw-engine step helpers (the reboot runs `modules_loaded` exactly once, so it
// cannot use `TestWorld`, whose constructor first-boots the modules and would
// double-register SR's approval payload handlers).
// ----------------------------------------------------------------------

fn conn(engine: &mut Engine, actor: ObjId) {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.step(
        Inbound::ConnUp {
            conn: u64::from(actor.0),
            actor,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
}

fn cmd(engine: &mut Engine, actor: ObjId, line: &str) -> Vec<(Audience, OutputEvent)> {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.step(
        Inbound::Cmd {
            conn: 0,
            actor,
            line: line.to_string(),
        },
        &mut out,
        &mut fx,
    );
    out.take()
}

fn state(engine: &Engine) -> &Sr5State {
    engine.world.module_state::<Sr5State>().expect("Sr5State")
}

fn tw_state(tw: &TestWorld) -> &Sr5State {
    tw.world().module_state::<Sr5State>().expect("Sr5State")
}

/// Round-trip a batch of module ops through a real store and shape it back into
/// the `shadowrun`-namespace boot data (exactly what the driver hands
/// `modules_loaded`). The namespace is the module NAME (`"shadowrun"`) — the key
/// both the write path and the boot loader (`module_json_list(module.name())`)
/// agree on.
async fn roundtrip(ops: Vec<ModuleOp>) -> HashMap<SmolStr, ModuleBootData> {
    let store = InMemoryStore::new();
    store
        .write(WriteBatch {
            serial: 1,
            module_ops: ops,
            ..WriteBatch::default()
        })
        .await
        .expect("store write");
    let kv = store
        .module_json_list("shadowrun")
        .await
        .expect("module_json_list");
    let boot = ModuleBootData {
        kv: kv.into_iter().map(|(k, v)| (SmolStr::new(k), v)).collect(),
        streams: HashMap::new(),
        ..Default::default()
    };
    HashMap::from([(SmolStr::new("shadowrun"), boot)])
}

// ----------------------------------------------------------------------
// The proof
// ----------------------------------------------------------------------

#[tokio::test]
async fn sr_extended_roll_survives_a_hot_restart() {
    // === Phase 1: run an extended test in the first engine ===
    let mut tw = TestWorld::with_modules(sr_modules());

    // Players in a fixed creation order so the reboot recreates the exact ids.
    // Ghost is created LAST so omitting it on reboot (the orphan) does not shift
    // any other id.
    let nick = tw.spawn_player("Nick");
    let probe = tw.spawn_player("Probe");
    let ghost = tw.spawn_player("Ghost");

    // Sheets: Nick + Ghost each get a Logic 5 + Computer 5 pool (= 10 dice).
    for who in ["Nick", "Ghost"] {
        tw.run(GOD, &format!("@sheet/set {who}/logic=5"));
        tw.run(GOD, &format!("@sheet/set {who}/active_skills:computer=5"));
    }

    // One interval of an extended test toward 20 hits (an unreachable threshold,
    // so it stays in-progress). Both Nick and Ghost start one.
    tw.run(nick, "+roll logic+computer extended 20");
    tw.run(ghost, "+roll logic+computer extended 20");

    // Capture the pre-restart truth to compare against.
    let pre = tw_state(&tw);
    let nick_ext: Sr5Extended = pre.extended(nick).expect("Nick extended roll").clone();
    assert_eq!(nick_ext.threshold, 20);
    assert_eq!(nick_ext.rolls.len(), 1, "one interval recorded");
    assert_eq!(
        nick_ext.next_pool, 9,
        "the pool shrank by one after interval 1"
    );
    assert!(pre.extended(ghost).is_some(), "Ghost extended set");
    assert!(
        !pre.recent_logs(nick, 1).is_empty(),
        "Nick has a logged roll pre-restart"
    );

    // The write-through ops the scene emitted, folded into one store batch.
    let ops = tw.take_module_ops();
    assert!(
        ops.iter().any(
            |o| matches!(&o.op, mush_core::ModuleWrite::JsonSet { key, .. } if key == "extended")
        ),
        "the extended roll emitted an extended doc"
    );
    let boot = roundtrip(ops).await;

    // === Phase 2: the hot restart — a fresh engine, world reloaded, one boot ===
    let (world, registry) = boot_world();
    let mut engine = Engine::with_modules(world, registry, sr_modules()).expect("engine builds");
    conn(&mut engine, GOD);

    // Recreate the exact object graph, OMITTING Ghost (destroyed while down —
    // the orphan). Same creation order ⇒ same ids.
    let ids: Vec<ObjId> = ["Nick", "Probe"]
        .iter()
        .map(|n| {
            engine
                .world
                .create(ObjKind::Player, n, GOD, Some(LIMBO))
                .expect("recreate player")
        })
        .collect();
    assert_eq!(
        ids,
        vec![nick, probe],
        "the reboot recreates the same object ids"
    );
    for &p in &ids {
        conn(&mut engine, p);
    }
    // Nick needs his sheet back to continue the extended test after the restart.
    cmd(&mut engine, GOD, "@sheet/set Nick/logic=5");
    cmd(&mut engine, GOD, "@sheet/set Nick/active_skills:computer=5");

    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine
        .modules_loaded(boot, &mut out, &mut fx)
        .expect("the sr boot data rehydrates");

    // The corrective re-persist: Ghost (orphan) was pruned, so the boot re-wrote
    // the corrected extended doc.
    let fixups = engine.take_module_ops();
    assert!(
        fixups.iter().any(
            |o| matches!(&o.op, mush_core::ModuleWrite::JsonSet { key, .. } if key == "extended")
        ),
        "pruning an orphan entry re-persists the corrected extended doc: {fixups:?}"
    );

    // --- The in-progress extended roll survived with its accumulated successes ---
    let post = state(&engine);
    assert_eq!(
        post.extended(nick).cloned(),
        Some(nick_ext.clone()),
        "the in-progress extended roll survived identically"
    );

    // --- The orphan was dropped, not resurrected ---
    assert!(
        post.extended(ghost).is_none(),
        "Ghost was destroyed while down: the orphan extended roll is DROPPED"
    );

    // --- The transient log ring was NOT persisted (it re-hydrates empty) ---
    assert!(
        post.recent_logs(nick, 1).is_empty(),
        "logs are not persisted (Nick)"
    );

    // --- The RNG was NOT persisted (re-seeded on boot). A fresh roll on the
    // reboot reproduces the pristine seed-0 stream; had the advanced RNG been
    // restored it would not match. Probe rolls FIRST, before any other RNG use. ---
    let mut expect_rng = ChaCha8Rng::seed_from_u64(S);
    let faces: Vec<u8> = (0..5)
        .map(|_| (expect_rng.next_u32() % 6) as u8 + 1)
        .collect();
    let expected = Sr5Pool::score(&faces, &Sr5Opts::default());
    cmd(&mut engine, probe, "+roll 5");
    let got: Sr5Outcome = state(&engine)
        .recent_logs(probe, 1)
        .first()
        .expect("Probe logged a roll")
        .outcome
        .clone();
    assert_eq!(
        got, expected,
        "the reboot RNG is re-seeded from S (not the persisted, advanced stream)"
    );

    // --- The surviving extended roll CONTINUES (it is not reset): the next
    // interval accumulates on top of the restored total and shrinks the pool
    // further (10 → 9 pre-restart, now 9 → 8). ---
    cmd(&mut engine, nick, "+roll logic+computer extended 20");
    let cont = state(&engine)
        .extended(nick)
        .expect("Nick extended continues")
        .clone();
    assert_eq!(
        cont.threshold, 20,
        "same test (not reset to a fresh threshold)"
    );
    assert_eq!(
        cont.rolls.len(),
        2,
        "the restored interval plus the new one"
    );
    assert_eq!(
        cont.next_pool, 8,
        "the pool kept shrinking from the restored 9"
    );
    assert!(
        cont.accumulated >= nick_ext.accumulated,
        "the new interval accumulated on top of the restored successes"
    );
}
