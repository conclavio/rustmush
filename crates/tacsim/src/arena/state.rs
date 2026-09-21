//! **The arena's state** (16b cut A.1; doc 16 §3.1 "Pulse phases", §3.2 "Space",
//! §3.3 "Units", §3.6 "Perception", §4 "Arena state").
//!
//! One [`Arena`] is one match: the definition it runs on, its units in unit-id
//! order, the per-`(unit, channel)` epoch table intents are authorised against,
//! the per-viewer shadow contact store, the due-event heap, the streaming
//! ledger, the clock, and the bounded per-unit queues of discrete actions. Every
//! stored quantity is an integer (doc 16 §3.1 "Determinism").
//!
//! Nothing here reads a clock, a file or a socket: [`Arena::new`] takes the seam
//! message of doc 16 §3.1 ([`CustodyOut`]) and two configuration blocks, and the
//! only way arena time advances is a caller handing [`Clock::advance`] a wall
//! reading.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mush_atlas::TerrainClass;
use mush_core::ObjId;
use mush_sim::data::Pose;
use mush_tactics::TacticalUnit;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use smol_str::SmolStr;

use crate::arena::clock::Clock;
use crate::arena::index::SpatialIndex;
use crate::arena::intents::Intent;
use crate::arena::ledger::{EventKind, Ledger};
use crate::arena::perception::{Eccm, FrameNotice, Jammer, SensorProfile, ThermalSignature};
use crate::arena::resolution::UnitProfile;
use crate::arena::step::FrameSlots;
use crate::arena::weapons::{Shot, WeaponBand};
use crate::config::{ArenaConfig, MatchConfig, SensorsConfig};
use crate::definitions::resolution::ResolutionDef;

/// The digest domain of [`ArenaDefinition::digest`]. Length-prefixed fields
/// follow it, so a definition's digest can never collide with another contract's
/// hash of the same bytes.
pub const DEFINITION_DIGEST_DOMAIN: &[u8] = b"tacsim.arena.definition.v1";

// ---------------------------------------------------------------------------
// Terrain
// ---------------------------------------------------------------------------

/// One cell of the arena window (doc 16 §3.2 "Terrain").
///
/// `move_cost` is the per-cell movement cost for the `tracked` mode, where
/// **`0` means impassable** and the terrain factor of doc 16 §3.4 step 2 is
/// `100 / move_cost`. Cut T fills it from `mush_atlas`'s passability and cost
/// tables; here it is data, so a test can lay down an impassable band with no
/// atlas at all. `class` is carried for cover and opacity, which cut S and cut W
/// consume.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainCell {
    /// The ground class, for cover and opacity.
    pub class: TerrainClass,
    /// The cell's elevation against the region datum, centimetres. A unit's
    /// `z_cm` equals the cell's `elev_cm` (doc 16 §3.2).
    pub elev_cm: i16,
    /// The `tracked` movement cost; `0` is impassable.
    pub move_cost: u16,
}

impl TerrainCell {
    /// Whether the `tracked` mode may enter this cell (doc 16 §3.4 step 4).
    #[must_use]
    pub const fn passable(&self) -> bool {
        self.move_cost != 0
    }
}

/// The arena window's cell grid: row-major, `width × height` cells of `cell_cm`
/// centimetres each (doc 16 §3.2 "The arena window", D-16-5: 256 by 256 at
/// 1,000 cm).
///
/// The fields are private because the length of `cells` is an invariant of the
/// other three: [`ArenaTerrain::new`] is the only way in, and it refuses a grid
/// whose cell count is not `width × height` rather than letting a short vector
/// turn every lookup off the end into a silent `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArenaTerrain {
    width: u16,
    height: u16,
    cell_cm: u16,
    cells: Vec<TerrainCell>,
}

impl ArenaTerrain {
    /// The grid, refusing anything that is not exactly `width × height` cells.
    ///
    /// # Errors
    ///
    /// [`ArenaError::TerrainShape`] for a zero dimension, a zero `cell_cm` (the
    /// divisor of [`ArenaTerrain::cell_of`]) or a cell count that is not the
    /// product.
    pub fn new(
        width: u16,
        height: u16,
        cell_cm: u16,
        cells: Vec<TerrainCell>,
    ) -> Result<ArenaTerrain, ArenaError> {
        let expected = usize::from(width).checked_mul(usize::from(height));
        match expected {
            Some(expected)
                if width != 0 && height != 0 && cell_cm != 0 && cells.len() == expected =>
            {
                Ok(ArenaTerrain {
                    width,
                    height,
                    cell_cm,
                    cells,
                })
            }
            _ => Err(ArenaError::TerrainShape {
                width,
                height,
                cell_cm,
                cells: cells.len(),
            }),
        }
    }

    /// A grid of one repeated cell — the shape a unit test lays down before it
    /// paints a band or an obstacle into it.
    ///
    /// # Errors
    ///
    /// As [`ArenaTerrain::new`].
    pub fn uniform(
        width: u16,
        height: u16,
        cell_cm: u16,
        cell: TerrainCell,
    ) -> Result<ArenaTerrain, ArenaError> {
        let count = usize::from(width).checked_mul(usize::from(height)).ok_or(
            ArenaError::TerrainShape {
                width,
                height,
                cell_cm,
                cells: 0,
            },
        )?;
        ArenaTerrain::new(width, height, cell_cm, vec![cell; count])
    }

    /// The cell at `(x, y)`, or `None` off the window.
    #[must_use]
    pub fn get(&self, x: u16, y: u16) -> Option<&TerrainCell> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = usize::from(y).checked_mul(usize::from(self.width))?;
        self.cells.get(index.checked_add(usize::from(x))?)
    }

    /// The cell a pose stands in, or `None` when the pose is off the window.
    ///
    /// Floor division of `x_cm` and `y_cm` by `cell_cm`. A negative coordinate
    /// is off the window by definition — the window's origin is its corner — so
    /// it answers `None` rather than folding toward zero, and a coordinate at or
    /// past `width × cell_cm` (or `height × cell_cm`) answers `None` too. The
    /// boundary belongs to the cell it opens: at `cell_cm = 1000`, `x_cm = 1000`
    /// is column 1, and `x_cm = 999` is column 0.
    #[must_use]
    pub fn cell_of(&self, pose: &Pose) -> Option<(u16, u16)> {
        self.cell_at(pose.x_cm, pose.y_cm)
    }

    /// The same, from two coordinates rather than from a pose (16b cut F.3) —
    /// what a [`SeenPose`] has and a [`Pose`] is not needed for.
    #[must_use]
    pub fn cell_at(&self, x_cm: i64, y_cm: i64) -> Option<(u16, u16)> {
        if x_cm < 0 || y_cm < 0 {
            return None;
        }
        let cell = i64::from(self.cell_cm);
        let x = u16::try_from(x_cm / cell).ok()?;
        let y = u16::try_from(y_cm / cell).ok()?;
        if x >= self.width || y >= self.height {
            return None;
        }
        Some((x, y))
    }

    /// Columns.
    #[must_use]
    pub const fn width(&self) -> u16 {
        self.width
    }

    /// Rows.
    #[must_use]
    pub const fn height(&self) -> u16 {
        self.height
    }

    /// The side of one cell, centimetres.
    #[must_use]
    pub const fn cell_cm(&self) -> u16 {
        self.cell_cm
    }

    /// Every cell, row-major — the read [`ArenaDefinition::digest`] and cut A.2's
    /// motion take.
    #[must_use]
    pub fn cells(&self) -> &[TerrainCell] {
        &self.cells
    }
}

/// **The arena's window is a cell grid the atlas trace can walk** (16b cut S.1;
/// doc 16 §3.2 "Line of sight").
///
/// This one impl is the whole of how perception traces without a window. Doc 16
/// §3.2 says line of sight *is* the atlas trace, and doc 16 §3.1 (D-16-1, D-16-3)
/// says the arena holds no store, no records and no window — a
/// `mush_atlas::SurfaceComposite` would drag a `SurfaceRef`, a room table and a
/// `FeatureOverlay` in, all of them record types the contract gate exists to keep
/// out. `mush_atlas::CellGrid` is the trace's *terrain* seam and nothing more: one
/// cell's class and its elevation. So the arena keeps its own grid, the atlas keeps
/// its one line walk, and `mush_atlas::trace_cells` is that walk over this.
impl mush_atlas::CellGrid for ArenaTerrain {
    fn cell_at(&self, x: u16, y: u16) -> Option<(TerrainClass, i32)> {
        let cell = self.get(x, y)?;
        Some((cell.class, i32::from(cell.elev_cm)))
    }
}

// ---------------------------------------------------------------------------
// Teams, sides, entry zones and the definition
// ---------------------------------------------------------------------------

/// A side of a match (doc 16 §1: red against blue).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Team {
    /// The red side.
    Red,
    /// The blue side.
    Blue,
}

impl Team {
    /// Both sides, in code order.
    pub const ALL: [Team; 2] = [Team::Red, Team::Blue];

    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Team::Red => "red",
            Team::Blue => "blue",
        }
    }

    /// The other side.
    #[must_use]
    pub const fn other(&self) -> Team {
        match self {
            Team::Red => Team::Blue,
            Team::Blue => Team::Red,
        }
    }

    /// **The letter a side's callsigns carry** (16b cut Q.1; doc 16 §3.6): `R`
    /// for red, `B` for blue, so a classified contact reads `RA`, `BC`.
    ///
    /// A `const fn` over the closed enum and **never the first character of a
    /// display name**: the letter is part of the one name every viewer calls a
    /// unit by, so it has to be a fact about the team and not about whatever a
    /// pack happened to call it. A third side is a compile error here and a
    /// failing test in [`Designator`]'s own module, which is where the rule that
    /// the letters are distinct and none of them is
    /// [`PROVISIONAL_PREFIX`] is written down.
    #[must_use]
    pub const fn letter(&self) -> char {
        match self {
            Team::Red => 'R',
            Team::Blue => 'B',
        }
    }
}

/// A left or right side of a unit — the argument of doc 16 §3.3's
/// `no_turn_toward(left)` consequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The unit's left.
    Left,
    /// The unit's right.
    Right,
}

impl Side {
    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

/// A team's deployment rectangle on the window, in cells, inclusive of both
/// corners (doc 16 §3.2 "Terrain": *two team entry zones*).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryZone {
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

/// **A named place on the window** (16b cut N.1; doc 16 §3.2: *three or four key
/// spots*), as the arena carries it.
///
/// Authored by `crate::definitions::arena::KeySpot` and carried across the seam
/// so that a controller and a pilot can call the same ground by the same name.
/// Cut N.1 carries it; the first reader that *routes* on it is cut N.2's order
/// (`objective` names a spot).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySpot {
    /// What it is called.
    pub name: SmolStr,
    /// Its column.
    pub x: u16,
    /// Its row.
    pub y: u16,
}

/// **One node of the patrol graph** (16b cut N.1; doc 16 §6: *the arena pack's
/// waypoint graph*).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waypoint {
    /// The node's authored id, which is what a ledger line or a report names it
    /// by.
    pub id: SmolStr,
    /// Its column.
    pub x: u16,
    /// Its row.
    pub y: u16,
}

/// **The patrol graph**, as the arena carries it (16b cut N.1).
///
/// The edges are authored as pairs of node *ids* and stored as pairs of node
/// **indices**, resolved once at construction: the reflex controller walks this
/// graph inside a pulse, and an id lookup per step would be a string comparison
/// per edge per tick. `adjacency` is one sorted, deduplicated row per node, so
/// "the next node from here" is a deterministic walk with no allocation and no
/// map.
///
/// An edge naming a node that is not in `nodes` is **dropped** at construction
/// rather than refused: the loader (`crate::definitions::arena::ArenaDef::check`)
/// is where an unresolvable edge is an authoring error, and a window with one
/// bad edge is still a window a match can be fought on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Waypoints {
    nodes: Vec<Waypoint>,
    adjacency: Vec<Vec<u16>>,
}

impl Waypoints {
    /// The graph, with `edges` naming node ids.
    #[must_use]
    pub fn new(nodes: Vec<Waypoint>, edges: &[(SmolStr, SmolStr)]) -> Waypoints {
        let index_of = |id: &SmolStr| -> Option<u16> {
            nodes
                .iter()
                .position(|node| node.id == *id)
                .and_then(|at| u16::try_from(at).ok())
        };
        let mut adjacency: Vec<Vec<u16>> = vec![Vec::new(); nodes.len()];
        for (from, to) in edges {
            let (Some(from), Some(to)) = (index_of(from), index_of(to)) else {
                continue;
            };
            if from == to {
                continue;
            }
            if let Some(row) = adjacency.get_mut(usize::from(from)) {
                row.push(to);
            }
            if let Some(row) = adjacency.get_mut(usize::from(to)) {
                row.push(from);
            }
        }
        for row in &mut adjacency {
            row.sort_unstable();
            row.dedup();
        }
        Waypoints { nodes, adjacency }
    }

    /// Every node, in authored order.
    #[must_use]
    pub fn nodes(&self) -> &[Waypoint] {
        &self.nodes
    }

    /// One node by index.
    #[must_use]
    pub fn node(&self, at: u16) -> Option<&Waypoint> {
        self.nodes.get(usize::from(at))
    }

    /// The nodes `at` is joined to, in index order.
    #[must_use]
    pub fn neighbours(&self, at: u16) -> &[u16] {
        self.adjacency
            .get(usize::from(at))
            .map_or(&[][..], Vec::as_slice)
    }

    /// Whether the graph has no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// **The node nearest a cell**, by squared cell distance, ties to the lower
    /// index — the `unreachable` edge's fallback and `Patrol`'s first goal.
    #[must_use]
    pub fn nearest(&self, x: u16, y: u16) -> Option<u16> {
        let mut best: Option<(u64, u16)> = None;
        for (at, node) in self.nodes.iter().enumerate() {
            let dx = i64::from(node.x) - i64::from(x);
            let dy = i64::from(node.y) - i64::from(y);
            let distance = (dx.saturating_mul(dx)).saturating_add(dy.saturating_mul(dy));
            let distance = u64::try_from(distance).unwrap_or(u64::MAX);
            let at = u16::try_from(at).unwrap_or(u16::MAX);
            if best.is_none_or(|(far, _)| distance < far) {
                best = Some((distance, at));
            }
        }
        best.map(|(_, at)| at)
    }
}

/// The arena a match runs on: its id, its window, its entry zones, its named
/// places and its patrol graph (doc 16 §4's `arenas/proving_ground.json`).
///
/// Immutable for the life of the match. Its [`digest`](ArenaDefinition::digest)
/// goes on the result record (doc 16 §4 "The result record"), which is how a
/// replayed ledger can be told to belong to the terrain it was produced on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArenaDefinition {
    /// The arena's content id.
    pub id: SmolStr,
    /// The window.
    pub terrain: ArenaTerrain,
    /// The deployment rectangles.
    pub entry_zones: Vec<EntryZone>,
    /// **The named places** (16b cut N.1; doc 16 §3.2).
    pub key_spots: Vec<KeySpot>,
    /// **The patrol graph the reflex controller walks** (16b cut N.1; doc 16 §6).
    pub waypoints: Waypoints,
}

impl ArenaDefinition {
    /// The definition's SHA-256 identity over a canonical byte encoding.
    ///
    /// The encoding, in order, with every multi-byte integer little-endian:
    ///
    /// 1. [`DEFINITION_DIGEST_DOMAIN`];
    /// 2. the id's byte length as `u64`, then its bytes;
    /// 3. `width`, `height`, `cell_cm`, each `u16`;
    /// 4. the cell count as `u64`, then per cell row-major: the class's byte
    ///    code as `u8`, `elev_cm` as `i16`, `move_cost` as `u16`;
    /// 5. the zone count as `u64`, then per zone in declared order: the team's
    ///    byte (`0` red, `1` blue) then `x0`, `y0`, `x1`, `y1` as `u16`;
    /// 6. **(16b cut N.1)** the key-spot count as `u64`, then per spot in declared
    ///    order: the name's byte length as `u64`, its bytes, `x` and `y` as `u16`;
    /// 7. **(16b cut N.1)** the waypoint-node count as `u64`, then per node in
    ///    declared order: the id's byte length as `u64`, its bytes, `x` and `y` as
    ///    `u16`, the neighbour count as `u64` and each neighbour index as `u16`.
    ///
    /// Every variable-length run is length-prefixed, so no two different
    /// definitions share an encoding.
    ///
    /// **Why the graph is in it** (16b cut N.1): the digest's job is that *a
    /// replayed ledger can be told to belong to the terrain it was produced on*,
    /// and from cut N a unit's route is a function of the waypoint graph as much
    /// as of the cells. Two windows with one terrain and two graphs produce two
    /// different matches, so they are two different definitions.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        let mut sha = Sha256::new();
        sha.update(DEFINITION_DIGEST_DOMAIN);
        sha.update((self.id.len() as u64).to_le_bytes());
        sha.update(self.id.as_bytes());
        sha.update(self.terrain.width().to_le_bytes());
        sha.update(self.terrain.height().to_le_bytes());
        sha.update(self.terrain.cell_cm().to_le_bytes());
        sha.update((self.terrain.cells().len() as u64).to_le_bytes());
        for cell in self.terrain.cells() {
            sha.update([cell.class as u8]);
            sha.update(cell.elev_cm.to_le_bytes());
            sha.update(cell.move_cost.to_le_bytes());
        }
        sha.update((self.entry_zones.len() as u64).to_le_bytes());
        for zone in &self.entry_zones {
            sha.update([match zone.team {
                Team::Red => 0_u8,
                Team::Blue => 1_u8,
            }]);
            sha.update(zone.x0.to_le_bytes());
            sha.update(zone.y0.to_le_bytes());
            sha.update(zone.x1.to_le_bytes());
            sha.update(zone.y1.to_le_bytes());
        }
        sha.update((self.key_spots.len() as u64).to_le_bytes());
        for spot in &self.key_spots {
            sha.update((spot.name.len() as u64).to_le_bytes());
            sha.update(spot.name.as_bytes());
            sha.update(spot.x.to_le_bytes());
            sha.update(spot.y.to_le_bytes());
        }
        let nodes = self.waypoints.nodes();
        sha.update((nodes.len() as u64).to_le_bytes());
        for (at, node) in nodes.iter().enumerate() {
            sha.update((node.id.len() as u64).to_le_bytes());
            sha.update(node.id.as_bytes());
            sha.update(node.x.to_le_bytes());
            sha.update(node.y.to_le_bytes());
            let neighbours = self
                .waypoints
                .neighbours(u16::try_from(at).unwrap_or(u16::MAX));
            sha.update((neighbours.len() as u64).to_le_bytes());
            for neighbour in neighbours {
                sha.update(neighbour.to_le_bytes());
            }
        }
        sha.finalize().into()
    }
}

// ---------------------------------------------------------------------------
// Mobility and condition
// ---------------------------------------------------------------------------

/// The `tracked` mobility-mode rules of doc 16 §3.3 "Mobility mode rules",
/// decoded out of M32's opaque `ModeRules` by the game crate.
///
/// All-zero (the [`Default`]) is a unit that cannot move at all, which is what a
/// wreck and an unmodelled hull get.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedRules {
    /// The forward ceiling, cm/s.
    pub max_speed_cms: u32,
    /// Acceleration, cm/s².
    pub accel_cms2: u32,
    /// Braking, cm/s² — the rate used when the magnitude of the speed falls.
    pub brake_cms2: u32,
    /// Turn rate, millidegrees per second.
    pub turn_rate_mdeg_s: u32,
    /// Reverse ceiling as a percentage of `max_speed_cms`.
    pub reverse_factor_pct: u8,
}

impl TrackedRules {
    /// The reverse ceiling in cm/s (a non-negative magnitude).
    #[must_use]
    pub const fn reverse_ceiling_cms(&self) -> u32 {
        self.max_speed_cms
            .saturating_mul(self.reverse_factor_pct as u32)
            / 100
    }
}

/// What damage has taken away from a unit's motion — doc 16 §3.3's `slow(50)`,
/// `no_turn_toward(left)`, `disable_motion` and `wreck` consequences, read off
/// the M32 unit by the game crate and cached here so a substep of motion needs
/// no section walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MotionLimits {
    /// The surviving fraction of the speed cap, per cent; `100` is no slow.
    pub slow_pct: u8,
    /// A side the unit can no longer turn toward (a destroyed track).
    pub no_turn_toward: Option<Side>,
    /// Motion is disabled outright (a destroyed engine).
    pub motion_disabled: bool,
    /// The unit is a wreck (doc 16 §3.3): an obstacle and a contact, never a
    /// target, and no longer a subject of any order.
    pub wreck: bool,
}

impl Default for MotionLimits {
    /// Undamaged: full speed, both tracks, motion enabled, not a wreck.
    fn default() -> MotionLimits {
        MotionLimits {
            slow_pct: 100,
            no_turn_toward: None,
            motion_disabled: false,
            wreck: false,
        }
    }
}

/// **The fractions of a turn, of a speed change and of a distance that a substep
/// did not spend** (16b cut Q.6; doc 16 §3.1 "Rate independence", §3.4).
///
/// Motion is three rates over arena time integrated into integer state, and
/// before this cut each of the three was written `rate × dt / 1000` and the
/// remainder thrown away — three truncations per unit per substep. That makes the
/// *substep length* an input to the answer: a tank at 1,500 cm/s crossing a
/// 250 ms substep travels 375 cm, and the same tank across five 50 ms substeps
/// travels five times 75, which agrees only because those numbers happen to be
/// whole. They usually are not, and a crawl is the plainest case: a unit ordered
/// to 3 cm/s covered 0.75 cm in a 250 ms substep, which truncated to nothing, so
/// it stood still for as long as it was driven.
///
/// **X.6's five-against-one `blocked` count was read as the same defect and is
/// not one** (16b cut Q.6 measured it): the carries removed every truncation in
/// motion and that count did not move — 250 ms still prints one and 150 ms still
/// prints five. Its cause is the *two flags* cut F.2 split deliberately, and
/// `crate::arena::motion`'s `one_obstacle_is_one_stop_at_every_partition` test
/// measures it rather than asserting it away.
///
/// So each rate keeps its remainder, exactly the way
/// [`HeatState::carry_milli`](crate::arena::state::HeatState::carry_milli) keeps
/// the fraction of a heat unit that a substep did not shed
/// ([`crate::arena::heat`]'s head is the argument, written once). The total
/// turned, gained or travelled over any interval is then the same however that
/// interval is cut — see [`crate::arena::motion`]'s head for exactly which
/// motions that makes exact and which it does not.
///
/// **Every field is folded into [`crate::arena::hash::state_hash`]**, because a
/// carry is the fractional part of authoritative state: two replays whose carries
/// differ are two replays that are about to disagree about a whole centimetre.
///
/// **A refused substep banks nothing.** Every stop, every wreck, every dead
/// engine and every cold hull zeroes what it refused, which is
/// [`crate::arena::heat`]'s own rule for a unit that has gone cold: a carry
/// belongs to arena time that was actually spent moving.
/// It carries **no serde derives**: [`UnitState`] itself has none, the carries
/// have no chosen wire form, and nothing in the tree encodes one (16b cut Q.6,
/// fixer). A cut that gives them one chooses the form then.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MotionCarry {
    /// Millidegrees × 1,000 not yet turned, in `0..1000`.
    pub turn: u32,
    /// cm/s × 1,000 not yet gained or shed, in `0..1000`.
    pub speed: u32,
    /// Centimetres × `1000 << 20` not yet travelled eastward, in `0..(1000 << 20)`
    /// — the scale is the millisecond's thousand times
    /// [`SIN_ONE`](crate::arena::motion::SIN_ONE)'s fixed point, because the east
    /// component is `speed × dt × sin` and neither division is taken until the
    /// carry has been added.
    pub east: i64,
    /// The same northward.
    pub north: i64,
}

/// A unit's power state (doc 16 §3.3 "Startup").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Power {
    /// Cold. Motion and fire refuse.
    #[default]
    Off,
    /// `STARTUP` has been accepted and `match.startup_ms` has not elapsed.
    Starting,
    /// Running.
    Online,
}

impl Power {
    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Power::Off => "off",
            Power::Starting => "starting",
            Power::Online => "online",
        }
    }

    /// The byte [`crate::arena::hash::state_hash`] folds in.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Power::Off => 0,
            Power::Starting => 1,
            Power::Online => 2,
        }
    }
}

// ---------------------------------------------------------------------------
// The seam message
// ---------------------------------------------------------------------------

/// How high a unit's eye sits when nothing says otherwise, centimetres (doc 16
/// §3.2 "Line of sight": *`sensor_height_cm` (new field; 150 by default)*).
pub const DEFAULT_SENSOR_HEIGHT_CM: u32 = 150;

/// **The rounds one weapon's own `ammo` cell carries** (16b cut F.1), or `None`
/// when the unit declares no such cell.
///
/// The mirror of [`crate::arena::damage::Arena::returned_units`]'s fold: that
/// writes the arena's live count back into `component/<weapon>/ammo` at check-in,
/// and this reads it at check-out, so a match opens on the magazine the last one
/// left. Negative values cannot occur (a cell is bounded) and are floored at zero
/// rather than wrapping.
fn rounds_on_record(unit: &mush_tactics::TacticalUnit, weapon: &SmolStr) -> Option<u32> {
    let addr = mush_tactics::CellAddr::Component {
        component: mush_tactics::ComponentId::new(weapon.as_str()).ok()?,
        cell: mush_tactics::CellId::new(crate::definitions::unit::AMMO_CELL).ok()?,
    };
    let held = unit.cell(&addr)?;
    Some(u32::try_from(held.current.max(0)).unwrap_or(u32::MAX))
}

/// **What a unit does with heat** (16b cut X.1; doc 16 §9 item 1: *a `heat` cell
/// and dissipation appear on the unit*).
///
/// Everything about heat that is **not** the number itself. The number is an M32
/// cell — `heat` on the unit's body section — because doc 16 §3.5 makes the M32
/// unit the truth of a unit's condition cells and because the check-in hands the
/// cells back whole; this is the arena's map to it, read once at check-out.
///
/// It is read and not carried across the seam. `CustodyUnit` is a struct six test
/// files build literally, so a new field on it would be a diff in every one of
/// them; the numbers are already on the `TacticalUnit` that crosses beside it, and
/// [`HeatState::read`] finds them there. The same reasoning is why the shot's own
/// cost is a cell on the weapon's component rather than a field on
/// [`WeaponSummary`].
///
/// A unit whose content declares no ceiling has **no** `HeatState` at all, which
/// is what makes the per-substep dissipation an `Option` test for every stock unit
/// in the harness's field of 300.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeatState {
    /// The section the `heat` and `heat_sink` cells sit on — found rather than
    /// derived, so the rule for *which* section lives in one place
    /// (`crate::definitions::unit::UnitSpec::heat_section`).
    pub section: SmolStr,
    /// The ceiling, off the `heat` cell's own maximum. `FIRE` refuses at it.
    pub max: u32,
    /// What the unit sheds per second of arena time, off the `heat_sink` cell.
    pub per_s: u32,
    /// What one shot of each weapon adds, by the weapon's component id; a weapon
    /// with no row makes no heat.
    pub per_shot: BTreeMap<SmolStr, u32>,
    /// **The thousandths of a heat unit the substeps have shed and not yet
    /// spent**, `0..1000`.
    ///
    /// Doc 16's model is rate-independent (D-16-2) and integer (D-16-4), and the
    /// two pull against each other here: `tempo.substep_max_ms` is 250, so
    /// `per_s × dt_ms / 1000` truncates to nothing for any rate under four a
    /// second and a tank at `Engaged` would never cool at all. Carrying the
    /// remainder makes the total shed over any interval exactly
    /// `per_s × ms / 1000` **however the interval is chopped**, which is the
    /// property the cross-tempo test asserts of everything else in the model.
    ///
    /// It is arena-side scratch and not state: it is reset with the heat it
    /// belongs to, never leaves the match, and is not folded into the state hash —
    /// for the same reason the opaque `TacticalUnit` is not
    /// ([`crate::arena::hash::state_hash`]).
    pub carry_milli: u32,
}

impl HeatState {
    /// **Read one unit's heat numbers off its cells** (16b cut X.1), or `None`
    /// for a unit that carries no `heat` cell — every stock unit.
    ///
    /// The section is found by probing each section the unit's profile declares
    /// for the cell, so `bind`'s choice of body section is authoritative and this
    /// never re-derives it. A unit with no profile (every hand-made rig) has no
    /// section to probe and no heat.
    #[must_use]
    pub fn read(
        unit: &TacticalUnit,
        profile: &UnitProfile,
        weapons: &[WeaponSummary],
    ) -> Option<HeatState> {
        let (section, cell) = profile.sections.iter().find_map(|section| {
            let addr = heat_addr(section.id.as_str(), crate::definitions::unit::HEAT_CELL)?;
            unit.cell(&addr).map(|cell| (section.id.clone(), *cell))
        })?;
        let per_s = heat_addr(section.as_str(), crate::definitions::unit::HEAT_SINK_CELL)
            .and_then(|addr| unit.cell(&addr))
            .map_or(0, |cell| u32::try_from(cell.current.max(0)).unwrap_or(0));
        let per_shot = weapons
            .iter()
            .filter_map(|weapon| {
                let addr = mush_tactics::CellAddr::Component {
                    component: mush_tactics::ComponentId::new(weapon.id.as_str()).ok()?,
                    cell: mush_tactics::CellId::new(crate::definitions::unit::HEAT_PER_SHOT_CELL)
                        .ok()?,
                };
                let value = u32::try_from(unit.cell(&addr)?.current.max(0)).ok()?;
                (value > 0).then(|| (weapon.id.clone(), value))
            })
            .collect();
        Some(HeatState {
            section,
            max: u32::try_from(cell.max.max(0)).unwrap_or(0),
            per_s,
            per_shot,
            carry_milli: 0,
        })
    }

    /// The address of one of this unit's heat cells.
    #[must_use]
    pub fn addr(&self, cell: &str) -> Option<mush_tactics::CellAddr> {
        heat_addr(self.section.as_str(), cell)
    }

    /// What one shot of `weapon` adds; `0` for a weapon that makes no heat.
    #[must_use]
    pub fn per_shot(&self, weapon: &str) -> u32 {
        self.per_shot.get(weapon).copied().unwrap_or(0)
    }
}

/// One section-cell address, or `None` for an id `mush-tactics` will not take.
fn heat_addr(section: &str, cell: &str) -> Option<mush_tactics::CellAddr> {
    Some(mush_tactics::CellAddr::Section {
        section: mush_tactics::SectionId::new(section).ok()?,
        cell: mush_tactics::CellId::new(cell).ok()?,
    })
}

/// **One weapon, as the check-out hands it over** (16b cut S.1; doc 16 §3.5
/// "Weapon definition").
///
/// Cut S needs exactly one number off it — the longest `max_range_cm` in the
/// match, which is what `tempo.alert_range_cm` is derived from when the key is
/// zero (16b §9) — and cut W needs all of them, so the whole row crosses the seam
/// once at check-out rather than twice. Every field is an integer (D-16-4).
///
/// **16b cut W.1 added [`WeaponSummary::bands`] and
/// [`WeaponSummary::count_per_load`]**, which are the two numbers a shot needs and
/// S.1 had no reader for: the load sizes the unit's `ammo` cell at check-out, and
/// the bands carry the per-band `dispersion_mrad` W.2's error cone is built from.
/// The row is the **definition**, immutable for the life of the match; what a shot
/// changes is the two live cells beside it on [`UnitState`]
/// (`ready_at_ms` and `ammo`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeaponSummary {
    /// The component id the pilot names in `FIRE <weapon>`.
    pub id: SmolStr,
    /// The outermost range band's edge, centimetres. Equal to the largest
    /// [`WeaponBand::max_range_cm`] of [`WeaponSummary::bands`]; carried
    /// separately because it is the one number cut S reads and a check-out with no
    /// authored bands still has a reach.
    pub max_range_cm: u32,
    /// The cycle, milliseconds (floor 1,000, W14 — asserted by the content loader,
    /// `crate::definitions::weapon::CYCLE_FLOOR_MS`, and taken as authored here).
    pub cycle_ms: u32,
    /// The ammunition family it fires.
    pub ammunition: SmolStr,
    /// Rounds per pull.
    pub burst_rounds: u32,
    /// Muzzle speed, cm/s; zero exactly for a beam (doc 16 §3.5: a beam's
    /// `impact_at_ms` is `now`).
    pub muzzle_speed_cms: u32,
    /// **The range bands, nearest first** (16b cut W.1; doc 16 §3.5 step 2), each
    /// with the weapon's own error cone in that band.
    pub bands: Vec<WeaponBand>,
    /// **What one round does** (16b cut W.2; doc 16 §3.5 "Ammunition definition":
    /// *`damage` (one fixed value)*), off the ammunition the weapon names — the
    /// second field of this row that is the ammunition's and not the weapon's.
    /// Zero for a weapon whose ammunition the catalog does not hold, which is a
    /// weapon that arrives and does nothing rather than one that refuses.
    pub damage: u32,
    /// **The section it is mounted in** (16b cut W.2; doc 16 §3.5 "Weapon
    /// definition": *`mount` (section and facet)*), which is what
    /// `remove_weapons(section)` names when a turret is shot off.
    pub mount_section: SmolStr,
    /// **How many rounds one load holds** (16b cut W.1), off the ammunition
    /// definition. It is what [`Arena::new`] sizes the unit's `ammo` cell from and
    /// what `WEAPONSTATUS` prints as the denominator.
    pub count_per_load: u32,
}

/// **Who is flying a unit** (16b cut N.1; doc 16 §6, §3.8 "Disconnects"; doc 07
/// §1.1's fallback controller).
///
/// A unit attribute at custody rather than a property of the match, because the
/// two coexist in one match: a lobby that fields fewer pilots than tanks puts
/// [`Controller::Reflex`] on the empty pods, and a seat released past the
/// linkdead grace hands one tank over mid-match while the others keep their
/// pilots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Controller {
    /// A player's verbs are the only source of this unit's orders.
    #[default]
    Pilot,
    /// The reflex controller ([`crate::arena::reflex`]) drives it.
    Reflex,
}

impl Controller {
    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Controller::Pilot => "pilot",
            Controller::Reflex => "reflex",
        }
    }

    /// The byte [`crate::arena::hash::state_hash`] folds in.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Controller::Pilot => 0,
            Controller::Reflex => 1,
        }
    }
}

/// One unit as it is handed over at check-out (doc 16 §3.1 "Custody").
#[derive(Clone, Debug)]
pub struct CustodyUnit {
    /// The world object the unit is.
    pub id: ObjId,
    /// Its side.
    pub team: Team,
    /// Its pose at check-out, which must be on the window.
    pub pose: Pose,
    /// Its decoded `tracked` rules.
    pub mobility: TrackedRules,
    /// **How high its eye sits**, centimetres (doc 16 §3.2; 16b cut S.1). Zero is
    /// read as [`DEFAULT_SENSOR_HEIGHT_CM`].
    pub sensor_height_cm: u32,
    /// **What it sees with** (16b cut S.1; doc 16 §3.6). `None` runs no sensor
    /// pass at all.
    pub sensor: Option<SensorProfile>,
    /// **What it shoots with** (16b cut S.1 carries it; 16b cut W.1 reads it).
    pub weapons: Vec<WeaponSummary>,
    /// **What it presents and what a hit may strike** (16b cut W.2; doc 16 §3.3
    /// "Facet attribution" and "Silhouette"), decoded off the authored `UnitSpec`
    /// at check-out. An empty profile presents nothing and is struck by nothing,
    /// which is what every fixture from cut A carries.
    pub profile: UnitProfile,
    /// **The gunnery rating resolving its shots** (16b cut W.2; doc 16 §3.5 step
    /// 2's `PilotProfile` seam), 1 to 5. Zero is read as
    /// [`crate::arena::resolution::STOCK_GUNNERY`] — *every stock rating is 1*.
    pub gunnery: u8,
    /// **Who flies it** (16b cut N.1). The check-out sets [`Controller::Reflex`]
    /// for a unit with no seated pilot and [`Controller::Pilot`] otherwise.
    pub controller: Controller,
    /// **The centre slot this unit was fielded from**, 1-based (16b cut Q.1), or
    /// `0` for a unit no lobby placed — a staff `@arena/start`, a fixture, the
    /// harness's field of three hundred.
    ///
    /// It is what [`Designator::Slot`] renders, so a pilot's `BA` is the pod
    /// `SIM STATUS` prints as `1` on blue. A `0` is not a designator: [`Arena::new`]
    /// gives every such unit the lowest free slot of its team in unit-id order, so
    /// a harness unit and a staff-fielded tank have stable callsigns with no lobby
    /// at all.
    pub slot: u8,
    /// M32 unit truth, carried opaque: cut A applies no plan to it (D-16-6).
    pub unit: TacticalUnit,
}

/// One seated pilot at check-out (doc 16 §3.1's `pilots[{unit, station, epoch}]`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pilot {
    /// The tank the station is on (doc 16 §3.3 "Stations are on the tank").
    pub unit: ObjId,
    /// The character in the seat.
    pub pilot: ObjId,
    /// The station's owner-local id.
    pub station: SmolStr,
    /// The authority epoch the seat was granted under.
    pub epoch: u64,
}

/// **The check-out message** (doc 16 §3.1 "The seam is seven messages"): the
/// arena definition, the units, the seated pilots and the match seed.
///
/// It is the only way state enters an [`Arena`]. The seed is the key of every
/// draw the match makes ([`crate::arena::rng`]).
#[derive(Clone, Debug)]
pub struct CustodyOut {
    /// The window the match runs on.
    pub definition: ArenaDefinition,
    /// The units, in any order; [`Arena::new`] sorts them.
    pub units: Vec<CustodyUnit>,
    /// The seated pilots.
    pub pilots: Vec<Pilot>,
    /// **The resolution table every shot of this match is resolved through** (16b
    /// cut W.2; doc 16 §3.5 steps 2 and 3), decoded off the content at check-out.
    ///
    /// The one content type the arena carries whole rather than in a copy of its
    /// own, and deliberately: `ResolutionDef::hit_permille` *is* doc 16 §3.5 step
    /// 3's interpolation, written and tested beside the table it walks, and a
    /// second copy of that walk in here would be the one place the cockpit's
    /// percentage and the draw's threshold could come to disagree.
    pub resolution: ResolutionDef,
    /// The match seed.
    pub seed: u64,
}

// ---------------------------------------------------------------------------
// Unit state
// ---------------------------------------------------------------------------

/// One unit's arena state (doc 16 §4 "Arena state").
#[derive(Clone, Debug)]
pub struct UnitState {
    /// The world object the unit is. The units vector is sorted on it.
    pub id: ObjId,
    /// Its side.
    pub team: Team,
    /// **The name every viewer that has told it apart calls it by** (16b cut
    /// Q.1): always a [`Designator::Slot`], resolved once at check-out from
    /// [`CustodyUnit::slot`] and fixed for the match.
    ///
    /// It is **not** folded into [`crate::arena::hash::state_hash`] on its own: it
    /// is a pure function of the custody message, it never moves, and two replays
    /// of one check-out resolve it identically — the same argument
    /// `slope_limit_pct` makes.
    pub callsign: Designator,
    /// Its pose: whole centimetres and millidegrees (doc 16 §3.2).
    pub pose: Pose,
    /// Its current speed along its heading, cm/s. **Negative is reverse.**
    pub speed_cms: i32,
    /// The helm's ordered heading, millidegrees in `0..360_000`.
    pub desired_heading_mdeg: u32,
    /// The helm's ordered speed, cm/s, negative for reverse.
    pub desired_speed_cms: i32,
    /// Power (doc 16 §3.3 "Startup").
    pub power: Power,
    /// The decoded `tracked` rules.
    pub mobility: TrackedRules,
    /// **The steepest edge this unit's locomotion will cross**, per cent (16b
    /// cut X.6; doc 16 §9 item 6), or `None` for no limit at all.
    ///
    /// Read off the M32 unit at check-out by
    /// [`crate::arena::slope::SlopeLimit::read`], beside [`HeatState::read`] and
    /// the three sight reads, and for the same reason: it is a fact about the
    /// unit's fit that nothing in a match moves, and `CustodyUnit` is a struct a
    /// dozen test files build literally.
    ///
    /// `None` is **no limit**, not a limit of zero — a unit whose content
    /// declares no locomotion the shipped table knows still drives, and still
    /// pays the slope's surcharge. It is not folded into
    /// [`crate::arena::hash::state_hash`], for the reason `jammer` is not: it
    /// never moves, and what it *does* is the refusals the hash already folds
    /// through the poses.
    pub slope_limit_pct: Option<u32>,
    /// **The uphill slope of the edge this unit entered its cell by**, per cent
    /// (16b cut X.6; doc 16 §3.4 step 2).
    ///
    /// Written by [`crate::arena::motion::integrate`] on every accepted
    /// crossing, beside `pose.z_cm`, and read by the terrain factor of the next
    /// substep: a tank half way up a ramp is held to `max_speed × 100 /
    /// (move_cost × (100 + grade) / 100)`. `0` on flat ground, which makes the
    /// factor exactly what it was before this cut.
    ///
    /// It **is** folded into [`crate::arena::hash::state_hash`], unlike the
    /// limit: it is authoritative state with one writer, it moves without a
    /// ledger event of its own, and a unit can sit in a cell for many substeps
    /// after the crossing that set it.
    pub grade_pct: u16,
    /// **What the last substep of motion did not spend** (16b cut Q.6; doc 16
    /// §3.1 "Rate independence") — the fractional turn, speed change and distance
    /// that would otherwise be truncated away three times a substep.
    ///
    /// Written only by [`crate::arena::motion::integrate`], and folded into
    /// [`crate::arena::hash::state_hash`] field by field: see [`MotionCarry`].
    pub carry: MotionCarry,
    /// What damage has taken from its motion.
    pub limits: MotionLimits,
    /// How high its eye sits, centimetres (doc 16 §3.2 "Line of sight").
    pub sensor_height_cm: u32,
    /// What it sees with (doc 16 §3.6); `None` runs no pass.
    pub sensor: Option<SensorProfile>,
    /// **What its sight makes of a target's heat** (16b cut X.3; doc 16 §9 item
    /// 3), or `None` for a sight with no thermal channel — every stock unit, so
    /// the term costs the pass one `Option` test per candidate.
    ///
    /// Read off the cells at check-out rather than carried across the seam, for
    /// the reason [`HeatState`] gives: `CustodyUnit` and the sensor row are
    /// structs a dozen test files build literally.
    pub thermal: Option<ThermalSignature>,
    /// **The jamming pod it carries** (16b cut X.4; doc 16 §9 item 4), or `None`
    /// for a unit that mounts none — every stock unit, so a stock match's sensor
    /// pass never scans for one.
    ///
    /// Read off the cells at check-out for the reason [`ThermalSignature`] is, and
    /// not folded into [`crate::arena::hash::state_hash`] for the reason the sensor
    /// row is not: it is a fact about the unit's fit that nothing in a match moves,
    /// and what a pod *does* shows up in the contacts the hash already folds.
    pub jammer: Option<Jammer>,
    /// **The counter-jamming filter it carries** (16b cut X.5; doc 16 §9 item 5), or
    /// `None` for a unit that mounts none — every stock unit, so a stock pass runs
    /// the arithmetic cut X.4 left it.
    ///
    /// Read off the cells at check-out for the reason [`ThermalSignature`] is, and
    /// left out of [`crate::arena::hash::state_hash`] for the reason `jammer` is: it
    /// is a fact about the unit's fit that nothing in a match moves, and what a
    /// filter *does* shows up in the contacts the hash already folds.
    ///
    /// It matters only to **this** unit's own pass: a filter is carried by the
    /// observer and recovers nothing for anybody standing beside it.
    pub eccm: Option<Eccm>,
    /// **What this platform's last sensor pass ran under** (16b cut D.4, `B-22`;
    /// 16c §5 Q17), or `None` for a receiver that is clean — or off. See
    /// [`Receiver`] for the three `None`s and the two declared limits.
    ///
    /// Written by [`Arena::sensor_pass`] alone, at the pass's own arena
    /// timestamp, from the fold `Arena::detections` already made. It is read by
    /// the frame and by the notice and by nothing else: no rule of the arena
    /// decides anything by it.
    ///
    /// Unlike `jammer` and `eccm` beside it, it **is** folded into
    /// [`crate::arena::hash::state_hash`]: those are facts about the unit's fit
    /// that never move, and this is state that persists between passes, decides a
    /// notice, and can move with no ledger line of its own (a pass that stops
    /// running writes `None` silently).
    pub receiver: Option<crate::arena::perception::Receiver>,
    /// What it shoots with — the definition rows, in component order.
    ///
    /// **16b cut W.2 made it mutable in one way and one only**: doc 16 §3.3's
    /// `remove_weapons(section)` consequence empties the rows mounted on a section
    /// that has been shot off, which is what makes a turreted tank stop being able
    /// to fire. Nothing else edits it, and no row's own numbers ever change.
    pub weapons: Vec<WeaponSummary>,
    /// **What it presents and what a hit may strike** (16b cut W.2; doc 16 §3.3).
    pub profile: UnitProfile,
    /// **The gunnery rating resolving its shots** (16b cut W.2), 1 to 5.
    pub gunnery: u8,
    /// **What it does with heat** (16b cut X.1; doc 16 §9 item 1), or `None` for
    /// a unit whose content declares none — every stock unit, so the per-substep
    /// dissipation costs the harness one `Option` test per unit.
    pub heat: Option<HeatState>,
    /// **Who flies it** (16b cut N.1; doc 16 §6). Set at check-out and moved by
    /// exactly one thing afterwards: a [`crate::arena::intents::Control`] that
    /// releases or restores the seat (doc 16 §3.8 "Disconnects").
    pub controller: Controller,
    /// M32 unit truth, carried opaque — and, from 16b cut W.2, written by exactly
    /// one thing: one [`mush_tactics::UnitMutationPlan`] per impact through
    /// `mush_tactics::apply` (doc 16 §3.5 "Damage").
    ///
    /// **16b cut Q.2 made "exactly one thing" true of the assignment too.** There
    /// were two production writers — the impact in `crate::arena::damage` and the
    /// heat plan in `crate::arena::heat` — and a cache beside a field with two
    /// writers is a cache that rots. [`Arena::set_unit`] is the only assignment of
    /// this field in non-test code, and it is what keeps
    /// [`UnitState::condition`] true of it.
    pub unit: TacticalUnit,
    /// **What a gunner would read off this tank**, cached (16b cut Q.2; doc 16
    /// §3.6), recomputed by [`Arena::set_unit`] on every write of
    /// [`UnitState::unit`] and by [`Arena::new`] at check-out.
    ///
    /// A cache and not authoritative state: it is a pure function of `unit` and of
    /// the two `[tacsim.sensors]` floors, neither of which moves inside a match
    /// except through the one writer. It is therefore **not** folded into
    /// [`crate::arena::hash::state_hash`] on its own — `unit` is what M32
    /// journals, and hashing a function of it would fold the same truth twice —
    /// while the *readings* copied off it onto contacts are folded, because those
    /// are knowledge and they go stale.
    ///
    /// It exists because a sensor pass reads it once per close classified
    /// detection, and walking five sections and their facets per detection per
    /// pass is exactly the per-pulse cost doc 16 §2 W4 bounds.
    pub condition: crate::arena::condition::Condition,
    /// **Per-weapon ready time in arena milliseconds** (16b cut W.1 fills A.1's
    /// map): the arena millisecond at or after which that weapon may fire again,
    /// set to `now + cycle_ms` by an accepted shot. One row per weapon, laid down
    /// at check-out at zero — a complete table, so a lookup is never the difference
    /// between "ready" and "no such weapon".
    pub ready_at_ms: BTreeMap<SmolStr, i64>,
    /// **Rounds left, per weapon** (16b cut W.1), laid down at check-out from each
    /// weapon's [`WeaponSummary::count_per_load`] and decremented by the burst an
    /// accepted shot fires. One row per weapon, as `ready_at_ms` is.
    pub ammo: BTreeMap<SmolStr, u32>,
    /// The platform's lock (16b cut W.1 gives `LOCK` its effect): the contact
    /// `FIRE` shoots at, which is always one this platform holds — a contact the
    /// platform loses clears it (`crate::arena::step`'s `lost` arm).
    pub lock: Option<ContactId>,
    /// Set by motion when this unit was stopped at a cell boundary during the
    /// substep just integrated, and cleared at the start of the next (doc 16
    /// §3.4 step 4: *one `blocked` notice per stop*).
    pub blocked: bool,
    /// **It is standing against something** (16b cut F.2), which is the condition
    /// `STATUS` prints `HELD` for and the edge the pilot's one notice per stop
    /// fires on.
    ///
    /// Not [`UnitState::blocked`], which is what the *substep just integrated*
    /// did: a tank ordered forward against a river bank covers no whole
    /// centimetre on some substeps, never reaches the boundary to be refused at,
    /// and so read `0 kph` with no `HELD` — the defect the 2026-09-18 smoke found.
    /// This is set by a stop, cleared by a substep that covered ground or by the
    /// order being taken off ([`crate::arena::motion::integrate`]), and left alone
    /// in between.
    ///
    /// **Output only.** Nothing in the arena decides anything by it — the reflex
    /// controller still reads `blocked`, and the ledger's `blocked` event is still
    /// that flag's rising edge — and it is deliberately **not** folded into
    /// [`crate::arena::hash::state_hash`], for the reason `held_losses` is not: it
    /// is a fact about what a pilot is being shown.
    pub held: bool,
    /// The seat has gone linkdead (doc 07 §1.2): the tank keeps its continuous
    /// orders and accepts no discrete action.
    pub linkdead: bool,
    /// **It has no weapons and cannot move** (16b cut W.2; doc 16 §3.8's *a wreck
    /// or disabled with no weapons*, doc 10 3A's *disabled precedes destroyed*).
    ///
    /// Set once, on the transition, by the consequence that finished it off, and
    /// journalled as `unit_disabled` then. It is **not** folded into the state
    /// hash: every transition into it is a ledger event, so the ledger's own
    /// sequence already moves with it (see [`crate::arena::hash::state_hash`]'s
    /// note on what is deliberately absent).
    pub disabled: bool,
    /// **How many shots it has fired** (16b cut W.2), for the result record's
    /// per-unit row (doc 16 §4).
    pub shots_fired: u32,
    /// **How many of them landed** (16b cut W.2), likewise.
    pub hits: u32,
}

// ---------------------------------------------------------------------------
// Contacts
// ---------------------------------------------------------------------------

/// **The arena's own contact identity** (doc 16 §3.6, D-16-10).
///
/// A random 64-bit value minted through [`crate::arena::rng::draw_contact_id`]
/// with the `contact` label — never sequential, never reused inside a match, and
/// rendered as sixteen lowercase base-16 digits. It is deliberately a **distinct
/// type** from `mush_core::ContactId`: the durable `tactics/contacts` family is
/// the world's to write and the arena cannot reach it, so the two identity
/// spaces must not be assignable to each other. Contacts die with the match; the
/// ledger's `detected`, `classified` and `lost` events are the record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContactId(pub u64);

impl std::fmt::Display for ContactId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// **The letter a provisional designator opens with** (16b cut Q.1).
///
/// Pinned distinct from every [`Team::letter`] by a unit test below, so `X2` can
/// never be read as a slot on some later side.
pub const PROVISIONAL_PREFIX: char = 'X';

/// **The short name a pilot reads and types a contact by** (16b cut Q.1; doc 16
/// §3.6 "What a pilot sees").
///
/// The owner's first hand-played match found the defect this type answers:
/// `LOCK 29b81a…` is sixteen base-16 digits to type in the heat of a fight, and
/// nothing shorter existed. So a contact is shown — and resolved — as one of two
/// things:
///
/// * [`Designator::Provisional`] — `X1`, `X2`, …, numbered **per viewer** in the
///   order that platform's own sensors first found things. It is viewer-scoped
///   and says nothing about the target at all, which is exactly what a platform
///   holding an unclassified return knows.
/// * [`Designator::Slot`] — the team's letter and the unit's centre slot in
///   bijective base 26 (`RA`, `BC`, `BAA` past twenty-six a side). It is the
///   **same for every viewer**, which is the point: once a tank has been told
///   apart, two pilots on one side are talking about `BA` and mean one machine.
///
/// **Which one a contact renders as is read off the live contact and never off a
/// flag.** A contact is not durable — [`ShadowContacts::expire`] removes the row
/// and the next detection re-inserts it `Unknown` under the same id — so a sticky
/// "has been classified" bit would name a fresh blip by team and slot. See
/// [`DesignatorBook`] and `Arena::designator_for`.
///
/// No `serde`: it is a rendering of arena state, and every wire form carries the
/// string [`std::fmt::Display`] produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Designator {
    /// A viewer-scoped number, from `1` (`X1`).
    Provisional(u16),
    /// A team letter and a 1-based centre slot (`RA`).
    Slot {
        /// The side.
        team: Team,
        /// The slot, from `1`.
        slot: u16,
    },
}

impl std::fmt::Display for Designator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Designator::Provisional(n) => write!(f, "{PROVISIONAL_PREFIX}{n}"),
            Designator::Slot { team, slot } => {
                write!(f, "{}{}", team.letter(), bijective_base26(*slot))
            }
        }
    }
}

/// **A 1-based slot in bijective base 26** (16b cut Q.1): `1` is `A`, `26` is
/// `Z`, `27` is `AA`.
///
/// Bijective and not ordinary base 26, because there is no zero slot: the sequence
/// has to run `Z`, `AA` rather than `Z`, `BA`, and every slot has exactly one
/// spelling. A slot of `0` — which [`Arena::new`] never produces, since it resolves
/// every unit onto a slot from `1` — renders as the empty string rather than
/// panicking, so a malformed designator is a name nothing resolves and not a
/// crash.
fn bijective_base26(slot: u16) -> String {
    let mut left = u32::from(slot);
    let mut letters: Vec<char> = Vec::new();
    while left > 0 {
        // `(left - 1) % 26` is under 26, so the narrowing is exact and the
        // fallback is unreachable.
        let index = u8::try_from((left.saturating_sub(1)) % 26).unwrap_or(0);
        letters.push(char::from(b'A'.saturating_add(index)));
        left = left.saturating_sub(1) / 26;
    }
    letters.reverse();
    letters.into_iter().collect()
}

/// **Give every unit its public callsign** (16b cut Q.1), over the sorted unit
/// vector [`Arena::new`] has just built.
///
/// Two passes per side, and the order is the whole of why a replay agrees:
///
/// 1. **Explicit slots first.** A unit the lobby fielded from pod `n` takes slot
///    `n`, whatever else is on the side. Two units of one side in one slot is
///    [`ArenaError::DuplicateSlot`] — they would answer to one name.
/// 2. **Then the rest, in unit-id order**, each taking the lowest slot of its
///    side that pass 1 did not claim and pass 2 has not. That is what gives a
///    staff `@arena/start`, an AI-filled pod with no centre row and the harness's
///    field of three hundred stable callsigns with no lobby at all.
///
/// The vector arrives sorted by id, so pass 2's "in unit-id order" needs no sort
/// of its own.
///
/// # Errors
///
/// [`ArenaError::DuplicateSlot`], naming the side and the slot.
fn resolve_callsigns(states: &mut [UnitState]) -> Result<(), ArenaError> {
    for team in Team::ALL {
        let mut taken: Vec<u16> = Vec::new();
        for state in states.iter().filter(|state| state.team == team) {
            let Designator::Slot { slot, .. } = state.callsign else {
                continue;
            };
            if slot == 0 {
                continue;
            }
            if taken.contains(&slot) {
                return Err(ArenaError::DuplicateSlot {
                    team: team.as_str(),
                    slot: u8::try_from(slot).unwrap_or(u8::MAX),
                });
            }
            taken.push(slot);
        }
        let mut next: u16 = 1;
        for state in states.iter_mut().filter(|state| state.team == team) {
            if state.callsign != (Designator::Slot { team, slot: 0 }) {
                continue;
            }
            while taken.contains(&next) {
                next = next.saturating_add(1);
            }
            state.callsign = Designator::Slot { team, slot: next };
            taken.push(next);
            next = next.saturating_add(1);
        }
    }
    Ok(())
}

/// How much a viewer knows about what it is holding (doc 16 §3.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    /// Held, not yet told apart from anything else.
    Unknown,
    /// Told apart: the viewer knows whose it is.
    Classified {
        /// The side the viewer has resolved.
        team: Team,
    },
}

impl Classification {
    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Classification::Unknown => "unknown",
            Classification::Classified { .. } => "classified",
        }
    }

    /// The side this viewer has resolved, if it has.
    #[must_use]
    pub const fn team(&self) -> Option<Team> {
        match self {
            Classification::Unknown => None,
            Classification::Classified { team } => Some(*team),
        }
    }

    /// The byte [`crate::arena::hash::state_hash`] folds in: `0` unknown, `1`
    /// classified red, `2` classified blue.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Classification::Unknown => 0,
            Classification::Classified { team: Team::Red } => 1,
            Classification::Classified { team: Team::Blue } => 2,
        }
    }
}

/// **Where a target was, and how it was moving, at the sighting that a contact
/// records** (16b cut F.3; doc 16 §3.6 read as the owner's principle that
/// knowledge is earned and goes stale).
///
/// Five integers and not a [`Pose`]: a pose carries the driver's opaque posture
/// blob, and what a plot needs is the three coordinates the one range function
/// takes, the heading the facet band reads, and the speed the error cone's
/// crossing term reads. All five are things a platform learns by *looking*, which
/// is why the plot carries them rather than any reader asking the target. Copying
/// five integers per refresh is the whole cost of the contact store knowing
/// *where it saw something* rather than where that something now is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenPose {
    /// East, region centimetres.
    pub x_cm: i64,
    /// North, region centimetres.
    pub y_cm: i64,
    /// Height against the region datum, centimetres.
    pub z_cm: i32,
    /// Which way the target was facing, millidegrees clockwise from north.
    pub heading_mdeg: u32,
    /// How fast it was making along that heading, centimetres a second, negative
    /// in reverse — the input to the advertised error cone's crossing term
    /// (`crate::arena::resolution::Arena::resolve_seen`).
    pub speed_cms: i32,
}

impl SeenPose {
    /// The plot a unit makes, at this instant.
    #[must_use]
    pub const fn of(unit: &UnitState) -> SeenPose {
        SeenPose {
            x_cm: unit.pose.x_cm,
            y_cm: unit.pose.y_cm,
            z_cm: unit.pose.z_cm,
            heading_mdeg: unit.pose.heading_mdeg,
            speed_cms: unit.speed_cms,
        }
    }

    /// The range from `pose` to this plot, centimetres — the same arithmetic
    /// [`crate::arena::index::range_cm`] does over two poses, so a plot's range
    /// and a true range are one function.
    #[must_use]
    pub fn range_from(&self, pose: &Pose) -> u64 {
        crate::arena::index::range_between(
            pose.x_cm, pose.y_cm, pose.z_cm, self.x_cm, self.y_cm, self.z_cm,
        )
    }

    /// The bearing from `pose` to this plot, millidegrees clockwise from north.
    #[must_use]
    pub fn bearing_from(&self, pose: &Pose) -> u32 {
        crate::arena::perception::bearing_of(
            self.x_cm.saturating_sub(pose.x_cm),
            self.y_cm.saturating_sub(pose.y_cm),
        )
    }

    /// The window cell this plot is in, or `None` for a plot off the window.
    #[must_use]
    pub fn cell(&self, terrain: &ArenaTerrain) -> Option<(u16, u16)> {
        terrain.cell_at(self.x_cm, self.y_cm)
    }
}

/// One contact a viewer platform holds (doc 16 §3.6's contact lifecycle as
/// vocabulary, in RAM).
///
/// **16b cut F.3 gave it its provenance and its plot.** Cut S.1 stored when the
/// contact was last refreshed and nothing about *who* refreshed it or *where the
/// target was*, and the two omissions cost the same thing: a contact that never
/// expired (two meshed teammates refreshing each other's copy for ever) and a
/// contact reported at the target's live pose (a tank tracked through a wall for
/// the whole time-to-live). Both are answered here, in the store, rather than at
/// every reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowContact {
    /// What is being held.
    pub target: ObjId,
    /// How well.
    pub class: Classification,
    /// Arena milliseconds at first detection.
    pub first_seen_ms: i64,
    /// Arena milliseconds at the last refresh — the input to expiry. Either this
    /// platform's own sensors or a teammate's share set it, and a share never
    /// raises it above the sharer's own sighting (16b cut F.3).
    pub last_seen_ms: i64,
    /// **Where the target was at the sighting that set [`ShadowContact::last_seen_ms`]**
    /// (16b cut F.3) — this platform's own, or the sharer's own as the share
    /// carried it.
    ///
    /// Every reader of a contact's position reads this and never the target's
    /// live pose: it is a plot, not a tether. While a target is in continuous view
    /// the plot is at most one `sensors.pass_ms` old.
    pub seen_at: SeenPose,
    /// **This platform's OWN last sighting** (16b cut F.3): when its own sensors
    /// saw the target and where the target was — or `None` for a copy that only
    /// ever arrived over the mesh.
    ///
    /// It is what a share is allowed to be built from (doc 16 §2 W6: shared
    /// contacts fan out only within comms range *of the platform that sees*), so a
    /// mesh copy is never relayed onward and two teammates cannot keep one
    /// another's copy alive after the last real sighting.
    ///
    /// **The stamp and the plot are one field** rather than two beside
    /// [`ShadowContact::seen_at`], because a share hands over both and they have
    /// to be the same sighting. Held apart, a platform whose own sighting is older
    /// than a teammate's share would relay its own *time* against the teammate's
    /// *place* — a second hop for the position, which is exactly what rule 1
    /// exists to stop.
    pub own_seen: Option<OwnSighting>,
    /// **The per-section bands this platform holds on the target**, with the
    /// arena timestamp of the look that took them (16b cut Q.2; doc 16 §3.6) —
    /// this platform's own, or a teammate's as a share carried it.
    ///
    /// `None` until the platform has had a **close look** at a contact that is
    /// **classified** (`crate::arena::perception::Detection`'s `close`), which is
    /// what makes a reading earned rather than a free consequence of detection.
    /// It is what `SCAN` prints, and it dies with the contact row: the store's
    /// `expire` removes the row, so a re-acquired blip is `Unknown` again and
    /// reads nothing until it has been told apart and looked at a second time.
    ///
    /// It is **not** [`OwnSighting::condition`]: this one may be a teammate's,
    /// and only this platform's own may be put back on the mesh.
    pub seen_condition: Option<crate::arena::condition::Reading>,
    /// **This hold was opened under a provisional number** (16b cut D.4, `B-21`).
    ///
    /// `true` at exactly one site — the `None` arm of [`Arena::sensor_pass`], with
    /// `[tacsim.sensors].designate_on_detection` off, where
    /// `Designators::assign` issues or re-uses the number — and never written
    /// again for the life of the row. `false` for a mesh arrival
    /// ([`Arena::share_pass`]'s insert) and in [`ShadowContact::sighted`]'s own
    /// default.
    ///
    /// It exists because the designator **book** is permanent and a hold is not.
    /// `Classified`'s `was` used to read the book, so a target this platform
    /// numbered in an earlier hold, lost, and then received over the mesh printed
    /// `New contact BA: …` and at once `Contact X2 identified: BA, blue.` — a
    /// rename of a row the pilot is not looking at. Read off the hold, a mesh
    /// arrival says nothing after its acquisition and a row this platform's own
    /// sensors opened still says the rename.
    ///
    /// It **is** folded into [`crate::arena::hash::state_hash`], because it moves
    /// with no distinguishable ledger event: a mesh `detected` and a sensor
    /// `detected` are byte for byte the same line, which is the argument cut Q.1
    /// made for the book itself.
    pub numbered: bool,
}

/// **One platform's own sighting of a target** (16b cut F.3): the moment, and
/// where the target was at it. See [`ShadowContact::own_seen`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnSighting {
    /// Arena milliseconds at the sighting.
    pub at_ms: i64,
    /// Where the target stood, as this platform's own sensors took it.
    pub plot: SeenPose,
    /// **The bands this platform's OWN sensors last read off the target** (16b
    /// cut Q.2), with the timestamp of *that look* and not of this sighting.
    ///
    /// The two stamps differ on purpose. A sighting is every pass the platform
    /// holds the target; a **look** is only the passes it held the target
    /// classified and inside the close-look reach. So this field **carries
    /// forward** across refreshes that did not earn a new reading, keeping its own
    /// older stamp, and a share hands the receiver a reading that is honestly as
    /// old as the look behind it.
    ///
    /// It is what may travel on the mesh, which is why it rides here and not
    /// beside [`ShadowContact::seen_condition`]: a copy of somebody else's reading
    /// must never be relayed a second hop (doc 16 §2 W6).
    pub condition: Option<crate::arena::condition::Reading>,
}

impl ShadowContact {
    /// **A contact this platform's own sensors have just taken** at `t_ms`, with
    /// the target where it stood at that sighting (16b cut F.3).
    ///
    /// **16b cut Q.2**: and with no reading of any kind. A contact this
    /// constructor makes is `Unknown` at every production call site, and an
    /// unidentified return has no section vocabulary to read bands into; the
    /// sensor pass writes one later, on a pass that finds the contact classified
    /// and close.
    #[must_use]
    pub const fn sighted(
        target: ObjId,
        class: Classification,
        t_ms: i64,
        seen_at: SeenPose,
    ) -> ShadowContact {
        ShadowContact {
            target,
            class,
            first_seen_ms: t_ms,
            last_seen_ms: t_ms,
            seen_at,
            own_seen: Some(OwnSighting {
                at_ms: t_ms,
                plot: seen_at,
                condition: None,
            }),
            seen_condition: None,
            // **16b cut D.4**: the constructor says nothing about the book, and
            // the one site that issues a number sets it. A fixture that builds a
            // contact by hand is therefore a hold with no number behind it, which
            // is the honest default.
            numbered: false,
        }
    }

    /// **What this platform may put on the mesh**, or `None` when it has nothing
    /// to say about this contact (16b cut F.3).
    ///
    /// Both of the cut's share rules, in the one place they are written, so
    /// [`Arena::sensor_pass`]'s *is there anything to say* and
    /// [`Arena::share_pass`]'s *what is there to say* cannot answer differently:
    /// the contact is **classified** (an `unknown` smudge means nothing to
    /// anybody else), and this platform's **own** sensors saw it inside the time
    /// to live.
    #[must_use]
    pub fn shareable(&self, cutoff_ms: i64) -> Option<(Team, OwnSighting)> {
        let team = self.class.team()?;
        let own = self.own_seen.filter(|own| own.at_ms >= cutoff_ms)?;
        Some((team, own))
    }
}

/// **The one byte-identical refusal for a contact id a platform does not hold**
/// (doc 16 §3.6: *unresolvable, unowned or expired* are one answer, and it is the
/// same answer a nonexistent unit gets).
///
/// It lives here, beside [`ShadowContacts::resolve`] — the only lookup a pilot's
/// id ever goes through — so that the rule and its sentence cannot be separated.
/// `crate::match_::cockpit` re-exports it under K.2's name, so the line a pilot
/// reads exists exactly once in the tree.
pub const NO_SUCH_CONTACT: &str = "No such contact.";

/// **How often one spent section may be said**, arena milliseconds (16b cut D.2,
/// `B-15`).
///
/// The window [`Arena::spent_said`] is read against, for the victim and for the
/// gunner who holds the target classified. It is arena time and not wall time and
/// not a per-pulse count, which is what makes the decision the same at every
/// cadence.
pub const SPENT_NOTICE_MS: i64 = 5_000;

/// **The per-viewer shadow contact store** (doc 16 §3.6): viewer platform to its
/// contacts, contact id to what it holds.
///
/// Cut A defined it and cut S.1's sensor pass fills it. It is a `BTreeMap` of
/// `BTreeMap` rather than a hash map so that iteration order is the state hash's
/// order and a replay cannot diverge on it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShadowContacts {
    by_viewer: BTreeMap<ObjId, BTreeMap<ContactId, ShadowContact>>,
}

impl ShadowContacts {
    /// An empty store.
    #[must_use]
    pub fn new() -> ShadowContacts {
        ShadowContacts::default()
    }

    /// Record or refresh a contact for `viewer`.
    pub fn insert(&mut self, viewer: ObjId, id: ContactId, contact: ShadowContact) {
        self.by_viewer
            .entry(viewer)
            .or_default()
            .insert(id, contact);
    }

    /// What `viewer` holds under `id`, if anything.
    ///
    /// An unresolvable, unowned or expired id is indistinguishable from a
    /// nonexistent one — doc 16 §3.6's byte-identical refusal is exactly this
    /// `None`.
    #[must_use]
    pub fn get(&self, viewer: ObjId, id: ContactId) -> Option<&ShadowContact> {
        self.by_viewer.get(&viewer)?.get(&id)
    }

    /// The same, for refreshing in place.
    pub fn get_mut(&mut self, viewer: ObjId, id: ContactId) -> Option<&mut ShadowContact> {
        self.by_viewer.get_mut(&viewer)?.get_mut(&id)
    }

    /// **The one lookup a pilot's named id goes through** (16b cut S.1; doc 16
    /// §3.6's existence-hiding rule).
    ///
    /// Byte for byte the same as [`ShadowContacts::get`] and deliberately a
    /// different name: every surface that resolves an id a *pilot typed* calls
    /// this, and the rule that goes with it is that its `None` becomes
    /// [`NO_SUCH_CONTACT`] and never a sentence about which of the three reasons
    /// applied. An id that was never minted, one minted for another platform and
    /// one this platform has let expire are one answer, so an unauthorised viewer
    /// cannot learn that a unit exists by naming it.
    #[must_use]
    pub fn resolve(&self, viewer: ObjId, id: ContactId) -> Option<&ShadowContact> {
        self.get(viewer, id)
    }

    /// The id `viewer` already holds `target` under, if any — so a refresh does
    /// not mint a second id for one target.
    #[must_use]
    pub fn id_of(&self, viewer: ObjId, target: ObjId) -> Option<ContactId> {
        self.by_viewer
            .get(&viewer)?
            .iter()
            .find(|(_, contact)| contact.target == target)
            .map(|(id, _)| *id)
    }

    /// Everything `viewer` holds, in contact-id order.
    pub fn of(&self, viewer: ObjId) -> impl Iterator<Item = (&ContactId, &ShadowContact)> {
        self.by_viewer.get(&viewer).into_iter().flatten()
    }

    /// **Every contact every viewer holds**, viewer object number ascending then
    /// contact id ascending — the two `BTreeMap`s' own order, which is why
    /// [`crate::arena::hash::state_hash`] can fold the store without a sort.
    pub fn all(&self) -> impl Iterator<Item = (ObjId, ContactId, &ShadowContact)> {
        self.by_viewer.iter().flat_map(|(viewer, held)| {
            held.iter()
                .map(move |(id, contact)| (*viewer, *id, contact))
        })
    }

    /// Drop one contact, answering what was dropped.
    pub fn remove(&mut self, viewer: ObjId, id: ContactId) -> Option<ShadowContact> {
        let held = self.by_viewer.get_mut(&viewer)?.remove(&id)?;
        if self.by_viewer.get(&viewer).is_some_and(BTreeMap::is_empty) {
            self.by_viewer.remove(&viewer);
        }
        Some(held)
    }

    /// Drop everything `viewer` has not refreshed since `before_ms`, answering
    /// the losses in contact-id order so the caller can journal one `lost` event
    /// each.
    pub fn expire(&mut self, viewer: ObjId, before_ms: i64) -> Vec<(ContactId, ShadowContact)> {
        let mut lost: Vec<(ContactId, ShadowContact)> = Vec::new();
        if let Some(held) = self.by_viewer.get_mut(&viewer) {
            let stale: Vec<ContactId> = held
                .iter()
                .filter(|(_, contact)| contact.last_seen_ms < before_ms)
                .map(|(id, _)| *id)
                .collect();
            for id in stale {
                if let Some(contact) = held.remove(&id) {
                    lost.push((id, contact));
                }
            }
            if held.is_empty() {
                self.by_viewer.remove(&viewer);
            }
        }
        lost
    }

    /// Forget every contact a viewer held (its platform is a wreck, or has left).
    pub fn clear_viewer(&mut self, viewer: ObjId) {
        self.by_viewer.remove(&viewer);
    }

    /// How many viewers hold anything.
    #[must_use]
    pub fn viewers(&self) -> usize {
        self.by_viewer.len()
    }

    /// How many contacts are held in total.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_viewer.values().map(BTreeMap::len).sum()
    }

    /// Whether anything at all is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// **The provisional numbers one match has issued** (16b cut Q.1), per viewer.
///
/// It holds numbers and **nothing else** — no class, no "has been classified"
/// flag, no team. That is the whole of its design, and it is the correction the
/// plan's first draft needed: a contact is not a durable object
/// ([`ShadowContacts::expire`] removes the row and the next detection re-inserts
/// it [`Classification::Unknown`] under the same id), so any bit that outlived the
/// row would name a freshly re-acquired blip by team and slot on every surface
/// that reads it. What a viewer *now* knows is read off the live contact
/// (`Arena::designator_for`); this only answers *which number this platform's own
/// sensors gave that target the first time they found it*.
///
/// **One writer**: `Arena::sensor_pass`'s `None` arm — the insert of an `Unknown`
/// contact by the platform's own sensors — and only while
/// `[tacsim.sensors].designate_on_detection` is off. A mesh arrival writes
/// nothing, so a number always means *something my own sensors found*, and the
/// sequence stays contiguous: a mesh-first target that later expires and is found
/// by this platform's own sight takes the next number then.
///
/// A number is **never reused** and a re-detected target **keeps** its number —
/// the same continuity the sixteen-digit id always had, since
/// `crate::arena::rng::draw_contact_id` mints one id per `(viewer, target)` pair
/// for the match.
///
/// **It is folded into [`crate::arena::hash::state_hash`]** (that function's own
/// note says why): it moves without a distinguishable ledger event, because a
/// mesh `detected` and a sensor `detected` are the same ledger line. It is not
/// journalled: the ledger names units by object id, which is the canon's
/// vocabulary, and a replay in a pilot's own vocabulary is not this cut's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DesignatorBook {
    /// The next number each viewer will issue, from `1`.
    next: BTreeMap<ObjId, u16>,
    /// `(viewer, target)` to the number that viewer gave that target.
    of: BTreeMap<(ObjId, ObjId), u16>,
}

impl DesignatorBook {
    /// The number `viewer` holds `target` under, minting the viewer's next one if
    /// it holds none.
    ///
    /// Saturating at [`u16::MAX`], which a match cannot reach: a viewer would have
    /// to detect 65,535 distinct targets, and the arena's own unit cap is far
    /// under that.
    pub fn assign(&mut self, viewer: ObjId, target: ObjId) -> u16 {
        if let Some(held) = self.of.get(&(viewer, target)) {
            return *held;
        }
        let next = self.next.entry(viewer).or_insert(1);
        let issued = *next;
        *next = next.saturating_add(1);
        self.of.insert((viewer, target), issued);
        issued
    }

    /// The number `viewer` holds `target` under, if it holds one.
    #[must_use]
    pub fn provisional(&self, viewer: ObjId, target: ObjId) -> Option<u16> {
        self.of.get(&(viewer, target)).copied()
    }

    /// The `next` table, viewer ascending — [`crate::arena::hash::state_hash`]'s
    /// walk.
    pub fn counters(&self) -> impl Iterator<Item = (ObjId, u16)> {
        self.next.iter().map(|(viewer, next)| (*viewer, *next))
    }

    /// How many viewers have issued a number — the hash's length prefix, read off
    /// the map rather than by counting the iterator, because the hash runs once a
    /// substep.
    #[must_use]
    pub fn counters_len(&self) -> usize {
        self.next.len()
    }

    /// How many numbers have been issued in all, likewise.
    #[must_use]
    pub fn issued_len(&self) -> usize {
        self.of.len()
    }

    /// Every issued number as `(viewer, target, n)`, viewer then target
    /// ascending — the `BTreeMap`'s own order, which is why the hash folds it
    /// without a sort.
    pub fn issued(&self) -> impl Iterator<Item = (ObjId, ObjId, u16)> {
        self.of
            .iter()
            .map(|((viewer, target), n)| (*viewer, *target, *n))
    }
}

// ---------------------------------------------------------------------------
// Channels and the epoch table
// ---------------------------------------------------------------------------

/// A control channel over a tank (doc 16 §3.3 "Stations are on the tank",
/// doc 09 C8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// Heading, speed and startup.
    Helm,
    /// Contacts and locks.
    Sensors,
    /// Weapons.
    Fire,
}

impl Channel {
    /// Every channel, in code order — the set a cockpit station grants.
    pub const ALL: [Channel; 3] = [Channel::Helm, Channel::Sensors, Channel::Fire];

    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Channel::Helm => "helm",
            Channel::Sensors => "sensors",
            Channel::Fire => "fire",
        }
    }

    /// The channel a word names, or `None`.
    #[must_use]
    pub fn parse(word: &str) -> Option<Channel> {
        match word {
            "helm" => Some(Channel::Helm),
            "sensors" => Some(Channel::Sensors),
            "fire" => Some(Channel::Fire),
            _ => None,
        }
    }
}

/// **The arena's authority epoch table** (doc 16 §3.1, doc 09 C8): the epoch
/// each `(unit, channel)` is currently at.
///
/// An [`Intent`] is validated at the world when it is enqueued and **authorised
/// at apply** here, against this table. The world raises the table through a
/// [`crate::arena::intents::Control`] on every seat activation, deactivation,
/// lease revocation, linkdead onset and revival, `EXIT` and `SIM ABORT`; the
/// arena then purges every queued intent below the new epoch, which is why a
/// pilot who types `FIRE` then `EXIT` in one burst never fires.
///
/// The table **never lowers**. A `Control` that arrives out of order, or twice,
/// cannot reopen an epoch a later one closed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EpochTable {
    epochs: BTreeMap<(ObjId, Channel), u64>,
}

impl EpochTable {
    /// An empty table: every `(unit, channel)` is at epoch `0`.
    #[must_use]
    pub fn new() -> EpochTable {
        EpochTable::default()
    }

    /// Raise `(unit, channel)` to `epoch`, answering whether it moved.
    ///
    /// Never lowers: a lower or equal `epoch` is a no-op answering `false`.
    pub fn raise(&mut self, unit: ObjId, channel: Channel, epoch: u64) -> bool {
        let slot = self.epochs.entry((unit, channel)).or_insert(0);
        if epoch > *slot {
            *slot = epoch;
            true
        } else {
            false
        }
    }

    /// The current epoch of `(unit, channel)`; `0` when the table has never held
    /// it.
    #[must_use]
    pub fn current(&self, unit: ObjId, channel: Channel) -> u64 {
        self.epochs.get(&(unit, channel)).copied().unwrap_or(0)
    }

    /// Whether `epoch` is the **exact** current epoch. A stale epoch is refused
    /// and so is one from the future: only the epoch the world last published is
    /// authority.
    #[must_use]
    pub fn authorises(&self, unit: ObjId, channel: Channel, epoch: u64) -> bool {
        self.current(unit, channel) == epoch
    }

    /// How many `(unit, channel)` pairs the table holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.epochs.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.epochs.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Due events
// ---------------------------------------------------------------------------

/// What a scheduled event does when its arena timestamp comes round (doc 16
/// §3.1 "Rate independence").
///
/// Cut A schedules [`DueKind::StartupComplete`] and [`DueKind::PositionSummary`]
/// and nothing else; the remaining kinds exist so that the heap, the ordering
/// and the state hash are built once and the later cuts add bodies rather than
/// shapes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DueKind {
    /// `match.startup_ms` after `STARTUP` was accepted: power goes online.
    StartupComplete {
        /// The unit starting up.
        unit: ObjId,
    },
    /// **A weapon's cycle has finished** (16b cut W.1). It changes no state —
    /// readiness is a comparison against `now_ms` — and exists so that
    /// `next_due_ms` wakes the pulse when the breech comes free.
    WeaponReady {
        /// The unit.
        unit: ObjId,
        /// The weapon's component id.
        weapon: SmolStr,
    },
    /// **A shot's flight time has elapsed** (16b cut W.1): the arena resolves it
    /// through `Arena::resolve_impact`.
    Impact {
        /// The shot's sequence number, which is also its draw label's argument.
        shot_seq: u64,
    },
    /// One unit's sensor pass (cut S).
    SensorPass {
        /// The observing unit.
        unit: ObjId,
        /// The pass's sequence number, which is also its draw label's argument.
        pass_seq: u64,
    },
    /// One unit's contact share (cut S).
    Share {
        /// The sharing unit.
        unit: ObjId,
    },
    /// **One AI unit's decision** (16b cut N.1; doc 16 §6), due every
    /// `arena.reflex.decide_ms` of arena time and staggered by unit id.
    ///
    /// It is a due event and not a phase-4 pass for the reason every other
    /// cadence in this engine is one: a due event is applied at **its own arena
    /// timestamp**, inside the substep whose boundary that timestamp fell in, so
    /// the same decisions happen at the same arena milliseconds at 5,000, 1,000
    /// and 500 ms. A controller run once per *pulse* would run ten times as often
    /// at the fast cadence as at the slow one, which is doc 16 §3.1's D-16-2
    /// exactly backwards — the tempo would be deciding outcomes.
    ReflexTick {
        /// The unit deciding.
        unit: ObjId,
        /// The decision's sequence number for that unit, from `0`.
        tick_seq: u64,
    },
    /// Every `ledger.summary_ms` of arena time, in every tempo (doc 16 §4).
    PositionSummary,
}

/// One scheduled event: its arena timestamp, its insertion sequence, and what it
/// does.
///
/// The derived order is `(t_ms, seq, kind)`, and `seq` is unique per heap, so it
/// is exactly doc 16 §3.1 phase 3(c)'s *timestamp order then event-sequence
/// order* — total, and independent of how many pulses have run.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DueEvent {
    /// When, in arena milliseconds.
    pub t_ms: i64,
    /// The monotonic insertion sequence, unique within the heap.
    pub seq: u64,
    /// What.
    pub kind: DueKind,
}

/// **The due-event heap**: a min-heap by `(t_ms, seq)` (doc 16 §4 "Arena state").
///
/// Implemented over a `BTreeSet` rather than a `BinaryHeap` so that
/// [`DueHeap::peek_due`] can hand out a reference to the minimum and
/// [`DueHeap::iter`] can walk the schedule in order — both of which cut A.2's
/// `step` and the frame builder want, and neither of which a binary heap gives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DueHeap {
    events: BTreeSet<DueEvent>,
    next_seq: u64,
}

impl DueHeap {
    /// An empty heap. The first scheduled event gets sequence `1`, so `0` is
    /// available as "no event".
    #[must_use]
    pub fn new() -> DueHeap {
        DueHeap::default()
    }

    /// Schedule `kind` at `t_ms`, answering its sequence number.
    pub fn schedule(&mut self, t_ms: i64, kind: DueKind) -> u64 {
        let seq = self.next_seq.saturating_add(1);
        self.next_seq = seq;
        self.events.insert(DueEvent { t_ms, seq, kind });
        seq
    }

    /// The earliest event, when it is due at or before `until_ms`.
    #[must_use]
    pub fn peek_due(&self, until_ms: i64) -> Option<&DueEvent> {
        self.events.first().filter(|event| event.t_ms <= until_ms)
    }

    /// Take the earliest event, when it is due at or before `until_ms`.
    pub fn pop_due(&mut self, until_ms: i64) -> Option<DueEvent> {
        if self.peek_due(until_ms).is_some() {
            self.events.pop_first()
        } else {
            None
        }
    }

    /// When the next event is due — half of `next_due_ms` (doc 16 §3.1 phase 5).
    #[must_use]
    pub fn next_t_ms(&self) -> Option<i64> {
        self.events.first().map(|event| event.t_ms)
    }

    /// The whole schedule, in `(t_ms, seq)` order.
    pub fn iter(&self) -> impl Iterator<Item = &DueEvent> {
        self.events.iter()
    }

    /// Drop every event whose kind `predicate` accepts, answering how many went
    /// — how a unit's scheduled work is cancelled when it becomes a wreck.
    pub fn retain(&mut self, mut predicate: impl FnMut(&DueKind) -> bool) -> usize {
        let before = self.events.len();
        let kept: BTreeSet<DueEvent> = std::mem::take(&mut self.events)
            .into_iter()
            .filter(|event| predicate(&event.kind))
            .collect();
        self.events = kept;
        before.saturating_sub(self.events.len())
    }

    /// How many events are scheduled — folded into the state hash.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether nothing is scheduled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Stopping and ending
// ---------------------------------------------------------------------------

/// Why the world asked the arena to stop (doc 16 §3.1's `Stop` message).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// `SIM ABORT`, or an empty side.
    Abort,
    /// `match.max_duration_ms`.
    Timeout,
    /// The match is void: nothing it derived may be committed.
    Void,
    /// **A side gave up** (16b cut D.3, `B-33`): `SIM CONCEDE`, by that side's
    /// first pilot.
    ///
    /// It carries the side that **conceded**, not the side that won, because that
    /// is the fact the world has: the arena turns it into
    /// [`EndReason::Conceded`], whose `winner` is the other one.
    Concede {
        /// The side that gave up.
        by: Team,
    },
    /// **Every pilot has left the match** (16b cut D.3, `B-33`).
    ///
    /// `[tacsim.match].unattended_ms` of world time after the last un-`left` slot
    /// went `left` during `Running`, the lobby's 1 s tick sends this. Doc 16 §11's
    /// N.1 amendment makes a released seat one-way, so a match every human has
    /// walked out of otherwise runs to `match.max_duration_ms` — an hour of
    /// machines driving at each other in a centre nobody can claim a slot in.
    Unattended,
}

/// How a match ended (doc 16 §4 "The result record").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// A side was eliminated; `None` is a draw (both sides gone at once).
    Elimination {
        /// The surviving side, or `None` for a draw.
        winner: Option<Team>,
    },
    /// **Nobody left on either side can fire a round** (16b cut Q.7), and the
    /// last one in flight has landed: a **draw**, naming no winner.
    ///
    /// It is deliberately not a redefinition of doc 16 §3.8's elimination, which
    /// stays *a wreck, or disabled with no weapons*: counting an empty magazine
    /// as out of action would make a solo player at full armour lose the instant
    /// their last round left the breech. This is the symmetric reading instead —
    /// a side that is dry while the other is not plays on, and only when neither
    /// can shoot is there nothing left to decide. Before it, such a match ran to
    /// `match.max_duration_ms`, which is an hour of two tanks driving at each
    /// other.
    Spent,
    /// **A side conceded** (16b cut D.3, `B-33`): `SIM CONCEDE`, by that side's
    /// first pilot, and the other side wins.
    ///
    /// It is an end and not an abort: the match was fought, so its cells are
    /// committed and the tanks stand as evidence exactly as after an elimination
    /// ([`crate::match_::MatchState::Ended`], and the winner's own outcome word).
    Conceded {
        /// The side that did **not** concede.
        winner: Team,
    },
    /// **Every pilot had left it** (16b cut D.3, `B-33`): `[tacsim.match]`'s
    /// `unattended_ms` elapsed with every slot `left`.
    ///
    /// A `timeout` in the result record's vocabulary (doc 16 §4 has six words and
    /// this is not a seventh), and not a void: the match really ran and what its
    /// machines did to each other is evidence like any other.
    Unattended,
    /// `match.max_duration_ms`, or `ledger.max_bytes` reached (doc 16 §4).
    Timeout,
    /// `SIM ABORT`.
    Abort,
    /// A panic or a watchdog: no arena-derived cell may be committed.
    Void,
    /// The bounded egress backed up past twice `ledger.max_unflushed_chunks`.
    LedgerOverflow,
}

/// The end of a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchEnd {
    /// Why.
    pub reason: EndReason,
    /// When, in arena milliseconds.
    pub t_ms: i64,
}

/// What can refuse an [`Arena`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ArenaError {
    /// The terrain's dimensions and its cell vector disagree, or a dimension or
    /// the cell size is zero.
    #[error(
        "arena terrain is {width} by {height} cells of {cell_cm} cm with {cells} cells: a window \
         is a non-zero width and height of non-zero cells, and holds exactly width x height of them"
    )]
    TerrainShape {
        /// Columns.
        width: u16,
        /// Rows.
        height: u16,
        /// The declared cell size.
        cell_cm: u16,
        /// How many cells were supplied.
        cells: usize,
    },
    /// Two custody units carry one object id.
    #[error("unit {unit} was handed over twice: one object is one unit")]
    DuplicateUnit {
        /// The repeated id.
        unit: ObjId,
    },
    /// A unit's pose is not on the window.
    #[error(
        "unit {unit} was handed over at ({x_cm}, {y_cm}) cm, which is off the arena window: every \
         unit starts on a cell of the definition it is checked out into"
    )]
    PoseOffWindow {
        /// The unit.
        unit: ObjId,
        /// Its east coordinate.
        x_cm: i64,
        /// Its north coordinate.
        y_cm: i64,
    },
    /// A unit's heading is not a bearing.
    #[error(
        "unit {unit} was handed over facing {heading_mdeg} millidegrees: a heading is 0..360000"
    )]
    BadHeading {
        /// The unit.
        unit: ObjId,
        /// The offered heading.
        heading_mdeg: u32,
    },
    /// Two units were handed over standing in one cell.
    #[error(
        "units {unit} and {other} were both handed over in cell ({x}, {y}): occupancy is one unit \
         per cell, so a check-out places them apart"
    )]
    CellOccupied {
        /// The unit that could not be placed.
        unit: ObjId,
        /// The unit already in the cell.
        other: ObjId,
        /// The column.
        x: u16,
        /// The row.
        y: u16,
    },
    /// A pilot names a unit that was not handed over.
    #[error(
        "pilot {pilot} is seated at station {station:?} on unit {unit}, which was not handed over: a \
         station's subject is a unit in custody"
    )]
    UnknownPilotUnit {
        /// The unit named.
        unit: ObjId,
        /// The pilot.
        pilot: ObjId,
        /// The station.
        station: SmolStr,
    },
    /// No units were handed over.
    #[error("no units were handed over: an arena with no units has no match to run")]
    NoUnits,
    /// **16b cut Q.1** — two units of one side were fielded from one centre slot,
    /// so they would answer to one callsign.
    #[error(
        "two units on the {team} side were handed over in slot {slot}: a slot is one unit, and a \
         callsign is one unit's name"
    )]
    DuplicateSlot {
        /// The side.
        team: &'static str,
        /// The repeated slot.
        slot: u8,
    },
}

// ---------------------------------------------------------------------------
// The arena
// ---------------------------------------------------------------------------

/// **One match** (doc 16 §4 "Arena state").
///
/// Every field is public because cut A.2's `motion`, `index` and `step` are
/// siblings in this module tree and the discipline that matters is the one the
/// contract test enforces — no I/O — not field privacy inside the library.
///
/// The two configuration blocks are **cloned in**, so `step` needs no
/// configuration argument and cannot be handed a different tempo table halfway
/// through a match.
#[derive(Clone, Debug)]
pub struct Arena {
    /// The window and its entry zones.
    pub definition: ArenaDefinition,
    /// The units, **sorted by id** — the order every deterministic walk uses.
    pub units: Vec<UnitState>,
    /// The authority epochs intents are authorised against.
    pub epochs: EpochTable,
    /// The per-viewer shadow contacts (cut S fills it).
    pub contacts: ShadowContacts,
    /// **The provisional numbers this match has issued** (16b cut Q.1). See
    /// [`DesignatorBook`]: numbers and nothing else, written at one site, folded
    /// into the state hash.
    pub designators: DesignatorBook,
    /// **The designator a contact carried at the moment it expired** (16b cut
    /// Q.1), `(viewer, contact)` to what the row read as.
    ///
    /// A `lost` notice is staged *after* the row is gone, so the class that
    /// decides between `BA` and `X2` is no longer readable anywhere. It is
    /// captured in `Arena::sensor_pass` from the record
    /// [`ShadowContacts::expire`] hands back and taken out again by the
    /// `EventKind::Lost` arm of the staging scan — so it is filled and drained
    /// **inside one substep's event list** and never survives the substep it was
    /// filled in.
    ///
    /// It is deliberately **not** empty at every hash point: the substep takes its
    /// record before the staging scan runs, so a substep in which a contact
    /// expired is hashed with that row's name still in here. That costs nothing,
    /// because the table is not folded into [`crate::arena::hash::state_hash`] at
    /// all — its contents are a function of state the hash already folds (the
    /// expiry is the contact store's own change; the name is read off the book and
    /// the callsign), so folding it would add no information and would make the
    /// hash a function of where the drain sits.
    pub lost_designators: BTreeMap<(ObjId, ContactId), Designator>,
    /// **The shots of this substep that found a section already destroyed** (16b
    /// cut D.2, `B-15`), by shot sequence.
    ///
    /// Filled by [`crate::arena::damage::Arena::apply_hit`] at the one branch
    /// where the plan has no ops — a round that arrived on a section already at
    /// zero armour and zero internals — and drained by the `hit` arm of the
    /// staging scan in the **same substep**, which is what tells the three
    /// readers' sentences apart.
    ///
    /// **It is deliberately not folded into
    /// [`crate::arena::hash::state_hash`]**, on `lost_designators`' own argument,
    /// which really does apply here: its contents are a function of state the
    /// hash already folds (the section's cells, and the `hit` line the ledger
    /// sequence counts), and it never survives the substep it was filled in. It
    /// is **not** empty at every hash point — the substep takes its record before
    /// the staging scan runs — and that costs nothing for the same reason.
    pub spent_hits: BTreeSet<u64>,
    /// **When each (platform told, target, section) last read a spent-section
    /// line** (16b cut D.2, `B-15`), in arena milliseconds.
    ///
    /// `B-15`: one duel put six `armour gone, 0 of 0 inside.` lines in front of
    /// one pilot in ten seconds. The round stays absorbed and journalled exactly
    /// as it was; what this bounds is the **telling**, to one line per section per
    /// [`SPENT_NOTICE_MS`] of arena time, for the two readers who can tell a spent
    /// section from a live one: the victim, whose own tank is outside the observer
    /// rule, and the gunner who **holds the target classified**.
    ///
    /// **The unidentified gunner has no entry here and is never suppressed**, and
    /// that is an observer-rule decision rather than an oversight: a line
    /// withheld after a round he watched land would tell him the round hit
    /// something already dead, and a per-section key would let him count the
    /// target's destroyed sections by counting lines per five seconds. Both are
    /// condition facts the close look is the price of — the argument
    /// [`crate::arena::step::Frame::scan_tables`] already makes for the same class
    /// of fact.
    ///
    /// **It IS folded into [`crate::arena::hash::state_hash`]**, and that is where
    /// it parts from `spent_hits` beside it: it persists across substeps and
    /// pulses and it decides what a later substep says, so `lost_designators`'
    /// argument is false of it. Two rules keep the fold rate-independent — the
    /// decision and the write happen whether or not anybody is seated and whether
    /// or not a frame is ever sent, because the scan runs per substep at the
    /// event's own timestamp; and entries are pruned **only on touch**, relative
    /// to the event being staged, never by a per-pulse sweep.
    pub spent_said: BTreeMap<(ObjId, ObjId, SmolStr), i64>,
    /// The scheduled events.
    pub due: DueHeap,
    /// The streaming match ledger.
    pub ledger: Ledger,
    /// **The uniform grid over unit positions** (doc 16 §3.2 "The spatial
    /// index"), rebuilt twice per substep by [`Arena::step`].
    ///
    /// It lives on the arena rather than inside the pulse so that its buffer —
    /// one slot per cell, 65,536 of them on the shipped window — is allocated
    /// once for the match instead of once per substep, and so that cut S's sensor
    /// pass reads the same index motion tested occupancy against.
    pub index: SpatialIndex,
    /// The adaptive tempo and the lag clamp.
    pub clock: Clock,
    /// The match seed: the key of every draw.
    pub seed: u64,
    /// **The resolution table** (16b cut W.2), cloned in at check-out for the same
    /// reason the configuration blocks are: `step` needs no content argument and
    /// cannot be handed a different curve halfway through a match.
    pub resolution: ResolutionDef,
    /// The seated pilots, as handed over.
    pub pilots: Vec<Pilot>,
    /// **What the reflex controller remembers** (16b cut N.1): one row per AI
    /// unit — its state in the machine, its goal, the path it is following and
    /// when it last searched for one — plus the A* scratch buffers and the
    /// per-window path-query budget.
    ///
    /// It lives on the arena rather than on [`UnitState`] because the scratch is
    /// one buffer for the whole match and not one per unit, and because a unit
    /// under a pilot carries no machine at all.
    pub reflex: crate::arena::reflex::ReflexMemory,
    /// Queued discrete actions per unit, bounded by
    /// `intents.max_pending_per_unit`.
    pub pending: BTreeMap<ObjId, VecDeque<Intent>>,
    /// **The shots in flight**, by shot sequence (16b cut W.1; doc 16 §4 "Arena
    /// state": *scheduled impacts*).
    ///
    /// One row per accepted shot, from the moment `FIRE` is accepted until its
    /// [`DueKind::Impact`] is applied, which is what removes it. The
    /// [`DueKind::Impact`] event carries the sequence and nothing else, so this is
    /// where the shot's shooter, weapon, target, predicted point and round count
    /// live — everything doc 16 §3.5's re-validation at impact reads.
    pub shots: BTreeMap<u64, Shot>,
    /// The sequence the next accepted shot takes; the first shot of a match is `1`,
    /// so `0` is available as "no shot". It is the argument of the `hit:`, `facet:`
    /// and `burst:` draw labels (doc 16 §3.5), so it never repeats within a match
    /// and never restarts.
    pub next_shot_seq: u64,
    /// **The hash pulse phase 4's events are recorded against** (16b cut W.1), set
    /// by [`crate::arena::step::Arena::step`] for the length of the phase and `None`
    /// everywhere else.
    ///
    /// Doc 16 §4 asks every ledger line to carry the substep state hash, and
    /// [`crate::arena::step`]'s substep already takes **one** hash for all of its
    /// events — *the state at the substep's end, before any of that substep's events
    /// was appended* — because that is what makes the value a function of the state
    /// rather than of the order the events happened to be written in. Phase 4 is the
    /// same case read once more: it runs at one arena timestamp with no time
    /// advancing inside it, so every action it resolves happened at one moment and
    /// belongs to one hash.
    ///
    /// It is also the difference between O(1) and O(actions) hashes a pulse, and the
    /// hash is a walk of every unit and every contact: a pulse in which 150 units
    /// lock and fire measured 9.5 ms against 4.2 ms with this slot, all of it the
    /// hash (16b cut W.1's harness run). A caller outside the pulse —
    /// [`Arena::event_hash`]'s `None` arm — takes a fresh hash, so a direct
    /// `resolve_fire` in a test is unchanged.
    ///
    /// It is a **view** and is deliberately not folded into the state hash, for the
    /// same reason `held_losses` is not: it is scratch for one phase, and folding it
    /// would make the hash a function of where in the pulse it was taken.
    pub action_hash: Option<u64>,
    /// A `Stop` the world sent, honoured by phase 1 of the next step.
    pub stop: Option<StopReason>,
    /// Arena time: milliseconds since match start.
    pub now_ms: i64,
    /// **The last frame each pilot was sent** (doc 16 §3.1's *latest-wins slot per
    /// pilot*), so a pulse that changed nothing a pilot can see sends nothing.
    pub last_frames: FrameSlots,
    /// **The notices this pulse owes each platform's pilots** (16b cut S.2; doc 16
    /// §3.6: *notices fire once per transition*), keyed by the **platform** and not
    /// by the pilot, because doc 16 §3.7 puts the station on the tank and two
    /// pilots in one tank hear one scope.
    ///
    /// Staged as the pass journals its `detected`, `classified` and `lost` events —
    /// so a pilot's lines and the black box can never say different things — and
    /// cleared wholesale by [`crate::arena::step::Arena::frames`] at the end of the
    /// pulse, which bounds it at one pulse's transitions.
    pub pending_notices: BTreeMap<ObjId, Vec<FrameNotice>>,
    /// **Losses whose notice is being held** (16b cut S.2), `(platform, contact)`
    /// to the arena millisecond the notice becomes due.
    ///
    /// This is [`FrameNotice`]'s hysteresis, and the whole of it: a loss goes in
    /// here instead of into `pending_notices`, a re-acquisition inside the window
    /// takes it back out and says nothing, and a sweep at the pulse's end says what
    /// is left. It is a **view** and is deliberately not folded into the state hash
    /// — the sweep happens at pulse boundaries, so folding it would make the hash a
    /// function of the tempo, which is exactly what the cross-tempo property
    /// refuses.
    ///
    /// **16b cut Q.1** put the designator in the value beside the deadline: the
    /// row is already gone when the notice is finally said, so the name it read
    /// as has to be carried from the expiry that took it.
    pub held_losses: BTreeMap<(ObjId, ContactId), (i64, Designator)>,
    /// **The state hash of every substep of the pulse just run**, in order, and
    /// cleared at the start of each pulse (doc 16 §3.1 "Determinism": *each substep
    /// produces a state hash recorded with its pulse*).
    ///
    /// The ledger records a hash on each event, but a substep that journalled
    /// nothing would leave the sequence with a hole, and the property the hash
    /// exists for — one scripted log replayed at three tempos produces one hash
    /// sequence — is about every substep. This is where a test, the harness and cut
    /// T's watchdog read it.
    pub substep_hashes: Vec<u64>,
    /// Set once, when the match ends.
    pub ended: Option<MatchEnd>,
    /// The frozen `[tacsim.arena]` block.
    pub arena_cfg: ArenaConfig,
    /// The frozen `[tacsim.match]` block.
    pub match_cfg: MatchConfig,
    /// The frozen `[tacsim.sensors]` block (16b cut S.1).
    pub sensors_cfg: SensorsConfig,
    /// **The held-contact range that raises `Alert`**, centimetres, *resolved*
    /// (doc 16 §3.1's tempo table; 16b §9's `tempo.alert_range_cm` row, whose
    /// *2 × max weapon range* 16b cut E.1 capped — see below).
    ///
    /// A configured zero means derived, and [`Arena::new`] derives it once at
    /// check-out from the units it was handed: **the lesser of twice the longest
    /// weapon `max_range_cm` in the match ([`UnitState::weapon_reach_cm`]) and
    /// the window's own diagonal**
    /// (`isqrt(width² + height²) × cell_cm`). 16b cut E.1 added the cap, because
    /// twice a 3 km gun is 6 km on a window whose furthest two points are 3.6 km
    /// apart — a threshold no pair could be outside, which made the `Alert`
    /// trigger read "a contact is held at all" (16c `T-32`).
    ///
    /// It is resolved here rather than read from the configuration on every pulse
    /// so that a match's `Alert` threshold cannot change under it, exactly as the
    /// two configuration blocks are cloned in for the same reason.
    pub alert_range_cm: u32,
    /// **The widest jamming bubble in this match**, centimetres, *resolved* (16b cut
    /// X.4; doc 16 §9 item 4) — and **zero for every match in which nothing carries
    /// a pod**, which is every match the game ships.
    ///
    /// It is the one radius [`crate::arena::perception::Arena::jamming_at`] queries
    /// the spatial index at, and the zero is what lets that function return before
    /// touching the index at all: the ECM term costs a stock match one integer
    /// comparison per sensor pass and nothing else.
    ///
    /// Resolved at check-out from the units it was handed, exactly as
    /// `alert_range_cm` is, and for the same reason: a match's reach cannot change
    /// under it. A pod that is wrecked or shut down mid-match stops radiating — the
    /// scan reads both at scan time — but the *query radius* stays where check-out
    /// put it, which costs one index query of a bubble nobody is filling and is the
    /// honest price of not recomputing a maximum per pass.
    pub jam_radius_cm: u32,
    /// **The squads this match's reflex units are commanded in** (16b cut N.2;
    /// doc 16 §6 "Deliberation": *per squad or commander rather than per unit*),
    /// derived at check-out by [`crate::deliberation::squads_of`] — the same
    /// function, over the same `(unit, team, controller)` triples, that files them
    /// on the match record, so the two groupings cannot drift.
    ///
    /// Empty for a match with no AI at all: an arena with no squads reports no
    /// signals and accepts no order, which is exactly what doc 16 §1's two-tank
    /// story wants.
    pub squads: Vec<crate::deliberation::squad::SquadRoster>,
    /// **What the world was last told about each squad** (16b cut N.2), so the
    /// trigger can be *event-driven* rather than a poll.
    ///
    /// A **view**, like `held_losses` and `action_hash`: it is a fact about what
    /// has been reported and not about the match, so
    /// [`crate::arena::hash::state_hash`] does not fold it. Folding it would make
    /// the hash a function of the request cadence, which is the opposite of what
    /// the cross-tempo property asks for.
    pub squad_watch: crate::arena::command::SquadWatchTable,
    /// Squads whose standing order ran out since the last signal was taken (16b
    /// cut N.2) — the `order_expired` trigger, drained by
    /// [`Arena::take_deliberation_signals`]. A view, for the same reason.
    pub squads_expired: crate::arena::command::ExpiredSquads,
}

impl Arena {
    /// **Open a match** from the check-out message.
    ///
    /// In order: the units are sorted by id and checked for duplicates; each
    /// pose is checked to be a bearing on the window; each seated pilot's
    /// station raises the epoch table; the spatial index is built over the
    /// opening poses; the first `position_summary` is scheduled at
    /// `ledger.summary_ms`; and `match_started` is the ledger's first event,
    /// carrying the opening state hash.
    ///
    /// # Stations and epochs
    ///
    /// A station is an owner-local id that grants a *set* of channels
    /// (`mush_core::ChannelGrant`), so this constructor reads
    /// [`Channel::parse`]: a station literally named `helm`, `sensors` or `fire`
    /// raises that one channel, and any other station name raises all three —
    /// the cockpit seat of doc 16 §3.7. Raising never lowers, so cut T is free
    /// to publish narrower per-channel epochs with an explicit `Control`
    /// immediately afterwards.
    ///
    /// # Errors
    ///
    /// [`ArenaError`] naming the unit, the pilot or the terrain. Nothing is
    /// partially built: a refusal leaves no arena.
    pub fn new(
        custody: CustodyOut,
        arena_cfg: &ArenaConfig,
        match_cfg: &MatchConfig,
        sensors_cfg: &SensorsConfig,
    ) -> Result<Arena, ArenaError> {
        let CustodyOut {
            definition,
            units,
            pilots,
            resolution,
            seed,
        } = custody;
        if units.is_empty() {
            return Err(ArenaError::NoUnits);
        }
        let mut states: Vec<UnitState> = Vec::with_capacity(units.len());
        for mut unit in units {
            if unit.pose.heading_mdeg >= 360_000 {
                return Err(ArenaError::BadHeading {
                    unit: unit.id,
                    heading_mdeg: unit.pose.heading_mdeg,
                });
            }
            let Some(opening_cell) = definition.terrain.cell_of(&unit.pose) else {
                return Err(ArenaError::PoseOffWindow {
                    unit: unit.id,
                    x_cm: unit.pose.x_cm,
                    y_cm: unit.pose.y_cm,
                });
            };
            // **16b cut X.6** (doc 16 §3.2 "Terrain"): *a unit's `z_cm` equals the
            // cell's `elev_cm`*, and that is true from the opening hash and not
            // merely from the first crossing. [`CustodyOut`] is the only door into
            // an arena; the arena is the authority for its own grid, so it does not
            // trust an inbound z — a stale one, or the flat zero every world-side
            // placement writes, is corrected here rather than carried for however
            // many substeps it takes the tank to cross a boundary.
            if let Some(cell) = definition.terrain.get(opening_cell.0, opening_cell.1) {
                unit.pose.z_cm = i32::from(cell.elev_cm);
            }
            // **16b cut X.1**: the heat numbers, read off the cells the
            // `TacticalUnit` already carries rather than across a widened seam
            // (see [`HeatState`]). `None` for every unit whose content declares
            // no ceiling.
            let heat = HeatState::read(&unit.unit, &unit.profile, &unit.weapons);
            // **16b cut X.3**: the sight's thermal channel, read the same way and
            // `None` for every unit whose mount authors none.
            let thermal = ThermalSignature::read(&unit.unit);
            // **16b cut X.4**: the jamming pod, likewise — and the window's own
            // cell size applied here, once, so the pass compares centimetres.
            let jammer = Jammer::read(&unit.unit, definition.terrain.cell_cm());
            // **16b cut X.5**: the counter-jamming filter, likewise — and `None`
            // when its component has been shot off, which is the one thing that
            // stops a filter working.
            let eccm = Eccm::read(&unit.unit);
            // **16b cut X.6**: the steepest edge this unit's locomotion will
            // cross, read off the same record and `None` for a unit that declares
            // no locomotion the shipped table knows — every engine fixture, which
            // therefore still drives and still pays the surcharge.
            let slope_limit_pct = crate::arena::slope::SlopeLimit::read(&unit.unit);
            // **16b cut F.1** (doc 16 §3.1 "Custody"): the match opens on the unit
            // the world hands over, and that unit's own record may already say it
            // is a wreck, has lost a track or has had its turret shot off. Until
            // this cut every match opened at `MotionLimits::default()`, so a hulk
            // drove and shot and could never be killed again; this reads the
            // consequences off the record and puts them in force before pulse one.
            let entry = crate::arena::damage::entry_consequences(&unit.unit, &unit.profile);
            let weapons: Vec<WeaponSummary> = unit
                .weapons
                .into_iter()
                .filter(|weapon| entry.carries(&weapon.mount_section))
                .collect();
            let weapons_left = weapons.len();
            // **16b cut Q.2**: the first value of the band cache, off the record
            // the world handed over — so a tank that checks in with a shot-off
            // turret reads `destroyed` there from the opening frame, exactly as
            // cut F.1's entry consequences already put its lost track in force.
            let condition = crate::arena::condition::Condition::of(
                &unit.unit,
                sensors_cfg.condition_green_permille.min(1_000),
                sensors_cfg.condition_yellow_permille.min(1_000),
            );
            states.push(UnitState {
                id: unit.id,
                team: unit.team,
                // **16b cut Q.1**: the slot as it was handed over, carried here
                // unresolved — `resolve_callsigns` below turns a `0` into the
                // lowest free slot of the team once every unit is in hand and the
                // vector is in unit-id order.
                callsign: Designator::Slot {
                    team: unit.team,
                    slot: u16::from(unit.slot),
                },
                desired_heading_mdeg: unit.pose.heading_mdeg,
                pose: unit.pose,
                speed_cms: 0,
                desired_speed_cms: 0,
                power: Power::Off,
                mobility: unit.mobility,
                slope_limit_pct,
                // A unit opens the match standing still, so it entered its cell by
                // no edge at all: grade zero, which is the flat factor.
                grade_pct: 0,
                // **16b cut Q.6**: and it has spent no arena time moving, so there
                // is no fraction of a turn, a speed change or a centimetre to
                // carry into its first substep.
                carry: MotionCarry::default(),
                limits: entry.limits,
                sensor_height_cm: if unit.sensor_height_cm == 0 {
                    DEFAULT_SENSOR_HEIGHT_CM
                } else {
                    unit.sensor_height_cm
                },
                // A wreck sees nothing (`Arena::destroy_unit`'s own rule), so one
                // that arrives as a wreck sees nothing either.
                sensor: if entry.limits.wreck {
                    None
                } else {
                    unit.sensor
                },
                thermal,
                jammer,
                eccm,
                // **16b cut D.4**: a unit checks out with a clean receiver and
                // learns otherwise at its first pass. There is no notice for it:
                // the transition is what is said, and there has been none.
                receiver: None,
                // 16b cut W.1: the two live cells, one row per weapon, laid down
                // here so that every later lookup is a read of a complete table.
                // A weapon id declared twice on one unit keeps one row, which is
                // the same thing the component table itself is.
                ready_at_ms: weapons
                    .iter()
                    .map(|weapon| (weapon.id.clone(), 0_i64))
                    .collect(),
                // **16b cut F.1**: the rounds the unit's own `ammo` cell carries,
                // not the catalog's full load. A tank that checked in with 31
                // rounds checks out with 31. The authored load is the fallback for
                // a unit whose record declares no such cell at all — every fixture
                // arena in this crate, and any content whose weapon keeps its
                // ammunition somewhere else.
                ammo: weapons
                    .iter()
                    .map(|weapon| {
                        (
                            weapon.id.clone(),
                            rounds_on_record(&unit.unit, &weapon.id)
                                .unwrap_or(weapon.count_per_load),
                        )
                    })
                    .collect(),
                weapons,
                profile: unit.profile,
                // Doc 16 §3.5 step 2: *every stock rating is 1 through the
                // `PilotProfile` seam*, which is also the sheet's own floor, so a
                // check-out that named none resolves a green gunner rather than
                // dividing by zero.
                gunnery: if unit.gunnery == 0 {
                    crate::arena::resolution::STOCK_GUNNERY
                } else {
                    unit.gunnery
                },
                heat,
                controller: unit.controller,
                unit: unit.unit,
                condition,
                lock: None,
                blocked: false,
                held: false,
                linkdead: false,
                // **16b cut F.1**: doc 16 §3.8's *disabled — no weapons and no
                // motion* is a fact about the unit, so a unit that arrives that
                // way arrives disabled. A wreck is past disabled and is never
                // journalled both (`Arena::settle_disabled`'s own rule).
                disabled: !entry.limits.wreck && weapons_left == 0 && entry.limits.motion_disabled,
                shots_fired: 0,
                hits: 0,
            });
        }
        states.sort_by_key(|state| state.id);
        if let Some(duplicate) = states
            .windows(2)
            .find(|pair| pair.first().map(|a| a.id) == pair.get(1).map(|b| b.id))
            .and_then(|pair| pair.first())
            .map(|state| state.id)
        {
            return Err(ArenaError::DuplicateUnit { unit: duplicate });
        }
        // **16b cut Q.1**: every unit's public callsign, resolved once, here —
        // after the sort, so the fallback order is unit-id order and a replay of
        // one check-out resolves the same names.
        resolve_callsigns(&mut states)?;
        let mut epochs = EpochTable::new();
        for pilot in &pilots {
            if states
                .binary_search_by_key(&pilot.unit, |state| state.id)
                .is_err()
            {
                return Err(ArenaError::UnknownPilotUnit {
                    unit: pilot.unit,
                    pilot: pilot.pilot,
                    station: pilot.station.clone(),
                });
            }
            match Channel::parse(&pilot.station) {
                Some(channel) => {
                    epochs.raise(pilot.unit, channel, pilot.epoch);
                }
                None => {
                    for channel in Channel::ALL {
                        epochs.raise(pilot.unit, channel, pilot.epoch);
                    }
                }
            }
        }
        let mut due = DueHeap::new();
        due.schedule(
            i64::from(arena_cfg.ledger.summary_ms),
            DueKind::PositionSummary,
        );
        // Doc 16 §3.4 step 5: occupancy is one unit per cell. Motion is what keeps
        // it true once the match is running (step 4 refuses an occupied cell), and
        // this is what makes it true to begin with — the alternative is a cell whose
        // occupancy has two answers, which every later reader of the index would
        // have to carry a rule for. **16b cut A.2 added this check**; cut T's
        // check-out is its caller and places the eight tanks on eight cells.
        let mut cells: Vec<((u16, u16), ObjId)> = states
            .iter()
            .filter_map(|state| {
                definition
                    .terrain
                    .cell_of(&state.pose)
                    .map(|cell| (cell, state.id))
            })
            .collect();
        cells.sort_unstable();
        if let Some(((x, y), unit, other)) = cells
            .windows(2)
            .find(|pair| pair.first().map(|(at, _)| *at) == pair.get(1).map(|(at, _)| *at))
            .and_then(|pair| {
                let (cell, lower) = *pair.first()?;
                let (_, higher) = *pair.get(1)?;
                Some((cell, higher, lower))
            })
        {
            return Err(ArenaError::CellOccupied { unit, other, x, y });
        }
        let mut index = SpatialIndex::new(&definition.terrain);
        index.rebuild(&definition.terrain, &states);
        // 16b cut N.2: who is commanded, and by whom (see `squads` below).
        let squad_input: Vec<(ObjId, Team, bool)> = states
            .iter()
            .map(|state| {
                (
                    state.id,
                    state.team,
                    state.controller == crate::arena::state::Controller::Reflex,
                )
            })
            .collect();
        // 16b cut N.1: the cheapest cell on the window, read once here so the A*
        // heuristic is admissible without a scan per query
        // (`crate::arena::path::min_move_cost`).
        let min_move_cost = crate::arena::path::min_move_cost(&definition.terrain);
        // 16b §9's `tempo.alert_range_cm`: **the lesser of twice the longest
        // weapon range in the match and the window's own diagonal**, and the
        // configured `0` is what asks for that. Derived once, here, from the units
        // this match was handed; a match whose units carry no weapon derives `0`
        // and never raises `Alert` on a held contact, which is the honest answer.
        // The reach half is `UnitState::weapon_reach_cm`, which exists to be read
        // here and is read nowhere else in the crate.
        //
        // **16b cut E.1 (16c `T-32`) added the diagonal cap.**
        // Twice the stock cannon's 3 km is 6 km on a window 2.56 km on a side,
        // whose furthest two points are 3.6 km apart — so the threshold was a
        // number no pair of units could ever be outside, and `Alert`'s "a contact
        // is held inside `alert_range_cm`" meant "a contact is held". Capped, it
        // is honest on any window, and on the shipped one it is still wider than
        // anything a sight plus a mesh hop can hold (900 m), so nothing about a
        // shipped match's tempo moves.
        let alert_range_cm = if arena_cfg.tempo.alert_range_cm == 0 {
            let twice = u64::from(
                states
                    .iter()
                    .map(UnitState::weapon_reach_cm)
                    .max()
                    .unwrap_or(0),
            )
            .saturating_mul(2);
            let width = u64::from(definition.terrain.width());
            let height = u64::from(definition.terrain.height());
            let diagonal = width
                .saturating_mul(width)
                .saturating_add(height.saturating_mul(height))
                .isqrt()
                .saturating_mul(u64::from(definition.terrain.cell_cm()));
            u32::try_from(twice.min(diagonal)).unwrap_or(u32::MAX)
        } else {
            arena_cfg.tempo.alert_range_cm
        };
        // **16b cut X.4**: the widest pod bubble on the field, and `0` when there is
        // no pod — see `jam_radius_cm`.
        let jam_radius_cm = states
            .iter()
            .filter_map(|state| state.jammer.map(|jammer| jammer.radius_cm))
            .max()
            .unwrap_or(0);
        let mut arena = Arena {
            definition,
            units: states,
            epochs,
            contacts: ShadowContacts::new(),
            designators: DesignatorBook::default(),
            lost_designators: BTreeMap::new(),
            // 16b cut D.2: the spent-round scratch and the spent-round window.
            spent_hits: BTreeSet::new(),
            spent_said: BTreeMap::new(),
            due,
            ledger: Ledger::new(&arena_cfg.ledger),
            index,
            clock: Clock::new(),
            seed,
            resolution,
            pilots,
            reflex: crate::arena::reflex::ReflexMemory::new(&arena_cfg.reflex, min_move_cost),
            pending: BTreeMap::new(),
            shots: BTreeMap::new(),
            next_shot_seq: 0,
            action_hash: None,
            stop: None,
            now_ms: 0,
            last_frames: FrameSlots::new(),
            pending_notices: BTreeMap::new(),
            held_losses: BTreeMap::new(),
            substep_hashes: Vec::new(),
            ended: None,
            arena_cfg: arena_cfg.clone(),
            match_cfg: match_cfg.clone(),
            sensors_cfg: sensors_cfg.clone(),
            alert_range_cm,
            jam_radius_cm,
            // **16b cut N.2**: the squads are *derived* from the check-out, by the
            // same `squads_of` the match record files them with — a side's reflex
            // units in pod pairs, lowest id commanding. Derived rather than handed
            // over because two groupings of one match would be two answers to
            // "which squad is that order for", and because it means the harness and
            // every fixture get the same grouping the production lobby does without
            // a second argument to thread.
            squads: crate::deliberation::squad::squads_of(&squad_input),
            squad_watch: crate::arena::command::SquadWatchTable::new(),
            squads_expired: crate::arena::command::ExpiredSquads::new(),
        };
        // Doc 16 §3.6: one pass per unit per `sensors.pass_ms`, staggered by unit
        // id. Scheduled after the arena is built because the stagger reads the
        // sorted unit vector, and before the opening hash because the hash folds
        // the schedule's length.
        arena.schedule_first_passes();
        // 16b cut N.1: doc 16 §6's controller, on the same shape — one tick per AI
        // unit per `arena.reflex.decide_ms`, staggered by unit id.
        arena.schedule_first_reflex_ticks();
        let opening = crate::arena::hash::state_hash(&arena);
        // A ledger that refuses its own first event is a black box that will not
        // record anything; the match still opens, and cut A.2's step reads
        // `ledger.overflowed()` on its own terms.
        let _ = arena
            .ledger
            .append(0, EventKind::MatchStarted, None, opening);
        Ok(arena)
    }

    /// One unit by id, by binary search over the sorted vector.
    #[must_use]
    pub fn unit(&self, id: ObjId) -> Option<&UnitState> {
        let index = self
            .units
            .binary_search_by_key(&id, |state| state.id)
            .ok()?;
        self.units.get(index)
    }

    /// The same, mutably.
    pub fn unit_mut(&mut self, id: ObjId) -> Option<&mut UnitState> {
        let index = self
            .units
            .binary_search_by_key(&id, |state| state.id)
            .ok()?;
        self.units.get_mut(index)
    }

    /// **The one assignment of [`UnitState::unit`] in non-test code** (16b cut
    /// Q.2; doc 16 §3.5 "Damage", D-16-6): take the unit `mush_tactics::apply`
    /// handed back, and recompute the band cache beside it.
    ///
    /// Cut W.2's rule was already that a unit's cells move only through one
    /// [`mush_tactics::UnitMutationPlan`] per change — but the *assignment* had
    /// two production sites, the impact in [`crate::arena::damage`] and the heat
    /// plan in [`crate::arena::heat`], and cut Q.2 added a cache
    /// ([`UnitState::condition`]) that both of them would have had to remember.
    /// A cache with two writers is a cache that rots, and a rotted band is a
    /// `SCAN` that tells a gunner a spent turret is green. So there is one
    /// function, both call it, and a third site added later is a diff somebody
    /// reads rather than a silent divergence.
    ///
    /// A unit this arena does not hold is a no-op: the caller has already
    /// resolved it, and `apply` produced a unit for an id that has left the match
    /// only if the match let go of it inside the same substep.
    pub(crate) fn set_unit(&mut self, id: ObjId, post: TacticalUnit) {
        // Read the floors before the borrow, and read them `.min(1000)` the way
        // every consumer of these keys does (their validation is Q.7's).
        let condition = crate::arena::condition::Condition::of(
            &post,
            self.sensors_cfg.condition_green_permille.min(1_000),
            self.sensors_cfg.condition_yellow_permille.min(1_000),
        );
        if let Some(state) = self.unit_mut(id) {
            state.unit = post;
            state.condition = condition;
        }
    }

    /// **Take one pilot off the frame roster** (16b cut F.1; doc 16 §3.6).
    ///
    /// The roster is what [`Arena::frames`] walks, so a pilot dropped from it is
    /// built no further frame and told no further notice. Keyed on the **pair**:
    /// a pilot who left `unit` is not a co-pilot still sitting in it, and a tank
    /// with two seats loses only the seat that went.
    ///
    /// The remembered frame goes with the row, so a pilot the world later seats
    /// again is sent a whole frame rather than compared against one from a life
    /// they no longer have. Nothing else in the arena reads the roster — it is not
    /// folded into [`crate::arena::hash::state_hash`], which is what keeps this
    /// invisible to the cross-tempo property — so this is the whole of the drop.
    ///
    /// Idempotent. Answers whether a row went.
    pub fn drop_pilot(&mut self, pilot: ObjId, unit: ObjId) -> bool {
        let before = self.pilots.len();
        self.pilots
            .retain(|seat| !(seat.pilot == pilot && seat.unit == unit));
        let dropped = self.pilots.len() != before;
        if dropped && !self.pilots.iter().any(|seat| seat.pilot == pilot) {
            self.last_frames.remove(&pilot);
        }
        dropped
    }

    /// **The hash one discrete action's ledger line carries** (16b cut W.1):
    /// [`Arena::action_hash`] while the pulse is inside phase 4, and a fresh
    /// [`crate::arena::hash::state_hash`] outside it.
    ///
    /// Every resolver of a discrete action goes through this rather than hashing for
    /// itself, so the phase's lines share one hash exactly as a substep's do, and a
    /// caller outside the pulse still gets an honest one.
    #[must_use]
    pub fn event_hash(&self) -> u64 {
        match self.action_hash {
            Some(hash) => hash,
            None => crate::arena::hash::state_hash(self),
        }
    }

    /// Ask the arena to stop. **Phase 1 of the next step honours it**; nothing
    /// here ends the match, so a `Stop` never lands in the middle of a substep.
    ///
    /// The first reason wins: a `Void` that follows an `Abort` does not rewrite
    /// history, and an `Abort` that follows a `Void` cannot make a voided match
    /// committable.
    pub fn stop(&mut self, reason: StopReason) {
        if self.stop.is_none() {
            self.stop = Some(reason);
        }
    }

    /// The fastest unit in the match, cm/s — the divisor of the substep length
    /// (doc 16 §3.1 "Rate independence"). Zero when nothing can move.
    ///
    /// A wreck and a unit whose motion is disabled are excluded: a substep sized
    /// for a speed no unit can reach costs every pulse a needless subdivision.
    #[must_use]
    pub fn max_speed_cms(&self) -> u32 {
        self.units
            .iter()
            .filter(|state| !state.limits.wreck && !state.limits.motion_disabled)
            .map(|state| state.mobility.max_speed_cms)
            .max()
            .unwrap_or(0)
    }

    /// The side of one cell, centimetres.
    #[must_use]
    pub fn cell_cm(&self) -> u16 {
        self.definition.terrain.cell_cm()
    }

    /// A `position_summary` payload over every unit, in unit-id order (doc 16
    /// §4): `(unit, x_cm, y_cm, z_cm, heading_mdeg)`.
    #[must_use]
    pub fn position_summary(&self) -> Vec<(ObjId, i64, i64, i32, u32)> {
        self.units
            .iter()
            .map(|state| {
                (
                    state.id,
                    state.pose.x_cm,
                    state.pose.y_cm,
                    state.pose.z_cm,
                    state.pose.heading_mdeg,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::test_support::{arena_of, custody, ground, pose, tank};

    /// **16b cut Q.1**: the letters are the whole of what keeps a provisional
    /// designator and a slot designator apart, so they have to be pairwise
    /// distinct and none of them may be [`PROVISIONAL_PREFIX`]. A third side is a
    /// failing test here with the fix beside it.
    #[test]
    fn every_team_letter_is_distinct_and_none_of_them_is_the_provisional_prefix() {
        let letters: Vec<char> = Team::ALL.iter().map(|team| team.letter()).collect();
        for (at, letter) in letters.iter().enumerate() {
            assert_ne!(
                *letter,
                PROVISIONAL_PREFIX,
                "{:?} spells its callsigns with the provisional prefix",
                Team::ALL.get(at)
            );
            assert!(
                letters.iter().skip(at + 1).all(|other| other != letter),
                "two sides share the letter {letter}"
            );
        }
    }

    /// The spelling of a designator, at each of the three shapes that matter: the
    /// provisional number, a slot inside the alphabet, and the first slot past it.
    #[test]
    fn a_designator_spells_a_number_a_letter_and_the_roll_past_z() {
        assert_eq!(Designator::Provisional(1).to_string(), "X1");
        assert_eq!(Designator::Provisional(12).to_string(), "X12");
        for (slot, spelling) in [
            (1_u16, "RA"),
            (2, "RB"),
            (12, "RL"),
            (26, "RZ"),
            (27, "RAA"),
            (28, "RAB"),
            (52, "RAZ"),
            (53, "RBA"),
        ] {
            assert_eq!(
                Designator::Slot {
                    team: Team::Red,
                    slot,
                }
                .to_string(),
                spelling
            );
        }
        assert_eq!(
            Designator::Slot {
                team: Team::Blue,
                slot: 3,
            }
            .to_string(),
            "BC"
        );
        // A slot of zero is a name nothing resolves, not a panic: `Arena::new`
        // never produces one.
        assert_eq!(
            Designator::Slot {
                team: Team::Blue,
                slot: 0,
            }
            .to_string(),
            "B"
        );
    }

    /// **16b cut Q.1**: a number is the viewer's own, issued in first-seen order,
    /// never reused, and kept for the target it was issued for.
    #[test]
    fn the_book_numbers_per_viewer_in_first_seen_order_and_never_reuses_one() {
        let mut book = DesignatorBook::default();
        assert_eq!(book.provisional(ObjId(1), ObjId(9)), None);
        assert_eq!(book.assign(ObjId(1), ObjId(9)), 1);
        assert_eq!(book.assign(ObjId(1), ObjId(4)), 2);
        // The same pair answers the same number for ever.
        assert_eq!(book.assign(ObjId(1), ObjId(9)), 1);
        assert_eq!(book.provisional(ObjId(1), ObjId(9)), Some(1));
        // A second viewer counts from one of its own.
        assert_eq!(book.assign(ObjId(2), ObjId(4)), 1);
        assert_eq!(book.provisional(ObjId(2), ObjId(9)), None);
        // The walks the hash folds, in map order.
        assert_eq!(
            book.counters().collect::<Vec<(ObjId, u16)>>(),
            vec![(ObjId(1), 3), (ObjId(2), 2)]
        );
        assert_eq!(
            book.issued().collect::<Vec<(ObjId, ObjId, u16)>>(),
            vec![
                (ObjId(1), ObjId(4), 2),
                (ObjId(1), ObjId(9), 1),
                (ObjId(2), ObjId(4), 1),
            ]
        );
    }

    #[test]
    fn terrain_refuses_a_grid_that_is_not_its_dimensions() {
        let short = ArenaTerrain::new(4, 4, 1_000, vec![ground(); 15]);
        assert!(matches!(
            short,
            Err(ArenaError::TerrainShape { cells: 15, .. })
        ));
        let zero_cell = ArenaTerrain::uniform(4, 4, 0, ground());
        assert!(matches!(
            zero_cell,
            Err(ArenaError::TerrainShape { cell_cm: 0, .. })
        ));
        let empty = ArenaTerrain::uniform(0, 4, 1_000, ground());
        assert!(matches!(
            empty,
            Err(ArenaError::TerrainShape { width: 0, .. })
        ));
        let ok = ArenaTerrain::uniform(4, 4, 1_000, ground()).expect("a 4x4 window");
        assert_eq!(ok.cells().len(), 16);
    }

    #[test]
    fn cell_of_floors_and_answers_none_off_the_window() {
        let terrain = ArenaTerrain::uniform(4, 3, 1_000, ground()).expect("a 4x3 window");
        // The origin, and the last centimetre of the first cell.
        assert_eq!(terrain.cell_of(&pose(0, 0)), Some((0, 0)));
        assert_eq!(terrain.cell_of(&pose(999, 999)), Some((0, 0)));
        // The boundary belongs to the cell it opens.
        assert_eq!(terrain.cell_of(&pose(1_000, 0)), Some((1, 0)));
        assert_eq!(terrain.cell_of(&pose(0, 1_000)), Some((0, 1)));
        // The far corner, and one centimetre past it on each axis.
        assert_eq!(terrain.cell_of(&pose(3_999, 2_999)), Some((3, 2)));
        assert_eq!(terrain.cell_of(&pose(4_000, 2_999)), None);
        assert_eq!(terrain.cell_of(&pose(3_999, 3_000)), None);
        // Negative is off the window, never folded toward zero.
        assert_eq!(terrain.cell_of(&pose(-1, 0)), None);
        assert_eq!(terrain.cell_of(&pose(0, -1)), None);
        assert_eq!(terrain.cell_of(&pose(-1_000, -1_000)), None);
        // And `get` agrees with the window's bounds.
        assert!(terrain.get(3, 2).is_some());
        assert!(terrain.get(4, 2).is_none());
        assert!(terrain.get(3, 3).is_none());
    }

    #[test]
    fn the_epoch_table_never_lowers_and_authorises_only_the_current_epoch() {
        let unit = ObjId(7);
        let mut epochs = EpochTable::new();
        assert_eq!(epochs.current(unit, Channel::Helm), 0);
        assert!(epochs.authorises(unit, Channel::Helm, 0));
        assert!(epochs.raise(unit, Channel::Helm, 3));
        assert_eq!(epochs.current(unit, Channel::Helm), 3);
        // A lower or equal epoch is a no-op.
        assert!(!epochs.raise(unit, Channel::Helm, 2));
        assert!(!epochs.raise(unit, Channel::Helm, 3));
        assert_eq!(epochs.current(unit, Channel::Helm), 3);
        // Only the exact current epoch authorises: stale and future both refuse.
        assert!(epochs.authorises(unit, Channel::Helm, 3));
        assert!(!epochs.authorises(unit, Channel::Helm, 2));
        assert!(!epochs.authorises(unit, Channel::Helm, 4));
        // Channels are independent, and so are units.
        assert_eq!(epochs.current(unit, Channel::Fire), 0);
        assert_eq!(epochs.current(ObjId(8), Channel::Helm), 0);
    }

    #[test]
    fn the_due_heap_is_a_min_heap_by_timestamp_then_sequence() {
        let mut due = DueHeap::new();
        let late = due.schedule(500, DueKind::PositionSummary);
        let early = due.schedule(100, DueKind::StartupComplete { unit: ObjId(1) });
        let tie = due.schedule(100, DueKind::StartupComplete { unit: ObjId(2) });
        assert_eq!((late, early, tie), (1, 2, 3));
        assert_eq!(due.len(), 3);
        assert_eq!(due.next_t_ms(), Some(100));
        // Nothing before its time.
        assert!(due.peek_due(99).is_none());
        assert!(due.pop_due(99).is_none());
        // Timestamp order first, then insertion sequence.
        assert_eq!(due.pop_due(100).map(|event| event.seq), Some(early));
        assert_eq!(due.pop_due(100).map(|event| event.seq), Some(tie));
        assert!(due.pop_due(100).is_none());
        assert_eq!(due.pop_due(i64::MAX).map(|event| event.seq), Some(late));
        assert!(due.is_empty());
    }

    #[test]
    fn opening_a_match_sorts_units_schedules_the_summary_and_journals_the_start() {
        let arena = arena_of(vec![
            tank(ObjId(9), Team::Blue, pose(2_500, 2_500)),
            tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
        ]);
        assert_eq!(
            arena.units.iter().map(|state| state.id).collect::<Vec<_>>(),
            vec![ObjId(4), ObjId(9)]
        );
        assert_eq!(arena.now_ms, 0);
        assert!(arena.ended.is_none());
        assert!(arena.stop.is_none());
        // Every unit starts cold, stationary, and ordered to hold its heading.
        for state in &arena.units {
            assert_eq!(state.power, Power::Off);
            assert_eq!(state.speed_cms, 0);
            assert_eq!(state.desired_speed_cms, 0);
            assert_eq!(state.desired_heading_mdeg, state.pose.heading_mdeg);
            assert_eq!(state.limits, MotionLimits::default());
            assert!(!state.linkdead);
            assert!(state.ready_at_ms.is_empty());
            assert!(state.lock.is_none());
        }
        // Three scheduled events: the first position summary at
        // `ledger.summary_ms`, and — **16b cut F.3** — one sensor pass per unit,
        // staggered at `id % pass_ms`. The pass is where a platform's contacts
        // expire, so every platform has one whether or not it carries a sight.
        assert_eq!(arena.due.len(), 3);
        assert_eq!(arena.due.next_t_ms(), Some(4), "unit #4's own stagger");
        // And one ledger event: `match_started`, sequence 1.
        assert_eq!(arena.ledger.seq(), 1);
        assert!(arena.ledger.bytes_total() > 0);
    }

    #[test]
    fn opening_a_match_refuses_a_pose_off_the_window_a_duplicate_and_a_stray_pilot() {
        let off = Arena::new(
            custody(
                vec![tank(ObjId(4), Team::Red, pose(99_000, 1_500))],
                Vec::new(),
            ),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        );
        assert!(matches!(
            off,
            Err(ArenaError::PoseOffWindow { unit: ObjId(4), .. })
        ));

        let twice = Arena::new(
            custody(
                vec![
                    tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
                    tank(ObjId(4), Team::Blue, pose(2_500, 2_500)),
                ],
                Vec::new(),
            ),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        );
        assert!(matches!(
            twice,
            Err(ArenaError::DuplicateUnit { unit: ObjId(4) })
        ));

        let stray = Arena::new(
            custody(
                vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))],
                vec![Pilot {
                    unit: ObjId(5),
                    pilot: ObjId(100),
                    station: SmolStr::new("cockpit"),
                    epoch: 1,
                }],
            ),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        );
        assert!(matches!(
            stray,
            Err(ArenaError::UnknownPilotUnit { unit: ObjId(5), .. })
        ));

        let nothing = Arena::new(
            custody(Vec::new(), Vec::new()),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        );
        assert!(matches!(nothing, Err(ArenaError::NoUnits)));
    }

    #[test]
    fn opening_a_match_refuses_two_units_in_one_cell() {
        // Doc 16 §3.4 step 5: occupancy is one unit per cell. Two poses in cell
        // (2, 2) — different centimetres, one cell — and the refusal names the
        // cell and the unit already in it (the lower id, because it is the one a
        // check-out would have placed first).
        let shared = Arena::new(
            custody(
                vec![
                    tank(ObjId(9), Team::Blue, pose(2_500, 2_500)),
                    tank(ObjId(4), Team::Red, pose(2_100, 2_900)),
                ],
                Vec::new(),
            ),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        );
        assert!(
            matches!(
                shared,
                Err(ArenaError::CellOccupied {
                    unit: ObjId(9),
                    other: ObjId(4),
                    x: 2,
                    y: 2
                })
            ),
            "{shared:?}"
        );
        // Adjacent cells are fine, and so is the cell boundary: 2,999 is column 2
        // and 3,000 is column 3.
        let apart = Arena::new(
            custody(
                vec![
                    tank(ObjId(4), Team::Red, pose(2_999, 2_500)),
                    tank(ObjId(9), Team::Blue, pose(3_000, 2_500)),
                ],
                Vec::new(),
            ),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        );
        assert!(apart.is_ok(), "{apart:?}");
    }

    #[test]
    fn a_seat_raises_its_channels_and_a_named_station_raises_only_its_own() {
        let arena = Arena::new(
            custody(
                vec![
                    tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
                    tank(ObjId(5), Team::Blue, pose(2_500, 2_500)),
                ],
                vec![
                    Pilot {
                        unit: ObjId(4),
                        pilot: ObjId(100),
                        station: SmolStr::new("cockpit"),
                        epoch: 4,
                    },
                    Pilot {
                        unit: ObjId(5),
                        pilot: ObjId(101),
                        station: SmolStr::new("fire"),
                        epoch: 7,
                    },
                ],
            ),
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .expect("two tanks and two seats");
        for channel in Channel::ALL {
            assert_eq!(arena.epochs.current(ObjId(4), channel), 4);
        }
        assert_eq!(arena.epochs.current(ObjId(5), Channel::Fire), 7);
        assert_eq!(arena.epochs.current(ObjId(5), Channel::Helm), 0);
        assert_eq!(arena.epochs.current(ObjId(5), Channel::Sensors), 0);
    }

    #[test]
    fn unit_lookup_is_by_id_and_stop_keeps_the_first_reason() {
        let mut arena = arena_of(vec![
            tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
            tank(ObjId(9), Team::Blue, pose(2_500, 2_500)),
        ]);
        assert_eq!(
            arena.unit(ObjId(9)).map(|state| state.team),
            Some(Team::Blue)
        );
        assert!(arena.unit(ObjId(5)).is_none());
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.speed_cms = 42;
        }
        assert_eq!(arena.unit(ObjId(4)).map(|state| state.speed_cms), Some(42));
        arena.stop(StopReason::Abort);
        arena.stop(StopReason::Void);
        assert_eq!(arena.stop, Some(StopReason::Abort));
    }

    #[test]
    fn the_definition_digest_is_stable_and_separates_every_field() {
        let base = ArenaDefinition {
            id: SmolStr::new("tacsim.arena.proving_ground"),
            terrain: ArenaTerrain::uniform(4, 4, 1_000, ground()).expect("a 4x4 window"),
            entry_zones: vec![EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: 1,
            }],
            key_spots: vec![KeySpot {
                name: SmolStr::new("The Ford"),
                x: 1,
                y: 2,
            }],
            waypoints: Waypoints::new(
                vec![
                    Waypoint {
                        id: SmolStr::new("west"),
                        x: 0,
                        y: 2,
                    },
                    Waypoint {
                        id: SmolStr::new("east"),
                        x: 3,
                        y: 2,
                    },
                ],
                &[(SmolStr::new("west"), SmolStr::new("east"))],
            ),
        };
        assert_eq!(base.digest(), base.clone().digest());

        let mut renamed = base.clone();
        renamed.id = SmolStr::new("tacsim.arena.proving_grounds");
        assert_ne!(base.digest(), renamed.digest());

        let mut repainted = base.clone();
        let mut cells = repainted.terrain.cells().to_vec();
        if let Some(cell) = cells.get_mut(5) {
            cell.move_cost = 0;
        }
        repainted.terrain =
            ArenaTerrain::new(4, 4, 1_000, cells).expect("the same window, one cell impassable");
        assert_ne!(base.digest(), repainted.digest());

        let mut moved_zone = base.clone();
        moved_zone.entry_zones = vec![EntryZone {
            team: Team::Blue,
            x0: 0,
            y0: 0,
            x1: 1,
            y1: 1,
        }];
        assert_ne!(base.digest(), moved_zone.digest());

        let mut resized = base.clone();
        resized.terrain = ArenaTerrain::uniform(2, 8, 1_000, ground()).expect("a 2x8 window");
        assert_ne!(base.digest(), resized.digest());

        // 16b cut N.1: the furniture is part of the definition's identity — two
        // windows with one terrain and two graphs are two arenas, because the
        // routes a replay reproduces are a function of the graph.
        let mut renamed_spot = base.clone();
        renamed_spot.key_spots = vec![KeySpot {
            name: SmolStr::new("The Crossing"),
            x: 1,
            y: 2,
        }];
        assert_ne!(base.digest(), renamed_spot.digest());

        let mut moved_spot = base.clone();
        moved_spot.key_spots = vec![KeySpot {
            name: SmolStr::new("The Ford"),
            x: 2,
            y: 2,
        }];
        assert_ne!(base.digest(), moved_spot.digest());

        let mut cut_edge = base.clone();
        cut_edge.waypoints = Waypoints::new(
            vec![
                Waypoint {
                    id: SmolStr::new("west"),
                    x: 0,
                    y: 2,
                },
                Waypoint {
                    id: SmolStr::new("east"),
                    x: 3,
                    y: 2,
                },
            ],
            &[],
        );
        assert_ne!(
            base.digest(),
            cut_edge.digest(),
            "the same nodes with no edge between them is a different graph"
        );
    }

    #[test]
    fn the_waypoint_graph_resolves_its_edges_and_drops_the_unresolvable() {
        let nodes = vec![
            Waypoint {
                id: SmolStr::new("a"),
                x: 0,
                y: 0,
            },
            Waypoint {
                id: SmolStr::new("b"),
                x: 4,
                y: 0,
            },
            Waypoint {
                id: SmolStr::new("c"),
                x: 4,
                y: 4,
            },
        ];
        let graph = Waypoints::new(
            nodes,
            &[
                (SmolStr::new("a"), SmolStr::new("b")),
                (SmolStr::new("b"), SmolStr::new("c")),
                // Declared twice, one of them backwards: one edge.
                (SmolStr::new("c"), SmolStr::new("b")),
                // A node nothing declares, and a self-edge: both dropped.
                (SmolStr::new("b"), SmolStr::new("ghost")),
                (SmolStr::new("a"), SmolStr::new("a")),
            ],
        );
        assert_eq!(graph.nodes().len(), 3);
        assert!(!graph.is_empty());
        assert_eq!(graph.neighbours(0), &[1]);
        assert_eq!(graph.neighbours(1), &[0, 2]);
        assert_eq!(graph.neighbours(2), &[1]);
        assert!(graph.neighbours(9).is_empty());
        assert_eq!(graph.node(2).map(|node| (node.x, node.y)), Some((4, 4)));
        // The nearest node, ties to the lower index.
        assert_eq!(graph.nearest(0, 1), Some(0));
        assert_eq!(graph.nearest(4, 3), Some(2));
        assert_eq!(graph.nearest(2, 0), Some(0));
        assert!(Waypoints::default().nearest(0, 0).is_none());
        assert!(Waypoints::default().is_empty());
    }

    /// A plot at `(x_cm, y_cm)`, facing north and standing still (16b cut F.3).
    fn plot(x_cm: i64, y_cm: i64) -> SeenPose {
        SeenPose {
            x_cm,
            y_cm,
            z_cm: 0,
            heading_mdeg: 0,
            speed_cms: 0,
        }
    }

    #[test]
    fn the_shadow_contact_store_holds_refreshes_and_expires() {
        let viewer = ObjId(4);
        let mut contacts = ShadowContacts::new();
        assert!(contacts.is_empty());
        let first = ContactId(0xdead_beef);
        let second = ContactId(0x0bad_f00d);
        contacts.insert(
            viewer,
            first,
            ShadowContact::sighted(ObjId(9), Classification::Unknown, 1_000, plot(0, 0)),
        );
        contacts.insert(
            viewer,
            second,
            ShadowContact {
                last_seen_ms: 4_000,
                ..ShadowContact::sighted(
                    ObjId(10),
                    Classification::Classified { team: Team::Blue },
                    1_000,
                    plot(0, 0),
                )
            },
        );
        assert_eq!(contacts.len(), 2);
        assert_eq!(contacts.viewers(), 1);
        assert_eq!(
            contacts.get(viewer, first).map(|held| held.target),
            Some(ObjId(9))
        );
        assert_eq!(contacts.id_of(viewer, ObjId(10)), Some(second));
        assert!(contacts.id_of(viewer, ObjId(11)).is_none());
        // Another viewer's id is nobody else's.
        assert!(contacts.get(ObjId(5), first).is_none());
        if let Some(held) = contacts.get_mut(viewer, first) {
            held.last_seen_ms = 2_000;
        }
        // Expiry answers the losses and keeps the refreshed one.
        let lost = contacts.expire(viewer, 3_000);
        assert_eq!(lost.len(), 1);
        assert_eq!(lost.first().map(|(id, _)| *id), Some(first));
        assert_eq!(contacts.len(), 1);
        contacts.clear_viewer(viewer);
        assert!(contacts.is_empty());
        assert_eq!(contacts.viewers(), 0);
    }

    #[test]
    fn channels_round_trip_their_words() {
        for channel in Channel::ALL {
            assert_eq!(Channel::parse(channel.as_str()), Some(channel));
        }
        assert!(Channel::parse("helms").is_none());
        assert!(Channel::parse("HELM").is_none());
        assert_eq!(Team::Red.other(), Team::Blue);
        assert_eq!(Side::Left.as_str(), "left");
        assert_eq!(Power::Online.code(), 2);
        assert_eq!(ContactId(0x1234).to_string(), "0000000000001234");
    }

    #[test]
    fn the_reverse_ceiling_is_a_percentage_of_the_forward_one() {
        let rules = TrackedRules {
            max_speed_cms: 1_000,
            accel_cms2: 200,
            brake_cms2: 400,
            turn_rate_mdeg_s: 30_000,
            reverse_factor_pct: 40,
        };
        assert_eq!(rules.reverse_ceiling_cms(), 400);
        assert_eq!(TrackedRules::default().reverse_ceiling_cms(), 0);
    }

    #[test]
    fn the_fastest_unit_ignores_wrecks_and_disabled_motion() {
        let mut arena = arena_of(vec![
            tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
            tank(ObjId(9), Team::Blue, pose(2_500, 2_500)),
        ]);
        assert_eq!(arena.max_speed_cms(), 1_000);
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.limits.wreck = true;
        }
        if let Some(state) = arena.unit_mut(ObjId(9)) {
            state.mobility.max_speed_cms = 300;
        }
        assert_eq!(arena.max_speed_cms(), 300);
        if let Some(state) = arena.unit_mut(ObjId(9)) {
            state.limits.motion_disabled = true;
        }
        assert_eq!(arena.max_speed_cms(), 0);
    }

    #[test]
    fn the_position_summary_is_every_unit_in_id_order() {
        let arena = arena_of(vec![
            tank(ObjId(9), Team::Blue, pose(2_500, 2_600)),
            tank(ObjId(4), Team::Red, pose(1_500, 1_600)),
        ]);
        assert_eq!(
            arena.position_summary(),
            vec![
                (ObjId(4), 1_500, 1_600, 0, 0),
                (ObjId(9), 2_500, 2_600, 0, 0)
            ]
        );
    }
}
