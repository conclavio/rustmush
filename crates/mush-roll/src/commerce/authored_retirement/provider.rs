use super::super::families::CommerceFamilies;
use super::super::participant::{RootParticipant, RootRequest};
use super::super::records::Record;
use super::reader::{MetadataReader, MetadataRequest};
use super::*;
use mush_core::action::{ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput};
use mush_core::mechanics::RulesValue;
use mush_core::participant::ParticipantHandle;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{ActionTxn, PrepareCtx, Registrar};
use std::collections::BTreeSet;
struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    retention: DefinitionRef,
    families: CommerceFamilies,
    reader: OwnerReadHandle<MetadataReader>,
    roots: ParticipantHandle<RootParticipant>,
    refusal: ActionError,
}
impl Provider {
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, PublicationError> {
        let request = Request::parse(inv, &self.action)?;
        let base = request.base()?;
        let subject = ctx
            .read_owner_preimage(
                txn,
                &self.reader,
                MetadataRequest {
                    name: request.name,
                    mind: request.mind,
                    base,
                },
            )
            .map_err(bad)?;
        if subject.actor != inv.actor() || subject.base != base {
            return Err(PublicationError::Mismatch);
        }
        let key = subject.selector.key();
        if txn.get(&self.families.draft_parents, &key).map_err(bad)? != Some(&subject.selector)
            || txn
                .get(&self.families.retired_draft_parents, &key)
                .map_err(bad)?
                .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let commit = ctx.commit_token();
        let output = ctx
            .prepare_owner(
                &self.roots,
                RootRequest::Retirement(Box::new(Retirement {
                    action: self.action.clone(),
                    commit,
                    subject,
                })),
            )
            .map_err(bad)?
            .retirement()?;
        output.operation.validate()?;
        output.tombstone.validate()?;
        if output.operation.operation != commit
            || output.operation.actor != inv.actor()
            || output.tombstone != output.operation.tombstone()?
        {
            return Err(PublicationError::Mismatch);
        }
        txn.insert(
            &self.families.authored_retirements,
            &output.operation.key(),
            output.operation.clone(),
        )
        .map_err(bad)?;
        txn.insert(
            &self.families.retired_draft_parents,
            &output.tombstone.key(),
            output.tombstone.clone(),
        )
        .map_err(bad)?;
        if txn
            .get(&self.families.authored_retirements, &output.operation.key())
            .map_err(bad)?
            != Some(&output.operation)
            || txn
                .get(
                    &self.families.retired_draft_parents,
                    &output.tombstone.key(),
                )
                .map_err(bad)?
                != Some(&output.tombstone)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(
                self.outcome.clone(),
                json!({"mind":output.operation.mind,"operation":commit}),
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
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
            || engine
                .world
                .module_state::<crate::RollStore>()
                .is_none_or(|s| s.commerce_ready().is_err())
            || engine.world.get(query.actor()).is_none_or(|o| {
                o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
            })
        {
            return Ok(None);
        }
        let bad = || self.refusal.clone();
        let defs = engine.registry.definitions().ok_or_else(bad)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| bad())?;
        for definition in [&self.action, &self.retention] {
            guards
                .add_definition(definition.clone(), defs)
                .map_err(|_| bad())?;
        }
        let mut fields = vec![("name", 64), ("edit", 20), ("revision", 20)];
        if self.action.version() == 2 {
            fields.push(("mind", 20));
        }
        let parameters = fields
            .into_iter()
            .map(|(name, maximum_bytes)| {
                Ok(mush_core::ProjectedParameter {
                    id: mush_core::mechanics::CanonicalId::new(name).map_err(|_| bad())?,
                    required: true,
                    kind: mush_core::ProjectedParameterKind::Text { maximum_bytes },
                })
            })
            .collect::<Result<Vec<_>, ActionError>>()?;
        Ok(Some(mush_core::ProviderOfferDraft {fixed_parameters:json!({}),parameters:mush_core::ProjectedParameterSchema::new(parameters).map_err(|_|bad())?,
            targets:mush_core::ProjectedTargetSchema::None,required_channels:Vec::new(),expires:None,
            presentation:mush_core::ActionPresentation::new("roll-retire-authored-draft","Remove an attributed working draft","Remove your working library entry while retaining saved history and existing grants.").map_err(|_|bad())?,
            guards:guards.finish().map_err(|_|bad())?}))
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    definitions: [DefinitionRef; 4],
) -> Result<(), ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("retirement belongs to Roll"));
    }
    let [action, outcome, metadata, captured] = definitions;
    if action != action_reference()? || captured != captured_action_reference()? {
        return Err(ModError::msg("retirement action differs"));
    }
    reg.owner_read_service(
        metadata.clone(),
        BTreeSet::from([OwnerId::new(crate::NS).map_err(error)?]),
        BTreeSet::new(),
        MetadataReader {
            families: families.clone(),
        },
    )?;
    let reader = reg.request_owner_read::<MetadataReader>(metadata)?;
    for action in [action, captured] {
        let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
        reg.action_provider(
            action.clone(),
            Provider {
                action,
                outcome: outcome.clone(),
                retention: retention_reference()?,
                families: families.clone(),
                reader: reader.clone(),
                roots,
                refusal: ActionError::new(
                    ActionErrorCode::ProviderRejected,
                    false,
                    "The owned attributed draft is unavailable or stale.",
                )
                .map_err(error)?,
            },
        )?;
    }
    Ok(())
}
