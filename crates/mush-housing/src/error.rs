//! [`HousingError`] — every refusal this crate can produce (E5a).
//!
//! **Every forbidden transition is a typed error.** Nothing in this crate panics on
//! record data, clamps a state machine silently, or returns a bare `bool` for a
//! refusal: a caller that asks for an arrow the tenancy does not have gets a variant
//! naming why, so E5b can render one message per reason and a test can pin the
//! difference between "not yet" ([`HousingError::NotDue`]) and "never"
//! ([`HousingError::State`]).

use mush_core::ObjId;

use crate::ids::TenancyId;

/// Why a housing operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HousingError {
    /// The tenancy is not in a state that admits this arrow (the `ContractError::State`
    /// shape).
    #[error("tenancy {0} is not in a state that allows that")]
    State(TenancyId),
    /// The tenancy has ended. **Nothing** moves an ended tenancy — not a period
    /// assessment, not a guest edit, not a second resolution (D-E5-1 runs both ways:
    /// a home never ends on its own, and an ended one never quietly reopens).
    #[error("tenancy {0} has already ended")]
    AlreadyEnded(TenancyId),
    /// Not every party accepted: the landlord, or one of the named tenants.
    #[error("the tenancy has not been accepted by every party")]
    NotAccepted,
    /// The arrow exists but its period count has not elapsed yet (`GRACE_PERIODS`,
    /// `DELINQUENT_PERIODS`). Distinct from [`HousingError::State`] on purpose: "not
    /// yet" is a retry, "never" is a bug.
    #[error("that transition is not due yet")]
    NotDue,
    /// A story-window operation on a tenancy that has no window open.
    #[error("tenancy {0} has no story window open")]
    NoWindow(TenancyId),
    /// The story window wants its level-2 escalation opened, and the caller supplied
    /// no escalation id. Refused rather than skipped: silently dropping the
    /// escalation is how a paged decision becomes a home nobody ever looks at.
    #[error("the story window needs its level-2 escalation record")]
    NeedsEscalation,
    /// A cap was exceeded (§9).
    #[error("housing record exceeds a cap: {0}")]
    TooLarge(&'static str),
    /// An operation named somebody who is not a tenant of this tenancy.
    #[error("#{0} is not a tenant of that tenancy")]
    NotATenant(ObjId),
    /// An abstract operation on an exact tenancy, or the reverse.
    #[error("that operation does not apply to this tenancy's funding")]
    WrongFunding,
    /// A shape rule this record cannot satisfy.
    #[error("invalid housing record: {0}")]
    Invalid(&'static str),
    /// Checked arithmetic overflowed. Period counts are `i64` positions on a cadence
    /// clock, and a corrupt document can hold any two of them.
    #[error("arithmetic overflow")]
    Overflow,
}
