//! Versioned immutable monetary consent. These values allocate nothing and
//! prove neither current rights nor payment. Only the closed owner producer
//! may join actual source/term/E4 history and publish them.

mod economics;
pub(crate) mod history;
mod records;
pub use economics::*;
pub use records::EntitlementAllocationV1;

pub(crate) mod royalties;

pub mod reader;

pub(crate) mod audit;

pub(crate) mod roots;

pub mod customer;
pub mod writer;

/// Doc 44 MW.2: the listing-eligibility read's public types (the read itself
/// is `writer::EntitlementPurchaseWriter::sellable`).
pub use super::withdrawal::sellable::{
    ListedSubject, SELLABLE_READS, SELLABLE_ROWS_DERIVED, SELLABLE_ROWS_ORIGINAL,
    SELLABLE_SUBJECTS, Sellable, Unsellable,
};

pub(crate) mod admission_capacity;
