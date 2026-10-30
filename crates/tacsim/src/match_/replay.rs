//! **The replay** (16b cut I.1; 16c N9 and Q37; doc 16 §3.6 "What a pilot
//! sees", §4 "Ledger events" and "The result record", §5 the observer rule).
//!
//! The black box records every unit's position every `ledger.summary_ms`
//! (5,000 ms of arena time), every shot and every wreck. This module is the
//! watching back, built the way 16b cut V.3 built the debrief:
//!
//! 1. **[`ReplayFold`]**, one pure fold per match, lives on the match's record
//!    ([`crate::match_::MatchRecord::replay`]). It is seeded at check-out with
//!    the opening poses (keyframe 0), fed every event of every chunk by
//!    `crate::arena_bridge`'s `on_chunk` as the world appends the chunk, and
//!    finished exactly once by [`crate::match_::finish`] into one
//!    [`Replay`] — written as **one `tacsim:replay:<match>` document**
//!    ([`replay_key`]) in the result's own action and held on the centre's RAM
//!    ring ([`crate::match_::MatchBook::replay`]) beside the ground it was
//!    fought on ([`crate::match_::MatchBook::ground`]). The ring's eviction
//!    deletes it with the reports, so the documents are bounded by the ring.
//! 2. **A replay is a picture, not a second tape** (the cut I design note,
//!    correction 2). Keyframes carry each unit's arena **cell** and whole-degree
//!    heading as positional rows in roster order; shots carry the firing time,
//!    the two ends by roster index and the outcome word; wrecks the time and the
//!    index; the roster callsigns and sides. **No object number, no pilot, no
//!    contact id, no draw, no seed, no `z`.** It is held to the project's bound
//!    on a pulled keyframe ([`REPLAY_MAX_BYTES`], `MAP_MAX_BYTES`, 16c `T-57`)
//!    by a running byte budget that is the binding cap behind the two shape caps
//!    ([`REPLAY_MAX_KEYFRAMES`], [`REPLAY_MAX_SHOTS`]).
//! 3. **`SIM REPLAY <match> [<m:ss> | next | prev] [<callsign>]`** ([`request`])
//!    prints one keyframe's window in line mode under the one map-window rule
//!    (`VIEW`'s 39 by 20, two printed columns a cell) and steps through them
//!    with the actor's **replay cursor** ([`ReplayCursor`]), which is also what
//!    the `arena.replay` and `arena.map` queries read: a panel query carries no
//!    parameters, so a query can only be keyed on who asks.
//!
//! # Objective and public (16c Q37)
//!
//! The replay of a finished OOC match is **both sides, as the arena recorded
//! them** — the tanks are back at spec and it is practice — and says so on
//! every reply. The debrief ([`crate::match_::report`]) stays the pilot's own
//! projection. What the replay never reads is anything per-viewer: no
//! `detected`, `classified`, `lost`, `locked` line and no draw is folded, so
//! nothing anybody's sensors saw is in it. A running match has no replay; its
//! one answer is `Match 12 is still running.`.
//!
//! # Held to the arena library's standard
//!
//! The fold and the window are under `match_/` and not `arena/`, so
//! `crates/mush-contract-tests/tests/arena_library.rs` does not scan them, but
//! they keep the same rules: no I/O, no world, no clock, no float, every map a
//! `BTreeMap`. The request, the queries' answers and `ground_for` are the
//! world-side half and the only part that touches a [`World`] or a [`Ctx`].

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{Audience, CmdError, Ctx, ObjId, World};
use mush_proto::OutputEvent;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::ledger::{EventKind, LedgerEvent};
use crate::arena::state::{ArenaDefinition, ArenaTerrain, Designator, KeySpot, MatchMode, PartyId};
use crate::arena_bridge::NOTICE_CATEGORY;
use crate::arena_task::MatchId;
use crate::match_::MatchBook;
use crate::match_::cockpit::{
    MAP_MAX_BYTES, RETAIN_LATEST, SPOT_RANGE_STEP_M, VIEW_CELL_COLUMNS, VIEW_COLUMNS, VIEW_ROWS,
    range_m_rounded, spot_markers, terrain_rows,
};
use crate::match_::lobby;
use crate::match_::record::{HISTORY_DEPTH, MatchOutcome, duration_words, outcome_words};

// ---------------------------------------------------------------------------
// The constants
// ---------------------------------------------------------------------------

/// The record word a replay document carries.
pub const REPLAY_RECORD: &str = "tacsim.replay";

/// **The most keyframes one replay keeps** (16b cut I.1): 736, which at the
/// shipped `ledger.summary_ms` of 5,000 is 61 minutes 20 seconds of arena time
/// — four times the fifteen-minute cap (16b cut AP.2), and the hour it was
/// before with two summaries of margin, pinned against the default
/// configuration. `MatchConfig::validate` puts no ceiling on the cap, so a
/// replay of a match an operator configured past the hour outruns this cap (a
/// 16c debt). A shape cap: the byte budget
/// ([`REPLAY_MAX_BYTES`]) is the one that binds first on a full staff match.
pub const REPLAY_MAX_KEYFRAMES: usize = 736;

/// **The most shots one replay keeps** (16b cut I.1). A shape cap, behind the
/// byte budget.
pub const REPLAY_MAX_SHOTS: usize = 1_024;

/// **The most units one replay names** (16b cut I.1):
/// [`crate::definitions::arena::MAX_ARENA_UNITS`], the most an arena fields, so
/// a staff match of twenty-four is named whole and a lobby match of twelve
/// (16b cut Z.3) is half of it. A roster past it keeps its first units in
/// unit-id order and sets `truncated`.
pub const REPLAY_MAX_UNITS: usize = crate::definitions::arena::MAX_ARENA_UNITS;

/// **The bound one replay document is held to**, bytes of encoded JSON (16b
/// cut I.1; 16c `T-57`'s rule): [`MAP_MAX_BYTES`], the most one pulled keyframe
/// may cost a session's output queue, because the `arena.replay` query answers
/// the whole document at once.
pub const REPLAY_MAX_BYTES: usize = MAP_MAX_BYTES;

/// **What the byte budget keeps for everything but keyframes and shots** (16b
/// cut I.1): the match's facts, the roster (at most [`REPLAY_MAX_UNITS`]
/// callsigns) and the wrecks (at most one a unit), which together are well
/// under a kilobyte at twenty-four units.
pub const REPLAY_HEAD_BYTES: usize = 4 * 1024;

/// **The line window a replay keyframe is drawn in**, cells (16b cut I.1): the
/// one map-window rule — `VIEW`'s [`VIEW_COLUMNS`] by [`VIEW_ROWS`], two
/// printed columns a cell on a line client — clamped to the arena.
pub const REPLAY_WINDOW: (u16, u16) = (VIEW_COLUMNS, VIEW_ROWS);

/// The marker a red unit is drawn with on a replay window.
pub const RED_GLYPH: &str = "o";

/// The marker a blue unit is drawn with on a replay window.
pub const BLUE_GLYPH: &str = "*";

/// The marker a unit is drawn with from the keyframe its wreck was recorded at
/// or before.
pub const WRECK_GLYPH: &str = "%";

/// The replay window's schema id, and the feed it is pushed and asked on
/// (`crate::arena_bridge::REPLAY_FEED`).
pub const REPLAY_SCHEMA: &str = "arena.replay";

/// Its version. A field added is not a version, as
/// [`crate::match_::cockpit::FRAME_VERSION`]'s comment says of the cockpit
/// frame.
pub const REPLAY_VERSION: u32 = 1;

/// **The shape the `arena.replay` query answers under** (16b cut I.1).
///
/// Deliberately **not** `grid`: the answer is the whole document, and a line
/// client that somehow received one must print nothing, which a shape it does
/// not know is how it does that (the rule [`crate::match_::cockpit::MAP_SHAPE`]
/// follows). The pushed window is the `grid`.
pub const REPLAY_DOCUMENT_SHAPE: &str = "replay";

/// **The most entries the off-window and shots lines name** (16b cut I.1)
/// before `, and N more`: Z.3's twelve tanks and a staff match's twenty-four
/// must not print a screenful.
pub const REPLAY_LIST_CAP: usize = 8;

/// The answer the `arena.replay` query gives an actor with no replay open —
/// nobody has typed `SIM REPLAY`, or their last one was refused.
pub const NO_REPLAY_OPEN: &str = "You have no replay open; type SIM REPLAY <match>.";

/// The usage line every malformed `SIM REPLAY` answers.
pub const REPLAY_USAGE: &str = "Usage: SIM REPLAY <match> [<m:ss> | next | prev] [<callsign>]";

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

/// **One unit of a replay's roster** (16b cut I.1): the callsign every viewer
/// calls it by and its side. No object number and no pilot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayRoster {
    /// The designator's `Display` (`RA`, `BC`).
    pub callsign: SmolStr,
    /// Its side.
    pub team: PartyId,
}

/// **Where one unit stood at one keyframe** (16b cut I.1): an arena cell and a
/// heading in whole degrees clockwise from north. No `z`: a unit's height
/// inside a match is its cell's elevation (doc 16 §3.2), which the ground
/// already says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyframeUnit {
    /// The arena cell's column.
    pub x: u16,
    /// The arena cell's row.
    pub y: u16,
    /// Whole degrees, `0..360`.
    pub heading_deg: u16,
}

/// **One keyframe** (16b cut I.1): every roster unit at one arena millisecond.
///
/// **Positional**: `units[i]` is roster entry `i`, `None` when the unit is not
/// in this keyframe (off the window, or not named by the arena's summary). One
/// slot per roster entry and no key per unit is what keeps a twenty-four-unit
/// hour near the bound.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keyframe {
    /// Arena milliseconds.
    pub t_ms: i64,
    /// One slot per roster entry, in roster order.
    pub units: Vec<Option<KeyframeUnit>>,
}

/// **What became of one shot** (16b cut I.1), as the tape said it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShotOutcome {
    /// Fired, and no `hit` or `miss` line followed before the tape ended.
    Flying,
    /// A `hit` line.
    Hit,
    /// A `miss` line, whatever its reason.
    Miss,
}

impl ShotOutcome {
    /// The document's word: `flying`, `hit` or `miss`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ShotOutcome::Flying => "flying",
            ShotOutcome::Hit => "hit",
            ShotOutcome::Miss => "miss",
        }
    }

    /// The word a reply line says: `in flight`, `hit` or `miss`.
    #[must_use]
    pub const fn words(&self) -> &'static str {
        match self {
            ShotOutcome::Flying => "in flight",
            ShotOutcome::Hit => "hit",
            ShotOutcome::Miss => "miss",
        }
    }

    /// The word read back, or `None`.
    #[must_use]
    pub fn parse(word: &str) -> Option<ShotOutcome> {
        match word {
            "flying" => Some(ShotOutcome::Flying),
            "hit" => Some(ShotOutcome::Hit),
            "miss" => Some(ShotOutcome::Miss),
            _ => None,
        }
    }
}

/// **What a replay left out, and why** (16b cut I.1): the causes behind
/// [`Replay::truncated`], each of which the reply words its own way.
///
/// **A cut is a seal, never a hole.** The first keyframe refused (at
/// [`REPLAY_MAX_KEYFRAMES`] or the byte budget) seals the picture at the last
/// keyframe kept: no later keyframe, shot or wreck is taken, and the shots and
/// wrecks already taken past that keyframe are dropped, so the replay is the
/// prefix its line names and a later, smaller keyframe cannot open a gap that
/// `next` would step across. The first shot refused seals the shots alone —
/// keyframes go on being taken, because the movement is what a replay is for,
/// and a sustained fight of twelve tanks passes [`REPLAY_MAX_SHOTS`] in
/// minutes. A lost page is a hole in the tape itself, which no seal can hide,
/// and is said as one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayCut {
    /// A roster past [`REPLAY_MAX_UNITS`] kept its first units.
    Roster,
    /// The keyframes stop before the match did.
    Keyframes,
    /// The shots stop before the match did.
    Shots,
    /// A page of the tape could not be read (`on_chunk`'s non-UTF-8 arm).
    Page,
}

impl ReplayCut {
    /// The document's word: `roster`, `keyframes`, `shots` or `page`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ReplayCut::Roster => "roster",
            ReplayCut::Keyframes => "keyframes",
            ReplayCut::Shots => "shots",
            ReplayCut::Page => "page",
        }
    }

    /// The word read back, or `None`.
    #[must_use]
    pub fn parse(word: &str) -> Option<ReplayCut> {
        match word {
            "roster" => Some(ReplayCut::Roster),
            "keyframes" => Some(ReplayCut::Keyframes),
            "shots" => Some(ReplayCut::Shots),
            "page" => Some(ReplayCut::Page),
            _ => None,
        }
    }
}

/// **One shot** (16b cut I.1): when it was fired, by and at which roster
/// entries, and what became of it. No weapon, no range and no draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayShot {
    /// The `fired` line's arena millisecond.
    pub t_ms: i64,
    /// The shooter's roster index.
    pub shooter: u16,
    /// The target's roster index.
    pub target: u16,
    /// What became of it.
    pub outcome: ShotOutcome,
}

/// **One finished match, as a replay** (16b cut I.1) — what
/// [`ReplayFold::finish`] answers, what [`Replay::to_document`] writes and what
/// the ring holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replay {
    /// The match.
    pub match_id: MatchId,
    /// The arena definition's id.
    pub arena: SmolStr,
    /// **The definition's digest** — the record's own
    /// `arena_definition_digest`, which is how the ground a replay is drawn on
    /// is told to be the ground it was fought on (`ground_for`).
    pub arena_digest: String,
    /// The side of one cell, centimetres.
    pub cell_cm: u16,
    /// The window's columns.
    pub width: u16,
    /// The window's rows.
    pub height: u16,
    /// The keyframe cadence the arena ran at, arena milliseconds.
    pub summary_ms: u32,
    /// The arena millisecond the match ended at (the end line's), or the last
    /// keyframe's when the tape carried none.
    pub end_ms: i64,
    /// The result's outcome.
    pub outcome: MatchOutcome,
    /// The result's reason word (`elimination`, `abort`, …).
    pub reason: SmolStr,
    /// **The rule the match was fought under** (16b cut O.1; plan item 39),
    /// written as the document's `"mode"` beside `outcome` and `reason`, so a
    /// hold is never read back as a plain fight. Set by
    /// [`crate::match_::finish`] off the record once the fold is finished;
    /// [`MatchMode::Skirmish`] until then, and for a document written before
    /// the cut, which has no key.
    pub mode: MatchMode,
    /// Every unit, in unit-id order.
    pub roster: Vec<ReplayRoster>,
    /// The keyframes, in arena-time order; keyframe 0 is the opening.
    pub keyframes: Vec<Keyframe>,
    /// The shots, in firing order.
    pub shots: Vec<ReplayShot>,
    /// The wrecks, as `(t_ms, roster index)`, in tape order; at most one a unit.
    pub destroyed: Vec<(i64, u16)>,
    /// Whether anything was left out: a roster past [`REPLAY_MAX_UNITS`], a
    /// keyframe or shot past a cap or the byte budget, or a lost chunk.
    /// Exactly `!cut.is_empty()` on every replay the fold writes.
    pub truncated: bool,
    /// **Why** (16b cut I.1): each cause once, in [`ReplayCut`]'s order — what
    /// the reply's truncation lines are worded from, and what tells the last
    /// keyframe of a whole replay (whose shots and wrecks run to `end_ms`) from
    /// the last keyframe of a sealed one (whose do not).
    pub cut: Vec<ReplayCut>,
}

/// **The fold from the tape to one [`Replay`]** (16b cut I.1), held on the
/// match's record while it runs.
///
/// `PartialEq` and not `Eq`: it lives on [`crate::match_::MatchRecord`], which
/// derives `Clone, Debug, PartialEq`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayFold {
    /// Object → roster index, for the units the roster kept.
    index: BTreeMap<ObjId, u16>,
    /// Shots fired and not yet resolved, by `shot_seq`, as an index into
    /// `replay.shots`. Bounded by [`REPLAY_MAX_SHOTS`].
    pending: BTreeMap<u64, usize>,
    /// The running encoded size of the keyframes and shots taken, bytes: each
    /// one's JSON array plus one comma.
    encoded_bytes: usize,
    /// The end line's arena millisecond, once one has been folded.
    ended_ms: Option<i64>,
    /// The replay as it grows.
    replay: Replay,
}

/// The encoded size one document array would add: its JSON and one comma.
fn encoded_len(value: &serde_json::Value) -> usize {
    serde_json::to_string(value).map_or(usize::MAX, |text| text.len().saturating_add(1))
}

/// A cell from a position, or `None` off the window.
fn cell_from(x_cm: i64, y_cm: i64, cell_cm: u16, width: u16, height: u16) -> Option<(u16, u16)> {
    let cell = i64::from(cell_cm.max(1));
    let x = u16::try_from(x_cm.div_euclid(cell)).ok()?;
    let y = u16::try_from(y_cm.div_euclid(cell)).ok()?;
    (x < width && y < height).then_some((x, y))
}

/// Whole degrees from millidegrees, `0..360`.
pub(crate) fn whole_degrees(heading_mdeg: u32) -> u16 {
    u16::try_from((heading_mdeg / 1_000) % 360).unwrap_or(0)
}

impl ReplayFold {
    /// **A fold for one match, seeded at check-out** (16b cut I.1).
    ///
    /// `roster` is every unit with the designator the arena resolved for it and
    /// its side; it is taken in **unit-id order**, sorted and deduplicated here
    /// defensively, and a unit past [`REPLAY_MAX_UNITS`] is left off with
    /// `truncated` set. `opening` is the check-out poses: keyframe 0 at
    /// `t_ms 0`, one slot per roster unit, `None` for a unit whose pose is off
    /// the window (or not given).
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        match_id: MatchId,
        arena: &SmolStr,
        arena_digest: &str,
        cell_cm: u16,
        width: u16,
        height: u16,
        summary_ms: u32,
        roster: &[(ObjId, Designator, PartyId)],
        opening: &[(ObjId, mush_sim::data::Pose)],
    ) -> ReplayFold {
        let mut sorted: Vec<(ObjId, Designator, PartyId)> = roster.to_vec();
        sorted.sort_by_key(|(unit, _, _)| *unit);
        sorted.dedup_by_key(|(unit, _, _)| *unit);
        let cut = if sorted.len() > REPLAY_MAX_UNITS {
            vec![ReplayCut::Roster]
        } else {
            Vec::new()
        };
        sorted.truncate(REPLAY_MAX_UNITS);
        let mut index: BTreeMap<ObjId, u16> = BTreeMap::new();
        let mut names: Vec<ReplayRoster> = Vec::with_capacity(sorted.len());
        for (at, (unit, designator, team)) in sorted.into_iter().enumerate() {
            let Ok(at) = u16::try_from(at) else {
                break;
            };
            index.insert(unit, at);
            names.push(ReplayRoster {
                callsign: SmolStr::new(designator.to_string()),
                team,
            });
        }
        let mut fold = ReplayFold {
            index,
            pending: BTreeMap::new(),
            encoded_bytes: 0,
            ended_ms: None,
            replay: Replay {
                match_id,
                arena: arena.clone(),
                arena_digest: arena_digest.to_owned(),
                cell_cm,
                width,
                height,
                summary_ms,
                end_ms: 0,
                // Placeholders: `finish` writes the result's own.
                outcome: MatchOutcome::Void,
                reason: SmolStr::default(),
                // 16b cut O.1: `crate::match_::finish` writes the record's.
                mode: MatchMode::Skirmish,
                roster: names,
                keyframes: Vec::new(),
                shots: Vec::new(),
                destroyed: Vec::new(),
                truncated: !cut.is_empty(),
                cut,
            },
        };
        let opening: Vec<(ObjId, i64, i64, u32)> = opening
            .iter()
            .map(|(unit, pose)| (*unit, pose.x_cm, pose.y_cm, pose.heading_mdeg))
            .collect();
        fold.take_keyframe(0, &opening);
        fold
    }

    /// **Fold one tape line** (16b cut I.1). Only the objective record is read
    /// — positions, shots, their outcomes, wrecks and the end — and nothing
    /// per-viewer ever: no `detected`, `classified`, `lost` or `locked` line and
    /// no draw.
    pub fn absorb(&mut self, event: &LedgerEvent) {
        match &event.kind {
            EventKind::PositionSummary { units } => {
                let units: Vec<(ObjId, i64, i64, u32)> = units
                    .iter()
                    .map(|(unit, x_cm, y_cm, _z_cm, heading)| (*unit, *x_cm, *y_cm, *heading))
                    .collect();
                self.take_keyframe(event.t_ms, &units);
            }
            EventKind::Fired {
                unit,
                target,
                shot_seq,
                ..
            } => {
                // A sealed picture takes no shot past its last keyframe, and
                // sealed shots take none at all (see [`ReplayCut`]).
                if self.sealed(ReplayCut::Keyframes) || self.sealed(ReplayCut::Shots) {
                    return;
                }
                let (Some(shooter), Some(target)) = (
                    self.index.get(unit).copied(),
                    self.index.get(target).copied(),
                ) else {
                    return;
                };
                let shot = ReplayShot {
                    t_ms: event.t_ms,
                    shooter,
                    target,
                    outcome: ShotOutcome::Flying,
                };
                let encoded = shot_json(&shot);
                if self.replay.shots.len() >= REPLAY_MAX_SHOTS || !self.within_budget(&encoded) {
                    self.mark(ReplayCut::Shots);
                    return;
                }
                self.encoded_bytes = self.encoded_bytes.saturating_add(encoded_len(&encoded));
                self.pending.insert(*shot_seq, self.replay.shots.len());
                self.replay.shots.push(shot);
            }
            // An outcome still resolves a shot taken before a seal: a shot fired
            // before the last keyframe kept and landing after it is a hit or a
            // miss, not a shot in flight.
            EventKind::Hit { shot_seq, .. } => self.resolve(*shot_seq, ShotOutcome::Hit),
            EventKind::Miss { shot_seq, .. } => self.resolve(*shot_seq, ShotOutcome::Miss),
            EventKind::UnitDestroyed { unit, .. } => {
                if !self.sealed(ReplayCut::Keyframes)
                    && let Some(at) = self.index.get(unit).copied()
                    && !self.replay.destroyed.iter().any(|(_, held)| *held == at)
                {
                    self.replay.destroyed.push((event.t_ms, at));
                }
            }
            EventKind::MatchEnded { .. } | EventKind::Aborted | EventKind::Void => {
                self.ended_ms = Some(event.t_ms);
            }
            _ => {}
        }
    }

    /// **A page of the tape is gone** (16b cut I.1): the replay says it is not
    /// whole, and says it as a lost page ([`ReplayCut::Page`]) — a hole, not a
    /// seal: the pages after it are folded as they come.
    pub fn lost_chunk(&mut self) {
        self.mark(ReplayCut::Page);
    }

    /// Record one cause of a cut, once, in [`ReplayCut`]'s order.
    fn mark(&mut self, cut: ReplayCut) {
        if let Err(at) = self.replay.cut.binary_search(&cut) {
            self.replay.cut.insert(at, cut);
        }
        self.replay.truncated = true;
    }

    /// Whether a cause has been recorded.
    fn sealed(&self, cut: ReplayCut) -> bool {
        self.replay.cut.contains(&cut)
    }

    /// **Seal the picture at its last keyframe** (16b cut I.1): the first
    /// keyframe refused. Every shot and wreck already taken past the last
    /// keyframe kept is dropped (with its pending outcome), so the replay is
    /// exactly the prefix `The replay holds the first …` names.
    fn seal_keyframes(&mut self) {
        self.mark(ReplayCut::Keyframes);
        let held = self
            .replay
            .keyframes
            .last()
            .map_or(0, |keyframe| keyframe.t_ms);
        self.replay.shots.retain(|shot| shot.t_ms <= held);
        let kept = self.replay.shots.len();
        self.pending.retain(|_, at| *at < kept);
        self.replay.destroyed.retain(|(when, _)| *when <= held);
    }

    /// **The replay** (16b cut I.1), under the result's own outcome and reason
    /// word. A shot still in flight stays `flying`; `end_ms` is the end line's,
    /// or the last keyframe's when the tape carried none.
    #[must_use]
    pub fn finish(self, outcome: MatchOutcome, reason: &str) -> Replay {
        let mut replay = self.replay;
        replay.outcome = outcome;
        replay.reason = SmolStr::new(reason);
        replay.end_ms = self
            .ended_ms
            .unwrap_or_else(|| replay.keyframes.last().map_or(0, |keyframe| keyframe.t_ms));
        replay
    }

    /// Whether one more document array fits the byte budget.
    fn within_budget(&self, next: &serde_json::Value) -> bool {
        REPLAY_HEAD_BYTES
            .saturating_add(self.encoded_bytes)
            .saturating_add(encoded_len(next))
            <= REPLAY_MAX_BYTES
    }

    /// One keyframe over the roster from `(unit, x_cm, y_cm, heading_mdeg)`
    /// rows; a unit not on the roster is ignored, a unit off the window is
    /// `None`. The first refusal (the cap or the budget) seals the picture
    /// ([`ReplayFold::seal_keyframes`]); every keyframe after it is refused
    /// too, however small, so a replay never has a gap in the middle.
    fn take_keyframe(&mut self, t_ms: i64, rows: &[(ObjId, i64, i64, u32)]) {
        if self.sealed(ReplayCut::Keyframes) {
            return;
        }
        let mut units: Vec<Option<KeyframeUnit>> = vec![None; self.replay.roster.len()];
        for (unit, x_cm, y_cm, heading_mdeg) in rows {
            let Some(at) = self.index.get(unit) else {
                continue;
            };
            let cell = cell_from(
                *x_cm,
                *y_cm,
                self.replay.cell_cm,
                self.replay.width,
                self.replay.height,
            );
            if let (Some(slot), Some((x, y))) = (units.get_mut(usize::from(*at)), cell) {
                *slot = Some(KeyframeUnit {
                    x,
                    y,
                    heading_deg: whole_degrees(*heading_mdeg),
                });
            }
        }
        let keyframe = Keyframe { t_ms, units };
        let encoded = keyframe_json(&keyframe);
        if self.replay.keyframes.len() >= REPLAY_MAX_KEYFRAMES || !self.within_budget(&encoded) {
            self.seal_keyframes();
            return;
        }
        self.encoded_bytes = self.encoded_bytes.saturating_add(encoded_len(&encoded));
        self.replay.keyframes.push(keyframe);
    }

    /// A pending shot's outcome, once. No pending shot, or a second outcome:
    /// ignored.
    fn resolve(&mut self, shot_seq: u64, outcome: ShotOutcome) {
        if let Some(at) = self.pending.remove(&shot_seq)
            && let Some(shot) = self.replay.shots.get_mut(at)
        {
            shot.outcome = outcome;
        }
    }

    /// The running encoded size of the keyframes and shots taken, bytes (for
    /// the size pins).
    #[must_use]
    pub fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }
}

// ---------------------------------------------------------------------------
// The document
// ---------------------------------------------------------------------------

/// One keyframe as the document writes it: `[t_ms, [[x, y, heading], null, …]]`.
fn keyframe_json(keyframe: &Keyframe) -> serde_json::Value {
    let units: Vec<serde_json::Value> = keyframe
        .units
        .iter()
        .map(|slot| match slot {
            Some(unit) => serde_json::json!([unit.x, unit.y, unit.heading_deg]),
            None => serde_json::Value::Null,
        })
        .collect();
    serde_json::json!([keyframe.t_ms, units])
}

/// One shot as the document writes it: `[t_ms, shooter, target, "hit"]`.
fn shot_json(shot: &ReplayShot) -> serde_json::Value {
    serde_json::json!([shot.t_ms, shot.shooter, shot.target, shot.outcome.as_str()])
}

/// **A replay document's key inside the tacsim namespace** (16b cut I.1):
/// `tacsim:replay:<match>`.
#[must_use]
pub fn replay_key(match_id: MatchId) -> SmolStr {
    SmolStr::new(format!("tacsim:replay:{}", match_id.0))
}

/// The match id a replay key names, or `None` for any other key.
#[must_use]
pub fn replay_id_of_key(key: &str) -> Option<MatchId> {
    key.strip_prefix("tacsim:replay:")
        .filter(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
        .and_then(|rest| rest.parse::<u64>().ok())
        .map(MatchId)
}

impl Replay {
    /// **The store document, and the `arena.replay` answer's body** (16b cut
    /// I.1):
    ///
    /// ```text
    /// { "record": "tacsim.replay", "record_version": 1, "match": 12,
    ///   "arena": "tacsim.arena.proving_ground", "arena_digest": "…",
    ///   "cell_cm": 1000, "width": 256, "height": 256, "summary_ms": 5000,
    ///   "end_ms": 272000, "outcome": "red", "reason": "elimination", "mode": "skirmish",
    ///   "roster": [{ "callsign": "RA", "team": "red" }, …],
    ///   "keyframes": [[0, [[6, 97, 90], [249, 97, 270]]], [5000, [[8, 97, 90], null]]],
    ///   "shots": [[35250, 0, 1, "hit"], …], "destroyed": [[41000, 1]],
    ///   "truncated": false, "cut": [] }
    /// ```
    ///
    /// `cut` names each cause of a `truncated` replay once, in [`ReplayCut`]'s
    /// order (`roster`, `keyframes`, `shots`, `page`).
    ///
    /// **16b cut O.1** added `"mode"` (plan item 39): a key added, not a
    /// version, so `record_version` stays 1 and a reader of an older document
    /// reads a skirmish.
    #[must_use]
    pub fn to_document(&self) -> serde_json::Value {
        let roster: Vec<serde_json::Value> = self
            .roster
            .iter()
            .map(|unit| serde_json::json!({ "callsign": unit.callsign.as_str(), "team": unit.team.as_str() }))
            .collect();
        let keyframes: Vec<serde_json::Value> = self.keyframes.iter().map(keyframe_json).collect();
        let shots: Vec<serde_json::Value> = self.shots.iter().map(shot_json).collect();
        let destroyed: Vec<serde_json::Value> = self
            .destroyed
            .iter()
            .map(|(t_ms, unit)| serde_json::json!([t_ms, unit]))
            .collect();
        serde_json::json!({
            "record": REPLAY_RECORD,
            "record_version": 1,
            "match": self.match_id.0,
            "arena": self.arena.as_str(),
            "arena_digest": self.arena_digest,
            "cell_cm": self.cell_cm,
            "width": self.width,
            "height": self.height,
            "summary_ms": self.summary_ms,
            "end_ms": self.end_ms,
            "outcome": self.outcome.as_str(),
            "reason": self.reason.as_str(),
            "mode": self.mode.as_str(),
            "roster": roster,
            "keyframes": keyframes,
            "shots": shots,
            "destroyed": destroyed,
            "truncated": self.truncated,
            "cut": self.cut.iter().map(ReplayCut::as_str).collect::<Vec<_>>(),
        })
    }

    /// **Read one replay document** (16b cut I.1): `None` unless its `record`
    /// is `tacsim.replay` and `match` is a number.
    ///
    /// Total over everything else: an absent or unreadable number reads `0`,
    /// word empty, outcome `void`; a roster entry's unreadable side reads red
    /// (it is kept, because the rows are positional); a keyframe row that is not
    /// three small numbers reads `None`, and each keyframe is padded or cut to
    /// the roster's length; a malformed keyframe, shot or wreck — or one naming
    /// an index past the roster — is skipped; an unknown `cut` word is
    /// skipped, and a `truncated` document naming no cause reads as cut at its
    /// last keyframe.
    #[must_use]
    pub fn from_document(value: &serde_json::Value) -> Option<Replay> {
        use serde_json::Value;
        if value.get("record").and_then(Value::as_str) != Some(REPLAY_RECORD) {
            return None;
        }
        let match_id = MatchId(value.get("match").and_then(Value::as_u64)?);
        let word =
            |key: &str| SmolStr::new(value.get(key).and_then(Value::as_str).unwrap_or_default());
        let small = |value: Option<&Value>| {
            value
                .and_then(Value::as_u64)
                .and_then(|n| u16::try_from(n).ok())
        };
        let empty = Vec::new();
        let array = |key: &str| value.get(key).and_then(Value::as_array).unwrap_or(&empty);
        let roster: Vec<ReplayRoster> = array("roster")
            .iter()
            .map(|entry| ReplayRoster {
                callsign: SmolStr::new(
                    entry
                        .get("callsign")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ),
                team: entry
                    .get("team")
                    .cloned()
                    .and_then(|team| serde_json::from_value::<PartyId>(team).ok())
                    .unwrap_or(PartyId::RED),
            })
            .collect();
        let size = roster.len();
        let on_roster = |at: u16| usize::from(at) < size;
        let keyframes: Vec<Keyframe> = array("keyframes")
            .iter()
            .filter_map(|row| {
                let row = row.as_array()?;
                let t_ms = row.first()?.as_i64()?;
                let mut units: Vec<Option<KeyframeUnit>> = row
                    .get(1)?
                    .as_array()?
                    .iter()
                    .map(|slot| {
                        let slot = slot.as_array()?;
                        Some(KeyframeUnit {
                            x: small(slot.first())?,
                            y: small(slot.get(1))?,
                            heading_deg: small(slot.get(2))?,
                        })
                    })
                    .collect();
                units.resize(size, None);
                Some(Keyframe { t_ms, units })
            })
            .collect();
        let shots: Vec<ReplayShot> = array("shots")
            .iter()
            .filter_map(|row| {
                let row = row.as_array()?;
                let shot = ReplayShot {
                    t_ms: row.first()?.as_i64()?,
                    shooter: small(row.get(1))?,
                    target: small(row.get(2))?,
                    outcome: ShotOutcome::parse(row.get(3)?.as_str()?)?,
                };
                (on_roster(shot.shooter) && on_roster(shot.target)).then_some(shot)
            })
            .collect();
        let destroyed: Vec<(i64, u16)> = array("destroyed")
            .iter()
            .filter_map(|row| {
                let row = row.as_array()?;
                let at = small(row.get(1))?;
                on_roster(at).then_some((row.first()?.as_i64()?, at))
            })
            .collect();
        // The causes, each once and in order; a `truncated` document that
        // names none is read as sealed at its last keyframe — the reading that
        // claims the least.
        let mut cut: Vec<ReplayCut> = array("cut")
            .iter()
            .filter_map(|word| word.as_str().and_then(ReplayCut::parse))
            .collect();
        cut.sort();
        cut.dedup();
        let truncated = value
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || !cut.is_empty();
        if truncated && cut.is_empty() {
            cut.push(ReplayCut::Keyframes);
        }
        Some(Replay {
            match_id,
            arena: word("arena"),
            arena_digest: word("arena_digest").to_string(),
            cell_cm: small(value.get("cell_cm")).unwrap_or(0),
            width: small(value.get("width")).unwrap_or(0),
            height: small(value.get("height")).unwrap_or(0),
            summary_ms: value
                .get("summary_ms")
                .and_then(Value::as_u64)
                .and_then(|ms| u32::try_from(ms).ok())
                .unwrap_or(0),
            end_ms: value.get("end_ms").and_then(Value::as_i64).unwrap_or(0),
            outcome: value
                .get("outcome")
                .cloned()
                .and_then(|outcome| serde_json::from_value::<MatchOutcome>(outcome).ok())
                .unwrap_or(MatchOutcome::Void),
            reason: word("reason"),
            // 16b cut O.1: absent or unknown reads a skirmish — every replay
            // written before the cut was one.
            mode: value
                .get("mode")
                .and_then(Value::as_str)
                .and_then(MatchMode::parse)
                .unwrap_or_default(),
            roster,
            keyframes,
            shots,
            destroyed,
            truncated,
            cut,
        })
    }

    /// The roster index a callsign names, case-insensitively.
    #[must_use]
    pub fn roster_index(&self, callsign: &str) -> Option<u16> {
        self.roster
            .iter()
            .position(|unit| unit.callsign.eq_ignore_ascii_case(callsign))
            .and_then(|at| u16::try_from(at).ok())
    }

    /// Roster entry `at`'s callsign, or the empty string.
    fn callsign(&self, at: u16) -> &str {
        self.roster
            .get(usize::from(at))
            .map_or("", |unit| unit.callsign.as_str())
    }

    /// **The arena millisecond keyframe `index` answers for** (16b cut I.1):
    /// the shots and wrecks its reply lists run to it, and a unit is drawn a
    /// wreck at it when its wreck was recorded at or before it.
    ///
    /// The keyframe's own `t_ms` — except **the last keyframe of a replay
    /// whose keyframes were not cut**, which answers for everything to
    /// `end_ms`. The arena writes a position summary only on its cadence and
    /// none at the end, so the killing shot and the wreck of an elimination
    /// almost always fall after the last keyframe, and that keyframe is the
    /// one place they can be shown. A replay sealed at a keyframe
    /// ([`ReplayCut::Keyframes`]) holds nothing past it, and its last keyframe
    /// answers for itself alone.
    #[must_use]
    pub fn horizon(&self, index: usize) -> i64 {
        let t_ms = self
            .keyframes
            .get(index)
            .map_or(0, |keyframe| keyframe.t_ms);
        let last = index.saturating_add(1) == self.keyframes.len();
        if last && !self.cut.contains(&ReplayCut::Keyframes) {
            t_ms.max(self.end_ms)
        } else {
            t_ms
        }
    }

    /// Whether roster entry `at` was a wreck by arena millisecond `t_ms`.
    fn wrecked_by(&self, at: u16, t_ms: i64) -> bool {
        self.destroyed
            .iter()
            .any(|(when, held)| *held == at && *when <= t_ms)
    }
}

// ---------------------------------------------------------------------------
// The window and the reply's lines (pure)
// ---------------------------------------------------------------------------

/// Where one keyframe's window sits (see [`window_of`]).
struct WindowPlace {
    /// The window's corner, arena cells.
    origin: (u16, u16),
    /// Its size, cells.
    size: (u16, u16),
    /// The cell it is centred on, which the off-window distances are read from.
    from: (u16, u16),
    /// Whether that cell is the focus unit's (else the arena's centre).
    focused: bool,
}

/// The window's corner and size for one keyframe: [`REPLAY_WINDOW`] clamped to
/// the arena, centred on the focus unit's cell at this keyframe with
/// `view_window`'s own clamp, or on the arena's centre when the focus is not in
/// it.
fn window_of(replay: &Replay, index: usize, focus: u16, terrain: &ArenaTerrain) -> WindowPlace {
    let width = REPLAY_WINDOW.0.min(terrain.width());
    let height = REPLAY_WINDOW.1.min(terrain.height());
    let placed = focus_cell(replay, index, focus);
    let (cx, cy) = placed.unwrap_or((terrain.width() / 2, terrain.height() / 2));
    let x0 = cx
        .saturating_sub(width / 2)
        .min(terrain.width().saturating_sub(width));
    let y0 = cy
        .saturating_sub(height / 2)
        .min(terrain.height().saturating_sub(height));
    WindowPlace {
        origin: (x0, y0),
        size: (width, height),
        from: (cx, cy),
        focused: placed.is_some(),
    }
}

/// The focus unit's cell at keyframe `index`, if it is in it.
fn focus_cell(replay: &Replay, index: usize, focus: u16) -> Option<(u16, u16)> {
    replay
        .keyframes
        .get(index)
        .and_then(|keyframe| keyframe.units.get(usize::from(focus)))
        .copied()
        .flatten()
        .map(|unit| (unit.x, unit.y))
}

/// A cell inside the window, relative to its corner.
fn in_window(cell: (u16, u16), origin: (u16, u16), size: (u16, u16)) -> Option<(u16, u16)> {
    let (x, y) = cell;
    let (x0, y0) = origin;
    if x < x0 || y < y0 {
        return None;
    }
    let (dx, dy) = (x.saturating_sub(x0), y.saturating_sub(y0));
    (dx < size.0 && dy < size.1).then_some((dx, dy))
}

/// **One keyframe's window** (16b cut I.1): `VIEW`'s grid shape, drawn from
/// the replay and the ground alone — pure, so the fixture set
/// (`web/src/lib/fixtures/replay/replay_window.json`, blessed by
/// `crates/tacsim/tests/replay.rs`) is exactly what the live reply pushes.
///
/// The keys: `schema { id: "arena.replay", version: 1, shape: "grid", retain:
/// "latest" }` — the **`retain` marker is required** (16b cut B.5: stepping must
/// never push the scene out of a reconnect's window, which is why `VIEW`'s grid
/// carries it too); `match`; `title` (`Replay of match 12 at 1:30`); `surface`
/// (the arena id); `width` and `height` ([`REPLAY_WINDOW`] clamped to the
/// arena); `cell_cm`; `origin` (centimetres) and `origin_cell`; `extent` (the
/// arena's size in cells); `cell_columns` 2; `rows` (row 0 the south edge);
/// `legend`; `units`; `you: null`; `keyframe { index, of, t_ms }` (`index`
/// 0-based, `of` the count).
///
/// **The markers, in drawing order**: the named spots inside the window in
/// authored order (`spot: true`, [`spot_markers`]), then the keyframe's units
/// in roster order as `{ "id": "BA", "name": "blue", "cell": [dx, dy], "glyph":
/// "*", "you": false }` — `name` the side's word and glyph [`RED_GLYPH`] or
/// [`BLUE_GLYPH`], or `wreck` and [`WRECK_GLYPH`] for a unit whose wreck was
/// recorded at or before this keyframe's [`Replay::horizon`] (its own `t_ms`,
/// or `end_ms` on the last keyframe of a whole replay, so the final window
/// shows how the match ended); a unit not in the keyframe or outside
/// the window is not drawn; a later marker overwrites an earlier one on the
/// same cell, the rule every compositor follows.
///
/// **The legend**: the terrain classes present, then `["o", "red"]`, `["*",
/// "blue"]` and `["%", "wreck"]`, each only when a marker of it was placed,
/// then each spot that **owns its cell** — that no later marker landed on —
/// exactly `view_window`'s owner rule.
///
/// Nothing here names a unit by anything but its callsign: no object number,
/// no pilot and no contact id, and `you` is `null` because a replay is nobody's
/// platform.
#[must_use]
pub fn replay_window(
    replay: &Replay,
    index: usize,
    focus: u16,
    terrain: &ArenaTerrain,
    spots: &[KeySpot],
) -> serde_json::Value {
    let WindowPlace { origin, size, .. } = window_of(replay, index, focus, terrain);
    let (x0, y0) = origin;
    let (rows, classes) = terrain_rows(terrain, origin, size);
    let mut legend: Vec<serde_json::Value> = classes
        .iter()
        .map(|class| serde_json::json!([class.glyph().to_string(), class.label()]))
        .collect();
    let (spot_units, spot_legend, _drawn) = spot_markers(spots, origin, size);
    let spot_count = spot_units.len();
    let mut units = spot_units;
    let keyframe = replay.keyframes.get(index);
    let t_ms = keyframe.map_or(0, |keyframe| keyframe.t_ms);
    // A wreck is drawn from the horizon: the last keyframe of a whole replay
    // shows the wrecks the match ended with ([`Replay::horizon`]).
    let horizon = replay.horizon(index);
    let (mut red, mut blue, mut wreck) = (false, false, false);
    if let Some(keyframe) = keyframe {
        for (at, slot) in keyframe.units.iter().enumerate() {
            let (Some(unit), Ok(at)) = (slot, u16::try_from(at)) else {
                continue;
            };
            let Some((dx, dy)) = in_window((unit.x, unit.y), origin, size) else {
                continue;
            };
            let Some(entry) = replay.roster.get(usize::from(at)) else {
                continue;
            };
            let (name, glyph) = if replay.wrecked_by(at, horizon) {
                wreck = true;
                ("wreck", WRECK_GLYPH)
            } else {
                match entry.team {
                    PartyId::RED => {
                        red = true;
                        ("red", RED_GLYPH)
                    }
                    // Doc 32 cut CF.0: the centre fields red and blue; any
                    // other party is drawn as the blue glyph is.
                    _ => {
                        blue = true;
                        ("blue", BLUE_GLYPH)
                    }
                }
            };
            units.push(serde_json::json!({
                "id": entry.callsign.as_str(),
                "name": name,
                "cell": [u32::from(dx), u32::from(dy)],
                "glyph": glyph,
                "you": false,
            }));
        }
    }
    for (drawn, glyph, label) in [
        (red, RED_GLYPH, "red"),
        (blue, BLUE_GLYPH, "blue"),
        (wreck, WRECK_GLYPH, "wreck"),
    ] {
        if drawn {
            legend.push(serde_json::json!([glyph, label]));
        }
    }
    // The spots are named last, and only those that own their cell — the owner
    // rule `view_window` applies, applied here once every marker is placed.
    for (at, entry) in spot_legend.into_iter().enumerate().take(spot_count) {
        let cell = units.get(at).and_then(|unit| unit.get("cell"));
        let covered = units
            .iter()
            .skip(at.saturating_add(1))
            .any(|later| later.get("cell") == cell);
        if !covered {
            legend.push(entry);
        }
    }
    serde_json::json!({
        "schema": {
            "id": REPLAY_SCHEMA,
            "version": REPLAY_VERSION,
            "shape": "grid",
            "retain": RETAIN_LATEST,
        },
        "match": replay.match_id.0,
        "title": format!("Replay of match {} at {}", replay.match_id.0, lobby::clock_text(t_ms)),
        "surface": replay.arena.as_str(),
        "width": size.0,
        "height": size.1,
        "cell_cm": terrain.cell_cm(),
        "origin": [
            i64::from(x0) * i64::from(terrain.cell_cm()),
            i64::from(y0) * i64::from(terrain.cell_cm()),
        ],
        "origin_cell": [x0, y0],
        "extent": [terrain.width(), terrain.height()],
        "cell_columns": VIEW_CELL_COLUMNS,
        "rows": rows,
        "legend": legend,
        "units": units,
        "you": serde_json::Value::Null,
        "keyframe": {
            "index": index,
            "of": replay.keyframes.len(),
            "t_ms": t_ms,
        },
    })
}

/// The keyframe cadence in words: `5 s`, or `100 ms` for a cadence that is not
/// whole seconds (a test configuration's).
fn every_words(summary_ms: u32) -> String {
    if summary_ms.is_multiple_of(1_000) {
        format!("{} s", summary_ms / 1_000)
    } else {
        format!("{summary_ms} ms")
    }
}

/// A capped list: the first [`REPLAY_LIST_CAP`] entries joined by `; `, then
/// `, and N more` for the rest.
fn capped(entries: &[String]) -> String {
    let shown: Vec<&str> = entries
        .iter()
        .take(REPLAY_LIST_CAP)
        .map(String::as_str)
        .collect();
    let mut text = shown.join("; ");
    let more = entries.len().saturating_sub(REPLAY_LIST_CAP);
    if more > 0 {
        text.push_str(&format!(", and {more} more"));
    }
    text
}

/// The centre of a cell, arena centimetres.
fn cell_centre_cm(cell: (u16, u16), cell_cm: u16) -> (i64, i64) {
    let side = i64::from(cell_cm);
    (
        i64::from(cell.0)
            .saturating_mul(side)
            .saturating_add(side / 2),
        i64::from(cell.1)
            .saturating_mul(side)
            .saturating_add(side / 2),
    )
}

/// **The reply `SIM REPLAY` says around its window** (16b cut I.1), pure: the
/// header, then the lines that follow the panel, in order.
///
/// ```text
/// Replay of match 12 at 1:30, keyframe 19 of 55 (4 m 32 s, red won (blue eliminated)): both sides as the arena recorded them every 5 s, around RA.
/// Off the window: BB 1810 m at 088° from RA.
/// Shots since 1:25: RA at BA, hit; BA at RA, miss.
/// Wrecked since 1:25: BB.
/// The replay holds the first 4 m 30 s of match 12; the tape has the rest.
/// Next: SIM REPLAY 12 next
/// ```
///
/// The header's keyframe is counted from 1 (`keyframe 19 of 55` is index 18).
/// `Off the window:` names every unit of the keyframe the window did not draw,
/// in roster order, with its range from the focus (metres, rounded to ten) and
/// its bearing (three digits) — from the arena's centre when the focus is not in
/// the keyframe — and a wreck as `BB (wreck)`; it and `Shots since` are capped
/// at [`REPLAY_LIST_CAP`] entries with `, and N more`. The shots and the wrecks
/// are those after the previous keyframe and at or before this one's
/// [`Replay::horizon`] (a shot still `flying` reads `in flight`): on the last
/// keyframe of a whole replay that is the end, and both lines say so —
/// `Shots since 4:25, to the end at 4:32: …`. Each list line is left out when
/// it would name nothing; the truncation lines are one a [`ReplayCut`] (none
/// when the replay is whole): `The replay holds the first 56 m 20 s of match
/// 12; the tape has the rest.` (keyframes), `The replay keeps the shots of
/// match 12 to 3:12; the tape has the rest.` (shots), `A page of the tape of
/// match 12 could not be read; the replay is missing what it held.` (a lost
/// page) and `The replay names the first 24 units of match 12; the tape has
/// the rest.` (the roster); and `Next:` is left out at the last keyframe.
#[must_use]
pub fn replay_lines(
    replay: &Replay,
    index: usize,
    focus: u16,
    terrain: &ArenaTerrain,
) -> (String, Vec<String>) {
    let id = replay.match_id.0;
    let keyframe = replay.keyframes.get(index);
    let t_ms = keyframe.map_or(0, |keyframe| keyframe.t_ms);
    let horizon = replay.horizon(index);
    let header = format!(
        "Replay of match {id} at {}, keyframe {} of {} ({}, {}{}): both sides as the arena \
         recorded them every {}, around {}.",
        lobby::clock_text(t_ms),
        index.saturating_add(1),
        replay.keyframes.len(),
        duration_words(replay.end_ms),
        outcome_words(replay.outcome, &replay.reason),
        // 16b cut O.1 (plan item 39): ` [hold]` for a match fought under another
        // rule, as the ring's line and both client heads carry it.
        crate::match_::record::mode_tag(Some(replay.mode)),
        every_words(replay.summary_ms),
        replay.callsign(focus),
    );
    let mut lines: Vec<String> = Vec::new();
    let WindowPlace {
        origin,
        size,
        from,
        focused,
    } = window_of(replay, index, focus, terrain);
    // (3) Off the window.
    if let Some(keyframe) = keyframe {
        let (fx, fy) = cell_centre_cm(from, terrain.cell_cm());
        let entries: Vec<String> = keyframe
            .units
            .iter()
            .enumerate()
            .filter_map(|(at, slot)| {
                let unit = (*slot)?;
                let at = u16::try_from(at).ok()?;
                if in_window((unit.x, unit.y), origin, size).is_some() {
                    return None;
                }
                let (ux, uy) = cell_centre_cm((unit.x, unit.y), terrain.cell_cm());
                let range = crate::arena::index::range_between(fx, fy, 0, ux, uy, 0);
                let bearing = crate::arena::perception::bearing_of(
                    ux.saturating_sub(fx),
                    uy.saturating_sub(fy),
                ) / 1_000;
                let wreck = if replay.wrecked_by(at, horizon) {
                    " (wreck)"
                } else {
                    ""
                };
                Some(format!(
                    "{}{wreck} {} m at {bearing:03}°",
                    replay.callsign(at),
                    range_m_rounded(range, SPOT_RANGE_STEP_M)
                ))
            })
            .collect();
        if !entries.is_empty() {
            let from_words = if focused {
                replay.callsign(focus).to_owned()
            } else {
                "the arena's centre".to_owned()
            };
            lines.push(format!(
                "Off the window: {} from {from_words}.",
                capped(&entries)
            ));
        }
    }
    // (4) and (5): what happened since the previous keyframe.
    let since = index
        .checked_sub(1)
        .and_then(|before| replay.keyframes.get(before))
        .map(|keyframe| keyframe.t_ms);
    // To the horizon: the last keyframe of a whole replay runs to the end, and
    // says so ([`Replay::horizon`]).
    let after = |at: i64| since.is_none_or(|since| at > since) && at <= horizon;
    let since_words = if horizon > t_ms {
        format!(
            "{}, to the end at {}",
            lobby::clock_text(since.unwrap_or(0)),
            lobby::clock_text(horizon)
        )
    } else {
        lobby::clock_text(since.unwrap_or(0))
    };
    let shots: Vec<String> = replay
        .shots
        .iter()
        .filter(|shot| after(shot.t_ms))
        .map(|shot| {
            format!(
                "{} at {}, {}",
                replay.callsign(shot.shooter),
                replay.callsign(shot.target),
                shot.outcome.words()
            )
        })
        .collect();
    if !shots.is_empty() {
        lines.push(format!("Shots since {since_words}: {}.", capped(&shots)));
    }
    let wrecked: Vec<&str> = replay
        .destroyed
        .iter()
        .filter(|(when, _)| after(*when))
        .map(|(_, at)| replay.callsign(*at))
        .collect();
    if !wrecked.is_empty() {
        lines.push(format!(
            "Wrecked since {since_words}: {}.",
            wrecked.join(", ")
        ));
    }
    // (6) The truncation, one line a cause ([`ReplayCut`]), each true of what
    // was cut: a sealed picture is the prefix its line names; sealed shots stop
    // at the last one kept while the keyframes run on; a lost page is a hole
    // the tape does not have either.
    for cut in &replay.cut {
        lines.push(match cut {
            ReplayCut::Roster => format!(
                "The replay names the first {} units of match {id}; the tape has the rest.",
                replay.roster.len()
            ),
            ReplayCut::Keyframes => format!(
                "The replay holds the first {} of match {id}; the tape has the rest.",
                duration_words(replay.keyframes.last().map_or(0, |keyframe| keyframe.t_ms))
            ),
            ReplayCut::Shots => format!(
                "The replay keeps the shots of match {id} to {}; the tape has the rest.",
                lobby::clock_text(replay.shots.last().map_or(0, |shot| shot.t_ms))
            ),
            ReplayCut::Page => format!(
                "A page of the tape of match {id} could not be read; the replay is missing \
                 what it held."
            ),
        });
    }
    // (7) The step.
    if index.saturating_add(1) < replay.keyframes.len() {
        lines.push(format!("Next: SIM REPLAY {id} next"));
    }
    (header, lines)
}

// ---------------------------------------------------------------------------
// The request's grammar (pure)
// ---------------------------------------------------------------------------

/// **Which keyframe a `SIM REPLAY` asks for** (16b cut I.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayStep {
    /// The first keyframe at or after this arena millisecond, else the last.
    At(i64),
    /// One on from the actor's cursor on the match.
    Next,
    /// One back from it.
    Prev,
    /// Keyframe 0.
    First,
    /// The last keyframe.
    Last,
}

/// **One parsed `SIM REPLAY`** (16b cut I.1): the match, at most one step and
/// at most one callsign word, as typed (resolved against the roster later).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayAsk {
    /// The match.
    pub match_id: MatchId,
    /// The step, if one was named.
    pub step: Option<ReplayStep>,
    /// The callsign word, if one was named.
    pub callsign: Option<String>,
}

/// `#12` or `12` as a match id.
fn match_word(word: &str) -> Option<MatchId> {
    let digits = word.strip_prefix('#').unwrap_or(word);
    (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        .then(|| digits.parse::<u64>().ok())
        .flatten()
        .map(MatchId)
}

/// A run of one or two digits (`m`, `mm`, `h`), or `None`.
fn short_number(word: &str) -> Option<i64> {
    (!word.is_empty() && word.len() <= 2 && word.chars().all(|c| c.is_ascii_digit()))
        .then(|| word.parse::<i64>().ok())
        .flatten()
}

/// Exactly two digits under sixty (`ss`, and `mm` after an hour), or `None`.
fn sexagesimal(word: &str) -> Option<i64> {
    (word.len() == 2)
        .then(|| short_number(word))
        .flatten()
        .filter(|n| *n < 60)
}

/// `m:ss`, `mm:ss` or `h:mm:ss` as arena milliseconds, or `None`.
fn time_word(word: &str) -> Option<i64> {
    let parts: Vec<&str> = word.split(':').collect();
    let seconds = match parts.as_slice() {
        [minutes, seconds] => short_number(minutes)?
            .saturating_mul(60)
            .saturating_add(sexagesimal(seconds)?),
        [hours, minutes, seconds] => short_number(hours)?
            .saturating_mul(3_600)
            .saturating_add(sexagesimal(minutes)?.saturating_mul(60))
            .saturating_add(sexagesimal(seconds)?),
        _ => return None,
    };
    Some(seconds.saturating_mul(1_000))
}

/// **Parse a `SIM REPLAY` line** (16b cut I.1): the match word (`#12` or `12`,
/// required), then in any order at most one step word (`m:ss`, `mm:ss`,
/// `h:mm:ss`, `next`, `prev`, `first`, `last`) and at most one callsign word
/// (letters only). Anything else is `None`, which the caller answers with
/// [`REPLAY_USAGE`].
#[must_use]
pub fn parse_ask(word: &str) -> Option<ReplayAsk> {
    let mut words = word.split_whitespace();
    let match_id = match_word(words.next()?)?;
    let (mut step, mut callsign) = (None, None);
    for word in words {
        let lower = word.to_ascii_lowercase();
        let named = match lower.as_str() {
            "next" => Some(ReplayStep::Next),
            "prev" => Some(ReplayStep::Prev),
            "first" => Some(ReplayStep::First),
            "last" => Some(ReplayStep::Last),
            _ => time_word(word).map(ReplayStep::At),
        };
        match named {
            Some(named) if step.is_none() => step = Some(named),
            Some(_) => return None,
            None if callsign.is_none() && word.chars().all(|c| c.is_ascii_alphabetic()) => {
                callsign = Some(word.to_ascii_uppercase());
            }
            None => return None,
        }
    }
    Some(ReplayAsk {
        match_id,
        step,
        callsign,
    })
}

// ---------------------------------------------------------------------------
// The sentences
// ---------------------------------------------------------------------------

/// `Match 12 is still running.` — the one answer an unfinished match gives.
fn still_running(id: MatchId) -> String {
    format!("Match {} is still running.", id.0)
}

/// The ring does not hold it.
fn not_remembered(id: MatchId) -> String {
    format!(
        "The centre remembers its last {HISTORY_DEPTH} matches; match {} is not one of them.",
        id.0
    )
}

/// A restart aborted it.
fn restarted(id: MatchId) -> String {
    format!(
        "The server restarted during match {}; there is no replay of it.",
        id.0
    )
}

/// Its result was written before this cut, or no replay is held for it.
fn before_replays(id: MatchId) -> String {
    format!(
        "Match {} was recorded before replays existed; there is nothing to watch of it.",
        id.0
    )
}

/// The ground loaded now is not the ground it was fought on.
fn foreign_ground(id: MatchId) -> String {
    format!(
        "The ground of match {} is not the ground loaded on this server.",
        id.0
    )
}

// ---------------------------------------------------------------------------
// The cursor, the ground and the request (world-side)
// ---------------------------------------------------------------------------

/// **One actor's replay cursor** (16b cut I.1): the finished match they are
/// watching, the keyframe they are at and the roster entry the window is
/// centred on.
///
/// RAM on [`MatchBook`], never persisted — and **the thing the `arena.replay`
/// and `arena.map` queries read for this actor**, because a panel query carries
/// no parameters. Written after a successful `SIM REPLAY`; dropped by every
/// refused one but the two end-of-tape refusals, by the ring forgetting the
/// match ([`MatchBook::forget_replay`]), and (16b cut I.4) by a watch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplayCursor {
    /// The match.
    pub match_id: MatchId,
    /// The keyframe, 0-based.
    pub index: usize,
    /// The roster entry the window is centred on.
    pub focus: u16,
}

/// **The ground of a remembered replay, built after a boot if it must be**
/// (16b cut I.1).
///
/// A read, then a write: the replay's `arena` and `arena_digest` are read off
/// the book and the borrow released; a ground the book already holds under that
/// digest is the answer; a digest already known not to match is `None`;
/// otherwise [`crate::match_::definition_for`] builds the arena **once** —
/// every terrain cell and J.2's horizon index, on the world task — and the
/// ground is kept only when its own digest equals the replay's (inserted under
/// it), else the digest is remembered in the mismatch set and every later call
/// answers `None` without building. Called by [`request`], which has the world
/// mutably; the queries only read.
pub(crate) fn ground_for(ctx: &mut Ctx<'_>, match_id: MatchId) -> Option<Arc<ArenaDefinition>> {
    let (arena, digest) = {
        let book = ctx.w.module_state::<MatchBook>()?;
        let replay = book.replays.get(&match_id)?;
        if let Some(ground) = book.grounds.get(&replay.arena_digest) {
            return Some(Arc::clone(ground));
        }
        if book.ground_mismatch.contains(&replay.arena_digest) {
            return None;
        }
        (replay.arena.clone(), replay.arena_digest.clone())
    };
    let built = crate::match_::definition_for(ctx.w, &arena)
        .ok()
        .flatten()
        .filter(|definition| crate::arena_bridge::digest_text(&definition.digest()) == digest);
    let book = ctx.w.module_state_mut::<MatchBook>()?;
    match built {
        Some(definition) => {
            let ground = Arc::new(definition);
            book.grounds.insert(digest, Arc::clone(&ground));
            Some(ground)
        }
        None => {
            tracing::warn!(
                match_id = match_id.0,
                arena = %arena,
                "tacsim: a remembered replay's ground is not the ground loaded on this server; \
                 it cannot be drawn"
            );
            book.ground_mismatch.insert(digest);
            None
        }
    }
}

/// Which end of the tape a `next` or `prev` ran into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TapeEnd {
    /// `prev` at keyframe 0.
    First,
    /// `next` at the last keyframe.
    Last,
}

/// **The keyframe a step lands on** (16b cut I.1), pure: no step or `first`
/// is keyframe 0; `last` the last; a time the first keyframe whose `t_ms` is
/// at or after it, else the last; `next` and `prev` one on or back from
/// `cursor` (the actor's cursor index on THIS match, clamped to the tape; none
/// counts as 0), refused at the ends with the end they ran into.
fn step_index(
    step: Option<ReplayStep>,
    cursor: Option<usize>,
    keyframes: &[Keyframe],
) -> Result<usize, TapeEnd> {
    let last = keyframes.len().saturating_sub(1);
    let at = cursor.map_or(0, |index| index.min(last));
    Ok(match step {
        None | Some(ReplayStep::First) => 0,
        Some(ReplayStep::Last) => last,
        Some(ReplayStep::At(asked)) => keyframes
            .iter()
            .position(|keyframe| keyframe.t_ms >= asked)
            .unwrap_or(last),
        Some(ReplayStep::Next) if at >= last => return Err(TapeEnd::Last),
        Some(ReplayStep::Next) => at.saturating_add(1),
        Some(ReplayStep::Prev) if at == 0 => return Err(TapeEnd::First),
        Some(ReplayStep::Prev) => at.saturating_sub(1),
    })
}

/// A refusal of [`request`], and whether it leaves the actor's cursor alone.
struct Refused {
    text: String,
    keeps_cursor: bool,
}

impl Refused {
    /// A refusal that drops the cursor — every one but the end of the tape.
    fn dropping(text: impl Into<String>) -> Refused {
        Refused {
            text: text.into(),
            keeps_cursor: false,
        }
    }

    /// An end-of-tape refusal, which leaves the cursor where it was.
    fn keeping(text: impl Into<String>) -> Refused {
        Refused {
            text: text.into(),
            keeps_cursor: true,
        }
    }
}

/// **`SIM REPLAY <match> [<m:ss> | next | prev] [<callsign>]`** (16b cut I.1;
/// 16c N9, Q37): one keyframe of a finished match the centre remembers, both
/// sides, in line mode.
///
/// Synchronous and RAM only: the replay and its ground are on the book. The
/// centre gate is `sim_command`'s; there is **no pilot gate** (Q37: public).
/// Refusals, in this order: the usage line ([`REPLAY_USAGE`]); (16b cut I.4)
/// `You are watching match 3; @arena/watch off first.`; `Match 12 is
/// still running.` (a record on the book in any state that is not an end — the
/// one answer an unfinished match gives, checked before the ring); `The centre
/// remembers its last 20 matches; match 12 is not one of them.`; `The server
/// restarted during match 3; there is no replay of it.`; `Match 3 was recorded
/// before replays existed; there is nothing to watch of it.`; `The ground of
/// match 12 is not the ground loaded on this server.`; `No unit of match 12 flew
/// as ZZ.`; and at the ends of the tape `That is the last keyframe of match
/// 12.` / `That is the first keyframe of match 12.`.
///
/// **Every refusal but the two end-of-tape ones drops the actor's cursor**: a
/// viewer whose ask was refused has no replay open, and the queries must not
/// answer an earlier match's document by accident.
///
/// # Errors
///
/// Every refusal above, as the line the player sees.
pub fn request(ctx: &mut Ctx<'_>, word: &str) -> Result<(), CmdError> {
    let actor = ctx.actor;
    match answer(ctx, word) {
        Ok(()) => Ok(()),
        Err(refused) => {
            if !refused.keeps_cursor
                && let Some(book) = ctx.w.module_state_mut::<MatchBook>()
            {
                book.replay_cursors.remove(&actor);
            }
            Err(CmdError::msg(refused.text))
        }
    }
}

/// [`request`]'s body: every check, then the reply and the cursor.
fn answer(ctx: &mut Ctx<'_>, word: &str) -> Result<(), Refused> {
    let actor = ctx.actor;
    let ask = parse_ask(word).ok_or_else(|| Refused::dropping(REPLAY_USAGE))?;
    let id = ask.match_id;
    // **16b cut I.4**: a watcher is refused, first after the usage line: one
    // picture at a time, and the watch is the one they asked for last.
    if let Some(watched) = crate::arena_bridge::watching(ctx.w, actor) {
        return Err(Refused::dropping(format!(
            "You are watching match {}; @arena/watch off first.",
            watched.0
        )));
    }
    let (replay, cursor, own_callsign) = {
        let book = ctx
            .w
            .module_state::<MatchBook>()
            .ok_or_else(|| Refused::dropping(not_remembered(id)))?;
        if book
            .get(id)
            .is_some_and(|record| !record.state.is_terminal())
        {
            return Err(Refused::dropping(still_running(id)));
        }
        let row = book
            .remembered(id)
            .ok_or_else(|| Refused::dropping(not_remembered(id)))?;
        if row.reason == "restart" {
            return Err(Refused::dropping(restarted(id)));
        }
        let replay = book
            .replays
            .get(&id)
            .filter(|_| row.replay)
            .cloned()
            .ok_or_else(|| Refused::dropping(before_replays(id)))?;
        let own = row
            .pilot_unit(actor)
            .map(|flown| flown.callsign.clone())
            .filter(|callsign| !callsign.is_empty());
        let cursor = book
            .replay_cursors
            .get(&actor)
            .copied()
            .filter(|cursor| cursor.match_id == id);
        (replay, cursor, own)
    };
    let ground = ground_for(ctx, id).ok_or_else(|| Refused::dropping(foreign_ground(id)))?;
    let named = match ask.callsign.as_deref() {
        Some(word) => Some(replay.roster_index(word).ok_or_else(|| {
            Refused::dropping(format!("No unit of match {} flew as {word}.", id.0))
        })?),
        None => None,
    };
    let index = step_index(
        ask.step,
        cursor.map(|cursor| cursor.index),
        &replay.keyframes,
    )
    .map_err(|end| {
        Refused::keeping(match end {
            TapeEnd::Last => format!("That is the last keyframe of match {}.", id.0),
            TapeEnd::First => format!("That is the first keyframe of match {}.", id.0),
        })
    })?;
    let focus = named
        .or_else(|| cursor.map(|cursor| cursor.focus))
        .or_else(|| own_callsign.and_then(|own| replay.roster_index(&own)))
        .unwrap_or(0);
    let window = replay_window(&replay, index, focus, &ground.terrain, &ground.key_spots);
    let (header, lines) = replay_lines(&replay, index, focus, &ground.terrain);
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, header);
    // The grid, pushed exactly as `VIEW` pushes its own.
    ctx.out.push(
        Audience::Actor(actor),
        OutputEvent::Panel {
            feed: SmolStr::new_static(crate::arena_bridge::REPLAY_FEED),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data: window,
        },
    );
    for line in lines {
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    }
    if let Some(book) = ctx.w.module_state_mut::<MatchBook>() {
        book.replay_cursors.insert(
            actor,
            ReplayCursor {
                match_id: id,
                index,
                focus,
            },
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The queries' reads (world-side, `&World` only)
// ---------------------------------------------------------------------------

/// The `arena.replay` query's schema object.
fn document_schema() -> serde_json::Value {
    serde_json::json!({
        "id": REPLAY_SCHEMA,
        "version": REPLAY_VERSION,
        "shape": REPLAY_DOCUMENT_SHAPE,
    })
}

/// `{ schema, available: false, reason }`.
fn unavailable(reason: String) -> serde_json::Value {
    serde_json::json!({
        "schema": document_schema(),
        "available": false,
        "reason": reason,
    })
}

/// **The `arena.replay` query's answer for `actor`** (16b cut I.1).
///
/// The query takes **no parameters** — a panel subscribe carries a feed and
/// nothing else — so it answers the match the actor's [`ReplayCursor`] names:
/// for an actor `lobby::outside_centre` admits whose cursor names a match the
/// ring remembers with a replay, `{ "schema": { "id": "arena.replay",
/// "version": 1, "shape": "replay" }, ..Replay::to_document() }`; otherwise `{
/// "schema": .., "available": false, "reason": "<sentence>" }` — outside the
/// centre the centre's own sentence; no cursor [`NO_REPLAY_OPEN`] (what a
/// refused `SIM REPLAY` leaves behind); a cursor on a match the book holds in a
/// state that is not an end `Match 12 is still running.` (**defensive and
/// unreachable** while match ids are monotonic and a refusal drops the cursor:
/// a cursor is only ever written for a finished match); a forgotten match the
/// remembers sentence; a restart the restart sentence; no replay the
/// before-replays sentence.
#[must_use]
pub fn replay_answer(w: &World, actor: ObjId) -> serde_json::Value {
    if let Err(refusal) = lobby::outside_centre(w, actor) {
        return unavailable(refusal.to_string());
    }
    let Some(book) = w.module_state::<MatchBook>() else {
        return unavailable(NO_REPLAY_OPEN.to_owned());
    };
    let Some(cursor) = book.replay_cursors.get(&actor) else {
        return unavailable(NO_REPLAY_OPEN.to_owned());
    };
    let id = cursor.match_id;
    if book
        .get(id)
        .is_some_and(|record| !record.state.is_terminal())
    {
        return unavailable(still_running(id));
    }
    let Some(row) = book.remembered(id) else {
        return unavailable(not_remembered(id));
    };
    if row.reason == "restart" {
        return unavailable(restarted(id));
    }
    let Some(replay) = book.replays.get(&id).filter(|_| row.replay) else {
        return unavailable(before_replays(id));
    };
    let mut answer = replay.to_document();
    if let Some(object) = answer.as_object_mut() {
        object.insert("schema".to_owned(), document_schema());
    }
    answer
}

/// **The finished match and its held ground an actor's cursor names** (16b cut
/// I.1), for `arena.map`'s third clause: the cursor's match must be one the
/// ring remembers with a replay AND its ground must be on the book. A read: a
/// ground not yet built after a boot is built by [`request`], never here.
#[must_use]
pub fn cursor_ground(w: &World, actor: ObjId) -> Option<(MatchId, Arc<ArenaDefinition>)> {
    let book = w.module_state::<MatchBook>()?;
    let cursor = book.replay_cursors.get(&actor)?;
    let id = cursor.match_id;
    let row = book.remembered(id)?;
    let replay = book.replays.get(&id).filter(|_| row.replay)?;
    let ground = book.ground(&replay.arena_digest)?;
    Some((id, ground))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ArenaConfig, MatchConfig};

    const RA: ObjId = ObjId(101);
    const RB: ObjId = ObjId(102);
    const BA: ObjId = ObjId(201);
    const BB: ObjId = ObjId(202);

    fn slot(team: PartyId, slot: u16) -> Designator {
        Designator::Slot { team, slot }
    }

    /// A pose at a cell's centre on 1,000 cm cells.
    fn pose(x: i64, y: i64, heading_mdeg: u32) -> mush_sim::data::Pose {
        let mut pose = mush_sim::data::Pose::at(
            x * 1_000 + 500,
            y * 1_000 + 500,
            0,
            crate::arena::test_support::rules(),
        );
        pose.heading_mdeg = heading_mdeg;
        pose
    }

    /// Four units on a 96 by 64 yard, the roster given out of order on purpose.
    fn yard_fold() -> ReplayFold {
        ReplayFold::new(
            MatchId(12),
            &SmolStr::new("fixture_yard"),
            "ab",
            1_000,
            96,
            64,
            5_000,
            &[
                (BB, slot(PartyId::BLUE, 2), PartyId::BLUE),
                (RA, slot(PartyId::RED, 1), PartyId::RED),
                (BA, slot(PartyId::BLUE, 1), PartyId::BLUE),
                (RB, slot(PartyId::RED, 2), PartyId::RED),
                (RA, slot(PartyId::RED, 1), PartyId::RED),
            ],
            &[
                (RA, pose(6, 4, 90_000)),
                (RB, pose(2, 9, 90_500)),
                (BA, pose(36, 14, 270_000)),
                // Off the window: `None` in keyframe 0.
                (BB, pose(200, 7, 270_000)),
            ],
        )
    }

    fn event(seq: u64, t_ms: i64, kind: EventKind) -> LedgerEvent {
        LedgerEvent {
            seq,
            t_ms,
            kind,
            draws: Vec::new(),
            hash: 0,
        }
    }

    fn summary(t_ms: i64, units: &[(ObjId, i64, i64)]) -> LedgerEvent {
        event(
            1,
            t_ms,
            EventKind::PositionSummary {
                units: units
                    .iter()
                    .map(|(unit, x, y)| (*unit, x * 1_000 + 500, y * 1_000 + 500, 0, 45_000))
                    .collect(),
            },
        )
    }

    fn fired(t_ms: i64, unit: ObjId, target: ObjId, shot_seq: u64) -> LedgerEvent {
        event(
            2,
            t_ms,
            EventKind::Fired {
                unit,
                weapon: SmolStr::new("cannon"),
                shot_seq,
                target,
                range_cm: 1_000,
                rounds: 1,
                by: None,
                slot: None,
                unlawful: false,
            },
        )
    }

    fn miss(t_ms: i64, shot_seq: u64) -> LedgerEvent {
        event(
            3,
            t_ms,
            EventKind::Miss {
                shot_seq,
                reason: SmolStr::new("dispersion"),
            },
        )
    }

    fn hit(t_ms: i64, shot_seq: u64, unit: ObjId, target: ObjId) -> LedgerEvent {
        event(
            4,
            t_ms,
            EventKind::Hit {
                shot_seq,
                unit,
                target,
                section: SmolStr::new("hull"),
                facet: SmolStr::new("front"),
                armor_section: SmolStr::new("hull"),
                rounds: 1,
                damage: 10,
                armor_left: 5,
                internals_left: 5,
                consequences: Vec::new(),
                cause: SmolStr::new("arena:hit"),
            },
        )
    }

    /// **The fold over a hand-written tape**: keyframe 0 from the opening in
    /// unit-id order (a duplicate roster row dropped, an off-window opening
    /// `None`); one keyframe a summary (a unit off the roster ignored, one off
    /// the window `None`); a shot's three outcomes; a `hit` with no `fired`
    /// ignored and a second outcome ignored; a wreck; the end; nothing
    /// per-viewer read.
    #[test]
    fn i1_the_fold_over_a_hand_written_tape() {
        let mut fold = yard_fold();
        for line in [
            summary(
                5_000,
                &[
                    (RA, 8, 4),
                    (RB, 2, 9),
                    (BA, 34, 14),
                    (BB, 14, 7),
                    (ObjId(999), 1, 1),
                ],
            ),
            fired(6_000, RA, BA, 1),
            hit(6_400, 1, RA, BA),
            // A second outcome for the same shot: ignored.
            miss(6_500, 1),
            // A `hit` with no `fired`: ignored.
            hit(6_600, 77, BA, RA),
            // A shot at a unit off the roster: ignored.
            fired(6_700, RA, ObjId(999), 2),
            event(
                5,
                7_000,
                EventKind::UnitDestroyed {
                    unit: BB,
                    by: Some(RA),
                },
            ),
            // A second wreck of the same unit: ignored.
            event(6, 7_100, EventKind::UnitDestroyed { unit: BB, by: None }),
            fired(8_000, BA, RA, 3),
            miss(8_200, 3),
            fired(9_000, RB, BB, 4),
            // Per-viewer lines: nothing is read of them.
            event(
                7,
                9_100,
                EventKind::Detected {
                    viewer: RA,
                    contact: crate::arena::state::ContactId(0x1234_5678_9abc_def0),
                    target: BA,
                },
            ),
            event(8, 9_200, EventKind::Lagged { dt_ms: 10 }),
            summary(
                10_000,
                &[(RA, 10, 4), (RB, 2, 9), (BA, 32, 14), (BB, 14, 7)],
            ),
            event(9, 10_500, EventKind::Aborted),
        ] {
            fold.absorb(&line);
        }
        let replay = fold.finish(MatchOutcome::Abort, "abort");
        assert_eq!(
            replay
                .roster
                .iter()
                .map(|unit| unit.callsign.as_str())
                .collect::<Vec<_>>(),
            vec!["RA", "RB", "BA", "BB"],
            "unit-id order, deduplicated"
        );
        assert_eq!(replay.keyframes.len(), 3);
        assert_eq!(
            replay
                .keyframes
                .first()
                .map(|keyframe| keyframe.units.clone()),
            Some(vec![
                Some(KeyframeUnit {
                    x: 6,
                    y: 4,
                    heading_deg: 90
                }),
                Some(KeyframeUnit {
                    x: 2,
                    y: 9,
                    heading_deg: 90
                }),
                Some(KeyframeUnit {
                    x: 36,
                    y: 14,
                    heading_deg: 270
                }),
                None,
            ])
        );
        assert_eq!(
            replay
                .keyframes
                .get(1)
                .map(|keyframe| (keyframe.t_ms, keyframe.units.clone())),
            Some((
                5_000,
                vec![
                    Some(KeyframeUnit {
                        x: 8,
                        y: 4,
                        heading_deg: 45
                    }),
                    Some(KeyframeUnit {
                        x: 2,
                        y: 9,
                        heading_deg: 45
                    }),
                    Some(KeyframeUnit {
                        x: 34,
                        y: 14,
                        heading_deg: 45
                    }),
                    Some(KeyframeUnit {
                        x: 14,
                        y: 7,
                        heading_deg: 45
                    }),
                ]
            ))
        );
        assert_eq!(
            replay.shots,
            vec![
                ReplayShot {
                    t_ms: 6_000,
                    shooter: 0,
                    target: 2,
                    outcome: ShotOutcome::Hit
                },
                ReplayShot {
                    t_ms: 8_000,
                    shooter: 2,
                    target: 0,
                    outcome: ShotOutcome::Miss
                },
                ReplayShot {
                    t_ms: 9_000,
                    shooter: 1,
                    target: 3,
                    outcome: ShotOutcome::Flying
                },
            ]
        );
        assert_eq!(replay.destroyed, vec![(7_000, 3)]);
        assert_eq!(replay.end_ms, 10_500, "the end line's");
        assert_eq!(
            (replay.outcome, replay.reason.as_str()),
            (MatchOutcome::Abort, "abort")
        );
        assert!(!replay.truncated);

        // No end line: the last keyframe's time.
        let mut open = yard_fold();
        open.absorb(&summary(5_000, &[(RA, 8, 4)]));
        assert_eq!(open.finish(MatchOutcome::Void, "void").end_ms, 5_000);

        // A lost chunk: truncated.
        let mut lost = yard_fold();
        lost.lost_chunk();
        lost.absorb(&summary(5_000, &[(RA, 8, 4)]));
        let lost = lost.finish(MatchOutcome::Void, "void");
        assert!(lost.truncated);
        assert_eq!(lost.cut, vec![ReplayCut::Page]);
        assert_eq!(lost.keyframes.len(), 2, "a lost page is a hole, not a seal");
    }

    /// **The caps and the budget**: a roster past the unit cap keeps its first
    /// units; keyframes past the keyframe cap and shots past the shot cap are
    /// refused with `truncated`; and the byte budget refuses the next keyframe
    /// when it would pass the bound.
    #[test]
    fn i1_each_cap_and_the_byte_budget_set_truncated() {
        // Units.
        let many: Vec<(ObjId, Designator, PartyId)> = (1..=30_u16)
            .map(|n| (ObjId(u32::from(n)), slot(PartyId::RED, n), PartyId::RED))
            .collect();
        let wide = ReplayFold::new(
            MatchId(1),
            &SmolStr::new("a"),
            "d",
            1_000,
            256,
            256,
            5_000,
            &many,
            &[],
        )
        .finish(MatchOutcome::Void, "void");
        assert_eq!(wide.roster.len(), REPLAY_MAX_UNITS);
        assert!(wide.truncated);
        assert_eq!(wide.cut, vec![ReplayCut::Roster]);

        // Keyframes.
        let mut long = yard_fold();
        for n in 1..=REPLAY_MAX_KEYFRAMES as i64 {
            long.absorb(&summary(n * 5_000, &[(RA, 8, 4)]));
        }
        let long = long.finish(MatchOutcome::Void, "void");
        assert_eq!(long.keyframes.len(), REPLAY_MAX_KEYFRAMES);
        assert!(long.truncated);
        assert_eq!(long.cut, vec![ReplayCut::Keyframes]);

        // Shots: sealed at the cap, while the keyframes go on.
        let mut busy = yard_fold();
        for n in 0..=REPLAY_MAX_SHOTS as u64 {
            busy.absorb(&fired(1_000, RA, BA, n));
        }
        busy.absorb(&summary(5_000, &[(RA, 8, 4)]));
        let busy = busy.finish(MatchOutcome::Void, "void");
        assert_eq!(busy.shots.len(), REPLAY_MAX_SHOTS);
        assert!(busy.truncated);
        assert_eq!(busy.cut, vec![ReplayCut::Shots]);
        assert_eq!(
            busy.keyframes.len(),
            2,
            "a shot cut does not stop the keyframes"
        );

        // The budget: a fold whose running size is at the bound takes nothing
        // more, keyframe or shot.
        let mut full = yard_fold();
        full.encoded_bytes = REPLAY_MAX_BYTES - REPLAY_HEAD_BYTES - 10;
        full.absorb(&summary(
            5_000,
            &[(RA, 8, 4), (RB, 2, 9), (BA, 34, 14), (BB, 14, 7)],
        ));
        full.absorb(&fired(6_000, RA, BA, 1));
        let full = full.finish(MatchOutcome::Void, "void");
        assert_eq!(full.keyframes.len(), 1, "only keyframe 0");
        assert!(full.shots.is_empty());
        assert!(full.truncated);
        assert_eq!(full.cut, vec![ReplayCut::Keyframes]);
    }

    /// **A cut is a seal, never a hole** (16b cut I.1, the fixer's R2): the
    /// first keyframe refused by the budget drops the shots and wrecks already
    /// taken past the last keyframe kept, refuses every later keyframe — a
    /// SMALLER one that would fit included — and every later shot and wreck,
    /// while an outcome still resolves a shot taken before the seal. And the
    /// first shot refused refuses every later one, with room to spare, while
    /// the keyframes run on.
    #[test]
    fn i1_a_cut_seals_the_picture_and_never_leaves_a_hole() {
        let mut fold = yard_fold();
        fold.absorb(&summary(5_000, &[(RA, 8, 4)]));
        // Fired before keyframe 1, landing after the seal.
        fold.absorb(&fired(4_900, RA, BA, 1));
        // Fired past keyframe 1 and before the refused keyframe: dropped by
        // the seal, as is its wreck.
        fold.absorb(&fired(6_000, RA, BA, 2));
        fold.absorb(&event(
            5,
            6_500,
            EventKind::UnitDestroyed { unit: BB, by: None },
        ));
        // A keyframe of four three-digit cells refused by the budget...
        let room = REPLAY_MAX_BYTES - REPLAY_HEAD_BYTES - fold.encoded_bytes;
        let wide = keyframe_json(&Keyframe {
            t_ms: 10_000,
            units: vec![
                Some(KeyframeUnit {
                    x: 88,
                    y: 44,
                    heading_deg: 45,
                });
                4
            ],
        });
        fold.encoded_bytes += room - encoded_len(&wide) + 1;
        fold.absorb(&summary(
            10_000,
            &[(RA, 88, 44), (RB, 88, 44), (BA, 88, 44), (BB, 88, 44)],
        ));
        // ... then one that would fit (one unit, one-digit cells): refused too.
        fold.absorb(&summary(15_000, &[(RA, 1, 1)]));
        fold.absorb(&hit(15_100, 1, RA, BA));
        fold.absorb(&fired(15_200, RA, BA, 3));
        fold.absorb(&event(
            6,
            15_300,
            EventKind::UnitDestroyed { unit: BA, by: None },
        ));
        let replay = fold.finish(MatchOutcome::Void, "void");
        assert_eq!(replay.cut, vec![ReplayCut::Keyframes]);
        assert_eq!(
            replay
                .keyframes
                .iter()
                .map(|keyframe| keyframe.t_ms)
                .collect::<Vec<_>>(),
            vec![0, 5_000],
            "no keyframe after the seal, however small"
        );
        assert_eq!(
            replay.shots,
            vec![ReplayShot {
                t_ms: 4_900,
                shooter: 0,
                target: 2,
                outcome: ShotOutcome::Hit,
            }],
            "only the shot at or before the last keyframe, its outcome resolved"
        );
        assert!(replay.destroyed.is_empty(), "no wreck past the seal");
        let held = replay.keyframes.last().map_or(0, |keyframe| keyframe.t_ms);
        assert!(replay.shots.iter().all(|shot| shot.t_ms <= held));

        // Shots: the first refused seals them, a smaller later one included.
        let mut shots = yard_fold();
        shots.encoded_bytes = REPLAY_MAX_BYTES
            - REPLAY_HEAD_BYTES
            - encoded_len(&shot_json(&ReplayShot {
                t_ms: 99_999,
                shooter: 0,
                target: 2,
                outcome: ShotOutcome::Flying,
            }))
            + 1;
        shots.absorb(&fired(99_999, RA, BA, 1));
        // All the room in the world after the refusal: still sealed.
        shots.encoded_bytes = 0;
        shots.absorb(&fired(100_000, RA, BA, 2));
        shots.absorb(&summary(105_000, &[(RA, 8, 4)]));
        let shots = shots.finish(MatchOutcome::Void, "void");
        assert_eq!(shots.cut, vec![ReplayCut::Shots]);
        assert!(shots.shots.is_empty());
        assert_eq!(shots.keyframes.len(), 2, "the keyframes run on");
    }

    /// **The document round-trips**, and a document that is not a replay reads
    /// `None`; the key and its reader.
    #[test]
    fn i1_the_document_round_trips() {
        let mut fold = yard_fold();
        fold.absorb(&summary(5_000, &[(RA, 8, 4), (BB, 14, 7)]));
        fold.absorb(&fired(6_000, RA, BA, 1));
        fold.absorb(&event(
            5,
            7_000,
            EventKind::UnitDestroyed { unit: BB, by: None },
        ));
        let replay = fold.finish(MatchOutcome::Red, "elimination");
        let document = replay.to_document();
        assert_eq!(document["record"], serde_json::json!("tacsim.replay"));
        assert_eq!(document["record_version"], serde_json::json!(1));
        assert_eq!(document["outcome"], serde_json::json!("red"));
        assert_eq!(
            document["roster"][0],
            serde_json::json!({ "callsign": "RA", "team": "red" })
        );
        assert_eq!(
            document["keyframes"][1],
            serde_json::json!([5_000, [[8, 4, 45], null, null, [14, 7, 45]]])
        );
        assert_eq!(
            document["shots"][0],
            serde_json::json!([6_000, 0, 2, "flying"])
        );
        assert_eq!(document["destroyed"], serde_json::json!([[7_000, 3]]));
        assert_eq!(document["cut"], serde_json::json!([]));
        assert_eq!(Replay::from_document(&document), Some(replay.clone()));
        // A cut replay round-trips its causes; a `truncated` document naming
        // none reads as sealed at its last keyframe, and an unknown word is
        // skipped.
        let mut cut = replay.clone();
        cut.truncated = true;
        cut.cut = vec![ReplayCut::Shots, ReplayCut::Page];
        let cut_document = cut.to_document();
        assert_eq!(cut_document["cut"], serde_json::json!(["shots", "page"]));
        assert_eq!(Replay::from_document(&cut_document), Some(cut));
        let mut bare = document.clone();
        bare["truncated"] = serde_json::json!(true);
        bare["cut"] = serde_json::json!(["later"]);
        assert_eq!(
            Replay::from_document(&bare).map(|read| (read.truncated, read.cut)),
            Some((true, vec![ReplayCut::Keyframes]))
        );
        assert_eq!(
            Replay::from_document(&serde_json::json!({ "record": "tacsim.match", "match": 1 })),
            None
        );
        assert_eq!(
            Replay::from_document(&serde_json::json!({ "record": REPLAY_RECORD })),
            None
        );
        assert_eq!(replay_key(MatchId(12)).as_str(), "tacsim:replay:12");
        assert_eq!(replay_id_of_key("tacsim:replay:12"), Some(MatchId(12)));
        assert_eq!(replay_id_of_key("tacsim:replay:"), None);
        assert_eq!(replay_id_of_key("tacsim:match:12"), None);
        assert_eq!(crate::match_::match_id_of_key("tacsim:replay:12"), None);
    }

    /// One arena hour at `units` units a keyframe (three-digit cells and
    /// headings, callsigns of two letters) and `REPLAY_MAX_SHOTS` shots spread
    /// evenly over it, the last `destroyed` units wrecked: `(replay, document
    /// bytes)`.
    fn an_hour(units: u16, destroyed: u16) -> (Replay, usize) {
        let roster: Vec<(ObjId, Designator, PartyId)> = (1..=units)
            .map(|n| {
                let team = if n % 2 == 1 { PartyId::RED } else { PartyId::BLUE };
                (ObjId(u32::from(n) + 1_000), slot(team, n.div_ceil(2)), team)
            })
            .collect();
        let column = |unit: ObjId| 100 + i64::from(unit.0 % 100);
        let opening: Vec<(ObjId, mush_sim::data::Pose)> = roster
            .iter()
            .map(|(unit, _, _)| (*unit, pose(column(*unit), 150, 359_000)))
            .collect();
        let mut fold = ReplayFold::new(
            MatchId(99),
            &SmolStr::new("tacsim.arena.proving_ground"),
            &"f".repeat(64),
            1_000,
            256,
            256,
            5_000,
            &roster,
            &opening,
        );
        let mut shot = 0_usize;
        let intervals = REPLAY_MAX_KEYFRAMES - 1;
        for n in 1..=intervals {
            let t_ms = i64::try_from(n).unwrap_or(0) * 5_000;
            fold.absorb(&event(
                1,
                t_ms,
                EventKind::PositionSummary {
                    units: roster
                        .iter()
                        .map(|(unit, _, _)| {
                            (*unit, column(*unit) * 1_000 + 500, 250_500, 0, 359_000)
                        })
                        .collect(),
                },
            ));
            // Shots spread evenly: 1,024 over the hour's 735 intervals.
            let due = (n * REPLAY_MAX_SHOTS / intervals).min(REPLAY_MAX_SHOTS);
            while shot < due {
                let seq = u64::try_from(shot).unwrap_or(0);
                let (a, _, _) = roster[shot % roster.len()];
                let (b, _, _) = roster[(shot + 1) % roster.len()];
                fold.absorb(&fired(t_ms + 1_234, a, b, seq));
                fold.absorb(&if shot.is_multiple_of(2) {
                    miss(t_ms + 4_000, seq)
                } else {
                    hit(t_ms + 4_000, seq, a, b)
                });
                shot += 1;
            }
        }
        for (unit, _, _) in roster.iter().rev().take(usize::from(destroyed)) {
            fold.absorb(&event(
                1,
                3_599_999,
                EventKind::UnitDestroyed {
                    unit: *unit,
                    by: None,
                },
            ));
        }
        fold.absorb(&event(
            1,
            3_600_000,
            EventKind::MatchEnded {
                reason: crate::arena::state::EndReason::Timeout,
            },
        ));
        let replay = fold.finish(MatchOutcome::Timeout, "timeout");
        let bytes = serde_json::to_string(&replay.to_document()).map_or(0, |text| text.len());
        (replay, bytes)
    }

    /// **A lobby-shaped hour folds WHOLE and serialises under the bound** (16b
    /// cut I.1; the cut I design note, correction 2): twelve units, 736
    /// keyframes, 1,024 shots, twelve wrecks.
    #[test]
    fn i1_a_lobby_shaped_hour_folds_whole_under_the_bound() {
        let (replay, bytes) = an_hour(12, 12);
        println!(
            "i1 lobby hour: {} keyframes, {} shots, {} wrecks, {bytes} bytes of {REPLAY_MAX_BYTES}",
            replay.keyframes.len(),
            replay.shots.len(),
            replay.destroyed.len()
        );
        assert!(!replay.truncated, "the lobby hour was cut");
        assert!(replay.cut.is_empty());
        assert_eq!(replay.keyframes.len(), REPLAY_MAX_KEYFRAMES);
        assert_eq!(replay.shots.len(), REPLAY_MAX_SHOTS);
        assert_eq!(replay.destroyed.len(), 12);
        assert!(bytes <= REPLAY_MAX_BYTES, "{bytes}");
    }

    /// **A twenty-four-unit hour is cut by the budget and keeps at least fifty
    /// minutes** (16b cut I.1): `truncated`, at least 600 keyframes, and the
    /// document still under the bound.
    #[test]
    fn i1_a_twenty_four_unit_hour_is_cut_by_the_budget_and_keeps_fifty_minutes() {
        let (replay, bytes) = an_hour(24, 24);
        let kept_ms = replay.keyframes.last().map_or(0, |keyframe| keyframe.t_ms);
        println!(
            "i1 staff hour: cut at keyframe {} ({} ms, {}), {} shots, {bytes} bytes of \
             {REPLAY_MAX_BYTES}",
            replay.keyframes.len(),
            kept_ms,
            duration_words(kept_ms),
            replay.shots.len()
        );
        assert!(replay.truncated);
        assert_eq!(replay.cut, vec![ReplayCut::Keyframes]);
        assert!(
            replay.keyframes.len() >= 600,
            "kept only {} keyframes",
            replay.keyframes.len()
        );
        assert!(bytes <= REPLAY_MAX_BYTES, "{bytes}");
        // Sealed at the last keyframe kept: no shot and no wreck past it.
        assert!(
            replay.shots.iter().all(|shot| shot.t_ms <= kept_ms),
            "a shot past the last keyframe"
        );
        assert!(replay.destroyed.iter().all(|(when, _)| *when <= kept_ms));
    }

    /// **The keyframe cap covers the configured hour with two summaries of
    /// margin** (16b cut I.1).
    #[test]
    fn i1_the_keyframe_cap_covers_the_match_deadline() {
        let summary_ms = i64::from(ArenaConfig::default().ledger.summary_ms);
        let deadline = i64::from(MatchConfig::default().max_duration_ms);
        assert!(REPLAY_MAX_KEYFRAMES as i64 * summary_ms >= deadline + 2 * summary_ms);
    }

    /// A 96 by 64 yard of open ground, 1,000 cm cells.
    fn yard() -> ArenaTerrain {
        ArenaTerrain::uniform(
            96,
            64,
            1_000,
            crate::arena::state::TerrainCell {
                class: mush_atlas::TerrainClass::Grass,
                elev_cm: 0,
                move_cost: 100,
            },
        )
        .unwrap_or_else(|error| panic!("the yard: {error:?}"))
    }

    fn spots() -> Vec<KeySpot> {
        vec![
            KeySpot {
                name: SmolStr::new("Near Spot"),
                x: 3,
                y: 2,
            },
            // Under RB at keyframe 1: drawn, covered, named nowhere.
            KeySpot {
                name: SmolStr::new("Covered"),
                x: 2,
                y: 9,
            },
            KeySpot {
                name: SmolStr::new("Far Spot"),
                x: 90,
                y: 60,
            },
        ]
    }

    fn fixture_replay() -> Replay {
        let mut fold = yard_fold();
        fold.absorb(&summary(
            5_000,
            &[(RA, 6, 4), (RB, 2, 9), (BA, 36, 14), (BB, 14, 7)],
        ));
        fold.absorb(&fired(6_000, RA, BA, 1));
        fold.absorb(&hit(6_300, 1, RA, BA));
        fold.absorb(&event(
            5,
            4_000,
            EventKind::UnitDestroyed { unit: BB, by: None },
        ));
        fold.absorb(&summary(
            10_000,
            &[(RA, 60, 40), (RB, 2, 9), (BA, 36, 14), (BB, 14, 7)],
        ));
        fold.finish(MatchOutcome::Red, "elimination")
    }

    /// **`replay_window` over the yard**: the south-west clamp, the marker order
    /// (spots, then the roster), a wreck's glyph and name, a covered spot named
    /// nowhere, the legend's glyph entries only when drawn, `you: null`,
    /// `retain: latest`, and the focus off the keyframe centring on the arena.
    #[test]
    fn i1_replay_window_over_the_yard() {
        let replay = fixture_replay();
        let window = replay_window(&replay, 1, 0, &yard(), &spots());
        assert_eq!(window["schema"]["id"], serde_json::json!("arena.replay"));
        assert_eq!(window["schema"]["shape"], serde_json::json!("grid"));
        assert_eq!(window["schema"]["retain"], serde_json::json!("latest"));
        assert_eq!(window["you"], serde_json::Value::Null);
        assert_eq!(
            window["title"],
            serde_json::json!("Replay of match 12 at 0:05")
        );
        assert_eq!(
            window["origin_cell"],
            serde_json::json!([0, 0]),
            "the south-west clamp"
        );
        assert_eq!(
            (window["width"].clone(), window["height"].clone()),
            (serde_json::json!(39), serde_json::json!(20))
        );
        assert_eq!(
            window["keyframe"],
            serde_json::json!({ "index": 1, "of": 3, "t_ms": 5_000 })
        );
        let units = window["units"].as_array().cloned().unwrap_or_default();
        let ids: Vec<&str> = units
            .iter()
            .filter_map(|unit| unit["id"].as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["A", "B", "RA", "RB", "BA", "BB"],
            "spots, then roster order"
        );
        let bb = units
            .iter()
            .find(|unit| unit["id"] == "BB")
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            (bb["name"].clone(), bb["glyph"].clone()),
            (serde_json::json!("wreck"), serde_json::json!("%"))
        );
        assert_eq!(
            units
                .iter()
                .find(|unit| unit["id"] == "RA")
                .map(|unit| unit["glyph"].clone()),
            Some(serde_json::json!("o"))
        );
        assert_eq!(
            units
                .iter()
                .find(|unit| unit["id"] == "BA")
                .map(|unit| unit["glyph"].clone()),
            Some(serde_json::json!("*"))
        );
        let legend = window["legend"].as_array().cloned().unwrap_or_default();
        assert!(legend.contains(&serde_json::json!(["o", "red"])));
        assert!(legend.contains(&serde_json::json!(["*", "blue"])));
        assert!(legend.contains(&serde_json::json!(["%", "wreck"])));
        assert!(legend.contains(&serde_json::json!(["A", "Near Spot"])));
        assert!(
            !legend.iter().any(|entry| entry[1] == "Covered"),
            "a covered spot is named nowhere"
        );
        assert!(
            !legend.iter().any(|entry| entry[1] == "Far Spot"),
            "a spot off the window is not drawn"
        );

        // Keyframe 0: BB is off the window (None), nobody is a wreck yet.
        let opening = replay_window(&replay, 0, 0, &yard(), &spots());
        let legend = opening["legend"].as_array().cloned().unwrap_or_default();
        assert!(
            !legend.contains(&serde_json::json!(["%", "wreck"])),
            "no wreck drawn, none named"
        );
        // Keyframe 2 centred on RA far away: only RA is drawn.
        let far = replay_window(&replay, 2, 0, &yard(), &spots());
        let ids: Vec<String> = far["units"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|unit| unit["id"].as_str().map(str::to_owned))
            .collect();
        assert_eq!(ids, vec!["RA"]);
        let legend = far["legend"].as_array().cloned().unwrap_or_default();
        assert!(
            !legend.contains(&serde_json::json!(["*", "blue"])),
            "a glyph only when drawn"
        );
        // A focus not in the keyframe: the arena's centre.
        let lost = replay_window(&replay, 0, 3, &yard(), &spots());
        assert_eq!(lost["origin_cell"], serde_json::json!([29, 22]));
    }

    /// **The reply's lines**: the header, the off-window line with its cap and
    /// wreck word, the shots and wrecks since the previous keyframe, `Next:`.
    #[test]
    fn i1_the_reply_lines_are_verbatim() {
        let replay = fixture_replay();
        let (header, lines) = replay_lines(&replay, 2, 0, &yard());
        assert_eq!(
            header,
            "Replay of match 12 at 0:10, keyframe 3 of 3 (10 s, red won (blue eliminated)): both \
             sides as the arena recorded them every 5 s, around RA."
        );
        assert_eq!(
            lines,
            vec![
                // RA stands at (60, 40): RB at (2, 9) is 58 cells west and 31
                // south, 657.6 m at 241.9°.
                "Off the window: RB 660 m at 241°; BA 350 m at 222°; BB (wreck) 570 m at 234° \
                 from RA."
                    .to_owned(),
                // The shot at 6,000 is after keyframe 1 and at or before this
                // one; the last keyframe owes no `Next:`.
                "Shots since 0:05: RA at BA, hit.".to_owned(),
            ]
        );
        let (_, lines) = replay_lines(&replay, 1, 0, &yard());
        assert_eq!(
            lines,
            vec![
                "Wrecked since 0:00: BB.".to_owned(),
                "Next: SIM REPLAY 12 next".to_owned(),
            ]
        );
        // Eight entries and the rest counted, on either list.
        let mut crowded = replay.clone();
        crowded.shots = (0..10)
            .map(|n| ReplayShot {
                t_ms: 7_000 + n,
                shooter: 0,
                target: 2,
                outcome: ShotOutcome::Miss,
            })
            .collect();
        let (_, lines) = replay_lines(&crowded, 2, 0, &yard());
        assert!(
            lines.contains(
                &"Shots since 0:05: RA at BA, miss; RA at BA, miss; RA at BA, miss; RA at BA, \
                  miss; RA at BA, miss; RA at BA, miss; RA at BA, miss; RA at BA, miss, and 2 \
                  more."
                    .to_owned()
            ),
            "{lines:?}"
        );
        assert_eq!(
            capped(&(1..=10).map(|n| n.to_string()).collect::<Vec<_>>()),
            "1; 2; 3; 4; 5; 6; 7; 8, and 2 more"
        );
        // One truncation line a cause, each true of what was cut.
        let mut cut = replay.clone();
        cut.truncated = true;
        cut.cut = vec![
            ReplayCut::Roster,
            ReplayCut::Keyframes,
            ReplayCut::Shots,
            ReplayCut::Page,
        ];
        let (_, lines) = replay_lines(&cut, 0, 0, &yard());
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("the tape") || line.contains("The replay"))
                .cloned()
                .collect::<Vec<_>>(),
            vec![
                "The replay names the first 4 units of match 12; the tape has the rest.".to_owned(),
                "The replay holds the first 10 s of match 12; the tape has the rest.".to_owned(),
                "The replay keeps the shots of match 12 to 0:06; the tape has the rest.".to_owned(),
                "A page of the tape of match 12 could not be read; the replay is missing what it \
                 held."
                    .to_owned(),
            ]
        );
        let mut shots_only = replay.clone();
        shots_only.truncated = true;
        shots_only.cut = vec![ReplayCut::Shots];
        let (_, lines) = replay_lines(&shots_only, 0, 0, &yard());
        assert!(
            !lines.iter().any(|line| line.contains("holds the first")),
            "a shot cut never claims the keyframes were cut: {lines:?}"
        );
        assert_eq!(every_words(100), "100 ms");
    }

    /// **The last keyframe of a whole replay runs to the end** (16b cut I.1,
    /// the fixer's T1): an elimination's killing shot and wreck, after the
    /// last position summary, are listed on it — `to the end at` — and its
    /// window draws the wreck; a keyframe before it does not; and the last
    /// keyframe of a replay sealed at a keyframe answers for itself alone.
    #[test]
    fn i1_the_last_keyframe_shows_how_the_match_ended() {
        let mut fold = yard_fold();
        fold.absorb(&summary(
            5_000,
            &[(RA, 6, 4), (RB, 2, 9), (BA, 10, 4), (BB, 14, 7)],
        ));
        // After the last summary: the killing shot and the wreck, then the end.
        fold.absorb(&fired(6_200, RA, BA, 1));
        fold.absorb(&hit(6_400, 1, RA, BA));
        fold.absorb(&event(
            5,
            6_400,
            EventKind::UnitDestroyed {
                unit: BA,
                by: Some(RA),
            },
        ));
        fold.absorb(&event(
            6,
            6_400,
            EventKind::MatchEnded {
                reason: crate::arena::state::EndReason::Timeout,
            },
        ));
        let replay = fold.finish(MatchOutcome::Red, "elimination");
        assert_eq!(replay.horizon(1), 6_400);
        assert_eq!(replay.horizon(0), 0);
        let (_, lines) = replay_lines(&replay, 1, 0, &yard());
        assert_eq!(
            lines,
            vec![
                "Shots since 0:00, to the end at 0:06: RA at BA, hit.".to_owned(),
                "Wrecked since 0:00, to the end at 0:06: BA.".to_owned(),
            ]
        );
        let window = replay_window(&replay, 1, 0, &yard(), &spots());
        assert!(
            window["units"].as_array().is_some_and(|units| units
                .iter()
                .any(|unit| unit["id"] == "BA" && unit["glyph"] == "%" && unit["name"] == "wreck")),
            "{window}"
        );
        // A replay sealed at that keyframe holds nothing past it, and says
        // nothing about an end it does not show.
        let mut sealed = replay.clone();
        sealed.shots.clear();
        sealed.destroyed.clear();
        sealed.truncated = true;
        sealed.cut = vec![ReplayCut::Keyframes];
        assert_eq!(sealed.horizon(1), 5_000);
        let (_, lines) = replay_lines(&sealed, 1, 0, &yard());
        assert!(
            !lines.iter().any(|line| line.contains("to the end")),
            "{lines:?}"
        );
    }

    /// **Where a step lands** (16b cut I.1, the fixer's T4): over keyframes at
    /// 0, 5,000 and 10,000 ms, a time is the first keyframe at or after it,
    /// else the last; `next` and `prev` step from the cursor and refuse at the
    /// ends; no cursor counts as keyframe 0; a cursor past the tape is clamped.
    #[test]
    fn i1_a_step_lands_on_the_keyframe_it_names() {
        let keyframes: Vec<Keyframe> = [0, 5_000, 10_000]
            .into_iter()
            .map(|t_ms| Keyframe {
                t_ms,
                units: Vec::new(),
            })
            .collect();
        let at =
            |step: Option<ReplayStep>, cursor: Option<usize>| step_index(step, cursor, &keyframes);
        for (asked, index) in [
            (0, 0),
            (3_000, 1),
            (5_000, 1),
            (6_000, 2),
            (10_000, 2),
            (11_000, 2),
        ] {
            assert_eq!(
                at(Some(ReplayStep::At(asked)), Some(0)),
                Ok(index),
                "{asked} ms"
            );
        }
        assert_eq!(at(None, Some(2)), Ok(0), "a bare ask is keyframe 0");
        assert_eq!(at(Some(ReplayStep::First), Some(2)), Ok(0));
        assert_eq!(at(Some(ReplayStep::Last), None), Ok(2));
        assert_eq!(at(Some(ReplayStep::Next), None), Ok(1), "no cursor is 0");
        assert_eq!(at(Some(ReplayStep::Next), Some(1)), Ok(2));
        assert_eq!(at(Some(ReplayStep::Next), Some(2)), Err(TapeEnd::Last));
        assert_eq!(at(Some(ReplayStep::Next), Some(9)), Err(TapeEnd::Last));
        assert_eq!(at(Some(ReplayStep::Prev), Some(2)), Ok(1));
        assert_eq!(
            at(Some(ReplayStep::Prev), Some(9)),
            Ok(1),
            "clamped, then back"
        );
        assert_eq!(at(Some(ReplayStep::Prev), Some(0)), Err(TapeEnd::First));
        assert_eq!(at(Some(ReplayStep::Prev), None), Err(TapeEnd::First));
    }

    /// **The grammar**: the match word, at most one step, at most one callsign,
    /// in any order.
    #[test]
    fn i1_the_grammar() {
        let ask = |word: &str| parse_ask(word);
        assert_eq!(
            ask("12"),
            Some(ReplayAsk {
                match_id: MatchId(12),
                step: None,
                callsign: None
            })
        );
        assert_eq!(
            ask("#12 1:30"),
            Some(ReplayAsk {
                match_id: MatchId(12),
                step: Some(ReplayStep::At(90_000)),
                callsign: None
            })
        );
        assert_eq!(
            ask("12 next ba"),
            Some(ReplayAsk {
                match_id: MatchId(12),
                step: Some(ReplayStep::Next),
                callsign: Some("BA".to_owned())
            })
        );
        assert_eq!(
            ask("12 ba prev").map(|ask| ask.step),
            Some(Some(ReplayStep::Prev))
        );
        assert_eq!(
            ask("12 first").map(|ask| ask.step),
            Some(Some(ReplayStep::First))
        );
        assert_eq!(
            ask("12 LAST").map(|ask| ask.step),
            Some(Some(ReplayStep::Last))
        );
        assert_eq!(
            ask("12 61:20").map(|ask| ask.step),
            Some(Some(ReplayStep::At(3_680_000)))
        );
        assert_eq!(
            ask("12 1:01:20").map(|ask| ask.step),
            Some(Some(ReplayStep::At(3_680_000)))
        );
        // `12 zz` parses; the roster refuses it.
        let zz = ask("12 zz").and_then(|ask| ask.callsign);
        assert_eq!(zz.as_deref(), Some("ZZ"));
        assert_eq!(fixture_replay().roster_index("zz"), None);
        assert_eq!(fixture_replay().roster_index("bb"), Some(3));
        for bad in [
            "",
            "twelve",
            "12 1:30 1:40",
            "12 ba bb",
            "12 next prev",
            "12 1:60",
            "12 13",
            "12 b1",
            "12 1:3",
        ] {
            assert_eq!(ask(bad), None, "{bad:?}");
        }
    }
}
