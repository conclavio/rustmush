//! Only the actual automatic service intent admits an occurrence publication.
use super::super::reader::{
    ReconformanceReadRequest, ReconformanceReadResult, ReconformanceReader,
};
use super::*;
use crate::commerce::{
    families::CommerceFamilies,
    participant::{RootParticipant, RootRequest},
};
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    PerceivedRef, PrincipalId,
};
use mush_core::participant::ParticipantHandle;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{ActionTxn, DefinitionRef, PrepareCtx, RulesValue};
struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    manifest: DefinitionRef,
    retention: DefinitionRef,
    f: CommerceFamilies,
    roots: ParticipantHandle<RootParticipant>,
    reader: OwnerReadHandle<ReconformanceReader>,
    ready: Readiness,
    runtime: service::Runtime,
    refusal: ActionError,
}
impl Provider {
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, PublicationError> {
        let intent = self.runtime.intent(inv, &self.action)?;
        self.ready.check(ctx.world().map_err(bad)?)?;
        intent.fresh(ctx.world().map_err(bad)?)?;
        let ReconformanceReadResult::Rejection { proof } = ctx
            .read_owner_preimage(
                txn,
                &self.reader,
                ReconformanceReadRequest::Rejection {
                    filed: intent.request.filed,
                },
            )
            .map_err(bad)?
        else {
            return Err(PublicationError::Mismatch);
        };
        if proof != intent.proof || proof.compatibility.digest() != self.manifest.digest() {
            return Err(PublicationError::Mismatch);
        }
        let filed = txn
            .get(&self.f.filed, &proof.filed.key())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Mismatch)?;
        let status = txn
            .get(&self.f.status, &proof.filed.key())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Mismatch)?;
        if status != proof.status || status.recalled_by.is_some() {
            return Err(PublicationError::Mismatch);
        }
        let operation = records::MeasuredRecall {
            format: 1,
            operation: ctx.commit_token(),
            action: self.action.clone(),
            manifest: self.manifest.clone(),
            actor: inv.actor(),
            filed: proof.filed,
            revision: proof.revision,
            birth_rating: proof.birth_rating,
            rejection: proof.rejection,
            plan: proof.plan,
            withdrawn_by: status.withdrawn_by,
            at_ic: ctx.world().map_err(bad)?.world_ic_now(),
            at_utc: ctx.world().map_err(bad)?.now(),
        };
        operation.matches(&filed)?;
        if txn
            .get(&self.f.measured_recalls, &operation.key())
            .map_err(bad)?
            .is_some()
            || txn
                .get(&self.f.registry_recalls, &operation.key())
                .map_err(bad)?
                .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let roots = publication::Recall::new(
            operation.clone(),
            intent,
            self.runtime.clone(),
            self.ready.clone(),
        )?;
        ctx.prepare_owner(&self.roots, RootRequest::MeasuredRecall(roots))
            .map_err(bad)?;
        txn.insert(
            &self.f.measured_recalls,
            &operation.key(),
            operation.clone(),
        )
        .map_err(bad)?;
        txn.replace(&self.f.status, &operation.filed.key(), operation.status())
            .map_err(bad)?;
        if txn
            .get(&self.f.measured_recalls, &operation.key())
            .map_err(bad)?
            != Some(&operation)
            || txn
                .get(&self.f.status, &operation.filed.key())
                .map_err(bad)?
                != Some(&operation.status())
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(
                self.outcome.clone(),
                serde_json::json!({"filed":operation.filed,"operation":operation.operation}),
            )
            .map_err(bad)?,
        )
        .with_definitions(vec![
            self.action.clone(),
            self.outcome.clone(),
            self.retention.clone(),
            self.manifest.clone(),
        ])
        .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.runtime
            .intent(inv, &self.action)
            .map(|_| ())
            .map_err(|_| self.refusal.clone())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare(inv, txn, ctx)
            .map_err(|_| self.refusal.clone())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        q: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || q.actor() != mush_core::GOD
            || q.principal() != &(PrincipalId::Object { id: mush_core::GOD })
            || q.projected_subject() != &PerceivedRef::SelfActor
            || q.truth_subject() != &(mush_core::SubjectKey::Object { id: mush_core::GOD })
            || self.ready.check(&engine.world).is_err()
        {
            return Ok(None);
        }
        let Some(intent) = self.runtime.projected() else {
            return Ok(None);
        };
        if intent.action != self.action || intent.fresh(&engine.world).is_err() {
            return Ok(None);
        }
        let fail = || self.refusal.clone();
        let defs = engine.registry.definitions().ok_or_else(fail)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        for definition in [&self.action, &self.retention] {
            guards
                .add_definition(definition.clone(), defs)
                .map_err(|_| fail())?;
        }
        guards
            .add(intent.guard().map_err(|_| fail())?)
            .map_err(|_| fail())?;
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: intent.parameters(),
            parameters: mush_core::ProjectedParameterSchema::new(vec![]).map_err(|_| fail())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: vec![],
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "roll-measured-recall-service",
                "Record measured Registry nonconformance",
                "Process one exact measured rejection.",
            )
            .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }
}
pub(super) fn register(
    reg: &mut Registrar<'_>,
    f: CommerceFamilies,
    definitions: [DefinitionRef; 2],
    manifest: DefinitionRef,
    reader: OwnerReadHandle<ReconformanceReader>,
    ready: Readiness,
    runtime: service::Runtime,
) -> Result<(), ModError> {
    let [action, outcome] = definitions;
    if reg.module() != crate::NS || action != protocol::action(&manifest)? {
        return Err(error("measured recall registration differs"));
    }
    let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
    reg.action_provider(
        action.clone(),
        Provider {
            action,
            outcome,
            manifest,
            retention: crate::commerce::authored_retirement::retention_reference()?,
            f,
            roots,
            reader,
            ready,
            runtime,
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "Measured Registry recall is unavailable or stale.",
            )
            .map_err(error)?,
        },
    )
}
