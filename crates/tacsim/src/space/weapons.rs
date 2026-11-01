//! **The 1:1 wall-time space weapon triad** (`docs/design/35-ship-free-flight-combat-and-mining.md`
//! §6.2, `D-35-8`, `Q-35-4`).
//!
//! All combat inside a [`super::bubble::LocalSpaceBubble`] runs on **1:1 wall time**:
//! - **Torpedoes (`TorpedoTube`)**: Inherit launcher velocity $\mathbf{v}_{\text{ship}}$ at launch,
//!   accelerate at $15\text{–}30\text{ g}$ ($150\text{–}300\text{ m/s}^2$) under proportional
//!   navigation guidance with finite $\Delta v$ ($8\text{–}15\text{ km/s}$), $100\text{–}2{,}500\text{ km}$
//!   effective range ($12\text{–}45\text{ s}$ wall-time flight), bounded by tube count and
//!   magazine stowage (`W15`). Countered by automatic **PDCs** (`+tac/pdc auto`), **Defensive
//!   Lasers**, **ECM/decoys**, or high-G burns that exhaust torpedo $\Delta v$.
//! - **Energy Weapons (`LaserMount`)**: Instant hit at speed of light ($c$), $20\text{–}500\text{ km}$
//!   range with diffraction falloff $1 / (1 + (R/R_0)^2)$, builds hull `heat_permille`, and can
//!   be toggled per mount between [`LaserMode::Offense`] and [`LaserMode::Defense`] (`D-35-8`).
//! - **Railguns (`RailgunMount`)**: $10\text{–}250\text{ km}$ range, unguided tungsten slug at
//!   $v_{\text{muz}} = 15\text{–}20\text{ km/s}$ + $\mathbf{v}_{\text{ship}}$ ($1.0\text{–}15\text{ s}$
//!   wall-time time-of-flight). **Cannot be intercepted by PDCs or lasers.** Countered by
//!   executing a **lateral jinking burn** during time-of-flight outside the shooter's predicted
//!   lead pip.
//! - **Point-Defense Cannons (`PdcMount`)**: $0\text{–}15\text{ km}$ rotary burst envelope;
//!   automatically engages incoming hostile torpedoes when `pdc_auto` is toggled on (`Q-35-4`).

use serde::{Deserialize, Serialize};

/// Operating mode for a shipboard laser mount (`D-35-8`, `Q-35-4`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaserMode {
    /// Fires at the locked hostile ship/target on command.
    #[default]
    Offense,
    /// Automatically intercepts incoming hostile torpedoes within defensive range.
    Defense,
}

/// Torpedo launcher configuration and magazine state (`W15`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TorpedoTube {
    pub id: String,
    /// Number of simultaneous tubes that can fire in one salvo.
    pub tubes: u8,
    /// Remaining torpedoes in the magazine (`W15`).
    pub magazine_remaining: u16,
    /// Drive acceleration of launched torpedoes in $\text{m/s}^2$ (e.g., $200\text{ m/s}^2 \approx 20\text{ g}$).
    pub accel_mps2: i64,
    /// Total $\Delta v$ budget of each torpedo in $\text{m/s}$ (e.g., $12{,}000\text{ m/s}$).
    pub max_dv_mps: i64,
    /// Raw warhead damage on impact.
    pub warhead_damage: u32,
    /// Minimum reload time between salvos in wall milliseconds.
    pub reload_cooldown_ms: u32,
    /// Remaining reload cooldown in wall milliseconds.
    pub cooldown_remaining_ms: u32,
}

impl TorpedoTube {
    /// Standard corvette / raider 2-tube torpedo launcher ($25\text{ g}$, $12\text{ km/s}$ $\Delta v$).
    #[must_use]
    pub fn standard_twin(id: impl Into<String>, magazine: u16) -> Self {
        Self {
            id: id.into(),
            tubes: 2,
            magazine_remaining: magazine,
            accel_mps2: 250,
            max_dv_mps: 12_000,
            warhead_damage: 180,
            reload_cooldown_ms: 8_000,
            cooldown_remaining_ms: 0,
        }
    }
}

/// Shipboard laser / pulsed-beam mount (`D-35-8`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaserMount {
    pub id: String,
    pub mode: LaserMode,
    /// Damage at zero range before diffraction falloff.
    pub base_damage: u32,
    /// Half-intensity diffraction range $R_0$ in metres (e.g., $120{,}000\text{ m}$).
    pub half_range_m: i64,
    /// Maximum effective range in metres (e.g., $500{,}000\text{ m}$).
    pub max_range_m: i64,
    /// Defensive anti-torpedo intercept range in metres when in [`LaserMode::Defense`].
    pub defense_range_m: i64,
    /// Hull heat added per shot in permille (`0..=1000`).
    pub heat_cost_permille: u16,
    /// Cooldown between pulses in wall milliseconds.
    pub cooldown_ms: u32,
    pub cooldown_remaining_ms: u32,
}

impl LaserMount {
    /// Standard dual-mode tactical laser turret ($20\text{–}500\text{ km}$ range).
    #[must_use]
    pub fn standard_turret(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            mode: LaserMode::Offense,
            base_damage: 55,
            half_range_m: 120_000,
            max_range_m: 500_000,
            defense_range_m: 45_000,
            heat_cost_permille: 140,
            cooldown_ms: 1_000,
            cooldown_remaining_ms: 0,
        }
    }

    /// Compute diffraction-attenuated damage at `distance_m`:
    /// $D(R) = \lfloor D_0 \cdot R_0^2 / (R_0^2 + R^2) \rfloor$.
    #[must_use]
    pub fn damage_at_range(&self, distance_m: i64) -> u32 {
        if distance_m < 0 || distance_m > self.max_range_m {
            return 0;
        }
        let r_km = (distance_m / 1_000).max(0) as i128;
        let r0_km = (self.half_range_m / 1_000).max(1) as i128;
        let num = (self.base_damage as i128) * r0_km * r0_km;
        let den = r0_km * r0_km + r_km * r_km;
        if den <= 0 {
            return self.base_damage;
        }
        (num / den).clamp(1, self.base_damage as i128) as u32
    }
}

/// High-velocity unguided electromagnetic railgun mount (`D-35-8`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RailgunMount {
    pub id: String,
    /// Muzzle velocity $v_{\text{muz}}$ in $\text{m/s}$ ($15{,}000\text{–}20{,}000\text{ m/s}$).
    pub muzzle_vel_mps: i64,
    /// Maximum effective engagement range in metres ($250{,}000\text{ m}$).
    pub max_range_m: i64,
    /// Kinetic slug damage on hit.
    pub slug_damage: u32,
    /// Remaining tungsten slugs in magazine (`W15`).
    pub rounds_remaining: u16,
    /// Capacitor / thermal spike per shot in permille.
    pub heat_cost_permille: u16,
    /// Capacitor recharge time in wall milliseconds.
    pub cooldown_ms: u32,
    pub cooldown_remaining_ms: u32,
}

impl RailgunMount {
    /// Standard $20\text{ km/s}$ spinal railgun ($10\text{–}250\text{ km}$ range).
    #[must_use]
    pub fn standard_spinal(id: impl Into<String>, rounds: u16) -> Self {
        Self {
            id: id.into(),
            muzzle_vel_mps: 20_000,
            max_range_m: 250_000,
            slug_damage: 145,
            rounds_remaining: rounds,
            heat_cost_permille: 220,
            cooldown_ms: 3_000,
            cooldown_remaining_ms: 0,
        }
    }
}

/// Rotary Point-Defense Cannon (PDC / CIWS) mount (`D-35-8`, `Q-35-4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdcMount {
    pub id: String,
    /// Terminal anti-torpedo interception radius in metres (e.g., $15{,}000\text{ m}$).
    pub envelope_m: i64,
    /// Rounds expended per intercept burst.
    pub burst_rounds: u16,
    /// Remaining rounds in drum (`W15`).
    pub ammo_remaining: u32,
    /// Whether this mount is operational (false if knocked out by a subsystem crit).
    pub operational: bool,
}

impl PdcMount {
    /// Standard $15\text{ km}$ rotary PDC mount with 2,000 rounds.
    #[must_use]
    pub fn standard_rotary(id: impl Into<String>, ammo: u32) -> Self {
        Self {
            id: id.into(),
            envelope_m: 15_000,
            burst_rounds: 50,
            ammo_remaining: ammo,
            operational: true,
        }
    }
}

/// A live guided torpedo in flight inside a [`super::bubble::LocalSpaceBubble`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InFlightTorpedo {
    pub id: u64,
    pub shooter_id: String,
    pub target_id: String,
    /// Current 3D local position in metres (`[x, y, z]`).
    pub pos_m: [i64; 3],
    /// Current 3D local velocity in $\text{mm/s}$ (`[vx, vy, vz]`).
    pub vel_mmps: [i64; 3],
    /// Guidance acceleration capability in $\text{m/s}^2$.
    pub accel_mps2: i64,
    /// Remaining $\Delta v$ budget in $\text{mm/s}$.
    pub remaining_dv_mmps: i64,
    /// Warhead damage on impact.
    pub warhead_damage: u32,
    /// Estimated time-to-impact in whole wall seconds (`TTI`).
    pub tti_s: i64,
}

/// A live unguided railgun tungsten slug in flight (`D-35-8`).
///
/// Railgun slugs **cannot be intercepted by PDCs or defensive lasers**.
/// Instead, at firing time the slug is launched toward the target's predicted
/// linear lead position $\mathbf{r}_{\text{lead}}$. If the target executes a
/// lateral jink burn during the slug's $1.0\text{–}15\text{ s}$ time-of-flight
/// so that its position at arrival deviates from the slug's trajectory by more
/// than `hit_radius_m`, the slug misses (`DodgedByJink`)!
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InFlightSlug {
    pub id: u64,
    pub shooter_id: String,
    pub target_id: String,
    /// Current 3D local position in metres.
    pub pos_m: [i64; 3],
    /// Constant 3D ballistic velocity in $\text{mm/s}$ ($\mathbf{v}_{\text{ship}} + \mathbf{v}_{\text{muz}}$).
    pub vel_mmps: [i64; 3],
    /// Predicted target 3D position at impact time when the shot was fired.
    pub aimed_lead_pos_m: [i64; 3],
    /// Remaining flight time in wall milliseconds until closest approach.
    pub remaining_flight_ms: u32,
    /// Effective hit envelope in metres around the slug's line of flight.
    pub hit_radius_m: i64,
    /// Kinetic damage if the target fails to jink out of the trajectory.
    pub slug_damage: u32,
}

/// Compute the 3D railgun lead point $\mathbf{r}_{\text{lead}}$ in metres and
/// time-of-flight in wall milliseconds for a target at `target_pos_m` with
/// velocity `target_vel_mmps` fired from `shooter_pos_m` with `shooter_vel_mmps`
/// and muzzle speed `muzzle_vel_mps`.
#[must_use]
pub fn compute_railgun_lead(
    shooter_pos_m: [i64; 3],
    shooter_vel_mmps: [i64; 3],
    target_pos_m: [i64; 3],
    target_vel_mmps: [i64; 3],
    muzzle_vel_mps: i64,
) -> ([i64; 3], u32) {
    let dx = (target_pos_m[0] - shooter_pos_m[0]) as i128;
    let dy = (target_pos_m[1] - shooter_pos_m[1]) as i128;
    let dz = (target_pos_m[2] - shooter_pos_m[2]) as i128;
    let dist_m = isqrt_i128(dx * dx + dy * dy + dz * dz).max(1);

    let rvx_mmps = (target_vel_mmps[0] - shooter_vel_mmps[0]) as i128;
    let rvy_mmps = (target_vel_mmps[1] - shooter_vel_mmps[1]) as i128;
    let rvz_mmps = (target_vel_mmps[2] - shooter_vel_mmps[2]) as i128;

    // Two-pass intercept time-of-flight in milliseconds:
    let muz_mmps = (muzzle_vel_mps.max(1_000) as i128) * 1_000;
    let mut tof_ms = ((dist_m * 1_000_000) / muz_mmps).clamp(100, 60_000);

    for _ in 0..2 {
        let lx = dx + (rvx_mmps * tof_ms) / 1_000_000;
        let ly = dy + (rvy_mmps * tof_ms) / 1_000_000;
        let lz = dz + (rvz_mmps * tof_ms) / 1_000_000;
        let lead_dist_m = isqrt_i128(lx * lx + ly * ly + lz * lz).max(1);
        tof_ms = ((lead_dist_m * 1_000_000) / muz_mmps).clamp(100, 60_000);
    }

    let lead_pos_m = [
        shooter_pos_m[0].saturating_add((dx + (rvx_mmps * tof_ms) / 1_000_000) as i64),
        shooter_pos_m[1].saturating_add((dy + (rvy_mmps * tof_ms) / 1_000_000) as i64),
        shooter_pos_m[2].saturating_add((dz + (rvz_mmps * tof_ms) / 1_000_000) as i64),
    ];
    (lead_pos_m, tof_ms as u32)
}

/// Deterministic integer square root for `i128` ($\lfloor \sqrt{n} \rfloor$).
#[must_use]
pub fn isqrt_i128(n: i128) -> i128 {
    if n <= 0 {
        return 0;
    }
    n.isqrt()
}
