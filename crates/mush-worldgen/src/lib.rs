//! # mush-worldgen — LLM-assisted worldbuilding
//!
//! The `worldgen` [`GameModule`]: hierarchical **themes**, and the
//! single-room **`@gen` draft pipeline** (design 03 §5). Built entirely on
//! the merged module API plus [`mush_llm`]'s `LlmService`; nothing the LLM
//! produces is ever auto-committed.
//!
//! ## Shape
//!
//! - [`Theme`] data lives on the atlas's places (S2b, doc 14 D-14-28): a
//!   region, locality, neighborhood or site carries a theme, the world theme is
//!   the default, `@atlas/theme` edits them, and [`resolve_theme`] (re-exported
//!   from `mush-atlas`) walks a room up through its site, neighborhood,
//!   locality and region to the world.
//! - `@gen here=<prose>` (gated on `BUILDER_LLM`) snapshots the prompt
//!   context synchronously and defers the LLM call as an
//!   [`Effect::ModuleAsync`](mush_core::engine::Effect::ModuleAsync) — the
//!   world loop never blocks (treaty ruling 3). The driver runs
//!   [`run_worldgen_async`] off-loop and re-enters via
//!   [`WorldCommand::ModuleAsyncDone`](mush_core::engine::WorldCommand::ModuleAsyncDone),
//!   which this module's [`AsyncCompletion`](mush_core::AsyncCompletion)
//!   stores as a **draft** and previews to the builder.
//! - The builder reviews with `@gen/accept [fields]` (writes `desc`,
//!   `short_desc`, `detail:*`, and exit descs through the ordinary
//!   attribute-set path), `@gen/edit`, `@gen/retry [guidance]`, and
//!   `@gen/discard`.
//! - Drafts persist via module ops (`draft:{room}:{builder}` in the
//!   `worldgen` namespace); the cache is rebuilt from boot data.
//! - Staff `@llm/budget|log|disable|enable` surface the `LlmService`
//!   ledger, audit log, and kill switch.
//!
//! ## Async re-entry path (documented seam)
//!
//! `@gen` → `Deferred::Effect(Effect::ModuleAsync{ module:"worldgen", .. })`
//! → the driver routes it (by module name) to
//! [`run_worldgen_async`]`(&LlmService, payload)` → the result re-enters as
//! `WorldCommand::ModuleAsyncDone{ module:"worldgen", .. }` → the engine
//! dispatches it to this module's registered `AsyncCompletion`. `mush-core`
//! owns the generic seam (`Effect::ModuleAsync` / `WorldCommand::ModuleAsyncDone`
//! / [`Registrar::async_completion`](mush_core::Registrar::async_completion));
//! `mushd` owns the `LlmService` handle and the route from the effect to
//! [`run_worldgen_async`].
//!
//! ## Batch `@gen/building` (M10, design 03 §5.3)
//!
//! `@gen/building <entrance>=<prose>` (gated on `BUILDER_LLM`) generates a
//! whole building in two reviewed stages, for coherence:
//!
//! 1. **Brief pass** — one [`LlmFeature::WorldgenBrief`](mush_llm::LlmFeature)
//!    call walks the building's room graph (BFS from the entrance, bounded
//!    [`MAX_BUILDING_ROOMS`](commands::MAX_BUILDING_ROOMS); interior = same
//!    zone as the entrance, or — for an unzoned entrance — within
//!    [`MAX_UNZONED_HOPS`](commands::MAX_UNZONED_HOPS)) and emits a
//!    [`BuildingBriefPayload`]: a shared palette + a one-line concept per
//!    room. The brief is a reviewable [`BuildingBrief`] draft — cheap to steer
//!    before N room calls.
//! 2. **Room pass** — accepting the brief drives a **completion-pipelined**
//!    sequence of `WorldgenRoom` calls at `Background` priority: each room's
//!    snapshot is (re)built on-loop with the brief + its concept + its
//!    already-drafted neighbors, so later rooms cohere with earlier drafts.
//!    Failures/refusals mark that room [`RoomState::NeedsAttention`] and the
//!    pipeline continues; a budget trip degrades the remaining rooms to
//!    needs-attention and stops (nothing auto-commits).
//!
//! The batch groups its drafts under a [`BuildingBrief`] record
//! (`building:{entrance}:{builder}`); `@gen/review [<building>]` lists it and
//! `@gen/review next` steps through the brief then each room draft with the
//! SAME `@gen/accept|edit|retry|discard` loop, plus `@gen/accept/all`. The
//! batch travels off-loop via the [`WorldgenReq`] envelope (a single-room
//! `@gen` still sends a bare [`GenSnapshot`], unchanged).

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

mod commands;
mod data;
mod generate;

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::{
    GameModule, ModError, ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, Registrar,
};
use mush_llm::LlmService;
use smol_str::SmolStr;

pub use commands::{MAX_BUILDING_ROOMS, MIN_BUILDING_ROOMS};
pub use data::{
    BatchRoom, BatchStatus, BriefRoomFact, BriefSnapshot, BuildingBrief, BuildingBriefContext,
    BuildingBriefPayload, DRAFT_FEED, Detail, DraftPayload, DraftStatus, ExitDraft, ExitFact,
    GenKind, GenOutcome, GenSnapshot, NS, NeighborDesc, ReviewCursor, ReviewTarget, RoomConcept,
    RoomFacts, RoomState, StoredDraft, WorldgenReq, WorldgenState, building_key, draft_key,
};
pub use generate::{
    brief_system_prompt, brief_user_prompt, extract_json_object, map_brief_concepts,
    run_worldgen_async, submit_building_brief_tool, submit_room_tool, system_prompt, user_prompt,
    user_prompt_inner,
};
pub use mush_atlas::theme::{DescLength, Theme, resolve_theme};

/// Tuning for the worldgen module (config knobs). Everything defaults, so a
/// keyless/config-less server behaves exactly as before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldgenConfig {
    /// The `@gen/building` interior-walk cap: the most rooms one batch will
    /// describe. Must be in [`MIN_BUILDING_ROOMS`]..=[`MAX_BUILDING_ROOMS`]
    /// and defaults to [`MAX_BUILDING_ROOMS`]. Invalid values fail module
    /// registration rather than creating an empty or unbounded batch.
    pub batch_max_rooms: usize,
}

impl Default for WorldgenConfig {
    fn default() -> WorldgenConfig {
        WorldgenConfig {
            batch_max_rooms: MAX_BUILDING_ROOMS,
        }
    }
}

/// The worldgen module. Construct with a shared [`LlmService`] handle (used
/// by the staff `@llm/*` views); the same service is what the driver calls
/// [`run_worldgen_async`] against off-loop.
pub struct WorldgenModule {
    llm: Arc<LlmService>,
    config: WorldgenConfig,
}

impl WorldgenModule {
    /// A worldgen module backed by `llm`, with default [`WorldgenConfig`].
    #[must_use]
    pub fn new(llm: Arc<LlmService>) -> WorldgenModule {
        WorldgenModule::with_config(llm, WorldgenConfig::default())
    }

    /// A worldgen module backed by `llm` with an explicit [`WorldgenConfig`]
    /// (the `[worldgen]` config knobs).
    #[must_use]
    pub fn with_config(llm: Arc<LlmService>, config: WorldgenConfig) -> WorldgenModule {
        WorldgenModule { llm, config }
    }
}

impl GameModule for WorldgenModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        validate_batch_max_rooms(self.config.batch_max_rooms)?;
        let panel = reg.panel_feed(DRAFT_FEED)?;
        reg.state(WorldgenState::with_batch_max(
            panel,
            self.config.batch_max_rooms,
        ));
        commands::register(reg, Arc::clone(&self.llm))
    }

    /// Rebuild the draft + batch caches from persisted `draft:*` /
    /// `building:*` documents. Tolerant: an unparseable doc is logged and
    /// skipped (never blocks boot), and unknown keys are ignored (a newer
    /// build's data).
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        boot: ModuleBootData,
    ) -> Result<(), ModError> {
        let mut drafts = HashMap::new();
        let mut briefs = HashMap::new();
        // Batches reconciled at boot (a `Generating` room downgraded to
        // `Pending`) that must be re-persisted so the store matches RAM.
        let mut reconciled: Vec<BuildingBrief> = Vec::new();
        for (key, value) in boot.kv {
            if key.starts_with("draft:") {
                match serde_json::from_value::<StoredDraft>(value) {
                    Ok(d) => {
                        drafts.insert((d.room, d.builder), d);
                    }
                    Err(e) => {
                        tracing::error!(%key, error = %e, "worldgen: unparseable draft skipped");
                    }
                }
            } else if key.starts_with("building:") {
                match serde_json::from_value::<BuildingBrief>(value) {
                    Ok(mut b) => {
                        // Reconcile a batch interrupted mid-room-pass by a
                        // reboot: a room persisted as `Generating` had an
                        // in-flight off-loop call the restart lost. Nothing
                        // re-picks a `Generating` room (`next_pending` matches
                        // only `Pending`) and `@gen/review` mislabels it
                        // "[generating]" forever — the wedged batch the
                        // durability model is meant to avoid. Downgrade it back
                        // to `Pending` so the pipeline re-drafts it; the batch
                        // stays `BatchStatus::Generating` and the next
                        // `@gen/review` re-arms the stalled pass (see
                        // `rearm_stalled_batch` in commands.rs). Deferring an
                        // effect here is forbidden at boot (the driver drops
                        // boot-time effects), so recovery rides the next live
                        // command instead.
                        let mut changed = false;
                        for r in &mut b.rooms {
                            if r.status == RoomState::Generating {
                                r.status = RoomState::Pending;
                                changed = true;
                            }
                        }
                        if changed {
                            reconciled.push(b.clone());
                        }
                        briefs.insert((b.entrance, b.builder), b);
                    }
                    Err(e) => {
                        tracing::error!(%key, error = %e, "worldgen: unparseable batch skipped");
                    }
                }
            } else {
                tracing::warn!(%key, "worldgen: unknown persisted key ignored");
            }
        }
        let st = ctx
            .state_mut::<WorldgenState>()
            .ok_or_else(|| ModError::msg("worldgen: state slot missing"))?;
        st.drafts = drafts;
        st.briefs = briefs;
        // Write the reconciled batches back through the store (write-through at
        // boot, like weather's orphan-cell cleanup) so the persisted doc no
        // longer carries the stale `Generating` room.
        for b in reconciled {
            match serde_json::to_value(&b) {
                Ok(value) => ctx.persist(ModuleOp {
                    ns: SmolStr::new(NS),
                    op: ModuleWrite::JsonSet {
                        key: building_key(b.entrance, b.builder),
                        value,
                    },
                }),
                Err(e) => {
                    tracing::error!(error = %e, "worldgen: could not re-serialize reconciled batch");
                }
            }
        }
        Ok(())
    }
}

fn validate_batch_max_rooms(value: usize) -> Result<(), ModError> {
    if (MIN_BUILDING_ROOMS..=MAX_BUILDING_ROOMS).contains(&value) {
        Ok(())
    } else {
        Err(ModError::msg(format!(
            "worldgen: batch_max_rooms must be between {MIN_BUILDING_ROOMS} and \
             {MAX_BUILDING_ROOMS}, got {value}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn building_room_cap_rejects_empty_and_unbounded_config() {
        assert!(validate_batch_max_rooms(MIN_BUILDING_ROOMS).is_ok());
        assert!(validate_batch_max_rooms(MAX_BUILDING_ROOMS).is_ok());
        assert!(validate_batch_max_rooms(MIN_BUILDING_ROOMS - 1).is_err());
        assert!(validate_batch_max_rooms(MAX_BUILDING_ROOMS + 1).is_err());
    }
}
