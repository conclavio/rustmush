//! Read-only selection of a possible terminal. This observation is never an
//! authorization token; actual native prepare/recheck repeats these predicates.
use super::{PaidResaleOutcome, native, refused};
use crate::EconomyStore;
use mush_core::{CommitToken, World, participant::ParticipantError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaidResaleAvailability {
    Current,
    RecipientUnavailable,
    MonetaryBindingChanged,
}
impl EconomyStore {
    pub fn paid_resale_availability(
        &self,
        world: &World,
        operation: CommitToken,
    ) -> Result<PaidResaleAvailability, ParticipantError> {
        if self.is_frozen() {
            return Err(refused());
        }
        let row = self.paid_resale_history(operation)?;
        if row.outcome() != PaidResaleOutcome::Funded || row.terminal().is_some() {
            return Err(refused());
        }
        Ok(
            if !native::recipient_available(self, world, row.consent()) {
                PaidResaleAvailability::RecipientUnavailable
            } else if !native::monetary_current(self, world, row.consent()) {
                PaidResaleAvailability::MonetaryBindingChanged
            } else {
                PaidResaleAvailability::Current
            },
        )
    }
}
