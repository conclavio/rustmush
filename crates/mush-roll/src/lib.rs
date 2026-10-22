//! Character-owned authoring workflow records (doc 17d M6.1).
//!
//! This crate owns drafts, immutable revision envelopes, selections and explicit
//! retention records. Game semantics and source storage belong to caller adapters.
//! Commercial values and closed first-filing owner adapters live in [`commerce`].
//! Data decoding never grants rights. Only actual registered owner composition
//! can activate filing. The closed IC writer consumes concrete Tactics and Sim
//! owner outputs; it has no Tacsim engine, interpreter or Market dependency.
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

// The deny above has no test exemption for `panic_in_result_fn`, so a test or
// fixture helper that returns `Result` checks with `check!`/`check_eq!`/
// `check_ne!` — its asserts as early `Err` returns. Declared before every other
// module because `macro_rules!` is textually scoped.
#[cfg(test)]
#[macro_use]
mod test_checks;

pub mod commerce;
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
