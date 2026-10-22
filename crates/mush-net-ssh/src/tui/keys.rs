//! [`KeyParser`] — raw PTY bytes to [`Key`]s for the modal TUI (interface
//! plan I2, D-I-17).
//!
//! One parser rule carries the whole Alt story: an `ESC` followed by a
//! printable byte *in the same burst* is `Alt+that key`, so `Alt+1` and
//! `Alt+t` decode alike; `ESC [` opens a CSI sequence and `ESC O` an SS3
//! one (arrows, Home/End, PgUp/PgDn, Delete, F-keys, Shift-Tab as `ESC [
//! Z`); an `ESC` that ends a burst is [`Key::Esc`]. A split Alt+key
//! therefore degrades to `Esc` then the key — leave input mode, keep the
//! draft, act in nav mode — which is the outcome Alt+key would have had, so
//! the ambiguity costs nothing. A CSI or SS3 sequence split across bursts
//! is still assembled (terminals write them whole; the state costs
//! nothing), and so is a UTF-8 character.
//!
//! Control characters decoded from UTF-8 (C1) are dropped, never
//! delivered: the injection boundary is the same as the line editor's.

/// One decoded key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A printable character (ASCII or decoded UTF-8).
    Char(char),
    /// A C0 control key as its letter: `Ctrl('a')` for `^A`, `Ctrl(']')`
    /// for `^]`, `Ctrl('k')` for `^K`.
    Ctrl(char),
    /// `ESC` + a printable byte in one burst.
    Alt(char),
    /// CR, LF, or CRLF (one key for the pair).
    Enter,
    /// A lone `ESC` at the end of a burst, or before a control byte.
    Esc,
    /// Backspace or DEL.
    Backspace,
    /// The Delete key (`ESC [ 3 ~`).
    Delete,
    /// Tab.
    Tab,
    /// Shift-Tab (`ESC [ Z`).
    BackTab,
    /// Cursor keys.
    Up,
    /// Cursor keys.
    Down,
    /// Cursor keys.
    Left,
    /// Cursor keys.
    Right,
    /// Home (`^A` is a separate `Ctrl('a')`).
    Home,
    /// End.
    End,
    /// Page up.
    PgUp,
    /// Page down.
    PgDn,
    /// A function key, `F(1)`..=`F(12)`.
    F(u8),
    /// **The kitty graphics probe answered OK** (pixel cockpit spike,
    /// 2026-10-21): an APC `ESC _ G i=31…;OK ESC \` reply to the probe
    /// image. Not a key a person pressed; the app feeds it to the probe's
    /// state machine. Replies to other image ids are swallowed.
    GraphicsOk,
    /// The probe answered with an error (the terminal parsed the APC but
    /// could not honour it).
    GraphicsErr,
    /// A primary device attributes reply (`CSI ? … c`), which every terminal
    /// sends for `CSI c`: the probe's "no answer is the answer" marker, and
    /// (doc 21 cut G.1) the attributes it lists, attribute `4` being sixel.
    DeviceAttributes(mush_pixel::caps::DeviceAttributes),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum EscState {
    Ground,
    /// Inside `ESC [`, accumulating parameter bytes.
    Csi(String),
    /// After `ESC O`, waiting for the final byte.
    Ss3,
    /// Inside `ESC _` (APC), accumulating the payload until `ESC \` (ST).
    /// `esc` is true after the `ESC` of a possible terminator.
    Apc {
        payload: String,
        esc: bool,
    },
}

/// The stateful byte decoder; one per TUI session.
#[derive(Clone, Debug)]
pub struct KeyParser {
    esc: EscState,
    pending: Vec<u8>,
    pending_len: usize,
    last_was_cr: bool,
}

impl Default for KeyParser {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyParser {
    /// A parser in the ground state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            esc: EscState::Ground,
            pending: Vec::new(),
            pending_len: 0,
            last_was_cr: false,
        }
    }

    /// Decode one burst of bytes into keys, in order.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Key> {
        let mut keys = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            i += 1;
            match std::mem::replace(&mut self.esc, EscState::Ground) {
                EscState::Csi(mut params) => match b {
                    0x20..=0x3f => {
                        if params.len() < 32 {
                            params.push(b as char);
                        }
                        self.esc = EscState::Csi(params);
                    }
                    0x40..=0x7e => {
                        if let Some(k) = csi_key(&params, b) {
                            keys.push(k);
                        }
                    }
                    // A control byte aborts the sequence and is dropped.
                    _ => {}
                },
                EscState::Ss3 => {
                    if let Some(k) = ss3_key(b) {
                        keys.push(k);
                    }
                }
                EscState::Apc { mut payload, esc } => {
                    if esc {
                        if b == b'\\' {
                            // ST: the sequence is whole.
                            if let Some(k) = apc_key(&payload) {
                                keys.push(k);
                            }
                        } else if i >= 2 && bytes[i - 2] == 0x1b {
                            // `ESC` followed by anything else abandons the
                            // APC; that `ESC` and this byte decode afresh.
                            i -= 2;
                        } else {
                            // The `ESC` ended the previous burst: it is an
                            // Esc, and this byte decodes afresh.
                            i -= 1;
                            keys.push(Key::Esc);
                        }
                    } else if b == 0x1b {
                        self.esc = EscState::Apc { payload, esc: true };
                    } else if b == 0x07 {
                        // BEL terminates too (some terminals end replies so).
                        if let Some(k) = apc_key(&payload) {
                            keys.push(k);
                        }
                    } else {
                        if payload.len() < APC_CAP {
                            payload.push(b as char);
                        }
                        self.esc = EscState::Apc {
                            payload,
                            esc: false,
                        };
                    }
                }
                EscState::Ground => {
                    if !self.pending.is_empty() {
                        if (0x80..0xC0).contains(&b) {
                            self.pending.push(b);
                            if self.pending.len() == self.pending_len {
                                let seq = std::mem::take(&mut self.pending);
                                if let Ok(s) = std::str::from_utf8(&seq)
                                    && let Some(c) = s.chars().next()
                                    && !c.is_control()
                                {
                                    keys.push(Key::Char(c));
                                }
                            }
                            continue;
                        }
                        // A broken sequence is dropped; this byte is decoded
                        // on its own.
                        self.pending.clear();
                    }
                    let was_cr = self.last_was_cr;
                    self.last_was_cr = false;
                    match b {
                        0x1b => match bytes.get(i) {
                            Some(b'[') => {
                                self.esc = EscState::Csi(String::new());
                                i += 1;
                            }
                            Some(b'O') => {
                                self.esc = EscState::Ss3;
                                i += 1;
                            }
                            Some(b'_') => {
                                self.esc = EscState::Apc {
                                    payload: String::new(),
                                    esc: false,
                                };
                                i += 1;
                            }
                            Some(&n) if (0x20..=0x7e).contains(&n) => {
                                keys.push(Key::Alt(n as char));
                                i += 1;
                            }
                            // Burst-final, or followed by a control byte
                            // (which is then decoded on its own).
                            _ => keys.push(Key::Esc),
                        },
                        b'\r' => {
                            self.last_was_cr = true;
                            keys.push(Key::Enter);
                        }
                        b'\n' => {
                            if !was_cr {
                                keys.push(Key::Enter);
                            }
                        }
                        0x08 | 0x7f => keys.push(Key::Backspace),
                        0x09 => keys.push(Key::Tab),
                        0x00..=0x1f => keys.push(Key::Ctrl(ctrl_letter(b))),
                        0x20..=0x7e => keys.push(Key::Char(b as char)),
                        0xc0..=0xdf => self.start_utf8(b, 2),
                        0xe0..=0xef => self.start_utf8(b, 3),
                        0xf0..=0xf7 => self.start_utf8(b, 4),
                        // A stray continuation or an invalid lead byte.
                        _ => {}
                    }
                }
            }
        }
        keys
    }

    fn start_utf8(&mut self, b: u8, len: usize) {
        self.pending.clear();
        self.pending.push(b);
        self.pending_len = len;
    }
}

/// The most payload bytes an APC keeps; a graphics reply is a few dozen.
const APC_CAP: usize = 256;

/// A whole APC payload (between `ESC _` and `ESC \`) as a key, or nothing:
/// only the kitty graphics probe's reply is one (its image id is
/// `mush_pixel::kitty::PROBE_IMAGE_ID`); every other APC is swallowed.
fn apc_key(payload: &str) -> Option<Key> {
    let keys = payload.strip_prefix('G')?;
    let probe_id = format!("i={}", mush_pixel::kitty::PROBE_IMAGE_ID);
    let names_the_probe = keys
        .split(';')
        .next()
        .is_some_and(|kv| kv.split(',').any(|kv| kv == probe_id));
    if !names_the_probe {
        return None;
    }
    Some(if mush_pixel::kitty::is_probe_ok(payload) {
        Key::GraphicsOk
    } else {
        Key::GraphicsErr
    })
}

/// `0x01` → `'a'`, … `0x1a` → `'z'`, `0x1c` → `'\\'`, `0x1d` → `']'`,
/// `0x1e` → `'^'`, `0x1f` → `'_'`, `0x00` → `'@'`.
fn ctrl_letter(b: u8) -> char {
    match b {
        0x01..=0x1a => (b'a' + b - 1) as char,
        0x1c => '\\',
        0x1d => ']',
        0x1e => '^',
        0x1f => '_',
        _ => '@',
    }
}

fn csi_key(params: &str, fin: u8) -> Option<Key> {
    let first = params.split(';').next().unwrap_or("");
    Some(match fin {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'Z' => Key::BackTab,
        // A primary device attributes reply, `CSI ? 6 2 ; … c` (a `CSI > … c`
        // is DA2 and `CSI c` alone is a request, neither of which a terminal
        // sends us unasked).
        b'c' => Key::DeviceAttributes(mush_pixel::caps::DeviceAttributes::parse(params)?),
        b'~' => match first {
            "1" | "7" => Key::Home,
            "4" | "8" => Key::End,
            "3" => Key::Delete,
            "5" => Key::PgUp,
            "6" => Key::PgDn,
            "11" => Key::F(1),
            "12" => Key::F(2),
            "13" => Key::F(3),
            "14" => Key::F(4),
            "15" => Key::F(5),
            "17" => Key::F(6),
            "18" => Key::F(7),
            "19" => Key::F(8),
            "20" => Key::F(9),
            "21" => Key::F(10),
            "23" => Key::F(11),
            "24" => Key::F(12),
            _ => return None,
        },
        _ => return None,
    })
}

fn ss3_key(fin: u8) -> Option<Key> {
    Some(match fin {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'P' => Key::F(1),
        b'Q' => Key::F(2),
        b'R' => Key::F(3),
        b'S' => Key::F(4),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(bytes: &[u8]) -> Vec<Key> {
        KeyParser::new().feed(bytes)
    }

    #[test]
    fn printable_control_and_enter() {
        assert_eq!(
            keys(b"a1\r"),
            vec![Key::Char('a'), Key::Char('1'), Key::Enter]
        );
        assert_eq!(keys(b"\r\n"), vec![Key::Enter], "CRLF is one Enter");
        assert_eq!(keys(b"\n"), vec![Key::Enter], "a bare LF still enters");
        let mut p = KeyParser::new();
        assert_eq!(p.feed(b"x\r"), vec![Key::Char('x'), Key::Enter]);
        assert_eq!(p.feed(b"\n"), vec![], "the LF of a split CRLF is swallowed");
        assert_eq!(
            keys(&[0x01, 0x0b, 0x10, 0x1d, 0x15, 0x17, 0x7f, 0x08, 0x09]),
            vec![
                Key::Ctrl('a'),
                Key::Ctrl('k'),
                Key::Ctrl('p'),
                Key::Ctrl(']'),
                Key::Ctrl('u'),
                Key::Ctrl('w'),
                Key::Backspace,
                Key::Backspace,
                Key::Tab,
            ]
        );
    }

    #[test]
    fn alt_is_esc_plus_printable_in_one_burst() {
        assert_eq!(keys(b"\x1b1"), vec![Key::Alt('1')]);
        assert_eq!(keys(b"\x1bt"), vec![Key::Alt('t')]);
        assert_eq!(keys(b"\x1b?"), vec![Key::Alt('?')]);
        // A burst-final ESC is Esc; the key arriving alone next is itself.
        let mut p = KeyParser::new();
        assert_eq!(p.feed(b"\x1b"), vec![Key::Esc]);
        assert_eq!(p.feed(b"1"), vec![Key::Char('1')]);
        // ESC before a control byte: Esc, then that key.
        assert_eq!(keys(b"\x1b\r"), vec![Key::Esc, Key::Enter]);
    }

    #[test]
    fn csi_and_ss3_sequences_including_shift_tab_and_fkeys() {
        assert_eq!(
            keys(b"\x1b[A\x1b[B\x1b[C\x1b[D\x1b[H\x1b[F\x1b[Z"),
            vec![
                Key::Up,
                Key::Down,
                Key::Right,
                Key::Left,
                Key::Home,
                Key::End,
                Key::BackTab
            ]
        );
        assert_eq!(
            keys(b"\x1b[5~\x1b[6~\x1b[3~\x1b[1~\x1b[4~"),
            vec![Key::PgUp, Key::PgDn, Key::Delete, Key::Home, Key::End]
        );
        assert_eq!(
            keys(b"\x1bOP\x1bOQ\x1b[15~\x1b[20~\x1b[1;5C"),
            vec![Key::F(1), Key::F(2), Key::F(5), Key::F(9), Key::Right]
        );
        // Shift-Tab is never mistaken for Alt+[.
        assert!(!keys(b"\x1b[Z").contains(&Key::Alt('[')));
        // A CSI split across bursts still assembles.
        let mut p = KeyParser::new();
        assert_eq!(p.feed(b"\x1b["), vec![]);
        assert_eq!(p.feed(b"D"), vec![Key::Left]);
        // Unknown sequences vanish without leaking bytes.
        assert_eq!(keys(b"\x1b[200~\x1b[?25h"), vec![]);
    }

    #[test]
    fn utf8_assembles_across_bursts_and_c1_is_dropped() {
        assert_eq!(keys("é".as_bytes()), vec![Key::Char('é')]);
        let mut p = KeyParser::new();
        assert_eq!(p.feed(&[0xC3]), vec![]);
        assert_eq!(p.feed(&[0xA9]), vec![Key::Char('é')]);
        assert_eq!(keys(&[0xC2, 0x9B]), vec![], "C1 CSI never becomes a key");
        assert_eq!(
            keys(&[0x80]),
            vec![],
            "a stray continuation byte is dropped"
        );
        assert_eq!(keys(&[0xC3, b'x']), vec![Key::Char('x')]);
    }

    /// Pixel cockpit spike (2026-10-21): the kitty graphics probe's reply
    /// and the DA1 reply decode as markers, every other APC vanishes, and a
    /// reply split across bursts still assembles.
    #[test]
    fn graphics_replies_and_da1_are_markers_and_other_apcs_vanish() {
        assert_eq!(keys(b"\x1b_Gi=31;OK\x1b\\"), vec![Key::GraphicsOk]);
        assert_eq!(
            keys(b"\x1b_Gi=31;ENOENT:unknown\x1b\\"),
            vec![Key::GraphicsErr]
        );
        // A reply for a frame image (id 1) is not the probe's.
        assert_eq!(keys(b"\x1b_Gi=1;OK\x1b\\"), vec![]);
        // Another APC altogether.
        assert_eq!(keys(b"\x1b_tmux;hello\x1b\\"), vec![]);
        // DA1 reply, with its attributes; DA2 is not it; a request is not it.
        let da1 = |p: &str| {
            Key::DeviceAttributes(mush_pixel::caps::DeviceAttributes::parse(p).unwrap_or_default())
        };
        assert_eq!(keys(b"\x1b[?62;22c"), vec![da1("?62;22")]);
        let wt = keys(b"\x1b[?61;4;6;7;14;21;22;23;24;28;32;42c");
        assert!(
            matches!(wt.as_slice(), [Key::DeviceAttributes(d)] if d.sixel() && d.level() == 61),
            "{wt:?}"
        );
        assert_eq!(keys(b"\x1b[>1;10;0c"), vec![]);
        assert_eq!(keys(b"\x1b[c"), vec![]);
        // The probe's two answers in one burst, in the order a kitty sends
        // them, then a typed key.
        assert_eq!(
            keys(b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c1"),
            vec![Key::GraphicsOk, da1("?62;"), Key::Char('1')]
        );
        // Split across bursts.
        let mut p = KeyParser::new();
        assert_eq!(p.feed(b"\x1b_Gi=3"), vec![]);
        assert_eq!(p.feed(b"1;OK\x1b"), vec![]);
        assert_eq!(p.feed(b"\\"), vec![Key::GraphicsOk]);
        // BEL ends one too, and an abandoned APC (ESC then not ST) does not
        // eat what follows.
        assert_eq!(keys(b"\x1b_Gi=31;OK\x07"), vec![Key::GraphicsOk]);
        assert_eq!(
            keys(b"\x1b_Gi=31;OK\x1b[A"),
            vec![Key::Up],
            "the abandoning ESC opens the CSI after it"
        );
        let mut p = KeyParser::new();
        assert_eq!(p.feed(b"\x1b_Gi=31;OK\x1b"), vec![]);
        assert_eq!(
            p.feed(b"x"),
            vec![Key::Esc, Key::Char('x')],
            "split: the ESC that ended the burst is an Esc"
        );
        // `ESC _` is never Alt+_.
        assert!(!keys(b"\x1b_Gi=31;OK\x1b\\").contains(&Key::Alt('_')));
    }
}
