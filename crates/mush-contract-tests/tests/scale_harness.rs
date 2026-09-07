//! **The scale harness** (H3; `docs/design/11-h-implementation-plan.md` §3.3).
//!
//! Four measurements the tree could not produce on demand before H3: the cold
//! economy-journal replay that 08g §12.1 item 5 keys its checkpoint decision
//! on, the world boot at tens of thousands of objects, the authoritative lane
//! under a sustained bundle soak (PLAN.md's never-built write-through soak),
//! and the shape of a tick-sized bundle against the kernel's three caps (10b
//! §9 q6).
//!
//! A fifth, from the economy checkpoint review (doc 12 §2): the same cold
//! replay **phase by phase** over a journal synthesized in milliseconds rather
//! than minted through the engine, at several scales.
//!
//! Every measuring test is `#[ignore]`: the CI `scale` job runs them on
//! `workflow_dispatch` and a weekly schedule with a live Redis. They
//! **report**, and assert a threshold only under `SCALE_ASSERT=1` (D-H-8), so a
//! slow runner cannot fail a scheduled run merely by being slow. Sizes come from
//! the environment — `SCALE_JOURNAL_ENTRIES`, `SCALE_ACCOUNTS`, `SCALE_OBJECTS`,
//! `SCALE_BUNDLES`, `SCALE_TICK_WRITES`, `SCALE_TICK_POST_BYTES`,
//! `SCALE_PROFILE_SCALES`, `SCALE_TX_TAIL_MAX`, `SCALE_CHECKPOINT_EVERY` — with the defaults
//! §3.3 and doc 12 name. The
//! un-ignored `h3_*` tests at the bottom run each seeder at a tiny size under
//! `cargo test --workspace`, so the ignored bodies cannot rot unnoticed.
//!
//! Redis twins run in logical dbs 1 (world boot) and 2 (soak); dbs 3–15 belong
//! to the other gated suites (see `README.md`).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use mush_config::metrics::{BUNDLE_BUCKETS_BYTES, LAG_BUCKETS_MS, Metrics, MetricsSnapshot};
use mush_core::ledger::DEFAULT_CHUNK_TARGET_BYTES;
use mush_core::{
    Audience, AuthoritativeBundle, AuthoritativeBundleDraft, AuthoritativeBundleError,
    AuthoritativeKey, AuthoritativeSemanticEvent, CanonicalHash, CanonicalId, CommitToken,
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    GOD, IdempotencyKey, IdempotencyScope, MAX_AUTHORITATIVE_BUNDLE_BYTES,
    MAX_AUTHORITATIVE_REPLAY_WRITES, MAX_DURABLE_OUTBOX_PAYLOAD_BYTES, ModuleBootData, ModuleOp,
    ModuleWrite, Obj, ObjId, ObjKind, OwnerId, PreparedCausalRef, PrincipalId, ReplayWrite,
    RequestUid, RulesValue, ScopedIdempotencyKey, canonical_json_bytes,
};
use mush_core::{Ledger, LedgerChunk, LedgerDoc, LedgerEntry, typed_entries};
use mush_economy::{
    CHECKPOINT_REVIEW_ENTRIES, CHECKPOINT_REVIEW_SECS, EconomicTransaction, EconomyStore,
    ReplayStats,
};
use mush_proto::OutputEvent;
use mush_server::persist::{
    AuthoritativeCommitFailure, PersistenceIntegrityLatch, StoreJob, load_or_seed_world,
    spawn_store_writer,
};
use mush_server::quarantine::QuarantineSpool;
use mush_server::world_task::writer_signal_channel;
use mush_store::{AuthoritativeReceipt, InMemoryStore, ObjectStore, WriteBatch};
use mush_testkit::{TestWorld, text_of};
use serde_json::{Value, json};
use smol_str::SmolStr;
use tokio::sync::{mpsc, oneshot};

// ---- knobs -----------------------------------------------------------------

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// D-H-8: the ignored tests report; they assert only when asked.
fn scale_assert() -> bool {
    std::env::var("SCALE_ASSERT").is_ok_and(|v| v.trim() == "1")
}

fn redis_url_with_db(base: &str, db: u32) -> String {
    let (scheme, rest) = base.split_once("://").unwrap_or(("redis", base));
    let authority = rest.split(['/', '?']).next().unwrap_or(rest);
    format!("{scheme}://{authority}/{db}")
}

/// A wiped `RedisStore` in logical db `db`, or `None` (with a skip line) when
/// there is no live Redis.
async fn live_redis(test: &str, db: u32) -> Option<mush_store::RedisStore> {
    let Some(url) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!("skipping {test}: REDIS_URL not set (no live Redis on this machine)");
        return None;
    };
    let store = mush_store::RedisStore::connect(&redis_url_with_db(&url, db))
        .await
        .expect("connect to REDIS_URL");
    store.wipe_namespaces().await.expect("wipe test namespaces");
    Some(store)
}

/// The bucket upper bound the `q`-th percentile falls in (the metrics
/// registry's own p99 rule, generalized), `max` in the open-topped bucket.
fn bucket_quantile(bounds: &[u64], counts: &[u64], count: u64, max: u64, q: u64) -> u64 {
    if count == 0 {
        return 0;
    }
    let target = count.saturating_mul(q).div_ceil(100).max(1);
    for (i, &bound) in bounds.iter().enumerate() {
        if counts[i] >= target {
            return bound;
        }
    }
    max
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

// ---- 1. the cold journal replay ----------------------------------------------

const ECON_NS: &str = "economy";

fn econ_world() -> TestWorld {
    // Economy authorizes through org; load both (the e2b rig). `SCALE_TX_TAIL_MAX`
    // (doc 12 J2) caps the RAM tail; 0 is the crate default, and a value past
    // the journal's length is the unbounded retention J2 replaced — the two
    // ends of the RAM measurement, on one binary.
    TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(
            mush_economy::EconomyModule::new().with_config(mush_economy::EconomyConfig {
                tx_tail_max: env_usize("SCALE_TX_TAIL_MAX", 0),
                // `SCALE_CHECKPOINT_EVERY` (doc 12 J3): the checkpoint cadence; 0
                // (the default here) writes none, so the replay numbers stay the
                // full-replay numbers unless the checkpointed boot is asked for.
                checkpoint_every_entries: u64::try_from(env_usize("SCALE_CHECKPOINT_EVERY", 0))
                    .unwrap_or(0),
                // `SCALE_IDEM_WINDOW_MAX` (doc 13 W2): the RAM window's count bound;
                // 0 is the crate default (1 000 000, no eviction at these scales).
                idempotency_window_max: env_usize("SCALE_IDEM_WINDOW_MAX", 0),
                ..mush_economy::EconomyConfig::default()
            }),
        ),
    ])
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}

/// Fold the economy namespace's pending module ops into `kv` the way a cold
/// store would hold them (last write per key wins; deletes remove).
fn fold_economy_ops(kv: &mut HashMap<SmolStr, Value>, tw: &mut TestWorld) {
    for op in tw.take_module_ops() {
        if op.ns != ECON_NS {
            continue;
        }
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.insert(key, value);
            }
            ModuleWrite::Del { key } => {
                kv.remove(&key);
            }
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
}

/// Seed `entries` minted transactions round-robin across `accounts` player
/// accounts through the real command surface, returning the economy
/// namespace as a cold store would hold it.
fn seed_journal(entries: usize, accounts: usize) -> Vec<(SmolStr, Value)> {
    let accounts = accounts.max(1);
    let mut tw = econ_world();
    let mut kv = HashMap::new();
    let ev = tw.run(GOD, "@instrument/register gold=2");
    assert!(!has_error(&ev), "instrument: {}", texts(&ev));
    let ev = tw.run(GOD, "@account/system faucet=gold");
    assert!(!has_error(&ev), "faucet: {}", texts(&ev));
    for n in 0..accounts {
        let p = tw.spawn_player(&format!("Scale{n}"));
        let ev = tw.run(GOD, &format!("@account/open #{}=gold", p.0));
        assert!(!has_error(&ev), "account {n}: {}", texts(&ev));
    }
    fold_economy_ops(&mut kv, &mut tw);
    for i in 0..entries {
        let to = 1 + (i % accounts);
        let ev = tw.run(GOD, &format!("@econ/mint 0={to} 1"));
        assert!(!has_error(&ev), "mint {i}: {}", texts(&ev));
        // Fold as we go so the pending op log stays small at 100 000 entries.
        if i % 1_000 == 999 {
            fold_economy_ops(&mut kv, &mut tw);
        }
    }
    fold_economy_ops(&mut kv, &mut tw);
    kv.into_iter().collect()
}

/// Boot a fresh economy from `kv` alone (ledger chunks may ride in it as
/// values — the engine moves them to text, as it does for every rig) and read
/// what the replay cost.
fn replay_journal(kv: Vec<(SmolStr, Value)>) -> ReplayStats {
    replay_split(kv, Vec::new())
}

/// [`replay_journal`] with the ledger chunks already in the text form the
/// production loader hands over (J1).
fn replay_split(kv: Vec<(SmolStr, Value)>, ledger: Vec<LedgerDoc>) -> ReplayStats {
    let tw = boot_split(kv, ledger);
    replay_stats(&tw)
}

/// Boot a fresh economy from split boot data and keep the world (the RAM
/// measurement holds it after the boot).
fn boot_split(kv: Vec<(SmolStr, Value)>, ledger: Vec<LedgerDoc>) -> TestWorld {
    let mut tw = econ_world();
    tw.modules_loaded(HashMap::from([(
        SmolStr::new(ECON_NS),
        ModuleBootData {
            kv,
            ledger,
            ..Default::default()
        },
    )]));
    tw
}

/// J4: boot the way the production loader would after reading the manifest
/// the sweep wrote into `kv` — the journal chunks below its floor never handed
/// over, the omission carried as the min-present marker. `None` when `kv`
/// holds no manifest.
fn boot_narrowed(kv: Vec<(SmolStr, Value)>, ledger: Vec<LedgerDoc>) -> Option<(TestWorld, u64)> {
    let (_, doc) = kv.iter().find(|(k, _)| k == mush_core::BOOT_MANIFEST_KEY)?;
    let manifest: mush_core::BootManifest = serde_json::from_value(doc.clone()).ok()?;
    let floor = manifest.floors().get("journal").copied()?;
    let ledger: Vec<LedgerDoc> = ledger
        .into_iter()
        .filter(|(key, _)| {
            mush_core::parse_chunk_key(key)
                .is_none_or(|(log, chunk)| log != "journal" || chunk >= floor)
        })
        .collect();
    let mut tw = econ_world();
    tw.modules_loaded(HashMap::from([(
        SmolStr::new(ECON_NS),
        ModuleBootData {
            kv,
            ledger,
            omitted: [(SmolStr::new("journal"), floor)].into_iter().collect(),
            ..Default::default()
        },
    )]));
    Some((tw, floor))
}

fn replay_stats(tw: &TestWorld) -> ReplayStats {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .boot_replay()
        .expect("the economy booted, so it recorded its replay")
}

/// The split boot data as a file (`SCALE_JOURNAL_OUT` / `SCALE_JOURNAL_FILE`):
/// what the RAM measurement of doc 12 J2 boots from in a process of its own,
/// so the seeder's own allocations are not in the number.
#[derive(serde::Serialize, serde::Deserialize)]
struct JournalFile {
    kv: Vec<(SmolStr, Value)>,
    ledger: Vec<LedgerDoc>,
}

fn write_journal_file(path: &str, kv: &[(SmolStr, Value)], ledger: &[LedgerDoc]) {
    let file = JournalFile {
        kv: kv.to_vec(),
        ledger: ledger.to_vec(),
    };
    let text = serde_json::to_string(&file).expect("the journal file encodes");
    std::fs::write(path, text).expect("the journal file is written");
}

fn read_journal_file(path: &str) -> JournalFile {
    let text = std::fs::read_to_string(path).expect("the journal file is read");
    serde_json::from_str(&text).expect("the journal file decodes")
}

/// The economy checkpoint review (`docs/design/12-economy-checkpoint-review.md`
/// §2): the journal `seed_journal` produces, **synthesized** — the real docs
/// (instrument, faucet, `accounts` player accounts, `meta`) as one real mint
/// left them, whose journal entry is the template every entry clones, packed into chunks
/// at the ledger's own 16 KiB seal — so a cold replay at `entries` costs
/// milliseconds to seed instead of minutes through the engine. Returns the
/// namespace as the production loader hands it over — ordinary docs as values,
/// ledger chunks as text (J1) — plus the chunk count and the journal's JSON
/// byte size.
fn synth_journal(entries: usize, accounts: usize) -> SyntheticJournal {
    let accounts = accounts.max(1);
    let mut tw = econ_world();
    let mut kv = HashMap::new();
    let ev = tw.run(GOD, "@instrument/register gold=2");
    assert!(!has_error(&ev), "instrument: {}", texts(&ev));
    let ev = tw.run(GOD, "@account/system faucet=gold");
    assert!(!has_error(&ev), "faucet: {}", texts(&ev));
    for n in 0..accounts {
        let p = tw.spawn_player(&format!("Scale{n}"));
        let ev = tw.run(GOD, &format!("@account/open #{}=gold", p.0));
        assert!(!has_error(&ev), "account {n}: {}", texts(&ev));
    }
    fold_economy_ops(&mut kv, &mut tw);
    let ev = tw.run(GOD, "@econ/mint 0=1 1");
    assert!(!has_error(&ev), "template mint: {}", texts(&ev));
    let mut after = HashMap::new();
    fold_economy_ops(&mut after, &mut tw);
    let chunk: LedgerChunk = serde_json::from_value(
        after
            .remove("log:journal:0")
            .expect("the template mint wrote chunk 0"),
    )
    .expect("chunk 0 parses");
    let template = chunk.entries[0].value.clone();
    // The documents as the template mint left them, so the faucet and account
    // 1 claim seq 0 with the balances the journal's seq 0 gives them. Captured
    // before the mint, both would carry `last_journal_seq == 0` as the
    // never-touched sentinel while the journal's entry 0 touches them, and the
    // boot reads that as drift at seq 0, not lag (doc 12 J3-F2): the
    // measurement then booted a frozen economy, whose sweep writes nothing.
    kv.extend(after);

    let mut chunks: Vec<LedgerChunk> = Vec::new();
    let mut open = LedgerChunk {
        chunk: 0,
        sealed: false,
        entries: Vec::new(),
    };
    let mut open_bytes = 0usize;
    let mut total_bytes = 0usize;
    for i in 0..entries {
        let seq = u64::try_from(i).expect("entry count fits u64");
        let mut tx = template.clone();
        tx["id"] = Value::from(seq + 1);
        tx["journal_seq"] = Value::from(seq);
        tx["idempotency"]["key"] = Value::from(format!("scale-{i}"));
        tx["posted_at"] = Value::from(1_700_000_000_u64 + seq);
        tx["lines"][1]["account"] =
            Value::from(u64::try_from(1 + i % accounts).expect("account id fits u64"));
        let entry = LedgerEntry { seq, value: tx };
        let bytes = serde_json::to_vec(&entry).expect("entry encodes").len();
        total_bytes += bytes;
        open_bytes += bytes;
        open.entries.push(entry);
        if open_bytes >= DEFAULT_CHUNK_TARGET_BYTES {
            open.sealed = true;
            let next = open.chunk + 1;
            chunks.push(std::mem::replace(
                &mut open,
                LedgerChunk {
                    chunk: next,
                    sealed: false,
                    entries: Vec::new(),
                },
            ));
            open_bytes = 0;
        }
    }
    if !open.entries.is_empty() {
        chunks.push(open);
    }
    let count = chunks.len();
    let ledger: Vec<LedgerDoc> = chunks
        .into_iter()
        .map(|c| {
            (
                SmolStr::new(format!("log:journal:{}", c.chunk)),
                serde_json::to_string(&c).expect("chunk encodes"),
            )
        })
        .collect();
    SyntheticJournal {
        kv: kv.into_iter().collect(),
        ledger,
        chunks: count,
        bytes: total_bytes,
    }
}

/// What [`synth_journal`] produces.
struct SyntheticJournal {
    kv: Vec<(SmolStr, Value)>,
    ledger: Vec<LedgerDoc>,
    chunks: usize,
    bytes: usize,
}

/// The cold replay's phases (doc 12 §1, as rebuilt by J1), each timed in
/// isolation on the same documents by calling exactly what `load_boot` calls,
/// in milliseconds.
struct ReplayPhases {
    /// `Ledger::inspect`: the one pass that decodes every chunk's facts from
    /// its text (the contiguity check, the seal rule, the tail).
    inspect: u64,
    /// `Ledger::restore_from_history` from that inspection.
    restore: u64,
    /// The typed pass: every chunk's entries decoded straight to
    /// `EconomicTransaction`, in the inspection's order.
    typed: u64,
}

fn replay_phases(ledger: &[LedgerDoc]) -> ReplayPhases {
    let mut journal = Ledger::new();
    let started = Instant::now();
    let history = journal.inspect(ledger);
    let inspect = ms(started.elapsed());
    assert!(
        history.is_clean(),
        "the synthetic journal is contiguous: {:?}",
        history.breaches()
    );

    let started = Instant::now();
    journal
        .restore_from_history(&history)
        .expect("the synthetic journal restores");
    let restore = ms(started.elapsed());

    let started = Instant::now();
    let mut txs: Vec<(u64, EconomicTransaction)> = Vec::with_capacity(history.entries());
    for (_, position) in history.chunk_docs(mush_economy::JOURNAL_LOG) {
        txs.extend(
            typed_entries::<EconomicTransaction>(&ledger[position].1).expect("entries type"),
        );
    }
    txs.sort_by_key(|(seq, _)| *seq);
    let typed = ms(started.elapsed());
    assert_eq!(txs.len(), history.entries());

    ReplayPhases {
        inspect,
        restore,
        typed,
    }
}

/// Doc 12 §2: where the cold replay's time goes, phase by phase, at each
/// scale in `SCALE_PROFILE_SCALES` (comma-separated entry counts; default
/// `25000,50000,100000`). `boot_total_ms` is the C17 timer; the three phases
/// are what J1 left of the serde work (doc 12 §7 J1 as built), and
/// `us_per_entry` and `entries_at_5s` are the numbers §4 derives the entry
/// line from.
#[test]
#[ignore = "scale harness: run by the CI `scale` job (workflow_dispatch / weekly)"]
fn scale_cold_journal_replay_phases() {
    let scales: Vec<usize> = std::env::var("SCALE_PROFILE_SCALES")
        .ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .filter(|v: &Vec<usize>| !v.is_empty())
        .unwrap_or_else(|| vec![25_000, 50_000, 100_000]);
    let accounts = env_usize("SCALE_ACCOUNTS", 100);
    println!(
        "scale_cold_journal_replay_phases: entries | chunks | json_kb | inspect_ms | \
         restore_ms | typed_ms | boot_total_ms | us_per_entry | entries_at_5s | \
         ckpt_boot_ms | ckpt_select_ms | ckpt_replayed | narrow_boot_ms | narrow_floor |          narrow_chunks"
    );
    let checkpoint_every = env_usize("SCALE_CHECKPOINT_EVERY", 0);
    for n in scales {
        let journal = synth_journal(n, accounts);
        if let Ok(path) = std::env::var("SCALE_JOURNAL_OUT") {
            write_journal_file(&path, &journal.kv, &journal.ledger);
            println!("wrote {path}: {n} entries, {} chunks", journal.chunks);
        }
        let phases = replay_phases(&journal.ledger);
        // Doc 12 J3: the checkpointed boot — boot once, let the sweep write a
        // checkpoint at the head, overlay its documents, and boot again.
        let (ckpt_boot_ms, ckpt_select_ms, ckpt_replayed, narrow) = if checkpoint_every > 0 {
            let mut tw = boot_split(journal.kv.clone(), journal.ledger.clone());
            let _ = tw.take_module_ops();
            tw.set_now(1_000);
            let _ = tw.fire_due(1_000_000);
            let mut kv: HashMap<SmolStr, Value> = journal.kv.iter().cloned().collect();
            fold_economy_ops(&mut kv, &mut tw);
            drop(tw);
            let kv: Vec<(SmolStr, Value)> = kv.into_iter().collect();
            assert!(
                kv.iter().any(|(k, _)| k.starts_with("checkpoint:")),
                "the sweep wrote a checkpoint"
            );
            let booted = boot_split(kv.clone(), journal.ledger.clone());
            let stats = replay_stats(&booted);
            assert!(
                stats.from_checkpoint.is_some(),
                "the second boot resumed from the checkpoint"
            );
            assert_eq!(stats.entries, n, "the journal is still counted whole");
            // J3-F4: the RAM tail is full after a checkpointed boot, across
            // however many chunks the tail spans.
            let tail_max = match env_usize("SCALE_TX_TAIL_MAX", 0) {
                0 => mush_economy::DEFAULT_TX_TAIL_MAX,
                m => m,
            };
            let tail = booted
                .world()
                .module_state::<EconomyStore>()
                .expect("economy store")
                .tail_len();
            assert_eq!(
                tail,
                n.min(tail_max),
                "the tail is kept across the checkpoint"
            );
            drop(booted);
            // Doc 12 J4: the same boot with the chunks below the manifest's
            // floor never handed over — what the narrowed fetch saves the boot
            // (the fetch's own saving is the store's, measured on Redis).
            let narrow = boot_narrowed(kv, journal.ledger.clone()).map(|(tw, floor)| {
                let s = replay_stats(&tw);
                assert_eq!(s.from_checkpoint, stats.from_checkpoint);
                assert_eq!(s.entries, n);
                (s.elapsed_ms, floor)
            });
            (
                stats.elapsed_ms,
                stats.checkpoint_ms,
                stats.replayed,
                narrow,
            )
        } else {
            (0, 0, 0, None)
        };
        let (narrow_boot_ms, narrow_floor) = narrow.unwrap_or((0, 0));
        let narrow_chunks = if narrow.is_some() {
            journal
                .chunks
                .saturating_sub(usize::try_from(narrow_floor).unwrap_or(usize::MAX))
        } else {
            0
        };
        let stats = replay_split(journal.kv, journal.ledger);
        assert_eq!(
            stats.entries, n,
            "every synthetic entry is one journal transaction"
        );
        let us_per_entry =
            stats.elapsed_ms.saturating_mul(1_000) / u64::try_from(n.max(1)).unwrap();
        let entries_at_line = CHECKPOINT_REVIEW_SECS
            .saturating_mul(1_000_000)
            .checked_div(us_per_entry)
            .unwrap_or(0);
        println!(
            "{n} | {} | {} | {} | {} | {} | {} | {us_per_entry} | {entries_at_line} | {ckpt_boot_ms} | {ckpt_select_ms} | {ckpt_replayed} | {narrow_boot_ms} | {narrow_floor} | {narrow_chunks}",
            journal.chunks,
            journal.bytes / 1024,
            phases.inspect,
            phases.restore,
            phases.typed,
            stats.elapsed_ms
        );
    }
}

/// Doc 12 J2 (lens M): boot the economy from `SCALE_JOURNAL_FILE` — split
/// boot data a previous `scale_cold_journal_replay_phases` run wrote with
/// `SCALE_JOURNAL_OUT` — in a process that did nothing else, then hold the
/// booted world for `SCALE_HOLD_MS` so an outside sampler can read the peak
/// and the at-rest working set. `SCALE_TX_TAIL_MAX` sets the RAM tail (0 is
/// the default; a value past the journal's length is unbounded retention).
#[test]
#[ignore = "scale harness: run by the CI `scale` job (workflow_dispatch / weekly)"]
fn scale_cold_journal_replay_from_file() {
    let Ok(path) = std::env::var("SCALE_JOURNAL_FILE") else {
        println!("scale_cold_journal_replay_from_file: SCALE_JOURNAL_FILE unset; nothing to boot");
        return;
    };
    let file = read_journal_file(&path);
    let entries = file.ledger.len();
    let tw = boot_split(file.kv, file.ledger);
    let stats = replay_stats(&tw);
    let store = tw
        .world()
        .module_state::<EconomyStore>()
        .expect("economy store");
    println!(
        "from_file: chunks={entries} entries={} replay_ms={} tail={} tail_max={} journal_len={} window={} window_max={}",
        stats.entries,
        stats.elapsed_ms,
        store.tail_len(),
        store.tx_tail_max(),
        store.journal_len(),
        store.window_len(),
        store.idempotency_window_max()
    );
    let hold_ms = env_usize("SCALE_HOLD_MS", 0);
    if hold_ms > 0 {
        std::thread::sleep(Duration::from_millis(u64::try_from(hold_ms).unwrap_or(0)));
    }
    // Doc 13 W2 (lens M): the world is leaked, not dropped, so the sampler's last
    // reading is the held store and not its teardown — the process exit reclaims
    // it all at once.
    std::mem::forget(tw);
}

#[test]
#[ignore = "scale harness: run by the CI `scale` job (workflow_dispatch / weekly)"]
fn scale_cold_journal_replay_reports_time() {
    let entries = env_usize("SCALE_JOURNAL_ENTRIES", 100_000);
    let accounts = env_usize("SCALE_ACCOUNTS", 100);
    let seeded_at = Instant::now();
    let kv = seed_journal(entries, accounts);
    let seed_ms = ms(seeded_at.elapsed());
    let stats = replay_journal(kv);
    println!(
        "scale_cold_journal_replay: entries={} accounts={accounts} seed_ms={seed_ms} \
         replay_ms={} review_at={CHECKPOINT_REVIEW_SECS}s/{CHECKPOINT_REVIEW_ENTRIES} entries",
        stats.entries, stats.elapsed_ms
    );
    assert_eq!(
        stats.entries, entries,
        "every mint is one journal transaction"
    );
    if scale_assert() {
        assert!(
            stats.elapsed_ms <= CHECKPOINT_REVIEW_SECS * 1_000
                && stats.entries <= CHECKPOINT_REVIEW_ENTRIES,
            "the cold replay crossed the checkpoint-review line (08g §12.1 item 5): \
             {} entries in {} ms",
            stats.entries,
            stats.elapsed_ms
        );
    }
}

// ---- 2. the world boot ---------------------------------------------------------

/// `objects` rooms, exits and players (round-robin) at ids `2..2+objects`,
/// as one ordinary batch after the minimal seed.
fn world_batch(objects: usize) -> WriteBatch {
    let first = 2u32;
    let mut upserts = Vec::with_capacity(objects);
    let mut last_room = ObjId(0);
    for n in 0..objects {
        let id = ObjId(first + u32::try_from(n).expect("object count fits u32"));
        let obj = match n % 3 {
            0 => {
                last_room = id;
                Obj::new(id, ObjKind::Room, format!("Room {n}"), GOD, 0)
            }
            1 => {
                let mut o = Obj::new(id, ObjKind::Exit, format!("out{n}"), GOD, 0);
                o.location = Some(last_room);
                o.destination = Some(ObjId(0));
                o
            }
            _ => {
                let mut o = Obj::new(id, ObjKind::Player, format!("Scale{n}"), id, 0);
                o.location = Some(last_room);
                o.home = Some(last_room);
                o
            }
        };
        upserts.push(obj);
    }
    WriteBatch {
        serial: 1,
        upserts,
        next_id: Some(first + u32::try_from(objects).expect("object count fits u32")),
        durable: true,
        ..WriteBatch::default()
    }
}

/// Seed, write `objects` objects, then time a boot from the store alone.
/// Returns the booted object count (seed included) and the boot time.
async fn boot_world(store: &dyn ObjectStore, objects: usize) -> (usize, Duration) {
    load_or_seed_world(store)
        .await
        .expect("seed the minimal world");
    store
        .write(world_batch(objects))
        .await
        .expect("write the scale batch");
    let started = Instant::now();
    let world = load_or_seed_world(store)
        .await
        .expect("boot the seeded world");
    (world.iter().count(), started.elapsed())
}

fn report_world_boot(label: &str, objects: usize, booted: usize, took: Duration) {
    println!(
        "scale_world_boot[{label}]: objects={objects} booted={booted} boot_ms={}",
        ms(took)
    );
    assert_eq!(
        booted,
        objects + 2,
        "every seeded object plus Limbo and God"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "scale harness: run by the CI `scale` job (workflow_dispatch / weekly)"]
async fn scale_world_boot_reports_time() {
    let objects = env_usize("SCALE_OBJECTS", 50_000);
    let store = InMemoryStore::new();
    let (booted, took) = boot_world(&store, objects).await;
    report_world_boot("memory", objects, booted, took);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "scale harness: run by the CI `scale` job (workflow_dispatch / weekly)"]
async fn scale_world_boot_redis_gated() {
    let Some(store) = live_redis("scale_world_boot_redis_gated", 1).await else {
        return;
    };
    let objects = env_usize("SCALE_OBJECTS", 50_000);
    let (booted, took) = boot_world(&store, objects).await;
    report_world_boot("redis", objects, booted, took);
}

// ---- 3. the authoritative soak ---------------------------------------------------

/// The definitions every soak bundle cites (one registration, cloned per
/// bundle, exactly as `authoritative_boot.rs::recovery_bundle` builds them).
struct SoakDefs {
    owner: OwnerId,
    action: DefinitionRef,
    outcome: DefinitionRef,
}

impl SoakDefs {
    fn new() -> Self {
        let owner = OwnerId::new("scale_soak").unwrap();
        let mut definitions = DefinitionRegistryBuilder::new();
        let action = definitions
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("soak_step").unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let outcome = definitions
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("outcome").unwrap(),
                    DefinitionId::new("soak_stepped").unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        Self {
            owner,
            action,
            outcome,
        }
    }

    /// Sixteen bytes that differ per bundle and per role.
    fn bytes16(i: u64, role: u8) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&i.to_le_bytes());
        b[15] = role;
        b
    }

    fn bytes32(i: u64, role: u8) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&i.to_le_bytes());
        b[31] = role;
        b
    }

    /// A draft carrying `writes`; every identity distinct per `i`.
    fn draft(&self, i: u64, writes: Vec<ReplayWrite>) -> AuthoritativeBundleDraft {
        let request = RequestUid::from_bytes(Self::bytes16(i, 2));
        let principal = PrincipalId::Object { id: GOD };
        AuthoritativeBundleDraft {
            token: CommitToken::from_bytes(Self::bytes16(i, 1)),
            request,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: principal.clone(),
                    owner: self.owner.clone(),
                    namespace: CanonicalId::new("soak_step").unwrap(),
                },
                0,
                IdempotencyKey::Request(request),
            ),
            ingress_hash: CanonicalHash::from_bytes(Self::bytes32(i, 3)),
            request_hash: CanonicalHash::from_bytes(Self::bytes32(i, 4)),
            plan_hash: CanonicalHash::from_bytes(Self::bytes32(i, 5)),
            action: self.action.clone(),
            provider: self.owner.clone(),
            cause: PreparedCausalRef::new(request, None, 0).unwrap(),
            definitions: vec![self.action.clone(), self.outcome.clone()],
            writes,
            outcome: RulesValue::new(self.outcome.clone(), json!({ "step": i })).unwrap(),
            resolution: None,
            semantic_events: vec![
                AuthoritativeSemanticEvent::root(
                    PreparedCausalRef::new(request, None, 0).unwrap(),
                    &self.action,
                    RulesValue::new(self.outcome.clone(), json!({ "step": i })).unwrap(),
                    vec![principal.clone()],
                )
                .unwrap(),
            ],
            eligible_witnesses: vec![principal],
            durable_outbox: vec![],
        }
    }

    /// Soak bundle `i`: one new thing at a fresh id plus the allocator bump.
    fn bundle(&self, i: u64) -> AuthoritativeBundle {
        let id = 10_000 + u32::try_from(i).expect("bundle count fits u32");
        AuthoritativeBundle::new(self.draft(
            i,
            vec![
                ReplayWrite::ObjectSet {
                    object: Obj::new(ObjId(id), ObjKind::Thing, format!("soak {i}"), GOD, 0),
                },
                ReplayWrite::ObjectAllocatorSet { next_id: id + 1 },
            ],
        ))
        .unwrap()
    }
}

/// What one soak produced.
struct SoakReport {
    finalized: u64,
    elapsed: Duration,
    metrics: MetricsSnapshot,
}

/// One receipt the consumer waits on.
enum Pending {
    Bundle(oneshot::Receiver<Result<AuthoritativeReceipt, AuthoritativeCommitFailure>>),
    Legacy(oneshot::Receiver<Result<u64, String>>),
}

/// Drive `bundles` real bundles through the real store writer, one ordinary
/// durable batch after every fourth, and read the lane's gauges back.
async fn soak(store: Arc<dyn ObjectStore>, bundles: usize) -> SoakReport {
    load_or_seed_world(&*store)
        .await
        .expect("seed the minimal world");
    let metrics = Arc::new(Metrics::new());
    let high_water = Arc::new(AtomicU64::new(0));
    let (signal_tx, _signal_rx) = writer_signal_channel();
    let (writer, join) = spawn_store_writer(
        Arc::clone(&store),
        signal_tx,
        Arc::clone(&metrics),
        high_water,
        QuarantineSpool::in_memory(),
        PersistenceIntegrityLatch::new(),
    );

    let started = Instant::now();
    let (pending_tx, mut pending_rx) = mpsc::unbounded_channel::<Pending>();
    // The producer blocks on lane capacity exactly as the world task would;
    // the consumer below drains receipts in order.
    let producer = tokio::spawn(async move {
        let defs = SoakDefs::new();
        let mut legacy_serial = 1u64;
        for i in 0..u64::try_from(bundles).expect("bundle count fits u64") {
            let (ownership, _owned) = oneshot::channel();
            let (notify, receipt) = oneshot::channel();
            writer
                .send(StoreJob::Authoritative {
                    bundle: Box::new(defs.bundle(i)),
                    ownership,
                    notify,
                })
                .await
                .expect("the writer outlives the producer");
            pending_tx
                .send(Pending::Bundle(receipt))
                .expect("the consumer outlives the producer");
            if i % 4 == 3 {
                // An ordinary durable batch between bundles: a module cell, so
                // the legacy lane never touches the object allocator the
                // bundles advance (reservation admission checks it).
                let (tx, rx) = oneshot::channel();
                writer
                    .send(StoreJob::Write {
                        batch: WriteBatch {
                            serial: legacy_serial,
                            module_ops: vec![ModuleOp {
                                ns: SmolStr::new("scale"),
                                op: ModuleWrite::JsonSet {
                                    key: SmolStr::new(format!("filler:{legacy_serial}")),
                                    value: json!({ "serial": legacy_serial }),
                                },
                            }],
                            durable: true,
                            ..WriteBatch::default()
                        },
                        notify: Some(tx),
                    })
                    .await
                    .expect("the writer outlives the producer");
                pending_tx
                    .send(Pending::Legacy(rx))
                    .expect("the consumer outlives the producer");
                legacy_serial += 1;
            }
        }
        // Dropping the last sender lets the writer drain and exit.
        drop(writer);
    });

    let mut finalized = 0u64;
    while let Some(pending) = pending_rx.recv().await {
        match pending {
            Pending::Bundle(receipt) => {
                receipt
                    .await
                    .expect("the writer answers every bundle")
                    .expect("every soak bundle finalizes");
                finalized += 1;
                // The lane observation and the slot release precede the
                // receipt: a completed commit never leaves the slot held.
                let snap = metrics.snapshot();
                assert!(
                    snap.auth_hold_count >= finalized,
                    "receipt {finalized} arrived before its lane observation"
                );
            }
            Pending::Legacy(receipt) => {
                receipt
                    .await
                    .expect("the writer answers every batch")
                    .expect("every filler batch commits");
            }
        }
    }
    producer.await.expect("producer ran to completion");
    join.await.expect("writer exited cleanly");
    SoakReport {
        finalized,
        elapsed: started.elapsed(),
        metrics: metrics.snapshot(),
    }
}

fn report_soak(label: &str, bundles: usize, report: &SoakReport) {
    let m = &report.metrics;
    let hold_p50 = bucket_quantile(
        &LAG_BUCKETS_MS,
        &m.auth_hold_buckets,
        m.auth_hold_count,
        m.auth_hold_max_ms,
        50,
    );
    let bytes_p50 = bucket_quantile(
        &BUNDLE_BUCKETS_BYTES,
        &m.auth_bytes_buckets,
        m.auth_bytes_count,
        m.auth_bytes_last,
        50,
    );
    let elapsed_ms = ms(report.elapsed);
    println!(
        "scale_authoritative_soak[{label}]: bundles={bundles} finalized={} elapsed_ms={elapsed_ms} \
         per_bundle_us={} hold_p50_le_ms={hold_p50} hold_p99_le_ms={} hold_max_ms={} \
         hold_mean_ms={} bytes_p50_le={bytes_p50} bytes_last={} writes_last={} \
         commits=F{}/R{}/I{}/Q{}",
        report.finalized,
        report
            .elapsed
            .as_micros()
            .checked_div(u128::try_from(bundles.max(1)).unwrap())
            .unwrap_or(0),
        m.authoritative_hold_p99_ms(),
        m.auth_hold_max_ms,
        m.authoritative_hold_mean_ms(),
        m.auth_bytes_last,
        m.auth_writes_last,
        m.auth_commits_finalized_total,
        m.auth_commits_refused_total,
        m.auth_commits_integrity_total,
        m.auth_commits_quarantined_total,
    );
    let n = u64::try_from(bundles).unwrap();
    assert_eq!(report.finalized, n, "every receipt arrived");
    assert_eq!(m.auth_commits_finalized_total, n);
    assert_eq!(m.auth_commits_refused_total, 0);
    assert_eq!(m.auth_commits_integrity_total, 0);
    assert_eq!(m.auth_commits_quarantined_total, 0);
    assert!(
        !m.auth_slot_held,
        "the slot is free once the writer has drained"
    );
    assert_eq!(m.auth_hold_count, n, "one hold observation per commit");
    assert_eq!(m.auth_bytes_count, n, "one size observation per commit");
    assert_eq!(
        m.auth_writes_count, n,
        "one write-count observation per commit"
    );
    assert_eq!(
        m.auth_writes_last, 2,
        "the soak bundle carries two replay writes"
    );
    assert!(m.auth_bytes_last > 0);
    if scale_assert() {
        assert!(
            m.authoritative_hold_p99_ms() <= 250,
            "p99 slot hold {} ms exceeds the 250 ms soak line",
            m.authoritative_hold_p99_ms()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "scale harness: run by the CI `scale` job (workflow_dispatch / weekly)"]
async fn scale_authoritative_soak_reports_hold_time() {
    let bundles = env_usize("SCALE_BUNDLES", 2_000);
    let report = soak(Arc::new(InMemoryStore::new()), bundles).await;
    report_soak("memory", bundles, &report);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "scale harness: run by the CI `scale` job (workflow_dispatch / weekly)"]
async fn scale_authoritative_soak_redis_gated() {
    let Some(store) = live_redis("scale_authoritative_soak_redis_gated", 2).await else {
        return;
    };
    let bundles = env_usize("SCALE_BUNDLES", 2_000);
    let report = soak(Arc::new(store), bundles).await;
    report_soak("redis", bundles, &report);
}

// ---- 4. the tick-shaped bundle ---------------------------------------------------

/// How a synthetic tick of `writes` post-images of `post_bytes` bytes each
/// sits against the three kernel caps.
#[derive(Debug)]
struct TickShape {
    writes: usize,
    post_bytes: usize,
    /// `Err` names the cap the builder refused on.
    encoded_len: Result<usize, String>,
    /// (cap name, numerator, cap) for each of the three caps.
    fractions: [(&'static str, usize, usize); 3],
}

impl TickShape {
    /// The cap the tick is nearest to, and how full it is (per mille).
    fn nearest(&self) -> (&'static str, u64) {
        self.fractions
            .iter()
            .map(|(name, used, cap)| {
                let used = u64::try_from(*used).unwrap_or(u64::MAX);
                let cap = u64::try_from(*cap).unwrap_or(u64::MAX).max(1);
                (*name, used.saturating_mul(1_000) / cap)
            })
            .max_by_key(|(_, per_mille)| *per_mille)
            .expect("three caps")
    }
}

fn tick_shape(writes: usize, post_bytes: usize) -> TickShape {
    let defs = SoakDefs::new();
    let post = json!({ "v": 1, "post": "x".repeat(post_bytes.saturating_sub(16)) });
    let post_encoded = canonical_json_bytes(&post)
        .expect("a string post-image encodes")
        .len();
    let replay = (0..writes)
        .map(|n| ReplayWrite::ModuleJsonSet {
            namespace: defs.owner.clone(),
            key: AuthoritativeKey::parse_canonical(format!("scale/v1/post/{n}")).unwrap(),
            value: post.clone(),
        })
        .collect();
    let encoded_len = match AuthoritativeBundle::new(defs.draft(0, replay)) {
        Ok(bundle) => Ok(bundle.encoded_len()),
        Err(AuthoritativeBundleError::BundleTooLarge { max }) => {
            Err(format!("bundle bytes cap ({max})"))
        }
        Err(other) => Err(format!("{other:?}")),
    };
    let bytes_used = encoded_len
        .clone()
        .unwrap_or(MAX_AUTHORITATIVE_BUNDLE_BYTES);
    TickShape {
        writes,
        post_bytes,
        encoded_len,
        fractions: [
            ("replay_writes", writes, MAX_AUTHORITATIVE_REPLAY_WRITES),
            ("bundle_bytes", bytes_used, MAX_AUTHORITATIVE_BUNDLE_BYTES),
            (
                "outbox_payload_bytes",
                post_encoded,
                MAX_DURABLE_OUTBOX_PAYLOAD_BYTES,
            ),
        ],
    }
}

fn report_tick(shape: &TickShape) {
    let (nearest, per_mille) = shape.nearest();
    println!(
        "scale_tick_shape: writes={} post_bytes={} encoded={:?} nearest_cap={nearest} \
         fill_per_mille={per_mille} fractions={:?}",
        shape.writes, shape.post_bytes, shape.encoded_len, shape.fractions
    );
}

#[test]
#[ignore = "scale harness: run by the CI `scale` job (workflow_dispatch / weekly)"]
fn scale_tick_shaped_bundle_measures_caps() {
    let writes = env_usize("SCALE_TICK_WRITES", 512);
    let post_bytes = env_usize("SCALE_TICK_POST_BYTES", 2_048);
    let shape = tick_shape(writes, post_bytes);
    report_tick(&shape);
    if scale_assert() {
        assert!(
            shape.encoded_len.is_ok(),
            "the tick does not fit: {shape:?}"
        );
        let (nearest, per_mille) = shape.nearest();
        assert!(
            per_mille <= 800,
            "the tick fills {nearest} to {per_mille}‰ — past the 80% headroom line"
        );
    }
}

// ---- the un-ignored tiny seeders ----------------------------------------------------

#[test]
fn h3_journal_seeder_yields_its_stated_count() {
    let stats = replay_journal(seed_journal(10, 3));
    assert_eq!(stats.entries, 10);
    // Before any boot there is nothing to report.
    assert!(EconomyStore::default().boot_replay().is_none());
}

#[test]
fn j2_the_journal_file_round_trips_and_boots() {
    let journal = synth_journal(12, 3);
    let path =
        std::env::temp_dir().join(format!("rustmush_j2_journal_{}.json", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    write_journal_file(&path, &journal.kv, &journal.ledger);
    let back = read_journal_file(&path);
    let _ = std::fs::remove_file(&path);
    assert_eq!(back.ledger, journal.ledger);
    assert_eq!(back.kv.len(), journal.kv.len());
    let tw = boot_split(back.kv, back.ledger);
    assert_eq!(replay_stats(&tw).entries, 12);
}

#[test]
fn h3_synthetic_journal_seeder_yields_its_stated_count_and_replays_clean() {
    let journal = synth_journal(10, 3);
    assert!(journal.chunks >= 1 && journal.bytes > 0);
    // The phases run on the same documents the boot then consumes.
    let _ = replay_phases(&journal.ledger);
    let stats = replay_split(journal.kv, journal.ledger);
    assert_eq!(stats.entries, 10);
    assert_eq!(stats.fetch_ms, 0, "a rig measures no fetch");
}

#[tokio::test(flavor = "multi_thread")]
async fn h3_world_seeder_yields_its_stated_count() {
    let store = InMemoryStore::new();
    let (booted, took) = boot_world(&store, 10).await;
    report_world_boot("memory/tiny", 10, booted, took);
}

#[tokio::test(flavor = "multi_thread")]
async fn h3_soak_seeder_finalizes_its_stated_count() {
    let report = soak(Arc::new(InMemoryStore::new()), 10).await;
    report_soak("memory/tiny", 10, &report);
}

#[test]
fn h3_tick_shape_reports_the_nearest_cap() {
    let shape = tick_shape(8, 64);
    report_tick(&shape);
    assert!(shape.encoded_len.is_ok());
    // Eight writes of 1 024 is the fullest of the three at this size.
    assert_eq!(shape.nearest().0, "replay_writes");
    // Past the write cap the builder refuses and the shape says which cap.
    let over = tick_shape(MAX_AUTHORITATIVE_REPLAY_WRITES + 1, 16);
    assert!(over.encoded_len.is_err(), "{over:?}");
}
