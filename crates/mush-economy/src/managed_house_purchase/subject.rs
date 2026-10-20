//! Exact organization and scope are the native issuing principal. There is no
//! curator/seller placeholder and no caller-controlled permission projection.
use super::{MAX_PAYOUTS, refused};
use crate::managed_entitlement::{GrossRecipientCut, Recipient};
use crate::managed_purchase::{FundingContext, PurchasePayout};
use crate::record::{AccountStatus, AssetLifecycle, InstrumentStatus};
use crate::{AccountId, Amount, EconomyStore, InstrumentId};
use mush_core::participant::ParticipantError;
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, Flags, ObjId, ObjKind};
use mush_org::{OrgId, OrgScopeId, OrgStatus, OrgStore, PrincipalId, ScopeStatus};
use serde::{Deserialize, Serialize};
use std::{any::TypeId, collections::BTreeMap};

/// Source-free terms. Only the exact configured native owner participant may
/// consume this input; it is not a public command or an authority constructor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HousePurchase {
    pub quote: CommitToken,
    pub quote_digest: DefinitionDigest,
    pub product_digest: DefinitionDigest,
    pub buyer: ObjId,
    pub organization: OrgId,
    pub scope: OrgScopeId,
    pub charter: ObjId,
    pub title_revision: u64,
    pub payer: AccountId,
    pub instrument: InstrumentId,
    pub price: Amount,
    pub registry: GrossRecipientCut,
    pub venue: Option<GrossRecipientCut>,
    pub remainder: Recipient,
    pub grant_economics_digest: DefinitionDigest,
    pub quoted_at_utc: i64,
    pub expires_at_utc: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HouseWire {
    quote: CommitToken,
    quote_digest: DefinitionDigest,
    product_digest: DefinitionDigest,
    buyer: ObjId,
    organization: OrgId,
    scope: OrgScopeId,
    charter: ObjId,
    title_revision: u64,
    payer: AccountId,
    instrument: InstrumentId,
    price: Amount,
    registry: CutWire,
    #[serde(deserialize_with = "required")]
    venue: Option<CutWire>,
    remainder: Recipient,
    grant_economics_digest: DefinitionDigest,
    quoted_at_utc: i64,
    expires_at_utc: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CutWire {
    permille: u16,
    #[serde(deserialize_with = "required")]
    recipient: Option<Recipient>,
}
fn required<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    T::deserialize(d)
}
impl From<&GrossRecipientCut> for CutWire {
    fn from(c: &GrossRecipientCut) -> Self {
        Self {
            permille: c.permille,
            recipient: c.recipient.clone(),
        }
    }
}
impl CutWire {
    fn data(&self) -> GrossRecipientCut {
        GrossRecipientCut {
            permille: self.permille,
            recipient: self.recipient.clone(),
        }
    }
}
impl From<&HousePurchase> for HouseWire {
    fn from(p: &HousePurchase) -> Self {
        Self {
            quote: p.quote,
            quote_digest: p.quote_digest,
            product_digest: p.product_digest,
            buyer: p.buyer,
            organization: p.organization,
            scope: p.scope,
            charter: p.charter,
            title_revision: p.title_revision,
            payer: p.payer,
            instrument: p.instrument.clone(),
            price: p.price,
            registry: (&p.registry).into(),
            venue: p.venue.as_ref().map(Into::into),
            remainder: p.remainder.clone(),
            grant_economics_digest: p.grant_economics_digest,
            quoted_at_utc: p.quoted_at_utc,
            expires_at_utc: p.expires_at_utc,
        }
    }
}
impl HouseWire {
    pub(crate) fn data(&self) -> Result<HousePurchase, ParticipantError> {
        let p = HousePurchase {
            quote: self.quote,
            quote_digest: self.quote_digest,
            product_digest: self.product_digest,
            buyer: self.buyer,
            organization: self.organization,
            scope: self.scope,
            charter: self.charter,
            title_revision: self.title_revision,
            payer: self.payer,
            instrument: self.instrument.clone(),
            price: self.price,
            registry: self.registry.data(),
            venue: self.venue.as_ref().map(CutWire::data),
            remainder: self.remainder.clone(),
            grant_economics_digest: self.grant_economics_digest,
            quoted_at_utc: self.quoted_at_utc,
            expires_at_utc: self.expires_at_utc,
        };
        p.payouts()?;
        Ok(p)
    }
}
pub(crate) fn matches_action(action: &DefinitionRef, transition: &str) -> bool {
    let id = match transition {
        "fund" => "purchase_house_mind",
        "grant" => "settle_house_mind",
        "refund" => "refund_house_mind",
        _ => return false,
    };
    action.key().owner.as_str() == "market"
        && action.key().kind.as_str() == "action"
        && action.key().id.as_str() == id
        && matches!(action.version(), 1 | 2)
}
impl HousePurchase {
    pub fn economic_digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        self.payouts()?;
        DefinitionDigest::for_canonical_value(
            "economy.house_mind_purchase.v1",
            &serde_json::to_value(HouseWire::from(self)).map_err(|_| refused())?,
        )
        .map_err(|_| refused())
    }
    pub fn payouts(&self) -> Result<Vec<PurchasePayout>, ParticipantError> {
        if self.buyer.0 == 0
            || self.charter.0 == 0
            || self.title_revision == 0
            || self.price.is_negative()
            || self.price.is_zero()
            || self.remainder.owner != PrincipalId::Organization(self.organization)
            || self
                .expires_at_utc
                .checked_sub(self.quoted_at_utc)
                .is_none_or(|n| !(1..=600).contains(&n))
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
    pub(crate) fn recipients(&self) -> Vec<&Recipient> {
        let mut out = Vec::with_capacity(MAX_PAYOUTS);
        out.extend(self.registry.recipient.iter());
        out.extend(self.venue.iter().flat_map(|c| c.recipient.iter()));
        out.push(&self.remainder);
        out
    }
    pub(crate) fn check_current(
        &self,
        state: &EconomyStore,
        ctx: &FundingContext<'_>,
    ) -> Result<(), ParticipantError> {
        self.payouts()?;
        let world = ctx.world;
        if state.frozen
            || !matches_action(ctx.action, "fund")
            || ctx.actor != self.buyer
            || ctx.principal != &(mush_core::PrincipalId::Object { id: self.buyer })
            || self.quote == ctx.commit
            || world.now() < self.quoted_at_utc
            || world.now() >= self.expires_at_utc
            || world
                .get(self.buyer)
                .is_none_or(|p| p.kind != ObjKind::Player || p.flags.contains(Flags::GOING))
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
        if payer.owner != PrincipalId::Object(self.buyer)
            || payer.scope.is_some()
            || payer.instrument != self.instrument
            || payer.status != AccountStatus::Active
            || payer.approval.is_some()
            || snapshot
                .instrument(&self.instrument)
                .is_none_or(|i| i.status != InstrumentStatus::Active || !i.transferable)
        {
            return Err(refused());
        }
        for r in self.recipients() {
            let account = snapshot.account(r.account).ok_or_else(refused)?;
            if account.owner != r.owner
                || account.instrument != self.instrument
                || account.status != AccountStatus::Active
            {
                return Err(refused());
            }
        }
        let treasury = snapshot
            .account(self.remainder.account)
            .ok_or_else(refused)?;
        if treasury.scope != Some(self.scope) {
            return Err(refused());
        }
        Ok(())
    }
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
