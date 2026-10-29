//! Source-free renewal consent data. Decoding conveys no commercial authority.

use crate::Holder;
use mush_core::participant::ParticipantError;
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, Flags, ObjId, ObjKind};
use mush_org::{OrgId, OrgScopeId, OrgStatus, OrgStore, PrincipalId, ScopeStatus};
use serde::{Deserialize, Serialize};
use std::{any::TypeId, collections::BTreeMap};

use super::{MAX_RENEWAL_OFFER_IC_SECONDS, refused};
use crate::managed_entitlement::{GrossRecipientCut, Recipient};
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
record!(RenewalBasisV1,BasisWire,{
    format:u16, licence:u64, predecessor_generation:u64, next_generation:u64,
    predecessor_operation:CommitToken, prior_quote:CommitToken,
    prior_quote_digest:DefinitionDigest, product_digest:DefinitionDigest,
    template_digest:DefinitionDigest, grant_economics_digest:DefinitionDigest,
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

record!(RenewalOfferTermsV1,TermsWire,{
    format:u16, buyer:Holder, organization:OrgId, scope:OrgScopeId,
    charter:ObjId, title_revision:u64, payer:AccountId, instrument:InstrumentId,
    price:Amount,
    #[serde(deserialize_with="gross_cut")] registry:GrossRecipientCut,
    #[serde(deserialize_with="venue_cut")] venue:Option<GrossRecipientCut>,
    remainder:Recipient,
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
        if self.buyer.is_nobody()
            || self.charter.0 == 0
            || self.title_revision == 0
            || self.price.is_negative()
            || self.price.is_zero()
            || self.remainder.owner != PrincipalId::Organization(self.organization)
        {
            return Err(refused());
        }
        let mut owners = BTreeMap::new();
        for r in self.recipients() {
            if r.account == self.payer
                || owners
                    .insert(r.account, &r.owner)
                    .is_some_and(|old| old != &r.owner)
            {
                return Err(refused());
            }
        }
        let mut amounts = BTreeMap::<AccountId, (PrincipalId, Amount)>::new();
        let mut assigned = Amount::ZERO;
        let mut rate = 0_u32;
        for cut in std::iter::once(&self.registry).chain(self.venue.as_ref()) {
            if cut.permille > 1000 || (cut.permille == 0) != cut.recipient.is_none() {
                return Err(refused());
            }
            rate = rate
                .checked_add(u32::from(cut.permille))
                .ok_or_else(refused)?;
            let amount = Amount::from_widened(
                self.price
                    .widen()
                    .checked_mul(i128::from(cut.permille))
                    .ok_or_else(refused)?
                    / 1000,
            )
            .ok_or_else(refused)?;
            assigned = assigned.checked_add(amount).ok_or_else(refused)?;
            if let Some(r) = &cut.recipient {
                merge(&mut amounts, r, amount)?;
            }
        }
        if rate > 1000 {
            return Err(refused());
        }
        merge(
            &mut amounts,
            &self.remainder,
            self.price.checked_sub(assigned).ok_or_else(refused)?,
        )?;
        if amounts.is_empty() || amounts.len() > crate::managed_house_purchase::MAX_PAYOUTS {
            return Err(refused());
        }
        Ok(amounts
            .into_iter()
            .map(|(account, (owner, amount))| PurchasePayout {
                account,
                owner,
                amount,
            })
            .collect())
    }
    pub(crate) fn recipients(&self) -> Vec<&Recipient> {
        let mut out = Vec::with_capacity(crate::managed_house_purchase::MAX_PAYOUTS);
        out.extend(self.registry.recipient.iter());
        out.extend(self.venue.iter().flat_map(|c| c.recipient.iter()));
        out.push(&self.remainder);
        out
    }
    pub(crate) fn check_current(
        &self,
        state: &EconomyStore,
        world: &mush_core::World,
    ) -> Result<(), ParticipantError> {
        self.validate()?;
        if state.frozen
            || !player(world, self.buyer.acting_character())
            || world
                .get(self.charter)
                .is_none_or(|p| p.flags.contains(Flags::GOING))
            || world
                .module_state_owner(TypeId::of::<OrgStore>())
                .is_none_or(|o| o.as_str() != "org")
        {
            return Err(refused());
        }
        let orgs = world.module_state::<OrgStore>().ok_or_else(refused)?;
        let org = orgs.state().org(self.organization).ok_or_else(refused)?;
        let scope = orgs.state().scope(self.scope).ok_or_else(refused)?;
        if org.status != OrgStatus::Active
            || scope.org != org.id
            || scope.status != ScopeStatus::Active
        {
            return Err(refused());
        }
        let snapshot = &state.snapshot;
        let title = snapshot.asset(self.charter).ok_or_else(refused)?;
        if title.title_owner != PrincipalId::Organization(self.organization)
            || title.scope != Some(self.scope)
            || title.revision != self.title_revision
            || title.lifecycle != AssetLifecycle::Active
            || title.frozen
            || title.stolen.is_some()
        {
            return Err(refused());
        }
        let payer = snapshot.account(self.payer).ok_or_else(refused)?;
        // M8c B3: the payer is the holder principal's account: a character's
        // personal one, or the organisation's treasury on one of its scopes.
        if payer.owner != self.buyer.principal()
            || !crate::holder::payer_scope_fits(world, self.buyer, payer.scope)
            || payer.instrument != self.instrument
            || payer.status != AccountStatus::Active
            || payer.approval.is_some()
            || snapshot
                .instrument(&self.instrument)
                .is_none_or(|i| i.status != InstrumentStatus::Active || !i.transferable)
        {
            return Err(refused());
        }
        for recipient in self.recipients() {
            let account = snapshot.account(recipient.account).ok_or_else(refused)?;
            if account.owner != recipient.owner
                || account.instrument != self.instrument
                || account.status != AccountStatus::Active
            {
                return Err(refused());
            }
        }
        if snapshot
            .account(self.remainder.account)
            .ok_or_else(refused)?
            .scope
            != Some(self.scope)
        {
            return Err(refused());
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
            "economy.house_mind_renewal_offer.v1",
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
fn merge(
    amounts: &mut BTreeMap<AccountId, (PrincipalId, Amount)>,
    r: &Recipient,
    amount: Amount,
) -> Result<(), ParticipantError> {
    if amount.is_negative() {
        return Err(refused());
    }
    if amount.is_zero() {
        return Ok(());
    }
    match amounts.entry(r.account) {
        std::collections::btree_map::Entry::Vacant(v) => {
            v.insert((r.owner.clone(), amount));
        }
        std::collections::btree_map::Entry::Occupied(mut o) => {
            let (owner, prior) = o.get_mut();
            if owner != &r.owner {
                return Err(refused());
            }
            *prior = prior.checked_add(amount).ok_or_else(refused)?;
        }
    }
    Ok(())
}
