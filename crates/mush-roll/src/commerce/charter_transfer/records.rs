//! Exact title consideration and historical ownership; never source authority.
use super::economic::TransferEconomic;
use crate::commerce::records::{InitialFilingOperation, PublicationError, Record};
use crate::commerce::{CharterId, FiledRevisionId, LineId, RevisionIdentity};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_economy::{AccountId, Amount, InstrumentId};
use serde::{Deserialize, Serialize};

pub(crate) const MAX_TRANSFERS: u64 = 65_536;
pub(crate) const MAX_OFFER_SECONDS: i64 = 86_400;
fn bad<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}
fn bounded<T: Serialize>(row: &T, limit: usize) -> Result<(), PublicationError> {
    mush_core::canonical_json_bytes_with_limit(&serde_json::to_value(row).map_err(bad)?, limit)
        .map(|_| ())
        .map_err(|_| PublicationError::Capacity)
}

/// A lookup key and exact immutable digest. Only a historical owner read proves it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CharterOwnershipRef {
    pub(crate) generation: u64,
    pub(crate) operation: CommitToken,
    pub(crate) digest: DefinitionDigest,
}
impl CharterOwnershipRef {
    pub(crate) fn transferred(&self) -> Result<(), PublicationError> {
        self.validate()?;
        if self.generation < 2 {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        if !(1..=MAX_TRANSFERS + 1).contains(&self.generation) {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
    pub(crate) fn birth(birth: &InitialFilingOperation) -> Result<Self, PublicationError> {
        birth.validate()?;
        if birth.economic.house.is_some()
            || birth.economic.issued_owner != mush_org::PrincipalId::Object(birth.requester)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            generation: 1,
            operation: birth.commit,
            digest: DefinitionDigest::for_canonical_value(
                "roll.charter_ownership.birth.v1",
                &serde_json::to_value(birth).map_err(bad)?,
            )
            .map_err(bad)?,
        })
    }
}

impl CharterOwnershipRef {
    /// M8c B4: the birth ownership of a House filing, the organisation's
    /// generation 1. A House charter is never transferred (`birth` refuses a
    /// House birth, and no transfer family admits one), so this is also its
    /// current ownership.
    pub(crate) fn house_birth(birth: &InitialFilingOperation) -> Result<Self, PublicationError> {
        birth.validate()?;
        let Some(house) = &birth.economic.house else {
            return Err(PublicationError::Mismatch);
        };
        if birth.economic.issued_owner != mush_org::PrincipalId::Organization(house.organization())
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            generation: 1,
            operation: birth.commit,
            digest: DefinitionDigest::for_canonical_value(
                "roll.charter_ownership.birth.v1",
                &serde_json::to_value(birth).map_err(bad)?,
            )
            .map_err(bad)?,
        })
    }
}

/// An omitted legacy field is allowed; an explicit null is not a new proof.
pub(crate) fn present_ownership<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<CharterOwnershipRef>, D::Error> {
    let value = CharterOwnershipRef::deserialize(d)?;
    value.transferred().map_err(serde::de::Error::custom)?;
    Ok(Some(value))
}

/// Real seller action's acknowledged immutable consent. No caller can install it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "OfferWire")]
pub(crate) struct CharterOffer {
    pub(crate) format: u16,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) charter: CharterId,
    pub(crate) birth: CommitToken,
    pub(crate) ownership: CharterOwnershipRef,
    pub(crate) seller: ObjId,
    pub(crate) buyer: ObjId,
    pub(crate) seller_account: AccountId,
    pub(crate) instrument: InstrumentId,
    pub(crate) price: Amount,
    pub(crate) title_revision: u64,
    pub(crate) line: LineId,
    pub(crate) filed: FiledRevisionId,
    pub(crate) revision: RevisionIdentity,
    pub(crate) offered_at_ic: ClockStamp,
    pub(crate) offered_at_utc: i64,
    pub(crate) expires_at_utc: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfferWire {
    format: u16,
    operation: CommitToken,
    action: DefinitionRef,
    charter: CharterId,
    birth: CommitToken,
    ownership: CharterOwnershipRef,
    seller: ObjId,
    buyer: ObjId,
    seller_account: AccountId,
    instrument: InstrumentId,
    price: Amount,
    title_revision: u64,
    line: LineId,
    filed: FiledRevisionId,
    revision: RevisionIdentity,
    offered_at_ic: ClockStamp,
    offered_at_utc: i64,
    expires_at_utc: i64,
}
impl TryFrom<OfferWire> for CharterOffer {
    type Error = PublicationError;
    fn try_from(w: OfferWire) -> Result<Self, Self::Error> {
        let r = Self {
            format: w.format,
            operation: w.operation,
            action: w.action,
            charter: w.charter,
            birth: w.birth,
            ownership: w.ownership,
            seller: w.seller,
            buyer: w.buyer,
            seller_account: w.seller_account,
            instrument: w.instrument,
            price: w.price,
            title_revision: w.title_revision,
            line: w.line,
            filed: w.filed,
            revision: w.revision,
            offered_at_ic: w.offered_at_ic,
            offered_at_utc: w.offered_at_utc,
            expires_at_utc: w.expires_at_utc,
        };
        r.validate()?;
        Ok(r)
    }
}
impl Record for CharterOffer {
    const FAMILY: &'static str = "roll_mind_charter_transfer_offers";
    const MAX_BYTES: usize = 8192;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.ownership.validate()?;
        crate::commerce::WorldIcSeconds::from_stamp(&self.offered_at_ic).map_err(bad)?;
        let duration = self
            .expires_at_utc
            .checked_sub(self.offered_at_utc)
            .ok_or(PublicationError::Invalid)?;
        if self.format != 1
            || self.action != super::protocol::offer_action().map_err(bad)?
            || self.seller.0 == 0
            || self.buyer.0 == 0
            || self.seller == self.buyer
            || self.operation == self.birth
            || self.operation == self.ownership.operation
            || self.price.minor() <= 0
            || self.title_revision == 0
            || self.title_revision == u64::MAX
            || !(1..=MAX_OFFER_SECONDS).contains(&duration)
            || (self.ownership.generation == 1 && self.ownership.operation != self.birth)
            || self.ownership.generation > MAX_TRANSFERS
        {
            return Err(PublicationError::Invalid);
        }
        bounded(self, Self::MAX_BYTES)
    }
}
impl CharterOffer {
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "roll.charter_transfer.offer.v1",
            &serde_json::to_value(self).map_err(bad)?,
        )
        .map_err(bad)
    }
    pub(crate) fn matches_birth(
        &self,
        birth: &InitialFilingOperation,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        birth.validate()?;
        if self.birth != birth.commit
            || self.charter != birth.charter
            || self.line != birth.line
            || birth.economic.house.is_some()
            || birth.economic.issued_owner != mush_org::PrincipalId::Object(birth.requester)
            || (self.ownership.generation == 1
                && (self.seller != birth.requester
                    || self.ownership != CharterOwnershipRef::birth(birth)?))
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "TransferWire")]
pub(crate) struct CharterTransfer {
    pub(crate) format: u16,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) charter: CharterId,
    pub(crate) generation: u64,
    pub(crate) birth: CommitToken,
    pub(crate) predecessor: CharterOwnershipRef,
    pub(crate) offer: CommitToken,
    pub(crate) offer_digest: DefinitionDigest,
    pub(crate) accepted_at_ic: ClockStamp,
    pub(crate) economic: TransferEconomic,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransferWire {
    format: u16,
    operation: CommitToken,
    action: DefinitionRef,
    charter: CharterId,
    generation: u64,
    birth: CommitToken,
    predecessor: CharterOwnershipRef,
    offer: CommitToken,
    offer_digest: DefinitionDigest,
    accepted_at_ic: ClockStamp,
    economic: TransferEconomic,
}
impl TryFrom<TransferWire> for CharterTransfer {
    type Error = PublicationError;
    fn try_from(w: TransferWire) -> Result<Self, Self::Error> {
        let r = Self {
            format: w.format,
            operation: w.operation,
            action: w.action,
            charter: w.charter,
            generation: w.generation,
            birth: w.birth,
            predecessor: w.predecessor,
            offer: w.offer,
            offer_digest: w.offer_digest,
            accepted_at_ic: w.accepted_at_ic,
            economic: w.economic,
        };
        r.validate()?;
        Ok(r)
    }
}
impl Record for CharterTransfer {
    const FAMILY: &'static str = "roll_mind_charter_transfers";
    const MAX_BYTES: usize = 40960;
    fn key(&self) -> String {
        Self::key_for(self.charter, self.generation)
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.predecessor.validate()?;
        self.economic.validate()?;
        let economic = self.economic.facts()?;
        crate::commerce::WorldIcSeconds::from_stamp(&self.accepted_at_ic).map_err(bad)?;
        if self.format != self.economic.format()
            || self.action
                != match self.format {
                    1 => super::protocol::accept_action().map_err(bad)?,
                    2 => {
                        crate::commerce::charter_pending::protocol::settle_action().map_err(bad)?
                    }
                    _ => return Err(PublicationError::Invalid),
                }
            || !(2..=MAX_TRANSFERS + 1).contains(&self.generation)
            || self.predecessor.generation.checked_add(1) != Some(self.generation)
            || economic.operation != self.operation
            || economic.charter != self.charter.object()
            || economic.offer != self.offer
            || economic.offer_digest != self.offer_digest
            || self.operation == self.birth
            || self.operation == self.predecessor.operation
            || self.offer == self.birth
            || self.offer == self.predecessor.operation
            || (self.predecessor.generation == 1 && self.predecessor.operation != self.birth)
        {
            return Err(PublicationError::Invalid);
        }
        if let TransferEconomic::Pending(native) = &self.economic {
            let c = native.pending.consent();
            let terminal = native
                .pending
                .terminal()
                .ok_or(PublicationError::Mismatch)?;
            if self.birth != c.terms.birth
                || self.predecessor.generation != c.terms.ownership.generation
                || self.predecessor.operation != c.terms.ownership.operation
                || self.predecessor.digest != c.terms.ownership.digest
                || self.accepted_at_ic != c.accepted_at_ic
                || self.action != terminal.decision().action
            {
                return Err(PublicationError::Mismatch);
            }
        }
        if self.format == 2 {
            crate::commerce::charter_pending::records::envelope(self)?;
        }
        bounded(
            self,
            if self.format == 1 {
                24576
            } else {
                Self::MAX_BYTES
            },
        )
    }
}
impl CharterTransfer {
    pub(crate) fn key_for(charter: CharterId, generation: u64) -> String {
        format!("{}/{generation:016x}", charter.key())
    }
    pub(crate) fn ownership(&self) -> Result<CharterOwnershipRef, PublicationError> {
        self.validate()?;
        Ok(CharterOwnershipRef {
            generation: self.generation,
            operation: self.operation,
            digest: DefinitionDigest::for_canonical_value(
                if self.format == 1 {
                    "roll.charter_ownership.transfer.v1"
                } else {
                    "roll.charter_ownership.transfer.v2"
                },
                &serde_json::to_value(self).map_err(bad)?,
            )
            .map_err(bad)?,
        })
    }
    pub(crate) fn matches_offer(&self, offer: &CharterOffer) -> Result<(), PublicationError> {
        self.validate()?;
        offer.validate()?;
        let economic = self.economic.facts()?;
        if self.format != 1
            || self.offer != offer.operation
            || self.offer_digest != offer.digest()?
            || self.charter != offer.charter
            || self.birth != offer.birth
            || self.predecessor != offer.ownership
            || economic.seller != offer.seller
            || economic.buyer != offer.buyer
            || economic.seller_account != offer.seller_account
            || economic.instrument != &offer.instrument
            || economic.price != offer.price
            || economic.title_revision != offer.title_revision
            || economic.posted_at_utc < offer.offered_at_utc
            || economic.posted_at_utc >= offer.expires_at_utc
            || self.accepted_at_ic.clock != offer.offered_at_ic.clock
            || self.accepted_at_ic.at < offer.offered_at_ic.at
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    pub(crate) fn matches_pending(
        &self,
        offer: &crate::commerce::charter_pending::records::PendingOffer,
        pending: &crate::commerce::charter_pending::records::PendingRecord,
    ) -> Result<(), PublicationError> {
        use crate::commerce::charter_pending::records::PendingState;
        self.validate()?;
        pending.matches_offer(offer)?;
        let TransferEconomic::Pending(economic) = &self.economic else {
            return Err(PublicationError::Mismatch);
        };
        let terminal = economic
            .pending
            .terminal()
            .ok_or(PublicationError::Mismatch)?;
        if self.format != 2
            || economic.pending != pending.economic
            || self.offer != offer.operation
            || self.offer_digest != offer.digest()?
            || self.charter != offer.charter
            || self.birth != offer.birth
            || self.predecessor != offer.ownership
            || self.accepted_at_ic != pending.accepted_at_ic
            || self.action != terminal.decision().action
            || !matches!(&pending.state, PendingState::Settled { terminal, transfer }
                if *terminal == self.operation && transfer == &self.ownership()?)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "HeadWire")]
pub(crate) struct CharterOwnershipHead {
    pub(crate) format: u16,
    pub(crate) charter: CharterId,
    pub(crate) current: CharterOwnershipRef,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadWire {
    format: u16,
    charter: CharterId,
    current: CharterOwnershipRef,
}
impl TryFrom<HeadWire> for CharterOwnershipHead {
    type Error = PublicationError;
    fn try_from(w: HeadWire) -> Result<Self, Self::Error> {
        let r = Self {
            format: w.format,
            charter: w.charter,
            current: w.current,
        };
        r.validate()?;
        Ok(r)
    }
}
impl Record for CharterOwnershipHead {
    const FAMILY: &'static str = "roll_mind_charter_ownership";
    const MAX_BYTES: usize = 4096;
    fn key(&self) -> String {
        self.charter.key()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.current.validate()?;
        if self.format != 1 || self.current.generation < 2 {
            return Err(PublicationError::Invalid);
        }
        bounded(self, Self::MAX_BYTES)
    }
}
