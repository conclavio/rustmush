//! Server-side ratatui TUI over SSH (milestone M15, interface plan I2) —
//! the third renderer of the shared per-connection
//! [`OutputEvent`] stream (line = `mush-ansi`, web
//! = JSON, TUI = ratatui).
//!
//! This module lives entirely in the transport layer: it never touches the
//! single-writer world. `@tui` flips the connection into `Mode::Tui`, the
//! output pump applies each event to a [`TuiApp`] and repaints one coalesced
//! frame through a clean-room `SshBackend`, and raw input is decoded into
//! keys the modal shell acts on. `@notui` (or the detach hotkey) flips back
//! to line mode, which is byte-for-byte unaffected when a connection never
//! enters the TUI (invariant T1).
//!
//! The shell is **modal** (D-I-17): numbered [`screen`]s of titled panes, a
//! nav mode where bare keys act and an input mode where the box is a normal
//! MUSH line; every nav hotkey is reachable from the box as Alt+key. A bare
//! key never sends a mutating command (D-I-18), and on the scene screen the
//! room's one-letter exits are keys (D-I-19).
//!

pub mod app;
mod backend;
mod cockpit_text;
pub mod helm;
pub mod keys;
mod mind_workshop;
pub mod panels;
pub mod pixel;
pub mod render;
pub mod replay;
pub mod screen;
pub mod sheet;

pub use app::{InputResult, Mode, StickyMode, TuiApp, TuiManifest};
use backend::{ENTER_ALT_SCREEN, LEAVE_ALT_SCREEN, SshBackend};

use mush_pixel::caps::Encoder;
use mush_proto::{ClientCaps, OutputEvent};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// A connected TUI: the ratatui [`Terminal`] (owning the `SshBackend` and
/// the double-buffer diff) plus the [`TuiApp`] state. Held in the connection
/// `Shared` while `Mode::Tui` is active; both the output
/// pump and the input handler reach it under the connection lock.
pub struct TuiSession {
    terminal: Terminal<SshBackend>,
    app: TuiApp,
}

impl TuiSession {
    /// Build a fresh TUI session for `caps` (does not paint), knowing what
    /// the server announced in its `Hello` and `SessionInfo`.
    #[must_use]
    pub fn new(caps: ClientCaps, manifest: TuiManifest) -> TuiSession {
        let backend = SshBackend::new(caps.width, caps.height, caps.color);
        // `Terminal::new` only fails if the backend's `size()` errors, which
        // `SshBackend` never does — but stay total rather than unwrap-panic.
        let terminal = Terminal::new(backend).unwrap_or_else(|_| {
            Terminal::new(SshBackend::new(80, 24, caps.color)).expect("infallible backend size")
        });
        let mut app = TuiApp::new(caps);
        app.set_manifest(manifest);
        TuiSession { terminal, app }
    }

    /// The queries the transport should submit right after entering the
    /// TUI (WHO, the command table, channels, vitals, the first page of
    /// scene history, the active screen's own feeds), through the read-only
    /// query seam. `&mut` because the history page arms its own latch.
    #[must_use]
    pub fn entry_queries(&mut self) -> Vec<(String, serde_json::Value)> {
        self.app.entry_queries()
    }

    /// Apply one event to the app state (no paint). Returns the read-only
    /// queries the app wants submitted as a result (a resync, a hydration).
    pub fn apply_event(&mut self, event: &OutputEvent) -> Vec<(String, serde_json::Value)> {
        self.app.apply_event(event)
    }

    /// Typed follow-up requests queued while applying an output event.
    pub fn take_invokes(&mut self) -> Vec<(u64, mush_proto::InvokeRequest)> {
        self.app.take_invokes()
    }

    /// The timer heal's queries (16b cut B.2, `B-59`): at most one
    /// `arena.map` ask, while a seated frame is held without its ground and the
    /// latch has aged — see [`TuiApp::heal_queries`]. The output pump calls
    /// this on its four-second tick; the clock is the app's own, so the timer
    /// and the frame path read one latch on one clock.
    #[must_use]
    pub fn heal_queries(&mut self) -> Vec<(String, serde_json::Value)> {
        let now = self.app.now();
        self.app.heal_queries(now)
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
        // Pixel cockpit spike (2026-10-21): an event or key repaint carries
        // NO picture. Pictures go out on the frame clock alone
        // ([`Self::pixel_frame`]): on dev2's first live turn the arena's
        // per-pulse events each repainted a moved camera too, and the wire
        // carried ~35 frames a second against a 20 Hz clock.
        let bytes = self.paint_text();
        // Doc 21 cut G.2: a repaint that wrote into a sixel picture's box (an
        // overlay, a resize's clear) erased those pixels; the next tick sends
        // the picture again.
        if self.terminal.backend_mut().take_touched() {
            self.app.pixel_text_touched();
        }
        bytes
    }

    /// The text frame alone: the ratatui diff, plus what a picture taken off
    /// screen since the last paint needs — a kitty image freed after it, a
    /// sixel one wiped by repainting the whole frame over it.
    fn paint_text(&mut self) -> Vec<u8> {
        // Keep the backend geometry in step with the app caps every frame so
        // a resize that arrived between paints is honored here.
        let caps = self.app.caps();
        self.terminal
            .backend_mut()
            .set_geometry(caps.width, caps.height, caps.color);
        let TuiSession { terminal, app } = self;
        let closed = app.pixel_take_close();
        // A sixel picture is pixels in the cell grid, not an image to free:
        // a diff would leave every cell the next screen draws as a blank
        // showing the old picture, so the whole frame is repainted.
        if closed == Some(Encoder::Sixel) || app.pixel_take_wipe() {
            let _ = terminal.clear();
        }
        terminal.backend_mut().set_watch(app.pixel_sixel_box());
        // The kitty keyboard protocol's push (the view opened) or pop (it
        // closed) goes ahead of the frame, so the next key already reads right.
        let mut bytes = app.pixel_take_terminal_bytes();
        // The only error path is a backend write error; `SshBackend` writes
        // into a `Vec` and never errors, so a failure just yields no bytes.
        let _ = terminal.draw(|frame| app.draw(frame));
        bytes.extend(terminal.backend_mut().take_output());
        if closed == Some(Encoder::Kitty) {
            bytes.extend_from_slice(&mush_pixel::kitty::delete_bytes(pixel::IMAGE_ID));
        }
        bytes
    }

    /// **One frame-clock tick's bytes** (pixel cockpit spike): the picture
    /// when it changed, with the text frame refreshed under it (the HUD's
    /// counters moved); NOTHING when it did not — not a cursor move, not a
    /// cell. The frame clock runs twenty times a second, and a tick that
    /// wrote even a few bytes would fill a stalled client's SSH window.
    #[must_use]
    pub fn pixel_frame(&mut self) -> Vec<u8> {
        let picture = self.app.pixel_paint();
        if picture.is_empty() {
            return Vec::new();
        }
        let mut bytes = self.paint_text();
        // The picture goes out after the text, so whatever this text paint
        // wrote into its box is drawn over.
        let _ = self.terminal.backend_mut().take_touched();
        bytes.extend(picture);
        bytes
    }

    /// The graphics capability probe (pixel cockpit spike; doc 21 cuts G.1,
    /// G.2): the kitty query, the cell-size request and DA1, sent once right
    /// after the first frame; starts the probe's clock.
    #[must_use]
    pub fn probe_bytes(&mut self) -> Vec<u8> {
        self.app.pixel_probe_sent();
        mush_pixel::caps::probe_bytes()
    }

    /// `@pixel/encoder <choice>` for this connection (doc 21 cut G.2).
    pub fn set_pixel_choice(&mut self, choice: mush_pixel::caps::EncoderChoice) {
        self.app.pixel_set_choice(choice);
    }

    /// What the probe found, in words, and the encoder `^G` would draw in.
    pub fn pixel_report(&mut self) -> (&'static str, Option<Encoder>) {
        self.app.pixel_report()
    }

    /// Whether the pixel view is open (the pump then runs its frame clock).
    #[must_use]
    pub fn pixel_active(&self) -> bool {
        self.app.pixel_active()
    }

    /// One frame-clock tick.
    pub fn pixel_tick(&mut self) {
        self.app.pixel_tick();
    }

    /// The frame clock's wait while the view is open (20 Hz, or uncapped).
    #[must_use]
    pub fn pixel_interval(&self) -> std::time::Duration {
        self.app.pixel_interval()
    }

    /// The bytes that free the picture's image when the view is open — to
    /// send before leaving the alternate screen.
    #[must_use]
    pub fn pixel_close_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        if self.app.pixel_active() && self.app.pixel_shown() == Some(Encoder::Kitty) {
            bytes.extend(mush_pixel::kitty::delete_bytes(pixel::IMAGE_ID));
        }
        // Doc 21 cut G.4: never leave the terminal in the keyboard protocol.
        if self.app.pixel_keyboard_pushed() {
            bytes.extend_from_slice(b"\x1b[<u");
        }
        bytes
    }

    /// The view's helm orders since the last call (doc 21 cut G.4): what the
    /// output pump sends to the world after a frame tick.
    pub fn pixel_take_orders(&mut self) -> Vec<String> {
        self.app.pixel_take_orders()
    }

    /// Leaving the TUI with the view open: the order that stops a turn in
    /// progress, to send before the TUI is dropped.
    pub fn pixel_leave_orders(&mut self) -> Vec<String> {
        self.app.pixel_leave_orders()
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

/// Paint `app` into a `w`×`h` test buffer and return it as text, one line
/// per row, trailing spaces trimmed — the golden-frame helper (plan §3.2).
#[must_use]
pub fn frame_text(app: &mut TuiApp, w: u16, h: u16) -> String {
    let mut caps = app.caps();
    caps.width = w;
    caps.height = h;
    app.set_caps(caps);
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|frame| app.draw(frame)).expect("draw");
    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..h {
        let mut row = String::new();
        for x in 0..w {
            row.push_str(buffer[(x, y)].symbol());
        }
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_proto::{
        ClientKind, ColorDepth, NarrativeKind, ObjRef, PanelFrameKind, RichText, WhoEntry,
    };

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
        let mut sess = TuiSession::new(caps(), TuiManifest::default());
        sess.apply_event(&OutputEvent::Who(vec![WhoEntry {
            name: "Alice".into(),
            alias: None,
            idle_s: 0,
            conn_s: 0,
            location: None,
            here: false,
            doing: None,
        }]));
        sess.apply_event(&OutputEvent::Narrative {
            kind: NarrativeKind::Say,
            actor: Some(ObjRef::new("#1", "Nick")),
            text: RichText::plain("hello world"),
        });
        let mut wide = caps();
        wide.width = 120;
        wide.height = 34;
        sess.set_caps(wide);
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
        assert!(text.contains("rustmush"), "tab strip missing");
    }

    #[test]
    fn resize_repaint_is_safe() {
        let mut sess = TuiSession::new(caps(), TuiManifest::default());
        sess.apply_event(&OutputEvent::Sys(RichText::plain("x")));
        let _ = sess.paint();
        let mut small = caps();
        small.width = 10;
        small.height = 4;
        sess.set_caps(small);
        // The first paint after a resize clears + reflows; must not panic. Ten
        // columns is below the floor (16b cut AB.1), so the frame is the floor
        // sentence, and its first row at that width is `The TUI`. The cell
        // diff separates words with cursor moves, so each word is a token of
        // its own: never assert the joined phrase.
        let bytes = sess.paint();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("TUI"), "{text:?}");
        // One column short of the floor: the sentence names that width.
        let mut narrow = caps();
        narrow.width = 119;
        narrow.height = 34;
        sess.set_caps(narrow);
        let bytes = sess.paint();
        let text = String::from_utf8_lossy(&bytes);
        for token in ["needs", "columns", "119"] {
            assert!(text.contains(token), "{token}: {text:?}");
        }
    }

    /// **16b cut B.3** (`B-66`): `@tui` opens on the room the connection was
    /// last shown — the status line names it and the scene screen's exit keys
    /// work — instead of `(no room yet)` until something else arrives. The pump
    /// records the room into the manifest (`server::remember_the_room`), and
    /// `TuiSession::new` hands it to the app through `set_manifest`.
    #[test]
    fn tui_opens_on_the_last_room_it_was_shown() {
        use mush_proto::{ExitRef, RoomView};
        let cellar = RoomView {
            name: "Cellar".into(),
            id: "#4".into(),
            desc: RichText::plain("Damp stone."),
            contents: vec![],
            exits: vec![ExitRef {
                name: "Up".into(),
                aliases: vec!["u".into()],
            }],
            zone: None,
            ambient: Default::default(),
        };
        let manifest = TuiManifest {
            room: Some(cellar.clone()),
            ..TuiManifest::default()
        };
        let mut sess = TuiSession::new(caps(), manifest);
        assert_eq!(sess.app.exit_keys(), &[('u', "Up".to_string())]);
        // At the floor (16b cut AB.1): below it row 1 is the floor sentence.
        let text = frame_text(&mut sess.app, 120, 34);
        let status = text.lines().nth(1).unwrap_or_default();
        assert!(status.starts_with("Cellar #4"), "{text}");
        assert!(!text.contains("no room yet"), "{text}");
        // A room the app already holds is newer than the manifest's.
        let mut app = TuiApp::new(caps());
        app.apply_event(&OutputEvent::Room(RoomView {
            name: "Attic".into(),
            id: "#5".into(),
            desc: RichText::plain("Dust."),
            contents: vec![],
            exits: vec![],
            zone: None,
            ambient: Default::default(),
        }));
        app.set_manifest(TuiManifest {
            room: Some(cellar),
            ..TuiManifest::default()
        });
        let text = frame_text(&mut app, 120, 34);
        assert!(
            text.lines().nth(1).unwrap_or_default().starts_with("Attic"),
            "{text}"
        );
    }

    #[test]
    fn frame_text_reads_the_painted_buffer() {
        let mut app = TuiApp::new(caps());
        app.apply_event(&OutputEvent::Sys(RichText::plain("hi there")));
        let text = frame_text(&mut app, 120, 34);
        assert_eq!(text.lines().count(), 34);
        assert!(text.contains("hi there"));
        assert!(text.contains("rustmush"));
    }

    /// Pixel cockpit spike (2026-10-21): the probe goes out after `@tui`, its
    /// answer comes back through the input path as marker keys, `^G` opens
    /// the view only once the answer is OK, the painted bytes carry the HUD
    /// and the kitty frame, an unchanged picture is not re-sent, a turn is,
    /// and closing frees the image.
    #[test]
    fn the_probe_reply_opens_the_pixel_view_and_paint_carries_a_kitty_frame() {
        let mut wide = caps();
        wide.width = 120;
        wide.height = 40;
        let mut sess = TuiSession::new(wide, TuiManifest::default());
        let probe = sess.probe_bytes();
        assert!(probe.starts_with(b"\x1b_Gi=31,"), "{probe:?}");
        assert!(probe.ends_with(b"\x1b[c"));
        // Before the answer, ^G does nothing.
        sess.feed_input(b"\x07");
        assert!(!sess.pixel_active());
        // The terminal answers OK then DA1, in one burst, as kitty does; the
        // markers ask for no repaint.
        let r = sess.feed_input(b"\x1b_Gi=31;OK\x1b\\\x1b[?62;22c");
        assert!(!r.redraw && r.submit.is_empty());
        // A seated cockpit frame and the arena map (the canonical fixtures).
        let cockpit: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
        ))
        .expect("cockpit fixture");
        let map: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../web/src/lib/fixtures/cockpit/arena_map.json"
        ))
        .expect("map fixture");
        sess.apply_event(&OutputEvent::Panel {
            feed: "arena.cockpit".into(),
            kind: PanelFrameKind::Keyframe,
            data: cockpit,
        });
        sess.apply_event(&OutputEvent::Panel {
            feed: "arena.map".into(),
            kind: PanelFrameKind::Keyframe,
            data: map,
        });
        let r = sess.feed_input(b"\x07");
        assert!(r.redraw && sess.pixel_active());
        // The key's repaint draws the HUD and no picture; the picture comes on
        // the first frame-clock tick, with the text frame refreshed under it.
        let hud = String::from_utf8_lossy(&sess.paint()).into_owned();
        assert!(hud.contains("PIXEL"), "the HUD: {hud:?}");
        assert!(
            !hud.contains("\x1b_Ga=T"),
            "no picture on a repaint: {hud:?}"
        );
        sess.pixel_tick();
        let bytes = sess.pixel_frame();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("\x1b[?2026h\x1b[2;1H\x1b_Ga=T,f=100,t=d,i=1,p=1,q=2,C=1,c=120,r=26,m="),
            "the 640x276 frame scaled into the top two-thirds, 120 by 26 cells: {text:?}"
        );
        assert!(text.ends_with("\x1b[?2026l\x1b8"));
        // The same picture is not sent twice — and an idle frame tick writes
        // NOTHING, not even a cursor move: twenty small writes a second would
        // fill a stalled client's SSH window (dev2's first live smoke).
        sess.pixel_tick();
        let again = sess.pixel_frame();
        assert!(again.is_empty(), "{:?}", String::from_utf8_lossy(&again));
        let repaint = String::from_utf8_lossy(&sess.paint()).into_owned();
        assert!(
            !repaint.contains("\x1b_Ga=T"),
            "an event repaint re-sends no picture"
        );
        // A turn, a tick with time on the clock, a new picture.
        sess.feed_input(b"\x1b[C");
        std::thread::sleep(std::time::Duration::from_millis(80));
        sess.pixel_tick();
        let turned = String::from_utf8_lossy(&sess.pixel_frame()).into_owned();
        assert!(turned.contains("\x1b_Ga=T"), "{turned:?}");
        // Leaving the TUI with the view open frees the image first.
        assert_eq!(
            sess.pixel_close_bytes(),
            b"\x1b_Ga=d,d=I,i=1,q=2;\x1b\\".to_vec()
        );
        // Esc closes it; the next paint carries the delete and no frame.
        sess.feed_input(b"\x1b");
        assert!(!sess.pixel_active());
        let closed = String::from_utf8_lossy(&sess.paint()).into_owned();
        assert!(
            closed.contains("\x1b_Ga=d,d=I,i=1,q=2;\x1b\\"),
            "{closed:?}"
        );
        assert!(!closed.contains("\x1b_Ga=T"));
        assert!(sess.pixel_close_bytes().is_empty());
        // A terminal that answers DA1 first never opens the view.
        let mut plain = TuiSession::new(caps(), TuiManifest::default());
        let _ = plain.probe_bytes();
        plain.feed_input(b"\x1b[?62;22c\x1b_Gi=31;OK\x1b\\");
        plain.feed_input(b"\x07");
        assert!(!plain.pixel_active());
    }

    /// **The text cockpit** (2026-10-22): chat is read and sent while
    /// flying. The comms show the scrollback's tail under the picture; Enter
    /// opens the chat line and stops a turn in progress; typed letters never
    /// steer; Enter sends the line as typed and flies again; Esc goes back to
    /// flying with the draft kept (the view stays open) and the next Enter
    /// brings it back; Esc while flying closes the view.
    #[test]
    fn the_text_cockpit_chats_while_flying() {
        let mut wide = caps();
        wide.width = 160;
        wide.height = 50;
        let mut sess = TuiSession::new(wide, TuiManifest::default());
        let _ = sess.probe_bytes();
        sess.feed_input(b"\x1b_Gi=31;OK\x1b\\\x1b[?62;22c");
        let cockpit: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
        ))
        .expect("cockpit fixture");
        sess.apply_event(&OutputEvent::Panel {
            feed: "arena.cockpit".into(),
            kind: PanelFrameKind::Keyframe,
            data: cockpit,
        });
        sess.apply_event(&OutputEvent::Narrative {
            kind: NarrativeKind::Say,
            actor: Some(ObjRef::new("#2", "Vasquez")),
            text: RichText::plain("contact bearing ninety"),
        });
        sess.feed_input(b"\x07");
        assert!(sess.pixel_active());
        let screen = String::from_utf8_lossy(&sess.paint()).into_owned();
        for word in [
            "ARMOUR", "WEAPONS", "CONTACTS", "COMMS", "Vasquez", "ninety", "READY",
        ] {
            assert!(screen.contains(word), "{word} under the picture");
        }
        sess.pixel_tick();
        assert!(
            !sess.pixel_frame().is_empty(),
            "the picture, which the helm reads"
        );

        // Flying: `a` turns the tank.
        let r = sess.feed_input(b"a");
        assert!(r.submit.iter().all(|l| l.starts_with("HEADING ")) && r.submit.len() == 1);
        // Enter opens the chat line and stops the turn.
        let r = sess.feed_input(b"\r");
        assert_eq!(r.submit.len(), 1, "the stop order: {:?}", r.submit);
        assert!(r.submit[0].starts_with("HEADING "));
        let screen = String::from_utf8_lossy(&sess.paint()).into_owned();
        assert!(screen.contains("send"), "the chat legend");
        // Typing never steers.
        let r = sess.feed_input(b"on my way ad");
        assert!(r.submit.is_empty(), "{:?}", r.submit);
        assert!(sess.pixel_active());
        // Enter sends the line as typed and flies again.
        let r = sess.feed_input(b"\r");
        assert_eq!(r.submit, ["on my way ad"]);
        let r = sess.feed_input(b"d");
        assert!(r.submit.iter().all(|l| l.starts_with("HEADING ")) && r.submit.len() == 1);
        sess.feed_input(b" ");

        // Esc goes back to flying with the draft kept; the view stays open.
        sess.feed_input(b"\rhalf a tho");
        sess.feed_input(b"\x1b");
        assert!(
            sess.pixel_active(),
            "Esc in the chat line does not close the view"
        );
        let screen = String::from_utf8_lossy(&sess.paint()).into_owned();
        assert!(screen.contains("(draft:"), "{screen:?}");
        let r = sess.feed_input(b"xyz");
        assert!(r.submit.is_empty(), "flying: those letters were keys");
        let r = sess.feed_input(b"\r");
        assert!(
            r.submit.iter().all(|l| l.starts_with("HEADING ")),
            "the draft is back on the line, nothing sent: {:?}",
            r.submit
        );
        let r = sess.feed_input(b"!\r");
        assert_eq!(r.submit, ["half a tho!"]);
        // Esc while flying closes the view.
        sess.feed_input(b"\x1b");
        assert!(!sess.pixel_active());
    }

    /// Doc 21 cut G.2, end to end through the session: a terminal that
    /// answers as Windows Terminal does (a cell size, then DA1 with sixel and
    /// no kitty answer) gets a sixel picture in its cell box; a resize clears
    /// the screen, so the next tick sends the picture again; closing wipes it
    /// with a full repaint and frees no kitty image.
    #[test]
    fn a_sixel_terminal_gets_a_sixel_picture_resent_after_a_clear_and_wiped_on_close() {
        let mut wide = caps();
        wide.width = 120;
        wide.height = 40;
        let mut sess = TuiSession::new(wide, TuiManifest::default());
        let probe = sess.probe_bytes();
        assert!(
            probe.ends_with(b"\x1b[16t\x1b[?u\x1b[c"),
            "the cell size is asked before DA1: {probe:?}"
        );
        let r = sess.feed_input(b"\x1b[6;20;10t\x1b[?61;4;6;22c");
        assert!(!r.redraw && r.submit.is_empty(), "markers, not keys");
        assert_eq!(
            sess.pixel_report(),
            ("sixel graphics OK", Some(Encoder::Sixel))
        );
        let cockpit: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
        ))
        .expect("cockpit fixture");
        let map: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../web/src/lib/fixtures/cockpit/arena_map.json"
        ))
        .expect("map fixture");
        for (feed, data) in [("arena.cockpit", cockpit), ("arena.map", map)] {
            sess.apply_event(&OutputEvent::Panel {
                feed: feed.into(),
                kind: PanelFrameKind::Keyframe,
                data,
            });
        }
        sess.feed_input(b"\x07");
        assert!(sess.pixel_active());
        let _ = sess.paint();
        sess.pixel_tick();
        let first = String::from_utf8_lossy(&sess.pixel_frame()).into_owned();
        assert!(
            first.contains("\x1b[2;1H\x1bP0;1;0q\"1;1;1200;517"),
            "the 640x276 frame shown at 1200x517, filling the top two-thirds"
        );
        assert!(!first.contains("\x1b_G"), "no kitty bytes");
        sess.pixel_tick();
        assert!(sess.pixel_frame().is_empty(), "unchanged: nothing");
        // A resize: the text repaint clears the screen, the picture goes again.
        let mut wider = wide;
        wider.width = 160;
        sess.set_caps(wider);
        let repaint = String::from_utf8_lossy(&sess.paint()).into_owned();
        assert!(repaint.contains("\x1b[2J"), "the resize cleared the screen");
        sess.pixel_tick();
        let again = String::from_utf8_lossy(&sess.pixel_frame()).into_owned();
        assert!(again.contains("\x1bP0;1;0q"), "resent after the clear");
        // Leaving with a sixel picture frees nothing; closing wipes it.
        assert!(sess.pixel_close_bytes().is_empty());
        sess.feed_input(b"\x1b");
        assert!(!sess.pixel_active());
        let closed = String::from_utf8_lossy(&sess.paint()).into_owned();
        assert!(
            closed.contains("\x1b[2J"),
            "a full repaint over the picture"
        );
        assert!(!closed.contains("\x1b_G"), "no kitty delete");
    }
}
