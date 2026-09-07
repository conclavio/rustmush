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

/// The I8 contract's F1 fixture: a `tactics.unit` keyframe, verbatim.
fn unit_keyframe() -> serde_json::Value {
    serde_json::json!({
        "object": { "id": "#7", "name": "Atlas" },
        "schema": "tacsim:unit:atlas@1#0000000000000000000000000000000000000000000000000000000000000000",
        "revision": 3,
        "state": "active",
        "record": null,
        "mobility": {
            "mode": "ground.biped",
            "switching": null,
            "modes": [
                { "id": "ground.biped", "locomotion": "legged", "mediums": ["ground"], "tags": [],
                  "nominal_speed_cms": 640, "transition_s": 0 }
            ]
        },
        "sections": [
            { "id": "center_torso", "condition": "ok", "state": "crit",
              "facets": [ { "id": "rear", "cells": ["armor"] } ],
              "slots": ["torso_hardpoint"] }
        ],
        "cells": [
            { "addr": "unit/heat",                      "label": "heat",      "current": 20, "min": 0, "max": 30, "state": "ok"   },
            { "addr": "section/center_torso/structure", "label": "structure", "current": 8,  "min": 0, "max": 8,  "state": "ok"   },
            { "addr": "facet/center_torso/rear/armor",  "label": "armor",     "current": 1,  "min": 0, "max": 6,  "state": "crit" },
            { "addr": "component/gauss/ammo",           "label": "ammo",      "current": 9,  "min": 0, "max": 12, "state": "ok"   }
        ],
        "components": [
            { "id": "gauss",
              "definition": "tacsim:component:gauss@1#0000000000000000000000000000000000000000000000000000000000000000",
              "install": "slot/center_torso/torso_hardpoint", "condition": "ok", "state": "ok", "asset": false }
        ],
        "capabilities": [ { "id": "tactics.mobility", "source": null } ],
        "counts": { "sections": 1, "cells": 4, "components": 1, "capabilities": 1 },
        "truncated": false
    })
}

/// The I8 contract's F3 fixture: the projected `@unit/damage` delta.
fn unit_delta() -> serde_json::Value {
    serde_json::json!({
        "object": { "id": "#7", "name": "Atlas" },
        "from_revision": 3, "to_revision": 4, "now": 1_757_000_000, "cause": "gauss",
        "entries": [
            { "kind": "cell",               "addr": "facet/center_torso/rear/armor", "from": 6, "to": 1 },
            { "kind": "section",            "section": "center_torso", "from": "ok", "to": "damaged" },
            { "kind": "component",          "component": "gauss", "from": "ok", "to": "destroyed" },
            { "kind": "asset",              "component": "gauss", "attached": false },
            { "kind": "capability_granted", "id": "tactics.mobility", "source": "gauss", "replaced": false },
            { "kind": "capability_revoked", "id": "tactics.mobility", "source": "gauss", "was_present": true }
        ]
    })
}

/// The fixture app with the I8 tactical frames applied, ready to paint
/// screen 8 or 9.
fn tactical_app() -> TuiApp {
    let mut app = fixture_app();
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("tactics.unit"),
        kind: PanelFrameKind::Keyframe,
        data: unit_keyframe(),
    });
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("tactics.unit"),
        kind: PanelFrameKind::Delta,
        data: unit_delta(),
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
            { "name": "Radio", "alias": "rad", "unread": 0, "gagged": false, "ic": true },
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
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("wod.vitals"),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::json!({
            "subject": { "id": "#5", "name": "Mireille" },
            "revision": 3,
            "vitals": [
                { "id": "wod.health", "label": "Health", "role": "dominant",
                  "value": { "kind": "track", "boxes": ["bashing", "lethal", null, null, null, null, null], "level": "Hurt" },
                  "state": "warn", "detail": "-1 dice" },
                { "id": "wod.willpower", "label": "Willpower", "role": "pool",
                  "value": { "kind": "bounded", "current": 6, "min": 1, "max": 10 }, "state": "ok" },
                { "id": "wod.blood", "label": "Blood", "role": "pool",
                  "value": { "kind": "bounded", "current": 3, "min": 0, "max": 13 }, "state": "crit" },
                { "id": "wod.wound", "label": "Wound", "role": "readout",
                  "value": { "kind": "text", "text": "-1" }, "state": "warn", "detail": "to every pool" }
            ]
        }),
    });
    // The I8 tactical unit feed: one keyframe (the contract's F1 fixture),
    // then the projected `@unit/damage` delta (F3) that feeds the journal
    // ring and the before/after table. Neither is a `.vitals` feed, so no
    // other screen's frame moves because of them.
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("tactics.unit"),
        kind: PanelFrameKind::Keyframe,
        data: unit_keyframe(),
    });
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("tactics.unit"),
        kind: PanelFrameKind::Delta,
        data: unit_delta(),
    });
    // The contacts table (the contract's F5 fixture): two rows, one real and
    // one phantom-backed, identical apart from the id and its prefill.
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("tactics.contacts"),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::json!({
            "schema": { "id": "tactics.contacts", "version": 1, "shape": "list",
                "columns": [ { "key": "id", "label": "contact", "align": "left" },
                             { "key": "gen", "label": "gen", "align": "right" },
                             { "key": "rev", "label": "rev", "align": "right" },
                             { "key": "state", "label": "state", "align": "left" },
                             { "key": "age", "label": "age", "align": "right" },
                             { "key": "band", "label": "band", "align": "left" },
                             { "key": "source", "label": "source", "align": "left" } ],
                "fields": [], "actionKey": "prefill" },
            "rows": [
                { "id": "aabbccddeeff", "gen": 1, "rev": 3, "state": "held", "age": "12s", "band": "close", "source": "local", "prefill": "+contact aabbccddeeff" },
                { "id": "001122334455", "gen": 1, "rev": 3, "state": "held", "age": "12s", "band": "close", "source": "local", "prefill": "+contact 001122334455" }
            ],
            "truncated": false
        }),
    });
    for (digit, name) in [
        (b'2', "sheet"),
        (b'3', "mail"),
        (b'4', "chan"),
        (b'5', "rolls"),
        (b'6', "intel"),
        (b'7', "actions"),
        (b'8', "damage"),
        (b'9', "unit"),
    ] {
        app.feed_input(&[digit]);
        let text = frame_text(&mut app, 120, 34);
        check(&format!("tui_{name}_120x34.txt"), &text);
    }
}

/// I8 (the layout hole): the before/after table's job is to show both sides
/// of a change, and at a fixed 34-column right column it could show neither
/// — the contract's own reference row is 36 characters, and `draw_pane`
/// neither wraps nor scrolls sideways. With the Right column sharing the
/// surplus, screen 8 at 120 columns reads whole rows.
#[test]
fn the_before_after_pane_shows_both_sides_at_one_twenty() {
    let mut app = tactical_app();
    app.feed_input(b"8");
    let text = frame_text(&mut app, 120, 34);
    for row in [
        "facet/center_torso/rear/armor  6 \u{2192} 1",
        "section center_torso  ok \u{2192} damaged",
        "component gauss  ok \u{2192} destroyed",
    ] {
        assert!(text.contains(row), "missing {row:?}:\n{text}");
    }
    assert!(
        text.contains("rev 4 \u{b7} gauss \u{b7} cell facet/center_torso/rear/armor 6\u{2192}1"),
        "the journal reads whole rows too:\n{text}"
    );
    // Narrower terminals keep the same three panes; the share only ever
    // moves columns Main was not using.
    let narrow = frame_text(&mut app, 80, 24);
    assert!(
        narrow.contains("before / after") && narrow.contains("journal"),
        "{narrow}"
    );
    for (w, h) in [(80u16, 24u16), (120, 34), (160, 44)] {
        let text = frame_text(&mut app, w, h);
        for line in text.lines() {
            assert!(
                line.chars().count() <= usize::from(w),
                "{w}x{h}: row too wide: {line:?}"
            );
        }
    }
}

/// I12: a page of scene history prepends ABOVE the live tail, oldest first,
/// and each recalled line renders through the same `render_event` a live one
/// did — same speaker gutter, same hanging indent. The floor marker sits at
/// the very top. The existing goldens must not move: no key was bound.
#[test]
fn narrative_history_golden_frame() {
    let mut app = fixture_app();
    // Exactly the reply `narrative_backlog_reply` builds: newest first, with
    // the correlation id 0 SSH always gets.
    app.apply_event(&OutputEvent::Reply {
        id: 0,
        event: Box::new(OutputEvent::Panel {
            feed: SmolStr::new_static("narrative"),
            kind: PanelFrameKind::Delta,
            data: serde_json::json!({ "backlog": {
                "before_seq": serde_json::Value::Null,
                "more": false,
                "evicted": true,
                "lines": [
                    { "seq": 44, "ts": 1_756_300_002_000i64, "kind": "pose",
                      "actor": { "id": "#5", "name": "Mireille" },
                      "text": RichText::plain("shuts the door on the weather and leans against it.") },
                    { "seq": 43, "ts": 1_756_300_001_000i64, "kind": "emit",
                      "actor": serde_json::Value::Null,
                      "text": RichText::plain("The lanterns gutter as the doors swing wide.") },
                    { "seq": 42, "ts": 1_756_300_000_000i64, "kind": "say",
                      "actor": { "id": "#5", "name": "Mireille" },
                      "text": RichText::plain(
                          "You are late, and the quai is already ankle deep in it — I told them \
                           on Tuesday and I will tell them again tonight.") },
                ],
            } }),
        }),
    });
    let text = frame_text(&mut app, 120, 34);
    for line in text.lines() {
        assert!(line.chars().count() <= 120, "row too wide: {line:?}");
    }
    check("tui_history_120x34.txt", &text);
}

/// I8: screen 9's compact section map puts every cell after the first on its
/// own row, so nothing is clipped off the right of the pane.
#[test]
fn the_unit_screen_keeps_every_section_cell_on_screen() {
    let mut app = tactical_app();
    app.feed_input(b"9");
    let text = frame_text(&mut app, 120, 34);
    assert!(
        text.contains(
            "\u{2717} center_torso  ok  structure \u{25b0}\u{25b0}\u{25b0}\u{25b0}\u{25b0} 8/8"
        ),
        "{text}"
    );
    assert!(
        text.contains("    rear/armor \u{25b0}\u{25b1}\u{25b1}\u{25b1}\u{25b1} 1/6"),
        "the second cell is a continuation row, not clipped:\n{text}"
    );
}
