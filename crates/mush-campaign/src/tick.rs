//! The campaign tick (E7 cut 9 B6) — **a pull, never a subscription**.
//!
//! ## Why it cannot be anything else
//!
//! The kernel fact cut 6 established: a `schedule_source`-fired action runs inside
//! `prepare_authoritative` with **no `&World`, no `Ctx` and no `Outbox`**, writes
//! only typed `state_family` records through an `ActionTxn`, cannot emit a
//! `WorldEvent`, and there is **no world-side seam on a scheduled authoritative
//! commit**. So every campaign reaction to durable state is a `Coarse` pull-tick on
//! `crates/mush-housing/src/tick.rs`'s exact shape — never an observer on a family
//! change, never a `schedule_source`, never a `WorldEvent` subscription.
//!
//! ## What one fire may do
//!
//! - **visit** at most [`MAX_VISITED_PER_FIRE`] operations — touched at all, due or
//!   not — and **work** at most [`MAX_WORKED_PER_FIRE`] of them, the two budgets
//!   enforced **independently**, from a **rotation cursor** that advances past the
//!   last *visited* id, skipped ones included (housing's D-LBK-2 correction: a
//!   cursor that moved only on worked records made "bounded per fire" true of the
//!   work and false of the scan);
//! - advance **at most one consumer per operation per fire**, each idempotent and
//!   keyed `campaign.resolve/{op}/{consumer}`;
//! - **skip a paused operation entirely.** Only the subject is stopped, so one
//!   operation's wall of refusals cannot starve another's fan-out (08c §5.2 B4).
//!
//! A re-run over the same documents is a **no-op past the applied point**, so a
//! reboot re-running the tick *is* the reconcile and there is no crash gap.
//!
//! ## The tick-cadence rule (cut 8's verify fold)
//!
//! `Ctx::submit_authoritative` is one per actor per deferred drain and every
//! `reg.tick` runs as GOD, so two tick submissions in one drain fan out and void
//! both. **This tick makes no authoritative submission at all** — it writes exempt
//! workflow documents, posts to the economy on the economy's own carve-out lane,
//! and commits canon — so it cannot collide with cut 8's 1 s `sim.domain_pulse` or
//! cut 7's 2 s `propagation.detection` however the fires line up. A source probe
//! pins that (`e7c9_p6`), and a two-fire test pins that the second fire stages
//! rather than repeating a key or freezing.

use mush_canon::CanonState;
use mush_core::{ClockStamp, Ctx, SceneId};

use crate::ids::OperationId;
use crate::record::{ObjectiveKind, OperationDoc, OperationState, PauseReason};
use crate::settle;
use crate::store::{CampaignStore, TickStats, persist_meta, put_operation};

/// The one tick tag this crate registers.
pub const TICK_TAG: &str = "campaign.operations";
/// Its period. Deliberately distinct from every other registered cadence so an
/// operator reading `@ps` can tell whose fire they are looking at.
pub const TICK_PERIOD_SECS: u64 = 15;
/// How many operations one fire may touch at all.
pub const MAX_VISITED_PER_FIRE: usize = 64;
/// How many of them it may work.
pub const MAX_WORKED_PER_FIRE: usize = 16;

/// What one operation cost this fire.
enum Step {
    /// Nothing to do: resolved, paused, or not yet due.
    Skipped,
    /// One transition, or one consumer.
    Worked,
    /// It was paused this fire.
    Paused,
}

/// One fire.
pub(crate) fn tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    let mut stats = TickStats::default();
    let (ids, cursor) = {
        let Some(store) = ctx.w.module_state::<CampaignStore>() else {
            return;
        };
        (store.operation_ids(), store.cursor)
    };
    if ids.is_empty() {
        if let Some(store) = ctx.w.module_state_mut::<CampaignStore>() {
            store.last_fire = stats;
        }
        return;
    }
    // The rotation: start at the first id at or past the cursor, wrap once.
    let start = ids.iter().position(|id| id.0 >= cursor).unwrap_or(0);
    let order: Vec<OperationId> = ids[start..]
        .iter()
        .chain(ids[..start].iter())
        .copied()
        .collect();

    let mut last = None;
    for id in order {
        // Both budgets, checked independently, before the visit is counted.
        if stats.visited >= MAX_VISITED_PER_FIRE || stats.worked >= MAX_WORKED_PER_FIRE {
            break;
        }
        stats.visited = stats.visited.saturating_add(1);
        last = Some(id);
        match step(ctx, id, &mut stats) {
            Step::Skipped => {}
            Step::Worked => stats.worked = stats.worked.saturating_add(1),
            Step::Paused => {
                stats.paused = stats.paused.saturating_add(1);
                stats.worked = stats.worked.saturating_add(1);
            }
        }
    }
    let Some(store) = ctx.w.module_state_mut::<CampaignStore>() else {
        return;
    };
    // Resume **past** the last id this fire visited — skipped ones included — so
    // the next fire starts with whoever was cut off rather than at the top again.
    if let Some(last) = last {
        store.cursor = last.0.saturating_add(1);
    }
    store.last_fire = stats;
    if last.is_some() {
        persist_meta(ctx);
    }
}

/// Read the world IC clock as a stamp on its own generation.
fn ic_now(ctx: &Ctx<'_>) -> ClockStamp {
    ctx.w.world_timeline().stamp_at(ctx.w.now())
}

fn step(ctx: &mut Ctx<'_>, id: OperationId, stats: &mut TickStats) -> Step {
    // **Ask what it is before cloning it.** Status and pause are read through the
    // borrow; the document is cloned only once it is known to be workable.
    let (live, paused, resolving) = {
        let Some(store) = ctx.w.module_state::<CampaignStore>() else {
            return Step::Skipped;
        };
        let Some(doc) = store.operation(id) else {
            return Step::Skipped;
        };
        (
            doc.runtime.state.is_live(),
            doc.runtime.paused.is_some(),
            matches!(doc.runtime.state, OperationState::Resolving { .. }),
        )
    };
    if !live {
        return Step::Skipped;
    }
    if paused {
        // Only the subject is stopped: the fire moves on to the next operation, so
        // a permanently failing consumer cannot starve anybody else's fan-out.
        return Step::Skipped;
    }
    if resolving {
        // At most ONE consumer per operation per fire.
        return match settle::advance_one(ctx, id) {
            Some(_) => {
                stats.consumers = stats.consumers.saturating_add(1);
                Step::Worked
            }
            None => Step::Skipped,
        };
    }
    let Some(doc) = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.operation(id).cloned())
    else {
        return Step::Skipped;
    };
    // (ii) the funding check, before anything else moves: an operation whose bond
    // has gone away pauses at its next phase boundary and pages. Never negative,
    // never continuing unfunded.
    if let Some(bond) = &doc.terms.bond
        && bond.hold.is_some()
        && crate::bond::live_hold(ctx, id).is_none()
    {
        settle::pause(
            ctx,
            id,
            PauseReason::Unfunded,
            &format!(
                "operation {id}'s bond is no longer held, so it cannot continue funded \
                 and will not advance. The decision is whether it ends: \
                 `@op/resolve {id}=abort` settles it and releases its units, and a fresh \
                 `@op/declare` is how it runs again — clearing the pause alone does not \
                 put the reservation back"
            ),
        );
        return Step::Paused;
    }
    let now = ic_now(ctx);
    // Two stamps are only ever compared on the same clock. A window on a retired
    // generation is FROZEN and paged, never silently compared as bare integers.
    let Ok(against_open) = now.checked_cmp(&doc.terms.window.0) else {
        settle::page(
            ctx,
            smol_str::SmolStr::new(format!("op:{}:clock", id.0)),
            &format!("operation {id}'s window is on a retired clock generation"),
            &format!(
                "operation {id} was declared on a world-IC generation that no longer runs, so \
                 its window cannot be compared. Inspect with `@op/show {id}` and abandon with \
                 `@op/resolve {id}=abort`"
            ),
        );
        return Step::Skipped;
    };
    let opened = against_open.is_ge();
    let closed = now
        .checked_cmp(&doc.terms.window.1)
        .is_ok_and(std::cmp::Ordering::is_ge);

    // (ii) the notice-window sweep.
    if matches!(doc.runtime.state, OperationState::Activated) && opened {
        let mut doc = doc;
        doc.runtime.state = OperationState::Running;
        doc.runtime.revision = doc.runtime.revision.saturating_add(1);
        put_operation(ctx, doc);
        return Step::Worked;
    }
    // (i) the objective, evaluated against DURABLE state — a pull.
    if matches!(doc.runtime.state, OperationState::Running) {
        let met = objective_met(ctx, &doc, closed);
        if met {
            return match settle::enter_resolving(ctx, id, "success") {
                true => Step::Worked,
                false => Step::Paused,
            };
        }
        if closed {
            return match settle::enter_resolving(ctx, id, "failure") {
                true => Step::Worked,
                false => Step::Paused,
            };
        }
    }
    Step::Skipped
}

/// Evaluate one operation's objective against durable state. Exhaustive, total, and
/// free of `?`, `unwrap` and panicking indexing.
#[must_use]
pub(crate) fn objective_met(ctx: &Ctx<'_>, doc: &OperationDoc, window_closed: bool) -> bool {
    match &doc.terms.objective.kind {
        ObjectiveKind::WindowElapsed => window_closed,
        ObjectiveKind::Muster { minimum } => {
            window_closed && doc.runtime.participants.len() >= usize::from(*minimum)
        }
        ObjectiveKind::CanonRecorded { kind, source } => {
            ctx.w.module_state::<CanonState>().is_some_and(|state| {
                mush_canon::records_for_source(state, source, 16)
                    .iter()
                    .any(|event| event.kind == *kind)
            })
        }
    }
}
