//! Captured ownership proof and current title are deliberately separate reads.
use super::records::*;
use crate::commerce::{
    families::CommerceFamilies,
    read::CommerceRead,
    records::{InitialFilingOperation, PublicationError, Record},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::{ActionTxn, ObjId, World};

fn bad<T>(_: T) -> PublicationError {
    PublicationError::Mismatch
}
fn economy(world: &World) -> Result<&mush_economy::EconomyStore, PublicationError> {
    if world
        .module_state_owner(std::any::TypeId::of::<mush_economy::EconomyStore>())
        .is_none_or(|owner| owner.as_str() != "economy")
    {
        return Err(PublicationError::Unavailable);
    }
    let e = world
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    e.charter_transfer_receipt_audit()
        .require_complete()
        .map_err(bad)?;
    Ok(e)
}
pub(crate) fn offer_root(offer: &CharterOffer) -> Result<RequiredRoot, PublicationError> {
    Ok(RequiredRoot {
        anchor: RootAnchor::CharterTransferOffer(offer.operation, offer.digest()?),
        identity: offer.revision,
    })
}
pub(crate) fn transfer_root(
    row: &CharterTransfer,
    offer: &CharterOffer,
) -> Result<RequiredRoot, PublicationError> {
    row.matches_offer(offer)?;
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
    row: Option<&CharterTransfer>,
    offer: Option<&CharterOffer>,
) -> Result<ObjId, PublicationError> {
    reference.validate()?;
    birth.validate()?;
    let e = economy(world)?;
    e.filing_receipt_audit()
        .verify(&birth.audit_subject()?)
        .map_err(bad)?;
    if reference.generation == 1 {
        if reference != &CharterOwnershipRef::birth(birth)? || row.is_some() || offer.is_some() {
            return Err(PublicationError::Mismatch);
        }
        return Ok(birth.requester);
    }
    let row = row.ok_or(PublicationError::Missing)?;
    let offer = offer.ok_or(PublicationError::Missing)?;
    offer.matches_birth(birth)?;
    row.matches_offer(offer)?;
    if &row.ownership()? != reference || row.birth != birth.commit || row.charter != birth.charter {
        return Err(PublicationError::Mismatch);
    }
    row.economic.verify(e)?;
    let roll = world
        .module_state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    roll.check_required_root(&offer_root(offer)?).map_err(bad)?;
    roll.check_required_root(&transfer_root(row, offer)?)
        .map_err(bad)?;
    Ok(row.economic.facts()?.buyer)
}
/// Historical proof does not consult a current title, term or person object.
/// Complete chain reconstruction is mandatory before Roll enables consumers.
pub(crate) fn historical(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    birth: &InitialFilingOperation,
    reference: &CharterOwnershipRef,
) -> Result<ObjId, PublicationError> {
    if reference.generation == 1 {
        return proved(ctx.world(), birth, reference, None, None);
    }
    let row = ctx
        .get(
            &f.charter_transfers,
            &CharterTransfer::key_for(birth.charter, reference.generation),
        )
        .map_err(bad)?
        .ok_or(PublicationError::Missing)?;
    if row.format == 2 {
        return crate::commerce::charter_pending::history::historical(
            ctx, f, birth, reference, row,
        );
    }
    let offer = ctx
        .get(&f.charter_transfer_offers, &row.offer.to_hex())
        .map_err(bad)?
        .ok_or(PublicationError::Missing)?;
    proved(ctx.world(), birth, reference, Some(row), Some(offer))
}
pub(crate) fn historical_in_txn(
    txn: &mut ActionTxn<'_>,
    world: &World,
    f: &CommerceFamilies,
    birth: &InitialFilingOperation,
    reference: &CharterOwnershipRef,
) -> Result<ObjId, PublicationError> {
    if reference.generation == 1 {
        return proved(world, birth, reference, None, None);
    }
    let row = txn
        .get(
            &f.charter_transfers,
            &CharterTransfer::key_for(birth.charter, reference.generation),
        )
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    if row.format == 2 {
        return crate::commerce::charter_pending::history::historical_in_txn(
            txn, world, f, birth, reference, &row,
        );
    }
    let offer = txn
        .get(&f.charter_transfer_offers, &row.offer.to_hex())
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    proved(world, birth, reference, Some(&row), Some(&offer))
}
pub(crate) fn current(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    birth: &InitialFilingOperation,
    owner: ObjId,
) -> Result<CharterOwnershipRef, PublicationError> {
    let reference = match ctx
        .get(&f.charter_ownership, &birth.charter.key())
        .map_err(bad)?
    {
        Some(head) => {
            head.validate()?;
            if head.charter != birth.charter {
                return Err(PublicationError::Mismatch);
            }
            head.current.clone()
        }
        None => CharterOwnershipRef::birth(birth)?,
    };
    if historical(ctx, f, birth, &reference)? != owner {
        return Err(PublicationError::Mismatch);
    }
    crate::commerce::owner_qualification::current_title(ctx.world(), owner, birth.charter)?;
    Ok(reference)
}
pub(crate) fn current_in_txn(
    txn: &mut ActionTxn<'_>,
    world: &World,
    f: &CommerceFamilies,
    birth: &InitialFilingOperation,
    owner: ObjId,
) -> Result<CharterOwnershipRef, PublicationError> {
    let reference = match txn
        .get(&f.charter_ownership, &birth.charter.key())
        .map_err(bad)?
    {
        Some(head) => {
            head.validate()?;
            if head.charter != birth.charter {
                return Err(PublicationError::Mismatch);
            }
            head.current.clone()
        }
        None => CharterOwnershipRef::birth(birth)?,
    };
    if historical_in_txn(txn, world, f, birth, &reference)? != owner {
        return Err(PublicationError::Mismatch);
    }
    crate::commerce::owner_qualification::current_title(world, owner, birth.charter)?;
    Ok(reference)
}

/// M8c B2/B4: a House charter's current ownership for an organisation holder:
/// the birth is the organisation's House filing, no ownership head has moved
/// it (none can), and the organisation's title is current for the acting
/// character (`current_title_for`).
fn house_current(
    world: &World,
    head: Option<&CharterOwnershipHead>,
    birth: &InitialFilingOperation,
    holder: crate::commerce::Holder,
) -> Result<CharterOwnershipRef, PublicationError> {
    let crate::commerce::Holder::Organization { org, .. } = holder else {
        return Err(PublicationError::Mismatch);
    };
    let reference = CharterOwnershipRef::house_birth(birth)?;
    if let Some(head) = head {
        head.validate()?;
        if head.charter != birth.charter || head.current != reference {
            return Err(PublicationError::Mismatch);
        }
    }
    if birth
        .economic
        .house
        .as_ref()
        .is_none_or(|h| h.organization() != org)
    {
        return Err(PublicationError::Mismatch);
    }
    economy(world)?
        .filing_receipt_audit()
        .verify(&birth.audit_subject()?)
        .map_err(bad)?;
    crate::commerce::owner_qualification::current_title_for(world, holder, birth.charter)?;
    Ok(reference)
}
/// `current` for a typed holder (M8c): a character holder as `current`, an
/// organisation holder through `house_current`.
pub(crate) fn current_for(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    birth: &InitialFilingOperation,
    holder: crate::commerce::Holder,
) -> Result<CharterOwnershipRef, PublicationError> {
    match holder {
        crate::commerce::Holder::Character(owner) => current(ctx, f, birth, owner),
        crate::commerce::Holder::Organization { .. } => {
            let head = ctx
                .get(&f.charter_ownership, &birth.charter.key())
                .map_err(bad)?;
            house_current(ctx.world(), head, birth, holder)
        }
    }
}
/// `current_in_txn` for a typed holder (M8c).
pub(crate) fn current_in_txn_for(
    txn: &mut ActionTxn<'_>,
    world: &World,
    f: &CommerceFamilies,
    birth: &InitialFilingOperation,
    holder: crate::commerce::Holder,
) -> Result<CharterOwnershipRef, PublicationError> {
    match holder {
        crate::commerce::Holder::Character(owner) => current_in_txn(txn, world, f, birth, owner),
        crate::commerce::Holder::Organization { .. } => {
            let head = txn
                .get(&f.charter_ownership, &birth.charter.key())
                .map_err(bad)?
                .cloned();
            house_current(world, head.as_ref(), birth, holder)
        }
    }
}

/// Absent capture is the immutable original-owner history, not an ever-owned
/// fallback. New transferred-owner records carry a complete native-backed ref.
pub(crate) fn require_captured(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    birth: &InitialFilingOperation,
    reference: Option<&CharterOwnershipRef>,
    actor: ObjId,
) -> Result<(), PublicationError> {
    let accepted = match reference {
        None => birth.requester,
        Some(reference) => historical(ctx, f, birth, reference)?,
    };
    if accepted != actor {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}
pub(crate) fn require_captured_in_txn(
    txn: &mut ActionTxn<'_>,
    world: &World,
    f: &CommerceFamilies,
    birth: &InitialFilingOperation,
    reference: Option<&CharterOwnershipRef>,
    actor: ObjId,
) -> Result<(), PublicationError> {
    let accepted = match reference {
        None => birth.requester,
        Some(reference) => historical_in_txn(txn, world, f, birth, reference)?,
    };
    if accepted != actor {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}
