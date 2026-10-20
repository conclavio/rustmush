//! Private fixed producer. A ticket selects actual owner-held work; all typed
//! records below derive from the exact two registered native outputs.
use super::{protocol::*, publication::Roots, records::*, *};
use crate::{
    RollStore,
    commerce::{
        FiledRevisionId,
        families::CommerceFamilies,
        participant::{RootParticipant, RootRequest},
        rating::{RatingCompatibility, RatingFamilies},
        records::{Record, canonical_duplicate_digest, schema_line_digest},
        roots::{RequiredRoot, RootAnchor},
    },
};
use mush_core::{
    CommitToken, DefinitionRef, Flags, GOD, ObjKind, PrepareCtx,
    action::{
        ActionError, ActionInvocation, ActionProvider, AuthoritativeActionOutput, PerceivedRef,
        PrincipalId,
    },
    action_state::{ActionTxn, StateFamilyHandle},
    mechanics::RulesValue,
    participant::{OwnerParticipant, ParticipantHandle},
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    ticket: u64,
    filed: FiledRevisionId,
    profile: String,
    role: ConductRole,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Existing {
    ticket: u64,
    job: CommitToken,
}
enum Request {
    Admit(Admission),
    Existing(Existing),
}
pub(crate) struct Builder<
    Q: ConductQuotaParticipant,
    A: OwnerParticipant<Request = ArtifactRequest, Output = ArtifactOutput>,
> {
    pub(crate) action: DefinitionRef,
    pub(crate) outcome: DefinitionRef,
    pub(crate) manifest: DefinitionRef,
    pub(crate) transition: Transition,
    pub(crate) expected: RatingCompatibility,
    pub(crate) families: CommerceFamilies,
    pub(crate) ratings: RatingFamilies,
    pub(crate) quota: ParticipantHandle<Q>,
    pub(crate) artifacts: ParticipantHandle<A>,
    pub(crate) roots: ParticipantHandle<RootParticipant>,
    pub(crate) refusal: ActionError,
}
struct Selected {
    request: QuotaRequest,
    before: Option<(ConductJob, ConductExecution)>,
    subject: Option<ConductSubject>,
}

impl<
    Q: ConductQuotaParticipant,
    A: OwnerParticipant<Request = ArtifactRequest, Output = ArtifactOutput>,
> Builder<Q, A>
{
    fn request(&self, inv: &ActionInvocation) -> Result<Request, PublicationError> {
        if inv.actor() != GOD
            || inv.principal() != &(PrincipalId::Object { id: GOD })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner != self.action.key().owner
        {
            return Err(PublicationError::Invalid);
        }
        if self.transition == Transition::Admit {
            serde_json::from_value(inv.parameters().clone())
                .map(Request::Admit)
                .map_err(invalid)
        } else {
            serde_json::from_value(inv.parameters().clone())
                .map(Request::Existing)
                .map_err(invalid)
        }
    }
    #[inline(never)]
    fn select(
        &self,
        request: Request,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<Box<Selected>, PublicationError> {
        let world = ctx.world().map_err(invalid)?;
        if world
            .get(GOD)
            .is_none_or(|p| p.kind != ObjKind::Player || p.flags.contains(Flags::GOING))
        {
            return Err(PublicationError::Unavailable);
        }
        let roll = world
            .module_state::<RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        roll.commerce_ready().map_err(invalid)?;
        let deployment = txn
            .get(&self.families.deployment, &"registry".to_owned())
            .map_err(invalid)?
            .ok_or(PublicationError::Unavailable)?;
        deployment.validate()?;
        let (request, before, subject) = match request {
            Request::Admit(request) => {
                let filed = txn
                    .get(&self.families.filed, &request.filed.key())
                    .map_err(invalid)?
                    .cloned()
                    .ok_or(PublicationError::Missing)?;
                filed.validate()?;
                let line = txn
                    .get(&self.families.lines, &filed.content.position().line.key())
                    .map_err(invalid)?
                    .cloned()
                    .ok_or(PublicationError::Missing)?;
                line.validate()?;
                let status = txn
                    .get(&self.families.status, &request.filed.key())
                    .map_err(invalid)?
                    .cloned()
                    .ok_or(PublicationError::Missing)?;
                status.validate()?;
                if status.filed != request.filed
                    || status.withdrawn_by.is_some()
                    || status.recalled_by.is_some()
                    || line.charter != filed.content.position().charter
                {
                    return Err(PublicationError::Unavailable);
                }
                let receipt = crate::commerce::rating::reconformance::current::receipt_in_txn(
                    txn,
                    &self.families,
                    &self.ratings,
                    &filed,
                    &self.expected,
                )
                .map_err(invalid)?;
                let reference = filed.content.revision().revision();
                let revision = roll
                    .revision(reference.key.character, reference.key)
                    .map_err(invalid)?;
                if !filed
                    .content
                    .revision()
                    .matches(revision)
                    .map_err(invalid)?
                    || schema_line_digest(revision.envelope())? != line.schema_digest
                    || canonical_duplicate_digest(revision)? != filed.duplicate
                {
                    return Err(PublicationError::Mismatch);
                }
                for root in [
                    RequiredRoot {
                        anchor: RootAnchor::Filed(request.filed),
                        identity: filed.content.revision(),
                    },
                    RequiredRoot {
                        anchor: RootAnchor::Rating {
                            job: receipt.publication().job(),
                            id: receipt.id(),
                        },
                        identity: filed.content.revision(),
                    },
                ] {
                    roll.check_required_root(&root).map_err(invalid)?;
                }
                let subject = ConductSubject::captured(
                    line.charter,
                    filed.content.position().line,
                    request.filed,
                    filed.content.revision(),
                    receipt.reference().map_err(invalid)?,
                    receipt.subject().clone(),
                    request.profile,
                    request.role,
                )?;
                let account = ctx.personal_account().map_err(invalid)?;
                (
                    QuotaRequest::admit(TicketId::new(request.ticket)?, subject.clone(), account),
                    None,
                    Some(subject),
                )
            }
            Request::Existing(request) => {
                let job = txn
                    .get(&self.families.conduct_jobs, &request.job.to_hex())
                    .map_err(invalid)?
                    .cloned()
                    .ok_or(PublicationError::Missing)?;
                let execution = txn
                    .get(&self.families.conduct_executions, &request.job.to_hex())
                    .map_err(invalid)?
                    .cloned()
                    .ok_or(PublicationError::Missing)?;
                execution.matches_job(&job)?;
                if execution.observation().phase.terminal()
                    || txn
                        .get(&self.families.conduct_results, &request.job.to_hex())
                        .map_err(invalid)?
                        .is_some()
                    || (!matches!(self.transition, Transition::Cancel | Transition::Recover)
                        && job.manifest() != &self.manifest)
                {
                    return Err(PublicationError::Unavailable);
                }
                Q::check_history(
                    world,
                    ConductHistoryRow {
                        job: &job,
                        execution: &execution,
                        result: None,
                    },
                )?;
                let request = QuotaRequest::existing(
                    TicketId::new(request.ticket)?,
                    job.admission().clone(),
                    execution.clone(),
                );
                (request, Some((job, execution)), None)
            }
        };
        Ok(Box::new(Selected {
            request,
            before,
            subject,
        }))
    }

    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, PublicationError> {
        let request = self.request(inv)?;
        let Selected {
            request,
            before,
            subject,
        } = *self.select(request, txn, ctx)?;
        let quota = ctx.prepare_owner(&self.quota, request).map_err(invalid)?;
        let commit = ctx.commit_token();
        if quota.operation() != commit
            || quota.transition() != self.transition
            || quota.admission().character() != GOD
        {
            return Err(PublicationError::Mismatch);
        }
        let artifact = ctx
            .prepare_owner(&self.artifacts, ArtifactRequest::from_quota(&quota))
            .map_err(invalid)?;
        artifact.matches(&quota)?;
        let measurement = artifact.into_measurement();
        let job = match &before {
            None => {
                if quota.job() != commit
                    || quota.before().is_some()
                    || quota.subject() != subject.as_ref()
                    || measurement.is_some()
                {
                    return Err(PublicationError::Mismatch);
                }
                ConductJob::observed(
                    commit,
                    self.action.clone(),
                    self.manifest.clone(),
                    subject.ok_or(PublicationError::Missing)?,
                    quota.admission().clone(),
                    quota.at_utc(),
                )?
            }
            Some((job, execution)) => {
                if quota.job() != job.operation()
                    || quota.admission() != job.admission()
                    || quota.before() != Some(execution.observation())
                    || quota.subject().is_some()
                {
                    return Err(PublicationError::Mismatch);
                }
                job.clone()
            }
        };
        let execution = ConductExecution {
            format: 1,
            job: job.operation(),
            updated_by: commit,
            action: self.action.clone(),
            observation: quota.after().clone(),
        };
        execution.matches_job(&job)?;
        let key = job.operation().to_hex();
        if before.is_none() {
            txn.insert(&self.families.conduct_jobs, &key, job.clone())
                .map_err(invalid)?;
            txn.insert(&self.families.conduct_executions, &key, execution.clone())
                .map_err(invalid)?;
        } else {
            txn.replace(&self.families.conduct_executions, &key, execution.clone())
                .map_err(invalid)?;
        }
        if execution.observation().phase.terminal() {
            let result = ConductResult {
                format: 1,
                job: job.operation(),
                operation: commit,
                action: self.action.clone(),
                job_digest: job.digest()?,
                execution_digest: execution.digest()?,
                phase: execution.observation().phase,
                artifact: execution.observation().result.clone(),
                measurement,
            };
            result.matches(&job, &execution)?;
            txn.insert(&self.families.conduct_results, &key, result.clone())
                .map_err(invalid)?;
            exact(txn, &self.families.conduct_results, &key, &result)?;
        } else if measurement.is_some() {
            return Err(PublicationError::Mismatch);
        }
        exact(txn, &self.families.conduct_jobs, &key, &job)?;
        exact(txn, &self.families.conduct_executions, &key, &execution)?;
        if before.is_none() {
            ctx.reserve_conduct_trial_completion(txn).map_err(invalid)?;
        } else {
            ctx.advance_conduct_trial_completion(txn, job.operation())
                .map_err(invalid)?;
        }
        let roll = ctx
            .world()
            .map_err(invalid)?
            .module_state::<RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        let roots = Roots::new(
            roll,
            self.action.clone(),
            commit,
            quota.at_utc(),
            &job,
            before,
        )?;
        ctx.prepare_owner(&self.roots, RootRequest::Conduct(Box::new(roots)))
            .map_err(invalid)?;
        let outcome = RulesValue::new(self.outcome.clone(),serde_json::json!({"job":job.operation(),"operation":commit,"transition":self.transition})).map_err(invalid)?;
        let definitions = if matches!(self.transition, Transition::Cancel | Transition::Recover) {
            vec![self.outcome.clone()]
        } else {
            vec![self.outcome.clone(), self.manifest.clone()]
        };
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(definitions)
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
fn exact<R: Clone + PartialEq + Send + Sync + 'static>(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<String, R>,
    key: &String,
    expected: &R,
) -> Result<(), PublicationError> {
    if txn.get(family, key).map_err(invalid)? == Some(expected) {
        Ok(())
    } else {
        Err(PublicationError::Mismatch)
    }
}
impl<
    Q: ConductQuotaParticipant,
    A: OwnerParticipant<Request = ArtifactRequest, Output = ArtifactOutput>,
> ActionProvider for Builder<Q, A>
{
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        if action != &self.action {
            return Ok(None);
        }
        let Some(selector) = Q::project_selector(engine, query, self.transition)
            .map_err(|_| self.refusal.clone())?
        else {
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
        if !matches!(self.transition, Transition::Cancel | Transition::Recover) {
            guards
                .add_definition(self.manifest.clone(), definitions)
                .map_err(|_| self.refusal.clone())?;
        }
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: selector.parameters(),
            parameters: mush_core::ProjectedParameterSchema::new(vec![])
                .map_err(|_| self.refusal.clone())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: vec![],
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "conduct-continuation",
                "Continue owned Conduct work",
                "Continue the selected bounded role trial.",
            )
            .map_err(|_| self.refusal.clone())?,
            guards: guards.finish().map_err(|_| self.refusal.clone())?,
        }))
    }
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        self.request(invocation)
            .map(|_| ())
            .map_err(|_| self.refusal.clone())
    }
    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare(invocation, txn, ctx)
            .map_err(|_| self.refusal.clone())
    }
}
