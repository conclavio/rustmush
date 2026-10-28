//! `registry_mind_inspection`'s provider (doc 17h cut M8a, A4; the IC sortie provider's
//! shape): read the hull in the prepare's transaction, land the Roll's record, answer.
use mush_core::action::{
    ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput, PrincipalId,
};
use mush_core::{
    ActionInvocation, ActionTxn, DefinitionRef, ModError, ObjId, PerceivedRef, PrepareCtx,
};
use serde::Deserialize;

use super::RegisteredInspectionWriter;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameters {
    hull: u32,
}

pub(super) struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    writer: RegisteredInspectionWriter,
    refusal: ActionError,
}

impl Provider {
    pub(super) fn new(
        action: DefinitionRef,
        outcome: DefinitionRef,
        writer: RegisteredInspectionWriter,
    ) -> Result<Self, ModError> {
        Ok(Self {
            action,
            outcome,
            writer,
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "The Registry inspection is unavailable or stale.",
            )
            .map_err(|e| ModError::msg(e.to_string()))?,
        })
    }

    /// The hull named, when the invocation is the actor's own with no targets.
    fn request(&self, inv: &ActionInvocation) -> Result<ObjId, ActionError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
        {
            return Err(self.refusal.clone());
        }
        let p: Parameters =
            serde_json::from_value(inv.parameters().clone()).map_err(|_| self.refusal.clone())?;
        if p.hull == 0 {
            return Err(self.refusal.clone());
        }
        Ok(ObjId(p.hull))
    }

    fn definitions(&self) -> Result<Vec<DefinitionRef>, ActionError> {
        let authority = self.writer.authority().map_err(|_| self.refusal.clone())?;
        Ok(vec![self.action.clone(), self.outcome.clone(), authority])
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
        let hull = self.request(inv)?;
        let found = {
            let world = ctx.world().map_err(|_| self.refusal.clone())?;
            super::facts::read(world, txn, hull).ok_or_else(|| self.refusal.clone())?
        };
        let record = self
            .writer
            .record(ctx, txn, inv.actor(), found.facts)
            .map_err(|_| self.refusal.clone())?;
        let result = mush_core::mechanics::RulesValue::new(
            self.outcome.clone(),
            serde_json::json!({"inspection": record.inspection, "hull": hull.0}),
        )
        .map_err(|_| self.refusal.clone())?;
        Ok(AuthoritativeActionOutput::new(result)
            .with_definitions(self.definitions()?)
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
            || !self.writer.offered(&engine.world, query.actor())
        {
            return Ok(None);
        }
        let fail = || self.refusal.clone();
        let definitions = engine.registry.definitions().ok_or_else(fail)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        for r in self.definitions()? {
            guards.add_definition(r, definitions).map_err(|_| fail())?;
        }
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: serde_json::json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(vec![
                mush_core::ProjectedParameter {
                    id: mush_core::mechanics::CanonicalId::new("hull").map_err(|_| fail())?,
                    required: true,
                    kind: mush_core::ProjectedParameterKind::Integer {
                        minimum: 1,
                        maximum: i64::from(u32::MAX),
                    },
                },
            ])
            .map_err(|_| fail())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "roll-registry-inspection",
                "Inspect a hull",
                "Read a hull's fitted core, its provenance and the seal on its open trip. One record; nothing changes.",
            )
            .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }
}
