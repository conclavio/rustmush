//! Actual role-trial publication. Wire records are historical structural data;
//! only the complete owner/typed/source-root join can mint a ConductTrialProof.
use super::records::PublicationError;
use mush_core::DefinitionDigest;
use serde::Serialize;

pub(crate) mod families;
pub(crate) mod history;
pub(crate) mod producer;
pub mod registration;
pub use registration::ConductConfig;
pub mod protocol;
pub(crate) mod publication;
pub(crate) mod reader;
pub(crate) mod state;
pub(crate) use history::metadata;
pub use reader::{ConductReadRequest, ConductReader, ConductTrialProof};
pub(crate) mod work_read;
pub use work_read::{
    ConductCandidate, ConductWorkRead, ConductWorkReader, ConductWorkRequest, ConductWorkState,
};
pub mod records;
pub use records::{ConductRole, ConductSubject};

pub const JOB_BYTES: usize = 8 * 1024;
pub const EXECUTION_BYTES: usize = 4 * 1024;
pub const RESULT_BYTES: usize = 4 * 1024;
pub const MAX_TRIALS: usize = 4096;
pub const MAX_RESULT_ARTIFACT_BYTES: u64 = 256 * 1024;
pub const REPETITIONS: u16 = 60;
pub const MAXIMUM_SIMULATION_MS: u64 = 120_000;

fn invalid(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Invalid
}
pub(crate) fn digest<T: Serialize>(
    domain: &'static str,
    value: &T,
    limit: usize,
) -> Result<DefinitionDigest, PublicationError> {
    DefinitionDigest::for_canonical_value_with_limit(
        domain,
        &serde_json::to_value(value).map_err(invalid)?,
        limit,
    )
    .map_err(|_| PublicationError::Capacity)
}
fn bounded<T: Serialize>(value: &T, limit: usize) -> Result<(), PublicationError> {
    mush_core::canonical_json_bytes_with_limit(
        &serde_json::to_value(value).map_err(invalid)?,
        limit,
    )
    .map(|_| ())
    .map_err(|_| PublicationError::Capacity)
}
