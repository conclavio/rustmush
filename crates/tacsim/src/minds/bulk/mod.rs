//! Off-arena author compute: one durable quota owner for proving and bench.
//! Workers receive a persisted bounded lease, never World or the live pool.
pub(crate) mod artifacts;
pub(crate) mod bench;
pub(crate) mod commit;
pub(crate) mod conduct_budget;
mod conduct_history;
mod conduct_index;
pub(crate) mod conduct_participant;
mod conduct_projection;
pub(crate) mod conduct_service;
pub(crate) mod conduct_tickets;
mod encoding;
pub(crate) mod eval;
pub(crate) mod executor;
mod frozen;
mod metrics;
pub(crate) mod model;
pub(crate) mod owner;
pub(crate) mod preparation;
pub(crate) mod proving;
pub(crate) mod publication;
pub(crate) mod query;
pub(crate) mod registry_budget;
pub(crate) mod registry_participant;
mod registry_projection;
mod registry_recovery;
pub(crate) mod registry_service;
pub(crate) mod registry_tickets;
pub(crate) mod replay;
pub(crate) mod report;
pub(crate) mod scenarios;
pub(crate) mod tape_cache;
pub(crate) mod world;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum BulkError {
    #[error("No such owned run or bench.")]
    Missing,
    #[error("The compute record is unreadable; repair is required.")]
    Quarantined,
    #[error("The compute record changed; retry the current action.")]
    Stale,
    #[error("The request was already consumed.")]
    Consumed,
    #[error("An account already has an admitted bulk job.")]
    Busy,
    #[error("The bounded compute queue is full.")]
    QueueFull,
    #[error("The account's UTC daily compute allowance is exhausted.")]
    Quota,
    #[error("Pinned records prevent retaining this result within the configured cap.")]
    Retention,
    #[error("Invalid author compute request: {0}")]
    Invalid(String),
}
fn invalid(text: impl Into<String>) -> BulkError {
    BulkError::Invalid(text.into())
}

mod bench_prepare;
pub(crate) mod bench_service;

pub(super) mod bench_roots;
pub(super) mod experiment;

pub(super) mod branch;
#[cfg(test)]
mod registry_integration_tests;
