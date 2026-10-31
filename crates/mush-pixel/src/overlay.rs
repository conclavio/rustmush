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
//! - [`draw_doll`]: the armour-status doll — the unit's look's grid (doc 22),
//!   each cell coloured by what is left of its part.
//! - [`draw_status`]: the one-line strip — callsign, speed, heading, lock.
//!
//! **The flight HUD** ([`draw_hud`], the text cockpit of 2026-10-22): when
//! the doll, the weapons, the heat and the contacts move to terminal text
//! under the picture, the picture keeps only what a pilot reads off the
//! view itself — a heading tape across the top (the hull's heading, an
//! ordered one and the contacts marked on it), the reticle in the centre
//! boxing the lock, and a speed bar on the right. Since doc 21 cut G.13 the
//! overhead's inset sits in the top-left corner under it, where
//! [`inset_rect`] puts it clear of all three.
//!
//! Everything is opaque and drawn from a small fixed set of colours, so the
//! frame stays under the palette PNG's 256; there is no alpha blending here
//! on purpose. The glyphs are a 5x7 bitmap font (capitals, digits, a little
//! punctuation), scaled with the frame: 1x at 320x220, 2x at 640x480.

use crate::Frame;
use crate::frame::Rgb;
use crate::raycast::{Ground, Map, SpriteKind};
use crate::scene::{Anchor, Item, Scene, Shape, Transform, Viewport};

/// A marked cell on the minimap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapMark {
    /// Arena cell, `x` east, `y` north.
    pub cell: (i64, i64),
    /// Drawn as.
    pub kind: SpriteKind,
}

/// **A campaign-layer mark on the minimap** (doc 32 cut CF.4; Q-CF-48): an IC
/// field's frame says which glyph stands in which cell — a block letter, a
/// claim's edge `#`, a pit `v`/`V`, a reading `1`–`3`/`+`/`.`, the pad `H`, a
/// wreck or remains `%`, a crate `&` — and the minimap draws it as that
/// glyph's one fixed shape (`campaign_item`), inventing none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapGlyph {
    /// Arena cell, `x` east, `y` north.
    pub cell: (i64, i64),
    /// The frame's glyph.
    pub glyph: char,
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

pub(crate) const PANEL: Rgb = [16, 18, 24];
pub(crate) const BORDER: Rgb = [96, 104, 120];
pub(crate) const TEXT: Rgb = [220, 224, 230];
pub(crate) const DIM: Rgb = [140, 146, 156];
pub(crate) const GOOD: Rgb = [70, 190, 90];
pub(crate) const WARN: Rgb = [230, 180, 50];
pub(crate) const BAD: Rgb = [220, 60, 50];
pub(crate) const GONE: Rgb = [60, 60, 66];
pub(crate) const YOU: Rgb = [255, 255, 255];
/// A hostile contact's marks (doc 32 cut CF.0, Q-CF-12: by relation).
pub(crate) const HOSTILE: Rgb = [230, 70, 60];
/// The own unit's and a friend's marks.
pub(crate) const FRIEND: Rgb = [90, 150, 240];
/// The replay's red team: a replay is the omniscient record, drawn by team.
pub(crate) const RED_TEAM: Rgb = HOSTILE;
/// The replay's blue team.
pub(crate) const BLUE_TEAM: Rgb = FRIEND;
const UNKNOWN: Rgb = [170, 170, 162];
/// The lock's box and label: a marker, never a team's colour.
pub(crate) const LOCK: Rgb = [240, 200, 60];
pub(crate) const WRECK: Rgb = [110, 100, 90];
/// Doc 32 cut CF.4: a claim's edge on the minimap.
pub(crate) const CLAIM: Rgb = [150, 120, 60];
/// Doc 32 cut CF.4: an ore reading on the minimap.
pub(crate) const ORE: Rgb = [200, 170, 60];
/// Doc 32 cut CF.4: a crate on the minimap.
pub(crate) const CRATE: Rgb = [180, 140, 90];

pub(crate) fn ground_rgb(ground: Ground) -> Rgb {
    match ground {
        Ground::Grass => [60, 100, 44],
        Ground::Water => [40, 70, 120],
        Ground::Road => [120, 112, 100],
        Ground::Rock => [104, 104, 112],
        Ground::Woods => [34, 70, 38],
        Ground::WoodsHeavy => [24, 52, 28],
        Ground::Wall => [150, 150, 158],
        Ground::Open => [100, 86, 64],
    }
}

fn kind_rgb(kind: SpriteKind) -> Rgb {
    match kind {
        SpriteKind::Hostile => HOSTILE,
        SpriteKind::Friend => FRIEND,
        SpriteKind::Unknown => UNKNOWN,
        SpriteKind::Wreck => WRECK,
    }
}

/// Green, amber, red or gone, by what is left of a thing.
pub(crate) fn ramp(left: i64, max: i64) -> Rgb {
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

pub(crate) fn fill_rect(frame: &mut Frame, x: i64, y: i64, w: i64, h: i64, px: Rgb) {
    for yy in y.max(0)..(y + h).max(0) {
        for xx in x.max(0)..(x + w).max(0) {
            frame.put(xx as usize, yy as usize, px);
        }
    }
}

pub(crate) fn outline_rect(frame: &mut Frame, x: i64, y: i64, w: i64, h: i64, px: Rgb) {
    fill_rect(frame, x, y, w, 1, px);
    fill_rect(frame, x, y + h - 1, w, 1, px);
    fill_rect(frame, x, y, 1, h, px);
    fill_rect(frame, x + w - 1, y, 1, h, px);
}

/// A panel: a filled box with a border.
pub(crate) fn panel(frame: &mut Frame, x: i64, y: i64, w: i64, h: i64) {
    fill_rect(frame, x, y, w, h, PANEL);
    outline_rect(frame, x, y, w, h, BORDER);
}

/// A line from `(x0, y0)` to `(x1, y1)`, Bresenham.
pub(crate) fn line(frame: &mut Frame, x0: i64, y0: i64, x1: i64, y1: i64, px: Rgb) {
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
pub(crate) fn glyph(c: char) -> Option<[u8; 7]> {
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
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        ',' => [0, 0, 0, 0, 0x0C, 0x04, 0x08],
        ';' => [0, 0x0C, 0x0C, 0, 0x0C, 0x04, 0x08],
        '\'' => [0x04, 0x04, 0x08, 0, 0, 0, 0],
        _ => return None,
    })
}

/// Draw `text` with its top-left at `(x, y)`, each glyph cell `6 * scale`
/// wide; unknown characters draw as `?`. Returns the width drawn.
pub(crate) fn text(frame: &mut Frame, x: i64, y: i64, s: &str, px: Rgb, scale: usize) -> i64 {
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

/// The minimap, bottom-left: 60 cells on a side around `you`, north up, a
/// pixel a cell at the spike's 320x220 and two at 640x480; ground coloured by
/// cell, the named spots' letters, the contacts' squares, and the pilot's
/// square with a heading line. **A scene since doc 21 cut G.5a** (§3.3: "a
/// scene is also what the minimap is"), drawn pixel for pixel as it was —
/// [`minimap_scene`] builds it.
pub fn draw_minimap(
    frame: &mut Frame,
    map: &Map,
    you: (i64, i64),
    heading_mdeg: u32,
    marks: &[MapMark],
    spots: &[Spot],
) {
    minimap_scene(frame, map, you, heading_mdeg, marks, spots).draw(frame);
}

/// [`draw_minimap`] with an IC field's campaign layer under the spots and the
/// contacts (doc 32 cut CF.4).
pub fn draw_minimap_with(
    frame: &mut Frame,
    map: &Map,
    (you, heading_mdeg): ((i64, i64), u32),
    marks: &[MapMark],
    spots: &[Spot],
    campaign: &[MapGlyph],
) {
    minimap_scene_with(frame, map, (you, heading_mdeg), marks, spots, campaign).draw(frame);
}

/// The minimap's scene for `frame` (see [`draw_minimap`]).
#[must_use]
pub fn minimap_scene(
    frame: &Frame,
    map: &Map,
    you: (i64, i64),
    heading_mdeg: u32,
    marks: &[MapMark],
    spots: &[Spot],
) -> Scene {
    minimap_scene_with(frame, map, (you, heading_mdeg), marks, spots, &[])
}

/// **How one campaign glyph is drawn on the minimap** (doc 32 cut CF.4): a
/// block letter or the pad as text, a claim's edge or a pit as its cell
/// filled, a reading as a dot, a wreck or a crate as a square. A glyph this
/// build does not know is not drawn.
fn campaign_item(at: (f64, f64), glyph: char, s: i64) -> Option<Item> {
    let cell = |fill: Rgb| Item::Rect {
        min: (at.0 - 0.5, at.1 - 0.5),
        max: (at.0 + 0.5, at.1 + 0.5),
        fill,
    };
    Some(match glyph {
        'A'..='P' => Item::Text {
            at,
            offset: (-2 * s, -3 * s),
            anchor: Anchor::TopLeft,
            text: glyph.to_string(),
            colour: DIM,
            scale: s as usize,
        },
        '#' => cell(CLAIM),
        'v' | 'V' => cell(GONE),
        '1' | '2' | '3' | '+' | '.' => Item::Marker {
            at,
            half: (s / 2).max(1),
            shape: Shape::Disc,
            colour: if glyph == '.' { DIM } else { ORE },
        },
        '%' => Item::Marker {
            at,
            half: s,
            shape: Shape::Square,
            colour: WRECK,
        },
        '&' => Item::Marker {
            at,
            half: s,
            shape: Shape::Square,
            colour: CRATE,
        },
        _ => return None,
    })
}

/// [`minimap_scene`] with the campaign layer (see [`draw_minimap_with`]).
#[must_use]
pub fn minimap_scene_with(
    frame: &Frame,
    map: &Map,
    (you, heading_mdeg): ((i64, i64), u32),
    marks: &[MapMark],
    spots: &[Spot],
    campaign: &[MapGlyph],
) -> Scene {
    let s = scale_for(frame) as i64;
    let view: i64 = 60; // cells on a side
    let size = view * s;
    let x0 = 4 * s;
    let y0 = frame.height() as i64 - size - 4 * s - 2;
    let (cx, cy) = you;
    let (left, south) = (cx - view / 2, cy - view / 2);
    let mut scene = Scene::new(Viewport {
        world: (
            left as f64,
            south as f64,
            (left + view) as f64,
            (south + view) as f64,
        ),
        frame: (x0, y0, size, size),
        transform: Transform::Linear,
    });
    scene.push(Item::Panel {
        rect: (x0 - 1, y0 - 1, size + 2, size + 2),
        fill: PANEL,
        border: Some(BORDER),
    });
    for my in south..south + view {
        for mx in left..left + view {
            let inside = mx >= 0 && my >= 0 && mx < map.width() as i64 && my < map.height() as i64;
            let fill = if inside {
                ground_rgb(map.get(mx, my))
            } else {
                PANEL
            };
            scene.push(Item::Rect {
                min: (mx as f64, my as f64),
                max: ((mx + 1) as f64, (my + 1) as f64),
                fill,
            });
        }
    }
    let shown = |cell: (i64, i64)| {
        let (dx, dy) = (cell.0 - left, cell.1 - south);
        ((0..view).contains(&dx) && (0..view).contains(&dy))
            .then_some((cell.0 as f64 + 0.5, cell.1 as f64 + 0.5))
    };
    // Doc 32 cut CF.4: the campaign layer, over the ground, under the spots.
    for mark in campaign {
        if let Some(item) = shown(mark.cell).and_then(|at| campaign_item(at, mark.glyph, s)) {
            scene.push(item);
        }
    }
    for spot in spots {
        if let Some(at) = shown(spot.cell) {
            scene.push(Item::Text {
                at,
                offset: (-2 * s, -3 * s),
                anchor: Anchor::TopLeft,
                text: spot.letter.to_string(),
                colour: TEXT,
                scale: s as usize,
            });
        }
    }
    for mark in marks {
        if let Some(at) = shown(mark.cell) {
            scene.push(Item::Marker {
                at,
                half: s,
                shape: Shape::Square,
                colour: kind_rgb(mark.kind),
            });
        }
    }
    if let Some(at) = shown(you) {
        scene.push(Item::Ray {
            at,
            bearing_deg: (heading_mdeg % 360_000) as f32 / 1000.0,
            len_px: 6.0 * s as f32,
            colour: YOU,
        });
        scene.push(Item::Marker {
            at,
            half: s,
            shape: Shape::Square,
            colour: YOU,
        });
    }
    scene
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

/// A doll and what is left of each of its parts, `(left, max)` a part.
pub type DollState<'a> = (&'a crate::look::Doll, &'a [Option<(i64, i64)>]);

/// **The armour-status doll**, bottom right (doc 22 D-22-2): the unit's look's
/// grid — `doll` with what is left of each of its parts, from
/// [`crate::look::Doll::condition`] — one square a cell, each coloured by the
/// ramp (green, amber, red, grey when gone). A unit type with no look gets a
/// bar a section instead, from `sections`.
pub fn draw_doll(frame: &mut Frame, doll: Option<DollState<'_>>, sections: &[Section]) {
    let s = scale_for(frame) as i64;
    let Some((doll, condition)) = doll else {
        draw_section_bars(frame, sections, s);
        return;
    };
    let c = 3 * s;
    let (w, h) = (doll.width as i64 * c, doll.height as i64 * c);
    let x0 = frame.width() as i64 - w - 6 * s;
    let y0 = frame.height() as i64 - h - 6 * s - 2;
    panel(frame, x0 - 2 * s, y0 - 2 * s, w + 4 * s, h + 4 * s);
    let tones = doll.tones();
    let part_at = |x: usize, y: usize| doll.cells.get(y * doll.width + x).copied().flatten();
    for y in 0..doll.height {
        for x in 0..doll.width {
            let Some(part) = part_at(x, y) else {
                continue;
            };
            let px = match condition.get(part).copied().flatten() {
                Some((left, max)) => ramp(left, max),
                None => GONE,
            };
            // Touching parts in the same state are told apart by tone.
            let px = if tones.get(part) == Some(&1) {
                lighter(px)
            } else {
                px
            };
            let (cx, cy) = (x0 + x as i64 * c, y0 + y as i64 * c);
            fill_rect(frame, cx, cy, c, c, px);
            // A part's edge against another part, or against nothing, is
            // outlined.
            if x + 1 >= doll.width || part_at(x + 1, y) != Some(part) {
                fill_rect(frame, cx + c - 1, cy, 1, c, PANEL);
            }
            if part_at(x, y + 1) != Some(part) {
                fill_rect(frame, cx, cy + c - 1, c, 1, PANEL);
            }
        }
    }
}

/// The lighter twin of a ramp colour, for a doll's tone `1`.
pub(crate) fn lighter(px: Rgb) -> Rgb {
    match px {
        GOOD => [120, 220, 130],
        WARN => [245, 210, 110],
        BAD => [240, 120, 105],
        _ => [96, 96, 104],
    }
}

/// The doll of a unit type with no look: a bar a section, its internals.
fn draw_section_bars(frame: &mut Frame, sections: &[Section], s: i64) {
    let (bw, bh) = (40 * s, 4 * s);
    let h = sections.len() as i64 * (bh + 2 * s);
    let x0 = frame.width() as i64 - bw - 6 * s;
    let y0 = frame.height() as i64 - h - 6 * s - 2;
    panel(frame, x0 - 2 * s, y0 - 2 * s, bw + 4 * s, h + 2 * s);
    for (i, sec) in sections.iter().enumerate() {
        let y = y0 + i as i64 * (bh + 2 * s);
        let left = if sec.condition == "destroyed" {
            0
        } else {
            sec.internals
        };
        fill_rect(frame, x0, y, bw, bh, GONE);
        let fill = bw * left.clamp(0, sec.internals_max.max(1)) / sec.internals_max.max(1);
        fill_rect(frame, x0, y, fill, bh, ramp(left, sec.internals_max));
    }
}

/// A plain card: the whole frame dark, `words` in the middle (a picture with
/// nothing to show, so the last one is not left standing).
pub fn draw_card(frame: &mut Frame, words: &str) {
    let s = scale_for(frame) as i64;
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    fill_rect(frame, 0, 0, w, h, PANEL);
    let tw = words.chars().count() as i64 * 6 * s - s;
    text(frame, (w - tw) / 2, (h - 7 * s) / 2, words, DIM, s as usize);
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

// ---- the flight HUD --------------------------------------------------------

/// The heading tape's reach either side of the camera, degrees.
const TAPE_HALF_DEG: i64 = 30;
/// The speed bar's full scale, km/h, until the speed passes it (the cockpit
/// frame carries no top speed yet): then the next 20 above.
const SPEED_FULL_KPH: i64 = 60;

/// A contact marked on the heading tape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TapeMark {
    /// Its bearing, clockwise from north, millidegrees.
    pub bearing_mdeg: i64,
    /// Drawn as.
    pub kind: SpriteKind,
}

/// Where the lock is in the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockAt {
    /// In view: its box, `(x, y, w, h)` in pixels.
    Box(i64, i64, i64, i64),
    /// Off the side of the view: `-1` left, `1` right.
    Off(i8),
    /// In the view's columns but clear over its top edge (`-1`) or under
    /// its bottom (`1`) — close by, up a crest or down a steep slope — at
    /// that column (doc 21a `X-03`).
    Beyond(i64, i8),
}

/// The lock, for the reticle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HudLock {
    /// Its designator.
    pub designator: String,
    /// Range, metres.
    pub range_m: i64,
    /// The hit chance, percent, when the frame gives one.
    pub percent: Option<i64>,
    /// Where it is drawn.
    pub at: LockAt,
}

/// **A contact's health, as the frame's one word says it** (doc 21 cut
/// G.14): the keyframe's `health` — the worst band the pilot's own scope
/// read off the hull, never a number — drawn as pips over the sprite and
/// beside the overhead's mark. A `wreck` is drawn as one already and gets
/// no gauge; a row with no word (unidentified, or never close enough for a
/// look) gets none either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    /// Every section green: three pips.
    Intact,
    /// Yellow at worst: two.
    Damaged,
    /// Red at worst, or a section gone: one.
    Crippled,
}

/// How many pips a gauge has.
pub const HEALTH_PIPS: i64 = 3;

impl Health {
    /// The frame's word as a gauge, or `None` for a word that draws none
    /// (`wreck`, an unknown word) and for no word.
    #[must_use]
    pub fn from_word(word: Option<&str>) -> Option<Health> {
        match word? {
            "intact" => Some(Health::Intact),
            "damaged" => Some(Health::Damaged),
            "crippled" => Some(Health::Crippled),
            _ => None,
        }
    }

    /// The pips lit, of [`HEALTH_PIPS`].
    #[must_use]
    pub const fn lit(self) -> i64 {
        match self {
            Health::Intact => 3,
            Health::Damaged => 2,
            Health::Crippled => 1,
        }
    }

    /// The lit pips' colour: the doll's own green, amber and red.
    #[must_use]
    pub(crate) const fn rgb(self) -> Rgb {
        match self {
            Health::Intact => GOOD,
            Health::Damaged => WARN,
            Health::Crippled => BAD,
        }
    }
}

/// A health gauge placed in the picture: over a contact's sprite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HealthMark {
    /// The sprite's centre column, pixels.
    pub x: i64,
    /// The sprite's top row, pixels (may be off the frame).
    pub top: i64,
    /// What it reads.
    pub health: Health,
    /// The contact's party letter, drawn right of the pips (doc 32 cut
    /// CF.0, Q-CF-12), `None` while unclassified.
    pub party: Option<char>,
    /// How the contact is drawn: the letter's colour, its relation's.
    pub kind: SpriteKind,
}

/// What the flight HUD draws.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hud {
    /// The camera's heading, millidegrees: the tape's centre.
    pub heading_mdeg: i64,
    /// The hull's heading, millidegrees: marked on the tape where a look
    /// has swung the camera off it.
    pub hull_mdeg: i64,
    /// A heading the helm has ordered and the hull has not reached.
    pub ordered_mdeg: Option<i64>,
    /// Speed, km/h.
    pub speed_kph: i64,
    /// The contacts, marked on the tape at their bearings.
    pub marks: Vec<TapeMark>,
    /// The lock, boxed by the reticle.
    pub lock: Option<HudLock>,
    /// The health gauges (G.14), one over each identified contact in view
    /// the frame gives a word for, the lock's among them.
    pub health: Vec<HealthMark>,
}

/// The signed arc from `from` to `to`, millidegrees in `-180_000..180_000`.
fn arc_mdeg(from: i64, to: i64) -> i64 {
    (to - from + 180_000).rem_euclid(360_000) - 180_000
}

/// **Where the HUD's minimap inset goes** (doc 21 cut G.13): the top-left
/// corner, `(x, y, side)` in pixels — square, `4s` in from the edges — kept
/// clear of everything the HUD draws at a fixed place: the heading tape
/// (its panel starts at `w/5`; the inset ends `8s` short of it), the lock's
/// off-side arrow at mid-height on the left edge (the inset ends `12s` over
/// the frame's middle), and the speed bar on the right. At 640 wide that is
/// 112 pixels square at `(8, 8)` on the 640x480 and 640x276 frames and 104
/// on the text cockpit's 640x256; at 320 wide, 56. `None` on a frame too
/// short for one (under 24 pixels a side). A lock clear over the top edge
/// in the far-left columns ([`LockAt::Beyond`]) puts its arrow and readout
/// over the inset's top rows — the readout is drawn after it, so it wins.
#[must_use]
pub fn inset_rect(frame: &Frame) -> Option<(i64, i64, usize)> {
    let s = scale_for(frame) as i64;
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    let side = (56 * s).min(h / 2 - 12 * s).min(w / 5 - 8 * s);
    usize::try_from(side)
        .ok()
        .filter(|side| *side >= 24)
        .map(|side| (4 * s, 4 * s, side))
}

/// The flight HUD over the view: the heading tape, the reticle and the
/// speed bar.
pub fn draw_hud(frame: &mut Frame, hud: &Hud) {
    let s = scale_for(frame) as i64;
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    draw_tape(frame, hud, s, w);
    let label = draw_reticle(frame, hud.lock.as_ref(), s, w, h);
    draw_health(frame, &hud.health, label, s, w, h);
    draw_speed(frame, hud.speed_kph, s, w, h);
}

/// A small triangle `size` rows deep with its point at `apex`, pointing up
/// (`dir` -1), down (1), left (-2) or right (2).
fn caret(frame: &mut Frame, (ax, ay): (i64, i64), size: i64, dir: i8, px: Rgb) {
    for i in 0..size {
        match dir {
            -1 => fill_rect(frame, ax - i, ay + i, 2 * i + 1, 1, px),
            1 => fill_rect(frame, ax - i, ay - i, 2 * i + 1, 1, px),
            -2 => fill_rect(frame, ax + i, ay - i, 1, 2 * i + 1, px),
            _ => fill_rect(frame, ax - i, ay - i, 1, 2 * i + 1, px),
        }
    }
}

/// The heading tape, top centre: ticks every 5°, a label every 10° (`N`,
/// `E`, `S`, `W` at the cardinals), the camera's heading read out under a
/// caret at its centre; the contacts as pips above it, the hull's heading
/// (when the look parts the camera from it) and an ordered one as markers
/// on it, pinned to the tape's end when they lie past it.
fn draw_tape(frame: &mut Frame, hud: &Hud, s: i64, w: i64) {
    let tw = w * 3 / 5;
    let x0 = (w - tw) / 2;
    let cx = w / 2;
    let y0 = 5 * s;
    let band = 14 * s;
    let cam = hud.heading_mdeg.rem_euclid(360_000);
    let reach = TAPE_HALF_DEG * 1_000;
    let at = |rel: i64| cx + rel.clamp(-reach, reach) * (tw / 2) / reach;
    panel(frame, x0, y0, tw, band);
    let lo = (cam / 1_000 - TAPE_HALF_DEG).div_euclid(5) * 5;
    for d in (lo..=cam / 1_000 + TAPE_HALF_DEG + 5).step_by(5) {
        let deg = d.rem_euclid(360);
        let rel = arc_mdeg(cam, deg * 1_000);
        if rel.abs() > reach {
            continue;
        }
        let x = at(rel);
        let len = if deg % 10 == 0 { 4 * s } else { 2 * s };
        fill_rect(frame, x, y0 + band - s - len, s, len, DIM);
        if deg % 10 == 0 {
            let label = match deg {
                0 => "N".to_string(),
                90 => "E".to_string(),
                180 => "S".to_string(),
                270 => "W".to_string(),
                d => format!("{d:03}"),
            };
            let lw = label.len() as i64 * 6 * s - s;
            if x - lw / 2 > x0 + s && x + lw / 2 < x0 + tw - s {
                text(frame, x - lw / 2, y0 + 2 * s, &label, TEXT, s as usize);
            }
        }
    }
    // The contacts, as pips above the tape.
    for m in &hud.marks {
        let rel = arc_mdeg(cam, m.bearing_mdeg);
        if rel.abs() <= reach {
            fill_rect(
                frame,
                at(rel) - s,
                y0 - 3 * s,
                2 * s,
                2 * s,
                kind_rgb(m.kind),
            );
        }
    }
    // The hull, where the look has parted the camera from it.
    let hull = arc_mdeg(cam, hud.hull_mdeg);
    if hull.abs() > 1_000 {
        caret(frame, (at(hull), y0 + band - s), 3 * s, 1, GOOD);
    }
    if let Some(ordered) = hud.ordered_mdeg {
        let x = at(arc_mdeg(cam, ordered));
        outline_rect(frame, x - 2 * s, y0 + band - 6 * s, 4 * s, 5 * s, WARN);
    }
    // The centre caret and the readout.
    caret(frame, (cx, y0 + band), 3 * s, -1, YOU);
    let readout = format!("{:03}", cam / 1_000);
    let rw = 3 * 6 * s + 3 * s;
    let ry = y0 + band + 4 * s;
    panel(frame, cx - rw / 2, ry, rw, 11 * s);
    text(
        frame,
        cx - rw / 2 + 2 * s,
        ry + 2 * s,
        &readout,
        YOU,
        s as usize,
    );
}

/// The reticle at the centre, and the lock: boxed where it stands with its
/// designator, range and hit chance under the box (over it where the box
/// runs off the bottom of the view), or an arrow at the side of the view it
/// lies off — or at the top or bottom edge it lies clear over, in its column
/// — with them beside it: the readout is in the view wherever the lock is.
/// Answers the readout's rectangle, `(x, y, w, h)`, so what is drawn after
/// it (the health gauges, G.14) keeps clear of it.
fn draw_reticle(
    frame: &mut Frame,
    lock: Option<&HudLock>,
    s: i64,
    w: i64,
    h: i64,
) -> Option<(i64, i64, i64, i64)> {
    let (cx, cy) = (w / 2, h / 2);
    let (gap, arm) = (3 * s, 5 * s);
    fill_rect(frame, cx - gap - arm, cy, arm, s, TEXT);
    fill_rect(frame, cx + gap + 1, cy, arm, s, TEXT);
    fill_rect(frame, cx, cy - gap - arm, s, arm, TEXT);
    fill_rect(frame, cx, cy + gap + 1, s, arm, TEXT);
    let lock = lock?;
    let mut label = format!("{} {}M", lock.designator.to_ascii_uppercase(), lock.range_m);
    if let Some(p) = lock.percent {
        label.push_str(&format!(" {p}%"));
    }
    let lw = label.len() as i64 * 6 * s;
    let (lx, ly) = match lock.at {
        LockAt::Box(x, y, bw, bh) => {
            let m = 2 * s;
            let (bx, by) = (x - m, y - m);
            let (bw, bh) = ((bw + 2 * m).max(8 * s), (bh + 2 * m).max(8 * s));
            outline_rect(frame, bx, by, bw, bh, LOCK);
            outline_rect(frame, bx + 1, by + 1, bw - 2, bh - 2, LOCK);
            let lx = (bx + bw / 2 - lw / 2).clamp(0, (w - lw).max(0));
            let under = by + bh + 2 * s;
            let ly = if under + 7 * s <= h {
                under
            } else {
                by - 9 * s
            };
            (lx, ly.clamp(0, (h - 7 * s).max(0)))
        }
        LockAt::Beyond(x, side) => {
            let x = x.clamp(0, w - 1);
            let lx = (x - lw / 2).clamp(0, (w - lw).max(0));
            if side < 0 {
                caret(frame, (x, 2 * s), 5 * s, -1, LOCK);
                (lx, 9 * s)
            } else {
                caret(frame, (x, h - 1 - 2 * s), 5 * s, 1, LOCK);
                (lx, h - 16 * s)
            }
        }
        LockAt::Off(side) => {
            let (ax, dir, lx) = if side < 0 {
                (2 * s, -2, 8 * s)
            } else {
                (w - 2 * s, 2, w - 8 * s - lw)
            };
            caret(frame, (ax, cy), 5 * s, dir, LOCK);
            (lx, cy - 3 * s)
        }
    };
    text(frame, lx, ly, &label, LOCK, s as usize);
    Some((lx, ly, lw, 7 * s))
}

/// **Where a health gauge sits over a sprite** (doc 21 cut G.14): its
/// `(x, y, w, h)` in pixels. [`HEALTH_PIPS`] pips `2s` square a gap `s`
/// apart — `8s` wide, `2s` tall — centred on the sprite's column (kept
/// inside the frame), its bottom `4s` over the sprite's top row: `2s` clear
/// of the lock's box, which starts `2s` over the sprite. Where the lock's
/// readout is drawn over its box instead of under it (the box running off
/// the bottom of the view) and the gauge would land on the readout, the
/// gauge moves up over the readout; and it never leaves the frame.
#[must_use]
pub fn health_rect(
    mark: &HealthMark,
    label: Option<(i64, i64, i64, i64)>,
    s: i64,
    w: i64,
    h: i64,
) -> (i64, i64, i64, i64) {
    let gw = (HEALTH_PIPS * 3 - 1) * s;
    let gh = 2 * s;
    let x = (mark.x - gw / 2).clamp(0, (w - gw).max(0));
    let mut y = mark.top - 6 * s;
    if let Some((lx, ly, lw, lh)) = label
        && x < lx + lw
        && lx < x + gw
        && y < ly + lh
        && ly < y + gh
    {
        y = ly - 4 * s;
    }
    (x, y.clamp(0, (h - gh).max(0)), gw, gh)
}

/// The health gauges (G.14): over each marked sprite, its lit pips in the
/// tier's colour and the rest in the doll's `gone` grey, where
/// [`health_rect`] puts them; and the contact's party letter `2s` right of
/// the pips in its relation's colour, its bottom on the gauge's (doc 32 cut
/// CF.0, Q-CF-12), kept inside the frame.
fn draw_health(
    frame: &mut Frame,
    marks: &[HealthMark],
    label: Option<(i64, i64, i64, i64)>,
    s: i64,
    w: i64,
    h: i64,
) {
    for mark in marks {
        let (x, y, _, gh) = health_rect(mark, label, s, w, h);
        for pip in 0..HEALTH_PIPS {
            let px = if pip < mark.health.lit() {
                mark.health.rgb()
            } else {
                GONE
            };
            fill_rect(frame, x + pip * 3 * s, y, 2 * s, gh, px);
        }
        if let Some(letter) = mark.party {
            let (lx, ly) = party_letter_at(x, y, gh, s, w);
            let mut buf = [0u8; 4];
            text(
                frame,
                lx,
                ly,
                letter.encode_utf8(&mut buf),
                kind_rgb(mark.kind),
                s as usize,
            );
        }
    }
}

/// Where a gauge's party letter goes (doc 32 cut CF.0, Q-CF-12): `2s` right
/// of the pips starting at `x` (`8s` wide), its 7-row glyph's bottom on the
/// gauge's bottom row (`y + gh`); left of the pips instead where the right
/// would leave the frame `w` wide. The glyph's top may run over the frame's
/// top edge, and is clipped there.
#[must_use]
pub fn party_letter_at(x: i64, y: i64, gh: i64, s: i64, w: i64) -> (i64, i64) {
    let gw = (HEALTH_PIPS * 3 - 1) * s;
    let lw = 5 * s;
    let right = x + gw + 2 * s;
    let lx = if right + lw <= w {
        right
    } else {
        x - 2 * s - lw
    };
    (lx, y + gh - 7 * s)
}

/// The speed bar, right of centre: filled from the bottom against
/// [`SPEED_FULL_KPH`] (or the next 20 above a faster speed), the speed over
/// it and `KPH` under it.
fn draw_speed(frame: &mut Frame, speed_kph: i64, s: i64, w: i64, h: i64) {
    let full = SPEED_FULL_KPH.max((speed_kph.max(0) + 19) / 20 * 20);
    let (bw, bh) = (4 * s, h * 2 / 5);
    let (x, y) = (w - 12 * s, h * 3 / 10);
    panel(frame, x, y, bw, bh);
    let fill = (bh - 2) * speed_kph.clamp(0, full) / full.max(1);
    fill_rect(frame, x + 1, y + bh - 1 - fill, bw - 2, fill, GOOD);
    let speed = format!("{}", speed_kph.max(0));
    let sw = speed.len() as i64 * 6 * s - s;
    text(frame, x + bw - sw, y - 9 * s, &speed, TEXT, s as usize);
    text(
        frame,
        x + bw - 17 * s,
        y + bh + 2 * s,
        "KPH",
        DIM,
        s as usize,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Det as _;

    /// Doc 21 cut G.13: the inset's square at the frame sizes the picture
    /// is drawn at, clear of the tape (which starts at `w/5`), the off-side
    /// arrow at mid-height and the speed bar; none on a frame too short.
    #[test]
    fn the_inset_sits_top_left_clear_of_the_hud() {
        for (w, h, want) in [
            (640, 480, Some((8, 8, 112))),
            (640, 276, Some((8, 8, 112))),
            (640, 256, Some((8, 8, 104))),
            (320, 220, Some((4, 4, 56))),
            (640, 60, None),
        ] {
            let frame = Frame::new(w, h);
            let got = inset_rect(&frame);
            assert_eq!(got, want, "{w}x{h}");
            if let Some((x, y, side)) = got {
                let s = scale_for(&frame) as i64;
                let (w, h) = (w as i64, h as i64);
                let (right, bottom) = (x + side as i64 + 1, y + side as i64 + 1);
                assert!(right < w / 5, "clear of the tape at {w}x{h}");
                assert!(bottom < h / 2 - 5 * s, "clear of the off arrow at {w}x{h}");
                assert!(right < w - 25 * s, "clear of the speed bar at {w}x{h}");
            }
        }
    }

    /// The minimap as it was drawn before doc 21 cut G.5a re-based it on the
    /// scene model, kept to prove the scene draws it pixel for pixel.
    /// The minimap, bottom-left: `view` cells on a side around `you`, north
    /// up, `scale` pixels a cell.
    fn draw_minimap_before_g5a(
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
            let (dx, dy) = h.to_radians().det_sin_cos();
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

    /// Doc 21 cut G.5a's acceptance: the minimap re-based on the scene model
    /// draws the very pixels it drew before — across both scales, the map's
    /// edges (cells off the map), spots, marks off and on the window, and
    /// eight headings.
    #[test]
    fn the_minimap_as_a_scene_is_pixel_equal_to_the_minimap_before() {
        let map = yard();
        let marks = [
            MapMark {
                cell: (30, 30),
                kind: SpriteKind::Hostile,
            },
            MapMark {
                cell: (50, 10),
                kind: SpriteKind::Friend,
            },
            MapMark {
                cell: (2, 60),
                kind: SpriteKind::Unknown,
            },
            MapMark {
                cell: (500, 500),
                kind: SpriteKind::Wreck,
            },
        ];
        let spots = [
            Spot {
                cell: (25, 3),
                letter: 'A',
            },
            Spot {
                cell: (90, 60),
                letter: 'B',
            },
        ];
        for (w, h) in [(320, 220), (640, 480), (640, 256)] {
            for you in [(6, 4), (48, 32), (95, 63), (0, 0)] {
                for heading in (0..8).map(|k| k * 45_000 + 7_000) {
                    let mut before = Frame::new(w, h);
                    draw_minimap_before_g5a(&mut before, &map, you, heading, &marks, &spots);
                    let mut after = Frame::new(w, h);
                    draw_minimap(&mut after, &map, you, heading, &marks, &spots);
                    assert!(before == after, "{w}x{h} at {you:?} heading {heading}");
                }
            }
        }
    }
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
                    kind: SpriteKind::Friend,
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
        // The tank's look's grid, every part at full, then the lookless
        // bars beside it.
        let doll = crate::look::for_unit("tacsim.unit.line_tank")
            .and_then(|look| look.doll.as_ref())
            .expect("the tank's doll");
        let full = vec![Some((10, 10)); doll.parts.len()];
        draw_doll(frame, Some((doll, full.as_slice())), &sections());
        draw_doll(frame, None, &sections());
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
