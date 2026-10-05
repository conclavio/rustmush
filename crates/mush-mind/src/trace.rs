//! **The trace** (17b cut M2.1; doc 17 §7 "The engine", §14 "The REPL, the
//! proving ground and the bench"; D-UM-24's `on_print` and `on_debug`).
//!
//! What a think says with `print` and `debug`: at most [`TRACE_LINES`] lines,
//! each cut to [`TRACE_WIDTH`] characters, the lines past the eighth dropped.
//! [`crate::Sandbox::think`] answers it beside the think's result, on every
//! path — a fault's trace is the author's best clue — and the game shows it
//! to the mind's author (the REPL and the bench, doc 17 §14). It is **never
//! journalled** and never hashed: it decides nothing.
//!
//! **Script-authored text.** A trace line — and a [`crate::Fault::Type`]'s
//! text, which `line` makes the same way — is written by the mind's
//! author, so every control character in it (a newline, an escape that would
//! drive a terminal) is a space here, and the line stops at [`TRACE_WIDTH`]
//! characters while it is being written, so a value thousands of elements
//! long is never formatted whole. A renderer still escapes it as it escapes
//! any player text: it is the author's words, never the game's.
//!
//! A thread-local, for the reason the budget is one (the crate head, "One
//! think per thread"); [`begin`] clears it at every think's start.

use std::cell::RefCell;
use std::fmt::{self, Write as _};

/// The most lines one think's trace keeps; a line past it is dropped.
pub const TRACE_LINES: usize = 8;

/// The most characters a trace line keeps; the rest of the line is cut.
pub const TRACE_WIDTH: usize = 120;

thread_local! {
    static TRACE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// **Clear the trace** for a new think.
pub fn begin() {
    TRACE.with(|trace| trace.borrow_mut().clear());
}

/// **Keep one line**: `text` as `line` writes it, or nothing when
/// [`TRACE_LINES`] are already kept.
pub fn push(text: &str) {
    TRACE.with(|trace| {
        let mut lines = trace.borrow_mut();
        if lines.len() < TRACE_LINES {
            lines.push(line(&text));
        }
    });
}

/// **Take the think's trace**, leaving it empty.
#[must_use]
pub fn take() -> Vec<String> {
    TRACE.with(|trace| std::mem::take(&mut *trace.borrow_mut()))
}

/// **One line of script-authored text**: `text` rendered with every control
/// character a space, cut to [`TRACE_WIDTH`] characters — and the rendering
/// itself stopped there (the writer refuses the rest, which a `Display` hands
/// back at its next write), so the cost of a line is bounded whatever the
/// value behind it (17b cut M2.1, review finding F3).
pub(crate) fn line(text: &dyn fmt::Display) -> String {
    let mut out = Bounded::default();
    // The writer's refusal at the width is the cut, not a failure.
    let _cut = write!(out, "{text}");
    out.text
}

/// A writer that keeps [`TRACE_WIDTH`] characters and refuses the rest.
#[derive(Default)]
struct Bounded {
    text: String,
    chars: usize,
}

impl fmt::Write for Bounded {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            if self.chars == TRACE_WIDTH {
                return Err(fmt::Error);
            }
            self.text.push(if c.is_control() { ' ' } else { c });
            self.chars += 1;
        }
        Ok(())
    }
}
