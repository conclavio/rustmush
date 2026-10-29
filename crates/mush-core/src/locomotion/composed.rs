//! Closed captured locomotion with composition-bound owner effects. Core mints
//! every route request and object image; owner data never substitutes movement.

use crate::participant::ParticipantError;
use crate::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    ModError, OwnerId,
};
use serde_json::{Value, json};

#[path = "composed_owner.rs"]
mod owner;
#[path = "composed_record.rs"]
mod record;
#[path = "composed_service.rs"]
mod service;
#[path = "composed_window.rs"]
mod window;
pub use owner::{MovementDraft, MovementIntent};
pub use record::{
    ComposedLocomotionReader, ComposedLocomotionReceipt, ComposedLocomotionReceiptPage,
    MovementEffectReceipt,
};
pub use record::{ComposedOwnerReader, ComposedOwnerReceipt, ComposedOwnerReceiptPage};
pub(crate) use service::ComposedMovementPermit;
pub use service::{ComposedLocomotion, ComposedLocomotionBuilder};
pub use window::{
    WindowLocomotionDraft, WindowLocomotionIntent, WindowMetricTarget, WindowProjection,
};

pub const ACTION: &str = "sim_composed_locomotion";
pub const SIM_PARTICIPANT: &str = "composed_locomotion_transition";
pub const MAX_COMPOSED_OWNERS: usize = 8;
pub const MAX_MOVEMENT_PROOF_BYTES: usize = 128 * 1024;
pub(crate) const RECORD_BYTES: usize = 256 * 1024;
pub(crate) const FAMILY: &str = "composed_locomotion";
pub(crate) const WINDOW_FAMILY: &str = "window_composed_locomotion";

pub fn action_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(bad)?, DefinitionId::new(ACTION).map_err(bad)?, 1,
        json!({"format":1,"protocol":"core_composed_captured_locomotion_v1",
            "admission":"actual_captured_command","moves":1,"objects":3,
            "subtree":256,"read_dependencies":512,"image_bytes":1_048_576,
            "owner_slots":MAX_COMPOSED_OWNERS,"owner_proof_bytes":MAX_MOVEMENT_PROOF_BYTES,
            "receipt_bytes":RECORD_BYTES,"manifest_bytes":super::completion::MAX_MOVEMENT_MANIFEST_BYTES,
            "completion":"exact_frozen_manifest","delivery":"same_commit",
            "empty_owner":"retained_registered_observation",
            "live_routing":"requires_complete_coverage",
            "core_presentation":"requires_durable_core_output",
            "capture":{"family":"sim.composed_locomotion_crossings","required":"same_commit","evidence":"uncertified"}}),
    ).map(DefinitionDescriptor::as_current).map_err(bad)
}
/// A distinct, retained profile for two Core moves and an attested projection.
/// A partial manifest remains partial; this definition grants no live routing.
pub fn window_action_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(bad)?, DefinitionId::new(ACTION).map_err(bad)?, 2,
        json!({"format":1,"protocol":"core_window_composed_locomotion_v1",
            "admission":"actual_captured_command","moves":2,"objects":4,
            "subtree":256,"read_dependencies":512,"image_bytes":1_048_576,
            "owner_slots":MAX_COMPOSED_OWNERS,"owner_proof_bytes":MAX_MOVEMENT_PROOF_BYTES,
            "receipt_bytes":RECORD_BYTES,"manifest_bytes":super::completion::MAX_MOVEMENT_MANIFEST_BYTES,
            "completion":"exact_frozen_manifest","delivery":"same_commit",
            "projection":"primary_proposal_bound_to_every_owner_intent",
            "window_notification":"requires_actual_owner_and_durable_output",
            "empty_owner":"retained_registered_observation",
            "live_routing":"requires_complete_coverage","core_presentation":"requires_durable_core_output",
            "capture":{"family":"sim.window_locomotion_crossings","required":"same_commit","evidence":"uncertified"}}),
    ).map(DefinitionDescriptor::as_current).map_err(bad)
}
pub(crate) fn exact_window_action(action: &DefinitionRef) -> Result<(), ParticipantError> {
    let expected = DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("sim").map_err(|_| refused())?,
            window_action_descriptor().map_err(|_| refused())?,
        )
        .map_err(|_| refused())?;
    if action != &expected {
        return Err(refused());
    }
    Ok(())
}
pub(crate) fn exact_composition_action(action: &DefinitionRef) -> Result<(), ParticipantError> {
    match action.version() {
        1 => exact_action(action),
        2 => exact_window_action(action),
        _ => Err(refused()),
    }
}
pub(crate) fn exact_action(action: &DefinitionRef) -> Result<(), ParticipantError> {
    let expected = DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("sim").map_err(|_| refused())?,
            action_descriptor().map_err(|_| refused())?,
        )
        .map_err(|_| refused())?;
    if action != &expected {
        return Err(refused());
    }
    Ok(())
}
pub(crate) fn bounded<T: serde::Serialize>(
    value: &T,
    max: usize,
) -> Result<Value, ParticipantError> {
    let value = serde_json::to_value(value).map_err(|_| ParticipantError::Encoding)?;
    crate::canonical_json_bytes_with_limit(&value, max)
        .map_err(|_| ParticipantError::ImageLimit)?;
    Ok(value)
}
pub(crate) fn refused() -> ParticipantError {
    ParticipantError::Authority
}
pub(crate) fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

#[cfg(test)]
#[path = "composed_tests.rs"]
mod tests;
