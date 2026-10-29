//! Opt-in prepared replacement of one registered component. This owner adapter
//! does not prove component custody, a fee, a title or a work order. Trusted
//! composition must pair those owners before exposing any install action.

use crate::module::{NS, UnitRecordStatus, UnitStore, encode_unit_document};
use crate::unit::replacement::{
    ComponentState, ReplacementJournal, registered_spec, replace_component,
};
use crate::{ComponentId, TacticalUnit};
use mush_core::authoritative::{AuthoritativeKey, ReplayWrite};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry, MechanicalState,
    OwnerId,
};
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    CommitToken, DefinitionRef, Flags, ModError, ModuleCtx, ObjId, ObjKind, Registrar,
};
use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::sync::{Arc, OnceLock};

/// At most sixteen exact roots can participate in one frozen replacement policy
/// (doc 17h cut M8b: with an ungoverning site, two chassis families of three
/// governed roots and three ungoverned twins are twelve, and the rig's coreless
/// base thirteen; without one, seven).
pub const MAX_REPLACEMENT_ROOTS: usize = 16;
fn refused() -> ParticipantError {
    ParticipantError::refused("component replacement is unavailable or changed")
}
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

/// Trusted ruleset policy selected once during module construction. It must
/// resolve actual current owner/facility/title/custody authority from the real
/// context at every call. A request cannot install another policy or callback.
/// Tactics independently validates the entire structural replacement; returning
/// facts here never supplies a writable unit image.
pub trait ReplacementPolicy: Send + Sync + 'static {
    type Request: Send + 'static;
    fn authorize(
        &self,
        context: &OwnerPrepareContext<'_>,
        request: &Self::Request,
        current: &TacticalUnit,
    ) -> Result<ReplacementChoice, ParticipantError>;
}

/// Only target and incoming facts may cross the policy boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplacementChoice {
    target: DefinitionRef,
    incoming: ComponentState,
}
impl ReplacementChoice {
    #[must_use]
    pub fn new(target: DefinitionRef, incoming: ComponentState) -> Self {
        Self { target, incoming }
    }
}
/// Exact hull and optimistic revision, plus the concrete policy's closed input.
/// There is no actor, module, commit token or raw unit postimage parameter.
pub struct ReplacementRequest<R> {
    object: ObjId,
    revision: u64,
    policy: R,
}
impl<R> ReplacementRequest<R> {
    #[must_use]
    pub fn new(object: ObjId, revision: u64, policy: R) -> Self {
        Self {
            object,
            revision,
            policy,
        }
    }
}
/// Read-only facts derived by the real owner. Custody publication must use the
/// displaced and inserted states from this output, in this same preparation.
pub struct ReplacementOutput {
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    journal: ReplacementJournal,
}
impl ReplacementOutput {
    #[must_use]
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }
    #[must_use]
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    #[must_use]
    pub const fn actor(&self) -> ObjId {
        self.actor
    }
    #[must_use]
    pub const fn journal(&self) -> &ReplacementJournal {
        &self.journal
    }
}

#[derive(Clone)]
struct Binding {
    action: DefinitionRef,
    component: ComponentId,
    roots: BTreeSet<DefinitionRef>,
    record: DefinitionRef,
    definitions: Arc<OnceLock<DefinitionRegistry>>,
}
/// The only requestable adapter for the policy type installed by Tactics.
pub struct ComponentReplacementParticipant<P: ReplacementPolicy> {
    binding: Binding,
    policy: Arc<P>,
}
/// Opaque retained image; constructed only by the actual Tactics participant.
pub struct ReplacementPatch<R> {
    request: ReplacementRequest<R>,
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    object: ObjectFence,
    before: TacticalUnit,
    record_index: usize,
    before_record: UnitRecordStatus,
    after: TacticalUnit,
    after_record: UnitRecordStatus,
    choice: ReplacementChoice,
}
#[derive(Clone, PartialEq)]
struct ObjectFence {
    owner: ObjId,
    location: Option<ObjId>,
    kind: ObjKind,
    flags: Flags,
    binding: mush_core::BehaviorBinding,
}
fn object_fence(world: &mush_core::World, object: ObjId) -> Result<ObjectFence, ParticipantError> {
    let row = world.get(object).ok_or_else(refused)?;
    let key = mush_core::BehaviorId::new(crate::module::UNIT_BEHAVIOR).map_err(|_| refused())?;
    let binding = row.behaviors.get(&key).ok_or_else(refused)?;
    if row.id != object
        || row.kind != ObjKind::Thing
        || row.flags.contains(Flags::GOING)
        || binding.schema_version != crate::UNIT_BINDING_VERSION
    {
        return Err(refused());
    }
    Ok(ObjectFence {
        owner: row.owner,
        location: row.location,
        kind: row.kind,
        flags: row.flags,
        binding: binding.clone(),
    })
}
fn idle_world(
    world: &mush_core::World,
    actor: ObjId,
    object: ObjId,
) -> Result<(), ParticipantError> {
    let actor = world.get(actor).ok_or_else(refused)?;
    if actor.kind != ObjKind::Player || actor.flags.contains(Flags::GOING) {
        return Err(refused());
    }
    // Missing or foreign projections are not proof that the unit is free.
    for kind in [
        std::any::TypeId::of::<mush_sim::DeployedUnits>(),
        std::any::TypeId::of::<mush_sim::SpatialState>(),
        std::any::TypeId::of::<mush_sim::JourneyBook>(),
    ] {
        if world
            .module_state_owner(kind)
            .is_none_or(|owner| owner.as_str() != "sim")
        {
            return Err(refused());
        }
    }
    world
        .module_state::<mush_sim::deployed::DeployedUnits>()
        .ok_or_else(refused)?;
    mush_sim::deployed::refuse_if_custodied(world, object).map_err(|_| refused())?;
    let spatial = world
        .module_state::<mush_sim::SpatialState>()
        .ok_or_else(refused)?;
    let journeys = world
        .module_state::<mush_sim::journey::JourneyBook>()
        .ok_or_else(refused)?;
    if !journeys.held.is_empty() {
        return Err(refused());
    }
    let mut ancestors = mush_sim::graph::ancestors(spatial, object).map_err(|_| refused())?;
    ancestors.push(object);
    for subject in ancestors {
        if journeys.under_way(subject)
            || mush_sim::journey::journeying_reason(journeys, world, subject).is_some()
        {
            return Err(refused());
        }
    }
    Ok(())
}
/// Read-only current refit preflight using the same actual owner lifecycle and
/// physical exclusion checks as the prepared adapter. It grants no mutation.
pub fn current_refit_unit(
    world: &mush_core::World,
    actor: ObjId,
    object: ObjId,
) -> Result<&TacticalUnit, ParticipantError> {
    idle_world(world, actor, object)?;
    if world
        .module_state_owner(std::any::TypeId::of::<UnitStore>())
        .is_none_or(|o| o.as_str() != NS)
    {
        return Err(refused());
    }
    let state = world.module_state::<UnitStore>().ok_or_else(refused)?;
    let record = state.record_definition.as_ref().ok_or_else(refused)?;
    let (_, unit, _) = active(state, object, record)?;
    Ok(unit)
}
fn world_ready(context: &OwnerPrepareContext<'_>, object: ObjId) -> Result<(), ParticipantError> {
    current_refit_unit(context.world(), context.invocation().actor(), object).map(|_| ())
}

fn active<'a>(
    state: &'a UnitStore,
    object: ObjId,
    record: &DefinitionRef,
) -> Result<(usize, &'a TacticalUnit, &'a UnitRecordStatus), ParticipantError> {
    if state.record_definition.as_ref() != Some(record)
        || state.is_ic_custodied(object)
        || state.deployed.contains_key(&object)
        || state.boot_deployed.iter().any(|(id, _)| *id == object)
        || state.binding_repairs.iter().any(|r| r.object == object)
    {
        return Err(refused());
    }
    let unit = state.units.get(&object).ok_or_else(refused)?;
    let mut rows = state
        .records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.claims_object(object));
    let (index, row) = rows.next().ok_or_else(refused)?;
    if rows.next().is_some()
        || row.key != format!("unit:{}", object.0)
        || row.key_object != Some(object)
        || row.object != Some(object)
        || row.state != MechanicalState::Active
        || row.unit.as_ref() != Some(unit)
        || row.deployed.is_some()
        || row.migration_plan.is_some()
        || unit.object() != object
    {
        return Err(refused());
    }
    let (_, digest) = encode_unit_document(record, unit, None).map_err(|_| refused())?;
    if row.payload_digest != Some(digest) {
        return Err(refused());
    }
    Ok((index, unit, row))
}
impl Binding {
    fn definitions(
        &self,
        unit: &TacticalUnit,
        choice: &ReplacementChoice,
    ) -> Result<&DefinitionRegistry, ParticipantError> {
        if !self.roots.contains(unit.schema()) || !self.roots.contains(&choice.target) {
            return Err(refused());
        }
        let definitions = self.definitions.get().ok_or_else(refused)?;
        let target = registered_spec(definitions, &choice.target).map_err(|_| refused())?;
        let mut refs = unit.definition_references();
        refs.extend(target.body().definition_references());
        refs.extend([
            self.action.clone(),
            self.record.clone(),
            choice.target.clone(),
        ]);
        for r in refs {
            definitions.get_exact(&r).map_err(|_| refused())?;
        }
        // This lane accepts a real intact incoming component, never a damage
        // reset of unrelated cells. Its asset pointer is still not custody proof.
        let baseline = TacticalUnit::from_spec(&target, unit.object()).map_err(|_| refused())?;
        let healthy = ComponentState::capture(&baseline, &self.component).map_err(|_| refused())?;
        let incoming = &choice.incoming;
        if incoming.installed().asset.is_none_or(|a| a.0 == 0)
            || incoming.installed().condition != healthy.installed().condition
            || incoming.installed().definition != healthy.installed().definition
            || incoming.installed().install != healthy.installed().install
            || incoming.cells() != healthy.cells()
            || incoming.capabilities() != healthy.capabilities()
        {
            return Err(refused());
        }
        Ok(definitions)
    }
}
impl<P: ReplacementPolicy> OwnerParticipant for ComponentReplacementParticipant<P> {
    type State = UnitStore;
    type Request = ReplacementRequest<P::Request>;
    type Output = ReplacementOutput;
    type Patch = ReplacementPatch<P::Request>;
    type Bound<'a> = BoundReplacement<'a>;
    fn prepare(
        &self,
        state: &UnitStore,
        context: &OwnerPrepareContext<'_>,
        request: Self::Request,
    ) -> Result<OwnerPreparation<Self::Output, Self::Patch>, ParticipantError> {
        if context.action() != &self.binding.action || request.object.0 == 0 {
            return Err(refused());
        }
        world_ready(context, request.object)?;
        let object = object_fence(context.world(), request.object)?;
        let (record_index, before, before_record) =
            active(state, request.object, &self.binding.record)?;
        if before.revision() != request.revision {
            return Err(refused());
        }
        let choice = self.policy.authorize(context, &request.policy, before)?;
        let definitions = self.binding.definitions(before, &choice)?;
        let (after, journal) = replace_component(
            definitions,
            before,
            &choice.target,
            &self.binding.component,
            &choice.incoming,
            request.revision,
            context.world().now(),
        )
        .map_err(|_| refused())?;
        let (document, digest) =
            encode_unit_document(&self.binding.record, &after, None).map_err(|_| refused())?;
        let mut after_record = before_record.clone();
        after_record.unit = Some(after.clone());
        after_record.payload_digest = Some(digest);
        let output = ReplacementOutput {
            action: context.action().clone(),
            commit: context.commit_token(),
            actor: context.invocation().actor(),
            journal,
        };
        let patch = ReplacementPatch {
            request,
            action: output.action.clone(),
            commit: output.commit,
            actor: output.actor,
            object,
            before: before.clone(),
            record_index,
            before_record: before_record.clone(),
            after,
            after_record,
            choice,
        };
        let write = ReplayWrite::ModuleJsonSet {
            namespace: OwnerId::new(NS).map_err(|_| refused())?,
            key: AuthoritativeKey::parse_canonical(format!("unit:{}", patch.before.object().0))
                .map_err(|_| refused())?,
            value: document,
        };
        Ok(OwnerPreparation::new(output, patch, vec![write]))
    }
    fn recheck(
        &self,
        state: &UnitStore,
        context: &OwnerPrepareContext<'_>,
        patch: &Self::Patch,
    ) -> Result<(), ParticipantError> {
        if context.action() != &self.binding.action
            || context.action() != &patch.action
            || context.commit_token() != patch.commit
            || context.invocation().actor() != patch.actor
        {
            return Err(refused());
        }
        world_ready(context, patch.request.object)?;
        if object_fence(context.world(), patch.request.object)? != patch.object {
            return Err(refused());
        }
        let (index, before, row) = active(state, patch.request.object, &self.binding.record)?;
        if index != patch.record_index || before != &patch.before || row != &patch.before_record {
            return Err(refused());
        }
        let choice = self
            .policy
            .authorize(context, &patch.request.policy, before)?;
        if choice != patch.choice {
            return Err(refused());
        }
        self.binding.definitions(before, &choice)?;
        Ok(())
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut UnitStore,
        patch: Self::Patch,
    ) -> Result<Self::Bound<'a>, ParticipantError> {
        let (index, before, row) = active(state, patch.request.object, &self.binding.record)?;
        if index != patch.record_index || before != &patch.before || row != &patch.before_record {
            return Err(refused());
        }
        let unit = state
            .units
            .get_mut(&patch.request.object)
            .ok_or_else(refused)?;
        let record = state.records.get_mut(index).ok_or_else(refused)?;
        Ok(BoundReplacement {
            unit,
            record,
            after: patch.after,
            after_record: patch.after_record,
        })
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if namespace.as_str() == NS => {
                crate::module::replacement_write_is_exact(key.as_str(), value, &self.binding.record)
            }
            _ => false,
        }
    }
}
/// Applying only moves already validated images into the two existing slots.
pub struct BoundReplacement<'a> {
    unit: &'a mut TacticalUnit,
    record: &'a mut UnitRecordStatus,
    after: TacticalUnit,
    after_record: UnitRecordStatus,
}
impl BoundParticipant for BoundReplacement<'_> {
    fn apply(self) {
        *self.unit = self.after;
        *self.record = self.after_record;
    }
}

pub(crate) trait Installer: Send + Sync + std::fmt::Debug {
    fn descriptor(&self, record: &DefinitionRef) -> Result<DefinitionDescriptor, ModError>;
    fn register(&self, reg: &mut Registrar<'_>, record: &DefinitionRef) -> Result<(), ModError>;
    fn loaded(&self, ctx: &ModuleCtx<'_>) -> Result<(), ModError>;
}
struct Install<P: ReplacementPolicy> {
    policy: Arc<P>,
    action: DefinitionRef,
    component: ComponentId,
    roots: BTreeSet<DefinitionRef>,
    definitions: Arc<OnceLock<DefinitionRegistry>>,
    marker: PhantomData<fn() -> P>,
}
impl<P: ReplacementPolicy> std::fmt::Debug for Install<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentReplacementInstaller")
            .field("action", &self.action)
            .field("component", &self.component)
            .finish_non_exhaustive()
    }
}
impl<P: ReplacementPolicy> Installer for Install<P> {
    fn descriptor(&self, record: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
        descriptor_with_record(&self.action, &self.component, &self.roots, record)
    }
    fn register(&self, reg: &mut Registrar<'_>, record: &DefinitionRef) -> Result<(), ModError> {
        if reg.module() != NS {
            return Err(bad("replacement owner differs"));
        }
        let descriptor = self.descriptor(record)?;
        let definition = reg.definition(descriptor)?;
        reg.owner_participant(
            definition,
            BTreeSet::from([self.action.clone()]),
            ComponentReplacementParticipant {
                binding: Binding {
                    action: self.action.clone(),
                    component: self.component.clone(),
                    roots: self.roots.clone(),
                    record: record.clone(),
                    definitions: Arc::clone(&self.definitions),
                },
                policy: Arc::clone(&self.policy),
            },
        )
    }
    fn loaded(&self, ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
        let definitions = ctx
            .reg
            .definitions()
            .ok_or_else(|| bad("replacement definitions unavailable"))?;
        if self.definitions.get().is_some() {
            return Err(bad("replacement runtime was already initialized"));
        }
        self.definitions
            .set(definitions.clone())
            .map_err(|_| bad("replacement runtime was already initialized"))
    }
}

/// Exact pure registration descriptor for trusted composition. This creates no
/// policy, participant, unit state or permission to install a component.
pub fn participant_descriptor(
    action: &DefinitionRef,
    component: &ComponentId,
    roots: &BTreeSet<DefinitionRef>,
) -> Result<DefinitionDescriptor, ModError> {
    let record = mush_core::DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(NS).map_err(bad)?,
            crate::module::current_unit_record_descriptor()?,
        )
        .map_err(bad)?;
    descriptor_with_record(action, component, roots, &record)
}
fn descriptor_with_record(
    action: &DefinitionRef,
    component: &ComponentId,
    roots: &BTreeSet<DefinitionRef>,
    record: &DefinitionRef,
) -> Result<DefinitionDescriptor, ModError> {
    if action.key().kind.as_str() != "action"
        || roots.is_empty()
        || roots.len() > MAX_REPLACEMENT_ROOTS
        || roots.iter().any(|r| r.key().kind.as_str() != "unit")
    {
        return Err(bad("invalid replacement registration"));
    }
    let mut refs: Vec<_> = roots
        .iter()
        .cloned()
        .chain([action.clone(), record.clone()])
        .collect();
    refs.sort();
    refs.dedup();
    Ok(DefinitionDescriptor::new(DefinitionKind::new("owner_participant").map_err(bad)?,
        DefinitionId::new("component_replacement").map_err(bad)?, 1,
        serde_json::json!({"format":1,"component":component,"roots":roots,"action":action,"record":record,"intact_incoming":true}))
        .map_err(bad)?.with_references(refs))
}

pub(crate) fn installer<P: ReplacementPolicy>(
    policy: P,
    action: DefinitionRef,
    component: ComponentId,
    roots: BTreeSet<DefinitionRef>,
) -> Arc<dyn Installer> {
    Arc::new(Install {
        policy: Arc::new(policy),
        action,
        component,
        roots,
        definitions: Arc::new(OnceLock::new()),
        marker: PhantomData,
    })
}

#[cfg(test)]
mod tests;
