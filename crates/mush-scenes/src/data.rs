//! The durable scene data model (design: `docs/scenes_factions_knowledge.md`
//! §1) and its persistence keys.
//!
//! A **scene** is an explicit, durable envelope around play — the opposite of
//! the core [`Scene`](mush_core::Scene), which is a transient per-boot runtime
//! handle for the tactics seam. A durable scene has a stable id that never
//! restarts, survives reboots (rehydrating as
//! [`Paused`](SceneStatus::Paused)`(`[`ServerRestart`](PauseReason::ServerRestart)`)`),
//! and records who is in it, whose turn it is, and its own fictional clock.
//!
//! Cut 1 ships the **timestop**: promotion of a room into an exclusive,
//! turn-based scene. Everything here is ruleset-agnostic; ruleset initiative
//! and mechanical-action routing layer on later through the same record.
//!
//! Live state is one [`ScenesState`] in a module state slot; every mutation
//! mirrors write-through to `mod:scenes:*` documents (`scene:{uid}` per record,
//! a `meta` doc for the id allocator).

use std::collections::BTreeSet;

use mush_core::{ClockRef, ClockStamp, Deferred, Ledger, ModuleOp, ModuleWrite, ObjId, Outbox};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::collections::BTreeMap;

/// The scenes persistence namespace (`mod:scenes:*`).
pub const NS: &str = "scenes";

/// The kv key of the scenes meta document (the id allocator resume point).
pub const META_KEY: &str = "meta";

/// The scene kind string for a timestop.
pub const KIND_TIMESTOP: &str = "timestop";

/// Default IC seconds per turn-based round (a WoD combat turn is ~3 IC
/// seconds; rulesets can override per scene later).
pub const DEFAULT_ROUND_SECONDS: u32 = 3;

// ---- E6b: B6 — the conferred sealer, the outcome, and the IC floor --------

/// The cap on a sealed outcome's chronicle note (08f §9).
pub const MAX_OUTCOME_NOTE_BYTES: usize = 240;

/// The cap on a sealed outcome's row name — the same 64 bytes the economy
/// enforces on an outcome row (`mush_economy::MAX_OUTCOME_NAME_BYTES`), so a
/// name that could never match a contract row cannot be sealed either.
pub const MAX_OUTCOME_NAME_BYTES: usize = 64;

/// How far ahead of the world's own IC clock a scene's floor may be raised
/// (E6b verify fold, R4): one IC year.
///
/// The raise is monotonic and has no inverse, so a typo is permanent — and the
/// floor's whole job is to catch a frozen scene clock **up** to the world, which
/// gives it no reason to run ahead of it.
pub const MAX_IC_FLOOR_LEAD_SECS: i64 = 365 * 86_400;

/// The cap on how many subjects one sealed outcome names. The economy's
/// party-intersection gate walks them, and canon stores them, so the set is
/// bounded at the producer.
pub const MAX_OUTCOME_SUBJECTS: usize = 32;

/// A sealed scene outcome — written **once**, terminal, never edited (08f §4.1).
///
/// `subjects` is named **at seal time**, never copied from `participants`
/// (frozen at scene creation with no add path), because a runner who joined
/// mid-scene would otherwise silently fail the economy's party-intersection
/// gate. `note` is chronicle text and carries no authority whatever.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SceneOutcomeWire")]
pub struct SceneOutcome {
    /// The outcome row this seal selects ("success" / "partial" / "failure").
    pub name: SmolStr,
    /// Who the outcome is about, named at seal time.
    pub subjects: Vec<ObjId>,
    /// Chronicle text (≤ [`MAX_OUTCOME_NOTE_BYTES`]); never authority.
    pub note: String,
    /// Who sealed it.
    pub sealed_by: ObjId,
    /// When (wall clock, unix seconds).
    pub sealed_at: i64,
}

impl SceneOutcome {
    /// Construct a sealed outcome, re-running every invariant. `None` when the
    /// name is empty or over-long, the note is over-long, or the subject list is
    /// empty or over-long.
    #[must_use]
    pub fn new(
        name: SmolStr,
        subjects: Vec<ObjId>,
        note: String,
        sealed_by: ObjId,
        sealed_at: i64,
    ) -> Option<SceneOutcome> {
        if name.trim().is_empty() || name.len() > MAX_OUTCOME_NAME_BYTES {
            return None;
        }
        if note.len() > MAX_OUTCOME_NOTE_BYTES {
            return None;
        }
        if subjects.is_empty() || subjects.len() > MAX_OUTCOME_SUBJECTS {
            return None;
        }
        let mut seen = BTreeSet::new();
        if !subjects.iter().all(|s| seen.insert(*s)) {
            return None;
        }
        Some(SceneOutcome {
            name,
            subjects,
            note,
            sealed_by,
            sealed_at,
        })
    }
}

/// The persisted wire of [`SceneOutcome`] — every stored document re-runs the
/// constructor's invariants on load (the E5a fold rule).
#[derive(Deserialize)]
struct SceneOutcomeWire {
    name: SmolStr,
    #[serde(default)]
    subjects: Vec<ObjId>,
    #[serde(default)]
    note: String,
    sealed_by: ObjId,
    sealed_at: i64,
}

impl TryFrom<SceneOutcomeWire> for SceneOutcome {
    type Error = &'static str;

    fn try_from(w: SceneOutcomeWire) -> Result<SceneOutcome, &'static str> {
        SceneOutcome::new(w.name, w.subjects, w.note, w.sealed_by, w.sealed_at)
            .ok_or("scene outcome fails its bounds")
    }
}

const fn initial_scene_clock_generation() -> u64 {
    1
}

/// A durable scene id: a monotonic per-world counter, **never reused** (the
/// allocator resume point persists in the meta doc, exactly like faction ids).
/// Unlike the core [`SceneId`](mush_core::SceneId) it does not restart at 0 on
/// boot, so a rehydrated scene keeps its identity and its persisted key.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SceneUid(pub u64);

impl std::fmt::Display for SceneUid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Why a scene is paused.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    /// A facilitator (or participant) paused turn advancement.
    Facilitator,
    /// The server restarted while the scene was live — it rehydrates paused
    /// and a facilitator must explicitly resume it (turns never auto-advance
    /// across a restart).
    ServerRestart,
}

/// A scene's lifecycle status.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneStatus {
    /// Membership is being assembled; not yet advancing turns.
    Preparing,
    /// Live turn-based (or freeform) play.
    Active,
    /// Turn advancement frozen; safety/inspection still work.
    Paused,
    /// Ended; the record is retained as immutable history.
    Closed,
}

impl SceneStatus {
    /// Whether the scene still holds its members (movement freeze, NPC
    /// suppression, exclusivity apply): every status except [`Closed`].
    #[must_use]
    pub fn holds_members(self) -> bool {
        !matches!(self, SceneStatus::Closed)
    }

    /// The staff-facing label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            SceneStatus::Preparing => "preparing",
            SceneStatus::Active => "active",
            SceneStatus::Paused => "paused",
            SceneStatus::Closed => "closed",
        }
    }
}

/// How a scene clocks its participants.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneMode {
    /// Recorded but free-flowing; no turn order.
    Freeform,
    /// Ordered turns and rounds (the timestop).
    TurnBased,
}

/// One durable scene — the persisted `scene:{uid}` document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneRecord {
    /// Stable durable id.
    pub id: SceneUid,
    /// Human title ("Timestop in the Tap Room").
    pub title: String,
    /// Scene kind (`"timestop"`).
    pub kind: SmolStr,
    /// Clocking mode.
    pub mode: SceneMode,
    /// Lifecycle status.
    pub status: SceneStatus,
    /// If paused, why.
    #[serde(default)]
    pub pause_reason: Option<PauseReason>,
    /// Exclusive scenes freeze their members: at most one exclusive scene per
    /// object, movement blocked, NPCs suppressed.
    pub exclusive: bool,
    /// The room the scene covers (cut 1: a single anchor).
    pub anchor: ObjId,
    /// Participants entitled to act, in **turn order**.
    pub participants: Vec<ObjId>,
    /// Observers who receive scene output but do not act.
    #[serde(default)]
    pub observers: Vec<ObjId>,
    /// Who may drive the scene (advance turns, pause, end) besides staff.
    pub facilitator: ObjId,
    /// Who created it.
    pub created_by: ObjId,
    /// Wall-clock creation time (unix seconds).
    pub created_at: i64,
    /// Exact versioned world-IC stamp captured at scene start — the floor of
    /// scene-local fictional time. A later world rate change cannot
    /// reinterpret where this scene began.
    pub ic_started_at: ClockStamp,
    /// Generation of the scene-local turn clock. Scene ids are never reused;
    /// generation one therefore lasts for the record's lifetime unless a
    /// future explicit re-clock operation opens a successor generation.
    #[serde(default = "initial_scene_clock_generation")]
    pub clock_generation: u64,
    /// Monotonic count of successful turn advances, starting at zero.
    #[serde(default)]
    pub turn: i64,
    /// IC seconds per round.
    pub round_seconds: u32,
    /// Current round, 1-based (0 while Preparing).
    pub round: u32,
    /// Index into `participants` of whose turn it is.
    pub turn_index: usize,
    /// Wall-clock end time, once closed.
    #[serde(default)]
    pub ended_at: Option<i64>,
    /// **The conferred sealer** (E6b, 08f §4.1, D-E6-7). `None` means this scene
    /// **cannot be sealed** — fail-closed exactly like the empty canon-outcome
    /// kind table it replaces. Conferred by staff with `@scene/facilitate`;
    /// never self-granted, because any connected player becomes a scene's
    /// `facilitator` merely by starting it.
    #[serde(default)]
    pub sealer: Option<ObjId>,
    /// **The run this scene's seal may name** (E6b verify fold, M2/R12), conferred
    /// with the sealer.
    ///
    /// `+scene/outcome`'s `run=` was free input, so a sealer conferred on *any*
    /// scene could name *any* contract id, pick that contract's own parties as
    /// subjects, and settle a run they had nothing to do with — the economy's three
    /// gates (kind, exact source, party intersection) all pass. The id is opaque
    /// here: `mush-scenes` has no economy edge and never interprets it. `None` means
    /// this scene's seal names no run at all and is chronicle only.
    #[serde(default)]
    pub seal_run: Option<u64>,
    /// The sealed outcome — written **once**, terminal, never edited (E6b).
    #[serde(default)]
    pub outcome: Option<SceneOutcome>,
    /// **The monotonic IC time floor** (doc 10 §15/6A; E6b owns the floor, cut 6
    /// owns the raise). A long timestop's scene clock advances only
    /// `round_seconds × (rounds − 1)`, so without a floor a seal would chronicle
    /// a settlement before events it followed. [`SceneRecord::scene_ic_time`]
    /// clamps to this; a **lower** value is refused, and raising it never writes
    /// [`SceneRecord::clock_generation`].
    #[serde(default)]
    pub ic_floor: i64,
    /// **The archive bound** (H1, D-H-2): the ledger's open chunk index for
    /// this scene's log, stamped when the scene closes (and again if a later
    /// seal rolls the chunk). After a restart a closed scene's log tail is no
    /// longer in RAM, so this is how `+timestop/log/archive` knows which
    /// `log:{id}:{n}` documents exist — `0..=log_chunks`, the last of them
    /// possibly unsealed. A record from before H1 carries `None` and is paged
    /// by probing forward from chunk 0.
    #[serde(default)]
    pub log_chunks: Option<u64>,
}

impl SceneRecord {
    /// The whole membership (participants ∪ observers), for freeze/release.
    pub fn members(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.participants
            .iter()
            .copied()
            .chain(self.observers.iter().copied())
    }

    /// Whose turn it is now (the participant at `turn_index`), if any.
    #[must_use]
    pub fn current_actor(&self) -> Option<ObjId> {
        self.participants.get(self.turn_index).copied()
    }

    /// The scene-local IC timestamp (unix seconds): the frozen start plus the
    /// elapsed rounds, **clamped up to [`SceneRecord::ic_floor`]** (E6b). A
    /// timestop played across real days occupies only seconds-to-minutes of IC
    /// time — the fiction is that the fight was quick — so without the floor a
    /// record stamped from this clock could sit before events it followed.
    #[must_use]
    pub fn scene_ic_time(&self) -> i64 {
        let rounds_elapsed = i64::from(self.round.saturating_sub(1));
        let elapsed = self
            .ic_started_at
            .at
            .saturating_add(rounds_elapsed.saturating_mul(i64::from(self.round_seconds)));
        elapsed.max(self.ic_floor)
    }

    /// Checked form used by boot validation and mechanical clock adapters. The
    /// floor is a `max`, so it can never overflow what the sum already proved.
    #[must_use]
    pub fn checked_scene_ic_time(&self) -> Option<i64> {
        let rounds_elapsed = i64::from(self.round.saturating_sub(1));
        rounds_elapsed
            .checked_mul(i64::from(self.round_seconds))
            .and_then(|elapsed| self.ic_started_at.at.checked_add(elapsed))
            .map(|at| at.max(self.ic_floor))
    }

    /// Exact typed stamp for the current scene turn.
    #[must_use]
    pub fn scene_turn_stamp(&self) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::SceneTurn {
                scene: self.id.0,
                generation: self.clock_generation,
            },
            at: self.turn,
        }
    }

    /// Validate the durable clock and cursor shape before it becomes live.
    pub fn validate_clock(&self) -> Result<(), &'static str> {
        if self.clock_generation == 0 {
            return Err("scene clock generation zero is reserved");
        }
        if self.turn < 0 {
            return Err("scene turn cannot be negative");
        }
        if !matches!(
            self.ic_started_at.clock,
            ClockRef::WorldIc { generation } if generation != 0
        ) {
            return Err("scene IC floor must name a nonzero world-IC generation");
        }
        if self.round_seconds == 0 {
            return Err("scene round duration must be positive");
        }
        if (self.participants.is_empty() && self.turn_index != 0)
            || (!self.participants.is_empty() && self.turn_index >= self.participants.len())
        {
            return Err("scene turn index is outside its participant order");
        }
        if self.checked_scene_ic_time().is_none() {
            return Err("scene IC clock arithmetic overflowed");
        }
        // ---- E6b: B6 — the sealer, the outcome and the floor ----
        if self.ic_floor < 0 {
            return Err("scene IC floor cannot be negative");
        }
        if let Some(outcome) = &self.outcome {
            // Fail closed: an outcome can only exist on a scene somebody was
            // conferred the authority to seal. A document claiming one without a
            // sealer is a forged seal, not a recoverable record.
            if self.sealer.is_none() {
                return Err("a sealed outcome needs a conferred sealer");
            }
            if outcome.subjects.len() > MAX_OUTCOME_SUBJECTS {
                return Err("a sealed outcome names too many subjects");
            }
            if outcome.note.len() > MAX_OUTCOME_NOTE_BYTES {
                return Err("a sealed outcome's note is too long");
            }
            if outcome.name.trim().is_empty() || outcome.name.len() > MAX_OUTCOME_NAME_BYTES {
                return Err("a sealed outcome's row name is empty or too long");
            }
        }
        Ok(())
    }

    /// Raise the IC floor to `at` (E6b, B6). **Monotonic**: a value at or below
    /// the current floor is refused, and the raise never touches
    /// [`SceneRecord::clock_generation`] — the generation lasts the record's
    /// lifetime (08i §9 answer 9).
    pub fn raise_ic_floor(&mut self, at: i64) -> Result<i64, &'static str> {
        if at < 0 {
            return Err("scene IC floor cannot be negative");
        }
        if at <= self.ic_floor {
            return Err("the scene IC floor only moves forward");
        }
        self.ic_floor = at;
        Ok(at)
    }

    /// Remove `obj` from the scene (participant or observer), keeping the turn
    /// cursor sane: an earlier participant shifts the cursor down, and the
    /// cursor never dangles past the end. Returns whether `obj` was a member.
    pub fn remove_member(&mut self, obj: ObjId) -> bool {
        let mut found = false;
        if let Some(pos) = self.participants.iter().position(|&p| p == obj) {
            self.participants.remove(pos);
            found = true;
            if pos < self.turn_index {
                self.turn_index -= 1;
            }
            if self.turn_index >= self.participants.len() {
                self.turn_index = 0;
            }
        }
        let before = self.observers.len();
        self.observers.retain(|&o| o != obj);
        found |= self.observers.len() != before;
        found
    }

    /// Advance to the next participant's turn; wrapping past the last starts a
    /// new round. No-op for an empty participant list. Returns the new
    /// `(round, current_actor)`.
    pub fn advance_turn(&mut self) -> Result<(u32, Option<ObjId>), SceneAdvanceError> {
        if self.participants.is_empty() {
            return Ok((self.round, None));
        }
        let turn = self
            .turn
            .checked_add(1)
            .ok_or(SceneAdvanceError::TurnExhausted)?;
        let next_index = self
            .turn_index
            .checked_add(1)
            .ok_or(SceneAdvanceError::TurnIndexExhausted)?;
        let (turn_index, round) = if next_index >= self.participants.len() {
            (
                0,
                self.round
                    .checked_add(1)
                    .ok_or(SceneAdvanceError::RoundExhausted)?,
            )
        } else {
            (next_index, self.round)
        };
        self.turn = turn;
        self.turn_index = turn_index;
        self.round = round;
        Ok((self.round, self.current_actor()))
    }
}

/// A scene turn counter reached its fixed durable representation limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SceneAdvanceError {
    TurnExhausted,
    TurnIndexExhausted,
    RoundExhausted,
}

impl std::fmt::Display for SceneAdvanceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TurnExhausted => formatter.write_str("scene turn counter is exhausted"),
            Self::TurnIndexExhausted => formatter.write_str("scene turn index is exhausted"),
            Self::RoundExhausted => formatter.write_str("scene round counter is exhausted"),
        }
    }
}

impl std::error::Error for SceneAdvanceError {}

/// What kind of thing one [`SceneEvent`] records. Cut 1 records the scene's
/// **structural** timeline (lifecycle + turns) plus **IC narrative** (speech
/// and poses) and room join/leave — the deterministic, no-LLM transcript. Dice
/// rolls and other mechanical outcomes join once commands route through the
/// turn.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneEventKind {
    /// The scene began.
    Started,
    /// Turn advanced (possibly a new round).
    TurnAdvanced,
    /// Turn advancement was paused.
    Paused,
    /// The scene resumed.
    Resumed,
    /// A participant was released from the freeze without ending the scene.
    Released,
    /// The scene ended.
    Ended,
    /// A participant spoke / posed (IC narrative).
    Said,
    /// A mechanical action resolved on a participant's turn (a dice roll).
    Rolled,
    /// Someone entered the scene's anchor room during play.
    Joined,
    /// Someone left the scene's anchor room during play.
    Left,
    /// The scene's outcome was **sealed** (E6b): terminal, written once.
    Sealed,
}

/// One immutable entry in a scene's event log — a **semantic** record of what
/// happened, never rendered output. The witness set is captured **at event
/// time** (it is the audience the event was delivered to) and never
/// recalculated, so a later reader sees who actually perceived it. The
/// ledger assigns the sequence number; this is the payload it stores.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneEvent {
    /// The kind of event.
    pub kind: SceneEventKind,
    /// Who acted (speaker, mover, facilitator), if applicable.
    #[serde(default)]
    pub actor: Option<ObjId>,
    /// The scene-local IC timestamp when it occurred (`scene_ic_time`).
    pub ic_time: i64,
    /// Wall-clock time (unix seconds).
    pub at: i64,
    /// Who perceived it — captured at event time, never recomputed.
    #[serde(default)]
    pub witnesses: Vec<ObjId>,
    /// Kind-specific detail (`{"text": ..., "narr": "say"}`, `{"round": N}`, …).
    #[serde(default)]
    pub detail: serde_json::Value,
}

impl SceneEvent {
    /// Build an event stamped with the scene's current fictional time and its
    /// present membership as the witness set.
    #[must_use]
    pub fn new(
        s: &SceneRecord,
        at: i64,
        kind: SceneEventKind,
        actor: Option<ObjId>,
        detail: serde_json::Value,
    ) -> SceneEvent {
        SceneEvent {
            kind,
            actor,
            ic_time: s.scene_ic_time(),
            at,
            witnesses: s.members().collect(),
            detail,
        }
    }
}

/// The composition-frozen [`crate::SealerGate`], held in module state (E6b
/// independent verify fold, M4).
///
/// A newtype so [`ScenesState`] keeps its derives: a boxed closure is `Clone` but
/// neither `Debug` nor `PartialEq`, and both of those are about the *state*, not
/// about the wiring — two states with the same scenes are the same state whichever
/// composition built them.
#[derive(Clone, Default)]
pub struct SealerGateSlot(pub Option<crate::SealerGate>);

impl std::fmt::Debug for SealerGateSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "SealerGateSlot(wired)"
        } else {
            "SealerGateSlot(none)"
        })
    }
}

impl PartialEq for SealerGateSlot {
    fn eq(&self, other: &SealerGateSlot) -> bool {
        self.0.is_some() == other.0.is_some()
    }
}

/// One archive page a reader is waiting on (H1): the correlation
/// `Ctx::read_store` returned and what the page was for, so the completion
/// renders the right view for the right scene. Transient world RAM — a
/// restart drops it, and a completion nobody is waiting on is dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingArchiveRead {
    /// The correlation the completion must carry.
    pub correlation: u64,
    /// When it was requested (world seconds). A request older than
    /// `STORE_READ_STALE_SECS` is superseded by the next one (verify fold B1).
    pub requested_at: i64,
    /// The scene whose log is being paged.
    pub scene: SceneUid,
    /// The requested page (0 is the newest).
    pub page: u64,
    /// `true` for `+recap/archive` (the witnessed-narrative view), `false`
    /// for `+timestop/log/archive` (the mechanical timeline).
    pub recap: bool,
    /// The chunk range the page covers, inclusive.
    pub chunk_lo: u64,
    /// The chunk range the page covers, inclusive.
    pub chunk_hi: u64,
}

/// The most archive reads one actor may have in flight (H1, D-H-3): a second
/// request while one is pending is refused with a line, never queued.
pub const MAX_PENDING_STORE_READS_PER_ACTOR: usize = 1;

/// The live scenes state, held in a module state slot on the `World`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScenesState {
    /// In-flight archive reads, one per requesting actor (H1). Transient.
    pub pending_reads: std::collections::HashMap<ObjId, PendingArchiveRead>,
    /// The wiring-frozen sealer gate (E6b independent verify fold, M4). `None` on
    /// any composition that wires no economy, which is the pre-fold behaviour.
    pub sealer_gate: SealerGateSlot,
    /// Every scene by durable id (Closed ones retained as history).
    pub scenes: BTreeMap<SceneUid, SceneRecord>,
    /// The durable-id allocator resume point (persisted in the meta doc).
    pub next_scene_uid: u64,
    /// The per-scene immutable event logs, keyed by the scene's durable id
    /// (stringified). Chunked, append-only, idempotent — the ledger substrate.
    /// Only each log's open tail lives here; sealed history is on disk.
    pub events: Ledger,
}

impl ScenesState {
    /// The scene an object is an active (non-closed) **member** of
    /// (participant or observer), if any — cut 1 enforces at most one such per
    /// object. Matches `members()`, not just participants, so a frozen observer
    /// is never invisible to the lookups that gate `+scene` and the veto
    /// message.
    #[must_use]
    pub fn active_scene_of(&self, obj: ObjId) -> Option<&SceneRecord> {
        self.scenes
            .values()
            .find(|s| s.status.holds_members() && s.members().any(|m| m == obj))
    }

    /// The active (non-closed) scene anchored to `room`, if any.
    #[must_use]
    pub fn active_scene_at(&self, room: ObjId) -> Option<&SceneRecord> {
        self.scenes
            .values()
            .find(|s| s.status.holds_members() && s.anchor == room)
    }

    /// The most recent scene (any status, **including Closed**) `obj` is a
    /// member of — for READ commands, so a scene stays reviewable right after
    /// it ends. Highest durable id wins (the current or most-recently-finished
    /// scene). Its transcript is readable while its log tail is in RAM.
    #[must_use]
    pub fn latest_scene_of(&self, obj: ObjId) -> Option<&SceneRecord> {
        self.scenes
            .values()
            .rev()
            .find(|s| s.members().any(|m| m == obj))
    }

    /// The most recent scene (any status) anchored to `room` — the staff
    /// read fallback (highest id wins).
    #[must_use]
    pub fn latest_scene_at(&self, room: ObjId) -> Option<&SceneRecord> {
        self.scenes.values().rev().find(|s| s.anchor == room)
    }

    /// Allocate the next durable scene id (callers persist the meta doc).
    pub fn alloc_uid(&mut self) -> SceneUid {
        let id = SceneUid(self.next_scene_uid);
        self.next_scene_uid += 1;
        id
    }

    /// Every object that any non-closed scene currently holds — the set the
    /// driver mirrors into the core exclusive-scene index (on freeze and on
    /// boot). Only exclusive scenes contribute.
    #[must_use]
    pub fn held_objects(&self) -> BTreeSet<ObjId> {
        let mut set = BTreeSet::new();
        for s in self.scenes.values() {
            if s.exclusive && s.status.holds_members() {
                set.extend(s.members());
            }
        }
        set
    }
}

// ----------------------------------------------------------------------
// Persistence keys + op helpers
// ----------------------------------------------------------------------

/// The kv key of a scene document.
#[must_use]
pub fn scene_key(id: SceneUid) -> SmolStr {
    SmolStr::new(format!("scene:{}", id.0))
}

/// Queue one scenes write on the outbox deferred side-channel.
pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

/// Mirror a scene document (plain data; serialization cannot fail).
pub(crate) fn persist_scene(out: &mut Outbox, s: &SceneRecord) {
    let value = serde_json::to_value(s).expect("scene doc serializes");
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: scene_key(s.id),
            value,
        },
    );
}

/// Mirror the meta document (the id allocator resume point).
pub(crate) fn persist_meta(out: &mut Outbox, next_scene_uid: u64) {
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: SmolStr::new_static(META_KEY),
            value: serde_json::json!({ "next_scene_uid": next_scene_uid }),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(participants: Vec<u32>) -> SceneRecord {
        SceneRecord {
            id: SceneUid(0),
            title: "T".into(),
            kind: SmolStr::new(KIND_TIMESTOP),
            mode: SceneMode::TurnBased,
            status: SceneStatus::Active,
            pause_reason: None,
            exclusive: true,
            anchor: ObjId(10),
            participants: participants.into_iter().map(ObjId).collect(),
            observers: Vec::new(),
            facilitator: ObjId(1),
            created_by: ObjId(1),
            created_at: 1000,
            ic_started_at: ClockStamp {
                clock: ClockRef::WorldIc { generation: 1 },
                at: 500,
            },
            clock_generation: 1,
            turn: 0,
            round_seconds: DEFAULT_ROUND_SECONDS,
            round: 1,
            turn_index: 0,
            ended_at: None,
            sealer: None,
            seal_run: None,
            outcome: None,
            ic_floor: 0,
            log_chunks: None,
        }
    }

    #[test]
    fn turn_advance_wraps_and_bumps_the_round() {
        let mut s = scene(vec![2, 3, 4]);
        assert_eq!(s.current_actor(), Some(ObjId(2)));
        assert_eq!(s.advance_turn().unwrap(), (1, Some(ObjId(3))));
        assert_eq!(s.advance_turn().unwrap(), (1, Some(ObjId(4))));
        // Wrapping past the last participant starts round 2 at the first.
        assert_eq!(s.advance_turn().unwrap(), (2, Some(ObjId(2))));
        assert_eq!(s.turn_index, 0);
        assert_eq!(s.turn, 3);
    }

    #[test]
    fn scene_local_clock_tracks_rounds_not_wall_time() {
        let mut s = scene(vec![2, 3]);
        // Round 1: the frozen start.
        assert_eq!(s.scene_ic_time(), 500);
        s.advance_turn().unwrap(); // still round 1 (2 -> 3)
        assert_eq!(s.scene_ic_time(), 500);
        s.advance_turn().unwrap(); // round 2
        assert_eq!(s.scene_ic_time(), 500 + i64::from(DEFAULT_ROUND_SECONDS));
        s.advance_turn().unwrap(); // round 2 (2 -> 3)
        s.advance_turn().unwrap(); // round 3
        assert_eq!(
            s.scene_ic_time(),
            500 + 2 * i64::from(DEFAULT_ROUND_SECONDS)
        );
    }

    #[test]
    fn held_objects_are_exclusive_non_closed_members() {
        let mut st = ScenesState::default();
        let mut s = scene(vec![2, 3]);
        s.observers = vec![ObjId(4)];
        s.id = st.alloc_uid();
        st.scenes.insert(s.id, s.clone());
        assert_eq!(
            st.held_objects(),
            BTreeSet::from([ObjId(2), ObjId(3), ObjId(4)])
        );
        // A closed scene holds no one.
        let closed_id = st.alloc_uid();
        let mut c = scene(vec![9]);
        c.id = closed_id;
        c.status = SceneStatus::Closed;
        st.scenes.insert(closed_id, c);
        assert!(!st.held_objects().contains(&ObjId(9)));
        // active_scene_of finds the live one, ignores the closed one.
        assert_eq!(st.active_scene_of(ObjId(2)).map(|s| s.id), Some(s.id));
        assert_eq!(st.active_scene_of(ObjId(9)), None);
    }

    #[test]
    fn advance_on_empty_participants_is_a_noop() {
        let mut s = scene(vec![]);
        assert_eq!(s.advance_turn().unwrap(), (1, None));
        assert_eq!(s.round, 1);
    }
}
