//! The economy's outbound event seam (E4c, 08c §6.1 item 3 / §12.4).
//!
//! [`crate::EconomicEvent`] is the pre-C6 form of doc 09's `SemanticEvent`, and it
//! carries **ids and kinds only** (D-E4-13): no amounts, no parties, no `cause`
//! text. The reason is not squeamishness about payload size — it is that an event
//! goes to every observer at once, and observers log. A staff reason string, a
//! payer's account id or a settlement total riding an event leaks through the first
//! observer that writes it to a channel, a canon record, or a trace line. An
//! observer that needs detail asks the economy for it **under the viewer's own
//! authority**, through the disclosure filter, which is the whole point of building
//! that filter in the same cut.
//!
//! ## Post-commit, and what that buys
//!
//! Events are emitted after [`crate::api`]'s `install_commit` and after the durable
//! gate is taken — the transaction is in the journal, the projections are queued on
//! the batch, and nothing fallible remains. An observer therefore never sees a state
//! that can still be rolled back, and an observer that itself mutates the world is
//! adding to a batch that is already committing.
//!
//! ## Re-entrancy
//!
//! An observer holds `&mut Ctx` and *can* post; the canon bridge deliberately does
//! not (it commits canon and nothing else). The depth guard here is insurance for
//! the case a future observer does: the economy sweep is a tick handler, and a
//! recursive observer chain that overflowed the stack would abort the world task
//! rather than refuse one operation. Beyond [`MAX_OBSERVER_DEPTH`] the emission is
//! dropped with a loud log — events are notifications, never authority, so dropping
//! one costs a chronicle line and never money.

use std::sync::Arc;

use mush_core::Ctx;

use crate::module::EconomyStore;
use crate::record::EconomicEvent;

/// How deep a chain of observer-driven economy events may run before the emission
/// is dropped and logged. Four is well past every legitimate chain (a settlement
/// posts, which emits, whose observer records canon, whose observer attests a second
/// contract, which settles) and far short of anything that could exhaust a stack.
pub const MAX_OBSERVER_DEPTH: u32 = 4;

/// Something that wants to hear what the economy did.
///
/// Frozen at wiring ([`crate::EconomyModule::with_observers`]) like every other
/// table in this crate: an observer set that could change at runtime would make
/// "who saw this settlement" a function of when it happened.
pub trait EconomyObserver: Send + Sync + std::fmt::Debug {
    /// Handle one event. **Never panics** (the sweep drives the same emissions from
    /// a tick handler) and never assumes the event is the whole story.
    fn on_economic_event(&self, ctx: &mut Ctx<'_>, ev: &EconomicEvent);
}

/// The wiring-frozen observer table.
pub type ObserverTable = Arc<Vec<Arc<dyn EconomyObserver>>>;

/// Deliver one event to every wired observer, post-commit.
pub(crate) fn emit(ctx: &mut Ctx<'_>, ev: &EconomicEvent) {
    let observers = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return };
        if store.observers.is_empty() {
            return;
        }
        if store.event_depth >= MAX_OBSERVER_DEPTH {
            tracing::error!(
                depth = store.event_depth,
                event = ?ev,
                "economy: observer chain hit its depth cap; the event was dropped"
            );
            return;
        }
        store.event_depth = store.event_depth.saturating_add(1);
        store.observers()
    };
    for o in observers.iter() {
        o.on_economic_event(ctx, ev);
    }
    if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
        store.event_depth = store.event_depth.saturating_sub(1);
    }
}
