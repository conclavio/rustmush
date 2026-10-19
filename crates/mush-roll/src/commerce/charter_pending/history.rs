//! Retained ownership proof from the exact native pending settlement. This
//! route never checks today's title, clocks, recipient liveness or parent rights.
use super::records::*;
use crate::commerce::{
    charter_transfer::records::{CharterOwnershipRef, CharterTransfer},
    families::CommerceFamilies,
    read::CommerceRead,
    records::{InitialFilingOperation, PublicationError, Record},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::{ActionTxn, ObjId, World};

fn bad<T>(_: T) -> PublicationError {
    PublicationError::Mismatch
}

pub(crate) fn offer_root(offer: &PendingOffer) -> Result<RequiredRoot, PublicationError> {
    Ok(RequiredRoot {
        anchor: RootAnchor::CharterPendingOffer(offer.operation, offer.digest()?),
        identity: offer.revision,
    })
}
pub(crate) fn funding_root(
    row: &PendingRecord,
    offer: &PendingOffer,
) -> Result<RequiredRoot, PublicationError> {
    row.matches_offer(offer)?;
    Ok(RequiredRoot {
        anchor: RootAnchor::CharterPendingFunding(row.operation, row.funding_digest()?),
        identity: offer.revision,
    })
}
pub(crate) fn terminal_root(
    row: &PendingRecord,
    offer: &PendingOffer,
) -> Result<RequiredRoot, PublicationError> {
    row.matches_offer(offer)?;
    if matches!(row.state, PendingState::Funded {}) {
        return Err(PublicationError::Mismatch);
    }
    Ok(RequiredRoot {
        anchor: RootAnchor::CharterPendingTerminal(row.operation, row.reference()?.digest),
        identity: offer.revision,
    })
}
pub(crate) fn transfer_root(
    row: &CharterTransfer,
    offer: &PendingOffer,
    pending: &PendingRecord,
) -> Result<RequiredRoot, PublicationError> {
    row.matches_pending(offer, pending)?;
    Ok(RequiredRoot {
        anchor: RootAnchor::CharterTransfer(
            row.charter,
            row.generation,
            row.operation,
            row.ownership()?.digest,
        ),
        identity: offer.revision,
    })
}

fn proved(
    world: &World,
    birth: &InitialFilingOperation,
    reference: &CharterOwnershipRef,
    row: &CharterTransfer,
    offer: &PendingOffer,
    pending: &PendingRecord,
) -> Result<ObjId, PublicationError> {
    reference.transferred()?;
    birth.validate()?;
    offer.matches_birth(birth)?;
    row.matches_pending(offer, pending)?;
    if row.ownership()? != *reference || row.birth != birth.commit || row.charter != birth.charter {
        return Err(PublicationError::Mismatch);
    }
    if world
        .module_state_owner(std::any::TypeId::of::<mush_economy::EconomyStore>())
        .is_none_or(|owner| owner.as_str() != "economy")
    {
        return Err(PublicationError::Unavailable);
    }
    let economy = world
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    economy
        .filing_receipt_audit()
        .verify(&birth.audit_subject()?)
        .map_err(bad)?;
    economy
        .charter_transfer_receipt_audit()
        .require_complete()
        .map_err(bad)?;
    row.economic.verify(economy)?;
    let roll = world
        .module_state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    for root in [
        offer_root(offer)?,
        funding_root(pending, offer)?,
        terminal_root(pending, offer)?,
        transfer_root(row, offer, pending)?,
    ] {
        roll.check_required_root(&root).map_err(bad)?;
    }
    Ok(row.economic.facts()?.buyer)
}

pub(crate) fn historical(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    birth: &InitialFilingOperation,
    reference: &CharterOwnershipRef,
    row: &CharterTransfer,
) -> Result<ObjId, PublicationError> {
    let super::super::charter_transfer::economic::TransferEconomic::Pending(native) = &row.economic
    else {
        return Err(PublicationError::Mismatch);
    };
    let pending = ctx
        .get(
            &f.charter_pending,
            &native.pending.consent().operation.to_hex(),
        )
        .map_err(bad)?
        .ok_or(PublicationError::Missing)?;
    let offer = ctx
        .get(&f.charter_pending_offers, &row.offer.to_hex())
        .map_err(bad)?
        .ok_or(PublicationError::Missing)?;
    proved(ctx.world(), birth, reference, row, offer, pending)
}
pub(crate) fn historical_in_txn(
    txn: &mut ActionTxn<'_>,
    world: &World,
    f: &CommerceFamilies,
    birth: &InitialFilingOperation,
    reference: &CharterOwnershipRef,
    row: &CharterTransfer,
) -> Result<ObjId, PublicationError> {
    let super::super::charter_transfer::economic::TransferEconomic::Pending(native) = &row.economic
    else {
        return Err(PublicationError::Mismatch);
    };
    let pending = txn
        .get(
            &f.charter_pending,
            &native.pending.consent().operation.to_hex(),
        )
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    let offer = txn
        .get(&f.charter_pending_offers, &row.offer.to_hex())
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    proved(world, birth, reference, row, &offer, &pending)
}
