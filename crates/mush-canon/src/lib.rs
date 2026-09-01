//! # mush-canon
//!
//! The **canon event ledger** (design: `docs/scenes_factions_knowledge.md`
//! §5): the durable, append-only record of what *became true* in the world —
//! the consequences selected from scene and world activity, distinct from the
//! full transcript (the per-scene event log).
//!
//! Domain modules emit canon events explicitly via [`commit`] alongside their
//! own state mutations (the scenes module records a scene's conclusion; faction
//! and ruleset modules will record territory changes, deaths, obligations). The
//! ledger never rewrites history: a [`retcon`] *appends* a superseding event, so
//! the audit trail and every derived artifact's source ids stay intact.
//!
//! Reading:
//! - `+chronicle` — the recent canon a viewer is entitled to (scope-filtered),
//!   the "previously on…" of the world.
//! - `@canon` (staff) — the full recent ledger with ids and scopes.
//! - `@canon/retcon <id>=<reason>` (staff) — append a supersession.
//!
//! Built on the chunked [`Ledger`](mush_core::Ledger) substrate: only the open
//! tail lives in RAM; deep history pages in through the store-read seam (built
//! with its consumers — cold recaps and deep chronicle).

mod commands;
mod data;

use mush_core::module::{GameModule, ModError, ModuleBootData, ModuleCtx, Registrar};
use mush_core::{Ctx, is_log_key};
use smol_str::SmolStr;

pub use data::{
    CanonDraft, CanonEvent, CanonId, CanonScope, CanonState, LOG, META_KEY, NS,
};

use data::{persist_meta, push_op};

/// The canon module. Hand one to `Engine::with_modules`.
pub struct CanonModule;

impl CanonModule {
    /// A fresh module instance.
    #[must_use]
    pub fn new() -> CanonModule {
        CanonModule
    }
}

impl Default for CanonModule {
    fn default() -> Self {
        CanonModule::new()
    }
}

impl GameModule for CanonModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(CanonState::default());
        commands::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let mut ledger = mush_core::Ledger::new();
        ledger.restore_from_kv(&data.kv);
        let mut next_from_meta: Option<u64> = None;
        for (key, value) in &data.kv {
            if key == META_KEY {
                next_from_meta = value.get("next_canon_id").and_then(serde_json::Value::as_u64);
            } else if is_log_key(key) {
                // Rehydrated into the ledger above.
            } else {
                tracing::warn!(%key, "canon: unknown persisted key ignored");
            }
        }
        // Resume the id allocator. `meta` is authoritative under normal
        // operation (it commits in the SAME batch as every append, so it can
        // never fall behind surviving chunks). The tail-max is a best-effort
        // safety net for a partial/stale-meta *restore*: it catches a stale
        // meta only while the newest ids are still in the open tail (the common
        // case — canon events are sparse, so the tail holds recent ids for a
        // very long time). A restore where the newest chunk is already sealed
        // AND meta is stale would need the store-read seam to rescan all chunks
        // for the true max — a documented ops-restore limitation, not a
        // normal-path hazard.
        let tail_max = ledger
            .open_entries(LOG)
            .iter()
            .filter_map(|e| serde_json::from_value::<CanonEvent>(e.value.clone()).ok())
            .map(|e| e.id.0)
            .max();
        let next_canon_id = next_from_meta
            .unwrap_or(0)
            .max(tail_max.map_or(0, |m| m + 1));
        *ctx.w
            .module_state_mut::<CanonState>()
            .expect("canon state slot installed at registration") = CanonState {
            ledger,
            next_canon_id,
        };
        Ok(())
    }
}

/// Record a durable consequence in the canon ledger, returning its stable id
/// (or `None` if the canon module is not loaded — emission is a no-op then, so
/// callers can emit unconditionally). The append is idempotent on retry and
/// commits in the caller's write batch.
///
/// Call this on the **success path**, after the mutation the event records has
/// committed to happening (the same ledger transaction discipline as the scene
/// log).
pub fn commit(ctx: &mut Ctx<'_>, draft: CanonDraft) -> Option<CanonId> {
    let now = ctx.w.now();
    let (id, next, op) = {
        let st = ctx.w.module_state_mut::<CanonState>()?;
        let id = st.alloc_id();
        let event = CanonEvent {
            id,
            kind: draft.kind,
            summary: draft.summary,
            entities: draft.entities,
            scope: draft.scope,
            ic_time: draft.ic_time,
            at: now,
            source: draft.source,
            supersedes: None,
            reason: String::new(),
            detail: draft.detail,
        };
        let value = serde_json::to_value(&event).expect("canon event serializes");
        let (_seq, op) = st.ledger.append(LOG, value);
        (id, st.next_canon_id, op)
    };
    push_op(ctx.out, op);
    persist_meta(ctx.out, next);
    Some(id)
}

/// Append a retcon superseding canon event `target`, with a `reason`. History
/// is never rewritten: this records a NEW event whose `supersedes` marks the
/// target, so readers derive the target as no-longer-current while the audit
/// trail is preserved. Returns the retcon's id (or `None` if canon is not
/// loaded).
pub fn retcon(ctx: &mut Ctx<'_>, target: CanonId, reason: &str) -> Option<CanonId> {
    let now = ctx.w.now();
    let ic_now = ctx.w.game_now_unix();
    let (id, next, op) = {
        let st = ctx.w.module_state_mut::<CanonState>()?;
        // Inherit the target's scope and entities so a retcon of a Secret /
        // Restricted event is not itself broadcast as Public — its existence
        // and reason are exactly as sensitive as what it supersedes.
        let (scope, entities) = st
            .recent()
            .into_iter()
            .find(|e| e.id == target)
            .map_or((CanonScope::Secret, Vec::new()), |e| (e.scope, e.entities));
        let id = st.alloc_id();
        let event = CanonEvent {
            id,
            kind: SmolStr::new_static("retcon"),
            summary: format!("Retcon of canon #{target}: {reason}"),
            entities,
            scope,
            ic_time: ic_now,
            at: now,
            source: format!("canon:{target}"),
            supersedes: Some(target),
            reason: reason.to_string(),
            detail: serde_json::Value::Null,
        };
        let value = serde_json::to_value(&event).expect("canon event serializes");
        let (_seq, op) = st.ledger.append(LOG, value);
        (id, st.next_canon_id, op)
    };
    push_op(ctx.out, op);
    persist_meta(ctx.out, next);
    Some(id)
}
