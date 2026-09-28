//! **The slope rule** (16b cut X.6; doc 16 §9 item 6, §3.2 "Terrain", §3.4 step
//! 2, §6 "AI").
//!
//! One rule, in one module, read by the two things that cross an edge: motion
//! ([`crate::arena::motion`], which drives over it) and the reflex controller's
//! A\* ([`crate::arena::path`], which plans over it). Two copies of the
//! arithmetic would be two answers, and the answer a route promised has to be
//! the answer the tank gets — a route over an edge the tank is then refused at
//! is a tank that drives at a bluff for the rest of the match.
//!
//! # What the rule is
//!
//! A cell's `move_cost` is a property of the cell; a slope is a property of the
//! *pair* of cells an edge joins. So the two are priced in different places:
//! [`crate::definitions::arena::from_pack`] prices each cell from the shipped
//! locomotion table once, at apply time — the `tracked` row on the cell, the
//! other two in the terrain's class table since 16b cut AE.1, and a unit pays
//! its own row ([`CostRow::read`], read here beside the limit) — and the
//! surcharge for the edge a unit crossed to get there is applied here, at the
//! crossing and in the search.
//!
//! * [`grade_pct`] is the **uphill** slope of a step, per cent — the atlas's own
//!   `mush_atlas::physics::slope_pct` and not a second copy of it, so a step the
//!   atlas calls 20 per cent is 20 per cent here.
//! * [`effective_cost`] is the surcharge: `cost × (100 + grade) / 100`, which at
//!   grade zero is `cost × 100 / 100` — the cost itself, exactly, with no branch.
//!   Every flat window therefore runs the arithmetic it ran before this cut.
//! * [`steepness_pct`] is the slope of the edge **whichever way it is crossed**,
//!   and [`too_steep`] compares it against [`SlopeLimit::read`]'s limit.
//!
//! # What the rule is not
//!
//! It is `slope_pct` plus the table's limit, and it is deliberately **not**
//! `mush_atlas::physics::move_cost`'s climb branch (`rise <= climb_cm &&
//! is_built(to.class)` gives slope 0 and a flat 50 per cent surcharge). That
//! branch models a kerb, a doorstep or a loading dock *inside* one cell; at the
//! arena's 10 m cells there is no kerb to model, and applying it would charge a
//! `legged` walker 50 per cent for the 100 cm road contour a tank climbs for 10.
//! So a `legged` unit stepping up 100 cm onto `Road` at 1,000 cm cells pays
//! `cost × 110 / 100` in the arena and `cost × 150 / 100` in the atlas, and that
//! difference is pinned by a test below rather than left to be discovered.
//!
//! The arena never calls `physics::move_cost`. What it calls is `slope_pct` and
//! `LocomotionTable::max_slope_pct`, and it composes them itself.
//!
//! # The one divergence from the atlas: the limit is symmetric
//!
//! `physics::move_cost` refuses a step whose **uphill** slope passes the limit,
//! and says nothing about the descent. In a world where a unit can be told to
//! walk one step that is fine. In an arena it is not: an uphill-only limit makes
//! any bluff a one-way door, and a tank that rolls down a 50 per cent drop it
//! cannot climb is a tank parked at the bottom of a hole for the rest of the
//! match, with a reflex controller re-planning at it every `repath_ms`. So
//! [`steepness_pct`] takes the edge's slope in both directions and the refusal
//! is symmetric. The *surcharge* stays uphill-only, because a descent costs no
//! more than the flat.
//!
//! # No I/O, and no read at pulse time
//!
//! The outside dependency is `mush_atlas::physics::{slope_pct, StepKind,
//! LocomotionTable}` — arithmetic and a table, no window, no record, no store, so
//! the no-I/O gate (doc 16 §3.1, D-16-1 and D-16-3) still holds.
//! `LocomotionTable::shipped()` is an `include_str!` parsed once behind a
//! `OnceLock`, and [`SlopeLimit::read`] is called once per unit at check-out, so
//! nothing here reads the table inside a pulse.

use mush_atlas::Cell;
use mush_atlas::physics::{LocomotionTable, StepKind, slope_pct};

use crate::arena::state::{CostRow, TerrainCell};
use crate::definitions::unit::{LEGGED, TRACKED, WHEELED};

/// The step between two cells, or `None` when they are not neighbours —
/// `mush_atlas::physics::StepKind::between`, named here so that the arena has
/// one door onto the atlas's step vocabulary.
#[must_use]
pub fn step_kind(from: (u16, u16), to: (u16, u16)) -> Option<StepKind> {
    StepKind::between(from, to)
}

/// An arena cell as the atlas's trace and physics vocabulary spells it: the two
/// fields the slope arithmetic reads and the defaults for the two it does not.
fn as_atlas(cell: &TerrainCell) -> Cell {
    Cell {
        class: cell.class,
        elev_cm: cell.elev_cm,
        ..Cell::default()
    }
}

/// **The uphill slope of the step `from → to`, per cent** — the atlas's own
/// `mush_atlas::physics::slope_pct` over the two cells.
///
/// A rise of zero or less answers `0`: a descent is not a climb, and this is the
/// number [`effective_cost`] surcharges by. A diagonal's run is `cell_cm × 141 /
/// 100`, which is the atlas's √2 and not this module's.
#[must_use]
pub fn grade_pct(from: &TerrainCell, to: &TerrainCell, kind: StepKind, cell_cm: u32) -> u32 {
    slope_pct(as_atlas(from), as_atlas(to), kind, cell_cm)
}

/// **The slope of an edge whichever way it is crossed**: the greater of the two
/// [`grade_pct`]s, which is the climb of whichever cell is the higher.
///
/// This is what [`too_steep`] compares against a limit, for the reason the
/// module head gives: a limit that refused only the climb would leave a unit at
/// the bottom of a drop it could not get out of.
#[must_use]
pub fn steepness_pct(from: &TerrainCell, to: &TerrainCell, kind: StepKind, cell_cm: u32) -> u32 {
    grade_pct(from, to, kind, cell_cm).max(grade_pct(to, from, kind, cell_cm))
}

/// Whether `steepness` passes `limit` — **strictly greater**, as the atlas's own
/// `move_cost` compares it, so a slope exactly equal to the limit is climbed.
///
/// `None` is no limit at all, which is what a unit with no declared locomotion
/// has (see [`SlopeLimit::read`]).
#[must_use]
pub fn too_steep(limit: Option<u32>, steepness: u32) -> bool {
    limit.is_some_and(|limit| steepness > limit)
}

/// **The slope surcharge**: `cost × (100 + grade_pct) / 100`.
///
/// At `grade_pct == 0` this is `cost × 100 / 100`, which is `cost` exactly and
/// for every `cost` — the flat-case identity the whole cut rests on, since it is
/// what keeps every pre-existing flat fixture running the arithmetic it ran
/// before. The product is taken in `u64` and saturates, so a pathological cost
/// and a pathological grade answer [`u32::MAX`] rather than wrapping.
#[must_use]
pub fn effective_cost(cost: u32, grade_pct: u32) -> u32 {
    let scaled = u64::from(cost).saturating_mul(100_u64.saturating_add(u64::from(grade_pct))) / 100;
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// **The mode a unit drives under**: the first mode whose locomotion is
/// [`TRACKED`], else the first whose locomotion is [`WHEELED`] (16b cut AE.1),
/// else the first whose locomotion is [`LEGGED`], else the first whose *id*
/// contains [`TRACKED`].
///
/// The same rungs `crate::match_::arena_def::tracked_rules` climbs to decode a
/// unit's kinematics, written once here so that the two readers — three, with
/// [`CostRow::read`] — climb one ladder rather than copies of it waiting to
/// disagree.
///
/// **The ladder is shared; what a caller does when it answers `None` is not.**
/// `tracked_rules` falls through to doc 16 §3.3's stock tank, because a unit
/// whose modes this does not recognise still has to drive something.
/// [`SlopeLimit::read`] falls back to the profile's **first** mode instead, so a
/// `hover` or `biped`-only unit is held to its own row rather than left
/// silently unlimited. A unit built that way therefore drives on the stock
/// tank's kinematics and is limited by its own locomotion — two rows, by
/// intent; no shipped unit is one. **16b cut AE.1** put `wheeled` on the ladder
/// itself, because the scout car is one: a wheeled unit now decodes its own
/// five numbers, is held to the wheeled row's 25 per cent and pays the wheeled
/// row's prices, all off the one mode.
#[must_use]
pub fn driving_mode(
    profile: &mush_tactics::MobilityProfile,
) -> Option<&mush_tactics::MobilityMode> {
    profile
        .modes()
        .iter()
        .find(|mode| mode.locomotion.as_str() == TRACKED)
        .or_else(|| {
            profile
                .modes()
                .iter()
                .find(|mode| mode.locomotion.as_str() == WHEELED)
        })
        .or_else(|| {
            profile
                .modes()
                .iter()
                .find(|mode| mode.locomotion.as_str() == LEGGED)
        })
        .or_else(|| {
            profile
                .modes()
                .iter()
                .find(|mode| mode.id.as_str().contains(TRACKED))
        })
}

/// **What the ground will let a unit climb** (doc 16 §9 item 6), read off the
/// shipped locomotion table at check-out.
///
/// A type rather than a bare function for the reason
/// [`crate::arena::state::HeatState`] is one: it is a fact read off the M32 unit
/// once, at the seam, and the read is the whole of it.
pub struct SlopeLimit;

impl SlopeLimit {
    /// The slope limit of the mode this unit drives under, per cent.
    ///
    /// `None` when the unit carries no mobility profile, when the profile
    /// carries no mode, or when its locomotion is not one the shipped table
    /// knows — and `None` is **no limit**, not a limit of zero: an engine
    /// fixture whose unit declares no locomotion still drives, and still pays
    /// the surcharge. The tags come from the mode itself, so a `jump` mode's
    /// `slope_factor` doubles its limit exactly as the atlas doubles it.
    ///
    /// The mode is [`driving_mode`]'s, **or the profile's first** when that
    /// ladder answers nothing: a unit whose only mode is `hover` is held to
    /// the hover row's 20 per cent rather than to nothing at all. That fallback
    /// is this reader's and not the kinematics reader's; see [`driving_mode`]
    /// for why the two part company there. (A `wheeled`-only unit was this
    /// fallback's case until 16b cut AE.1 put `wheeled` on the ladder; it
    /// answers the wheeled row's 25 per cent either way.)
    #[must_use]
    pub fn read(unit: &mush_tactics::TacticalUnit) -> Option<u32> {
        let profile = unit.mobility()?;
        let mode = driving_mode(profile).or_else(|| profile.modes().first())?;
        let tags: Vec<&str> = mode
            .tags
            .iter()
            .map(mush_tactics::MobilityTag::as_str)
            .collect();
        LocomotionTable::shipped().max_slope_pct(mode.locomotion.as_str(), &tags)
    }
}

impl CostRow {
    /// **The row a unit pays, read off the M32 unit at check-out** (16b cut
    /// AE.1; 16c `B-58`), beside [`SlopeLimit::read`] and off the same mode.
    ///
    /// The locomotion of [`driving_mode`]'s mode, **or the profile's first
    /// mode's** when that ladder answers nothing — [`SlopeLimit::read`]'s own
    /// fallback, so the limit and the prices a unit is held to are one mode's —
    /// mapped to its row: `tracked`, `wheeled`, `legged`. **Anything else is
    /// [`CostRow::Tracked`]**: a unit with no mobility profile (every engine
    /// fixture in this crate), a profile with no mode, or a locomotion the three
    /// rows do not name (`hover`), which is the price every unit paid before
    /// this cut. A read, not a guess: the tracked row is what the cells carry
    /// as `move_cost`.
    #[must_use]
    pub fn read(unit: &mush_tactics::TacticalUnit) -> CostRow {
        let Some(profile) = unit.mobility() else {
            return CostRow::Tracked;
        };
        let Some(mode) = driving_mode(profile).or_else(|| profile.modes().first()) else {
            return CostRow::Tracked;
        };
        match mode.locomotion.as_str() {
            WHEELED => CostRow::Wheeled,
            LEGGED => CostRow::Legged,
            _ => CostRow::Tracked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_atlas::TerrainClass;
    use mush_atlas::physics::move_cost;

    /// A cell of `class` at `elev_cm`; `move_cost` is not read by anything here.
    fn cell(class: TerrainClass, elev_cm: i16) -> TerrainCell {
        TerrainCell {
            class,
            elev_cm,
            move_cost: 100,
        }
    }

    #[test]
    fn the_grade_is_the_atlases_own_slope() {
        for (from_cm, to_cm) in [
            (0_i16, 0_i16),
            (0, 100),
            (0, 500),
            (500, 0),
            (-400, 400),
            (400, -400),
            (0, 1_200),
            (-100, -100),
        ] {
            for kind in [StepKind::Orthogonal, StepKind::Diagonal] {
                for cell_cm in [500_u32, 1_000, 2_000] {
                    let from = cell(TerrainClass::Ground, from_cm);
                    let to = cell(TerrainClass::Ground, to_cm);
                    assert_eq!(
                        grade_pct(&from, &to, kind, cell_cm),
                        slope_pct(as_atlas(&from), as_atlas(&to), kind, cell_cm),
                        "{from_cm} -> {to_cm} at {cell_cm} cm, {kind:?}"
                    );
                }
            }
        }
        // The two numbers the fixtures below lean on, spelled out.
        let flat = cell(TerrainClass::Ground, 0);
        let up = cell(TerrainClass::Ground, 300);
        assert_eq!(grade_pct(&flat, &up, StepKind::Orthogonal, 1_000), 30);
        assert_eq!(
            grade_pct(&up, &flat, StepKind::Orthogonal, 1_000),
            0,
            "a descent is not a climb"
        );
        // A diagonal's run is the longer one, so the same rise is a gentler grade.
        assert_eq!(grade_pct(&flat, &up, StepKind::Diagonal, 1_000), 21);
    }

    #[test]
    fn the_steepness_of_an_edge_is_the_number_the_climb_would_have_been() {
        // The numbers, not the symmetry: `steepness_pct` is a `max` of the two
        // grades and so is symmetric by construction, which makes asserting that
        // it is symmetric an assertion that `max(a, b) == max(b, a)`. What is
        // worth pinning is the value — the climb of whichever cell is the higher
        // — because that is the number a limit is compared against.
        for (a_cm, b_cm, orthogonal, diagonal) in [
            (0_i16, 0_i16, 0_u32, 0_u32),
            (0, 100, 10, 7),
            (0, 500, 50, 35),
            (-400, 400, 80, 56),
            (200, -300, 50, 35),
        ] {
            let a = cell(TerrainClass::Ground, a_cm);
            let b = cell(TerrainClass::Ground, b_cm);
            for (kind, want) in [
                (StepKind::Orthogonal, orthogonal),
                (StepKind::Diagonal, diagonal),
            ] {
                assert_eq!(
                    steepness_pct(&a, &b, kind, 1_000),
                    want,
                    "{a_cm} / {b_cm}, {kind:?}"
                );
                assert_eq!(
                    steepness_pct(&b, &a, kind, 1_000),
                    want,
                    "{b_cm} / {a_cm}, {kind:?}"
                );
                // And the uphill grade is one of the two, so a rule that read the
                // grade where it means the steepness would answer this instead.
                assert_eq!(
                    grade_pct(&a, &b, kind, 1_000).max(grade_pct(&b, &a, kind, 1_000)),
                    want
                );
            }
        }
        // The limit is strictly greater, so a slope equal to it is climbed.
        assert!(!too_steep(Some(45), 45));
        assert!(too_steep(Some(45), 46));
        assert!(!too_steep(None, 10_000), "no limit refuses nothing");
    }

    #[test]
    fn the_flat_case_is_the_identity() {
        // The whole of why every flat fixture in this crate runs unchanged.
        for cost in 0..=u32::from(u16::MAX) {
            assert_eq!(effective_cost(cost, 0), cost);
        }
        assert_eq!(effective_cost(100, 30), 130);
        assert_eq!(effective_cost(130, 20), 156);
        // Integer division truncates, exactly as the atlas's does.
        assert_eq!(effective_cost(100, 7), 107);
        assert_eq!(effective_cost(3, 33), 3);
        // And it saturates rather than wrapping.
        assert_eq!(effective_cost(u32::MAX, u32::MAX), u32::MAX);
    }

    #[test]
    fn an_arena_edge_is_the_atlases_step_but_for_the_diagonal_constant() {
        let table = LocomotionTable::shipped();
        let from = cell(TerrainClass::Ground, 0);
        let to = cell(TerrainClass::Ground, 300);
        let base = u32::from(
            table
                .cost("ground", TRACKED, &[])
                .and_then(|cost| u16::try_from(cost).ok())
                .expect("the tracked row prices ground"),
        );
        // Orthogonal: the arena's edge IS the atlas's step.
        let grade = grade_pct(&from, &to, StepKind::Orthogonal, 1_000);
        assert_eq!(
            move_cost(
                table,
                as_atlas(&from),
                as_atlas(&to),
                StepKind::Orthogonal,
                1_000,
                TRACKED,
                &[]
            ),
            Ok(effective_cost(base, grade))
        );
        // Diagonal: the same composition, over a different √2. The atlas's is
        // 141/100 and the arena's A\* is 181/128 — a residual that predates this
        // cut (`crate::arena::path::DIAGONAL_NUM`), named here rather than fixed.
        let grade = grade_pct(&from, &to, StepKind::Diagonal, 1_000);
        assert_eq!(
            move_cost(
                table,
                as_atlas(&from),
                as_atlas(&to),
                StepKind::Diagonal,
                1_000,
                TRACKED,
                &[]
            ),
            Ok(effective_cost(base * 141 / 100, grade))
        );
        // 1.41 against 1.4140625: at the tracked row's ground cost the two round
        // to the same whole hundredth, and at a dearer cell they do not.
        assert_eq!(base * 141 / 100, base * 181 / 128);
        assert_eq!((400 * 141 / 100, 400 * 181 / 128), (564, 565));
    }

    #[test]
    fn the_climb_rule_is_the_pinned_difference_against_the_atlas() {
        // A `legged` walker stepping up 100 cm onto road at 10 m cells: the atlas
        // calls it a kerb and charges half again; the arena calls it a contour and
        // charges the 10 per cent grade. See the module head.
        let table = LocomotionTable::shipped();
        let from = cell(TerrainClass::Grass, 0);
        let to = cell(TerrainClass::Road, 100);
        let base = u32::from(
            table
                .cost("road", LEGGED, &[])
                .and_then(|cost| u16::try_from(cost).ok())
                .expect("the legged row prices road"),
        );
        assert_eq!(
            move_cost(
                table,
                as_atlas(&from),
                as_atlas(&to),
                StepKind::Orthogonal,
                1_000,
                LEGGED,
                &[]
            ),
            Ok(base * 150 / 100),
            "the atlas applies the climb surcharge"
        );
        let grade = grade_pct(&from, &to, StepKind::Orthogonal, 1_000);
        assert_eq!(grade, 10);
        assert_eq!(
            effective_cost(base, grade),
            base * 110 / 100,
            "the arena applies the grade"
        );
    }

    /// A one-mode profile over a vocabulary wide enough to declare it — the
    /// engine's base plus `maglev`, so the "a locomotion the table does not
    /// know" case can be built at all.
    fn profile(locomotion: &str, id: &str) -> mush_tactics::MobilityProfile {
        use mush_tactics::{
            LocomotionId, MediumId, MobilityMode, MobilityModeId, MobilityProfile,
            MobilityVocabulary,
        };
        let mode_id = MobilityModeId::new(id).expect("a canonical mode id");
        let mode = MobilityMode::new(
            mode_id.clone(),
            [MediumId::new("ground").expect("a canonical medium")],
            LocomotionId::new(locomotion).expect("a canonical locomotion"),
            crate::arena::test_support::rules(),
        );
        let mut vocabulary = MobilityVocabulary::base();
        vocabulary.declare_locomotion(LocomotionId::new("maglev").expect("a canonical locomotion"));
        MobilityProfile::new(vec![mode], mode_id, &vocabulary.freeze()).expect("a one-mode profile")
    }

    /// The smallest M32 unit that carries a mobility profile —
    /// `crate::arena::test_support::tactical_unit`'s shape with `mobility` filled.
    fn unit_with(profile: mush_tactics::MobilityProfile) -> mush_tactics::TacticalUnit {
        use crate::arena::test_support::{dref, rules};
        let spec = mush_tactics::UnitSpec {
            definition: dref("unit", "tacsim.unit.slope_probe"),
            sections: vec![mush_tactics::SectionSpec {
                id: mush_tactics::SectionId::new("hull").expect("a canonical section"),
                condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
                facets: Vec::new(),
                slots: Vec::new(),
                cells: Vec::new(),
            }],
            unit_cells: Vec::new(),
            components: Vec::new(),
            capabilities: Vec::new(),
            mobility: Some(profile),
            rules: rules(),
        };
        mush_tactics::TacticalUnit::from_spec(&spec, mush_core::ObjId(4))
            .expect("a one-section unit")
    }

    #[test]
    fn the_limit_is_the_modes_own_and_an_unknown_locomotion_has_none() {
        // A wheeled-only profile: since 16b cut AE.1 `wheeled` is a rung of the
        // ladder itself (the scout car drives on it), so the ladder answers and
        // the limit is the wheeled row's own 25 per cent.
        let wheeled = profile("wheeled", "ground.wheeled");
        assert_eq!(
            driving_mode(&wheeled).map(|mode| mode.locomotion.as_str().to_string()),
            Some(WHEELED.to_string())
        );
        assert_eq!(SlopeLimit::read(&unit_with(wheeled.clone())), Some(25));
        assert_eq!(CostRow::read(&unit_with(wheeled)), CostRow::Wheeled);
        // A hover-only profile is the first-mode fallback's case now: the
        // hover row's 20 per cent, and the tracked prices.
        let hover = profile("hover", "ground.hover");
        assert!(driving_mode(&hover).is_none());
        assert_eq!(SlopeLimit::read(&unit_with(hover.clone())), Some(20));
        assert_eq!(CostRow::read(&unit_with(hover)), CostRow::Tracked);
        // A locomotion the table does not know has no limit at all.
        assert_eq!(
            SlopeLimit::read(&unit_with(profile("maglev", "ground.maglev"))),
            None
        );
        // And the ladder: tracked first, then legged, then an id that says so.
        let tracked = profile(TRACKED, "ground.tracked");
        assert_eq!(
            driving_mode(&tracked).map(|mode| mode.locomotion.as_str().to_string()),
            Some(TRACKED.to_string())
        );
        assert_eq!(SlopeLimit::read(&unit_with(tracked.clone())), Some(45));
        assert_eq!(CostRow::read(&unit_with(tracked)), CostRow::Tracked);
        let legged = profile(LEGGED, "ground.legged");
        assert_eq!(
            driving_mode(&legged).map(|mode| mode.locomotion.as_str().to_string()),
            Some(LEGGED.to_string())
        );
        assert_eq!(SlopeLimit::read(&unit_with(legged.clone())), Some(70));
        assert_eq!(CostRow::read(&unit_with(legged)), CostRow::Legged);
        let keyed = profile("hover", "ground.tracked.hover");
        assert_eq!(
            driving_mode(&keyed).map(|mode| mode.id.as_str().to_string()),
            Some("ground.tracked.hover".to_string())
        );
    }

    #[test]
    fn a_unit_with_no_mobility_profile_has_no_limit() {
        // Every engine fixture in this crate: `mobility: None`, so no limit, and
        // still the surcharge. That is what keeps the pre-existing motion tests
        // green (see `crate::arena::motion`'s own note).
        let unit = crate::arena::test_support::tactical_unit(mush_core::ObjId(4));
        assert_eq!(SlopeLimit::read(&unit), None);
        // 16b cut AE.1: and the tracked prices, which is what they always paid.
        assert_eq!(CostRow::read(&unit), CostRow::Tracked);
    }
}
