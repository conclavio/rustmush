//! Private proving-world cargo. These lots never enter live Inventory or E4.
//! The real capacity calculator and immutable relocation rules still govern
//! every load/unload. Only an observed native route arrival can unload a lot.
use crate::{
    arena::{Arena, perception::DriveOutcome},
    definitions::GoodsTable,
    port::capacity::{self, Admission, Compartment},
};
use mush_core::{CanonicalId, DefinitionDigest, ObjId};
use mush_economy::{ExternalRef, VaultId};
use mush_inventory::record::{EndpointRef, Grade, Lineage, Lot, LotId};
use mush_roll::commerce::rating::RatingError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CargoSpec {
    pub(super) compartment: CanonicalId,
    pub(super) accepts: BTreeSet<CanonicalId>,
    pub(super) limits: BTreeMap<CanonicalId, u64>,
    pub(super) commodity: CanonicalId,
    pub(super) quantity: u64,
    pub(super) grade: u8,
}
impl CargoSpec {
    pub(super) fn validate(&self) -> Result<(), RatingError> {
        if self.accepts.is_empty()
            || self.accepts.len() > 8
            || self.limits.is_empty()
            || self.limits.len() > 8
            || self.limits.values().any(|n| *n == 0)
            || !self.limits.keys().any(|key| key.as_str() == "si.mass_kg")
            || self.quantity == 0
            || self.quantity > 1_000
        {
            return Err(RatingError::Bound);
        }
        Grade::new(self.grade).map_err(|_| RatingError::Invalid)?;
        Ok(())
    }
}

/// Immutable evidence of the real private lot before/after checked custody.
/// This is result data, never an Inventory authority or qualification token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct CargoReport {
    initial: Lot,
    loaded: Lot,
    terminal: Lot,
    measures: BTreeMap<CanonicalId, u64>,
    carrier: ObjId,
    destination: (u16, u16),
    loaded_at_ms: i64,
    delivered_at_ms: Option<i64>,
}

pub(super) struct Cargo {
    initial: Lot,
    loaded: Lot,
    current: Lot,
    measures: BTreeMap<CanonicalId, u64>,
    carrier: ObjId,
    destination: (u16, u16),
    delivered_at: Option<i64>,
}
impl Cargo {
    /// Called only by fixed scenario construction before the private Arena can
    /// step. The source endpoint, carrier endpoint and destination are private
    /// sandbox IDs, deliberately unrelated to any economic vault in the World.
    pub(super) fn load(
        spec: &CargoSpec,
        compartment: &Compartment,
        goods: &GoodsTable,
        carrier: ObjId,
        destination: (u16, u16),
        scenario: DefinitionDigest,
        seed: u64,
    ) -> Result<Self, RatingError> {
        spec.validate()?;
        if compartment.id != spec.compartment
            || compartment.accepts != spec.accepts
            || compartment.limits != spec.limits
        {
            return Err(RatingError::Identity);
        }
        let Admission::Admitted(measures) = capacity::admits(
            goods,
            compartment,
            &BTreeMap::new(),
            (&spec.commodity, spec.quantity),
        ) else {
            return Err(RatingError::Bound);
        };
        let identity = super::super::identity(
            "tacsim.conduct.sandbox-lot.v1",
            &(scenario, seed, carrier, spec),
            16 * 1024,
        )?;
        let mut id = [0; 8];
        id.copy_from_slice(identity.as_bytes().get(..8).ok_or(RatingError::Identity)?);
        let initial = Lot::new(
            LotId(u64::from_be_bytes(id)),
            EndpointRef(VaultId(0)),
            spec.commodity.clone(),
            spec.quantity,
            Grade::new(spec.grade).map_err(|_| RatingError::Invalid)?,
            Lineage::Origin {
                source: ExternalRef {
                    provider: "tacsim".into(),
                    kind: "conduct_sandbox".into(),
                    id: identity.to_hex().into(),
                    revision: Some(1),
                },
            },
            0,
        )
        .map_err(|_| RatingError::Invalid)?;
        let loaded = initial
            .checked_relocate(initial.revision(), EndpointRef(VaultId(1)))
            .map_err(|_| RatingError::Identity)?;
        Ok(Self {
            initial,
            current: loaded.clone(),
            loaded,
            measures,
            carrier,
            destination,
            delivered_at: None,
        })
    }

    /// Observes actual native movement without constructing another MindView,
    /// touching perception hysteresis, or giving the script any private pose.
    /// A mere placement at the destination cannot produce a native Arrived row.
    pub(super) fn observe(&mut self, arena: &Arena) -> Result<(), RatingError> {
        if self.delivered_at.is_some() {
            return self.check();
        }
        let unit = arena.unit(self.carrier).ok_or(RatingError::Incomplete)?;
        if unit.limits.wreck || unit.seal.closed {
            return self.check();
        }
        if arena.now_ms > 0
            && unit.speed_cms == 0
            && arena.motors.row(self.carrier).is_some_and(|row| {
                row.requested_goal == Some(self.destination)
                    && row.outcome == Some(DriveOutcome::Arrived)
            })
        {
            self.current = self
                .current
                .checked_relocate(self.current.revision(), EndpointRef(VaultId(2)))
                .map_err(|_| RatingError::Identity)?;
            self.delivered_at = Some(arena.now_ms);
        }
        self.check()
    }
    fn check(&self) -> Result<(), RatingError> {
        let loaded = self
            .initial
            .checked_relocate(self.initial.revision(), EndpointRef(VaultId(1)))
            .map_err(|_| RatingError::Identity)?;
        let expected = if self.delivered_at.is_some() {
            loaded
                .checked_relocate(loaded.revision(), EndpointRef(VaultId(2)))
                .map_err(|_| RatingError::Identity)?
        } else {
            loaded.clone()
        };
        if loaded != self.loaded || expected != self.current {
            return Err(RatingError::Identity);
        }
        Ok(())
    }
    pub(super) fn delivered(&self) -> bool {
        self.delivered_at.is_some()
    }
    pub(super) fn report(&self) -> Result<CargoReport, RatingError> {
        self.check()?;
        Ok(CargoReport {
            initial: self.initial.clone(),
            loaded: self.loaded.clone(),
            terminal: self.current.clone(),
            measures: self.measures.clone(),
            carrier: self.carrier,
            destination: self.destination,
            loaded_at_ms: 0,
            delivered_at_ms: self.delivered_at,
        })
    }
}
