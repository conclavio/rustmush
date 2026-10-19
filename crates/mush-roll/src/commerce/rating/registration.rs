//! Closed Registry declarations and the private typed installer selected by
//! trusted composition. A descriptor/reference is data, never a certificate or
//! a participant handle; the real registry checks exact owners, types and grants.

pub mod reconformance;
pub use reconformance::ReconformanceDefinitions;

use std::marker::PhantomData;

use mush_core::action::{ActionError, ActionErrorCode};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    OwnerId,
};
use mush_core::participant::OwnerParticipant;
use mush_core::{ModError, Registrar};
use serde_json::json;

use super::producer::RegistryBuilder;
use super::protocol::{ArtifactOutput, ArtifactRequest, Transition};
use super::{RatingCompatibility, RatingFamilies};
use crate::commerce::families::CommerceFamilies;
use crate::commerce::participant::RootParticipant;

pub const TRANSITIONS: [Transition; 5] = [
    Transition::Admit,
    Transition::Grant,
    Transition::Settle,
    Transition::Cancel,
    Transition::Recover,
];

fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn descriptor(
    kind: &str,
    id: &str,
    value: serde_json::Value,
) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(error)?,
        DefinitionId::new(id).map_err(error)?,
        1,
        value,
    )
    .map_err(error)
}
fn reference(owner: &str, descriptor: DefinitionDescriptor) -> Result<DefinitionRef, ModError> {
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new(owner).map_err(error)?, descriptor)
        .map_err(error)
}

/// One structural declaration shared by the closed filing, rating and future
/// paid-root variants. Actual action grants still determine who may call it.
pub fn commercial_roots_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "owner_participant",
        "commercial_roots",
        json!({
            "format":1,"protocol":"commercial_retention_v1"
        }),
    )
}

#[derive(Clone)]
pub struct RegistryDefinitions {
    manifest: DefinitionRef,
    actions: [DefinitionDescriptor; 5],
    references: [DefinitionRef; 5],
    quota: DefinitionDescriptor,
    artifacts: DefinitionDescriptor,
    outcome: DefinitionDescriptor,
}
impl RegistryDefinitions {
    /// Pure deterministic composition data. Consumers must register these
    /// actual definitions and exact typed participant grants; no transport
    /// accepts this value and constructing it grants no action authority.
    pub fn new(manifest: DefinitionRef) -> Result<Self, ModError> {
        if manifest.key().owner.as_str() != "tacsim"
            || manifest.key().kind.as_str() != "registry_manifest"
            || manifest.key().id.as_str() != "combat"
            || manifest.version() != 1
        {
            return Err(ModError::msg("Registry manifest identity"));
        }
        let quota = descriptor(
            "owner_participant",
            "registry_quota",
            json!({
                "format":1,"protocol":"registry_rating_v1","slot":"quota"
            }),
        )?;
        let artifacts = descriptor(
            "owner_participant",
            "registry_artifacts",
            json!({
                "format":1,"protocol":"registry_rating_v1","slot":"artifacts"
            }),
        )?;
        let refs = vec![
            manifest.clone(),
            reference("roll", commercial_roots_descriptor()?)?,
            reference("tacsim", quota.clone())?,
            reference("tacsim", artifacts.clone())?,
        ];
        let mut actions = Vec::with_capacity(5);
        let mut references = Vec::with_capacity(5);
        for transition in TRANSITIONS {
            let word = match transition {
                Transition::Admit => "admit",
                Transition::Grant => "grant",
                Transition::Settle => "settle",
                Transition::Cancel => "cancel",
                Transition::Recover => "recover",
            };
            let action = descriptor(
                "action",
                transition.action_id(),
                json!({
                    "format":1,"protocol":"registry_rating_v1","transition":word
                }),
            )?
            .with_references(refs.clone());
            references.push(reference("roll", action.clone())?);
            actions.push(action);
        }
        Ok(Self {
            manifest,
            actions: actions
                .try_into()
                .map_err(|_| ModError::msg("Registry action count"))?,
            references: references
                .try_into()
                .map_err(|_| ModError::msg("Registry reference count"))?,
            quota,
            artifacts,
            outcome: descriptor(
                "outcome",
                "rating_transition_committed",
                json!({
                    "format":1,"protocol":"registry_rating_v1"
                }),
            )?,
        })
    }
    pub fn actions(&self) -> &[DefinitionRef; 5] {
        &self.references
    }
    pub fn quota(&self) -> &DefinitionDescriptor {
        &self.quota
    }
    pub fn artifacts(&self) -> &DefinitionDescriptor {
        &self.artifacts
    }
    pub fn manifest(&self) -> &DefinitionRef {
        &self.manifest
    }
    pub(crate) fn definitions(&self) -> impl Iterator<Item = DefinitionDescriptor> + '_ {
        self.actions
            .iter()
            .cloned()
            .chain(std::iter::once(self.outcome.clone()))
    }
}

/// The only implementation is the type-selected adapter below. Module callers
/// cannot inject an arbitrary installer callback through RollModule.
pub(crate) trait Installer: Send {
    fn audit_stopped_jobs(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
        executions: &[super::execution::RatingExecution],
    ) -> Result<(), super::RatingError>;

    fn register(
        &self,
        reg: &mut Registrar<'_>,
        definitions: &RegistryDefinitions,
        compatibility: &RatingCompatibility,
        commerce: CommerceFamilies,
        ratings: RatingFamilies,
        marked: bool,
        reconformance: Option<&ReconformanceDefinitions>,
        measured_recall: Option<[DefinitionRef; 2]>,
    ) -> Result<Option<super::reconformance::recall::Readiness>, ModError>;
}
struct Owners<Q, A>(PhantomData<fn() -> (Q, A)>);
pub(crate) fn owners<Q, A>() -> Box<dyn Installer>
where
    Q: super::protocol::RegistryQuotaParticipant,
    A: OwnerParticipant<Request = ArtifactRequest, Output = ArtifactOutput>,
{
    Box::new(Owners::<Q, A>(PhantomData))
}
impl<Q, A> Installer for Owners<Q, A>
where
    Q: super::protocol::RegistryQuotaParticipant,
    A: OwnerParticipant<Request = ArtifactRequest, Output = ArtifactOutput>,
{
    fn audit_stopped_jobs(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
        executions: &[super::execution::RatingExecution],
    ) -> Result<(), super::RatingError> {
        Q::audit_stopped_jobs(ctx, executions)
    }

    fn register(
        &self,
        reg: &mut Registrar<'_>,
        definitions: &RegistryDefinitions,
        compatibility: &RatingCompatibility,
        commerce: CommerceFamilies,
        ratings: RatingFamilies,
        marked: bool,
        reconformance: Option<&ReconformanceDefinitions>,
        measured_recall: Option<[DefinitionRef; 2]>,
    ) -> Result<Option<super::reconformance::recall::Readiness>, ModError> {
        if reg.module() != crate::NS || definitions.manifest.digest() != compatibility.digest() {
            return Err(ModError::msg("Registry producer composition identity"));
        }
        let marked = if marked {
            Some(crate::commerce::marked_rating::MarkedRatingReader::request(
                reg,
                &definitions.manifest,
            )?)
        } else {
            None
        };
        let outcome = reg.definition(definitions.outcome.clone())?;
        for ((descriptor, expected), transition) in definitions
            .actions
            .iter()
            .zip(&definitions.references)
            .zip(TRANSITIONS)
        {
            let action = reg.definition(descriptor.clone())?;
            if action != *expected {
                return Err(ModError::msg("Registry action registration differs"));
            }
            let quota = reg.request_owner_participant::<Q>(action.clone())?;
            let artifacts = reg.request_owner_participant::<A>(action.clone())?;
            let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
            reg.action_provider(
                action.clone(),
                RegistryBuilder {
                    action,
                    outcome: outcome.clone(),
                    manifest: definitions.manifest.clone(),
                    compatibility: compatibility.clone(),
                    transition,
                    commerce: commerce.clone(),
                    ratings: ratings.clone(),
                    quota,
                    artifacts,
                    roots,
                    marked: if transition == Transition::Admit {
                        marked.clone()
                    } else {
                        None
                    },
                    reconformance: None,
                    refusal: ActionError::new(
                        ActionErrorCode::ProviderRejected,
                        false,
                        "The owned Registry computation is unavailable or its evidence changed.",
                    )
                    .map_err(error)?,
                },
            )?;
        }
        reg.registry_rating_capacity(definitions.manifest.clone(), definitions.references.clone())?;
        if let Some(reconformance) = reconformance {
            if reconformance.manifest() != &definitions.manifest {
                return Err(error("Registry service manifest differs"));
            }
            let reader = super::reconformance::reader::ReconformanceReader::request(
                reg,
                &definitions.manifest,
            )?;
            let outcome = reg.definition(reconformance.outcome.clone())?;
            for ((descriptor, expected), transition) in reconformance
                .descriptors
                .iter()
                .zip(reconformance.actions())
                .zip(TRANSITIONS)
            {
                let action = reg.definition(descriptor.clone())?;
                if action != *expected {
                    return Err(error("Registry service action differs"));
                }
                let quota = reg.request_owner_participant::<Q>(action.clone())?;
                let artifacts = reg.request_owner_participant::<A>(action.clone())?;
                let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
                reg.action_provider(
                    action.clone(),
                    RegistryBuilder {
                        action,
                        outcome: outcome.clone(),
                        manifest: definitions.manifest.clone(),
                        compatibility: compatibility.clone(),
                        transition,
                        commerce: commerce.clone(),
                        ratings: ratings.clone(),
                        quota,
                        artifacts,
                        roots,
                        marked: None,
                        reconformance: Some(reader.clone()),
                        refusal: ActionError::new(
                            ActionErrorCode::ProviderRejected,
                            false,
                            "The current Registry service work is unavailable or changed.",
                        )
                        .map_err(error)?,
                    },
                )?;
            }
            reg.registry_reconformance_capacity(reconformance.actions().clone())?;
            let ready = super::reconformance::recall::register(
                reg,
                commerce,
                ratings,
                compatibility.clone(),
                definitions.manifest.clone(),
                measured_recall.ok_or_else(|| error("measured recall actions absent"))?,
                reader,
            )?;
            return Ok(Some(ready));
        }
        if measured_recall.is_some() {
            return Err(error("measured recall without service profile"));
        }
        Ok(None)
    }
}
