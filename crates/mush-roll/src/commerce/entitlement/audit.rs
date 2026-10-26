//! Closed new-protocol half of the shared paid-history audit. Empty scans are
//! mandatory; no configured entitlement owner means every new family is empty.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ModuleCtx;
use mush_economy::{EconomyStore, managed_entitlement::EntitlementProtocol};

use super::super::audit::{read, scan};
use super::super::families::CommerceFamilies;
use super::super::purchase::PurchasePhase;
use super::super::rating::RatingFamilies;
use super::super::records::{PublicationError, Record};
use super::super::roots::{RequiredRoot, RootAnchor};
use super::super::{LicenceId, RevisionIdentity};
use super::history;
use super::{EntitlementAllocationV1, royalties};
use crate::RollStore;

pub(crate) fn required(
    row: &EntitlementAllocationV1,
) -> Result<Vec<RequiredRoot>, PublicationError> {
    row.validate()?;
    Ok(vec![
        RequiredRoot {
            anchor: RootAnchor::PaidOperation(row.operation),
            identity: row.revision,
        },
        RequiredRoot {
            anchor: RootAnchor::Term(row.licence, row.term),
            identity: row.revision,
        },
    ])
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn audit(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    enabled: bool,
    ids: &mut BTreeSet<LicenceId>,
    allocated: &mut impl FnMut(u64) -> Result<(), PublicationError>,
) -> Result<BTreeMap<RootAnchor, RevisionIdentity>, PublicationError> {
    let mut roots = BTreeMap::new();
    if !enabled {
        if scan(ctx, &f.entitlement_allocations, |_| Ok(()))? != 0
            || scan(ctx, &f.term_economics, |_| Ok(()))? != 0
            || scan(ctx, &f.filed_royalties, |_| Ok(()))? != 0
        {
            return Err(PublicationError::Unavailable);
        }
        return Ok(roots);
    }
    let economy = ctx
        .state::<EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    if economy.is_frozen() {
        return Err(PublicationError::Unavailable);
    }
    let history = economy.purchase_receipt_audit();
    history
        .require_complete()
        .map_err(|_| PublicationError::Unavailable)?;

    scan(ctx, &f.filed_royalties, |row| {
        royalties::verify_stored(ctx, roll, f, ratings, row)
    })?;
    scan(ctx, &f.entitlement_allocations, |row| {
        if !ids.insert(row.licence)
            || ctx
                .action_state(&f.licences, &row.licence.key())
                .map_err(|_| PublicationError::Unavailable)?
                .is_some()
            || ctx
                .action_state(&f.paid_allocations, &row.operation.to_hex())
                .map_err(|_| PublicationError::Unavailable)?
                .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        allocated(row.licence.get())?;
        let filed = read(ctx, &f.filed, row.filed.key())?;
        let original = filed.content.parent().is_none();
        if filed.content.revision() != row.revision
            || filed.content.position().charter != row.charter
            || filed.content.position().line != row.line
            || original != (row.protocol == EntitlementProtocol::OriginalV3)
        {
            return Err(PublicationError::Mismatch);
        }
        let chain = read(ctx, &f.filed_royalties, row.filed.key())?;
        if chain.revision() != row.revision
            || chain.reference()? != row.economics.grant().royalty_chain()
        {
            return Err(PublicationError::Mismatch);
        }
        let rated = super::super::rating::reconformance::history::certificate(
            ctx,
            ratings,
            filed,
            &row.rating,
        )
        .map_err(|_| PublicationError::Unavailable)?;
        if rated.subject().revision() != row.revision || row.terms.class() < rated.minimum() {
            return Err(PublicationError::Mismatch);
        }
        let actual = history
            .get(row.operation)
            .map_err(|_| PublicationError::Unavailable)?;
        history::verify_entitlement_history(row, actual, rated.compatibility().digest())?;
        let licence = ctx
            .action_state(&f.paid_licences, &row.licence.key())
            .map_err(|_| PublicationError::Unavailable)?;
        let term_key = format!("{}/{}", row.licence.key(), row.term.key());
        let term = ctx
            .action_state(&f.paid_terms, &term_key)
            .map_err(|_| PublicationError::Unavailable)?;
        let economics = ctx
            .action_state(&f.term_economics, &term_key)
            .map_err(|_| PublicationError::Unavailable)?;
        match row.phase {
            PurchasePhase::Granted { .. } => {
                let licence = licence.ok_or(PublicationError::Missing)?;
                let birth = super::super::paid::PersonalPaidLicence::original(
                    row.licence,
                    row.charter,
                    row.line,
                    row.holder,
                    licence.term(),
                    row.operation,
                )?;
                history::verify_entitlement_grant(
                    row,
                    &birth,
                    term.ok_or(PublicationError::Missing)?,
                    economics.ok_or(PublicationError::Missing)?,
                    actual,
                    rated.compatibility().digest(),
                )?;
                super::super::paid_resale::history::current_from_birth(
                    ctx,
                    f,
                    licence,
                    super::super::entitlement::history::Birth::Entitlement(row),
                )?;
            }
            PurchasePhase::Funded {} | PurchasePhase::Refunded { .. } => {
                if licence.is_some() || term.is_some() || economics.is_some() {
                    return Err(PublicationError::Mismatch);
                }
            }
        }
        for required in required(row)? {
            roll.check_required_root(&required)
                .map_err(|_| PublicationError::Mismatch)?;
            if roots.insert(required.anchor, required.identity).is_some() {
                return Err(PublicationError::Mismatch);
            }
        }
        Ok(())
    })?;
    // Generation one belongs to its actual birth; every later generation must
    // name its own actual Granted renewal. The full attempt/head audit also
    // runs before readiness, rejecting disabled/all-missing/branched history.
    scan(ctx, &f.term_economics, |row| {
        if row.generation().get() == 1 {
            let birth = read(ctx, &f.entitlement_allocations, row.operation().to_hex())?;
            if !matches!(birth.phase, PurchasePhase::Granted { .. }) || &birth.economics != row {
                return Err(PublicationError::Mismatch);
            }
        } else {
            let operation = read(ctx, &f.renewal_operations, row.operation().to_hex())?;
            if operation.granted_rows()?.2 != *row {
                return Err(PublicationError::Mismatch);
            }
        }
        Ok(())
    })?;
    Ok(roots)
}
