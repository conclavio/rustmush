//! Bounded, read-only planning of Atlas's actual window-arrival selectors.
//!
//! A selected cell is an attempted legacy placement, not Sim admission or an
//! owner proof. In particular, release may succeed before placement refuses.
//! No state, subscriptions, output, or persistence are changed here.

use std::collections::BTreeSet;

use mush_core::{DefinitionDigest, ObjId, World, canonical_json_bytes_with_limit};
use mush_sim::{DomainId, SpatialParent, SpatialState};
use serde::Serialize;

use crate::geom::PointCm;
use crate::module::AtlasState;
use crate::movement::{CaptureError, MovementCapture, WindowEffect};
use crate::record::{Endpoint, Footprint, PortalGeometry, RecordKey, Records, RoomGeo};
use crate::window::{self, Composite, SurfaceComposite};

pub const MAX_WINDOW_PLAN_BYTES: usize = 256 * 1024;
pub const MAX_WINDOW_PLAN_PLACEMENTS: usize = 4096;
pub const MAX_WINDOW_PLAN_PORTAL_PROBES: usize = 4096;
pub const MAX_WINDOW_PLAN_SURFACES: usize = 8;
const SELECTOR_BYTES: usize = 8 * 1024 * 1024;
const OCCUPANCY_BYTES: usize = 1024 * 1024;

/// Immutable observations from the real owner slots. There is deliberately no
/// decoder or install capability: a future placement protocol needs its own
/// versioned proof, grants, and final Sim/Atlas owner binds.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WindowArrivalPlan {
    format: u16,
    capture: MovementCapture,
    placement: Option<PlacementPlan>,
}

impl WindowArrivalPlan {
    #[must_use]
    pub const fn capture(&self) -> &MovementCapture {
        &self.capture
    }
    #[must_use]
    pub const fn placement(&self) -> Option<&PlacementPlan> {
        self.placement.as_ref()
    }
    /// The release selected before the placement attempt, if any.
    #[must_use]
    pub fn release(&self) -> Option<DomainId> {
        match self.capture.window_effect() {
            WindowEffect::NeedsOwnerComposition { release, .. } => release.map(DomainId),
            _ => None,
        }
    }
    pub fn encode(&self) -> Result<Vec<u8>, CaptureError> {
        canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(|_| CaptureError::Encoding)?,
            MAX_WINDOW_PLAN_BYTES,
        )
        .map_err(|_| CaptureError::Limit)
    }
    /// Recapture the original route and UTC without running either callback.
    pub fn recheck(&self, world: &World) -> Result<(), CaptureError> {
        if plan(
            world,
            self.capture.actor(),
            self.capture.from(),
            self.capture.to(),
            self.capture.at(),
        )? != *self
        {
            return Err(CaptureError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlacementPlan {
    domain: u64,
    portal: PortalSelection,
    geometry: GeometryObservation,
    occupancy: Option<OccupancyObservation>,
    outcome: EntryOutcome,
}
impl PlacementPlan {
    #[must_use]
    pub const fn domain(&self) -> DomainId {
        DomainId(self.domain)
    }
    #[must_use]
    pub const fn portal(&self) -> &PortalSelection {
        &self.portal
    }
    #[must_use]
    pub const fn outcome(&self) -> &EntryOutcome {
        &self.outcome
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PortalSelection {
    probes: usize,
    digest: DefinitionDigest,
    selected: Option<RecordKey>,
}
impl PortalSelection {
    #[must_use]
    pub const fn selected(&self) -> Option<&RecordKey> {
        self.selected.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum GeometryObservation {
    MissingWindow,
    Undrawn {
        revision: u64,
    },
    Drawn {
        revision: u64,
        digest: DefinitionDigest,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct OccupancyObservation {
    records: usize,
    digest: DefinitionDigest,
}

/// These are selector results, never successful owner effects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntryOutcome {
    MissingWindow,
    Undrawn,
    NoRoomGeometry,
    NoSurface,
    NoRoomIndex,
    NoFreeCell,
    Selected {
        surface: usize,
        x: u16,
        y: u16,
        x_cm: i64,
        y_cm: i64,
        z_cm: i32,
        /// Atlas's actual resolver result, before Sim's presentation fallback.
        projected_room: Option<ObjId>,
    },
}

/// Capture the selectors over borrowed state. Bounds precede all copies and
/// encodings; neither opaque actor postures nor the complete World are cloned.
pub fn plan(
    world: &World,
    actor: ObjId,
    from: Option<ObjId>,
    to: ObjId,
    at: i64,
) -> Result<WindowArrivalPlan, CaptureError> {
    let capture = crate::movement::capture(world, actor, from, to, at)?;
    let domain = match capture.window_effect() {
        WindowEffect::NeedsOwnerComposition {
            place_or_repose, ..
        } => place_or_repose,
        _ => None,
    };
    let placement = domain
        .map(|domain| placement(world, from, to, domain))
        .transpose()?;
    let result = WindowArrivalPlan {
        format: 1,
        capture,
        placement,
    };
    let _ = result.encode()?;
    Ok(result)
}

fn placement(
    world: &World,
    from: Option<ObjId>,
    to: ObjId,
    domain: u64,
) -> Result<PlacementPlan, CaptureError> {
    let state = world
        .module_state::<AtlasState>()
        .ok_or(CaptureError::Invalid)?;
    let portal = portal_selection(&state.records, from, to)?;
    let mut result = PlacementPlan {
        domain,
        portal,
        geometry: GeometryObservation::MissingWindow,
        occupancy: None,
        outcome: EntryOutcome::MissingWindow,
    };
    let Some(entry) = state.windows.open.get(&domain) else {
        return Ok(result);
    };
    result.geometry = GeometryObservation::Undrawn {
        revision: entry.revision,
    };
    result.outcome = EntryOutcome::Undrawn;
    let Some(composite) = &entry.composite else {
        return Ok(result);
    };
    let geo = state.records.roomgeo.get(&to);
    check_geometry(composite, geo)?;
    // Only actual selector inputs are encoded. Feature overlays, tile pins,
    // cached composite digests, and pending async jobs are not consulted here.
    let surfaces: Vec<_> = composite
        .surfaces
        .iter()
        .map(|s| (&s.surface, &s.grid, &s.cells, &s.rooms))
        .collect();
    let room = geo.map(|geo| (&geo.surface, footprint_bits(&geo.footprint)));
    result.geometry = GeometryObservation::Drawn {
        revision: entry.revision,
        digest: digest(
            "atlas.movement.window_geometry.v1",
            &(surfaces, room, state.limits.nearest_room_cells),
            SELECTOR_BYTES,
        )?,
    };
    let Some(geo) = geo else {
        result.outcome = EntryOutcome::NoRoomGeometry;
        return Ok(result);
    };
    let Some((si, surface)) = composite
        .surfaces
        .iter()
        .enumerate()
        .find(|(_, s)| s.surface == geo.surface)
    else {
        result.outcome = EntryOutcome::NoSurface;
        return Ok(result);
    };
    let Some(index) = surface
        .rooms
        .iter()
        .position(|room| *room == to)
        .and_then(|index| u16::try_from(index + 1).ok())
    else {
        result.outcome = EntryOutcome::NoRoomIndex;
        return Ok(result);
    };
    let (observation, taken) = occupancy(world, DomainId(domain), surface)?;
    result.occupancy = Some(observation);
    let geometry = result
        .portal
        .selected
        .as_ref()
        .and_then(|key| state.records.portals.get(key))
        .map(|p| &p.geometry);
    let Some((x, y)) = window::entry_cell_on_surface(surface, index, geo, geometry, &taken) else {
        result.outcome = EntryOutcome::NoFreeCell;
        return Ok(result);
    };
    // The same fresh passive posture as legacy `place_in_room`, never a copy of
    // the actor's opaque driver state. Only its metric fields enter the plan.
    let pose = window::pose_at(surface, x, y).ok_or(CaptureError::Invalid)?;
    result.outcome = EntryOutcome::Selected {
        surface: si,
        x,
        y,
        x_cm: pose.x_cm,
        y_cm: pose.y_cm,
        z_cm: pose.z_cm,
        projected_room: window::resolve_composite_room(
            composite,
            state.limits.nearest_room_cells,
            &pose,
        ),
    };
    Ok(result)
}

fn digest(
    value_domain: &str,
    value: &impl Serialize,
    limit: usize,
) -> Result<DefinitionDigest, CaptureError> {
    DefinitionDigest::for_canonical_value_with_limit(
        value_domain,
        &serde_json::to_value(value).map_err(|_| CaptureError::Encoding)?,
        limit,
    )
    .map_err(|_| CaptureError::Limit)
}

fn check_geometry(composite: &Composite, geo: Option<&RoomGeo>) -> Result<(), CaptureError> {
    if composite.surfaces.len() > MAX_WINDOW_PLAN_SURFACES {
        return Err(CaptureError::Limit);
    }
    if geo.is_some_and(|geo| matches!(&geo.footprint, Footprint::Polygon(poly) if poly.0.len() > crate::geom::MAX_POLYGON_POINTS)) {
        return Err(CaptureError::Limit);
    }
    for surface in &composite.surfaces {
        let grid = surface.grid;
        if surface.cells.len() > 65_536 || surface.rooms.len() > 4096 {
            return Err(CaptureError::Limit);
        }
        // This also makes the shared selector's cell arithmetic safe before it
        // touches a Grid, including malformed in-memory zero resolutions.
        if grid.resolution_cm == 0
            || grid.width != surface.cells.width()
            || grid.height != surface.cells.height()
            || grid.origin.x.unsigned_abs() > 100_000_000
            || grid.origin.y.unsigned_abs() > 100_000_000
        {
            return Err(CaptureError::Invalid);
        }
    }
    Ok(())
}

#[derive(Serialize)]
enum FootprintBits {
    Rect([[u64; 2]; 2]),
    Polygon(Vec<[u64; 2]>),
    Anchor([u64; 2], u64),
}
fn footprint_bits(footprint: &Footprint) -> FootprintBits {
    let point = |p: crate::geom::Point2| [p.0.to_bits(), p.1.to_bits()];
    match footprint {
        Footprint::Rect(rect) => FootprintBits::Rect([point(rect.0), point(rect.1)]),
        Footprint::Polygon(poly) => {
            FootprintBits::Polygon(poly.0.iter().map(|p| point(*p)).collect())
        }
        Footprint::Anchor { point: p, radius_m } => {
            FootprintBits::Anchor(point(*p), radius_m.to_bits())
        }
    }
}

#[derive(Serialize)]
enum EndpointFact {
    Room(ObjId),
    Outside,
}
fn endpoint(endpoint: Endpoint) -> EndpointFact {
    match endpoint {
        Endpoint::Room(room) => EndpointFact::Room(room),
        Endpoint::Outside(_) => EndpointFact::Outside,
    }
}
#[derive(Serialize)]
enum GeometryBits {
    Point([u64; 3]),
    Segment([u64; 3], [u64; 3]),
}
fn geometry_bits(geometry: PortalGeometry) -> GeometryBits {
    let point = |p: crate::geom::Point3| [p.0.to_bits(), p.1.to_bits(), p.2.to_bits()];
    match geometry {
        PortalGeometry::Point(p) => GeometryBits::Point(point(p)),
        PortalGeometry::Segment(a, b) => GeometryBits::Segment(point(a), point(b)),
    }
}
fn portal_selection(
    records: &Records,
    from: Option<ObjId>,
    to: ObjId,
) -> Result<PortalSelection, CaptureError> {
    let mut rows = Vec::new();
    let mut bytes = 0_usize;
    let mut selected = None;
    if let Some(from) = from {
        for (key, portal) in &records.portals {
            bytes = bytes
                .checked_add(
                    key.pack.as_str().len()
                        + key.entry.as_str().len()
                        + portal.key.pack.as_str().len()
                        + portal.key.entry.as_str().len(),
                )
                .ok_or(CaptureError::Limit)?;
            if rows.len() >= MAX_WINDOW_PLAN_PORTAL_PROBES
                || bytes > crate::movement::MAX_MOVEMENT_KEY_BYTES
            {
                return Err(CaptureError::Limit);
            }
            rows.push((key, &portal.key, endpoint(portal.from), endpoint(portal.to)));
            if matches!((portal.from, portal.to), (Endpoint::Room(a), Endpoint::Room(b))
                if (a == from && b == to) || (b == from && a == to))
            {
                selected = Some(portal.key.clone());
                break;
            }
        }
    }
    // `portal_between` returns the record key, then entry selection looks that
    // key up again. Preserve both steps even when a map key is inconsistent.
    let geometry = selected
        .as_ref()
        .and_then(|key| records.portals.get(key))
        .map(|p| geometry_bits(p.geometry));
    Ok(PortalSelection {
        probes: rows.len(),
        digest: digest(
            "atlas.movement.window_portal.v1",
            &(rows, geometry),
            OCCUPANCY_BYTES,
        )?,
        selected,
    })
}

fn occupancy(
    world: &World,
    domain: DomainId,
    surface: &SurfaceComposite,
) -> Result<(OccupancyObservation, BTreeSet<(u16, u16)>), CaptureError> {
    let mut rows = Vec::new();
    let mut taken = BTreeSet::new();
    if let Some(state) = world.module_state::<SpatialState>() {
        if state.placements.len() > MAX_WINDOW_PLAN_PLACEMENTS {
            return Err(CaptureError::Limit);
        }
        for (key, placement) in &state.placements {
            let SpatialParent::Direct { domain: held, pose } = &placement.parent else {
                continue;
            };
            if *held != domain {
                continue;
            }
            if pose.x_cm.unsigned_abs() > 1_000_000_000 || pose.y_cm.unsigned_abs() > 1_000_000_000
            {
                return Err(CaptureError::Invalid);
            }
            let excluded = mush_sim::deployed::excluded(
                world,
                mush_sim::deployed::Shown::Unit(placement.object),
            );
            rows.push((
                *key,
                placement.object,
                placement.revision,
                pose.x_cm,
                pose.y_cm,
                excluded,
            ));
            if !excluded
                && let Some(cell) = surface.cell_of(PointCm {
                    x: pose.x_cm,
                    y: pose.y_cm,
                })
            {
                taken.insert(cell);
            }
        }
    }
    Ok((
        OccupancyObservation {
            records: rows.len(),
            digest: digest("atlas.movement.window_occupancy.v1", &rows, OCCUPANCY_BYTES)?,
        },
        taken,
    ))
}

#[cfg(test)]
#[path = "movement_window_tests.rs"]
mod tests;
