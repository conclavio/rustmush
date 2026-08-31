//! [`SshBackend`] — a clean-room ratatui [`Backend`] that renders into a
//! `Vec<u8>` of VT100/ANSI bytes instead of a live tty (design M15 D1).
//!
//! ratatui's [`Terminal`](ratatui::Terminal) owns the double buffer and its
//! diff; it hands this backend only the changed `(x, y, &Cell)` cells via
//! [`Backend::draw`]. The backend serializes each run as a cursor-position
//! move (`CUP`), an `SGR` style change (gated by the connection's
//! [`ColorDepth`]), and the glyph. `size()` reports the PTY dimensions the
//! transport last observed; the alternate-screen / cursor control sequences
//! are exposed as plain byte helpers for the output pump to frame a session.
//!
//! There is no local terminal here, so cursor *queries*
//! ([`Backend::get_cursor_position`]) return the last position this backend
//! wrote rather than probing a device — the fullscreen `Terminal::draw`
//! path never queries, it only sets.

use std::io;

use mush_proto::{Color as MushColor, ColorDepth};
use ratatui::backend::{Backend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::{Color, Modifier};

/// Enter the alternate screen and hide the cursor. Emitted on `@tui`.
pub const ENTER_ALT_SCREEN: &[u8] = b"\x1b[?1049h\x1b[?25l";

/// Show the cursor and leave the alternate screen. Emitted on `@notui`, on a
/// `Disconnect`, and best-effort when a TUI connection tears down — so no
/// client is ever left stuck on the alternate screen (invariant T5).
pub const LEAVE_ALT_SCREEN: &[u8] = b"\x1b[?25h\x1b[?1049l\x1b[0m";

/// Upper bound on reported terminal columns. ratatui allocates a
/// `width * height` cell buffer (twice — front and back), so a client that
/// reports a window size in the tens of thousands (a pty/window-change request
/// carries up to `u16::MAX` each) would OOM-abort the shared server process on
/// `@tui` or the next resize paint. Clamp well beyond any real terminal so the
/// allocation stays bounded (invariant T4).
const MAX_WIDTH: u16 = 400;
/// Upper bound on reported terminal rows; see [`MAX_WIDTH`].
const MAX_HEIGHT: u16 = 200;

/// A ratatui backend that accumulates VT100 output bytes.
///
/// The transport drains [`take_output`](SshBackend::take_output) after each
/// `Terminal::draw` and writes the bytes to the SSH channel — synchronously
/// under the connection lock, so the async write happens only after the lock
/// is dropped (invariant T2).
#[derive(Debug)]
pub struct SshBackend {
    out: Vec<u8>,
    width: u16,
    height: u16,
    depth: ColorDepth,
    /// Last position written by `draw`/`set_cursor_position`, returned by the
    /// (never-hit, on the fullscreen path) cursor query.
    cursor: Position,
}

impl SshBackend {
    /// A backend sized for `width` x `height` at the given color `depth`.
    #[must_use]
    pub fn new(width: u16, height: u16, depth: ColorDepth) -> SshBackend {
        SshBackend {
            out: Vec::new(),
            width: width.clamp(1, MAX_WIDTH),
            height: height.clamp(1, MAX_HEIGHT),
            depth,
            cursor: Position::ORIGIN,
        }
    }

    /// Update the reported terminal geometry / color depth (on PTY resize or
    /// caps renegotiation). Takes effect on the next `Terminal::draw`, whose
    /// autoresize picks up the new [`size`](Backend::size).
    pub fn set_geometry(&mut self, width: u16, height: u16, depth: ColorDepth) {
        self.width = width.clamp(1, MAX_WIDTH);
        self.height = height.clamp(1, MAX_HEIGHT);
        self.depth = depth;
    }

    /// Take the accumulated output bytes, leaving the buffer empty.
    #[must_use]
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    /// Cursor-position (`CUP`) move to 0-based `(x, y)` — emitted 1-based.
    fn push_cup(&mut self, x: u16, y: u16) {
        self.out.extend_from_slice(
            format!("\x1b[{};{}H", y.saturating_add(1), x.saturating_add(1)).as_bytes(),
        );
    }
}

/// Map a ratatui [`Color`] to a `mush_proto` color so the shared
/// downgrade/`SGR` logic renders it. `Reset` and the terminal-default cases
/// carry no color.
fn to_mush_color(c: Color) -> Option<MushColor> {
    let idx = match c {
        Color::Reset => return None,
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
        // Palette index 0..=15 is the ANSI-16 set; everything above is a
        // 256-color index. Splitting here keeps 16-color-only terminals off
        // the `38;5;n` form they may not grok.
        Color::Indexed(n) if n < 16 => u16::from(n),
        Color::Indexed(n) => return Some(MushColor::Idx256(n)),
        Color::Rgb(r, g, b) => return Some(MushColor::Rgb(r, g, b)),
    };
    Some(MushColor::Ansi(idx as u8))
}

/// SGR parameter fragment for one already-downgraded color, `base` 30 (fg) /
/// 40 (bg). Indices 0..=15 use the basic/bright forms; 256 and RGB use the
/// extended forms. Mirrors `mush_ansi`'s color params so both renderers agree
/// on the wire.
fn color_params(c: MushColor, base: u8) -> String {
    match c {
        MushColor::Ansi(n) => {
            let n = n % 16;
            if n < 8 {
                format!("{}", base + n)
            } else {
                format!("{}", base + 52 + n) // 90..=97 fg, 100..=107 bg
            }
        }
        MushColor::Idx256(i) => format!("{};5;{i}", base + 8),
        MushColor::Rgb(r, g, b) => format!("{};2;{r};{g};{b}", base + 8),
    }
}

/// The complete `SGR` escape for one cell's style at `depth`, or the reset
/// `\x1b[0m` alone when the cell is unstyled. Empty when `depth` is
/// [`ColorDepth::None`] (colorless terminals get glyphs only, matching the
/// line renderer). A leading reset makes every run self-contained so a stale
/// attribute never bleeds from the previous cell.
fn cell_sgr(cell: &Cell, depth: ColorDepth) -> Vec<u8> {
    if matches!(depth, ColorDepth::None) {
        return Vec::new();
    }
    let mut parts: Vec<String> = Vec::new();
    let m = cell.modifier;
    if m.contains(Modifier::BOLD) {
        parts.push("1".to_string());
    }
    if m.contains(Modifier::DIM) {
        parts.push("2".to_string());
    }
    if m.contains(Modifier::ITALIC) {
        parts.push("3".to_string());
    }
    if m.contains(Modifier::UNDERLINED) {
        parts.push("4".to_string());
    }
    if m.contains(Modifier::REVERSED) {
        parts.push("7".to_string());
    }
    if let Some(c) = to_mush_color(cell.fg).and_then(|c| mush_ansi::downgrade(c, depth)) {
        parts.push(color_params(c, 30));
    }
    if let Some(c) = to_mush_color(cell.bg).and_then(|c| mush_ansi::downgrade(c, depth)) {
        parts.push(color_params(c, 40));
    }
    let mut seq = b"\x1b[0m".to_vec();
    if !parts.is_empty() {
        seq.extend_from_slice(format!("\x1b[{}m", parts.join(";")).as_bytes());
    }
    seq
}

impl Backend for SshBackend {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        // `None` after a non-contiguous jump forces a fresh CUP. `cur_sgr`
        // tracks the style in effect: a bare `\x1b[0m` is the reset baseline,
        // so an unstyled run emits no SGR at all, and a style change always
        // re-emits (each `cell_sgr` leads with a reset, so runs are
        // self-contained). `depth == None` short-circuits every SGR.
        let colored = !matches!(self.depth, ColorDepth::None);
        let reset: &[u8] = b"\x1b[0m";
        let mut expected: Option<(u16, u16)> = None;
        let mut cur_sgr: Vec<u8> = reset.to_vec();
        for (x, y, cell) in content {
            if expected != Some((x, y)) {
                self.push_cup(x, y);
            }
            if colored {
                let sgr = cell_sgr(cell, self.depth);
                if sgr != cur_sgr {
                    self.out.extend_from_slice(&sgr);
                    cur_sgr = sgr;
                }
            }
            let sym = cell.symbol();
            if sym.is_empty() {
                // Wide-glyph continuation cell: emit nothing and force the
                // next cell to re-anchor with a CUP.
                expected = None;
                continue;
            }
            self.out.extend_from_slice(sym.as_bytes());
            self.cursor = Position {
                x: x.saturating_add(1),
                y,
            };
            expected = Some((x.saturating_add(1), y));
        }
        // Leave the terminal reset only if a non-default style was left open.
        if colored && cur_sgr != reset {
            self.out.extend_from_slice(reset);
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.out.extend_from_slice(b"\x1b[?25l");
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.out.extend_from_slice(b"\x1b[?25h");
        Ok(())
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(self.cursor)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        let pos = position.into();
        self.push_cup(pos.x, pos.y);
        self.cursor = pos;
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        // Reset attributes, erase the whole screen, home the cursor.
        self.out.extend_from_slice(b"\x1b[0m\x1b[2J\x1b[H");
        self.cursor = Position::ORIGIN;
        Ok(())
    }

    fn size(&self) -> io::Result<Size> {
        Ok(Size {
            width: self.width,
            height: self.height,
        })
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: Size {
                width: self.width,
                height: self.height,
            },
            pixels: Size {
                width: 0,
                height: 0,
            },
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn cell(sym: &str) -> Cell {
        let mut c = Cell::EMPTY;
        c.set_symbol(sym);
        c
    }

    #[test]
    fn draw_emits_cup_sgr_and_glyph() {
        let mut be = SshBackend::new(80, 24, ColorDepth::Ansi16);
        let mut a = cell("A");
        a.set_style(Style::default().fg(Color::Cyan));
        let cells = [(2u16, 1u16, &a)];
        be.draw(cells.into_iter()).unwrap();
        let out = String::from_utf8(be.take_output()).unwrap();
        // CUP to row 2 col 3 (1-based), cyan fg (36), glyph, trailing reset.
        assert_eq!(out, "\x1b[2;3H\x1b[0m\x1b[36mA\x1b[0m");
    }

    #[test]
    fn contiguous_cells_skip_redundant_cup() {
        let mut be = SshBackend::new(80, 24, ColorDepth::Ansi16);
        let a = cell("h");
        let b = cell("i");
        let cells = [(0u16, 0u16, &a), (1u16, 0u16, &b)];
        be.draw(cells.into_iter()).unwrap();
        let out = String::from_utf8(be.take_output()).unwrap();
        // Unstyled run: one CUP, both glyphs contiguous, no SGR at all.
        assert_eq!(out, "\x1b[1;1Hhi");
    }

    #[test]
    fn non_contiguous_cells_reissue_cup() {
        let mut be = SshBackend::new(80, 24, ColorDepth::Ansi16);
        let a = cell("x");
        let b = cell("y");
        let cells = [(0u16, 0u16, &a), (5u16, 3u16, &b)];
        be.draw(cells.into_iter()).unwrap();
        let out = String::from_utf8(be.take_output()).unwrap();
        assert!(out.contains("\x1b[1;1Hx"));
        assert!(out.contains("\x1b[4;6Hy"));
    }

    #[test]
    fn color_depth_none_strips_sgr() {
        let mut be = SshBackend::new(80, 24, ColorDepth::None);
        let mut a = cell("Z");
        a.set_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
        let cells = [(0u16, 0u16, &a)];
        be.draw(cells.into_iter()).unwrap();
        let out = String::from_utf8(be.take_output()).unwrap();
        assert_eq!(out, "\x1b[1;1HZ");
    }

    #[test]
    fn size_reflects_geometry() {
        let mut be = SshBackend::new(100, 40, ColorDepth::TrueColor);
        assert_eq!(be.size().unwrap(), Size::new(100, 40));
        be.set_geometry(60, 20, ColorDepth::Ansi256);
        assert_eq!(be.size().unwrap(), Size::new(60, 20));
    }

    #[test]
    fn geometry_is_clamped_to_a_sane_maximum() {
        // A hostile / oversized window report must not drive an unbounded
        // ratatui buffer allocation: both constructor and resize cap the size.
        let mut be = SshBackend::new(u16::MAX, u16::MAX, ColorDepth::Ansi16);
        assert_eq!(be.size().unwrap(), Size::new(MAX_WIDTH, MAX_HEIGHT));
        be.set_geometry(50_000, 40_000, ColorDepth::Ansi16);
        assert_eq!(be.size().unwrap(), Size::new(MAX_WIDTH, MAX_HEIGHT));
        // The low end still floors at 1.
        be.set_geometry(0, 0, ColorDepth::Ansi16);
        assert_eq!(be.size().unwrap(), Size::new(1, 1));
    }

    #[test]
    fn clear_and_cursor_sequences() {
        let mut be = SshBackend::new(80, 24, ColorDepth::Ansi16);
        be.clear().unwrap();
        be.hide_cursor().unwrap();
        be.show_cursor().unwrap();
        be.set_cursor_position(Position { x: 4, y: 2 }).unwrap();
        let out = String::from_utf8(be.take_output()).unwrap();
        assert!(out.starts_with("\x1b[0m\x1b[2J\x1b[H"));
        assert!(out.contains("\x1b[?25l"));
        assert!(out.contains("\x1b[?25h"));
        assert!(out.ends_with("\x1b[3;5H"));
        assert_eq!(be.get_cursor_position().unwrap(), Position { x: 4, y: 2 });
    }

    #[test]
    fn bright_color_uses_9x_form() {
        let mut be = SshBackend::new(80, 24, ColorDepth::Ansi16);
        let mut a = cell("!");
        a.set_style(Style::default().fg(Color::LightCyan));
        let cells = [(0u16, 0u16, &a)];
        be.draw(cells.into_iter()).unwrap();
        let out = String::from_utf8(be.take_output()).unwrap();
        assert!(out.contains("\x1b[96m"), "got {out:?}");
    }
}
