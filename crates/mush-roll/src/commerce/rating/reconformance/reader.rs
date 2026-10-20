//! Closed Registry inspection of retained filed revisions. This service never
//! grants author/customer source rights and accepts no actor or account argument.
use super::{ReconformanceAdmission, ReconformanceHead};
use crate::commerce::rating::{
    RatingCompatibility, RatingFamilies, RatingJob, RatingJobState, execution::RatingExecution,
};
use crate::commerce::{
    FiledRevisionId,
    families::CommerceFamilies,
    records::Record,
    roots::{RequiredRoot, RootAnchor},
};
use crate::{Revision, RollStore};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{CommitToken, DefinitionRef, Flags, GOD, ModError, ObjKind, Registrar};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeSet;

pub use super::recall::history::RejectionProof;
pub const SCAN_ROWS: usize = 16;
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReconformanceReadRequest {
    Scan { after: Option<String> },
    Candidate { filed: FiledRevisionId },
    Job { job: CommitToken },
    Rejection { filed: FiledRevisionId },
}
#[derive(Serialize)]
pub struct ReconformanceCandidate {
    revision: Revision,
    binding: ReconformanceAdmission,
}
impl ReconformanceCandidate {
    pub fn revision(&self) -> &Revision {
        &self.revision
    }
    pub fn binding(&self) -> &ReconformanceAdmission {
        &self.binding
    }
    pub fn into_parts(self) -> (Revision, ReconformanceAdmission) {
        (self.revision, self.binding)
    }
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReconformanceReadResult {
    Page {
        filed: Vec<FiledRevisionId>,
        next: Option<String>,
    },
    Candidate {
        candidate: Box<ReconformanceCandidate>,
    },
    Rejection {
        proof: Box<RejectionProof>,
    },
    Job {
        job: Box<RatingJob>,
        execution: Box<RatingExecution>,
    },
}
pub struct ReconformanceReader {
    commerce: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    super::super::registration::ReconformanceDefinitions::new(manifest.clone())?;
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("registry_reconformance").map_err(error)?,
        1,
        json!({"format":1,"protocol":"registry_reconformance_v1","service_actor":GOD,
            "scan_rows":SCAN_ROWS,"filed_revision_only":true,"source_bytes":false,
            "account_ids":false}),
    )
    .map(|d| d.with_references(vec![manifest.clone()]))
    .map_err(error)
}
impl ReconformanceReader {
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
    commerce: CommerceFamilies,
    ratings: RatingFamilies,
    manifest: &DefinitionRef,
    expected: RatingCompatibility,
) -> Result<(), ModError> {
    if expected.digest() != manifest.digest() {
        return Err(error("reconformance manifest"));
    }
    let definition = reg.definition(descriptor(manifest)?)?;
    reg.owner_read_service(
        definition,
        BTreeSet::from([
            OwnerId::new("roll").map_err(error)?,
            OwnerId::new("tacsim").map_err(error)?,
        ]),
        BTreeSet::from([OwnerId::new("tacsim").map_err(error)?]),
        ReconformanceReader {
            commerce,
            ratings,
            expected,
        },
    )
}
impl OwnerReadService for ReconformanceReader {
    type State = RollStore;
    type Request = ReconformanceReadRequest;
    type Output = ReconformanceReadResult;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: Self::Request,
    ) -> Result<Self::Output, OwnerReadError> {
        roll.commerce_ready()
            .map_err(|_| OwnerReadError::Unavailable)?;
        if ctx.actor() != GOD
            || ctx
                .world()
                .get(GOD)
                .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        {
            return Err(OwnerReadError::Refused);
        }
        match request {
            ReconformanceReadRequest::Scan { after } => {
                if after
                    .as_deref()
                    .is_some_and(|k| !crate::commerce::families::valid_key("roll_mind_filed", k))
                {
                    return Err(OwnerReadError::Refused);
                }
                let page =
                    ctx.scan_action_state(&self.commerce.filed, after.as_deref(), SCAN_ROWS)?;
                let mut filed = Vec::with_capacity(page.entries().len());
                for entry in page.entries() {
                    let row = entry.value();
                    row.validate().map_err(|_| OwnerReadError::Integrity)?;
                    if row.key() != entry.encoded_key() {
                        return Err(OwnerReadError::Integrity);
                    }
                    filed.push(row.content.id());
                }
                Ok(ReconformanceReadResult::Page {
                    filed,
                    next: page.next_after().map(ToString::to_string),
                })
            }
            ReconformanceReadRequest::Candidate { filed } => {
                let row = ctx
                    .action_state(&self.commerce.filed, &filed.key())?
                    .ok_or(OwnerReadError::Refused)?;
                row.validate().map_err(|_| OwnerReadError::Integrity)?;
                if row.content.id() != filed {
                    return Err(OwnerReadError::Integrity);
                }
                let status = ctx
                    .action_state(&self.commerce.status, &filed.key())?
                    .ok_or(OwnerReadError::Integrity)?;
                status.validate().map_err(|_| OwnerReadError::Integrity)?;
                if status.filed != filed {
                    return Err(OwnerReadError::Integrity);
                }
                if status.recalled_by.is_some() {
                    return Err(OwnerReadError::Refused);
                }
                let birth = self
                    .ratings
                    .resolve_receipt_read(ctx, &row.rating)
                    .map_err(|_| OwnerReadError::Integrity)?;
                if birth.subject().revision() != row.content.revision()
                    || birth.subject().definition() != row.definition
                {
                    return Err(OwnerReadError::Integrity);
                }
                let key = ReconformanceHead::key_for(filed, self.expected.digest())
                    .map_err(|_| OwnerReadError::Integrity)?;
                let previous = match ctx.action_state(&self.commerce.reconformance_heads, &key)? {
                    None => {
                        if birth.compatibility() == &self.expected {
                            return Err(OwnerReadError::Refused);
                        }
                        None
                    }
                    Some(head) => {
                        head.validate().map_err(|_| OwnerReadError::Integrity)?;
                        if head.key() != key
                            || head.filed != filed
                            || head.compatibility != self.expected.digest()
                        {
                            return Err(OwnerReadError::Integrity);
                        }
                        let (job, execution) = self.job(ctx, head.job)?;
                        let binding = execution
                            .admission()
                            .reconformance
                            .as_ref()
                            .ok_or(OwnerReadError::Integrity)?;
                        if binding.filed() != filed
                            || binding.birth_rating() != row.rating
                            || job.compatibility() != &self.expected
                        {
                            return Err(OwnerReadError::Integrity);
                        }
                        if !matches!(
                            job.state(),
                            RatingJobState::Failed { .. }
                                | RatingJobState::Interrupted {}
                                | RatingJobState::Cancelled {}
                        ) {
                            return Err(OwnerReadError::Refused);
                        }
                        Some(head.job)
                    }
                };
                let identity = row.content.revision();
                roll.check_required_root(&RequiredRoot {
                    anchor: RootAnchor::Filed(filed),
                    identity,
                })
                .map_err(|_| OwnerReadError::Integrity)?;
                let reference = identity.revision();
                let revision = roll
                    .revision(reference.key.character, reference.key)
                    .map_err(|_| OwnerReadError::Integrity)?;
                if !identity
                    .matches(revision)
                    .map_err(|_| OwnerReadError::Integrity)?
                {
                    return Err(OwnerReadError::Integrity);
                }
                let binding = ReconformanceAdmission::new(filed, row.rating, previous)
                    .map_err(|_| OwnerReadError::Integrity)?;
                Ok(ReconformanceReadResult::Candidate {
                    candidate: Box::new(ReconformanceCandidate {
                        revision: revision.clone(),
                        binding,
                    }),
                })
            }
            ReconformanceReadRequest::Rejection { filed } => {
                let proof = super::recall::history::resolve(
                    ctx,
                    &self.commerce,
                    &self.ratings,
                    filed,
                    &self.expected,
                )
                .map_err(|_| OwnerReadError::Refused)?;
                if proof.status.recalled_by.is_some() {
                    return Err(OwnerReadError::Refused);
                }
                Ok(ReconformanceReadResult::Rejection {
                    proof: Box::new(proof),
                })
            }
            ReconformanceReadRequest::Job { job } => {
                let (job, execution) = self.job(ctx, job)?;
                Ok(ReconformanceReadResult::Job {
                    job: Box::new(job.clone()),
                    execution: Box::new(execution.clone()),
                })
            }
        }
    }
}
impl ReconformanceReader {
    fn job<'a>(
        &self,
        ctx: &'a OwnerReadContext<'_>,
        id: CommitToken,
    ) -> Result<(&'a RatingJob, &'a RatingExecution), OwnerReadError> {
        let job = ctx
            .action_state(&self.ratings.jobs, &id)?
            .ok_or(OwnerReadError::Refused)?;
        let execution = ctx
            .action_state(&self.ratings.executions, &id)?
            .ok_or(OwnerReadError::Integrity)?;
        execution
            .matches_job(job)
            .map_err(|_| OwnerReadError::Integrity)?;
        if execution.admission().reconformance.is_none() {
            return Err(OwnerReadError::Refused);
        }
        Ok((job, execution))
    }
}
