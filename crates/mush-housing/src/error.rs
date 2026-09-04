//! [`HousingError`] — every refusal this crate can produce (E5a).
//!
//! **Every forbidden transition is a typed error.** Nothing in this crate panics on
//! record data, clamps a state machine silently, or returns a bare `bool` for a
//! refusal: a caller that asks for an arrow the tenancy does not have gets a variant
//! naming why, so E5b can render one message per reason and a test can pin the
//! difference between "not yet" ([`HousingError::NotDue`]) and "never"
//! ([`HousingError::State`]).

use mush_core::ObjId;

use crate::ids::{PropertyId, TenancyId};

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
    /// The story window already carries a job, a scene or an escalation, and a
    /// *different* one was offered. Refused rather than overwritten: the first id is
    /// the one the ST, the queue and the escalation ladder are all pointed at, and
    /// replacing it orphans an open job or an open page that nobody is now watching.
    #[error("the story window already has a different record attached")]
    WindowAlreadyAttached,
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
    /// E5c: [`crate::create_tenancy`] was pointed at a property this world does not
    /// have. Deliberately **not** an existence oracle at the command layer —
    /// `@tenancy/create` folds it into `No such property.` like every other refusal
    /// that names a record the actor may not view.
    #[error("no such property: {0}")]
    NoSuchProperty(PropertyId),
    /// The property is `Withdrawn`: a service decision, and nobody moves into a
    /// residence that is out of service.
    #[error("property {0} is withdrawn")]
    Withdrawn(PropertyId),
    /// The property already carries a live tenancy. **One live tenancy per property**
    /// (E5a fold, handoff 1): two would mean two rosters, two guest lists and two
    /// answers to the same door.
    #[error("property {0} already has a live tenancy")]
    AlreadyLet(PropertyId),
    /// The named cadence clock does not resolve. A tenancy assessed on a clock that
    /// does not exist is one the tick freezes and pages about forever, so it is
    /// refused at creation instead.
    #[error("that cadence clock does not resolve")]
    NoSuchCadence,
    /// An id allocator is at the `u64` ceiling (E4b1's B2 rule: refuse one operation
    /// loudly, never wrap a live id).
    #[error("the {0} allocator is exhausted")]
    Exhausted(&'static str),
    /// The housing module is not loaded.
    #[error("housing is not loaded")]
    NotLoaded,
}
