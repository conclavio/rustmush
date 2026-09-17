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
            EndReason::Elimination { .. } | EndReason::Timeout => MatchState::Ended,
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
    /// Cut T.3 has no damage model, so no `Elimination` can actually be reached
    /// — every match this cut runs ends `Abort`, `Timeout` or `Void`. The
    /// elimination arms are written out anyway because the arena's own
    /// [`EndReason`] already carries the winning side, and reading it is the
    /// whole of what cut W needs from this function.
    fn from(reason: EndReason) -> MatchOutcome {
        match reason {
            EndReason::Elimination {
                winner: Some(Team::Red),
            } => MatchOutcome::Red,
            EndReason::Elimination {
                winner: Some(Team::Blue),
            } => MatchOutcome::Blue,
            EndReason::Elimination { winner: None } => MatchOutcome::Draw,
            EndReason::Timeout => MatchOutcome::Timeout,
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
    /// It came back unable to fight (cut W).
    Disabled,
    /// It is a wreck (cut W).
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
    /// Shots fired. `0` until cut W.
    pub shots: u32,
    /// Hits landed. `0` until cut W.
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
    /// Doc 16's `CustodyIn` carries a final pose per unit and cut T.2's `Ended`
    /// does not, so until cut W widens that message this is where a check-in's
    /// final pose comes from. Two sources feed it, both the arena's own numbers:
    /// the **ledger's `position_summary`** events, which name every unit every
    /// `ledger.summary_ms` whether or not anybody is watching, and the
    /// **cockpit frames**, which are fresher but only exist for a unit with a
    /// pilot in it. Neither invents anything: the posture a check-in writes is
    /// the one the check-out snapshot carried, because the arena never touches a
    /// posture.
    pub last_pose: std::collections::BTreeMap<ObjId, PoseNumbers>,
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
/// digest, the arena definition's digest, and the start and end times. It is
/// written through the module's own namespace on the ordinary module-op lane.
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
    let (teams, arena, arena_digest, started_ms, log_id) = match &record {
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
        ),
        None => (
            Vec::new(),
            String::new(),
            String::new(),
            0,
            crate::arena_bridge::match_log_id(match_id),
        ),
    };
    let value = serde_json::json!({
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
        "started_ms": started_ms,
        "ended_ms": ctx.w.now().saturating_mul(1_000),
        "arena_ms": end.t_ms,
    });
    crate::store::push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: result_key(match_id),
            value,
        },
    );
}

/// **The `aborted(restart)` result** (16b T5, doc 16 §3.1 "Boot reconciliation").
///
/// A match whose units were still checked out at boot has no arena, no ledger
/// digest and no end timestamp — the process that held all three is gone. What it
/// does have is an id, a set of units the recovery pass restored, and a reason,
/// and that is exactly what this writes.
pub fn write_restart_abort(ctx: &mut Ctx<'_>, match_id: MatchId, units: &[ObjId]) {
    let rows: Vec<UnitResult> = units.iter().copied().map(UnitResult::intact).collect();
    let value = serde_json::json!({
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
        "started_ms": 0,
        "ended_ms": ctx.w.now().saturating_mul(1_000),
        "arena_ms": 0,
    });
    crate::store::push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: result_key(match_id),
            value,
        },
    );
}

/// The word the result's `reason` field carries.
const fn reason_word(reason: EndReason) -> &'static str {
    match reason {
        EndReason::Elimination { .. } => "elimination",
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
