//! The instruments drawn INTO the frame (the owner's 640x480 target of
//! 2026-10-21: "when we go to add things like minimaps, heat gauges, weapon
//! recycle statuses, an armor-status doll, we'll appreciate the extra
//! pixels"). Drawn after the raycaster, over the picture, from the same
//! `arena.cockpit` and `arena.map` data the TUI's panes read:
//!
//! - [`draw_minimap`]: a window of the arena around the pilot, north up,
//!   ground coloured by cell, the pilot as a dot with a heading line, the
//!   contacts and the named spots marked.
//! - [`draw_heat`]: the heat gauge, green to amber to red, `OVERHEAT` when
//!   the frame says so.
//! - [`draw_weapons`]: one row a weapon — name, a recycle bar (full when
//!   ready, a countdown while it cycles), rounds, the hit chance.
//! - [`draw_doll`]: the armour-status doll — a top-down tank whose hull
//!   facets, turret, tracks and engine are coloured by what is left of them.
//! - [`draw_status`]: the one-line strip — callsign, speed, heading, lock.
//!
//! Everything is opaque and drawn from a small fixed set of colours, so the
//! frame stays under the palette PNG's 256; there is no alpha blending here
//! on purpose. The glyphs are a 5x7 bitmap font (capitals, digits, a little
//! punctuation), scaled with the frame: 1x at 320x220, 2x at 640x480.

use crate::Frame;
use crate::frame::Rgb;
use crate::raycast::{Ground, Map, SpriteKind};

/// A marked cell on the minimap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapMark {
    /// Arena cell, `x` east, `y` north.
    pub cell: (i64, i64),
    /// Drawn as.
    pub kind: SpriteKind,
}

/// A named spot on the minimap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    /// Arena cell.
    pub cell: (i64, i64),
    /// Its letter.
    pub letter: char,
}

/// The heat gauge's reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heat {
    /// Heat now.
    pub heat: i64,
    /// Heat at which the hull overheats.
    pub max: i64,
    /// The frame's own verdict.
    pub overheated: bool,
}

/// One weapon's row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeaponRow {
    /// The weapon's id (`cannon`, `machine_gun`).
    pub name: String,
    /// Ready to fire.
    pub ready: bool,
    /// Milliseconds until ready, when not.
    pub ready_in_ms: i64,
    /// Rounds left.
    pub rounds: i64,
    /// Rounds when full.
    pub rounds_max: i64,
    /// Chance to hit the lock, per cent, when there is a lock.
    pub percent: Option<u64>,
}

/// One facet of a section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facet {
    /// `front`, `rear`, `left`, `right`, `top`.
    pub id: String,
    /// Armour left.
    pub armor: i64,
    /// Armour when whole.
    pub armor_max: i64,
}

/// One section of the hull.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// `hull`, `turret`, `left_track`, `right_track`, `engine`.
    pub id: String,
    /// `intact`, `damaged`, `destroyed`, as the frame words it.
    pub condition: String,
    /// Internals left.
    pub internals: i64,
    /// Internals when whole.
    pub internals_max: i64,
    /// Its facets.
    pub facets: Vec<Facet>,
}

/// The status strip's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// The pilot's callsign.
    pub callsign: String,
    /// Speed, km/h.
    pub speed_kph: i64,
    /// The camera's heading, degrees.
    pub heading_deg: i64,
    /// The lock's designator and range in metres, when there is one.
    pub lock: Option<(String, i64)>,
    /// `online` / `offline` / whatever the frame says.
    pub power: String,
}

// ---- colours ---------------------------------------------------------------

const PANEL: Rgb = [16, 18, 24];
const BORDER: Rgb = [96, 104, 120];
const TEXT: Rgb = [220, 224, 230];
const DIM: Rgb = [140, 146, 156];
const GOOD: Rgb = [70, 190, 90];
const WARN: Rgb = [230, 180, 50];
const BAD: Rgb = [220, 60, 50];
const GONE: Rgb = [60, 60, 66];
const YOU: Rgb = [255, 255, 255];
const HOSTILE: Rgb = [230, 70, 60];
const FRIENDLY: Rgb = [90, 150, 240];
const UNKNOWN: Rgb = [220, 210, 120];
const WRECK: Rgb = [110, 100, 90];

fn ground_rgb(ground: Ground) -> Rgb {
    match ground {
        Ground::Grass => [60, 100, 44],
        Ground::Water => [40, 70, 120],
        Ground::Road => [120, 112, 100],
        Ground::Woods => [34, 70, 38],
        Ground::WoodsHeavy => [24, 52, 28],
        Ground::Wall => [150, 150, 158],
        Ground::Open => [100, 86, 64],
    }
}

fn kind_rgb(kind: SpriteKind) -> Rgb {
    match kind {
        SpriteKind::Hostile => HOSTILE,
        SpriteKind::Friendly => FRIENDLY,
        SpriteKind::Unknown => UNKNOWN,
        SpriteKind::Wreck => WRECK,
    }
}

/// Green, amber, red or gone, by what is left of a thing.
fn ramp(left: i64, max: i64) -> Rgb {
    if max <= 0 || left <= 0 {
        GONE
    } else if left * 3 >= max * 2 {
        GOOD
    } else if left * 3 >= max {
        WARN
    } else {
        BAD
    }
}

// ---- primitives ------------------------------------------------------------

/// The overlay's scale: 1 at the spike's 320x220, 2 at the 640x480 target.
#[must_use]
pub fn scale_for(frame: &Frame) -> usize {
    if frame.width() >= 640 { 2 } else { 1 }
}

fn fill_rect(frame: &mut Frame, x: i64, y: i64, w: i64, h: i64, px: Rgb) {
    for yy in y.max(0)..(y + h).max(0) {
        for xx in x.max(0)..(x + w).max(0) {
            frame.put(xx as usize, yy as usize, px);
        }
    }
}

fn outline_rect(frame: &mut Frame, x: i64, y: i64, w: i64, h: i64, px: Rgb) {
    fill_rect(frame, x, y, w, 1, px);
    fill_rect(frame, x, y + h - 1, w, 1, px);
    fill_rect(frame, x, y, 1, h, px);
    fill_rect(frame, x + w - 1, y, 1, h, px);
}

/// A panel: a filled box with a border.
fn panel(frame: &mut Frame, x: i64, y: i64, w: i64, h: i64) {
    fill_rect(frame, x, y, w, h, PANEL);
    outline_rect(frame, x, y, w, h, BORDER);
}

/// A line from `(x0, y0)` to `(x1, y1)`, Bresenham.
fn line(frame: &mut Frame, x0: i64, y0: i64, x1: i64, y1: i64, px: Rgb) {
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let (mut x, mut y, mut err) = (x0, y0, dx + dy);
    for _ in 0..4096 {
        if x >= 0 && y >= 0 {
            frame.put(x as usize, y as usize, px);
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// The 5x7 font: each glyph is seven rows of five bits, top row first.
fn glyph(c: char) -> Option<[u8; 7]> {
    Some(match c.to_ascii_uppercase() {
        ' ' => [0, 0, 0, 0, 0, 0, 0],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '/' => [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
        '.' => [0, 0, 0, 0, 0, 0x0C, 0x0C],
        ':' => [0, 0x0C, 0x0C, 0, 0x0C, 0x0C, 0],
        '-' => [0, 0, 0, 0x1F, 0, 0, 0],
        '+' => [0, 0x04, 0x04, 0x1F, 0x04, 0x04, 0],
        '_' => [0, 0, 0, 0, 0, 0, 0x1F],
        '?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0, 0x04],
        _ => return None,
    })
}

/// Draw `text` with its top-left at `(x, y)`, each glyph cell `6 * scale`
/// wide; unknown characters draw as `?`. Returns the width drawn.
fn text(frame: &mut Frame, x: i64, y: i64, s: &str, px: Rgb, scale: usize) -> i64 {
    let sc = scale as i64;
    let mut cx = x;
    for c in s.chars() {
        let rows = glyph(c).or_else(|| glyph('?')).unwrap_or([0; 7]);
        for (r, bits) in rows.iter().enumerate() {
            for col in 0..5 {
                if bits & (0x10 >> col) != 0 {
                    fill_rect(frame, cx + col * sc, y + r as i64 * sc, sc, sc, px);
                }
            }
        }
        cx += 6 * sc;
    }
    cx - x
}

// ---- the instruments -------------------------------------------------------

/// The minimap, bottom-left: `view` cells on a side around `you`, north
/// up, `scale` pixels a cell.
pub fn draw_minimap(
    frame: &mut Frame,
    map: &Map,
    you: (i64, i64),
    heading_mdeg: u32,
    marks: &[MapMark],
    spots: &[Spot],
) {
    let s = scale_for(frame) as i64;
    let view: i64 = 60; // cells on a side
    let px_cell = s; // 1 px a cell at 320, 2 at 640
    let size = view * px_cell;
    let x0 = 4 * s;
    let y0 = frame.height() as i64 - size - 4 * s - 2;
    panel(frame, x0 - 1, y0 - 1, size + 2, size + 2);
    let (cx, cy) = you;
    let left = cx - view / 2;
    let south = cy - view / 2;
    for row in 0..view {
        for col in 0..view {
            let (mx, my) = (left + col, south + row);
            let ground =
                if mx < 0 || my < 0 || mx >= map.width() as i64 || my >= map.height() as i64 {
                    None
                } else {
                    Some(map.get(mx, my))
                };
            let px = ground.map_or(PANEL, ground_rgb);
            // North up: row 0 is the south edge, drawn at the bottom.
            let sy = y0 + (view - 1 - row) * px_cell;
            fill_rect(frame, x0 + col * px_cell, sy, px_cell, px_cell, px);
        }
    }
    let to_px = |cell: (i64, i64)| -> Option<(i64, i64)> {
        let (dx, dy) = (cell.0 - left, cell.1 - south);
        if (0..view).contains(&dx) && (0..view).contains(&dy) {
            Some((
                x0 + dx * px_cell + px_cell / 2,
                y0 + (view - 1 - dy) * px_cell + px_cell / 2,
            ))
        } else {
            None
        }
    };
    for spot in spots {
        if let Some((px, py)) = to_px(spot.cell) {
            text(
                frame,
                px - 2 * s,
                py - 3 * s,
                &spot.letter.to_string(),
                TEXT,
                s as usize,
            );
        }
    }
    for mark in marks {
        if let Some((px, py)) = to_px(mark.cell) {
            fill_rect(
                frame,
                px - s,
                py - s,
                2 * s + 1,
                2 * s + 1,
                kind_rgb(mark.kind),
            );
        }
    }
    if let Some((px, py)) = to_px(you) {
        let h = (heading_mdeg % 360_000) as f32 / 1000.0;
        let (dx, dy) = h.to_radians().sin_cos();
        let len = 6.0 * s as f32;
        line(
            frame,
            px,
            py,
            px + (dx * len) as i64,
            py - (dy * len) as i64,
            YOU,
        );
        fill_rect(frame, px - s, py - s, 2 * s + 1, 2 * s + 1, YOU);
    }
}

/// The heat gauge, bottom centre.
pub fn draw_heat(frame: &mut Frame, heat: &Heat) {
    let s = scale_for(frame) as i64;
    let w = 100 * s;
    let h = 7 * s;
    let x0 = (frame.width() as i64 - w) / 2;
    let y0 = frame.height() as i64 - h - 4 * s - 2;
    panel(frame, x0 - 1, y0 - 1, w + 2, h + 2);
    let frac = if heat.max > 0 {
        (heat.heat.max(0) as f32 / heat.max as f32).min(1.0)
    } else {
        0.0
    };
    let fill = (w as f32 * frac) as i64;
    let colour = if heat.overheated || frac >= 0.8 {
        BAD
    } else if frac >= 0.5 {
        WARN
    } else {
        GOOD
    };
    fill_rect(frame, x0, y0, fill, h, colour);
    // A hull without a heat model (the frame's `heat` is null, drawn here as
    // zeros) keeps the gauge's place with a dash, so the layout never jumps.
    let label = if heat.overheated {
        "OVERHEAT".to_string()
    } else if heat.max <= 0 {
        "HEAT --".to_string()
    } else {
        format!("HEAT {}/{}", heat.heat, heat.max)
    };
    let tw = label.len() as i64 * 6 * s;
    text(
        frame,
        x0 + (w - tw) / 2,
        y0 - 8 * s,
        &label,
        if heat.overheated { BAD } else { TEXT },
        s as usize,
    );
}

/// The weapons, top right: a row each.
pub fn draw_weapons(frame: &mut Frame, rows: &[WeaponRow]) {
    if rows.is_empty() {
        return;
    }
    let s = scale_for(frame) as i64;
    // Columns at 1x: name 48, bar 30, rounds 42, chance 18, with gaps.
    let w = 150 * s;
    let row_h = 10 * s;
    let x0 = frame.width() as i64 - w - 4 * s;
    // Under the status strip (which is 9 * s tall at 4 * s).
    let y0 = 17 * s;
    let h = rows.len() as i64 * row_h + 2 * s;
    panel(frame, x0 - 1, y0 - 1, w + 2, h + 2);
    for (i, row) in rows.iter().enumerate() {
        let y = y0 + s + i as i64 * row_h;
        let name: String = row
            .name
            .chars()
            .take(8)
            .collect::<String>()
            .replace('_', " ");
        text(frame, x0 + s, y, &name, TEXT, s as usize);
        // The recycle bar: 30 px wide at 1x.
        let bx = x0 + 52 * s;
        let bw = 30 * s;
        outline_rect(frame, bx, y, bw, 7 * s, DIM);
        if row.ready {
            fill_rect(frame, bx, y, bw, 7 * s, GOOD);
        } else {
            let secs = format!("{:.1}S", row.ready_in_ms.max(0) as f32 / 1000.0);
            text(frame, bx + 3 * s, y, &secs, WARN, s as usize);
        }
        let rounds = format!("{}/{}", row.rounds, row.rounds_max);
        let rounds_colour = ramp(row.rounds, row.rounds_max);
        text(
            frame,
            bx + bw + 4 * s,
            y,
            &rounds,
            rounds_colour,
            s as usize,
        );
        if let Some(p) = row.percent {
            let pct = format!("{p}%");
            let tw = pct.len() as i64 * 6 * s;
            text(frame, x0 + w - tw - s, y, &pct, TEXT, s as usize);
        }
    }
}

/// The armour-status doll, bottom right: a top-down tank.
pub fn draw_doll(frame: &mut Frame, sections: &[Section]) {
    let s = scale_for(frame) as i64;
    let w = 48 * s;
    let h = 64 * s;
    let x0 = frame.width() as i64 - w - 4 * s;
    let y0 = frame.height() as i64 - h - 4 * s - 2;
    panel(frame, x0 - 1, y0 - 1, w + 2, h + 2);
    let find = |id: &str| sections.iter().find(|sec| sec.id == id);
    let facet = |sec: Option<&Section>, id: &str| -> Rgb {
        sec.and_then(|sec| sec.facets.iter().find(|f| f.id == id))
            .map_or(GONE, |f| ramp(f.armor, f.armor_max))
    };
    let body = |sec: Option<&Section>| -> Rgb {
        sec.map_or(GONE, |sec| {
            if sec.condition == "destroyed" {
                GONE
            } else {
                ramp(sec.internals, sec.internals_max)
            }
        })
    };
    let hull = find("hull");
    // Hull body: a rectangle inset from the panel, with four facet plates
    // around it and the top plate as its fill.
    let (hx, hy, hw, hh) = (x0 + 10 * s, y0 + 6 * s, w - 20 * s, h - 12 * s);
    fill_rect(frame, hx, hy, hw, hh, facet(hull, "top"));
    let t = 3 * s; // plate thickness
    fill_rect(frame, hx, hy - t, hw, t, facet(hull, "front"));
    fill_rect(frame, hx, hy + hh, hw, t, facet(hull, "rear"));
    fill_rect(frame, hx - t, hy, t, hh, facet(hull, "left"));
    fill_rect(frame, hx + hw, hy, t, hh, facet(hull, "right"));
    // Tracks: bars outside the side plates, coloured by the section.
    fill_rect(
        frame,
        hx - t - 2 * s - 2 * s,
        hy - t,
        2 * s,
        hh + 2 * t,
        body(find("left_track")),
    );
    fill_rect(
        frame,
        hx + hw + t + 2 * s,
        hy - t,
        2 * s,
        hh + 2 * t,
        body(find("right_track")),
    );
    // Engine: a block at the rear inside the hull.
    fill_rect(
        frame,
        hx + hw / 4,
        hy + hh - 10 * s,
        hw / 2,
        7 * s,
        body(find("engine")),
    );
    // Turret: a square forward of centre, its top facet, with a barrel.
    let turret = find("turret");
    let (tx, ty, tw) = (hx + hw / 2 - 7 * s, hy + 8 * s, 14 * s);
    fill_rect(frame, tx, ty, tw, tw, facet(turret, "top"));
    outline_rect(frame, tx, ty, tw, tw, body(turret));
    fill_rect(
        frame,
        tx + tw / 2 - s,
        hy - t - 4 * s,
        2 * s,
        ty - hy + t + 4 * s,
        body(turret),
    );
    // Section internals as a border on the hull outline.
    outline_rect(frame, hx - t, hy - t, hw + 2 * t, hh + 2 * t, body(hull));
}

/// The status strip, top left.
pub fn draw_status(frame: &mut Frame, status: &Status) {
    let s = scale_for(frame) as i64;
    let mut words = format!(
        "{}  {} KPH  HDG {:03}  {}",
        status.callsign.to_ascii_uppercase(),
        status.speed_kph,
        status.heading_deg.rem_euclid(360),
        status.power.to_ascii_uppercase()
    );
    if let Some((who, range)) = &status.lock {
        words.push_str(&format!("  LOCK {} {}M", who.to_ascii_uppercase(), range));
    }
    let w = words.len() as i64 * 6 * s + 2 * s;
    let x0 = 4 * s;
    let y0 = 4 * s;
    panel(frame, x0 - 1, y0 - 1, w + 2, 9 * s + 2);
    text(frame, x0 + s, y0 + s, &words, TEXT, s as usize);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raycast::{Camera, render};

    fn yard() -> Map {
        let mut map = Map::new(96, 64, 1000);
        for y in 0..64 {
            for x in 0..96 {
                map.set(
                    x,
                    y,
                    if (20..30).contains(&x) && (2..6).contains(&y) {
                        Ground::Water
                    } else {
                        Ground::Grass
                    },
                );
            }
        }
        for x in 40..50 {
            map.set(x, 20, Ground::Wall);
        }
        map
    }

    fn sections() -> Vec<Section> {
        vec![
            Section {
                id: "engine".into(),
                condition: "intact".into(),
                internals: 40,
                internals_max: 40,
                facets: vec![],
            },
            Section {
                id: "hull".into(),
                condition: "intact".into(),
                internals: 60,
                internals_max: 60,
                facets: vec![
                    Facet {
                        id: "front".into(),
                        armor: 55,
                        armor_max: 80,
                    },
                    Facet {
                        id: "rear".into(),
                        armor: 30,
                        armor_max: 30,
                    },
                ],
            },
            Section {
                id: "left_track".into(),
                condition: "intact".into(),
                internals: 6,
                internals_max: 24,
                facets: vec![Facet {
                    id: "left".into(),
                    armor: 4,
                    armor_max: 24,
                }],
            },
            Section {
                id: "right_track".into(),
                condition: "destroyed".into(),
                internals: 0,
                internals_max: 24,
                facets: vec![],
            },
            Section {
                id: "turret".into(),
                condition: "intact".into(),
                internals: 30,
                internals_max: 30,
                facets: vec![Facet {
                    id: "top".into(),
                    armor: 18,
                    armor_max: 20,
                }],
            },
        ]
    }

    fn draw_all(frame: &mut Frame) {
        let map = yard();
        let cam = Camera {
            x_cm: 6_500,
            y_cm: 4_500,
            heading_mdeg: 45_000,
        };
        render(frame, &map, &cam, &[]);
        draw_minimap(
            frame,
            &map,
            (6, 4),
            45_000,
            &[
                MapMark {
                    cell: (36, 14),
                    kind: SpriteKind::Hostile,
                },
                MapMark {
                    cell: (2, 9),
                    kind: SpriteKind::Friendly,
                },
            ],
            &[Spot {
                cell: (25, 3),
                letter: 'A',
            }],
        );
        draw_heat(
            frame,
            &Heat {
                heat: 40,
                max: 100,
                overheated: false,
            },
        );
        draw_weapons(
            frame,
            &[
                WeaponRow {
                    name: "cannon".into(),
                    ready: true,
                    ready_in_ms: 0,
                    rounds: 14,
                    rounds_max: 20,
                    percent: Some(62),
                },
                WeaponRow {
                    name: "machine_gun".into(),
                    ready: false,
                    ready_in_ms: 900,
                    rounds: 380,
                    rounds_max: 500,
                    percent: None,
                },
            ],
        );
        draw_doll(frame, &sections());
        draw_status(
            frame,
            &Status {
                callsign: "RA".into(),
                speed_kph: 43,
                heading_deg: 45,
                lock: Some(("BA".into(), 440)),
                power: "online".into(),
            },
        );
    }

    #[test]
    fn every_instrument_fits_the_palette_at_both_sizes() {
        for (w, h) in [(320usize, 220usize), (640, 480)] {
            let mut frame = Frame::new(w, h);
            draw_all(&mut frame);
            assert!(
                frame.count_colours(256) <= 256,
                "{w}x{h} overflowed the palette"
            );
            // The minimap's panel sits bottom-left and shows water (blue-ish)
            // somewhere in its window; the pilot's white dot is drawn.
            let s = scale_for(&frame);
            let white = (0..w)
                .flat_map(|x| (0..h).map(move |y| (x, y)))
                .any(|(x, y)| frame.get(x, y) == YOU);
            assert!(white, "the pilot's dot at scale {s}");
        }
    }

    #[test]
    fn the_font_draws_and_unknown_glyphs_fall_back() {
        let mut frame = Frame::new(100, 20);
        frame.clear([0, 0, 0]);
        let width = text(&mut frame, 1, 1, "HDG 090 ~", TEXT, 1);
        assert_eq!(width, 9 * 6);
        // 'H' lights its left column at the top-left.
        assert_eq!(frame.get(1, 1), TEXT);
        assert_eq!(frame.get(2, 1), [0, 0, 0]);
    }

    #[test]
    fn the_ramp_and_the_heat_colour_follow_the_fraction() {
        assert_eq!(ramp(80, 80), GOOD);
        assert_eq!(ramp(40, 80), WARN);
        assert_eq!(ramp(10, 80), BAD);
        assert_eq!(ramp(0, 80), GONE);
        let mut frame = Frame::new(640, 480);
        frame.clear([0, 0, 0]);
        draw_heat(
            &mut frame,
            &Heat {
                heat: 95,
                max: 100,
                overheated: true,
            },
        );
        let red = (0..640usize)
            .flat_map(|x| (0..480usize).map(move |y| (x, y)))
            .any(|(x, y)| frame.get(x, y) == BAD);
        assert!(red);
    }
}
