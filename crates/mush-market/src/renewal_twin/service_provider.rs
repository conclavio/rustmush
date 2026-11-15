//! The sixth provider is available only for one private retained service intent.
//! A public GOD offer or a decoded selector cannot create that intent.
use super::Issuer;
use super::config::RenewalConfig;
use super::families::Families;
use super::records::{OperationRow, ReceiptRow};
use mush_core::action::{
    ActionError, ActionInvocation, ActionProvider, AuthoritativeActionOutput, PerceivedRef,
    PrincipalId,
};
use mush_core::participant::ParticipantHandle;
use mush_core::{ActionTxn, DefinitionRef, GOD, PrepareCtx, RulesValue};
use mush_economy::registry_filing::InitialFilingParticipant;
use mush_roll::commerce::renewal_twin::{Binding as _, writer::RegisteredRenewalWriter};
pub(crate) struct Provider<I: Issuer> {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub config: RenewalConfig<I>,
    pub families: Families<I>,
    pub economy: ParticipantHandle<InitialFilingParticipant>,
    pub writer: RegisteredRenewalWriter<I>,
    pub refusal: ActionError,
}
fn invalid(_: impl std::fmt::Display) -> String {
    super::records::invalid()
}
impl<I: Issuer> Provider<I> {
    fn intent(&self, inv: &ActionInvocation) -> Result<super::service::CloseIntent<I>, String> {
        let service = self
            .config
            .service
            .as_ref()
            .ok_or_else(super::records::invalid)?;
        let runtime = service.runtime.lock().map_err(invalid)?;
        let intent = runtime.close.as_ref().ok_or_else(super::records::invalid)?;
        if !runtime.enabled
            || inv.actor() != GOD
            || inv.principal() != &(PrincipalId::Object { id: GOD })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner.as_str() != crate::NS
            || !inv.keyed_by(&intent.key)
            || intent.action != self.action
            || inv.parameters() != &intent.parameters()
        {
            return Err(super::records::invalid());
        }
        Ok(intent.clone())
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, String> {
        let intent = self.intent(inv)?;
        let operation = intent.before.operation();
        let before = txn
            .get(&self.families.operations, &operation)
            .map_err(invalid)?
            .cloned()
            .ok_or_else(super::records::invalid)?;
        if before != intent.before
            || txn
                .get(&self.families.receipts, &operation)
                .map_err(invalid)?
                .is_some()
            || super::service::close_reason::<I>(ctx.world().map_err(invalid)?, &before)
                != Some(intent.reason)
        {
            return Err(super::records::invalid());
        }
        let roll = I::operation(&self.writer, ctx, txn, operation).map_err(invalid)?;
        before.matches_roll(&roll)?;
        let output = ctx
            .prepare_owner(
                &self.economy,
                I::service_close_request(operation, intent.reason),
            )
            .and_then(I::offer_output)
            .map_err(invalid)?;
        let roll = I::service_close(&self.writer, ctx, txn, &roll, &output).map_err(invalid)?;
        let after = before.progressed(&roll)?;
        txn.replace(&self.families.operations, &operation, after.clone())
            .map_err(invalid)?;
        txn.insert(&self.families.receipts, &operation, after.receipt()?)
            .map_err(invalid)?;
        if txn
            .get(&self.families.operations, &operation)
            .map_err(invalid)?
            != Some(&after)
        {
            return Err(super::records::invalid());
        }
        txn.get(&self.families.receipts, &operation)
            .map_err(invalid)?
            .ok_or_else(super::records::invalid)?
            .matches_operation(&after)?;
        I::advance_completion(ctx, txn, operation).map_err(invalid)?;
        let outcome=RulesValue::new(self.outcome.clone(),serde_json::json!({"kind":"service_closed","operation":operation,"commit":ctx.commit_token()})).map_err(invalid)?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![self.outcome.clone(), self.config.manifest.clone()])
            .with_eligible_witnesses(vec![
                inv.principal().clone(),
                PrincipalId::Object {
                    id: before.binding().holder().acting_character(),
                },
            ]))
    }
}
impl<I: Issuer> ActionProvider for Provider<I> {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.intent(inv)
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
            || q.actor() != GOD
            || q.principal() != &(PrincipalId::Object { id: GOD })
            || q.projected_subject() != &PerceivedRef::SelfActor
            || q.truth_subject() != &(mush_core::SubjectKey::Object { id: GOD })
        {
            return Ok(None);
        }
        let Some(service) = &self.config.service else {
            return Ok(None);
        };
        let Ok(runtime) = service.runtime.lock() else {
            return Err(self.refusal.clone());
        };
        let Some(intent) = runtime.close.as_ref().filter(|_| runtime.enabled) else {
            return Ok(None);
        };
        if intent.action != self.action
            || super::service::close_reason::<I>(&engine.world, &intent.before)
                != Some(intent.reason)
        {
            return Ok(None);
        }
        let bad = || self.refusal.clone();
        let defs = engine.registry.definitions().ok_or_else(bad)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| bad())?;
        for r in [
            &self.action,
            &self.config.manifest,
            self.config.participants.economy(),
            self.config.participants.retention(),
        ] {
            guards.add_definition(r.clone(), defs).map_err(|_| bad())?;
        }
        guards
            .add(intent.guard().map_err(|_| bad())?)
            .map_err(|_| bad())?;
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: intent.parameters(),
            parameters: mush_core::ProjectedParameterSchema::new(vec![]).map_err(|_| bad())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: vec![],
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                I::SERVICE_VERB,
                "Close an unavailable renewal offer",
                "Process the exact retained unfunded offer.",
            )
            .map_err(|_| bad())?,
            guards: guards.finish().map_err(|_| bad())?,
        }))
    }
}
