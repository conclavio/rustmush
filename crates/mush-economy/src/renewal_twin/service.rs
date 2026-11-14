//! Separate immutable nonmonetary closure proof. Historical offer bytes and
//! their exact close-room reservations remain unchanged.
use super::{
    Basis as _, Issuer, Offer as _, RenewalAuditError, Terms as _, action_matches,
    history::OfferRecord, refused,
};
use mush_core::participant::ParticipantError;
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionRef, GOD, ObjId};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

pub(crate) const MAX_BYTES: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenewalServiceCloseReason {
    DeadlineReached,
    ClockInvalidated,
}

/// Closed data, never a service capability. Only the actual owner output or
/// borrowed complete audit establishes a published closure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(bound = "")]
pub struct RenewalServiceClosureV1<I: Issuer> {
    format: u16,
    commit: CommitToken,
    action: DefinitionRef,
    actor: ObjId,
    at_ic: ClockStamp,
    posted_at_utc: i64,
    reason: RenewalServiceCloseReason,
    #[serde(skip)]
    issuer: PhantomData<I>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClosureWire {
    format: u16,
    commit: CommitToken,
    action: DefinitionRef,
    actor: ObjId,
    at_ic: ClockStamp,
    posted_at_utc: i64,
    reason: RenewalServiceCloseReason,
}
impl<'de, I: Issuer> Deserialize<'de> for RenewalServiceClosureV1<I> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = ClosureWire::deserialize(d)?;
        let row = Self {
            format: w.format,
            commit: w.commit,
            action: w.action,
            actor: w.actor,
            at_ic: w.at_ic,
            posted_at_utc: w.posted_at_utc,
            reason: w.reason,
            issuer: PhantomData,
        };
        row.validate().map_err(serde::de::Error::custom)?;
        Ok(row)
    }
}
impl<I: Issuer> RenewalServiceClosureV1<I> {
    pub fn validate(&self) -> Result<(), RenewalAuditError> {
        if self.format != 1
            || self.actor != GOD
            || !action_matches(&self.action, I::SERVICE_CLOSE_ACTION)
            || !matches!(self.at_ic.clock, ClockRef::WorldIc { generation } if generation != 0)
            || serde_json::to_vec(self)
                .map_err(|_| RenewalAuditError::Refused)?
                .len()
                > super::MAX_RENEWAL_RECEIPT_BYTES
        {
            return Err(RenewalAuditError::Refused);
        }
        Ok(())
    }
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn actor(&self) -> ObjId {
        self.actor
    }
    pub fn at_ic(&self) -> &ClockStamp {
        &self.at_ic
    }
    pub const fn posted_at_utc(&self) -> i64 {
        self.posted_at_utc
    }
    pub const fn reason(&self) -> RenewalServiceCloseReason {
        self.reason
    }

    pub(super) fn create(
        context: &super::install::OfferContext<'_>,
        reason: RenewalServiceCloseReason,
    ) -> Result<Self, ParticipantError> {
        let row = Self {
            format: 1,
            commit: context.commit,
            action: context.action.clone(),
            actor: context.actor,
            at_ic: context.world.world_ic_now(),
            posted_at_utc: context.world.now(),
            reason,
            issuer: PhantomData,
        };
        row.validate().map_err(|_| refused())?;
        Ok(row)
    }
    pub(super) fn require_origin(&self, row: &OfferRecord<I>) -> Result<(), ParticipantError> {
        self.validate().map_err(|_| refused())?;
        if row.closure.is_some()
            || self.commit == row.operation
            || self.commit == row.offer.basis().predecessor_operation()
            || self.commit == row.offer.basis().prior_quote()
            || row.previous() == Some(self.commit)
            || self.posted_at_utc < row.created_at_utc
        {
            return Err(refused());
        }
        match self.reason {
            RenewalServiceCloseReason::DeadlineReached => {
                if self.at_ic.clock != row.created_at_ic.clock
                    || self.at_ic.at < row.offer.terms().expires_at().at
                    || self.at_ic.at < row.created_at_ic.at
                {
                    return Err(refused());
                }
            }
            RenewalServiceCloseReason::ClockInvalidated => {
                if self.at_ic.clock == row.created_at_ic.clock {
                    return Err(refused());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound = "")]
pub(super) struct ServiceRecord<I: Issuer> {
    pub format: u16,
    pub operation: CommitToken,
    pub closure: RenewalServiceClosureV1<I>,
}
impl<I: Issuer> ServiceRecord<I> {
    pub(super) fn validate(&self) -> Result<(), ParticipantError> {
        self.closure.validate().map_err(|_| refused())?;
        if self.format != 1
            || self.operation == self.closure.commit
            || serde_json::to_vec(self)
                .map_err(|_| ParticipantError::Encoding)?
                .len()
                > MAX_BYTES
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) fn require_origin(&self, origin: &OfferRecord<I>) -> Result<(), ParticipantError> {
        self.validate()?;
        if self.operation != origin.operation {
            return Err(refused());
        }
        self.closure.require_origin(origin)
    }
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, ParticipantError> {
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err(refused());
        }
        let row: Self = serde_json::from_slice(bytes).map_err(|_| refused())?;
        row.validate()?;
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| refused())?;
        if serde_json::to_value(&row).map_err(|_| refused())? != value {
            return Err(refused());
        }
        Ok(row)
    }
}
pub(crate) fn key<I: Issuer>(operation: CommitToken) -> String {
    format!("{}{}", I::SERVICE_PREFIX, operation.to_hex())
}

pub(super) fn contract<I: Issuer>(
    origin: &OfferRecord<I>,
    close: &ServiceRecord<I>,
) -> Result<crate::contract::EconomicContract, ParticipantError> {
    close.require_origin(origin)?;
    let mut contract = super::contract(origin)?;
    contract.state = match close.closure.reason {
        RenewalServiceCloseReason::DeadlineReached => crate::contract::ContractState::Expired,
        RenewalServiceCloseReason::ClockInvalidated => crate::contract::ContractState::Cancelled,
    };
    contract.revision = 2;
    Ok(contract)
}
