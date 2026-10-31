//! **The bay refit's parts** (doc 31 cut UD.4b): Inventory's custody of the
//! part a bay order fits and of the part it takes out.
//!
//! Accept holds a one-unit lot in the bay's stores for the order (the lot
//! stays where it is; its head marks it held, so no ordinary verb moves,
//! splits or sells it). Install consumes the held lot and, when the refit
//! takes a part out, returns that part to the same stores as a new lot.
//! Cancel releases the hold. The kernel's `bay_refit` protocol joins every
//! record here to the Industry order exactly; this module proves quantity and
//! identity only. Payment, the refit and the order remain their owners'.
use crate::{EndpointRef, Lot, LotCensus, LotId};
use mush_core::{ActionTxn, CommitToken, DefinitionRef, ModError, Registrar, StateFamilyHandle};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const ACCEPT_ACTION: &str = "accept_bay_refit";
pub const INSTALL_ACTION: &str = "install_bay_refit";
pub const CANCEL_ACTION: &str = "cancel_bay_refit";
pub const MAX_OPERATIONS: u64 = 8192;
const PARTS: &str = "inventory_bay_parts";
const HEADS: &str = "inventory_bay_part_heads";
const CENSUS: &str = "inventory_bay_part_census";

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("bay part identity or history disagrees")]
    Integrity,
    #[error("bay part authority is unavailable")]
    Unavailable,
    #[error("bay part is held or retired")]
    Reserved,
    #[error("bay part capacity is exhausted")]
    Capacity,
}
fn bad<T>(_: T) -> Error {
    Error::Integrity
}

/// Trusted composition chooses the exact three Industry bay actions. No
/// command or durable row can create a registered handle.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayPartBinding {
    accept: DefinitionRef,
    install: DefinitionRef,
    cancel: DefinitionRef,
}
impl BayPartBinding {
    /// # Errors
    ///
    /// [`Error::Integrity`] for an action other than Industry's version-1
    /// `accept_bay_refit`, `install_bay_refit` and `cancel_bay_refit`.
    pub fn new(
        accept: DefinitionRef,
        install: DefinitionRef,
        cancel: DefinitionRef,
    ) -> Result<Self, Error> {
        let value = Self {
            accept,
            install,
            cancel,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn actions(&self) -> [&DefinitionRef; 3] {
        [&self.accept, &self.install, &self.cancel]
    }
    fn validate(&self) -> Result<(), Error> {
        for (r, id) in
            self.actions()
                .into_iter()
                .zip([ACCEPT_ACTION, INSTALL_ACTION, CANCEL_ACTION])
        {
            if r.key().owner.as_str() != "industry"
                || r.key().kind.as_str() != "action"
                || r.key().id.as_str() != id
                || r.version() != 1
            {
                return Err(Error::Integrity);
            }
        }
        Ok(())
    }
}

/// Where a held part stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BayPartState {
    Held,
    Consumed,
    Released,
}

/// **One order's part custody**, in the exact shape the kernel joins: the
/// order, the lot held, the stores it is in, the Accept that held it, where
/// it stands and the terminal commit that moved it last.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayPartCustody {
    format: u16,
    order: u64,
    lot: LotId,
    stores: EndpointRef,
    accepted: CommitToken,
    state: BayPartState,
    #[serde(deserialize_with = "required_option")]
    terminal: Option<CommitToken>,
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
impl BayPartCustody {
    pub const fn order(&self) -> u64 {
        self.order
    }
    pub const fn lot(&self) -> LotId {
        self.lot
    }
    pub const fn stores(&self) -> EndpointRef {
        self.stores
    }
    pub const fn state(&self) -> BayPartState {
        self.state
    }
    pub const fn accepted(&self) -> CommitToken {
        self.accepted
    }
    pub const fn terminal(&self) -> Option<CommitToken> {
        self.terminal
    }
    fn validate(&self) -> Result<(), Error> {
        if self.format != 1
            || (self.state == BayPartState::Held) != self.terminal.is_none()
            || self.terminal == Some(self.accepted)
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}
/// A held lot's head: the order holding it, so ordinary verbs refuse it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Head {
    format: u16,
    lot: LotId,
    order: u64,
}
/// How many orders have ever held a part.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Census {
    format: u16,
    count: u64,
}

fn descriptor(name: &str) -> Result<mush_core::StateFamilyDescriptor, ModError> {
    mush_core::bay_refit_protocol::family_descriptor(name).map_err(|e| ModError::msg(e.to_string()))
}
fn key(value: u64) -> String {
    format!("{value:016x}")
}
fn decode<T: serde::de::DeserializeOwned>(v: &serde_json::Value) -> Result<T, String> {
    serde_json::from_value(v.clone()).map_err(|e| e.to_string())
}
fn encode<T: Serialize>(r: &T) -> Result<serde_json::Value, String> {
    serde_json::to_value(r).map_err(|e| e.to_string())
}

#[derive(Debug)]
struct Shared {
    binding: BayPartBinding,
    parts: StateFamilyHandle<u64, BayPartCustody>,
    heads: StateFamilyHandle<LotId, Head>,
    census: StateFamilyHandle<(), Census>,
    lots: StateFamilyHandle<LotId, Lot>,
    lot_census: StateFamilyHandle<EndpointRef, LotCensus>,
    core_inputs: Option<crate::core_input::CoreInputHandle>,
}
/// The registered bay part families, handed to the Industry bay provider.
#[derive(Clone, Debug)]
pub struct BayPartHandle {
    shared: Arc<Shared>,
}

pub(crate) fn register(
    reg: &mut Registrar<'_>,
    binding: BayPartBinding,
    lots: StateFamilyHandle<LotId, Lot>,
    lot_census: StateFamilyHandle<EndpointRef, LotCensus>,
    core_inputs: Option<crate::core_input::CoreInputHandle>,
) -> Result<BayPartHandle, ModError> {
    binding
        .validate()
        .map_err(|e| ModError::msg(e.to_string()))?;
    let parts = reg.state_family(
        descriptor(PARTS)?,
        |k: &u64| Ok(key(*k)),
        decode::<BayPartCustody>,
        |r: &BayPartCustody| r.validate().map_err(|e| e.to_string()),
        encode::<BayPartCustody>,
    )?;
    let heads = reg.state_family(
        descriptor(HEADS)?,
        |k: &LotId| Ok(key(k.0)),
        decode::<Head>,
        |r: &Head| {
            if r.format == 1 {
                Ok(())
            } else {
                Err("bay part head format".into())
            }
        },
        encode::<Head>,
    )?;
    let census = reg.state_family(
        descriptor(CENSUS)?,
        |_: &()| Ok("registry".into()),
        decode::<Census>,
        |r: &Census| {
            if r.format == 1 && r.count <= MAX_OPERATIONS {
                Ok(())
            } else {
                Err("bay part census bounds".into())
            }
        },
        encode::<Census>,
    )?;
    Ok(BayPartHandle {
        shared: Arc::new(Shared {
            binding,
            parts,
            heads,
            census,
            lots,
            lot_census,
            core_inputs,
        }),
    })
}

/// What Install does with the parts: consume the held lot (when the refit
/// fits one) and return the part taken out as `returned` (when it takes one).
#[derive(Clone, Debug)]
pub struct BayInstall {
    pub order: u64,
    pub commit: CommitToken,
    pub stores: EndpointRef,
    pub consume: bool,
    pub returned: Option<Lot>,
}

impl BayPartHandle {
    /// The order's custody, if it holds a part.
    ///
    /// # Errors
    ///
    /// [`Error::Integrity`] for a record that disagrees with its key.
    pub fn custody(
        &self,
        txn: &mut ActionTxn<'_>,
        order: u64,
    ) -> Result<Option<BayPartCustody>, Error> {
        let value = txn.get(&self.shared.parts, &order).map_err(bad)?.cloned();
        if let Some(v) = &value {
            v.validate()?;
            if v.order != order {
                return Err(Error::Integrity);
            }
        }
        Ok(value)
    }
    /// The custody holding `lot` now, if any.
    fn holder(&self, txn: &mut ActionTxn<'_>, lot: LotId) -> Result<Option<BayPartCustody>, Error> {
        let Some(head) = txn.get(&self.shared.heads, &lot).map_err(bad)?.cloned() else {
            return Ok(None);
        };
        let custody = self.custody(txn, head.order)?.ok_or(Error::Integrity)?;
        if head.lot != lot || custody.lot != lot {
            return Err(Error::Integrity);
        }
        Ok(Some(custody))
    }
    /// Refuse an ordinary verb on a lot a bay order holds or consumed.
    pub(crate) fn require_unheld(&self, txn: &mut ActionTxn<'_>, lot: LotId) -> Result<(), Error> {
        match self.holder(txn, lot)?.map(|c| c.state) {
            Some(BayPartState::Held | BayPartState::Consumed) => Err(Error::Reserved),
            _ => Ok(()),
        }
    }
    /// Refuse an ordinary verb minting a lot id a bay order once held.
    pub(crate) fn require_new(&self, txn: &mut ActionTxn<'_>, lot: LotId) -> Result<(), Error> {
        if txn.get(&self.shared.heads, &lot).map_err(bad)?.is_some() {
            Err(Error::Reserved)
        } else {
            Ok(())
        }
    }
    fn industry(txn: &ActionTxn<'_>) -> Result<(), Error> {
        if txn.owner().as_str() != "industry" {
            return Err(Error::Unavailable);
        }
        Ok(())
    }
    /// **Accept**: hold `lot`, a one-unit lot in `stores`, for `order`.
    ///
    /// # Errors
    ///
    /// [`Error::Reserved`] for a lot a bay order or a core input holds, and
    /// [`Error::Integrity`] for a lot elsewhere or of more than one unit.
    pub fn reserve(
        &self,
        txn: &mut ActionTxn<'_>,
        order: u64,
        lot: LotId,
        stores: EndpointRef,
        accepted: CommitToken,
    ) -> Result<BayPartCustody, Error> {
        Self::industry(txn)?;
        let f = &self.shared;
        if self.custody(txn, order)?.is_some() {
            return Err(Error::Reserved);
        }
        let current = txn
            .get(&f.lots, &lot)
            .map_err(bad)?
            .cloned()
            .ok_or(Error::Unavailable)?;
        if current.id() != lot || current.endpoint() != stores || current.quantity() != 1 {
            return Err(Error::Integrity);
        }
        if let Some(core) = &f.core_inputs {
            core.require_unheld(txn, lot).map_err(|_| Error::Reserved)?;
        }
        let previous = self.holder(txn, lot)?;
        if previous
            .as_ref()
            .is_some_and(|c| c.state != BayPartState::Released)
        {
            return Err(Error::Reserved);
        }
        let census = txn.get(&f.census, &()).map_err(bad)?.cloned();
        let count = census.as_ref().map_or(0, |c| c.count);
        if count >= MAX_OPERATIONS {
            return Err(Error::Capacity);
        }
        let custody = BayPartCustody {
            format: 1,
            order,
            lot,
            stores,
            accepted,
            state: BayPartState::Held,
            terminal: None,
        };
        let head = Head {
            format: 1,
            lot,
            order,
        };
        let next = Census {
            format: 1,
            count: count + 1,
        };
        txn.insert(&f.parts, &order, custody.clone()).map_err(bad)?;
        if previous.is_some() {
            txn.replace(&f.heads, &lot, head).map_err(bad)?;
        } else {
            txn.insert(&f.heads, &lot, head).map_err(bad)?;
        }
        if census.is_some() {
            txn.replace(&f.census, &(), next).map_err(bad)?;
        } else {
            txn.insert(&f.census, &(), next).map_err(bad)?;
        }
        Ok(custody)
    }
    fn finish(
        &self,
        txn: &mut ActionTxn<'_>,
        order: u64,
        commit: CommitToken,
        state: BayPartState,
    ) -> Result<BayPartCustody, Error> {
        let old = self.custody(txn, order)?.ok_or(Error::Unavailable)?;
        if old.state != BayPartState::Held || commit == old.accepted {
            return Err(Error::Integrity);
        }
        let next = BayPartCustody {
            state,
            terminal: Some(commit),
            ..old
        };
        txn.replace(&self.shared.parts, &order, next.clone())
            .map_err(bad)?;
        Ok(next)
    }
    /// **Cancel**: release the order's held lot where it lies.
    ///
    /// # Errors
    ///
    /// [`Error::Unavailable`] for an order holding no part.
    pub fn release(
        &self,
        txn: &mut ActionTxn<'_>,
        order: u64,
        commit: CommitToken,
    ) -> Result<BayPartCustody, Error> {
        Self::industry(txn)?;
        self.finish(txn, order, commit, BayPartState::Released)
    }
    /// **Install**: consume the held lot and return the part taken out to the
    /// same stores, moving the stores' lot census by exactly one out and one
    /// back, written once.
    ///
    /// # Errors
    ///
    /// [`Error::Integrity`] for a returned lot elsewhere, of more than one
    /// unit, or with an id already in use; [`Error::Unavailable`] for an
    /// Install that moves no part, or consumes a part the order does not hold.
    pub fn install(
        &self,
        txn: &mut ActionTxn<'_>,
        request: BayInstall,
    ) -> Result<Option<BayPartCustody>, Error> {
        Self::industry(txn)?;
        let f = &self.shared;
        if !request.consume && request.returned.is_none() {
            return Err(Error::Unavailable);
        }
        let custody = if request.consume {
            let c = self.finish(txn, request.order, request.commit, BayPartState::Consumed)?;
            if c.stores != request.stores {
                return Err(Error::Integrity);
            }
            txn.remove(&f.lots, &c.lot).map_err(bad)?;
            Some(c)
        } else {
            None
        };
        if let Some(lot) = &request.returned {
            if lot.endpoint() != request.stores
                || lot.quantity() != 1
                || txn.get(&f.lots, &lot.id()).map_err(bad)?.is_some()
                || txn.get(&f.heads, &lot.id()).map_err(bad)?.is_some()
            {
                return Err(Error::Integrity);
            }
            txn.insert(&f.lots, &lot.id(), lot.clone()).map_err(bad)?;
        }
        let census = txn
            .get(&f.lot_census, &request.stores)
            .map_err(bad)?
            .copied()
            .ok_or(Error::Integrity)?;
        let delta = i64::from(request.returned.is_some()) - i64::from(request.consume);
        if request.consume && census.get() == 0 {
            return Err(Error::Integrity);
        }
        let next = census.checked_move(delta).map_err(bad)?;
        txn.replace(&f.lot_census, &request.stores, next)
            .map_err(bad)?;
        Ok(custody)
    }
    /// The binding's three actions.
    pub fn binding(&self) -> &BayPartBinding {
        &self.shared.binding
    }
}

impl crate::InventoryFamilies {
    /// The bay part families, when composed.
    pub fn bay_parts(&self) -> Option<&BayPartHandle> {
        self.bay_parts.as_ref()
    }
}

#[cfg(test)]
#[path = "bay_part_tests.rs"]
mod tests;
