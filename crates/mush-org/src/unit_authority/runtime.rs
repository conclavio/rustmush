//! Typed issue/revoke providers and their final live-authority revision fence.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionValidationContext, OwnerId,
};
use mush_core::read_service::OwnerReadHandle;
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalHash, CanonicalId, Ctx, DefinitionRef, Engine,
    IdempotencyKeyText, LegalActionQuery, ModError, ObjId, PerceivedRef, PrepareCtx,
    ProjectedParameter, ProjectedParameterKind, ProjectedParameterSchema, ProjectedTargetSchema,
    ProviderOfferDraft, Registrar, RevisionKey, RevisionKind, RevisionSet, RevisionStamp,
    RevisionValidator, RevisionValidatorContext, RulesValue, StateFamilyHandle, SubjectKey, World,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::families::{self, UnitGrantRegistry};
use super::reader::{
    self, UnitAuthorityReader, UnitAuthorityRequest, UnitAuthorityResponse, UnitGrantMutation,
};
use super::record::UnitAuthorityFacts;

pub const ISSUE_ACTION: &str = "unitgrant_issue";
pub const REVOKE_ACTION: &str = "unitgrant_revoke";
const OUTCOME: &str = "unitgrant_result";
const REVISION: &str = "unitgrant_authority";
const MAX_PENDING: usize = 256;
const INTENT_LIFETIME: i64 = 300;
const MAX_INTENT_BYTES: usize = 12 * 1024;

/// Frozen composition seam. Implementations borrow actual owner state; they
/// must not replace durable mission/asset records with a permissive cache.
pub trait UnitAuthorityWorldFacts: Send + Sync + std::fmt::Debug {
    fn facts<'a>(&'a self, world: &'a World) -> Box<dyn UnitAuthorityFacts + 'a>;
}

/// Transient command input only. It contains no resolved grant, witness or
/// authorization answer, and is never the source of published grant truth.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingIntent {
    pub(crate) created_at: i64,
    pub(crate) registry_revision: u64,
    pub(crate) mutation: UnitGrantMutation,
}
impl PendingIntent {
    pub(crate) fn is_current(&self, now: i64) -> bool {
        now >= self.created_at
            && now
                .checked_sub(self.created_at)
                .is_some_and(|age| age <= INTENT_LIFETIME)
    }
}
#[derive(Clone, Default)]
struct Pending(BTreeMap<ObjId, PendingIntent>);
pub(crate) fn pending(world: &World, actor: ObjId) -> Option<&PendingIntent> {
    world.module_state::<Pending>()?.0.get(&actor)
}

#[derive(Clone, Debug)]
pub struct UnitAuthorityHandles {
    pub registry: StateFamilyHandle<(), UnitGrantRegistry>,
    pub reader: OwnerReadHandle<UnitAuthorityReader>,
    pub issue: DefinitionRef,
    pub revoke: DefinitionRef,
}

pub(crate) fn error(message: &'static str) -> ActionError {
    ActionError::new(ActionErrorCode::ProviderRejected, false, message)
        .expect("static unit-authority error is bounded")
}
fn intent_hash(actor: ObjId, intent: &PendingIntent) -> Result<CanonicalHash, ActionError> {
    CanonicalHash::for_canonical_value(
        "org.unitgrant_request.v1",
        1,
        &json!({"actor":actor,"intent":intent}),
        MAX_INTENT_BYTES * 2,
    )
    .map_err(|_| error("unit-grant request refused"))
}
fn mod_error(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
fn descriptor(kind: &str, id: &str, value: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(mod_error)?,
        DefinitionId::new(id).map_err(mod_error)?,
        1,
        value,
    )
    .map_err(mod_error)
}
fn exact(
    descriptors: Vec<DefinitionDescriptor>,
) -> impl Fn(&DefinitionValidationContext<'_>) -> Result<(), String> + Send + Sync + 'static {
    move |ctx| {
        let entry = ctx.definition();
        let Some(d) = descriptors.iter().find(|d| {
            d.id() == &entry.reference().key().id && d.version() == entry.reference().version()
        }) else {
            return Err("unknown unit-authority definition".into());
        };
        let mut references = d.references().to_vec();
        references.sort();
        if d.payload() != entry.payload()
            || references != entry.references()
            || d.is_current() != entry.is_current()
        {
            return Err("unit-authority definition differs".into());
        }
        Ok(())
    }
}

/// Called only by the owning OrgModule. Reader allowlists and dependency health
/// are frozen with the actual composed owner roster.
pub fn register(
    reg: &mut Registrar<'_>,
    facts: Arc<dyn UnitAuthorityWorldFacts>,
    mut requesters: BTreeSet<OwnerId>,
    dependencies: BTreeSet<OwnerId>,
) -> Result<UnitAuthorityHandles, ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("unit grants require Org ownership"));
    }
    let registry = families::register(reg)?;
    let read_descriptor = reader::definition_descriptor()?;
    reg.definition_validator(
        DefinitionKind::new("owner_read").map_err(mod_error)?,
        exact(vec![read_descriptor.clone()]),
    )?;
    let read_definition = reg.definition(read_descriptor)?;
    requesters.insert(OwnerId::new(crate::NS).map_err(mod_error)?);
    reg.owner_read_service(
        read_definition.clone(),
        requesters,
        dependencies,
        UnitAuthorityReader {
            registry: registry.clone(),
            facts: facts.clone(),
        },
    )?;
    let reader = reg.request_owner_read::<UnitAuthorityReader>(read_definition)?;
    let outcome_descriptor = descriptor(
        "outcome",
        OUTCOME,
        json!({"format":1,"fields":["actor","grant","revision","status"]}),
    )?;
    reg.definition_validator(
        DefinitionKind::new("outcome").map_err(mod_error)?,
        exact(vec![outcome_descriptor.clone()]),
    )?;
    let outcome = reg.definition(outcome_descriptor)?;
    let actions: Vec<_> = [ISSUE_ACTION, REVOKE_ACTION]
        .into_iter()
        .map(|id| {
            descriptor(
                "action",
                id,
                json!({"format":1,"subject":"self_actor","targets":0,"family":families::FAMILY}),
            )
            .map(|d| d.as_current().with_references(vec![outcome.clone()]))
        })
        .collect::<Result<_, _>>()?;
    reg.definition_validator(
        DefinitionKind::new("action").map_err(mod_error)?,
        exact(actions.clone()),
    )?;
    let mut refs = Vec::new();
    for d in actions {
        let action = reg.definition(d)?;
        reg.action_provider(
            action.clone(),
            Provider {
                action: action.clone(),
                outcome: outcome.clone(),
                registry: registry.clone(),
                reader: reader.clone(),
                facts: facts.clone(),
            },
        )?;
        refs.push(action);
    }
    let handles = UnitAuthorityHandles {
        registry: registry.clone(),
        reader,
        issue: refs
            .first()
            .cloned()
            .ok_or_else(|| ModError::msg("unit issue missing"))?,
        revoke: refs
            .get(1)
            .cloned()
            .ok_or_else(|| ModError::msg("unit revoke missing"))?,
    };
    reg.state(Pending::default());
    reg.state(handles.clone());
    reg.revision_validator(
        super::operation::kind()?,
        super::operation::Validator {
            registry: registry.clone(),
            facts: facts.clone(),
        },
    )?;
    reg.revision_validator(
        RevisionKind::new(CanonicalId::new(REVISION).map_err(mod_error)?),
        Validator { registry, facts },
    )?;
    super::commands::register(reg)?;
    Ok(handles)
}

fn revision_key(actor: ObjId) -> Result<RevisionKey, ActionError> {
    Ok(RevisionKey {
        owner: OwnerId::new(crate::NS).map_err(|_| error("unit authority unavailable"))?,
        kind: RevisionKind::new(
            CanonicalId::new(REVISION).map_err(|_| error("unit authority unavailable"))?,
        ),
        subject: SubjectKey::Object { id: actor },
        facet: None,
    })
}
fn generation(
    world: &World,
    facts: &dyn UnitAuthorityWorldFacts,
    registry: &UnitGrantRegistry,
    actor: ObjId,
) -> Result<u64, ActionError> {
    registry
        .validate()
        .map_err(|_| error("unit authority unavailable"))?;
    let intent = pending(world, actor);
    let proposal = intent.and_then(|i| reader::evaluate(world, facts, registry, actor, i).ok());
    let hash = CanonicalHash::for_canonical_value(
        "org.unitgrant_authority.v1",
        1,
        &json!({"intent":intent,"registry_revision":registry.revision,"proposal":proposal}),
        MAX_INTENT_BYTES * 2,
    )
    .map_err(|_| error("unit authority unavailable"))?;
    u64::from_str_radix(
        hash.to_hex()
            .get(..16)
            .ok_or_else(|| error("unit authority unavailable"))?,
        16,
    )
    .map_err(|_| error("unit authority unavailable"))
}
struct Validator {
    registry: StateFamilyHandle<(), UnitGrantRegistry>,
    facts: Arc<dyn UnitAuthorityWorldFacts>,
}
impl RevisionValidator for Validator {
    fn current_generation(
        &self,
        ctx: &RevisionValidatorContext<'_>,
        key: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        let SubjectKey::Object { id } = key.subject else {
            return Ok(None);
        };
        if key != &revision_key(id)? {
            return Ok(None);
        }
        let registry = ctx
            .action_state(&self.registry, &())
            .map_err(|_| error("unit authority unavailable"))?
            .cloned()
            .unwrap_or_default();
        generation(ctx.world(), self.facts.as_ref(), &registry, id).map(Some)
    }
}

struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    registry: StateFamilyHandle<(), UnitGrantRegistry>,
    reader: OwnerReadHandle<UnitAuthorityReader>,
    facts: Arc<dyn UnitAuthorityWorldFacts>,
}
impl Provider {
    fn matches(&self, intent: &PendingIntent) -> bool {
        matches!(
            (&intent.mutation, self.action.key().id.as_str()),
            (UnitGrantMutation::Issue(_), ISSUE_ACTION)
                | (UnitGrantMutation::Revoke { .. }, REVOKE_ACTION)
        )
    }
    fn intent(&self, i: &ActionInvocation) -> Result<PendingIntent, ActionError> {
        if i.subject() != &PerceivedRef::SelfActor
            || !i.targets().is_empty()
            || i.principal() != &(mush_core::PrincipalId::Object { id: i.actor() })
            || i.parameters().as_object().is_none_or(|p| p.len() != 2)
        {
            return Err(error("unit-grant request refused"));
        }
        let value = i
            .parameters()
            .get("intent")
            .ok_or_else(|| error("unit-grant request refused"))?;
        if serde_json::to_vec(value)
            .map_err(|_| error("unit-grant request refused"))?
            .len()
            > MAX_INTENT_BYTES
        {
            return Err(error("unit-grant request refused"));
        }
        let intent: PendingIntent = serde_json::from_value(value.clone())
            .map_err(|_| error("unit-grant request refused"))?;
        if !self.matches(&intent)
            || i.parameters().get("request").and_then(Value::as_str)
                != Some(intent_hash(i.actor(), &intent)?.to_hex().as_str())
        {
            return Err(error("unit-grant request refused"));
        }
        Ok(intent)
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, i: &ActionInvocation) -> Result<(), ActionError> {
        self.intent(i).map(|_| ())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        q: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || q.projected_subject() != &PerceivedRef::SelfActor
            || q.truth_subject() != &(SubjectKey::Object { id: q.actor() })
        {
            return Ok(None);
        }
        let Some(intent) = pending(&engine.world, q.actor()).filter(|i| self.matches(i)) else {
            return Ok(None);
        };
        let registry = engine
            .action_state()
            .get(&self.registry, &())
            .map_err(|_| error("unit authority unavailable"))?
            .cloned()
            .unwrap_or_default();
        if reader::evaluate(
            &engine.world,
            self.facts.as_ref(),
            &registry,
            q.actor(),
            intent,
        )
        .is_err()
        {
            return Ok(None);
        }
        let revisions = RevisionSet::from_stamps([RevisionStamp {
            key: revision_key(q.actor())?,
            generation: generation(&engine.world, self.facts.as_ref(), &registry, q.actor())?,
        }])
        .map_err(|_| error("unit authority unavailable"))?;
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| error("unit authority unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(revisions)
            .map_err(|_| error("unit authority unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| error("unit authority unavailable"))?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({"intent":intent}),
            parameters: ProjectedParameterSchema::new(vec![ProjectedParameter {
                id: CanonicalId::new("request").map_err(|_| error("unit authority unavailable"))?,
                required: true,
                kind: ProjectedParameterKind::Text { maximum_bytes: 64 },
            }])
            .map_err(|_| error("unit authority unavailable"))?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                self.action.key().id.as_str(),
                "Commit unit grant",
                "Apply the requested unit permission change.",
            )
            .map_err(|_| error("unit authority unavailable"))?,
            guards: guards
                .finish()
                .map_err(|_| error("unit authority unavailable"))?,
        }))
    }
    fn prepare_authoritative_with_context(
        &self,
        i: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let intent = self.intent(i)?;
        let answer = context
            .read_owner_preimage(txn, &self.reader, UnitAuthorityRequest::Prepare(intent))
            .map_err(|_| error("unit-grant request refused"))?;
        let UnitAuthorityResponse::Prepared {
            registry_revision,
            grant,
        } = answer
        else {
            return Err(error("unit-grant request refused"));
        };
        let before = txn
            .get(&self.registry, &())
            .map_err(|_| error("unit authority unavailable"))?
            .cloned();
        let registry = before.clone().unwrap_or_default();
        if registry.revision != registry_revision {
            return Err(error("unit-grant request changed"));
        }
        let next = registry
            .with_grant(grant.clone())
            .map_err(|_| error("unit-grant registry capacity reached"))?;
        if before.is_some() {
            txn.replace(&self.registry, &(), next)
        } else {
            txn.insert(&self.registry, &(), next)
        }
        .map_err(|_| error("unit-grant write refused"))?;
        let outcome = RulesValue::new(self.outcome.clone(), json!({"actor":i.actor(),"grant":grant.id,"revision":grant.revision,"status":grant.status}))
            .map_err(|_| error("unit-grant outcome refused"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![i.principal().clone()]))
    }
    fn resolved_revision_keys(
        &self,
        resolved: &mush_core::ResolvedActionInvocation,
    ) -> Result<Vec<RevisionKey>, ActionError> {
        self.validate_invocation(resolved.invocation())?;
        // project_offer already stamps this actor's live-authority revision in
        // the offer guard set. The engine retains that stamp through the final
        // take_staged_action recheck. There are no privately resolved targets
        // here, so adding the same coordinate again would be a duplicate guard
        // (RevisionSet::from_stamps refuses duplicates), not another fence.
        Ok(Vec::new())
    }
}

/// Thin ingress: only bounded input is staged, then submitted to the ordinary
/// offer/coordinator path. It emits no success before durable acknowledgement.
pub(crate) fn submit(ctx: &mut Ctx<'_>, mutation: UnitGrantMutation) -> Result<(), String> {
    if ctx.authoritative_pending(ctx.actor)
        || ctx.authoritative_commit_pending()
        || ctx.authoritative_queue_full()
    {
        return Err("A unit-grant request is already pending.".into());
    }
    let h = ctx
        .w
        .module_state::<UnitAuthorityHandles>()
        .cloned()
        .ok_or("Unit authority unavailable.")?;
    let registry = ctx
        .action_state(&h.registry, &())
        .map_err(|_| "Unit authority unavailable.")?
        .cloned()
        .unwrap_or_default();
    registry.validate()?;
    let intent = PendingIntent {
        created_at: ctx.w.now(),
        registry_revision: registry.revision,
        mutation,
    };
    if serde_json::to_vec(&intent)
        .map_err(|_| "Invalid unit-grant request.")?
        .len()
        > MAX_INTENT_BYTES
    {
        return Err("Unit-grant request is too large.".into());
    }
    let action = match intent.mutation {
        UnitGrantMutation::Issue(_) => h.issue,
        UnitGrantMutation::Revoke { .. } => h.revoke,
    };
    let now = ctx.w.now();
    let requests = ctx
        .w
        .module_state_mut::<Pending>()
        .ok_or("Unit authority unavailable.")?;
    requests.0.retain(|_, i| i.is_current(now));
    if requests.0.len() >= MAX_PENDING && !requests.0.contains_key(&ctx.actor) {
        return Err("Unit-grant requests are busy.".into());
    }
    requests.0.insert(ctx.actor, intent.clone());
    if ctx
        .read_owner(&h.reader, UnitAuthorityRequest::Prepare(intent.clone()))
        .is_err()
    {
        if let Some(requests) = ctx.w.module_state_mut::<Pending>()
            && requests.0.get(&ctx.actor) == Some(&intent)
        {
            requests.0.remove(&ctx.actor);
        }
        return Err("Unit-grant request refused.".into());
    }
    let key = intent_hash(ctx.actor, &intent).map_err(|_| "Invalid unit-grant request.")?;
    let idempotency = IdempotencyKeyText::parse_canonical(format!("unitgrant/{}", key.to_hex()))
        .map_err(|_| "Invalid unit-grant request.")?;
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({"request":key.to_hex()}),
        idempotency,
    );
    Ok(())
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
