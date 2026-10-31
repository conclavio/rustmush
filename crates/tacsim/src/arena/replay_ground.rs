//! Complete immutable ground for a private replay. No lookup against today's
//! content is permitted: the tape covers all bytes, including locomotion prices
//! and mode declarations omitted from the historical arena-definition digest.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::state::{
    ArenaDefinition, ArenaMode, ArenaTerrain, EntryZone, HoldRule, KeySpot, TerrainCell, Waypoint,
    Waypoints,
};

const MAX_CELLS: usize = 1024 * 1024;
const MAX_NODES: usize = 4096;
const MAX_WORD: usize = 128;
/// One home a party at most, and a field holds at most 26 parties.
const MAX_HOMES: usize = 26;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplayGround {
    id: SmolStr,
    width: u16,
    height: u16,
    cell_cm: u16,
    cells: Vec<TerrainCell>,
    row_costs: Option<[[u16; 3]; 18]>,
    entry_zones: Vec<EntryZone>,
    key_spots: Vec<(SmolStr, u16, u16)>,
    nodes: Vec<(SmolStr, u16, u16, Vec<u16>)>,
    hold_modes: Vec<(u16, u16, u32)>,
    /// Doc 32 cut CF.6b: an IC field's homes; absent on every authored arena's tape.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    homes: Vec<EntryZone>,
}

impl ReplayGround {
    pub(crate) fn capture(definition: &ArenaDefinition) -> Result<Self, &'static str> {
        let ground = Self {
            id: definition.id.clone(),
            width: definition.terrain.width(),
            height: definition.terrain.height(),
            cell_cm: definition.terrain.cell_cm(),
            cells: definition.terrain.cells().to_vec(),
            row_costs: definition.terrain.replay_prices(),
            entry_zones: definition.entry_zones.clone(),
            key_spots: definition
                .key_spots
                .iter()
                .map(|s| (s.name.clone(), s.x, s.y))
                .collect(),
            nodes: definition
                .waypoints
                .nodes()
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let neighbours = u16::try_from(i)
                        .ok()
                        .map(|i| definition.waypoints.neighbours(i).to_vec())
                        .unwrap_or_default();
                    (n.id.clone(), n.x, n.y, neighbours)
                })
                .collect(),
            hold_modes: definition
                .modes
                .iter()
                .map(|mode| match mode {
                    ArenaMode::Hold(rule) => (rule.spot, rule.radius_cells, rule.hold_ms),
                })
                .collect(),
            homes: definition.homes.clone(),
        };
        ground.validate()?;
        Ok(ground)
    }

    fn validate(&self) -> Result<(), &'static str> {
        let word = |name: &SmolStr| {
            !name.is_empty() && name.len() <= MAX_WORD && !name.chars().any(char::is_control)
        };
        let cell = |x, y| x < self.width && y < self.height;
        if !word(&self.id)
            || self.width == 0
            || self.height == 0
            || self.cell_cm == 0
            || self.cells.len() > MAX_CELLS
            || self.cells.len() != usize::from(self.width) * usize::from(self.height)
            || self.entry_zones.len() > 16
            || self.key_spots.len() > super::state::SPOT_LETTERS.len()
            || self.nodes.len() > MAX_NODES
            || self.hold_modes.len() > 1
            || self.homes.len() > MAX_HOMES
        {
            return Err("invalid captured ground dimensions");
        }
        for zone in self.entry_zones.iter().chain(&self.homes) {
            if zone.x0 > zone.x1 || zone.y0 > zone.y1 || !cell(zone.x1, zone.y1) {
                return Err("invalid captured entry zone");
            }
        }
        let mut names = BTreeSet::new();
        for (name, x, y) in &self.key_spots {
            if !word(name) || !cell(*x, *y) || !names.insert(name) {
                return Err("invalid captured key spot");
            }
        }
        names.clear();
        for (at, (name, x, y, neighbours)) in self.nodes.iter().enumerate() {
            if !word(name)
                || !cell(*x, *y)
                || !names.insert(name)
                || neighbours.len() > MAX_NODES
                || neighbours.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err("invalid captured waypoint");
            }
            for neighbour in neighbours {
                let Some((_, _, _, reverse)) = self.nodes.get(usize::from(*neighbour)) else {
                    return Err("missing captured waypoint edge");
                };
                if !reverse.contains(&u16::try_from(at).map_err(|_| "waypoint index overflow")?) {
                    return Err("asymmetric captured waypoint edge");
                }
            }
        }
        for (spot, radius, milliseconds) in &self.hold_modes {
            if usize::from(*spot) >= self.key_spots.len() || *radius == 0 || *milliseconds == 0 {
                return Err("invalid captured Hold mode");
            }
        }
        Ok(())
    }

    pub(crate) fn restore(self) -> Result<ArenaDefinition, &'static str> {
        self.validate()?;
        let terrain = ArenaTerrain::new(self.width, self.height, self.cell_cm, self.cells)
            .map_err(|_| "invalid captured terrain")?
            .with_replay_prices(self.row_costs);
        let mut edges = Vec::new();
        for (at, (id, _, _, neighbours)) in self.nodes.iter().enumerate() {
            for neighbour in neighbours {
                if at <= usize::from(*neighbour) {
                    let other = self
                        .nodes
                        .get(usize::from(*neighbour))
                        .ok_or("missing waypoint")?;
                    edges.push((id.clone(), other.0.clone()));
                }
            }
        }
        let nodes = self
            .nodes
            .into_iter()
            .map(|(id, x, y, _)| Waypoint { id, x, y })
            .collect();
        Ok(ArenaDefinition {
            id: self.id,
            terrain,
            entry_zones: self.entry_zones,
            key_spots: self
                .key_spots
                .into_iter()
                .map(|(name, x, y)| KeySpot { name, x, y })
                .collect(),
            waypoints: Waypoints::new(nodes, &edges),
            modes: self
                .hold_modes
                .into_iter()
                .map(|(spot, radius_cells, hold_ms)| {
                    ArenaMode::Hold(HoldRule {
                        spot,
                        radius_cells,
                        hold_ms,
                    })
                })
                .collect(),
            homes: self.homes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::{CostRow, PartyId};
    use mush_atlas::TerrainClass;

    fn definition() -> ArenaDefinition {
        let mut prices = [[100; 3]; 18];
        for row in &mut prices {
            row[1] = 321;
            row[2] = 456;
        }
        let terrain = ArenaTerrain::new(
            3,
            3,
            1000,
            vec![
                TerrainCell {
                    class: TerrainClass::Grass,
                    elev_cm: 0,
                    move_cost: 100,
                };
                9
            ],
        )
        .expect("terrain")
        .with_replay_prices(Some(prices));
        ArenaDefinition {
            id: "captured".into(),
            terrain,
            entry_zones: vec![EntryZone {
                team: PartyId::RED,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: 1,
            }],
            key_spots: vec![KeySpot {
                name: "hill".into(),
                x: 1,
                y: 1,
            }],
            waypoints: Waypoints::new(
                vec![
                    Waypoint {
                        id: "west".into(),
                        x: 0,
                        y: 1,
                    },
                    Waypoint {
                        id: "east".into(),
                        x: 2,
                        y: 1,
                    },
                ],
                &[("west".into(), "east".into())],
            ),
            modes: vec![ArenaMode::Hold(HoldRule {
                spot: 0,
                radius_cells: 1,
                hold_ms: 10_000,
            })],
            homes: Vec::new(),
        }
    }

    #[test]
    fn capture_round_trip_keeps_prices_modes_graph_and_ground() {
        let original = definition();
        let captured = ReplayGround::capture(&original).expect("capture");
        let bytes = serde_json::to_vec(&captured).expect("encode");
        let decoded: ReplayGround = serde_json::from_slice(&bytes).expect("decode");
        assert_eq!(decoded.restore().expect("restore"), original);
        assert_eq!(
            original.terrain.replay_prices().expect("prices")[0][CostRow::Wheeled.index()],
            321
        );
    }

    #[test]
    fn malformed_graph_and_dimensions_refuse_before_cache_construction() {
        let captured = ReplayGround::capture(&definition()).expect("capture");
        let mut broken = captured.clone();
        broken.nodes[0].3.push(9);
        assert!(broken.restore().is_err());
        let mut broken = captured.clone();
        broken.nodes[1].3.clear();
        assert!(broken.restore().is_err());
        let mut broken = captured;
        broken.cells.pop();
        assert!(broken.restore().is_err());
    }
}
