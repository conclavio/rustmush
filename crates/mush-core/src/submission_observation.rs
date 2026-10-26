//! Volatile, one-attempt observation of the ordinary command submission lane.
//!
//! The module receives no completion capability. Only the Engine's installed
//! runtime supplies the read-only backend; the server keeps its concrete paired
//! state private and completes it from proof-bearing coordinator branches.

use crate::AuthoritativeSubmit;
use std::any::Any;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

const WAITING: u8 = 0;
const DRIVEN: u8 = 1;
const NOT_DISPATCHED: u8 = 2;

/// A terminal observation for one exact attempt, consumed at most once.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmissionDisposition {
    /// This attempt never entered the coordinator. Retry is permitted; this is
    /// not evidence about any earlier attempt using the same durable key.
    NotDispatched,
    /// The runtime proved no durable intent exists for this exact attempt.
    NoIntent,
}

/// The runtime's read-only half. Registration is Engine-only, never a module
/// registrar operation or a caller-supplied callback on an individual submit.
pub trait SubmissionDispositionReader: Send + Sync {
    /// True only after exact coordinator proof. Missing acknowledgement, Drop,
    /// a failed lookup, Pending and Unknown must all remain false.
    fn no_intent(&self) -> bool;
}

/// Paired runtime state for one Engine-minted observation. The concrete driver
/// half may be private to the server crate; modules never receive it.
pub struct SubmissionRuntimeState {
    reader: Arc<dyn SubmissionDispositionReader>,
    driver: Box<dyn Any + Send>,
}

impl SubmissionRuntimeState {
    /// Constructed only by the trusted Engine runtime adapter.
    pub fn new(reader: Arc<dyn SubmissionDispositionReader>, driver: Box<dyn Any + Send>) -> Self {
        Self { reader, driver }
    }
}

/// Trusted embedding runtime, installed outside all module registration APIs.
pub trait SubmissionRuntime: Send + Sync {
    /// Bind the private completion state to this exact immutable selector.
    fn observe(&self, request: &AuthoritativeSubmit) -> SubmissionRuntimeState;
}

struct Cell {
    phase: AtomicU8,
    reader: OnceLock<Arc<dyn SubmissionDispositionReader>>,
}

/// Read-only, non-cloneable and non-serializable handle retained by the module.
/// `None` is unresolved, including accepted work with a missing/dropped reply.
pub struct SubmissionObservation {
    cell: Arc<Cell>,
    consumed: bool,
}

impl SubmissionObservation {
    /// Consume a terminal fact before starting another attempt. An old handle
    /// can never report a second refusal for a newer retry.
    pub fn take_terminal(&mut self) -> Option<SubmissionDisposition> {
        if self.consumed {
            return None;
        }
        let result = match self.cell.phase.load(Ordering::Acquire) {
            NOT_DISPATCHED => SubmissionDisposition::NotDispatched,
            DRIVEN
                if self
                    .cell
                    .reader
                    .get()
                    .is_some_and(|reader| reader.no_intent()) =>
            {
                SubmissionDisposition::NoIntent
            }
            _ => return None,
        };
        self.consumed = true;
        Some(result)
    }
}

impl std::fmt::Debug for SubmissionObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubmissionObservation")
            .field("consumed", &self.consumed)
            .finish_non_exhaustive()
    }
}

struct DispatchGuard(Arc<Cell>);
impl Drop for DispatchGuard {
    fn drop(&mut self) {
        // Deferred is Clone for existing Outbox inspection. Only the last copy
        // may report that this attempt never entered the driver, and at most
        // one copy can claim the driver transition below.
        let _ = self.0.phase.compare_exchange(
            WAITING,
            NOT_DISPATCHED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

/// Opaque deferred selector. Fields/construction are core-private; cloning
/// existing Deferred values cannot manufacture a second observed attempt.
#[derive(Clone)]
pub struct ObservedSubmission {
    request: Box<AuthoritativeSubmit>,
    guard: Arc<DispatchGuard>,
}

impl std::fmt::Debug for ObservedSubmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservedSubmission")
            .field("actor", &self.request.actor)
            .finish_non_exhaustive()
    }
}
impl PartialEq for ObservedSubmission {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.guard, &other.guard) && self.request == other.request
    }
}

impl ObservedSubmission {
    pub(crate) fn new(request: Box<AuthoritativeSubmit>) -> (Self, SubmissionObservation) {
        let cell = Arc::new(Cell {
            phase: AtomicU8::new(WAITING),
            reader: OnceLock::new(),
        });
        let observation = SubmissionObservation {
            cell: Arc::clone(&cell),
            consumed: false,
        };
        (
            Self {
                request,
                guard: Arc::new(DispatchGuard(cell)),
            },
            observation,
        )
    }
    pub(crate) fn actor(&self) -> crate::ObjId {
        self.request.actor
    }
}

pub(crate) enum QueuedSubmission {
    Plain(Box<AuthoritativeSubmit>),
    Observed(ObservedSubmission),
    Retry(DrivenSubmission),
}
impl QueuedSubmission {
    pub(crate) const fn is_retry(&self) -> bool {
        matches!(self, Self::Retry(_))
    }

    pub(crate) fn actor(&self) -> crate::ObjId {
        match self {
            Self::Plain(request) => request.actor,
            Self::Observed(request) => request.actor(),
            Self::Retry(driven) => driven.request.actor,
        }
    }
    pub(crate) fn drive(
        self,
        runtime: Option<&dyn SubmissionRuntime>,
        placement: Arc<()>,
    ) -> Option<DrivenSubmission> {
        match self {
            Self::Plain(request) => Some(DrivenSubmission {
                request,
                driver: None,
                placement,
                observed: false,
            }),
            Self::Retry(driven) => Some(driven),
            Self::Observed(request) => {
                if request
                    .guard
                    .0
                    .phase
                    .compare_exchange(WAITING, DRIVEN, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    return None;
                }
                let driver = runtime.map(|runtime| {
                    let state = runtime.observe(&request.request);
                    // One successful WAITING -> DRIVEN transition per cell.
                    let _ = request.guard.0.reader.set(state.reader);
                    state.driver
                });
                Some(DrivenSubmission {
                    request: request.request,
                    driver,
                    placement,
                    observed: true,
                })
            }
        }
    }
}

/// Runtime-only handoff from the Engine queue. No module context exposes this
/// value or the concrete paired completion state.
pub struct DrivenSubmission {
    request: Box<AuthoritativeSubmit>,
    driver: Option<Box<dyn Any + Send>>,
    placement: Arc<()>,
    observed: bool,
}
impl DrivenSubmission {
    /// The immutable selector retained through a read-only preflight retry.
    #[must_use]
    pub fn request(&self) -> &AuthoritativeSubmit {
        &self.request
    }

    /// Plain legacy submissions keep their existing presentation-only refusal.
    #[must_use]
    pub const fn is_observed(&self) -> bool {
        self.observed
    }

    pub(crate) fn belongs_to(&self, placement: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.placement, placement)
    }

    /// The embedding driver downcasts only its own private completion type.
    pub fn into_parts(self) -> (Box<AuthoritativeSubmit>, Option<Box<dyn Any + Send>>) {
        (self.request, self.driver)
    }
}

#[cfg(test)]
mod tests;
