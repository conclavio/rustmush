//! Safe Conduct's immutable financial terms and clock arithmetic.
//!
//! These values confer no cover, discount eligibility, funding, mechanical-loss
//! authority or claim. They deliberately expose both conditional prices rather
//! than accepting an eligibility boolean. The eventual closed owner service must
//! join exact approved certification and genuine IC evidence, actual title,
//! premium and insurer liability in the acknowledged prepared transition.
//!
//! No default composition activates underwriting. In particular an OOC seal,
//! purchased licence or decoded value here cannot activate it.

mod audit;
mod authority;
pub(crate) mod boot;
mod capacity;
pub(crate) mod families;
mod fence;
mod host;
mod metadata;
mod policy;
mod product;
mod protocol;
mod provider;
mod publication;
mod qualification;
mod records;
pub(crate) mod registration;
mod terms;
mod window;

pub use crate::minds::ic_sortie::{
    AcknowledgedDeparture, CoveredDeparture, WithheldDeparture, WithheldOpen, WithheldUnadmitted,
};
pub use host::{
    CoverageCompletion, CoverageHost, CoverageSubmissionState, CoverageTicket, CoverageUnadmitted,
};
pub use registration::CoverageConfiguration;

pub use product::{
    ApprovedConductV1, CoverageExclusion, CoverageProductV1, LossRule, SafeConductRole,
};
pub use terms::{CoverageAmounts, CoverageIllustration, CoverageTermsV1};
pub use window::CoverageWindow;

/// Fixed diagnostic categories never identify another principal or seal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CoverageError {
    #[error("coverage authority or its current owner state is unavailable")]
    Authority,
    #[error("invalid or unsupported coverage terms")]
    Terms,
    #[error("invalid or unsupported coverage product")]
    Product,
    #[error("coverage value is outside the agreed bounds")]
    Value,
    #[error("coverage encoding exceeds its bound")]
    Bound,
    #[error("coverage arithmetic overflow")]
    Arithmetic,
    #[error("coverage requires the same versioned world-IC clock")]
    Clock,
    #[error("loss timestamp is outside the covered interval")]
    OutsideTerm,
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod product_tests;
