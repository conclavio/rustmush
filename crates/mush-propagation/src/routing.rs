//! R2: bounded stationary routes, frozen physical legs and shared FIFO capacity.
//! The delivery pass remains the sole scheduler. All mutable progress is persisted
//! with the envelope; the capacity post-image shares its commit boundary.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use mush_core::{CanonicalId, ClockStamp, ObjId, ObjKind, World};
use mush_sim::SpatialState;
use mush_sim::relay::{ClassPolicy, LinkState, MAX_NODES, MAX_PAIRS, RelayRegistry, RelayStore};
use serde::{Deserialize, Serialize};

use crate::record::{Envelope, EnvelopeState, MAX_ENVELOPE_HOPS, PropagationStore};

pub const STATE_KEY: &str = "relay_transport";
pub const MAX_LINK_HOPS: usize = MAX_ENVELOPE_HOPS as usize;
pub const MAX_ROUTE_LEGS: usize = MAX_LINK_HOPS + 2;
pub const MAX_PENDING_ROUTES: usize = 128;
pub const MAX_LINK_QUEUE: usize = 64;
pub const MAX_FORWARDINGS: u8 = 4;
pub const MAX_PHYSICAL_LEGS: u8 = 8;
pub const BYTES_PER_UNIT: usize = 128;

/// A game supplies astronomical distances without adding a game dependency here.
pub trait MetricProvider: std::fmt::Debug + Send + Sync {
    fn snapshot(&self, world: &World, at: &ClockStamp) -> Box<dyn Metric>;
}

pub trait Metric: std::fmt::Debug {
    fn between(&self, world: &World, from: ObjId, to: ObjId) -> Measurement;
}

#[derive(Clone, Debug)]
pub enum Measurement {
    /// Neither endpoint belongs to the strategic model; use local physics.
    Local,
    /// A strategic endpoint is missing or unavailable; never assume zero lag.
    Unknown,
    Known {
        seconds: i64,
        basis: String,
    },
}

#[derive(Clone, Debug)]
pub struct RoutingServices(pub Arc<dyn MetricProvider>);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Leg {
    pub from: ObjId,
    pub to: ObjId,
    pub pair: Option<CanonicalId>,
    pub relay: Option<ObjId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flight {
    pub started: ClockStamp,
    pub due: ClockStamp,
    pub seconds: i64,
    pub basis: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletedLeg {
    pub leg: Leg,
    pub flight: Flight,
    pub arrived: ClockStamp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transit {
    pub version: u16,
    pub registry_revision: u64,
    pub position: ObjId,
    pub destination: ObjId,
    pub legs: Vec<Leg>,
    pub next: usize,
    pub flight: Option<Flight>,
    pub queued_at: Option<ClockStamp>,
    pub forwardings: u8,
    pub physical_legs: u8,
    pub history: Vec<CompletedLeg>,
    pub last_work: Work,
}

impl Transit {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.registry_revision == 0
            || self.legs.is_empty()
            || self.legs.len() > MAX_ROUTE_LEGS
            || self.next > self.legs.len()
            || self.forwardings > MAX_FORWARDINGS
            || self.physical_legs > MAX_PHYSICAL_LEGS
            || self.history.len() > usize::from(self.physical_legs)
            || self.history.iter().any(|h| {
                h.leg.pair.is_some()
                    || h.flight.seconds < 0
                    || h.flight.basis.len() > 256
                    || h.flight.started.clock != h.flight.due.clock
                    || h.arrived.clock != h.flight.due.clock
                    || h.arrived.at < h.flight.due.at
                    || h.flight.started.at.checked_add(h.flight.seconds) != Some(h.flight.due.at)
            })
            || self.legs.iter().filter(|l| l.pair.is_some()).count() > MAX_LINK_HOPS
            || self
                .legs
                .iter()
                .any(|l| l.pair.is_some() != l.relay.is_some())
            || self.legs.windows(2).any(|p| p[0].to != p[1].from)
            || self.legs.last().is_none_or(|l| l.to != self.destination)
            || self.last_work.metric_reads > 2 * MAX_NODES + 1
            || self.last_work.edge_visits > 2 * MAX_PAIRS * MAX_LINK_HOPS
        {
            return Err("invalid or over-bound relay itinerary".into());
        }
        if self
            .legs
            .get(self.next)
            .is_some_and(|l| l.from != self.position)
            || (self.next == self.legs.len() && self.position != self.destination)
        {
            return Err("itinerary cursor contradicts its position".into());
        }
        if let Some(flight) = &self.flight
            && (self.legs.get(self.next).is_none_or(|l| l.pair.is_some())
                || self.queued_at.is_some()
                || flight.seconds < 0
                || flight.basis.len() > 256
                || flight.started.clock != flight.due.clock
                || flight.started.at.checked_add(flight.seconds) != Some(flight.due.at))
        {
            return Err("invalid frozen physical leg".into());
        }
        if self.queued_at.is_some() && self.legs.get(self.next).is_none_or(|l| l.pair.is_none()) {
            return Err("queue ticket does not name a link leg".into());
        }
        Ok(())
    }

    pub fn reanchor(&mut self, clock: &mush_core::ClockRef) {
        if let Some(flight) = &mut self.flight {
            flight.started.clock = clock.clone();
            flight.due.clock = clock.clone();
        }
        if let Some(at) = &mut self.queued_at {
            at.clock = clock.clone();
        }
        for h in &mut self.history {
            h.flight.started.clock = clock.clone();
            h.flight.due.clock = clock.clone();
            h.arrived.clock = clock.clone();
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Work {
    pub metric_reads: usize,
    pub edge_visits: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capacity {
    pub at: i64,
    pub units: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportState {
    pub version: u16,
    pub capacity: BTreeMap<CanonicalId, Capacity>,
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            version: 1,
            capacity: BTreeMap::new(),
        }
    }
}

impl TransportState {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.capacity.len() > MAX_PAIRS
            || self.capacity.values().any(|b| b.units > 1_000_000)
        {
            return Err("invalid relay transport capacity state".into());
        }
        Ok(())
    }

    fn consume(&mut self, pair: &CanonicalId, now: i64, units: u32, limit: u32) -> bool {
        if !self.capacity.contains_key(pair) && self.capacity.len() >= MAX_PAIRS {
            return false;
        }
        let bucket = self
            .capacity
            .entry(pair.clone())
            .or_insert(Capacity { at: now, units: 0 });
        if now < bucket.at {
            return false;
        }
        if now > bucket.at {
            *bucket = Capacity { at: now, units: 0 };
        }
        if bucket.units.saturating_add(units) > limit {
            return false;
        }
        bucket.units += units;
        true
    }
}

pub fn registry(world: &World) -> Option<&RelayRegistry> {
    world.module_state::<RelayStore>()?.registry()
}

pub fn enabled(world: &World) -> bool {
    world.module_state::<RoutingServices>().is_some() && registry(world).is_some()
}

pub fn place(world: &World, state: &SpatialState, object: ObjId) -> Option<ObjId> {
    let room = if world.get(object)?.kind == ObjKind::Room {
        object
    } else {
        crate::context::place_of(state, world, object)?
    };
    (room != state.recovery_room
        && room != state.in_transit_room
        && world.get(room).is_some_and(|o| o.kind == ObjKind::Room))
    .then_some(room)
}

fn measure(
    metric: &dyn Metric,
    world: &World,
    state: &SpatialState,
    from: ObjId,
    to: ObjId,
    at: &ClockStamp,
) -> Option<(i64, String)> {
    match metric.between(world, from, to) {
        Measurement::Known { seconds, basis } if seconds >= 0 && basis.len() <= 256 => {
            Some((seconds, basis))
        }
        Measurement::Local => {
            let context =
                crate::context::PropagationContext::build(state, world, from, to, at.clone())?;
            Some((context.time_of_flight(), "local.medium.v1".into()))
        }
        _ => None,
    }
}

#[derive(Clone, Debug)]
struct Path {
    cost: i64,
    nodes: Vec<CanonicalId>,
    pairs: Vec<CanonicalId>,
}

fn rank(path: &Path) -> (i64, &[CanonicalId], &[CanonicalId]) {
    (path.cost, &path.pairs, &path.nodes)
}

/// Layered bounded graph walk. Even a miss visits the same bounded edge table;
/// cycles cannot extend a path, and a hit does not stop the scan early.
#[allow(clippy::too_many_arguments)]
pub fn plan(
    world: &World,
    state: &SpatialState,
    metric: &dyn Metric,
    from: ObjId,
    to: ObjId,
    at: &ClockStamp,
    used_hops: usize,
    units: u32,
) -> (Option<Transit>, Work) {
    let Some(catalog) = registry(world) else {
        return (None, Work::default());
    };
    plan_catalog(
        catalog, world, state, metric, from, to, at, used_hops, units,
    )
}

#[allow(clippy::too_many_arguments)]
fn plan_catalog(
    catalog: &RelayRegistry,
    world: &World,
    state: &SpatialState,
    metric: &dyn Metric,
    from: ObjId,
    to: ObjId,
    at: &ClockStamp,
    used_hops: usize,
    units: u32,
) -> (Option<Transit>, Work) {
    let mut work = Work {
        metric_reads: 1,
        edge_visits: 0,
    };
    let direct = measure(metric, world, state, from, to, at);
    let mut layers = BTreeMap::<CanonicalId, Path>::new();
    let mut ends = BTreeMap::new();
    for (id, node) in &catalog.nodes {
        let ingress = measure(metric, world, state, from, node.place, at);
        let egress = measure(metric, world, state, node.place, to, at);
        work.metric_reads += 2;
        let ingress = if node.coverage.contains(&from) {
            Some(0)
        } else {
            ingress.map(|v| v.0)
        };
        let egress = if node.coverage.contains(&to) {
            Some(0)
        } else {
            egress.map(|v| v.0)
        };
        if let Some(cost) = ingress {
            layers.insert(
                id.clone(),
                Path {
                    cost,
                    nodes: vec![id.clone()],
                    pairs: vec![],
                },
            );
        }
        ends.insert(id.clone(), egress);
    }
    let mut best: Option<Path> = None;
    for depth in 0..MAX_LINK_HOPS {
        let mut next = BTreeMap::<CanonicalId, Path>::new();
        for (pair_id, pair) in &catalog.pairs {
            for direction in 0..2 {
                work.edge_visits += 1;
                let a = &pair.halves[direction].node;
                let b = &pair.halves[1 - direction].node;
                if depth + used_hops >= MAX_LINK_HOPS
                    || pair.state() != LinkState::Live
                    || pair.class_policy != ClassPolicy::Exchange
                    || pair.capacity_units_per_ic_second < units
                {
                    continue;
                }
                let Some(path) = layers.get(a) else {
                    continue;
                };
                if path.nodes.contains(b) {
                    continue;
                }
                let mut candidate = path.clone();
                candidate.nodes.push(b.clone());
                candidate.pairs.push(pair_id.clone());
                if next.get(b).is_none_or(|old| rank(&candidate) < rank(old)) {
                    next.insert(b.clone(), candidate);
                }
            }
        }
        for (node, path) in &next {
            let Some(Some(end)) = ends.get(node) else {
                continue;
            };
            let Some(cost) = path.cost.checked_add(*end) else {
                continue;
            };
            let mut candidate = path.clone();
            candidate.cost = cost;
            if best.as_ref().is_none_or(|old| {
                (
                    candidate.cost,
                    candidate.pairs.len(),
                    &candidate.pairs,
                    &candidate.nodes,
                ) < (old.cost, old.pairs.len(), &old.pairs, &old.nodes)
            }) {
                best = Some(candidate);
            }
        }
        layers = next;
    }
    let use_pair = best
        .as_ref()
        .is_some_and(|p| direct.as_ref().is_none_or(|d| p.cost < d.0));
    let legs = if use_pair {
        let path = best.expect("selected path");
        let mut legs = Vec::new();
        let first = &catalog.nodes[&path.nodes[0]];
        legs.push(Leg {
            from,
            to: first.place,
            pair: None,
            relay: None,
        });
        for (i, pair) in path.pairs.iter().enumerate() {
            let a = &catalog.nodes[&path.nodes[i]];
            let b = &catalog.nodes[&path.nodes[i + 1]];
            legs.push(Leg {
                from: a.place,
                to: b.place,
                pair: Some(pair.clone()),
                relay: Some(b.object),
            });
        }
        let last = &catalog.nodes[path.nodes.last().expect("path node")];
        legs.push(Leg {
            from: last.place,
            to,
            pair: None,
            relay: None,
        });
        legs
    } else if direct.is_some() {
        vec![Leg {
            from,
            to,
            pair: None,
            relay: None,
        }]
    } else {
        return (None, work);
    };
    (
        Some(Transit {
            version: 1,
            registry_revision: catalog.revision,
            position: from,
            destination: to,
            legs,
            next: 0,
            flight: None,
            queued_at: None,
            forwardings: 0,
            physical_legs: 0,
            history: vec![],
            last_work: work,
        }),
        work,
    )
}

pub fn admission(
    store: &PropagationStore,
    route: Option<&Transit>,
    existing: Option<u64>,
) -> Result<(), String> {
    let pending: Vec<_> = store
        .envelopes()
        .filter(|e| {
            Some(e.id()) != existing
                && matches!(
                    e.state(),
                    EnvelopeState::Queued | EnvelopeState::Transmitted
                )
        })
        .collect();
    if existing.is_none() && pending.len() >= MAX_PENDING_ROUTES {
        return Err("Transport pending queue is full; try again later.".into());
    }
    if let Some(route) = route {
        for pair in route.legs.iter().filter_map(|l| l.pair.as_ref()) {
            let count = pending
                .iter()
                .filter(|e| {
                    e.transit().is_some_and(|r| {
                        r.legs
                            .iter()
                            .skip(r.next)
                            .any(|l| l.pair.as_ref() == Some(pair))
                    })
                })
                .count();
            if count >= MAX_LINK_QUEUE {
                return Err("Transport link queue is full; try again later.".into());
            }
        }
    }
    Ok(())
}

pub fn prepare(
    world: &World,
    state: &SpatialState,
    envelope: &Envelope,
    now: &ClockStamp,
) -> Option<Transit> {
    let services = world.module_state::<RoutingServices>()?;
    let metric = services.0.snapshot(world, now);
    plan(
        world,
        state,
        metric.as_ref(),
        place(
            world,
            state,
            envelope
                .path()
                .last()
                .map_or(envelope.issuer(), |h| h.relay),
        )?,
        place(world, state, envelope.recipient())?,
        now,
        envelope.path().len(),
        message_units(envelope),
    )
    .0
}

fn message_units(envelope: &Envelope) -> u32 {
    envelope.payload().len().div_ceil(BYTES_PER_UNIT) as u32
}

pub struct Pass<'a> {
    pub world: &'a World,
    pub state: &'a SpatialState,
    pub metric: Box<dyn Metric>,
    queues: BTreeMap<CanonicalId, VecDeque<u64>>,
    jamming: BTreeMap<ObjId, u32>,
}

impl<'a> Pass<'a> {
    pub fn new(
        world: &'a World,
        state: &'a SpatialState,
        store: &PropagationStore,
        now: &ClockStamp,
    ) -> Option<Self> {
        if !enabled(world) {
            return None;
        }
        let mut rows = Vec::new();
        for e in store.envelopes().filter(|e| {
            matches!(
                e.state(),
                EnvelopeState::Queued | EnvelopeState::Transmitted
            )
        }) {
            if let Some(r) = e.transit()
                && let Some(at) = &r.queued_at
                && let Some(pair) = r.legs.get(r.next).and_then(|l| l.pair.as_ref())
            {
                rows.push((at.at, e.id(), pair.clone()));
            }
        }
        rows.sort();
        let mut queues = BTreeMap::<CanonicalId, VecDeque<u64>>::new();
        for (_, id, pair) in rows {
            queues.entry(pair).or_default().push_back(id);
        }
        Some(Self {
            world,
            state,
            metric: world
                .module_state::<RoutingServices>()?
                .0
                .snapshot(world, now),
            queues,
            jamming: store.jamming_map(),
        })
    }

    /// Collapse zero-time legs under a fixed transition budget. A positive-time
    /// leg or a new FIFO ticket ends the fire and must commit before completion.
    pub fn step(
        &mut self,
        store: &mut PropagationStore,
        envelope: &Envelope,
        now: &ClockStamp,
    ) -> Option<Envelope> {
        let mut current = envelope.clone();
        for _ in 0..(2 * MAX_ROUTE_LEGS + 2) {
            let Some(next) = self.step_once(store, &current, now) else {
                break;
            };
            let queued = next.transit().is_some_and(|r| r.queued_at.is_some())
                && current.transit().is_none_or(|r| r.queued_at.is_none());
            let in_flight = next
                .transit()
                .and_then(|r| r.flight.as_ref())
                .is_some_and(|f| f.due.at > now.at);
            current = next;
            if queued
                || in_flight
                || !matches!(
                    current.state(),
                    EnvelopeState::Queued | EnvelopeState::Transmitted
                )
            {
                break;
            }
        }
        (current != *envelope).then_some(current)
    }

    fn step_once(
        &mut self,
        store: &mut PropagationStore,
        envelope: &Envelope,
        now: &ClockStamp,
    ) -> Option<Envelope> {
        if !matches!(
            envelope.state(),
            EnvelopeState::Queued | EnvelopeState::Transmitted
        ) {
            return None;
        }
        let catalog = registry(self.world)?;
        let mut route = match envelope.transit() {
            Some(route) => route.clone(),
            None => {
                let from = place(
                    self.world,
                    self.state,
                    envelope
                        .path()
                        .last()
                        .map_or(envelope.issuer(), |h| h.relay),
                )?;
                let to = place(self.world, self.state, envelope.recipient())?;
                let route = plan(
                    self.world,
                    self.state,
                    self.metric.as_ref(),
                    from,
                    to,
                    now,
                    envelope.path().len(),
                    message_units(envelope),
                )
                .0?;
                if admission(store, Some(&route), Some(envelope.id())).is_err() {
                    return None;
                }
                return Some(envelope.with_transit(route));
            }
        };
        // Finish an already transmitted leg using only its frozen clock facts.
        if let Some(flight) = &route.flight {
            if now.clock != flight.due.clock || now.at < flight.due.at {
                return None;
            }
            let leg = route.legs.get(route.next)?;
            if self.jamming.get(&leg.to).copied().unwrap_or(0) >= crate::context::JAMMING_BLOCKS_AT
            {
                return None;
            }
            route.history.push(CompletedLeg {
                leg: leg.clone(),
                flight: flight.clone(),
                arrived: now.clone(),
            });
            route.position = leg.to;
            route.next += 1;
            route.flight = None;
            return Some(envelope.with_transit(route));
        }
        let destination = place(self.world, self.state, envelope.recipient())?;
        if route.destination != destination || route.registry_revision != catalog.revision {
            if route.destination != destination && route.forwardings >= MAX_FORWARDINGS {
                return Some(envelope.in_state(EnvelopeState::Expired, now.clone()));
            }
            let mut revised = plan(
                self.world,
                self.state,
                self.metric.as_ref(),
                route.position,
                destination,
                now,
                envelope.path().len(),
                message_units(envelope),
            )
            .0?;
            if admission(store, Some(&revised), Some(envelope.id())).is_err() {
                return None;
            }
            revised.forwardings = route.forwardings + u8::from(route.destination != destination);
            revised.physical_legs = route.physical_legs;
            revised.history = route.history;
            return Some(envelope.with_transit(revised));
        }
        let Some(leg) = route.legs.get(route.next).cloned() else {
            return Some(envelope.in_state(EnvelopeState::Delivered, now.clone()));
        };
        if let Some(pair_id) = &leg.pair {
            let pair = catalog.pairs.get(pair_id)?;
            if pair.state() != LinkState::Live || pair.class_policy != ClassPolicy::Exchange {
                return None;
            }
            if route.queued_at.is_none() {
                if self
                    .queues
                    .get(pair_id)
                    .is_some_and(|q| q.len() >= MAX_LINK_QUEUE)
                {
                    return None;
                }
                route.queued_at = Some(now.clone());
                return Some(envelope.with_transit(route));
            }
            let queue = self.queues.get_mut(pair_id)?;
            if queue.front() != Some(&envelope.id()) {
                return None;
            }
            if self.jamming.get(&leg.to).copied().unwrap_or(0) >= crate::context::JAMMING_BLOCKS_AT
            {
                return None;
            }
            let units = message_units(envelope);
            if !store
                .transport
                .consume(pair_id, now.at, units, pair.capacity_units_per_ic_second)
            {
                return None;
            }
            let mut next = match envelope.relayed(leg.relay?, now.clone()) {
                Ok(next) => next,
                Err(_) => return Some(envelope.in_state(EnvelopeState::Expired, now.clone())),
            };
            queue.pop_front();
            route.position = leg.to;
            route.next += 1;
            route.queued_at = None;
            next = next.with_transit(route);
            Some(next)
        } else {
            if route.physical_legs >= MAX_PHYSICAL_LEGS {
                return Some(envelope.in_state(EnvelopeState::Expired, now.clone()));
            }
            let covered = catalog.nodes.values().any(|n| {
                (n.place == leg.to && n.coverage.contains(&leg.from))
                    || (n.place == leg.from && n.coverage.contains(&leg.to))
            });
            let (seconds, basis) = if covered {
                (0, "relay.coverage.v1".into())
            } else {
                measure(
                    self.metric.as_ref(),
                    self.world,
                    self.state,
                    leg.from,
                    leg.to,
                    now,
                )?
            };
            let due = ClockStamp {
                clock: now.clock.clone(),
                at: now.at.checked_add(seconds)?,
            };
            route.flight = Some(Flight {
                started: now.clone(),
                due,
                seconds,
                basis,
            });
            route.physical_legs += 1;
            Some(
                envelope
                    .in_state(EnvelopeState::Transmitted, now.clone())
                    .with_transit(route),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::ClockRef;
    use mush_sim::relay::{RelayHalf, RelayNode, RelayPair};

    fn cid(s: &str) -> CanonicalId {
        CanonicalId::new(s).unwrap()
    }
    fn clock(at: i64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::Utc,
            at,
        }
    }

    #[derive(Debug)]
    struct TestMetric {
        end: ObjId,
    }
    impl Metric for TestMetric {
        fn between(&self, _: &World, from: ObjId, to: ObjId) -> Measurement {
            if (from == ObjId(100) && to == ObjId(10)) || (from == self.end && to == ObjId(200)) {
                Measurement::Known {
                    seconds: 2,
                    basis: "test.sky".into(),
                }
            } else {
                Measurement::Unknown
            }
        }
    }

    fn catalog() -> RelayRegistry {
        let operator = serde_json::from_value(serde_json::json!(0)).unwrap();
        let mut nodes = BTreeMap::new();
        for (i, name) in ["a", "b", "c", "unreachable"].into_iter().enumerate() {
            let place = ObjId((i as u32 + 1) * 10);
            nodes.insert(
                cid(name),
                RelayNode {
                    object: ObjId(place.0 + 1),
                    place,
                    label: name.into(),
                    operator,
                    coverage: vec![place],
                },
            );
        }
        let mut pairs = BTreeMap::new();
        for (i, (id, a, b)) in [("ab", "a", "b"), ("bc", "b", "c"), ("ca", "c", "a")]
            .into_iter()
            .enumerate()
        {
            pairs.insert(
                cid(id),
                RelayPair {
                    capacity_units_per_ic_second: 8,
                    class_policy: ClassPolicy::Exchange,
                    halves: [a, b].map(|node| RelayHalf {
                        id: cid(&format!("{id}.{node}")),
                        object: ObjId(300 + i as u32 * 2 + u32::from(node == b)),
                        node: cid(node),
                        enabled: true,
                        powered: true,
                        staffed: true,
                        destroyed: false,
                    }),
                },
            );
        }
        RelayRegistry {
            version: 1,
            revision: 1,
            source_pack: cid("test.relay"),
            source_entry: cid("network"),
            operator,
            nodes,
            pairs,
        }
    }

    #[test]
    fn cycle_hit_and_miss_visit_the_same_bounded_graph_and_respect_remaining_hops() {
        let world = World::default();
        let spatial = SpatialState::default();
        let at = clock(0);
        let mut registry = catalog();
        registry.validate().unwrap();
        let (hit, hit_work) = plan_catalog(
            &registry,
            &world,
            &spatial,
            &TestMetric { end: ObjId(30) },
            ObjId(100),
            ObjId(200),
            &at,
            0,
            1,
        );
        let (miss, miss_work) = plan_catalog(
            &registry,
            &world,
            &spatial,
            &TestMetric { end: ObjId(40) },
            ObjId(100),
            ObjId(200),
            &at,
            0,
            1,
        );
        let hit = hit.unwrap();
        hit.validate().unwrap();
        assert!(miss.is_none());
        assert_eq!(hit_work, miss_work);
        assert_eq!(hit_work.metric_reads, 9);
        assert_eq!(hit_work.edge_visits, 24);
        assert_eq!(hit.legs.iter().filter(|l| l.pair.is_some()).count(), 1);
        registry.pairs.get_mut(&cid("ca")).unwrap().class_policy = ClassPolicy::Private;
        let (around, _) = plan_catalog(
            &registry,
            &world,
            &spatial,
            &TestMetric { end: ObjId(30) },
            ObjId(100),
            ObjId(200),
            &at,
            0,
            1,
        );
        assert_eq!(
            around
                .unwrap()
                .legs
                .iter()
                .filter(|l| l.pair.is_some())
                .count(),
            2
        );
        let (bounded, _) = plan_catalog(
            &registry,
            &world,
            &spatial,
            &TestMetric { end: ObjId(30) },
            ObjId(100),
            ObjId(200),
            &at,
            MAX_LINK_HOPS - 1,
            1,
        );
        assert!(bounded.is_none());
    }

    #[test]
    fn corrupt_frozen_time_and_cursor_are_refused_and_generation_change_preserves_due_scalar() {
        let route = plan_catalog(
            &catalog(),
            &World::default(),
            &SpatialState::default(),
            &TestMetric { end: ObjId(30) },
            ObjId(100),
            ObjId(200),
            &clock(0),
            0,
            1,
        )
        .0
        .unwrap();
        let mut invalid = route.clone();
        invalid.next = MAX_ROUTE_LEGS + 1;
        assert!(invalid.validate().is_err());
        let mut flight = route;
        flight.physical_legs = 1;
        flight.flight = Some(Flight {
            started: clock(10),
            due: clock(12),
            seconds: 2,
            basis: "test".into(),
        });
        flight.validate().unwrap();
        flight.reanchor(&ClockRef::WorldIc { generation: 2 });
        assert_eq!(flight.flight.as_ref().unwrap().due.at, 12);
        assert_eq!(flight.flight.as_ref().unwrap().started.at, 10);
        flight.validate().unwrap();
        flight.flight.as_mut().unwrap().due.at = 11;
        assert!(flight.validate().is_err());
    }

    #[test]
    fn legacy_dispatch_kind_is_inferred_and_reports_or_messages_are_not_standing_orders() {
        use crate::record::DispatchKind;
        let order = Envelope::issue(1, ObjId(2), ObjId(3), "hold".into(), 1, clock(0))
            .unwrap()
            .in_state(EnvelopeState::Delivered, clock(1));
        let mut legacy = serde_json::to_value(&order).unwrap();
        legacy.as_object_mut().unwrap().remove("kind");
        assert_eq!(
            serde_json::from_value::<Envelope>(legacy).unwrap().kind(),
            DispatchKind::Order
        );
        let mut mail = serde_json::to_value(&order).unwrap();
        mail["id"] = 2.into();
        mail["kind"] = "mail".into();
        mail["revision"] = 99.into();
        let mail: Envelope = serde_json::from_value(mail).unwrap();
        let mut store = PropagationStore::default();
        store.put_envelope(order.clone()).unwrap();
        store.put_envelope(mail).unwrap();
        assert_eq!(crate::pass::active_order(&store, ObjId(3)), Some(&order));
        let ack = Envelope::acknowledge(3, &order, clock(2)).unwrap();
        let mut bad = serde_json::to_value(ack).unwrap();
        bad["kind"] = "order".into();
        assert!(serde_json::from_value::<Envelope>(bad).is_err());
    }

    #[test]
    fn shared_capacity_is_conserved_across_wire_round_trip_and_clock_rollback() {
        let mut state = TransportState::default();
        let pair = cid("ab");
        assert!(state.consume(&pair, 10, 4, 8));
        let mut loaded: TransportState =
            serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
        assert!(loaded.consume(&pair, 10, 4, 8));
        assert!(!loaded.consume(&pair, 10, 1, 8));
        assert!(!loaded.consume(&pair, 9, 1, 8));
        assert!(loaded.consume(&pair, 11, 8, 8));
    }
}
