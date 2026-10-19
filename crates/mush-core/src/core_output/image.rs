//! Private built-in object union. No owner supplies an Obj postimage.
use super::policy::{
    CoreOutputPolicy, OutputPolicyFacts, OutputPolicyRequest, OutputRecheckContext,
};
use super::{NAME, object_digest};
use crate::authoritative::ReplayWrite;
use crate::flags::Flags;
use crate::mechanics::{DefinitionRef, canonical_json_bytes_with_limit};
use crate::obj::{GOD, Obj, ObjId, ObjKind};
use crate::participant::ParticipantError;
use crate::reserved_shell::{BoundShellObjects, ShellObjectStorage, ShellObjects};
use crate::{CommitToken, World};
use std::any::TypeId;
use std::fmt;
use std::sync::Arc;

pub(crate) trait OutputFence: Send {
    fn recheck(&self, world: &World) -> Result<(), ParticipantError>;
    fn local(&self, world: &World) -> Result<(), ParticipantError>;
}
pub(crate) struct PolicyFence<P: CoreOutputPolicy> {
    pub(crate) policy: Arc<P>,
    pub(crate) actor: ObjId,
    pub(crate) action: DefinitionRef,
    pub(crate) commit: CommitToken,
    pub(crate) request: OutputPolicyRequest,
    pub(crate) facts: OutputPolicyFacts,
}
impl<P: CoreOutputPolicy> PolicyFence<P> {
    fn state<'a>(&self, world: &'a World) -> Result<&'a P::State, ParticipantError> {
        if world
            .module_state_owner(TypeId::of::<P::State>())
            .is_none_or(|o| o.as_str() != "industry")
        {
            return Err(ParticipantError::Authority);
        }
        world
            .module_state::<P::State>()
            .ok_or(ParticipantError::Unavailable)
    }
}
impl<P: CoreOutputPolicy> OutputFence for PolicyFence<P> {
    fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        self.local(world)?;
        self.policy
            .recheck(
                self.state(world)?,
                &OutputRecheckContext {
                    world,
                    actor: self.actor,
                    action: &self.action,
                    commit: self.commit,
                },
                &self.request,
                &self.facts,
            )
            .map_err(|_| ParticipantError::Authority)?;
        self.local(world)
    }
    fn local(&self, world: &World) -> Result<(), ParticipantError> {
        if self.policy.generation(self.state(world)?) != self.facts.generation() {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
}

pub(crate) struct CoreOutputObjects {
    fence: Box<dyn OutputFence>,
    actor: Obj,
    before_next: u32,
    object: Option<Obj>,
    container: Option<(Obj, Option<Obj>)>,
    create: bool,
    writes: Vec<ReplayWrite>,
}
impl fmt::Debug for CoreOutputObjects {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoreOutputObjects")
            .field("create", &self.create)
            .finish_non_exhaustive()
    }
}
impl CoreOutputObjects {
    pub(crate) fn verify(
        world: &World,
        actor: ObjId,
        object: Option<Obj>,
        fence: Box<dyn OutputFence>,
    ) -> Result<Self, ParticipantError> {
        let actor = actor_image(world, actor)?;
        if object.as_ref().is_some_and(|o| world.get(o.id) != Some(o)) {
            return Err(ParticipantError::Authority);
        }
        let container = match &object {
            Some(object) => {
                let id = object.location.ok_or(ParticipantError::Authority)?;
                let before = world.get(id).cloned().ok_or(ParticipantError::Authority)?;
                if before
                    .contents
                    .iter()
                    .filter(|id| **id == object.id)
                    .count()
                    != 1
                {
                    return Err(ParticipantError::Authority);
                }
                Some((before, None))
            }
            None => None,
        };
        let image = Self {
            fence,
            actor,
            before_next: world.next_id(),
            object,
            container,
            create: false,
            writes: Vec::new(),
        };
        image.recheck(world)?;
        Ok(image)
    }
    pub(crate) fn create(
        world: &World,
        actor: ObjId,
        container: ObjId,
        fence: Box<dyn OutputFence>,
    ) -> Result<Self, ParticipantError> {
        let actor = actor_image(world, actor)?;
        let before_next = world.next_id();
        let next_id = before_next
            .checked_add(1)
            .ok_or(ParticipantError::ImageLimit)?;
        if before_next < 2 || world.get(ObjId(before_next)).is_some() {
            return Err(ParticipantError::Authority);
        }
        let before = world
            .get(container)
            .cloned()
            .ok_or(ParticipantError::Authority)?;
        if !matches!(before.kind, ObjKind::Thing | ObjKind::Room)
            || before.flags.contains(Flags::GOING)
            || before.contents.contains(&ObjId(before_next))
        {
            return Err(ParticipantError::Authority);
        }
        // Fresh allocation cannot introduce an ancestor cycle. A corrupt
        // pre-existing container chain still refuses instead of blessing it.
        let mut seen = std::collections::BTreeSet::new();
        let mut at = Some(container);
        while let Some(id) = at {
            if seen.len() >= 256 || !seen.insert(id) {
                return Err(ParticipantError::Authority);
            }
            at = world.get(id).ok_or(ParticipantError::Authority)?.location;
        }
        let mut object = Obj::new(ObjId(before_next), ObjKind::Thing, NAME, GOD, world.now());
        object.location = Some(container);
        object_digest(&object)?;
        let mut after = before.clone();
        after.contents.push(object.id);
        after.modified = world.now();
        let writes = vec![
            ReplayWrite::ObjectSet {
                object: object.clone(),
            },
            ReplayWrite::ObjectSet {
                object: after.clone(),
            },
            ReplayWrite::ObjectAllocatorSet { next_id },
        ];
        let value = serde_json::to_value(&writes).map_err(|_| ParticipantError::Encoding)?;
        canonical_json_bytes_with_limit(
            &value,
            crate::authoritative::MAX_AUTHORITATIVE_BUNDLE_BYTES,
        )
        .map_err(|_| ParticipantError::ImageLimit)?;
        let result = Self {
            fence,
            actor,
            before_next,
            object: Some(object),
            container: Some((before, Some(after))),
            create: true,
            writes,
        };
        result.recheck(world)?;
        Ok(result)
    }
    pub(crate) fn object(&self) -> Option<&Obj> {
        self.object.as_ref()
    }
    fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        self.fence.recheck(world)?;
        if world.get(self.actor.id) != Some(&self.actor)
            || world.next_id() != self.before_next
            || self
                .container
                .as_ref()
                .is_some_and(|(before, _)| world.get(before.id) != Some(before))
            || self.object.as_ref().is_some_and(|object| {
                if self.create {
                    world.get(object.id).is_some()
                } else {
                    world.get(object.id) != Some(object)
                }
            })
        {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
    fn bind(
        self,
        storage: ShellObjectStorage<'_>,
    ) -> Result<BoundCoreOutputObjects<'_>, ParticipantError> {
        let ShellObjectStorage { objects, next_id } = storage;
        if *next_id != self.before_next
            || objects.len() != self.before_next as usize
            || objects
                .get(self.actor.id.0 as usize)
                .and_then(Option::as_ref)
                != Some(&self.actor)
            || self.container.as_ref().is_some_and(|(before, _)| {
                objects.get(before.id.0 as usize).and_then(Option::as_ref) != Some(before)
            })
        {
            return Err(ParticipantError::Authority);
        }
        if self.create {
            if self.object.as_ref().is_none_or(|o| o.id.0 != *next_id) || self.container.is_none() {
                return Err(ParticipantError::Authority);
            }
            objects
                .try_reserve(1)
                .map_err(|_| ParticipantError::ImageLimit)?;
        } else if self
            .object
            .as_ref()
            .is_some_and(|o| objects.get(o.id.0 as usize).and_then(Option::as_ref) != Some(o))
        {
            return Err(ParticipantError::Authority);
        }
        let next = if self.create {
            next_id.checked_add(1).ok_or(ParticipantError::ImageLimit)?
        } else {
            *next_id
        };
        // Every occupied index and the append capacity are now validated under
        // the retained exclusive Vec borrow; apply makes no fallible decisions.
        Ok(BoundCoreOutputObjects {
            objects,
            next_id,
            next,
            created: self.create.then_some(self.object).flatten(),
            container: self.container.and_then(|(_, after)| after),
        })
    }
}
fn actor_image(world: &World, actor: ObjId) -> Result<Obj, ParticipantError> {
    let object = world
        .get(actor)
        .cloned()
        .ok_or(ParticipantError::Authority)?;
    if object.kind != ObjKind::Player
        || object.flags.contains(Flags::GOING)
        || world.get(GOD).is_none()
    {
        return Err(ParticipantError::Authority);
    }
    Ok(object)
}
pub(crate) struct BoundCoreOutputObjects<'a> {
    objects: &'a mut Vec<Option<Obj>>,
    next_id: &'a mut u32,
    next: u32,
    created: Option<Obj>,
    container: Option<Obj>,
}
impl BoundCoreOutputObjects<'_> {
    fn apply(self) {
        if let Some(container) = self.container {
            // This exact occupied index was validated under the retained
            // exclusive Vec borrow in bind. No callback or lookup can intervene.
            let index = container.id.0 as usize;
            self.objects[index] = Some(container);
        }
        if let Some(object) = self.created {
            self.objects.push(Some(object));
        }
        *self.next_id = self.next;
    }
}

#[derive(Debug)]
pub(crate) enum BuiltinObjects {
    Shell(ShellObjects),
    CoreOutput(CoreOutputObjects),
}
impl From<ShellObjects> for BuiltinObjects {
    fn from(value: ShellObjects) -> Self {
        Self::Shell(value)
    }
}
impl BuiltinObjects {
    pub(crate) fn creates_object(&self) -> bool {
        match self {
            Self::Shell(x) => x.creates_object(),
            Self::CoreOutput(x) => x.create,
        }
    }
    pub(crate) fn writes(&self) -> &[ReplayWrite] {
        match self {
            Self::Shell(x) => x.writes(),
            Self::CoreOutput(x) => &x.writes,
        }
    }
    pub(crate) fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        match self {
            Self::Shell(x) => x.recheck(world),
            Self::CoreOutput(x) => x.recheck(world),
        }
    }
    pub(crate) fn local(&self, world: &World) -> Result<(), ParticipantError> {
        match self {
            Self::Shell(_) => Ok(()),
            Self::CoreOutput(x) => x.fence.local(world),
        }
    }
    pub(crate) fn bind(
        self,
        storage: ShellObjectStorage<'_>,
    ) -> Result<BoundBuiltinObjects<'_>, ParticipantError> {
        match self {
            Self::Shell(x) => x.bind(storage).map(BoundBuiltinObjects::Shell),
            Self::CoreOutput(x) => x.bind(storage).map(BoundBuiltinObjects::CoreOutput),
        }
    }
}
pub(crate) enum BoundBuiltinObjects<'a> {
    Shell(BoundShellObjects<'a>),
    CoreOutput(BoundCoreOutputObjects<'a>),
}
impl BoundBuiltinObjects<'_> {
    pub(crate) fn apply(self) {
        match self {
            Self::Shell(x) => x.apply(),
            Self::CoreOutput(x) => x.apply(),
        }
    }
}
