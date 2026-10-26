//! Build a new consent surface exclusively from current owner observations and
//! immutable acquired renewal terms. A later retry captures fresh recipients;
//! the old offer and old paid term remain immutable.
use super::RenewalConfig;
use mush_core::{ClockStamp, ObjId, World};
use mush_economy::managed_entitlement::{
    GrossRecipientCut, Recipient, RoyaltyRecipientCut, RoyaltyTitleRecipient,
};
use mush_economy::managed_renewal::{
    RenewalBasisV1, RenewalLineageV1, RenewalOffer, RenewalOfferTermsV1,
};
use mush_economy::{
    Account, AccountStatus, AssetLifecycle, EconomySnapshot, EconomyStore, InstrumentId,
    InstrumentStatus,
};
use mush_roll::commerce::renewal::reader::RenewalFacts;
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
pub(super) fn build(
    config: &RenewalConfig,
    world: &World,
    facts: &RenewalFacts,
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
    let payer = personal(e.snapshot(), instrument, binding.holder())?.id;
    let seller = facts.sale().seller();
    let remainder = Recipient {
        account: personal(e.snapshot(), instrument, seller)?.id,
        owner: mush_org::PrincipalId::Object(seller),
    };
    cut(e, instrument, &config.policy.registry)?;
    if let Some(venue) = &config.policy.venue {
        cut(e, instrument, venue)?;
    }
    let mut parents = Vec::with_capacity(3);
    for edge in facts.sale().royalty().edges() {
        if parents.len() == 3 {
            return Err(super::records::invalid());
        }
        let charter = ObjId(edge.charter().get());
        let permille = edge.royalty_permille();
        let title = if permille == 0 {
            None
        } else {
            let title = e
                .snapshot()
                .asset(charter)
                .ok_or_else(super::records::invalid)?;
            let mush_org::PrincipalId::Object(owner) = &title.title_owner else {
                return Err(super::records::invalid());
            };
            if title.scope.is_some()
                || title.frozen
                || title.stolen.is_some()
                || title.lifecycle != AssetLifecycle::Active
                || world
                    .get(charter)
                    .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING))
                || world.get(*owner).is_none_or(|o| {
                    o.kind != mush_core::ObjKind::Player
                        || o.flags.contains(mush_core::Flags::GOING)
                })
            {
                return Err(super::records::invalid());
            }
            Some(RoyaltyTitleRecipient {
                title_revision: title.revision,
                recipient: Recipient {
                    account: personal(e.snapshot(), instrument, *owner)?.id,
                    owner: mush_org::PrincipalId::Object(*owner),
                },
            })
        };
        parents.push(RoyaltyRecipientCut {
            charter,
            permille,
            title,
        });
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
            royalty_chain_digest: binding.grant().royalty_chain().digest(),
        },
        terms: RenewalOfferTermsV1 {
            format: 1,
            lineage: if facts.sale().parent().is_some() {
                RenewalLineageV1::Derived
            } else {
                RenewalLineageV1::Original
            },
            buyer: binding.holder(),
            seller,
            charter: ObjId(binding.charter().get()),
            title_revision: facts.sale().title_revision(),
            payer,
            instrument: instrument.clone(),
            price: template.price(),
            registry: config.policy.registry.clone(),
            venue: config.policy.venue.clone(),
            parents,
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
