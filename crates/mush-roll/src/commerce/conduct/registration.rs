//! Trusted composition fixes both native adapter types and exact five actions.
//! Nothing in a request or stored record chooses an owner implementation.
use super::{
    protocol::*,
    state::{self, Configuration},
    *,
};
use crate::commerce::{
    families::CommerceFamilies,
    participant::RootParticipant,
    rating::{RatingCompatibility, RatingFamilies},
};
use mush_core::{
    DefinitionRef, ModError, Registrar,
    action::{ActionError, ActionErrorCode},
    mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    },
    participant::OwnerParticipant,
};
use std::{marker::PhantomData, sync::Arc};
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

pub fn quota_descriptor() -> Result<DefinitionDescriptor, ModError> {
    participant("conduct_quota", "conduct_quota_v1")
}
pub fn artifacts_descriptor() -> Result<DefinitionDescriptor, ModError> {
    participant("conduct_artifacts", "conduct_artifacts_v1")
}
fn participant(id: &str, protocol: &str) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_participant").map_err(error)?,
        DefinitionId::new(id).map_err(error)?,
        1,
        serde_json::json!({"format":1,"protocol":protocol}),
    )
    .map_err(error)
}
fn outcome() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("outcome").map_err(error)?,
        DefinitionId::new("conduct_trial_progress").map_err(error)?,
        1,
        serde_json::json!({"format":1,"protocol":"roll.conduct_trial.v1"}),
    )
    .map_err(error)
}
pub fn participants() -> Result<[DefinitionRef; 3], ModError> {
    let resolve = |owner: &str, d| {
        DefinitionRegistryBuilder::new()
            .register(OwnerId::new(owner).map_err(error)?, d)
            .map_err(error)
    };
    Ok([
        resolve(
            "roll",
            crate::commerce::rating::registration::commercial_roots_descriptor()?,
        )?,
        resolve("tacsim", quota_descriptor()?)?,
        resolve("tacsim", artifacts_descriptor()?)?,
    ])
}
#[derive(Clone)]
pub struct ConductConfig {
    manifest: DefinitionRef,
    installer: Arc<dyn Installer>,
}
impl std::fmt::Debug for ConductConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConductConfig")
            .field("manifest", &self.manifest)
            .finish_non_exhaustive()
    }
}
impl ConductConfig {
    pub fn new<Q, A>(manifest: DefinitionRef) -> Result<Self, ModError>
    where
        Q: ConductQuotaParticipant,
        A: OwnerParticipant<Request = ArtifactRequest, Output = ArtifactOutput>,
    {
        action_descriptor(&manifest, Transition::Admit).map_err(error)?;
        Ok(Self {
            manifest,
            installer: Arc::new(Owners::<Q, A>(PhantomData)),
        })
    }
    pub fn manifest(&self) -> &DefinitionRef {
        &self.manifest
    }
    pub fn actions(&self) -> Result<[DefinitionRef; 5], ModError> {
        let actions = Transition::ALL
            .into_iter()
            .map(|t| action_ref(&self.manifest, t).map_err(error))
            .collect::<Result<Vec<_>, _>>()?;
        actions
            .try_into()
            .map_err(|_| error("Conduct action count"))
    }
    pub(crate) fn definitions(&self) -> Result<Vec<DefinitionDescriptor>, ModError> {
        let mut result = Transition::ALL
            .into_iter()
            .map(|t| action_descriptor(&self.manifest, t).map_err(error))
            .collect::<Result<Vec<_>, _>>()?;
        result.push(outcome()?);
        Ok(result)
    }
    pub(crate) fn register(
        &self,
        reg: &mut Registrar<'_>,
        families: CommerceFamilies,
        ratings: RatingFamilies,
        expected: RatingCompatibility,
    ) -> Result<Configuration, ModError> {
        self.installer
            .register(reg, self, families, ratings, expected)
    }
}
trait Installer: Send + Sync {
    fn register(
        &self,
        reg: &mut Registrar<'_>,
        config: &ConductConfig,
        families: CommerceFamilies,
        ratings: RatingFamilies,
        expected: RatingCompatibility,
    ) -> Result<Configuration, ModError>;
}
struct Owners<Q, A>(PhantomData<fn() -> (Q, A)>);
impl<Q, A> Installer for Owners<Q, A>
where
    Q: ConductQuotaParticipant,
    A: OwnerParticipant<Request = ArtifactRequest, Output = ArtifactOutput>,
{
    fn register(
        &self,
        reg: &mut Registrar<'_>,
        config: &ConductConfig,
        families: CommerceFamilies,
        ratings: RatingFamilies,
        expected: RatingCompatibility,
    ) -> Result<Configuration, ModError> {
        if reg.module() != crate::NS {
            return Err(error("Conduct owner registration"));
        }
        let result = reg.definition(outcome()?)?;
        for (transition, reference) in Transition::ALL.into_iter().zip(config.actions()?) {
            let action =
                reg.definition(action_descriptor(&config.manifest, transition).map_err(error)?)?;
            if action != reference {
                return Err(error("Conduct exact action registration"));
            }
            let quota = reg.request_owner_participant::<Q>(action.clone())?;
            let artifacts = reg.request_owner_participant::<A>(action.clone())?;
            let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
            reg.action_provider(
                action.clone(),
                super::producer::Builder {
                    action,
                    outcome: result.clone(),
                    manifest: config.manifest.clone(),
                    transition,
                    expected: expected.clone(),
                    families: families.clone(),
                    ratings: ratings.clone(),
                    quota,
                    artifacts,
                    roots,
                    refusal: ActionError::new(
                        ActionErrorCode::ProviderRejected,
                        false,
                        "The owned Conduct trial is unavailable or its evidence changed.",
                    )
                    .map_err(error)?,
                },
            )?;
        }
        reg.conduct_trial_capacity(config.manifest.clone(), participants()?, config.actions()?)?;
        super::reader::register(reg, families.clone(), &config.manifest)?;
        super::work_read::register(reg, families, &config.manifest)?;
        Ok(Configuration {
            native: state::actual::<Q>(),
            ratings,
            expected,
            manifest: config.manifest.clone(),
        })
    }
}
