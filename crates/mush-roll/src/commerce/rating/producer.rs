//! The closed typed Registry producer. It consumes only the outputs of the
//! exact registered quota/artifact handles, then derives all typed post-images
//! and retention from those same observations. No measurement is a wire value.

use mush_core::action::{
    ActionError, ActionInvocation, ActionProvider, AuthoritativeActionOutput, PerceivedRef,
    PrincipalId,
};
use mush_core::action_state::{ActionTxn, StateFamilyHandle};
use mush_core::mechanics::{DefinitionRef, RulesValue};
use mush_core::participant::{OwnerParticipant, ParticipantHandle};
use mush_core::{CommitToken, PrepareCtx};
use serde::Deserialize;
use serde_json::json;

use super::execution::{ExecutionPhase, RatingExecution};
use super::protocol::{
    ArtifactOutput, ArtifactRequest, QuotaOutput, QuotaRequest, TicketId, Transition,
};
use super::reconformance::{
    ReconformanceHead,
    reader::{ReconformanceReadRequest, ReconformanceReadResult, ReconformanceReader},
};
use super::{
    RatingCompatibility, RatingCompletion, RatingError, RatingFailure, RatingFamilies, RatingJob,
    RatingJobState, RatingPage, RatingPublication, RatingReceipt, RatingStatus,
};
use crate::commerce::families::CommerceFamilies;
use crate::commerce::participant::{RootParticipant, RootRequest};
use crate::commerce::rating_roots::RatingRoots;
use crate::commerce::records::{Record, schema_line_digest};
use crate::commerce::{FiledRevisionId, RevisionIdentity};
use crate::{RevisionKey, RevisionRef, RollStore};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionWire {
    ticket: u64,
    mind: u64,
    revision: u64,
    revision_digest: mush_core::DefinitionDigest,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReconformanceWire {
    ticket: u64,
    filed: FiledRevisionId,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransitionWire {
    ticket: u64,
    job: CommitToken,
}
enum Request {
    Reconform {
        ticket: TicketId,
        filed: FiledRevisionId,
    },
    Admit {
        ticket: TicketId,
        revision: RevisionRef,
    },
    Existing {
        ticket: TicketId,
        job: CommitToken,
    },
}

struct PreparedSelection {
    quota_request: QuotaRequest,
    old: Option<(RatingJob, RatingExecution)>,
    marked: Option<crate::commerce::marked_rating::MarkedRatingAccess>,
    reconformance: Option<super::reconformance::ReconformanceAdmission>,
}

/// This type and its constructor fields are private to Roll. Trusted module
/// composition supplies concrete owner types; clients cannot replace a handle,
/// owner output, current manifest, or typed family.
pub(crate) struct RegistryBuilder<Q, A>
where
    Q: super::protocol::RegistryQuotaParticipant,
    A: OwnerParticipant<Request = ArtifactRequest, Output = ArtifactOutput>,
{
    pub(crate) action: DefinitionRef,
    pub(crate) outcome: DefinitionRef,
    pub(crate) manifest: DefinitionRef,
    pub(crate) compatibility: RatingCompatibility,
    pub(crate) transition: Transition,
    pub(crate) commerce: CommerceFamilies,
    pub(crate) ratings: RatingFamilies,
    pub(crate) quota: ParticipantHandle<Q>,
    pub(crate) artifacts: ParticipantHandle<A>,
    pub(crate) roots: ParticipantHandle<RootParticipant>,
    pub(crate) marked: Option<
        mush_core::read_service::OwnerReadHandle<
            crate::commerce::marked_rating::MarkedRatingReader,
        >,
    >,
    pub(crate) reconformance: Option<mush_core::read_service::OwnerReadHandle<ReconformanceReader>>,
    pub(crate) refusal: ActionError,
}

fn invalid<T>(_: T) -> RatingError {
    RatingError::Invalid
}

impl<Q, A> RegistryBuilder<Q, A>
where
    Q: super::protocol::RegistryQuotaParticipant,
    A: OwnerParticipant<Request = ArtifactRequest, Output = ArtifactOutput>,
{
    fn request(&self, inv: &ActionInvocation) -> Result<Request, RatingError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner != self.action.key().owner
        {
            return Err(RatingError::Invalid);
        }
        if self.reconformance.is_some() && inv.actor() != mush_core::GOD {
            return Err(RatingError::Identity);
        }
        if self.transition == Transition::Admit && self.reconformance.is_some() {
            let wire: ReconformanceWire =
                serde_json::from_value(inv.parameters().clone()).map_err(invalid)?;
            return Ok(Request::Reconform {
                ticket: TicketId::new(wire.ticket)?,
                filed: wire.filed,
            });
        }
        if self.transition == Transition::Admit {
            let wire: AdmissionWire =
                serde_json::from_value(inv.parameters().clone()).map_err(invalid)?;
            if wire.mind == 0 || wire.revision == 0 {
                return Err(RatingError::Invalid);
            }
            Ok(Request::Admit {
                ticket: TicketId::new(wire.ticket)?,
                revision: RevisionRef {
                    key: RevisionKey {
                        character: inv.actor(),
                        mind: wire.mind,
                        number: wire.revision,
                    },
                    digest: wire.revision_digest,
                },
            })
        } else {
            let wire: TransitionWire =
                serde_json::from_value(inv.parameters().clone()).map_err(invalid)?;
            Ok(Request::Existing {
                ticket: TicketId::new(wire.ticket)?,
                job: wire.job,
            })
        }
    }

    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, RatingError> {
        // Fixed phase names aid diagnostics without source or refusal payloads.
        let mut stage = "request";
        let result = (|| {
            let request = self.request(inv)?;
            // Active composition and the all-loaded source/root audit are required
            // for every transition. Initialize registers no producer or worker.
            context
                .world()
                .map_err(invalid)?
                .module_state::<RollStore>()
                .ok_or(RatingError::Unavailable)?
                .commerce_ready()
                .map_err(invalid)?;
            let deployment = txn
                .get(&self.commerce.deployment, &"registry".to_owned())
                .map_err(invalid)?
                .ok_or(RatingError::Unavailable)?;
            deployment.validate().map_err(invalid)?;

            let PreparedSelection {
                quota_request,
                old,
                marked,
                reconformance,
            } = *self.select_quota(request, inv, txn, context)?;
            // These are capabilities for the actual registered slots. Structural
            // constructors on an output type are not an alternate input to here.
            stage = "quota";
            let quota = context
                .prepare_owner(&self.quota, quota_request)
                .map_err(invalid)?;
            if quota.operation() != context.commit_token()
                || quota.transition() != self.transition
                || quota.admission().character != inv.actor()
                || quota.admission().reconformance.is_some() != self.reconformance.is_some()
                || !accepts_compatibility(
                    self.transition,
                    &self.compatibility,
                    quota.admission().compatibility,
                )
            {
                return Err(RatingError::Identity);
            }
            stage = "artifact";
            let artifacts = context
                .prepare_owner(&self.artifacts, ArtifactRequest::from_quota(&quota))
                .map_err(invalid)?;
            if !artifacts.matches(&quota)? {
                return Err(RatingError::Identity);
            }
            let commit = context.commit_token();
            let published = match old {
                None => self.publish_admission(
                    txn,
                    context,
                    &quota,
                    artifacts,
                    (marked, reconformance),
                    commit,
                )?,
                Some((old_job, before)) => self.publish_existing(
                    txn,
                    context,
                    &quota,
                    artifacts,
                    (commit, old_job, before),
                    &mut stage,
                )?,
            };
            self.finish_publication(txn, context, inv, commit, published, &mut stage)
        })();
        result.inspect_err(
            |_| tracing::debug!(target:"registry_preparation",stage,"Registry preparation refused"),
        )
    }

    #[inline(never)]
    fn finish_publication(
        &self,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
        inv: &ActionInvocation,
        commit: CommitToken,
        published: Box<(RatingJob, RatingExecution, RatingRoots)>,
        stage: &mut &'static str,
    ) -> Result<AuthoritativeActionOutput, RatingError> {
        let (job, execution, mut roots) = *published;
        if self.reconformance.is_some() {
            roots = roots.with_reconformance(self.action.clone(), self.manifest.clone())?;
        }
        // Read through the exact typed post-images before asking the final
        // participant to reserve/exchange source roots in the same commit.
        *stage = "readback";
        exact(txn, &self.ratings.jobs, &job.job(), &job)?;
        exact(txn, &self.ratings.executions, &job.job(), &execution)?;
        if let Some(admission) = execution.admission().reconformance.as_ref() {
            let filed = txn
                .get(&self.commerce.filed, &admission.filed().key())
                .map_err(invalid)?
                .cloned()
                .ok_or(RatingError::Incomplete)?;
            let receipt = match job.state() {
                RatingJobState::Completed { receipt } => Some(
                    txn.get(&self.ratings.receipts, &receipt.job())
                        .map_err(invalid)?
                        .cloned()
                        .ok_or(RatingError::Incomplete)?,
                ),
                _ => None,
            };
            let change = crate::commerce::follow::Change::certificate(
                &filed,
                &job,
                &execution,
                receipt.as_ref(),
            )
            .map_err(invalid)?;
            roots = roots.with_follow(change);
        }
        *stage = "root-owner";
        context
            .prepare_owner(&self.roots, RootRequest::Rating(Box::new(roots)))
            .map_err(invalid)?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({"job":job.job(),"operation":commit,"transition":self.transition.action_id()}),
        )
        .map_err(invalid)?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![self.outcome.clone(), self.manifest.clone()])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }

    #[inline(never)]
    fn select_quota(
        &self,
        request: Request,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<Box<PreparedSelection>, RatingError> {
        let (quota_request, old, marked, reconformance) = match request {
            Request::Reconform { ticket, filed } => {
                let handle = self
                    .reconformance
                    .as_ref()
                    .ok_or(RatingError::Unavailable)?;
                let result = context
                    .read_owner_preimage(txn, handle, ReconformanceReadRequest::Candidate { filed })
                    .map_err(invalid)?;
                let ReconformanceReadResult::Candidate { candidate } = result else {
                    return Err(RatingError::Identity);
                };
                let identity = RevisionIdentity::capture(candidate.revision()).map_err(invalid)?;
                let proof = context.personal_account().map_err(invalid)?;
                (
                    QuotaRequest::admit(ticket, identity, proof),
                    None,
                    None,
                    Some(candidate.binding().clone()),
                )
            }
            Request::Admit { ticket, revision } => {
                let exact = context
                    .world()
                    .map_err(invalid)?
                    .module_state::<RollStore>()
                    .ok_or(RatingError::Unavailable)?
                    .revision(inv.actor(), revision.key)
                    .map_err(invalid)?;
                if exact.reference() != revision {
                    return Err(RatingError::Identity);
                }
                let identity = RevisionIdentity::capture(exact).map_err(invalid)?;
                let marked = if exact.require_original_access().is_ok() {
                    None
                } else {
                    let handle = self.marked.as_ref().ok_or(RatingError::Unavailable)?;
                    let access = context
                        .read_owner_preimage(
                            txn,
                            handle,
                            crate::commerce::marked_rating::MarkedRatingRequest::new(revision),
                        )
                        .map_err(invalid)?;
                    if access.actor() != inv.actor()
                        || RevisionIdentity::capture(access.revision()).map_err(invalid)?
                            != identity
                    {
                        return Err(RatingError::Identity);
                    }
                    Some(access)
                };
                let proof = context.personal_account().map_err(invalid)?;
                (
                    QuotaRequest::admit(ticket, identity, proof),
                    None,
                    marked,
                    None,
                )
            }
            Request::Existing { ticket, job } => {
                let old_job = txn
                    .get(&self.ratings.jobs, &job)
                    .map_err(invalid)?
                    .cloned()
                    .ok_or(RatingError::Unavailable)?;
                let execution = txn
                    .get(&self.ratings.executions, &job)
                    .map_err(invalid)?
                    .cloned()
                    .ok_or(RatingError::Unavailable)?;
                execution.matches_job(&old_job)?;
                if execution.admission().character != inv.actor()
                    || execution.admission().reconformance.is_some() != self.reconformance.is_some()
                    || (self.reconformance.is_none()
                        && old_job.subject().revision().revision().key.character != inv.actor())
                    || !accepts_compatibility(
                        self.transition,
                        &self.compatibility,
                        old_job.compatibility().digest(),
                    )
                    || execution.observation().phase.terminal()
                {
                    return Err(RatingError::Identity);
                }
                let request = match self.transition {
                    Transition::Grant => QuotaRequest::grant(ticket, execution.clone()),
                    Transition::Settle => QuotaRequest::settle(ticket, execution.clone()),
                    Transition::Cancel => QuotaRequest::cancel(
                        ticket,
                        execution.clone(),
                        context.personal_account().map_err(invalid)?,
                    ),
                    Transition::Recover => QuotaRequest::recover(ticket, execution.clone()),
                    Transition::Admit => return Err(RatingError::Invalid),
                };
                (request, Some((old_job, execution)), None, None)
            }
        };
        Ok(Box::new(PreparedSelection {
            quota_request,
            old,
            marked,
            reconformance,
        }))
    }

    #[inline(never)]
    fn publish_admission(
        &self,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
        quota: &QuotaOutput,
        artifacts: ArtifactOutput,
        selected: (
            Option<crate::commerce::marked_rating::MarkedRatingAccess>,
            Option<super::reconformance::ReconformanceAdmission>,
        ),
        commit: CommitToken,
    ) -> Result<Box<(RatingJob, RatingExecution, RatingRoots)>, RatingError> {
        let (marked, reconformance) = selected;
        if artifacts.into_measured().is_some() {
            return Err(RatingError::Identity);
        }
        if quota.admission().reconformance != reconformance {
            return Err(RatingError::Identity);
        }
        let (subject, compatibility) = quota.subject().ok_or(RatingError::Incomplete)?;
        let exact_revision = context
            .world()
            .map_err(invalid)?
            .module_state::<RollStore>()
            .ok_or(RatingError::Unavailable)?
            .revision(
                subject.revision().revision().key.character,
                subject.revision().revision().key,
            )
            .map_err(invalid)?;
        let envelope = super::digest(
            "tacsim.registry.envelope.v1",
            exact_revision.envelope(),
            crate::MAX_ENVELOPE_BYTES,
        )?;
        if quota.job() != commit
            || compatibility != &self.compatibility
            || subject.revision() != RevisionIdentity::capture(exact_revision).map_err(invalid)?
            || subject.envelope() != envelope
            || subject.schema() != schema_line_digest(exact_revision.envelope()).map_err(invalid)?
        {
            return Err(RatingError::Identity);
        }
        let job = RatingJob::new(
            commit,
            subject.clone(),
            compatibility.clone(),
            quota.admission().plan,
        );
        let execution =
            RatingExecution::admit(&job, quota.admission().clone(), quota.after().clone())?;
        txn.insert(&self.ratings.jobs, &commit, job.clone())
            .map_err(invalid)?;
        txn.insert(&self.ratings.executions, &commit, execution.clone())
            .map_err(invalid)?;
        if let Some(binding) = &reconformance {
            let head = ReconformanceHead {
                format: 1,
                filed: binding.filed(),
                compatibility: self.compatibility.digest(),
                job: commit,
                admitted_at_utc: context.world().map_err(invalid)?.now(),
            };
            head.validate().map_err(invalid)?;
            let key = head.key();
            let previous = txn
                .get(&self.commerce.reconformance_heads, &key)
                .map_err(invalid)?;
            if previous.map(|h| h.job) != binding.previous_attempt() {
                return Err(RatingError::Identity);
            }
            if previous.is_some() {
                txn.replace(&self.commerce.reconformance_heads, &key, head.clone())
                    .map_err(invalid)?;
            } else {
                txn.insert(&self.commerce.reconformance_heads, &key, head.clone())
                    .map_err(invalid)?;
            }
            exact(txn, &self.commerce.reconformance_heads, &key, &head)?;
        }
        let roots = match marked {
            Some(access) => {
                RatingRoots::admission_marked(&job, commit, self.action.clone(), access)?
            }
            None => RatingRoots::admission(&job, commit)?,
        };
        context
            .reserve_registry_rating_completion(txn)
            .map_err(invalid)?;
        Ok(Box::new((job, execution, roots)))
    }

    #[inline(never)]
    fn publish_existing(
        &self,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
        quota: &QuotaOutput,
        artifacts: ArtifactOutput,
        previous: (CommitToken, RatingJob, RatingExecution),
        stage: &mut &'static str,
    ) -> Result<Box<(RatingJob, RatingExecution, RatingRoots)>, RatingError> {
        let (commit, old_job, before) = previous;
        if quota.job() != old_job.job()
            || quota.admission() != before.admission()
            || quota.before() != Some(before.observation())
            || quota.subject().is_some()
        {
            return Err(RatingError::Identity);
        }
        *stage = "result-publication";
        let mut rejection = None;
        let completion = match artifacts.into_measured() {
            Some(super::nonconformance::MeasuredRegistryResult::Certified(measured)) => {
                let issued = context.world().map_err(invalid)?.world_ic_now();
                Some(measured.publish_parts(RatingPublication::new(old_job.job(), issued)?)?)
            }
            Some(super::nonconformance::MeasuredRegistryResult::Nonconforming(measured)) => {
                let issued = context.world().map_err(invalid)?.world_ic_now();
                rejection =
                    Some(measured.publish_parts(RatingPublication::new(old_job.job(), issued)?)?);
                None
            }
            None => None,
        };
        let receipt = completion.as_ref().map(|(receipt, _)| receipt);
        let rejected = rejection.as_ref().map(|(receipt, _)| receipt);
        *stage = "job-transition";
        let job = resulting_job(&old_job, quota, receipt, rejected)?;
        *stage = "observation";
        let execution =
            before.observed(before.observation(), quota.after().clone(), &job, commit)?;
        if let Some((receipt, pages)) = &completion {
            execution.verify_completion(&job, receipt, pages)?;
            self.stage_completion(txn, &execution, receipt, pages)?;
        }
        if let Some((receipt, pages)) = &rejection {
            *stage = "rejection-verify";
            execution.verify_nonconformance(&job, receipt, pages)?;
            *stage = "rejection-insert";
            txn.insert(&self.ratings.nonconformance, &job.job(), receipt.clone())
                .map_err(invalid)?;
            *stage = "rejection-readback";
            exact(txn, &self.ratings.nonconformance, &job.job(), receipt)?;
            *stage = "rejection-pages";
            for page in pages {
                let key = page.key();
                txn.insert(&self.ratings.pages, &key, page.clone())
                    .map_err(invalid)?;
                exact(txn, &self.ratings.pages, &key, page)?;
            }
        }
        *stage = "job-replace";
        txn.replace(&self.ratings.jobs, &job.job(), job.clone())
            .map_err(invalid)?;
        txn.replace(&self.ratings.executions, &job.job(), execution.clone())
            .map_err(invalid)?;
        *stage = "root-delta";
        let roots = if job == old_job {
            RatingRoots::settled_progress(&job, commit)?
        } else {
            RatingRoots::transition(&old_job, &job, receipt, commit)?
        };
        *stage = "capacity";
        if quota.after().phase.terminal() {
            context
                .finish_registry_rating_completion(txn, job.job())
                .map_err(invalid)?;
        }
        Ok(Box::new((job, execution, roots)))
    }

    fn stage_completion(
        &self,
        txn: &mut ActionTxn<'_>,
        execution: &RatingExecution,
        receipt: &RatingReceipt,
        pages: &[RatingPage],
    ) -> Result<(), RatingError> {
        let job = execution.job();
        let completion = RatingCompletion::for_execution(receipt, execution)?;
        let status = RatingStatus::active(receipt)?;
        txn.insert(&self.ratings.receipts, &job, receipt.clone())
            .map_err(invalid)?;
        txn.insert(&self.ratings.completions, &job, completion.clone())
            .map_err(invalid)?;
        txn.insert(&self.ratings.status, &job, status.clone())
            .map_err(invalid)?;
        for page in pages {
            let key = page.key();
            txn.insert(&self.ratings.pages, &key, page.clone())
                .map_err(invalid)?;
            exact(txn, &self.ratings.pages, &key, page)?;
        }
        exact(txn, &self.ratings.receipts, &job, receipt)?;
        exact(txn, &self.ratings.completions, &job, &completion)?;
        exact(txn, &self.ratings.status, &job, &status)
    }
}

fn exact<K, V>(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<K, V>,
    key: &K,
    expected: &V,
) -> Result<(), RatingError>
where
    K: Send + Sync + 'static,
    V: Clone + PartialEq + Send + Sync + 'static,
{
    if txn.get(family, key).map_err(invalid)? != Some(expected) {
        return Err(RatingError::Identity);
    }
    Ok(())
}

// Stopped-worker recovery never certifies or executes old inputs. It preserves
// the exact acknowledged compatibility while releasing a pending computation.
// Admission and continuing/settling work still require the current manifest.
fn accepts_compatibility(
    transition: Transition,
    current: &RatingCompatibility,
    captured: mush_core::DefinitionDigest,
) -> bool {
    transition == Transition::Recover || captured == current.digest()
}

fn resulting_job(
    old: &RatingJob,
    quota: &QuotaOutput,
    receipt: Option<&RatingReceipt>,
    rejected: Option<&super::nonconformance::RatingNonconformance>,
) -> Result<RatingJob, RatingError> {
    use ExecutionPhase as Phase;
    if (receipt.is_some() && rejected.is_some())
        || (quota.after().phase == Phase::Complete) != (receipt.is_some() || rejected.is_some())
    {
        return Err(RatingError::Incomplete);
    }
    let next = match (quota.transition(), quota.after().phase) {
        (Transition::Grant, Phase::Running) => RatingJobState::Running {
            lease_sequence: quota
                .after()
                .next_sequence
                .checked_sub(1)
                .ok_or(RatingError::Bound)?,
        },
        (Transition::Settle, Phase::Running) | (Transition::Cancel, Phase::CancelRequested) => {
            if !matches!(old.state(), RatingJobState::Running { .. }) {
                return Err(RatingError::Identity);
            }
            return Ok(old.clone());
        }
        (Transition::Settle, Phase::Complete) => match (receipt, rejected) {
            (Some(receipt), None) => RatingJobState::Completed {
                receipt: receipt.reference()?,
            },
            (None, Some(receipt)) if quota.admission().measured_rejection => {
                RatingJobState::Nonconforming {
                    receipt: receipt.reference()?,
                }
            }
            _ => return Err(RatingError::Incomplete),
        },
        (Transition::Settle, Phase::Failed) => RatingJobState::Failed {
            reason: RatingFailure::Execution,
        },
        (Transition::Settle | Transition::Cancel, Phase::Cancelled) => RatingJobState::Cancelled {},
        (Transition::Recover, Phase::Interrupted) => RatingJobState::Interrupted {},
        _ => return Err(RatingError::Invalid),
    };
    old.transition(next)
}

impl<Q, A> ActionProvider for RegistryBuilder<Q, A>
where
    Q: super::protocol::RegistryQuotaParticipant,
    A: OwnerParticipant<Request = ArtifactRequest, Output = ArtifactOutput>,
{
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let selector = if self.reconformance.is_some() {
            Q::project_reconformance_selector(engine, query, self.transition)
        } else {
            Q::project_selector(engine, query, self.transition)
        };
        let Some(selector) = selector.map_err(|_| self.refusal.clone())? else {
            return Ok(None);
        };
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| self.refusal.clone())?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| self.refusal.clone())?;
        guards
            .add_definition(self.action.clone(), definitions)
            .map_err(|_| self.refusal.clone())?;
        guards
            .add_definition(self.manifest.clone(), definitions)
            .map_err(|_| self.refusal.clone())?;
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: selector.parameters(),
            parameters: mush_core::ProjectedParameterSchema::new(vec![])
                .map_err(|_| self.refusal.clone())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: vec![],
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "registry-continuation",
                "Continue owned Registry work",
                "Continue the already selected bounded Registry operation.",
            )
            .map_err(|_| self.refusal.clone())?,
            guards: guards.finish().map_err(|_| self.refusal.clone())?,
        }))
    }
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.request(inv)
            .map(|_| ())
            .map_err(|_| self.refusal.clone())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare(inv, txn, context)
            .map_err(|_| self.refusal.clone())
    }
}

#[cfg(test)]
#[path = "producer_tests.rs"]
mod tests;
