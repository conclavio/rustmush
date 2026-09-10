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
                    output.push_str(std::str::from_utf8(&self.utf8[..good]).unwrap());
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
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
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
