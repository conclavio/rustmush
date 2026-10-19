//! Closed saved-revision update producer, with actual Registry and fee owners.
use super::{
    publication::UpdateRoots,
    records::{UpdateOperation, UpdatePosition},
};
use crate::commerce::{
    FiledRevisionId, LineId, RevisionIdentity, TermPhase, WorldIcSeconds,
    families::CommerceFamilies,
    filed::{FiledRevisionContent, FilingPosition, FilingSequence},
    participant::{RootParticipant, RootRequest},
    rating::{RatingCompatibility, RatingFamilies, RatingId, RatingRef},
    records::*,
};
use crate::{RevisionKey, RevisionRef, RollStore};
use mush_core::action::{
    ActionError, ActionInvocation, ActionProvider, AuthoritativeActionOutput, PerceivedRef,
    PrincipalId,
};
use mush_core::action_state::ActionTxn;
use mush_core::mechanics::{DefinitionDigest, DefinitionRef, RulesValue};
use mush_core::participant::ParticipantHandle;
use mush_core::{CommitToken, PrepareCtx};
use mush_economy::registry_filing::{
    EconomyRequest, InitialFilingParticipant, RegistryCompatibleUpdate,
};
use mush_economy::{AccountId, EconomyStore};
use serde::Deserialize;
use serde_json::json;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    line: u64,
    mind: u64,
    revision: u64,
    revision_digest: DefinitionDigest,
    rating_job: CommitToken,
    rating_id: RatingId,
    rating_digest: DefinitionDigest,
    payer: AccountId,
}
struct Request {
    line: LineId,
    revision: RevisionRef,
    rating: RatingRef,
    payer: AccountId,
}
pub(crate) struct UpdateProvider {
    pub(crate) action: DefinitionRef,
    pub(crate) outcome: DefinitionRef,
    pub(crate) manifest: DefinitionRef,
    pub(crate) families: CommerceFamilies,
    pub(crate) ratings: RatingFamilies,
    pub(crate) compatibility: RatingCompatibility,
    pub(crate) economy: ParticipantHandle<InitialFilingParticipant>,
    pub(crate) roots: ParticipantHandle<RootParticipant>,
    pub(crate) refusal: ActionError,
}
fn bad<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}
impl UpdateProvider {
    fn request(&self, inv: &ActionInvocation) -> Result<Request, PublicationError> {
        super::check_action(&self.action)?;
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner != self.action.key().owner
        {
            return Err(PublicationError::Invalid);
        }
        let w: Wire = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
        if w.mind == 0
            || w.revision == 0
            || w.mind > i64::MAX as u64
            || w.revision > i64::MAX as u64
            || w.line > i64::MAX as u64
        {
            return Err(PublicationError::Invalid);
        }
        Ok(Request{line:LineId::new(w.line).map_err(bad)?,revision:RevisionRef{key:RevisionKey{character:inv.actor(),mind:w.mind,number:w.revision},digest:w.revision_digest},
            rating:serde_json::from_value(json!({"schema":1,"job":w.rating_job,"id":w.rating_id,"receipt_digest":w.rating_digest})).map_err(bad)?,payer:w.payer})
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, PublicationError> {
        let request = self.request(inv)?;
        let line = txn
            .get(&self.families.lines, &request.line.key())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        line.validate()?;
        let birth = txn
            .get(&self.families.operations, &line.operation.to_string())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        birth.validate()?;
        let charter = txn
            .get(&self.families.charters, &line.charter.key())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let licence = txn
            .get(&self.families.licences, &birth.licence.key())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let term = txn
            .get(
                &self.families.terms,
                &OwnerTerm::encoded_key(birth.licence, licence.term),
            )
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        charter.validate()?;
        licence.validate()?;
        term.validate()?;
        if birth.format != 1
            || birth.parent.is_some()
            || birth.economic.house.is_some()
            || birth.requester != inv.actor()
            || birth.line != line.id
            || birth.charter != line.charter
            || charter.operation != birth.commit
            || charter.line != line.id
            || licence.operation != birth.commit
            || licence.charter != line.charter
            || licence.line != line.id
            || licence.term != birth.term
            || term.operation != birth.commit
            || term.licence != licence.id
            || term.generation != licence.term
            || term.format != 1
            || term.parent.is_some()
            || term.revision != (RevisionPolicy::Follow { line: line.id })
        {
            return Err(PublicationError::Mismatch);
        }
        let previous = txn
            .get(&self.families.filed, &line.head.key())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        previous.validate()?;
        if previous.content.position().sequence.get().checked_add(1) != Some(line.next_sequence)
            || previous.content.position().charter != line.charter
            || previous.content.position().line != line.id
        {
            return Err(PublicationError::Mismatch);
        }
        let (revision, observed, title_revision) = {
            let world = ctx.world().map_err(bad)?;
            let roll = world
                .module_state::<RollStore>()
                .ok_or(PublicationError::Unavailable)?;
            roll.commerce_ready().map_err(bad)?;
            roll.owned_mind(inv.actor(), request.revision.key.mind)
                .map_err(bad)?;
            let revision = roll
                .revision(inv.actor(), request.revision.key)
                .map_err(bad)?;
            revision.require_original_access().map_err(bad)?;
            if revision.reference() != request.revision {
                return Err(PublicationError::Mismatch);
            }
            let observed = world.world_ic_now();
            if !matches!(
                term.window
                    .phase_at(WorldIcSeconds::from_stamp(&observed).map_err(bad)?),
                TermPhase::Active | TermPhase::Grace
            ) {
                return Err(PublicationError::Unavailable);
            }
            let economy = world
                .module_state::<EconomyStore>()
                .ok_or(PublicationError::Unavailable)?;
            let title = economy
                .snapshot()
                .asset(line.charter.object())
                .ok_or(PublicationError::Missing)?;
            // The actual E4 participant below repeats all current title, actor,
            // catalog, account and authorization predicates before custody.
            (revision.clone(), observed, title.revision)
        };
        let identity = RevisionIdentity::capture(&revision).map_err(bad)?;
        let schema = schema_line_digest(revision.envelope())?;
        let duplicate = canonical_duplicate_digest(&revision)?;
        if schema != line.schema_digest
            || txn
                .get(&self.families.identities, &duplicate.to_string())
                .map_err(bad)?
                .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let rating = self
            .ratings
            .complete_active_receipt_in_txn(txn, &request.rating, &self.compatibility)
            .map_err(bad)?;
        if rating.subject().revision() != identity || rating.subject().schema() != schema {
            return Err(PublicationError::Mismatch);
        }
        let mut deployment = txn
            .get(&self.families.deployment, &"registry".to_owned())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Unavailable)?;
        deployment.validate()?;
        let filed_id = FiledRevisionId::new(deployment.next_filed).map_err(bad)?;
        deployment.next_filed = deployment
            .next_filed
            .checked_add(1)
            .ok_or(PublicationError::Capacity)?;
        let sequence = line.next_sequence;
        let mut after = line.clone();
        after.format = 2;
        after.head = filed_id;
        after.next_sequence = sequence.checked_add(1).ok_or(PublicationError::Capacity)?;
        after.validate()?;
        let economic = ctx
            .prepare_owner(
                &self.economy,
                EconomyRequest::CompatibleUpdate(RegistryCompatibleUpdate {
                    charter: line.charter.object(),
                    title_revision,
                    payer: request.payer,
                }),
            )
            .map_err(bad)?
            .compatible_update()
            .map_err(bad)?;
        let commit = ctx.commit_token();
        if economic.action() != &self.action
            || economic.commit_token() != commit
            || economic.actor() != inv.actor()
        {
            return Err(PublicationError::Mismatch);
        }
        let file = FiledRecord {
            format: 3,
            content: FiledRevisionContent::capture(
                filed_id,
                FilingPosition {
                    charter: line.charter,
                    line: line.id,
                    sequence: FilingSequence::new(sequence).map_err(bad)?,
                },
                &revision,
                WorldIcSeconds::from_stamp(&observed).map_err(bad)?,
                None,
            )
            .map_err(bad)?,
            definition: rating.subject().definition(),
            duplicate,
            rating: request.rating,
            operation: commit,
        };
        let operation = UpdateOperation {
            format: 1,
            commit,
            action: self.action.clone(),
            actor: inv.actor(),
            initial: birth.commit,
            charter: line.charter,
            line: line.id,
            licence: birth.licence,
            term: term.generation,
            previous: line.head,
            filed: filed_id,
            sequence,
            revision: identity,
            rating: request.rating,
            schema,
            duplicate,
            at_ic: observed,
            economic: economic.subject().clone(),
        };
        operation.check_birth(&birth)?;
        operation.check_filed(&file)?;
        let position = UpdatePosition {
            format: 1,
            line: line.id,
            sequence,
            filed: filed_id,
            operation: commit,
        };
        position.check(&operation)?;
        let status = RevisionStatus {
            format: 1,
            filed: filed_id,
            withdrawn_by: None,
            recalled_by: None,
        };
        let duplicate_row = FiledIdentityIndex {
            format: 1,
            duplicate,
            revision: identity,
            filed: filed_id,
        };
        txn.insert(&self.families.filed, &file.key(), file.clone())
            .map_err(bad)?;
        txn.insert(&self.families.status, &status.key(), status.clone())
            .map_err(bad)?;
        txn.insert(
            &self.families.identities,
            &duplicate_row.key(),
            duplicate_row.clone(),
        )
        .map_err(bad)?;
        txn.insert(
            &self.families.update_positions,
            &position.key(),
            position.clone(),
        )
        .map_err(bad)?;
        txn.insert(
            &self.families.line_updates,
            &operation.key(),
            operation.clone(),
        )
        .map_err(bad)?;
        txn.replace(&self.families.lines, &after.key(), after.clone())
            .map_err(bad)?;
        txn.replace(
            &self.families.deployment,
            &deployment.key(),
            deployment.clone(),
        )
        .map_err(bad)?;
        if txn.get(&self.families.filed, &file.key()).map_err(bad)? != Some(&file)
            || txn.get(&self.families.status, &status.key()).map_err(bad)? != Some(&status)
            || txn
                .get(&self.families.identities, &duplicate_row.key())
                .map_err(bad)?
                != Some(&duplicate_row)
            || txn
                .get(&self.families.update_positions, &position.key())
                .map_err(bad)?
                != Some(&position)
            || txn
                .get(&self.families.line_updates, &operation.key())
                .map_err(bad)?
                != Some(&operation)
            || txn.get(&self.families.lines, &after.key()).map_err(bad)? != Some(&after)
            || txn
                .get(&self.families.deployment, &deployment.key())
                .map_err(bad)?
                != Some(&deployment)
        {
            return Err(PublicationError::Mismatch);
        }
        let roots = UpdateRoots::new(&operation, &previous, term.window)?;
        ctx.prepare_owner(&self.roots, RootRequest::CompatibleUpdate(roots))
            .map_err(bad)?;
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(
                self.outcome.clone(),
                json!({"line":line.id,"filed":filed_id,"sequence":sequence,"operation":commit}),
            )
            .map_err(bad)?,
        )
        .with_definitions(vec![self.outcome.clone(), self.manifest.clone()])
        .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for UpdateProvider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.request(inv)
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
                .module_state::<RollStore>()
                .is_none_or(|s| s.commerce_ready().is_err())
        {
            return Ok(None);
        }
        let bad = || self.refusal.clone();
        let definitions = engine.registry.definitions().ok_or_else(bad)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| bad())?;
        for definition in [&self.action, &self.manifest] {
            guards
                .add_definition(definition.clone(), definitions)
                .map_err(|_| bad())?;
        }
        let mut parameters = Vec::new();
        for (name, bytes) in [
            ("revision_digest", 64),
            ("rating_job", 32),
            ("rating_id", 64),
            ("rating_digest", 64),
        ] {
            parameters.push(mush_core::ProjectedParameter {
                id: mush_core::mechanics::CanonicalId::new(name).map_err(|_| bad())?,
                required: true,
                kind: mush_core::ProjectedParameterKind::Text {
                    maximum_bytes: bytes,
                },
            });
        }
        for (name, minimum) in [("line", 1), ("mind", 1), ("revision", 1), ("payer", 0)] {
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
                "roll-update-original-mind",
                "File a compatible mind update",
                "Publish an exactly certified saved revision on your original line.",
            )
            .map_err(|_| bad())?,
            guards: guards.finish().map_err(|_| bad())?,
        }))
    }
}
