//! Closed new-protocol half of the shared paid-history audit. Empty scans are
//! mandatory; no configured entitlement owner means every new family is empty.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ModuleCtx;
use mush_economy::EconomyStore;

use super::super::audit::{read, scan};
use super::super::families::CommerceFamilies;
use super::super::purchase::PurchasePhase;
use super::super::rating::RatingFamilies;
use super::super::records::{PublicationError, Record};
use super::super::roots::{RequiredRoot, RootAnchor};
use super::super::{LicenceId, RevisionIdentity};
use super::HouseAllocationV1;
use super::history;
use crate::RollStore;

pub(crate) fn required(row: &HouseAllocationV1) -> Result<Vec<RequiredRoot>, PublicationError> {
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
        if scan(ctx, &f.house_allocations, |_| Ok(()))? != 0
            || scan(ctx, &f.house_economics, |_| Ok(()))? != 0
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

    scan(ctx, &f.house_allocations, |row| {
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
        if ctx
            .action_state(&f.entitlement_allocations, &row.operation.to_hex())
            .map_err(|_| PublicationError::Unavailable)?
            .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let filed = read(ctx, &f.filed, row.filed.key())?;
        let charter = read(ctx, &f.charters, row.charter.key())?;
        let birth = read(ctx, &f.operations, charter.operation.to_hex())?;
        let house = birth
            .economic
            .house
            .as_ref()
            .ok_or(PublicationError::Mismatch)?;
        if birth.format != 2
            || house.organization() != row.issuer.organization()
            || house.scope() != row.issuer.scope()
            || birth.economic.issued_owner != row.issuer.principal()
            || birth.charter != row.charter
            || birth.line != row.line
        {
            return Err(PublicationError::Mismatch);
        }
        economy
            .filing_receipt_audit()
            .verify(&birth.audit_subject()?)
            .map_err(|_| PublicationError::Mismatch)?;

        if filed.content.revision() != row.revision
            || filed.content.position().charter != row.charter
            || filed.content.position().line != row.line
            || filed.content.parent().is_some()
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
        history::verify_history(row, actual, rated.compatibility().digest())?;
        let licence = ctx
            .action_state(&f.paid_licences, &row.licence.key())
            .map_err(|_| PublicationError::Unavailable)?;
        let term_key = format!("{}/{}", row.licence.key(), row.term.key());
        let term = ctx
            .action_state(&f.paid_terms, &term_key)
            .map_err(|_| PublicationError::Unavailable)?;
        let economics = ctx
            .action_state(&f.house_economics, &term_key)
            .map_err(|_| PublicationError::Unavailable)?;
        match row.phase {
            PurchasePhase::Granted { .. } => history::verify_grant(
                row,
                licence.ok_or(PublicationError::Missing)?,
                term.ok_or(PublicationError::Missing)?,
                economics.ok_or(PublicationError::Missing)?,
                actual,
                rated.compatibility().digest(),
            )?,
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
    scan(ctx, &f.house_economics, |row| {
        if row.generation().get() == 1 {
            let birth = read(ctx, &f.house_allocations, row.operation().to_hex())?;
            if !matches!(birth.phase, PurchasePhase::Granted { .. }) || &birth.economics != row {
                return Err(PublicationError::Mismatch);
            }
        } else {
            let operation = read(ctx, &f.house_renewal_operations, row.operation().to_hex())?;
            let licence = read(ctx, &f.paid_licences, row.licence().key())?;
            let birth = read(ctx, &f.house_allocations, licence.operation().to_hex())?;
            if operation.granted_rows()?.2 != *row
                || !operation.binding().matches_birth(licence)
                || operation.binding().issuer() != birth.issuer()
            {
                return Err(PublicationError::Mismatch);
            }
        }
        Ok(())
    })?;
    Ok(roots)
}
