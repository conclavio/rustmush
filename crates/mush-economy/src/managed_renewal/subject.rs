//! Source-free renewal consent data. Decoding conveys no commercial authority.

use crate::Holder;
use mush_core::checked;
use mush_core::participant::ParticipantError;
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, Flags, ObjId};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};

use crate::managed_entitlement::{
    EconomicRecipe, GrossRecipientCut, Recipient, RoyaltyRecipientCut, RoyaltyTitleRecipient,
};
use crate::managed_purchase::PurchasePayout;
use crate::module::EconomyStore;
use crate::record::{AccountStatus, AssetLifecycle, InstrumentStatus};
use crate::renewal_twin::wire::{gross_cut, required_option, venue_cut};
use crate::renewal_twin::{player, refused};
use crate::{AccountId, Amount, InstrumentId};

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

checked!(pub struct RenewalBasisV1, BasisWire, pub fields, {
    format:u16, licence:u64, predecessor_generation:u64, next_generation:u64,
    predecessor_operation:CommitToken, prior_quote:CommitToken,
    prior_quote_digest:DefinitionDigest, product_digest:DefinitionDigest,
    template_digest:DefinitionDigest, grant_economics_digest:DefinitionDigest,
    royalty_chain_digest:DefinitionDigest,
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenewalLineageV1 {
    Original,
    Derived,
}

checked!(pub struct RenewalOfferTermsV1, TermsWire, pub fields, attrs on wire, {
    format:u16, lineage:RenewalLineageV1, buyer:Holder, seller:ObjId,
    charter:ObjId, title_revision:u64, payer:AccountId, instrument:InstrumentId,
    price:Amount,
    #[serde(deserialize_with="gross_cut")] registry:GrossRecipientCut,
    #[serde(deserialize_with="venue_cut")] venue:Option<GrossRecipientCut>,
    #[serde(deserialize_with="parent_cuts")] parents:Vec<RoyaltyRecipientCut>, remainder:Recipient,
    not_before:ClockStamp, expires_at:ClockStamp,
});
impl RenewalOfferTermsV1 {
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
        // M8c B3: an organisation holder renews through its acting character.
        if state.frozen
            || !player(world, self.buyer.acting_character())
            || !player(world, self.seller)
        {
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
        // M8c B3: the payer is the holder principal's account: a character's
        // personal one, or the organisation's treasury on one of its scopes.
        if account.owner != self.buyer.principal()
            || !crate::holder::payer_scope_fits(world, self.buyer, account.scope)
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

checked!(pub struct RenewalOffer, OfferWire, pub fields, { format:u16, basis:RenewalBasisV1, terms:RenewalOfferTermsV1 });

crate::renewal_twin::subject_records!(crate::renewal_twin::Personal);

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
