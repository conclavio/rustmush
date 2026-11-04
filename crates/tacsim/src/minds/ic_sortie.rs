//! Dedicated IC execution and custody. This is not an OOC match or marker.
//! The region host's actual transition producer remains a separate prerequisite.
mod audit;
pub(crate) mod boot;
mod execution;
mod host;
mod participant;
pub(crate) mod protocol;
mod provider;
mod records;
pub(crate) mod registration;
mod state;

pub use host::coverage::{
    AcknowledgedDeparture, CoveredDeparture, WithheldDeparture, WithheldOpen, WithheldUnadmitted,
};
pub(crate) use host::coverage::{
    CommittedCoverageClose, audit_history as audit_coverage_history,
    audit_qualified_history as audit_qualified_coverage_history,
};
pub use host::{
    IcHost, IcHostCompletion, IcHostTicket, IcRetainedExecution, IcSubmissionState,
    IcUnadmittedOpen,
};
pub use participant::{CloseEvent, IcSortieParticipant, StartPermit};
pub use state::IcSorties;

/// **Whether the IC claim path is composed in `world`** (doc 23 cut S.4b-1): the seal owner's
/// book is registered only by an Active mind-commerce composition `with_ic_minds`. Doc 23
/// D-23-22's refusal at `go` reads it; S.4b-2's claim arm opens claims only where it holds.
#[must_use]
pub fn claim_path_composed(world: &mush_core::World) -> bool {
    world.module_state::<IcSorties>().is_some()
}

#[cfg(test)]
pub(crate) use participant::{
    fixture_close, fixture_covered_close, fixture_covered_loss, fixture_start, fixture_start_ic,
    fixture_withheld,
};
