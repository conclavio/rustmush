//! Actual quota and artifact owners for acknowledged Conduct work. Requests
//! resolve private worker tickets; decoded observations never authorize writes.
use super::{
    artifacts::Artifacts,
    conduct_projection,
    conduct_tickets::{Ticket, TicketKind},
    model::{Account, RunId, utc_day},
    owner::{BulkStore, Prepared},
    publication::account_key,
};
use mush_core::action::{PerceivedRef, PrincipalId};
use mush_core::authoritative::{AuthoritativeKey, ReplayWrite};
use mush_core::mechanics::OwnerId;
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    AccountId, DefinitionDigest, DefinitionRef, ObjId, account::PersonalAccountObservation,
};
use mush_roll::commerce::conduct::protocol::{
    QuotaOperation, QuotaOutput, QuotaRequest, TicketId, Transition,
};
use std::sync::Arc;

fn refuse(_: impl std::fmt::Display) -> ParticipantError {
    ParticipantError::refused("Conduct owner evidence is unavailable or changed")
}
fn utc_now() -> Result<i64, ParticipantError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|t| i64::try_from(t.as_millis()).ok())
        .ok_or_else(|| refuse("UTC"))
}
#[derive(Clone)]
pub(crate) struct Policy {
    manifest: DefinitionRef,
    policy: DefinitionDigest,
    actions: [DefinitionRef; 5],
}
impl Policy {
    pub(crate) fn new(
        manifest: DefinitionRef,
        policy: DefinitionDigest,
        actions: [DefinitionRef; 5],
    ) -> Result<Self, ParticipantError> {
        for (actual, phase) in actions.iter().zip(Transition::ALL) {
            if *actual
                != mush_roll::commerce::conduct::protocol::action_ref(&manifest, phase)
                    .map_err(refuse)?
            {
                return Err(ParticipantError::Registration);
            }
        }
        Ok(Self {
            manifest,
            policy,
            actions,
        })
    }
    fn check(
        &self,
        context: &OwnerPrepareContext<'_>,
        phase: Transition,
        character: ObjId,
    ) -> Result<(), ParticipantError> {
        let inv = context.invocation();
        let expected = self
            .actions
            .iter()
            .zip(Transition::ALL)
            .find(|(_, p)| *p == phase)
            .map(|(r, _)| r)
            .ok_or(ParticipantError::Registration)?;
        if character != mush_core::GOD
            || inv.actor() != character
            || inv.principal() != &(PrincipalId::Object { id: character })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || context.action() != expected
            || context.world().get(character).is_none_or(|o| {
                o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
            })
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
) -> Result<(), ParticipantError> {
    if proof.requester() != mush_core::GOD
        || !proof.matches_preparation(context.commit_token())
        || !proof.matches_account(account)
    {
        return Err(ParticipantError::Authority);
    }
    current_account(context, account)
}
fn current_account(
    context: &OwnerPrepareContext<'_>,
    account: AccountId,
) -> Result<(), ParticipantError> {
    if context
        .world()
        .module_state::<mush_sim::AccountOracle>()
        .and_then(|a| a.account_of(mush_core::GOD))
        != Some(account)
    {
        Err(ParticipantError::Authority)
    } else {
        Ok(())
    }
}
fn ticket(
    context: &OwnerPrepareContext<'_>,
    id: TicketId,
) -> Result<Arc<Ticket>, ParticipantError> {
    context
        .world()
        .module_state::<Artifacts>()
        .ok_or(ParticipantError::Slot)?
        .conduct
        .get(id)
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
            .apply_conduct_checked(self.patch.change, self.patch.output.operation());
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
                subject,
                account: proof,
            } => {
                let ticket = ticket(context, id)?;
                self.policy
                    .check(context, Transition::Admit, ticket.character())?;
                let TicketKind::Admit(data) = &ticket.kind else {
                    return Err(ParticipantError::Type);
                };
                personal(context, &proof, data.account)?;
                let budget = data.plan.conduct.as_ref().ok_or(ParticipantError::Type)?;
                if data.subject != subject || budget.policy != self.policy.policy {
                    return Err(ParticipantError::Authority);
                }
                let (_, change) = state
                    .prepare_conduct_admit(
                        data.account,
                        data.character,
                        data.nonce,
                        data.plan.clone(),
                        operation,
                        now,
                    )
                    .map_err(refuse)?;
                (
                    id,
                    ticket,
                    change,
                    operation,
                    None,
                    Some(subject),
                    Some(proof),
                )
            }
            QuotaOperation::Existing {
                ticket: id,
                admission,
                before,
            } => {
                let ticket = ticket(context, id)?;
                let phase = ticket.transition();
                self.policy.check(context, phase, ticket.character())?;
                if phase == Transition::Cancel
                    && before.observation().phase
                        == mush_roll::commerce::rating::execution::ExecutionPhase::CancelRequested
                {
                    return Err(ParticipantError::Authority);
                }
                let binding = ticket.binding().ok_or(ParticipantError::Type)?;
                if binding.job != before.job()
                    || binding.run.0 != admission.run()
                    || binding.character != admission.character()
                {
                    return Err(ParticipantError::Authority);
                }
                conduct_projection::require_before(
                    binding.account,
                    binding.job,
                    state
                        .run(binding.account, binding.character, binding.run)
                        .map_err(refuse)?,
                    &admission,
                    before.observation(),
                )
                .map_err(refuse)?;
                if matches!(phase, Transition::Grant | Transition::Settle)
                    && admission.policy() != self.policy.policy
                {
                    return Err(ParticipantError::Authority);
                }
                let change = match &ticket.kind {
                    TicketKind::Grant {
                        binding,
                        maximum_ms,
                    } => {
                        current_account(context, binding.account)?;
                        state.prepare_grant(binding.account, binding.run, now, *maximum_ms)
                    }
                    TicketKind::Settle { receipt, .. } => {
                        state.prepare_finish(receipt.clone(), now)
                    }
                    TicketKind::Cancel(binding) => {
                        state.prepare_cancel(binding.account, binding.character, binding.run, now)
                    }
                    TicketKind::Recover(binding) => state.prepare_conduct_recover(*binding, now),
                    TicketKind::Admit(_) => return Err(ParticipantError::Type),
                }
                .map_err(refuse)?;
                (
                    id,
                    ticket,
                    change,
                    binding.job,
                    Some((admission, before)),
                    None,
                    None,
                )
            }
        };
        state.check_conduct(&change, operation).map_err(refuse)?;
        let run_id = before
            .as_ref()
            .map(|(a, _)| RunId(a.run()))
            .or_else(|| change.conduct_run(job))
            .ok_or(ParticipantError::Type)?;
        let run = change.resulting_run(run_id).ok_or(ParticipantError::Type)?;
        let output = QuotaOutput::observed(
            operation,
            job,
            ticket.transition(),
            id,
            conduct_projection::admission(ticket.account(), job, run).map_err(refuse)?,
            before.as_ref().map(|(_, r)| r.observation().clone()),
            conduct_projection::observation(run).map_err(refuse)?,
            subject,
            context.world().now(),
        )
        .map_err(refuse)?;
        if before
            .as_ref()
            .is_some_and(|(a, _)| a != output.admission())
        {
            return Err(ParticipantError::Authority);
        }
        let write = ReplayWrite::ModuleJsonSet {
            namespace: OwnerId::new(crate::NS).map_err(refuse)?,
            key: AuthoritativeKey::parse_canonical(account_key(change.account()))
                .map_err(refuse)?,
            value: change.document().map_err(refuse)?,
        };
        Ok(OwnerPreparation::new(
            output.clone(),
            QuotaPatch {
                change,
                output,
                ticket,
                proof,
            },
            vec![write],
        ))
    }
    fn recheck(
        &self,
        state: &BulkStore,
        context: &OwnerPrepareContext<'_>,
        patch: &QuotaPatch,
    ) -> Result<(), ParticipantError> {
        self.policy
            .check(context, patch.output.transition(), patch.ticket.character())?;
        if context.commit_token() != patch.output.operation()
            || context.world().now() < patch.output.at_utc()
        {
            return Err(ParticipantError::Authority);
        }
        context
            .world()
            .module_state::<Artifacts>()
            .ok_or(ParticipantError::Slot)?
            .conduct
            .check(patch.output.ticket(), &patch.ticket)
            .map_err(refuse)?;
        if let Some(proof) = &patch.proof {
            personal(context, proof, patch.ticket.account())?;
        }
        if patch.output.transition() == Transition::Grant {
            current_account(context, patch.ticket.account())?;
            let day = utc_day(utc_now()?);
            if patch
                .output
                .after()
                .flight
                .as_ref()
                .is_none_or(|f| f.utc_day != day)
            {
                return Err(ParticipantError::Authority);
            }
        }
        state
            .check_conduct(&patch.change, patch.output.operation())
            .map_err(refuse)
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut BulkStore,
        patch: QuotaPatch,
    ) -> Result<BoundQuota<'a>, ParticipantError> {
        state
            .check_conduct(&patch.change, patch.output.operation())
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
        namespace.as_str() == crate::NS
            && serde_json::from_value::<Account>(value.clone())
                .is_ok_and(|a| key.as_str() == account_key(a.id).as_str())
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
    blob: Option<super::artifacts::PreparedBlob>,
    existing: Option<(super::artifacts::Key, serde_json::Value)>,
    admission: Option<super::conduct_index::PreparedAdmission>,
    retained: Option<super::conduct_index::Entry>,
    history_before: Option<super::conduct_history::Evidence>,
    history_after: super::conduct_history::Evidence,
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
            admission,
            retained: _,
            history_before: _,
            history_after,
        } = self.patch;
        if let Some(admission) = admission {
            self.state.conduct_index.install(admission);
        }
        if let Some(blob) = blob {
            self.state.install(blob.manifest);
        }
        self.state
            .conduct_history
            .install(quota.job(), history_after);
        self.state.conduct.acknowledge_checked(
            &ticket,
            super::conduct_tickets::Acknowledged {
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
    type Request = mush_roll::commerce::conduct::protocol::ArtifactRequest;
    type Output = mush_roll::commerce::conduct::protocol::ArtifactOutput;
    type Patch = ArtifactPatch;
    type Bound<'a> = BoundArtifacts<'a>;
    fn prepare(
        &self,
        state: &Artifacts,
        context: &OwnerPrepareContext<'_>,
        request: Self::Request,
    ) -> Result<OwnerPreparation<Self::Output, Self::Patch>, ParticipantError> {
        use super::artifacts::{Key, Part, PreparedBlob};
        use mush_roll::commerce::{
            conduct::{protocol::ArtifactOutput, records::ConductJob},
            rating::execution::ExecutionPhase,
        };
        let quota = request.into_quota();
        let ticket = state.conduct.get(quota.ticket()).map_err(refuse)?;
        self.policy
            .check(context, quota.transition(), ticket.character())?;
        if context.commit_token() != quota.operation()
            || context.world().now() != quota.at_utc()
            || ticket.transition() != quota.transition()
            || quota.admission().character() != ticket.character()
        {
            return Err(ParticipantError::Authority);
        }
        let key = |part| Key {
            account: ticket.account(),
            run: RunId(quota.admission().run()),
            part,
        };
        let mut admission = None;
        let mut retained = None;
        state.conduct_history.require().map_err(refuse)?;
        let mut history_before = None;
        let mut history_after = None;
        if quota.transition() != Transition::Admit {
            let entry = state.conduct_index.get(quota.job()).map_err(refuse)?;
            if entry.account() != ticket.account()
                || entry.job().admission() != quota.admission()
                || matches!(quota.transition(), Transition::Grant | Transition::Settle)
                    && entry.job().manifest() != &self.policy.manifest
            {
                return Err(ParticipantError::Authority);
            }
            let evidence = state.conduct_history.get(entry.job()).map_err(refuse)?;
            history_before = Some(evidence.clone());
            history_after = Some(evidence.clone());
            retained = Some(entry.clone());
        }
        let (blob, measurement) = match (&ticket.kind, quota.transition()) {
            (TicketKind::Admit(data), Transition::Admit) => {
                if quota.job() != context.commit_token()
                    || quota.admission().nonce() != data.nonce
                    || quota.subject() != Some(&data.subject)
                    || quota.admission().policy() != self.policy.policy
                    || quota.admission().start().sha256.as_bytes()
                        != &data.start.bytes().descriptor().sha256
                    || quota.admission().start().bytes != data.start.bytes().descriptor().bytes
                {
                    return Err(ParticipantError::Authority);
                }
                require_terminal_artifact_room(ticket.account(), RunId(quota.admission().run()))?;
                let job = ConductJob::observed(
                    quota.job(),
                    context.action().clone(),
                    self.policy.manifest.clone(),
                    data.subject.clone(),
                    quota.admission().clone(),
                    quota.at_utc(),
                )
                .map_err(refuse)?;
                data.start.matches(&job, &data.plan).map_err(refuse)?;
                history_after =
                    Some(super::conduct_history::Evidence::admission(&job).map_err(refuse)?);
                admission = Some(
                    state
                        .conduct_index
                        .prepare(ticket.account(), job)
                        .map_err(refuse)?,
                );
                let mut bytes = data.start.bytes().clone();
                (
                    Some(
                        PreparedBlob::prepare(key(Part::ConductStart), &mut bytes, vec![])
                            .map_err(refuse)?,
                    ),
                    None,
                )
            }
            (
                TicketKind::Settle {
                    binding,
                    artifact,
                    measurement,
                    ..
                },
                Transition::Settle,
            ) => {
                if binding.job != quota.job() || binding.run.0 != quota.admission().run() {
                    return Err(ParticipantError::Authority);
                }
                if quota.after().phase == ExecutionPhase::Complete {
                    let mut bytes = artifact.as_ref().ok_or(ParticipantError::Type)?.clone();
                    let measured = measurement.as_ref().ok_or(ParticipantError::Type)?;
                    if measured.policy() != quota.admission().policy()
                        || measured.subject() != quota.admission().subject()
                        || quota.after().result.as_ref().is_none_or(|r| {
                            r.sha256.as_bytes() != &bytes.descriptor().sha256
                                || r.bytes != bytes.descriptor().bytes
                        })
                    {
                        return Err(ParticipantError::Authority);
                    }
                    (
                        Some(
                            PreparedBlob::prepare(key(Part::ConductResult), &mut bytes, vec![])
                                .map_err(refuse)?,
                        ),
                        Some(measured.clone()),
                    )
                } else {
                    (None, None)
                }
            }
            (TicketKind::Grant { binding, .. }, Transition::Grant)
            | (TicketKind::Cancel(binding), Transition::Cancel)
            | (TicketKind::Recover(binding), Transition::Recover) => {
                if binding.job != quota.job() || binding.run.0 != quota.admission().run() {
                    return Err(ParticipantError::Authority);
                }
                (None, None)
            }
            _ => return Err(ParticipantError::Type),
        };
        if let Some(blob) = &blob {
            state.check_new(blob).map_err(refuse)?;
        }
        if let Some(measured) = &measurement {
            let entry = retained.as_ref().ok_or(ParticipantError::Authority)?;
            history_after = Some(
                history_before
                    .as_ref()
                    .ok_or(ParticipantError::Authority)?
                    .completed(entry.job(), measured, quota.after().charged_unit_ms)
                    .map_err(refuse)?,
            );
        }
        let output = ArtifactOutput::observed(&quota, measurement).map_err(refuse)?;
        let mut writes = blob
            .as_ref()
            .map(|blob| {
                blob.writes
                    .iter()
                    .map(super::conduct_index::replay)
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
            .map_err(refuse)?
            .unwrap_or_default();
        if let Some(admission) = &admission {
            writes.extend(
                admission
                    .writes()
                    .map_err(refuse)?
                    .iter()
                    .map(super::conduct_index::replay)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(refuse)?,
            );
        }
        let existing = if quota.transition() != Transition::Admit {
            let key = key(Part::ConductStart);
            let manifest = state.get(key).map_err(refuse)?;
            if manifest.artifact.sha256 != *quota.admission().start().sha256.as_bytes()
                || manifest.artifact.bytes != quota.admission().start().bytes
            {
                return Err(ParticipantError::Authority);
            }
            let value = serde_json::to_value(manifest).map_err(refuse)?;
            if blob.is_none() {
                writes.push(
                    super::conduct_index::replay(&mush_core::ModuleWrite::JsonSet {
                        key: key.document(),
                        value: value.clone(),
                    })
                    .map_err(refuse)?,
                );
            }
            Some((key, value))
        } else {
            None
        };
        let patch = ArtifactPatch {
            quota,
            ticket,
            blob,
            existing,
            admission,
            retained,
            history_before,
            history_after: history_after.ok_or(ParticipantError::Authority)?,
        };
        check_artifacts(state, &patch)?;
        Ok(OwnerPreparation::new(output, patch, writes))
    }
    fn recheck(
        &self,
        state: &Artifacts,
        context: &OwnerPrepareContext<'_>,
        patch: &ArtifactPatch,
    ) -> Result<(), ParticipantError> {
        self.policy
            .check(context, patch.quota.transition(), patch.ticket.character())?;
        if context.commit_token() != patch.quota.operation()
            || context.world().now() < patch.quota.at_utc()
        {
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
        super::artifacts::accepts_registry_write(write) || super::conduct_index::accepts(write)
    }
}
fn check_artifacts(state: &Artifacts, patch: &ArtifactPatch) -> Result<(), ParticipantError> {
    state
        .conduct_history
        .check(patch.quota.job(), patch.history_before.as_ref())
        .map_err(refuse)?;
    state
        .conduct
        .check(patch.quota.ticket(), &patch.ticket)
        .map_err(refuse)?;
    if let Some(admission) = &patch.admission {
        state.conduct_index.check(admission).map_err(refuse)?;
    }
    if let Some(retained) = &patch.retained {
        if state.conduct_index.get(patch.quota.job()).map_err(refuse)? != retained {
            return Err(ParticipantError::Authority);
        }
    }
    if let Some(blob) = &patch.blob {
        state.check_new(blob).map_err(refuse)?;
    }
    if let Some((key, value)) = &patch.existing {
        if serde_json::to_value(state.get(*key).map_err(refuse)?).map_err(refuse)? != *value {
            return Err(ParticipantError::Authority);
        }
    }
    Ok(())
}

/// Capacity-only image, never decoded as authority or installed. The result's
/// maximum UTF-8 bytes are pessimistically all six-byte JSON escapes. Include
/// the complete account ceiling and fixed typed/root allowance in this vector;
/// leave another 64KiB for the ordinary bundle envelope and dispatch metadata.
fn require_terminal_artifact_room(account: AccountId, run: RunId) -> Result<(), ParticipantError> {
    use super::artifacts::{Key, Part, PreparedBlob};
    use mush_roll::commerce::conduct::MAX_RESULT_ARTIFACT_BYTES;
    let mut bytes = super::model::ArtifactBytes::new(vec![1; MAX_RESULT_ARTIFACT_BYTES as usize])
        .map_err(refuse)?;
    let blob = PreparedBlob::prepare(
        Key {
            account,
            run,
            part: Part::ConductResult,
        },
        &mut bytes,
        vec![],
    )
    .map_err(refuse)?;
    let mut writes = blob
        .writes
        .iter()
        .map(super::conduct_index::replay)
        .collect::<Result<Vec<_>, _>>()
        .map_err(refuse)?;
    writes.push(
        super::conduct_index::replay(&mush_core::ModuleWrite::JsonSet {
            key: account_key(account),
            value: serde_json::Value::String("x".repeat(super::model::MAX_ACCOUNT_DOCUMENT)),
        })
        .map_err(refuse)?,
    );
    let material =
        serde_json::json!({"writes":writes,"typed_and_root_ceiling":"x".repeat(64*1024)});
    mush_core::canonical_json_bytes_with_limit(
        &material,
        mush_core::authoritative::MAX_AUTHORITATIVE_BUNDLE_BYTES - 64 * 1024,
    )
    .map(|_| ())
    .map_err(refuse)
}

impl mush_roll::commerce::conduct::protocol::ConductQuotaParticipant for QuotaParticipant {
    fn project_selector(
        engine: &mush_core::Engine,
        query: &mush_core::LegalActionQuery,
        transition: Transition,
    ) -> Result<
        Option<mush_roll::commerce::conduct::protocol::ConductSelector>,
        mush_roll::commerce::records::PublicationError,
    > {
        use mush_roll::commerce::records::PublicationError;
        if query.actor() != mush_core::GOD
            || query.principal() != &(PrincipalId::Object { id: mush_core::GOD })
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: mush_core::GOD })
            || query.projected_subject() != &PerceivedRef::SelfActor
            || engine.world.get(mush_core::GOD).is_none_or(|o| {
                o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
            })
        {
            return Ok(None);
        }
        let owner = OwnerId::new(crate::NS).map_err(|_| PublicationError::Invalid)?;
        engine
            .registry
            .require_mechanics_owner_ready(&owner)
            .map_err(|_| PublicationError::Unavailable)?;
        let result = engine
            .world
            .module_state::<Artifacts>()
            .ok_or(PublicationError::Unavailable)?
            .conduct
            .selector(transition)
            .map_err(|_| PublicationError::Unavailable)?;
        engine
            .registry
            .require_mechanics_owner_ready(&owner)
            .map_err(|_| PublicationError::Unavailable)?;
        Ok(result)
    }
    fn audit_history(
        ctx: &mush_core::ModuleCtx<'_>,
        rows: &[mush_roll::commerce::conduct::protocol::ConductHistoryRow<'_>],
    ) -> Result<(), mush_roll::commerce::records::PublicationError> {
        super::conduct_history::audit(ctx.w, rows)
            .map_err(|_| mush_roll::commerce::records::PublicationError::Mismatch)
    }
    fn check_history(
        world: &mush_core::World,
        row: mush_roll::commerce::conduct::protocol::ConductHistoryRow<'_>,
    ) -> Result<(), mush_roll::commerce::records::PublicationError> {
        super::conduct_history::check(world, row)
            .map_err(|_| mush_roll::commerce::records::PublicationError::Mismatch)
    }
}
