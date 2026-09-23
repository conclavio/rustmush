//! **The match's states and the one document it leaves behind** (16b cut T.3;
//! doc 16 §3.8 "Lifecycle", §4 "The result record").

use mush_core::{Ctx, ObjId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::state::{EndReason, MatchEnd, Team};
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
    /// A unit that came back whole.
    #[must_use]
    pub const fn intact(unit: ObjId) -> UnitResult {
        UnitResult {
            unit: unit.0,
            outcome: UnitOutcome::Intact,
            shots: 0,
            hits: 0,
        }
    }

    /// A unit the world could not take back.
    #[must_use]
    pub const fn lost(unit: ObjId) -> UnitResult {
        UnitResult {
            unit: unit.0,
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
    pub last_pose: std::collections::BTreeMap<ObjId, PoseNumbers>,
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
}

/// The result document's key inside the tacsim namespace.
#[must_use]
pub fn result_key(match_id: MatchId) -> SmolStr {
    SmolStr::new(format!("tacsim:match:{}", match_id.0))
}

/// **Write one match's result** (doc 16 §4), once, at the end.
///
/// One document, well under the 256 KiB record ceiling: the teams and their
/// pilots, the outcome with its reason, one row per unit, the ledger's log id and
/// digest, **the match seed** (16b cut M.3), the arena definition's digest, and
/// the start and end times. It is written through the module's own namespace on
/// the ordinary module-op lane.
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
/// must never be rendered to a player. Today no verb reads this document back:
/// nothing under `crate::match_` reads `tacsim:match:<id>`, and `verbs::fault`
/// reads [`MatchRecord::seed`] under the fault-injection feature only. 16c `N2`'s
/// report and history verbs, when they are built, must redact it.
pub fn write_result(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    outcome: MatchOutcome,
    end: MatchEnd,
    ledger_digest: &str,
    units: &[UnitResult],
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
    );
    crate::store::push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: result_key(match_id),
            value,
        },
    );
}

/// **The result document [`write_result`] writes** (16b cut M.3 split it out of
/// the write, so the document's shape — the seed on it above all — is testable
/// without a world). `record` is the match's own record when the book still
/// holds it; `ended_ms` is the world's clock in milliseconds.
fn result_document(
    record: Option<&MatchRecord>,
    match_id: MatchId,
    outcome: MatchOutcome,
    end: MatchEnd,
    ledger_digest: &str,
    units: &[UnitResult],
    ended_ms: i64,
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
                        "pilots": roster
                            .pilots
                            .iter()
                            .map(|(pilot, unit)| serde_json::json!({
                                "pilot": pilot.0,
                                "unit": unit.0,
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
        "record_version": 1,
        "match": match_id.0,
        "arena": arena,
        "arena_definition_digest": arena_digest,
        "outcome": outcome.as_str(),
        "reason": reason_word(end.reason),
        "teams": teams,
        "units": units,
        "ledger": { "log": log_id, "digest": ledger_digest },
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
pub fn write_restart_abort(ctx: &mut Ctx<'_>, match_id: MatchId, units: &[ObjId]) {
    let value = restart_abort_document(match_id, units, ctx.w.now().saturating_mul(1_000));
    crate::store::push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: result_key(match_id),
            value,
        },
    );
}

/// The document [`write_restart_abort`] writes (split out by 16b cut M.3, as
/// `result_document` is).
fn restart_abort_document(match_id: MatchId, units: &[ObjId], ended_ms: i64) -> serde_json::Value {
    let rows: Vec<UnitResult> = units.iter().copied().map(UnitResult::intact).collect();
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
}
