//! Fuzz-style property tests for [`LineEditor::feed`] (M9 hardening).
//!
//! The line editor parses attacker-controlled channel bytes (an SSH client can
//! send *anything* down a PTY: split UTF-8, CSI floods, NUL, overlong
//! sequences, megabytes of one character). It must never panic, must respect
//! its 8 KiB character cap, must only ever echo well-formed UTF-8, and must
//! never surface a control character inside a submitted line (the ANSI-/
//! newline-injection vector). These properties are asserted here over hundreds
//! of random byte streams; the hand-written cases in `editor.rs` cover the
//! specific behaviours.

use mush_net_ssh::{EchoMode, EditorAction, LineEditor, MAX_LINE_CHARS};
use proptest::prelude::*;

/// Feed `bytes` to a fresh editor, split at the given chunk boundaries, and
/// return `(submitted lines, all echo bytes)`.
fn drive(bytes: &[u8], chunks: &[usize], mode: EchoMode) -> (Vec<String>, Vec<u8>) {
    let mut ed = LineEditor::new();
    ed.set_mode(mode);
    let mut echo = Vec::new();
    let mut lines = Vec::new();
    let mut pos = 0;
    // Walk the requested split points (already sorted, in-range), then a final
    // chunk to the end.
    let mut bounds: Vec<usize> = chunks.to_vec();
    bounds.push(bytes.len());
    for &b in &bounds {
        let end = b.min(bytes.len()).max(pos);
        let mut out = Vec::new();
        for action in ed.feed(&bytes[pos..end], &mut out) {
            match action {
                EditorAction::Submit(s) => lines.push(s),
                EditorAction::EofQuit => {}
            }
        }
        echo.extend_from_slice(&out);
        pos = end;
    }
    (lines, echo)
}

/// Every invariant the editor must uphold, for one run.
fn assert_invariants(bytes: &[u8], chunks: &[usize], mode: EchoMode) {
    let (lines, echo) = drive(bytes, chunks, mode);

    // 1. Echo is always well-formed UTF-8 — we never splice raw input bytes
    //    back out; printable chars are re-encoded, control/escape bytes become
    //    a bell or a fixed ASCII sequence.
    assert!(
        std::str::from_utf8(&echo).is_ok(),
        "echo bytes were not valid UTF-8: {echo:?}"
    );

    for line in &lines {
        // 2. A submitted line NEVER contains a control character (C0, DEL, C1,
        //    ESC, CR, LF, or tab). This is the injection boundary: whatever a
        //    client pastes, the parsed command line is clean.
        assert!(
            !line.chars().any(char::is_control),
            "submitted line carried a control char: {line:?}"
        );
        // 3. The 8 KiB character cap holds.
        assert!(
            line.chars().count() <= MAX_LINE_CHARS,
            "submitted line exceeded MAX_LINE_CHARS ({} chars)",
            line.chars().count()
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Arbitrary byte streams, fed in arbitrary chunks, in every echo mode.
    #[test]
    fn feed_never_panics_and_keeps_invariants(
        bytes in proptest::collection::vec(any::<u8>(), 0..2048),
        splits in proptest::collection::vec(any::<usize>(), 0..8),
        mode in prop_oneof![
            Just(EchoMode::Normal),
            Just(EchoMode::Masked),
            Just(EchoMode::NoEcho),
        ],
    ) {
        let mut chunks: Vec<usize> = splits.into_iter().map(|s| s % (bytes.len() + 1)).collect();
        chunks.sort_unstable();
        assert_invariants(&bytes, &chunks, mode);
    }

    /// Streams biased toward "interesting" bytes: ESC, CSI finals, CR/LF, NUL,
    /// UTF-8 lead/continuation bytes, backspace, the control keys the editor
    /// binds. These trigger far more of the state machine than uniform bytes.
    #[test]
    fn feed_hostile_alphabet_keeps_invariants(
        bytes in proptest::collection::vec(
            prop_oneof![
                Just(0x1bu8),  // ESC
                Just(b'['), Just(b'O'), Just(b';'), Just(b'~'),
                Just(b'A'), Just(b'B'), Just(b'C'), Just(b'D'), Just(b'H'), Just(b'F'),
                Just(b'\r'), Just(b'\n'), Just(0x00), Just(0x7f), Just(0x08),
                Just(0x01), Just(0x03), Just(0x04), Just(0x05), Just(0x15), Just(0x17),
                Just(0x09), Just(0xc3), Just(0xa9), Just(0xf0), Just(0x9f), Just(0x80),
                (0x20u8..0x7f),
            ],
            0..1024,
        ),
        splits in proptest::collection::vec(any::<usize>(), 0..12),
    ) {
        let mut chunks: Vec<usize> = splits.into_iter().map(|s| s % (bytes.len() + 1)).collect();
        chunks.sort_unstable();
        assert_invariants(&bytes, &chunks, EchoMode::Normal);
    }

    /// Chunk boundaries must not change the parse: `feed` is defined as a
    /// per-byte fold, so splitting a stream anywhere yields the same submitted
    /// lines and the same echo bytes as feeding it whole. This guards the
    /// carried state (pending UTF-8 bytes, escape state, CRLF swallow) across
    /// `feed` calls.
    #[test]
    fn chunking_is_transparent(
        bytes in proptest::collection::vec(any::<u8>(), 0..1024),
        splits in proptest::collection::vec(any::<usize>(), 0..10),
    ) {
        let mut chunks: Vec<usize> = splits.into_iter().map(|s| s % (bytes.len() + 1)).collect();
        chunks.sort_unstable();
        let whole = drive(&bytes, &[], EchoMode::Normal);
        let split = drive(&bytes, &chunks, EchoMode::Normal);
        prop_assert_eq!(whole, split);
    }

    /// The character cap holds no matter how much is thrown at it, and a
    /// submit right after the flood returns a line at exactly the cap.
    #[test]
    fn cap_holds_under_flood(extra in 0usize..2048) {
        let mut ed = LineEditor::new();
        let mut out = Vec::new();
        let flood = vec![b'a'; MAX_LINE_CHARS + extra];
        ed.feed(&flood, &mut out);
        prop_assert!(std::str::from_utf8(&out).is_ok());
        let mut out2 = Vec::new();
        let actions = ed.feed(b"\r", &mut out2);
        prop_assert_eq!(actions.len(), 1);
        if let EditorAction::Submit(s) = &actions[0] {
            prop_assert_eq!(s.chars().count(), MAX_LINE_CHARS);
        } else {
            prop_assert!(false, "expected Submit");
        }
    }
}
