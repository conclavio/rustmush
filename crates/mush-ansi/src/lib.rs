//! # mush-ansi — `OutputEvent` -> ANSI renderer
//!
//! The SSH-side renderer for the shared semantic protocol (design 02 §5.7):
//! [`render`] turns each [`mush_proto::OutputEvent`] into classic MUSH text
//! lines, resolving semantic span tags through an [`AnsiTheme`], merging
//! explicit span styles over theme styles (explicit fields win), and
//! downgrading colors to the connection's
//! [`ColorDepth`](mush_proto::ColorDepth):
//! `Rgb -> nearest xterm-256 -> nearest ANSI-16 -> plain`, with everything
//! (bold included) stripped at `ColorDepth::None`.
//!
//! Invariants:
//!
//! - **CRLF only.** SSH raw mode gets `\r\n` line ends, never a bare `\n`.
//!   Only [`OutputEvent::Prompt`](mush_proto::OutputEvent::Prompt) emits
//!   without a terminator (the cursor stays after the prompt).
//! - **ANSI-aware wrapping.** Lines word-wrap at `caps.width`; escape
//!   sequences are zero-width and never split across lines; a style active
//!   at a break is reset and reopened, so every physical line is
//!   self-contained. Words longer than the width are not split.
//! - **No injected control bytes.** Control characters inside span text are
//!   stripped (embedded `\n` becomes a line break) — spans can never
//!   smuggle raw escapes to another player's terminal.
//!
//! [`ansi_len`], [`wrap`] and [`wrap_indent`] are public because
//! `mush-net-ssh` reuses them for line-editor redraw math and the TUI's
//! scrollback gutter.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

mod color;
pub mod panel;
mod render;
mod theme;
mod wrap;

pub use color::{ANSI16_RGB, PALETTE_256, downgrade, nearest_16, nearest_256, sgr};
pub use panel::{
    GRID_MAX_COLUMNS, GRID_MAX_ROWS, expand_rows, grid_lines, is_grid, viewport, window_rows,
};
pub use render::{fmt_duration, render};
pub use theme::{AnsiTheme, merge_styles};
pub use wrap::{MIN_HANG_BODY, RESET, ansi_len, wrap, wrap_indent};
