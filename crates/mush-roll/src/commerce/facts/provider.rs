//! Only the actual automatic service intent admits an occurrence publication.
use super::super::{
    families::CommerceFamilies,
    participant::{RootParticipant, RootRequest},
};
use super::*;
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    PerceivedRef, PrincipalId,
};
use mush_core::participant::ParticipantHandle;
use mush_core::{ActionTxn, DefinitionRef, PrepareCtx, RulesValue};
struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    retention: DefinitionRef,
    f: CommerceFamilies,
    roots: ParticipantHandle<RootParticipant>,
    reader: OwnerReadHandle<reader::Reader>,
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
        self.ready.check(ctx.world().map_err(invalid)?)?;
        intent.fresh(ctx.world().map_err(invalid)?)?;
        let reader::Reply::Term(proof) = ctx
            .read_owner_preimage(
                txn,
                &self.reader,
                reader::Query::Term(intent.request.clone()),
            )
            .map_err(invalid)?
        else {
            return Err(PublicationError::Mismatch);
        };
        if proof.accepted != intent.accepted {
            return Err(PublicationError::Mismatch);
        }
        proof.require_lapsed(&ctx.world().map_err(invalid)?.world_ic_now())?;
        proof.require_before(txn, &self.f)?;
        let occurrence = records::LapseOccurrence {
            format: 1,
            operation: ctx.commit_token(),
            action: self.action.clone(),
            term: proof.accepted.clone(),
            observed_head: proof.observed_head(),
            actor: inv.actor(),
            at_ic: ctx.world().map_err(invalid)?.world_ic_now(),
            at_utc: ctx.world().map_err(invalid)?.now(),
        };
        occurrence.validate()?;
        if txn
            .get(&self.f.lapse_occurrences, &occurrence.key())
            .map_err(invalid)?
            .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let roots = publication::Lapse::new(
            occurrence.clone(),
            intent,
            self.runtime.clone(),
            self.ready.clone(),
        )?;
        ctx.prepare_owner(&self.roots, RootRequest::TermLapse(roots))
            .map_err(invalid)?;
        txn.insert(
            &self.f.lapse_occurrences,
            &occurrence.key(),
            occurrence.clone(),
        )
        .map_err(invalid)?;
        if txn
            .get(&self.f.lapse_occurrences, &occurrence.key())
            .map_err(invalid)?
            != Some(&occurrence)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(AuthoritativeActionOutput::new(RulesValue::new(self.outcome.clone(),serde_json::json!({"licence":occurrence.term.licence,"generation":occurrence.term.generation,"operation":occurrence.operation})).map_err(invalid)?)
            .with_definitions(vec![self.action.clone(),self.outcome.clone(),self.retention.clone()]).with_eligible_witnesses(vec![inv.principal().clone()]))
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
                "roll-minds-lapse-service",
                "Record an expired mind term",
                "Process one exact retained term observation.",
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
    reader: OwnerReadHandle<reader::Reader>,
    ready: Readiness,
    runtime: service::Runtime,
) -> Result<(), ModError> {
    let [action, outcome] = definitions;
    if reg.module() != crate::NS || action != protocol::action()? {
        return Err(error("minds lapse registration differs"));
    }
    let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
    reg.action_provider(
        action.clone(),
        Provider {
            action,
            outcome,
            retention: super::super::authored_retirement::retention_reference()?,
            f,
            roots,
            reader,
            ready,
            runtime,
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "Mind term occurrence is unavailable or stale.",
            )
            .map_err(error)?,
        },
    )
}
