//! Full retained Market/Roll/E4 joins. No fresh-sale eligibility or expiry
//! predicate is used to decide whether immutable past payment exists.
use super::Families;
use super::purchase_provider::{OriginalPurchaseConfig, PurchaseFamilies, economics, terms};
use super::purchase_records::Purchase;
use super::records::Row;
use mush_core::{CommitToken, ModError, ModuleCtx, StateFamilyHandle};
use mush_economy::EconomyStore;
use mush_roll::commerce::purchase::PurchasePhase;
use std::collections::BTreeSet;

fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(format!("paid purchase recovery: {e}"))
}
fn scan<R: Row>(
    ctx: &ModuleCtx<'_>,
    family: &StateFamilyHandle<CommitToken, R>,
    mut inspect: impl FnMut(&str, &R) -> Result<(), ModError>,
) -> Result<(), ModError> {
    let mut after = None;
    let mut count = 0_usize;
    loop {
        let page = ctx
            .scan_action_state(family, after.as_deref(), 128)
            .map_err(error)?;
        for entry in page.entries() {
            count = count
                .checked_add(1)
                .ok_or_else(|| error("record count overflow"))?;
            if count > 32768 {
                return Err(error("retained purchase bound exceeded"));
            }
            entry.value().validate().map_err(error)?;
            inspect(entry.encoded_key(), entry.value())?;
        }
        let Some(next) = page.next_after() else {
            return Ok(());
        };
        if page.entries().is_empty() || after.as_deref().is_some_and(|old| old >= next.as_str()) {
            return Err(error("nonprogressing recovery page"));
        }
        after = Some(next.to_string());
    }
}
fn operation<'a>(
    ctx: &'a ModuleCtx<'_>,
    families: &PurchaseFamilies,
    token: CommitToken,
) -> Result<&'a Purchase, ModError> {
    let row = ctx
        .action_state(&families.operations, &token)
        .map_err(error)?
        .ok_or_else(|| error("operation is missing"))?;
    row.validate().map_err(error)?;
    if row.operation != token {
        return Err(error("operation key differs"));
    }
    Ok(row)
}
impl PurchaseFamilies {
    pub(super) fn checked_operation(
        &self,
        ctx: &ModuleCtx<'_>,
        config: &OriginalPurchaseConfig,
        offers: &Families,
        token: CommitToken,
    ) -> Result<Purchase, ModError> {
        let row = operation(ctx, self, token)?;
        let economy = ctx
            .state::<EconomyStore>()
            .ok_or_else(|| error("economy is unavailable"))?;
        if economy.is_frozen() {
            return Err(error("economy is frozen"));
        }
        let audit = economy.purchase_receipt_audit();
        audit.require_complete().map_err(error)?;
        let proof = audit.get(row.operation).map_err(error)?;
        if proof.effect() != &row.effect
            || !proof.matches_purchase(&economics(&row.quote).map_err(error)?, &row.funding)
        {
            return Err(error("quote/economics differ from actual funding"));
        }
        let expected = match proof.terminal() {
            None => PurchasePhase::Funded {},
            Some(t) if t.granted() => PurchasePhase::Granted { commit: t.commit() },
            Some(t) => PurchasePhase::Refunded { commit: t.commit() },
        };
        if row.phase != expected {
            return Err(error("typed phase differs from journal"));
        }
        let allocation = config
            .writer
            .audit_allocation_for_boot(
                ctx,
                row.operation,
                terms(&row.quote),
                proof.terminal().and_then(|t| t.issued_at_ic()),
            )
            .map_err(error)?;
        row.matches_allocation(&allocation).map_err(error)?;
        if ctx
            .action_state(&offers.quotes, &row.quote.publication)
            .map_err(error)?
            .is_some()
        {
            return Err(error("funding did not consume acknowledged quote"));
        }
        let receipt = ctx
            .action_state(&self.receipts, &row.operation)
            .map_err(error)?;
        if proof.terminal().is_some() {
            receipt
                .ok_or_else(|| error("terminal receipt is missing"))?
                .verify_history(row, proof)
                .map_err(error)?;
        } else if receipt.is_some() {
            return Err(error("unfunded terminal receipt"));
        }
        Ok(row.clone())
    }

    pub(crate) fn audit_with_entitlements(
        &self,
        ctx: &ModuleCtx<'_>,
        config: &OriginalPurchaseConfig,
        offers: &Families,
        entitlements_enabled: bool,
    ) -> Result<(), ModError> {
        self.audit_with_houses(ctx, config, offers, entitlements_enabled, false)
    }
    pub(crate) fn audit_with_houses(
        &self,
        ctx: &ModuleCtx<'_>,
        config: &OriginalPurchaseConfig,
        offers: &Families,
        entitlements_enabled: bool,
        houses_enabled: bool,
    ) -> Result<(), ModError> {
        let economy = ctx
            .state::<EconomyStore>()
            .ok_or_else(|| error("economy is unavailable"))?;
        if economy.is_frozen() {
            return Err(error("economy is frozen"));
        }
        let audit = economy.purchase_receipt_audit();
        audit.require_complete().map_err(error)?;
        let mut quotes = BTreeSet::new();
        scan(ctx, &self.operations, |key, row| {
            if key != row.operation.to_hex() || !quotes.insert(row.quote.publication) {
                return Err(error("crossed operation or consumed quote"));
            }
            self.checked_operation(ctx, config, offers, row.operation)?;
            Ok(())
        })?;
        // Independent E4 history is mandatory even when every Market row is
        // missing or an old typed counter/map was restored as a plausible empty.
        for proof in audit.receipts().map_err(error)? {
            if proof.house().map_err(error)?.is_some() {
                if !houses_enabled {
                    return Err(error("house economic history is not configured"));
                }
                continue;
            }
            if proof.entitlement().map_err(error)?.is_some() {
                if !entitlements_enabled {
                    return Err(error("entitlement history is not configured"));
                }
                continue;
            }
            if proof.original().map_err(error)?.is_none() {
                return Err(error("unknown economic purchase history"));
            }

            let row = operation(ctx, self, proof.operation())?;
            if proof.effect() != &row.effect
                || !proof.matches_purchase(&economics(&row.quote).map_err(error)?, &row.funding)
            {
                return Err(error("unaccounted economic purchase"));
            }
        }
        scan(ctx, &self.receipts, |key, receipt| {
            if key != receipt.operation().to_hex() {
                return Err(error("receipt key differs"));
            }
            let row = operation(ctx, self, receipt.operation())?;
            receipt
                .verify_history(row, audit.get(row.operation).map_err(error)?)
                .map_err(error)
        })?;
        Ok(())
    }
}
