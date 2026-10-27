//! Closed renewal evidence. Decoded values convey no entitlement or payment.
//!
//! Historical birth, immutable granted generations, and the current pointer
//! have separate checks. Only the paired owner writer can advance the pointer.

pub(crate) mod audit;
mod chain;
pub(crate) mod history;
pub mod protocol;
pub mod reader;
mod records;
pub(crate) mod roots;

pub use records::{
    RenewalAttemptV1, RenewalBindingV1, RenewalClosureV1, RenewalHeadV1, RenewalOperationV1,
    RenewalStateV1,
};

#[cfg(test)]
pub(crate) mod tests;

pub mod writer;

mod capacity;
