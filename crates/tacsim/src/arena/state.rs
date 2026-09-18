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
use crate::arena::perception::{FrameNotice, SensorProfile};
use crate::arena::step::FrameSlots;
use crate::arena::weapons::{Shot, WeaponBand};
use crate::config::{ArenaConfig, MatchConfig, SensorsConfig};

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
        if pose.x_cm < 0 || pose.y_cm < 0 {
            return None;
        }
        let cell = i64::from(self.cell_cm);
        let x = u16::try_from(pose.x_cm / cell).ok()?;
        let y = u16::try_from(pose.y_cm / cell).ok()?;
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

/// The arena a match runs on: its id, its window and its entry zones (doc 16
/// §4's `arenas/proving_ground.json`).
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
    ///    byte (`0` red, `1` blue) then `x0`, `y0`, `x1`, `y1` as `u16`.
    ///
    /// Every variable-length run is length-prefixed, so no two different
    /// definitions share an encoding.
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
    /// **How many rounds one load holds** (16b cut W.1), off the ammunition
    /// definition. It is what [`Arena::new`] sizes the unit's `ammo` cell from and
    /// what `WEAPONSTATUS` prints as the denominator.
    pub count_per_load: u32,
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
    /// What damage has taken from its motion.
    pub limits: MotionLimits,
    /// How high its eye sits, centimetres (doc 16 §3.2 "Line of sight").
    pub sensor_height_cm: u32,
    /// What it sees with (doc 16 §3.6); `None` runs no pass.
    pub sensor: Option<SensorProfile>,
    /// What it shoots with — the immutable definition rows, in component order.
    pub weapons: Vec<WeaponSummary>,
    /// M32 unit truth, carried opaque.
    pub unit: TacticalUnit,
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
    /// The seat has gone linkdead (doc 07 §1.2): the tank keeps its continuous
    /// orders and accepts no discrete action.
    pub linkdead: bool,
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

/// One contact a viewer platform holds (doc 16 §3.6's contact lifecycle as
/// vocabulary, in RAM).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowContact {
    /// What is being held.
    pub target: ObjId,
    /// How well.
    pub class: Classification,
    /// Arena milliseconds at first detection.
    pub first_seen_ms: i64,
    /// Arena milliseconds at the last refresh — the input to expiry.
    pub last_seen_ms: i64,
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
    /// The seated pilots, as handed over.
    pub pilots: Vec<Pilot>,
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
    pub held_losses: BTreeMap<(ObjId, ContactId), i64>,
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
    /// (doc 16 §3.1's tempo table; 16b §9: `tempo.alert_range_cm` is *2 × max
    /// weapon range*).
    ///
    /// A configured zero means derived, and [`Arena::new`] derives it once at
    /// check-out from the units it was handed — twice the longest weapon
    /// `max_range_cm` of any unit in the match. It is resolved here rather than
    /// read from the configuration on every pulse so that a match's `Alert`
    /// threshold cannot change under it, exactly as the two configuration blocks
    /// are cloned in for the same reason.
    pub alert_range_cm: u32,
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
            seed,
        } = custody;
        if units.is_empty() {
            return Err(ArenaError::NoUnits);
        }
        let mut states: Vec<UnitState> = Vec::with_capacity(units.len());
        for unit in units {
            if unit.pose.heading_mdeg >= 360_000 {
                return Err(ArenaError::BadHeading {
                    unit: unit.id,
                    heading_mdeg: unit.pose.heading_mdeg,
                });
            }
            if definition.terrain.cell_of(&unit.pose).is_none() {
                return Err(ArenaError::PoseOffWindow {
                    unit: unit.id,
                    x_cm: unit.pose.x_cm,
                    y_cm: unit.pose.y_cm,
                });
            }
            states.push(UnitState {
                id: unit.id,
                team: unit.team,
                desired_heading_mdeg: unit.pose.heading_mdeg,
                pose: unit.pose,
                speed_cms: 0,
                desired_speed_cms: 0,
                power: Power::Off,
                mobility: unit.mobility,
                limits: MotionLimits::default(),
                sensor_height_cm: if unit.sensor_height_cm == 0 {
                    DEFAULT_SENSOR_HEIGHT_CM
                } else {
                    unit.sensor_height_cm
                },
                sensor: unit.sensor,
                // 16b cut W.1: the two live cells, one row per weapon, laid down
                // here so that every later lookup is a read of a complete table.
                // A weapon id declared twice on one unit keeps one row, which is
                // the same thing the component table itself is.
                ready_at_ms: unit
                    .weapons
                    .iter()
                    .map(|weapon| (weapon.id.clone(), 0_i64))
                    .collect(),
                ammo: unit
                    .weapons
                    .iter()
                    .map(|weapon| (weapon.id.clone(), weapon.count_per_load))
                    .collect(),
                weapons: unit.weapons,
                unit: unit.unit,
                lock: None,
                blocked: false,
                linkdead: false,
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
        // 16b §9: `tempo.alert_range_cm` is *2 × max weapon range*, and the
        // configured `0` is what asks for that. Derived once, here, from the units
        // this match was handed; a match whose units carry no weapon derives `0`
        // and never raises `Alert` on a held contact, which is the honest answer.
        let alert_range_cm = if arena_cfg.tempo.alert_range_cm == 0 {
            states
                .iter()
                .flat_map(|state| state.weapons.iter())
                .map(|weapon| weapon.max_range_cm)
                .max()
                .unwrap_or(0)
                .saturating_mul(2)
        } else {
            arena_cfg.tempo.alert_range_cm
        };
        let mut arena = Arena {
            definition,
            units: states,
            epochs,
            contacts: ShadowContacts::new(),
            due,
            ledger: Ledger::new(&arena_cfg.ledger),
            index,
            clock: Clock::new(),
            seed,
            pilots,
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
        };
        // Doc 16 §3.6: one pass per unit per `sensors.pass_ms`, staggered by unit
        // id. Scheduled after the arena is built because the stagger reads the
        // sorted unit vector, and before the opening hash because the hash folds
        // the schedule's length.
        arena.schedule_first_passes();
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
        // One scheduled event: the first position summary at `ledger.summary_ms`.
        assert_eq!(arena.due.len(), 1);
        assert_eq!(arena.due.next_t_ms(), Some(5_000));
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
            ShadowContact {
                target: ObjId(9),
                class: Classification::Unknown,
                first_seen_ms: 1_000,
                last_seen_ms: 1_000,
            },
        );
        contacts.insert(
            viewer,
            second,
            ShadowContact {
                target: ObjId(10),
                class: Classification::Classified { team: Team::Blue },
                first_seen_ms: 1_000,
                last_seen_ms: 4_000,
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
