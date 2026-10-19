//! Verified private replay. Every output is recomputed by the sealed runners.
//! Work yields between recorded application barriers and never reaches a world,
//! provider, live hull or LLM. Bench/proving own admission and CPU quotas.
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::arena::input_tape::{
    EngineStamp, MAX_CHUNKS, MAX_MAX_BYTES, TAPE_FORMAT, TapeChunk, TapeCursor, TapeError,
    TapeEvent, TapeHeader, TapeMark, TapeStatus, ThinkIdentity, ThinkWitness, encoded_len,
};
use crate::arena::{Arena, Snapshot, state_hash};

impl EngineStamp {
    pub fn current() -> Self {
        Self {
            engine: crate::arena::input_tape::REPLAY_ENGINE.into(),
            abi: crate::minds::MIND_ABI.into(),
            snapshot: crate::arena::snapshot::SNAPSHOT_FORMAT,
            physics: mush_atlas::physics::LOCOMOTION_DIGEST.into(),
            standard: crate::arena::digest_text(&crate::minds::content::standard_digest().0),
        }
    }
}

/// Shared proving/replay admission. Takes a frozen owned header, never a live
/// arena. Strict validation and compilation finish before the first think.
pub(crate) fn open_frozen_header(header: &TapeHeader) -> Result<Arena, TapeError> {
    header.validate_shape()?;
    if header.engine != EngineStamp::current() {
        return Err(TapeError::Mismatch(
            "engine, source, ABI or physics identity changed".into(),
        ));
    }
    if encoded_len(header)? as u64 > header.limit {
        return Err(incomplete("opening byte bound"));
    }
    let definition = header.ground.clone().restore().map_err(incomplete)?;
    let mut arena =
        Snapshot::decode(&header.opening, definition).map_err(|e| incomplete(e.to_string()))?;
    if arena.units.len() != header.seals.len() || arena.tape_cursor.is_some() {
        return Err(incomplete("opening roster or cursor"));
    }
    for seal in &header.seals {
        seal.verify(&arena.classes, &seal.weapons)
            .map_err(|e| incomplete(e.to_string()))?;
        let Some(unit) = arena.unit(seal.unit) else {
            return Err(incomplete("opening seal has no hull"));
        };
        if unit.seal != seal.seal_ref() {
            return Err(incomplete("opening hull differs from original seal"));
        }
    }
    install_header_sources(header, &mut arena)?;
    arena.clock = crate::arena::clock::Clock::from_replay(header.clock);
    Ok(arena)
}
fn install_header_sources(header: &TapeHeader, arena: &mut Arena) -> Result<(), TapeError> {
    let sources: BTreeMap<[u8; 32], String> = header
        .sources
        .iter()
        .map(|s| (s.seal, s.text.clone()))
        .collect();
    let cfg = arena.arena_cfg.clone();
    super::install_sealed_runners(arena, &cfg, &|key| sources.get(key).cloned())
        .map_err(|e| TapeError::Mismatch(format!("sealed source does not compile: {e}")))?;
    Ok(())
}

#[derive(Clone, Debug)]
pub struct VerifiedTape {
    header: Arc<TapeHeader>,
    events: Vec<(TapeEvent, Option<TapeCursor>)>,
    mark: TapeMark,
    artifact: [u8; 32],
}
impl VerifiedTape {
    /// All durable chunks, sources, seals and protocol identities are checked
    /// before any call to step/think. Extra tail chunks refuse as well as gaps.
    pub fn verify(
        header: Arc<TapeHeader>,
        chunks: &[Arc<TapeChunk>],
        mark: TapeMark,
    ) -> Result<Self, TapeError> {
        header.validate_shape()?;
        if header.engine != EngineStamp::current() {
            return Err(TapeError::Mismatch(
                "engine, source, ABI, snapshot or physics identity changed".into(),
            ));
        }
        if mark.format != TAPE_FORMAT
            || mark.match_id != header.match_id
            || chunks.len() > MAX_CHUNKS
        {
            return Err(incomplete("record identity or chunk count"));
        }
        if !matches!(mark.status, TapeStatus::Complete { .. }) {
            return Err(TapeError::Unavailable(match &mark.status {
                TapeStatus::Incomplete { reason } => reason.clone(),
                _ => "recording has not completed".into(),
            }));
        }
        let mut bytes = encoded_len(&*header)? as u64;
        let mut cursor = header.cursor()?;
        let mut events = Vec::new();
        for chunk in chunks {
            bytes = bytes
                .checked_add(encoded_len(&**chunk)? as u64)
                .ok_or_else(|| incomplete("byte count overflow"))?;
            if bytes > header.limit || bytes > MAX_MAX_BYTES {
                return Err(incomplete("recording byte bound"));
            }
            cursor = chunk.after(cursor)?;
            for (index, event) in chunk.events.iter().enumerate() {
                events.push((
                    event.clone(),
                    (index + 1 == chunk.events.len()).then_some(cursor),
                ));
            }
        }
        if cursor != mark.cursor || bytes != mark.bytes {
            return Err(incomplete("completion cursor or byte count differs"));
        }
        let artifact = crate::arena::input_tape::hash_value(b"tacsim-tape-artifact-v1", &mark)?;
        let tape = Self {
            header,
            events,
            mark,
            artifact,
        };
        // Decode strict original schema/catalog/physics and compile every source
        // now. Compilation is not simulation and cannot mutate any live state.
        let _ = tape.open_arena()?;
        Ok(tape)
    }
    fn open_arena(&self) -> Result<Arena, TapeError> {
        let mut arena = open_frozen_header(&self.header)?;
        arena.tape_cursor = Some(self.header.cursor()?);
        arena.tape_witnesses.enable();
        Ok(arena)
    }
    pub fn artifact_digest(&self) -> [u8; 32] {
        self.artifact
    }
    pub fn encoded_bytes(&self) -> u64 {
        self.mark.bytes
    }
    pub fn event_count(&self) -> usize {
        self.events.len()
    }
    pub fn unit_count(&self) -> usize {
        self.header.seals.len()
    }
    pub fn match_id(&self) -> u64 {
        self.header.match_id
    }
    pub fn duration_ms(&self) -> i64 {
        match self.mark.status {
            TapeStatus::Complete { at, .. } => at,
            _ => 0,
        }
    }
    pub fn header(&self) -> &TapeHeader {
        &self.header
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayProgress {
    pub event: usize,
    pub at_ms: i64,
    pub complete: bool,
    pub simulated_ms: u64,
    pub thinks: usize,
}

pub struct ReplaySession {
    tape: Arc<VerifiedTape>,
    arena: Option<Arena>,
    pulse: Option<crate::arena::step::OwnedPulse>,
    event: usize,
    at_ms: i64,
    native_substep_ms: u32,
    last_thinks: Vec<ThinkWitness>,
}
impl ReplaySession {
    pub fn new(tape: Arc<VerifiedTape>) -> Result<Self, TapeError> {
        let arena = tape.open_arena()?;
        let at_ms = arena.now_ms;
        let native_substep_ms = arena.arena_cfg.tempo.substep_max_ms;
        Ok(Self {
            tape,
            arena: Some(arena),
            pulse: None,
            event: 0,
            at_ms,
            native_substep_ms,
            last_thinks: Vec::new(),
        })
    }
    /// No arena can escape while its pulse is partially applied. Public own-hull
    /// views and branch edits are available only at complete application barriers.
    pub fn arena(&self) -> Option<&Arena> {
        self.arena.as_ref()
    }
    pub(crate) fn arena_mut(&mut self) -> Option<&mut Arena> {
        self.arena.as_mut()
    }
    pub fn next_simulation_ms(&self) -> u64 {
        self.pulse
            .as_ref()
            .map_or(0, |pulse| u64::from(pulse.next_simulation_ms()))
    }
    pub fn unit_count(&self) -> usize {
        self.tape.unit_count()
    }
    pub fn native_substep_ms(&self) -> u32 {
        self.native_substep_ms
    }
    /// A settled-barrier fork copies frozen state; it never runs unpaid catchup.
    pub fn fork_settled(&self) -> Result<Self, TapeError> {
        let source = self
            .arena
            .as_ref()
            .ok_or_else(|| incomplete("pulse is still active"))?;
        let snapshot = Snapshot::encode(source).map_err(|e| incomplete(e.to_string()))?;
        let definition = self
            .tape
            .header
            .ground
            .clone()
            .restore()
            .map_err(incomplete)?;
        let mut arena = Snapshot::decode(&snapshot.document, definition)
            .map_err(|e| incomplete(e.to_string()))?;
        install_header_sources(&self.tape.header, &mut arena)?;
        arena.clock = crate::arena::clock::Clock::from_replay(source.clock.replay_state());
        arena.tape_witnesses.enable();
        Ok(Self {
            tape: Arc::clone(&self.tape),
            arena: Some(arena),
            pulse: None,
            event: self.event,
            at_ms: self.at_ms,
            native_substep_ms: self.native_substep_ms,
            last_thinks: self.last_thinks.clone(),
        })
    }
    /// Consume a settled private clone for a counterfactual timeline. A
    /// delivered answer may await its future landing; unanswered collected
    /// thinks and an active pulse may not cross this ownership boundary.
    pub(crate) fn into_settled_arena(self) -> Result<Arena, TapeError> {
        if self.pulse.is_some() || self.arena.as_ref().is_none_or(|a| !a.awaiting.is_empty()) {
            return Err(incomplete("fork is not at a settled barrier"));
        }
        self.arena
            .ok_or_else(|| incomplete("fork arena unavailable"))
    }
    pub fn event(&self) -> usize {
        self.event
    }
    pub fn last_thinks(&self) -> &[ThinkWitness] {
        &self.last_thinks
    }
    pub fn rewind(&mut self) -> Result<(), TapeError> {
        let arena = self.tape.open_arena()?;
        self.at_ms = arena.now_ms;
        self.arena = Some(arena);
        self.pulse = None;
        self.event = 0;
        self.last_thinks.clear();
        Ok(())
    }
    pub fn advance_substep(&mut self) -> Result<ReplayProgress, TapeError> {
        self.advance(1)
    }
    /// Bounded scheduler slices, not recorded pulses. Each slice applies at most
    /// one captured native substep or one non-time phase. A custom larger native
    /// substep delays fairness by its own validated bound; it is never split.
    pub fn advance(&mut self, max_slices: usize) -> Result<ReplayProgress, TapeError> {
        let start = self.at_ms;
        let mut thinks = 0;
        for _ in 0..max_slices.clamp(1, 64) {
            if self.event == self.tape.events.len() {
                break;
            }
            let (recorded, cursor) = self
                .tape
                .events
                .get(self.event)
                .cloned()
                .ok_or_else(|| incomplete("replay event cursor is outside the tape"))?;
            if let Some(mut pulse) = self.pulse.take() {
                let slice = pulse.advance();
                self.at_ms = slice.at_ms;
                if !slice.done {
                    self.pulse = Some(pulse);
                    continue;
                }
                let (arena, output) = match pulse.finish() {
                    Ok(done) => done,
                    Err(pulse) => {
                        self.pulse = Some(pulse);
                        return Err(incomplete("pulse completion unavailable"));
                    }
                };
                self.arena = Some(arena);
                self.compare_pulse(&recorded, &output)?;
            } else {
                self.last_thinks.clear();
                if let TapeEvent::Pulse {
                    wall,
                    controls,
                    intents,
                    cards,
                    ..
                } = &recorded
                {
                    let mut arena = self
                        .arena
                        .take()
                        .ok_or_else(|| incomplete("ready replay arena unavailable"))?;
                    for card in cards {
                        arena.stage_card(card.unit, card.change.clone());
                    }
                    self.pulse = Some(crate::arena::step::OwnedPulse::new(
                        arena, *wall, controls, intents,
                    ));
                    continue;
                }
                match &recorded {
                    TapeEvent::Settle {
                        thinks: expected,
                        at,
                        hash,
                    } => {
                        self.arena
                            .as_mut()
                            .ok_or_else(|| incomplete("ready replay arena unavailable"))?
                            .settle_inline();
                        self.compare_thinks(expected)?;
                        self.compare_state(*at, *hash)?;
                    }
                    TapeEvent::Stop { reason } => self
                        .arena
                        .as_mut()
                        .ok_or_else(|| incomplete("ready replay arena unavailable"))?
                        .stop(*reason),
                    TapeEvent::LedgerAck { through } => self
                        .arena
                        .as_mut()
                        .ok_or_else(|| incomplete("ready replay arena unavailable"))?
                        .ledger
                        .mark_flushed(*through),
                    TapeEvent::TempoFloor { period } => self
                        .arena
                        .as_mut()
                        .ok_or_else(|| incomplete("ready replay arena unavailable"))?
                        .clock
                        .set_floor(*period),
                    TapeEvent::Watch { on } => {
                        let arena = self
                            .arena
                            .as_mut()
                            .ok_or_else(|| incomplete("ready replay arena unavailable"))?;
                        arena.watched = *on;
                        if *on {
                            arena.last_observer_s = None;
                        }
                    }
                    TapeEvent::Restart {
                        hash,
                        clock,
                        ledger_flushed,
                    } => {
                        let current = self
                            .arena
                            .as_ref()
                            .ok_or_else(|| incomplete("ready replay arena unavailable"))?;
                        if state_hash(current) != *hash {
                            return Err(self.different("restart checkpoint"));
                        }
                        let snapshot =
                            Snapshot::encode(current).map_err(|e| incomplete(e.to_string()))?;
                        let ground = self
                            .tape
                            .header
                            .ground
                            .clone()
                            .restore()
                            .map_err(incomplete)?;
                        let mut arena = Snapshot::decode(&snapshot.document, ground)
                            .map_err(|e| incomplete(e.to_string()))?;
                        install_header_sources(&self.tape.header, &mut arena)?;
                        arena.ledger.mark_flushed(*ledger_flushed);
                        arena.clock = crate::arena::clock::Clock::from_replay(*clock);
                        arena.tape_witnesses.enable();
                        self.arena = Some(arena);
                    }
                    TapeEvent::Pulse { .. } => return Err(incomplete("unexpected nested pulse")),
                }
            }
            thinks += self.last_thinks.len();
            let arena = self
                .arena
                .as_mut()
                .ok_or_else(|| incomplete("completed replay barrier unavailable"))?;
            self.at_ms = arena.now_ms;
            if let Some(cursor) = cursor {
                arena.tape_cursor = Some(cursor);
            }
            self.event += 1;
        }
        let complete = self.event == self.tape.events.len();
        if complete && let TapeStatus::Complete { at, hash, end } = &self.tape.mark.status {
            self.compare_state(*at, *hash)?;
            if self
                .arena
                .as_ref()
                .ok_or_else(|| incomplete("complete replay unavailable"))?
                .ended
                .as_ref()
                != Some(end)
            {
                return Err(self.different("terminal outcome"));
            }
        }
        Ok(ReplayProgress {
            event: self.event,
            at_ms: self.at_ms,
            complete,
            simulated_ms: self.at_ms.saturating_sub(start).max(0) as u64,
            thinks,
        })
    }
    fn compare_pulse(
        &mut self,
        recorded: &TapeEvent,
        output: &crate::arena::step::StepOutput,
    ) -> Result<(), TapeError> {
        let TapeEvent::Pulse {
            hashes,
            thinks,
            pending,
            at,
            hash,
            next_due,
            ..
        } = recorded
        else {
            return Err(incomplete("active pulse has no recorded pulse"));
        };
        let arena = self
            .arena
            .as_ref()
            .ok_or_else(|| incomplete("completed replay pulse unavailable"))?;
        let scheduled: Vec<_> = output
            .thinks
            .iter()
            .map(|t| ThinkIdentity::capture(t, t.epochs))
            .collect();
        if &arena.substep_hashes != hashes {
            return Err(self.different("substep hash"));
        }
        if &scheduled != pending {
            return Err(self.different("pending task identity, epochs or generation"));
        }
        if output.next_due_ms != *next_due {
            return Err(self.different("recorded clock schedule"));
        }
        self.compare_thinks(thinks)?;
        self.compare_state(*at, *hash)
    }
    fn compare_thinks(&mut self, expected: &[ThinkWitness]) -> Result<(), TapeError> {
        self.last_thinks = self
            .arena
            .as_mut()
            .ok_or_else(|| incomplete("completed replay barrier unavailable"))?
            .tape_witnesses
            .take()?;
        if self.last_thinks != expected {
            return Err(self.different("think intents, memory, fault, sample or frozen identity"));
        }
        Ok(())
    }
    fn compare_state(&self, at: i64, hash: u64) -> Result<(), TapeError> {
        let arena = self
            .arena
            .as_ref()
            .ok_or_else(|| incomplete("pulse is still active"))?;
        if arena.now_ms != at || state_hash(arena) != hash {
            return Err(self.different("arena time or state hash"));
        }
        Ok(())
    }
    fn different(&self, what: &str) -> TapeError {
        TapeError::Mismatch(format!("event {} at {} ms: {what}", self.event, self.at_ms))
    }
}

fn incomplete(why: impl Into<String>) -> TapeError {
    TapeError::Incomplete(why.into())
}

/// Proving's synchronous path records the same two barriers as the live pool:
/// pre-settlement Pulse, then the exact inline answer delivery. It does not turn
/// the post-settlement state into a fictitious Pulse. With the continuation cut,
/// the caller drives the pulse slices first and calls this finalizer at its end.
pub fn record_completed_pulse(
    arena: &mut Arena,
    recorder: &mut crate::arena::input_tape::TapeRecorder,
    wall: i64,
    controls: &[crate::arena::intents::Control],
    intents: &[crate::arena::intents::Intent],
    cards: Vec<crate::arena::input_tape::TapeCard>,
    out: &crate::arena::step::StepOutput,
) -> Result<Vec<crate::arena::input_tape::TapeWrite>, TapeError> {
    let pending = out
        .thinks
        .iter()
        .map(|think| ThinkIdentity::capture(think, think.epochs))
        .collect();
    let thinks = arena.tape_witnesses.take()?;
    recorder.push(TapeEvent::Pulse {
        wall,
        controls: controls.to_vec(),
        intents: intents.to_vec(),
        cards,
        hashes: arena.substep_hashes.clone(),
        thinks,
        pending,
        at: arena.now_ms,
        hash: state_hash(arena),
        next_due: out.next_due_ms,
    })
}
pub fn recorded_step_settled(
    arena: &mut Arena,
    recorder: &mut crate::arena::input_tape::TapeRecorder,
    wall: i64,
    controls: &[crate::arena::intents::Control],
    intents: &[crate::arena::intents::Intent],
    cards: Vec<crate::arena::input_tape::TapeCard>,
) -> Result<
    (
        crate::arena::step::StepOutput,
        Vec<crate::arena::input_tape::TapeWrite>,
    ),
    TapeError,
> {
    for card in &cards {
        arena.stage_card(card.unit, card.change.clone());
    }
    let mut out = arena.step(wall, controls, intents);
    let mut writes = record_completed_pulse(arena, recorder, wall, controls, intents, cards, &out)?;
    let thinks = std::mem::take(&mut out.thinks);
    if !thinks.is_empty() {
        let settled = arena.deliver_answers(crate::arena::mind::run_batch_inline(&thinks));
        out.motor.mind_faults = out.motor.mind_faults.saturating_add(settled.faults);
        out.motor.mind_overflows = out.motor.mind_overflows.saturating_add(settled.overflows);
        writes.extend(recorder.push(TapeEvent::Settle {
            thinks: arena.tape_witnesses.take()?,
            at: arena.now_ms,
            hash: state_hash(arena),
        })?);
    }
    Ok((out, writes))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::arena::input_tape::*;
    use crate::arena::intents::{Control, ControlState, Intent, IntentKind};
    use crate::arena::state::{Channel, Controller, StopReason, Team};
    use crate::arena::test_support::{cannon, custody, pose, tactical_unit_cored, tank};
    use crate::arena::{CoreClass, SealEngine};
    use crate::config::{ArenaConfig, MatchConfig, MindsConfig, SensorsConfig};
    use mush_core::ObjId;

    pub(crate) fn opening(count: u32) -> (Arena, Arc<TapeHeader>) {
        opening_source(count, super::super::content::STANDARD_RHAI)
    }

    // Deterministic quiet author program: real compiled thinks and changing
    // canonical memory, without an early combat end masking duration coverage.
    const QUIET_SOURCE: &str = r#"fn think(v, c) {
        if !("visits" in this) { this.visits = 0; }
        this.visits += 1;
        if this.visits == 1 { startup(); }
    }"#;

    fn opening_source(count: u32, source: &str) -> (Arena, Arc<TapeHeader>) {
        use sha2::{Digest as _, Sha256};
        let mut units = Vec::new();
        for n in 0..count {
            let id = ObjId(n + 1);
            let team = if n % 2 == 0 { Team::Red } else { Team::Blue };
            let mut unit = tank(
                id,
                team,
                pose(((n % 8) * 1000 + 500) as i64, ((n / 8) * 1000 + 500) as i64),
            );
            unit.unit = tactical_unit_cored(id, CoreClass::III);
            unit.controller = Controller::Mind;
            unit.weapons = vec![cannon()];
            let mut terms = crate::match_::SealTerms::fixture();
            terms.rated = CoreClass::III;
            terms.class_cap = CoreClass::III;
            terms.engine_for_minds = SealEngine::Scripted;
            if source != super::super::content::STANDARD_RHAI {
                let digest: [u8; 32] = Sha256::digest(source.as_bytes()).into();
                terms.identity = crate::arena::profile_seal::MindIdentity {
                    mind: "Quiet replay fixture".into(),
                    origin: crate::arena::profile_seal::MindOrigin::Draft {
                        character: ObjId(1000 + n),
                        mind: 1,
                        revision: 1,
                        digest: mush_core::DefinitionDigest::from_bytes(digest),
                    },
                    source_sha256: digest,
                };
            }
            terms.principal = Some(crate::deliberation::order::CharacterRef::Character {
                id: ObjId(1000 + n),
            });
            let profiles = ["Line", "Sniper", "Hunter", "Sentry"]
                .into_iter()
                .map(|name| {
                    (
                        name.into(),
                        crate::arena::card::preset(name, &terms.rom_cfg).unwrap(),
                    )
                })
                .collect();
            unit.reseal(&terms.with_profiles(profiles)).unwrap();
            units.push(unit);
        }
        let seals = units.iter().map(|u| u.seal.clone()).collect::<Vec<_>>();
        let sources: std::collections::BTreeMap<[u8; 32], String> = seals
            .iter()
            .map(|seal| (seal.digest(), source.to_owned()))
            .collect();
        let mut arena = Arena::new_with_minds(
            custody(units, vec![]),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
            &MindsConfig::default(),
        )
        .unwrap();
        let cfg = arena.arena_cfg.clone();
        super::super::install_sealed_runners(&mut arena, &cfg, &|key| sources.get(key).cloned())
            .unwrap();
        let header = Arc::new(
            TapeHeader::capture(
                &arena,
                7,
                seals,
                &sources,
                EngineStamp::current(),
                DEFAULT_MAX_BYTES,
            )
            .unwrap(),
        );
        (arena, header)
    }
    fn completed(cadence: i64, restart: bool) -> (Arc<VerifiedTape>, u64, usize) {
        let (mut arena, header) = opening(2);
        let mut recorder = TapeRecorder::open(&header).unwrap();
        arena.tape_cursor = Some(recorder.cursor());
        arena.tape_witnesses.enable();
        let mut writes = Vec::new();
        let mut wall = 0;
        for half in 0..2 {
            let end = (half + 1) * 5000;
            while arena.now_ms < end {
                assert!(
                    arena.ended.is_none(),
                    "fixture must remain alive before the requested stop"
                );
                let intents = if half == 1 && arena.now_ms == 5000 {
                    vec![Intent {
                        unit: ObjId(1),
                        channel: Channel::Helm,
                        epoch: 0,
                        seq: 55,
                        kind: IntentKind::Profile { index: 1 },
                    }]
                } else {
                    vec![]
                };
                let (_, new) =
                    recorded_step_settled(&mut arena, &mut recorder, wall, &[], &intents, vec![])
                        .unwrap();
                writes.extend(new);
                wall += cadence;
            }
            if half == 0 && restart {
                let (cursor, checkpoint) = recorder.checkpoint().unwrap();
                writes.extend(checkpoint);
                arena.tape_cursor = Some(cursor);
                let frozen = Snapshot::encode(&arena).unwrap();
                let ground = header.ground.clone().restore().unwrap();
                arena = Snapshot::decode(&frozen.document, ground).unwrap();
                install_header_sources(&header, &mut arena).unwrap();
                arena.resume_at(0);
                arena.tape_witnesses.enable();
                wall = cadence;
                writes.extend(
                    recorder
                        .push(TapeEvent::Restart {
                            hash: state_hash(&arena),
                            clock: arena.clock.replay_state(),
                            ledger_flushed: arena.ledger.flushed_upto(),
                        })
                        .unwrap(),
                );
            }
        }
        // An authority change and staged diagnostic edit cross the exact input
        // boundary too; one zero-time pulse leaves its accepted profile readable.
        let controls = [Control {
            unit: ObjId(1),
            channel: Channel::Helm,
            epoch: 1,
            state: ControlState::Live,
            pilot: Some(ObjId(1000)),
        }];
        let intent = Intent {
            unit: ObjId(1),
            channel: Channel::Helm,
            epoch: 1,
            seq: 99,
            kind: IntentKind::Heading { mdeg: 90_000 },
        };
        let cards = vec![TapeCard {
            unit: ObjId(2),
            change: crate::arena::card::CardChange::Set {
                slider: "rally_s".into(),
                value: "15".into(),
            },
        }];
        let (_, new) = recorded_step_settled(
            &mut arena,
            &mut recorder,
            wall - cadence,
            &controls,
            &[intent],
            cards,
        )
        .unwrap();
        writes.extend(new);
        writes.extend(recorder.push(TapeEvent::Watch { on: true }).unwrap());
        arena.watched = true;
        arena.last_observer_s = None;
        writes.extend(
            recorder
                .push(TapeEvent::TempoFloor { period: Some(1000) })
                .unwrap(),
        );
        arena.clock.set_floor(Some(1000));
        writes.extend(recorder.push(TapeEvent::LedgerAck { through: 0 }).unwrap());
        arena.ledger.mark_flushed(0);
        writes.extend(
            recorder
                .push(TapeEvent::Stop {
                    reason: StopReason::Abort,
                })
                .unwrap(),
        );
        arena.stop(StopReason::Abort);
        let (_, new) =
            recorded_step_settled(&mut arena, &mut recorder, wall, &[], &[], vec![]).unwrap();
        writes.extend(new);
        let final_hash = state_hash(&arena);
        writes.extend(
            recorder
                .finish(arena.now_ms, final_hash, arena.ended.unwrap())
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
        let witnesses = chunks
            .iter()
            .flat_map(|c| &c.events)
            .map(|event| match event {
                TapeEvent::Pulse { thinks, .. } | TapeEvent::Settle { thinks, .. } => thinks.len(),
                _ => 0,
            })
            .sum();
        assert!(
            witnesses > 0,
            "actual sealed Rhai runners produced witnessed answers"
        );
        (
            Arc::new(VerifiedTape::verify(header, &chunks, mark).unwrap()),
            final_hash,
            witnesses,
        )
    }
    #[test]
    fn m6_actual_rerun_recomputes_all_thinks_hashes_inputs_and_resume_at_three_cadences() {
        for cadence in [5000, 1000, 500] {
            for restart in [false, true] {
                let (tape, hash, expected_thinks) = completed(cadence, restart);
                let mut replay = ReplaySession::new(tape).unwrap();
                let mut seen = 0;
                let mut turns = 0;
                loop {
                    let cost = replay.next_simulation_ms();
                    let progress = replay.advance_substep().unwrap();
                    turns += 1;
                    assert_eq!(progress.simulated_ms, cost);
                    if replay.arena().is_none() {
                        assert!(replay.fork_settled().is_err());
                    }
                    seen += progress.thinks;
                    assert!(turns < 2000);
                    if progress.complete {
                        break;
                    }
                }
                assert_eq!(seen, expected_thinks);
                assert_eq!(state_hash(replay.arena().unwrap()), hash);
                let fork = replay.fork_settled().unwrap();
                assert_eq!(state_hash(fork.arena().unwrap()), hash);
            }
        }
    }
    #[test]
    fn m7_old_tape_engine_and_snapshot_refuse_before_replay_starts() {
        let (_, header) = opening(2);
        assert_eq!(TAPE_FORMAT, 2);
        assert_eq!(EngineStamp::current().engine, "tacsim-engine-m7-v1");
        assert_eq!(EngineStamp::current().snapshot, 16);
        for mutation in 0..4 {
            let mut old = (*header).clone();
            match mutation {
                0 => old.format = 1,
                1 => old.engine.engine = "tacsim-engine-m6.5-v1".into(),
                2 => old.engine.snapshot = 13,
                _ => {
                    old.opening["format"] = serde_json::json!(13);
                    old.opening["state"] = serde_json::json!("malformed old state");
                    assert!(matches!(
                        Snapshot::head(&old.opening),
                        Err(crate::arena::snapshot::SnapshotError::Format {
                            found: 13,
                            expected: 16
                        })
                    ));
                }
            }
            assert!(
                open_frozen_header(&old).is_err(),
                "old protocol mutation {mutation}"
            );
        }
    }

    #[test]
    fn m6_frozen_opening_refuses_missing_source_changed_abi_engine_seal_and_ground() {
        let (_, header) = opening(2);
        for mutation in 0..6 {
            let mut changed = (*header).clone();
            match mutation {
                0 => changed.sources.clear(),
                1 => changed.engine.abi.push('!'),
                2 => changed.engine.engine.push('!'),
                3 => changed.sources[0].text.push_str("//different bytes"),
                4 => changed.seals[0].source_sha256[0] ^= 1,
                _ => {
                    let mut value = serde_json::to_value(&changed.ground).unwrap();
                    value["width"] = serde_json::json!(0);
                    changed.ground = serde_json::from_value(value).unwrap();
                }
            }
            assert!(open_frozen_header(&changed).is_err(), "mutation {mutation}");
        }
    }
    #[test]
    fn m6_byte_envelope_full_header_and_bounded_chunks_fit_24_class_iii_for_fifteen_minutes() {
        let (_, header) = opening(24);
        assert_eq!(header.seals.len(), 24);
        assert!(header.seals.iter().all(|s| s.class == CoreClass::III));
        let mut recorder = TapeRecorder::open(&header).unwrap();
        let mut chunk_bytes = 0usize;
        let mut chunks = 0usize;
        let token = |unit| crate::arena::profile_runtime::ProfileToken {
            unit,
            generation: 0,
            key: header.seals[(unit.0 - 1) as usize]
                .cards
                .selected
                .key(0)
                .unwrap(),
        };
        for at in (250..=900_000).step_by(250) {
            let jobs = (1..=24)
                .map(|n| ThinkIdentity {
                    unit: ObjId(n),
                    seq: (at / 250) as u64,
                    at,
                    lands: at + 250,
                    epochs: [0; 3],
                    profile: token(ObjId(n)),
                    class: CoreClass::III,
                    rom: false,
                })
                .collect::<Vec<_>>();
            let events = [
                TapeEvent::Pulse {
                    wall: at,
                    controls: vec![],
                    intents: vec![],
                    cards: vec![],
                    hashes: vec![u64::MAX],
                    thinks: vec![],
                    pending: jobs.clone(),
                    at,
                    hash: u64::MAX,
                    next_due: at + 250,
                },
                TapeEvent::Settle {
                    thinks: jobs
                        .into_iter()
                        .map(|job| ThinkWitness {
                            job,
                            answer: [255; 32],
                        })
                        .collect(),
                    at,
                    hash: u64::MAX,
                },
            ];
            for event in events {
                for write in recorder.push(event).unwrap() {
                    if let TapeWrite::Chunk(chunk) = write {
                        chunk_bytes += encoded_len(&*chunk).unwrap();
                        chunks += 1;
                    }
                }
            }
        }
        let (_, tail) = recorder.checkpoint().unwrap();
        for write in tail {
            if let TapeWrite::Chunk(chunk) = write {
                chunk_bytes += encoded_len(&*chunk).unwrap();
                chunks += 1;
            }
        }
        let bytes = encoded_len(&*header).unwrap() + chunk_bytes;
        assert!(bytes > 16 * 1024 * 1024);
        assert!(bytes < DEFAULT_MAX_BYTES as usize);
        assert!(chunks < MAX_CHUNKS);
        eprintln!(
            "24 III / 900s: header={} chunks={} bytes={bytes}",
            encoded_len(&*header).unwrap(),
            chunks
        );
    }

    struct RecordedRun {
        arena: Arena,
        header: Arc<TapeHeader>,
        recorder: TapeRecorder,
        writes: Vec<TapeWrite>,
        wall: i64,
        cadence: i64,
    }
    impl RecordedRun {
        fn quiet(units: u32, cadence: i64) -> Self {
            let (mut arena, header) = opening_source(units, QUIET_SOURCE);
            let recorder = TapeRecorder::open(&header).unwrap();
            arena.tape_cursor = Some(recorder.cursor());
            arena.tape_witnesses.enable();
            Self {
                arena,
                header,
                recorder,
                writes: Vec::new(),
                wall: 0,
                cadence,
            }
        }
        fn pulse(&mut self, controls: &[Control], intents: &[Intent]) {
            let (out, writes) = recorded_step_settled(
                &mut self.arena,
                &mut self.recorder,
                self.wall,
                controls,
                intents,
                Vec::new(),
            )
            .unwrap();
            assert_eq!(
                out.motor.mind_faults, 0,
                "real sealed fixture script must not fault"
            );
            assert_eq!(out.motor.mind_overflows, 0);
            self.writes.extend(writes);
            // Acknowledge only chunks the real pulse handed to its world-side
            // writer. Do not add an unrecorded flush or alter chunk boundaries.
            if let Some(through) = out.ledger_chunks.iter().map(|chunk| chunk.seq).max() {
                self.arena.ledger.mark_flushed(through);
                self.writes.extend(
                    self.recorder
                        .push(TapeEvent::LedgerAck { through })
                        .unwrap(),
                );
            }
            self.wall += self.cadence;
        }
        fn to(&mut self, at: i64) {
            while self.arena.now_ms < at {
                assert!(
                    self.arena.ended.is_none(),
                    "fixture ended before requested time {at}"
                );
                self.pulse(&[], &[]);
            }
        }
        fn restart(&mut self) {
            let (cursor, writes) = self.recorder.checkpoint().unwrap();
            self.writes.extend(writes);
            self.arena.tape_cursor = Some(cursor);
            let snapshot = Snapshot::encode(&self.arena).unwrap();
            let definition = self.header.ground.clone().restore().unwrap();
            self.arena = Snapshot::decode(&snapshot.document, definition).unwrap();
            install_header_sources(&self.header, &mut self.arena).unwrap();
            self.arena.resume_at(0);
            self.arena.tape_witnesses.enable();
            self.wall = self.cadence;
            self.writes.extend(
                self.recorder
                    .push(TapeEvent::Restart {
                        hash: state_hash(&self.arena),
                        clock: self.arena.clock.replay_state(),
                        ledger_flushed: self.arena.ledger.flushed_upto(),
                    })
                    .unwrap(),
            );
        }
        fn stop(&mut self) {
            self.writes.extend(
                self.recorder
                    .push(TapeEvent::Stop {
                        reason: StopReason::Abort,
                    })
                    .unwrap(),
            );
            self.arena.stop(StopReason::Abort);
            self.pulse(&[], &[]);
        }
        fn finish(mut self) -> (Arc<VerifiedTape>, u64, [u8; 32], usize, usize, usize) {
            let hash = state_hash(&self.arena);
            let digest = self.arena.ledger.digest();
            let end = self
                .arena
                .ended
                .expect("a real terminal game before completion");
            self.writes
                .extend(self.recorder.finish(self.arena.now_ms, hash, end).unwrap());
            let chunks = self
                .writes
                .iter()
                .filter_map(|write| match write {
                    TapeWrite::Chunk(chunk) => Some(Arc::clone(chunk)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let mark = self
                .writes
                .iter()
                .rev()
                .find_map(|write| match write {
                    TapeWrite::Mark(mark) => Some(mark.clone()),
                    _ => None,
                })
                .unwrap();
            assert!(matches!(mark.status, TapeStatus::Complete { .. }));
            let thinks = chunks
                .iter()
                .flat_map(|chunk| &chunk.events)
                .map(|event| match event {
                    TapeEvent::Pulse { thinks, .. } | TapeEvent::Settle { thinks, .. } => {
                        thinks.len()
                    }
                    _ => 0,
                })
                .sum();
            let substeps = chunks
                .iter()
                .flat_map(|chunk| &chunk.events)
                .map(|event| match event {
                    TapeEvent::Pulse { hashes, .. } => hashes.len(),
                    _ => 0,
                })
                .sum();
            let bytes = encoded_len(&*self.header).unwrap()
                + encoded_len(&mark).unwrap()
                + chunks
                    .iter()
                    .map(|chunk| encoded_len(&**chunk).unwrap())
                    .sum::<usize>();
            assert!(bytes < DEFAULT_MAX_BYTES as usize);
            assert!(chunks.len() < MAX_CHUNKS);
            eprintln!(
                "recording header_bytes={} chunks={} complete_bytes={bytes}",
                encoded_len(&*self.header).unwrap(),
                chunks.len()
            );
            (
                Arc::new(VerifiedTape::verify(self.header, &chunks, mark).unwrap()),
                hash,
                digest,
                thinks,
                substeps,
                bytes,
            )
        }
    }

    fn assert_full_replay(
        tape: Arc<VerifiedTape>,
        hash: u64,
        digest: [u8; 32],
        thinks: usize,
    ) -> usize {
        let max_slices = tape
            .events
            .iter()
            .map(|(event, _)| match event {
                TapeEvent::Pulse { hashes, .. } => hashes.len() + 8,
                _ => 1,
            })
            .sum::<usize>()
            + 1;
        let mut replay = ReplaySession::new(tape).unwrap();
        let mut seen = 0;
        for turns in 1..=max_slices {
            let cost = replay.next_simulation_ms();
            let progress = replay.advance_substep().unwrap();
            assert_eq!(progress.simulated_ms, cost);
            seen += progress.thinks;
            if progress.complete {
                let arena = replay.arena().unwrap();
                assert_eq!(state_hash(arena), hash);
                assert_eq!(arena.ledger.digest(), digest);
                assert_eq!(seen, thinks, "every real think was recomputed and compared");
                return turns;
            }
        }
        panic!("replay did not complete within its recorded native-substep/phase bound");
    }

    #[test]
    fn m6_mixed_inputs_replay_accepted_order_rules_place_release_and_reclaim() {
        use crate::arena::conditions::{Arming, Rule};
        use crate::deliberation::order::{CharacterRef, Constraints, Objective, Order};
        for cadence in [5000, 1000, 500] {
            for restart in [false, true] {
                let mut run = RecordedRun::quiet(2, cadence);
                run.to(15_000);
                let pilot = ObjId(1000);
                let unit = ObjId(1);
                let other = ObjId(2);
                let squad = crate::deliberation::SquadId::parse("blue-1").unwrap();
                let order = Order {
                    squad,
                    objective: Objective::Hold,
                    target: None,
                    horizon_ms: 600_000,
                    constraints: Constraints {
                        hold_fire: true,
                        max_range_band: None,
                    },
                    ttl_ms: 600_000,
                    issued_ms: run.arena.now_ms,
                    issuer: None,
                    by: CharacterRef::Character { id: pilot },
                    spot: None,
                };
                run.pulse(
                    &[],
                    &[Intent {
                        unit: other,
                        channel: Channel::Helm,
                        epoch: 0,
                        seq: 1,
                        kind: IntentKind::Order(Box::new(order)),
                    }],
                );
                assert!(
                    run.arena.minds[&other].order.is_some(),
                    "the accepted leader order must change the real row"
                );
                let controls = |state, epoch| {
                    Channel::ALL
                        .iter()
                        .map(|&channel| Control {
                            unit,
                            channel,
                            epoch,
                            state,
                            pilot: Some(pilot),
                        })
                        .collect::<Vec<_>>()
                };
                let input = |channel, epoch, seq, kind| Intent {
                    unit,
                    channel,
                    epoch,
                    seq,
                    kind,
                };
                run.pulse(
                    &controls(ControlState::Live, 1),
                    &[input(
                        Channel::Helm,
                        1,
                        2,
                        IntentKind::Heading { mdeg: 90_000 },
                    )],
                );
                assert_eq!(run.arena.unit(unit).unwrap().controller, Controller::Pilot);
                run.pulse(
                    &[],
                    &[input(Channel::Helm, 1, 3, IntentKind::Profile { index: 1 })],
                );
                assert_eq!(run.arena.profiles[&unit].index, 1);
                let rule = Rule::hold_rule("cannon".into(), vec![]);
                run.pulse(
                    &[],
                    &[input(
                        Channel::Fire,
                        1,
                        4,
                        IntentKind::Condition(Arming::Rules {
                            slot: Some(1),
                            rules: vec![rule.clone()],
                        }),
                    )],
                );
                assert_eq!(run.arena.conditions.slots(unit)[0], Some(rule));
                run.pulse(
                    &[],
                    &[input(Channel::Helm, 1, 5, IntentKind::Place { x: 6, y: 6 })],
                );
                assert_eq!(
                    run.arena
                        .definition
                        .terrain
                        .cell_of(&run.arena.unit(unit).unwrap().pose),
                    Some((6, 6))
                );
                if restart {
                    run.restart();
                }
                run.pulse(&controls(ControlState::Released, 2), &[]);
                assert_eq!(run.arena.unit(unit).unwrap().controller, Controller::Mind);
                run.pulse(
                    &controls(ControlState::Live, 3),
                    &[input(
                        Channel::Helm,
                        3,
                        6,
                        IntentKind::Heading { mdeg: 180_000 },
                    )],
                );
                assert_eq!(run.arena.unit(unit).unwrap().controller, Controller::Pilot);
                run.pulse(
                    &[],
                    &[Intent {
                        unit: other,
                        channel: Channel::Helm,
                        epoch: 0,
                        seq: 7,
                        kind: IntentKind::ReleaseOrder {
                            squad,
                            issuer: None,
                        },
                    }],
                );
                assert!(run.arena.minds[&other].order.is_none());
                run.stop();
                let (tape, hash, digest, thinks, _, _) = run.finish();
                assert!(thinks > 0);
                assert_full_replay(tape, hash, digest, thinks);
            }
        }
    }

    /// Manual CI evidence, not a wall-clock performance gate. The separate
    /// byte-envelope test stresses wide serialized numbers; this fixture runs
    /// real compiled Class-III thinks for the entire supported 900-second match.
    #[test]
    #[ignore = "manual replay evidence: 24 Class III minds through 900s plus full re-simulation"]
    fn m6_real_24_class_iii_900s_complete_tape_replays_every_think_and_substep() {
        let started = std::time::Instant::now();
        let mut run = RecordedRun::quiet(24, 250);
        assert!(
            run.header
                .seals
                .iter()
                .all(|seal| seal.class == CoreClass::III)
        );
        assert!(
            run.header
                .seals
                .iter()
                .all(|seal| seal.cards.selected.len() == 4)
        );
        run.to(900_000);
        assert_eq!(run.arena.now_ms, 900_000);
        assert_eq!(
            run.arena.ended.unwrap().reason,
            crate::arena::EndReason::Timeout
        );
        assert!(run.arena.units.iter().all(|unit| !unit.limits.wreck));
        for mind in run.arena.minds.values() {
            assert!(mind.dropped.is_none());
            let memory = mush_mind::memory::decode(&mind.memory).unwrap();
            assert!(
                matches!(memory.get("visits"), Some(mush_mind::memory::Value::Int(n)) if *n >= 3500)
            );
        }
        let seed = run.arena.seed;
        let (tape, hash, digest, thinks, substeps, bytes) = run.finish();
        assert!(
            thinks >= 24 * 3500,
            "all 24 real Class-III runners stayed active"
        );
        assert!(substeps >= 3600);
        let recording_seconds = started.elapsed().as_secs_f64();
        let replay_started = std::time::Instant::now();
        let slices = assert_full_replay(tape, hash, digest, thinks);
        eprintln!(
            "24 Class III / 900000ms: seed={seed} cadence_ms=250 complete_bytes={bytes} thinks={thinks} substeps={substeps} replay_slices={slices} final_hash={hash:016x} ledger_sha256={} record_s={recording_seconds:.3} replay_s={:.3}",
            crate::arena::digest_text(&digest),
            replay_started.elapsed().as_secs_f64()
        );
    }
}
