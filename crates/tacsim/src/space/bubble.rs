//! **Tier-2 Local Space Bubble** (`docs/design/35-ship-free-flight-combat-and-mining.md`
//! §6.1–§6.3, `D-35-1`, `D-35-3`, `D-35-5`, `D-35-7`, `D-35-8`, `D-35-12`).
//!
//! A [`LocalSpaceBubble`] is a bounded 3D local-space domain ($\pm 100{,}000\text{ km}$)
//! co-moving with an orbital site, station, asteroid cluster, or interception point:
//! - Advances on **1:1 wall time** (`Q-CF-32`, adaptive step with substeps at most
//!   [`MAX_SUBSTEP_MS`] = $250\text{ ms}$, `D-16-2`).
//! - Reuses `dray.json`'s canonical sections ([`ShipSectionId`]: `hull`, `hold`, `drive`)
//!   and facet bands ([`FacetBand`]: `front`, `left`, `right`, `rear`, `top`) (`D-35-1`).
//! - Never permadeaths a hull (`FO-12`, `D-35-12`): disabling `drive` or critical `hull`
//!   transitions the ship into [`HullStatus::DisabledDerelict`] with `epoch_open: true`
//!   and a 48 h ($172{,}800\text{ s}$) life-support & salvage contest window.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::arena::party::{Mark, PartyId, Relation};
use crate::space::weapons::{
    DEFAULT_TORPEDO_MAX_FLIGHT_MS, InFlightSlug, InFlightTorpedo, LaserMode, LaserMount, PdcMount,
    RailgunMount, TorpedoTube, compute_railgun_lead, isqrt_i128,
};

/// Maximum substep duration in wall milliseconds (`D-16-2`).
pub const MAX_SUBSTEP_MS: u32 = 250;

/// Default 48-hour open-epoch derelict life-support and salvage contest window in seconds (`D-35-12`).
pub const DERELICT_WINDOW_S: u32 = 48 * 3_600;

/// Identifier of a [`LocalSpaceBubble`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BubbleId(pub String);

/// **The `n`-th party admitted to a bubble, lettered by the arena** (Q-CF-11,
/// Q-40-22): `A` first. Past the twenty-third it stays on the last letter, as a
/// bubble never admits that many.
#[must_use]
pub const fn admitted_party(n: usize) -> PartyId {
    match PartyId::nth(n) {
        Some(party) => party,
        None => PartyId::known(b'Z'),
    }
}

/// **The space track's party wire** (Q-40-22): a [`SpaceShipState`]'s party
/// persists as its admission number from 1 (`A` is 1), the number the documents
/// already hold, so the seam moves no persisted document. 0 reads as `A`, as it
/// always did.
pub(super) mod party_wire {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{PartyId, admitted_party};
    use crate::arena::party::LETTERS;

    pub(in crate::space) fn serialize<S: Serializer>(
        party: &PartyId,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let at = LETTERS.iter().position(|&b| b == party.code()).unwrap_or(0);
        u32::try_from(at)
            .unwrap_or(0)
            .saturating_add(1)
            .serialize(serializer)
    }

    pub(in crate::space) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<PartyId, D::Error> {
        let number = u32::deserialize(deserializer)?;
        Ok(admitted_party(
            usize::try_from(number.saturating_sub(1)).unwrap_or(usize::MAX),
        ))
    }
}

/// **The feed word a holotank or cockpit reads for `relation`** (`D-35-1`): own
/// and friend read `friend`, neutral and unknown read `neutral`.
#[must_use]
pub const fn relation_feed_word(relation: Relation) -> &'static str {
    match relation {
        Relation::Own | Relation::Friend => "friend",
        Relation::Hostile => "hostile",
        Relation::Neutral | Relation::Unknown => "neutral",
    }
}

/// Canonical armour facet bands matching `content/units/dray.json` (`D-35-1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FacetBand {
    Front,
    Left,
    Right,
    Rear,
    Top,
}

impl FacetBand {
    #[must_use]
    pub fn id_str(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Left => "left",
            Self::Right => "right",
            Self::Rear => "rear",
            Self::Top => "top",
        }
    }
}

/// Canonical hull sections matching `content/units/dray.json` (`D-35-1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ShipSectionId {
    Hull,
    Hold,
    Drive,
}

impl ShipSectionId {
    #[must_use]
    pub fn id_str(self) -> &'static str {
        match self {
            Self::Hull => "hull",
            Self::Hold => "hold",
            Self::Drive => "drive",
        }
    }

    /// Map a space hull section onto the shared [`crate::carrier_damage::CarrierPart`]
    /// table (`Q-36-47`, `SF.R20`).
    #[must_use]
    pub const fn carrier_part(self) -> Option<crate::carrier_damage::CarrierPart> {
        match self {
            Self::Hull => Some(crate::carrier_damage::CarrierPart::Hull),
            Self::Drive => Some(crate::carrier_damage::CarrierPart::Drive),
            Self::Hold => None,
        }
    }
}

/// Subsystem critical hits (`D-35-1`, §6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SubsystemCrit {
    /// Halves radiator heat dissipation and raises IR signature.
    RadiatorDamaged,
    /// Reduces maximum drive thrust by 50%.
    DriveDegraded,
    /// Knocks out a point-defense cannon arc.
    PdcOffline,
    /// Degrades passive sensor lock range.
    SensorDegraded,
}

/// Operational vs. open-epoch derelict state of a space hull (`D-35-12`, `FO-12`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HullStatus {
    Operational,
    /// Disabled hull whose lifecycle epoch stays open (`doc 23`, `doc 27a`).
    DisabledDerelict {
        epoch_open: bool,
        life_support_remaining_s: u32,
        salvage_contest_window_s: u32,
    },
}

/// Authored section destruction consequence from `UnitSpec::sections` (`D-35-1`, `SF.R15`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpaceSectionConsequence {
    DestroyUnit,
    DisableMotion,
    Slow { percent: u32 },
}

/// Armour and structure state for one section of a ship (`D-35-1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipSectionState {
    pub id: ShipSectionId,
    pub facets: BTreeMap<FacetBand, u32>,
    pub internals: u32,
    pub max_internals: u32,
    /// Optional armour facet on another section through which this section is struck
    /// (e.g. `drive` struck through `(Hull, Rear)` in `dray.json`, `D-35-1`, `SF.R15`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub struck_through: Option<(ShipSectionId, FacetBand)>,
    /// Authored consequences applied when this section reaches `0` internals (`D-35-1`, `SF.R15`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consequences: Vec<SpaceSectionConsequence>,
}

impl ShipSectionState {
    /// Map this section's authored consequences (or section id when none are authored)
    /// onto [`crate::carrier_damage::CarrierPart`] (`Q-36-47`, `SF.R20`).
    #[must_use]
    pub fn carrier_part(&self) -> Option<crate::carrier_damage::CarrierPart> {
        if self.consequences.is_empty() {
            return self.id.carrier_part();
        }
        if self
            .consequences
            .iter()
            .any(|c| matches!(c, SpaceSectionConsequence::DestroyUnit))
        {
            Some(crate::carrier_damage::CarrierPart::Hull)
        } else if self
            .consequences
            .iter()
            .any(|c| matches!(c, SpaceSectionConsequence::DisableMotion))
        {
            Some(crate::carrier_damage::CarrierPart::Drive)
        } else {
            None
        }
    }
}

fn default_dray_facet_weights() -> BTreeMap<FacetBand, Vec<(ShipSectionId, u32)>> {
    BTreeMap::from([
        (FacetBand::Front, vec![(ShipSectionId::Hull, 100)]),
        (
            FacetBand::Left,
            vec![(ShipSectionId::Hull, 50), (ShipSectionId::Hold, 50)],
        ),
        (
            FacetBand::Right,
            vec![(ShipSectionId::Hull, 50), (ShipSectionId::Hold, 50)],
        ),
        (
            FacetBand::Rear,
            vec![(ShipSectionId::Hull, 50), (ShipSectionId::Drive, 50)],
        ),
        (
            FacetBand::Top,
            vec![(ShipSectionId::Hull, 40), (ShipSectionId::Hold, 60)],
        ),
    ])
}

fn is_default_dray_facet_weights(weights: &BTreeMap<FacetBand, Vec<(ShipSectionId, u32)>>) -> bool {
    *weights == default_dray_facet_weights()
}

fn is_default_rcs_authority(rcs: &super::docking::DockingRcsAuthority) -> bool {
    *rcs == super::docking::DockingRcsAuthority::default()
}

/// Emissions control posture (`mush-sim` `Emcon{0..=3}`, `D-35-9`, `D-35-10`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EmconPosture {
    /// Running dark: active sensors off, radiators retracted (minimal cooling).
    Ghost = 0,
    /// Passive sensors only, reduced thermal output.
    Passive = 1,
    /// Normal cruise posture.
    #[default]
    Standard = 2,
    /// Full active radar/lidar sweep and combat telemetry.
    ActiveCombat = 3,
}

impl EmconPosture {
    #[must_use]
    pub fn code_str(self) -> &'static str {
        match self {
            Self::Ghost => "GHOST",
            Self::Passive => "PASSIVE",
            Self::Standard => "STD",
            Self::ActiveCombat => "COMBAT",
        }
    }
}

fn default_dry_mass_t() -> u32 {
    240
}

fn is_default_dry_mass_t(v: &u32) -> bool {
    *v == 240
}

fn default_payload_ceiling_kg() -> u64 {
    181_000
}

fn is_default_payload_ceiling_kg(v: &u64) -> bool {
    *v == 181_000
}

fn default_propellant_cap_kg() -> u32 {
    80_000
}

fn is_default_propellant_cap_kg(v: &u32) -> bool {
    *v == 80_000
}

fn default_rated_accel_mg() -> u32 {
    50
}

fn is_default_rated_accel_mg(v: &u32) -> bool {
    *v == 50
}

fn default_exhaust_kms() -> u32 {
    6_000
}

fn is_default_exhaust_kms(v: &u32) -> bool {
    *v == 6_000
}

fn default_slow_pct() -> u8 {
    100
}

fn is_default_slow_pct(v: &u8) -> bool {
    *v == 100
}

fn default_pixel_model() -> String {
    "freighter".to_owned()
}

fn is_default_pixel_model(v: &String) -> bool {
    v == "freighter"
}

fn is_zero_vec3(v: &[i64; 3]) -> bool {
    *v == [0, 0, 0]
}

fn is_zero_attitude(v: &(i32, i32, i32)) -> bool {
    *v == (0, 0, 0)
}

fn is_zero_u16(v: &u16) -> bool {
    *v == 0
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}

/// A ship or hull inside a [`LocalSpaceBubble`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceShipState {
    pub id: String,
    pub callsign: String,
    pub hull_class: String,
    /// Canonical `mush-pixel` 3D sprite archetype from the hull's [`crate::space::custody::SpaceUnitOverlay`] (`SF.R13`, `SF.R20`, `D-35-15`).
    #[serde(
        default = "default_pixel_model",
        skip_serializing_if = "is_default_pixel_model"
    )]
    pub pixel_model: String,
    /// The ship's party, lettered by the arena; its wire stays the space
    /// track's admission number (Q-40-22: no persisted document moves).
    #[serde(with = "party_wire")]
    pub party: PartyId,
    /// 3D local position in metres (`[x, y, z]`).
    pub pos_m: [i64; 3],
    /// Sub-metre position remainder in micrometres (`-999_999..=999_999`, `35a` §5.2).
    #[serde(default, skip_serializing_if = "is_zero_vec3")]
    pub pos_rem_um: [i64; 3],
    /// 3D local velocity in $\text{mm/s}$ (`[vx, vy, vz]`).
    pub vel_mmps: [i64; 3],
    /// Commanded 3D thrust acceleration in $\text{m/s}^2$ (`[ax, ay, az]`).
    #[serde(default, skip_serializing_if = "is_zero_vec3")]
    pub thrust_mps2: [i64; 3],
    /// Commanded 3D thrust acceleration in $\text{mm/s}^2$ (`[ax, ay, az]`, `SF.R20`).
    #[serde(default, skip_serializing_if = "is_zero_vec3")]
    pub thrust_mmps2: [i64; 3],
    /// Maximum maneuvering / combat thrust acceleration in $\text{m/s}^2$.
    pub max_accel_mps2: i64,
    /// Mass-adjusted maximum flip-drive acceleration in $\text{mm/s}^2$ (`SF.R20`).
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub max_accel_mmps2: i64,
    /// Dry hull mass in tonnes from `UnitSpec::logistics` (`D-35-1`, `240 t` for Dray/Dray-P, `4,000 t` for Halcyon).
    #[serde(
        default = "default_dry_mass_t",
        skip_serializing_if = "is_default_dry_mass_t"
    )]
    pub dry_mass_t: u32,
    /// Payload ceiling in kilograms from `UnitSpec::logistics` (`D-35-1`).
    #[serde(
        default = "default_payload_ceiling_kg",
        skip_serializing_if = "is_default_payload_ceiling_kg"
    )]
    pub payload_ceiling_kg: u64,
    /// Propellant tank capacity in kilograms from `UnitSpec::logistics.stowage["propellant"]` (`D-35-1`).
    #[serde(
        default = "default_propellant_cap_kg",
        skip_serializing_if = "is_default_propellant_cap_kg"
    )]
    pub propellant_cap_kg: u32,
    /// Unimpaired rated flip-drive continuous acceleration in milli-g from `UnitSpec::logistics.drive.law` (`D-35-1`, `SF.R20`).
    #[serde(
        default = "default_rated_accel_mg",
        skip_serializing_if = "is_default_rated_accel_mg"
    )]
    pub base_rated_accel_mg: u32,
    /// Current `Slow` consequence cap in percent (`1..=100`, `100` when unimpaired, `SF.R20`).
    #[serde(
        default = "default_slow_pct",
        skip_serializing_if = "is_default_slow_pct"
    )]
    pub slow_pct: u8,
    /// Rated flip-drive continuous acceleration in milli-g from `UnitSpec::logistics.drive.law` (`50 mg` $\approx 0.49\text{ m/s}^2$, `D-35-1`).
    #[serde(
        default = "default_rated_accel_mg",
        skip_serializing_if = "is_default_rated_accel_mg"
    )]
    pub rated_accel_mg: u32,
    /// Drive exhaust velocity in $\text{km/s}$ from `UnitSpec::logistics.drive.law` (`6,000 km/s`, `D-35-1`).
    #[serde(
        default = "default_exhaust_kms",
        skip_serializing_if = "is_default_exhaust_kms"
    )]
    pub exhaust_kms: u32,
    /// Hull attitude `(yaw_deg, pitch_deg, roll_deg)` in whole degrees.
    #[serde(default, skip_serializing_if = "is_zero_attitude")]
    pub attitude_deg: (i32, i32, i32),
    /// Remaining reaction mass in `kg` (`D-35-6`).
    pub propellant_kg: u32,
    /// Remaining $\Delta v$ reserve in $\text{m/s}$.
    pub delta_v_mps: i64,
    /// Minimum $\Delta v$ needed to return to the nearest open port (`RETURN_RESERVE`, `D-35-6`).
    pub return_reserve_dv_mps: i64,
    /// Hull heat cell in permille (`0..=1000`, `D-35-1`).
    #[serde(default, skip_serializing_if = "is_zero_u16")]
    pub heat_permille: u16,
    /// Base radiator cooling rate in permille per wall second.
    pub cooling_rate_permille_s: u16,
    /// Whether automatic point-defense interception (`+tac/pdc auto`) is enabled (`Q-35-4`).
    pub pdc_auto: bool,
    /// Whether all crew are currently linkdead (`D-35-7` ballistic safety rule).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub crew_linkdead: bool,
    pub emcon: EmconPosture,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked_target_id: Option<String>,
    /// Effective hull cross-section radius for unguided projectile impacts in metres.
    pub hit_radius_m: i64,
    pub sections: BTreeMap<ShipSectionId, ShipSectionState>,
    /// Authored facet band section weights from `UnitSpec::facets` (`D-35-1`, `SF.R15`).
    #[serde(
        default = "default_dray_facet_weights",
        skip_serializing_if = "is_default_dray_facet_weights"
    )]
    pub facet_weights: BTreeMap<FacetBand, Vec<(ShipSectionId, u32)>>,
    /// 6-DOF RCS thruster authority from the hull's [`crate::space::custody::SpaceUnitOverlay`] (`D-35-1`, `SF.R15`).
    #[serde(default, skip_serializing_if = "is_default_rcs_authority")]
    pub rcs_authority: super::docking::DockingRcsAuthority,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crits: Vec<SubsystemCrit>,
    pub status: HullStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub torpedo_tubes: Vec<TorpedoTube>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lasers: Vec<LaserMount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub railguns: Vec<RailgunMount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pdcs: Vec<PdcMount>,
    /// Remaining ECM / thermal-radar decoy canisters (`SF.15`, `D-35-8`).
    #[serde(default)]
    pub decoys_remaining: u8,
    /// Remaining active duration of a deployed decoy cloud in wall milliseconds (`SF.15`).
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub active_decoy_ms: u32,
    /// Non-propellant cargo/bay payload mass in kilograms for rocket-equation burns (`SF.R11`, `D-35-6`).
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub non_prop_payload_kg: u64,
    /// Sub-$\text{m/s}$ $\Delta v$ remainder in $\text{mm/s}$ accumulated across `250 ms` substeps (`SF.R11`).
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub dv_rem_mmps: i64,
}

impl SpaceShipState {
    /// Construct a `Dray` / `Dray-P` bulk/prospector hull with the exact section and
    /// facet armour values from `crates/tacsim/content/units/dray.json` (`D-35-1`).
    #[must_use]
    pub fn new_dray(
        id: impl Into<String>,
        callsign: impl Into<String>,
        party: PartyId,
        pos_m: [i64; 3],
        vel_mmps: [i64; 3],
    ) -> Self {
        let mut sections = BTreeMap::new();
        sections.insert(
            ShipSectionId::Hull,
            ShipSectionState {
                id: ShipSectionId::Hull,
                facets: BTreeMap::from([
                    (FacetBand::Front, 120),
                    (FacetBand::Left, 80),
                    (FacetBand::Right, 80),
                    (FacetBand::Rear, 60),
                    (FacetBand::Top, 60),
                ]),
                internals: 600,
                max_internals: 600,
                struck_through: None,
                consequences: vec![SpaceSectionConsequence::DestroyUnit],
            },
        );
        sections.insert(
            ShipSectionId::Hold,
            ShipSectionState {
                id: ShipSectionId::Hold,
                facets: BTreeMap::from([
                    (FacetBand::Left, 60),
                    (FacetBand::Right, 60),
                    (FacetBand::Top, 50),
                ]),
                internals: 400,
                max_internals: 400,
                struck_through: None,
                consequences: vec![SpaceSectionConsequence::Slow { percent: 75 }],
            },
        );
        sections.insert(
            ShipSectionId::Drive,
            ShipSectionState {
                id: ShipSectionId::Drive,
                facets: BTreeMap::new(),
                internals: 250,
                max_internals: 250,
                struck_through: Some((ShipSectionId::Hull, FacetBand::Rear)),
                consequences: vec![SpaceSectionConsequence::DisableMotion],
            },
        );

        let mut ship = Self {
            id: id.into(),
            callsign: callsign.into(),
            hull_class: "tacsim.unit.dray".to_owned(),
            pixel_model: "freighter".to_owned(),
            party,
            pos_m,
            pos_rem_um: [0, 0, 0],
            vel_mmps,
            thrust_mps2: [0, 0, 0],
            thrust_mmps2: [0, 0, 0],
            max_accel_mps2: 1,
            max_accel_mmps2: 645,
            dry_mass_t: 240,
            payload_ceiling_kg: 181_000,
            propellant_cap_kg: 80_000,
            base_rated_accel_mg: 50,
            slow_pct: 100,
            rated_accel_mg: 50,
            exhaust_kms: 6_000,
            attitude_deg: (0, 0, 0),
            propellant_kg: 80_000,
            delta_v_mps: 0,
            return_reserve_dv_mps: 900,
            heat_permille: 0,
            cooling_rate_permille_s: 30,
            pdc_auto: true,
            crew_linkdead: false,
            emcon: EmconPosture::Standard,
            locked_target_id: None,
            hit_radius_m: 5,
            sections,
            facet_weights: default_dray_facet_weights(),
            rcs_authority: super::docking::DockingRcsAuthority::default(),
            crits: Vec::new(),
            status: HullStatus::Operational,
            torpedo_tubes: Vec::new(),
            lasers: vec![
                LaserMount::standard_turret("laser_dorsal"),
                LaserMount::standard_turret("laser_ventral"),
            ],
            railguns: Vec::new(),
            pdcs: vec![
                PdcMount::standard_rotary("pdc_dorsal", 2_000),
                PdcMount::standard_rotary("pdc_ventral", 2_000),
            ],
            decoys_remaining: 4,
            active_decoy_ms: 0,
            non_prop_payload_kg: 0,
            dv_rem_mmps: 0,
        };
        if let Some(spec) = super::custody::unit_spec_for_id("tacsim.unit.dray") {
            super::custody::apply_unit_spec_to_space_ship(&mut ship, spec);
        } else {
            ship.recompute_drive_kinematics();
        }
        super::custody::apply_space_unit_overlay(
            &mut ship,
            &super::custody::space_unit_overlay_for_id("tacsim.unit.dray"),
        );
        ship
    }

    /// Rated continuous flip-drive acceleration in $\text{m/s}^2$ from [`Self::rated_accel_mg`]
    /// (`50 mg` $\approx 0.4903\text{ m/s}^2$, `D-35-1`).
    #[must_use]
    pub fn rated_accel_mps2(&self) -> f64 {
        f64::from(self.rated_accel_mg) / 1_000.0 * crate::port::passage::G0_MPS2
    }

    fn hull_drive(&self) -> Option<crate::port::passage::HullDrive> {
        Some(crate::port::passage::HullDrive {
            class: mush_core::CanonicalId::new("space").ok()?,
            law: crate::definitions::logistics::DriveLaw::Flip {
                accel_mg: self.rated_accel_mg,
                exhaust_kms: self.exhaust_kms.max(1),
            },
            payload_ceiling_kg: self
                .payload_ceiling_kg
                .max(u64::from(self.propellant_cap_kg)),
            throttle_permille: crate::port::passage::MAX_THROTTLE_PERMILLE,
        })
    }

    /// Recompute [`Self::rated_accel_mg`], [`Self::max_accel_mmps2`], and [`Self::max_accel_mps2`]
    /// from [`Self::base_rated_accel_mg`], [`Self::slow_pct`], and the hull's current mass (`SF.R20`).
    pub fn recompute_drive_accel(&mut self) {
        let pct = u32::from(self.slow_pct.clamp(1, 100));
        self.rated_accel_mg = (self.base_rated_accel_mg.saturating_mul(pct) / 100).max(1);
        let dry_kg = u64::from(self.dry_mass_t).saturating_mul(1_000);
        let payload_kg = self
            .non_prop_payload_kg
            .saturating_add(u64::from(self.propellant_kg));
        let accel_mps2 = self
            .hull_drive()
            .and_then(|hd| crate::port::passage::flip_accel_mps2(&hd, dry_kg, payload_kg))
            .unwrap_or_else(|| self.rated_accel_mps2());
        self.max_accel_mmps2 = ((accel_mps2 * 1_000.0).round() as i64).max(1);
        self.max_accel_mps2 = (accel_mps2.ceil() as i64).max(1);
    }

    /// Recompute both mass-adjusted drive acceleration ([`Self::recompute_drive_accel`]) and
    /// rocket-equation [`Self::delta_v_mps`] from the hull's `UnitSpec` drive law (`SF.R20`).
    pub fn recompute_drive_kinematics(&mut self) {
        self.recompute_drive_accel();
        let dry_kg = u64::from(self.dry_mass_t).saturating_mul(1_000);
        let payload_kg = self
            .non_prop_payload_kg
            .saturating_add(u64::from(self.propellant_kg));
        if let Some(hull_drive) = self.hull_drive()
            && let Ok(dv) = crate::port::passage::remaining_dv_mps_for_propellant(
                &hull_drive,
                dry_kg,
                payload_kg,
                u64::from(self.propellant_kg),
            )
        {
            self.delta_v_mps = i64::from(dv);
        }
    }

    /// Evaluate the shared [`crate::carrier_damage::CarrierDamage`] table for this ship's
    /// zero-internals sections (`Q-36-47`, `SF.R20`).
    #[must_use]
    pub fn carrier_damage(&self) -> crate::carrier_damage::CarrierDamage {
        crate::carrier_damage::CarrierDamage::of_parts(
            self.sections
                .values()
                .filter(|sec| sec.internals == 0)
                .filter_map(ShipSectionState::carrier_part),
        )
    }

    /// Construct an armed corvette / raider hull with torpedo tubes, spinal railgun,
    /// dual-mode laser turret, and rotary PDCs.
    #[must_use]
    pub fn new_corvette(
        id: impl Into<String>,
        callsign: impl Into<String>,
        party: PartyId,
        pos_m: [i64; 3],
        vel_mmps: [i64; 3],
    ) -> Self {
        let mut ship = Self::new_dray(id, callsign, party, pos_m, vel_mmps);
        ship.hull_class = "tacsim.unit.corvette".to_owned();
        if let Some(spec) = super::custody::unit_spec_for_id("tacsim.unit.corvette") {
            super::custody::apply_unit_spec_to_space_ship(&mut ship, spec);
        } else {
            ship.base_rated_accel_mg = 500;
            ship.rated_accel_mg = 500;
            ship.exhaust_kms = 12_000;
            ship.recompute_drive_kinematics();
        }
        super::custody::apply_space_unit_overlay(
            &mut ship,
            &super::custody::space_unit_overlay_for_id("tacsim.unit.corvette"),
        );
        ship
    }

    /// Overall hull integrity percentage (`0..=100`).
    #[must_use]
    pub fn hull_integrity_pct(&self) -> u32 {
        let mut cur = 0u64;
        let mut max = 0u64;
        for sec in self.sections.values() {
            cur = cur.saturating_add(u64::from(sec.internals));
            max = max.saturating_add(u64::from(sec.max_internals));
        }
        if max == 0 {
            return 100;
        }
        ((cur * 100) / max).min(100) as u32
    }

    /// Current scalar speed in whole $\text{m/s}$.
    #[must_use]
    pub fn speed_mps(&self) -> i64 {
        let vx = (self.vel_mmps[0] / 1_000) as i128;
        let vy = (self.vel_mmps[1] / 1_000) as i128;
        let vz = (self.vel_mmps[2] / 1_000) as i128;
        isqrt_i128(vx * vx + vy * vy + vz * vz) as i64
    }

    /// Whether the ship is currently firing its main drive.
    #[must_use]
    pub fn is_burning(&self) -> bool {
        self.status == HullStatus::Operational
            && !self.crew_linkdead
            && (self.thrust_mps2 != [0, 0, 0] || self.thrust_mmps2 != [0, 0, 0])
    }

    /// Canonical `mush-pixel` 3D sprite archetype (`"freighter"`, `"corvette"`, `"raider"`,
    /// `"halcyon"`, `"drone"`, or `"derelict"`, `SF.R13`, `SF.R20`, `D-35-15`).
    #[must_use]
    pub fn pixel_model(&self) -> &str {
        if matches!(self.status, HullStatus::DisabledDerelict { .. }) {
            "derelict"
        } else {
            self.pixel_model.as_str()
        }
    }

    /// Select which [`FacetBand`] faces an incoming attack arriving along vector
    /// `incoming_from_m = attacker_pos_m - self.pos_m`, accounting for the hull's
    /// attitude `(yaw_deg, pitch_deg, roll_deg)` (`D-35-1`, §6.3).
    #[must_use]
    pub fn select_facet_for_attack(&self, attacker_pos_m: [i64; 3]) -> FacetBand {
        let dx = (attacker_pos_m[0] - self.pos_m[0]) as f64;
        let dy = (attacker_pos_m[1] - self.pos_m[1]) as f64;
        let dz = (attacker_pos_m[2] - self.pos_m[2]) as f64;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt().max(1.0);

        let (yaw_d, pitch_d, roll_d) = self.attitude_deg;
        let yaw = (yaw_d as f64).to_radians();
        let pitch = (pitch_d as f64).to_radians();
        let roll = (roll_d as f64).to_radians();

        // Ship body axes in world space before roll:
        let fwd_x = yaw.cos() * pitch.cos();
        let fwd_y = yaw.sin() * pitch.cos();
        let fwd_z = pitch.sin();

        let right0_x = -yaw.sin();
        let right0_y = yaw.cos();
        let right0_z = 0.0;

        let up0_x = -yaw.cos() * pitch.sin();
        let up0_y = -yaw.sin() * pitch.sin();
        let up0_z = pitch.cos();

        let local_fwd = (dx * fwd_x + dy * fwd_y + dz * fwd_z) / dist;
        if local_fwd >= 0.65 {
            return FacetBand::Front;
        }
        if local_fwd <= -0.65 {
            return FacetBand::Rear;
        }

        let r0 = (dx * right0_x + dy * right0_y + dz * right0_z) / dist;
        let u0 = (dx * up0_x + dy * up0_y + dz * up0_z) / dist;

        // Rotate transverse vector by hull roll:
        let local_right = r0 * roll.cos() + u0 * roll.sin();
        let local_up = -r0 * roll.sin() + u0 * roll.cos();

        if local_up > local_right.abs() && local_up > 0.25 {
            FacetBand::Top
        } else if local_right >= 0.0 {
            FacetBand::Right
        } else {
            FacetBand::Left
        }
    }

    /// Apply raw weapon `damage` arriving on `facet` (`D-35-1`, `D-35-12`, `SF.R15`).
    ///
    /// Routes incoming damage through the hull's authored [`Self::facet_weights`],
    /// [`ShipSectionState::struck_through`] armour facets, and [`ShipSectionState::consequences`]:
    /// - `Front` $\to$ `Hull` (`100%`)
    /// - `Left` / `Right` $\to$ `Hull` (`50%`) and `Hold` (`50%`)
    /// - `Top` $\to$ `Hull` (`40%`) and `Hold` (`60%`)
    /// - `Rear` $\to$ `Hull` (`50%`) and `Drive` (`50%`, struck through `Hull.Rear`)
    pub fn apply_damage(&mut self, facet: FacetBand, damage: u32) -> (ShipSectionId, u32, u32) {
        let weights: Vec<(ShipSectionId, u32)> = self
            .facet_weights
            .get(&facet)
            .cloned()
            .filter(|w| !w.is_empty())
            .unwrap_or_else(|| vec![(ShipSectionId::Hull, 100)]);
        let total_weight: u32 = weights.iter().map(|(_, w)| *w).sum::<u32>().max(1);

        let mut armor_groups: Vec<(_, Vec<_>)> = Vec::new();
        for (sec_id, weight) in &weights {
            if !self.sections.contains_key(sec_id) || *weight == 0 {
                continue;
            }
            let armor_key = self
                .sections
                .get(sec_id)
                .and_then(|s| s.struck_through)
                .unwrap_or((*sec_id, facet));
            if let Some((_, members)) = armor_groups.iter_mut().find(|(k, _)| *k == armor_key) {
                members.push((*sec_id, *weight));
            } else {
                armor_groups.push((armor_key, vec![(*sec_id, *weight)]));
            }
        }
        if armor_groups.is_empty() {
            armor_groups.push((
                (ShipSectionId::Hull, facet),
                vec![(ShipSectionId::Hull, 100)],
            ));
        }

        let mut armor_absorbed = 0u32;
        let mut internal_dmg = 0u32;
        let mut target_section = weights
            .first()
            .map_or(ShipSectionId::Hull, |(sec_id, _)| *sec_id);
        let mut remaining_damage = damage;
        let group_count = armor_groups.len();

        for (group_idx, ((armor_sec, armor_band), members)) in armor_groups.into_iter().enumerate()
        {
            let group_weight: u32 = members.iter().map(|(_, w)| *w).sum::<u32>().max(1);
            let group_damage = if group_idx + 1 == group_count {
                remaining_damage
            } else {
                let share = ((u64::from(damage) * u64::from(group_weight))
                    / u64::from(total_weight)) as u32;
                share.min(remaining_damage)
            };
            remaining_damage = remaining_damage.saturating_sub(group_damage);

            let mut group_overflow = group_damage;
            if let Some(sec) = self.sections.get_mut(&armor_sec)
                && let Some(armor) = sec.facets.get_mut(&armor_band)
            {
                let absorbed = (*armor).min(group_overflow);
                *armor -= absorbed;
                armor_absorbed = armor_absorbed.saturating_add(absorbed);
                group_overflow -= absorbed;
            }

            if group_overflow == 0 {
                continue;
            }

            let mut rem_overflow = group_overflow;
            let member_count = members.len();
            let mut spillover = 0u32;
            for (mem_idx, (sec_id, weight)) in members.iter().copied().enumerate() {
                let sec_share = if mem_idx + 1 == member_count {
                    rem_overflow
                } else {
                    let share = ((u64::from(group_overflow) * u64::from(weight))
                        / u64::from(group_weight)) as u32;
                    share.min(rem_overflow)
                };
                rem_overflow = rem_overflow.saturating_sub(sec_share);
                if let Some(sec) = self.sections.get_mut(&sec_id) {
                    let dealt = sec.internals.min(sec_share);
                    sec.internals -= dealt;
                    internal_dmg = internal_dmg.saturating_add(dealt);
                    if dealt > 0 {
                        target_section = sec_id;
                    }
                    spillover = spillover.saturating_add(sec_share.saturating_sub(dealt));
                }
            }
            if spillover > 0 {
                for (sec_id, _) in &members {
                    if spillover == 0 {
                        break;
                    }
                    if let Some(sec) = self.sections.get_mut(sec_id)
                        && sec.internals > 0
                    {
                        let dealt = sec.internals.min(spillover);
                        sec.internals -= dealt;
                        internal_dmg = internal_dmg.saturating_add(dealt);
                        spillover -= dealt;
                        target_section = *sec_id;
                    }
                }
                if spillover > 0
                    && let Some(hull_sec) = self.sections.get_mut(&ShipSectionId::Hull)
                    && hull_sec.internals > 0
                {
                    let dealt = hull_sec.internals.min(spillover);
                    hull_sec.internals -= dealt;
                    internal_dmg = internal_dmg.saturating_add(dealt);
                }
            }
        }

        // Subsystem crit check when internals take penetrating damage (`D-35-1`, §6.3, `SF.R15`)
        let hull_lost = self
            .sections
            .get(&ShipSectionId::Hull)
            .map_or(0, |s| s.max_internals.saturating_sub(s.internals));
        let drive_lost = self
            .sections
            .get(&ShipSectionId::Drive)
            .map_or(0, |s| s.max_internals.saturating_sub(s.internals));
        if (internal_dmg >= 40 || hull_lost >= 30)
            && !self.crits.contains(&SubsystemCrit::RadiatorDamaged)
        {
            self.crits.push(SubsystemCrit::RadiatorDamaged);
        }
        if (internal_dmg >= 30 || drive_lost >= 25)
            && !self.crits.contains(&SubsystemCrit::DriveDegraded)
        {
            self.crits.push(SubsystemCrit::DriveDegraded);
        }
        if internal_dmg >= 25
            && matches!(
                facet,
                FacetBand::Top | FacetBand::Left | FacetBand::Right | FacetBand::Front
            )
            && let Some(pdc) = self.pdcs.iter_mut().find(|p| p.operational)
        {
            pdc.operational = false;
            if !self.crits.contains(&SubsystemCrit::PdcOffline) {
                self.crits.push(SubsystemCrit::PdcOffline);
            }
        }
        if internal_dmg >= 35
            && matches!(facet, FacetBand::Front | FacetBand::Top)
            && !self.crits.contains(&SubsystemCrit::SensorDegraded)
        {
            self.crits.push(SubsystemCrit::SensorDegraded);
        }

        // Apply authored section consequences (`slow_75`, `disable_motion`, `destroy_unit`)
        // and open-epoch derelict disablement via the shared `carrier_damage` table (`D-35-1`,
        // `D-35-12`, `FO-12`, `Q-36-47`, `SF.R20`).
        let disable_ship = self
            .carrier_damage()
            .has(crate::carrier_damage::CarrierLoss::RefusesLift);
        let mut next_slow_pct = self.slow_pct;
        let mut any_slow = false;
        for sec in self.sections.values() {
            if sec.internals == 0 {
                for cons in &sec.consequences {
                    if let SpaceSectionConsequence::Slow { percent } = *cons {
                        let clamped = u8::try_from(percent.clamp(1, 100)).unwrap_or(100);
                        next_slow_pct =
                            crate::arena::damage::apply_slow_consequence(next_slow_pct, clamped);
                        any_slow = true;
                    }
                }
            }
        }
        if any_slow {
            self.slow_pct = next_slow_pct;
            self.recompute_drive_accel();
            if !self.crits.contains(&SubsystemCrit::DriveDegraded) {
                self.crits.push(SubsystemCrit::DriveDegraded);
            }
        }
        if disable_ship && self.status == HullStatus::Operational {
            self.status = HullStatus::DisabledDerelict {
                epoch_open: true,
                life_support_remaining_s: DERELICT_WINDOW_S,
                salvage_contest_window_s: DERELICT_WINDOW_S,
            };
            self.thrust_mps2 = [0, 0, 0];
            self.thrust_mmps2 = [0, 0, 0];
        }

        (target_section, armor_absorbed, internal_dmg)
    }
}

/// Discrete combat & telemetry events emitted during a [`LocalSpaceBubble::step_wall_ms_at`] call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpaceCombatEvent {
    TorpedoLaunched {
        torpedo_id: u64,
        shooter_id: String,
        target_id: String,
        inherited_vel_mmps: [i64; 3],
    },
    TorpedoInterceptedByPdc {
        torpedo_id: u64,
        defender_id: String,
        pdc_id: String,
    },
    TorpedoInterceptedByLaser {
        torpedo_id: u64,
        defender_id: String,
        laser_id: String,
    },
    TorpedoExhaustedDeltaV {
        torpedo_id: u64,
    },
    TorpedoHit {
        torpedo_id: u64,
        target_id: String,
        facet: FacetBand,
        section: ShipSectionId,
        armor_damage: u32,
        internal_damage: u32,
    },
    LaserHit {
        shooter_id: String,
        target_id: String,
        distance_m: i64,
        facet: FacetBand,
        section: ShipSectionId,
        armor_damage: u32,
        internal_damage: u32,
        shooter_heat_permille: u16,
    },
    RailgunFired {
        slug_id: u64,
        shooter_id: String,
        target_id: String,
        aimed_lead_pos_m: [i64; 3],
        flight_ms: u32,
    },
    RailgunDodgedByJink {
        slug_id: u64,
        shooter_id: String,
        target_id: String,
        displacement_from_lead_m: i64,
    },
    RailgunHit {
        slug_id: u64,
        shooter_id: String,
        target_id: String,
        facet: FacetBand,
        section: ShipSectionId,
        armor_damage: u32,
        internal_damage: u32,
    },
    ShipDisabledDerelict {
        ship_id: String,
        epoch_open: bool,
        life_support_remaining_s: u32,
    },
    TankReserveWarning {
        ship_id: String,
        remaining_dv_mps: i64,
        return_reserve_dv_mps: i64,
    },
    TorpedoSpoofedByDecoy {
        torpedo_id: u64,
        defender_id: String,
        decoys_remaining: u8,
    },
    CoolantBlowdown {
        ship_id: String,
        propellant_spent_kg: u32,
        new_heat_permille: u16,
    },
    SubsystemRepaired {
        ship_id: String,
        crit: SubsystemCrit,
    },
}

/// Summary returned by [`LocalSpaceBubble::step_wall_ms_at`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BubbleStepReport {
    pub wall_advanced_ms: u32,
    pub substeps_executed: u32,
    pub events: Vec<SpaceCombatEvent>,
    pub state_hash: u64,
}

/// Serde helper that encodes a [`BTreeMap<K, V>`] as a JSON array of `[K, V]` pairs so
/// non-string map keys (tuples, `ObjId`, structs, enums) round-trip through `serde_json`.
pub mod vec_tuple_map {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Serialize `map` as a `Vec<(&K, &V)>`.
    ///
    /// # Errors
    ///
    /// Propagates any serializer error.
    pub fn serialize<K, V, S>(map: &BTreeMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize + Ord,
        V: Serialize,
        S: Serializer,
    {
        let entries: Vec<(&K, &V)> = map.iter().collect();
        entries.serialize(serializer)
    }

    /// Deserialize a `BTreeMap<K, V>` from a `Vec<(K, V)>`.
    ///
    /// # Errors
    ///
    /// Propagates any deserializer error.
    pub fn deserialize<'de, K, V, D>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        let entries: Vec<(K, V)> = Vec::deserialize(deserializer)?;
        Ok(entries.into_iter().collect())
    }
}

/// Projected celestial body for the `space.cockpit` bridge canopy feed (`SF.R13`, `SF.R16`, `D-35-15`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanopyBodyProjection {
    pub id: String,
    pub label: String,
    pub bearing_deg: i32,
    pub pitch_deg: i32,
    pub radius_px: i32,
    pub radius_km: i64,
    pub distance_km: i64,
}

impl CanopyBodyProjection {
    /// Convert this projection into its `space.cockpit` JSON entry.
    #[must_use]
    pub fn to_feed_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.label,
            "label": self.label,
            "kind": self.id,
            "bearing_deg": self.bearing_deg,
            "pitch_deg": self.pitch_deg,
            "radius_px": self.radius_px,
            "radius_km": self.radius_km,
            "distance_km": self.distance_km,
        })
    }
}

/// Default celestial bodies visible in the bridge canopy for an encounter site (`SF.R13`, `D-35-15`).
#[must_use]
pub fn default_canopy_bodies_for_site(site_id: &str) -> Vec<CanopyBodyProjection> {
    let lower = site_id.to_ascii_lowercase();
    if lower.contains("neptune") || lower.contains("triton") {
        vec![
            CanopyBodyProjection {
                id: "neptune".to_owned(),
                label: "NEPTUNE".to_owned(),
                bearing_deg: 0,
                pitch_deg: -4,
                radius_px: 44,
                radius_km: 24_622,
                distance_km: 85_000,
            },
            CanopyBodyProjection {
                id: "triton".to_owned(),
                label: "TRITON".to_owned(),
                bearing_deg: 26,
                pitch_deg: 5,
                radius_px: 20,
                radius_km: 1_353,
                distance_km: 42_000,
            },
            CanopyBodyProjection {
                id: "sun".to_owned(),
                label: "SOL".to_owned(),
                bearing_deg: 180,
                pitch_deg: 2,
                radius_px: 6,
                radius_km: 696_340,
                distance_km: 4_495_000_000,
            },
        ]
    } else if lower.contains("saturn") || lower.contains("enceladus") {
        vec![
            CanopyBodyProjection {
                id: "saturn".to_owned(),
                label: "SATURN".to_owned(),
                bearing_deg: 0,
                pitch_deg: -3,
                radius_px: 46,
                radius_km: 58_232,
                distance_km: 140_000,
            },
            CanopyBodyProjection {
                id: "enceladus".to_owned(),
                label: "ENCELADUS".to_owned(),
                bearing_deg: 24,
                pitch_deg: 4,
                radius_px: 16,
                radius_km: 252,
                distance_km: 35_000,
            },
            CanopyBodyProjection {
                id: "sun".to_owned(),
                label: "SOL".to_owned(),
                bearing_deg: 180,
                pitch_deg: 2,
                radius_px: 8,
                radius_km: 696_340,
                distance_km: 1_430_000_000,
            },
        ]
    } else if lower.contains("jupiter")
        || lower.contains("europa")
        || lower.contains("callisto")
        || lower.contains("ganymede")
    {
        vec![
            CanopyBodyProjection {
                id: "jupiter".to_owned(),
                label: "JUPITER".to_owned(),
                bearing_deg: 0,
                pitch_deg: -4,
                radius_px: 48,
                radius_km: 69_911,
                distance_km: 180_000,
            },
            CanopyBodyProjection {
                id: "europa".to_owned(),
                label: "EUROPA".to_owned(),
                bearing_deg: 22,
                pitch_deg: 4,
                radius_px: 18,
                radius_km: 1_560,
                distance_km: 45_000,
            },
            CanopyBodyProjection {
                id: "sun".to_owned(),
                label: "SOL".to_owned(),
                bearing_deg: 180,
                pitch_deg: 2,
                radius_px: 10,
                radius_km: 696_340,
                distance_km: 778_500_000,
            },
        ]
    } else if lower.contains("phobos") || lower.contains("mars") {
        vec![
            CanopyBodyProjection {
                id: "mars".to_owned(),
                label: "MARS".to_owned(),
                bearing_deg: 10,
                pitch_deg: -5,
                radius_px: 38,
                radius_km: 3_390,
                distance_km: 9_376,
            },
            CanopyBodyProjection {
                id: "phobos".to_owned(),
                label: "PHOBOS".to_owned(),
                bearing_deg: 342,
                pitch_deg: 4,
                radius_px: 18,
                radius_km: 11,
                distance_km: 120,
            },
            CanopyBodyProjection {
                id: "sun".to_owned(),
                label: "SOL".to_owned(),
                bearing_deg: 180,
                pitch_deg: 2,
                radius_px: 14,
                radius_km: 696_340,
                distance_km: 227_900_000,
            },
        ]
    } else {
        vec![
            CanopyBodyProjection {
                id: "ceres".to_owned(),
                label: "CERES".to_owned(),
                bearing_deg: 12,
                pitch_deg: -4,
                radius_px: 28,
                radius_km: 473,
                distance_km: 4_500,
            },
            CanopyBodyProjection {
                id: "jupiter".to_owned(),
                label: "JUPITER".to_owned(),
                bearing_deg: 72,
                pitch_deg: 3,
                radius_px: 14,
                radius_km: 69_911,
                distance_km: 360_000_000,
            },
            CanopyBodyProjection {
                id: "sun".to_owned(),
                label: "SOL".to_owned(),
                bearing_deg: 180,
                pitch_deg: 2,
                radius_px: 12,
                radius_km: 696_340,
                distance_km: 414_000_000,
            },
        ]
    }
}

/// Compute the bridge canopy's celestial `bodies` from [`crate::sky::Sky`] (`sky.position_km(&body, now_ic)`)
/// at the hull's live 3D heliocentric state vector `observer_pos_km` (`SF.R24`, `D-35-15`).
#[must_use]
pub fn compute_ephemeris_canopy_bodies(
    w: &mush_core::World,
    observer_pos_km: [i64; 3],
    site_id: &str,
    now_ic: i64,
) -> Vec<CanopyBodyProjection> {
    let mut bodies = default_canopy_bodies_for_site(site_id);
    let Some(store) = w.module_state::<crate::store::TacsimStore>() else {
        return bodies;
    };
    let Ok(lane) = mush_core::CanonicalId::new(crate::definitions::content::LANE_PACK) else {
        return bodies;
    };
    let Some(version) = store.applied_in(w, &lane) else {
        return bodies;
    };
    let Ok((sky, _pack)) = crate::port::voyage::lane_sky_of(w, version) else {
        return bodies;
    };

    let obs_pos = crate::sky::PosKm {
        x: observer_pos_km[0],
        y: observer_pos_km[1],
        z: observer_pos_km[2],
    };

    let primary_pos = bodies
        .first()
        .and_then(|b| mush_core::CanonicalId::new(&b.id).ok())
        .and_then(|cid| sky.position_km(&cid, now_ic));

    if let (Some(primary), Some(p_pos)) = (bodies.first_mut(), primary_pos) {
        let rel = [
            p_pos.x.saturating_sub(obs_pos.x),
            p_pos.y.saturating_sub(obs_pos.y),
            p_pos.z.saturating_sub(obs_pos.z),
        ];
        let raw_dist_km = i64::try_from(obs_pos.distance_km(p_pos)).unwrap_or(i64::MAX);
        let dir = if rel == [0, 0, 0] {
            [p_pos.x, p_pos.y, p_pos.z]
        } else {
            rel
        };
        let (b_f, p_f) = vector_to_bearing_pitch(dir);
        primary.bearing_deg = (b_f.round() as i32).rem_euclid(360);
        primary.pitch_deg = (p_f.round() as i32 - 4).clamp(-45, 45);
        if raw_dist_km > 0 {
            primary.distance_km = raw_dist_km;
        }
    }

    let primary_bearing = bodies.first().map_or(0, |b| b.bearing_deg);
    let primary_pitch = bodies.first().map_or(-4, |b| b.pitch_deg);

    if let Some(sec_id) = bodies.get(1).map(|b| b.id.clone())
        && let Ok(sec_cid) = mush_core::CanonicalId::new(&sec_id)
        && let Some(s_pos) = sky.position_km(&sec_cid, now_ic)
        && let Some(sec) = bodies.get_mut(1)
    {
        let rel_s = [
            s_pos.x.saturating_sub(obs_pos.x),
            s_pos.y.saturating_sub(obs_pos.y),
            s_pos.z.saturating_sub(obs_pos.z),
        ];
        let raw_s_dist_km = i64::try_from(obs_pos.distance_km(s_pos)).unwrap_or(i64::MAX);
        let is_moon_of_primary =
            primary_pos.is_some_and(|p_pos| p_pos.distance_km(s_pos) < 5_000_000);
        if is_moon_of_primary {
            let p_pos = primary_pos.unwrap_or(s_pos);
            sec.bearing_deg = (primary_bearing + 26).rem_euclid(360);
            sec.pitch_deg = (primary_pitch + 9).clamp(-45, 45);
            if p_pos == obs_pos {
                let orb_dist_km =
                    i64::try_from(p_pos.distance_km(s_pos)).unwrap_or(sec.distance_km);
                sec.distance_km = orb_dist_km.max(1);
            } else if raw_s_dist_km > 0 {
                sec.distance_km = raw_s_dist_km;
            }
        } else {
            let dir_s = if rel_s == [0, 0, 0] {
                [s_pos.x, s_pos.y, s_pos.z]
            } else {
                rel_s
            };
            let (s_b_f, s_p_f) = vector_to_bearing_pitch(dir_s);
            sec.bearing_deg = (s_b_f.round() as i32).rem_euclid(360);
            sec.pitch_deg = s_p_f.round() as i32;
            if raw_s_dist_km > 0 {
                sec.distance_km = raw_s_dist_km;
            }
        }
    }

    if let Some(sun) = bodies.iter_mut().find(|b| b.id == "sun") {
        let ref_pos = if obs_pos.x == 0 && obs_pos.y == 0 && obs_pos.z == 0 {
            primary_pos.unwrap_or(obs_pos)
        } else {
            obs_pos
        };
        if ref_pos.x != 0 || ref_pos.y != 0 || ref_pos.z != 0 {
            let (sun_b_f, sun_p_f) = vector_to_bearing_pitch([-ref_pos.x, -ref_pos.y, -ref_pos.z]);
            sun.bearing_deg = (sun_b_f.round() as i32).rem_euclid(360);
            sun.pitch_deg = (sun_p_f.round() as i32 + 2).clamp(-45, 45);
            let sun_dist_km =
                i64::try_from(ref_pos.distance_km(crate::sky::PosKm { x: 0, y: 0, z: 0 }))
                    .unwrap_or(sun.distance_km);
            if sun_dist_km > 0 {
                sun.distance_km = sun_dist_km;
            }
        }
    }

    bodies
}

/// A Tier-2 **Local Space Bubble** anchored at an orbital site or encounter point (`D-35-3`, `D-35-5`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalSpaceBubble {
    pub id: BubbleId,
    pub site_id: String,
    pub title: String,
    /// Total 1:1 wall milliseconds elapsed since bubble creation.
    pub wall_elapsed_ms: u64,
    /// Stamped IC epoch seconds (`Q-CF-32`).
    pub ic_timestamp_s: i64,
    /// Whether the bubble is frozen due to a W-clock outage (`Q-CF-33`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outage_frozen: bool,
    pub ships: BTreeMap<String, SpaceShipState>,
    /// Canonical kernel party relation books (`D-35-1`, `D-35-11`, `Q-CF-5`–`Q-CF-8`).
    #[serde(default)]
    pub party_relations: crate::arena::party::Relations,
    /// **The parties screening as a customs picket** (doc 40 Q-40-22): a picket's
    /// inspection, which the arena has no relation for, is a flag on the picket's
    /// party; it guards the neutrals it screens as it would a friend. Absent from
    /// the document while empty, so only a screening picket's bubble carries it.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub inspecting: BTreeSet<PartyId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub torpedoes: Vec<InFlightTorpedo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub railgun_slugs: Vec<InFlightSlug>,
    /// Celestial bodies visible in the bridge canopy (`SF.R13`, `SF.R16`, `D-35-15`).
    #[serde(default)]
    pub bodies: Vec<CanopyBodyProjection>,
    next_ordnance_id: u64,
}

impl LocalSpaceBubble {
    /// Create a new site-anchored [`LocalSpaceBubble`].
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        site_id: impl Into<String>,
        title: impl Into<String>,
        ic_timestamp_s: i64,
    ) -> Self {
        let site_id = site_id.into();
        let bodies = default_canopy_bodies_for_site(&site_id);
        Self {
            id: BubbleId(id.into()),
            site_id,
            title: title.into(),
            wall_elapsed_ms: 0,
            ic_timestamp_s,
            outage_frozen: false,
            ships: BTreeMap::new(),
            party_relations: crate::arena::party::Relations::new(),
            inspecting: BTreeSet::new(),
            torpedoes: Vec::new(),
            railgun_slugs: Vec::new(),
            bodies,
            next_ordnance_id: 1,
        }
    }

    /// Insert or update a ship in the bubble and admit its [`PartyId`].
    pub fn insert_ship(&mut self, ship: SpaceShipState) {
        self.party_relations.admit(ship.party);
        self.ships.insert(ship.id.clone(), ship);
    }

    /// **Mark `party_a` and `party_b` each other's `mark`** (`D-35-1`, `D-35-11`,
    /// Q-40-22): both books, so the standing is symmetric; `None` clears both
    /// marks back to neutral.
    pub fn set_party_relation(&mut self, party_a: PartyId, party_b: PartyId, mark: Option<Mark>) {
        self.party_relations.admit(party_a);
        self.party_relations.admit(party_b);
        self.party_relations.mark(party_a, party_b, mark);
        self.party_relations.mark(party_b, party_a, mark);
    }

    /// **What `viewer` believes `other` is** (`D-35-1`, Q-CF-7): the arena's
    /// precedence over this bubble's books.
    #[must_use]
    pub fn party_relation(&self, viewer: PartyId, other: PartyId) -> Relation {
        self.party_relations.relation(viewer, other)
    }

    /// Flag `party` as screening (a customs picket's inspection, Q-40-22), or
    /// clear the flag.
    pub fn set_inspecting(&mut self, party: PartyId, screening: bool) {
        if screening {
            self.inspecting.insert(party);
        } else {
            self.inspecting.remove(&party);
        }
    }

    /// Whether `party` is screening as a customs picket (Q-40-22).
    #[must_use]
    pub fn is_inspecting(&self, party: PartyId) -> bool {
        self.inspecting.contains(&party)
    }

    /// Toggle automatic point-defense (`+tac/pdc auto`) for `ship_id` (`Q-35-4`, `D-35-8`).
    pub fn set_pdc_auto(&mut self, ship_id: &str, enabled: bool) -> bool {
        if let Some(ship) = self.ships.get_mut(ship_id) {
            ship.pdc_auto = enabled;
            true
        } else {
            false
        }
    }

    /// Toggle a laser mount on `ship_id` between [`LaserMode::Offense`] and [`LaserMode::Defense`] (`D-35-8`).
    pub fn set_laser_mode(&mut self, ship_id: &str, laser_id: &str, mode: LaserMode) -> bool {
        if let Some(ship) = self.ships.get_mut(ship_id)
            && let Some(mount) = ship.lasers.iter_mut().find(|l| l.id == laser_id)
        {
            mount.mode = mode;
            return true;
        }
        false
    }

    /// Set the [`EmconPosture`] (`Ghost`, `Passive`, `Standard`, `ActiveCombat`) on `ship_id` (`SF.15`, `D-35-9`, `D-35-10`).
    pub fn set_emcon(&mut self, ship_id: &str, posture: EmconPosture) -> bool {
        if let Some(ship) = self.ships.get_mut(ship_id) {
            ship.emcon = posture;
            true
        } else {
            false
        }
    }

    /// Set or clear the fire-control target lock (`locked_target_id`) on `ship_id` (`SF.R3`, `D-35-9`).
    pub fn set_locked_target(&mut self, ship_id: &str, target_id: Option<String>) -> bool {
        if let Some(ship) = self.ships.get_mut(ship_id) {
            ship.locked_target_id = target_id;
            true
        } else {
            false
        }
    }

    /// Deploy an active thermal/radar ECM decoy canister (`+tac/decoy`, `SF.15`, `D-35-8`).
    ///
    /// Returns `Ok(decoys_remaining)` when a canister is launched (active for 15 wall seconds).
    pub fn launch_decoy(&mut self, ship_id: &str) -> Result<u8, &'static str> {
        let Some(ship) = self.ships.get_mut(ship_id) else {
            return Err("ship not found in bubble");
        };
        if ship.status != HullStatus::Operational {
            return Err("hull disabled");
        }
        if ship.decoys_remaining == 0 {
            return Err("no decoy canisters remaining");
        }
        ship.decoys_remaining -= 1;
        ship.active_decoy_ms = 15_000;
        Ok(ship.decoys_remaining)
    }

    /// Emergency open-cycle coolant blowdown (`+helm/coolant`, `SF.15`, `D-35-1`):
    /// expends `250 kg` of reaction mass/coolant to immediately dump `450 permille` of hull heat.
    pub fn blowdown_coolant(&mut self, ship_id: &str) -> Result<SpaceCombatEvent, &'static str> {
        let Some(ship) = self.ships.get_mut(ship_id) else {
            return Err("ship not found in bubble");
        };
        if ship.propellant_kg < 250 {
            return Err("insufficient coolant/propellant reserve (requires 250 kg)");
        }
        ship.propellant_kg -= 250;
        ship.heat_permille = ship.heat_permille.saturating_sub(450);
        Ok(SpaceCombatEvent::CoolantBlowdown {
            ship_id: ship.id.clone(),
            propellant_spent_kg: 250,
            new_heat_permille: ship.heat_permille,
        })
    }

    /// Damage-control repair of a subsystem critical hit (`+helm/repair`, `SF.15`, `D-35-1`).
    pub fn repair_subsystem_crit(
        &mut self,
        ship_id: &str,
        crit: SubsystemCrit,
    ) -> Result<SpaceCombatEvent, &'static str> {
        let Some(ship) = self.ships.get_mut(ship_id) else {
            return Err("ship not found in bubble");
        };
        let Some(pos) = ship.crits.iter().position(|c| *c == crit) else {
            return Err("subsystem is not damaged");
        };
        ship.crits.remove(pos);
        if crit == SubsystemCrit::PdcOffline {
            for pdc in &mut ship.pdcs {
                pdc.operational = true;
            }
        }
        Ok(SpaceCombatEvent::SubsystemRepaired {
            ship_id: ship.id.clone(),
            crit,
        })
    }

    /// Command a 3D burn vector `[ax, ay, az]` in $\text{m/s}^2$ on `ship_id` (`D-35-6`).
    ///
    /// If `force_into_reserve` is `false` and the ship is at or below `return_reserve_dv_mps`,
    /// refuses the burn and returns `Err(SpaceCombatEvent::TankReserveWarning)`.
    pub fn command_burn(
        &mut self,
        ship_id: &str,
        thrust_mps2: [i64; 3],
        force_into_reserve: bool,
    ) -> Result<(), SpaceCombatEvent> {
        let Some(ship) = self.ships.get_mut(ship_id) else {
            return Ok(());
        };
        if ship.status != HullStatus::Operational || ship.crew_linkdead {
            ship.thrust_mps2 = [0, 0, 0];
            ship.thrust_mmps2 = [0, 0, 0];
            return Ok(());
        }
        let clamped = [
            thrust_mps2[0].clamp(-1_000_000, 1_000_000),
            thrust_mps2[1].clamp(-1_000_000, 1_000_000),
            thrust_mps2[2].clamp(-1_000_000, 1_000_000),
        ];
        let requested_mag = isqrt_i128(
            (clamped[0] as i128).pow(2) + (clamped[1] as i128).pow(2) + (clamped[2] as i128).pow(2),
        ) as i64;
        if requested_mag > 0
            && !force_into_reserve
            && ship.delta_v_mps <= ship.return_reserve_dv_mps
        {
            return Err(SpaceCombatEvent::TankReserveWarning {
                ship_id: ship.id.clone(),
                remaining_dv_mps: ship.delta_v_mps,
                return_reserve_dv_mps: ship.return_reserve_dv_mps,
            });
        }
        let max_a = if ship.crits.contains(&SubsystemCrit::DriveDegraded) {
            (ship.max_accel_mps2 / 2).max(1)
        } else {
            ship.max_accel_mps2
        };
        let next_thrust = if requested_mag > max_a && requested_mag > 0 {
            let max_i128 = i128::from(max_a);
            let mag_i128 = i128::from(requested_mag);
            [
                ((i128::from(clamped[0]) * max_i128) / mag_i128) as i64,
                ((i128::from(clamped[1]) * max_i128) / mag_i128) as i64,
                ((i128::from(clamped[2]) * max_i128) / mag_i128) as i64,
            ]
        } else {
            clamped
        };
        let actual_mag = isqrt_i128(
            (next_thrust[0] as i128).pow(2)
                + (next_thrust[1] as i128).pow(2)
                + (next_thrust[2] as i128).pow(2),
        ) as i64;
        if actual_mag > 0
            && !force_into_reserve
            && ship.delta_v_mps.saturating_sub(actual_mag) <= ship.return_reserve_dv_mps
        {
            return Err(SpaceCombatEvent::TankReserveWarning {
                ship_id: ship.id.clone(),
                remaining_dv_mps: ship.delta_v_mps,
                return_reserve_dv_mps: ship.return_reserve_dv_mps,
            });
        }
        if actual_mag > 0 && ship.thrust_mps2 != next_thrust {
            let law = crate::definitions::logistics::DriveLaw::Flip {
                accel_mg: ship.rated_accel_mg,
                exhaust_kms: ship.exhaust_kms.max(1),
            };
            let dry_kg = u64::from(ship.dry_mass_t).saturating_mul(1_000);
            let payload_kg = ship
                .non_prop_payload_kg
                .saturating_add(u64::from(ship.propellant_kg));
            let dv_u32 = u32::try_from(actual_mag).unwrap_or(1).max(1);
            let burn_kg =
                crate::port::passage::burn_kg_for_law_dv(&law, dry_kg, payload_kg, dv_u32)
                    .unwrap_or(1)
                    .max(1);
            ship.propellant_kg = ship
                .propellant_kg
                .saturating_sub(u32::try_from(burn_kg).unwrap_or(u32::MAX));
            ship.delta_v_mps = (ship.delta_v_mps - actual_mag).max(0);
            ship.vel_mmps[0] = ship.vel_mmps[0].saturating_add(next_thrust[0] * 1_000);
            ship.vel_mmps[1] = ship.vel_mmps[1].saturating_add(next_thrust[1] * 1_000);
            ship.vel_mmps[2] = ship.vel_mmps[2].saturating_add(next_thrust[2] * 1_000);
        }
        ship.thrust_mps2 = next_thrust;
        ship.thrust_mmps2 = [
            next_thrust[0].saturating_mul(1_000),
            next_thrust[1].saturating_mul(1_000),
            next_thrust[2].saturating_mul(1_000),
        ];
        Ok(())
    }

    /// Command a 3D burn vector `[ax, ay, az]` in $\text{mm/s}^2$ on `ship_id` (`D-35-6`, `SF.R20`).
    ///
    /// Caps acceleration magnitude by `ship.max_accel_mmps2` (halved when `DriveDegraded`)
    /// so sub-$1\text{ m/s}^2$ haulers accelerate at their true mass-adjusted flip-drive rate.
    pub fn command_burn_mmps2(
        &mut self,
        ship_id: &str,
        thrust_mmps2: [i64; 3],
        force_into_reserve: bool,
    ) -> Result<[i64; 3], SpaceCombatEvent> {
        let Some(ship) = self.ships.get_mut(ship_id) else {
            return Ok([0, 0, 0]);
        };
        if ship.status != HullStatus::Operational || ship.crew_linkdead {
            ship.thrust_mps2 = [0, 0, 0];
            ship.thrust_mmps2 = [0, 0, 0];
            return Ok([0, 0, 0]);
        }
        let clamped = [
            thrust_mmps2[0].clamp(-1_000_000_000, 1_000_000_000),
            thrust_mmps2[1].clamp(-1_000_000_000, 1_000_000_000),
            thrust_mmps2[2].clamp(-1_000_000_000, 1_000_000_000),
        ];
        let requested_mag_mmps2 = isqrt_i128(
            (clamped[0] as i128).pow(2) + (clamped[1] as i128).pow(2) + (clamped[2] as i128).pow(2),
        ) as i64;
        if requested_mag_mmps2 > 0
            && !force_into_reserve
            && ship.delta_v_mps <= ship.return_reserve_dv_mps
        {
            return Err(SpaceCombatEvent::TankReserveWarning {
                ship_id: ship.id.clone(),
                remaining_dv_mps: ship.delta_v_mps,
                return_reserve_dv_mps: ship.return_reserve_dv_mps,
            });
        }
        let base_max_mmps2 = if ship.max_accel_mmps2 > 0 {
            ship.max_accel_mmps2
        } else {
            (ship.max_accel_mps2.saturating_mul(1_000)).max(1)
        };
        let max_a_mmps2 = if ship.crits.contains(&SubsystemCrit::DriveDegraded) {
            (base_max_mmps2 / 2).max(1)
        } else {
            base_max_mmps2
        };
        let next_thrust_mmps2 = if requested_mag_mmps2 > max_a_mmps2 && requested_mag_mmps2 > 0 {
            let max_i128 = i128::from(max_a_mmps2);
            let mag_i128 = i128::from(requested_mag_mmps2);
            [
                ((i128::from(clamped[0]) * max_i128) / mag_i128) as i64,
                ((i128::from(clamped[1]) * max_i128) / mag_i128) as i64,
                ((i128::from(clamped[2]) * max_i128) / mag_i128) as i64,
            ]
        } else {
            clamped
        };
        let actual_mag_mmps2 = isqrt_i128(
            (next_thrust_mmps2[0] as i128).pow(2)
                + (next_thrust_mmps2[1] as i128).pow(2)
                + (next_thrust_mmps2[2] as i128).pow(2),
        ) as i64;
        let actual_dv_mps = ((actual_mag_mmps2 + 999) / 1_000).max(i64::from(actual_mag_mmps2 > 0));
        if actual_mag_mmps2 > 0
            && !force_into_reserve
            && ship.delta_v_mps.saturating_sub(actual_dv_mps) <= ship.return_reserve_dv_mps
        {
            return Err(SpaceCombatEvent::TankReserveWarning {
                ship_id: ship.id.clone(),
                remaining_dv_mps: ship.delta_v_mps,
                return_reserve_dv_mps: ship.return_reserve_dv_mps,
            });
        }
        if actual_mag_mmps2 > 0 && ship.thrust_mmps2 != next_thrust_mmps2 {
            let law = crate::definitions::logistics::DriveLaw::Flip {
                accel_mg: ship.rated_accel_mg,
                exhaust_kms: ship.exhaust_kms.max(1),
            };
            let dry_kg = u64::from(ship.dry_mass_t).saturating_mul(1_000);
            let payload_kg = ship
                .non_prop_payload_kg
                .saturating_add(u64::from(ship.propellant_kg));
            let dv_u32 = u32::try_from(actual_dv_mps).unwrap_or(1).max(1);
            let burn_kg =
                crate::port::passage::burn_kg_for_law_dv(&law, dry_kg, payload_kg, dv_u32)
                    .unwrap_or(1)
                    .max(1);
            ship.propellant_kg = ship
                .propellant_kg
                .saturating_sub(u32::try_from(burn_kg).unwrap_or(u32::MAX));
            ship.delta_v_mps = (ship.delta_v_mps - actual_dv_mps).max(0);
            ship.vel_mmps[0] = ship.vel_mmps[0].saturating_add(next_thrust_mmps2[0]);
            ship.vel_mmps[1] = ship.vel_mmps[1].saturating_add(next_thrust_mmps2[1]);
            ship.vel_mmps[2] = ship.vel_mmps[2].saturating_add(next_thrust_mmps2[2]);
        }
        ship.thrust_mmps2 = next_thrust_mmps2;
        ship.thrust_mps2 = [
            next_thrust_mmps2[0] / 1_000,
            next_thrust_mmps2[1] / 1_000,
            next_thrust_mmps2[2] / 1_000,
        ];
        Ok(next_thrust_mmps2)
    }

    /// Launch a torpedo salvo from `shooter_id` at `target_id` (`D-35-8`).
    ///
    /// Each launched torpedo inherits `shooter`'s exact 3D velocity `vel_mmps` at launch.
    pub fn fire_torpedo_salvo(
        &mut self,
        shooter_id: &str,
        target_id: &str,
        count: u8,
    ) -> Vec<SpaceCombatEvent> {
        let mut events = Vec::new();
        let (Some(shooter), Some(target)) = (self.ships.get(shooter_id), self.ships.get(target_id))
        else {
            return events;
        };
        if shooter.status != HullStatus::Operational
            || shooter_id == target_id
            || self.party_relation(shooter.party, target.party) != Relation::Hostile
        {
            return events;
        }
        let shooter_party = shooter.party;
        let target_party = target.party;
        self.party_relations.note_fire(shooter_party, target_party);
        let (Some(shooter), Some(target)) = (self.ships.get(shooter_id), self.ships.get(target_id))
        else {
            return events;
        };
        let shooter_pos = shooter.pos_m;
        let shooter_vel = shooter.vel_mmps;
        let target_pos = target.pos_m;

        let dx = (target_pos[0] - shooter_pos[0]) as i128;
        let dy = (target_pos[1] - shooter_pos[1]) as i128;
        let dz = (target_pos[2] - shooter_pos[2]) as i128;
        let dist_m = isqrt_i128(dx * dx + dy * dy + dz * dz).max(1) as i64;

        let Some(shooter_mut) = self.ships.get_mut(shooter_id) else {
            return events;
        };
        let Some(tube) = shooter_mut.torpedo_tubes.first_mut() else {
            return events;
        };
        if tube.cooldown_remaining_ms > 0 || tube.magazine_remaining == 0 {
            return events;
        }
        let salvo = count
            .min(tube.tubes)
            .min(tube.magazine_remaining.min(u16::from(u8::MAX)) as u8);
        if salvo == 0 {
            return events;
        }
        tube.magazine_remaining -= u16::from(salvo);
        tube.cooldown_remaining_ms = tube.reload_cooldown_ms;

        let accel = tube.accel_mps2.max(1);
        let max_dv_mmps = tube.max_dv_mps * 1_000;
        let dmg = tube.warhead_damage;
        let est_tti_s = ((2 * dist_m) / accel).isqrt().clamp(1, 120);

        for _ in 0..salvo {
            let tid = self.next_ordnance_id;
            self.next_ordnance_id = self.next_ordnance_id.saturating_add(1);
            self.torpedoes.push(InFlightTorpedo {
                id: tid,
                shooter_id: shooter_id.to_owned(),
                target_id: target_id.to_owned(),
                pos_m: shooter_pos,
                pos_rem_um: [0, 0, 0],
                vel_mmps: shooter_vel,
                accel_mps2: accel,
                remaining_dv_mmps: max_dv_mmps,
                warhead_damage: dmg,
                integrity: 100,
                tti_s: est_tti_s,
                flight_elapsed_ms: 0,
                max_flight_ms: DEFAULT_TORPEDO_MAX_FLIGHT_MS,
                dv_exhausted_emitted: false,
            });
            events.push(SpaceCombatEvent::TorpedoLaunched {
                torpedo_id: tid,
                shooter_id: shooter_id.to_owned(),
                target_id: target_id.to_owned(),
                inherited_vel_mmps: shooter_vel,
            });
        }
        events
    }

    /// Fire all offensive lasers on `shooter_id` at `target_id` (`D-35-8`).
    ///
    /// Instant speed-of-light hit; adds `heat_cost_permille` to `shooter_id` and
    /// applies diffraction-attenuated damage to `target_id`'s facing [`FacetBand`].
    pub fn fire_offensive_lasers(
        &mut self,
        shooter_id: &str,
        target_id: &str,
    ) -> Vec<SpaceCombatEvent> {
        let mut events = Vec::new();
        let (Some(shooter), Some(target)) = (self.ships.get(shooter_id), self.ships.get(target_id))
        else {
            return events;
        };
        if shooter.status != HullStatus::Operational
            || shooter_id == target_id
            || self.party_relation(shooter.party, target.party) != Relation::Hostile
        {
            return events;
        }
        let shooter_party = shooter.party;
        let target_party = target.party;
        self.party_relations.note_fire(shooter_party, target_party);
        let (Some(shooter), Some(target)) = (self.ships.get(shooter_id), self.ships.get(target_id))
        else {
            return events;
        };
        let shooter_pos = shooter.pos_m;
        let target_pos = target.pos_m;
        let dx = (target_pos[0] - shooter_pos[0]) as i128;
        let dy = (target_pos[1] - shooter_pos[1]) as i128;
        let dz = (target_pos[2] - shooter_pos[2]) as i128;
        let distance_m = isqrt_i128(dx * dx + dy * dy + dz * dz) as i64;

        let mut shots: Vec<(u32, u16)> = Vec::new();
        if let Some(shooter_mut) = self.ships.get_mut(shooter_id) {
            for mount in &mut shooter_mut.lasers {
                if mount.mode != LaserMode::Offense || mount.cooldown_remaining_ms > 0 {
                    continue;
                }
                if shooter_mut
                    .heat_permille
                    .saturating_add(mount.heat_cost_permille)
                    > 1_000
                {
                    continue;
                }
                let dmg = mount.damage_at_range(distance_m);
                if dmg == 0 {
                    continue;
                }
                shooter_mut.heat_permille = shooter_mut
                    .heat_permille
                    .saturating_add(mount.heat_cost_permille);
                mount.cooldown_remaining_ms = mount.cooldown_ms;
                shots.push((dmg, shooter_mut.heat_permille));
            }
        }

        for (dmg, shooter_heat) in shots {
            if let Some(target_mut) = self.ships.get_mut(target_id) {
                let prev_status = target_mut.status;
                let facet = target_mut.select_facet_for_attack(shooter_pos);
                let (section, armor_damage, internal_damage) = target_mut.apply_damage(facet, dmg);
                events.push(SpaceCombatEvent::LaserHit {
                    shooter_id: shooter_id.to_owned(),
                    target_id: target_id.to_owned(),
                    distance_m,
                    facet,
                    section,
                    armor_damage,
                    internal_damage,
                    shooter_heat_permille: shooter_heat,
                });
                if prev_status == HullStatus::Operational
                    && let HullStatus::DisabledDerelict {
                        epoch_open,
                        life_support_remaining_s,
                        ..
                    } = target_mut.status
                {
                    events.push(SpaceCombatEvent::ShipDisabledDerelict {
                        ship_id: target_id.to_owned(),
                        epoch_open,
                        life_support_remaining_s,
                    });
                }
            }
        }
        events
    }

    /// Fire a high-velocity unguided railgun slug from `shooter_id` at `target_id` (`D-35-8`).
    ///
    /// Computes the predicted linear intercept lead point $\mathbf{r}_{\text{lead}}$ and
    /// launches an [`InFlightSlug`]. Cannot be intercepted by PDCs or lasers, but misses if
    /// the target executes a lateral jink burn during time-of-flight!
    pub fn fire_railgun(&mut self, shooter_id: &str, target_id: &str) -> Vec<SpaceCombatEvent> {
        let mut events = Vec::new();
        let (Some(shooter), Some(target)) = (self.ships.get(shooter_id), self.ships.get(target_id))
        else {
            return events;
        };
        if shooter.status != HullStatus::Operational
            || shooter_id == target_id
            || self.party_relation(shooter.party, target.party) != Relation::Hostile
        {
            return events;
        }
        let shooter_party = shooter.party;
        let target_party = target.party;
        self.party_relations.note_fire(shooter_party, target_party);
        let (Some(shooter), Some(target)) = (self.ships.get(shooter_id), self.ships.get(target_id))
        else {
            return events;
        };
        let shooter_pos = shooter.pos_m;
        let shooter_vel = shooter.vel_mmps;
        let target_pos = target.pos_m;
        let target_vel = target.vel_mmps;
        let target_hit_radius = target.hit_radius_m;

        let dx = (target_pos[0] - shooter_pos[0]) as i128;
        let dy = (target_pos[1] - shooter_pos[1]) as i128;
        let dz = (target_pos[2] - shooter_pos[2]) as i128;
        let dist_m = isqrt_i128(dx * dx + dy * dy + dz * dz) as i64;

        let Some(shooter_mut) = self.ships.get_mut(shooter_id) else {
            return events;
        };
        let Some(rg) = shooter_mut.railguns.first_mut() else {
            return events;
        };
        if rg.cooldown_remaining_ms > 0 || rg.rounds_remaining == 0 || dist_m > rg.max_range_m {
            return events;
        }
        if shooter_mut
            .heat_permille
            .saturating_add(rg.heat_cost_permille)
            > 1_000
        {
            return events;
        }
        rg.rounds_remaining -= 1;
        rg.cooldown_remaining_ms = rg.cooldown_ms;
        shooter_mut.heat_permille = shooter_mut
            .heat_permille
            .saturating_add(rg.heat_cost_permille);

        let muz_mps = rg.muzzle_vel_mps;
        let dmg = rg.slug_damage;
        let (lead_pos_m, tof_ms) =
            compute_railgun_lead(shooter_pos, shooter_vel, target_pos, target_vel, muz_mps);

        let lx = (lead_pos_m[0] - shooter_pos[0]) as i128;
        let ly = (lead_pos_m[1] - shooter_pos[1]) as i128;
        let lz = (lead_pos_m[2] - shooter_pos[2]) as i128;
        let tof_i128 = i128::from(tof_ms.max(1));
        let vel_mmps = [
            ((lx * 1_000_000) / tof_i128) as i64,
            ((ly * 1_000_000) / tof_i128) as i64,
            ((lz * 1_000_000) / tof_i128) as i64,
        ];

        let slug_id = self.next_ordnance_id;
        self.next_ordnance_id = self.next_ordnance_id.saturating_add(1);
        self.railgun_slugs.push(InFlightSlug {
            id: slug_id,
            shooter_id: shooter_id.to_owned(),
            target_id: target_id.to_owned(),
            pos_m: shooter_pos,
            pos_rem_um: [0, 0, 0],
            vel_mmps,
            aimed_lead_pos_m: lead_pos_m,
            remaining_flight_ms: tof_ms,
            hit_radius_m: target_hit_radius,
            slug_damage: dmg,
        });
        events.push(SpaceCombatEvent::RailgunFired {
            slug_id,
            shooter_id: shooter_id.to_owned(),
            target_id: target_id.to_owned(),
            aimed_lead_pos_m: lead_pos_m,
            flight_ms: tof_ms,
        });
        events
    }

    /// Advance the bubble on **1:1 wall time** by `dt_wall_ms` while stamping
    /// the caller-provided W-clock `ic_now_s` (`D-35-3`, `Q-CF-32`, `D-16-2`, `35a` §5.3).
    ///
    /// Substeps in chunks of at most [`MAX_SUBSTEP_MS`] ($250\text{ ms}$) and enforces
    /// a monotonic guard on `self.ic_timestamp_s` so a rewound caller timestamp can
    /// never move the bubble's IC stamp backward.
    pub fn step_wall_ms_at(&mut self, dt_wall_ms: u32, ic_now_s: i64) -> BubbleStepReport {
        if self.outage_frozen || dt_wall_ms == 0 {
            self.ic_timestamp_s = self.ic_timestamp_s.max(ic_now_s);
            return BubbleStepReport {
                wall_advanced_ms: 0,
                substeps_executed: 0,
                events: Vec::new(),
                state_hash: self.state_hash(),
            };
        }

        let prev_wall_s = self.wall_elapsed_ms / 1_000;
        let mut remaining_ms = dt_wall_ms;
        let mut substeps = 0u32;
        let mut events = Vec::new();

        while remaining_ms > 0 {
            let step_ms = remaining_ms.min(MAX_SUBSTEP_MS);
            remaining_ms -= step_ms;
            substeps += 1;
            self.wall_elapsed_ms = self.wall_elapsed_ms.saturating_add(u64::from(step_ms));
            self.step_substep(step_ms, &mut events);
        }

        let new_wall_s = self.wall_elapsed_ms / 1_000;
        let elapsed_whole_s =
            (new_wall_s.saturating_sub(prev_wall_s)).min(u64::from(u32::MAX)) as u32;
        if elapsed_whole_s > 0 {
            for ship in self.ships.values_mut() {
                if let HullStatus::DisabledDerelict {
                    ref mut life_support_remaining_s,
                    ref mut salvage_contest_window_s,
                    ..
                } = ship.status
                {
                    *life_support_remaining_s =
                        life_support_remaining_s.saturating_sub(elapsed_whole_s);
                    *salvage_contest_window_s =
                        salvage_contest_window_s.saturating_sub(elapsed_whole_s);
                }
            }
        }
        self.ic_timestamp_s = self.ic_timestamp_s.max(ic_now_s);

        BubbleStepReport {
            wall_advanced_ms: dt_wall_ms,
            substeps_executed: substeps,
            events,
            state_hash: self.state_hash(),
        }
    }

    fn step_substep(&mut self, step_ms: u32, events: &mut Vec<SpaceCombatEvent>) {
        let step_i64 = i64::from(step_ms);

        // 1. Step ship kinematics, propellant debit (`D-35-6`), linkdead cut (`D-35-7`), and cooling
        for ship in self.ships.values_mut() {
            if ship.crew_linkdead || ship.status != HullStatus::Operational {
                ship.thrust_mps2 = [0, 0, 0];
                ship.thrust_mmps2 = [0, 0, 0];
            }

            let eff_thrust_mmps2 = if ship.thrust_mmps2 != [0, 0, 0] {
                ship.thrust_mmps2
            } else {
                [
                    ship.thrust_mps2[0].saturating_mul(1_000),
                    ship.thrust_mps2[1].saturating_mul(1_000),
                    ship.thrust_mps2[2].saturating_mul(1_000),
                ]
            };
            let ax = eff_thrust_mmps2[0];
            let ay = eff_thrust_mmps2[1];
            let az = eff_thrust_mmps2[2];
            let accel_mag_mmps2 =
                isqrt_i128((ax as i128).pow(2) + (ay as i128).pow(2) + (az as i128).pow(2)) as i64;

            if accel_mag_mmps2 > 0 && ship.propellant_kg > 0 && ship.delta_v_mps > 0 {
                // dv_mmps = (a_mmps2 * step_ms) / 1000
                ship.vel_mmps[0] = ship.vel_mmps[0].saturating_add((ax * step_i64) / 1_000);
                ship.vel_mmps[1] = ship.vel_mmps[1].saturating_add((ay * step_i64) / 1_000);
                ship.vel_mmps[2] = ship.vel_mmps[2].saturating_add((az * step_i64) / 1_000);

                let dv_mmps = (accel_mag_mmps2.saturating_mul(step_i64)) / 1_000;
                let total_dv_mmps = ship.dv_rem_mmps.saturating_add(dv_mmps);
                let dv_spent_mps = (total_dv_mmps / 1_000).max(0);
                ship.dv_rem_mmps = total_dv_mmps % 1_000;
                if dv_spent_mps > 0 {
                    ship.delta_v_mps = (ship.delta_v_mps - dv_spent_mps).max(0);
                    let law = crate::definitions::logistics::DriveLaw::Flip {
                        accel_mg: ship.rated_accel_mg,
                        exhaust_kms: ship.exhaust_kms.max(1),
                    };
                    let dry_kg = u64::from(ship.dry_mass_t).saturating_mul(1_000);
                    let payload_kg = ship
                        .non_prop_payload_kg
                        .saturating_add(u64::from(ship.propellant_kg));
                    let dv_u32 = u32::try_from(dv_spent_mps).unwrap_or(1).max(1);
                    let fuel_burned =
                        crate::port::passage::burn_kg_for_law_dv(&law, dry_kg, payload_kg, dv_u32)
                            .unwrap_or(1)
                            .max(1);
                    ship.propellant_kg = ship
                        .propellant_kg
                        .saturating_sub(u32::try_from(fuel_burned).unwrap_or(u32::MAX));
                }
                if ship.propellant_kg == 0 || ship.delta_v_mps == 0 {
                    ship.thrust_mps2 = [0, 0, 0];
                    ship.thrust_mmps2 = [0, 0, 0];
                }
            }

            // Integrate position with sub-metre micrometre remainder (`35a` §5.2):
            advance_pos_with_rem(
                &mut ship.pos_m,
                &mut ship.pos_rem_um,
                ship.vel_mmps,
                step_ms,
            );

            // Weapon cooldowns and radiator cooling
            for t in &mut ship.torpedo_tubes {
                t.cooldown_remaining_ms = t.cooldown_remaining_ms.saturating_sub(step_ms);
            }
            for l in &mut ship.lasers {
                l.cooldown_remaining_ms = l.cooldown_remaining_ms.saturating_sub(step_ms);
            }
            for r in &mut ship.railguns {
                r.cooldown_remaining_ms = r.cooldown_remaining_ms.saturating_sub(step_ms);
            }
            for p in &mut ship.pdcs {
                p.cooldown_remaining_ms = p.cooldown_remaining_ms.saturating_sub(step_ms);
            }

            let base_cooling = if ship.crits.contains(&SubsystemCrit::RadiatorDamaged) {
                ship.cooling_rate_permille_s / 2
            } else {
                ship.cooling_rate_permille_s
            };
            let eff_cooling = match ship.emcon {
                EmconPosture::Ghost => (base_cooling / 4).max(1),
                EmconPosture::Passive => ((base_cooling * 3) / 4).max(1),
                EmconPosture::Standard | EmconPosture::ActiveCombat => base_cooling,
            };
            let cooled = ((u32::from(eff_cooling) * step_ms) / 1_000) as u16;
            ship.heat_permille = ship.heat_permille.saturating_sub(cooled);
            ship.active_decoy_ms = ship.active_decoy_ms.saturating_sub(step_ms);
        }

        // 2. Step guided torpedoes (`D-35-8`): proportional navigation + ballistic coast + defensive screen
        let old_torpedoes = std::mem::take(&mut self.torpedoes);
        let mut surviving_torpedoes = Vec::with_capacity(old_torpedoes.len());
        for mut torp in old_torpedoes {
            torp.flight_elapsed_ms = torp.flight_elapsed_ms.saturating_add(step_ms);
            if torp.max_flight_ms > 0 && torp.flight_elapsed_ms > torp.max_flight_ms {
                continue;
            }
            let Some(target) = self.ships.get(&torp.target_id) else {
                continue;
            };
            let target_pos = target.pos_m;
            let target_vel = target.vel_mmps;
            let target_party = target.party;

            let dx = (target_pos[0] - torp.pos_m[0]) as i128;
            let dy = (target_pos[1] - torp.pos_m[1]) as i128;
            let dz = (target_pos[2] - torp.pos_m[2]) as i128;
            let dist_m = isqrt_i128(dx * dx + dy * dy + dz * dz).max(1);

            // Proportional navigation burn toward predicted intercept point while delta-v remains (`35a` §5.9)
            let step_dv_mmps = torp.accel_mps2 * step_i64;
            if torp.remaining_dv_mmps > 0 {
                let burn_mmps = step_dv_mmps.min(torp.remaining_dv_mmps);
                torp.remaining_dv_mmps -= burn_mmps;

                let rvx_mps = ((target_vel[0] - torp.vel_mmps[0]) / 1_000) as i128;
                let rvy_mps = ((target_vel[1] - torp.vel_mmps[1]) / 1_000) as i128;
                let rvz_mps = ((target_vel[2] - torp.vel_mmps[2]) / 1_000) as i128;
                let closing_speed_mps =
                    isqrt_i128(rvx_mps * rvx_mps + rvy_mps * rvy_mps + rvz_mps * rvz_mps)
                        .max((torp.accel_mps2 as i128) * 2)
                        .max(100);
                let t_go_s = (dist_m / closing_speed_mps).clamp(0, 10);

                let aim_x = dx + rvx_mps * t_go_s;
                let aim_y = dy + rvy_mps * t_go_s;
                let aim_z = dz + rvz_mps * t_go_s;
                let aim_dist = isqrt_i128(aim_x * aim_x + aim_y * aim_y + aim_z * aim_z).max(1);

                let ux = (aim_x * 1_000) / aim_dist;
                let uy = (aim_y * 1_000) / aim_dist;
                let uz = (aim_z * 1_000) / aim_dist;
                torp.vel_mmps[0] =
                    torp.vel_mmps[0].saturating_add(((ux * i128::from(burn_mmps)) / 1_000) as i64);
                torp.vel_mmps[1] =
                    torp.vel_mmps[1].saturating_add(((uy * i128::from(burn_mmps)) / 1_000) as i64);
                torp.vel_mmps[2] =
                    torp.vel_mmps[2].saturating_add(((uz * i128::from(burn_mmps)) / 1_000) as i64);

                if torp.remaining_dv_mmps == 0 && !torp.dv_exhausted_emitted {
                    torp.dv_exhausted_emitted = true;
                    events.push(SpaceCombatEvent::TorpedoExhaustedDeltaV {
                        torpedo_id: torp.id,
                    });
                }
            } else if !torp.dv_exhausted_emitted {
                torp.dv_exhausted_emitted = true;
                events.push(SpaceCombatEvent::TorpedoExhaustedDeltaV {
                    torpedo_id: torp.id,
                });
            }

            // Advance torpedo position (continues coasting ballistically even when remaining_dv_mmps == 0)
            advance_pos_with_rem(
                &mut torp.pos_m,
                &mut torp.pos_rem_um,
                torp.vel_mmps,
                step_ms,
            );

            // Recompute distance after step
            let ndx = (target_pos[0] - torp.pos_m[0]) as i128;
            let ndy = (target_pos[1] - torp.pos_m[1]) as i128;
            let ndz = (target_pos[2] - torp.pos_m[2]) as i128;
            let new_dist_m = isqrt_i128(ndx * ndx + ndy * ndy + ndz * ndz) as i64;

            let rel_vx = ((torp.vel_mmps[0] - target_vel[0]) / 1_000) as i128;
            let rel_vy = ((torp.vel_mmps[1] - target_vel[1]) / 1_000) as i128;
            let rel_vz = ((torp.vel_mmps[2] - target_vel[2]) / 1_000) as i128;
            let closing_mps =
                isqrt_i128(rel_vx * rel_vx + rel_vy * rel_vy + rel_vz * rel_vz).max(100) as i64;
            torp.tti_s = if torp.remaining_dv_mmps > 0 && torp.accel_mps2 > 0 {
                let v = i128::from(closing_mps);
                let a = i128::from(torp.accel_mps2);
                let d = i128::from(new_dist_m.max(0));
                let disc = v.saturating_mul(v).saturating_add(2 * a * d);
                ((isqrt_i128(disc) - v) / a).max(0) as i64
            } else {
                (new_dist_m / closing_mps).max(0)
            };

            // Check automatic defensive laser & PDC interception across target and escorting ships
            let shooter_party = self.ships.get(&torp.shooter_id).map(|s| s.party);
            let defender_ids: Vec<String> = self
                .ships
                .values()
                .filter(|s| {
                    if s.id == torp.shooter_id || s.status != HullStatus::Operational {
                        return false;
                    }
                    let rel_to_target = self.party_relation(s.party, target_party);
                    rel_to_target.is_friendly()
                        || (rel_to_target == Relation::Neutral
                            && (self.is_inspecting(s.party)
                                || shooter_party.is_some_and(|sp| {
                                    self.party_relation(s.party, sp) == Relation::Hostile
                                })))
                })
                .map(|s| s.id.clone())
                .collect();

            let mut intercepted = false;
            for def_id in defender_ids {
                if intercepted {
                    break;
                }
                let Some(def_ship) = self.ships.get_mut(&def_id) else {
                    continue;
                };
                let ddx = (def_ship.pos_m[0] - torp.pos_m[0]) as i128;
                let ddy = (def_ship.pos_m[1] - torp.pos_m[1]) as i128;
                let ddz = (def_ship.pos_m[2] - torp.pos_m[2]) as i128;
                let def_dist_m = isqrt_i128(ddx * ddx + ddy * ddy + ddz * ddz) as i64;

                // 2a. Defensive-mode lasers (`LaserMode::Defense`)
                for laser in &mut def_ship.lasers {
                    if laser.mode == LaserMode::Defense
                        && laser.cooldown_remaining_ms == 0
                        && def_dist_m <= laser.defense_range_m
                        && def_ship
                            .heat_permille
                            .saturating_add(laser.heat_cost_permille)
                            <= 1_000
                    {
                        def_ship.heat_permille = def_ship
                            .heat_permille
                            .saturating_add(laser.heat_cost_permille);
                        laser.cooldown_remaining_ms = laser.cooldown_ms;
                        events.push(SpaceCombatEvent::TorpedoInterceptedByLaser {
                            torpedo_id: torp.id,
                            defender_id: def_id.clone(),
                            laser_id: laser.id.clone(),
                        });
                        intercepted = true;
                        break;
                    }
                }
                if intercepted {
                    break;
                }

                // 2b. Active ECM / thermal-radar decoy spoofing (`+tac/decoy`, `SF.15`, `35a` §5.8):
                // Only spoofs torpedoes locked onto the ship that deployed the decoy.
                if def_id == torp.target_id && def_ship.active_decoy_ms > 0 && def_dist_m <= 35_000
                {
                    def_ship.active_decoy_ms = 0;
                    events.push(SpaceCombatEvent::TorpedoSpoofedByDecoy {
                        torpedo_id: torp.id,
                        defender_id: def_id.clone(),
                        decoys_remaining: def_ship.decoys_remaining,
                    });
                    intercepted = true;
                    break;
                }

                // 2c. Automatic PDC screen (`+tac/pdc auto`, `35a` §5.8):
                // Rate-limited by `pdc.cooldown_remaining_ms` and accumulates deterministic
                // `damage_per_burst` against `torp.integrity` (`100` at launch).
                if def_ship.pdc_auto {
                    for pdc in &mut def_ship.pdcs {
                        if pdc.operational
                            && pdc.cooldown_remaining_ms == 0
                            && pdc.ammo_remaining >= u32::from(pdc.burst_rounds)
                            && def_dist_m <= pdc.envelope_m
                        {
                            pdc.ammo_remaining -= u32::from(pdc.burst_rounds);
                            pdc.cooldown_remaining_ms = pdc.cooldown_ms;
                            torp.integrity =
                                torp.integrity.saturating_sub(pdc.damage_per_burst.max(1));
                            if torp.integrity == 0 {
                                events.push(SpaceCombatEvent::TorpedoInterceptedByPdc {
                                    torpedo_id: torp.id,
                                    defender_id: def_id.clone(),
                                    pdc_id: pdc.id.clone(),
                                });
                                intercepted = true;
                                break;
                            }
                        }
                    }
                }
            }

            if intercepted {
                continue;
            }

            // Check terminal impact
            let impact_threshold_m = ((closing_mps * step_i64) / 1_000).max(500);
            if new_dist_m <= impact_threshold_m {
                if let Some(target_mut) = self.ships.get_mut(&torp.target_id) {
                    let prev_status = target_mut.status;
                    let facet = target_mut.select_facet_for_attack(torp.pos_m);
                    let (section, armor_damage, internal_damage) =
                        target_mut.apply_damage(facet, torp.warhead_damage);
                    events.push(SpaceCombatEvent::TorpedoHit {
                        torpedo_id: torp.id,
                        target_id: torp.target_id.clone(),
                        facet,
                        section,
                        armor_damage,
                        internal_damage,
                    });
                    if prev_status == HullStatus::Operational
                        && let HullStatus::DisabledDerelict {
                            epoch_open,
                            life_support_remaining_s,
                            ..
                        } = target_mut.status
                    {
                        events.push(SpaceCombatEvent::ShipDisabledDerelict {
                            ship_id: torp.target_id.clone(),
                            epoch_open,
                            life_support_remaining_s,
                        });
                    }
                }
                continue;
            }

            surviving_torpedoes.push(torp);
        }
        self.torpedoes = surviving_torpedoes;

        // 3. Step unguided railgun slugs (`D-35-8`): uninterceptable by PDCs, dodged by jinking!
        let mut surviving_slugs = Vec::with_capacity(self.railgun_slugs.len());
        for mut slug in self.railgun_slugs.drain(..) {
            let dt = step_ms.min(slug.remaining_flight_ms);
            advance_pos_with_rem(&mut slug.pos_m, &mut slug.pos_rem_um, slug.vel_mmps, dt);
            slug.remaining_flight_ms -= dt;

            if slug.remaining_flight_ms == 0 {
                if let Some(target_mut) = self.ships.get_mut(&slug.target_id) {
                    let overstep_ms = i128::from(step_ms.saturating_sub(dt));
                    let target_impact_x_um = i128::from(target_mut.pos_m[0]) * 1_000_000
                        + i128::from(target_mut.pos_rem_um[0])
                        - i128::from(target_mut.vel_mmps[0]) * overstep_ms;
                    let target_impact_y_um = i128::from(target_mut.pos_m[1]) * 1_000_000
                        + i128::from(target_mut.pos_rem_um[1])
                        - i128::from(target_mut.vel_mmps[1]) * overstep_ms;
                    let target_impact_z_um = i128::from(target_mut.pos_m[2]) * 1_000_000
                        + i128::from(target_mut.pos_rem_um[2])
                        - i128::from(target_mut.vel_mmps[2]) * overstep_ms;
                    let jx_um =
                        target_impact_x_um - i128::from(slug.aimed_lead_pos_m[0]) * 1_000_000;
                    let jy_um =
                        target_impact_y_um - i128::from(slug.aimed_lead_pos_m[1]) * 1_000_000;
                    let jz_um =
                        target_impact_z_um - i128::from(slug.aimed_lead_pos_m[2]) * 1_000_000;
                    let displacement_um =
                        isqrt_i128(jx_um * jx_um + jy_um * jy_um + jz_um * jz_um) as i64;
                    let displacement_m = (displacement_um + 999_999) / 1_000_000;

                    if displacement_um > slug.hit_radius_m.saturating_mul(1_000_000) {
                        events.push(SpaceCombatEvent::RailgunDodgedByJink {
                            slug_id: slug.id,
                            shooter_id: slug.shooter_id,
                            target_id: slug.target_id,
                            displacement_from_lead_m: displacement_m,
                        });
                    } else {
                        let prev_status = target_mut.status;
                        let facet = target_mut.select_facet_for_attack(slug.pos_m);
                        let (section, armor_damage, internal_damage) =
                            target_mut.apply_damage(facet, slug.slug_damage);
                        events.push(SpaceCombatEvent::RailgunHit {
                            slug_id: slug.id,
                            shooter_id: slug.shooter_id,
                            target_id: slug.target_id.clone(),
                            facet,
                            section,
                            armor_damage,
                            internal_damage,
                        });
                        if prev_status == HullStatus::Operational
                            && let HullStatus::DisabledDerelict {
                                epoch_open,
                                life_support_remaining_s,
                                ..
                            } = target_mut.status
                        {
                            events.push(SpaceCombatEvent::ShipDisabledDerelict {
                                ship_id: slug.target_id,
                                epoch_open,
                                life_support_remaining_s,
                            });
                        }
                    }
                }
            } else {
                surviving_slugs.push(slug);
            }
        }
        self.railgun_slugs = surviving_slugs;
    }

    /// Deterministic 64-bit FNV-1a digest of the bubble's authoritative state.
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |bytes: &[u8]| {
            for &b in bytes {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        mix(&self.wall_elapsed_ms.to_le_bytes());
        mix(&self.ic_timestamp_s.to_le_bytes());
        for (id, ship) in &self.ships {
            mix(id.as_bytes());
            for p in ship.pos_m {
                mix(&p.to_le_bytes());
            }
            for r in ship.pos_rem_um {
                mix(&r.to_le_bytes());
            }
            for v in ship.vel_mmps {
                mix(&v.to_le_bytes());
            }
            mix(&ship.propellant_kg.to_le_bytes());
            mix(&ship.heat_permille.to_le_bytes());
            for sec in ship.sections.values() {
                mix(&sec.internals.to_le_bytes());
            }
        }
        for torp in &self.torpedoes {
            mix(&torp.id.to_le_bytes());
            mix(&torp.integrity.to_le_bytes());
            for p in torp.pos_m {
                mix(&p.to_le_bytes());
            }
            for r in torp.pos_rem_um {
                mix(&r.to_le_bytes());
            }
        }
        for slug in &self.railgun_slugs {
            mix(&slug.id.to_le_bytes());
            for p in slug.pos_m {
                mix(&p.to_le_bytes());
            }
            for r in slug.pos_rem_um {
                mix(&r.to_le_bytes());
            }
            mix(&slug.remaining_flight_ms.to_le_bytes());
        }
        h
    }

    /// Build a `space.holotank` feed JSON keyframe from `observer_ship_id`'s perspective,
    /// ready for `mush_pixel::holotank::HolotankPicture`.
    #[must_use]
    pub fn holotank_feed(&self, observer_ship_id: &str) -> Value {
        let obs = self.ships.get(observer_ship_id);
        let obs_party = obs.map(|s| s.party);
        let locked_id = obs.and_then(|s| s.locked_target_id.as_deref());
        let pdc_range_km = obs
            .and_then(|s| s.pdcs.first())
            .map(|p| (p.envelope_m as f64) / 1_000.0)
            .unwrap_or(15.0);

        let mut contacts = Vec::with_capacity(self.ships.len());
        for ship in self.ships.values() {
            if ship.id != observer_ship_id
                && !obs.is_some_and(|o| {
                    super::commands::is_bubble_ship_detected(o, ship, self.ic_timestamp_s)
                })
            {
                continue;
            }
            let rel = obs_party.map_or("neutral", |viewer| {
                relation_feed_word(self.party_relation(viewer, ship.party))
            });
            contacts.push(json!({
                "id": ship.id,
                "label": ship.callsign,
                "kind": if matches!(ship.status, HullStatus::DisabledDerelict { .. }) {
                    "derelict"
                } else {
                    "ship"
                },
                "relation": rel,
                "you": ship.id == observer_ship_id,
                "locked": locked_id == Some(ship.id.as_str()),
                "x_km": (ship.pos_m[0] as f64) / 1_000.0,
                "y_km": (ship.pos_m[1] as f64) / 1_000.0,
                "z_km": (ship.pos_m[2] as f64) / 1_000.0,
                "vx_kms": (ship.vel_mmps[0] as f64) / 1_000_000.0,
                "vy_kms": (ship.vel_mmps[1] as f64) / 1_000_000.0,
                "vz_kms": (ship.vel_mmps[2] as f64) / 1_000_000.0,
                "ax_mps2": ship.thrust_mps2[0],
                "ay_mps2": ship.thrust_mps2[1],
                "az_mps2": ship.thrust_mps2[2],
            }));
        }

        let mut torpedoes = Vec::with_capacity(self.torpedoes.len());
        for torp in &self.torpedoes {
            if obs.is_some_and(|o| {
                !super::commands::is_bubble_torpedo_detected(o, torp, self.ic_timestamp_s)
            }) {
                continue;
            }
            let target_pos = self
                .ships
                .get(&torp.target_id)
                .filter(|s| {
                    s.id == observer_ship_id
                        || obs.is_some_and(|o| {
                            super::commands::is_bubble_ship_detected(o, s, self.ic_timestamp_s)
                        })
                })
                .map(|s| s.pos_m)
                .unwrap_or(torp.pos_m);
            torpedoes.push(json!({
                "id": torp.id,
                "x_km": (torp.pos_m[0] as f64) / 1_000.0,
                "y_km": (torp.pos_m[1] as f64) / 1_000.0,
                "z_km": (torp.pos_m[2] as f64) / 1_000.0,
                "target_x_km": (target_pos[0] as f64) / 1_000.0,
                "target_y_km": (target_pos[1] as f64) / 1_000.0,
                "target_z_km": (target_pos[2] as f64) / 1_000.0,
                "tti_s": torp.tti_s,
            }));
        }

        json!({
            "title": self.title,
            "range_km": 250.0,
            "pdc_range_km": pdc_range_km,
            "contacts": contacts,
            "torpedoes": torpedoes,
        })
    }

    /// Build a `space.cockpit` feed JSON keyframe from `observer_ship_id`'s bridge canopy,
    /// ready for `mush_pixel::space_cockpit::SpaceCockpitPicture`.
    #[must_use]
    pub fn cockpit_feed(&self, observer_ship_id: &str) -> Value {
        let Some(obs) = self.ships.get(observer_ship_id) else {
            return json!({ "refused": true });
        };
        let (yaw_deg, pitch_deg, roll_deg) = obs.attitude_deg;
        let (prograde_b, prograde_p) = vector_to_bearing_pitch(obs.vel_mmps);
        let muz_mps = obs
            .railguns
            .first()
            .map(|r| r.muzzle_vel_mps)
            .unwrap_or(20_000);

        let mut contacts = Vec::new();
        for ship in self.ships.values() {
            if ship.id == observer_ship_id
                || !super::commands::is_bubble_ship_detected(obs, ship, self.ic_timestamp_s)
            {
                continue;
            }
            let rel_pos = [
                ship.pos_m[0] - obs.pos_m[0],
                ship.pos_m[1] - obs.pos_m[1],
                ship.pos_m[2] - obs.pos_m[2],
            ];
            let (bearing_deg, pitch_c) = vector_to_bearing_pitch(rel_pos);
            let locked = obs.locked_target_id.as_deref() == Some(ship.id.as_str());
            let (lead_pos, _) =
                compute_railgun_lead(obs.pos_m, obs.vel_mmps, ship.pos_m, ship.vel_mmps, muz_mps);
            let (lead_b, lead_p) = vector_to_bearing_pitch([
                lead_pos[0] - obs.pos_m[0],
                lead_pos[1] - obs.pos_m[1],
                lead_pos[2] - obs.pos_m[2],
            ]);

            contacts.push(json!({
                "id": ship.id,
                "label": ship.callsign,
                "kind": if matches!(ship.status, HullStatus::DisabledDerelict { .. }) {
                    "derelict"
                } else {
                    "ship"
                },
                "pixel_model": ship.pixel_model(),
                "relation": relation_feed_word(self.party_relation(obs.party, ship.party)),
                "locked": locked,
                "burning": ship.is_burning(),
                "bearing_deg": bearing_deg,
                "pitch_deg": pitch_c,
                "lead_bearing_deg": lead_b,
                "lead_pitch_deg": lead_p,
            }));
        }

        for torp in &self.torpedoes {
            if !super::commands::is_bubble_torpedo_detected(obs, torp, self.ic_timestamp_s) {
                continue;
            }
            let rel_pos = [
                torp.pos_m[0] - obs.pos_m[0],
                torp.pos_m[1] - obs.pos_m[1],
                torp.pos_m[2] - obs.pos_m[2],
            ];
            let (bearing_deg, pitch_c) = vector_to_bearing_pitch(rel_pos);
            contacts.push(json!({
                "id": format!("torp.{}", torp.id),
                "label": format!("TORP-{}", torp.id),
                "kind": "torpedo",
                "pixel_model": "torpedo",
                "relation": if torp.shooter_id == observer_ship_id {
                    "friend"
                } else {
                    "hostile"
                },
                "locked": false,
                "burning": torp.remaining_dv_mmps > 0,
                "bearing_deg": bearing_deg,
                "pitch_deg": pitch_c,
                "tti_s": torp.tti_s,
            }));
        }

        let bodies_json: Vec<Value> = if self.bodies.is_empty() {
            default_canopy_bodies_for_site(&self.site_id)
                .iter()
                .map(CanopyBodyProjection::to_feed_json)
                .collect()
        } else {
            self.bodies
                .iter()
                .map(CanopyBodyProjection::to_feed_json)
                .collect()
        };

        json!({
            "callsign": obs.callsign,
            "yaw_deg": yaw_deg,
            "pitch_deg": pitch_deg,
            "roll_deg": roll_deg,
            "speed_mps": obs.speed_mps(),
            "prograde_bearing_deg": prograde_b,
            "prograde_pitch_deg": prograde_p,
            "delta_v_mps": obs.delta_v_mps,
            "heat_permille": obs.heat_permille,
            "hull_pct": obs.hull_integrity_pct(),
            "pdc_mode": if obs.pdc_auto { "AUTO" } else { "MAN" },
            "emcon": obs.emcon.code_str(),
            "bodies": bodies_json,
            "contacts": contacts,
        })
    }

    /// Capture a serializable mid-battle snapshot of this bubble (`D-35-1`, `35a` §8.2).
    #[must_use]
    pub fn snapshot(&self) -> BubbleSnapshot {
        BubbleSnapshot {
            bubble: self.clone(),
            state_hash: self.state_hash(),
        }
    }

    /// Restore a [`LocalSpaceBubble`] from a [`BubbleSnapshot`].
    #[must_use]
    pub fn from_snapshot(snapshot: BubbleSnapshot) -> Self {
        snapshot.bubble
    }

    /// Apply a single [`BubbleInputCommand`] deterministically (`D-35-1`, `35a` §8.2).
    pub fn apply_input(&mut self, cmd: &BubbleInputCommand) -> Vec<SpaceCombatEvent> {
        match cmd {
            BubbleInputCommand::CommandBurn {
                ship_id,
                thrust_mps2,
                force_into_reserve,
            } => match self.command_burn(ship_id, *thrust_mps2, *force_into_reserve) {
                Ok(()) => Vec::new(),
                Err(ev) => vec![ev],
            },
            BubbleInputCommand::CommandBurnMmps2 {
                ship_id,
                thrust_mmps2,
                force_into_reserve,
            } => match self.command_burn_mmps2(ship_id, *thrust_mmps2, *force_into_reserve) {
                Ok(_) => Vec::new(),
                Err(ev) => vec![ev],
            },
            BubbleInputCommand::SetAttitude {
                ship_id,
                attitude_deg,
            } => {
                if let Some(ship) = self.ships.get_mut(ship_id) {
                    ship.attitude_deg = *attitude_deg;
                }
                Vec::new()
            }
            BubbleInputCommand::SetPdcAuto { ship_id, enabled } => {
                let _ = self.set_pdc_auto(ship_id, *enabled);
                Vec::new()
            }
            BubbleInputCommand::SetLaserMode {
                ship_id,
                laser_id,
                mode,
            } => {
                let _ = self.set_laser_mode(ship_id, laser_id, *mode);
                Vec::new()
            }
            BubbleInputCommand::SetEmcon { ship_id, posture } => {
                let _ = self.set_emcon(ship_id, *posture);
                Vec::new()
            }
            BubbleInputCommand::SetLockedTarget { ship_id, target_id } => {
                let _ = self.set_locked_target(ship_id, target_id.clone());
                Vec::new()
            }
            BubbleInputCommand::LaunchDecoy { ship_id } => {
                let _ = self.launch_decoy(ship_id);
                Vec::new()
            }
            BubbleInputCommand::BlowdownCoolant { ship_id } => {
                self.blowdown_coolant(ship_id).into_iter().collect()
            }
            BubbleInputCommand::RepairCrit { ship_id, crit } => self
                .repair_subsystem_crit(ship_id, *crit)
                .into_iter()
                .collect(),
            BubbleInputCommand::FireTorpedoSalvo {
                shooter_id,
                target_id,
                count,
            } => self.fire_torpedo_salvo(shooter_id, target_id, *count),
            BubbleInputCommand::FireOffensiveLasers {
                shooter_id,
                target_id,
            } => self.fire_offensive_lasers(shooter_id, target_id),
            BubbleInputCommand::FireRailgun {
                shooter_id,
                target_id,
            } => self.fire_railgun(shooter_id, target_id),
            BubbleInputCommand::StepWall {
                dt_wall_ms,
                ic_now_s,
            } => self.step_wall_ms_at(*dt_wall_ms, *ic_now_s).events,
        }
    }
}

/// Serializable snapshot of a [`LocalSpaceBubble`] (`D-35-1`, `35a` §8.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BubbleSnapshot {
    pub bubble: LocalSpaceBubble,
    pub state_hash: u64,
}

/// Deterministic input command recorded on a [`BubbleInputTape`] (`D-35-1`, `35a` §8.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BubbleInputCommand {
    CommandBurn {
        ship_id: String,
        thrust_mps2: [i64; 3],
        force_into_reserve: bool,
    },
    CommandBurnMmps2 {
        ship_id: String,
        thrust_mmps2: [i64; 3],
        force_into_reserve: bool,
    },
    SetAttitude {
        ship_id: String,
        attitude_deg: (i32, i32, i32),
    },
    SetPdcAuto {
        ship_id: String,
        enabled: bool,
    },
    SetLaserMode {
        ship_id: String,
        laser_id: String,
        mode: LaserMode,
    },
    SetEmcon {
        ship_id: String,
        posture: EmconPosture,
    },
    SetLockedTarget {
        ship_id: String,
        target_id: Option<String>,
    },
    LaunchDecoy {
        ship_id: String,
    },
    BlowdownCoolant {
        ship_id: String,
    },
    RepairCrit {
        ship_id: String,
        crit: SubsystemCrit,
    },
    FireTorpedoSalvo {
        shooter_id: String,
        target_id: String,
        count: u8,
    },
    FireOffensiveLasers {
        shooter_id: String,
        target_id: String,
    },
    FireRailgun {
        shooter_id: String,
        target_id: String,
    },
    StepWall {
        dt_wall_ms: u32,
        ic_now_s: i64,
    },
}

/// Timestamped entry on a [`BubbleInputTape`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BubbleInputEntry {
    pub wall_elapsed_ms: u64,
    pub command: BubbleInputCommand,
}

/// Deterministic input tape for a [`LocalSpaceBubble`] (`D-35-1`, `35a` §8.2).
///
/// Replaying a [`BubbleInputTape`] from its `initial_snapshot` (or from a
/// mid-battle [`BubbleSnapshot`] plus the remaining tail of entries) reproduces
/// the exact [`LocalSpaceBubble::state_hash`] and [`SpaceCombatEvent`] stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BubbleInputTape {
    pub initial_snapshot: BubbleSnapshot,
    pub entries: Vec<BubbleInputEntry>,
}

impl BubbleInputTape {
    /// Start recording a new input tape from `bubble`'s current state.
    #[must_use]
    pub fn new(bubble: &LocalSpaceBubble) -> Self {
        Self {
            initial_snapshot: bubble.snapshot(),
            entries: Vec::new(),
        }
    }

    /// Record `command` on the tape and apply it to `bubble`.
    pub fn record_and_apply(
        &mut self,
        bubble: &mut LocalSpaceBubble,
        command: BubbleInputCommand,
    ) -> Vec<SpaceCombatEvent> {
        self.entries.push(BubbleInputEntry {
            wall_elapsed_ms: bubble.wall_elapsed_ms,
            command: command.clone(),
        });
        bubble.apply_input(&command)
    }

    /// Replay the entire tape from `initial_snapshot`, returning the resulting
    /// [`LocalSpaceBubble`] and all emitted [`SpaceCombatEvent`]s.
    #[must_use]
    pub fn replay(&self) -> (LocalSpaceBubble, Vec<SpaceCombatEvent>) {
        self.replay_from_snapshot(self.initial_snapshot.clone(), 0)
    }

    /// Replay entries starting at `start_entry_idx` on top of `snapshot`.
    #[must_use]
    pub fn replay_from_snapshot(
        &self,
        snapshot: BubbleSnapshot,
        start_entry_idx: usize,
    ) -> (LocalSpaceBubble, Vec<SpaceCombatEvent>) {
        let mut bubble = LocalSpaceBubble::from_snapshot(snapshot);
        let mut events = Vec::new();
        for entry in self.entries.iter().skip(start_entry_idx) {
            events.extend(bubble.apply_input(&entry.command));
        }
        (bubble, events)
    }
}

fn advance_pos_with_rem(
    pos_m: &mut [i64; 3],
    pos_rem_um: &mut [i64; 3],
    vel_mmps: [i64; 3],
    dt_ms: u32,
) {
    let dt = i128::from(dt_ms);
    for axis in 0..3 {
        // 1 mm/s * 1 ms = 1 micrometre (1e-6 m).
        let total_um = i128::from(pos_rem_um[axis]) + i128::from(vel_mmps[axis]) * dt;
        let dm = (total_um / 1_000_000) as i64;
        let rem = (total_um % 1_000_000) as i64;
        pos_m[axis] = pos_m[axis].saturating_add(dm);
        pos_rem_um[axis] = rem;
    }
}

fn vector_to_bearing_pitch(v: [i64; 3]) -> (f64, f64) {
    let x = v[0] as f64;
    let y = v[1] as f64;
    let z = v[2] as f64;
    let horiz = (x * x + y * y).sqrt();
    if horiz < 1e-6 && z.abs() < 1e-6 {
        return (0.0, 0.0);
    }
    let bearing = y.atan2(x).to_degrees().rem_euclid(360.0);
    let pitch = z.atan2(horiz.max(1e-6)).to_degrees();
    (bearing, pitch)
}
