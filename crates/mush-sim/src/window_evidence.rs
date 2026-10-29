//! Historical data validation only. Nothing decoded here can become a Core
//! preview, native Plan, owner patch, installation or eligibility capability.

use super::*;
use crate::data::{DomainRecord, PlacementRecord, UnplacedInfo};
use crate::personnel::Crossing;
use serde::{Deserialize, Deserializer};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Authority {
    HypotheticalRequiresJointOwners,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Coverage {
    UncertifiedLegacyPaths,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    CoreAdmission,
    Release,
    Place,
}
fn required<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    Option::deserialize(d)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    domain: DomainId,
    x_cm: i64,
    y_cm: i64,
    z_cm: i32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Native {
    format: u16,
    authority: Authority,
    actor: ObjId,
    #[serde(deserialize_with = "required")]
    source: Option<ObjId>,
    admitted_room: ObjId,
    projected_room: ObjId,
    at_utc: i64,
    prepared_at: ClockStamp,
    #[serde(deserialize_with = "required")]
    target: Option<Target>,
    #[serde(deserialize_with = "required")]
    before: Option<PlacementRecord>,
    domains: Vec<(DomainId, Option<DomainRecord>)>,
    local_fence: DefinitionDigest,
    phases: Vec<Phase>,
    #[serde(deserialize_with = "required")]
    after: Option<PlacementRecord>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Phase {
    placements: Vec<PlacementRecord>,
    projections: Vec<(ObjId, ObjId)>,
    audit: Value,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stage {
    kind: Kind,
    prepared_at: ClockStamp,
    before_ancestry_digest: DefinitionDigest,
    after_ancestry_digest: DefinitionDigest,
    capture_digest: DefinitionDigest,
    crossings: Vec<Crossing>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Occupancy {
    domain: DomainId,
    records: usize,
    digest: DefinitionDigest,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativePreimage {
    digest: DefinitionDigest,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    format: u16,
    authority: Authority,
    coverage: Coverage,
    native: Native,
    stages: Vec<Stage>,
    read_dependencies: Vec<ObjId>,
    #[serde(deserialize_with = "required")]
    occupancy: Option<Occupancy>,
    native_preimage: NativePreimage,
}

/// Validated passive evidence, still requiring an actual owner receipt and
/// same-commit records. There is no conversion back to an executable capture.
pub struct WindowMovementEvidence {
    data: Box<Evidence>,
    digest: DefinitionDigest,
}
impl WindowMovementEvidence {
    pub fn actor(&self) -> ObjId {
        self.data.native.actor
    }
    pub fn source(&self) -> Option<ObjId> {
        self.data.native.source
    }
    pub fn admitted_room(&self) -> ObjId {
        self.data.native.admitted_room
    }
    pub fn projected_room(&self) -> ObjId {
        self.data.native.projected_room
    }
    pub fn at_utc(&self) -> i64 {
        self.data.native.at_utc
    }
    pub fn prepared_at(&self) -> &ClockStamp {
        &self.data.native.prepared_at
    }
    pub fn digest(&self) -> DefinitionDigest {
        self.digest
    }
    pub fn stage_count(&self) -> usize {
        self.data.stages.len()
    }
    pub fn crossing_count(&self) -> usize {
        self.data.stages.iter().map(|s| s.crossings.len()).sum()
    }
    pub fn read_dependencies(&self) -> &[ObjId] {
        &self.data.read_dependencies
    }
    pub fn final_placement(&self) -> Option<&PlacementRecord> {
        self.data.native.after.as_ref()
    }
}

/// Check against the digest retained by a genuine owner receipt. A caller's
/// freshly computed digest alone grants no authority; historical callers must
/// also bind actor, route, clock and commit to that actual receipt.
pub fn validate_evidence(
    value: &Value,
    expected: DefinitionDigest,
) -> Result<WindowMovementEvidence, ParticipantError> {
    preflight(value)?;
    let digest = DefinitionDigest::for_canonical_value_with_limit(
        "sim.window_movement.capture.v2",
        value,
        MAX_WINDOW_MOVEMENT_BYTES,
    )
    .map_err(|_| ParticipantError::ImageLimit)?;
    if !nonzero(expected) || digest != expected {
        return Err(refused());
    }
    let data: Evidence =
        serde_json::from_value(value.clone()).map_err(|_| ParticipantError::Encoding)?;
    if serde_json::to_value(&data).map_err(|_| ParticipantError::Encoding)? != *value {
        return Err(refused());
    }
    check(&data)?;
    Ok(WindowMovementEvidence {
        data: Box::new(data),
        digest,
    })
}
fn nonzero(digest: DefinitionDigest) -> bool {
    digest.as_bytes() != &[0; 32]
}
fn preflight(value: &Value) -> Result<(), ParticipantError> {
    let mut nodes = 65_536_usize;
    let mut text = MAX_WINDOW_MOVEMENT_BYTES;
    let mut pending = vec![(value, 1_usize)];
    while let Some((value, depth)) = pending.pop() {
        nodes = nodes.checked_sub(1).ok_or(ParticipantError::ImageLimit)?;
        if depth > 96 {
            return Err(ParticipantError::ImageLimit);
        }
        match value {
            Value::Array(values) => {
                if values.len() > nodes.saturating_sub(pending.len()) {
                    return Err(ParticipantError::ImageLimit);
                }
                pending.extend(values.iter().map(|v| (v, depth + 1)));
            }
            Value::Object(values) => {
                if values.len() > nodes.saturating_sub(pending.len()) {
                    return Err(ParticipantError::ImageLimit);
                }
                for (key, value) in values {
                    text = text
                        .checked_sub(key.len())
                        .ok_or(ParticipantError::ImageLimit)?;
                    pending.push((value, depth + 1));
                }
            }
            Value::String(value) => {
                text = text
                    .checked_sub(value.len())
                    .ok_or(ParticipantError::ImageLimit)?;
            }
            _ => {}
        }
    }
    Ok(())
}
fn check(e: &Evidence) -> Result<(), ParticipantError> {
    let n = &e.native;
    if e.format != 2
        || n.format != 1
        || e.stages.is_empty()
        || e.stages.len() > 3
        || n.phases.len() + 1 != e.stages.len()
        || !nonzero(n.local_fence)
        || !nonzero(e.native_preimage.digest)
        || e.read_dependencies.len() > crate::personnel_capture::MAX_DEPENDENCIES
        || e.read_dependencies.windows(2).any(|w| w[0] >= w[1])
        || !matches!(n.prepared_at.clock, ClockRef::WorldIc { generation } if generation > 0)
        || n.actor == n.admitted_room
        || n.actor == n.projected_room
    {
        return Err(refused());
    }
    let dependencies: BTreeSet<_> = e.read_dependencies.iter().copied().collect();
    if [n.actor, n.admitted_room, n.projected_room]
        .into_iter()
        .chain(n.source)
        .any(|id| !dependencies.contains(&id))
    {
        return Err(refused());
    }
    let mut total = 0usize;
    for (index, stage) in e.stages.iter().enumerate() {
        if stage.prepared_at != n.prepared_at
            || !nonzero(stage.before_ancestry_digest)
            || !nonzero(stage.after_ancestry_digest)
            || (index == 0 && stage.kind != Kind::CoreAdmission)
            || (index > 0
                && e.stages[index - 1].after_ancestry_digest != stage.before_ancestry_digest)
            || stage
                .crossings
                .iter()
                .any(|c| !dependencies.contains(&c.character) || !dependencies.contains(&c.carrier))
            || crate::personnel_capture::window_phase_digest(
                &stage.prepared_at,
                &stage.before_ancestry_digest,
                &stage.after_ancestry_digest,
                &stage.crossings,
            )? != stage.capture_digest
        {
            return Err(refused());
        }
        total = total
            .checked_add(stage.crossings.len())
            .ok_or(ParticipantError::ImageLimit)?;
        if total > crate::personnel_capture::MAX_CROSSINGS {
            return Err(ParticipantError::ImageLimit);
        }
    }
    let source = match n.before.as_ref().map(|p| &p.parent) {
        Some(SpatialParent::Direct { domain, .. }) => Some(*domain),
        None | Some(SpatialParent::Unplaced) => None,
        _ => return Err(refused()),
    };
    let target = n.target.as_ref().map(|t| t.domain);
    let domains: BTreeSet<_> = source.into_iter().chain(target).collect();
    if n.domains.len() != domains.len()
        || n.domains
            .iter()
            .map(|(id, _)| *id)
            .ne(domains.iter().copied())
    {
        return Err(refused());
    }
    for (id, record) in &n.domains {
        let record = record.as_ref().ok_or_else(refused)?;
        if record.id != *id {
            return Err(refused());
        }
        crate::physical::encoded_size(record, RECORD_BYTES)?;
    }
    match (&n.target, &e.occupancy) {
        (None, None) if n.projected_room == n.admitted_room => {}
        (Some(target), Some(occupancy))
            if target.domain == occupancy.domain
                && occupancy.records
                    <= crate::window_occupancy::MAX_WINDOW_OCCUPANCY_PLACEMENTS
                && nonzero(occupancy.digest) => {}
        _ => return Err(refused()),
    }
    crate::physical::encoded_size(&n.before, RECORD_BYTES)?;
    if n.before.as_ref().is_some_and(|p| p.object != n.actor) {
        return Err(refused());
    }
    let mut kinds = Vec::new();
    if source.is_some() && source != target {
        kinds.push(Kind::Release);
    }
    if target.is_some() {
        kinds.push(Kind::Place);
    }
    if n.phases.len() != kinds.len() {
        return Err(refused());
    }
    let mut current = n.before.clone();
    for (index, (phase, kind)) in n.phases.iter().zip(kinds).enumerate() {
        if e.stages[index + 1].kind != kind || phase.placements.len() != 1 {
            return Err(refused());
        }
        let revision = current
            .as_ref()
            .map_or(0, |p| p.revision)
            .checked_add(1)
            .ok_or_else(refused)?;
        let actual = &phase.placements[0];
        if actual.object != n.actor || actual.revision != revision {
            return Err(refused());
        }
        match kind {
            Kind::Release => {
                let previous = current.as_ref().ok_or_else(refused)?;
                let expected = PlacementRecord {
                    object: n.actor,
                    parent: SpatialParent::Unplaced,
                    revision,
                    unplaced: Some(UnplacedInfo {
                        reason: "left the map".into(),
                        by: Some(n.actor),
                        at: n.at_utc,
                        previous: Some(previous.parent.clone()),
                        quarantined: false,
                    }),
                };
                if *actual != expected
                    || !phase.projections.is_empty()
                    || phase.audit
                        != json!({"op":"unplace","obj":n.actor.0,"reason":"left the map","by":n.actor.0})
                {
                    return Err(refused());
                }
            }
            Kind::Place => {
                let target = n.target.as_ref().ok_or_else(refused)?;
                let parent = SpatialParent::Direct {
                    domain: target.domain,
                    pose: crate::Pose::at(
                        target.x_cm,
                        target.y_cm,
                        target.z_cm,
                        crate::passive_rules_value(json!({}))
                            .map_err(|_| ParticipantError::Encoding)?,
                    ),
                };
                let expected = PlacementRecord {
                    object: n.actor,
                    parent: parent.clone(),
                    revision,
                    unplaced: None,
                };
                if *actual != expected
                    || phase.projections != vec![(n.actor, n.projected_room)]
                    || phase.audit
                        != json!({"op":"place","obj":n.actor.0,"parent":parent,"previous":current.as_ref().map(|p| &p.parent)})
                {
                    return Err(refused());
                }
            }
            Kind::CoreAdmission => return Err(refused()),
        }
        current = Some(actual.clone());
    }
    if current != n.after {
        return Err(refused());
    }
    Ok(())
}
