//! Cold-process custody, not a worker outcome. Only the real bulk hydration
//! path captures these exact acknowledged bindings, while both newly-created
//! worker lanes have never accepted any work. No elapsed-time inference exists.

use std::collections::{BTreeSet, VecDeque};

use super::{BulkError, invalid, owner::BulkStore, registry_tickets::Binding};

// Absolute configured admission ceiling: eight workers plus 128 queued jobs.
// A lower configuration after restart must still be able to clean up the old
// admitted set. Recovery retains no Machine, source, artifact or decoded plan.
pub(super) const MAX_STOPPED_JOBS: usize =
    mush_roll::commerce::rating::protocol::MAX_STOPPED_REGISTRY_JOBS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Unavailable {
    MissingActor,
    NotPlayer,
    GoingActor,
    MissingAccount,
}
pub(super) fn unavailable(world: &mush_core::World, binding: Binding) -> Option<Unavailable> {
    let Some(actor) = world.get(binding.character) else {
        return Some(Unavailable::MissingActor);
    };
    if actor.flags.contains(mush_core::flags::Flags::GOING) {
        return Some(Unavailable::GoingActor);
    }
    if actor.kind != mush_core::ObjKind::Player {
        return Some(Unavailable::NotPlayer);
    }
    if super::world::account(world, binding.character).is_err() {
        return Some(Unavailable::MissingAccount);
    }
    // A reassigned actor's current account is not the cleanup payer. The
    // original persisted account/job binding remains the sole quota authority.
    None
}

pub(super) struct StoppedBinding {
    binding: Binding,
}
impl StoppedBinding {
    pub(super) fn binding(&self) -> Binding {
        self.binding
    }
}
#[derive(Default)]
pub(super) struct Recovery {
    begun: bool,
    loaded: bool,
    pending: VecDeque<StoppedBinding>,
}
impl Recovery {
    pub(super) fn begin(&mut self) -> Result<(), BulkError> {
        if self.begun {
            return Err(invalid("bulk hydration already began"));
        }
        self.begun = true;
        Ok(())
    }
    pub(super) fn capture(
        &mut self,
        _: super::world::ColdHydration,
        owner: &BulkStore,
    ) -> Result<(), BulkError> {
        if !self.begun || self.loaded {
            return Err(invalid("Registry cold hydration phase"));
        }
        let mut pending = VecDeque::new();
        let mut jobs = BTreeSet::new();
        for binding in owner.stopped_registry_bindings() {
            if pending.len() >= MAX_STOPPED_JOBS || !jobs.insert(binding.job) {
                return Err(invalid("Registry cold job count or duplicate identity"));
            }
            pending.push_back(StoppedBinding { binding });
        }
        self.pending = pending;
        self.loaded = true;
        Ok(())
    }
    pub(super) fn pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub(super) fn take_available(
        &mut self,
        available: impl Fn(Binding) -> bool,
    ) -> Option<StoppedBinding> {
        let at = self
            .pending
            .iter()
            .position(|stopped| available(stopped.binding))?;
        self.pending.remove(at)
    }
}
