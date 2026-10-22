//! **The sixel encoder** (doc 21 §4.2, D-21-3; cut G.2): the same [`Frame`]
//! as a DEC sixel stream, for the terminals that draw sixel and not the kitty
//! protocol — Windows Terminal above all, which is how a Windows player who
//! types `ssh` in PowerShell sees the picture at all.
//!
//! The stream: `DCS 0;1;0 q`, the raster attributes `"1;1;W;H` (square
//! pixels, the frame's size), one colour register a colour the frame uses
//! (`#i;2;r;g;b`, percent), then the picture in bands six rows tall — for
//! each colour present in a band, `#i` and one sixel character a column (the
//! six rows' bits plus 63), runs of four or more written `!n<char>`, `$` back
//! to the band's start between colours and `-` down to the next band — and
//! `ST`. A colour's line stops at its last set column, since a sixel with no
//! bits paints nothing.
//!
//! Unlike kitty there is no image id: sixel pixels are drawn into the cell
//! grid at the cursor, at the terminal's own pixel size, and are gone the
//! moment text is written over them. So the frame is not scaled by the
//! terminal — the renderer sizes it to the cell box ([`fit`]) — and the whole
//! picture is sent again whenever the text frame touched it.
//!
//! **Its clock is the picture's own, 20 Hz for the cockpit** (the owner,
//! 2026-10-22, revising D-21-16's 10 Hz once G.2 measured it): the 10 Hz
//! default assumed a sixel frame twice the PNG's bytes, and on the flat
//! cockpit frame it is not — 20.6 KB raw against kitty's 9.4 KB, but 6.6 KB
//! each after zlib, so on a compressed SSH link the two cost the same.
//!
//! The frame's colours are a closed set (D-21-7), at most 256, so every
//! colour gets its own register. A frame with more is cut to 3-3-2 bits a
//! channel first, which the producers never trigger and a test pins.

use std::collections::HashMap;

use crate::Frame;
use crate::frame::Rgb;

/// Begin a sixel image: pixel aspect from the raster attributes (`0`), the
/// pixels the image does not paint left as they are (`1`; every pixel of a
/// frame is painted), grid size unset (`0`).
const DCS_SIXEL: &[u8] = b"\x1bP0;1;0q";
/// String terminator.
const ST: &[u8] = b"\x1b\\";

/// `frame` as a sixel stream, from `DCS` to `ST`. Placement (the cursor
/// move before it) is the caller's.
#[must_use]
pub fn encode(frame: &Frame) -> Vec<u8> {
    let (w, h) = (frame.width(), frame.height());
    let (palette, index) = indexed(frame);
    let mut out = Vec::with_capacity(w * h / 4 + palette.len() * 20 + 64);
    out.extend_from_slice(DCS_SIXEL);
    out.extend_from_slice(format!("\"1;1;{w};{h}").as_bytes());
    for (i, rgb) in palette.iter().enumerate() {
        let [r, g, b] = rgb.map(percent);
        out.extend_from_slice(format!("#{i};2;{r};{g};{b}").as_bytes());
    }
    if w == 0 {
        out.extend_from_slice(ST);
        return out;
    }

    // One band's colour lines: `slot[colour]` is the colour's line in
    // `lines` for this band (or `u16::MAX`), `used` the colours in the order
    // they first appear in the band.
    let mut slot = vec![u16::MAX; palette.len()];
    let mut used: Vec<usize> = Vec::new();
    let mut lines: Vec<u8> = Vec::new();
    let rows: Vec<&[u8]> = index.chunks_exact(w).collect();
    for (band, band_rows) in rows.chunks(6).enumerate() {
        if band > 0 {
            out.push(b'-');
        }
        for &c in &used {
            if let Some(s) = slot.get_mut(c) {
                *s = u16::MAX;
            }
        }
        used.clear();
        lines.clear();
        for (bit, row) in band_rows.iter().enumerate() {
            for (x, &c) in row.iter().enumerate() {
                let c = usize::from(c);
                let Some(s) = slot.get_mut(c) else {
                    continue;
                };
                if *s == u16::MAX {
                    *s = u16::try_from(used.len()).unwrap_or(u16::MAX - 1);
                    used.push(c);
                    lines.resize(lines.len() + w, 0);
                }
                if let Some(cell) = lines.get_mut(usize::from(*s) * w + x) {
                    *cell |= 1 << bit;
                }
            }
        }
        for (k, (&c, line)) in used.iter().zip(lines.chunks_exact(w)).enumerate() {
            if k > 0 {
                out.push(b'$');
            }
            out.extend_from_slice(format!("#{c}").as_bytes());
            let end = line
                .iter()
                .rposition(|&bits| bits != 0)
                .map_or(0, |i| i + 1);
            push_runs(&mut out, line.get(..end).unwrap_or(&[]));
        }
    }
    out.extend_from_slice(ST);
    out
}

/// `bits` (one column's six bits each) as sixel characters, a run of four
/// or more as `!n<char>`.
fn push_runs(out: &mut Vec<u8>, bits: &[u8]) {
    let mut i = 0;
    while let Some(&b) = bits.get(i) {
        let run = bits
            .get(i..)
            .map_or(1, |rest| rest.iter().take_while(|&&x| x == b).count());
        let ch = b + 63;
        if run >= 4 {
            out.extend_from_slice(format!("!{run}").as_bytes());
            out.push(ch);
        } else {
            out.extend(std::iter::repeat_n(ch, run));
        }
        i += run;
    }
}

/// A channel as sixel's percent, `0..=100`.
fn percent(v: u8) -> u8 {
    ((u16::from(v) * 100 + 127) / 255) as u8
}

/// The frame's palette in first-appearance order and each pixel's index.
/// Long runs of one colour are the rule, so the last lookup is remembered
/// (as the PNG encoder does). More than 256 colours: cut to 3-3-2 and index
/// again, which always fits.
fn indexed(frame: &Frame) -> (Vec<Rgb>, Vec<u8>) {
    index_pixels(frame.rgb().as_chunks::<3>().0, |px| px).unwrap_or_else(|| {
        let cut = |[r, g, b]: Rgb| [r & 0xe0, g & 0xe0, b & 0xc0];
        index_pixels(frame.rgb().as_chunks::<3>().0, cut).unwrap_or_default()
    })
}

fn index_pixels(pixels: &[Rgb], map: impl Fn(Rgb) -> Rgb) -> Option<(Vec<Rgb>, Vec<u8>)> {
    let mut palette: Vec<Rgb> = Vec::new();
    let mut lookup: HashMap<Rgb, u8> = HashMap::new();
    let mut index = Vec::with_capacity(pixels.len());
    let mut last: Option<(Rgb, u8)> = None;
    for &raw in pixels {
        let i = match last {
            Some((colour, i)) if colour == raw => i,
            _ => {
                let px = map(raw);
                let i = match lookup.get(&px) {
                    Some(&i) => i,
                    None => {
                        let i = u8::try_from(palette.len()).ok()?;
                        palette.push(px);
                        lookup.insert(px, i);
                        i
                    }
                };
                last = Some((raw, i));
                i
            }
        };
        index.push(i);
    }
    Some((palette, index))
}

/// The frame size a sixel picture takes: `want` (the view's chosen frame,
/// 640x480 or 320x220) if it fits in `avail` pixels, else the largest frame
/// of the same aspect that does, never less than one pixel a side.
#[must_use]
pub fn fit(want: (usize, usize), avail: (usize, usize)) -> (usize, usize) {
    let (ww, wh) = (want.0.max(1), want.1.max(1));
    let (aw, ah) = (avail.0.max(1), avail.1.max(1));
    if ww <= aw && wh <= ah {
        return (ww, wh);
    }
    // Scale by the tighter side, rounding down so it fits.
    if aw * wh <= ah * ww {
        (aw, (aw * wh / ww).max(1))
    } else {
        ((ah * ww / wh).max(1), ah)
    }
}

/// Wrap one encoded picture for the wire: synchronized output, the cursor at
/// cell (`col`, `row`) (1-based, as `CUP` counts), the sixel stream.
#[must_use]
pub fn frame_bytes(sixel: &[u8], col: u16, row: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(sixel.len() + 32);
    out.extend_from_slice(crate::kitty::SYNC_BEGIN);
    out.extend_from_slice(format!("\x1b[{row};{col}H").as_bytes());
    out.extend_from_slice(sixel);
    out.extend_from_slice(crate::kitty::SYNC_END);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sixel decoder for the encoder's own subset: the raster attributes,
    /// `#i;2;r;g;b` definitions, `#i` selections, sixel characters, `!n`
    /// repeats, `$` and `-`. Returns the size and the pixels, unpainted ones
    /// `None`.
    fn decode(stream: &[u8]) -> (usize, usize, Vec<Option<Rgb>>) {
        let body = stream
            .strip_prefix(DCS_SIXEL)
            .and_then(|b| b.strip_suffix(ST))
            .unwrap_or_default();
        let mut pos = 0usize;
        let num = |pos: &mut usize| {
            let start = *pos;
            while body.get(*pos).is_some_and(u8::is_ascii_digit) {
                *pos += 1;
            }
            std::str::from_utf8(body.get(start..*pos).unwrap_or_default())
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0)
        };
        assert_eq!(body.first(), Some(&b'"'), "raster attributes first");
        pos += 1;
        let mut raster = Vec::new();
        for k in 0..4 {
            if k > 0 {
                assert_eq!(body.get(pos), Some(&b';'));
                pos += 1;
            }
            raster.push(num(&mut pos));
        }
        let [pan, pad, w, h] = raster.as_slice() else {
            return (0, 0, Vec::new());
        };
        assert_eq!((*pan, *pad), (1, 1), "square pixels");
        let (w, h) = (*w, *h);
        let mut pixels: Vec<Option<Rgb>> = vec![None; w * h];
        let mut registers: HashMap<usize, Rgb> = HashMap::new();
        let (mut x, mut band, mut colour) = (0usize, 0usize, 0usize);
        let unpercent = |p: usize| ((p * 255 + 50) / 100) as u8;
        while let Some(&b) = body.get(pos) {
            pos += 1;
            match b {
                b'#' => {
                    let i = num(&mut pos);
                    if body.get(pos) == Some(&b';') {
                        pos += 1;
                        assert_eq!(num(&mut pos), 2, "RGB definitions only");
                        let mut c = [0u8; 3];
                        for v in &mut c {
                            assert_eq!(body.get(pos), Some(&b';'));
                            pos += 1;
                            *v = unpercent(num(&mut pos));
                        }
                        registers.insert(i, c);
                    } else {
                        colour = i;
                    }
                }
                b'$' => x = 0,
                b'-' => {
                    x = 0;
                    band += 1;
                }
                b'!' => {
                    let n = num(&mut pos);
                    let ch = body.get(pos).copied().unwrap_or(b'?');
                    pos += 1;
                    for _ in 0..n {
                        paint(&mut pixels, w, h, x, band, ch, registers.get(&colour));
                        x += 1;
                    }
                }
                other => {
                    assert!(
                        (63..=126).contains(&other),
                        "unexpected byte {other} in the sixel body"
                    );
                    paint(&mut pixels, w, h, x, band, other, registers.get(&colour));
                    x += 1;
                }
            }
        }
        (w, h, pixels)
    }

    fn paint(
        pixels: &mut [Option<Rgb>],
        w: usize,
        h: usize,
        x: usize,
        band: usize,
        ch: u8,
        colour: Option<&Rgb>,
    ) {
        let bits = ch - 63;
        for k in 0..6 {
            let y = band * 6 + k;
            if bits & (1 << k) != 0
                && x < w
                && y < h
                && let Some(px) = pixels.get_mut(y * w + x)
            {
                *px = colour.copied();
            }
        }
    }

    /// What a channel comes back as after the trip through percent.
    fn through_percent(px: Rgb) -> Rgb {
        px.map(|v| ((usize::from(percent(v)) * 255 + 50) / 100) as u8)
    }

    fn assert_round_trips(frame: &Frame) {
        let stream = encode(frame);
        let (w, h, pixels) = decode(&stream);
        assert_eq!((w, h), (frame.width(), frame.height()));
        for y in 0..h {
            for x in 0..w {
                assert_eq!(
                    pixels.get(y * w + x).copied().flatten(),
                    Some(through_percent(frame.get(x, y))),
                    "pixel {x},{y}"
                );
            }
        }
    }

    /// The byte form, pinned on a frame small enough to read: two colours,
    /// seven rows (a full band and one row of the next), a run long enough
    /// to repeat.
    #[test]
    fn a_small_frame_encodes_to_these_bytes() {
        let mut f = Frame::new(5, 7);
        f.clear([255, 0, 0]);
        for y in 0..7 {
            f.put(4, y, [0, 0, 255]);
        }
        let s = String::from_utf8(encode(&f)).unwrap_or_default();
        assert_eq!(
            s,
            "\x1bP0;1;0q\"1;1;5;7#0;2;100;0;0#1;2;0;0;100#0!4~$#1!4?~-#0!4@$#1!4?@\x1b\\"
        );
    }

    #[test]
    fn the_cockpit_frame_decodes_back_to_its_pixels() {
        let map: serde_json::Value = serde_json::from_str(include_str!(
            "../../../web/src/lib/fixtures/cockpit/arena_map.json"
        ))
        .unwrap_or_default();
        let cockpit: serde_json::Value = serde_json::from_str(include_str!(
            "../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
        ))
        .unwrap_or_default();
        let feeds: HashMap<String, serde_json::Value> = [
            (crate::cockpit::COCKPIT_FEED.to_string(), cockpit),
            (crate::cockpit::ARENA_MAP_FEED.to_string(), map),
        ]
        .into_iter()
        .collect();
        for (w, h) in [(640, 480), (320, 220), (333, 250)] {
            let mut frame = Frame::new(w, h);
            let mut picture = crate::cockpit::CockpitPicture::new();
            assert!(crate::picture::Picture::draw(
                &mut picture,
                &mut frame,
                &feeds,
                &crate::picture::Look::default()
            ));
            assert_round_trips(&frame);
        }
    }

    #[test]
    fn edge_frames_round_trip() {
        // Heights that are and are not a multiple of six, one column, empty.
        for (w, h) in [(1, 1), (1, 6), (3, 13), (9, 12)] {
            let mut f = Frame::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    f.put(x, y, [(x * 40) as u8, (y * 20) as u8, 7]);
                }
            }
            assert_round_trips(&f);
        }
        let empty = encode(&Frame::new(0, 0));
        assert_eq!(empty, b"\x1bP0;1;0q\"1;1;0;0\x1b\\");
    }

    /// More than 256 colours is never drawn by a producer, but it must still
    /// encode: cut to 3-3-2, every register under 256.
    #[test]
    fn a_frame_past_256_colours_is_cut_to_fit() {
        let mut f = Frame::new(64, 8);
        for y in 0..8 {
            for x in 0..64 {
                f.put(x, y, [(x * 4) as u8, (y * 32) as u8, (x + y) as u8]);
            }
        }
        assert!(f.count_colours(256) > 256);
        let (palette, index) = indexed(&f);
        assert!(palette.len() <= 256);
        assert_eq!(index.len(), 64 * 8);
        let (w, h, pixels) = decode(&encode(&f));
        assert_eq!((w, h), (64, 8));
        assert!(pixels.iter().all(Option::is_some), "every pixel painted");
    }

    #[test]
    fn fit_keeps_the_want_or_shrinks_to_the_aspect() {
        assert_eq!(fit((640, 480), (1600, 900)), (640, 480));
        // Height-bound: 400 px of room under a 480 px frame.
        assert_eq!(fit((640, 480), (1000, 400)), (533, 400));
        // Width-bound.
        assert_eq!(fit((640, 480), (320, 900)), (320, 240));
        assert_eq!(fit((640, 480), (0, 0)), (1, 1));
    }
}
