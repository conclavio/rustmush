//! The Registry's two sanctions, as one text: the exact filed-revision recall
//! (`commerce::registry_recall`) and the accepted-term revocation
//! (`commerce::registry_revocation`). Doc 46 CT.9.
//!
//! What the two share is the trusted policy that names a real Registry authority
//! ([`RegistryPolicy`]) and the scoped authorization repeated before custody (`authority`).
//! What each is, is [`Review`]: sealed, with exactly two implementations, [`ExactRecall`] and
//! [`TermRevocation`]. Each implementation is in its family's `policy.rs`, beside the public
//! name the family has always had (`RegistryRecallPolicy`, `RegistryRevocationPolicy`), and
//! holds the family's identity strings with the values its files held.
//!
//! There is no `dyn`, no callback and no registration: the policy and the authority are
//! resolved at compile time over the closed pair. The rest of each family (its records,
//! notices, history, audit and provider) is its own and is not shared.

pub(crate) mod authority;
pub(crate) mod policy;
pub use policy::RegistryPolicy;

use mush_core::World;
use std::fmt::Debug;

/// The seal, and what an issuer decides. Visible to the crate so that each family's own
/// file holds its implementation; nothing outside the crate can name it.
pub(crate) mod seal {
    use super::World;

    /// A Registry sanction's identity and its one check of its own.
    pub trait Sealed {
        /// The id of the policy's authority descriptor.
        const AUTHORITY: &'static str;
        /// The id of the sanction's action descriptor.
        const ACTION: &'static str;
        /// The id of the sanction's outcome descriptor.
        const OUTCOME: &'static str;
        /// The capability the live Org authorizer is asked for.
        const CAPABILITY: &'static str;
        /// The refusal when the registry holds no definitions at readiness.
        const UNAVAILABLE: &'static str;
        /// The refusal when readiness is captured twice.
        const CAPTURED: &'static str;
        /// The definition owners that must be ready, in the order they are asked.
        const OWNERS: &'static [&'static str];
        /// The body of the action descriptor.
        fn action_value() -> serde_json::Value;
        /// Whether a module state the sanction reads, beyond Org's and the Roll's, is not
        /// owned by its owner. Asked where the old file asked it, between the two.
        fn owner_missing(world: &World) -> bool;
    }
}

/// A Registry sanction: [`ExactRecall`] or [`TermRevocation`].
///
/// Sealed: nothing outside the crate can implement it, as its supertrait cannot be named:
///
/// ```compile_fail
/// #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// struct Pardon;
/// impl mush_roll::commerce::registry_twin::Review for Pardon {}
/// ```
pub trait Review: seal::Sealed + Copy + Debug + Default + Eq + Send + Sync + 'static {}

/// The exact filed-revision recall (`commerce::registry_recall`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExactRecall;
impl Review for ExactRecall {}

/// The accepted-term revocation (`commerce::registry_revocation`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TermRevocation;
impl Review for TermRevocation {}

#[cfg(test)]
mod tests;
