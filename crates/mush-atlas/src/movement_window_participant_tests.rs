#![cfg(test)]

use super::*;
use mush_core::locomotion::completion::MovementCoverage;
use mush_core::locomotion::composed::{
    ComposedLocomotionBuilder, ComposedOwnerReader, WindowLocomotionDraft, WindowLocomotionIntent,
};
use mush_core::participant::BoundParticipant;
use mush_core::{
    ActionError, ActionInvocation, ActionProvider, DefinitionRef, DefinitionValidationContext,
    Engine, GameModule, Registry, World,
};
use std::sync::OnceLock;

// Registration-only fixture. It refuses every execution and supplies no Sim
// placement proof, recovery evidence or substitute physical implementation.
#[derive(Clone)]
struct RefusingState;
struct RefusingPrimary;
struct NeverBound;
impl BoundParticipant for NeverBound {
    fn apply(self) {}
}
impl OwnerParticipant for RefusingPrimary {
    type State = RefusingState;
    type Request = WindowLocomotionIntent;
    type Output = WindowLocomotionDraft;
    type Patch = ();
    type Bound<'a> = NeverBound;
    fn prepare(
        &self,
        _: &Self::State,
        _: &OwnerPrepareContext<'_>,
        _: Self::Request,
    ) -> Result<OwnerPreparation<Self::Output, Self::Patch>, ParticipantError> {
        Err(refused())
    }
    fn recheck(
        &self,
        _: &Self::State,
        _: &OwnerPrepareContext<'_>,
        _: &Self::Patch,
    ) -> Result<(), ParticipantError> {
        Err(refused())
    }
    fn bind<'a>(
        self: Arc<Self>,
        _: &'a mut Self::State,
        _: Self::Patch,
    ) -> Result<Self::Bound<'a>, ParticipantError> {
        Err(refused())
    }
    fn accepts_write(&self, _: &ReplayWrite) -> bool {
        false
    }
}
struct RefusingProvider;
impl ActionProvider for RefusingProvider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
}
struct SimRegistration {
    reader: Arc<OnceLock<ComposedOwnerReader>>,
    action: Arc<OnceLock<DefinitionRef>>,
}
impl GameModule for SimRegistration {
    fn name(&self) -> &'static str {
        "sim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        for kind in ["action", "owner_participant"] {
            reg.definition_validator(
                DefinitionKind::new(kind).map_err(bad)?,
                |_: &DefinitionValidationContext<'_>| Ok(()),
            )?;
        }
        let action = reg.definition(composed::window_action_descriptor()?)?;
        let primary = reg.definition(
            DefinitionDescriptor::new(
                DefinitionKind::new("owner_participant").map_err(bad)?,
                DefinitionId::new(composed::SIM_PARTICIPANT).map_err(bad)?,
                2,
                json!({"fixture":"refuses_every_execution"}),
            )
            .map_err(bad)?
            .as_current(),
        )?;
        reg.state(RefusingState);
        reg.owner_participant(
            primary.clone(),
            BTreeSet::from([action.clone()]),
            RefusingPrimary,
        )?;
        let mut definitions = DefinitionRegistryBuilder::new();
        let participant = definitions
            .register(OwnerId::new(NS).map_err(bad)?, participant_descriptor()?)
            .map_err(bad)?;
        let observers = observer_descriptors()?
            .into_iter()
            .map(|d| {
                definitions
                    .register(OwnerId::new(NS).map_err(bad)?, d)
                    .map_err(bad)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let service =
            ComposedLocomotionBuilder::new_window::<RefusingPrimary>(reg, action.clone(), primary)?
                .owner::<AtlasWindowMovementParticipant>(reg, participant.clone(), observers)?
                .finish(reg)?;
        self.reader
            .set(service.owner_reader(&participant).map_err(bad)?)
            .map_err(|_| bad("reader reused"))?;
        self.action
            .set(action.clone())
            .map_err(|_| bad("action reused"))?;
        reg.action_provider(action, RefusingProvider)
    }
}

#[test]
fn actual_atlas_registration_covers_discovery_and_keeps_window_arrival_as_a_blocker() {
    let reader = Arc::new(OnceLock::new());
    let action = Arc::new(OnceLock::new());
    let engine = Box::new(
        Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![
                Box::new(SimRegistration {
                    reader: Arc::clone(&reader),
                    action: Arc::clone(&action),
                }),
                Box::new(crate::AtlasModule::new().with_window_composed_movement(reader)),
            ],
        )
        .expect("actual Atlas registration"),
    );
    let manifest = engine
        .registry
        .movement_completion_manifest(action.get().unwrap())
        .unwrap()
        .snapshot();
    assert_eq!(manifest.entries().len(), 2);
    assert_eq!(manifest.groups().len(), 1);
    assert_eq!(
        manifest.groups()[0].participant().key().id.as_str(),
        PARTICIPANT
    );
    assert!(manifest.has_blockers());
    assert_eq!(
        manifest.entries()[0]
            .observer()
            .as_ref()
            .unwrap()
            .key()
            .id
            .as_str(),
        "travel_discovery"
    );
    assert_eq!(
        manifest.entries()[1]
            .observer()
            .as_ref()
            .unwrap()
            .key()
            .id
            .as_str(),
        "window_arrival"
    );
    assert_eq!(
        manifest.entries()[1].coverage(),
        MovementCoverage::LegacyBlocker
    );
}
