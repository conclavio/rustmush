//! Mandatory three-owner reverse completeness, including an empty Market
//! projection. Historical evidence is independent of current issuer rights.
use super::families::Row;
use super::{Families, MindRenewalOperationV1, RenewalConfig};
use mush_core::{CommitToken, ModError, ModuleCtx, StateFamilyHandle};
use mush_economy::EconomyStore;
use std::collections::BTreeSet;
fn error(_: impl std::fmt::Display) -> ModError {
    ModError::msg("renewal paired recovery is incomplete or contradictory")
}
fn scan<R: Row>(
    ctx: &ModuleCtx<'_>,
    family: &StateFamilyHandle<CommitToken, R>,
    mut inspect: impl FnMut(&R) -> Result<(), ModError>,
) -> Result<usize, ModError> {
    let mut after = None;
    let mut count = 0usize;
    loop {
        let page = ctx
            .scan_action_state(family, after.as_deref(), 128)
            .map_err(error)?;
        for entry in page.entries() {
            count = count.checked_add(1).ok_or_else(|| error("count"))?;
            if count > 32768 {
                return Err(error("bound"));
            }
            let row = entry.value();
            row.validate().map_err(error)?;
            if entry.encoded_key() != row.key() {
                return Err(error("key"));
            }
            inspect(row)?;
        }
        let Some(next) = page.next_after() else {
            return Ok(count);
        };
        if page.entries().is_empty() || after.as_deref().is_some_and(|old| old >= next.as_str()) {
            return Err(error("nonprogress"));
        }
        after = Some(next.to_string());
    }
}
fn operation<'a>(
    ctx: &'a ModuleCtx<'_>,
    f: &Families,
    id: CommitToken,
) -> Result<&'a MindRenewalOperationV1, ModError> {
    let row = ctx
        .action_state(&f.operations, &id)
        .map_err(error)?
        .ok_or_else(|| error("missing"))?;
    row.validate().map_err(error)?;
    if row.operation() != id {
        return Err(error("key"));
    }
    Ok(row)
}
impl Families {
    pub(crate) fn audit(
        &self,
        ctx: &ModuleCtx<'_>,
        config: &RenewalConfig,
    ) -> Result<(), ModError> {
        let e = ctx
            .state::<EconomyStore>()
            .ok_or_else(|| error("economy"))?;
        let actual = e.renewal_audit().map_err(error)?;
        actual.require_complete().map_err(error)?;
        let mut seen = BTreeSet::new();
        let n = scan(ctx, &self.operations, |row| {
            if !seen.insert(row.operation()) {
                return Err(error("duplicate"));
            }
            let proof = actual.get(row.operation()).map_err(error)?;
            row.verify_history(&proof).map_err(error)?;
            let roll = config
                .writer
                .operation_for_boot(ctx, row.operation())
                .map_err(error)?;
            row.matches_roll(&roll).map_err(error)?;
            let receipt = ctx
                .action_state(&self.receipts, &row.operation())
                .map_err(error)?;
            if row.state().pending() {
                if receipt.is_some() {
                    return Err(error("premature receipt"));
                }
            } else {
                receipt
                    .ok_or_else(|| error("receipt"))?
                    .verify_history(row, &proof)
                    .map_err(error)?;
            }
            Ok(())
        })?;
        if n != actual.len() {
            return Err(error("economic count"));
        }
        for proof in actual.offers() {
            let proof = proof.map_err(error)?;
            operation(ctx, self, proof.operation())?
                .verify_history(&proof)
                .map_err(error)?;
        }
        let rows = config.writer.operations_for_boot(ctx).map_err(error)?;
        if rows.len() != n {
            return Err(error("roll count"));
        }
        for roll in rows {
            operation(ctx, self, roll.operation())?
                .matches_roll(&roll)
                .map_err(error)?;
        }
        scan(ctx, &self.receipts, |r| {
            let row = operation(ctx, self, r.operation())?;
            r.verify_history(row, &actual.get(row.operation()).map_err(error)?)
                .map_err(error)
        })?;
        Ok(())
    }
}
pub(crate) fn require_disabled(ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
    // Market may exist in a world with no economy module at all. If the owner
    // is present, only its completed boot inventory can establish absence.
    if let Some(e) = ctx.state::<EconomyStore>() {
        e.require_no_renewal_history().map_err(error)?;
    }
    Ok(())
}
