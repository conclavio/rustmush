//! **Opt-in prepared refit of a hull to another registered root** (doc 31 cut
//! UD.4b, D-31-14/15 as amended): the owner adapter the generalized refit
//! pipeline asks to move a hull's definition. It is
//! [`crate::component_replacement`]'s sibling — the same owner lifecycle and
//! physical exclusion fences, the same exact-record and exact-write rules —
//! but it runs [`refit_component`]: at most one component out and one in, on
//! one mount, each under its own variant's id. Like its sibling it proves no
//! custody, fee, title or work order; trusted composition pairs those owners.
//!
//! It registers under its own participant id (`component_refit`), so the
//! kernel's core-exchange pairing, which names `component_replacement`,
//! is untouched.

use crate::component_replacement::{Installer, ObjectFence, active, object_fence, world_ready};
use crate::module::{NS, UnitRecordStatus, UnitStore, encode_unit_document};
use crate::unit::replacement::{ComponentState, RefitJournal, refit_component, registered_spec};
use crate::{CellId, ComponentId, TacticalUnit};
use mush_core::authoritative::{AuthoritativeKey, ReplayWrite};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry, OwnerId,
};
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{CommitToken, DefinitionRef, ModError, ModuleCtx, ObjId, Registrar};
use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::sync::{Arc, OnceLock};

/// At most this many exact roots take part in one frozen refit policy.
pub const MAX_REFIT_ROOTS: usize = 64;

fn refused() -> ParticipantError {
    ParticipantError::refused("refit is unavailable or changed")
}
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

/// Trusted ruleset policy selected once at module construction. It must
/// resolve the actual owner, facility, title and part authority from the real
/// context at every call; Tactics independently validates the whole
/// structural refit, and a returned choice never supplies a writable image.
pub trait RefitPolicy: Send + Sync + 'static {
    type Request: Send + 'static;
    fn authorize(
        &self,
        context: &OwnerPrepareContext<'_>,
        request: &Self::Request,
        current: &TacticalUnit,
    ) -> Result<RefitChoice, ParticipantError>;
}

/// Only the target, what comes out and what goes in cross the policy boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct RefitChoice {
    target: DefinitionRef,
    remove: Option<ComponentId>,
    install: Option<(ComponentId, ComponentState)>,
}
impl RefitChoice {
    #[must_use]
    pub fn new(
        target: DefinitionRef,
        remove: Option<ComponentId>,
        install: Option<(ComponentId, ComponentState)>,
    ) -> Self {
        Self {
            target,
            remove,
            install,
        }
    }
}

/// Exact hull and optimistic revision, plus the concrete policy's closed input.
pub struct RefitRequest<R> {
    object: ObjId,
    revision: u64,
    policy: R,
}
impl<R> RefitRequest<R> {
    #[must_use]
    pub fn new(object: ObjId, revision: u64, policy: R) -> Self {
        Self {
            object,
            revision,
            policy,
        }
    }
}

/// Read-only facts derived by the real owner: custody publication uses the
/// removed and installed states from this output, in this same preparation.
pub struct RefitOutput {
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    journal: RefitJournal,
}
impl RefitOutput {
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
    pub const fn journal(&self) -> &RefitJournal {
        &self.journal
    }
}

#[derive(Clone)]
struct Binding {
    action: DefinitionRef,
    roots: BTreeSet<DefinitionRef>,
    derived: Vec<CellId>,
    record: DefinitionRef,
    definitions: Arc<OnceLock<DefinitionRegistry>>,
}

/// The only requestable adapter for the policy type installed by Tactics.
pub struct ComponentRefitParticipant<P: RefitPolicy> {
    binding: Binding,
    policy: Arc<P>,
}

/// Opaque retained image; constructed only by the actual Tactics participant.
pub struct RefitPatch<R> {
    request: RefitRequest<R>,
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    object: ObjectFence,
    before: TacticalUnit,
    record_index: usize,
    before_record: UnitRecordStatus,
    after: TacticalUnit,
    after_record: UnitRecordStatus,
    choice: RefitChoice,
}

impl Binding {
    fn definitions(
        &self,
        unit: &TacticalUnit,
        choice: &RefitChoice,
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
        // A fitted part goes in as a real intact part: never a reset of
        // unrelated cells, never a damaged incoming body.
        if let Some((id, incoming)) = &choice.install {
            let baseline =
                TacticalUnit::from_spec(&target, unit.object()).map_err(|_| refused())?;
            let healthy = ComponentState::capture(&baseline, id).map_err(|_| refused())?;
            if incoming.installed().condition != healthy.installed().condition
                || incoming.installed().definition != healthy.installed().definition
                || incoming.installed().install != healthy.installed().install
                || incoming.cells() != healthy.cells()
                || incoming.capabilities() != healthy.capabilities()
            {
                return Err(refused());
            }
        }
        Ok(definitions)
    }

    fn refit(
        &self,
        definitions: &DefinitionRegistry,
        before: &TacticalUnit,
        choice: &RefitChoice,
        revision: u64,
        now: i64,
    ) -> Result<(TacticalUnit, RefitJournal), ParticipantError> {
        refit_component(
            definitions,
            before,
            &choice.target,
            choice.remove.as_ref(),
            choice.install.as_ref().map(|(id, state)| (id, state)),
            &self.derived,
            revision,
            now,
        )
        .map_err(|_| refused())
    }
}

impl<P: RefitPolicy> OwnerParticipant for ComponentRefitParticipant<P> {
    type State = UnitStore;
    type Request = RefitRequest<P::Request>;
    type Output = RefitOutput;
    type Patch = RefitPatch<P::Request>;
    type Bound<'a> = BoundRefit<'a>;
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
        let (after, journal) = self.binding.refit(
            definitions,
            before,
            &choice,
            request.revision,
            context.world().now(),
        )?;
        let (document, digest) =
            encode_unit_document(&self.binding.record, &after, None).map_err(|_| refused())?;
        let mut after_record = before_record.clone();
        after_record.unit = Some(after.clone());
        after_record.payload_digest = Some(digest);
        let output = RefitOutput {
            action: context.action().clone(),
            commit: context.commit_token(),
            actor: context.invocation().actor(),
            journal,
        };
        let patch = RefitPatch {
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
        Ok(BoundRefit {
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
pub struct BoundRefit<'a> {
    unit: &'a mut TacticalUnit,
    record: &'a mut UnitRecordStatus,
    after: TacticalUnit,
    after_record: UnitRecordStatus,
}
impl BoundParticipant for BoundRefit<'_> {
    fn apply(self) {
        *self.unit = self.after;
        *self.record = self.after_record;
    }
}

struct Install<P: RefitPolicy> {
    policy: Arc<P>,
    action: DefinitionRef,
    roots: BTreeSet<DefinitionRef>,
    derived: Vec<CellId>,
    definitions: Arc<OnceLock<DefinitionRegistry>>,
    marker: PhantomData<fn() -> P>,
}
impl<P: RefitPolicy> std::fmt::Debug for Install<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentRefitInstaller")
            .field("action", &self.action)
            .field("roots", &self.roots.len())
            .finish_non_exhaustive()
    }
}
impl<P: RefitPolicy> Installer for Install<P> {
    fn descriptor(&self, record: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
        descriptor_with_record(&self.action, &self.roots, &self.derived, record)
    }
    fn register(&self, reg: &mut Registrar<'_>, record: &DefinitionRef) -> Result<(), ModError> {
        if reg.module() != NS {
            return Err(bad("refit owner differs"));
        }
        let descriptor = self.descriptor(record)?;
        let definition = reg.definition(descriptor)?;
        reg.owner_participant(
            definition,
            BTreeSet::from([self.action.clone()]),
            ComponentRefitParticipant {
                binding: Binding {
                    action: self.action.clone(),
                    roots: self.roots.clone(),
                    derived: self.derived.clone(),
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
            .ok_or_else(|| bad("refit definitions unavailable"))?;
        if self.definitions.get().is_some() {
            return Err(bad("refit runtime was already initialized"));
        }
        self.definitions
            .set(definitions.clone())
            .map_err(|_| bad("refit runtime was already initialized"))
    }
}

/// Exact pure registration descriptor for trusted composition. This creates no
/// policy, participant, unit state or permission to refit anything.
pub fn participant_descriptor(
    action: &DefinitionRef,
    roots: &BTreeSet<DefinitionRef>,
    derived: &[CellId],
) -> Result<DefinitionDescriptor, ModError> {
    let record = mush_core::DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(NS).map_err(bad)?,
            crate::module::current_unit_record_descriptor()?,
        )
        .map_err(bad)?;
    descriptor_with_record(action, roots, derived, &record)
}

fn descriptor_with_record(
    action: &DefinitionRef,
    roots: &BTreeSet<DefinitionRef>,
    derived: &[CellId],
    record: &DefinitionRef,
) -> Result<DefinitionDescriptor, ModError> {
    if action.key().kind.as_str() != "action"
        || roots.len() < 2
        || roots.len() > MAX_REFIT_ROOTS
        || roots.iter().any(|r| r.key().kind.as_str() != "unit")
    {
        return Err(bad("invalid refit registration"));
    }
    let mut refs: Vec<_> = roots
        .iter()
        .cloned()
        .chain([action.clone(), record.clone()])
        .collect();
    refs.sort();
    refs.dedup();
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_participant").map_err(bad)?,
        DefinitionId::new("component_refit").map_err(bad)?,
        1,
        serde_json::json!({
            "format": 1,
            "roots": roots,
            "derived": derived,
            "action": action,
            "record": record,
            "intact_incoming": true,
            "mount_stable": true,
        }),
    )
    .map_err(bad)?
    .with_references(refs))
}

pub(crate) fn installer<P: RefitPolicy>(
    policy: P,
    action: DefinitionRef,
    roots: BTreeSet<DefinitionRef>,
    derived: Vec<CellId>,
) -> Arc<dyn Installer> {
    Arc::new(Install {
        policy: Arc::new(policy),
        action,
        roots,
        derived,
        definitions: Arc::new(OnceLock::new()),
        marker: PhantomData,
    })
}

#[cfg(test)]
mod tests;
