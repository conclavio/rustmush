//! **The probe a close-only objective reads** (doc 50 Q-50-57 item 3; Q-50-21, Q-50-28).
//!
//! An [`crate::ObjectiveKind::ProbeAtClose`] objective is answered by its owner, not by this
//! crate: for the Isidis operation, later, the tank farm's level against its threshold. The owner
//! is a ruleset this crate takes no edge to, so the answer arrives through a composition seam on
//! [`crate::CampaignModule::with_formation_kinds`]' shape: the composition root hands in one
//! [`CloseProbe`], which it builds over the owner's public API.
//!
//! The tick asks it once, when the window has closed, and never before. An answer of `None`
//! (no probe composed, a name the owner does not know, or an owner that cannot tell yet) is
//! a wait: the operation does not resolve, a person is paged once, and the next fire asks
//! again. Reading a missing answer as "not met" would hand a broken probe's operation to
//! whichever side "not met" favors.

use std::sync::Arc;

use mush_core::World;

use crate::ids::OperationId;

/// **The owner's answer to a close-only objective.** `Some(true)` is met, `Some(false)` is not,
/// and `None` is "cannot tell", which the tick treats as a wait.
pub trait CloseProbe: Send + Sync {
    /// Read `probe` for `operation`, now (the window has closed).
    fn read(&self, w: &World, probe: &str, operation: OperationId) -> Option<bool>;
}

/// **The probe's seat in world state** (this module's head): the composition root's
/// [`CloseProbe`], or nothing.
#[derive(Clone, Default)]
pub struct CloseProbes(Option<Arc<dyn CloseProbe>>);

impl CloseProbes {
    /// The seam, wired.
    #[must_use]
    pub fn new(probe: Arc<dyn CloseProbe>) -> CloseProbes {
        CloseProbes(Some(probe))
    }

    /// Ask the composed probe. Unwired is `None`, which is a wait.
    #[must_use]
    pub fn read(&self, w: &World, probe: &str, operation: OperationId) -> Option<bool> {
        self.0
            .as_ref()
            .and_then(|seam| seam.read(w, probe, operation))
    }
}

impl std::fmt::Debug for CloseProbes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("CloseProbes")
            .field(&self.0.is_some())
            .finish()
    }
}
