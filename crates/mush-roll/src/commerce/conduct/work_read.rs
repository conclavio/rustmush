//! Private-service capture and exact acknowledgment readback. The source
//! revision is exposed only to the registered Tacsim worker under actual GOD
//! dispatch; it does not grant customer source or qualification rights.
use super::{history, protocol::ConductHistoryRow, records::*, *};
use crate::{
    Revision, RollStore,
    commerce::{
        FiledRevisionId,
        families::CommerceFamilies,
        records::{Record, canonical_duplicate_digest, schema_line_digest},
        roots::{RequiredRoot, RootAnchor},
    },
};
use mush_core::{
    CommitToken, DefinitionRef, Flags, GOD, ModError, ObjKind, Registrar,
    action::PrincipalId,
    mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    },
    read_service::{OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService},
};
use std::collections::BTreeSet;

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConductWorkRequest {
    Candidate {
        filed: FiledRevisionId,
        profile: String,
        role: ConductRole,
    },
    Job {
        operation: CommitToken,
    },
    Scan {
        after: Option<String>,
    },
    CurrentProof {
        operation: CommitToken,
    },
}
#[derive(Serialize)]
pub struct ConductCandidate {
    revision: Revision,
    subject: ConductSubject,
    manifest: DefinitionRef,
}
impl ConductCandidate {
    pub fn revision(&self) -> &Revision {
        &self.revision
    }
    pub fn subject(&self) -> &ConductSubject {
        &self.subject
    }
    pub fn manifest(&self) -> &DefinitionRef {
        &self.manifest
    }
    pub fn into_parts(self) -> (Revision, ConductSubject, DefinitionRef) {
        (self.revision, self.subject, self.manifest)
    }
}
#[derive(Serialize)]
pub struct ConductWorkState {
    job: ConductJob,
    execution: ConductExecution,
    result: Option<ConductResult>,
}
impl ConductWorkState {
    pub fn job(&self) -> &ConductJob {
        &self.job
    }
    pub fn execution(&self) -> &ConductExecution {
        &self.execution
    }
    pub fn result(&self) -> Option<&ConductResult> {
        self.result.as_ref()
    }
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConductWorkRead {
    Proof {
        proof: ConductTrialProof,
    },
    Candidate {
        candidate: ConductCandidate,
    },
    Job {
        state: ConductWorkState,
    },
    Page {
        operations: Vec<CommitToken>,
        next: Option<String>,
    },
}
pub struct ConductWorkReader {
    families: CommerceFamilies,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    protocol::action_descriptor(manifest, protocol::Transition::Admit).map_err(error)?;
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new("conduct_work").map_err(error)?,
        1,
        serde_json::json!({"format":1,"protocol":"conduct_work_v1","service_actor":GOD,
            "scan_rows":16,"filed_revision_only":true,"source_bytes":false,
            "actual_native_readback":true,"current_qualification_readback":true,"manifest":manifest}),
    )
    .map(|d| d.with_references(vec![manifest.clone()]))
    .map_err(error)
}
impl ConductWorkReader {
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
    fn state(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        operation: CommitToken,
    ) -> Result<ConductWorkState, OwnerReadError> {
        let job = ctx
            .action_state(&self.families.conduct_jobs, &operation.to_hex())?
            .ok_or(OwnerReadError::Refused)?;
        let execution = ctx
            .action_state(&self.families.conduct_executions, &operation.to_hex())?
            .ok_or(OwnerReadError::Integrity)?;
        let result = ctx.action_state(&self.families.conduct_results, &operation.to_hex())?;
        execution
            .matches_job(job)
            .map_err(|_| OwnerReadError::Integrity)?;
        if let Some(result) = result {
            result
                .matches(job, execution)
                .map_err(|_| OwnerReadError::Integrity)?;
        }
        if result.is_some() != execution.observation().phase.terminal() {
            return Err(OwnerReadError::Integrity);
        }
        let config = roll
            .conduct
            .configuration()
            .map_err(|_| OwnerReadError::Unavailable)?;
        history::subject(ctx, roll, &self.families, config, job, false)
            .map_err(|_| OwnerReadError::Integrity)?;
        config
            .native
            .check(
                ctx.world(),
                ConductHistoryRow {
                    job,
                    execution,
                    result,
                },
            )
            .map_err(|_| OwnerReadError::Integrity)?;
        Ok(ConductWorkState {
            job: job.clone(),
            execution: execution.clone(),
            result: result.cloned(),
        })
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    manifest: &DefinitionRef,
) -> Result<(), ModError> {
    let definition = reg.definition(descriptor(manifest)?)?;
    reg.owner_read_service(
        definition,
        BTreeSet::from([OwnerId::new("tacsim").map_err(error)?]),
        BTreeSet::from([OwnerId::new("tacsim").map_err(error)?]),
        ConductWorkReader { families },
    )
}
impl OwnerReadService for ConductWorkReader {
    type State = RollStore;
    type Request = ConductWorkRequest;
    type Output = ConductWorkRead;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: Self::Request,
    ) -> Result<Self::Output, OwnerReadError> {
        roll.commerce_ready()
            .map_err(|_| OwnerReadError::Unavailable)?;
        if ctx.actor() != GOD
            || ctx.principal() != &(PrincipalId::Object { id: GOD })
            || ctx
                .world()
                .get(GOD)
                .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        {
            return Err(OwnerReadError::Authority);
        }
        match request {
            ConductWorkRequest::CurrentProof { operation } => Ok(ConductWorkRead::Proof {
                proof: super::reader::read_proof(
                    roll,
                    ctx,
                    &self.families,
                    ConductReadRequest::current(operation),
                )?,
            }),
            ConductWorkRequest::Job { operation } => Ok(ConductWorkRead::Job {
                state: self.state(roll, ctx, operation)?,
            }),
            ConductWorkRequest::Scan { after } => {
                if after
                    .as_ref()
                    .is_some_and(|k| CommitToken::from_hex(k).is_err())
                {
                    return Err(OwnerReadError::Refused);
                }
                let page =
                    ctx.scan_action_state(&self.families.conduct_jobs, after.as_deref(), 16)?;
                let mut operations = Vec::with_capacity(page.entries().len());
                for entry in page.entries() {
                    entry
                        .value()
                        .validate()
                        .map_err(|_| OwnerReadError::Integrity)?;
                    operations.push(entry.value().operation());
                }
                Ok(ConductWorkRead::Page {
                    operations,
                    next: page.next_after().map(|s| s.to_string()),
                })
            }
            ConductWorkRequest::Candidate {
                filed,
                profile,
                role,
            } => {
                let config = roll
                    .conduct
                    .configuration()
                    .map_err(|_| OwnerReadError::Unavailable)?;
                let row = ctx
                    .action_state(&self.families.filed, &filed.key())?
                    .ok_or(OwnerReadError::Refused)?;
                row.validate().map_err(|_| OwnerReadError::Integrity)?;
                let line = ctx
                    .action_state(&self.families.lines, &row.content.position().line.key())?
                    .ok_or(OwnerReadError::Integrity)?;
                let status = ctx
                    .action_state(&self.families.status, &filed.key())?
                    .ok_or(OwnerReadError::Integrity)?;
                line.validate().map_err(|_| OwnerReadError::Integrity)?;
                status.validate().map_err(|_| OwnerReadError::Integrity)?;
                if row.content.id() != filed
                    || status.filed != filed
                    || line.charter != row.content.position().charter
                    || status.withdrawn_by.is_some()
                    || status.recalled_by.is_some()
                {
                    return Err(OwnerReadError::Refused);
                }
                let receipt = crate::commerce::rating::reconformance::current::receipt(
                    ctx,
                    &self.families,
                    &config.ratings,
                    row,
                    &config.expected,
                )
                .map_err(|_| OwnerReadError::Refused)?;
                let identity = row.content.revision();
                let reference = identity.revision();
                let revision = roll
                    .revision(reference.key.character, reference.key)
                    .map_err(|_| OwnerReadError::Integrity)?;
                if !identity
                    .matches(revision)
                    .map_err(|_| OwnerReadError::Integrity)?
                    || schema_line_digest(revision.envelope())
                        .map_err(|_| OwnerReadError::Integrity)?
                        != line.schema_digest
                    || canonical_duplicate_digest(revision)
                        .map_err(|_| OwnerReadError::Integrity)?
                        != row.duplicate
                {
                    return Err(OwnerReadError::Integrity);
                }
                for anchor in [
                    RootAnchor::Filed(filed),
                    RootAnchor::Rating {
                        job: receipt.publication().job(),
                        id: receipt.id(),
                    },
                ] {
                    roll.check_required_root(&RequiredRoot { anchor, identity })
                        .map_err(|_| OwnerReadError::Integrity)?;
                }
                let subject = ConductSubject::captured(
                    line.charter,
                    row.content.position().line,
                    filed,
                    identity,
                    receipt.reference().map_err(|_| OwnerReadError::Integrity)?,
                    receipt.subject().clone(),
                    profile,
                    role,
                )
                .map_err(|_| OwnerReadError::Refused)?;
                Ok(ConductWorkRead::Candidate {
                    candidate: ConductCandidate {
                        revision: revision.clone(),
                        subject,
                        manifest: config.manifest.clone(),
                    },
                })
            }
        }
    }
}
