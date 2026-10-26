//! Exact, source-free readback for the registered game's private continuation.
//! The actor comes from normal core dispatch, never from this request. An
//! acknowledgement is not expanded into a second copy of typed evidence.

use std::collections::BTreeSet;

use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{CommitToken, DefinitionRef, ModError, Registrar};
use serde::Serialize;
use serde_json::json;

use super::{RatingFamilies, RatingJob, execution::RatingExecution};
use crate::RollStore;

#[derive(Serialize)]
pub struct RegistryReadRequest {
    job: CommitToken,
}
impl RegistryReadRequest {
    pub const fn new(job: CommitToken) -> Self {
        Self { job }
    }
}
#[derive(Serialize)]
pub struct RegistryReadResult {
    job: RatingJob,
    execution: RatingExecution,
}
impl RegistryReadResult {
    pub const fn job(&self) -> &RatingJob {
        &self.job
    }
    pub const fn execution(&self) -> &RatingExecution {
        &self.execution
    }
}
pub struct RegistryReader {
    families: RatingFamilies,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    super::registration::RegistryDefinitions::new(manifest.clone())?;
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("registry_continuation").map_err(error)?, 1,
        json!({"format":1,"own_job_only":true,"maximum_rows":2,"source_bytes":false,"account_ids":false}),
    ).map_err(error)?.with_references(vec![manifest.clone()]))
}
impl RegistryReader {
    pub fn request(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        let reference = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                descriptor(manifest)?,
            )
            .map_err(error)?;
        reg.request_owner_read(reference)
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: RatingFamilies,
    manifest: &DefinitionRef,
) -> Result<(), ModError> {
    let definition = reg.definition(descriptor(manifest)?)?;
    reg.owner_read_service(
        definition,
        BTreeSet::from([OwnerId::new("tacsim").map_err(error)?]),
        BTreeSet::from([OwnerId::new("tacsim").map_err(error)?]),
        RegistryReader { families },
    )
}
impl OwnerReadService for RegistryReader {
    type State = RollStore;
    type Request = RegistryReadRequest;
    type Output = RegistryReadResult;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: Self::Request,
    ) -> Result<Self::Output, OwnerReadError> {
        roll.commerce_ready()
            .map_err(|_| OwnerReadError::Unavailable)?;
        let job = ctx
            .action_state(&self.families.jobs, &request.job)?
            .ok_or(OwnerReadError::Refused)?;
        if job.job() != request.job
            || job.subject().revision().revision().key.character != ctx.actor()
        {
            return Err(OwnerReadError::Refused);
        }
        let execution = ctx
            .action_state(&self.families.executions, &request.job)?
            .ok_or(OwnerReadError::Integrity)?;
        execution
            .matches_job(job)
            .map_err(|_| OwnerReadError::Integrity)?;
        Ok(RegistryReadResult {
            job: job.clone(),
            execution: execution.clone(),
        })
    }
}
