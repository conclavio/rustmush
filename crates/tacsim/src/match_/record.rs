//! **The match's states and the one document it leaves behind** (16b cut T.3;
//! doc 16 §3.8 "Lifecycle", §4 "The result record").

use std::collections::BTreeMap;

use mush_core::{Ctx, ObjId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::state::{Designator, EndReason, MatchEnd, Team};
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
            EndReason::Elimination { .. }
            | EndReason::Spent
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamRoster {
    /// Which side.
    pub team: Team,
    /// Its units.
    pub units: Vec<ObjId>,
    /// Its `(pilot, unit)` pairs at check-out.
    pub pilots: Vec<(ObjId, ObjId)>,
}

/// **One match, while the world is tracking it** — RAM only (D-16-3).
#[derive(Clone, Debug, PartialEq)]
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
    /// `(unit, team, controller)` triples the `CustodyOut` carries, so the record's
    /// grouping and the arena's are the same function of the same data and cannot
    /// disagree. It is on the record and not only in the arena because the world is
    /// what fans an order out to a squad's units and what says the sentence when one
    /// expires, and both need the roster without asking across the seam.
    ///
    /// Empty for a match of nothing but pilots, which is doc 16 §1's A1 story.
    pub squads: Vec<crate::deliberation::SquadRoster>,
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
pub fn write_result(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    outcome: MatchOutcome,
    end: MatchEnd,
    ledger_digest: &str,
    units: &[UnitResult],
    chunks: u64,
) {
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
    );
    remember_and_push(ctx, match_id, value);
}

/// **The one tail both result writers share** (16b cut V.2): remember the
/// document on the ring, push it, and delete the report documents of the row the
/// ring let go.
///
/// The deletes ride the same outbox as the result, so the report documents are
/// bounded by the ring whichever writer evicted the row: a restart abort that
/// pushes the twenty-first row out is an eviction too.
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
        for unit in &evicted.reports {
            crate::store::push_op(
                ctx.out,
                mush_core::ModuleWrite::Del {
                    key: report_key(evicted.id, *unit),
                },
            );
        }
    }
}

/// **The result document [`write_result`] writes** (16b cut M.3 split it out of
/// the write, so the document's shape — the seed on it above all — is testable
/// without a world). `record` is the match's own record when the book still
/// holds it; `ended_ms` is the world's clock in milliseconds; `chunks` is the
/// tape's chunk count (16b cut V.2).
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
) -> serde_json::Value {
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
    serde_json::json!({
        "record": "tacsim.match",
        // 16b cut V.2 added `callsign` (unit and pilot rows) and `ledger.chunks`
        // and left this at 1: a key added is not a version — the rule
        // `crate::match_::cockpit::FRAME_VERSION`'s comment states — and a reader of
        // a document without them reads the empty name and nought
        // ([`HistoryRow::from_document`]).
        "record_version": 1,
        "match": match_id.0,
        "arena": arena,
        "arena_definition_digest": arena_digest,
        "outcome": outcome.as_str(),
        "reason": reason_word(end.reason),
        "teams": teams,
        "units": units,
        "ledger": { "log": log_id, "digest": ledger_digest, "chunks": chunks },
        // 16b cut M.3 (16c `T-07`): staff-only — see `write_result`.
        "seed": seed,
        "started_ms": started_ms,
        "ended_ms": ended_ms,
        "arena_ms": end.t_ms,
    })
}

/// **The `aborted(restart)` result** (16b T5, doc 16 §3.1 "Boot reconciliation").
///
/// A match whose units were still checked out at boot has no arena, no ledger
/// digest and no end timestamp — the process that held all three is gone. What it
/// does have is an id, a set of units the recovery pass restored, and a reason,
/// and that is exactly what this writes.
///
/// **16b cut M.3**: its `"seed"` is `0`. There was no arena to have drawn
/// anything — the record the seed lived on went with the process — and the key
/// is present so the result document has one shape whichever way it was written.
///
/// **16b cut V.2**: it is remembered on the history ring exactly as
/// `write_result`'s document is (the same value, before the push), so `SIM
/// HISTORY` lists a restarted match the moment the boot writes it.
pub fn write_restart_abort(ctx: &mut Ctx<'_>, match_id: MatchId, units: &[ObjId]) {
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
    })
}

/// The word the result's `reason` field carries.
const fn reason_word(reason: EndReason) -> &'static str {
    match reason {
        EndReason::Elimination { .. } => "elimination",
        // 16b cut Q.7.
        EndReason::Spent => "spent",
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
    /// - `ledger.chunks` absent reads `0`, `reports` absent reads empty, and every
    ///   other absent word or number reads empty or `0`.
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
        })
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
            "  #{}  {}  {}  {} v {}  {}",
            row.id.0,
            outcome_words(row.outcome, &row.reason),
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
            callsigns: BTreeMap::new(),
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
            callsigns: BTreeMap::from([
                (ObjId(101), slot(Team::Red, 1)),
                (ObjId(102), slot(Team::Red, 2)),
                (ObjId(201), slot(Team::Blue, 1)),
            ]),
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
        assert!(row.reports.is_empty(), "V.2 writes no report key");
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
}
