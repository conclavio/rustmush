//! # mush-worldgen — LLM-assisted worldbuilding
//!
//! The `worldgen` [`GameModule`]: hierarchical **themes**, and the
//! single-room **`@gen` draft pipeline** (design 03 §5). Built entirely on
//! the merged module API plus [`mush_llm`]'s `LlmService`; nothing the LLM
//! produces is ever auto-committed.
//!
//! ## Shape
//!
//! - [`Theme`] data lives in a `worldgen.theme` attribute on zones / rooms /
//!   `#0`, edited via `@theme` and resolved hierarchically
//!   ([`resolve_theme`]).
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
//! ## Not built (documented seams)
//!
//! - **Batch `@gen/building`** (whole-building brief + BFS room pass) is
//!   M10: [`GenKind`] has a single `Room` arm, and the `WorldgenBrief`
//!   feature route is reserved by `mush-llm`; add a `Brief` arm and a
//!   `@gen/building` handler there.

mod commands;
mod data;
mod generate;
mod theme;

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::{GameModule, ModError, ModuleBootData, ModuleCtx, Registrar};
use mush_llm::LlmService;

pub use data::{
    DRAFT_FEED, Detail, DraftPayload, DraftStatus, ExitDraft, ExitFact, GenKind, GenOutcome,
    GenSnapshot, NS, NeighborDesc, RoomFacts, StoredDraft, WorldgenState, draft_key,
};
pub use generate::{
    extract_json_object, run_worldgen_async, submit_room_tool, system_prompt, user_prompt,
};
pub use theme::{DescLength, THEME_ATTR, Theme, resolve_theme, theme_chain, theme_on};

/// The worldgen module. Construct with a shared [`LlmService`] handle (used
/// by the staff `@llm/*` views); the same service is what the driver calls
/// [`run_worldgen_async`] against off-loop.
pub struct WorldgenModule {
    llm: Arc<LlmService>,
}

impl WorldgenModule {
    /// A worldgen module backed by `llm`.
    #[must_use]
    pub fn new(llm: Arc<LlmService>) -> WorldgenModule {
        WorldgenModule { llm }
    }
}

impl GameModule for WorldgenModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let panel = reg.panel_feed(DRAFT_FEED)?;
        reg.state(WorldgenState::new(panel));
        commands::register(reg, Arc::clone(&self.llm))
    }

    /// Rebuild the draft cache from persisted `draft:*` documents. Tolerant:
    /// an unparseable draft is logged and skipped (never blocks boot), and
    /// unknown keys are ignored (a newer build's data).
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        boot: ModuleBootData,
    ) -> Result<(), ModError> {
        let mut drafts = HashMap::new();
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
            } else {
                tracing::warn!(%key, "worldgen: unknown persisted key ignored");
            }
        }
        let st = ctx
            .state_mut::<WorldgenState>()
            .ok_or_else(|| ModError::msg("worldgen: state slot missing"))?;
        st.drafts = drafts;
        Ok(())
    }
}
