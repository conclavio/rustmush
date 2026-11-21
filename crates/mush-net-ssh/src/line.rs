//! [`LineBuffer`] — the one line-editing core (interface plan I2, D-I-6):
//! a buffer of characters, a cursor, and a history ring, with every edit
//! the classic keys perform. [`LineEditor`](crate::LineEditor) puts echo
//! and escape decoding on top of it for line mode; the TUI drives it from
//! decoded keys. Pure: no I/O, no bytes, no clocks.
//!
//! Every mutating method reports whether it changed anything (or what it
//! removed), so a caller can ring a bell or redraw only when needed, and
//! the two drivers stay byte-for-byte faithful to their old behaviour.
//!
//! Internally the line is stored in small [`VecDeque`] chunks (so edits at
//! cursor `0`, mid-line, or end are bounded and never perform O(N) full-line
//! `memmove`, eliminating `P2-CLIENT-3` cursor-0 amplification) and unedited
//! history navigation stays O(1) until the line is actually mutated.

use std::collections::VecDeque;

/// Maximum line length in characters; further input is refused.
pub const MAX_LINE_CHARS: usize = 8192;

/// Maximum UTF-8 byte length of a single buffered line (16 KiB). Prevents
/// 4-byte UTF-8 input from multiplying per-line and history memory 4× past
/// [`MAX_LINE_CHARS`] while still admitting full [`MAX_LINE_CHARS`] ASCII and
/// 2-byte UTF-8 lines.
pub const MAX_LINE_BYTES: usize = 16 * 1024;

/// Per-connection history depth.
pub const HISTORY_CAP: usize = 200;

/// Maximum characters stored in a single [`ChunkedChars`] block.
const CHUNK_MAX: usize = 64;
/// Split point when a full block overflows.
const CHUNK_SPLIT: usize = CHUNK_MAX / 2;
/// Coalesce threshold after deletions so adjacent tiny blocks merge and the
/// total block count stays bounded (`<= MAX_LINE_CHARS / 32 + 2`).
const CHUNK_MERGE_THRESHOLD: usize = CHUNK_SPLIT / 2;

/// Chunked character sequence keeping insert/delete at cursor 0 and end O(1),
/// mid-line edits O(1) amortized over tiny L1-resident blocks, and `home`/`end`
/// strictly O(1).
#[derive(Clone, Debug, Default)]
struct ChunkedChars {
    chunks: Vec<VecDeque<char>>,
    len: usize,
    utf8_bytes: usize,
}

impl ChunkedChars {
    fn len(&self) -> usize {
        self.len
    }

    fn utf8_bytes(&self) -> usize {
        self.utf8_bytes
    }

    fn first(&self) -> Option<char> {
        self.chunks.first().and_then(VecDeque::front).copied()
    }

    fn clear(&mut self) {
        self.chunks.clear();
        self.len = 0;
        self.utf8_bytes = 0;
    }

    fn push_back(&mut self, c: char) {
        if let Some(last) = self.chunks.last_mut()
            && last.len() < CHUNK_MAX
        {
            last.push_back(c);
        } else {
            let mut chunk = VecDeque::with_capacity(CHUNK_MAX);
            chunk.push_back(c);
            self.chunks.push(chunk);
        }
        self.len += 1;
        self.utf8_bytes = self.utf8_bytes.saturating_add(c.len_utf8());
    }

    fn insert_at(&mut self, index: usize, c: char) {
        if index >= self.len {
            self.push_back(c);
            return;
        }
        if index == 0 {
            if let Some(first) = self.chunks.first_mut() {
                if first.len() < CHUNK_MAX {
                    first.push_front(c);
                } else {
                    let right = first.split_off(CHUNK_SPLIT);
                    first.push_front(c);
                    self.chunks.insert(1, right);
                }
            } else {
                let mut chunk = VecDeque::with_capacity(CHUNK_MAX);
                chunk.push_back(c);
                self.chunks.push(chunk);
            }
            self.len += 1;
            self.utf8_bytes = self.utf8_bytes.saturating_add(c.len_utf8());
            return;
        }
        let mut rem = index;
        let mut ci = 0;
        while ci < self.chunks.len() {
            let clen = self.chunks[ci].len();
            if rem <= clen {
                break;
            }
            rem -= clen;
            ci += 1;
        }
        if ci >= self.chunks.len() {
            self.push_back(c);
            return;
        }
        if self.chunks[ci].len() >= CHUNK_MAX {
            let right = self.chunks[ci].split_off(CHUNK_SPLIT);
            self.chunks.insert(ci + 1, right);
            if rem > CHUNK_SPLIT {
                rem -= CHUNK_SPLIT;
                ci += 1;
            }
        }
        self.chunks[ci].insert(rem, c);
        self.len += 1;
        self.utf8_bytes = self.utf8_bytes.saturating_add(c.len_utf8());
    }

    fn remove_at(&mut self, index: usize) -> Option<char> {
        if index >= self.len {
            return None;
        }
        if index == 0 {
            let c = self.chunks.first_mut()?.pop_front()?;
            self.len -= 1;
            self.utf8_bytes = self.utf8_bytes.saturating_sub(c.len_utf8());
            self.fixup_after_remove(0);
            return Some(c);
        }
        if index + 1 == self.len {
            let last_idx = self.chunks.len().checked_sub(1)?;
            let c = self.chunks.get_mut(last_idx)?.pop_back()?;
            self.len -= 1;
            self.utf8_bytes = self.utf8_bytes.saturating_sub(c.len_utf8());
            self.fixup_after_remove(last_idx);
            return Some(c);
        }
        let (ci, off) = self.locate(index)?;
        let c = self.chunks.get_mut(ci)?.remove(off)?;
        self.len -= 1;
        self.utf8_bytes = self.utf8_bytes.saturating_sub(c.len_utf8());
        self.fixup_after_remove(ci);
        Some(c)
    }

    fn locate(&self, index: usize) -> Option<(usize, usize)> {
        if index >= self.len {
            return None;
        }
        let mut rem = index;
        for (ci, chunk) in self.chunks.iter().enumerate() {
            let clen = chunk.len();
            if rem < clen {
                return Some((ci, rem));
            }
            rem -= clen;
        }
        None
    }

    fn prev_pos(&self, ci: usize, off: usize) -> Option<(usize, usize)> {
        if off > 0 {
            Some((ci, off - 1))
        } else if ci > 0 {
            let prev_len = self.chunks.get(ci - 1)?.len();
            prev_len.checked_sub(1).map(|last| (ci - 1, last))
        } else {
            None
        }
    }

    fn fixup_after_remove(&mut self, ci: usize) {
        let Some(clen) = self.chunks.get(ci).map(VecDeque::len) else {
            return;
        };
        if clen == 0 {
            self.chunks.remove(ci);
            return;
        }
        if clen < CHUNK_MERGE_THRESHOLD {
            if ci + 1 < self.chunks.len() && clen + self.chunks[ci + 1].len() <= CHUNK_MAX {
                let right = self.chunks.remove(ci + 1);
                if let Some(left) = self.chunks.get_mut(ci) {
                    left.extend(right);
                }
            } else if ci > 0 && self.chunks[ci - 1].len() + clen <= CHUNK_MAX {
                let right = self.chunks.remove(ci);
                if let Some(left) = self.chunks.get_mut(ci - 1) {
                    left.extend(right);
                }
            }
        }
    }

    fn drain_prefix(&mut self, count: usize) {
        let count = count.min(self.len);
        if count == 0 {
            return;
        }
        if count == self.len {
            self.clear();
            return;
        }
        let mut rem = count;
        let mut drop_chunks = 0;
        let mut removed_bytes = 0usize;
        for chunk in &self.chunks {
            if rem >= chunk.len() {
                rem -= chunk.len();
                removed_bytes += chunk.iter().map(|c| c.len_utf8()).sum::<usize>();
                drop_chunks += 1;
            } else {
                break;
            }
        }
        if drop_chunks > 0 {
            self.chunks.drain(..drop_chunks);
        }
        if rem > 0
            && let Some(first) = self.chunks.first_mut()
        {
            for c in first.drain(..rem) {
                removed_bytes += c.len_utf8();
            }
            self.fixup_after_remove(0);
        }
        self.len -= count;
        self.utf8_bytes = self.utf8_bytes.saturating_sub(removed_bytes);
    }

    fn drain_range(&mut self, start: usize, end: usize) {
        let end = end.min(self.len);
        if start >= end {
            return;
        }
        if start == 0 {
            self.drain_prefix(end);
            return;
        }
        let Some((start_ci, start_off)) = self.locate(start) else {
            return;
        };
        let Some((end_ci, end_off)) = self.locate(end - 1) else {
            return;
        };
        let mut removed_bytes = 0usize;
        if start_ci == end_ci {
            if let Some(chunk) = self.chunks.get_mut(start_ci) {
                for c in chunk.drain(start_off..=end_off) {
                    removed_bytes += c.len_utf8();
                }
            }
            self.len -= end - start;
            self.utf8_bytes = self.utf8_bytes.saturating_sub(removed_bytes);
            self.fixup_after_remove(start_ci);
            return;
        }
        if let Some(tail) = self.chunks.get_mut(end_ci) {
            for c in tail.drain(..=end_off) {
                removed_bytes += c.len_utf8();
            }
        }
        if start_ci + 1 < end_ci {
            for chunk in self.chunks.drain((start_ci + 1)..end_ci) {
                removed_bytes += chunk.iter().map(|c| c.len_utf8()).sum::<usize>();
            }
        }
        if let Some(head) = self.chunks.get_mut(start_ci) {
            for c in head.drain(start_off..) {
                removed_bytes += c.len_utf8();
            }
        }
        self.len -= end - start;
        self.utf8_bytes = self.utf8_bytes.saturating_sub(removed_bytes);
        self.fixup_after_remove(start_ci + 1);
        self.fixup_after_remove(start_ci);
    }

    fn kill_word_before(&mut self, cursor: usize) -> Option<usize> {
        if cursor == 0 || cursor > self.len {
            return None;
        }
        let mut pos = cursor;
        let mut cur = self.locate(cursor - 1);
        while let Some((ci, off)) = cur {
            if self
                .chunks
                .get(ci)
                .and_then(|chunk| chunk.get(off))
                .is_some_and(|c| c.is_whitespace())
            {
                pos -= 1;
                cur = self.prev_pos(ci, off);
            } else {
                break;
            }
        }
        while let Some((ci, off)) = cur {
            if self
                .chunks
                .get(ci)
                .and_then(|chunk| chunk.get(off))
                .is_some_and(|c| !c.is_whitespace())
            {
                pos -= 1;
                cur = self.prev_pos(ci, off);
            } else {
                break;
            }
        }
        self.drain_range(pos, cursor);
        Some(pos)
    }

    fn load_clean_str(&mut self, text: &str) {
        self.clear();
        for c in text.chars() {
            self.push_back(c);
        }
    }

    fn collect_string(&self) -> String {
        let mut s = String::with_capacity(self.utf8_bytes);
        for chunk in &self.chunks {
            for &c in chunk {
                s.push(c);
            }
        }
        s
    }

    fn take_string(&mut self) -> String {
        let s = self.collect_string();
        self.clear();
        s
    }
}

/// Stored history entry with its precomputed character length so unedited
/// history recall (`↑`/`↓`) stays strictly O(1).
#[derive(Clone, Debug)]
struct HistoryEntry {
    text: String,
    char_len: usize,
}

/// Iterator over the characters currently in a [`LineBuffer`].
pub struct LineChars<'a> {
    inner: LineCharsInner<'a>,
    remaining: usize,
}

enum LineCharsInner<'a> {
    Chunked {
        chunks: std::slice::Iter<'a, VecDeque<char>>,
        current: Option<std::collections::vec_deque::Iter<'a, char>>,
    },
    Str(std::str::Chars<'a>),
}

impl Iterator for LineChars<'_> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        let ch = match &mut self.inner {
            LineCharsInner::Str(it) => it.next(),
            LineCharsInner::Chunked { chunks, current } => loop {
                if let Some(cur) = current
                    && let Some(&c) = cur.next()
                {
                    break Some(c);
                }
                match chunks.next() {
                    Some(next_chunk) => *current = Some(next_chunk.iter()),
                    None => {
                        *current = None;
                        break None;
                    }
                }
            },
        };
        if ch.is_some() {
            self.remaining = self.remaining.saturating_sub(1);
        }
        ch
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for LineChars<'_> {
    fn len(&self) -> usize {
        self.remaining
    }
}

/// A line under edit plus its history.
#[derive(Clone, Debug)]
pub struct LineBuffer {
    buf: ChunkedChars,
    cursor: usize,
    history: VecDeque<HistoryEntry>,
    history_cap: usize,
    /// `Some(i)` while navigating: the index into `history` being shown.
    hist_pos: Option<usize>,
    /// True while `hist_pos` points directly at an unmodified history entry
    /// (so repeated `↑`/`↓` navigation never copies or allocates).
    hist_unedited: bool,
    /// The in-progress line stashed when history navigation began.
    saved: ChunkedChars,
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
            buf: ChunkedChars::default(),
            cursor: 0,
            history: VecDeque::new(),
            history_cap,
            hist_pos: None,
            hist_unedited: false,
            saved: ChunkedChars::default(),
        }
    }

    fn active_history(&self) -> Option<&HistoryEntry> {
        self.hist_unedited
            .then_some(self.hist_pos)
            .flatten()
            .and_then(|p| self.history.get(p))
    }

    fn materialize_history(&mut self) {
        if !self.hist_unedited {
            return;
        }
        self.hist_unedited = false;
        self.buf.clear();
        if let Some(entry) = self.hist_pos.and_then(|p| self.history.get(p)) {
            let text = entry.text.clone();
            self.buf.load_clean_str(&text);
        }
    }

    /// Whether the buffer currently views an unedited history entry whose
    /// first mutation will copy `utf8_bytes()` characters into the edit buffer.
    #[must_use]
    pub(crate) fn is_history_unedited(&self) -> bool {
        self.hist_unedited
    }

    /// The buffer as text.
    #[must_use]
    pub fn text(&self) -> String {
        if let Some(entry) = self.active_history() {
            entry.text.clone()
        } else {
            self.buf.collect_string()
        }
    }

    /// Iterate over the buffer's characters in order without allocating.
    #[must_use]
    pub fn chars(&self) -> LineChars<'_> {
        if let Some(entry) = self.active_history() {
            LineChars {
                inner: LineCharsInner::Str(entry.text.chars()),
                remaining: entry.char_len,
            }
        } else {
            LineChars {
                inner: LineCharsInner::Chunked {
                    chunks: self.buf.chunks.iter(),
                    current: None,
                },
                remaining: self.buf.len(),
            }
        }
    }

    /// The first character in the buffer, if any, in O(1).
    #[must_use]
    pub fn first_char(&self) -> Option<char> {
        if let Some(entry) = self.active_history() {
            entry.text.chars().next()
        } else {
            self.buf.first()
        }
    }

    /// The cursor, as a character index into the buffer.
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Characters in the buffer.
    #[must_use]
    pub fn len(&self) -> usize {
        if let Some(entry) = self.active_history() {
            entry.char_len
        } else {
            self.buf.len()
        }
    }

    /// Current UTF-8 byte length of the buffer.
    #[must_use]
    pub fn utf8_bytes(&self) -> usize {
        if let Some(entry) = self.active_history() {
            entry.text.len()
        } else {
            self.buf.utf8_bytes()
        }
    }

    /// Whether the buffer is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the cursor sits after the last character.
    #[must_use]
    pub fn at_end(&self) -> bool {
        self.cursor == self.len()
    }

    /// Recorded history lines, oldest first.
    #[must_use]
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Insert `c` at the cursor. Refused (`false`) for a control character
    /// or forbidden BiDi/line-separator control — the terminal/BiDi injection
    /// vector — or when the line is at [`MAX_LINE_CHARS`] or
    /// `MAX_LINE_BYTES`.
    pub fn insert(&mut self, c: char) -> bool {
        if crate::is_stripped_terminal_char(c)
            || self.len() >= MAX_LINE_CHARS
            || self.utf8_bytes().saturating_add(c.len_utf8()) > MAX_LINE_BYTES
        {
            return false;
        }
        self.materialize_history();
        self.buf.insert_at(self.cursor, c);
        self.cursor += 1;
        true
    }

    /// Remove the character before the cursor, returning it.
    pub fn backspace(&mut self) -> Option<char> {
        if self.cursor == 0 {
            return None;
        }
        self.materialize_history();
        let removed = self.buf.remove_at(self.cursor - 1)?;
        self.cursor -= 1;
        Some(removed)
    }

    /// Remove the character under the cursor, returning it.
    pub fn delete(&mut self) -> Option<char> {
        if self.cursor >= self.len() {
            return None;
        }
        self.materialize_history();
        self.buf.remove_at(self.cursor)
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
        if self.cursor < self.len() {
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
        let len = self.len();
        let moved = len - self.cursor;
        self.cursor = len;
        moved
    }

    /// Delete everything before the cursor (`^U`); `false` when nothing was.
    pub fn kill_to_start(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.materialize_history();
        self.buf.drain_prefix(self.cursor);
        self.cursor = 0;
        true
    }

    /// Delete the word before the cursor and the spaces after it (`^W`);
    /// `false` at the start of the line.
    pub fn kill_word(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.materialize_history();
        let Some(new_cursor) = self.buf.kill_word_before(self.cursor) else {
            return false;
        };
        self.cursor = new_cursor;
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
        self.hist_unedited = true;
        self.cursor = self.len();
        true
    }

    /// Show the next history line, or restore the stashed line past the
    /// newest; `false` when not navigating.
    pub fn history_down(&mut self) -> bool {
        match self.hist_pos {
            None => return false,
            Some(p) if p + 1 < self.history.len() => {
                self.hist_pos = Some(p + 1);
                self.hist_unedited = true;
            }
            Some(_) => {
                self.hist_pos = None;
                self.hist_unedited = false;
                self.buf = std::mem::take(&mut self.saved);
            }
        }
        self.cursor = self.len();
        true
    }

    /// Leave history navigation, keeping whatever the buffer shows.
    pub fn cancel_history(&mut self) {
        self.materialize_history();
        self.hist_pos = None;
    }

    /// Take the line, clearing the buffer. With `record`, a non-empty line
    /// that differs from the newest history entry is appended (the ring
    /// drops its oldest past the cap).
    pub fn submit(&mut self, record: bool) -> String {
        let (line, char_len) = if self.hist_unedited {
            self.hist_unedited = false;
            self.buf.clear();
            self.hist_pos
                .and_then(|p| self.history.get(p))
                .map(|entry| (entry.text.clone(), entry.char_len))
                .unwrap_or_default()
        } else {
            let char_len = self.buf.len();
            let text = self.buf.take_string();
            (text, char_len)
        };
        self.cursor = 0;
        self.hist_pos = None;
        self.saved.clear();
        if record
            && self.history_cap > 0
            && !line.is_empty()
            && self.history.back().map(|e| e.text.as_str()) != Some(line.as_str())
        {
            self.history.push_back(HistoryEntry {
                text: line.clone(),
                char_len,
            });
            if self.history.len() > self.history_cap {
                self.history.pop_front();
            }
        }
        line
    }

    /// Replace the buffer with `text`, cursor at the end (control and BiDi /
    /// line-separator characters dropped, capped at [`MAX_LINE_CHARS`] and
    /// `MAX_LINE_BYTES`); leaves history navigation.
    pub fn set(&mut self, text: &str) {
        self.hist_unedited = false;
        self.hist_pos = None;
        self.buf.clear();
        for c in text.chars() {
            if crate::is_stripped_terminal_char(c) {
                continue;
            }
            if self.buf.len() >= MAX_LINE_CHARS
                || self.buf.utf8_bytes().saturating_add(c.len_utf8()) > MAX_LINE_BYTES
            {
                break;
            }
            self.buf.push_back(c);
        }
        self.cursor = self.buf.len();
    }

    /// Empty the buffer and leave history navigation.
    pub fn clear(&mut self) {
        self.buf.clear();
        self.cursor = 0;
        self.hist_pos = None;
        self.hist_unedited = false;
        self.saved.clear();
    }

    /// Drop the most recently recorded line (a credential the welcome
    /// screen must not keep recallable).
    pub fn forget_last_history(&mut self) {
        self.materialize_history();
        self.history.pop_back();
        self.hist_pos = None;
    }

    /// Wipe all recorded history entries and leave history navigation.
    /// Used when leaving pre-auth screens so welcome lines can never leak
    /// into in-game ↑ recall.
    pub fn clear_history(&mut self) {
        self.materialize_history();
        self.history.clear();
        self.hist_pos = None;
        self.saved.clear();
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
        assert!(
            !b.insert('\u{202e}') && !b.insert('\u{2066}') && !b.insert('\u{2028}'),
            "BiDi and Unicode line separators never enter"
        );
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

    #[test]
    fn utf8_byte_cap_bounds_multibyte_lines_and_recovers_on_edits() {
        let mut b = LineBuffer::default();
        // '🎲' is 4 UTF-8 bytes: at most MAX_LINE_BYTES / 4 fit.
        let max_dice = MAX_LINE_BYTES / 4;
        for _ in 0..max_dice {
            assert!(b.insert('🎲'));
        }
        assert!(
            !b.insert('🎲'),
            "must refuse once UTF-8 bytes hit MAX_LINE_BYTES"
        );
        assert_eq!(b.text().len(), MAX_LINE_BYTES);
        // Deleting one frees 4 bytes.
        assert_eq!(b.backspace(), Some('🎲'));
        assert!(b.insert('a'));
        assert!(b.text().len() <= MAX_LINE_BYTES);
        // `set` also enforces MAX_LINE_BYTES.
        b.set(&"🎲".repeat(MAX_LINE_CHARS));
        assert!(b.text().len() <= MAX_LINE_BYTES);
        assert_eq!(b.len(), max_dice);
    }

    #[test]
    fn clear_history_wipes_recorded_lines_and_navigation() {
        let mut b = LineBuffer::default();
        b.set("first");
        assert_eq!(b.utf8_bytes(), 5);
        b.submit(true);
        b.set("second");
        b.submit(true);
        assert_eq!(b.history_len(), 2);
        assert!(b.history_up());
        b.clear_history();
        assert_eq!(b.history_len(), 0);
        assert!(!b.history_up());
        assert!(!b.history_down());
        assert_eq!(
            b.text(),
            "second",
            "clear_history keeps currently shown line"
        );
    }

    #[test]
    fn cursor_zero_and_midline_edits_across_chunk_boundaries_stay_consistent() {
        // Fill near MAX_LINE_CHARS to span >100 chunks, then exercise cursor-0
        // insert/backspace, home/end alternation, mid-line kill_word and
        // kill_to_start across multiple chunk boundaries (`P2-CLIENT-3`).
        let mut b = LineBuffer::default();
        let seed = "word ".repeat(MAX_LINE_CHARS / 5 - 1);
        b.set(&seed);
        assert_eq!(b.text(), seed);
        assert_eq!(b.home(), seed.len());

        // 16,384 insert+backspace pairs at cursor 0 (zero O(N) memmove).
        for _ in 0..16_384 {
            assert!(b.insert('X'));
            assert_eq!(b.first_char(), Some('X'));
            assert_eq!(b.backspace(), Some('X'));
        }
        assert_eq!(b.cursor(), 0);
        assert_eq!(b.text(), seed);

        // Alternating home/end + insert/delete at both ends stays O(1).
        for _ in 0..4_096 {
            assert_eq!(b.end(), seed.len());
            assert!(b.insert('Z'));
            assert_eq!(b.backspace(), Some('Z'));
            assert_eq!(b.home(), seed.len());
            assert!(b.insert('A'));
            assert_eq!(b.backspace(), Some('A'));
        }
        assert_eq!(b.text(), seed);

        // Cross-chunk kill_word and kill_to_start.
        b.set(&format!("{} alpha   beta", "x".repeat(200)));
        assert!(b.kill_word());
        assert_eq!(b.text(), format!("{} alpha   ", "x".repeat(200)));
        assert!(b.kill_word());
        assert_eq!(b.text(), format!("{} ", "x".repeat(200)));
        // Move cursor into the middle of chunk 2 and kill_to_start.
        for _ in 0..50 {
            assert!(b.left());
        }
        assert!(b.kill_to_start());
        assert_eq!(b.text(), format!("{} ", "x".repeat(49)));
        assert_eq!(b.utf8_bytes(), b.text().len());
        assert_eq!(b.chars().len(), b.len());
    }

    #[test]
    fn lazy_history_navigation_stays_unedited_until_mutated() {
        let mut b = LineBuffer::default();
        b.set(&"a".repeat(MAX_LINE_CHARS));
        b.submit(true);
        b.set(&"b".repeat(MAX_LINE_CHARS));
        b.submit(true);

        assert!(b.history_up());
        assert!(b.is_history_unedited());
        assert_eq!(b.len(), MAX_LINE_CHARS);
        assert_eq!(b.utf8_bytes(), MAX_LINE_CHARS);
        assert_eq!(b.first_char(), Some('b'));
        assert!(b.history_up());
        assert!(b.is_history_unedited());
        assert_eq!(b.first_char(), Some('a'));
        // Moving cursor does not materialize history.
        assert_eq!(b.home(), MAX_LINE_CHARS);
        assert!(b.is_history_unedited());
        // Mutating materializes once and keeps exact state.
        assert_eq!(b.delete(), Some('a'));
        assert!(!b.is_history_unedited());
        assert_eq!(b.len(), MAX_LINE_CHARS - 1);
        assert!(b.insert('c'));
        assert_eq!(b.first_char(), Some('c'));
    }
}
