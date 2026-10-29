//! Bounded facts for one prepared physical transition, never continuous aboard evidence.
//!
//! The caller has already admitted the canonical plan and checked the resident
//! spatial size limits. This module overlays only that plan's exact projections;
//! it neither changes World nor treats a placement as a substitute for presence.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mush_core::participant::ParticipantError;
use mush_core::{ClockRef, ClockStamp, DefinitionDigest, Flags, ObjId, ObjKind, World};
use serde::Serialize;

use crate::data::{InteriorSpaceRecord, PlacementRecord, SpaceAnchor, SpatialParent, SpatialState};
use crate::personnel::{Crossing, CrossingDirection};
use crate::transition::{Plan, Transition};

pub(crate) const MAX_ACTORS: usize = 256;
pub(crate) const MAX_CROSSINGS: usize = 256;
pub(crate) const MAX_DEPENDENCIES: usize = 512;
const MAX_STEPS: usize = 8_192;
const MAX_FACT_BYTES: usize = 128 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaptureFacts {
    pub(crate) prepared_at: ClockStamp,
    pub(crate) before_ancestry_digest: DefinitionDigest,
    pub(crate) after_ancestry_digest: DefinitionDigest,
    pub(crate) capture_digest: DefinitionDigest,
    pub(crate) crossings: Vec<Crossing>,
    pub(crate) read_dependencies: BTreeSet<ObjId>,
}

fn refused(message: &'static str) -> ParticipantError {
    ParticipantError::Refused(message.into())
}

struct Reads<'a> {
    world: &'a World,
    dependencies: BTreeSet<ObjId>,
    contents: BTreeMap<ObjId, BTreeSet<ObjId>>,
    steps: usize,
}
impl<'a> Reads<'a> {
    fn step(&mut self) -> Result<(), ParticipantError> {
        self.steps = self
            .steps
            .checked_add(1)
            .ok_or_else(|| refused("personnel traversal limit"))?;
        if self.steps > MAX_STEPS {
            return Err(refused("personnel traversal limit"));
        }
        Ok(())
    }

    fn touch(&mut self, object: ObjId) -> Result<(), ParticipantError> {
        self.step()?;
        if self
            .world
            .get(object)
            .is_none_or(|o| o.flags.contains(Flags::GOING))
        {
            return Err(refused("personnel dependency is unavailable"));
        }
        self.dependencies.insert(object);
        if self.dependencies.len() > MAX_DEPENDENCIES {
            return Err(refused("personnel dependency limit"));
        }
        Ok(())
    }

    fn contents(&mut self, holder: ObjId) -> Result<&BTreeSet<ObjId>, ParticipantError> {
        if !self.contents.contains_key(&holder) {
            self.touch(holder)?;
            let mut ids = BTreeSet::new();
            let world = self.world;
            let source = &world
                .get(holder)
                .ok_or_else(|| refused("missing holder"))?
                .contents;
            for &child in source {
                self.step()?;
                if !ids.insert(child) {
                    return Err(refused("invalid personnel contents"));
                }
            }
            self.contents.insert(holder, ids);
        }
        self.contents
            .get(&holder)
            .ok_or_else(|| refused("missing contents"))
    }

    fn inverse(&mut self, object: ObjId) -> Result<(), ParticipantError> {
        self.touch(object)?;
        if let Some(holder) = self.world.get(object).and_then(|o| o.location)
            && !self.contents(holder)?.contains(&object)
        {
            return Err(refused("personnel location and inverse disagree"));
        }
        Ok(())
    }
}

struct View<'a> {
    state: &'a SpatialState,
    placements: BTreeMap<ObjId, &'a PlacementRecord>,
    spaces: BTreeMap<ObjId, &'a InteriorSpaceRecord>,
    projections: BTreeMap<ObjId, ObjId>,
}
enum Parent {
    Interior(ObjId),
    Attached(ObjId, crate::data::MountId),
    Direct,
    Unplaced,
}
impl<'a> View<'a> {
    fn before(state: &'a SpatialState) -> Self {
        Self {
            state,
            placements: BTreeMap::new(),
            spaces: BTreeMap::new(),
            projections: BTreeMap::new(),
        }
    }

    fn after(state: &'a SpatialState, plan: &'a Plan) -> Result<Self, ParticipantError> {
        if !plan.retire.is_empty()
            || plan.set_placements.len() + plan.set_spaces.len() > MAX_DEPENDENCIES
            || plan.projections.len() > MAX_DEPENDENCIES
        {
            return Err(refused("unsupported personnel plan"));
        }
        let mut view = Self::before(state);
        for placement in &plan.set_placements {
            if view
                .placements
                .insert(placement.object, placement)
                .is_some()
            {
                return Err(refused("duplicate personnel placement"));
            }
        }
        for space in &plan.set_spaces {
            if view.spaces.insert(space.object, space).is_some() {
                return Err(refused("duplicate personnel space"));
            }
        }
        for &(object, destination) in &plan.projections {
            if view.projections.insert(object, destination).is_some() {
                return Err(refused("duplicate personnel projection"));
            }
        }
        Ok(view)
    }

    fn parent(&self, object: ObjId) -> Parent {
        match self
            .placements
            .get(&object)
            .copied()
            .or_else(|| self.state.placements.get(&object))
            .map(|p| &p.parent)
        {
            Some(SpatialParent::Interior { space, .. }) => Parent::Interior(*space),
            Some(SpatialParent::Attached { host, mount, .. }) => {
                Parent::Attached(*host, mount.clone())
            }
            Some(SpatialParent::Direct { .. }) => Parent::Direct,
            Some(SpatialParent::Unplaced) | None => Parent::Unplaced,
        }
    }

    fn space(&self, object: ObjId) -> Option<&InteriorSpaceRecord> {
        self.spaces
            .get(&object)
            .copied()
            .or_else(|| self.state.spaces.get(&object))
    }

    fn location(
        &self,
        reads: &mut Reads<'_>,
        object: ObjId,
    ) -> Result<Option<ObjId>, ParticipantError> {
        reads.inverse(object)?;
        let location = self
            .projections
            .get(&object)
            .copied()
            .or_else(|| reads.world.get(object).and_then(|o| o.location));
        if let Some(location) = location {
            reads.touch(location)?;
        }
        Ok(location)
    }

    fn room_space(
        &self,
        reads: &mut Reads<'_>,
        room: ObjId,
    ) -> Result<Option<ObjId>, ParticipantError> {
        reads.touch(room)?;
        let owns =
            |space: &InteriorSpaceRecord| space.object == room || space.rooms.contains(&room);
        let mut found = None;
        if let Some(&space) = self.state.derived.member_of.get(&room) {
            reads.touch(space)?;
            let record = self
                .space(space)
                .ok_or_else(|| refused("missing indexed interior"))?;
            if owns(record) {
                found = Some(space);
            } else if !self.spaces.contains_key(&space) {
                return Err(refused("interior membership index disagrees"));
            }
        }
        if self.space(room).is_some() {
            if found.is_some_and(|id| id != room) {
                return Err(refused("ambiguous interior root"));
            }
            found = Some(room);
        }
        for (&space, record) in &self.spaces {
            reads.step()?;
            if owns(record) {
                reads.touch(space)?;
                if found.is_some_and(|id| id != space) {
                    return Err(refused("ambiguous interior membership"));
                }
                found = Some(space);
            }
        }
        Ok(found)
    }

    fn anchored_spaces(
        &self,
        reads: &mut Reads<'_>,
        host: ObjId,
    ) -> Result<BTreeSet<ObjId>, ParticipantError> {
        let mut found = BTreeSet::new();
        if let Some(spaces) = self.state.derived.anchored_spaces.get(&host) {
            for &space in spaces {
                reads.step()?;
                let record = self
                    .space(space)
                    .ok_or_else(|| refused("missing carrier interior"))?;
                if matches!(record.anchor, SpaceAnchor::MovingObject { host: h } if h == host) {
                    reads.touch(space)?;
                    found.insert(space);
                } else if !self.spaces.contains_key(&space) {
                    return Err(refused("carrier index disagrees"));
                }
            }
        }
        for (&space, record) in &self.spaces {
            reads.step()?;
            if matches!(record.anchor, SpaceAnchor::MovingObject { host: h } if h == host) {
                reads.touch(space)?;
                found.insert(space);
            }
        }
        Ok(found)
    }

    fn carrier(&self, reads: &mut Reads<'_>, host: ObjId) -> Result<bool, ParticipantError> {
        Ok(!self.anchored_spaces(reads, host)?.is_empty())
    }

    fn physical_path(
        &self,
        reads: &mut Reads<'_>,
        object: ObjId,
    ) -> Result<Vec<ObjId>, ParticipantError> {
        let mut path = Vec::new();
        let mut seen = BTreeSet::new();
        let mut current = Some(object);
        while let Some(node) = current {
            if !seen.insert(node) || path.len() > mush_core::MAX_CONTAINMENT_DEPTH {
                return Err(refused("invalid personnel containment"));
            }
            reads.touch(node)?;
            path.push(node);
            if reads
                .world
                .get(node)
                .is_some_and(|o| o.kind == ObjKind::Room)
            {
                break;
            }
            current = self.location(reads, node)?;
        }
        Ok(path)
    }

    fn physical_start(
        &self,
        reads: &mut Reads<'_>,
        path: &[ObjId],
    ) -> Result<Option<Node>, ParticipantError> {
        // Skip the character/object itself; the nearest enclosing carrier or
        // actual authored room supplies implicit membership for unplaced people.
        for &node in path.iter().skip(1) {
            if reads
                .world
                .get(node)
                .is_some_and(|o| o.kind == ObjKind::Room)
            {
                return self
                    .room_space(reads, node)
                    .map(|space| space.map(Node::Space));
            }
            if self.carrier(reads, node)? {
                return Ok(Some(Node::Object(node)));
            }
        }
        Ok(None)
    }

    fn interior_matches(
        &self,
        reads: &mut Reads<'_>,
        path: &[ObjId],
        space: ObjId,
    ) -> Result<(), ParticipantError> {
        reads.touch(space)?;
        // An explicit Interior projection is the member room itself. Only an
        // unplaced object may derive implicit membership through containers.
        if path.len() != 2 {
            return Err(refused("personnel interior projection is indirect"));
        }
        let room = path.last().copied().filter(|room| {
            reads
                .world
                .get(*room)
                .is_some_and(|o| o.kind == ObjKind::Room)
        });
        if room
            .map(|room| self.room_space(reads, room))
            .transpose()?
            .flatten()
            != Some(space)
        {
            return Err(refused(
                "personnel placement disagrees with actual interior",
            ));
        }
        Ok(())
    }

    fn attachment(
        &self,
        reads: &mut Reads<'_>,
        object: ObjId,
        host: ObjId,
        mount: &crate::data::MountId,
    ) -> Result<(), ParticipantError> {
        reads.touch(host)?;
        let exposed = self
            .state
            .mounts
            .get(&host)
            .and_then(|m| m.mounts.get(mount))
            .ok_or_else(|| refused("personnel attachment mount is missing"))?
            .exposed;
        let expected = if exposed {
            self.location(reads, host)?
        } else {
            Some(host)
        };
        if expected.is_none() || self.location(reads, object)? != expected {
            return Err(refused("personnel attachment projection disagrees"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Node {
    Space(ObjId),
    Object(ObjId),
}
impl Node {
    fn id(self) -> ObjId {
        match self {
            Self::Space(id) | Self::Object(id) => id,
        }
    }
}

#[derive(Serialize)]
struct Ancestry {
    character: ObjId,
    physical: Vec<ObjId>,
    structural: Vec<ObjId>,
    carriers: BTreeSet<ObjId>,
}

fn ancestry(
    view: &View<'_>,
    reads: &mut Reads<'_>,
    character: ObjId,
) -> Result<Ancestry, ParticipantError> {
    let physical = view.physical_path(reads, character)?;
    let mut next = match view.parent(character) {
        Parent::Interior(space) => {
            view.interior_matches(reads, &physical, space)?;
            Some(Node::Space(space))
        }
        Parent::Attached(host, mount) => {
            view.attachment(reads, character, host, &mount)?;
            Some(Node::Object(host))
        }
        Parent::Direct => {
            if view.physical_start(reads, &physical)?.is_some() {
                return Err(refused("direct personnel placement lies inside a carrier"));
            }
            None
        }
        Parent::Unplaced => view.physical_start(reads, &physical)?,
    };
    let mut structural = Vec::new();
    let mut carriers = BTreeSet::new();
    let mut seen = BTreeSet::from([character]);
    while let Some(node) = next {
        let id = node.id();
        if structural.len() >= crate::data::MAX_SPATIAL_DEPTH || !seen.insert(id) {
            return Err(refused("invalid personnel carrier ancestry"));
        }
        reads.touch(id)?;
        structural.push(id);
        next = match node {
            Node::Space(space) => {
                let record = view
                    .space(space)
                    .ok_or_else(|| refused("personnel interior is missing"))?;
                match record.anchor {
                    SpaceAnchor::MovingObject { host } => {
                        reads.touch(host)?;
                        carriers.insert(host);
                        Some(Node::Object(host))
                    }
                    SpaceAnchor::FixedDomain { .. } | SpaceAnchor::Unanchored => None,
                }
            }
            Node::Object(object) => {
                if view.carrier(reads, object)? {
                    carriers.insert(object);
                }
                match view.parent(object) {
                    Parent::Interior(space) => {
                        let path = view.physical_path(reads, object)?;
                        view.interior_matches(reads, &path, space)?;
                        Some(Node::Space(space))
                    }
                    Parent::Attached(host, mount) => {
                        view.attachment(reads, object, host, &mount)?;
                        Some(Node::Object(host))
                    }
                    Parent::Direct => {
                        let path = view.physical_path(reads, object)?;
                        if view.physical_start(reads, &path)?.is_some() {
                            return Err(refused("direct carrier placement lies inside a carrier"));
                        }
                        None
                    }
                    Parent::Unplaced => {
                        let path = view.physical_path(reads, object)?;
                        view.physical_start(reads, &path)?
                    }
                }
            }
        };
    }
    Ok(Ancestry {
        character,
        physical,
        structural,
        carriers,
    })
}

fn seeds(
    state: &SpatialState,
    transition: &Transition,
    plan: &Plan,
    baseline: &BTreeSet<ObjId>,
    reads: &mut Reads<'_>,
) -> Result<BTreeSet<ObjId>, ParticipantError> {
    let root = match transition {
        Transition::Place { obj, .. } | Transition::Unplace { obj, .. } => *obj,
        Transition::Space { space, .. } | Transition::Members { space, .. } => *space,
        _ => return Err(refused("unsupported personnel transition")),
    };
    let mut roots = BTreeSet::from([root]);
    for &node in baseline {
        reads.touch(node)?;
        if reads.world.get(node).is_some_and(|o| o.kind.is_actor()) {
            roots.insert(node);
        }
        if let Some(space) = state.spaces.get(&node) {
            roots.insert(space.object);
            for &room in &space.rooms {
                reads.step()?;
                roots.insert(room);
            }
        }
    }
    for space in &plan.set_spaces {
        roots.insert(space.object);
        for &room in &space.rooms {
            reads.step()?;
            roots.insert(room);
        }
        // Adding/removing a host's first/last interior changes whether people
        // physically contained by that host are aboard a carrier. Those hosts
        // need a census even when they have no structural placement children.
        for record in [state.spaces.get(&space.object), Some(space)]
            .into_iter()
            .flatten()
        {
            if let SpaceAnchor::MovingObject { host } = record.anchor {
                roots.insert(host);
            }
        }
    }
    roots.extend(plan.projections.iter().map(|&(object, _)| object));
    if roots.len() > MAX_DEPENDENCIES {
        return Err(refused("personnel root limit"));
    }
    Ok(roots)
}

fn actors(
    reads: &mut Reads<'_>,
    roots: BTreeSet<ObjId>,
    before: &View<'_>,
    after: &View<'_>,
) -> Result<BTreeSet<ObjId>, ParticipantError> {
    let mut pending: VecDeque<_> = roots.iter().copied().collect();
    let mut scheduled = roots;
    let mut actors = BTreeSet::new();
    while let Some(node) = pending.pop_front() {
        reads.inverse(node)?;
        if reads.world.get(node).is_some_and(|o| o.kind.is_actor()) {
            actors.insert(node);
            if actors.len() > MAX_ACTORS {
                return Err(refused("personnel actor limit"));
            }
        }
        let children = reads.contents(node)?.clone();
        for child in children {
            reads.step()?;
            if reads
                .world
                .get(child)
                .is_none_or(|o| o.location != Some(node))
            {
                return Err(refused("personnel contents and location disagree"));
            }
            enqueue(reads, &mut pending, &mut scheduled, child)?;
        }
        // A physically nested, unplaced hull need not occur in the spatial
        // subtree. Its separate anchored rooms still move with its ancestry.
        // Follow both views so newly anchored and formerly anchored cabins are
        // included. Never expand an arbitrary domain or the whole World.
        for view in [before, after] {
            let mut spaces = view.anchored_spaces(reads, node)?;
            if view.space(node).is_some() {
                spaces.insert(node);
            }
            for space in spaces {
                enqueue(reads, &mut pending, &mut scheduled, space)?;
                let record = view
                    .space(space)
                    .ok_or_else(|| refused("missing census interior"))?;
                if record.rooms.len() > crate::data::MAX_SPACE_ROOMS {
                    return Err(refused("personnel interior room limit"));
                }
                for &room in &record.rooms {
                    reads.step()?;
                    enqueue(reads, &mut pending, &mut scheduled, room)?;
                }
            }
        }
    }
    Ok(actors)
}

fn enqueue(
    reads: &mut Reads<'_>,
    pending: &mut VecDeque<ObjId>,
    scheduled: &mut BTreeSet<ObjId>,
    node: ObjId,
) -> Result<(), ParticipantError> {
    reads.touch(node)?;
    if scheduled.insert(node) {
        if scheduled.len() > MAX_DEPENDENCIES {
            return Err(refused("personnel frontier limit"));
        }
        pending.push_back(node);
    }
    Ok(())
}

fn digest<T: Serialize>(domain: &str, value: &T) -> Result<DefinitionDigest, ParticipantError> {
    let value = serde_json::to_value(value).map_err(|_| ParticipantError::Encoding)?;
    mush_core::canonical_json_bytes_with_limit(&value, MAX_FACT_BYTES)
        .map_err(|_| refused("personnel facts exceed byte limit"))?;
    DefinitionDigest::for_canonical_value(domain, &value).map_err(|_| ParticipantError::Encoding)
}

pub(crate) fn derive(
    state: &SpatialState,
    world: &World,
    transition: &Transition,
    plan: &Plan,
    stamp: &ClockStamp,
    baseline_dependencies: &BTreeSet<ObjId>,
) -> Result<CaptureFacts, ParticipantError> {
    if !matches!(stamp.clock, ClockRef::WorldIc { generation } if generation > 0)
        || baseline_dependencies.len() > MAX_DEPENDENCIES
    {
        return Err(refused("invalid personnel capture boundary"));
    }
    let mut reads = Reads {
        world,
        dependencies: BTreeSet::new(),
        contents: BTreeMap::new(),
        steps: 0,
    };
    let before = View::before(state);
    let after = View::after(state, plan)?;
    for (&object, &destination) in &after.projections {
        reads.inverse(object)?;
        reads.touch(destination)?;
        if world.get(object).and_then(|o| o.location) != Some(destination)
            && reads.contents(destination)?.contains(&object)
        {
            return Err(refused("personnel destination inverse disagrees"));
        }
    }
    let roots = seeds(state, transition, plan, baseline_dependencies, &mut reads)?;
    derive_views(
        before,
        after,
        reads,
        roots,
        stamp,
        CaptureDomain::Structural,
    )
}

/// Ordinary locomotion changes only the actual actor's Core projection. The
/// canonical placement/space maps remain byte-for-byte unchanged, including an
/// absent placement row. The caller supplies already-admitted route dependencies.
pub(crate) fn derive_actor_move(
    state: &SpatialState,
    world: &World,
    actor: ObjId,
    destination: ObjId,
    stamp: &ClockStamp,
    baseline_dependencies: &BTreeSet<ObjId>,
) -> Result<CaptureFacts, ParticipantError> {
    derive_actor_move_in(
        state,
        world,
        actor,
        destination,
        stamp,
        baseline_dependencies,
        CaptureDomain::Locomotion,
    )
}

pub(crate) fn derive_window_admission(
    state: &SpatialState,
    world: &World,
    actor: ObjId,
    destination: ObjId,
    stamp: &ClockStamp,
    dependencies: &BTreeSet<ObjId>,
) -> Result<CaptureFacts, ParticipantError> {
    derive_actor_move_in(
        state,
        world,
        actor,
        destination,
        stamp,
        dependencies,
        CaptureDomain::Window,
    )
}

fn derive_actor_move_in(
    state: &SpatialState,
    world: &World,
    actor: ObjId,
    destination: ObjId,
    stamp: &ClockStamp,
    baseline_dependencies: &BTreeSet<ObjId>,
    domain: CaptureDomain,
) -> Result<CaptureFacts, ParticipantError> {
    if !matches!(stamp.clock, ClockRef::WorldIc { generation } if generation > 0)
        || baseline_dependencies.len() > MAX_DEPENDENCIES
        || world
            .get(actor)
            .is_none_or(|object| !object.kind.is_actor())
    {
        return Err(refused("invalid locomotion capture boundary"));
    }
    let mut reads = Reads {
        world,
        dependencies: BTreeSet::new(),
        contents: BTreeMap::new(),
        steps: 0,
    };
    for &dependency in baseline_dependencies {
        reads.touch(dependency)?;
    }
    reads.inverse(actor)?;
    reads.touch(destination)?;
    if world.get(actor).and_then(|object| object.location) != Some(destination)
        && reads.contents(destination)?.contains(&actor)
    {
        return Err(refused("locomotion destination inverse disagrees"));
    }
    let before = View::before(state);
    let mut after = View::before(state);
    after.projections.insert(actor, destination);
    derive_views(before, after, reads, BTreeSet::from([actor]), stamp, domain)
}

#[derive(Clone, Copy)]
enum CaptureDomain {
    Structural,
    Locomotion,
    Window,
}

/// Original World rows remain the read fences. Both semantic ancestry views
/// overlay the genuine first Core move; the admitted native plan supplies only
/// its subsequent actor projection. No World or actor record is substituted.
pub(crate) fn derive_window_phase(
    state: &SpatialState,
    world: &World,
    actor: ObjId,
    current_room: ObjId,
    plan: &Plan,
    stamp: &ClockStamp,
    dependencies: &BTreeSet<ObjId>,
) -> Result<CaptureFacts, ParticipantError> {
    if !matches!(stamp.clock, ClockRef::WorldIc {generation} if generation > 0)
        || dependencies.len() > MAX_DEPENDENCIES
        || !plan.set_spaces.is_empty()
        || !plan.retire.is_empty()
        || plan.set_placements.iter().any(|p| p.object != actor)
        || plan.projections.iter().any(|(object, _)| *object != actor)
    {
        return Err(refused("unsupported window personnel phase"));
    }
    let mut reads = Reads {
        world,
        dependencies: BTreeSet::new(),
        contents: BTreeMap::new(),
        steps: 0,
    };
    for &dependency in dependencies {
        reads.touch(dependency)?;
    }
    reads.inverse(actor)?;
    reads.touch(current_room)?;
    let mut before = View::before(state);
    before.projections.insert(actor, current_room);
    let mut after = View::after(state, plan)?;
    after.projections.entry(actor).or_insert(current_room);
    for &destination in after.projections.values() {
        reads.touch(destination)?;
    }
    derive_views(
        before,
        after,
        reads,
        BTreeSet::from([actor]),
        stamp,
        CaptureDomain::Window,
    )
}

fn derive_views(
    before: View<'_>,
    after: View<'_>,
    mut reads: Reads<'_>,
    roots: BTreeSet<ObjId>,
    stamp: &ClockStamp,
    domain: CaptureDomain,
) -> Result<CaptureFacts, ParticipantError> {
    let people = actors(&mut reads, roots, &before, &after)?;
    let mut old = Vec::with_capacity(people.len());
    let mut new = Vec::with_capacity(people.len());
    let mut crossings = Vec::new();
    for character in people {
        let pre = ancestry(&before, &mut reads, character)?;
        let post = ancestry(&after, &mut reads, character)?;
        for (&carrier, direction) in pre
            .carriers
            .difference(&post.carriers)
            .map(|id| (id, CrossingDirection::Exit))
            .chain(
                post.carriers
                    .difference(&pre.carriers)
                    .map(|id| (id, CrossingDirection::Enter)),
            )
        {
            if crossings.len() >= MAX_CROSSINGS {
                return Err(refused("personnel crossing limit"));
            }
            crossings.push(Crossing {
                character,
                carrier,
                direction,
            });
        }
        old.push(pre);
        new.push(post);
    }
    crossings.sort_by_key(|row| (row.character, row.carrier));
    let ancestry_domain = match domain {
        CaptureDomain::Structural => "sim.personnel.ancestry.v1",
        CaptureDomain::Locomotion => "sim.locomotion.ancestry.v1",
        CaptureDomain::Window => "sim.window_locomotion.ancestry.v1",
    };
    let before_ancestry_digest = digest(ancestry_domain, &old)?;
    let after_ancestry_digest = digest(ancestry_domain, &new)?;
    let capture_digest = match domain {
        CaptureDomain::Structural => crate::personnel::capture_digest(
            stamp,
            &before_ancestry_digest,
            &after_ancestry_digest,
            &crossings,
        )?,
        CaptureDomain::Locomotion => crate::locomotion_capture::capture_digest(
            stamp,
            &before_ancestry_digest,
            &after_ancestry_digest,
            &crossings,
        )?,
        CaptureDomain::Window => digest(
            "sim.window_locomotion.phase.v1",
            &serde_json::json!({"prepared_at":stamp,"before_ancestry_digest":before_ancestry_digest,"after_ancestry_digest":after_ancestry_digest,"crossings":crossings}),
        )?,
    };
    Ok(CaptureFacts {
        prepared_at: stamp.clone(),
        before_ancestry_digest,
        after_ancestry_digest,
        capture_digest,
        crossings,
        read_dependencies: reads.dependencies,
    })
}

#[cfg(test)]
#[path = "personnel_capture_tests.rs"]
mod tests;
