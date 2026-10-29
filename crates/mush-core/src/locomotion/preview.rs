//! Bounded Core object previews rooted in a genuine locomotion admission.
//!
//! A projection supplied here is hypothetical data, not Sim authority. These
//! previews cannot install images, deserialize caller rows, complete observers,
//! or alter the existing v1 movement protocols. They supply the intermediate
//! object state a future joint placement protocol must read without cloning
//! World or treating the original admitted room as the final projection.

use std::collections::{BTreeMap, BTreeSet};

use crate::object_read::{ObjectRead, ReadLimit, ReadTrace, TrackedRead};
use crate::participant::ParticipantError;
use crate::{Obj, ObjId, ReplayWrite, World, canonical_json_bytes_with_limit};

use super::LocomotionAdmission;
use super::image::{
    IMAGE_BYTES, PreparedRows, chain, charge_inverse, container, existing, limit, post_chain,
    prepare_rows, refused, subtree,
};

/// Owned, bounded observations. Original admission and image preimages are
/// retained independently; each keeps its existing read census/byte limits.
/// There is no conversion from this public type to an installation capability.
pub struct LocomotionPreview {
    actor: ObjId,
    source: Option<ObjId>,
    admitted_room: ObjId,
    projected_room: Option<ObjId>,
    at_utc: i64,
    admission_trace: ReadTrace,
    rows: PreparedRows,
}

impl std::fmt::Debug for LocomotionPreview {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocomotionPreview")
            .field("actor", &self.actor)
            .field("admitted_room", &self.admitted_room)
            .field("projected_room", &self.projected_room)
            .field("touched_objects", &self.rows.after.keys())
            .finish_non_exhaustive()
    }
}

impl LocomotionPreview {
    pub(crate) fn prepare(
        world: &World,
        admission: &LocomotionAdmission,
        dependencies: &BTreeSet<ObjId>,
        projected_room: Option<ObjId>,
    ) -> Result<Self, ParticipantError> {
        let at_utc = world.now();
        // The ordinary path shares this exact row builder, with no projection.
        let rows = prepare_rows(world, admission, dependencies)?;
        let rows = match projected_room {
            Some(destination) => rows.project(
                world,
                admission.actor(),
                admission.destination(),
                destination,
                at_utc,
            )?,
            None => rows,
        };
        Ok(Self {
            actor: admission.actor(),
            source: admission.source(),
            admitted_room: admission.destination(),
            projected_room,
            at_utc,
            admission_trace: admission.trace.clone(),
            rows,
        })
    }
    #[must_use]
    pub const fn actor(&self) -> ObjId {
        self.actor
    }
    #[must_use]
    pub const fn source(&self) -> Option<ObjId> {
        self.source
    }
    #[must_use]
    pub const fn admitted_room(&self) -> ObjId {
        self.admitted_room
    }
    #[must_use]
    pub const fn destination(&self) -> ObjId {
        match self.projected_room {
            Some(room) => room,
            None => self.admitted_room,
        }
    }
    #[must_use]
    pub const fn at_utc(&self) -> i64 {
        self.at_utc
    }
    /// Only captured rows are readable. Unknown scope is an error, distinct
    /// from an absence retained by the original admission's tracked reader.
    pub fn object(&self, id: ObjId) -> Result<Option<&Obj>, ParticipantError> {
        if let Some(object) = self.rows.after.get(&id) {
            return Ok(Some(object));
        }
        self.rows
            .trace
            .observed(id)
            .or_else(|| self.admission_trace.observed(id))
            .ok_or_else(refused)
    }
    /// Includes genuinely touched intermediate rooms even if their final value
    /// happens to equal its original bytes at this UTC. It is not a write list.
    pub fn touched_objects(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.rows.after.keys().copied()
    }
    /// Rechecks original Core object facts only. Actual dispatch, typed-state,
    /// clocks and owner hooks still belong to the retained preparation protocol.
    pub fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        if !self.admission_trace.matches(world) || !self.rows.trace.matches(world) {
            return Err(refused());
        }
        Ok(())
    }
}

/// Semantic reads see the bounded first move's postimages while every read is
/// still charged to a real original World row. Nothing substitutes a World.
struct OverlayRead<'a, 'w> {
    reader: &'a mut TrackedRead<'w>,
    after: &'a BTreeMap<ObjId, Obj>,
}
impl<'a, 'w: 'a> ObjectRead<'a> for OverlayRead<'a, 'w> {
    type Error = ReadLimit;
    fn object(&mut self, id: ObjId) -> Result<Option<&'a Obj>, ReadLimit> {
        let original = self.reader.object(id)?;
        Ok(self.after.get(&id).or(original))
    }
    fn charge_work(&mut self, units: usize) -> Result<(), ReadLimit> {
        self.reader.charge_work(units)
    }
    fn object_slots(&mut self) -> Result<usize, ReadLimit> {
        self.reader.object_slots()
    }
}

impl PreparedRows {
    pub(super) fn project(
        mut self,
        world: &World,
        actor: ObjId,
        current: ObjId,
        destination: ObjId,
        at_utc: i64,
    ) -> Result<Self, ParticipantError> {
        if !self.trace.matches(world) || actor == destination {
            return Err(refused());
        }
        let mut reader = TrackedRead::new(world);
        // One aggregate image-read budget covers both steps, not a fresh
        // additional budget for each projection. Absence stays observable.
        for id in self.trace.observed_ids() {
            reader.object(id).map_err(limit)?;
        }
        {
            let mut overlay = OverlayRead {
                reader: &mut reader,
                after: &self.after,
            };
            let object = existing(&mut overlay, actor)?;
            if !object.kind.is_actor() || object.location != Some(current) {
                return Err(refused());
            }
            chain(&mut overlay, actor)?;
            chain(&mut overlay, destination)?;
            let target = existing(&mut overlay, destination)?;
            charge_inverse(&mut overlay, target)?;
            if !container(target)
                || target.exits.contains(&actor)
                || target.contents.iter().filter(|id| **id == actor).count()
                    != usize::from(current == destination)
            {
                return Err(refused());
            }
            let holder = existing(&mut overlay, current)?;
            charge_inverse(&mut overlay, holder)?;
            if !container(holder)
                || holder.exits.contains(&actor)
                || holder.contents.iter().filter(|id| **id == actor).count() != 1
            {
                return Err(refused());
            }
            for descendant in subtree(&mut overlay, actor)? {
                post_chain(&mut overlay, descendant, actor, destination)?;
            }
        }
        if current != destination {
            let touched = BTreeSet::from([actor, current, destination]);
            for id in &touched {
                let original = existing(&mut reader, *id)?;
                self.before.entry(*id).or_insert_with(|| original.clone());
                self.after.entry(*id).or_insert_with(|| original.clone());
            }
            self.after
                .get_mut(&current)
                .ok_or_else(refused)?
                .contents
                .retain(|id| *id != actor);
            self.after
                .get_mut(&destination)
                .ok_or_else(refused)?
                .contents
                .push(actor);
            self.after.get_mut(&actor).ok_or_else(refused)?.location = Some(destination);
            for id in touched {
                self.after.get_mut(&id).ok_or_else(refused)?.modified = at_utc;
            }
        }
        self.writes = self
            .after
            .values()
            .cloned()
            .map(|object| ReplayWrite::ObjectSet { object })
            .collect();
        canonical_json_bytes_with_limit(
            &serde_json::to_value(&self.writes).map_err(|_| ParticipantError::Encoding)?,
            IMAGE_BYTES,
        )
        .map_err(|_| ParticipantError::ImageLimit)?;
        self.trace = reader.finish().map_err(limit)?;
        Ok(self)
    }
}
