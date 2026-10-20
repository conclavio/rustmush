//! Signed scoped publication of one exact configured product. Product data
//! alone never supplies a scoped grant; the actual metadata owner repeats it.
use super::{
    CoverageError, CoverageProductV1, authority,
    families::{self, Families},
    metadata,
    protocol::Protocol,
};
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    PerceivedRef, PrincipalId,
};
use mush_core::participant::ParticipantHandle;
use mush_core::{ActionTxn, DefinitionRef, PrepareCtx, RulesValue};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    product: String,
}
pub(super) struct Provider {
    pub(super) protocol: Protocol,
    pub(super) action: DefinitionRef,
    pub(super) outcome: DefinitionRef,
    pub(super) families: Families,
    pub(super) products: Vec<CoverageProductV1>,
    pub(super) owner: ParticipantHandle<metadata::Participant>,
    pub(super) readiness: authority::Readiness,
    pub(super) refusal: ActionError,
    pub(super) conduct: Option<super::qualification::Configuration>,
    pub(super) conduct_reader: Option<
        mush_core::read_service::OwnerReadHandle<mush_roll::commerce::conduct::ConductReader>,
    >,
    pub(super) writer: mush_roll::commerce::coverage::RegisteredCoveragePublicationWriter,
}
impl Provider {
    fn product(&self, inv: &ActionInvocation) -> Result<&CoverageProductV1, CoverageError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner.as_str() != crate::NS
        {
            return Err(CoverageError::Authority);
        }
        let request: Request =
            serde_json::from_value(inv.parameters().clone()).map_err(|_| CoverageError::Product)?;
        let id = mush_core::DefinitionDigest::from_hex(&request.product)
            .map_err(|_| CoverageError::Product)?;
        if id.to_hex() != request.product {
            return Err(CoverageError::Product);
        }
        self.products
            .iter()
            .find(|p| p.digest().is_ok_and(|actual| actual == id))
            .ok_or(CoverageError::Product)
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, CoverageError> {
        let product = self.product(inv)?.clone();
        self.readiness.check(&self.action)?;
        let proofs = super::qualification::publication_proofs(
            ctx,
            txn,
            &product,
            self.conduct.as_ref(),
            self.conduct_reader.as_ref(),
        )?;
        let output = ctx
            .prepare_owner(
                &self.owner,
                metadata::Request {
                    product,
                    proofs: proofs.clone(),
                },
            )
            .map_err(|_| CoverageError::Authority)?;
        let row = output.into_row();
        if row.operation != ctx.commit_token()
            || row.action != self.action
            || row.publisher != inv.actor()
            || row.protocol != self.protocol
        {
            return Err(CoverageError::Authority);
        }
        self.families.admit(txn, 0)?;
        families::insert(txn, &self.families.products, &row)?;
        if !proofs.is_empty() {
            self.writer
                .publish(
                    ctx,
                    txn,
                    inv.actor(),
                    row.digest()?,
                    row.product.digest()?,
                    proofs,
                )
                .map_err(|_| CoverageError::Authority)?;
        }
        let outcome=RulesValue::new(self.outcome.clone(),serde_json::json!({"operation":row.operation,"kind":"published","digest":row.digest()?})).map_err(|_|CoverageError::Product)?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![
                self.action.clone(),
                self.outcome.clone(),
                self.protocol.manifest.clone(),
            ])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.product(inv)
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
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
            || self.readiness.check(action).is_err()
        {
            return Ok(None);
        }
        let mut choices = Vec::new();
        for product in &self.products {
            if authority::capture(
                &engine.world,
                query.actor(),
                product.insurer(),
                product.scope(),
            )
            .is_ok()
            {
                choices.push(product.digest().map_err(|_| self.refusal.clone())?.to_hex());
            }
        }
        if choices.is_empty() {
            return Ok(None);
        }
        choices.sort();
        choices.dedup();
        let fail = || self.refusal.clone();
        let defs = engine.registry.definitions().ok_or_else(fail)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        guards
            .add_definition(action.clone(), defs)
            .map_err(|_| fail())?;
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: serde_json::json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(vec![
                mush_core::ProjectedParameter {
                    id: mush_core::mechanics::CanonicalId::new("product").map_err(|_| fail())?,
                    required: true,
                    kind: mush_core::ProjectedParameterKind::Choice { choices },
                },
            ])
            .map_err(|_| fail())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "coverage-publish",
                "Publish hull coverage",
                "Publish these exact insurance terms for your organization.",
            )
            .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }
}
pub(super) fn refusal() -> Result<ActionError, mush_core::ModError> {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "Coverage is unavailable or its authority has changed.",
    )
    .map_err(|e| mush_core::ModError::msg(e.to_string()))
}
