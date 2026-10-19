//! Complete native/typed/Root census, including the all-missing typed case.
use super::*;
use crate::commerce::{
    audit::{read, scan},
    families::CommerceFamilies,
    records::Record,
};
use mush_core::ModuleCtx;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn audit(
    ctx: &ModuleCtx<'_>,
    f: &CommerceFamilies,
    enabled: bool,
    pending_enabled: bool,
) -> Result<(), PublicationError> {
    let e = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    let native = e.charter_transfer_receipt_audit();
    native.require_complete().map_err(bad)?;
    // Retain compact identities only; immutable source-free payloads stay in
    // their actual typed owner store throughout the complete boot census.
    let mut offers = BTreeSet::new();
    let mut chains = BTreeMap::<CharterId, BTreeMap<u64, mush_core::CommitToken>>::new();
    let mut commits = BTreeSet::new();
    let mut immediate_commits = BTreeSet::new();
    scan(ctx, &f.charter_transfer_offers, |offer| {
        if !enabled || offers.len() >= crate::MAX_RECORDS || !offers.insert(offer.operation) {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    scan(ctx, &f.charter_transfers, |row| {
        if !enabled
            || commits.len() >= MAX_TRANSFERS as usize
            || !commits.insert(row.operation)
            || offers.contains(&row.operation)
            || chains
                .entry(row.charter)
                .or_default()
                .insert(row.generation, row.operation)
                .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        match &row.economic {
            economic::TransferEconomic::Immediate(_) => {
                immediate_commits.insert(row.operation);
            }
            economic::TransferEconomic::Pending(_) if pending_enabled => {}
            _ => return Err(PublicationError::Mismatch),
        }
        Ok(())
    })?;
    for (charter, chain) in &chains {
        let c = read(ctx, &f.charters, charter.key())?;
        let birth = read(ctx, &f.operations, c.operation.to_hex())?;
        let mut previous = CharterOwnershipRef::birth(birth)?;
        let mut holder = birth.requester;
        let mut seq = birth.economic.journal_seq;
        let mut used_offers = BTreeSet::new();
        for (generation, operation) in chain {
            let row = read(
                ctx,
                &f.charter_transfers,
                CharterTransfer::key_for(*charter, *generation),
            )?;
            if row.operation != *operation {
                return Err(PublicationError::Mismatch);
            }
            let required = match &row.economic {
                economic::TransferEconomic::Immediate(_) => {
                    if !offers.contains(&row.offer) {
                        return Err(PublicationError::Mismatch);
                    }
                    let offer = read(ctx, &f.charter_transfer_offers, row.offer.to_hex())?;
                    offer.matches_birth(birth)?;
                    history::transfer_root(row, offer)?
                }
                economic::TransferEconomic::Pending(native) => {
                    let offer = read(ctx, &f.charter_pending_offers, row.offer.to_hex())?;
                    let pending = read(
                        ctx,
                        &f.charter_pending,
                        native.pending.consent().operation.to_hex(),
                    )?;
                    offer.matches_birth(birth)?;
                    super::super::charter_pending::history::transfer_root(row, offer, pending)?
                }
            };
            let facts = row.economic.facts()?;
            if row.predecessor != previous
                || facts.seller != holder
                || facts.journal_seq <= seq
                || !used_offers.insert(row.offer)
            {
                return Err(PublicationError::Mismatch);
            }
            row.economic.verify(e)?;
            previous = row.ownership()?;
            holder = facts.buyer;
            seq = facts.journal_seq;
            ctx.w
                .module_state::<crate::RollStore>()
                .ok_or(PublicationError::Unavailable)?
                .check_required_root(&required)
                .map_err(bad)?;
        }
        let head = read(ctx, &f.charter_ownership, charter.key())?;
        if head.charter != *charter
            || head.current != previous
            || e.snapshot()
                .asset(charter.object())
                .is_none_or(|a| a.title_owner != mush_org::PrincipalId::Object(holder))
        {
            return Err(PublicationError::Mismatch);
        }
    }
    scan(ctx, &f.charter_ownership, |head| {
        let (generation, operation) = chains
            .get(&head.charter)
            .and_then(|c| c.last_key_value())
            .ok_or(PublicationError::Missing)?;
        let last = read(
            ctx,
            &f.charter_transfers,
            CharterTransfer::key_for(head.charter, *generation),
        )?;
        if !enabled || last.operation != *operation || head.current != last.ownership()? {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    for operation in &offers {
        let offer = read(ctx, &f.charter_transfer_offers, operation.to_hex())?;
        let birth = read(ctx, &f.operations, offer.birth.to_hex())?;
        offer.matches_birth(birth)?;
        let filed = read(ctx, &f.filed, offer.filed.key())?;
        if filed.content.position().charter != offer.charter
            || filed.content.position().line != offer.line
            || filed.content.revision() != offer.revision
            || history::historical(ctx, f, birth, &offer.ownership)? != offer.seller
        {
            return Err(PublicationError::Mismatch);
        }
        if offer.ownership.generation > 1 {
            let operation = chains
                .get(&offer.charter)
                .and_then(|c| c.get(&offer.ownership.generation))
                .ok_or(PublicationError::Missing)?;
            let predecessor = read(
                ctx,
                &f.charter_transfers,
                CharterTransfer::key_for(offer.charter, offer.ownership.generation),
            )?;
            if predecessor.operation != *operation {
                return Err(PublicationError::Mismatch);
            }
            if offer.offered_at_utc < predecessor.economic.facts()?.posted_at_utc {
                return Err(PublicationError::Mismatch);
            }
        }
        ctx.w
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .check_required_root(&history::offer_root(offer)?)
            .map_err(bad)?;
    }
    let actual = native.receipts().map_err(bad)?;
    if actual.len() != immediate_commits.len() {
        return Err(PublicationError::Mismatch);
    }
    for receipt in actual {
        if !immediate_commits.remove(&receipt.commit) {
            return Err(PublicationError::Mismatch);
        }
    }
    Ok(())
}

pub(crate) fn root(
    ctx: &ModuleCtx<'_>,
    f: &CommerceFamilies,
    anchor: &crate::commerce::roots::RootAnchor,
) -> Result<RequiredRoot, PublicationError> {
    use crate::commerce::roots::RootAnchor;
    match anchor {
        RootAnchor::CharterTransferOffer(commit, _) => {
            history::offer_root(read(ctx, &f.charter_transfer_offers, commit.to_hex())?)
        }
        RootAnchor::CharterTransfer(charter, generation, commit, _) => {
            let row = read(
                ctx,
                &f.charter_transfers,
                CharterTransfer::key_for(*charter, *generation),
            )?;
            if row.operation != *commit {
                return Err(PublicationError::Mismatch);
            }
            match &row.economic {
                economic::TransferEconomic::Immediate(_) => history::transfer_root(
                    row,
                    read(ctx, &f.charter_transfer_offers, row.offer.to_hex())?,
                ),
                economic::TransferEconomic::Pending(native) => {
                    super::super::charter_pending::history::transfer_root(
                        row,
                        read(ctx, &f.charter_pending_offers, row.offer.to_hex())?,
                        read(
                            ctx,
                            &f.charter_pending,
                            native.pending.consent().operation.to_hex(),
                        )?,
                    )
                }
            }
        }
        _ => Err(PublicationError::Mismatch),
    }
}
