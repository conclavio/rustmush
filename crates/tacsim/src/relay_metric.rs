//! R2's stationary strategic metric. The propagation crate owns scheduling;
//! this adapter supplies only the authored port/body mapping and the existing sky.
use std::collections::BTreeMap;

use mush_core::{CanonicalId, ClockStamp, ObjId, World};
use mush_propagation::routing::{Measurement, Metric, MetricProvider};

use crate::{TacsimStore, pack::PackEntry, sky::Sky};

pub const MAX_STRATEGIC_PORTS: usize = 32;

#[derive(Debug)]
pub struct RelayMetric;

#[derive(Debug, Default)]
struct Snapshot {
    sky: Option<Sky>,
    revision: String,
    at: i64,
    rooms: BTreeMap<ObjId, CanonicalId>,
    areas: BTreeMap<CanonicalId, CanonicalId>,
}

impl MetricProvider for RelayMetric {
    fn snapshot(&self, world: &World, at: &ClockStamp) -> Box<dyn Metric> {
        let mut snapshot = Snapshot {
            at: at.at,
            ..Snapshot::default()
        };
        let Some(store) = world.module_state::<TacsimStore>() else {
            return Box::new(snapshot);
        };
        let lane = CanonicalId::new(crate::definitions::content::LANE_PACK).expect("lane ID");
        let Some(version) = store.applied_in(world, &lane) else {
            return Box::new(snapshot);
        };
        let Some(loaded) = store.pack(&lane, version) else {
            return Box::new(snapshot);
        };
        // A bound on records read, not merely on the successful output. Oversized
        // maps refuse this metric rather than silently omitting the last port.
        let maps: Vec<_> = store.port_maps().take(MAX_STRATEGIC_PORTS + 1).collect();
        if maps.len() > MAX_STRATEGIC_PORTS {
            return Box::new(snapshot);
        }
        for map in maps {
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
            for room in [map.host, map.presentation].into_iter().flatten() {
                snapshot.rooms.insert(ObjId(room), body.clone());
            }
        }
        let sky = Sky::from_pack(&loaded.pack, &store.config().time.ephemeris);
        let Ok(revision) = sky.revision() else {
            return Box::new(Snapshot::default());
        };
        snapshot.revision = revision.to_hex();
        snapshot.sky = Some(sky);
        Box::new(snapshot)
    }
}

impl Snapshot {
    fn body<'a>(&'a self, world: &World, room: ObjId) -> Option<&'a CanonicalId> {
        self.rooms.get(&room).or_else(|| {
            let atlas = world.module_state::<mush_atlas::AtlasState>()?;
            let geo = atlas.records.roomgeo.get(&room)?;
            self.areas.get(&geo.neighborhood.pack)
        })
    }
}

impl Metric for Snapshot {
    fn between(&self, world: &World, from: ObjId, to: ObjId) -> Measurement {
        let Some(sky) = &self.sky else {
            return Measurement::Unknown;
        };
        match (self.body(world, from), self.body(world, to)) {
            (Some(a), Some(b)) if a != b => sky
                .light_lag_s(a, self.at, b, self.at)
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
