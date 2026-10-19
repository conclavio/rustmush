//! Actual account/title projection. Wallet iteration order never chooses a payee.
use mush_core::{Flags, ObjId, ObjKind, World};
use mush_economy::managed_entitlement::{
    GrossRecipientCut, Recipient, RoyaltyRecipientCut, RoyaltyTitleRecipient,
};
use mush_economy::{
    Account, AccountId, AccountStatus, AssetLifecycle, EconomyStore, InstrumentId, InstrumentStatus,
};
use mush_org::PrincipalId;
use mush_roll::commerce::paid_resale::ResaleParentRate;

pub(super) fn live(world: &World) -> Result<&EconomyStore, String> {
    let e = world
        .module_state::<EconomyStore>()
        .ok_or("economy unavailable")?;
    if e.is_frozen()
        || world
            .module_state_owner(std::any::TypeId::of::<EconomyStore>())
            .is_none_or(|o| o.as_str() != "economy")
    {
        return Err("economy unavailable".into());
    }
    Ok(e)
}
fn player(world: &World, actor: ObjId) -> Result<(), String> {
    if world
        .get(actor)
        .is_none_or(|p| p.kind != ObjKind::Player || p.flags.contains(Flags::GOING))
    {
        return Err("personal recipient unavailable".into());
    }
    Ok(())
}
pub(super) fn personal<'a>(
    world: &World,
    e: &'a EconomyStore,
    instrument: &InstrumentId,
    actor: ObjId,
    id: AccountId,
) -> Result<&'a Account, String> {
    player(world, actor)?;
    if e.snapshot()
        .instrument(instrument)
        .is_none_or(|i| i.status != InstrumentStatus::Active)
    {
        return Err("instrument unavailable".into());
    }
    let account = e.snapshot().account(id).ok_or("personal account missing")?;
    if account.owner != PrincipalId::Object(actor)
        || account.scope.is_some()
        || account.instrument != *instrument
        || account.status != AccountStatus::Active
    {
        return Err("personal account differs".into());
    }
    Ok(account)
}
fn unique<'a>(
    world: &World,
    e: &'a EconomyStore,
    instrument: &InstrumentId,
    actor: ObjId,
) -> Result<&'a Account, String> {
    let owner = PrincipalId::Object(actor);
    let mut rows = e.snapshot().accounts_of(&owner).filter(|a| {
        a.instrument == *instrument && a.scope.is_none() && a.status == AccountStatus::Active
    });
    let one = rows.next().ok_or("personal royalty account unavailable")?;
    if rows.next().is_some() {
        return Err("personal royalty account is ambiguous".into());
    }
    personal(world, e, instrument, actor, one.id)
}
pub(super) fn cuts(
    e: &EconomyStore,
    instrument: &InstrumentId,
    registry: &GrossRecipientCut,
    venue: Option<&GrossRecipientCut>,
) -> Result<(), String> {
    for cut in std::iter::once(registry).chain(venue) {
        if cut.permille > 1000 || (cut.permille == 0) != cut.recipient.is_none() {
            return Err("accepted cut differs".into());
        }
        if let Some(r) = &cut.recipient {
            if e.snapshot().account(r.account).is_none_or(|a| {
                a.owner != r.owner
                    || a.instrument != *instrument
                    || a.status != AccountStatus::Active
            }) {
                return Err("accepted recipient unavailable".into());
            }
        }
    }
    Ok(())
}
pub(super) fn parents(
    world: &World,
    e: &EconomyStore,
    instrument: &InstrumentId,
    rates: &[ResaleParentRate],
) -> Result<Vec<RoyaltyRecipientCut>, String> {
    if rates.len() > 3 {
        return Err("parent cut bound".into());
    }
    let mut out = Vec::with_capacity(rates.len());
    for rate in rates {
        let charter = rate.charter();
        let title = if rate.permille() == 0 {
            None
        } else {
            if world
                .get(charter)
                .is_none_or(|o| o.flags.contains(Flags::GOING))
            {
                return Err("royalty charter unavailable".into());
            }
            let asset = e.snapshot().asset(charter).ok_or("royalty title missing")?;
            let PrincipalId::Object(actor) = &asset.title_owner else {
                return Err("personal royalty title required".into());
            };
            if asset.scope.is_some()
                || asset.frozen
                || asset.frozen_reason.is_some()
                || asset.stolen.is_some()
                || !asset.encumbrances.is_empty()
                || asset.lifecycle != AssetLifecycle::Active
            {
                return Err("royalty title unavailable".into());
            }
            let account = unique(world, e, instrument, *actor)?;
            Some(RoyaltyTitleRecipient {
                title_revision: asset.revision,
                recipient: Recipient {
                    account: account.id,
                    owner: PrincipalId::Object(*actor),
                },
            })
        };
        out.push(RoyaltyRecipientCut {
            charter,
            permille: rate.permille(),
            title,
        });
    }
    Ok(out)
}
