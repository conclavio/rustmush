//! **Where a match is fought, authored** (16b cut C.2, C1/C3; doc 16 §3.2
//! "Space", §4 "Content packs"; D-16-5).
//!
//! The arena entry names ground the **atlas** owns and lays the match's own
//! furniture over it: the window's corner and side, the two team entry zones,
//! the named key spots, the waypoint graph cut N's reflex controller patrols,
//! and the eight stock tanks the applier makes.
//!
//! # It names records, not cells
//!
//! Terrain is the atlas's (doc 16 §3.2: *the existing 18-class cell grid with
//! the engine's cover and opacity per class*), so this entry carries no cell of
//! its own: it names the region and the heightfield entries whose tiles cover
//! the window, and [`from_pack`] reads them out of the live records at
//! `@arena/start`. That is why the reference check for an arena is at **apply**
//! and not at load — a pack can be verified with no world at all, and an atlas
//! record only exists once a world has applied the pack that makes it.
//!
//! # Why `tiles` and not `tile`
//!
//! A 256 by 256 field of little-endian samples is 175 KB of base64, and
//! `mush_content::MAX_ENTRY_BYTES` is 64 KiB, so C.1's ruling is that a
//! window-sized field ships as four 128 by 128 `heightfield` entries. Four
//! entries are four tiles, so the arena names a list; each tile is placed from
//! its own record's bounding box, which means the list's order says nothing and
//! a mis-authored offset is a refusal rather than a silently rotated map.

use mush_atlas::TerrainClass;
use mush_atlas::physics::LocomotionTable;
use mush_atlas::record::{RecordKey, Records, TileRec};
use mush_core::CanonicalId;
use serde::{Deserialize, Serialize};

use crate::arena::state::{
    ArenaDefinition, ArenaTerrain, EntryZone, KeySpot as ArenaKeySpot, Team, TerrainCell,
    Waypoint as ArenaWaypoint, Waypoints,
};
use crate::definitions::DefinitionError;
use crate::definitions::unit::TRACKED;

/// The most heightfield entries one arena may name. Four tile the 256 by 256
/// window at C.1's 128 by 128 quadrants; sixteen is the same window at 64.
pub const MAX_ARENA_TILES: usize = 16;

/// The most named key spots one arena may carry (doc 16 §3.2: *three or four*).
pub const MAX_KEY_SPOTS: usize = 32;

/// The most waypoint nodes one arena may carry. Cut N walks the graph inside a
/// pulse, so its size is a bound and not a preference.
pub const MAX_WAYPOINT_NODES: usize = 256;

/// The most waypoint edges.
pub const MAX_WAYPOINT_EDGES: usize = 1_024;

/// The most stock units one arena may place — `match.max_slots_per_side` (12)
/// on each side (16b §9).
pub const MAX_ARENA_UNITS: usize = 24;

/// A team's deployment rectangle, in cells, inclusive of both corners.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryZoneDef {
    /// The side that deploys here.
    pub team: Team,
    /// The west column.
    pub x0: u16,
    /// The south row.
    pub y0: u16,
    /// The east column, inclusive.
    pub x1: u16,
    /// The north row, inclusive.
    pub y1: u16,
}

/// A named place on the map: what a pilot and a controller both call it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeySpot {
    /// The name.
    pub name: String,
    /// Its column.
    pub x: u16,
    /// Its row.
    pub y: u16,
}

/// One node of the patrol graph (doc 16 §6: *the arena pack's waypoint graph*).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaypointNode {
    /// The node's id, named by the edges.
    pub id: CanonicalId,
    /// Its column.
    pub x: u16,
    /// Its row.
    pub y: u16,
}

/// The patrol graph.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaypointGraph {
    /// The nodes.
    pub nodes: Vec<WaypointNode>,
    /// Undirected edges, each naming two node ids.
    pub edges: Vec<[CanonicalId; 2]>,
}

/// One stock unit the arena's applier makes (doc 16 §3.3 "The stock tanks are
/// persistent objects").
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TankDef {
    /// The slot's id — stable across re-applies, which is what makes the
    /// applier idempotent without inventing a second identity for the object.
    pub id: CanonicalId,
    /// The object's name in the world.
    pub name: String,
    /// Its side.
    pub team: Team,
    /// Its opening column.
    pub x: u16,
    /// Its opening row.
    pub y: u16,
    /// Its opening heading, millidegrees clockwise from north.
    pub heading_mdeg: u32,
    /// The unit schema it is made from — an entry id, checked at load.
    pub unit: CanonicalId,
}

/// One arena (doc 16 §4's `arenas/proving_ground.json`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaDef {
    /// The human-facing name.
    pub label: String,
    /// The **atlas** pack whose records carry this arena's ground.
    pub pack: CanonicalId,
    /// The region entry in that pack.
    pub region: CanonicalId,
    /// The `heightfield` entries in that pack whose tiles cover the window.
    pub tiles: Vec<CanonicalId>,
    /// The window's south-west corner in region metres, `[east, north]`.
    pub origin_m: [i64; 2],
    /// The window's width in cells.
    pub cols: u16,
    /// Its height in cells.
    pub rows: u16,
    /// One cell's side, centimetres.
    pub cell_cm: u16,
    /// The deployment rectangles, one per side.
    pub entry_zones: Vec<EntryZoneDef>,
    /// Named places.
    pub key_spots: Vec<KeySpot>,
    /// The patrol graph.
    pub waypoints: WaypointGraph,
    /// The stock units.
    pub tanks: Vec<TankDef>,
    /// Why these numbers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl ArenaDef {
    /// The record key of the region this arena stands on.
    ///
    /// # Errors
    ///
    /// Never: both halves are already canonical ids.
    #[must_use]
    pub fn region_key(&self) -> RecordKey {
        RecordKey::new(self.pack.clone(), self.region.clone())
    }

    /// The record keys of the tiles that cover the window.
    ///
    /// A `heightfield` entry makes exactly one tile, keyed `<entry>.tile.0` by
    /// the atlas's own applier, so the entry id in this pack and the tile record
    /// in that one are one derivation and not two authored names.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] for an entry id the derived key will not form.
    pub fn tile_keys(&self, entry: &str) -> Result<Vec<RecordKey>, DefinitionError> {
        let mut keys = Vec::with_capacity(self.tiles.len());
        for tile in &self.tiles {
            let id = CanonicalId::new(format!("{tile}.tile.0")).map_err(|error| {
                DefinitionError::value(entry, format!("tile `{tile}`: {error}"))
            })?;
            keys.push(RecordKey::new(self.pack.clone(), id));
        }
        Ok(keys)
    }

    /// Re-run every bound and every internal reference (the unit each tank
    /// names is checked across the pack by the loader, not here).
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry and what is wrong.
    pub fn check(&self, entry: &str) -> Result<(), DefinitionError> {
        crate::definitions::check_label(entry, &self.label)?;
        crate::definitions::check_note(entry, &self.note)?;
        if self.cols == 0
            || self.rows == 0
            || self.cols > mush_atlas::cells::MAX_TILE_SIDE
            || self.rows > mush_atlas::cells::MAX_TILE_SIDE
        {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "an arena window is 1 to {} cells a side; {}×{} is not",
                    mush_atlas::cells::MAX_TILE_SIDE,
                    self.cols,
                    self.rows
                ),
            ));
        }
        if !mush_atlas::record::RESOLUTIONS_CM.contains(&self.cell_cm) {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "cell_cm is one of {:?}; {} is not",
                    mush_atlas::record::RESOLUTIONS_CM,
                    self.cell_cm
                ),
            ));
        }
        if self.tiles.is_empty() || self.tiles.len() > MAX_ARENA_TILES {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "an arena names 1 to {MAX_ARENA_TILES} heightfield entries; {} is not",
                    self.tiles.len()
                ),
            ));
        }
        let mut seen: Vec<&CanonicalId> = Vec::new();
        for tile in &self.tiles {
            if seen.contains(&tile) {
                return Err(DefinitionError::value(
                    entry,
                    format!("tile `{tile}` is named twice"),
                ));
            }
            seen.push(tile);
        }
        self.tile_keys(entry)?;
        if self.entry_zones.len() != Team::ALL.len() {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "an arena declares one entry zone per side ({}); {} is not",
                    Team::ALL.len(),
                    self.entry_zones.len()
                ),
            ));
        }
        for team in Team::ALL {
            if !self.entry_zones.iter().any(|zone| zone.team == team) {
                return Err(DefinitionError::value(
                    entry,
                    format!("no entry zone for the {} side", team.as_str()),
                ));
            }
        }
        for zone in &self.entry_zones {
            if zone.x1 < zone.x0 || zone.y1 < zone.y0 {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "the {} entry zone runs backwards: {},{} to {},{}",
                        zone.team.as_str(),
                        zone.x0,
                        zone.y0,
                        zone.x1,
                        zone.y1
                    ),
                ));
            }
            self.on_window(entry, zone.x1, zone.y1, "an entry zone")?;
        }
        if self.key_spots.len() > MAX_KEY_SPOTS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "an arena names at most {MAX_KEY_SPOTS} key spots; {} is not",
                    self.key_spots.len()
                ),
            ));
        }
        for spot in &self.key_spots {
            crate::definitions::check_label(entry, &spot.name)?;
            self.on_window(entry, spot.x, spot.y, &format!("key spot `{}`", spot.name))?;
        }
        self.check_waypoints(entry)?;
        if self.tanks.len() > MAX_ARENA_UNITS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "an arena places at most {MAX_ARENA_UNITS} units; {} is not",
                    self.tanks.len()
                ),
            ));
        }
        let mut ids: Vec<&CanonicalId> = Vec::new();
        let mut cells: Vec<(u16, u16)> = Vec::new();
        for tank in &self.tanks {
            if ids.contains(&&tank.id) {
                return Err(DefinitionError::value(
                    entry,
                    format!("unit slot `{}` is declared twice", tank.id),
                ));
            }
            ids.push(&tank.id);
            crate::definitions::check_label(entry, &tank.name)?;
            self.on_window(entry, tank.x, tank.y, &format!("unit `{}`", tank.id))?;
            // Doc 16 §3.4 step 5: one unit per cell, and `Arena::new` refuses a
            // check-out that breaks it — so an arena that authored two tanks on
            // one cell would be a pack that cannot start a match.
            if cells.contains(&(tank.x, tank.y)) {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "unit `{}` opens on {},{}, where another already stands",
                        tank.id, tank.x, tank.y
                    ),
                ));
            }
            cells.push((tank.x, tank.y));
            if tank.heading_mdeg >= 360_000 {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "unit `{}`'s heading_mdeg is 0 to 359999; {} is not",
                        tank.id, tank.heading_mdeg
                    ),
                ));
            }
            let zone = self
                .entry_zones
                .iter()
                .find(|zone| zone.team == tank.team)
                .ok_or_else(|| {
                    DefinitionError::value(
                        entry,
                        format!("unit `{}` has no entry zone for its side", tank.id),
                    )
                })?;
            if tank.x < zone.x0 || tank.x > zone.x1 || tank.y < zone.y0 || tank.y > zone.y1 {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "unit `{}` opens on {},{}, outside its own side's entry zone",
                        tank.id, tank.x, tank.y
                    ),
                ));
            }
        }
        Ok(())
    }

    fn check_waypoints(&self, entry: &str) -> Result<(), DefinitionError> {
        if self.waypoints.nodes.len() > MAX_WAYPOINT_NODES {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "an arena's waypoint graph is at most {MAX_WAYPOINT_NODES} nodes; {} is not",
                    self.waypoints.nodes.len()
                ),
            ));
        }
        if self.waypoints.edges.len() > MAX_WAYPOINT_EDGES {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "an arena's waypoint graph is at most {MAX_WAYPOINT_EDGES} edges; {} is not",
                    self.waypoints.edges.len()
                ),
            ));
        }
        let mut ids: Vec<&CanonicalId> = Vec::new();
        for node in &self.waypoints.nodes {
            if ids.contains(&&node.id) {
                return Err(DefinitionError::value(
                    entry,
                    format!("waypoint `{}` is declared twice", node.id),
                ));
            }
            ids.push(&node.id);
            self.on_window(entry, node.x, node.y, &format!("waypoint `{}`", node.id))?;
        }
        for edge in &self.waypoints.edges {
            for end in edge {
                if !ids.contains(&end) {
                    return Err(DefinitionError::value(
                        entry,
                        format!("a waypoint edge names `{end}`, which is not a node"),
                    ));
                }
            }
            if edge[0] == edge[1] {
                return Err(DefinitionError::value(
                    entry,
                    format!("waypoint `{}` has an edge to itself", edge[0]),
                ));
            }
        }
        Ok(())
    }

    fn on_window(&self, entry: &str, x: u16, y: u16, what: &str) -> Result<(), DefinitionError> {
        if x >= self.cols || y >= self.rows {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "{what} is at {x},{y}, off a {}×{} window",
                    self.cols, self.rows
                ),
            ));
        }
        Ok(())
    }
}

/// Why an authored arena could not be built over this world's records.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ArenaBuildError {
    /// The region the arena names is not in the atlas's records.
    #[error("arena {arena}: the atlas has no region {region}")]
    NoRegion {
        /// The arena entry.
        arena: String,
        /// The region record key.
        region: String,
    },
    /// A tile the arena names is not in the atlas's records.
    #[error("arena {arena}: the atlas has no tile {tile}")]
    NoTile {
        /// The arena entry.
        arena: String,
        /// The tile record key.
        tile: String,
    },
    /// A tile is in a different region, or at a different cell size, from the
    /// arena that names it.
    #[error("arena {arena}: tile {tile} {detail}")]
    TileMismatch {
        /// The arena entry.
        arena: String,
        /// The tile record key.
        tile: String,
        /// What disagrees.
        detail: String,
    },
    /// The named tiles do not cover the whole window.
    #[error("arena {arena}: {missing} of {total} cells are covered by no tile")]
    NotCovered {
        /// The arena entry.
        arena: String,
        /// How many cells no tile covered.
        missing: usize,
        /// How many the window has.
        total: usize,
    },
    /// The window itself is not a shape the arena accepts.
    #[error("arena {arena}: {detail}")]
    Shape {
        /// The arena entry.
        arena: String,
        /// What is wrong.
        detail: String,
    },
}

/// **Build the arena's window from this world's atlas records** (16b cut C.2;
/// doc 16 §3.2 "Terrain", §3.4 step 2).
///
/// Every cell of the window comes from the tile that covers it: its class and
/// its `elev_cm` verbatim, and its `move_cost` from the **shipped locomotion
/// table**'s `tracked` row — the same table `mush_atlas::physics::move_cost` and
/// the atlas's own pathfinder read, so a cell a tank cannot enter in the arena
/// is a cell a tank cannot enter anywhere. A class the table prices `null` (the
/// void, a wall, deep water, heavy woods) becomes `move_cost: 0`, which is the
/// arena's own spelling of impassable.
///
/// The per-cell cost is the class's flat row and **not** `physics::move_cost`:
/// that function prices a *step* from one cell onto another, with the diagonal
/// factor and the slope surcharge, and doc 16 §3.4 step 2's terrain factor is
/// `100 / move_cost` **of the cell the unit is in**. Slope is a property of a
/// pair of cells and belongs to the motion that crosses them.
///
/// **16b cut X.6 is where that slope finally lands.** `elev_cm` was already
/// copied verbatim and has been carried onto a unit's `z_cm` and honoured by the
/// line of sight since cut S.1; what cut X.6 adds is the edge. The slope of a
/// crossing is priced by [`crate::arena::slope`] at the crossing itself and in
/// the reflex controller's A\*, off this same shipped table's limits, and an edge
/// steeper than a unit's locomotion will take is refused there. So the cell's
/// cost and the edge's surcharge are two halves of one answer, applied in the two
/// places each belongs.
///
/// The arena still never calls `physics::move_cost`, and in particular its
/// **climb branch is not applied**: `rise <= climb_cm && is_built(to.class)`
/// models a kerb or a doorstep inside one cell, and these cells are 10 m across,
/// so a `legged` unit stepping up 100 cm onto `Road` pays `cost × 110 / 100` here
/// against the atlas's `cost × 150 / 100`. [`crate::arena::slope`]'s head argues
/// it and its tests pin it.
///
/// # Errors
///
/// [`ArenaBuildError`] naming the arena and the record that is missing or
/// disagrees. This is the **apply-time** half of an arena's reference check.
pub fn from_pack(
    entry: &str,
    def: &ArenaDef,
    records: &Records,
) -> Result<ArenaDefinition, ArenaBuildError> {
    let region = def.region_key();
    if !records.regions.contains_key(&region) {
        return Err(ArenaBuildError::NoRegion {
            arena: entry.to_string(),
            region: region.to_string(),
        });
    }
    let total = usize::from(def.cols)
        .checked_mul(usize::from(def.rows))
        .ok_or_else(|| ArenaBuildError::Shape {
            arena: entry.to_string(),
            detail: format!("{}×{} cells overflows", def.cols, def.rows),
        })?;
    let mut cells: Vec<Option<TerrainCell>> = vec![None; total];
    let table = LocomotionTable::shipped();
    let keys = def
        .tile_keys(entry)
        .map_err(|error| ArenaBuildError::Shape {
            arena: entry.to_string(),
            detail: error.to_string(),
        })?;
    for key in keys {
        let tile = records
            .tiles
            .get(&key)
            .ok_or_else(|| ArenaBuildError::NoTile {
                arena: entry.to_string(),
                tile: key.to_string(),
            })?;
        if tile.region != region {
            return Err(ArenaBuildError::TileMismatch {
                arena: entry.to_string(),
                tile: key.to_string(),
                detail: format!("is in region {}, not {region}", tile.region),
            });
        }
        if tile.resolution_cm != def.cell_cm {
            return Err(ArenaBuildError::TileMismatch {
                arena: entry.to_string(),
                tile: key.to_string(),
                detail: format!(
                    "is at {} cm cells, and the arena is at {}",
                    tile.resolution_cm, def.cell_cm
                ),
            });
        }
        let (ox, oy) = tile_origin(entry, def, &key, tile)?;
        for y in 0..tile.cells.height() {
            for x in 0..tile.cells.width() {
                let Some(cell) = tile.cells.get(x, y) else {
                    continue;
                };
                let (Some(wx), Some(wy)) = (ox.checked_add(x), oy.checked_add(y)) else {
                    continue;
                };
                if wx >= def.cols || wy >= def.rows {
                    continue;
                }
                let index = usize::from(wy)
                    .checked_mul(usize::from(def.cols))
                    .and_then(|row| row.checked_add(usize::from(wx)));
                if let Some(slot) = index.and_then(|index| cells.get_mut(index)) {
                    *slot = Some(TerrainCell {
                        class: cell.class,
                        elev_cm: cell.elev_cm,
                        move_cost: move_cost_of(table, cell.class),
                    });
                }
            }
        }
    }
    let missing = cells.iter().filter(|cell| cell.is_none()).count();
    if missing != 0 {
        return Err(ArenaBuildError::NotCovered {
            arena: entry.to_string(),
            missing,
            total,
        });
    }
    let cells: Vec<TerrainCell> = cells.into_iter().flatten().collect();
    let terrain = ArenaTerrain::new(def.cols, def.rows, def.cell_cm, cells).map_err(|error| {
        ArenaBuildError::Shape {
            arena: entry.to_string(),
            detail: error.to_string(),
        }
    })?;
    Ok(ArenaDefinition {
        id: smol_str::SmolStr::new(entry),
        terrain,
        entry_zones: def
            .entry_zones
            .iter()
            .map(|zone| EntryZone {
                team: zone.team,
                x0: zone.x0,
                y0: zone.y0,
                x1: zone.x1,
                y1: zone.y1,
            })
            .collect(),
        // **16b cut N.1**: the two authored tables C.2 wrote and nothing read. The
        // key spots cross whole; the waypoint graph's id-named edges are resolved
        // into node indices once, here, because cut N's controller walks the graph
        // inside a pulse and a string comparison per edge per tick is not a thing
        // the pulse budget has room for.
        key_spots: def
            .key_spots
            .iter()
            .map(|spot| ArenaKeySpot {
                name: smol_str::SmolStr::new(&spot.name),
                x: spot.x,
                y: spot.y,
            })
            .collect(),
        waypoints: Waypoints::new(
            def.waypoints
                .nodes
                .iter()
                .map(|node| ArenaWaypoint {
                    id: smol_str::SmolStr::new(node.id.as_str()),
                    x: node.x,
                    y: node.y,
                })
                .collect(),
            &def.waypoints
                .edges
                .iter()
                .map(|[from, to]| {
                    (
                        smol_str::SmolStr::new(from.as_str()),
                        smol_str::SmolStr::new(to.as_str()),
                    )
                })
                .collect::<Vec<_>>(),
        ),
    })
}

/// A class's flat `tracked` cost, `0` for impassable.
///
/// `mush_atlas::physics::Cost` is a `u32` of hundredths of a cell; the arena
/// stores a `u16`, which is 655 cells of cost and two orders of magnitude past
/// the shipped table's worst row.
fn move_cost_of(table: &LocomotionTable, class: TerrainClass) -> u16 {
    match table.cost(class.label(), TRACKED, &[]) {
        Some(cost) => u16::try_from(cost).unwrap_or(u16::MAX),
        None => 0,
    }
}

/// Where a tile's south-west corner sits in the window, in cells.
fn tile_origin(
    entry: &str,
    def: &ArenaDef,
    key: &RecordKey,
    tile: &TileRec,
) -> Result<(u16, u16), ArenaBuildError> {
    let mismatch = |detail: String| ArenaBuildError::TileMismatch {
        arena: entry.to_string(),
        tile: key.to_string(),
        detail,
    };
    let cell_cm = i64::from(def.cell_cm);
    let origin_cm = (
        def.origin_m[0].saturating_mul(100),
        def.origin_m[1].saturating_mul(100),
    );
    let tile_cm = (
        mush_atlas::geom::cm(tile.bbox.0.0).map_err(|error| mismatch(error.to_string()))?,
        mush_atlas::geom::cm(tile.bbox.0.1).map_err(|error| mismatch(error.to_string()))?,
    );
    let offset = |tile_cm: i64, origin_cm: i64, axis: &str| -> Result<u16, ArenaBuildError> {
        let delta = tile_cm.saturating_sub(origin_cm);
        if delta < 0 || delta % cell_cm != 0 {
            return Err(mismatch(format!(
                "starts {delta} cm {axis} of the window's corner, which is not a whole \
                 number of {cell_cm} cm cells"
            )));
        }
        u16::try_from(delta / cell_cm).map_err(|_| {
            mismatch(format!(
                "starts {delta} cm {axis} of the window's corner, off the map"
            ))
        })
    };
    Ok((
        offset(tile_cm.0, origin_cm.0, "east")?,
        offset(tile_cm.1, origin_cm.1, "north")?,
    ))
}
