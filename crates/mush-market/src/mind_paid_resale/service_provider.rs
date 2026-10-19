//! Only an actual Roll-owned private continuation intent reaches this provider.
//! Service actor identity and fixed public parameters alone are insufficient.
use super::{
    PaidResaleConfig,
    families::{Families, envelope},
    records::{MindPaidResaleOperation, MindPaidResaleReceipt},
};
use mush_core::action::{
    ActionError, ActionInvocation, ActionProvider, AuthoritativeActionOutput, PerceivedRef,
    PrincipalId,
};
use mush_core::participant::ParticipantHandle;
use mush_core::{ActionTxn, DefinitionRef, PrepareCtx, RulesValue};
use mush_economy::registry_filing::{EconomyRequest, InitialFilingParticipant};
use mush_roll::commerce::paid_resale::writer::RegisteredPaidResaleWriter;
fn bad(_: impl std::fmt::Display) -> String {
    "paid resale terminal unavailable".into()
}
pub(super) struct Provider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub config: PaidResaleConfig,
    pub families: Families,
    pub economy: ParticipantHandle<InitialFilingParticipant>,
    pub writer: RegisteredPaidResaleWriter,
    pub refusal: ActionError,
}
impl Provider {
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, String> {
        self.writer.check_service_invocation(inv).map_err(bad)?;
        let observed = self.writer.observe_terminal(ctx, txn, inv).map_err(bad)?;
        let before = observed.before();
        let acceptance = before.operation();
        let market = txn
            .get(&self.families.operations, &acceptance)
            .map_err(bad)?
            .cloned()
            .ok_or_else(|| bad("missing pair"))?;
        market.matches_roll(before)?;
        let offer = txn
            .get(&self.families.offers, &before.offer())
            .map_err(bad)?
            .cloned()
            .ok_or_else(|| bad("missing offer"))?;
        offer.matches_roll(before)?;
        let output = ctx
            .prepare_owner(
                &self.economy,
                EconomyRequest::PaidResaleTerminal(observed.request()),
            )
            .and_then(|o| o.paid_resale())
            .map_err(bad)?;
        let row = self
            .writer
            .terminal(ctx, txn, &observed, &output)
            .map_err(bad)?;
        offer.matches_roll(&row)?;
        let next = MindPaidResaleOperation::from_roll(&row)?;
        let receipt = MindPaidResaleReceipt::from_roll(&row)?;
        envelope(
            &self.families.operations,
            acceptance,
            serde_json::to_value(&next).map_err(bad)?,
        )?;
        envelope(
            &self.families.receipts,
            receipt.acceptance(),
            serde_json::to_value(&receipt).map_err(bad)?,
        )?;
        txn.replace(&self.families.operations, &acceptance, next.clone())
            .map_err(bad)?;
        txn.insert(
            &self.families.receipts,
            &receipt.acceptance(),
            receipt.clone(),
        )
        .map_err(bad)?;
        if txn
            .get(&self.families.operations, &acceptance)
            .map_err(bad)?
            != Some(&next)
            || txn
                .get(&self.families.receipts, &receipt.acceptance())
                .map_err(bad)?
                != Some(&receipt)
        {
            return Err(bad("readback"));
        }
        ctx.close_paid_resale_completion(txn, acceptance)
            .map_err(bad)?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            serde_json::json!({"acceptance":acceptance,"operation":ctx.commit_token()}),
        )
        .map_err(bad)?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![self.outcome.clone()])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.writer
            .check_service_invocation(inv)
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
            || q.actor() != self.config.service
            || q.principal() != &(PrincipalId::Object { id: q.actor() })
            || q.projected_subject() != &PerceivedRef::SelfActor
            || q.truth_subject() != &(mush_core::SubjectKey::Object { id: q.actor() })
        {
            return Ok(None);
        }
        let Some(intent) = self
            .writer
            .service_offer(&engine.world, q.actor())
            .map_err(|_| self.refusal.clone())?
        else {
            return Ok(None);
        };
        let fail = || self.refusal.clone();
        let defs = engine
            .registry
            .definitions()
            .ok_or_else(|| self.refusal.clone())?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        for r in [
            &self.action,
            self.config.participants.economy(),
            self.config.participants.retention(),
        ] {
            guards.add_definition(r.clone(), defs).map_err(|_| fail())?;
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
                "paid-resale-service",
                "Complete retained paid licence custody",
                "Process the exact retained transfer intent.",
            )
            .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }
}
