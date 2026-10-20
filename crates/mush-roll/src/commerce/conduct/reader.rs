//! Opaque owner-created readback. Metadata is immutable provenance; current
//! selection additionally walks the complete current certificate exactly once.
use super::{history, records::*, state::Native, *};
use crate::{
    RollStore,
    commerce::{
        CharterId, FiledRevisionId, LineId, RevisionIdentity, families::CommerceFamilies,
        rating::RatingRef, read::CommerceRead, roots::RequiredRoot,
    },
};
use mush_core::{
    CommitToken, DefinitionRef, Flags, ModError, ModuleCtx, ObjKind, Registrar,
    action::PrincipalId,
    mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    },
    participant::OwnerPrepareContext,
    read_service::{OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService},
};
use std::{any::TypeId, collections::BTreeSet, sync::Arc};

#[derive(Serialize)]
pub struct ConductReadRequest {
    operation: CommitToken,
    current: bool,
}
impl ConductReadRequest {
    pub const fn metadata(operation: CommitToken) -> Self {
        Self {
            operation,
            current: false,
        }
    }
    pub const fn current(operation: CommitToken) -> Self {
        Self {
            operation,
            current: true,
        }
    }
}

#[derive(Clone, Serialize)]
pub struct ConductTrialProof {
    job: ConductJob,
    execution: ConductExecution,
    result: ConductResult,
    digest: DefinitionDigest,
    result_digest: DefinitionDigest,
    measurement_digest: DefinitionDigest,
    #[serde(skip)]
    native: Arc<dyn Native>,
    #[serde(skip)]
    current: Option<(u64, u64)>,
}
impl std::fmt::Debug for ConductTrialProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConductTrialProof")
            .field("operation", &self.operation())
            .field("digest", &self.digest)
            .finish()
    }
}
impl ConductTrialProof {
    pub fn operation(&self) -> CommitToken {
        self.job.operation()
    }
    pub fn charter(&self) -> CharterId {
        self.job.subject().charter()
    }
    pub fn line(&self) -> LineId {
        self.job.subject().line()
    }
    pub fn filed(&self) -> FiledRevisionId {
        self.job.subject().filed()
    }
    pub fn revision(&self) -> RevisionIdentity {
        self.job.subject().revision()
    }
    pub fn rating(&self) -> RatingRef {
        self.job.subject().rating()
    }
    pub fn profile(&self) -> &str {
        self.job.subject().profile()
    }
    pub fn role(&self) -> ConductRole {
        self.job.subject().role()
    }
    pub const fn digest(&self) -> DefinitionDigest {
        self.digest
    }
    pub fn result_digest(&self) -> DefinitionDigest {
        self.result_digest
    }
    pub fn measurement_digest(&self) -> DefinitionDigest {
        // Construction requires a real complete passed result. Returning the
        // already-validated digest avoids a new serialization callback here.
        self.measurement_digest
    }
    pub fn manifest(&self) -> &DefinitionRef {
        self.job.manifest()
    }
    pub(crate) fn root(&self) -> Result<RequiredRoot, PublicationError> {
        history::job_root(&self.job)
    }

    /// Refusal-only comparison. A clone does not acquire current eligibility.
    pub fn check_history(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        let world = ctx.world();
        if world
            .module_state_owner(TypeId::of::<RollStore>())
            .is_none_or(|o| o.as_str() != crate::NS)
        {
            return Err(PublicationError::Unavailable);
        }
        let roll = world
            .module_state::<RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        roll.commerce_ready().map_err(invalid)?;
        self.native.check(
            world,
            super::protocol::ConductHistoryRow {
                job: &self.job,
                execution: &self.execution,
                result: Some(&self.result),
            },
        )?;
        for root in [history::job_root(&self.job)?, self.root()?] {
            roll.check_required_root(&root).map_err(invalid)?;
        }
        Ok(())
    }
    /// The complete certificate was read in the actual preparation's committed
    /// typed preimage. Core repeats its exact revision fence; all current Roll
    /// publication mutations also change this retained owner generation.
    pub fn check_current(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        self.check_history(ctx)?;
        let expected = self.current.ok_or(PublicationError::Unavailable)?;
        let roll = ctx
            .world()
            .module_state::<RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        if roll.conduct_stamp().map_err(invalid)? != expected
            || self.manifest() != &roll.conduct.configuration()?.manifest
        {
            return Err(PublicationError::Stale);
        }
        Ok(())
    }
}

pub(super) fn proof(
    ctx: &impl CommerceRead,
    roll: &RollStore,
    f: &CommerceFamilies,
    operation: CommitToken,
    complete_certificate: bool,
) -> Result<ConductTrialProof, PublicationError> {
    let config = roll.conduct.configuration()?;
    let job = history::row(ctx, &f.conduct_jobs, operation.to_hex())?;
    let execution = history::row(ctx, &f.conduct_executions, operation.to_hex())?;
    let result = history::row(ctx, &f.conduct_results, operation.to_hex())?;
    result.matches(job, execution)?;
    if !result.qualifies() {
        return Err(PublicationError::Unavailable);
    }
    history::subject(ctx, roll, f, config, job, complete_certificate)?;
    config.native.check(
        ctx.world(),
        super::protocol::ConductHistoryRow {
            job,
            execution,
            result: Some(result),
        },
    )?;
    let result_digest = result.digest()?;
    let measurement_digest = result
        .measurement()
        .ok_or(PublicationError::Missing)?
        .digest();
    let digest = super::digest(
        "roll.conduct.receipt.v1",
        &(job.digest()?, result_digest, measurement_digest),
        4096,
    )?;
    Ok(ConductTrialProof {
        job: job.clone(),
        execution: execution.clone(),
        result: result.clone(),
        digest,
        native: config.native.clone(),
        current: None,
        result_digest,
        measurement_digest,
    })
}

pub struct ConductReader {
    families: CommerceFamilies,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    super::protocol::action_descriptor(manifest, super::protocol::Transition::Admit)
        .map_err(error)?;
    DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?, DefinitionId::new("conduct_trial").map_err(error)?, 1,
        serde_json::json!({"format":1,"protocol":"conduct_trial_read_v1","history":"actual_native_typed_roots","current":"complete_selected_certificate","manifest":manifest,"source_bytes":false}))
        .map(|d| d.with_references(vec![manifest.clone()])).map_err(error)
}
impl ConductReader {
    pub fn request(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
    ) -> Result<OwnerReadHandle<Self>, ModError> {
        let definition = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                descriptor(manifest)?,
            )
            .map_err(error)?;
        reg.request_owner_read(definition)
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
        BTreeSet::from([
            OwnerId::new("roll").map_err(error)?,
            OwnerId::new("tacsim").map_err(error)?,
        ]),
        BTreeSet::from([OwnerId::new("tacsim").map_err(error)?]),
        ConductReader { families },
    )
}
impl OwnerReadService for ConductReader {
    type State = RollStore;
    type Request = ConductReadRequest;
    type Output = ConductTrialProof;
    fn read(
        &self,
        roll: &RollStore,
        ctx: &OwnerReadContext<'_>,
        request: Self::Request,
    ) -> Result<Self::Output, OwnerReadError> {
        read_proof(roll, ctx, &self.families, request)
    }
}

// Both registered readers use the same actual history and current-certificate checks.
pub(super) fn read_proof(
    roll: &RollStore,
    ctx: &OwnerReadContext<'_>,
    families: &CommerceFamilies,
    request: ConductReadRequest,
) -> Result<ConductTrialProof, OwnerReadError> {
    if ctx.principal() != &(PrincipalId::Object { id: ctx.actor() })
        || ctx
            .world()
            .get(ctx.actor())
            .is_none_or(|p| p.kind != ObjKind::Player || p.flags.contains(Flags::GOING))
    {
        return Err(OwnerReadError::Authority);
    }
    roll.commerce_ready()
        .map_err(|_| OwnerReadError::Unavailable)?;
    let mut proof = proof(ctx, roll, families, request.operation, false)
        .map_err(|_| OwnerReadError::Refused)?;
    if request.current {
        let config = roll
            .conduct
            .configuration()
            .map_err(|_| OwnerReadError::Unavailable)?;
        let filed = history::row(ctx, &families.filed, proof.filed().key())
            .map_err(|_| OwnerReadError::Integrity)?;
        let status = history::row(ctx, &families.status, proof.filed().key())
            .map_err(|_| OwnerReadError::Integrity)?;
        if status.filed != proof.filed()
            || status.withdrawn_by.is_some()
            || status.recalled_by.is_some()
            || proof.manifest() != &config.manifest
        {
            return Err(OwnerReadError::Refused);
        }
        let receipt = crate::commerce::rating::reconformance::current::receipt(
            ctx,
            families,
            &config.ratings,
            filed,
            &config.expected,
        )
        .map_err(|_| OwnerReadError::Refused)?;
        if receipt.reference().map_err(|_| OwnerReadError::Integrity)? != proof.rating()
            || receipt.subject() != proof.job.subject().worker()
        {
            return Err(OwnerReadError::Refused);
        }
        proof.current = Some(
            roll.conduct_stamp()
                .map_err(|_| OwnerReadError::Unavailable)?,
        );
    }
    Ok(proof)
}
