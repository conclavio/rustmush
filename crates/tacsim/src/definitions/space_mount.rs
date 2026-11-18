//! **A ship's armament, authored** (doc 49 unit CC.6; Q-49-11, Q-49-24, Q-49-25).
//!
//! Two pack entry kinds, in the same `rules_definition` pack format as the ground's mounts and
//! decoded as strictly (`deny_unknown_fields`, integers only):
//!
//! * **`space_mount`** ([`SpaceMountDef`]): one kind of mount a ship can carry, and every number
//!   the space kernel reads off it. A mount is one of four families: twin torpedo tubes, a laser
//!   turret, a spinal gun, or a point-defence rotary. Reaches are metres and run past the
//!   ground's 100 km cap ([`MAX_SPACE_RANGE_M`]); a laser has no magazine, so nothing here asks
//!   for a load of at least one.
//! * **`space_fit`** ([`SpaceFitDef`]): what one hull carries. It names the unit it fits, and
//!   for each mount its id on the hull (the id a mind's `fire(id, slot)` and the commands name),
//!   the `space_mount` row it is, its magazine, and, for a laser, the modes the fit allows and
//!   the mode a fresh hull starts in (Q-49-24: the fit names the modes, so a defence-only laser
//!   is the fit's rule, not a doctrine's).
//!
//! The ground's `weapon` kind ([`crate::definitions::WeaponDef`]) does not fit space: it caps a
//! range band at 100 km and fires a round family whose load is at least one.
//!
//! # Not minted
//!
//! Neither kind is a definition the registry mints: no unit binding names a space mount by
//! reference, and the space kernel reads the rows by id. [`crate::definitions::Catalog::build`]
//! checks each entry's bounds, refuses an id two packs declare, and checks every reference a fit
//! makes (its unit, its mount rows, a magazine where the family keeps one and none where it
//! does not, and modes the row has), exactly as it checks the minted kinds.
//!
//! # Units
//!
//! Metres, metres per second, metres per second squared, wall milliseconds, per-mille of the
//! hull's heat ceiling. Damage is the space kernel's own figure.

use mush_core::CanonicalId;
use serde::{Deserialize, Serialize};

use crate::definitions::DefinitionError;

/// The longest reach a space mount may declare, metres: two thousand kilometres, four times the
/// furthest shipped laser and past any bubble's sensor horizon.
pub const MAX_SPACE_RANGE_M: i64 = 2_000_000;

/// The shortest a space mount may cycle, wall milliseconds: one bubble substep
/// (`crate::space::bubble::MAX_SUBSTEP_MS`). A shorter cycle would fire as often as the bubble
/// happened to step. The 5 s floor for a mount a player or a mind fires (Q-49-19) is the
/// palette's (doc 49 unit CC.8), not the format's.
pub const SPACE_CYCLE_FLOOR_MS: u32 = 250;

/// The slowest a space mount may cycle: an hour.
pub const MAX_SPACE_CYCLE_MS: u32 = 3_600_000;

/// The most damage one round, slug, warhead or beam may declare.
pub const MAX_SPACE_DAMAGE: u32 = 100_000;

/// The most tubes one salvo may fire.
pub const MAX_SALVO: u8 = 8;

/// The hardest a torpedo may accelerate, m/s²: a thousand g.
pub const MAX_TORPEDO_ACCEL_MPS2: i64 = 10_000;

/// The most change of speed a torpedo may carry, m/s.
pub const MAX_TORPEDO_DV_MPS: i64 = 100_000;

/// The slowest and fastest a spinal round may leave the muzzle, m/s.
pub const SPINAL_MUZZLE_MPS: (i64, i64) = (1_000, 1_000_000);

/// The shortest half-intensity reach a laser may declare, metres: the falloff is computed in
/// whole kilometres.
pub const MIN_LASER_FALLOFF_M: i64 = 1_000;

/// The most rounds a point-defence burst may spend.
pub const MAX_BURST_ROUNDS: u16 = 10_000;

/// The most a tube family or a spinal gun may stow.
pub const MAX_MAGAZINE: u32 = 60_000;

/// The most a point-defence rotary may stow.
pub const MAX_PD_MAGAZINE: u32 = 1_000_000;

/// The most mounts one fit may carry.
pub const MAX_FIT_MOUNTS: usize = 16;

/// The most decoy canisters one fit may stow.
pub const MAX_DECOYS: u8 = 64;

/// The largest RCS figure one fit may declare, in its field's unit.
pub const MAX_RCS: i64 = 10_000;

/// **A laser's mode** (Q-49-24): the content word for each.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaserModeWord {
    /// It fires at her lock on command.
    Offence,
    /// It stops incoming torpedoes on its own (D-35-8).
    Defence,
}

impl LaserModeWord {
    /// The content word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            LaserModeWord::Offence => "offence",
            LaserModeWord::Defence => "defence",
        }
    }
}

/// **Twin torpedo tubes**: the salvo, the reload, and each torpedo's drive and warhead.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TubesDef {
    /// Tubes that fire together: the most one salvo launches.
    pub salvo: u8,
    /// Wall milliseconds between salvos.
    pub reload_ms: u32,
    /// Each torpedo's drive, m/s².
    pub accel_mps2: i64,
    /// Each torpedo's change of speed, m/s.
    pub dv_mps: i64,
    /// Each warhead's damage.
    pub warhead_damage: u32,
}

/// **One laser mode's cycle** (Q-49-24: the cycle belongs to the mode).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaserModeDef {
    /// Wall milliseconds between pulses in this mode.
    pub cycle_ms: u32,
}

/// **The modes a laser row can work in**, each with its own cycle. At least one.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaserModesDef {
    /// Firing at her lock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offence: Option<LaserModeDef>,
    /// Stopping incoming torpedoes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defence: Option<LaserModeDef>,
}

impl LaserModesDef {
    /// The mode `word`'s cycle, when the row has it.
    #[must_use]
    pub const fn mode(&self, word: LaserModeWord) -> Option<LaserModeDef> {
        match word {
            LaserModeWord::Offence => self.offence,
            LaserModeWord::Defence => self.defence,
        }
    }
}

/// **A laser turret**: damage falling with range, its reaches, heat per shot, and its modes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaserDef {
    /// Damage at no range, before the falloff.
    pub base_damage: u32,
    /// The falloff's half-intensity reach, metres: damage at range `R` is
    /// `base_damage × R0² / (R0² + R²)`, computed in whole kilometres.
    pub falloff_half_m: i64,
    /// The furthest an offence pulse lands, metres.
    pub max_range_m: i64,
    /// The furthest a defence pulse stops a torpedo, metres.
    pub defence_range_m: i64,
    /// Hull heat each pulse adds, per-mille of the ceiling.
    pub heat_per_shot_permille: u16,
    /// The modes the row can work in, each with its cycle.
    pub modes: LaserModesDef,
}

/// **A spinal gun**: an unguided round, its muzzle speed, reach, damage, heat and cycle.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpinalDef {
    /// The round's speed leaving the muzzle, m/s (the hull's own velocity is added).
    pub muzzle_mps: i64,
    /// The furthest it fires, metres.
    pub max_range_m: i64,
    /// Each round's damage.
    pub round_damage: u32,
    /// Hull heat each round adds, per-mille of the ceiling.
    pub heat_per_shot_permille: u16,
    /// Wall milliseconds between rounds.
    pub cycle_ms: u32,
}

/// **A point-defence rotary**: its envelope, burst, cycle and what a burst does to a torpedo.
/// It fires on its own when automatic point defence is on (D-35-8).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointDefenceDef {
    /// The furthest a burst reaches an incoming torpedo, metres.
    pub envelope_m: i64,
    /// Rounds each burst spends.
    pub burst_rounds: u16,
    /// Wall milliseconds between bursts.
    pub cycle_ms: u32,
    /// What one burst takes off a torpedo's integrity (a torpedo leaves the tube at 100).
    pub damage_per_burst: u16,
}

/// **Which family a mount is**, with that family's numbers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceMountFamily {
    /// Twin torpedo tubes.
    Tubes(TubesDef),
    /// A laser turret.
    Laser(LaserDef),
    /// A spinal gun.
    Spinal(SpinalDef),
    /// A point-defence rotary.
    PointDefence(PointDefenceDef),
}

impl SpaceMountFamily {
    /// The family's content word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            SpaceMountFamily::Tubes(_) => "tubes",
            SpaceMountFamily::Laser(_) => "laser",
            SpaceMountFamily::Spinal(_) => "spinal",
            SpaceMountFamily::PointDefence(_) => "point_defence",
        }
    }

    /// The most a fit may stow for one mount of this family, or `None` for a family that keeps
    /// no magazine (a laser).
    #[must_use]
    pub const fn magazine_cap(&self) -> Option<u32> {
        match self {
            SpaceMountFamily::Tubes(_) | SpaceMountFamily::Spinal(_) => Some(MAX_MAGAZINE),
            SpaceMountFamily::PointDefence(_) => Some(MAX_PD_MAGAZINE),
            SpaceMountFamily::Laser(_) => None,
        }
    }
}

/// **One space mount row** (the module head).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceMountDef {
    /// The human-facing name.
    pub label: String,
    /// The family and its numbers.
    pub family: SpaceMountFamily,
    /// Why these numbers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

fn bounded<T: PartialOrd + std::fmt::Display + Copy>(
    entry: &str,
    field: &str,
    value: T,
    low: T,
    high: T,
) -> Result<(), DefinitionError> {
    if value < low || value > high {
        return Err(DefinitionError::value(
            entry,
            format!("{field} is {low} to {high}; {value} is not"),
        ));
    }
    Ok(())
}

fn cycle(entry: &str, field: &str, value: u32) -> Result<(), DefinitionError> {
    bounded(
        entry,
        field,
        value,
        SPACE_CYCLE_FLOOR_MS,
        MAX_SPACE_CYCLE_MS,
    )
}

fn reach(entry: &str, field: &str, value: i64) -> Result<(), DefinitionError> {
    bounded(entry, field, value, 1, MAX_SPACE_RANGE_M)
}

fn damage(entry: &str, field: &str, value: u32) -> Result<(), DefinitionError> {
    bounded(entry, field, value, 1, MAX_SPACE_DAMAGE)
}

fn heat(entry: &str, value: u16) -> Result<(), DefinitionError> {
    bounded(entry, "heat_per_shot_permille", value, 0, 1_000)
}

impl SpaceMountDef {
    /// Re-run every bound.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry and the field.
    pub fn check(&self, entry: &str) -> Result<(), DefinitionError> {
        crate::definitions::check_label(entry, &self.label)?;
        crate::definitions::check_note(entry, &self.note)?;
        match &self.family {
            SpaceMountFamily::Tubes(tubes) => {
                bounded(entry, "salvo", tubes.salvo, 1, MAX_SALVO)?;
                cycle(entry, "reload_ms", tubes.reload_ms)?;
                bounded(
                    entry,
                    "accel_mps2",
                    tubes.accel_mps2,
                    1,
                    MAX_TORPEDO_ACCEL_MPS2,
                )?;
                bounded(entry, "dv_mps", tubes.dv_mps, 1, MAX_TORPEDO_DV_MPS)?;
                damage(entry, "warhead_damage", tubes.warhead_damage)?;
            }
            SpaceMountFamily::Laser(laser) => {
                damage(entry, "base_damage", laser.base_damage)?;
                bounded(
                    entry,
                    "falloff_half_m",
                    laser.falloff_half_m,
                    MIN_LASER_FALLOFF_M,
                    MAX_SPACE_RANGE_M,
                )?;
                reach(entry, "max_range_m", laser.max_range_m)?;
                reach(entry, "defence_range_m", laser.defence_range_m)?;
                heat(entry, laser.heat_per_shot_permille)?;
                if laser.modes.offence.is_none() && laser.modes.defence.is_none() {
                    return Err(DefinitionError::value(
                        entry,
                        "a laser works in at least one mode, offense or defense",
                    ));
                }
                if let Some(mode) = laser.modes.offence {
                    cycle(entry, "modes.offence.cycle_ms", mode.cycle_ms)?;
                }
                if let Some(mode) = laser.modes.defence {
                    cycle(entry, "modes.defence.cycle_ms", mode.cycle_ms)?;
                }
            }
            SpaceMountFamily::Spinal(spinal) => {
                bounded(
                    entry,
                    "muzzle_mps",
                    spinal.muzzle_mps,
                    SPINAL_MUZZLE_MPS.0,
                    SPINAL_MUZZLE_MPS.1,
                )?;
                reach(entry, "max_range_m", spinal.max_range_m)?;
                damage(entry, "round_damage", spinal.round_damage)?;
                heat(entry, spinal.heat_per_shot_permille)?;
                cycle(entry, "cycle_ms", spinal.cycle_ms)?;
            }
            SpaceMountFamily::PointDefence(pd) => {
                reach(entry, "envelope_m", pd.envelope_m)?;
                bounded(entry, "burst_rounds", pd.burst_rounds, 1, MAX_BURST_ROUNDS)?;
                cycle(entry, "cycle_ms", pd.cycle_ms)?;
                bounded(entry, "damage_per_burst", pd.damage_per_burst, 1, 1_000)?;
            }
        }
        Ok(())
    }
}

/// **A hull's RCS thruster authority** for the docking game (D-35-14), per 60 Hz frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceRcsDef {
    /// Lateral velocity change per frame of thrust, mm/s.
    pub trans_dv_mmps_per_frame: i64,
    /// Closure velocity change per frame of thrust, mm/s.
    pub closure_dv_mmps_per_frame: i64,
    /// Pitch and yaw rate change per frame of trim, mdeg/s.
    pub attitude_rate_mdeg_s_per_frame: i32,
    /// Roll rate change per frame of trim, mdeg/s.
    pub roll_rate_mdeg_s_per_frame: i32,
    /// The dampener's lateral bleed per frame, mm/s.
    pub damp_trans_mmps_per_frame: i64,
    /// The dampener's pitch and yaw bleed per frame, mdeg/s.
    pub damp_attitude_mdeg_s_per_frame: i32,
    /// The dampener's roll bleed per frame, mdeg/s.
    pub damp_roll_mdeg_s_per_frame: i32,
    /// Propellant debited per twelve axis-frame pulses, kilograms.
    pub kg_per_pulse_batch: u32,
}

impl SpaceRcsDef {
    fn check(&self, entry: &str) -> Result<(), DefinitionError> {
        for (field, value) in [
            ("rcs.trans_dv_mmps_per_frame", self.trans_dv_mmps_per_frame),
            (
                "rcs.closure_dv_mmps_per_frame",
                self.closure_dv_mmps_per_frame,
            ),
            (
                "rcs.attitude_rate_mdeg_s_per_frame",
                i64::from(self.attitude_rate_mdeg_s_per_frame),
            ),
            (
                "rcs.roll_rate_mdeg_s_per_frame",
                i64::from(self.roll_rate_mdeg_s_per_frame),
            ),
            (
                "rcs.damp_trans_mmps_per_frame",
                self.damp_trans_mmps_per_frame,
            ),
            (
                "rcs.damp_attitude_mdeg_s_per_frame",
                i64::from(self.damp_attitude_mdeg_s_per_frame),
            ),
            (
                "rcs.damp_roll_mdeg_s_per_frame",
                i64::from(self.damp_roll_mdeg_s_per_frame),
            ),
            ("rcs.kg_per_pulse_batch", i64::from(self.kg_per_pulse_batch)),
        ] {
            bounded(entry, field, value, 1, MAX_RCS)?;
        }
        Ok(())
    }
}

/// **One mount on a hull** (the module head).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceFitMount {
    /// Its id on the hull: what a mind's `fire(id, slot)` and the commands name. Unique in the
    /// fit.
    pub id: CanonicalId,
    /// The `space_mount` row it is.
    pub mount: CanonicalId,
    /// What it stows: torpedoes, spinal rounds or point-defence rounds. Required for a family
    /// that keeps a magazine and refused for a laser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magazine: Option<u32>,
    /// A laser's modes the fit allows, a subset of the row's (Q-49-24). Required for a laser and
    /// refused for any other family.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<LaserModeWord>,
    /// The mode a fresh hull's laser starts in, one of [`Self::modes`]. Required for a laser and
    /// refused for any other family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<LaserModeWord>,
}

impl SpaceFitMount {
    /// Whether the fit lets this mount work in `word`.
    #[must_use]
    pub fn allows(&self, word: LaserModeWord) -> bool {
        self.modes.contains(&word)
    }
}

/// **One hull's fit** (the module head).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceFitDef {
    /// The human-facing name.
    pub label: String,
    /// The unit the fit is for: a `unit` entry some loaded pack declares.
    pub unit: CanonicalId,
    /// The look the space cockpit draws her with (`freighter`, `corvette`, `raider`, `halcyon`).
    pub pixel_model: CanonicalId,
    /// Decoy canisters she stows.
    pub decoys: u8,
    /// Her RCS authority in the docking game.
    pub rcs: SpaceRcsDef,
    /// Her mounts, in the order each family's mounts are kept and shown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<SpaceFitMount>,
    /// Why this fit.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl SpaceFitDef {
    /// Re-run every bound the fit can check alone; the references to its unit and its mount
    /// rows are the catalogue build's ([`Self::check_against`]).
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry: a bound, too many mounts, an id twice, a
    /// laser start mode the fit does not allow, or a mode named twice.
    pub fn check(&self, entry: &str) -> Result<(), DefinitionError> {
        crate::definitions::check_label(entry, &self.label)?;
        crate::definitions::check_note(entry, &self.note)?;
        bounded(entry, "decoys", self.decoys, 0, MAX_DECOYS)?;
        self.rcs.check(entry)?;
        if self.mounts.len() > MAX_FIT_MOUNTS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "a fit carries at most {MAX_FIT_MOUNTS} mounts; {} is not",
                    self.mounts.len()
                ),
            ));
        }
        for (i, mount) in self.mounts.iter().enumerate() {
            if self
                .mounts
                .iter()
                .take(i)
                .any(|earlier| earlier.id == mount.id)
            {
                return Err(DefinitionError::value(
                    entry,
                    format!("mount `{}` is named twice", mount.id),
                ));
            }
            for (j, word) in mount.modes.iter().enumerate() {
                if mount.modes.iter().take(j).any(|earlier| earlier == word) {
                    return Err(DefinitionError::value(
                        entry,
                        format!("mount `{}` names mode `{}` twice", mount.id, word.as_str()),
                    ));
                }
            }
            if let Some(start) = mount.mode
                && !mount.allows(start)
            {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "mount `{}` starts in `{}`, which its fit does not allow",
                        mount.id,
                        start.as_str()
                    ),
                ));
            }
        }
        Ok(())
    }

    /// **Check the fit's references** (the module head): every mount names a `space_mount` row
    /// `mount_of` finds, stows a magazine within its family's cap where the family keeps one and
    /// none where it does not, and a laser names modes its row has and a start mode.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Reference`] for a row no loaded pack declares, and
    /// [`DefinitionError::Value`] for a magazine or a mode the row does not take.
    pub fn check_against<'a>(
        &self,
        entry: &str,
        mount_of: impl Fn(&CanonicalId) -> Option<&'a SpaceMountDef>,
    ) -> Result<(), DefinitionError> {
        for mount in &self.mounts {
            let Some(row) = mount_of(&mount.mount) else {
                return Err(DefinitionError::reference(
                    entry,
                    "space_mount",
                    mount.mount.as_str(),
                ));
            };
            match (row.family.magazine_cap(), mount.magazine) {
                (Some(cap), Some(stowed)) if stowed > cap => {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "mount `{}` stows {stowed}; a {} mount stows at most {cap}",
                            mount.id,
                            row.family.as_str()
                        ),
                    ));
                }
                (Some(_), None) => {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "mount `{}` is a {} mount, so it names its magazine",
                            mount.id,
                            row.family.as_str()
                        ),
                    ));
                }
                (None, Some(_)) => {
                    return Err(DefinitionError::value(
                        entry,
                        format!("mount `{}` is a laser, which keeps no magazine", mount.id),
                    ));
                }
                _ => {}
            }
            match &row.family {
                SpaceMountFamily::Laser(laser) => {
                    if mount.modes.is_empty() || mount.mode.is_none() {
                        return Err(DefinitionError::value(
                            entry,
                            format!(
                                "mount `{}` is a laser, so it names the modes its fit allows \
                                 and the mode it starts in",
                                mount.id
                            ),
                        ));
                    }
                    if let Some(word) = mount
                        .modes
                        .iter()
                        .find(|word| laser.modes.mode(**word).is_none())
                    {
                        return Err(DefinitionError::value(
                            entry,
                            format!(
                                "mount `{}` allows `{}`, which row `{}` does not work in",
                                mount.id,
                                word.as_str(),
                                mount.mount
                            ),
                        ));
                    }
                }
                _ if !mount.modes.is_empty() || mount.mode.is_some() => {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "mount `{}` is a {} mount, which has no modes",
                            mount.id,
                            row.family.as_str()
                        ),
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> CanonicalId {
        CanonicalId::new(text).expect("a canonical id")
    }

    fn laser_row() -> SpaceMountDef {
        SpaceMountDef {
            label: "Turret".to_owned(),
            family: SpaceMountFamily::Laser(LaserDef {
                base_damage: 110,
                falloff_half_m: 120_000,
                max_range_m: 500_000,
                defence_range_m: 45_000,
                heat_per_shot_permille: 140,
                modes: LaserModesDef {
                    offence: Some(LaserModeDef { cycle_ms: 1_000 }),
                    defence: None,
                },
            }),
            note: String::new(),
        }
    }

    fn tubes_row() -> SpaceMountDef {
        SpaceMountDef {
            label: "Tubes".to_owned(),
            family: SpaceMountFamily::Tubes(TubesDef {
                salvo: 2,
                reload_ms: 8_000,
                accel_mps2: 250,
                dv_mps: 12_000,
                warhead_damage: 180,
            }),
            note: String::new(),
        }
    }

    fn fit(mounts: Vec<SpaceFitMount>) -> SpaceFitDef {
        SpaceFitDef {
            label: "Hull".to_owned(),
            unit: id("tacsim.unit.dray"),
            pixel_model: id("freighter"),
            decoys: 4,
            rcs: SpaceRcsDef {
                trans_dv_mmps_per_frame: 30,
                closure_dv_mmps_per_frame: 25,
                attitude_rate_mdeg_s_per_frame: 180,
                roll_rate_mdeg_s_per_frame: 240,
                damp_trans_mmps_per_frame: 15,
                damp_attitude_mdeg_s_per_frame: 90,
                damp_roll_mdeg_s_per_frame: 120,
                kg_per_pulse_batch: 1,
            },
            mounts,
            note: String::new(),
        }
    }

    fn mount(
        on: &str,
        row: &str,
        magazine: Option<u32>,
        modes: &[LaserModeWord],
        mode: Option<LaserModeWord>,
    ) -> SpaceFitMount {
        SpaceFitMount {
            id: id(on),
            mount: id(row),
            magazine,
            modes: modes.to_vec(),
            mode,
        }
    }

    fn rows(id: &CanonicalId) -> Option<&'static SpaceMountDef> {
        static LASER: std::sync::OnceLock<SpaceMountDef> = std::sync::OnceLock::new();
        static TUBES: std::sync::OnceLock<SpaceMountDef> = std::sync::OnceLock::new();
        match id.as_str() {
            "row.laser" => Some(LASER.get_or_init(laser_row)),
            "row.tubes" => Some(TUBES.get_or_init(tubes_row)),
            _ => None,
        }
    }

    #[test]
    fn cc6_a_space_reach_runs_past_the_grounds_hundred_kilometres_and_is_bounded() {
        let mut row = laser_row();
        assert!(row.check("row").is_ok(), "500 km is a space reach");
        if let SpaceMountFamily::Laser(laser) = &mut row.family {
            laser.max_range_m = MAX_SPACE_RANGE_M + 1;
        }
        assert!(row.check("row").is_err());
        let mut idle = laser_row();
        if let SpaceMountFamily::Laser(laser) = &mut idle.family {
            laser.modes = LaserModesDef::default();
        }
        assert!(idle.check("row").is_err(), "a laser works in some mode");
        let mut fast = tubes_row();
        if let SpaceMountFamily::Tubes(tubes) = &mut fast.family {
            tubes.reload_ms = SPACE_CYCLE_FLOOR_MS - 1;
        }
        assert!(fast.check("row").is_err(), "no cycle under one substep");
    }

    #[test]
    fn cc6_a_fits_references_are_checked() {
        let offence = [LaserModeWord::Offence];
        let good = fit(vec![
            mount(
                "laser_1",
                "row.laser",
                None,
                &offence,
                Some(LaserModeWord::Offence),
            ),
            mount("torp_bow", "row.tubes", Some(8), &[], None),
        ]);
        assert!(good.check("fit").is_ok());
        assert!(good.check_against("fit", rows).is_ok());

        let refused = |mounts: Vec<SpaceFitMount>| {
            let fit = fit(mounts);
            fit.check("fit").is_err() || fit.check_against("fit", rows).is_err()
        };
        assert!(
            refused(vec![mount("a", "row.none", Some(1), &[], None)]),
            "no such row"
        );
        assert!(
            refused(vec![mount("a", "row.tubes", None, &[], None)]),
            "tubes name a magazine"
        );
        assert!(
            refused(vec![mount(
                "a",
                "row.laser",
                Some(1),
                &offence,
                Some(LaserModeWord::Offence)
            )]),
            "a laser keeps no magazine"
        );
        assert!(
            refused(vec![mount("a", "row.laser", None, &[], None)]),
            "a laser names its modes"
        );
        assert!(
            refused(vec![mount(
                "a",
                "row.laser",
                None,
                &[LaserModeWord::Defence],
                Some(LaserModeWord::Defence)
            )]),
            "a mode the row does not work in"
        );
        assert!(
            refused(vec![mount(
                "a",
                "row.laser",
                None,
                &offence,
                Some(LaserModeWord::Defence)
            )]),
            "a start mode the fit does not allow"
        );
        assert!(
            refused(vec![mount(
                "a",
                "row.tubes",
                Some(1),
                &offence,
                Some(LaserModeWord::Offence)
            )]),
            "tubes have no modes"
        );
        assert!(
            refused(vec![
                mount("a", "row.tubes", Some(1), &[], None),
                mount("a", "row.tubes", Some(1), &[], None),
            ]),
            "one id, one mount"
        );
        assert!(
            refused(vec![mount(
                "a",
                "row.tubes",
                Some(MAX_MAGAZINE + 1),
                &[],
                None
            )]),
            "a magazine past its cap"
        );
    }
}
