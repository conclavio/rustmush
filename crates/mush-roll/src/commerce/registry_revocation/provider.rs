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
    policy: RegistryRevocationPolicy,
    action: DefinitionRef,
    outcome: DefinitionRef,
    authority: DefinitionRef,
    retention: DefinitionRef,
    families: CommerceFamilies,
    ratings: super::super::rating::RatingFamilies,
    roots: ParticipantHandle<RootParticipant>,
    readiness: Readiness,
    pending: Option<(
        super::super::charter_pending::Readiness,
        ParticipantHandle<mush_economy::registry_filing::InitialFilingParticipant>,
    )>,
    resale: Option<(
        super::super::paid_resale::writer::PaidResaleWriter,
        ParticipantHandle<mush_economy::registry_filing::InitialFilingParticipant>,
    )>,
    refusal: ActionError,
}
impl Provider {
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, PublicationError> {
        let id = Request::parse(inv, &self.action, &self.policy)?;
        let world = ctx.world().map_err(bad)?;
        // Actual scoped Registry authority precedes any private holding lookup.
        let (authorization, at_ic) =
            authority::observe(world, &self.readiness, &self.policy, inv.actor())?;
        let roll = world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        roll.commerce_ready().map_err(bad)?;
        let (term, charter, line, birth_operation) =
            history::capture(txn, world, &self.families, &self.ratings, id)?;
        if !matches!(
            term.window()
                .phase_at(super::super::WorldIcSeconds::from_stamp(&at_ic).map_err(bad)?),
            super::super::TermPhase::Active | super::super::TermPhase::Grace
        ) || txn
            .get(&self.families.revoked_terms, &term.status_key())
            .map_err(bad)?
            .is_some()
        {
            return Err(PublicationError::Unavailable);
        }
        let line_row = txn
            .get(&self.families.lines, &line.key())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        line_row.validate()?;
        let filed = match term.policy() {
            super::super::records::RevisionPolicy::Pin { filed, .. } => filed,
            super::super::records::RevisionPolicy::Follow { line: follow } if follow == line => {
                line_row.head
            }
            _ => return Err(PublicationError::Mismatch),
        };
        let file = txn
            .get(&self.families.filed, &filed.key())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let initial =
            super::super::line_update::audit::birth_in_txn(txn, world, &self.families, &file)?;
        file.validate()?;
        initial.validate()?;
        if line_row.id != line
            || file.content.id() != filed
            || line_row.charter != charter
            || file.content.position().charter != charter
            || file.content.position().line != line
            || initial.economic.issuer != self.policy.issuer
        {
            return Err(PublicationError::Mismatch);
        }
        roll.check_required_root(&RequiredRoot {
            anchor: RootAnchor::Filed(filed),
            identity: file.content.revision(),
        })
        .map_err(bad)?;
        let operation = records::RevocationOperation {
            format: 1,
            operation: ctx.commit_token(),
            action: self.action.clone(),
            authority: self.policy.clone(),
            actor: inv.actor(),
            authorization,
            term,
            charter,
            line,
            birth_operation,
            initial_operation: initial.commit,
            initial_issuer: initial.economic.issuer.clone(),
            filed,
            revision: file.content.revision(),
            reason: records::Reason::RegistryReview,
            at_ic,
        };
        operation.validate()?;
        let fence = RevocationFence {
            operation: operation.clone(),
            readiness: self.readiness.clone(),
        };
        let pending = self
            .pending
            .as_ref()
            .map(|(ready, economy)| {
                super::super::charter_pending::compound::prepare(
                    &self.families,
                    ready,
                    economy,
                    inv,
                    txn,
                    ctx,
                    &operation,
                    fence.clone(),
                )
            })
            .transpose()?
            .flatten();
        let funding = pending.as_ref().map(|(funding, _)| *funding);
        let resale = self
            .resale
            .as_ref()
            .map(|(writer, economy)| {
                super::super::paid_resale::compound::prepare(
                    writer,
                    economy,
                    txn,
                    ctx,
                    &operation,
                    fence.clone(),
                )
            })
            .transpose()?
            .flatten();
        let resale_funding = resale.as_ref().map(|(funding, _)| *funding);
        if pending.is_some() && resale.is_some() {
            return Err(PublicationError::Mismatch);
        }
        ctx.prepare_owner(
            &self.roots,
            RootRequest::RegistryRevocation(Box::new(Revocation {
                operation: operation.clone(),
                fence,
                pending: pending.map(|(_, roots)| roots),
                resale: resale.map(|(_, roots)| roots),
            })),
        )
        .map_err(bad)?;
        txn.insert(
            &self.families.registry_revocations,
            &operation.key(),
            operation.clone(),
        )
        .map_err(bad)?;
        let status = operation.status()?;
        txn.insert(&self.families.revoked_terms, &status.key(), status.clone())
            .map_err(bad)?;
        if txn
            .get(&self.families.registry_revocations, &operation.key())
            .map_err(bad)?
            != Some(&operation)
            || txn
                .get(&self.families.revoked_terms, &status.key())
                .map_err(bad)?
                != Some(&status)
        {
            return Err(PublicationError::Mismatch);
        }
        if let Some(funding) = funding {
            ctx.close_charter_pending_completion(txn, funding)
                .map_err(bad)?;
        }
        if let Some(funding) = resale_funding {
            ctx.close_paid_resale_completion(txn, funding)
                .map_err(bad)?;
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(
                self.outcome.clone(),
                json!({"licence":id,"generation":operation.term.generation(),"operation":operation.operation,"grace_end":operation.term.window().grace_end(),"reason":"registry_review"}),
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
                id:mush_core::mechanics::CanonicalId::new("licence").map_err(|_|fail())?,required:true,
                kind:mush_core::ProjectedParameterKind::Text {maximum_bytes:20},
            }]).map_err(|_|fail())?,targets:mush_core::ProjectedTargetSchema::None,
            required_channels:Vec::new(),expires:None,
            presentation:mush_core::ActionPresentation::new("roll-registry-revocation","Revoke a licence term",
                "Revoke the current accepted term through its original grace end. Running sorties retain their captured warrant.").map_err(|_|fail())?,
            guards:guards.finish().map_err(|_|fail())?,
        }))
    }
}
pub(crate) fn register_with_resale(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    ratings: super::super::rating::RatingFamilies,
    policy: RegistryRevocationPolicy,
    definitions: [DefinitionRef; 3],
    pending: Option<super::super::charter_pending::Readiness>,
    resale: Option<super::super::paid_resale::writer::PaidResaleWriter>,
) -> Result<Readiness, ModError> {
    let [authority, action, outcome] = definitions;
    if reg.module() != crate::NS || action != policy.action()? {
        return Err(error("Registry revocation registration differs"));
    }
    let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
    let readiness = Readiness::default();
    let (pending, resale) = if pending.is_some() || resale.is_some() {
        let economy = reg
            .request_owner_participant::<mush_economy::registry_filing::InitialFilingParticipant>(
                action.clone(),
            )?;
        (
            pending.map(|ready| (ready, economy.clone())),
            resale.map(|writer| (writer, economy.clone())),
        )
    } else {
        (None, None)
    };
    reg.action_provider(
        action.clone(),
        Provider {
            policy,
            action,
            outcome,
            authority,
            retention: super::super::authored_retirement::retention_reference()?,
            families,
            ratings,
            roots,
            readiness: readiness.clone(),
            pending,
            resale,
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "The Registry revocation is unavailable or stale.",
            )
            .map_err(error)?,
        },
    )?;
    Ok(readiness)
}
