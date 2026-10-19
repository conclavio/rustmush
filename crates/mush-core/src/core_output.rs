//! Closed Industry hardware output reservation. The actual owner policy names
//! a retained accepted order and trusted facility container; Core derives every
//! object/index/allocator image. This module grants no certificate or E4 title.
use crate::World;
use crate::action::{ActionInvocation, CommitToken, PerceivedRef, PrincipalId};
use crate::action_state::{ActionTxn, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use crate::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind,
    DefinitionRef, OwnerId, canonical_json_bytes_with_limit,
};
use crate::module::{ModError, ModuleCtx, Registrar};
use crate::obj::{GOD, Obj, ObjId, ObjKind};
use crate::participant::ParticipantError;
use crate::read_service::OwnerReadHandle;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;

mod policy;
use policy::PolicyReader;
pub use policy::{
    CoreOutputOrder, CoreOutputPolicy, OutputPhase, OutputPolicyFacts, OutputPolicyRequest,
    OutputRecheckContext,
};
pub(crate) mod image;
mod records;
pub use records::{CoreOutputHistory, ReservedCoreOutput};
use records::{OutputDeployment, OutputRecord, OutputState};

pub const INITIALIZE_ACTION: &str = "initialize_core_production";
pub const RESERVE_ACTION: &str = "reserve_core_output";
pub const COMPLETE_ACTION: &str = "complete_core_integration";
pub const PURPOSE: &str = "industry.core_integration";
const NAME: &str = "Reserved Mind core";
const FORMAT: u16 = 1;
const RECORD_BYTES: usize = 8192;

/// Trusted construction fixes the exact three actions and recipe, rather than
/// accepting a caller purpose, object location, action ref or mutation image.
#[derive(Clone, Debug)]
pub struct CoreOutputBinding {
    actions: [DefinitionRef; 3],
    recipe: DefinitionRef,
}
impl CoreOutputBinding {
    pub fn new(actions: [DefinitionRef; 3], recipe: DefinitionRef) -> Result<Self, ModError> {
        if !actions
            .iter()
            .zip([INITIALIZE_ACTION, RESERVE_ACTION, COMPLETE_ACTION])
            .all(|(a, id)| fixed_action(a, id))
            || recipe.key().kind.as_str() != "recipe"
            || recipe.key().id.as_str() != "core_integrate"
            || recipe.version() != 1
        {
            return Err(ModError::msg("invalid fixed core output binding"));
        }
        Ok(Self { actions, recipe })
    }
    pub fn actions(&self) -> &[DefinitionRef; 3] {
        &self.actions
    }
    pub fn recipe(&self) -> &DefinitionRef {
        &self.recipe
    }
    pub fn policy_descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        let mut references: Vec<_> = self
            .actions
            .iter()
            .cloned()
            .chain(std::iter::once(self.recipe.clone()))
            .collect();
        references.sort();
        DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(mod_error)?, DefinitionId::new("core_output_policy").map_err(mod_error)?, 1,
            json!({"format":1,"protocol":"industry_core_output_v1","purpose":PURPOSE,"actions":self.actions,"recipe":self.recipe,"record_bytes":RECORD_BYTES}))
            .map(|d| d.with_references(references))
            .map_err(mod_error)
    }
    fn action(&self, phase: OutputPhase) -> &DefinitionRef {
        match phase {
            OutputPhase::Initialize => &self.actions[0],
            OutputPhase::Reserve => &self.actions[1],
            OutputPhase::Complete => &self.actions[2],
        }
    }
}

pub struct CoreOutputReservations<P: CoreOutputPolicy> {
    records: StateFamilyHandle<u64, OutputRecord>,
    deployment: StateFamilyHandle<(), OutputDeployment>,
    reader: OwnerReadHandle<PolicyReader<P>>,
    policy: Arc<P>,
    binding: CoreOutputBinding,
}
impl<P: CoreOutputPolicy> Clone for CoreOutputReservations<P> {
    fn clone(&self) -> Self {
        Self {
            records: self.records.clone(),
            deployment: self.deployment.clone(),
            reader: self.reader.clone(),
            policy: Arc::clone(&self.policy),
            binding: self.binding.clone(),
        }
    }
}
impl<P: CoreOutputPolicy> CoreOutputReservations<P> {
    /// Register under the actual Industry state owner. Definitions and owner
    /// dependencies are frozen by the ordinary registrar. No provider is
    /// activated here; Industry supplies its real typed/economic producer.
    pub fn register(
        reg: &mut Registrar<'_>,
        binding: CoreOutputBinding,
        policy: P,
        mut dependencies: BTreeSet<OwnerId>,
    ) -> Result<Self, ModError> {
        if reg.module() != "industry" {
            return Err(ModError::msg("core outputs require the Industry owner"));
        }
        dependencies.extend(binding.actions.iter().map(|r| r.key().owner.clone()));
        dependencies.insert(binding.recipe.key().owner.clone());
        let definition = reg.definition(binding.policy_descriptor()?)?;
        let policy = Arc::new(policy);
        reg.owner_read_service(
            definition.clone(),
            BTreeSet::from([OwnerId::new("industry").map_err(mod_error)?]),
            dependencies,
            PolicyReader {
                policy: Arc::clone(&policy),
            },
        )?;
        let reader = reg.request_owner_read::<PolicyReader<P>>(definition)?;
        let records = reg.state_family(
            descriptor("core_output_reservations")?,
            |order: &u64| Ok(order.to_string()),
            |v: &Value| decode::<OutputRecord>(v),
            |r: &OutputRecord| r.validate().map_err(|e| e.to_string()),
            |r: &OutputRecord| encode(r),
        )?;
        let deployment = reg.state_family(
            descriptor("core_output_deployment")?,
            |_: &()| Ok("registry".to_owned()),
            |v: &Value| decode::<OutputDeployment>(v),
            |r: &OutputDeployment| r.validate().map_err(|e| e.to_string()),
            |r: &OutputDeployment| encode(r),
        )?;
        Ok(Self {
            records,
            deployment,
            reader,
            policy,
            binding,
        })
    }
    pub(crate) fn request(&self, phase: OutputPhase, order: Option<u64>) -> OutputPolicyRequest {
        OutputPolicyRequest { phase, order }
    }
    pub(crate) fn reader(&self) -> &OwnerReadHandle<PolicyReader<P>> {
        &self.reader
    }
    fn authority(
        &self,
        world: &World,
        inv: &ActionInvocation,
        action: &DefinitionRef,
        txn: &ActionTxn<'_>,
        request: &OutputPolicyRequest,
        facts: &OutputPolicyFacts,
    ) -> Result<(), ParticipantError> {
        if action != self.binding.action(request.phase())
            || txn.owner().as_str() != "industry"
            || inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || world.get(inv.actor()).is_none_or(|o| {
                o.kind != ObjKind::Player || o.flags.contains(crate::flags::Flags::GOING)
            })
        {
            return Err(ParticipantError::Authority);
        }
        match (request.phase(), request.order(), facts.order_facts()) {
            (OutputPhase::Initialize, None, None) if inv.actor() == GOD => {}
            (OutputPhase::Reserve | OutputPhase::Complete, Some(id), Some(order))
                if id == order.id()
                    && order.requester() == inv.actor()
                    && order.recipe() == &self.binding.recipe =>
            {
                order.validate().map_err(|_| ParticipantError::Authority)?;
            }
            _ => return Err(ParticipantError::Authority),
        }
        Ok(())
    }
    fn fence(
        &self,
        inv: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        request: OutputPolicyRequest,
        facts: OutputPolicyFacts,
    ) -> Box<dyn image::OutputFence> {
        Box::new(image::PolicyFence {
            policy: Arc::clone(&self.policy),
            actor: inv.actor(),
            action: action.clone(),
            commit,
            request,
            facts,
        })
    }
    pub(crate) fn initialize(
        &self,
        world: &World,
        inv: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        txn: &mut ActionTxn<'_>,
        facts: OutputPolicyFacts,
    ) -> Result<image::CoreOutputObjects, ParticipantError> {
        let request = self.request(OutputPhase::Initialize, None);
        self.authority(world, inv, action, txn, &request, &facts)?;
        if !txn.family_is_empty(&self.records).map_err(state_error)?
            || !txn.family_is_empty(&self.deployment).map_err(state_error)?
        {
            return Err(ParticipantError::Authority);
        }
        let image = image::CoreOutputObjects::verify(
            world,
            inv.actor(),
            None,
            self.fence(inv, action, commit, request, facts),
        )?;
        txn.insert(
            &self.deployment,
            &(),
            OutputDeployment {
                format: FORMAT,
                initialized_by: commit,
                action: action.clone(),
                born: 0,
                consumed: 0,
            },
        )
        .map_err(state_error)?;
        Ok(image)
    }
    pub(crate) fn reserve(
        &self,
        world: &World,
        inv: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        txn: &mut ActionTxn<'_>,
        order: u64,
        facts: OutputPolicyFacts,
    ) -> Result<(ReservedCoreOutput, image::CoreOutputObjects), ParticipantError> {
        let request = self.request(OutputPhase::Reserve, Some(order));
        self.authority(world, inv, action, txn, &request, &facts)?;
        let identity = facts
            .order_facts()
            .cloned()
            .ok_or(ParticipantError::Authority)?;
        let mut deployment = txn
            .get(&self.deployment, &())
            .map_err(state_error)?
            .cloned()
            .ok_or(ParticipantError::Authority)?;
        deployment.validate()?;
        if let Some(row) = txn
            .get(&self.records, &order)
            .map_err(state_error)?
            .cloned()
        {
            require_reserved(world, &identity, &row)?;
            let image = image::CoreOutputObjects::verify(
                world,
                inv.actor(),
                Some(row.birth_object()),
                self.fence(inv, action, commit, request, facts),
            )?;
            // Same actual accepted order reuses its original acknowledged object
            // and receipt. The repeated typed image supplies a real commit fence.
            txn.replace(&self.records, &order, row.clone())
                .map_err(state_error)?;
            return Ok((
                ReservedCoreOutput {
                    row,
                    preparation: commit,
                    prepared_action: action.clone(),
                },
                image,
            ));
        }
        let image = image::CoreOutputObjects::create(
            world,
            inv.actor(),
            identity.container(),
            self.fence(inv, action, commit, request, facts),
        )?;
        let object = image.object().ok_or(ParticipantError::Authority)?;
        let row = OutputRecord {
            format: FORMAT,
            birth: deployment
                .born
                .checked_add(1)
                .ok_or(ParticipantError::ImageLimit)?,
            purpose: PURPOSE.into(),
            order: identity,
            object: object.id,
            created_at: object.created,
            object_digest: object_digest(object)?,
            reservation: commit,
            action: action.clone(),
            state: OutputState::Reserved {},
        };
        row.validate()?;
        deployment.born = row.birth;
        deployment.validate()?;
        txn.replace(&self.deployment, &(), deployment)
            .map_err(state_error)?;
        txn.insert(&self.records, &order, row.clone())
            .map_err(state_error)?;
        Ok((
            ReservedCoreOutput {
                row,
                preparation: commit,
                prepared_action: action.clone(),
            },
            image,
        ))
    }
    pub(crate) fn consume(
        &self,
        world: &World,
        inv: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        txn: &mut ActionTxn<'_>,
        order: u64,
        facts: OutputPolicyFacts,
    ) -> Result<(ReservedCoreOutput, image::CoreOutputObjects), ParticipantError> {
        let request = self.request(OutputPhase::Complete, Some(order));
        self.authority(world, inv, action, txn, &request, &facts)?;
        let identity = facts.order_facts().ok_or(ParticipantError::Authority)?;
        let mut deployment = txn
            .get(&self.deployment, &())
            .map_err(state_error)?
            .cloned()
            .ok_or(ParticipantError::Authority)?;
        deployment.validate()?;
        let mut row = txn
            .get(&self.records, &order)
            .map_err(state_error)?
            .cloned()
            .ok_or(ParticipantError::Authority)?;
        require_reserved(world, identity, &row)?;
        let reserved = ReservedCoreOutput {
            row: row.clone(),
            preparation: commit,
            prepared_action: action.clone(),
        };
        let image = image::CoreOutputObjects::verify(
            world,
            inv.actor(),
            Some(row.birth_object()),
            self.fence(inv, action, commit, request, facts),
        )?;
        row.state = OutputState::Consumed {
            completion: commit,
            action: action.clone(),
        };
        row.validate()?;
        deployment.consumed = deployment
            .consumed
            .checked_add(1)
            .ok_or(ParticipantError::ImageLimit)?;
        deployment.validate()?;
        txn.replace(&self.deployment, &(), deployment)
            .map_err(state_error)?;
        txn.replace(&self.records, &order, row)
            .map_err(state_error)?;
        Ok((reserved, image))
    }
    pub fn empty_for_initialization(&self, ctx: &ModuleCtx<'_>) -> Result<bool, ParticipantError> {
        Ok(ctx
            .scan_action_state(&self.records, None, 1)
            .map_err(|_| ParticipantError::Authority)?
            .entries()
            .is_empty()
            && ctx
                .scan_action_state(&self.deployment, None, 1)
                .map_err(|_| ParticipantError::Authority)?
                .entries()
                .is_empty())
    }
    pub fn verify_deployment(
        &self,
        ctx: &ModuleCtx<'_>,
        initialized_by: CommitToken,
    ) -> Result<(), ParticipantError> {
        let row = ctx
            .action_state(&self.deployment, &())
            .map_err(|_| ParticipantError::Authority)?
            .ok_or(ParticipantError::Authority)?;
        row.validate()?;
        if row.initialized_by != initialized_by {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
    /// Audit immutable birth and current supported state independently. The
    /// initial protocol has no delivery transition: consumed objects remain at
    /// their birth container until a separate closed, acknowledged delivery
    /// extension provides its own exact current-location history.
    pub fn audit(
        &self,
        ctx: &ModuleCtx<'_>,
        mut visit: impl FnMut(CoreOutputHistory) -> Result<(), ParticipantError>,
    ) -> Result<(), ParticipantError> {
        let deployment = ctx
            .action_state(&self.deployment, &())
            .map_err(|_| ParticipantError::Authority)?
            .ok_or(ParticipantError::Authority)?;
        deployment.validate()?;
        let markers = ctx
            .scan_action_state(&self.deployment, None, 2)
            .map_err(|_| ParticipantError::Authority)?;
        if markers.entries().len() != 1
            || markers
                .entries()
                .first()
                .is_none_or(|r| r.encoded_key() != "registry")
            || markers.next_after().is_some()
        {
            return Err(ParticipantError::Authority);
        }
        let mut births = BTreeSet::new();
        let mut objects = BTreeSet::new();
        let mut count = 0u64;
        let mut after = None;
        loop {
            let page = ctx
                .scan_action_state(&self.records, after.as_deref(), 128)
                .map_err(|_| ParticipantError::Authority)?;
            for entry in page.entries() {
                let row = entry.value();
                row.validate()?;
                if entry.encoded_key() != row.order.id().to_string()
                    || row.birth > deployment.born
                    || !births.insert(row.birth)
                    || !objects.insert(row.object)
                {
                    return Err(ParticipantError::Authority);
                }
                require_birth_world(ctx.w, row)?;
                if matches!(row.state, OutputState::Consumed { .. }) {
                    count = count.checked_add(1).ok_or(ParticipantError::ImageLimit)?;
                }
                visit(CoreOutputHistory { row: row.clone() })?;
            }
            after = page.next_after().map(ToString::to_string);
            if after.is_none() {
                break;
            }
        }
        if births.len() as u64 != deployment.born || count != deployment.consumed {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
}
fn require_reserved(
    world: &World,
    order: &CoreOutputOrder,
    row: &OutputRecord,
) -> Result<(), ParticipantError> {
    row.validate()?;
    if &row.order != order || row.state != (OutputState::Reserved {}) {
        return Err(ParticipantError::Authority);
    }
    require_birth_world(world, row)
}
fn require_birth_world(world: &World, row: &OutputRecord) -> Result<(), ParticipantError> {
    if world.get(row.object) != Some(&row.birth_object())
        || world
            .get(row.order.container())
            .is_none_or(|c| c.contents.iter().filter(|id| **id == row.object).count() != 1)
    {
        return Err(ParticipantError::Authority);
    }
    Ok(())
}
fn fixed_action(def: &DefinitionRef, id: &str) -> bool {
    def.key().owner.as_str() == "industry"
        && def.key().kind.as_str() == "action"
        && def.key().id.as_str() == id
        && def.version() == 1
}
fn mod_error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn state_error(_: crate::action_state::ActionStateError) -> ParticipantError {
    ParticipantError::Authority
}
fn bounded<T: Serialize>(row: &T) -> Result<Value, ParticipantError> {
    let v = serde_json::to_value(row).map_err(|_| ParticipantError::Encoding)?;
    canonical_json_bytes_with_limit(&v, RECORD_BYTES).map_err(|_| ParticipantError::ImageLimit)?;
    Ok(v)
}
fn encode<T: Serialize>(row: &T) -> Result<Value, String> {
    bounded(row).map_err(|e| e.to_string())
}
fn decode<T: serde::de::DeserializeOwned + Serialize>(v: &Value) -> Result<T, String> {
    canonical_json_bytes_with_limit(v, RECORD_BYTES).map_err(|e| e.to_string())?;
    let row = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    if serde_json::to_value(&row).map_err(|e| e.to_string())? != *v {
        return Err("noncanonical core output row".into());
    }
    Ok(row)
}
fn object_digest(object: &Obj) -> Result<DefinitionDigest, ParticipantError> {
    DefinitionDigest::for_canonical_value("core.industry_core_output.v1", &bounded(object)?)
        .map_err(|_| ParticipantError::Encoding)
}
pub(crate) fn descriptor(name: &str) -> Result<StateFamilyDescriptor, ModError> {
    let key = |s: &str| CanonicalId::new(format!("industry.{name}.{s}.v1")).map_err(mod_error);
    StateFamilyDescriptor::new(
        StateFamilyId::new(name).map_err(mod_error)?,
        key("key")?,
        key("value")?,
        FORMAT,
        key("decode")?,
        key("validate")?,
        key("canonical")?,
    )
    .and_then(|d| d.with_limits(32, RECORD_BYTES + 1024))
    .map_err(mod_error)
}

#[cfg(test)]
mod tests;
