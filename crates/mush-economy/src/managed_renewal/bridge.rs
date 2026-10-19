//! Closed Economy half of renewal composition and complete boot recovery.
//! Neither decoded rows nor a disabled/unbooted owner establish empty history.
use super::{
    history::{self, OfferBook},
    receipt::RenewalAuditError,
};
use crate::contract::EconomicContract;
use crate::record::Hold;
use crate::{ContractId, EconomySnapshot, EconomyStore, HoldId};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{DefinitionRef, ModError};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(crate) struct Configuration {
    pub(crate) manifest: DefinitionRef,
    pub(crate) participants: [DefinitionRef; 2],
    pub(crate) actions: [DefinitionRef; 5],
}
impl Configuration {
    pub(crate) fn validate(&self, economy: &DefinitionRef) -> Result<(), ModError> {
        let [configured_economy, roll] = &self.participants;
        if configured_economy != economy
            || roll.key().owner.as_str() != "roll"
            || roll.key().kind.as_str() != "owner_participant"
            || roll.key().id.as_str() != "commercial_roots"
            || roll.version() != 1
            || self.manifest.key().owner.as_str() != "tacsim"
            || self.manifest.key().kind.as_str() != "registry_manifest"
            || self.manifest.key().id.as_str() != "combat"
            || self.manifest.version() != 1
        {
            return Err(failed());
        }
        for ((id, transition), reference) in [
            (super::DRAFT_ACTION, "draft"),
            (super::FUND_ACTION, "fund"),
            (super::GRANT_ACTION, "grant"),
            (super::REFUND_ACTION, "refund"),
            (super::CLOSE_ACTION, "unfunded_close"),
        ]
        .into_iter()
        .zip(&self.actions)
        {
            let descriptor = DefinitionDescriptor::new(
                DefinitionKind::new("action").map_err(|_| failed())?,
                DefinitionId::new(id).map_err(|_| failed())?, 1,
                serde_json::json!({"format":1,"protocol":"personal_mind_renewal_v1","transition":transition}),
            ).map_err(|_| failed())?.with_references(vec![self.manifest.clone(), configured_economy.clone(), roll.clone()]);
            let expected = DefinitionRegistryBuilder::new()
                .register(OwnerId::new("market").map_err(|_| failed())?, descriptor)
                .map_err(|_| failed())?;
            if reference != &expected {
                return Err(failed());
            }
        }
        Ok(())
    }
    pub(crate) fn require(
        &self,
        action: &DefinitionRef,
        id: &str,
    ) -> Result<(), mush_core::participant::ParticipantError> {
        let [draft, fund, grant, refund, close] = &self.actions;
        let expected = match id {
            super::DRAFT_ACTION => draft,
            super::FUND_ACTION => fund,
            super::GRANT_ACTION => grant,
            super::REFUND_ACTION => refund,
            super::CLOSE_ACTION => close,
            _ => return Err(super::refused()),
        };
        if expected != action {
            return Err(super::refused());
        }
        Ok(())
    }
}
fn failed() -> ModError {
    ModError::msg("economy: renewal composition or complete recovery refused")
}

/// Retain only exact bounded immutable owner documents. Disabled composition
/// refuses their presence before any legacy projection can be healed.
pub(crate) fn documents(
    kv: &[(smol_str::SmolStr, serde_json::Value)],
    enabled: bool,
) -> Result<Vec<(String, Vec<u8>)>, ModError> {
    let mut documents = Vec::new();
    for (name, value) in kv.iter().filter(|(k, _)| k.starts_with(history::PREFIX)) {
        if !enabled || documents.len() == history::MAX_RECORDS {
            return Err(failed());
        }
        let bytes = mush_core::canonical_json_bytes_with_limit(value, history::MAX_RECORD_BYTES)
            .map_err(|_| failed())?;
        documents.push((name.to_string(), bytes));
    }
    Ok(documents)
}
/// A checkpoint can never hide renewal journal entries, even in disabled mode.
/// Old checkpoints with no renewal namespace retain their omitted empty field.
pub(crate) fn checkpoint_complete(
    fold: &crate::fold::FoldState,
    rows: &[crate::checkpoint::IdemRow],
    enabled: bool,
) -> bool {
    if enabled {
        return fold.renewal_receipts.checkpoint_complete(rows);
    }
    !rows.iter().any(|row| row.ns.as_str() == super::NAMESPACE)
        && (fold.renewal_receipts.uninitialized()
            || fold.renewal_receipts.checkpoint_complete(rows))
}
fn is_renewal(contract: &EconomicContract) -> bool {
    contract
        .consent
        .managed
        .as_ref()
        .and_then(|m| m.renewal_offer_marker())
        .is_some()
}
fn is_renewal_hold(hold: &Hold) -> bool {
    hold.holder.namespace.as_str() == super::NAMESPACE
}

/// Runs on the actual complete fold and raw contracts/holds before legacy
/// repair. The returned book is the only configured boot route to its latch.
pub(crate) fn recover(
    fold: &mut crate::fold::FoldState,
    snapshot: &mut EconomySnapshot,
    contracts: &mut BTreeMap<ContractId, EconomicContract>,
    holds: &BTreeMap<HoldId, Hold>,
    documents: &[(String, Vec<u8>)],
    enabled: bool,
) -> Result<OfferBook, ModError> {
    if !enabled {
        if !documents.is_empty()
            || contracts.values().any(is_renewal)
            || holds.values().any(is_renewal_hold)
            || fold.journal_holds.values().any(is_renewal_hold)
            || !(fold.renewal_receipts.uninitialized()
                || fold.renewal_receipts.checkpoint_complete(&[]))
        {
            return Err(failed());
        }
        return Ok(OfferBook::default());
    }
    fold.renewal_receipts
        .finish_replay()
        .map_err(|_| failed())?;
    let mut exact = EconomySnapshot::new();
    let mut contract_ids = BTreeSet::new();
    for contract in contracts.values().filter(|c| is_renewal(c)) {
        if contract_ids.len() == history::MAX_RECORDS {
            return Err(failed());
        }
        contract_ids.insert(contract.id);
        exact.put_contract(contract.clone());
    }
    // Include every actual hold touching this inventory, including crossed
    // namespace claims. Unrelated legacy holds need no duplicate allocation.
    let mut retained_holds = 0usize;
    for hold in holds.values().filter(|h| is_renewal_hold(h) || matches!(h.purpose, crate::record::HoldPurpose::Escrow { contract } if contract_ids.contains(&contract))) {
        if retained_holds == history::MAX_RECORDS { return Err(failed()); }
        retained_holds += 1;
        exact.put_hold(hold.clone());
    }
    let mut hold_ids = BTreeSet::new();
    for actual in fold.renewal_receipts.receipts().map_err(|_| failed())? {
        let receipt = actual.funding();
        let contract = contracts.get(&receipt.contract()).ok_or_else(failed)?;
        let hold = holds.get(&receipt.hold()).ok_or_else(failed)?;
        if !contract_ids.contains(&contract.id)
            || !hold_ids.insert(hold.id)
            || fold.journal_holds.get(&hold.id) != Some(hold)
        {
            return Err(failed());
        }
        actual
            .verify_documents(contract, hold)
            .map_err(|_| failed())?;
    }
    if holds
        .values()
        .any(|h| is_renewal_hold(h) && !hold_ids.contains(&h.id))
        || fold
            .journal_holds
            .values()
            .any(|h| (is_renewal_hold(h) || matches!(h.purpose, crate::record::HoldPurpose::Escrow { contract } if contract_ids.contains(&contract))) && !hold_ids.contains(&h.id))
    {
        return Err(failed());
    }
    let book = OfferBook::from_documents_with_renewals(&exact, documents, &fold.renewal_receipts)
        .map_err(|_| failed())?;
    for id in contract_ids {
        snapshot.put_contract(contracts.remove(&id).ok_or_else(failed)?);
    }
    Ok(book)
}

/// Source-free absence proof, independent of enabled renewal business policy.
/// The private latch is set by real boot only; default/unavailable is unknown.
pub(crate) fn require_absent(state: &EconomyStore) -> Result<(), RenewalAuditError> {
    if state.frozen || !state.renewal_inventory_checked {
        return Err(RenewalAuditError::Incomplete);
    }
    if state.renewal_offers.operations().len() != 0
        || state.snapshot.contracts().any(is_renewal)
        || state.snapshot.holds().any(is_renewal_hold)
        || state.fold.journal_holds.values().any(is_renewal_hold)
        || !(state.fold.renewal_receipts.uninitialized()
            || state.fold.renewal_receipts.checkpoint_complete(&[]))
    {
        return Err(RenewalAuditError::Refused);
    }
    Ok(())
}
