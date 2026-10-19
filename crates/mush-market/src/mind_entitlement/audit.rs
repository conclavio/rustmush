//! Complete retained Market/Roll/E4 joins. Historical money and immutable
//! royalty consent remain valid independently of current seller eligibility.
use super::codec::Row;
use super::{EntitlementConfig, EntitlementListingV1, EntitlementPurchaseV1, Families};
use mush_core::{CommitToken, ModError, ModuleCtx, StateFamilyHandle};
use mush_economy::EconomyStore;
use mush_roll::commerce::entitlement::RoyaltyBasisV1;
use mush_roll::commerce::purchase::PurchasePhase;
use std::collections::BTreeSet;
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(format!("entitlement recovery: {e}"))
}
fn scan<R: Row + Clone + Send + Sync + 'static>(
    ctx: &ModuleCtx<'_>,
    family: &StateFamilyHandle<CommitToken, R>,
    mut inspect: impl FnMut(&R) -> Result<(), ModError>,
) -> Result<u64, ModError> {
    let mut after = None;
    let mut count = 0_u64;
    loop {
        let page = ctx
            .scan_action_state(family, after.as_deref(), 128)
            .map_err(error)?;
        for entry in page.entries() {
            count = count
                .checked_add(1)
                .ok_or_else(|| error("record count overflow"))?;
            if count > super::families::MAX_RETAINED {
                return Err(error("retained entitlement bound exceeded"));
            }
            let row = entry.value();
            row.validate().map_err(error)?;
            if entry.encoded_key() != row.key() {
                return Err(error("entitlement key differs"));
            }
            inspect(row)?;
        }
        let Some(next) = page.next_after() else {
            return Ok(count);
        };
        if page.entries().is_empty() || after.as_deref().is_some_and(|old| old >= next.as_str()) {
            return Err(error("nonprogressing entitlement page"));
        }
        after = Some(next.to_string());
    }
}
fn operation<'a>(
    ctx: &'a ModuleCtx<'_>,
    f: &Families,
    id: CommitToken,
) -> Result<&'a EntitlementPurchaseV1, ModError> {
    let p = ctx
        .action_state(&f.operations, &id)
        .map_err(error)?
        .ok_or_else(|| error("entitlement operation missing"))?;
    p.validate().map_err(error)?;
    if p.operation != id {
        return Err(error("entitlement operation key differs"));
    }
    Ok(p)
}
fn listing<'a>(
    ctx: &'a ModuleCtx<'_>,
    f: &Families,
    id: CommitToken,
) -> Result<&'a EntitlementListingV1, ModError> {
    let row = ctx
        .action_state(&f.listings, &id)
        .map_err(error)?
        .ok_or_else(|| error("entitlement listing missing"))?;
    row.validate().map_err(error)?;
    if row.publication != id {
        return Err(error("entitlement listing key differs"));
    }
    Ok(row)
}
fn quote_listing(q: &super::EntitlementQuoteV1, l: &EntitlementListingV1) -> Result<(), ModError> {
    q.validate().map_err(error)?;
    if q.listing != l.publication
        || q.listing_digest != l.digest().map_err(error)?
        || q.subject != l.subject
        || q.product != l.product
        || q.royalty != l.royalty
        || q.price != l.price
        || q.instrument != l.instrument
        || q.registry != l.registry
        || q.venue != l.venue
        || q.policy != l.policy
    {
        return Err(error("entitlement quote differs from immutable listing"));
    }
    Ok(())
}
impl Families {
    pub(crate) fn audit(
        &self,
        ctx: &ModuleCtx<'_>,
        config: &EntitlementConfig,
        original_enabled: bool,
    ) -> Result<(), ModError> {
        let e = ctx
            .state::<EconomyStore>()
            .ok_or_else(|| error("economy unavailable"))?;
        if e.is_frozen() {
            return Err(error("economy frozen"));
        }
        let audit = e.purchase_receipt_audit();
        audit.require_complete().map_err(error)?;
        let listings = scan(ctx, &self.listings, |l| {
            config
                .writer
                .audit_royalty_for_boot(ctx, &l.royalty)
                .map_err(error)
        })?;
        let quotes = scan(ctx, &self.quotes, |q| {
            quote_listing(q, listing(ctx, self, q.listing)?)
        })?;
        let mut consumed = BTreeSet::new();
        let operations = scan(ctx, &self.operations, |p| {
            if !consumed.insert(p.quote.publication) {
                return Err(error("quote funded twice"));
            }
            quote_listing(&p.quote, listing(ctx, self, p.quote.listing)?)?;
            if ctx
                .action_state(&self.quotes, &p.quote.publication)
                .map_err(error)?
                .is_some()
            {
                return Err(error("funded quote was not consumed"));
            }
            let actual = audit.get(p.operation).map_err(error)?;
            p.verify_history(actual).map_err(error)?;
            let allocation = config
                .writer
                .audit_allocation_for_boot(
                    ctx,
                    p.operation,
                    &super::publication::terms(&p.quote),
                    actual.terminal().and_then(|t| t.issued_at_ic()),
                )
                .map_err(error)?;
            p.matches_allocation(&allocation).map_err(error)?;
            let receipt = ctx
                .action_state(&self.receipts, &p.operation)
                .map_err(error)?;
            match p.phase {
                PurchasePhase::Funded {} => {
                    if receipt.is_some() {
                        return Err(error("funded entitlement carries terminal receipt"));
                    }
                }
                PurchasePhase::Granted { .. } | PurchasePhase::Refunded { .. } => receipt
                    .ok_or_else(|| error("terminal receipt missing"))?
                    .verify_history(p, actual)
                    .map_err(error)?,
            }
            Ok(())
        })?;
        // Reverse history is mandatory even if all new Market families are empty.
        for actual in audit.receipts().map_err(error)? {
            if actual.entitlement().map_err(error)?.is_some() {
                operation(ctx, self, actual.operation())?
                    .verify_history(actual)
                    .map_err(error)?;
            } else if !original_enabled || actual.original().map_err(error)?.is_none() {
                return Err(error("economic history has no configured purchase owner"));
            }
        }
        scan(ctx, &self.receipts, |receipt| {
            let p = operation(ctx, self, receipt.operation())?;
            receipt
                .verify_history(p, audit.get(p.operation).map_err(error)?)
                .map_err(error)
        })?;
        self.check_counts(ctx, listings, quotes, operations)
            .map_err(error)?;
        // A first derived listing may also materialize its historical original
        // parent's absent empty chain. That is the only secondary publication.
        for chain in config
            .writer
            .royalty_publications_for_boot(ctx)
            .map_err(error)?
        {
            let cause = listing(ctx, self, chain.publication())?;
            if chain == cause.royalty {
                continue;
            }
            let parent = cause
                .royalty
                .edges()
                .first()
                .ok_or_else(|| error("orphan royalty publication"))?;
            if !chain.edges().is_empty()
                || chain.filed() != parent.parent().filed
                || chain.revision() != parent.parent().revision
                || parent.royalty_permille() != 0
                || !matches!(
                    parent.basis(),
                    RoyaltyBasisV1::LegacySourceFormsV2Zero { .. }
                )
            {
                return Err(error("royalty publication has no exact listing cause"));
            }
        }
        Ok(())
    }
}
