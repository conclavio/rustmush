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
    action: DefinitionRef,
    outcome: DefinitionRef,
    retention: DefinitionRef,
    families: CommerceFamilies,
    roots: ParticipantHandle<RootParticipant>,
    readiness: EconomyReadiness,
    refusal: ActionError,
}
impl Provider {
    fn subject(
        &self,
        actor: ObjId,
        filed: FiledRevisionId,
        txn: &mut ActionTxn<'_>,
        ctx: &PrepareCtx<'_>,
    ) -> Result<Subject, PublicationError> {
        self.readiness.check()?;
        let world = ctx.world().map_err(bad)?;
        let roll = world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        roll.commerce_ready()
            .map_err(|_| PublicationError::Unavailable)?;
        let file = txn
            .get(&self.families.filed, &filed.key())
            .map_err(bad)?
            .ok_or(PublicationError::Missing)?
            .clone();
        let initial =
            super::super::line_update::audit::birth_in_txn(txn, world, &self.families, &file)?;
        let status = txn
            .get(&self.families.status, &filed.key())
            .map_err(bad)?
            .ok_or(PublicationError::Missing)?
            .clone();
        file.validate()?;
        initial.validate()?;
        status.validate()?;
        if initial.requester != actor
            || initial.revision.revision().key.character != actor
            || initial.economic.house.is_some()
            || initial.economic.issued_owner != mush_org::PrincipalId::Object(actor)
            || (file.format != 3 && initial.filed != filed)
            || file.content.id() != filed
            || status.filed != filed
            || file.content.revision().revision().key.character != actor
            || (file.format != 3
                && (file.content.revision() != initial.revision
                    || initial.commit != file.operation))
            || status.withdrawn_by.is_some()
            || status.recalled_by.is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        roll.check_required_root(&RequiredRoot {
            anchor: RootAnchor::Filed(filed),
            identity: file.content.revision(),
        })
        .map_err(|_| PublicationError::Mismatch)?;
        Ok(Subject {
            actor,
            title_revision: current_title(world, actor, initial.charter)?,
            initial,
            filed: file,
            status,
        })
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, PublicationError> {
        let filed = Request::parse(inv, &self.action)?;
        let subject = self.subject(inv.actor(), filed, txn, ctx)?;
        if subject.actor != inv.actor()
            || txn.get(&self.families.filed, &filed.key()).map_err(bad)? != Some(&subject.filed)
            || txn
                .get(&self.families.operations, &subject.initial.key())
                .map_err(bad)?
                != Some(&subject.initial)
            || txn.get(&self.families.status, &filed.key()).map_err(bad)? != Some(&subject.status)
        {
            return Err(PublicationError::Mismatch);
        }
        let request = Withdrawal::new(
            &self.action,
            ctx.commit_token(),
            &subject,
            self.readiness.clone(),
        )?;
        let operation = request.operation.clone();
        let status = operation.status();
        ctx.prepare_owner(&self.roots, RootRequest::Withdrawal(request))
            .map_err(bad)?;
        txn.insert(
            &self.families.withdrawals,
            &operation.key(),
            operation.clone(),
        )
        .map_err(bad)?;
        txn.replace(&self.families.status, &filed.key(), status.clone())
            .map_err(bad)?;
        if txn
            .get(&self.families.withdrawals, &operation.key())
            .map_err(bad)?
            != Some(&operation)
            || txn.get(&self.families.status, &filed.key()).map_err(bad)? != Some(&status)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(
                self.outcome.clone(),
                json!({"filed":filed,"operation":operation.operation}),
            )
            .map_err(bad)?,
        )
        .with_definitions(vec![
            self.action.clone(),
            self.outcome.clone(),
            self.retention.clone(),
        ])
        .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        Request::parse(inv, &self.action)
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
            || self.readiness.check().is_err()
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
            || engine
                .world
                .get(query.actor())
                .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
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
        for definition in [&self.action, &self.retention] {
            guards
                .add_definition(definition.clone(), defs)
                .map_err(|_| fail())?;
        }
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters:json!({}),
            parameters:mush_core::ProjectedParameterSchema::new(vec![mush_core::ProjectedParameter {
                id:mush_core::mechanics::CanonicalId::new("filed").map_err(|_|fail())?,required:true,
                kind:mush_core::ProjectedParameterKind::Text {maximum_bytes:20},
            }]).map_err(|_|fail())?,
            targets:mush_core::ProjectedTargetSchema::None,required_channels:Vec::new(),expires:None,
            presentation:mush_core::ActionPresentation::new("roll-withdraw-authored-mind","Withdraw an authored filed mind","Stop fresh offers for a filed revision you authored and still personally own. Existing paid Pins remain usable.").map_err(|_|fail())?,
            guards:guards.finish().map_err(|_|fail())?,
        }))
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    definitions: [DefinitionRef; 2],
) -> Result<EconomyReadiness, ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("withdrawal belongs to Roll"));
    }
    let [action, outcome] = definitions;
    if action != action_reference()? {
        return Err(ModError::msg("withdrawal action differs"));
    }
    let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
    let readiness = EconomyReadiness::default();
    reg.action_provider(
        action.clone(),
        Provider {
            action,
            outcome,
            retention: super::super::authored_retirement::retention_reference()?,
            families,
            roots,
            readiness: readiness.clone(),
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "The personally authored filed revision is unavailable or stale.",
            )
            .map_err(error)?,
        },
    )?;
    Ok(readiness)
}
