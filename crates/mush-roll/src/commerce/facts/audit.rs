//! Mandatory historical occurrence/root joins. Never a fresh-rights check.
use super::super::{families::CommerceFamilies, rating::RatingFamilies};
use super::*;
use crate::RollStore;
use mush_core::ModuleCtx;
use std::collections::BTreeSet;

pub(crate) fn causes(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
) -> Result<(), PublicationError> {
    let mut operations = BTreeSet::new();
    // Always visit the family, including an empty inventory. Reverse roots are
    // checked by the common commercial-root audit, independently of opt-in.
    super::super::audit::scan(ctx, &f.lapse_occurrences, |row| {
        if !operations.insert(row.operation) {
            return Err(PublicationError::Mismatch);
        }
        let proof =
            history::historical(ctx, roll, f, ratings, row.term.licence, row.term.generation)?;
        proof.require_historical_lapse(row)?;
        roll.check_required_root(&row.root()?).map_err(invalid)
    })?;
    Ok(())
}

pub(crate) fn root_subject(
    ctx: &ModuleCtx<'_>,
    f: &CommerceFamilies,
    licence: super::super::LicenceId,
    generation: super::super::TermGeneration,
    operation: mush_core::CommitToken,
    digest: mush_core::DefinitionDigest,
) -> Result<super::super::RevisionIdentity, PublicationError> {
    let key = format!("{}/{}", licence.key(), generation.key());
    let row = super::super::audit::read(ctx, &f.lapse_occurrences, key)?;
    if row.operation != operation || row.digest()? != digest {
        return Err(PublicationError::Mismatch);
    }
    Ok(row.term.revision)
}
