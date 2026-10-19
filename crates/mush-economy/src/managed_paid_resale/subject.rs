//! Immutable scalar and monetary consent, with bounded nested decoding.
use super::*;
use crate::managed_entitlement::{
    GrossRecipientCut, Recipient, RoyaltyRecipientCut, RoyaltyTitleRecipient,
};
use crate::managed_purchase::PurchasePayout;
use crate::{AccountId, Amount, InstrumentId};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, ObjId};
use serde::{Deserialize, Serialize};

macro_rules! closed {
    ($name:ident, $wire:ident, {$($field:ident:$ty:ty),* $(,)?}) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
        pub struct $name {$(pub $field:$ty),*}
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $wire {$($field:$ty),*}
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D:serde::Deserializer<'de>>(d:D)->Result<Self,D::Error> {
                let w=$wire::deserialize(d)?;
                let row=Self {$($field:w.$field),*};
                row.validate().map_err(serde::de::Error::custom)?;
                Ok(row)
            }
        }
    };
}
pub(super) fn world(clock: &ClockRef) -> Result<(), ParticipantError> {
    if !matches!(clock, ClockRef::WorldIc {generation} if *generation != 0) {
        return Err(refused());
    }
    Ok(())
}
pub(super) fn bounded<T: Serialize>(value: &T, limit: usize) -> Result<Vec<u8>, ParticipantError> {
    mush_core::canonical_json_bytes_with_limit(
        &serde_json::to_value(value).map_err(|_| refused())?,
        limit,
    )
    .map_err(|_| refused())
}
fn required<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaidBirthRouteV1 {
    Legacy,
    Entitlement,
    House,
}
closed!(PaidBirthRefV1,BirthWire,{route:PaidBirthRouteV1,operation:CommitToken,digest:DefinitionDigest});
impl PaidBirthRefV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        bounded(self, 1024).map(|_| ())
    }
}
closed!(PaidHolderRefV1,HolderWire,{generation:u64,operation:CommitToken,digest:DefinitionDigest});
impl PaidHolderRefV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        if !(1..=MAX_HOLDER_GENERATION).contains(&self.generation) {
            return Err(refused());
        }
        Ok(())
    }
}
closed!(PaidResaleWindowV1,WindowWire,{
    format:u16,eligibility_start:i64,issued_at:i64,term_end:i64,grace_end:i64,
});
impl PaidResaleWindowV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        let term = i128::from(self.term_end) - i128::from(self.eligibility_start);
        let grace = i128::from(self.grace_end) - i128::from(self.term_end);
        if self.format != 1
            || !(1..=7776000).contains(&term)
            || !(0..=259200).contains(&grace)
            || self.issued_at >= self.grace_end
        {
            return Err(refused());
        }
        Ok(())
    }
}
closed!(PaidResaleTermBasisV1,TermWire,{
    generation:u64,operation:CommitToken,digest:DefinitionDigest,clock:ClockRef,
    window:PaidResaleWindowV1,terms_digest:DefinitionDigest,policy_digest:DefinitionDigest,
});
impl PaidResaleTermBasisV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        if self.generation == 0 {
            return Err(refused());
        }
        world(&self.clock)?;
        self.window.validate()
    }
}
closed!(PaidResaleBasisV1,BasisWire,{
    licence:u64,charter:ObjId,line:u64,birth:PaidBirthRefV1,holder:PaidHolderRefV1,
    term:PaidResaleTermBasisV1,economics_digest:DefinitionDigest,royalty_chain_digest:DefinitionDigest,
});
impl PaidResaleBasisV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        self.birth.validate()?;
        self.holder.validate()?;
        self.term.validate()?;
        if self.licence == 0
            || self.charter.0 == 0
            || self.line == 0
            || self.holder.generation >= MAX_HOLDER_GENERATION
            || (self.holder.generation == 1
                && (self.holder.operation != self.birth.operation
                    || self.holder.digest != self.birth.digest))
        {
            return Err(refused());
        }
        Ok(())
    }
}

// New wire requires explicit nulls without weakening/changing old cut codecs.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrossWire {
    permille: u16,
    #[serde(deserialize_with = "required")]
    recipient: Option<Recipient>,
}
impl From<GrossWire> for GrossRecipientCut {
    fn from(v: GrossWire) -> Self {
        Self {
            permille: v.permille,
            recipient: v.recipient,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoyaltyWire {
    charter: ObjId,
    permille: u16,
    #[serde(deserialize_with = "required")]
    title: Option<RoyaltyTitleRecipient>,
}
fn parents<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<RoyaltyRecipientCut>, D::Error> {
    struct Bounded;
    impl<'de> serde::de::Visitor<'de> for Bounded {
        type Value = Vec<RoyaltyRecipientCut>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("at most three immutable parent cuts")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            if seq.size_hint().is_some_and(|n| n > 3) {
                return Err(serde::de::Error::custom("parent cut bound"));
            }
            let mut values = Vec::with_capacity(3);
            while let Some(v) = seq.next_element::<RoyaltyWire>()? {
                if values.len() == 3 {
                    return Err(serde::de::Error::custom("parent cut bound"));
                }
                values.push(RoyaltyRecipientCut {
                    charter: v.charter,
                    permille: v.permille,
                    title: v.title,
                });
            }
            Ok(values)
        }
    }
    d.deserialize_seq(Bounded)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PaidResaleTermsV1 {
    pub format: u16,
    pub offer: CommitToken,
    pub offer_action: DefinitionRef,
    pub basis: PaidResaleBasisV1,
    pub seller: ObjId,
    pub buyer: ObjId,
    pub instrument: InstrumentId,
    pub price: Amount,
    pub registry: GrossRecipientCut,
    pub venue: Option<GrossRecipientCut>,
    pub parents: Vec<RoyaltyRecipientCut>,
    pub remainder: Recipient,
    pub offered_at_ic: ClockStamp,
    pub offered_at_utc: i64,
    pub expires_at_utc: i64,
    pub settle_seconds: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TermsWire {
    format: u16,
    offer: CommitToken,
    offer_action: DefinitionRef,
    basis: PaidResaleBasisV1,
    seller: ObjId,
    buyer: ObjId,
    instrument: InstrumentId,
    price: Amount,
    registry: GrossWire,
    #[serde(deserialize_with = "required")]
    venue: Option<GrossWire>,
    #[serde(deserialize_with = "parents")]
    parents: Vec<RoyaltyRecipientCut>,
    remainder: Recipient,
    offered_at_ic: ClockStamp,
    offered_at_utc: i64,
    expires_at_utc: i64,
    settle_seconds: i64,
}
impl<'de> Deserialize<'de> for PaidResaleTermsV1 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = TermsWire::deserialize(d)?;
        let row = Self {
            format: w.format,
            offer: w.offer,
            offer_action: w.offer_action,
            basis: w.basis,
            seller: w.seller,
            buyer: w.buyer,
            instrument: w.instrument,
            price: w.price,
            registry: w.registry.into(),
            venue: w.venue.map(Into::into),
            parents: w.parents,
            remainder: w.remainder,
            offered_at_ic: w.offered_at_ic,
            offered_at_utc: w.offered_at_utc,
            expires_at_utc: w.expires_at_utc,
            settle_seconds: w.settle_seconds,
        };
        row.validate().map_err(serde::de::Error::custom)?;
        Ok(row)
    }
}
impl PaidResaleTermsV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        self.basis.validate()?;
        world(&self.offered_at_ic.clock)?;
        let duration = i128::from(self.expires_at_utc) - i128::from(self.offered_at_utc);
        if self.format != 1
            || !action_matches(&self.offer_action, OFFER)
            || self.seller.0 == 0
            || self.buyer.0 == 0
            || self.seller == self.buyer
            || self.price.minor() <= 0
            || !(1..=i128::from(MAX_OFFER_SECONDS)).contains(&duration)
            || self.settle_seconds != SETTLEMENT_IC_SECONDS
            || self.parents.len() > 3
            || self.offer == self.basis.birth.operation
            || self.offer == self.basis.holder.operation
            || self.offer == self.basis.term.operation
            || self.remainder.owner != mush_org::PrincipalId::Object(self.seller)
        {
            return Err(refused());
        }
        validate_cuts(self)?;
        bounded(self, MAX_SUBJECT_BYTES)?;
        Ok(())
    }
    pub fn deadline(&self, accepted: &ClockStamp) -> Result<ClockStamp, ParticipantError> {
        self.validate()?;
        world(&accepted.clock)?;
        if accepted.clock != self.offered_at_ic.clock || accepted.at < self.offered_at_ic.at {
            return Err(refused());
        }
        let at = i128::from(self.basis.term.window.term_end)
            .min(i128::from(accepted.at) + i128::from(self.settle_seconds));
        Ok(ClockStamp {
            clock: accepted.clock.clone(),
            at: i64::try_from(at).map_err(|_| refused())?,
        })
    }
    pub fn payouts(&self, payer: AccountId) -> Result<Vec<PurchasePayout>, ParticipantError> {
        self.validate()?;
        // Same checked floor-before-merge calculation; no charter-owner premise
        // is borrowed by this property transfer. The seller owns the licence.
        crate::managed_entitlement::subject::GrossMoneySplit {
            buyer: self.buyer,
            seller: self.seller,
            charter: self.basis.charter,
            payer,
            price: self.price,
            registry: &self.registry,
            venue: self.venue.as_ref(),
            parents: &self.parents,
            remainder: &self.remainder,
        }
        .payouts()
    }
}
fn validate_cuts(t: &PaidResaleTermsV1) -> Result<(), ParticipantError> {
    let mut total = 0u32;
    let mut parent_total = 0u32;
    let mut charters = std::collections::BTreeSet::from([t.basis.charter]);
    let mut accounts = std::collections::BTreeMap::new();
    let mut identity = |r: &Recipient| -> Result<(), ParticipantError> {
        if accounts
            .insert(r.account, r.owner.clone())
            .is_some_and(|old| old != r.owner)
        {
            return Err(refused());
        }
        Ok(())
    };
    identity(&t.remainder)?;
    for c in std::iter::once(&t.registry).chain(t.venue.iter()) {
        if c.permille > 1000 || (c.permille == 0) != c.recipient.is_none() {
            return Err(refused());
        }
        total = total
            .checked_add(u32::from(c.permille))
            .ok_or_else(refused)?;
        if let Some(r) = &c.recipient {
            identity(r)?;
        }
    }
    for c in &t.parents {
        if c.charter.0 == 0 || !charters.insert(c.charter) || (c.permille == 0) != c.title.is_none()
        {
            return Err(refused());
        }
        parent_total = parent_total
            .checked_add(u32::from(c.permille))
            .ok_or_else(refused)?;
        if let Some(title) = &c.title {
            if title.title_revision == 0
                || !matches!(title.recipient.owner,mush_org::PrincipalId::Object(id) if id.0!=0)
            {
                return Err(refused());
            }
            identity(&title.recipient)?;
        }
    }
    if parent_total > 300 || total.checked_add(parent_total).is_none_or(|n| n > 1000) {
        return Err(refused());
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaidResaleModeV1 {
    Immediate,
    Pending,
}
closed!(PaidResaleClaimsV1,ClaimsWire,{next_transition:u64,open_paid:u32});
impl PaidResaleClaimsV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        if self.next_transition == 0 || self.open_paid > 32768 {
            return Err(refused());
        }
        Ok(())
    }
}
closed!(PaidResaleConsentV1,ConsentWire,{
    format:u16,operation:CommitToken,effect:DefinitionRef,offer_digest:DefinitionDigest,
    terms:PaidResaleTermsV1,payer:AccountId,mode:PaidResaleModeV1,accepted_at_ic:ClockStamp,
    accepted_at_utc:i64,deadline:ClockStamp,claims:PaidResaleClaimsV1,
});
impl PaidResaleConsentV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        self.terms.validate()?;
        self.claims.validate()?;
        let immediate = self.mode == PaidResaleModeV1::Immediate;
        if self.format != 1
            || !action_matches(&self.effect, if immediate { ACCEPT } else { FUND })
            || immediate != (self.claims.open_paid == 0)
            || self.operation == self.terms.offer
            || self.operation == self.terms.basis.birth.operation
            || self.operation == self.terms.basis.holder.operation
            || self.operation == self.terms.basis.term.operation
            || self.accepted_at_utc < self.terms.offered_at_utc
            || self.accepted_at_utc >= self.terms.expires_at_utc
            || self.deadline != self.terms.deadline(&self.accepted_at_ic)?
        {
            return Err(refused());
        }
        self.terms.payouts(self.payer)?;
        bounded(self, MAX_SUBJECT_BYTES)?;
        Ok(())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value_with_limit(
            "economy.paid_resale.consent.v1",
            &serde_json::to_value(self).map_err(|_| refused())?,
            MAX_SUBJECT_BYTES,
        )
        .map_err(|_| refused())
    }
    pub fn decode(raw: &[u8]) -> Result<Self, ParticipantError> {
        if raw.is_empty() || raw.len() > MAX_SUBJECT_BYTES {
            return Err(refused());
        }
        let row: Self = serde_json::from_slice(raw).map_err(|_| refused())?;
        if bounded(&row, MAX_SUBJECT_BYTES)? != raw {
            return Err(refused());
        }
        row.validate()?;
        Ok(row)
    }
    pub fn deadline_reached(&self, now: &ClockStamp) -> Result<bool, ParticipantError> {
        self.validate()?;
        world(&now.clock)?;
        if now.clock != self.accepted_at_ic.clock || now.at < self.accepted_at_ic.at {
            return Err(refused());
        }
        Ok(now.at >= self.deadline.at)
    }
}
