//! Callback-free canonical native preimage for a later real owner bind.
//! Core rows, owner-read observations and the recovery append bind separately.

use super::*;

const MAX_NATIVE_PREIMAGE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Serialize)]
pub(super) struct NativeFence {
    digest: DefinitionDigest,
}
impl NativeFence {
    pub(super) fn capture(state: &SpatialState) -> Result<Self, ParticipantError> {
        Ok(Self {
            digest: digest(state)?,
        })
    }
    pub(super) fn recheck(&self, state: &SpatialState) -> Result<(), ParticipantError> {
        if digest(state)? != self.digest {
            return Err(refused());
        }
        Ok(())
    }
}
fn digest(state: &SpatialState) -> Result<DefinitionDigest, ParticipantError> {
    // Run the existing opaque-payload and resident bounds before recursively
    // encoding anything. No World or SpatialState clone is needed for binding.
    posture_bounds(state)?;
    crate::physical::resident_bound(state)?;
    if crate::graph::rebuild(state) != state.derived || !state.unbound_placement_docs.is_empty() {
        return Err(refused());
    }
    let held_placements: Vec<_> = state
        .held_placements
        .iter()
        .map(|(id, row)| (id, &row.key, &row.reason))
        .collect();
    let held_assignments: Vec<_> = state
        .held_assignments
        .iter()
        .map(|(id, row)| (id, &row.key, &row.reason))
        .collect();
    let value = json!({
        "domains":state.domains,"placements":state.placements,"spaces":state.spaces,
        "mounts":state.mounts,"stations":state.stations,"assignments":state.assignments,
        "control_gen":state.control_gen,"held_placements":held_placements,
        "held_assignments":held_assignments,"next_domain_id":state.next_domain_id,
        "recovery_room":state.recovery_room,"in_transit_room":state.in_transit_room,
    });
    DefinitionDigest::for_canonical_value_with_limit(
        "sim.window.native_preimage.v1",
        &value,
        MAX_NATIVE_PREIMAGE_BYTES,
    )
    .map_err(|_| ParticipantError::ImageLimit)
}
