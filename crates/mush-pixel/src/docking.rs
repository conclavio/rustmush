//! **The Out-of-Time-Loop 6-DOF Docking Camera** (doc 35 cut SF.5, §8.3, Q-35-5, D-35-14).
//!
//! Renders the 60 fps client-side bow docking collar camera (`space.docking`
//! feed) while the docking hull is paused in a `DockingSession` outside the
//! world tick:
//! - 256-colour 4×4 Bayer-dithered station bulkhead / derelict hull backdrop
//! - True 6-DOF perspective geometric wireframe docking collar (outer octagonal
//!   ring, inner airlock seal, radial latches, top roll fiducial chevron, and
//!   three 3D approach corridor gates)
//! - Lateral Translation Gate Box (`±0.25 m` / `±250 mm`) with drift vector pip
//! - Pitch & Yaw alignment scales (`±2.0°` / `±2000 mdeg`)
//! - Roll alignment arc (`±3.0°` / `±3000 mdeg`, `Q`/`E`)
//! - Closure Velocity Tape (`0.1–0.8 m/s` / `100–800 mm/s` green latch zone, `R`/`F`)
//! - Non-colliding key legend (`WASD` trans, `Arrows` pitch/yaw, `Q/E` roll,
//!   `R/F` closure, `X` dampeners — no bare `Shift` or digit keys, D-35-16).

use std::time::Duration;

use serde_json::Value;

use crate::Frame;
use crate::frame::Rgb;
use crate::math::Det as _;
use crate::overlay::{
    BAD, BORDER, DIM, GOOD, HOSTILE, LOCK, PANEL, TEXT, WARN, YOU, draw_card, fill_rect, line,
    outline_rect, scale_for, text,
};
use crate::picture::{BUCKET_A_INTERVAL, Feeds, Look, Picture};
use crate::ship_shader::draw_docking_hull_wireframe;

/// The 6-DOF bow docking camera feed.
pub const SPACE_DOCKING_FEED: &str = "space.docking";

/// Horizontal field of view of the bow docking camera, degrees.
pub const DOCKING_FOV_DEG: f64 = 60.0;

/// Lateral alignment gate tolerance in millimetres (`±0.25 m`).
pub const GATE_LATERAL_MM: i64 = 250;
/// Pitch and yaw alignment gate tolerance in millidegrees (`±2.0°`).
pub const GATE_ATTITUDE_MDEG: i64 = 2_000;
/// Roll alignment gate tolerance in millidegrees (`±3.0°`).
pub const GATE_ROLL_MDEG: i64 = 3_000;
/// Minimum positive closure rate for magnetic latch (`0.10 m/s`).
pub const GATE_CLOSURE_MIN_MMPS: i64 = 100;
/// Maximum safe closure rate before collar scrape/bounce (`0.80 m/s`).
pub const GATE_CLOSURE_MAX_MMPS: i64 = 800;

const DEEP_SPACE: Rgb = [4, 6, 12];
const BULKHEAD_DARK: Rgb = [18, 22, 30];
const BULKHEAD_MID: Rgb = [32, 38, 50];
const BULKHEAD_LIT: Rgb = [52, 62, 78];
const COLLAR_CAVITY: Rgb = [8, 10, 16];
const CHEVRON_CYAN: Rgb = [72, 188, 232];
const STROBE_BRIGHT: Rgb = [236, 244, 255];

#[cfg(feature = "encode")]
use crate::overhead::{place_kitty_subrect, wrap_kitty_sync};

/// Retained Kitty image ID (`i=188`) for the 6-DOF Docking Camera picture.
#[cfg(feature = "encode")]
pub const IMAGE_ID_DOCKING: u32 = 188;
/// Retained Kitty placement ID (`p=88`) for the 6-DOF Docking Camera picture.
#[cfg(feature = "encode")]
pub const PLACEMENT_DOCKING: u32 = 88;
#[cfg(feature = "encode")]
const Z_DOCKING: i32 = -50;

/// The Out-of-Time-Loop 6-DOF Docking Camera as a [`Picture`].
#[derive(Debug, Default)]
pub struct DockingPicture {
    #[cfg(feature = "encode")]
    kitty_hash: Option<u64>,
    #[cfg(feature = "encode")]
    kitty_area: Option<(u16, u16, u16, u16)>,
}

impl DockingPicture {
    /// A new 6-DOF Docking Camera picture.
    #[must_use]
    pub fn new() -> DockingPicture {
        DockingPicture::default()
    }

    /// Return Kitty Graphics escape bytes that delete the retained image.
    #[cfg(feature = "encode")]
    pub fn cleanup_kitty_bytes(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.kitty_hash.is_some() {
            crate::kitty::delete_image_bytes(&mut out, IMAGE_ID_DOCKING);
        }
        self.kitty_hash = None;
        self.kitty_area = None;
        out
    }

    /// Invalidate the retained Kitty image if the terminal evicted it.
    #[cfg(feature = "encode")]
    pub fn invalidate_image(&mut self, image_id: u32) {
        if image_id == IMAGE_ID_DOCKING {
            self.kitty_hash = None;
            self.kitty_area = None;
        }
    }

    /// Force the retained Kitty placement to re-emit on the next tick.
    #[cfg(feature = "encode")]
    pub fn invalidate_placements(&mut self) {
        self.kitty_area = None;
    }

    /// Render the 6-DOF Docking Camera as a retained Kitty Graphics stream (`a=t` + `a=p`).
    #[cfg(feature = "encode")]
    pub fn draw_kitty(
        &mut self,
        feeds: &dyn Feeds,
        look: &Look,
        area: (u16, u16, u16, u16),
        cell: Option<crate::caps::CellSize>,
        step_down: bool,
    ) -> Option<(crate::iso25d::Viewport25D, Vec<u8>)> {
        let vp = crate::cockpit_kitty::viewport_fp(area.2, area.3, cell, step_down);
        let (vw, vh) = (vp.width_px, vp.height_px);
        let mut frame = Frame::new(vw, vh);
        if !self.draw(&mut frame, feeds, look) {
            return None;
        }
        let placed_area = (
            area.0.saturating_add(vp.col_offset),
            area.1.saturating_add(vp.row_offset),
            vp.cols,
            vp.rows,
        );
        let h = frame.hash();
        let mut ops = Vec::new();
        if self.kitty_hash != Some(h) {
            let png = crate::png::encode(&frame);
            crate::kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_DOCKING);
            self.kitty_hash = Some(h);
            self.kitty_area = None;
        }
        if self.kitty_area != Some(placed_area) {
            place_kitty_subrect(
                &mut ops,
                IMAGE_ID_DOCKING,
                PLACEMENT_DOCKING,
                placed_area,
                (0, 0, vw as u32, vh as u32),
                Z_DOCKING,
            );
            self.kitty_area = Some(placed_area);
        }
        Some((vp, wrap_kitty_sync(ops)))
    }
}

impl Picture for DockingPicture {
    fn feeds(&self) -> &'static [&'static str] {
        &[SPACE_DOCKING_FEED]
    }

    fn interval(&self) -> Duration {
        BUCKET_A_INTERVAL
    }

    fn draw(&mut self, frame: &mut Frame, feeds: &dyn Feeds, look: &Look) -> bool {
        let Some(docking) = feeds.feed(SPACE_DOCKING_FEED) else {
            return false;
        };
        if docking.get("refused").and_then(Value::as_bool) == Some(true) {
            draw_card(frame, "NO DOCKING CAMERA");
            return true;
        }
        draw_docking_camera(frame, docking, look);
        true
    }
}

/// 6-DOF pose and velocity state parsed from the `space.docking` feed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DockingCameraPose {
    /// Lateral horizontal offset from collar centerline in mm (`+` = right).
    pub dx_mm: i64,
    /// Vertical offset from collar centerline in mm (`+` = up).
    pub dy_mm: i64,
    /// Axial distance from bow collar to target collar in mm (`>= 0`).
    pub dz_mm: i64,
    /// Pitch misalignment in millidegrees (`+` = nose up).
    pub pitch_mdeg: i64,
    /// Yaw misalignment in millidegrees (`+` = nose right).
    pub yaw_mdeg: i64,
    /// Roll misalignment in millidegrees (`+` = clockwise).
    pub roll_mdeg: i64,
    /// Lateral horizontal drift rate in mm/s.
    pub vx_mmps: i64,
    /// Vertical drift rate in mm/s.
    pub vy_mmps: i64,
    /// Closure rate toward the collar in mm/s (`+` = closing).
    pub vz_mmps: i64,
}

impl DockingCameraPose {
    /// Whether lateral offset `(dx_mm, dy_mm)` is within the `±250 mm` gate.
    #[must_use]
    pub const fn lateral_aligned(&self) -> bool {
        self.dx_mm.abs() <= GATE_LATERAL_MM && self.dy_mm.abs() <= GATE_LATERAL_MM
    }

    /// Whether pitch and yaw are within the `±2.0°` (`±2000 mdeg`) gate.
    #[must_use]
    pub const fn attitude_aligned(&self) -> bool {
        self.pitch_mdeg.abs() <= GATE_ATTITUDE_MDEG && self.yaw_mdeg.abs() <= GATE_ATTITUDE_MDEG
    }

    /// Whether roll is within the `±3.0°` (`±3000 mdeg`) gate.
    #[must_use]
    pub const fn roll_aligned(&self) -> bool {
        self.roll_mdeg.abs() <= GATE_ROLL_MDEG
    }

    /// Whether closure rate is within the `0.10..=0.80 m/s` latch gate.
    #[must_use]
    pub const fn closure_safe(&self) -> bool {
        self.vz_mmps >= GATE_CLOSURE_MIN_MMPS && self.vz_mmps <= GATE_CLOSURE_MAX_MMPS
    }

    /// Whether all spatial and angular alignment gates are satisfied.
    #[must_use]
    pub const fn pose_aligned(&self) -> bool {
        self.lateral_aligned() && self.attitude_aligned() && self.roll_aligned()
    }

    /// Whether all pose gates and closure rate are ready for clamp lock.
    #[must_use]
    pub const fn ready_to_clamp(&self) -> bool {
        self.pose_aligned() && self.closure_safe()
    }
}

/// Maximum number of 60 Hz frames in a client docking input tape (`3,600` frames = `60 s`, `D-35-14`).
pub const MAX_DOCKING_CLIENT_TAPE_FRAMES: usize = 3_600;

/// Client-side 60 Hz 6-DOF control input for a single frame (`Q-35-5`, `D-35-14`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DockingClientInput {
    /// Lateral horizontal RCS pulse (`-1`, `0`, `+1`).
    pub trans_x: i8,
    /// Lateral vertical RCS pulse (`-1`, `0`, `+1`).
    pub trans_y: i8,
    /// Forward/aft closure RCS pulse (`-1`, `0`, `+1`).
    pub closure_pulse: i8,
    /// Pitch trim pulse (`-1`, `0`, `+1`).
    pub pitch_trim: i8,
    /// Yaw trim pulse (`-1`, `0`, `+1`).
    pub yaw_trim: i8,
    /// Roll trim pulse (`-1`, `0`, `+1`).
    pub roll_trim: i8,
    /// Toggle coupled RCS rotational/lateral dampeners (`X` / `Space`).
    pub toggle_dampeners: bool,
    /// Abort the docking session (`Esc`).
    pub abort: bool,
}

impl DockingClientInput {
    const fn encode_axis(val: i8) -> u16 {
        if val > 0 {
            0b01
        } else if val < 0 {
            0b10
        } else {
            0b00
        }
    }

    const fn decode_axis(bits: u16) -> i8 {
        match bits & 0b11 {
            0b01 => 1,
            0b10 => -1,
            _ => 0,
        }
    }

    /// Pack this frame's control inputs into the 16-bit RLE wire bitmask (`D-35-14`).
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

    /// Unpack a 16-bit RLE wire bitmask into a [`DockingClientInput`] (`D-35-14`).
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

/// Per-hull 6-DOF RCS thruster authority parameters for the 60 Hz client docking simulation (`SF.R16`, `D-35-14`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DockingClientRcsAuthority {
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
}

impl Default for DockingClientRcsAuthority {
    fn default() -> Self {
        Self {
            trans_dv_mmps_per_frame: 30,
            closure_dv_mmps_per_frame: 25,
            attitude_rate_mdeg_s_per_frame: 180,
            roll_rate_mdeg_s_per_frame: 240,
            damp_trans_mmps_per_frame: 15,
            damp_attitude_mdeg_s_per_frame: 90,
            damp_roll_mdeg_s_per_frame: 120,
        }
    }
}

const fn bleed_i64(val: i64, step: i64) -> i64 {
    if val > step {
        val - step
    } else if val < -step {
        val + step
    } else {
        0
    }
}

const fn bleed_i32(val: i32, step: i32) -> i32 {
    if val > step {
        val - step
    } else if val < -step {
        val + step
    } else {
        0
    }
}

/// Shared 60 Hz 6-DOF integer kinematics integrator (`D-35-14`, `SF.R24`).
///
/// Steps `pose`, angular `rates_mdeg_s` (`[pitch, yaw, roll]`), and modulo-60 sub-frame
/// remainders (`rem_pos_mm`, `rem_att_mdeg`) by one `16.67 ms` frame and returns the
/// number of active RCS axis pulses fired during the frame (including coupled dampeners).
/// Compiled into `tacsim`, the SSH TUI (`mush-net-ssh`), and the browser WASM (`mush-pixel-wasm`)
/// as the single 6-DOF docking kinematics implementation.
#[allow(clippy::too_many_arguments)]
pub fn step_docking_kinematics_60fps(
    pose: &mut DockingCameraPose,
    rates_mdeg_s: &mut [i32; 3],
    rem_pos_mm: &mut [i64; 3],
    rem_att_mdeg: &mut [i32; 3],
    dampeners_on: &mut bool,
    input: DockingClientInput,
    auth: &DockingClientRcsAuthority,
    target_tumble_mdeg_s: [i32; 3],
) -> u32 {
    if input.toggle_dampeners {
        *dampeners_on = !*dampeners_on;
    }

    let tx = i64::from(input.trans_x.clamp(-1, 1));
    let ty = i64::from(input.trans_y.clamp(-1, 1));
    let tz = i64::from(input.closure_pulse.clamp(-1, 1));
    let dpitch = i32::from(input.pitch_trim.clamp(-1, 1));
    let dyaw = i32::from(input.yaw_trim.clamp(-1, 1));
    let droll = i32::from(input.roll_trim.clamp(-1, 1));

    pose.vx_mmps = pose
        .vx_mmps
        .saturating_add(tx.saturating_mul(auth.trans_dv_mmps_per_frame));
    pose.vy_mmps = pose
        .vy_mmps
        .saturating_add(ty.saturating_mul(auth.trans_dv_mmps_per_frame));
    pose.vz_mmps = pose
        .vz_mmps
        .saturating_add(tz.saturating_mul(auth.closure_dv_mmps_per_frame));
    rates_mdeg_s[0] =
        rates_mdeg_s[0].saturating_add(dpitch.saturating_mul(auth.attitude_rate_mdeg_s_per_frame));
    rates_mdeg_s[1] =
        rates_mdeg_s[1].saturating_add(dyaw.saturating_mul(auth.attitude_rate_mdeg_s_per_frame));
    rates_mdeg_s[2] =
        rates_mdeg_s[2].saturating_add(droll.saturating_mul(auth.roll_rate_mdeg_s_per_frame));

    let mut frame_pulses = input.active_axis_pulses();

    if *dampeners_on {
        if tx == 0 && pose.vx_mmps != 0 {
            pose.vx_mmps = bleed_i64(pose.vx_mmps, auth.damp_trans_mmps_per_frame);
            frame_pulses = frame_pulses.saturating_add(1);
        }
        if ty == 0 && pose.vy_mmps != 0 {
            pose.vy_mmps = bleed_i64(pose.vy_mmps, auth.damp_trans_mmps_per_frame);
            frame_pulses = frame_pulses.saturating_add(1);
        }
        if dpitch == 0 && rates_mdeg_s[0] != 0 {
            rates_mdeg_s[0] = bleed_i32(rates_mdeg_s[0], auth.damp_attitude_mdeg_s_per_frame);
            frame_pulses = frame_pulses.saturating_add(1);
        }
        if dyaw == 0 && rates_mdeg_s[1] != 0 {
            rates_mdeg_s[1] = bleed_i32(rates_mdeg_s[1], auth.damp_attitude_mdeg_s_per_frame);
            frame_pulses = frame_pulses.saturating_add(1);
        }
        if droll == 0 && rates_mdeg_s[2] != 0 {
            rates_mdeg_s[2] = bleed_i32(rates_mdeg_s[2], auth.damp_roll_mdeg_s_per_frame);
            frame_pulses = frame_pulses.saturating_add(1);
        }
    }

    let tot_x = pose.vx_mmps.saturating_add(rem_pos_mm[0]);
    pose.dx_mm = pose.dx_mm.saturating_add(tot_x / 60);
    rem_pos_mm[0] = tot_x % 60;

    let tot_y = pose.vy_mmps.saturating_add(rem_pos_mm[1]);
    pose.dy_mm = pose.dy_mm.saturating_add(tot_y / 60);
    rem_pos_mm[1] = tot_y % 60;

    let tot_z = pose.vz_mmps.saturating_add(rem_pos_mm[2]);
    pose.dz_mm = pose.dz_mm.saturating_sub(tot_z / 60).max(0);
    rem_pos_mm[2] = tot_z % 60;

    let net_pitch = rates_mdeg_s[0].saturating_sub(target_tumble_mdeg_s[0]);
    let tot_pitch = net_pitch.saturating_add(rem_att_mdeg[0]);
    pose.pitch_mdeg = pose.pitch_mdeg.saturating_add(i64::from(tot_pitch / 60));
    rem_att_mdeg[0] = tot_pitch % 60;

    let net_yaw = rates_mdeg_s[1].saturating_sub(target_tumble_mdeg_s[1]);
    let tot_yaw = net_yaw.saturating_add(rem_att_mdeg[1]);
    pose.yaw_mdeg = pose.yaw_mdeg.saturating_add(i64::from(tot_yaw / 60));
    rem_att_mdeg[1] = tot_yaw % 60;

    let net_roll = rates_mdeg_s[2].saturating_sub(target_tumble_mdeg_s[2]);
    let tot_roll = net_roll.saturating_add(rem_att_mdeg[2]);
    pose.roll_mdeg = pose.roll_mdeg.saturating_add(i64::from(tot_roll / 60));
    rem_att_mdeg[2] = tot_roll % 60;

    frame_pulses
}

/// Client-side 60 Hz 6-DOF docking session stepper and RLE input-tape recorder (`Q-35-5`, `D-35-14`, `SF.R16`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockingClientSession {
    /// Server-issued session ID (`"dock.<hull>.<seq>"`).
    pub session_id: String,
    /// Live 6-DOF collar pose stepped locally at 60 Hz.
    pub pose: DockingCameraPose,
    /// Pitch angular rate in `mdeg/s`.
    pub pitch_rate_mdeg_s: i32,
    /// Yaw angular rate in `mdeg/s`.
    pub yaw_rate_mdeg_s: i32,
    /// Roll angular rate in `mdeg/s`.
    pub roll_rate_mdeg_s: i32,
    /// Target collar tumble rate `[pitch_mdeg_s, yaw_mdeg_s, roll_mdeg_s]`.
    pub target_tumble_mdeg_s: [i32; 3],
    /// Hull-specific 6-DOF RCS thruster authority (`SF.R16`).
    pub rcs_authority: DockingClientRcsAuthority,
    /// Whether coupled RCS dampeners are enabled.
    pub dampeners_on: bool,
    rem_x: i64,
    rem_y: i64,
    rem_z: i64,
    rem_pitch: i32,
    rem_yaw: i32,
    rem_roll: i32,
    spans: Vec<(u16, u16)>,
    frames_stepped: usize,
}

impl DockingClientSession {
    /// Initialize a client-side 60 Hz docking session from a `space.docking` feed JSON value.
    #[must_use]
    pub fn from_feed(docking: &Value) -> Self {
        let session_id = docking
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or("dock.0.1")
            .to_owned();
        let tumble = docking
            .get("target_tumble_mdeg_s")
            .and_then(Value::as_array)
            .map_or([0, 0, 0], |arr| {
                let p = arr.first().and_then(Value::as_i64).unwrap_or(0) as i32;
                let y = arr.get(1).and_then(Value::as_i64).unwrap_or(0) as i32;
                let r = arr.get(2).and_then(Value::as_i64).unwrap_or(0) as i32;
                [p, y, r]
            });
        let rcs_authority =
            docking
                .get("rcs_authority")
                .map_or_else(DockingClientRcsAuthority::default, |auth| {
                    DockingClientRcsAuthority {
                        trans_dv_mmps_per_frame: int_field(auth, "trans_dv_mmps_per_frame", 30),
                        closure_dv_mmps_per_frame: int_field(auth, "closure_dv_mmps_per_frame", 25),
                        attitude_rate_mdeg_s_per_frame: int_field(
                            auth,
                            "attitude_rate_mdeg_s_per_frame",
                            180,
                        ) as i32,
                        roll_rate_mdeg_s_per_frame: int_field(
                            auth,
                            "roll_rate_mdeg_s_per_frame",
                            240,
                        ) as i32,
                        damp_trans_mmps_per_frame: int_field(auth, "damp_trans_mmps_per_frame", 15),
                        damp_attitude_mdeg_s_per_frame: int_field(
                            auth,
                            "damp_attitude_mdeg_s_per_frame",
                            90,
                        ) as i32,
                        damp_roll_mdeg_s_per_frame: int_field(
                            auth,
                            "damp_roll_mdeg_s_per_frame",
                            120,
                        ) as i32,
                    }
                });
        Self {
            session_id,
            pose: DockingCameraPose {
                dx_mm: int_field(docking, "dx_mm", 600),
                dy_mm: int_field(docking, "dy_mm", -420),
                dz_mm: int_field(docking, "dz_mm", 2_400),
                pitch_mdeg: int_field(docking, "pitch_mdeg", -2_400),
                yaw_mdeg: int_field(docking, "yaw_mdeg", 2_400),
                roll_mdeg: int_field(docking, "roll_mdeg", -3_600),
                vx_mmps: int_field(docking, "vx_mmps", 0),
                vy_mmps: int_field(docking, "vy_mmps", 0),
                vz_mmps: int_field(docking, "vz_mmps", 0),
            },
            pitch_rate_mdeg_s: int_field(docking, "pitch_rate_mdeg_s", 0) as i32,
            yaw_rate_mdeg_s: int_field(docking, "yaw_rate_mdeg_s", 0) as i32,
            roll_rate_mdeg_s: int_field(docking, "roll_rate_mdeg_s", 0) as i32,
            target_tumble_mdeg_s: tumble,
            rcs_authority,
            dampeners_on: docking
                .get("dampeners")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            rem_x: 0,
            rem_y: 0,
            rem_z: 0,
            rem_pitch: 0,
            rem_yaw: 0,
            rem_roll: 0,
            spans: Vec::new(),
            frames_stepped: 0,
        }
    }

    /// Return the total number of 60 Hz frames stepped in this client session.
    #[must_use]
    pub const fn frames_stepped(&self) -> usize {
        self.frames_stepped
    }

    /// Step the local 6-DOF collar pose by one 60 Hz frame (`16.67 ms`) and append `input`
    /// to the bounded RLE tape (`<= 3,600` frames).
    pub fn step_60fps(&mut self, input: DockingClientInput) {
        if self.frames_stepped >= MAX_DOCKING_CLIENT_TAPE_FRAMES {
            return;
        }
        let mask = input.to_bitmask();
        if let Some(last) = self.spans.last_mut()
            && last.0 == mask
            && last.1 < u16::MAX
        {
            last.1 = last.1.saturating_add(1);
        } else {
            self.spans.push((mask, 1));
        }

        let mut rates = [
            self.pitch_rate_mdeg_s,
            self.yaw_rate_mdeg_s,
            self.roll_rate_mdeg_s,
        ];
        let mut rem_pos = [self.rem_x, self.rem_y, self.rem_z];
        let mut rem_att = [self.rem_pitch, self.rem_yaw, self.rem_roll];
        let auth = self.rcs_authority;
        let _ = step_docking_kinematics_60fps(
            &mut self.pose,
            &mut rates,
            &mut rem_pos,
            &mut rem_att,
            &mut self.dampeners_on,
            input,
            &auth,
            self.target_tumble_mdeg_s,
        );
        self.pitch_rate_mdeg_s = rates[0];
        self.yaw_rate_mdeg_s = rates[1];
        self.roll_rate_mdeg_s = rates[2];
        self.rem_x = rem_pos[0];
        self.rem_y = rem_pos[1];
        self.rem_z = rem_pos[2];
        self.rem_pitch = rem_att[0];
        self.rem_yaw = rem_att[1];
        self.rem_roll = rem_att[2];

        self.frames_stepped = self.frames_stepped.saturating_add(1);
    }

    /// Return the number of currently satisfied alignment gates (`0..=5`).
    #[must_use]
    pub const fn gates_green(&self) -> u8 {
        let mut gates_green = 0u8;
        if self.pose.lateral_aligned()
            && self.pose.vx_mmps.abs() <= 300
            && self.pose.vy_mmps.abs() <= 300
        {
            gates_green += 1;
        }
        if self.pose.attitude_aligned()
            && self.pitch_rate_mdeg_s.abs() <= 500
            && self.yaw_rate_mdeg_s.abs() <= 500
        {
            gates_green += 1;
        }
        if self.pose.roll_aligned() && self.roll_rate_mdeg_s.abs() <= 500 {
            gates_green += 1;
        }
        if self.pose.closure_safe() {
            gates_green += 1;
        }
        if self.pose.dz_mm >= 0 && self.pose.dz_mm <= 500 {
            gates_green += 1;
        }
        gates_green
    }

    /// Return whether all 5 clamp alignment gates (including `dz_mm <= 500`) are satisfied.
    #[must_use]
    pub const fn clamped(&self) -> bool {
        self.pose.dz_mm <= 500
            && self.pose.ready_to_clamp()
            && self.pose.vx_mmps.abs() <= 300
            && self.pose.vy_mmps.abs() <= 300
            && self.pitch_rate_mdeg_s.abs() <= 500
            && self.yaw_rate_mdeg_s.abs() <= 500
            && self.roll_rate_mdeg_s.abs() <= 500
    }

    /// Compute the next 60 Hz control input toward the 5-gate clamp envelope using this hull's
    /// [`DockingClientRcsAuthority`] (`SF.R16`).
    ///
    /// Behind the `autodock` feature since doc 35 cut `SF.R31`: an auto-solver, for tests only.
    #[cfg(any(test, feature = "autodock"))]
    #[must_use]
    pub fn next_guidance_input(&self) -> DockingClientInput {
        let mut input = DockingClientInput::default();
        let dv_trans = self.rcs_authority.trans_dv_mmps_per_frame.max(5);
        let dv_close = self.rcs_authority.closure_dv_mmps_per_frame.max(5);
        let d_att = self.rcs_authority.attitude_rate_mdeg_s_per_frame.max(30);
        let d_roll = self.rcs_authority.roll_rate_mdeg_s_per_frame.max(30);

        let fast_vx = (dv_trans * 6).min(240);
        let slow_vx = (dv_trans * 2).min(120);
        let dead_vx = (dv_trans / 2).max(5);
        let target_vx = if self.pose.dx_mm > 120 {
            -fast_vx
        } else if self.pose.dx_mm < -120 {
            fast_vx
        } else if self.pose.dx_mm > 20 {
            -slow_vx
        } else if self.pose.dx_mm < -20 {
            slow_vx
        } else {
            0
        };
        if self.pose.vx_mmps < target_vx - dead_vx {
            input.trans_x = 1;
        } else if self.pose.vx_mmps > target_vx + dead_vx {
            input.trans_x = -1;
        }

        let target_vy = if self.pose.dy_mm > 120 {
            -fast_vx
        } else if self.pose.dy_mm < -120 {
            fast_vx
        } else if self.pose.dy_mm > 20 {
            -slow_vx
        } else if self.pose.dy_mm < -20 {
            slow_vx
        } else {
            0
        };
        if self.pose.vy_mmps < target_vy - dead_vx {
            input.trans_y = 1;
        } else if self.pose.vy_mmps > target_vy + dead_vx {
            input.trans_y = -1;
        }

        let fast_att = (d_att * 2).min(420);
        let slow_att = d_att.min(240);
        let dead_att = (d_att / 2).max(15);
        let target_pitch_rate = if self.pose.pitch_mdeg > 600 {
            -fast_att
        } else if self.pose.pitch_mdeg < -600 {
            fast_att
        } else if self.pose.pitch_mdeg > 90 {
            -slow_att
        } else if self.pose.pitch_mdeg < -90 {
            slow_att
        } else {
            self.target_tumble_mdeg_s[0]
        };
        if self.pitch_rate_mdeg_s < target_pitch_rate - dead_att {
            input.pitch_trim = 1;
        } else if self.pitch_rate_mdeg_s > target_pitch_rate + dead_att {
            input.pitch_trim = -1;
        }

        let target_yaw_rate = if self.pose.yaw_mdeg > 600 {
            -fast_att
        } else if self.pose.yaw_mdeg < -600 {
            fast_att
        } else if self.pose.yaw_mdeg > 90 {
            -slow_att
        } else if self.pose.yaw_mdeg < -90 {
            slow_att
        } else {
            self.target_tumble_mdeg_s[1]
        };
        if self.yaw_rate_mdeg_s < target_yaw_rate - dead_att {
            input.yaw_trim = 1;
        } else if self.yaw_rate_mdeg_s > target_yaw_rate + dead_att {
            input.yaw_trim = -1;
        }

        let fast_roll = (d_roll * 2).min(480);
        let slow_roll = d_roll.min(240);
        let dead_roll = (d_roll / 2).max(15);
        let target_roll_rate = if self.pose.roll_mdeg > 900 {
            -fast_roll
        } else if self.pose.roll_mdeg < -900 {
            fast_roll
        } else if self.pose.roll_mdeg > 120 {
            -slow_roll
        } else if self.pose.roll_mdeg < -120 {
            slow_roll
        } else {
            self.target_tumble_mdeg_s[2]
        };
        if self.roll_rate_mdeg_s < target_roll_rate - dead_roll {
            input.roll_trim = 1;
        } else if self.roll_rate_mdeg_s > target_roll_rate + dead_roll {
            input.roll_trim = -1;
        }

        let lat_ok = self.pose.lateral_aligned()
            && self.pose.vx_mmps.abs() <= 300
            && self.pose.vy_mmps.abs() <= 300;
        let att_ok = self.pose.attitude_aligned()
            && self.pitch_rate_mdeg_s.abs() <= 500
            && self.yaw_rate_mdeg_s.abs() <= 500;
        let roll_ok = self.pose.roll_aligned() && self.roll_rate_mdeg_s.abs() <= 500;
        let target_closure = if lat_ok && att_ok && roll_ok { 350 } else { 0 };
        let dead_close = dv_close.clamp(5, 20);
        if self.pose.vz_mmps < target_closure - dead_close {
            input.closure_pulse = 1;
        } else if self.pose.vz_mmps > target_closure + dead_close {
            input.closure_pulse = -1;
        }

        input
    }

    /// Play the client 60 Hz docking session forward until `clamped()` is reached (up to 3,600 frames).
    ///
    /// Behind the `autodock` feature since doc 35 cut `SF.R31`: an auto-solver, for tests only.
    #[cfg(any(test, feature = "autodock"))]
    pub fn play_to_clamp(&mut self) {
        while !self.clamped() && self.frames_stepped < MAX_DOCKING_CLIENT_TAPE_FRAMES {
            let input = self.next_guidance_input();
            self.step_60fps(input);
        }
    }

    /// Overlay the live client-stepped 6-DOF pose onto a `space.docking` feed JSON object (`SF.R16`).
    #[must_use]
    pub fn to_feed_value(&self, base: &Value) -> Value {
        let mut obj = base.as_object().cloned().unwrap_or_default();
        obj.insert("dx_mm".to_owned(), Value::from(self.pose.dx_mm));
        obj.insert("dy_mm".to_owned(), Value::from(self.pose.dy_mm));
        obj.insert("dz_mm".to_owned(), Value::from(self.pose.dz_mm));
        obj.insert("pitch_mdeg".to_owned(), Value::from(self.pose.pitch_mdeg));
        obj.insert("yaw_mdeg".to_owned(), Value::from(self.pose.yaw_mdeg));
        obj.insert("roll_mdeg".to_owned(), Value::from(self.pose.roll_mdeg));
        obj.insert("vx_mmps".to_owned(), Value::from(self.pose.vx_mmps));
        obj.insert("vy_mmps".to_owned(), Value::from(self.pose.vy_mmps));
        obj.insert("vz_mmps".to_owned(), Value::from(self.pose.vz_mmps));
        obj.insert(
            "pitch_rate_mdeg_s".to_owned(),
            Value::from(self.pitch_rate_mdeg_s),
        );
        obj.insert(
            "yaw_rate_mdeg_s".to_owned(),
            Value::from(self.yaw_rate_mdeg_s),
        );
        obj.insert(
            "roll_rate_mdeg_s".to_owned(),
            Value::from(self.roll_rate_mdeg_s),
        );
        let gates_green = self.gates_green();
        obj.insert("dampeners".to_owned(), Value::from(self.dampeners_on));
        obj.insert("gates_green".to_owned(), Value::from(gates_green));
        obj.insert(
            "status".to_owned(),
            Value::from(if self.clamped() {
                "ALIGNED-READY"
            } else {
                "APPROACHING"
            }),
        );
        Value::Object(obj)
    }

    /// Encode the recorded 60 Hz input spans into the RLE wire string (`"<hex_mask>*<frames>,..."`).
    #[must_use]
    pub fn encode_rle(&self) -> String {
        self.spans
            .iter()
            .map(|(mask, count)| format!("{mask:04x}*{count}"))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Format the server verification command (`"+helm/dock tape <session_id> <rle_tape>"`).
    #[must_use]
    pub fn format_tape_command(&self) -> String {
        format!("+helm/dock tape {} {}", self.session_id, self.encode_rle())
    }
}

fn int_field(v: &Value, key: &str, default: i64) -> i64 {
    v.get(key)
        .and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f.round() as i64)))
        .unwrap_or(default)
}

fn bayer4(x: i64, y: i64) -> u8 {
    let bx = (x.rem_euclid(4)) as u8;
    let by = (y.rem_euclid(4)) as u8;
    match (by, bx) {
        (0, 0) => 0,
        (0, 1) => 8,
        (0, 2) => 2,
        (0, 3) => 10,
        (1, 0) => 12,
        (1, 1) => 4,
        (1, 2) => 14,
        (1, 3) => 6,
        (2, 0) => 3,
        (2, 1) => 11,
        (2, 2) => 1,
        (2, 3) => 9,
        (3, 0) => 15,
        (3, 1) => 7,
        (3, 2) => 13,
        _ => 5,
    }
}

/// Project a 3D point `(x_mm, y_mm, z_mm)` in the target docking collar's frame
/// (where `(0, 0, 0)` is the collar center and `-z_mm` extends toward the
/// approaching ship) onto screen pixels `(px, py)`.
#[must_use]
pub fn project_collar_point(
    (w, h): (i64, i64),
    pose: &DockingCameraPose,
    (x_mm, y_mm, z_mm): (f64, f64, f64),
    fov_deg: f64,
) -> Option<(i64, i64)> {
    // Vector from bow camera at (dx_mm, dy_mm, -dz_mm) to collar vertex (x_mm, y_mm, z_mm).
    let rx = x_mm - (pose.dx_mm as f64);
    let ry = y_mm - (pose.dy_mm as f64);
    let rz = (pose.dz_mm.max(400) as f64) + z_mm;

    // Rotate by inverse of ship attitude misalignment (yaw, pitch, roll).
    let yaw_rad = -(pose.yaw_mdeg as f64) / 1_000.0 * std::f64::consts::PI / 180.0;
    let pitch_rad = -(pose.pitch_mdeg as f64) / 1_000.0 * std::f64::consts::PI / 180.0;
    let roll_rad = -(pose.roll_mdeg as f64) / 1_000.0 * std::f64::consts::PI / 180.0;

    let (sy, cy) = yaw_rad.det_sin_cos();
    let x1 = rx * cy + rz * sy;
    let z1 = -rx * sy + rz * cy;
    let y1 = ry;

    let (sp, cp) = pitch_rad.det_sin_cos();
    let y2 = y1 * cp - z1 * sp;
    let z2 = y1 * sp + z1 * cp;
    let x2 = x1;

    if z2 <= 150.0 {
        return None;
    }

    let (sr, cr) = roll_rad.det_sin_cos();
    let x3 = x2 * cr - y2 * sr;
    let y3 = x2 * sr + y2 * cr;

    let half_fov_rad = (fov_deg.clamp(20.0, 110.0) * 0.5).to_radians();
    let (s_fov, c_fov) = half_fov_rad.det_sin_cos();
    let focal = (w.min(h) as f64) * 0.5 * (c_fov / s_fov.max(0.1));

    let sx = (w / 2) + ((x3 / z2) * focal).round() as i64;
    let sy_px = (h / 2) - ((y3 / z2) * focal).round() as i64;
    if sx < -w || sx > 2 * w || sy_px < -h || sy_px > 2 * h {
        return None;
    }
    Some((sx, sy_px))
}

/// Render the full Out-of-Time-Loop 6-DOF Docking Camera into `frame`.
pub fn draw_docking_camera(frame: &mut Frame, docking: &Value, look: &Look) {
    let sc = scale_for(frame);
    let s = sc as i64;
    frame.clear(DEEP_SPACE);

    let pose = DockingCameraPose {
        dx_mm: int_field(docking, "dx_mm", 0),
        dy_mm: int_field(docking, "dy_mm", 0),
        dz_mm: int_field(docking, "dz_mm", 12_000).max(200),
        pitch_mdeg: int_field(docking, "pitch_mdeg", 0),
        yaw_mdeg: int_field(docking, "yaw_mdeg", 0) + look.offset_mdeg,
        roll_mdeg: int_field(docking, "roll_mdeg", 0),
        vx_mmps: int_field(docking, "vx_mmps", 0),
        vy_mmps: int_field(docking, "vy_mmps", 0),
        vz_mmps: int_field(docking, "vz_mmps", 0),
    };
    let zoom = if look.zoom_permille == 0 {
        1.0
    } else {
        (f64::from(look.zoom_permille) / 1_000.0).clamp(0.5, 2.5)
    };
    let fov_deg = DOCKING_FOV_DEG / zoom;

    // 1. Deterministic background starfield & Bayer-dithered target bulkhead silhouette
    draw_target_hull_backdrop(frame, &pose, fov_deg);
    let target_kind = docking
        .get("pixel_model")
        .and_then(Value::as_str)
        .or_else(|| docking.get("target_kind").and_then(Value::as_str))
        .unwrap_or("ship");
    draw_docking_hull_wireframe(frame, target_kind, &pose, fov_deg, BULKHEAD_LIT);

    // 2. True 6-DOF perspective wireframe docking collar & 3D approach chevrons
    draw_wireframe_collar(frame, &pose, fov_deg, s);

    // 3. Central Lateral Translation Gate Box (`±0.25 m`), Pitch/Yaw scales, Roll arc, Closure tape
    draw_alignment_hud(frame, docking, &pose, sc);
}

fn draw_target_hull_backdrop(frame: &mut Frame, pose: &DockingCameraPose, fov_deg: f64) {
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    // Starfield points shifted by yaw/pitch
    for i in 0..72_u64 {
        let seed = i
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let bx = ((seed >> 16) % (w.max(1) as u64)) as i64;
        let by = ((seed >> 32) % (h.max(1) as u64)) as i64;
        let sx = (bx - pose.yaw_mdeg / 120).rem_euclid(w.max(1));
        let sy = (by + pose.pitch_mdeg / 120).rem_euclid(h.max(1));
        let col = if i % 5 == 0 {
            STROBE_BRIGHT
        } else {
            BULKHEAD_LIT
        };
        frame.set_px_safe(sx, sy, col);
    }

    // Project target bulkhead bounding box around the collar (radius 4.2 m = 4200 mm)
    let Some((cx, cy)) = project_collar_point((w, h), pose, (0.0, 0.0, 0.0), fov_deg) else {
        return;
    };
    let edge =
        project_collar_point((w, h), pose, (4_200.0, 0.0, 0.0), fov_deg).unwrap_or((cx + 80, cy));
    let max_hull_r = w.max(1);
    let hull_r = (edge.0 - cx)
        .abs()
        .hypot_i64((edge.1 - cy).abs())
        .clamp(24.min(max_hull_r), max_hull_r);
    let inner_r = (hull_r * 24) / 100;

    let x0 = (cx - hull_r).max(0);
    let x1 = (cx + hull_r).min(w - 1);
    let y0 = (cy - hull_r).max(0);
    let y1 = (cy + hull_r).min(h - 1);
    let r2 = hull_r * hull_r;
    let inner_r2 = inner_r * inner_r;

    for py in y0..=y1 {
        let dy = py - cy;
        for px in x0..=x1 {
            let dx = px - cx;
            let d2 = dx * dx + dy * dy;
            if d2 > r2 {
                continue;
            }
            if d2 <= inner_r2 {
                frame.set_px_safe(px, py, COLLAR_CAVITY);
                continue;
            }
            // Radial shading + panel grid + 4x4 Bayer ordered dithering
            let norm_q = ((d2 * 16) / r2.max(1)).clamp(0, 15) as u8;
            let thresh = bayer4(px, py);
            let seam = (dx.abs() % 28 == 0) || (dy.abs() % 28 == 0);
            let col = if seam {
                BULKHEAD_DARK
            } else if norm_q + (thresh / 2) < 9 {
                BULKHEAD_LIT
            } else if norm_q < 13 || thresh < 8 {
                BULKHEAD_MID
            } else {
                BULKHEAD_DARK
            };
            frame.set_px_safe(px, py, col);
        }
    }
}

fn draw_wireframe_collar(frame: &mut Frame, pose: &DockingCameraPose, fov_deg: f64, s: i64) {
    let dims = (frame.width() as i64, frame.height() as i64);

    // Wireframe color reflects alignment state
    let collar_col = if pose.ready_to_clamp() {
        GOOD
    } else if pose.vz_mmps > GATE_CLOSURE_MAX_MMPS {
        HOSTILE
    } else if pose.pose_aligned() {
        YOU
    } else if pose.dx_mm.abs() <= GATE_LATERAL_MM * 2
        && pose.dy_mm.abs() <= GATE_LATERAL_MM * 2
        && pose.pitch_mdeg.abs() <= GATE_ATTITUDE_MDEG * 2
        && pose.yaw_mdeg.abs() <= GATE_ATTITUDE_MDEG * 2
    {
        WARN
    } else {
        BAD
    };

    // 1. Three 3D approach corridor gates in front of the collar (Z = -4500, -2500, -1000 mm)
    for (idx, z_gate) in [-4_500.0_f64, -2_500.0, -1_000.0].into_iter().enumerate() {
        if (pose.dz_mm as f64) + z_gate <= 350.0 {
            continue;
        }
        let half_w = 1_350.0 - (idx as f64) * 120.0;
        let half_h = 1_050.0 - (idx as f64) * 90.0;
        let corners = [
            (-half_w, -half_h, z_gate),
            (half_w, -half_h, z_gate),
            (half_w, half_h, z_gate),
            (-half_w, half_h, z_gate),
        ];
        for edge_idx in 0..4 {
            let Some(c0) = corners.get(edge_idx).copied() else {
                continue;
            };
            let Some(c1) = corners.get((edge_idx + 1) % 4).copied() else {
                continue;
            };
            if let (Some(p0), Some(p1)) = (
                project_collar_point(dims, pose, c0, fov_deg),
                project_collar_point(dims, pose, c1, fov_deg),
            ) {
                line(frame, p0.0, p0.1, p1.0, p1.1, CHEVRON_CYAN);
            }
        }
    }

    // 2. Octagonal outer docking collar ring (R = 1800 mm) & inner seal ring (R = 1000 mm)
    let outer_r = 1_800.0_f64;
    let inner_r = 1_000.0_f64;
    for seg in 0..8_u32 {
        let a0 = (f64::from(seg) * 45.0 + 22.5).to_radians();
        let a1 = (f64::from(seg + 1) * 45.0 + 22.5).to_radians();
        let (s0, c0) = a0.det_sin_cos();
        let (s1, c1) = a1.det_sin_cos();

        let out0 = (outer_r * c0, outer_r * s0, 0.0);
        let out1 = (outer_r * c1, outer_r * s1, 0.0);
        let in0 = (inner_r * c0, inner_r * s0, 0.0);
        let in1 = (inner_r * c1, inner_r * s1, 0.0);

        if let (Some(p0), Some(p1)) = (
            project_collar_point(dims, pose, out0, fov_deg),
            project_collar_point(dims, pose, out1, fov_deg),
        ) {
            line(frame, p0.0, p0.1, p1.0, p1.1, collar_col);
            // Vertex strobe light
            fill_rect(
                frame,
                p0.0 - s,
                p0.1 - s,
                2 * s + 1,
                2 * s + 1,
                STROBE_BRIGHT,
            );
        }
        if let (Some(p0), Some(p1)) = (
            project_collar_point(dims, pose, in0, fov_deg),
            project_collar_point(dims, pose, in1, fov_deg),
        ) {
            line(frame, p0.0, p0.1, p1.0, p1.1, collar_col);
        }
        // Radial latch strut between inner and outer rings
        if let (Some(p_in), Some(p_out)) = (
            project_collar_point(dims, pose, in0, fov_deg),
            project_collar_point(dims, pose, out0, fov_deg),
        ) {
            line(frame, p_in.0, p_in.1, p_out.0, p_out.1, collar_col);
        }
    }

    // 3. Top Roll Alignment Fiducial Chevron on the collar (at +Y top of collar)
    let roll_col = if pose.roll_aligned() { GOOD } else { WARN };
    let fid_left = (-260.0, inner_r + 180.0, 0.0);
    let fid_apex = (0.0, outer_r + 320.0, 0.0);
    let fid_right = (260.0, inner_r + 180.0, 0.0);
    if let (Some(pl), Some(pa), Some(pr)) = (
        project_collar_point(dims, pose, fid_left, fov_deg),
        project_collar_point(dims, pose, fid_apex, fov_deg),
        project_collar_point(dims, pose, fid_right, fov_deg),
    ) {
        line(frame, pl.0, pl.1, pa.0, pa.1, roll_col);
        line(frame, pr.0, pr.1, pa.0, pa.1, roll_col);
        line(frame, pl.0, pl.1, pr.0, pr.1, roll_col);
    }

    // 4. Center collar fiducial crosshairs at (0, 0, 0)
    if let Some((cx, cy)) = project_collar_point(dims, pose, (0.0, 0.0, 0.0), fov_deg) {
        line(frame, cx - 10 * s, cy, cx + 10 * s, cy, collar_col);
        line(frame, cx, cy - 10 * s, cx, cy + 10 * s, collar_col);
    }
}

fn draw_alignment_hud(frame: &mut Frame, docking: &Value, pose: &DockingCameraPose, sc: usize) {
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    let s = sc as i64;
    let cx = w / 2;
    let cy = h / 2;

    // 1. Central Lateral Translation Gate Box (`±250 mm`)
    let gate_half = 28 * s;
    let gate_col = if pose.lateral_aligned() { GOOD } else { WARN };
    outline_rect(
        frame,
        cx - gate_half,
        cy - gate_half,
        gate_half * 2,
        gate_half * 2,
        gate_col,
    );
    // Bore-sight center tick
    line(frame, cx - 4 * s, cy, cx + 4 * s, cy, LOCK);
    line(frame, cx, cy - 4 * s, cx, cy + 4 * s, LOCK);

    // Translation error pip (maps ±500 mm across ±gate_half*2, so ±250 mm lands on the gate box edge)
    let pip_dx = ((pose.dx_mm * gate_half) / GATE_LATERAL_MM).clamp(-gate_half * 2, gate_half * 2);
    let pip_dy = (-(pose.dy_mm * gate_half) / GATE_LATERAL_MM).clamp(-gate_half * 2, gate_half * 2);
    let pip_x = cx + pip_dx;
    let pip_y = cy + pip_dy;
    let pip_col = if pose.lateral_aligned() { GOOD } else { BAD };
    fill_rect(
        frame,
        pip_x - 3 * s,
        pip_y - 3 * s,
        6 * s + 1,
        6 * s + 1,
        pip_col,
    );

    // Drift velocity vector from the translation pip
    let vel_dx = ((pose.vx_mmps * s) / 25).clamp(-24 * s, 24 * s);
    let vel_dy = (-(pose.vy_mmps * s) / 25).clamp(-24 * s, 24 * s);
    if vel_dx != 0 || vel_dy != 0 {
        line(frame, pip_x, pip_y, pip_x + vel_dx, pip_y + vel_dy, YOU);
    }

    // 2. Yaw Alignment Scale (above gate box, `±2.0°`) & Pitch Alignment Scale (left of gate box)
    let yaw_bar_y = cy - gate_half - 16 * s;
    let yaw_half_w = 40 * s;
    line(
        frame,
        cx - yaw_half_w,
        yaw_bar_y,
        cx + yaw_half_w,
        yaw_bar_y,
        BORDER,
    );
    let yaw_gate_w = yaw_half_w / 2;
    line(
        frame,
        cx - yaw_gate_w,
        yaw_bar_y - 3 * s,
        cx - yaw_gate_w,
        yaw_bar_y + 3 * s,
        GOOD,
    );
    line(
        frame,
        cx + yaw_gate_w,
        yaw_bar_y - 3 * s,
        cx + yaw_gate_w,
        yaw_bar_y + 3 * s,
        GOOD,
    );
    let yaw_needle_x =
        cx + ((pose.yaw_mdeg * yaw_gate_w) / GATE_ATTITUDE_MDEG).clamp(-yaw_half_w, yaw_half_w);
    let att_col = if pose.attitude_aligned() { GOOD } else { WARN };
    fill_rect(
        frame,
        yaw_needle_x - 2 * s,
        yaw_bar_y - 4 * s,
        4 * s + 1,
        8 * s + 1,
        att_col,
    );

    let pitch_bar_x = cx - gate_half - 16 * s;
    let pitch_half_h = 40 * s;
    line(
        frame,
        pitch_bar_x,
        cy - pitch_half_h,
        pitch_bar_x,
        cy + pitch_half_h,
        BORDER,
    );
    let pitch_gate_h = pitch_half_h / 2;
    line(
        frame,
        pitch_bar_x - 3 * s,
        cy - pitch_gate_h,
        pitch_bar_x + 3 * s,
        cy - pitch_gate_h,
        GOOD,
    );
    line(
        frame,
        pitch_bar_x - 3 * s,
        cy + pitch_gate_h,
        pitch_bar_x + 3 * s,
        cy + pitch_gate_h,
        GOOD,
    );
    let pitch_needle_y = cy
        - ((pose.pitch_mdeg * pitch_gate_h) / GATE_ATTITUDE_MDEG)
            .clamp(-pitch_half_h, pitch_half_h);
    fill_rect(
        frame,
        pitch_bar_x - 4 * s,
        pitch_needle_y - 2 * s,
        8 * s + 1,
        4 * s + 1,
        att_col,
    );

    // 3. Roll Alignment Indicator (top of HUD below header, `±3.0°`)
    let roll_y = cy - gate_half - 30 * s;
    let roll_half_w = 36 * s;
    let roll_gate_w = roll_half_w / 2;
    line(
        frame,
        cx - roll_half_w,
        roll_y,
        cx + roll_half_w,
        roll_y,
        DIM,
    );
    line(
        frame,
        cx - roll_gate_w,
        roll_y - 3 * s,
        cx - roll_gate_w,
        roll_y + 3 * s,
        GOOD,
    );
    line(
        frame,
        cx + roll_gate_w,
        roll_y - 3 * s,
        cx + roll_gate_w,
        roll_y + 3 * s,
        GOOD,
    );
    let roll_needle_x =
        cx + ((pose.roll_mdeg * roll_gate_w) / GATE_ROLL_MDEG).clamp(-roll_half_w, roll_half_w);
    let roll_col = if pose.roll_aligned() { GOOD } else { WARN };
    fill_rect(
        frame,
        roll_needle_x - 2 * s,
        roll_y - 3 * s,
        4 * s + 1,
        6 * s + 1,
        roll_col,
    );

    // 4. Right-side Closure Velocity Tape (`0.1–0.8 m/s` green latch zone)
    let tape_x = cx + gate_half + 18 * s;
    let tape_top = cy - 44 * s;
    let tape_h = 88 * s;
    fill_rect(frame, tape_x, tape_top, 10 * s, tape_h, PANEL);
    outline_rect(frame, tape_x, tape_top, 10 * s, tape_h, BORDER);
    // Green latch band (`100..=800 mm/s` out of `-200..=1400 mm/s` scale)
    let green_top = tape_top + (tape_h * (1_400 - GATE_CLOSURE_MAX_MMPS)) / 1_600;
    let green_bot = tape_top + (tape_h * (1_400 - GATE_CLOSURE_MIN_MMPS)) / 1_600;
    fill_rect(
        frame,
        tape_x + 2 * s,
        green_top,
        6 * s,
        (green_bot - green_top).max(2 * s),
        GOOD,
    );
    let vz_clamped = pose.vz_mmps.clamp(-200, 1_400);
    let vz_y = tape_top + (tape_h * (1_400 - vz_clamped)) / 1_600;
    let vz_col = if pose.closure_safe() {
        GOOD
    } else if pose.vz_mmps > GATE_CLOSURE_MAX_MMPS {
        HOSTILE
    } else {
        WARN
    };
    fill_rect(
        frame,
        tape_x - 2 * s,
        vz_y - 2 * s,
        14 * s,
        4 * s + 1,
        vz_col,
    );

    // 5. Top Telemetry Header Bar
    fill_rect(frame, 0, 0, w, 16 * s, PANEL);
    line(frame, 0, 16 * s, w - 1, 16 * s, BORDER);
    let target_label = docking
        .get("target_label")
        .and_then(Value::as_str)
        .unwrap_or("DOCKING-COLLAR");
    let status =
        docking
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or(if pose.ready_to_clamp() {
                "ALIGNED-READY"
            } else {
                "APPROACHING"
            });
    let rng_m = (pose.dz_mm as f64) / 1_000.0;
    let _ = text(
        frame,
        6 * s,
        4 * s,
        &format!("TGT:{target_label} RNG:{rng_m:.2}M [{status}]"),
        TEXT,
        sc,
    );
    let paused = docking
        .get("paused_from_world_tick")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if paused {
        let _ = text(
            frame,
            w - 148 * s,
            4 * s,
            "60FPS [WORLD TICK PAUSED]",
            YOU,
            sc,
        );
    }

    // 6. Bottom Gate Readouts & Control Strip
    let foot_h = 24 * s;
    let foot_y = h - foot_h;
    fill_rect(frame, 0, foot_y, w, foot_h, PANEL);
    line(frame, 0, foot_y, w - 1, foot_y, BORDER);

    let dx_m = (pose.dx_mm as f64) / 1_000.0;
    let dy_m = (pose.dy_mm as f64) / 1_000.0;
    let p_deg = (pose.pitch_mdeg as f64) / 1_000.0;
    let y_deg = (pose.yaw_mdeg as f64) / 1_000.0;
    let r_deg = (pose.roll_mdeg as f64) / 1_000.0;
    let vz_mps = (pose.vz_mmps as f64) / 1_000.0;
    let damp = if docking
        .get("dampeners")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        "ON"
    } else {
        "OFF"
    };

    let _ = text(
        frame,
        6 * s,
        foot_y + 3 * s,
        &format!(
            "DX:{dx_m:+.2}M DY:{dy_m:+.2}M ATT:{p_deg:+.1}/{y_deg:+.1}D ROLL:{r_deg:+.1}D CLOS:{vz_mps:+.2}M/S DAMP:{damp}"
        ),
        if pose.ready_to_clamp() { GOOD } else { TEXT },
        sc,
    );
    let _ = text(
        frame,
        6 * s,
        foot_y + 13 * s,
        "WASD:TRANS ARROWS:PITCH/YAW Q/E:ROLL R/F:CLOSURE X:DAMP",
        DIM,
        sc,
    );
}

trait HypotI64 {
    fn hypot_i64(self, other: i64) -> i64;
}

impl HypotI64 for i64 {
    fn hypot_i64(self, other: i64) -> i64 {
        ((self as f64).det_hypot(other as f64)).round() as i64
    }
}

trait SafeSet {
    fn set_px_safe(&mut self, x: i64, y: i64, rgb: Rgb);
}

impl SafeSet for Frame {
    fn set_px_safe(&mut self, x: i64, y: i64, rgb: Rgb) {
        if x >= 0 && y >= 0 && (x as usize) < self.width() && (y as usize) < self.height() {
            self.put(x as usize, y as usize, rgb);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;

    #[test]
    fn docking_camera_renders_6dof_wireframe_collar_and_gates_under_256_colours() {
        let mut feeds = HashMap::new();
        feeds.insert(
            SPACE_DOCKING_FEED.to_owned(),
            json!({
                "target_label": "DERELICT-RAIDER-1",
                "target_kind": "disabled_derelict",
                "mandatory": true,
                "paused_from_world_tick": true,
                "dx_mm": 80,
                "dy_mm": -60,
                "dz_mm": 4_500,
                "pitch_mdeg": 400,
                "yaw_mdeg": -500,
                "roll_mdeg": 750,
                "vx_mmps": -10,
                "vy_mmps": 15,
                "vz_mmps": 350,
                "dampeners": true,
                "status": "ALIGNED-READY"
            }),
        );

        let mut pic = DockingPicture::new();
        let mut frame = Frame::new(640, 480);
        assert!(pic.draw(&mut frame, &feeds, &Look::default()));
        let colours = frame.count_colours(256);
        assert!(
            (6..=64).contains(&colours),
            "expected rich 6-DOF docking frame within 256-colour limit, got {colours}"
        );
    }

    #[test]
    fn docking_pose_alignment_gates_and_perspective_shift_with_offset() {
        let aligned = DockingCameraPose {
            dx_mm: 120,
            dy_mm: -190,
            dz_mm: 3_000,
            pitch_mdeg: 1_200,
            yaw_mdeg: -1_500,
            roll_mdeg: 2_100,
            vx_mmps: 0,
            vy_mmps: 0,
            vz_mmps: 450,
        };
        assert!(aligned.ready_to_clamp());

        let too_fast = DockingCameraPose {
            vz_mmps: 1_100,
            ..aligned
        };
        assert!(too_fast.pose_aligned());
        assert!(!too_fast.closure_safe());
        assert!(!too_fast.ready_to_clamp());

        let p_center = project_collar_point((640, 480), &aligned, (0.0, 0.0, 0.0), 60.0);
        let shifted = DockingCameraPose {
            dx_mm: 1_200,
            ..aligned
        };
        let p_shifted = project_collar_point((640, 480), &shifted, (0.0, 0.0, 0.0), 60.0);
        assert_ne!(p_center, p_shifted);
    }
}
