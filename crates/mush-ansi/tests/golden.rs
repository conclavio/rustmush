//! Golden byte-level renderer tests: representative events at every color
//! depth, wrap correctness with embedded escapes, downgrade spot checks,
//! and the CRLF-only invariant across all variants.

use mush_ansi::{AnsiTheme, ansi_len, render};
use mush_proto::{
    ActionStatusId, ClientCaps, ClientKind, Color, ColorDepth, ExitRef, InputMode, NarrativeKind,
    ObjRef, OutputEvent, PanelFrameKind, ProjectedActionReceipt, ProjectedActionResult, PromptSpec,
    RichText, RoomView, Span, Style, WhoEntry,
};

fn caps(color: ColorDepth, width: u16) -> ClientCaps {
    ClientCaps {
        kind: ClientKind::Ssh,
        color,
        width,
        height: 24,
        utf8: true,
        structured: false,
    }
}

fn rendered(event: &OutputEvent, color: ColorDepth, width: u16) -> Vec<u8> {
    let mut out = Vec::new();
    render(event, &caps(color, width), &AnsiTheme::default(), &mut out);
    out
}

fn rendered_str(event: &OutputEvent, color: ColorDepth, width: u16) -> String {
    String::from_utf8(rendered(event, color, width)).unwrap()
}

fn nick() -> ObjRef {
    ObjRef::new("#5", "Nick")
}

fn say(text: &str) -> OutputEvent {
    OutputEvent::Narrative {
        kind: NarrativeKind::Say,
        actor: Some(nick()),
        text: RichText::plain(text),
    }
}

fn action_receipt_event() -> OutputEvent {
    let status =
        ActionStatusId::new("k5:toy:brace:0000000000000007:01010101010101010101010101010101")
            .unwrap();
    let result = ProjectedActionResult::new(
        "brace",
        "Brace committed.",
        serde_json::json!({"event": "braced", "focus": 2}),
    )
    .unwrap();
    OutputEvent::View(
        ProjectedActionReceipt::new(status, "02020202020202020202020202020202", Some(result))
            .unwrap()
            .view()
            .unwrap(),
    )
}

#[test]
fn action_receipt_has_a_line_fallback_with_recovery_reference() {
    assert_eq!(
        rendered_str(&action_receipt_event(), ColorDepth::None, 0),
        "Brace committed. Transaction: k5:toy:brace:0000000000000007:01010101010101010101010101010101\r\n"
    );
    let OutputEvent::View(view) = action_receipt_event() else {
        unreachable!()
    };
    let forged = OutputEvent::View(
        view.with_commit("03030303030303030303030303030303")
            .unwrap(),
    );
    assert!(rendered(&forged, ColorDepth::None, 0).is_empty());
}

// -- narrative goldens -------------------------------------------------------

#[test]
fn say_golden_ansi16_and_none() {
    let ev = say("hello");
    assert_eq!(
        rendered(&ev, ColorDepth::Ansi16, 0),
        b"\x1b[36mNick\x1b[0m says, \"hello\"\r\n"
    );
    assert_eq!(
        rendered(&ev, ColorDepth::None, 0),
        b"Nick says, \"hello\"\r\n"
    );
}

#[test]
fn narrative_kind_goldens() {
    let mk = |kind, text: &str| OutputEvent::Narrative {
        kind,
        actor: Some(nick()),
        text: RichText::plain(text),
    };
    let d = ColorDepth::None;
    assert_eq!(
        rendered_str(&mk(NarrativeKind::Pose, "grins."), d, 0),
        "Nick grins.\r\n"
    );
    assert_eq!(
        rendered_str(&mk(NarrativeKind::SemiPose, "'s eyes narrow."), d, 0),
        "Nick's eyes narrow.\r\n"
    );
    assert_eq!(
        rendered_str(&mk(NarrativeKind::Emit, "A cold wind blows."), d, 0),
        "A cold wind blows.\r\n"
    );
    assert_eq!(
        rendered_str(&mk(NarrativeKind::Spoof, "The walls whisper."), d, 0),
        "The walls whisper.\r\n"
    );
    assert_eq!(
        rendered_str(&mk(NarrativeKind::Page, "hi there"), d, 0),
        "Nick pages: hi there\r\n"
    );
    assert_eq!(
        rendered_str(&mk(NarrativeKind::Whisper, "psst"), d, 0),
        "Nick whispers, \"psst\"\r\n"
    );
    assert_eq!(
        rendered_str(&mk(NarrativeKind::Ooc, "brb"), d, 0),
        "<OOC> Nick says, \"brb\"\r\n"
    );
    // Actorless lines fall back sensibly.
    let anon = OutputEvent::Narrative {
        kind: NarrativeKind::Say,
        actor: None,
        text: RichText::plain("boo"),
    };
    assert_eq!(rendered_str(&anon, d, 0), "Someone says, \"boo\"\r\n");
}

// -- room golden -------------------------------------------------------------

#[test]
fn room_golden_ansi16() {
    let ev = OutputEvent::Room(RoomView {
        name: "Hall".into(),
        id: "#2".into(),
        desc: RichText::plain("A big hall."),
        contents: vec![ObjRef::new("#7", "ball")],
        exits: vec![
            ExitRef {
                name: "north".into(),
                aliases: vec!["n".into()],
            },
            ExitRef {
                name: "south".into(),
                aliases: vec![],
            },
        ],
        zone: None,
    });
    let expected = "\x1b[1mHall(#2)\x1b[0m\r\n\
                    A big hall.\r\n\
                    Contents:\r\n\
                    ball\r\n\
                    Obvious exits:\r\n\
                    \x1b[1;36mnorth\x1b[0m  \x1b[1;36msouth\x1b[0m\r\n";
    assert_eq!(rendered_str(&ev, ColorDepth::Ansi16, 0), expected);
    // Empty contents/exits: the label lines vanish.
    let bare = OutputEvent::Room(RoomView {
        name: "Void".into(),
        id: "#9".into(),
        desc: RichText::plain("Nothing."),
        contents: vec![],
        exits: vec![],
        zone: None,
    });
    assert_eq!(
        rendered_str(&bare, ColorDepth::None, 0),
        "Void(#9)\r\nNothing.\r\n"
    );
}

// -- channel goldens ---------------------------------------------------------

fn channel(kind: NarrativeKind, title: Option<&str>, text: &str) -> OutputEvent {
    OutputEvent::Channel {
        channel: "pub".into(),
        seq: 1,
        actor: nick(),
        title: title.map(str::to_string),
        kind,
        text: RichText::plain(text),
        ts: 0,
    }
}

#[test]
fn channel_goldens() {
    assert_eq!(
        rendered(
            &channel(NarrativeKind::Say, None, "hello"),
            ColorDepth::Ansi16,
            0
        ),
        b"\x1b[33m[pub]\x1b[0m \x1b[36mNick\x1b[0m: hello\r\n"
    );
    assert_eq!(
        rendered_str(
            &channel(NarrativeKind::Pose, Some("Duke"), "waves."),
            ColorDepth::None,
            0
        ),
        "[pub] Duke Nick waves.\r\n"
    );
    assert_eq!(
        rendered_str(
            &channel(NarrativeKind::SemiPose, None, "'s here."),
            ColorDepth::None,
            0
        ),
        "[pub] Nick's here.\r\n"
    );
    assert_eq!(
        rendered_str(
            &channel(NarrativeKind::Emit, None, "The channel hums."),
            ColorDepth::None,
            0
        ),
        "[pub] The channel hums.\r\n"
    );
}

// -- downgrade through the renderer ------------------------------------------

#[test]
fn rgb_downgrades_per_depth() {
    let ev = OutputEvent::Sys(
        RichText::new().styled(Style::default().with_fg(Color::Rgb(255, 0, 0)), "X"),
    );
    assert_eq!(
        rendered(&ev, ColorDepth::TrueColor, 0),
        b"\x1b[38;2;255;0;0mX\x1b[0m\r\n"
    );
    assert_eq!(
        rendered(&ev, ColorDepth::Ansi256, 0),
        b"\x1b[38;5;196mX\x1b[0m\r\n"
    );
    assert_eq!(
        rendered(&ev, ColorDepth::Ansi16, 0),
        b"\x1b[91mX\x1b[0m\r\n"
    );
    assert_eq!(rendered(&ev, ColorDepth::None, 0), b"X\r\n");
}

#[test]
fn explicit_style_wins_over_theme_per_field() {
    // Tag says red; explicit fg green wins.
    let ev = OutputEvent::Sys(RichText(vec![
        Span::tagged("error", "ok").with_style(Style::default().with_fg(Color::Ansi(2))),
    ]));
    assert_eq!(
        rendered(&ev, ColorDepth::Ansi16, 0),
        b"\x1b[32mok\x1b[0m\r\n"
    );
    // Theme bold (exit) merges with an explicit fg.
    let ev2 = OutputEvent::Sys(RichText(vec![
        Span::tagged("exit", "door").with_style(Style::default().with_fg(Color::Ansi(7))),
    ]));
    assert_eq!(
        rendered(&ev2, ColorDepth::Ansi16, 0),
        b"\x1b[1;37mdoor\x1b[0m\r\n"
    );
}

#[test]
fn theme_overrides_apply() {
    let mut theme = AnsiTheme::default();
    theme.set(
        "error",
        Style::default().with_fg(Color::Ansi(3)).with_bold(),
    );
    let ev = OutputEvent::Sys(RichText::tag("error", "nope"));
    let mut out = Vec::new();
    render(&ev, &caps(ColorDepth::Ansi16, 0), &theme, &mut out);
    assert_eq!(out, b"\x1b[1;33mnope\x1b[0m\r\n");
}

// -- WHO ---------------------------------------------------------------------

fn who_entries() -> Vec<WhoEntry> {
    vec![
        WhoEntry {
            name: "Nick".into(),
            alias: None,
            idle_s: 5,
            conn_s: 3600,
            location: None,
            doing: Some("Building".into()),
        },
        WhoEntry {
            name: "Wizardess".into(),
            alias: Some("Wiz".into()),
            idle_s: 120,
            conn_s: 90_000,
            location: None,
            doing: None,
        },
    ]
}

#[test]
fn who_golden_width_78() {
    let ev = OutputEvent::Who(who_entries());
    let text = rendered_str(&ev, ColorDepth::None, 78);
    let lines: Vec<&str> = text.split("\r\n").collect();
    // Aligned columns at fixed widths 16/5/8 + two-space gutters.
    assert_eq!(
        lines[0],
        format!("{:<16}  {:>5}  {:>8}  {}", "Name", "Idle", "Conn", "Doing")
    );
    assert_eq!(
        lines[1],
        format!("{:<16}  {:>5}  {:>8}  {}", "Nick", "5s", "1h", "Building")
    );
    // Empty Doing trims the trailing padding.
    assert_eq!(
        lines[2],
        format!("{:<16}  {:>5}  {:>8}", "Wizardess", "2m", "1d")
    );
    assert_eq!(lines[3], "2 players connected.");
    assert_eq!(lines[4], "");
    assert_eq!(lines.len(), 5);
}

#[test]
fn who_fits_narrow_terminals() {
    let ev = OutputEvent::Who(who_entries());
    let text = rendered_str(&ev, ColorDepth::None, 30);
    for line in text.split("\r\n") {
        assert!(ansi_len(line) <= 30, "WHO line exceeds width: {line:?}");
    }
    // Long names truncate inside the name column instead of shifting it.
    let long = OutputEvent::Who(vec![WhoEntry {
        name: "Extraordinarily-Long-Name".into(),
        alias: None,
        idle_s: 1,
        conn_s: 1,
        location: None,
        doing: None,
    }]);
    let t = rendered_str(&long, ColorDepth::None, 78);
    assert!(t.contains("Extraordinarily-"));
    assert!(!t.contains("Extraordinarily-Long-Name"));
}

// -- wrapping ----------------------------------------------------------------

#[test]
fn wrap_never_splits_escapes_and_reopens_style() {
    let ev = OutputEvent::Sys(RichText::tag(
        "error",
        "alpha beta gamma delta epsilon zeta eta theta",
    ));
    let text = rendered_str(&ev, ColorDepth::Ansi16, 20);
    let lines: Vec<&str> = text.strip_suffix("\r\n").unwrap().split("\r\n").collect();
    assert!(lines.len() >= 2, "expected wrapping: {lines:?}");
    for (i, line) in lines.iter().enumerate() {
        assert!(ansi_len(line) <= 20, "line too wide: {line:?}");
        // Every ESC begins a complete CSI sequence ending in 'm' within the line.
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                assert_eq!(chars.next(), Some('['), "non-CSI escape in {line:?}");
                let mut terminated = false;
                for c2 in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c2) {
                        assert_eq!(c2, 'm');
                        terminated = true;
                        break;
                    }
                }
                assert!(terminated, "escape split across lines: {line:?}");
            }
        }
        // The style is reopened on continuation lines and reset before CRLF.
        assert!(
            line.starts_with("\x1b[31m"),
            "line {i} lost its style: {line:?}"
        );
        assert!(line.ends_with("\x1b[0m"), "line {i} leaks style: {line:?}");
    }
    // No color depth: same text, no escapes at all.
    let plain = rendered_str(&ev, ColorDepth::None, 20);
    assert!(!plain.contains('\u{1b}'));
    assert_eq!(
        plain,
        "alpha beta gamma\r\ndelta epsilon zeta\r\neta theta\r\n"
    );
}

#[test]
fn width_one_and_long_words() {
    let ev = OutputEvent::Sys(RichText::plain("ab cd"));
    assert_eq!(rendered_str(&ev, ColorDepth::None, 1), "ab\r\ncd\r\n");
    let long = OutputEvent::Sys(RichText::plain("aa supercalifragilistic bb"));
    assert_eq!(
        rendered_str(&long, ColorDepth::None, 10),
        "aa\r\nsupercalifragilistic\r\nbb\r\n"
    );
}

// -- remaining variants ------------------------------------------------------

#[test]
fn mail_notice_golden() {
    let ev = OutputEvent::MailNotice {
        from: ObjRef::new("#1", "Wizard"),
        subject: "Greetings".into(),
        id: 3,
        unread: 4,
    };
    assert_eq!(
        rendered_str(&ev, ColorDepth::None, 0),
        "MAIL: New message (3) from Wizard: Greetings (4 unread)\r\n"
    );
    assert!(rendered_str(&ev, ColorDepth::Ansi16, 0).starts_with("\x1b[32m"));
}

#[test]
fn weather_sys_disconnect_golden() {
    let weather = OutputEvent::Weather {
        zone: "downtown".into(),
        text: RichText::plain("Rain begins to fall."),
        icon: "rain".into(),
    };
    assert_eq!(
        rendered_str(&weather, ColorDepth::None, 0),
        "Rain begins to fall.\r\n"
    );
    let sys = OutputEvent::Sys(RichText::tag("system", "Saved."));
    assert_eq!(
        rendered(&sys, ColorDepth::Ansi16, 0),
        b"\x1b[32mSaved.\x1b[0m\r\n"
    );
    let bye = OutputEvent::Disconnect {
        reason: RichText::plain("Goodbye."),
    };
    assert_eq!(rendered_str(&bye, ColorDepth::None, 0), "Goodbye.\r\n");
}

#[test]
fn hello_session_info_reply_golden() {
    let hello = OutputEvent::Hello {
        proto: 1,
        server: "rustmush 0.1".into(),
        features: vec!["channels".into()],
        panels: vec![],
    };
    assert_eq!(
        rendered_str(&hello, ColorDepth::None, 0),
        "rustmush 0.1 (proto 1)\r\n"
    );
    let info = OutputEvent::SessionInfo {
        character: nick(),
        account: "nick".into(),
        connections: 2,
    };
    assert_eq!(
        rendered_str(&info, ColorDepth::None, 0),
        "Connected as Nick (account nick, 2 connections).\r\n"
    );
    let one = OutputEvent::SessionInfo {
        character: nick(),
        account: "nick".into(),
        connections: 1,
    };
    assert_eq!(
        rendered_str(&one, ColorDepth::None, 0),
        "Connected as Nick (account nick, 1 connection).\r\n"
    );
    // Reply unwraps to its payload.
    let reply = OutputEvent::Reply {
        id: 42,
        event: Box::new(say("hi")),
    };
    assert_eq!(
        rendered_str(&reply, ColorDepth::None, 0),
        "Nick says, \"hi\"\r\n"
    );
}

#[test]
fn prompt_ping_panel_golden() {
    let prompt = OutputEvent::Prompt(PromptSpec {
        mode: InputMode::Normal,
        text: Some(RichText::plain("> ")),
    });
    // Prompts leave the cursor on the line: no CRLF.
    assert_eq!(rendered(&prompt, ColorDepth::None, 0), b"> ");
    let silent_prompt = OutputEvent::Prompt(PromptSpec {
        mode: InputMode::Masked,
        text: None,
    });
    assert_eq!(rendered(&silent_prompt, ColorDepth::None, 0), b"");
    assert_eq!(
        rendered(&OutputEvent::Ping { nonce: 7 }, ColorDepth::Ansi16, 0),
        b""
    );
    let panel = OutputEvent::Panel {
        feed: "wod.roll".into(),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::json!({"dice": 5}),
    };
    assert_eq!(rendered(&panel, ColorDepth::Ansi16, 0), b"");
}

// -- invariants --------------------------------------------------------------

fn all_variant_events() -> Vec<OutputEvent> {
    vec![
        OutputEvent::Room(RoomView {
            name: "Hall".into(),
            id: "#2".into(),
            desc: RichText::plain("A big hall with a very long description that wraps."),
            contents: vec![ObjRef::new("#7", "ball")],
            exits: vec![ExitRef {
                name: "north".into(),
                aliases: vec![],
            }],
            zone: None,
        }),
        say("a somewhat longer line of speech to exercise the wrapper"),
        OutputEvent::Sys(RichText::tag("error", "Huh?  (Type \"help\" for help.)")),
        channel(NarrativeKind::Say, Some("Duke"), "hello there"),
        OutputEvent::MailNotice {
            from: nick(),
            subject: "hi".into(),
            id: 1,
            unread: 1,
        },
        OutputEvent::Who(who_entries()),
        OutputEvent::Panel {
            feed: "wod.roll".into(),
            kind: PanelFrameKind::Delta,
            data: serde_json::json!({}),
        },
        OutputEvent::Weather {
            zone: "z".into(),
            text: RichText::plain("Rain."),
            icon: "rain".into(),
        },
        OutputEvent::Reply {
            id: 9,
            event: Box::new(say("nested")),
        },
        OutputEvent::Prompt(PromptSpec {
            mode: InputMode::Normal,
            text: Some(RichText::plain("> ")),
        }),
        OutputEvent::Ping { nonce: 1 },
        OutputEvent::Hello {
            proto: 1,
            server: "rustmush 0.1".into(),
            features: vec![],
            panels: vec![],
        },
        OutputEvent::SessionInfo {
            character: nick(),
            account: "nick".into(),
            connections: 1,
        },
        OutputEvent::Disconnect {
            reason: RichText::plain("Goodbye."),
        },
    ]
}

#[test]
fn crlf_only_invariant_across_all_variants() {
    let depths = [
        ColorDepth::None,
        ColorDepth::Ansi16,
        ColorDepth::Ansi256,
        ColorDepth::TrueColor,
    ];
    for ev in all_variant_events() {
        for depth in depths {
            for width in [0u16, 1, 10, 80] {
                let out = rendered(&ev, depth, width);
                for (i, &b) in out.iter().enumerate() {
                    if b == b'\n' {
                        assert!(
                            i > 0 && out[i - 1] == b'\r',
                            "bare \\n at {i} in {ev:?} depth {depth:?} width {width}"
                        );
                    }
                    if b == b'\r' {
                        assert_eq!(
                            out.get(i + 1),
                            Some(&b'\n'),
                            "bare \\r at {i} in {ev:?} depth {depth:?} width {width}"
                        );
                    }
                    // Only ESC/CR/LF may appear below 0x20.
                    assert!(
                        b >= 0x20 || b == 0x1b || b == b'\r' || b == b'\n',
                        "control byte {b:#04x} in {ev:?} depth {depth:?} width {width}"
                    );
                }
                if matches!(depth, ColorDepth::None) {
                    assert!(
                        !out.contains(&0x1b),
                        "escape at ColorDepth::None in {ev:?} width {width}"
                    );
                }
            }
        }
    }
}

#[test]
fn embedded_newlines_and_control_injection() {
    // A span with an embedded \n becomes two CRLF lines.
    let ev = OutputEvent::Sys(RichText::plain("line1\nline2"));
    assert_eq!(rendered_str(&ev, ColorDepth::None, 0), "line1\r\nline2\r\n");
    // Raw ESC smuggled into span text is stripped, never forwarded.
    let evil = OutputEvent::Sys(RichText::plain("bad\x1b[31mesc"));
    let out = rendered(&evil, ColorDepth::TrueColor, 0);
    assert!(!out.contains(&0x1b));
    assert_eq!(out, b"bad[31mesc\r\n");
}
