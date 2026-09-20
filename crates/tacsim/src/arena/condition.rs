//! **What a gunner may read off a target's hull** (16b cut Q.2; doc 16 §2 W5,
//! §3.3 "Sections, facets and struck-through", §3.6 "What a pilot sees").
//!
//! The owner's first hand-played match asked for a `SCAN`: *the enemy status —
//! not exact numbers but green/yellow/red*. That sentence is the whole of this
//! module. A [`Condition`] is one two-bit band per section of a unit, in the
//! unit's own section order, and a [`Reading`] is one of those with the arena
//! timestamp it was taken at.
//!
//! # Why it is a band and never a number
//!
//! Doc 16 §5 gives a pilot *armor and internals per section as current over max*
//! for their **own** tank, which is physical knowledge — a crew reads its own
//! gauges. What a sight can take off somebody else's tank is coarser than that by
//! a long way, and doc 16 §3.6's rule is that a frame carries what the platform
//! *knows*. Four words is what a look through an optic is honestly worth, so four
//! words is what is stored: there is no exact figure anywhere in this type for a
//! later surface to leak.
//!
//! # Where the floors come from
//!
//! `[tacsim.sensors].condition_green_permille` and `condition_yellow_permille`,
//! passed in rather than read here, because this module has no configuration and
//! [`crate::arena::state::Arena`] has exactly one place that recomputes a cached
//! band ([`crate::arena::state::Arena::set_unit`]). They are configuration and not
//! constants so the owner can tune them on a box with a server-child restart and
//! no build, and not content because shipped content is compiled into the binary
//! and an operator pack cannot override a shipped id.
//!
//! # Sixteen
//!
//! [`MAX_SCANNED_SECTIONS`] is a **defensive clamp** and not the rule. The rule is
//! validation: `crate::definitions::unit::UnitDef::check` refuses a unit spec that
//! declares more sections than this, so no unit an arena can ever hold reaches the
//! clamp. Sixteen sections at two bits each is exactly the `u32` the band field
//! is, which is why the packing needs no allocation and why a contact's reading
//! costs thirteen bytes.

use mush_tactics::TacticalUnit;
use serde::{Deserialize, Serialize};

use crate::definitions::unit::{ARMOR_CELL, INTACT, INTERNALS_CELL};

/// The most sections one [`Condition`] carries — see the module head.
pub const MAX_SCANNED_SECTIONS: usize = 16;

/// How many bits one section's band occupies in [`Condition::bands`].
const BITS_PER_SECTION: usize = 2;

/// The mask of one band.
const BAND_MASK: u32 = 0b11;

/// Per mille of a whole section.
const FULL_PERMILLE: u64 = 1_000;

/// **How one section of a unit reads to somebody looking at it** (16b cut Q.2).
///
/// Four words, in the order a section gets worse, so the discriminants are also
/// the wire encoding: two bits, packed into [`Condition::bands`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionBand {
    /// At or above `condition_green_permille` of what it holds whole.
    Green = 0,
    /// At or above `condition_yellow_permille`.
    Yellow = 1,
    /// Below both, and still there.
    Red = 2,
    /// Its M32 condition is not `intact`, or its internals are at their floor.
    Destroyed = 3,
}

impl ConditionBand {
    /// The word a pilot reads, which is also the wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ConditionBand::Green => "green",
            ConditionBand::Yellow => "yellow",
            ConditionBand::Red => "red",
            ConditionBand::Destroyed => "destroyed",
        }
    }

    /// The one letter [`Condition::letters`] packs it as — the compact spelling a
    /// panel and a fixture read, so a five-section tank is `ggyrd`.
    #[must_use]
    pub const fn letter(&self) -> char {
        match self {
            ConditionBand::Green => 'g',
            ConditionBand::Yellow => 'y',
            ConditionBand::Red => 'r',
            ConditionBand::Destroyed => 'd',
        }
    }

    /// The two bits this band packs as.
    const fn code(self) -> u32 {
        self as u32
    }

    /// The band two bits unpack to. Every one of the four values is a band, so
    /// there is nothing here to refuse.
    const fn from_code(code: u32) -> ConditionBand {
        match code & BAND_MASK {
            0 => ConditionBand::Green,
            1 => ConditionBand::Yellow,
            2 => ConditionBand::Red,
            _ => ConditionBand::Destroyed,
        }
    }
}

/// **One unit's per-section bands**, in the unit's own section order (16b cut
/// Q.2) — the order [`mush_tactics::TacticalUnit::sections`] answers in, which is
/// section-id order and is the order `STATUS`'s damage block already walks.
///
/// Two integers and no allocation: this rides every classified contact inside the
/// close-look reach, and a `Vec` per contact per pass is the one thing doc 16 §2
/// W4's budget could not absorb. `sections` is how many of the sixteen slots are
/// meaningful, so [`Condition::band`] can answer `None` for a slot that is not a
/// section rather than `Green` for one that does not exist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    /// Two bits a section, lowest section first.
    pub bands: u32,
    /// How many sections are packed, at most [`MAX_SCANNED_SECTIONS`].
    pub sections: u8,
}

/// **One sighting's worth of that**, with the arena timestamp it was taken at
/// (16b cut Q.2).
///
/// The stamp is the reading's own and **not** the contact's `last_seen_ms`: a
/// plot refreshes on every pass a platform holds the target, and a reading only
/// on the passes it held the target classified and close. So `SCAN` says how old
/// the *look* is, which is a different and older number than how old the track
/// is, and a pilot who has driven out to 900 m is told that what they are reading
/// is twenty seconds stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reading {
    /// Arena milliseconds at the look.
    pub at_ms: i64,
    /// What it read.
    pub condition: Condition,
}

impl Condition {
    /// **Read a unit's bands off its M32 cells** (16b cut Q.2), section by
    /// section in the unit's own order.
    ///
    /// A section is [`ConditionBand::Destroyed`] when its M32 condition is not
    /// `intact` — which is the same opaque condition `STATUS` prints and
    /// `crate::arena::damage` sets — or when its internals cell is at its floor.
    /// Otherwise its fraction is
    /// `(its facets' armour left + its internals left) × 1000 / (their maxima)`,
    /// taken against the two floors: a facet-less section (doc 16 §3.3's engine,
    /// struck through the hull's rear) is its internals alone, and a section whose
    /// cells hold nothing at all reads `Green` rather than dividing by zero.
    ///
    /// Integer arithmetic throughout (D-16-4). It reads at most
    /// [`MAX_SCANNED_SECTIONS`] sections; see the module head for why that is a
    /// clamp and not the rule.
    #[must_use]
    pub fn of(unit: &TacticalUnit, green_permille: u32, yellow_permille: u32) -> Condition {
        let armor = mush_tactics::CellId::new(ARMOR_CELL).ok();
        let internals = mush_tactics::CellId::new(INTERNALS_CELL).ok();
        let mut out = Condition::default();
        for (index, (id, section)) in unit.sections().enumerate().take(MAX_SCANNED_SECTIONS) {
            let inside = internals.as_ref().and_then(|cell| {
                unit.cell(&mush_tactics::CellAddr::Section {
                    section: id.clone(),
                    cell: cell.clone(),
                })
            });
            let band = if section.condition.as_str() != INTACT
                || inside.is_some_and(|cell| cell.current <= cell.min)
            {
                ConditionBand::Destroyed
            } else {
                let mut left: u64 = 0;
                let mut whole: u64 = 0;
                if let Some(cell) = inside {
                    left = left.saturating_add(cell.current.max(0).unsigned_abs());
                    whole = whole.saturating_add(cell.max.max(0).unsigned_abs());
                }
                for facet in &section.facets {
                    let Some(cell) = armor.as_ref().and_then(|cell| {
                        unit.cell(&mush_tactics::CellAddr::Facet {
                            section: id.clone(),
                            facet: facet.clone(),
                            cell: cell.clone(),
                        })
                    }) else {
                        continue;
                    };
                    left = left.saturating_add(cell.current.max(0).unsigned_abs());
                    whole = whole.saturating_add(cell.max.max(0).unsigned_abs());
                }
                band_of(left, whole, green_permille, yellow_permille)
            };
            out = out.with_band(index, band);
        }
        out
    }

    /// The band of the section at `index` in the unit's section order, or `None`
    /// for a slot beyond [`Condition::sections`].
    #[must_use]
    pub const fn band(&self, index: usize) -> Option<ConditionBand> {
        if index >= self.sections as usize || index >= MAX_SCANNED_SECTIONS {
            return None;
        }
        Some(ConditionBand::from_code(
            self.bands >> (index * BITS_PER_SECTION),
        ))
    }

    /// The same condition with one section's band replaced, widening the packed
    /// length when the index is past it.
    ///
    /// An index past [`MAX_SCANNED_SECTIONS`] is answered unchanged rather than
    /// refused: the packing is a clamp and nothing in the arena can produce one
    /// (see the module head).
    #[must_use]
    pub const fn with_band(self, index: usize, band: ConditionBand) -> Condition {
        if index >= MAX_SCANNED_SECTIONS {
            return self;
        }
        let shift = index * BITS_PER_SECTION;
        let bands = (self.bands & !(BAND_MASK << shift)) | (band.code() << shift);
        let wanted = (index + 1) as u8;
        let sections = if wanted > self.sections {
            wanted
        } else {
            self.sections
        };
        Condition { bands, sections }
    }

    /// **This condition, with every section `already` reads as destroyed still
    /// destroyed** (16b cut Q.2).
    ///
    /// A section does not come back. The bands are otherwise a *snapshot* and the
    /// newer one wins outright — but a reading a platform holds may carry one
    /// fact its sensors never took: the section its own gunner watched come apart
    /// ([`crate::arena::state::Arena::observe_section_lost`]), written in with the
    /// **old look's** stamp because it is not a look. A teammate's share taken
    /// between that look and the fire is therefore *stamped* newer while knowing
    /// strictly less, and adopting it whole would make `SCAN` contradict the line
    /// the pilot was just sent. Destruction is the one monotone thing a reading
    /// carries, so it is the one thing carried across.
    ///
    /// Sections past either condition's packed length are left alone: two
    /// readings of one contact are always of one machine, so the lengths agree.
    #[must_use]
    pub const fn keeping_destroyed(self, already: Condition) -> Condition {
        let mut out = self;
        let mut index = 0;
        while index < MAX_SCANNED_SECTIONS {
            if matches!(already.band(index), Some(ConditionBand::Destroyed))
                && out.band(index).is_some()
            {
                out = out.with_band(index, ConditionBand::Destroyed);
            }
            index += 1;
        }
        out
    }

    /// The compact spelling: one [`ConditionBand::letter`] per packed section, in
    /// the same order. A unit with nothing packed is the empty string.
    #[must_use]
    pub fn letters(&self) -> String {
        (0..usize::from(self.sections))
            .filter_map(|index| self.band(index))
            .map(|band| band.letter())
            .collect()
    }
}

/// The band a fraction falls in, against the two floors.
///
/// `whole` of zero is [`ConditionBand::Green`]: a section that holds nothing when
/// it is whole has lost nothing, and the alternative is a division by zero. The
/// floors are read `.min(1000)` by every caller, so a misconfigured key can make
/// everything `Red` and can never make the arithmetic overflow.
fn band_of(left: u64, whole: u64, green_permille: u32, yellow_permille: u32) -> ConditionBand {
    if whole == 0 {
        return ConditionBand::Green;
    }
    let permille = left.saturating_mul(FULL_PERMILLE) / whole;
    if permille >= u64::from(green_permille) {
        ConditionBand::Green
    } else if permille >= u64::from(yellow_permille) {
        ConditionBand::Yellow
    } else {
        ConditionBand::Red
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::test_support::{line_tank_unit, tactical_unit};
    use crate::config::SensorsConfig;
    use mush_core::ObjId;
    use mush_tactics::{CellAddr, CellId, ConditionId, FacetId, SectionId, UnitMutationOp};

    const UNIT: ObjId = ObjId(4);

    /// The shipped floors, which every test here reads rather than re-typing.
    fn floors() -> (u32, u32) {
        let cfg = SensorsConfig::default();
        (cfg.condition_green_permille, cfg.condition_yellow_permille)
    }

    /// One plan against a fixture unit, applied — the same helper
    /// `crate::arena::damage`'s tests use, so a band is always read off a unit a
    /// real plan produced.
    fn apply(unit: &TacticalUnit, ops: Vec<UnitMutationOp>) -> TacticalUnit {
        let plan = mush_tactics::UnitMutationPlan::new(ops, None, None)
            .unwrap_or_else(|_| panic!("the fixture plan fits"));
        let (post, _) = mush_tactics::apply(unit, &plan, 0)
            .unwrap_or_else(|error| panic!("the fixture plan applies: {error}"));
        post
    }

    fn armor(section: &str, facet: &str) -> CellAddr {
        CellAddr::Facet {
            section: SectionId::new(section).expect("a section"),
            facet: FacetId::new(facet).expect("a facet"),
            cell: CellId::new(ARMOR_CELL).expect("a cell"),
        }
    }

    fn internals(section: &str) -> CellAddr {
        CellAddr::Section {
            section: SectionId::new(section).expect("a section"),
            cell: CellId::new(INTERNALS_CELL).expect("a cell"),
        }
    }

    /// The stock tank's five sections, in the order `TacticalUnit::sections`
    /// answers in — which is the order every band index in this file is against.
    const STOCK_SECTIONS: [&str; 5] = ["engine", "hull", "left_track", "right_track", "turret"];

    #[test]
    fn a_whole_tank_is_green_section_by_section() {
        let (green, yellow) = floors();
        let condition = Condition::of(&line_tank_unit(UNIT), green, yellow);
        assert_eq!(condition.sections, 5, "the stock tank's five sections");
        assert_eq!(condition.letters(), "ggggg");
        for index in 0..STOCK_SECTIONS.len() {
            assert_eq!(condition.band(index), Some(ConditionBand::Green));
        }
        assert_eq!(condition.band(5), None, "there is no sixth section");
    }

    #[test]
    fn the_two_floors_are_where_a_band_changes() {
        // The engine has no facets at all (doc 16 §3.3: it is struck through the
        // hull's rear), so its fraction is its internals alone — which makes it
        // the one section whose band can be driven to an exact per mille.
        let whole = line_tank_unit(UNIT);
        let max = whole.cell(&internals("engine")).map_or(0, |cell| cell.max);
        assert_eq!(max, 120, "the fixture engine holds 120 internals");
        // 667, 334 of 1,000: the first value at or above each floor, and the
        // first below it.
        for (value, expected) in [
            (120_i64, ConditionBand::Green),
            (81, ConditionBand::Green),
            (80, ConditionBand::Yellow),
            (41, ConditionBand::Yellow),
            (40, ConditionBand::Red),
            (1, ConditionBand::Red),
        ] {
            let (green, yellow) = floors();
            let hurt = apply(
                &whole,
                vec![UnitMutationOp::SetCell {
                    addr: internals("engine"),
                    value,
                }],
            );
            assert_eq!(
                Condition::of(&hurt, green, yellow).band(0),
                Some(expected),
                "{value} of {max} internals"
            );
        }
    }

    #[test]
    fn an_empty_section_is_destroyed_and_so_is_a_condition_that_is_not_intact() {
        let (green, yellow) = floors();
        let whole = line_tank_unit(UNIT);
        // Internals at the floor: destroyed, whatever the armour still says.
        let spent = apply(
            &whole,
            vec![UnitMutationOp::SetCell {
                addr: internals("turret"),
                value: 0,
            }],
        );
        assert_eq!(
            Condition::of(&spent, green, yellow).band(4),
            Some(ConditionBand::Destroyed),
            "the turret's internals are gone"
        );
        // And the condition alone, with every cell untouched.
        let marked = apply(
            &whole,
            vec![UnitMutationOp::SetSectionCondition {
                section: SectionId::new("turret").expect("a section"),
                condition: ConditionId::new("destroyed").expect("a condition"),
            }],
        );
        assert_eq!(
            Condition::of(&marked, green, yellow).band(4),
            Some(ConditionBand::Destroyed),
            "a section that is not intact reads destroyed"
        );
    }

    #[test]
    fn a_wreck_reads_destroyed_where_the_wreck_was_marked() {
        let (green, yellow) = floors();
        let wreck = apply(
            &line_tank_unit(UNIT),
            vec![UnitMutationOp::SetSectionCondition {
                section: SectionId::new(crate::arena::damage::HULL_SECTION).expect("a section"),
                condition: ConditionId::new(crate::arena::damage::WRECK_CONDITION)
                    .expect("a condition"),
            }],
        );
        let condition = Condition::of(&wreck, green, yellow);
        assert_eq!(
            condition.band(1),
            Some(ConditionBand::Destroyed),
            "the hull"
        );
        assert_eq!(condition.letters(), "gdggg");
    }

    #[test]
    fn armour_and_internals_are_one_fraction() {
        let (green, yellow) = floors();
        // The left track: one facet of 120 armour and 100 internals, 220 whole.
        // Strip the armour and the section is 100 of 220 — 454 per mille, which
        // is yellow and not red, because the internals are still whole.
        let stripped = apply(
            &line_tank_unit(UNIT),
            vec![UnitMutationOp::SetCell {
                addr: armor("left_track", "left"),
                value: 0,
            }],
        );
        assert_eq!(
            Condition::of(&stripped, green, yellow).band(2),
            Some(ConditionBand::Yellow),
            "100 of 220 is 454 per mille"
        );
    }

    #[test]
    fn a_one_section_unit_packs_one_band() {
        let (green, yellow) = floors();
        let condition = Condition::of(&tactical_unit(UNIT), green, yellow);
        assert_eq!(condition.sections, 1);
        assert_eq!(condition.letters(), "g");
        assert_eq!(condition.band(0), Some(ConditionBand::Green));
        assert_eq!(condition.band(1), None);
    }

    #[test]
    fn sixteen_sections_pack_and_read_back() {
        let every = [
            ConditionBand::Green,
            ConditionBand::Yellow,
            ConditionBand::Red,
            ConditionBand::Destroyed,
        ];
        let mut condition = Condition::default();
        for index in 0..MAX_SCANNED_SECTIONS {
            condition = condition.with_band(index, every[index % every.len()]);
        }
        assert_eq!(condition.sections, 16);
        assert_eq!(condition.letters(), "gyrdgyrdgyrdgyrd");
        for index in 0..MAX_SCANNED_SECTIONS {
            assert_eq!(condition.band(index), Some(every[index % every.len()]));
        }
        // The seventeenth is not a section and cannot be made one.
        let past = condition.with_band(MAX_SCANNED_SECTIONS, ConditionBand::Red);
        assert_eq!(past, condition, "the clamp holds");
        assert_eq!(condition.band(MAX_SCANNED_SECTIONS), None);
    }

    #[test]
    fn with_band_replaces_one_section_and_leaves_the_rest() {
        let (green, yellow) = floors();
        let whole = Condition::of(&line_tank_unit(UNIT), green, yellow);
        let marked = whole.with_band(4, ConditionBand::Destroyed);
        assert_eq!(marked.letters(), "ggggd");
        assert_eq!(marked.sections, whole.sections);
        // Replacing it again is not additive: the two bits are cleared first.
        assert_eq!(
            marked.with_band(4, ConditionBand::Yellow).letters(),
            "ggggy"
        );
    }

    #[test]
    fn a_newer_reading_keeps_the_sections_the_older_one_knew_were_gone() {
        let (green, yellow) = floors();
        let whole = Condition::of(&line_tank_unit(UNIT), green, yellow);
        // What the platform holds: its own look, with the turret written in by
        // the gunner who watched it come apart.
        let watched = whole.with_band(4, ConditionBand::Destroyed);
        // What a teammate shares: a look taken *before* the fire, so it is
        // stamped newer and knows strictly less.
        assert_eq!(whole.keeping_destroyed(watched).letters(), "ggggd");
        // Everything else the newer reading says wins outright — a section that
        // has only got worse is still the newer word for it.
        let hurt = whole
            .with_band(1, ConditionBand::Yellow)
            .with_band(2, ConditionBand::Red);
        assert_eq!(hurt.keeping_destroyed(watched).letters(), "gyrgd");
        // And destruction is the only thing carried: an older `Red` does not
        // survive a newer `Green`.
        let stale = whole.with_band(0, ConditionBand::Red);
        assert_eq!(whole.keeping_destroyed(stale).letters(), "ggggg");
        // A section the newer reading does not pack is not invented.
        let one = Condition::default().with_band(0, ConditionBand::Green);
        assert_eq!(one.keeping_destroyed(watched).letters(), "g");
        assert_eq!(one.keeping_destroyed(watched).sections, 1);
    }

    #[test]
    fn the_words_and_the_letters_are_one_vocabulary() {
        for band in [
            ConditionBand::Green,
            ConditionBand::Yellow,
            ConditionBand::Red,
            ConditionBand::Destroyed,
        ] {
            assert_eq!(
                band.as_str().chars().next(),
                Some(band.letter()),
                "{band:?}'s letter is its word's first letter"
            );
            assert_eq!(ConditionBand::from_code(band.code()), band);
        }
    }

    #[test]
    fn a_section_that_holds_nothing_reads_green_rather_than_dividing_by_zero() {
        assert_eq!(band_of(0, 0, 667, 334), ConditionBand::Green);
        assert_eq!(band_of(0, 1, 667, 334), ConditionBand::Red);
        // The floors are read at most 1,000 by every caller; at 1,000 a whole
        // section is still green.
        assert_eq!(band_of(10, 10, 1_000, 1_000), ConditionBand::Green);
    }
}
