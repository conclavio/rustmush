//! Retained paid allocations are obligations even before grant and after refund.
//! Boot checks immutable receipt/row/root joins, never today's seller rights.
use super::audit::{read, scan};
use super::families::CommerceFamilies;
use super::paid::PersonalPaidLicence;
use super::purchase::{PaidAllocation, PurchasePhase};
use super::rating::RatingFamilies;
use super::records::{PublicationError, Record, RevisionPolicy};
use super::roots::RootAnchor;
use super::{RevisionIdentity, WorldIcSeconds};
use crate::RollStore;
use mush_core::ModuleCtx;
use mush_economy::{EconomyStore, managed_purchase::PurchaseHistory};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn audit(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    enabled: bool,
    allocated: impl FnMut(u64) -> Result<(), PublicationError>,
) -> Result<BTreeMap<RootAnchor, RevisionIdentity>, PublicationError> {
    audit_with_entitlements(ctx, roll, f, ratings, enabled, false, allocated)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn audit_with_entitlements(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    enabled: bool,
    entitlements: bool,
    allocated: impl FnMut(u64) -> Result<(), PublicationError>,
) -> Result<BTreeMap<RootAnchor, RevisionIdentity>, PublicationError> {
    audit_with_renewals(
        ctx,
        roll,
        f,
        ratings,
        enabled,
        entitlements,
        false,
        allocated,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn audit_with_renewals(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    enabled: bool,
    entitlements: bool,
    renewals: bool,
    allocated: impl FnMut(u64) -> Result<(), PublicationError>,
) -> Result<BTreeMap<RootAnchor, RevisionIdentity>, PublicationError> {
    audit_with_houses(
        ctx,
        roll,
        f,
        ratings,
        enabled,
        entitlements,
        renewals,
        false,
        allocated,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn audit_with_houses(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    enabled: bool,
    entitlements: bool,
    renewals: bool,
    houses: bool,
    allocated: impl FnMut(u64) -> Result<(), PublicationError>,
) -> Result<BTreeMap<RootAnchor, RevisionIdentity>, PublicationError> {
    audit_with_house_renewals(
        ctx,
        roll,
        f,
        ratings,
        enabled,
        entitlements,
        renewals,
        houses,
        false,
        allocated,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn audit_with_house_renewals(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    enabled: bool,
    entitlements: bool,
    renewals: bool,
    houses: bool,
    house_renewals: bool,
    mut allocated: impl FnMut(u64) -> Result<(), PublicationError>,
) -> Result<BTreeMap<RootAnchor, RevisionIdentity>, PublicationError> {
    use super::entitlement::history::{self, Birth};
    let mut roots = BTreeMap::new();
    if !enabled && !houses {
        if scan(ctx, &f.paid_allocations, |_| Ok(()))? != 0
            || scan(ctx, &f.paid_licences, |_| Ok(()))? != 0
            || scan(ctx, &f.paid_terms, |_| Ok(()))? != 0
        {
            return Err(PublicationError::Unavailable);
        }
        // New families are never a healthy ignored extension of an old-only
        // or disabled deployment, including an all-missing old allocation set.
        super::entitlement::audit::audit(
            ctx,
            roll,
            f,
            ratings,
            false,
            &mut BTreeSet::new(),
            &mut allocated,
        )?;
        super::renewal::audit::audit(ctx, roll, f, ratings, false)?;
        super::house_renewal::audit::audit(ctx, roll, f, ratings, false)?;
        super::house_purchase::audit::audit(
            ctx,
            roll,
            f,
            ratings,
            false,
            &mut BTreeSet::new(),
            &mut allocated,
        )?;
        if entitlements || renewals || houses || house_renewals {
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
    let mut ids = BTreeSet::new();
    scan(ctx, &f.paid_allocations, |row| {
        if !enabled {
            return Err(PublicationError::Unavailable);
        }
        if !ids.insert(row.licence)
            || ctx
                .action_state(&f.licences, &row.licence.key())
                .map_err(|_| PublicationError::Unavailable)?
                .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        allocated(row.licence.get())?;
        let filed = read(ctx, &f.filed, row.filed.key())?;
        if filed.content.revision() != row.revision
            || filed.content.position().charter != row.charter
            || filed.content.position().line != row.line
            || filed.content.parent().is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let rated =
            super::rating::reconformance::history::certificate(ctx, ratings, filed, &row.rating)
                .map_err(|_| PublicationError::Unavailable)?;
        if rated.subject().revision() != row.revision || row.terms.class() < rated.minimum() {
            return Err(PublicationError::Mismatch);
        }
        let actual = history
            .get(row.operation)
            .map_err(|_| PublicationError::Unavailable)?;
        verify_history(row, actual)?;
        let licence = ctx
            .action_state(&f.paid_licences, &row.licence.key())
            .map_err(|_| PublicationError::Unavailable)?;
        let term = ctx
            .action_state(
                &f.paid_terms,
                &format!("{}/{}", row.licence.key(), row.term.key()),
            )
            .map_err(|_| PublicationError::Unavailable)?;
        match row.phase {
            PurchasePhase::Granted { .. } => {
                let licence = licence.ok_or(PublicationError::Missing)?;
                let term = term.ok_or(PublicationError::Missing)?;
                let birth = PersonalPaidLicence::original(
                    row.licence,
                    row.charter,
                    row.line,
                    row.holder,
                    row.term,
                    row.operation,
                )?;
                verify_granted(row, &birth, term, actual)?;
                if licence.term() != row.term {
                    return Err(PublicationError::Mismatch);
                }
                super::paid_resale::history::current_from_birth(
                    ctx,
                    f,
                    licence,
                    super::entitlement::history::Birth::Legacy(row),
                )?;
            }
            PurchasePhase::Funded {} | PurchasePhase::Refunded { .. } => {
                if licence.is_some() || term.is_some() {
                    return Err(PublicationError::Mismatch);
                }
            }
        }
        for required in super::paid_roots::required(row)? {
            roll.check_required_root(&required)
                .map_err(|_| PublicationError::Mismatch)?;
            if roots.insert(required.anchor, required.identity).is_some() {
                return Err(PublicationError::Mismatch);
            }
        }
        Ok(())
    })?;
    for (anchor, identity) in super::entitlement::audit::audit(
        ctx,
        roll,
        f,
        ratings,
        entitlements,
        &mut ids,
        &mut allocated,
    )? {
        if roots.insert(anchor, identity).is_some() {
            return Err(PublicationError::Mismatch);
        }
    }
    for (anchor, identity) in super::house_purchase::audit::audit(
        ctx,
        roll,
        f,
        ratings,
        houses,
        &mut ids,
        &mut allocated,
    )? {
        if roots.insert(anchor, identity).is_some() {
            return Err(PublicationError::Mismatch);
        }
    }
    for actual in history
        .receipts()
        .map_err(|_| PublicationError::Unavailable)?
    {
        match history::resolve(ctx, f, actual.operation())? {
            Birth::House(row) => {
                if !houses {
                    return Err(PublicationError::Unavailable);
                }
                let rated = ratings
                    .resolve_receipt(ctx, &row.rating)
                    .map_err(|_| PublicationError::Unavailable)?;
                super::house_purchase::history::verify_history(
                    row,
                    actual,
                    rated.compatibility().digest(),
                )?;
            }
            Birth::Legacy(row) => {
                if !enabled {
                    return Err(PublicationError::Unavailable);
                }
                if actual
                    .entitlement()
                    .map_err(|_| PublicationError::Mismatch)?
                    .is_some()
                {
                    return Err(PublicationError::Mismatch);
                }
                verify_history(row, actual)?;
            }
            Birth::Entitlement(row) => {
                if !entitlements {
                    return Err(PublicationError::Unavailable);
                }
                let rated = ratings
                    .resolve_receipt(ctx, &row.rating)
                    .map_err(|_| PublicationError::Unavailable)?;
                history::verify_entitlement_history(row, actual, rated.compatibility().digest())?;
            }
        }
    }
    // Every retained offer, including the zero-row case, must match the actual
    // Economy replay audit before any current-generation pointer is accepted.
    if renewals && !entitlements {
        return Err(PublicationError::Unavailable);
    }
    for (anchor, identity) in super::renewal::audit::audit(ctx, roll, f, ratings, renewals)? {
        if roots.insert(anchor, identity).is_some() {
            return Err(PublicationError::Mismatch);
        }
    }
    if house_renewals && !houses {
        return Err(PublicationError::Unavailable);
    }
    for (anchor, identity) in
        super::house_renewal::audit::audit(ctx, roll, f, ratings, house_renewals)?
    {
        if roots.insert(anchor, identity).is_some() {
            return Err(PublicationError::Mismatch);
        }
    }
    scan(ctx, &f.paid_licences, |row| {
        let birth = history::resolve(ctx, f, row.operation())?;
        if birth.licence() != row.id() || !matches!(birth.phase(), PurchasePhase::Granted { .. }) {
            return Err(PublicationError::Mismatch);
        }
        let term = read(
            ctx,
            &f.paid_terms,
            format!("{}/{}", row.id().key(), row.term().key()),
        )?;
        history::current(ctx, f, ratings, row, term)?;
        Ok(())
    })?;
    scan(ctx, &f.paid_terms, |row| {
        if row.generation().get() == 1 {
            let birth = history::resolve(ctx, f, row.operation())?;
            if birth.licence() != row.licence()
                || birth.term() != row.generation()
                || !matches!(birth.phase(), PurchasePhase::Granted { .. })
            {
                return Err(PublicationError::Mismatch);
            }
        } else {
            let licence = read(ctx, &f.paid_licences, row.licence().key())?;
            match history::resolve(ctx, f, licence.operation())? {
                Birth::House(_) if house_renewals => {
                    let operation =
                        read(ctx, &f.house_renewal_operations, row.operation().to_hex())?;
                    if operation.granted_rows()?.1 != *row {
                        return Err(PublicationError::Mismatch);
                    }
                }
                Birth::Entitlement(_) if renewals => {
                    let operation = read(ctx, &f.renewal_operations, row.operation().to_hex())?;
                    if operation.granted_rows()?.1 != *row {
                        return Err(PublicationError::Mismatch);
                    }
                }
                _ => return Err(PublicationError::Unavailable),
            }
        }
        Ok(())
    })?;
    Ok(roots)
}
fn verify_history(row: &PaidAllocation, actual: &PurchaseHistory) -> Result<(), PublicationError> {
    row.validate()?;
    if actual.operation() != row.operation
        || actual.funding() != &row.funding
        || actual.effect().version() != u32::from(row.format)
    {
        return Err(PublicationError::Mismatch);
    }
    let phase = match actual.terminal() {
        None => PurchasePhase::Funded {},
        Some(t) if t.granted() => PurchasePhase::Granted { commit: t.commit() },
        Some(t) => PurchasePhase::Refunded { commit: t.commit() },
    };
    if row.phase != phase {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

/// Shared exact first-product join. This checks retained birth/payment facts,
/// never today's issuer title, owner term, parent rights or market eligibility.
pub(super) fn verify_granted(
    row: &PaidAllocation,
    licence: &PersonalPaidLicence,
    term: &super::paid::PaidTermRecord,
    actual: &PurchaseHistory,
) -> Result<(), PublicationError> {
    verify_history(row, actual)?;
    if !matches!(row.phase, PurchasePhase::Granted { .. }) {
        return Err(PublicationError::Mismatch);
    }
    let expected = PersonalPaidLicence::original(
        row.licence,
        row.charter,
        row.line,
        row.holder,
        row.term,
        row.operation,
    )?;
    let issued = WorldIcSeconds::from_stamp(
        actual
            .terminal()
            .and_then(|t| t.issued_at_ic())
            .ok_or(PublicationError::Missing)?,
    )
    .map_err(|_| PublicationError::Invalid)?;
    if licence != &expected
        || term.licence() != row.licence
        || term.generation() != row.term
        || term.operation() != row.operation
        || term.policy()
            != (RevisionPolicy::Pin {
                filed: row.filed,
                revision: row.revision,
            })
        || term.terms() != row.terms
        || term.window().issued_at() != issued
        || term.window().eligibility_start() != issued
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}
