//! Actual quote-time recipient projection. E4 rechecks all account/title inputs
//! under its owner lock at funding and grant, including positive cuts rounded 0.
use super::codec::Bounded;
use super::records::{ParentCut, Payout};
use mush_core::{ObjId, World};
use mush_economy::managed_entitlement::{
    GrossRecipientCut, Recipient, RoyaltyRecipientCut, RoyaltyTitleRecipient,
};
use mush_economy::{
    Account, AccountStatus, AssetLifecycle, EconomyStore, InstrumentId, InstrumentStatus,
};
use mush_roll::commerce::entitlement::FiledRoyaltyTermsV1;
pub(super) fn live(world: &World) -> Result<&EconomyStore, String> {
    let e = world
        .module_state::<EconomyStore>()
        .ok_or("economy unavailable")?;
    if world
        .module_state_owner(std::any::TypeId::of::<EconomyStore>())
        .is_none_or(|o| o.as_str() != "economy")
        || e.is_frozen()
    {
        return Err("economy unavailable".into());
    }
    Ok(e)
}
pub(super) fn personal<'a>(
    e: &'a EconomyStore,
    instrument: &InstrumentId,
    actor: ObjId,
) -> Result<&'a Account, String> {
    if e.snapshot()
        .instrument(instrument)
        .is_none_or(|i| i.status != InstrumentStatus::Active)
    {
        return Err("instrument unavailable".into());
    }
    e.snapshot()
        .accounts_of(&mush_org::PrincipalId::Object(actor))
        .find(|a| {
            a.instrument == *instrument && a.scope.is_none() && a.status == AccountStatus::Active
        })
        .ok_or_else(|| "personal account unavailable".into())
}
/// M8c B3: the organisation's treasury on `scope`: its one Active account
/// in `instrument` owned by the organisation on that scope (the House's
/// `mind_house::accounts::treasury` rule).
pub(crate) fn treasury<'a>(
    e: &'a EconomyStore,
    instrument: &InstrumentId,
    org: mush_org::OrgId,
    scope: mush_org::OrgScopeId,
) -> Result<&'a Account, String> {
    if e.snapshot()
        .instrument(instrument)
        .is_none_or(|i| i.status != InstrumentStatus::Active || !i.transferable)
    {
        return Err("instrument unavailable".into());
    }
    let principal = mush_org::PrincipalId::Organization(org);
    let mut accounts = e.snapshot().accounts_of(&principal).filter(|a| {
        a.instrument == *instrument
            && a.owner == principal
            && a.scope == Some(scope)
            && a.status == AccountStatus::Active
    });
    let account = accounts.next().ok_or("organisation treasury unavailable")?;
    if accounts.next().is_some() {
        return Err("ambiguous organisation treasury".into());
    }
    Ok(account)
}
pub(super) fn recipient(
    e: &EconomyStore,
    instrument: &InstrumentId,
    r: &Recipient,
) -> Result<(), String> {
    if e.snapshot().account(r.account).is_none_or(|a| {
        a.owner != r.owner || a.instrument != *instrument || a.status != AccountStatus::Active
    }) {
        return Err("recipient account unavailable".into());
    }
    Ok(())
}
pub(super) fn cut(
    e: &EconomyStore,
    instrument: &InstrumentId,
    cut: &GrossRecipientCut,
) -> Result<(), String> {
    if let Some(r) = &cut.recipient {
        recipient(e, instrument, r)?;
    }
    Ok(())
}
pub(super) fn parents(
    world: &World,
    e: &EconomyStore,
    instrument: &InstrumentId,
    chain: &FiledRoyaltyTermsV1,
) -> Result<Bounded<ParentCut, 3>, String> {
    let mut out = Vec::with_capacity(chain.edges().len());
    for edge in chain.edges() {
        let charter = ObjId(edge.charter().get());
        let permille = edge.royalty_permille();
        let title = if permille == 0 {
            None
        } else {
            if world
                .get(charter)
                .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING))
            {
                return Err("royalty charter unavailable".into());
            }
            let title = e.snapshot().asset(charter).ok_or("royalty title missing")?;
            let mush_org::PrincipalId::Object(actor) = &title.title_owner else {
                return Err("personal royalty title owner required".into());
            };
            if title.scope.is_some()
                || title.frozen
                || title.stolen.is_some()
                || title.lifecycle != AssetLifecycle::Active
            {
                return Err("royalty title unavailable".into());
            }
            if world.get(*actor).is_none_or(|o| {
                o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
            }) {
                return Err("royalty recipient unavailable".into());
            }
            let account = personal(e, instrument, *actor)?;
            Some(RoyaltyTitleRecipient {
                title_revision: title.revision,
                recipient: Recipient {
                    account: account.id,
                    owner: mush_org::PrincipalId::Object(*actor),
                },
            })
        };
        out.push(ParentCut(RoyaltyRecipientCut {
            charter,
            permille,
            title,
        }));
    }
    Ok(Bounded(out))
}
pub(super) fn fill_payouts(q: &mut super::EntitlementQuoteV1) -> Result<(), String> {
    q.payouts = Bounded(
        q.economic_data(q.raw_digest()?)?
            .payouts()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|p| Payout {
                account: p.account,
                owner: p.owner,
                amount: p.amount,
            })
            .collect(),
    );
    super::codec::Row::validate(q)
}
