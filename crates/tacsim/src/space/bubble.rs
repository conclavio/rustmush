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

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::space::weapons::{
    InFlightSlug, InFlightTorpedo, LaserMode, LaserMount, PdcMount, RailgunMount, TorpedoTube,
    compute_railgun_lead, isqrt_i128,
};

/// Maximum substep duration in wall milliseconds (`D-16-2`).
pub const MAX_SUBSTEP_MS: u32 = 250;

/// Default 48-hour open-epoch derelict life-support and salvage contest window in seconds (`D-35-12`).
pub const DERELICT_WINDOW_S: u32 = 48 * 3_600;

/// Identifier of a [`LocalSpaceBubble`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BubbleId(pub String);

/// Party identifier inside a multi-party space bubble (`D-35-1`, `D-35-11`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SpacePartyId(pub u32);

/// Standing relation between two parties in a [`LocalSpaceBubble`] (`D-35-11`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpaceRelation {
    Friendly,
    #[default]
    Neutral,
    /// Customs picket or patrol actively inspecting or screening a vessel.
    Inspecting,
    Hostile,
}

impl SpaceRelation {
    #[must_use]
    pub fn as_feed_str(self) -> &'static str {
        match self {
            Self::Friendly => "friend",
            Self::Neutral => "neutral",
            Self::Inspecting => "neutral",
            Self::Hostile => "hostile",
        }
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

/// Armour and structure state for one section of a ship (`D-35-1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipSectionState {
    pub id: ShipSectionId,
    pub facets: BTreeMap<FacetBand, u32>,
    pub internals: u32,
    pub max_internals: u32,
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

/// A ship or hull inside a [`LocalSpaceBubble`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceShipState {
    pub id: String,
    pub callsign: String,
    pub hull_class: String,
    pub party: SpacePartyId,
    /// 3D local position in metres (`[x, y, z]`).
    pub pos_m: [i64; 3],
    /// 3D local velocity in $\text{mm/s}$ (`[vx, vy, vz]`).
    pub vel_mmps: [i64; 3],
    /// Commanded 3D thrust acceleration in $\text{m/s}^2$ (`[ax, ay, az]`).
    pub thrust_mps2: [i64; 3],
    /// Maximum rated drive acceleration in $\text{m/s}^2$.
    pub max_accel_mps2: i64,
    /// Hull attitude `(yaw_deg, pitch_deg, roll_deg)` in whole degrees.
    pub attitude_deg: (i32, i32, i32),
    /// Remaining reaction mass in `kg` (`D-35-6`).
    pub propellant_kg: u32,
    /// Remaining $\Delta v$ reserve in $\text{m/s}$.
    pub delta_v_mps: i64,
    /// Minimum $\Delta v$ needed to return to the nearest open port (`RETURN_RESERVE`, `D-35-6`).
    pub return_reserve_dv_mps: i64,
    /// Hull heat cell in permille (`0..=1000`, `D-35-1`).
    pub heat_permille: u16,
    /// Base radiator cooling rate in permille per wall second.
    pub cooling_rate_permille_s: u16,
    /// Whether automatic point-defense interception (`+tac/pdc auto`) is enabled (`Q-35-4`).
    pub pdc_auto: bool,
    /// Whether all crew are currently linkdead (`D-35-7` ballistic safety rule).
    pub crew_linkdead: bool,
    pub emcon: EmconPosture,
    pub locked_target_id: Option<String>,
    /// Effective hull cross-section radius for unguided projectile impacts in metres.
    pub hit_radius_m: i64,
    pub sections: BTreeMap<ShipSectionId, ShipSectionState>,
    pub crits: Vec<SubsystemCrit>,
    pub status: HullStatus,
    pub torpedo_tubes: Vec<TorpedoTube>,
    pub lasers: Vec<LaserMount>,
    pub railguns: Vec<RailgunMount>,
    pub pdcs: Vec<PdcMount>,
}

impl SpaceShipState {
    /// Construct a `Dray` / `Dray-P` bulk/prospector hull with the exact section and
    /// facet armour values from `crates/tacsim/content/units/dray.json` (`D-35-1`).
    #[must_use]
    pub fn new_dray(
        id: impl Into<String>,
        callsign: impl Into<String>,
        party: SpacePartyId,
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
            },
        );
        sections.insert(
            ShipSectionId::Drive,
            ShipSectionState {
                id: ShipSectionId::Drive,
                facets: BTreeMap::new(),
                internals: 250,
                max_internals: 250,
            },
        );

        Self {
            id: id.into(),
            callsign: callsign.into(),
            hull_class: "tacsim.unit.dray".to_owned(),
            party,
            pos_m,
            vel_mmps,
            thrust_mps2: [0, 0, 0],
            max_accel_mps2: 25,
            attitude_deg: (0, 0, 0),
            propellant_kg: 80_000,
            delta_v_mps: 4_500,
            return_reserve_dv_mps: 900,
            heat_permille: 80,
            cooling_rate_permille_s: 30,
            pdc_auto: true,
            crew_linkdead: false,
            emcon: EmconPosture::Standard,
            locked_target_id: None,
            hit_radius_m: 60,
            sections,
            crits: Vec::new(),
            status: HullStatus::Operational,
            torpedo_tubes: Vec::new(),
            lasers: vec![LaserMount::standard_turret("laser_dorsal")],
            railguns: Vec::new(),
            pdcs: vec![PdcMount::standard_rotary("pdc_ dorsal", 2_000)],
        }
    }

    /// Construct an armed corvette / raider hull with torpedo tubes, spinal railgun,
    /// dual-mode laser turret, and rotary PDCs.
    #[must_use]
    pub fn new_corvette(
        id: impl Into<String>,
        callsign: impl Into<String>,
        party: SpacePartyId,
        pos_m: [i64; 3],
        vel_mmps: [i64; 3],
    ) -> Self {
        let mut ship = Self::new_dray(id, callsign, party, pos_m, vel_mmps);
        ship.hull_class = "tacsim.unit.corvette".to_owned();
        ship.max_accel_mps2 = 60;
        ship.delta_v_mps = 9_500;
        ship.torpedo_tubes = vec![TorpedoTube::standard_twin("torp_bow", 8)];
        ship.lasers = vec![
            LaserMount::standard_turret("laser_1"),
            LaserMount::standard_turret("laser_2"),
        ];
        ship.railguns = vec![RailgunMount::standard_spinal("rail_spinal", 24)];
        ship.pdcs = vec![
            PdcMount::standard_rotary("pdc_port", 3_000),
            PdcMount::standard_rotary("pdc_stbd", 3_000),
        ];
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
            && (self.thrust_mps2[0] != 0 || self.thrust_mps2[1] != 0 || self.thrust_mps2[2] != 0)
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

    /// Apply raw weapon `damage` arriving on `facet` (`D-35-1`, `D-35-12`).
    ///
    /// Follows `dray.json`'s section mapping:
    /// - `Front` $\to$ `Hull`
    /// - `Left` / `Right` / `Top` $\to$ `Hull` and `Hold`
    /// - `Rear` $\to$ `Hull` with `struck_through` to `Drive`
    pub fn apply_damage(&mut self, facet: FacetBand, damage: u32) -> (ShipSectionId, u32, u32) {
        let primary_section = match facet {
            FacetBand::Front => ShipSectionId::Hull,
            FacetBand::Rear => ShipSectionId::Hull,
            FacetBand::Left | FacetBand::Right | FacetBand::Top => ShipSectionId::Hull,
        };

        let mut remaining = damage;
        let mut armor_absorbed = 0u32;

        if let Some(sec) = self.sections.get_mut(&primary_section)
            && let Some(armor) = sec.facets.get_mut(&facet)
        {
            let absorbed = (*armor).min(remaining);
            *armor -= absorbed;
            armor_absorbed = absorbed;
            remaining -= absorbed;
        }

        // In `dray.json`, `drive` has `struck_through: { section: "hull", facet: "rear" }`
        // and `rear` band splits 50/50 between `hull` and `drive`.
        let target_section = if facet == FacetBand::Rear && remaining > 0 {
            ShipSectionId::Drive
        } else {
            primary_section
        };

        let mut internal_dmg = 0u32;
        if remaining > 0
            && let Some(sec) = self.sections.get_mut(&target_section)
        {
            let dealt = sec.internals.min(remaining);
            sec.internals -= dealt;
            internal_dmg = dealt;
        }

        // Subsystem crit check when internals take heavy damage
        if internal_dmg >= 40
            && target_section == ShipSectionId::Hull
            && !self.crits.contains(&SubsystemCrit::RadiatorDamaged)
        {
            self.crits.push(SubsystemCrit::RadiatorDamaged);
        }
        if internal_dmg >= 30
            && target_section == ShipSectionId::Drive
            && !self.crits.contains(&SubsystemCrit::DriveDegraded)
        {
            self.crits.push(SubsystemCrit::DriveDegraded);
        }

        // Open-epoch derelict disablement check (`D-35-12`, `FO-12`):
        // Never permadeath-delete the hull; transition to DisabledDerelict with epoch_open = true.
        let drive_zero = self
            .sections
            .get(&ShipSectionId::Drive)
            .is_some_and(|s| s.internals == 0);
        let hull_zero = self
            .sections
            .get(&ShipSectionId::Hull)
            .is_some_and(|s| s.internals == 0);

        if (drive_zero || hull_zero) && self.status == HullStatus::Operational {
            self.status = HullStatus::DisabledDerelict {
                epoch_open: true,
                life_support_remaining_s: DERELICT_WINDOW_S,
                salvage_contest_window_s: DERELICT_WINDOW_S,
            };
            self.thrust_mps2 = [0, 0, 0];
        }

        (target_section, armor_absorbed, internal_dmg)
    }
}

/// Discrete combat & telemetry events emitted during a [`LocalSpaceBubble::step_wall_ms`] call.
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
}

/// Summary returned by [`LocalSpaceBubble::step_wall_ms`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BubbleStepReport {
    pub wall_advanced_ms: u32,
    pub substeps_executed: u32,
    pub events: Vec<SpaceCombatEvent>,
    pub state_hash: u64,
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
    pub outage_frozen: bool,
    pub ships: BTreeMap<String, SpaceShipState>,
    pub relations: BTreeMap<(SpacePartyId, SpacePartyId), SpaceRelation>,
    pub torpedoes: Vec<InFlightTorpedo>,
    pub railgun_slugs: Vec<InFlightSlug>,
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
        Self {
            id: BubbleId(id.into()),
            site_id: site_id.into(),
            title: title.into(),
            wall_elapsed_ms: 0,
            ic_timestamp_s,
            outage_frozen: false,
            ships: BTreeMap::new(),
            relations: BTreeMap::new(),
            torpedoes: Vec::new(),
            railgun_slugs: Vec::new(),
            next_ordnance_id: 1,
        }
    }

    /// Insert or update a ship in the bubble.
    pub fn insert_ship(&mut self, ship: SpaceShipState) {
        self.ships.insert(ship.id.clone(), ship);
    }

    /// Set the symmetric standing relation between `party_a` and `party_b`.
    pub fn set_party_relation(
        &mut self,
        party_a: SpacePartyId,
        party_b: SpacePartyId,
        relation: SpaceRelation,
    ) {
        self.relations.insert((party_a, party_b), relation);
        self.relations.insert((party_b, party_a), relation);
    }

    /// Query the standing relation between two parties.
    #[must_use]
    pub fn party_relation(&self, party_a: SpacePartyId, party_b: SpacePartyId) -> SpaceRelation {
        if party_a == party_b {
            return SpaceRelation::Friendly;
        }
        self.relations
            .get(&(party_a, party_b))
            .copied()
            .unwrap_or(SpaceRelation::Neutral)
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
            return Ok(());
        }
        let requested_mag = isqrt_i128(
            (thrust_mps2[0] as i128).pow(2)
                + (thrust_mps2[1] as i128).pow(2)
                + (thrust_mps2[2] as i128).pow(2),
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
        if requested_mag > max_a && requested_mag > 0 {
            ship.thrust_mps2 = [
                (thrust_mps2[0] * max_a) / requested_mag,
                (thrust_mps2[1] * max_a) / requested_mag,
                (thrust_mps2[2] * max_a) / requested_mag,
            ];
        } else {
            ship.thrust_mps2 = thrust_mps2;
        }
        Ok(())
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
        if shooter.status != HullStatus::Operational {
            return events;
        }
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
                vel_mmps: shooter_vel,
                accel_mps2: accel,
                remaining_dv_mmps: max_dv_mmps,
                warhead_damage: dmg,
                tti_s: est_tti_s,
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
        if shooter.status != HullStatus::Operational {
            return events;
        }
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
        if shooter.status != HullStatus::Operational {
            return events;
        }
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

    /// Advance the bubble on **1:1 wall time** by `dt_wall_ms` (`D-35-3`, `Q-CF-32`, `D-16-2`).
    ///
    /// Substeps in chunks of at most [`MAX_SUBSTEP_MS`] ($250\text{ ms}$).
    pub fn step_wall_ms(&mut self, dt_wall_ms: u32) -> BubbleStepReport {
        if self.outage_frozen || dt_wall_ms == 0 {
            return BubbleStepReport {
                wall_advanced_ms: 0,
                substeps_executed: 0,
                events: Vec::new(),
                state_hash: self.state_hash(),
            };
        }

        let mut remaining_ms = dt_wall_ms;
        let mut substeps = 0u32;
        let mut events = Vec::new();

        while remaining_ms > 0 {
            let step_ms = remaining_ms.min(MAX_SUBSTEP_MS);
            remaining_ms -= step_ms;
            substeps += 1;
            self.wall_elapsed_ms = self.wall_elapsed_ms.saturating_add(u64::from(step_ms));
            if self.wall_elapsed_ms.is_multiple_of(1_000) {
                self.ic_timestamp_s = self.ic_timestamp_s.saturating_add(1);
            }
            self.step_substep(step_ms, &mut events);
        }

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
            }

            let ax = ship.thrust_mps2[0];
            let ay = ship.thrust_mps2[1];
            let az = ship.thrust_mps2[2];
            let accel_mag =
                isqrt_i128((ax as i128).pow(2) + (ay as i128).pow(2) + (az as i128).pow(2)) as i64;

            if accel_mag > 0 && ship.propellant_kg > 0 && ship.delta_v_mps > 0 {
                // dv_mmps = a_mps2 * step_ms
                ship.vel_mmps[0] = ship.vel_mmps[0].saturating_add(ax * step_i64);
                ship.vel_mmps[1] = ship.vel_mmps[1].saturating_add(ay * step_i64);
                ship.vel_mmps[2] = ship.vel_mmps[2].saturating_add(az * step_i64);

                let dv_spent_mps = ((accel_mag * step_i64) / 1_000).max(0);
                ship.delta_v_mps = (ship.delta_v_mps - dv_spent_mps).max(0);
                let fuel_burned = ((accel_mag as u32).saturating_mul(step_ms) / 2_500).max(1);
                ship.propellant_kg = ship.propellant_kg.saturating_sub(fuel_burned);
                if ship.propellant_kg == 0 || ship.delta_v_mps == 0 {
                    ship.thrust_mps2 = [0, 0, 0];
                }
            }

            // Integrate position: dx_m = (vel_mmps * step_ms) / 1_000_000
            for axis in 0..3 {
                let dpos = (ship.vel_mmps[axis] as i128 * i128::from(step_ms)) / 1_000_000;
                ship.pos_m[axis] = ship.pos_m[axis].saturating_add(dpos as i64);
            }

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

            let eff_cooling = if ship.crits.contains(&SubsystemCrit::RadiatorDamaged) {
                ship.cooling_rate_permille_s / 2
            } else {
                ship.cooling_rate_permille_s
            };
            let cooled = ((u32::from(eff_cooling) * step_ms) / 1_000) as u16;
            ship.heat_permille = ship.heat_permille.saturating_sub(cooled);
        }

        // 2. Step guided torpedoes (`D-35-8`): proportional navigation + defensive laser / PDC screen
        let old_torpedoes = std::mem::take(&mut self.torpedoes);
        let mut surviving_torpedoes = Vec::with_capacity(old_torpedoes.len());
        for mut torp in old_torpedoes {
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

            // Proportional guidance burn toward intercept point if delta-v remains
            let step_dv_mmps = torp.accel_mps2 * step_i64;
            if torp.remaining_dv_mmps > 0 {
                let burn_mmps = step_dv_mmps.min(torp.remaining_dv_mmps);
                torp.remaining_dv_mmps -= burn_mmps;

                // Steer toward relative closing vector
                let ux = (dx * 1_000) / dist_m;
                let uy = (dy * 1_000) / dist_m;
                let uz = (dz * 1_000) / dist_m;
                torp.vel_mmps[0] =
                    torp.vel_mmps[0].saturating_add(((ux * i128::from(burn_mmps)) / 1_000) as i64);
                torp.vel_mmps[1] =
                    torp.vel_mmps[1].saturating_add(((uy * i128::from(burn_mmps)) / 1_000) as i64);
                torp.vel_mmps[2] =
                    torp.vel_mmps[2].saturating_add(((uz * i128::from(burn_mmps)) / 1_000) as i64);
            } else {
                events.push(SpaceCombatEvent::TorpedoExhaustedDeltaV {
                    torpedo_id: torp.id,
                });
                continue;
            }

            // Advance torpedo position
            for axis in 0..3 {
                let dpos = (torp.vel_mmps[axis] as i128 * i128::from(step_ms)) / 1_000_000;
                torp.pos_m[axis] = torp.pos_m[axis].saturating_add(dpos as i64);
            }

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
            torp.tti_s = (new_dist_m / closing_mps).max(0);

            // Check automatic defensive laser & PDC interception across target and escorting ships
            let defender_ids: Vec<String> = self
                .ships
                .values()
                .filter(|s| {
                    s.id != torp.shooter_id
                        && s.status == HullStatus::Operational
                        && matches!(
                            self.party_relation(s.party, target_party),
                            SpaceRelation::Friendly | SpaceRelation::Inspecting
                        )
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

                // 2b. Automatic PDC screen (`+tac/pdc auto`)
                if def_ship.pdc_auto {
                    for pdc in &mut def_ship.pdcs {
                        if pdc.operational
                            && pdc.ammo_remaining >= u32::from(pdc.burst_rounds)
                            && def_dist_m <= pdc.envelope_m
                        {
                            pdc.ammo_remaining -= u32::from(pdc.burst_rounds);
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
            for axis in 0..3 {
                let dpos = (slug.vel_mmps[axis] as i128 * i128::from(dt)) / 1_000_000;
                slug.pos_m[axis] = slug.pos_m[axis].saturating_add(dpos as i64);
            }
            slug.remaining_flight_ms -= dt;

            if slug.remaining_flight_ms == 0 {
                if let Some(target_mut) = self.ships.get_mut(&slug.target_id) {
                    let jx = (target_mut.pos_m[0] - slug.aimed_lead_pos_m[0]) as i128;
                    let jy = (target_mut.pos_m[1] - slug.aimed_lead_pos_m[1]) as i128;
                    let jz = (target_mut.pos_m[2] - slug.aimed_lead_pos_m[2]) as i128;
                    let displacement_m = isqrt_i128(jx * jx + jy * jy + jz * jz) as i64;

                    if displacement_m > slug.hit_radius_m {
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
            for p in torp.pos_m {
                mix(&p.to_le_bytes());
            }
        }
        for slug in &self.railgun_slugs {
            mix(&slug.id.to_le_bytes());
            mix(&slug.remaining_flight_ms.to_le_bytes());
        }
        h
    }

    /// Build a `space.holotank` feed JSON keyframe from `observer_ship_id`'s perspective,
    /// ready for `mush_pixel::holotank::HolotankPicture`.
    #[must_use]
    pub fn holotank_feed(&self, observer_ship_id: &str) -> Value {
        let obs = self.ships.get(observer_ship_id);
        let obs_party = obs.map(|s| s.party).unwrap_or(SpacePartyId(0));
        let locked_id = obs.and_then(|s| s.locked_target_id.as_deref());
        let pdc_range_km = obs
            .and_then(|s| s.pdcs.first())
            .map(|p| (p.envelope_m as f64) / 1_000.0)
            .unwrap_or(15.0);

        let mut contacts = Vec::with_capacity(self.ships.len());
        for ship in self.ships.values() {
            let rel = self.party_relation(obs_party, ship.party).as_feed_str();
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
            let target_pos = self
                .ships
                .get(&torp.target_id)
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
            if ship.id == observer_ship_id {
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
                "kind": "ship",
                "relation": self.party_relation(obs.party, ship.party).as_feed_str(),
                "locked": locked,
                "burning": ship.is_burning(),
                "bearing_deg": bearing_deg,
                "pitch_deg": pitch_c,
                "lead_bearing_deg": lead_b,
                "lead_pitch_deg": lead_p,
            }));
        }

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
            "contacts": contacts,
        })
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
