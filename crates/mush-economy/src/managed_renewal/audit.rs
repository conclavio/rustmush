//! Borrowed evidence for a paired owner audit. No serialized value, mutable
//! contract, or public constructor can establish the combined runtime latch.
use super::{
    RenewalAuditError, RenewalHistory, RenewalOffer, UnfundedCloseReason,
    history::{Closure, OfferRecord},
};
use crate::{ContractId, EconomyStore};
use mush_core::{ClockStamp, CommitToken, DefinitionRef};

pub struct RenewalAudit<'a> {
    state: &'a EconomyStore,
}
pub(crate) fn read(state: &EconomyStore) -> Result<RenewalAudit<'_>, RenewalAuditError> {
    if state.frozen || !state.renewal_offers.combined_verified() {
        return Err(RenewalAuditError::Incomplete);
    }
    state.fold.renewal_receipts.require_complete()?;
    Ok(RenewalAudit { state })
}
impl<'a> RenewalAudit<'a> {
    pub fn require_complete(&self) -> Result<(), RenewalAuditError> {
        read(self.state).map(|_| ())
    }
    pub fn len(&self) -> usize {
        self.state.renewal_offers.operations().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn get(&self, operation: CommitToken) -> Result<RenewalOfferView<'a>, RenewalAuditError> {
        self.require_complete()?;
        let record = self
            .state
            .renewal_offers
            .get(operation)
            .ok_or(RenewalAuditError::Refused)?;
        record.validate().map_err(|_| RenewalAuditError::Refused)?;
        let history = self.state.fold.renewal_receipts.find(operation)?;
        if let Some(h) = history {
            if h.origin() != record || record.closure.is_some() {
                return Err(RenewalAuditError::Refused);
            }
            h.verify_documents(
                self.state
                    .snapshot
                    .contract(record.contract)
                    .ok_or(RenewalAuditError::Refused)?,
                self.state
                    .snapshot
                    .hold(h.funding().hold())
                    .ok_or(RenewalAuditError::Refused)?,
            )?;
        } else {
            record
                .verify_documents(&self.state.snapshot)
                .map_err(|_| RenewalAuditError::Refused)?;
        }
        Ok(RenewalOfferView { record, history })
    }
    /// At most the existing 32,768 permanent offers. Every yielded row repeats
    /// its exact current projection join; a failure is not an absent row.
    pub fn offers(
        &self,
    ) -> impl ExactSizeIterator<Item = Result<RenewalOfferView<'a>, RenewalAuditError>> + '_ {
        self.state
            .renewal_offers
            .operations()
            .map(|op| self.get(op))
    }
}

/// Non-Clone, non-Deserialize borrowed evidence. Consumers must join their
/// exact typed operation; this is not a current-rights or money capability.
pub struct RenewalOfferView<'a> {
    record: &'a OfferRecord,
    history: Option<&'a RenewalHistory>,
}
impl<'a> RenewalOfferView<'a> {
    pub fn format(&self) -> u16 {
        self.record.format
    }
    pub fn attempt(&self) -> super::RenewalAttemptV1 {
        self.record.attempt()
    }
    pub fn operation(&self) -> CommitToken {
        self.record.operation
    }
    pub fn action(&self) -> &'a DefinitionRef {
        &self.record.action
    }
    pub fn offer(&self) -> &'a RenewalOffer {
        &self.record.offer
    }
    pub fn contract(&self) -> ContractId {
        self.record.contract
    }
    pub fn consent_hash(&self) -> [u8; 32] {
        self.record.consent_hash
    }
    pub fn created_at_ic(&self) -> &'a ClockStamp {
        &self.record.created_at_ic
    }
    pub fn created_at_utc(&self) -> i64 {
        self.record.created_at_utc
    }
    pub fn closure(&self) -> Option<RenewalClosureView<'a>> {
        self.record
            .closure
            .as_ref()
            .map(|closure| RenewalClosureView { closure })
    }
    pub fn history(&self) -> Option<&'a RenewalHistory> {
        self.history
    }
}
pub struct RenewalClosureView<'a> {
    closure: &'a Closure,
}
impl<'a> RenewalClosureView<'a> {
    pub fn commit(&self) -> CommitToken {
        self.closure.commit
    }
    pub fn action(&self) -> &'a DefinitionRef {
        &self.closure.action
    }
    pub fn at_ic(&self) -> &'a ClockStamp {
        &self.closure.at_ic
    }
    pub fn at_utc(&self) -> i64 {
        self.closure.at_utc
    }
    pub fn reason(&self) -> UnfundedCloseReason {
        self.closure.reason
    }
}
