//! The durable scene data model (design: `docs/design/scenes-factions-knowledge.md`
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

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::ops::{Deref, Index};

use mush_core::{ClockRef, ClockStamp, Deferred, Ledger, ModuleOp, ModuleWrite, ObjId, Outbox};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The scenes persistence namespace (`mod:scenes:*`).
pub const NS: &str = "scenes";

/// The kv key of the scenes meta document (the id allocator resume point).
pub const META_KEY: &str = "meta";

/// The scene kind string for a timestop.
pub const KIND_TIMESTOP: &str = "timestop";

/// Default IC seconds per turn-based round (a WoD combat turn is ~3 IC
/// seconds; rulesets can override per scene later).
pub const DEFAULT_ROUND_SECONDS: u32 = 3;

/// Maximum consent votes retained on one [`SceneRecord`].
pub const MAX_CONSENT_VOTES: usize = 128;

/// Maximum closed historical scenes retained in [`ScenesState`]. Oldest closed
/// scenes past this cap are evicted from RAM and deleted from module KV.
pub const MAX_RETAINED_CLOSED_SCENES: usize = 256;

/// Maximum timestop scenes one non-staff player may start within
/// [`SCENE_CREATE_WINDOW_SECS`].
pub const MAX_SCENES_PER_PLAYER_WINDOW: usize = 5;

/// Rolling rate-limit window (seconds) for per-player scene creation.
pub const SCENE_CREATE_WINDOW_SECS: i64 = 600;

/// Maximum idle wall time (seconds) an exclusive scene may sit with no
/// activity before the periodic tick automatically releases everyone and closes it.
pub const SCENE_IDLE_TIMEOUT_SECS: i64 = 4 * 3600;

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

/// Why [`SceneRecord::raise_ic_floor_on_arrival`] (E7 cut 6, 10b §3.6 OPEN 3)
/// refused. Every arm names a fail-closed condition E6b's floor rule (08f R4)
/// forbids; none is silently swallowed, so the arrival caller can page the
/// anomaly with the offending numbers rather than losing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrivalFloorRefusal {
    /// The arrival stamp is negative or more than [`MAX_IC_FLOOR_LEAD_SECS`]
    /// ahead of the world's own IC clock.
    AheadOfWorld {
        /// The refused stamp.
        at: i64,
        /// The world's IC clock at the raise.
        world_now: i64,
    },
    /// The scene's outcome is already sealed; its IC time is canon.
    AlreadySealed,
    /// The stamp is at or below the current floor (a floor only moves forward).
    NotForward {
        /// The refused stamp.
        at: i64,
        /// The current floor it did not clear.
        floor: i64,
    },
}

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
    /// suppression, exclusivity apply): every status except [`SceneStatus::Closed`].
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
    /// Consent answers so far (D6), newest last.
    #[serde(default)]
    pub consent: Vec<ConsentVote>,
    /// `+timestop/nudge <minutes>`: after this long without the current actor
    /// acting, they are asked to pass or hold (D6). `None` = no nudging.
    #[serde(default)]
    pub nudge_after_s: Option<u32>,
    /// When the current turn began, Unix seconds (start, `/next`, `/resume`,
    /// `/rewind`), for the nudge clock.
    #[serde(default)]
    pub turn_started_at: i64,
    /// Identifies a visit to a turn, including rewinds and nudge-policy resets.
    /// MAX is a fail-closed sentinel: no further nudges may be issued.
    #[serde(default)]
    pub turn_visit: u64,
    /// The `(round, turn_index)` already nudged, so a turn is nudged once.
    #[serde(default)]
    pub nudged: Option<(u32, usize)>,
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

    /// Raise the IC floor for a **journey arrival** (E7 cut 6, B4 / 10b §3.6
    /// OPEN 3) — the single public seam a `mush-sim` arrival calls so the whole
    /// of E6b's floor rule (08f R4) lives in one place instead of being
    /// re-derived across a crate edge:
    ///
    /// - **monotonic**: it only moves forward ([`SceneRecord::raise_ic_floor`]);
    /// - **bounded**: at most [`MAX_IC_FLOOR_LEAD_SECS`] ahead of the world's own
    ///   IC clock (`world_now`), so a pasted or overflowed arrival stamp cannot
    ///   strand every future record of this scene in the far future;
    /// - **refused after the seal**: an already-sealed outcome's `ic_time` is
    ///   canon and immutable;
    /// - and it **never touches [`SceneRecord::clock_generation`]** — the
    ///   generation lasts the record's lifetime (08i §9 answer 9), which the
    ///   `debug_assert` and the arrival case of E6b's monotonicity test pin.
    ///
    /// The check order matches `@scene/icfloor` (`mush-scenes/src/commands.rs`,
    /// `scene_icfloor`): bound, then seal, then the monotonic raise, so the two
    /// callers refuse the same input identically. A journey arrival **cannot be
    /// delayed** by a scene (Q6A/S6A): the caller commits the arrival first and
    /// this raise is the last, non-fatal step — a refusal here is a paged
    /// anomaly, never a hold on the journey.
    pub fn raise_ic_floor_on_arrival(
        &mut self,
        at: i64,
        world_now: i64,
    ) -> Result<i64, ArrivalFloorRefusal> {
        if at < 0 || at > world_now.saturating_add(MAX_IC_FLOOR_LEAD_SECS) {
            return Err(ArrivalFloorRefusal::AheadOfWorld { at, world_now });
        }
        if self.outcome.is_some() {
            return Err(ArrivalFloorRefusal::AlreadySealed);
        }
        let generation = self.clock_generation;
        let raised = self
            .raise_ic_floor(at)
            .map_err(|_| ArrivalFloorRefusal::NotForward {
                at,
                floor: self.ic_floor,
            })?;
        debug_assert_eq!(
            generation, self.clock_generation,
            "an arrival IC-floor raise must never bump the scene clock generation"
        );
        Ok(raised)
    }

    /// Invalidate any outstanding nudge without reusing a turn visit.
    pub fn invalidate_nudge(&mut self) {
        self.turn_visit = self.turn_visit.saturating_add(1);
        self.nudged = None;
    }

    /// Remove a member and keep the turn cursor within the participant list.
    pub fn remove_member(&mut self, obj: ObjId) -> bool {
        let mut found = false;
        if let Some(pos) = self.participants.iter().position(|&p| p == obj) {
            self.invalidate_nudge();
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
        self.invalidate_nudge();
        self.turn = turn;
        self.turn_index = turn_index;
        self.round = round;
        Ok((self.round, self.current_actor()))
    }

    /// Step the turn back one participant (D6 `+timestop/rewind`): to the
    /// previous index, or to the last participant of the previous round.
    /// `None` when there is nothing before the first turn of round one. The
    /// monotonic `turn` counter is not decremented (it counts advances).
    pub fn rewind_turn(&mut self) -> Option<(u32, Option<ObjId>)> {
        if self.participants.is_empty() {
            return None;
        }
        if self.turn_index > 0 {
            self.turn_index -= 1;
        } else if self.round > 1 {
            self.round -= 1;
            self.turn_index = self.participants.len() - 1;
        } else {
            return None;
        }
        self.invalidate_nudge();
        Some((self.round, self.current_actor()))
    }

    /// Record a consent vote with deduplication and bounded retention.
    ///
    /// Replaces any existing vote for the same `(about, participant, round)`
    /// (or `(about, participant)` for one-shot subjects `end`/`outcome`), and
    /// drops the oldest votes if [`MAX_CONSENT_VOTES`] would be exceeded.
    pub fn record_consent(&mut self, vote: ConsentVote) {
        let single_shot = matches!(vote.about.as_str(), CONSENT_END | CONSENT_OUTCOME);
        if let Some(existing) = self.consent.iter_mut().find(|v| {
            v.about == vote.about
                && v.participant == vote.participant
                && (single_shot || v.round == vote.round)
        }) {
            *existing = vote;
            return;
        }
        if self.consent.len() >= MAX_CONSENT_VOTES {
            let excess = self.consent.len() + 1 - MAX_CONSENT_VOTES;
            self.consent.drain(0..excess);
        }
        self.consent.push(vote);
    }

    /// The consent tally for `about`: choice → count, in a stable order (D6).
    #[must_use]
    pub fn consent_tally(&self, about: &str) -> BTreeMap<SmolStr, usize> {
        let mut tally = BTreeMap::new();
        for vote in self.consent.iter().filter(|v| v.about == about) {
            *tally.entry(vote.choice.clone()).or_insert(0) += 1;
        }
        tally
    }

    /// Every consent tally, keyed by subject (for the keyframe).
    #[must_use]
    pub fn consent_summary(&self) -> serde_json::Value {
        let mut subjects: BTreeMap<SmolStr, BTreeMap<SmolStr, usize>> = BTreeMap::new();
        for vote in &self.consent {
            *subjects
                .entry(vote.about.clone())
                .or_default()
                .entry(vote.choice.clone())
                .or_insert(0) += 1;
        }
        serde_json::to_value(subjects).unwrap_or(serde_json::Value::Null)
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
    /// A participant answered (or let expire) a consent request (interface
    /// programme D6): `detail` carries `about` (`end`, `outcome`, `rewind`,
    /// `nudge`) and `choice` (`agree`, `disagree`, `abstain`, `pass`, `hold`).
    /// Advisory — recorded, never a veto.
    Consent,
    /// The facilitator rewound the turn (D6).
    Rewound,
}

/// How long a consent request stays open before silence is recorded as an
/// abstention (interface programme D6, `CONSENT_DEADLINE_S`).
pub const CONSENT_DEADLINE_S: i64 = 900;

/// The nudge tick's period: how often idle turns are checked (D6). A nudge is
/// therefore late by at most this much.
pub const NUDGE_TICK_S: u64 = 30;

/// The longest `+timestop/nudge` interval, in minutes (a day).
pub const MAX_NUDGE_MINUTES: u32 = 1440;

/// Consent subjects (D6): what a request was about, as recorded on the vote
/// and in the `Consent` scene event.
pub const CONSENT_END: &str = "end";
pub const CONSENT_OUTCOME: &str = "outcome";
pub const CONSENT_REWIND: &str = "rewind";
pub const CONSENT_NUDGE: &str = "nudge";

/// One participant's answer to a consent request (D6), kept on the record so
/// the facilitator can see the tally and the keyframe can count it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentVote {
    /// What was asked about: `end`, `outcome`, `rewind`, `nudge`.
    pub about: SmolStr,
    /// Who answered.
    pub participant: ObjId,
    /// `agree`, `disagree`, `abstain` (silence or a decline), or for a nudge
    /// `pass` / `hold`.
    pub choice: SmolStr,
    /// The round the question was asked in.
    pub round: u32,
    /// When the answer landed, Unix seconds.
    pub at: i64,
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

/// What another module does when a scene starts or ends (S4: the atlas opens a
/// tactical window over a timestop's room and closes it with the scene).
pub type SceneHookFn = std::sync::Arc<dyn Fn(&mut mush_core::Ctx<'_>, &SceneRecord) + Send + Sync>;

/// The hooks other modules register, each at most once (plan §3.5: "the
/// `ScenesModule` on an engagement start").
#[derive(Clone, Default)]
pub struct SceneHooks {
    /// After a scene starts and its record persists.
    pub started: Option<SceneHookFn>,
    /// After a scene closes and its record persists.
    pub ended: Option<SceneHookFn>,
}

impl std::fmt::Debug for SceneHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceneHooks")
            .field("started", &self.started.is_some())
            .field("ended", &self.ended.is_some())
            .finish()
    }
}

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

/// Active-only lookup indexes kept in sync with [`SceneMap`]'s records so
/// global command dispatch, room observers, and periodic ticks never scan
/// closed scenes.
#[derive(Clone, Debug, Default)]
struct ActiveIndexes {
    ids: BTreeSet<SceneUid>,
    by_anchor: HashMap<ObjId, BTreeSet<SceneUid>>,
    by_member: HashMap<ObjId, BTreeSet<SceneUid>>,
}

impl ActiveIndexes {
    fn index(&mut self, rec: &SceneRecord) {
        if !rec.status.holds_members() {
            return;
        }
        self.ids.insert(rec.id);
        self.by_anchor.entry(rec.anchor).or_default().insert(rec.id);
        for m in rec.members() {
            self.by_member.entry(m).or_default().insert(rec.id);
        }
    }

    fn unindex(&mut self, rec: &SceneRecord) {
        if !self.ids.remove(&rec.id) {
            return;
        }
        if let Some(set) = self.by_anchor.get_mut(&rec.anchor) {
            set.remove(&rec.id);
            if set.is_empty() {
                self.by_anchor.remove(&rec.anchor);
            }
        }
        for m in rec.members() {
            if let Some(set) = self.by_member.get_mut(&m) {
                set.remove(&rec.id);
                if set.is_empty() {
                    self.by_member.remove(&m);
                }
            }
        }
    }
}

/// Durable scene map with synchronized active-only lookup indexes.
///
/// Derefs to `BTreeMap<SceneUid, SceneRecord>` for all read operations while
/// maintaining O(1) active-scene lookups by member and anchor room across
/// `insert`, `remove`, and [`SceneMap::update_scene`]. Direct [`Self::get_mut`]
/// remains available for external callers (marking the entry dirty so active
/// lookups still reflect any field change); callers inside `mush-scenes` that
/// change `status`, `anchor`, or membership go through [`Self::update_scene`].
#[derive(Clone, Debug, Default)]
pub struct SceneMap {
    records: BTreeMap<SceneUid, SceneRecord>,
    active: ActiveIndexes,
    /// Records borrowed via direct [`Self::get_mut`], whose index fields
    /// (`status`/`anchor`/`members`) may have been modified outside
    /// [`Self::update_scene`]. Empty on all normal command/observer paths.
    dirty: BTreeSet<SceneUid>,
}

impl PartialEq for SceneMap {
    fn eq(&self, other: &Self) -> bool {
        self.records == other.records
    }
}

impl Deref for SceneMap {
    type Target = BTreeMap<SceneUid, SceneRecord>;

    fn deref(&self) -> &Self::Target {
        &self.records
    }
}

impl Index<&SceneUid> for SceneMap {
    type Output = SceneRecord;

    fn index(&self, index: &SceneUid) -> &Self::Output {
        &self.records[index]
    }
}

impl SceneMap {
    /// Build a [`SceneMap`] from an existing map of records, indexing every
    /// non-closed scene once.
    #[must_use]
    pub fn from_records(records: BTreeMap<SceneUid, SceneRecord>) -> Self {
        let mut active = ActiveIndexes::default();
        for rec in records.values() {
            active.index(rec);
        }
        Self {
            records,
            active,
            dirty: BTreeSet::new(),
        }
    }

    /// Re-sync any records previously mutated via direct [`Self::get_mut`].
    pub fn sync_dirty(&mut self) {
        if self.dirty.is_empty() {
            return;
        }
        let ids: Vec<SceneUid> = std::mem::take(&mut self.dirty).into_iter().collect();
        for id in ids {
            if self.active.ids.remove(&id) {
                self.active.by_anchor.retain(|_, set| {
                    set.remove(&id);
                    !set.is_empty()
                });
                self.active.by_member.retain(|_, set| {
                    set.remove(&id);
                    !set.is_empty()
                });
            }
            if let Some(rec) = self.records.get(&id) {
                self.active.index(rec);
            }
        }
    }

    /// Insert or replace a scene record, updating the active-only indexes.
    pub fn insert(&mut self, id: SceneUid, rec: SceneRecord) -> Option<SceneRecord> {
        self.sync_dirty();
        if let Some(prev) = self.records.get(&id) {
            self.active.unindex(prev);
        }
        self.active.index(&rec);
        self.records.insert(id, rec)
    }

    /// Remove a scene record, clearing it from the active-only indexes.
    pub fn remove(&mut self, id: &SceneUid) -> Option<SceneRecord> {
        self.sync_dirty();
        let prev = self.records.remove(id)?;
        self.active.unindex(&prev);
        Some(prev)
    }

    /// Direct mutable borrow (preserves `BTreeMap::get_mut` signature for
    /// cross-crate callers and tests). Marks `id` dirty so lookups stay exact
    /// without scanning closed scenes; callers inside `mush-scenes` should
    /// prefer [`Self::update_scene`].
    pub fn get_mut(&mut self, id: &SceneUid) -> Option<&mut SceneRecord> {
        if !self.records.contains_key(id) {
            return None;
        }
        self.dirty.insert(*id);
        self.records.get_mut(id)
    }

    /// Mutate one scene record while keeping the active-only indexes in sync.
    pub fn update_scene<R>(
        &mut self,
        id: SceneUid,
        f: impl FnOnce(&mut SceneRecord) -> R,
    ) -> Option<R> {
        self.sync_dirty();
        let record = self.records.get_mut(&id)?;
        self.active.unindex(record);
        let out = f(record);
        self.active.index(record);
        Some(out)
    }

    /// Lowest-id active (non-closed) scene holding `obj`, in O(1) when clean.
    #[must_use]
    pub fn active_for_member(&self, obj: ObjId) -> Option<SceneUid> {
        let indexed = self.active.by_member.get(&obj).and_then(|ids| {
            ids.iter().copied().find(|id| {
                self.records
                    .get(id)
                    .is_some_and(|r| r.status.holds_members() && r.members().any(|m| m == obj))
            })
        });
        if self.dirty.is_empty() {
            return indexed;
        }
        let dirty_match = self.dirty.iter().copied().find(|id| {
            self.records
                .get(id)
                .is_some_and(|r| r.status.holds_members() && r.members().any(|m| m == obj))
        });
        match (indexed, dirty_match) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// All active (non-closed) scenes holding `obj`.
    #[must_use]
    pub fn active_ids_for_member(&self, obj: ObjId) -> Vec<SceneUid> {
        let mut out: BTreeSet<SceneUid> = self
            .active
            .by_member
            .get(&obj)
            .map(|ids| {
                ids.iter()
                    .copied()
                    .filter(|id| {
                        self.records.get(id).is_some_and(|r| {
                            r.status.holds_members() && r.members().any(|m| m == obj)
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        for &id in &self.dirty {
            if self
                .records
                .get(&id)
                .is_some_and(|r| r.status.holds_members() && r.members().any(|m| m == obj))
            {
                out.insert(id);
            }
        }
        out.into_iter().collect()
    }

    /// Lowest-id active (non-closed) scene anchored to `room`, in O(1) when clean.
    #[must_use]
    pub fn active_for_anchor(&self, room: ObjId) -> Option<SceneUid> {
        let indexed = self.active.by_anchor.get(&room).and_then(|ids| {
            ids.iter().copied().find(|id| {
                self.records
                    .get(id)
                    .is_some_and(|r| r.status.holds_members() && r.anchor == room)
            })
        });
        if self.dirty.is_empty() {
            return indexed;
        }
        let dirty_match = self.dirty.iter().copied().find(|id| {
            self.records
                .get(id)
                .is_some_and(|r| r.status.holds_members() && r.anchor == room)
        });
        match (indexed, dirty_match) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Iterator over active (non-closed) scene records only.
    pub fn active_records(&self) -> impl Iterator<Item = &SceneRecord> {
        self.active
            .ids
            .union(&self.dirty)
            .filter_map(|id| self.records.get(id))
            .filter(|r| r.status.holds_members())
    }

    /// Number of active (non-closed) scenes.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.active_records().count()
    }

    /// Number of closed scenes currently retained.
    #[must_use]
    pub fn closed_count(&self) -> usize {
        self.records.len().saturating_sub(self.active_count())
    }
}

/// The live scenes state, held in a module state slot on the `World`.
#[derive(Clone, Debug, Default)]
pub struct ScenesState {
    /// In-flight archive reads, one per requesting actor (H1). Transient.
    pub pending_reads: HashMap<ObjId, PendingArchiveRead>,
    /// The wiring-frozen sealer gate (E6b independent verify fold, M4). `None` on
    /// any composition that wires no economy, which is the pre-fold behaviour.
    pub sealer_gate: SealerGateSlot,
    /// Every scene by durable id (Closed ones retained up to
    /// [`MAX_RETAINED_CLOSED_SCENES`]), with active-only lookup indexes.
    pub scenes: SceneMap,
    /// The durable-id allocator resume point (persisted in the meta doc).
    pub next_scene_uid: u64,
    /// The per-scene immutable event logs, keyed by the scene's durable id
    /// (stringified). Chunked, append-only, idempotent — the ledger substrate.
    /// Only each log's open tail lives here; sealed history is on disk.
    pub events: Ledger,
    /// Per-player rolling timestamps of recently started scenes (transient).
    pub(crate) creation_history: HashMap<ObjId, VecDeque<i64>>,
    /// Transient last-activity wall timestamp per active scene (speech/rolls/turns).
    pub(crate) last_activity: HashMap<SceneUid, i64>,
}

impl PartialEq for ScenesState {
    fn eq(&self, other: &Self) -> bool {
        self.pending_reads == other.pending_reads
            && self.sealer_gate == other.sealer_gate
            && self.scenes == other.scenes
            && self.next_scene_uid == other.next_scene_uid
            && self.events == other.events
    }
}

impl ScenesState {
    /// The scene an object is an active (non-closed) **member** of
    /// (participant or observer), if any — cut 1 enforces at most one such per
    /// object. O(1) via active-only member index; never scans closed scenes.
    #[must_use]
    pub fn active_scene_of(&self, obj: ObjId) -> Option<&SceneRecord> {
        self.scenes
            .active_for_member(obj)
            .and_then(|id| self.scenes.get(&id))
    }

    /// The active (non-closed) scene anchored to `room`, if any. O(1) via
    /// active-only anchor index; never scans closed scenes.
    #[must_use]
    pub fn active_scene_at(&self, room: ObjId) -> Option<&SceneRecord> {
        self.scenes
            .active_for_anchor(room)
            .and_then(|id| self.scenes.get(&id))
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

    /// Check whether `actor` has room in their rolling scene-creation window.
    /// Returns `Ok(())` or `Err(seconds_until_next_slot)`.
    pub fn check_creation_limit(&mut self, actor: ObjId, now: i64) -> Result<(), i64> {
        let cutoff = now.saturating_sub(SCENE_CREATE_WINDOW_SECS);
        let Some(times) = self.creation_history.get_mut(&actor) else {
            return Ok(());
        };
        while times.front().is_some_and(|&ts| ts <= cutoff) {
            times.pop_front();
        }
        if times.len() >= MAX_SCENES_PER_PLAYER_WINDOW {
            let oldest = times.front().copied().unwrap_or(now);
            let wait = oldest
                .saturating_add(SCENE_CREATE_WINDOW_SECS)
                .saturating_sub(now)
                .max(1);
            return Err(wait);
        }
        if times.is_empty() {
            self.creation_history.remove(&actor);
        }
        Ok(())
    }

    /// Record one scene creation for `actor` at wall time `now`.
    pub fn record_creation(&mut self, actor: ObjId, now: i64) {
        let cutoff = now.saturating_sub(SCENE_CREATE_WINDOW_SECS);
        let times = self.creation_history.entry(actor).or_default();
        while times.front().is_some_and(|&ts| ts <= cutoff) {
            times.pop_front();
        }
        times.push_back(now);
    }

    /// Record wall-clock activity on `scene` (turn advance, speech, roll, etc.).
    pub fn note_activity(&mut self, scene: SceneUid, now: i64) {
        let entry = self.last_activity.entry(scene).or_insert(now);
        if now > *entry {
            *entry = now;
        }
    }

    /// Most recent wall-clock activity timestamp observed for `rec`.
    #[must_use]
    pub fn last_activity_at(&self, rec: &SceneRecord) -> i64 {
        self.last_activity
            .get(&rec.id)
            .copied()
            .unwrap_or(rec.created_at)
            .max(rec.turn_started_at)
            .max(rec.created_at)
    }

    /// Evict the oldest closed scenes past `max_closed`, returning their ids
    /// so callers can delete their persisted documents. Active/paused scenes
    /// and scenes with in-flight archive reads are never evicted.
    pub fn prune_closed_scenes(&mut self, max_closed: usize) -> Vec<SceneUid> {
        let closed = self.scenes.closed_count();
        let excess = closed.saturating_sub(max_closed);
        if excess == 0 {
            return Vec::new();
        }
        let in_flight: BTreeSet<SceneUid> = self.pending_reads.values().map(|r| r.scene).collect();
        let victims: Vec<SceneUid> = self
            .scenes
            .values()
            .filter(|s| s.status == SceneStatus::Closed && !in_flight.contains(&s.id))
            .map(|s| s.id)
            .take(excess)
            .collect();
        for id in &victims {
            self.scenes.remove(id);
            self.last_activity.remove(id);
        }
        victims
    }

    /// Every object that any non-closed scene currently holds — the set the
    /// driver mirrors into the core exclusive-scene index (on freeze and on
    /// boot). Only exclusive scenes contribute.
    #[must_use]
    pub fn held_objects(&self) -> BTreeSet<ObjId> {
        let mut set = BTreeSet::new();
        for s in self.scenes.active_records() {
            if s.exclusive {
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

/// The public half of `persist_scene`, for the **one** cross-crate writer this
/// crate has: `mush-sim`'s journey arrival, which raises an onboard scene's IC
/// floor through [`SceneRecord::raise_ic_floor_on_arrival`] and must mirror the
/// raised record in the same `Outbox` batch as the arrival itself (E7 cut 6 B4,
/// 10b §3.6 OPEN 3). It is deliberately the *only* seam that writes a scene
/// document from outside `mush-scenes`: everything else about a scene — its
/// lifecycle, its turn cursor, its seal — stays this crate's.
pub fn persist_scene_record(out: &mut Outbox, s: &SceneRecord) {
    persist_scene(out, s);
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
            consent: Vec::new(),
            nudge_after_s: None,
            turn_started_at: 0,
            turn_visit: 0,
            nudged: None,
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

    // ---- E7c6: B4 — the arrival IC-floor-raise seam (10b §3.6 OPEN 3) ----

    #[test]
    fn e7c6_b4_arrival_floor_is_monotonic_and_keeps_generation_1() {
        // The arrival case added to E6b's start->advance->pause/restart->advance
        // monotonicity discipline: an arrival raise moves the floor forward,
        // clamps `scene_ic_time` up to it, and never bumps `clock_generation`.
        let mut s = scene(vec![2, 3]);
        let world_now = 10_000;
        assert_eq!(s.scene_ic_time(), 500);
        assert_eq!(s.raise_ic_floor_on_arrival(4_000, world_now), Ok(4_000));
        assert_eq!(s.ic_floor, 4_000);
        assert_eq!(
            s.scene_ic_time(),
            4_000,
            "the clock is clamped up to the floor"
        );
        assert_eq!(
            s.clock_generation, 1,
            "an arrival raise never bumps the generation"
        );
        // A later arrival still moves forward.
        assert_eq!(s.raise_ic_floor_on_arrival(5_500, world_now), Ok(5_500));
        assert_eq!(s.clock_generation, 1);
    }

    #[test]
    fn e7c6_b4_arrival_floor_only_moves_forward() {
        let mut s = scene(vec![2, 3]);
        s.raise_ic_floor_on_arrival(4_000, 10_000).unwrap();
        // At or below the current floor is refused (not silently ignored).
        assert_eq!(
            s.raise_ic_floor_on_arrival(4_000, 10_000),
            Err(ArrivalFloorRefusal::NotForward {
                at: 4_000,
                floor: 4_000
            })
        );
        assert_eq!(
            s.raise_ic_floor_on_arrival(3_000, 10_000),
            Err(ArrivalFloorRefusal::NotForward {
                at: 3_000,
                floor: 4_000
            })
        );
        assert_eq!(
            s.ic_floor, 4_000,
            "a refused raise leaves the floor unchanged"
        );
    }

    #[test]
    fn e7c6_b4_arrival_floor_is_bounded_ahead_of_the_world() {
        let mut s = scene(vec![2, 3]);
        let world_now = 10_000;
        // A stamp more than one IC year ahead of the world is refused.
        let too_far = world_now + MAX_IC_FLOOR_LEAD_SECS + 1;
        assert_eq!(
            s.raise_ic_floor_on_arrival(too_far, world_now),
            Err(ArrivalFloorRefusal::AheadOfWorld {
                at: too_far,
                world_now
            })
        );
        // A negative stamp is refused the same way.
        assert!(matches!(
            s.raise_ic_floor_on_arrival(-1, world_now),
            Err(ArrivalFloorRefusal::AheadOfWorld { .. })
        ));
        // Exactly at the bound is allowed.
        let at_bound = world_now + MAX_IC_FLOOR_LEAD_SECS;
        assert_eq!(
            s.raise_ic_floor_on_arrival(at_bound, world_now),
            Ok(at_bound)
        );
        assert_eq!(s.ic_floor, at_bound);
    }

    #[test]
    fn e7c6_b4_arrival_floor_is_refused_after_the_seal() {
        let mut s = scene(vec![2, 3]);
        s.sealer = Some(ObjId(1));
        s.outcome = SceneOutcome::new(
            SmolStr::new("success"),
            vec![ObjId(2)],
            String::new(),
            ObjId(1),
            9_000,
        );
        assert!(s.outcome.is_some());
        assert_eq!(
            s.raise_ic_floor_on_arrival(4_000, 10_000),
            Err(ArrivalFloorRefusal::AlreadySealed)
        );
        assert_eq!(s.ic_floor, 0, "a sealed scene's floor is immutable");
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

    #[test]
    fn turn_visit_survives_position_revisits_and_never_wraps() {
        let mut s = scene(vec![2, 3, 4]);
        let first = s.turn_visit;
        s.advance_turn().unwrap();
        s.rewind_turn().unwrap();
        assert_eq!(s.current_actor(), Some(ObjId(2)));
        assert!(s.turn_visit > first);
        let before = s.turn_visit;
        s.remove_member(ObjId(2));
        assert!(s.turn_visit > before);
        s.turn_visit = u64::MAX - 1;
        s.invalidate_nudge();
        s.invalidate_nudge();
        assert_eq!(s.turn_visit, u64::MAX);
    }

    #[test]
    fn older_scene_records_default_the_turn_visit() {
        let mut value = serde_json::to_value(scene(vec![2, 3])).unwrap();
        value.as_object_mut().unwrap().remove("turn_visit");
        let restored: SceneRecord = serde_json::from_value(value).unwrap();
        assert_eq!(restored.turn_visit, 0);
    }

    #[test]
    fn high24_active_indexes_ignore_closed_scenes_and_track_updates() {
        let mut st = ScenesState::default();
        for i in 0..500u64 {
            let mut closed = scene(vec![100]);
            closed.id = SceneUid(i);
            closed.anchor = ObjId(50);
            closed.status = SceneStatus::Closed;
            st.scenes.insert(closed.id, closed);
        }
        let mut active = scene(vec![2, 3]);
        active.id = SceneUid(999);
        active.anchor = ObjId(50);
        active.status = SceneStatus::Active;
        st.scenes.insert(active.id, active);

        assert_eq!(st.scenes.active_count(), 1);
        assert_eq!(st.scenes.closed_count(), 500);
        assert_eq!(st.active_scene_of(ObjId(100)), None);
        assert_eq!(
            st.active_scene_of(ObjId(2)).map(|s| s.id),
            Some(SceneUid(999))
        );
        assert_eq!(
            st.active_scene_at(ObjId(50)).map(|s| s.id),
            Some(SceneUid(999))
        );

        st.scenes.update_scene(SceneUid(999), |s| {
            s.status = SceneStatus::Closed;
        });
        assert_eq!(st.scenes.active_count(), 0);
        assert_eq!(st.active_scene_of(ObjId(2)), None);
        assert_eq!(st.active_scene_at(ObjId(50)), None);
    }

    #[test]
    fn high24_prune_closed_scenes_evicts_oldest_and_preserves_active() {
        let mut st = ScenesState::default();
        for _ in 0..(MAX_RETAINED_CLOSED_SCENES + 10) {
            let id = st.alloc_uid();
            let mut c = scene(vec![2]);
            c.id = id;
            c.status = SceneStatus::Closed;
            st.scenes.insert(id, c);
        }
        let live_id = st.alloc_uid();
        let mut live = scene(vec![3]);
        live.id = live_id;
        live.status = SceneStatus::Active;
        st.scenes.insert(live_id, live);

        let evicted = st.prune_closed_scenes(MAX_RETAINED_CLOSED_SCENES);
        assert_eq!(evicted.len(), 10);
        assert_eq!(evicted[0], SceneUid(0));
        assert_eq!(evicted[9], SceneUid(9));
        assert_eq!(st.scenes.closed_count(), MAX_RETAINED_CLOSED_SCENES);
        assert!(st.scenes.get(&live_id).is_some());
        assert!(st.scenes.get(&SceneUid(0)).is_none());
        assert!(st.scenes.get(&SceneUid(10)).is_some());
    }

    #[test]
    fn high24_record_consent_dedups_and_caps_at_max_consent_votes() {
        let mut s = scene(vec![2, 3]);
        s.record_consent(ConsentVote {
            about: SmolStr::new(CONSENT_REWIND),
            participant: ObjId(2),
            choice: SmolStr::new("agree"),
            round: 1,
            at: 100,
        });
        s.record_consent(ConsentVote {
            about: SmolStr::new(CONSENT_REWIND),
            participant: ObjId(2),
            choice: SmolStr::new("disagree"),
            round: 1,
            at: 101,
        });
        assert_eq!(s.consent.len(), 1, "duplicate vote replaced in place");
        assert_eq!(s.consent[0].choice, "disagree");

        for round in 2..=(MAX_CONSENT_VOTES as u32 + 25) {
            s.record_consent(ConsentVote {
                about: SmolStr::new(CONSENT_REWIND),
                participant: ObjId(2),
                choice: SmolStr::new("agree"),
                round,
                at: i64::from(round),
            });
        }
        assert_eq!(s.consent.len(), MAX_CONSENT_VOTES);
        assert_eq!(
            s.consent.last().unwrap().round,
            MAX_CONSENT_VOTES as u32 + 25
        );
    }
}
