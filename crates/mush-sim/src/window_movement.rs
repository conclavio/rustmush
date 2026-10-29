//! Read-only Sim plans for a hypothetical, Core-validated window arrival.
//!
//! This supplies the real release/place transition data for an already attached
//! actor with no physical dependents. The supplied projection still needs the
//! actual Atlas owner's attestation. There is no receipt, write capability,
//! decoder, live route, or personnel-eligibility claim here.

use std::collections::BTreeSet;

use mush_core::locomotion::preview::LocomotionPreview;
use mush_core::participant::ParticipantError;
use mush_core::{ClockRef, ClockStamp, DefinitionDigest, ObjId, World};
use serde::Serialize;
use serde_json::{Value, json};

use crate::data::{DomainId, SpatialParent, SpatialState};
use crate::transition::{Plan, Transition};

#[path = "window_native_fence.rs"]
mod native_fence;
#[path = "window_ordered.rs"]
mod ordered;
pub use ordered::{OrderedWindowMovementCapture, capture_ordered};

pub const MAX_WINDOW_MOVEMENT_BYTES: usize = 128 * 1024;
const RECORD_BYTES: usize = 16 * 1024;

/// The fresh passive pose used by Atlas entry, never a caller's opaque posture.
/// Metric coordinates and domain are hypothetical input, not owner authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WindowTarget {
    domain: DomainId,
    x_cm: i64,
    y_cm: i64,
    z_cm: i32,
}
impl WindowTarget {
    #[must_use]
    pub const fn at(domain: DomainId, x_cm: i64, y_cm: i64, z_cm: i32) -> Self {
        Self {
            domain,
            x_cm,
            y_cm,
            z_cm,
        }
    }
}

pub struct WindowMovementCapture {
    core: LocomotionPreview,
    target: Option<WindowTarget>,
    stamp: ClockStamp,
    bytes: Vec<u8>,
    digest: DefinitionDigest,
}
impl WindowMovementCapture {
    #[must_use]
    pub const fn actor(&self) -> ObjId {
        self.core.actor()
    }
    #[must_use]
    pub const fn prepared_at(&self) -> &ClockStamp {
        &self.stamp
    }
    #[must_use]
    pub const fn digest(&self) -> DefinitionDigest {
        self.digest
    }
    pub fn encode(&self) -> Vec<u8> {
        self.bytes.clone()
    }
    pub fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        if derive(world, &self.core, self.target, &self.stamp)? != self.bytes {
            return Err(refused());
        }
        Ok(())
    }
}
fn refused() -> ParticipantError {
    ParticipantError::refused("window movement capture is unavailable, unsupported or changed")
}

/// Capture only; even successful planning grants neither Sim admission nor
/// permission to publish the Core projection supplied by the caller.
pub fn capture(
    world: &World,
    core: LocomotionPreview,
    target: Option<WindowTarget>,
) -> Result<WindowMovementCapture, ParticipantError> {
    let stamp = world.world_ic_now();
    let bytes = derive(world, &core, target, &stamp)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| ParticipantError::Encoding)?;
    let digest = DefinitionDigest::for_canonical_value_with_limit(
        "sim.window_movement.capture.v1",
        &value,
        MAX_WINDOW_MOVEMENT_BYTES,
    )
    .map_err(|_| ParticipantError::ImageLimit)?;
    Ok(WindowMovementCapture {
        core,
        target,
        stamp,
        bytes,
        digest,
    })
}

fn derive(
    world: &World,
    core: &LocomotionPreview,
    target: Option<WindowTarget>,
    stamp: &ClockStamp,
) -> Result<Vec<u8>, ParticipantError> {
    Ok(derive_phases(world, core, target, stamp)?.bytes)
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum PhaseKind {
    Release,
    Place,
}
struct NativePhase {
    kind: PhaseKind,
    plan: Plan,
}
struct DerivedPhases {
    bytes: Vec<u8>,
    phases: Vec<NativePhase>,
    dependencies: BTreeSet<ObjId>,
}

fn derive_phases(
    world: &World,
    core: &LocomotionPreview,
    target: Option<WindowTarget>,
    stamp: &ClockStamp,
) -> Result<DerivedPhases, ParticipantError> {
    core.recheck(world)?;
    let now = world.world_ic_now();
    if !matches!(stamp.clock, ClockRef::WorldIc { generation } if generation > 0)
        || now.clock != stamp.clock
        || now.at < stamp.at
    {
        return Err(refused());
    }
    if target.is_none() && core.destination() != core.admitted_room() {
        return Err(refused());
    }
    let state = world.module_state::<SpatialState>().ok_or_else(refused)?;
    posture_bounds(state)?;
    crate::physical::resident_bound(state)?;
    if crate::graph::rebuild(state) != state.derived {
        return Err(refused());
    }
    let actor = core.actor();
    if state.held_placement(actor).is_some()
        || !state.descendant_edges(actor).is_empty()
        || state.domains.values().any(|domain| domain.host == actor)
        || state.spaces.contains_key(&actor)
        || state.mounts.contains_key(&actor)
        || state.stations.contains_key(&actor)
        || state.assignments.contains_key(&actor)
        || state.held_assignments.contains_key(&actor)
    {
        return Err(refused());
    }
    let before = state.placements.get(&actor);
    crate::physical::encoded_size(&before, RECORD_BYTES)?;
    if before.is_some_and(|record| record.object != actor) {
        return Err(refused());
    }
    let source = match before.map(|record| &record.parent) {
        Some(SpatialParent::Direct { domain, .. }) => Some(*domain),
        None | Some(SpatialParent::Unplaced) => None,
        _ => return Err(refused()),
    };
    let destination = target.map(|target| target.domain);
    let domains: BTreeSet<_> = source.into_iter().chain(destination).collect();
    let mut nodes = BTreeSet::from([actor]);
    nodes.extend(crate::graph::ancestors(state, actor).map_err(|_| refused())?);
    for id in &domains {
        let domain = state.domains.get(id).ok_or_else(refused)?;
        if domain.id != *id {
            return Err(refused());
        }
        crate::physical::encoded_size(domain, RECORD_BYTES)?;
        nodes.insert(domain.host);
        nodes.extend(crate::graph::ancestors(state, domain.host).map_err(|_| refused())?);
    }
    let fence = crate::physical::state_digest(state, &nodes, &domains)?;
    let mut post = state.clone();
    let mut phases = Vec::new();
    if source.is_some() && source != destination {
        let release = crate::transition::plan_at(
            &post,
            world,
            &Transition::Unplace {
                obj: actor,
                reason: "left the map".into(),
                by: Some(actor),
                keep: true,
            },
            core.at_utc(),
        )
        .map_err(|_| refused())?;
        push_phase(world, &mut post, release, PhaseKind::Release, &mut phases)?;
    }
    if let Some(target) = target {
        let pose = crate::Pose::at(
            target.x_cm,
            target.y_cm,
            target.z_cm,
            crate::passive_rules_value(json!({})).map_err(|_| ParticipantError::Encoding)?,
        );
        let placement = crate::transition::plan_window_place(
            &post,
            world,
            &SpatialParent::Direct {
                domain: target.domain,
                pose,
            },
            core,
        )
        .map_err(|_| refused())?;
        push_phase(world, &mut post, placement, PhaseKind::Place, &mut phases)?;
    }
    let domain_records: Vec<_> = domains
        .iter()
        .map(|id| (id, state.domains.get(id)))
        .collect();
    let phase_values:Vec<_> = phases.iter().map(|phase| {
        let plan = &phase.plan;
        json!({"placements":plan.set_placements,"projections":plan.projections,"audit":plan.audit})
    }).collect();
    let value = json!({"format":1,"authority":"hypothetical_requires_joint_owners",
        "actor":actor,"source":core.source(),"admitted_room":core.admitted_room(),
        "projected_room":core.destination(),"at_utc":core.at_utc(),"prepared_at":stamp,
        "target":target,"before":before,"domains":domain_records,"local_fence":fence,
        "phases":phase_values,"after":post.placements.get(&actor)});
    let bytes = mush_core::canonical_json_bytes_with_limit(&value, MAX_WINDOW_MOVEMENT_BYTES)
        .map_err(|_| ParticipantError::ImageLimit)?;
    Ok(DerivedPhases {
        bytes,
        phases,
        dependencies: nodes,
    })
}

fn push_phase(
    world: &World,
    state: &mut SpatialState,
    plan: Plan,
    kind: PhaseKind,
    phases: &mut Vec<NativePhase>,
) -> Result<(), ParticipantError> {
    if !plan.set_spaces.is_empty() || !plan.retire.is_empty() {
        return Err(refused());
    }
    crate::ic_exclusion::check_plan(world, &plan).map_err(|_| refused())?;
    for record in &plan.set_placements {
        state.placements.insert(record.object, record.clone());
    }
    state.derived = crate::graph::rebuild(state);
    crate::physical::resident_bound(state)?;
    phases.push(NativePhase { kind, plan });
    Ok(())
}

/// A stream byte bound cannot stop recursion before serde enters an opaque
/// payload. Preflight every posture the resident-state encoder will encounter,
/// with a shared node/text bound, before cloning or recursively encoding state.
fn posture_bounds(state: &SpatialState) -> Result<(), ParticipantError> {
    if state
        .placements
        .len()
        .saturating_add(state.spaces.len())
        .saturating_add(state.stations.len())
        > 4096
    {
        return Err(ParticipantError::ImageLimit);
    }
    let mut nodes = 65_536_usize;
    let mut text = 4 * 1024 * 1024_usize;
    for record in state.placements.values() {
        for parent in std::iter::once(&record.parent).chain(
            record
                .unplaced
                .as_ref()
                .and_then(|info| info.previous.as_ref()),
        ) {
            let posture = match parent {
                SpatialParent::Direct { pose, .. } => Some(&pose.posture),
                SpatialParent::Attached { local_pose, .. } => local_pose.as_ref(),
                _ => None,
            };
            if let Some(posture) = posture {
                value_bounds(posture.data(), &mut nodes, &mut text)?;
            }
        }
    }
    for space in state.spaces.values() {
        if let crate::SpaceAnchor::FixedDomain {
            anchor_pose: Some(pose),
            ..
        } = &space.anchor
        {
            value_bounds(pose.data(), &mut nodes, &mut text)?;
        }
        for egress in space.egresses.values() {
            if let Some(pose) = &egress.to_pose {
                value_bounds(pose.data(), &mut nodes, &mut text)?;
            }
        }
    }
    for host in state.stations.values() {
        for station in host.stations.values() {
            value_bounds(station.policy.data(), &mut nodes, &mut text)?;
        }
    }
    Ok(())
}
fn value_bounds(
    value: &Value,
    nodes: &mut usize,
    text: &mut usize,
) -> Result<(), ParticipantError> {
    let mut pending = vec![(value, 1_usize)];
    while let Some((value, depth)) = pending.pop() {
        *nodes = nodes.checked_sub(1).ok_or(ParticipantError::ImageLimit)?;
        if depth > 64 {
            return Err(ParticipantError::ImageLimit);
        }
        match value {
            Value::Array(values) => {
                if values.len() > nodes.saturating_sub(pending.len()) {
                    return Err(ParticipantError::ImageLimit);
                }
                pending.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) => {
                if values.len() > nodes.saturating_sub(pending.len()) {
                    return Err(ParticipantError::ImageLimit);
                }
                for (key, value) in values {
                    *text = text
                        .checked_sub(key.len())
                        .ok_or(ParticipantError::ImageLimit)?;
                    pending.push((value, depth + 1));
                }
            }
            Value::String(value) => {
                *text = text
                    .checked_sub(value.len())
                    .ok_or(ParticipantError::ImageLimit)?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "window_movement_tests.rs"]
mod tests;
