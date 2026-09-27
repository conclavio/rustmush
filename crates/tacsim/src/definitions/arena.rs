//! **Where a match is fought, authored** (16b cut C.2, C1/C3; doc 16 §3.2
//! "Space", §4 "Content packs"; D-16-5).
//!
//! The arena entry names ground the **atlas** owns and lays the match's own
//! furniture over it: the window's corner and side, the two team entry zones,
//! the named key spots, the waypoint graph cut N's reflex controller patrols,
//! and the stock tanks the applier makes (twelve since 16b cut Z.3).
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
    ArenaDefinition, ArenaMode, ArenaTerrain, EntryZone, HoldRule, KeySpot as ArenaKeySpot,
    PROVISIONAL_PREFIX, Team, TerrainCell, Waypoint as ArenaWaypoint, Waypoints,
};
use crate::definitions::DefinitionError;
use crate::definitions::unit::TRACKED;

/// The most heightfield entries one arena may name. Four tile the 256 by 256
/// window at C.1's 128 by 128 quadrants; sixteen is the same window at 64.
pub const MAX_ARENA_TILES: usize = 16;

/// The most named key spots one arena may carry (doc 16 §3.2: *three or four*).
///
/// **16b cut V.1 made it the alphabet's length.** Every spot is drawn on the
/// map as one letter of [`crate::match_::cockpit::SPOT_LETTERS`] by its authored
/// index, and those are the twenty-two capitals that are neither a terrain glyph
/// (`D`, `T`, `W`) nor the provisional prefix (`X`); a spot past them could not
/// be drawn, so it is refused here, at load. Doc 16 §3.2 asks for three or four
/// and the shipped ground has four, so no content moved. A test in
/// `match_::cockpit` pins the two numbers together.
pub const MAX_KEY_SPOTS: usize = 22;

/// **The longest a key spot's name may be**, bytes (16b cut V.1). The web
/// cockpit cleans every legend name to forty characters and the TUI and the line
/// client print a name whole, so the three legends only agree under this bound.
pub const MAX_SPOT_NAME_BYTES: usize = 40;

/// The most waypoint nodes one arena may carry. Cut N walks the graph inside a
/// pulse, so its size is a bound and not a preference.
pub const MAX_WAYPOINT_NODES: usize = 256;

/// The most waypoint edges.
pub const MAX_WAYPOINT_EDGES: usize = 1_024;

/// The most stock units one arena may place — `match.max_slots_per_side` (12)
/// on each side (16b §9).
pub const MAX_ARENA_UNITS: usize = 24;

/// **The widest a Hold square may be**, cells either side of its spot (16b cut
/// O.1; plan item 1): the presence test is one Chebyshev comparison per unit
/// per substep whatever the radius, and thirty-two cells is 320 m on the shipped
/// ground — a square 650 m across, wider than any spot a side could be said to
/// hold.
pub const MAX_HOLD_RADIUS_CELLS: u16 = 32;

/// **The shortest a Hold may last**, arena milliseconds (16b cut O.1): ten
/// seconds, which is what makes the settle order's tie rule true — a
/// `HoldSettles` is scheduled at least this long before it pops, so it carries a
/// lower sequence than any impact due at its own millisecond.
pub const MIN_HOLD_MS: u32 = 10_000;

/// **The longest a Hold may last**, arena milliseconds (16b cut O.1): an hour,
/// the stock `match.max_duration_ms`.
pub const MAX_HOLD_MS: u32 = 3_600_000;

/// **One mode a ground declares it supports** (16b cut O.1; 16c N11), beyond the
/// skirmish every ground supports and none declares.
///
/// Named `ArenaModeDef` and not `ModeDef` because [`crate::definitions::unit`]
/// already owns that name for a mobility mode, and strict rustdoc wants every
/// intra-doc link to resolve one way.
///
/// ```json
/// { "mode": "hold", "spot": "The Crossroads", "radius_cells": 6, "hold_ms": 120000 }
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArenaModeDef {
    /// **Keep a key spot uncontested for a time** — the side alone within
    /// `radius_cells` of the named spot for `hold_ms` wins.
    Hold {
        /// One of the arena's key spots, by its exact authored name.
        spot: String,
        /// The half-side of the square round the spot, cells:
        /// `1..=`[`MAX_HOLD_RADIUS_CELLS`].
        radius_cells: u16,
        /// How long a side must be alone on it, milliseconds: whole seconds,
        /// [`MIN_HOLD_MS`] to [`MAX_HOLD_MS`].
        hold_ms: u32,
    },
}

impl ArenaModeDef {
    /// The mode's word — what a refusal names it by.
    #[must_use]
    pub const fn word(&self) -> &'static str {
        match self {
            ArenaModeDef::Hold { .. } => "hold",
        }
    }
}

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
    /// **The modes this ground supports** beyond the skirmish (16b cut O.1; 16c
    /// N11), in authored order. Absent and `[]` are the same thing — a ground
    /// that is fought only to elimination — and an empty list is not written.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<ArenaModeDef>,
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
        for (index, spot) in self.key_spots.iter().enumerate() {
            crate::definitions::check_label(entry, &spot.name)?;
            // 16b cut V.1: and the name is one a pilot can type and three
            // legends can print alike.
            check_spot_name(
                entry,
                index,
                &spot.name,
                self.key_spots.get(..index).unwrap_or_default(),
            )?;
            self.on_window(entry, spot.x, spot.y, &format!("key spot `{}`", spot.name))?;
        }
        self.check_modes(entry)?;
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

    /// **The modes a ground declares, checked at load** (16b cut O.1; plan item
    /// 20), after the key spots they name. Six refusals, in this order, for each
    /// declaration:
    ///
    /// 1. a mode word **declared twice** — one ground, one rule per mode;
    /// 2. a `spot` that is not **exactly** one of the arena's key-spot names —
    ///    case-sensitive, because the name is resolved once, here and in
    ///    [`from_pack`], and a second spelling would be a second name;
    /// 3. `radius_cells` outside `1..=`[`MAX_HOLD_RADIUS_CELLS`];
    /// 4. `hold_ms` outside [`MIN_HOLD_MS`]`..=`[`MAX_HOLD_MS`];
    /// 5. `hold_ms` that is **not whole seconds** — the settle is at
    ///    `since + hold_ms`, `since` is a substep boundary, and a whole second is
    ///    what keeps the win's millisecond on every cadence's lattice;
    /// 6. a hold square (`[x−r, x+r] × [y−r, y+r]`, clipped to the window) that
    ///    **reaches an entry zone** — a side that musters on the spot would hold
    ///    it from the first substep.
    ///
    /// Validation encodes nothing, so a declaration that passes digests exactly
    /// as its bytes do.
    fn check_modes(&self, entry: &str) -> Result<(), DefinitionError> {
        let mut words: Vec<&'static str> = Vec::new();
        for mode in &self.modes {
            let word = mode.word();
            if words.contains(&word) {
                return Err(DefinitionError::value(
                    entry,
                    format!("mode `{word}` is declared twice"),
                ));
            }
            words.push(word);
            match mode {
                ArenaModeDef::Hold {
                    spot,
                    radius_cells,
                    hold_ms,
                } => {
                    let Some(at) = self.key_spots.iter().find(|named| named.name == *spot) else {
                        return Err(DefinitionError::value(
                            entry,
                            format!(
                                "mode `{word}` names key spot `{spot}`, which the arena does not \
                                 declare"
                            ),
                        ));
                    };
                    if *radius_cells == 0 || *radius_cells > MAX_HOLD_RADIUS_CELLS {
                        return Err(DefinitionError::value(
                            entry,
                            format!(
                                "mode `{word}` radius_cells is 1 to {MAX_HOLD_RADIUS_CELLS}; \
                                 {radius_cells} is not"
                            ),
                        ));
                    }
                    if !(MIN_HOLD_MS..=MAX_HOLD_MS).contains(hold_ms) {
                        return Err(DefinitionError::value(
                            entry,
                            format!(
                                "mode `{word}` hold_ms is {MIN_HOLD_MS} to {MAX_HOLD_MS}; \
                                 {hold_ms} is not"
                            ),
                        ));
                    }
                    if hold_ms % 1_000 != 0 {
                        return Err(DefinitionError::value(
                            entry,
                            format!("mode `{word}` hold_ms is whole seconds; {hold_ms} is not"),
                        ));
                    }
                    let x0 = at.x.saturating_sub(*radius_cells);
                    let x1 =
                        at.x.saturating_add(*radius_cells)
                            .min(self.cols.saturating_sub(1));
                    let y0 = at.y.saturating_sub(*radius_cells);
                    let y1 =
                        at.y.saturating_add(*radius_cells)
                            .min(self.rows.saturating_sub(1));
                    if let Some(zone) = self.entry_zones.iter().find(|zone| {
                        x0 <= zone.x1 && zone.x0 <= x1 && y0 <= zone.y1 && zone.y0 <= y1
                    }) {
                        return Err(DefinitionError::value(
                            entry,
                            format!(
                                "mode `{word}` at `{spot}` reaches the {} entry zone",
                                zone.team.as_str()
                            ),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// **The declared modes, resolved** (16b cut O.1): each
    /// [`ArenaModeDef::Hold`] as an [`ArenaMode::Hold`] whose `spot` is the
    /// named key spot's **index** — what [`from_pack`] puts on the definition,
    /// and what `match_::lobby`'s words read without building a window. A
    /// declaration naming no spot (which [`ArenaDef::check`] refuses at load) is
    /// left out rather than guessed at.
    #[must_use]
    pub fn arena_modes(&self) -> Vec<ArenaMode> {
        self.modes
            .iter()
            .filter_map(|mode| match mode {
                ArenaModeDef::Hold {
                    spot,
                    radius_cells,
                    hold_ms,
                } => self
                    .key_spots
                    .iter()
                    .position(|named| named.name == *spot)
                    .and_then(|index| u16::try_from(index).ok())
                    .map(|index| {
                        ArenaMode::Hold(HoldRule {
                            spot: index,
                            radius_cells: *radius_cells,
                            hold_ms: *hold_ms,
                        })
                    }),
            })
            .collect()
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

/// **A key spot's name, checked at load** (16b cut V.1), beside
/// [`crate::definitions::check_label`]'s byte bound. `earlier` is the spots
/// authored before this one, `index` this one's place in the list.
///
/// The rules, each with its reason:
///
/// * **2 to [`MAX_SPOT_NAME_BYTES`] bytes of printable ASCII** (`0x20..=0x7E`),
///   **not all spaces** — the web cleans a legend name to forty characters and
///   the TUI and the line client print it whole, so the three legends only agree
///   under a bound and over bytes all three print as themselves; and one byte
///   would be a name no longer than the map letter it stands beside;
/// * **no space at either end** — `RANGE` trims the word a pilot types and the
///   web trims the name it prints, so `The Ford ` could be reached only by its
///   letter and would read one way on the web and another on the TUI and the
///   line client;
/// * **unique when ASCII-case-folded** — `RANGE the ford` folds case, so two
///   names that fold together are one word naming two places (every earlier
///   name already passed the edge rule, so the fold compares trimmed names);
/// * **not a designator's word form** — a team letter (`R`, `B`) followed only by
///   ASCII letters, or [`PROVISIONAL_PREFIX`] followed only by ASCII digits, in
///   either case: the two shapes `match_::cockpit::resolve_designator` resolves.
///   `RANGE`/`BEARING` try the designator first and the spot second, and the
///   second arm only never collides with the first because no name can be one.
///
/// Validation encodes nothing, so the shipped ground's digest does not move.
///
/// # Errors
///
/// [`DefinitionError::Value`] naming the entry and the spot.
fn check_spot_name(
    entry: &str,
    index: usize,
    name: &str,
    earlier: &[KeySpot],
) -> Result<(), DefinitionError> {
    let refuse = |why: &str| {
        Err(DefinitionError::value(
            entry,
            format!("key spot {index} `{name}`: {why}"),
        ))
    };
    if name.len() < 2 || name.len() > MAX_SPOT_NAME_BYTES {
        return refuse(&format!(
            "a spot's name is 2 to {MAX_SPOT_NAME_BYTES} bytes; {} is not",
            name.len()
        ));
    }
    if !name.bytes().all(|byte| (0x20..=0x7E).contains(&byte)) {
        return refuse("a spot's name is printable ASCII");
    }
    if name.bytes().all(|byte| byte == b' ') {
        return refuse("a spot's name is not all spaces");
    }
    if name.trim() != name {
        return refuse("a spot's name neither starts nor ends with a space");
    }
    if let Some(twin) = earlier
        .iter()
        .find(|spot| spot.name.eq_ignore_ascii_case(name))
    {
        return refuse(&format!(
            "a spot's name is its own whatever the case; `{}` is taken",
            twin.name
        ));
    }
    if is_designator_shaped(name) {
        return refuse("a spot's name may not read as a contact's designator");
    }
    Ok(())
}

/// **Whether a word has the shape of a designator** (16b cut V.1), either case:
/// a team's letter followed only by ASCII letters (`RA`, `bAA`) — a slot in
/// bijective base 26 — or [`PROVISIONAL_PREFIX`] followed only by ASCII digits
/// (`X1`). The team letters are read off [`Team::ALL`], so a third side is
/// covered without touching this.
fn is_designator_shaped(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(first) = chars.next().map(|c| c.to_ascii_uppercase()) else {
        return false;
    };
    let rest = chars.as_str();
    if rest.is_empty() {
        return false;
    }
    if Team::ALL.iter().any(|team| team.letter() == first) {
        return rest.chars().all(|c| c.is_ascii_alphabetic());
    }
    first == PROVISIONAL_PREFIX && rest.chars().all(|c| c.is_ascii_digit())
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
    /// **16b cut M.4** (16c `T-14`) — an entry zone holds no cell a unit could
    /// open on: none that is passable with a passable orthogonal neighbour
    /// (`ArenaTerrain::has_way_out`). A side whose zone is all water or all
    /// walled-in cells could field nobody, or field a tank that can never move,
    /// so the content is refused when the arena is built. The sentence names the
    /// arena and the team, which are content, and no unit.
    #[error(
        "arena {arena}: team {team}'s entry zone has no passable cell with a passable neighbour"
    )]
    EntryZoneUnusable {
        /// The arena entry.
        arena: String,
        /// The side whose zone it is, by its wire word (`red`, `blue`).
        team: &'static str,
    },
    /// **16b cut U.4** (16c `T-57`) — the arena's `arena.map` keyframe would be
    /// past [`crate::match_::cockpit::MAP_MAX_BYTES`], the bound a pulled
    /// keyframe is held to. Refused when the arena is applied, so no loaded
    /// arena can make the map handler send one. See [`arena_map_within_bound`].
    #[error(
        "{arena}: the arena map is {bytes} bytes, past the {bound}-byte bound a pulled keyframe is held to"
    )]
    MapTooLarge {
        /// The arena entry.
        arena: String,
        /// How many bytes its widest map frame encodes to.
        bytes: usize,
        /// The bound it passed.
        bound: usize,
    },
}

/// **The widest `match` an `arena.map` frame can carry** (16b cut U.4): the
/// frame spells the match id as a JSON integer, and `u64::MAX` is its longest
/// spelling — twenty digits against `MatchId(0)`'s one.
const WIDEST_MATCH: crate::arena_task::MatchId = crate::arena_task::MatchId(u64::MAX);

/// **Whether an arena's map fits the bound a pulled keyframe is held to** (16b
/// cut U.4, 16c `T-57`), answering the encoded size.
///
/// It encodes [`crate::match_::cockpit::map_frame`] — the frame the `arena.map`
/// handler sends, and nothing else — at its widest: `WIDEST_MATCH` (`u64::MAX`) for the
/// `match` key and, for `surface`, `entry` itself, because the handler's surface
/// is the running definition's `id` and [`from_pack`] builds that `id` from the
/// entry it is given (the stock stand-in ids never reach an applier). So a
/// definition this passes can never be sent larger.
///
/// Called **once, at apply**, by the arena applier after [`from_pack`] — not
/// inside `from_pack`, which `crate::match_::definition_for` rebuilds on every
/// match start and every `@arena/pose`, and which must not encode a 256 × 256
/// map each time.
///
/// **16b cut V.1**: `spots` is the definition's own `key_spots`, encoded with
/// the terrain because they are bytes of the very frame this bounds.
///
/// # Errors
///
/// [`ArenaBuildError::MapTooLarge`], naming the entry, the size and the bound.
pub fn arena_map_within_bound(
    entry: &str,
    terrain: &ArenaTerrain,
    spots: &[ArenaKeySpot],
) -> Result<usize, ArenaBuildError> {
    let frame = crate::match_::cockpit::map_frame(WIDEST_MATCH, entry, terrain, spots);
    // A `Value` always encodes; `usize::MAX` would refuse rather than pass, which
    // is the fail-closed direction if it ever did not.
    let bytes = serde_json::to_vec(&frame).map_or(usize::MAX, |encoded| encoded.len());
    let bound = crate::match_::cockpit::MAP_MAX_BYTES;
    if bytes > bound {
        return Err(ArenaBuildError::MapTooLarge {
            arena: entry.to_string(),
            bytes,
            bound,
        });
    }
    Ok(bytes)
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
    // **16b cut M.4** (16c `T-14`): every entry zone holds at least one cell a
    // unit could open on and drive out of, or no check-out can place that side.
    for zone in &def.entry_zones {
        let usable =
            (zone.y0..=zone.y1).any(|y| (zone.x0..=zone.x1).any(|x| terrain.has_way_out(x, y)));
        if !usable {
            return Err(ArenaBuildError::EntryZoneUnusable {
                arena: entry.to_string(),
                team: zone.team.as_str(),
            });
        }
    }
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
        // **16b cut O.1** (16c N11): the declared modes, each spot resolved from
        // its name to its index once, here, as the graph's edges are above.
        modes: def.arena_modes(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(name: &str) -> KeySpot {
        KeySpot {
            name: name.to_owned(),
            x: 0,
            y: 0,
        }
    }

    /// The detail a refusal carries, or `None` for a name that passed.
    fn refused(name: &str, earlier: &[KeySpot]) -> Option<String> {
        match check_spot_name("tacsim.arena.test", earlier.len(), name, earlier) {
            Ok(()) => None,
            Err(DefinitionError::Value { entry, detail }) => {
                assert_eq!(entry, "tacsim.arena.test", "the refusal names the entry");
                assert!(
                    detail.contains(&format!("`{name}`")),
                    "and the spot: {detail}"
                );
                Some(detail)
            }
            Err(other) => panic!("a spot's name refuses as a value: {other:?}"),
        }
    }

    /// **16b cut V.1**: every rule `check_spot_name` holds a name to, one case
    /// each, and the shipped four passing them all.
    #[test]
    fn a_spot_name_is_short_printable_unique_and_never_a_designator() {
        // The shipped ground's four, in authored order, each against the ones
        // before it — which is how `ArenaDef::check` calls it.
        let shipped = ["The Ford", "The Crossroads", "North Barn", "East Compound"];
        let mut earlier: Vec<KeySpot> = Vec::new();
        for name in shipped {
            assert_eq!(refused(name, &earlier), None, "{name} is a good name");
            earlier.push(spot(name));
        }
        // Length: one byte is shorter than a name may be, forty-one longer.
        assert!(refused("A", &[]).is_some_and(|why| why.contains("2 to 40 bytes")));
        let long = "N".repeat(41);
        assert!(refused(&long, &[]).is_some_and(|why| why.contains("41 is not")));
        assert_eq!(refused(&"N".repeat(40), &[]), None, "forty is the bound");
        // Printable ASCII: a non-ASCII byte and a control byte both refuse.
        assert!(refused("The F\u{f6}rd", &[]).is_some_and(|why| why.contains("printable ASCII")));
        assert!(refused("The\tFord", &[]).is_some_and(|why| why.contains("printable ASCII")));
        // All spaces.
        assert!(refused("   ", &[]).is_some_and(|why| why.contains("all spaces")));
        // A space at either end: the typed word is trimmed and the web trims the
        // name it prints, so an edge space makes a name no pilot can type and
        // two legends that disagree — and would slip `The Ford ` past the twin
        // rule beside `The Ford`.
        for name in ["The Ford ", " The Ford", " Ford "] {
            assert!(
                refused(name, &[]).is_some_and(|why| why.contains("starts nor ends")),
                "`{name}` has a space at an edge"
            );
        }
        assert!(
            refused("The Ford ", &[spot("The Ford")]).is_some(),
            "a trailing-space twin of an earlier name never loads"
        );
        // Two names that differ only in case are one word naming two places.
        assert!(refused("THE FORD", &[spot("The Ford")]).is_some_and(|why| why.contains("taken")));
        // A designator's shapes, in either case: a team letter and letters, the
        // provisional prefix and digits.
        for word in ["RA", "BAA", "ra", "Bc", "X1", "x12"] {
            assert!(
                refused(word, &[]).is_some_and(|why| why.contains("designator")),
                "{word} reads as a designator"
            );
        }
        // …and the near misses that do not: a space, a digit after a team
        // letter, a letter after the prefix, and a letter that is no team's.
        for word in ["R A", "R1", "XA", "X 1", "Ford", "AB"] {
            assert_eq!(refused(word, &[]), None, "{word} is not a designator");
        }
    }

    /// The cap is the alphabet's length (16b cut V.1), so every spot an arena
    /// may load has a letter to be drawn with.
    #[test]
    fn the_spot_cap_is_twenty_two() {
        assert_eq!(MAX_KEY_SPOTS, 22);
        assert_eq!(MAX_KEY_SPOTS, crate::match_::cockpit::SPOT_LETTERS.len());
    }

    // -----------------------------------------------------------------------
    // 16b cut O.1 — the modes a ground declares
    // -----------------------------------------------------------------------

    /// The shipped arena's own entry, decoded — a real, valid `ArenaDef` to
    /// vary one field of.
    fn shipped() -> ArenaDef {
        let doc: serde_json::Value =
            serde_json::from_str(crate::definitions::content::PROVING_GROUND_JSON)
                .expect("the arena pack is json");
        serde_json::from_value(doc["entries"]["tacsim.arena.proving_ground"]["def"].clone())
            .expect("the shipped arena entry decodes")
    }

    fn hold(spot: &str, radius_cells: u16, hold_ms: u32) -> ArenaModeDef {
        ArenaModeDef::Hold {
            spot: spot.to_owned(),
            radius_cells,
            hold_ms,
        }
    }

    /// The detail `check` refuses `def` with, or `None` when it passes.
    fn refusal(def: &ArenaDef) -> Option<String> {
        match def.check("tacsim.arena.test") {
            Ok(()) => None,
            Err(DefinitionError::Value { detail, .. }) => Some(detail),
            Err(other) => panic!("a mode refuses as a value: {other:?}"),
        }
    }

    /// **16b cut O.1**: every one of `check`'s six mode refusals, verbatim, in
    /// its order, and the shipped declaration passing them all.
    #[test]
    fn o1_the_six_mode_refusals_are_verbatim() {
        let with = |modes: Vec<ArenaModeDef>| {
            let mut def = shipped();
            def.modes = modes;
            refusal(&def)
        };
        // The shipped declaration passes.
        assert_eq!(with(vec![hold("The Crossroads", 6, 120_000)]), None);
        // 1. Declared twice.
        assert_eq!(
            with(vec![
                hold("The Crossroads", 6, 120_000),
                hold("North Barn", 6, 120_000)
            ]),
            Some("mode `hold` is declared twice".to_owned())
        );
        // 2. A spot the arena does not declare — and one differing only in case.
        assert_eq!(
            with(vec![hold("The Pub", 6, 120_000)]),
            Some(
                "mode `hold` names key spot `The Pub`, which the arena does not declare".to_owned()
            )
        );
        assert_eq!(
            with(vec![hold("the crossroads", 6, 120_000)]),
            Some(
                "mode `hold` names key spot `the crossroads`, which the arena does not declare"
                    .to_owned()
            ),
            "the name is matched exactly, case and all"
        );
        // 3. The radius, both ends.
        for radius in [0, MAX_HOLD_RADIUS_CELLS + 1] {
            assert_eq!(
                with(vec![hold("The Crossroads", radius, 120_000)]),
                Some(format!(
                    "mode `hold` radius_cells is 1 to 32; {radius} is not"
                ))
            );
        }
        assert_eq!(
            with(vec![hold("North Barn", MAX_HOLD_RADIUS_CELLS, 120_000)]),
            None
        );
        // 4. The time, both ends.
        for ms in [MIN_HOLD_MS - 1_000, MAX_HOLD_MS + 1_000] {
            assert_eq!(
                with(vec![hold("The Crossroads", 6, ms)]),
                Some(format!(
                    "mode `hold` hold_ms is 10000 to 3600000; {ms} is not"
                ))
            );
        }
        assert_eq!(with(vec![hold("The Crossroads", 6, MIN_HOLD_MS)]), None);
        assert_eq!(with(vec![hold("The Crossroads", 6, MAX_HOLD_MS)]), None);
        // 5. Whole seconds.
        assert_eq!(
            with(vec![hold("The Crossroads", 6, 10_500)]),
            Some("mode `hold` hold_ms is whole seconds; 10500 is not".to_owned())
        );
        // 6. A square that reaches an entry zone: a spot at column 20 with a
        //    square of six reaches the red zone's column 15; a square of four
        //    stops at column 16.
        let mut near = shipped();
        near.key_spots.push(KeySpot {
            name: "Red Gate".to_owned(),
            x: 20,
            y: 50,
        });
        near.modes = vec![hold("Red Gate", 6, 120_000)];
        assert_eq!(
            refusal(&near),
            Some("mode `hold` at `Red Gate` reaches the red entry zone".to_owned())
        );
        // The exact touch: a square of five starts ON column 15, the zone's
        // own last column — one shared column is a reach.
        near.modes = vec![hold("Red Gate", 5, 120_000)];
        assert_eq!(
            refusal(&near),
            Some("mode `hold` at `Red Gate` reaches the red entry zone".to_owned()),
            "a square sharing the zone's edge column reaches it"
        );
        near.modes = vec![hold("Red Gate", 4, 120_000)];
        assert_eq!(refusal(&near), None, "column 16 is clear of the zone");
    }

    /// **16b cut O.1**: `modes` absent and `[]` both parse to the empty list,
    /// and an empty list is not written back.
    #[test]
    fn o1_modes_absent_and_empty_both_parse() {
        let doc: serde_json::Value =
            serde_json::from_str(crate::definitions::content::PROVING_GROUND_JSON)
                .expect("the arena pack is json");
        let mut entry = doc["entries"]["tacsim.arena.proving_ground"]["def"].clone();
        entry
            .as_object_mut()
            .expect("an object")
            .remove("modes")
            .expect("the shipped entry declares its modes");
        let absent: ArenaDef = serde_json::from_value(entry.clone()).expect("absent parses");
        assert!(absent.modes.is_empty());
        entry["modes"] = serde_json::json!([]);
        let empty: ArenaDef = serde_json::from_value(entry).expect("[] parses");
        assert!(empty.modes.is_empty());
        let written = serde_json::to_value(&empty).expect("encodes");
        assert!(
            written.get("modes").is_none(),
            "an empty list is not written"
        );
        // An unknown key on a declaration is refused, as everywhere in a pack.
        let bad: Result<ArenaModeDef, _> = serde_json::from_value(serde_json::json!({
            "mode": "hold", "spot": "The Crossroads", "radius_cells": 6, "hold_ms": 120000,
            "points": 3
        }));
        assert!(bad.is_err());
    }

    /// **16b cut O.1**: the declaration resolves its spot's name to its index,
    /// which is what `from_pack` puts on the definition.
    #[test]
    fn o1_the_declared_spot_resolves_to_its_index() {
        let def = shipped();
        assert_eq!(
            def.arena_modes(),
            vec![ArenaMode::Hold(HoldRule {
                spot: 1,
                radius_cells: 6,
                hold_ms: 120_000,
            })]
        );
        let mut none = def.clone();
        none.modes = Vec::new();
        assert!(none.arena_modes().is_empty());
        // A name `check` would refuse is left out, never guessed at.
        let mut stray = def;
        stray.modes = vec![hold("The Pub", 6, 120_000)];
        assert!(stray.arena_modes().is_empty());
    }
}
