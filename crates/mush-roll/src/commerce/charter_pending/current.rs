//! Source-free current admission and exact pending-head fences. Historical
//! ownership/source/paid use never calls this admission gate.
use super::records::*;
use crate::commerce::{
    CharterId,
    charter_transfer::{history as ownership, records::CharterOwnershipRef},
    families::CommerceFamilies,
    read::CommerceRead,
    records::{FiledRecord, InitialFilingOperation, OwnerTerm, PublicationError, Record},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::{ActionTxn, ObjId, World};
fn bad<T>(_: T) -> PublicationError {
    PublicationError::Mismatch
}

fn pair(world: &World, head: &PendingHead, row: &PendingRecord) -> Result<(), PublicationError> {
    head.matches(row)?;
    let economy = world
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    if economy
        .pending_charter_history(row.operation)
        .map_err(bad)?
        != &row.economic
    {
        return Err(PublicationError::Mismatch);
    }
    if matches!(row.state, PendingState::Funded {}) {
        return Err(PublicationError::Unavailable);
    }
    Ok(())
}
pub(crate) fn require_clear(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    charter: CharterId,
) -> Result<(), PublicationError> {
    if let Some(head) = ctx
        .get(&f.charter_pending_heads, &charter.key())
        .map_err(bad)?
    {
        if head.charter != charter {
            return Err(PublicationError::Mismatch);
        }
        let row = ctx
            .get(&f.charter_pending, &head.current.operation.to_hex())
            .map_err(bad)?
            .ok_or(PublicationError::Missing)?;
        pair(ctx.world(), head, row)?;
    }
    Ok(())
}
pub(crate) fn require_clear_in_txn(
    txn: &mut ActionTxn<'_>,
    world: &World,
    f: &CommerceFamilies,
    charter: CharterId,
) -> Result<(), PublicationError> {
    if let Some(head) = txn
        .get(&f.charter_pending_heads, &charter.key())
        .map_err(bad)?
        .cloned()
    {
        if head.charter != charter {
            return Err(PublicationError::Mismatch);
        }
        let row = txn
            .get(&f.charter_pending, &head.current.operation.to_hex())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        pair(world, &head, &row)?;
    }
    Ok(())
}

pub(crate) struct Basis {
    pub(crate) birth: InitialFilingOperation,
    pub(crate) filed: FiledRecord,
    pub(crate) ownership: CharterOwnershipRef,
    pub(crate) term: OwnerTerm,
}
/// Title consideration does not depend on current source entitlement, recall,
/// conformance or term phase. It does require the actual current immutable term
/// and ownership history, so neither expiry nor a past transfer erases title.
pub(crate) fn basis(
    txn: &mut ActionTxn<'_>,
    world: &World,
    f: &CommerceFamilies,
    charter: CharterId,
    seller: ObjId,
) -> Result<Basis, PublicationError> {
    let roll = world
        .module_state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    roll.commerce_ready().map_err(bad)?;
    let c = txn
        .get(&f.charters, &charter.key())
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    let birth = txn
        .get(&f.operations, &c.operation.to_hex())
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    let line = txn
        .get(&f.lines, &c.line.key())
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    let filed = txn
        .get(&f.filed, &line.head.key())
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    let licence = txn
        .get(&f.licences, &birth.licence.key())
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    c.validate()?;
    birth.validate()?;
    line.validate()?;
    filed.validate()?;
    licence.validate()?;
    if c.id != charter
        || c.operation != birth.commit
        || c.line != birth.line
        || birth.charter != charter
        || line.id != c.line
        || line.charter != charter
        || filed.content.id() != line.head
        || filed.content.position().charter != charter
        || filed.content.position().line != c.line
        || licence.operation != birth.commit
        || licence.id != birth.licence
        || licence.charter != charter
        || licence.line != birth.line
        || birth.economic.house.is_some()
    {
        return Err(PublicationError::Mismatch);
    }
    let ownership = ownership::current_in_txn(txn, world, f, &birth, seller)?;
    if ownership.generation > 1 {
        let previous = txn
            .get(
                &f.charter_transfers,
                &crate::commerce::charter_transfer::records::CharterTransfer::key_for(
                    charter,
                    ownership.generation,
                ),
            )
            .map_err(bad)?
            .ok_or(PublicationError::Missing)?;
        if previous.ownership()? != ownership
            || world.now() < previous.economic.facts()?.posted_at_utc
        {
            return Err(PublicationError::Mismatch);
        }
    }
    let term =
        crate::commerce::owner_attestation::history::current_in_txn(txn, world, f, &licence)?;
    roll.check_required_root(&RequiredRoot {
        anchor: RootAnchor::Filed(filed.content.id()),
        identity: filed.content.revision(),
    })
    .map_err(bad)?;
    Ok(Basis {
        birth,
        filed,
        ownership,
        term,
    })
}

pub(crate) fn matches_offer(basis: &Basis, offer: &PendingOffer) -> Result<(), PublicationError> {
    offer.matches_birth(&basis.birth)?;
    offer.owner_term.matches(&basis.term)?;
    if basis.ownership != offer.ownership
        || basis.filed.content.id() != offer.filed
        || basis.filed.content.revision() != offer.revision
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}
