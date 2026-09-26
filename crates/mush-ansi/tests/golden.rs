//! Golden byte-level renderer tests: representative events at every color
//! depth, wrap correctness with embedded escapes, downgrade spot checks,
//! and the CRLF-only invariant across all variants.

use mush_ansi::{AnsiTheme, ansi_len, render};
use mush_proto::{
    ActionStatusId, ClientCaps, ClientKind, Color, ColorDepth, ExitRef, InputMode, NarrativeKind,
    NoticeAction, NoticeSeverity, ObjRef, OutputEvent, PanelFrameKind, ProjectedActionReceipt,
    ProjectedActionResult, PromptSpec, RequestKind, RequestOption, RichText, RoomView, Span, Style,
    WhoEntry,
};

fn caps(color: ColorDepth, width: u16) -> ClientCaps {
    caps_utf8(color, width, true)
}

fn caps_utf8(color: ColorDepth, width: u16, utf8: bool) -> ClientCaps {
    ClientCaps {
        kind: ClientKind::Ssh,
        color,
        width,
        height: 24,
        utf8,
        structured: false,
    }
}

fn rendered(event: &OutputEvent, color: ColorDepth, width: u16) -> Vec<u8> {
    let mut out = Vec::new();
    render(event, &caps(color, width), &AnsiTheme::default(), &mut out);
    out
}

fn rendered_ascii(event: &OutputEvent, width: u16) -> String {
    let mut out = Vec::new();
    render(
        event,
        &caps_utf8(ColorDepth::None, width, false),
        &AnsiTheme::default(),
        &mut out,
    );
    String::from_utf8(out).unwrap()
}

fn room(contents: Vec<ObjRef>, exits: Vec<ExitRef>) -> RoomView {
    RoomView {
        name: "Hall".into(),
        id: "#2".into(),
        desc: RichText::plain("A big hall."),
        contents,
        exits,
        zone: None,
        ambient: Default::default(),
    }
}

/// The rule under a room heading: `n` box-drawing dashes.
fn rule(n: usize) -> String {
    "\u{2500}".repeat(n)
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

/// An emit renders bare. A spoof only reaches a listener entitled to its
/// source (the world task downgrades it to an emit for everyone else), so
/// it names the true sender; a long one wraps flush like any narration.
#[test]
fn spoofed_emit_names_its_sender_and_plain_emit_does_not() {
    let emit = OutputEvent::Narrative {
        kind: NarrativeKind::Emit,
        actor: Some(nick()),
        text: RichText::plain("A door slams somewhere below."),
    };
    let spoof = OutputEvent::Narrative {
        kind: NarrativeKind::Spoof,
        actor: Some(nick()),
        text: RichText::plain("A door slams somewhere below."),
    };
    assert_eq!(
        rendered_ascii(&emit, 80),
        "A door slams somewhere below.\r\n"
    );
    assert_eq!(
        rendered_ascii(&spoof, 80),
        "[Nick] A door slams somewhere below.\r\n"
    );
}

/// A notice with the loudest severity and a one-tap action (interface plan
/// I11) — the shape line mode has to flatten into one row.
fn notice_event() -> OutputEvent {
    OutputEvent::Notice {
        id: 3,
        severity: NoticeSeverity::Urgent,
        category: "page".into(),
        text: RichText::plain("Zoe pages: are you about?"),
        action: Some(NoticeAction {
            label: "Reply".into(),
            prefill: "page Zoe=".into(),
        }),
        ts: 1_757_000_000_000,
    }
}

/// A two-option ask (interface plan I10): the numbers line mode prints are
/// the contract the `respond <n>` interpreter decodes.
fn choice_request_event() -> OutputEvent {
    OutputEvent::Request {
        epoch: "test".into(),
        id: 7,
        kind: RequestKind::Choice,
        prompt: RichText::plain("Who takes the watch?"),
        options: vec![
            RequestOption {
                id: "mara".into(),
                label: "Mara".into(),
            },
            RequestOption {
                id: "juno".into(),
                label: "Juno".into(),
            },
        ],
        deadline_ms: None,
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
    // A spoof only reaches a listener entitled to its source, so it is named.
    assert_eq!(
        rendered_str(&mk(NarrativeKind::Spoof, "The walls whisper."), d, 0),
        "[Nick] The walls whisper.\r\n"
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
        ambient: Default::default(),
    });
    // I3: `Name #id` (id dim), a rule, the description, a blank line, then
    // labelled facts in a nine-column gutter.
    let expected = format!(
        "\x1b[1mHall\x1b[0m \x1b[90m#2\x1b[0m\r\n\
         \x1b[90m{}\x1b[0m\r\n\
         A big hall.\r\n\
         \r\n\
         Here     ball\r\n\
         Exits    \x1b[1;36mnorth\x1b[0m  \x1b[1;36msouth\x1b[0m\r\n",
        rule(72)
    );
    assert_eq!(rendered_str(&ev, ColorDepth::Ansi16, 0), expected);
    // Empty contents/exits: the fact lines and the blank line vanish.
    let bare = OutputEvent::Room(RoomView {
        name: "Void".into(),
        id: "#9".into(),
        desc: RichText::plain("Nothing."),
        contents: vec![],
        exits: vec![],
        zone: None,
        ambient: Default::default(),
    });
    assert_eq!(
        rendered_str(&bare, ColorDepth::None, 0),
        format!("Void #9\r\n{}\r\nNothing.\r\n", rule(72))
    );
}

#[test]
fn room_rule_fits_the_width_and_falls_back_to_ascii() {
    let ev = OutputEvent::Room(room(vec![], vec![]));
    // min(width, 72): narrower terminals get a shorter rule, wider ones 72.
    let narrow = rendered_str(&ev, ColorDepth::None, 40);
    assert_eq!(narrow.split("\r\n").nth(1).unwrap(), rule(40));
    let wide = rendered_str(&ev, ColorDepth::None, 200);
    assert_eq!(wide.split("\r\n").nth(1).unwrap(), rule(72));
    for width in [1u16, 5, 40, 80] {
        // The rule itself never exceeds the width (a name is a word, and a
        // word longer than the width overflows by the documented rule).
        // (At width 1 the heading itself wraps, so find the rule by content.)
        let text = rendered_str(&ev, ColorDepth::None, width);
        let rule_line = text
            .split("\r\n")
            .find(|l| !l.is_empty() && l.chars().all(|c| c == '\u{2500}'))
            .expect("a rule line");
        assert!(
            ansi_len(rule_line) <= usize::from(width),
            "rule exceeds width {width}: {rule_line:?}"
        );
    }
    // No UTF-8: plain dashes, never a box-drawing character.
    let ascii = rendered_ascii(&ev, 30);
    assert_eq!(ascii.split("\r\n").nth(1).unwrap(), "-".repeat(30));
    assert!(!ascii.contains('\u{2500}'));
}

#[test]
fn room_facts_align_in_the_gutter_and_wrap_under_it() {
    let contents: Vec<ObjRef> = (0..12)
        .map(|i| ObjRef::new(format!("#{i}"), format!("guest{i:02}")))
        .collect();
    let exits = vec![
        ExitRef {
            name: "north".into(),
            aliases: vec![],
        },
        ExitRef {
            name: "quai".into(),
            aliases: vec![],
        },
    ];
    let mut view = room(contents, exits);
    view.ambient.insert(
        "weather".into(),
        serde_json::json!({ "state": "rain", "line": "Overhead: steady rain, a cold wind off the water, visibility poor." }),
    );
    let ev = OutputEvent::Room(view);
    for width in [40u16, 60, 78] {
        let text = rendered_str(&ev, ColorDepth::None, width);
        let lines: Vec<&str> = text.split("\r\n").collect();
        let here = lines
            .iter()
            .position(|l| l.starts_with("Here     "))
            .unwrap();
        let exits = lines
            .iter()
            .position(|l| l.starts_with("Exits    "))
            .unwrap();
        let weather = lines
            .iter()
            .position(|l| l.starts_with("Weather  "))
            .unwrap();
        assert!(here < exits && exits < weather);
        assert_eq!(lines[here - 1], "", "a blank line precedes the facts");
        // Every continuation line sits under the gutter, and nothing is wider
        // than the terminal.
        for (i, line) in lines.iter().enumerate() {
            if i > here && i != exits && i != weather && !line.is_empty() {
                assert!(
                    line.starts_with("         "),
                    "not under the gutter: {line:?}"
                );
            }
            assert!(
                ansi_len(line) <= usize::from(width),
                "too wide at {width}: {line:?}"
            );
        }
        assert!(text.contains("guest11"), "every occupant survives wrapping");
        assert!(text.contains("poor."), "the weather line survives wrapping");
    }
}

#[test]
fn narrative_hanging_indent_is_crlf_clean_and_rewraps() {
    let ev = say("the quick brown fox jumps over the lazy dog and keeps on running far away");
    let narrow = rendered_str(&ev, ColorDepth::None, 30);
    let lines: Vec<&str> = narrow.trim_end_matches("\r\n").split("\r\n").collect();
    assert!(lines.len() >= 3);
    assert!(lines[0].starts_with("Nick says, \""));
    for l in &lines[1..] {
        assert!(
            l.starts_with("     ") && !l.starts_with("      "),
            "indent 5: {l:?}"
        );
        assert!(ansi_len(l) <= 30);
    }
    // A wider terminal re-wraps to fewer lines with the same indent.
    let wide = rendered_str(&ev, ColorDepth::None, 60);
    let wide_lines: Vec<&str> = wide.trim_end_matches("\r\n").split("\r\n").collect();
    assert!(wide_lines.len() < lines.len());
    assert!(wide_lines[1].starts_with("     "));
    // The bytes stay CRLF-only.
    let bytes = rendered(&ev, ColorDepth::Ansi16, 30);
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            assert_eq!(bytes[i - 1], b'\r');
        }
    }
    // An emit has no speaker column and wraps flush.
    let emit = OutputEvent::Narrative {
        kind: NarrativeKind::Emit,
        actor: None,
        text: RichText::plain("a long scene-setting line that goes on past the width"),
    };
    let flush = rendered_str(&emit, ColorDepth::None, 30);
    let second = flush.split("\r\n").nth(1).unwrap();
    assert!(!second.is_empty() && !second.starts_with(' '), "{flush:?}");
}

#[test]
fn roll_line_renders_as_is_with_the_marker_styled() {
    // The layout `mush_ruleset::roll_narration` produces (interface plan I3).
    let text = RichText(vec![
        Span::tagged("roll", "roll"),
        Span::plain("  "),
        Span::tagged("player", "Mireille"),
        Span::plain("  perception+alertness  8 6 4 3 = 2 successes  diff 6"),
    ]);
    let ev = OutputEvent::Narrative {
        kind: NarrativeKind::Emit,
        actor: Some(ObjRef::new("#5", "Mireille")),
        text,
    };
    assert_eq!(
        rendered_str(&ev, ColorDepth::None, 0),
        "roll  Mireille  perception+alertness  8 6 4 3 = 2 successes  diff 6\r\n"
    );
    assert_eq!(
        rendered_str(&ev, ColorDepth::Ansi16, 0),
        "\x1b[1;35mroll\x1b[0m  \x1b[36mMireille\x1b[0m  perception+alertness  8 6 4 3 = 2 successes  diff 6\r\n"
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
            location: Some("Hall".into()),
            here: true,
            doing: Some("Building".into()),
        },
        WhoEntry {
            name: "Wizardess".into(),
            alias: Some("Wiz".into()),
            idle_s: 120,
            conn_s: 90_000,
            location: None,
            here: false,
            doing: None,
        },
    ]
}

#[test]
fn who_golden_width_78() {
    let ev = OutputEvent::Who(who_entries());
    let text = rendered_str(&ev, ColorDepth::None, 78);
    let lines: Vec<&str> = text.split("\r\n").collect();
    // Aligned columns at fixed widths 16/5/8/14 + two-space gutters (I3
    // adds `Where`, D-I-12).
    assert_eq!(
        lines[0],
        format!(
            "{:<16}  {:>5}  {:>8}  {:<14}  {}",
            "Name", "Idle", "Conn", "Where", "Doing"
        )
    );
    assert_eq!(
        lines[1],
        format!(
            "{:<16}  {:>5}  {:>8}  {:<14}  {}",
            "Nick", "5s", "1h", "Hall", "Building"
        )
    );
    // Empty Where and Doing trim the trailing padding.
    assert_eq!(
        lines[2],
        format!("{:<16}  {:>5}  {:>8}", "Wizardess", "2m", "1d")
    );
    assert_eq!(lines[3], "2 connected, 1 in your room.");
    assert_eq!(lines[4], "");
    assert_eq!(lines.len(), 5);
}

#[test]
fn who_where_is_whatever_the_server_allowed_and_control_bytes_cannot_shift_a_column() {
    // A staffer's listing carries every location; a player's only their own
    // room's — the renderer prints what it is given (D-I-12 lives in core).
    let mut entries = who_entries();
    entries[1].location = Some("Tower".into());
    let text = rendered_str(&OutputEvent::Who(entries), ColorDepth::None, 78);
    let lines: Vec<&str> = text.split("\r\n").collect();
    assert!(lines[2].contains("  Tower"));
    assert_eq!(lines[3], "2 connected, 1 in your room.");
    // Control characters in a name, a location or a doing are dropped before
    // the columns are laid out, so every row's Idle column starts at the
    // header's.
    let evil = vec![WhoEntry {
        name: "Ni\x07ck\x1b[31m".into(),
        alias: None,
        idle_s: 5,
        conn_s: 5,
        location: Some("Ha\x08ll".into()),
        here: true,
        doing: Some("bad\x1b[2Jthings".into()),
    }];
    let out = rendered(&OutputEvent::Who(evil), ColorDepth::None, 78);
    assert!(!out.contains(&0x1b) && !out.contains(&0x07) && !out.contains(&0x08));
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.split("\r\n").collect();
    let idle_col = lines[0].find(" Idle").unwrap();
    assert_eq!(&lines[1][idle_col..idle_col + 5], "   5s");
    assert!(lines[1].contains("  Hall  "), "{:?}", lines[1]);
    // The ESC is gone; the printable remainder of the sequence stays text.
    assert!(lines[1].ends_with("bad[2Jthings"), "{:?}", lines[1]);
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
        here: false,
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
        // Look-back LB-9: `Notice`, `Request`, `View`, `Composing` and
        // `Resumed` never rode the CRLF and control-byte sweep, though three
        // of them print text a module authored.
        notice_event(),
        choice_request_event(),
        // The same two with hostile server-authored text: an embedded ESC,
        // a bare newline and a lone CR in every field a module chooses.
        OutputEvent::Notice {
            id: 5,
            severity: NoticeSeverity::Warn,
            category: "pa\x1b[31mge\n".into(),
            text: RichText::plain("two\nlines\rand \x1b[2Jan escape"),
            action: Some(NoticeAction {
                label: "Open\n".into(),
                prefill: "page \x1b]0;x\x07Zoe=".into(),
            }),
            ts: 0,
        },
        OutputEvent::Request {
            epoch: "test".into(),
            id: 6,
            kind: RequestKind::Choice,
            prompt: RichText::plain("pick\none\x1b[31m"),
            options: vec![
                RequestOption {
                    id: "a".into(),
                    label: "A\nlabel".into(),
                },
                RequestOption {
                    id: "b".into(),
                    label: "B\x1b[0m".into(),
                },
            ],
            deadline_ms: None,
        },
        action_receipt_event(),
        OutputEvent::Composing {
            actor: nick(),
            active: true,
        },
        OutputEvent::Resumed { replayed: true },
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
            ambient: Default::default(),
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

// -- I10 / I11 line-mode goldens (look-back LB-9) ----------------------------

/// Look-back LB-9: `Notice`, `Request`, `View`, `Composing` and `Resumed`
/// reached line mode with no golden of their own, though three of them print
/// server-authored text. One frozen render each, at the depth a plain SSH
/// client gets.
#[test]
fn notice_and_request_line_goldens() {
    assert_eq!(
        rendered_str(&notice_event(), ColorDepth::None, 0),
        "[page] Zoe pages: are you about?  (page Zoe=)\r\n"
    );
    // The severity picks the tag and nothing else: Urgent is the error
    // colour, Info and Warn the system one.
    let calm = OutputEvent::Notice {
        id: 4,
        severity: NoticeSeverity::Info,
        category: "jobs".into(),
        text: RichText::plain("Job 12 was answered."),
        action: None,
        ts: 0,
    };
    assert_eq!(
        rendered_str(&calm, ColorDepth::None, 0),
        "[jobs] Job 12 was answered.\r\n"
    );
    assert_ne!(
        rendered(&notice_event(), ColorDepth::Ansi16, 0),
        rendered(&calm, ColorDepth::Ansi16, 0)
    );

    assert_eq!(
        rendered_str(&choice_request_event(), ColorDepth::None, 0),
        "The game asks:\r\n\
         Who takes the watch?\r\n\
         \x20 1  Mara\r\n\
         \x20 2  Juno\r\n\
         \x20 answer:  respond <number>   respond cancel\r\n"
    );
    let confirm = OutputEvent::Request {
        epoch: "test".into(),
        id: 8,
        kind: RequestKind::Confirm,
        prompt: RichText::plain("Take the watch?"),
        options: vec![],
        deadline_ms: None,
    };
    assert_eq!(
        rendered_str(&confirm, ColorDepth::None, 0),
        "The game asks:\r\n\
         Take the watch?\r\n\
         \x20 answer:  respond yes   respond no   respond cancel\r\n"
    );
    let text = OutputEvent::Request {
        epoch: "test".into(),
        id: 9,
        kind: RequestKind::Text,
        prompt: RichText::plain("Name the watch officer."),
        options: vec![],
        deadline_ms: None,
    };
    assert_eq!(
        rendered_str(&text, ColorDepth::None, 0),
        "The game asks:\r\n\
         Name the watch officer.\r\n\
         \x20 answer:  respond <your answer>   respond cancel\r\n"
    );

    // `Composing` and `Resumed` render nothing at all in a scrolling stream:
    // there is nowhere to put "is typing" and nothing to say when it stops,
    // and SSH never resumes. Pinned so a later "just print it" is a decision.
    assert!(
        rendered(
            &OutputEvent::Composing {
                actor: nick(),
                active: true,
            },
            ColorDepth::TrueColor,
            80
        )
        .is_empty()
    );
    assert!(
        rendered(
            &OutputEvent::Resumed { replayed: true },
            ColorDepth::TrueColor,
            80
        )
        .is_empty()
    );
}

/// Look-back LB-9: the option numbers line mode prints ARE the numbers
/// `respond <n>` decodes with `checked_sub(1)` (`mush-core`'s
/// `line_respond`). Nothing bound the two before. This side owns only the
/// printing: the numbers are read back off the rendered page and fed through
/// the same 1-based rule, and each must land on the option whose label sits
/// on that row.
#[test]
fn the_printed_option_numbers_are_the_ones_respond_decodes() {
    let OutputEvent::Request { options, .. } = choice_request_event() else {
        unreachable!()
    };
    let page = rendered_str(&choice_request_event(), ColorDepth::None, 0);
    let rows: Vec<&str> = page
        .lines()
        .filter(|l| l.trim_start().starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    assert_eq!(rows.len(), options.len(), "one numbered row per option");
    for row in rows {
        let mut parts = row.split_whitespace();
        let printed: usize = parts.next().unwrap().parse().expect("a bare number");
        let label = parts.next().expect("a label");
        let chosen = options
            .get(printed.checked_sub(1).expect("1-based, never 0"))
            .expect("the printed number indexes an option");
        assert_eq!(chosen.label, label, "row {printed} names {label}");
    }
    // And the first is 1, not 0: a `respond 0` has nothing to land on.
    assert!(
        page.contains("  1  Mara") && page.contains("  2  Juno"),
        "{page}"
    );
    assert!(!page.contains("  0  "), "{page}");
}

// -- the canonical cockpit fixtures (16b cut Q.3; doc 16 §5) -----------------
//
// One scene, three payloads, produced by `tacsim`'s pure builders and committed
// under `web/src/lib/fixtures/cockpit/`. This side owns the LINE renderer's half
// of doc 16 §5's parity claim: what the `grid`-shaped cockpit frame prints, and
// that the other two print nothing at all.
//
// `view_grid.txt` is the twenty map rows of that print, north-up, ONE GLYPH PER
// CELL: since 16b cut B.4 the window is 39 by 20 cells and the line client
// prints each cell two columns wide, so the file holds the printed rows halved
// (every glyph pair collapsed to one). It is what 16b cut Q.4's (TUI) and Q.5's
// (web) compositions of `arena_map.json` + `cockpit_frame.json` at exactly 39 by
// 20 cells must equal byte for byte — that equality, and not three separate
// goldens, is what makes the three renderers one renderer.
//
// Re-bless with `TACSIM_BLESS=1 cargo test -p mush-ansi --test golden`.

/// The fixture directory, from this crate's manifest.
fn fixture_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/src/lib/fixtures/cockpit")
}

/// One committed payload as a `Panel` keyframe on its own feed.
fn fixture_panel(feed: &str, text: &str) -> OutputEvent {
    OutputEvent::Panel {
        feed: feed.into(),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::from_str(text).expect("a committed fixture is JSON"),
    }
}

/// The 78-by-20 print of the committed `view_frame.json` — 39 by 20 cells, each
/// printed two columns wide (16b cut B.4) — with the map rows pinned PER CELL as
/// `view_grid.txt`.
///
/// **The parity rule** (16b cut B.4, `B-24`): `view_grid.txt` stays one glyph per
/// CELL, 20 rows of 39, because that is what the TUI's, the web's and tacsim's
/// own compositions draw and compare against it. This test is the one place the
/// doubling is seen: every printed row is 78 columns, every glyph pair in it is
/// one glyph twice, and the rows HALVED (each pair collapsed to one glyph) are
/// what is compared with — and blessed as — `view_grid.txt`.
#[test]
fn the_cockpit_window_fixture_prints_the_committed_grid() {
    // Width 80 less the two-column margin is 78; height 24 less the four lines
    // of chrome is 20 — the terminal `VIEW_COLUMNS` (39 cells, two columns
    // each) by `VIEW_ROWS` was sized for.
    let view = fixture_panel(
        "arena.cockpit",
        include_str!("../../../web/src/lib/fixtures/cockpit/view_frame.json"),
    );
    let page = rendered_str(&view, ColorDepth::None, 80);
    let lines: Vec<&str> = page.trim_end_matches("\r\n").split("\r\n").collect();
    assert_eq!(
        lines.first().copied(),
        Some("Arena window (fixture_yard, 1000 cm cells)"),
        "{page}"
    );
    assert!(
        lines.get(1).is_some_and(|l| l.starts_with("legend: ")),
        "{page}"
    );
    assert_eq!(
        lines.last().copied(),
        // 16b cut B.4 (the `T-60` footer slip): the extent is the ARENA's, in
        // arena cells — the window's corner (`origin_cell`) plus the viewport,
        // the arena's size (`extent`), the platform's own arena cell, and what
        // a cell is. It used to count the window's own cells.
        Some("x 0..38  y 0..19  of 96x64  you at (6, 4)  (10 m a cell, north up)"),
        "the footer names the window in arena cells and where the platform is: {page}"
    );
    // The map itself: the twenty rows immediately above the footer, north-up.
    // Found from the footer rather than by counting from the top, because the
    // legend is one line of frame data and the wrapper may give it two.
    let footer = lines
        .iter()
        .rposition(|line| line.starts_with("x 0.."))
        .expect("a footer");
    let printed: Vec<&str> = lines
        .iter()
        .skip(footer.saturating_sub(20))
        .take(20)
        .copied()
        .collect();
    assert_eq!(printed.len(), 20, "twenty rows: {page}");
    // Every printed row is 78 columns and every glyph pair is one glyph twice;
    // the per-cell row is each pair collapsed to one glyph.
    let mut grid = String::new();
    for row in &printed {
        let glyphs: Vec<char> = row.chars().collect();
        assert_eq!(glyphs.len(), 78, "seventy-eight columns: {row:?}");
        for pair in glyphs.chunks(2) {
            assert_eq!(
                pair.first(),
                pair.get(1),
                "each cell is one glyph printed twice: {row:?}"
            );
            grid.extend(pair.first());
        }
        grid.push('\n');
    }
    for row in grid.lines() {
        assert_eq!(row.chars().count(), 39, "thirty-nine cells: {row:?}");
    }
    // The pilot's own marker, the hostile and the unknown on the building are
    // all where the fixture put them — read off the PER-CELL rows (the printed
    // page halved), because that is the thing the TUI and the web have to match.
    let row_from_south = |y: usize| {
        grid.lines()
            .nth(19_usize.saturating_sub(y))
            .expect("a row")
            .chars()
            .collect::<Vec<char>>()
    };
    assert_eq!(row_from_south(4).get(6).copied(), Some('@'));
    assert_eq!(row_from_south(14).get(36).copied(), Some('x'));
    assert_eq!(row_from_south(9).get(2).copied(), Some('+'));
    assert_eq!(
        row_from_south(7).get(14).copied(),
        Some('?'),
        "the unknown contact's marker beats the building it stands on — and \
         (16b cut V.1) the spot `B` placed under it first: the contact owns the \
         spot's cell"
    );
    assert_eq!(row_from_south(7).get(13).copied(), Some('#'));
    // **16b cut V.1**: the named spot `A` The Ford, drawn over the pond.
    assert_eq!(
        row_from_south(3).get(25).copied(),
        Some('A'),
        "the spot's letter is drawn over the water it stands in"
    );
    // The legend names the spot after the contacts — the line client prints
    // each contact's glyph with its class word (`? contact`), the TUI and the web
    // with its designator (`? X3`) — and never the covered `B`.
    let head = lines
        .iter()
        .take(footer.saturating_sub(20))
        .copied()
        .collect::<Vec<&str>>()
        .join(" ");
    let contact = head
        .find("? contact")
        .expect("the unknown contact's legend entry");
    let spot = head.find("A The Ford").expect("the spot's legend entry");
    assert!(
        contact < spot,
        "the spot is named after the contacts: {head}"
    );
    assert!(
        !head.contains("The Roof"),
        "a covered spot is named nowhere: {head}"
    );

    let path = fixture_dir().join("view_grid.txt");
    if std::env::var("TACSIM_BLESS").is_ok() {
        std::fs::write(&path, &grid).expect("the fixture is writable");
        return;
    }
    assert_eq!(
        include_str!("../../../web/src/lib/fixtures/cockpit/view_grid.txt"),
        grid,
        "web/src/lib/fixtures/cockpit/view_grid.txt is out of date; re-bless with \
         `TACSIM_BLESS=1 cargo test -p mush-ansi --test golden`"
    );
}

/// **The line-client-unchanged proof** (16b cut Q.3): neither the per-pulse
/// cockpit frame nor the whole-arena map prints one byte on a terminal.
///
/// The cockpit frame declares no shape at all, and the map declares `terrain`,
/// which is not `grid` — so a pilot who never types `VIEW` is sent a keyframe
/// twice a second and reads nothing, and a line client that somehow subscribed
/// to `arena.map` would print nothing rather than 256 rows.
#[test]
fn the_cockpit_frame_and_the_arena_map_print_nothing_on_a_line_client() {
    for (feed, text) in [
        (
            "arena.cockpit",
            include_str!("../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"),
        ),
        (
            "arena.map",
            include_str!("../../../web/src/lib/fixtures/cockpit/arena_map.json"),
        ),
    ] {
        let panel = fixture_panel(feed, text);
        for depth in [ColorDepth::None, ColorDepth::TrueColor] {
            for width in [0_u16, 40, 80, 200] {
                assert!(
                    rendered(&panel, depth, width).is_empty(),
                    "{feed} printed something at {depth:?}/{width}"
                );
            }
        }
    }
}

/// **16b cut I.1 (16c N9)**: `SIM REPLAY`'s pushed grid prints as `VIEW`'s
/// does, and the `arena.replay` query's document prints nothing.
///
/// The window is `web/src/lib/fixtures/replay/replay_window.json`, which
/// `crates/tacsim/tests/replay.rs` blesses (keyframe 1 of a hand-built replay on
/// the cockpit fixtures' yard): a `grid` of 39 by 20 cells, two columns a cell,
/// `you: null` — so the footer names the window and no platform — with `RA` at
/// (6, 4) drawn `o`, `BA` at (36, 14) drawn `*`, and `BB` standing on The Roof
/// at (14, 7) drawn `*` over the spot `B` it covers. The document is
/// `replay.json` under the query's own `replay` shape, which is not `grid`: a
/// line client that somehow received one prints not one byte. No golden file is
/// blessed here; 16b cut I.2 blesses `replay_grid.txt` from this print.
#[test]
fn i1_the_replay_window_prints_as_view_does_and_the_document_prints_nothing() {
    let window = fixture_panel(
        "arena.replay",
        include_str!("../../../web/src/lib/fixtures/replay/replay_window.json"),
    );
    let page = rendered_str(&window, ColorDepth::None, 80);
    let lines: Vec<&str> = page.trim_end_matches("\r\n").split("\r\n").collect();
    assert_eq!(
        lines.first().copied(),
        Some("Replay of match 7 at 0:05 (fixture_yard, 1000 cm cells)"),
        "{page}"
    );
    assert_eq!(
        lines.last().copied(),
        Some("x 0..38  y 0..19  of 96x64  (10 m a cell, north up)"),
        "a replay is nobody's platform, so the footer names no `you at`: {page}"
    );
    let footer = lines
        .iter()
        .rposition(|line| line.starts_with("x 0.."))
        .expect("a footer");
    let printed: Vec<&str> = lines
        .iter()
        .skip(footer.saturating_sub(20))
        .take(20)
        .copied()
        .collect();
    assert_eq!(printed.len(), 20, "twenty rows: {page}");
    let grid: Vec<Vec<char>> = printed
        .iter()
        .map(|row| {
            let glyphs: Vec<char> = row.chars().collect();
            assert_eq!(glyphs.len(), 78, "seventy-eight columns: {row:?}");
            glyphs
                .chunks(2)
                .filter_map(|pair| pair.first().copied())
                .collect()
        })
        .collect();
    let at = |x: usize, y: usize| {
        grid.get(19_usize.saturating_sub(y))
            .and_then(|row| row.get(x))
            .copied()
    };
    assert_eq!(at(6, 4), Some('o'), "RA");
    assert_eq!(at(2, 9), Some('o'), "RB");
    assert_eq!(at(36, 14), Some('*'), "BA");
    assert_eq!(at(14, 7), Some('*'), "BB, over the spot it covers");
    assert_eq!(at(25, 3), Some('A'), "The Ford");
    let head = lines
        .iter()
        .take(footer.saturating_sub(20))
        .copied()
        .collect::<Vec<&str>>()
        .join(" ");
    assert!(head.contains("o red") && head.contains("* blue"), "{head}");
    assert!(
        !head.contains("The Roof"),
        "a covered spot is named nowhere: {head}"
    );

    let document = fixture_panel(
        "arena.replay",
        include_str!("../../../web/src/lib/fixtures/replay/replay.json"),
    );
    let OutputEvent::Panel {
        feed,
        kind,
        mut data,
    } = document
    else {
        unreachable!("fixture_panel builds a panel");
    };
    data["schema"] = serde_json::json!({ "id": "arena.replay", "version": 1, "shape": "replay" });
    let answer = OutputEvent::Panel { feed, kind, data };
    for depth in [ColorDepth::None, ColorDepth::TrueColor] {
        for width in [0_u16, 40, 80, 200] {
            assert!(
                rendered(&answer, depth, width).is_empty(),
                "the replay document printed something at {depth:?}/{width}"
            );
        }
    }
}

/// Look-back I3-C36: a room field carrying a newline used to forge a second
/// fact row. `LineWriter::write` splits an embedded newline with `end_line()`
/// at indent 0 — flush left, exactly where a fact LABEL sits — and room
/// fields are module- and builder-written, never sanitised on the way in the
/// way player lines are. An object called "ball\nExits    nowhere" printed a
/// row indistinguishable from the real `Exits` row below it.
#[test]
fn control_bytes_in_room_fields_cannot_break_the_fact_gutter() {
    let mut view = RoomView {
        name: "Hall\nExits    forged".into(),
        id: "#2".into(),
        desc: RichText::plain("A big hall."),
        contents: vec![
            ObjRef::new("#7", "ball\nHere     forged"),
            ObjRef::new("#8", "lamp\x1b[31m"),
        ],
        exits: vec![ExitRef {
            name: "north\nWeather  forged".into(),
            aliases: vec![],
        }],
        zone: None,
        ambient: Default::default(),
    };
    view.ambient.insert(
        "weather".into(),
        serde_json::json!({ "line": "Steady rain.\nHere     forged again" }),
    );
    let out = rendered_str(&OutputEvent::Room(view), ColorDepth::None, 80);
    let lines: Vec<&str> = out.lines().collect();
    for label in ["Here", "Exits", "Weather"] {
        assert_eq!(
            lines.iter().filter(|l| l.starts_with(label)).count(),
            1,
            "exactly one {label} row:\n{out}"
        );
    }
    // Every continuation inside the facts block wraps under the nine-column
    // gutter; nothing a field carried reaches column 0.
    let first_fact = lines.iter().position(|l| l.starts_with("Here")).unwrap();
    for line in &lines[first_fact..] {
        assert!(
            line.starts_with("Here")
                || line.starts_with("Exits")
                || line.starts_with("Weather")
                || line.starts_with("         ")
                || line.is_empty(),
            "a field escaped the gutter: {line:?}\n{out}"
        );
    }
    for line in &lines {
        assert!(line.chars().count() <= 80, "row too wide: {line:?}");
    }
    // The forged labels are still THERE as text — flattened onto their own
    // field's line, which is the honest place for them.
    assert!(out.contains("Here     ball Here     forged"), "{out}");
}
