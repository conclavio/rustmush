//! **The hit table** (16b cut C.2, C1; doc 16 §3.5 "Resolution", §4 "Content
//! packs"; D-16-8).
//!
//! Doc 16 §3.5 step 3: *`p_permille = hit_curve(angular_size_mrad × 1000 /
//! error_mrad)`, a named monotone table in config with interpolation, clamped to
//! `[resolution.p_min, resolution.p_max]`*. This is that table, and the
//! per-mille weights of step 2 beside it.
//!
//! Everything is per-mille, so the whole resolution is integer arithmetic
//! (D-16-4) and the cockpit's percentage in `WEAPONSTATUS` is a render of the
//! same number the draw compares against.

use serde::{Deserialize, Serialize};

use crate::definitions::DefinitionError;

/// One in a thousand is the unit of every probability in the arena.
pub const PERMILLE: u32 = 1_000;

/// The most points one hit curve may carry. A curve is read once per shot and
/// walked linearly; thirty-two points is a smooth curve and a bounded walk.
pub const MAX_HIT_CURVE_POINTS: usize = 32;

/// The largest ratio a curve may name on its x axis.
///
/// `x` is `angular_size_mrad × 1000 / error_mrad`, so it is a per-mille ratio of
/// two milliradian figures and a million is a target a thousand times wider than
/// the error cone — past any shot the stock rules take.
pub const MAX_HIT_CURVE_X: u32 = 1_000_000;

/// The largest weight one error term may carry, per mille. Ten is a term that
/// multiplies the cone by ten.
pub const MAX_ERROR_WEIGHT: u32 = 10_000;

/// **What a beam's attenuation is worth when a table does not say** (16b cut
/// X.1; doc 16 §9 item 1), per mille.
///
/// The same ×1.0 `shooter_speed` and `gunnery` carry, so a beam that reaches its
/// target through half-obscuring ground adds half a milliradian to the cone.
///
/// It has a default because the stock game has no beam: `content/resolution.json`
/// was authored for four terms and a fifth key in it would move its digest and
/// the pin that guards it, for a number every stock shot multiplies by zero. A
/// table that wants another weight authors one; one that says nothing gets this,
/// and — because the field is skipped when it equals this — a table that says
/// nothing serialises to exactly the four keys it always did.
pub const DEFAULT_BEAM_ATTENUATION: u32 = 1_000;

/// The per-mille weights of doc 16 §3.5 step 2's named terms.
///
/// Each is a **weight**, not a value: cut W multiplies the term's own measured
/// quantity (the shooter's speed, the target's lateral speed, the gunnery rating
/// through the `PilotProfile` seam, the sensor's condition cell, and — 16b cut
/// X.1, for a beam alone — the attenuation of the ground its line crosses) by
/// the weight and adds the result to the weapon's band dispersion. Cut C authors
/// the weights and reads none of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorTerms {
    /// How much the shooter's own speed widens the cone.
    pub shooter_speed: u32,
    /// How much the target's speed across the line of fire widens it.
    pub target_lateral_speed: u32,
    /// How much the gunner's rating narrows or widens it. Every stock rating is
    /// 1 through the `PilotProfile` seam (doc 16 §3.5 step 2).
    pub gunnery: u32,
    /// How much a damaged sensor widens it.
    pub sensor_condition: u32,
    /// **How much the ground a beam crosses widens it** (16b cut X.1; doc 16 §9
    /// item 1: *resolution uses `beam_path` attenuation as a cover term*).
    ///
    /// Measured against a **fully** attenuated line: a beam that loses half of
    /// itself on the way adds half this weight's worth. It is zero for every
    /// direct-fire weapon, because the term is a beam's alone, and
    /// [`DEFAULT_BEAM_ATTENUATION`] for a table that does not name it.
    #[serde(
        default = "default_beam_attenuation",
        skip_serializing_if = "is_default_beam_attenuation"
    )]
    pub beam_attenuation: u32,
}

/// The default of [`ErrorTerms::beam_attenuation`], for `serde`.
const fn default_beam_attenuation() -> u32 {
    DEFAULT_BEAM_ATTENUATION
}

/// Whether the authored weight is the default, so an unauthored one is not
/// written back out (see [`DEFAULT_BEAM_ATTENUATION`]).
fn is_default_beam_attenuation(value: &u32) -> bool {
    *value == DEFAULT_BEAM_ATTENUATION
}

/// One resolution table (doc 16 §3.5).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionDef {
    /// The human-facing name.
    pub label: String,
    /// `[[x_permille, p_permille], ..]`, x strictly increasing and p never
    /// falling: the curve is monotone by validation, not by hope.
    pub hit_curve: Vec<[u32; 2]>,
    /// The weights of step 2.
    pub error_terms: ErrorTerms,
    /// The floor a resolved probability is clamped to, per mille.
    pub p_min: u32,
    /// The ceiling, per mille.
    pub p_max: u32,
    /// Why these numbers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl ResolutionDef {
    /// The curve's value at `x`, linearly interpolated and clamped to
    /// `[p_min, p_max]` — doc 16 §3.5 step 3, as a pure function of the table.
    ///
    /// Below the first point the first point's value holds; above the last, the
    /// last's. Cut W is the caller; cut C authors the table and proves the
    /// walk here so the shipped curve has a reader before it has a consumer.
    #[must_use]
    pub fn hit_permille(&self, x: u32) -> u32 {
        let mut answer = self.hit_curve.first().map_or(0, |point| point[1]);
        let mut previous: Option<[u32; 2]> = None;
        for point in &self.hit_curve {
            if x >= point[0] {
                answer = point[1];
                previous = Some(*point);
                continue;
            }
            if let Some(low) = previous {
                // Integer interpolation, and integer all the way down (D-16-4):
                // every term is at most a million by the curve's own bounds, so
                // nothing here can leave `i64`.
                let span = i64::from(point[0].saturating_sub(low[0])).max(1);
                let into = i64::from(x.saturating_sub(low[0]));
                let rise = i64::from(point[1]) - i64::from(low[1]);
                let step = rise.saturating_mul(into) / span;
                answer = u32::try_from(i64::from(low[1]).saturating_add(step)).unwrap_or(low[1]);
            }
            break;
        }
        answer.clamp(self.p_min, self.p_max)
    }

    /// Re-run every bound: the curve's shape, its monotonicity, and the clamps.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry and what is wrong.
    pub fn check(&self, entry: &str) -> Result<(), DefinitionError> {
        crate::definitions::check_label(entry, &self.label)?;
        crate::definitions::check_note(entry, &self.note)?;
        if self.hit_curve.len() < 2 || self.hit_curve.len() > MAX_HIT_CURVE_POINTS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "a hit curve is 2 to {MAX_HIT_CURVE_POINTS} points; {} is not",
                    self.hit_curve.len()
                ),
            ));
        }
        // `0 <= p_min <= p_max <= 1000`, the brief's clamp rule, checked before
        // the curve so a curve inside a nonsense window is not reported as the
        // curve's fault.
        if self.p_min > self.p_max || self.p_max > PERMILLE {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "the clamps are 0 <= p_min <= p_max <= {PERMILLE}; {} and {} are not",
                    self.p_min, self.p_max
                ),
            ));
        }
        let mut last: Option<[u32; 2]> = None;
        for point in &self.hit_curve {
            if point[0] > MAX_HIT_CURVE_X {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "a hit curve's x is at most {MAX_HIT_CURVE_X}; {} is not",
                        point[0]
                    ),
                ));
            }
            if point[1] > PERMILLE {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "a hit curve's probability is at most {PERMILLE} per mille; {} is not",
                        point[1]
                    ),
                ));
            }
            if let Some(previous) = last {
                if point[0] <= previous[0] {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "a hit curve's x strictly increases; {} after {} does not",
                            point[0], previous[0]
                        ),
                    ));
                }
                if point[1] < previous[1] {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "a hit curve never falls (doc 16 §3.5 step 3: a monotone table); \
                             {} after {} does",
                            point[1], previous[1]
                        ),
                    ));
                }
            }
            last = Some(*point);
        }
        for (name, weight) in [
            ("shooter_speed", self.error_terms.shooter_speed),
            (
                "target_lateral_speed",
                self.error_terms.target_lateral_speed,
            ),
            ("gunnery", self.error_terms.gunnery),
            ("sensor_condition", self.error_terms.sensor_condition),
            ("beam_attenuation", self.error_terms.beam_attenuation),
        ] {
            if weight > MAX_ERROR_WEIGHT {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "error_terms.{name} is at most {MAX_ERROR_WEIGHT} per mille; \
                         {weight} is not"
                    ),
                ));
            }
        }
        Ok(())
    }
}
