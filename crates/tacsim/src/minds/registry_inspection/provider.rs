//! `registry_mind_inspection`'s provider (doc 17h cut M8a, A4; the IC sortie provider's
//! shape): read the hull in the prepare's transaction (the known-hull gate included, the fix
//! round's F2), land the Roll's record, answer — and, the fix round's F3 under the lead's
//! ruling, render the one receipt line from the exact facts recorded, keep it in the outcome
//! and project it for the operator, so what the operator reads is what was recorded. A
//! refused prepare records nothing and the driver prints only its refusal.
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
        let trip_open = found.opened_ic.is_some();
        let record = self
            .writer
            .record(ctx, txn, inv.actor(), found.facts)
            .map_err(|_| self.refusal.clone())?;
        let summary = {
            let world = ctx.world().map_err(|_| self.refusal.clone())?;
            super::facts::receipt(world, &record.facts(), trip_open)
        };
        let result = mush_core::mechanics::RulesValue::new(
            self.outcome.clone(),
            serde_json::json!({
                "inspection": record.inspection,
                "hull": hull.0,
                "operator": inv.actor().0,
                "summary": summary,
            }),
        )
        .map_err(|_| self.refusal.clone())?;
        Ok(AuthoritativeActionOutput::new(result)
            .with_definitions(self.definitions()?)
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }

    /// **The receipt** (the fix round's F3): the line the prepare rendered from the recorded
    /// facts, for the operator who asked; pure over the durable outcome.
    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &mush_core::mechanics::RulesValue,
        viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        if action != &self.action || outcome.schema() != &self.outcome {
            return Err(self.refusal.clone());
        }
        let data = outcome.data();
        let operator = data
            .get("operator")
            .and_then(serde_json::Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .map(ObjId)
            .ok_or_else(|| self.refusal.clone())?;
        if viewer != &(PrincipalId::Object { id: operator }) {
            return Err(self.refusal.clone());
        }
        let summary = data
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| self.refusal.clone())?;
        mush_proto::ProjectedActionResult::new(super::INSPECTION_ACTION, summary, data.clone())
            .map(Some)
            .map_err(|_| self.refusal.clone())
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
