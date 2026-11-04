//! Doc 43 cut TS.4: the TUI's [`KeyParser`] and the terminal-reply parsers in
//! `mush_pixel::caps` against arbitrary bytes.
//!
//! `KeyParser::feed` is a hand-written state machine over raw PTY bytes from
//! the internet, with index rewinds inside its APC/DCS arm; the terminal's
//! capability replies (`CSI ? … c`, `CSI 6 ; … t`) reach
//! [`DeviceAttributes::parse`] and [`CellSize::parse`] through it. A panic
//! here loses the connection's task. The properties: no input panics, no
//! input grows the parser's carried state without bound, and a printable key
//! is never a control character.

use mush_net_ssh::tui::keys::{Key, KeyParser};
use mush_pixel::caps::{CellSize, DeviceAttributes, EncoderChoice};
use proptest::prelude::*;

/// Bytes weighted toward the parser's interesting alphabet: the escape
/// introducers and terminators, parameter bytes, finals, UTF-8 lead and
/// continuation bytes, and anything at all.
fn hostile_byte() -> impl Strategy<Value = u8> {
    prop_oneof![
        4 => Just(0x1b),
        2 => prop::sample::select(b"[O_P\\?;:>|~cutABCDGHFZ".to_vec()),
        2 => b'0'..=b'9',
        1 => Just(0x07),
        1 => 0x80u8..=0xff,
        2 => any::<u8>(),
    ]
}

/// A stream of bursts, each one `feed` call.
fn bursts() -> impl Strategy<Value = Vec<Vec<u8>>> {
    prop::collection::vec(prop::collection::vec(hostile_byte(), 0..96), 0..12)
}

/// Feed every burst; check each key; return the parser for state checks.
fn drive(bursts: &[Vec<u8>]) -> KeyParser {
    let mut parser = KeyParser::new();
    for burst in bursts {
        for key in parser.feed(burst) {
            if let Key::Char(c) | Key::Alt(c) = key {
                assert!(!c.is_control(), "a printable key carried {c:?}");
            }
        }
    }
    parser
}

/// The parser's carried state, as its `Debug` shows it, is bounded: an
/// unterminated CSI keeps 32 parameter bytes and an unterminated APC or DCS
/// 256 payload bytes, however much follows.
const STATE_DEBUG_CAP: usize = 4096;

#[test]
fn unterminated_sequences_keep_bounded_state() {
    for open in [&b"\x1b["[..], b"\x1b_", b"\x1bP>|"] {
        let mut parser = KeyParser::new();
        parser.feed(open);
        for _ in 0..64 {
            parser.feed(&[b'1'; 4096]);
        }
        let carried = format!("{parser:?}").len();
        assert!(
            carried < STATE_DEBUG_CAP,
            "{open:?} then 256 KiB carried {carried} bytes of state"
        );
    }
}

#[test]
fn capability_replies_with_extreme_numbers_parse_or_refuse() {
    for params in [
        "?",
        "?;",
        "?99999999999999999999;4",
        "?62;63;64;4294967295;18446744073709551616;-1;4",
        "?62;;;;4",
        "?\u{0}",
    ] {
        let _ = DeviceAttributes::parse(params);
    }
    assert!(DeviceAttributes::parse("?62;4").is_some_and(|d| d.sixel()));
    for params in [
        "6",
        "6;",
        "6;0;10",
        "6;65536;10",
        "6;20;10;1",
        "6;20;-10",
        "6;;",
    ] {
        assert_eq!(CellSize::parse(params), None, "{params:?}");
    }
    assert_eq!(
        CellSize::parse("6;20;10"),
        Some(CellSize {
            width: 10,
            height: 20
        })
    );
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    /// Arbitrary bursts never panic the parser, its keys are well formed and
    /// its carried state stays bounded.
    #[test]
    fn arbitrary_bursts_never_panic(bursts in bursts()) {
        let parser = drive(&bursts);
        let carried = format!("{parser:?}").len();
        prop_assert!(carried < STATE_DEBUG_CAP);
    }

    /// Fully arbitrary bytes, split anywhere.
    #[test]
    fn arbitrary_bytes_never_panic(
        bytes in prop::collection::vec(any::<u8>(), 0..2048),
        splits in prop::collection::vec(any::<prop::sample::Index>(), 0..8),
    ) {
        let mut cuts: Vec<usize> = splits.iter().map(|s| s.index(bytes.len() + 1)).collect();
        cuts.push(0);
        cuts.push(bytes.len());
        cuts.sort_unstable();
        let bursts: Vec<Vec<u8>> = cuts.windows(2).map(|w| bytes[w[0]..w[1]].to_vec()).collect();
        drive(&bursts);
    }

    /// Arbitrary capability replies wrapped as the terminal sends them reach
    /// the caps parsers through the key parser without a panic.
    #[test]
    fn arbitrary_capability_replies_never_panic(params in "[0-9;?:>-]{0,48}", tail in prop::collection::vec(any::<u8>(), 0..16)) {
        for fin in *b"ctu~" {
            let mut reply = b"\x1b[".to_vec();
            reply.extend_from_slice(params.as_bytes());
            reply.push(fin);
            reply.extend_from_slice(&tail);
            drive(&[reply]);
        }
    }

    /// The caps parsers take any text at all.
    #[test]
    fn caps_parsers_take_any_text(s in any::<String>()) {
        let _ = DeviceAttributes::parse(&s);
        let _ = DeviceAttributes::parse(&format!("?{s}"));
        let _ = CellSize::parse(&s);
        let _ = CellSize::parse(&format!("6;{s}"));
        let _ = EncoderChoice::parse(&s);
    }
}
