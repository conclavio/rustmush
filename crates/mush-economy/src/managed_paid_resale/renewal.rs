//! Historical native origin of a transferred-holder renewal. Roll separately
//! proves the opaque holder digest and current pointer through its own owner.

use super::{PaidBirthRouteV1, PaidHolderRefV1, PaidResaleOutcome, book::PaidResaleBook, refused};
use mush_core::participant::ParticipantError;
use mush_core::{ClockStamp, CommitToken, ObjId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct RenewalHolderBasisV1 {
    pub acceptance: CommitToken,
    pub ownership: PaidHolderRefV1,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    acceptance: CommitToken,
    ownership: PaidHolderRefV1,
}
impl<'de> Deserialize<'de> for RenewalHolderBasisV1 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = Wire::deserialize(d)?;
        let row = Self {
            acceptance: w.acceptance,
            ownership: w.ownership,
        };
        row.validate().map_err(serde::de::Error::custom)?;
        Ok(row)
    }
}
impl RenewalHolderBasisV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        self.ownership.validate()?;
        if self.ownership.generation < 2 {
            return Err(refused());
        }
        Ok(())
    }
    /// The real completed native book supplies this immutable origin. This is
    /// deliberately independent of today's holder, source, title, or issuer.
    /// The actual Roll observation retains those fresh admission predicates.
    pub(crate) fn require_origin(
        &self,
        book: &PaidResaleBook,
        licence: u64,
        buyer: ObjId,
        charter: ObjId,
        predecessor_generation: u64,
        predecessor_operation: CommitToken,
        at: &ClockStamp,
        utc: i64,
        route: PaidBirthRouteV1,
    ) -> Result<(), ParticipantError> {
        self.validate()?;
        let row = book.get(self.acceptance)?;
        let consent = row.consent();
        let terms = &consent.terms;
        let basis = &terms.basis;
        let (operation, issued, posted) = row.terminal().map_or(
            (
                consent.operation,
                &consent.accepted_at_ic,
                consent.accepted_at_utc,
            ),
            |t| {
                (
                    t.decision().operation,
                    &t.decision().at_ic,
                    t.decision().posted_at_utc,
                )
            },
        );
        if row.outcome() != PaidResaleOutcome::Settled
            || consent.operation != self.acceptance
            || basis.licence != licence
            || basis.charter != charter
            || basis.birth.route != route
            || terms.buyer != buyer
            || basis.holder.generation.checked_add(1) != Some(self.ownership.generation)
            || operation != self.ownership.operation
            || predecessor_generation < basis.term.generation
            || (predecessor_generation == basis.term.generation
                && predecessor_operation != basis.term.operation)
            || (at.clock == issued.clock && at.at < issued.at)
            || utc < posted
        {
            return Err(refused());
        }
        Ok(())
    }
}
