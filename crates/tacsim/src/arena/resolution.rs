//! **Whether a shot can hit** (16b cut W.2, W2; doc 16 §3.5 "Resolution" steps 1
//! to 5, §3.3 "Facet attribution" and "Silhouette").
//!
//! Doc 16 §3.5 resolves a shot **at the moment `FIRE` is accepted**, in five
//! steps, and this module is those five steps and nothing else: what the target
//! presents, how wide the shooter's error is, the curve that turns the ratio of
//! the two into a probability, the draw that compares against it, and the number
//! the cockpit shows before any of it happens.
//!
//! # Everything is an angle, and the angle is in microradians
//!
//! The whole mechanic is one comparison: **how big does the target look against
//! how badly the gun scatters**. Doc 16 §3.5 writes both figures in milliradians
//! and forms `x = angular_size_mrad × 1000 / error_mrad`, which is a per-mille
//! ratio of two angles — dimensionless, so the unit the two are measured in
//! cancels. This cut measures both in **microradians** for one reason: a
//! milliradian is far too coarse an integer for the numbers the content actually
//! authors. The stock tank presents 350 cm at 1 km, which is 3.5 mrad; in whole
//! milliradians that is 3, and the shot the content's own note works out as
//! `x = 1166` would come out as `x = 1000`. In microradians it is 3500 against a
//! 3000 µrad cone and `x` is 1166 exactly — the content author's arithmetic,
//! reproduced. Nothing about the doc's formula changes: the same ratio, three more
//! digits, no rounding of a weapon's dispersion to the nearest whole cone.
//!
//! # The error terms, and what one unit of each is worth
//!
//! `resolution.error_terms` (cut C.2, `content/resolution.json`) carries the
//! **per-mille weights** and no measurements: *cut W multiplies the term's own
//! measured quantity by the weight and adds the result to the weapon's band
//! dispersion*. So this cut owes each term a measurement, and the calibration
//! below is it — one **reference unit** of any term is worth `1000` µrad (one
//! milliradian) at a weight of `1000` (×1.0):
//!
//! | Term | One reference unit is | At the stock weight |
//! |---|---|---|
//! | `shooter_speed` | [`REFERENCE_SPEED_CMS`] (10 m/s) of the shooter's own speed | 1,000: a tank at its 15 m/s ceiling adds 1.5 mrad |
//! | `target_lateral_speed` | the same speed **across** the line of fire | 800: a target crossing at 15 m/s adds 1.2 mrad |
//! | `gunnery` | a rating of 1 ([`PilotProfile`](crate::sheet::PilotProfile)'s floor), falling as `1 / rating` | 1,000: a green gunner adds 1 mrad, an ace 0.2 |
//! | `sensor_condition` | a sight at **zero** condition; nothing at full | 500: a wrecked sight adds 0.5 mrad |
//! | `beam_attenuation` | a beam's line **wholly** attenuated by the ground it crosses ([`FULL_ATTENUATION`]); zero for anything that is not a beam | 1,000: a beam through half-obscuring ground adds 0.5 mrad (16b cut X.1) |
//!
//! Every term **adds**: a cone is never narrower than the weapon's own
//! dispersion, and a better gunner adds less rather than subtracting. That is the
//! one reading of "narrows or widens" that stays monotone and cannot drive the
//! cone to zero. It does mean the stock shot is a little harder than
//! `content/resolution.json`'s note works out — the note omits the terms, so its
//! 1 km shot is 40 % where this arithmetic makes it 30 % — and that is the note
//! describing the curve rather than the resolution.
//!
//! # What the impact re-resolves, and what it does not
//!
//! Doc 16 §3.5 puts re-validation at the impact and says *a target that changed
//! facet band re-resolves with the same `hit` draw*. So the probability is
//! computed **here**, at acceptance, and carried on the
//! [`Shot`](crate::arena::weapons::Shot): the shooter's speed, the gunner, the
//! sight and the range are all facts about the moment the round left the barrel.
//! The one thing the flight can change is which face of the target the round
//! arrives at, so [`Arena::reresolve`] recomputes the angular size from the new
//! band's width against the **same** error cone and the **same** draw. Everything
//! else about the shot is already decided.

use mush_atlas::Cover;
use mush_core::ObjId;
use smol_str::SmolStr;

use crate::arena::index::range_cm;
use crate::arena::motion::{SIN_ONE, sin_fx};
use crate::arena::perception::bearing_mdeg;
use crate::arena::rng::{DrawLabel, draw, draw_permille, splitmix64};
use crate::arena::state::{Arena, Side, UnitState};
use crate::definitions::resolution::PERMILLE;

// ---------------------------------------------------------------------------
// The calibration constants
// ---------------------------------------------------------------------------

/// **One reference unit of speed**, cm/s: 10 m/s (see the module head's table).
///
/// It is not a tank's ceiling and is not meant to be — it is the divisor that
/// turns a speed into the multiple of a reference the weights are authored
/// against, so a faster unit class does not need a new table.
pub const REFERENCE_SPEED_CMS: u32 = 1_000;

/// **What one reference unit of any error term is worth**, microradians.
///
/// A milliradian. Every weight in `resolution.error_terms` is a per-mille
/// multiplier of this (see the module head).
pub const TERM_URAD: u32 = 1_000;

/// Microradians in a milliradian — the factor a band's authored
/// `dispersion_mrad` is raised by.
pub const URAD_PER_MRAD: u32 = 1_000;

/// The gunnery rating a unit with no pilot profile is resolved at: the sheet's
/// own floor (`crate::sheet::schema::RATING_MIN`), and doc 16 §3.5's *every stock
/// rating is 1*.
pub const STOCK_GUNNERY: u8 = 1;

/// A sight at full health, as the M32 `condition` cell carries it
/// (`crate::definitions::unit::MAX_CONDITION`).
pub const FULL_CONDITION: u32 = 100;

/// **A fully attenuated beam**, percent — one reference unit of the fifth error
/// term (16b cut X.1; doc 16 §9 item 1).
///
/// The trace's `attenuation` is a percentage of the line lost to the ground it
/// crosses, so a hundred is the whole of it and the term is worth its weight
/// there; the raw sum a walk gathers can run past a hundred through enough
/// partial cells, and the term is capped here rather than in the walk.
pub const FULL_ATTENUATION: u32 = 100;

/// The narrowest cone the arithmetic will divide by, microradians. A weapon with
/// no authored dispersion, a still shooter, an ace gunner and a perfect sight
/// would otherwise be a division by zero; one microradian is a cone a hundredth
/// the width of anything the content authors, so the clamp is a guard and never a
/// rule.
pub const MIN_ERROR_URAD: u32 = 1;

// ---------------------------------------------------------------------------
// What a unit presents, and what a hit may strike
// ---------------------------------------------------------------------------

/// **One facet band of a unit** (16b cut W.2), as the check-out hands it over:
/// what it presents and what a hit on it may strike.
///
/// The arena's own copy of the two halves `crate::definitions::unit::UnitSpec`
/// authors separately — `silhouettes` (doc 16 §3.3 "Silhouette") and `facets`
/// (doc 16 §3.3 "Facet attribution", W12) — joined on the band id, because every
/// reader of one wants the other. Same reason
/// [`crate::arena::weapons::WeaponBand`] is a copy of the weapon decoder's row:
/// the content types are `serde`-shaped and live beside the pack loader, and the
/// arena holds neither (D-16-1, D-16-3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BandProfile {
    /// The band: `front`, `left`, `right`, `rear`, `top`. It is also a facet id,
    /// which is what lets a hit on a band find the armour it must pass.
    pub band: SmolStr,
    /// Presented width, centimetres — the figure the angular size is taken from.
    pub width_cm: u32,
    /// Presented height, centimetres — the other half of the presented area.
    pub height_cm: u32,
    /// The sections a hit on this band may strike, and their weights (doc 16
    /// §3.3: *the band's weights need not sum to anything*).
    pub sections: Vec<(SmolStr, u16)>,
}

/// **What a section's destruction does** (16b cut W.2; doc 16 §3.3's *Consequence
/// at zero internals* column) — the arena's own copy of
/// `crate::definitions::unit::Consequence`, a closed vocabulary over open ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Consequence {
    /// The unit becomes a wreck (doc 16 §3.3 "A wreck").
    DestroyUnit,
    /// Every weapon mounted on that section is removed.
    RemoveWeapons {
        /// The section whose weapons go.
        section: SmolStr,
    },
    /// The speed ceiling falls to `percent` of the rules value.
    Slow {
        /// The surviving percentage, 1 to 100.
        percent: u8,
    },
    /// The unit can no longer turn toward that side.
    NoTurnToward {
        /// Left or right.
        side: Side,
    },
    /// The unit cannot move at all.
    DisableMotion,
}

impl Consequence {
    /// The word the ledger records (the same word
    /// `crate::definitions::unit::Consequence::label` renders).
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Consequence::DestroyUnit => "destroy_unit",
            Consequence::RemoveWeapons { .. } => "remove_weapons",
            Consequence::Slow { .. } => "slow",
            Consequence::NoTurnToward { .. } => "no_turn_toward",
            Consequence::DisableMotion => "disable_motion",
        }
    }
}

/// **One section of a unit** (16b cut W.2), as the check-out hands it over.
///
/// Only what damage needs that the [`mush_tactics::TacticalUnit`] does not say
/// legibly: which facets the section carries (so a band can be turned into an
/// armour cell), what happens at zero internals, and where a section with no
/// armour of its own is struck through (doc 16 §3.3: the engine has *no facets;
/// struck through `hull.rear`*).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionProfile {
    /// The section: `hull`, `turret`, `left_track`, `right_track`, `engine`.
    pub id: SmolStr,
    /// The facet ids it declares, in authored order. Empty for a section struck
    /// through another's.
    pub facets: Vec<SmolStr>,
    /// What happens when its `internals` reach zero, in authored order.
    pub consequences: Vec<Consequence>,
    /// `(section, facet)` whose armour a shot must pass to reach this one.
    pub struck_through: Option<(SmolStr, SmolStr)>,
}

/// **The damage model of one unit** (16b cut W.2), crossed once at check-out.
///
/// A unit with an empty profile can still be shot at and still resolves — it
/// presents nothing, so its angular size is zero and doc 16 §3.5's *zero
/// presented area is zero* answers — which is what keeps every cut-A and cut-S
/// fixture running unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitProfile {
    /// The facet bands, in authored order.
    pub bands: Vec<BandProfile>,
    /// The sections, in authored order.
    pub sections: Vec<SectionProfile>,
}

impl UnitProfile {
    /// One band by its id.
    #[must_use]
    pub fn band(&self, id: &str) -> Option<&BandProfile> {
        self.bands.iter().find(|band| band.band == id)
    }

    /// One section by its id.
    #[must_use]
    pub fn section(&self, id: &str) -> Option<&SectionProfile> {
        self.sections.iter().find(|section| section.id == id)
    }
}

// ---------------------------------------------------------------------------
// Step 1's first half: which face is presented
// ---------------------------------------------------------------------------

/// **The four facet bands a direct-fire shot can arrive at**, and the sectors
/// that select them (doc 16 §3.3 "Facet attribution": *the impact's relative
/// bearing selects a facet band*).
///
/// `top` is deliberately absent: doc 16 §3.3's default table gives it to hull and
/// turret and 16b's cut W row says *top never for direct fire* — a round on a flat
/// trajectory arrives at a side, and the overhead band waits for the indirect fire
/// of doc 16 §9.
pub const DIRECT_BANDS: [&str; 4] = ["front", "right", "rear", "left"];

/// A quarter turn in millidegrees.
const QUARTER_MDEG: u32 = 90_000;

/// A full turn in millidegrees.
const FULL_MDEG: u32 = 360_000;

/// **Which band `shooter` presents itself to on `target`** (doc 16 §3.3).
///
/// The relative bearing of the shooter *seen from the target*: the absolute
/// bearing from the target to the shooter, less the target's own heading. The four
/// 90° sectors are centred on the target's axes — front is the 45° either side of
/// dead ahead — so a round from directly abeam is a side hit and a round from 44°
/// off the bow is a front hit, which is the coarse-but-legible reading doc 16 asks
/// for (*no geometry is computed*).
///
/// Two units in one cell (a range of zero) answer `front`: [`bearing_mdeg`] has no
/// bearing to give, and a point-blank shot at a unit's own cell is not a case the
/// occupancy rule allows anyway.
#[must_use]
pub fn facet_band(target: &UnitState, shooter: &UnitState) -> &'static str {
    let absolute = bearing_mdeg(&target.pose, &shooter.pose);
    let relative = (absolute + FULL_MDEG).saturating_sub(target.pose.heading_mdeg) % FULL_MDEG;
    // The sectors are centred, so the half-sector is rotated away before the
    // division: 315_000..360_000 and 0..45_000 both land in sector 0.
    let sector = ((relative + QUARTER_MDEG / 2) % FULL_MDEG) / QUARTER_MDEG;
    DIRECT_BANDS
        .get(usize::try_from(sector).unwrap_or(0))
        .copied()
        .unwrap_or("front")
}

// ---------------------------------------------------------------------------
// The resolution itself
// ---------------------------------------------------------------------------

/// **One resolved shot** (16b cut W.2; doc 16 §3.5 steps 1 to 3), every figure an
/// integer.
///
/// It is recorded on the [`crate::arena::weapons::Shot`] at acceptance and read
/// again at the impact, so the black box and the re-validation agree on the
/// arithmetic by construction rather than by running it twice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The facet band the target presented toward the shooter.
    pub band: SmolStr,
    /// The presented width after cover, centimetres. Zero is doc 16 §3.5's *zero
    /// presented area*.
    pub presented_width_cm: u32,
    /// The presented area after cover, square centimetres.
    pub presented_area_cm2: u64,
    /// The target's angular size, microradians.
    pub angular_urad: u32,
    /// The error cone, microradians.
    pub error_urad: u32,
    /// The curve's argument: `angular_urad × 1000 / error_urad`, per mille.
    pub x_permille: u32,
    /// The probability one round lands, per mille — clamped to
    /// `[resolution.p_min, resolution.p_max]`, and exactly zero for a target that
    /// presents nothing.
    pub p_permille: u32,
    /// What the target's cover did to the presented width.
    pub cover: Cover,
}

impl Resolution {
    /// A shot at something that presents nothing: doc 16 §3.5's *zero presented
    /// area is zero*, which is the one answer the clamp to `p_min` does not
    /// override.
    #[must_use]
    pub fn blind(band: &str, error_urad: u32, cover: Cover) -> Resolution {
        Resolution {
            band: SmolStr::new(band),
            presented_width_cm: 0,
            presented_area_cm2: 0,
            angular_urad: 0,
            error_urad,
            x_permille: 0,
            p_permille: 0,
            cover,
        }
    }
}

/// **What cover leaves of a presented silhouette**, per mille (doc 16 §3.5 step
/// 1: *`Partial` halves it, `Full` zeroes it*).
#[must_use]
pub const fn cover_permille(cover: Cover) -> u32 {
    match cover {
        Cover::None => PERMILLE,
        Cover::Partial => PERMILLE / 2,
        Cover::Full => 0,
    }
}

impl Arena {
    /// **Resolve one shot**, doc 16 §3.5 steps 1 to 3 (16b cut W.2).
    ///
    /// `range_cm` is the range the shot is taken at, which at acceptance is the
    /// range to the predicted point; `cover` is the target cell's cover off the
    /// trace. Both are the caller's because the caller has already traced — doc 16
    /// §3.5 costs the round whatever the trace says, so the trace happens once and
    /// its answer is passed in rather than taken again here.
    ///
    /// The three steps, in order:
    ///
    /// 1. the presented silhouette: the band the target shows the shooter, its
    ///    authored width and height, scaled by cover, and the angular size of the
    ///    width at this range;
    /// 2. the error cone: the weapon band's `dispersion_mrad` plus the four
    ///    weighted terms (see the module head);
    /// 3. the curve: `hit_permille(angular × 1000 / error)`, clamped by the table
    ///    itself, and zero for a target that presents nothing.
    ///
    /// A unit this arena does not hold, or a weapon it does not carry, resolves as
    /// [`Resolution::blind`] on a bare cone: there is nothing to present and
    /// nothing to aim, and an answer of zero is better than an `Option` every
    /// caller would have to turn into one anyway.
    #[must_use]
    pub fn resolve_shot(
        &self,
        shooter: ObjId,
        target: ObjId,
        weapon: &str,
        range_cm: u64,
        cover: Cover,
    ) -> Resolution {
        let (Some(from), Some(at)) = (self.unit(shooter), self.unit(target)) else {
            return Resolution::blind("front", MIN_ERROR_URAD, cover);
        };
        let band = facet_band(at, from);
        let error_urad = self.error_urad(from, at, weapon, range_cm);
        let Some(silhouette) = at.profile.band(band) else {
            // A unit whose content authored no silhouette for this band presents
            // nothing to it. Every cut-A and cut-S fixture is such a unit.
            return Resolution::blind(band, error_urad, cover);
        };
        let scale = cover_permille(cover);
        let width = u64::from(silhouette.width_cm)
            .saturating_mul(u64::from(scale))
            .saturating_div(u64::from(PERMILLE));
        let presented_width_cm = u32::try_from(width).unwrap_or(u32::MAX);
        if presented_width_cm == 0 {
            return Resolution::blind(band, error_urad, cover);
        }
        // Doc 16 §3.5 step 1 scales *the presented area*, and the angular size is
        // *the width's*: cover takes its bite out of the frontage, so one scale
        // does both and the area halved by `Partial` is the area the doc halves.
        let presented_area_cm2 = width.saturating_mul(u64::from(silhouette.height_cm));
        let angular_urad = u32::try_from(
            u64::from(presented_width_cm)
                .saturating_mul(1_000_000)
                .saturating_div(range_cm.max(1)),
        )
        .unwrap_or(u32::MAX);
        let (x_permille, p_permille) = self.hit_permille(angular_urad, error_urad);
        Resolution {
            band: SmolStr::new(band),
            presented_width_cm,
            presented_area_cm2,
            angular_urad,
            error_urad,
            x_permille,
            p_permille,
            cover,
        }
    }

    /// **Doc 16 §3.5 step 3**: the curve's argument and its value.
    ///
    /// `x = angular_urad × 1000 / error_urad`, capped at the curve's own
    /// `MAX_HIT_CURVE_X` so a point-blank shot cannot walk off the end of the
    /// table, and then the table's interpolation and its `[p_min, p_max]` clamp
    /// (`crate::definitions::resolution::ResolutionDef::hit_permille`, which is
    /// where the walk is written and tested).
    #[must_use]
    pub fn hit_permille(&self, angular_urad: u32, error_urad: u32) -> (u32, u32) {
        let x = u64::from(angular_urad)
            .saturating_mul(u64::from(PERMILLE))
            .saturating_div(u64::from(error_urad.max(MIN_ERROR_URAD)));
        let x = u32::try_from(x)
            .unwrap_or(u32::MAX)
            .min(crate::definitions::resolution::MAX_HIT_CURVE_X);
        (x, self.resolution.hit_permille(x))
    }

    /// **Doc 16 §3.5 step 2**: the error cone, microradians.
    ///
    /// The weapon band's own dispersion at this range plus the four weighted
    /// terms. A range past the weapon's last band has no dispersion to read and
    /// takes the **outermost** band's: the shot is out of reach and is going to
    /// miss, and taking the widest authored cone says so in the arithmetic rather
    /// than by a special case.
    #[must_use]
    pub fn error_urad(
        &self,
        shooter: &UnitState,
        target: &UnitState,
        weapon: &str,
        range_cm: u64,
    ) -> u32 {
        let dispersion_mrad = shooter
            .weapon(weapon)
            .map(|summary| {
                summary
                    .band_of(range_cm)
                    .map(|(_, band)| band.dispersion_mrad)
                    .or_else(|| summary.bands.last().map(|band| band.dispersion_mrad))
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let terms = &self.resolution.error_terms;
        let mut urad = u64::from(dispersion_mrad).saturating_mul(u64::from(URAD_PER_MRAD));
        // **16b cut X.1** (doc 16 §9 item 1): the fifth term, and a beam's alone.
        // The ground a beam crosses scatters it, so the line it arrives on is
        // wider than the one it left — which is the same thing cover is for a
        // round, said as a cone rather than as a silhouette. A direct-fire weapon
        // takes no trace for it and adds nothing: the `is_beam` test is the whole
        // of what every stock shot pays for this term existing.
        if shooter
            .weapon(weapon)
            .is_some_and(crate::arena::state::WeaponSummary::is_beam)
        {
            let attenuation = self
                .fire_attenuation(shooter.id, target.id)
                .min(FULL_ATTENUATION);
            urad = urad.saturating_add(term_urad(
                terms.beam_attenuation,
                attenuation.saturating_mul(PERMILLE) / FULL_ATTENUATION,
            ));
        }
        urad = urad.saturating_add(term_urad(
            terms.shooter_speed,
            speed_permille(shooter.speed_cms),
        ));
        urad = urad.saturating_add(term_urad(
            terms.target_lateral_speed,
            speed_permille(lateral_cms(shooter, target)),
        ));
        urad = urad.saturating_add(term_urad(
            terms.gunnery,
            u32::from(STOCK_GUNNERY.max(1)).saturating_mul(PERMILLE)
                / u32::from(shooter.gunnery.max(1)),
        ));
        urad = urad.saturating_add(term_urad(
            terms.sensor_condition,
            FULL_CONDITION.saturating_sub(sight_condition(shooter)) * PERMILLE / FULL_CONDITION,
        ));
        u32::try_from(urad).unwrap_or(u32::MAX).max(MIN_ERROR_URAD)
    }

    /// **Doc 16 §3.5 step 4**: how many rounds of a shot land.
    ///
    /// A single round is the `hit` draw against `p` — *`u = draw(seed,
    /// hit:<shot_seq>) mod 1000`; hit if `u < p_permille`* — and a burst of `k` is
    /// *one binomial sample from `(k, p)` using `burst:<shot_seq>`*. The single
    /// round is not also drawn from `burst`: for `k = 1` the two are the same
    /// question, and asking it twice would put two labels on one round.
    ///
    /// **The binomial from one 64-bit draw.** `burst` is drawn once and then walked
    /// through [`splitmix64`] one step per round, each step's value taken `% 1000`
    /// and compared against `p`. That is `k` independent Bernoulli trials — the mix
    /// is a bijection with good avalanche, so successive states are uncorrelated —
    /// out of a single labelled draw, which is what keeps doc 16 §3.5's pinned
    /// label set at five kinds however long a burst gets. The walk is deterministic
    /// and platform-independent for the same reason the draw itself is
    /// ([`crate::arena::rng`]).
    #[must_use]
    pub fn rounds_landed(&self, shot_seq: u64, rounds: u32, p_permille: u32) -> u32 {
        if p_permille == 0 || rounds == 0 {
            return 0;
        }
        if rounds == 1 {
            let u = u32::from(draw_permille(self.seed, &DrawLabel::Hit { shot_seq }));
            return u32::from(u < p_permille);
        }
        let mut state = draw(self.seed, &DrawLabel::Burst { shot_seq });
        let mut landed = 0_u32;
        for _ in 0..rounds {
            state = splitmix64(state);
            if u32::try_from(state % u64::from(PERMILLE)).unwrap_or(0) < p_permille {
                landed = landed.saturating_add(1);
            }
        }
        landed
    }

    /// **The draw one shot's outcome is keyed on**, and the label the ledger
    /// records beside it (doc 16 §4: *carrying the draw label and value where a
    /// draw occurred*).
    ///
    /// A burst records its `burst` draw and a single round its `hit` draw, which is
    /// exactly the one [`Arena::rounds_landed`] read.
    #[must_use]
    pub fn shot_draw(&self, shot_seq: u64, rounds: u32) -> (String, u64) {
        let label = if rounds > 1 {
            DrawLabel::Burst { shot_seq }
        } else {
            DrawLabel::Hit { shot_seq }
        };
        (label.encode(), draw(self.seed, &label))
    }

    /// **Re-resolve a shot whose target turned in flight** (doc 16 §3.5: *a target
    /// that changed facet band re-resolves with the same `hit` draw*).
    ///
    /// Only the angular size is recomputed, from the new band's authored width at
    /// the **acceptance** range and through the **acceptance** cone: the shooter's
    /// speed, the gunner and the sight are facts about the moment the round left,
    /// and the range is the one the round was aimed at. The draw is not re-made at
    /// all — it is keyed on the shot's sequence, so reading it again is reading the
    /// same number (that is the whole reason doc 16 §3.5 keys draws on labels
    /// rather than on a stream position).
    #[must_use]
    pub fn reresolve(
        &self,
        target: ObjId,
        taken: &Resolution,
        band: &str,
        range_cm: u64,
    ) -> Resolution {
        let Some(at) = self.unit(target) else {
            return Resolution::blind(band, taken.error_urad, taken.cover);
        };
        let Some(silhouette) = at.profile.band(band) else {
            return Resolution::blind(band, taken.error_urad, taken.cover);
        };
        let scale = cover_permille(taken.cover);
        let width = u64::from(silhouette.width_cm)
            .saturating_mul(u64::from(scale))
            .saturating_div(u64::from(PERMILLE));
        let presented_width_cm = u32::try_from(width).unwrap_or(u32::MAX);
        if presented_width_cm == 0 {
            return Resolution::blind(band, taken.error_urad, taken.cover);
        }
        let angular_urad = u32::try_from(
            u64::from(presented_width_cm)
                .saturating_mul(1_000_000)
                .saturating_div(range_cm.max(1)),
        )
        .unwrap_or(u32::MAX);
        let (x_permille, p_permille) = self.hit_permille(angular_urad, taken.error_urad);
        Resolution {
            band: SmolStr::new(band),
            presented_width_cm,
            presented_area_cm2: width.saturating_mul(u64::from(silhouette.height_cm)),
            angular_urad,
            error_urad: taken.error_urad,
            x_permille,
            p_permille,
            cover: taken.cover,
        }
    }

    /// **Doc 16 §3.5 step 5**: the chance the platform's own lock would be hit at,
    /// per mille, for the weapon with the best of it — the number `LOCK` and
    /// `WEAPONSTATUS` print *before any shot*.
    ///
    /// `None` with no lock, with a lock whose contact is no longer held, or on a
    /// platform carrying nothing. It traces, exactly as the shot will: what the
    /// pilot is shown is what a shot taken now would be resolved at, cover and all.
    #[must_use]
    pub fn lock_p_permille(&self, platform: ObjId) -> Option<u32> {
        let state = self.unit(platform)?;
        let contact = state.lock?;
        let target = self.contacts.resolve(platform, contact)?.target;
        let at = self.unit(target)?;
        let range = range_cm(&state.pose, &at.pose);
        let cover = self.fire_cover(platform, target);
        state
            .weapons
            .iter()
            .map(|weapon| {
                self.resolve_shot(platform, target, &weapon.id, range, cover)
                    .p_permille
            })
            .max()
    }
}

/// One error term in microradians: `weight × units / 1000`, where both are per
/// mille and one whole unit is [`TERM_URAD`].
fn term_urad(weight_permille: u32, units_permille: u32) -> u64 {
    u64::from(weight_permille)
        .saturating_mul(u64::from(units_permille))
        .saturating_mul(u64::from(TERM_URAD))
        .saturating_div(u64::from(PERMILLE).saturating_mul(u64::from(PERMILLE)))
}

/// A speed as a multiple of [`REFERENCE_SPEED_CMS`], per mille. Reverse counts as
/// motion: a tank backing away at 7 m/s is shaking its gun exactly as much as one
/// driving forward at 7 m/s.
fn speed_permille(speed_cms: i32) -> u32 {
    let magnitude = u64::from(speed_cms.unsigned_abs());
    u32::try_from(
        magnitude
            .saturating_mul(u64::from(PERMILLE))
            .saturating_div(u64::from(REFERENCE_SPEED_CMS.max(1))),
    )
    .unwrap_or(u32::MAX)
}

/// **The target's speed across the line of fire**, cm/s and never negative.
///
/// The target's velocity is `speed_cms` along its heading; the line of fire runs
/// along the bearing from the shooter to it. The component across that line is
/// `speed × sin(heading − bearing)`, taken through the arena's own fixed-point
/// sine ([`sin_fx`]) so the figure is an integer a replay reproduces. A target
/// driving straight at or away from the shooter contributes nothing, which is the
/// whole point of the term.
fn lateral_cms(shooter: &UnitState, target: &UnitState) -> i32 {
    let bearing = bearing_mdeg(&shooter.pose, &target.pose);
    let across = (target.pose.heading_mdeg + FULL_MDEG).saturating_sub(bearing) % FULL_MDEG;
    let scaled = i64::from(target.speed_cms)
        .saturating_mul(sin_fx(across))
        .saturating_div(SIN_ONE);
    i32::try_from(scaled.abs()).unwrap_or(i32::MAX)
}

/// **The shooter's sight condition**, 0 to [`FULL_CONDITION`].
///
/// The `condition` cell of the **first component, in component-id order, that has
/// one**, read off the M32 unit. For doc 16 §3.3's tank that is exactly the
/// `visual_sensor`: a weapon component has `ammo` and `ready_at_ms`, the power plant
/// has no cell at all, and the sight is the only thing with a `condition`
/// (`crate::definitions::unit::bind` is what gives each role its cells). The arena
/// cannot ask *which* component is the sight — the seam carries the sensor's
/// numbers, not the component id it came from — so a unit whose content gave a
/// second component a `condition` cell would be read through whichever id sorts
/// first. Carrying the sensor component's id across the seam is the clean fix and is
/// a field nobody needs yet.
///
/// A unit with no such cell — every cut-A fixture — reads as **full**: the term is
/// *how much a damaged sensor widens the cone*, and a unit that has no sensor to
/// damage is not shooting worse for the lack of one (what it cannot do is hold a
/// contact, which is `FIRE`'s own refusal).
fn sight_condition(shooter: &UnitState) -> u32 {
    let Ok(cell) = mush_tactics::CellId::new(crate::definitions::unit::CONDITION_CELL) else {
        return FULL_CONDITION;
    };
    shooter
        .unit
        .components()
        .find_map(|(component, _)| {
            shooter
                .unit
                .cell(&mush_tactics::CellAddr::Component {
                    component: component.clone(),
                    cell: cell.clone(),
                })
                .map(|scalar| scalar.current)
        })
        .map_or(FULL_CONDITION, |current| {
            u32::try_from(current.clamp(0, i64::from(FULL_CONDITION))).unwrap_or(FULL_CONDITION)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::{Power, Team};
    use crate::arena::test_support::{arena_of, cannon, line_tank, pose, scout};

    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);

    /// Two tanks 1,000 cm apart on the same row, both carrying the stock
    /// silhouettes and facet table.
    fn duel() -> Arena {
        let mut arena = arena_of(vec![
            line_tank(RED, Team::Red, pose(1_500, 1_500)),
            line_tank(BLUE, Team::Blue, pose(1_500, 2_500)),
        ]);
        for id in [RED, BLUE] {
            if let Some(state) = arena.unit_mut(id) {
                state.power = Power::Online;
            }
        }
        arena
    }

    #[test]
    fn the_band_is_the_sector_the_shooter_stands_in_relative_to_the_targets_heading() {
        let arena = duel();
        let red = arena.unit(RED).unwrap_or_else(|| panic!("red"));
        let blue = arena.unit(BLUE).unwrap_or_else(|| panic!("blue"));
        // Blue faces north (0 mdeg) and red is due south of it: a rear hit.
        assert_eq!(facet_band(blue, red), "rear");
        // Red faces north and blue is due north of it: a front hit.
        assert_eq!(facet_band(red, blue), "front");
        // Turn blue to face east: red is now off its right quarter... no, red is
        // due south, which from an east-facing tank is 90° to the right of dead
        // ahead's opposite — the relative bearing of red seen from blue is 180°
        // less blue's heading.
        let mut arena = duel();
        if let Some(state) = arena.unit_mut(BLUE) {
            state.pose.heading_mdeg = 90_000;
        }
        let red = arena.unit(RED).unwrap_or_else(|| panic!("red"));
        let blue = arena.unit(BLUE).unwrap_or_else(|| panic!("blue"));
        assert_eq!(facet_band(blue, red), "right");
        if let Some(state) = arena.unit_mut(BLUE) {
            state.pose.heading_mdeg = 270_000;
        }
        let red = arena.unit(RED).unwrap_or_else(|| panic!("red"));
        let blue = arena.unit(BLUE).unwrap_or_else(|| panic!("blue"));
        assert_eq!(facet_band(blue, red), "left");
    }

    #[test]
    fn cover_scales_the_presented_width_and_full_cover_presents_nothing() {
        let arena = duel();
        let open = arena.resolve_shot(RED, BLUE, "cannon", 100_000, Cover::None);
        let partial = arena.resolve_shot(RED, BLUE, "cannon", 100_000, Cover::Partial);
        let full = arena.resolve_shot(RED, BLUE, "cannon", 100_000, Cover::Full);
        // The rear silhouette is 350 cm wide (the stock tank's).
        assert_eq!(open.presented_width_cm, 350);
        assert_eq!(partial.presented_width_cm, 175);
        assert_eq!(full.presented_width_cm, 0);
        assert_eq!(open.angular_urad, 3_500, "350 cm at 1 km is 3.5 mrad");
        assert_eq!(partial.angular_urad, 1_750);
        assert_eq!(full.p_permille, 0, "zero presented area is zero");
        assert!(partial.p_permille < open.p_permille);
    }

    #[test]
    fn the_error_cone_is_the_dispersion_plus_the_four_weighted_terms() {
        let mut arena = duel();
        // At rest, full sight, gunnery 1: the band's 3 mrad plus 1 mrad of
        // gunnery.
        let still = arena.resolve_shot(RED, BLUE, "cannon", 100_000, Cover::None);
        assert_eq!(still.error_urad, 4_000);
        assert_eq!(still.x_permille, 875, "3500 x 1000 / 4000");
        // A moving shooter widens it: 1,000 cm/s is one reference unit at weight
        // 1,000, so one more milliradian.
        if let Some(state) = arena.unit_mut(RED) {
            state.speed_cms = 1_000;
        }
        let moving = arena.resolve_shot(RED, BLUE, "cannon", 100_000, Cover::None);
        assert_eq!(moving.error_urad, 5_000);
        assert!(moving.p_permille < still.p_permille);
        // An ace gunner narrows it: 1,000 / 5 is 200 µrad instead of 1,000.
        if let Some(state) = arena.unit_mut(RED) {
            state.speed_cms = 0;
            state.gunnery = 5;
        }
        let ace = arena.resolve_shot(RED, BLUE, "cannon", 100_000, Cover::None);
        assert_eq!(ace.error_urad, 3_200);
        assert!(ace.p_permille > still.p_permille);
        // The outer band is wider still (6 mrad at 2 km), which is the content's
        // own worked example.
        let far = arena.resolve_shot(RED, BLUE, "cannon", 200_000, Cover::None);
        assert_eq!(far.angular_urad, 1_750);
        assert!(far.p_permille < still.p_permille);
    }

    #[test]
    fn a_target_crossing_the_line_of_fire_widens_the_cone_and_one_driving_along_it_does_not() {
        let mut arena = duel();
        // Blue is due north of red, so a heading of 90° (east) is straight across
        // the line of fire and 0° (north) is straight along it.
        if let Some(state) = arena.unit_mut(BLUE) {
            state.speed_cms = 1_000;
            state.pose.heading_mdeg = 0;
        }
        let along = arena.resolve_shot(RED, BLUE, "cannon", 100_000, Cover::None);
        if let Some(state) = arena.unit_mut(BLUE) {
            state.pose.heading_mdeg = 90_000;
        }
        let across = arena.resolve_shot(RED, BLUE, "cannon", 100_000, Cover::None);
        assert_eq!(along.error_urad, 4_000, "nothing across the line");
        assert_eq!(across.error_urad, 4_800, "one unit at weight 800");
    }

    #[test]
    fn a_unit_with_no_authored_silhouette_presents_nothing() {
        let arena = arena_of(vec![
            scout(RED, Team::Red, pose(1_500, 1_500)),
            scout(BLUE, Team::Blue, pose(1_500, 2_500)),
        ]);
        let taken = arena.resolve_shot(RED, BLUE, "cannon", 1_000, Cover::None);
        assert_eq!(taken.presented_width_cm, 0);
        assert_eq!(taken.p_permille, 0);
        // And the cone is still the real one, so the record says what was aimed.
        assert_eq!(taken.error_urad, 4_000);
        assert_eq!(
            cannon().bands.first().map(|band| band.dispersion_mrad),
            Some(3)
        );
    }

    #[test]
    fn a_burst_is_one_draw_walked_once_per_round_and_a_single_round_is_the_hit_draw() {
        let arena = duel();
        // A certainty and an impossibility bracket the walk.
        assert_eq!(arena.rounds_landed(1, 10, 1_000), 10);
        assert_eq!(arena.rounds_landed(1, 10, 0), 0);
        assert_eq!(arena.rounds_landed(1, 0, 1_000), 0);
        // A middling probability lands some of them, and the same shot always the
        // same ones.
        let first = arena.rounds_landed(7, 10, 500);
        assert_eq!(first, arena.rounds_landed(7, 10, 500));
        assert!(first <= 10);
        // A single round is the `hit` draw, and the label the ledger records says
        // which question was asked.
        let u = u32::from(draw_permille(arena.seed, &DrawLabel::Hit { shot_seq: 3 }));
        assert_eq!(arena.rounds_landed(3, 1, u.saturating_add(1)), 1);
        assert_eq!(arena.rounds_landed(3, 1, u), 0, "hit if u < p, strictly");
        assert!(arena.shot_draw(3, 1).0.starts_with("hit:"));
        assert!(arena.shot_draw(3, 4).0.starts_with("burst:"));
    }

    #[test]
    fn re_resolving_a_turned_target_reads_the_new_band_against_the_same_cone() {
        let arena = duel();
        let rear = arena.resolve_shot(RED, BLUE, "cannon", 100_000, Cover::None);
        assert_eq!(rear.band, "rear");
        // The flank is 700 cm across where the rear is 350: twice the angular
        // size, against the cone the shot was taken with.
        let flank = arena.reresolve(BLUE, &rear, "left", 100_000);
        assert_eq!(flank.presented_width_cm, 700);
        assert_eq!(flank.angular_urad, 7_000);
        assert_eq!(flank.error_urad, rear.error_urad);
        assert!(flank.p_permille > rear.p_permille);
    }

    #[test]
    fn a_damaged_sight_widens_the_cone() {
        let mut arena = duel();
        let healthy = arena.resolve_shot(RED, BLUE, "cannon", 100_000, Cover::None);
        // The `condition` cell of the tank's own sensor component, taken to zero.
        let addr = mush_tactics::CellAddr::Component {
            component: mush_tactics::ComponentId::new("visual_sensor")
                .unwrap_or_else(|_| panic!("a component id")),
            cell: mush_tactics::CellId::new(crate::definitions::unit::CONDITION_CELL)
                .unwrap_or_else(|_| panic!("a cell id")),
        };
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetCell { addr, value: 0 }],
            None,
            None,
        )
        .unwrap_or_else(|_| panic!("a one-op plan"));
        if let Some(state) = arena.unit_mut(RED) {
            let (post, _) = mush_tactics::apply(&state.unit, &plan, 0)
                .unwrap_or_else(|error| panic!("the sight is dimmed: {error}"));
            state.unit = post;
        }
        let blind = arena.resolve_shot(RED, BLUE, "cannon", 100_000, Cover::None);
        assert_eq!(healthy.error_urad, 4_000);
        assert_eq!(blind.error_urad, 4_500, "500 µrad at weight 500");
    }
}
