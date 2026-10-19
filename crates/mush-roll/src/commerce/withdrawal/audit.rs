//! Historical cause/status/retention bijection, independent of current rights.
use super::super::{
    audit::{read, scan},
    families::CommerceFamilies,
};
use super::*;
use mush_core::ModuleCtx;

pub(crate) fn status(
    ctx: &ModuleCtx<'_>,
    families: &CommerceFamilies,
    status: &super::super::records::RevisionStatus,
) -> Result<(), PublicationError> {
    if status.recalled_by.is_some() {
        return Err(PublicationError::Mismatch);
    }
    if let Some(cause) = status.withdrawn_by {
        let operation = read(ctx, &families.withdrawals, cause.to_hex())?;
        if operation.status() != *status {
            return Err(PublicationError::Mismatch);
        }
        let filed = read(ctx, &families.filed, status.filed.key())?;
        let initial = read(ctx, &families.operations, filed.operation.to_hex())?;
        operation.matches_initial(initial, filed)?;
    }
    Ok(())
}
pub(crate) fn causes(
    ctx: &ModuleCtx<'_>,
    roll: &crate::RollStore,
    families: &CommerceFamilies,
) -> Result<(), PublicationError> {
    scan(ctx, &families.withdrawals, |operation| {
        let filed = read(ctx, &families.filed, operation.filed.key())?;
        let initial = read(ctx, &families.operations, operation.initial_filing.to_hex())?;
        operation.matches_initial(initial, filed)?;
        if read(ctx, &families.status, operation.filed.key())? != &operation.status() {
            return Err(PublicationError::Mismatch);
        }
        roll.check_required_root(&operation.root()?)
            .map_err(|_| PublicationError::Mismatch)
    })?;
    Ok(())
}
