//! Build a new consent surface exclusively from current owner observations and
//! immutable acquired renewal terms. A later retry captures fresh recipients;
//! the old offer and old paid term remain immutable.
use super::RenewalConfig;
use mush_core::{ClockStamp, ObjId, World};
use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
use mush_economy::managed_house_renewal::{RenewalBasisV1, RenewalOffer, RenewalOfferTermsV1};
use mush_economy::{
    Account, AccountStatus, EconomySnapshot, EconomyStore, InstrumentId, InstrumentStatus,
};
use mush_roll::commerce::house_renewal::reader::RenewalFacts;
fn invalid(_: impl std::fmt::Display) -> String {
    super::records::invalid()
}
pub(super) fn personal<'a>(
    snapshot: &'a EconomySnapshot,
    instrument: &InstrumentId,
    actor: ObjId,
) -> Result<&'a Account, String> {
    let owner = mush_org::PrincipalId::Object(actor);
    let mut matching = snapshot.accounts_of(&owner).filter(|a| {
        a.instrument == *instrument
            && a.scope.is_none()
            && a.status == AccountStatus::Active
            && a.approval.is_none()
    });
    let account = matching.next().ok_or_else(super::records::invalid)?;
    // This route has no explicit wallet choice. Ambiguity cannot silently pick
    // an account; a later buyer-choice producer must capture an exact selection.
    if matching.next().is_some() {
        return Err(super::records::invalid());
    }
    Ok(account)
}
fn cut(
    e: &EconomyStore,
    instrument: &InstrumentId,
    value: &GrossRecipientCut,
) -> Result<(), String> {
    if let Some(r) = &value.recipient
        && e.snapshot().account(r.account).is_none_or(|a| {
            a.owner != r.owner || a.instrument != *instrument || a.status != AccountStatus::Active
        })
    {
        return Err(super::records::invalid());
    }
    Ok(())
}
/// M8c B3: the payer for `holder`: a character's one personal account
/// (`personal`), or an organisation's treasury on its root scope
/// (`mind_entitlement::accounts::treasury`).
fn payer<'a>(
    e: &'a EconomyStore,
    world: &World,
    instrument: &InstrumentId,
    holder: mush_economy::Holder,
) -> Result<&'a Account, String> {
    match holder {
        mush_economy::Holder::Character(c) => personal(e.snapshot(), instrument, c),
        mush_economy::Holder::Organization { org, .. } => {
            let scope = mush_roll::commerce::org_holder::root_scope(world, org)
                .ok_or_else(super::records::invalid)?;
            crate::mind_entitlement::accounts::treasury(e, instrument, org, scope).map_err(invalid)
        }
    }
}
/// `actor` is the invocation's actor: M8c B3 re-stamps an organisation
/// holder with it as the acting character (`org_holder::acting_as`), so a
/// character granted `roll.holder.org` runs the renewal steps for the
/// organisation; a character holder must be the actor itself.
pub(super) fn build(
    config: &RenewalConfig,
    world: &World,
    facts: &RenewalFacts,
    actor: ObjId,
) -> Result<RenewalOffer, String> {
    config.policy.validate().map_err(invalid)?;
    let e = world
        .module_state::<EconomyStore>()
        .ok_or_else(super::records::invalid)?;
    if world
        .module_state_owner(std::any::TypeId::of::<EconomyStore>())
        .is_none_or(|o| o.as_str() != "economy")
        || e.is_frozen()
    {
        return Err(super::records::invalid());
    }
    let binding = facts.binding();
    let template = binding.grant().renewal();
    let instrument = template.instrument();
    if e.snapshot()
        .instrument(instrument)
        .is_none_or(|i| i.status != InstrumentStatus::Active || !i.transferable)
    {
        return Err(super::records::invalid());
    }
    let buyer = mush_roll::commerce::org_holder::acting_as(world, binding.holder(), actor)
        .map_err(|_| super::records::invalid())?;
    let payer = payer(e, world, instrument, buyer)?.id;
    if !config.accepts_purchase_template(template) {
        return Err(super::records::invalid());
    }
    let issuer = binding.issuer();
    let principal = issuer.principal();
    let snapshot = e.snapshot();
    let treasury = {
        let mut treasuries = snapshot.accounts_of(&principal).filter(|a| {
            a.instrument == *instrument
                && a.owner == principal
                && a.scope == Some(issuer.scope())
                && a.status == AccountStatus::Active
                && a.approval.is_none()
        });
        let treasury = treasuries.next().ok_or_else(super::records::invalid)?;
        if treasuries.next().is_some() {
            return Err(super::records::invalid());
        }
        treasury.id
    };
    let remainder = Recipient {
        account: treasury,
        owner: principal,
    };
    cut(e, instrument, &config.policy.registry)?;
    if let Some(venue) = &config.policy.venue {
        cut(e, instrument, venue)?;
    }
    let now = world.world_ic_now();
    let expires_at = ClockStamp {
        clock: now.clock.clone(),
        at: now
            .at
            .checked_add(config.policy.offer_ic_seconds)
            .ok_or_else(super::records::invalid)?,
    };
    let offer = RenewalOffer {
        format: 1,
        basis: RenewalBasisV1 {
            format: 1,
            licence: binding.licence().get(),
            predecessor_generation: binding.predecessor().get(),
            next_generation: binding.next().get(),
            predecessor_operation: binding.predecessor_operation(),
            prior_quote: facts.prior_quote(),
            prior_quote_digest: facts.prior_quote_digest(),
            product_digest: binding.product_digest(),
            template_digest: binding.template_digest().map_err(invalid)?,
            grant_economics_digest: binding.grant().digest().map_err(invalid)?,
        },
        terms: RenewalOfferTermsV1 {
            format: 1,
            buyer,
            organization: issuer.organization(),
            scope: issuer.scope(),
            charter: ObjId(binding.charter().get()),
            title_revision: facts.sale().title_revision(),
            payer,
            instrument: instrument.clone(),
            price: template.price(),
            registry: config.policy.registry.clone(),
            venue: config.policy.venue.clone(),
            remainder,
            not_before: now,
            expires_at,
        },
    };
    offer.validate().map_err(invalid)?;
    // Public offer ingress has the same explicit size ceiling as E4's records.
    let bytes = serde_json::to_vec(&offer).map_err(invalid)?;
    RenewalOffer::decode(&bytes).map_err(invalid)
}
