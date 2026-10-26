//! Immutable source-free monetary consent; decoding is never seller, buyer,
//! claim, title or Registry authority. The actual paired owners must prove it.
use super::*;
use crate::{AccountId, Amount, InstrumentId};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, ObjId};
use serde::{Deserialize, Serialize};

macro_rules! closed {
    ($name:ident, $wire:ident, { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
        pub struct $name { $(pub $field: $ty),* }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $wire { $($field: $ty),* }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let w = $wire::deserialize(d)?;
                let row = Self { $($field: w.$field),* };
                row.validate().map_err(serde::de::Error::custom)?;
                Ok(row)
            }
        }
    }
}
fn world(stamp: &ClockStamp) -> Result<(), ParticipantError> {
    if !matches!(stamp.clock, ClockRef::WorldIc { generation } if generation != 0) {
        return Err(refused());
    }
    Ok(())
}
fn bytes<T: Serialize>(row: &T) -> Result<Vec<u8>, ParticipantError> {
    mush_core::canonical_json_bytes_with_limit(
        &serde_json::to_value(row).map_err(|_| ParticipantError::Encoding)?,
        MAX_CHARTER_PENDING_SUBJECT_BYTES,
    )
    .map_err(|_| refused())
}
closed!(CharterOwnershipBasisV1, OwnershipWire, {
    generation: u64, operation: CommitToken, digest: DefinitionDigest,
});
impl CharterOwnershipBasisV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        // One generation remains available for the pending settlement.
        if !(1..MAX_CHARTER_OWNERSHIP_GENERATION).contains(&self.generation) {
            return Err(refused());
        }
        Ok(())
    }
}
closed!(CharterTermBasisV1, TermWire, {
    licence: u64, generation: u64, operation: CommitToken,
    digest: DefinitionDigest, term_end: i64, grace_end: i64,
});
impl CharterTermBasisV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        if self.licence == 0
            || self.generation == 0
            || self
                .grace_end
                .checked_sub(self.term_end)
                .is_none_or(|n| !(0..=MAX_CHARTER_SETTLEMENT_IC_SECONDS).contains(&n))
        {
            return Err(refused());
        }
        // The complete actual Roll term is joined by its digest and operation.
        // These two dates are the economic deadline bound, not a new term.
        Ok(())
    }
}
closed!(CharterPendingTermsV1, TermsWire, {
    format: u16, offer: CommitToken, offer_action: DefinitionRef,
    offer_digest: DefinitionDigest, charter: ObjId, birth: CommitToken,
    ownership: CharterOwnershipBasisV1, owner_term: CharterTermBasisV1,
    title_revision: u64, seller: ObjId, buyer: ObjId,
    seller_account: AccountId, instrument: InstrumentId, price: Amount,
    offered_at_ic: ClockStamp, offered_at_utc: i64, expires_at_utc: i64,
    settle_seconds: i64,
});
impl CharterPendingTermsV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        self.ownership.validate()?;
        self.owner_term.validate()?;
        world(&self.offered_at_ic)?;
        if self.format != 1
            || !action_matches(&self.offer_action, OFFER_ACTION, 2)
            || self.charter.0 == 0
            || self.seller.0 == 0
            || self.buyer.0 == 0
            || self.seller == self.buyer
            || self.title_revision == 0
            || self.title_revision == u64::MAX
            || self.price.minor() <= 0
            || self.offer == self.birth
            || self.offer == self.ownership.operation
            || self.offer == self.owner_term.operation
            || (self.ownership.generation == 1 && self.ownership.operation != self.birth)
            || !(1..=MAX_CHARTER_SETTLEMENT_IC_SECONDS).contains(&self.settle_seconds)
            || self
                .expires_at_utc
                .checked_sub(self.offered_at_utc)
                .is_none_or(|n| !(1..=MAX_CHARTER_PENDING_OFFER_SECONDS).contains(&n))
        {
            return Err(refused());
        }
        bytes(self)?;
        Ok(())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value_with_limit(
            "roll.charter_pending.terms.v1",
            &serde_json::to_value(self).map_err(|_| ParticipantError::Encoding)?,
            MAX_CHARTER_PENDING_SUBJECT_BYTES,
        )
        .map_err(|_| refused())
    }
    pub fn deadline(&self, accepted: &ClockStamp) -> Result<ClockStamp, ParticipantError> {
        self.validate()?;
        world(accepted)?;
        if accepted.clock != self.offered_at_ic.clock || accepted.at < self.offered_at_ic.at {
            return Err(refused());
        }
        // Compute the mathematical minimum before narrowing. A lawful term
        // cap can be representable even when accepted+delay exceeds i64::MAX.
        let after = i128::from(accepted.at) + i128::from(self.settle_seconds);
        let at = i128::from(self.owner_term.term_end).min(after);
        Ok(ClockStamp {
            clock: accepted.clock.clone(),
            at: i64::try_from(at).map_err(|_| refused())?,
        })
    }
}
closed!(CharterPendingConsentV1, ConsentWire, {
    format: u16, operation: CommitToken, effect: DefinitionRef,
    terms: CharterPendingTermsV1, payer: AccountId,
    accepted_at_ic: ClockStamp, accepted_at_utc: i64, deadline: ClockStamp,
});
impl CharterPendingConsentV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        self.terms.validate()?;
        if self.format != 1
            || !action_matches(&self.effect, FUND_ACTION, 1)
            || self.operation == self.terms.offer
            || self.operation == self.terms.birth
            || self.operation == self.terms.ownership.operation
            || self.operation == self.terms.owner_term.operation
            || self.payer == self.terms.seller_account
            || self.accepted_at_utc < self.terms.offered_at_utc
            || self.accepted_at_utc >= self.terms.expires_at_utc
            || self.deadline != self.terms.deadline(&self.accepted_at_ic)?
        {
            return Err(refused());
        }
        bytes(self)?;
        Ok(())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value_with_limit(
            "roll.charter_pending.consent.v1",
            &serde_json::to_value(self).map_err(|_| ParticipantError::Encoding)?,
            MAX_CHARTER_PENDING_SUBJECT_BYTES,
        )
        .map_err(|_| refused())
    }
    pub fn decode(raw: &[u8]) -> Result<Self, ParticipantError> {
        if raw.len() > MAX_CHARTER_PENDING_SUBJECT_BYTES {
            return Err(refused());
        }
        let row: Self = serde_json::from_slice(raw).map_err(|_| refused())?;
        row.validate()?;
        if bytes(&row)? != raw {
            return Err(refused());
        }
        Ok(row)
    }
    /// Positive trigger only. A past deadline is immediately due; it never
    /// becomes an expired offer/refund merely because a service wakes later.
    pub fn deadline_reached(&self, observed: &ClockStamp) -> Result<bool, ParticipantError> {
        self.validate()?;
        world(observed)?;
        if observed.clock != self.accepted_at_ic.clock || observed.at < self.accepted_at_ic.at {
            return Err(refused());
        }
        Ok(observed.at >= self.deadline.at)
    }
}
