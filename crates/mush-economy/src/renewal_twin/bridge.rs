//! Closed Economy half of renewal composition and complete boot recovery.
//! Neither decoded rows nor a disabled/unbooted owner establish empty history.
use super::{
    Issuer,
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
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct Configuration<I: Issuer> {
    pub(crate) manifest: DefinitionRef,
    pub(crate) participants: [DefinitionRef; 2],
    pub(crate) actions: [DefinitionRef; 5],
    pub(crate) service_close: Option<DefinitionRef>,
    pub(crate) issuer: PhantomData<I>,
}
impl<I: Issuer> Configuration<I> {
    pub(crate) fn service_reference(&self) -> Result<DefinitionRef, ModError> {
        let [economy, retention] = &self.participants;
        let descriptor=DefinitionDescriptor::new(DefinitionKind::new("action").map_err(|_|failed())?,
            DefinitionId::new(I::SERVICE_CLOSE_ACTION).map_err(|_|failed())?,1,
            serde_json::json!({"format":1,"protocol":I::SERVICE_PROTOCOL,"transition":"unfunded_service_close","service_actor":1}))
            .map_err(|_|failed())?.with_references(vec![self.manifest.clone(),economy.clone(),retention.clone()]);
        DefinitionRegistryBuilder::new()
            .register(OwnerId::new("market").map_err(|_| failed())?, descriptor)
            .map_err(|_| failed())
    }

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
            (I::DRAFT_ACTION, "draft"),
            (I::FUND_ACTION, "fund"),
            (I::GRANT_ACTION, "grant"),
            (I::REFUND_ACTION, "refund"),
            (I::CLOSE_ACTION, "unfunded_close"),
        ]
        .into_iter()
        .zip(&self.actions)
        {
            let descriptor = DefinitionDescriptor::new(
                DefinitionKind::new("action").map_err(|_| failed())?,
                DefinitionId::new(id).map_err(|_| failed())?,
                1,
                serde_json::json!({"format":1,"protocol":I::PROTOCOL,"transition":transition}),
            )
            .map_err(|_| failed())?
            .with_references(vec![
                self.manifest.clone(),
                configured_economy.clone(),
                roll.clone(),
            ]);
            let expected = DefinitionRegistryBuilder::new()
                .register(OwnerId::new("market").map_err(|_| failed())?, descriptor)
                .map_err(|_| failed())?;
            if reference != &expected {
                return Err(failed());
            }
        }
        if let Some(action) = &self.service_close
            && action != &self.service_reference()?
        {
            return Err(failed());
        }
        Ok(())
    }
    pub(crate) fn require(
        &self,
        action: &DefinitionRef,
        id: &str,
    ) -> Result<(), mush_core::participant::ParticipantError> {
        let [draft, fund, grant, refund, close] = &self.actions;
        // An associated constant of the issuer cannot be a pattern: the arms of the old
        // `match`, in their order.
        let expected = if id == I::DRAFT_ACTION {
            draft
        } else if id == I::FUND_ACTION {
            fund
        } else if id == I::GRANT_ACTION {
            grant
        } else if id == I::REFUND_ACTION {
            refund
        } else if id == I::CLOSE_ACTION {
            close
        } else if id == I::SERVICE_CLOSE_ACTION {
            self.service_close.as_ref().ok_or_else(super::refused)?
        } else {
            return Err(super::refused());
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
pub(crate) fn documents<I: Issuer>(
    kv: &[(smol_str::SmolStr, serde_json::Value)],
    enabled: bool,
) -> Result<Vec<(String, Vec<u8>)>, ModError> {
    let mut documents = Vec::new();
    let mut offers = 0usize;
    let mut services = 0usize;
    for (name, value) in kv
        .iter()
        .filter(|(k, _)| k.starts_with(I::OFFER_PREFIX) || k.starts_with(I::SERVICE_PREFIX))
    {
        let (count, limit) = if name.starts_with(I::SERVICE_PREFIX) {
            (&mut services, super::service::MAX_BYTES)
        } else {
            (&mut offers, history::MAX_RECORD_BYTES)
        };
        if !enabled || *count == history::MAX_RECORDS {
            return Err(failed());
        }
        *count += 1;
        let bytes =
            mush_core::canonical_json_bytes_with_limit(value, limit).map_err(|_| failed())?;
        documents.push((name.to_string(), bytes));
    }
    Ok(documents)
}
/// A checkpoint can never hide renewal journal entries, even in disabled mode.
/// Old checkpoints with no renewal namespace retain their omitted empty field.
pub(crate) fn checkpoint_complete<I: Issuer>(
    fold: &crate::fold::FoldState,
    rows: &[crate::checkpoint::IdemRow],
    enabled: bool,
) -> bool {
    if enabled {
        return I::receipts(fold).checkpoint_complete(rows);
    }
    !rows.iter().any(|row| row.ns.as_str() == I::NAMESPACE)
        && (I::receipts(fold).uninitialized() || I::receipts(fold).checkpoint_complete(rows))
}
fn is_renewal<I: Issuer>(contract: &EconomicContract) -> bool {
    contract
        .consent
        .managed
        .as_ref()
        .and_then(I::marker)
        .is_some()
}
fn is_renewal_hold<I: Issuer>(hold: &Hold) -> bool {
    hold.holder.namespace.as_str() == I::NAMESPACE
}

/// Runs on the actual complete fold and raw contracts/holds before legacy
/// repair. The returned book is the only configured boot route to its latch.
pub(crate) fn recover<I: Issuer>(
    fold: &mut crate::fold::FoldState,
    snapshot: &mut EconomySnapshot,
    contracts: &mut BTreeMap<ContractId, EconomicContract>,
    holds: &BTreeMap<HoldId, Hold>,
    documents: &[(String, Vec<u8>)],
    enabled: bool,
) -> Result<OfferBook<I>, ModError> {
    if !enabled {
        if !documents.is_empty()
            || contracts.values().any(is_renewal::<I>)
            || holds.values().any(is_renewal_hold::<I>)
            || fold.journal_holds.values().any(is_renewal_hold::<I>)
            || !(I::receipts(fold).uninitialized() || I::receipts(fold).checkpoint_complete(&[]))
        {
            return Err(failed());
        }
        return Ok(OfferBook::default());
    }
    I::receipts_mut(fold)
        .finish_replay()
        .map_err(|_| failed())?;
    let mut exact = EconomySnapshot::new();
    let mut contract_ids = BTreeSet::new();
    for contract in contracts.values().filter(|c| is_renewal::<I>(c)) {
        if contract_ids.len() == history::MAX_RECORDS {
            return Err(failed());
        }
        contract_ids.insert(contract.id);
        exact.put_contract(contract.clone());
    }
    // Include every actual hold touching this inventory, including crossed
    // namespace claims. Unrelated legacy holds need no duplicate allocation.
    for (retained_holds, hold) in holds.values().filter(|h| is_renewal_hold::<I>(h) || matches!(h.purpose, crate::record::HoldPurpose::Escrow { contract } if contract_ids.contains(&contract))).enumerate() {
        if retained_holds == history::MAX_RECORDS { return Err(failed()); }
        exact.put_hold(hold.clone());
    }
    let mut hold_ids = BTreeSet::new();
    for actual in I::receipts(fold).receipts().map_err(|_| failed())? {
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
        .any(|h| is_renewal_hold::<I>(h) && !hold_ids.contains(&h.id))
        || fold
            .journal_holds
            .values()
            .any(|h| (is_renewal_hold::<I>(h) || matches!(h.purpose, crate::record::HoldPurpose::Escrow { contract } if contract_ids.contains(&contract))) && !hold_ids.contains(&h.id))
    {
        return Err(failed());
    }
    let book = OfferBook::from_documents_with_resale(
        &exact,
        documents,
        I::receipts(fold),
        &fold.paid_resale_receipts,
    )
    .map_err(|_| failed())?;
    for id in contract_ids {
        snapshot.put_contract(contracts.remove(&id).ok_or_else(failed)?);
    }
    Ok(book)
}

/// Source-free absence proof, independent of enabled renewal business policy.
/// The private latch is set by real boot only; default/unavailable is unknown.
pub(crate) fn require_absent<I: Issuer>(state: &EconomyStore) -> Result<(), RenewalAuditError> {
    if state.frozen || !I::inventory_checked(state) {
        return Err(RenewalAuditError::Incomplete);
    }
    if I::book(state).operations().len() != 0
        || state.snapshot.contracts().any(is_renewal::<I>)
        || state.snapshot.holds().any(is_renewal_hold::<I>)
        || state.fold.journal_holds.values().any(is_renewal_hold::<I>)
        || !(I::receipts(&state.fold).uninitialized()
            || I::receipts(&state.fold).checkpoint_complete(&[]))
    {
        return Err(RenewalAuditError::Refused);
    }
    Ok(())
}
