//! Trusted Market publication adapter, installed only by the actual registrar.
//! Rows are equality inputs, never monetary or holder authority. The Roll
//! compound producer supplies the actual opaque native terminal and Root fence.
pub trait PaidResaleMarketWriter: Send + Sync + 'static {
    fn publish_terminal(
        &self,
        ctx: &mush_core::PrepareCtx<'_>,
        txn: &mut mush_core::ActionTxn<'_>,
        before: &super::PaidResaleRecord,
        after: &super::PaidResaleRecord,
    ) -> Result<(), String>;
}
