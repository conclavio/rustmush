//! Closed ordinary locomotion images. The retained admission alone supplies
//! the actor and route; Sim supplies read dependencies, never replacement moves
//! or object postimages. Ordinary MoveAttempt and command fences remain in the
//! retained dispatch observation. This module invokes no hooks or exemptions.

use std::collections::{BTreeMap, BTreeSet};

use crate::authoritative::ReplayWrite;
use crate::mechanics::{DefinitionDigest, DefinitionRef, canonical_json_bytes_with_limit};
use crate::object_read::{ObjectRead, ReadTrace, TrackedRead};
use crate::participant::ParticipantError;
use crate::reserved_shell::ShellObjectStorage;
use crate::{CommitToken, Flags, Obj, ObjId, ObjKind, World};

use super::LocomotionAdmission;
use super::transition::LocomotionObjectDigest;

pub(crate) const MAX_LOCOMOTION_DEPENDENCIES: usize = 512;
pub(crate) const MAX_LOCOMOTION_SUBTREE: usize = 256;
pub(crate) const IMAGE_BYTES: usize = 1_048_576;

pub(crate) struct LocomotionObjects {
    trace: ReadTrace,
    before: BTreeMap<ObjId, Obj>,
    after: BTreeMap<ObjId, Obj>,
    writes: Vec<ReplayWrite>,
    pub(crate) participant: DefinitionRef,
    pub(crate) scenes_participant: Option<DefinitionRef>,
    pub(crate) composed: Option<super::composed::ComposedMovementPermit>,
    pub(crate) action: DefinitionRef,
    pub(crate) commit: CommitToken,
}

impl std::fmt::Debug for LocomotionObjects {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocomotionObjects")
            .field("objects", &self.after.keys())
            .finish_non_exhaustive()
    }
}

impl LocomotionObjects {
    pub(crate) fn prepare(
        world: &World,
        admission: &LocomotionAdmission,
        dependencies: &BTreeSet<ObjId>,
        participant: DefinitionRef,
        action: DefinitionRef,
        commit: CommitToken,
    ) -> Result<Self, ParticipantError> {
        if admission.binding.action != action || admission.binding.commit != commit {
            return Err(ParticipantError::Authority);
        }
        let PreparedRows {
            trace,
            before,
            after,
            writes,
        } = prepare_rows(world, admission, dependencies)?;
        let image = Self {
            trace,
            before,
            after,
            writes,
            participant,
            scenes_participant: None,
            composed: None,
            action,
            commit,
        };
        image.local(world)?;
        Ok(image)
    }

    pub(crate) fn writes(&self) -> &[ReplayWrite] {
        &self.writes
    }

    pub(crate) fn digests(&self) -> Result<Vec<LocomotionObjectDigest>, ParticipantError> {
        self.after
            .iter()
            .map(|(id, after)| {
                Ok(LocomotionObjectDigest {
                    object: *id,
                    before: object_digest(self.before.get(id).ok_or_else(refused)?)?,
                    after: object_digest(after)?,
                })
            })
            .collect()
    }

    pub(crate) fn local(&self, world: &World) -> Result<(), ParticipantError> {
        if let Some(composed) = &self.composed {
            composed.check(&self.action)?;
        }
        if !self.trace.matches(world) {
            return Err(refused());
        }
        Ok(())
    }

    pub(crate) fn bind(
        self,
        storage: ShellObjectStorage<'_>,
    ) -> Result<BoundLocomotionObjects<'_>, ParticipantError> {
        let ShellObjectStorage { objects, .. } = storage;
        if let Some(composed) = &self.composed {
            composed.check(&self.action)?;
        }
        if !self.trace.matches_objects(objects) {
            return Err(refused());
        }
        Ok(BoundLocomotionObjects {
            objects,
            after: self.after,
        })
    }
}

pub(super) struct PreparedRows {
    pub(super) trace: ReadTrace,
    pub(super) before: BTreeMap<ObjId, Obj>,
    pub(super) after: BTreeMap<ObjId, Obj>,
    pub(super) writes: Vec<ReplayWrite>,
}

pub(super) fn prepare_rows(
    world: &World,
    admission: &LocomotionAdmission,
    dependencies: &BTreeSet<ObjId>,
) -> Result<PreparedRows, ParticipantError> {
    if super::authenticated_actor(&admission.binding.invocation)? != admission.actor()
        || !admission.trace.matches(world)
    {
        return Err(ParticipantError::Authority);
    }
    if dependencies.len() > MAX_LOCOMOTION_DEPENDENCIES {
        return Err(ParticipantError::ImageLimit);
    }
    let actor = admission.actor();
    let source = admission.source();
    let destination = admission.destination();
    let mut reader = TrackedRead::new(world);
    for dependency in dependencies {
        existing(&mut reader, *dependency)?;
    }
    let object = existing(&mut reader, actor)?;
    if !object.kind.is_actor() || object.location != source || actor == destination {
        return Err(refused());
    }
    chain(&mut reader, actor)?;
    chain(&mut reader, destination)?;
    let target = existing(&mut reader, destination)?;
    charge_inverse(&mut reader, target)?;
    if !container(target) || target.exits.contains(&actor) {
        return Err(refused());
    }
    let target_count = target.contents.iter().filter(|id| **id == actor).count();
    if target_count != usize::from(source == Some(destination)) {
        return Err(refused());
    }
    if let Some(source) = source {
        let holder = existing(&mut reader, source)?;
        charge_inverse(&mut reader, holder)?;
        if !container(holder)
            || holder.contents.iter().filter(|id| **id == actor).count() != 1
            || holder.exits.contains(&actor)
        {
            return Err(refused());
        }
    }
    // A carried container can contain further objects or people. Their
    // inverse edges and resulting depth must be proved before publication.
    let descendants = subtree(&mut reader, actor)?;
    for descendant in descendants {
        post_chain(&mut reader, descendant, actor, destination)?;
    }
    let mut before = BTreeMap::new();
    let mut after = BTreeMap::new();
    if source != Some(destination) {
        let changed: BTreeSet<_> = [Some(actor), source, Some(destination)]
            .into_iter()
            .flatten()
            .collect();
        for id in changed {
            // existing() streams and bounds the entire row before this
            // recursive clone. Only these at most three rows are written.
            let object = existing(&mut reader, id)?.clone();
            before.insert(id, object.clone());
            after.insert(id, object);
        }
        if let Some(source) = source {
            after
                .get_mut(&source)
                .ok_or_else(refused)?
                .contents
                .retain(|id| *id != actor);
        }
        after
            .get_mut(&destination)
            .ok_or_else(refused)?
            .contents
            .push(actor);
        after.get_mut(&actor).ok_or_else(refused)?.location = Some(destination);
        for object in after.values_mut() {
            object.modified = world.now();
        }
    }
    // World::move_to returns early for an exit back to the same location.
    // Keep that legitimate no-op byte-for-byte: no reordered contents or
    // modified timestamps, while the Sim audit remains a real owner write.
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
    Ok(PreparedRows {
        trace: reader.finish().map_err(limit)?,
        before,
        after,
        writes,
    })
}

pub(super) fn refused() -> ParticipantError {
    ParticipantError::refused("ordinary locomotion object image refused")
}

pub(super) fn limit(_: crate::object_read::ReadLimit) -> ParticipantError {
    ParticipantError::ImageLimit
}

pub(super) fn container(object: &Obj) -> bool {
    matches!(
        object.kind,
        ObjKind::Room | ObjKind::Thing | ObjKind::Player | ObjKind::Npc
    )
}

pub(super) fn existing<'w, R: ObjectRead<'w, Error = crate::object_read::ReadLimit>>(
    reader: &mut R,
    id: ObjId,
) -> Result<&'w Obj, ParticipantError> {
    reader
        .object(id)
        .map_err(limit)?
        .filter(|object| object.id == id && !object.flags.contains(Flags::GOING))
        .ok_or_else(refused)
}

pub(super) fn charge_inverse<'w, R: ObjectRead<'w, Error = crate::object_read::ReadLimit>>(
    reader: &mut R,
    holder: &Obj,
) -> Result<(), ParticipantError> {
    reader
        .charge_work(holder.contents.len().saturating_add(holder.exits.len()))
        .map_err(limit)
}

pub(super) fn chain<'w, R: ObjectRead<'w, Error = crate::object_read::ReadLimit>>(
    reader: &mut R,
    root: ObjId,
) -> Result<(), ParticipantError> {
    let mut next = Some(root);
    let mut seen = BTreeSet::new();
    while let Some(id) = next {
        if seen.len() >= crate::MAX_CONTAINMENT_DEPTH || !seen.insert(id) {
            return Err(refused());
        }
        let object = existing(reader, id)?;
        if let Some(parent) = object.location {
            let holder = existing(reader, parent)?;
            charge_inverse(reader, holder)?;
            if !container(holder)
                || holder.contents.iter().filter(|child| **child == id).count() != 1
                || holder.exits.contains(&id)
            {
                return Err(refused());
            }
        }
        next = object.location;
    }
    Ok(())
}

pub(super) fn subtree<'w, R: ObjectRead<'w, Error = crate::object_read::ReadLimit>>(
    reader: &mut R,
    actor: ObjId,
) -> Result<BTreeSet<ObjId>, ParticipantError> {
    let mut pending = vec![actor];
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if seen.len() >= MAX_LOCOMOTION_SUBTREE || !seen.insert(id) {
            return Err(refused());
        }
        let object = existing(reader, id)?;
        if object.contents.len() > MAX_LOCOMOTION_SUBTREE.saturating_sub(seen.len() + pending.len())
        {
            return Err(ParticipantError::ImageLimit);
        }
        let mut unique = BTreeSet::new();
        reader.charge_work(object.contents.len()).map_err(limit)?;
        for child in &object.contents {
            reader.charge_work(object.exits.len()).map_err(limit)?;
            if !unique.insert(*child) || object.exits.contains(child) {
                return Err(refused());
            }
            let child_object = existing(reader, *child)?;
            if child_object.location != Some(id) || !container(child_object) {
                return Err(refused());
            }
            pending.push(*child);
        }
    }
    Ok(seen)
}

pub(super) fn post_chain<'w, R: ObjectRead<'w, Error = crate::object_read::ReadLimit>>(
    reader: &mut R,
    root: ObjId,
    actor: ObjId,
    destination: ObjId,
) -> Result<(), ParticipantError> {
    let mut next = Some(root);
    let mut seen = BTreeSet::new();
    while let Some(id) = next {
        if seen.len() >= crate::MAX_CONTAINMENT_DEPTH || !seen.insert(id) {
            return Err(refused());
        }
        let object = existing(reader, id)?;
        next = if id == actor {
            Some(destination)
        } else {
            object.location
        };
    }
    Ok(())
}

fn object_digest(object: &Obj) -> Result<DefinitionDigest, ParticipantError> {
    DefinitionDigest::for_canonical_value_with_limit(
        "core.locomotion.object.v1",
        &serde_json::to_value(object).map_err(|_| ParticipantError::Encoding)?,
        IMAGE_BYTES,
    )
    .map_err(|_| ParticipantError::ImageLimit)
}

pub(crate) struct BoundLocomotionObjects<'a> {
    objects: &'a mut Vec<Option<Obj>>,
    after: BTreeMap<ObjId, Obj>,
}

impl BoundLocomotionObjects<'_> {
    pub(crate) fn apply(self) {
        for (id, object) in self.after {
            // Every occupied index was verified under this exclusive borrow.
            self.objects[id.0 as usize] = Some(object);
        }
    }
}

#[cfg(test)]
mod tests;
