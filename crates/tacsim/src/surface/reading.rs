//! **The reading rule, version 1** (doc 20 cut SM.1; §5, §11's Instruments row).
//!
//! What an instrument may say about the ground, as pure quantisations of the keyed geology
//! (`crate::surface::geology`): never a draw of its own beyond the one keyed band side below,
//! never an RNG (D-20-4), and never a word the truth does not support.
//!
//! - **R0**, the Registry's survey: the province's words, the same for everyone;
//! - **R1**, a scan, sharpened by the footing it was taken from: on **rock** each family's field
//!   word exactly; on **regolith** a two-word band that never excludes the truth; on **ice** no
//!   return. The band's side is one keyed draw per (field, family), so repeating a scan never
//!   narrows it (SM.4a's "You learn nothing new here.") and its position does not tell the truth,
//!   the barren and rich edges excepted;
//! - **R2**, a sounding or a core: the pocket's class per family.
//!
//! [`READING_RULE_VERSION`] is what SM.4's chart rows pin, so a later rule never silently
//! re-reads an old row.

use mush_atlas::TerrainClass;
use mush_core::GeologyMac;
use serde::{Deserialize, Serialize};

use super::geology::{
    BodyGeology, FieldGround, LEVEL_READING, OreFamily, PocketRow, ProvinceGround, Word, by_family,
    draw,
};

/// The reading rule's version.
pub const READING_RULE_VERSION: u16 = 1;

/// What a scan stands on (§5): the terrain class under the vehicle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Footing {
    /// Bare rock: an exact field word.
    Rock,
    /// Regolith: a two-word band.
    Regolith,
    /// Ice: no return.
    Ice,
}

impl Footing {
    /// The footing a terrain class gives, `None` for a class no scan is taken from.
    #[must_use]
    pub fn of(class: TerrainClass) -> Option<Footing> {
        match class {
            TerrainClass::Rock => Some(Footing::Rock),
            TerrainClass::Regolith => Some(Footing::Regolith),
            TerrainClass::Ice => Some(Footing::Ice),
            TerrainClass::Void
            | TerrainClass::Paved
            | TerrainClass::Ground
            | TerrainClass::Grass
            | TerrainClass::Water
            | TerrainClass::Floor
            | TerrainClass::Rubble
            | TerrainClass::Road
            | TerrainClass::Sand
            | TerrainClass::Wall
            | TerrainClass::Door
            | TerrainClass::Platform
            | TerrainClass::WoodsLight
            | TerrainClass::WoodsHeavy
            | TerrainClass::WaterDeep => None,
        }
    }

    /// The footing as a player reads it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Footing::Rock => "rock",
            Footing::Regolith => "regolith",
            Footing::Ice => "ice",
        }
    }

    /// **How sharp a scan from this footing reads** (doc 20 cut SM.4a; §5 "footing sets
    /// sharpness, never range"): ice 0 (no return), regolith 1 (a band), rock 2 (an exact
    /// word). The one place the order lives: a chart row keeps its sharpest footing, and the
    /// preview's better footing is a sharper one.
    #[must_use]
    pub fn sharpness(self) -> u8 {
        match self {
            Footing::Ice => 0,
            Footing::Regolith => 1,
            Footing::Rock => 2,
        }
    }
}

/// One family's R1 reading of a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldReading {
    /// The field word itself (rock).
    Exact(Word),
    /// Two adjacent words, low then high, the truth one of them (regolith).
    Band(Word, Word),
    /// Nothing came back (ice).
    NoReturn,
}

impl FieldReading {
    /// The reading in words: `fair`, `fair–rich` or `no return`.
    #[must_use]
    pub fn words(&self) -> String {
        match self {
            FieldReading::Exact(word) => word.name().to_owned(),
            FieldReading::Band(low, high) => format!("{}–{}", low.name(), high.name()),
            FieldReading::NoReturn => "no return".to_owned(),
        }
    }

    /// The ore-pocket counts the reading admits: the union of its words' pocket ranges.
    #[must_use]
    pub fn pockets(&self) -> Option<(u8, u8)> {
        match self {
            FieldReading::Exact(word) => Some(word.pocket_range()),
            FieldReading::Band(low, high) => Some((low.pocket_range().0, high.pocket_range().1)),
            FieldReading::NoReturn => None,
        }
    }
}

/// R0, the Registry's survey: the province's words, the same for everyone.
#[must_use]
pub fn read_r0(province: &ProvinceGround) -> [Word; 4] {
    province.words
}

/// R1, a scan: rock reads each family's field word exactly; regolith a two-word band that never
/// excludes the truth; ice no return.
#[must_use]
pub fn read_r1(
    mac: &GeologyMac,
    body: &BodyGeology,
    field: &FieldGround,
    footing: Footing,
) -> [FieldReading; 4] {
    OreFamily::ALL.map(|family| {
        let truth = by_family(field.words, family);
        match footing {
            Footing::Rock => FieldReading::Exact(truth),
            Footing::Ice => FieldReading::NoReturn,
            Footing::Regolith => regolith_band(mac, body, field, family, truth),
        }
    })
}

/// The regolith band: barren → barren–lean, rich → fair–rich; otherwise one keyed draw at level
/// 3 over (field, family) picks the word below on an even draw and the word above on an odd one.
/// `field.field` is already normalised (`field_ground` stores it so), so the seam is invisible.
fn regolith_band(
    mac: &GeologyMac,
    body: &BodyGeology,
    field: &FieldGround,
    family: OreFamily,
    truth: Word,
) -> FieldReading {
    match truth {
        Word::Barren => FieldReading::Band(Word::Barren, Word::Lean),
        Word::Rich => FieldReading::Band(Word::Fair, Word::Rich),
        Word::Lean | Word::Fair => {
            let [d0, ..] = draw(mac, &body.body, LEVEL_READING, field.field, family);
            if d0 % 2 == 0 {
                FieldReading::Band(truth.shifted(-1), truth)
            } else {
                FieldReading::Band(truth, truth.shifted(1))
            }
        }
    }
}

/// R2, a sounding or a core: the pocket's class per family.
#[must_use]
pub fn read_r2(pocket: &PocketRow) -> [Word; 4] {
    pocket.classes
}
