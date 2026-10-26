//! Closed renewal of an actually issued personal House licence.
//! Birth, current pointer and retained terminal histories are distinct proofs.
mod chain;
pub(crate) mod history;
pub mod protocol;
mod records;
pub use records::{
    RenewalAttemptV1, RenewalBindingV1, RenewalClosureV1, RenewalHeadV1, RenewalOperationV1,
    RenewalStateV1,
};

pub(crate) mod audit;
mod capacity;
pub mod reader;
pub(crate) mod roots;
pub mod writer;

#[cfg(test)]
mod tests;
