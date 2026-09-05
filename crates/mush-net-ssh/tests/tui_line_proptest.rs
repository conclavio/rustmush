//! The shared line-editing core (interface plan I2, D-I-6): the TUI's input
//! mode and the line editor, fed the same key bytes, must agree on the
//! buffer, the cursor and every submitted line — history recall and cursor
//! edits included — because both drive one [`LineBuffer`].
//!
//! [`LineBuffer`]: mush_net_ssh::LineBuffer

use mush_net_ssh::tui::TuiApp;
use mush_net_ssh::{EditorAction, LineEditor};
use mush_proto::{ClientCaps, ClientKind, ColorDepth};
use proptest::prelude::*;

fn caps() -> ClientCaps {
    ClientCaps {
        kind: ClientKind::Ssh,
        color: ColorDepth::Ansi16,
        width: 120,
        height: 34,
        utf8: true,
        structured: false,
    }
}

/// One key both drivers understand identically. Excluded on purpose: `/`
/// on an empty box (the TUI's palette), Tab (completion vs. reserved), and
/// any Alt form (the TUI's nav actions).
fn key() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        8 => (b'a'..=b'z').prop_map(|c| vec![c]),
        3 => Just(b" ".to_vec()),
        2 => Just(b"\x1b[D".to_vec()), // left
        2 => Just(b"\x1b[C".to_vec()), // right
        1 => Just(b"\x1bOD".to_vec()), // SS3 left
        1 => Just(b"\x1b[H".to_vec()), // home
        1 => Just(b"\x1b[F".to_vec()), // end
        1 => Just(b"\x01".to_vec()),   // ^A
        1 => Just(b"\x05".to_vec()),   // ^E
        2 => Just(b"\x7f".to_vec()),   // backspace
        1 => Just(b"\x1b[3~".to_vec()), // delete
        1 => Just(b"\x15".to_vec()),   // ^U
        1 => Just(b"\x17".to_vec()),   // ^W
        1 => Just(b"\x03".to_vec()),   // ^C
        2 => Just(b"\r".to_vec()),     // enter
        2 => Just(b"\x1b[A".to_vec()), // history up
        2 => Just(b"\x1b[B".to_vec()), // history down
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn tui_input_mode_matches_the_line_editor(keys in proptest::collection::vec(key(), 0..60)) {
        let bytes: Vec<u8> = keys.concat();
        let mut ed = LineEditor::new();
        let mut echo = Vec::new();
        let ed_submits: Vec<String> = ed
            .feed(&bytes, &mut echo)
            .into_iter()
            .filter_map(|a| match a {
                EditorAction::Submit(s) => Some(s),
                EditorAction::EofQuit => None,
            })
            .collect();
        let mut app = TuiApp::new(caps());
        app.feed_input(b"\r");
        // The TUI leaves input mode on Enter with an empty box, so feed the
        // keys one at a time and reopen the box whenever that happened.
        let mut tui_submits = Vec::new();
        for k in &keys {
            if app.mode() != mush_net_ssh::tui::Mode::Input {
                app.feed_input(b"\r");
            }
            let r = app.feed_input(k);
            tui_submits.extend(r.submit);
        }
        // An empty Enter is a no-op line for the editor and a mode change
        // for the TUI; compare the non-empty submissions.
        let ed_nonempty: Vec<&String> = ed_submits.iter().filter(|s| !s.is_empty()).collect();
        let tui_nonempty: Vec<&String> = tui_submits.iter().filter(|s| !s.is_empty()).collect();
        prop_assert_eq!(tui_nonempty, ed_nonempty);
        if app.mode() == mush_net_ssh::tui::Mode::Input {
            prop_assert_eq!(app.input_text(), ed.line());
            prop_assert_eq!(app.input_cursor(), ed.cursor());
        } else {
            prop_assert!(ed.line().is_empty());
        }
    }
}
