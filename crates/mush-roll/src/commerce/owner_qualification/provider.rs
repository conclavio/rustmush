use super::super::{
    dependent_owner::OwnerParentBinding,
    families::CommerceFamilies,
    parent_grant::{ParentGrantReader, ParentGrantRequest},
    participant::{RootParticipant, RootRequest},
    records::{OwnerTerm, PublicationError, Record},
};
use super::records::{QualificationHead, QualificationOperation, SelectedParent};
use super::*;
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    PerceivedRef, PrincipalId,
};
use mush_core::participant::ParticipantHandle;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{ActionTxn, ModError, PrepareCtx, Registrar, RulesValue};
use serde::Deserialize;
use serde_json::json;

fn invalid<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    owner_licence: u64,
    parent_licence: u64,
}
impl Request {
    pub(super) fn parse(inv: &ActionInvocation) -> Result<Self, PublicationError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner.as_str() != crate::NS
        {
            return Err(PublicationError::Invalid);
        }
        let request: Self = serde_json::from_value(inv.parameters().clone()).map_err(invalid)?;
        if request.owner()? == request.parent()? {
            return Err(PublicationError::Invalid);
        }
        Ok(request)
    }
    pub(super) fn owner(&self) -> Result<LicenceId, PublicationError> {
        LicenceId::new(self.owner_licence).map_err(invalid)
    }
    pub(super) fn parent(&self) -> Result<LicenceId, PublicationError> {
        LicenceId::new(self.parent_licence).map_err(invalid)
    }
}
struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    manifest: DefinitionRef,
    retention: DefinitionRef,
    families: CommerceFamilies,
    parent: OwnerReadHandle<ParentGrantReader>,
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
        let request = Request::parse(inv)?;
        let owner_licence = request.owner()?;
        let actor = inv.actor();
        let licence = txn
            .get(&self.families.licences, &owner_licence.key())
            .map_err(invalid)?
            .ok_or(PublicationError::Missing)?
            .clone();
        // Lapsed metadata replacement remains legal. Prove the actual current
        // generation separately while all qualification history keeps birth term1.
        super::super::owner_attestation::history::current_in_txn(
            txn,
            ctx.world().map_err(invalid)?,
            &self.families,
            &licence,
        )?;
        let birth = txn
            .get(&self.families.operations, &licence.operation.to_hex())
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let ownership = super::super::charter_transfer::history::current_in_txn(
            txn,
            ctx.world().map_err(invalid)?,
            &self.families,
            &birth,
            actor,
        )?;
        let binding = txn
            .get(
                &self.families.owner_parents,
                &OwnerParentBinding::key_for(owner_licence, birth.requester),
            )
            .map_err(invalid)?
            .ok_or(PublicationError::Missing)?
            .clone();
        let filed = txn
            .get(&self.families.filed, &binding.filed.key())
            .map_err(invalid)?
            .ok_or(PublicationError::Missing)?
            .clone();
        let term = txn
            .get(
                &self.families.terms,
                &OwnerTerm::encoded_key(owner_licence, binding.term),
            )
            .map_err(invalid)?
            .ok_or(PublicationError::Missing)?
            .clone();
        let charter = txn
            .get(&self.families.charters, &licence.charter.key())
            .map_err(invalid)?
            .ok_or(PublicationError::Missing)?
            .clone();
        let line = txn
            .get(&self.families.lines, &licence.line.key())
            .map_err(invalid)?
            .ok_or(PublicationError::Missing)?
            .clone();
        licence.validate()?;
        charter.validate()?;
        line.validate()?;
        binding.check_rows(&filed, &term, &birth)?;
        if licence.id != owner_licence
            || licence.charter != binding.charter
            || licence.line != birth.line
            || licence.operation != binding.operation
            || binding.parent.owner != birth.requester
            || birth.economic.house.is_some()
            || birth.economic.issued_owner != mush_org::PrincipalId::Object(birth.requester)
            || charter.id != licence.charter
            || charter.line != licence.line
            || charter.operation != birth.commit
            || line.id != licence.line
            || line.charter != licence.charter
            || line.operation != birth.commit
            || line.head != filed.content.id()
        {
            return Err(PublicationError::Mismatch);
        }
        let source = txn
            .get(&self.families.filed, &binding.parent.parent.filed.key())
            .map_err(invalid)?
            .ok_or(PublicationError::Missing)?
            .clone();
        let source_status = txn
            .get(&self.families.status, &binding.parent.parent.filed.key())
            .map_err(invalid)?
            .ok_or(PublicationError::Missing)?
            .clone();
        source.validate()?;
        source_status.validate()?;
        if source.content.id() != binding.parent.parent.filed
            || source.content.revision() != binding.parent.parent.revision
            || source.content.position().charter != binding.parent.charter
            || source.content.position().line != binding.parent.line
            || source_status.filed != binding.parent.parent.filed
            || source_status.recalled_by.is_some()
        {
            return Err(PublicationError::Unavailable);
        }
        let head_key = QualificationHead::key_for(owner_licence, actor);
        let old_head = txn
            .get(&self.families.owner_qualification_heads, &head_key)
            .map_err(invalid)?
            .cloned();
        let old_operation = match &old_head {
            None => None,
            Some(head) => {
                let op = txn
                    .get(
                        &self.families.owner_qualification_operations,
                        &head.operation.to_hex(),
                    )
                    .map_err(invalid)?
                    .ok_or(PublicationError::Missing)?
                    .clone();
                head.matches(&op)?;
                op.matches_birth(&binding, &filed, &term, &birth)?;
                super::super::charter_transfer::history::require_captured_in_txn(
                    txn,
                    ctx.world().map_err(invalid)?,
                    &self.families,
                    &birth,
                    op.charter_ownership.as_ref(),
                    actor,
                )?;
                Some(op)
            }
        };
        let title_revision = {
            let world = ctx.world().map_err(invalid)?;
            let roll = world
                .module_state::<crate::RollStore>()
                .ok_or(PublicationError::Unavailable)?;
            roll.commerce_ready()
                .map_err(|_| PublicationError::Unavailable)?;
            current_title(world, actor, licence.charter)?
        };
        // One actual shared owner-created grant. Its typed reads and immutable
        // owner observations remain attached through stage/take.
        let parent = ctx
            .read_owner_preimage(
                txn,
                &self.parent,
                ParentGrantRequest {
                    licence: request.parent()?,
                },
            )
            .map_err(invalid)?;
        let world = ctx.world().map_err(invalid)?;
        parent.check_fresh(actor, world)?;
        if parent.actor() != actor
            || parent.holding().charter() != binding.parent.charter
            || parent.holding().line() != binding.parent.line
        {
            return Err(PublicationError::Mismatch);
        }
        let operation = QualificationOperation {
            format: if ownership.generation == 1 { 1 } else { 2 },
            charter_ownership: (ownership.generation > 1).then_some(ownership),
            operation: ctx.commit_token(),
            action: self.action.clone(),
            manifest: self.manifest.clone(),
            owner: actor,
            owner_licence,
            owner_term: binding.term,
            charter: licence.charter,
            line: licence.line,
            filed: filed.content.id(),
            child_revision: filed.content.revision(),
            birth_operation: birth.commit,
            title_revision,
            source_parent: binding.parent.parent,
            parent_charter: binding.parent.charter,
            parent_line: binding.parent.line,
            selected_parent: SelectedParent::capture(parent.holding()),
            ordinal: old_head
                .as_ref()
                .map_or(Some(1), |h| h.ordinal.checked_add(1))
                .ok_or(PublicationError::Capacity)?,
            previous: old_head.as_ref().map(|h| h.operation),
            observed_ic: world.world_ic_now(),
        };
        operation.matches_birth(&binding, &filed, &term, &birth)?;
        operation.follows(old_operation.as_ref())?;
        let next_head = operation.head();
        let request = Qualification {
            operation: operation.clone(),
            fence: QualificationFence {
                action: self.action.clone(),
                commit: ctx.commit_token(),
                actor,
                owner_licence,
                charter: licence.charter,
                title_revision,
                parent,
            },
        };
        ctx.prepare_owner(&self.roots, RootRequest::OwnerQualification(request))
            .map_err(invalid)?;
        txn.insert(
            &self.families.owner_qualification_operations,
            &operation.key(),
            operation.clone(),
        )
        .map_err(invalid)?;
        match old_head {
            None => txn
                .insert(
                    &self.families.owner_qualification_heads,
                    &head_key,
                    next_head.clone(),
                )
                .map_err(invalid)?,
            Some(_) => txn
                .replace(
                    &self.families.owner_qualification_heads,
                    &head_key,
                    next_head.clone(),
                )
                .map_err(invalid)?,
        }
        if txn
            .get(
                &self.families.owner_qualification_operations,
                &operation.key(),
            )
            .map_err(invalid)?
            != Some(&operation)
            || txn
                .get(&self.families.owner_qualification_heads, &head_key)
                .map_err(invalid)?
                != Some(&next_head)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(
                self.outcome.clone(),
                json!({"owner_licence":owner_licence.get(),"operation":operation.operation}),
            )
            .map_err(invalid)?,
        )
        .with_definitions(vec![
            self.action.clone(),
            self.outcome.clone(),
            self.manifest.clone(),
            self.retention.clone(),
        ])
        .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        Request::parse(inv)
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
            || query.projected_subject() != &PerceivedRef::SelfActor
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || engine
                .world
                .get(query.actor())
                .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
            || engine
                .world
                .module_state::<crate::RollStore>()
                .is_none_or(|r| r.commerce_ready().is_err())
        {
            return Ok(None);
        }
        let fail = || self.refusal.clone();
        let defs = engine.registry.definitions().ok_or_else(fail)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        for d in [&self.action, &self.manifest, &self.retention] {
            guards.add_definition(d.clone(), defs).map_err(|_| fail())?;
        }
        let parameters = ["owner_licence", "parent_licence"]
            .into_iter()
            .map(|id| {
                Ok(mush_core::ProjectedParameter {
                    id: mush_core::CanonicalId::new(id).map_err(|_| fail())?,
                    required: true,
                    kind: mush_core::ProjectedParameterKind::Integer {
                        minimum: 1,
                        maximum: i64::MAX,
                    },
                })
            })
            .collect::<Result<Vec<_>, ActionError>>()?;
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters:json!({}), parameters:mush_core::ProjectedParameterSchema::new(parameters).map_err(|_|fail())?,
            targets:mush_core::ProjectedTargetSchema::None,required_channels:Vec::new(),expires:None,
            presentation:mush_core::ActionPresentation::new("roll-replace-owner-parent","Restore a dependent owner's parent rights",
                "Select your dependent owner licence and a current Open licence on its original parent line. Existing child dates remain unchanged.").map_err(|_|fail())?,
            guards:guards.finish().map_err(|_|fail())?,
        }))
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    manifest: &DefinitionRef,
    parent: OwnerReadHandle<ParentGrantReader>,
    definitions: [DefinitionRef; 2],
) -> Result<(), ModError> {
    let [action, outcome] = definitions;
    if reg.module() != crate::NS || action != protocol::action(manifest).map_err(error)? {
        return Err(ModError::msg("owner qualification registration differs"));
    }
    let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
    reg.action_provider(
        action.clone(),
        Provider {
            action,
            outcome,
            manifest: manifest.clone(),
            retention: super::super::authored_retirement::retention_reference()?,
            families,
            parent,
            roots,
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "The dependent owner or qualifying parent licence is unavailable or stale.",
            )
            .map_err(error)?,
        },
    )
}
