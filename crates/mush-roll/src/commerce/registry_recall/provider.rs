use super::super::{
    families::CommerceFamilies,
    participant::{RootParticipant, RootRequest},
};
use super::*;
use mush_core::action::{ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput};
use mush_core::mechanics::RulesValue;
use mush_core::participant::ParticipantHandle;
use mush_core::{ActionTxn, PrepareCtx, Registrar};

struct Provider {
    policy: RegistryRecallPolicy,
    action: DefinitionRef,
    outcome: DefinitionRef,
    authority: DefinitionRef,
    retention: DefinitionRef,
    families: CommerceFamilies,
    roots: ParticipantHandle<RootParticipant>,
    readiness: Readiness,
    refusal: ActionError,
}
impl Provider {
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, PublicationError> {
        let filed = Request::parse(inv, &self.action, &self.policy)?;
        let world = ctx.world().map_err(bad)?;
        // Authorization precedes any private commercial lookup.
        let (authorization, at_ic) =
            authority::observe(world, &self.readiness, &self.policy, inv.actor())?;
        let roll = world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        roll.commerce_ready().map_err(bad)?;
        let file = txn
            .get(&self.families.filed, &filed.key())
            .map_err(bad)?
            .ok_or(PublicationError::Missing)?
            .clone();
        let initial =
            super::super::line_update::audit::birth_in_txn(txn, world, &self.families, &file)?;
        let before = txn
            .get(&self.families.status, &filed.key())
            .map_err(bad)?
            .ok_or(PublicationError::Missing)?
            .clone();
        file.validate()?;
        initial.validate()?;
        before.validate()?;
        if before.filed != filed
            || before.recalled_by.is_some()
            || initial.economic.issuer != self.policy.issuer
        {
            return Err(PublicationError::Mismatch);
        }
        roll.check_required_root(&RequiredRoot {
            anchor: RootAnchor::Filed(filed),
            identity: file.content.revision(),
        })
        .map_err(bad)?;
        let operation = records::RecallOperation {
            format: 1,
            operation: ctx.commit_token(),
            action: self.action.clone(),
            authority: self.policy.clone(),
            actor: inv.actor(),
            authorization,
            filed,
            revision: file.content.revision(),
            rating: file.rating.clone(),
            initial_operation: initial.commit,
            initial_issuer: initial.economic.issuer.clone(),
            withdrawn_by: before.withdrawn_by,
            reason: records::Reason::RegistryReview,
            at_ic,
        };
        operation.matches(&initial, &file)?;
        let fence = RecallFence {
            operation: operation.clone(),
            readiness: self.readiness.clone(),
        };
        ctx.prepare_owner(
            &self.roots,
            RootRequest::RegistryRecall(Box::new(Recall {
                operation: operation.clone(),
                fence,
            })),
        )
        .map_err(bad)?;
        txn.insert(
            &self.families.registry_recalls,
            &operation.key(),
            operation.clone(),
        )
        .map_err(bad)?;
        let status = operation.status();
        txn.replace(&self.families.status, &filed.key(), status.clone())
            .map_err(bad)?;
        if txn
            .get(&self.families.registry_recalls, &operation.key())
            .map_err(bad)?
            != Some(&operation)
            || txn.get(&self.families.status, &filed.key()).map_err(bad)? != Some(&status)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(
                self.outcome.clone(),
                json!({"filed":filed,"operation":operation.operation,"reason":"registry_review"}),
            )
            .map_err(bad)?,
        )
        .with_definitions(vec![
            self.action.clone(),
            self.outcome.clone(),
            self.authority.clone(),
            self.retention.clone(),
        ])
        .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        Request::parse(inv, &self.action, &self.policy)
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
            || authority::observe(&engine.world, &self.readiness, &self.policy, query.actor())
                .is_err()
            || engine
                .world
                .module_state::<crate::RollStore>()
                .is_none_or(|s| s.commerce_ready().is_err())
        {
            return Ok(None);
        }
        let fail = || self.refusal.clone();
        let defs = engine.registry.definitions().ok_or_else(fail)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        for definition in [&self.action, &self.authority, &self.retention] {
            guards
                .add_definition(definition.clone(), defs)
                .map_err(|_| fail())?;
        }
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters:json!({"reason":"registry_review"}),
            parameters:mush_core::ProjectedParameterSchema::new(vec![mush_core::ProjectedParameter {
                id:mush_core::mechanics::CanonicalId::new("filed").map_err(|_|fail())?,required:true,
                kind:mush_core::ProjectedParameterKind::Text {maximum_bytes:20},
            }]).map_err(|_|fail())?,targets:mush_core::ProjectedTargetSchema::None,
            required_channels:Vec::new(),expires:None,
            presentation:mush_core::ActionPresentation::new("roll-registry-recall","Recall a filed revision",
                "Recall this exact filed revision after Registry review. Running sorties retain their captured mind.").map_err(|_|fail())?,
            guards:guards.finish().map_err(|_|fail())?,
        }))
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    policy: RegistryRecallPolicy,
    definitions: [DefinitionRef; 3],
) -> Result<Readiness, ModError> {
    let [authority, action, outcome] = definitions;
    if reg.module() != crate::NS || action != policy.action()? {
        return Err(error("Registry recall registration differs"));
    }
    let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
    let readiness = Readiness::default();
    reg.action_provider(
        action.clone(),
        Provider {
            policy,
            action,
            outcome,
            authority,
            retention: super::super::authored_retirement::retention_reference()?,
            families,
            roots,
            readiness: readiness.clone(),
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "The Registry recall is unavailable or stale.",
            )
            .map_err(error)?,
        },
    )?;
    Ok(readiness)
}
