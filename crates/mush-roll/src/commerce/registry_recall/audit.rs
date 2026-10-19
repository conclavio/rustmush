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
        let operation = read(ctx, &f.registry_recalls, cause.to_hex())?;
        if operation.status() != *status {
            return Err(PublicationError::Mismatch);
        }
        let filed = read(ctx, &f.filed, status.filed.key())?;
        let initial = super::super::line_update::audit::birth(ctx, f, filed)?;
        operation.matches(initial, filed)?;
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
    Ok(())
}
