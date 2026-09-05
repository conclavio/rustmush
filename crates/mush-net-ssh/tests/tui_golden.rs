//! Golden frames for the modal TUI (interface plan I2): the scene screen
//! with fixture data at the three budget sizes — 80×24 (scrollback, input
//! and the chrome), 120×34 (the whole scene screen), 160×44 (the HUD case).
//!
//! The frames live under `tests/golden/`; `UPDATE_GOLDEN=1 cargo test -p
//! mush-net-ssh --test tui_golden` rewrites them, and the diff is the
//! review.

use std::path::PathBuf;

use mush_net_ssh::tui::{TuiApp, TuiManifest, frame_text};
use mush_proto::{
    ClientCaps, ClientKind, ColorDepth, ExitRef, NarrativeKind, ObjRef, OutputEvent,
    PanelFrameKind, RichText, RoomView, SmolStr, Span, ViewEnvelope, WhoEntry,
};

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

fn who(name: &str, idle_s: u64, here: bool) -> WhoEntry {
    WhoEntry {
        name: name.into(),
        alias: None,
        idle_s,
        conn_s: 3600,
        location: here.then(|| "Hall".to_string()),
        here,
        doing: None,
    }
}

fn say(name: &str, text: &str) -> OutputEvent {
    OutputEvent::Narrative {
        kind: NarrativeKind::Say,
        actor: Some(ObjRef::new("#5", name)),
        text: RichText::plain(text),
    }
}

/// The scene screen with a room, people, talk, a roll, a channel line,
/// vitals, an action offer and a scene glance.
fn fixture_app() -> TuiApp {
    let mut app = TuiApp::new(caps());
    app.set_clock(|| 14 * 3600 + 30 * 60);
    app.set_manifest(TuiManifest {
        panels: vec!["wod.vitals".into(), "wod.sheet".into(), "mail".into()],
        character: Some("Nick".into()),
    });
    let mut room = RoomView {
        name: "The Hall".into(),
        id: "#2".into(),
        desc: RichText::plain(
            "A long hall of worn flagstones, lit by lanterns that gutter whenever the doors open. \
             Rain drums on the high windows.",
        ),
        contents: vec![
            ObjRef::new("#5", "Mireille"),
            ObjRef::new("#9", "a brass lamp"),
        ],
        exits: vec![
            ExitRef {
                name: "North".into(),
                aliases: vec!["n".into()],
            },
            ExitRef {
                name: "East".into(),
                aliases: vec!["e".into()],
            },
            ExitRef {
                name: "quai".into(),
                aliases: vec![],
            },
        ],
        zone: Some(ObjRef::new("#3", "Downtown")),
        ambient: Default::default(),
    };
    room.ambient.insert(
        "weather".into(),
        serde_json::json!({ "state": "rain", "line": "Overhead: steady rain, a cold wind off the water." }),
    );
    room.ambient.insert(
        "ic_time".into(),
        serde_json::json!({ "hour": 21, "minute": 15, "season": "autumn" }),
    );
    app.apply_event(&OutputEvent::Room(room));
    app.apply_event(&OutputEvent::Who(vec![
        who("Nick", 5, true),
        who("Mireille", 42, true),
        who("Bob", 900, false),
    ]));
    app.apply_event(&say(
        "Mireille",
        "The rain again. Every night this week, and the quai will be under water by morning if it keeps on.",
    ));
    app.apply_event(&OutputEvent::Narrative {
        kind: NarrativeKind::Pose,
        actor: Some(ObjRef::new("#1", "Nick")),
        text: RichText::plain("shakes the water from his coat and looks for a chair."),
    });
    app.apply_event(&OutputEvent::Narrative {
        kind: NarrativeKind::Emit,
        actor: Some(ObjRef::new("#5", "Mireille")),
        text: RichText(vec![
            Span::tagged("roll", "roll"),
            Span::plain("  "),
            Span::tagged("player", "Mireille"),
            Span::plain("  perception+alertness  8 6 4 3 = 2 successes  diff 6"),
        ]),
    });
    app.apply_event(&OutputEvent::Channel {
        channel: "pub".into(),
        seq: 12,
        actor: ObjRef::new("#7", "Zoe"),
        title: None,
        kind: NarrativeKind::Say,
        text: RichText::plain("anyone up for the docks scene later?"),
        ts: 0,
    });
    app.apply_event(&OutputEvent::Sys(RichText::tag(
        "system",
        "Bob has connected.",
    )));
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("wod.vitals"),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::json!({
            "subject": { "id": "#1", "name": "Nick" }, "revision": 3,
            "vitals": [
                { "id": "wod.health", "label": "Health", "role": "dominant",
                  "value": { "kind": "track", "boxes": ["bashing", "lethal", null, null, null, null, null], "level": "Hurt" },
                  "state": "warn", "detail": "-1" },
                { "id": "wod.willpower_temp", "label": "Willpower", "role": "pool",
                  "value": { "kind": "bounded", "current": 6, "min": 0, "max": 8 }, "state": "ok" },
                { "id": "wod.bloodpool", "label": "Blood", "role": "pool",
                  "value": { "kind": "bounded", "current": 3, "min": 0, "max": 13 }, "state": "crit" }
            ]
        }),
    });
    let actions = ViewEnvelope::keyframe(
        "legal.actions",
        1,
        7,
        3,
        serde_json::json!([
            { "offer": "opaque", "presentation": { "verb": "brace", "label": "Brace", "help": "Adopt a braced stance." } },
            { "offer": "opaque2", "presentation": { "verb": "aim", "label": "Aim", "help": "Take a moment to aim." } }
        ]),
    )
    .unwrap();
    app.apply_event(&OutputEvent::View(actions));
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("scenes"),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::json!({
            "id": 4, "title": "Rain on the Quai", "kind": "social", "mode": "open", "status": "active",
            "round": 2, "turn": 1, "current_actor": { "id": "#5", "name": "Mireille" }
        }),
    });
    app
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

fn check(name: &str, actual: &str) {
    let path = golden_path(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {}: {e}; run with UPDATE_GOLDEN=1",
            path.display()
        )
    });
    let expected = expected.replace("\r\n", "\n");
    if expected != actual {
        let mut report = format!("golden {} differs:\n", path.display());
        for (i, (e, a)) in expected.lines().zip(actual.lines()).enumerate() {
            if e != a {
                report.push_str(&format!(
                    "  row {i:>2} expected: {e}\n  row {i:>2}   actual: {a}\n"
                ));
            }
        }
        report.push_str("(UPDATE_GOLDEN=1 rewrites the files)");
        panic!("{report}");
    }
}

#[test]
fn scene_screen_golden_frames() {
    let mut app = fixture_app();
    for (w, h) in [(80u16, 24u16), (120, 34), (160, 44)] {
        let text = frame_text(&mut app, w, h);
        assert_eq!(text.lines().count(), usize::from(h));
        for line in text.lines() {
            assert!(
                line.chars().count() <= usize::from(w),
                "{w}x{h}: row too wide: {line:?}"
            );
        }
        check(&format!("tui_scene_{w}x{h}.txt"), &text);
    }
}

#[test]
fn every_screen_has_a_golden_at_the_budget_size() {
    let mut app = fixture_app();
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("mail"),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::json!({ "folder": "inbox", "entries": [
            { "index": 1, "status": "unread", "from": { "name": "Zoe" }, "subject": "Docks scene" },
            { "index": 2, "status": "read", "from": { "name": "Bob" }, "subject": "Re: the lamp" },
        ]}),
    });
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("comsys"),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::json!({ "channels": [
            { "name": "Public", "alias": "pub", "unread": 1, "gagged": false },
            { "name": "Staff", "alias": "st", "unread": 0, "gagged": false },
        ]}),
    });
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("wod.roll"),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::json!({ "who": "#5", "summary": "Mireille rolls perception+alertness (4 dice, diff 6) and rolled 8 6 4 3 -> 2 successes" }),
    });
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("intel"),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::json!({ "count": 1, "records": [
            { "subject": "Mallory", "kind": "spoke", "detail": "plotting by the quai" }
        ]}),
    });
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("wod.sheet"),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::json!({
            "target": "#1",
            "schema": { "ruleset": "wod", "version": 1, "sections": [
                { "id": "attrs", "label": "Attributes", "layout": "list", "fields": [
                    { "id": "strength", "label": "Strength", "ty": { "kind": "dots", "min": 1, "max": 5 }, "default": 1 },
                    { "id": "dexterity", "label": "Dexterity", "ty": { "kind": "dots", "min": 1, "max": 5 }, "default": 1 }
                ] },
                { "id": "pools", "label": "Pools", "layout": "list", "fields": [
                    { "id": "willpower", "label": "Willpower", "ty": { "kind": "number", "min": 1, "max": 10 }, "default": 1 },
                    { "id": "health", "label": "Health", "ty": { "kind": "track", "size": { "fixed": 7 },
                      "levels": [ { "id": "bruised", "label": "Bruised", "penalty": 0 }, { "id": "hurt", "label": "Hurt", "penalty": -1 } ] }, "default": [] }
                ] }
            ] },
            "approved": { "state": "approved", "approved": true },
            "data": { "strength": 3, "dexterity": 2, "willpower": 6, "health": ["bashing", "lethal", null, null, null, null, null] }
        }),
    });
    for (digit, name) in [
        (b'2', "sheet"),
        (b'3', "mail"),
        (b'4', "chan"),
        (b'5', "rolls"),
        (b'6', "intel"),
        (b'7', "actions"),
    ] {
        app.feed_input(&[digit]);
        let text = frame_text(&mut app, 120, 34);
        check(&format!("tui_{name}_120x34.txt"), &text);
    }
}
