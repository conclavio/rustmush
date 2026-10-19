//! Raw native joins precede legacy repair. Missing history is never empty.
use super::{book::PaidResaleBook, refused};
use crate::contract::EconomicContract;
use crate::record::Hold;
use crate::{ContractId, EconomySnapshot, HoldId};
use mush_core::participant::ParticipantError;
use std::collections::BTreeMap;

pub(crate) fn recover(
    book: &mut PaidResaleBook,
    journal_holds: &BTreeMap<HoldId, Hold>,
    snapshot: &mut EconomySnapshot,
    contracts: &mut BTreeMap<ContractId, EconomicContract>,
    holds: &BTreeMap<HoldId, Hold>,
    enabled: bool,
) -> Result<(), ParticipantError> {
    book.finish_replay(contracts, holds, journal_holds)?;
    if !enabled {
        book.require_absent()?;
    }
    // Only bounded protocol-owned contracts move into the native snapshot;
    // unrelated records remain for their own closed or legacy recovery path.
    for row in book.histories()? {
        snapshot.put_contract(contracts.remove(&row.contract_id()).ok_or_else(refused)?);
    }
    Ok(())
}
