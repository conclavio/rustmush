//! Private physical image for the fixed completed-Core delivery operation.
//! The trusted Industry policy and typed service are wired separately; no
//! provider can supply a World object postimage through this module.
use crate::action_state::TypedStateLens;
use crate::authoritative::ReplayWrite;
use crate::mechanics::canonical_json_bytes_with_limit;
use crate::module::{HookReadContext, HookReg};
use crate::participant::ParticipantError;
use crate::reserved_shell::ShellObjectStorage;
use crate::{Flags, Obj, ObjId, ObjKind, World};
use std::collections::BTreeSet;
use std::sync::Arc;

// This is a private local ceiling, not an increase to the authoritative bundle
// cap. Complete replay-envelope boundary tests are required before publication.
const IMAGE_BYTES: usize = 1_048_576;

/// Core captures the real registered hook slice. There is no detached lens or
/// public constructor, and hooks are never invoked during acknowledged apply.
#[derive(Clone, Debug)]
pub(crate) struct CarryCheck {
    hooks: Arc<[HookReg]>,
    actor: ObjId,
    object: ObjId,
    destination: ObjId,
    exchange: Option<Arc<crate::core_exchange::ExchangeCarryAttempt>>,
}
impl CarryCheck {
    pub(crate) fn capture(
        hooks: Arc<[HookReg]>,
        world: &World,
        lens: TypedStateLens<'_>,
        actor: ObjId,
        object: ObjId,
        destination: ObjId,
    ) -> Result<Self, ParticipantError> {
        let result = Self {
            hooks,
            actor,
            object,
            destination,
            exchange: None,
        };
        result.recheck(world, lens)?;
        Ok(result)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture_exchange(
        hooks: Arc<[HookReg]>,
        world: &World,
        lens: TypedStateLens<'_>,
        actor: ObjId,
        object: ObjId,
        destination: ObjId,
        exchange: Arc<crate::core_exchange::ExchangeCarryAttempt>,
    ) -> Result<Self, ParticipantError> {
        let result = Self {
            hooks,
            actor,
            object,
            destination,
            exchange: Some(exchange),
        };
        result.recheck(world, lens)?;
        Ok(result)
    }
    pub(crate) fn recheck(
        &self,
        world: &World,
        lens: TypedStateLens<'_>,
    ) -> Result<(), ParticipantError> {
        if !lens.is_live() {
            return Err(ParticipantError::Unavailable);
        }
        if let Some(attempt) = &self.exchange {
            attempt.require_move(self.actor, self.object, self.destination)?;
        }
        for hook in self.hooks.iter() {
            let context = HookReadContext::for_module(world, lens, &hook.module);
            let context = if let Some(attempt) = &self.exchange {
                context.with_core_exchange_attempt(attempt)
            } else {
                context
            };
            hook.hook
                .carry_attempt(&context, self.actor, self.object, self.destination)
                .map_err(|_| ParticipantError::Authority)?;
        }
        Ok(())
    }
}

/// The delivery service supplies an opaque registered-owner fence, not raw
/// object images. Business freshness is repeated only before writer custody.
pub(crate) trait DeliveryFence: Send {
    fn recheck(&self, world: &World) -> Result<(), ParticipantError>;
    fn local(&self, world: &World) -> Result<(), ParticipantError>;
}

pub(crate) struct DeliveryObjects {
    before: [Obj; 3],
    after: [Obj; 3],
    ancestors: Vec<Obj>,
    carry: CarryCheck,
    fence: Box<dyn DeliveryFence>,
    writes: Vec<ReplayWrite>,
}
impl std::fmt::Debug for DeliveryObjects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeliveryObjects")
            .field("objects", &self.before.each_ref().map(|o| o.id))
            .finish_non_exhaustive()
    }
}
impl DeliveryObjects {
    pub(crate) fn prepare(
        world: &World,
        lens: TypedStateLens<'_>,
        object: ObjId,
        container: ObjId,
        requester: ObjId,
        carry: CarryCheck,
        fence: Box<dyn DeliveryFence>,
    ) -> Result<Self, ParticipantError> {
        if object == container
            || object == requester
            || container == requester
            || carry.actor != requester
            || carry.object != object
            || carry.destination != requester
        {
            return Err(ParticipantError::Authority);
        }
        let get = |id| world.get(id).cloned().ok_or(ParticipantError::Authority);
        let item = get(object)?;
        let source = get(container)?;
        let destination = get(requester)?;
        if item.kind != ObjKind::Thing
            || item.location != Some(container)
            || !item.contents.is_empty()
            || !item.exits.is_empty()
            || !matches!(source.kind, ObjKind::Thing | ObjKind::Room)
            || destination.kind != ObjKind::Player
            || [&item, &source, &destination]
                .into_iter()
                .any(|o| o.flags.contains(Flags::GOING))
            || source.contents.iter().filter(|id| **id == object).count() != 1
            || destination.contents.contains(&object)
            || source.exits.contains(&object)
            || destination.exits.contains(&object)
        {
            return Err(ParticipantError::Authority);
        }
        world
            .can_move_to(object, requester)
            .map_err(|_| ParticipantError::Authority)?;
        // Capture complete ancestry preimages too: holder ancestry may change
        // even while the three written objects remain byte-identical.
        let mut ancestors = Vec::new();
        let mut retained = BTreeSet::new();
        for start in [container, requester] {
            let mut seen = BTreeSet::new();
            let mut at = Some(start);
            while let Some(id) = at {
                if seen.len() >= crate::MAX_CONTAINMENT_DEPTH || !seen.insert(id) || id == object {
                    return Err(ParticipantError::Authority);
                }
                let parent = get(id)?;
                if parent.flags.contains(Flags::GOING) {
                    return Err(ParticipantError::Authority);
                }
                if retained.insert(id) {
                    ancestors.push(parent.clone());
                }
                at = parent.location;
            }
        }
        // Adding the leaf below destination must fit the actual global depth.
        let mut depth = 1usize;
        let mut at = Some(requester);
        while let Some(id) = at {
            depth = depth.checked_add(1).ok_or(ParticipantError::ImageLimit)?;
            if depth > crate::MAX_CONTAINMENT_DEPTH {
                return Err(ParticipantError::Authority);
            }
            at = world.get(id).ok_or(ParticipantError::Authority)?.location;
        }
        let before = [item, source, destination];
        let mut after = before.clone();
        after[0].location = Some(requester);
        after[0].modified = world.now();
        after[1].contents.retain(|id| *id != object);
        after[1].modified = world.now();
        after[2].contents.push(object);
        after[2].modified = world.now();
        let writes: Vec<_> = after
            .iter()
            .cloned()
            .map(|object| ReplayWrite::ObjectSet { object })
            .collect();
        let value = serde_json::to_value(&writes).map_err(|_| ParticipantError::Encoding)?;
        canonical_json_bytes_with_limit(&value, IMAGE_BYTES)
            .map_err(|_| ParticipantError::ImageLimit)?;
        let result = Self {
            before,
            after,
            ancestors,
            carry,
            fence,
            writes,
        };
        result.recheck(world, lens)?;
        Ok(result)
    }
    pub(crate) fn writes(&self) -> &[ReplayWrite] {
        &self.writes
    }
    pub(crate) fn recheck(
        &self,
        world: &World,
        lens: TypedStateLens<'_>,
    ) -> Result<(), ParticipantError> {
        self.fence.recheck(world)?;
        self.local(world)?;
        self.carry.recheck(world, lens)?;
        self.local(world)
    }
    pub(crate) fn local(&self, world: &World) -> Result<(), ParticipantError> {
        self.fence.local(world)?;
        if self
            .before
            .iter()
            .chain(&self.ancestors)
            .any(|o| world.get(o.id) != Some(o))
        {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
    pub(crate) fn bind(
        self,
        storage: ShellObjectStorage<'_>,
    ) -> Result<BoundDeliveryObjects<'_>, ParticipantError> {
        let ShellObjectStorage { objects, .. } = storage;
        if self
            .before
            .iter()
            .chain(&self.ancestors)
            .any(|o| objects.get(o.id.0 as usize).and_then(Option::as_ref) != Some(o))
        {
            return Err(ParticipantError::Authority);
        }
        Ok(BoundDeliveryObjects {
            objects,
            after: self.after,
        })
    }
}
pub(crate) struct BoundDeliveryObjects<'a> {
    objects: &'a mut Vec<Option<Obj>>,
    after: [Obj; 3],
}
impl BoundDeliveryObjects<'_> {
    pub(crate) fn apply(self) {
        // All exact occupied indices are bound under the same exclusive borrow.
        for object in self.after {
            let index = object.id.0 as usize;
            self.objects[index] = Some(object);
        }
    }
}

mod policy;
mod records;
use crate::action::{ActionInvocation, PerceivedRef, PrincipalId};
use crate::action_state::{ActionTxn, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use crate::core_output::{CoreOutputPolicy, CoreOutputReservations};
use crate::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, OwnerId,
};
use crate::read_service::OwnerReadHandle;
use crate::{CommitToken, ModError, ModuleCtx, Registrar};
pub use policy::{CoreDeliveryPolicy, DeliveryFacts, DeliveryRequest};
use records::DeliveryRecord;
pub use records::{CoreDeliveryHistory, DeliveredCoreOutput};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const ACTION: &str = "deliver_core_output";
const RECORD_BYTES: usize = 16_384;
const FAMILY: &str = "core_output_deliveries";

/// Trusted construction selects the actual Industry policy and previously
/// registered output service. No caller provides a source, destination or image.
pub struct CoreOutputDeliveries<P: CoreOutputPolicy, D: CoreDeliveryPolicy> {
    outputs: CoreOutputReservations<P>,
    records: StateFamilyHandle<u64, DeliveryRecord>,
    reader: OwnerReadHandle<policy::DeliveryReader<D>>,
    policy: Arc<D>,
    action: DefinitionRef,
}
impl<P: CoreOutputPolicy, D: CoreDeliveryPolicy> Clone for CoreOutputDeliveries<P, D> {
    fn clone(&self) -> Self {
        Self {
            outputs: self.outputs.clone(),
            records: self.records.clone(),
            reader: self.reader.clone(),
            policy: Arc::clone(&self.policy),
            action: self.action.clone(),
        }
    }
}
impl<P: CoreOutputPolicy, D: CoreDeliveryPolicy> CoreOutputDeliveries<P, D> {
    pub fn policy_descriptor(action: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
        if !crate::core_output::fixed_action(action, ACTION) {
            return Err(error("invalid Core delivery action"));
        }
        Ok(DefinitionDescriptor::new(
            DefinitionKind::new("owner_read").map_err(error)?,
            DefinitionId::new("core_delivery_policy").map_err(error)?,
            1,
            json!({"format":1,"protocol":"industry_core_delivery_v1","action":action,
                "objects":3,"image_bytes":IMAGE_BYTES,"record_bytes":RECORD_BYTES,
                "destination":"actual_requester","storage":"ordinary_container"}),
        )
        .map_err(error)?
        .with_references(vec![action.clone()]))
    }
    pub fn register(
        reg: &mut Registrar<'_>,
        outputs: CoreOutputReservations<P>,
        action: DefinitionRef,
        policy: D,
        mut dependencies: BTreeSet<OwnerId>,
    ) -> Result<Self, ModError> {
        if reg.module() != "industry" || !crate::core_output::fixed_action(&action, ACTION) {
            return Err(error("invalid fixed Core delivery binding"));
        }
        dependencies.insert(OwnerId::new("industry").map_err(error)?);
        let descriptor = Self::policy_descriptor(&action)?;
        let definition = reg.definition(descriptor)?;
        let policy = Arc::new(policy);
        reg.owner_read_service(
            definition.clone(),
            BTreeSet::from([OwnerId::new("industry").map_err(error)?]),
            dependencies,
            policy::DeliveryReader {
                policy: Arc::clone(&policy),
            },
        )?;
        let reader = reg.request_owner_read::<policy::DeliveryReader<D>>(definition)?;
        let key =
            |name: &str| CanonicalId::new(format!("industry.{FAMILY}.{name}.v1")).map_err(error);
        let descriptor = StateFamilyDescriptor::new(
            StateFamilyId::new(FAMILY).map_err(error)?,
            key("key")?,
            key("value")?,
            1,
            key("decode")?,
            key("validate")?,
            key("canonical")?,
        )
        .and_then(|d| d.with_limits(32, RECORD_BYTES))
        .map_err(error)?;
        let records = reg.state_family(
            descriptor,
            |order: &u64| Ok(order.to_string()),
            |v: &Value| decode(v),
            |r: &DeliveryRecord| r.validate().map_err(|e| e.to_string()),
            |r: &DeliveryRecord| bounded(r).map_err(|e| e.to_string()),
        )?;
        Ok(Self {
            outputs,
            records,
            reader,
            policy,
            action,
        })
    }
    pub(crate) fn reader(&self) -> &OwnerReadHandle<policy::DeliveryReader<D>> {
        &self.reader
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &self,
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        txn: &mut ActionTxn<'_>,
        order: u64,
        facts: DeliveryFacts,
        hooks: Arc<[HookReg]>,
        base: &crate::action_state::ActionReadBase,
        provider: &crate::action::ProviderRegistrationOrdinal,
    ) -> Result<(DeliveredCoreOutput, DeliveryObjects), ParticipantError> {
        if action != &self.action
            || txn.owner().as_str() != "industry"
            || facts.order().id() != order
            || facts.order().requester() != invocation.actor()
            || invocation.principal()
                != &(PrincipalId::Object {
                    id: invocation.actor(),
                })
            || invocation.subject() != &PerceivedRef::SelfActor
            || !invocation.targets().is_empty()
        {
            return Err(ParticipantError::Authority);
        }
        let output = txn
            .get(&self.outputs.records, &order)
            .map_err(state_error)?
            .cloned()
            .ok_or(ParticipantError::Authority)?;
        output.validate()?;
        let crate::core_output::records::OutputState::Consumed { completion, .. } = &output.state
        else {
            return Err(ParticipantError::Authority);
        };
        if output.order != *facts.order()
            || output.object != facts.object()
            || output.reservation != facts.reservation()
            || *completion != facts.completion()
            || txn
                .get(&self.records, &order)
                .map_err(state_error)?
                .is_some()
        {
            return Err(ParticipantError::Authority);
        }
        crate::core_output::require_birth_world(world, &output)?;
        let image = {
            let lens = txn
                .read_preimage(base, provider)
                .map_err(|_| ParticipantError::Authority)?;
            let carry = CarryCheck::capture(
                hooks,
                world,
                lens,
                invocation.actor(),
                output.object,
                output.order.requester(),
            )?;
            let fence = Box::new(policy::PolicyFence {
                policy: Arc::clone(&self.policy),
                actor: invocation.actor(),
                action: action.clone(),
                commit,
                request: DeliveryRequest { order },
                facts,
            });
            DeliveryObjects::prepare(
                world,
                lens,
                output.object,
                output.order.container(),
                output.order.requester(),
                carry,
                fence,
            )?
        };
        let mut delivered = output.birth_object();
        delivered.location = Some(output.order.requester());
        delivered.modified = world.now();
        let row = DeliveryRecord {
            format: 1,
            operation: commit,
            action: action.clone(),
            output,
            at_utc: world.now(),
            delivered_digest: crate::core_output::object_digest(&delivered)?,
        };
        row.validate()?;
        let envelope = json!({"family":{"owner":"industry","id":FAMILY},
            "schema":{"id":format!("industry.{FAMILY}.value.v1"),"version":1},"key":order.to_string(),"value":row});
        canonical_json_bytes_with_limit(&envelope, RECORD_BYTES)
            .map_err(|_| ParticipantError::ImageLimit)?;
        txn.insert(&self.records, &order, row.clone())
            .map_err(state_error)?;
        Ok((DeliveredCoreOutput { row }, image))
    }
    /// The full output birth census is retained. This closed path recognizes
    /// only its own actual delivery family; every row is reverse-joined and
    /// returned for Industry's independent same-ack receipt/index audit.
    pub fn audit(
        &self,
        ctx: &ModuleCtx<'_>,
        mut output: impl FnMut(crate::core_output::CoreOutputHistory) -> Result<(), ParticipantError>,
        mut delivery: impl FnMut(CoreDeliveryHistory) -> Result<(), ParticipantError>,
    ) -> Result<(), ParticipantError> {
        let mut rows = BTreeMap::new();
        let mut after = None;
        loop {
            let page = ctx
                .scan_action_state(&self.records, after.as_deref(), 128)
                .map_err(|_| ParticipantError::Authority)?;
            for entry in page.entries() {
                let row = entry.value();
                row.validate()?;
                if rows.len() >= crate::action_state::MAX_ACTION_STATE_BOOT_RECORDS
                    || entry.encoded_key() != row.output.order.id().to_string()
                    || rows.insert(row.output.order.id(), row.clone()).is_some()
                {
                    return Err(ParticipantError::Authority);
                }
            }
            after = page.next_after().map(ToString::to_string);
            if after.is_none() {
                break;
            }
        }
        let mut seen = BTreeSet::new();
        self.outputs.audit_checked(
            ctx,
            |world, birth| {
                if let Some(row) = rows.get(&birth.order.id()) {
                    if row.output != *birth || !seen.insert(birth.order.id()) {
                        return Err(ParticipantError::Authority);
                    }
                    require_delivered_world(world, row)
                } else {
                    crate::core_output::require_birth_world(world, birth)
                }
            },
            |history| output(history),
        )?;
        if seen.len() != rows.len() {
            return Err(ParticipantError::Authority);
        }
        for row in rows.into_values() {
            delivery(CoreDeliveryHistory {
                receipt: DeliveredCoreOutput { row },
            })?;
        }
        Ok(())
    }
    pub fn require_no_deliveries(&self, ctx: &ModuleCtx<'_>) -> Result<(), ParticipantError> {
        if !ctx
            .scan_action_state(&self.records, None, 1)
            .map_err(|_| ParticipantError::Authority)?
            .entries()
            .is_empty()
        {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
}
fn require_delivered_world(world: &World, row: &DeliveryRecord) -> Result<(), ParticipantError> {
    let object = world
        .get(row.output.object)
        .ok_or(ParticipantError::Authority)?;
    let mut immutable = object.clone();
    // Ordinary later carry updates only these two fields. Fitted/departed or
    // otherwise transformed cores need their own future closed history route.
    immutable.location = Some(row.output.order.container());
    immutable.modified = row.output.created_at;
    if immutable != row.output.birth_object() || object.flags.contains(Flags::GOING) {
        return Err(ParticipantError::Authority);
    }
    let mut at = Some(object.id);
    let mut seen = BTreeSet::new();
    while let Some(id) = at {
        if seen.len() >= crate::MAX_CONTAINMENT_DEPTH || !seen.insert(id) {
            return Err(ParticipantError::Authority);
        }
        let child = world.get(id).ok_or(ParticipantError::Authority)?;
        if child.flags.contains(Flags::GOING) {
            return Err(ParticipantError::Authority);
        }
        if let Some(parent) = child.location {
            let holder = world.get(parent).ok_or(ParticipantError::Authority)?;
            if holder.contents.iter().filter(|x| **x == id).count() != 1
                || holder.exits.contains(&id)
            {
                return Err(ParticipantError::Authority);
            }
        } else if id == object.id {
            return Err(ParticipantError::Authority);
        }
        at = child.location;
    }
    Ok(())
}
fn error(e: impl std::fmt::Display) -> ModError {
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
fn decode(v: &Value) -> Result<DeliveryRecord, String> {
    canonical_json_bytes_with_limit(v, RECORD_BYTES).map_err(|e| e.to_string())?;
    let row: DeliveryRecord = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    if serde_json::to_value(&row).map_err(|e| e.to_string())? != *v {
        return Err("noncanonical Core delivery record".into());
    }
    Ok(row)
}

#[cfg(test)]
mod tests;
