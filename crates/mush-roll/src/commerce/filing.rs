//! The closed initial original filing. Every post-image is derived from one
//! acknowledged shell, exact saved revision and actual Registry/E4 results.
//! This provider neither renews a term nor interprets a derivative as original.

use mush_core::action::{
    ActionError, ActionInvocation, ActionProvider, AuthoritativeActionOutput, PerceivedRef,
    PrincipalId as Actor,
};
use mush_core::action_state::{ActionTxn, StateFamilyHandle};
use mush_core::mechanics::{DefinitionRef, RulesValue};
use mush_core::participant::ParticipantHandle;
use mush_core::reserved_shell::{ShellKey, ShellReservations};
use mush_core::{ClockStamp, ObjId, PrepareCtx};
use mush_economy::AccountId;
use mush_economy::registry_filing::{
    EconomyRequest, InitialFilingOutput, InitialFilingParticipant, RegistryHouseFiling,
    RegistryInitialFiling,
};
use mush_org::PrincipalId;
use serde::Deserialize;
use serde_json::json;

use super::families::CommerceFamilies;
use super::filed::{FiledRevisionContent, FilingPosition, FilingSequence};
use super::participant::{RootParticipant, RootRequest};
use super::rating::{RatingCompatibility, RatingFamilies, RatingReceipt, RatingRef};
use super::records::*;
use super::roots::InitialRoots;
use super::{
    CharterId, FiledRevisionId, LicenceId, LineId, MAX_GRACE_SECONDS, MAX_OWNER_TERM_SECONDS,
    RevisionIdentity, TermGeneration, TermWindow, WorldIcSeconds,
};
use crate::{Revision, RevisionRef, RollStore};

/// Only actual composition supplies these frozen product terms. A client's
/// request cannot select its term length, fee, issuer, class or compatibility.
/// The bound is the owner bound: a House owner term may run to 365 days (17g C0),
/// while composition keeps personal and dependent owner terms at 90.
#[derive(Clone, Debug)]
pub struct InitialFilingTerms {
    term_seconds: i64,
    grace_seconds: i64,
}
impl InitialFilingTerms {
    pub fn new(term_seconds: i64, grace_seconds: i64) -> Result<Self, PublicationError> {
        if !(1..=MAX_OWNER_TERM_SECONDS).contains(&term_seconds)
            || !(0..=MAX_GRACE_SECONDS).contains(&grace_seconds)
        {
            return Err(PublicationError::Invalid);
        }
        Ok(Self {
            term_seconds,
            grace_seconds,
        })
    }
    pub(crate) const fn durations(&self) -> (i64, i64) {
        (self.term_seconds, self.grace_seconds)
    }
    pub(crate) fn payload(&self) -> serde_json::Value {
        json!({"format":1,"original_only":true,"term_seconds":self.term_seconds,
            "grace_seconds":self.grace_seconds,"owner_revision_policy":"follow"})
    }
    pub(crate) fn dependent_payload(&self) -> serde_json::Value {
        json!({"format":1,"protocol":"personal_dependent_filing_v1",
            "parent":"current_immediate_open_grant","term_seconds":self.term_seconds,
            "grace_seconds":self.grace_seconds,"owner_revision_policy":"follow","sales":false})
    }
    fn window(&self, issued: &ClockStamp) -> Result<TermWindow, PublicationError> {
        let now = WorldIcSeconds::from_stamp(issued).map_err(invalid)?;
        let mut end = issued.clone();
        end.at = end
            .at
            .checked_add(self.term_seconds)
            .ok_or(PublicationError::Capacity)?;
        let mut grace = end.clone();
        grace.at = grace
            .at
            .checked_add(self.grace_seconds)
            .ok_or(PublicationError::Capacity)?;
        TermWindow::new(
            now,
            now,
            WorldIcSeconds::from_stamp(&end).map_err(invalid)?,
            WorldIcSeconds::from_stamp(&grace).map_err(invalid)?,
        )
        .map_err(invalid)
    }
}

struct Request {
    shell: ShellKey,
    revision: RevisionRef,
    rating: RatingRef,
    payer: AccountId,
    charter_name: String,
    house: Option<(mush_org::OrgId, mush_org::OrgScopeId)>,
}

/// Flat bounded offer fields; actor identity is always supplied by the kernel.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestWire {
    shell_request: mush_core::RequestUid,
    mind: u64,
    revision: u64,
    revision_digest: mush_core::mechanics::DefinitionDigest,
    rating_job: mush_core::CommitToken,
    rating_id: super::rating::RatingId,
    rating_digest: mush_core::mechanics::DefinitionDigest,
    payer: AccountId,
    charter_name: String,
}

pub(crate) struct InitialFilingBuilder {
    pub(crate) house: bool,
    pub(crate) dependent: Option<
        mush_core::read_service::OwnerReadHandle<super::dependent_filing::FilingParentReader>,
    >,
    pub(crate) action: DefinitionRef,
    pub(crate) outcome: DefinitionRef,
    pub(crate) manifest: DefinitionRef,
    pub(crate) compatibility: RatingCompatibility,
    pub(crate) terms: InitialFilingTerms,
    pub(crate) families: CommerceFamilies,
    pub(crate) ratings: RatingFamilies,
    pub(crate) shells: ShellReservations,
    pub(crate) economy: ParticipantHandle<InitialFilingParticipant>,
    pub(crate) roots: ParticipantHandle<RootParticipant>,
    pub(crate) refusal: ActionError,
}

fn invalid<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}

/// The one subject shared by every image. It is internal and not deserializable.
struct Subject {
    actor: ObjId,
    revision: Revision,
    identity: RevisionIdentity,
    issued: ClockStamp,
    parent: Option<super::dependent_owner::DependentParent>,
}

impl InitialFilingBuilder {
    fn request(&self, inv: &ActionInvocation) -> Result<Request, PublicationError> {
        if inv.principal() != &(Actor::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner != self.action.key().owner
        {
            return Err(PublicationError::Invalid);
        }
        let mut parameters = inv.parameters().clone();
        let house = if self.house {
            let fields = parameters
                .as_object_mut()
                .ok_or(PublicationError::Invalid)?;
            let organization = serde_json::from_value(
                fields
                    .remove("organization")
                    .ok_or(PublicationError::Invalid)?,
            )
            .map_err(invalid)?;
            let scope =
                serde_json::from_value(fields.remove("scope").ok_or(PublicationError::Invalid)?)
                    .map_err(invalid)?;
            Some((organization, scope))
        } else {
            None
        };
        let wire: RequestWire = serde_json::from_value(parameters).map_err(invalid)?;
        let request = Request {
            shell: serde_json::from_value(
                json!({"requester":inv.actor(),"request":wire.shell_request}),
            )
            .map_err(invalid)?,
            revision: RevisionRef {
                key: crate::RevisionKey {
                    character: inv.actor(),
                    mind: wire.mind,
                    number: wire.revision,
                },
                digest: wire.revision_digest,
            },
            rating: serde_json::from_value(
                json!({"job":wire.rating_job,"id":wire.rating_id,"schema":1,"receipt_digest":wire.rating_digest}),
            )
            .map_err(invalid)?,
            payer: wire.payer,
            charter_name: wire.charter_name,
            house,
        };
        if request.shell.requester() != inv.actor()
            || request.revision.key.character != inv.actor()
            || request.charter_name.is_empty()
            || request.charter_name.len() > MAX_CHARTER_NAME_BYTES
            || request.charter_name.trim() != request.charter_name
            || request.charter_name.chars().any(char::is_control)
        {
            return Err(PublicationError::Invalid);
        }
        Ok(request)
    }

    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, PublicationError> {
        let request = self.request(inv)?;
        if self.house && self.dependent.is_some() {
            return Err(PublicationError::Invalid);
        }
        let parent_access = match &self.dependent {
            Some(reader) => Some(
                context
                    .read_owner_preimage(
                        txn,
                        reader,
                        super::dependent_filing::FilingParentRequest {
                            revision: request.revision,
                        },
                    )
                    .map_err(invalid)?,
            ),
            None => None,
        };
        let subject = {
            let world = context.world().map_err(invalid)?;
            let store = world
                .module_state::<RollStore>()
                .ok_or(PublicationError::Unavailable)?;
            store.commerce_ready().map_err(invalid)?;
            // Active owned-library membership is part of original publication.
            // A source/revision key alone is never an ownership oracle.
            store
                .owned_mind(inv.actor(), request.revision.key.mind)
                .map_err(invalid)?;
            let revision = store
                .revision(inv.actor(), request.revision.key)
                .map_err(invalid)?;
            // This closed action issues an original charter only. Owning a
            // marked working copy cannot relabel its ancestry as parent=None.
            let parent = match &parent_access {
                None => {
                    revision.require_original_access().map_err(invalid)?;
                    None
                }
                Some(access) => {
                    if revision != access.revision() {
                        return Err(PublicationError::Mismatch);
                    }
                    Some(access.parent(self.terms.term_seconds, self.terms.grace_seconds)?)
                }
            };
            if revision.reference() != request.revision {
                return Err(PublicationError::Mismatch);
            }
            Subject {
                actor: inv.actor(),
                identity: RevisionIdentity::capture(revision).map_err(invalid)?,
                revision: revision.clone(),
                issued: world.world_ic_now(),
                parent,
            }
        };
        // These are actual typed reads in the transaction's global revision
        // fence. The manifest is also an exact definition guard on the action.
        let rating = self
            .ratings
            .complete_active_receipt_in_txn(txn, &request.rating, &self.compatibility)
            .map_err(invalid)?;
        let schema = schema_line_digest(subject.revision.envelope())?;
        let duplicate = canonical_duplicate_digest(&subject.revision)?;
        if rating.subject().revision() != subject.identity || rating.subject().schema() != schema {
            return Err(PublicationError::Mismatch);
        }
        let shell = context
            .resolve_initial_filing_shell(&self.shells, txn, request.shell)
            .map_err(invalid)?;
        if shell.requester() != subject.actor {
            return Err(PublicationError::Mismatch);
        }
        let charter = CharterId::from_object(shell.object()).map_err(invalid)?;
        let commit = context.commit_token();
        let mut deployment = txn
            .get(&self.families.deployment, &"registry".to_owned())
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Unavailable)?;
        deployment.validate()?;
        let line = LineId::new(deployment.next_line).map_err(invalid)?;
        let filed = FiledRevisionId::new(deployment.next_filed).map_err(invalid)?;
        let licence = LicenceId::new(deployment.next_licence).map_err(invalid)?;
        let term = TermGeneration::new(1).map_err(invalid)?;
        deployment.next_line = deployment
            .next_line
            .checked_add(1)
            .ok_or(PublicationError::Capacity)?;
        deployment.next_filed = deployment
            .next_filed
            .checked_add(1)
            .ok_or(PublicationError::Capacity)?;
        deployment.next_licence = deployment
            .next_licence
            .checked_add(1)
            .ok_or(PublicationError::Capacity)?;

        let economic_request = match request.house {
            Some((organization, scope)) => {
                EconomyRequest::HouseInitialFiling(RegistryHouseFiling {
                    shell: shell.object(),
                    payer: request.payer,
                    organization,
                    scope,
                })
            }
            None => {
                let request = RegistryInitialFiling {
                    shell: shell.object(),
                    payer: request.payer,
                    owner: PrincipalId::Object(subject.actor),
                    scope: None,
                };
                if subject.parent.is_some() {
                    EconomyRequest::DependentInitialFiling(request)
                } else {
                    request.into()
                }
            }
        };
        let economic = context
            .prepare_owner(&self.economy, economic_request)
            .map_err(invalid)?
            .initial_filing()
            .map_err(invalid)?;
        let content = FiledRevisionContent::capture(
            filed,
            FilingPosition {
                charter,
                line,
                sequence: FilingSequence::new(1).map_err(invalid)?,
            },
            &subject.revision,
            WorldIcSeconds::from_stamp(&subject.issued).map_err(invalid)?,
            subject.parent.as_ref().map(|p| p.parent),
        )
        .map_err(invalid)?;
        let expected_window = match &subject.parent {
            Some(parent) => parent
                .child_window(WorldIcSeconds::from_stamp(&subject.issued).map_err(invalid)?)?,
            None => self.terms.window(&subject.issued)?,
        };
        let rows = InitialRows {
            charter: CharterRecord {
                format: PUBLICATION_FORMAT,
                id: charter,
                name: request.charter_name,
                shell_receipt: shell.receipt(),
                operation: commit,
                line,
            },
            line: LineRecord {
                format: PUBLICATION_FORMAT,
                id: line,
                charter,
                schema_digest: schema,
                head: filed,
                next_sequence: 2,
                operation: commit,
            },
            filed: FiledRecord {
                format: if subject.parent.is_some() {
                    2
                } else {
                    PUBLICATION_FORMAT
                },
                content,
                definition: rating.subject().definition(),
                duplicate,
                rating: request.rating,
                operation: commit,
            },
            status: RevisionStatus {
                format: PUBLICATION_FORMAT,
                filed,
                withdrawn_by: None,
                recalled_by: None,
            },
            licence: OwnerLicence {
                format: PUBLICATION_FORMAT,
                id: licence,
                charter,
                line,
                term,
                operation: commit,
            },
            term: OwnerTerm {
                attestation: None,
                parent: subject.parent.clone(),
                format: if subject.parent.is_some() {
                    2
                } else {
                    PUBLICATION_FORMAT
                },
                licence,
                generation: term,
                window: expected_window,
                revision: RevisionPolicy::Follow { line },
                operation: commit,
            },
            operation: InitialFilingOperation {
                parent: subject.parent.clone(),
                format: if subject.parent.is_some() {
                    3
                } else if self.house {
                    2
                } else {
                    PUBLICATION_FORMAT
                },
                commit,
                requester: subject.actor,
                shell: shell.key(),
                shell_receipt: shell.receipt(),
                charter,
                line,
                filed,
                licence,
                term,
                revision: subject.identity,
                rating: request.rating,
                economic: FilingEconomicReceipt::from_prepared(&economic)?,
            },
            identity: FiledIdentityIndex {
                format: PUBLICATION_FORMAT,
                duplicate,
                revision: subject.identity,
                filed,
            },
            position: FiledPositionIndex {
                format: PUBLICATION_FORMAT,
                line,
                filed,
            },
        };
        rows.validate(&subject, &economic, &rating, expected_window)?;
        let binding =
            subject
                .parent
                .as_ref()
                .map(|parent| super::dependent_owner::OwnerParentBinding {
                    format: 1,
                    licence,
                    term,
                    charter,
                    filed,
                    parent: parent.clone(),
                    operation: commit,
                });
        if let Some(binding) = &binding {
            binding.check_rows(&rows.filed, &rows.term, &rows.operation)?;
        }
        super::dependent_owner::check_prepared_links(
            txn,
            &self.families,
            context
                .world()
                .map_err(invalid)?
                .module_state::<RollStore>()
                .ok_or(PublicationError::Unavailable)?,
            &rows.filed.content,
            &rows.line,
        )?;
        // Only the one complete builder writes these post-images. No same-commit
        // triggers are emitted by this initial-filing provider.
        rows.stage(txn, &self.families)?;
        if let Some(binding) = &binding {
            txn.insert(
                &self.families.owner_parents,
                &binding.key(),
                binding.clone(),
            )
            .map_err(invalid)?;
            if txn
                .get(&self.families.owner_parents, &binding.key())
                .map_err(invalid)?
                != Some(binding)
            {
                return Err(PublicationError::Mismatch);
            }
        }
        txn.replace(
            &self.families.deployment,
            &"registry".to_owned(),
            deployment,
        )
        .map_err(invalid)?;
        rows.check_staged(txn, &self.families)?;
        // Derive roots from the checked actual post-images, not caller tuples.
        let roots = InitialRoots::from_records(&rows.filed, &rows.term, &rows.operation)?
            .with_follow(&rows.filed, &rating)?;
        context
            .consume_initial_filing_shell(&self.shells, txn, shell)
            .map_err(invalid)?;
        let root_request = match parent_access {
            Some(access) => RootRequest::DependentFiling(Box::new(access.into_roots(
                self.action.clone(),
                roots,
                expected_window,
            ))),
            None => RootRequest::InitialFiling(Box::new(roots)),
        };
        let _usage = context
            .prepare_owner(&self.roots, root_request)
            .map_err(invalid)?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({"charter":charter,
            "line":line,"filed":filed,"licence":licence,"operation":commit}),
        )
        .map_err(invalid)?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![self.outcome.clone(), self.manifest.clone()])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}

/// Kept together so validation and staged-read equality cannot accidentally
/// accept an individually valid row from another revision, shell or E4 result.
#[derive(Clone)]
struct InitialRows {
    charter: CharterRecord,
    line: LineRecord,
    filed: FiledRecord,
    status: RevisionStatus,
    licence: OwnerLicence,
    term: OwnerTerm,
    operation: InitialFilingOperation,
    identity: FiledIdentityIndex,
    position: FiledPositionIndex,
}
impl InitialRows {
    fn validate(
        &self,
        subject: &Subject,
        economic: &InitialFilingOutput,
        rating: &RatingReceipt,
        expected_window: TermWindow,
    ) -> Result<(), PublicationError> {
        self.check_subject(
            subject,
            economic.commit_token(),
            &FilingEconomicReceipt::from_prepared(economic)?,
            rating,
            expected_window,
        )
    }

    // Data coherence only. The sole publishing caller above supplies the
    // receipt captured from its actual non-deserializable E4 prepared output.
    fn check_subject(
        &self,
        subject: &Subject,
        economic_commit: mush_core::CommitToken,
        economic: &FilingEconomicReceipt,
        rating: &RatingReceipt,
        expected_window: TermWindow,
    ) -> Result<(), PublicationError> {
        match &subject.parent {
            None => subject
                .revision
                .require_original_access()
                .map_err(invalid)?,
            Some(parent) => {
                if subject.revision.provenance.as_ref()
                    != Some(&crate::DerivativeProvenance {
                        parent: parent.parent,
                        charter: parent.charter,
                        line: parent.line,
                    })
                {
                    return Err(PublicationError::Mismatch);
                }
            }
        }
        let o = &self.operation;
        if economic_commit != o.commit
            || *economic != o.economic
            || o.requester != subject.actor
            || o.revision != subject.identity
            || !economic.owner_matches(subject.actor)
            || self.charter.id.object() != economic.title
            || self.charter.operation != o.commit
            || self.charter.shell_receipt != o.shell_receipt
            || self.charter.line != o.line
            || self.line.charter != o.charter
            || self.line.id != o.line
            || self.line.head != o.filed
            || self.line.operation != o.commit
            || self.line.schema_digest != schema_line_digest(subject.revision.envelope())?
            || self.filed.content.revision() != subject.identity
            || self.filed.content.filed_at()
                != WorldIcSeconds::from_stamp(&subject.issued).map_err(invalid)?
            || self.filed.content.parent() != subject.parent.as_ref().map(|p| p.parent)
            || self.operation.parent != subject.parent
            || self.term.parent != subject.parent
            || self.filed.content.position().sequence.get() != 1
            || self.filed.operation != o.commit
            || self.filed.content.id() != o.filed
            || self.filed.rating != o.rating
            || self.filed.rating != rating.reference().map_err(invalid)?
            || self.filed.definition != rating.subject().definition()
            || self.filed.duplicate != canonical_duplicate_digest(&subject.revision)?
            || rating.subject().revision() != subject.identity
            || rating.subject().schema() != self.line.schema_digest
            || self.filed.content.position().charter != o.charter
            || self.filed.content.position().line != o.line
            || self.status.filed != o.filed
            || self.status.withdrawn_by.is_some()
            || self.status.recalled_by.is_some()
            || self.licence.id != o.licence
            || self.licence.charter != o.charter
            || self.licence.line != o.line
            || self.licence.term != o.term
            || self.licence.operation != o.commit
            || self.term.licence != o.licence
            || self.term.generation != o.term
            || self.term.operation != o.commit
            || self.term.window != expected_window
            || self.term.revision != (RevisionPolicy::Follow { line: o.line })
            || self.identity.revision != subject.identity
            || self.identity.duplicate != self.filed.duplicate
            || self.identity.filed != o.filed
            || self.position.line != o.line
            || self.position.filed != o.filed
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    fn stage(&self, txn: &mut ActionTxn<'_>, f: &CommerceFamilies) -> Result<(), PublicationError> {
        macro_rules! insert {
            ($field:ident, $handle:ident) => {{
                self.$field.validate()?;
                txn.insert(&f.$handle, &self.$field.key(), self.$field.clone())
                    .map_err(invalid)?;
            }};
        }
        insert!(charter, charters);
        insert!(line, lines);
        insert!(filed, filed);
        insert!(status, status);
        insert!(licence, licences);
        insert!(term, terms);
        insert!(operation, operations);
        insert!(identity, identities);
        insert!(position, positions);
        Ok(())
    }
    fn check_staged(
        &self,
        txn: &mut ActionTxn<'_>,
        f: &CommerceFamilies,
    ) -> Result<(), PublicationError> {
        fn exact<R: Record + PartialEq>(
            txn: &mut ActionTxn<'_>,
            handle: &StateFamilyHandle<String, R>,
            row: &R,
        ) -> Result<(), PublicationError> {
            if txn.get(handle, &row.key()).map_err(invalid)? != Some(row) {
                return Err(PublicationError::Mismatch);
            }
            Ok(())
        }
        exact(txn, &f.charters, &self.charter)?;
        exact(txn, &f.lines, &self.line)?;
        exact(txn, &f.filed, &self.filed)?;
        exact(txn, &f.status, &self.status)?;
        exact(txn, &f.licences, &self.licence)?;
        exact(txn, &f.terms, &self.term)?;
        exact(txn, &f.operations, &self.operation)?;
        exact(txn, &f.identities, &self.identity)?;
        exact(txn, &f.positions, &self.position)
    }
}

impl ActionProvider for InitialFilingBuilder {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.request(inv)
            .map(|_| ())
            .map_err(|_| self.refusal.clone())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare(inv, txn, context)
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
                .module_state::<RollStore>()
                .is_none_or(|store| store.commerce_ready().is_err())
        {
            return Ok(None);
        }
        let bad = || self.refusal.clone();
        let definitions = engine.registry.definitions().ok_or_else(bad)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| bad())?;
        guards
            .add_definition(self.action.clone(), definitions)
            .map_err(|_| bad())?;
        guards
            .add_definition(self.manifest.clone(), definitions)
            .map_err(|_| bad())?;
        let mut parameters = Vec::new();
        for (name, bytes) in [
            ("shell_request", 32),
            ("revision_digest", 64),
            ("rating_job", 32),
            ("rating_id", 64),
            ("rating_digest", 64),
            ("charter_name", MAX_CHARTER_NAME_BYTES),
        ] {
            parameters.push(mush_core::ProjectedParameter {
                id: mush_core::mechanics::CanonicalId::new(name).map_err(|_| bad())?,
                required: true,
                kind: mush_core::ProjectedParameterKind::Text {
                    maximum_bytes: bytes,
                },
            });
        }
        let mut integers = vec![("mind", 1), ("revision", 1), ("payer", 0)];
        if self.house {
            integers.extend([("organization", 0), ("scope", 0)]);
        }
        for (name, minimum) in integers {
            parameters.push(mush_core::ProjectedParameter {
                id: mush_core::mechanics::CanonicalId::new(name).map_err(|_| bad())?,
                required: true,
                kind: mush_core::ProjectedParameterKind::Integer {
                    minimum,
                    maximum: i64::MAX,
                },
            });
        }
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(parameters).map_err(|_| bad())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                if self.dependent.is_some() { "roll-file-dependent-mind" } else if self.house { "roll-file-house-mind" } else { "roll-file-original-mind" },
                if self.dependent.is_some() { "File a dependent mind privately" } else if self.house { "File an original house mind" } else { "File an original mind" },
                if self.dependent.is_some() { "File an exact rated revision under current immediate-parent rights." }
                else if self.house { "File an owned rated revision to an authorized organization scope." } else {
                    "File an owned rated revision privately and issue its charter and owner licence."
                },
            )
            .map_err(|_| bad())?,
            guards: guards.finish().map_err(|_| bad())?,
        }))
    }
}

#[cfg(test)]
mod subject_tests;
