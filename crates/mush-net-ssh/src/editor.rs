//! Server-side raw-mode line editor (design 02 §5.4).
//!
//! When an SSH client allocates a PTY its terminal goes raw and *all* line
//! discipline is the server's job: echo, backspace, cursor movement,
//! history, masked password entry. [`LineEditor`] is that discipline as a
//! pure state machine — [`feed`](LineEditor::feed) takes raw channel bytes
//! and appends the echo/redraw bytes for the client to `out`; it performs
//! no I/O of its own, which keeps it unit-testable and fuzz-friendly. The
//! buffer, cursor and history themselves live in the shared
//! [`LineBuffer`] (interface plan I2, D-I-6) — the TUI edits with the same
//! core, so the two never drift.
//!
//! Supported editing: Backspace/DEL, ←/→ (CSI and SS3 forms), Home/End
//! (`^A`/`^E`, CSI `H`/`F`, CSI `1~`/`4~`/`7~`/`8~`), Delete (CSI `3~`),
//! `^U` kill-to-start, `^W` kill-word, ↑/↓ history (cap
//! [`HISTORY_CAP`]), `^C` clear line, `^D` on an empty line =
//! [`EditorAction::EofQuit`] (quit confirmation is the driver's job), `^D`
//! mid-line = delete-at-cursor. CR and CRLF (and bare LF) submit. Tab is
//! reserved. Lines cap at [`MAX_LINE_CHARS`] characters (bell beyond).
//! Incoming bytes are decoded as UTF-8 incrementally, tolerating multibyte
//! sequences split across `feed` calls; invalid sequences ring the bell
//! and are dropped. Decoded control characters (including C1) are never
//! inserted into the buffer.
//!
//! Column math assumes one terminal column per `char`; double-width CJK
//! glyphs can misalign the cursor during mid-line edits (accepted for M1 —
//! a full redraw on submit or `^C` always resolves it).

use crate::line::LineBuffer;
pub use crate::line::{HISTORY_CAP, MAX_LINE_CHARS};

const BELL: u8 = 0x07;
const CRLF: &[u8] = b"\r\n";
/// CR + erase-to-end-of-line: start of every full-line redraw.
const ERASE_LINE: &[u8] = b"\r\x1b[K";

/// Maximum echo/redraw bytes appended to `out` by a single [`LineEditor::feed`]
/// call (CRIT-1). One full redraw of a max-length 4-byte UTF-8 line is ~32 KiB
/// plus prompt; 64 KiB accommodates normal pastes and full-line redraws while
/// preventing single-packet amplification OOMs.
pub const MAX_FEED_OUT_BYTES: usize = 64 * 1024;

/// Maximum completed actions (`Submit` / `EofQuit`) emitted by a single
/// [`LineEditor::feed`] call (`P2-CLIENT-3`). Bounds empty-line `\r` and `^D`
/// floods in a single 32 KiB SSH packet while remaining transparent to normal
/// chunked input streams.
const MAX_FEED_ACTIONS: usize = 1024;

/// Maximum cumulative UTF-8 bytes of unedited history entries materialized into
/// the edit buffer within a single [`LineEditor::feed`] call (`P2-CLIENT-3`).
/// Bounds worst-case `Up` + edit + `Down` thrashing on 16 KiB history entries
/// without affecting normal history recall or small chunked streams.
const MAX_FEED_HISTORY_MATERIALIZE_BYTES: usize = 128 * 1024;

fn decimal_digits(mut n: usize) -> usize {
    let mut digits = 1;
    while n >= 10 {
        n /= 10;
        digits += 1;
    }
    digits
}

/// Output writer for one [`LineEditor::feed`] call: coalesces full-line redraws
/// via `dirty_redraw` and enforces a strict per-call output-growth ceiling
/// without ever splitting a UTF-8 codepoint or ANSI escape sequence.
struct FeedBuf<'a> {
    out: &'a mut Vec<u8>,
    limit: usize,
    dirty_redraw: bool,
    action_bytes: usize,
    action_limit: usize,
    history_materialized_bytes: usize,
    record_history: bool,
}

impl FeedBuf<'_> {
    fn push_byte(&mut self, b: u8) {
        if self.out.len() < self.limit {
            self.out.push(b);
        }
    }

    fn push_slice(&mut self, bytes: &[u8]) {
        if self.out.len().saturating_add(bytes.len()) <= self.limit {
            self.out.extend_from_slice(bytes);
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty_redraw = true;
    }

    fn flush_redraw(&mut self, editor: &LineEditor) {
        if !self.dirty_redraw {
            return;
        }
        self.dirty_redraw = false;
        let needed = editor.redraw_len();
        if self.out.len().saturating_add(needed) <= self.limit {
            editor.redraw(self.out);
        }
    }

    fn allow_history_materialize(&mut self, line: &LineBuffer) -> bool {
        if !line.is_history_unedited() {
            return true;
        }
        let next = self
            .history_materialized_bytes
            .saturating_add(line.utf8_bytes());
        if next > MAX_FEED_HISTORY_MATERIALIZE_BYTES {
            return false;
        }
        self.history_materialized_bytes = next;
        true
    }
}

/// How typed characters are echoed back to the client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EchoMode {
    /// Echo characters as typed.
    #[default]
    Normal,
    /// Echo `*` per character (passwords).
    Masked,
    /// Echo nothing at all.
    NoEcho,
}

/// What [`LineEditor::feed`] produced besides echo bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorAction {
    /// A completed input line (CR/CRLF/LF).
    Submit(String),
    /// `^D` on an empty line: the player asked to disconnect; the driver
    /// runs the quit confirmation.
    EofQuit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum EscState {
    None,
    /// Saw ESC, waiting for `[` (CSI) or `O` (SS3).
    Esc,
    /// Inside a CSI sequence, accumulating parameter/intermediate bytes.
    Csi(String),
    /// Saw ESC `O`, waiting for the single final byte.
    Ss3,
}

/// The server-side line editor. Pure: no I/O, no clocks, no allocation
/// beyond its own buffers.
#[derive(Debug)]
pub struct LineEditor {
    line: LineBuffer,
    prompt: String,
    mode: EchoMode,
    /// Partial UTF-8 sequence carried across `feed` calls.
    pending: Vec<u8>,
    /// Expected total length of the pending UTF-8 sequence.
    pending_len: usize,
    esc: EscState,
    /// Set by `\r` so an immediately following `\n` (CRLF) is swallowed.
    last_was_cr: bool,
}

impl Default for LineEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl LineEditor {
    /// Fresh editor: empty buffer, empty prompt, [`EchoMode::Normal`].
    pub fn new() -> Self {
        Self {
            line: LineBuffer::new(HISTORY_CAP),
            prompt: String::new(),
            mode: EchoMode::Normal,
            pending: Vec::new(),
            pending_len: 0,
            esc: EscState::None,
            last_was_cr: false,
        }
    }

    /// Replace the prompt shown by [`redraw`](Self::redraw). The string may
    /// contain SGR escapes (it is written verbatim); it must not contain
    /// cursor-moving sequences.
    pub fn set_prompt(&mut self, prompt: impl Into<String>) {
        self.prompt = prompt.into();
    }

    /// The current prompt.
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// Switch echo mode. Entering or leaving `Normal` cancels any history
    /// navigation (masked/no-echo lines never touch history).
    pub fn set_mode(&mut self, mode: EchoMode) {
        if mode != self.mode {
            self.line.cancel_history();
        }
        self.mode = mode;
    }

    /// The current echo mode.
    pub fn mode(&self) -> EchoMode {
        self.mode
    }

    /// Whether the edit buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.line.is_empty()
    }

    /// The current buffer content (for tests/diagnostics).
    pub fn line(&self) -> String {
        self.line.text()
    }

    /// The cursor as a character index (for tests/diagnostics).
    pub fn cursor(&self) -> usize {
        self.line.cursor()
    }

    /// Drop the most recently submitted line from history. Used by the
    /// welcome screen after a `connect`/`create` line so plaintext
    /// passwords are not retained (or recallable with ↑) for the life of
    /// the connection.
    pub fn forget_last_history(&mut self) {
        self.line.forget_last_history();
    }

    /// Wipe all recorded history entries. Used when leaving pre-auth
    /// screens so no pre-login line can ever leak into in-game ↑ recall.
    pub fn clear_history(&mut self) {
        self.line.clear_history();
    }

    /// Whether anything of this editor is visible on the client's current
    /// terminal line — used by the output pump to decide whether async
    /// output must erase and re-print the line around itself.
    pub fn has_visible_line(&self) -> bool {
        !self.prompt.is_empty() || (self.mode != EchoMode::NoEcho && !self.line.is_empty())
    }

    /// Re-print the prompt and buffer from column 0 (CR, erase line,
    /// prompt, visible buffer, cursor repositioned). Used after async
    /// output interrupted the line and internally after mid-line edits.
    pub fn redraw(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(ERASE_LINE);
        out.extend_from_slice(self.prompt.as_bytes());
        match self.mode {
            EchoMode::Normal => {
                let mut buf = [0u8; 4];
                for c in self.line.chars() {
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
            }
            EchoMode::Masked => out.extend(std::iter::repeat_n(b'*', self.line.len())),
            EchoMode::NoEcho => {}
        }
        if self.mode != EchoMode::NoEcho {
            let back = self.line.len() - self.line.cursor();
            if back > 0 {
                out.extend_from_slice(format!("\x1b[{back}D").as_bytes());
            }
        }
    }

    fn redraw_len(&self) -> usize {
        let mut len = ERASE_LINE.len() + self.prompt.len();
        match self.mode {
            EchoMode::Normal => len += self.line.utf8_bytes(),
            EchoMode::Masked => len += self.line.len(),
            EchoMode::NoEcho => {}
        }
        if self.mode != EchoMode::NoEcho {
            let back = self.line.len() - self.line.cursor();
            if back > 0 {
                len += 3 + decimal_digits(back);
            }
        }
        len
    }

    /// Feed raw channel bytes. Echo/redraw bytes for the client are
    /// appended to `out` (with redraws coalesced across the call and growth
    /// capped at [`MAX_FEED_OUT_BYTES`]); completed lines and EOF-quit
    /// requests are returned in order.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<u8>) -> Vec<EditorAction> {
        self.feed_with_history(bytes, out, true)
    }

    /// Like [`feed`](Self::feed), with explicit control over whether
    /// submitted lines enter recall history. Pre-auth screens pass `false`
    /// so pipelined multi-line bursts carrying credentials never persist
    /// plaintext passwords in recall memory.
    pub fn feed_with_history(
        &mut self,
        bytes: &[u8],
        out: &mut Vec<u8>,
        record_history: bool,
    ) -> Vec<EditorAction> {
        let mut actions = Vec::new();
        let mut buf = FeedBuf {
            limit: out.len().saturating_add(MAX_FEED_OUT_BYTES),
            out,
            dirty_redraw: false,
            action_bytes: 0,
            action_limit: bytes.len().saturating_mul(4).max(MAX_FEED_OUT_BYTES),
            history_materialized_bytes: 0,
            record_history,
        };
        for &b in bytes {
            self.feed_byte(b, &mut buf, &mut actions);
        }
        buf.flush_redraw(self);
        actions
    }

    fn feed_byte(&mut self, b: u8, buf: &mut FeedBuf<'_>, actions: &mut Vec<EditorAction>) {
        // Escape sequences are pure ASCII; they pre-empt UTF-8 assembly.
        if self.esc != EscState::None {
            self.feed_esc(b, buf);
            return;
        }
        if !self.pending.is_empty() {
            if (0x80..0xC0).contains(&b) {
                self.pending.push(b);
                if self.pending.len() == self.pending_len {
                    let seq = std::mem::take(&mut self.pending);
                    match std::str::from_utf8(&seq) {
                        Ok(s) => {
                            if let Some(c) = s.chars().next() {
                                self.insert_char(c, buf);
                            }
                        }
                        Err(_) => buf.push_byte(BELL),
                    }
                }
                return;
            }
            // Broken sequence: drop it, then process this byte normally.
            self.pending.clear();
            buf.push_byte(BELL);
        }
        let was_cr = self.last_was_cr;
        self.last_was_cr = false;
        match b {
            0x1b => self.esc = EscState::Esc,
            b'\r' => {
                self.last_was_cr = true;
                self.submit(buf, actions);
            }
            b'\n' => {
                if !was_cr {
                    self.submit(buf, actions);
                }
            }
            0x08 | 0x7f => self.backspace(buf),
            0x01 => self.home(buf),
            0x05 => self.end(buf),
            0x15 => self.kill_to_start(buf),
            0x17 => self.kill_word(buf),
            0x03 => self.ctrl_c(buf),
            0x04 => {
                if self.line.is_empty() {
                    if actions.len() < MAX_FEED_ACTIONS {
                        actions.push(EditorAction::EofQuit);
                    } else {
                        buf.push_byte(BELL);
                    }
                } else {
                    self.delete_at_cursor(buf);
                }
            }
            0x09 => {} // Tab reserved for future completion.
            0x00..=0x1f => {}
            0x20..=0x7e => self.insert_char(b as char, buf),
            0xc0..=0xdf => self.start_utf8(b, 2),
            0xe0..=0xef => self.start_utf8(b, 3),
            0xf0..=0xf7 => self.start_utf8(b, 4),
            _ => buf.push_byte(BELL), // stray continuation / invalid lead byte
        }
    }

    fn start_utf8(&mut self, b: u8, len: usize) {
        self.pending.clear();
        self.pending.push(b);
        self.pending_len = len;
    }

    fn feed_esc(&mut self, b: u8, buf: &mut FeedBuf<'_>) {
        match std::mem::replace(&mut self.esc, EscState::None) {
            EscState::None => unreachable!("feed_esc called without escape state"),
            EscState::Esc => match b {
                b'[' => self.esc = EscState::Csi(String::new()),
                b'O' => self.esc = EscState::Ss3,
                // Alt-<key> and unknown ESC sequences: swallow the byte.
                _ => {}
            },
            EscState::Csi(mut params) => match b {
                0x20..=0x3f => {
                    // Parameter and intermediate bytes (bounded: hostile
                    // streams cannot grow this without a final byte forever;
                    // cap defensively).
                    if params.len() < 32 {
                        params.push(b as char);
                    }
                    self.esc = EscState::Csi(params);
                }
                0x40..=0x7e => self.dispatch_key(&params, b, buf),
                _ => {} // C0 inside CSI: abort the sequence.
            },
            EscState::Ss3 => self.dispatch_key("", b, buf),
        }
    }

    fn dispatch_key(&mut self, params: &str, final_byte: u8, buf: &mut FeedBuf<'_>) {
        match final_byte {
            b'A' => self.history_up(buf),
            b'B' => self.history_down(buf),
            b'C' => self.right(buf),
            b'D' => self.left(buf),
            b'H' => self.home(buf),
            b'F' => self.end(buf),
            b'~' => match params.split(';').next().unwrap_or("") {
                "1" | "7" => self.home(buf),
                "4" | "8" => self.end(buf),
                "3" => self.delete_at_cursor(buf),
                _ => {}
            },
            _ => {}
        }
    }

    // ---- buffer edits ---------------------------------------------------

    fn insert_char(&mut self, c: char, buf: &mut FeedBuf<'_>) {
        if !buf.allow_history_materialize(&self.line) {
            buf.push_byte(BELL);
            return;
        }
        let at_end = self.line.at_end();
        // A control character (incl. C1), BiDi/line-separator control, or a
        // full line is refused with a bell.
        if !self.line.insert(c) {
            buf.push_byte(BELL);
            return;
        }
        if at_end && !buf.dirty_redraw {
            match self.mode {
                EchoMode::Normal => {
                    let mut b = [0u8; 4];
                    buf.push_slice(c.encode_utf8(&mut b).as_bytes());
                }
                EchoMode::Masked => buf.push_byte(b'*'),
                EchoMode::NoEcho => {}
            }
        } else if self.mode != EchoMode::NoEcho {
            buf.mark_dirty();
        }
    }

    fn backspace(&mut self, buf: &mut FeedBuf<'_>) {
        if !buf.allow_history_materialize(&self.line) {
            buf.push_byte(BELL);
            return;
        }
        let at_end = self.line.at_end();
        let Some(removed) = self.line.backspace() else {
            buf.push_byte(BELL);
            return;
        };
        match self.mode {
            EchoMode::NoEcho => {}
            EchoMode::Masked => {
                if at_end && !buf.dirty_redraw {
                    buf.push_slice(b"\x08 \x08");
                } else {
                    buf.mark_dirty();
                }
            }
            EchoMode::Normal => {
                if at_end && removed.is_ascii() && !buf.dirty_redraw {
                    buf.push_slice(b"\x08 \x08");
                } else {
                    buf.mark_dirty();
                }
            }
        }
    }

    fn delete_at_cursor(&mut self, buf: &mut FeedBuf<'_>) {
        if !buf.allow_history_materialize(&self.line) {
            buf.push_byte(BELL);
            return;
        }
        if self.line.delete().is_some() {
            if self.mode != EchoMode::NoEcho {
                buf.mark_dirty();
            }
        } else {
            buf.push_byte(BELL);
        }
    }

    fn kill_to_start(&mut self, buf: &mut FeedBuf<'_>) {
        if !buf.allow_history_materialize(&self.line) {
            buf.push_byte(BELL);
            return;
        }
        if self.line.kill_to_start() && self.mode != EchoMode::NoEcho {
            buf.mark_dirty();
        }
    }

    fn kill_word(&mut self, buf: &mut FeedBuf<'_>) {
        if !buf.allow_history_materialize(&self.line) {
            buf.push_byte(BELL);
            return;
        }
        if self.line.kill_word() && self.mode != EchoMode::NoEcho {
            buf.mark_dirty();
        }
    }

    fn ctrl_c(&mut self, buf: &mut FeedBuf<'_>) {
        buf.flush_redraw(self);
        self.line.clear();
        buf.push_slice(b"^C");
        buf.push_slice(CRLF);
        buf.mark_dirty();
    }

    // ---- cursor movement ------------------------------------------------

    fn move_echo(&self, n: usize, dir: u8, buf: &mut FeedBuf<'_>) {
        if n > 0 && self.mode != EchoMode::NoEcho && !buf.dirty_redraw {
            buf.push_slice(format!("\x1b[{n}{}", dir as char).as_bytes());
        }
    }

    fn home(&mut self, buf: &mut FeedBuf<'_>) {
        let moved = self.line.home();
        self.move_echo(moved, b'D', buf);
    }

    fn end(&mut self, buf: &mut FeedBuf<'_>) {
        let moved = self.line.end();
        self.move_echo(moved, b'C', buf);
    }

    fn left(&mut self, buf: &mut FeedBuf<'_>) {
        if self.line.left() {
            self.move_echo(1, b'D', buf);
        }
    }

    fn right(&mut self, buf: &mut FeedBuf<'_>) {
        if self.line.right() {
            self.move_echo(1, b'C', buf);
        }
    }

    // ---- history --------------------------------------------------------

    fn history_up(&mut self, buf: &mut FeedBuf<'_>) {
        if self.mode != EchoMode::Normal {
            return;
        }
        if self.line.history_up() {
            buf.mark_dirty();
        } else {
            buf.push_byte(BELL);
        }
    }

    fn history_down(&mut self, buf: &mut FeedBuf<'_>) {
        if self.mode != EchoMode::Normal {
            return;
        }
        if self.line.history_down() {
            buf.mark_dirty();
        } else {
            buf.push_byte(BELL);
        }
    }

    // ---- submit ---------------------------------------------------------

    fn submit(&mut self, buf: &mut FeedBuf<'_>, actions: &mut Vec<EditorAction>) {
        buf.flush_redraw(self);
        let line_bytes = self.line.utf8_bytes();
        let record = buf.record_history && self.mode == EchoMode::Normal;
        buf.push_slice(CRLF);
        if actions.len() < MAX_FEED_ACTIONS
            && buf.action_bytes.saturating_add(line_bytes) <= buf.action_limit
        {
            let line = self.line.submit(record);
            buf.action_bytes = buf.action_bytes.saturating_add(line.len());
            actions.push(EditorAction::Submit(line));
        } else {
            self.line.clear();
            buf.push_byte(BELL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed bytes, returning (actions, echo-bytes).
    fn feed(ed: &mut LineEditor, bytes: &[u8]) -> (Vec<EditorAction>, Vec<u8>) {
        let mut out = Vec::new();
        let actions = ed.feed(bytes, &mut out);
        (actions, out)
    }

    fn submit_one(ed: &mut LineEditor, bytes: &[u8]) -> String {
        let (actions, _) = feed(ed, bytes);
        assert_eq!(actions.len(), 1, "expected one action from {bytes:?}");
        match &actions[0] {
            EditorAction::Submit(s) => s.clone(),
            other => panic!("expected Submit, got {other:?}"),
        }
    }

    #[test]
    fn plain_line_cr_submits_and_echoes() {
        let mut ed = LineEditor::new();
        let (actions, out) = feed(&mut ed, b"say hi\r");
        assert_eq!(actions, vec![EditorAction::Submit("say hi".into())]);
        assert_eq!(out, b"say hi\r\n");
        assert!(ed.is_empty());
    }

    #[test]
    fn crlf_submits_once_lf_alone_submits() {
        let mut ed = LineEditor::new();
        let (actions, _) = feed(&mut ed, b"one\r\ntwo\n");
        assert_eq!(
            actions,
            vec![
                EditorAction::Submit("one".into()),
                EditorAction::Submit("two".into()),
            ]
        );
        // A CR and LF split across feeds still counts once.
        let (a1, _) = feed(&mut ed, b"three\r");
        let (a2, _) = feed(&mut ed, b"\n");
        assert_eq!(a1, vec![EditorAction::Submit("three".into())]);
        assert!(a2.is_empty());
    }

    #[test]
    fn backspace_and_del_edit() {
        let mut ed = LineEditor::new();
        let (_, out) = feed(&mut ed, b"cat\x08");
        assert!(out.ends_with(b"\x08 \x08"), "backspace-at-end fast path");
        assert_eq!(submit_one(&mut ed, b"r\r"), "car");
        // DEL (0x7f) behaves the same.
        feed(&mut ed, b"dog\x7f");
        assert_eq!(submit_one(&mut ed, b"\r"), "do");
        // Backspace on empty line rings the bell, no crash.
        let (_, out) = feed(&mut ed, b"\x08");
        assert_eq!(out, vec![0x07]);
    }

    #[test]
    fn arrows_and_midline_insert() {
        let mut ed = LineEditor::new();
        // "hlelo" -> fix via arrows: left 3, delete 'l', end, ...
        feed(&mut ed, b"helo");
        // Left twice: cursor between 'e' and 'l'.
        feed(&mut ed, b"\x1b[D\x1b[D");
        feed(&mut ed, b"l");
        assert_eq!(submit_one(&mut ed, b"\r"), "hello");
    }

    #[test]
    fn ss3_arrows_work_like_csi() {
        let mut ed = LineEditor::new();
        feed(&mut ed, b"ab");
        feed(&mut ed, b"\x1bOD"); // SS3 left
        feed(&mut ed, b"X");
        assert_eq!(submit_one(&mut ed, b"\r"), "aXb");
    }

    #[test]
    fn home_end_ctrl_a_e() {
        let mut ed = LineEditor::new();
        feed(&mut ed, b"world");
        feed(&mut ed, b"\x01"); // ^A
        feed(&mut ed, b"hello ");
        feed(&mut ed, b"\x05"); // ^E
        feed(&mut ed, b"!");
        assert_eq!(submit_one(&mut ed, b"\r"), "hello world!");
        // CSI H / F and vt220 1~/4~ variants.
        feed(&mut ed, b"mid");
        feed(&mut ed, b"\x1b[H");
        feed(&mut ed, b">");
        feed(&mut ed, b"\x1b[4~");
        feed(&mut ed, b"<");
        assert_eq!(submit_one(&mut ed, b"\r"), ">mid<");
    }

    #[test]
    fn delete_key_csi_3_tilde() {
        let mut ed = LineEditor::new();
        feed(&mut ed, b"abc\x1b[H\x1b[3~");
        assert_eq!(submit_one(&mut ed, b"\r"), "bc");
    }

    #[test]
    fn ctrl_u_kills_to_start() {
        let mut ed = LineEditor::new();
        feed(&mut ed, b"delete all this\x15");
        assert!(ed.is_empty());
        assert_eq!(submit_one(&mut ed, b"kept\r"), "kept");
        // ^U mid-line only kills before the cursor.
        feed(&mut ed, b"headtail");
        feed(&mut ed, b"\x1b[D\x1b[D\x1b[D\x1b[D"); // before "tail"
        feed(&mut ed, b"\x15");
        assert_eq!(submit_one(&mut ed, b"\r"), "tail");
    }

    #[test]
    fn ctrl_w_kills_word() {
        let mut ed = LineEditor::new();
        feed(&mut ed, b"say hello there\x17");
        assert_eq!(submit_one(&mut ed, b"\r"), "say hello ");
        feed(&mut ed, b"one two  \x17"); // trailing spaces + word
        assert_eq!(submit_one(&mut ed, b"\r"), "one ");
        // ^W at start of line is a no-op.
        let (_, out) = feed(&mut ed, b"\x17");
        assert!(out.is_empty());
    }

    #[test]
    fn ctrl_c_clears_line() {
        let mut ed = LineEditor::new();
        let (actions, out) = feed(&mut ed, b"oops\x03");
        assert!(actions.is_empty());
        assert!(ed.is_empty());
        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("^C\r\n"), "echoes ^C + newline: {s:?}");
        assert_eq!(submit_one(&mut ed, b"fresh\r"), "fresh");
    }

    #[test]
    fn ctrl_d_empty_is_eof_quit_midline_deletes() {
        let mut ed = LineEditor::new();
        let (actions, _) = feed(&mut ed, b"\x04");
        assert_eq!(actions, vec![EditorAction::EofQuit]);
        // Mid-line ^D deletes the char under the cursor.
        feed(&mut ed, b"abc\x1b[H\x04");
        assert_eq!(submit_one(&mut ed, b"\r"), "bc");
    }

    #[test]
    fn history_up_down() {
        let mut ed = LineEditor::new();
        submit_one(&mut ed, b"first\r");
        submit_one(&mut ed, b"second\r");
        // Up: most recent first.
        feed(&mut ed, b"\x1b[A");
        assert_eq!(ed.line(), "second");
        feed(&mut ed, b"\x1b[A");
        assert_eq!(ed.line(), "first");
        // Past the oldest: bell, stays.
        let (_, out) = feed(&mut ed, b"\x1b[A");
        assert!(out.contains(&0x07));
        assert_eq!(ed.line(), "first");
        // Down back to the newest, then to the stashed (empty) line.
        feed(&mut ed, b"\x1b[B");
        assert_eq!(ed.line(), "second");
        feed(&mut ed, b"\x1b[B");
        assert_eq!(ed.line(), "");
        // A partial line survives a history round-trip.
        feed(&mut ed, b"partial");
        feed(&mut ed, b"\x1b[A");
        assert_eq!(ed.line(), "second");
        feed(&mut ed, b"\x1b[B");
        assert_eq!(ed.line(), "partial");
    }

    #[test]
    fn history_skips_empty_and_duplicate_lines() {
        let mut ed = LineEditor::new();
        submit_one(&mut ed, b"cmd\r");
        submit_one(&mut ed, b"\r"); // empty: not recorded
        submit_one(&mut ed, b"cmd\r"); // duplicate of back(): not recorded
        feed(&mut ed, b"\x1b[A");
        assert_eq!(ed.line(), "cmd");
        let (_, out) = feed(&mut ed, b"\x1b[A");
        assert!(out.contains(&0x07), "only one entry recorded");
    }

    #[test]
    fn forget_last_history_drops_the_sensitive_line() {
        let mut ed = LineEditor::new();
        submit_one(&mut ed, b"look\r");
        submit_one(&mut ed, b"connect nick hunter2\r");
        ed.forget_last_history();
        // Up recalls "look", not the credential line.
        feed(&mut ed, b"\x1b[A");
        assert_eq!(ed.line(), "look");
        let (_, out) = feed(&mut ed, b"\x1b[A");
        assert!(out.contains(&0x07), "only one entry left");
        // Forgetting on an empty history is harmless.
        let mut ed2 = LineEditor::new();
        ed2.forget_last_history();
        let (_, out) = feed(&mut ed2, b"\x1b[A");
        assert!(out.contains(&0x07));
    }

    #[test]
    fn history_caps_at_limit() {
        let mut ed = LineEditor::new();
        for i in 0..(HISTORY_CAP + 50) {
            submit_one(&mut ed, format!("line{i}\r").as_bytes());
        }
        // Walk all the way back: exactly HISTORY_CAP steps.
        let mut ups = 0;
        loop {
            let (_, out) = feed(&mut ed, b"\x1b[A");
            if out.contains(&0x07) {
                break;
            }
            ups += 1;
        }
        assert_eq!(ups, HISTORY_CAP);
        assert_eq!(ed.line(), "line50"); // oldest survivor
    }

    #[test]
    fn masked_mode_echoes_stars_and_skips_history() {
        let mut ed = LineEditor::new();
        ed.set_mode(EchoMode::Masked);
        let (_, out) = feed(&mut ed, b"secret");
        assert_eq!(out, b"******");
        // Backspace in masked mode erases a star.
        let (_, out) = feed(&mut ed, b"\x08");
        assert_eq!(out, b"\x08 \x08");
        assert_eq!(submit_one(&mut ed, b"\r"), "secre");
        // Not recorded in history; up/down are inert in masked mode.
        let (_, out) = feed(&mut ed, b"\x1b[A");
        assert!(out.is_empty());
        assert!(ed.is_empty());
        ed.set_mode(EchoMode::Normal);
        let (_, out) = feed(&mut ed, b"\x1b[A");
        assert!(out.contains(&0x07), "history is empty");
    }

    #[test]
    fn noecho_mode_emits_nothing_but_newline() {
        let mut ed = LineEditor::new();
        ed.set_mode(EchoMode::NoEcho);
        let (_, out) = feed(&mut ed, b"hidden");
        assert!(out.is_empty());
        let mut out = Vec::new();
        let actions = ed.feed(b"\r", &mut out);
        assert_eq!(actions, vec![EditorAction::Submit("hidden".into())]);
        assert_eq!(out, b"\r\n");
    }

    #[test]
    fn utf8_multibyte_and_split_sequences() {
        let mut ed = LineEditor::new();
        let (_, out) = feed(&mut ed, "café".as_bytes());
        assert_eq!(out, "café".as_bytes());
        assert_eq!(submit_one(&mut ed, b"\r"), "café");
        // é = 0xC3 0xA9 split across two feeds.
        feed(&mut ed, &[0xC3]);
        assert!(ed.is_empty(), "partial sequence not yet inserted");
        feed(&mut ed, &[0xA9]);
        assert_eq!(ed.line(), "é");
        assert_eq!(submit_one(&mut ed, b"\r"), "é");
        // 4-byte emoji split three ways.
        let bytes = "🎲".as_bytes();
        feed(&mut ed, &bytes[..1]);
        feed(&mut ed, &bytes[1..3]);
        feed(&mut ed, &bytes[3..]);
        assert_eq!(submit_one(&mut ed, b"\r"), "🎲");
    }

    #[test]
    fn invalid_utf8_is_dropped_with_bell() {
        let mut ed = LineEditor::new();
        // Stray continuation byte.
        let (_, out) = feed(&mut ed, &[0x80]);
        assert_eq!(out, vec![0x07]);
        // Lead byte followed by ASCII: sequence dropped, ASCII processed.
        let (_, out) = feed(&mut ed, &[0xC3, b'x']);
        assert_eq!(out, vec![0x07, b'x']);
        assert_eq!(submit_one(&mut ed, b"\r"), "x");
        // Overlong/invalid completed sequence: bell, nothing inserted.
        let (_, out) = feed(&mut ed, &[0xC0, 0x80]);
        assert_eq!(out, vec![0x07]);
        assert!(ed.is_empty());
    }

    #[test]
    fn decoded_control_chars_are_rejected() {
        let mut ed = LineEditor::new();
        // U+009B (C1 CSI) arrives as valid UTF-8 0xC2 0x9B — must not enter
        // the buffer (ANSI injection via paste).
        let (_, out) = feed(&mut ed, &[0xC2, 0x9B]);
        assert_eq!(out, vec![0x07]);
        assert!(ed.is_empty());
    }

    #[test]
    fn line_cap_rings_bell() {
        let mut ed = LineEditor::new();
        let big = vec![b'a'; MAX_LINE_CHARS];
        feed(&mut ed, &big);
        let (_, out) = feed(&mut ed, b"b");
        assert_eq!(out, vec![0x07]);
        let line = submit_one(&mut ed, b"\r");
        assert_eq!(line.len(), MAX_LINE_CHARS);
    }

    #[test]
    fn redraw_reprints_prompt_and_buffer() {
        let mut ed = LineEditor::new();
        ed.set_prompt("> ");
        feed(&mut ed, b"hello");
        feed(&mut ed, b"\x1b[D\x1b[D"); // cursor after "hel"
        let mut out = Vec::new();
        ed.redraw(&mut out);
        assert_eq!(out, b"\r\x1b[K> hello\x1b[2D");
        // Masked redraw shows stars.
        ed.set_mode(EchoMode::Masked);
        let mut out = Vec::new();
        ed.redraw(&mut out);
        assert_eq!(out, b"\r\x1b[K> *****\x1b[2D");
        // NoEcho redraw shows only the prompt.
        ed.set_mode(EchoMode::NoEcho);
        let mut out = Vec::new();
        ed.redraw(&mut out);
        assert_eq!(out, b"\r\x1b[K> ");
    }

    #[test]
    fn midline_insert_redraws_fully() {
        let mut ed = LineEditor::new();
        ed.set_prompt("> ");
        feed(&mut ed, b"ac");
        feed(&mut ed, b"\x1b[D");
        let (_, out) = feed(&mut ed, b"b");
        // Full redraw: erase, prompt, "abc", cursor back over "c".
        assert_eq!(out, b"\r\x1b[K> abc\x1b[1D");
        assert_eq!(submit_one(&mut ed, b"\r"), "abc");
    }

    #[test]
    fn has_visible_line_tracks_prompt_and_buffer() {
        let mut ed = LineEditor::new();
        assert!(!ed.has_visible_line());
        feed(&mut ed, b"x");
        assert!(ed.has_visible_line());
        submit_one(&mut ed, b"\r");
        assert!(!ed.has_visible_line());
        ed.set_prompt("> ");
        assert!(ed.has_visible_line());
        // NoEcho hides the buffer but not the prompt.
        ed.set_prompt("");
        ed.set_mode(EchoMode::NoEcho);
        feed(&mut ed, b"hidden");
        assert!(!ed.has_visible_line());
    }

    #[test]
    fn split_csi_across_feeds() {
        let mut ed = LineEditor::new();
        feed(&mut ed, b"ab");
        feed(&mut ed, &[0x1b]);
        feed(&mut ed, b"[");
        feed(&mut ed, b"D");
        feed(&mut ed, b"X");
        assert_eq!(submit_one(&mut ed, b"\r"), "aXb");
    }

    #[test]
    fn unknown_csi_sequences_are_swallowed() {
        let mut ed = LineEditor::new();
        // Mouse/function-key noise must not corrupt the buffer.
        feed(&mut ed, b"\x1b[15~\x1b[200~\x1b[?25h");
        assert!(ed.is_empty());
        assert_eq!(submit_one(&mut ed, b"ok\r"), "ok");
    }

    #[test]
    fn tab_is_ignored() {
        let mut ed = LineEditor::new();
        feed(&mut ed, b"a\tb");
        assert_eq!(submit_one(&mut ed, b"\r"), "ab");
    }

    fn count_subslice(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|w| *w == needle)
            .count()
    }

    #[test]
    fn midline_edit_burst_coalesces_to_single_redraw() {
        let mut ed = LineEditor::new();
        ed.set_prompt("> ");
        let seed = vec![b'a'; MAX_LINE_CHARS - 1];
        feed(&mut ed, &seed);
        // Move cursor one left so every insert/backspace is a mid-line edit.
        feed(&mut ed, b"\x1b[D");

        // 16 KiB burst of alternating mid-line insert + backspace (8,192 pairs).
        let mut burst = Vec::with_capacity(16 * 1024);
        for _ in 0..8_192 {
            burst.extend_from_slice(b"x\x7f");
        }
        let (actions, out) = feed(&mut ed, &burst);
        assert!(actions.is_empty());
        assert!(std::str::from_utf8(&out).is_ok());
        assert_eq!(
            count_subslice(&out, ERASE_LINE),
            1,
            "mid-line edit burst must emit exactly one coalesced redraw"
        );
        assert!(
            out.len() <= MAX_FEED_OUT_BYTES,
            "output len {} exceeded cap {MAX_FEED_OUT_BYTES}",
            out.len()
        );
        assert_eq!(ed.line().len(), MAX_LINE_CHARS - 1);
    }

    #[test]
    fn history_toggle_burst_coalesces_to_single_redraw() {
        let mut ed = LineEditor::new();
        ed.set_prompt("> ");
        let mut first = vec![b'a'; MAX_LINE_CHARS];
        first.push(b'\r');
        let mut second = vec![b'b'; MAX_LINE_CHARS];
        second.push(b'\r');
        submit_one(&mut ed, &first);
        submit_one(&mut ed, &second);

        // 18 KiB burst of Up/Down arrows (3,072 pairs).
        let mut burst = Vec::with_capacity(18 * 1024);
        for _ in 0..3_072 {
            burst.extend_from_slice(b"\x1b[A\x1b[B");
        }
        // End on Up so a full MAX_LINE_CHARS line is shown.
        burst.extend_from_slice(b"\x1b[A");
        let (actions, out) = feed(&mut ed, &burst);
        assert!(actions.is_empty());
        assert!(std::str::from_utf8(&out).is_ok());
        assert_eq!(
            count_subslice(&out, ERASE_LINE),
            1,
            "history Up/Down burst must emit exactly one coalesced redraw"
        );
        assert!(out.len() <= MAX_FEED_OUT_BYTES);
        assert_eq!(ed.line().len(), MAX_LINE_CHARS);
    }

    #[test]
    fn history_submit_burst_caps_output_and_action_growth() {
        let mut ed = LineEditor::new();
        // Record a max-length 4-byte UTF-8 line (32 KiB).
        let mut big = "🎲".repeat(MAX_LINE_CHARS).into_bytes();
        big.push(b'\r');
        submit_one(&mut ed, &big);

        // 32 KiB burst of Up + Submit (8,192 repetitions).
        let mut burst = Vec::with_capacity(32 * 1024);
        for _ in 0..8_192 {
            burst.extend_from_slice(b"\x1b[A\r");
        }
        let (actions, out) = feed(&mut ed, &burst);
        assert!(
            std::str::from_utf8(&out).is_ok(),
            "capped output must stay valid UTF-8"
        );
        assert!(
            out.len() <= MAX_FEED_OUT_BYTES,
            "output len {} exceeded cap {MAX_FEED_OUT_BYTES}",
            out.len()
        );
        let action_bytes: usize = actions
            .iter()
            .map(|a| match a {
                EditorAction::Submit(s) => s.len(),
                EditorAction::EofQuit => 0,
            })
            .sum();
        assert!(
            action_bytes <= burst.len() * 4,
            "submitted action bytes {action_bytes} should be bounded"
        );
    }

    #[test]
    fn coalesced_redraw_flushes_once_before_submit_and_ctrl_c() {
        let mut ed = LineEditor::new();
        ed.set_prompt("> ");
        feed(&mut ed, b"ac");
        feed(&mut ed, b"\x1b[D");
        // Multiple mid-line edits then submit in one feed: one redraw before CRLF.
        let (actions, out) = feed(&mut ed, b"b1\x7f\r");
        assert_eq!(actions, vec![EditorAction::Submit("abc".into())]);
        assert_eq!(count_subslice(&out, ERASE_LINE), 1);
        assert!(out.ends_with(b"\r\n"));

        // Multiple mid-line edits then Ctrl-C: one redraw before ^C, one for cleared prompt.
        feed(&mut ed, b"ac\x1b[D");
        let (actions, out) = feed(&mut ed, b"b1\x7f\x03");
        assert!(actions.is_empty());
        assert!(ed.is_empty());
        assert_eq!(count_subslice(&out, ERASE_LINE), 2);
        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("> abc\x1b[1D^C\r\n\r\x1b[K> "), "{s:?}");
    }

    #[test]
    fn feed_with_history_false_suppresses_pipelined_credentials_and_clear_history_wipes() {
        let mut ed = LineEditor::new();
        let mut out = Vec::new();
        let actions = ed.feed_with_history(b"connect alice hunter2\rsay hello\r", &mut out, false);
        assert_eq!(
            actions,
            vec![
                EditorAction::Submit("connect alice hunter2".into()),
                EditorAction::Submit("say hello".into()),
            ]
        );
        // Up-arrow recalls nothing and rings the bell.
        let (_, out) = feed(&mut ed, b"\x1b[A");
        assert_eq!(out, vec![BELL]);
        assert!(ed.is_empty());

        // Normal recording still works, and clear_history wipes all entries.
        submit_one(&mut ed, b"look\r");
        submit_one(&mut ed, b"who\r");
        ed.clear_history();
        let (_, out) = feed(&mut ed, b"\x1b[A");
        assert_eq!(out, vec![BELL]);
        assert!(ed.is_empty());
    }

    #[test]
    fn cursor_zero_edit_burst_and_empty_submit_flood_stay_bounded() {
        let mut ed = LineEditor::new();
        ed.set_prompt("> ");
        let seed = vec![b'a'; MAX_LINE_CHARS - 1];
        feed(&mut ed, &seed);
        // Move to cursor 0 (Ctrl-A), then send 32 KiB of alternating insert +
        // backspace (P2-CLIENT-3): previously ~1.07 GiB of synchronous memmove.
        feed(&mut ed, b"\x01");
        assert_eq!(ed.cursor(), 0);
        let mut burst = Vec::with_capacity(32 * 1024);
        for _ in 0..16_384 {
            burst.extend_from_slice(b"x\x7f");
        }
        let mut out = Vec::new();
        let actions = ed.feed_with_history(&burst, &mut out, false);
        assert!(actions.is_empty());
        assert_eq!(ed.cursor(), 0);
        assert_eq!(ed.line().len(), MAX_LINE_CHARS - 1);
        assert!(out.len() <= MAX_FEED_OUT_BYTES);
        assert_eq!(count_subslice(&out, ERASE_LINE), 1);

        // 32 KiB of empty submits or Ctrl-D is bounded by MAX_FEED_ACTIONS.
        let mut ed2 = LineEditor::new();
        let (cr_actions, _) = feed(&mut ed2, &vec![b'\r'; 32 * 1024]);
        assert_eq!(cr_actions.len(), MAX_FEED_ACTIONS);
        let (eof_actions, _) = feed(&mut ed2, &vec![0x04; 32 * 1024]);
        assert_eq!(eof_actions.len(), MAX_FEED_ACTIONS);

        // BiDi and Unicode line separators are rejected with bell.
        let mut ed3 = LineEditor::new();
        let (_, out) = feed(&mut ed3, "a\u{202e}\u{2028}\u{2066}b".as_bytes());
        assert_eq!(ed3.line(), "ab");
        assert_eq!(out.iter().filter(|&&b| b == BELL).count(), 3);
    }
}
