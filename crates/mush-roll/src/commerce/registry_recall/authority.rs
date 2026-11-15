//! Actual scoped authorization, repeated before custody. Witnesses are attribution.
//!
//! One text for both Registry sanctions (`registry_twin::authority`, doc 46 CT.9), at this
//! family's issuer.
use super::super::registry_twin::{ExactRecall, authority};
#[cfg(test)]
pub(crate) use authority::Source;
pub(crate) use authority::{Authorization, observe, recheck};
pub(crate) type Readiness = authority::Readiness<ExactRecall>;
