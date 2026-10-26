use super::{
    host::{IcHost, Intent},
    participant::IcSortieParticipant,
};
use mush_core::action::{
    ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput, PrincipalId,
};
use mush_core::{
    ActionInvocation, ActionTxn, DefinitionDigest, DefinitionRef, ModError, PerceivedRef,
    PrepareCtx,
};
use mush_roll::commerce::ic_claims::{IcClaimConfig, RegisteredIcWriter};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameters {
    ticket: DefinitionDigest,
}
pub(super) struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    config: IcClaimConfig,
    closing: bool,
    host: IcHost,
    writer: RegisteredIcWriter<IcSortieParticipant>,
    refusal: ActionError,
}
impl Provider {
    pub(super) fn new(
        action: DefinitionRef,
        outcome: DefinitionRef,
        config: IcClaimConfig,
        closing: bool,
        host: IcHost,
        writer: RegisteredIcWriter<IcSortieParticipant>,
    ) -> Result<Self, ModError> {
        Ok(Self {
            action,
            outcome,
            config,
            closing,
            host,
            writer,
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "Actual IC host transition unavailable or evidence changed",
            )
            .map_err(|e| ModError::msg(e.to_string()))?,
        })
    }
    fn request(&self, inv: &ActionInvocation) -> Result<Intent, ActionError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
        {
            return Err(self.refusal.clone());
        }
        let p: Parameters =
            serde_json::from_value(inv.parameters().clone()).map_err(|_| self.refusal.clone())?;
        self.host
            .lookup(p.ticket, inv.actor(), self.closing)
            .map_err(|_| self.refusal.clone())
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.request(inv).map(|_| ())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let intent = self.request(inv)?;
        let receipt = match intent {
            Intent::Open {
                start,
                licence,
                profiles,
            } => self.writer.open(ctx, txn, start, licence, profiles),
            Intent::Close(event) => self.writer.close(ctx, txn, event),
        }
        .map_err(|_| self.refusal.clone())?;
        let result = mush_core::mechanics::RulesValue::new(
            self.outcome.clone(),
            serde_json::json!({"open":receipt.open(),"closed":receipt.closed()}),
        )
        .map_err(|_| self.refusal.clone())?;
        Ok(AuthoritativeActionOutput::new(result)
            .with_definitions(
                std::iter::once(self.action.clone())
                    .chain([self.outcome.clone(), self.config.manifest().clone()])
                    .chain(self.config.participants())
                    .collect(),
            )
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
            || !self.host.ready(&engine.world)
        {
            return Ok(None);
        }
        let Some(intent) = self
            .host
            .selected(query.actor(), self.closing)
            .map_err(|_| self.refusal.clone())?
        else {
            return Ok(None);
        };
        if engine
            .world
            .module_state::<super::IcSorties>()
            .is_none_or(|s| s.request_used(intent.request()))
        {
            return Ok(None);
        }
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| self.refusal.clone())?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| self.refusal.clone())?;
        for r in std::iter::once(&self.action)
            .chain([&self.outcome, self.config.manifest()])
            .chain(self.config.participants().iter())
        {
            guards
                .add_definition(r.clone(), definitions)
                .map_err(|_| self.refusal.clone())?;
        }
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: serde_json::json!({"ticket":intent.request()}),
            parameters: mush_core::ProjectedParameterSchema::new(vec![])
                .map_err(|_| self.refusal.clone())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: vec![],
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "ic-mind-transition",
                "Apply owned IC transition",
                "Apply the genuine transition already retained by the IC host.",
            )
            .map_err(|_| self.refusal.clone())?,
            guards: guards.finish().map_err(|_| self.refusal.clone())?,
        }))
    }
}
