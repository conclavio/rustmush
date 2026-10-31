//! Closed-loop bench machine. Construction consumes a verified, owner-bound
//! tape lease on a preparation worker. No live arena is accepted or exposed.
use super::{
    BulkError,
    executor::{Machine, Ready, Turn},
    invalid,
    model::{RunId, Status},
};
use crate::{
    arena::{author_trace::ThinkTrace, profile_seal::MindOrigin},
    arena_task::MatchId,
    match_::mind_tape::TapeLease,
    minds::resimulation::{ReplaySession, VerifiedTape},
};
use mush_core::{AccountId, ObjId};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplayCommand {
    RunTo(i64),
    BackTo(i64),
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Picture {
    /// Owner-rechecked metadata, never part of the emitted panel or replay hash.
    #[serde(skip)]
    pub access_origin: Option<MindOrigin>,
    pub unit: ObjId,
    pub event: usize,
    pub at_ms: i64,
    pub complete: bool,
    /// Full reference hash is an opaque string, never a rounded JS number.
    pub state_hash: String,
    pub view: Value,
    pub card: Option<Value>,
    /// Absent unless the selected source is readable by the authorized actor.
    pub memory: Option<Value>,
    pub traces: Vec<ThinkTrace>,
    pub diff: Option<super::experiment::DiffPicture>,
    pub private_run: bool,
    pub timeline: super::branch::Timeline,
    pub possessed: bool,
    pub refusals: Vec<String>,
}
/// The private producer mints authority only after current owner checks and
/// complete tape verification. Historical leases retain their own opaque pin;
/// proving pins are held by the world session until all workers have exited.
pub(super) struct ReferenceLease {
    pub tape: Arc<VerifiedTape>,
    historical: Option<TapeLease>,
    actor: ObjId,
    pub private: bool,
}
impl ReferenceLease {
    pub fn historical(lease: TapeLease, actor: ObjId) -> Self {
        Self {
            tape: Arc::clone(&lease.tape),
            historical: Some(lease),
            actor,
            private: false,
        }
    }
    pub fn proving(
        start: super::artifacts::Read,
        result: super::artifacts::Read,
        game: usize,
        actor: ObjId,
    ) -> Result<Self, BulkError> {
        let starts = super::frozen::FrozenStarts::decode(start.finish()?.bytes())?;
        let result = result.finish()?;
        let finished: super::proving::ProvingResult = serde_json::from_slice(result.bytes())
            .map_err(|_| invalid("proving result document"))?;
        if finished.format != 2
            || finished.tapes.len() != starts.games.len()
            || finished.report.games.len() != starts.games.len()
        {
            return Err(invalid("proving tape count"));
        }
        finished.report.validate()?;
        if starts.context.as_ref() != Some(&finished.context) {
            return Err(invalid("proving report context differs from opening"));
        }
        for (index, report) in finished.report.games.iter().enumerate() {
            let opening = starts
                .games
                .get(index)
                .ok_or_else(|| invalid("proving opening cursor"))?;
            let tape = finished
                .tapes
                .get(index)
                .ok_or_else(|| invalid("proving tape cursor"))?;
            if usize::from(tape.game) != index
                || report.seed != opening.seed
                || report.swapped != opening.swapped
                || u16::from(report.units) != opening.unit_count
            {
                return Err(invalid("proving report differs from recorded outcome"));
            }
            if let super::proving::Recording::Complete { mark, .. } = &tape.recording
                && !matches!(&mark.status, crate::arena::input_tape::TapeStatus::Complete { at, hash, .. } if *at == i64::from(report.elapsed_ms) && *hash == report.final_hash)
            {
                return Err(invalid("proving report differs from recorded outcome"));
            }
        }
        let header = starts.header(game)?;
        if header.seals.iter().any(|s|!matches!(&s.principal,Some(crate::deliberation::order::CharacterRef::Character{id}) if *id==actor)) {
            return Err(BulkError::Missing);
        }
        let entry = finished
            .tapes
            .get(game)
            .filter(|t| usize::from(t.game) == game)
            .ok_or(BulkError::Missing)?;
        let super::proving::Recording::Complete { chunks, mark } = &entry.recording else {
            return Err(invalid(
                "this game's bench recording is unavailable (capacity); its completed report remains available",
            ));
        };
        let chunks = chunks.iter().cloned().map(Arc::new).collect::<Vec<_>>();
        let tape = Arc::new(
            VerifiedTape::verify(Arc::new(header), &chunks, mark.clone()).map_err(tape_error)?,
        );
        Ok(Self {
            tape,
            historical: None,
            actor,
            private: true,
        })
    }
    fn owns(&self, unit: ObjId) -> bool {
        if let Some(lease) = &self.historical {
            return lease.owns(self.actor, unit);
        }
        self.tape.header().seals.iter().any(|s|s.unit==unit && matches!(&s.principal,Some(crate::deliberation::order::CharacterRef::Character{id}) if *id==self.actor))
    }
    pub fn artifact(&self) -> super::model::Artifact {
        super::model::Artifact {
            sha256: self.tape.artifact_digest(),
            bytes: self.tape.encoded_bytes(),
        }
    }
}
// Fresh/unrun minds use the engine's empty-byte sentinel; a piloted hull may
// have no MindState at all. Neither case hides a malformed nonempty record.
fn read_memory(bytes: Option<&[u8]>) -> Result<Value, BulkError> {
    match bytes {
        None | Some([]) => Ok(serde_json::json!({})),
        Some(bytes) => {
            let value: Value = serde_json::from_slice(bytes)
                .map_err(|_| invalid("retained mind memory is unreadable"))?;
            if !value.is_object() {
                return Err(invalid("retained mind memory is unreadable"));
            }
            Ok(value)
        }
    }
}

struct ReplayMachine {
    lease: ReferenceLease,
    session: ReplaySession,
    unit: ObjId,
    memory_allowed: bool,
    diff: Option<super::experiment::Diff>,
    branch: Option<Box<super::branch::Branch>>,
    goal: Option<i64>,
    rewind: bool,
    picture: Arc<Picture>,
}
impl ReplayMachine {
    fn new(lease: ReferenceLease, actor: ObjId, unit: ObjId) -> Result<Self, BulkError> {
        if actor != lease.actor || !lease.owns(unit) {
            return Err(BulkError::Missing);
        }
        let seal = lease
            .tape
            .header()
            .seals
            .iter()
            .find(|s| s.unit == unit)
            .ok_or(BulkError::Missing)?;
        let memory_allowed = match &seal.requested.origin {
            MindOrigin::Doctrine { .. } => true,
            MindOrigin::Draft { character, .. } => *character == actor,
            MindOrigin::Filed { .. } => false,
        };
        let session = ReplaySession::new(Arc::clone(&lease.tape)).map_err(tape_error)?;
        let mut value = Self {
            lease,
            session,
            unit,
            memory_allowed,
            diff: None,
            branch: None,
            goal: None,
            rewind: false,
            picture: Arc::new(Picture {
                access_origin: None,
                unit,
                event: 0,
                at_ms: 0,
                complete: false,
                state_hash: String::new(),
                view: Value::Null,
                card: None,
                memory: None,
                traces: Vec::new(),
                diff: None,
                private_run: false,
                timeline: super::branch::Timeline::Reference,
                possessed: false,
                refusals: Vec::new(),
            }),
        };
        value.refresh()?;
        Ok(value)
    }
    fn refresh(&mut self) -> Result<(), BulkError> {
        let Some(arena) = self
            .branch
            .as_ref()
            .map_or_else(|| self.session.arena(), |b| b.arena())
        else {
            return Ok(());
        };
        let frame = arena
            .frame_for(self.unit, arena.clock.tempo())
            .ok_or(BulkError::Missing)?;
        let memory_allowed = self
            .branch
            .as_ref()
            .map_or(self.memory_allowed, |b| b.memory_allowed);
        let memory = if memory_allowed {
            Some(read_memory(
                arena
                    .minds
                    .get(&self.unit)
                    .map(|mind| mind.memory.as_slice()),
            )?)
        } else {
            None
        };
        let picture = Picture {
            access_origin: self
                .lease
                .tape
                .header()
                .seals
                .iter()
                .find(|seal| seal.unit == self.unit)
                .map(|seal| seal.requested.origin.clone()),
            unit: self.unit,
            event: self.session.event(),
            at_ms: arena.now_ms,
            complete: self.branch.as_ref().map_or(
                self.session.event() == self.lease.tape.event_count(),
                |_| arena.ended.is_some(),
            ),
            state_hash: format!("{:016x}", crate::arena::state_hash(arena)),
            view: crate::match_::cockpit::status_frame(MatchId(self.lease.tape.match_id()), &frame),
            card: if memory_allowed {
                let card = &arena
                    .profiles
                    .get(&self.unit)
                    .ok_or(BulkError::Missing)?
                    .card;
                Some(
                    serde_json::to_value(card.values().ok_or(BulkError::Missing)?)
                        .map_err(|_| invalid("bench card is unreadable"))?,
                )
            } else {
                None
            },
            memory,
            diff: self.diff.as_ref().map(|diff| diff.picture()).transpose()?,
            private_run: self.lease.private,
            timeline: self
                .branch
                .as_ref()
                .map_or(super::branch::Timeline::Reference, |b| b.timeline.clone()),
            possessed: self.branch.as_ref().is_some_and(|b| b.possessed()),
            refusals: self
                .branch
                .as_ref()
                .map_or_else(Vec::new, |b| b.refusals.clone()),
            traces: if memory_allowed {
                arena.traces.recent(self.unit, Some(64))
            } else {
                Vec::new()
            },
        };
        // Retained public picture/trace is bounded separately from the engine.
        if super::encoding::encode(&picture, 256 * 1024)?.len() > 256 * 1024 {
            return Err(invalid("bench picture bound"));
        }
        self.picture = Arc::new(picture);
        Ok(())
    }
    fn replace_branch(
        &mut self,
        next: Option<Box<super::branch::Branch>>,
    ) -> Result<(), BulkError> {
        let old = std::mem::replace(&mut self.branch, next);
        if let Err(error) = self.refresh() {
            self.branch = old;
            return Err(error);
        }
        Ok(())
    }
    fn begin(&mut self, command: ReplayCommand) -> Result<(), BulkError> {
        if self.goal.is_some() {
            return Err(BulkError::Busy);
        }
        let at = self.picture.at_ms;
        let end = self.lease.tape.duration_ms();
        let goal = match command {
            ReplayCommand::RunTo(t) if t >= at && t <= end => t,
            ReplayCommand::BackTo(t)
                if t >= self.branch.as_ref().map_or(0, |b| b.opening_ms()) && t <= at =>
            {
                self.rewind = true;
                t
            }
            _ => return Err(invalid("bench target is outside the retained timeline")),
        };
        self.goal = Some(goal);
        Ok(())
    }
    fn reference_reached(&self, goal: i64) -> bool {
        // The terminal timestamp can precede final same-time Settle/ack
        // barriers. Only the full verified cursor completes an end target.
        self.session.arena().is_some_and(|a| a.awaiting.is_empty())
            && (self.picture.complete
                || (goal < self.lease.tape.duration_ms() && self.picture.at_ms >= goal))
    }
}
impl Machine for ReplayMachine {
    fn native_substep_ms(&self) -> u32 {
        self.session.native_substep_ms()
    }
    fn next_simulation_ms(&self) -> u64 {
        if self.rewind || self.goal.is_none() {
            0
        } else {
            self.branch.as_ref().map_or_else(
                || self.session.next_simulation_ms(),
                |b| b.next_simulation_ms(),
            )
        }
    }
    fn step(&mut self, maximum_sim_ms: u64) -> Result<Turn, BulkError> {
        let goal = self
            .goal
            .ok_or_else(|| invalid("bench has no admitted command"))?;
        if let Some(branch) = &mut self.branch {
            if self.rewind {
                branch.rewind()?;
                self.rewind = false;
                self.refresh()?;
                return Ok(Turn {
                    simulation_ms: 0,
                    result: None,
                    terminal: None,
                });
            }
            if branch
                .arena()
                .is_some_and(|a| a.now_ms >= goal || a.ended.is_some())
            {
                self.goal = None;
                self.refresh()?;
                return Ok(Turn {
                    simulation_ms: 0,
                    result: None,
                    terminal: Some(Status::Complete),
                });
            }
            let used = branch.advance(maximum_sim_ms)?;
            let done = branch
                .arena()
                .is_some_and(|a| a.now_ms >= goal || a.ended.is_some());
            if done {
                self.goal = None;
            }
            self.refresh()?;
            return Ok(Turn {
                simulation_ms: used,
                result: None,
                terminal: done.then_some(Status::Complete),
            });
        }
        if self.rewind {
            self.session.rewind().map_err(tape_error)?;
            if let Some(diff) = &mut self.diff {
                diff.reset(0)?;
                diff.install(self.session.arena_mut().ok_or(BulkError::Busy)?);
            }
            self.rewind = false;
            self.refresh()?;
            return Ok(Turn {
                simulation_ms: 0,
                result: None,
                terminal: None,
            });
        }
        if self.reference_reached(goal) {
            self.goal = None;
            return Ok(Turn {
                simulation_ms: 0,
                result: None,
                terminal: Some(Status::Complete),
            });
        }
        if self.session.next_simulation_ms() > maximum_sim_ms {
            return Err(invalid("bench quantum differs from its lease"));
        }
        let progress = self.session.advance_substep().map_err(tape_error)?;
        if let (Some(diff), Some(arena)) = (&mut self.diff, self.session.arena_mut()) {
            diff.install(arena);
        }
        self.refresh()?;
        let done = self.reference_reached(goal);
        if done {
            self.goal = None;
        }
        Ok(Turn {
            simulation_ms: progress.simulated_ms,
            result: None,
            terminal: done.then_some(Status::Complete),
        })
    }
    fn bench_start(&mut self, command: ReplayCommand) -> Result<(), BulkError> {
        self.begin(command)
    }
    fn bench_edit(&mut self, edit: super::experiment::Edit) -> Result<(), BulkError> {
        if self.goal.is_some() {
            return Err(BulkError::Busy);
        }
        match edit {
            super::experiment::Edit::Reference => {
                let arena = self.session.arena_mut().ok_or(BulkError::Busy)?;
                if !arena.awaiting.is_empty() {
                    return Err(BulkError::Busy);
                }
                let mut diff = self.diff.take();
                if let Some(d) = &mut diff {
                    d.uninstall(arena);
                }
                if let Err(error) = self.replace_branch(None) {
                    self.diff = diff;
                    if let Some(d) = &mut self.diff {
                        d.install(self.session.arena_mut().ok_or(BulkError::Busy)?);
                    }
                    return Err(error);
                }
                return Ok(());
            }
            super::experiment::Edit::Swap { candidate, serial } => {
                if self.diff.is_some() {
                    return Err(invalid("BENCH DIFF OFF before a mutable fork"));
                }
                super::branch::Branch::preflight(self.session.arena().ok_or(BulkError::Busy)?)?;
                let mut arena = self
                    .session
                    .fork_settled()
                    .map_err(tape_error)?
                    .into_settled_arena()
                    .map_err(tape_error)?;
                let seal = self
                    .lease
                    .tape
                    .header()
                    .seals
                    .iter()
                    .find(|s| s.unit == self.unit)
                    .ok_or(BulkError::Missing)?;
                super::experiment::BoundCandidate::prepare(&arena, seal, &candidate)?
                    .install(&mut arena)?;
                let branch = super::branch::Branch::new(
                    arena,
                    self.lease.actor,
                    self.unit,
                    serial,
                    self.session.event(),
                    true,
                )?;
                return self.replace_branch(Some(branch));
            }
            super::experiment::Edit::Possess { serial } => {
                if self.diff.is_some() {
                    return Err(invalid("BENCH DIFF OFF before a mutable fork"));
                }
                if let Some(branch) = &mut self.branch {
                    branch.possess(true)?;
                } else {
                    super::branch::Branch::preflight(self.session.arena().ok_or(BulkError::Busy)?)?;
                    let arena = self
                        .session
                        .fork_settled()
                        .map_err(tape_error)?
                        .into_settled_arena()
                        .map_err(tape_error)?;
                    let mut branch = super::branch::Branch::new(
                        arena,
                        self.lease.actor,
                        self.unit,
                        serial,
                        self.session.event(),
                        self.memory_allowed,
                    )?;
                    branch.possess(true)?;
                    return self.replace_branch(Some(branch));
                }
                return self.refresh();
            }
            super::experiment::Edit::Release => {
                self.branch
                    .as_mut()
                    .ok_or(BulkError::Missing)?
                    .possess(false)?;
                return self.refresh();
            }
            super::experiment::Edit::Do(text) => {
                self.branch
                    .as_mut()
                    .ok_or(BulkError::Missing)?
                    .command(&text)?;
                return self.refresh();
            }
            _ => {}
        }
        if self.branch.is_some() {
            return Err(invalid(
                "BENCH REFERENCE before comparing historical pictures",
            ));
        }
        let arena = self.session.arena_mut().ok_or(BulkError::Busy)?;
        if !arena.awaiting.is_empty() {
            return Err(BulkError::Busy);
        }
        let next = match edit {
            super::experiment::Edit::Diff(candidate) => {
                if self.lease.tape.unit_count() >= 300 {
                    return Err(invalid(
                        "candidate comparison exceeds the charged roster bound",
                    ));
                }
                let seal = self
                    .lease
                    .tape
                    .header()
                    .seals
                    .iter()
                    .find(|s| s.unit == self.unit)
                    .ok_or(BulkError::Missing)?;
                let bound = super::experiment::BoundCandidate::prepare(arena, seal, &candidate)?;
                Some(super::experiment::Diff::new(bound, arena.now_ms))
            }
            super::experiment::Edit::DiffOff => None,
            _ => return Err(invalid("unsupported bench edit")),
        };
        let mut previous = self.diff.take();
        if let Some(diff) = &mut previous {
            diff.uninstall(arena);
        }
        self.diff = next;
        if let Some(diff) = &mut self.diff {
            diff.install(arena);
        }
        if let Err(error) = self.refresh() {
            let arena = self.session.arena_mut().ok_or(BulkError::Busy)?;
            if let Some(diff) = &mut self.diff {
                diff.uninstall(arena);
            }
            self.diff = previous;
            if let Some(diff) = &mut self.diff {
                diff.install(arena);
            }
            return Err(error);
        }
        Ok(())
    }
    fn bench_unit_count(&self) -> Option<u16> {
        u16::try_from(self.lease.tape.unit_count() + usize::from(self.diff.is_some())).ok()
    }
    fn bench_act_as(&mut self, callsign: &str) -> Result<(), BulkError> {
        if !self.lease.private
            || self.goal.is_some()
            || self.diff.is_some()
            || self.branch.is_some()
        {
            return Err(BulkError::Missing);
        }
        let arena = self.session.arena().ok_or(BulkError::Busy)?;
        let unit = arena
            .callsigns()
            .iter()
            .find(|(u, c)| self.lease.owns(*u) && c.to_string().eq_ignore_ascii_case(callsign))
            .map(|(u, _)| *u)
            .ok_or(BulkError::Missing)?;
        let seal = self
            .lease
            .tape
            .header()
            .seals
            .iter()
            .find(|s| s.unit == unit)
            .ok_or(BulkError::Missing)?;
        let allowed = match &seal.requested.origin {
            MindOrigin::Doctrine { .. } => true,
            MindOrigin::Draft { character, .. } => *character == self.lease.actor,
            MindOrigin::Filed { .. } => false,
        };
        let old = (self.unit, self.memory_allowed);
        self.unit = unit;
        self.memory_allowed = allowed;
        if let Err(error) = self.refresh() {
            self.unit = old.0;
            self.memory_allowed = old.1;
            return Err(error);
        }
        Ok(())
    }
    fn bench_abort_start(&mut self) {
        self.goal = None;
        self.rewind = false;
    }
    fn bench_picture(&self) -> Option<Arc<Picture>> {
        Some(Arc::clone(&self.picture))
    }
}
fn tape_error(_error: crate::arena::input_tape::TapeError) -> BulkError {
    invalid("reference replay failed validation or diverged")
}

/// Preparation-worker output. Permission sets are derived from verified seals,
/// never supplied by a request. No source or omniscient Arena is returned.
pub(super) struct Opened {
    pub ready: Ready,
    pub reference: super::bench::Reference,
    pub selected: ObjId,
    pub plan: super::model::Plan,
}
pub(super) fn open_reference(
    account: AccountId,
    lease: ReferenceLease,
    id: super::bench::ReferenceId,
    callsign: Option<&str>,
) -> Result<Opened, BulkError> {
    use std::collections::BTreeSet;
    let actor = lease.actor;
    let arena = ReplaySession::new(Arc::clone(&lease.tape)).map_err(tape_error)?;
    let arena = arena.arena().ok_or(BulkError::Missing)?;
    let inspect = lease
        .tape
        .header()
        .seals
        .iter()
        .filter(|s| lease.owns(s.unit))
        .map(|s| s.unit)
        .collect::<BTreeSet<_>>();
    let selected = match callsign {
        Some(name) => arena
            .callsigns()
            .iter()
            .find(|(unit, label)| {
                inspect.contains(unit) && label.to_string().eq_ignore_ascii_case(name)
            })
            .map(|(unit, _)| *unit)
            .ok_or(BulkError::Missing)?,
        None => *inspect.iter().next().ok_or(BulkError::Missing)?,
    };
    let memory = lease
        .tape
        .header()
        .seals
        .iter()
        .filter(|s| {
            inspect.contains(&s.unit)
                && match &s.requested.origin {
                    MindOrigin::Doctrine { .. } => true,
                    MindOrigin::Draft { character, .. } => *character == actor,
                    MindOrigin::Filed { .. } => false,
                }
        })
        .map(|s| s.unit)
        .collect();
    let reference = super::bench::Reference::verified(
        id,
        lease.artifact(),
        inspect.clone(),
        memory,
        if lease.private {
            inspect
        } else {
            BTreeSet::new()
        },
    )?;
    let plan = super::model::Plan {
        registry: None,
        conduct: None,
        kind: super::model::WorkKind::Bench,
        scenario: super::model::Scenario::Muster,
        opponent: super::model::Opponent::Mirror,
        per_side: 1,
        unit_count: u16::try_from(lease.tape.unit_count()).map_err(|_| invalid("bench roster"))?,
        games: 1,
        seeds: vec![0],
        limit_ms: u32::try_from(lease.tape.duration_ms())
            .map_err(|_| invalid("bench duration"))?
            .max(1),
        native_substep_ms: arena.substep_len_ms() as u32,
        start: lease.artifact(),
        result_reservation_bytes: 1,
    };
    plan.validate()?;
    let machine = ReplayMachine::new(lease, actor, selected)?;
    Ok(Opened {
        ready: Ready::new(account, RunId(0), plan.start.sha256, Box::new(machine)),
        reference,
        selected,
        plan,
    })
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[test]
    fn fresh_proving_opening_and_piloted_history_have_empty_memory() {
        assert_eq!(read_memory(Some(&[])).unwrap(), serde_json::json!({}));
        assert_eq!(read_memory(None).unwrap(), serde_json::json!({}));
        assert_eq!(
            read_memory(Some(br#"{"n":7}"#)).unwrap(),
            serde_json::json!({"n":7})
        );
    }

    #[test]
    fn malformed_nonempty_memory_still_refuses() {
        assert!(read_memory(Some(b"{")).is_err());
        assert!(read_memory(Some(b"null")).is_err());
        assert!(read_memory(Some(b"[]")).is_err());
    }
}

#[cfg(test)]
mod settle_boundary_tests {
    use super::*;
    use crate::arena::{
        Controller, PartyId,
        input_tape::{EngineStamp, MIN_MAX_BYTES, TapeEvent, TapeHeader, TapeRecorder, TapeWrite},
        state_hash,
        test_support::{arena_of, pose, tank},
    };
    use crate::minds::resimulation::recorded_step_settled;
    fn tape() -> ReferenceLease {
        let actor = ObjId(99);
        let mut red = tank(ObjId(4), PartyId::RED, pose(1500, 4500));
        red.controller = Controller::Mind;
        red.reseal(&crate::match_::SealTerms::fixture()).unwrap();
        red.seal.principal =
            Some(crate::deliberation::order::CharacterRef::Character { id: actor });
        let units = vec![red, tank(ObjId(9), PartyId::BLUE, pose(6500, 4500))];
        let seals = units.iter().map(|u| u.seal.clone()).collect();
        let mut arena = arena_of(units);
        let header = Arc::new(
            TapeHeader::capture(
                &arena,
                1,
                seals,
                &std::collections::BTreeMap::new(),
                EngineStamp::current(),
                MIN_MAX_BYTES,
            )
            .unwrap(),
        );
        let mut recorder = TapeRecorder::open(&header).unwrap();
        arena.tape_witnesses.enable();
        let mut writes = Vec::new();
        for wall in (0..=3000).step_by(250) {
            writes.extend(
                recorded_step_settled(&mut arena, &mut recorder, wall, &[], &[], vec![])
                    .unwrap()
                    .1,
            );
        }
        writes.extend(
            recorder
                .push(TapeEvent::Stop {
                    reason: crate::arena::StopReason::Abort,
                })
                .unwrap(),
        );
        arena.stop(crate::arena::StopReason::Abort);
        writes.extend(
            recorded_step_settled(&mut arena, &mut recorder, 3250, &[], &[], vec![])
                .unwrap()
                .1,
        );
        // These legal zero-time barriers follow the terminal Pulse. Reaching
        // its timestamp alone must not omit their verification/effects.
        arena.settle_inline();
        writes.extend(
            recorder
                .push(TapeEvent::Settle {
                    thinks: arena.tape_witnesses.take().unwrap(),
                    at: arena.now_ms,
                    hash: state_hash(&arena),
                })
                .unwrap(),
        );
        let through = arena.ledger.last_closed_seq();
        assert!(through > 0, "the finished match has a real ledger chunk");
        writes.extend(recorder.push(TapeEvent::LedgerAck { through }).unwrap());
        arena.ledger.mark_flushed(through);
        writes.extend(
            recorder
                .finish(arena.now_ms, state_hash(&arena), arena.ended.unwrap())
                .unwrap(),
        );
        let chunks = writes
            .iter()
            .filter_map(|w| {
                if let TapeWrite::Chunk(c) = w {
                    Some(c.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let mark = writes
            .iter()
            .rev()
            .find_map(|w| {
                if let TapeWrite::Mark(m) = w {
                    Some(m.clone())
                } else {
                    None
                }
            })
            .unwrap();
        ReferenceLease {
            tape: Arc::new(VerifiedTape::verify(header, &chunks, mark).unwrap()),
            historical: None,
            actor,
            private: true,
        }
    }
    fn candidate() -> super::super::scenarios::Candidate {
        super::super::scenarios::Candidate::resolve(
            &mush_roll::RollStore::default(),
            &crate::store::TacsimStore::default(),
            ObjId(99),
            "Standard/Line",
        )
        .unwrap()
    }
    #[test]
    fn access_metadata_tracks_verified_selection_and_is_absent_from_picture_wire() {
        let mut machine = ReplayMachine::new(tape(), ObjId(99), ObjId(4)).unwrap();
        let origin = machine
            .lease
            .tape
            .header()
            .seals
            .iter()
            .find(|seal| seal.unit == ObjId(4))
            .unwrap()
            .requested
            .origin
            .clone();
        assert_eq!(machine.picture.access_origin.as_ref(), Some(&origin));
        let callsign = machine
            .session
            .arena()
            .unwrap()
            .callsigns()
            .iter()
            .find(|(unit, _)| *unit == ObjId(4))
            .unwrap()
            .1
            .to_string();
        machine.bench_act_as(&callsign).unwrap();
        assert_eq!(machine.picture.access_origin.as_ref(), Some(&origin));
        machine
            .bench_edit(super::super::experiment::Edit::Swap {
                candidate: candidate(),
                serial: 1,
            })
            .unwrap();
        // Candidate access is independently retained by the world Entry; the
        // selected reference still binds the verified tape's original source.
        assert_eq!(machine.picture.access_origin.as_ref(), Some(&origin));
        machine
            .bench_edit(super::super::experiment::Edit::Reference)
            .unwrap();
        let wire = serde_json::to_value(machine.picture.as_ref()).unwrap();
        assert!(wire.get("access_origin").is_none());
        assert_eq!(machine.picture.access_origin.as_ref(), Some(&origin));
    }
    #[test]
    fn terminal_target_drains_same_time_settle_and_ledger_ack_to_the_exact_cursor() {
        let mut machine = ReplayMachine::new(tape(), ObjId(99), ObjId(4)).unwrap();
        let end = machine.lease.tape.duration_ms();
        let events = machine.lease.tape.event_count();
        assert!(end > 1000);
        machine.begin(ReplayCommand::RunTo(1000)).unwrap();
        for _ in 0..300 {
            if machine.step(250).unwrap().terminal == Some(Status::Complete) {
                break;
            }
        }
        assert!(machine.goal.is_none());
        assert!(machine.picture.at_ms >= 1000 && machine.picture.at_ms < end);
        assert!(
            !machine.picture.complete,
            "intermediate targets still stop early"
        );
        machine.begin(ReplayCommand::RunTo(end)).unwrap();
        for _ in 0..300 {
            let turn = machine.step(250).unwrap();
            assert_eq!(
                turn.terminal, None,
                "the terminal barriers remain unpaid/unprocessed"
            );
            if machine.picture.event + 2 == events {
                break;
            }
        }
        assert_eq!(machine.picture.at_ms, end);
        assert_eq!(machine.picture.event + 2, events);
        assert!(!machine.picture.complete);
        assert!(machine.session.arena().unwrap().awaiting.is_empty());
        assert_eq!(machine.session.arena().unwrap().ledger.flushed_upto(), 0);

        let settle = machine.step(0).unwrap();
        assert_eq!(settle.simulation_ms, 0);
        assert_eq!(settle.terminal, None);
        assert_eq!(machine.picture.event + 1, events);
        let ack = machine.step(0).unwrap();
        assert_eq!(ack.simulation_ms, 0);
        assert_eq!(ack.terminal, Some(Status::Complete));
        assert_eq!(machine.picture.event, events);
        assert_eq!(machine.picture.at_ms, end);
        assert!(machine.picture.complete);
        let arena = machine.session.arena().unwrap();
        assert!(arena.ledger.flushed_upto() > 0);
        assert_eq!(arena.ledger.flushed_upto(), arena.ledger.last_closed_seq());
        assert_eq!(
            machine.picture.state_hash,
            format!("{:016x}", state_hash(arena))
        );

        machine.begin(ReplayCommand::RunTo(end)).unwrap();
        let again = machine.step(0).unwrap();
        assert_eq!(again.terminal, Some(Status::Complete));
        assert_eq!(again.simulation_ms, 0);
        assert_eq!(machine.picture.event, events);
    }
    #[test]
    fn diff_edits_wait_for_captured_pulse_jobs_and_paid_step_settles_them() {
        let mut machine = ReplayMachine::new(tape(), ObjId(99), ObjId(4)).unwrap();
        machine
            .bench_edit(super::super::experiment::Edit::Diff(candidate()))
            .unwrap();
        for _ in 0..300 {
            machine.session.advance_substep().unwrap();
            if machine
                .session
                .arena()
                .is_some_and(|a| !a.awaiting.is_empty())
            {
                break;
            }
        }
        assert!(
            machine
                .session
                .arena()
                .is_some_and(|a| !a.awaiting.is_empty()),
            "fixture must stop after a real Pulse and before Settle"
        );
        let calls = machine.diff.as_ref().unwrap().picture().unwrap().thinks;
        assert_eq!(machine.bench_unit_count(), Some(3));
        assert_eq!(
            machine.bench_edit(super::super::experiment::Edit::DiffOff),
            Err(BulkError::Busy)
        );
        assert_eq!(
            machine.bench_edit(super::super::experiment::Edit::Diff(candidate())),
            Err(BulkError::Busy)
        );
        assert_eq!(machine.bench_unit_count(), Some(3));
        machine.refresh().unwrap();
        machine.goal = Some(machine.picture.at_ms);
        let mut ended = false;
        for _ in 0..30 {
            if machine.step(250).unwrap().terminal == Some(Status::Complete) {
                ended = true;
                break;
            }
        }
        assert!(ended);
        assert!(machine.session.arena().unwrap().awaiting.is_empty());
        assert!(machine.diff.as_ref().unwrap().picture().unwrap().thinks > calls);
        machine
            .bench_edit(super::super::experiment::Edit::DiffOff)
            .unwrap();
        assert_eq!(machine.bench_unit_count(), Some(2));
        assert!(machine.diff.is_none());
    }
    #[test]
    fn mutable_fork_and_possession_leave_reference_hash_and_cursor_untouched() {
        let mut machine = ReplayMachine::new(tape(), ObjId(99), ObjId(4)).unwrap();
        let reference_hash = crate::arena::state_hash(machine.session.arena().unwrap());
        let reference_event = machine.session.event();
        machine
            .bench_edit(super::super::experiment::Edit::Possess { serial: 7 })
            .unwrap();
        assert!(machine.picture.possessed);
        assert!(
            matches!(&machine.picture.timeline,super::super::branch::Timeline::Fork{serial,..} if serial=="7")
        );
        assert!(machine.bench_act_as("RA").is_err());
        assert!(
            machine
                .bench_edit(super::super::experiment::Edit::Diff(candidate()))
                .is_err()
        );
        machine
            .bench_edit(super::super::experiment::Edit::Do("STARTUP".into()))
            .unwrap();
        machine.begin(ReplayCommand::RunTo(1000)).unwrap();
        for _ in 0..200 {
            if machine.step(250).unwrap().terminal == Some(Status::Complete) {
                break;
            }
        }
        assert!(machine.goal.is_none());
        let fork_hash = machine.picture.state_hash.clone();
        assert_eq!(
            crate::arena::state_hash(machine.session.arena().unwrap()),
            reference_hash
        );
        assert_eq!(machine.session.event(), reference_event);
        machine.begin(ReplayCommand::BackTo(0)).unwrap();
        for _ in 0..20 {
            if machine.step(250).unwrap().terminal == Some(Status::Complete) {
                break;
            }
        }
        machine.begin(ReplayCommand::RunTo(1000)).unwrap();
        for _ in 0..200 {
            if machine.step(250).unwrap().terminal == Some(Status::Complete) {
                break;
            }
        }
        assert_eq!(machine.picture.state_hash, fork_hash);
        machine
            .bench_edit(super::super::experiment::Edit::Release)
            .unwrap();
        assert!(!machine.picture.possessed);
        machine
            .bench_edit(super::super::experiment::Edit::Reference)
            .unwrap();
        assert!(matches!(
            machine.picture.timeline,
            super::super::branch::Timeline::Reference
        ));
        assert_eq!(machine.picture.state_hash, format!("{reference_hash:016x}"));
        assert_eq!(machine.session.event(), reference_event);
    }
    #[test]
    fn swap_is_a_fresh_private_runtime_and_does_not_change_reference_custody() {
        let mut machine = ReplayMachine::new(tape(), ObjId(99), ObjId(4)).unwrap();
        let old = machine
            .session
            .arena()
            .unwrap()
            .unit(ObjId(4))
            .unwrap()
            .seal
            .clone();
        let epochs = crate::arena::Channel::ALL
            .map(|c| machine.session.arena().unwrap().epochs.current(ObjId(4), c));
        machine
            .bench_edit(super::super::experiment::Edit::Swap {
                candidate: candidate(),
                serial: 8,
            })
            .unwrap();
        let arena = machine.branch.as_ref().unwrap().arena().unwrap();
        assert!(arena.minds[&ObjId(4)].memory.is_empty());
        assert!(arena.minds[&ObjId(4)].pending.is_none());
        assert!(arena.awaiting.is_empty());
        assert_eq!(
            old.channels,
            vec![crate::arena::Channel::Helm, crate::arena::Channel::Sensors],
            "this reference is deliberately an unarmed hull"
        );
        for (i, c) in crate::arena::Channel::ALL.into_iter().enumerate() {
            // SWAP renews the sealed stations. It must not invent authority
            // for the absent Fire station on this unarmed reference.
            assert_eq!(
                arena.epochs.current(ObjId(4), c),
                epochs[i] + u64::from(old.channels.contains(&c)),
                "fork {c:?} epoch"
            );
            assert_eq!(
                machine.session.arena().unwrap().epochs.current(ObjId(4), c),
                epochs[i],
                "the reference {c:?} epoch is unchanged"
            );
        }
        assert_eq!(
            machine
                .session
                .arena()
                .unwrap()
                .unit(ObjId(4))
                .unwrap()
                .seal,
            old
        );
        assert!(machine.picture.diff.is_none());
        assert!(machine.picture.refusals.is_empty());
        machine
            .bench_edit(super::super::experiment::Edit::Reference)
            .unwrap();
        assert_eq!(
            machine
                .session
                .arena()
                .unwrap()
                .unit(ObjId(4))
                .unwrap()
                .seal,
            old
        );
    }
}
