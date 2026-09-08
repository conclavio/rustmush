//! Wire-shape tests: serde round-trips for every variant plus golden JSON
//! assertions pinning the exact on-the-wire encoding of representative
//! frames. If a golden test here fails, the wire format changed — that is a
//! breaking protocol change and requires a PROTO_VERSION bump.

use mush_proto::{
    ClientCaps, ClientCommand, ClientKind, Color, ColorDepth, Envelope, ExitRef, InputMode,
    NarrativeKind, ObjRef, OutputEvent, PROTO_VERSION, PanelFrameKind, ProjectedCommitToken,
    PromptSpec, Query, RichText, RoomView, Span, Style, ViewEnvelope, WhoEntry,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use std::fmt::Debug;

/// Serialize → deserialize → compare, and also re-serialize the round-tripped
/// value to catch asymmetric skip/default attributes.
fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: &T) {
    let json = serde_json::to_string(value).expect("serialize");
    let back: T = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(&back, value, "round-trip changed the value: {json}");
    let json2 = serde_json::to_string(&back).expect("re-serialize");
    assert_eq!(json, json2, "re-serialization not stable");
}

fn wizard() -> ObjRef {
    ObjRef::new("#123", "Wizard")
}

fn sample_room() -> RoomView {
    RoomView {
        name: "The Rusty Anchor".into(),
        id: "#42".into(),
        desc: RichText::plain("A dim tavern smelling of salt and old rope."),
        contents: vec![wizard(), ObjRef::new("#200", "Barkeep")],
        exits: vec![
            ExitRef {
                name: "north".into(),
                aliases: vec!["n".into()],
            },
            ExitRef {
                name: "out".into(),
                aliases: vec![],
            },
        ],
        zone: Some(ObjRef::new("#7", "Docks")),
        ambient: Default::default(),
    }
}

// ---------------------------------------------------------------------------
// Round-trips: every OutputEvent variant
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_every_output_event_variant() {
    let styled = RichText::new()
        .styled(
            Style::default()
                .with_fg(Color::Rgb(200, 30, 30))
                .with_bold(),
            "WARNING",
        )
        .text(" the kettle is boiling");

    let events = vec![
        OutputEvent::Room(sample_room()),
        OutputEvent::Narrative {
            kind: NarrativeKind::Pose,
            actor: Some(wizard()),
            text: RichText::tag("pose", "Wizard grins."),
        },
        OutputEvent::Narrative {
            kind: NarrativeKind::Emit,
            actor: None,
            text: RichText::plain("Rain begins to fall."),
        },
        OutputEvent::Sys(styled),
        OutputEvent::Channel {
            channel: "pub".into(),
            seq: 991,
            actor: wizard(),
            title: Some("the Grey".into()),
            kind: NarrativeKind::Say,
            text: RichText::tag("channel", "hello, channel"),
            ts: 1_756_300_000_123,
        },
        OutputEvent::MailNotice {
            from: wizard(),
            subject: "meeting at the docks".into(),
            id: 17,
            unread: 3,
        },
        OutputEvent::Who(vec![
            WhoEntry {
                name: "Wizard".into(),
                alias: Some("Wiz".into()),
                idle_s: 42,
                conn_s: 3600,
                location: Some("The Rusty Anchor".into()),
                here: false,
                doing: Some("brooding".into()),
            },
            WhoEntry {
                name: "Guest1".into(),
                alias: None,
                idle_s: 0,
                conn_s: 5,
                location: None,
                here: false,
                doing: None,
            },
        ]),
        OutputEvent::Panel {
            feed: "wod.roll".into(),
            kind: PanelFrameKind::Delta,
            data: json!({"dice": [8, 10, 3], "successes": 2}),
        },
        OutputEvent::View(
            ViewEnvelope::keyframe("legal.actions", 1, 7, 11, json!([{"verb": "brace"}])).unwrap(),
        ),
        OutputEvent::Weather {
            zone: "docks".into(),
            text: RichText::tag("weather", "Fog rolls in off the bay."),
            icon: "fog".into(),
        },
        OutputEvent::Composing {
            actor: wizard(),
            active: true,
        },
        OutputEvent::Composing {
            actor: wizard(),
            active: false,
        },
        OutputEvent::Prompt(PromptSpec {
            mode: InputMode::Masked,
            text: Some(RichText::plain("Password: ")),
        }),
        OutputEvent::Prompt(PromptSpec {
            mode: InputMode::Normal,
            text: None,
        }),
        OutputEvent::Ping { nonce: 7 },
        OutputEvent::Resumed { replayed: true },
        OutputEvent::Resumed { replayed: false },
        OutputEvent::Reply {
            id: 3,
            event: Box::new(OutputEvent::Who(vec![])),
        },
        OutputEvent::Hello {
            proto: PROTO_VERSION,
            server: "rustmush 0.1".into(),
            features: vec!["mail".into(), "channels".into(), "wod".into()],
            panels: vec!["wod.roll".into(), "wod.sheet".into()],
        },
        OutputEvent::SessionInfo {
            character: wizard(),
            account: "nick".into(),
            connections: 2,
        },
        OutputEvent::Disconnect {
            reason: RichText::tag("system", "Server shutting down."),
        },
    ];
    for ev in &events {
        roundtrip(ev);
        // Every event must also survive inside an envelope (flatten path).
        roundtrip(&Envelope::new(7, 1_756_300_000_000, ev.clone()));
    }
}

// ---------------------------------------------------------------------------
// Round-trips: every ClientCommand and Query variant
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_every_client_command_variant() {
    let commands = vec![
        ClientCommand::Line("look north".into()),
        ClientCommand::Req {
            id: 1,
            q: Query::MailList { folder: None },
        },
        ClientCommand::Req {
            id: 2,
            q: Query::MailList {
                folder: Some("archive".into()),
            },
        },
        ClientCommand::Req {
            id: 3,
            q: Query::Who,
        },
        ClientCommand::Req {
            id: 4,
            q: Query::Sheet,
        },
        ClientCommand::Req {
            id: 5,
            q: Query::ChannelBacklog {
                channel: "pub".into(),
                before_seq: Some(991),
                limit: 50,
            },
        },
        ClientCommand::Req {
            id: 6,
            q: Query::ChannelBacklog {
                channel: "staff".into(),
                before_seq: None,
                limit: 20,
            },
        },
        ClientCommand::Req {
            id: 7,
            q: Query::PanelSubscribe {
                feed: "wod.roll".into(),
            },
        },
        ClientCommand::Req {
            id: 8,
            q: Query::Commands,
        },
        ClientCommand::Req {
            id: 9,
            q: Query::NarrativeBacklog {
                before_seq: Some(4180),
                limit: 100,
            },
        },
        ClientCommand::Req {
            id: 10,
            q: Query::NarrativeBacklog {
                before_seq: None,
                limit: 20,
            },
        },
        ClientCommand::Composing { active: true },
        ClientCommand::Composing { active: false },
        ClientCommand::Resume { last_seq: 4180 },
        ClientCommand::Ack {
            commit: ProjectedCommitToken::new("0123456789abcdef0123456789abcdef").unwrap(),
        },
        ClientCommand::Pong { nonce: 99 },
        ClientCommand::Caps(ClientCaps {
            kind: ClientKind::Web,
            color: ColorDepth::TrueColor,
            width: 120,
            height: 40,
            utf8: true,
            structured: true,
        }),
    ];
    for cmd in &commands {
        roundtrip(cmd);
    }

    assert!(
        serde_json::from_value::<ClientCommand>(json!({
            "t": "ack",
            "d": { "commit": "not-a-commit" }
        }))
        .is_err(),
        "malformed semantic acknowledgements must fail protocol decoding"
    );

    // Interface plan I12 (look-back I12-C2): a `NarrativeBacklog` with no
    // cursor omits the key on the wire, and the literal the web client
    // hand-writes for its first page decodes to exactly that.
    let first_page = ClientCommand::Req {
        id: 11,
        q: Query::NarrativeBacklog {
            before_seq: None,
            limit: 200,
        },
    };
    let encoded = serde_json::to_value(&first_page).unwrap();
    assert!(
        encoded["d"]["q"]["d"].get("before_seq").is_none(),
        "`before_seq: None` is omitted, not `null`: {encoded}"
    );
    let literal: ClientCommand = serde_json::from_value(json!({
        "t": "req",
        "d": { "id": 11, "q": { "t": "narrative_backlog", "d": { "limit": 200 } } }
    }))
    .unwrap();
    assert_eq!(literal, first_page);
}

#[test]
fn roundtrip_client_caps_and_colors() {
    for caps in [
        ClientCaps {
            kind: ClientKind::Ssh,
            color: ColorDepth::Ansi256,
            width: 80,
            height: 24,
            utf8: true,
            structured: false,
        },
        ClientCaps {
            kind: ClientKind::Web,
            color: ColorDepth::TrueColor,
            width: 120,
            height: 40,
            utf8: true,
            structured: true,
        },
        ClientCaps {
            kind: ClientKind::Ssh,
            color: ColorDepth::None,
            width: 132,
            height: 50,
            utf8: false,
            structured: false,
        },
    ] {
        roundtrip(&caps);
    }
    for color in [Color::Ansi(1), Color::Idx256(208), Color::Rgb(1, 2, 3)] {
        roundtrip(&color);
    }
}

// ---------------------------------------------------------------------------
// Golden wire shapes — exact strings; field order is part of the contract
// only insofar as these must PARSE identically, but we pin the exact bytes
// to catch accidental attribute churn.
// ---------------------------------------------------------------------------

#[test]
fn golden_say_narrative() {
    let ev = OutputEvent::Narrative {
        kind: NarrativeKind::Say,
        actor: Some(wizard()),
        text: RichText::tag("say", "Wizard says, \"hello\""),
    };
    assert_eq!(
        serde_json::to_string(&ev).unwrap(),
        r##"{"t":"narrative","d":{"kind":"say","actor":{"id":"#123","name":"Wizard"},"text":[{"text":"Wizard says, \"hello\"","tag":"say"}]}}"##
    );
}

#[test]
fn golden_room() {
    let ev = OutputEvent::Room(RoomView {
        name: "The Rusty Anchor".into(),
        id: "#42".into(),
        desc: RichText::plain("A dim tavern."),
        contents: vec![wizard()],
        exits: vec![ExitRef {
            name: "north".into(),
            aliases: vec!["n".into()],
        }],
        zone: None,
        ambient: Default::default(),
    });
    assert_eq!(
        serde_json::to_string(&ev).unwrap(),
        r##"{"t":"room","d":{"name":"The Rusty Anchor","id":"#42","desc":[{"text":"A dim tavern."}],"contents":[{"id":"#123","name":"Wizard"}],"exits":[{"name":"north","aliases":["n"]}]}}"##
    );
}

#[test]
fn golden_panel() {
    let ev = OutputEvent::Panel {
        feed: "wod.roll".into(),
        kind: PanelFrameKind::Keyframe,
        data: json!({"pool": 7}),
    };
    assert_eq!(
        serde_json::to_string(&ev).unwrap(),
        r#"{"t":"panel","d":{"feed":"wod.roll","kind":"keyframe","data":{"pool":7}}}"#
    );
}

#[test]
fn golden_semantic_view() {
    let ev = OutputEvent::View(
        ViewEnvelope::delta("unit.status", 2, 9, 43, 42, json!({"speed": 3}))
            .unwrap()
            .with_commit("txn-public-17")
            .unwrap(),
    );
    assert_eq!(
        serde_json::to_string(&ev).unwrap(),
        r#"{"t":"view","d":{"view":"unit.status","schema_version":2,"generation":9,"sequence":43,"base_sequence":42,"commit":"txn-public-17","data":{"speed":3}}}"#
    );
}

#[test]
fn golden_envelope_flattens_event_tag() {
    let env = Envelope::new(
        4212,
        1_756_300_000_123,
        OutputEvent::Sys(RichText::tag("system", "Server restarting.")),
    );
    assert_eq!(
        serde_json::to_string(&env).unwrap(),
        r#"{"seq":4212,"ts":1756300000123,"t":"sys","d":[{"text":"Server restarting.","tag":"system"}]}"#
    );
    // And the flattened form parses back.
    let back: Envelope = serde_json::from_str(
        r#"{"seq":4212,"ts":1756300000123,"t":"sys","d":[{"text":"Server restarting.","tag":"system"}]}"#,
    )
    .unwrap();
    assert_eq!(back, env);
}

#[test]
fn golden_client_line_and_req() {
    assert_eq!(
        serde_json::to_string(&ClientCommand::Line("look".into())).unwrap(),
        r#"{"t":"line","d":"look"}"#
    );
    assert_eq!(
        serde_json::to_string(&ClientCommand::Req {
            id: 3,
            q: Query::Who
        })
        .unwrap(),
        r#"{"t":"req","d":{"id":3,"q":{"t":"who"}}}"#
    );
    assert_eq!(
        serde_json::to_string(&ClientCommand::Resume { last_seq: 4180 }).unwrap(),
        r#"{"t":"resume","d":{"last_seq":4180}}"#
    );
}

#[test]
fn golden_hello() {
    let ev = OutputEvent::Hello {
        proto: PROTO_VERSION,
        server: "rustmush 0.1".into(),
        features: vec!["mail".into(), "channels".into()],
        panels: vec!["wod.roll".into()],
    };
    assert_eq!(
        serde_json::to_string(&ev).unwrap(),
        r#"{"t":"hello","d":{"proto":1,"server":"rustmush 0.1","features":["mail","channels"],"panels":["wod.roll"]}}"#
    );
}

#[test]
fn golden_channel() {
    let ev = OutputEvent::Channel {
        channel: "pub".into(),
        seq: 991,
        actor: wizard(),
        title: Some("the Grey".into()),
        kind: NarrativeKind::Say,
        text: RichText::tag("channel", "hello, channel"),
        ts: 1_756_300_000_123,
    };
    assert_eq!(
        serde_json::to_string(&ev).unwrap(),
        r##"{"t":"channel","d":{"channel":"pub","seq":991,"actor":{"id":"#123","name":"Wizard"},"title":"the Grey","kind":"say","text":[{"text":"hello, channel","tag":"channel"}],"ts":1756300000123}}"##
    );
}

#[test]
fn golden_prompt_masked_and_input_modes() {
    let ev = OutputEvent::Prompt(PromptSpec {
        mode: InputMode::Masked,
        text: Some(RichText::plain("Password: ")),
    });
    assert_eq!(
        serde_json::to_string(&ev).unwrap(),
        r#"{"t":"prompt","d":{"mode":"masked","text":[{"text":"Password: "}]}}"#
    );
    // The mode names themselves are contract: SSH password masking keys
    // off these exact strings.
    assert_eq!(
        serde_json::to_value(InputMode::Normal).unwrap(),
        json!("normal")
    );
    assert_eq!(
        serde_json::to_value(InputMode::Masked).unwrap(),
        json!("masked")
    );
    assert_eq!(
        serde_json::to_value(InputMode::NoEcho).unwrap(),
        json!("no_echo")
    );
}

#[test]
fn golden_semipose_narrative() {
    let ev = OutputEvent::Narrative {
        kind: NarrativeKind::SemiPose,
        actor: Some(wizard()),
        text: RichText::plain("'s eyes narrow."),
    };
    assert_eq!(
        serde_json::to_string(&ev).unwrap(),
        r##"{"t":"narrative","d":{"kind":"semi_pose","actor":{"id":"#123","name":"Wizard"},"text":[{"text":"'s eyes narrow."}]}}"##
    );
}

#[test]
fn golden_client_caps() {
    let caps = ClientCaps {
        kind: ClientKind::Web,
        color: ColorDepth::TrueColor,
        width: 120,
        height: 40,
        utf8: true,
        structured: true,
    };
    assert_eq!(
        serde_json::to_string(&caps).unwrap(),
        r#"{"kind":"web","color":"true_color","width":120,"height":40,"utf8":true,"structured":true}"#
    );
    assert_eq!(
        serde_json::to_string(&ClientCommand::Caps(caps)).unwrap(),
        r#"{"t":"caps","d":{"kind":"web","color":"true_color","width":120,"height":40,"utf8":true,"structured":true}}"#
    );
    // Depth names are contract for the ANSI downgrade path.
    assert_eq!(
        serde_json::to_value(ColorDepth::None).unwrap(),
        json!("none")
    );
    assert_eq!(
        serde_json::to_value(ColorDepth::Ansi16).unwrap(),
        json!("ansi16")
    );
    assert_eq!(
        serde_json::to_value(ColorDepth::Ansi256).unwrap(),
        json!("ansi256")
    );
    assert_eq!(
        serde_json::to_value(ColorDepth::TrueColor).unwrap(),
        json!("true_color")
    );
}

#[test]
fn golden_ping_pong() {
    assert_eq!(
        serde_json::to_string(&OutputEvent::Ping { nonce: 7 }).unwrap(),
        r#"{"t":"ping","d":{"nonce":7}}"#
    );
    assert_eq!(
        serde_json::to_string(&ClientCommand::Pong { nonce: 7 }).unwrap(),
        r#"{"t":"pong","d":{"nonce":7}}"#
    );
}

#[test]
fn golden_resume_flow() {
    assert_eq!(
        serde_json::to_string(&ClientCommand::Resume { last_seq: 4180 }).unwrap(),
        r#"{"t":"resume","d":{"last_seq":4180}}"#
    );
    assert_eq!(
        serde_json::to_string(&OutputEvent::Resumed { replayed: false }).unwrap(),
        r#"{"t":"resumed","d":{"replayed":false}}"#
    );
}

#[test]
fn golden_reply_wraps_payload_event() {
    let ev = OutputEvent::Reply {
        id: 3,
        event: Box::new(OutputEvent::Who(vec![])),
    };
    assert_eq!(
        serde_json::to_string(&ev).unwrap(),
        r#"{"t":"reply","d":{"id":3,"event":{"t":"who","d":[]}}}"#
    );
}

// ---------------------------------------------------------------------------
// Tolerance: additive evolution must not break older peers
// ---------------------------------------------------------------------------

#[test]
fn unknown_fields_in_payload_are_ignored() {
    let ev: OutputEvent = serde_json::from_str(
        r#"{"t":"narrative","d":{"kind":"say","text":[{"text":"hi"}],"future_field":true}}"#,
    )
    .unwrap();
    assert_eq!(
        ev,
        OutputEvent::Narrative {
            kind: NarrativeKind::Say,
            actor: None,
            text: RichText::plain("hi"),
        }
    );
}

#[test]
fn unknown_fields_beside_envelope_are_ignored() {
    let env: Envelope = serde_json::from_str(
        r#"{"seq":1,"ts":2,"t":"sys","d":[{"text":"x"}],"future_top_level":"y"}"#,
    )
    .unwrap();
    assert_eq!(
        env,
        Envelope::new(1, 2, OutputEvent::Sys(RichText::plain("x")))
    );
}

#[test]
fn optional_and_defaulted_fields_may_be_absent() {
    // Room without contents/exits/zone.
    let ev: OutputEvent = serde_json::from_str(
        r##"{"t":"room","d":{"name":"Void","id":"#0","desc":[{"text":"Nothing."}]}}"##,
    )
    .unwrap();
    let OutputEvent::Room(room) = ev else {
        panic!("expected room");
    };
    assert!(room.contents.is_empty() && room.exits.is_empty() && room.zone.is_none());

    // Hello without panels.
    let ev: OutputEvent = serde_json::from_str(
        r#"{"t":"hello","d":{"proto":1,"server":"rustmush 0.1","features":[]}}"#,
    )
    .unwrap();
    let OutputEvent::Hello { panels, proto, .. } = ev else {
        panic!("expected hello");
    };
    assert!(panels.is_empty());
    assert_eq!(proto, PROTO_VERSION);

    // Bare span: style/tag/link all default.
    let span: Span = serde_json::from_str(r#"{"text":"hi"}"#).unwrap();
    assert_eq!(span, Span::plain("hi"));
}

#[test]
fn styled_span_full_roundtrip() {
    let span = Span::tagged("exit", "north")
        .with_style(
            Style::default()
                .with_fg(Color::Idx256(45))
                .with_bg(Color::Ansi(0))
                .with_underline(),
        )
        .with_link("north");
    roundtrip(&span);
    let json = serde_json::to_value(&span).unwrap();
    assert_eq!(json["style"]["fg"], json!({"idx256": 45}));
    assert_eq!(json["link"], json!({"cmd": "north"}));
    // Booleans that are false are skipped entirely.
    assert!(json["style"].get("bold").is_none());
    assert_eq!(json["style"]["underline"], json!(true));
}

#[test]
fn request_and_respond_roundtrip() {
    use mush_proto::{
        ClientCommand, OutputEvent, RequestKind, RequestOption, RespondChoice, RichText,
    };

    // A Choice request with options and a deadline.
    let req = OutputEvent::Request {
        epoch: "test".into(),
        id: 7,
        kind: RequestKind::Choice,
        prompt: RichText::plain("Aim at whom?"),
        options: vec![
            RequestOption {
                id: "razor".into(),
                label: "Razor".into(),
            },
            RequestOption {
                id: "mara".into(),
                label: "Mara".into(),
            },
        ],
        deadline_ms: Some(30_000),
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["t"], "request");
    assert_eq!(json["d"]["kind"], "choice");
    assert_eq!(json["d"]["options"][0]["id"], "razor");
    let back: OutputEvent = serde_json::from_value(json).unwrap();
    assert_eq!(back, req);

    // A Confirm request omits options and deadline on the wire.
    let confirm = OutputEvent::Request {
        epoch: "test".into(),
        id: 8,
        kind: RequestKind::Confirm,
        prompt: RichText::plain("Spend Willpower?"),
        options: vec![],
        deadline_ms: None,
    };
    let cj = serde_json::to_value(&confirm).unwrap();
    assert!(cj["d"].get("options").is_none(), "empty options skipped");
    assert!(cj["d"].get("deadline_ms").is_none(), "no deadline skipped");
    assert_eq!(serde_json::from_value::<OutputEvent>(cj).unwrap(), confirm);

    // The four response shapes.
    for (choice, tag) in [
        (RespondChoice::Confirm(true), "confirm"),
        (RespondChoice::Choose("razor".into()), "choose"),
        (RespondChoice::Text("north gate".into()), "text"),
        (RespondChoice::Cancel, "cancel"),
    ] {
        let cmd = ClientCommand::Respond {
            epoch: "test".into(),
            id: 7,
            choice: choice.clone(),
        };
        let j = serde_json::to_value(&cmd).unwrap();
        assert_eq!(j["t"], "respond");
        assert_eq!(j["d"]["choice"]["t"], tag);
        assert_eq!(serde_json::from_value::<ClientCommand>(j).unwrap(), cmd);
    }
}

#[test]
fn notice_roundtrip() {
    use mush_proto::{NoticeAction, NoticeSeverity, OutputEvent, RichText};
    let n = OutputEvent::Notice {
        id: 3,
        severity: NoticeSeverity::Urgent,
        category: "page".into(),
        text: RichText::plain("Razor pages: meet me"),
        action: Some(NoticeAction {
            label: "Reply".into(),
            prefill: "page Razor=".into(),
        }),
        ts: 1_700_000_000_000,
    };
    let j = serde_json::to_value(&n).unwrap();
    assert_eq!(j["t"], "notice");
    assert_eq!(j["d"]["severity"], "urgent");
    assert_eq!(j["d"]["action"]["prefill"], "page Razor=");
    assert_eq!(serde_json::from_value::<OutputEvent>(j).unwrap(), n);

    // action is omitted on the wire when None.
    let n2 = OutputEvent::Notice {
        id: 4,
        severity: NoticeSeverity::Info,
        category: "mail".into(),
        text: RichText::plain("new mail"),
        action: None,
        ts: 1,
    };
    let j2 = serde_json::to_value(&n2).unwrap();
    assert!(j2["d"].get("action").is_none(), "action skipped when None");
    assert_eq!(serde_json::from_value::<OutputEvent>(j2).unwrap(), n2);
}

#[test]
fn legacy_respond_decodes_with_an_empty_epoch_for_engine_rejection() {
    let command: ClientCommand =
        serde_json::from_str(r#"{"t":"respond","d":{"id":1,"choice":{"t":"confirm","d":true}}}"#)
            .unwrap();
    assert!(matches!(command, ClientCommand::Respond { epoch, .. } if epoch.is_empty()));
}
