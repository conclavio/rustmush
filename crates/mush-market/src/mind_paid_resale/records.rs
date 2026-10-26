//! Strict complete Market records. Pair checks are data relations; only actual
//! Roll owner reads and opaque Economy outputs may authorize publication.
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef};
use mush_economy::managed_paid_resale::{PaidResaleHistoryV1, PaidResaleTermsV1};
use mush_roll::commerce::{
    LicenceId,
    paid_resale::{PaidResaleBinding, PaidResaleRecord, PaidResaleState, protocol},
};
use serde::{Deserialize, Serialize};

fn invalid(_: impl std::fmt::Display) -> String {
    "paid resale records disagree or exceed their bounds".into()
}
fn bounded<T: Serialize>(r: &T, max: usize) -> Result<Vec<u8>, String> {
    mush_core::canonical_json_bytes_with_limit(&serde_json::to_value(r).map_err(invalid)?, max)
        .map_err(invalid)
}
fn digest<T: Serialize>(domain: &str, r: &T, max: usize) -> Result<DefinitionDigest, String> {
    DefinitionDigest::for_canonical_value_with_limit(
        domain,
        &serde_json::to_value(r).map_err(invalid)?,
        max,
    )
    .map_err(invalid)
}
fn decode<T: serde::de::DeserializeOwned + Serialize>(
    bytes: &[u8],
    max: usize,
) -> Result<T, String> {
    if bytes.is_empty() || bytes.len() > max {
        return Err(invalid("size"));
    }
    let row: T = serde_json::from_slice(bytes).map_err(invalid)?;
    if bounded(&row, max)? != bytes {
        return Err(invalid("canonical"));
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
                let w=$wire::deserialize(d)?; let row=Self {$($field:w.$field),*};
                row.validate().map_err(serde::de::Error::custom)?; Ok(row)
            }
        }
    };
}
data!(MindPaidResaleOffer,OfferWire,{
    format:u16,operation:CommitToken,action:DefinitionRef,binding:PaidResaleBinding,terms:PaidResaleTermsV1,
});
impl MindPaidResaleOffer {
    pub const FAMILY: &'static str = "market_mind_paid_resale_offers";
    pub const MAX_BYTES: usize = 16384;
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        decode(bytes, Self::MAX_BYTES)
    }
    pub fn validate(&self) -> Result<(), String> {
        self.binding.validate().map_err(invalid)?;
        self.terms.validate().map_err(invalid)?;
        if self.format != 1
            || self.action != protocol::actions().map_err(invalid)?[0]
            || self.operation != self.terms.offer
            || self.action != self.terms.offer_action
            || self.binding.native_basis().map_err(invalid)? != self.terms.basis
        {
            return Err(invalid("offer affinity"));
        }
        bounded(self, Self::MAX_BYTES).map(|_| ())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, String> {
        self.validate()?;
        digest("market.paid_resale.offer.v1", self, Self::MAX_BYTES)
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn binding(&self) -> &PaidResaleBinding {
        &self.binding
    }
    pub fn terms(&self) -> &PaidResaleTermsV1 {
        &self.terms
    }
    pub fn matches_roll(&self, row: &PaidResaleRecord) -> Result<(), String> {
        self.validate()?;
        row.validate().map_err(invalid)?;
        if self.operation != row.offer()
            || self.digest()? != row.offer_digest()
            || &self.binding != row.binding()
            || self.terms != row.economic().consent().terms
        {
            return Err(invalid("offer pair"));
        }
        Ok(())
    }
}
data!(MindPaidResaleOperation,OperationWire,{
    format:u16,operation:CommitToken,action:DefinitionRef,licence:LicenceId,sequence:u64,
    offer:CommitToken,offer_digest:DefinitionDigest,roll_digest:DefinitionDigest,state:PaidResaleState,
});
impl MindPaidResaleOperation {
    pub const FAMILY: &'static str = "market_mind_paid_resales";
    pub const MAX_BYTES: usize = 40960;
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        decode(bytes, Self::MAX_BYTES)
    }
    pub fn validate(&self) -> Result<(), String> {
        let actions = protocol::actions().map_err(invalid)?;
        if self.format != 1
            || !(1..=65536).contains(&self.sequence)
            || self.offer == self.operation
            || (self.action != actions[1] && self.action != actions[2])
        {
            return Err(invalid("operation"));
        }
        match &self.state {
            PaidResaleState::Funded {} if self.action != actions[2] => {
                return Err(invalid("funded"));
            }
            PaidResaleState::Settled {
                terminal,
                ownership,
            } => {
                ownership.validate().map_err(invalid)?;
                if ownership.generation < 2
                    || ownership.operation != *terminal
                    || (*terminal == self.operation) != (self.action == actions[1])
                {
                    return Err(invalid("settled"));
                }
            }
            PaidResaleState::Refunded { terminal, reason }
                if (self.action != actions[2]
                    || *terminal == self.operation
                    || reason.settles()) =>
            {
                return Err(invalid("refund"));
            }
            _ => (),
        }
        bounded(self, Self::MAX_BYTES).map(|_| ())
    }
    pub(super) fn from_roll(row: &PaidResaleRecord) -> Result<Self, String> {
        row.validate().map_err(invalid)?;
        let result = Self {
            format: 1,
            operation: row.operation(),
            action: row.action().clone(),
            licence: row.binding().licence(),
            sequence: row.sequence(),
            offer: row.offer(),
            offer_digest: row.offer_digest(),
            roll_digest: row.digest().map_err(invalid)?,
            state: row.state().clone(),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn matches_roll(&self, row: &PaidResaleRecord) -> Result<(), String> {
        self.validate()?;
        if self != &Self::from_roll(row)? {
            return Err(invalid("Roll pair"));
        }
        Ok(())
    }
    pub fn verify_history(
        &self,
        row: &PaidResaleRecord,
        actual: &PaidResaleHistoryV1,
    ) -> Result<(), String> {
        self.matches_roll(row)?;
        actual.digest().map_err(invalid)?;
        if actual != row.economic() {
            return Err(invalid("native pair"));
        }
        Ok(())
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    pub fn state(&self) -> &PaidResaleState {
        &self.state
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResaleOutcome {
    Settled,
    Refunded,
}
data!(MindPaidResaleReceipt,ReceiptWire,{
    format:u16,operation:CommitToken,acceptance:CommitToken,licence:LicenceId,offer:CommitToken,
    offer_digest:DefinitionDigest,roll_digest:DefinitionDigest,economic_digest:DefinitionDigest,outcome:ResaleOutcome,
});
impl MindPaidResaleReceipt {
    pub const FAMILY: &'static str = "market_mind_paid_resale_receipts";
    pub const MAX_BYTES: usize = 8192;
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        decode(bytes, Self::MAX_BYTES)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.format != 1
            || self.operation == self.offer
            || self.acceptance == self.offer
            || (self.outcome == ResaleOutcome::Refunded && self.operation == self.acceptance)
        {
            return Err(invalid("terminal"));
        }
        bounded(self, Self::MAX_BYTES).map(|_| ())
    }
    pub(super) fn from_roll(row: &PaidResaleRecord) -> Result<Self, String> {
        row.validate().map_err(invalid)?;
        let (operation, outcome) = match row.state() {
            PaidResaleState::Funded {} => return Err(invalid("not terminal")),
            PaidResaleState::Settled { terminal, .. } => (*terminal, ResaleOutcome::Settled),
            PaidResaleState::Refunded { terminal, .. } => (*terminal, ResaleOutcome::Refunded),
        };
        let result = Self {
            format: 1,
            operation,
            acceptance: row.operation(),
            licence: row.binding().licence(),
            offer: row.offer(),
            offer_digest: row.offer_digest(),
            roll_digest: row.digest().map_err(invalid)?,
            economic_digest: row.economic().digest().map_err(invalid)?,
            outcome,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn matches_roll(&self, row: &PaidResaleRecord) -> Result<(), String> {
        self.validate()?;
        if self != &Self::from_roll(row)? {
            return Err(invalid("terminal pair"));
        }
        Ok(())
    }
    pub const fn acceptance(&self) -> CommitToken {
        self.acceptance
    }
}
