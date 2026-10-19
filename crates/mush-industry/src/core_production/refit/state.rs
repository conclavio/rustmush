//! Derived acknowledged offer/work projection. Typed and native terminal rows
//! remain authoritative after Install, which has no third owner image.
use super::records::*;
use crate::workorder::WorkOrderId;
use mush_core::CommitToken;
use std::{collections::BTreeMap, sync::Arc};
#[derive(Clone, Default)]
pub(crate) struct Projection {
    pub(crate) ready: bool,
    pub(crate) index: Option<RefitIndex>,
    pub(crate) offers: BTreeMap<CommitToken, Arc<RefitOffer>>,
    pub(crate) orders: BTreeMap<WorkOrderId, Arc<RefitOrder>>,
}
