//! Registered adapters over the actual quota/artifact slots. Their requests are
//! closed Roll values; tickets are private worker/service products. Every image,
//! check and encoding precedes the first owner apply.

use mush_core::action::{PerceivedRef, PrincipalId};
use mush_core::authoritative::{AuthoritativeKey, ReplayWrite};
use mush_core::mechanics::OwnerId;
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{AccountId, DefinitionRef, ObjId, account::PersonalAccountObservation};
use mush_roll::commerce::rating::execution::{ExecutionPhase, RatingExecution};
use mush_roll::commerce::rating::protocol::{
    ArtifactOutput, ArtifactRequest, QuotaOperation, QuotaOutput, QuotaRequest, TicketId,
    Transition,
};
use std::sync::Arc;

use super::{
    artifacts::{Artifacts, Key, Part, PreparedBlob},
    model::{Account, RunId},
    owner::{BulkStore, Prepared},
    publication::account_key,
    registry_projection,
    registry_tickets::{Acknowledged, Ticket, TicketKind},
};

fn refuse(_: impl std::fmt::Display) -> ParticipantError {
    ParticipantError::refused("Registry owner evidence is unavailable or changed")
}

/// Trusted composition supplies actual exact registered references in canonical
/// admit/grant/settle/cancel/recover order, never request-selected definitions.
#[derive(Clone)]
pub(crate) struct Policy {
    actions: [DefinitionRef; 5],
}
impl Policy {
    pub(crate) fn new(actions: [DefinitionRef; 5]) -> Result<Self, ParticipantError> {
        for (reference, expected) in actions.iter().zip([
            Transition::Admit,
            Transition::Grant,
            Transition::Settle,
            Transition::Cancel,
            Transition::Recover,
        ]) {
            if reference.key().owner.as_str() != "roll"
                || reference.key().kind.as_str() != "action"
                || reference.key().id.as_str() != expected.action_id()
                || reference.version() != 1
            {
                return Err(ParticipantError::Registration);
            }
        }
        Ok(Self { actions })
    }
    fn check(
        &self,
        context: &OwnerPrepareContext<'_>,
        transition: Transition,
        character: ObjId,
    ) -> Result<(), ParticipantError> {
        let inv = context.invocation();
        if inv.actor() != character
            || inv.principal() != &(PrincipalId::Object { id: character })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || context.action().key().id.as_str() != transition.action_id()
            || !self.actions.iter().any(|action| action == context.action())
        {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
}

fn personal(
    context: &OwnerPrepareContext<'_>,
    proof: &PersonalAccountObservation,
    account: AccountId,
    character: ObjId,
) -> Result<(), ParticipantError> {
    if proof.requester() != character
        || context.invocation().actor() != character
        || !proof.matches_preparation(context.commit_token())
        || !proof.matches_account(account)
        || context
            .world()
            .module_state::<mush_sim::AccountOracle>()
            .and_then(|oracle| oracle.account_of(character))
            != Some(account)
    {
        return Err(ParticipantError::Authority);
    }
    Ok(())
}
fn utc_now() -> Result<i64, ParticipantError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .ok_or_else(|| ParticipantError::refused("Registry UTC clock is unavailable"))
}
fn ticket(
    context: &OwnerPrepareContext<'_>,
    id: TicketId,
) -> Result<Arc<Ticket>, ParticipantError> {
    context
        .world()
        .module_state::<Artifacts>()
        .ok_or(ParticipantError::Slot)?
        .registry
        .get(id)
        .map_err(refuse)
}
fn check_before(
    store: &BulkStore,
    ticket: &Ticket,
    before: &RatingExecution,
) -> Result<(), ParticipantError> {
    let binding = ticket.binding().ok_or(ParticipantError::Type)?;
    if binding.job != before.job()
        || binding.character != before.admission().character
        || binding.run.0 != before.admission().run
    {
        return Err(ParticipantError::Authority);
    }
    let run = store
        .run(binding.account, binding.character, binding.run)
        .map_err(refuse)?;
    registry_projection::require_before(
        binding.account,
        binding.job,
        run,
        before.admission(),
        before.observation(),
    )
    .map_err(refuse)
}

pub(crate) struct QuotaParticipant {
    policy: Policy,
}
impl QuotaParticipant {
    pub(crate) fn new(policy: Policy) -> Self {
        Self { policy }
    }
}
pub(crate) struct QuotaPatch {
    change: Prepared,
    output: QuotaOutput,
    ticket: Arc<Ticket>,
    proof: Option<PersonalAccountObservation>,
}
pub(crate) struct BoundQuota<'a> {
    state: &'a mut BulkStore,
    patch: QuotaPatch,
}
impl BoundParticipant for BoundQuota<'_> {
    fn apply(self) {
        self.state
            .apply_registry_checked(self.patch.change, self.patch.output.operation());
    }
}
impl OwnerParticipant for QuotaParticipant {
    type State = BulkStore;
    type Request = QuotaRequest;
    type Output = QuotaOutput;
    type Patch = QuotaPatch;
    type Bound<'a> = BoundQuota<'a>;
    fn prepare(
        &self,
        state: &BulkStore,
        context: &OwnerPrepareContext<'_>,
        request: QuotaRequest,
    ) -> Result<OwnerPreparation<QuotaOutput, QuotaPatch>, ParticipantError> {
        let operation = context.commit_token();
        let now = utc_now()?;
        let (id, ticket, change, job, before, subject, proof) = match request.into_operation() {
            QuotaOperation::Admit {
                ticket: id,
                revision,
                account: proof,
            } => {
                let ticket = ticket(context, id)?;
                self.policy
                    .check(context, Transition::Admit, ticket.character())?;
                let TicketKind::Admit(data) = &ticket.kind else {
                    return Err(ParticipantError::Type);
                };
                personal(context, &proof, data.account, data.character)?;
                if data.subject.revision() != revision {
                    return Err(ParticipantError::Authority);
                }
                let (_, change) = state
                    .prepare_registry_admit(
                        data.account,
                        data.character,
                        data.nonce,
                        data.plan.clone(),
                        operation,
                        now,
                    )
                    .map_err(refuse)?;
                let subject = Some((data.subject.clone(), data.compatibility.clone()));
                (id, ticket, change, operation, None, subject, Some(proof))
            }
            QuotaOperation::Grant { ticket: id, before } => {
                let ticket = ticket(context, id)?;
                self.policy
                    .check(context, Transition::Grant, ticket.character())?;
                check_before(state, &ticket, &before)?;
                let TicketKind::Grant {
                    binding,
                    maximum_ms,
                } = &ticket.kind
                else {
                    return Err(ParticipantError::Type);
                };
                let change = state
                    .prepare_grant(binding.account, binding.run, now, *maximum_ms)
                    .map_err(refuse)?;
                let job = before.job();
                (id, ticket, change, job, Some(before), None, None)
            }
            QuotaOperation::Settle { ticket: id, before } => {
                let ticket = ticket(context, id)?;
                self.policy
                    .check(context, Transition::Settle, ticket.character())?;
                check_before(state, &ticket, &before)?;
                let TicketKind::Settle { receipt, .. } = &ticket.kind else {
                    return Err(ParticipantError::Type);
                };
                let change = state.prepare_finish(receipt.clone(), now).map_err(refuse)?;
                let job = before.job();
                (id, ticket, change, job, Some(before), None, None)
            }
            QuotaOperation::Cancel {
                ticket: id,
                before,
                account: proof,
            } => {
                let ticket = ticket(context, id)?;
                self.policy
                    .check(context, Transition::Cancel, ticket.character())?;
                check_before(state, &ticket, &before)?;
                let TicketKind::Cancel(binding) = &ticket.kind else {
                    return Err(ParticipantError::Type);
                };
                personal(context, &proof, binding.account, binding.character)?;
                let change = state
                    .prepare_cancel(binding.account, binding.character, binding.run, now)
                    .map_err(refuse)?;
                let job = before.job();
                (id, ticket, change, job, Some(before), None, Some(proof))
            }
            QuotaOperation::Recover { ticket: id, before } => {
                let ticket = ticket(context, id)?;
                self.policy
                    .check(context, Transition::Recover, ticket.character())?;
                check_before(state, &ticket, &before)?;
                let TicketKind::Recover(binding) = &ticket.kind else {
                    return Err(ParticipantError::Type);
                };
                let change = state
                    .prepare_recover(binding.account, now)
                    .map_err(refuse)?;
                let job = before.job();
                (id, ticket, change, job, Some(before), None, None)
            }
        };
        state.check_registry(&change, operation).map_err(refuse)?;
        let run_id = before
            .as_ref()
            .map(|row| RunId(row.admission().run))
            .or_else(|| change.registry_run(job))
            .ok_or(ParticipantError::Type)?;
        let run = change.resulting_run(run_id).ok_or(ParticipantError::Type)?;
        let output = QuotaOutput::observed(
            operation,
            job,
            ticket.transition(),
            id,
            registry_projection::admission(ticket.account(), job, run).map_err(refuse)?,
            before.as_ref().map(|row| row.observation().clone()),
            registry_projection::observation(run).map_err(refuse)?,
            subject,
        )
        .map_err(refuse)?;
        if let Some(before) = before
            && before.admission() != output.admission()
        {
            return Err(ParticipantError::Authority);
        }
        let writes = vec![ReplayWrite::ModuleJsonSet {
            namespace: OwnerId::new(crate::NS).map_err(refuse)?,
            key: AuthoritativeKey::parse_canonical(account_key(change.account()))
                .map_err(refuse)?,
            value: change.document().map_err(refuse)?,
        }];
        let patch = QuotaPatch {
            change,
            output: output.clone(),
            ticket,
            proof,
        };
        Ok(OwnerPreparation::new(output, patch, writes))
    }
    fn recheck(
        &self,
        state: &BulkStore,
        context: &OwnerPrepareContext<'_>,
        patch: &QuotaPatch,
    ) -> Result<(), ParticipantError> {
        self.policy
            .check(context, patch.output.transition(), patch.ticket.character())?;
        if context.commit_token() != patch.output.operation() {
            return Err(ParticipantError::Authority);
        }
        context
            .world()
            .module_state::<Artifacts>()
            .ok_or(ParticipantError::Slot)?
            .registry
            .check(patch.output.ticket(), &patch.ticket)
            .map_err(refuse)?;
        if let Some(proof) = &patch.proof {
            personal(
                context,
                proof,
                patch.ticket.account(),
                patch.ticket.character(),
            )?;
        }
        state
            .check_registry(&patch.change, patch.output.operation())
            .map_err(refuse)
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut BulkStore,
        patch: QuotaPatch,
    ) -> Result<BoundQuota<'a>, ParticipantError> {
        state
            .check_registry(&patch.change, patch.output.operation())
            .map_err(refuse)?;
        Ok(BoundQuota { state, patch })
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        let ReplayWrite::ModuleJsonSet {
            namespace,
            key,
            value,
        } = write
        else {
            return false;
        };
        if namespace.as_str() != crate::NS {
            return false;
        }
        let Ok(account) = serde_json::from_value::<Account>(value.clone()) else {
            return false;
        };
        key.as_str() == account_key(account.id).as_str()
    }
}

pub(crate) struct ArtifactParticipant {
    policy: Policy,
}
impl ArtifactParticipant {
    pub(crate) fn new(policy: Policy) -> Self {
        Self { policy }
    }
}
pub(crate) struct ArtifactPatch {
    quota: QuotaOutput,
    ticket: Arc<Ticket>,
    blob: Option<PreparedBlob>,
    // Non-blob transitions still bind the actual immutable Start manifest.
    // Rewriting its identical image gives the durable owner a real write while
    // its private ticket is consumed; no new journal/authority is manufactured.
    existing: Option<(Key, serde_json::Value)>,
}
pub(crate) struct BoundArtifacts<'a> {
    state: &'a mut Artifacts,
    patch: ArtifactPatch,
}
impl BoundParticipant for BoundArtifacts<'_> {
    fn apply(self) {
        let ArtifactPatch {
            quota,
            ticket,
            blob,
            existing: _,
        } = self.patch;
        if let Some(blob) = blob {
            self.state.install(blob.manifest);
        }
        self.state.registry.acknowledge_checked(
            &ticket,
            Acknowledged {
                ticket: quota.ticket(),
                operation: quota.operation(),
                job: quota.job(),
                transition: quota.transition(),
            },
        );
    }
}
impl OwnerParticipant for ArtifactParticipant {
    type State = Artifacts;
    type Request = ArtifactRequest;
    type Output = ArtifactOutput;
    type Patch = ArtifactPatch;
    type Bound<'a> = BoundArtifacts<'a>;
    fn prepare(
        &self,
        state: &Artifacts,
        context: &OwnerPrepareContext<'_>,
        request: ArtifactRequest,
    ) -> Result<OwnerPreparation<ArtifactOutput, ArtifactPatch>, ParticipantError> {
        let quota = request.into_quota();
        let ticket = state.registry.get(quota.ticket()).map_err(refuse)?;
        self.policy
            .check(context, quota.transition(), ticket.character())?;
        if context.commit_token() != quota.operation()
            || ticket.transition() != quota.transition()
            || quota.admission().character != ticket.character()
        {
            return Err(ParticipantError::Authority);
        }
        let key = |part| Key {
            account: ticket.account(),
            run: RunId(quota.admission().run),
            part,
        };
        let (blob, measured) = match (&ticket.kind, quota.transition()) {
            (TicketKind::Admit(data), Transition::Admit) => {
                if quota.job() != context.commit_token()
                    || quota.admission().nonce != data.nonce
                    || quota.subject() != Some((&data.subject, &data.compatibility))
                {
                    return Err(ParticipantError::Authority);
                }
                let mut bytes = data.start.clone();
                (
                    Some(
                        PreparedBlob::prepare(key(Part::Start), &mut bytes, vec![])
                            .map_err(refuse)?,
                    ),
                    None,
                )
            }
            (
                TicketKind::Settle {
                    binding,
                    artifact,
                    measured,
                    ..
                },
                Transition::Settle,
            ) => {
                if binding.job != quota.job() || binding.run.0 != quota.admission().run {
                    return Err(ParticipantError::Authority);
                }
                if quota.after().phase == ExecutionPhase::Complete {
                    let mut bytes = artifact.as_ref().ok_or(ParticipantError::Type)?.clone();
                    (
                        Some(
                            PreparedBlob::prepare(key(Part::Result), &mut bytes, vec![])
                                .map_err(refuse)?,
                        ),
                        Some(measured.as_ref().ok_or(ParticipantError::Type)?.clone()),
                    )
                } else {
                    // A cancellation that wins the race may discard an actual
                    // completed worker artifact, but never publishes a receipt.
                    (None, None)
                }
            }
            (TicketKind::Grant { binding, .. }, Transition::Grant)
            | (TicketKind::Cancel(binding), Transition::Cancel)
            | (TicketKind::Recover(binding), Transition::Recover) => {
                if binding.job != quota.job() || binding.run.0 != quota.admission().run {
                    return Err(ParticipantError::Authority);
                }
                (None, None)
            }
            _ => return Err(ParticipantError::Type),
        };
        if let Some(blob) = &blob {
            state.check_new(blob).map_err(refuse)?;
        }
        let output = ArtifactOutput::observed_result(&quota, measured).map_err(refuse)?;
        let mut writes = blob
            .as_ref()
            .map(|blob| {
                blob.writes
                    .iter()
                    .map(replay_write)
                    .collect::<Result<Vec<_>, ParticipantError>>()
            })
            .transpose()?
            .unwrap_or_default();
        let existing = if quota.transition() != Transition::Admit {
            let key = key(Part::Start);
            let manifest = state.get(key).map_err(refuse)?;
            if manifest.artifact.sha256 != *quota.admission().start.sha256.as_bytes()
                || manifest.artifact.bytes != quota.admission().start.bytes
            {
                return Err(ParticipantError::Authority);
            }
            let value = serde_json::to_value(manifest).map_err(refuse)?;
            if blob.is_none() {
                writes.push(ReplayWrite::ModuleJsonSet {
                    namespace: OwnerId::new(crate::NS).map_err(refuse)?,
                    key: AuthoritativeKey::parse_canonical(key.document()).map_err(refuse)?,
                    value: value.clone(),
                });
            }
            Some((key, value))
        } else {
            None
        };
        Ok(OwnerPreparation::new(
            output,
            ArtifactPatch {
                quota,
                ticket,
                blob,
                existing,
            },
            writes,
        ))
    }
    fn recheck(
        &self,
        state: &Artifacts,
        context: &OwnerPrepareContext<'_>,
        patch: &ArtifactPatch,
    ) -> Result<(), ParticipantError> {
        self.policy
            .check(context, patch.quota.transition(), patch.ticket.character())?;
        if context.commit_token() != patch.quota.operation() {
            return Err(ParticipantError::Authority);
        }
        check_artifacts(state, patch)
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut Artifacts,
        patch: ArtifactPatch,
    ) -> Result<BoundArtifacts<'a>, ParticipantError> {
        check_artifacts(state, &patch)?;
        Ok(BoundArtifacts { state, patch })
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        super::artifacts::accepts_registry_write(write)
    }
}
fn check_artifacts(state: &Artifacts, patch: &ArtifactPatch) -> Result<(), ParticipantError> {
    state
        .registry
        .check(patch.quota.ticket(), &patch.ticket)
        .map_err(refuse)?;
    if let Some(blob) = &patch.blob {
        state.check_new(blob).map_err(refuse)?;
    }
    if let Some((key, value)) = &patch.existing
        && serde_json::to_value(state.get(*key).map_err(refuse)?).map_err(refuse)? != *value
    {
        return Err(ParticipantError::Authority);
    }
    Ok(())
}
fn replay_write(write: &mush_core::ModuleWrite) -> Result<ReplayWrite, ParticipantError> {
    match write {
        mush_core::ModuleWrite::JsonSet { key, value } => Ok(ReplayWrite::ModuleJsonSet {
            namespace: OwnerId::new(crate::NS).map_err(refuse)?,
            key: AuthoritativeKey::parse_canonical(key).map_err(refuse)?,
            value: value.clone(),
        }),
        _ => Err(ParticipantError::WriteFamily),
    }
}

#[cfg(test)]
#[path = "registry_participant_tests.rs"]
mod tests;

impl mush_roll::commerce::rating::protocol::RegistryQuotaParticipant for QuotaParticipant {
    fn audit_stopped_jobs(
        ctx: &mush_core::ModuleCtx<'_>,
        executions: &[RatingExecution],
    ) -> Result<(), mush_roll::commerce::rating::RatingError> {
        use mush_roll::commerce::rating::RatingError;
        ctx.state::<BulkStore>()
            .ok_or(RatingError::Unavailable)?
            .audit_registry_boot(executions)
            .map_err(|_| RatingError::Incomplete)
    }

    fn project_selector(
        engine: &mush_core::Engine,
        query: &mush_core::LegalActionQuery,
        transition: Transition,
    ) -> Result<
        Option<mush_roll::commerce::rating::protocol::RegistrySelector>,
        mush_roll::commerce::rating::RatingError,
    > {
        use mush_roll::commerce::rating::RatingError;
        if query.principal() != &(mush_core::PrincipalId::Object { id: query.actor() })
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
            || engine.world.get(query.actor()).is_none_or(|o| {
                o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
            })
        {
            return Ok(None);
        }
        let owner = OwnerId::new(crate::NS).map_err(|_| RatingError::Invalid)?;
        engine
            .registry
            .require_mechanics_owner_ready(&owner)
            .map_err(|_| RatingError::Unavailable)?;
        let result = engine
            .world
            .module_state::<Artifacts>()
            .ok_or(RatingError::Unavailable)?
            .registry
            .selector(query.actor(), transition)
            .map_err(|_| RatingError::Unavailable)?;
        engine
            .registry
            .require_mechanics_owner_ready(&owner)
            .map_err(|_| RatingError::Unavailable)?;
        Ok(result)
    }
}
