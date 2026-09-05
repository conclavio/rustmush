//! [`LineBuffer`] — the one line-editing core (interface plan I2, D-I-6):
//! a buffer of characters, a cursor, and a history ring, with every edit
//! the classic keys perform. [`LineEditor`](crate::LineEditor) puts echo
//! and escape decoding on top of it for line mode; the TUI drives it from
//! decoded keys. Pure: no I/O, no bytes, no clocks.
//!
//! Every mutating method reports whether it changed anything (or what it
//! removed), so a caller can ring a bell or redraw only when needed, and
//! the two drivers stay byte-for-byte faithful to their old behaviour.

use std::collections::VecDeque;

/// Maximum line length in characters; further input is refused.
pub const MAX_LINE_CHARS: usize = 8192;

/// Per-connection history depth.
pub const HISTORY_CAP: usize = 200;

/// A line under edit plus its history.
#[derive(Clone, Debug)]
pub struct LineBuffer {
    buf: Vec<char>,
    cursor: usize,
    history: VecDeque<String>,
    history_cap: usize,
    /// `Some(i)` while navigating: the index into `history` being shown.
    hist_pos: Option<usize>,
    /// The in-progress line stashed when history navigation began.
    saved: Vec<char>,
}

impl Default for LineBuffer {
    fn default() -> Self {
        Self::new(HISTORY_CAP)
    }
}

impl LineBuffer {
    /// An empty buffer keeping at most `history_cap` submitted lines.
    #[must_use]
    pub fn new(history_cap: usize) -> Self {
        Self {
            buf: Vec::new(),
            cursor: 0,
            history: VecDeque::new(),
            history_cap,
            hist_pos: None,
            saved: Vec::new(),
        }
    }

    /// The buffer as text.
    #[must_use]
    pub fn text(&self) -> String {
        self.buf.iter().collect()
    }

    /// The buffer's characters.
    #[must_use]
    pub fn chars(&self) -> &[char] {
        &self.buf
    }

    /// The cursor, as a character index into the buffer.
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Characters in the buffer.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Whether the buffer is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Whether the cursor sits after the last character.
    #[must_use]
    pub fn at_end(&self) -> bool {
        self.cursor == self.buf.len()
    }

    /// Recorded history lines, oldest first.
    #[must_use]
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Insert `c` at the cursor. Refused (`false`) for a control character
    /// — the ANSI-injection vector — or when the line is at
    /// [`MAX_LINE_CHARS`].
    pub fn insert(&mut self, c: char) -> bool {
        if c.is_control() || self.buf.len() >= MAX_LINE_CHARS {
            return false;
        }
        if self.cursor == self.buf.len() {
            self.buf.push(c);
        } else {
            self.buf.insert(self.cursor, c);
        }
        self.cursor += 1;
        true
    }

    /// Remove the character before the cursor, returning it.
    pub fn backspace(&mut self) -> Option<char> {
        if self.cursor == 0 {
            return None;
        }
        let removed = self.buf.remove(self.cursor - 1);
        self.cursor -= 1;
        Some(removed)
    }

    /// Remove the character under the cursor, returning it.
    pub fn delete(&mut self) -> Option<char> {
        if self.cursor < self.buf.len() {
            Some(self.buf.remove(self.cursor))
        } else {
            None
        }
    }

    /// Move the cursor one left; `false` at the start.
    pub fn left(&mut self) -> bool {
        if self.cursor > 0 {
            self.cursor -= 1;
            true
        } else {
            false
        }
    }

    /// Move the cursor one right; `false` at the end.
    pub fn right(&mut self) -> bool {
        if self.cursor < self.buf.len() {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    /// Move the cursor to the start, returning how far it moved.
    pub fn home(&mut self) -> usize {
        let moved = self.cursor;
        self.cursor = 0;
        moved
    }

    /// Move the cursor to the end, returning how far it moved.
    pub fn end(&mut self) -> usize {
        let moved = self.buf.len() - self.cursor;
        self.cursor = self.buf.len();
        moved
    }

    /// Delete everything before the cursor (`^U`); `false` when nothing was.
    pub fn kill_to_start(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.buf.drain(..self.cursor);
        self.cursor = 0;
        true
    }

    /// Delete the word before the cursor and the spaces after it (`^W`);
    /// `false` at the start of the line.
    pub fn kill_word(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
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
        true
    }

    /// Show the previous history line (stashing the in-progress line on
    /// the first step); `false` when there is nothing older.
    pub fn history_up(&mut self) -> bool {
        if self.history.is_empty() {
            return false;
        }
        match self.hist_pos {
            None => {
                self.saved = std::mem::take(&mut self.buf);
                self.hist_pos = Some(self.history.len() - 1);
            }
            Some(0) => return false,
            Some(p) => self.hist_pos = Some(p - 1),
        }
        if let Some(p) = self.hist_pos {
            self.buf = self.history[p].chars().collect();
        }
        self.cursor = self.buf.len();
        true
    }

    /// Show the next history line, or restore the stashed line past the
    /// newest; `false` when not navigating.
    pub fn history_down(&mut self) -> bool {
        match self.hist_pos {
            None => return false,
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
        true
    }

    /// Leave history navigation, keeping whatever the buffer shows.
    pub fn cancel_history(&mut self) {
        self.hist_pos = None;
    }

    /// Take the line, clearing the buffer. With `record`, a non-empty line
    /// that differs from the newest history entry is appended (the ring
    /// drops its oldest past the cap).
    pub fn submit(&mut self, record: bool) -> String {
        let line: String = self.buf.drain(..).collect();
        self.cursor = 0;
        self.hist_pos = None;
        self.saved.clear();
        if record && !line.is_empty() && self.history.back() != Some(&line) {
            self.history.push_back(line.clone());
            if self.history.len() > self.history_cap {
                self.history.pop_front();
            }
        }
        line
    }

    /// Replace the buffer with `text`, cursor at the end (control characters
    /// dropped); leaves history navigation.
    pub fn set(&mut self, text: &str) {
        self.buf = text.chars().filter(|c| !c.is_control()).collect();
        self.buf.truncate(MAX_LINE_CHARS);
        self.cursor = self.buf.len();
        self.hist_pos = None;
    }

    /// Empty the buffer and leave history navigation.
    pub fn clear(&mut self) {
        self.buf.clear();
        self.cursor = 0;
        self.hist_pos = None;
    }

    /// Drop the most recently recorded line (a credential the welcome
    /// screen must not keep recallable).
    pub fn forget_last_history(&mut self) {
        self.history.pop_back();
        self.hist_pos = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> LineBuffer {
        let mut b = LineBuffer::new(HISTORY_CAP);
        for c in s.chars() {
            assert!(b.insert(c));
        }
        b
    }

    #[test]
    fn insert_edit_and_move() {
        let mut b = typed("helo");
        assert!(b.left() && b.left());
        assert!(b.insert('l'));
        assert_eq!(b.text(), "hello");
        assert_eq!(b.cursor(), 3);
        assert_eq!(b.end(), 2);
        assert!(!b.right());
        assert_eq!(b.home(), 5);
        assert!(!b.left());
        assert_eq!(b.delete(), Some('h'));
        assert_eq!(b.text(), "ello");
        assert!(!b.insert('\u{7}'), "control characters never enter");
        assert!(!b.insert('\u{9b}'));
    }

    #[test]
    fn kills_and_backspace() {
        let mut b = typed("say hello there");
        assert!(b.kill_word());
        assert_eq!(b.text(), "say hello ");
        assert!(b.kill_to_start());
        assert!(b.is_empty());
        assert!(!b.kill_to_start() && !b.kill_word());
        assert_eq!(b.backspace(), None);
        let mut b = typed("ab");
        assert_eq!(b.backspace(), Some('b'));
        assert!(b.at_end());
    }

    #[test]
    fn history_round_trip_and_cap() {
        let mut b = LineBuffer::new(2);
        for l in ["first", "second", "second", "", "third"] {
            b.set(l);
            b.submit(true);
        }
        assert_eq!(b.history_len(), 2, "duplicates, empties and the cap");
        b.set("partial");
        assert!(b.history_up());
        assert_eq!(b.text(), "third");
        assert!(b.history_up());
        assert_eq!(b.text(), "second");
        assert!(!b.history_up(), "nothing older");
        assert!(b.history_down());
        assert!(b.history_down());
        assert_eq!(b.text(), "partial", "the stash comes back");
        assert!(!b.history_down());
        b.forget_last_history();
        assert_eq!(b.history_len(), 1);
        // A masked line is never recorded.
        b.set("hunter2");
        assert_eq!(b.submit(false), "hunter2");
        assert_eq!(b.history_len(), 1);
    }

    #[test]
    fn line_cap_holds() {
        let mut b = LineBuffer::default();
        for _ in 0..MAX_LINE_CHARS {
            assert!(b.insert('a'));
        }
        assert!(!b.insert('b'));
        assert_eq!(b.len(), MAX_LINE_CHARS);
        b.set(&"x".repeat(MAX_LINE_CHARS + 5));
        assert_eq!(b.len(), MAX_LINE_CHARS);
    }
}
