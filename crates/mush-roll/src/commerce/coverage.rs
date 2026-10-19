//! Closed underwriting observations and permanent retention. These projections
//! never mint an IC departure, a role result, a policy or a physical loss.
pub(crate) mod audit;
mod protocol;
pub(crate) mod reader;
pub(crate) mod records;
pub(crate) mod roots;
mod writer;

pub use protocol::{ACTIONS as COVERAGE_ACTIONS, CoverageProtocol};
pub use reader::{CoverageOpenFacts, CoverageOpenReader, CoverageOpenRequest};
pub use records::CoverageRetention;
pub use roots::CoveragePrecommitCheck;
pub use writer::{
    CoverageWriteResult, CoverageWriter, PreparedCoverageOpen, RegisteredCoverageWriter,
};
