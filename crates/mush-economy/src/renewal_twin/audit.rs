//! Borrowed evidence for a paired owner audit. No serialized value, mutable
//! contract, or public constructor can establish the combined runtime latch.
use super::{
    Issuer, RenewalAuditError, RenewalHistory, UnfundedCloseReason,
    history::{Closure, OfferRecord},
};
use crate::{ContractId, EconomyStore};
use mush_core::{ClockStamp, CommitToken, DefinitionRef};
use std::marker::PhantomData;

pub struct RenewalAudit<'a, I: Issuer> {
    state: &'a EconomyStore,
    issuer: PhantomData<I>,
}
pub(crate) fn read<I: Issuer>(
    state: &EconomyStore,
) -> Result<RenewalAudit<'_, I>, RenewalAuditError> {
    if state.frozen || !I::book(state).combined_verified() {
        return Err(RenewalAuditError::Incomplete);
    }
    I::receipts(&state.fold).require_complete()?;
    Ok(RenewalAudit {
        state,
        issuer: PhantomData,
    })
}
impl<'a, I: Issuer> RenewalAudit<'a, I> {
    pub fn require_complete(&self) -> Result<(), RenewalAuditError> {
        read::<I>(self.state).map(|_| ())
    }
    pub fn len(&self) -> usize {
        I::book(self.state).operations().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn get(
        &self,
        operation: CommitToken,
    ) -> Result<RenewalOfferView<'a, I>, RenewalAuditError> {
        self.require_complete()?;
        let record = I::book(self.state)
            .get(operation)
            .ok_or(RenewalAuditError::Refused)?;
        record.validate().map_err(|_| RenewalAuditError::Refused)?;
        let history = I::receipts(&self.state.fold).find(operation)?;
        if let Some(h) = history {
            if h.origin() != record
                || record.closure.is_some()
                || I::book(self.state).service(operation).is_some()
            {
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
            I::book(self.state)
                .verify_offer(record, &self.state.snapshot)
                .map_err(|_| RenewalAuditError::Refused)?;
        }
        Ok(RenewalOfferView {
            record,
            history,
            service: I::book(self.state).service(operation),
        })
    }
    /// At most the existing 32,768 permanent offers. Every yielded row repeats
    /// its exact current projection join; a failure is not an absent row.
    pub fn offers(
        &self,
    ) -> impl ExactSizeIterator<Item = Result<RenewalOfferView<'a, I>, RenewalAuditError>> + '_
    {
        I::book(self.state).operations().map(|op| self.get(op))
    }
}

/// Non-Clone, non-Deserialize borrowed evidence. Consumers must join their
/// exact typed operation; this is not a current-rights or money capability.
pub struct RenewalOfferView<'a, I: Issuer> {
    record: &'a OfferRecord<I>,
    history: Option<&'a RenewalHistory<I>>,
    service: Option<&'a super::service::ServiceRecord<I>>,
}
impl<'a, I: Issuer> RenewalOfferView<'a, I> {
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
    pub fn offer(&self) -> &'a I::Offer {
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
    pub fn service_closed(&self) -> Option<&'a super::RenewalServiceClosureV1<I>> {
        self.service.map(|record| &record.closure)
    }
    pub fn history(&self) -> Option<&'a RenewalHistory<I>> {
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
