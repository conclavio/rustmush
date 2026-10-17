//! Recording hooks at the actual application boundaries, never the raw inbox.
use super::*;
use crate::arena::input_tape::*;

pub(super) type Start = (Option<TapeRecorder>, Vec<TapeWrite>);
const MAX_QUEUED_BYTES: usize = 2 * 1024 * 1024;

pub(super) fn fresh(
    arena: &mut Arena,
    id: MatchId,
    seals: Vec<crate::arena::mind::MindSeal>,
    sources: &BTreeMap<[u8; 32], String>,
    limit: u64,
) -> Start {
    let result = TapeHeader::capture(arena, id.0, seals, sources, EngineStamp::current(), limit)
        .and_then(|header| TapeRecorder::open(&header).map(|recorder| (recorder, header)));
    match result {
        Ok((recorder, header)) => {
            arena.tape_witnesses.enable();
            arena.tape_cursor = Some(recorder.cursor());
            (Some(recorder), vec![TapeWrite::Open(Arc::new(header))])
        }
        Err(error) => (
            None,
            vec![TapeWrite::Mark(TapeMark {
                format: TAPE_FORMAT,
                match_id: id.0,
                cursor: TapeCursor::default(),
                bytes: 0,
                status: TapeStatus::Incomplete {
                    reason: error.to_string(),
                },
            })],
        ),
    }
}
pub(super) fn resumed(
    arena: &mut Arena,
    id: MatchId,
    state: Option<crate::match_::mind_tape::ResumeTape>,
) -> Start {
    let Some(state) = state else {
        arena.tape_cursor = None;
        return (None, Vec::new());
    };
    match TapeRecorder::resume(id.0, state.limit, state.bytes, state.cursor) {
        Ok(recorder) => {
            arena.tape_witnesses.enable();
            (Some(recorder), Vec::new())
        }
        Err(error) => (
            None,
            vec![TapeWrite::Mark(TapeMark {
                format: TAPE_FORMAT,
                match_id: id.0,
                cursor: state.cursor,
                bytes: state.bytes,
                status: TapeStatus::Incomplete {
                    reason: error.to_string(),
                },
            })],
        ),
    }
}
impl Task {
    pub(super) fn record(&mut self, event: TapeEvent) {
        let result = self.tape.as_mut().map(|recorder| recorder.push(event));
        match result {
            Some(Ok(writes)) => self.queue_tape(writes),
            Some(Err(error)) => self.fail_tape(error.to_string()),
            None => {}
        }
    }
    pub(super) fn fail_tape(&mut self, why: String) {
        tracing::warn!(match_id=%self.match_id,reason=%why,"bench recording incomplete; live play continues");
        self.arena.tape_witnesses.disable();
        self.arena.tape_cursor = None;
        if self.tape.is_none() {
            return;
        }
        self.backlog
            .retain(|entry| !matches!(entry, Outbound::Tape(_)));
        if let Some(mut recorder) = self.tape.take() {
            self.backlog.push_back(Outbound::Tape(recorder.fail(why)));
        }
    }
    fn queue_tape(&mut self, writes: Vec<TapeWrite>) {
        for write in writes {
            // One bounded opening can exceed the rolling queue budget. Arc
            // ownership keeps its snapshot/source payload single-copy here.
            let added = match &write {
                TapeWrite::Open(_) => 0,
                TapeWrite::Chunk(chunk) => encoded_len(&**chunk).unwrap_or(usize::MAX),
                TapeWrite::Mark(mark) => encoded_len(mark).unwrap_or(usize::MAX),
            };
            let queued = self
                .backlog
                .iter()
                .filter_map(|entry| match entry {
                    Outbound::Tape(TapeWrite::Chunk(chunk)) => {
                        Some(encoded_len(&**chunk).unwrap_or(usize::MAX))
                    }
                    Outbound::Tape(TapeWrite::Mark(mark)) => {
                        Some(encoded_len(mark).unwrap_or(usize::MAX))
                    }
                    _ => None,
                })
                .fold(0usize, usize::saturating_add);
            if queued.saturating_add(added) > MAX_QUEUED_BYTES {
                self.fail_tape(
                    "world persistence is not draining the bounded recording queue".into(),
                );
                return;
            }
            self.backlog.push_back(Outbound::Tape(write));
        }
    }
    pub(super) fn record_pulse(
        &mut self,
        wall: i64,
        controls: &[Control],
        intents: &[Intent],
        out: &StepOutput,
    ) {
        let cards = std::mem::take(&mut self.tape_cards);
        if self.tape.is_none() {
            return;
        }
        let thinks = match self.arena.tape_witnesses.take() {
            Ok(rows) => rows,
            Err(error) => {
                self.fail_tape(error.to_string());
                return;
            }
        };
        let pending = out
            .thinks
            .iter()
            .map(|think| ThinkIdentity::capture(think, think.epochs))
            .collect();
        self.record(TapeEvent::Pulse {
            wall,
            controls: controls.to_vec(),
            intents: intents.to_vec(),
            cards,
            hashes: self.arena.substep_hashes.clone(),
            thinks,
            pending,
            at: self.arena.now_ms,
            hash: crate::arena::state_hash(&self.arena),
            next_due: out.next_due_ms,
        });
    }
    pub(super) fn record_settle(&mut self) {
        if self.tape.is_none() {
            return;
        }
        match self.arena.tape_witnesses.take() {
            Ok(thinks) => self.record(TapeEvent::Settle {
                thinks,
                at: self.arena.now_ms,
                hash: crate::arena::state_hash(&self.arena),
            }),
            Err(error) => self.fail_tape(error.to_string()),
        }
    }
    pub(super) fn checkpoint_tape(&mut self) {
        match self.tape.as_mut().map(TapeRecorder::checkpoint) {
            Some(Ok((cursor, writes))) => {
                self.queue_tape(writes);
                if self.tape.is_some() {
                    self.arena.tape_cursor = Some(cursor);
                }
            }
            Some(Err(error)) => self.fail_tape(error.to_string()),
            None => {}
        }
    }
    pub(super) fn finish_tape(&mut self, end: MatchEnd) {
        if self.arena.ended != Some(end) {
            self.fail_tape("host panic/watchdog overrode the simulated ending".into());
            return;
        }
        let result = self.tape.as_mut().map(|r| {
            r.finish(
                self.arena.now_ms,
                crate::arena::state_hash(&self.arena),
                end,
            )
        });
        match result {
            Some(Ok(writes)) => self.queue_tape(writes),
            Some(Err(error)) => self.fail_tape(error.to_string()),
            None => {}
        }
    }
}
