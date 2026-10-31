//! **The armour doll as a picture** (doc 21 cut G.6a; bucket B): the
//! pilot's own tank from the held `arena.cockpit` frame, drawn large — the
//! unit's look's doll grid (doc 22), each part coloured by what is left of it
//! and touching parts toned apart — beside a legend that names every section:
//! its swatch, its condition, its internals as a bar and a count, and each
//! facet's armour under it. A unit with no look draws the legend alone.
//!
//! The doll is the cockpit's own instrument made a page (the first-person
//! picture carries a small one); the fitting paper doll with hardpoints and
//! arcs (G.6's other half) waits on a fitting feature to draw.

use std::time::Duration;

use serde_json::Value;

use crate::Frame;
use crate::cockpit::{COCKPIT_FEED, sections_from};
use crate::frame::Rgb;
use crate::overlay::{
    BAD, DIM, GONE, PANEL, Section, TEXT, draw_card, fill_rect, lighter, outline_rect, panel, ramp,
    text,
};
use crate::picture::{BUCKET_B_INTERVAL, Feeds, Look, Picture};

const BACK: Rgb = [10, 12, 16];
const EDGE: Rgb = [96, 104, 120];

#[cfg(feature = "encode")]
use crate::overhead::{place_kitty_subrect, wrap_kitty_sync};

/// Retained Kitty image ID (`i=181`) for the armour-status doll picture.
#[cfg(feature = "encode")]
pub const IMAGE_ID_DOLL: u32 = 181;
/// Retained Kitty placement ID (`p=81`) for the armour-status doll picture.
#[cfg(feature = "encode")]
pub const PLACEMENT_DOLL: u32 = 81;
#[cfg(feature = "encode")]
const Z_DOLL: i32 = -50;

/// The doll as a [`Picture`]: drawn from the held `arena.cockpit` on bucket
/// B's clock (on change).
#[derive(Debug, Default)]
pub struct DollPicture {
    #[cfg(feature = "encode")]
    kitty_hash: Option<u64>,
    #[cfg(feature = "encode")]
    kitty_area: Option<(u16, u16, u16, u16)>,
}

impl DollPicture {
    /// A doll picture.
    #[must_use]
    pub fn new() -> DollPicture {
        DollPicture::default()
    }

    /// Return Kitty Graphics escape bytes that delete the retained doll image
    /// (`IMAGE_ID_DOLL`), and reset cached placement state.
    #[cfg(feature = "encode")]
    pub fn cleanup_kitty_bytes(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.kitty_hash.is_some() {
            crate::kitty::delete_image_bytes(&mut out, IMAGE_ID_DOLL);
        }
        self.kitty_hash = None;
        self.kitty_area = None;
        out
    }

    /// Invalidate the retained Kitty doll image if the terminal evicted it.
    #[cfg(feature = "encode")]
    pub fn invalidate_image(&mut self, image_id: u32) {
        if image_id == IMAGE_ID_DOLL {
            self.kitty_hash = None;
            self.kitty_area = None;
        }
    }

    /// Force the retained Kitty doll placement to re-emit on the next tick.
    #[cfg(feature = "encode")]
    pub fn invalidate_placements(&mut self) {
        self.kitty_area = None;
    }

    /// Render the armour-status doll page as a retained Kitty Graphics stream
    /// (`a=t` + `a=p`), scaling up to `1280x` when `cell` is known and
    /// `!step_down` (`640x` fallback otherwise).
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
            crate::kitty::transmit_bytes(&mut ops, &png, IMAGE_ID_DOLL);
            self.kitty_hash = Some(h);
            self.kitty_area = None;
        }
        if self.kitty_area != Some(placed_area) {
            place_kitty_subrect(
                &mut ops,
                IMAGE_ID_DOLL,
                PLACEMENT_DOLL,
                placed_area,
                (0, 0, vw as u32, vh as u32),
                Z_DOLL,
            );
            self.kitty_area = Some(placed_area);
        }
        Some((vp, wrap_kitty_sync(ops)))
    }
}

impl Picture for DollPicture {
    fn feeds(&self) -> &'static [&'static str] {
        &[COCKPIT_FEED]
    }

    fn interval(&self) -> Duration {
        BUCKET_B_INTERVAL
    }

    fn draw(&mut self, frame: &mut Frame, feeds: &dyn Feeds, _look: &Look) -> bool {
        match feeds
            .feed(COCKPIT_FEED)
            .filter(|c| c.get("seated").and_then(Value::as_bool) != Some(false))
        {
            Some(cockpit) => draw_doll_page(frame, cockpit),
            None => draw_card(frame, "NOT IN A COCKPIT"),
        }
        true
    }
}

/// The colour a part or section is drawn in: what is left of it, toned
/// lighter where `tone` says it touches another in the same state.
fn colour(left: Option<(i64, i64)>, tone: bool) -> Rgb {
    let px = left.map_or(GONE, |(l, m)| ramp(l, m));
    if tone { lighter(px) } else { px }
}

/// The page: a title, the doll on the left, the legend on the right.
pub fn draw_doll_page(frame: &mut Frame, cockpit: &Value) {
    let (w, h) = (frame.width() as i64, frame.height() as i64);
    let s: i64 = if w.min(h) >= 360 { 2 } else { 1 };
    fill_rect(frame, 0, 0, w, h, BACK);
    let sections = sections_from(cockpit);
    let look = cockpit
        .get("unit_type")
        .and_then(Value::as_str)
        .and_then(crate::look::for_unit);
    let doll = look.and_then(|l| l.doll.as_ref());
    let callsign = cockpit
        .get("callsign")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let unit = look.map_or("UNKNOWN UNIT", |l| l.label.as_str());
    let title = format!("ARMOUR - {callsign} - {unit}");
    let fits = usize::try_from((w - 4 * s) / (6 * s)).unwrap_or(0);
    text(
        frame,
        2 * s,
        2 * s,
        &title.chars().take(fits).collect::<String>().to_uppercase(),
        TEXT,
        s as usize,
    );
    let top = 13 * s;
    let room_h = h - top - 4 * s;
    // The legend is sized first, from its fixed columns. A wide frame (the
    // text cockpit's picture) puts the doll on the left and the legend
    // beside it; a tall one (4:3) puts the doll on top, centred, and the
    // legend under it.
    let line = 10 * s;
    let legend_w = LEGEND_CHARS * 6 * s + BAR * s + 14 * s;
    let legend_h: i64 = sections
        .iter()
        .map(|sec| line * (1 + sec.facets.len() as i64) + 2 * s)
        .sum();
    let stacked = h * 5 >= w * 3;
    let (room_w, room_dh) = if stacked {
        (w - 8 * s, (room_h - legend_h - 6 * s).max(room_h / 3))
    } else {
        ((w - legend_w - 16 * s).max(w / 4), room_h)
    };

    // The doll, as large as that room allows.
    let mut doll_bottom = top;
    let legend_x = match doll {
        Some(doll) if doll.width > 0 && doll.height > 0 => {
            let condition = doll.condition(cockpit.get("sections").unwrap_or(&Value::Null));
            let tones = doll.tones();
            let c = (room_w / doll.width as i64)
                .min(room_dh / doll.height as i64)
                .max(1);
            let (dw, dh) = (doll.width as i64 * c, doll.height as i64 * c);
            let (x0, y0) = if stacked {
                ((w - dw) / 2, top + 2 * s)
            } else {
                (4 * s, top + (room_h - dh) / 2)
            };
            panel(frame, x0 - 2 * s, y0 - 2 * s, dw + 4 * s, dh + 4 * s);
            let part_at =
                |x: usize, y: usize| doll.cells.get(y * doll.width + x).copied().flatten();
            for y in 0..doll.height {
                for x in 0..doll.width {
                    let Some(part) = part_at(x, y) else {
                        continue;
                    };
                    let px = colour(
                        condition.get(part).copied().flatten(),
                        tones.get(part) == Some(&1),
                    );
                    let (cx, cy) = (x0 + x as i64 * c, y0 + y as i64 * c);
                    fill_rect(frame, cx, cy, c, c, px);
                    if x + 1 >= doll.width || part_at(x + 1, y) != Some(part) {
                        fill_rect(frame, cx + c - 1, cy, 1, c, PANEL);
                    }
                    if part_at(x, y + 1) != Some(part) {
                        fill_rect(frame, cx, cy + c - 1, c, 1, PANEL);
                    }
                }
            }
            doll_bottom = y0 + dh + 6 * s;
            if stacked {
                ((w - legend_w) / 2).max(4 * s)
            } else {
                x0 + dw + 8 * s
            }
        }
        _ => 4 * s,
    };

    // The legend: a section a block, its facets under it; a second column
    // where the first runs out of room and two whole legends fit across.
    let legend_top = if stacked { doll_bottom } else { top };
    let room = (w - legend_x - 2 * s).max(0);
    let columns = if room >= 2 * legend_w { 2 } else { 1 };
    let col_w = legend_w;
    let (mut col, mut y) = (0i64, legend_top);
    for sec in &sections {
        let needed = line * (1 + sec.facets.len() as i64);
        if y + needed > h - 2 * s && y > legend_top {
            col += 1;
            y = legend_top;
        }
        if col >= columns {
            break;
        }
        let x = legend_x + col * col_w;
        draw_section(frame, sec, x, y, col_w, s);
        y += needed + 2 * s;
    }
}

/// The legend's text columns, characters: a section's name, its condition
/// word, and its count (the bar sits between the word and the count).
const NAME_CHARS: i64 = 12;
const WORD_CHARS: i64 = 10;
const COUNT_CHARS: i64 = 8;
const LEGEND_CHARS: i64 = NAME_CHARS + WORD_CHARS + COUNT_CHARS;
/// The bar's width, in the scale's pixels.
const BAR: i64 = 44;

/// One section's block in the legend at `(x, y)`, `_width` wide: swatch,
/// name, condition, a bar and the count; each facet under it, its bar in
/// the same column.
fn draw_section(frame: &mut Frame, sec: &Section, x: i64, y: i64, _width: i64, s: i64) {
    let left = (sec.internals_max > 0).then_some((sec.internals, sec.internals_max));
    let swatch = colour(left, false);
    fill_rect(frame, x, y, 7 * s, 7 * s, swatch);
    outline_rect(frame, x, y, 7 * s, 7 * s, EDGE);
    let name: String = sec
        .id
        .replace('_', " ")
        .to_uppercase()
        .chars()
        .take(NAME_CHARS as usize - 1)
        .collect();
    let destroyed = sec.condition == "destroyed";
    text(frame, x + 10 * s, y, &name, TEXT, s as usize);
    let tx = x + 10 * s + NAME_CHARS * 6 * s;
    let word: String = sec
        .condition
        .to_uppercase()
        .chars()
        .take(WORD_CHARS as usize - 1)
        .collect();
    text(
        frame,
        tx,
        y,
        &word,
        if destroyed { BAD } else { DIM },
        s as usize,
    );
    let count = format!("{}/{}", sec.internals, sec.internals_max);
    let bar_x = tx + WORD_CHARS * 6 * s;
    let bar_w = BAR * s;
    bar(frame, bar_x, y + s, bar_w, 5 * s, left);
    text(frame, bar_x + bar_w + 4 * s, y, &count, TEXT, s as usize);
    for (i, f) in sec.facets.iter().enumerate() {
        let fy = y + 10 * s * (i as i64 + 1);
        let fl = (f.armor_max > 0).then_some((f.armor, f.armor_max));
        text(frame, x + 16 * s, fy, &f.id.to_uppercase(), DIM, s as usize);
        bar(frame, bar_x, fy + s, bar_w, 4 * s, fl);
        text(
            frame,
            bar_x + bar_w + 4 * s,
            fy,
            &format!("{}/{}", f.armor, f.armor_max),
            DIM,
            s as usize,
        );
    }
}

/// A bar `w` x `h`: filled to what is left, coloured by it, on a dark track.
fn bar(frame: &mut Frame, x: i64, y: i64, w: i64, h: i64, left: Option<(i64, i64)>) {
    if w <= 0 {
        return;
    }
    fill_rect(frame, x, y, w, h, PANEL);
    if let Some((l, m)) = left.filter(|(_, m)| *m > 0) {
        let fill = (w * l.clamp(0, m) / m).max(0);
        fill_rect(frame, x, y, fill, h, ramp(l, m));
    }
    outline_rect(frame, x, y, w, h, EDGE);
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::overlay::GOOD;

    fn cockpit() -> Value {
        serde_json::from_str(include_str!(
            "../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
        ))
        .expect("cockpit fixture")
    }

    fn draw(c: Option<Value>, w: usize, h: usize) -> Frame {
        let mut f = Frame::new(w, h);
        let feeds: HashMap<String, Value> = c
            .into_iter()
            .map(|c| (COCKPIT_FEED.to_string(), c))
            .collect();
        assert!(DollPicture::new().draw(&mut f, &feeds, &Look::default()));
        f
    }

    fn has(f: &Frame, c: Rgb) -> bool {
        (0..f.height()).any(|y| (0..f.width()).any(|x| f.get(x, y) == c))
    }

    /// The fixture's tank drawn: its doll in its parts' colours and the
    /// legend with the sections' bars; a hit section's colour moves.
    #[test]
    fn the_doll_page_draws_the_tank_and_its_legend() {
        let f = draw(Some(cockpit()), 640, 230);
        assert!(has(&f, GOOD), "a whole part is green");
        assert!(has(&f, TEXT), "the title and legend");
        assert!(f.count_colours(256) <= 64);
        let mut hurt = cockpit();
        for sec in hurt["sections"].as_array_mut().unwrap() {
            if sec["id"] == "turret" {
                sec["internals"] = 1.into();
                sec["condition"] = "damaged".into();
            }
        }
        assert_ne!(
            draw(Some(hurt), 640, 230).hash(),
            f.hash(),
            "the turret's colour moved"
        );
        assert_ne!(draw(Some(cockpit()), 640, 480).hash(), f.hash());
    }

    /// No seat is a card; an unknown unit draws the legend alone.
    #[test]
    fn no_seat_is_a_card_and_no_look_is_the_legend() {
        let empty = draw(None, 320, 220);
        let unseated = draw(Some(serde_json::json!({"seated": false})), 320, 220);
        assert_eq!(empty.hash(), unseated.hash());
        let mut c = cockpit();
        c["unit_type"] = "nothing_known".into();
        let f = draw(Some(c), 640, 230);
        assert!(has(&f, TEXT));
    }

    #[cfg(feature = "encode")]
    #[test]
    fn retained_kitty_doll_scales_to_1280_and_skips_unchanged_ticks() {
        use crate::caps::CellSize;
        let feeds: HashMap<String, Value> = [(COCKPIT_FEED.to_string(), cockpit())]
            .into_iter()
            .collect();
        let mut pic = DollPicture::new();
        let area = (1, 1, 128, 36);
        let cell = Some(CellSize {
            width: 10,
            height: 20,
        });
        let (vp, b1) = pic
            .draw_kitty(&feeds, &Look::default(), area, cell, false)
            .expect("drawn");
        assert_eq!(vp.width_px, 1280);
        let s1 = String::from_utf8_lossy(&b1);
        assert!(s1.contains("a=t,f=100,t=d,i=181"));
        assert!(s1.contains("a=p,i=181,p=81"));

        let (_, b2) = pic
            .draw_kitty(&feeds, &Look::default(), area, cell, false)
            .expect("drawn");
        assert!(b2.is_empty(), "unchanged doll emits 0 bytes");

        let (vp_fb, _) = pic
            .draw_kitty(&feeds, &Look::default(), area, None, false)
            .expect("fallback");
        assert_eq!(vp_fb.width_px, 640);
    }
}
