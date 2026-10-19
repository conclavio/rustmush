//! Immutable seller consent, one admitted pending escrow, permanent terminal
//! head. These strict rows are data; actual owner history and Root custody prove them.
use crate::commerce::charter_transfer::records::CharterOwnershipRef;
use crate::commerce::records::{OwnerTerm, PublicationError, Record};
use crate::commerce::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermWindow,
};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_economy::managed_charter_transfer::{
    CharterOwnershipBasisV1, CharterPendingDecisionKindV1, CharterPendingHistory,
    CharterPendingTermsV1, CharterTermBasisV1,
};
use mush_economy::{AccountId, Amount, InstrumentId};
use serde::{Deserialize, Serialize};
pub(crate) const MAX_PENDING_HISTORY: u64 = 65_536;
pub(crate) const MAX_PENDING_ROW_BYTES: usize = 40_960;
fn bad<T>(_: T) -> PublicationError {
    PublicationError::Mismatch
}
fn bounded<T: Serialize>(row: &T, max: usize) -> Result<(), PublicationError> {
    mush_core::canonical_json_bytes_with_limit(&serde_json::to_value(row).map_err(bad)?, max)
        .map(|_| ())
        .map_err(|_| PublicationError::Capacity)
}
pub(crate) fn envelope<R: Record + Serialize>(row: &R) -> Result<(), PublicationError> {
    bounded(
        &serde_json::json!({
            "family":{"owner":crate::NS,"id":R::FAMILY},
            "schema":{"id":format!("{}.value.v1",R::FAMILY),"version":1},
            "key":row.key(),"value":row
        }),
        R::MAX_BYTES,
    )
}
fn digest<T: Serialize>(domain: &str, row: &T) -> Result<DefinitionDigest, PublicationError> {
    DefinitionDigest::for_canonical_value_with_limit(
        domain,
        &serde_json::to_value(row).map_err(bad)?,
        MAX_PENDING_ROW_BYTES,
    )
    .map_err(bad)
}
macro_rules! closed {
 ($name:ident,$wire:ident,{$($field:ident:$ty:ty),*$(,)?})=>{
  #[derive(Clone,Debug,PartialEq,Eq,Serialize)] pub(crate) struct $name {$(pub(crate) $field:$ty),*}
  #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct $wire {$($field:$ty),*}
  impl<'de> Deserialize<'de> for $name {fn deserialize<D:serde::Deserializer<'de>>(d:D)->Result<Self,D::Error>{let w=$wire::deserialize(d)?;let row=Self {$($field:w.$field),*};row.validate().map_err(serde::de::Error::custom)?;Ok(row)}}
 };
}
closed!(PendingOwnerTerm,OwnerTermWire,{licence:LicenceId,generation:TermGeneration,operation:CommitToken,digest:DefinitionDigest,window:TermWindow});
impl PendingOwnerTerm {
    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        bounded(self, 1024)
    }
    pub(crate) fn capture(term: &OwnerTerm) -> Result<Self, PublicationError> {
        term.validate()?;
        Ok(Self {
            licence: term.licence,
            generation: term.generation,
            operation: term.operation,
            digest: digest("roll.charter_pending.owner_term.v1", term)?,
            window: term.window,
        })
    }
    pub(crate) fn matches(&self, term: &OwnerTerm) -> Result<(), PublicationError> {
        if self != &Self::capture(term)? {
            Err(PublicationError::Mismatch)
        } else {
            Ok(())
        }
    }
    fn native(&self) -> CharterTermBasisV1 {
        CharterTermBasisV1 {
            licence: self.licence.get(),
            generation: self.generation.get(),
            operation: self.operation,
            digest: self.digest,
            term_end: self.window.term_end().get(),
            grace_end: self.window.grace_end().get(),
        }
    }
}
closed!(PendingOffer,OfferWire,{
 format:u16,operation:CommitToken,action:DefinitionRef,charter:CharterId,birth:CommitToken,ownership:CharterOwnershipRef,
 seller:ObjId,buyer:ObjId,seller_account:AccountId,instrument:InstrumentId,price:Amount,title_revision:u64,
 line:LineId,filed:FiledRevisionId,revision:RevisionIdentity,owner_term:PendingOwnerTerm,
 offered_at_ic:ClockStamp,offered_at_utc:i64,expires_at_utc:i64,settle_seconds:i64
});
impl PendingOffer {
    fn native_unchecked(&self, offer_digest: DefinitionDigest) -> CharterPendingTermsV1 {
        CharterPendingTermsV1 {
            format: 1,
            offer: self.operation,
            offer_action: self.action.clone(),
            offer_digest,
            charter: self.charter.object(),
            birth: self.birth,
            ownership: CharterOwnershipBasisV1 {
                generation: self.ownership.generation,
                operation: self.ownership.operation,
                digest: self.ownership.digest,
            },
            owner_term: self.owner_term.native(),
            title_revision: self.title_revision,
            seller: self.seller,
            buyer: self.buyer,
            seller_account: self.seller_account,
            instrument: self.instrument.clone(),
            price: self.price,
            offered_at_ic: self.offered_at_ic.clone(),
            offered_at_utc: self.offered_at_utc,
            expires_at_utc: self.expires_at_utc,
            settle_seconds: self.settle_seconds,
        }
    }
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        digest("roll.charter_pending.offer.v1", self)
    }
    pub(crate) fn matches_birth(
        &self,
        birth: &crate::commerce::records::InitialFilingOperation,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        birth.validate()?;
        if self.birth != birth.commit
            || self.charter != birth.charter
            || self.line != birth.line
            || self.owner_term.licence != birth.licence
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
    pub(crate) fn native(&self) -> Result<CharterPendingTermsV1, PublicationError> {
        Ok(self.native_unchecked(self.digest()?))
    }
}
impl Record for PendingOffer {
    const FAMILY: &'static str = "roll_mind_charter_pending_offers";
    const MAX_BYTES: usize = 8192;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.ownership.validate()?;
        self.owner_term.validate()?;
        if self.format != 1 || self.action != super::protocol::offer_action().map_err(bad)? {
            return Err(PublicationError::Invalid);
        }
        // Digest bytes carry no authority here; the native shape check does not
        // recurse through this row's self-hash.
        self.native_unchecked(DefinitionDigest::from_bytes([255; 32]))
            .validate()
            .map_err(bad)?;
        envelope(self)
    }
}
closed!(ClaimObservation,ClaimWire,{generation:u64,next_transition:u64,open_owned:u32});
impl ClaimObservation {
    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        if self.next_transition == 0
            || self.open_owned as usize > crate::commerce::ic_claims::MAX_OPEN_IC_CLAIMS
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingRef {
    pub(crate) sequence: u64,
    pub(crate) operation: CommitToken,
    pub(crate) digest: DefinitionDigest,
}
fn required_previous<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<PendingRef>, D::Error> {
    Option::<PendingRef>::deserialize(d)
}
// Explicit null for the first head is required; omitted ancestry is never a
// legacy fallback. This wire is kept explicit rather than a serde default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PendingState {
    Funded {},
    Settled {
        terminal: CommitToken,
        transfer: CharterOwnershipRef,
    },
    Refunded {
        terminal: CommitToken,
        reason: CharterPendingDecisionKindV1,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct PendingRecord {
    pub(crate) format: u16,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) sequence: u64,
    pub(crate) previous: Option<PendingRef>,
    pub(crate) offer: CommitToken,
    pub(crate) offer_digest: DefinitionDigest,
    pub(crate) charter: CharterId,
    pub(crate) ownership: CharterOwnershipRef,
    pub(crate) owner_term: PendingOwnerTerm,
    pub(crate) accepted_at_ic: ClockStamp,
    pub(crate) deadline: ClockStamp,
    pub(crate) claim_observation: ClaimObservation,
    pub(crate) economic: CharterPendingHistory,
    pub(crate) state: PendingState,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingWire {
    format: u16,
    operation: CommitToken,
    action: DefinitionRef,
    sequence: u64,
    #[serde(deserialize_with = "required_previous")]
    previous: Option<PendingRef>,
    offer: CommitToken,
    offer_digest: DefinitionDigest,
    charter: CharterId,
    ownership: CharterOwnershipRef,
    owner_term: PendingOwnerTerm,
    accepted_at_ic: ClockStamp,
    deadline: ClockStamp,
    claim_observation: ClaimObservation,
    economic: CharterPendingHistory,
    state: PendingState,
}
impl<'de> Deserialize<'de> for PendingRecord {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = PendingWire::deserialize(d)?;
        let row = Self {
            format: w.format,
            operation: w.operation,
            action: w.action,
            sequence: w.sequence,
            previous: w.previous,
            offer: w.offer,
            offer_digest: w.offer_digest,
            charter: w.charter,
            ownership: w.ownership,
            owner_term: w.owner_term,
            accepted_at_ic: w.accepted_at_ic,
            deadline: w.deadline,
            claim_observation: w.claim_observation,
            economic: w.economic,
            state: w.state,
        };
        row.validate().map_err(serde::de::Error::custom)?;
        Ok(row)
    }
}
impl Record for PendingRecord {
    const FAMILY: &'static str = "roll_mind_charter_pending";
    const MAX_BYTES: usize = MAX_PENDING_ROW_BYTES;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.ownership.validate()?;
        self.owner_term.validate()?;
        self.claim_observation.validate()?;
        // Public strict decoder validates the complete native history; constructing
        // data never replaces the mandatory actual native lookup at publication/boot.
        let raw = mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(&self.economic).map_err(bad)?,
            mush_economy::managed_charter_transfer::MAX_PENDING_HISTORY_BYTES,
        )
        .map_err(bad)?;
        CharterPendingHistory::decode(&raw).map_err(bad)?;
        let c = self.economic.consent();
        if self.format != 1
            || self.action != super::protocol::fund_action().map_err(bad)?
            || self.operation != c.operation
            || self.action != c.effect
            || !(1..=MAX_PENDING_HISTORY).contains(&self.sequence)
            || self.offer != c.terms.offer
            || self.offer_digest != c.terms.offer_digest
            || self.charter.object() != c.terms.charter
            || self.owner_term.native() != c.terms.owner_term
            || self.ownership.generation != c.terms.ownership.generation
            || self.ownership.operation != c.terms.ownership.operation
            || self.ownership.digest != c.terms.ownership.digest
            || self.accepted_at_ic != c.accepted_at_ic
            || self.deadline != c.deadline
        {
            return Err(PublicationError::Mismatch);
        }
        match (&self.previous, self.sequence) {
            (None, 1) => (),
            (Some(p), n)
                if p.sequence.checked_add(1) == Some(n)
                    && p.operation != self.operation
                    && p.operation != self.offer =>
            {
                ()
            }
            _ => return Err(PublicationError::Mismatch),
        }
        match (&self.state, self.economic.terminal()) {
            (PendingState::Funded {}, None) => (),
            (PendingState::Settled { terminal, transfer }, Some(t))
                if t.decision().reason.settles() =>
            {
                transfer.transferred()?;
                if *terminal != t.decision().operation
                    || transfer.operation != *terminal
                    || transfer.generation
                        != self
                            .ownership
                            .generation
                            .checked_add(1)
                            .ok_or(PublicationError::Capacity)?
                    || t.decision().action != super::protocol::settle_action().map_err(bad)?
                {
                    return Err(PublicationError::Mismatch);
                }
            }
            (PendingState::Refunded { terminal, reason }, Some(t)) if !reason.settles() => {
                if *terminal != t.decision().operation || reason != &t.decision().reason {
                    return Err(PublicationError::Mismatch);
                }
                // Policy-specific Registry actions are exact-fenced by the actual paired
                // sanction producer; the native closed decoder retains that full ref.
                if !matches!(reason,CharterPendingDecisionKindV1::RegistryRevoked {sanction,..} if *sanction==*terminal)
                    && t.decision().action != super::protocol::refund_action().map_err(bad)?
                {
                    return Err(PublicationError::Mismatch);
                }
            }
            _ => return Err(PublicationError::Mismatch),
        }
        envelope(self)
    }
}
impl PendingRecord {
    pub(crate) fn funding_digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        // Exact immutable accepted fields; neither a later terminal nor a state
        // label changes the acknowledged funding root.
        let mut value = serde_json::to_value(self).map_err(bad)?;
        let object = value.as_object_mut().ok_or(PublicationError::Invalid)?;
        object.remove("state");
        object.insert(
            "economic".into(),
            serde_json::json!({
                "consent": self.economic.consent(), "funding": self.economic.funding()
            }),
        );
        digest("roll.charter_pending.funding.v1", &value)
    }
    pub(crate) fn reference(&self) -> Result<PendingRef, PublicationError> {
        self.validate()?;
        Ok(PendingRef {
            sequence: self.sequence,
            operation: self.operation,
            digest: digest("roll.charter_pending.record.v1", self)?,
        })
    }
    pub(crate) fn matches_offer(&self, offer: &PendingOffer) -> Result<(), PublicationError> {
        self.validate()?;
        if self.economic.consent().terms != offer.native()? || self.owner_term != offer.owner_term {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
closed!(PendingHead,HeadWire,{format:u16,charter:CharterId,current:PendingRef,state:PendingState});
impl Record for PendingHead {
    const FAMILY: &'static str = "roll_mind_charter_pending_head";
    const MAX_BYTES: usize = 4096;
    fn key(&self) -> String {
        self.charter.key()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1 || !(1..=MAX_PENDING_HISTORY).contains(&self.current.sequence) {
            return Err(PublicationError::Invalid);
        }
        envelope(self)
    }
}
impl PendingHead {
    pub(crate) fn of(row: &PendingRecord) -> Result<Self, PublicationError> {
        Ok(Self {
            format: 1,
            charter: row.charter,
            current: row.reference()?,
            state: row.state.clone(),
        })
    }
    pub(crate) fn matches(&self, row: &PendingRecord) -> Result<(), PublicationError> {
        self.validate()?;
        if self != &Self::of(row)? {
            Err(PublicationError::Mismatch)
        } else {
            Ok(())
        }
    }
}
