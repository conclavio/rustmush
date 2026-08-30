//! Server-side ratatui TUI over SSH (milestone M15) — the third renderer of
//! the shared per-connection [`OutputEvent`](mush_proto::OutputEvent) stream
//! (line = `mush-ansi`, web = JSON, TUI = ratatui).
//!
//! This module lives entirely in the transport layer: it never touches the
//! single-writer world. `+tui` flips the connection into [`Mode::Tui`], the
//! output pump applies each event to a [`TuiApp`] and repaints one coalesced
//! frame through a clean-room [`SshBackend`], and raw input is decoded into
//! the app's input line / navigation. `+notui` (or the detach hotkey) flips
//! back to line mode, which is byte-for-byte unaffected when a connection
//! never enters the TUI (invariant T1).
//!
//! [`Mode::Tui`]: crate::server (private)

mod app;
mod backend;
mod render;

use app::{InputResult, TuiApp};
use backend::{ENTER_ALT_SCREEN, LEAVE_ALT_SCREEN, SshBackend};

use mush_proto::{ClientCaps, OutputEvent};
use ratatui::Terminal;

/// A connected TUI: the ratatui [`Terminal`] (owning the [`SshBackend`] and
/// the double-buffer diff) plus the [`TuiApp`] state. Held in the connection
/// `Shared` while [`Mode::Tui`](crate::server) is active; both the output
/// pump and the input handler reach it under the connection lock.
pub struct TuiSession {
    terminal: Terminal<SshBackend>,
    app: TuiApp,
}

impl TuiSession {
    /// Build a fresh TUI session for `caps` (does not paint).
    #[must_use]
    pub fn new(caps: ClientCaps) -> TuiSession {
        let backend = SshBackend::new(caps.width, caps.height, caps.color);
        // `Terminal::new` only fails if the backend's `size()` errors, which
        // `SshBackend` never does — but stay total rather than unwrap-panic.
        let terminal = Terminal::new(backend).unwrap_or_else(|_| {
            Terminal::new(SshBackend::new(80, 24, caps.color)).expect("infallible backend size")
        });
        TuiSession {
            terminal,
            app: TuiApp::new(caps),
        }
    }

    /// Apply one event to the app state (no paint).
    pub fn apply_event(&mut self, event: &OutputEvent) {
        self.app.apply_event(event);
    }

    /// Feed a burst of raw input bytes; see [`InputResult`].
    pub fn feed_input(&mut self, bytes: &[u8]) -> InputResult {
        self.app.feed_input(bytes)
    }

    /// Update capabilities on resize (geometry + color depth). The next
    /// [`paint`](Self::paint) picks up the new size via the terminal's
    /// autoresize (which clears + reflows).
    pub fn set_caps(&mut self, caps: ClientCaps) {
        self.app.set_caps(caps);
        self.terminal
            .backend_mut()
            .set_geometry(caps.width, caps.height, caps.color);
    }

    /// Render one frame and return the bytes to write to the channel. Pure
    /// and synchronous: the caller holds the connection lock across this and
    /// drops it before the async write (invariant T2).
    #[must_use]
    pub fn paint(&mut self) -> Vec<u8> {
        // Keep the backend geometry in step with the app caps every frame so
        // a resize that arrived between paints is honored here.
        let caps = self.app.caps();
        self.terminal
            .backend_mut()
            .set_geometry(caps.width, caps.height, caps.color);
        let TuiSession { terminal, app } = self;
        // The only error path is a backend write error; `SshBackend` writes
        // into a `Vec` and never errors, so a failure just yields no bytes.
        let _ = terminal.draw(|frame| app.draw(frame));
        terminal.backend_mut().take_output()
    }

    /// Bytes that enter the alternate screen and hide the cursor.
    #[must_use]
    pub fn enter_bytes() -> &'static [u8] {
        ENTER_ALT_SCREEN
    }

    /// Bytes that restore the cursor and leave the alternate screen.
    #[must_use]
    pub fn leave_bytes() -> &'static [u8] {
        LEAVE_ALT_SCREEN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_proto::{ClientKind, ColorDepth, NarrativeKind, ObjRef, RichText, WhoEntry};

    fn caps() -> ClientCaps {
        ClientCaps {
            kind: ClientKind::Ssh,
            color: ColorDepth::Ansi16,
            width: 80,
            height: 24,
            utf8: true,
            structured: false,
        }
    }

    #[test]
    fn paint_produces_frame_bytes_with_who_and_scrollback() {
        let mut sess = TuiSession::new(caps());
        sess.apply_event(&OutputEvent::Who(vec![WhoEntry {
            name: "Alice".into(),
            alias: None,
            idle_s: 0,
            conn_s: 0,
            location: None,
            doing: None,
        }]));
        sess.apply_event(&OutputEvent::Narrative {
            kind: NarrativeKind::Say,
            actor: Some(ObjRef::new("#1", "Nick")),
            text: RichText::plain("hello world"),
        });
        let bytes = sess.paint();
        let text = String::from_utf8_lossy(&bytes);
        // The buffer diff re-emits only changed cells, so a space that
        // matches the blank screen is skipped and words are separated by a
        // cursor move — assert on single-word tokens, not on the joined
        // phrase.
        assert!(text.contains("Alice"), "WHO rail missing: {text:?}");
        assert!(text.contains("hello"), "scrollback word 'hello' missing");
        assert!(text.contains("world"), "scrollback word 'world' missing");
        assert!(text.contains("WHO"), "rail title missing");
        assert!(text.contains("rustmush"), "main frame title missing");
    }

    #[test]
    fn resize_repaint_is_safe() {
        let mut sess = TuiSession::new(caps());
        sess.apply_event(&OutputEvent::Sys(RichText::plain("x")));
        let _ = sess.paint();
        let mut small = caps();
        small.width = 10;
        small.height = 4;
        sess.set_caps(small);
        // The first paint after a resize clears + reflows; must not panic.
        let bytes = sess.paint();
        assert!(!bytes.is_empty());
    }
}
