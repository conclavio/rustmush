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

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use mush_core::{ObjId, PanelFeedHandle};

use crate::experience::PersonalMemoryStore;
use crate::intel::IntelStore;
use crate::knowledge::LorePools;
use crate::memory::{LongTermStore, MemoryBuffer, MemoryEvent};
use crate::snapshot::{MemoryReferences, RecalledMemory};
use crate::stimulus::Stimulus;

/// Hard ceiling on queued stimuli for one NPC. The mailbox is fed by player
/// activity, so this is a security bound rather than merely a tuning default.
pub const MAX_PENDING_STIMULI: usize = 64;

/// Hard ceiling on the serialized payload retained in one NPC mailbox.
pub const MAX_PENDING_STIMULUS_BYTES: usize = 32 * 1024;

/// H2b: the default scene-turn decision deadline, seconds (11-h §7
/// `NPC_SCENE_TURN_DEADLINE_SECS`; `[npc].scene_turn_deadline_secs`).
pub const DEFAULT_SCENE_TURN_DEADLINE_SECS: i64 = 120;

/// A failed memory fold first retries after thirty seconds.
pub const SUMMARY_RETRY_INITIAL_MS: i64 = 30_000;

/// Repeated memory-fold failures wait no longer than five minutes.
pub const SUMMARY_RETRY_MAX_MS: i64 = 300_000;

/// How long a participant remains the NPC's conversational focus, in milliseconds.
pub const CONVERSATION_FOCUS_MS: i64 = 120_000;

/// The exact events submitted to one summarizer. They remain in working memory
/// until that call succeeds, and arrivals cannot become part of its acknowledgement.
#[derive(Clone, Debug)]
pub struct PendingFold {
    /// The summarizer call that owns this batch.
    pub correlation: u64,
    /// Stable event identities captured before the off-loop call.
    pub events: Vec<MemoryEvent>,
}

/// A recent conversational partner, scoped to the room where they spoke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversationFocus {
    /// The other participant.
    pub participant: ObjId,
    /// The room in which the conversation was observed.
    pub room: ObjId,
    /// Last relevant speech, in Unix milliseconds.
    pub last_at: i64,
}

/// Host tuning knobs.
#[derive(Clone, Debug)]
pub struct NpcConfig {
    /// Coalescing window: collect stimuli for this long before firing one turn
    /// (design 03 §6.2: 1–2s).
    pub coalesce: Duration,
    /// The poll cadence of the driving tick.
    pub tick: Duration,
    /// Maximum UTF-8 bytes of recent transcript put in the prompt. The field
    /// retains its legacy name; this is a byte budget, not a character count.
    pub transcript_chars: usize,
    /// How many long-term memories to recall per turn.
    pub recall_top_n: usize,
    /// H2b: how long an NPC's scene turn may wait for its durable decision
    /// before the idle fallback fires, seconds (`[npc].scene_turn_deadline_secs`).
    pub scene_turn_deadline_secs: i64,
}

impl Default for NpcConfig {
    fn default() -> NpcConfig {
        NpcConfig {
            coalesce: Duration::from_millis(1500),
            tick: Duration::from_secs(1),
            transcript_chars: 6000,
            recall_top_n: 5,
            scene_turn_deadline_secs: DEFAULT_SCENE_TURN_DEADLINE_SECS,
        }
    }
}

/// One NPC's live runtime state (design 03 §6.2). Cloneable — the whole host
/// slot is cloned when a `World` snapshot is taken.
#[derive(Clone, Debug, Default)]
pub struct NpcRuntime {
    /// Pending stimuli awaiting the next turn.
    pub mailbox: Vec<Stimulus>,
    /// Serialized bytes currently retained by `mailbox`.
    pub mailbox_bytes: usize,
    /// When the current batch's first stimulus arrived (ms), for the window.
    pub pending_since: Option<i64>,
    /// One LLM call at a time: `true` between fire and completion.
    pub in_flight: bool,
    /// The in-flight turn's correlation id.
    pub correlation: Option<u64>,
    /// Reference-bearing memory actions must cite this exact captured turn.
    pub(crate) captured_memory: Option<(u64, MemoryReferences)>,
    /// The newest correlation this NPC has issued or applied, in flight or
    /// not. The world allocates correlations monotonically, so a completion
    /// at or below this watermark is a replay.
    pub last_correlation: Option<u64>,
    /// When the last turn started (ms) — the cooldown anchor.
    pub last_turn_at: Option<i64>,
    /// The rolling memory buffer (layer 1) + cached summary (layer 2).
    pub memory: MemoryBuffer,
    /// Whether the one-time "degraded to fallback" staff notice has fired.
    pub degraded_notified: bool,
    /// Whether a memory-fold (summarize) turn is pending.
    pub summarize_pending: bool,
    /// Events awaiting a correlated summarizer result; never drained at submission.
    pub pending_fold: Option<PendingFold>,
    /// Consecutive failed folds, saturated once the retry delay reaches its cap.
    pub summarize_failures: u8,
    /// Earliest next summarizer attempt, in milliseconds.
    pub summarize_retry_at: Option<i64>,
    /// One ordinary turn may run before the next eligible fold.
    pub dialogue_since_summary: bool,
    /// Short-lived participant focus; never a source of additional perception rights.
    pub conversation_focus: Option<ConversationFocus>,
    /// Stimuli refused or evicted to maintain the hard mailbox bounds.
    pub dropped_stimuli: u64,
}

impl NpcRuntime {
    /// Whether a requested fold may start after its retry delay.
    #[must_use]
    pub fn summarize_ready(&self, now_ms: i64) -> bool {
        self.summarize_pending
            && !self.in_flight
            && self.pending_fold.is_none()
            && self.summarize_retry_at.is_none_or(|at| now_ms >= at)
    }

    /// Give queued dialogue one turn between eligible folds, while preventing a
    /// constantly refilled mailbox from starving memory consolidation.
    #[must_use]
    pub fn fold_due(&self, now_ms: i64) -> bool {
        self.summarize_ready(now_ms) && (self.mailbox.is_empty() || self.dialogue_since_summary)
    }

    /// The recent conversational partner in this room, if the focus is still fresh.
    #[must_use]
    pub fn focus_for(&self, room: ObjId, now_ms: i64) -> Option<ObjId> {
        let focus = self.conversation_focus?;
        let elapsed = now_ms.checked_sub(focus.last_at)?;
        (focus.room == room && (0..CONVERSATION_FOCUS_MS).contains(&elapsed))
            .then_some(focus.participant)
    }

    /// Refresh the participant and room of a conversation the NPC perceived.
    pub fn note_conversation(&mut self, participant: ObjId, room: ObjId, now_ms: i64) {
        self.conversation_focus = Some(ConversationFocus {
            participant,
            room,
            last_at: now_ms,
        });
    }

    /// Forget a departing participant without clearing an unrelated conversation.
    pub fn clear_conversation(&mut self, participant: ObjId) {
        if self
            .conversation_focus
            .is_some_and(|focus| focus.participant == participant)
        {
            self.conversation_focus = None;
        }
    }

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
    /// Engine-observed experiences and subjective beliefs, mirrored by `mind:{npc}`.
    pub personal_memory: PersonalMemoryStore,
    /// Shared world-knowledge lore pools (design: M10d), rebuilt from boot data
    /// and mirrored by the persisted `lore:{slug}` documents.
    pub lore_pools: LorePools,
    /// The M12 informant loop's intel log: provenance-tagged records grouped by
    /// principal, rebuilt from boot data and mirrored by the persisted
    /// `intel:{principal}` documents.
    pub intel: IntelStore,
    /// The `npc.thinking` panel feed handle (`None` in headless/test worlds).
    pub thinking: Option<PanelFeedHandle>,
    /// Tuning.
    pub config: NpcConfig,
    /// H2a: decisions waiting for the agent tick to capture them, oldest
    /// first. The tick issues **one per step** (a step that captures twice
    /// captures nothing), so this is a queue, not a set.
    pub decision_requests: VecDeque<crate::decision::NpcDecisionRequest>,
    /// S6 (doc 14 §4.6 rule 4): every interface-ability call, newest last,
    /// at most [`MAX_INTERFACE_AUDIT_ROWS`]; `@agent/audit` reads it.
    pub interface_audit: VecDeque<InterfaceAuditRow>,
}

/// The most interface-ability audit rows kept.
pub const MAX_INTERFACE_AUDIT_ROWS: usize = 256;

/// One interface-ability call: who fronted it, who asked, what, with what.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InterfaceAuditRow {
    /// Unix seconds.
    pub at: i64,
    /// The NPC.
    pub npc: ObjId,
    /// The asker the ability ran as.
    pub asker: ObjId,
    /// The ability name.
    pub ability: smol_str::SmolStr,
    /// The input, abbreviated.
    pub input: String,
    /// Whether it answered (else refused).
    pub ok: bool,
}

impl NpcHostState {
    /// Record one interface-ability call.
    pub fn audit_interface(&mut self, row: InterfaceAuditRow) {
        if self.interface_audit.len() >= MAX_INTERFACE_AUDIT_ROWS {
            self.interface_audit.pop_front();
        }
        self.interface_audit.push_back(row);
    }
}

impl NpcHostState {
    /// H2a: queue one durable-decision request for the tick to capture.
    /// Returns `false` (and keeps nothing) when the queue is full
    /// (`MAX_DECISION_REQUESTS`).
    pub fn request_decision(&mut self, request: crate::decision::NpcDecisionRequest) -> bool {
        if self.decision_requests.len() >= crate::decision::MAX_DECISION_REQUESTS {
            return false;
        }
        self.decision_requests.push_back(request);
        true
    }

    /// A fresh host around the reserved panel handle.
    #[must_use]
    pub fn new(thinking: PanelFeedHandle) -> NpcHostState {
        NpcHostState {
            runtimes: HashMap::new(),
            long_term: LongTermStore::default(),
            personal_memory: PersonalMemoryStore::default(),
            lore_pools: LorePools::default(),
            intel: IntelStore::default(),
            thinking: Some(thinking),
            config: NpcConfig::default(),
            decision_requests: VecDeque::new(),
            interface_audit: VecDeque::new(),
        }
    }

    /// A host with no panel feed (headless tests).
    #[must_use]
    pub fn headless() -> NpcHostState {
        NpcHostState {
            runtimes: HashMap::new(),
            long_term: LongTermStore::default(),
            personal_memory: PersonalMemoryStore::default(),
            lore_pools: LorePools::default(),
            intel: IntelStore::default(),
            thinking: None,
            config: NpcConfig::default(),
            decision_requests: VecDeque::new(),
            interface_audit: VecDeque::new(),
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
    /// stale (design 03 §6.2). At the hard count/byte ceilings, newer stimuli
    /// replace the oldest stimulus of no greater priority; direct speech has
    /// the highest priority, ordinary speech/game events are next, and stale
    /// presence/ambient events are lowest. Nothing can grow this mailbox
    /// beyond its explicit bounds.
    ///
    /// Returns whether the new stimulus was retained. An accepted stimulus may
    /// have evicted one or more older entries.
    pub fn enqueue(&mut self, npc: ObjId, stim: Stimulus, now_ms: i64) -> bool {
        let rt = self.runtime_mut(npc);
        if rt.in_flight && stim.is_low_value() {
            rt.dropped_stimuli = rt.dropped_stimuli.saturating_add(1);
            return false;
        }
        let incoming_bytes = stimulus_bytes(&stim);
        if incoming_bytes > MAX_PENDING_STIMULUS_BYTES {
            rt.dropped_stimuli = rt.dropped_stimuli.saturating_add(1);
            return false;
        }
        let incoming_priority = stimulus_priority(&stim);
        while rt.mailbox.len() >= MAX_PENDING_STIMULI
            || rt.mailbox_bytes.saturating_add(incoming_bytes) > MAX_PENDING_STIMULUS_BYTES
        {
            let Some(index) = rt
                .mailbox
                .iter()
                .position(|queued| stimulus_priority(queued) <= incoming_priority)
            else {
                rt.dropped_stimuli = rt.dropped_stimuli.saturating_add(1);
                return false;
            };
            let removed = rt.mailbox.remove(index);
            rt.mailbox_bytes = rt.mailbox_bytes.saturating_sub(stimulus_bytes(&removed));
            rt.dropped_stimuli = rt.dropped_stimuli.saturating_add(1);
        }
        if rt.pending_since.is_none() {
            rt.pending_since = Some(now_ms);
        }
        rt.mailbox.push(stim);
        rt.mailbox_bytes = rt.mailbox_bytes.saturating_add(incoming_bytes);
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
        rt.captured_memory = None;
        rt.last_correlation = Some(correlation);
        rt.last_turn_at = Some(now_ms);
        rt.dialogue_since_summary = true;
        rt.pending_since = None;
        rt.mailbox_bytes = 0;
        std::mem::take(&mut rt.mailbox)
    }

    /// Capture only references present in the finalized prompt for this call.
    pub(crate) fn capture_memory_references(
        &mut self,
        npc: ObjId,
        correlation: u64,
        recalled: &[RecalledMemory],
    ) {
        let Some(rt) = self.runtimes.get_mut(&npc) else {
            return;
        };
        if rt.in_flight && rt.correlation == Some(correlation) {
            rt.captured_memory = MemoryReferences::from_recalled(recalled)
                .map(|references| (correlation, references));
        }
    }

    /// Read before finishing the call; idle and mismatched completions have no
    /// reference authority even when their ordinary actions may be applied.
    pub(crate) fn memory_references_for(&self, npc: ObjId, correlation: u64) -> MemoryReferences {
        self.runtime(npc)
            .filter(|rt| rt.in_flight && rt.correlation == Some(correlation))
            .and_then(|rt| rt.captured_memory.as_ref())
            .filter(|(captured, _)| *captured == correlation)
            .map(|(_, references)| references.clone())
            .unwrap_or_default()
    }

    /// Start a memory-fold (summarize) turn: retain the oldest half in working
    /// memory and capture its identities for a correlated acknowledgement.
    /// Returns the captured event texts, or nothing if another call is active.
    pub fn begin_summarize(&mut self, npc: ObjId, correlation: u64) -> Vec<String> {
        let rt = self.runtime_mut(npc);
        if rt.in_flight || rt.pending_fold.is_some() {
            return Vec::new();
        }
        let events = rt.memory.fold_candidate();
        if events.is_empty() {
            rt.summarize_pending = false;
            return Vec::new();
        }
        rt.in_flight = true;
        rt.correlation = Some(correlation);
        rt.captured_memory = None;
        rt.last_correlation = Some(correlation);
        rt.summarize_pending = false;
        rt.dialogue_since_summary = false;
        let texts = events.iter().map(MemoryEvent::render).collect();
        rt.pending_fold = Some(PendingFold {
            correlation,
            events,
        });
        texts
    }

    /// Finish only the captured fold identified by `correlation`. A successful
    /// parse acknowledges exactly those events; any failure retains them and
    /// backs off retries. Unknown, stale and duplicate results change nothing.
    pub fn complete_summarize(
        &mut self,
        npc: ObjId,
        correlation: u64,
        now_ms: i64,
        success: bool,
    ) -> bool {
        let Some(rt) = self.runtimes.get_mut(&npc) else {
            return false;
        };
        if rt
            .pending_fold
            .as_ref()
            .is_none_or(|fold| fold.correlation != correlation)
        {
            return false;
        }
        let Some(fold) = rt.pending_fold.take() else {
            return false;
        };
        if success {
            rt.memory.acknowledge_fold(&fold.events);
            rt.summarize_failures = 0;
            rt.summarize_retry_at = None;
            rt.summarize_pending = rt.memory.is_overflowing();
        } else {
            rt.summarize_failures = rt.summarize_failures.saturating_add(1).min(5);
            let delay = (SUMMARY_RETRY_INITIAL_MS
                << u32::from(rt.summarize_failures.saturating_sub(1)))
            .min(SUMMARY_RETRY_MAX_MS);
            rt.summarize_retry_at = Some(now_ms.saturating_add(delay));
            rt.summarize_pending = true;
        }
        // The generic completion seam may already have cleared this call.
        // Never clear an ordinary turn that began afterwards.
        if rt.correlation == Some(correlation) {
            rt.in_flight = false;
            rt.correlation = None;
        }
        if !rt.mailbox.is_empty() && rt.pending_since.is_none() {
            rt.pending_since = Some(now_ms);
        }
        true
    }

    /// Finish a turn: clear the in-flight flag (cooldown was anchored at
    /// [`NpcHostState::begin_turn`]). If new stimuli queued during the call and
    /// the mailbox is non-empty, restart the window so the next turn is
    /// scheduled. A captured fold survives this generic completion; only
    /// [`NpcHostState::complete_summarize`] can acknowledge or reject its batch.
    pub fn finish_turn(&mut self, npc: ObjId, now_ms: i64) {
        if let Some(rt) = self.runtimes.get_mut(&npc) {
            rt.in_flight = false;
            rt.correlation = None;
            rt.captured_memory = None;
            if !rt.mailbox.is_empty() && rt.pending_since.is_none() {
                rt.pending_since = Some(now_ms);
            }
        }
    }

    /// Decide whether a completion for `correlation` may be applied, clearing
    /// the in-flight turn when it is that turn's. While a turn is in flight
    /// only its own correlation is accepted: a duplicate or delayed completion
    /// from an older turn must neither clear a newer turn nor make its stale
    /// actions eligible for application. An idle NPC — including one that has
    /// never turned — accepts a completion it has not seen before (the typed
    /// completion is the world task's re-entry seam for a finished turn, and
    /// a finished turn may reach an NPC with nothing in flight); anything at
    /// or below its `last_correlation` watermark is a replay and is dropped.
    pub fn finish_turn_if_correlation(
        &mut self,
        npc: ObjId,
        correlation: u64,
        now_ms: i64,
    ) -> bool {
        match self.runtimes.get(&npc) {
            Some(rt) if rt.in_flight => {
                if rt.correlation != Some(correlation) {
                    return false;
                }
                self.finish_turn(npc, now_ms);
                true
            }
            Some(rt) if rt.last_correlation.is_some_and(|last| correlation <= last) => false,
            _ => {
                self.runtime_mut(npc).last_correlation = Some(correlation);
                true
            }
        }
    }

    /// Drop an NPC's pending mailbox and reset its window (no players, or the
    /// NPC is disabled — no wakes, no tokens).
    pub fn drop_mailbox(&mut self, npc: ObjId) {
        if let Some(rt) = self.runtimes.get_mut(&npc) {
            rt.mailbox.clear();
            rt.mailbox_bytes = 0;
            rt.pending_since = None;
        }
    }
}

/// A `Duration` as milliseconds (clamped).
fn dur_ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

fn stimulus_priority(stim: &Stimulus) -> u8 {
    if stim.is_direct() {
        2
    } else if stim.is_low_value() {
        0
    } else {
        1
    }
}

fn stimulus_bytes(stim: &Stimulus) -> usize {
    serde_json::to_vec(stim).map_or(usize::MAX, |encoded| encoded.len())
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
    fn memory_references_belong_only_to_the_captured_active_turn() {
        let mut h = host();
        let npc = ObjId(9);
        let recalled = vec![RecalledMemory {
            subject: "Alice".into(),
            notes: vec!["experience #2; belief #3".into()],
            evidence_ids: vec![2],
            belief_ids: vec![3],
        }];
        // Idle completions may carry legacy actions, but no source authority.
        h.capture_memory_references(npc, 1, &recalled);
        assert!(h.memory_references_for(npc, 1).allows(None, None));
        assert!(!h.memory_references_for(npc, 1).allows(Some(2), None));
        h.begin_turn(npc, 0, 1);
        h.capture_memory_references(npc, 1, &recalled);
        assert!(h.memory_references_for(npc, 1).allows(Some(2), Some(3)));
        assert!(!h.memory_references_for(npc, 1).allows(Some(4), Some(3)));
        assert!(!h.memory_references_for(npc, 1).allows(Some(2), Some(4)));
        assert!(!h.memory_references_for(npc, 2).allows(Some(2), None));
        let captured = h.memory_references_for(npc, 1);
        assert!(h.finish_turn_if_correlation(npc, 1, 10));
        assert!(captured.allows(Some(2), Some(3)));
        assert!(!h.memory_references_for(npc, 1).allows(Some(2), None));
        h.begin_turn(npc, 20, 2);
        assert!(!h.memory_references_for(npc, 2).allows(Some(2), None));
        h.capture_memory_references(npc, 1, &recalled);
        assert!(!h.memory_references_for(npc, 2).allows(Some(2), None));
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

    #[test]
    fn mailbox_is_bounded_and_prefers_new_direct_speech() {
        let mut h = host();
        let npc = ObjId(9);
        for n in 0..=MAX_PENDING_STIMULI {
            assert!(h.enqueue(npc, spoken(n as u32, &format!("line-{n}")), n as i64));
        }
        let rt = h.runtime(npc).unwrap();
        assert_eq!(rt.mailbox.len(), MAX_PENDING_STIMULI);
        assert_eq!(rt.dropped_stimuli, 1);
        assert!(!rt.mailbox.contains(&spoken(0, "line-0")));
        assert!(
            rt.mailbox
                .contains(&spoken(MAX_PENDING_STIMULI as u32, "line-64"))
        );
        assert!(rt.mailbox_bytes <= MAX_PENDING_STIMULUS_BYTES);
    }

    #[test]
    fn lower_priority_stimulus_cannot_evict_direct_speech() {
        let mut h = host();
        let npc = ObjId(9);
        for n in 0..MAX_PENDING_STIMULI {
            assert!(h.enqueue(npc, spoken(n as u32, "direct"), n as i64));
        }
        assert!(!h.enqueue(npc, Stimulus::PlayerArrived { who: ObjId(99) }, 100));
        let rt = h.runtime(npc).unwrap();
        assert_eq!(rt.mailbox.len(), MAX_PENDING_STIMULI);
        assert!(rt.mailbox.iter().all(Stimulus::is_direct));
        assert_eq!(rt.dropped_stimuli, 1);
    }

    #[test]
    fn single_oversized_stimulus_is_refused() {
        let mut h = host();
        let npc = ObjId(9);
        let huge = "x".repeat(MAX_PENDING_STIMULUS_BYTES);
        assert!(!h.enqueue(npc, spoken(2, &huge), 1000));
        let rt = h.runtime(npc).unwrap();
        assert!(rt.mailbox.is_empty());
        assert_eq!(rt.dropped_stimuli, 1);
    }

    #[test]
    fn completion_must_match_current_correlation() {
        let mut h = host();
        let npc = ObjId(9);
        h.enqueue(npc, spoken(2, "first"), 0);
        h.begin_turn(npc, 0, 10);

        assert!(!h.finish_turn_if_correlation(npc, 9, 1));
        assert!(h.runtime(npc).unwrap().in_flight);
        assert_eq!(h.runtime(npc).unwrap().correlation, Some(10));

        assert!(h.finish_turn_if_correlation(npc, 10, 1));
        assert!(!h.runtime(npc).unwrap().in_flight);
        assert!(!h.finish_turn_if_correlation(npc, 10, 2));
    }

    #[test]
    fn idle_npc_applies_an_unseen_completion_once() {
        let mut h = host();
        let npc = ObjId(9);
        // Nothing in flight and no runtime yet: a completion re-entered for
        // this NPC is applied ...
        assert!(h.finish_turn_if_correlation(npc, 1, 0));
        assert!(!h.runtime(npc).unwrap().in_flight);
        // ... exactly once — a replay, or anything older, is dropped ...
        assert!(!h.finish_turn_if_correlation(npc, 1, 1));
        assert!(!h.finish_turn_if_correlation(npc, 0, 1));
        // ... and a newer one is applied.
        assert!(h.finish_turn_if_correlation(npc, 2, 2));

        // A turn begun afterwards still accepts only its own correlation,
        // whether the stranger is newer or older ...
        h.begin_turn(npc, 3, 10);
        assert!(!h.finish_turn_if_correlation(npc, 11, 4));
        assert!(!h.finish_turn_if_correlation(npc, 2, 4));
        assert!(h.runtime(npc).unwrap().in_flight);
        assert!(h.finish_turn_if_correlation(npc, 10, 5));
        assert!(!h.runtime(npc).unwrap().in_flight);
        // ... and, once finished, refuses everything it has already seen.
        assert!(!h.finish_turn_if_correlation(npc, 10, 6));
        assert!(!h.finish_turn_if_correlation(npc, 2, 6));
    }

    fn overflowing_host(npc: ObjId) -> NpcHostState {
        let mut h = host();
        let rt = h.runtime_mut(npc);
        for n in 0..60 {
            rt.memory.push(n, format!("observed-{n}"));
        }
        rt.summarize_pending = true;
        h
    }

    #[test]
    fn failed_summary_retains_its_batch_through_generic_completion() {
        let npc = ObjId(9);
        let mut h = overflowing_host(npc);
        let submitted = h.begin_summarize(npc, 10);
        assert_eq!(submitted.len(), 30);
        let retained = h.runtime(npc).unwrap().memory.events.clone();
        assert_eq!(retained.len(), 60);

        // A parsed-invalid result follows the same generic re-entry path as a
        // valid completion. It must not consume the pending batch early.
        assert!(h.finish_turn_if_correlation(npc, 10, 100));
        assert!(h.runtime(npc).unwrap().pending_fold.is_some());
        assert!(h.complete_summarize(npc, 10, 100, false));
        let rt = h.runtime(npc).unwrap();
        assert_eq!(rt.memory.events, retained);
        assert!(rt.pending_fold.is_none());
        assert!(!rt.in_flight);
        assert!(!rt.summarize_ready(30_099));
        assert!(rt.summarize_ready(30_100));

        assert_eq!(h.begin_summarize(npc, 11), submitted);
        assert!(h.complete_summarize(npc, 11, 30_200, true));
        let rt = h.runtime(npc).unwrap();
        assert_eq!(rt.memory.events.len(), 30);
        assert_eq!(rt.summarize_failures, 0);
        assert_eq!(rt.summarize_retry_at, None);
        assert!(!rt.summarize_pending);
    }

    #[test]
    fn summary_acknowledges_captured_ids_not_later_identical_events() {
        let npc = ObjId(9);
        let mut h = host();
        for _ in 0..60 {
            h.runtime_mut(npc).memory.push(1, "same words");
        }
        h.runtime_mut(npc).summarize_pending = true;
        assert_eq!(h.begin_summarize(npc, 10).len(), 30);
        let folded = h
            .runtime(npc)
            .unwrap()
            .pending_fold
            .as_ref()
            .unwrap()
            .events
            .clone();
        for _ in 0..25 {
            h.runtime_mut(npc).memory.push(1, "same words");
        }
        let expected: Vec<_> = h
            .runtime(npc)
            .unwrap()
            .memory
            .events
            .iter()
            .filter(|event| !folded.iter().any(|old| old.sequence == event.sequence))
            .cloned()
            .collect();

        assert!(h.complete_summarize(npc, 10, 100, true));
        let rt = h.runtime(npc).unwrap();
        assert_eq!(rt.memory.events, expected);
        assert_eq!(rt.memory.events.len(), 55);
        assert!(rt.summarize_pending);
    }

    #[test]
    fn stale_or_duplicate_summary_cannot_clear_a_new_call() {
        let npc = ObjId(9);
        let mut h = overflowing_host(npc);
        assert!(!h.complete_summarize(ObjId(99), 1, 0, true));
        assert!(h.runtime(ObjId(99)).is_none());

        h.begin_summarize(npc, 10);
        assert!(h.complete_summarize(npc, 10, 100, true));
        h.enqueue(npc, spoken(2, "continue"), 101);
        h.begin_turn(npc, 102, 20);
        let retained = h.runtime(npc).unwrap().memory.events.clone();
        assert!(!h.complete_summarize(npc, 10, 103, true));
        assert!(!h.complete_summarize(npc, 21, 104, false));
        let rt = h.runtime(npc).unwrap();
        assert!(rt.in_flight);
        assert_eq!(rt.correlation, Some(20));
        assert_eq!(rt.memory.events, retained);

        h.finish_turn_if_correlation(npc, 20, 105);
        h.runtime_mut(npc).summarize_pending = true;
        h.begin_summarize(npc, 30);
        assert!(!h.complete_summarize(npc, 10, 106, true));
        assert_eq!(h.runtime(npc).unwrap().correlation, Some(30));
        assert!(h.runtime(npc).unwrap().pending_fold.is_some());
        assert!(h.complete_summarize(npc, 30, 107, false));
        assert!(!h.complete_summarize(npc, 30, 999, false));
        assert_eq!(h.runtime(npc).unwrap().summarize_retry_at, Some(30_107));
    }

    #[test]
    fn a_second_summary_start_cannot_replace_the_pending_batch() {
        let npc = ObjId(9);
        let mut h = overflowing_host(npc);
        h.begin_summarize(npc, 10);
        assert!(h.begin_summarize(npc, 11).is_empty());
        h.finish_turn(npc, 100);
        assert!(h.begin_summarize(npc, 12).is_empty());
        assert_eq!(
            h.runtime(npc)
                .unwrap()
                .pending_fold
                .as_ref()
                .unwrap()
                .correlation,
            10
        );
        assert!(h.complete_summarize(npc, 10, 101, true));
    }

    #[test]
    fn summary_retries_back_off_and_stop_at_five_minutes() {
        let npc = ObjId(9);
        let mut h = overflowing_host(npc);
        let mut now = 100;
        for (index, delay) in [30_000, 60_000, 120_000, 240_000, 300_000, 300_000]
            .into_iter()
            .enumerate()
        {
            assert!(h.runtime(npc).unwrap().summarize_ready(now));
            let correlation = index as u64 + 1;
            assert_eq!(h.begin_summarize(npc, correlation).len(), 30);
            assert!(h.complete_summarize(npc, correlation, now, false));
            let rt = h.runtime(npc).unwrap();
            assert_eq!(rt.memory.events.len(), 60);
            assert_eq!(rt.summarize_retry_at, Some(now + delay));
            assert!(!rt.summarize_ready(now + delay - 1));
            assert!(rt.summarize_ready(now + delay));
            now += delay;
        }
        assert_eq!(h.runtime(npc).unwrap().summarize_failures, 5);
        h.begin_summarize(npc, 7);
        assert!(h.complete_summarize(npc, 7, i64::MAX - 1, false));
        assert_eq!(h.runtime(npc).unwrap().summarize_retry_at, Some(i64::MAX));
    }

    #[test]
    fn continuous_chatter_allows_one_dialogue_between_eligible_folds() {
        let npc = ObjId(9);
        let mut h = overflowing_host(npc);
        h.enqueue(npc, spoken(2, "first"), 0);
        assert!(!h.runtime(npc).unwrap().fold_due(0));
        h.begin_turn(npc, 1, 1);
        h.enqueue(npc, spoken(2, "second"), 2);
        h.finish_turn(npc, 3);
        assert!(h.runtime(npc).unwrap().fold_due(3));

        h.begin_summarize(npc, 2);
        assert!(!h.runtime(npc).unwrap().fold_due(4));
        // Enough new events arrive to leave another fold due, even after this
        // captured batch is acknowledged and the hard cap evicts some old lines.
        for n in 0..51 {
            h.runtime_mut(npc).memory.push(10 + n, format!("new-{n}"));
        }
        assert!(h.complete_summarize(npc, 2, 5, true));
        assert!(h.runtime(npc).unwrap().summarize_ready(5));
        assert!(!h.runtime(npc).unwrap().fold_due(5));
        assert_eq!(h.begin_turn(npc, 6, 3), vec![spoken(2, "second")]);
        h.enqueue(npc, spoken(2, "third"), 7);
        h.finish_turn(npc, 8);
        assert!(h.runtime(npc).unwrap().fold_due(8));

        // With no dialogue to serve, a due fold need not wait for a conversation.
        h.drop_mailbox(npc);
        h.runtime_mut(npc).dialogue_since_summary = false;
        assert!(h.runtime(npc).unwrap().fold_due(8));
    }

    #[test]
    fn conversation_focus_is_room_scoped_short_lived_and_departure_sensitive() {
        let mut rt = NpcRuntime::default();
        let alice = ObjId(1);
        let bob = ObjId(2);
        let room = ObjId(10);
        assert_eq!(rt.focus_for(room, 0), None);
        rt.note_conversation(alice, room, 1_000);
        assert_eq!(rt.focus_for(room, 1_000), Some(alice));
        assert_eq!(rt.focus_for(room, 120_999), Some(alice));
        assert_eq!(rt.focus_for(room, 121_000), None);
        assert_eq!(rt.focus_for(ObjId(11), 2_000), None);
        assert_eq!(rt.focus_for(room, 999), None);
        rt.clear_conversation(bob);
        assert_eq!(rt.focus_for(room, 2_000), Some(alice));
        rt.note_conversation(bob, room, 2_000);
        rt.clear_conversation(alice);
        assert_eq!(rt.focus_for(room, 2_001), Some(bob));
        rt.clear_conversation(bob);
        assert_eq!(rt.focus_for(room, 2_002), None);
        rt.note_conversation(alice, room, i64::MIN);
        assert_eq!(rt.focus_for(room, i64::MAX), None);
    }
}
