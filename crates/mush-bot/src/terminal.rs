//! Incremental terminal-text decoder. Escape/control bytes are never forwarded
//! to the runner's terminal (including OSC clipboard/title sequences).
#[derive(Default)]
pub struct Decoder {
    escape: Escape,
    utf8: Vec<u8>,
}

#[derive(Default)]
enum Escape {
    #[default]
    Text,
    Esc,
    Csi,
    String,
    StringEsc,
}

fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

fn is_c1_control(c: char) -> bool {
    ('\u{0080}'..='\u{009f}').contains(&c)
}

fn allowed_terminal_char(c: char) -> bool {
    matches!(c, '\n' | '\t') || (!c.is_control() && !is_c1_control(c) && !is_bidi_control(c))
}

impl Decoder {
    pub fn feed(&mut self, bytes: &[u8]) -> String {
        for &b in bytes {
            self.escape = match self.escape {
                Escape::Text => match b {
                    0x1b => Escape::Esc,
                    b'\n' | b'\t' | 0x20..=0x7e | 0x80..=0xff => {
                        self.utf8.push(b);
                        Escape::Text
                    }
                    _ => Escape::Text,
                },
                Escape::Esc => match b {
                    b'[' => Escape::Csi,
                    b']' | b'P' | b'_' | b'^' => Escape::String,
                    _ => Escape::Text,
                },
                Escape::Csi => {
                    if (0x40..=0x7e).contains(&b) {
                        Escape::Text
                    } else {
                        Escape::Csi
                    }
                }
                Escape::String => match b {
                    7 => Escape::Text,
                    0x1b => Escape::StringEsc,
                    _ => Escape::String,
                },
                Escape::StringEsc => {
                    if b == b'\\' {
                        Escape::Text
                    } else {
                        Escape::String
                    }
                }
            };
        }
        let mut output = String::new();
        loop {
            match std::str::from_utf8(&self.utf8) {
                Ok(s) => {
                    output.push_str(s);
                    self.utf8.clear();
                    break;
                }
                Err(e) => {
                    let good = e.valid_up_to();
                    if let Ok(prefix) = std::str::from_utf8(&self.utf8[..good]) {
                        output.push_str(prefix);
                    }
                    self.utf8.drain(..good);
                    if let Some(n) = e.error_len() {
                        output.push('\u{fffd}');
                        self.utf8.drain(..n);
                    } else {
                        break;
                    }
                }
            }
        }
        output
            .chars()
            .filter(|&c| allowed_terminal_char(c))
            .collect()
    }
}

pub fn plain(text: &str) -> String {
    Decoder::default().feed(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c1_controls_and_bidi_overrides_are_stripped() {
        let sample = concat!(
            "ok",
            "\u{0080}",
            "\u{009b}31m",
            "\u{009d}52;c;YQ==\u{009c}",
            "\u{009f}",
            "\u{061c}",
            "\u{200e}",
            "\u{200f}",
            "\u{202a}",
            "\u{202b}",
            "\u{202c}",
            "\u{202d}",
            "\u{202e}",
            "\u{2066}",
            "\u{2067}",
            "\u{2068}",
            "\u{2069}",
            "end\t\n"
        );
        assert_eq!(plain(sample), "ok31m52;c;YQ==end\t\n");
        let mut d = Decoder::default();
        assert_eq!(d.feed(b"a\xc2"), "a");
        assert_eq!(d.feed(b"\x9bb\xe2\x80"), "b");
        assert_eq!(d.feed(b"\xaec"), "c");
    }

    #[test]
    fn fragmented_utf8_csi_and_osc_never_reach_terminal() {
        let mut d = Decoder::default();
        assert_eq!(d.feed(b"\x1b[3"), "");
        assert_eq!(d.feed(b"1mR\xc3"), "R");
        assert_eq!(d.feed(b"\xa9\x1b]52;clipboard"), "é");
        assert_eq!(d.feed(b"\x1b"), "");
        assert_eq!(d.feed(b"\\rose\r\n\x1b[0m"), "rose\n");
        assert_eq!(d.feed(b"\xffok"), "�ok");
    }
}
