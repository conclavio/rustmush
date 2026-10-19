//! Private deterministic input recording. This is not the public picture replay.
//! Inputs are already coalesced, and each pool settlement is a separate barrier.
//! No recorded mind output is ever used as an input to a replayed arena.
use std::collections::BTreeMap;
use std::io::Write;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::Snapshot;
use super::clock::ReplayClock;
use super::intents::{Control, Intent};
use super::mind::{CollectedThink, MindSeal, ThinkResult};
use super::profile_runtime::ProfileToken;
use super::replay_ground::ReplayGround;
use super::state::{Arena, MatchEnd, StopReason};

/// M7 carries mandatory filed certificates in opening snapshots and seals.
pub const TAPE_FORMAT: u16 = 2;
/// Bump whenever native execution or input application changes without an ABI
/// change. Also captures the script body, physics and snapshot protocol below.
pub const REPLAY_ENGINE: &str = "tacsim-engine-m7-v1";
pub const DEFAULT_MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const MIN_MAX_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_MAX_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_CHUNKS: usize = 8192;
pub const CHUNK_TARGET: usize = 64 * 1024;
pub const MAX_EVENT_BYTES: usize = 512 * 1024;
pub const MAX_WITNESSES: usize = 4096;
pub const MAX_EVENTS: usize = 250_000;
const MARKER_RESERVE: u64 = 4096;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TapeError {
    #[error("This match's bench recording is unavailable: {0}.")]
    Unavailable(String),
    #[error("The bench recording is incomplete: {0}.")]
    Incomplete(String),
    #[error("The bench recording does not match this engine: {0}.")]
    Mismatch(String),
}
fn invalid(word: impl Into<String>) -> TapeError {
    TapeError::Incomplete(word.into())
}

/// Compact cursor binds a snapshot to a complete durable chunk prefix. It is
/// carried by snapshots, but is observational and never folded into state_hash.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TapeCursor {
    pub chunks: u32,
    pub events: u64,
    #[serde(with = "super::snapshot::digest_serde")]
    pub digest: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineStamp {
    pub engine: String,
    pub abi: String,
    pub snapshot: u32,
    pub physics: String,
    pub standard: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenSource {
    #[serde(with = "super::snapshot::digest_serde")]
    pub seal: [u8; 32],
    #[serde(with = "super::snapshot::digest_serde")]
    pub sha256: [u8; 32],
    pub text: String,
}

/// Opening snapshot freezes all arena/match/sensor/class configuration and
/// per-revision card catalog. Ground additionally retains the historical digest's
/// omitted row prices. Full seals retain both requested and effective identities.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TapeHeader {
    pub format: u16,
    pub match_id: u64,
    pub limit: u64,
    pub engine: EngineStamp,
    pub opening: serde_json::Value,
    pub clock: ReplayClock,
    pub(crate) ground: ReplayGround,
    pub seals: Vec<MindSeal>,
    pub sources: Vec<FrozenSource>,
}
impl TapeHeader {
    /// SHA-256 of the exact frozen-ground JSON used by proving reports. This
    /// exposes identity only, without publishing terrain or retained source.
    pub fn ground_sha256(&self) -> Result<[u8; 32], TapeError> {
        hash_value(b"", &self.ground)
    }

    pub fn capture(
        arena: &Arena,
        match_id: u64,
        seals: Vec<MindSeal>,
        sources: &BTreeMap<[u8; 32], String>,
        engine: EngineStamp,
        limit: u64,
    ) -> Result<Self, TapeError> {
        validate_limit(limit)?;
        if arena.tape_cursor.is_some() {
            return Err(invalid("opening already has a tape cursor"));
        }
        let opening = Snapshot::encode(arena)
            .map_err(|e| invalid(e.to_string()))?
            .document;
        let sources = sources
            .iter()
            .map(|(seal, text)| FrozenSource {
                seal: *seal,
                sha256: Sha256::digest(text.as_bytes()).into(),
                text: text.clone(),
            })
            .collect();
        let value = Self {
            format: TAPE_FORMAT,
            match_id,
            limit,
            engine,
            opening,
            clock: arena.clock.replay_state(),
            ground: ReplayGround::capture(&arena.definition).map_err(invalid)?,
            seals,
            sources,
        };
        value.validate_shape()?;
        if encoded_len(&value)? as u64 + MARKER_RESERVE > limit {
            return Err(TapeError::Unavailable(
                "opening exceeds the configured recording bound".into(),
            ));
        }
        Ok(value)
    }
    pub fn validate_shape(&self) -> Result<(), TapeError> {
        validate_limit(self.limit)?;
        if self.format != TAPE_FORMAT {
            return Err(TapeError::Mismatch(
                "format, ABI, physics or executable identity changed".into(),
            ));
        }
        if self.match_id == 0
            || self.seals.is_empty()
            || self.seals.len() > 512
            || self.sources.len() > 512
        {
            return Err(invalid("opening identity or roster bound"));
        }
        let mut units = std::collections::BTreeSet::new();
        let mut keys = std::collections::BTreeSet::new();
        for seal in &self.seals {
            if !units.insert(seal.unit) || seal.abi.as_str() != self.engine.abi {
                return Err(invalid("duplicate seal or changed ABI"));
            }
        }
        for source in &self.sources {
            if source.text.len() > 64 * 1024
                || !keys.insert(source.seal)
                || <[u8; 32]>::from(Sha256::digest(source.text.as_bytes())) != source.sha256
            {
                return Err(invalid("source bound, duplicate, or hash"));
            }
            let Some(seal) = self.seals.iter().find(|s| s.digest() == source.seal) else {
                return Err(invalid("source has no original seal"));
            };
            if seal.source_sha256 != source.sha256
                || seal.engine != super::mind::SealEngine::Scripted
            {
                return Err(invalid("source differs from effective executable seal"));
            }
        }
        for seal in &self.seals {
            if seal.engine == super::mind::SealEngine::Scripted && !keys.contains(&seal.digest()) {
                return Err(invalid("missing sealed source"));
            }
        }
        Ok(())
    }
    pub fn cursor(&self) -> Result<TapeCursor, TapeError> {
        Ok(TapeCursor {
            digest: hash_value(b"tacsim-tape-header-v1", self)?,
            ..TapeCursor::default()
        })
    }
}

/// The frozen job identity, including the original station epochs and profile
/// generation. A refused/stale answer is still witnessed before its rejection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThinkIdentity {
    pub unit: ObjId,
    pub seq: u64,
    pub at: i64,
    pub lands: i64,
    pub epochs: [u64; 3],
    pub profile: ProfileToken,
    pub class: super::core::CoreClass,
    pub rom: bool,
}
impl ThinkIdentity {
    pub fn capture(think: &CollectedThink, epochs: [u64; 3]) -> Self {
        Self {
            unit: think.unit,
            seq: think.think_seq,
            at: think.t_ms,
            lands: think.lands_ms,
            epochs,
            profile: think.profile_token,
            class: think.profile.class,
            rom: think.dropped,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThinkWitness {
    pub job: ThinkIdentity,
    /// Streaming canonical digest of every intent, memory byte, fault cause and
    /// rule sample. Debug print/diagnostic collection is deliberately excluded.
    #[serde(with = "super::snapshot::digest_serde")]
    pub answer: [u8; 32],
}
#[derive(Clone, Debug, Default)]
pub struct WitnessBuffer {
    enabled: bool,
    overflow: bool,
    rows: Vec<ThinkWitness>,
}
impl WitnessBuffer {
    pub fn enable(&mut self) {
        self.enabled = true;
        self.overflow = false;
        self.rows.clear();
    }
    pub fn disable(&mut self) {
        self.enabled = false;
        self.rows.clear();
    }
    pub fn observe(&mut self, think: &CollectedThink, result: &ThinkResult, epochs: [u64; 3]) {
        if !self.enabled {
            return;
        }
        if self.rows.len() >= MAX_WITNESSES {
            self.overflow = true;
            return;
        }
        let output = (
            &result.intents,
            &result.memory,
            result.fault.as_ref().map(|f| f.cause.as_str()),
            &result.rule_sample,
        );
        match hash_value(b"tacsim-think-answer-v1", &output) {
            Ok(answer) => self.rows.push(ThinkWitness {
                job: ThinkIdentity::capture(think, epochs),
                answer,
            }),
            Err(_) => self.overflow = true,
        }
    }
    pub fn take(&mut self) -> Result<Vec<ThinkWitness>, TapeError> {
        if self.overflow {
            self.disable();
            return Err(TapeError::Unavailable(
                "think witness buffer overflow".into(),
            ));
        }
        Ok(std::mem::take(&mut self.rows))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TapeCard {
    pub unit: ObjId,
    pub change: super::card::CardChange,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TapeEvent {
    Pulse {
        wall: i64,
        controls: Vec<Control>,
        intents: Vec<Intent>,
        cards: Vec<TapeCard>,
        hashes: Vec<u64>,
        thinks: Vec<ThinkWitness>,
        pending: Vec<ThinkIdentity>,
        at: i64,
        hash: u64,
        next_due: i64,
    },
    Settle {
        thinks: Vec<ThinkWitness>,
        at: i64,
        hash: u64,
    },
    Stop {
        reason: StopReason,
    },
    LedgerAck {
        through: u64,
    },
    TempoFloor {
        period: Option<u32>,
    },
    Watch {
        on: bool,
    },
    /// Normal live restart resets wall/floor/pin and snapshot-only scratch.
    /// Re-encode/decode the state already reproduced at the exact checkpoint;
    /// this applies the normal snapshot scratch reset, then restores the actual
    /// new process clock. No unverified snapshot can skip recorded work.
    Restart {
        hash: u64,
        clock: ReplayClock,
        ledger_flushed: u64,
    },
}
impl TapeEvent {
    pub fn validate(&self) -> Result<(), TapeError> {
        if encoded_len(self)? > MAX_EVENT_BYTES {
            return Err(invalid("input event exceeds bound"));
        }
        match self {
            Self::Pulse {
                controls,
                intents,
                cards,
                hashes,
                thinks,
                pending,
                ..
            } if controls.len() > 8192
                || intents.len() > 8192
                || cards.len() > 8192
                || hashes.len() > 4096
                || thinks.len() > MAX_WITNESSES
                || pending.len() > 512 =>
            {
                Err(invalid("pulse shape bound"))
            }
            Self::Settle { thinks, .. } if thinks.len() > MAX_WITNESSES => {
                Err(invalid("settlement bound"))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TapeChunk {
    pub seq: u32,
    pub first: u64,
    #[serde(with = "super::snapshot::digest_serde")]
    pub previous: [u8; 32],
    #[serde(with = "super::snapshot::digest_serde")]
    pub digest: [u8; 32],
    pub events: Vec<TapeEvent>,
}
impl TapeChunk {
    fn new(cursor: TapeCursor, events: Vec<TapeEvent>) -> Result<Self, TapeError> {
        let mut chunk = Self {
            seq: cursor.chunks,
            first: cursor.events,
            previous: cursor.digest,
            digest: [0; 32],
            events,
        };
        chunk.digest = hash_value(
            b"tacsim-tape-chunk-v1",
            &(chunk.seq, chunk.first, chunk.previous, &chunk.events),
        )?;
        Ok(chunk)
    }
    pub fn after(&self, cursor: TapeCursor) -> Result<TapeCursor, TapeError> {
        if self.seq != cursor.chunks
            || self.first != cursor.events
            || self.previous != cursor.digest
            || self.events.is_empty()
            || self.events.len() > 1024
            || self.seq as usize >= MAX_CHUNKS
        {
            return Err(invalid("missing, duplicated or unordered input chunk"));
        }
        if self.digest
            != hash_value(
                b"tacsim-tape-chunk-v1",
                &(self.seq, self.first, self.previous, &self.events),
            )?
        {
            return Err(invalid("input chunk digest differs"));
        }
        for event in &self.events {
            event.validate()?;
        }
        let events = cursor
            .events
            .checked_add(self.events.len() as u64)
            .ok_or_else(|| invalid("event count"))?;
        if events > MAX_EVENTS as u64 {
            return Err(invalid("event count bound"));
        }
        Ok(TapeCursor {
            chunks: cursor.chunks + 1,
            events,
            digest: self.digest,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum TapeStatus {
    Recording,
    Complete { at: i64, hash: u64, end: MatchEnd },
    Incomplete { reason: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TapeMark {
    pub format: u16,
    pub match_id: u64,
    pub cursor: TapeCursor,
    pub bytes: u64,
    pub status: TapeStatus,
}
#[derive(Clone, Debug)]
pub enum TapeWrite {
    Open(std::sync::Arc<TapeHeader>),
    Chunk(std::sync::Arc<TapeChunk>),
    Mark(TapeMark),
}

/// Bounded producer. Queue backpressure is handled by the task's separate tape
/// byte budget. A refusal disables only recording, never arena execution.
#[derive(Debug)]
pub struct TapeRecorder {
    id: u64,
    limit: u64,
    bytes: u64,
    cursor: TapeCursor,
    events: Vec<TapeEvent>,
    event_bytes: usize,
    stopped: bool,
}
impl TapeRecorder {
    pub fn open(header: &TapeHeader) -> Result<Self, TapeError> {
        header.validate_shape()?;
        let bytes = encoded_len(header)? as u64;
        if bytes + MARKER_RESERVE > header.limit {
            return Err(TapeError::Unavailable(
                "opening exceeds the configured recording bound".into(),
            ));
        }
        Ok(Self {
            id: header.match_id,
            limit: header.limit,
            bytes,
            cursor: header.cursor()?,
            events: Vec::new(),
            event_bytes: 0,
            stopped: false,
        })
    }
    pub fn resume(id: u64, limit: u64, bytes: u64, cursor: TapeCursor) -> Result<Self, TapeError> {
        validate_limit(limit)?;
        if bytes.saturating_add(MARKER_RESERVE) > limit
            || cursor.chunks as usize > MAX_CHUNKS
            || cursor.events > MAX_EVENTS as u64
        {
            return Err(invalid("resume recording budget"));
        }
        Ok(Self {
            id,
            limit,
            bytes,
            cursor,
            events: Vec::new(),
            event_bytes: 0,
            stopped: false,
        })
    }
    pub fn push(&mut self, event: TapeEvent) -> Result<Vec<TapeWrite>, TapeError> {
        if self.stopped {
            return Ok(Vec::new());
        }
        event.validate()?;
        let size = encoded_len(&event)?;
        if self
            .bytes
            .saturating_add(self.event_bytes as u64)
            .saturating_add(size as u64)
            .saturating_add(MARKER_RESERVE)
            > self.limit
        {
            return Err(TapeError::Unavailable(
                "configured input recording byte bound reached; live play continues".into(),
            ));
        }
        let mut writes = Vec::new();
        if !self.events.is_empty()
            && (self.event_bytes + size > CHUNK_TARGET || self.events.len() >= 1024)
        {
            writes.extend(self.flush()?);
        }
        self.event_bytes += size + 1;
        self.events.push(event);
        Ok(writes)
    }
    pub fn flush(&mut self) -> Result<Vec<TapeWrite>, TapeError> {
        if self.stopped || self.events.is_empty() {
            return Ok(Vec::new());
        }
        let chunk = TapeChunk::new(self.cursor, std::mem::take(&mut self.events))?;
        let next = chunk.after(self.cursor)?;
        let bytes = self
            .bytes
            .checked_add(encoded_len(&chunk)? as u64)
            .ok_or_else(|| invalid("recording byte count"))?;
        if bytes + MARKER_RESERVE > self.limit {
            return Err(TapeError::Unavailable(
                "configured input recording byte bound reached; live play continues".into(),
            ));
        }
        self.bytes = bytes;
        self.cursor = next;
        self.event_bytes = 0;
        Ok(vec![TapeWrite::Chunk(std::sync::Arc::new(chunk))])
    }
    pub fn checkpoint(&mut self) -> Result<(TapeCursor, Vec<TapeWrite>), TapeError> {
        if self.stopped {
            return Err(invalid("recording is already closed"));
        }
        let mut writes = self.flush()?;
        writes.push(TapeWrite::Mark(self.mark(TapeStatus::Recording)));
        Ok((self.cursor, writes))
    }
    pub fn finish(
        &mut self,
        at: i64,
        hash: u64,
        end: MatchEnd,
    ) -> Result<Vec<TapeWrite>, TapeError> {
        if self.stopped {
            return Err(invalid("recording is already closed"));
        }
        let mut writes = self.flush()?;
        self.stopped = true;
        writes.push(TapeWrite::Mark(self.mark(TapeStatus::Complete {
            at,
            hash,
            end,
        })));
        Ok(writes)
    }
    pub fn fail(&mut self, reason: impl Into<String>) -> TapeWrite {
        self.stopped = true;
        self.events.clear();
        self.event_bytes = 0;
        let reason = reason.into().chars().take(240).collect();
        TapeWrite::Mark(self.mark(TapeStatus::Incomplete { reason }))
    }
    pub fn cursor(&self) -> TapeCursor {
        self.cursor
    }
    fn mark(&self, status: TapeStatus) -> TapeMark {
        TapeMark {
            format: TAPE_FORMAT,
            match_id: self.id,
            cursor: self.cursor,
            bytes: self.bytes,
            status,
        }
    }
}
pub fn validate_limit(limit: u64) -> Result<(), TapeError> {
    if !(MIN_MAX_BYTES..=MAX_MAX_BYTES).contains(&limit) {
        Err(TapeError::Unavailable(
            "tape_max_bytes must be 4 to 128 MiB".into(),
        ))
    } else {
        Ok(())
    }
}
struct HashWriter(Sha256);
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn hash_value(value_domain: &[u8], value: &impl Serialize) -> Result<[u8; 32], TapeError> {
    let mut writer = HashWriter(Sha256::new());
    writer.0.update(value_domain);
    serde_json::to_writer(&mut writer, value).map_err(|e| invalid(e.to_string()))?;
    Ok(writer.0.finalize().into())
}
struct CountWriter(usize);
impl Write for CountWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("record too large"))?;
        if self.0 > MAX_MAX_BYTES as usize {
            return Err(std::io::Error::other("record too large"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn encoded_len(value: &impl Serialize) -> Result<usize, TapeError> {
    let mut out = CountWriter(0);
    serde_json::to_writer(&mut out, value).map_err(|e| invalid(e.to_string()))?;
    Ok(out.0)
}

#[cfg(test)]
mod tests {
    use super::super::core::CoreClass;
    use super::super::runtime_card::ProfileKey;
    use super::*;
    fn job(unit: u32, at: i64) -> ThinkIdentity {
        ThinkIdentity {
            unit: ObjId(unit),
            seq: (at / 250) as u64,
            at,
            lands: at + 250,
            epochs: [0; 3],
            profile: ProfileToken {
                unit: ObjId(unit),
                generation: 0,
                key: ProfileKey {
                    definition: mush_core::DefinitionDigest::from_bytes([0; 32]),
                    index: 0,
                },
            },
            class: CoreClass::III,
            rom: false,
        }
    }
    fn pulse(at: i64) -> TapeEvent {
        TapeEvent::Pulse {
            wall: at,
            controls: vec![],
            intents: vec![],
            cards: vec![],
            hashes: vec![u64::MAX],
            thinks: vec![],
            pending: (1..=24).map(|n| job(n, at)).collect(),
            at,
            hash: u64::MAX,
            next_due: at + 250,
        }
    }
    #[test]
    fn m6_tape_chunks_refuse_gap_reorder_mutation_and_duplicates() {
        let start = TapeCursor {
            digest: [1; 32],
            ..TapeCursor::default()
        };
        let first = TapeChunk::new(start, vec![TapeEvent::LedgerAck { through: 2 }]).unwrap();
        let second = TapeChunk::new(
            first.after(start).unwrap(),
            vec![TapeEvent::Watch { on: true }],
        )
        .unwrap();
        assert!(second.after(start).is_err());
        assert!(first.after(first.after(start).unwrap()).is_err());
        let mut changed = first.clone();
        changed.events[0] = TapeEvent::LedgerAck { through: 3 };
        assert!(changed.after(start).is_err());
        let second_cursor = second.after(first.after(start).unwrap()).unwrap();
        assert_eq!((second_cursor.chunks, second_cursor.events), (2, 2));
    }
    #[test]
    fn m6_tape_full_launch_duration_class_iii_capacity_is_measured() {
        // Admission permits24*18k=432k ops/s below500k-24k=476k.
        // This records the maximum launch roster at the fastest class for the
        // entire shipped15min duration, including pending and settled identity.
        let mut bytes = 0usize;
        for at in (250..=900_000).step_by(250) {
            bytes += encoded_len(&pulse(at)).unwrap();
            bytes += encoded_len(&TapeEvent::Settle {
                thinks: (1..=24)
                    .map(|n| ThinkWitness {
                        job: job(n, at),
                        answer: [255; 32],
                    })
                    .collect(),
                at,
                hash: u64::MAX,
            })
            .unwrap();
        }
        assert!(
            bytes > 16 * 1024 * 1024,
            "a16MiB default would silently exclude an admitted roster"
        );
        // Leave8MiB for opening snapshot/ground/seals and3MiB for chunk framing
        // and representative command traffic; arbitrary floods use the declared
        // explicit incomplete path, never a supposedly complete truncated tape.
        assert!(
            bytes + 11 * 1024 * 1024 < 64 * 1024 * 1024,
            "measured event bytes={bytes}"
        );
    }
    #[test]
    fn m6_recording_overflow_refuses_before_appending_and_failure_is_terminal() {
        let mut recorder = TapeRecorder::resume(
            7,
            MIN_MAX_BYTES,
            MIN_MAX_BYTES - MARKER_RESERVE - 10,
            TapeCursor::default(),
        )
        .unwrap();
        assert!(recorder.push(pulse(250)).is_err());
        assert_eq!(recorder.cursor(), TapeCursor::default());
        let TapeWrite::Mark(mark) = recorder.fail("bound reached") else {
            panic!()
        };
        assert!(matches!(mark.status, TapeStatus::Incomplete { .. }));
        assert!(
            recorder
                .push(TapeEvent::Watch { on: false })
                .unwrap()
                .is_empty()
        );
        assert!(recorder.flush().unwrap().is_empty());
        assert!(recorder.checkpoint().is_err());
    }
    #[test]
    fn m6_witness_hash_includes_memory_intents_fault_and_sample_not_trace() {
        // The same helper hashes exactly the typed observable answer tuple,
        // and is streaming even for full allowed classIII memory.
        let intents: Vec<super::super::intents::IntentKind> = vec![];
        let sample: Option<super::super::profile_rules::RuleSample> = None;
        let a = hash_value(
            b"tacsim-think-answer-v1",
            &(&intents, vec![1, 2], Option::<&str>::None, &sample),
        )
        .unwrap();
        let b = hash_value(
            b"tacsim-think-answer-v1",
            &(&intents, vec![1, 3], Option::<&str>::None, &sample),
        )
        .unwrap();
        let c = hash_value(
            b"tacsim-think-answer-v1",
            &(&intents, vec![1, 2], Some("memory"), &sample),
        )
        .unwrap();
        assert_ne!(a, b);
        assert_ne!(a, c);
    }
}
