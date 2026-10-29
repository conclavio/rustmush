//! Atlas's actual discovery image and window-selector attestation. This covers
//! travel discovery only: window-arrival map output remains a callback blocker.

use crate::module::{AtlasState, NS};
use crate::movement_history::refused;
use crate::movement_participant::{AtlasMovementParticipant, EventClaim};
use mush_core::locomotion::composed::{self, MovementDraft, MovementIntent};
use mush_core::participant::{
    OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, ModError,
    OwnerId, Registrar, ReplayWrite,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};

pub const PARTICIPANT: &str = "window_movement_discovery";
#[path = "movement_window_knowledge.rs"]
mod knowledge;
pub(crate) use knowledge::OrderedProof;
pub use knowledge::{BoundWindowKnowledge, WindowKnowledgePatch};
fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
pub fn participant_descriptor_v1() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(DefinitionKind::new("owner_participant").map_err(bad)?, DefinitionId::new(PARTICIPANT).map_err(bad)?, 1,
        json!({"state":"AtlasState","protocol":"composed_atlas_window_discovery_v1",
            "capture_bytes":131072,"key_bytes":65536,"proof_depth":32,"proof_nodes":65536,
            "known_places":512,"known_policy":"player_fifo_duplicate_unchanged",
            "window_policy":"attest_actual_selectors","completion":["travel_discovery"],
            "window_arrival":"requires_durable_map_output","writes":["known_document","movement_knowledge_ledger"],
            "delivery":"same_commit","proof":"original_window_plan_and_knowledge_position_v1"})).map_err(bad)
}
pub fn participant_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(DefinitionKind::new("owner_participant").map_err(bad)?, DefinitionId::new(PARTICIPANT).map_err(bad)?, 2,
        json!({"state":"AtlasState","protocol":"ordered_atlas_window_discovery_v2",
            "capture_bytes":262144,"key_bytes_per_leg":65536,"proof_bytes":131072,
            "proof_depth":32,"proof_nodes":65536,"max_core_legs":2,"known_places":512,
            "known_policy":"ordered_player_fifo_duplicate_unchanged",
            "window_policy":"attest_actual_selectors","completion":["travel_discovery"],
            "window_arrival":"requires_durable_map_output","writes":["final_known_document","ordered_movement_knowledge_ledger"],
            "delivery":"same_commit","proof":"original_window_plan_and_ordered_knowledge_positions_v2"})).map_err(bad)
}
pub fn observer_descriptors() -> Result<Vec<DefinitionDescriptor>, ModError> {
    Ok(vec![mush_core::moved_observer::descriptor(
        "travel_discovery",
        1,
    )?])
}
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    let part = reg.definition(participant_descriptor()?.as_current())?;
    let mut definitions = DefinitionRegistryBuilder::new();
    let action = definitions
        .register(
            OwnerId::new("sim").map_err(bad)?,
            composed::window_action_descriptor()?,
        )
        .map_err(bad)?;
    let observers = observer_descriptors()?
        .into_iter()
        .map(|descriptor| {
            definitions
                .register(OwnerId::new(NS).map_err(bad)?, descriptor)
                .map_err(bad)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let old = reg.definition(participant_descriptor_v1()?)?;
    reg.definition(mush_core::locomotion::completion::binding_descriptor(
        "atlas_window_discovery_completion",
        1,
        action.clone(),
        old,
        observers.clone(),
    )?)?;
    reg.owner_participant(
        part.clone(),
        BTreeSet::from([action.clone()]),
        AtlasWindowMovementParticipant,
    )?;
    reg.register_moved_completion::<AtlasWindowMovementParticipant>(
        "atlas_window_discovery_completion",
        2,
        action,
        part,
        observers,
    )?;
    Ok(())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Proof {
    pub(crate) format: u16,
    pub(crate) window: Value,
    pub(crate) history_next: u64,
    #[serde(deserialize_with = "required_event")]
    pub(crate) event: Option<EventClaim>,
}
fn required_event<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<EventClaim>, D::Error> {
    Option::deserialize(d)
}

pub struct AtlasWindowMovementParticipant;
impl OwnerParticipant for AtlasWindowMovementParticipant {
    type State = AtlasState;
    type Request = MovementIntent;
    type Output = MovementDraft;
    type Patch = WindowKnowledgePatch;
    type Bound<'a> = BoundWindowKnowledge<'a>;
    fn prepare(
        &self,
        state: &AtlasState,
        ctx: &OwnerPrepareContext<'_>,
        intent: MovementIntent,
    ) -> Result<OwnerPreparation<MovementDraft, WindowKnowledgePatch>, ParticipantError> {
        let projection = intent.window_projection().ok_or_else(refused)?;
        let plan = crate::movement_window::plan(
            ctx.world(),
            intent.actor(),
            intent.source(),
            intent.destination(),
            intent.at_utc(),
        )
        .map_err(|_| refused())?;
        plan.check_projection(projection).map_err(|_| refused())?;
        knowledge::prepare(state, ctx, intent, plan)
    }
    fn recheck(
        &self,
        state: &AtlasState,
        ctx: &OwnerPrepareContext<'_>,
        patch: &WindowKnowledgePatch,
    ) -> Result<(), ParticipantError> {
        patch.recheck(state, ctx)
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut AtlasState,
        patch: WindowKnowledgePatch,
    ) -> Result<Self::Bound<'a>, ParticipantError> {
        patch.bind(state)
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        AtlasMovementParticipant.accepts_write(write)
    }
}

#[cfg(test)]
#[path = "movement_window_participant_tests.rs"]
mod tests;
