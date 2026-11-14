//! **The refit pair's paid leg, written once** (doc 46 CT.7, Q-46-5).
//!
//! `crate::bay_refit` (a component refit in a facility bay, doc 31 UD.4b) and
//! `crate::core_refit` (a certified core fitted into a hull, doc 17h) shared five owner files
//! nearly line for line: the sparse fold delta (`fold`), the prebuilt E4 images (`install`),
//! the live hold, capture and release (`money`), the fixed money-only plan alphabet (`plans`)
//! and the permanent journal evidence (`receipt`). They are here, generic over the sealed
//! [`Leg`], with [`Bay`] and [`Core`] its two implementations. Each family's module keeps its
//! own subject, policy, attributions, configuration, authority checks and boot bridge, the
//! `impl Leg` for its member, and the old paths (`bay_refit::receipt::BayPaymentHistory`,
//! `core_refit::money::Request` and the rest) as aliases, so no caller and no test moved.
//!
//! No persisted byte moved: every record keeps its fields in their order, each identity string
//! is its family's associated constant with the value its file held, and the two records whose
//! serialized field names differ stay each family's own.

mod fold;
pub(crate) mod install;
mod leg;
pub(crate) mod money;
pub(crate) mod plans;
pub(crate) mod receipt;
#[cfg(test)]
pub(crate) mod tests;

pub use leg::{Bay, Core, Leg, Policy, Subject, Supplier};

use crate::{Account, EconomicAsset, InstrumentDef};
use mush_core::ClockStamp;

/// Accept's request: the subject the requester accepts.
pub struct FundingRequest<M: Leg> {
    pub subject: M::Subject,
}

/// The live facts an Accept or an Install was prepared against, re-read at recheck.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurrentFacts<M: Leg> {
    pub accounts: Vec<Account>,
    pub instruments: Vec<InstrumentDef>,
    pub titles: Vec<EconomicAsset>,
    pub supplier: Option<M::Supplier>,
    pub order: Option<M::Order>,
    pub at: ClockStamp,
    pub utc: i64,
}

/// The owner's one refusal, in the leg's own words.
pub(crate) fn refused<M: Leg>() -> mush_core::participant::ParticipantError {
    mush_core::participant::ParticipantError::Refused(M::REFUSED.into())
}
