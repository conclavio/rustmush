//! Complete forward/reverse cause/status/root proof, independent of current
//! authority and the mutable current term pointer.
use super::super::{
    audit::{read, scan},
    families::CommerceFamilies,
    rating::RatingFamilies,
};
use super::*;
use mush_core::ModuleCtx;
pub(crate) fn causes(
    ctx: &ModuleCtx<'_>,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
) -> Result<(), PublicationError> {
    scan(ctx, &f.registry_revocations, |op| {
        history::verify(ctx, f, ratings, op)?;
        read(ctx, &f.revoked_terms, op.term.status_key())?.matches(op)
    })?;
    scan(ctx, &f.revoked_terms, |status| {
        let op = read(ctx, &f.registry_revocations, status.operation.to_hex())?;
        status.matches(op)
    })?;
    Ok(())
}
