//! The serde data that crosses the async boundary and lands in the store:
//! the draft payload, the persisted draft doc, the request snapshot, the
//! completion outcome, and the in-RAM [`WorldgenState`] slot.

use std::collections::HashMap;

use mush_core::{ObjId, PanelFeedHandle};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::theme::Theme;

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
}

/// The persistence key a draft lives under.
#[must_use]
pub fn draft_key(room: ObjId, builder: ObjId) -> SmolStr {
    SmolStr::new(format!("draft:{}:{}", room.0, builder.0))
}

/// Which generation this is (single room now; whole-building brief is the
/// M10 seam — see [`crate`] docs).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GenKind {
    /// A single room draft (`@gen here=<prose>`).
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
    },
}

/// In-RAM working state (a module state slot): the draft-preview panel
/// handle and the live draft cache, mirrored by the persisted
/// `draft:{room}:{builder}` documents. Rebuilt from boot data on load.
#[derive(Clone)]
pub struct WorldgenState {
    /// The reserved `worldgen.draft` panel feed.
    pub panel: PanelFeedHandle,
    /// Live drafts, keyed `(room, builder)`.
    pub drafts: HashMap<(ObjId, ObjId), StoredDraft>,
}

impl WorldgenState {
    /// A fresh state around the reserved panel handle.
    #[must_use]
    pub fn new(panel: PanelFeedHandle) -> WorldgenState {
        WorldgenState {
            panel,
            drafts: HashMap::new(),
        }
    }

    /// The draft for `(room, builder)`, if any.
    #[must_use]
    pub fn draft(&self, room: ObjId, builder: ObjId) -> Option<&StoredDraft> {
        self.drafts.get(&(room, builder))
    }
}
