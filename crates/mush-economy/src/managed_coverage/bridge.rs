//! Complete raw native joins run before legacy repair or document mutation.
use super::{book::CoverageBook, refused};
use crate::contract::EconomicContract;
use crate::record::{EconomicAsset, Encumbrance, Hold};
use crate::{ContractId, EconomySnapshot, HoldId};
use mush_core::{ObjId, participant::ParticipantError};
use std::collections::BTreeMap;

#[allow(clippy::too_many_arguments)]
pub(crate) fn recover(
    book: &mut CoverageBook,
    journal_holds: &BTreeMap<HoldId, Hold>,
    journal_assets: &BTreeMap<ObjId, EconomicAsset>,
    snapshot: &mut EconomySnapshot,
    contracts: &mut BTreeMap<ContractId, EconomicContract>,
    holds: &BTreeMap<HoldId, Hold>,
    assets: &BTreeMap<ObjId, EconomicAsset>,
    enabled: bool,
) -> Result<(), ParticipantError> {
    book.finish_replay(contracts, holds, journal_holds)?;
    if !enabled {
        book.require_absent()?;
    }
    let mut active = BTreeMap::new();
    for row in book.histories()? {
        let hull = row.consent().basis.hull;
        let actual = assets.get(&hull).ok_or_else(refused)?;
        let journal = journal_assets.get(&hull).ok_or_else(refused)?;
        let mut normalized = actual.clone();
        normalized.frozen = false;
        normalized.frozen_reason = None;
        if &normalized != journal {
            return Err(refused());
        }
        if let Some(terminal) = row.terminal() {
            if journal.last_journal_seq < terminal.journal_seq()
                || (journal.last_journal_seq == terminal.journal_seq()
                    && journal != terminal.title())
                || journal.encumbrances.contains(&Encumbrance::Collateral {
                    contract: row.contract_id(),
                })
            {
                return Err(refused());
            }
        } else {
            if journal != row.binding().title() || active.insert(row.contract_id(), hull).is_some()
            {
                return Err(refused());
            }
        }
    }
    // A matching collateral reference on another asset is not evidence that the
    // insured hull is still pledged. Check the inverse before moving contracts.
    for asset in assets.values().chain(journal_assets.values()) {
        for encumbrance in &asset.encumbrances {
            if let Encumbrance::Collateral { contract } = encumbrance
                && contracts.get(contract).is_some_and(|c| {
                    c.consent
                        .managed
                        .as_ref()
                        .and_then(|m| m.hull_coverage_marker())
                        .is_some()
                })
                && active.get(contract) != Some(&asset.object)
            {
                return Err(refused());
            }
        }
    }
    for row in book.histories()? {
        snapshot.put_contract(contracts.remove(&row.contract_id()).ok_or_else(refused)?);
    }
    Ok(())
}
