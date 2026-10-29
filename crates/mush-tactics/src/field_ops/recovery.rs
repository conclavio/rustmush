//! `H2-A` — Universal tow coupling, field inspection, tonnage-differential speed
//! scaling, carrier bay capacity checks, and durable recovery operation state
//! (design doc 27 §4.1, §5, `FO-1`–`FO-3`, `FO-13`, `Q-S5`–`Q-S9`, `Q-S11`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::field_ops::condition::{
    FieldCloseOutcome, FieldUnitAssessment, OperationalState, validate_close_outcome,
};
use crate::unit::TacticalUnit;

/// Maximum inclusive casualty-to-tug mass ratio in permille (`2_000` = `2.0×`).
pub const MAX_TOW_MASS_RATIO_PERMILLE: u64 = 2_000;

/// Default maximum physical attachment or bay-loading reach in centimeters (`10 m`).
pub const DEFAULT_MAX_ATTACH_DISTANCE_CM: u64 = 1_000;

/// Maximum recovery operations retained in one [`RecoveryBook`].
pub const MAX_RECOVERY_OPERATIONS: usize = 1_024;

/// Physical mass, cargo, and dimensional envelope of a field unit (`FO-3`, `FO-13`, `Q-S8`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UnitMassProfileWire", into = "UnitMassProfileWire")]
pub struct UnitMassProfile {
    dry_mass_kg: u64,
    cargo_mass_kg: u64,
    cargo_volume_l: u64,
    width_cm: u32,
    height_cm: u32,
}

impl UnitMassProfile {
    /// Construct a validated [`UnitMassProfile`].
    ///
    /// # Errors
    /// Returns [`RecoveryError`] if `dry_mass_kg == 0`, dimensions are `0`, or
    /// `dry_mass_kg + cargo_mass_kg` overflows `u64`.
    pub fn new(
        dry_mass_kg: u64,
        cargo_mass_kg: u64,
        cargo_volume_l: u64,
        width_cm: u32,
        height_cm: u32,
    ) -> Result<Self, RecoveryError> {
        if dry_mass_kg == 0 {
            return Err(RecoveryError::ZeroDryMass);
        }
        if width_cm == 0 || height_cm == 0 {
            return Err(RecoveryError::ZeroDimensions {
                width_cm,
                height_cm,
            });
        }
        let _ = dry_mass_kg
            .checked_add(cargo_mass_kg)
            .ok_or(RecoveryError::MassOverflow)?;
        Ok(Self {
            dry_mass_kg,
            cargo_mass_kg,
            cargo_volume_l,
            width_cm,
            height_cm,
        })
    }

    /// Dry chassis mass in kilograms (`> 0`).
    #[must_use]
    pub const fn dry_mass_kg(&self) -> u64 {
        self.dry_mass_kg
    }

    /// Retained onboard cargo mass in kilograms.
    #[must_use]
    pub const fn cargo_mass_kg(&self) -> u64 {
        self.cargo_mass_kg
    }

    /// Retained onboard cargo volume in liters.
    #[must_use]
    pub const fn cargo_volume_l(&self) -> u64 {
        self.cargo_volume_l
    }

    /// Hull width in centimeters.
    #[must_use]
    pub const fn width_cm(&self) -> u32 {
        self.width_cm
    }

    /// Hull height in centimeters.
    #[must_use]
    pub const fn height_cm(&self) -> u32 {
        self.height_cm
    }

    /// Total laden mass (`dry_mass_kg + cargo_mass_kg`) in kilograms.
    #[must_use]
    pub const fn total_mass_kg(&self) -> u64 {
        self.dry_mass_kg.saturating_add(self.cargo_mass_kg)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnitMassProfileWire {
    dry_mass_kg: u64,
    cargo_mass_kg: u64,
    cargo_volume_l: u64,
    width_cm: u32,
    height_cm: u32,
}

impl TryFrom<UnitMassProfileWire> for UnitMassProfile {
    type Error = RecoveryError;

    fn try_from(wire: UnitMassProfileWire) -> Result<Self, Self::Error> {
        Self::new(
            wire.dry_mass_kg,
            wire.cargo_mass_kg,
            wire.cargo_volume_l,
            wire.width_cm,
            wire.height_cm,
        )
    }
}

impl From<UnitMassProfile> for UnitMassProfileWire {
    fn from(profile: UnitMassProfile) -> Self {
        Self {
            dry_mass_kg: profile.dry_mass_kg,
            cargo_mass_kg: profile.cargo_mass_kg,
            cargo_volume_l: profile.cargo_volume_l,
            width_cm: profile.width_cm,
            height_cm: profile.height_cm,
        }
    }
}

/// Towing rig profile of the towing unit (`FO-13`, `Q-S6`).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum TowRigProfile {
    /// Universal baseline tow coupling available to any mobile unit.
    #[default]
    Standard,
    /// Dedicated recovery vehicle or heavy tow-rig optimization (`FO-13`, `Q-S6`).
    RecoveryOptimized,
}

/// Compute the casualty-to-tug mass ratio in permille (`1_000` = `1.0×`, `2_000` = `2.0×`)
/// after validating the inclusive `2×` tonnage ceiling (`FO-13`, `Q-S6`).
///
/// # Errors
/// Returns [`RecoveryError::ZeroDryMass`] if either mass is `0`, or
/// [`RecoveryError::ExceedsTonnageCap`] if `casualty_mass_kg > tug_mass_kg * 2`.
pub fn tow_mass_ratio_permille(
    tug_mass_kg: u64,
    casualty_mass_kg: u64,
) -> Result<u64, RecoveryError> {
    if tug_mass_kg == 0 || casualty_mass_kg == 0 {
        return Err(RecoveryError::ZeroDryMass);
    }
    let max_casualty_mass_kg = tug_mass_kg
        .checked_mul(2)
        .ok_or(RecoveryError::MassOverflow)?;
    if casualty_mass_kg > max_casualty_mass_kg {
        return Err(RecoveryError::ExceedsTonnageCap {
            tug_mass_kg,
            casualty_mass_kg,
            max_casualty_mass_kg,
        });
    }
    let scaled = casualty_mass_kg
        .checked_mul(1_000)
        .ok_or(RecoveryError::MassOverflow)?;
    Ok(scaled / tug_mass_kg)
}

/// Deterministic speed factor in permille (`1..=1_000`) for a tug towing a casualty (`FO-13`, `Q-S6`).
///
/// # Errors
/// Returns [`RecoveryError`] if masses are zero or `casualty_mass_kg > tug_mass_kg * 2`.
pub fn tow_speed_factor_permille(
    tug_mass_kg: u64,
    casualty_mass_kg: u64,
    profile: TowRigProfile,
) -> Result<u32, RecoveryError> {
    let ratio_permille = tow_mass_ratio_permille(tug_mass_kg, casualty_mass_kg)?;
    let factor = match profile {
        TowRigProfile::Standard => 850_u64.saturating_sub((ratio_permille * 300) / 1_000),
        TowRigProfile::RecoveryOptimized => 950_u64.saturating_sub((ratio_permille * 225) / 1_000),
    };
    u32::try_from(factor.max(1)).map_err(|_| RecoveryError::MassOverflow)
}

/// Compute the coupled tow speed in `cm/s` from the tug's `base_speed_cms`, tug mass,
/// casualty mass, and [`TowRigProfile`] (`FO-13`, `Q-S6`).
///
/// # Errors
/// Returns [`RecoveryError`] if `base_speed_cms == 0`, masses are `0`, or
/// `casualty_mass_kg > tug_mass_kg * 2`.
pub fn scaled_tow_speed_cms(
    base_speed_cms: u32,
    tug_mass_kg: u64,
    casualty_mass_kg: u64,
    profile: TowRigProfile,
) -> Result<u32, RecoveryError> {
    if base_speed_cms == 0 {
        return Err(RecoveryError::ZeroTugBaseSpeed);
    }
    let factor_permille = u64::from(tow_speed_factor_permille(
        tug_mass_kg,
        casualty_mass_kg,
        profile,
    )?);
    let scaled = (u64::from(base_speed_cms) * factor_permille) / 1_000;
    u32::try_from(scaled.max(1)).map_err(|_| RecoveryError::MassOverflow)
}

/// Field surface position of a unit (`site` + centimeter coordinates).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldPose {
    /// Surface site/region object id.
    pub site: ObjId,
    /// X coordinate in centimeters.
    pub x_cm: i64,
    /// Y coordinate in centimeters.
    pub y_cm: i64,
}

impl FieldPose {
    /// Validate that `site` is non-zero.
    ///
    /// # Errors
    /// Returns [`RecoveryError::InvalidObjectId`] if `site.0 == 0`.
    pub const fn validate(&self) -> Result<(), RecoveryError> {
        if self.site.0 == 0 {
            return Err(RecoveryError::InvalidObjectId("site"));
        }
        Ok(())
    }

    /// Integer Euclidean distance in centimeters to `other` if on the same `site`,
    /// or `None` if on different sites.
    #[must_use]
    pub fn distance_cm_to(&self, other: &Self) -> Option<u64> {
        if self.site != other.site {
            return None;
        }
        let dx = i128::from(self.x_cm) - i128::from(other.x_cm);
        let dy = i128::from(self.y_cm) - i128::from(other.y_cm);
        let dist_sq = dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy));
        let root = isqrt_i128(dist_sq);
        u64::try_from(root).ok()
    }
}

const fn isqrt_i128(n: i128) -> i128 {
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Prospective tug parameters for [`inspect_field_unit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TugInspectionContext {
    /// Tug unit object id.
    pub tug: ObjId,
    /// Tug operational state.
    pub tug_state: OperationalState,
    /// Whether the tug can move under its own power.
    pub tug_can_move: bool,
    /// Tug base speed in `cm/s`.
    pub tug_base_speed_cms: u32,
    /// Tug total mass in kilograms.
    pub tug_mass_kg: u64,
    /// Tug rig profile.
    pub rig_profile: TowRigProfile,
    /// Tug field pose.
    pub tug_pose: FieldPose,
    /// Maximum attach distance in centimeters.
    pub max_attach_distance_cm: u64,
}

/// Tow feasibility evaluation included in a [`FieldInspectionReport`] (`Q-S5`, `FO-13`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TowEvaluation {
    /// Inspecting tug object id.
    pub tug: ObjId,
    /// Whether the tug can immediately attach and tow this casualty.
    pub eligible: bool,
    /// Tug total mass in kilograms.
    pub tug_mass_kg: u64,
    /// Casualty total mass (`dry + cargo`) in kilograms.
    pub casualty_mass_kg: u64,
    /// Maximum casualty mass this tug can tow (`2 * tug_mass_kg`).
    pub max_casualty_mass_kg: u64,
    /// Casualty-to-tug mass ratio in permille (`1_000` = `1.0×`), if `tug_mass_kg > 0`.
    pub mass_ratio_permille: Option<u64>,
    /// Resulting coupled tow speed in `cm/s` if eligible.
    pub coupled_speed_cms: Option<u32>,
    /// Distance between tug and casualty in centimeters, if on the same site.
    pub distance_cm: Option<u64>,
    /// Refusal code if `eligible == false`.
    pub refusal_reason: Option<SmolStr>,
}

/// Structured field inspection report for a target unit (`Q-S5`, `FO-1`–`FO-3`, `FO-13`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldInspectionReport {
    /// Target unit object id.
    pub target: ObjId,
    /// Target field pose.
    pub pose: FieldPose,
    /// Derived operational state and section/component survival breakdown.
    pub assessment: FieldUnitAssessment,
    /// Target chassis dry mass, retained cargo mass/volume, and dimensions.
    pub mass: UnitMassProfile,
    /// Optional tow feasibility evaluation for a prospective tug.
    pub tow_evaluation: Option<TowEvaluation>,
}

/// Inspect a field unit's operational state, surviving components, retained cargo,
/// and (optionally) tow feasibility for an inspecting tug (`Q-S5`).
#[must_use]
pub fn inspect_field_unit(
    assessment: FieldUnitAssessment,
    mass: UnitMassProfile,
    pose: FieldPose,
    tug_ctx: Option<TugInspectionContext>,
    book: Option<&RecoveryBook>,
) -> FieldInspectionReport {
    let target = assessment.unit;
    let tow_evaluation = tug_ctx.map(|ctx| {
        let casualty_mass_kg = mass.total_mass_kg();
        let max_casualty_mass_kg = ctx.tug_mass_kg.saturating_mul(2);
        let mass_ratio_permille =
            (ctx.tug_mass_kg > 0).then(|| casualty_mass_kg.saturating_mul(1_000) / ctx.tug_mass_kg);
        let distance_cm = ctx.tug_pose.distance_cm_to(&pose);

        let refusal = if ctx.tug == target {
            Some("self_tow")
        } else if !ctx.tug_state.is_operational()
            || !ctx.tug_can_move
            || ctx.tug_base_speed_cms == 0
        {
            Some("tug_not_mobile")
        } else if !assessment.state.is_towable() {
            Some(match assessment.state {
                OperationalState::Operational => "target_still_operational",
                OperationalState::Irrecoverable => "target_irrecoverable",
                _ => "target_not_towable",
            })
        } else if book.is_some_and(|b| b.active_operation_for_unit(ctx.tug).is_some()) {
            Some("tug_already_busy")
        } else if book.is_some_and(|b| b.active_operation_for_casualty(target).is_some()) {
            Some("casualty_already_attached")
        } else if distance_cm.is_none() {
            Some("different_site")
        } else if distance_cm.is_some_and(|d| d > ctx.max_attach_distance_cm) {
            Some("out_of_attach_range")
        } else if casualty_mass_kg > max_casualty_mass_kg {
            Some("exceeds_2x_tonnage_cap")
        } else {
            None
        };

        let coupled_speed_cms = if refusal.is_none() {
            scaled_tow_speed_cms(
                ctx.tug_base_speed_cms,
                ctx.tug_mass_kg,
                casualty_mass_kg,
                ctx.rig_profile,
            )
            .ok()
        } else {
            None
        };

        TowEvaluation {
            tug: ctx.tug,
            eligible: refusal.is_none() && coupled_speed_cms.is_some(),
            tug_mass_kg: ctx.tug_mass_kg,
            casualty_mass_kg,
            max_casualty_mass_kg,
            mass_ratio_permille,
            coupled_speed_cms,
            distance_cm,
            refusal_reason: refusal.map(SmolStr::new),
        }
    });

    FieldInspectionReport {
        target,
        pose,
        assessment,
        mass,
        tow_evaluation,
    }
}

/// Destination carrier bay capacity envelope checked when loading a towed/recovered unit (`Q-S8`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayCapacitySpec {
    /// Maximum units the bay slot can hold.
    pub max_units: u32,
    /// Units currently stowed in the bay slot.
    pub current_units: u32,
    /// Maximum total mass (`kg`) the bay/carrier can accept.
    pub max_mass_kg: u64,
    /// Current stowed mass (`kg`) in the bay/carrier.
    pub current_mass_kg: u64,
    /// Maximum cargo/stowed volume (`L`) the bay/carrier can accept.
    pub max_volume_l: u64,
    /// Current stowed volume (`L`) in the bay/carrier.
    pub current_volume_l: u64,
    /// Maximum unit width (`cm`) the bay entrance/cradle accepts.
    pub max_width_cm: u32,
    /// Maximum unit height (`cm`) the bay entrance/cradle accepts.
    pub max_height_cm: u32,
}

impl BayCapacitySpec {
    /// Validate that `casualty` (including its retained cargo mass and volume) fits into this bay (`Q-S8`).
    ///
    /// # Errors
    /// Returns [`RecoveryError`] if unit count, width/height dimensions, total mass, or volume
    /// would exceed the bay's capacity.
    pub fn check_can_load(&self, casualty: &UnitMassProfile) -> Result<(), RecoveryError> {
        if self.current_units >= self.max_units {
            return Err(RecoveryError::BaySlotFull {
                current_units: self.current_units,
                max_units: self.max_units,
            });
        }
        if casualty.width_cm() > self.max_width_cm || casualty.height_cm() > self.max_height_cm {
            return Err(RecoveryError::BayDimensionExceeded {
                unit_width_cm: casualty.width_cm(),
                unit_height_cm: casualty.height_cm(),
                max_width_cm: self.max_width_cm,
                max_height_cm: self.max_height_cm,
            });
        }
        let next_mass = self
            .current_mass_kg
            .checked_add(casualty.total_mass_kg())
            .ok_or(RecoveryError::MassOverflow)?;
        if next_mass > self.max_mass_kg {
            return Err(RecoveryError::BayMassExceeded {
                required_mass_kg: next_mass,
                max_mass_kg: self.max_mass_kg,
            });
        }
        let next_volume = self
            .current_volume_l
            .checked_add(casualty.cargo_volume_l())
            .ok_or(RecoveryError::MassOverflow)?;
        if next_volume > self.max_volume_l {
            return Err(RecoveryError::BayVolumeExceeded {
                required_volume_l: next_volume,
                max_volume_l: self.max_volume_l,
            });
        }
        Ok(())
    }
}

/// Strongly typed identifier for a durable [`RecoveryOperation`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct RecoveryOpId(u64);

impl RecoveryOpId {
    /// Construct a non-zero [`RecoveryOpId`].
    ///
    /// # Errors
    /// Returns [`RecoveryError::ZeroOperationId`] if `raw == 0`.
    pub const fn new(raw: u64) -> Result<Self, RecoveryError> {
        if raw == 0 {
            return Err(RecoveryError::ZeroOperationId);
        }
        Ok(Self(raw))
    }

    /// Underlying numeric id (`>= 1`).
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for RecoveryOpId {
    type Error = RecoveryError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RecoveryOpId> for u64 {
    fn from(id: RecoveryOpId) -> Self {
        id.0
    }
}

impl fmt::Display for RecoveryOpId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "recop-{}", self.0)
    }
}

/// Physical stage of a durable [`RecoveryOperation`] (`Q-S5`, `Q-S7`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryStage {
    /// Physically coupled at `pose`; exclusive tow control established.
    Attached {
        /// Coupled field pose.
        pose: FieldPose,
    },
    /// Actively towing across the field; both tug and casualty share `pose`.
    Towing {
        /// Current coupled field pose.
        pose: FieldPose,
        /// Cumulative distance towed in centimeters.
        distance_towed_cm: u64,
    },
    /// Uncoupled/released on the field; casualty remains at `pose` in its damaged state.
    Detached {
        /// Field pose where the casualty was released.
        pose: FieldPose,
        /// Cumulative distance towed prior to detachment.
        distance_towed_cm: u64,
    },
    /// Loaded into a carrier bay in its exact damaged state (`FO-1`, `FO-3`, `Q-S8`).
    LoadedIntoBay {
        /// Destination carrier object id.
        carrier: ObjId,
        /// Destination bay slot id.
        slot: SmolStr,
        /// Cumulative distance towed prior to bay load.
        distance_towed_cm: u64,
    },
}

impl RecoveryStage {
    /// Whether this stage holds active exclusive coupling over the tug and casualty.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Attached { .. } | Self::Towing { .. })
    }

    /// Current coupled field pose if the operation is `Attached` or `Towing`.
    #[must_use]
    pub const fn active_pose(&self) -> Option<FieldPose> {
        match self {
            Self::Attached { pose } | Self::Towing { pose, .. } => Some(*pose),
            Self::Detached { .. } | Self::LoadedIntoBay { .. } => None,
        }
    }

    /// Cumulative distance towed in centimeters.
    #[must_use]
    pub const fn distance_towed_cm(&self) -> u64 {
        match self {
            Self::Attached { .. } => 0,
            Self::Towing {
                distance_towed_cm, ..
            }
            | Self::Detached {
                distance_towed_cm, ..
            }
            | Self::LoadedIntoBay {
                distance_towed_cm, ..
            } => *distance_towed_cm,
        }
    }

    fn validate(&self, tug: ObjId, casualty: ObjId) -> Result<(), RecoveryError> {
        match self {
            Self::Attached { pose } | Self::Towing { pose, .. } | Self::Detached { pose, .. } => {
                pose.validate()?;
            }
            Self::LoadedIntoBay { carrier, slot, .. } => {
                if carrier.0 == 0 || *carrier == casualty || *carrier == tug {
                    return Err(RecoveryError::InvalidCarrierObject {
                        carrier: *carrier,
                        tug,
                        casualty,
                    });
                }
                if slot.trim().is_empty() {
                    return Err(RecoveryError::EmptyBaySlot);
                }
            }
        }
        Ok(())
    }
}

/// Durable record of one physical tow/recovery operation (`H2-A`, `Q-S5`–`Q-S9`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RecoveryOperationWire", into = "RecoveryOperationWire")]
pub struct RecoveryOperation {
    id: RecoveryOpId,
    revision: u64,
    tug: ObjId,
    casualty: ObjId,
    rig_profile: TowRigProfile,
    tug_base_speed_cms: u32,
    tug_mass_kg: u64,
    casualty_mass: UnitMassProfile,
    coupled_speed_cms: u32,
    casualty_state: OperationalState,
    casualty_unit_revision: u64,
    stage: RecoveryStage,
    started_at_ic: i64,
    updated_at_ic: i64,
}

impl RecoveryOperation {
    /// Operation id.
    #[must_use]
    pub const fn id(&self) -> RecoveryOpId {
        self.id
    }

    /// Operation revision (`>= 1`).
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Towing unit object id.
    #[must_use]
    pub const fn tug(&self) -> ObjId {
        self.tug
    }

    /// Towed casualty unit object id.
    #[must_use]
    pub const fn casualty(&self) -> ObjId {
        self.casualty
    }

    /// Tug rig profile.
    #[must_use]
    pub const fn rig_profile(&self) -> TowRigProfile {
        self.rig_profile
    }

    /// Tug uncoupled base speed (`cm/s`).
    #[must_use]
    pub const fn tug_base_speed_cms(&self) -> u32 {
        self.tug_base_speed_cms
    }

    /// Tug total mass (`kg`) at attachment.
    #[must_use]
    pub const fn tug_mass_kg(&self) -> u64 {
        self.tug_mass_kg
    }

    /// Casualty mass, retained cargo mass/volume, and dimensions.
    #[must_use]
    pub const fn casualty_mass(&self) -> &UnitMassProfile {
        &self.casualty_mass
    }

    /// Scaled coupled tow speed (`cm/s`).
    #[must_use]
    pub const fn coupled_speed_cms(&self) -> u32 {
        self.coupled_speed_cms
    }

    /// Casualty operational state (`Immobile`, `Unarmed`, `Disabled`, or `Wreck`).
    #[must_use]
    pub const fn casualty_state(&self) -> OperationalState {
        self.casualty_state
    }

    /// Pinned `TacticalUnit::revision()` of the casualty (verifying no silent healing during tow/load).
    #[must_use]
    pub const fn casualty_unit_revision(&self) -> u64 {
        self.casualty_unit_revision
    }

    /// Current stage of the recovery operation.
    #[must_use]
    pub const fn stage(&self) -> &RecoveryStage {
        &self.stage
    }

    /// IC timestamp when physical attachment completed.
    #[must_use]
    pub const fn started_at_ic(&self) -> i64 {
        self.started_at_ic
    }

    /// IC timestamp of the most recent stage update.
    #[must_use]
    pub const fn updated_at_ic(&self) -> i64 {
        self.updated_at_ic
    }

    fn validate(&self) -> Result<(), RecoveryError> {
        if self.tug.0 == 0 {
            return Err(RecoveryError::InvalidObjectId("tug"));
        }
        if self.casualty.0 == 0 {
            return Err(RecoveryError::InvalidObjectId("casualty"));
        }
        if self.tug == self.casualty {
            return Err(RecoveryError::SelfTow(self.tug));
        }
        if self.revision == 0 {
            return Err(RecoveryError::ZeroRevision);
        }
        if self.updated_at_ic < self.started_at_ic {
            return Err(RecoveryError::TimestampRegression {
                previous: self.started_at_ic,
                proposed: self.updated_at_ic,
            });
        }
        if !self.casualty_state.is_towable()
            && !(self.casualty_state == OperationalState::Irrecoverable
                && matches!(self.stage, RecoveryStage::Detached { .. }))
        {
            return Err(RecoveryError::TargetNotTowable {
                casualty: self.casualty,
                state: self.casualty_state,
            });
        }
        let expected_speed = scaled_tow_speed_cms(
            self.tug_base_speed_cms,
            self.tug_mass_kg,
            self.casualty_mass.total_mass_kg(),
            self.rig_profile,
        )?;
        if self.coupled_speed_cms != expected_speed {
            return Err(RecoveryError::CoupledSpeedMismatch {
                expected: expected_speed,
                actual: self.coupled_speed_cms,
            });
        }
        self.stage.validate(self.tug, self.casualty)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryOperationWire {
    id: RecoveryOpId,
    revision: u64,
    tug: ObjId,
    casualty: ObjId,
    rig_profile: TowRigProfile,
    tug_base_speed_cms: u32,
    tug_mass_kg: u64,
    casualty_mass: UnitMassProfile,
    coupled_speed_cms: u32,
    casualty_state: OperationalState,
    casualty_unit_revision: u64,
    stage: RecoveryStage,
    started_at_ic: i64,
    updated_at_ic: i64,
}

impl TryFrom<RecoveryOperationWire> for RecoveryOperation {
    type Error = RecoveryError;

    fn try_from(wire: RecoveryOperationWire) -> Result<Self, Self::Error> {
        let op = Self {
            id: wire.id,
            revision: wire.revision,
            tug: wire.tug,
            casualty: wire.casualty,
            rig_profile: wire.rig_profile,
            tug_base_speed_cms: wire.tug_base_speed_cms,
            tug_mass_kg: wire.tug_mass_kg,
            casualty_mass: wire.casualty_mass,
            coupled_speed_cms: wire.coupled_speed_cms,
            casualty_state: wire.casualty_state,
            casualty_unit_revision: wire.casualty_unit_revision,
            stage: wire.stage,
            started_at_ic: wire.started_at_ic,
            updated_at_ic: wire.updated_at_ic,
        };
        op.validate()?;
        Ok(op)
    }
}

impl From<RecoveryOperation> for RecoveryOperationWire {
    fn from(op: RecoveryOperation) -> Self {
        Self {
            id: op.id,
            revision: op.revision,
            tug: op.tug,
            casualty: op.casualty,
            rig_profile: op.rig_profile,
            tug_base_speed_cms: op.tug_base_speed_cms,
            tug_mass_kg: op.tug_mass_kg,
            casualty_mass: op.casualty_mass,
            coupled_speed_cms: op.coupled_speed_cms,
            casualty_state: op.casualty_state,
            casualty_unit_revision: op.casualty_unit_revision,
            stage: op.stage,
            started_at_ic: op.started_at_ic,
            updated_at_ic: op.updated_at_ic,
        }
    }
}

/// Request to physically attach a tug to a disabled/immobile/wrecked field casualty (`Q-S5`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachTowRequest {
    /// Mobile operational tug unit.
    pub tug: ObjId,
    /// Tug operational state (`Operational`).
    pub tug_state: OperationalState,
    /// Whether the tug has functional locomotion.
    pub tug_can_move: bool,
    /// Tug uncoupled base speed in `cm/s`.
    pub tug_base_speed_cms: u32,
    /// Tug total mass in kilograms.
    pub tug_mass_kg: u64,
    /// Tug rig profile (`Standard` vs `RecoveryOptimized`).
    pub rig_profile: TowRigProfile,
    /// Tug field pose at attachment.
    pub tug_pose: FieldPose,
    /// Target casualty assessment.
    pub casualty_assessment: FieldUnitAssessment,
    /// Target casualty mass and cargo profile.
    pub casualty_mass: UnitMassProfile,
    /// Target casualty field pose at attachment.
    pub casualty_pose: FieldPose,
    /// Maximum allowed attachment distance in centimeters.
    pub max_attach_distance_cm: u64,
    /// IC timestamp of attachment.
    pub at_ic: i64,
}

/// Request to load an attached/towed casualty into a carrier bay (`FO-1`, `FO-3`, `Q-S8`).
#[derive(Clone, Debug)]
pub struct LoadIntoBayRequest<'a> {
    /// Active recovery operation id.
    pub id: RecoveryOpId,
    /// Expected operation revision.
    pub expected_revision: u64,
    /// Destination carrier object id.
    pub carrier: ObjId,
    /// Destination carrier bay slot id.
    pub slot: SmolStr,
    /// Carrier field pose.
    pub carrier_pose: FieldPose,
    /// Maximum allowed loading distance in centimeters.
    pub max_load_distance_cm: u64,
    /// Carrier bay capacity envelope.
    pub bay_capacity: &'a BayCapacitySpec,
    /// Damaged casualty `TacticalUnit` (must match pinned revision; not auto-healed).
    pub casualty_unit: &'a TacticalUnit,
    /// IC timestamp of bay load.
    pub at_ic: i64,
}

/// Outcome returned when a towed casualty is loaded into a carrier bay (`FO-1`, `FO-3`, `Q-S8`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedCasualtyOutcome {
    /// Completed recovery operation id.
    pub operation_id: RecoveryOpId,
    /// Recovered casualty unit id.
    pub casualty: ObjId,
    /// Tug unit that delivered the casualty.
    pub recovered_by: ObjId,
    /// Destination carrier object id.
    pub carrier: ObjId,
    /// Destination carrier bay slot id.
    pub slot: SmolStr,
    /// Preserved operational state of the casualty (not healed on load).
    pub preserved_state: OperationalState,
    /// Preserved `TacticalUnit::revision()` of the casualty.
    pub preserved_unit_revision: u64,
    /// Preserved onboard cargo mass (`kg`) carried into the bay.
    pub preserved_cargo_mass_kg: u64,
    /// Preserved onboard cargo volume (`L`) carried into the bay.
    pub preserved_cargo_volume_l: u64,
    /// Validated IC hull epoch close outcome (`FieldCloseOutcome::RecoveredToBase { base: carrier }`).
    pub close_outcome: FieldCloseOutcome,
}

/// Durable book of physical tow/recovery operations (`H2-A`, `Q-S5`–`Q-S9`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RecoveryBookWire", into = "RecoveryBookWire")]
pub struct RecoveryBook {
    next_id: u64,
    operations: BTreeMap<RecoveryOpId, RecoveryOperation>,
}

impl Default for RecoveryBook {
    fn default() -> Self {
        Self::new()
    }
}

impl RecoveryBook {
    /// Construct an empty [`RecoveryBook`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next_id: 1,
            operations: BTreeMap::new(),
        }
    }

    /// Number of recorded operations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.operations.len()
    }

    /// Whether the book has no operations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    /// Next operation id to be allocated.
    #[must_use]
    pub const fn next_id(&self) -> u64 {
        self.next_id
    }

    /// Fetch an operation by id.
    #[must_use]
    pub fn get(&self, id: RecoveryOpId) -> Option<&RecoveryOperation> {
        self.operations.get(&id)
    }

    /// Iterate over all operations in deterministic id order.
    pub fn operations(&self) -> impl Iterator<Item = (&RecoveryOpId, &RecoveryOperation)> {
        self.operations.iter()
    }

    /// Return the active (`Attached` or `Towing`) operation where `casualty` is being towed, if any.
    #[must_use]
    pub fn active_operation_for_casualty(&self, casualty: ObjId) -> Option<&RecoveryOperation> {
        self.operations
            .values()
            .find(|op| op.casualty == casualty && op.stage.is_active())
    }

    /// Return the active (`Attached` or `Towing`) operation where `tug` is towing, if any.
    #[must_use]
    pub fn active_operation_for_tug(&self, tug: ObjId) -> Option<&RecoveryOperation> {
        self.operations
            .values()
            .find(|op| op.tug == tug && op.stage.is_active())
    }

    /// Return any active (`Attached` or `Towing`) operation involving `unit` as either tug or casualty.
    #[must_use]
    pub fn active_operation_for_unit(&self, unit: ObjId) -> Option<&RecoveryOperation> {
        self.operations
            .values()
            .find(|op| (op.tug == unit || op.casualty == unit) && op.stage.is_active())
    }

    /// Attach `req.tug` to `req.casualty_assessment.unit`, establishing exclusive physical
    /// towing control (`FO-13`, `Q-S5`, `Q-S6`, `Q-S9`).
    ///
    /// # Errors
    /// Returns [`RecoveryError`] if attachment preconditions or the `2×` tonnage cap fail.
    pub fn attach(&mut self, req: AttachTowRequest) -> Result<&RecoveryOperation, RecoveryError> {
        let casualty = req.casualty_assessment.unit;
        if req.tug.0 == 0 {
            return Err(RecoveryError::InvalidObjectId("tug"));
        }
        if casualty.0 == 0 {
            return Err(RecoveryError::InvalidObjectId("casualty"));
        }
        if req.tug == casualty {
            return Err(RecoveryError::SelfTow(req.tug));
        }
        req.tug_pose.validate()?;
        req.casualty_pose.validate()?;

        if !req.tug_state.is_operational() || !req.tug_can_move || req.tug_base_speed_cms == 0 {
            return Err(RecoveryError::TugNotMobile {
                tug: req.tug,
                state: req.tug_state,
            });
        }
        if !req.casualty_assessment.state.is_towable() {
            return Err(RecoveryError::TargetNotTowable {
                casualty,
                state: req.casualty_assessment.state,
            });
        }

        if let Some(existing) = self.active_operation_for_tug(req.tug) {
            return Err(RecoveryError::TugAlreadyActive {
                tug: req.tug,
                existing_op: existing.id,
            });
        }
        if let Some(existing) = self.active_operation_for_casualty(req.tug) {
            return Err(RecoveryError::UnitCurrentlyTowed {
                unit: req.tug,
                existing_op: existing.id,
            });
        }
        if let Some(existing) = self.active_operation_for_casualty(casualty) {
            return Err(RecoveryError::CasualtyAlreadyAttached {
                casualty,
                existing_op: existing.id,
                controlling_tug: existing.tug,
            });
        }
        if let Some(existing) = self.active_operation_for_tug(casualty) {
            return Err(RecoveryError::TugAlreadyActive {
                tug: casualty,
                existing_op: existing.id,
            });
        }

        let distance_cm =
            req.tug_pose
                .distance_cm_to(&req.casualty_pose)
                .ok_or(RecoveryError::SiteMismatch {
                    tug_site: req.tug_pose.site,
                    target_site: req.casualty_pose.site,
                })?;
        if distance_cm > req.max_attach_distance_cm {
            return Err(RecoveryError::OutOfAttachRange {
                distance_cm,
                max_distance_cm: req.max_attach_distance_cm,
            });
        }

        let coupled_speed_cms = scaled_tow_speed_cms(
            req.tug_base_speed_cms,
            req.tug_mass_kg,
            req.casualty_mass.total_mass_kg(),
            req.rig_profile,
        )?;

        if self.operations.len() >= MAX_RECOVERY_OPERATIONS {
            return Err(RecoveryError::RecoveryBookFull);
        }
        let id = RecoveryOpId::new(self.next_id)?;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(RecoveryError::OperationIdOverflow)?;

        let op = RecoveryOperation {
            id,
            revision: 1,
            tug: req.tug,
            casualty,
            rig_profile: req.rig_profile,
            tug_base_speed_cms: req.tug_base_speed_cms,
            tug_mass_kg: req.tug_mass_kg,
            casualty_mass: req.casualty_mass,
            coupled_speed_cms,
            casualty_state: req.casualty_assessment.state,
            casualty_unit_revision: req.casualty_assessment.unit_revision,
            stage: RecoveryStage::Attached { pose: req.tug_pose },
            started_at_ic: req.at_ic,
            updated_at_ic: req.at_ic,
        };
        op.validate()?;
        self.operations.insert(id, op);
        self.operations
            .get(&id)
            .ok_or(RecoveryError::UnknownOperation(id))
    }

    /// Advance a coupled tow operation to `next_pose` over `dt_secs` (`> 0`), checking
    /// that the step distance does not exceed `coupled_speed_cms * dt_secs` (`FO-13`).
    ///
    /// # Errors
    /// Returns [`RecoveryError`] if the operation is not active, changes sites, exceeds
    /// the coupled speed allowance for `dt_secs`, or regresses timestamps.
    pub fn advance_tow(
        &mut self,
        id: RecoveryOpId,
        expected_revision: u64,
        next_pose: FieldPose,
        dt_secs: u32,
        at_ic: i64,
    ) -> Result<&RecoveryOperation, RecoveryError> {
        next_pose.validate()?;
        if dt_secs == 0 {
            return Err(RecoveryError::ZeroDurationStep);
        }
        let current = self
            .operations
            .get(&id)
            .ok_or(RecoveryError::UnknownOperation(id))?;
        if current.revision != expected_revision {
            return Err(RecoveryError::RevisionMismatch {
                id,
                expected: expected_revision,
                actual: current.revision,
            });
        }
        if at_ic < current.updated_at_ic {
            return Err(RecoveryError::TimestampRegression {
                previous: current.updated_at_ic,
                proposed: at_ic,
            });
        }
        let prev_pose = current
            .stage
            .active_pose()
            .ok_or(RecoveryError::OperationNotActive(id))?;
        let step_dist_cm =
            prev_pose
                .distance_cm_to(&next_pose)
                .ok_or(RecoveryError::SiteMismatch {
                    tug_site: prev_pose.site,
                    target_site: next_pose.site,
                })?;
        let max_step_cm = u64::from(current.coupled_speed_cms)
            .checked_mul(u64::from(dt_secs))
            .ok_or(RecoveryError::MassOverflow)?;
        if step_dist_cm > max_step_cm {
            return Err(RecoveryError::StepExceedsCoupledSpeed {
                step_distance_cm: step_dist_cm,
                max_step_cm,
                coupled_speed_cms: current.coupled_speed_cms,
                dt_secs,
            });
        }
        let total_dist = current
            .stage
            .distance_towed_cm()
            .checked_add(step_dist_cm)
            .ok_or(RecoveryError::MassOverflow)?;
        let next_rev = current
            .revision
            .checked_add(1)
            .ok_or(RecoveryError::OperationIdOverflow)?;

        let op = self
            .operations
            .get_mut(&id)
            .ok_or(RecoveryError::UnknownOperation(id))?;
        op.revision = next_rev;
        op.stage = RecoveryStage::Towing {
            pose: next_pose,
            distance_towed_cm: total_dist,
        };
        op.updated_at_ic = at_ic;
        self.operations
            .get(&id)
            .ok_or(RecoveryError::UnknownOperation(id))
    }

    /// Release/detach an active tow coupling, leaving the casualty at its current field
    /// pose in its exact damaged state (`Q-S5`).
    ///
    /// # Errors
    /// Returns [`RecoveryError`] if the operation does not exist, is not active, or
    /// `expected_revision` mismatches.
    pub fn detach(
        &mut self,
        id: RecoveryOpId,
        expected_revision: u64,
        at_ic: i64,
    ) -> Result<&RecoveryOperation, RecoveryError> {
        let current = self
            .operations
            .get(&id)
            .ok_or(RecoveryError::UnknownOperation(id))?;
        if current.revision != expected_revision {
            return Err(RecoveryError::RevisionMismatch {
                id,
                expected: expected_revision,
                actual: current.revision,
            });
        }
        if at_ic < current.updated_at_ic {
            return Err(RecoveryError::TimestampRegression {
                previous: current.updated_at_ic,
                proposed: at_ic,
            });
        }
        let pose = current
            .stage
            .active_pose()
            .ok_or(RecoveryError::OperationNotActive(id))?;
        let dist = current.stage.distance_towed_cm();
        let next_rev = current
            .revision
            .checked_add(1)
            .ok_or(RecoveryError::OperationIdOverflow)?;

        let op = self
            .operations
            .get_mut(&id)
            .ok_or(RecoveryError::UnknownOperation(id))?;
        op.revision = next_rev;
        op.stage = RecoveryStage::Detached {
            pose,
            distance_towed_cm: dist,
        };
        op.updated_at_ic = at_ic;
        self.operations
            .get(&id)
            .ok_or(RecoveryError::UnknownOperation(id))
    }

    /// Synchronize an active tow operation when its towed `casualty` suffers additional
    /// combat damage (`Q-H8`, `Q-S7`).
    ///
    /// Updates the pinned `casualty_state` and `casualty_unit_revision` on the active
    /// operation so subsequent bay loading accepts the newly damaged revision. If the
    /// additional hits rendered the casualty [`OperationalState::Irrecoverable`], the
    /// operation automatically detaches at its current pose.
    ///
    /// # Errors
    /// Returns [`RecoveryError`] if `at_ic` regresses or revision counter overflows.
    pub fn sync_casualty_damage(
        &mut self,
        casualty: ObjId,
        new_state: OperationalState,
        new_unit_revision: u64,
        at_ic: i64,
    ) -> Result<Option<(RecoveryOpId, RecoveryStage)>, RecoveryError> {
        let Some(op_id) = self
            .active_operation_for_casualty(casualty)
            .map(RecoveryOperation::id)
        else {
            return Ok(None);
        };
        let current = self
            .operations
            .get(&op_id)
            .ok_or(RecoveryError::UnknownOperation(op_id))?;
        if at_ic < current.updated_at_ic {
            return Err(RecoveryError::TimestampRegression {
                previous: current.updated_at_ic,
                proposed: at_ic,
            });
        }
        let next_rev = current
            .revision
            .checked_add(1)
            .ok_or(RecoveryError::OperationIdOverflow)?;
        let pose = current
            .stage
            .active_pose()
            .ok_or(RecoveryError::OperationNotActive(op_id))?;
        let dist = current.stage.distance_towed_cm();

        let op = self
            .operations
            .get_mut(&op_id)
            .ok_or(RecoveryError::UnknownOperation(op_id))?;
        op.revision = next_rev;
        op.casualty_state = new_state;
        op.casualty_unit_revision = new_unit_revision;
        op.updated_at_ic = at_ic;
        if !new_state.is_towable() {
            op.stage = RecoveryStage::Detached {
                pose,
                distance_towed_cm: dist,
            };
        }
        Ok(Some((op_id, op.stage.clone())))
    }

    /// Complete a recovery operation by loading the towed casualty into a carrier bay
    /// (`FO-1`, `FO-3`, `Q-S5`, `Q-S8`, `Q-S9`, `Q-S11`).
    ///
    /// # Errors
    /// Returns [`RecoveryError`] if any capacity, proximity, revision, or close invariant fails.
    pub fn load_into_bay(
        &mut self,
        req: LoadIntoBayRequest<'_>,
    ) -> Result<LoadedCasualtyOutcome, RecoveryError> {
        let LoadIntoBayRequest {
            id,
            expected_revision,
            carrier,
            slot,
            carrier_pose,
            max_load_distance_cm,
            bay_capacity,
            casualty_unit,
            at_ic,
        } = req;
        carrier_pose.validate()?;
        if slot.trim().is_empty() {
            return Err(RecoveryError::EmptyBaySlot);
        }
        let current = self
            .operations
            .get(&id)
            .ok_or(RecoveryError::UnknownOperation(id))?;
        if current.revision != expected_revision {
            return Err(RecoveryError::RevisionMismatch {
                id,
                expected: expected_revision,
                actual: current.revision,
            });
        }
        if at_ic < current.updated_at_ic {
            return Err(RecoveryError::TimestampRegression {
                previous: current.updated_at_ic,
                proposed: at_ic,
            });
        }
        if carrier.0 == 0 || carrier == current.casualty || carrier == current.tug {
            return Err(RecoveryError::InvalidCarrierObject {
                carrier,
                tug: current.tug,
                casualty: current.casualty,
            });
        }
        if casualty_unit.object() != current.casualty {
            return Err(RecoveryError::CasualtyObjectMismatch {
                expected: current.casualty,
                actual: casualty_unit.object(),
            });
        }
        if casualty_unit.revision() != current.casualty_unit_revision {
            return Err(RecoveryError::CasualtyUnitRevisionChanged {
                casualty: current.casualty,
                expected: current.casualty_unit_revision,
                actual: casualty_unit.revision(),
            });
        }

        let tow_pose = current
            .stage
            .active_pose()
            .ok_or(RecoveryError::OperationNotActive(id))?;
        let distance_cm =
            tow_pose
                .distance_cm_to(&carrier_pose)
                .ok_or(RecoveryError::SiteMismatch {
                    tug_site: tow_pose.site,
                    target_site: carrier_pose.site,
                })?;
        if distance_cm > max_load_distance_cm {
            return Err(RecoveryError::OutOfAttachRange {
                distance_cm,
                max_distance_cm: max_load_distance_cm,
            });
        }

        bay_capacity.check_can_load(&current.casualty_mass)?;

        let close_outcome = FieldCloseOutcome::RecoveredToBase { base: carrier };
        validate_close_outcome(current.casualty, current.casualty_state, &close_outcome)
            .map_err(RecoveryError::Condition)?;

        let dist = current.stage.distance_towed_cm();
        let next_rev = current
            .revision
            .checked_add(1)
            .ok_or(RecoveryError::OperationIdOverflow)?;
        let outcome = LoadedCasualtyOutcome {
            operation_id: id,
            casualty: current.casualty,
            recovered_by: current.tug,
            carrier,
            slot: slot.clone(),
            preserved_state: current.casualty_state,
            preserved_unit_revision: current.casualty_unit_revision,
            preserved_cargo_mass_kg: current.casualty_mass.cargo_mass_kg(),
            preserved_cargo_volume_l: current.casualty_mass.cargo_volume_l(),
            close_outcome,
        };

        let op = self
            .operations
            .get_mut(&id)
            .ok_or(RecoveryError::UnknownOperation(id))?;
        op.revision = next_rev;
        op.stage = RecoveryStage::LoadedIntoBay {
            carrier,
            slot,
            distance_towed_cm: dist,
        };
        op.updated_at_ic = at_ic;
        Ok(outcome)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryBookWire {
    next_id: u64,
    operations: Vec<RecoveryOperation>,
}

impl TryFrom<RecoveryBookWire> for RecoveryBook {
    type Error = RecoveryError;

    fn try_from(wire: RecoveryBookWire) -> Result<Self, Self::Error> {
        if wire.next_id == 0 {
            return Err(RecoveryError::ZeroOperationId);
        }
        if wire.operations.len() > MAX_RECOVERY_OPERATIONS {
            return Err(RecoveryError::RecoveryBookFull);
        }
        let mut operations = BTreeMap::new();
        let mut active_units = BTreeSet::new();

        for op in wire.operations {
            let id = op.id();
            if id.get() >= wire.next_id {
                return Err(RecoveryError::InvalidNextId {
                    op_id: id,
                    next_id: wire.next_id,
                });
            }
            if operations.contains_key(&id) {
                return Err(RecoveryError::DuplicateOperationId(id));
            }
            if op.stage().is_active() {
                if !active_units.insert(op.tug()) {
                    return Err(RecoveryError::TugAlreadyActive {
                        tug: op.tug(),
                        existing_op: id,
                    });
                }
                if !active_units.insert(op.casualty()) {
                    return Err(RecoveryError::CasualtyAlreadyAttached {
                        casualty: op.casualty(),
                        existing_op: id,
                        controlling_tug: op.tug(),
                    });
                }
            }
            operations.insert(id, op);
        }

        Ok(Self {
            next_id: wire.next_id,
            operations,
        })
    }
}

impl From<RecoveryBook> for RecoveryBookWire {
    fn from(book: RecoveryBook) -> Self {
        Self {
            next_id: book.next_id,
            operations: book.operations.into_values().collect(),
        }
    }
}

/// Errors returned by `H2-A` universal towing, inspection, bay loading, and durable
/// recovery state operations.
#[derive(Clone, Debug, PartialEq)]
pub enum RecoveryError {
    /// Underlying [`crate::field_ops::condition::ConditionError`].
    Condition(crate::field_ops::condition::ConditionError),
    /// Object id was `#0`.
    InvalidObjectId(&'static str),
    /// Dry mass was `0`.
    ZeroDryMass,
    /// Unit width or height was `0`.
    ZeroDimensions {
        /// Width in cm.
        width_cm: u32,
        /// Height in cm.
        height_cm: u32,
    },
    /// Arithmetic overflow computing mass, distance, or speed.
    MassOverflow,
    /// Tug base speed was `0`.
    ZeroTugBaseSpeed,
    /// Casualty total mass exceeds `2 × tug_mass_kg` (`FO-13`, `Q-S6`).
    ExceedsTonnageCap {
        /// Tug total mass in kg.
        tug_mass_kg: u64,
        /// Casualty total mass in kg.
        casualty_mass_kg: u64,
        /// Maximum allowed casualty mass (`2 * tug_mass_kg`).
        max_casualty_mass_kg: u64,
    },
    /// Operation id was `0`.
    ZeroOperationId,
    /// Operation revision was `0`.
    ZeroRevision,
    /// Operation id or revision overflowed `u64::MAX`.
    OperationIdOverflow,
    /// Unit attempted to tow itself.
    SelfTow(ObjId),
    /// Tug is not operational or cannot move under its own power.
    TugNotMobile {
        /// Tug object id.
        tug: ObjId,
        /// Tug operational state.
        state: OperationalState,
    },
    /// Target is not in a towable state (`Operational` or `Irrecoverable`).
    TargetNotTowable {
        /// Target object id.
        casualty: ObjId,
        /// Target operational state.
        state: OperationalState,
    },
    /// Tug is already engaged in an active tow operation (`Q-S9`).
    TugAlreadyActive {
        /// Tug object id.
        tug: ObjId,
        /// Existing active operation id.
        existing_op: RecoveryOpId,
    },
    /// Unit is currently being towed and cannot act as a tug (`Q-S9`).
    UnitCurrentlyTowed {
        /// Unit object id.
        unit: ObjId,
        /// Existing active operation id.
        existing_op: RecoveryOpId,
    },
    /// Casualty is already attached to an active tow operation (`Q-S9`).
    CasualtyAlreadyAttached {
        /// Casualty object id.
        casualty: ObjId,
        /// Existing active operation id.
        existing_op: RecoveryOpId,
        /// Tug holding exclusive control.
        controlling_tug: ObjId,
    },
    /// Tug and target/carrier are on different surface sites.
    SiteMismatch {
        /// Tug site.
        tug_site: ObjId,
        /// Target/carrier site.
        target_site: ObjId,
    },
    /// Tug and target/carrier are beyond physical attachment/loading reach.
    OutOfAttachRange {
        /// Actual distance in cm.
        distance_cm: u64,
        /// Maximum allowed distance in cm.
        max_distance_cm: u64,
    },
    /// Stored `coupled_speed_cms` does not match the deterministic tonnage curve.
    CoupledSpeedMismatch {
        /// Expected speed in cm/s.
        expected: u32,
        /// Actual speed in cm/s.
        actual: u32,
    },
    /// [`RecoveryBook`] reached [`MAX_RECOVERY_OPERATIONS`].
    RecoveryBookFull,
    /// Referenced [`RecoveryOpId`] was not found.
    UnknownOperation(RecoveryOpId),
    /// Duplicate [`RecoveryOpId`] in serialized book.
    DuplicateOperationId(RecoveryOpId),
    /// Serialized `next_id` was not strictly greater than all operation IDs.
    InvalidNextId {
        /// Operation id.
        op_id: RecoveryOpId,
        /// Invalid `next_id`.
        next_id: u64,
    },
    /// Optimistic revision check failed on [`RecoveryOperation`].
    RevisionMismatch {
        /// Operation id.
        id: RecoveryOpId,
        /// Expected revision.
        expected: u64,
        /// Actual revision.
        actual: u64,
    },
    /// Operation is no longer active (`Detached` or `LoadedIntoBay`).
    OperationNotActive(RecoveryOpId),
    /// Timestamp regressed.
    TimestampRegression {
        /// Previous timestamp.
        previous: i64,
        /// Proposed earlier timestamp.
        proposed: i64,
    },
    /// Coupled movement step had `dt_secs == 0`.
    ZeroDurationStep,
    /// Coupled movement step exceeded `coupled_speed_cms * dt_secs`.
    StepExceedsCoupledSpeed {
        /// Requested step distance in cm.
        step_distance_cm: u64,
        /// Maximum allowed step distance in cm.
        max_step_cm: u64,
        /// Coupled speed in cm/s.
        coupled_speed_cms: u32,
        /// Step duration in seconds.
        dt_secs: u32,
    },
    /// Carrier object id is `#0` or equal to tug/casualty.
    InvalidCarrierObject {
        /// Carrier object id.
        carrier: ObjId,
        /// Tug object id.
        tug: ObjId,
        /// Casualty object id.
        casualty: ObjId,
    },
    /// Carrier bay slot string was empty.
    EmptyBaySlot,
    /// Casualty `TacticalUnit::object()` did not match the recovery operation's casualty.
    CasualtyObjectMismatch {
        /// Expected casualty object id.
        expected: ObjId,
        /// Actual unit object id.
        actual: ObjId,
    },
    /// Casualty `TacticalUnit::revision()` changed unexpectedly during tow/bay load.
    CasualtyUnitRevisionChanged {
        /// Casualty object id.
        casualty: ObjId,
        /// Pinned revision at attachment.
        expected: u64,
        /// Actual revision presented at bay load.
        actual: u64,
    },
    /// Carrier bay slot has no free unit capacity (`Q-S8`).
    BaySlotFull {
        /// Current units in slot.
        current_units: u32,
        /// Max units in slot.
        max_units: u32,
    },
    /// Casualty silhouette exceeds carrier bay entrance/cradle dimensions (`Q-S8`).
    BayDimensionExceeded {
        /// Casualty width in cm.
        unit_width_cm: u32,
        /// Casualty height in cm.
        unit_height_cm: u32,
        /// Bay max width in cm.
        max_width_cm: u32,
        /// Bay max height in cm.
        max_height_cm: u32,
    },
    /// Casualty total mass (dry + retained cargo) exceeds carrier bay mass limit (`Q-S8`).
    BayMassExceeded {
        /// Total required mass (`current + casualty`) in kg.
        required_mass_kg: u64,
        /// Bay max mass in kg.
        max_mass_kg: u64,
    },
    /// Casualty retained cargo volume exceeds carrier bay volume limit (`Q-S8`).
    BayVolumeExceeded {
        /// Total required volume (`current + casualty`) in L.
        required_volume_l: u64,
        /// Bay max volume in L.
        max_volume_l: u64,
    },
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Condition(e) => write!(f, "condition error: {e}"),
            Self::InvalidObjectId(field) => write!(f, "invalid zero object id for {field}"),
            Self::ZeroDryMass => f.write_str("unit dry mass must be non-zero"),
            Self::ZeroDimensions {
                width_cm,
                height_cm,
            } => write!(
                f,
                "unit dimensions must be non-zero (got {width_cm}x{height_cm} cm)"
            ),
            Self::MassOverflow => {
                f.write_str("arithmetic overflow in mass or distance calculation")
            }
            Self::ZeroTugBaseSpeed => f.write_str("tug base speed must be non-zero"),
            Self::ExceedsTonnageCap {
                tug_mass_kg,
                casualty_mass_kg,
                max_casualty_mass_kg,
            } => write!(
                f,
                "casualty mass {casualty_mass_kg} kg exceeds 2x tug mass cap ({max_casualty_mass_kg} kg for {tug_mass_kg} kg tug)"
            ),
            Self::ZeroOperationId => f.write_str("recovery operation id must be non-zero"),
            Self::ZeroRevision => f.write_str("recovery operation revision must be non-zero"),
            Self::OperationIdOverflow => f.write_str("recovery operation id or revision overflow"),
            Self::SelfTow(u) => write!(f, "unit {u:?} cannot tow itself"),
            Self::TugNotMobile { tug, state } => {
                write!(f, "tug {tug:?} in state '{state}' is not mobile")
            }
            Self::TargetNotTowable { casualty, state } => {
                write!(f, "target {casualty:?} in state '{state}' is not towable")
            }
            Self::TugAlreadyActive { tug, existing_op } => {
                write!(f, "tug {tug:?} is already active in {existing_op}")
            }
            Self::UnitCurrentlyTowed { unit, existing_op } => {
                write!(f, "unit {unit:?} is currently being towed in {existing_op}")
            }
            Self::CasualtyAlreadyAttached {
                casualty,
                existing_op,
                controlling_tug,
            } => write!(
                f,
                "casualty {casualty:?} is already attached in {existing_op} by tug {controlling_tug:?}"
            ),
            Self::SiteMismatch {
                tug_site,
                target_site,
            } => write!(
                f,
                "site mismatch: tug at {tug_site:?} vs target at {target_site:?}"
            ),
            Self::OutOfAttachRange {
                distance_cm,
                max_distance_cm,
            } => write!(
                f,
                "distance {distance_cm} cm exceeds maximum reach {max_distance_cm} cm"
            ),
            Self::CoupledSpeedMismatch { expected, actual } => write!(
                f,
                "coupled speed mismatch: expected {expected} cm/s, actual {actual} cm/s"
            ),
            Self::RecoveryBookFull => f.write_str("recovery operation book is full"),
            Self::UnknownOperation(id) => write!(f, "unknown recovery operation {id}"),
            Self::DuplicateOperationId(id) => write!(f, "duplicate recovery operation {id}"),
            Self::InvalidNextId { op_id, next_id } => {
                write!(f, "operation {op_id} is not less than next_id {next_id}")
            }
            Self::RevisionMismatch {
                id,
                expected,
                actual,
            } => write!(
                f,
                "revision mismatch on {id}: expected {expected}, actual {actual}"
            ),
            Self::OperationNotActive(id) => {
                write!(f, "recovery operation {id} is no longer active")
            }
            Self::TimestampRegression { previous, proposed } => write!(
                f,
                "timestamp regression: previous {previous}, proposed {proposed}"
            ),
            Self::ZeroDurationStep => f.write_str("tow movement step duration must be non-zero"),
            Self::StepExceedsCoupledSpeed {
                step_distance_cm,
                max_step_cm,
                coupled_speed_cms,
                dt_secs,
            } => write!(
                f,
                "step distance {step_distance_cm} cm exceeds coupled allowance {max_step_cm} cm ({coupled_speed_cms} cm/s * {dt_secs} s)"
            ),
            Self::InvalidCarrierObject {
                carrier,
                tug,
                casualty,
            } => write!(
                f,
                "invalid carrier {carrier:?} for tug {tug:?} and casualty {casualty:?}"
            ),
            Self::EmptyBaySlot => f.write_str("carrier bay slot id must not be empty"),
            Self::CasualtyObjectMismatch { expected, actual } => write!(
                f,
                "casualty unit mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::CasualtyUnitRevisionChanged {
                casualty,
                expected,
                actual,
            } => write!(
                f,
                "casualty {casualty:?} revision changed during recovery: expected {expected}, got {actual}"
            ),
            Self::BaySlotFull {
                current_units,
                max_units,
            } => write!(
                f,
                "carrier bay slot full ({current_units}/{max_units} units)"
            ),
            Self::BayDimensionExceeded {
                unit_width_cm,
                unit_height_cm,
                max_width_cm,
                max_height_cm,
            } => write!(
                f,
                "unit envelope {unit_width_cm}x{unit_height_cm} cm exceeds bay limit {max_width_cm}x{max_height_cm} cm"
            ),
            Self::BayMassExceeded {
                required_mass_kg,
                max_mass_kg,
            } => write!(
                f,
                "required bay mass {required_mass_kg} kg exceeds limit {max_mass_kg} kg"
            ),
            Self::BayVolumeExceeded {
                required_volume_l,
                max_volume_l,
            } => write!(
                f,
                "required bay volume {required_volume_l} L exceeds limit {max_volume_l} L"
            ),
        }
    }
}

impl std::error::Error for RecoveryError {}
