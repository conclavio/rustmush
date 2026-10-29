//! Ordered candidate ancestry, with no install, receipt or coverage grant.
//! Native phase plans remain typed and private; historical bytes cannot mint one.

use super::*;
use crate::personnel_capture::{self, CaptureFacts};

pub struct OrderedWindowMovementCapture {
    core: Box<LocomotionPreview>,
    target: Option<WindowTarget>,
    stamp: ClockStamp,
    bytes: Vec<u8>,
    digest: DefinitionDigest,
    dependencies: BTreeSet<ObjId>,
    native_phases: Vec<NativePhase>,
    occupancy: Option<crate::window_occupancy::OccupancyFence>,
    native_fence: native_fence::NativeFence,
}
impl OrderedWindowMovementCapture {
    #[must_use]
    pub fn actor(&self) -> ObjId {
        self.core.actor()
    }
    #[must_use]
    pub fn at_utc(&self) -> i64 {
        self.core.at_utc()
    }
    #[must_use]
    pub const fn prepared_at(&self) -> &ClockStamp {
        &self.stamp
    }
    #[must_use]
    pub const fn digest(&self) -> DefinitionDigest {
        self.digest
    }
    #[must_use]
    pub fn stage_count(&self) -> usize {
        self.native_phases.len() + 1
    }
    #[must_use]
    pub const fn read_dependencies(&self) -> &BTreeSet<ObjId> {
        &self.dependencies
    }
    pub fn encode(&self) -> Vec<u8> {
        self.bytes.clone()
    }
    /// Check just the selected domain's actual placement coordinates at native
    /// slot bind. This performs no World read or callback. Custody exclusions,
    /// topology, Core rows and Atlas selectors still need their own guards.
    pub fn recheck_occupancy(&self, state: &SpatialState) -> Result<(), ParticipantError> {
        if self
            .occupancy
            .as_ref()
            .is_some_and(|fence| !fence.recheck(state))
        {
            return Err(refused());
        }
        Ok(())
    }
    /// Validate the actual native slot's complete canonical graph, placement,
    /// control and allocator preimage without World or external callbacks.
    /// This grants no install. Core, foreign reads, Atlas and the recovery
    /// append retain their own required bindings.
    pub fn recheck_native(&self, state: &SpatialState) -> Result<(), ParticipantError> {
        self.native_fence.recheck(state)
    }
    pub fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        if derive_ordered(world, &self.core, self.target, &self.stamp)?.bytes != self.bytes {
            return Err(refused());
        }
        Ok(())
    }
}

/// Distinct v2 candidate data. The admitted Core step and every selected native
/// phase retain their own ancestry/crossings, even when the net move is a loop.
pub fn capture_ordered(
    world: &World,
    core: LocomotionPreview,
    target: Option<WindowTarget>,
) -> Result<OrderedWindowMovementCapture, ParticipantError> {
    let stamp = world.world_ic_now();
    let derived = derive_ordered(world, &core, target, &stamp)?;
    let value: Value =
        serde_json::from_slice(&derived.bytes).map_err(|_| ParticipantError::Encoding)?;
    let digest = DefinitionDigest::for_canonical_value_with_limit(
        "sim.window_movement.capture.v2",
        &value,
        MAX_WINDOW_MOVEMENT_BYTES,
    )
    .map_err(|_| ParticipantError::ImageLimit)?;
    Ok(OrderedWindowMovementCapture {
        core: Box::new(core),
        target,
        stamp,
        bytes: derived.bytes,
        digest,
        dependencies: derived.dependencies,
        native_phases: derived.native_phases,
        occupancy: derived.occupancy,
        native_fence: derived.native_fence,
    })
}
struct Ordered {
    bytes: Vec<u8>,
    dependencies: BTreeSet<ObjId>,
    native_phases: Vec<NativePhase>,
    occupancy: Option<crate::window_occupancy::OccupancyFence>,
    native_fence: native_fence::NativeFence,
}
fn derive_ordered(
    world: &World,
    core: &LocomotionPreview,
    target: Option<WindowTarget>,
    stamp: &ClockStamp,
) -> Result<Ordered, ParticipantError> {
    let native = derive_phases(world, core, target, stamp)?;
    let mut dependencies = native.dependencies;
    dependencies.extend(core.touched_objects());
    dependencies.extend(core.source());
    dependencies.extend([core.actor(), core.admitted_room(), core.destination()]);
    if dependencies.len() > personnel_capture::MAX_DEPENDENCIES {
        return Err(ParticipantError::ImageLimit);
    }
    // The same resident bound and opaque-payload preflight already ran before
    // native planning. Only a bounded SpatialState is copied, never World.
    let mut post = world
        .module_state::<SpatialState>()
        .ok_or_else(refused)?
        .clone();
    let native_fence = native_fence::NativeFence::capture(&post)?;
    let occupancy = target
        .map(|target| crate::window_occupancy::OccupancyFence::capture(&post, target.domain))
        .transpose()
        .map_err(|_| refused())?;
    let first = personnel_capture::derive_window_admission(
        &post,
        world,
        core.actor(),
        core.admitted_room(),
        stamp,
        &dependencies,
    )?;
    let mut previous = first.after_ancestry_digest;
    let mut total = first.crossings.len();
    dependencies.extend(first.read_dependencies.iter().copied());
    let mut stages = vec![stage("core_admission", &first)];
    let mut room = core.admitted_room();
    for phase in &native.phases {
        let facts = personnel_capture::derive_window_phase(
            &post,
            world,
            core.actor(),
            room,
            &phase.plan,
            stamp,
            &dependencies,
        )?;
        if facts.before_ancestry_digest != previous {
            return Err(refused());
        }
        previous = facts.after_ancestry_digest;
        total = total
            .checked_add(facts.crossings.len())
            .ok_or(ParticipantError::ImageLimit)?;
        dependencies.extend(facts.read_dependencies.iter().copied());
        if dependencies.len() > personnel_capture::MAX_DEPENDENCIES
            || total > personnel_capture::MAX_CROSSINGS
        {
            return Err(ParticipantError::ImageLimit);
        }
        stages.push(stage(
            match phase.kind {
                PhaseKind::Release => "release",
                PhaseKind::Place => "place",
            },
            &facts,
        ));
        if let Some((_, destination)) = phase.plan.projections.first() {
            room = *destination;
        }
        for record in &phase.plan.set_placements {
            post.placements.insert(record.object, record.clone());
        }
        post.derived = crate::graph::rebuild(&post);
    }
    if room != core.destination()
        || dependencies.len() > personnel_capture::MAX_DEPENDENCIES
        || total > personnel_capture::MAX_CROSSINGS
    {
        return Err(refused());
    }
    let native_value: Value =
        serde_json::from_slice(&native.bytes).map_err(|_| ParticipantError::Encoding)?;
    let value = json!({"format":2,"authority":"hypothetical_requires_joint_owners",
        "coverage":"uncertified_legacy_paths","native":native_value,"stages":stages,"read_dependencies":dependencies,
        "occupancy":occupancy,"native_preimage":native_fence});
    let bytes = mush_core::canonical_json_bytes_with_limit(&value, MAX_WINDOW_MOVEMENT_BYTES)
        .map_err(|_| ParticipantError::ImageLimit)?;
    Ok(Ordered {
        bytes,
        dependencies,
        native_phases: native.phases,
        occupancy,
        native_fence,
    })
}
fn stage(kind: &str, facts: &CaptureFacts) -> Value {
    json!({"kind":kind,"prepared_at":facts.prepared_at,"before_ancestry_digest":facts.before_ancestry_digest,
        "after_ancestry_digest":facts.after_ancestry_digest,"capture_digest":facts.capture_digest,"crossings":facts.crossings})
}
