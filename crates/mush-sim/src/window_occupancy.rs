//! Bounded metric rows shared by Atlas selection and the native window fence.
//!
//! These are placement observations only. Custody exclusions, Atlas geometry,
//! Core objects and owner readiness retain their separate guards. No snapshot
//! grants admission, installs a placement or establishes personnel history.

use mush_core::{DefinitionDigest, ObjId};
use serde::Serialize;

use crate::{DomainId, SpatialParent, SpatialState};

pub const MAX_WINDOW_OCCUPANCY_PLACEMENTS: usize = 4096;
const MAX_OCCUPANCY_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OccupancyError {
    #[error("window occupancy exceeds its bounded envelope")]
    Limit,
    #[error("window occupancy has an unsupported coordinate")]
    Coordinate,
}

/// Only fields consulted by entry-cell occupancy, in canonical map-key order.
/// The map key and stored object id are both observations, even if malformed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct OccupancyRow {
    pub key: ObjId,
    pub object: ObjId,
    pub revision: u64,
    pub x_cm: i64,
    pub y_cm: i64,
}

pub fn rows(state: &SpatialState, domain: DomainId) -> Result<Vec<OccupancyRow>, OccupancyError> {
    if state.placements.len() > MAX_WINDOW_OCCUPANCY_PLACEMENTS {
        return Err(OccupancyError::Limit);
    }
    let mut rows = Vec::new();
    for (key, placement) in &state.placements {
        let SpatialParent::Direct { domain: held, pose } = &placement.parent else {
            continue;
        };
        if *held != domain {
            continue;
        }
        if pose.x_cm.unsigned_abs() > 1_000_000_000 || pose.y_cm.unsigned_abs() > 1_000_000_000 {
            return Err(OccupancyError::Coordinate);
        }
        rows.push(OccupancyRow {
            key: *key,
            object: placement.object,
            revision: placement.revision,
            x_cm: pose.x_cm,
            y_cm: pose.y_cm,
        });
    }
    Ok(rows)
}

/// Retained scalar preimage for final binding to the actual SpatialState slot.
/// There is deliberately no decoder and no owner or write capability here.
#[derive(PartialEq, Eq, Serialize)]
pub(super) struct OccupancyFence {
    domain: DomainId,
    records: usize,
    digest: DefinitionDigest,
}
impl OccupancyFence {
    pub(super) fn capture(state: &SpatialState, domain: DomainId) -> Result<Self, OccupancyError> {
        let rows = rows(state, domain)?;
        let digest = DefinitionDigest::for_canonical_value_with_limit(
            "sim.window_occupancy.preimage.v1",
            &serde_json::to_value(&rows).map_err(|_| OccupancyError::Limit)?,
            MAX_OCCUPANCY_BYTES,
        )
        .map_err(|_| OccupancyError::Limit)?;
        Ok(Self {
            domain,
            records: rows.len(),
            digest,
        })
    }
    pub(super) fn recheck(&self, state: &SpatialState) -> bool {
        Self::capture(state, self.domain).is_ok_and(|now| now == *self)
    }
}

#[cfg(test)]
#[path = "window_occupancy_tests.rs"]
mod tests;
