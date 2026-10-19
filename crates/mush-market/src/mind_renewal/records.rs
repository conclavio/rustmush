//! Exact Market wire. Roll validates the common state relation; Market also
//! pins the retained manifest, complete offer and exact owner references.
use mush_core::{
    ClockStamp, CommitToken, DefinitionDigest, DefinitionRef,
    mechanics::{DefinitionRegistryBuilder, OwnerId},
};
use mush_economy::{
    ContractId,
    managed_renewal::{RenewalOffer, RenewalOutcome, UnfundedCloseReason},
};
use mush_roll::commerce::{
    LicenceId, TermGeneration,
    purchase::PurchaseParticipants,
    renewal::{
        RenewalAttemptV1, RenewalBindingV1, RenewalOperationV1, RenewalStateV1,
        protocol::{self, Transition},
    },
};
use serde::{Deserialize, Serialize};

pub(super) fn invalid() -> String {
    "renewal records disagree or exceed their bounds".into()
}
fn error(_: impl std::fmt::Display) -> String {
    invalid()
}
fn bounded<T: Serialize>(row: &T, max: usize) -> Result<(), String> {
    let value = serde_json::to_value(row).map_err(error)?;
    mush_core::canonical_json_bytes_with_limit(&value, max).map_err(error)?;
    Ok(())
}
fn decode<T: serde::de::DeserializeOwned + Serialize>(
    bytes: &[u8],
    max: usize,
) -> Result<T, String> {
    if bytes.is_empty() || bytes.len() > max {
        return Err(invalid());
    }
    // Typed first rejects duplicate fields before Value can collapse them.
    let row: T = serde_json::from_slice(bytes).map_err(error)?;
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(error)?;
    if serde_json::to_value(&row).map_err(error)? != value {
        return Err(invalid());
    }
    Ok(row)
}
macro_rules! data {
    ($name:ident,$wire:ident,{$($field:ident:$ty:ty),* $(,)?}) => {
        #[derive(Clone,Debug,PartialEq,Eq,Serialize)]
        pub struct $name {$(pub(super) $field:$ty),*}
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $wire {$($field:$ty),*}
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D:serde::Deserializer<'de>>(d:D)->Result<Self,D::Error> {
                let row=$wire::deserialize(d)?;
                let value=Self {$($field:row.$field),*};
                value.validate().map_err(serde::de::Error::custom)?;
                Ok(value)
            }
        }
    };
}
data!(MindRenewalOperationV1,OperationWire,{
    format:u16,operation:CommitToken,action:DefinitionRef,attempt:RenewalAttemptV1,
    manifest:DefinitionRef,participants:[DefinitionRef;2],binding:RenewalBindingV1,
    offer:RenewalOffer,offer_digest:DefinitionDigest,contract:ContractId,consent_hash:[u8;32],
    created_at_ic:ClockStamp,created_at_utc:i64,state:RenewalStateV1,
});

/// A serialization projection, never an actual Roll owner output.
#[derive(Serialize)]
struct Shared<'a> {
    format: u16,
    operation: CommitToken,
    action: &'a DefinitionRef,
    attempt: RenewalAttemptV1,
    binding: &'a RenewalBindingV1,
    offer_digest: DefinitionDigest,
    contract: ContractId,
    consent_hash: [u8; 32],
    created_at_ic: &'a ClockStamp,
    created_at_utc: i64,
    state: &'a RenewalStateV1,
}
impl MindRenewalOperationV1 {
    pub const FAMILY: &'static str = "market_mind_renewal_operation";
    /// Raw/body ceiling. Registration must separately bound the complete typed document.
    pub const MAX_BYTES: usize = 32_768;
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        decode(bytes, Self::MAX_BYTES)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.format != 1 || self.action != self.expected_action(Transition::Draft)? {
            return Err(invalid());
        }
        bounded(
            &self.offer,
            mush_economy::managed_renewal::MAX_RENEWAL_OFFER_BYTES,
        )?;
        let shared = self.roll_projection()?;
        shared.matches_offer(&self.offer).map_err(error)?;
        let start = &self.offer.terms.not_before;
        let expiry = &self.offer.terms.expires_at;
        if self.offer.digest().map_err(error)? != self.offer_digest
            || self.created_at_ic.clock != start.clock
            || self.created_at_ic.at < start.at
            || self.created_at_ic.at >= expiry.at
        {
            return Err(invalid());
        }
        if let Some(funding) = self.state.funding() {
            if funding.action() != &self.expected_action(Transition::Fund)?
                || funding.at_ic().clock != start.clock
                || funding.at_ic().at < start.at
                || funding.at_ic().at >= expiry.at
            {
                return Err(invalid());
            }
        }
        match &self.state {
            RenewalStateV1::Granted { terminal, .. } => {
                if terminal.action() != &self.expected_action(Transition::Grant)?
                    || terminal.at_ic().clock != expiry.clock
                    || terminal.at_ic().at >= expiry.at
                {
                    return Err(invalid());
                }
            }
            RenewalStateV1::UnfundedClosed { closure } => {
                if closure.at_ic().clock != self.created_at_ic.clock
                    || closure.at_ic().at < self.created_at_ic.at
                    || (closure.reason() == UnfundedCloseReason::Expired
                        && closure.at_ic().at < expiry.at)
                {
                    return Err(invalid());
                }
            }
            RenewalStateV1::UnfundedServiceClosed { closure } => match closure.reason() {
                mush_economy::managed_renewal::RenewalServiceCloseReason::DeadlineReached => {
                    if closure.at_ic().clock != self.created_at_ic.clock
                        || closure.at_ic().at < expiry.at
                    {
                        return Err(invalid());
                    }
                }
                mush_economy::managed_renewal::RenewalServiceCloseReason::ClockInvalidated => {
                    if closure.at_ic().clock == self.created_at_ic.clock {
                        return Err(invalid());
                    }
                }
            },
            _ => {}
        }
        // Refund/Close can use a current cleanup descriptor after a manifest
        // change. Their closed locator is checked by the shared codec; actual
        // owner history and dispatch establish the complete action reference.
        bounded(self, Self::MAX_BYTES)
    }
    pub(super) fn expected_action(&self, transition: Transition) -> Result<DefinitionRef, String> {
        let [economy, retention] = &self.participants;
        let participants =
            PurchaseParticipants::new(economy.clone(), retention.clone()).map_err(error)?;
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("market").map_err(error)?,
                protocol::descriptor(transition, &self.manifest, &participants).map_err(error)?,
            )
            .map_err(error)
    }
    pub(super) fn roll_projection(&self) -> Result<RenewalOperationV1, String> {
        let shared = Shared {
            format: self.format,
            operation: self.operation,
            action: &self.action,
            attempt: self.attempt,
            binding: &self.binding,
            offer_digest: self.offer_digest,
            contract: self.contract,
            consent_hash: self.consent_hash,
            created_at_ic: &self.created_at_ic,
            created_at_utc: self.created_at_utc,
            state: &self.state,
        };
        serde_json::from_value(serde_json::to_value(shared).map_err(error)?).map_err(error)
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn key(&self) -> String {
        self.operation.to_hex()
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn attempt(&self) -> RenewalAttemptV1 {
        self.attempt
    }
    pub fn manifest(&self) -> &DefinitionRef {
        &self.manifest
    }
    pub fn participants(&self) -> &[DefinitionRef; 2] {
        &self.participants
    }
    pub fn binding(&self) -> &RenewalBindingV1 {
        &self.binding
    }
    pub fn offer(&self) -> &RenewalOffer {
        &self.offer
    }
    pub const fn offer_digest(&self) -> DefinitionDigest {
        self.offer_digest
    }
    pub const fn contract(&self) -> ContractId {
        self.contract
    }
    pub const fn consent_hash(&self) -> &[u8; 32] {
        &self.consent_hash
    }
    pub fn created_at_ic(&self) -> &ClockStamp {
        &self.created_at_ic
    }
    pub const fn created_at_utc(&self) -> i64 {
        self.created_at_utc
    }
    pub fn state(&self) -> &RenewalStateV1 {
        &self.state
    }
}

data!(MindRenewalReceiptV1,ReceiptWire,{
    format:u16,operation:CommitToken,licence:LicenceId,predecessor:TermGeneration,
    next:TermGeneration,contract:ContractId,consent_hash:[u8;32],state:RenewalStateV1,
});
impl MindRenewalReceiptV1 {
    pub const FAMILY: &'static str = "market_mind_renewal_receipt";
    pub const MAX_BYTES: usize = 8_192;
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        decode(bytes, Self::MAX_BYTES)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.format != 1 || self.predecessor.get().checked_add(1) != Some(self.next.get()) {
            return Err(invalid());
        }
        match &self.state {
            RenewalStateV1::Granted { funding, terminal }
            | RenewalStateV1::Refunded { funding, terminal } => {
                let granted = matches!(&self.state, RenewalStateV1::Granted { .. });
                if funding.commit() == self.operation
                    || funding.contract() != self.contract
                    || funding.consent_hash() != &self.consent_hash
                    || terminal.commit() == self.operation
                    || terminal.commit() == funding.commit()
                    || terminal.transaction() == funding.transaction()
                    || terminal.journal_seq() <= funding.journal_seq()
                    || terminal.posted_at_utc() < funding.posted_at_utc()
                    || (terminal.outcome() == RenewalOutcome::Granted) != granted
                    || (granted
                        && (terminal.at_ic().clock != funding.at_ic().clock
                            || terminal.at_ic().at < funding.at_ic().at))
                {
                    return Err(invalid());
                }
            }
            RenewalStateV1::UnfundedClosed { closure } if closure.commit() != self.operation => {
                closure.validate().map_err(error)?;
            }
            RenewalStateV1::UnfundedServiceClosed { closure }
                if closure.commit() != self.operation =>
            {
                closure.validate().map_err(error)?;
            }
            _ => return Err(invalid()),
        }
        bounded(self, Self::MAX_BYTES)
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn key(&self) -> String {
        self.operation.to_hex()
    }
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    pub const fn predecessor(&self) -> TermGeneration {
        self.predecessor
    }
    pub const fn next(&self) -> TermGeneration {
        self.next
    }
    pub const fn contract(&self) -> ContractId {
        self.contract
    }
    pub const fn consent_hash(&self) -> &[u8; 32] {
        &self.consent_hash
    }
    pub fn state(&self) -> &RenewalStateV1 {
        &self.state
    }
}
