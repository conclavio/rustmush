//! **Out-of-time-loop 6-DOF docking mini-game, port approach bonus, and derelict salvage**
//! (Doc 35 cut `SF.5`; `docs/design/35-ship-free-flight-combat-and-mining.md` §8.3, `Q-35-5`, `D-35-14`, `D-35-16`).
//!
//! Runs at 60 fps while pausing the docking hull in a [`DockingSession`]
//! outside the world tick (`paused_from_world_tick = true`):
//! - **Mandatory** for unassisted targets (`DisabledDerelict`, `ShipToShip`,
//!   `UnpoweredFieldMooring`) within `500 m`.
//! - **Opt-in** at station ports (`PortBerth`), where manual 6-DOF alignment
//!   saves terminal approach RCS propellant (`saved_delta_v_mps`) and approach
//!   time (`saved_approach_ic_secs`) compared to automated arrival docking.
//! - **Salvage & Tow (`doc 27/27a`)**: Hard-docking with an open-epoch
//!   [`HullStatus::DisabledDerelict`] permits attaching a salvage/tow line
//!   subject to the `2×` hull tonnage tow limit and `48 h` Registry contest
//!   window (`possession is not title`).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::space::bubble::{FacetBand, HullStatus, LocalSpaceBubble, ShipSectionId, SpacePartyId};

/// Maximum coarse range in metres to initiate a 6-DOF docking session (`500 m`).
pub const MAX_DOCKING_INIT_RANGE_M: i64 = 500;

/// Maximum coarse relative speed in m/s to initiate a 6-DOF docking session (`15 m/s`).
pub const MAX_DOCKING_INIT_REL_SPEED_MPS: i64 = 15;

/// Lateral alignment gate tolerance in millimetres (`±0.25 m`).
pub const DOCK_GATE_LATERAL_MM: i64 = 250;

/// Pitch and yaw alignment gate tolerance in millidegrees (`±2.0°`).
pub const DOCK_GATE_ATTITUDE_MDEG: i64 = 2_000;

/// Roll alignment gate tolerance in millidegrees (`±3.0°`).
pub const DOCK_GATE_ROLL_MDEG: i64 = 3_000;

/// Minimum positive closure rate in mm/s for magnetic latch (`0.10 m/s`).
pub const DOCK_GATE_CLOSURE_MIN_MMPS: i64 = 100;

/// Maximum safe closure rate in mm/s before collar scrape/bounce (`0.80 m/s`).
pub const DOCK_GATE_CLOSURE_MAX_MMPS: i64 = 800;

/// 48-hour salvage contest window in IC seconds (`doc 27a`: `48 * 3600 = 172,800 s`).
pub const SALVAGE_CONTEST_WINDOW_IC_SECS: u64 = 48 * 3_600;

/// What kind of target the bow collar is docking with (`Q-35-5`, `D-35-14`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockingTargetKind {
    /// Opt-in manual approach at a station port berth (`+helm/dock manual`).
    PortBerth {
        port_id: String,
        berth_label: String,
    },
    /// Mandatory manual hard-docking with an unpowered open-epoch derelict hull (`doc 23/27a`).
    DisabledDerelict {
        derelict_ship_id: String,
        derelict_tonnage_t: u32,
        prior_owner_party: Option<SpacePartyId>,
    },
    /// Mandatory manual hard-docking with another spacecraft in deep space.
    ShipToShip {
        partner_ship_id: String,
        partner_label: String,
    },
    /// Mandatory manual mooring at an unpowered field buoy or asteroid depot anchor.
    UnpoweredFieldMooring { site_id: String, mooring_id: String },
}

impl DockingTargetKind {
    /// Whether the 6-DOF docking mini-game is mandatory for this target (`Q-35-5`).
    #[must_use]
    pub const fn is_mandatory(&self) -> bool {
        !matches!(self, Self::PortBerth { .. })
    }

    /// Short display label for the `space.docking` camera HUD.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::PortBerth { berth_label, .. } => berth_label.as_str(),
            Self::DisabledDerelict {
                derelict_ship_id, ..
            } => derelict_ship_id.as_str(),
            Self::ShipToShip { partner_label, .. } => partner_label.as_str(),
            Self::UnpoweredFieldMooring { mooring_id, .. } => mooring_id.as_str(),
        }
    }

    /// Wire kind string for `space.docking`.
    #[must_use]
    pub const fn kind_str(&self) -> &'static str {
        match self {
            Self::PortBerth { .. } => "port_berth",
            Self::DisabledDerelict { .. } => "disabled_derelict",
            Self::ShipToShip { .. } => "ship_to_ship",
            Self::UnpoweredFieldMooring { .. } => "field_mooring",
        }
    }
}

/// 6-DOF pose and linear/angular rates of the docking hull relative to the target collar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockingPose6Dof {
    /// Horizontal offset from collar centerline in mm (`+` = starboard/right).
    pub dx_mm: i64,
    /// Vertical offset from collar centerline in mm (`+` = dorsal/up).
    pub dy_mm: i64,
    /// Axial separation from bow collar to target collar in mm (`>= 0`).
    pub dz_mm: i64,
    /// Pitch misalignment in millidegrees (`+` = nose up).
    pub pitch_mdeg: i64,
    /// Yaw misalignment in millidegrees (`+` = nose right).
    pub yaw_mdeg: i64,
    /// Roll misalignment in millidegrees (`+` = clockwise).
    pub roll_mdeg: i64,
    /// Horizontal drift velocity in mm/s.
    pub vx_mmps: i64,
    /// Vertical drift velocity in mm/s.
    pub vy_mmps: i64,
    /// Axial closure velocity toward the collar in mm/s (`+` = closing).
    pub vz_mmps: i64,
    /// Pitch angular rate in millidegrees/s.
    pub vpitch_mdeg_s: i64,
    /// Yaw angular rate in millidegrees/s.
    pub vyaw_mdeg_s: i64,
    /// Roll angular rate in millidegrees/s.
    pub vroll_mdeg_s: i64,
}

impl DockingPose6Dof {
    /// Whether lateral position `(dx_mm, dy_mm)` is inside the `±250 mm` (`±0.25 m`) gate.
    #[must_use]
    pub const fn lateral_aligned(&self) -> bool {
        self.dx_mm.abs() <= DOCK_GATE_LATERAL_MM && self.dy_mm.abs() <= DOCK_GATE_LATERAL_MM
    }

    /// Whether pitch and yaw are inside the `±2000 mdeg` (`±2.0°`) gate.
    #[must_use]
    pub const fn attitude_aligned(&self) -> bool {
        self.pitch_mdeg.abs() <= DOCK_GATE_ATTITUDE_MDEG
            && self.yaw_mdeg.abs() <= DOCK_GATE_ATTITUDE_MDEG
    }

    /// Whether roll is inside the `±3000 mdeg` (`±3.0°`) gate.
    #[must_use]
    pub const fn roll_aligned(&self) -> bool {
        self.roll_mdeg.abs() <= DOCK_GATE_ROLL_MDEG
    }

    /// Whether closure speed is inside the `100..=800 mm/s` (`0.10..=0.80 m/s`) latch band.
    #[must_use]
    pub const fn closure_safe(&self) -> bool {
        self.vz_mmps >= DOCK_GATE_CLOSURE_MIN_MMPS && self.vz_mmps <= DOCK_GATE_CLOSURE_MAX_MMPS
    }

    /// Whether all spatial and angular gates are satisfied.
    #[must_use]
    pub const fn pose_aligned(&self) -> bool {
        self.lateral_aligned() && self.attitude_aligned() && self.roll_aligned()
    }

    /// Whether all 5 alignment gates (lateral, pitch/yaw, roll, closure rate) are satisfied.
    #[must_use]
    pub const fn ready_to_clamp(&self) -> bool {
        self.pose_aligned() && self.closure_safe()
    }
}

/// Non-colliding 6-DOF control input for a single 60 fps frame (`D-35-16`).
///
/// Uses `W/S/A/D` for lateral/vertical RCS translation, `Arrow Keys` for
/// pitch/yaw trim, `Q/E` for roll, `R/F` for forward/reverse closure pulse,
/// and `X` to toggle RCS dampeners (never binding bare `Shift` or digit keys).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockingControlInput {
    /// Lateral horizontal thruster command: `-1` (`A` left), `0`, `+1` (`D` right).
    pub trans_x: i8,
    /// Vertical thruster command: `-1` (`S` down), `0`, `+1` (`W` up).
    pub trans_y: i8,
    /// Axial closure thruster pulse: `-1` (`F` brake/reverse), `0`, `+1` (`R` forward closure).
    pub closure_pulse: i8,
    /// Pitch trim command: `-1` (`Down Arrow`), `0`, `+1` (`Up Arrow`).
    pub pitch_trim: i8,
    /// Yaw trim command: `-1` (`Left Arrow`), `0`, `+1` (`Right Arrow`).
    pub yaw_trim: i8,
    /// Roll trim command: `-1` (`Q` CCW), `0`, `+1` (`E` CW).
    pub roll_trim: i8,
    /// Toggle automatic RCS lateral/rotational dampeners (`X` key).
    pub toggle_dampeners: bool,
    /// Abort the docking approach and back clear.
    pub abort: bool,
}

/// Bounded outcome of a completed [`DockingSession`] merged back into the world state (`D-35-14`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockingOutcome {
    /// Magnetic latches engaged cleanly within all 6-DOF gates at `< 0.8 m/s`.
    Clamped {
        target: DockingTargetKind,
        /// Final closure speed at latch in mm/s (`100..=800`).
        latch_closure_mmps: i64,
        /// Terminal approach propellant saved (`m/s`) when opt-in docking at a port.
        saved_delta_v_mps: u32,
        /// Terminal approach time saved (`IC seconds`) when opt-in docking at a port.
        saved_approach_ic_secs: u64,
    },
    /// Bow collar struck the target ring outside alignment gates or above `0.80 m/s`,
    /// scraping the `Front` facet and bouncing the hull back to safe stand-off.
    CollarScrapeBounce {
        target: DockingTargetKind,
        impact_closure_mmps: i64,
        front_facet_damage: u32,
        collar_damaged: bool,
    },
    /// Pilot aborted the approach and backed clear.
    Aborted { target: DockingTargetKind },
}

/// Live or completed state of a [`DockingSession`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockingSessionState {
    /// Session is actively running client-side at 60 fps while paused from the world tick.
    Active,
    /// Session has finished and is ready to merge its bounded [`DockingOutcome`].
    Completed(DockingOutcome),
}

/// Errors returned when initiating a [`DockingSession`] or attaching a salvage tow line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockingError {
    /// Ship was not found in the local space bubble.
    ShipNotFound { ship_id: String },
    /// Target ship/derelict was not found in the local space bubble.
    TargetNotFound { target_id: String },
    /// Distance to the target exceeds [`MAX_DOCKING_INIT_RANGE_M`] (`500 m`).
    OutOfRange { distance_m: i64, max_range_m: i64 },
    /// Coarse relative velocity exceeds [`MAX_DOCKING_INIT_REL_SPEED_MPS`] (`15 m/s`).
    ExcessCoarseVelocity {
        rel_speed_mps: i64,
        max_rel_speed_mps: i64,
    },
    /// Target ship is still powered/active and is not an open-epoch derelict.
    TargetNotDerelict { target_id: String },
    /// Session is not yet clamped to a derelict hull.
    NotClampedToDerelict,
    /// Derelict hull exceeds `doc 27a`'s `2×` towing ship tonnage limit.
    TowTonnageExceeded {
        towing_ship_tonnage_t: u32,
        derelict_tonnage_t: u32,
        max_towable_tonnage_t: u32,
    },
}

impl std::fmt::Display for DockingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ShipNotFound { ship_id } => write!(f, "docking ship '{ship_id}' not found"),
            Self::TargetNotFound { target_id } => {
                write!(f, "docking target '{target_id}' not found")
            }
            Self::OutOfRange {
                distance_m,
                max_range_m,
            } => write!(
                f,
                "docking target at {distance_m} m exceeds {max_range_m} m initiation range"
            ),
            Self::ExcessCoarseVelocity {
                rel_speed_mps,
                max_rel_speed_mps,
            } => write!(
                f,
                "coarse relative speed {rel_speed_mps} m/s exceeds {max_rel_speed_mps} m/s limit"
            ),
            Self::TargetNotDerelict { target_id } => {
                write!(f, "target '{target_id}' is not an open-epoch derelict")
            }
            Self::NotClampedToDerelict => {
                write!(
                    f,
                    "ship must be hard-docked (clamped) to a derelict before attaching tow"
                )
            }
            Self::TowTonnageExceeded {
                towing_ship_tonnage_t,
                derelict_tonnage_t,
                max_towable_tonnage_t,
            } => write!(
                f,
                "derelict tonnage {derelict_tonnage_t} t exceeds 2x tow cap ({max_towable_tonnage_t} t) for {towing_ship_tonnage_t} t hull"
            ),
        }
    }
}

impl std::error::Error for DockingError {}

/// Salvage tow record created after hard-docking with an open-epoch derelict (`doc 27/27a`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalvageTowRecord {
    pub towing_ship_id: String,
    pub derelict_ship_id: String,
    pub derelict_tonnage_t: u32,
    /// Preserved prior owner party (`doc 27a`: possession is not title).
    pub prior_owner_party: Option<SpacePartyId>,
    /// True when a non-owner recovered the hull, opening the 48 h Registry contest window.
    pub contest_open: bool,
    /// IC timestamp when the 48 h contest window expires (`now_ic_secs + 172,800`).
    pub contest_expires_ic_secs: u64,
}

/// Out-of-time-loop 60 fps 6-DOF docking session (`Q-35-5`, `D-35-14`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockingSession {
    pub session_id: String,
    pub ship_id: String,
    pub ship_party: SpacePartyId,
    pub ship_tonnage_t: u32,
    pub target: DockingTargetKind,
    /// While true, the ship is paused from the world/bubble tick (`Q-35-5`).
    pub paused_from_world_tick: bool,
    pub pose: DockingPose6Dof,
    /// Target collar angular drift (mdeg/s) for tumbling derelicts.
    pub target_tumble_mdeg_s: (i64, i64, i64),
    pub dampeners_active: bool,
    pub frame_index: u64,
    pub rcs_delta_v_spent_mmps: u64,
    pub state: DockingSessionState,
}

impl DockingSession {
    /// Start an opt-in 6-DOF docking approach at a station port berth (`+helm/dock manual`).
    #[must_use]
    pub fn start_port_approach(
        session_id: impl Into<String>,
        ship_id: impl Into<String>,
        ship_party: SpacePartyId,
        ship_tonnage_t: u32,
        port_id: impl Into<String>,
        berth_label: impl Into<String>,
        initial_pose: DockingPose6Dof,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            ship_id: ship_id.into(),
            ship_party,
            ship_tonnage_t,
            target: DockingTargetKind::PortBerth {
                port_id: port_id.into(),
                berth_label: berth_label.into(),
            },
            paused_from_world_tick: true,
            pose: initial_pose,
            target_tumble_mdeg_s: (0, 0, 0),
            dampeners_active: true,
            frame_index: 0,
            rcs_delta_v_spent_mmps: 0,
            state: DockingSessionState::Active,
        }
    }

    /// Initiate a mandatory 6-DOF hard-docking session with an open-epoch
    /// [`HullStatus::DisabledDerelict`] inside a [`LocalSpaceBubble`].
    ///
    /// Verifies that the docking ship is within `500 m` and `15 m/s` of the
    /// derelict hull and pauses the docking ship from the world tick (`Q-35-5`).
    pub fn start_derelict_docking(
        bubble: &LocalSpaceBubble,
        session_id: impl Into<String>,
        ship_id: &str,
        derelict_id: &str,
        ship_tonnage_t: u32,
        derelict_tonnage_t: u32,
        initial_pose: DockingPose6Dof,
    ) -> Result<Self, DockingError> {
        let ship = bubble
            .ships
            .get(ship_id)
            .ok_or_else(|| DockingError::ShipNotFound {
                ship_id: ship_id.to_owned(),
            })?;
        let derelict =
            bubble
                .ships
                .get(derelict_id)
                .ok_or_else(|| DockingError::TargetNotFound {
                    target_id: derelict_id.to_owned(),
                })?;

        if !matches!(derelict.status, HullStatus::DisabledDerelict { .. }) {
            return Err(DockingError::TargetNotDerelict {
                target_id: derelict_id.to_owned(),
            });
        }

        let dx = (derelict.pos_m[0] - ship.pos_m[0]) as f64;
        let dy = (derelict.pos_m[1] - ship.pos_m[1]) as f64;
        let dz = (derelict.pos_m[2] - ship.pos_m[2]) as f64;
        let dist_m = (dx * dx + dy * dy + dz * dz).sqrt().round() as i64;
        if dist_m > MAX_DOCKING_INIT_RANGE_M {
            return Err(DockingError::OutOfRange {
                distance_m: dist_m,
                max_range_m: MAX_DOCKING_INIT_RANGE_M,
            });
        }

        let dvx = (derelict.vel_mmps[0] - ship.vel_mmps[0]) as f64 / 1_000.0;
        let dvy = (derelict.vel_mmps[1] - ship.vel_mmps[1]) as f64 / 1_000.0;
        let dvz = (derelict.vel_mmps[2] - ship.vel_mmps[2]) as f64 / 1_000.0;
        let rel_speed_mps = (dvx * dvx + dvy * dvy + dvz * dvz).sqrt().round() as i64;
        if rel_speed_mps > MAX_DOCKING_INIT_REL_SPEED_MPS {
            return Err(DockingError::ExcessCoarseVelocity {
                rel_speed_mps,
                max_rel_speed_mps: MAX_DOCKING_INIT_REL_SPEED_MPS,
            });
        }

        Ok(Self {
            session_id: session_id.into(),
            ship_id: ship_id.to_owned(),
            ship_party: ship.party,
            ship_tonnage_t,
            target: DockingTargetKind::DisabledDerelict {
                derelict_ship_id: derelict_id.to_owned(),
                derelict_tonnage_t,
                prior_owner_party: Some(derelict.party),
            },
            paused_from_world_tick: true,
            pose: initial_pose,
            // Disabled derelicts have a gentle residual tumble the pilot trims out
            target_tumble_mdeg_s: (60, -60, 120),
            dampeners_active: true,
            frame_index: 0,
            rcs_delta_v_spent_mmps: 0,
            state: DockingSessionState::Active,
        })
    }

    /// Advance the client-side 6-DOF docking physics by one 60 fps frame (`1/60 s`).
    pub fn step_frame_60fps(&mut self, input: DockingControlInput) -> DockingSessionState {
        if !matches!(self.state, DockingSessionState::Active) {
            return self.state.clone();
        }
        self.frame_index = self.frame_index.saturating_add(1);

        if input.abort {
            self.paused_from_world_tick = false;
            let outcome = DockingOutcome::Aborted {
                target: self.target.clone(),
            };
            self.state = DockingSessionState::Completed(outcome);
            return self.state.clone();
        }

        if input.toggle_dampeners {
            self.dampeners_active = !self.dampeners_active;
        }

        // 1. Apply pilot RCS translation pulses (15 mm/s per frame = 0.90 m/s^2)
        let dv_x = i64::from(input.trans_x.clamp(-1, 1)) * 15;
        let dv_y = i64::from(input.trans_y.clamp(-1, 1)) * 15;
        let dv_z = i64::from(input.closure_pulse.clamp(-1, 1)) * 20;
        self.pose.vx_mmps += dv_x;
        self.pose.vy_mmps += dv_y;
        self.pose.vz_mmps += dv_z;
        self.rcs_delta_v_spent_mmps = self
            .rcs_delta_v_spent_mmps
            .saturating_add((dv_x.abs() + dv_y.abs() + dv_z.abs()) as u64);

        // 2. Apply pilot RCS attitude trim pulses (90 mdeg/s per frame)
        let dpitch = i64::from(input.pitch_trim.clamp(-1, 1)) * 90;
        let dyaw = i64::from(input.yaw_trim.clamp(-1, 1)) * 90;
        let droll = i64::from(input.roll_trim.clamp(-1, 1)) * 120;
        self.pose.vpitch_mdeg_s += dpitch;
        self.pose.vyaw_mdeg_s += dyaw;
        self.pose.vroll_mdeg_s += droll;

        // 3. Apply automatic RCS lateral & rotational dampeners when active and axis is uncommanded
        if self.dampeners_active {
            if input.trans_x == 0 {
                self.pose.vx_mmps = damp_toward_zero(self.pose.vx_mmps, 8);
            }
            if input.trans_y == 0 {
                self.pose.vy_mmps = damp_toward_zero(self.pose.vy_mmps, 8);
            }
            if input.pitch_trim == 0 {
                self.pose.vpitch_mdeg_s = damp_toward_zero(self.pose.vpitch_mdeg_s, 45);
            }
            if input.yaw_trim == 0 {
                self.pose.vyaw_mdeg_s = damp_toward_zero(self.pose.vyaw_mdeg_s, 45);
            }
            if input.roll_trim == 0 {
                self.pose.vroll_mdeg_s = damp_toward_zero(self.pose.vroll_mdeg_s, 60);
            }
        }

        // 4. Integrate 1/60 s step (using integer division `/ 60`)
        self.pose.dx_mm += self.pose.vx_mmps / 60;
        self.pose.dy_mm += self.pose.vy_mmps / 60;
        self.pose.dz_mm -= self.pose.vz_mmps / 60;

        self.pose.pitch_mdeg += (self.pose.vpitch_mdeg_s + self.target_tumble_mdeg_s.0) / 60;
        self.pose.yaw_mdeg += (self.pose.vyaw_mdeg_s + self.target_tumble_mdeg_s.1) / 60;
        self.pose.roll_mdeg += (self.pose.vroll_mdeg_s + self.target_tumble_mdeg_s.2) / 60;

        // 5. Check collar contact plane (`dz_mm <= 0`)
        if self.pose.dz_mm <= 0 {
            self.paused_from_world_tick = false;
            if self.pose.ready_to_clamp() {
                self.pose.dz_mm = 0;
                let (saved_dv, saved_secs) = match self.target {
                    DockingTargetKind::PortBerth { .. } => {
                        let spent_mps = (self.rcs_delta_v_spent_mmps / 1_000) as u32;
                        (45_u32.saturating_sub(spent_mps.min(35)), 900_u64)
                    }
                    _ => (0, 0),
                };
                let outcome = DockingOutcome::Clamped {
                    target: self.target.clone(),
                    latch_closure_mmps: self.pose.vz_mmps,
                    saved_delta_v_mps: saved_dv,
                    saved_approach_ic_secs: saved_secs,
                };
                self.state = DockingSessionState::Completed(outcome);
            } else {
                // Collar scrape & bounce!
                let impact_vz = self.pose.vz_mmps.max(0);
                let over_speed = (impact_vz - DOCK_GATE_CLOSURE_MAX_MMPS).max(0);
                let front_facet_damage = (8 + (over_speed / 120) as u32).clamp(8, 45);
                let collar_damaged = impact_vz > 1_100 || !self.pose.lateral_aligned();
                // Bounce hull back to 1.2 m stand-off with reverse velocity
                self.pose.dz_mm = 1_200;
                self.pose.vz_mmps = -240;
                let outcome = DockingOutcome::CollarScrapeBounce {
                    target: self.target.clone(),
                    impact_closure_mmps: impact_vz,
                    front_facet_damage,
                    collar_damaged,
                };
                self.state = DockingSessionState::Completed(outcome);
            }
        }

        self.state.clone()
    }

    /// Merge the completed [`DockingSession`] outcome back into the [`LocalSpaceBubble`],
    /// applying any collar-scrape damage to the ship's `Front` armour facet (`dray.json`).
    pub fn apply_outcome_to_bubble(&self, bubble: &mut LocalSpaceBubble) -> Option<DockingOutcome> {
        let DockingSessionState::Completed(ref outcome) = self.state else {
            return None;
        };
        if let DockingOutcome::CollarScrapeBounce {
            front_facet_damage, ..
        } = outcome
            && let Some(ship) = bubble.ships.get_mut(&self.ship_id)
            && let Some(hull_sec) = ship.sections.get_mut(&ShipSectionId::Hull)
        {
            let absorbed = if let Some(armor) = hull_sec.facets.get_mut(&FacetBand::Front) {
                let abs = (*armor).min(*front_facet_damage);
                *armor -= abs;
                abs
            } else {
                0
            };
            let bleed = front_facet_damage.saturating_sub(absorbed);
            if bleed > 0 {
                hull_sec.internals = hull_sec.internals.saturating_sub(bleed);
            }
        }
        Some(outcome.clone())
    }

    /// Attach a salvage/tow line after hard-docking (`Clamped`) with a
    /// [`DockingTargetKind::DisabledDerelict`], enforcing `doc 27a`'s `2×`
    /// tonnage tow limit and opening the 48-hour Registry contest window when
    /// recovered by a non-owner.
    pub fn attach_salvage_tow(&self, now_ic_secs: u64) -> Result<SalvageTowRecord, DockingError> {
        let DockingSessionState::Completed(DockingOutcome::Clamped { ref target, .. }) = self.state
        else {
            return Err(DockingError::NotClampedToDerelict);
        };
        let DockingTargetKind::DisabledDerelict {
            ref derelict_ship_id,
            derelict_tonnage_t,
            prior_owner_party,
        } = *target
        else {
            return Err(DockingError::NotClampedToDerelict);
        };

        let max_towable = self.ship_tonnage_t.saturating_mul(2);
        if derelict_tonnage_t > max_towable {
            return Err(DockingError::TowTonnageExceeded {
                towing_ship_tonnage_t: self.ship_tonnage_t,
                derelict_tonnage_t,
                max_towable_tonnage_t: max_towable,
            });
        }

        let contest_open = prior_owner_party.is_some_and(|owner| owner != self.ship_party);
        Ok(SalvageTowRecord {
            towing_ship_id: self.ship_id.clone(),
            derelict_ship_id: derelict_ship_id.clone(),
            derelict_tonnage_t,
            prior_owner_party,
            contest_open,
            contest_expires_ic_secs: now_ic_secs.saturating_add(SALVAGE_CONTEST_WINDOW_IC_SECS),
        })
    }

    /// Build the `space.docking` JSON feed consumed by `mush-pixel`'s `DockingPicture`.
    #[must_use]
    pub fn docking_feed(&self) -> Value {
        let status = match &self.state {
            DockingSessionState::Active => {
                if self.pose.ready_to_clamp() {
                    "ALIGNED-READY"
                } else if self.pose.pose_aligned() {
                    "POSE-ALIGNED"
                } else {
                    "APPROACHING"
                }
            }
            DockingSessionState::Completed(DockingOutcome::Clamped { .. }) => "CLAMPED",
            DockingSessionState::Completed(DockingOutcome::CollarScrapeBounce { .. }) => {
                "COLLAR-SCRAPE"
            }
            DockingSessionState::Completed(DockingOutcome::Aborted { .. }) => "ABORTED",
        };

        json!({
            "session_id": self.session_id,
            "ship_id": self.ship_id,
            "target_label": self.target.label(),
            "target_kind": self.target.kind_str(),
            "mandatory": self.target.is_mandatory(),
            "paused_from_world_tick": self.paused_from_world_tick,
            "dx_mm": self.pose.dx_mm,
            "dy_mm": self.pose.dy_mm,
            "dz_mm": self.pose.dz_mm,
            "pitch_mdeg": self.pose.pitch_mdeg,
            "yaw_mdeg": self.pose.yaw_mdeg,
            "roll_mdeg": self.pose.roll_mdeg,
            "vx_mmps": self.pose.vx_mmps,
            "vy_mmps": self.pose.vy_mmps,
            "vz_mmps": self.pose.vz_mmps,
            "dampeners": self.dampeners_active,
            "status": status
        })
    }
}

const fn damp_toward_zero(val: i64, step: i64) -> i64 {
    if val > step {
        val - step
    } else if val < -step {
        val + step
    } else {
        0
    }
}
