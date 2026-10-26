//! Separate permanent paired identities; no publication constructor is public.
use super::codec::{self, Row};
use super::records::EntitlementQuoteV1;
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef};
use mush_economy::managed_purchase::FundingReceipt;
use mush_economy::{ContractId, HoldId, TransactionId};
use mush_roll::commerce::entitlement::EntitlementAllocationV1;
use mush_roll::commerce::purchase::{PurchaseParticipants, PurchasePhase};
use mush_roll::commerce::{LicenceId, TermGeneration};
use serde::{Deserialize, Deserializer, Serialize};

fn invalid() -> String {
    "permanent entitlement records disagree".into()
}
macro_rules! data {
    ($name:ident,$wire:ident,{$($(#[$attr:meta])* $field:ident:$ty:ty),* $(,)?}) => {
        #[derive(Clone,Debug,PartialEq,Eq,Serialize)]
        pub struct $name {$(pub(super) $field:$ty),*}
        #[derive(Deserialize)] #[serde(deny_unknown_fields)]
        struct $wire {$($(#[$attr])* $field:$ty),*}
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D:Deserializer<'de>>(d:D)->Result<Self,D::Error> {
                let row=$wire::deserialize(d)?; let value=Self {$($field:row.$field),*};
                value.validate().map_err(serde::de::Error::custom)?; Ok(value)
            }
        }
    };
}
data!(EntitlementPurchaseV1,PurchaseWire,{
    format:u16,operation:CommitToken,effect:DefinitionRef,participants:PurchaseParticipants,
    quote:EntitlementQuoteV1,quote_digest:DefinitionDigest,licence:LicenceId,term:TermGeneration,
    funding:FundingReceipt,phase:PurchasePhase,updated_by:CommitToken,
});
impl EntitlementPurchaseV1 {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        codec::decode(bytes)
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub const fn phase(&self) -> PurchasePhase {
        self.phase
    }
    pub(super) fn matches_allocation(&self, a: &EntitlementAllocationV1) -> Result<(), String> {
        self.validate()?;
        let q = &self.quote;
        if a.protocol() != q.product.protocol
            || a.operation() != self.operation
            || a.participants() != &self.participants
            || a.licence() != self.licence
            || a.term() != self.term
            || a.quote() != q.publication
            || a.quote_digest() != self.quote_digest
            || a.product_digest() != q.product_digest
            || a.holder() != q.buyer
            || a.charter() != q.subject.charter
            || a.line() != q.subject.line
            || a.filed() != q.subject.filed
            || a.revision() != q.subject.revision
            || a.rating() != q.subject.rating
            || a.terms() != q.product.terms
            || a.economics().grant() != &q.product.grant
            || a.funding() != &self.funding
            || a.phase() != self.phase
            || a.updated_by() != self.updated_by
        {
            return Err(invalid());
        }
        Ok(())
    }
}
impl Row for EntitlementPurchaseV1 {
    const FAMILY: &'static str = "market_mind_entitlement_purchase";
    const MAX_BYTES: usize = 32768;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), String> {
        self.quote.validate()?;
        self.phase
            .validate(self.operation, self.updated_by)
            .map_err(|e| e.to_string())?;
        if self.format != 1
            || self.term.get() != 1
            || self.operation == self.quote.publication
            || self.quote_digest != self.quote.digest()?
            || self.effect
                != super::protocol::historical_fund(
                    self.quote.product.protocol,
                    self.quote.subject.compatibility,
                    &self.participants,
                )?
        {
            return Err(invalid());
        }
        codec::bounded(self, Self::MAX_BYTES)?;
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Outcome {
    Granted {},
    Refunded {},
}
data!(EntitlementReceiptV1,ReceiptWire,{
    format:u16,operation:CommitToken,commit:CommitToken,outcome:Outcome,
    contract:ContractId,hold:HoldId,transaction:TransactionId,journal_seq:u64,
    licence:LicenceId,term:TermGeneration,quote_digest:DefinitionDigest,product_digest:DefinitionDigest,
    grant_economics_digest:DefinitionDigest,
    #[serde(deserialize_with="codec::required")] issued_at_ic:Option<ClockStamp>,
});
impl EntitlementReceiptV1 {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        codec::decode(bytes)
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub(super) fn matches_purchase(&self, p: &EntitlementPurchaseV1) -> Result<(), String> {
        self.validate()?;
        p.validate()?;
        let phase = match self.outcome {
            Outcome::Granted {} => PurchasePhase::Granted {
                commit: self.commit,
            },
            Outcome::Refunded {} => PurchasePhase::Refunded {
                commit: self.commit,
            },
        };
        if self.operation != p.operation
            || self.commit != p.updated_by
            || p.phase != phase
            || self.contract != p.funding.contract()
            || self.hold != p.funding.hold()
            || self.transaction == p.funding.transaction()
            || self.journal_seq <= p.funding.journal_seq()
            || self
                .issued_at_ic
                .as_ref()
                .is_some_and(|stamp| stamp.clock != p.funding.at_ic().clock)
            || self.licence != p.licence
            || self.term != p.term
            || self.quote_digest != p.quote_digest
            || self.product_digest != p.quote.product_digest
            || self.grant_economics_digest
                != p.quote.product.grant.digest().map_err(|e| e.to_string())?
        {
            return Err(invalid());
        }
        Ok(())
    }
}
impl Row for EntitlementReceiptV1 {
    const FAMILY: &'static str = "market_mind_entitlement_receipt";
    const MAX_BYTES: usize = 8192;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), String> {
        if self.format != 1 || self.operation == self.commit || self.term.get() != 1 {
            return Err(invalid());
        }
        match (self.outcome, self.issued_at_ic.as_ref()) {
            (Outcome::Granted {}, Some(stamp)) if matches!(stamp.clock,ClockRef::WorldIc{generation} if generation!=0) =>
                {}
            (Outcome::Refunded {}, None) => {}
            _ => return Err(invalid()),
        }
        codec::bounded(self, Self::MAX_BYTES)?;
        Ok(())
    }
}
