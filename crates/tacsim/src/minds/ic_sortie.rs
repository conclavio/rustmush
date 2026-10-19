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

pub use host::{
    HostClose, HostStart, IcHost, IcHostCompletion, IcHostTicket, IcRetainedExecution,
    IcSubmissionState, IcUnadmittedOpen,
};
pub use participant::{
    CloseEvent, IcSortieOutput, IcSortieParticipant, IcSortieRequest, StartPermit,
};
pub use state::IcSorties;

#[cfg(test)]
pub(crate) use participant::{fixture_close, fixture_start};
