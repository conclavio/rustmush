//! Mandatory raw-document/replay pairing before legacy contract repair. Missing
//! owner history is not an empty inventory; disabled composition rejects it.
use super::{NAMESPACE, refused};
use crate::contract::EconomicContract;
use crate::fold::FoldState;
use crate::record::{Hold, HoldPurpose};
use crate::{ContractId, EconomySnapshot, HoldId};
use mush_core::participant::ParticipantError;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn recover(
    state: &mut FoldState,
    snapshot: &mut EconomySnapshot,
    contracts: &mut BTreeMap<ContractId, EconomicContract>,
    holds: &BTreeMap<HoldId, Hold>,
    enabled: bool,
) -> Result<(), ParticipantError> {
    state.charter_pending_receipts.finish_replay()?;
    if !enabled {
        state.charter_pending_receipts.require_absent()?;
    }
    let mut ids = BTreeSet::new();
    let mut hold_ids = BTreeSet::new();
    for history in state.charter_pending_receipts.histories()? {
        let c = contracts.get(&history.contract_id()).ok_or_else(refused)?;
        let h = holds.get(&history.funding().hold()).ok_or_else(refused)?;
        history.verify_documents(c, h)?;
        if state.journal_holds.get(&h.id) != Some(h) || !ids.insert(c.id) || !hold_ids.insert(h.id)
        {
            return Err(refused());
        }
    }
    if contracts.values().any(|c| {
        c.consent
            .managed
            .as_ref()
            .and_then(|m| m.charter_pending_marker())
            .is_some()
            && !ids.contains(&c.id)
    }) {
        return Err(refused());
    }
    // Both physical documents and journal-only holds are checked. A foreign
    // namespace pointing to a real pending contract cannot be adopted later.
    for h in holds.values().chain(state.journal_holds.values()) {
        let belongs = h.holder.namespace.as_str() == NAMESPACE
            || matches!(h.purpose, HoldPurpose::Escrow { contract } if ids.contains(&contract));
        if belongs && (!hold_ids.contains(&h.id) || state.journal_holds.get(&h.id) != Some(h)) {
            return Err(refused());
        }
    }
    state
        .charter_transfer_receipts
        .verify_pending_lineages(&state.charter_pending_receipts)
        .map_err(|_| refused())?;
    for id in ids {
        snapshot.put_contract(contracts.remove(&id).ok_or_else(refused)?);
    }
    Ok(())
}
