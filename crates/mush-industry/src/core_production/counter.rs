//! **What a fitting shop's counter reads** (doc 47 CO.6): where a requester's integrations and
//! refits stand, and the dies an agent issued that no offer names yet.
//!
//! A counter (the game's `+core`, `crates/tacsim/src/hardware/counter.rs`) acts for the shop's
//! stock agent and for the requester, one typed action at a time, and learns what each act did
//! only from durable state: an action's result is never handed back to whoever submitted it. The
//! offers, orders and deliveries are this owner's acknowledged projection; the dies are its
//! this-boot record (the owner state's `issued`). This is a read and
//! nothing else: it grants no authority, and every act it informs is checked in full by its
//! provider when it is submitted.
use super::DieClass;
use super::participant::CoreProductionState;
use crate::workorder::WorkOrderId;
use mush_core::{CommitToken, ObjId, World};
use mush_inventory::LotId;

/// How many issued dies the this-boot record keeps.
pub const MAX_ISSUED: usize = 256;

/// **One integration offered to the requester**: its offer, its class, who offered it, when it
/// lapses, and its work order once it is accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntegrationLine {
    /// The offer's operation, which an acceptance names.
    pub offer: CommitToken,
    /// The class the offer integrates.
    pub class: DieClass,
    /// The agent who offered it.
    pub operator: ObjId,
    /// The die lot the offer names, for a supplier-stock offer.
    pub lot: Option<LotId>,
    /// The offer lapses at this UTC second.
    pub expires_utc: i64,
    /// The order its acceptance opened, if any.
    pub order: Option<IntegrationOrder>,
}

/// **An accepted integration's order**, as far as it has gone. Whether its output is reserved
/// is not here: the reservation is the kernel's, and this owner's projection does not carry it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntegrationOrder {
    /// The order, which reserve, complete and deliver name.
    pub id: WorkOrderId,
    /// The acceptance's commit: the core's manufacture, which a refit's offer names.
    pub accepted: CommitToken,
    /// Every scheduled hour of work has run.
    pub worked: bool,
    /// The completion's titled output, once completed; `None` before, or when cancelled.
    pub completed: Option<ObjId>,
    /// Whether it was cancelled.
    pub cancelled: bool,
    /// The output was delivered into the requester's hands.
    pub delivered: bool,
}

/// **One refit offered to the requester**: the hull, the incoming core, who offered it, when it
/// lapses, and its order once accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefitLine {
    /// The offer's operation, which an acceptance names.
    pub offer: CommitToken,
    /// The hull it fits.
    pub hull: ObjId,
    /// The core it fits.
    pub incoming: ObjId,
    /// The agent who offered it.
    pub operator: ObjId,
    /// The offer lapses at this UTC second.
    pub expires_utc: i64,
    /// The order its acceptance opened, if any: its id, and whether every hour has run.
    pub order: Option<(WorkOrderId, bool)>,
}

/// **The counter's read for one requester.**
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CounterView {
    /// The integrations offered to the requester, oldest offer first.
    pub integrations: Vec<IntegrationLine>,
    /// The refits offered to the requester, oldest offer first.
    pub refits: Vec<RefitLine>,
}

/// **Where `requester`'s integrations and refits stand**, from this owner's acknowledged
/// projection. `None` while the owner is not composed or not ready.
#[must_use]
pub fn view(world: &World, requester: ObjId) -> Option<CounterView> {
    let state = world.module_state::<CoreProductionState>()?;
    if !state.ready {
        return None;
    }
    let mut out = CounterView::default();
    if let Some(p) = state.integration.as_ref().filter(|p| p.ready) {
        for offer in p.offers.values().filter(|o| o.requester == requester) {
            let lot = match &offer.input {
                super::integration::offer::OfferedInput::SupplierStock { lot, .. } => {
                    Some(lot.id())
                }
                _ => None,
            };
            let order = p
                .orders
                .values()
                .find(|o| o.intent.offer == offer.operation)
                .map(|o| {
                    let (completed, cancelled) = match o.terminal.as_ref().map(|t| t.result()) {
                        Some(mush_economy::core_integration::CoreTerminalResult::Completed {
                            output,
                            ..
                        }) => (Some(output.object), false),
                        Some(mush_economy::core_integration::CoreTerminalResult::Cancelled {}) => {
                            (None, true)
                        }
                        None => (None, false),
                    };
                    IntegrationOrder {
                        id: o.intent.order,
                        accepted: o.accepted,
                        worked: o.progress.production().is_complete(),
                        completed,
                        cancelled,
                        delivered: p.deliveries.contains_key(&o.intent.order),
                    }
                });
            out.integrations.push(IntegrationLine {
                offer: offer.operation,
                class: offer.recipe.class(),
                operator: offer.operator,
                lot,
                expires_utc: offer.expires_utc,
                order,
            });
        }
    }
    if let Some(p) = state.refit.as_ref().filter(|p| p.ready) {
        for offer in p.offers.values().filter(|o| o.requester == requester) {
            let order = p
                .orders
                .values()
                .find(|o| o.intent.offer.operation == offer.operation)
                .map(|o| (o.intent.order, o.progress.production().is_complete()));
            out.refits.push(RefitLine {
                offer: offer.operation,
                hull: offer.hull,
                incoming: offer.incoming,
                operator: offer.operator,
                expires_utc: offer.expires_utc,
                order,
            });
        }
    }
    Some(out)
}

/// **Whether the certified origin is initialized** (doc 48e, services G28): its `Initialize`
/// credited the die depot's opening stock in its own bundle, once, so from then on the matter
/// census counts that stock as a source. Read from this owner's acknowledged index.
#[must_use]
pub fn origin_initialized(world: &World) -> bool {
    world
        .module_state::<CoreProductionState>()
        .is_some_and(|state| state.index.is_some())
}

/// **The die lots a live integration offer names** (doc 48e, production G14): an offer still open
/// (unaccepted and not lapsed at `world.now()`), or one accepted, whose order took its die. A
/// lapsed offer's die was never taken and is free to offer again; before this, any offer that
/// ever named a die withheld it for good, so an offer that lapsed (a buyer whose money was held)
/// stranded the die and the next order cut another from the finite depot.
#[must_use]
pub fn live_offered_lots(world: &World) -> std::collections::BTreeSet<LotId> {
    let Some(p) = world
        .module_state::<CoreProductionState>()
        .and_then(|state| state.integration.as_ref())
    else {
        return std::collections::BTreeSet::new();
    };
    let now = world.now();
    p.offers
        .values()
        .filter(|o| {
            o.expires_utc > now
                || p.orders
                    .values()
                    .any(|order| order.intent.offer == o.operation)
        })
        .filter_map(|o| match &o.input {
            super::integration::offer::OfferedInput::SupplierStock { lot, .. } => Some(lot.id()),
            _ => None,
        })
        .collect()
}

/// **The dies `agent` issued since this boot that no live integration offer names**
/// ([`live_offered_lots`]), oldest first: the issuing operation, the class and the lot. The lot
/// may since have left the agent's stock by another road; an offer that names it is then refused
/// by its provider. A restart forgets this record: the game's counter reads the retained
/// certificates for a die it can offer again.
#[must_use]
pub fn unoffered_dies(world: &World, agent: ObjId) -> Vec<(CommitToken, DieClass, LotId)> {
    let Some(state) = world.module_state::<CoreProductionState>() else {
        return Vec::new();
    };
    let offered = live_offered_lots(world);
    state
        .issued
        .values()
        .filter(|(_, by, _, lot)| *by == agent && !offered.contains(lot))
        .map(|(op, _, class, lot)| (*op, *class, *lot))
        .collect()
}
