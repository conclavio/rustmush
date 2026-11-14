//! The Roll's renewal half, written once for both issuers (doc 46, CT.5).
//!
//! A mind licence is renewed by one of two families: the personal one
//! (`commerce::renewal`) and the House one (`commerce::house_renewal`). Their records, chain,
//! history, roots, capacity, protocol, audit and writer were two copies of the same text.
//! Everything they share is here, generic over the sealed [`Issuer`] and its crate-private
//! rules (`Rules`); what an issuer alone decides (its binding's shape, its sale reader, the
//! accounts a renewal pairs, the House's buyer admission, which of the closed lists' entries
//! are its own) is its implementation of those traits, in its family's `mod.rs`. Each family
//! keeps its own reader (`reader.rs`, real logic) and its own `checked!` records, and its
//! module tree is the instantiation point: every path a caller or another crate names is
//! still there.
//!
//! The issuer types are Economy's own (`mush_economy::renewal_twin::{Personal, House}`): the
//! Roll's renewal is paired with Economy's owner, so a renewal family has one issuer type
//! across the two crates, and the Roll's trait is a subtrait of Economy's.

mod issuer;
pub(crate) use issuer::Rules;
pub use issuer::{Binding, Closure, Facts, Issuer, Operation, TermEconomics};
pub use mush_economy::renewal_twin::{House, Personal};

pub(crate) mod audit;
pub(crate) mod capacity;
pub(crate) mod chain;
pub(crate) mod history;
pub mod protocol;
pub mod records;
pub(crate) mod roots;
pub mod writer;

#[cfg(test)]
pub(crate) mod tests;
