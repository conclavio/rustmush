//! **The seam between the synchronous `Arena::step` and the sim task's think
//! pool — the gate of 17b cut M4.1** (doc 17 §6 "Execution policy",
//! "Snapshot and resume"; D-UM-22, D-UM-23; 17b §3's M4 row, findings (1),
//! (2), (10) and (11)).
//!
//! A think is collected at its lattice millisecond (`Arena::collect_think`)
//! and its answer written at the head of the next substep
//! (`Arena::settle_collected`) — by the task's delivery of the pool's
//! answers before its next step (`Arena::deliver_answers`), or inline at that
//! substep's head inside `Arena::step` (`Arena::settle_inline`). The claim
//! these fixtures hold the seam to: **the per-substep hash sequence and the
//! tape are one function of sim time**, whichever thread ran the runner and
//! however the wall cut the pulses — so a run stepped inline, a run whose
//! batches a real `MindPool` answers at one, two and four workers, a run of
//! one substep a pulse and a run through the real arena task on tokio's
//! clock are one event list and one hash sequence.
//!
//! * [`m4_one_jittered_wake_sequence_hashes_the_same_inline_and_pooled`] — ONE
//!   seeded sequence of lattice-multiple wakes (one to eight substeps a
//!   pulse — never a remainder against the lattice, which moves views and
//!   shots on its own) stepped five ways on a mixed field (a native and a
//!   scripted mind a side, an unarmed one among them).
//! * [`m4_real_tasks_at_one_two_and_four_workers_hash_as_the_library`] — the
//!   same field through [`tacsim::arena_task::spawn_arena`] on a private
//!   pool of one, two and four workers over three seeds, on tokio's paused
//!   clock, held to the library driven on the task's own wake rule — the
//!   tape, the end, the ledger digest and every snapshot's state hash.
//! * [`m4_an_unanswered_think_refuses_the_snapshot_by_name`] — `Snapshot::encode`
//!   while a think awaits its answer.
//! * [`m4_a_fault_s_lines_carry_the_think_s_own_substep_hash_at_three_cadences`] —
//!   the `mind_fault` and `mind_dropped` lines under the think's substep's
//!   end hash, one tape at three cadences.
//! * The twin's pooled lane through real tasks ([`m4_real_tasks_over_the_twin_s_thirty_hash_as_the_library`],
//!   fifteen matches since 17b cut M4.5a) is `#[ignore]`d — the gate run, five
//!   times by hand in release.

use crate::support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mush_core::ObjId;
use smol_str::SmolStr;
use support::{Muster, SEED, booted_world, install_sealed, sealed_sources, shipped_custody_seeded};
use tacsim::arena::hash::state_hash;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::mind::{MindRunner, ThinkFault, ThinkJob, ThinkResult, run_batch_inline};
use tacsim::arena::snapshot::SnapshotError;
use tacsim::arena::state::{Arena, CustodyOut, Team};
use tacsim::arena::step::StepOutput;
use tacsim::arena::{EndReason, Snapshot};
use tacsim::arena_task::{ArenaOut, MatchId, spawn_arena};
use tacsim::config::{ArenaConfig, MatchConfig, MindsConfig, SensorsConfig};
use tacsim::minds::MindPool;
use tokio::sync::mpsc;

/// **The mixed field**: red a native stock mind and a scripted Standard
/// Doctrine, blue a native stock mind and an unarmed one — so a Rhai runner
/// thinks on the pool's threads beside the native one, and an unarmed hull's
/// helm-only thinks ride the same batches.
fn red() -> [Muster; 2] {
    [Muster::mind(Team::Red, 1), Muster::scripted(Team::Red, 2)]
}

/// See [`red`].
fn blue() -> [Muster; 2] {
    [
        Muster::mind(Team::Blue, 1),
        Muster::mind_unarmed(Team::Blue, 2),
    ]
}

/// The custody of the mixed field at `seed`.
fn custody(world: &mush_testkit::TestWorld, seed: u64) -> CustodyOut {
    shipped_custody_seeded(world, &[&red(), &blue()], seed)
}

/// **The match rules every run here plays under**: the shipped rules with
/// the match ended by its clock at `cap_ms` (a `Timeout`), so a run that
/// nobody wins still ends.
fn rules(cap_ms: u32) -> MatchConfig {
    MatchConfig {
        max_duration_ms: cap_ms,
        ..MatchConfig::default()
    }
}

/// **The arena of `custody`, as the task opens it**: `Arena::new` on the
/// shipped configs, the sealed runners installed.
fn open(custody: CustodyOut, match_cfg: &MatchConfig) -> Arena {
    let sources = sealed_sources(&custody.units);
    let mut arena = Arena::new(
        custody,
        &ArenaConfig::default(),
        match_cfg,
        &SensorsConfig::default(),
    )
    .expect("the mixed field checks out");
    install_sealed(&mut arena, &sources);
    arena
}

/// The substep hashes of a run with the millisecond each substep ended at.
type SubstepEnds = Vec<(i64, u64)>;

/// One tape line as `(t_ms, kind as JSON, hash)`.
fn line(event: &LedgerEvent) -> (i64, String, u64) {
    (
        event.t_ms,
        serde_json::to_string(&event.kind).unwrap_or_default(),
        event.hash,
    )
}

/// Every line the bytes hold.
fn lines_of(bytes: &[u8]) -> Vec<(i64, String, u64)> {
    String::from_utf8(bytes.to_vec())
        .unwrap_or_default()
        .lines()
        .filter_map(|text| serde_json::from_str::<LedgerEvent>(text).ok())
        .map(|event| line(&event))
        .collect()
}

/// **What one run came to**: every substep hash in order, every tape line,
/// the end, the final state hash, the ledger digest, and — for the library
/// runs — the state hash after every pulse keyed by the arena millisecond
/// (what a snapshot taken then carries).
#[derive(Clone, Debug)]
struct Run {
    hashes: Vec<u64>,
    lines: Vec<(i64, String, u64)>,
    ended: Option<EndReason>,
    state_hash: u64,
    ledger_digest: [u8; 32],
    after_pulse: BTreeMap<i64, u64>,
    pulses: u32,
    thinks: u32,
    /// Pulses that handed thinks out (`StepOutput::thinks` non-empty).
    pulses_with_thinks: u32,
    /// Pulses of more than one substep (the inline path inside `step`).
    pulses_of_many: u32,
}

/// Hold two runs equal line for line, hash for hash.
fn assert_same(name: &str, a: &Run, b: &Run) {
    if let Some(at) =
        (0..a.lines.len().max(b.lines.len())).find(|at| a.lines.get(*at) != b.lines.get(*at))
    {
        panic!(
            "{name}: the tapes part at line {at}:\n  left:  {:?}\n  right: {:?}",
            a.lines.get(at),
            b.lines.get(at)
        );
    }
    assert_eq!(a.hashes, b.hashes, "{name}: the substep hash sequence");
    assert_eq!(a.ended, b.ended, "{name}: the end");
    assert_eq!(a.state_hash, b.state_hash, "{name}: the final state hash");
    assert_eq!(
        a.ledger_digest, b.ledger_digest,
        "{name}: the ledger digest"
    );
}

/// **How a library run answers its thinks between pulses.**
#[derive(Clone, Copy, Debug)]
enum Settle {
    /// `Arena::step` alone: the next step's head runs them inline.
    Inline,
    /// `Arena::step_settled`: run inline right after the step.
    Settled,
    /// `Arena::step`, the batch on a real pool of this many workers, awaited
    /// and delivered before the next step — the task's path, done by hand.
    Pooled(u32),
}

/// **One library run** of `arena` on the wake sequence `wakes` answers (the
/// wall of each pulse after the first at wall 0; `None` ends the run), its
/// thinks answered by `settle`.
fn run(
    mut arena: Arena,
    settle: Settle,
    mut wakes: impl FnMut(&Arena, &StepOutput, i64) -> Option<i64>,
) -> Run {
    let pool = match settle {
        Settle::Pooled(workers) => Some(MindPool::new(workers)),
        Settle::Inline | Settle::Settled => None,
    };
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut after_pulse: BTreeMap<i64, u64> = BTreeMap::new();
    let (mut pulses, mut thinks, mut pulses_with_thinks, mut pulses_of_many) = (0, 0, 0, 0);
    let mut wall = 0_i64;
    let ended = loop {
        let out = match settle {
            Settle::Settled => arena.step_settled(wall, &[], &[]),
            Settle::Inline | Settle::Pooled(_) => arena.step(wall, &[], &[]),
        };
        pulses += 1;
        thinks += out.motor.thinks;
        if out.substeps > 1 {
            pulses_of_many += 1;
        }
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        hashes.extend(arena.substep_hashes.iter().copied());
        if !out.thinks.is_empty() {
            pulses_with_thinks += 1;
            if let Some(pool) = &pool {
                let answers = pool
                    .submit(out.thinks.clone())
                    .wait()
                    .expect("no runner panics here");
                let settled = arena.deliver_answers(answers);
                assert_eq!(
                    settled.written,
                    u32::try_from(out.thinks.len()).unwrap_or(0)
                );
                assert_eq!(settled.dropped, 0);
            }
        }
        // `Settle::Inline` leaves them for the next step's head; the state
        // hash after this pulse is therefore read AFTER an inline settle of
        // its own, so every run keys the same fact: the arena between two of
        // the task's pulses, every answer written.
        if matches!(settle, Settle::Inline) && !arena.thinks_outstanding().is_empty() {
            // Not settled: the next `step` does it. (The per-pulse hash below
            // is skipped for this run; the final one settles first.)
        } else {
            after_pulse.insert(arena.now_ms, state_hash(&arena));
        }
        if out.ended.is_some() {
            break out.ended;
        }
        match wakes(&arena, &out, wall) {
            Some(next) => wall = next,
            None => break None,
        }
    };
    if matches!(settle, Settle::Inline) {
        let answers = run_batch_inline(arena.thinks_outstanding());
        let _ = arena.deliver_answers(answers);
    }
    // A match that ended inside a substep that collected a think leaves that
    // think awaiting for good: nothing is written after the end (its hull
    // stays in `awaiting`, its job in `thinks_outstanding`; a hull destroyed
    // in that same substep left `awaiting` but its job stays). A run that did
    // not end has every think answered.
    if ended.is_none() {
        assert!(arena.awaiting.is_empty(), "every think answered at the end");
    } else {
        let outstanding: std::collections::BTreeSet<ObjId> = arena
            .thinks_outstanding()
            .iter()
            .map(|think| think.unit)
            .collect();
        assert!(
            arena.awaiting.iter().all(|unit| outstanding.contains(unit)),
            "every hull awaiting at the end has its think among the ending substep's: {:?} within {outstanding:?}",
            arena.awaiting
        );
    }
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    Run {
        hashes,
        lines: lines_of(&bytes),
        ended: ended.map(|end| end.reason),
        state_hash: state_hash(&arena),
        ledger_digest: arena.ledger.digest(),
        after_pulse,
        pulses,
        thinks,
        pulses_with_thinks,
        pulses_of_many,
    }
}

/// **A seeded sequence of lattice-multiple wakes**: each pulse one to eight
/// substeps (250 ms each on the shipped ground) after the last, from a
/// linear congruential draw on `seed` — never a remainder against the
/// lattice. Stops at `until_ms`.
fn jittered(seed: u64, until_ms: i64) -> impl FnMut(&Arena, &StepOutput, i64) -> Option<i64> {
    let mut state = seed;
    move |arena: &Arena, _: &StepOutput, wall: i64| {
        if arena.now_ms >= until_ms {
            return None;
        }
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let substeps = i64::try_from((state >> 33) % 8).unwrap_or(0) + 1;
        // The last pulse lands on `until_ms` exactly (a lattice point), so
        // every run of this sequence ends at one arena millisecond.
        Some((wall + substeps * arena.substep_len_ms()).min(until_ms))
    }
}

/// **One substep a pulse**, to `until_ms`.
fn every_substep(until_ms: i64) -> impl FnMut(&Arena, &StepOutput, i64) -> Option<i64> {
    move |arena: &Arena, _: &StepOutput, wall: i64| {
        (arena.now_ms < until_ms).then(|| wall + arena.substep_len_ms())
    }
}

/// **The task's own wake rule**: the next pulse at the step's own
/// `next_due_ms` — what `Task::pulse` sleeps to (`deadline(base, next_due_ms)`,
/// which the paused clock reaches exactly), in the wall milliseconds the
/// step was given.
fn task_rule() -> impl FnMut(&Arena, &StepOutput, i64) -> Option<i64> {
    |_: &Arena, out: &StepOutput, _: i64| Some(out.next_due_ms)
}

/// **17b cut M4.1 — the seam's one hash sequence** (doc 17 §6 "Execution
/// policy", D-UM-22; the M4 row's gate, item (i)): the mixed field at
/// [`SEED`] on ONE seeded sequence of lattice-multiple wakes (one to eight
/// substeps a pulse) over ninety arena seconds, stepped: with `Arena::step`
/// alone (every think of a pulse's last substep written inline at the next
/// step's head); with `Arena::step_settled`; with a real [`MindPool`] of
/// one, two and four workers answering each pulse's batch before the next
/// step (the task's path by hand); and at one substep a pulse — one tape,
/// one substep hash sequence, one end, one final state hash, one ledger
/// digest. The jittered runs cut pulses of several substeps (the inline
/// path inside `step`) and hand thinks out of others (the pooled path), and
/// every run thought.
#[test]
fn m4_one_jittered_wake_sequence_hashes_the_same_inline_and_pooled() {
    let world = booted_world();
    let cap = 300_000;
    let until = 90_000;
    let inline = run(
        open(custody(&world, SEED), &rules(cap)),
        Settle::Inline,
        jittered(SEED, until),
    );
    assert!(inline.thinks > 0, "the field thought");
    assert!(
        inline.pulses_with_thinks > 0,
        "some pulse handed thinks out"
    );
    assert!(inline.pulses_of_many > 0, "some pulse ran several substeps");
    assert!(inline.pulses < inline.hashes.len().try_into().unwrap_or(u32::MAX));
    let settled = run(
        open(custody(&world, SEED), &rules(cap)),
        Settle::Settled,
        jittered(SEED, until),
    );
    assert_same("step_settled against step", &settled, &inline);
    for workers in [1, 2, 4] {
        let pooled = run(
            open(custody(&world, SEED), &rules(cap)),
            Settle::Pooled(workers),
            jittered(SEED, until),
        );
        assert_same(
            &format!("pooled at {workers} workers against inline"),
            &pooled,
            &inline,
        );
        assert_eq!(
            pooled.after_pulse, settled.after_pulse,
            "the state between pulses at {workers} workers"
        );
    }
    let lattice = run(
        open(custody(&world, SEED), &rules(cap)),
        Settle::Settled,
        every_substep(until),
    );
    assert_same(
        "one substep a pulse against the jittered wakes",
        &lattice,
        &inline,
    );
    assert_ne!(
        lattice.pulses, inline.pulses,
        "the two cut the wall differently"
    );
    println!(
        "m4_jittered_seam: pulses={} substeps={} thinks={} pulses_with_thinks={} pulses_of_many={} lines={}",
        inline.pulses,
        inline.hashes.len(),
        inline.thinks,
        inline.pulses_with_thinks,
        inline.pulses_of_many,
        inline.lines.len()
    );
}

/// **Every message the task publishes until it ends**, on the paused clock
/// (`how_long` of virtual time at most).
async fn collect(rx: &mut mpsc::Receiver<ArenaOut>, how_long: Duration) -> Vec<ArenaOut> {
    let deadline = tokio::time::Instant::now() + how_long;
    let mut out = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return out;
        }
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(message)) => {
                let ended = matches!(message, ArenaOut::Ended { .. });
                out.push(message);
                if ended {
                    return out;
                }
            }
            Ok(None) | Err(_) => return out,
        }
    }
}

/// **One match through the real task** on `pool`, to its end: the tape its
/// chunks carried, the end and the digest it reported, and every snapshot's
/// `(at_ms, state_hash)`.
async fn through_the_task(
    custody: CustodyOut,
    match_cfg: &MatchConfig,
    pool: Arc<MindPool>,
    virtual_cap: Duration,
) -> (
    Vec<(i64, String, u64)>,
    Option<EndReason>,
    [u8; 32],
    Vec<(i64, u64)>,
) {
    let sources = sealed_sources(&custody.units);
    let (egress_tx, mut egress_rx) = mpsc::channel(4_096);
    let seed = custody.seed;
    let (_handle, _join, _callsigns) = spawn_arena(
        &tokio::runtime::Handle::current(),
        MatchId(seed),
        custody,
        &ArenaConfig::default(),
        match_cfg,
        &SensorsConfig::default(),
        &MindsConfig::default(),
        &sources,
        pool,
        egress_tx,
    )
    .expect("the mixed field checks out through the task");
    let messages = collect(&mut egress_rx, virtual_cap).await;
    let mut bytes: Vec<u8> = Vec::new();
    let mut snapshots: Vec<(i64, u64)> = Vec::new();
    let mut ended = None;
    let mut digest = [0_u8; 32];
    for message in &messages {
        match message {
            ArenaOut::Ledger { chunk, .. } => bytes.extend_from_slice(&chunk.bytes),
            ArenaOut::Snapshot { snapshot, .. } => {
                snapshots.push((snapshot.at_ms, snapshot.state_hash))
            }
            ArenaOut::Ended {
                end,
                digest: reported,
                ..
            } => {
                ended = Some(end.reason);
                digest = *reported;
            }
            _ => {}
        }
    }
    (lines_of(&bytes), ended, digest, snapshots)
}

/// **17b cut M4.1 — real tasks at one, two and four workers hash as the
/// library** (doc 17 §6 "Execution policy", "Snapshot and resume"; D-UM-22,
/// D-UM-23; the M4 row's gate, item (i)): the mixed field over three seeds
/// through [`spawn_arena`] on tokio's paused clock — the task's own wake
/// rule, its batches on a private [`MindPool`] of one, two and four workers
/// awaited at the next pulse's head and before every five-second snapshot —
/// held to the library driven on the same rule (`Arena::next_due_ms` after
/// each step, settled between pulses): the tape line for line, the end, the
/// ledger digest, and every snapshot's `state_hash` equal to the library's
/// after the pulse of that millisecond (the batch was awaited before the
/// encode, so the snapshot carries every answer).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m4_real_tasks_at_one_two_and_four_workers_hash_as_the_library() {
    let world = booted_world();
    let match_cfg = rules(60_000);
    for seed in [SEED, SEED + 1, SEED + 2] {
        let library = run(
            open(custody(&world, seed), &match_cfg),
            Settle::Settled,
            task_rule(),
        );
        assert_eq!(
            library.ended,
            Some(EndReason::Timeout),
            "seed {seed} runs to its cap"
        );
        assert!(library.thinks > 0);
        for workers in [1, 2, 4] {
            let (lines, ended, digest, snapshots) = through_the_task(
                custody(&world, seed),
                &match_cfg,
                MindPool::new(workers),
                Duration::from_secs(600),
            )
            .await;
            let name = format!("seed {seed} at {workers} workers");
            if let Some(at) = (0..lines.len().max(library.lines.len()))
                .find(|at| lines.get(*at) != library.lines.get(*at))
            {
                panic!(
                    "{name}: the task's tape parts from the library's at line {at}:\n  task:    {:?}\n  library: {:?}",
                    lines.get(at),
                    library.lines.get(at)
                );
            }
            assert_eq!(ended, library.ended, "{name}: the end");
            assert_eq!(digest, library.ledger_digest, "{name}: the ledger digest");
            assert!(
                snapshots.len() >= 10,
                "{name}: a snapshot every five seconds: {}",
                snapshots.len()
            );
            for (at_ms, hash) in &snapshots {
                assert_eq!(
                    library.after_pulse.get(at_ms),
                    Some(hash),
                    "{name}: the snapshot at {at_ms} ms carries the state the library has after that pulse"
                );
            }
            println!(
                "m4_task_seam: seed={seed} workers={workers} lines={} snapshots={} end={:?}",
                lines.len(),
                snapshots.len(),
                ended
            );
        }
    }
}

/// **The twin's pooled lane through real tasks at one, two and four
/// workers** (the M4 row's gate, item (i): *real tasks at workers 1, 2 and 4
/// equal to the library driven on the task's own wake rule over … the twin's
/// thirty (run five times)*): the pooled lane's composition — two stock
/// tanks a side, the native stock mind (T1) on both, the fifteen seeds from
/// [`SEED`] — to the 900 s cap, each match through the task at each worker
/// count and held to the library as the fixture above holds it. `#[ignore]`d:
/// the gate run is
/// `cargo test --release -p tacsim --test shard_2 minds_pool::m4_real_tasks_over_the_twin_s_thirty_hash_as_the_library -- --ignored --exact --nocapture`,
/// five times.
///
/// **17b cut M4.5a** (the twin's T0 column retired): the reflex machine (T0)
/// that faced T1 here is gone with the column, so both sides muster
/// [`Muster::mind`] — T1 against T1, the twin's own pooled lane since the
/// cut — and the sides are no longer swapped: with one custody on both
/// sides a swapped match is the same match, so the lane plays the fifteen
/// seeds once each (the "thirty" of the name is the M4.1 lane's, kept so
/// the gated-test inventory and `ci.yml` name the same test).
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "17b cut M4.1's gate lane over the twin's fifteen pooled matches: run by hand in release, five times"]
async fn m4_real_tasks_over_the_twin_s_thirty_hash_as_the_library() {
    let world = booted_world();
    let match_cfg = rules(900_000);
    let t1 = |team| [Muster::mind(team, 1), Muster::mind(team, 2)];
    let started = std::time::Instant::now();
    let mut matches = 0_u32;
    for k in 0..15_u64 {
        let seed = SEED + k;
        let field = |world: &mush_testkit::TestWorld| {
            shipped_custody_seeded(world, &[&t1(Team::Red), &t1(Team::Blue)], seed)
        };
        let library = run(
            open(field(&world), &match_cfg),
            Settle::Settled,
            task_rule(),
        );
        for workers in [1, 2, 4] {
            let (lines, ended, digest, snapshots) = through_the_task(
                field(&world),
                &match_cfg,
                MindPool::new(workers),
                Duration::from_secs(3_600),
            )
            .await;
            let name = format!("seed {seed} at {workers} workers");
            assert_eq!(lines, library.lines, "{name}: the tape");
            assert_eq!(ended, library.ended, "{name}: the end");
            assert_eq!(digest, library.ledger_digest, "{name}: the ledger digest");
            for (at_ms, hash) in &snapshots {
                assert_eq!(
                    library.after_pulse.get(at_ms),
                    Some(hash),
                    "{name}: the snapshot at {at_ms}"
                );
            }
            matches += 1;
        }
    }
    println!(
        "m4_task_seam_fifteen: matches={matches} wall_s={}",
        started.elapsed().as_secs()
    );
}

/// **17b cut M4.1 — an unanswered think refuses the snapshot by name** (doc
/// 17 §6 "Snapshot and resume"; D-UM-22, D-UM-23; the M4 row's finding
/// (2)): the mixed field stepped with `Arena::step` alone until a pulse
/// hands thinks out — `Snapshot::encode` answers `SnapshotError::Awaiting`
/// naming the hulls, in id order; the answers delivered, it encodes.
#[test]
fn m4_an_unanswered_think_refuses_the_snapshot_by_name() {
    let world = booted_world();
    let mut arena = open(custody(&world, SEED), &rules(300_000));
    let mut wall = 0;
    let outstanding = loop {
        let out = arena.step(wall, &[], &[]);
        assert!(out.ended.is_none());
        if !out.thinks.is_empty() {
            break out.thinks;
        }
        wall += arena.substep_len_ms();
        assert!(wall < 5_000, "a think within the first grid");
    };
    let units: Vec<ObjId> = arena.awaiting.iter().copied().collect();
    let mut collected: Vec<ObjId> = outstanding.iter().map(|think| think.unit).collect();
    collected.sort_unstable();
    assert_eq!(units, collected, "the set awaiting is the set collected");
    let refused = Snapshot::encode(&arena);
    assert_eq!(
        refused.as_ref().map(|_| ()).map_err(Clone::clone),
        Err(SnapshotError::Awaiting {
            units: units.clone()
        })
    );
    let named = units
        .iter()
        .map(|unit| format!("#{}", unit.0))
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(
        refused.map(|_| ()).map_err(|error| error.to_string()),
        Err(format!(
            "a think awaits its answer on {named}: the batch was not delivered before the encode"
        ))
    );
    let settled = arena.deliver_answers(run_batch_inline(&outstanding));
    assert_eq!(
        settled.written,
        u32::try_from(outstanding.len()).unwrap_or(0)
    );
    assert!(arena.awaiting.is_empty());
    let snapshot = Snapshot::encode(&arena).expect("every answer written: it encodes");
    assert_eq!(snapshot.state_hash, state_hash(&arena));
}

/// **A runner that faults every think `ops`**, recording each job's
/// `(unit, think_seq, t_ms)`.
#[derive(Debug, Default)]
struct Faulting {
    seen: Mutex<Vec<(ObjId, u64, i64)>>,
}

impl MindRunner for Faulting {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push((job.unit, job.think_seq, job.t_ms));
        }
        ThinkResult {
            diagnostics: None,
            rule_sample: None,
            intents: Vec::new(),
            memory: job.memory.to_vec(),
            fault: Some(ThinkFault {
                cause: SmolStr::new_static("ops"),
            }),
            trace: Vec::new(),
        }
    }
}

/// **17b cut M4.1 — a fault's lines carry the think's own substep's end
/// hash, at three cadences** (the M4 row's finding (1); doc 17 §8, D-UM-31,
/// D-UM-32): red's native mind on a runner that faults every think `ops`,
/// blue a pilot's idle hull, at 5,000, 1,000 and 500 ms (settled between pulses) —
/// one tape, one substep hash sequence; every `mind_fault` line at the
/// think's own `t_ms` carries the hash of the substep that millisecond fell
/// in (the substep ending at the first lattice point at or after it), and
/// the `mind_dropped { strikes }` line beside the third carries the same.
/// (The lines are written at the next substep's head, after whatever the
/// think's substep journalled at later milliseconds, so the tape is not
/// `t_ms`-monotonic across them — declared in doc 17 §24, and not asserted
/// here: a quiet substep journals nothing after the think.)
///
/// **17b cut M4.5a**: blue was the reflex machine; it is now a pilot's
/// hull nobody drives ([`Muster::piloted`]) — NOT a [`Muster::mind`]: the
/// faulting runner is installed as the arena-wide fallback
/// (`Arena::mind_runner`), which every Native-sealed mind on the field thinks
/// on, so a blue mind would fault too and the tape would carry two drops
/// against the one this pins. Red's faults, drop and hashes are the claims,
/// unchanged.
#[test]
fn m4_a_fault_s_lines_carry_the_think_s_own_substep_hash_at_three_cadences() {
    let world = booted_world();
    let field = |world: &mush_testkit::TestWorld| {
        shipped_custody_seeded(
            world,
            &[
                &[Muster::mind(Team::Red, 1)],
                &[Muster::piloted(Team::Blue, 1)],
            ],
            SEED,
        )
    };
    let mut runs: Vec<(i64, Run, SubstepEnds)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = open(field(&world), &rules(300_000));
        let faulting = Arc::new(Faulting::default());
        arena.mind_runner = faulting.clone();
        // The substep hashes with the millisecond each substep ended at.
        let mut ends: Vec<(i64, u64)> = Vec::new();
        let mut pulses_ends: Vec<(i64, u64)> = Vec::new();
        let run = run(
            arena,
            Settle::Settled,
            |arena: &Arena, _: &StepOutput, wall: i64| {
                // Read after each step: the step's substeps, each one lattice
                // step after the last, ending at `now_ms`.
                let step_ms = arena.substep_len_ms();
                let count = i64::try_from(arena.substep_hashes.len()).unwrap_or(0);
                for (at, hash) in arena.substep_hashes.iter().enumerate() {
                    let n = i64::try_from(at).unwrap_or(0);
                    pulses_ends.push((arena.now_ms - (count - 1 - n) * step_ms, *hash));
                }
                (arena.now_ms < 10_000).then_some(wall + cadence)
            },
        );
        ends.append(&mut pulses_ends);
        assert!(
            faulting.seen.lock().map(|seen| seen.len()).unwrap_or(0) >= 4,
            "at least four faulting thinks in ten seconds"
        );
        runs.push((cadence, run, ends));
    }
    let (_, first, first_ends) = runs.first().cloned().expect("three runs");
    for (cadence, run, _) in runs.iter().skip(1) {
        assert_same(&format!("{cadence} ms against 5,000 ms"), run, &first);
    }
    let faults: Vec<&(i64, String, u64)> = first
        .lines
        .iter()
        .filter(|line| line.1.contains("\"mind_fault\"") || line.1.contains("\"mind_dropped\""))
        .collect();
    assert!(
        faults.len() >= 5,
        "four faults and a drop: {}",
        faults.len()
    );
    let step_ms = 250_i64;
    for (t_ms, kind, hash) in &faults {
        let end = t_ms.div_euclid(step_ms) * step_ms
            + if t_ms.rem_euclid(step_ms) == 0 {
                0
            } else {
                step_ms
            };
        let recorded: Vec<u64> = first_ends
            .iter()
            .filter(|(at, _)| *at == end)
            .map(|(_, hash)| *hash)
            .collect();
        assert_eq!(recorded.len(), 1, "one substep ends at {end} ms");
        assert_eq!(
            Some(hash),
            recorded.first(),
            "{kind} at {t_ms} ms carries the hash of the substep ending at {end} ms"
        );
    }
    let dropped: Vec<&(i64, String, u64)> = faults
        .iter()
        .copied()
        .filter(|line| line.1.contains("\"mind_dropped\""))
        .collect();
    assert_eq!(dropped.len(), 1, "one drop");
    let third_fault = faults
        .iter()
        .filter(|line| line.1.contains("\"mind_fault\""))
        .nth(2)
        .expect("a third fault");
    assert_eq!(
        dropped.first().map(|line| line.2),
        Some(third_fault.2),
        "the drop rides the third fault's hash"
    );
}
