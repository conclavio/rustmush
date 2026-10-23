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
    /// The terminal's cell size in sixel pixels (`CSI 6 ; h ; w t`), its
    /// answer to the probe's `CSI 16 t` (doc 21 cut G.2). Not a key either.
    CellSize(mush_pixel::caps::CellSize),
    /// The kitty keyboard protocol's flags (`CSI ? flags u`), the answer to
    /// the probe's `CSI ? u` (doc 21 cut G.4): the terminal speaks the
    /// protocol. Not a key.
    KeyboardFlags(u16),
    /// A cursor key held down (the kitty keyboard protocol's repeat event,
    /// only while the pixel view has pushed it).
    Repeat(Arrow),
    /// A cursor key let go (the protocol's release event): what ends a
    /// held turn.
    Release(Arrow),
    /// **The terminal's name and version** (doc 21 cut G.10): its DCS reply
    /// `ESC P >| <text> ESC \` to the probe's `XTVERSION` (`CSI > 0 q`).
    /// Not a key; the app hands it to the sounds.
    TermVersion(TermVersion),
}

/// An `XTVERSION` reply's text, cut to [`TermVersion::CAP`] bytes of
/// printable ASCII and kept inline so a [`Key`] stays `Copy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermVersion {
    bytes: [u8; TermVersion::CAP],
    len: u8,
}

impl TermVersion {
    /// The most bytes kept.
    pub const CAP: usize = 48;

    /// The reply's text, printable ASCII only, cut to [`Self::CAP`].
    #[must_use]
    pub fn new(text: &str) -> TermVersion {
        let mut bytes = [0u8; TermVersion::CAP];
        let mut len = 0usize;
        for b in text.bytes().filter(|b| (0x20..0x7f).contains(b)) {
            let Some(slot) = bytes.get_mut(len) else {
                break;
            };
            *slot = b;
            len += 1;
        }
        TermVersion {
            bytes,
            len: u8::try_from(len).unwrap_or(0),
        }
    }

    /// The text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.bytes
            .get(..usize::from(self.len))
            .and_then(|b| std::str::from_utf8(b).ok())
            .unwrap_or("")
    }
}

/// A cursor key, for the kitty keyboard protocol's repeat and release events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    /// Up.
    Up,
    /// Down.
    Down,
    /// Left.
    Left,
    /// Right.
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum EscState {
    Ground,
    /// Inside `ESC [`, accumulating parameter bytes.
    Csi(String),
    /// After `ESC O`, waiting for the final byte.
    Ss3,
    /// Inside `ESC _` (APC) or, `dcs`, `ESC P` (DCS, doc 21 cut G.10),
    /// accumulating the payload until `ESC \` (ST). `esc` is true after the
    /// `ESC` of a possible terminator.
    Apc {
        payload: String,
        esc: bool,
        dcs: bool,
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
                EscState::Apc {
                    mut payload,
                    esc,
                    dcs,
                } => {
                    let whole = |payload: &str| {
                        if dcs {
                            dcs_key(payload)
                        } else {
                            apc_key(payload)
                        }
                    };
                    if esc {
                        if b == b'\\' {
                            // ST: the sequence is whole.
                            if let Some(k) = whole(&payload) {
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
                        self.esc = EscState::Apc {
                            payload,
                            esc: true,
                            dcs,
                        };
                    } else if b == 0x07 {
                        // BEL terminates too (some terminals end replies so).
                        if let Some(k) = whole(&payload) {
                            keys.push(k);
                        }
                    } else {
                        if payload.len() < APC_CAP {
                            payload.push(b as char);
                        }
                        self.esc = EscState::Apc {
                            payload,
                            esc: false,
                            dcs,
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
                                    dcs: false,
                                };
                                i += 1;
                            }
                            // Doc 21 cut G.10: a DCS reply (`XTVERSION`'s) is
                            // swallowed as an APC is. A burst that ends on
                            // the `P` is Alt-P typed, not a reply.
                            Some(b'P') if bytes.get(i + 1).is_some() => {
                                self.esc = EscState::Apc {
                                    payload: String::new(),
                                    esc: false,
                                    dcs: true,
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

/// A whole DCS payload (between `ESC P` and `ESC \`) as a key, or nothing
/// (doc 21 cut G.10): only the `XTVERSION` reply (`>|` and the terminal's
/// name and version) is one; every other DCS is swallowed.
fn dcs_key(payload: &str) -> Option<Key> {
    let text = payload.strip_prefix(">|")?;
    Some(Key::TermVersion(TermVersion::new(text.trim())))
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
    // The kitty keyboard protocol's event type, `CSI 1 ; <mods>:<event> D`:
    // 1 press (also the default), 2 repeat, 3 release.
    let event = params
        .split(';')
        .nth(1)
        .and_then(|m| m.split(':').nth(1))
        .unwrap_or("1");
    let arrow = |a: Arrow, press: Key| match event {
        "2" => Key::Repeat(a),
        "3" => Key::Release(a),
        _ => press,
    };
    // Another key's repeat is the key again; its release is nothing.
    if event == "3" && !matches!(fin, b'A'..=b'D') {
        return None;
    }
    Some(match fin {
        b'A' => arrow(Arrow::Up, Key::Up),
        b'B' => arrow(Arrow::Down, Key::Down),
        b'C' => arrow(Arrow::Right, Key::Right),
        b'D' => arrow(Arrow::Left, Key::Left),
        b'H' => Key::Home,
        b'F' => Key::End,
        b'Z' => Key::BackTab,
        // A primary device attributes reply, `CSI ? 6 2 ; … c` (a `CSI > … c`
        // is DA2 and `CSI c` alone is a request, neither of which a terminal
        // sends us unasked).
        b'c' => Key::DeviceAttributes(mush_pixel::caps::DeviceAttributes::parse(params)?),
        // A window report: only the cell size is asked for, and only it is a
        // marker; every other `CSI … t` is swallowed.
        b't' => Key::CellSize(mush_pixel::caps::CellSize::parse(params)?),
        // The kitty keyboard protocol (doc 21 cut G.4): the flags reply, or a
        // key in its `CSI <code>[;<mods>[:<event>]] u` form.
        b'u' => match params.strip_prefix('?') {
            Some(flags) => Key::KeyboardFlags(flags.parse().ok()?),
            None => csi_u_key(params)?,
        },
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

/// A key in the kitty keyboard protocol's `CSI <code>[:<alts>][;<mods>[:<event>]] u`
/// form, which the pixel view's pushed flags (disambiguate, report event
/// types) make the terminal send for Esc and for Ctrl and Alt chords: Esc is
/// `CSI 27 u`, `^]` is `CSI 93;5 u`. Mapped to the legacy key it stands for,
/// so the view's Esc and the detach still work; a release is nothing.
fn csi_u_key(params: &str) -> Option<Key> {
    let mut fields = params.split(';');
    let code: u32 = fields.next()?.split(':').next()?.parse().ok()?;
    let mut mods_event = fields.next().unwrap_or("1").split(':');
    let mods: u32 = mods_event.next().and_then(|m| m.parse().ok()).unwrap_or(1);
    if mods_event.next() == Some("3") {
        return None;
    }
    let bits = mods.saturating_sub(1);
    let (alt, ctrl) = (bits & 2 != 0, bits & 4 != 0);
    let c = char::from_u32(code)?;
    Some(match code {
        27 => Key::Esc,
        13 => Key::Enter,
        9 => Key::Tab,
        127 | 8 => Key::Backspace,
        _ if ctrl && c.is_ascii() => Key::Ctrl(c.to_ascii_lowercase()),
        _ if alt && !c.is_control() => Key::Alt(c),
        _ if !c.is_control() => Key::Char(c),
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
        // The cell-size report, and another window report that is not it.
        assert_eq!(
            keys(b"\x1b[6;20;10t"),
            vec![Key::CellSize(mush_pixel::caps::CellSize {
                width: 10,
                height: 20
            })]
        );
        assert_eq!(keys(b"\x1b[4;480;640t"), vec![]);
        // The kitty keyboard protocol (doc 21 cut G.4): its flags reply; a
        // cursor key's press, repeat and release; Esc and the Ctrl chords in
        // the `CSI u` form, which must keep closing the view and detaching;
        // another key's release is nothing.
        assert_eq!(keys(b"\x1b[?0u"), vec![Key::KeyboardFlags(0)]);
        assert_eq!(keys(b"\x1b[?3u"), vec![Key::KeyboardFlags(3)]);
        assert_eq!(
            keys(b"\x1b[D\x1b[1;1:2D\x1b[1;1:3D\x1b[1;1:1C"),
            vec![
                Key::Left,
                Key::Repeat(Arrow::Left),
                Key::Release(Arrow::Left),
                Key::Right
            ]
        );
        assert_eq!(
            keys(b"\x1b[27u\x1b[93;5u\x1b[103;5u\x1b[120;3u\x1b[97u\x1b[97;1:3u"),
            vec![
                Key::Esc,
                Key::Ctrl(']'),
                Key::Ctrl('g'),
                Key::Alt('x'),
                Key::Char('a')
            ]
        );
        assert_eq!(keys(b"\x1b[3;1:3~"), vec![], "a Delete release");
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

    /// **Doc 21 cut G.10**: `XTVERSION`'s DCS reply decodes as the
    /// terminal's name, whole or split, ST- or BEL-ended, beside the DA1
    /// that follows it; any other DCS is swallowed; and none of it reaches
    /// the input line as F1 and typed text. A burst that ends on `ESC P` is
    /// Alt-P typed.
    #[test]
    fn a_dcs_reply_is_swallowed_and_xtversion_is_the_terminals_name() {
        let named = |k: &[Key]| match k {
            [Key::TermVersion(v), ..] => v.as_str().to_string(),
            _ => String::new(),
        };
        let w11 = keys(b"\x1bP>|w11term 0.4.1\x1b\\");
        assert_eq!(named(&w11), "w11term 0.4.1");
        assert_eq!(w11.len(), 1);
        let with_da1 = keys(b"\x1bP>|WezTerm 20240203\x1b\\\x1b[?62;22c");
        assert_eq!(named(&with_da1), "WezTerm 20240203");
        assert!(matches!(with_da1.as_slice(), [_, Key::DeviceAttributes(_)]));
        assert_eq!(named(&keys(b"\x1bP>|kitty(0.36)\x07")), "kitty(0.36)");
        let mut p = KeyParser::new();
        assert_eq!(p.feed(b"\x1bP>|w11t"), vec![]);
        assert_eq!(named(&p.feed(b"erm 1\x1b\\")), "w11term 1");
        // A DECRQSS answer, say: gone, and the key after it kept.
        assert_eq!(keys(b"\x1bP1$r0m\x1b\\a"), vec![Key::Char('a')]);
        // Never F1 and typed text.
        assert!(!keys(b"\x1bP>|w11term 1\x1b\\").contains(&Key::F(1)));
        // Alt-P alone is Alt-P.
        assert_eq!(keys(b"\x1bP"), vec![Key::Alt('P')]);
        // An over-long name is cut, never more than the cap.
        let long = format!("\x1bP>|{}\x1b\\", "x".repeat(200));
        assert_eq!(named(&keys(long.as_bytes())).len(), TermVersion::CAP);
    }
}
