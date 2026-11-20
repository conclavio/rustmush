//! One closed acknowledged fork of an actual original paid Open parent.
//! Editing/Registry/dependent filing are deliberately separate consumers.

use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    PerceivedRef, PrincipalId,
};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    RulesValue,
};
use mush_core::participant::{OwnerPrepareContext, ParticipantHandle};
use mush_core::read_service::{OwnerReadHandle, OwnerReadService};
use mush_core::{ActionTxn, CommitToken, DefinitionRef, ModError, PrepareCtx, Registrar};
use serde::Deserialize;
use serde_json::json;
use std::marker::PhantomData;

use super::LicenceId;
use super::families::CommerceFamilies;
use super::fork_protocol::{ForkMaterial, ForkMaterialRequest};
use super::parent_grant::{CurrentParentGrant, ParentGrantReader, ParentGrantRequest};
use super::participant::{RootParticipant, RootRequest};
use super::records::{PublicationError, Record};
use crate::RollStore;

const ACTION: &str = "fork_paid_mind";
const OUTCOME: &str = "paid_mind_forked";
/// Bounded lifetime derivative drafts (active + retired) per character so a
/// zero-fee fork/retire loop cannot exhaust `RollStore::MAX_RECORDS`.
pub(crate) const MAX_DERIVATIVE_DRAFTS_PER_CHARACTER: usize = 64;

fn check_derivative_draft_quota(
    roll: &RollStore,
    txn: &mut ActionTxn<'_>,
    families: &CommerceFamilies,
    actor: mush_core::ObjId,
) -> Result<(), PublicationError> {
    let active = roll
        .derivative_minds()
        .filter(|mind| mind.character == actor)
        .count();
    let mut retired = 0usize;
    for commit in roll.draft_retirement_commits() {
        if txn
            .get(&families.authored_retirements, &commit.to_hex())
            .map_err(invalid)?
            .is_some_and(|row| row.actor == actor)
        {
            retired += 1;
        }
    }
    if active.saturating_add(retired) >= MAX_DERIVATIVE_DRAFTS_PER_CHARACTER {
        return Err(PublicationError::Capacity);
    }
    Ok(())
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn invalid<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}

pub(crate) fn definitions(manifest: &DefinitionRef) -> Result<[DefinitionDescriptor; 2], ModError> {
    Ok([
        DefinitionDescriptor::new(
            DefinitionKind::new("action").map_err(error)?,
            DefinitionId::new(ACTION).map_err(error)?,
            1,
            json!({"format":1,"protocol":"authored_paid_fork_v1","original_parent_only":true,
                "personal_pin":true,"forms":["open","open_resale"],"editing":false}),
        )
        .map_err(error)?
        .with_references(vec![manifest.clone()]),
        DefinitionDescriptor::new(
            DefinitionKind::new("outcome").map_err(error)?,
            DefinitionId::new(OUTCOME).map_err(error)?,
            1,
            json!({"format":1}),
        )
        .map_err(error)?,
    ])
}

/// Private owner request: there is no wire constructor, arbitrary image list,
/// supplied actor, ancestor rewrite or output-derived public capability.
pub(crate) struct AuthoredFork {
    action: DefinitionRef,
    commit: CommitToken,
    grant: CurrentParentGrant,
    name: String,
    material: ForkMaterial,
}
impl AuthoredFork {
    pub(crate) fn commit(&self) -> CommitToken {
        self.commit
    }
    pub(crate) fn grant(&self) -> &CurrentParentGrant {
        &self.grant
    }
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
    pub(crate) fn material(&self) -> &ForkMaterial {
        &self.material
    }
    pub(crate) fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub(crate) fn check_context(
        &self,
        context: &OwnerPrepareContext<'_>,
    ) -> Result<(), PublicationError> {
        check_action(&self.action, self.commit, &self.grant, context)
    }
}
pub(crate) fn check_action(
    action: &DefinitionRef,
    commit: CommitToken,
    grant: &CurrentParentGrant,
    context: &OwnerPrepareContext<'_>,
) -> Result<(), PublicationError> {
    if !exact_action(action, context.action())
        || commit != context.commit_token()
        || context.invocation().principal() != &(PrincipalId::Object { id: grant.actor() })
        || context.invocation().subject() != &PerceivedRef::SelfActor
        || !context.invocation().targets().is_empty()
    {
        return Err(PublicationError::Mismatch);
    }
    grant.check_fresh(context.invocation().actor(), context.world())
}

fn exact_action(captured: &DefinitionRef, current: &DefinitionRef) -> bool {
    captured == current
        && captured.key().owner.as_str() == crate::NS
        && captured.key().kind.as_str() == "action"
        && captured.key().id.as_str() == ACTION
        && captured.version() == 1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    parent_licence: LicenceId,
    name: String,
}
struct ForkBuilder<S: OwnerReadService<Request = ForkMaterialRequest, Output = ForkMaterial>> {
    action: DefinitionRef,
    outcome: DefinitionRef,
    manifest: DefinitionRef,
    families: CommerceFamilies,
    parent: OwnerReadHandle<ParentGrantReader>,
    source: OwnerReadHandle<S>,
    roots: ParticipantHandle<RootParticipant>,
    refusal: ActionError,
}
impl<S: OwnerReadService<Request = ForkMaterialRequest, Output = ForkMaterial>> ForkBuilder<S> {
    fn request(&self, inv: &ActionInvocation) -> Result<Request, PublicationError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner != self.action.key().owner
        {
            return Err(PublicationError::Invalid);
        }
        let request: Request = serde_json::from_value(inv.parameters().clone()).map_err(invalid)?;
        if crate::canonical_name(&request.name).map_err(invalid)? != request.name {
            return Err(PublicationError::Invalid);
        }
        Ok(request)
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, PublicationError> {
        let request = self.request(inv)?;
        {
            let world = ctx.world().map_err(invalid)?;
            let roll = world
                .module_state::<RollStore>()
                .ok_or(PublicationError::Unavailable)?;
            check_derivative_draft_quota(roll, txn, &self.families, inv.actor())?;
        }
        let grant = ctx
            .read_owner_preimage(
                txn,
                &self.parent,
                ParentGrantRequest {
                    licence: request.parent_licence,
                },
            )
            .map_err(invalid)?;
        if grant.actor() != inv.actor() || grant.licence() != request.parent_licence {
            return Err(PublicationError::Mismatch);
        }
        let material = ctx
            .read_owner_preimage(txn, &self.source, ForkMaterialRequest::from_parent(&grant))
            .map_err(invalid)?;
        material
            .check(grant.holding().revision())
            .map_err(invalid)?;
        let commit = ctx.commit_token();
        let selection = ctx
            .prepare_owner(
                &self.roots,
                RootRequest::AuthoredFork(Box::new(AuthoredFork {
                    action: self.action.clone(),
                    commit,
                    grant,
                    name: request.name,
                    material,
                })),
            )
            .map_err(invalid)?
            .authored_fork()?;
        selection.validate()?;
        if selection.actor != inv.actor()
            || selection.created != commit
            || selection.licence != request.parent_licence
        {
            return Err(PublicationError::Mismatch);
        }
        txn.insert(
            &self.families.draft_parents,
            &selection.key(),
            selection.clone(),
        )
        .map_err(invalid)?;
        if txn
            .get(&self.families.draft_parents, &selection.key())
            .map_err(invalid)?
            != Some(&selection)
        {
            return Err(PublicationError::Mismatch);
        }
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({"mind":selection.mind,
            "operation":commit}),
        )
        .map_err(invalid)?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![self.outcome.clone(), self.manifest.clone()])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl<S: OwnerReadService<Request = ForkMaterialRequest, Output = ForkMaterial>> ActionProvider
    for ForkBuilder<S>
{
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
        let defs = engine.registry.definitions().ok_or_else(bad)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| bad())?;
        for definition in [&self.action, &self.manifest] {
            guards
                .add_definition(definition.clone(), defs)
                .map_err(|_| bad())?;
        }
        let parameters = [("parent_licence", 20), ("name", 64)]
            .into_iter()
            .map(|(name, maximum_bytes)| {
                Ok(mush_core::ProjectedParameter {
                    id: mush_core::mechanics::CanonicalId::new(name).map_err(|_| bad())?,
                    required: true,
                    kind: mush_core::ProjectedParameterKind::Text { maximum_bytes },
                })
            })
            .collect::<Result<Vec<_>, ActionError>>()?;
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(parameters).map_err(|_| bad())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "roll-fork-paid-mind",
                "Fork an Open mind",
                "Create an attributed working copy from your current Open parent license.",
            )
            .map_err(|_| bad())?,
            guards: guards.finish().map_err(|_| bad())?,
        }))
    }
}

pub(crate) trait Installer: Send {
    fn register(
        &self,
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
        families: CommerceFamilies,
        parent: OwnerReadHandle<ParentGrantReader>,
        definitions: [DefinitionRef; 2],
    ) -> Result<(), ModError>;
}
struct Source<S>(PhantomData<fn() -> S>);
pub(crate) fn source<S>() -> Box<dyn Installer>
where
    S: OwnerReadService<Request = ForkMaterialRequest, Output = ForkMaterial>,
{
    Box::new(Source::<S>(PhantomData))
}
impl<S: OwnerReadService<Request = ForkMaterialRequest, Output = ForkMaterial>> Installer
    for Source<S>
{
    fn register(
        &self,
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
        families: CommerceFamilies,
        parent: OwnerReadHandle<ParentGrantReader>,
        definitions: [DefinitionRef; 2],
    ) -> Result<(), ModError> {
        // The owner registered these once before granting RootParticipant's
        // exact action. Reuse those references for provider/outcome binding.
        let [action, outcome] = definitions;
        let source = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("tacsim").map_err(error)?,
                super::fork_protocol::material_descriptor(manifest)?,
            )
            .map_err(error)?;
        let source = reg.request_owner_read::<S>(source)?;
        let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
        reg.action_provider(
            action.clone(),
            ForkBuilder {
                action,
                outcome,
                manifest: manifest.clone(),
                families,
                parent,
                source,
                roots,
                refusal: ActionError::new(
                    ActionErrorCode::ProviderRejected,
                    false,
                    "The current Open parent cannot be forked under these terms.",
                )
                .map_err(error)?,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error>>;
    fn reference(
        owner: &str,
        kind: &str,
        id: &str,
        version: u32,
        payload: serde_json::Value,
    ) -> Result<DefinitionRef, ModError> {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(owner).map_err(error)?,
                DefinitionDescriptor::new(
                    DefinitionKind::new(kind).map_err(error)?,
                    DefinitionId::new(id).map_err(error)?,
                    version,
                    payload,
                )
                .map_err(error)?,
            )
            .map_err(error)
    }
    #[test]
    fn participant_cannot_reuse_an_old_or_foreign_action_grant_for_a_fork() -> TestResult {
        let manifest = reference(
            "tacsim",
            "registry_manifest",
            "combat",
            1,
            json!({"fixture":"identity only"}),
        )?;
        let [descriptor, _] = definitions(&manifest)?;
        let actual =
            DefinitionRegistryBuilder::new().register(OwnerId::new(crate::NS)?, descriptor)?;
        check!(exact_action(&actual, &actual));
        // This is the predicate used before owner preparation and again before
        // custody; a shared RootParticipant type does not make its grants fungible.
        for (owner, kind, id, version) in [
            ("roll", "action", "initial_filing", 1),
            ("roll", "action", "rating_admit", 1),
            ("roll", "action", "rating_grant", 1),
            ("roll", "action", "rating_settle", 1),
            ("market", "action", "purchase_original_mind", 2),
            ("market", "action", "settle_original_mind", 2),
            ("market", "action", "fork_paid_mind", 1),
            ("roll", "outcome", "fork_paid_mind", 1),
            ("roll", "action", "fork_paid_mind", 2),
        ] {
            let other = reference(owner, kind, id, version, json!({"fixture":"identity only"}))?;
            check!(!exact_action(&other, &other));
            check!(!exact_action(&actual, &other));
        }
        let altered = reference(
            "roll",
            "action",
            "fork_paid_mind",
            1,
            json!({"format":1,"protocol":"arbitrary"}),
        )?;
        check_ne!(actual.digest(), altered.digest());
        check!(
            !exact_action(&actual, &altered),
            "same locator does not replace the captured exact configured action"
        );
        Ok(())
    }

    #[inline(never)]
    fn seed_parent_revision_kv()
    -> Result<Vec<(smol_str::SmolStr, serde_json::Value)>, Box<dyn std::error::Error>> {
        use crate::{Envelope, WorkingCopy};
        use mush_core::ObjId;

        let mut kv = Vec::new();
        let mut store = RollStore::default();
        let draft_copy = WorkingCopy {
            source: "seed".into(),
            envelope: Envelope {
                schema: json!({"widgets": []}),
                presets: json!({}),
                profiles: json!({"line": {"min_odds": 30}}),
                rules: json!({"line": []}),
                declared_class: "I".into(),
                mind_abi: "test=1".into(),
            },
        };
        let create = store.prepare_create(ObjId(1), "parent", draft_copy)?;
        for op in create.operations() {
            if let mush_core::ModuleWrite::JsonSet { key, value } = &op.op {
                kv.push((key.clone(), value.clone()));
            }
        }
        store.publish_with(create, |_| {})?;
        Ok(kv)
    }

    #[inline(never)]
    fn seed_retired_derivative_drafts(
        engine: &mut mush_core::Engine,
        f: &CommerceFamilies,
        actor: mush_core::ObjId,
        kv: &mut Vec<(smol_str::SmolStr, serde_json::Value)>,
    ) -> TestResult {
        use crate::commerce::authored_retirement::records::RetirementOperation;
        use crate::commerce::roots::RootAnchor;
        use crate::{ReferenceKey, ReferenceKind, ReferenceRecord};
        use mush_core::{DefinitionDigest, ObjId};

        let digest = DefinitionDigest::from_bytes([9; 32]);
        let rev_ref = crate::RevisionRef {
            key: crate::RevisionKey {
                character: ObjId(1),
                mind: 1,
                number: 1,
            },
            digest,
        };
        let parent_id: crate::commerce::RevisionIdentity = serde_json::from_value(json!({
            "format": 1,
            "revision": rev_ref,
            "source": digest
        }))?;
        let ret_action = crate::commerce::authored_retirement::action_reference()?;
        let mut txn = engine.action_transaction(&f.authored_retirements)?;
        for i in 0..MAX_DERIVATIVE_DRAFTS_PER_CHARACTER {
            let commit = CommitToken::from_bytes([u8::try_from(i + 1)?; 16]);
            let op = RetirementOperation {
                format: 1,
                actor,
                mind: u64::try_from(i + 1)?,
                selection: DefinitionDigest::from_bytes([u8::try_from(i + 1)?; 32]),
                operation: commit,
                base: crate::EditBase {
                    edit: 0,
                    revision: None,
                },
                action: ret_action.clone(),
                name: format!("d_{i}"),
                parent: parent_id,
                unnamed: Vec::new(),
                named: Vec::new(),
            };
            let anchor = RootAnchor::DraftRetirement(commit, op.digest()?);
            let ref_rec = ReferenceRecord {
                format: crate::FORMAT,
                key: ReferenceKey {
                    owner: crate::NS.into(),
                    kind: ReferenceKind::Commercial,
                    record: anchor.record(),
                },
                revisions: vec![rev_ref],
            };
            kv.push((ref_rec.key.key().into(), serde_json::to_value(&ref_rec)?));
            txn.insert(&f.authored_retirements, &op.key(), op)?;
        }
        let (_, install) = txn.prepare()?.into_parts();
        engine.install_prepared_action(install)?;
        Ok(())
    }

    #[test]
    fn high19_authored_fork_caps_total_active_and_retired_derivative_drafts() -> TestResult {
        use mush_core::{GameModule, ModuleBootData, ObjId, Registry, World};
        use mush_testkit::fixtures::boxed_engine;
        use std::sync::{Arc, Mutex};

        struct FamiliesMod(Arc<Mutex<Option<CommerceFamilies>>>);
        impl GameModule for FamiliesMod {
            fn name(&self) -> &'static str {
                crate::NS
            }
            fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
                let f = CommerceFamilies::register(reg)?;
                *self.0.lock().map_err(|_| ModError::msg("lock"))? = Some(f);
                Ok(())
            }
        }

        let slot = Arc::new(Mutex::new(None));
        let mut engine = boxed_engine(
            World::new_minimal(),
            Registry::new(),
            vec![Box::new(FamiliesMod(slot.clone()))],
        )?;
        let f = slot.lock().map_err(|_| "lock")?.clone().ok_or("families")?;

        let actor = ObjId(42);
        let other = ObjId(43);
        // Seed parent revision so RollStore::load validates the commercial anchors cleanly.
        let mut kv = seed_parent_revision_kv()?;
        // Seed MAX_DERIVATIVE_DRAFTS_PER_CHARACTER retired derivative drafts for `actor`.
        seed_retired_derivative_drafts(&mut engine, &f, actor, &mut kv)?;
        let roll = Box::new(RollStore::load_commerce(&ModuleBootData {
            kv,
            ..Default::default()
        })?);
        check_eq!(
            roll.draft_retirement_commits().len(),
            MAX_DERIVATIVE_DRAFTS_PER_CHARACTER
        );
        let mut txn = engine.action_transaction(&f.authored_retirements)?;
        check_eq!(
            check_derivative_draft_quota(&roll, &mut txn, &f, actor),
            Err(PublicationError::Capacity),
            "actor at MAX_DERIVATIVE_DRAFTS_PER_CHARACTER retired drafts cannot fork more"
        );
        check!(
            check_derivative_draft_quota(&roll, &mut txn, &f, other).is_ok(),
            "unrelated character retains full derivative quota"
        );
        Ok(())
    }
}
