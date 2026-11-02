//! **Client-Side 60 fps 6-DOF Docking Mini-Game & Server Input-Tape Verification (`SF.R7`)**
//! (`docs/design/35-ship-free-flight-combat-and-mining.md` §8, §9 Story 2 step 3, §10.2 `SF.R7`,
//! `Q-35-5`, `D-35-12`, `D-35-14`, `docs/design/35a-space-flight-review.md` §1, §4, §5 #2/#7, §8.3).
//!
//! Implements the shared deterministic 60 Hz 6-DOF docking kinematics, bounded RLE
//! [`DockingInputTape`] wire format ($\le 3{,}600$ frames = $60\text{ s}$ at $60\text{ Hz}$),
//! server-side [`replay_docking_tape`] verification from [`DockingChallenge::start_pose`],
//! collar-scrape `Front`-facet impact damage on hard bounce ($v_z > 800\text{ mm/s}$),
//! live `stow.propellant` RCS debit, and salvage tow-line attachment under `doc 27/27a`'s
//! $2\times$ canonical `UnitSpec` dry-mass cap.

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

use super::{
    DOCK_GATE_ATTITUDE_MDEG, DOCK_GATE_CLOSURE_MAX_MMPS, DOCK_GATE_CLOSURE_MIN_MMPS,
    DOCK_GATE_LATERAL_MM, DOCK_GATE_ROLL_MDEG, DockingPose6Dof,
};

/// Fixed simulation frame rate ($60\text{ Hz}$) for the 6-DOF docking mini-game (`Q-35-5`, `D-35-14`).
pub const DOCKING_FPS: i64 = 60;

/// Maximum number of 60 Hz frames allowed in a single submitted [`DockingInputTape`]
/// ($3{,}600\text{ frames} = 60\text{ s}$ at $60\text{ Hz}$, `D-35-14`).
pub const MAX_DOCKING_TAPE_FRAMES: usize = 3_600;

/// Maximum forward range ($500\text{ mm}$) at which the docking clamp can engage (`D-35-14`).
pub const DOCK_GATE_RANGE_Z_MM: i64 = 500;

/// Maximum residual lateral velocity ($300\text{ mm/s}$) permitted at clamp engagement.
pub const DOCK_GATE_MAX_LAT_VEL_MMPS: i64 = 300;

/// Maximum residual angular rate ($500\text{ mdeg/s}$) permitted at clamp engagement.
pub const DOCK_GATE_MAX_ANG_RATE_MDEG_S: i32 = 500;

/// Default `Front`-facet armor damage (points) inflicted by a hard collar-scrape bounce
/// ($v_z > 800\text{ mm/s}$ or unaligned contact at $z = 0$, `D-35-14`).
pub const COLLAR_SCRAPE_FRONT_ARMOR_DAMAGE: u32 = 15;

/// Per-frame 6-DOF thruster/trim control state for a single 60 Hz docking frame (`D-35-14`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockingControlInput {
    /// Lateral horizontal RCS pulse (`-1` = port/left, `0` = coast, `+1` = starboard/right).
    pub trans_x: i8,
    /// Lateral vertical RCS pulse (`-1` = down, `0` = coast, `+1` = up).
    pub trans_y: i8,
    /// Forward/aft closure RCS pulse (`-1` = braking/aft, `0` = coast, `+1` = forward closure).
    pub closure_pulse: i8,
    /// Pitch trim pulse (`-1` = nose down, `0` = hold, `+1` = nose up).
    pub pitch_trim: i8,
    /// Yaw trim pulse (`-1` = nose left, `0` = hold, `+1` = nose right).
    pub yaw_trim: i8,
    /// Roll trim pulse (`-1` = roll CCW, `0` = hold, `+1` = roll CW).
    pub roll_trim: i8,
    /// Edge-triggered toggle for coupled rotational/translational RCS dampeners (`Space`).
    pub toggle_dampeners: bool,
    /// Explicit pilot abort (`Esc` / `x`).
    pub abort: bool,
}

impl DockingControlInput {
    /// Encode a 3-state axis (`-1`, `0`, `+1`) into a 2-bit field (`0b00 = 0`, `0b01 = +1`, `0b10 = -1`).
    const fn encode_axis(val: i8) -> u16 {
        if val > 0 {
            0b01
        } else if val < 0 {
            0b10
        } else {
            0b00
        }
    }

    /// Decode a 2-bit field (`0b00 = 0`, `0b01 = +1`, `0b10 = -1`) into a 3-state axis.
    const fn decode_axis(bits: u16) -> i8 {
        match bits & 0b11 {
            0b01 => 1,
            0b10 => -1,
            _ => 0,
        }
    }

    /// Pack this frame's control inputs into a compact 16-bit bitmask for RLE wire transport (`D-35-14`).
    #[must_use]
    pub const fn to_bitmask(&self) -> u16 {
        let mut mask = 0u16;
        mask |= Self::encode_axis(self.trans_x);
        mask |= Self::encode_axis(self.trans_y) << 2;
        mask |= Self::encode_axis(self.closure_pulse) << 4;
        mask |= Self::encode_axis(self.pitch_trim) << 6;
        mask |= Self::encode_axis(self.yaw_trim) << 8;
        mask |= Self::encode_axis(self.roll_trim) << 10;
        if self.toggle_dampeners {
            mask |= 1 << 12;
        }
        if self.abort {
            mask |= 1 << 13;
        }
        mask
    }

    /// Unpack a 16-bit RLE wire bitmask into a [`DockingControlInput`].
    #[must_use]
    pub const fn from_bitmask(mask: u16) -> Self {
        Self {
            trans_x: Self::decode_axis(mask),
            trans_y: Self::decode_axis(mask >> 2),
            closure_pulse: Self::decode_axis(mask >> 4),
            pitch_trim: Self::decode_axis(mask >> 6),
            yaw_trim: Self::decode_axis(mask >> 8),
            roll_trim: Self::decode_axis(mask >> 10),
            toggle_dampeners: (mask & (1 << 12)) != 0,
            abort: (mask & (1 << 13)) != 0,
        }
    }

    /// Return the number of active RCS thruster axes firing in this frame (`0..=6`).
    #[must_use]
    pub const fn active_axis_pulses(&self) -> u32 {
        let mut count = 0u32;
        if self.trans_x != 0 {
            count += 1;
        }
        if self.trans_y != 0 {
            count += 1;
        }
        if self.closure_pulse != 0 {
            count += 1;
        }
        if self.pitch_trim != 0 {
            count += 1;
        }
        if self.yaw_trim != 0 {
            count += 1;
        }
        if self.roll_trim != 0 {
            count += 1;
        }
        count
    }
}

/// Ship RCS thruster authority parameters for the 60 Hz 6-DOF docking simulation (`D-35-14`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockingRcsAuthority {
    /// Lateral velocity change per 60 Hz frame of RCS thrust (`mm/s`).
    pub trans_dv_mmps_per_frame: i64,
    /// Closure velocity change per 60 Hz frame of RCS thrust (`mm/s`).
    pub closure_dv_mmps_per_frame: i64,
    /// Pitch/yaw angular rate change per 60 Hz frame of RCS trim (`mdeg/s`).
    pub attitude_rate_mdeg_s_per_frame: i32,
    /// Roll angular rate change per 60 Hz frame of RCS trim (`mdeg/s`).
    pub roll_rate_mdeg_s_per_frame: i32,
    /// Coupled dampener lateral velocity bleed per 60 Hz frame (`mm/s`).
    pub damp_trans_mmps_per_frame: i64,
    /// Coupled dampener pitch/yaw rate bleed per 60 Hz frame (`mdeg/s`).
    pub damp_attitude_mdeg_s_per_frame: i32,
    /// Coupled dampener roll rate bleed per 60 Hz frame (`mdeg/s`).
    pub damp_roll_mdeg_s_per_frame: i32,
    /// Propellant mass (`kg`) debited per 12 RCS axis-frame pulses.
    pub kg_per_pulse_batch: u32,
}

impl Default for DockingRcsAuthority {
    fn default() -> Self {
        Self {
            trans_dv_mmps_per_frame: 30,
            closure_dv_mmps_per_frame: 25,
            attitude_rate_mdeg_s_per_frame: 180,
            roll_rate_mdeg_s_per_frame: 240,
            damp_trans_mmps_per_frame: 15,
            damp_attitude_mdeg_s_per_frame: 90,
            damp_roll_mdeg_s_per_frame: 120,
            kg_per_pulse_batch: 1,
        }
    }
}

/// Per-gate alignment breakdown for the 5 docking collar gates (`doc 35` §8.1, `D-35-14`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockingGateStatus {
    /// Lateral gate: `|lateral_x_mm| <= 250` and `|lateral_y_mm| <= 250` and `|vel_x|, |vel_y| <= 300 mm/s`.
    pub lateral_ok: bool,
    /// Attitude gate: `|pitch_err_mdeg| <= 2,000` and `|yaw_err_mdeg| <= 2,000` and rates `<= 500 mdeg/s`.
    pub attitude_ok: bool,
    /// Roll gate: `|roll_err_mdeg| <= 3,000` and `|roll_rate_mdeg_s| <= 500 mdeg/s`.
    pub roll_ok: bool,
    /// Closure speed gate: `100 <= closure_mmps <= 800`.
    pub closure_ok: bool,
    /// Forward range gate: `0 <= range_z_mm <= 500`.
    pub range_ok: bool,
}

impl DockingGateStatus {
    /// Return the number of currently satisfied alignment gates (`0..=5`).
    #[must_use]
    pub const fn green_count(&self) -> u8 {
        let mut count = 0u8;
        if self.lateral_ok {
            count += 1;
        }
        if self.attitude_ok {
            count += 1;
        }
        if self.roll_ok {
            count += 1;
        }
        if self.closure_ok {
            count += 1;
        }
        if self.range_ok {
            count += 1;
        }
        count
    }

    /// Return whether all 5 gates are simultaneously green (`doc 35` §8.1).
    #[must_use]
    pub const fn all_green(&self) -> bool {
        self.lateral_ok && self.attitude_ok && self.roll_ok && self.closure_ok && self.range_ok
    }
}

/// Evaluate all 5 docking alignment gates against `pose` (`doc 35` §8.1, `D-35-14`).
#[must_use]
pub const fn evaluate_docking_gates(pose: &DockingPose6Dof) -> DockingGateStatus {
    let lateral_ok = pose.lateral_x_mm.abs() <= DOCK_GATE_LATERAL_MM
        && pose.lateral_y_mm.abs() <= DOCK_GATE_LATERAL_MM
        && pose.vel_x_mmps.abs() <= DOCK_GATE_MAX_LAT_VEL_MMPS
        && pose.vel_y_mmps.abs() <= DOCK_GATE_MAX_LAT_VEL_MMPS;
    let attitude_ok = pose.pitch_err_mdeg.abs() <= DOCK_GATE_ATTITUDE_MDEG
        && pose.yaw_err_mdeg.abs() <= DOCK_GATE_ATTITUDE_MDEG
        && pose.pitch_rate_mdeg_s.abs() <= DOCK_GATE_MAX_ANG_RATE_MDEG_S
        && pose.yaw_rate_mdeg_s.abs() <= DOCK_GATE_MAX_ANG_RATE_MDEG_S;
    let roll_ok = pose.roll_err_mdeg.abs() <= DOCK_GATE_ROLL_MDEG
        && pose.roll_rate_mdeg_s.abs() <= DOCK_GATE_MAX_ANG_RATE_MDEG_S;
    let closure_ok = pose.closure_mmps >= DOCK_GATE_CLOSURE_MIN_MMPS
        && pose.closure_mmps <= DOCK_GATE_CLOSURE_MAX_MMPS;
    let range_ok = pose.range_z_mm >= 0 && pose.range_z_mm <= DOCK_GATE_RANGE_Z_MM;
    DockingGateStatus {
        lateral_ok,
        attitude_ok,
        roll_ok,
        closure_ok,
        range_ok,
    }
}

/// Target classification for a 6-DOF [`DockingSession`] (`doc 35` §8.1, `D-35-12`, `D-35-14`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockingTargetKind {
    /// Docking at a stationary port or orbital station berth (merges into `Berth::Docked`).
    PortBerth {
        /// Canonical port or body identifier (e.g. `"stickney"`, `"ceres"`).
        port_id: String,
        /// Display name of the port berth.
        port_name: String,
    },
    /// Hard-docking with a disabled derelict hull in a tactical bubble or orbital site (`D-35-12`).
    DisabledDerelict {
        /// Object ID of the disabled derelict hull.
        derelict_hull: ObjId,
        /// Display name of the derelict hull.
        derelict_name: String,
    },
}

/// Deterministic 60 Hz docking challenge parameters issued by the server when a
/// [`DockingSession`] begins (`D-35-14`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockingChallenge {
    /// Unique session identifier (`"dock.<hull>.<seq>"`).
    pub session_id: String,
    /// Starting 6-DOF collar pose (guaranteed outside the alignment gates so an empty/default
    /// input tape cannot auto-clamp; `35a` §1, §4).
    pub start_pose: DockingPose6Dof,
    /// Ship RCS thruster authority for the session.
    pub rcs_authority: DockingRcsAuthority,
    /// Maximum allowed tape length in 60 Hz frames ($\le 3{,}600$).
    pub max_frames: usize,
    /// Target collar tumble rate `[pitch_mdeg_s, yaw_mdeg_s, roll_mdeg_s]` (`[0, 0, 0]` for ports,
    /// non-zero for unstabilised derelicts; `doc 35` §8.1).
    pub target_tumble_mdeg_s: [i32; 3],
}

impl DockingChallenge {
    /// Construct a standard port berth docking challenge whose `start_pose` begins outside
    /// every alignment gate (`lateral`, `attitude`, `roll`, `closure`, and `range_z`).
    #[must_use]
    pub fn standard_port(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            start_pose: DockingPose6Dof {
                lateral_x_mm: 600,
                lateral_y_mm: -420,
                range_z_mm: 2_400,
                vel_x_mmps: 0,
                vel_y_mmps: 0,
                closure_mmps: 0,
                pitch_err_mdeg: -2_400,
                yaw_err_mdeg: 2_400,
                roll_err_mdeg: -3_600,
                pitch_rate_mdeg_s: 0,
                yaw_rate_mdeg_s: 0,
                roll_rate_mdeg_s: 0,
            },
            rcs_authority: DockingRcsAuthority::default(),
            max_frames: MAX_DOCKING_TAPE_FRAMES,
            target_tumble_mdeg_s: [0, 0, 0],
        }
    }

    /// Construct a derelict collar docking challenge with a gentle residual tumble rate (`doc 35` §8.1).
    #[must_use]
    pub fn derelict_collar(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            start_pose: DockingPose6Dof {
                lateral_x_mm: -540,
                lateral_y_mm: 480,
                range_z_mm: 2_100,
                vel_x_mmps: 0,
                vel_y_mmps: 0,
                closure_mmps: 0,
                pitch_err_mdeg: 2_400,
                yaw_err_mdeg: -2_400,
                roll_err_mdeg: 3_600,
                pitch_rate_mdeg_s: 0,
                yaw_rate_mdeg_s: 0,
                roll_rate_mdeg_s: 0,
            },
            rcs_authority: DockingRcsAuthority::default(),
            max_frames: MAX_DOCKING_TAPE_FRAMES,
            target_tumble_mdeg_s: [0, 0, 60],
        }
    }
}

/// Active server-side docking session state for a hull (`Q-35-5`, `D-35-14`).
///
/// While present in `SpaceBubbleBook::docking_sessions`, `paused_from_world_tick` steps
/// the hull out of the live space/bubble tick loop until the client submits its bounded
/// [`DockingInputTape`] or aborts (`35a` §5 #7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockingSession {
    /// Carrier hull object ID performing the docking approach.
    pub hull_id: ObjId,
    /// Target port berth or disabled derelict hull.
    pub target: DockingTargetKind,
    /// Deterministic challenge parameters and starting 6-DOF pose.
    pub challenge: DockingChallenge,
    /// Current server-tracked live pose (for interactive `DOCK ...` step commands prior to tape commit).
    pub live_stepper: DockingStepperState,
    /// Whether this hull is currently stepped out of the world tick (`Q-35-5`, `D-35-14`).
    pub paused_from_world_tick: bool,
    /// World IC timestamp (`s`) when the session was opened.
    pub started_ic_s: i64,
}

/// One run-length encoded span of identical 60 Hz [`DockingControlInput`] frames (`D-35-14`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockingTapeSpan {
    /// Packed 16-bit [`DockingControlInput`] bitmask.
    pub mask: u16,
    /// Number of consecutive 60 Hz frames (`>= 1`) with `mask`.
    pub frames: u16,
}

/// Bounded sequence of 60 Hz pilot inputs ($\le 3{,}600$ frames) submitted by the client
/// at the end of a 6-DOF docking approach (`D-35-14`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockingInputTape {
    /// Run-length encoded input spans.
    pub spans: Vec<DockingTapeSpan>,
}

/// Validation or replay failure on a submitted [`DockingInputTape`] (`D-35-14`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockingTapeError {
    /// The submitted tape contains zero frames.
    EmptyTape,
    /// The submitted tape exceeds [`MAX_DOCKING_TAPE_FRAMES`] (`3,600` frames).
    ExceedsMaxFrames {
        /// Total frames requested in the tape.
        frames: usize,
        /// Maximum allowed frames.
        max_frames: usize,
    },
    /// A span in the RLE string could not be parsed (`<hex_mask>*<count>`).
    MalformedRle(String),
    /// The session ID did not match the hull's active [`DockingSession`].
    SessionIdMismatch {
        /// Expected active session ID.
        expected: String,
        /// Submitted session ID.
        got: String,
    },
}

impl std::fmt::Display for DockingTapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyTape => write!(f, "docking input tape is empty"),
            Self::ExceedsMaxFrames { frames, max_frames } => write!(
                f,
                "docking input tape length ({frames} frames) exceeds cap ({max_frames} frames)"
            ),
            Self::MalformedRle(msg) => write!(f, "malformed docking RLE tape: {msg}"),
            Self::SessionIdMismatch { expected, got } => write!(
                f,
                "docking session ID mismatch (expected '{expected}', got '{got}')"
            ),
        }
    }
}

impl DockingInputTape {
    /// Construct a tape from a slice of per-frame [`DockingControlInput`]s, compressing
    /// consecutive identical inputs into [`DockingTapeSpan`]s.
    #[must_use]
    pub fn from_frames(frames: &[DockingControlInput]) -> Self {
        let mut spans: Vec<DockingTapeSpan> = Vec::new();
        for input in frames {
            let mask = input.to_bitmask();
            if let Some(last) = spans.last_mut()
                && last.mask == mask
                && last.frames < u16::MAX
            {
                last.frames = last.frames.saturating_add(1);
                continue;
            }
            spans.push(DockingTapeSpan { mask, frames: 1 });
        }
        Self { spans }
    }

    /// Append a single 60 Hz frame input to the RLE tape.
    pub fn push_frame(&mut self, input: DockingControlInput) {
        let mask = input.to_bitmask();
        if let Some(last) = self.spans.last_mut()
            && last.mask == mask
            && last.frames < u16::MAX
        {
            last.frames = last.frames.saturating_add(1);
            return;
        }
        self.spans.push(DockingTapeSpan { mask, frames: 1 });
    }

    /// Return the total number of 60 Hz frames across all spans.
    #[must_use]
    pub fn total_frames(&self) -> usize {
        self.spans.iter().fold(0usize, |acc, span| {
            acc.saturating_add(usize::from(span.frames))
        })
    }

    /// Encode the tape as a compact ASCII RLE string (`"<hex_mask>*<frames>,..."`).
    #[must_use]
    pub fn encode_rle(&self) -> String {
        self.spans
            .iter()
            .map(|span| format!("{:04x}*{}", span.mask, span.frames))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Decode a compact ASCII RLE string (`"<hex_mask>*<frames>,..."`) into a bounded
    /// [`DockingInputTape`].
    ///
    /// # Errors
    ///
    /// Returns [`DockingTapeError`] if the string is empty, malformed, or exceeds
    /// [`MAX_DOCKING_TAPE_FRAMES`].
    pub fn decode_rle(encoded: &str) -> Result<Self, DockingTapeError> {
        let trimmed = encoded.trim();
        if trimmed.is_empty() {
            return Err(DockingTapeError::EmptyTape);
        }
        let mut spans = Vec::new();
        let mut total = 0usize;
        for token in trimmed.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            let Some((mask_hex, count_str)) = token.split_once('*') else {
                return Err(DockingTapeError::MalformedRle(format!(
                    "missing '*' separator in span '{token}'"
                )));
            };
            let mask_clean = mask_hex
                .trim()
                .strip_prefix("0x")
                .or_else(|| mask_hex.trim().strip_prefix("0X"))
                .unwrap_or(mask_hex.trim());
            let mask = u16::from_str_radix(mask_clean, 16).map_err(|_| {
                DockingTapeError::MalformedRle(format!("invalid hex mask '{mask_hex}'"))
            })?;
            let frames = count_str.trim().parse::<u16>().map_err(|_| {
                DockingTapeError::MalformedRle(format!("invalid frame count '{count_str}'"))
            })?;
            if frames == 0 {
                return Err(DockingTapeError::MalformedRle(
                    "span frame count must be >= 1".to_owned(),
                ));
            }
            total = total.saturating_add(usize::from(frames));
            if total > MAX_DOCKING_TAPE_FRAMES {
                return Err(DockingTapeError::ExceedsMaxFrames {
                    frames: total,
                    max_frames: MAX_DOCKING_TAPE_FRAMES,
                });
            }
            spans.push(DockingTapeSpan { mask, frames });
        }
        if spans.is_empty() {
            return Err(DockingTapeError::EmptyTape);
        }
        Ok(Self { spans })
    }
}

/// Exact 60 Hz integer integrator state with sub-frame modulo-60 remainder accumulators
/// (`35a` §5 #2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockingStepperState {
    /// Current 6-DOF collar pose.
    pub pose: DockingPose6Dof,
    /// Whether coupled RCS rotational/lateral dampeners are currently active.
    pub dampeners_on: bool,
    /// Modulo-60 lateral X integration remainder (`mm * 60`).
    pub rem_x: i64,
    /// Modulo-60 lateral Y integration remainder (`mm * 60`).
    pub rem_y: i64,
    /// Modulo-60 forward Z integration remainder (`mm * 60`).
    pub rem_z: i64,
    /// Modulo-60 pitch integration remainder (`mdeg * 60`).
    pub rem_pitch: i32,
    /// Modulo-60 yaw integration remainder (`mdeg * 60`).
    pub rem_yaw: i32,
    /// Modulo-60 roll integration remainder (`mdeg * 60`).
    pub rem_roll: i32,
    /// Cumulative active RCS axis pulses fired during the session.
    pub rcs_pulses: u32,
    /// Number of 60 Hz frames stepped so far.
    pub frames_stepped: usize,
}

impl DockingStepperState {
    /// Initialise a 60 Hz stepper from `start_pose` with coupled dampeners enabled.
    #[must_use]
    pub const fn new(start_pose: DockingPose6Dof) -> Self {
        Self {
            pose: start_pose,
            dampeners_on: true,
            rem_x: 0,
            rem_y: 0,
            rem_z: 0,
            rem_pitch: 0,
            rem_yaw: 0,
            rem_roll: 0,
            rcs_pulses: 0,
            frames_stepped: 0,
        }
    }

    /// Bleed a signed velocity toward zero by at most `step`.
    const fn bleed_i64(val: i64, step: i64) -> i64 {
        if val > step {
            val - step
        } else if val < -step {
            val + step
        } else {
            0
        }
    }

    /// Bleed a signed angular rate toward zero by at most `step`.
    const fn bleed_i32(val: i32, step: i32) -> i32 {
        if val > step {
            val - step
        } else if val < -step {
            val + step
        } else {
            0
        }
    }

    /// Advance the 6-DOF collar state by a single 60 Hz frame (`16.67 ms`).
    pub fn step_frame(
        &mut self,
        input: DockingControlInput,
        auth: &DockingRcsAuthority,
        target_tumble_mdeg_s: [i32; 3],
    ) {
        if input.toggle_dampeners {
            self.dampeners_on = !self.dampeners_on;
        }

        let tx = i64::from(input.trans_x.clamp(-1, 1));
        let ty = i64::from(input.trans_y.clamp(-1, 1));
        let tz = i64::from(input.closure_pulse.clamp(-1, 1));
        let dpitch = i32::from(input.pitch_trim.clamp(-1, 1));
        let dyaw = i32::from(input.yaw_trim.clamp(-1, 1));
        let droll = i32::from(input.roll_trim.clamp(-1, 1));

        self.pose.vel_x_mmps = self
            .pose
            .vel_x_mmps
            .saturating_add(tx.saturating_mul(auth.trans_dv_mmps_per_frame));
        self.pose.vel_y_mmps = self
            .pose
            .vel_y_mmps
            .saturating_add(ty.saturating_mul(auth.trans_dv_mmps_per_frame));
        self.pose.closure_mmps = self
            .pose
            .closure_mmps
            .saturating_add(tz.saturating_mul(auth.closure_dv_mmps_per_frame));

        self.pose.pitch_rate_mdeg_s = self
            .pose
            .pitch_rate_mdeg_s
            .saturating_add(dpitch.saturating_mul(auth.attitude_rate_mdeg_s_per_frame));
        self.pose.yaw_rate_mdeg_s = self
            .pose
            .yaw_rate_mdeg_s
            .saturating_add(dyaw.saturating_mul(auth.attitude_rate_mdeg_s_per_frame));
        self.pose.roll_rate_mdeg_s = self
            .pose
            .roll_rate_mdeg_s
            .saturating_add(droll.saturating_mul(auth.roll_rate_mdeg_s_per_frame));

        let mut frame_pulses = input.active_axis_pulses();

        if self.dampeners_on {
            if tx == 0 && self.pose.vel_x_mmps != 0 {
                self.pose.vel_x_mmps =
                    Self::bleed_i64(self.pose.vel_x_mmps, auth.damp_trans_mmps_per_frame);
                frame_pulses = frame_pulses.saturating_add(1);
            }
            if ty == 0 && self.pose.vel_y_mmps != 0 {
                self.pose.vel_y_mmps =
                    Self::bleed_i64(self.pose.vel_y_mmps, auth.damp_trans_mmps_per_frame);
                frame_pulses = frame_pulses.saturating_add(1);
            }
            if dpitch == 0 && self.pose.pitch_rate_mdeg_s != 0 {
                self.pose.pitch_rate_mdeg_s = Self::bleed_i32(
                    self.pose.pitch_rate_mdeg_s,
                    auth.damp_attitude_mdeg_s_per_frame,
                );
                frame_pulses = frame_pulses.saturating_add(1);
            }
            if dyaw == 0 && self.pose.yaw_rate_mdeg_s != 0 {
                self.pose.yaw_rate_mdeg_s = Self::bleed_i32(
                    self.pose.yaw_rate_mdeg_s,
                    auth.damp_attitude_mdeg_s_per_frame,
                );
                frame_pulses = frame_pulses.saturating_add(1);
            }
            if droll == 0 && self.pose.roll_rate_mdeg_s != 0 {
                self.pose.roll_rate_mdeg_s =
                    Self::bleed_i32(self.pose.roll_rate_mdeg_s, auth.damp_roll_mdeg_s_per_frame);
                frame_pulses = frame_pulses.saturating_add(1);
            }
        }

        self.rcs_pulses = self.rcs_pulses.saturating_add(frame_pulses);

        // Exact modulo-60 sub-frame integration (`35a` §5 #2):
        let tot_x = self.pose.vel_x_mmps.saturating_add(self.rem_x);
        self.pose.lateral_x_mm = self.pose.lateral_x_mm.saturating_add(tot_x / DOCKING_FPS);
        self.rem_x = tot_x % DOCKING_FPS;

        let tot_y = self.pose.vel_y_mmps.saturating_add(self.rem_y);
        self.pose.lateral_y_mm = self.pose.lateral_y_mm.saturating_add(tot_y / DOCKING_FPS);
        self.rem_y = tot_y % DOCKING_FPS;

        let tot_z = self.pose.closure_mmps.saturating_add(self.rem_z);
        self.pose.range_z_mm = self
            .pose
            .range_z_mm
            .saturating_sub(tot_z / DOCKING_FPS)
            .max(0);
        self.rem_z = tot_z % DOCKING_FPS;

        let net_pitch_rate = self
            .pose
            .pitch_rate_mdeg_s
            .saturating_sub(target_tumble_mdeg_s[0]);
        let tot_pitch = net_pitch_rate.saturating_add(self.rem_pitch);
        self.pose.pitch_err_mdeg = self.pose.pitch_err_mdeg.saturating_add(tot_pitch / 60);
        self.rem_pitch = tot_pitch % 60;

        let net_yaw_rate = self
            .pose
            .yaw_rate_mdeg_s
            .saturating_sub(target_tumble_mdeg_s[1]);
        let tot_yaw = net_yaw_rate.saturating_add(self.rem_yaw);
        self.pose.yaw_err_mdeg = self.pose.yaw_err_mdeg.saturating_add(tot_yaw / 60);
        self.rem_yaw = tot_yaw % 60;

        let net_roll_rate = self
            .pose
            .roll_rate_mdeg_s
            .saturating_sub(target_tumble_mdeg_s[2]);
        let tot_roll = net_roll_rate.saturating_add(self.rem_roll);
        self.pose.roll_err_mdeg = self.pose.roll_err_mdeg.saturating_add(tot_roll / 60);
        self.rem_roll = tot_roll % 60;

        self.frames_stepped = self.frames_stepped.saturating_add(1);
    }

    /// Compute total RCS propellant mass (`kg`) spent so far (`>= 1 kg` whenever any pulse fired).
    #[must_use]
    pub const fn rcs_spent_kg(&self, auth: &DockingRcsAuthority) -> u32 {
        if self.rcs_pulses == 0 {
            0
        } else {
            let batches = self.rcs_pulses.div_ceil(12);
            batches.saturating_mul(auth.kg_per_pulse_batch)
        }
    }
}

/// Terminal outcome of replaying a [`DockingInputTape`] (`D-35-14`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockingOutcome {
    /// All 5 alignment gates were green inside `0 <= range_z_mm <= 500 mm`: hard dock achieved.
    Clamped,
    /// Contact plane (`range_z_mm == 0`) was struck with excessive closure speed
    /// ($v_z > 800\text{ mm/s}$) or unaligned collar attitude/offset, bouncing the ship
    /// back and scraping `Front` facet armor.
    CollarScrapeBounce {
        /// Impact closure speed (`mm/s`).
        impact_closure_mmps: i64,
        /// `Front` facet armor damage (`pts`) inflicted by the collar scrape.
        front_armor_damage: u32,
    },
    /// The tape ended before reaching clamp range with all 5 gates green (rejected; no state change).
    OutOfGate {
        /// Final gate status at the end of the tape.
        gates: DockingGateStatus,
    },
    /// The pilot explicitly aborted the docking approach (`input.abort == true`).
    Aborted,
}

/// Deterministic verification report produced by [`replay_docking_tape`] (`D-35-14`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockingReplayReport {
    /// Terminal outcome of the replayed input tape.
    pub outcome: DockingOutcome,
    /// Final 6-DOF collar pose at termination.
    pub final_pose: DockingPose6Dof,
    /// Final 5-gate status at termination.
    pub final_gates: DockingGateStatus,
    /// Number of 60 Hz frames replayed before termination.
    pub frames_replayed: usize,
    /// Total RCS propellant (`kg`) consumed by the maneuver (`stow.propellant` debit).
    pub rcs_spent_kg: u32,
}

/// Replay a client-submitted [`DockingInputTape`] from `challenge.start_pose` at 60 Hz
/// and verify whether it achieves a valid hard-dock clamp (`D-35-14`).
///
/// # Errors
///
/// Returns [`DockingTapeError`] if the tape is empty or exceeds `challenge.max_frames`.
pub fn replay_docking_tape(
    challenge: &DockingChallenge,
    tape: &DockingInputTape,
) -> Result<DockingReplayReport, DockingTapeError> {
    let total_frames = tape.total_frames();
    if total_frames == 0 {
        return Err(DockingTapeError::EmptyTape);
    }
    let max_frames = challenge.max_frames.min(MAX_DOCKING_TAPE_FRAMES);
    if total_frames > max_frames {
        return Err(DockingTapeError::ExceedsMaxFrames {
            frames: total_frames,
            max_frames,
        });
    }

    let mut stepper = DockingStepperState::new(challenge.start_pose.clone());

    for span in &tape.spans {
        let input = DockingControlInput::from_bitmask(span.mask);
        for _ in 0..span.frames {
            if input.abort {
                let final_gates = evaluate_docking_gates(&stepper.pose);
                let rcs_spent_kg = stepper.rcs_spent_kg(&challenge.rcs_authority);
                return Ok(DockingReplayReport {
                    outcome: DockingOutcome::Aborted,
                    final_pose: stepper.pose,
                    final_gates,
                    frames_replayed: stepper.frames_stepped,
                    rcs_spent_kg,
                });
            }

            stepper.step_frame(
                input,
                &challenge.rcs_authority,
                challenge.target_tumble_mdeg_s,
            );

            let gates = evaluate_docking_gates(&stepper.pose);
            if gates.all_green() {
                let rcs_spent_kg = stepper.rcs_spent_kg(&challenge.rcs_authority);
                return Ok(DockingReplayReport {
                    outcome: DockingOutcome::Clamped,
                    final_pose: stepper.pose,
                    final_gates: gates,
                    frames_replayed: stepper.frames_stepped,
                    rcs_spent_kg,
                });
            }

            // Hard impact or unaligned contact at `range_z_mm == 0` (`D-35-14`):
            if stepper.pose.range_z_mm == 0 && stepper.pose.closure_mmps > 0 {
                let over_speed =
                    (stepper.pose.closure_mmps - DOCK_GATE_CLOSURE_MAX_MMPS).max(0) as u32;
                let front_armor_damage =
                    COLLAR_SCRAPE_FRONT_ARMOR_DAMAGE.saturating_add(over_speed / 50);
                let rcs_spent_kg = stepper.rcs_spent_kg(&challenge.rcs_authority);
                return Ok(DockingReplayReport {
                    outcome: DockingOutcome::CollarScrapeBounce {
                        impact_closure_mmps: stepper.pose.closure_mmps,
                        front_armor_damage,
                    },
                    final_pose: stepper.pose,
                    final_gates: gates,
                    frames_replayed: stepper.frames_stepped,
                    rcs_spent_kg,
                });
            }
        }
    }

    let final_gates = evaluate_docking_gates(&stepper.pose);
    let rcs_spent_kg = stepper.rcs_spent_kg(&challenge.rcs_authority);
    let outcome = if final_gates.all_green() {
        DockingOutcome::Clamped
    } else {
        DockingOutcome::OutOfGate { gates: final_gates }
    };
    Ok(DockingReplayReport {
        outcome,
        final_pose: stepper.pose,
        final_gates,
        frames_replayed: stepper.frames_stepped,
        rcs_spent_kg,
    })
}

/// Parse a single text docking control order (`"DOCK TRANS UP"`, `"DOCK PITCH DOWN"`, etc.)
/// emitted by the web/TUI canopy bindings (`35a` §1 row 8).
#[must_use]
pub fn parse_dock_text_order(order: &str) -> Option<DockingControlInput> {
    let normalized = order.trim().to_ascii_uppercase();
    let tail = normalized
        .strip_prefix("DOCK ")
        .unwrap_or(normalized.as_str())
        .trim();
    let mut input = DockingControlInput::default();
    match tail {
        "TRANS UP" | "UP" => input.trans_y = 1,
        "TRANS DOWN" | "DOWN" => input.trans_y = -1,
        "TRANS LEFT" | "LEFT" => input.trans_x = -1,
        "TRANS RIGHT" | "RIGHT" => input.trans_x = 1,
        "CLOSURE FWD" | "CLOSURE FORWARD" | "FWD" | "FORWARD" => input.closure_pulse = 1,
        "CLOSURE BACK" | "CLOSURE REVERSE" | "BACK" | "REVERSE" | "AFT" => input.closure_pulse = -1,
        "PITCH UP" => input.pitch_trim = 1,
        "PITCH DOWN" => input.pitch_trim = -1,
        "YAW LEFT" => input.yaw_trim = -1,
        "YAW RIGHT" => input.yaw_trim = 1,
        "ROLL CCW" | "ROLL LEFT" => input.roll_trim = -1,
        "ROLL CW" | "ROLL RIGHT" => input.roll_trim = 1,
        "DAMP" | "DAMPENERS" | "TOGGLE DAMP" | "DAMPENERS TOGGLE" => input.toggle_dampeners = true,
        "ABORT" => input.abort = true,
        _ => return None,
    }
    Some(input)
}

/// Deterministic closed-loop guidance helper that computes a valid [`DockingInputTape`]
/// from `challenge.start_pose` into the 5-gate clamp envelope (`<= 3,600` frames).
#[must_use]
pub fn solve_docking_tape(challenge: &DockingChallenge) -> DockingInputTape {
    let mut stepper = DockingStepperState::new(challenge.start_pose.clone());
    let mut tape = DockingInputTape::default();
    let max_frames = challenge.max_frames.min(MAX_DOCKING_TAPE_FRAMES);

    for _ in 0..max_frames {
        let gates = evaluate_docking_gates(&stepper.pose);
        if gates.all_green() {
            break;
        }

        let mut input = DockingControlInput::default();

        // 1. Lateral X control: target `lateral_x_mm -> 0` with bounded approach speed `<= 180 mm/s`.
        let target_vx = if stepper.pose.lateral_x_mm > 120 {
            -180
        } else if stepper.pose.lateral_x_mm < -120 {
            180
        } else if stepper.pose.lateral_x_mm > 20 {
            -60
        } else if stepper.pose.lateral_x_mm < -20 {
            60
        } else {
            0
        };
        if stepper.pose.vel_x_mmps < target_vx - 15 {
            input.trans_x = 1;
        } else if stepper.pose.vel_x_mmps > target_vx + 15 {
            input.trans_x = -1;
        }

        // 2. Lateral Y control: target `lateral_y_mm -> 0` with bounded approach speed `<= 180 mm/s`.
        let target_vy = if stepper.pose.lateral_y_mm > 120 {
            -180
        } else if stepper.pose.lateral_y_mm < -120 {
            180
        } else if stepper.pose.lateral_y_mm > 20 {
            -60
        } else if stepper.pose.lateral_y_mm < -20 {
            60
        } else {
            0
        };
        if stepper.pose.vel_y_mmps < target_vy - 15 {
            input.trans_y = 1;
        } else if stepper.pose.vel_y_mmps > target_vy + 15 {
            input.trans_y = -1;
        }

        // 3. Pitch control: target `pitch_err_mdeg -> 0`.
        let target_pitch_rate = if stepper.pose.pitch_err_mdeg > 600 {
            -360
        } else if stepper.pose.pitch_err_mdeg < -600 {
            360
        } else if stepper.pose.pitch_err_mdeg > 90 {
            -180
        } else if stepper.pose.pitch_err_mdeg < -90 {
            180
        } else {
            challenge.target_tumble_mdeg_s[0]
        };
        if stepper.pose.pitch_rate_mdeg_s < target_pitch_rate - 90 {
            input.pitch_trim = 1;
        } else if stepper.pose.pitch_rate_mdeg_s > target_pitch_rate + 90 {
            input.pitch_trim = -1;
        }

        // 4. Yaw control: target `yaw_err_mdeg -> 0`.
        let target_yaw_rate = if stepper.pose.yaw_err_mdeg > 600 {
            -360
        } else if stepper.pose.yaw_err_mdeg < -600 {
            360
        } else if stepper.pose.yaw_err_mdeg > 90 {
            -180
        } else if stepper.pose.yaw_err_mdeg < -90 {
            180
        } else {
            challenge.target_tumble_mdeg_s[1]
        };
        if stepper.pose.yaw_rate_mdeg_s < target_yaw_rate - 90 {
            input.yaw_trim = 1;
        } else if stepper.pose.yaw_rate_mdeg_s > target_yaw_rate + 90 {
            input.yaw_trim = -1;
        }

        // 5. Roll control: target `roll_err_mdeg -> 0`.
        let target_roll_rate = if stepper.pose.roll_err_mdeg > 900 {
            -480
        } else if stepper.pose.roll_err_mdeg < -900 {
            480
        } else if stepper.pose.roll_err_mdeg > 120 {
            -240
        } else if stepper.pose.roll_err_mdeg < -120 {
            240
        } else {
            challenge.target_tumble_mdeg_s[2]
        };
        if stepper.pose.roll_rate_mdeg_s < target_roll_rate - 120 {
            input.roll_trim = 1;
        } else if stepper.pose.roll_rate_mdeg_s > target_roll_rate + 120 {
            input.roll_trim = -1;
        }

        // 6. Closure control: once lateral, attitude, and roll are aligned, maintain safe
        // closure rate (`350 mm/s`, squarely inside `100..=800 mm/s`).
        let cur_gates = evaluate_docking_gates(&stepper.pose);
        let target_closure = if cur_gates.lateral_ok && cur_gates.attitude_ok && cur_gates.roll_ok {
            350
        } else {
            0
        };
        if stepper.pose.closure_mmps < target_closure - 20 {
            input.closure_pulse = 1;
        } else if stepper.pose.closure_mmps > target_closure + 20 {
            input.closure_pulse = -1;
        }

        tape.push_frame(input);
        stepper.step_frame(
            input,
            &challenge.rcs_authority,
            challenge.target_tumble_mdeg_s,
        );
    }

    tape
}

/// Deterministic helper that generates a hard-impact [`DockingInputTape`] exceeding
/// `DOCK_GATE_CLOSURE_MAX_MMPS` (`> 800 mm/s`) into `range_z_mm == 0` (`D-35-14`).
#[must_use]
pub fn solve_hard_bounce_tape(challenge: &DockingChallenge) -> DockingInputTape {
    let mut stepper = DockingStepperState::new(challenge.start_pose.clone());
    let mut tape = DockingInputTape::default();
    let max_frames = challenge.max_frames.min(MAX_DOCKING_TAPE_FRAMES);
    let input = DockingControlInput {
        closure_pulse: 1,
        ..DockingControlInput::default()
    };
    for _ in 0..max_frames {
        tape.push_frame(input);
        stepper.step_frame(
            input,
            &challenge.rcs_authority,
            challenge.target_tumble_mdeg_s,
        );
        if stepper.pose.range_z_mm == 0 && stepper.pose.closure_mmps > DOCK_GATE_CLOSURE_MAX_MMPS {
            break;
        }
    }
    tape
}

/// Active tow-line record linking a tug hull to a hard-docked disabled derelict (`D-35-12`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalvageTowRecord {
    /// Tug hull object ID.
    pub tug_hull: ObjId,
    /// Disabled derelict hull object ID under tow.
    pub derelict_hull: ObjId,
    /// Tug dry mass in tonnes (`UnitSpec.logistics.dry_mass_t`).
    pub tug_dry_mass_t: u32,
    /// Derelict dry mass in tonnes (`UnitSpec.logistics.dry_mass_t`, must be `<= 2 * tug_dry_mass_t`).
    pub derelict_dry_mass_t: u32,
    /// Real wall-clock unix timestamp (`s`) when the 48-hour salvage claim window expires.
    pub contest_expires_real_s: i64,
}
