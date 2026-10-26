//! Current personal payer and exact scoped organization beneficiary selection.
use super::codec::Bounded;
use super::records::Payout;
use mush_core::{ObjId, World};
use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
use mush_economy::{Account, AccountStatus, EconomyStore, InstrumentId, InstrumentStatus};
use mush_roll::commerce::house_catalogue::HouseIssuer;
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
        .is_none_or(|i| i.status != InstrumentStatus::Active || !i.transferable)
    {
        return Err("instrument unavailable".into());
    }
    let principal = mush_org::PrincipalId::Object(actor);
    let mut candidates = e.snapshot().accounts_of(&principal).filter(|a| {
        a.instrument == *instrument && a.scope.is_none() && a.status == AccountStatus::Active
    });
    let account = candidates.next().ok_or("personal account unavailable")?;
    if candidates.next().is_some() {
        return Err("ambiguous personal account".into());
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
pub(super) fn treasury<'a>(
    e: &'a EconomyStore,
    instrument: &InstrumentId,
    issuer: HouseIssuer,
) -> Result<&'a Account, String> {
    if e.snapshot()
        .instrument(instrument)
        .is_none_or(|i| i.status != InstrumentStatus::Active || !i.transferable)
    {
        return Err("instrument unavailable".into());
    }
    let principal = issuer.principal();
    let mut accounts = e.snapshot().accounts_of(&principal).filter(|a| {
        a.instrument == *instrument
            && a.owner == principal
            && a.scope == Some(issuer.scope())
            && a.status == AccountStatus::Active
    });
    let account = accounts.next().ok_or("house treasury unavailable")?;
    if accounts.next().is_some() {
        return Err("ambiguous house treasury".into());
    }
    Ok(account)
}
pub(super) fn fill_payouts(q: &mut super::HouseQuoteV1) -> Result<(), String> {
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
