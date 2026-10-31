//! Private proving execution over complete frozen content scenarios. This owner
//! accepts immutable headers from the authorized production scenario factory;
//! neither a live Arena nor a World can cross the worker boundary.
use super::{
    BulkError,
    executor::{Machine, Ready, Turn},
    invalid,
    metrics::{self, Metrics, Operations},
    model::{ArtifactBytes, MAX_ARTIFACT_BYTES, Plan, RunId, Status, WorkKind},
    report::{Game, Outcome, Report},
};
use crate::arena::{
    Arena, EndReason, PartyId,
    input_tape::{TapeChunk, TapeError, TapeEvent, TapeMark, TapeRecorder, TapeWrite},
    state_hash,
};
use crate::minds::resimulation::{open_frozen_header, record_completed_pulse};
use mush_core::AccountId;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use super::frozen::FrozenStarts;
// The closed Game schema has three <=128-entry maps of <=128-byte keys,
// 4501 u32 histogram buckets, and bounded scalar fields. Even maximal escaped
// keys/counts fit this reserve; the capacity regression constructs that case.
const REPORT_GAME_RESERVE: u64 = 160 * 1024;
const RESULT_METADATA_RESERVE: u64 = 64 * 1024;
const TAPE_ENVELOPE_RESERVE: u64 = 2048;
fn tape_budget(plan: &Plan) -> Result<u64, BulkError> {
    plan.result_reservation_bytes
        .checked_sub(REPORT_GAME_RESERVE * u64::from(plan.games) + RESULT_METADATA_RESERVE)
        .ok_or_else(|| invalid("result reservation cannot retain every game report"))
}

/// No public fields: a ready machine remains bound to its serialized start and
/// the exact roster/seed plan. Only the production content factory mints this.
pub(crate) struct FrozenProving {
    start: FrozenStarts,
    bytes: ArtifactBytes,
    plan: Plan,
}
impl FrozenProving {
    pub(super) fn prepare(mut plan: Plan, start: FrozenStarts) -> Result<Self, BulkError> {
        if plan.kind != WorkKind::Proving
            || start.games.len() != usize::from(plan.games)
            || plan.seeds.len() != usize::from(plan.games / 2)
        {
            return Err(invalid("frozen proving game count"));
        }
        for (i, game) in start.games.iter().enumerate() {
            let seed = plan
                .seeds
                .get(i / 2)
                .ok_or_else(|| invalid("frozen paired seed count"))?;
            if game.seed != *seed
                || game.swapped != (i % 2 == 1)
                || game.unit_count != plan.unit_count
            {
                return Err(invalid("frozen paired seed, side or roster"));
            }
        }
        start
            .context
            .as_ref()
            .ok_or_else(|| invalid("missing proving context"))?
            .validate()?;
        let bytes = ArtifactBytes::new(super::encoding::encode(
            &start,
            MAX_ARTIFACT_BYTES as usize,
        )?)?;
        plan.start = bytes.descriptor().clone();
        plan.validate()?;
        tape_budget(&plan)?;
        Ok(Self { start, bytes, plan })
    }
    pub fn plan(&self) -> &Plan {
        &self.plan
    }
    pub(super) fn bytes_mut(&mut self) -> &mut ArtifactBytes {
        &mut self.bytes
    }
    /// Called after the exact start/plan/run has been published durably. Source
    /// compilation occurs on the private worker at each game's opening turn.
    pub fn ready(self, account: AccountId, run: RunId) -> Ready {
        Ready::new(
            account,
            run,
            self.bytes.descriptor().sha256,
            Box::new(ProvingMachine {
                frozen: self,
                index: 0,
                active: None,
                finished: Vec::new(),
                report: Report { games: Vec::new() },
                retained: 0,
            }),
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GameTape {
    /// Index into the immutable frozen start. Full source is not duplicated.
    pub game: u16,
    pub recording: Recording,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Recording {
    Complete {
        chunks: Vec<TapeChunk>,
        mark: TapeMark,
    },
    Unavailable {
        reason: UnavailableReason,
    },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UnavailableReason {
    Capacity,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProvingResult {
    pub format: u16,
    pub report: Report,
    pub context: super::report::Context,
    pub tapes: Vec<GameTape>,
}
impl ProvingResult {
    fn recording_lines(&self) -> Vec<String> {
        let unavailable = self
            .tapes
            .iter()
            .filter(|t| matches!(t.recording, Recording::Unavailable { .. }))
            .map(|t| (u32::from(t.game) + 1).to_string())
            .collect::<Vec<_>>();
        let mut lines = vec![format!(
            "Bench recordings retained: {} of {} games.",
            self.tapes.len() - unavailable.len(),
            self.tapes.len()
        )];
        if !unavailable.is_empty() {
            lines.push(format!("Bench recording unavailable (capacity), games: {}. Complete game reports retained.", unavailable.join(", ")));
        }
        lines
    }
}
struct Active {
    arena: Arena,
    recorder: Option<TapeRecorder>,
    chunks: Vec<TapeChunk>,
    metrics: Metrics,
    operations: Arc<Mutex<Operations>>,
    wall: i64,
    bytes: u64,
}
struct ProvingMachine {
    frozen: FrozenProving,
    index: usize,
    active: Option<Active>,
    finished: Vec<GameTape>,
    report: Report,
    retained: u64,
}
impl ProvingMachine {
    fn open(&mut self) -> Result<(), BulkError> {
        let game = self
            .frozen
            .start
            .games
            .get(self.index)
            .ok_or_else(|| invalid("proving game cursor"))?;
        let header = self.frozen.start.header(self.index)?;
        let mut arena = open_frozen_header(&header).map_err(tape_error)?;
        let plan = &self.frozen.plan;
        let red = arena
            .units
            .iter()
            .filter(|u| u.team == PartyId::RED)
            .count();
        let blue = arena
            .units
            .iter()
            .filter(|u| u.team == PartyId::BLUE)
            .count();
        if arena.seed != game.seed
            || arena.now_ms != 0
            || arena.ended.is_some()
            || arena.units.len() != usize::from(plan.unit_count)
            || red != usize::from(plan.per_side)
            || blue != red
            || arena.match_cfg.max_duration_ms != plan.limit_ms
            || arena.substep_len_ms() != i64::from(plan.native_substep_ms)
        {
            return Err(invalid("frozen proving state differs from admitted plan"));
        }
        // No external deliberator is called. Any configured initial orders are
        // already in the frozen snapshot; returned signals are discarded.
        let allowance = tape_budget(plan)?.saturating_sub(self.retained);
        let recorder = if allowance > TAPE_ENVELOPE_RESERVE {
            arena.tape_witnesses.enable();
            Some(TapeRecorder::open(&header).map_err(tape_error)?)
        } else {
            None
        };
        let operations = metrics::instrument(&mut arena);
        arena.acceptance = Some(crate::arena::acceptance::Acceptance::default());
        self.active = Some(Active {
            arena,
            recorder,
            chunks: Vec::new(),
            metrics: Metrics::default(),
            operations,
            wall: 0,
            bytes: 0,
        });
        Ok(())
    }
    fn omit_recording(active: &mut Active) {
        active.recorder = None;
        active.chunks.clear();
        active.bytes = 0;
        active.arena.tape_witnesses.disable();
    }
    fn collect(
        active: &mut Active,
        writes: Result<Vec<TapeWrite>, TapeError>,
        remaining: u64,
    ) -> Result<Option<TapeMark>, BulkError> {
        let writes = match writes {
            Ok(writes) => writes,
            Err(TapeError::Unavailable(_)) => {
                Self::omit_recording(active);
                return Ok(None);
            }
            Err(error) => return Err(tape_error(error)),
        };
        let mut mark = None;
        for write in writes {
            match write {
                TapeWrite::Chunk(chunk) => {
                    let size = serde_json::to_vec(&*chunk)
                        .map_err(|_| invalid("proving tape encoding"))?
                        .len() as u64;
                    let next = active
                        .bytes
                        .checked_add(size.saturating_add(1))
                        .ok_or_else(|| invalid("proving tape byte count"))?;
                    // Reserve the complete report and tape envelope BEFORE
                    // retaining a chunk. An unavailable tape keeps no prefix.
                    if next.saturating_add(TAPE_ENVELOPE_RESERVE) > remaining {
                        Self::omit_recording(active);
                        return Ok(None);
                    }
                    active.bytes = next;
                    active.chunks.push((*chunk).clone());
                }
                TapeWrite::Mark(value) => mark = Some(value),
                TapeWrite::Open(_) => return Err(invalid("duplicate proving opening")),
            }
        }
        Ok(mark)
    }
    // Same Pulse/Settle barriers as recorded_step_settled, but recording
    // capacity must never prevent the actual answer delivery or game outcome.
    fn advance(
        active: &mut Active,
        remaining: u64,
    ) -> Result<crate::arena::step::StepOutput, BulkError> {
        let mut out = active.arena.step(active.wall, &[], &[]);
        if let Some(recorder) = active.recorder.as_mut() {
            let writes = record_completed_pulse(
                &mut active.arena,
                recorder,
                active.wall,
                &[],
                &[],
                Vec::new(),
                &out,
            );
            Self::collect(active, writes, remaining)?;
        }
        let thinks = std::mem::take(&mut out.thinks);
        if !thinks.is_empty() {
            let settled = active
                .arena
                .deliver_answers(crate::arena::mind::run_batch_inline(&thinks));
            out.motor.mind_faults = out.motor.mind_faults.saturating_add(settled.faults);
            out.motor.mind_overflows = out.motor.mind_overflows.saturating_add(settled.overflows);
            if let Some(recorder) = active.recorder.as_mut() {
                let writes = active.arena.tape_witnesses.take().and_then(|thinks| {
                    recorder.push(TapeEvent::Settle {
                        thinks,
                        at: active.arena.now_ms,
                        hash: state_hash(&active.arena),
                    })
                });
                Self::collect(active, writes, remaining)?;
            }
        }
        Ok(out)
    }
}
impl Machine for ProvingMachine {
    fn native_substep_ms(&self) -> u32 {
        self.frozen.plan.native_substep_ms
    }
    fn next_simulation_ms(&self) -> u64 {
        if self.active.is_some() {
            u64::from(self.frozen.plan.native_substep_ms)
        } else {
            0
        }
    }
    fn step(&mut self, maximum_sim_ms: u64) -> Result<Turn, BulkError> {
        let Some(active) = self.active.as_mut() else {
            self.open()?;
            return Ok(Turn {
                simulation_ms: 0,
                result: None,
                terminal: None,
            });
        };
        let cadence = u64::from(self.frozen.plan.native_substep_ms);
        if maximum_sim_ms < cadence {
            return Err(invalid("proving quantum differs from captured native step"));
        }
        let remaining = tape_budget(&self.frozen.plan)?.saturating_sub(self.retained);
        let before = active.arena.now_ms;
        active.wall = active
            .wall
            .checked_add(cadence as i64)
            .ok_or_else(|| invalid("proving manual clock"))?;
        let out = Self::advance(active, remaining)?;
        let simulation_ms = active.arena.now_ms.saturating_sub(before).max(0) as u64;
        if simulation_ms > cadence {
            return Err(invalid("proving exceeded one native substep"));
        }
        active.metrics.motor(&out.motor);
        active.metrics.chunks(&out.ledger_chunks)?;
        // Drain the streamed ledger with exact replay acknowledgments. This is
        // private result observation, never the world's persistence lane.
        active.arena.ledger.flush();
        let tail = active.arena.ledger.take_chunks();
        active.metrics.chunks(&tail)?;
        let through = out.ledger_chunks.iter().chain(&tail).map(|c| c.seq).max();
        if let Some(through) = through {
            active.arena.ledger.mark_flushed(through);
            if let Some(recorder) = active.recorder.as_mut() {
                let writes = recorder.push(TapeEvent::LedgerAck { through });
                Self::collect(active, writes, remaining)?;
            }
        }
        let Some(end) = active.arena.ended else {
            return Ok(Turn {
                simulation_ms,
                result: None,
                terminal: None,
            });
        };
        let opening = self
            .frozen
            .start
            .games
            .get(self.index)
            .ok_or_else(|| invalid("proving completion cursor"))?;
        let (seed, swapped) = (opening.seed, opening.swapped);
        let outcome = outcome(end.reason, swapped)?;
        let hash = state_hash(&active.arena);
        let mark = if let Some(recorder) = active.recorder.as_mut() {
            let writes = recorder.finish(active.arena.now_ms, hash, end);
            Self::collect(active, writes, remaining)?
        } else {
            None
        };
        let recording = match mark {
            Some(mark) => Recording::Complete {
                chunks: std::mem::take(&mut active.chunks),
                mark,
            },
            None if active.recorder.is_none() => Recording::Unavailable {
                reason: UnavailableReason::Capacity,
            },
            None => return Err(invalid("proving completion marker missing")),
        };
        let mut game = Game {
            seed,
            swapped,
            outcome,
            elapsed_ms: u32::try_from(active.arena.now_ms)
                .map_err(|_| invalid("proving elapsed time"))?,
            units: self.frozen.plan.unit_count as u8,
            final_hash: hash,
            ledger_digest: active.arena.ledger.digest(),
            section_losses: BTreeMap::new(),
            weapon_outcomes: BTreeMap::new(),
            odds_buckets: [0; 10],
            facet_outcomes: BTreeMap::new(),
            rounds: 0,
            rounds_left: 0,
            lost_first_section: None,
            rules_fired: 0,
            rules_held: 0,
            faults: 0,
            operations: vec![0; 4501],
            unmeasured_thinks: 0,
            odds_unobserved: 0,
        };
        let focus = if game.swapped {
            PartyId::BLUE
        } else {
            PartyId::RED
        };
        std::mem::take(&mut active.metrics).finish(
            &mut game,
            &active.operations,
            &active.arena,
            focus,
        )?;
        game.odds_buckets = active
            .arena
            .acceptance
            .as_ref()
            .ok_or_else(|| invalid("acceptance observer missing"))?
            .buckets();
        game.odds_unobserved = 0;
        // Guard the independently proven closed report bound before retention.
        super::encoding::encode(&game, REPORT_GAME_RESERVE as usize)?;
        self.active = None;
        self.report.games.push(game);
        let tape = GameTape {
            game: self.index as u16,
            recording,
        };
        let retained =
            super::encoding::encode(&tape, remaining.max(TAPE_ENVELOPE_RESERVE) as usize)?.len()
                as u64;
        if matches!(tape.recording, Recording::Complete { .. }) {
            self.retained = self
                .retained
                .checked_add(retained)
                .ok_or_else(|| invalid("proving retained tape count"))?;
        }
        self.finished.push(tape);
        self.index += 1;
        if self.index < self.frozen.start.games.len() {
            return Ok(Turn {
                simulation_ms,
                result: None,
                terminal: None,
            });
        }
        self.report.validate()?;
        let result = ProvingResult {
            format: 2,
            report: self.report.clone(),
            context: self
                .frozen
                .start
                .context
                .clone()
                .ok_or_else(|| invalid("missing proving context"))?,
            tapes: std::mem::take(&mut self.finished),
        };
        let bytes =
            super::encoding::encode(&result, self.frozen.plan.result_reservation_bytes as usize)?;
        if bytes.len() as u64 > self.frozen.plan.result_reservation_bytes
            || bytes.len() as u64 > MAX_ARTIFACT_BYTES
        {
            return Err(invalid("proving complete result exceeded reservation"));
        }
        Ok(Turn {
            simulation_ms,
            result: Some(
                ArtifactBytes::new(bytes)?
                    .with_summary(
                        result
                            .context
                            .lines()
                            .into_iter()
                            .chain(result.report.lines())
                            .chain(result.recording_lines())
                            .collect(),
                    )?
                    .with_context(result.context)?,
            ),
            terminal: Some(Status::Complete),
        })
    }
}
fn outcome(reason: EndReason, swapped: bool) -> Result<Outcome, BulkError> {
    let focus = if swapped { PartyId::BLUE } else { PartyId::RED };
    let winner = match reason {
        EndReason::Elimination { winner } => winner,
        EndReason::Hold { winner } => Some(winner),
        EndReason::Conceded { winner } => winner,
        EndReason::Spent => None,
        EndReason::Timeout | EndReason::Unattended | EndReason::Disengaged => {
            return Ok(Outcome::Timeout);
        }
        EndReason::Abort | EndReason::Void | EndReason::LedgerOverflow => {
            return Err(invalid("proving stopped without a valid game outcome"));
        }
    };
    Ok(match winner {
        Some(team) if team == focus => Outcome::Win,
        Some(_) => Outcome::Loss,
        None => Outcome::Draw,
    })
}
fn tape_error(error: crate::arena::input_tape::TapeError) -> BulkError {
    invalid(format!("proving recording: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn max_report_game(index: u16) -> Game {
        let keys = (0..128)
            .map(|n| (format!("{n:03}{}", "\"".repeat(125)), u32::MAX))
            .collect::<BTreeMap<_, _>>();
        Game {
            seed: u64::MAX - u64::from(index / 2),
            swapped: index % 2 == 1,
            outcome: Outcome::Timeout,
            elapsed_ms: 900_000,
            units: 12,
            final_hash: u64::MAX,
            ledger_digest: [255; 32],
            section_losses: keys.clone(),
            weapon_outcomes: keys.clone(),
            facet_outcomes: keys,
            odds_buckets: [u32::MAX; 10],
            rounds: u32::MAX,
            rounds_left: u32::MAX,
            lost_first_section: Some(true),
            rules_fired: u32::MAX,
            rules_held: u32::MAX,
            faults: u32::MAX,
            operations: vec![u32::MAX; 4501],
            unmeasured_thinks: u32::MAX,
            odds_unobserved: u32::MAX,
        }
    }
    fn report_context() -> super::super::report::Context {
        let selection = super::super::report::Selection {
            class: "III".into(),
            profile: "Line".into(),
            source_sha256: "f".repeat(64),
        };
        super::super::report::Context {
            scenario: super::super::model::Scenario::Muster,
            scenario_content_sha256: "f".repeat(64),
            ground: "proving".into(),
            ground_sha256: "f".repeat(64),
            config_sha256: "f".repeat(64),
            chassis: "heavy".into(),
            startup_ms: 2000,
            max_duration_ms: 900_000,
            native_substep_ms: 250,
            candidate: selection.clone(),
            opponent: selection,
            pair_seeds: (0..50).map(|n| u64::MAX - n).collect(),
        }
    }
    #[test]
    fn one_hundred_complete_timeout_reports_fit_even_when_every_tape_is_unavailable() {
        let result = ProvingResult {
            format: 2,
            report: Report {
                games: (0..100).map(max_report_game).collect(),
            },
            context: report_context(),
            tapes: (0..100)
                .map(|game| GameTape {
                    game,
                    recording: Recording::Unavailable {
                        reason: UnavailableReason::Capacity,
                    },
                })
                .collect(),
        };
        result.report.validate().unwrap();
        result.context.validate().unwrap();
        for game in &result.report.games {
            assert!(super::super::encoding::encode(game, REPORT_GAME_RESERVE as usize).is_ok());
        }
        let bytes = super::super::encoding::encode(&result, MAX_ARTIFACT_BYTES as usize).unwrap();
        let restored: ProvingResult = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.report.counts(), [0, 0, 0, 100]);
        assert_eq!(restored.report, result.report);
        let summary = result
            .context
            .lines()
            .into_iter()
            .chain(result.report.lines())
            .chain(result.recording_lines())
            .collect();
        ArtifactBytes::new(bytes)
            .unwrap()
            .with_summary(summary)
            .unwrap();
        assert_eq!(restored.tapes.len(), 100);
        assert!(restored.tapes.iter().all(|t| matches!(
            t.recording,
            Recording::Unavailable {
                reason: UnavailableReason::Capacity
            }
        )));
    }

    fn active_fixture(record: bool) -> Active {
        use crate::arena::{
            input_tape::{EngineStamp, MIN_MAX_BYTES, TapeHeader},
            test_support::{arena_at_epoch, pose, tank},
        };
        let units = vec![
            tank(mush_core::ObjId(1), PartyId::RED, pose(500, 500)),
            tank(mush_core::ObjId(2), PartyId::BLUE, pose(6500, 6500)),
        ];
        let seals = units.iter().map(|u| u.seal.clone()).collect();
        let mut arena = arena_at_epoch(units, 0);
        let header = TapeHeader::capture(
            &arena,
            1,
            seals,
            &BTreeMap::new(),
            EngineStamp::current(),
            MIN_MAX_BYTES,
        )
        .unwrap();
        let recorder = record.then(|| TapeRecorder::open(&header).unwrap());
        if record {
            arena.tape_witnesses.enable();
        }
        let operations = metrics::instrument(&mut arena);
        Active {
            arena,
            recorder,
            chunks: Vec::new(),
            metrics: Metrics::default(),
            operations,
            wall: 0,
            bytes: 0,
        }
    }
    #[test]
    fn recording_capacity_discards_the_prefix_and_does_not_change_simulation() {
        let mut limited = active_fixture(true);
        let mut plain = active_fixture(false);
        for wall in (0..=4000).step_by(250) {
            limited.wall = wall;
            plain.wall = wall;
            ProvingMachine::advance(&mut limited, 0).unwrap();
            ProvingMachine::advance(&mut plain, 0).unwrap();
            if let Some(recorder) = limited.recorder.as_mut() {
                let writes = recorder.flush();
                ProvingMachine::collect(&mut limited, writes, 0).unwrap();
            }
            assert_eq!(state_hash(&limited.arena), state_hash(&plain.arena));
            assert_eq!(limited.arena.ledger.digest(), plain.arena.ledger.digest());
        }
        assert!(limited.recorder.is_none());
        assert!(limited.chunks.is_empty());
        assert_eq!(limited.bytes, 0);
    }

    #[test]
    fn paired_side_outcomes_do_not_double_count_a_red_bias() {
        assert_eq!(
            outcome(
                EndReason::Elimination {
                    winner: Some(PartyId::RED)
                },
                false
            ),
            Ok(Outcome::Win)
        );
        assert_eq!(
            outcome(
                EndReason::Elimination {
                    winner: Some(PartyId::RED)
                },
                true
            ),
            Ok(Outcome::Loss)
        );
        assert_eq!(outcome(EndReason::Spent, false), Ok(Outcome::Draw));
        assert_eq!(outcome(EndReason::Timeout, false), Ok(Outcome::Timeout));
        assert!(outcome(EndReason::LedgerOverflow, false).is_err());
    }
}
