//! **The keyed geology function** (doc 20 cut SM.1; §3.2, §11's Geology row, D-20-4).
//!
//! The ground under a body is a pure function of a body-fixed position and the deployment's
//! geology key: HMAC-SHA256 ([`mush_core::GeologyMac`]) over the cell's address, generated
//! **top-down** so every reading nests inside the one above it:
//!
//! 1. **province** — one cell of the body's 56 × 28 chart (§3.1; 6.43° at the equator): per
//!    raw family a word, drawn from the body's province row for that family (R0, the
//!    Registry's survey);
//! 2. **field** — 320 m, an 8 × 8 square of pockets: per family a word drawn from the field
//!    table's row for the province's word, moved by a block's authored class shift (§3.1's
//!    `prospectivity`; never where the province is barren in that family — the SM.1 brief's
//!    decision 13, held twice: the table's `barren` field row weights barren alone, and the
//!    shift skips a barren-province family), and then an ore-pocket count inside that word's
//!    range (R1);
//! 3. **pocket** — 40 m: per family barren, lean, fair or rich, with tonnes inside the class's
//!    range (R2). The field's count of ore pockets is placed by a keyed rank, at most two
//!    families a pocket, so **the field's word is always the honest summary of its pockets**.
//!
//! One word set everywhere — barren, lean, fair, rich ([`Word`]). The count and tonnes ranges
//! are code constants (§11), not table data, so "at most 64 ore pockets a block a family" (four
//! fields of at most 16) holds for any table a lane authors. Integers only; no RNG — the same
//! key, body, level, cell and family give the same draw forever (D-20-4).
//!
//! **Truth never serializes** (doc 20 §16, "no surface that reveals truth"): the ground types
//! here derive no serde traits (`truth_serialization_policy.rs` asserts it). The vocabulary
//! ([`OreFamily`], [`Word`]) and the table's lane wire ([`GeologyTableWire`]) do.
//!
//! **The lattice** is body-fixed (x east, y north of the equator), its width a whole number of
//! fields around the equator from the integer circumference `radius × 2π` with 2π ≈ 710/113,
//! its height half that, rounded down to whole fields on each side of the equator. `x` wraps;
//! every coordinate is normalised by its own level's width **before** it enters the HMAC, so the
//! seam is invisible; `y` never wraps (off the lattice is [`GeologyError::OffLattice`]).

use std::collections::BTreeMap;

use mush_core::{CanonicalId, GeologyMac};
use serde::{Deserialize, Serialize};

/// The HMAC domain of every geology draw (version 1 of the message layout).
pub const GEOLOGY_DOMAIN: &[u8] = b"rustmush.geology.v1\0";
/// The most ore pockets a field holds of one family (a rich field's top, §11).
pub const MAX_FIELD_POCKETS: u8 = 16;
/// Pockets along one side of a field: 8 × 8 = 64 a field of 320 m.
pub const POCKETS_PER_FIELD_SIDE: i64 = 8;
/// Pockets a field holds.
pub const POCKETS_PER_FIELD: usize = 64;
/// The most families one pocket carries (§3.2).
pub const MAX_FAMILIES_PER_POCKET: u8 = 2;
/// The province chart's columns (§3.1: 1,568 provinces a body).
pub const PROVINCE_COLUMNS: u32 = 56;
/// The province chart's rows.
pub const PROVINCE_ROWS: u32 = 28;
/// The largest class shift a block may author either way (§3.1's `prospectivity`).
pub const MAX_SHIFT: i8 = 3;
/// The pocket's edge, metres (a field is 320 m).
pub const POCKET_M: i64 = 40;
/// The field's edge, metres.
pub const FIELD_M: i64 = 320;

/// The levels of the draw, the `level` byte of the message.
const LEVEL_PROVINCE: u8 = 0;
const LEVEL_FIELD: u8 = 1;
const LEVEL_POCKET: u8 = 2;
/// R1's regolith band side (`crate::surface::reading`).
pub(crate) const LEVEL_READING: u8 = 3;

// ---------------------------------------------------------------------------
// The vocabulary
// ---------------------------------------------------------------------------

/// Doc 10 4A's four raw families, in this fixed order (the order is part of the HMAC input and
/// of every table's layout); serde names are `definitions::resources::FAMILIES`' first four.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OreFamily {
    /// Metals.
    Metals,
    /// Volatiles.
    Volatiles,
    /// Carbon feedstock.
    CarbonFeedstock,
    /// Strategic elements.
    StrategicElements,
}

impl OreFamily {
    /// Every family, in the fixed order.
    pub const ALL: [OreFamily; 4] = [
        OreFamily::Metals,
        OreFamily::Volatiles,
        OreFamily::CarbonFeedstock,
        OreFamily::StrategicElements,
    ];

    /// The family's code, 0..=3: its byte in the HMAC message and its slot in every table.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            OreFamily::Metals => 0,
            OreFamily::Volatiles => 1,
            OreFamily::CarbonFeedstock => 2,
            OreFamily::StrategicElements => 3,
        }
    }

    /// The family's id (`definitions::resources::FAMILIES`).
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            OreFamily::Metals => "metals",
            OreFamily::Volatiles => "volatiles",
            OreFamily::CarbonFeedstock => "carbon_feedstock",
            OreFamily::StrategicElements => "strategic_elements",
        }
    }

    /// The family as a player reads it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            OreFamily::Metals => "metals",
            OreFamily::Volatiles => "volatiles",
            OreFamily::CarbonFeedstock => "carbon feedstock",
            OreFamily::StrategicElements => "strategic elements",
        }
    }
}

/// **A family by a word a player typed** (doc 20 cut SM.4b: `+tv/map [family]`; SM.7's
/// `+tv/mine [family]` reuses it), case-insensitive: `metal`, `metals`, `metal_ore`; `ice`,
/// `volatiles`, `ice_ore`; `carbon`, `carbon_ore`, `carbon_feedstock`; `strategic`,
/// `strategic_ore`, `strategic_elements`. `None` for anything else.
#[must_use]
pub fn family_of_word(word: &str) -> Option<OreFamily> {
    match word.to_ascii_lowercase().as_str() {
        "metal" | "metals" | "metal_ore" => Some(OreFamily::Metals),
        "ice" | "volatiles" | "ice_ore" => Some(OreFamily::Volatiles),
        "carbon" | "carbon_ore" | "carbon_feedstock" => Some(OreFamily::CarbonFeedstock),
        "strategic" | "strategic_ore" | "strategic_elements" => Some(OreFamily::StrategicElements),
        _ => None,
    }
}

/// The one word set (doc 20 §3.2): the code's P0–P3/A–D never reach a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Word {
    /// No ore.
    Barren,
    /// Lean.
    Lean,
    /// Fair.
    Fair,
    /// Rich.
    Rich,
}

impl Word {
    /// Every word, lowest first.
    pub const ALL: [Word; 4] = [Word::Barren, Word::Lean, Word::Fair, Word::Rich];

    /// The word's code, 0..=3.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Word::Barren => 0,
            Word::Lean => 1,
            Word::Fair => 2,
            Word::Rich => 3,
        }
    }

    /// The word of a code, `None` past 3.
    #[must_use]
    pub const fn of_code(code: u8) -> Option<Word> {
        match code {
            0 => Some(Word::Barren),
            1 => Some(Word::Lean),
            2 => Some(Word::Fair),
            3 => Some(Word::Rich),
            _ => None,
        }
    }

    /// The word as a player reads it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Word::Barren => "barren",
            Word::Lean => "lean",
            Word::Fair => "fair",
            Word::Rich => "rich",
        }
    }

    /// The ore-pocket count a field word stands for (§3.2, §11): 0, 1–4, 5–8, 9–16.
    #[must_use]
    pub const fn pocket_range(self) -> (u8, u8) {
        match self {
            Word::Barren => (0, 0),
            Word::Lean => (1, 4),
            Word::Fair => (5, 8),
            Word::Rich => (9, MAX_FIELD_POCKETS),
        }
    }

    /// The tonnes a pocket class holds (§3.2, §11): 0, 3–6, 6–12, 12–24.
    #[must_use]
    pub const fn tonnes_range(self) -> (u8, u8) {
        match self {
            Word::Barren => (0, 0),
            Word::Lean => (3, 6),
            Word::Fair => (6, 12),
            Word::Rich => (12, 24),
        }
    }

    /// The field word a count is the honest summary of (the inverse of
    /// [`Word::pocket_range`]); `None` above 16.
    #[must_use]
    pub fn of_count(count: u8) -> Option<Word> {
        Word::ALL.into_iter().find(|word| {
            let (lo, hi) = word.pocket_range();
            (lo..=hi).contains(&count)
        })
    }

    /// `self` moved by `shift` classes, saturating at barren and rich (§3.1's authored shift).
    #[must_use]
    pub fn shifted(self, shift: i8) -> Word {
        let moved = i16::from(self.code())
            .saturating_add(i16::from(shift))
            .clamp(0, 3);
        u8::try_from(moved)
            .ok()
            .and_then(Word::of_code)
            .unwrap_or(if shift < 0 { Word::Barren } else { Word::Rich })
    }
}

/// One of four, by family, without indexing.
#[must_use]
pub(crate) fn by_family<T: Copy>(values: [T; 4], family: OreFamily) -> T {
    let [metals, volatiles, carbon, strategic] = values;
    match family {
        OreFamily::Metals => metals,
        OreFamily::Volatiles => volatiles,
        OreFamily::CarbonFeedstock => carbon,
        OreFamily::StrategicElements => strategic,
    }
}

/// One of four, by word, without indexing.
#[must_use]
pub(crate) fn by_word<T: Copy>(values: [T; 4], word: Word) -> T {
    let [barren, lean, fair, rich] = values;
    match word {
        Word::Barren => barren,
        Word::Lean => lean,
        Word::Fair => fair,
        Word::Rich => rich,
    }
}

/// Set one of four, by family, without indexing.
pub(crate) fn set_family<T>(values: &mut [T; 4], family: OreFamily, value: T) {
    let [metals, volatiles, carbon, strategic] = values;
    let slot = match family {
        OreFamily::Metals => metals,
        OreFamily::Volatiles => volatiles,
        OreFamily::CarbonFeedstock => carbon,
        OreFamily::StrategicElements => strategic,
    };
    *slot = value;
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why a geology call refused. Every variant names what it refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GeologyError {
    /// The table has no province row for a family.
    #[error("the geology table has no province row for {0}")]
    MissingProvinceRow(&'static str),
    /// The table has no field row for a province word.
    #[error("the geology table has no field row for a {0} province")]
    MissingFieldRow(&'static str),
    /// The table has no pocket row for a field word.
    #[error("the geology table has no pocket row for a {0} field")]
    MissingPocketRow(&'static str),
    /// A pocket row keyed `barren`: a barren field has no ore pockets to class.
    #[error("the geology table's pocket rows are lean, fair and rich; a barren field has none")]
    BarrenPocketRow,
    /// A pocket row weighting `barren`: an ore pocket is never barren.
    #[error("the geology table's {0} pocket row weights barren; an ore pocket is never barren")]
    BarrenPocketClass(&'static str),
    /// The `barren` field row weighting any ore word: a province barren in a family has no ore
    /// fields in it (the SM.1 brief's decision 13; R0 never lies by omission).
    #[error(
        "the geology table's field barren row weights {0}; a province barren in a family has \
         only barren fields in it"
    )]
    BarrenProvinceField(&'static str),
    /// A row whose weights do not sum to 1,000.
    #[error("the geology table's {row} row sums to {sum}, not 1000")]
    RowSum {
        /// The row, as `province metals`, `field fair` or `pocket rich`.
        row: String,
        /// What it summed to.
        sum: u32,
    },
    /// A body radius outside 1..=10,000 km.
    #[error("a body's radius {0} km is outside 1 to 10000")]
    Radius(u32),
    /// A point off the body's lattice (past a pole).
    #[error("the point is off the body's lattice")]
    OffLattice,
    /// A class shift outside -3..=3.
    #[error("a class shift of {0} is outside -3 to 3")]
    Shift(i8),
    /// A site origin that is not a field corner.
    #[error("a site's origin must be a field corner (both coordinates multiples of 8)")]
    Origin,
    /// A site spec naming another body.
    #[error("the site names {spec}, not {body}")]
    BodyMismatch {
        /// The body the spec names.
        spec: String,
        /// The body the table is for.
        body: String,
    },
}

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

/// A body's geology table on the wire (lane data from SM.2): per-mille weights, every row summing
/// to 1,000, a missing word 0. Strict at every level.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeologyTableWire {
    /// Per family, the province word's weights.
    pub province: BTreeMap<OreFamily, BTreeMap<Word, u16>>,
    /// Per province word, the field word's weights.
    pub field: BTreeMap<Word, BTreeMap<Word, u16>>,
    /// Per field word (lean, fair, rich), an ore pocket's class weights (never barren).
    pub pocket: BTreeMap<Word, BTreeMap<Word, u16>>,
}

/// A checked geology table: three 4 × 4 per-mille grids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeologyTable {
    /// family × province word.
    province: [[u16; 4]; 4],
    /// province word × field word.
    field: [[u16; 4]; 4],
    /// field word × pocket class; the barren row and the barren column are all zero.
    pocket: [[u16; 4]; 4],
}

/// One wire row as four weights, a missing word 0, checked to sum to 1,000.
fn row_of(wire: &BTreeMap<Word, u16>, name: String) -> Result<[u16; 4], GeologyError> {
    let row = Word::ALL.map(|word| wire.get(&word).copied().unwrap_or(0));
    let sum = row
        .iter()
        .fold(0_u32, |acc, w| acc.saturating_add(u32::from(*w)));
    if sum == 1_000 {
        Ok(row)
    } else {
        Err(GeologyError::RowSum { row: name, sum })
    }
}

/// Four weights back to a wire row, zero weights omitted.
fn wire_of(row: [u16; 4]) -> BTreeMap<Word, u16> {
    Word::ALL
        .into_iter()
        .zip(row)
        .filter(|(_, weight)| *weight > 0)
        .collect()
}

impl GeologyTable {
    /// Check a wire table: a province row for each of the four families, a field row for each of
    /// the four words (the `barren` one weighting barren alone: decision 13), pocket rows for
    /// exactly lean, fair and rich with no barren weight, every row summing to exactly 1,000.
    ///
    /// # Errors
    ///
    /// The first refusal, each its own [`GeologyError`] variant naming the row.
    pub fn from_wire(wire: &GeologyTableWire) -> Result<GeologyTable, GeologyError> {
        let mut province = [[0_u16; 4]; 4];
        for family in OreFamily::ALL {
            let row = wire
                .province
                .get(&family)
                .ok_or(GeologyError::MissingProvinceRow(family.id()))?;
            set_family(
                &mut province,
                family,
                row_of(row, format!("province {}", family.id()))?,
            );
        }
        let mut field = [[0_u16; 4]; 4];
        for (slot, word) in field.iter_mut().zip(Word::ALL) {
            let row = wire
                .field
                .get(&word)
                .ok_or(GeologyError::MissingFieldRow(word.name()))?;
            *slot = row_of(row, format!("field {}", word.name()))?;
        }
        // Decision 13, held by the table and not only by the shift guard: a province barren in a
        // family yields barren fields in it, so the `barren` field row weights barren alone.
        // The field pass's guard keeps a shift from lifting such a family; this keeps the draw
        // itself from doing so, for any table a lane authors (SM.2).
        if let Some(word) = Word::ALL
            .into_iter()
            .find(|word| *word != Word::Barren && by_word(by_word(field, Word::Barren), *word) > 0)
        {
            return Err(GeologyError::BarrenProvinceField(word.name()));
        }
        if wire.pocket.contains_key(&Word::Barren) {
            return Err(GeologyError::BarrenPocketRow);
        }
        let mut pocket = [[0_u16; 4]; 4];
        for (slot, word) in pocket.iter_mut().zip(Word::ALL) {
            if word == Word::Barren {
                continue;
            }
            let row = wire
                .pocket
                .get(&word)
                .ok_or(GeologyError::MissingPocketRow(word.name()))?;
            if row.contains_key(&Word::Barren) {
                return Err(GeologyError::BarrenPocketClass(word.name()));
            }
            *slot = row_of(row, format!("pocket {}", word.name()))?;
        }
        Ok(GeologyTable {
            province,
            field,
            pocket,
        })
    }

    /// The table back on the wire (zero weights omitted): `from_wire(&t.wire()) == t`.
    #[must_use]
    pub fn wire(&self) -> GeologyTableWire {
        GeologyTableWire {
            province: OreFamily::ALL
                .into_iter()
                .map(|family| (family, wire_of(by_family(self.province, family))))
                .collect(),
            field: Word::ALL
                .into_iter()
                .map(|word| (word, wire_of(by_word(self.field, word))))
                .collect(),
            pocket: Word::ALL
                .into_iter()
                .filter(|word| *word != Word::Barren)
                .map(|word| (word, wire_of(by_word(self.pocket, word))))
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// The body and its lattice
// ---------------------------------------------------------------------------

/// A body's ground: its id, its radius (the pocket lattice's extent) and its table. They reach
/// the world only through `crate::surface::install_body_geology`; since doc 20 cut SM.2 they are
/// sourced from the lane's `BodyDef` (`radius_km` and `geology`) by
/// `crate::surface::lane_geologies` and installed by `crate::surface::install_lane_geology`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BodyGeology {
    /// The body.
    pub body: CanonicalId,
    /// Its mean radius, km.
    pub radius_km: u32,
    /// Its geology table.
    pub table: GeologyTable,
}

/// The body-fixed pocket lattice: `columns` pockets around the equator (x wraps), `rows` from
/// pole to pole (`-rows/2 <= y < rows/2`). Both are whole fields; `rows` is whole fields on each
/// side of the equator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lattice {
    /// Pockets around the equator.
    pub columns: i64,
    /// Pockets from pole to pole.
    pub rows: i64,
}

impl Lattice {
    /// The lattice of a body of `radius_km`: the circumference `radius × 710/113` (2π, floored),
    /// whole 320 m fields around it, and half of that, whole fields each side of the equator.
    #[must_use]
    pub fn of_radius(radius_km: u32) -> Lattice {
        let circumference_m = u64::from(radius_km)
            .saturating_mul(1_000)
            .saturating_mul(710)
            / 113;
        let fields_around = circumference_m / 320;
        let columns = i64::try_from(fields_around.saturating_mul(8)).unwrap_or(0);
        let rows = (columns / 32).saturating_mul(16);
        Lattice { columns, rows }
    }

    /// Fields around the equator.
    #[must_use]
    pub fn field_columns(&self) -> i64 {
        self.columns / POCKETS_PER_FIELD_SIDE
    }

    /// The southernmost pocket row (inclusive).
    #[must_use]
    pub fn south(&self) -> i64 {
        -(self.rows / 2)
    }

    /// One past the northernmost pocket row.
    #[must_use]
    pub fn north(&self) -> i64 {
        self.rows / 2
    }

    /// A pocket x on the lattice: level 2's normalisation, by `columns`.
    fn norm_pocket(&self, x: i64) -> Result<i64, GeologyError> {
        x.checked_rem_euclid(self.columns)
            .ok_or(GeologyError::OffLattice)
    }

    /// A field x on the field lattice: levels 1 and 3's normalisation, by `columns / 8` (exact:
    /// `columns` is a multiple of 8 by construction).
    fn norm_field(&self, fx: i64) -> Result<i64, GeologyError> {
        fx.checked_rem_euclid(self.field_columns())
            .ok_or(GeologyError::OffLattice)
    }

    /// Whether a pocket row is on the lattice.
    fn pocket_row_on(&self, y: i64) -> bool {
        self.rows > 0 && (self.south()..self.north()).contains(&y)
    }

    /// The province of a pocket: `(x · 56 / columns, (y + rows/2) · 28 / rows)` — public
    /// since doc 20 cut SM.2, which reads a site's province from its centre pocket
    /// (`crate::port::site::site_province`).
    ///
    /// # Errors
    ///
    /// [`GeologyError::OffLattice`] past a pole.
    pub fn province_of_pocket(&self, x: i64, y: i64) -> Result<(u32, u32), GeologyError> {
        if !self.pocket_row_on(y) {
            return Err(GeologyError::OffLattice);
        }
        let x = self.norm_pocket(x)?;
        let column = x
            .checked_mul(i64::from(PROVINCE_COLUMNS))
            .and_then(|v| v.checked_div(self.columns))
            .and_then(|v| u32::try_from(v).ok())
            .ok_or(GeologyError::OffLattice)?;
        let row = y
            .checked_add(self.north())
            .and_then(|v| v.checked_mul(i64::from(PROVINCE_ROWS)))
            .and_then(|v| v.checked_div(self.rows))
            .and_then(|v| u32::try_from(v).ok())
            .ok_or(GeologyError::OffLattice)?;
        Ok((column, row))
    }
}

impl BodyGeology {
    /// A body's ground.
    ///
    /// # Errors
    ///
    /// [`GeologyError::Radius`] outside 1..=10,000 km.
    pub fn new(
        body: CanonicalId,
        radius_km: u32,
        table: GeologyTable,
    ) -> Result<BodyGeology, GeologyError> {
        if !(1..=10_000).contains(&radius_km) {
            return Err(GeologyError::Radius(radius_km));
        }
        Ok(BodyGeology {
            body,
            radius_km,
            table,
        })
    }

    /// The body's pocket lattice.
    #[must_use]
    pub fn lattice(&self) -> Lattice {
        Lattice::of_radius(self.radius_km)
    }
}

// ---------------------------------------------------------------------------
// The draw
// ---------------------------------------------------------------------------

/// One HMAC per (level, cell, family), read as four little-endian `u64`s:
/// `GEOLOGY_DOMAIN || body || 0x00 || level || x (i64 LE) || y (i64 LE) || family`.
pub(crate) fn draw(
    mac: &GeologyMac,
    body: &CanonicalId,
    level: u8,
    (x, y): (i64, i64),
    family: OreFamily,
) -> [u64; 4] {
    let body = body.as_str().as_bytes();
    let mut message = Vec::with_capacity(GEOLOGY_DOMAIN.len() + body.len() + 19);
    message.extend_from_slice(GEOLOGY_DOMAIN);
    message.extend_from_slice(body);
    message.push(0);
    message.push(level);
    message.extend_from_slice(&x.to_le_bytes());
    message.extend_from_slice(&y.to_le_bytes());
    message.push(family.code());
    let bytes = mac.draw(&message);
    let mut out = [0_u64; 4];
    // `as_chunks` (clippy's form of `chunks_exact(8)`): four whole 8-byte words, no indexing.
    let (words, _) = bytes.as_chunks::<8>();
    for (slot, word) in out.iter_mut().zip(words) {
        *slot = u64::from_le_bytes(*word);
    }
    out
}

/// A per-mille pick over `d % 1000`, cumulatively in word order. A checked row sums to 1,000,
/// so the loop always answers; the barren fallback is for an all-zero row (the pocket table's
/// barren row, which is never read).
fn pick(row: [u16; 4], d: u64) -> Word {
    let roll = d % 1_000;
    let mut acc = 0_u64;
    for (word, weight) in Word::ALL.into_iter().zip(row) {
        acc = acc.saturating_add(u64::from(weight));
        if roll < acc {
            return word;
        }
    }
    Word::Barren
}

/// A uniform value in `lo..=hi` from a 64-bit draw (`lo + d % (hi − lo + 1)`; the modulo bias
/// of a 64-bit draw over a span of at most 25 is accepted and recorded, SM.1 brief §3.4).
fn uniform((lo, hi): (u8, u8), d: u64) -> u8 {
    let span = u64::from(hi.saturating_sub(lo)).saturating_add(1);
    u8::try_from(d % span).map_or(hi, |offset| lo.saturating_add(offset))
}

// ---------------------------------------------------------------------------
// The passes
// ---------------------------------------------------------------------------

/// A province's survey words (R0), one per family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvinceGround {
    /// The province's (column, row) on the 56 × 28 chart.
    pub province: (u32, u32),
    /// Its word per family.
    pub words: [Word; 4],
}

/// One pocket's class and tonnes per family; barren ⇔ tonnes 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PocketRow {
    /// The class per family.
    pub classes: [Word; 4],
    /// The tonnes per family.
    pub tonnes: [u8; 4],
}

impl PocketRow {
    /// A pocket with no ore.
    pub const BARREN: PocketRow = PocketRow {
        classes: [Word::Barren; 4],
        tonnes: [0; 4],
    };

    /// How many families the pocket carries.
    #[must_use]
    pub fn families(&self) -> u8 {
        let carried = self.classes.iter().filter(|c| **c != Word::Barren).count();
        u8::try_from(carried).unwrap_or(u8::MAX)
    }
}

/// A field's ground: its province, the shift it was drawn under, its word and ore-pocket count
/// per family, and its 64 pockets (index `ly * 8 + lx` from its south-west).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldGround {
    /// The field on the field lattice, x normalised.
    pub field: (i64, i64),
    /// Its province.
    pub province: ProvinceGround,
    /// The class shift it was drawn under.
    pub shift: i8,
    /// Its word per family.
    pub words: [Word; 4],
    /// Its ore-pocket count per family (`Word::of_count(counts[f]) == Some(words[f])`).
    pub counts: [u8; 4],
    /// Its 64 pockets.
    pub pockets: Vec<PocketRow>,
}

impl FieldGround {
    /// The pocket at `(lx, ly)` inside the field, 0..8 each.
    #[must_use]
    pub fn pocket(&self, lx: i64, ly: i64) -> Option<PocketRow> {
        if !(0..POCKETS_PER_FIELD_SIDE).contains(&lx) || !(0..POCKETS_PER_FIELD_SIDE).contains(&ly)
        {
            return None;
        }
        let index = usize::try_from(ly * POCKETS_PER_FIELD_SIDE + lx).ok()?;
        self.pockets.get(index).copied()
    }
}

/// R0's province: per family a word from the table's province row.
///
/// # Errors
///
/// [`GeologyError::OffLattice`] for a province off the 56 × 28 chart.
pub fn province_at(
    mac: &GeologyMac,
    body: &BodyGeology,
    province: (u32, u32),
) -> Result<ProvinceGround, GeologyError> {
    let mut words = [Word::Barren; 4];
    for (slot, family) in words.iter_mut().zip(OreFamily::ALL) {
        *slot = province_word(mac, body, province, family)?;
    }
    Ok(ProvinceGround { province, words })
}

/// **One family's R0 word of a province** (doc 20 cut SM.9; §3.1's ground chart): the one HMAC
/// [`province_at`] draws for that family, so the body's wash in one family costs 1,568 draws a
/// frame (56 × 28), not four times that. [`province_at`] is the four of these, so the two can
/// never disagree (reading rule v1's literal pin holds it).
///
/// # Errors
///
/// [`GeologyError::OffLattice`] for a province off the 56 × 28 chart.
pub fn province_word(
    mac: &GeologyMac,
    body: &BodyGeology,
    province: (u32, u32),
    family: OreFamily,
) -> Result<Word, GeologyError> {
    if province.0 >= PROVINCE_COLUMNS || province.1 >= PROVINCE_ROWS {
        return Err(GeologyError::OffLattice);
    }
    let cell = (i64::from(province.0), i64::from(province.1));
    let [d0, ..] = draw(mac, &body.body, LEVEL_PROVINCE, cell, family);
    Ok(pick(by_family(body.table.province, family), d0))
}

/// The ground of one field under a class `shift` (§3.1's block shift; 0 for the body's own
/// ground): its province, its word and count per family, and its 64 pockets.
///
/// # Errors
///
/// [`GeologyError::Shift`] outside -3..=3; [`GeologyError::OffLattice`] past a pole.
pub fn field_ground(
    mac: &GeologyMac,
    body: &BodyGeology,
    field: (i64, i64),
    shift: i8,
) -> Result<FieldGround, GeologyError> {
    if !(-MAX_SHIFT..=MAX_SHIFT).contains(&shift) {
        return Err(GeologyError::Shift(shift));
    }
    let lattice = body.lattice();
    let fx = lattice.norm_field(field.0)?;
    let fy = field.1;
    let origin_x = fx
        .checked_mul(POCKETS_PER_FIELD_SIDE)
        .ok_or(GeologyError::OffLattice)?;
    let origin_y = fy
        .checked_mul(POCKETS_PER_FIELD_SIDE)
        .ok_or(GeologyError::OffLattice)?;
    // A field is on the lattice when its south-west pocket is: rows is whole fields each side.
    let province = province_at(mac, body, lattice.province_of_pocket(origin_x, origin_y)?)?;

    let mut words = [Word::Barren; 4];
    let mut counts = [0_u8; 4];
    for family in OreFamily::ALL {
        let [d0, d1, ..] = draw(mac, &body.body, LEVEL_FIELD, (fx, fy), family);
        let province_word = by_family(province.words, family);
        let drawn = pick(by_word(body.table.field, province_word), d0);
        // The SM.1 brief's decision 13: a block shift never lifts a family its province says is
        // barren (Ceres's strategic elements, Vesta's carbon — doc 20 §3.1, §3.2, §11); a
        // negative shift still lowers any family. The one place the guard lives.
        let word = if province_word == Word::Barren {
            drawn
        } else {
            drawn.shifted(shift)
        };
        let count = match word {
            Word::Barren => 0,
            _ => uniform(word.pocket_range(), d1),
        };
        set_family(&mut words, family, word);
        set_family(&mut counts, family, count);
    }

    // The pockets: per family, rank the 64 by a keyed draw and give the family to the first
    // `count` that carry fewer than two (total demand ≤ 64 against a capacity of 128, so every
    // count is placed). A selected pocket's class is drawn from the field word's pocket row
    // (never barren), its tonnes uniform over the class's range.
    let mut pockets = vec![PocketRow::BARREN; POCKETS_PER_FIELD];
    for family in OreFamily::ALL {
        let count = usize::from(by_family(counts, family));
        if count == 0 {
            continue;
        }
        let mut ranked: Vec<(u64, usize, u64, u64)> = Vec::with_capacity(POCKETS_PER_FIELD);
        for ly in 0..POCKETS_PER_FIELD_SIDE {
            for lx in 0..POCKETS_PER_FIELD_SIDE {
                let px = origin_x.saturating_add(lx);
                let py = origin_y.saturating_add(ly);
                let index = usize::try_from(ly * POCKETS_PER_FIELD_SIDE + lx).unwrap_or(0);
                let [rank, class_draw, tonnes_draw, _] = draw(
                    mac,
                    &body.body,
                    LEVEL_POCKET,
                    (lattice.norm_pocket(px)?, py),
                    family,
                );
                ranked.push((rank, index, class_draw, tonnes_draw));
            }
        }
        ranked.sort_unstable_by_key(|(rank, index, ..)| (*rank, *index));
        let word = by_family(words, family);
        let mut placed = 0_usize;
        for (_, index, class_draw, tonnes_draw) in ranked {
            if placed >= count {
                break;
            }
            let Some(row) = pockets.get_mut(index) else {
                continue;
            };
            if row.families() >= MAX_FAMILIES_PER_POCKET {
                continue;
            }
            let class = pick(by_word(body.table.pocket, word), class_draw);
            set_family(&mut row.classes, family, class);
            set_family(
                &mut row.tonnes,
                family,
                uniform(class.tonnes_range(), tonnes_draw),
            );
            placed = placed.saturating_add(1);
        }
    }

    Ok(FieldGround {
        field: (fx, fy),
        province,
        shift,
        words,
        counts,
        pockets,
    })
}

/// The pocket at a body-lattice point and the field it lies in, on the body's own ground
/// (shift 0).
///
/// # Errors
///
/// [`GeologyError::OffLattice`] past a pole.
pub fn pocket_at(
    mac: &GeologyMac,
    body: &BodyGeology,
    pocket: (i64, i64),
) -> Result<(FieldGround, PocketRow), GeologyError> {
    let field = (
        pocket.0.div_euclid(POCKETS_PER_FIELD_SIDE),
        pocket.1.div_euclid(POCKETS_PER_FIELD_SIDE),
    );
    let ground = field_ground(mac, body, field, 0)?;
    let row = ground
        .pocket(
            pocket.0.rem_euclid(POCKETS_PER_FIELD_SIDE),
            pocket.1.rem_euclid(POCKETS_PER_FIELD_SIDE),
        )
        .ok_or(GeologyError::OffLattice)?;
    Ok((ground, row))
}

/// A pocket x normalised onto the body's lattice (what `@geology` prints).
///
/// # Errors
///
/// [`GeologyError::OffLattice`] for an empty lattice.
pub fn normalise_pocket(
    body: &BodyGeology,
    pocket: (i64, i64),
) -> Result<(i64, i64), GeologyError> {
    let lattice = body.lattice();
    if !lattice.pocket_row_on(pocket.1) {
        return Err(GeologyError::OffLattice);
    }
    Ok((lattice.norm_pocket(pocket.0)?, pocket.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_and_their_ranges_invert() {
        for count in 0..=16_u8 {
            let word = Word::of_count(count).expect("a word");
            let (lo, hi) = word.pocket_range();
            assert!((lo..=hi).contains(&count));
        }
        assert_eq!(Word::of_count(17), None);
        assert_eq!(Word::Lean.shifted(3), Word::Rich);
        assert_eq!(Word::Lean.shifted(-3), Word::Barren);
        assert_eq!(Word::Fair.shifted(0), Word::Fair);
        assert_eq!(Word::Barren.shifted(1), Word::Lean);
        assert_eq!(uniform((3, 6), 0), 3);
        assert_eq!(uniform((3, 6), 3), 6);
        assert_eq!(uniform((3, 6), 4), 3);
        assert_eq!(pick([0, 600, 400, 0], 599), Word::Lean);
        assert_eq!(pick([0, 600, 400, 0], 600), Word::Fair);
        assert_eq!(pick([0, 600, 400, 0], 1_999), Word::Fair);
    }
}
