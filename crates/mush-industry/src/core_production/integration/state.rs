//! Derived owner projection plus independent immutable admission census.
use super::{offer::FactoryOffer, progress::CoreProgress, records::*};
use mush_core::{CommitToken, DefinitionRef};
use mush_economy::core_integration::CoreTerminalReceipt;
use std::{collections::BTreeMap, sync::Arc};
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OrderView {
    pub(crate) intent: AcceptedIntent,
    pub(crate) accepted: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) progress: CoreProgress,
    pub(crate) terminal: Option<CoreTerminalReceipt>,
}
impl OrderView {
    pub(crate) fn capture(order: &CoreIntegrationOrder) -> Self {
        Self {
            intent: order.intent.clone(),
            accepted: order.accepted,
            action: order.action.clone(),
            progress: order.progress.clone(),
            terminal: order.terminal.clone(),
        }
    }
}
#[derive(Clone, Default)]
pub(crate) struct Projection {
    pub(crate) ready: bool,
    pub(crate) index: Option<IntegrationIndex>,
    pub(crate) offers: BTreeMap<CommitToken, Arc<FactoryOffer>>,
    pub(crate) orders: BTreeMap<crate::workorder::WorkOrderId, Arc<OrderView>>,
}
