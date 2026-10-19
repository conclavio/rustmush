//! **The match's states and the one document it leaves behind** (16b cut T.3;
//! doc 16 §3.8 "Lifecycle", §4 "The result record").
//!
//! # The two documents of a running match (16b cut AF.2; 16c N16)
//!
//! Beside the result, which a match leaves behind, a **running** match now keeps
//! two documents in the store while it runs, written on the arena writer lane
//! (the module's own op lane, `crate::store::push_op`, the lane its chunks ride):
//!
//! - **`tacsim:snapshot:<n>`** ([`snapshot_key`]) — the arena's own document,
//!   exactly as [`crate::arena::Snapshot::encode`] built it, written by
//!   `crate::arena_bridge`'s `on_snapshot` every `[tacsim.arena].snapshot_ms` of
//!   arena time. It carries **no `match` key** (the arena library has no match
//!   id, and its document refuses unknown keys): the key alone names the match,
//!   and nothing but `on_snapshot` writes it.
//! - **`tacsim:live:<n>`** ([`live_key`]) — **the live record** ([`LiveRecord`]),
//!   the world's own RAM the next boot will need to resume the match: the
//!   [`MatchRecord`] with its debrief folds and its replay fold, the centre with
//!   its slots, the tape's counters and the moment of the last snapshot. Written
//!   by [`write_live`] at check-out, beside every snapshot (in the same action,
//!   so both ride one batch) and when the record parks in `Stopping`.
//!
//! Both are deleted with the result, in its action, on every end including a
//! void ([`delete_live_and_snapshot`]), and by the boot for every match it does
//! not resume (in 16b cut AF.2, every one: a restart still voids; AF.3 resumes).
//!
//! **Disjoint key ownership.** Both keys are in tacsim's own namespace (the
//! module stamps `ns: "tacsim"` on every op it pushes), beside
//! `tacsim:match:<n>`, `tacsim:report:<n>:<unit>` and `tacsim:replay:<n>`; the
//! tape is `log:match.<n>:<k>` (`mush_core::chunk_key`). No writer writes
//! another's key, and the four id parsers — [`match_id_of_key`],
//! [`crate::match_::replay::replay_id_of_key`], [`snapshot_id_of_key`] and
//! [`live_id_of_key`] — accept pairwise disjoint sets (the unit tests below).
//!
//! **Readable by nobody but the boot** (doc 16 §1's observer rule, for two new
//! surfaces). The live record holds account ids, the seed and every pilot's
//! fold; the snapshot holds every contact id and every position. The only store
//! reads this crate makes are `SIM REPORT`'s ([`report_key`]),
//! `@arena/report`'s (`mush_core::chunk_key`) and M6 BENCH's completed,
//! character-owned immutable proving blobs. Tests below check those key spaces
//! and pin every reader, so another by-key reader requires review before it
//! could address either private running document.
//!
//! **The `LIVE_FORMAT` policy.** A later shape change to [`MatchRecord`],
//! [`crate::match_::lobby::Centre`], [`crate::match_::lobby::Slot`],
//! [`crate::match_::report::Fold`] or [`crate::match_::replay::ReplayFold`] (or
//! anything they hold) is either **an added field with `#[serde(default)]`** — an
//! older live record still decodes, which is how a running half written by one
//! build resumes under the next — or **a [`LIVE_FORMAT`] bump**, which voids a
//! running match at the reload by name. 16b cut AF.3 adds one field the first
//! way. Every struct of the tree refuses an unknown key, so a record written by
//! a newer build is refused rather than half-read.

use std::borrow::Cow;
use std::collections::BTreeMap;

use mush_core::{Ctx, ObjId};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use smol_str::SmolStr;

use crate::arena::state::{Designator, EndReason, MatchEnd, MatchMode, Team};
use crate::arena_task::MatchId;

/// **The lifecycle** of doc 16 §3.8.
///
/// ```text
/// Idle → Selection → Boarding → Running → Ended → Idle
///                  ↘        ↘        ↘
///                   Aborted ← Aborted ← Aborted | Void → Idle
/// ```
///
/// `Stopping` is not in the diagram and is not a state of the *match*: it is the
/// world's note that a `Stop` has been sent and the arena has not reported its
/// end yet. It exists so `@arena/abort` is idempotent and `@arena/status` can say
/// what is happening; the match is still `Running` as far as the arena is
/// concerned, which is exactly why a second abort is harmless.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchState {
    /// No match.
    #[default]
    Idle,
    /// Slots are being claimed (cut K's `SIM TEAM`).
    Selection,
    /// Pods are locked and pilots are boarding (cut K's `BOARD`).
    Boarding,
    /// The arena is stepping. The units are checked out.
    Running,
    /// A `Stop` has been sent and the arena has not answered yet.
    Stopping,
    /// The arena ended the match on its own terms.
    Ended,
    /// The match was aborted.
    Aborted,
    /// A panic, a stall or a ledger overflow voided the match.
    Void,
}

impl MatchState {
    /// The word `@arena/status` prints.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            MatchState::Idle => "idle",
            MatchState::Selection => "selection",
            MatchState::Boarding => "boarding",
            MatchState::Running => "running",
            MatchState::Stopping => "stopping",
            MatchState::Ended => "ended",
            MatchState::Aborted => "aborted",
            MatchState::Void => "void",
        }
    }

    /// Whether the units of this match are checked out — the one question every
    /// caller actually asks.
    #[must_use]
    pub const fn is_live(&self) -> bool {
        matches!(self, MatchState::Running | MatchState::Stopping)
    }

    /// Whether this state may become `Running`. `Boarding → Running` is the
    /// check-out trigger; nothing else reaches it (doc 16 §3.8).
    #[must_use]
    pub const fn may_start(&self) -> bool {
        matches!(self, MatchState::Boarding)
    }

    /// **Whether this state is an end** (16b cut F.1) — the three states doc 16
    /// §3.8 draws the `→ Idle` edge out of, and the ones a claim resets through.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            MatchState::Ended | MatchState::Aborted | MatchState::Void
        )
    }

    /// Whether this state may become `Aborted` — every state before an end can,
    /// and one before `Running` has no custody to return.
    #[must_use]
    pub const fn may_abort(&self) -> bool {
        matches!(
            self,
            MatchState::Selection
                | MatchState::Boarding
                | MatchState::Running
                | MatchState::Stopping
        )
    }
}

impl From<EndReason> for MatchState {
    fn from(reason: EndReason) -> MatchState {
        match reason {
            // 16b cut Q.7 (`Spent`) and 16b cut D.3 (`Conceded`, `Unattended`):
            // every one of these is a match that ran to a conclusion, so it ends
            // like an elimination and not like an abort — and its cells are
            // committed, which is what makes the loser's wreck evidence.
            //
            // 16b cut O.1 (`Hold`): a side kept the key spot, which is a match
            // fought to a conclusion like any other.
            EndReason::Elimination { .. }
            | EndReason::Spent
            | EndReason::Hold { .. }
            | EndReason::Conceded { .. }
            | EndReason::Unattended
            | EndReason::Timeout => MatchState::Ended,
            EndReason::Abort => MatchState::Aborted,
            // A ledger overflow is a void by doc 16 §3.1's own words: *on `Void`
            // (panic or ledger overflow) the world restores each unit from the
            // check-out snapshot*. The arena reports it as its own reason so the
            // ledger says which one it was; custody treats the two alike.
            EndReason::Void | EndReason::LedgerOverflow => MatchState::Void,
        }
    }
}

/// **The outcome of doc 16 §4**: `red | blue | draw | abort | timeout | void`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchOutcome {
    /// Red won.
    Red,
    /// Blue won.
    Blue,
    /// Neither side was eliminated.
    Draw,
    /// Somebody stopped it.
    Abort,
    /// `match.max_duration_ms` or the ledger's own ceiling ended it.
    Timeout,
    /// A panic, a stall or a ledger overflow.
    Void,
}

impl MatchOutcome {
    /// The word the result document carries.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            MatchOutcome::Red => "red",
            MatchOutcome::Blue => "blue",
            MatchOutcome::Draw => "draw",
            MatchOutcome::Abort => "abort",
            MatchOutcome::Timeout => "timeout",
            MatchOutcome::Void => "void",
        }
    }
}

impl From<EndReason> for MatchOutcome {
    /// **16b cut W.2 made the elimination arms reachable**: doc 16 §3.8's team
    /// elimination is what `Arena::settle_elimination` records, and the arena's own
    /// [`EndReason`] carries the winning side, so this is a read and never a
    /// judgement.
    fn from(reason: EndReason) -> MatchOutcome {
        match reason {
            EndReason::Elimination {
                winner: Some(Team::Red),
            } => MatchOutcome::Red,
            EndReason::Elimination {
                winner: Some(Team::Blue),
            } => MatchOutcome::Blue,
            // 16b cut Q.7: nobody can shoot, so nobody won.
            EndReason::Elimination { winner: None } | EndReason::Spent => MatchOutcome::Draw,
            // 16b cut D.3 (`B-33`): a concede names its winner, so the outcome is
            // that side's word and not a seventh one.
            EndReason::Conceded { winner: Team::Red } => MatchOutcome::Red,
            EndReason::Conceded { winner: Team::Blue } => MatchOutcome::Blue,
            // 16b cut O.1: a hold names its winner too, and the reason word
            // (`hold`) says how.
            EndReason::Hold { winner: Team::Red } => MatchOutcome::Red,
            EndReason::Hold { winner: Team::Blue } => MatchOutcome::Blue,
            // And a match every pilot had left is a `timeout`: doc 16 §4's
            // vocabulary is six words, and this is the one that means *the clock
            // ended it and nobody was eliminated*.
            EndReason::Unattended | EndReason::Timeout => MatchOutcome::Timeout,
            EndReason::Abort => MatchOutcome::Abort,
            EndReason::Void | EndReason::LedgerOverflow => MatchOutcome::Void,
        }
    }
}

/// **What happened to one unit** (doc 16 §4): `intact | disabled | destroyed |
/// custody_lost`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitOutcome {
    /// It came back whole.
    Intact,
    /// It came back unable to fight — no weapons and no motion (doc 16 §3.8; 16b
    /// cut W.2).
    Disabled,
    /// It is a wreck (doc 16 §3.3; 16b cut W.2).
    Destroyed,
    /// The world could not take it back: the object is gone, or its record
    /// refused the return.
    CustodyLost,
}

/// One unit's row on the result record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitResult {
    /// The unit.
    pub unit: u32,
    /// **The unit's callsign** (16b cut V.2), wire key `"callsign"`: the
    /// [`Designator`]'s `Display` (`RA`, `BA`), as the arena resolved it at
    /// check-out ([`MatchRecord::callsigns`]). Empty for a row written with no
    /// record to name it from — the boot's `aborted(restart)` rows.
    pub callsign: SmolStr,
    /// How it came back.
    pub outcome: UnitOutcome,
    /// Shots fired, off the arena's own counter (16b cut W.2). `0` for a unit the
    /// arena's end message did not name, and for every unit on a void.
    pub shots: u32,
    /// **How many of them took something off something**, likewise (16b cut Q.6).
    ///
    /// It is [`crate::arena::damage::UnitReturn::hits`] copied through the check-in
    /// message, and that field's doc is the argument: it is **not** the black
    /// box's count of `hit` lines, because a round that reaches a section already
    /// at zero armour and zero internals did land and is journalled, but moved no
    /// cell. A reader of this record who cross-checks the two will find them
    /// differing by exactly the rounds spent on spent sections.
    pub hits: u32,
}

impl UnitResult {
    /// A unit that came back whole, under `callsign` (16b cut V.2).
    #[must_use]
    pub fn intact(unit: ObjId, callsign: SmolStr) -> UnitResult {
        UnitResult {
            unit: unit.0,
            callsign,
            outcome: UnitOutcome::Intact,
            shots: 0,
            hits: 0,
        }
    }

    /// A unit the world could not take back, under `callsign` (16b cut V.2).
    #[must_use]
    pub fn lost(unit: ObjId, callsign: SmolStr) -> UnitResult {
        UnitResult {
            unit: unit.0,
            callsign,
            outcome: UnitOutcome::CustodyLost,
            shots: 0,
            hits: 0,
        }
    }
}

/// One side, as the record holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamRoster {
    /// Which side.
    pub team: Team,
    /// Its units.
    pub units: Vec<ObjId>,
    /// Its `(pilot, unit)` pairs at check-out.
    pub pilots: Vec<(ObjId, ObjId)>,
}

/// **One match, while the world is tracking it** — in RAM, and mirrored to
/// `tacsim:live:<n>` ([`LiveRecord`]) at check-out, with every snapshot and
/// when it parks in `Stopping` (16b cut AF.2), which is what a boot resumes
/// it from (16b cut AF.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchRecord {
    /// The match.
    pub id: MatchId,
    /// Where it is in doc 16 §3.8's machine.
    pub state: MatchState,
    /// The arena definition it names.
    pub arena: SmolStr,
    /// That definition's digest, so a replayed ledger can be told to belong to
    /// the terrain it was produced on (doc 16 §4).
    pub arena_definition_digest: String,
    /// Its sides.
    pub teams: Vec<TeamRoster>,
    /// When it started, in world milliseconds.
    pub started_ms: i64,
    /// How it ended, once it has.
    pub ended: Option<MatchEnd>,
    /// The ledger's log id (`match.<id>`).
    pub ledger_log_id: String,
    /// The ledger's digest, once the arena has reported it.
    pub ledger_digest: String,
    /// The match seed, so a fault can be armed against exactly this arena and a
    /// replay can name the stream it drew from.
    pub seed: u64,
    /// **The last pose the world saw for each unit**, as the four integers doc
    /// 16 §3.2 defines a pose by.
    ///
    /// Doc 16's `CustodyIn` carries a final pose per unit, and **16b cut W.2** put
    /// one on `ArenaOut::Ended`; this is what a check-in falls back to for a unit
    /// that message did not name. Two sources feed it, both the arena's own numbers:
    /// the **ledger's `position_summary`** events, which name every unit every
    /// `ledger.summary_ms` whether or not anybody is watching, and the
    /// **cockpit frames**, which are fresher but only exist for a unit with a
    /// pilot in it. Neither invents anything: the posture a check-in writes is
    /// the one the check-out snapshot carried, because the arena never touches a
    /// posture.
    pub last_pose: BTreeMap<ObjId, PoseNumbers>,
    /// **The squads its reflex units are commanded in** (16b cut N.2; doc 16 §6
    /// "Deliberation": *requested per squad or commander rather than per unit*).
    ///
    /// Derived at check-out by `crate::deliberation::squads_of` from the same
    /// `(unit, team, controller)` triples the `CustodyOut` carries and the same
    /// `[tacsim.arena].squad_size` (16b cut Z.1), so the record's
    /// grouping and the arena's are the same function of the same data and cannot
    /// disagree. It is on the record and not only in the arena because the world is
    /// what fans an order out to a squad's units and what says the sentence when one
    /// expires, and both need the roster without asking across the seam.
    ///
    /// Empty for a match of nothing but pilots, which is doc 16 §1's A1 story.
    pub squads: Vec<crate::deliberation::SquadRoster>,
    /// **The squads' lance-leaders** (17b cut L.1; doc 17 §4 "Where a
    /// lance-leader sits", D-UM-53): minted by [`crate::match_::start`] beside
    /// [`MatchRecord::squads`] — one sim persona per squad
    /// (`crate::deliberation::fill_leaders`), because every squad of the OOC
    /// sim is a fill squad — and read by `crate::arena_bridge`'s
    /// `on_deliberate` (a squad with no row is never asked),
    /// `on_deliberation_reply` (a reply whose author is not the row is dropped)
    /// and `admit_order` (a persona's order is admitted only for its own squad).
    ///
    /// `#[serde(default)]` under this module head's `LIVE_FORMAT` policy — an
    /// added defaulted field, so no bump: a live or result document written
    /// before this cut has no leaders, which is true of it.
    #[serde(default)]
    pub leaders: Vec<crate::deliberation::LanceLeader>,
    /// **Every unit's callsign, as the arena resolved it** (16b cut V.2).
    ///
    /// Read off the arena `crate::arena_task::spawn_arena` built
    /// (`crate::arena::state::Arena::callsigns`) and handed back through
    /// `crate::arena_bridge::open_match`; written by [`crate::match_::start`]
    /// beside the `Boarding → Running` transition. The arena is the one thing
    /// that applied 16b cut Q.1's slot rule — explicit slots first, then the
    /// slot-0 units in unit-id order — so the roster's order is not the slot
    /// order and this is not re-derived from it. Empty before `Running`, and for
    /// a check-out the arena refused.
    pub callsigns: BTreeMap<ObjId, Designator>,
    /// **One debrief fold per human pilot's unit** (16b cut V.3; 16c N2, Q37).
    ///
    /// Built by [`crate::match_::start`] beside [`MatchRecord::callsigns`], from
    /// the roster ([`crate::match_::report::Roster::from_record`]) and the
    /// match's own `designate_on_detection`; fed every event of every chunk by
    /// `crate::arena_bridge`'s `on_chunk` as the world appends it; taken off
    /// the record and finished exactly once by [`crate::match_::finish`], which
    /// writes one `tacsim:report:<match>:<unit>` document per fold
    /// ([`write_report`]). A unit with no pilot has none. Empty before
    /// `Running`.
    pub folds: BTreeMap<ObjId, crate::match_::report::Fold>,
    /// **Tape lines the world could not parse** (16b cut V.3), counted by
    /// `on_chunk` as it hands each chunk's events to the folds.
    pub unparsed: u32,
    /// **The replay fold** (16b cut I.1; 16c N9, Q37): one per match, both
    /// sides, built by [`crate::match_::start`] beside [`MatchRecord::folds`]
    /// from the check-out poses and the arena's callsigns; fed every event of
    /// every chunk by `crate::arena_bridge`'s `on_chunk`; taken off the record
    /// and finished exactly once by [`crate::match_::finish`], which writes one
    /// `tacsim:replay:<match>` document ([`write_replay`]). `None` before
    /// `Running`.
    pub replay: Option<crate::match_::replay::ReplayFold>,
    /// **The rule the match is fought under** (16b cut O.1; 16c N11), as
    /// [`crate::match_::start`] was given it and the `CustodyOut` carried it —
    /// what the result document's `"mode"` and the replay's say.
    pub mode: MatchMode,
    /// **Every unit's seal** (17b cut M3.3; doc 17 §13 "Where it goes",
    /// D-UM-46): one [`MindSealRecord`] per unit, keyed by unit — in the arena
    /// exactly one sortie per unit, opened at the pod's exit and closed at the
    /// match's end. Written by [`crate::match_::start`] from each
    /// `CustodyUnit::seal` before the record is inserted, and carried by the
    /// live record, so a resume can refuse by name a seal this server cannot
    /// run (`crate::match_::resume::resume_pending`). The arena's own copy of
    /// each seal's reference rides its snapshot and is never re-derived from
    /// this. The result document carries nothing of it.
    ///
    /// `#[serde(default)]` under this module head's `LIVE_FORMAT` policy (no
    /// bump, [`MatchRecord::leaders`]' precedent): a live record written
    /// before this cut has no seals, which is true of it.
    #[serde(default)]
    pub minds: BTreeMap<ObjId, MindSealRecord>,
}

/// **One sortie's seal, as the world keeps it** (17b cut M3.3; doc 17 §13,
/// D-UM-46): the record is keyed by `(unit, sortie)` — the unit is
/// [`MatchRecord::minds`]' key, the sortie is here — and holds the seal whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MindSealRecord {
    /// The sortie ([`crate::arena::mind::SortieId::in_match`]).
    pub sortie: crate::arena::mind::SortieId,
    /// The seal.
    pub seal: crate::arena::mind::MindSeal,
}

/// The four integers of a pose (doc 16 §3.2), without the opaque posture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoseNumbers {
    /// East, centimetres.
    pub x_cm: i64,
    /// North, centimetres.
    pub y_cm: i64,
    /// Height, centimetres.
    pub z_cm: i32,
    /// Bearing, millidegrees.
    pub heading_mdeg: u32,
}

impl PoseNumbers {
    /// The numbers of a pose.
    #[must_use]
    pub const fn of(pose: &mush_sim::data::Pose) -> PoseNumbers {
        PoseNumbers {
            x_cm: pose.x_cm,
            y_cm: pose.y_cm,
            z_cm: pose.z_cm,
            heading_mdeg: pose.heading_mdeg,
        }
    }

    /// These numbers on `posture` — the posture the check-out snapshot carried.
    #[must_use]
    pub fn onto(&self, posture: mush_sim::data::RulesValue) -> mush_sim::data::Pose {
        mush_sim::data::Pose {
            x_cm: self.x_cm,
            y_cm: self.y_cm,
            z_cm: self.z_cm,
            heading_mdeg: self.heading_mdeg,
            posture,
        }
    }
}

impl MatchRecord {
    /// Every unit in the match, in roster order.
    #[must_use]
    pub fn units(&self) -> Vec<ObjId> {
        self.teams
            .iter()
            .flat_map(|roster| roster.units.iter().copied())
            .collect()
    }

    /// Whether this match holds `unit`.
    #[must_use]
    pub fn holds(&self, unit: ObjId) -> bool {
        self.teams.iter().any(|roster| roster.units.contains(&unit))
    }

    /// Which side `unit` is on.
    #[must_use]
    pub fn team_of(&self, unit: ObjId) -> Option<Team> {
        self.teams
            .iter()
            .find(|roster| roster.units.contains(&unit))
            .map(|roster| roster.team)
    }

    /// **`unit`'s callsign as the result record writes it** (16b cut V.2): the
    /// designator's `Display` off [`MatchRecord::callsigns`], or the empty string
    /// for a unit the arena named nothing for (which a unit of a running match
    /// never is).
    #[must_use]
    pub fn callsign_word(&self, unit: ObjId) -> SmolStr {
        self.callsigns
            .get(&unit)
            .map(|designator| SmolStr::new(designator.to_string()))
            .unwrap_or_default()
    }
}

/// The result document's key inside the tacsim namespace.
#[must_use]
pub fn result_key(match_id: MatchId) -> SmolStr {
    SmolStr::new(format!("tacsim:match:{}", match_id.0))
}

/// **Write one match's result** (doc 16 §4), once, at the end.
///
/// One document, well under the 256 KiB record ceiling: the teams and their
/// pilots, the outcome with its reason, one row per unit, the ledger's log id,
/// digest and chunk count, **the match seed** (16b cut M.3), the arena
/// definition's digest, and the start and end times. It is written through the
/// module's own namespace on the ordinary module-op lane.
///
/// **The seed** (16b cut M.3, 16c `T-07`) is `"seed"` at the document's top
/// level: [`MatchRecord::seed`], the value the arena drew every draw of the match
/// with. With it and the ledger — whose lines carry each draw's label and value
/// (`LedgerEvent::draws`) — a reader can re-derive every draw, so which section
/// a round found and whether a pass told a target apart are checkable from the
/// record alone, beside the digest that proves the log is the one the arena
/// wrote. A match whose record is gone by the time the result is written writes
/// `0`, the value [`write_restart_abort`] writes too.
///
/// **The seed is staff-only.** It re-derives every draw of the match and,
/// through `crate::arena::rng::draw_contact_id`, every contact's object id, so it
/// must never be rendered to a player. The one reader of this document is the
/// history ring ([`MatchBook::remember`](crate::match_::MatchBook::remember)),
/// which keeps it on [`HistoryRow::seed`] for the staff report, and
/// [`history_lines`] — what `SIM HISTORY` prints — never reads it.
///
/// **16b cut V.2: the callsigns and the chunk count.** Every unit row and every
/// pilot row carries `"callsign"` ([`MatchRecord::callsigns`]), and `"ledger"`
/// carries `"chunks"`: `chunks`, the `MatchMirror::chunks_appended` of the mirror
/// `crate::arena_bridge::ArenaRegistry::untrack` handed back in
/// [`crate::match_::finish`] — the whole tape, because the arena task sends every
/// queued chunk before its `Ended`.
///
/// **And the ring.** The document is handed to [`HistoryRow::from_document`] and
/// remembered on the book **before** it is pushed — the very value, so what `SIM
/// HISTORY` reads and what the store holds cannot disagree — and a row the ring
/// evicts has its report documents deleted in this same action
/// ([`report_key`]; 16b cut V.3 writes them).
///
/// **16b cut V.3: `"reports"`.** The units a report document was written for in
/// this same action ([`write_report`], called first by
/// [`crate::match_::finish`]), in id order — what [`HistoryRow::reports`] reads
/// and what the ring's eviction deletes.
///
/// **16b cut I.1: `"replay"`.** Whether a `tacsim:replay:<match>` document
/// rode this same batch ([`write_replay`], called by [`crate::match_::finish`]
/// before this) — what [`HistoryRow::replay`] reads, what `SIM REPLAY` asks
/// before it looks for one, and what the ring's eviction deletes.
#[allow(clippy::too_many_arguments)]
pub fn write_result(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    outcome: MatchOutcome,
    end: MatchEnd,
    ledger_digest: &str,
    units: &[UnitResult],
    chunks: u64,
    reports: &[ObjId],
    replay: bool,
) {
    super::mind_tape::close_incomplete(ctx, match_id, "match ended without a complete recording");
    let record = ctx
        .w
        .module_state::<super::MatchBook>()
        .and_then(|book| book.get(match_id))
        .cloned();
    let ended_ms = ctx.w.now().saturating_mul(1_000);
    let value = result_document(
        record.as_ref(),
        match_id,
        outcome,
        end,
        ledger_digest,
        units,
        ended_ms,
        chunks,
        reports,
        replay,
    );
    remember_and_push(ctx, match_id, value);
}

/// **Write one pilot's report document** (16b cut V.3; 16c N2, Q37) at
/// [`report_key`], through the module's own namespace on the ordinary module-op
/// lane — [`crate::match_::finish`] calls it once per fold, before
/// `record::write_result`, so the result can name it and both ride one batch.
///
/// The shape is [`crate::match_::report::report_document`]'s: the match, the
/// unit, its pilot and callsign, the timeline, what was not kept, the tail, the
/// gap, the rounds per weapon and the kills. It holds what the pilot's own
/// platform knew and nothing else — never a draw, a seed, a contact id or an
/// object number in a line.
pub fn write_report(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    unit: ObjId,
    pilot: Option<ObjId>,
    callsign: &str,
    report: &crate::match_::report::Report,
) {
    crate::store::push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: report_key(match_id, unit),
            value: crate::match_::report::report_document(match_id, unit, pilot, callsign, report),
        },
    );
}

/// **Write one match's replay document** (16b cut I.1; 16c N9, Q37) at
/// [`crate::match_::replay::replay_key`], through the module's own namespace on
/// the ordinary module-op lane, exactly as [`write_report`] writes —
/// [`crate::match_::finish`] calls it once, before `record::write_result`, so
/// the result can say it exists and both ride one batch.
///
/// The shape is [`crate::match_::replay::Replay::to_document`]'s: both sides as
/// the arena recorded them, by callsign — never an object number, a pilot, a
/// contact id, a draw, a seed or a `z`.
pub fn write_replay(ctx: &mut Ctx<'_>, match_id: MatchId, replay: &crate::match_::replay::Replay) {
    crate::store::push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: crate::match_::replay::replay_key(match_id),
            value: replay.to_document(),
        },
    );
}

/// **The one tail both result writers share** (16b cut V.2): remember the
/// document on the ring, push it, and delete the report documents of the row the
/// ring let go.
///
/// The deletes ride the same outbox as the result, so the report documents are
/// bounded by the ring whichever writer evicted the row: a restart abort that
/// pushes the twenty-first row out is an eviction too.
///
/// **16b cut I.1**: and its replay document, when its row says one was written
/// ([`HistoryRow::replay`]), deleted in the same write beside the reports — and
/// the book's RAM copy, every cursor on it and any ground no remembered replay
/// names any more ([`crate::match_::MatchBook::forget_replay`]).
fn remember_and_push(ctx: &mut Ctx<'_>, match_id: MatchId, value: serde_json::Value) {
    let evicted = HistoryRow::from_document(&value).and_then(|row| {
        ctx.w
            .module_state_mut::<super::MatchBook>()
            .and_then(|book| book.remember(row))
    });
    crate::store::push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: result_key(match_id),
            value,
        },
    );
    if let Some(evicted) = evicted {
        if let Err(error) = super::mind_tape::evict(ctx, evicted.id) {
            tracing::warn!(%error, "retired input recording remains pinned or conservatively retained");
        }
        for unit in &evicted.reports {
            crate::store::push_op(
                ctx.out,
                mush_core::ModuleWrite::Del {
                    key: report_key(evicted.id, *unit),
                },
            );
        }
        if evicted.replay {
            crate::store::push_op(
                ctx.out,
                mush_core::ModuleWrite::Del {
                    key: crate::match_::replay::replay_key(evicted.id),
                },
            );
        }
        if let Some(book) = ctx.w.module_state_mut::<super::MatchBook>() {
            book.forget_replay(evicted.id);
        }
    }
}

/// **The result document [`write_result`] writes** (16b cut M.3 split it out of
/// the write, so the document's shape — the seed on it above all — is testable
/// without a world). `record` is the match's own record when the book still
/// holds it; `ended_ms` is the world's clock in milliseconds; `chunks` is the
/// tape's chunk count (16b cut V.2); `reports` the units a report document was
/// written for (16b cut V.3); `replay` whether a replay document was (16b cut
/// I.1).
#[allow(clippy::too_many_arguments)]
fn result_document(
    record: Option<&MatchRecord>,
    match_id: MatchId,
    outcome: MatchOutcome,
    end: MatchEnd,
    ledger_digest: &str,
    units: &[UnitResult],
    ended_ms: i64,
    chunks: u64,
    reports: &[ObjId],
    replay: bool,
) -> serde_json::Value {
    // 16b cut O.1: the mode off the record, and no key at all without one — a
    // document whose match the book no longer holds does not know it.
    let mode = record.map(|record| record.mode);
    let (teams, arena, arena_digest, started_ms, log_id, seed) = match record {
        Some(record) => (
            record
                .teams
                .iter()
                .map(|roster| {
                    serde_json::json!({
                        "team": roster.team.as_str(),
                        "units": roster.units.iter().map(|unit| unit.0).collect::<Vec<u32>>(),
                        // 16b cut V.2: each pilot row names the unit's callsign
                        // too, so a reader binds a pilot to a name without a
                        // second walk over the unit rows.
                        "pilots": roster
                            .pilots
                            .iter()
                            .map(|(pilot, unit)| serde_json::json!({
                                "pilot": pilot.0,
                                "unit": unit.0,
                                "callsign": record.callsign_word(*unit),
                            }))
                            .collect::<Vec<serde_json::Value>>(),
                    })
                })
                .collect::<Vec<serde_json::Value>>(),
            record.arena.to_string(),
            record.arena_definition_digest.clone(),
            record.started_ms,
            record.ledger_log_id.clone(),
            record.seed,
        ),
        None => (
            Vec::new(),
            String::new(),
            String::new(),
            0,
            crate::arena_bridge::match_log_id(match_id),
            0,
        ),
    };
    // Construct an object directly: later optional retained roots cannot be
    // silently skipped because a generic Value was not an object.
    let mut document = serde_json::Map::from_iter([
        ("record".into(), serde_json::Value::from("tacsim.match")),
        // 16b cut V.2 added `callsign` (unit and pilot rows) and `ledger.chunks`
        // and left this at 1: a key added is not a version — the rule
        // `crate::match_::cockpit::FRAME_VERSION`'s comment states — and a reader of
        // a document without them reads the empty name and nought
        // ([`HistoryRow::from_document`]).
        ("record_version".into(), 1.into()),
        ("match".into(), match_id.0.into()),
        ("arena".into(), arena.into()),
        ("arena_definition_digest".into(), arena_digest.into()),
        ("outcome".into(), outcome.as_str().into()),
        ("reason".into(), reason_word(end.reason).into()),
        ("teams".into(), serde_json::Value::Array(teams)),
        ("units".into(), serde_json::json!(units)),
        (
            "ledger".into(),
            serde_json::json!({ "log": log_id, "digest": ledger_digest, "chunks": chunks }),
        ),
        // 16b cut M.3 (16c `T-07`): staff-only — see `write_result`.
        ("seed".into(), seed.into()),
        ("started_ms".into(), started_ms.into()),
        ("ended_ms".into(), ended_ms.into()),
        ("arena_ms".into(), end.t_ms.into()),
        // 16b cut V.3: the units whose report documents rode this same batch.
        (
            "reports".into(),
            serde_json::Value::Array(reports.iter().map(|unit| unit.0.into()).collect()),
        ),
        // 16b cut I.1: whether this match's replay document rode it too.
        ("replay".into(), replay.into()),
    ]);
    // **16b cut O.1** (plan items 19, 39): the rule it was fought under — a key
    // added, not a version, by V.2's rule above. Absent reads `None`
    // ([`HistoryRow::from_document`]).
    if let Some(mode) = mode {
        document.insert("mode".to_owned(), serde_json::Value::from(mode.as_str()));
    }
    if let Some(record) = record {
        let revisions =
            super::selection_refs::revisions(record.minds.values().map(|row| &row.seal));
        if !revisions.is_empty() {
            document.insert(
                "mind_revisions".into(),
                serde_json::Value::Array(
                    revisions
                        .into_iter()
                        .map(revision_reference_document)
                        .collect(),
                ),
            );
        }
    }
    serde_json::Value::Object(document)
}

/// The retention wire shape uses only integer keys and a canonical digest.
/// Build its values directly; final match publication has no fallible serializer.
fn revision_reference_document(reference: mush_roll::RevisionRef) -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::from_iter([
        (
            "key".into(),
            serde_json::Value::Object(serde_json::Map::from_iter([
                ("character".into(), reference.key.character.0.into()),
                ("mind".into(), reference.key.mind.into()),
                ("number".into(), reference.key.number.into()),
            ])),
        ),
        ("digest".into(), reference.digest.to_hex().into()),
    ]))
}

#[cfg(test)]
mod retained_revision_wire_tests {
    #[test]
    fn direct_result_reference_encoding_matches_the_owner_schema() {
        let reference = mush_roll::RevisionRef {
            key: mush_roll::RevisionKey {
                character: mush_core::ObjId(42),
                mind: u64::MAX,
                number: 17,
            },
            digest: mush_core::mechanics::DefinitionDigest::from_bytes([255; 32]),
        };
        assert_eq!(
            super::revision_reference_document(reference),
            serde_json::to_value(reference).expect("owner schema")
        );
    }
}

/// **The `aborted(restart)` result** (16b T5, doc 16 §3.1 "Boot reconciliation").
///
/// The result of a match that was running when the server last stopped and
/// could not resume (16b cut AF.3, `crate::match_::void_at_boot`, whose log line
/// names the reason from a closed list); until that cut it was every restart's
/// answer. Such a match has no arena, no ledger digest and no end timestamp the
/// boot can trust. What it does have is an id, a set of units restored from
/// their check-out snapshots, and a reason, and that is exactly what this
/// writes.
///
/// **16b cut M.3**: its `"seed"` is `0`. There was no arena to have drawn
/// anything — the record the seed lived on went with the process — and the key
/// is present so the result document has one shape whichever way it was written.
///
/// **16b cut V.2**: it is remembered on the history ring exactly as
/// `write_result`'s document is (the same value, before the push), so `SIM
/// HISTORY` lists a restarted match the moment the boot writes it.
pub fn write_restart_abort(ctx: &mut Ctx<'_>, match_id: MatchId, units: &[ObjId]) {
    super::mind_tape::close_incomplete(ctx, match_id, "match could not resume after restart");
    let value = restart_abort_document(match_id, units, ctx.w.now().saturating_mul(1_000));
    remember_and_push(ctx, match_id, value);
}

/// The document [`write_restart_abort`] writes (split out by 16b cut M.3, as
/// `result_document` is).
///
/// **16b cut V.2**: every unit row's `"callsign"` is `""` and `"ledger"`'s
/// `"chunks"` is `0` — there was no record to name the units from and no mirror
/// to count the tape on, both having gone with the process. `"teams"` stays `[]`,
/// so a reader places these units on no side ([`HistoryUnit::team`] is `None`).
///
/// **16b cut O.1** (plan item 19): and it writes **no `"mode"`**. The boot does
/// not know what rule the match was fought under — the record that held it went
/// with the process — so the row reads `mode: None` and carries no tag, rather
/// than calling a hold a skirmish.
fn restart_abort_document(match_id: MatchId, units: &[ObjId], ended_ms: i64) -> serde_json::Value {
    let rows: Vec<UnitResult> = units
        .iter()
        .map(|unit| UnitResult::intact(*unit, SmolStr::default()))
        .collect();
    serde_json::json!({
        "record": "tacsim.match",
        "record_version": 1,
        "match": match_id.0,
        "arena": "",
        "arena_definition_digest": "",
        "outcome": MatchOutcome::Abort.as_str(),
        "reason": "restart",
        "teams": [],
        "units": rows,
        "ledger": {
            "log": crate::arena_bridge::match_log_id(match_id),
            "digest": "",
            "chunks": 0,
        },
        // 16b cut M.3: no arena drew anything.
        "seed": 0,
        "started_ms": 0,
        "ended_ms": ended_ms,
        "arena_ms": 0,
        // 16b cut V.3: there was no fold to finish, so no report was written.
        "reports": [],
        // 16b cut I.1: and no replay either — the process that folded one is
        // gone, and a restart has nothing to watch.
        "replay": false,
    })
}

/// The word the result's `reason` field carries.
///
/// **16b cut V.3** made it `pub(crate)`: a report's end line is
/// [`end_words`] over `MatchOutcome::from(reason)` and this word.
pub(crate) const fn reason_word(reason: EndReason) -> &'static str {
    match reason {
        EndReason::Elimination { .. } => "elimination",
        // 16b cut Q.7.
        EndReason::Spent => "spent",
        // 16b cut O.1: the outcome beside it names the side that held.
        EndReason::Hold { .. } => "hold",
        // 16b cut D.3 (`B-33`): the result document's own two new words. The
        // outcome beside them already names the winner.
        EndReason::Conceded { .. } => "conceded",
        EndReason::Unattended => "unattended",
        EndReason::Timeout => "timeout",
        EndReason::Abort => "abort",
        EndReason::Void => "void",
        EndReason::LedgerOverflow => "ledger_overflow",
    }
}

/// The match id a result key names, for the boot pass's high-water read.
#[must_use]
pub fn match_id_of_key(key: &str) -> Option<MatchId> {
    key.strip_prefix("tacsim:match:")
        .filter(|rest| rest.chars().all(|c| c.is_ascii_digit()) && !rest.is_empty())
        .and_then(|rest| rest.parse::<u64>().ok())
        .map(MatchId)
}

// ---------------------------------------------------------------------------
// The two documents of a running match (16b cut AF.2; 16c N16)
// ---------------------------------------------------------------------------

/// **The arena snapshot's key** (16b cut AF.2): `tacsim:snapshot:<n>`.
#[must_use]
pub fn snapshot_key(match_id: MatchId) -> SmolStr {
    SmolStr::new(format!("tacsim:snapshot:{}", match_id.0))
}

/// **The live record's key** (16b cut AF.2): `tacsim:live:<n>`.
#[must_use]
pub fn live_key(match_id: MatchId) -> SmolStr {
    SmolStr::new(format!("tacsim:live:{}", match_id.0))
}

/// The match id `prefix` followed by digits and nothing else names, or `None`.
fn id_after(key: &str, prefix: &str) -> Option<MatchId> {
    key.strip_prefix(prefix)
        .filter(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
        .and_then(|rest| rest.parse::<u64>().ok())
        .map(MatchId)
}

/// The match id a snapshot key names (the exact form: digits only), or `None`
/// for any other key — what the boot reads its leftover snapshots by.
#[must_use]
pub fn snapshot_id_of_key(key: &str) -> Option<MatchId> {
    id_after(key, "tacsim:snapshot:")
}

/// The match id a live record's key names (the exact form: digits only), or
/// `None` for any other key.
#[must_use]
pub fn live_id_of_key(key: &str) -> Option<MatchId> {
    id_after(key, "tacsim:live:")
}

/// The live record's `record` word: a reader checks it first.
pub const LIVE_RECORD: &str = "tacsim.live";

/// **The live record's format** (16b cut AF.2): bumped by a shape change that
/// is not an added `#[serde(default)]` field — this module's head has the
/// policy — and a record of another format is refused by name.
// M6: immutable authored custody replaces the old stock-only seal/profile
// shape. Old live records must fail the outer version check before state decode.
/// M7 requires complete filed certification in retained sortie custody.
pub const LIVE_FORMAT: u32 = 3;

/// **What the world holds of one running match, as the boot would need it**
/// (16b cut AF.2): the document at [`live_key`].
///
/// The document is `{"record": "tacsim.live", "format": 2, "match": <n>,
/// "at_ms", "chunks_appended", "chunks_lost", "resumed_from_ms": null | <ms>,
/// "state": <one JSON string>, "state_digest": <64 lowercase base-16
/// characters>}`, where `state` is `serde_json::to_string` of the record and
/// the centre and `state_digest` is SHA-256 over its bytes. **The state is a
/// string for the reason the arena snapshot's is**
/// ([`crate::arena::snapshot`]'s head): the folds hold contact ids and the
/// record a seed, full-range `u64`s the store's Redis seam re-serialises and
/// does not promise to keep; a string comes back byte for byte, and the digest
/// proves it did.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveRecord {
    /// The match: the document's `match`, which must equal its key's and the
    /// record's own id.
    pub match_id: MatchId,
    /// The arena millisecond of the last snapshot written beside it, or `0`
    /// before the first — what 16b cut AF.3 checks against the snapshot's own
    /// `at_ms` (the two documents from one moment).
    pub at_ms: i64,
    /// Chunks appended to the tape so far (`MatchMirror::chunks_appended`).
    pub chunks_appended: u64,
    /// Chunks the tape refused so far (`MatchMirror::chunks_lost`).
    pub chunks_lost: u64,
    /// The snapshot moment the match was last resumed from, or `None` for a
    /// match never resumed. 16b cut AF.3's resume writes it (the resume-once
    /// rule's memory: a boot never resumes a match again from the very
    /// snapshot it was last resumed from).
    pub resumed_from_ms: Option<i64>,
    /// The match's record, folds and replay fold included.
    pub record: MatchRecord,
    /// The centre as it stood — the lobby's, when it is running this match,
    /// and otherwise the one `lobby::adopt` binds for a staff `@arena/start`.
    pub centre: crate::match_::lobby::Centre,
}

/// **Why a live record was refused** (16b cut AF.2). Each `Display` starts
/// `the live record does not decode:`, the sentence 16b cut AF.3's void reason
/// prints whole.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LiveRecordError {
    /// The document's `record` is not [`LIVE_RECORD`].
    #[error("the live record does not decode: not a live record (record {found})")]
    Record {
        /// What the `record` key held, as JSON.
        found: String,
    },
    /// The document is a live record of another format.
    #[error("the live record does not decode: live record format {found} is not {expected}")]
    Format {
        /// The document's format.
        found: u64,
        /// [`LIVE_FORMAT`].
        expected: u32,
    },
    /// The state string is not the one the document was written with.
    #[error(
        "the live record does not decode: state digest mismatch: recorded {recorded}, computed \
         {computed}"
    )]
    DigestMismatch {
        /// The document's `state_digest`.
        recorded: String,
        /// SHA-256 over the state string as given.
        computed: String,
    },
    /// A missing or unknown key, a value of the wrong shape (`serde`'s own
    /// message), or a record whose id is not the document's match.
    #[error("the live record does not decode: {0}")]
    Malformed(String),
}

/// The live record's document, as `serde` reads and writes it: every key of
/// [`LiveRecord`]'s shape and no other.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveDocument<'a> {
    record: String,
    format: u32,
    #[serde(rename = "match")]
    match_id: MatchId,
    at_ms: i64,
    chunks_appended: u64,
    chunks_lost: u64,
    resumed_from_ms: Option<i64>,
    #[serde(borrow)]
    state: Cow<'a, str>,
    state_digest: String,
}

/// The state string's contents, borrowed for the write — the world loop pays
/// this every snapshot and must not clone the folds to do it.
#[derive(Serialize)]
struct LiveStateRef<'a> {
    record: &'a MatchRecord,
    centre: &'a crate::match_::lobby::Centre,
}

/// The state string's contents, owned for the read; the same two keys.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveState {
    record: MatchRecord,
    centre: crate::match_::lobby::Centre,
}

/// SHA-256 over `state`'s bytes, as every digest in the tree is written.
fn live_state_digest(state: &str) -> String {
    crate::arena::digest_text(&Sha256::digest(state.as_bytes()).into())
}

/// **Build a live document from borrows** — what [`LiveRecord::to_document`]
/// and [`write_live`] both write.
#[allow(clippy::too_many_arguments)]
fn live_document(
    match_id: MatchId,
    at_ms: i64,
    chunks_appended: u64,
    chunks_lost: u64,
    resumed_from_ms: Option<i64>,
    record: &MatchRecord,
    centre: &crate::match_::lobby::Centre,
) -> Result<serde_json::Value, LiveRecordError> {
    let state = serde_json::to_string(&LiveStateRef { record, centre })
        .map_err(|error| LiveRecordError::Malformed(error.to_string()))?;
    serde_json::to_value(LiveDocument {
        record: LIVE_RECORD.to_owned(),
        format: LIVE_FORMAT,
        match_id,
        at_ms,
        chunks_appended,
        chunks_lost,
        resumed_from_ms,
        state_digest: live_state_digest(&state),
        state: Cow::Borrowed(&state),
    })
    .map_err(|error| LiveRecordError::Malformed(error.to_string()))
}

impl LiveRecord {
    /// **The document at [`live_key`]** (this type's doc has its shape).
    ///
    /// # Errors
    ///
    /// [`LiveRecordError::Malformed`] when a value does not serialise, which no
    /// value of this tree does (every map in it is keyed by an integer or a
    /// string).
    pub fn to_document(&self) -> Result<serde_json::Value, LiveRecordError> {
        live_document(
            self.match_id,
            self.at_ms,
            self.chunks_appended,
            self.chunks_lost,
            self.resumed_from_ms,
            &self.record,
            &self.centre,
        )
    }

    /// **A live record back out of its document** (16b cut AF.2), checked in
    /// this order: the record word, the format, the document's own shape, the
    /// state digest (before the state is parsed), the state's shape, and that
    /// the record inside is the document's match. The caller compares
    /// [`LiveRecord::match_id`] with the key it read the document under (the V.2
    /// rule: a document whose own `match` disagrees with its key is not that
    /// match's).
    ///
    /// # Errors
    ///
    /// Each check refuses by its own [`LiveRecordError`] variant.
    pub fn from_document(document: &serde_json::Value) -> Result<LiveRecord, LiveRecordError> {
        let record = document.get("record");
        if record.and_then(serde_json::Value::as_str) != Some(LIVE_RECORD) {
            return Err(LiveRecordError::Record {
                found: record.map_or_else(|| "absent".to_owned(), ToString::to_string),
            });
        }
        let format = document.get("format").and_then(serde_json::Value::as_u64);
        if format != Some(u64::from(LIVE_FORMAT)) {
            return Err(LiveRecordError::Format {
                found: format.unwrap_or(0),
                expected: LIVE_FORMAT,
            });
        }
        let head = LiveDocument::deserialize(document)
            .map_err(|error| LiveRecordError::Malformed(error.to_string()))?;
        let computed = live_state_digest(&head.state);
        if computed != head.state_digest {
            return Err(LiveRecordError::DigestMismatch {
                recorded: head.state_digest,
                computed,
            });
        }
        let LiveState { record, centre } = serde_json::from_str(&head.state)
            .map_err(|error| LiveRecordError::Malformed(error.to_string()))?;
        if record.id != head.match_id {
            return Err(LiveRecordError::Malformed(format!(
                "the document is match {} and the record inside it is match {}",
                head.match_id, record.id
            )));
        }
        Ok(LiveRecord {
            match_id: head.match_id,
            at_ms: head.at_ms,
            chunks_appended: head.chunks_appended,
            chunks_lost: head.chunks_lost,
            resumed_from_ms: head.resumed_from_ms,
            record,
            centre,
        })
    }
}

/// **Write one match's live record** (16b cut AF.2) at [`live_key`], on the
/// module's own op lane.
///
/// Read off the registry's mirror: `at_ms` (the last snapshot's moment, `0`
/// before the first), the two chunk counters and `resumed_from_ms`; off the
/// book: the record and the centre — the book's own centre when it is on this
/// match, and otherwise the centre `lobby::adopt` binds for a staff
/// `@arena/start` (`Running`, on this match, nothing else). Serialised from
/// borrows: nothing of the record is cloned on the world loop.
///
/// **Three callers**: `crate::match_::start` at check-out, in the action that
/// writes the markers; `crate::arena_bridge`'s `on_snapshot`, beside the
/// snapshot document in one action; and the two writers that park the record in
/// `Stopping` (`crate::match_::abort` and `lobby::unattended_deadline`), so a
/// stop in flight at a crash is found by the boot as stopping and is never
/// undone by a resume (16b cut AF.3). A match the book does not hold writes
/// nothing; a record that does not serialise (which none does) is logged.
pub fn write_live(ctx: &mut Ctx<'_>, match_id: MatchId) {
    let (at_ms, chunks_appended, chunks_lost, resumed_from_ms) = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.mirror(match_id))
        .map_or((0, 0, 0, None), |mirror| {
            (
                mirror.snapshot_at_ms.unwrap_or(0),
                mirror.chunks_appended,
                mirror.chunks_lost,
                mirror.resumed_from_ms,
            )
        });
    let Some(book) = ctx.w.module_state::<super::MatchBook>() else {
        return;
    };
    let Some(record) = book.get(match_id) else {
        return;
    };
    let adopted;
    let centre = if book.centre().match_id == Some(match_id) {
        book.centre()
    } else {
        adopted = crate::match_::lobby::Centre {
            state: MatchState::Running,
            match_id: Some(match_id),
            ..crate::match_::lobby::Centre::default()
        };
        &adopted
    };
    let written = live_document(
        match_id,
        at_ms,
        chunks_appended,
        chunks_lost,
        resumed_from_ms,
        record,
        centre,
    );
    match written {
        Ok(value) => crate::store::push_op(
            ctx.out,
            mush_core::ModuleWrite::JsonSet {
                key: live_key(match_id),
                value,
            },
        ),
        Err(error) => tracing::error!(
            match_id = match_id.0,
            %error,
            "tacsim: a match's live record could not be written; a restart would not find it"
        ),
    }
}

/// **Delete both documents of a running match** (16b cut AF.2): the live
/// record and the arena snapshot, on the module's own op lane — by
/// `crate::match_::finish` in the result's action on every end, a void
/// included, and by the boot for every leftover pair it does not resume.
pub fn delete_live_and_snapshot(ctx: &mut Ctx<'_>, match_id: MatchId) {
    for key in [live_key(match_id), snapshot_key(match_id)] {
        crate::store::push_op(ctx.out, mush_core::ModuleWrite::Del { key });
    }
}

// ---------------------------------------------------------------------------
// The history ring (16b cut V.2; 16c N2, the result half)
// ---------------------------------------------------------------------------

/// **How many finished matches the centre remembers** (16c N2, owner default).
///
/// Past [`mush_core::MAX_STORE_READ_KEYS`] on purpose: the ring is never read
/// from the store. Every result document is one of the module's boot documents,
/// so the boot fills the ring ([`crate::match_::MatchBook::remember`]) and every
/// result write keeps it current; `SIM HISTORY` is a read of RAM.
pub const HISTORY_DEPTH: usize = 20;

/// **A report document's key inside the tacsim namespace** (16b cut V.2 defines
/// it; 16b cut V.3 writes the documents): `tacsim:report:<match>:<unit>`, one per
/// human pilot's unit.
///
/// Defined here, beside [`result_key`], because the ring is what bounds them: a
/// row the ring evicts has every report named on its [`HistoryRow::reports`]
/// deleted in the same write that evicted it.
#[must_use]
pub fn report_key(match_id: MatchId, unit: ObjId) -> SmolStr {
    SmolStr::new(format!("tacsim:report:{}:{}", match_id.0, unit.0))
}

/// **One unit's line of a remembered match** (16b cut V.2), as the result
/// document recorded it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryUnit {
    /// The unit.
    pub unit: ObjId,
    /// Its side. `None` for a unit on no roster — a restart abort's, whose
    /// document has `"teams": []`.
    pub team: Option<Team>,
    /// Its callsign (`RA`), or empty on a document written before 16b cut V.2
    /// and on a restart abort's rows.
    pub callsign: SmolStr,
    /// The first pilot the roster seated on it, or `None` for a machine.
    ///
    /// The first and not a list: every centre pod is one cockpit seat holding all
    /// three channels (doc 16 §3.7), so a centre unit has at most one pilot. A
    /// staff `@arena/start` crew of two reads as the first of them.
    pub pilot: Option<ObjId>,
    /// How it came back.
    pub outcome: UnitOutcome,
    /// Shots it fired.
    pub shots: u32,
    /// Shots of those that took something off something.
    pub hits: u32,
}

/// **One finished match, as the centre remembers it** (16b cut V.2) — the facts
/// of one `tacsim:match:<id>` document, read once by
/// [`HistoryRow::from_document`] and kept on
/// [`crate::match_::MatchBook`]'s ring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRow {
    /// The match.
    pub id: MatchId,
    /// Its outcome.
    pub outcome: MatchOutcome,
    /// The result's reason word (`elimination`, `conceded`, `restart`, …).
    pub reason: SmolStr,
    /// The arena definition's id (empty on a restart abort).
    pub arena: SmolStr,
    /// When it started, world milliseconds (`0` on a restart abort).
    pub started_ms: i64,
    /// When its result was written, world milliseconds.
    pub ended_ms: i64,
    /// The arena's own clock at the end (`0` on a restart abort).
    pub arena_ms: i64,
    /// The ledger's log id (`match.<id>`).
    pub log: String,
    /// The ledger's digest.
    pub digest: String,
    /// How many chunks the tape has (`0` on a document written before 16b cut
    /// V.2 and on a restart abort).
    pub chunks: u64,
    /// Staff-only, as on the document (16b cut M.3): nothing a player reads may
    /// print it.
    pub seed: u64,
    /// One line per unit row of the document, in its order.
    pub units: Vec<HistoryUnit>,
    /// Whether every unit row carried a callsign — false for a document written
    /// before 16b cut V.2, and for a restart abort.
    pub named: bool,
    /// The units a report document was written for (16b cut V.3 writes the key;
    /// absent reads empty).
    pub reports: Vec<ObjId>,
    /// **Whether a replay document was written with it** (16b cut I.1; absent
    /// reads `false`, which is every result written before the cut — and what
    /// `SIM REPLAY` answers `recorded before replays existed` for).
    pub replay: bool,
    /// **The rule it was fought under** (16b cut O.1; plan item 19), or `None`
    /// where the document carries no `"mode"` — a restart abort's, whose boot
    /// did not know it, and every result written before the cut. A mode other
    /// than [`MatchMode::Skirmish`] is tagged on the ring's line and the
    /// report's header (` [hold]`), so a match fought under another rule is
    /// never read as a plain fight.
    pub mode: Option<MatchMode>,
}

impl HistoryRow {
    /// **Read one result document** (16b cut V.2): `None` unless its `"record"`
    /// is `tacsim.match` and it names a match id and a known outcome word.
    ///
    /// Total over everything else, because a document written before this cut
    /// is still a result the centre should list:
    ///
    /// - each `units[]` row gives a [`HistoryUnit`] (`unit`, `outcome`, `shots`,
    ///   `hits`, `callsign`); a row with no unit id or an unknown outcome word is
    ///   skipped, and its absence makes the row unnamed;
    /// - a unit's side is the `teams[]` entry whose `units` holds it, and `None`
    ///   when none does (a restart abort's `"teams": []`); its pilot is the first
    ///   `teams[].pilots[]` row whose `unit` is it;
    /// - `named` is true when every unit row carried a non-empty `callsign`;
    /// - `ledger.chunks` absent reads `0`, `reports` absent reads empty,
    ///   `replay` absent reads `false` (16b cut I.1), `mode` absent or unknown
    ///   reads `None` (16b cut O.1), and every other absent word or number reads
    ///   empty or `0`.
    #[must_use]
    pub fn from_document(value: &serde_json::Value) -> Option<HistoryRow> {
        use serde_json::Value;
        if value.get("record").and_then(Value::as_str) != Some("tacsim.match") {
            return None;
        }
        let id = MatchId(value.get("match").and_then(Value::as_u64)?);
        let outcome: MatchOutcome =
            serde_json::from_value(value.get("outcome").cloned().unwrap_or(Value::Null)).ok()?;
        let word =
            |key: &str| SmolStr::new(value.get(key).and_then(Value::as_str).unwrap_or_default());
        let number = |key: &str| value.get(key).and_then(Value::as_i64).unwrap_or(0);
        let object = |value: &Value| {
            value
                .as_u64()
                .and_then(|id| u32::try_from(id).ok())
                .map(ObjId)
        };
        let empty = Vec::new();
        let teams = value
            .get("teams")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        let team_of = |unit: ObjId| {
            teams
                .iter()
                .find(|team| {
                    team.get("units")
                        .and_then(Value::as_array)
                        .is_some_and(|units| units.iter().any(|held| object(held) == Some(unit)))
                })
                .and_then(|team| serde_json::from_value::<Team>(team.get("team")?.clone()).ok())
        };
        let pilot_of = |unit: ObjId| {
            teams
                .iter()
                .filter_map(|team| team.get("pilots").and_then(Value::as_array))
                .flatten()
                .find(|row| row.get("unit").and_then(object) == Some(unit))
                .and_then(|row| row.get("pilot").and_then(object))
        };
        let rows = value
            .get("units")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        let mut named = true;
        let mut units: Vec<HistoryUnit> = Vec::with_capacity(rows.len());
        for row in rows {
            let parsed = row.get("unit").and_then(object).and_then(|unit| {
                let outcome: UnitOutcome =
                    serde_json::from_value(row.get("outcome")?.clone()).ok()?;
                let count = |key: &str| {
                    row.get(key)
                        .and_then(Value::as_u64)
                        .and_then(|count| u32::try_from(count).ok())
                        .unwrap_or(0)
                };
                Some(HistoryUnit {
                    unit,
                    team: team_of(unit),
                    callsign: SmolStr::new(
                        row.get("callsign")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    ),
                    pilot: pilot_of(unit),
                    outcome,
                    shots: count("shots"),
                    hits: count("hits"),
                })
            });
            match parsed {
                Some(unit) => {
                    named &= !unit.callsign.is_empty();
                    units.push(unit);
                }
                None => named = false,
            }
        }
        let ledger = value.get("ledger");
        let ledger_word = |key: &str| {
            ledger
                .and_then(|ledger| ledger.get(key))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Some(HistoryRow {
            id,
            outcome,
            reason: word("reason"),
            arena: word("arena"),
            started_ms: number("started_ms"),
            ended_ms: number("ended_ms"),
            arena_ms: number("arena_ms"),
            log: ledger_word("log"),
            digest: ledger_word("digest"),
            chunks: ledger
                .and_then(|ledger| ledger.get("chunks"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            seed: value.get("seed").and_then(Value::as_u64).unwrap_or(0),
            units,
            named,
            reports: value
                .get("reports")
                .and_then(Value::as_array)
                .map(|reports| reports.iter().filter_map(object).collect())
                .unwrap_or_default(),
            replay: value
                .get("replay")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            mode: value
                .get("mode")
                .and_then(Value::as_str)
                .and_then(MatchMode::parse),
        })
    }

    /// **The ` [hold]` tag** (16b cut O.1; plan item 19): ` [{mode}]` for a row
    /// fought under a mode other than [`MatchMode::Skirmish`], and nothing for a
    /// skirmish or a row that does not say. The ring's line and the report's
    /// header append it after the outcome words.
    #[must_use]
    pub fn mode_tag(&self) -> String {
        mode_tag(self.mode)
    }

    /// The unit `actor` piloted in this match, if they piloted one.
    #[must_use]
    pub fn pilot_unit(&self, actor: ObjId) -> Option<&HistoryUnit> {
        self.units.iter().find(|unit| unit.pilot == Some(actor))
    }

    /// **One side's losses**, as `(destroyed or disabled, of)` over the units on
    /// that side. A teamless unit (a restart abort's) is on neither.
    #[must_use]
    pub fn losses(&self, team: Team) -> (u32, u32) {
        let side = self.units.iter().filter(|unit| unit.team == Some(team));
        let (mut lost, mut of) = (0_u32, 0_u32);
        for unit in side {
            of = of.saturating_add(1);
            if matches!(unit.outcome, UnitOutcome::Destroyed | UnitOutcome::Disabled) {
                lost = lost.saturating_add(1);
            }
        }
        (lost, of)
    }

    /// How many units fought on `team` — the `2` of `SIM HISTORY`'s `2 v 2`. A
    /// teamless unit counts on neither side, so a restart row reads `0 v 0`.
    #[must_use]
    pub fn side_size(&self, team: Team) -> u32 {
        self.losses(team).1
    }
}

// ---------------------------------------------------------------------------
// The words (16b cut V.2): pure, pinned, and what V.3's report reads too
// ---------------------------------------------------------------------------

/// **A world second as a civil UTC minute** (16b cut V.2): `YYYY-MM-DD HH:MM
/// UTC`. A negative second clamps to the epoch.
///
/// The days-to-civil step is Howard Hinnant's `civil_from_days`, inline: the
/// proleptic Gregorian calendar over 400-year eras, exact for every `i64`
/// second a world clock can hold, with no calendar dependency for one line.
#[must_use]
pub fn civil_utc(secs: i64) -> String {
    let secs = secs.max(0);
    let days = secs.div_euclid(86_400);
    let of_day = secs.rem_euclid(86_400);
    let (hour, minute) = (of_day / 3_600, of_day % 3_600 / 60);
    // `civil_from_days`, shifted so the era starts on 0000-03-01.
    let z = days.saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146_096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11], March first
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = yoe
        .saturating_add(era.saturating_mul(400))
        .saturating_add(i64::from(month <= 2));
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

/// **A length of arena time in words** (16b cut V.2): `4 m 32 s`, `12 s` under a
/// minute, `0 s` for nothing, `1 h 2 m 3 s` past an hour. Whole seconds, rounded
/// down; a negative length reads `0 s`.
#[must_use]
pub fn duration_words(ms: i64) -> String {
    let secs = ms.max(0) / 1_000;
    let (hours, minutes, seconds) = (secs / 3_600, secs % 3_600 / 60, secs % 60);
    if hours > 0 {
        format!("{hours} h {minutes} m {seconds} s")
    } else if minutes > 0 {
        format!("{minutes} m {seconds} s")
    } else {
        format!("{seconds} s")
    }
}

/// **The one table both word functions read** (16b cut V.2): an outcome and its
/// reason word as a head and, where the pair has one, the bracketed end.
///
/// Closed: every pair a result document can carry is a row, and any other pair
/// — a document from a build with a word this one does not know — falls to the
/// two words as written.
fn outcome_table(outcome: MatchOutcome, reason: &str) -> (&'static str, Option<String>) {
    let bracket = |text: &str| Some(text.to_owned());
    match (outcome, reason) {
        (MatchOutcome::Red, "elimination") => ("red won", bracket("blue eliminated")),
        (MatchOutcome::Blue, "elimination") => ("blue won", bracket("red eliminated")),
        (MatchOutcome::Red, "conceded") => ("red won", bracket("blue conceded")),
        (MatchOutcome::Blue, "conceded") => ("blue won", bracket("red conceded")),
        // 16b cut O.1 (16c N11).
        (MatchOutcome::Red, "hold") => ("red won", bracket("red held the key spot")),
        (MatchOutcome::Blue, "hold") => ("blue won", bracket("blue held the key spot")),
        (MatchOutcome::Draw, "spent") => ("draw", bracket("both sides out of ammunition")),
        (MatchOutcome::Draw, "elimination") => ("draw", bracket("both sides eliminated")),
        (MatchOutcome::Timeout, "unattended") => ("timed out", bracket("every pilot had left")),
        (MatchOutcome::Timeout, "timeout") => ("timed out", bracket("the clock ran out")),
        (MatchOutcome::Abort, "abort") => ("aborted", None),
        (MatchOutcome::Abort, "restart") => ("aborted", bracket("the server restarted")),
        (MatchOutcome::Void, "void") => ("void", None),
        (MatchOutcome::Void, "ledger_overflow") => ("void", bracket("the tape overflowed")),
        (outcome, reason) => (outcome.as_str(), bracket(reason)),
    }
}

/// **The ` [hold]` tag for a mode word** (16b cut O.1; plan items 19, 39):
/// ` [{mode}]` for a mode other than [`MatchMode::Skirmish`], and the empty
/// string for a skirmish or for no mode at all — what the ring's line, the
/// report's header and the replay's head each append after the outcome words.
/// **16b cut O.3**'s range reads ` [range]` through it with no code of its own,
/// so a range is never read as a fight anywhere it is shown.
#[must_use]
pub fn mode_tag(mode: Option<MatchMode>) -> String {
    match mode {
        Some(mode) if mode != MatchMode::Skirmish => format!(" [{}]", mode.as_str()),
        _ => String::new(),
    }
}

/// **How a match ended, in public words** (16b cut V.2): `red won (blue
/// eliminated)`, `aborted (the server restarted)`, `void`. Written as `<head>
/// (<end_words>)` over one table, so this and [`end_words`] cannot drift.
#[must_use]
pub fn outcome_words(outcome: MatchOutcome, reason: &str) -> String {
    match outcome_table(outcome, reason) {
        (head, Some(end)) => format!("{head} ({end})"),
        (head, None) => head.to_owned(),
    }
}

/// **The bracketed part of [`outcome_words`] alone** (16b cut V.2): `red
/// eliminated`, `both sides out of ammunition`; for a pair with no bracket
/// (`abort`, `void`), the reason word as written. 16b cut V.3's end line reads
/// it.
#[must_use]
pub fn end_words(outcome: MatchOutcome, reason: &str) -> String {
    outcome_table(outcome, reason)
        .1
        .unwrap_or_else(|| reason.to_owned())
}

/// **An arena's authored name** (16b cut V.2): the `label` of the loaded
/// `ArenaDef` with this id, or the id itself when the catalog has none (the
/// stand-in window, a pack since unloaded, a restart abort's empty id).
#[must_use]
pub fn arena_label(w: &mush_core::World, id: &str) -> String {
    w.module_state::<crate::store::TacsimStore>()
        .and_then(|slot| {
            mush_core::CanonicalId::new(id).ok().and_then(|entry| {
                slot.definitions()
                    .arenas
                    .get(&entry)
                    .map(|def| def.label.clone())
            })
        })
        .unwrap_or_else(|| id.to_owned())
}

/// The sentence `SIM HISTORY` answers with when the ring is empty.
pub const NO_HISTORY: &str = "The centre has no finished match on record.";

/// **`SIM HISTORY`'s lines** (16b cut V.2; 16c N2): a header, then one line per
/// remembered match, newest first as `rows` gives them.
///
/// ```text
/// The centre's last 3 matches, newest first:
///   #12  blue won (red eliminated)  4 m 32 s  2 v 2  2026-09-24 14:02 UTC  (you flew RA)
/// ```
///
/// The `#` is the **match id**. The fields are the outcome in words, the arena
/// time, red's size against blue's, and when the result was written; ` (you
/// flew RA)` closes a row `reader` piloted, and only when the document named
/// that unit — a row written before 16b cut V.2 has pilots and no callsigns. The
/// public outcome and nothing of the field (16c N2, doc 16 §1): no unit's object
/// number, no pilot's name, no position, no contact and never the seed.
///
/// **16b cut O.1**: a match fought under a mode other than the skirmish carries
/// its tag after the outcome words — `#3  red won (red held the key spot)
/// [hold]  2 m 0 s  …` — and a row that does not say its mode carries none.
#[must_use]
pub fn history_lines<'a>(rows: impl Iterator<Item = &'a HistoryRow>, reader: ObjId) -> Vec<String> {
    let rows: Vec<&HistoryRow> = rows.collect();
    let header = match rows.len() {
        0 => return vec![NO_HISTORY.to_owned()],
        1 => "The centre's last match:".to_owned(),
        n => format!("The centre's last {n} matches, newest first:"),
    };
    let mut lines = Vec::with_capacity(rows.len().saturating_add(1));
    lines.push(header);
    for row in rows {
        let mut line = format!(
            "  #{}  {}{}  {}  {} v {}  {}",
            row.id.0,
            outcome_words(row.outcome, &row.reason),
            // 16b cut O.1: ` [hold]` after the outcome words, for a match fought
            // under another rule.
            row.mode_tag(),
            duration_words(row.arena_ms),
            row.side_size(Team::Red),
            row.side_size(Team::Blue),
            civil_utc(row.ended_ms / 1_000),
        );
        if let Some(flown) = row
            .pilot_unit(reader)
            .filter(|unit| !unit.callsign.is_empty())
        {
            line.push_str(&format!("  (you flew {})", flown.callsign));
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_result_key_round_trips_and_the_counter_key_is_not_a_result() {
        assert_eq!(result_key(MatchId(12)).as_str(), "tacsim:match:12");
        assert_eq!(match_id_of_key("tacsim:match:12"), Some(MatchId(12)));
        assert_eq!(match_id_of_key(super::super::SEQ_KEY), None);
        assert_eq!(match_id_of_key("tacsim:match:"), None);
        assert_eq!(match_id_of_key("tacsim:pack:a@1"), None);
    }

    /// **16b cut M.3** (16c `T-07`): the result document carries the record's
    /// seed at its top level, and the restart-written one carries `0`.
    #[test]
    fn m3_the_result_carries_the_seed_and_the_restart_result_carries_zero() {
        let record = MatchRecord {
            id: MatchId(7),
            state: MatchState::Running,
            arena: SmolStr::new("tacsim.arena.test"),
            arena_definition_digest: "ab".to_owned(),
            teams: Vec::new(),
            started_ms: 1_000,
            ended: None,
            ledger_log_id: "match.7".to_owned(),
            ledger_digest: String::new(),
            seed: 2_350_235_023_502_350,
            last_pose: std::collections::BTreeMap::new(),
            squads: Vec::new(),
            leaders: Vec::new(),
            callsigns: BTreeMap::new(),
            folds: BTreeMap::new(),
            unparsed: 0,
            replay: None,
            mode: MatchMode::Skirmish,
            minds: BTreeMap::new(),
        };
        let end = MatchEnd {
            reason: EndReason::Timeout,
            t_ms: 90_000,
        };
        let written = result_document(
            Some(&record),
            MatchId(7),
            MatchOutcome::Timeout,
            end,
            "cd",
            &[],
            5_000,
            0,
            &[],
            false,
        );
        assert_eq!(
            written["seed"],
            serde_json::json!(2_350_235_023_502_350_u64)
        );
        assert_eq!(written["ledger"]["digest"], serde_json::json!("cd"));
        // The seed is a top-level key and nowhere else: it is not folded into
        // the ledger object a reader might hand on.
        assert!(written["ledger"].get("seed").is_none());
        // A record the book no longer holds has nothing to say.
        let orphan = result_document(
            None,
            MatchId(7),
            MatchOutcome::Timeout,
            end,
            "cd",
            &[],
            5_000,
            0,
            &[],
            false,
        );
        assert_eq!(orphan["seed"], serde_json::json!(0));
        // The restart result: no arena drew anything.
        let restart = restart_abort_document(MatchId(7), &[ObjId(4)], 5_000);
        assert_eq!(restart["seed"], serde_json::json!(0));
        assert_eq!(restart["reason"], serde_json::json!("restart"));
    }

    #[test]
    fn every_end_reason_has_a_state_and_an_outcome() {
        for (reason, state, outcome) in [
            (
                EndReason::Elimination { winner: None },
                MatchState::Ended,
                MatchOutcome::Draw,
            ),
            (
                EndReason::Elimination {
                    winner: Some(Team::Blue),
                },
                MatchState::Ended,
                MatchOutcome::Blue,
            ),
            // 16b cut Q.7: a draw that names no winner.
            (EndReason::Spent, MatchState::Ended, MatchOutcome::Draw),
            // 16b cut D.3 (`B-33`): an end with a winner, and an end with none —
            // neither of them an abort and neither of them a void.
            (
                EndReason::Conceded { winner: Team::Red },
                MatchState::Ended,
                MatchOutcome::Red,
            ),
            (
                EndReason::Conceded { winner: Team::Blue },
                MatchState::Ended,
                MatchOutcome::Blue,
            ),
            // 16b cut O.1: a hold names its winner, and ends like any fight.
            (
                EndReason::Hold { winner: Team::Red },
                MatchState::Ended,
                MatchOutcome::Red,
            ),
            (
                EndReason::Unattended,
                MatchState::Ended,
                MatchOutcome::Timeout,
            ),
            (EndReason::Timeout, MatchState::Ended, MatchOutcome::Timeout),
            (EndReason::Abort, MatchState::Aborted, MatchOutcome::Abort),
            (EndReason::Void, MatchState::Void, MatchOutcome::Void),
            (
                EndReason::LedgerOverflow,
                MatchState::Void,
                MatchOutcome::Void,
            ),
        ] {
            assert_eq!(MatchState::from(reason), state);
            assert_eq!(MatchOutcome::from(reason), outcome);
            assert!(!MatchState::from(reason).is_live());
        }
        assert!(MatchState::Running.is_live());
        assert!(MatchState::Stopping.is_live());
        assert!(MatchState::Boarding.may_start());
        assert!(!MatchState::Running.may_start());
    }

    // -----------------------------------------------------------------------
    // 16b cut V.2 — the result reader, the ring and the words
    // -----------------------------------------------------------------------

    use crate::arena::state::Designator;

    /// A running record of a 2 v 1: red 101 (pilot 5, `RA`) and 102 (a machine,
    /// `RB`), blue 201 (pilot 6, `BA`).
    fn v2_record() -> MatchRecord {
        let slot = |team: Team, slot: u16| Designator::Slot { team, slot };
        MatchRecord {
            id: MatchId(12),
            state: MatchState::Running,
            arena: SmolStr::new("tacsim.arena.proving_ground"),
            arena_definition_digest: "ab".to_owned(),
            teams: vec![
                TeamRoster {
                    team: Team::Red,
                    units: vec![ObjId(101), ObjId(102)],
                    pilots: vec![(ObjId(5), ObjId(101))],
                },
                TeamRoster {
                    team: Team::Blue,
                    units: vec![ObjId(201)],
                    pilots: vec![(ObjId(6), ObjId(201))],
                },
            ],
            started_ms: 1_000,
            ended: None,
            ledger_log_id: "match.12".to_owned(),
            ledger_digest: String::new(),
            seed: 2_350_235_023_502_350,
            last_pose: BTreeMap::new(),
            squads: Vec::new(),
            leaders: Vec::new(),
            callsigns: BTreeMap::from([
                (ObjId(101), slot(Team::Red, 1)),
                (ObjId(102), slot(Team::Red, 2)),
                (ObjId(201), slot(Team::Blue, 1)),
            ]),
            folds: BTreeMap::new(),
            unparsed: 0,
            replay: None,
            mode: MatchMode::Skirmish,
            minds: BTreeMap::new(),
        }
    }

    /// The rows `finish` would build for [`v2_record`]: red's pilot won, red's
    /// machine was disabled, blue's tank is a wreck.
    fn v2_rows(record: &MatchRecord) -> Vec<UnitResult> {
        vec![
            UnitResult {
                unit: 101,
                callsign: record.callsign_word(ObjId(101)),
                outcome: UnitOutcome::Intact,
                shots: 9,
                hits: 4,
            },
            UnitResult {
                unit: 102,
                callsign: record.callsign_word(ObjId(102)),
                outcome: UnitOutcome::Disabled,
                shots: 3,
                hits: 1,
            },
            UnitResult {
                unit: 201,
                callsign: record.callsign_word(ObjId(201)),
                outcome: UnitOutcome::Destroyed,
                shots: 7,
                hits: 2,
            },
        ]
    }

    fn v2_document() -> serde_json::Value {
        let record = v2_record();
        result_document(
            Some(&record),
            MatchId(12),
            MatchOutcome::Red,
            MatchEnd {
                reason: EndReason::Elimination {
                    winner: Some(Team::Red),
                },
                t_ms: 272_000,
            },
            "cd",
            &v2_rows(&record),
            1_790_258_520_000,
            42,
            &[],
            false,
        )
    }

    /// **16b cut V.2**: the document names every unit and every pilot's unit by
    /// its callsign, counts the tape under `ledger`, and reads back into one
    /// [`HistoryRow`] with every fact in place — the seed included, for staff.
    #[test]
    fn v2_a_result_document_round_trips_into_a_history_row() {
        let written = v2_document();
        assert_eq!(written["units"][0]["callsign"], serde_json::json!("RA"));
        assert_eq!(written["units"][1]["callsign"], serde_json::json!("RB"));
        assert_eq!(written["units"][2]["callsign"], serde_json::json!("BA"));
        assert_eq!(
            written["teams"][0]["pilots"][0],
            serde_json::json!({ "pilot": 5, "unit": 101, "callsign": "RA" })
        );
        assert_eq!(
            written["teams"][1]["pilots"][0],
            serde_json::json!({ "pilot": 6, "unit": 201, "callsign": "BA" })
        );
        assert_eq!(written["ledger"]["chunks"], serde_json::json!(42));
        assert_eq!(written["record_version"], serde_json::json!(1));

        let row = HistoryRow::from_document(&written).expect("a result document");
        assert_eq!(row.id, MatchId(12));
        assert_eq!(row.outcome, MatchOutcome::Red);
        assert_eq!(row.reason.as_str(), "elimination");
        assert_eq!(row.arena.as_str(), "tacsim.arena.proving_ground");
        assert_eq!(
            (row.started_ms, row.ended_ms, row.arena_ms),
            (1_000, 1_790_258_520_000, 272_000)
        );
        assert_eq!((row.log.as_str(), row.digest.as_str()), ("match.12", "cd"));
        assert_eq!(row.chunks, 42);
        assert_eq!(row.seed, 2_350_235_023_502_350);
        assert!(row.named);
        assert!(row.reports.is_empty(), "written with no report");
        assert_eq!(
            row.units,
            vec![
                HistoryUnit {
                    unit: ObjId(101),
                    team: Some(Team::Red),
                    callsign: SmolStr::new("RA"),
                    pilot: Some(ObjId(5)),
                    outcome: UnitOutcome::Intact,
                    shots: 9,
                    hits: 4,
                },
                HistoryUnit {
                    unit: ObjId(102),
                    team: Some(Team::Red),
                    callsign: SmolStr::new("RB"),
                    pilot: None,
                    outcome: UnitOutcome::Disabled,
                    shots: 3,
                    hits: 1,
                },
                HistoryUnit {
                    unit: ObjId(201),
                    team: Some(Team::Blue),
                    callsign: SmolStr::new("BA"),
                    pilot: Some(ObjId(6)),
                    outcome: UnitOutcome::Destroyed,
                    shots: 7,
                    hits: 2,
                },
            ]
        );
        // The readers.
        assert_eq!(
            row.pilot_unit(ObjId(5)).map(|unit| unit.unit),
            Some(ObjId(101))
        );
        assert_eq!(
            row.pilot_unit(ObjId(6)).map(|unit| unit.unit),
            Some(ObjId(201))
        );
        assert_eq!(row.pilot_unit(ObjId(7)), None, "a bystander flew nothing");
        assert_eq!(row.side_size(Team::Red), 2);
        assert_eq!(row.side_size(Team::Blue), 1);
        assert_eq!(row.losses(Team::Red), (1, 2), "disabled is a loss");
        assert_eq!(row.losses(Team::Blue), (1, 1), "destroyed is a loss");

        // `reports`, when a later cut writes it, is read back.
        let mut reported = written.clone();
        reported["reports"] = serde_json::json!([101, 201]);
        assert_eq!(
            HistoryRow::from_document(&reported).map(|row| row.reports),
            Some(vec![ObjId(101), ObjId(201)])
        );
        // Not a result document: no row.
        assert_eq!(
            HistoryRow::from_document(&serde_json::json!({ "next_id": 12 })),
            None
        );
        let mut foreign = written;
        foreign["record"] = serde_json::json!("tacsim.report");
        assert_eq!(HistoryRow::from_document(&foreign), None);
    }

    /// **16b cut V.2**: a document written before this cut — the owner's dev1
    /// matches — has pilot rows and no callsign and no chunk count, and reads as
    /// an unnamed row whose pilots are still bound to their units.
    #[test]
    fn v2_a_document_without_callsigns_reads_unnamed() {
        let mut old = v2_document();
        for row in old["units"].as_array_mut().expect("unit rows") {
            row.as_object_mut().expect("a unit row").remove("callsign");
        }
        for team in old["teams"].as_array_mut().expect("teams") {
            for pilot in team["pilots"].as_array_mut().expect("pilot rows") {
                pilot
                    .as_object_mut()
                    .expect("a pilot row")
                    .remove("callsign");
            }
        }
        old["ledger"]
            .as_object_mut()
            .expect("the ledger")
            .remove("chunks");
        let row = HistoryRow::from_document(&old).expect("still a result document");
        assert!(!row.named);
        assert_eq!(row.chunks, 0);
        assert!(row.units.iter().all(|unit| unit.callsign.is_empty()));
        assert_eq!(
            row.pilot_unit(ObjId(5)).map(|unit| unit.unit),
            Some(ObjId(101))
        );
        // One named row and one unnamed is still unnamed.
        let mut half = v2_document();
        half["units"][1]["callsign"] = serde_json::json!("");
        assert_eq!(
            HistoryRow::from_document(&half).map(|row| row.named),
            Some(false)
        );
    }

    /// **16b cut V.2**: the boot's `aborted(restart)` document reads as reason
    /// `restart`, every unit unnamed and on no side, `0 v 0`.
    #[test]
    fn v2_a_restart_abort_reads_teamless_and_unnamed() {
        let written = restart_abort_document(MatchId(10), &[ObjId(101), ObjId(201)], 5_000);
        assert_eq!(written["units"][0]["callsign"], serde_json::json!(""));
        assert_eq!(written["ledger"]["chunks"], serde_json::json!(0));
        let row = HistoryRow::from_document(&written).expect("a result document");
        assert_eq!(row.id, MatchId(10));
        assert_eq!(row.outcome, MatchOutcome::Abort);
        assert_eq!(row.reason.as_str(), "restart");
        assert_eq!(row.units.len(), 2);
        assert!(
            row.units.iter().all(|unit| unit.team.is_none()
                && unit.callsign.is_empty()
                && unit.pilot.is_none())
        );
        assert!(!row.named);
        assert_eq!(
            (row.side_size(Team::Red), row.side_size(Team::Blue)),
            (0, 0)
        );
        assert_eq!(row.losses(Team::Red), (0, 0));
        assert_eq!(row.chunks, 0);
        assert_eq!(row.seed, 0);
    }

    /// A bare row with an id, for the ring.
    fn bare_row(id: u64) -> HistoryRow {
        HistoryRow {
            id: MatchId(id),
            outcome: MatchOutcome::Abort,
            reason: SmolStr::new("abort"),
            arena: SmolStr::default(),
            started_ms: 0,
            ended_ms: 0,
            arena_ms: 0,
            log: String::new(),
            digest: String::new(),
            chunks: 0,
            seed: 0,
            units: Vec::new(),
            named: true,
            reports: Vec::new(),
            replay: false,
            mode: None,
        }
    }

    fn ring_ids(book: &super::super::MatchBook) -> Vec<u64> {
        book.history().map(|row| row.id.0).collect()
    }

    /// **16b cut V.2**: the ring orders by id whatever the order of the calls,
    /// replaces a row of the same id, and at [`HISTORY_DEPTH`] answers the row
    /// that fell off — the lowest id.
    #[test]
    fn v2_the_ring_orders_by_id_replaces_and_truncates() {
        let mut book = super::super::MatchBook::new();
        assert_eq!(book.history().count(), 0);
        for id in [3, 1, 2] {
            assert_eq!(book.remember(bare_row(id)), None);
        }
        assert_eq!(
            ring_ids(&book),
            vec![3, 2, 1],
            "descending, whatever the order"
        );
        // The same id again: replaced in place, not listed twice.
        let mut again = bare_row(2);
        again.reason = SmolStr::new("restart");
        assert_eq!(book.remember(again), None);
        assert_eq!(ring_ids(&book), vec![3, 2, 1]);
        assert_eq!(
            book.remembered(MatchId(2)).map(|row| row.reason.as_str()),
            Some("restart")
        );
        assert_eq!(book.remembered(MatchId(9)), None);
        // Up to the depth, nothing falls off.
        for id in 4..=20 {
            assert_eq!(book.remember(bare_row(id)), None, "row {id}");
        }
        assert_eq!(book.history().count(), HISTORY_DEPTH);
        // The twenty-first: the oldest goes, and is answered.
        let mut reported = bare_row(1);
        reported.reports = vec![ObjId(101)];
        assert_eq!(book.remember(reported.clone()), None, "a replacement");
        assert_eq!(book.remember(bare_row(21)), Some(reported));
        assert_eq!(book.history().count(), HISTORY_DEPTH);
        assert_eq!(ring_ids(&book).first(), Some(&21));
        assert_eq!(ring_ids(&book).last(), Some(&2));
        assert_eq!(book.remembered(MatchId(1)), None);
        // A row older than every row of a full ring falls straight off.
        assert_eq!(
            book.remember(bare_row(1)).map(|row| row.id),
            Some(MatchId(1))
        );
        assert_eq!(ring_ids(&book).last(), Some(&2));
        // The report key the eviction deletes.
        assert_eq!(
            report_key(MatchId(1), ObjId(101)).as_str(),
            "tacsim:report:1:101"
        );
        assert_eq!(
            match_id_of_key(report_key(MatchId(1), ObjId(101)).as_str()),
            None,
            "a report key is not a result key"
        );
    }

    #[test]
    fn v2_civil_utc_is_pinned() {
        assert_eq!(civil_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(civil_utc(1_000_000_000), "2001-09-09 01:46 UTC");
        assert_eq!(civil_utc(1_700_000_000), "2023-11-14 22:13 UTC");
        assert_eq!(civil_utc(-5), "1970-01-01 00:00 UTC", "a negative clamps");
        // And the calendar's corners: a leap day, a century that is not one.
        assert_eq!(civil_utc(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(civil_utc(4_102_444_800), "2100-01-01 00:00 UTC");
        assert_eq!(civil_utc(1_790_258_520), "2026-09-24 14:02 UTC");
    }

    #[test]
    fn v2_duration_words_are_pinned() {
        assert_eq!(duration_words(272_000), "4 m 32 s");
        assert_eq!(duration_words(272_999), "4 m 32 s", "whole seconds, down");
        assert_eq!(duration_words(0), "0 s");
        assert_eq!(duration_words(-1), "0 s");
        assert_eq!(duration_words(12_000), "12 s");
        assert_eq!(duration_words(3_723_000), "1 h 2 m 3 s");
        assert_eq!(duration_words(60_000), "1 m 0 s");
    }

    #[test]
    fn v2_outcome_words_and_end_words_are_one_table() {
        for (outcome, reason, words, end) in [
            (
                MatchOutcome::Red,
                "elimination",
                "red won (blue eliminated)",
                "blue eliminated",
            ),
            (
                MatchOutcome::Blue,
                "elimination",
                "blue won (red eliminated)",
                "red eliminated",
            ),
            (
                MatchOutcome::Red,
                "conceded",
                "red won (blue conceded)",
                "blue conceded",
            ),
            (
                MatchOutcome::Blue,
                "conceded",
                "blue won (red conceded)",
                "red conceded",
            ),
            (
                MatchOutcome::Draw,
                "spent",
                "draw (both sides out of ammunition)",
                "both sides out of ammunition",
            ),
            (
                MatchOutcome::Draw,
                "elimination",
                "draw (both sides eliminated)",
                "both sides eliminated",
            ),
            (
                MatchOutcome::Timeout,
                "unattended",
                "timed out (every pilot had left)",
                "every pilot had left",
            ),
            (
                MatchOutcome::Timeout,
                "timeout",
                "timed out (the clock ran out)",
                "the clock ran out",
            ),
            (MatchOutcome::Abort, "abort", "aborted", "abort"),
            (
                MatchOutcome::Abort,
                "restart",
                "aborted (the server restarted)",
                "the server restarted",
            ),
            (MatchOutcome::Void, "void", "void", "void"),
            (
                MatchOutcome::Void,
                "ledger_overflow",
                "void (the tape overflowed)",
                "the tape overflowed",
            ),
            // Any other pair: the two words as written.
            (MatchOutcome::Red, "timeout", "red (timeout)", "timeout"),
            (MatchOutcome::Draw, "wibble", "draw (wibble)", "wibble"),
        ] {
            assert_eq!(
                outcome_words(outcome, reason),
                words,
                "{outcome:?} {reason}"
            );
            assert_eq!(end_words(outcome, reason), end, "{outcome:?} {reason}");
        }
        // Every reason word the writer can produce has a row for its outcome.
        for reason in [
            EndReason::Elimination { winner: None },
            EndReason::Elimination {
                winner: Some(Team::Red),
            },
            EndReason::Spent,
            // 16b cut O.1.
            EndReason::Hold { winner: Team::Red },
            EndReason::Conceded { winner: Team::Blue },
            EndReason::Unattended,
            EndReason::Timeout,
            EndReason::Abort,
            EndReason::Void,
            EndReason::LedgerOverflow,
        ] {
            let outcome = MatchOutcome::from(reason);
            let words = outcome_words(outcome, reason_word(reason));
            assert_ne!(
                words,
                format!("{} ({})", outcome.as_str(), reason_word(reason)),
                "{reason:?} fell through the table"
            );
        }
    }

    /// **16b cut V.2**: `SIM HISTORY`'s lines, verbatim — the suffix on a row
    /// the reader flew, no suffix on a row they flew before callsigns were
    /// written, and the empty and one-row headers.
    #[test]
    fn v2_history_lines_are_verbatim() {
        let mut newest = HistoryRow::from_document(&v2_document()).expect("a row");
        // Make it the brief's `2 v 2` blue win.
        newest.outcome = MatchOutcome::Blue;
        newest.units.push(HistoryUnit {
            unit: ObjId(202),
            team: Some(Team::Blue),
            callsign: SmolStr::new("BB"),
            pilot: None,
            outcome: UnitOutcome::Intact,
            shots: 0,
            hits: 0,
        });
        let mut spent = bare_row(11);
        spent.outcome = MatchOutcome::Draw;
        spent.reason = SmolStr::new("spent");
        spent.arena_ms = 550_000;
        spent.ended_ms = 1_790_257_200_000;
        spent.units = vec![
            HistoryUnit {
                unit: ObjId(101),
                team: Some(Team::Red),
                callsign: SmolStr::new("RA"),
                pilot: Some(ObjId(7)),
                outcome: UnitOutcome::Intact,
                shots: 30,
                hits: 12,
            },
            HistoryUnit {
                unit: ObjId(201),
                team: Some(Team::Blue),
                callsign: SmolStr::new("BA"),
                pilot: Some(ObjId(8)),
                outcome: UnitOutcome::Intact,
                shots: 30,
                hits: 9,
            },
        ];
        let restart = HistoryRow::from_document(&restart_abort_document(
            MatchId(10),
            &[ObjId(101), ObjId(201)],
            1_790_254_920_000,
        ))
        .expect("a row");
        let rows = [newest, spent, restart];
        assert_eq!(
            history_lines(rows.iter(), ObjId(5)),
            vec![
                "The centre's last 3 matches, newest first:",
                "  #12  blue won (red eliminated)  4 m 32 s  2 v 2  2026-09-24 14:02 UTC  (you flew RA)",
                "  #11  draw (both sides out of ammunition)  9 m 10 s  1 v 1  2026-09-24 13:40 UTC",
                "  #10  aborted (the server restarted)  0 s  0 v 0  2026-09-24 13:02 UTC",
            ]
        );
        // A bystander reads the same rows and no suffix.
        let bystander = history_lines(rows.iter(), ObjId(99));
        assert!(bystander.iter().all(|line| !line.contains("you flew")));
        // A row flown before V.2: a pilot row, no callsign, no half a sentence.
        let mut unnamed = rows[0].clone();
        for unit in &mut unnamed.units {
            unit.callsign = SmolStr::default();
        }
        unnamed.named = false;
        assert_eq!(
            history_lines(std::iter::once(&unnamed), ObjId(5)),
            vec![
                "The centre's last match:",
                "  #12  blue won (red eliminated)  4 m 32 s  2 v 2  2026-09-24 14:02 UTC",
            ]
        );
        assert_eq!(
            history_lines(std::iter::empty(), ObjId(5)),
            vec![NO_HISTORY]
        );
        assert_eq!(NO_HISTORY, "The centre has no finished match on record.");
        // Nothing of the field: never the seed, never an object number.
        let seed = rows[0].seed.to_string();
        for line in history_lines(rows.iter(), ObjId(5)) {
            assert!(!line.contains(&seed), "{line}");
            for unit in ["101", "102", "201", "202"] {
                assert!(
                    line.split_whitespace()
                        .all(|word| word.trim_start_matches('#') != unit),
                    "{line}"
                );
            }
        }
    }

    /// **16b cut V.3**: the result document names the units whose report
    /// documents rode its batch, in the order given (id order, from `finish`),
    /// and reads them back; the restart abort names none.
    #[test]
    fn v3_the_result_names_its_reports_and_the_restart_abort_names_none() {
        let record = v2_record();
        let written = result_document(
            Some(&record),
            MatchId(12),
            MatchOutcome::Red,
            MatchEnd {
                reason: EndReason::Elimination {
                    winner: Some(Team::Red),
                },
                t_ms: 272_000,
            },
            "cd",
            &v2_rows(&record),
            1_790_258_520_000,
            42,
            &[ObjId(101), ObjId(201)],
            true,
        );
        assert_eq!(written["reports"], serde_json::json!([101, 201]));
        // 16b cut I.1: and the replay flag, read back; a restart writes none.
        assert_eq!(written["replay"], serde_json::json!(true));
        assert_eq!(
            HistoryRow::from_document(&written).map(|row| row.replay),
            Some(true)
        );
        assert_eq!(
            HistoryRow::from_document(&written).map(|row| row.reports),
            Some(vec![ObjId(101), ObjId(201)])
        );
        let restart = restart_abort_document(MatchId(13), &[ObjId(101)], 5_000);
        assert_eq!(restart["reports"], serde_json::json!([]));
        assert_eq!(restart["replay"], serde_json::json!(false));
        assert_eq!(
            HistoryRow::from_document(&restart).map(|row| row.replay),
            Some(false)
        );
        assert_eq!(
            HistoryRow::from_document(&restart).map(|row| row.reports),
            Some(Vec::new())
        );
        // A report key is the unit's, under the match's.
        assert_eq!(
            report_key(MatchId(12), ObjId(101)).as_str(),
            "tacsim:report:12:101"
        );
    }

    /// **16b cut O.1** (plan items 19, 39): a Hold's words and its tag — the
    /// result carries `mode`, the ring and the header append ` [hold]`, a
    /// skirmish tags nothing, and the restart document writes no `mode` and
    /// reads with no tag.
    #[test]
    fn o1_a_hold_is_worded_tagged_and_the_restart_row_is_not() {
        assert_eq!(
            outcome_words(MatchOutcome::Red, "hold"),
            "red won (red held the key spot)"
        );
        assert_eq!(
            outcome_words(MatchOutcome::Blue, "hold"),
            "blue won (blue held the key spot)"
        );
        assert_eq!(
            end_words(MatchOutcome::Red, "hold"),
            "red held the key spot"
        );
        assert_eq!(reason_word(EndReason::Hold { winner: Team::Blue }), "hold");
        assert_eq!(
            MatchOutcome::from(EndReason::Hold { winner: Team::Blue }),
            MatchOutcome::Blue
        );
        let mut record = v2_record();
        record.mode = MatchMode::Hold;
        let written = result_document(
            Some(&record),
            MatchId(3),
            MatchOutcome::Red,
            MatchEnd {
                reason: EndReason::Hold { winner: Team::Red },
                t_ms: 120_250,
            },
            "cd",
            &v2_rows(&record),
            1_790_258_520_000,
            42,
            &[],
            false,
        );
        assert_eq!(written["mode"], serde_json::json!("hold"));
        assert_eq!(written["reason"], serde_json::json!("hold"));
        let row = HistoryRow::from_document(&written).expect("a row");
        assert_eq!(row.mode, Some(MatchMode::Hold));
        assert_eq!(row.mode_tag(), " [hold]");
        let lines = history_lines(std::iter::once(&row), ObjId(99));
        assert_eq!(
            lines.get(1).map(String::as_str),
            Some(
                "  #3  red won (red held the key spot) [hold]  2 m 0 s  2 v 1  2026-09-24 14:02 UTC"
            )
        );
        // A skirmish writes its word and tags nothing.
        let skirmish = HistoryRow::from_document(&v2_document()).expect("a row");
        assert_eq!(skirmish.mode, Some(MatchMode::Skirmish));
        assert_eq!(skirmish.mode_tag(), "");
        // The restart document knows no mode, and says none.
        let restart = restart_abort_document(MatchId(4), &[ObjId(101)], 5_000);
        assert!(restart.get("mode").is_none());
        let row = HistoryRow::from_document(&restart).expect("a row");
        assert_eq!(row.mode, None);
        assert_eq!(row.mode_tag(), "");
        // A record the book no longer holds writes no key either.
        let orphan = result_document(
            None,
            MatchId(5),
            MatchOutcome::Abort,
            MatchEnd {
                reason: EndReason::Abort,
                t_ms: 0,
            },
            "",
            &[],
            0,
            0,
            &[],
            false,
        );
        assert!(orphan.get("mode").is_none());
    }

    // -----------------------------------------------------------------------
    // 16b cut AF.2 (16c N16): the two documents of a running match
    // -----------------------------------------------------------------------

    /// One key parser, named.
    type Parser = (&'static str, fn(&str) -> Option<MatchId>);

    /// Every tacsim key parser of the running and finished matches.
    fn parsers() -> [Parser; 4] {
        [
            ("match", match_id_of_key),
            ("replay", crate::match_::replay::replay_id_of_key),
            ("snapshot", snapshot_id_of_key),
            ("live", live_id_of_key),
        ]
    }

    /// **16b cut AF.2**: the two new keys round-trip, the parses are exact
    /// (digits only), and the four parsers are pairwise disjoint — each accepts
    /// its own key and refuses every other, a report key included.
    #[test]
    fn af2_the_four_key_parsers_are_pairwise_disjoint() {
        assert_eq!(snapshot_key(MatchId(7)).as_str(), "tacsim:snapshot:7");
        assert_eq!(live_key(MatchId(7)).as_str(), "tacsim:live:7");
        assert_eq!(snapshot_id_of_key("tacsim:snapshot:7"), Some(MatchId(7)));
        assert_eq!(live_id_of_key("tacsim:live:7"), Some(MatchId(7)));
        for bad in [
            "tacsim:live:",
            "tacsim:live:7x",
            "tacsim:live:-7",
            "tacsim:live:7:1",
            "tacsim:live: 7",
            "tacsim:snapshot:",
            "tacsim:snapshot:+7",
        ] {
            assert_eq!(live_id_of_key(bad), None, "{bad}");
            assert_eq!(snapshot_id_of_key(bad), None, "{bad}");
        }
        let keys = [
            ("match", "tacsim:match:1"),
            ("replay", "tacsim:replay:1"),
            ("snapshot", "tacsim:snapshot:1"),
            ("live", "tacsim:live:1"),
            ("report", "tacsim:report:1:2"),
        ];
        for (owner, key) in keys {
            for (parser, parse) in parsers() {
                assert_eq!(
                    parse(key).is_some(),
                    owner == parser,
                    "the {parser} parser on {key}"
                );
            }
        }
    }

    /// `report_key`'s shape: `tacsim:report:<match>:<unit>`, digits both.
    fn is_report_key(key: &str) -> bool {
        key.strip_prefix("tacsim:report:")
            .and_then(|rest| rest.split_once(':'))
            .is_some_and(|(id, unit)| {
                [id, unit]
                    .iter()
                    .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
            })
    }

    /// **16b cut AF.2 — the two documents are readable by nobody but the boot**
    /// (doc 16 §1's observer rule, for two new surfaces).
    ///
    /// Reviewed readers use report/log keys or bounded immutable proving-blob
    /// chunks. None can address a live match or its snapshot. BENCH separately
    /// gates the blob reader on a complete, character-owned proving run before
    /// requesting any key (`bench_service::completed_proving`).
    #[test]
    fn af2_no_key_a_store_read_can_build_is_a_running_matchs_document() {
        let ids = [
            0_u64,
            1,
            2,
            9,
            10,
            99,
            12_345,
            u64::from(u32::MAX),
            u64::MAX,
        ];
        let units = [0_u32, 1, 101, 4_096, u32::MAX];
        let mut read = Vec::new();
        for id in ids {
            for unit in units {
                read.push(report_key(MatchId(id), ObjId(unit)));
            }
            let log = crate::arena_bridge::match_log_id(MatchId(id));
            for chunk in [0_u64, 1, 15, 16, 1_000_000, u64::MAX] {
                read.push(mush_core::chunk_key(&log, chunk));
            }
        }
        use crate::minds::bulk::{
            artifacts::{Key, Manifest, Part, Read},
            model::{Artifact, RunId},
        };
        let account = mush_core::AccountId::mint("store-key-proof", 1).expect("account");
        for id in ids.into_iter().filter(|id| *id > 0) {
            for part in [Part::Start, Part::Result] {
                let key = Key {
                    account,
                    run: RunId(id),
                    part,
                };
                let reader = Read::new(Manifest {
                    format: 1,
                    key,
                    artifact: Artifact {
                        sha256: [0; 32],
                        bytes: 100,
                    },
                    chunks: 2,
                    summary: Vec::new(),
                    context: None,
                })
                .expect("bounded proving reader");
                assert_eq!(reader.keys(), vec![key.chunk(0), key.chunk(1)]);
                read.extend(reader.keys());
                for chunk in [2, 15, 16, 512, u16::MAX] {
                    read.push(key.chunk(chunk));
                }
            }
        }
        for key in &read {
            assert!(
                is_report_key(key)
                    || mush_core::parse_chunk_key(key).is_some()
                    || key.starts_with("tacsim:mindbulk:blob:"),
                "{key} is outside every reviewed read key space"
            );
            for (parser, parse) in parsers() {
                assert_eq!(parse(key), None, "the {parser} parser accepts {key}");
            }
            assert!(
                !key.starts_with("tacsim:live:") && !key.starts_with("tacsim:snapshot:"),
                "{key}"
            );
        }
    }

    /// Pin every by-key store reader. M6 adds BENCH's bounded immutable proving
    /// artifact reader; it accepts only completed owner-scoped runs, and its
    /// chunk key space is checked above. A future reader needs the same review.
    #[test]
    fn af2_the_crate_store_readers_have_reviewed_key_spaces() {
        // Built from two halves, so this file's own text is not a caller.
        let needle = concat!("read_", "store(");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root.clone()];
        let mut callers: Vec<(String, usize)> = Vec::new();
        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir).expect("the source directory reads");
            for entry in entries {
                let path = entry.expect("a directory entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("a source file reads");
                let calls: usize = text
                    .lines()
                    .map(|line| line.split("//").next().unwrap_or_default())
                    .map(|code| code.matches(needle).count())
                    .sum();
                if calls > 0 {
                    let name = path
                        .strip_prefix(&root)
                        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    callers.push((name, calls));
                }
            }
        }
        callers.sort();
        assert_eq!(
            callers,
            vec![
                ("match_/report.rs".to_owned(), 2),
                ("minds/bulk/bench_service.rs".to_owned(), 1),
            ],
            "the crate's store reads moved: review the new reader's authorization and \
             prove its keys cannot address running match documents before changing this inventory"
        );
    }

    /// A record carrying everything a running match's does: a pilot fold, a
    /// replay fold, a full-range seed, a last pose, and a centre with a seated
    /// slot on it.
    fn af2_live() -> LiveRecord {
        let mut record = v2_record();
        record.seed = u64::MAX - 1;
        record.last_pose.insert(
            ObjId(101),
            PoseNumbers {
                x_cm: 1_500,
                y_cm: 2_500,
                z_cm: 30,
                heading_mdeg: 90_000,
            },
        );
        let roster = crate::match_::report::Roster::from_record(&record);
        record.folds = roster
            .pilots()
            .map(|(_, unit)| {
                (
                    unit,
                    crate::match_::report::Fold::new(
                        roster.clone(),
                        crate::match_::report::Reader::Pilot { unit },
                        crate::match_::report::ReportRules {
                            designate_on_detection: false,
                        },
                    ),
                )
            })
            .collect();
        let named: Vec<(ObjId, Designator, Team)> = record
            .callsigns
            .iter()
            .filter_map(|(unit, name)| record.team_of(*unit).map(|team| (*unit, *name, team)))
            .collect();
        record.replay = Some(crate::match_::replay::ReplayFold::new(
            record.id,
            &record.arena,
            &record.arena_definition_digest,
            1_000,
            64,
            64,
            5_000,
            &named,
            &[],
        ));
        let account = mush_core::AccountId::mint("pilot-5", 5).expect("an account id mints");
        let centre = crate::match_::lobby::Centre {
            state: MatchState::Running,
            match_id: Some(record.id),
            arena: record.arena.clone(),
            slots: vec![crate::match_::lobby::Slot {
                team: Team::Red,
                index: 1,
                account,
                pilot: ObjId(5),
                pod: Some(ObjId(900)),
                unit: Some(ObjId(101)),
                left: true,
                dropped_at_ms: Some(1_234),
                unit_type: None,
                released_by_resume: false,
                mind: None,
            }],
            leads: vec![(Team::Red, account)],
            ..crate::match_::lobby::Centre::default()
        };
        LiveRecord {
            match_id: record.id,
            at_ms: 15_000,
            chunks_appended: 4,
            chunks_lost: 1,
            resumed_from_ms: None,
            record,
            centre,
        }
    }

    #[test]
    fn l5_live_leaders_round_trip_and_missing_field_stays_leaderless() {
        use crate::deliberation::{CharacterRef, LanceLeader, SquadId};
        let mut live = af2_live();
        let red = SquadId {
            team: Team::Red,
            index: 1,
        };
        let blue = SquadId {
            team: Team::Blue,
            index: 1,
        };
        live.record.leaders = vec![
            LanceLeader {
                squad: red,
                by: CharacterRef::Persona { squad: red },
            },
            LanceLeader {
                squad: blue,
                by: CharacterRef::Character { id: ObjId(5) },
            },
        ];
        let mut document = live.to_document().expect("a live leader record serializes");
        let restored = LiveRecord::from_document(&document).expect("leaders restore");
        assert_eq!(restored.record.leaders, live.record.leaders);

        // Model the earlier shape within this supported live envelope. The
        // absence means no named leader, not permission to mint one at decode.
        let mut state: serde_json::Value =
            serde_json::from_str(document["state"].as_str().expect("encoded live state"))
                .expect("live state JSON");
        assert!(
            state["record"]
                .as_object_mut()
                .expect("record")
                .remove("leaders")
                .is_some()
        );
        let state = serde_json::to_string(&state).expect("old record shape");
        document["state_digest"] = serde_json::json!(live_state_digest(&state));
        document["state"] = serde_json::json!(state);
        let restored = LiveRecord::from_document(&document).expect("supported missing field");
        assert!(restored.record.leaders.is_empty());
        assert_eq!(restored.record.squads, live.record.squads);
        assert_eq!(restored.centre, live.centre);
    }

    #[test]
    fn m6_live_record_retains_the_leads_selected_class_cap() {
        let mut live = af2_live();
        let account = live.centre.leads[0].1;
        live.centre
            .fill_classes
            .push(crate::match_::lobby::FillClass {
                team: Team::Red,
                lead: account,
                class: crate::arena::core::CoreClass::II,
            });
        let document = live.to_document().expect("live cap serializes");
        let restored = LiveRecord::from_document(&document).expect("live cap restores");
        assert_eq!(restored.centre.fill_classes, live.centre.fill_classes);
        assert_eq!(
            restored.centre.fill_classes[0].class,
            crate::arena::core::CoreClass::II
        );
    }

    #[test]
    fn m7_pre_certificate_live_formats_refuse_before_old_state_decode() {
        let mut document = af2_live().to_document().expect("current live record");
        assert_eq!(LIVE_FORMAT, 3, "M7 certified custody is live format 3");
        // Even a malformed inner state must receive the outer version refusal.
        document["state"] = serde_json::json!("not a current state");
        document["state_digest"] = serde_json::json!("0".repeat(64));
        for old in [1, 2] {
            document["format"] = serde_json::json!(old);
            assert_eq!(
                LiveRecord::from_document(&document),
                Err(LiveRecordError::Format {
                    found: old,
                    expected: LIVE_FORMAT
                })
            );
        }
        document["format"] = serde_json::json!(LIVE_FORMAT);
        assert!(matches!(
            LiveRecord::from_document(&document),
            Err(LiveRecordError::DigestMismatch { .. })
        ));
    }

    /// **16b cut AF.2**: a live record round-trips equal — through a JSON
    /// string, as the store would hand it back — with its keys exactly the
    /// document's, and every refusal is by name.
    #[test]
    fn af2_a_live_record_round_trips_and_refuses_by_name() {
        let live = af2_live();
        let document = live.to_document().expect("a live record serialises");
        let object = document.as_object().expect("an object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "at_ms",
                "chunks_appended",
                "chunks_lost",
                "format",
                "match",
                "record",
                "resumed_from_ms",
                "state",
                "state_digest",
            ]
        );
        assert_eq!(document["record"], LIVE_RECORD);
        assert_eq!(document["format"], LIVE_FORMAT);
        assert_eq!(document["match"], 12);
        assert!(document["resumed_from_ms"].is_null());
        assert!(document["state"].is_string(), "the state is one string");
        let text = serde_json::to_string(&document).expect("the document prints");
        let back: serde_json::Value = serde_json::from_str(&text).expect("and parses");
        assert_eq!(LiveRecord::from_document(&back), Ok(live.clone()));

        let with = |edit: &dyn Fn(&mut serde_json::Value)| {
            let mut copy = document.clone();
            edit(&mut copy);
            LiveRecord::from_document(&copy)
        };
        // A moved byte in the state: the digest refuses it before the parse.
        let moved = with(&|doc| {
            let state = doc["state"].as_str().unwrap_or_default().replacen(
                "\"started_ms\":1000",
                "\"started_ms\":1001",
                1,
            );
            doc["state"] = serde_json::Value::from(state);
        });
        assert!(
            matches!(moved, Err(LiveRecordError::DigestMismatch { .. })),
            "{moved:?}"
        );
        // The same edit with the digest re-stamped decodes — to another record.
        let restamped = with(&|doc| {
            let state = doc["state"].as_str().unwrap_or_default().replacen(
                "\"started_ms\":1000",
                "\"started_ms\":1001",
                1,
            );
            doc["state_digest"] = serde_json::Value::from(live_state_digest(&state));
            doc["state"] = serde_json::Value::from(state);
        });
        assert_eq!(restamped.map(|back| back.record.started_ms), Ok(1_001));
        let other = with(&|doc| doc["record"] = serde_json::Value::from("tacsim.snapshot"));
        assert!(
            matches!(other, Err(LiveRecordError::Record { .. })),
            "{other:?}"
        );
        let format = with(&|doc| doc["format"] = serde_json::Value::from(LIVE_FORMAT + 1));
        assert_eq!(
            format,
            Err(LiveRecordError::Format {
                found: u64::from(LIVE_FORMAT + 1),
                expected: LIVE_FORMAT
            })
        );
        let unknown = with(&|doc| {
            if let Some(object) = doc.as_object_mut() {
                object.insert("extra".to_owned(), serde_json::Value::from(1));
            }
        });
        assert!(
            matches!(unknown, Err(LiveRecordError::Malformed(_))),
            "{unknown:?}"
        );
        let elsewhere = with(&|doc| doc["match"] = serde_json::Value::from(13));
        assert!(
            matches!(elsewhere, Err(LiveRecordError::Malformed(_))),
            "{elsewhere:?}"
        );
        // An unknown key inside the state, re-stamped: the state's own shape.
        let inner = with(&|doc| {
            let state = doc["state"].as_str().unwrap_or_default().replacen(
                "{\"record\":{",
                "{\"record\":{\"extra\":1,",
                1,
            );
            doc["state_digest"] = serde_json::Value::from(live_state_digest(&state));
            doc["state"] = serde_json::Value::from(state);
        });
        assert!(
            matches!(inner, Err(LiveRecordError::Malformed(_))),
            "{inner:?}"
        );
        // Every refusal says what AF.3's void reason prints.
        for refused in [moved, other, format, unknown, elsewhere, inner] {
            let said = refused
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default();
            assert!(
                said.starts_with("the live record does not decode: "),
                "{said}"
            );
        }
    }
}
