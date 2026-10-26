//! Source-free renewal consent data. Decoding conveys no commercial authority.

use mush_core::participant::ParticipantError;
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, Flags, ObjId, ObjKind};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};

use super::{MAX_RENEWAL_OFFER_IC_SECONDS, refused};
use crate::managed_entitlement::{
    EconomicRecipe, GrossRecipientCut, Recipient, RoyaltyRecipientCut, RoyaltyTitleRecipient,
};
use crate::managed_purchase::PurchasePayout;
use crate::module::EconomyStore;
use crate::record::{AccountStatus, AssetLifecycle, InstrumentStatus};
use crate::{AccountId, Amount, InstrumentId};

macro_rules! record {
    ($name:ident,$wire:ident,{$($(#[$attr:meta])* $field:ident:$ty:ty),* $(,)?}) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
        pub struct $name { $(pub $field:$ty),* }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $wire { $($(#[$attr])* $field:$ty),* }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D:serde::Deserializer<'de>>(d:D)->Result<Self,D::Error> {
                let w=$wire::deserialize(d)?;
                let value=Self {$($field:w.$field),*};
                value.validate().map_err(serde::de::Error::custom)?;
                Ok(value)
            }
        }
    }
}

fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

// New renewal wire requires explicit nulls in every optional economic role.
// The shared C structs and their historical wire remain unchanged.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrossWire {
    permille: u16,
    #[serde(deserialize_with = "required_option")]
    recipient: Option<Recipient>,
}
impl From<GrossWire> for GrossRecipientCut {
    fn from(w: GrossWire) -> Self {
        Self {
            permille: w.permille,
            recipient: w.recipient,
        }
    }
}
fn gross_cut<'de, D: serde::Deserializer<'de>>(d: D) -> Result<GrossRecipientCut, D::Error> {
    GrossWire::deserialize(d).map(Into::into)
}
fn venue_cut<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<GrossRecipientCut>, D::Error> {
    Option::<GrossWire>::deserialize(d).map(|v| v.map(Into::into))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentWire {
    charter: ObjId,
    permille: u16,
    #[serde(deserialize_with = "required_option")]
    title: Option<RoyaltyTitleRecipient>,
}
fn parent_cuts<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<RoyaltyRecipientCut>, D::Error> {
    struct Parents;
    impl<'de> serde::de::Visitor<'de> for Parents {
        type Value = Vec<RoyaltyRecipientCut>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("at most three royalty parents")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let limit = crate::managed_entitlement::MAX_PARENT_CUTS;
            if seq.size_hint().is_some_and(|n| n > limit) {
                return Err(serde::de::Error::custom("too many royalty parents"));
            }
            let mut rows = Vec::with_capacity(limit);
            while let Some(w) = seq.next_element::<ParentWire>()? {
                if rows.len() == limit {
                    return Err(serde::de::Error::custom("too many royalty parents"));
                }
                rows.push(RoyaltyRecipientCut {
                    charter: w.charter,
                    permille: w.permille,
                    title: w.title,
                });
            }
            Ok(rows)
        }
    }
    d.deserialize_seq(Parents)
}

record!(RenewalBasisV1,BasisWire,{
    format:u16, licence:u64, predecessor_generation:u64, next_generation:u64,
    predecessor_operation:CommitToken, prior_quote:CommitToken,
    prior_quote_digest:DefinitionDigest, product_digest:DefinitionDigest,
    template_digest:DefinitionDigest, grant_economics_digest:DefinitionDigest,
    royalty_chain_digest:DefinitionDigest,
});
impl RenewalBasisV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1
            || self.licence == 0
            || self.predecessor_generation == 0
            || self.predecessor_generation.checked_add(1) != Some(self.next_generation)
            || self.predecessor_operation == self.prior_quote
        {
            return Err(refused());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenewalLineageV1 {
    Original,
    Derived,
}

record!(RenewalOfferTermsV1,TermsWire,{
    format:u16, lineage:RenewalLineageV1, buyer:ObjId, seller:ObjId,
    charter:ObjId, title_revision:u64, payer:AccountId, instrument:InstrumentId,
    price:Amount,
    #[serde(deserialize_with="gross_cut")] registry:GrossRecipientCut,
    #[serde(deserialize_with="venue_cut")] venue:Option<GrossRecipientCut>,
    #[serde(deserialize_with="parent_cuts")] parents:Vec<RoyaltyRecipientCut>, remainder:Recipient,
    not_before:ClockStamp, expires_at:ClockStamp,
});
impl RenewalOfferTermsV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1
            || !matches!(self.not_before.clock,ClockRef::WorldIc{generation} if generation != 0)
            || self.not_before.clock != self.expires_at.clock
            || self
                .expires_at
                .at
                .checked_sub(self.not_before.at)
                .is_none_or(|n| !(1..=MAX_RENEWAL_OFFER_IC_SECONDS).contains(&n))
        {
            return Err(refused());
        }
        self.payouts()?;
        Ok(())
    }
    pub fn payouts(&self) -> Result<Vec<PurchasePayout>, ParticipantError> {
        EconomicRecipe {
            requires_parents: self.lineage == RenewalLineageV1::Derived,
            buyer: self.buyer,
            seller: self.seller,
            charter: self.charter,
            title_revision: self.title_revision,
            payer: self.payer,
            price: self.price,
            registry: &self.registry,
            venue: self.venue.as_ref(),
            parents: &self.parents,
            remainder: &self.remainder,
        }
        .payouts()
    }
    pub(crate) fn recipients(&self) -> Vec<&Recipient> {
        let mut recipients = Vec::with_capacity(crate::managed_entitlement::MAX_PAYOUTS);
        recipients.extend(self.registry.recipient.iter());
        recipients.extend(self.venue.iter().flat_map(|c| c.recipient.iter()));
        recipients.extend(
            self.parents
                .iter()
                .filter_map(|c| c.title.as_ref().map(|t| &t.recipient)),
        );
        recipients.push(&self.remainder);
        recipients
    }
    pub(crate) fn check_current(
        &self,
        state: &EconomyStore,
        world: &mush_core::World,
    ) -> Result<(), ParticipantError> {
        self.validate()?;
        if state.frozen || !player(world, self.buyer) || !player(world, self.seller) {
            return Err(refused());
        }
        title(
            state,
            world,
            self.charter,
            self.title_revision,
            &PrincipalId::Object(self.seller),
        )?;
        for parent in &self.parents {
            if let Some(t) = &parent.title {
                title(
                    state,
                    world,
                    parent.charter,
                    t.title_revision,
                    &t.recipient.owner,
                )?;
            }
        }
        let account = state.snapshot.account(self.payer).ok_or_else(refused)?;
        if account.owner != PrincipalId::Object(self.buyer)
            || account.scope.is_some()
            || account.instrument != self.instrument
            || account.status != AccountStatus::Active
            || account.approval.is_some()
            || state
                .snapshot
                .instrument(&self.instrument)
                .is_none_or(|i| i.status != InstrumentStatus::Active || !i.transferable)
        {
            return Err(refused());
        }
        for recipient in self.recipients() {
            let account = state
                .snapshot
                .account(recipient.account)
                .ok_or_else(refused)?;
            if account.owner != recipient.owner
                || account.instrument != self.instrument
                || account.status != AccountStatus::Active
            {
                return Err(refused());
            }
        }
        Ok(())
    }
}

record!(RenewalOffer,OfferWire,{ format:u16, basis:RenewalBasisV1, terms:RenewalOfferTermsV1 });
impl RenewalOffer {
    /// Plain data only. Public byte ingress must use this bounded, typed-first
    /// decoder; Deserialize exists for the already bounded owner document.
    pub fn decode(bytes: &[u8]) -> Result<Self, ParticipantError> {
        if bytes.len() > super::MAX_RENEWAL_OFFER_BYTES {
            return Err(refused());
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| refused())?;
        value.validate()?;
        let raw: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| refused())?;
        if serde_json::to_value(&value).map_err(|_| ParticipantError::Encoding)? != raw {
            return Err(refused());
        }
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1 {
            return Err(refused());
        }
        self.basis.validate()?;
        self.terms.validate()
    }
    /// Canonical identity of validated plain data, never economic authority.
    /// Paired owners compare it with the digest in the opaque prepared output.
    pub fn digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "economy.mind_renewal_offer.v1",
            &serde_json::to_value(self).map_err(|_| ParticipantError::Encoding)?,
        )
        .map_err(|_| refused())
    }
}

pub(crate) fn player(world: &mush_core::World, id: ObjId) -> bool {
    world
        .get(id)
        .is_some_and(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
}
fn title(
    state: &EconomyStore,
    world: &mush_core::World,
    id: ObjId,
    revision: u64,
    owner: &PrincipalId,
) -> Result<(), ParticipantError> {
    if !matches!(owner,PrincipalId::Object(person) if player(world,*person))
        || world.get(id).is_none_or(|o| o.flags.contains(Flags::GOING))
    {
        return Err(refused());
    }
    let title = state.snapshot.asset(id).ok_or_else(refused)?;
    if title.title_owner != *owner
        || title.scope.is_some()
        || title.revision != revision
        || title.lifecycle != AssetLifecycle::Active
        || title.frozen
        || title.stolen.is_some()
    {
        return Err(refused());
    }
    Ok(())
}
