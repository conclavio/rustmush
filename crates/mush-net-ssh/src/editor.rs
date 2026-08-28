//! Server-side raw-mode line editor (design 02 §5.4).
//!
//! When an SSH client allocates a PTY its terminal goes raw and *all* line
//! discipline is the server's job: echo, backspace, cursor movement,
//! history, masked password entry. [`LineEditor`] is that discipline as a
//! pure state machine — [`feed`](LineEditor::feed) takes raw channel bytes
//! and appends the echo/redraw bytes for the client to `out`; it performs
//! no I/O of its own, which keeps it unit-testable and fuzz-friendly.
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

use std::collections::VecDeque;

/// Maximum line length in characters; further input rings the bell.
pub const MAX_LINE_CHARS: usize = 8192;

/// Per-connection history depth.
pub const HISTORY_CAP: usize = 200;

const BELL: u8 = 0x07;
const CRLF: &[u8] = b"\r\n";
/// CR + erase-to-end-of-line: start of every full-line redraw.
const ERASE_LINE: &[u8] = b"\r\x1b[K";

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
    buf: Vec<char>,
    cursor: usize,
    prompt: String,
    mode: EchoMode,
    history: VecDeque<String>,
    /// `Some(i)` while navigating: index into `history` being shown.
    hist_pos: Option<usize>,
    /// The in-progress line stashed when history navigation began.
    saved: Vec<char>,
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
            buf: Vec::new(),
            cursor: 0,
            prompt: String::new(),
            mode: EchoMode::Normal,
            history: VecDeque::new(),
            hist_pos: None,
            saved: Vec::new(),
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
            self.hist_pos = None;
        }
        self.mode = mode;
    }

    /// The current echo mode.
    pub fn mode(&self) -> EchoMode {
        self.mode
    }

    /// Whether the edit buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// The current buffer content (for tests/diagnostics).
    pub fn line(&self) -> String {
        self.buf.iter().collect()
    }

    /// Drop the most recently submitted line from history. Used by the
    /// welcome screen after a `connect`/`create` line so plaintext
    /// passwords are not retained (or recallable with ↑) for the life of
    /// the connection.
    pub fn forget_last_history(&mut self) {
        self.history.pop_back();
        self.hist_pos = None;
    }

    /// Whether anything of this editor is visible on the client's current
    /// terminal line — used by the output pump to decide whether async
    /// output must erase and re-print the line around itself.
    pub fn has_visible_line(&self) -> bool {
        !self.prompt.is_empty() || (self.mode != EchoMode::NoEcho && !self.buf.is_empty())
    }

    /// Re-print the prompt and buffer from column 0 (CR, erase line,
    /// prompt, visible buffer, cursor repositioned). Used after async
    /// output interrupted the line and internally after mid-line edits.
    pub fn redraw(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(ERASE_LINE);
        out.extend_from_slice(self.prompt.as_bytes());
        match self.mode {
            EchoMode::Normal => {
                let mut s = String::with_capacity(self.buf.len());
                s.extend(self.buf.iter());
                out.extend_from_slice(s.as_bytes());
            }
            EchoMode::Masked => out.extend(std::iter::repeat_n(b'*', self.buf.len())),
            EchoMode::NoEcho => {}
        }
        if self.mode != EchoMode::NoEcho {
            let back = self.buf.len() - self.cursor;
            if back > 0 {
                out.extend_from_slice(format!("\x1b[{back}D").as_bytes());
            }
        }
    }

    /// Feed raw channel bytes. Echo/redraw bytes for the client are
    /// appended to `out`; completed lines and EOF-quit requests are
    /// returned in order.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<u8>) -> Vec<EditorAction> {
        let mut actions = Vec::new();
        for &b in bytes {
            self.feed_byte(b, out, &mut actions);
        }
        actions
    }

    fn feed_byte(&mut self, b: u8, out: &mut Vec<u8>, actions: &mut Vec<EditorAction>) {
        // Escape sequences are pure ASCII; they pre-empt UTF-8 assembly.
        if self.esc != EscState::None {
            self.feed_esc(b, out);
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
                                self.insert_char(c, out);
                            }
                        }
                        Err(_) => out.push(BELL),
                    }
                }
                return;
            }
            // Broken sequence: drop it, then process this byte normally.
            self.pending.clear();
            out.push(BELL);
        }
        let was_cr = self.last_was_cr;
        self.last_was_cr = false;
        match b {
            0x1b => self.esc = EscState::Esc,
            b'\r' => {
                self.last_was_cr = true;
                self.submit(out, actions);
            }
            b'\n' => {
                if !was_cr {
                    self.submit(out, actions);
                }
            }
            0x08 | 0x7f => self.backspace(out),
            0x01 => self.home(out),
            0x05 => self.end(out),
            0x15 => self.kill_to_start(out),
            0x17 => self.kill_word(out),
            0x03 => self.ctrl_c(out),
            0x04 => {
                if self.buf.is_empty() {
                    actions.push(EditorAction::EofQuit);
                } else {
                    self.delete_at_cursor(out);
                }
            }
            0x09 => {} // Tab reserved for future completion.
            0x00..=0x1f => {}
            0x20..=0x7e => self.insert_char(b as char, out),
            0xc0..=0xdf => self.start_utf8(b, 2),
            0xe0..=0xef => self.start_utf8(b, 3),
            0xf0..=0xf7 => self.start_utf8(b, 4),
            _ => out.push(BELL), // stray continuation / invalid lead byte
        }
    }

    fn start_utf8(&mut self, b: u8, len: usize) {
        self.pending.clear();
        self.pending.push(b);
        self.pending_len = len;
    }

    fn feed_esc(&mut self, b: u8, out: &mut Vec<u8>) {
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
                0x40..=0x7e => self.dispatch_key(&params, b, out),
                _ => {} // C0 inside CSI: abort the sequence.
            },
            EscState::Ss3 => self.dispatch_key("", b, out),
        }
    }

    fn dispatch_key(&mut self, params: &str, final_byte: u8, out: &mut Vec<u8>) {
        match final_byte {
            b'A' => self.history_up(out),
            b'B' => self.history_down(out),
            b'C' => self.right(out),
            b'D' => self.left(out),
            b'H' => self.home(out),
            b'F' => self.end(out),
            b'~' => match params.split(';').next().unwrap_or("") {
                "1" | "7" => self.home(out),
                "4" | "8" => self.end(out),
                "3" => self.delete_at_cursor(out),
                _ => {}
            },
            _ => {}
        }
    }

    // ---- buffer edits ---------------------------------------------------

    fn insert_char(&mut self, c: char, out: &mut Vec<u8>) {
        // Never let decoded control characters (incl. C1) into the buffer:
        // they are the ANSI-injection vector.
        if c.is_control() {
            out.push(BELL);
            return;
        }
        if self.buf.len() >= MAX_LINE_CHARS {
            out.push(BELL);
            return;
        }
        if self.cursor == self.buf.len() {
            self.buf.push(c);
            self.cursor += 1;
            match self.mode {
                EchoMode::Normal => {
                    let mut b = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
                }
                EchoMode::Masked => out.push(b'*'),
                EchoMode::NoEcho => {}
            }
        } else {
            self.buf.insert(self.cursor, c);
            self.cursor += 1;
            self.redraw(out);
        }
    }

    fn backspace(&mut self, out: &mut Vec<u8>) {
        if self.cursor == 0 {
            out.push(BELL);
            return;
        }
        let at_end = self.cursor == self.buf.len();
        let removed = self.buf.remove(self.cursor - 1);
        self.cursor -= 1;
        match self.mode {
            EchoMode::NoEcho => {}
            EchoMode::Masked => {
                if at_end {
                    out.extend_from_slice(b"\x08 \x08");
                } else {
                    self.redraw(out);
                }
            }
            EchoMode::Normal => {
                if at_end && removed.is_ascii() {
                    out.extend_from_slice(b"\x08 \x08");
                } else {
                    self.redraw(out);
                }
            }
        }
    }

    fn delete_at_cursor(&mut self, out: &mut Vec<u8>) {
        if self.cursor < self.buf.len() {
            self.buf.remove(self.cursor);
            self.redraw(out);
        } else {
            out.push(BELL);
        }
    }

    fn kill_to_start(&mut self, out: &mut Vec<u8>) {
        if self.cursor > 0 {
            self.buf.drain(..self.cursor);
            self.cursor = 0;
            self.redraw(out);
        }
    }

    fn kill_word(&mut self, out: &mut Vec<u8>) {
        if self.cursor == 0 {
            return;
        }
        let mut i = self.cursor;
        while i > 0 && self.buf[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !self.buf[i - 1].is_whitespace() {
            i -= 1;
        }
        self.buf.drain(i..self.cursor);
        self.cursor = i;
        self.redraw(out);
    }

    fn ctrl_c(&mut self, out: &mut Vec<u8>) {
        self.buf.clear();
        self.cursor = 0;
        self.hist_pos = None;
        out.extend_from_slice(b"^C");
        out.extend_from_slice(CRLF);
        self.redraw(out);
    }

    // ---- cursor movement ------------------------------------------------

    fn move_echo(&self, n: usize, dir: u8, out: &mut Vec<u8>) {
        if n > 0 && self.mode != EchoMode::NoEcho {
            out.extend_from_slice(format!("\x1b[{n}{}", dir as char).as_bytes());
        }
    }

    fn home(&mut self, out: &mut Vec<u8>) {
        self.move_echo(self.cursor, b'D', out);
        self.cursor = 0;
    }

    fn end(&mut self, out: &mut Vec<u8>) {
        self.move_echo(self.buf.len() - self.cursor, b'C', out);
        self.cursor = self.buf.len();
    }

    fn left(&mut self, out: &mut Vec<u8>) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.move_echo(1, b'D', out);
        }
    }

    fn right(&mut self, out: &mut Vec<u8>) {
        if self.cursor < self.buf.len() {
            self.cursor += 1;
            self.move_echo(1, b'C', out);
        }
    }

    // ---- history --------------------------------------------------------

    fn history_up(&mut self, out: &mut Vec<u8>) {
        if self.mode != EchoMode::Normal {
            return;
        }
        if self.history.is_empty() {
            out.push(BELL);
            return;
        }
        match self.hist_pos {
            None => {
                self.saved = std::mem::take(&mut self.buf);
                self.hist_pos = Some(self.history.len() - 1);
            }
            Some(0) => {
                out.push(BELL);
                return;
            }
            Some(p) => self.hist_pos = Some(p - 1),
        }
        if let Some(p) = self.hist_pos {
            self.buf = self.history[p].chars().collect();
        }
        self.cursor = self.buf.len();
        self.redraw(out);
    }

    fn history_down(&mut self, out: &mut Vec<u8>) {
        if self.mode != EchoMode::Normal {
            return;
        }
        match self.hist_pos {
            None => {
                out.push(BELL);
                return;
            }
            Some(p) if p + 1 < self.history.len() => {
                self.hist_pos = Some(p + 1);
                self.buf = self.history[p + 1].chars().collect();
            }
            Some(_) => {
                self.hist_pos = None;
                self.buf = std::mem::take(&mut self.saved);
            }
        }
        self.cursor = self.buf.len();
        self.redraw(out);
    }

    // ---- submit ---------------------------------------------------------

    fn submit(&mut self, out: &mut Vec<u8>, actions: &mut Vec<EditorAction>) {
        let line: String = self.buf.drain(..).collect();
        self.cursor = 0;
        self.hist_pos = None;
        self.saved.clear();
        out.extend_from_slice(CRLF);
        if self.mode == EchoMode::Normal && !line.is_empty() && self.history.back() != Some(&line) {
            self.history.push_back(line.clone());
            if self.history.len() > HISTORY_CAP {
                self.history.pop_front();
            }
        }
        actions.push(EditorAction::Submit(line));
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
}
