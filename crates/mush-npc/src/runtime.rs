//! The per-NPC runtime and the host state slot (design 03 §6.2). `mush-core` is
//! sync and owns the world, so the design's "per-NPC tokio task" becomes
//! **per-NPC state** held in a module state slot and driven by the module's
//! tick + observers:
//!
//! - a mailbox of pending [`Stimulus`]es,
//! - a coalescing window (first-stimulus timestamp),
//! - a cooldown clock,
//! - an in-flight flag (one LLM call per NPC),
//! - the rolling [`MemoryBuffer`].
//!
//! The tick polls [`NpcHostState`] each cadence; a ready NPC's batch is drained
//! and fired as one off-loop turn.

use std::collections::HashMap;
use std::time::Duration;

use mush_core::{ObjId, PanelFeedHandle};

use crate::memory::{LongTermStore, MemoryBuffer};
use crate::stimulus::Stimulus;

/// Host tuning knobs.
#[derive(Clone, Debug)]
pub struct NpcConfig {
    /// Coalescing window: collect stimuli for this long before firing one turn
    /// (design 03 §6.2: 1–2s).
    pub coalesce: Duration,
    /// The poll cadence of the driving tick.
    pub tick: Duration,
    /// Max characters of recent transcript put in the prompt.
    pub transcript_chars: usize,
    /// How many long-term memories to recall per turn.
    pub recall_top_n: usize,
}

impl Default for NpcConfig {
    fn default() -> NpcConfig {
        NpcConfig {
            coalesce: Duration::from_millis(1500),
            tick: Duration::from_secs(1),
            transcript_chars: 1500,
            recall_top_n: 5,
        }
    }
}

/// One NPC's live runtime state (design 03 §6.2). Cloneable — the whole host
/// slot is cloned when a `World` snapshot is taken.
#[derive(Clone, Debug, Default)]
pub struct NpcRuntime {
    /// Pending stimuli awaiting the next turn.
    pub mailbox: Vec<Stimulus>,
    /// When the current batch's first stimulus arrived (ms), for the window.
    pub pending_since: Option<i64>,
    /// One LLM call at a time: `true` between fire and completion.
    pub in_flight: bool,
    /// The in-flight turn's correlation id.
    pub correlation: Option<u64>,
    /// When the last turn started (ms) — the cooldown anchor.
    pub last_turn_at: Option<i64>,
    /// The rolling memory buffer (layer 1) + cached summary (layer 2).
    pub memory: MemoryBuffer,
    /// Whether the one-time "degraded to fallback" staff notice has fired.
    pub degraded_notified: bool,
    /// Whether a memory-fold (summarize) turn is pending.
    pub summarize_pending: bool,
}

impl NpcRuntime {
    /// Whether the current batch contains a direct address (shortens cooldown).
    #[must_use]
    pub fn has_direct(&self) -> bool {
        self.mailbox.iter().any(Stimulus::is_direct)
    }

    /// Whether the coalescing window has elapsed for the current batch.
    #[must_use]
    pub fn window_elapsed(&self, now_ms: i64, coalesce: Duration) -> bool {
        match self.pending_since {
            Some(t) => now_ms - t >= dur_ms(coalesce),
            None => false,
        }
    }

    /// Whether the cooldown allows a turn now. A direct address halves it;
    /// ambience never shortens it (design 03 §6.2).
    #[must_use]
    pub fn cooldown_ok(&self, now_ms: i64, cooldown: Duration, direct: bool) -> bool {
        match self.last_turn_at {
            None => true,
            Some(t) => {
                let cd = dur_ms(cooldown);
                let cd = if direct { cd / 2 } else { cd };
                now_ms - t >= cd
            }
        }
    }
}

/// The module state slot: every NPC's runtime plus the shared long-term store
/// and the `npc.thinking` panel handle.
#[derive(Clone)]
pub struct NpcHostState {
    /// Per-NPC runtimes, created lazily on first stimulus.
    pub runtimes: HashMap<ObjId, NpcRuntime>,
    /// Durable subject-keyed memories (layer 3), rebuilt from boot data.
    pub long_term: LongTermStore,
    /// The `npc.thinking` panel feed handle (`None` in headless/test worlds).
    pub thinking: Option<PanelFeedHandle>,
    /// Tuning.
    pub config: NpcConfig,
}

impl NpcHostState {
    /// A fresh host around the reserved panel handle.
    #[must_use]
    pub fn new(thinking: PanelFeedHandle) -> NpcHostState {
        NpcHostState {
            runtimes: HashMap::new(),
            long_term: LongTermStore::default(),
            thinking: Some(thinking),
            config: NpcConfig::default(),
        }
    }

    /// A host with no panel feed (headless tests).
    #[must_use]
    pub fn headless() -> NpcHostState {
        NpcHostState {
            runtimes: HashMap::new(),
            long_term: LongTermStore::default(),
            thinking: None,
            config: NpcConfig::default(),
        }
    }

    /// The runtime for `npc`, created empty if absent.
    pub fn runtime_mut(&mut self, npc: ObjId) -> &mut NpcRuntime {
        self.runtimes.entry(npc).or_default()
    }

    /// Read a runtime, if present.
    #[must_use]
    pub fn runtime(&self, npc: ObjId) -> Option<&NpcRuntime> {
        self.runtimes.get(&npc)
    }

    /// Enqueue a stimulus for `npc` at `now_ms`. While a turn is in flight,
    /// **low-value** stimuli (arrivals/departures/ambience) are dropped as
    /// stale (design 03 §6.2); speech always queues for the next turn.
    /// Returns whether it was actually enqueued.
    pub fn enqueue(&mut self, npc: ObjId, stim: Stimulus, now_ms: i64) -> bool {
        let rt = self.runtime_mut(npc);
        if rt.in_flight && stim.is_low_value() {
            return false;
        }
        if rt.pending_since.is_none() {
            rt.pending_since = Some(now_ms);
        }
        rt.mailbox.push(stim);
        true
    }

    /// NPCs with a non-empty mailbox that are not currently in flight.
    #[must_use]
    pub fn pending(&self) -> Vec<ObjId> {
        let mut v: Vec<ObjId> = self
            .runtimes
            .iter()
            .filter(|(_, rt)| !rt.in_flight && (!rt.mailbox.is_empty() || rt.summarize_pending))
            .map(|(id, _)| *id)
            .collect();
        v.sort_by_key(|id| id.0);
        v
    }

    /// Start a turn for `npc`: drain the mailbox, mark in flight, anchor the
    /// cooldown at `now_ms`, and record the correlation id. Returns the drained
    /// batch (may be empty for a pure summarize turn).
    pub fn begin_turn(&mut self, npc: ObjId, now_ms: i64, correlation: u64) -> Vec<Stimulus> {
        let rt = self.runtime_mut(npc);
        rt.in_flight = true;
        rt.correlation = Some(correlation);
        rt.last_turn_at = Some(now_ms);
        rt.pending_since = None;
        std::mem::take(&mut rt.mailbox)
    }

    /// Start a memory-fold (summarize) turn: mark in flight and drain the
    /// oldest half of the buffer to fold. Returns the drained event texts.
    pub fn begin_summarize(&mut self, npc: ObjId, correlation: u64) -> Vec<String> {
        let rt = self.runtime_mut(npc);
        rt.in_flight = true;
        rt.correlation = Some(correlation);
        rt.summarize_pending = false;
        rt.memory
            .drain_old_half()
            .into_iter()
            .map(|e| e.text)
            .collect()
    }

    /// Finish a turn: clear the in-flight flag (cooldown was anchored at
    /// [`NpcHostState::begin_turn`]). If new stimuli queued during the call and
    /// the mailbox is non-empty, restart the window so the next turn is
    /// scheduled.
    pub fn finish_turn(&mut self, npc: ObjId, now_ms: i64) {
        if let Some(rt) = self.runtimes.get_mut(&npc) {
            rt.in_flight = false;
            rt.correlation = None;
            if !rt.mailbox.is_empty() && rt.pending_since.is_none() {
                rt.pending_since = Some(now_ms);
            }
        }
    }

    /// Drop an NPC's pending mailbox and reset its window (no players, or the
    /// NPC is disabled — no wakes, no tokens).
    pub fn drop_mailbox(&mut self, npc: ObjId) {
        if let Some(rt) = self.runtimes.get_mut(&npc) {
            rt.mailbox.clear();
            rt.pending_since = None;
        }
    }
}

/// A `Duration` as milliseconds (clamped).
fn dur_ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stimulus::{SpeechKind, Stimulus};

    fn host() -> NpcHostState {
        NpcHostState::headless()
    }

    fn spoken(by: u32, text: &str) -> Stimulus {
        Stimulus::SpokenTo {
            by: ObjId(by),
            text: text.into(),
            kind: SpeechKind::Say,
        }
    }

    #[test]
    fn rapid_chatter_coalesces_into_one_batch() {
        let mut h = host();
        let npc = ObjId(9);
        h.enqueue(npc, spoken(2, "hi"), 1000);
        h.enqueue(npc, spoken(3, "hello"), 1100);
        h.enqueue(
            npc,
            Stimulus::HeardInRoom {
                by: ObjId(2),
                text: "chatter".into(),
            },
            1200,
        );
        // Window not elapsed yet at 1200 (coalesce 1500ms from 1000).
        let rt = h.runtime(npc).unwrap();
        assert!(!rt.window_elapsed(1200, h.config.coalesce));
        // Elapsed at 2500.
        assert!(
            h.runtime(npc)
                .unwrap()
                .window_elapsed(2500, h.config.coalesce)
        );
        let batch = h.begin_turn(npc, 2500, 1);
        assert_eq!(batch.len(), 3);
        // Mailbox drained, in flight.
        assert!(h.runtime(npc).unwrap().in_flight);
        assert!(h.runtime(npc).unwrap().mailbox.is_empty());
    }

    #[test]
    fn one_in_flight_low_value_dropped_speech_queued() {
        let mut h = host();
        let npc = ObjId(9);
        h.enqueue(npc, spoken(2, "hi"), 1000);
        h.begin_turn(npc, 2500, 1);
        assert!(h.runtime(npc).unwrap().in_flight);
        // Arrivals during the call are dropped as stale.
        let arrived = h.enqueue(npc, Stimulus::PlayerArrived { who: ObjId(3) }, 2600);
        assert!(!arrived);
        // Speech queues for the next turn.
        let queued = h.enqueue(npc, spoken(2, "still there?"), 2700);
        assert!(queued);
        assert_eq!(h.runtime(npc).unwrap().mailbox.len(), 1);
        // Not pending while in flight.
        assert!(h.pending().is_empty());
        // After completion, the queued speech makes it pending again.
        h.finish_turn(npc, 2800);
        assert_eq!(h.pending(), vec![npc]);
    }

    #[test]
    fn cooldown_blocks_then_allows_direct_shortens() {
        let mut h = host();
        let npc = ObjId(9);
        // First turn at t=0.
        h.enqueue(npc, spoken(2, "hi"), 0);
        h.begin_turn(npc, 0, 1);
        h.finish_turn(npc, 100);
        let cd = Duration::from_secs(15); // Minor default
        let rt = h.runtime(npc).unwrap();
        // 5s later: ambient batch still blocked (full 15s cooldown).
        assert!(!rt.cooldown_ok(5_000, cd, false));
        // A direct address halves it to 7.5s: 8s later it's allowed.
        assert!(rt.cooldown_ok(8_000, cd, true));
        // Full window always allowed.
        assert!(rt.cooldown_ok(15_000, cd, false));
    }

    #[test]
    fn no_stimuli_means_not_pending() {
        let h = host();
        assert!(h.pending().is_empty());
    }
}
