use super::super::{
    TermGeneration,
    dependent_owner::OwnerParentBinding,
    families::CommerceFamilies,
    filing::InitialFilingTerms,
    owner_qualification::records::{QualificationHead, SelectedParent},
    parent_grant::{ParentGrantReader, ParentGrantRequest},
    participant::{RootParticipant, RootRequest},
    rating::{RatingCompatibility, RatingFamilies},
    records::{OwnerTerm, PublicationError, Record, RevisionPolicy},
};
use super::records::{AttestationEvidence, AttestationOperation, ParentQualification, term_digest};
use super::*;
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    PerceivedRef, PrincipalId,
};
use mush_core::participant::ParticipantHandle;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{ActionTxn, Flags, ModError, ObjKind, PrepareCtx, Registrar, RulesValue};
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
        let r: Self = serde_json::from_value(inv.parameters().clone()).map_err(invalid)?;
        r.licence()?;
        Ok(r)
    }
    pub(super) fn licence(&self) -> Result<LicenceId, PublicationError> {
        LicenceId::new(self.owner_licence).map_err(invalid)
    }
}
struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    manifest: DefinitionRef,
    retention: DefinitionRef,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
    terms: InitialFilingTerms,
    parent: Option<OwnerReadHandle<ParentGrantReader>>,
    roots: ParticipantHandle<RootParticipant>,
    readiness: super::super::withdrawal::EconomyReadiness,
    refusal: ActionError,
}
impl Provider {
    #[inline(never)]
    fn current_parent(
        &self,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        selected: (LicenceId, mush_core::ObjId),
        basis: (
            &super::super::records::InitialFilingOperation,
            &super::super::records::FiledRecord,
            &super::super::charter_transfer::records::CharterOwnershipRef,
        ),
    ) -> Result<Option<super::super::parent_grant::CurrentParentGrant>, PublicationError> {
        let (id, actor) = selected;
        let (birth, filed, ownership) = basis;
        let parent = if let Some(initial) = &birth.parent {
            let handle = self.parent.as_ref().ok_or(PublicationError::Unavailable)?;
            let binding = txn
                .get(
                    &self.families.owner_parents,
                    &OwnerParentBinding::key_for(id, birth.requester),
                )
                .map_err(invalid)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            let birth_term = txn
                .get(
                    &self.families.terms,
                    &OwnerTerm::encoded_key(id, birth.term),
                )
                .map_err(invalid)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            let birth_filed = txn
                .get(&self.families.filed, &binding.filed.key())
                .map_err(invalid)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            binding.check_rows(&birth_filed, &birth_term, birth)?;
            if filed.content.parent() != Some(binding.parent.parent) {
                return Err(PublicationError::Mismatch);
            }
            let source = txn
                .get(&self.families.filed, &initial.parent.filed.key())
                .map_err(invalid)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            let status = txn
                .get(&self.families.status, &initial.parent.filed.key())
                .map_err(invalid)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            source.validate()?;
            status.validate()?;
            if source.content.revision() != initial.parent.revision
                || source.content.position().charter != initial.charter
                || source.content.position().line != initial.line
                || status.filed != initial.parent.filed
                || status.recalled_by.is_some()
            {
                return Err(PublicationError::Unavailable);
            }
            let head = txn
                .get(
                    &self.families.owner_qualification_heads,
                    &QualificationHead::key_for(id, actor),
                )
                .map_err(invalid)?
                .cloned();
            let selected = match head {
                None if ownership.generation == 1 => initial.licence,
                None => return Err(PublicationError::Unavailable),
                Some(head) => {
                    let op = txn
                        .get(
                            &self.families.owner_qualification_operations,
                            &head.operation.to_hex(),
                        )
                        .map_err(invalid)?
                        .cloned()
                        .ok_or(PublicationError::Missing)?;
                    head.matches(&op)?;
                    op.matches_birth(&binding, &birth_filed, &birth_term, birth)?;
                    super::super::charter_transfer::history::require_captured_in_txn(
                        txn,
                        ctx.world().map_err(invalid)?,
                        &self.families,
                        birth,
                        op.charter_ownership.as_ref(),
                        actor,
                    )?;
                    if op.charter_ownership.as_ref()
                        != (ownership.generation > 1).then_some(ownership)
                    {
                        return Err(PublicationError::Unavailable);
                    }
                    ctx.world()
                        .map_err(invalid)?
                        .module_state::<crate::RollStore>()
                        .ok_or(PublicationError::Unavailable)?
                        .check_required_root(&op.root()?)
                        .map_err(invalid)?;
                    op.selected_parent.licence
                }
            };
            let actual = ctx
                .read_owner_preimage(txn, handle, ParentGrantRequest { licence: selected })
                .map_err(invalid)?;
            actual.check_fresh(actor, ctx.world().map_err(invalid)?)?;
            if actual.holding().charter() != initial.charter
                || actual.holding().line() != initial.line
            {
                return Err(PublicationError::Mismatch);
            }
            Some(actual)
        } else {
            None
        };
        Ok(parent)
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, PublicationError> {
        self.readiness.check()?;
        let id = Request::parse(inv)?.licence()?;
        let actor = inv.actor();
        let licence = txn
            .get(&self.families.licences, &id.key())
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        super::super::charter_pending::current::require_clear_in_txn(
            txn,
            ctx.world().map_err(invalid)?,
            &self.families,
            licence.charter,
        )?;
        let old =
            history::current_in_txn(txn, ctx.world().map_err(invalid)?, &self.families, &licence)?;
        super::super::registry_revocation::current::require_in_txn(
            txn,
            &self.families,
            id,
            old.generation,
            Some(
                super::super::WorldIcSeconds::from_stamp(
                    &ctx.world().map_err(invalid)?.world_ic_now(),
                )
                .map_err(invalid)?,
            ),
        )?;
        let birth = txn
            .get(&self.families.operations, &licence.operation.to_hex())
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let line = txn
            .get(&self.families.lines, &licence.line.key())
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let charter = txn
            .get(&self.families.charters, &licence.charter.key())
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let filed = txn
            .get(&self.families.filed, &line.head.key())
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let status = txn
            .get(&self.families.status, &line.head.key())
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        birth.validate()?;
        line.validate()?;
        charter.validate()?;
        filed.validate()?;
        status.validate()?;
        let ownership = super::super::charter_transfer::history::current_in_txn(
            txn,
            ctx.world().map_err(invalid)?,
            &self.families,
            &birth,
            actor,
        )?;
        if birth.economic.house.is_some()
            || birth.economic.issued_owner != mush_org::PrincipalId::Object(birth.requester)
            || charter.id != licence.charter
            || charter.line != licence.line
            || charter.operation != birth.commit
            || line.id != licence.line
            || line.charter != licence.charter
            || line.operation != birth.commit
            || filed.content.id() != line.head
            || filed.content.position().charter != licence.charter
            || filed.content.position().line != line.id
            || status.filed != line.head
            || status.withdrawn_by.is_some()
            || status.recalled_by.is_some()
        {
            return Err(PublicationError::Unavailable);
        }
        let current_birth = super::super::line_update::audit::birth_in_txn(
            txn,
            ctx.world().map_err(invalid)?,
            &self.families,
            &filed,
        )?;
        if current_birth.commit != birth.commit {
            return Err(PublicationError::Mismatch);
        }
        let parent = self.current_parent(txn, ctx, (id, actor), (&birth, &filed, &ownership))?;
        let rating = super::super::rating::reconformance::current::receipt_in_txn(
            txn,
            &self.families,
            &self.ratings,
            &filed,
            &self.expected,
        )
        .map_err(invalid)?;
        if rating.subject().revision() != filed.content.revision()
            || rating.subject().definition() != filed.definition
            || rating.subject().schema() != line.schema_digest
        {
            return Err(PublicationError::Mismatch);
        }
        let world = ctx.world().map_err(invalid)?;
        let observed = world.world_ic_now();
        let now = WorldIcSeconds::from_stamp(&observed).map_err(invalid)?;
        let title_revision =
            super::super::owner_qualification::current_title(world, actor, licence.charter)?;
        if now < old.window.term_end() {
            return Err(PublicationError::Unavailable);
        }
        let roll = world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        let revision = roll
            .revision(
                filed.content.revision().revision().key.character,
                filed.content.revision().revision().key,
            )
            .map_err(invalid)?;
        if !filed
            .content
            .revision()
            .matches(revision)
            .map_err(invalid)?
        {
            return Err(PublicationError::Mismatch);
        }
        if birth.parent.is_none() {
            revision.require_original_access().map_err(invalid)?;
        }
        let (term_seconds, grace_seconds) = self.terms.durations();
        let evidence = AttestationEvidence {
            term_seconds,
            grace_seconds,
            parent: match (&birth.parent, &parent) {
                (None, None) => None,
                (Some(initial), Some(actual)) => Some(ParentQualification {
                    source: initial.parent,
                    charter: initial.charter,
                    line: initial.line,
                    selected: SelectedParent::capture(actual.holding()),
                }),
                _ => return Err(PublicationError::Mismatch),
            },
        };
        let window = evidence.window(now)?;
        let next = OwnerTerm {
            format: if parent.is_some() { 4 } else { 3 },
            licence: id,
            generation: TermGeneration::new(
                old.generation
                    .get()
                    .checked_add(1)
                    .ok_or(PublicationError::Capacity)?,
            )
            .map_err(invalid)?,
            window,
            revision: RevisionPolicy::Follow { line: licence.line },
            operation: ctx.commit_token(),
            parent: None,
            attestation: Some(evidence),
        };
        let operation = AttestationOperation {
            format: if ownership.generation == 1 { 1 } else { 2 },
            charter_ownership: (ownership.generation > 1).then_some(ownership),
            operation: ctx.commit_token(),
            action: self.action.clone(),
            manifest: self.manifest.clone(),
            owner: actor,
            licence: id,
            charter: licence.charter,
            line: licence.line,
            birth_operation: birth.commit,
            predecessor: old.generation,
            predecessor_operation: old.operation,
            predecessor_digest: term_digest(&old)?,
            filed: filed.content.id(),
            revision: filed.content.revision(),
            rating: rating.reference().map_err(invalid)?,
            title_revision,
            observed_ic: observed.clone(),
            observed_utc: world.now(),
            next: next.clone(),
        };
        operation.follows(&old, &birth)?;
        let request = Attestation {
            operation: operation.clone(),
            fence: AttestationFence {
                action: self.action.clone(),
                commit: ctx.commit_token(),
                actor,
                licence: id,
                charter: licence.charter,
                title_revision,
                observed,
                utc: world.now(),
                predecessor: old.window,
                next: window,
                parent,
                readiness: self.readiness.clone(),
            },
        };
        ctx.prepare_owner(
            &self.roots,
            RootRequest::OwnerAttestation(Box::new(request)),
        )
        .map_err(invalid)?;
        let mut current = licence;
        current.format = 2;
        current.term = next.generation;
        txn.insert(
            &self.families.owner_attestations,
            &operation.key(),
            operation.clone(),
        )
        .map_err(invalid)?;
        txn.insert(&self.families.terms, &next.key(), next.clone())
            .map_err(invalid)?;
        txn.replace(&self.families.licences, &id.key(), current.clone())
            .map_err(invalid)?;
        if txn
            .get(&self.families.owner_attestations, &operation.key())
            .map_err(invalid)?
            != Some(&operation)
            || txn
                .get(&self.families.terms, &next.key())
                .map_err(invalid)?
                != Some(&next)
            || txn
                .get(&self.families.licences, &id.key())
                .map_err(invalid)?
                != Some(&current)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(AuthoritativeActionOutput::new(RulesValue::new(self.outcome.clone(),json!({"owner_licence":id.get(),"term":next.generation.get(),"operation":operation.operation})).map_err(invalid)?)
            .with_definitions(vec![self.action.clone(),self.outcome.clone(),self.manifest.clone(),self.retention.clone()])
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
        if self.readiness.check().is_err()
            || action != &self.action
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
        let parameters = ["owner_licence"]
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
            presentation:mush_core::ActionPresentation::new("roll-attest-owner-mind","Renew an owner mind licence",
                "Attest your own filed mind after its current term ends. This free renewal never changes paid customer terms.").map_err(|_|fail())?,
            guards:guards.finish().map_err(|_|fail())?,
        }))
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    families: CommerceFamilies,
    manifest: &DefinitionRef,
    parent: Option<OwnerReadHandle<ParentGrantReader>>,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
    terms: InitialFilingTerms,
    definitions: [DefinitionRef; 2],
) -> Result<super::super::withdrawal::EconomyReadiness, ModError> {
    let [action, outcome] = definitions;
    let (term_seconds, grace_seconds) = terms.durations();
    if reg.module() != crate::NS
        || action != protocol::action(manifest, term_seconds, grace_seconds).map_err(error)?
    {
        return Err(ModError::msg("owner attestation registration differs"));
    }
    let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
    let readiness = super::super::withdrawal::EconomyReadiness::default();
    reg.action_provider(
        action.clone(),
        Provider {
            action,
            outcome,
            manifest: manifest.clone(),
            retention: super::super::authored_retirement::retention_reference()?,
            families,
            ratings,
            expected,
            terms,
            parent,
            roots,
            readiness: readiness.clone(),
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "The owner licence is unavailable, not due, or no longer eligible for attestation.",
            )
            .map_err(error)?,
        },
    )?;
    Ok(readiness)
}
