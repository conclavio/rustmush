//! Merged Core images for one bounded, registered Sim structural transition.
use super::*;
use crate::authoritative::ReplayWrite;
use crate::core_delivery::CarryCheck;
use crate::reserved_shell::ShellObjectStorage;
use crate::{Flags, Obj, ObjKind};
use std::collections::BTreeMap;

pub(crate) struct PhysicalTransitionObjects {
    before: BTreeMap<ObjId, Obj>,
    after: BTreeMap<ObjId, Obj>,
    carry: Vec<CarryCheck>,
    pub(crate) participant: DefinitionRef,
    pub(crate) action: DefinitionRef,
    pub(crate) commit: CommitToken,
    writes: Vec<ReplayWrite>,
}
impl std::fmt::Debug for PhysicalTransitionObjects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhysicalTransitionObjects")
            .field("objects", &self.after.keys())
            .finish_non_exhaustive()
    }
}
impl PhysicalTransitionObjects {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        world: &World,
        lens: crate::action_state::TypedStateLens<'_>,
        actor: ObjId,
        draft: &PhysicalTransitionDraft,
        hooks: Arc<[crate::module::HookReg]>,
        move_hooks: Arc<[crate::module::HookReg]>,
        participant: DefinitionRef,
        action: DefinitionRef,
        commit: CommitToken,
    ) -> Result<Self, ParticipantError> {
        let mut before = BTreeMap::new();
        let mut read_bytes = 2usize;
        capture_chain(world, actor, &mut before, &mut read_bytes)?;
        capture_chain(world, draft.root, &mut before, &mut read_bytes)?;
        // Spatial changes may have no location projection at all. Their real
        // owner-selected World dependencies still bind through acknowledged
        // installation, independently of the owner's RAM state slot.
        for dependency in &draft.read_dependencies {
            capture(world, *dependency, &mut before, &mut read_bytes)?;
        }
        if before.get(&actor).is_none_or(|o| o.kind != ObjKind::Player) {
            return Err(refused());
        }
        let moving: BTreeSet<_> = draft.moves.iter().map(|m| m.object).collect();
        if moving.len() != draft.moves.len() || moving.len() > MAX_PHYSICAL_MOVES {
            return Err(refused());
        }
        let mut changed = BTreeSet::new();
        let mut descendants = BTreeSet::new();
        for m in &draft.moves {
            capture_chain(world, m.object, &mut before, &mut read_bytes)?;
            capture_chain(world, m.destination, &mut before, &mut read_bytes)?;
            if let Some(source) = m.source {
                capture_chain(world, source, &mut before, &mut read_bytes)?;
            }
            let item = before.get(&m.object).ok_or_else(refused)?;
            let to = before.get(&m.destination).ok_or_else(refused)?;
            if !matches!(item.kind, ObjKind::Thing | ObjKind::Player | ObjKind::Npc)
                || !container(to)
                || item.location != m.source
                || m.source == Some(m.destination)
                || m.object == m.destination
                || to.contents.contains(&m.object)
                || to.exits.contains(&m.object)
            {
                return Err(refused());
            }
            if let Some(source) = m.source {
                let from = before.get(&source).ok_or_else(refused)?;
                if !container(from)
                    || from.contents.iter().filter(|id| **id == m.object).count() != 1
                    || from.exits.contains(&m.object)
                {
                    return Err(refused());
                }
            }
            changed.insert(m.object);
            changed.insert(m.destination);
            changed.extend(m.source);
            // The complete bounded subtree matters: an occupied hull can
            // deepen descendants that have no projection of their own.
            let mut todo = vec![m.object];
            let mut local = BTreeSet::new();
            while let Some(id) = todo.pop() {
                if !local.insert(id) || local.len() > MAX_PHYSICAL_MOVES {
                    return Err(refused());
                }
                descendants.insert(id);
                if descendants.len() > MAX_PHYSICAL_MOVES {
                    return Err(ParticipantError::ImageLimit);
                }
                capture(world, id, &mut before, &mut read_bytes)?;
                let object = before.get(&id).ok_or_else(refused)?;
                if object.contents.len()
                    > MAX_PHYSICAL_MOVES.saturating_sub(local.len() + todo.len())
                {
                    return Err(ParticipantError::ImageLimit);
                }
                let mut unique = BTreeSet::new();
                for child in &object.contents {
                    if !unique.insert(*child)
                        || object.exits.contains(child)
                        || world
                            .get(*child)
                            .is_none_or(|o| o.location != Some(id) || o.kind == ObjKind::Exit)
                    {
                        return Err(refused());
                    }
                    todo.push(*child);
                }
            }
        }
        if changed.len() > MAX_OBJECTS {
            return Err(ParticipantError::ImageLimit);
        }
        let mut after: BTreeMap<_, _> = changed
            .into_iter()
            .map(|id| {
                before
                    .get(&id)
                    .cloned()
                    .map(|row| (id, row))
                    .ok_or_else(refused)
            })
            .collect::<Result<_, _>>()?;
        // Remove then add, merging each row once even when a mover is also a
        // container. No intermediate state is ever published.
        for m in &draft.moves {
            if let Some(source) = m.source {
                after
                    .get_mut(&source)
                    .ok_or_else(refused)?
                    .contents
                    .retain(|id| *id != m.object);
            }
        }
        for m in &draft.moves {
            after
                .get_mut(&m.destination)
                .ok_or_else(refused)?
                .contents
                .push(m.object);
            after.get_mut(&m.object).ok_or_else(refused)?.location = Some(m.destination);
        }
        for object in after.values_mut() {
            object.modified = world.now();
        }
        // Validate cycles and descendant depth against the whole postgraph.
        for id in descendants.iter().chain(after.keys()) {
            let mut at = Some(*id);
            let mut seen = BTreeSet::new();
            while let Some(id) = at {
                if seen.len() >= crate::MAX_CONTAINMENT_DEPTH || !seen.insert(id) {
                    return Err(refused());
                }
                let object = after
                    .get(&id)
                    .or_else(|| before.get(&id))
                    .ok_or_else(refused)?;
                if let Some(parent) = object.location {
                    let holder = after
                        .get(&parent)
                        .or_else(|| before.get(&parent))
                        .ok_or_else(refused)?;
                    if holder.contents.iter().filter(|child| **child == id).count() != 1
                        || holder.exits.contains(&id)
                    {
                        return Err(refused());
                    }
                }
                at = object.location;
            }
        }
        let attempt = Arc::new(PhysicalTransitionAttempt {
            actor,
            root: draft.root,
            action: action.clone(),
            commit,
            moves: draft.moves.clone(),
        });
        let mut carry = Vec::new();
        for m in &draft.moves {
            carry.push(CarryCheck::capture_physical(
                Arc::clone(&hooks),
                world,
                lens,
                actor,
                m.object,
                m.destination,
                Arc::clone(&attempt),
                if before.get(&m.object).is_some_and(|o| o.kind.is_actor()) {
                    Arc::clone(&move_hooks)
                } else {
                    Arc::from([])
                },
            )?);
        }
        let writes: Vec<_> = after
            .values()
            .cloned()
            .map(|object| ReplayWrite::ObjectSet { object })
            .collect();
        canonical_json_bytes_with_limit(
            &serde_json::to_value(&writes).map_err(|_| ParticipantError::Encoding)?,
            IMAGE_BYTES,
        )
        .map_err(|_| ParticipantError::ImageLimit)?;
        let image = Self {
            before,
            after,
            carry,
            participant,
            action,
            commit,
            writes,
        };
        image.recheck(world, lens)?;
        Ok(image)
    }
    pub(crate) fn digests(&self) -> Result<Vec<PhysicalObjectDigest>, ParticipantError> {
        self.after
            .iter()
            .map(|(id, after)| {
                Ok(PhysicalObjectDigest {
                    object: *id,
                    before: object_digest(self.before.get(id).ok_or_else(refused)?)?,
                    after: object_digest(after)?,
                })
            })
            .collect()
    }
    pub(crate) fn writes(&self) -> &[ReplayWrite] {
        &self.writes
    }
    pub(crate) fn recheck(
        &self,
        world: &World,
        lens: crate::action_state::TypedStateLens<'_>,
    ) -> Result<(), ParticipantError> {
        self.local(world)?;
        for carry in &self.carry {
            carry.recheck(world, lens)?;
        }
        self.local(world)
    }
    pub(crate) fn local(&self, world: &World) -> Result<(), ParticipantError> {
        if self.before.values().any(|o| world.get(o.id) != Some(o)) {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn bind(
        self,
        storage: ShellObjectStorage<'_>,
    ) -> Result<BoundPhysicalTransitionObjects<'_>, ParticipantError> {
        let ShellObjectStorage { objects, .. } = storage;
        if self
            .before
            .values()
            .any(|o| objects.get(o.id.0 as usize).and_then(Option::as_ref) != Some(o))
        {
            return Err(refused());
        }
        Ok(BoundPhysicalTransitionObjects {
            objects,
            after: self.after,
        })
    }
}
fn container(object: &Obj) -> bool {
    matches!(
        object.kind,
        ObjKind::Room | ObjKind::Thing | ObjKind::Player | ObjKind::Npc
    )
}
fn capture(
    world: &World,
    id: ObjId,
    before: &mut BTreeMap<ObjId, Obj>,
    read_bytes: &mut usize,
) -> Result<(), ParticipantError> {
    if before.contains_key(&id) {
        return Ok(());
    }
    if before.len() >= MAX_OBJECTS * crate::MAX_CONTAINMENT_DEPTH {
        return Err(ParticipantError::ImageLimit);
    }
    let object = world.get(id).ok_or_else(refused)?;
    if object.flags.contains(Flags::GOING) {
        return Err(refused());
    }
    // Stream into a bounded counter before cloning any contents or attributes.
    // The aggregate includes ample map-key/separator overhead per row.
    let remaining = IMAGE_BYTES
        .checked_sub(*read_bytes)
        .and_then(|n| n.checked_sub(16))
        .ok_or(ParticipantError::ImageLimit)?;
    let mut size = ObjectSize {
        bytes: 0,
        remaining,
    };
    serde_json::to_writer(&mut size, object).map_err(|_| ParticipantError::ImageLimit)?;
    *read_bytes += size.bytes + 16;
    before.insert(id, object.clone());
    Ok(())
}
struct ObjectSize {
    bytes: usize,
    remaining: usize,
}
impl std::io::Write for ObjectSize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(std::io::Error::other("physical image limit"));
        }
        self.remaining -= bytes.len();
        self.bytes += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn capture_chain(
    world: &World,
    root: ObjId,
    before: &mut BTreeMap<ObjId, Obj>,
    read_bytes: &mut usize,
) -> Result<(), ParticipantError> {
    let mut at = Some(root);
    let mut seen = BTreeSet::new();
    while let Some(id) = at {
        if seen.len() >= crate::MAX_CONTAINMENT_DEPTH || !seen.insert(id) {
            return Err(refused());
        }
        capture(world, id, before, read_bytes)?;
        let object = world.get(id).ok_or_else(refused)?;
        if let Some(parent) = object.location {
            // Bound the parent before scanning its inverse lists, rather than
            // waiting for the next ancestor iteration to charge that row.
            capture(world, parent, before, read_bytes)?;
            let holder = world.get(parent).ok_or_else(refused)?;
            if !container(holder)
                || holder.contents.iter().filter(|child| **child == id).count() != 1
                || holder.exits.contains(&id)
            {
                return Err(refused());
            }
        }
        at = object.location;
    }
    Ok(())
}
fn object_digest(object: &Obj) -> Result<DefinitionDigest, ParticipantError> {
    DefinitionDigest::for_canonical_value_with_limit(
        "core.physical_transition.object.v1",
        &serde_json::to_value(object).map_err(|_| ParticipantError::Encoding)?,
        IMAGE_BYTES,
    )
    .map_err(|_| ParticipantError::ImageLimit)
}
pub(crate) struct BoundPhysicalTransitionObjects<'a> {
    objects: &'a mut Vec<Option<Obj>>,
    after: BTreeMap<ObjId, Obj>,
}
impl BoundPhysicalTransitionObjects<'_> {
    pub(crate) fn apply(self) {
        for (id, object) in self.after {
            self.objects[id.0 as usize] = Some(object);
        }
    }
}
