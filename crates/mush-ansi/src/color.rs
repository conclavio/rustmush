//! Color downgrade and SGR sequence construction.
//!
//! The downgrade chain is `Rgb -> nearest xterm-256 -> nearest ANSI-16 ->
//! plain`, driven by the connection's [`ColorDepth`]. Nearest-color lookups
//! run against palettes precomputed at compile time ([`PALETTE_256`],
//! [`ANSI16_RGB`]) — no external color crates.

use mush_proto::{Color, ColorDepth, Style};

/// Canonical xterm RGB values for the 16 system colors (indices 0–15).
///
/// Used only for *downgrading* richer colors toward the 16-color palette;
/// real terminals theme these freely.
pub const ANSI16_RGB: [(u8, u8, u8); 16] = [
    (0, 0, 0),       // 0 black
    (205, 0, 0),     // 1 red
    (0, 205, 0),     // 2 green
    (205, 205, 0),   // 3 yellow
    (0, 0, 238),     // 4 blue
    (205, 0, 205),   // 5 magenta
    (0, 205, 205),   // 6 cyan
    (229, 229, 229), // 7 white
    (127, 127, 127), // 8 bright black
    (255, 0, 0),     // 9 bright red
    (0, 255, 0),     // 10 bright green
    (255, 255, 0),   // 11 bright yellow
    (92, 92, 255),   // 12 bright blue
    (255, 0, 255),   // 13 bright magenta
    (0, 255, 255),   // 14 bright cyan
    (255, 255, 255), // 15 bright white
];

/// The 6 channel levels of the xterm 6x6x6 color cube.
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// The full xterm 256-color palette as RGB triples, computed at compile
/// time: 16 system colors, the 6x6x6 cube (16–231), grayscale ramp
/// (232–255).
pub const PALETTE_256: [(u8, u8, u8); 256] = build_palette_256();

const fn build_palette_256() -> [(u8, u8, u8); 256] {
    let mut p = [(0u8, 0u8, 0u8); 256];
    let mut i = 0;
    while i < 16 {
        p[i] = ANSI16_RGB[i];
        i += 1;
    }
    while i < 232 {
        let c = i - 16;
        p[i] = (CUBE[c / 36], CUBE[(c / 6) % 6], CUBE[c % 6]);
        i += 1;
    }
    while i < 256 {
        let v = (8 + 10 * (i - 232)) as u8;
        p[i] = (v, v, v);
        i += 1;
    }
    p
}

/// Squared RGB distance.
fn dist2(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let dr = i32::from(a.0) - i32::from(b.0);
    let dg = i32::from(a.1) - i32::from(b.1);
    let db = i32::from(a.2) - i32::from(b.2);
    (dr * dr + dg * dg + db * db) as u32
}

/// The nearest xterm 256-color index for an RGB value.
///
/// Only indices 16–255 (cube + grayscale) are candidates: the 16 system
/// colors are user-themed and unreliable as RGB targets. Ties resolve to
/// the lowest index.
#[must_use]
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    let want = (r, g, b);
    let mut best = 16u8;
    let mut best_d = u32::MAX;
    let mut i = 16usize;
    while i < 256 {
        let d = dist2(want, PALETTE_256[i]);
        if d < best_d {
            best_d = d;
            best = i as u8;
        }
        i += 1;
    }
    best
}

/// The nearest basic ANSI color (0–15) for an RGB value, measured against
/// [`ANSI16_RGB`]. Ties resolve to the lowest index.
#[must_use]
pub fn nearest_16(r: u8, g: u8, b: u8) -> u8 {
    let want = (r, g, b);
    let mut best = 0u8;
    let mut best_d = u32::MAX;
    for (i, &rgb) in ANSI16_RGB.iter().enumerate() {
        let d = dist2(want, rgb);
        if d < best_d {
            best_d = d;
            best = i as u8;
        }
    }
    best
}

/// Downgrade one color to what `depth` can display.
///
/// Returns `None` when the connection displays no color at all.
#[must_use]
pub fn downgrade(c: Color, depth: ColorDepth) -> Option<Color> {
    match depth {
        ColorDepth::None => None,
        ColorDepth::TrueColor => Some(c),
        ColorDepth::Ansi256 => Some(match c {
            Color::Rgb(r, g, b) => Color::Idx256(nearest_256(r, g, b)),
            other => other,
        }),
        ColorDepth::Ansi16 => Some(match c {
            Color::Ansi(n) => Color::Ansi(n % 16),
            Color::Idx256(i) if i < 16 => Color::Ansi(i),
            Color::Idx256(i) => {
                let (r, g, b) = PALETTE_256[i as usize];
                Color::Ansi(nearest_16(r, g, b))
            }
            Color::Rgb(r, g, b) => Color::Ansi(nearest_16(r, g, b)),
        }),
    }
}

/// SGR parameter fragment for one color: `base` is 30 (foreground) or 40
/// (background).
fn color_params(c: Color, base: u8) -> String {
    match c {
        Color::Ansi(n) => {
            let n = n % 16;
            if n < 8 {
                format!("{}", base + n)
            } else {
                // Bright variants: 90–97 fg, 100–107 bg.
                format!("{}", base + 60 + (n - 8))
            }
        }
        Color::Idx256(i) => format!("{};5;{i}", base + 8),
        Color::Rgb(r, g, b) => format!("{};2;{r};{g};{b}", base + 8),
    }
}

/// The complete SGR escape sequence for `style` at `depth`, downgrading
/// colors as needed. Empty when nothing survives (plain style, or
/// [`ColorDepth::None`] which strips *all* attributes including bold).
#[must_use]
pub fn sgr(style: Style, depth: ColorDepth) -> String {
    if matches!(depth, ColorDepth::None) {
        return String::new();
    }
    let mut parts: Vec<String> = Vec::new();
    if style.bold {
        parts.push("1".to_string());
    }
    if style.italic {
        parts.push("3".to_string());
    }
    if style.underline {
        parts.push("4".to_string());
    }
    if let Some(c) = style.fg.and_then(|c| downgrade(c, depth)) {
        parts.push(color_params(c, 30));
    }
    if let Some(c) = style.bg.and_then(|c| downgrade(c, depth)) {
        parts.push(color_params(c, 40));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", parts.join(";"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_shape() {
        assert_eq!(PALETTE_256[0], (0, 0, 0));
        assert_eq!(PALETTE_256[15], (255, 255, 255));
        assert_eq!(PALETTE_256[16], (0, 0, 0)); // cube (0,0,0)
        assert_eq!(PALETTE_256[196], (255, 0, 0)); // cube (5,0,0)
        assert_eq!(PALETTE_256[231], (255, 255, 255)); // cube (5,5,5)
        assert_eq!(PALETTE_256[232], (8, 8, 8)); // grayscale start
        assert_eq!(PALETTE_256[255], (238, 238, 238)); // grayscale end
    }

    #[test]
    fn nearest_256_spot_checks() {
        assert_eq!(nearest_256(255, 0, 0), 196); // exact cube hit
        assert_eq!(nearest_256(128, 128, 128), 244); // exact grayscale hit
        assert_eq!(nearest_256(0, 0, 0), 16); // black -> cube origin, not system 0
        assert_eq!(nearest_256(0, 95, 135), 24); // cube (0,1,2)
    }

    #[test]
    fn nearest_16_spot_checks() {
        assert_eq!(nearest_16(255, 0, 0), 9); // exact bright red
        assert_eq!(nearest_16(200, 0, 0), 1); // near plain red
        assert_eq!(nearest_16(0, 0, 0), 0);
        assert_eq!(nearest_16(128, 128, 128), 8);
        assert_eq!(nearest_16(250, 250, 250), 15);
    }

    #[test]
    fn downgrade_chain() {
        let red = Color::Rgb(255, 0, 0);
        assert_eq!(downgrade(red, ColorDepth::TrueColor), Some(red));
        assert_eq!(
            downgrade(red, ColorDepth::Ansi256),
            Some(Color::Idx256(196))
        );
        assert_eq!(downgrade(red, ColorDepth::Ansi16), Some(Color::Ansi(9)));
        assert_eq!(downgrade(red, ColorDepth::None), None);
        // 256 index downgrades through its palette RGB.
        assert_eq!(
            downgrade(Color::Idx256(196), ColorDepth::Ansi16),
            Some(Color::Ansi(9))
        );
        // Low 256 indices are already system colors.
        assert_eq!(
            downgrade(Color::Idx256(3), ColorDepth::Ansi16),
            Some(Color::Ansi(3))
        );
        // Basic ANSI passes through everywhere colored.
        assert_eq!(
            downgrade(Color::Ansi(6), ColorDepth::Ansi256),
            Some(Color::Ansi(6))
        );
    }

    #[test]
    fn sgr_sequences() {
        let cyan = Style::default().with_fg(Color::Ansi(6));
        assert_eq!(sgr(cyan, ColorDepth::Ansi16), "\x1b[36m");
        let bright = Style::default().with_fg(Color::Ansi(14));
        assert_eq!(sgr(bright, ColorDepth::Ansi16), "\x1b[96m");
        let full = Style::default()
            .with_bold()
            .with_italic()
            .with_underline()
            .with_fg(Color::Rgb(1, 2, 3))
            .with_bg(Color::Ansi(0));
        assert_eq!(
            sgr(full, ColorDepth::TrueColor),
            "\x1b[1;3;4;38;2;1;2;3;40m"
        );
        assert_eq!(sgr(Style::default(), ColorDepth::TrueColor), "");
        // None strips everything, bold included.
        assert_eq!(sgr(full, ColorDepth::None), "");
        // 256 fg/bg forms.
        let idx = Style::default()
            .with_fg(Color::Idx256(208))
            .with_bg(Color::Idx256(17));
        assert_eq!(sgr(idx, ColorDepth::Ansi256), "\x1b[38;5;208;48;5;17m");
    }
}
