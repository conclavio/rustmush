//! Strategic radio distance: stationary bodies and real ticketed carrier journeys.
//! Propagation owns scheduling. This read-only projection creates no ship coordinate.
use std::collections::BTreeMap;

use mush_core::{CanonicalHash, CanonicalId, ClockStamp, Ctx, HookReadContext, ObjId, World};
use mush_propagation::routing::{Measurement, Metric, MetricProvider};

use crate::port::families::{PortHandles, Ticket, TicketId};
use crate::{TacsimStore, pack::PackEntry, sky::Sky};

pub const MAX_STRATEGIC_PORTS: usize = 32;
pub const MAX_STRATEGIC_HULLS: usize = 64;

#[derive(Debug)]
pub struct RelayMetric;

/// Freeze a sender's structural address at submission, including aliases that
/// have no typed ticket read context. This does not calculate distance or claim
/// the ship's ticket is valid: the existing routing snapshot still owns that.
#[must_use]
pub fn source_address(world: &World, object: ObjId) -> Option<ObjId> {
    let state = world.module_state::<mush_sim::SpatialState>()?;
    if state.is_quarantined(object) {
        return None;
    }
    let store = world.module_state::<TacsimStore>()?;
    let maps: Vec<_> = store.port_maps().take(MAX_STRATEGIC_PORTS + 1).collect();
    if maps.len() > MAX_STRATEGIC_PORTS {
        return None;
    }
    let hulls: Vec<_> = maps
        .iter()
        .flat_map(|m| &m.hulls)
        .take(MAX_STRATEGIC_HULLS + 1)
        .map(|h| ObjId(h.hull))
        .collect();
    if hulls.len() > MAX_STRATEGIC_HULLS {
        return None;
    }
    let room = mush_propagation::routing::place(world, state, object);
    let start = room
        .and_then(|r| mush_sim::graph::anchored_space_of(world, r))
        .unwrap_or_else(|| {
            if state.is_placed(object) {
                object
            } else {
                room.unwrap_or(object)
            }
        });
    for candidate in std::iter::once(start).chain(mush_sim::graph::ancestors(state, start).ok()?) {
        if state.is_quarantined(candidate) {
            return None;
        }
        if hulls.contains(&candidate) {
            return Some(candidate);
        }
        if world
            .module_state::<mush_sim::JourneyBook>()
            .is_some_and(|b| b.get(candidate).is_some())
        {
            return None;
        }
    }
    room
}

#[derive(Debug, Default)]
struct Snapshot {
    sky: Option<Sky>,
    revision: String,
    at: i64,
    rooms: BTreeMap<ObjId, CanonicalId>,
    areas: BTreeMap<CanonicalId, CanonicalId>,
    hulls: BTreeMap<ObjId, Option<Position>>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct Segment {
    from: CanonicalId,
    to: CanonicalId,
    distance_km: u64,
    elapsed: u32,
    seconds: u32,
}

#[derive(Clone, Debug, serde::Serialize)]
struct Position {
    body: CanonicalId,
    /// Distance to the nearer end of the active frozen leg, rounded up to km.
    offset_km: u64,
    /// Ticket identity and frozen leg inputs, hashed into the flight's basis.
    source: String,
    segment: Option<Segment>,
}

impl MetricProvider for RelayMetric {
    fn snapshot(&self, world: &World, at: &ClockStamp) -> Box<dyn Metric> {
        Box::new(Snapshot::build(world, at, None))
    }

    fn snapshot_in(&self, ctx: &Ctx<'_>, at: &ClockStamp) -> Box<dyn Metric> {
        // The composition injects TacSim's adapter. Its immutable hook view is
        // stamped with TacSim's owner, just like direct registered hook calls;
        // propagation receives only the projection, never a family handle or
        // write delegation over TacSim's tickets.
        let read = ctx.hook_read_context(crate::NS);
        Box::new(Snapshot::build(ctx.w, at, Some(&read)))
    }
}

impl Snapshot {
    fn build(world: &World, at: &ClockStamp, read: Option<&HookReadContext<'_>>) -> Self {
        let mut snapshot = Snapshot {
            at: at.at,
            ..Snapshot::default()
        };
        let Some(store) = world.module_state::<TacsimStore>() else {
            return snapshot;
        };
        let Ok(lane) = CanonicalId::new(crate::definitions::content::LANE_PACK) else {
            return snapshot;
        };
        let Some(version) = store.applied_in(world, &lane) else {
            return snapshot;
        };
        let Some(loaded) = store.pack(&lane, version) else {
            return snapshot;
        };
        // A bound on records read, not merely on the successful output. Oversized
        // maps refuse this metric rather than silently omitting the last port.
        let maps: Vec<_> = store.port_maps().take(MAX_STRATEGIC_PORTS + 1).collect();
        if maps.len() > MAX_STRATEGIC_PORTS {
            return snapshot;
        }
        let mut domains = BTreeMap::new();
        for map in &maps {
            if map.pack != lane {
                continue;
            }
            let Some(PackEntry::Port(port)) = loaded.pack.entry(&map.entry) else {
                continue;
            };
            let Some(PackEntry::Location(location)) = loaded.pack.entry(&port.location) else {
                continue;
            };
            let Some(body) = &location.body else {
                continue;
            };
            snapshot.areas.insert(port.area.clone(), body.clone());
            if let Some(domain) = map.domain {
                domains.insert(mush_sim::DomainId(domain), body.clone());
            }
            for room in [map.host, map.presentation].into_iter().flatten() {
                snapshot.rooms.insert(ObjId(room), body.clone());
            }
        }
        let sky = Sky::from_pack(&loaded.pack, &store.config().time.ephemeris);
        let Ok(revision) = sky.revision() else {
            return Snapshot::default();
        };
        snapshot.revision = revision.to_hex();
        snapshot.sky = Some(sky);
        for row in maps
            .iter()
            .filter(|map| map.pack == lane)
            .flat_map(|map| &map.hulls)
            .take(MAX_STRATEGIC_HULLS + 1)
        {
            if snapshot.hulls.len() >= MAX_STRATEGIC_HULLS {
                return Snapshot::default();
            }
            let hull = ObjId(row.hull);
            let position = if world
                .module_state::<mush_sim::JourneyBook>()
                .is_some_and(|b| b.under_way(hull))
            {
                read.and_then(|read| moving_position(read, store, hull, at.at))
            } else {
                mush_sim::pose_of(world, hull)
                    .and_then(|(domain, _)| domains.get(&domain))
                    .map(|body| Position {
                        body: body.clone(),
                        offset_km: 0,
                        source: format!("docked:{}", hull.0),
                        segment: None,
                    })
            };
            if snapshot.hulls.insert(hull, position).is_some() {
                return Snapshot::default();
            }
        }
        snapshot
    }
    fn body(&self, world: &World, room: ObjId) -> Option<CanonicalId> {
        self.rooms.get(&room).cloned().or_else(|| {
            let atlas = world.module_state::<mush_atlas::AtlasState>()?;
            let geo = atlas.records.roomgeo.get(&room)?;
            if let Some(body) = self.areas.get(&geo.neighborhood.pack) {
                return Some(body.clone());
            }
            // Fixed key lookups, not a global room scan. Checked strategic sites
            // need no invented trade port merely to participate in radio distance.
            let neighborhood = atlas.records.neighborhoods.get(&geo.neighborhood)?;
            let region = atlas.records.regions.get(&neighborhood.region)?;
            let body = match &region.kind {
                mush_atlas::record::RegionKind::Surface { body }
                | mush_atlas::record::RegionKind::Orbital { body } => body,
                _ => return None,
            };
            let body = CanonicalId::new(body.as_str()).ok()?;
            self.sky.as_ref()?.position_km(&body, self.at)?;
            Some(body)
        })
    }

    fn physical_endpoint(&self, world: &World, endpoint: ObjId) -> ObjId {
        crate::port::ship::hull_of_room(world, endpoint).map_or(endpoint, |(v, _)| v.hull())
    }
    fn position(&self, world: &World, endpoint: ObjId) -> Option<Position> {
        let endpoint = self.physical_endpoint(world, endpoint);
        if let Some(position) = self.hulls.get(&endpoint) {
            return position.clone();
        }
        Some(Position {
            body: self.body(world, endpoint)?,
            offset_km: 0,
            source: "body".into(),
            segment: None,
        })
    }
}

impl Metric for Snapshot {
    fn coverage(
        &self,
        world: &World,
        node: &mush_sim::relay::RelayNode,
        endpoint: ObjId,
    ) -> Option<bool> {
        // An arbitrary station mounted inside an unsupported hull gains no magic
        // fixed-room coverage. Only an actually commissioned capital socket works.
        if crate::port::ship::hull_of_room(world, node.place).is_some() {
            let (hull, radius) = crate::relay_mobile::active(world, node.object)?;
            let endpoint = self.physical_endpoint(world, endpoint);
            if hull == endpoint {
                self.hulls.get(&hull)?.as_ref()?;
                return Some(true);
            }
            let a = self.position(world, hull)?;
            let b = self.position(world, endpoint)?;
            return (near_distance(&a, &b)? <= u64::from(radius)).then_some(true);
        }
        Some(node.coverage.contains(&endpoint))
    }
    fn endpoint(
        &self,
        world: &World,
        state: &mush_sim::SpatialState,
        object: ObjId,
    ) -> Option<ObjId> {
        self.sky.as_ref()?;
        if state.is_quarantined(object) {
            return None;
        }
        // Structural ancestry covers nested carried interiors as well as the
        // seven authored Dray rooms. The engine bounds this walk at depth 8.
        let room = mush_propagation::routing::place(world, state, object);
        let start = room
            .and_then(|room| mush_sim::graph::anchored_space_of(world, room))
            .unwrap_or_else(|| {
                if state.is_placed(object) {
                    object
                } else {
                    room.unwrap_or(object)
                }
            });
        let ancestors = mush_sim::graph::ancestors(state, start).ok()?;
        for candidate in std::iter::once(start).chain(ancestors) {
            if state.is_quarantined(candidate) {
                return None;
            }
            if let Some(position) = self.hulls.get(&candidate) {
                return position.as_ref().map(|_| candidate);
            }
            if world
                .module_state::<mush_sim::JourneyBook>()
                .is_some_and(|b| b.get(candidate).is_some())
            {
                return None; // An unsupported or stranded carrier is not local space.
            }
        }
        room
    }

    fn between(&self, world: &World, from: ObjId, to: ObjId) -> Measurement {
        let from = self.physical_endpoint(world, from);
        let to = self.physical_endpoint(world, to);
        let Some(sky) = &self.sky else {
            return Measurement::Unknown;
        };
        if self.hulls.contains_key(&from) || self.hulls.contains_key(&to) {
            let (Some(a), Some(b)) = (self.position(world, from), self.position(world, to)) else {
                return Measurement::Unknown;
            };
            let Some(km) = sky
                .distance_km(&a.body, self.at, &b.body, self.at)
                .and_then(|km| km.checked_add(a.offset_km))
                .and_then(|km| km.checked_add(b.offset_km))
            else {
                return Measurement::Unknown;
            };
            let seconds = if from == to {
                0
            } else {
                // Non-colocated radio is positive even at departure/docking.
                ((u128::from(km) * 1_000).div_ceil(u128::from(crate::sky::LIGHT_M_PER_S))).max(1)
            };
            let Ok(seconds) = i64::try_from(seconds) else {
                return Measurement::Unknown;
            };
            let Ok(digest) = CanonicalHash::for_canonical_value(
                "tacsim.relay_metric.v1",
                1,
                &serde_json::json!({"sky":self.revision,"at":self.at,"from":from,"to":to,"a":a,"b":b}),
                4096,
            ) else {
                return Measurement::Unknown;
            };
            return Measurement::Known {
                seconds,
                basis: format!("ship.v1:{}", digest.to_hex()),
            };
        }
        match (self.body(world, from), self.body(world, to)) {
            (Some(a), Some(b)) if a != b => sky
                .light_lag_s(&a, self.at, &b, self.at)
                .and_then(|s| i64::try_from(s).ok())
                .map_or(Measurement::Unknown, |seconds| Measurement::Known {
                    seconds,
                    basis: format!("sky.v1:{}:{a}:{b}:{}", self.revision, self.at),
                }),
            // The stationary strategic model places a port at its body. Within
            // that body, retain the existing local metric when it resolves;
            // otherwise round non-colocated short radio legs up to one IC second.
            (Some(a), Some(_)) => {
                let seconds = world
                    .module_state::<mush_sim::SpatialState>()
                    .and_then(|state| {
                        mush_propagation::PropagationContext::build(
                            state,
                            world,
                            from,
                            to,
                            ClockStamp {
                                clock: world.world_ic_now().clock,
                                at: self.at,
                            },
                        )
                    })
                    .map_or(i64::from(from != to), |context| context.time_of_flight());
                Measurement::Known {
                    seconds,
                    basis: format!("sky.body-local.v1:{a}"),
                }
            }
            (None, None) => Measurement::Local,
            _ => Measurement::Unknown,
        }
    }
}

/// The active cursor and the frozen ticket must describe the SAME passage.
/// A deferred journey tick holds at its current endpoint until the cursor moves;
/// radio does not advance the carrier or consume its next leg on its behalf.
fn moving_position(
    read: &HookReadContext<'_>,
    store: &TacsimStore,
    hull: ObjId,
    at: i64,
) -> Option<Position> {
    let world = read.world();
    let journey = world.module_state::<mush_sim::JourneyBook>()?.get(hull)?;
    let crate::port::VoyageState::Present(voyage) = store.voyage_state(hull) else {
        return None;
    };
    let id = TicketId {
        hull,
        seq: voyage.last_seq,
    };
    let handles = world.module_state::<PortHandles>()?;
    let ticket = read.action_state(&handles.families.tickets, &id).ok()??;
    if ticket.ticket_id() != id
        || ticket.check().is_err()
        || journey.route().id() != &ticket.route
        || journey.departed().at != ticket.plan.depart_ic
        || journey.route().legs().len() != ticket.plan.legs.len()
        || journey.leg_times().is_none_or(|times| {
            times
                .iter()
                .copied()
                .ne(ticket.plan.legs.iter().map(|leg| leg.secs))
        })
    {
        return None;
    }
    ticket_position(store, ticket, usize::try_from(journey.cursor()).ok()?, at)
}

fn ticket_position(
    store: &TacsimStore,
    ticket: &Ticket,
    cursor: usize,
    at: i64,
) -> Option<Position> {
    let loaded = store.pack(&ticket.route_pack, ticket.route_version)?;
    let PackEntry::Route(route) = loaded.pack.entry(&ticket.route)? else {
        return None;
    };
    let leg = ticket.plan.legs.get(cursor)?;
    if at < leg.t0 || route.legs.len() != ticket.plan.legs.len() {
        return None;
    }
    for (authored, frozen) in route.legs.iter().zip(&ticket.plan.legs) {
        let PackEntry::Location(location) = loaded.pack.entry(&authored.to)? else {
            return None;
        };
        if authored.to != frozen.to || location.body.as_ref() != Some(&frozen.body) {
            return None;
        }
    }
    let from = if cursor == 0 {
        let PackEntry::Location(location) = loaded.pack.entry(&route.from)? else {
            return None;
        };
        location.body.as_ref()?
    } else {
        &ticket.plan.legs.get(cursor - 1)?.body
    };
    let elapsed = at.checked_sub(leg.t0)?.clamp(0, i64::from(leg.secs)) as u64;
    let (body, part) = if elapsed * 2 <= u64::from(leg.secs) {
        (from, elapsed)
    } else {
        (&leg.body, u64::from(leg.secs) - elapsed)
    };
    let offset_km =
        u64::try_from((u128::from(leg.dist_km) * u128::from(part)).div_ceil(u128::from(leg.secs)))
            .ok()?;
    Some(Position {
        body: body.clone(),
        offset_km,
        segment: Some(Segment {
            from: from.clone(),
            to: leg.body.clone(),
            distance_km: leg.dist_km,
            elapsed: u32::try_from(elapsed).ok()?,
            seconds: leg.secs,
        }),
        source: format!(
            "{}:{}/{cursor}:{}/{}/{}/{}/{}",
            ticket.hull.0,
            ticket.seq,
            ticket.route_pack,
            ticket.route_version,
            leg.dist_km,
            elapsed,
            leg.secs
        ),
    })
}

/// Aboard-only estimate to a PUBLIC live Exchange endpoint. This is one-way
/// physical time, not queue truth, delivery confirmation or a remote ACK.
pub(crate) fn lag_line(ctx: &Ctx<'_>, hull: ObjId) -> String {
    use mush_sim::relay::{ClassPolicy, LinkState};
    let snapshot = RelayMetric.snapshot_in(ctx, &ctx.w.world_ic_now());
    if matches!(snapshot.between(ctx.w, hull, hull), Measurement::Unknown) {
        return "Radio: physical lag unavailable; traffic waits for a known endpoint.".into();
    }
    let lag = mush_propagation::routing::registry(ctx.w).and_then(|registry| {
        registry
            .pairs
            .values()
            .filter(|p| p.class_policy == ClassPolicy::Exchange && p.state() == LinkState::Live)
            .flat_map(|p| p.halves.iter())
            .filter_map(|half| registry.nodes.get(&half.node))
            .filter_map(|node| match snapshot.between(ctx.w, hull, node.place) {
                Measurement::Known { seconds, .. } => Some(seconds),
                _ => None,
            })
            .min()
    });
    lag.map_or_else(|| "Radio: no live Exchange endpoint; conventional radio remains available.".into(),
        |s| format!("Radio: about {s} IC seconds one way to an Exchange endpoint; queue time and returning acknowledgements are additional."))
}

/// Only shared frozen edges or their endpoints establish local proximity. Two
/// ships near different edges' nearer bodies are not assumed to share a position.
fn near_distance(a: &Position, b: &Position) -> Option<u64> {
    match (&a.segment, &b.segment) {
        (None, None) if a.body == b.body => Some(0),
        (Some(x), None) if b.body == x.from => u64::try_from(
            (u128::from(x.distance_km) * u128::from(x.elapsed)).div_ceil(u128::from(x.seconds)),
        )
        .ok(),
        (Some(x), None) if b.body == x.to => u64::try_from(
            (u128::from(x.distance_km) * u128::from(x.seconds - x.elapsed))
                .div_ceil(u128::from(x.seconds)),
        )
        .ok(),
        (None, Some(_)) => near_distance(b, a),
        (Some(x), Some(y))
            if x.distance_km == y.distance_km && x.from == y.from && x.to == y.to =>
        {
            segment_separation(x, y, false)
        }
        (Some(x), Some(y))
            if x.distance_km == y.distance_km && x.from == y.to && x.to == y.from =>
        {
            segment_separation(x, y, true)
        }
        _ => None,
    }
}

fn segment_separation(x: &Segment, y: &Segment, reverse: bool) -> Option<u64> {
    let a = u128::from(x.distance_km)
        .checked_mul(u128::from(x.elapsed))?
        .checked_mul(u128::from(y.seconds))?;
    let elapsed = if reverse {
        y.seconds.checked_sub(y.elapsed)?
    } else {
        y.elapsed
    };
    let b = u128::from(y.distance_km)
        .checked_mul(u128::from(elapsed))?
        .checked_mul(u128::from(x.seconds))?;
    let denominator = u128::from(x.seconds).checked_mul(u128::from(y.seconds))?;
    (denominator != 0)
        .then(|| u64::try_from(a.abs_diff(b).div_ceil(denominator)).ok())
        .flatten()
}

#[cfg(test)]
mod mobile_coverage_tests {
    use super::*;
    fn position(from: &str, to: &str, distance: u64, elapsed: u32, seconds: u32) -> Position {
        Position {
            body: CanonicalId::new(from).unwrap(),
            offset_km: 0,
            source: "test".into(),
            segment: Some(Segment {
                from: CanonicalId::new(from).unwrap(),
                to: CanonicalId::new(to).unwrap(),
                distance_km: distance,
                elapsed,
                seconds,
            }),
        }
    }
    fn body(name: &str) -> Position {
        Position {
            body: CanonicalId::new(name).unwrap(),
            offset_km: 0,
            source: "test".into(),
            segment: None,
        }
    }
    #[test]
    fn radius_rounds_outward_at_both_ends_and_between_carriers() {
        let a = position("a", "b", 600_001, 1, 2);
        assert_eq!(near_distance(&a, &body("a")), Some(300_001));
        assert_eq!(near_distance(&a, &body("b")), Some(300_001));
        assert_eq!(
            near_distance(&a, &position("a", "b", 600_001, 0, 2)),
            Some(300_001)
        );
        assert_eq!(
            near_distance(&a, &position("b", "a", 600_001, 1, 2)),
            Some(0)
        );
        assert_eq!(
            near_distance(&a, &position("b", "a", 600_001, 0, 2)),
            Some(300_001)
        );
        assert_eq!(near_distance(&a, &a), Some(0));
    }
    #[test]
    fn unrelated_edges_and_inconsistent_frozen_distances_have_no_nearby_claim() {
        let a = position("a", "b", 100, 1, 2);
        assert_eq!(near_distance(&a, &position("a", "c", 100, 1, 2)), None);
        assert_eq!(near_distance(&a, &position("a", "b", 101, 1, 2)), None);
        assert_eq!(near_distance(&a, &body("c")), None);
    }
}
