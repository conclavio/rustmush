//! Historical cause/status/root bijection. Today's policy never supplies past authority.
use super::super::{
    audit::{read, scan},
    families::CommerceFamilies,
    rating::families::RatingFamilies,
};
use super::*;
use mush_core::ModuleCtx;
pub(crate) fn status(
    ctx: &ModuleCtx<'_>,
    f: &CommerceFamilies,
    status: &super::super::records::RevisionStatus,
) -> Result<(), PublicationError> {
    if let Some(cause) = status.recalled_by {
        let old = ctx
            .action_state(&f.registry_recalls, &cause.to_hex())
            .map_err(bad)?;
        let measured = ctx
            .action_state(&f.measured_recalls, &cause.to_hex())
            .map_err(bad)?;
        let filed = read(ctx, &f.filed, status.filed.key())?;
        match (old, measured) {
            (Some(operation), None) => {
                if operation.status() != *status {
                    return Err(PublicationError::Mismatch);
                }
                let initial = super::super::line_update::audit::birth(ctx, f, filed)?;
                operation.matches(initial, filed)?;
            }
            (None, Some(operation)) => {
                operation.matches(filed)?;
                if operation.status() != *status {
                    return Err(PublicationError::Mismatch);
                }
            }
            _ => return Err(PublicationError::Mismatch),
        }
    }
    Ok(())
}
pub(crate) fn causes(
    ctx: &ModuleCtx<'_>,
    roll: &crate::RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
) -> Result<(), PublicationError> {
    scan(ctx, &f.registry_recalls, |op| {
        let filed = read(ctx, &f.filed, op.filed.key())?;
        let initial = super::super::line_update::audit::birth(ctx, f, filed)?;
        op.matches(initial, filed)?;
        if read(ctx, &f.status, op.filed.key())? != &op.status() {
            return Err(PublicationError::Mismatch);
        }
        let rating = ratings.resolve_receipt(ctx, &op.rating).map_err(bad)?;
        if rating.subject().revision() != op.revision
            || rating.subject().definition() != filed.definition
        {
            return Err(PublicationError::Mismatch);
        }
        roll.check_required_root(&op.root()?).map_err(bad)
    })?;
    scan(ctx, &f.measured_recalls, |op| {
        if ctx
            .action_state(&f.registry_recalls, &op.key())
            .map_err(bad)?
            .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        super::super::rating::reconformance::recall::history::verify(ctx, f, ratings, op)
    })?;
    Ok(())
}

/// Exactly one closed cause family owns a recall root. No fallback on malformed evidence.
pub(crate) fn cause_root(
    ctx: &impl super::super::read::CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    cause: CommitToken,
) -> Result<RequiredRoot, PublicationError> {
    use super::super::entitlement::history::read;
    let old = ctx.get(&f.registry_recalls, &cause.to_hex()).map_err(bad)?;
    let measured = ctx.get(&f.measured_recalls, &cause.to_hex()).map_err(bad)?;
    match (old, measured) {
        (Some(op), None) => {
            let filed = read(ctx, &f.filed, op.filed.key())?;
            let initial = super::super::line_update::audit::birth(ctx, f, filed)?;
            op.matches(initial, filed)?;
            if read(ctx, &f.status, op.filed.key())? != &op.status() {
                return Err(PublicationError::Mismatch);
            }
            let rating = ratings.resolve_receipt_read(ctx, &op.rating).map_err(bad)?;
            if rating.subject().revision() != op.revision
                || rating.subject().definition() != filed.definition
            {
                return Err(PublicationError::Mismatch);
            }
            op.root()
        }
        (None, Some(op)) => {
            super::super::rating::reconformance::recall::history::verify(ctx, f, ratings, op)?;
            op.root()
        }
        _ => Err(PublicationError::Mismatch),
    }
}
