//! M17 — the WoD scene-state hot-restart proof.
//!
//! A mid-scene hot restart (M16: the gateway holds connections while
//! `mush-server` restarts and reloads the world from Redis) must be INVISIBLE.
//! WoD keeps its live scene state in the [`WodState`] slot, which a restart
//! drops — most visibly, a cloaked vampire pops into view. M17 mirrors the
//! scene-invisible subset (concealment + pierced verdicts, accepted-but-unthrown
//! M13 pendings, in-progress extended rolls) to the `mod:wod:*` docs on every
//! mutation and re-hydrates it in [`WodRuleset::on_world_loaded`].
//!
//! This test drives the **real** command / mutation paths (`+obfuscate on`,
//! `+roll … ext`, `+cast … proposing …` → ruling completion), captures the
//! persistence ops those emit, round-trips them through a real
//! [`InMemoryStore`], then REBUILDS a fresh engine and runs the single
//! `modules_loaded` boot path (the same one that runs `on_world_loaded`), and
//! asserts every invariant the milestone names:
//!
//! - **S1** concealment + verdicts survive at the same rating (the cloaked
//!   vampire stays hidden);
//! - **S2** an accepted pending survives and the next `+roll` consumes it as
//!   ST-authoritative; an EXPIRED pending is dropped; an ORPHANED entry (its
//!   object destroyed while down) is dropped; an in-progress extended roll
//!   survives with its accumulated successes;
//! - **S3** the RNG is NOT persisted (a fresh roll reproduces the seed-0 stream)
//!   and the transient roll-log ring is NOT persisted (it comes back empty).

use std::collections::HashMap;

use mush_core::{
    Audience, Effect, Effects, Engine, GOD, GameModule, Inbound, LIMBO, ModuleBootData, ModuleOp,
    ObjId, ObjKind, Outbox,
};
use mush_dice::owod20::{ExtendedRoll, Owod20Opts, Owod20Outcome, Owod20Pool};
use mush_jobs::JobsModule;
use mush_proto::OutputEvent;
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use mush_testkit::{TestWorld, boot_world};
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use ruleset_wod::WodRuleset;
use ruleset_wod::st::{ActionKind, Ruling, RulingResult, StResult};
use ruleset_wod::state::WodState;
use smol_str::SmolStr;

/// A fixed RNG seed shared by both engine instances (register-time), so the
/// reboot's fresh RNG is at the same seed-0 position as a pristine world — the
/// basis of the "RNG was not persisted" check.
const S: u64 = 4242;

/// The modules under test: WoD needs the jobs module for its approval queues.
fn wod_modules() -> Vec<Box<dyn GameModule>> {
    vec![
        Box::new(JobsModule::new()),
        Box::new(WodRuleset::with_seed(S)),
    ]
}

// ----------------------------------------------------------------------
// Raw-engine step helpers (the reboot runs `modules_loaded` exactly once, so
// it cannot use `TestWorld`, whose constructor first-boots the modules and
// would double-register WoD's approval payload handlers).
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

/// Step a command on a `TestWorld` capturing the deferred effects too (needed to
/// read the correlation an `+cast` deferred).
fn step_cmd(tw: &mut TestWorld, actor: ObjId, line: &str) -> Vec<Effect> {
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
    fx.drain()
}

/// The correlation id of the one `Effect::ModuleAsync` a propose deferred.
fn deferred_correlation(effects: &[Effect]) -> u64 {
    effects
        .iter()
        .find_map(|e| match e {
            Effect::ModuleAsync { correlation, .. } => Some(*correlation),
            _ => None,
        })
        .expect("a proposal defers one Effect::ModuleAsync")
}

/// Re-enter a ruling completion on a `TestWorld` (the ST adjudication re-entry
/// the driver performs). `payload` is a serialized [`StResult`].
fn apply_ruling(tw: &mut TestWorld, actor: ObjId, correlation: u64, payload: String) {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(
        Inbound::Apply(mush_core::WorldCommand::ModuleAsyncDone {
            module: "wod".into(),
            actor,
            correlation,
            payload,
        }),
        &mut out,
        &mut fx,
    );
}

/// An accepting Mage ruling at `difficulty` (arete pool), serialized as the
/// completion payload — the same shape `run_st_async` returns, built directly so
/// the proof needs no LLM.
fn accept_ruling(difficulty: u8) -> String {
    serde_json::to_string(&StResult::Ruling(RulingResult {
        freeform: "a working".into(),
        proposed_spheres: vec!["Forces".into()],
        proposed_pool: "arete".into(),
        vulgar: false,
        ruling: Ruling {
            accept: true,
            spheres_valid: true,
            pool: "arete".into(),
            difficulty,
            witness_difficulty: 6,
            ..Ruling::default()
        },
        kind: ActionKind::Mage,
        error: None,
    }))
    .expect("ruling result serializes")
}

fn state(engine: &Engine) -> &WodState {
    engine.world.module_state::<WodState>().expect("WodState")
}

fn tw_state(tw: &TestWorld) -> &WodState {
    tw.world().module_state::<WodState>().expect("WodState")
}

/// Round-trip a batch of module ops through a real store and shape it back into
/// the `wod`-namespace boot data (exactly what the driver hands `modules_loaded`).
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
        .module_json_list("wod")
        .await
        .expect("module_json_list");
    let boot = ModuleBootData {
        kv: kv.into_iter().map(|(k, v)| (SmolStr::new(k), v)).collect(),
        streams: HashMap::new(),
        ..Default::default()
    };
    HashMap::from([(SmolStr::new("wod"), boot)])
}

// ----------------------------------------------------------------------
// The proof
// ----------------------------------------------------------------------

#[tokio::test]
async fn wod_scene_state_survives_a_hot_restart() {
    // === Phase 1: live a scene in the first engine ===
    let mut tw = TestWorld::with_modules(wod_modules());

    // Players, created in a fixed order so the reboot can recreate the exact
    // object ids. Ghost is created LAST so omitting it on reboot (the orphan)
    // does not shift any other id.
    let vlad = tw.spawn_player("Vlad");
    let seer = tw.spawn_player("Seer");
    let mort = tw.spawn_player("Mort");
    let nick = tw.spawn_player("Nick");
    let stale = tw.spawn_player("Stale");
    let rae = tw.spawn_player("Rae");
    let probe = tw.spawn_player("Probe");
    let ghost = tw.spawn_player("Ghost");

    // Sheets (all in Limbo together).
    tw.run(GOD, "@sheet/set Vlad/template=vampire");
    tw.run(GOD, "@sheet/set Vlad/disciplines:Obfuscate=2");
    tw.run(GOD, "@sheet/set Seer/template=vampire");
    tw.run(GOD, "@sheet/set Seer/perception=5");
    tw.run(GOD, "@sheet/set Seer/awareness=5");
    tw.run(GOD, "@sheet/set Seer/disciplines:Auspex=5");
    tw.run(GOD, "@sheet/set Mort/perception=5");
    tw.run(GOD, "@sheet/set Mort/awareness=5");
    tw.run(GOD, "@sheet/set Nick/template=mage");
    tw.run(GOD, "@sheet/set Nick/arete=4");
    tw.run(GOD, "@sheet/set Nick/spheres:Forces=2");
    tw.run(GOD, "@sheet/set Stale/template=mage");
    tw.run(GOD, "@sheet/set Stale/arete=4");
    tw.run(GOD, "@sheet/set Stale/spheres:Forces=2");
    tw.run(GOD, "@sheet/set Rae/strength=3");
    tw.run(GOD, "@sheet/set Rae/brawl=2");
    tw.run(GOD, "@sheet/set Ghost/template=vampire");
    tw.run(GOD, "@sheet/set Ghost/disciplines:Obfuscate=3");

    tw.set_now(1000);

    // (S1) Two vampires cloak. Every present observer rolls once to notice.
    tw.run(vlad, "+obfuscate on");
    tw.run(ghost, "+obfuscate on");

    // (S2 extended) An in-progress extended roll toward 5 successes.
    tw.run(rae, "+roll strength+brawl ext 5");

    // (S2 pending, expired) Stale proposes at t=1000 → pending expires at 1300.
    let stale_fx = step_cmd(&mut tw, stale, "+cast Kindle a flame proposing Forces");
    apply_ruling(
        &mut tw,
        stale,
        deferred_correlation(&stale_fx),
        accept_ruling(5),
    );
    assert!(tw_state(&tw).pending(stale).is_some(), "Stale pending set");

    // (S2 pending, survives) Nick proposes at t=10000 → pending expires at 10300.
    tw.set_now(10_000);
    let nick_fx = step_cmd(&mut tw, nick, "+cast Hurl a bolt of fire proposing Forces");
    apply_ruling(
        &mut tw,
        nick,
        deferred_correlation(&nick_fx),
        accept_ruling(7),
    );

    // Capture the pre-restart truth to compare against.
    let pre = tw_state(&tw);
    assert_eq!(
        pre.concealed_rating(vlad),
        Some(2),
        "Vlad cloaked at rating 2"
    );
    assert_eq!(
        pre.concealed_rating(ghost),
        Some(3),
        "Ghost cloaked at rating 3"
    );
    let seer_v = pre.perception_verdict(seer, vlad);
    let mort_v = pre.perception_verdict(mort, vlad);
    assert_eq!(
        mort_v,
        Some(false),
        "the no-Auspex mortal cannot pierce Vlad"
    );
    let rae_ext: ExtendedRoll = pre.extended(rae).expect("Rae extended roll").clone();
    assert_eq!(rae_ext.target, 5);
    assert_eq!(rae_ext.rolls.len(), 1, "one extended roll recorded");
    // The FULL pending, captured to prove every field round-trips through serde
    // on reload (not just difficulty) — a dropped field would fail the assert below.
    let nick_pending_pre = pre.pending(nick).cloned().expect("Nick pending");
    assert_eq!(nick_pending_pre.difficulty, 7);
    // The seer + Rae logged rolls (perception pierce / the extended throw) — used
    // below to prove the log ring is NOT persisted.
    assert!(
        !pre.recent_logs(rae, 1).is_empty(),
        "Rae has a logged roll pre-restart"
    );

    // The write-through ops the scene emitted, folded into one store batch.
    let ops = tw.take_module_ops();
    assert!(
        ops.iter().any(
            |o| matches!(&o.op, mush_core::ModuleWrite::JsonSet { key, .. } if key == "conceal")
        ),
        "the cloak emitted a conceal doc"
    );
    let boot = roundtrip(ops).await;

    // === Phase 2: the hot restart — a fresh engine, world reloaded, one boot ===
    let (world, registry) = boot_world();
    let mut engine = Engine::with_modules(world, registry, wod_modules()).expect("engine builds");
    conn(&mut engine, GOD);

    // Recreate the exact object graph, OMITTING Ghost (destroyed while down —
    // the orphan). Same creation order ⇒ same ids.
    let ids: Vec<ObjId> = ["Vlad", "Seer", "Mort", "Nick", "Stale", "Rae", "Probe"]
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
        vec![vlad, seer, mort, nick, stale, rae, probe],
        "the reboot recreates the same object ids"
    );
    for &p in &ids {
        conn(&mut engine, p);
    }
    // Only the actors that ROLL after the restart need their sheet back (the
    // rest need only to exist, for the liveness check).
    cmd(&mut engine, GOD, "@sheet/set Nick/template=mage");
    cmd(&mut engine, GOD, "@sheet/set Nick/arete=4");
    cmd(&mut engine, GOD, "@sheet/set Nick/spheres:Forces=2");
    cmd(&mut engine, GOD, "@sheet/set Probe/strength=3");
    cmd(&mut engine, GOD, "@sheet/set Probe/brawl=2");

    // Reload now sits AFTER Stale's expiry (1300) but BEFORE Nick's (10300).
    engine.set_now(5000);
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine
        .modules_loaded(boot, &mut out, &mut fx)
        .expect("the wod boot data rehydrates");

    // The corrective re-persist: Ghost (orphan) and Stale (expired) were pruned,
    // so the boot re-wrote the corrected conceal + pending docs.
    let fixups = engine.take_module_ops();
    assert!(
        fixups.iter().any(
            |o| matches!(&o.op, mush_core::ModuleWrite::JsonSet { key, .. } if key == "conceal")
        ) && fixups.iter().any(
            |o| matches!(&o.op, mush_core::ModuleWrite::JsonSet { key, .. } if key == "pending")
        ),
        "pruning an orphan/expired entry re-persists the corrected docs: {fixups:?}"
    );

    // --- S1: concealment + verdicts survived at the same rating ---
    let post = state(&engine);
    assert_eq!(
        post.concealed_rating(vlad),
        Some(2),
        "the cloaked vampire is STILL concealed at the same rating (the bug is fixed)"
    );
    assert!(post.is_concealed(vlad));
    assert_eq!(
        post.perception_verdict(seer, vlad),
        seer_v,
        "seer verdict intact"
    );
    assert_eq!(
        post.perception_verdict(mort, vlad),
        mort_v,
        "mortal verdict intact"
    );
    assert_eq!(post.perception_verdict(mort, vlad), Some(false));

    // --- S2: orphan dropped, not resurrected ---
    assert!(
        !post.is_concealed(ghost),
        "Ghost was destroyed while down: the orphan concealment is DROPPED"
    );

    // --- S2: extended roll survived with its accumulated successes ---
    assert_eq!(
        post.extended(rae).cloned(),
        Some(rae_ext),
        "the in-progress extended roll survived identically"
    );

    // --- S2: pending survived (Nick), expired dropped (Stale) ---
    // Assert the WHOLE PendingAction round-tripped (every field), not just
    // difficulty — a serde regression that silently dropped a field would fail here.
    assert_eq!(
        post.pending(nick),
        Some(&nick_pending_pre),
        "the accepted-but-unthrown ruling survived the restart with every field intact"
    );
    assert!(
        post.pending(stale).is_none(),
        "the expired pending is dropped on reload (stays expired)"
    );

    // --- S3: the log ring was NOT persisted (it re-hydrates empty) ---
    assert!(
        post.recent_logs(rae, 1).is_empty(),
        "logs are not persisted (Rae)"
    );
    assert!(
        post.recent_logs(seer, 1).is_empty(),
        "logs are not persisted (Seer)"
    );

    // --- S3: the RNG was NOT persisted (re-seeded on boot). A fresh roll on the
    // reboot reproduces the pristine seed-0 stream; had the advanced RNG been
    // restored it would not match. Probe rolls FIRST, before any other RNG use. ---
    let mut expect_rng = ChaCha8Rng::seed_from_u64(S);
    let faces: Vec<u8> = (0..5)
        .map(|_| (expect_rng.next_u32() % 10) as u8 + 1)
        .collect();
    let expected = Owod20Pool::score(&faces, &Owod20Opts::new(6));
    cmd(&mut engine, probe, "+roll strength+brawl");
    let got: Owod20Outcome = state(&engine)
        .recent_logs(probe, 1)
        .first()
        .expect("Probe logged a roll")
        .outcome
        .clone();
    assert_eq!(
        got, expected,
        "the reboot RNG is re-seeded from S (not the persisted, advanced stream)"
    );

    // --- S2: the surviving pending is consumed as ST-authoritative ---
    cmd(&mut engine, nick, "+roll/silent");
    let nick_roll = state(&engine)
        .recent_logs(nick, 1)
        .first()
        .expect("Nick logged the ST-authoritative throw")
        .outcome
        .clone();
    assert_eq!(
        nick_roll.difficulty, 7,
        "the next +roll runs the RULING's difficulty (ST-authoritative)"
    );
    assert!(
        state(&engine).pending(nick).is_none(),
        "the pending is consumed exactly once"
    );
}
