//! Complete native/typed/source-root inverse, including the all-missing case.
//! Actual Tacsim then joins publication/quote/host/policy before enabling cover.
use super::CoverageRetention;
use crate::{
    RollStore,
    commerce::{
        families::CommerceFamilies,
        records::{PublicationError, Record},
    },
};
use mush_core::{CommitToken, DefinitionDigest, ModuleCtx};
use mush_economy::EconomyStore;
use std::collections::BTreeSet;

fn bad(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}
pub(crate) fn all(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    enabled: bool,
) -> Result<Vec<CoverageRetention>, PublicationError> {
    let mut rows = Vec::new();
    super::super::audit::scan(ctx, &f.coverages, |row| {
        if !enabled || rows.len() >= 65536 {
            return Err(PublicationError::Unavailable);
        }
        rows.push(row.clone());
        Ok(())
    })?;
    let Some(economy) = ctx.state::<EconomyStore>() else {
        return if rows.is_empty() && !enabled {
            Ok(rows)
        } else {
            Err(PublicationError::Unavailable)
        };
    };
    if ctx
        .w
        .module_state_owner(std::any::TypeId::of::<EconomyStore>())
        .is_none_or(|v| v.as_str() != "economy")
    {
        return Err(PublicationError::Unavailable);
    }
    let native = economy.coverage_histories().map_err(bad)?;
    if native.len() != rows.len() {
        return Err(PublicationError::Mismatch);
    }
    let mut ids = BTreeSet::new();
    for row in &rows {
        row.validate()?;
        if !ids.insert(row.operation()) {
            return Err(PublicationError::Mismatch);
        }
        let history = economy.coverage_history(row.operation()).map_err(bad)?;
        row.matches_history(history)?;
        let claim =
            super::super::audit::read(ctx, &f.ic_claims, row.consent().basis.ic_open.to_hex())?;
        row.matches_claim(claim)?;
        roll.check_required_root(&crate::commerce::ic_claims::roots::required(claim)?)
            .map_err(bad)?;
        roll.check_required_root(&row.root()?).map_err(bad)?;
    }
    for history in native {
        if !ids.contains(&history.consent().operation) {
            return Err(PublicationError::Missing);
        }
    }
    Ok(rows)
}
pub(crate) fn root(
    ctx: &ModuleCtx<'_>,
    f: &CommerceFamilies,
    operation: CommitToken,
    digest: DefinitionDigest,
) -> Result<crate::commerce::RevisionIdentity, PublicationError> {
    let row = super::super::audit::read(ctx, &f.coverages, operation.to_hex())?;
    if row.consent().digest().map_err(bad)? != digest {
        return Err(PublicationError::Mismatch);
    }
    Ok(row.revision())
}
