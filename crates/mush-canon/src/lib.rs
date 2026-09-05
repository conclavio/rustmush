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

pub use data::{CanonDraft, CanonEvent, CanonId, CanonScope, CanonState, LOG, META_KEY, NS};

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
        ledger
            .restore_from_kv(&data.kv)
            .map_err(|error| ModError::msg(format!("canon ledger restore failed: {error}")))?;
        let mut next_from_meta: Option<u64> = None;
        for (key, value) in &data.kv {
            if key == META_KEY {
                next_from_meta = value
                    .get("next_canon_id")
                    .and_then(serde_json::Value::as_u64);
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
        // E6b independent verify fold (R3): decode the trailing **sealed** chunks
        // once, here, where the boot documents are still in hand. A sealed tail
        // restores as an empty open chunk, so without this the durable read seam
        // answers nothing at all whenever a 16 KB boundary fell between a seal and
        // the bounce — and the settlement that seal was evidence for waits for its
        // deadline. Bounded at `SEALED_TAIL_CHUNKS`, decoded once per boot, never
        // appended to afterwards.
        let sealed_tail: Vec<CanonEvent> =
            mush_core::recent_sealed(&data.kv, LOG, data::SEALED_TAIL_CHUNKS)
                .into_iter()
                .filter_map(|e| serde_json::from_value::<CanonEvent>(e.value).ok())
                .collect();
        // The allocator floor takes the sealed window into account too: it is the
        // half of the documented restore limitation above that this window can now
        // answer.
        let sealed_max = sealed_tail.iter().map(|e| e.id.0).max();
        let next_canon_id = next_canon_id.max(sealed_max.map_or(0, |m| m.saturating_add(1)));
        tracing::info!(
            sealed_events = sealed_tail.len(),
            "canon: sealed-tail evidence window restored"
        );
        *ctx.w
            .module_state_mut::<CanonState>()
            .expect("canon state slot installed at registration") = CanonState {
            ledger,
            next_canon_id,
            sealed_tail,
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
/// log). This is now doubly load-bearing: the `CanonRecorded` event below runs
/// its observers **synchronously**, and a faction clock's observer can flip
/// territory, complete a clock, and append further canon — all on the caller's
/// write batch. If the command later returns `Err`, dispatch truncates every
/// deferred persist but does **not** roll back RAM, so those observer mutations
/// would diverge from the store until reboot. Commit must therefore be the last
/// mutating act on any path that can still fail — treat it as terminal.
pub fn commit(ctx: &mut Ctx<'_>, draft: CanonDraft) -> Option<CanonId> {
    let now = ctx.w.now();
    // E6b verify fold (R9): **the producer's cap and the consumer's must agree.**
    // The economy remembers a record's entities bounded at its own
    // `MAX_CANON_ENTITIES`, so a draft naming more than that matched on the live edge
    // and was silently missed by the recovery re-scan. Capping here — the one place
    // every producer passes through — makes the two the same set rather than two
    // different truncations of one.
    let mut draft = draft;
    draft.entities.truncate(MAX_ENTITIES_PER_EVENT);
    // Captured for the observable `CanonRecorded` event emitted below.
    let kind = draft.kind.to_string();
    let entities = draft.entities.clone();
    // E4c: the event carries the draft's provenance too, so a source-specific
    // observer (the economy's contract attestation) can tell one front's
    // `scene.concluded` from another's without an id→event query this module
    // does not offer.
    let source = draft.source.clone();
    let (id, next, op) = {
        let st = ctx.w.module_state_mut::<CanonState>()?;
        let id = CanonId(st.next_canon_id);
        let Some(next) = st.next_canon_id.checked_add(1) else {
            tracing::error!("canon: id allocator exhausted; event refused without mutation");
            return None;
        };
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
        let prepared = match st.ledger.prepare_append(LOG, value) {
            Ok(prepared) => prepared,
            Err(error) => {
                tracing::error!(%error, "canon: unsafe ledger append refused without mutation");
                return None;
            }
        };
        let (_seq, op) = st
            .ledger
            .apply_prepared(prepared)
            .expect("canon ledger cannot change during one exclusive state borrow");
        st.next_canon_id = next;
        (id, next, op)
    };
    push_op(ctx.out, op);
    persist_meta(ctx.out, next);
    // Announce the recording so consequence-driven simulation (faction clocks)
    // can react. A no-op for anyone not subscribed. Retcons deliberately do NOT
    // emit this — they are meta-events, not new consequences. Observers run
    // synchronously and may themselves commit canon, re-entering here; keeping
    // this chain finite is each observer's responsibility (the faction clock
    // observer reserves its consequence kind and never re-advances a completed
    // clock — see mush_faction::on_canon_recorded).
    ctx.emit_world_event(&mush_core::WorldEvent::CanonRecorded {
        canon_id: id.0,
        kind,
        entities,
        source,
    });
    // **Terminal means terminal** (E4c verify fold, V1). Everything queued up to
    // here — this append, its meta, and every write the synchronous observers made
    // (an economy attestation, a settlement's journal chunk, a faction clock) — is
    // pinned against dispatch's abort path. RAM is not rolled back when a command
    // later returns `Err`, so dropping these would leave the store behind a world
    // that has already moved: canon recorded and an escrow released, with neither on
    // disk. Anything the command queues *after* this point is discarded as usual.
    ctx.out.pin_deferred();
    Some(id)
}

// ---- E6b: B5 — DL-09, the bounded read seam keyed on `source` -------------

/// How many entities one canon event names (E6b verify fold, R9).
///
/// It is the economy's own `MAX_CANON_ENTITIES`, restated here because this is the
/// side that must not exceed it: a consumer bounds what it remembers, and a producer
/// that wrote more would be relying on a set the recovery path cannot reconstruct.
pub const MAX_ENTITIES_PER_EVENT: usize = 64;

/// How many records one [`records_for_source`] lookup ever returns (E6b, DL-09).
///
/// The economy's re-scan asks per live `Canon` condition, so the cost of one
/// re-scan is bounded by (conditions × this), and a front that records canon every
/// minute cannot make a recovery walk unbounded.
pub const MAX_RECORDS_PER_SOURCE: usize = 8;

/// The canon records naming `source`, newest first, bounded (E6b, DL-09).
///
/// This is the read seam `mush-canon` did not have: it offers no id→event query,
/// and deep history has no store-read seam yet (08c §6.1 deviation 8), so this
/// answers from the ledger's **open tail** — which is exactly what a restart
/// rehydrates, and therefore exactly what makes a seal committed before a reboot
/// findable again. A record older than the open tail is the documented limit, and
/// `@contract/resolve <id>=settle` is the answer past it.
///
/// It is a free function over the state rather than a method on it so the caller
/// that bridges it into another module (the server's economy wiring) needs no new
/// type to reach across.
#[must_use]
pub fn records_for_source(state: &CanonState, source: &str, limit: usize) -> Vec<CanonEvent> {
    // E6b independent verify fold (R7): **decode the open tail once.** It was decoded
    // twice per lookup — once by `superseded_in_tail` and once by `recent` — and the
    // re-scan asks this per source, so a 64-entry tail was decoded 64 times per
    // recovery for nothing.
    let mut events = state.recent();
    // E6b independent verify fold (R3): and the last sealed chunks join it. The open
    // tail is empty whenever the tail chunk sealed before the process ended, which is
    // exactly the reboot DL-09 exists for; the boot-time window is the rest of the
    // seam. Sealed entries are older, so they go first and the newest-first reversal
    // below still holds.
    let mut all = state.sealed_tail.clone();
    all.append(&mut events);
    // E6b verify fold (R2): **a retconned record is not evidence.** A retcon appends
    // rather than rewrites and deliberately emits no `CanonRecorded`, so the economy
    // never hears it; without this filter the recovery path would resurrect a seal a
    // staffer had already superseded — the one route by which a re-scan could settle
    // a contract on retracted evidence, which the live path could never do.
    let superseded = CanonState::superseded_in(&all);
    let mut out: Vec<CanonEvent> = all
        .into_iter()
        .filter(|e| e.source == source && e.supersedes.is_none() && !superseded.contains(&e.id))
        .collect();
    out.reverse();
    // A record can appear in both windows only if a chunk were both sealed and open,
    // which the ledger's own invariants forbid; the id is still the dedupe key, so a
    // future window change cannot double-count evidence.
    let mut seen = std::collections::BTreeSet::new();
    out.retain(|e| seen.insert(e.id));
    out.truncate(limit.min(MAX_RECORDS_PER_SOURCE));
    out
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
        let id = CanonId(st.next_canon_id);
        let Some(next) = st.next_canon_id.checked_add(1) else {
            tracing::error!("canon: id allocator exhausted; retcon refused without mutation");
            return None;
        };
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
        let prepared = match st.ledger.prepare_append(LOG, value) {
            Ok(prepared) => prepared,
            Err(error) => {
                tracing::error!(%error, "canon: unsafe retcon ledger append refused without mutation");
                return None;
            }
        };
        let (_seq, op) = st
            .ledger
            .apply_prepared(prepared)
            .expect("canon ledger cannot change during one exclusive state borrow");
        st.next_canon_id = next;
        (id, next, op)
    };
    push_op(ctx.out, op);
    persist_meta(ctx.out, next);
    Some(id)
}
