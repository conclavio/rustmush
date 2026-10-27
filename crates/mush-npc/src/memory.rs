//! NPC memory (design 03 §6.5), three layers cheapest-first:
//!
//! 1. [`MemoryBuffer`] — an in-RAM rolling buffer of recent room events, byte-
//!    bounded and lost on restart (acceptable).
//! 2. A persisted conversation summary (attribute
//!    [`SUMMARY_ATTR`](crate::profile::SUMMARY_ATTR)) — when the buffer
//!    overflows, an `NpcSummarize` turn folds the oldest half into it.
//! 3. [`MemoryRecord`]s — durable, subject-keyed facts persisted as
//!    `mem:{npc}:{subject}` module documents, written by the `remember` tool.
//!
//! Recall v1 is deliberately dumb ([`LongTermStore::recall`]): fetch records whose subject
//! matches a present player's name, newest-first. Embedding recall is a
//! documented M10 seam behind the [`MemoryRecall`] trait — **not built here**.

use std::collections::{HashMap, HashSet};

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// How many recent events the rolling buffer keeps before it overflows and a
/// summarize turn is triggered (design 03 §6.5: ~50).
pub const BUFFER_CAP: usize = 50;

/// Maximum characters retained from one script-formatted recent event. The
/// sanitizer also strips controls and collapses it to one line.
pub const MAX_MEMORY_EVENT_CHARS: usize = 1024;

/// Maximum UTF-8 bytes retained across one NPC's recent event buffer.
pub const MAX_MEMORY_BUFFER_BYTES: usize = 256 * 1024;

/// Maximum characters in the persisted rolling summary.
pub const MAX_MEMORY_SUMMARY_CHARS: usize = 4_000;

/// Maximum distinct durable memory subjects retained for one NPC.
pub const MAX_MEMORY_SUBJECTS_PER_NPC: usize = 256;

/// Maximum notes retained in one subject record.
pub const MAX_MEMORY_NOTES_PER_SUBJECT: usize = 20;

/// Maximum canonical subject-key bytes in one durable memory record.
pub const MAX_MEMORY_SUBJECT_BYTES: usize = 128;

/// Maximum UTF-8 bytes in one durable memory note.
pub const MAX_MEMORY_NOTE_BYTES: usize = 4 * 1024;

/// Maximum encoded JSON bytes in one complete durable memory record.
pub const MAX_MEMORY_RECORD_BYTES: usize = 128 * 1024;

/// Maximum memory records returned by one built-in recall request.
pub const MAX_MEMORY_RECALL_RECORDS: usize = 32;

/// The conversational role of a recent event, independent of its rendered text.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// Background room activity, including legacy unclassified events.
    #[default]
    Ambient,
    /// Speech or an emote observed in the room.
    Speech,
    /// Speech addressed to this NPC.
    DirectedSpeech,
    /// This NPC's own public speech or emote.
    SelfSpeech,
    /// An action outcome or a privacy-safe acknowledgement.
    Outcome,
}

impl MemoryKind {
    fn retention_priority(self) -> u8 {
        match self {
            Self::Ambient => 0,
            Self::Speech => 1,
            Self::DirectedSpeech | Self::SelfSpeech | Self::Outcome => 2,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Ambient => "room event",
            Self::Speech => "speech",
            Self::DirectedSpeech => "to you",
            Self::SelfSpeech => "your speech",
            Self::Outcome => "outcome",
        }
    }
}

/// Identity and conversational context retained alongside a formatted event.
/// IDs guide selection only; transcript rendering uses the names in the text.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemoryContext {
    /// Who spoke or acted, when known.
    pub speaker: Option<ObjId>,
    /// Where the event was observed.
    pub room: Option<ObjId>,
    /// The intended conversational partner, when known.
    pub addressee: Option<ObjId>,
    /// How the event participates in the conversation.
    pub kind: MemoryKind,
}

/// One line in the rolling buffer — a script-formatted, observed event.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemoryEvent {
    /// IC/wall timestamp (unix seconds).
    pub ts: i64,
    /// The formatted line, e.g. `Alice says, "…"` or `Bob arrives.`.
    pub text: String,
    /// Monotonic identity within this buffer. Zero marks a legacy event until
    /// the buffer normalizes it before issuing a fold candidate.
    #[serde(default)]
    pub sequence: u64,
    /// Who spoke or acted, when known.
    #[serde(default)]
    pub speaker: Option<ObjId>,
    /// Where the event was observed.
    #[serde(default)]
    pub room: Option<ObjId>,
    /// The intended conversational partner, when known.
    #[serde(default)]
    pub addressee: Option<ObjId>,
    /// How the event participates in the conversation.
    #[serde(default)]
    pub kind: MemoryKind,
}

impl MemoryEvent {
    /// Render the observation time and kind without exposing internal object IDs.
    #[must_use]
    pub fn render(&self) -> String {
        format!("[{}; {}] {}", self.ts, self.kind.label(), self.text)
    }

    fn selection_priority(&self, focus: Option<ObjId>) -> u8 {
        if self.kind != MemoryKind::Ambient
            && focus.is_some_and(|who| self.speaker == Some(who) || self.addressee == Some(who))
        {
            3
        } else {
            self.kind.retention_priority()
        }
    }

    fn render_bounded(&self, max_bytes: usize) -> Option<String> {
        let prefix = format!("[{}; {}] ", self.ts, self.kind.label());
        let available = max_bytes.checked_sub(prefix.len())?;
        let first = self.text.chars().next()?;
        if available < first.len_utf8() {
            return None;
        }
        let mut rendered = prefix;
        if self.text.len() <= available {
            rendered.push_str(&self.text);
        } else {
            // Keep at least one complete character. Mark truncation only when
            // its marker also fits; even tiny budgets remain a strict byte cap.
            let ellipsis = "…";
            let mark = available >= first.len_utf8() + ellipsis.len();
            let limit = available - if mark { ellipsis.len() } else { 0 };
            let mut boundary = limit.min(self.text.len());
            while !self.text.is_char_boundary(boundary) {
                boundary -= 1;
            }
            rendered.push_str(&self.text[..boundary]);
            if mark {
                rendered.push_str(ellipsis);
            }
        }
        Some(rendered)
    }
}

/// The per-NPC rolling event buffer (design 03 §6.5 layer 1). In-RAM only;
/// lost on restart.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryBuffer {
    /// The persisted rolling summary (layer 2), cached here for the prompt.
    pub summary: String,
    /// Recent events, oldest first.
    pub events: Vec<MemoryEvent>,
    /// Next unused event identity; defaults to zero in legacy serialized buffers
    /// and is advanced past all existing identities before allocating one.
    #[serde(default)]
    pub next_sequence: u64,
}

/// Hard ceiling on retained events, regardless of summarize cadence. A slow or
/// repeatedly failing summary can leave incoming events unconsolidated; bound RAM by
/// dropping lower-priority activity first, oldest within each priority. Set well
/// above [`BUFFER_CAP`] so normal folding is unaffected.
pub const HARD_CAP: usize = BUFFER_CAP * 2;

impl MemoryBuffer {
    /// Append an unclassified event, preserving the legacy caller API.
    pub fn push(&mut self, ts: i64, text: impl Into<String>) {
        self.push_event(ts, text, MemoryContext::default());
    }

    /// Append a structured event, enforcing the count and aggregate byte bounds.
    /// Background activity is evicted before directed speech, replies or outcomes.
    pub fn push_event(&mut self, ts: i64, text: impl Into<String>, context: MemoryContext) {
        let text = crate::action::sanitize_line(&text.into(), MAX_MEMORY_EVENT_CHARS);
        if text.is_empty() || !self.normalize_sequences() {
            return;
        }
        let Some(next) = self.next_sequence.checked_add(1) else {
            return;
        };
        self.events.push(MemoryEvent {
            ts,
            text,
            sequence: self.next_sequence,
            speaker: context.speaker,
            room: context.room,
            addressee: context.addressee,
            kind: context.kind,
        });
        self.next_sequence = next;
        let mut bytes: usize = self.events.iter().map(|event| event.text.len()).sum();
        while self.events.len() > HARD_CAP || bytes > MAX_MEMORY_BUFFER_BYTES {
            let Some(victim) = self
                .events
                .iter()
                .enumerate()
                .min_by_key(|(index, event)| (event.kind.retention_priority(), *index))
                .map(|(index, _)| index)
            else {
                break;
            };
            bytes -= self.events.remove(victim).text.len();
        }
    }

    /// Give legacy and duplicate IDs fresh identities without reusing an issued
    /// sequence. Exhaustion refuses new work rather than risking a stale fold
    /// acknowledgement deleting a newer event.
    fn normalize_sequences(&mut self) -> bool {
        let highest = self
            .events
            .iter()
            .map(|event| event.sequence)
            .max()
            .unwrap_or(0);
        let Some(after_highest) = highest.checked_add(1) else {
            return false;
        };
        self.next_sequence = self.next_sequence.max(after_highest);
        let mut seen = HashSet::with_capacity(self.events.len());
        for event in &mut self.events {
            if event.sequence != 0 && seen.insert(event.sequence) {
                continue;
            }
            let Some(next) = self.next_sequence.checked_add(1) else {
                return false;
            };
            event.sequence = self.next_sequence;
            self.next_sequence = next;
            seen.insert(event.sequence);
        }
        true
    }

    /// Whether the buffer has grown past [`BUFFER_CAP`] and should be folded.
    #[must_use]
    pub fn is_overflowing(&self) -> bool {
        self.events.len() > BUFFER_CAP
    }

    /// Split off the oldest half of the buffer for summarization, leaving the
    /// newer half in place. Returns the drained (old) events.
    pub fn drain_old_half(&mut self) -> Vec<MemoryEvent> {
        let half = self.events.len() / 2;
        self.events.drain(..half).collect()
    }

    /// Snapshot the oldest half without consuming it. A failed fold can retry
    /// while later events continue arriving in the bounded live buffer.
    #[must_use]
    pub fn fold_candidate(&mut self) -> Vec<MemoryEvent> {
        if !self.normalize_sequences() {
            return Vec::new();
        }
        self.events
            .iter()
            .take(self.events.len() / 2)
            .cloned()
            .collect()
    }

    /// Retire only the exact event identities represented by a successful fold.
    /// Already-evicted events and repeated acknowledgements are harmless; new
    /// arrivals cannot be removed even when their time and text match the batch.
    pub fn acknowledge_fold(&mut self, batch: &[MemoryEvent]) {
        let acknowledged: HashSet<u64> = batch
            .iter()
            .map(|event| event.sequence)
            .filter(|sequence| *sequence != 0)
            .collect();
        self.events
            .retain(|event| !acknowledged.contains(&event.sequence));
    }

    /// Render meaningful conversation before ambient activity, strictly capped
    /// to UTF-8 bytes including timestamp/kind labels and newline separators.
    #[must_use]
    pub fn transcript(&self, max_bytes: usize) -> String {
        self.transcript_for(max_bytes, None)
    }

    /// Prefer speech involving the active conversational partner, then directed
    /// speech, own replies and outcomes, then other speech and background events.
    /// Select newest first within each tier, but render the selected lines in
    /// their original chronological order. Internal object IDs are never rendered.
    #[must_use]
    pub fn transcript_for(&self, max_bytes: usize, focus: Option<ObjId>) -> String {
        let mut candidates: Vec<_> = self.events.iter().enumerate().collect();
        candidates.sort_by_key(|(index, event)| {
            (
                std::cmp::Reverse(event.selection_priority(focus)),
                std::cmp::Reverse(*index),
            )
        });
        let mut chosen = Vec::new();
        let mut total = 0usize;
        for (index, event) in candidates {
            let separator = usize::from(!chosen.is_empty());
            let remaining = max_bytes.saturating_sub(total).saturating_sub(separator);
            if let Some(line) = event.render_bounded(remaining) {
                total += separator + line.len();
                chosen.push((index, line));
            }
        }
        chosen.sort_by_key(|(index, _)| *index);
        chosen
            .into_iter()
            .map(|(_, line)| line)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// One durable, subject-keyed memory (design 03 §6.5 layer 3), persisted as a
/// `mem:{npc}:{subject}` module document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    /// The NPC this belongs to.
    pub npc: ObjId,
    /// What it is about (a character name or topic), lowercased for matching.
    pub subject: String,
    /// The remembered note(s), newest last.
    pub notes: Vec<String>,
    /// When last written (unix seconds) — the recency key.
    pub updated: i64,
}

/// One bounded memory mutation plus an optional record evicted to preserve the
/// per-NPC subject ceiling. Callers must persist the delete and replacement in
/// the same world step.
#[derive(Clone, Debug, PartialEq)]
pub struct MemoryUpdate {
    /// The inserted or updated record.
    pub record: MemoryRecord,
    /// The least-recently-updated record removed when a new subject filled the
    /// per-NPC subject budget.
    pub evicted: Option<MemoryRecord>,
}

/// Why a durable memory record was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryRecordError {
    /// The subject does not produce one non-empty canonical key.
    InvalidSubject,
    /// The subject key exceeds its hard byte ceiling.
    SubjectTooLarge,
    /// A record contains no notes or more than the hard note count.
    InvalidNoteCount,
    /// At least one note is empty or exceeds its hard byte ceiling.
    InvalidNote,
    /// The encoded record exceeds its complete-document ceiling.
    RecordTooLarge,
}

/// The persistence key one record lives under (subject slugged so it is a safe
/// key). One document per `(npc, subject)`; notes accumulate inside it.
#[must_use]
pub fn record_key(npc: ObjId, subject: &str) -> SmolStr {
    SmolStr::new(format!("mem:{}:{}", npc.0, slug(subject)))
}

/// Lowercase + collapse to `[a-z0-9_]` for a safe, stable key/match token.
#[must_use]
pub fn slug(subject: &str) -> String {
    let mut s: String = subject
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    while s.contains("__") {
        s = s.replace("__", "_");
    }
    s.trim_matches('_').to_string()
}

/// The long-term store for one server: `(npc, subject-slug) -> record`, held in
/// the module state slot and mirrored by the persisted documents. Rebuilt from
/// boot data on load.
#[derive(Clone, Debug, Default)]
pub struct LongTermStore {
    records: HashMap<(ObjId, String), MemoryRecord>,
}

impl LongTermStore {
    /// Insert or update a record for `(npc, subject)`, appending `note` and
    /// stamping `updated`. A new subject beyond the per-NPC ceiling evicts the
    /// least recently updated record with a stable subject tie-break. Returns
    /// the complete mutation so the caller can persist both sides.
    pub fn remember(
        &mut self,
        npc: ObjId,
        subject: &str,
        note: &str,
        now: i64,
    ) -> Result<MemoryUpdate, MemoryRecordError> {
        let subject = canonical_subject(subject)?;
        let key = (npc, subject.clone());
        let mut rec = self.records.get(&key).cloned().unwrap_or(MemoryRecord {
            npc,
            subject,
            notes: Vec::new(),
            updated: now,
        });
        rec.notes.push(note.to_string());
        if rec.notes.len() > MAX_MEMORY_NOTES_PER_SUBJECT {
            let overflow = rec.notes.len() - MAX_MEMORY_NOTES_PER_SUBJECT;
            rec.notes.drain(..overflow);
        }
        rec.updated = now;
        validate_record(&rec)?;

        let evicted = if !self.records.contains_key(&key)
            && self.subject_count(npc) >= MAX_MEMORY_SUBJECTS_PER_NPC
        {
            let victim = self
                .records
                .iter()
                .filter(|((record_npc, _), _)| *record_npc == npc)
                .min_by(|left, right| {
                    left.1
                        .updated
                        .cmp(&right.1.updated)
                        .then_with(|| left.1.subject.cmp(&right.1.subject))
                })
                .map(|(victim, _)| victim.clone());
            victim.and_then(|victim| self.records.remove(&victim))
        } else {
            None
        };
        self.records.insert(key, rec.clone());
        Ok(MemoryUpdate {
            record: rec,
            evicted,
        })
    }

    /// Load one record from boot data. The same deterministic eviction policy
    /// makes the retained set independent of document iteration order.
    pub fn insert(&mut self, rec: MemoryRecord) -> Result<Option<MemoryRecord>, MemoryRecordError> {
        validate_record(&rec)?;
        let npc = rec.npc;
        let key = (npc, rec.subject.clone());
        self.records.insert(key, rec);
        let evicted = if self.subject_count(npc) > MAX_MEMORY_SUBJECTS_PER_NPC {
            let victim = self
                .records
                .iter()
                .filter(|((record_npc, _), _)| *record_npc == npc)
                .min_by(|left, right| {
                    left.1
                        .updated
                        .cmp(&right.1.updated)
                        .then_with(|| left.1.subject.cmp(&right.1.subject))
                })
                .map(|(victim, _)| victim.clone());
            victim.and_then(|victim| self.records.remove(&victim))
        } else {
            None
        };
        Ok(evicted)
    }

    /// The durable record for `(npc, subject)`, if any (subject matched by the
    /// same slug [`LongTermStore::recall`] uses). Backs
    /// [`familiarity_of`](crate::knowledge::familiarity_of).
    #[must_use]
    pub fn record_for(&self, npc: ObjId, subject: &str) -> Option<&MemoryRecord> {
        self.records.get(&(npc, slug(subject)))
    }

    /// The number of distinct durable-memory subjects recorded for `npc` (staff
    /// `@agent/show` uses this to report persisted memory).
    #[must_use]
    pub fn subject_count(&self, npc: ObjId) -> usize {
        self.records.keys().filter(|(id, _)| *id == npc).count()
    }

    /// Recall v1: records for `npc` whose subject matches any present player
    /// name, newest-first, up to `top_n` (design 03 §6.5). The embedding
    /// upgrade is the M10 [`MemoryRecall`] seam.
    #[must_use]
    pub fn recall(&self, npc: ObjId, present_names: &[String], top_n: usize) -> Vec<MemoryRecord> {
        let wanted: Vec<String> = present_names.iter().map(|n| slug(n)).collect();
        let mut hits: Vec<&MemoryRecord> = self
            .records
            .values()
            .filter(|r| r.npc == npc && wanted.contains(&r.subject))
            .collect();
        hits.sort_by_key(|r| std::cmp::Reverse(r.updated));
        hits.into_iter()
            .take(top_n.min(MAX_MEMORY_RECALL_RECORDS))
            .cloned()
            .collect()
    }
}

fn canonical_subject(subject: &str) -> Result<String, MemoryRecordError> {
    let subject = slug(subject);
    if subject.is_empty() {
        return Err(MemoryRecordError::InvalidSubject);
    }
    if subject.len() > MAX_MEMORY_SUBJECT_BYTES {
        return Err(MemoryRecordError::SubjectTooLarge);
    }
    Ok(subject)
}

fn validate_record(rec: &MemoryRecord) -> Result<(), MemoryRecordError> {
    let subject = canonical_subject(&rec.subject)?;
    if subject != rec.subject {
        return Err(MemoryRecordError::InvalidSubject);
    }
    if rec.notes.is_empty() || rec.notes.len() > MAX_MEMORY_NOTES_PER_SUBJECT {
        return Err(MemoryRecordError::InvalidNoteCount);
    }
    if rec
        .notes
        .iter()
        .any(|note| note.trim().is_empty() || note.len() > MAX_MEMORY_NOTE_BYTES)
    {
        return Err(MemoryRecordError::InvalidNote);
    }
    let encoded = serde_json::to_vec(rec).map_err(|_| MemoryRecordError::RecordTooLarge)?;
    if encoded.len() > MAX_MEMORY_RECORD_BYTES {
        return Err(MemoryRecordError::RecordTooLarge);
    }
    Ok(())
}

/// The seam for a smarter recall backend (embedding / vector search over Redis
/// 8), documented in the PLAN and design 03 §6.5 as an M10 extension. Recall v1
/// ([`LongTermStore::recall`]) is the built-in name-match implementation; this
/// trait is **declared, not built**. It adds the free-text `cue` the built-in
/// lacks, so a vector backend implements a strict superset. Landing one is a
/// small, localized change — add a `dyn MemoryRecall` slot to `NpcHostState` and
/// branch to it in `memory_context` — not a rewrite of the agent loop.
pub trait MemoryRecall: Send + Sync {
    /// Return the most relevant records for `npc` given the present player
    /// names and a free-text cue (the batched stimuli).
    fn recall(
        &self,
        npc: ObjId,
        present_names: &[String],
        cue: &str,
        top_n: usize,
    ) -> Vec<MemoryRecord>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_overflow_drains_old_half() {
        let mut b = MemoryBuffer::default();
        for i in 0..(BUFFER_CAP + 2) {
            b.push(i as i64, format!("event {i}"));
        }
        assert!(b.is_overflowing());
        let n_before = b.events.len();
        let old = b.drain_old_half();
        assert_eq!(old.len(), n_before / 2);
        assert_eq!(b.events.len(), n_before - old.len());
        assert_eq!(old[0].text, "event 0");
    }

    #[test]
    fn push_enforces_hard_cap_dropping_oldest() {
        let mut b = MemoryBuffer::default();
        // Far more than the hard cap, as under sustained chatter with no lull to
        // let a summarize turn drain the buffer.
        for i in 0..(HARD_CAP + 50) {
            b.push(i as i64, format!("event {i}"));
        }
        assert_eq!(b.events.len(), HARD_CAP, "RAM stays bounded");
        // The oldest lines were dropped; the newest survive.
        assert_eq!(
            b.events.last().unwrap().text,
            format!("event {}", HARD_CAP + 49)
        );
        assert!(!b.events.iter().any(|e| e.text == "event 0"));
    }

    #[test]
    fn push_enforces_per_event_and_aggregate_byte_bounds() {
        let mut buffer = MemoryBuffer::default();
        let large = "é".repeat(MAX_MEMORY_EVENT_CHARS * 2);
        for ts in 0..HARD_CAP as i64 {
            buffer.push(ts, &large);
        }
        assert!(
            buffer
                .events
                .iter()
                .all(|event| event.text.chars().count() <= MAX_MEMORY_EVENT_CHARS + 1)
        );
        assert!(
            buffer
                .events
                .iter()
                .map(|event| event.text.len())
                .sum::<usize>()
                <= MAX_MEMORY_BUFFER_BYTES
        );
    }

    #[test]
    fn transcript_caps_from_the_newest() {
        let mut b = MemoryBuffer::default();
        for i in 0..10 {
            b.push(i, format!("line{i}"));
        }
        let t = b.transcript(30);
        // Keeps the newest and drops the oldest.
        assert!(t.contains("line9"));
        assert!(!t.contains("line0"));
    }

    #[test]
    fn transcript_respects_every_tiny_utf8_byte_budget() {
        let mut buffer = MemoryBuffer::default();
        buffer.push_event(
            17,
            "界🙂é".repeat(MAX_MEMORY_EVENT_CHARS),
            MemoryContext {
                speaker: Some(ObjId(991_337)),
                room: Some(ObjId(991_338)),
                addressee: Some(ObjId(991_339)),
                kind: MemoryKind::SelfSpeech,
            },
        );
        for budget in 0..160 {
            let transcript = buffer.transcript_for(budget, Some(ObjId(991_339)));
            assert!(transcript.len() <= budget, "{budget}: {transcript:?}");
            assert!(!transcript.contains("99133"), "IDs stay out of the prompt");
            if !transcript.is_empty() {
                assert!(transcript.starts_with("[17; your speech] 界"));
            }
        }
        let full = buffer.events[0].render();
        assert_eq!(buffer.transcript(full.len()), full);
        assert_eq!(buffer.transcript(0), "");

        buffer.push(18, "A brief room event.");
        let complete: Vec<_> = buffer.events.iter().map(MemoryEvent::render).collect();
        let joined = complete.join("\n");
        assert_eq!(buffer.transcript(joined.len()), joined);
        assert!(buffer.transcript(joined.len() - 1).len() < joined.len());
    }

    #[test]
    fn ambient_flood_preserves_the_directed_exchange_and_renders_it_in_order() {
        let mut buffer = MemoryBuffer::default();
        let alice = ObjId(70);
        let npc = ObjId(80);
        buffer.push_event(
            1,
            "Alice says, \"Will you keep the parcel?\"",
            MemoryContext {
                speaker: Some(alice),
                addressee: Some(npc),
                kind: MemoryKind::DirectedSpeech,
                ..MemoryContext::default()
            },
        );
        buffer.push_event(
            2,
            "Bartender says, \"Until tomorrow.\"",
            MemoryContext {
                speaker: Some(npc),
                addressee: Some(alice),
                kind: MemoryKind::SelfSpeech,
                ..MemoryContext::default()
            },
        );
        let expected = buffer
            .events
            .iter()
            .map(MemoryEvent::render)
            .collect::<Vec<_>>()
            .join("\n");
        for index in 0..HARD_CAP * 3 {
            buffer.push(index as i64 + 3, format!("Bystander {index} arrives."));
        }
        assert_eq!(buffer.events.len(), HARD_CAP);
        assert_eq!(buffer.events[0].kind, MemoryKind::DirectedSpeech);
        assert_eq!(buffer.events[1].kind, MemoryKind::SelfSpeech);
        assert_eq!(buffer.transcript_for(expected.len(), Some(alice)), expected);
        assert_eq!(buffer.transcript(expected.len()), expected);
        assert!(
            buffer
                .events
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        );
    }

    #[test]
    fn focus_prefers_the_active_speaker_over_newer_unrelated_speech() {
        let mut buffer = MemoryBuffer::default();
        let alice = ObjId(70);
        buffer.push_event(
            10,
            "Alice says, \"The blue parcel.\"",
            MemoryContext {
                speaker: Some(alice),
                kind: MemoryKind::Speech,
                ..MemoryContext::default()
            },
        );
        let expected = buffer.events[0].render();
        buffer.push_event(
            11,
            "Bob asks about the weather.",
            MemoryContext {
                speaker: Some(ObjId(90)),
                kind: MemoryKind::DirectedSpeech,
                ..MemoryContext::default()
            },
        );
        assert_eq!(buffer.transcript_for(expected.len(), Some(alice)), expected);
        assert!(buffer.transcript(expected.len()).contains("Bob"));
    }

    #[test]
    fn meaningful_events_still_obey_count_and_byte_limits() {
        let mut buffer = MemoryBuffer::default();
        let context = MemoryContext {
            kind: MemoryKind::Outcome,
            ..MemoryContext::default()
        };
        for index in 0..HARD_CAP + 20 {
            buffer.push_event(index as i64, format!("result {index}"), context);
        }
        assert_eq!(buffer.events.len(), HARD_CAP);
        assert_eq!(buffer.events[0].text, "result 20");
        let large = "🙂".repeat(MAX_MEMORY_EVENT_CHARS);
        for index in 0..HARD_CAP {
            buffer.push_event(index as i64, &large, context);
        }
        assert!(
            buffer.events.len() < HARD_CAP,
            "the byte ceiling also evicts"
        );
        assert!(
            buffer
                .events
                .iter()
                .map(|event| event.text.len())
                .sum::<usize>()
                <= MAX_MEMORY_BUFFER_BYTES
        );
        let sequences: HashSet<_> = buffer.events.iter().map(|event| event.sequence).collect();
        assert_eq!(sequences.len(), buffer.events.len());
    }

    #[test]
    fn fold_acknowledgement_preserves_identical_interleaved_arrivals() {
        let mut buffer = MemoryBuffer::default();
        for index in 0..4 {
            buffer.push(index, format!("line {index}"));
        }
        let before = buffer.events.clone();
        let batch = buffer.fold_candidate();
        assert_eq!(batch, before[..2]);
        assert_eq!(
            buffer.events, before,
            "snapshotting does not consume events"
        );
        buffer.push(batch[0].ts, batch[0].text.clone());
        let arrival = buffer.events.last().unwrap().clone();
        assert!(arrival.sequence > batch[0].sequence);
        buffer.acknowledge_fold(&batch);
        assert_eq!(buffer.events.len(), 3);
        assert!(buffer.events.contains(&arrival));
        assert!(!buffer.events.contains(&batch[0]));
        let after = buffer.events.clone();
        buffer.acknowledge_fold(&batch);
        assert_eq!(buffer.events, after, "acknowledgement is idempotent");
    }

    #[test]
    fn fold_acknowledgement_tolerates_emergency_eviction_without_deleting_new_events() {
        let mut buffer = MemoryBuffer::default();
        for index in 0..HARD_CAP {
            buffer.push(index as i64, format!("line {index}"));
        }
        let batch = buffer.fold_candidate();
        let watermark = buffer.next_sequence;
        for index in 0..20 {
            buffer.push(index, format!("new {index}"));
        }
        assert!(
            !buffer.events.contains(&batch[0]),
            "some folded candidates were evicted"
        );
        let arrivals: Vec<_> = buffer
            .events
            .iter()
            .filter(|event| event.sequence >= watermark)
            .cloned()
            .collect();
        buffer.acknowledge_fold(&batch);
        assert_eq!(arrivals.len(), 20);
        assert!(arrivals.iter().all(|event| buffer.events.contains(event)));
        assert!(batch.iter().all(|event| !buffer.events.contains(event)));
        assert_eq!(buffer.events.len(), HARD_CAP / 2 + arrivals.len());
    }

    #[test]
    fn legacy_serialized_events_get_stable_fold_identities() {
        let old = serde_json::json!({
            "summary": "Earlier tonight.",
            "events": [
                {"ts": 1, "text": "Alice arrives."},
                {"ts": 2, "text": "Alice says hello."},
                {"ts": 3, "text": "Bartender replies."},
                {"ts": 4, "text": "A door opens."}
            ]
        });
        let mut buffer: MemoryBuffer = serde_json::from_value(old).unwrap();
        assert_eq!(buffer.next_sequence, 0);
        assert!(buffer.events.iter().all(|event| event.sequence == 0
            && event.kind == MemoryKind::Ambient
            && event.speaker.is_none()
            && event.room.is_none()
            && event.addressee.is_none()));
        let unissued = buffer.events.clone();
        buffer.acknowledge_fold(&unissued);
        assert_eq!(
            buffer.events.len(),
            4,
            "legacy zero is not an issued identity"
        );
        let batch = buffer.fold_candidate();
        assert_eq!(
            batch.iter().map(|event| event.sequence).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            buffer.fold_candidate(),
            batch,
            "retry retains the identities"
        );
        let encoded = serde_json::to_vec(&buffer).unwrap();
        let mut decoded: MemoryBuffer = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, buffer);
        decoded.push(5, "Another arrival.");
        decoded.acknowledge_fold(&batch);
        assert_eq!(
            decoded
                .events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
    }

    #[test]
    fn exhausted_sequence_never_reuses_an_issued_fold_identity() {
        let mut buffer = MemoryBuffer::default();
        buffer.push(1, "Old line.");
        buffer.next_sequence = u64::MAX;
        let before = buffer.events.clone();
        buffer.push(2, "Must not reuse the old identity.");
        assert_eq!(buffer.events, before);
        assert_eq!(buffer.next_sequence, u64::MAX);
    }

    #[test]
    fn slug_is_safe_and_stable() {
        assert_eq!(slug("Alice O'Brien"), "alice_o_brien");
        assert_eq!(slug("  The Cellar!!  "), "the_cellar");
        assert_eq!(record_key(ObjId(7), "Alice").as_str(), "mem:7:alice");
    }

    #[test]
    fn recall_matches_present_players_newest_first() {
        let mut s = LongTermStore::default();
        let npc = ObjId(9);
        s.remember(npc, "Alice", "owes me 50 dollars", 100).unwrap();
        s.remember(npc, "Bob", "hates the cold", 200).unwrap();
        s.remember(npc, "Alice", "asked about the cellar", 300)
            .unwrap();
        // Only Alice is present.
        let hits = s.recall(npc, &["Alice".into()], 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].subject, "alice");
        assert_eq!(hits[0].notes.len(), 2);
        // Both present: newest subject-record first.
        let hits = s.recall(npc, &["Alice".into(), "Bob".into()], 5);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].subject, "alice"); // updated=300 > bob 200
        // A different NPC shares nothing.
        assert!(s.recall(ObjId(10), &["Alice".into()], 5).is_empty());
    }

    #[test]
    fn subjects_evict_oldest_with_stable_tie_break() {
        let mut store = LongTermStore::default();
        let npc = ObjId(11);
        for index in 0..MAX_MEMORY_SUBJECTS_PER_NPC {
            store
                .remember(npc, &format!("subject_{index:03}"), "note", 10)
                .unwrap();
        }
        let update = store.remember(npc, "newest", "note", 20).unwrap();
        assert_eq!(store.subject_count(npc), MAX_MEMORY_SUBJECTS_PER_NPC);
        assert_eq!(update.evicted.unwrap().subject, "subject_000");
        assert!(store.record_for(npc, "newest").is_some());
    }

    #[test]
    fn invalid_and_oversized_records_are_refused_without_mutation() {
        let mut store = LongTermStore::default();
        assert_eq!(
            store.remember(ObjId(1), "!!!", "note", 1),
            Err(MemoryRecordError::InvalidSubject)
        );
        assert_eq!(
            store.remember(ObjId(1), "alice", &"x".repeat(MAX_MEMORY_NOTE_BYTES + 1), 1),
            Err(MemoryRecordError::InvalidNote)
        );
        assert_eq!(store.subject_count(ObjId(1)), 0);
    }

    #[test]
    fn recall_has_a_hard_result_ceiling() {
        let mut store = LongTermStore::default();
        let npc = ObjId(12);
        let mut present = Vec::new();
        for index in 0..(MAX_MEMORY_RECALL_RECORDS + 4) {
            let subject = format!("player_{index}");
            store.remember(npc, &subject, "note", index as i64).unwrap();
            present.push(subject);
        }
        assert_eq!(
            store.recall(npc, &present, usize::MAX).len(),
            MAX_MEMORY_RECALL_RECORDS
        );
    }

    #[test]
    fn boot_insertion_keeps_the_same_newest_set_in_any_order() {
        let npc = ObjId(13);
        let records: Vec<_> = (0..=MAX_MEMORY_SUBJECTS_PER_NPC)
            .map(|index| MemoryRecord {
                npc,
                subject: format!("subject_{index:03}"),
                notes: vec!["note".to_owned()],
                updated: index as i64,
            })
            .collect();
        let mut forward = LongTermStore::default();
        for record in &records {
            forward.insert(record.clone()).unwrap();
        }
        let mut reverse = LongTermStore::default();
        for record in records.iter().rev() {
            reverse.insert(record.clone()).unwrap();
        }
        assert_eq!(
            forward.record_for(npc, "subject_000").is_some(),
            reverse.record_for(npc, "subject_000").is_some()
        );
        assert!(forward.record_for(npc, "subject_000").is_none());
        assert!(reverse.record_for(npc, "subject_256").is_some());
        assert_eq!(forward.subject_count(npc), MAX_MEMORY_SUBJECTS_PER_NPC);
        assert_eq!(reverse.subject_count(npc), MAX_MEMORY_SUBJECTS_PER_NPC);
    }
}
