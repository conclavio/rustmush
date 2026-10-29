//! Actual movement owner wiring. This root may depend on every owner; Sim
//! retains only the opaque typed composition produced during registration.

use std::sync::{Arc, OnceLock};

use mush_core::locomotion::composed::{
    ComposedLocomotion, ComposedLocomotionBuilder, ComposedOwnerReader,
};
use mush_core::locomotion::scene_transition;
use mush_core::{DefinitionRegistryBuilder, ModError, OwnerId, Registrar};
use mush_scenes::movement_composed::{ComposedScenesMovementParticipant, participant_descriptor};

pub(crate) type SceneReader = Arc<OnceLock<ComposedOwnerReader>>;

fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

pub(crate) fn factory(reader: SceneReader) -> mush_sim::ComposedMovementFactory {
    Arc::new(
        move |reg: &mut Registrar<'_>,
              builder: ComposedLocomotionBuilder|
              -> Result<ComposedLocomotion, ModError> {
            let mut definitions = DefinitionRegistryBuilder::new();
            let owner = OwnerId::new("scenes").map_err(bad)?;
            let participant = definitions
                .register(owner.clone(), participant_descriptor()?)
                .map_err(bad)?;
            let observer = definitions
                .register(owner, scene_transition::observer_descriptor()?)
                .map_err(bad)?;
            let service = builder
                .owner::<ComposedScenesMovementParticipant>(
                    reg,
                    participant.clone(),
                    vec![observer],
                )?
                .finish(reg)?;
            reader
                .set(service.owner_reader(&participant).map_err(bad)?)
                .map_err(|_| {
                    bad("composed Scenes movement reader already belongs to a composition")
                })?;
            Ok(service)
        },
    )
}
