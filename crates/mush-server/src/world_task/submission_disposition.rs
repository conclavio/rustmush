//! Server-private completion authority for observed command attempts.
//!
//! The read-only core handle cannot reach any of these types. A vacant permit
//! is consumed by the same coordinator call; a staged refusal requires the
//! writer's exact Absent disposition and a successful abort of its installation.

use super::{
    ActionCoordinator, ActionPermit, AuthoritativeCommitFailure, StagedAuthoritativeAction,
};
use mush_core::submission_observation::{
    SubmissionDispositionReader, SubmissionRuntime, SubmissionRuntimeState,
};
use mush_core::{AuthoritativeSubmit, CanonicalHash, CommitToken, Engine, IngressEnvelope};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct Runtime;
struct ReadState(AtomicBool);
impl SubmissionDispositionReader for ReadState {
    fn no_intent(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
impl SubmissionRuntime for Runtime {
    fn observe(&self, request: &AuthoritativeSubmit) -> SubmissionRuntimeState {
        let state = Arc::new(ReadState(AtomicBool::new(false)));
        let attempt = Attempt {
            state: Arc::clone(&state),
            // Epoch is assigned by the actual driver. All remaining canonical
            // ingress facts are already exact and bounded by this constructor.
            expected: ActionCoordinator::command_ingress(request, 0).ok(),
        };
        SubmissionRuntimeState::new(state, Box::new(attempt))
    }
}

pub(super) struct Attempt {
    state: Arc<ReadState>,
    expected: Option<IngressEnvelope>,
}
impl Attempt {
    pub(super) fn from_driver(driver: Option<Box<dyn std::any::Any + Send>>) -> Option<Self> {
        driver
            .and_then(|value| value.downcast::<Self>().ok())
            .map(|value| *value)
    }
    pub(super) fn vacant(self, permit: &ActionPermit) -> Result<VacantAttempt, String> {
        let expected = self.expected.ok_or("observed selector was not canonical")?;
        let actual = permit.ingress();
        if expected.request() != actual.request()
            || expected.principal() != actual.principal()
            || expected.ingress_hash() != actual.ingress_hash()
            || expected.idempotency().scope() != actual.idempotency().scope()
            || expected.idempotency().key() != actual.idempotency().key()
        {
            return Err("observed submission differs from the vacant ingress".into());
        }
        Ok(VacantAttempt { state: self.state })
    }
}

pub(super) struct VacantAttempt {
    state: Arc<ReadState>,
}
impl VacantAttempt {
    pub(super) fn refuse_unstaged(self, engine: &Engine) {
        // A provider error is not enough if a wiring error left an installation
        // staged. Retain custody in that case; no success or timeout inference.
        if !engine.has_staged_action() {
            self.state.0.store(true, Ordering::Release);
        }
    }
    pub(super) fn staged(self, action: &StagedAuthoritativeAction) -> StagedAttempt {
        StagedAttempt {
            state: self.state,
            token: action.token,
            hash: action.bundle_hash,
        }
    }
}

pub(super) struct StagedAttempt {
    state: Arc<ReadState>,
    token: CommitToken,
    hash: CanonicalHash,
}
impl StagedAttempt {
    /// `failure` came from the exact PendingReceipt; Refused is produced by the
    /// writer only with an actual BundleStatus::Absent witness. The installation
    /// and expected identity are taken from that same PendingCommit.
    pub(super) fn abort_refused(
        self,
        engine: &mut Engine,
        installation: mush_core::composed::ComposedInstallation,
        failure: &AuthoritativeCommitFailure,
        identity: Option<(CommitToken, CanonicalHash)>,
    ) -> Result<(), String> {
        if !matches!(failure, AuthoritativeCommitFailure::Refused(_))
            || identity != Some((self.token, self.hash))
        {
            return Err("observed refusal does not match the exact pending installation".into());
        }
        engine
            .abort_prepared_action(installation)
            .map_err(|error| error.to_string())?;
        self.state.0.store(true, Ordering::Release);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
