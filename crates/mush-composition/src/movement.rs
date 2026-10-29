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
pub(crate) type AtlasReader = Arc<OnceLock<ComposedOwnerReader>>;

pub(crate) fn window_selector() -> mush_sim::window_movement::WindowSelector {
    Arc::new(|world, actor, source, destination, at| {
        let refused = |_| {
            mush_core::participant::ParticipantError::refused("Atlas window selection unavailable")
        };
        let plan = mush_atlas::movement_window::plan(world, actor, source, destination, at)
            .map_err(refused)?;
        let (room, target) = plan.projection_target().map_err(refused)?;
        Ok(mush_sim::window_movement::WindowSelection::new(
            room,
            plan.digest().map_err(refused)?,
            target,
        ))
    })
}

pub(crate) fn window_factory(
    scene_reader: SceneReader,
    reader: AtlasReader,
) -> mush_sim::ComposedMovementFactory {
    Arc::new(move |reg, builder| {
        let mut definitions = DefinitionRegistryBuilder::new();
        let owner = OwnerId::new("atlas").map_err(bad)?;
        let participant = definitions
            .register(
                owner.clone(),
                mush_atlas::movement_window_participant::participant_descriptor()?,
            )
            .map_err(bad)?;
        let observers = mush_atlas::movement_window_participant::observer_descriptors()?
            .into_iter()
            .map(|descriptor| definitions.register(owner.clone(), descriptor).map_err(bad))
            .collect::<Result<Vec<_>, _>>()?;
        let scene_owner = OwnerId::new("scenes").map_err(bad)?;
        let scene_participant = definitions
            .register(
                scene_owner.clone(),
                mush_scenes::movement_window::participant_descriptor()?,
            )
            .map_err(bad)?;
        let scene_observer = definitions
            .register(scene_owner, scene_transition::observer_descriptor()?)
            .map_err(bad)?;
        let old_scene_participant = definitions
            .register(
                OwnerId::new("scenes").map_err(bad)?,
                mush_scenes::movement_window::participant_descriptor_v1()?,
            )
            .map_err(bad)?;
        let service = builder
            .owner::<mush_scenes::movement_window::WindowScenesMovementParticipant>(
                reg,
                scene_participant.clone(),
                vec![scene_observer],
            )?
            .owner::<mush_atlas::movement_window_participant::AtlasWindowMovementParticipant>(
                reg,
                participant.clone(),
                observers,
            )?
            .finish(reg)?;
        scene_reader
            .set(
                service
                    .owner_reader_retaining(&scene_participant, &[old_scene_participant])
                    .map_err(bad)?,
            )
            .map_err(|_| bad("window Scenes reader already belongs to a composition"))?;
        reader
            .set(service.owner_reader(&participant).map_err(bad)?)
            .map_err(|_| bad("window Atlas reader already belongs to a composition"))?;
        Ok(service)
    })
}

fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

pub(crate) fn factory(
    reader: SceneReader,
    atlas_reader: AtlasReader,
) -> mush_sim::ComposedMovementFactory {
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
            let atlas_owner = OwnerId::new("atlas").map_err(bad)?;
            let atlas_participant = definitions
                .register(
                    atlas_owner.clone(),
                    mush_atlas::movement_participant::participant_descriptor()?,
                )
                .map_err(bad)?;
            let atlas_observers = mush_atlas::movement_participant::observer_descriptors()?
                .into_iter()
                .map(|descriptor| {
                    definitions
                        .register(atlas_owner.clone(), descriptor)
                        .map_err(bad)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let service = builder
                .owner::<ComposedScenesMovementParticipant>(
                    reg,
                    participant.clone(),
                    vec![observer],
                )?
                .owner::<mush_atlas::movement_participant::AtlasMovementParticipant>(
                    reg,
                    atlas_participant.clone(),
                    atlas_observers,
                )?
                .finish(reg)?;
            reader
                .set(service.owner_reader(&participant).map_err(bad)?)
                .map_err(|_| {
                    bad("composed Scenes movement reader already belongs to a composition")
                })?;
            atlas_reader
                .set(service.owner_reader(&atlas_participant).map_err(bad)?)
                .map_err(|_| bad("Atlas movement reader already belongs to a composition"))?;
            Ok(service)
        },
    )
}
