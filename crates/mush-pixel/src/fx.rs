//! Shared FORGE3D-baked combat FX sprites (muzzle flash, weapon tracers,
//! ballistic impact bursts, fireball explosions, hull-struck shock rings)
//! and 20 Hz sub-pulse contact dead-reckoning across First-Person
//! ([`crate::cockpit::CockpitPicture`]), 2.5D Tactical
//! ([`crate::iso25d::Iso25DPicture`]), and Overhead
//! ([`crate::overhead::OverheadPicture`]) views.
//!
//! Sprite grids are tight-cropped `20×20` 3-bit (`' .:-=+*#'`, levels `0..=7`)
//! reductions from FORGE3D *Sci-Fi Effects* (`vulcan_muzzle.tga`,
//! `vulcan_heavy_impact.tga`, `Fire_001.tga`..`Fire_003.tga`, and
//! `laser_impulse_impact_ring_001.tga`). Software [`Frame`] rendering maps the
//! 7 non-transparent intensity levels onto [`crate::overlay`]'s existing
//! constant colours so FX bursts add zero extra palette entries and always stay
//! within the 256-colour indexed PNG budget.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::Frame;
use crate::frame::Rgb;
use crate::look::Facing;
use crate::math::Det as _;
use crate::overlay;

/// Number of 20 Hz ticks (`300 ms`) a combat FX burst remains visible.
pub const FX_DURATION_TICKS: u8 = 6;

/// Side length of each baked FORGE3D FX sprite grid.
pub const FX_GRID_SIDE: usize = 20;

/// FORGE3D `vulcan_muzzle.tga` flipbook frames (cells 0 and 1), `20×20`.
pub const MUZZLE_FRAMES: [[&str; FX_GRID_SIDE]; 2] = [
    [
        "                    ",
        "                    ",
        "                    ",
        "                    ",
        "                    ",
        "               .... ",
        "          ....:.....",
        "         ..:::------",
        " ::-::...:==++++****",
        ":####*+--=+**#######",
        "-####*+==+***#######",
        ".:---::..:-==+++++**",
        "         ...:-------",
        "          ....::::::",
        "             .......",
        "                    ",
        "                    ",
        "                    ",
        "                    ",
        "                    ",
    ],
    [
        "                    ",
        "                    ",
        "                    ",
        "                    ",
        "                    ",
        "....                ",
        "....................",
        "-----:::::::----::::",
        "**++++=====++++=====",
        "####################",
        "####################",
        "***++***++++++++++++",
        "-==---------------::",
        ":::::::..:::::::....",
        "................... ",
        "                    ",
        "                    ",
        "                    ",
        "                    ",
        "                    ",
    ],
];

/// FORGE3D `vulcan_heavy_impact.tga` + `solo_gun_flare_02.tga` ballistic hit
/// frames, `20×20`.
pub const HIT_FRAMES: [[&str; FX_GRID_SIDE]; 2] = [
    [
        "                    ",
        "                    ",
        "                    ",
        "          .         ",
        "        ....        ",
        "      ...::...      ",
        "     ..::---:..     ",
        "     ..:=**=::+.    ",
        "     ..-*##*-:..    ",
        "     ..-*###-:.. .. ",
        "     ..:=##*-:...   ",
        "     ...:--=-..     ",
        "     ....:::..      ",
        "      .....-:.      ",
        "     ......:+:.     ",
        "     .......=. .    ",
        "      ...  ... .    ",
        "            .       ",
        "              .     ",
        "                    ",
    ],
    [
        "                    ",
        "                    ",
        "                    ",
        "      ....          ",
        "      ....:....     ",
        "     .....:.:....  .",
        "     ...:::::...... ",
        "    ...:--=--:...   ",
        "   ...:-=====-:...  ",
        "   ...:-=++==-...   ",
        "   ...:-+*#*=-...   ",
        "   ....:-=+*=-:..   ",
        "   .....::---::.    ",
        "   ......::-:.:.    ",
        "     ....:::....    ",
        "     ....:.....     ",
        "      ........      ",
        "                    ",
        "                    ",
        "                    ",
    ],
];

/// FORGE3D `Fire_001.tga`, `Fire_002.tga`, `Fire_003.tga` explosion fireball
/// frames, `20×20`.
pub const FIRE_FRAMES: [[&str; FX_GRID_SIDE]; 3] = [
    [
        "                    ",
        "          :-:...    ",
        "     . :=+++--:-.   ",
        "   ...=**#*==+-..   ",
        "  ...:=**+-++-=:.   ",
        "   .::-+=+=+---:..  ",
        "   .:--==-=-=--:.:  ",
        "   ..:-==-=-+++-:-. ",
        "   .--=+-=--=====+. ",
        "   .=+=+++=-:-==-+. ",
        "   .:=++*++-:..:--. ",
        "   .:-++=+==:..:-:. ",
        "  .::=+=++-=:. ...  ",
        "  .:-++-::::... ..  ",
        "  .:-=-=-:.....     ",
        "  ..:-::-:...       ",
        "  ..:----:.         ",
        "  .-=+--:.          ",
        "   .:::.            ",
        "                    ",
    ],
    [
        "                    ",
        "   .:.              ",
        "   .*=.             ",
        " ..:**:    .        ",
        " :=***-:.. . .:     ",
        " ..-+*=::::  ::.    ",
        "   .+#=.-=:: ...    ",
        "   .+#*+-+=-.:.     ",
        "   --*+*+=--::.     ",
        "  .=*#*=:...::.     ",
        "  .+*#+:....:..  .. ",
        "  .=##=. ..:..  .:: ",
        "   .-=+:. :::  .:-: ",
        "     :--:..::  .::. ",
        "     .:--..::.  .   ",
        "      :-==.:..      ",
        "     . :=-...       ",
        "        :...        ",
        "        :           ",
        "        .           ",
    ],
    [
        "                    ",
        "           ...      ",
        "       .....::..    ",
        "       .+-::---.    ",
        "    .:..=+-:--+-..  ",
        "   .:-..:----:=+:.. ",
        "  .:---:::=+---==:. ",
        " .:-==+:=+=#*+==+:. ",
        " .:--=****--+**++-. ",
        " .:-*+*#*=--=*+++-. ",
        " .:*#==*+-:-=====-. ",
        " .:+=--=::::::-=-:. ",
        " .:-------:::=+-:.  ",
        " .:--=-==--::+*-:.  ",
        " .::=++==----==::.  ",
        "  .:-=++=-:----:.   ",
        "   .:--=--:--::.    ",
        "    .:-:::::::.     ",
        "     .........      ",
        "                    ",
    ],
];

/// FORGE3D `laser_impulse_impact_ring_001.tga` shockwave ring, `20×20`.
pub const RING_FRAME: [&str; FX_GRID_SIDE] = [
    "         .          ",
    "      ...-:..:.     ",
    "     :-==*+=++:     ",
    "    .-*+=+++**-:    ",
    "   .:=+-...:-=+=:   ",
    "  .----      .=+-.  ",
    "  .--.        .==:  ",
    "  :-:          :=:  ",
    " .:-:          .=-. ",
    " .::.          .==. ",
    " .::.          .-=. ",
    " .-:.          .--  ",
    " .--:.         .-:  ",
    "  --:.        .-=.  ",
    "  :==:.       :+=.  ",
    "  .-===:.....:=+:.  ",
    "   .-++-:::-=++-.   ",
    "    .-:-======:.    ",
    "     . .:::::..     ",
    "        ...         ",
];

/// Truecolor RGBA ramp for levels `1..=7` (`'.'`, `':'`, `'-'`, `'='`, `'+'`,
/// `'*'`, `'#'`) in retained Kitty RGBA overlays.
pub const FX_RGBA_RAMP: [[u8; 4]; 7] = [
    [48, 42, 38, 135],    // 1 '.' outer dark smoke
    [82, 56, 44, 170],    // 2 ':' warm smoke
    [156, 62, 28, 200],   // 3 '-' deep ember
    [224, 108, 34, 225],  // 4 '=' hot orange
    [246, 168, 52, 240],  // 5 '+' bright amber
    [252, 220, 112, 250], // 6 '*' golden flare
    [255, 248, 208, 255], // 7 '#' white-hot core
];

/// Truecolor RGBA ramp for hull-struck shockwave rings (`severe = true` shifts
/// toward crimson).
pub const FX_STRUCK_RGBA_RAMP: [[u8; 4]; 7] = [
    [64, 28, 28, 130],
    [112, 36, 32, 165],
    [176, 44, 36, 195],
    [224, 58, 42, 220],
    [245, 84, 52, 238],
    [252, 148, 84, 248],
    [255, 224, 184, 255],
];

/// Indexed-palette-safe RGB ramp for software [`Frame`] rendering. Uses
/// [`crate::overlay`] constants that are already present in the HUD palette so
/// FX bursts add zero new colours to a 256-colour indexed PNG.
pub const FX_FRAME_RAMP: [Rgb; 7] = [
    overlay::GONE,    // 1 '.'
    overlay::BORDER,  // 2 ':'
    overlay::BAD,     // 3 '-'
    overlay::HOSTILE, // 4 '='
    overlay::WARN,    // 5 '+'
    overlay::LOCK,    // 6 '*'
    overlay::YOU,     // 7 '#'
];

/// Map a sprite grid character from `' .:-=+*#'` to intensity `0..=7`.
#[must_use]
pub const fn fx_char_level(b: u8) -> u8 {
    match b {
        b'.' => 1,
        b':' => 2,
        b'-' => 3,
        b'=' => 4,
        b'+' => 5,
        b'*' => 6,
        b'#' => 7,
        _ => 0,
    }
}

/// Stamp a `20×20` FORGE3D sprite `grid` centred at `(cx, cy)` with half-width
/// `radius_px` (`>= 2`) using `ramp` (`1..=7` -> `[u8; 4]`).
pub fn stamp_fx_sprite<F>(
    grid: &[&str; FX_GRID_SIDE],
    cx: i32,
    cy: i32,
    radius_px: i32,
    ramp: &[[u8; 4]; 7],
    put: &mut F,
) where
    F: FnMut(i32, i32, [u8; 4]),
{
    let r = radius_px.clamp(2, 64);
    let diam = (r * 2).max(1);
    for dy in -r..=r {
        let gy = (((dy + r) * (FX_GRID_SIDE as i32 - 1)) / diam).clamp(0, FX_GRID_SIDE as i32 - 1)
            as usize;
        let Some(row) = grid.get(gy).map(|s| s.as_bytes()) else {
            continue;
        };
        for dx in -r..=r {
            let gx = (((dx + r) * (FX_GRID_SIDE as i32 - 1)) / diam)
                .clamp(0, FX_GRID_SIDE as i32 - 1) as usize;
            let Some(&ch) = row.get(gx) else {
                continue;
            };
            let lvl = fx_char_level(ch);
            if lvl == 0 {
                continue;
            }
            if let Some(&col) = ramp.get(usize::from(lvl - 1)) {
                put(cx + dx, cy + dy, col);
            }
        }
    }
}

/// Kind of transient combat visual effect triggered by `arena.cockpit` `"notices"`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CombatFxKind {
    /// Outgoing weapon fire (`"hit"`, `"kill"`, `"target_section_lost"`, `"miss"`, `"slot_fired"`).
    Fire {
        /// Target contact `"id"` or `"designator"`, if known.
        target_key: Option<String>,
        /// Whether the shot hit the target.
        hit: bool,
        /// Whether the shot destroyed the target.
        kill: bool,
    },
    /// Incoming hit on the player's own unit (`"struck"`, `"struck_spent"`, `"section_lost"`).
    Struck {
        /// Whether an armour section was lost (`"section_lost"`).
        severe: bool,
    },
}

/// Active transient combat effect with remaining 20 Hz frame ticks (`1..=FX_DURATION_TICKS`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CombatFx {
    /// Effect kind and target metadata.
    pub kind: CombatFxKind,
    /// Remaining 20 Hz render ticks before expiry.
    pub ticks_left: u8,
}

/// Ingest any new combat `"notices"` from `frame` into `active_fx` when `t_ms`
/// advances beyond `last_fx_t_ms`.
pub fn sync_combat_fx(
    last_fx_t_ms: &mut Option<i64>,
    active_fx: &mut Vec<CombatFx>,
    frame: &Value,
) {
    let t_ms = frame.get("t_ms").and_then(Value::as_i64).unwrap_or(0);
    let Some(prev_t) = *last_fx_t_ms else {
        *last_fx_t_ms = Some(t_ms);
        return;
    };
    if t_ms <= prev_t {
        return;
    }
    *last_fx_t_ms = Some(t_ms);

    let Some(notices) = frame.get("notices").and_then(Value::as_array) else {
        return;
    };
    if notices.is_empty() {
        return;
    }

    let fallback_target = frame
        .get("lock")
        .and_then(Value::as_str)
        .or_else(|| frame.get("lock_designator").and_then(Value::as_str))
        .map(str::to_owned);

    for n in notices {
        let Some(kind_str) = n.get("kind").and_then(Value::as_str) else {
            continue;
        };
        let target_key = n
            .get("contact")
            .and_then(Value::as_str)
            .or_else(|| n.get("designator").and_then(Value::as_str))
            .map(str::to_owned)
            .or_else(|| fallback_target.clone());
        match kind_str {
            "hit" | "target_section_lost" => {
                active_fx.push(CombatFx {
                    kind: CombatFxKind::Fire {
                        target_key,
                        hit: true,
                        kill: false,
                    },
                    ticks_left: FX_DURATION_TICKS,
                });
            }
            "kill" => {
                active_fx.push(CombatFx {
                    kind: CombatFxKind::Fire {
                        target_key,
                        hit: true,
                        kill: true,
                    },
                    ticks_left: FX_DURATION_TICKS,
                });
            }
            "miss" | "slot_fired" => {
                active_fx.push(CombatFx {
                    kind: CombatFxKind::Fire {
                        target_key,
                        hit: false,
                        kill: false,
                    },
                    ticks_left: FX_DURATION_TICKS,
                });
            }
            "struck" | "struck_spent" => {
                active_fx.push(CombatFx {
                    kind: CombatFxKind::Struck { severe: false },
                    ticks_left: FX_DURATION_TICKS,
                });
            }
            "section_lost" => {
                active_fx.push(CombatFx {
                    kind: CombatFxKind::Struck { severe: true },
                    ticks_left: FX_DURATION_TICKS,
                });
            }
            _ => {}
        }
    }
}

/// Decrement `ticks_left` on all active FX and drop expired entries.
pub fn step_combat_fx(active_fx: &mut Vec<CombatFx>) {
    for fx in &mut *active_fx {
        fx.ticks_left = fx.ticks_left.saturating_sub(1);
    }
    active_fx.retain(|fx| fx.ticks_left > 0);
}

/// Paint one [`CombatFx`] burst (FORGE3D muzzle flash at `(gun_x, gun_y)`,
/// glowing tracer streak toward `target_xy`, FORGE3D hit/kill explosion burst
/// at `target_xy`, or FORGE3D shockwave ring around `struck_xy`) via `put`.
pub fn paint_single_fx<F>(
    fx: &CombatFx,
    (gun_x, gun_y): (i32, i32),
    target_xy: Option<(i32, i32, i32)>,
    (struck_x, struck_y): (i32, i32),
    put: &mut F,
) where
    F: FnMut(i32, i32, [u8; 4]),
{
    let elapsed = FX_DURATION_TICKS.saturating_sub(fx.ticks_left);
    let progress = f32::from(elapsed) / f32::from(FX_DURATION_TICKS);
    match &fx.kind {
        CombatFxKind::Fire { hit, kill, .. } => {
            // 1. FORGE3D `vulcan_muzzle` flash at the gun mount during the first half of the burst.
            if fx.ticks_left >= FX_DURATION_TICKS / 2 {
                let [m0, m1] = &MUZZLE_FRAMES;
                let muzzle_grid = if fx.ticks_left >= FX_DURATION_TICKS - 1 {
                    m1
                } else {
                    m0
                };
                let r_m = if fx.ticks_left >= FX_DURATION_TICKS - 1 {
                    8
                } else {
                    6
                };
                stamp_fx_sprite(muzzle_grid, gun_x, gun_y, r_m, &FX_RGBA_RAMP, put);
            }

            let Some((raw_tx, raw_ty, target_r)) = target_xy else {
                return;
            };
            let tx = raw_tx + if *hit { 0 } else { 14 };
            let ty = raw_ty + if *hit { 0 } else { -10 };

            // 2. Glowing weapon tracer streak from gun mount to target.
            let steps = 48i32;
            let t_start = (progress * 0.35).clamp(0.0, 0.7);
            let t_end = (progress * 1.35 + 0.35).clamp(0.25, 1.0);
            let s0 = (t_start * steps as f32) as i32;
            let s1 = (t_end * steps as f32) as i32;
            let [_, _, _, r3, _, r5, r6] = FX_RGBA_RAMP;
            let core_col = if *hit { r6 } else { r5 };
            let glow_col = r3;
            for s in s0..=s1 {
                if !*hit && (s & 3) == 2 {
                    continue;
                }
                let f = s as f32 / steps as f32;
                let px = gun_x + ((tx - gun_x) as f32 * f).round() as i32;
                let py = gun_y + ((ty - gun_y) as f32 * f).round() as i32;
                put(px, py, core_col);
                put(px + 1, py, core_col);
                put(px, py + 1, glow_col);
            }

            // 3. FORGE3D target impact explosion (`vulcan_heavy_impact` / `Fire_001..003`).
            if *hit {
                let base_r = target_r.clamp(8, 26);
                let max_r = if *kill { (base_r * 3) / 2 } else { base_r };
                let r_burst = ((0.55 + 0.45 * progress) * max_r as f32)
                    .round()
                    .clamp(6.0, 36.0) as i32;
                if *kill {
                    let fire_idx = usize::from(elapsed / 2).min(FIRE_FRAMES.len() - 1);
                    if let Some(grid) = FIRE_FRAMES.get(fire_idx) {
                        stamp_fx_sprite(grid, tx, ty, r_burst, &FX_RGBA_RAMP, put);
                    }
                    // Overlay shock ring on kill.
                    stamp_fx_sprite(
                        &RING_FRAME,
                        tx,
                        ty,
                        (r_burst + 4).min(40),
                        &FX_RGBA_RAMP,
                        put,
                    );
                } else {
                    let hit_idx = usize::from(elapsed >= 3);
                    if let Some(grid) = HIT_FRAMES.get(hit_idx) {
                        stamp_fx_sprite(grid, tx, ty, r_burst, &FX_RGBA_RAMP, put);
                    }
                }
            }
        }
        CombatFxKind::Struck { severe } => {
            let r_ring = if *severe { 22 } else { 16 };
            let ramp = if *severe {
                &FX_STRUCK_RGBA_RAMP
            } else {
                &FX_RGBA_RAMP
            };
            stamp_fx_sprite(&RING_FRAME, struck_x, struck_y, r_ring, ramp, put);
        }
    }
}

/// Convert a truecolor FX RGBA pixel into a palette-safe [`Rgb`] from
/// [`FX_FRAME_RAMP`] for software [`Frame`] drawing.
#[must_use]
pub fn rgba_to_frame_rgb(col: [u8; 4]) -> Rgb {
    let [r, g, b, _] = col;
    let [f0, f1, f2, f3, f4, f5, f6] = FX_FRAME_RAMP;
    let lum = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
    if lum >= 225 {
        f6
    } else if lum >= 195 {
        f5
    } else if lum >= 155 {
        f4
    } else if lum >= 115 {
        f3
    } else if lum >= 80 {
        f2
    } else if lum >= 55 {
        f1
    } else {
        f0
    }
}

/// Paint active combat FX onto a software [`Frame`] using only the 7
/// [`FX_FRAME_RAMP`] colours (zero extra palette budget).
pub fn draw_fx_on_frame<F>(
    frame: &mut Frame,
    active_fx: &[CombatFx],
    gun_xy: (i32, i32),
    struck_xy: (i32, i32),
    mut resolve_target: F,
) where
    F: FnMut(Option<&str>) -> Option<(i32, i32, i32)>,
{
    if active_fx.is_empty() {
        return;
    }
    let w = frame.width() as i32;
    let h = frame.height() as i32;
    for fx in active_fx {
        let target_xy = match &fx.kind {
            CombatFxKind::Fire { target_key, .. } => resolve_target(target_key.as_deref()),
            CombatFxKind::Struck { .. } => None,
        };
        let mut put = |x: i32, y: i32, col: [u8; 4]| {
            if x >= 0 && x < w && y >= 0 && y < h {
                frame.put(x as usize, y as usize, rgba_to_frame_rgb(col));
            }
        };
        paint_single_fx(fx, gun_xy, target_xy, struck_xy, &mut put);
    }
}

// ---- 20 Hz Sub-Pulse Contact Dead-Reckoning --------------------------------

/// Per-contact world position and velocity track across 1 Hz arena pulses.
#[derive(Clone, Copy, Debug)]
struct ContactTrack {
    wx_m: f32,
    wy_m: f32,
    vx_mps: f32,
    vy_mps: f32,
    t_ms: i64,
}

/// Tracks contact positions across 1 Hz `arena.cockpit` pulses and dead-reckons
/// moving contacts (`"moving": true`) at 20 Hz (`50 ms` sub-pulse steps) between
/// pulses.
///
/// On the first render of any new `t_ms` (`sub_pulse_ticks == 0`), every contact
/// sits at its exact keyframe coordinate (`(0.0, 0.0)` offset), preserving
/// pixel-exact golden parity for single-frame renders.
#[derive(Clone, Debug, Default)]
pub struct ContactTracker {
    last_t_ms: Option<i64>,
    sub_pulse_ticks: u16,
    tracks: BTreeMap<String, ContactTrack>,
}

impl ContactTracker {
    /// Create a new empty contact tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Current sub-pulse tick count (`0` on the keyframe pulse itself, `1..=19`
    /// across the 20 Hz ticks between 1 Hz server pulses).
    #[must_use]
    pub fn sub_pulse_ticks(&self) -> u16 {
        self.sub_pulse_ticks
    }

    /// Update the tracker for one render tick of `cockpit` given the ego
    /// vehicle's keyframe world position `(base_x_m, base_y_m)`.
    pub fn step_frame(&mut self, cockpit: &Value, base_x_m: f32, base_y_m: f32) {
        let Some(t_ms) = cockpit.get("t_ms").and_then(Value::as_i64) else {
            self.sub_pulse_ticks = 0;
            return;
        };
        let Some(contacts) = cockpit.get("contacts").and_then(Value::as_array) else {
            self.last_t_ms = Some(t_ms);
            self.sub_pulse_ticks = 0;
            return;
        };
        let any_moving = contacts.iter().any(|c| {
            c.get("moving").and_then(Value::as_bool) == Some(true)
                && c.get("reads_dead").and_then(Value::as_bool) != Some(true)
        });

        if self.last_t_ms != Some(t_ms) {
            self.sub_pulse_ticks = 0;
            self.last_t_ms = Some(t_ms);
            let mut next_tracks = BTreeMap::new();
            for (ci, c) in contacts.iter().enumerate() {
                let Some(bearing_deg) = c.get("bearing_deg").and_then(Value::as_f64) else {
                    continue;
                };
                let Some(range_m) = c.get("range_m").and_then(Value::as_f64) else {
                    continue;
                };
                let key = contact_track_key(c, ci);
                let rad = (bearing_deg as f32).to_radians();
                let (sin_b, cos_b) = rad.det_sin_cos();
                let r_f = range_m as f32;
                let wx_m = base_x_m + r_f * sin_b;
                let wy_m = base_y_m + r_f * cos_b;
                let moving = c.get("moving").and_then(Value::as_bool) == Some(true)
                    && c.get("reads_dead").and_then(Value::as_bool) != Some(true);

                let (vx_mps, vy_mps) = if !moving {
                    (0.0, 0.0)
                } else if let Some(prev) = self.tracks.get(&key)
                    && t_ms > prev.t_ms
                    && (t_ms - prev.t_ms) <= 3_000
                {
                    let dt_s = ((t_ms - prev.t_ms) as f32 * 0.001).max(0.1);
                    let meas_vx = ((wx_m - prev.wx_m) / dt_s).clamp(-45.0, 45.0);
                    let meas_vy = ((wy_m - prev.wy_m) / dt_s).clamp(-45.0, 45.0);
                    if meas_vx.abs() > 0.2 || meas_vy.abs() > 0.2 {
                        (meas_vx, meas_vy)
                    } else {
                        heading_velocity_mps(c, bearing_deg as f32)
                    }
                } else {
                    heading_velocity_mps(c, bearing_deg as f32)
                };

                next_tracks.insert(
                    key,
                    ContactTrack {
                        wx_m,
                        wy_m,
                        vx_mps,
                        vy_mps,
                        t_ms,
                    },
                );
            }
            self.tracks = next_tracks;
        } else if any_moving {
            self.sub_pulse_ticks = self.sub_pulse_ticks.saturating_add(1).min(19);
        } else {
            self.sub_pulse_ticks = 0;
        }
    }

    /// Return the dead-reckoned world offset `(dx_m, dy_m)` for contact `c` at
    /// index `ci` on the current sub-pulse tick.
    #[must_use]
    pub fn contact_offset_m(&self, c: &Value, ci: usize) -> (f32, f32) {
        if self.sub_pulse_ticks == 0 {
            return (0.0, 0.0);
        }
        let moving = c.get("moving").and_then(Value::as_bool) == Some(true)
            && c.get("reads_dead").and_then(Value::as_bool) != Some(true);
        if !moving {
            return (0.0, 0.0);
        }
        let dt_s = f32::from(self.sub_pulse_ticks) * 0.05;
        let key = contact_track_key(c, ci);
        if let Some(track) = self.tracks.get(&key) {
            return (track.vx_mps * dt_s, track.vy_mps * dt_s);
        }
        let bearing_deg = c.get("bearing_deg").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let (vx, vy) = heading_velocity_mps(c, bearing_deg);
        (vx * dt_s, vy * dt_s)
    }

    /// Return the dead-reckoned `(bearing_deg, range_m)` of contact `c` relative
    /// to `(base_x_m, base_y_m)` on the current sub-pulse tick.
    #[must_use]
    pub fn contact_bearing_range(&self, c: &Value, ci: usize) -> (f64, f64) {
        let bearing_deg = c.get("bearing_deg").and_then(Value::as_f64).unwrap_or(0.0);
        let range_m = c.get("range_m").and_then(Value::as_f64).unwrap_or(0.0);
        let (dx_m, dy_m) = self.contact_offset_m(c, ci);
        if dx_m == 0.0 && dy_m == 0.0 {
            return (bearing_deg, range_m);
        }
        let rad = (bearing_deg as f32).to_radians();
        let (sin_b, cos_b) = rad.det_sin_cos();
        let r_f = range_m as f32;
        let wx = r_f * sin_b + dx_m;
        let wy = r_f * cos_b + dy_m;
        let new_r = wx.det_hypot(wy).max(1.0);
        let new_b = wx.det_atan2(wy).to_degrees().rem_euclid(360.0);
        (f64::from(new_b), f64::from(new_r))
    }
}

fn contact_track_key(c: &Value, ci: usize) -> String {
    c.get("id")
        .and_then(Value::as_str)
        .or_else(|| c.get("designator").and_then(Value::as_str))
        .map_or_else(|| format!("idx:{ci}"), str::to_owned)
}

/// Derive a moving contact's world velocity vector `(vx_mps, vy_mps)` from its
/// explicit `"heading_deg"` (if present) or its `"bearing_deg"` + `"facing"` /
/// `"aspect"` relative to the line of sight.
fn heading_velocity_mps(c: &Value, bearing_deg: f32) -> (f32, f32) {
    let heading_deg = if let Some(h) = c.get("heading_deg").and_then(Value::as_f64) {
        (h as f32).rem_euclid(360.0)
    } else if let Some(hm) = c.get("heading_mdeg").and_then(Value::as_i64) {
        ((hm as f32) * 0.001).rem_euclid(360.0)
    } else {
        let facing_word = c
            .get("facing")
            .or_else(|| c.get("aspect"))
            .and_then(Value::as_str)
            .unwrap_or("front");
        let facing = Facing::from_aspect(facing_word);
        (bearing_deg + 180.0 - (facing as i32 as f32) * 45.0).rem_euclid(360.0)
    };
    let speed_mps = c
        .get("speed_kph")
        .and_then(Value::as_f64)
        .map_or(10.0f32, |kph| ((kph as f32) / 3.6).clamp(2.0, 40.0));
    let rad = heading_deg.to_radians();
    let (sin_h, cos_h) = rad.det_sin_cos();
    (speed_mps * sin_h, speed_mps * cos_h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn forge3d_sprites_stamp_within_palette_budget() {
        let mut frame = Frame::new(160, 120);
        let active = vec![
            CombatFx {
                kind: CombatFxKind::Fire {
                    target_key: Some("BA".to_string()),
                    hit: true,
                    kill: true,
                },
                ticks_left: FX_DURATION_TICKS,
            },
            CombatFx {
                kind: CombatFxKind::Struck { severe: true },
                ticks_left: FX_DURATION_TICKS - 1,
            },
        ];
        draw_fx_on_frame(&mut frame, &active, (80, 105), (80, 60), |_| {
            Some((80, 45, 16))
        });
        assert!(frame.count_colours(256) <= 8);
    }

    #[test]
    fn contact_tracker_dead_reckons_between_pulses_and_resets_on_new_pulse() {
        let mut tracker = ContactTracker::new();
        let frame_t0 = json!({
            "t_ms": 1000,
            "contacts": [
                {
                    "id": "BA",
                    "bearing_deg": 0.0,
                    "range_m": 100.0,
                    "facing": "right",
                    "moving": true
                }
            ]
        });
        let c = &frame_t0["contacts"][0];
        // First render at t_ms=1000: sub_pulse_ticks == 0, zero offset.
        tracker.step_frame(&frame_t0, 0.0, 0.0);
        assert_eq!(tracker.sub_pulse_ticks(), 0);
        assert_eq!(tracker.contact_offset_m(c, 0), (0.0, 0.0));

        // Second render at same t_ms=1000: contact glides east (`facing = "right"` at bearing 0° -> heading 90°).
        tracker.step_frame(&frame_t0, 0.0, 0.0);
        assert_eq!(tracker.sub_pulse_ticks(), 1);
        let (dx, dy) = tracker.contact_offset_m(c, 0);
        assert!(dx > 0.4, "expected eastward dead-reckoning: dx={dx}");
        assert!(dy.abs() < 0.05, "expected near-zero dy: dy={dy}");

        // Next pulse t_ms=2000 resets sub_pulse_ticks to 0.
        let frame_t1 = json!({
            "t_ms": 2000,
            "contacts": [
                {
                    "id": "BA",
                    "bearing_deg": 5.0,
                    "range_m": 100.4,
                    "facing": "right",
                    "moving": true
                }
            ]
        });
        tracker.step_frame(&frame_t1, 0.0, 0.0);
        assert_eq!(tracker.sub_pulse_ticks(), 0);
        assert_eq!(
            tracker.contact_offset_m(&frame_t1["contacts"][0], 0),
            (0.0, 0.0)
        );
    }
}
