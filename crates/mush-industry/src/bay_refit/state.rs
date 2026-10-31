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
    /// The orders as this owner last acknowledged them. **An Install never passes through
    /// this owner** (`provider::register` gives action 4 no `BayOwner`: Install already binds
    /// Tactics and Economy, and a ledger batch holds at most two tails,
    /// `mush_core::ledger::BoundLedgerBatch`), so an installed order's row here keeps
    /// `terminal: None` and `installation: None` until the next boot rebuilds it from the store
    /// (`audit`). Never read a terminal from here: read Economy's `bay_refit_history(accepted)`
    /// or the store row's `terminal`, as the live paths do. A Cancel does pass through this
    /// owner, so a cancelled row here is current.
    pub(crate) orders: BTreeMap<WorkOrderId, Arc<BayOrder>>,
}
