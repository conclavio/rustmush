//! **The boot resumes a running match** (16b cut AF.3; 16c N16; doc 16 §3.1
//! "Durability", "Boot reconciliation", §3.8 "Disconnects", §8 "Restart
//! mid-match").
//!
//! 16b cut AF.1 made the arena a document and AF.2 wrote it every
//! `[tacsim.arena].snapshot_ms` of arena time, beside the world's own record of
//! the match (`tacsim:snapshot:<n>` and `tacsim:live:<n>`, one action on the
//! arena writer lane). This is the boot reading them back: the arena rebuilt on
//! the ground it was fought on, re-opened as a task with its clock re-anchored,
//! the world's record and centre as the last snapshot left them, every seat
//! released to its sealed mind ("to doctrine" here until 17b cut M5.7: one
//! word for every mind) — so the pilots come back through Y.3's `BOARD`, reading
//! what they missed — and a match that cannot resume voided **loudly**, by one
//! reason out of a closed list ([`ResumeRefusal`]).
//!
//! # Two phases, both inside the boot
//!
//! **Phase A** (`phase_a`, from the crate's `boot_matches`, inside
//! `on_world_loaded`) decides and writes nothing but voids. `mush-tactics`' own
//! `on_world_loaded` has run (composition: `mush-sim`, `mush-tactics`, then this
//! crate) and kept every `deployed` marker it found for this window
//! (`mush_tactics::custody::take_boot_deployed`); the markers are grouped by the
//! match they name, and each match is **resumable** only if every check, in a
//! fixed order, passes — the layer on, snapshots on, the live record present,
//! decodable and running, the snapshot's head readable, the two documents naming
//! one ground and one movement table, taken at one moment, the markers exactly the
//! record's roster, and not already resumed from this very snapshot. Then the
//! resumable logs' ledger tails are restored from **their own** chunk documents
//! and the tape is checked against the snapshot. No arena is decoded here: the
//! ground is built from the atlas, which is only certain once every module has
//! loaded.
//!
//! **Phase B** (`phase_b`, from the crate's `on_all_loaded`, before the pod
//! sweep) runs [`resume_pending`] per match in id order, **the fallible steps
//! first** — the ground, the one decode, the tape's resume marker prepared, the
//! task opened — and only then the steps that cannot fail: the records, the
//! marker applied, the seats, the live record rewritten.
//!
//! # The resume-once rule
//!
//! The live record written by a resume carries `resumed_from_ms`, the snapshot
//! moment it resumed from. A process that resumed a match from a snapshot and
//! died before the arena wrote the next one would find the same two documents
//! again; the match is then voided rather than resumed a third time, so one
//! match's fault can never become a crash loop the gateway's `CrashGuard` ends
//! by giving up on the whole game.
//!
//! # The rewind
//!
//! A resume rewinds the match to its last snapshot (16c `Q-AF4`): up to
//! `snapshot_ms` of arena time the old process fought past it are abandoned,
//! and their chunks stay in the append-only log behind a `resumed` marker every
//! reader truncates at (`crate::arena_bridge::tape_in_order`). A pilot the
//! resume released reads [`RESUME_LINE`] before Y.3's [`mind_line`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::{ChunkFloors, Ctx, LedgerDoc, ObjId};
use smol_str::SmolStr;

use crate::arena::intents::ControlState;
use crate::arena::snapshot::{Snapshot, SnapshotError, SnapshotHead};
use crate::arena_bridge;
use crate::arena_task::MatchId;
use crate::match_::{LiveRecord, LiveRecordError, MatchBook, MatchState, lobby, record};

/// **The line a pilot the resume released reads first** (16b cut AF.3; 16c
/// `Q-AF4` at its default). It names no unit and says nothing a sensor did not:
/// only that the match went back to its last save, so a hit or a lock read in
/// the abandoned seconds did not happen.
pub const RESUME_LINE: &str = "The server restarted and the match resumed from its last save; \
                               what happened after that save did not count.";

/// **Y.3's line** (16b cut Y.3; 16c `N4`): the pilot's own tank is flying on
/// its mind and `BOARD` takes it back. One function, so `SeatFacts::connected`
/// and the boarding tick say the same bytes.
///
/// **17b cut M5.3**: `{tank} is flying on its mind; BOARD takes it back.` —
/// it was `doctrine_line`, saying `… is flying on doctrine; …`, until that cut.
/// A released hull flies the mind sealed for it at the check-out (Standard
/// Doctrine unless its pilot chose another — a fixture's mind before 17b cut
/// M6), and the line names none: one word for every mind, as the board's drop
/// clause ([`lobby::DROP_CLAUSE`]) is.
#[must_use]
pub fn mind_line(w: &mush_core::World, tank: ObjId) -> String {
    format!(
        "{} is flying on its mind; BOARD takes it back.",
        mush_core::output::display_name(w, tank)
    )
}

/// **One match phase A found resumable** (16b cut AF.3), held on the match
/// book until phase B: its live record decoded, its snapshot document whole
/// (the state string is decoded once, in phase B, against the real ground),
/// the head read off it, and the units its markers name.
#[derive(Clone, Debug)]
pub struct PendingResume {
    /// The match.
    pub match_id: MatchId,
    /// `tacsim:live:<n>`, decoded.
    pub live: LiveRecord,
    /// `tacsim:snapshot:<n>`, as the store gave it back.
    pub document: serde_json::Value,
    /// The snapshot's head ([`Snapshot::head`]).
    pub head: SnapshotHead,
    /// The units still checked out to it, in id order.
    pub units: Vec<ObjId>,
}

/// **Why a running match could not resume** (16b cut AF.3): the closed list of
/// twenty-one, fourteen decided in phase A and seven in phase B (17b cut M3.3
/// adds the two seal refusals, [`ResumeRefusal::MindAbi`] and
/// [`ResumeRefusal::MindSource`]). Its `Display` is
/// the reason the boot's one error line names, word for word.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResumeRefusal {
    /// `[tacsim].enabled` is off.
    #[error("the simulator is off on this server")]
    LayerOff,
    /// `[tacsim.arena].snapshot_ms` is `0`: the operator's kill switch.
    #[error("snapshots are off on this server")]
    SnapshotsOff,
    /// No `tacsim:live:<n>` whose own match is the key's.
    #[error("no live record")]
    NoLiveRecord,
    /// The live record is there and refuses (its `Display` begins `the live
    /// record does not decode:`).
    #[error("{0}")]
    LiveRecord(LiveRecordError),
    /// The live record is parked (`stopping`): somebody ended the match — or
    /// its centre is not running yet (`boarding`: the check-out's own write,
    /// found beside an opening snapshot whose live rewrite a batch applied in
    /// part lost).
    #[error("the live record is not running (state {state})")]
    NotRunning {
        /// The record's state word.
        state: &'static str,
    },
    /// No `tacsim:snapshot:<n>`.
    #[error("no snapshot document")]
    NoSnapshot,
    /// The snapshot's head (phase A) or its whole decode (phase B) refused.
    #[error("the snapshot does not decode: {}", snapshot_words(.0))]
    Snapshot(SnapshotError),
    /// The head and the record name different grounds.
    #[error("the snapshot and the record name different grounds")]
    DifferentGrounds,
    /// The snapshot was priced under another locomotion table.
    #[error("the ground's movement prices changed (recorded {recorded}, loaded {loaded})")]
    MovementPrices {
        /// The head's `physics_digest`.
        recorded: String,
        /// `mush_atlas::physics::LOCOMOTION_DIGEST`.
        loaded: String,
    },
    /// The two documents are of two moments (a batch applied in part).
    #[error(
        "the live record and the snapshot are from different moments (live at {live} ms, \
         snapshot at {snapshot} ms)"
    )]
    DifferentMoments {
        /// The live record's `at_ms`.
        live: i64,
        /// The head's `at_ms`.
        snapshot: i64,
    },
    /// The units the markers name are not the record's roster.
    #[error("the units still checked out are not the record's roster")]
    RosterMismatch,
    /// The resume-once rule (this module's head).
    #[error(
        "the server stopped again before this match's next snapshot (already resumed from \
         {at_ms} ms)"
    )]
    AlreadyResumed {
        /// The snapshot moment it was last resumed from.
        at_ms: i64,
    },
    /// The resumed log's tail refused: a breach in its chunk documents, or a
    /// marker that would not append.
    #[error("the ledger tail could not be restored: {0}")]
    LedgerTail(String),
    /// The durable tape does not reach the snapshot (E0: a later batch can
    /// commit while an earlier unsealed tail write is dropped).
    #[error(
        "the tape is behind the snapshot (last durable chunk {last}, snapshot at chunk {chunk})"
    )]
    TapeBehind {
        /// The highest arena chunk the kept tape holds.
        last: u64,
        /// The head's `chunk_seq`.
        chunk: u64,
    },
    /// The ground could not be built from the loaded content.
    #[error("the ground {arena} could not be built: {error}")]
    GroundUnbuilt {
        /// The ground's id.
        arena: SmolStr,
        /// Why.
        error: String,
    },
    /// The loaded ground is not the one the match was fought on.
    #[error(
        "the ground {arena} is not the ground loaded on this server (recorded {recorded}, \
         loaded {loaded})"
    )]
    GroundDiffers {
        /// The ground's id.
        arena: SmolStr,
        /// The head's digest.
        recorded: String,
        /// The loaded ground's digest.
        loaded: String,
    },
    /// The decoded arena hashes to something else.
    #[error("state hash mismatch (recorded {recorded}, decoded {decoded})")]
    StateHash {
        /// The head's `state_hash`.
        recorded: u64,
        /// The decoded arena's.
        decoded: u64,
    },
    /// The boot is not inside a runtime to spawn the task on.
    #[error("no runtime at boot")]
    NoRuntime,
    /// The task could not be opened.
    #[error("the arena could not be opened: {0}")]
    NotOpened(String),
    /// **17b cut M3.3** (doc 17 §13; D-UM-23's R-B clause, D-16-3): a unit's
    /// seal was taken under another mind engine ABI than this server's
    /// `MIND_ABI` — checked on EVERY seal, a reflex one included, as
    /// `SnapshotError::Physics` voids regardless of use, so a deploy that
    /// bumps `MIND_ABI` voids every in-flight match at that boot.
    #[error("{unit}'s mind was sealed on engine {recorded} and this server runs {running}")]
    MindAbi {
        /// The unit.
        unit: ObjId,
        /// The seal's `abi`.
        recorded: SmolStr,
        /// This server's `MIND_ABI`.
        running: SmolStr,
    },
    /// **17b cut M3.3** (doc 17 §13): a scripted seal names a source neither
    /// compiled in (Standard Doctrine's own, 17b cut M4.3) nor in the store
    /// (`tacsim:mindsrc:<sha256>`), so the mind cannot be run again.
    #[error(
        "{unit}'s mind source {} is neither this server's Standard Doctrine nor in its store",
        crate::arena::digest_text(.digest)
    )]
    MindSource {
        /// The unit.
        unit: ObjId,
        /// The seal's `source_sha256`.
        digest: [u8; 32],
    },
}

/// A snapshot refusal in the words after `the snapshot does not decode:` — its
/// own `Display`, except `Malformed`, whose `Display` already begins with those
/// words and is not said twice.
fn snapshot_words(error: &SnapshotError) -> String {
    match error {
        SnapshotError::Malformed(detail) => detail.clone(),
        other => other.to_string(),
    }
}

/// **What `boot_matches` read off the store for this module**: every running
/// match's two documents, by id.
#[derive(Debug, Default)]
pub(crate) struct BootDocuments {
    /// `tacsim:live:<n>`, decoded — or its refusal — by the key's id; a record
    /// whose own match disagrees with its key is left out (the V.2 rule).
    pub(crate) lives: BTreeMap<MatchId, Result<LiveRecord, LiveRecordError>>,
    /// `tacsim:snapshot:<n>`, as given, by the key's id.
    pub(crate) snapshots: BTreeMap<MatchId, serde_json::Value>,
    /// Every id either key names, so a pair no marker claims is deleted.
    pub(crate) ids: BTreeSet<MatchId>,
}

/// **Phase A** (16b cut AF.3; this module's head): decide which matches with
/// markers resume, restore their logs' tails, void the rest by name, delete the
/// documents nobody claims, and answer the resumed matches' units — the tanks
/// the boot's stock-tank reset must leave alone, quietly.
pub(crate) fn phase_a(
    ctx: &mut Ctx<'_>,
    mut documents: BootDocuments,
    ledger: &[LedgerDoc],
    omitted: &ChunkFloors,
) -> BTreeSet<ObjId> {
    let mut by_match: BTreeMap<MatchId, Vec<ObjId>> = BTreeMap::new();
    for (unit, marker) in mush_tactics::custody::take_boot_deployed(ctx.w) {
        // Doc 32 cut CF.2 (Q-CF-35): an IC field's unit is never voided to its check-out
        // snapshot; the field host resumes it or demotes it from the field's last snapshot.
        if let Some(field) = &marker.mark.field {
            crate::field::boot::adopt_mark(ctx.w, field.site.as_str(), unit);
            continue;
        }
        by_match
            .entry(MatchId(marker.match_id()))
            .or_default()
            .push(unit);
    }
    let (enabled, snapshot_ms) = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .map_or((false, 0), |store| {
            (store.enabled(), store.config().arena.snapshot_ms)
        });
    let mut candidates: Vec<PendingResume> = Vec::new();
    let mut voids: Vec<(MatchId, Vec<ObjId>, ResumeRefusal)> = Vec::new();
    for (match_id, mut units) in by_match {
        units.sort_unstable();
        let live = documents.lives.remove(&match_id);
        let snapshot = documents.snapshots.remove(&match_id);
        match check(enabled, snapshot_ms, match_id, &units, live, snapshot) {
            Ok(pending) => candidates.push(pending),
            Err(refusal) => voids.push((match_id, units, refusal)),
        }
    }

    // The ledger tails, for the resumable logs alone and over their own
    // documents alone. Each log is inspected by itself first — on a scratch
    // ledger at the registry's target, which restores exactly as the registry's
    // will and refuses exactly as it would — so a breach voids its own match and
    // no other; the tape is checked against the snapshot on the same
    // inspection; and then the survivors are restored together, because a
    // restore replaces the ledger's whole set of tails.
    let target = arena_bridge::ledger_chunk_target(ctx.w);
    let mut resumable: Vec<PendingResume> = Vec::new();
    let mut resumed_docs: Vec<LedgerDoc> = Vec::new();
    let mut logs: BTreeSet<String> = BTreeSet::new();
    for pending in candidates {
        let log = arena_bridge::match_log_id(pending.match_id);
        let docs: Vec<LedgerDoc> = ledger
            .iter()
            .filter(|doc| mush_core::parse_chunk_key(&doc.0).is_some_and(|(id, _)| id == log))
            .cloned()
            .collect();
        let history = mush_core::inspect_ledger_from(&docs, Some(target), omitted);
        if let Err(error) = mush_core::Ledger::with_chunk_target(target)
            .restore_logs_from_history(&history, |_| true)
        {
            voids.push((
                pending.match_id,
                pending.units,
                ResumeRefusal::LedgerTail(error.to_string()),
            ));
            continue;
        }
        let last = last_durable_chunk(&history, &log, &docs);
        if last < pending.head.chunk_seq {
            voids.push((
                pending.match_id,
                pending.units,
                ResumeRefusal::TapeBehind {
                    last,
                    chunk: pending.head.chunk_seq,
                },
            ));
            continue;
        }
        resumed_docs.extend(docs);
        logs.insert(log);
        resumable.push(pending);
    }
    if !resumable.is_empty() {
        let history = mush_core::inspect_ledger_from(&resumed_docs, Some(target), omitted);
        if let Err(error) = arena_bridge::restore_ledger_tails(ctx.w, &history, &logs) {
            // Every log passed alone, so this is not expected; when it happens
            // no tail is restored, and a resumed append could then overwrite
            // chunk 0 — so none of them resumes.
            for pending in resumable.drain(..) {
                voids.push((
                    pending.match_id,
                    pending.units,
                    ResumeRefusal::LedgerTail(error.to_string()),
                ));
            }
        }
    }

    voids.sort_by_key(|(match_id, _, _)| *match_id);
    let voided: BTreeSet<MatchId> = voids.iter().map(|(match_id, _, _)| *match_id).collect();
    for (match_id, units, refusal) in voids {
        void_at_boot(ctx, match_id, &units, &refusal.to_string());
    }
    // A running match's documents that no marker claims describe a match that
    // is over: deleted, as AF.2's boot deleted every one.
    let resumed: BTreeSet<MatchId> = resumable.iter().map(|pending| pending.match_id).collect();
    for match_id in &documents.ids {
        if !resumed.contains(match_id) && !voided.contains(match_id) {
            record::delete_live_and_snapshot(ctx, *match_id);
        }
    }
    let keep: BTreeSet<ObjId> = resumable
        .iter()
        .flat_map(|pending| pending.units.iter().copied())
        .collect();
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        book.pending_resumes = resumable;
    }
    keep
}

/// **Phase A's named checks for one match, in their order** (the module
/// head). The first to fail is the reason.
fn check(
    enabled: bool,
    snapshot_ms: u32,
    match_id: MatchId,
    units: &[ObjId],
    live: Option<Result<LiveRecord, LiveRecordError>>,
    snapshot: Option<serde_json::Value>,
) -> Result<PendingResume, ResumeRefusal> {
    if !enabled {
        return Err(ResumeRefusal::LayerOff);
    }
    if snapshot_ms == 0 {
        return Err(ResumeRefusal::SnapshotsOff);
    }
    let live = live
        .ok_or(ResumeRefusal::NoLiveRecord)?
        .map_err(ResumeRefusal::LiveRecord)?;
    if live.record.state != MatchState::Running {
        return Err(ResumeRefusal::NotRunning {
            state: live.record.state.as_str(),
        });
    }
    // And its centre (16b cut AF.2's carried constraint): `at_ms` is `0` both
    // for the check-out's own write — a lobby centre still `Boarding` there —
    // and for the opening snapshot, so a batch that landed the opening
    // snapshot and lost its live rewrite would pass the moment check below
    // with the check-out's record beside it.
    if live.centre.state != MatchState::Running {
        return Err(ResumeRefusal::NotRunning {
            state: live.centre.state.as_str(),
        });
    }
    let document = snapshot.ok_or(ResumeRefusal::NoSnapshot)?;
    let head = Snapshot::head(&document).map_err(ResumeRefusal::Snapshot)?;
    if head.arena != live.record.arena
        || head.definition_digest != live.record.arena_definition_digest
    {
        return Err(ResumeRefusal::DifferentGrounds);
    }
    // The definition's digest omits the per-row prices since 16b cut AE.1, so
    // this is the only check that catches a re-priced class before phase B.
    if head.physics_digest != mush_atlas::physics::LOCOMOTION_DIGEST {
        return Err(ResumeRefusal::MovementPrices {
            recorded: head.physics_digest,
            loaded: mush_atlas::physics::LOCOMOTION_DIGEST.to_owned(),
        });
    }
    // The two `JsonSet`s ride one action, but a batch can apply in part (E0).
    if live.at_ms != head.at_ms {
        return Err(ResumeRefusal::DifferentMoments {
            live: live.at_ms,
            snapshot: head.at_ms,
        });
    }
    let marked: BTreeSet<ObjId> = units.iter().copied().collect();
    let roster: BTreeSet<ObjId> = live.record.units().into_iter().collect();
    if marked != roster {
        return Err(ResumeRefusal::RosterMismatch);
    }
    if live.resumed_from_ms == Some(head.at_ms) {
        return Err(ResumeRefusal::AlreadyResumed { at_ms: head.at_ms });
    }
    Ok(PendingResume {
        match_id,
        live,
        document,
        head,
        units: units.to_vec(),
    })
}

/// **The highest arena chunk one log's durable tape holds**, after
/// [`arena_bridge::tape_in_order`] — so the abandoned tail behind an earlier
/// resume's marker does not count — or `0` for a log with no chunk at all.
fn last_durable_chunk(history: &mush_core::LedgerHistory, log: &str, docs: &[LedgerDoc]) -> u64 {
    let mut entries: Vec<serde_json::Value> = Vec::new();
    for (_, position) in history.chunk_docs(log) {
        let Some((_, text)) = docs.get(position) else {
            continue;
        };
        if let Ok(chunk) = serde_json::from_str::<mush_core::LedgerChunk>(text) {
            entries.extend(chunk.entries.into_iter().map(|entry| entry.value));
        }
    }
    let (kept, _) = arena_bridge::tape_in_order(&entries);
    kept.iter()
        .filter_map(|entry| entry.get("chunk").and_then(serde_json::Value::as_u64))
        .max()
        .unwrap_or(0)
}

/// **Phase B** (16b cut AF.3; this module's head): every match phase A held,
/// re-opened in id order through [`resume_pending`], each refusal voided by
/// name — and then, when any was, the stock-tank reset once more with the
/// matches still resumed kept, so a roster tank of a match voided here comes
/// back at spec and at muster exactly as a phase-A void's does.
pub(crate) fn phase_b(ctx: &mut Ctx<'_>) {
    let pending = ctx
        .w
        .module_state_mut::<MatchBook>()
        .map(|book| std::mem::take(&mut book.pending_resumes))
        .unwrap_or_default();
    if pending.is_empty() {
        return;
    }
    let mut kept: BTreeSet<ObjId> = BTreeSet::new();
    let mut voided = false;
    for pending in pending {
        let (match_id, units) = (pending.match_id, pending.units.clone());
        match resume_pending(ctx, pending) {
            Ok(()) => kept.extend(units),
            Err(refusal) => {
                void_at_boot(ctx, match_id, &units, &refusal.to_string());
                voided = true;
            }
        }
    }
    if voided {
        let tanks = crate::match_::reset_stock_tanks(ctx, &kept);
        if tanks.changed() {
            tracing::info!(
                restored = tanks.restored.len(),
                remustered = tanks.remustered.len(),
                "tacsim: the stock tanks of a match that could not resume are back at their \
                 spec and at muster"
            );
        }
    }
}

/// **Re-open one match phase A found resumable** (16b cut AF.3), in this order —
/// the fallible steps first, so a refusal leaves nothing registered and
/// nothing written:
///
/// 1. **the ground**, built from the loaded content once per digest and held on
///    the book (`MatchBook::hold_ground`, so a replay of the same ground shares
///    it), and refused unless its digest is the head's;
/// 2. **the one decode** ([`Snapshot::decode`]: every check of 16b cut AF.1 —
///    the movement table again, the state digest, the parse, the pose checks on
///    the real ground, and the state hash); then (**17b cut M3.3**) **every
///    seal** the record carries held runnable here — its `abi` this build's
///    `MIND_ABI`, a scripted seal's source in the store
///    ([`ResumeRefusal::MindAbi`], [`ResumeRefusal::MindSource`]);
/// 3. **the tape's resume marker, prepared** — `{"resumed": {"from_chunk",
///    "at_ms", "restart_ms"}}` — because a tail sealed at its target refuses it;
/// 4. **the task** ([`arena_bridge::resume_match`]), which answers the decoded
///    arena's seated roster.
///
/// Then what cannot fail:
///
/// 5. **the records**: the match's record back on the book as the live record
///    carried it (the folds, the replay fold, the callsigns, the squads, the
///    seed, the mode), and the centre as recorded — `Running` on this match,
///    its slots, leads, mode and range as they were, its boarding deadline and
///    unattended stamp cleared. The live record always carries a centre on
///    this match (`record::write_live` writes the slotless one `lobby::adopt`
///    binds when the book's centre is not on it), so a staff `@arena/start`
///    match's centre comes back as `adopt` left it, with no second path;
/// 6. **the marker applied**, in the boot's own write, ahead of the task's
///    first chunk (which reaches the world only through a later drain);
/// 7. **the seats, reconciled from both sides**: every `(pilot, unit)` the
///    decoded arena still seats is released to its sealed mind on every channel
///    (`slot_dropped` first, when its slot is still seated — an `EXIT` the world
///    saw after the snapshot's pulse stays final), and every seated, un-left
///    slot whose pilot the arena does NOT hold (a `BOARD` reclaim the world saw
///    between the snapshot's pulse and its drain) is dropped and marked left with
///    no `Control` — the arena already flies that tank. Every slot either touched
///    is marked `released_by_resume` and owed the two lines
///    (`MatchBook::defer_resume_lines`); then, when every slot is left and
///    nothing stamped the unattended clock, it is stamped now;
/// 8. **the live record rewritten** ([`record::write_live`]), carrying
///    `resumed_from_ms` and the reconciled slots.
///
/// # Errors
///
/// The phase-B [`ResumeRefusal`]s, and [`ResumeRefusal::Snapshot`] or
/// [`ResumeRefusal::LedgerTail`] from steps 2 and 3.
pub fn resume_pending(ctx: &mut Ctx<'_>, pending: PendingResume) -> Result<(), ResumeRefusal> {
    let PendingResume {
        match_id,
        live,
        document,
        head,
        units,
    } = pending;
    // (1) The ground.
    let ground = ground_for_resume(ctx, &live.record.arena, &head.definition_digest)?;
    // (2) The one decode.
    let limit = ctx.w.module_state::<crate::store::TacsimStore>().map_or(
        crate::arena::snapshot_capacity::SNAPSHOT_MAX_BYTES,
        |store| store.config().arena.snapshot_max_bytes,
    );
    let arena = Snapshot::decode_with_limit(&document, ground.as_ref().clone(), limit).map_err(
        |error| match error {
            SnapshotError::HashMismatch { recorded, decoded } => {
                ResumeRefusal::StateHash { recorded, decoded }
            }
            other => ResumeRefusal::Snapshot(other),
        },
    )?;
    // (2b) **17b cut M3.3** (doc 17 §13): every seal the record carries must
    // be runnable on this server — its ABI this build's, and a scripted
    // seal's source in the store. The decoded arena keeps its own seal
    // references (the snapshot carries them); nothing here re-derives them.
    refuse_unrunnable_seals(ctx, &live.record)?;
    // (3) The marker, prepared.
    let now_ms = ctx.w.now().saturating_mul(1_000);
    let marker = serde_json::json!({
        "resumed": {
            "from_chunk": head.chunk_seq,
            "at_ms": head.at_ms,
            "restart_ms": now_ms,
        }
    });
    let prepared = arena_bridge::prepare_resume_marker(ctx.w, match_id, marker)
        .map_err(ResumeRefusal::LedgerTail)?;
    // (4) The task.
    let roster =
        arena_bridge::resume_match(ctx, match_id, arena, &live).map_err(|error| match error {
            arena_bridge::OpenMatchError::NoRuntime => ResumeRefusal::NoRuntime,
            other => ResumeRefusal::NotOpened(other.to_string()),
        })?;

    // (5) The records. The live record always carries a centre on this match —
    // `record::write_live` writes the book's centre when it is on the match and
    // otherwise the one `lobby::adopt` binds for a staff `@arena/start` — so the
    // recorded centre is restored in both cases, and a staff match comes back
    // exactly as `adopt` left it.
    let LiveRecord {
        record, mut centre, ..
    } = live;
    centre.state = MatchState::Running;
    centre.match_id = Some(match_id);
    centre.board_deadline_ms = None;
    centre.unattended_since_ms = None;
    centre.unattended_warned = false;
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        book.matches.insert(match_id, record);
        *book.centre_mut() = centre;
    }
    // (6) The marker, applied.
    arena_bridge::append_resume_marker(ctx, match_id, prepared);
    // (7) The seats.
    let released = reconcile_seats(ctx, &roster, now_ms);
    // (8) The live record.
    record::write_live(ctx, match_id);
    tracing::info!(
        match_id = match_id.0,
        at_ms = head.at_ms,
        units = units.len(),
        seats_released = released,
        "tacsim: match {} resumed from its snapshot at {} ms of arena time; every seat is \
         released to doctrine until its pilot boards again",
        match_id.0,
        head.at_ms
    );
    Ok(())
}

/// **Step 2b of [`resume_pending`]** (17b cut M3.3; doc 17 §13, D-16-3): the
/// first seal, in unit order, that this server cannot run, refused by name —
/// one recorded under another `MIND_ABI` (every engine's, the reflex's
/// included), or a [`SealEngine::Scripted`] one whose source the store does
/// not hold. The caller voids the match with the refusal's own sentence
/// ([`void_at_boot`], which writes no tape line).
///
/// [`SealEngine::Scripted`]: crate::arena::mind::SealEngine::Scripted
fn refuse_unrunnable_seals(
    ctx: &Ctx<'_>,
    record: &crate::match_::MatchRecord,
) -> Result<(), ResumeRefusal> {
    let store = ctx.w.module_state::<crate::store::TacsimStore>();
    for (unit, sealed) in &record.minds {
        let seal = &sealed.seal;
        if seal.abi != crate::minds::MIND_ABI {
            return Err(ResumeRefusal::MindAbi {
                unit: *unit,
                recorded: seal.abi.clone(),
                running: SmolStr::new_static(crate::minds::MIND_ABI),
            });
        }
        // 17b cut M4.3: the compiled-in Standard Doctrine answers its own
        // digest with no store (`crate::minds::source_lookup`).
        if seal.engine == crate::arena::mind::SealEngine::Scripted
            && crate::minds::source_lookup(store, &seal.source_sha256).is_none()
        {
            return Err(ResumeRefusal::MindSource {
                unit: *unit,
                digest: seal.source_sha256,
            });
        }
    }
    Ok(())
}

/// Step 1 of [`resume_pending`]: the ground held on the book under `recorded`,
/// or built from the loaded content, checked and held.
fn ground_for_resume(
    ctx: &mut Ctx<'_>,
    arena: &SmolStr,
    recorded: &str,
) -> Result<Arc<crate::arena::state::ArenaDefinition>, ResumeRefusal> {
    if let Some(held) = ctx
        .w
        .module_state::<MatchBook>()
        .and_then(|book| book.ground(recorded))
    {
        return Ok(held);
    }
    let built = crate::match_::definition_for(ctx.w, arena)
        .map_err(|error| ResumeRefusal::GroundUnbuilt {
            arena: arena.clone(),
            error,
        })?
        .ok_or_else(|| ResumeRefusal::GroundUnbuilt {
            arena: arena.clone(),
            error: "no such arena is loaded".to_owned(),
        })?;
    let loaded = arena_bridge::digest_text(&built.digest());
    if loaded != recorded {
        return Err(ResumeRefusal::GroundDiffers {
            arena: arena.clone(),
            recorded: recorded.to_owned(),
            loaded,
        });
    }
    let ground = Arc::new(built);
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        book.hold_ground(loaded, Arc::clone(&ground));
    }
    Ok(ground)
}

/// Step 7 of [`resume_pending`]: the seats, from both sides, and the unattended
/// restamp. Answers how many slots it released.
fn reconcile_seats(ctx: &mut Ctx<'_>, roster: &[(ObjId, ObjId)], now_ms: i64) -> usize {
    let mut touched: Vec<(ObjId, ObjId)> = Vec::new();
    // (i) From the arena's roster: every seat it still holds goes to its mind.
    for (pilot, unit) in roster {
        if lobby::slot_dropped(ctx, *pilot, now_ms) {
            touched.push((*pilot, *unit));
        }
        // Unconditionally: the arena flies this tank under a pilot controller
        // until it hears otherwise, whatever the slot says.
        arena_bridge::control_all_channels(ctx, *unit, ControlState::Released, *pilot);
    }
    // (ii) From the slots: a seat the world gave back after the snapshot's pulse
    // (a reclaim whose `Live` the arena never took) is dropped again, with no
    // `Control` — the arena already flies it and holds its `at_release` row.
    let reclaimed: Vec<(ObjId, ObjId)> = lobby::centre(ctx.w)
        .map(|centre| {
            centre
                .slots
                .iter()
                .filter(|slot| !slot.left && !roster.iter().any(|(pilot, _)| *pilot == slot.pilot))
                .filter_map(|slot| slot.unit.map(|unit| (slot.pilot, unit)))
                .collect()
        })
        .unwrap_or_default();
    for (pilot, unit) in reclaimed {
        lobby::slot_dropped(ctx, pilot, now_ms);
        lobby::slot_left(ctx, pilot);
        touched.push((pilot, unit));
    }
    let released = touched.len();
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        for (pilot, unit) in touched {
            if let Some(slot) = book
                .centre_mut()
                .slots
                .iter_mut()
                .find(|slot| slot.pilot == pilot)
            {
                slot.released_by_resume = true;
            }
            book.defer_resume_lines(pilot, unit);
        }
        // The restamp: `slot_left` stamps only while it marks a slot, so a
        // match whose slots were ALL left before the restart would otherwise
        // run its full length on the one centre with nobody back.
        let centre = book.centre_mut();
        if centre.unattended_since_ms.is_none()
            && !centre.slots.is_empty()
            && centre.slots.iter().all(|slot| slot.left)
        {
            centre.unattended_since_ms = Some(now_ms);
        }
    }
    released
}

/// **Void one running match the boot could not resume** (16b cut AF.3; what
/// every restart did to every match before it): each unit restored from its
/// check-out snapshot (`mush_tactics::custody::restore_at_boot`) and its seats
/// released, the `aborted(restart)` result written
/// ([`record::write_restart_abort`], its document unchanged), the match's two
/// documents deleted, and **one** error line naming the reason — a
/// [`ResumeRefusal`]'s words.
pub fn void_at_boot(ctx: &mut Ctx<'_>, match_id: MatchId, units: &[ObjId], reason: &str) {
    for unit in units {
        if let Err(error) = mush_tactics::custody::restore_at_boot(ctx, *unit) {
            tracing::error!(
                %unit,
                match_id = match_id.0,
                %error,
                "tacsim: a unit of a match that could not resume was not restored at boot; its \
                 marker stays, and `@arena/release <unit>` or the next boot is what clears one"
            );
        }
        crate::match_::release_seats_after_restart(ctx, *unit);
    }
    record::write_restart_abort(ctx, match_id, units);
    record::delete_live_and_snapshot(ctx, match_id);
    tracing::error!(
        match_id = match_id.0,
        units = units.len(),
        reason,
        "tacsim: match {} was still running when this server last stopped and could not resume \
         ({reason}); its units are restored from their check-out snapshots and the match is \
         recorded aborted(restart)",
        match_id.0
    );
}
