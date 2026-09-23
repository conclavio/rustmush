//! The serde data that crosses the async boundary and lands in the store:
//! the draft payload, the persisted draft doc, the request snapshot, the
//! completion outcome, and the in-RAM [`WorldgenState`] slot.

use std::collections::HashMap;

use mush_core::{ObjId, PanelFeedHandle};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use mush_atlas::theme::Theme;

/// The module's persistence namespace and identity.
pub const NS: &str = "worldgen";

/// The web panel feed id side-by-side draft previews arrive on.
pub const DRAFT_FEED: &str = "worldgen.draft";

/// A single room's generated content — the structured output the model
/// produces and the shape a draft carries. Every field is optional on the
/// wire (`desc` is the only one the schema requires); missing fields
/// deserialize to their defaults so lenient parsing never fails on a
/// well-formed-but-sparse object.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DraftPayload {
    /// The room description (`desc` attribute).
    pub desc: String,
    /// One-line glance description (`short_desc` attribute).
    pub short_desc: String,
    /// `look <keyword>` detail entries (`detail:<keyword>` attributes).
    pub details: Vec<Detail>,
    /// Proposed descriptions for the room's **existing** exits.
    pub exits: Vec<ExitDraft>,
}

/// One `look <keyword>` detail.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Detail {
    /// The keyword `look <keyword>` matches.
    pub keyword: String,
    /// What the looker sees.
    pub desc: String,
}

/// A proposed exit description. The model only fills these for exits that
/// already exist (listed in the prompt); it never invents topology.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExitDraft {
    /// The direction/handle used to match the existing exit (e.g. "north").
    pub direction: String,
    /// A display name suggestion (informational; not auto-applied).
    pub name: String,
    /// The exit's `desc` (what `look <exit>` shows).
    pub desc: String,
    /// The third-person transit line others see (`osucc`).
    pub transit_msg: String,
}

/// A draft's lifecycle status.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DraftStatus {
    /// Awaiting review.
    #[default]
    Pending,
    /// Accepted (the draft is cleared on accept; this is transient).
    Accepted,
}

/// A persisted draft document (`draft:{room}:{builder}` in the `worldgen`
/// namespace). Never auto-committed — the builder reviews it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredDraft {
    /// The room the draft describes.
    pub room: ObjId,
    /// The builder who requested it (drafts are per-builder-per-room).
    pub builder: ObjId,
    /// The generated content.
    pub payload: DraftPayload,
    /// The builder's original brief prose (reused verbatim by `@gen/retry`).
    #[serde(default)]
    pub prose: String,
    /// SHA-256 of the assembled prompt (audit correlation).
    pub prompt_hash: String,
    /// The model that produced it.
    pub model: String,
    /// IC/wall creation time (unix seconds).
    pub created: i64,
    /// Lifecycle status.
    pub status: DraftStatus,
    /// The building batch this draft belongs to (the entrance room's id), when
    /// it was produced by `@gen/building`; `None` for a single-room `@gen`.
    /// Lets `@gen/review <building>` and `@gen/accept` update batch progress.
    #[serde(default)]
    pub building: Option<ObjId>,
}

/// The persistence key a draft lives under.
#[must_use]
pub fn draft_key(room: ObjId, builder: ObjId) -> SmolStr {
    SmolStr::new(format!("draft:{}:{}", room.0, builder.0))
}

/// Which generation this is. Single-room drafts carry `Room`; the
/// whole-building brief and its per-room passes are modeled by
/// [`BriefSnapshot`] / [`WorldgenReq`] rather than a `GenKind` arm, so a
/// batch room's [`GenSnapshot`] stays a plain `Room`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GenKind {
    /// A single room draft (`@gen here=<prose>`) or one batch room.
    Room,
}

/// One neighboring room's accepted description, seeded into the prompt for
/// cross-room consistency (the adjacency graph is the retrieval mechanism).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NeighborDesc {
    /// The exit direction reaching the neighbor (e.g. "north").
    pub direction: String,
    /// The neighbor room's name.
    pub name: String,
    /// The neighbor's current (accepted) `desc`.
    pub desc: String,
}

/// One existing exit of the room, as a fact stated to the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExitFact {
    /// The exit's primary display name.
    pub name: String,
    /// Its aliases (`;`-segments plus indexed aliases).
    pub aliases: Vec<String>,
    /// The destination room's name, if linked.
    pub destination: Option<String>,
}

/// The room's immutable facts (name, exits) stated to the model — never
/// invented, only described.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomFacts {
    /// The room's name.
    pub name: String,
    /// The room's existing exits.
    pub exits: Vec<ExitFact>,
}

/// The synchronously-captured context handed to the off-loop generator (the
/// [`Effect::ModuleAsync`](mush_core::engine::Effect::ModuleAsync) payload).
/// Owned data only — no world references (treaty ruling 3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenSnapshot {
    /// What kind of generation.
    pub kind: GenKind,
    /// The room being described.
    pub room: ObjId,
    /// The requesting builder.
    pub builder: ObjId,
    /// The resolved theme block.
    pub theme: Theme,
    /// The room's facts.
    pub facts: RoomFacts,
    /// Up to K=4 neighbors' accepted descriptions.
    pub neighbors: Vec<NeighborDesc>,
    /// The builder's free-prose brief.
    pub prose: String,
    /// Extra guidance appended on `@gen/retry`.
    #[serde(default)]
    pub guidance: Option<String>,
    /// The previously-rejected draft, included as "avoid this" on retry.
    #[serde(default)]
    pub prior_draft: Option<DraftPayload>,
}

/// The generator's result (the
/// [`WorldCommand::ModuleAsyncDone`](mush_core::engine::WorldCommand::ModuleAsyncDone)
/// payload).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum GenOutcome {
    /// A draft is ready.
    Ok {
        /// The room it describes.
        room: ObjId,
        /// The builder who requested it.
        builder: ObjId,
        /// The generated content.
        draft: DraftPayload,
        /// The builder's original brief prose (carried through for retry).
        prose: String,
        /// SHA-256 of the assembled prompt.
        prompt_hash: String,
        /// The model that produced it.
        model: String,
        /// The building batch (entrance id) this room belongs to, when
        /// produced by `@gen/building`; `None` for a single-room `@gen`.
        #[serde(default)]
        building: Option<ObjId>,
    },
    /// Generation failed (budget, refusal, malformed twice, transport).
    Err {
        /// The room it was for.
        room: ObjId,
        /// The builder who requested it.
        builder: ObjId,
        /// A player-readable message.
        message: String,
        /// Whether `@gen/retry` might succeed.
        retryable: bool,
        /// The building batch (entrance id) this room belongs to, when
        /// produced by `@gen/building`; `None` for a single-room `@gen`.
        #[serde(default)]
        building: Option<ObjId>,
        /// Whether the failure was a budget ceiling trip — a batch degrades
        /// the remaining rooms to `needs_attention` rather than churning
        /// through N more doomed pre-checks.
        #[serde(default)]
        budget: bool,
    },
    /// A building brief is ready (the reviewable first stage of a batch).
    Brief {
        /// The building's entrance room.
        entrance: ObjId,
        /// The builder who requested it.
        builder: ObjId,
        /// The structured brief (shared palette + per-room concepts).
        brief: BuildingBriefPayload,
        /// The builder's original building prose (carried through for retry).
        prose: String,
        /// SHA-256 of the assembled prompt.
        prompt_hash: String,
        /// The model that produced it.
        model: String,
    },
    /// Brief generation failed.
    BriefErr {
        /// The building's entrance room.
        entrance: ObjId,
        /// The builder who requested it.
        builder: ObjId,
        /// A player-readable message.
        message: String,
        /// Whether `@gen/retry` might succeed.
        retryable: bool,
    },
    /// A planning run ended (S3): the outcome folds into its proposal.
    Plan {
        /// What the run came to.
        outcome: crate::proposal::PlanOutcome,
    },
}

/// In-RAM working state (a module state slot): the draft-preview panel
/// handle and the live draft cache, mirrored by the persisted
/// `draft:{room}:{builder}` documents. Rebuilt from boot data on load.
#[derive(Clone)]
pub struct WorldgenState {
    /// The reserved `worldgen.draft` panel feed.
    pub panel: PanelFeedHandle,
    /// Live drafts, keyed `(room, builder)` (single-room AND batch rooms).
    pub drafts: HashMap<(ObjId, ObjId), StoredDraft>,
    /// Live building batches, keyed `(entrance, builder)`, mirrored by the
    /// persisted `building:{entrance}:{builder}` documents.
    pub briefs: HashMap<(ObjId, ObjId), BuildingBrief>,
    /// Transient per-builder `@gen/review` cursors (not persisted — pure
    /// review-navigation state, rebuilt empty on boot).
    pub review: HashMap<ObjId, ReviewCursor>,
    /// The `@gen/building` interior-walk cap (config knob, default
    /// [`MAX_BUILDING_ROOMS`](crate::commands::MAX_BUILDING_ROOMS)). Held here
    /// so the batch command reads it from state; not persisted (it is a boot
    /// config value, re-applied on every boot from [`crate::WorldgenConfig`]).
    pub batch_max_rooms: usize,
    /// The building agent's proposals (S3), keyed by id, mirrored by the
    /// persisted `proposal:{id}` documents.
    pub proposals: std::collections::BTreeMap<u64, crate::proposal::ProposalRec>,
    /// The next proposal id (persisted as `proposal_meta`).
    pub next_proposal: u64,
    /// The planning limits (config knobs; not persisted).
    pub plan: crate::plan::PlanLimits,
}

impl WorldgenState {
    /// A fresh state around the reserved panel handle, using the default
    /// [`MAX_BUILDING_ROOMS`](crate::commands::MAX_BUILDING_ROOMS) batch cap.
    #[must_use]
    pub fn new(panel: PanelFeedHandle) -> WorldgenState {
        WorldgenState::with_batch_max(panel, crate::commands::MAX_BUILDING_ROOMS)
    }

    /// A fresh state with an explicit `@gen/building` room cap (config knob).
    #[must_use]
    pub fn with_batch_max(panel: PanelFeedHandle, batch_max_rooms: usize) -> WorldgenState {
        WorldgenState {
            panel,
            drafts: HashMap::new(),
            briefs: HashMap::new(),
            review: HashMap::new(),
            batch_max_rooms,
            proposals: std::collections::BTreeMap::new(),
            next_proposal: 1,
            plan: crate::plan::PlanLimits::default(),
        }
    }

    /// The draft for `(room, builder)`, if any.
    #[must_use]
    pub fn draft(&self, room: ObjId, builder: ObjId) -> Option<&StoredDraft> {
        self.drafts.get(&(room, builder))
    }

    /// The building batch for `(entrance, builder)`, if any.
    #[must_use]
    pub fn brief(&self, entrance: ObjId, builder: ObjId) -> Option<&BuildingBrief> {
        self.briefs.get(&(entrance, builder))
    }

    /// Find the builder's batch that contains `room` (as its entrance or one
    /// of its rooms), if exactly one such batch exists.
    #[must_use]
    pub fn batch_containing(&self, builder: ObjId, room: ObjId) -> Option<&BuildingBrief> {
        self.briefs
            .values()
            .find(|b| b.builder == builder && (b.entrance == room || b.contains(room)))
    }

    /// The builder's batches (any status).
    pub fn batches_of(&self, builder: ObjId) -> impl Iterator<Item = &BuildingBrief> {
        self.briefs.values().filter(move |b| b.builder == builder)
    }
}

/// A transient `@gen/review` cursor: which building the builder is stepping
/// through and which item is current. Purely in-RAM review navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReviewCursor {
    /// The building (entrance room) being reviewed.
    pub entrance: ObjId,
    /// The current review item.
    pub target: ReviewTarget,
}

/// What a [`ReviewCursor`] currently points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewTarget {
    /// The building brief (accept it to launch the room pass).
    Brief,
    /// One room's draft.
    Room(ObjId),
}

// ----------------------------------------------------------------------
// Batch (whole-building) generation — design 03 §5.3
// ----------------------------------------------------------------------

/// Per-room progress within a building batch.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomState {
    /// Not yet generated (waiting its turn in the pipeline).
    #[default]
    Pending,
    /// A room call is in flight (deferred/running off-loop).
    Generating,
    /// A draft is ready for review.
    Drafted,
    /// Generation failed, the room was refused, or it needs manual attention
    /// (a budget trip mid-batch degrades the remaining rooms here).
    NeedsAttention,
    /// The draft was reviewed and accepted (written through the attr path).
    Accepted,
}

impl RoomState {
    /// A short human label for `@gen/review` listings.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            RoomState::Pending => "pending",
            RoomState::Generating => "generating",
            RoomState::Drafted => "drafted",
            RoomState::NeedsAttention => "needs attention",
            RoomState::Accepted => "accepted",
        }
    }

    /// Whether this room has a draft awaiting review.
    #[must_use]
    pub fn reviewable(self) -> bool {
        matches!(self, RoomState::Drafted | RoomState::NeedsAttention)
    }
}

/// A building batch's lifecycle.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    /// The brief is being generated.
    #[default]
    BriefPending,
    /// The brief is ready for review (before any room call is spent).
    BriefReady,
    /// The brief was accepted; rooms are being drafted.
    Generating,
    /// Every room has been drafted or marked needs-attention.
    Complete,
    /// The brief generation failed.
    Failed,
}

impl BatchStatus {
    /// A short human label for `@gen/review` listings.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            BatchStatus::BriefPending => "brief generating",
            BatchStatus::BriefReady => "brief ready",
            BatchStatus::Generating => "generating rooms",
            BatchStatus::Complete => "complete",
            BatchStatus::Failed => "failed",
        }
    }
}

/// One room within a building batch (stored in BFS order from the entrance).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BatchRoom {
    /// The room object.
    pub id: ObjId,
    /// Its name at batch-creation time (for review listings).
    pub name: String,
    /// The brief's one-line concept for this room (filled once the brief
    /// completes; seeded into the room prompt).
    #[serde(default)]
    pub concept: String,
    /// Progress.
    #[serde(default)]
    pub status: RoomState,
}

/// A persisted building batch record (`building:{entrance}:{builder}` in the
/// `worldgen` namespace). Groups a batch's brief + per-room drafts. Never
/// auto-commits anything — the brief and each room draft are reviewed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildingBrief {
    /// The entrance room — the batch id.
    pub entrance: ObjId,
    /// The builder who requested it.
    pub builder: ObjId,
    /// The builder's building prose (reused verbatim by brief `@gen/retry`).
    pub prose: String,
    /// Creation time (unix seconds).
    pub created: i64,
    /// Lifecycle status.
    pub status: BatchStatus,
    /// The brief's overall building concept (filled once the brief completes).
    #[serde(default)]
    pub concept: String,
    /// The brief's shared palette — motifs/materials/light+sound (filled once
    /// the brief completes; seeded into every room prompt).
    #[serde(default)]
    pub palette: String,
    /// Every room in the building, BFS order from the entrance.
    pub rooms: Vec<BatchRoom>,
    /// SHA-256 of the brief prompt (audit correlation).
    #[serde(default)]
    pub prompt_hash: String,
    /// The model that produced the brief.
    #[serde(default)]
    pub model: String,
}

impl BuildingBrief {
    /// Whether `room` is one of the batch's rooms.
    #[must_use]
    pub fn contains(&self, room: ObjId) -> bool {
        self.rooms.iter().any(|r| r.id == room)
    }

    /// The batch room record for `room`, if any.
    #[must_use]
    pub fn room(&self, room: ObjId) -> Option<&BatchRoom> {
        self.rooms.iter().find(|r| r.id == room)
    }

    /// Mutable access to the batch room record for `room`.
    pub fn room_mut(&mut self, room: ObjId) -> Option<&mut BatchRoom> {
        self.rooms.iter_mut().find(|r| r.id == room)
    }

    /// The next room still `Pending`, in BFS order (the pipeline's next step).
    #[must_use]
    pub fn next_pending(&self) -> Option<ObjId> {
        self.rooms
            .iter()
            .find(|r| r.status == RoomState::Pending)
            .map(|r| r.id)
    }

    /// How many rooms have a draft (drafted or already accepted).
    #[must_use]
    pub fn drafted_count(&self) -> usize {
        self.rooms
            .iter()
            .filter(|r| matches!(r.status, RoomState::Drafted | RoomState::Accepted))
            .count()
    }

    /// The shared brief context seeded into each room prompt.
    #[must_use]
    pub fn context(&self) -> BuildingBriefContext {
        BuildingBriefContext {
            palette: self.palette.clone(),
            building_concept: self.concept.clone(),
        }
    }
}

/// The persistence key a building batch lives under.
#[must_use]
pub fn building_key(entrance: ObjId, builder: ObjId) -> SmolStr {
    SmolStr::new(format!("building:{}:{}", entrance.0, builder.0))
}

/// The shared brief context seeded into every batch room's prompt (the
/// palette + overall concept; the room's own concept travels as its prose).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BuildingBriefContext {
    /// Shared motifs, materials, light and sound.
    pub palette: String,
    /// The overall building concept.
    pub building_concept: String,
}

/// One room's fact line for the brief graph walk — name + its interior exits,
/// as topology stated to the model (never invented).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BriefRoomFact {
    /// The room object (for mapping the brief's concepts back).
    pub id: ObjId,
    /// The room's name.
    pub name: String,
    /// Interior exits as `"dir -> DestName"` descriptors.
    pub exits: Vec<String>,
}

/// The synchronously-captured context for the BRIEF pass. Owned data only —
/// the graph walk (BFS from the entrance) is resolved on-loop and travels
/// here (treaty ruling 3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BriefSnapshot {
    /// The building's entrance room (the batch id).
    pub entrance: ObjId,
    /// The requesting builder.
    pub builder: ObjId,
    /// The resolved theme block.
    pub theme: Theme,
    /// The builder's free-prose building brief.
    pub prose: String,
    /// The building's rooms, BFS order (the graph walk).
    pub rooms: Vec<BriefRoomFact>,
    /// Extra guidance appended on brief `@gen/retry`.
    #[serde(default)]
    pub guidance: Option<String>,
    /// A previously-rejected brief, included as "avoid this" on retry.
    #[serde(default)]
    pub prior_brief: Option<BuildingBriefPayload>,
}

/// The structured output of the brief call: shared palette + a one-line
/// concept per room. Lenient parse (like [`DraftPayload`]): missing fields
/// default so a sparse-but-valid object never fails.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildingBriefPayload {
    /// One-line overall concept for the whole building.
    pub concept: String,
    /// Shared motifs, materials, light and sound.
    pub palette: String,
    /// One concept line per room.
    pub rooms: Vec<RoomConcept>,
}

/// One room's concept within a [`BuildingBriefPayload`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoomConcept {
    /// The 1-based room index from the brief prompt (primary mapping key).
    pub index: u32,
    /// The room name echoed back (secondary mapping key).
    pub name: String,
    /// The one-line concept.
    pub concept: String,
}

/// The off-loop request envelope for **batch** work. A single-room `@gen`
/// still serializes a bare [`GenSnapshot`] — [`run_worldgen_async`]
/// falls back to decoding one directly, so the historical wire format is
/// unchanged and only batch work is wrapped.
///
/// [`run_worldgen_async`]: crate::run_worldgen_async
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "wg", rename_all = "snake_case")]
pub enum WorldgenReq {
    /// The building brief pass (one strong-model call).
    Brief(BriefSnapshot),
    /// One room within a building batch — a normal room [`GenSnapshot`] plus
    /// the batch id and the shared brief context seeded into its prompt.
    BuildingRoom {
        /// The room's generation snapshot (neighbors already seeded from
        /// sibling drafts on-loop).
        snap: GenSnapshot,
        /// The building batch this room belongs to (the entrance id).
        building: ObjId,
        /// The shared brief context.
        brief: BuildingBriefContext,
    },
    /// A planning run of the building agent (S3). Boxed: the snapshot carries the
    /// validator's whole context.
    Plan(Box<crate::proposal::PlanSnapshot>),
}
