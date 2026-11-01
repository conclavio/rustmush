//! **Tier-1 system free flight, mid-leg decoupling (`+helm/decouple`), live propellant debit,
//! and outer-system expeditions** (Doc 35 cut `SF.6`;
//! `docs/design/35-ship-free-flight-combat-and-mining.md` §6.1, §9 Story 2, `Q-35-2`, `D-35-5..10`).
//!
//! Tracks ships flying off-corridor on the `WorldTimeline` IC clock:
//! - **Mid-Leg Decoupling (`+helm/decouple`)**: Interpolates the ship's 3D
//!   heliocentric ecliptic J2000 position (`pos_m: [i128; 3]`) and velocity
//!   (`vel_mmps: [i64; 3]`) from the sky chord at `now_ic`, voids the frozen
//!   journey ticket (`D-18-46`), and credits unburned remaining propellant to
//!   the live tank (`D-35-6`).
//! - **Standing-Order Channels (`mush-sim`)**: Consumes 3D `Heading{x,y,z}`,
//!   `Thrust`, and `Emcon{0..=3}` on each IC step (`step_ic_seconds`), plus
//!   guidance modes (`Manual`, `Prograde`, `Retrograde`, `Intercept`,
//!   `MatchVelocity`).
//! - **Return-Reserve Guard & Re-Coupling (`+helm/autopilot <port>`)**:
//!   [`SpaceFlightRecord::plan_passage_from`] computes the $\Delta v$ and
//!   brachistochrone transit time from any 3D point/velocity back to a port,
//!   raising [`FlightError::TankReserveWarning`] before a burn dips into
//!   `RETURN_RESERVE` unless `force = true` (`+helm/burn/force`) or long-range
//!   drop tanks are fitted.

use mush_core::CanonicalId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PosKm;
use crate::sky::{LIGHT_M_PER_S, Sky};
use crate::space::bubble::{EmconPosture, SpacePartyId};

/// Guidance laws available to a free-flying ship on the Tier-1 IC clock (§6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlightGuidanceMode {
    /// Direct 3D attitude / `Heading{x,y,z}` and `Thrust` standing order.
    Manual,
    /// Align nose with current heliocentric velocity vector $\mathbf{v}$.
    Prograde,
    /// Align nose opposite current heliocentric velocity vector $-\mathbf{v}$.
    Retrograde,
    /// Continuous brachistochrone intercept toward `target_body`, flipping at midpoint when `flip_burn` is true.
    Intercept {
        target_body: CanonicalId,
        flip_burn: bool,
    },
    /// Null relative velocity with `target_body` upon arrival.
    MatchVelocity { target_body: CanonicalId },
}

/// Record of a filed journey ticket that was voided when the pilot executed `+helm/decouple` (`D-35-5`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecoupledLegTicket {
    pub origin_body: CanonicalId,
    pub dest_body: CanonicalId,
    pub depart_ic: i64,
    pub planned_arrive_ic: i64,
    pub decoupled_at_ic: i64,
    /// True once `+helm/decouple` voids the frozen ticket so `relay_metric` skips its cursor check.
    pub ticket_voided: bool,
    /// Propellant (`kg`) credited back to the ship's live tank from the unburned portion of the leg.
    pub refunded_propellant_kg: u32,
}

/// A passage plan computed from an arbitrary 3D heliocentric position and velocity
/// back to a destination port body (`plan_passage_from`, `D-35-5`, `D-35-6`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FromPointPassagePlan {
    pub dest_body: CanonicalId,
    pub depart_ic: i64,
    pub arrive_ic: i64,
    pub distance_km: u64,
    pub required_dv_mps: i64,
    pub required_propellant_kg: u32,
    pub within_current_dv: bool,
    pub leaves_return_reserve: bool,
}

/// A newly filed passage ticket created when a free-flying ship executes
/// `+helm/autopilot <port>` (`recouple_autopilot`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoupledJourneyTicket {
    pub ship_id: String,
    pub dest_body: CanonicalId,
    pub depart_ic: i64,
    pub arrive_ic: i64,
    pub distance_km: u64,
    pub debited_dv_mps: i64,
    pub debited_propellant_kg: u32,
}

/// Errors returned by Tier-1 free-flight commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlightError {
    /// Body was not found in the active [`Sky`].
    UnknownBody { body: String },
    /// Burn would drop `delta_v_mps` below `return_reserve_dv_mps` without `force = true`.
    TankReserveWarning {
        requested_dv_mps: i64,
        available_dv_mps: i64,
        return_reserve_dv_mps: i64,
    },
    /// Insufficient $\Delta v$ to complete the requested passage or recouple to autopilot.
    InsufficientDeltaV {
        required_dv_mps: i64,
        available_dv_mps: i64,
    },
    /// Cannot recouple to autopilot while engaged in a hostile combat bubble.
    EngagedInCombat,
}

impl std::fmt::Display for FlightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownBody { body } => write!(f, "unknown sky body '{body}'"),
            Self::TankReserveWarning {
                requested_dv_mps,
                available_dv_mps,
                return_reserve_dv_mps,
            } => write!(
                f,
                "TANK_RESERVE_WARN: burn requires {requested_dv_mps} m/s (available {available_dv_mps} m/s, return reserve {return_reserve_dv_mps} m/s); use +helm/burn/force to override"
            ),
            Self::InsufficientDeltaV {
                required_dv_mps,
                available_dv_mps,
            } => write!(
                f,
                "insufficient delta-v: need {required_dv_mps} m/s, have {available_dv_mps} m/s"
            ),
            Self::EngagedInCombat => {
                write!(f, "cannot engage filed autopilot while engaged in combat")
            }
        }
    }
}

impl std::error::Error for FlightError {}

/// Tier-1 free-flight state for a spacecraft on the `WorldTimeline` IC clock (§6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceFlightRecord {
    pub ship_id: String,
    pub callsign: String,
    pub hull_class: String,
    pub party: SpacePartyId,
    /// Heliocentric ecliptic J2000 position in metres (`[x_m, y_m, z_m]`).
    pub pos_m: [i128; 3],
    /// Heliocentric ecliptic J2000 velocity in mm/s (`[vx_mmps, vy_mmps, vz_mmps]`).
    pub vel_mmps: [i64; 3],
    /// Hull attitude `(yaw_deg, pitch_deg, roll_deg)` in whole degrees.
    pub attitude_deg: (i32, i32, i32),
    /// Standing-order 3D unit heading vector in permille (`mush-sim` `Heading{x,y,z}`).
    pub heading_permille: [i32; 3],
    /// Commanded drive acceleration in $\text{m/s}^2$ (`mush-sim` `Thrust`).
    pub thrust_mps2: i64,
    /// Maximum rated drive acceleration in $\text{m/s}^2$.
    pub max_accel_mps2: i64,
    /// Emissions control posture (`mush-sim` `Emcon{0..=3}`).
    pub emcon: EmconPosture,
    /// Active guidance mode.
    pub guidance: FlightGuidanceMode,
    /// Remaining reaction mass in `kg` (`D-35-6`).
    pub propellant_kg: u32,
    /// Remaining $\Delta v$ budget in $\text{m/s}$.
    pub delta_v_mps: i64,
    /// Minimum $\Delta v$ reserved to reach the nearest open port (`RETURN_RESERVE`, `D-35-6`).
    pub return_reserve_dv_mps: i64,
    /// Whether external long-range expedition drop tanks are fitted.
    pub drop_tanks_fitted: bool,
    /// Current IC timestamp in seconds.
    pub ic_timestamp_s: i64,
    /// Voided journey ticket if this flight originated from `+helm/decouple`.
    pub decoupled_ticket: Option<DecoupledLegTicket>,
}

impl SpaceFlightRecord {
    /// Undock a ship manually (`+helm/undock`) from `port_body` at `now_ic` into
    /// Tier-1 system free flight, inheriting the port body's heliocentric
    /// position and orbital velocity (`Sky::position_km`, `Sky::velocity_mps`).
    pub fn undock_from_body(
        sky: &Sky,
        ship_id: impl Into<String>,
        callsign: impl Into<String>,
        hull_class: impl Into<String>,
        party: SpacePartyId,
        port_body: &CanonicalId,
        now_ic: i64,
    ) -> Result<Self, FlightError> {
        let pos_km =
            sky.position_km(port_body, now_ic)
                .ok_or_else(|| FlightError::UnknownBody {
                    body: port_body.as_str().to_owned(),
                })?;
        let vel_mps =
            sky.velocity_mps(port_body, now_ic)
                .ok_or_else(|| FlightError::UnknownBody {
                    body: port_body.as_str().to_owned(),
                })?;

        Ok(Self {
            ship_id: ship_id.into(),
            callsign: callsign.into(),
            hull_class: hull_class.into(),
            party,
            pos_m: [
                i128::from(pos_km.x) * 1_000,
                i128::from(pos_km.y) * 1_000,
                i128::from(pos_km.z) * 1_000,
            ],
            vel_mmps: [
                i64::from(vel_mps[0]) * 1_000,
                i64::from(vel_mps[1]) * 1_000,
                i64::from(vel_mps[2]) * 1_000,
            ],
            attitude_deg: (0, 0, 0),
            heading_permille: [1_000, 0, 0],
            thrust_mps2: 0,
            max_accel_mps2: 20,
            emcon: EmconPosture::Standard,
            guidance: FlightGuidanceMode::Manual,
            propellant_kg: 48_000,
            delta_v_mps: 24_000,
            return_reserve_dv_mps: 6_000,
            drop_tanks_fitted: false,
            ic_timestamp_s: now_ic,
            decoupled_ticket: None,
        })
    }

    /// Execute `+helm/decouple` mid-leg between `origin_body` and `dest_body` at
    /// `now_ic`, breaking the frozen journey ticket (`D-18-46` superseded by
    /// `D-35-5`), placing the hull at its interpolated 3D heliocentric position
    /// and velocity, and crediting the unburned fraction of `total_leg_propellant_kg`.
    #[allow(clippy::too_many_arguments)]
    pub fn decouple_from_leg(
        sky: &Sky,
        ship_id: impl Into<String>,
        callsign: impl Into<String>,
        hull_class: impl Into<String>,
        party: SpacePartyId,
        origin_body: &CanonicalId,
        dest_body: &CanonicalId,
        depart_ic: i64,
        planned_arrive_ic: i64,
        now_ic: i64,
        total_leg_propellant_kg: u32,
        base_reserve_propellant_kg: u32,
    ) -> Result<Self, FlightError> {
        let p0 =
            sky.position_km(origin_body, depart_ic)
                .ok_or_else(|| FlightError::UnknownBody {
                    body: origin_body.as_str().to_owned(),
                })?;
        let p1 = sky
            .position_km(dest_body, planned_arrive_ic)
            .ok_or_else(|| FlightError::UnknownBody {
                body: dest_body.as_str().to_owned(),
            })?;

        let span_s = (planned_arrive_ic - depart_ic).max(1);
        let elapsed_s = (now_ic - depart_ic).clamp(0, span_s);
        let frac_permille = i128::from((elapsed_s * 1_000) / span_s);

        let x_km =
            i128::from(p0.x) + ((i128::from(p1.x) - i128::from(p0.x)) * frac_permille) / 1_000;
        let y_km =
            i128::from(p0.y) + ((i128::from(p1.y) - i128::from(p0.y)) * frac_permille) / 1_000;
        let z_km =
            i128::from(p0.z) + ((i128::from(p1.z) - i128::from(p0.z)) * frac_permille) / 1_000;

        // Average chord velocity in mm/s along the leg at decoupling instant
        let vx_mmps =
            (((i128::from(p1.x) - i128::from(p0.x)) * 1_000_000) / i128::from(span_s)) as i64;
        let vy_mmps =
            (((i128::from(p1.y) - i128::from(p0.y)) * 1_000_000) / i128::from(span_s)) as i64;
        let vz_mmps =
            (((i128::from(p1.z) - i128::from(p0.z)) * 1_000_000) / i128::from(span_s)) as i64;

        let remaining_permille = (1_000 - frac_permille).clamp(0, 1_000) as u32;
        let refunded_propellant_kg =
            (total_leg_propellant_kg.saturating_mul(remaining_permille)) / 1_000;
        let live_propellant_kg = base_reserve_propellant_kg.saturating_add(refunded_propellant_kg);
        let live_delta_v_mps = i64::from(live_propellant_kg) / 2;

        Ok(Self {
            ship_id: ship_id.into(),
            callsign: callsign.into(),
            hull_class: hull_class.into(),
            party,
            pos_m: [x_km * 1_000, y_km * 1_000, z_km * 1_000],
            vel_mmps: [vx_mmps, vy_mmps, vz_mmps],
            attitude_deg: (0, 0, 0),
            heading_permille: [1_000, 0, 0],
            thrust_mps2: 0,
            max_accel_mps2: 20,
            emcon: EmconPosture::Standard,
            guidance: FlightGuidanceMode::Manual,
            propellant_kg: live_propellant_kg,
            delta_v_mps: live_delta_v_mps,
            return_reserve_dv_mps: 6_000,
            drop_tanks_fitted: false,
            ic_timestamp_s: now_ic,
            decoupled_ticket: Some(DecoupledLegTicket {
                origin_body: origin_body.clone(),
                dest_body: dest_body.clone(),
                depart_ic,
                planned_arrive_ic,
                decoupled_at_ic: now_ic,
                ticket_voided: true,
                refunded_propellant_kg,
            }),
        })
    }

    /// Fit long-range expedition drop tanks (Story 2 step 4), expanding
    /// `propellant_kg` and `delta_v_mps` for outer-system expeditions to Saturn,
    /// Uranus, and Neptune.
    pub fn fit_long_range_drop_tanks(&mut self) {
        if !self.drop_tanks_fitted {
            self.drop_tanks_fitted = true;
            self.propellant_kg = self.propellant_kg.saturating_add(160_000);
            self.delta_v_mps = self.delta_v_mps.saturating_add(95_000);
        }
    }

    /// Current heliocentric position rounded to whole kilometres (`PosKm`).
    #[must_use]
    pub const fn pos_km(&self) -> PosKm {
        PosKm {
            x: (self.pos_m[0] / 1_000) as i64,
            y: (self.pos_m[1] / 1_000) as i64,
            z: (self.pos_m[2] / 1_000) as i64,
        }
    }

    /// Distance in kilometres from the ship's current 3D position to `body` at `self.ic_timestamp_s`.
    #[must_use]
    pub fn distance_to_body_km(&self, sky: &Sky, body: &CanonicalId) -> Option<u64> {
        let target_pos = sky.position_km(body, self.ic_timestamp_s)?;
        Some(self.pos_km().distance_km(target_pos))
    }

    /// One-way light lag in whole seconds from the ship's current 3D position to `body` (`D-35-10`).
    #[must_use]
    pub fn light_lag_to_body_s(&self, sky: &Sky, body: &CanonicalId) -> Option<u64> {
        let km = self.distance_to_body_km(sky, body)?;
        Some((km * 1_000).div_ceil(LIGHT_M_PER_S))
    }

    /// Apply `mush-sim` standing-order channels (`Heading{x,y,z}`, `Thrust`, `Emcon{0..=3}`).
    pub fn apply_standing_orders(
        &mut self,
        heading_permille: [i32; 3],
        thrust_mps2: i64,
        emcon: EmconPosture,
    ) {
        let mag_sq = i64::from(heading_permille[0]) * i64::from(heading_permille[0])
            + i64::from(heading_permille[1]) * i64::from(heading_permille[1])
            + i64::from(heading_permille[2]) * i64::from(heading_permille[2]);
        if mag_sq > 0 {
            let mag = (mag_sq as f64).sqrt();
            self.heading_permille = [
                ((f64::from(heading_permille[0]) / mag) * 1_000.0).round() as i32,
                ((f64::from(heading_permille[1]) / mag) * 1_000.0).round() as i32,
                ((f64::from(heading_permille[2]) / mag) * 1_000.0).round() as i32,
            ];
            let yaw = (f64::from(self.heading_permille[1]))
                .atan2(f64::from(self.heading_permille[0]))
                .to_degrees()
                .rem_euclid(360.0)
                .round() as i32;
            let pitch = (f64::from(self.heading_permille[2]) / 1_000.0)
                .clamp(-1.0, 1.0)
                .asin()
                .to_degrees()
                .round() as i32;
            self.attitude_deg = (yaw, pitch, self.attitude_deg.2);
        }
        self.thrust_mps2 = thrust_mps2.clamp(0, self.max_accel_mps2);
        self.emcon = emcon;
    }

    /// Command a timed burn (`+helm/burn`) or intercept burn, checking the
    /// `RETURN_RESERVE` guard (`D-35-6`) unless `force = true` (`+helm/burn/force`).
    pub fn command_burn_dv(
        &mut self,
        heading_permille: [i32; 3],
        thrust_mps2: i64,
        duration_ic_s: u64,
        force: bool,
    ) -> Result<(), FlightError> {
        let clamped_thrust = thrust_mps2.clamp(0, self.max_accel_mps2);
        let requested_dv = clamped_thrust.saturating_mul(duration_ic_s as i64);
        if requested_dv > self.delta_v_mps {
            return Err(FlightError::InsufficientDeltaV {
                required_dv_mps: requested_dv,
                available_dv_mps: self.delta_v_mps,
            });
        }
        if !force && (self.delta_v_mps - requested_dv) < self.return_reserve_dv_mps {
            return Err(FlightError::TankReserveWarning {
                requested_dv_mps: requested_dv,
                available_dv_mps: self.delta_v_mps,
                return_reserve_dv_mps: self.return_reserve_dv_mps,
            });
        }
        self.apply_standing_orders(heading_permille, clamped_thrust, self.emcon);
        Ok(())
    }

    /// Step the ship's Tier-1 heliocentric trajectory forward by `dt_ic_s` IC
    /// seconds on the `WorldTimeline`, debiting live propellant (`D-35-6`) and
    /// steering toward any active [`FlightGuidanceMode`].
    pub fn step_ic_seconds(&mut self, sky: &Sky, dt_ic_s: u64) {
        if dt_ic_s == 0 {
            return;
        }
        let dt = dt_ic_s as i64;

        // Update steering vector if an automated guidance law is active
        match &self.guidance {
            FlightGuidanceMode::Manual => {}
            FlightGuidanceMode::Prograde => {
                let speed = vector_mag_i64(self.vel_mmps);
                if speed > 0 {
                    self.heading_permille = [
                        ((self.vel_mmps[0] * 1_000) / speed) as i32,
                        ((self.vel_mmps[1] * 1_000) / speed) as i32,
                        ((self.vel_mmps[2] * 1_000) / speed) as i32,
                    ];
                }
            }
            FlightGuidanceMode::Retrograde => {
                let speed = vector_mag_i64(self.vel_mmps);
                if speed > 0 {
                    self.heading_permille = [
                        ((-self.vel_mmps[0] * 1_000) / speed) as i32,
                        ((-self.vel_mmps[1] * 1_000) / speed) as i32,
                        ((-self.vel_mmps[2] * 1_000) / speed) as i32,
                    ];
                }
            }
            FlightGuidanceMode::Intercept {
                target_body,
                flip_burn,
            } => {
                if let Some(target_km) = sky.position_km(target_body, self.ic_timestamp_s + dt) {
                    let dx_m = i128::from(target_km.x) * 1_000 - self.pos_m[0];
                    let dy_m = i128::from(target_km.y) * 1_000 - self.pos_m[1];
                    let dz_m = i128::from(target_km.z) * 1_000 - self.pos_m[2];
                    let dist_m = ((dx_m as f64).hypot(dy_m as f64).hypot(dz_m as f64)).max(1.0);
                    let ux = ((dx_m as f64 / dist_m) * 1_000.0).round() as i32;
                    let uy = ((dy_m as f64 / dist_m) * 1_000.0).round() as i32;
                    let uz = ((dz_m as f64 / dist_m) * 1_000.0).round() as i32;

                    // Check stopping distance `v^2 / (2 * a)` for flip-and-burn deceleration
                    let speed_mps = (vector_mag_i64(self.vel_mmps) / 1_000).max(0);
                    let accel = self.thrust_mps2.max(1);
                    let stop_dist_m =
                        (i128::from(speed_mps) * i128::from(speed_mps)) / i128::from(2 * accel);
                    if *flip_burn && (dist_m as i128) <= stop_dist_m {
                        self.heading_permille = [-ux, -uy, -uz];
                    } else {
                        self.heading_permille = [ux, uy, uz];
                    }
                    self.apply_standing_orders(self.heading_permille, self.thrust_mps2, self.emcon);
                }
            }
            FlightGuidanceMode::MatchVelocity { target_body } => {
                if let Some(target_vel_mps) = sky.velocity_mps(target_body, self.ic_timestamp_s) {
                    let dv = [
                        i64::from(target_vel_mps[0]) * 1_000 - self.vel_mmps[0],
                        i64::from(target_vel_mps[1]) * 1_000 - self.vel_mmps[1],
                        i64::from(target_vel_mps[2]) * 1_000 - self.vel_mmps[2],
                    ];
                    let dmag = vector_mag_i64(dv);
                    if dmag <= 1_000 {
                        self.vel_mmps = [
                            i64::from(target_vel_mps[0]) * 1_000,
                            i64::from(target_vel_mps[1]) * 1_000,
                            i64::from(target_vel_mps[2]) * 1_000,
                        ];
                        self.thrust_mps2 = 0;
                    } else {
                        self.heading_permille = [
                            ((dv[0] * 1_000) / dmag) as i32,
                            ((dv[1] * 1_000) / dmag) as i32,
                            ((dv[2] * 1_000) / dmag) as i32,
                        ];
                    }
                }
            }
        }

        // Apply active thrust up to available delta_v_mps and debit live propellant (`D-35-6`)
        let requested_dv_mps = self.thrust_mps2.saturating_mul(dt);
        let actual_dv_mps = requested_dv_mps.min(self.delta_v_mps).max(0);
        self.delta_v_mps -= actual_dv_mps;
        let prop_burned_kg = (actual_dv_mps as u32).saturating_mul(2);
        self.propellant_kg = self.propellant_kg.saturating_sub(prop_burned_kg);
        if self.delta_v_mps == 0 {
            self.thrust_mps2 = 0;
        }

        let dv_mmps = actual_dv_mps * 1_000;
        let dvx = (dv_mmps * i64::from(self.heading_permille[0])) / 1_000;
        let dvy = (dv_mmps * i64::from(self.heading_permille[1])) / 1_000;
        let dvz = (dv_mmps * i64::from(self.heading_permille[2])) / 1_000;

        let v0 = self.vel_mmps;
        self.vel_mmps[0] = self.vel_mmps[0].saturating_add(dvx);
        self.vel_mmps[1] = self.vel_mmps[1].saturating_add(dvy);
        self.vel_mmps[2] = self.vel_mmps[2].saturating_add(dvz);

        // Trapezoidal position integration: `pos_m += ((v0 + v1) / 2) * dt / 1000`
        for ((pos, v_start), v_end) in self
            .pos_m
            .iter_mut()
            .zip(v0.iter())
            .zip(self.vel_mmps.iter())
        {
            let avg_vel_mmps = (i128::from(*v_start) + i128::from(*v_end)) / 2;
            let disp_m = (avg_vel_mmps * i128::from(dt)) / 1_000;
            *pos += disp_m;
        }

        self.ic_timestamp_s = self.ic_timestamp_s.saturating_add(dt);
    }

    /// Execute a high-energy outer-system expedition transfer to `dest_body`
    /// (e.g. `saturn`, `uranus`, `neptune`, `triton`), advancing the ship's
    /// heliocentric state along the brachistochrone curve, debiting $\Delta v$
    /// and propellant, and parking in a scenic flyby vantage `standoff_km` from
    /// the target body.
    pub fn fly_expedition_to_body(
        &mut self,
        sky: &Sky,
        dest_body: &CanonicalId,
        standoff_km: i64,
        force_reserve: bool,
    ) -> Result<FromPointPassagePlan, FlightError> {
        let plan = self.plan_passage_from(sky, dest_body)?;
        if !plan.within_current_dv {
            return Err(FlightError::InsufficientDeltaV {
                required_dv_mps: plan.required_dv_mps,
                available_dv_mps: self.delta_v_mps,
            });
        }
        if !force_reserve && !plan.leaves_return_reserve {
            return Err(FlightError::TankReserveWarning {
                requested_dv_mps: plan.required_dv_mps,
                available_dv_mps: self.delta_v_mps,
                return_reserve_dv_mps: self.return_reserve_dv_mps,
            });
        }

        let dest_pos_km =
            sky.position_km(dest_body, plan.arrive_ic)
                .ok_or_else(|| FlightError::UnknownBody {
                    body: dest_body.as_str().to_owned(),
                })?;
        let dest_vel_mps = sky.velocity_mps(dest_body, plan.arrive_ic).ok_or_else(|| {
            FlightError::UnknownBody {
                body: dest_body.as_str().to_owned(),
            }
        })?;

        self.delta_v_mps -= plan.required_dv_mps;
        self.propellant_kg = self
            .propellant_kg
            .saturating_sub(plan.required_propellant_kg);
        self.ic_timestamp_s = plan.arrive_ic;
        self.pos_m = [
            i128::from(dest_pos_km.x - standoff_km.max(1_000)) * 1_000,
            i128::from(dest_pos_km.y) * 1_000,
            i128::from(dest_pos_km.z) * 1_000,
        ];
        self.vel_mmps = [
            i64::from(dest_vel_mps[0]) * 1_000,
            i64::from(dest_vel_mps[1]) * 1_000,
            i64::from(dest_vel_mps[2]) * 1_000,
        ];
        self.attitude_deg = (0, 0, 0);
        self.heading_permille = [1_000, 0, 0];
        self.thrust_mps2 = 0;
        Ok(plan)
    }

    /// Plan a passage from the ship's current arbitrary 3D heliocentric position
    /// and velocity (`pos_m`, `vel_mmps`) to `dest_body` (`plan_passage_from`,
    /// `D-35-5`, `D-35-6`).
    pub fn plan_passage_from(
        &self,
        sky: &Sky,
        dest_body: &CanonicalId,
    ) -> Result<FromPointPassagePlan, FlightError> {
        let start_km = self.pos_km();
        let mut arrive_ic = self.ic_timestamp_s + 3_600;
        let mut dist_km = 0_u64;

        // 5-pass brachistochrone intercept refinement (`doc 18`, `D-35-5`)
        let accel_mps2 = self.max_accel_mps2.max(1) as f64;
        for _ in 0..5 {
            let target_km =
                sky.position_km(dest_body, arrive_ic)
                    .ok_or_else(|| FlightError::UnknownBody {
                        body: dest_body.as_str().to_owned(),
                    })?;
            dist_km = start_km.distance_km(target_km);
            let dist_m = (dist_km as f64) * 1_000.0;
            let transit_s = 2.0 * (dist_m / accel_mps2).sqrt();
            arrive_ic = self.ic_timestamp_s + (transit_s.round() as i64).max(600);
        }

        // Compute required delta-v (scaled for campaign corridor fuel economy + velocity match)
        let base_dv_mps = ((dist_km as f64).sqrt() * 0.85).round() as i64;
        let required_dv_mps = base_dv_mps.clamp(400, 75_000);
        let required_propellant_kg = (required_dv_mps as u32).saturating_mul(2);

        Ok(FromPointPassagePlan {
            dest_body: dest_body.clone(),
            depart_ic: self.ic_timestamp_s,
            arrive_ic,
            distance_km: dist_km,
            required_dv_mps,
            required_propellant_kg,
            within_current_dv: self.delta_v_mps >= required_dv_mps,
            leaves_return_reserve: (self.delta_v_mps - required_dv_mps)
                >= self.return_reserve_dv_mps,
        })
    }

    /// Execute `+helm/autopilot <port>` (`recouple_autopilot`, `D-35-5`):
    /// plans a passage from the ship's current 3D point back into a filed
    /// journey to `port_body`, verifying the ship is unengaged and has enough
    /// $\Delta v$.
    pub fn recouple_autopilot(
        &mut self,
        sky: &Sky,
        port_body: &CanonicalId,
        engaged_in_combat: bool,
    ) -> Result<RecoupledJourneyTicket, FlightError> {
        if engaged_in_combat {
            return Err(FlightError::EngagedInCombat);
        }
        let plan = self.plan_passage_from(sky, port_body)?;
        if !plan.within_current_dv {
            return Err(FlightError::InsufficientDeltaV {
                required_dv_mps: plan.required_dv_mps,
                available_dv_mps: self.delta_v_mps,
            });
        }
        self.delta_v_mps -= plan.required_dv_mps;
        self.propellant_kg = self
            .propellant_kg
            .saturating_sub(plan.required_propellant_kg);
        self.thrust_mps2 = 0;

        Ok(RecoupledJourneyTicket {
            ship_id: self.ship_id.clone(),
            dest_body: port_body.clone(),
            depart_ic: plan.depart_ic,
            arrive_ic: plan.arrive_ic,
            distance_km: plan.distance_km,
            debited_dv_mps: plan.required_dv_mps,
            debited_propellant_kg: plan.required_propellant_kg,
        })
    }

    /// Build a live `space.cockpit` JSON feed from the ship's 3D heliocentric
    /// position and attitude, projecting the specified visible celestial bodies
    /// (e.g. `neptune`, `triton`, `saturn`, `titan`) so the First-Person Space
    /// Bridge Canopy (`SpaceCockpitPicture`) renders them across the glass.
    #[must_use]
    pub fn canopy_feed(&self, sky: &Sky, visible_bodies: &[CanonicalId]) -> Value {
        let ship_km = self.pos_km();
        let mut body_entries = Vec::with_capacity(visible_bodies.len());
        for body_id in visible_bodies {
            let Some(bpos) = sky.position_km(body_id, self.ic_timestamp_s) else {
                continue;
            };
            let dx = (bpos.x - ship_km.x) as f64;
            let dy = (bpos.y - ship_km.y) as f64;
            let dz = (bpos.z - ship_km.z) as f64;
            let dist_km = (dx * dx + dy * dy + dz * dz).sqrt().max(1.0);
            let bearing_deg = dy.atan2(dx).to_degrees().rem_euclid(360.0);
            let pitch_deg = (dz / dist_km).clamp(-1.0, 1.0).asin().to_degrees();
            let radius_km = sky
                .outer_body(body_id)
                .map_or(2_500.0, |def| f64::from(def.radius_km));
            let ring_tilt_deg = match body_id.as_str() {
                "saturn" => 26.7,
                "uranus" => 97.8,
                "neptune" => 22.0,
                _ => 0.0,
            };
            body_entries.push(json!({
                "id": body_id.as_str(),
                "label": body_id.as_str().to_ascii_uppercase(),
                "bearing_deg": bearing_deg,
                "pitch_deg": pitch_deg,
                "distance_km": dist_km,
                "radius_km": radius_km,
                "spin_deg": ((self.ic_timestamp_s / 60) % 360) as f64,
                "ring_tilt_deg": ring_tilt_deg,
            }));
        }

        let speed_mps = vector_mag_i64(self.vel_mmps) / 1_000;
        json!({
            "callsign": self.callsign,
            "yaw_deg": self.attitude_deg.0,
            "pitch_deg": self.attitude_deg.1,
            "roll_deg": self.attitude_deg.2,
            "speed_mps": speed_mps,
            "delta_v_mps": self.delta_v_mps,
            "heat_permille": 120,
            "hull_pct": 100,
            "pdc_mode": "AUTO",
            "emcon": self.emcon.code_str(),
            "prograde_bearing_deg": self.attitude_deg.0,
            "prograde_pitch_deg": self.attitude_deg.1,
            "bodies": body_entries,
            "contacts": []
        })
    }
}

fn vector_mag_i64(v: [i64; 3]) -> i64 {
    let x = v[0] as f64;
    let y = v[1] as f64;
    let z = v[2] as f64;
    (x * x + y * y + z * z).sqrt().round() as i64
}
