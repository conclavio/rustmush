//! **What a unit shoots with** (16b cut C.2, C1; doc 16 §3.5 "Weapon
//! definition", §4 "Content packs").
//!
//! Every number cut W's resolution reads, authored: how the shot is delivered,
//! where it is mounted, the range bands and their dispersion, the cycle floor of
//! W14, the ammunition it names, the burst, the muzzle speed the flight time is
//! computed from, and the heat cell that exists for the laser of §9 and is zero
//! on both stock weapons.
//!
//! Integers only (D-16-4): centimetres, milliradians, milliseconds, cm/s.

use mush_core::CanonicalId;
use serde::{Deserialize, Serialize};

use crate::definitions::DefinitionError;

/// **The cycle floor** (doc 49 Q-49-2, Q-49-19, unit CC.2): no mount a player
/// or a mind fires cycles faster than five seconds.
///
/// The owner's ruling of 2026-11-15, from play-testing ("the shots happen too
/// fast"): at 5 s the cadence tax (the 250 ms lattice plus typing time) is
/// about a twentieth of a cycle, and every shot is a decision. It is refused on
/// load, so a content edit cannot bring back a one-second gun. An
/// automatic-defence mount ([`WeaponDef::automatic_defence`]) keeps
/// [`AUTOMATIC_DEFENCE_FLOOR_MS`] instead. Until CC.2 the floor was W14's
/// 1,000 ms (16b §9), which is now the automatic-defence floor.
pub const CYCLE_FLOOR_MS: u32 = 5_000;

/// **W14's floor, kept for automatic defence** (16b §9 "Constants"; doc 49
/// Q-49-2, Q-49-10): an automatic-defence mount (point defence, a laser in its
/// defence mode) answers incoming rounds, not contacts, and nobody types it,
/// so it keeps the engine's own floor. That floor is a property of the
/// engine's tempo: `Tempo::Engaged` is 500 ms, and a mount that could fire
/// every substep would make a match's shot count a function of how finely the
/// arena happened to substep. No ground mount ships in this class yet.
pub const AUTOMATIC_DEFENCE_FLOOR_MS: u32 = 1_000;

/// The slowest a weapon may cycle: an hour, past any match at the stock cap
/// (fifteen minutes since 16b cut AP.2; `MatchConfig::validate` sets the cap no
/// ceiling).
pub const MAX_CYCLE_MS: u32 = 3_600_000;

/// The longest range band one weapon may declare: a hundred kilometres, two
/// orders of magnitude past the 2.56 km arena (D-16-5).
pub const MAX_RANGE_CM: u32 = 10_000_000;

/// The most range bands one weapon may declare.
pub const MAX_RANGE_BANDS: usize = 8;

/// The widest dispersion one band may declare: ten radians is past a circle, so
/// anything at or above it is an authoring mistake rather than a wild weapon.
pub const MAX_DISPERSION_MRAD: u32 = 10_000;

/// The most rounds one burst may fire.
pub const MAX_BURST_ROUNDS: u32 = 1_000;

/// The fastest a projectile may fly, cm/s: ten per cent of light, which is past
/// anything the corridor builds and short of the flight time overflowing.
pub const MAX_MUZZLE_SPEED_CMS: u32 = 3_000_000_000;

/// The most heat one shot may add.
pub const MAX_HEAT_PER_SHOT: u32 = 100_000;

/// How a shot reaches its target (doc 16 §3.5).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// A projectile: `impact_at_ms = now + range_cm × 1000 / muzzle_speed_cms`.
    Direct,
    /// A beam: `impact_at_ms = now` (doc 16 §9 item 1; no stock weapon uses it).
    Beam,
}

impl Delivery {
    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Delivery::Direct => "direct",
            Delivery::Beam => "beam",
        }
    }
}

/// Where a weapon sits on its unit: the section that carries it and the facet it
/// points out of (doc 16 §3.5 "mount").
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    /// The section, named in the unit's own section table.
    pub section: CanonicalId,
    /// The facet, named in that section's facet table.
    pub facet: CanonicalId,
}

/// One range band: everything inside `max_range_cm` and outside the previous
/// band's, at this dispersion (doc 16 §3.5 step 2).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeBand {
    /// The band's outer edge, centimetres.
    pub max_range_cm: u32,
    /// The weapon's own error cone in this band, milliradians.
    pub dispersion_mrad: u32,
}

/// One weapon (doc 16 §3.5 "Weapon definition").
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponDef {
    /// The human-facing name.
    pub label: String,
    /// Projectile or beam.
    pub delivery: Delivery,
    /// Section and facet.
    pub mount: Mount,
    /// The range bands, nearest first.
    pub bands: Vec<RangeBand>,
    /// The cycle, milliseconds. At or above [`CYCLE_FLOOR_MS`], or
    /// [`AUTOMATIC_DEFENCE_FLOOR_MS`] for an automatic-defence mount.
    pub cycle_ms: u32,
    /// The ammunition family this weapon fires — an entry id another pack entry
    /// declares, checked at load.
    pub ammunition: CanonicalId,
    /// Rounds per pull: 1 for the cannon, `k` for the machine gun.
    pub burst_rounds: u32,
    /// Muzzle speed, cm/s. Zero exactly when `delivery` is `beam`.
    pub muzzle_speed_cms: u32,
    /// Heat per shot. Zero on both stock weapons; the cell exists for lasers.
    pub heat_per_shot: u32,
    /// **An automatic-defence mount** (doc 49 Q-49-2, Q-49-10, unit CC.2):
    /// one that answers incoming rounds on its own, which nobody types. It
    /// keeps [`AUTOMATIC_DEFENCE_FLOOR_MS`] where every other mount is held to
    /// [`CYCLE_FLOOR_MS`]. Absent (and so `false`) on every shipped ground
    /// mount, and written only when set, so a pack that does not set it signs
    /// as it did. **Refused at load for now** (doc 48e kernel G13): no ground
    /// fire path answers incoming rounds on its own, so a mount set so would be
    /// an ordinary gun under the 1 s floor.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub automatic_defence: bool,
    /// Why these numbers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl WeaponDef {
    /// The weapon's longest range, centimetres — what a match's
    /// `tempo.alert_range_cm` is derived from (16b §9).
    #[must_use]
    pub fn max_range_cm(&self) -> u32 {
        self.bands
            .iter()
            .map(|band| band.max_range_cm)
            .max()
            .unwrap_or(0)
    }

    /// **The floor this mount's cycle is held to** (doc 49 unit CC.2):
    /// [`AUTOMATIC_DEFENCE_FLOOR_MS`] for an automatic-defence mount, else
    /// [`CYCLE_FLOOR_MS`].
    #[must_use]
    pub const fn cycle_floor_ms(&self) -> u32 {
        if self.automatic_defence {
            AUTOMATIC_DEFENCE_FLOOR_MS
        } else {
            CYCLE_FLOOR_MS
        }
    }

    /// Re-run every bound.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry and the field.
    pub fn check(&self, entry: &str) -> Result<(), DefinitionError> {
        crate::definitions::check_label(entry, &self.label)?;
        crate::definitions::check_note(entry, &self.note)?;
        if self.bands.is_empty() || self.bands.len() > MAX_RANGE_BANDS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "a weapon declares 1 to {MAX_RANGE_BANDS} range bands; {} is not",
                    self.bands.len()
                ),
            ));
        }
        let mut previous = 0_u32;
        for band in &self.bands {
            if band.max_range_cm <= previous || band.max_range_cm > MAX_RANGE_CM {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "range bands run outward and stop at {MAX_RANGE_CM} cm; \
                         {} after {previous} does not",
                        band.max_range_cm
                    ),
                ));
            }
            if band.dispersion_mrad == 0 || band.dispersion_mrad > MAX_DISPERSION_MRAD {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "dispersion_mrad is 1 to {MAX_DISPERSION_MRAD}; {} is not",
                        band.dispersion_mrad
                    ),
                ));
            }
            previous = band.max_range_cm;
        }
        let floor = self.cycle_floor_ms();
        if self.cycle_ms < floor || self.cycle_ms > MAX_CYCLE_MS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "cycle_ms is {floor} to {MAX_CYCLE_MS} ({}); {} is not",
                    if self.automatic_defence {
                        "the automatic-defense floor"
                    } else {
                        "the 5 s floor"
                    },
                    self.cycle_ms
                ),
            ));
        }
        // Doc 48e kernel G13: no ground fire path answers incoming rounds on its own yet, so a
        // mount marked automatic would be an ordinary gun under the 1 s floor, typed and fired
        // by players and minds. It is refused at load until that path exists.
        if self.automatic_defence {
            return Err(DefinitionError::value(
                entry,
                "automatic_defence: no ground mount fires on its own yet, so none may be marked                  automatic (it would be a 1 s gun a player could fire)",
            ));
        }
        if self.burst_rounds == 0 || self.burst_rounds > MAX_BURST_ROUNDS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "burst_rounds is 1 to {MAX_BURST_ROUNDS}; {} is not",
                    self.burst_rounds
                ),
            ));
        }
        match self.delivery {
            Delivery::Direct
                if self.muzzle_speed_cms == 0 || self.muzzle_speed_cms > MAX_MUZZLE_SPEED_CMS =>
            {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "a direct weapon's muzzle_speed_cms is 1 to {MAX_MUZZLE_SPEED_CMS} \
                         (the flight time divides by it); {} is not",
                        self.muzzle_speed_cms
                    ),
                ));
            }
            Delivery::Beam if self.muzzle_speed_cms != 0 => {
                return Err(DefinitionError::value(
                    entry,
                    "a beam arrives at once, so its muzzle_speed_cms is 0".to_string(),
                ));
            }
            _ => {}
        }
        if self.heat_per_shot > MAX_HEAT_PER_SHOT {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "heat_per_shot is at most {MAX_HEAT_PER_SHOT}; {} is not",
                    self.heat_per_shot
                ),
            ));
        }
        Ok(())
    }
}
