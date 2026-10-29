//! Bounded economic terms, not a source/parent licence or posting capability.

use crate::Holder;
use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, Flags, ObjId, ObjKind};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};

use super::{MAX_PARENT_CUTS, MAX_PARENT_PERMILLE, MAX_PAYOUTS, refused};
use crate::managed_purchase::{FundingContext, PurchasePayout};
use crate::module::EconomyStore;
use crate::record::{AccountStatus, AssetLifecycle, InstrumentStatus};
use crate::{AccountId, Amount, InstrumentId, PERMILLE};
use mush_core::participant::ParticipantError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntitlementProtocol {
    OriginalV3,
    DerivedV1,
}
impl EntitlementProtocol {
    pub const fn version(self) -> u32 {
        match self {
            Self::OriginalV3 => 3,
            Self::DerivedV1 => 1,
        }
    }
    pub const fn name(self) -> &'static str {
        match self {
            Self::OriginalV3 => "original_mind_purchase_v3",
            Self::DerivedV1 => "derived_mind_purchase_v1",
        }
    }
    pub(crate) fn action(self, transition: &str) -> Option<&'static str> {
        match (self, transition) {
            (Self::OriginalV3, "fund") => Some("purchase_original_mind"),
            (Self::OriginalV3, "grant") => Some("settle_original_mind"),
            (Self::OriginalV3, "refund") => Some("refund_original_mind"),
            (Self::DerivedV1, "fund") => Some("purchase_derived_mind"),
            (Self::DerivedV1, "grant") => Some("settle_derived_mind"),
            (Self::DerivedV1, "refund") => Some("refund_derived_mind"),
            _ => None,
        }
    }
    pub(crate) fn matches_action(self, action: &DefinitionRef, transition: &str) -> bool {
        action.key().owner.as_str() == "market"
            && action.key().kind.as_str() == "action"
            && Some(action.key().id.as_str()) == self.action(transition)
            && action.version() == self.version()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipient {
    pub account: AccountId,
    pub owner: PrincipalId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrossRecipientCut {
    pub permille: u16,
    /// Zero rates have no invented account; positive rates require a recipient.
    pub recipient: Option<Recipient>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoyaltyTitleRecipient {
    pub title_revision: u64,
    pub recipient: Recipient,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoyaltyRecipientCut {
    pub charter: ObjId,
    pub permille: u16,
    /// The immutable zero edge is retained even though it has no monetary row.
    pub title: Option<RoyaltyTitleRecipient>,
}

/// Only a registered owner participant consumes this data. It has no decoder,
/// authority callback, parent-rights flag, arbitrary plan or source payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntitlementPurchase {
    pub protocol: EntitlementProtocol,
    pub quote: CommitToken,
    pub quote_digest: DefinitionDigest,
    pub product_digest: DefinitionDigest,
    /// Doc 17h §3 M8c (B1): who the licence is bought for; a character's
    /// bytes are unchanged.
    pub buyer: Holder,
    pub seller: ObjId,
    pub charter: ObjId,
    pub title_revision: u64,
    pub payer: AccountId,
    pub instrument: InstrumentId,
    pub price: Amount,
    pub registry: GrossRecipientCut,
    pub venue: Option<GrossRecipientCut>,
    pub parents: Vec<RoyaltyRecipientCut>,
    pub remainder: Recipient,
    pub royalty_chain_digest: DefinitionDigest,
    pub grant_economics_digest: DefinitionDigest,
    pub quoted_at_utc: i64,
    pub expires_at_utc: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntitlementWire {
    protocol: EntitlementProtocol,
    quote: CommitToken,
    quote_digest: DefinitionDigest,
    product_digest: DefinitionDigest,
    buyer: Holder,
    seller: ObjId,
    charter: ObjId,
    title_revision: u64,
    payer: AccountId,
    instrument: InstrumentId,
    price: Amount,
    registry: GrossRecipientCut,
    venue: Option<GrossRecipientCut>,
    parents: Vec<RoyaltyRecipientCut>,
    remainder: Recipient,
    royalty_chain_digest: DefinitionDigest,
    grant_economics_digest: DefinitionDigest,
    quoted_at_utc: i64,
    expires_at_utc: i64,
}
impl From<&EntitlementPurchase> for EntitlementWire {
    fn from(p: &EntitlementPurchase) -> Self {
        Self {
            protocol: p.protocol,
            quote: p.quote,
            quote_digest: p.quote_digest,
            product_digest: p.product_digest,
            buyer: p.buyer,
            seller: p.seller,
            charter: p.charter,
            title_revision: p.title_revision,
            payer: p.payer,
            instrument: p.instrument.clone(),
            price: p.price,
            registry: p.registry.clone(),
            venue: p.venue.clone(),
            parents: p.parents.clone(),
            remainder: p.remainder.clone(),
            royalty_chain_digest: p.royalty_chain_digest,
            grant_economics_digest: p.grant_economics_digest,
            quoted_at_utc: p.quoted_at_utc,
            expires_at_utc: p.expires_at_utc,
        }
    }
}
impl EntitlementWire {
    pub(crate) fn data(&self) -> Result<EntitlementPurchase, ParticipantError> {
        let p = EntitlementPurchase {
            protocol: self.protocol,
            quote: self.quote,
            quote_digest: self.quote_digest,
            product_digest: self.product_digest,
            buyer: self.buyer,
            seller: self.seller,
            charter: self.charter,
            title_revision: self.title_revision,
            payer: self.payer,
            instrument: self.instrument.clone(),
            price: self.price,
            registry: self.registry.clone(),
            venue: self.venue.clone(),
            parents: self.parents.clone(),
            remainder: self.remainder.clone(),
            royalty_chain_digest: self.royalty_chain_digest,
            grant_economics_digest: self.grant_economics_digest,
            quoted_at_utc: self.quoted_at_utc,
            expires_at_utc: self.expires_at_utc,
        };
        p.payouts()?;
        Ok(p)
    }
}

impl EntitlementPurchase {
    pub fn economic_digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        self.payouts()?;
        DefinitionDigest::for_canonical_value(
            "economy.mind_entitlement.v1",
            &serde_json::to_value(EntitlementWire::from(self)).map_err(|_| refused())?,
        )
        .map_err(|_| refused())
    }

    /// Full-gross, role-by-role floors; this result alone cannot post money.
    pub fn payouts(&self) -> Result<Vec<PurchasePayout>, ParticipantError> {
        if self
            .expires_at_utc
            .checked_sub(self.quoted_at_utc)
            .is_none_or(|n| !(1..=600).contains(&n))
        {
            return Err(refused());
        }
        EconomicRecipe {
            requires_parents: self.protocol == EntitlementProtocol::DerivedV1,
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

    pub(crate) fn check_current(
        &self,
        state: &EconomyStore,
        context: &FundingContext<'_>,
    ) -> Result<(), ParticipantError> {
        self.payouts()?;
        let world = context.world;
        // M8c B1/B3: the payer is the holder's principal's account (a
        // personal account, or the organisation's treasury on one of its
        // scopes). F2: a character's purchase is funded by that character;
        // an organisation's by the member who quoted it (doc 17h §3 M8f H1,
        // the lead's ruling), the market's provider having re-asked the
        // organisation's grant for this actor (`org_holder::acts_for`, with
        // the price) before this participant runs.
        let actor_fits = match self.buyer {
            crate::Holder::Character(c) => context.actor == c,
            crate::Holder::Organization { acting, .. } => context.actor == acting,
        };
        if state.frozen
            || !self.protocol.matches_action(context.action, "fund")
            || !actor_fits
            || context.principal != &(mush_core::PrincipalId::Object { id: context.actor })
            || self.quote == context.commit
            || world.now() < self.quoted_at_utc
            || world.now() >= self.expires_at_utc
            || [context.actor, self.seller]
                .iter()
                .any(|id| !player(world, *id))
        {
            return Err(refused());
        }
        check_title(
            state,
            world,
            self.charter,
            self.title_revision,
            &PrincipalId::Object(self.seller),
        )?;
        for cut in &self.parents {
            if let Some(title) = &cut.title {
                check_title(
                    state,
                    world,
                    cut.charter,
                    title.title_revision,
                    &title.recipient.owner,
                )?;
            }
        }
        let snapshot = &state.snapshot;
        let payer = snapshot.account(self.payer).ok_or_else(refused)?;
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
        Ok(())
    }

    pub(crate) fn recipients(&self) -> Vec<&Recipient> {
        let mut roles = Vec::with_capacity(MAX_PAYOUTS);
        roles.extend(self.registry.recipient.iter());
        roles.extend(self.venue.iter().flat_map(|cut| cut.recipient.iter()));
        roles.extend(
            self.parents
                .iter()
                .filter_map(|cut| cut.title.as_ref().map(|t| &t.recipient)),
        );
        roles.push(&self.remainder);
        roles
    }
}
fn player(world: &mush_core::World, id: ObjId) -> bool {
    world
        .get(id)
        .is_some_and(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
}
fn check_title(
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
fn add_cut(
    gross: Amount,
    rate: u16,
    to: Option<&Recipient>,
    total: &mut u32,
    assigned: &mut Amount,
    amounts: &mut BTreeMap<AccountId, (PrincipalId, Amount)>,
) -> Result<(), ParticipantError> {
    if u32::from(rate) > PERMILLE {
        return Err(refused());
    }
    *total = total.checked_add(u32::from(rate)).ok_or_else(refused)?;
    let amount = Amount::from_widened(
        gross
            .widen()
            .checked_mul(i128::from(rate))
            .ok_or_else(refused)?
            / i128::from(PERMILLE),
    )
    .ok_or_else(refused)?;
    *assigned = assigned.checked_add(amount).ok_or_else(refused)?;
    if let Some(to) = to {
        merge(amounts, to, amount)?;
    }
    Ok(())
}
fn merge(
    amounts: &mut BTreeMap<AccountId, (PrincipalId, Amount)>,
    to: &Recipient,
    amount: Amount,
) -> Result<(), ParticipantError> {
    if amount.is_negative() {
        return Err(refused());
    }
    if amount.is_zero() {
        return Ok(());
    }
    let previous = amounts.get(&to.account);
    if previous.is_some_and(|(owner, _)| owner != &to.owner) {
        return Err(refused());
    }
    let value = previous
        .map_or(Amount::ZERO, |(_, n)| *n)
        .checked_add(amount)
        .ok_or_else(refused)?;
    amounts.insert(to.account, (to.owner.clone(), value));
    Ok(())
}

/// Borrowed, pure economic recipe shared by purchase and renewal preparation.
/// It contains no action, quote, clock, credential or posting capability.
pub(crate) struct EconomicRecipe<'a> {
    pub requires_parents: bool,
    pub buyer: Holder,
    pub seller: ObjId,
    pub charter: ObjId,
    pub title_revision: u64,
    pub payer: AccountId,
    pub price: Amount,
    pub registry: &'a GrossRecipientCut,
    pub venue: Option<&'a GrossRecipientCut>,
    pub parents: &'a [RoyaltyRecipientCut],
    pub remainder: &'a Recipient,
}
impl EconomicRecipe<'_> {
    pub(crate) fn payouts(&self) -> Result<Vec<PurchasePayout>, ParticipantError> {
        // Preserve the complete old issuer/ancestry shape checks. The shared
        // money-only splitter below does not infer charter-title authority.
        if self.title_revision == 0
            || (!self.requires_parents && !self.parents.is_empty())
            || (self.requires_parents && self.parents.is_empty())
        {
            return Err(refused());
        }
        GrossMoneySplit {
            // The money split names the acting character: an organisation
            // buying from its own acting character's charter is refused as
            // a self-sale, as a character's is.
            buyer: self.buyer.acting_character(),
            seller: self.seller,
            charter: self.charter,
            payer: self.payer,
            price: self.price,
            registry: self.registry,
            venue: self.venue,
            parents: self.parents,
            remainder: self.remainder,
        }
        .payouts()
    }
}

/// Pure gross-rate calculation, also used when a paid holder sells an existing
/// instrument. It carries no current issuer/title/source authority or grant.
pub(crate) struct GrossMoneySplit<'a> {
    pub buyer: ObjId,
    pub seller: ObjId,
    pub charter: ObjId,
    pub payer: AccountId,
    pub price: Amount,
    pub registry: &'a GrossRecipientCut,
    pub venue: Option<&'a GrossRecipientCut>,
    pub parents: &'a [RoyaltyRecipientCut],
    pub remainder: &'a Recipient,
}
impl GrossMoneySplit<'_> {
    pub(crate) fn payouts(&self) -> Result<Vec<PurchasePayout>, ParticipantError> {
        if self.buyer.0 == 0
            || self.seller.0 == 0
            || self.buyer == self.seller
            || self.charter.0 == 0
            || self.price.is_negative()
            || self.price.is_zero()
            || self.remainder.owner != PrincipalId::Object(self.seller)
            || self.remainder.account == self.payer
            || self.parents.len() > MAX_PARENT_CUTS
        {
            return Err(refused());
        }
        // The identity of a consented role stays exact even when its frozen
        // gross-rate floor is zero. Zero journal lines are omitted below, not
        // used to excuse crossed or nonexistent recipient accounts.
        let mut owners = BTreeMap::new();
        for recipient in self.recipients() {
            if owners
                .insert(recipient.account, &recipient.owner)
                .is_some_and(|owner| owner != &recipient.owner)
            {
                return Err(refused());
            }
        }
        let mut amounts = BTreeMap::<AccountId, (PrincipalId, Amount)>::new();
        let mut assigned = Amount::ZERO;
        let mut total_rate = 0_u32;
        for cut in std::iter::once(self.registry).chain(self.venue) {
            if (cut.permille == 0) != cut.recipient.is_none()
                || cut
                    .recipient
                    .as_ref()
                    .is_some_and(|r| r.account == self.payer)
            {
                return Err(refused());
            }
            add_cut(
                self.price,
                cut.permille,
                cut.recipient.as_ref(),
                &mut total_rate,
                &mut assigned,
                &mut amounts,
            )?;
        }
        let mut parent_rate = 0_u32;
        let mut charters = BTreeSet::from([self.charter]);
        for cut in self.parents {
            if cut.charter.0 == 0
                || !charters.insert(cut.charter)
                || (cut.permille == 0) != cut.title.is_none()
                || cut.title.as_ref().is_some_and(|t| {
                    t.title_revision == 0
                        || !matches!(t.recipient.owner, PrincipalId::Object(id) if id.0 != 0)
                })
            {
                return Err(refused());
            }
            parent_rate = parent_rate
                .checked_add(u32::from(cut.permille))
                .ok_or_else(refused)?;
            add_cut(
                self.price,
                cut.permille,
                cut.title.as_ref().map(|t| &t.recipient),
                &mut total_rate,
                &mut assigned,
                &mut amounts,
            )?;
        }
        if parent_rate > u32::from(MAX_PARENT_PERMILLE) || total_rate > PERMILLE {
            return Err(refused());
        }
        merge(
            &mut amounts,
            self.remainder,
            self.price.checked_sub(assigned).ok_or_else(refused)?,
        )?;
        if amounts.is_empty() || amounts.len() > MAX_PAYOUTS {
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
    fn recipients(&self) -> Vec<&Recipient> {
        let mut roles = Vec::with_capacity(MAX_PAYOUTS);
        roles.extend(self.registry.recipient.iter());
        roles.extend(self.venue.into_iter().flat_map(|cut| cut.recipient.iter()));
        roles.extend(
            self.parents
                .iter()
                .filter_map(|cut| cut.title.as_ref().map(|t| &t.recipient)),
        );
        roles.push(self.remainder);
        roles
    }
}
