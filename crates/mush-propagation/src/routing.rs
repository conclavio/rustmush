//! R2: bounded stationary routes, frozen physical legs and shared FIFO capacity.
//! The delivery pass remains the sole scheduler. All mutable progress is persisted
//! with the envelope; the capacity post-image shares its commit boundary.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use mush_core::{CanonicalId, ClockStamp, ObjId, ObjKind, World};
use mush_sim::SpatialState;
use mush_sim::relay::{LinkState, MAX_NODES, MAX_PAIRS, RelayRegistry, RelayStore};
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

    /// Engine dispatch supplies the live read lens to the composition's adapter.
    /// Detached callers retain stationary physics; a ship needs its real ticket.
    fn snapshot_in(&self, ctx: &mush_core::Ctx<'_>, at: &ClockStamp) -> Box<dyn Metric> {
        self.snapshot(ctx.w, at)
    }
}

pub trait Metric: std::fmt::Debug {
    /// None refuses this uplink; Some(true) is within a node's local coverage.
    /// Moving installations supply a checked snapshot, never mutate the registry.
    fn coverage(
        &self,
        _world: &World,
        node: &mush_sim::relay::RelayNode,
        endpoint: ObjId,
    ) -> Option<bool> {
        Some(node.coverage.contains(&endpoint))
    }

    fn between(&self, world: &World, from: ObjId, to: ObjId) -> Measurement;

    /// A moving carrier remains one receiving endpoint across its interior rooms.
    fn endpoint(&self, world: &World, state: &SpatialState, object: ObjId) -> Option<ObjId> {
        place(world, state, object)
    }
}

pub fn snapshot_in(ctx: &mush_core::Ctx<'_>, at: &ClockStamp) -> Option<Box<dyn Metric>> {
    Some(
        ctx.w
            .module_state::<RoutingServices>()?
            .0
            .snapshot_in(ctx, at),
    )
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
pub struct RoutingServices(
    pub Arc<dyn MetricProvider>,
    pub Option<Arc<dyn crate::service::Integration>>,
);

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
pub struct Serialization {
    pub total: u32,
    pub sent: u32,
    pub started: i64,
    pub last: i64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transit {
    pub version: u16,
    pub registry_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serialization: Option<Serialization>,
    #[serde(default)]
    pub admitted_operators: Vec<u64>,
    pub position: ObjId,
    pub destination: ObjId,
    pub legs: Vec<Leg>,
    pub next: usize,
    pub flight: Option<Flight>,
    pub queued_at: Option<ClockStamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<crate::service::Ticket>,
    pub forwardings: u8,
    pub physical_legs: u8,
    pub history: Vec<CompletedLeg>,
    pub last_work: Work,
}

impl Transit {
    pub fn validate(&self) -> Result<(), String> {
        let admitted: std::collections::BTreeSet<_> = self.admitted_operators.iter().collect();
        if admitted.len() != self.admitted_operators.len() || admitted.len() > MAX_LINK_HOPS {
            return Err("Invalid operator admission history".into());
        }
        if let Some(ticket) = &self.service
            && let Some(operator) = ticket.policy.as_ref().and_then(|p| p.operator)
            && (ticket.repeat_operator || self.serialization.is_some())
            && !self.admitted_operators.contains(&operator)
        {
            return Err("Service ticket contradicts operator admission history".into());
        }
        if let Some(p) = &self.serialization
            && (!(1..=8).contains(&p.total)
                || p.sent == 0
                || p.sent >= p.total
                || p.last < p.started
                || self.flight.is_some()
                || self.queued_at.as_ref().is_none_or(|q| p.started < q.at)
                || self.legs.get(self.next).is_none_or(|l| l.pair.is_none())
                || self.service.as_ref().is_some_and(|t| t.units != p.total))
        {
            return Err("Invalid relay serialization progress".into());
        }
        if self
            .service
            .as_ref()
            .is_some_and(|s| !s.valid() || self.queued_at.is_none())
            || self.version != 1
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportState {
    pub version: u16,
    pub capacity: BTreeMap<CanonicalId, Capacity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<crate::service::State>,
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            version: 1,
            capacity: BTreeMap::new(),
            service: None,
        }
    }
}

impl TransportState {
    /// This wire contains only string keys and integers. Construct its JSON
    /// directly so persisting capacity cannot panic or omit a spent bucket.
    pub(crate) fn to_value(&self) -> serde_json::Value {
        let capacity = self
            .capacity
            .iter()
            .map(|(pair, bucket)| {
                let mut value = serde_json::json!({"at": bucket.at, "units": bucket.units});
                if let Some(tokens) = bucket.tokens {
                    value["tokens"] = tokens.into();
                }
                (pair.to_string(), value)
            })
            .collect();
        let mut value = serde_json::json!({"version":self.version,"capacity":serde_json::Value::Object(capacity)});
        if let Some(service) = &self.service {
            value["service"] = serde_json::json!(service);
        }
        value
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(service) = &self.service {
            service.validate()?;
        }
        if self.version != 1
            || self.capacity.len() > MAX_PAIRS
            || self
                .capacity
                .values()
                .any(|b| b.units > 1_000_000 || b.tokens.is_some_and(|t| t > 1_000_000))
        {
            return Err("invalid relay transport capacity state".into());
        }
        Ok(())
    }

    pub(crate) fn available(&self, pair: &CanonicalId, now: i64, rate: u32, burst: u32) -> u32 {
        match self.capacity.get(pair) {
            Some(b) if b.at > now => 0,
            Some(b) => {
                // Legacy buckets record units spent in one IC second. Convert
                // the unspent balance conservatively; never refill on reboot.
                let balance = b.tokens.unwrap_or_else(|| burst.saturating_sub(b.units));
                let elapsed = now.saturating_sub(b.at) as u64;
                u64::from(balance)
                    .saturating_add(elapsed.saturating_mul(u64::from(rate)))
                    .min(u64::from(burst)) as u32
            }
            None => burst,
        }
    }
    fn next_bucket(
        &self,
        pair: &CanonicalId,
        now: i64,
        units: u32,
        rate: u32,
        burst: u32,
    ) -> Option<Capacity> {
        if units == 0 || (!self.capacity.contains_key(pair) && self.capacity.len() >= MAX_PAIRS) {
            return None;
        }
        let available = self.available(pair, now, rate, burst);
        if units > available {
            return None;
        }
        let spent = self
            .capacity
            .get(pair)
            .filter(|b| b.at == now)
            .map_or(0, |b| b.units);
        Some(Capacity {
            at: now,
            units: spent.saturating_add(units),
            tokens: Some(available - units),
        })
    }
    #[cfg(test)]
    fn consume(&mut self, pair: &CanonicalId, now: i64, units: u32, rate: u32, burst: u32) -> bool {
        let Some(bucket) = self.next_bucket(pair, now, units, rate, burst) else {
            return false;
        };
        self.capacity.insert(pair.clone(), bucket);
        true
    }
    /// Materialize under the old rate before changing it. Raising a burst or
    /// changing policy twice in one second cannot mint spendable capacity.
    pub(crate) fn reconfigure(
        &mut self,
        pair: &CanonicalId,
        now: i64,
        old_rate: u32,
        old_burst: u32,
        new_burst: u32,
    ) -> Result<(), String> {
        if self.capacity.get(pair).is_some_and(|b| b.at > now)
            || (!self.capacity.contains_key(pair) && self.capacity.len() >= MAX_PAIRS)
        {
            return Err("Relay capacity cannot be reconfigured at this clock or bound.".into());
        }
        let tokens = self
            .available(pair, now, old_rate, old_burst)
            .min(new_burst);
        let units = self
            .capacity
            .get(pair)
            .filter(|b| b.at == now)
            .map_or(0, |b| b.units);
        self.capacity.insert(
            pair.clone(),
            Capacity {
                at: now,
                units,
                tokens: Some(tokens),
            },
        );
        Ok(())
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
    endpoints: Option<(ObjId, ObjId)>,
) -> (Option<Transit>, Work) {
    plan_as(
        world, state, metric, from, to, at, used_hops, units, endpoints, None,
    )
}
#[allow(clippy::too_many_arguments)]
fn plan_as(
    world: &World,
    state: &SpatialState,
    metric: &dyn Metric,
    from: ObjId,
    to: ObjId,
    at: &ClockStamp,
    used_hops: usize,
    units: u32,
    endpoints: Option<(ObjId, ObjId)>,
    representation: Option<mush_core::RelayRepresentation>,
) -> (Option<Transit>, Work) {
    let Some(catalog) = registry(world) else {
        return (None, Work::default());
    };
    plan_catalog(
        catalog,
        world,
        state,
        metric,
        from,
        to,
        at,
        used_hops,
        units,
        endpoints,
        representation,
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
    endpoints: Option<(ObjId, ObjId)>,
    representation: Option<mush_core::RelayRepresentation>,
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
        let ingress = match metric.coverage(world, node, from) {
            Some(true) => Some(0),
            Some(false) => ingress.map(|v| v.0),
            None => None,
        };
        let egress = match metric.coverage(world, node, to) {
            Some(true) => Some(0),
            Some(false) => egress.map(|v| v.0),
            None => None,
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
    let permitted: std::collections::BTreeSet<_> = catalog
        .pairs
        .iter()
        .filter(|(id, p)| {
            endpoints.map_or(
                p.class_policy == mush_sim::relay::ClassPolicy::Exchange,
                |ends| crate::service::allowed_as(world, id, ends, representation),
            )
        })
        .map(|(id, _)| id.clone())
        .collect();
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
                    || !permitted.contains(pair_id)
                    || units
                        > crate::schedule::current(
                            world
                                .module_state::<PropagationStore>()
                                .and_then(|s| s.transport.service.as_ref()),
                            pair_id,
                            pair.capacity_units_per_ic_second,
                        )
                        .1
                        .max_bytes
                            / 128
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
    let best = best.filter(|p| direct.as_ref().is_none_or(|d| p.cost < d.0));
    let legs = if let Some(path) = best {
        let Some(nodes) = path
            .nodes
            .iter()
            .map(|id| catalog.nodes.get(id))
            .collect::<Option<Vec<_>>>()
        else {
            return (None, work);
        };
        let Some((first, rest)) = nodes.split_first() else {
            return (None, work);
        };
        let mut legs = Vec::new();
        legs.push(Leg {
            from,
            to: first.place,
            pair: None,
            relay: None,
        });
        for (pair, endpoints) in path.pairs.iter().zip(nodes.windows(2)) {
            let [a, b] = endpoints else {
                return (None, work);
            };
            legs.push(Leg {
                from: a.place,
                to: b.place,
                pair: Some(pair.clone()),
                relay: Some(b.object),
            });
        }
        let last = rest.last().unwrap_or(first);
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
            serialization: None,
            admitted_operators: vec![],
            position: from,
            destination: to,
            legs,
            next: 0,
            flight: None,
            queued_at: None,
            service: None,
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
    prepare_with(world, state, envelope, now, metric.as_ref())
}

pub fn prepare_with(
    world: &World,
    state: &SpatialState,
    envelope: &Envelope,
    now: &ClockStamp,
    metric: &dyn Metric,
) -> Option<Transit> {
    plan_as(
        world,
        state,
        metric,
        envelope.dispatch_ref().and_then(|d| d.origin).or_else(|| {
            metric.endpoint(
                world,
                state,
                envelope
                    .path()
                    .last()
                    .map_or(envelope.issuer(), |h| h.relay),
            )
        })?,
        destination(metric, world, state, envelope)?,
        now,
        envelope.path().len(),
        message_units(envelope),
        Some((envelope.issuer(), envelope.recipient())),
        envelope.representation(),
    )
    .0
}

fn message_units(envelope: &Envelope) -> u32 {
    crate::service::units(envelope)
}

fn destination(
    metric: &dyn Metric,
    world: &World,
    state: &SpatialState,
    envelope: &Envelope,
) -> Option<ObjId> {
    if world
        .get(envelope.recipient())
        .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING))
    {
        return envelope.dispatch_ref().and_then(|d| d.fallback);
    }
    metric.endpoint(world, state, envelope.recipient())
}

pub struct Pass<'a> {
    pub world: &'a World,
    pub state: &'a SpatialState,
    pub metric: Box<dyn Metric>,
    queues: BTreeMap<CanonicalId, VecDeque<u64>>,
    jamming: BTreeMap<ObjId, u32>,
    paid_capture: bool,
}

impl<'a> Pass<'a> {
    pub fn new(
        world: &'a World,
        state: &'a SpatialState,
        store: &PropagationStore,
        now: &ClockStamp,
        metric: Option<Box<dyn Metric>>,
    ) -> Option<Self> {
        if !enabled(world) {
            return None;
        }
        let services = world.module_state::<RoutingServices>()?;
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
                let rank = store.transport.service.as_ref().map_or(0, |s| {
                    let class = crate::service::effective(s, world, pair, e)
                        .map_or(crate::service::Class::Deferred, |v| v.0);
                    (class as i64)
                        .saturating_sub(
                            now.at.saturating_sub(at.at)
                                / r.service
                                    .as_ref()
                                    .and_then(|t| t.policy.as_ref())
                                    .map_or(s.config.aging_s, |p| p.settings.aging_s),
                        )
                        .max(0)
                });
                rows.push((rank, at.at, e.id(), pair.clone()));
            }
        }
        rows.sort();
        let mut queues = BTreeMap::<CanonicalId, VecDeque<u64>>::new();
        for (_, _, id, pair) in rows {
            queues.entry(pair).or_default().push_back(id);
        }
        Some(Self {
            world,
            state,
            metric: metric.unwrap_or_else(|| services.0.snapshot(world, now)),
            queues,
            jamming: store.jamming_map(),
            paid_capture: false,
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
                let from = envelope.dispatch_ref().and_then(|d| d.origin).or_else(|| {
                    self.metric.endpoint(
                        self.world,
                        self.state,
                        envelope
                            .path()
                            .last()
                            .map_or(envelope.issuer(), |h| h.relay),
                    )
                })?;
                let to = destination(self.metric.as_ref(), self.world, self.state, envelope)?;
                let route = plan_as(
                    self.world,
                    self.state,
                    self.metric.as_ref(),
                    from,
                    to,
                    now,
                    envelope.path().len(),
                    message_units(envelope),
                    Some((envelope.issuer(), envelope.recipient())),
                    envelope.representation(),
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
        let destination = destination(self.metric.as_ref(), self.world, self.state, envelope)?;
        let denied = route
            .legs
            .iter()
            .skip(route.next)
            .filter_map(|l| l.pair.as_ref())
            .any(|pair| {
                !crate::service::allowed_as(
                    self.world,
                    pair,
                    (envelope.issuer(), envelope.recipient()),
                    envelope.representation(),
                )
            });
        let oversized = route
            .legs
            .iter()
            .enumerate()
            .skip(route.next)
            .any(|(index, leg)| {
                !(index == route.next && route.service.is_some())
                    && leg.pair.as_ref().is_some_and(|id| {
                        catalog.pairs.get(id).is_some_and(|p| {
                            message_units(envelope)
                                > crate::schedule::current(
                                    store.transport.service.as_ref(),
                                    id,
                                    p.capacity_units_per_ic_second,
                                )
                                .1
                                .max_bytes
                                    / 128
                        })
                    })
            });
        let uncovered = route
            .legs
            .iter()
            .skip(route.next)
            .filter(|l| l.pair.is_none())
            .any(|l| {
                catalog.nodes.values().any(|n| {
                    (n.place == l.to && self.metric.coverage(self.world, n, l.from).is_none())
                        || (n.place == l.from
                            && self.metric.coverage(self.world, n, l.to).is_none())
                })
            });
        if route.destination != destination
            || route.registry_revision != catalog.revision
            || denied
            || oversized
            || uncovered
        {
            if route.destination != destination && route.forwardings >= MAX_FORWARDINGS {
                return Some(envelope.in_state(EnvelopeState::Expired, now.clone()));
            }
            let mut revised = plan_as(
                self.world,
                self.state,
                self.metric.as_ref(),
                route.position,
                destination,
                now,
                envelope.path().len(),
                message_units(envelope),
                Some((envelope.issuer(), envelope.recipient())),
                envelope.representation(),
            )
            .0?;
            if admission(store, Some(&revised), Some(envelope.id())).is_err() {
                return None;
            }
            revised.forwardings = route.forwardings + u8::from(route.destination != destination);
            revised.physical_legs = route.physical_legs;
            revised.history = route.history;
            revised.admitted_operators = route.admitted_operators;
            return Some(envelope.with_transit(revised));
        }
        let Some(leg) = route.legs.get(route.next).cloned() else {
            return Some(envelope.in_state(EnvelopeState::Delivered, now.clone()));
        };
        if let Some(pair_id) = &leg.pair {
            let pair = catalog.pairs.get(pair_id)?;
            if pair.state() != LinkState::Live {
                return None;
            }
            let (revision, settings) = crate::schedule::current(
                store.transport.service.as_ref(),
                pair_id,
                pair.capacity_units_per_ic_second,
            );
            let operator = crate::service::integration(self.world)
                .and_then(|api| api.operator(self.world, pair_id));
            let repeated = operator.is_some_and(|o| route.admitted_operators.contains(&o));
            if route.queued_at.is_none()
                || (store.transport.service.is_some() && route.service.is_none())
            {
                if route.queued_at.is_none()
                    && self
                        .queues
                        .get(pair_id)
                        .is_some_and(|q| q.len() >= MAX_LINK_QUEUE)
                {
                    return None;
                }
                let queued = route.queued_at.clone().unwrap_or_else(|| now.clone());
                if let Some(service) = &store.transport.service {
                    let (class, throttled) =
                        crate::service::effective(service, self.world, pair_id, envelope)?;
                    let base_class = Some(crate::commerce::base(
                        service, self.world, pair_id, envelope,
                    ));
                    let units = message_units(envelope);
                    let available =
                        store
                            .transport
                            .available(pair_id, now.at, settings.rate, settings.burst);
                    route.service = Some(crate::service::Ticket {
                        version: 1,
                        class,
                        base_class,
                        throttled,
                        units,
                        eligible_at: settings.eligible_at(class, queued.at, repeated),
                        declared_s: if repeated {
                            0
                        } else {
                            settings.standard_delay_s
                        },
                        actual_ready: (available > 0
                            && self.jamming.get(&leg.to).copied().unwrap_or(0)
                                < crate::context::JAMMING_BLOCKS_AT)
                            .then_some(now.at),
                        policy: Some(crate::schedule::Snapshot {
                            revision,
                            operator,
                            settings: settings.clone(),
                        }),
                        repeat_operator: repeated,
                    });
                }
                route.queued_at = Some(queued);
                return Some(envelope.with_transit(route));
            }
            // Policy applies again immediately before transmission. Preserve the
            // purchased/granted base and queue age, never stack demotions on retries.
            if let (Some(state), Some(ticket), Some(queued)) = (
                &store.transport.service,
                &mut route.service,
                &route.queued_at,
            ) {
                let (class, throttled) =
                    crate::service::effective(state, self.world, pair_id, envelope)?;
                if ticket.class != class || ticket.throttled != throttled {
                    ticket.base_class =
                        Some(crate::commerce::base(state, self.world, pair_id, envelope));
                    ticket.class = class;
                    ticket.throttled = throttled;
                    ticket.eligible_at = ticket.policy.as_ref().map_or_else(
                        || state.config.eligible_at(class, queued.at),
                        |p| {
                            p.settings
                                .eligible_at(class, queued.at, ticket.repeat_operator)
                        },
                    );
                    ticket.actual_ready = None;
                    return Some(envelope.with_transit(route));
                }
            }
            let available =
                store
                    .transport
                    .available(pair_id, now.at, settings.rate, settings.burst);
            if let Some(ticket) = route.service.as_mut()
                && ticket.actual_ready.is_none()
                && available > 0
                && self.jamming.get(&leg.to).copied().unwrap_or(0)
                    < crate::context::JAMMING_BLOCKS_AT
            {
                ticket.actual_ready = Some(now.at);
                return Some(envelope.with_transit(route));
            }
            // A closed Deferred window or jammed destination cannot block open
            // work. Large messages retain class/FIFO rank while serializing;
            // aging prevents a stream of higher-class work starving them.
            let queue = self.queues.get_mut(pair_id)?;
            let chosen = queue
                .iter()
                .find(|id| {
                    store.envelope(**id).is_some_and(|e| {
                        if !matches!(
                            e.state(),
                            EnvelopeState::Queued | EnvelopeState::Transmitted
                        ) {
                            return false;
                        }
                        let Some(r) = e.transit() else { return false };
                        let Some(l) = r.legs.get(r.next) else {
                            return false;
                        };
                        if r.registry_revision != catalog.revision
                            || self::destination(self.metric.as_ref(), self.world, self.state, e)
                                != Some(r.destination)
                            || self.jamming.get(&l.to).copied().unwrap_or(0)
                                >= crate::context::JAMMING_BLOCKS_AT
                        {
                            return false;
                        }
                        if store.transport.service.as_ref().is_some_and(|state| {
                            crate::service::effective(state, self.world, pair_id, e).is_none_or(
                                |(class, throttled)| {
                                    r.service.as_ref().is_some_and(|t| {
                                        t.class != class || t.throttled != throttled
                                    })
                                },
                            )
                        }) {
                            return false;
                        }
                        r.service.as_ref().map_or_else(
                            || available > 0,
                            |t| {
                                available > 0
                                    && now.at >= t.eligible_at
                                    && (t.class != crate::service::Class::Deferred
                                        || store.transport.service.as_ref().is_some_and(|s| {
                                            t.policy.as_ref().map_or_else(
                                                || s.config.window_open(now.at),
                                                |p| p.settings.window_open(now.at),
                                            )
                                        }))
                            },
                        )
                    })
                })
                .copied();
            if chosen != Some(envelope.id()) {
                return None;
            }
            if self.jamming.get(&leg.to).copied().unwrap_or(0) >= crate::context::JAMMING_BLOCKS_AT
            {
                return None;
            }
            let needs_capture = store
                .transport
                .service
                .as_ref()
                .and_then(|s| s.commerce.as_ref())
                .is_some_and(|m| {
                    crate::service::integration(self.world)
                        .and_then(|api| api.operator(self.world, pair_id))
                        == Some(m.operator)
                        && m.active(envelope.id())
                            .is_some_and(|p| p.phase == crate::commerce::Phase::Reserved)
                });
            if needs_capture
                && (self.paid_capture
                    || !store
                        .transport
                        .service
                        .as_ref()
                        .and_then(|s| s.commerce.as_ref())
                        .and_then(|m| m.active(envelope.id()))
                        .is_some_and(|p| {
                            now.at < p.service_until
                                && crate::service::integration(self.world)
                                    .is_some_and(|api| api.reserved(self.world, p))
                        }))
            {
                return None;
            }
            let total = message_units(envelope);
            let sent = route.serialization.as_ref().map_or(0, |p| p.sent);
            let units = total.saturating_sub(sent).min(available);
            if units == 0 {
                return None;
            }
            if operator.is_some_and(|o| !route.admitted_operators.contains(&o))
                && route.admitted_operators.len() >= MAX_LINK_HOPS
            {
                return Some(envelope.in_state(EnvelopeState::Expired, now.clone()));
            }
            // Preflight every fallible step before marking money or capacity.
            let completed = if sent + units == total {
                Some(match envelope.relayed(leg.relay?, now.clone()) {
                    Ok(next) => next,
                    Err(_) => return Some(envelope.in_state(EnvelopeState::Expired, now.clone())),
                })
            } else {
                None
            };
            let bucket = store.transport.next_bucket(
                pair_id,
                now.at,
                units,
                settings.rate,
                settings.burst,
            )?;
            // First-unit capture, progress and capacity share the existing strict
            // durable transaction. Failed capture installs none of this pass.
            if !crate::commerce::consuming(store, self.world, pair_id, envelope.id()) {
                return None;
            }
            store.transport.capacity.insert(pair_id.clone(), bucket);
            self.paid_capture |= needs_capture;
            if let Some(operator) = operator
                && !route.admitted_operators.contains(&operator)
            {
                route.admitted_operators.push(operator);
            }
            let started = route.serialization.as_ref().map_or(now.at, |p| p.started);
            if sent + units < total {
                route.serialization = Some(Serialization {
                    total,
                    sent: sent + units,
                    started,
                    last: now.at,
                });
                return Some(
                    envelope
                        .in_state(EnvelopeState::Transmitted, now.clone())
                        .with_transit(route),
                );
            }
            let mut next = completed?;
            queue.retain(|id| *id != envelope.id());
            if let (Some(state), Some(ticket), Some(queued)) = (
                &mut store.transport.service,
                &route.service,
                &route.queued_at,
            ) {
                state.receipt(crate::service::Receipt {
                    serialized_from: Some(started),
                    representation: envelope.representation(),
                    envelope: envelope.id(),
                    pair: pair_id.clone(),
                    issuer: envelope.issuer(),
                    recipient: envelope.recipient(),
                    reference: envelope.dispatch_ref().map(|d| d.key.clone()),
                    kind: envelope.kind(),
                    ticket: ticket.clone(),
                    queued: queued.at,
                    transmitted: now.at,
                    arrived: None,
                });
            }
            route.position = leg.to;
            route.next += 1;
            route.queued_at = None;
            route.service = None;
            route.serialization = None;
            next = next.with_transit(route);
            Some(next)
        } else {
            if route.physical_legs >= MAX_PHYSICAL_LEGS {
                return Some(envelope.in_state(EnvelopeState::Expired, now.clone()));
            }
            let covered = catalog.nodes.values().any(|n| {
                (n.place == leg.to && self.metric.coverage(self.world, n, leg.from) == Some(true))
                    || (n.place == leg.from
                        && self.metric.coverage(self.world, n, leg.to) == Some(true))
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
    use mush_sim::relay::{ClassPolicy, RelayHalf, RelayNode, RelayPair};

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
                        damaged: false,
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
            losses: BTreeMap::new(),
            structural_version: 0,
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
            None,
            None,
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
            None,
            None,
        );
        let hit = hit.unwrap();
        hit.validate().unwrap();
        assert!(miss.is_none());
        assert_eq!(hit_work, miss_work);
        assert_eq!(hit_work.metric_reads, 9);
        assert_eq!(hit_work.edge_visits, 24);
        assert_eq!(hit.legs.iter().filter(|l| l.pair.is_some()).count(), 1);
        registry.pairs.get_mut(&cid("ca")).unwrap().class_policy = ClassPolicy::Private;
        let (private, _) = plan_catalog(
            &registry,
            &world,
            &spatial,
            &TestMetric { end: ObjId(30) },
            ObjId(100),
            ObjId(200),
            &at,
            0,
            1,
            None,
            None,
        );
        assert_eq!(
            private
                .unwrap()
                .legs
                .iter()
                .filter(|l| l.pair.is_some())
                .count(),
            2
        );
        registry.pairs.get_mut(&cid("ca")).unwrap().halves[0].enabled = false;
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
            None,
            None,
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
            None,
            None,
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
            None,
            None,
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
        mail["dispatch"] = serde_json::json!({"version":1,"key":"mail:v1:1:3:out","bytes":10,"origin":null,"fallback":null});
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
        assert!(state.consume(&pair, 10, 4, 8, 8));
        assert_eq!(state.to_value(), serde_json::to_value(&state).unwrap());
        let mut loaded: TransportState = serde_json::from_value(state.to_value()).unwrap();
        assert!(loaded.consume(&pair, 10, 4, 8, 8));
        assert!(!loaded.consume(&pair, 10, 1, 8, 8));
        assert!(!loaded.consume(&pair, 9, 1, 8, 8));
        assert!(loaded.consume(&pair, 11, 8, 8, 8));
    }

    #[test]
    fn capacity_post_image_matches_the_wire_at_integer_limits() {
        let mut state = TransportState::default();
        assert_eq!(state.to_value(), serde_json::to_value(&state).unwrap());
        state.capacity.insert(
            cid("ab"),
            Capacity {
                at: i64::MIN,
                units: 0,
                tokens: None,
            },
        );
        state.capacity.insert(
            cid("ba"),
            Capacity {
                at: i64::MAX,
                units: u32::MAX,
                tokens: None,
            },
        );
        assert_eq!(state.to_value(), serde_json::to_value(&state).unwrap());
        assert_eq!(
            serde_json::from_value::<TransportState>(state.to_value()).unwrap(),
            state
        );
    }
    #[test]
    fn token_rate_burst_policy_changes_and_legacy_buckets_conserve_capacity() {
        let mut state = TransportState::default();
        let pair = cid("ab");
        assert!(state.consume(&pair, 10, 6, 2, 6));
        assert!(!state.consume(&pair, 10, 1, 2, 6));
        assert!(state.consume(&pair, 11, 2, 2, 6));
        let mut state: TransportState = serde_json::from_value(state.to_value()).unwrap();
        assert_eq!(state.available(&pair, 11, 2, 6), 0);
        assert_eq!(state.available(&pair, 10, 2, 6), 0);
        assert_eq!(state.available(&pair, 100, 2, 6), 6);
        state.reconfigure(&pair, 100, 2, 6, 20).unwrap();
        state.reconfigure(&pair, 100, 1, 20, 30).unwrap();
        assert_eq!(state.available(&pair, 100, 1, 30), 6);
        assert_eq!(state.available(&pair, 101, 1, 30), 7);
        assert!(state.reconfigure(&pair, 99, 1, 30, 30).is_err());
        state.capacity.insert(
            pair.clone(),
            Capacity {
                at: 10,
                units: 8,
                tokens: None,
            },
        );
        assert_eq!(state.available(&pair, 10, 8, 8), 0);
        assert_eq!(state.available(&pair, 11, 8, 8), 8);
    }
}
