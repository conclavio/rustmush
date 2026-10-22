//! Full Market/Roll/native inverse, including unaccepted retained seller offers.
use super::{
    PaidResaleConfig,
    families::{Families, Row},
    records::*,
};
use mush_core::{CommitToken, ModError, ModuleCtx, StateFamilyHandle};
use mush_economy::EconomyStore;
use mush_roll::commerce::paid_resale::ResaleRates;
use std::collections::BTreeSet;
fn error(_: impl std::fmt::Display) -> ModError {
    ModError::msg("paid resale paired history is incomplete or contradictory")
}
fn scan<R: Row>(
    ctx: &ModuleCtx<'_>,
    f: &StateFamilyHandle<CommitToken, R>,
    mut inspect: impl FnMut(&R) -> Result<(), ModError>,
) -> Result<usize, ModError> {
    let mut after = None;
    let mut n = 0usize;
    loop {
        let page = ctx
            .scan_action_state(f, after.as_deref(), 128)
            .map_err(error)?;
        for entry in page.entries() {
            n = n.checked_add(1).ok_or_else(|| error("count"))?;
            if n > 65536 {
                return Err(error("bound"));
            }
            let row = entry.value();
            row.validate().map_err(error)?;
            if entry.encoded_key() != row.key().to_hex() {
                return Err(error("key"));
            }
            inspect(row)?;
        }
        let Some(next) = page.next_after() else {
            return Ok(n);
        };
        if page.entries().is_empty() || after.as_deref().is_some_and(|old| old >= next.as_str()) {
            return Err(error("progress"));
        }
        after = Some(next.to_string());
    }
}
fn read<'a, R: Row>(
    ctx: &'a ModuleCtx<'_>,
    f: &StateFamilyHandle<CommitToken, R>,
    id: CommitToken,
) -> Result<&'a R, ModError> {
    let row = ctx
        .action_state(f, &id)
        .map_err(error)?
        .ok_or_else(|| error("missing"))?;
    row.validate().map_err(error)?;
    if row.key() != id {
        return Err(error("key"));
    }
    Ok(row)
}
impl Families {
    pub(crate) fn audit(
        &self,
        ctx: &ModuleCtx<'_>,
        config: &PaidResaleConfig,
        legacy: Option<&crate::mind_trade::Families>,
    ) -> Result<(), ModError> {
        let e = ctx
            .state::<EconomyStore>()
            .ok_or_else(|| error("economy"))?;
        let mut native = BTreeSet::new();
        for h in e.paid_resale_histories().map_err(error)? {
            if native.len() >= 65536 || !native.insert(h.consent().operation) {
                return Err(error("native"));
            }
        }
        let mut offers = BTreeSet::new();
        scan(ctx, &self.offers, |offer| {
            if !offers.insert(offer.operation()) {
                return Err(error("offer"));
            }
            let proof = config
                .writer
                .binding_for_boot(ctx, offer.binding())
                .map_err(error)?;
            if proof.seller() != offer.terms().seller
                || proof.economics().instrument() != &offer.terms().instrument
            {
                return Err(error("offer binding"));
            }
            let (registry, venue, parents) = match proof.economics().rates() {
                ResaleRates::Legacy {
                    quote,
                    quote_digest,
                } => {
                    let quote = legacy
                        .ok_or_else(|| error("legacy family"))?
                        .resale_quote_for_boot(ctx, *quote)
                        .map_err(error)?;
                    if quote.digest().map_err(error)? != *quote_digest {
                        return Err(error("quote digest"));
                    }
                    let (instrument, registry, venue) = quote.resale_rates().map_err(error)?;
                    if instrument != offer.terms().instrument {
                        return Err(error("instrument"));
                    }
                    (registry, venue, vec![])
                }
                ResaleRates::Accepted {
                    registry,
                    venue,
                    parents,
                } => (registry.clone(), venue.clone(), parents.clone()),
            };
            if registry != offer.terms().registry
                || venue != offer.terms().venue
                || parents.len() != offer.terms().parents.len()
                || parents
                    .iter()
                    .zip(&offer.terms().parents)
                    .any(|(a, b)| a.charter() != b.charter || a.permille() != b.permille)
            {
                return Err(error("accepted cuts"));
            }
            // Captured positive title/account identity is historical consent;
            // no current title, lifecycle, source or issuer gate belongs here.
            Ok(())
        })?;
        let mut used_offers = BTreeSet::new();
        let mut receipt_keys = BTreeSet::new();
        let mut terminal_commits = BTreeSet::new();
        let n = scan(ctx, &self.operations, |op| {
            if !native.remove(&op.operation()) {
                return Err(error("native pair"));
            }
            let roll = config
                .writer
                .operation_for_boot(ctx, op.operation())
                .map_err(error)?;
            op.verify_history(&roll, e.paid_resale_history(op.operation()).map_err(error)?)
                .map_err(error)?;
            let offer = read(ctx, &self.offers, roll.offer())?;
            offer.matches_roll(&roll).map_err(error)?;
            if !used_offers.insert(offer.operation()) {
                return Err(error("one use"));
            }
            if let Some(terminal) = roll.state().terminal() {
                if !terminal_commits.insert(terminal) || !receipt_keys.insert(op.operation()) {
                    return Err(error("terminal alias"));
                }
                read(ctx, &self.receipts, op.operation())?
                    .matches_roll(&roll)
                    .map_err(error)?;
            }
            Ok(())
        })?;
        if !native.is_empty() {
            return Err(error("omitted native"));
        }
        let rows = config.writer.operations_for_boot(ctx).map_err(error)?;
        if rows.len() != n {
            return Err(error("omitted Roll"));
        }
        for id in rows {
            let row = config.writer.operation_for_boot(ctx, id).map_err(error)?;
            read(ctx, &self.operations, id)?
                .matches_roll(&row)
                .map_err(error)?;
        }
        scan(ctx, &self.receipts, |receipt| {
            if !receipt_keys.remove(&receipt.key()) {
                return Err(error("orphan receipt"));
            }
            let op = read(ctx, &self.operations, receipt.acceptance())?;
            let roll = config
                .writer
                .operation_for_boot(ctx, op.operation())
                .map_err(error)?;
            receipt.matches_roll(&roll).map_err(error)
        })?;
        if !receipt_keys.is_empty() {
            return Err(error("omitted receipt"));
        }
        Ok(())
    }
}
pub(crate) fn require_disabled(ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
    if let Some(e) = ctx.state::<EconomyStore>() {
        if e.paid_resale_histories().map_err(error)?.len() != 0 {
            return Err(error("disabled native"));
        }
    }
    Ok(())
}
