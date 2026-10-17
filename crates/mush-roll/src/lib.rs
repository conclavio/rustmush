//! Character-owned authoring workflow records (doc 17d M6.1).
//!
//! This crate owns drafts, immutable revision envelopes, selections and explicit
//! retention records. Game semantics and source storage belong to caller adapters.
//! It has no simulation, script, market, licence or settlement dependency.
//!
//! Every mutation is prepared, validated and fully serialized before publication.
//! A caller publishes both owners inside one world operation and queues all their
//! writes in that operation's atomic batch. Unreadable records stay on disk and
//! block collection; boot never turns corruption into permission to reclaim data.

// WS01-S12 / 17d M6.1: a new persistence owner starts at zero for every
// panic-shaped restriction lint; the workspace policy gate holds that boundary.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

mod model;
mod owner;

pub use model::*;
pub use owner::*;

/// A bounded refusal, with the offending field or record named.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RollError {
    #[error("{field}: {message}")]
    Invalid { field: String, message: String },
    #[error("No such owned mind or revision.")]
    Missing,
    #[error("The edit base is stale; reload the draft.")]
    StaleEdit,
    #[error("The prepared operation is stale; prepare it again.")]
    StaleOperation,
    #[error("{0} is already in use.")]
    Exists(String),
    #[error("The digest abbreviation is ambiguous.")]
    Ambiguous,
    #[error("Collection is blocked by unreadable records: {0}.")]
    Quarantined(String),
    #[error("The {0} allocator is exhausted.")]
    Exhausted(&'static str),
}

fn invalid(field: impl Into<String>, message: impl Into<String>) -> RollError {
    RollError::Invalid {
        field: field.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
