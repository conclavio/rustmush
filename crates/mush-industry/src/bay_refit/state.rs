//! The bay owner's acknowledged projection: the admission census, the
//! standing offers and the orders. Typed rows remain authoritative; this is
//! rebuilt from them and the census at every boot.
use super::records::{BayIndex, BayOffer, BayOrder};
use crate::workorder::WorkOrderId;
use mush_core::CommitToken;
use std::{collections::BTreeMap, sync::Arc};
#[derive(Clone, Default)]
pub(crate) struct BayRefitState {
    pub(crate) ready: bool,
    pub(crate) generation: u64,
    pub(crate) index: Option<BayIndex>,
    pub(crate) offers: BTreeMap<CommitToken, Arc<BayOffer>>,
    pub(crate) orders: BTreeMap<WorkOrderId, Arc<BayOrder>>,
}
