use super::{
    families::{Families, envelope},
    records::{MindPaidResaleOperation, MindPaidResaleReceipt},
};
use mush_roll::commerce::paid_resale::{PaidResaleMarketWriter, PaidResaleRecord};
pub(super) struct Writer(pub(super) Families);
impl PaidResaleMarketWriter for Writer {
    fn publish_terminal(
        &self,
        ctx: &mush_core::PrepareCtx<'_>,
        txn: &mut mush_core::ActionTxn<'_>,
        before: &PaidResaleRecord,
        after: &PaidResaleRecord,
    ) -> Result<(), String> {
        if before.operation() != after.operation()
            || before.binding() != after.binding()
            || after.state().terminal() != Some(ctx.commit_token())
        {
            return Err("compound resale affinity differs".into());
        }
        let operation = before.operation();
        let old = txn
            .get(&self.0.operations, &operation)
            .map_err(|e| e.to_string())?
            .cloned()
            .ok_or("compound Market operation missing")?;
        old.matches_roll(before)?;
        let offer = txn
            .get(&self.0.offers, &before.offer())
            .map_err(|e| e.to_string())?
            .ok_or("compound Market offer missing")?;
        offer.matches_roll(before)?;
        offer.matches_roll(after)?;
        let next = MindPaidResaleOperation::from_roll(after)?;
        let receipt = MindPaidResaleReceipt::from_roll(after)?;
        envelope(
            &self.0.operations,
            operation,
            serde_json::to_value(&next).map_err(|e| e.to_string())?,
        )?;
        envelope(
            &self.0.receipts,
            receipt.acceptance(),
            serde_json::to_value(&receipt).map_err(|e| e.to_string())?,
        )?;
        txn.replace(&self.0.operations, &operation, next.clone())
            .map_err(|e| e.to_string())?;
        txn.insert(&self.0.receipts, &receipt.acceptance(), receipt.clone())
            .map_err(|e| e.to_string())?;
        if txn
            .get(&self.0.operations, &operation)
            .map_err(|e| e.to_string())?
            != Some(&next)
            || txn
                .get(&self.0.receipts, &receipt.acceptance())
                .map_err(|e| e.to_string())?
                != Some(&receipt)
        {
            return Err("compound Market readback differs".into());
        }
        Ok(())
    }
}
