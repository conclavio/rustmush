//! NPC memory (design 03 §6.5), three layers cheapest-first:
//!
//! 1. [`MemoryBuffer`] — an in-RAM rolling buffer of recent room events, token-
//!    capped and lost on restart (acceptable).
//! 2. A persisted conversation summary (attribute
//!    [`SUMMARY_ATTR`](crate::profile::SUMMARY_ATTR)) — when the buffer
//!    overflows, an `NpcSummarize` turn folds the oldest half into it.
//! 3. [`MemoryRecord`]s — durable, subject-keyed facts persisted as
//!    `mem:{npc}:{subject}` module documents, written by the `remember` tool
//!    and the summarizer.
//!
//! Recall v1 is deliberately dumb ([`recall`]): fetch records whose subject
//! matches a present player's name, newest-first. Embedding recall is a
//! documented M10 seam behind the [`MemoryRecall`] trait — **not built here**.

use std::collections::HashMap;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// How many recent events the rolling buffer keeps before it overflows and a
/// summarize turn is triggered (design 03 §6.5: ~50, token-capped).
pub const BUFFER_CAP: usize = 50;

/// One line in the rolling buffer — a script-formatted room event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryEvent {
    /// IC/wall timestamp (unix seconds).
    pub ts: i64,
    /// The formatted line, e.g. `Alice says, "…"` or `Bob arrives.`.
    pub text: String,
}

/// The per-NPC rolling event buffer (design 03 §6.5 layer 1). In-RAM only;
/// lost on restart.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryBuffer {
    /// The persisted rolling summary (layer 2), cached here for the prompt.
    pub summary: String,
    /// Recent events, oldest first.
    pub events: Vec<MemoryEvent>,
}

/// Hard ceiling on retained events, regardless of summarize cadence. Under
/// sustained sub-tick chatter the mailbox can refill before each tick, starving
/// the summarize path that normally drains the buffer; this bounds RAM anyway by
/// dropping the oldest lines beyond the cap (they would have been folded into the
/// summary at the next lull). Set well above [`BUFFER_CAP`] so normal folding is
/// unaffected.
pub const HARD_CAP: usize = BUFFER_CAP * 2;

impl MemoryBuffer {
    /// Append an event, enforcing the hard RAM cap (oldest dropped past
    /// [`HARD_CAP`]).
    pub fn push(&mut self, ts: i64, text: impl Into<String>) {
        self.events.push(MemoryEvent {
            ts,
            text: text.into(),
        });
        if self.events.len() > HARD_CAP {
            let overflow = self.events.len() - HARD_CAP;
            self.events.drain(..overflow);
        }
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

    /// Render the recent transcript, newest-last, capped to `max_chars`
    /// (design 03 §6.3 item 7). Older lines are dropped first.
    #[must_use]
    pub fn transcript(&self, max_chars: usize) -> String {
        let mut chosen: Vec<&str> = Vec::new();
        let mut total = 0usize;
        for ev in self.events.iter().rev() {
            let len = ev.text.len() + 1;
            if total + len > max_chars && !chosen.is_empty() {
                break;
            }
            total += len;
            chosen.push(ev.text.as_str());
        }
        chosen.reverse();
        chosen.join("\n")
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
    /// stamping `updated`. Returns the merged record (to persist).
    pub fn remember(&mut self, npc: ObjId, subject: &str, note: &str, now: i64) -> MemoryRecord {
        let key = (npc, slug(subject));
        let rec = self.records.entry(key).or_insert_with(|| MemoryRecord {
            npc,
            subject: slug(subject),
            notes: Vec::new(),
            updated: now,
        });
        rec.notes.push(note.to_string());
        // Keep the note list bounded so a single subject can't grow forever.
        if rec.notes.len() > 20 {
            let overflow = rec.notes.len() - 20;
            rec.notes.drain(..overflow);
        }
        rec.updated = now;
        rec.clone()
    }

    /// Load one record (from boot data).
    pub fn insert(&mut self, rec: MemoryRecord) {
        self.records.insert((rec.npc, slug(&rec.subject)), rec);
    }

    /// The durable record for `(npc, subject)`, if any (subject matched by the
    /// same slug [`recall`](LongTermStore::recall) uses). Backs
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
        hits.into_iter().take(top_n).cloned().collect()
    }
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
    fn transcript_caps_from_the_newest() {
        let mut b = MemoryBuffer::default();
        for i in 0..10 {
            b.push(i, format!("line{i}"));
        }
        let t = b.transcript(20);
        // Keeps the newest and drops the oldest.
        assert!(t.contains("line9"));
        assert!(!t.contains("line0"));
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
        s.remember(npc, "Alice", "owes me 50 dollars", 100);
        s.remember(npc, "Bob", "hates the cold", 200);
        s.remember(npc, "Alice", "asked about the cellar", 300);
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
}
