//! Golden frames for the modal TUI (interface plan I2): the scene screen
//! with fixture data at the budget sizes — 120×34, the floor every screen is
//! laid out for (16c `Q-AB1`, 16b cut AB.1), and 160×44, the HUD case — and
//! the cockpit at its typical 160×48 as well. Narrower than the floor a screen
//! is one sentence and the box (`tui_floor_80x24.txt`).
//!
//! The frames live under `tests/golden/`; `UPDATE_GOLDEN=1 cargo test -p
//! mush-net-ssh --test tui_golden` rewrites them, and the diff is the
//! review.

use std::path::PathBuf;

use mush_ansi::AnsiTheme;
use mush_net_ssh::tui::panels::PanelStore;
use mush_net_ssh::tui::render::{
    PaneRow, RenderCtx, floor_line, render_cockpit_map_rows, render_cockpit_status_rows,
};
use mush_net_ssh::tui::replay::Picture;
use mush_net_ssh::tui::screen::{COCKPIT_ROWS, COCKPIT_SCREEN};
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
        room: None,
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
    for (w, h) in [(120u16, 34u16), (160, 44)] {
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

/// The floor sentence as a frame carries it: the rows above the input box,
/// trimmed, the empty ones dropped, joined with single spaces.
fn sentence_of(text: &str) -> String {
    text.lines()
        .take_while(|line| !line.starts_with('\u{250c}'))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// **Below the floor, one sentence on every screen** (16b cut AB.1, 16c
/// `Q-AB1`): the fixture app at 80 by 24 on the scene screen is the golden, and
/// every other screen — the cockpit too, on an app seated in one — paints the
/// same text there, because no screen is drawn at all. At 100 by 30 and 119 by
/// 34 the sentence names that width. A terminal as wide as the floor and one
/// row shorter than the budget paints the cockpit's panes: the floor is a
/// width, not a height.
#[test]
fn the_floor_is_one_sentence_on_every_screen() {
    let mut app = fixture_app();
    let floor = frame_text(&mut app, 80, 24);
    assert_eq!(floor.lines().count(), 24);
    assert_eq!(sentence_of(&floor), floor_line(80), "{floor}");
    check("tui_floor_80x24.txt", &floor);
    for digit in b'2'..=b'9' {
        app.feed_input(&[digit]);
        assert_eq!(
            frame_text(&mut app, 80, 24),
            floor,
            "screen {}",
            digit - b'0'
        );
    }
    let mut pilot = cockpit_app();
    assert_eq!(pilot.active_screen(), COCKPIT_SCREEN);
    assert_eq!(frame_text(&mut pilot, 80, 24), floor, "the cockpit");
    for (w, h) in [(100u16, 30u16), (119, 34)] {
        let text = frame_text(&mut pilot, w, h);
        assert_eq!(sentence_of(&text), floor_line(w), "{w}x{h}:\n{text}");
        for line in text.lines() {
            assert!(line.chars().count() <= usize::from(w), "{line:?}");
        }
    }
    let text = frame_text(&mut pilot, 120, 33);
    for title in ["status", "damage", "weapons", "contacts", "notices"] {
        assert!(
            text.contains(&format!("\u{250c}{title}")),
            "120x33: {title} is painted:\n{text}"
        );
    }
    assert!(!text.contains("needs at least"), "{text}");
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
        // Look-back I7-C16: the backlog pane frames ONE channel's messages
        // and its border read the literal word "channel". It names the
        // channel the screen is acting on — the one `b` asks backlog for.
        if name == "chan" {
            assert!(text.contains("\u{250c}Radio"), "the pane names it:\n{text}");
            assert!(!text.contains("\u{250c}channel"), "{text}");
        }
        check(&format!("tui_{name}_120x34.txt"), &text);
    }
}

/// I8 (the layout hole): the before/after table's job is to show both sides
/// of a change, and at a fixed 34-column right column it could show neither
/// — the contract's own reference row is 36 characters, and `draw_pane` then
/// clipped at the pane's edge. With the Right column sharing the surplus,
/// screen 8 at 120 columns reads whole rows, each on ONE line (since 16b cut
/// B.3 a row too long for its pane wraps rather than clips; the share is what
/// keeps these on one line).
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
    for (w, h) in [(120u16, 34u16), (160, 44)] {
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

/// Look-back LB-8: the vitals strip is ONE row and does not wrap, and a
/// tactics unit publishes up to sixteen pool cells under a feed name that
/// sorts before `wod.vitals` — so on a tacsim server the pilot's own health
/// was pushed hundreds of columns off the right edge and simply vanished.
/// The character's own feed leads now, and the cells that do not fit are
/// counted rather than dropped in silence.
#[test]
fn the_vitals_strip_paints_the_character_first_and_counts_what_it_cut() {
    let mut app = fixture_app();
    let pool = |id: &str, label: &str, current: i64, max: i64| {
        serde_json::json!({
            "id": id, "label": label, "role": "pool",
            "value": { "kind": "bounded", "current": current, "min": 0, "max": max },
            "state": "ok"
        })
    };
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("tactics.vitals"),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::json!({
            "subject": { "id": "#7", "name": "Atlas" }, "revision": 2,
            "vitals": [
                pool("unit/heat", "heat", 20, 30),
                pool("section/center_torso/structure", "structure", 8, 8),
                pool("facet/center_torso/rear/armor", "armor", 1, 6),
                pool("component/gauss/ammo", "ammo", 9, 12),
                pool("unit/fuel", "fuel", 40, 60),
                pool("unit/coolant", "coolant", 5, 5),
            ]
        }),
    });
    // At the floor (16b cut AB.1): the fixture's six pool cells still carry
    // the strip past 120 columns, so the cut and its count are what is read.
    let text = frame_text(&mut app, 120, 34);
    for line in text.lines() {
        assert!(line.chars().count() <= 120, "row too wide: {line:?}");
    }
    let strip = text
        .lines()
        .find(|l| l.contains("[ Health"))
        .unwrap_or_else(|| panic!("no vitals strip:\n{text}"));
    assert!(
        strip.starts_with("[ Health"),
        "the pilot's own dominant vital leads: {strip:?}"
    );
    assert!(
        strip.trim_end().ends_with(" more"),
        "the cut says how many cells went: {strip:?}"
    );
    assert!(
        !strip.contains("coolant"),
        "the cells past the edge are gone, not clipped mid-cell: {strip:?}"
    );
    check("tui_vitals_two_feeds_120x34.txt", &text);
}

// ---------------------------------------------------------------------------
// The cockpit screen (16b cut Q.4; doc 16 §5)
// ---------------------------------------------------------------------------
//
// One scene, committed under `web/src/lib/fixtures/cockpit/` by 16b cut Q.3 and
// read by all three renderers. Doc 16 §5 asks for renderer PARITY, and three
// renderers each pinned to a golden of its own is regression and not parity —
// so the load-bearing assertion here is not the frame goldens below but
// `the_cockpit_map_composes_the_committed_grid`: this client's composition of
// `arena_map.json` + `cockpit_frame.json` at exactly 39 by 20 equals the twenty
// rows `mush_ansi` prints of the server-side window, byte for byte.

/// The per-pulse cockpit keyframe, feed `arena.cockpit`.
const COCKPIT_FRAME: &str =
    include_str!("../../../web/src/lib/fixtures/cockpit/cockpit_frame.json");
/// The whole arena's terrain, feed `arena.map`, pulled and never pushed.
const ARENA_MAP: &str = include_str!("../../../web/src/lib/fixtures/cockpit/arena_map.json");
/// The twenty map rows a line client prints of the window, north-up.
const VIEW_GRID: &str = include_str!("../../../web/src/lib/fixtures/cockpit/view_grid.txt");

fn fixture(text: &str) -> serde_json::Value {
    serde_json::from_str(text).expect("a committed fixture is JSON")
}

fn arena_panel(feed: &'static str, text: &str) -> OutputEvent {
    OutputEvent::Panel {
        feed: SmolStr::new_static(feed),
        kind: PanelFrameKind::Keyframe,
        data: fixture(text),
    }
}

/// A render context over a store holding the committed terrain.
fn map_ctx<'a>(theme: &'a AnsiTheme, store: &'a PanelStore, pane: (u16, u16)) -> RenderCtx<'a> {
    RenderCtx {
        theme,
        depth: ColorDepth::None,
        utf8: true,
        store,
        selected_channel: None,
        pane: Some(pane),
        picture: None,
    }
}

fn store_with_the_ground() -> PanelStore {
    let mut store = PanelStore::new();
    store.apply_panel(
        &SmolStr::new_static("arena.map"),
        PanelFrameKind::Keyframe,
        &fixture(ARENA_MAP),
        0,
    );
    store
}

/// The fixture app on a server that publishes the arena feeds, with the map
/// pulled and one cockpit frame applied.
fn cockpit_app() -> TuiApp {
    let mut app = fixture_app();
    app.set_manifest(TuiManifest {
        panels: vec![
            "wod.vitals".into(),
            "wod.sheet".into(),
            "mail".into(),
            "arena.cockpit".into(),
            "arena.map".into(),
        ],
        character: Some("Nick".into()),
        room: None,
    });
    app.apply_event(&arena_panel("arena.map", ARENA_MAP));
    app.apply_event(&arena_panel("arena.cockpit", COCKPIT_FRAME));
    app
}

/// **The parity test** (doc 16 §5): the TUI's composition of the two committed
/// payloads at exactly 39 by 20 IS the grid a line client prints, one glyph a
/// cell (the line client prints each twice; `view_grid.txt` is per cell).
///
/// If this ever fails, the first differing cell is a defect in one of the three
/// compositors (or in the fixture), not a golden to re-bless.
#[test]
fn the_cockpit_map_composes_the_committed_grid() {
    let store = store_with_the_ground();
    let theme = AnsiTheme::default();
    // 39 by 22 is the PANE; the renderer keeps two rows for its heading and
    // its marker legend, so the window it carves is the fixtures' 39 by 20
    // (16b cut B.4: `VIEW` is 39 cells across; the TUI's pane draws one glyph a
    // cell, and only the line client doubles).
    let ctx = map_ctx(&theme, &store, (39, 22));
    let rows = render_cockpit_map_rows(&fixture(COCKPIT_FRAME), &ctx);
    // The heading line, then exactly twenty grid rows, then the marker legend.
    assert_eq!(rows.rows.len(), 22, "one head, twenty rows, one legend");
    let grid: String = rows
        .rows
        .iter()
        .skip(1)
        .take(20)
        .map(|row| format!("{}\n", row.text()))
        .collect();
    let expected = VIEW_GRID.replace("\r\n", "\n");
    if grid != expected {
        for (i, (want, got)) in expected.lines().zip(grid.lines()).enumerate() {
            assert_eq!(
                want, got,
                "row {i} (north-up) differs; that is a compositor defect, not a golden"
            );
        }
        panic!("the composed grid and view_grid.txt differ in length:\n{grid}");
    }
    // The pilot's own marker, the hostile and the unknown on the building: the
    // row indexes are from the SOUTH, which is `rows.rows[20 - y]`.
    let from_south = |y: usize| -> String {
        rows.rows
            .get(20 - y)
            .map(mush_net_ssh::tui::render::PaneRow::text)
            .unwrap_or_default()
    };
    assert_eq!(from_south(4).chars().nth(6), Some('@'));
    assert_eq!(from_south(14).chars().nth(36), Some('x'));
    assert_eq!(from_south(9).chars().nth(2), Some('+'));
    assert_eq!(
        from_south(7).chars().nth(14),
        Some('?'),
        "the unknown contact's marker beats the building it stands on"
    );
    assert_eq!(from_south(7).chars().nth(13), Some('#'));
    // **16b cut V.1**: the named spot `A` The Ford over the pond, and the spot
    // `B` The Roof under `X3` — drawn first, then covered: the contact owns the
    // spot's cell, so the cell reads `?`.
    assert_eq!(from_south(3).chars().nth(25), Some('A'));
    assert_eq!(
        from_south(7).chars().nth(14),
        Some('?'),
        "the contact owns the spot's cell"
    );
    // X4 plots at (50, 33), off a twenty-row window: not drawn, and not in the
    // marker legend either — it is still in the contacts pane. The spots are
    // named after the contacts, and the covered `B` not at all.
    let legend = rows.rows.last().map(|row| row.text()).unwrap_or_default();
    assert_eq!(legend, "x BA  + RB  ? X3  A The Ford", "{legend:?}");
    assert!(!legend.contains("X4"));
    assert!(!legend.contains("The Roof"));
}

/// A render context over a store holding the committed terrain with its
/// `spots` replaced (16b cut V.1).
fn store_with_spots(spots: serde_json::Value) -> PanelStore {
    let mut map = fixture(ARENA_MAP);
    map["spots"] = spots;
    let mut store = PanelStore::new();
    store.apply_panel(
        &SmolStr::new_static("arena.map"),
        PanelFrameKind::Keyframe,
        &map,
        0,
    );
    store
}

/// **16b cut V.1: a spot under the pilot's own `@` is the `@`'s.** The `@` is
/// placed after the spots and recorded in the owner map, so it owns the cell,
/// the spot beneath it is named nowhere, and the `@` itself is never listed.
#[test]
fn a_spot_under_the_pilot_s_own_marker_is_hidden_and_the_marker_is_never_listed() {
    let store = store_with_spots(serde_json::json!([
        { "letter": "A", "name": "The Ford", "cell": [25, 3] },
        { "letter": "B", "name": "Home Plate", "cell": [6, 4] },
    ]));
    let theme = AnsiTheme::default();
    let ctx = map_ctx(&theme, &store, (39, 22));
    let rows = render_cockpit_map_rows(&fixture(COCKPIT_FRAME), &ctx);
    let from_south = |y: usize| -> String {
        rows.rows
            .get(20 - y)
            .map(mush_net_ssh::tui::render::PaneRow::text)
            .unwrap_or_default()
    };
    assert_eq!(
        from_south(4).chars().nth(6),
        Some('@'),
        "the pilot's own marker covers the spot"
    );
    assert_eq!(from_south(3).chars().nth(25), Some('A'));
    let legend = rows.rows.last().map(|row| row.text()).unwrap_or_default();
    assert_eq!(legend, "x BA  + RB  ? X3  A The Ford", "{legend:?}");
    assert!(!legend.contains("Home Plate"), "{legend:?}");
    assert!(!legend.contains('@'), "{legend:?}");
}

/// **16b cut V.1: a pane too narrow for the whole legend drops the spot before
/// it drops a contact.** `fit_legend` keeps whole entries from the FRONT, and
/// the contacts come first, so `x BA` survives and `1 more` is the spot.
#[test]
fn a_narrow_legend_keeps_the_contacts_and_counts_the_spot_it_dropped() {
    let store = store_with_spots(serde_json::json!([
        { "letter": "A", "name": "The Long Ford Across The Pond", "cell": [25, 3] },
    ]));
    let theme = AnsiTheme::default();
    let ctx = map_ctx(&theme, &store, (39, 22));
    let rows = render_cockpit_map_rows(&fixture(COCKPIT_FRAME), &ctx);
    // The letter is still on the ground; only its name is cut from the legend.
    let south_3 = rows
        .rows
        .get(20 - 3)
        .map(mush_net_ssh::tui::render::PaneRow::text)
        .unwrap_or_default();
    assert_eq!(south_3.chars().nth(25), Some('A'));
    let legend = rows.rows.last().map(|row| row.text()).unwrap_or_default();
    assert_eq!(legend, "x BA  + RB  ? X3  1 more", "{legend:?}");
    assert!(legend.chars().count() <= 39);
}

/// The composition at a NON-ZERO window origin, which the committed fixture
/// cannot catch (its own origin is `[0, 0]`): a marker's `cell` is ABSOLUTE and
/// a compositor must subtract the window's corner from it.
#[test]
fn a_window_away_from_the_corner_subtracts_its_own_origin() {
    let store = store_with_the_ground();
    let theme = AnsiTheme::default();
    // Small enough that the platform's cell does not clamp to a corner; two
    // rows of the twelve are the pane's own chrome, so the window is 20 by 10.
    let ctx = map_ctx(&theme, &store, (20, 12));
    let mut frame = fixture(COCKPIT_FRAME);
    frame["cell"] = serde_json::json!([60, 40]);
    frame["contacts"] = serde_json::json!([
        { "designator": "BA", "class": "blue", "friendly": false, "cell": [55, 38],
          "bearing_deg": 45, "band": "close", "range_m": 100, "age_s": 1, "condition": null },
        { "designator": "X9", "class": "unknown", "friendly": false, "cell": [6, 4],
          "bearing_deg": 0, "band": "far", "range_m": 900, "age_s": 9, "condition": null }
    ]);
    let rows = render_cockpit_map_rows(&frame, &ctx);
    assert_eq!(rows.rows.len(), 12, "one head, ten rows, one legend");
    let from_south = |y: usize| -> String {
        rows.rows
            .get(10 - y)
            .map(mush_net_ssh::tui::render::PaneRow::text)
            .unwrap_or_default()
    };
    // x0 = 60 - 20/2 = 50, y0 = 40 - 10/2 = 35, both inside the 96x64 arena.
    assert_eq!(
        from_south(5).chars().nth(10),
        Some('@'),
        "the pilot at 60,40"
    );
    assert_eq!(
        from_south(3).chars().nth(5),
        Some('x'),
        "the contact at 55,38 draws at 5,3 of the window"
    );
    let legend = rows.rows.last().map(|row| row.text()).unwrap_or_default();
    assert_eq!(
        legend, "x BA",
        "the contact back at the arena's corner is off this window"
    );
}

/// The Left column of a painted row: its first 34 glyphs (16b cut AB.1: the
/// status and damage panes' column at every width from the floor).
fn left_of(line: &str) -> String {
    line.chars().take(34).collect()
}

/// The Right column of a painted row at `w` columns: its last
/// `46 + (w - 116) / 2` glyphs (the column the weapons, contacts and notices
/// panes share, with its half of the surplus).
fn right_of(line: &str, w: u16) -> String {
    let right = usize::from(46 + (w - 116) / 2);
    let chars: Vec<char> = line.chars().collect();
    let at = usize::from(w).saturating_sub(right);
    chars.iter().skip(at).collect()
}

/// Whether some ONE painted row holds `needle` inside its Left column.
fn left_holds(text: &str, needle: &str) -> bool {
    text.lines().any(|line| left_of(line).contains(needle))
}

/// Whether some ONE painted row holds `needle` inside its Right column.
fn right_holds(text: &str, needle: &str, w: u16) -> bool {
    text.lines().any(|line| right_of(line, w).contains(needle))
}

/// The whole screen at the budget sizes, on a server that publishes the arena
/// feeds. The FIRST cockpit frame of a match opens the screen by itself, which
/// is what the `active_screen` assertion pins.
///
/// **16b cut AB.1** (16c `Q-AB1`, superseding cut B.3's 80 by 24 and 100 by 30):
/// the floor, 120 by 34, and the typical terminal, 160 by 48, each a golden;
/// and 120 by 30 (`COCKPIT_ROWS`), where every pane is still painted and the
/// legend ends on `? help`. At all three every row is on ONE line of its
/// column: the five sections start rows of the Left column with each facet
/// cell whole, the contact rows sit whole in the Right column (all four at the
/// budget sizes; at 120 by 30 the contacts pane's five inner rows hold three,
/// with X4 below the fold), and the notices pane — on the Right since the cut,
/// under the contacts — is the frame's two notices composed from their fields
/// and none of the scrollback's chatter. One row under `COCKPIT_ROWS` the
/// damage pane goes first, for height.
#[test]
fn the_cockpit_screen_paints_at_both_budget_sizes() {
    let mut app = cockpit_app();
    assert_eq!(
        app.active_screen(),
        COCKPIT_SCREEN,
        "the first frame of a match opens the screen"
    );
    for (w, h) in [(120u16, 34u16), (120, COCKPIT_ROWS), (160, 48)] {
        let text = frame_text(&mut app, w, h);
        assert_eq!(text.lines().count(), usize::from(h));
        for line in text.lines() {
            assert!(
                line.chars().count() <= usize::from(w),
                "{w}x{h}: row too wide: {line:?}"
            );
        }
        for title in ["status", "damage", "weapons", "contacts", "notices"] {
            assert!(
                text.contains(&format!("\u{250c}{title}")),
                "{w}x{h}: {title} is painted:\n{text}"
            );
        }
        assert!(text.contains("\u{250c} map "), "{w}x{h}: the map:\n{text}");
        // The legend ends on `? help`, never mid-word.
        let legend = text.lines().last().unwrap_or_default();
        assert!(legend.ends_with("? help"), "{w}x{h}: {legend:?}");
        // The five sections, each starting a row of the Left column.
        for section in ["engine", "hull", "left_track", "right_track", "turret"] {
            assert!(
                text.lines()
                    .any(|line| left_of(line).starts_with(&format!("\u{2502}{section}  "))),
                "{w}x{h}: {section} starts no row of the Left column:\n{text}"
            );
        }
        // Every facet cell whole on one row.
        for cell in [
            "front 55/80",
            "rear 30/30",
            "left 4/24",
            "right 0/24",
            "top 18/20",
        ] {
            assert!(
                left_holds(&text, cell),
                "{w}x{h}: {cell:?} is split:\n{text}"
            );
        }
        // Every contact row whole in the Right column. At 120 by 30 the Right
        // column's even split (8/7/8) leaves the contacts pane five inner
        // rows, and the fixture's contacts take six lines (BA's SCAN row
        // wraps once at 46 columns), so the last contact, X4, is below the
        // fold there: the pane is not focused, so nothing scrolls it into
        // view. The rows above it are whole.
        let x4 = "X4  unknown  030\u{b0}  far  600 m  6 s";
        for row in [
            "BA  blue  045\u{b0}  far  450 m  1 s",
            "RB  red  270\u{b0}  close  100 m  0 s",
            "X3  unknown  315\u{b0}  effective  200 m  3 s",
            x4,
        ] {
            if h == COCKPIT_ROWS && row == x4 {
                assert!(
                    !right_holds(&text, row, w),
                    "{w}x{h}: X4's row was expected below the fold:\n{text}"
                );
                continue;
            }
            assert!(
                right_holds(&text, row, w),
                "{w}x{h}: {row:?} is not whole in the Right column:\n{text}"
            );
        }
        // The ring, under `┌notices` in the Right column.
        let notices_at = text
            .lines()
            .position(|line| right_of(line, w).starts_with("\u{250c}notices"))
            .unwrap_or_else(|| panic!("{w}x{h}: no notices pane on the Right:\n{text}"));
        for line in ["X4 new contact", "BA section destroyed: turret"] {
            assert!(
                text.lines()
                    .skip(notices_at)
                    .any(|row| right_of(row, w).contains(line)),
                "{w}x{h}: the ring's {line:?}:\n{text}"
            );
        }
        assert!(
            !text.contains("Bob has connected") && !text.contains("anyone up"),
            "{w}x{h}: the scrollback's chatter is not the notices pane:\n{text}"
        );
    }
    // One row under `COCKPIT_ROWS`: damage goes first, the rest stand.
    let text = frame_text(&mut app, 120, 29);
    assert!(!text.contains("\u{250c}damage"), "{text}");
    for title in ["status", "weapons", "contacts", "notices"] {
        assert!(
            text.contains(&format!("\u{250c}{title}")),
            "{title}:\n{text}"
        );
    }
    assert!(text.contains("\u{250c} map "), "{text}");
    for (w, h) in [(120u16, 34u16), (160, 48)] {
        let text = frame_text(&mut app, w, h);
        check(&format!("tui_cockpit_{w}x{h}.txt"), &text);
    }
}

/// **The status pane is painted whole at the floor** (16b cut AB.1; cut B.3's
/// `B-10` at eighty columns before it): a jammed receiver, a wreck (so the
/// speed line carries `DESTROYED`) and a lock, at 120 by 34 — the golden. Then
/// the FULLEST block the frame can send, at the floor and at 160 by 48: an
/// overheated platform and a lock past a kilometre at 100% on top of the rest.
/// At the Left column's 32 inner columns not one of the seven rows wraps, so
/// each is read on ONE line of the column; the Lock is the LAST row, the one a
/// short pane would lose and the one a gunner reads before `FIRE`. The golden
/// is the review; the assertions are the proof the status pane's `min_w` 34
/// and `min_h` 7 are right.
#[test]
fn the_jammed_wreck_s_status_is_painted_whole_at_the_floor() {
    let arena_app = |frame: serde_json::Value| {
        let app = cockpit_app_with(frame);
        assert_eq!(app.active_screen(), COCKPIT_SCREEN);
        app
    };
    let mut frame = fixture(COCKPIT_FRAME);
    frame["receiver"] = serde_json::json!({ "budget": 15, "budget_max": 40, "reach_percent": 60 });
    frame["wreck"] = serde_json::json!(true);
    let mut app = arena_app(frame.clone());
    let text = frame_text(&mut app, 120, 34);
    for row in [
        "Callsign RA",
        "power online",
        "43 kph  heading 045\u{b0}  DESTROYED",
        "heat 40/100",
        "receiver 15/40  60%",
        "scope hostile",
        "Lock BA  440 m  62%",
    ] {
        assert!(
            left_holds(&text, row),
            "{row:?} is not whole on one row at 120x34:\n{text}"
        );
    }
    for line in text.lines() {
        assert!(line.chars().count() <= 120, "row too wide: {line:?}");
    }
    check("tui_cockpit_120x34_jammed.txt", &text);

    // The fullest block: every row at its longest.
    frame["heat"]["heat"] = serde_json::json!(100);
    frame["heat"]["overheated"] = serde_json::json!(true);
    frame["lock_range_m"] = serde_json::json!(1440);
    frame["lock_percent"] = serde_json::json!(100);
    let mut app = arena_app(frame);
    for (w, h) in [(120u16, 34u16), (160, 48)] {
        let text = frame_text(&mut app, w, h);
        for row in [
            "Callsign RA",
            "power online",
            "43 kph  heading 045\u{b0}  DESTROYED",
            "heat 100/100  OVERHEATED",
            "receiver 15/40  60%",
            "scope hostile",
            "Lock BA  1440 m  100%",
        ] {
            assert!(
                left_holds(&text, row),
                "{w}x{h}: {row:?} is not whole on one row of the status pane:\n{text}"
            );
        }
    }
}

/// The shipped line tank's five sections as the frame carries them
/// (`crates/tacsim/content/units/line_tank.json`, undamaged): the hull's five
/// facets and the turret's four at three digits, one facet on each track, none
/// on the engine.
fn line_tank_sections() -> serde_json::Value {
    serde_json::json!([
        { "id": "hull", "condition": "intact", "internals": 200, "internals_max": 200,
          "facets": [
            { "id": "front", "armor": 400, "armor_max": 400 },
            { "id": "left", "armor": 250, "armor_max": 250 },
            { "id": "right", "armor": 250, "armor_max": 250 },
            { "id": "rear", "armor": 150, "armor_max": 150 },
            { "id": "top", "armor": 100, "armor_max": 100 } ] },
        { "id": "turret", "condition": "intact", "internals": 150, "internals_max": 150,
          "facets": [
            { "id": "front", "armor": 350, "armor_max": 350 },
            { "id": "left", "armor": 220, "armor_max": 220 },
            { "id": "right", "armor": 220, "armor_max": 220 },
            { "id": "rear", "armor": 140, "armor_max": 140 } ] },
        { "id": "left_track", "condition": "intact", "internals": 100, "internals_max": 100,
          "facets": [ { "id": "left", "armor": 120, "armor_max": 120 } ] },
        { "id": "right_track", "condition": "destroyed", "internals": 100, "internals_max": 100,
          "facets": [ { "id": "right", "armor": 120, "armor_max": 120 } ] },
        { "id": "engine", "condition": "intact", "internals": 120, "internals_max": 120,
          "facets": [] }
    ])
}

/// **The shipped tank's damage is whole at the floor** (16b cut AB.1; the B
/// row's residual that at 120 by 34 the damage pane showed three of the five
/// sections): the cockpit frame with its sections replaced by the line tank's,
/// the stock roster's, painted at 120 by 34 and at 120 by 30 (`COCKPIT_ROWS`,
/// Windows Terminal's default size). Every section row and every facet cell is
/// inside the Left column, whole on one row.
#[test]
fn a_line_tank_s_damage_is_whole_at_the_floor() {
    let mut frame = fixture(COCKPIT_FRAME);
    frame["sections"] = line_tank_sections();
    let mut app = cockpit_app_with(frame);
    for h in [34u16, 30] {
        let text = frame_text(&mut app, 120, h);
        for row in [
            "hull  intact  200/200",
            "turret  intact  150/150",
            "left_track  intact  100/100",
            "right_track  destroyed  100/100",
            "engine  intact  120/120",
        ] {
            assert!(
                text.lines()
                    .any(|line| left_of(line).starts_with(&format!("\u{2502}{row}"))),
                "120x{h}: {row:?} starts no row of the Left column:\n{text}"
            );
        }
        for cell in [
            "front 400/400",
            "left 250/250",
            "right 250/250",
            "rear 150/150",
            "top 100/100",
            "front 350/350",
            "left 220/220",
            "right 220/220",
            "rear 140/140",
            "left 120/120",
            "right 120/120",
        ] {
            assert!(
                left_holds(&text, cell),
                "120x{h}: {cell:?} is not whole in the Left column:\n{text}"
            );
        }
    }
}

/// The committed catch-up cases (16b cut Y.4), the parity fixture all three
/// renderers read.
const CATCHUP_LINES: &str =
    include_str!("../../../web/src/lib/fixtures/cockpit/catchup_lines.json");

/// The fixture app with one cockpit frame of `frame` applied.
fn cockpit_app_with(frame: serde_json::Value) -> TuiApp {
    let mut app = fixture_app();
    app.set_manifest(TuiManifest {
        panels: vec![
            "wod.vitals".into(),
            "wod.sheet".into(),
            "mail".into(),
            "arena.cockpit".into(),
            "arena.map".into(),
        ],
        character: Some("Nick".into()),
        room: None,
    });
    app.apply_event(&arena_panel("arena.map", ARENA_MAP));
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("arena.cockpit"),
        kind: PanelFrameKind::Keyframe,
        data: frame,
    });
    app
}

/// **A reclaim's catch-up rings before the frame's notices** (16b cut Y.4; doc
/// 07 §1.3 step 5): the fixture cockpit with the fixture's FULL catch-up on it
/// rings the report's seven lines — composed from the object's fields,
/// `catchup_words` — and then the frame's two notices. A second frame carrying
/// `null` rings nothing of it again.
///
/// The golden is the same frame with its two notices taken off — a reclaim
/// whose first pulse said nothing else — because the notices pane shows the
/// ring's TAIL, and with the notices on it the screen is
/// `tui_cockpit_120x34.txt` all but a few rows and pins little of the report.
/// At the floor (16b cut AB.1) the pane is seven rows at 46 columns and the
/// report wants nine (the machine's line and the `Sections:` line each wrap
/// once), so the pane shows the whole lines of the tail that fit — six rows —
/// and the first two lines are cut from the top; at 160 by 48 the pane's rows
/// at 66 columns hold the whole report.
#[test]
fn a_reclaims_catch_up_rings_before_the_frames_notices() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(CATCHUP_LINES).expect("catchup_lines.json is an array");
    let full = cases.first().expect("the full case");
    let lines: Vec<String> = full
        .get("lines")
        .and_then(serde_json::Value::as_array)
        .expect("lines")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect();
    let mut frame = fixture(COCKPIT_FRAME);
    frame["catchup"] = full.get("catchup").cloned().expect("a catchup object");
    let mut app = cockpit_app_with(frame.clone());
    let mut rung: Vec<String> = app.panels().cockpit_notices().iter().cloned().collect();
    let mut want = lines.clone();
    want.extend([
        "X4 new contact".to_string(),
        "BA section destroyed: turret".to_string(),
    ]);
    assert_eq!(rung, want);
    // The next frame is the committed one, `catchup: null`: its notices only.
    app.apply_event(&arena_panel("arena.cockpit", COCKPIT_FRAME));
    rung = app.panels().cockpit_notices().iter().cloned().collect();
    assert_eq!(
        rung.iter()
            .filter(|line| line.starts_with("Back aboard"))
            .count(),
        1,
        "{rung:?}"
    );

    // The golden: the report alone in the ring, its tail in the pane.
    frame["notices"] = serde_json::json!([]);
    let mut app = cockpit_app_with(frame);
    assert_eq!(
        app.panels()
            .cockpit_notices()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        lines
    );
    let text = frame_text(&mut app, 120, 34);
    for line in text.lines() {
        assert!(line.chars().count() <= 120, "row too wide: {line:?}");
    }
    // The tail that fits whole lines, in the Right column: the position, the
    // contacts, the sections (two lines at 46 columns), the rounds and the
    // power — six of the pane's seven rows. The machine's line (two rows)
    // does not fit above them, and the head is cut with it.
    for shown in [
        "  Position: cell (64, 97) then, (70, 101) now.",
        "  Contacts: new BA, X3; lost X2.",
        "  Sections: left_track intact then, destroyed",
        "now; hull armour 230 then, 90 now.",
        "  Rounds spent: cannon 3, machine_gun 40.",
        "  Power online.",
    ] {
        assert!(
            right_holds(&text, shown, 120),
            "120x34: the report's tail, {shown:?}:\n{text}"
        );
    }
    for cut in ["Back aboard RA after", "The machine was withdrawing"] {
        assert!(
            !text.contains(cut),
            "120x34: {cut:?} is cut from the top:\n{text}"
        );
    }
    check("tui_cockpit_120x34_catchup.txt", &text);
    // At the typical size the whole report is on the pane.
    let text = frame_text(&mut app, 160, 48);
    assert!(
        right_holds(&text, "Back aboard RA after 2 m 14 s away.", 160),
        "160x48: the report whole:\n{text}"
    );
    assert!(right_holds(&text, "Power online.", 160), "{text}");
}

/// **A malformed catch-up rings nothing** (16b cut Y.4): a string, a number
/// or an array where the object belongs is not a report, and the frame's own
/// notices are all the ring gets.
#[test]
fn a_malformed_catch_up_rings_nothing() {
    for bad in [
        serde_json::json!("Back aboard RA after 1 s away."),
        serde_json::json!(7),
        serde_json::json!([1, 2]),
    ] {
        let mut frame = fixture(COCKPIT_FRAME);
        frame["catchup"] = bad.clone();
        let app = cockpit_app_with(frame);
        let rung: Vec<String> = app.panels().cockpit_notices().iter().cloned().collect();
        assert_eq!(
            rung,
            vec![
                "X4 new contact".to_string(),
                "BA section destroyed: turret".to_string()
            ],
            "{bad}"
        );
    }
}

/// The tenth tab exists only where the feed does, and the screen reads the one
/// frame: the callsign, the lock, the sections, the weapons and the contacts,
/// with nothing on it that the frame did not carry.
#[test]
fn the_cockpit_screen_reads_the_frame_and_nothing_else() {
    let mut app = cockpit_app();
    let text = frame_text(&mut app, 120, 34);
    assert!(text.contains("0 cockpit"), "the tenth tab:\n{text}");
    for needle in [
        "Callsign RA",
        "power online",
        "43 kph",
        "heat 40/100",
        // 16b cut D.2 (16c `T-59`): this platform's own scope, where the arena's
        // one global cadence word used to be.
        "scope hostile",
        // 16b cut D.4 (16c `T-20`): the lock's range is rounded to what the
        // plot's own age can carry, and the shared scene's plot is 1,200 ms old
        // — 20 m of drift, so a 20 m step, and 430 rounds half up to 440.
        "Lock BA  440 m  62%",
        "cannon  ready  14/20  62%",
    ] {
        assert!(text.contains(needle), "missing {needle:?}:\n{text}");
    }
    // The contacts pane lists the contact the map could not draw.
    assert!(
        text.contains("X4"),
        "the off-window contact is listed:\n{text}"
    );
    // Nothing on the screen names an object or a unit: the frame carries the
    // unit number and the sixteen-digit contact ids, and neither is printed.
    //
    // The unit needle is read OUT of the fixture and asserted in the form the
    // frame carries it — `"unit": 101`, a JSON number — as well as with the
    // `#` a dbref would wear. A needle for only the `#` form could not fail:
    // no renderer would produce it out of a number, and the obvious accident
    // (printing the field as it stands) would sail past.
    let unit = fixture(COCKPIT_FRAME)
        .get("unit")
        .and_then(serde_json::Value::as_i64)
        .expect("the fixture carries the platform's object number");
    for leak in [
        format!("#{unit}"),
        unit.to_string(),
        "1111111111111111".to_string(),
        "4444444444444444".to_string(),
    ] {
        assert!(
            !text.contains(&leak),
            "the cockpit leaked {leak:?}:\n{text}"
        );
    }
}

// ---------------------------------------------------------------------------
// 16b cut I.2 — the replay on screen 0 (16c N9, Q37)
// ---------------------------------------------------------------------------
//
// One replay, read by every renderer: `web/src/lib/fixtures/replay/`, whose two
// JSON files 16b cut I.1's blesser writes (`crates/tacsim/tests/replay.rs`) and
// whose `replay_grid.txt` `mush-ansi`'s golden blesses from the line client's
// print of the window. The replay is match 7 on the cockpit fixtures' yard:
// four tanks — RA and RB red, BA and BB blue — three keyframes at 0, 5,000 and
// 10,000 ms, BB wrecked at 7,000 on The Roof, two shots.

/// The grid `SIM REPLAY 7` pushes at keyframe 1 (focus RA).
const REPLAY_WINDOW: &str = include_str!("../../../web/src/lib/fixtures/replay/replay_window.json");
/// The whole replay document.
const REPLAY_DOCUMENT: &str = include_str!("../../../web/src/lib/fixtures/replay/replay.json");
/// The twenty map rows a line client prints of the window, one glyph a cell.
const REPLAY_GRID: &str = include_str!("../../../web/src/lib/fixtures/replay/replay_grid.txt");

/// The document as the `arena.replay` query answers it: under its own shape.
fn replay_answer() -> serde_json::Value {
    let mut data = fixture(REPLAY_DOCUMENT);
    data["schema"] = serde_json::json!({ "id": "arena.replay", "version": 1, "shape": "replay" });
    data
}

/// The fixture app with a replay of match 7 open on screen 0: no seat, the
/// grid pushed, the document and the ground held, at keyframe 1.
fn replay_app() -> TuiApp {
    let mut app = fixture_app();
    app.set_manifest(TuiManifest {
        panels: vec![
            "wod.vitals".into(),
            "wod.sheet".into(),
            "mail".into(),
            "arena.cockpit".into(),
            "arena.map".into(),
            "arena.replay".into(),
        ],
        character: Some("Nick".into()),
        room: None,
    });
    app.apply_event(&arena_panel("arena.replay", REPLAY_WINDOW));
    // The cockpit ask screen 0 makes is answered the cleared frame.
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("arena.cockpit"),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::json!({
            "schema": { "id": "arena.cockpit", "version": 1, "retain": "latest" },
            "seated": false
        }),
    });
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("arena.replay"),
        kind: PanelFrameKind::Keyframe,
        data: replay_answer(),
    });
    app.apply_event(&arena_panel("arena.map", ARENA_MAP));
    app
}

/// **The parity** (16b cut I.2; doc 16 §5): the TUI's replay map at a 39 by 22
/// pane over `arena_map.json` and `replay.json` at keyframe 1, focus RA, IS the
/// grid the line client prints of `replay_window.json`, one glyph a cell — and
/// the web's `composeReplayWindow` is held to the same file. A difference is a
/// compositor defect, never a golden to re-bless.
#[test]
fn i2_the_replay_map_composes_the_committed_grid() {
    let mut store = store_with_the_ground();
    store.apply_panel(
        &SmolStr::new_static("arena.replay"),
        PanelFrameKind::Keyframe,
        &replay_answer(),
        0,
    );
    let theme = AnsiTheme::default();
    let picture = Picture {
        match_id: 7,
        at: 1,
        focus: 0,
        live: false,
    };
    let ctx = RenderCtx {
        picture: Some(&picture),
        ..map_ctx(&theme, &store, (39, 22))
    };
    // A cleared cockpit frame: the replay is what an unseated screen 0 draws.
    let rows = render_cockpit_map_rows(&serde_json::json!({ "seated": false }), &ctx);
    assert_eq!(rows.rows.len(), 22, "one head, twenty rows, one legend");
    assert_eq!(
        rows.rows.first().map(PaneRow::text).as_deref(),
        Some("Replay 7 \u{2014} 0:05 (2/3)")
    );
    let grid: String = rows
        .rows
        .iter()
        .skip(1)
        .take(20)
        .map(|row| format!("{}\n", row.text()))
        .collect();
    let expected = REPLAY_GRID.replace("\r\n", "\n");
    if grid != expected {
        for (i, (want, got)) in expected.lines().zip(grid.lines()).enumerate() {
            assert_eq!(
                want, got,
                "row {i} (north-up) differs; that is a compositor defect, not a golden"
            );
        }
        panic!("the composed grid and replay_grid.txt differ in length:\n{grid}");
    }
    // The owner rule: BB stands on The Roof and owns its cell, so `B` is named
    // nowhere; every unit is named in roster order, then the spot it left.
    let legend = rows.rows.last().map(PaneRow::text).unwrap_or_default();
    assert_eq!(legend, "o RA  o RB  * BA  * BB  A The Ford", "{legend:?}");

    // The last keyframe of a whole replay answers to the end (10,000 ms here),
    // so BB — wrecked at 7,000 — is drawn `%` and named `% BB`, still owning
    // the spot's cell; no heading row wraps.
    let last = Picture { at: 2, ..picture };
    let ctx = RenderCtx {
        picture: Some(&last),
        ..map_ctx(&theme, &store, (39, 22))
    };
    let rows = render_cockpit_map_rows(&serde_json::json!({ "seated": false }), &ctx);
    let from_south =
        |y: usize| -> String { rows.rows.get(20 - y).map(PaneRow::text).unwrap_or_default() };
    assert_eq!(from_south(7).chars().nth(14), Some('%'), "BB's wreck");
    assert_eq!(from_south(4).chars().nth(9), Some('o'), "RA at (9, 4)");
    let legend = rows.rows.last().map(PaneRow::text).unwrap_or_default();
    assert_eq!(legend, "o RA  o RB  * BA  % BB  A The Ford", "{legend:?}");
    // Without UTF-8 the heading's dash degrades.
    let ascii = RenderCtx {
        utf8: false,
        picture: Some(&last),
        ..map_ctx(&theme, &store, (39, 22))
    };
    let rows = render_cockpit_map_rows(&serde_json::json!({ "seated": false }), &ascii);
    assert_eq!(
        rows.rows.first().map(PaneRow::text).as_deref(),
        Some("Replay 7 - 0:10 (3/3)")
    );
}

/// **The status pane of a CUT replay** (16b cut I.2): the `Cut:` lines are
/// rendered by `render_cockpit_status_rows` itself, not typed out — one per
/// `cut` word in the document's order, with the clocks read off the document
/// (the last keyframe's for `keyframes`, the last shot's for `shots`, the
/// roster's length for `roster`), a `truncated` document naming no cause read
/// as cut at its keyframes, and none at all for a whole one. Every line, at a
/// four-digit match id, the longest outcome words, an eight-letter focus and
/// an hour-long clock, sits inside the Left pane's 32 columns, so none wraps
/// at the floor.
#[test]
fn i2_the_status_pane_words_every_cut_inside_the_left_pane() {
    let theme = AnsiTheme::default();
    let picture = Picture {
        match_id: 9999,
        at: 2,
        focus: 0,
        live: false,
    };
    let status = |doc: serde_json::Value| -> Vec<String> {
        let mut store = store_with_the_ground();
        store.apply_panel(
            &SmolStr::new_static("arena.replay"),
            PanelFrameKind::Keyframe,
            &doc,
            0,
        );
        let ctx = RenderCtx {
            picture: Some(&picture),
            ..map_ctx(&theme, &store, (32, 12))
        };
        render_cockpit_status_rows(&serde_json::json!({ "seated": false }), &ctx)
            .rows
            .iter()
            .map(PaneRow::text)
            .collect()
    };
    let mut doc = replay_answer();
    doc["match"] = serde_json::json!(9999);
    doc["outcome"] = serde_json::json!("blue");
    doc["reason"] = serde_json::json!("elimination");
    doc["roster"][0]["callsign"] = serde_json::json!("ABCDEFGH");
    doc["keyframes"][2][0] = serde_json::json!(3_680_000);
    doc["end_ms"] = serde_json::json!(3_690_000);
    doc["shots"][1][0] = serde_json::json!(3_599_000);
    doc["truncated"] = serde_json::json!(true);
    doc["cut"] = serde_json::json!(["roster", "keyframes", "shots", "page"]);
    let head = [
        "Match 9999  blue won (red elim.)",
        "Keyframe 3/3  61:20 of 61:30",
        "Focus ABCDEFGH  (u next)",
        "Arena positions, every 5 s.",
        "Both sides shown: match over.",
    ];
    let cut = status(doc.clone());
    let mut want: Vec<String> = head.iter().map(ToString::to_string).collect();
    want.extend(
        [
            "Cut: first 4 units kept.",
            "Cut: first 61:20 kept.",
            "Cut: shots to 59:59 kept.",
            "Cut: a page of tape lost.",
        ]
        .map(String::from),
    );
    assert_eq!(cut, want);
    for line in &cut {
        assert!(
            line.chars().count() <= 32,
            "{line:?} wraps in the Left pane"
        );
    }
    // Truncated, naming no cause: cut at the keyframes, one line.
    doc["cut"] = serde_json::json!([]);
    let unnamed = status(doc.clone());
    assert_eq!(unnamed.len(), head.len() + 1, "{unnamed:?}");
    assert_eq!(
        unnamed.last().map(String::as_str),
        Some("Cut: first 61:20 kept.")
    );
    // Whole: no `Cut:` line at all.
    doc["truncated"] = serde_json::json!(false);
    let whole = status(doc);
    assert_eq!(whole, head.map(String::from).to_vec());
}

/// **The picture on screen 0 at the budget sizes** (16b cut I.2): the layout is
/// the cockpit's — the same six panes in the same columns — and every row is
/// within the width, exactly `h` rows; at 120 the legend is the picture's five
/// entries whole and `Enter type` and `? help` after them; no status sentence
/// wraps in the Left column; both sides are shown and the screen says so, and
/// nothing names a pilot or an object. The goldens are the review.
#[test]
fn i2_the_replay_screen_paints_at_both_budget_sizes() {
    let mut app = replay_app();
    assert_eq!(
        app.active_screen(),
        COCKPIT_SCREEN,
        "the grid opened screen 0"
    );
    for (w, h) in [(120u16, 34u16), (160, 48)] {
        let text = frame_text(&mut app, w, h);
        assert_eq!(text.lines().count(), usize::from(h));
        for line in text.lines() {
            assert!(
                line.chars().count() <= usize::from(w),
                "{w}x{h}: row too wide: {line:?}"
            );
        }
        // The cockpit's layout: the same panes, the same columns.
        for title in ["status", "damage", "weapons", "contacts", "notices"] {
            assert!(
                text.contains(&format!("\u{250c}{title}")),
                "{w}x{h}: {title} is painted:\n{text}"
            );
        }
        assert!(text.contains("\u{250c} map "), "{w}x{h}: the map:\n{text}");
        assert!(text.contains("0 cockpit"), "{w}x{h}: the tab:\n{text}");
        // Every status sentence whole on ONE row of the Left column.
        for sentence in [
            "Match 7  aborted",
            "Keyframe 2/3  0:05 of 0:10",
            "Focus RA  (u next)",
            "Arena positions, every 5 s.",
            "Both sides shown: match over.",
        ] {
            assert!(
                left_holds(&text, sentence),
                "{w}x{h}: {sentence:?} is not whole in the Left column:\n{text}"
            );
        }
        // The map pane's own heading and legend. At the floor the map is 36
        // cells across and BA, at (36, 14), is off it; at 160 it is drawn.
        assert!(text.contains("Replay 7 \u{2014} 0:05 (2/3)"), "{text}");
        let legend = if w == 120 {
            "o RA  o RB  * BB  A The Ford"
        } else {
            "o RA  o RB  * BA  * BB  A The Ford"
        };
        assert!(text.contains(legend), "{w}x{h}: {legend:?}:\n{text}");
        // Both sides' callsigns in the contacts pane, whole, in roster order.
        for row in [
            "RA   red   (6, 4)  090\u{b0}",
            "RB   red   (2, 9)  000\u{b0}",
            "BA   blue  (36, 14)  270\u{b0}",
            "BB   blue  (14, 7)  225\u{b0}",
        ] {
            assert!(
                right_holds(&text, row, w),
                "{w}x{h}: {row:?} is not whole in the Right column:\n{text}"
            );
        }
        // Keyframe 1 is after the first keyframe and before any shot or wreck.
        assert!(left_holds(&text, "Nothing yet."), "{text}");
        // The picture's legend, whole, then `Enter type` and `? help`.
        let legend = text.lines().last().unwrap_or_default();
        assert_eq!(
            legend,
            ", prev \u{b7} . next \u{b7} u focus \u{b7} x close \u{b7} p pull \u{b7} Enter type \u{b7} ? help",
            "{w}x{h}"
        );
        // Nothing names a pilot or an object.
        for leak in ["Nick", "#5", "#7", "Callsign", "lock"] {
            assert!(
                !text
                    .lines()
                    .skip(2)
                    .any(|line| line.contains(leak) && !line.contains("Mireille")),
                "{w}x{h}: the replay screen names {leak:?}:\n{text}"
            );
        }
        check(&format!("tui_replay_{w}x{h}.txt"), &text);
    }
    // Stepped to the end: the shots and the wreck, newest first.
    app.feed_input(b".");
    let text = frame_text(&mut app, 120, 34);
    assert!(left_holds(&text, "0:07  BB wrecked"), "{text}");
    assert!(right_holds(&text, "0:08  BA at RA: miss", 120), "{text}");
    assert!(right_holds(&text, "0:06  RA at BA: hit", 120), "{text}");
    let miss = text.find("0:08  BA at RA: miss").unwrap_or(usize::MAX);
    let hit = text.find("0:06  RA at BA: hit").unwrap_or(0);
    assert!(miss < hit, "newest first:\n{text}");
    assert!(right_holds(&text, "BB   blue  wreck", 120), "{text}");
    assert!(left_holds(&text, "Keyframe 3/3  0:10 of 0:10"), "{text}");
}

// ---------------------------------------------------------------------------
// 16b cut I.4 — the live picture on screen 0 (16c N10)
// ---------------------------------------------------------------------------

/// A staff picture of match 7 on the cockpit fixtures' yard, as
/// `tacsim::match_::observer::observer_frame` builds one: four tanks, BA held,
/// BB a wreck on The Roof, RB starting and off the window, a 5,000 ms floor.
fn live_picture() -> serde_json::Value {
    serde_json::json!({
        "schema": { "id": "arena.observer", "version": 1, "retain": "latest" },
        "match": 7, "now_ms": 90_000, "clock": "1:30", "tempo": "engaged",
        "floor_ms": 5_000, "cell_cm": 1_000, "width": 96, "height": 64,
        "in_flight": 2,
        "units": [
            ["RA", "R", 6, 4, 90, 301, "pm"],
            ["RB", "R", null, null, 0, 303, "s"],
            ["BA", "B", 20, 14, 270, 302, "ph"],
            ["BB", "B", 14, 7, 225, 304, "w"],
        ],
    })
}

/// The fixture app watching match 7: the first picture pushed, the cockpit ask
/// screen 0 makes answered the cleared frame, the ground held.
fn watch_app() -> TuiApp {
    let mut app = fixture_app();
    app.set_manifest(TuiManifest {
        panels: vec![
            "wod.vitals".into(),
            "wod.sheet".into(),
            "mail".into(),
            "arena.cockpit".into(),
            "arena.map".into(),
            "arena.replay".into(),
            "arena.observer".into(),
        ],
        character: Some("Nick".into()),
        room: None,
    });
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("arena.observer"),
        kind: PanelFrameKind::Keyframe,
        data: live_picture(),
    });
    app.apply_event(&OutputEvent::Panel {
        feed: SmolStr::new_static("arena.cockpit"),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::json!({
            "schema": { "id": "arena.cockpit", "version": 1, "retain": "latest" },
            "seated": false
        }),
    });
    app.apply_event(&arena_panel("arena.map", ARENA_MAP));
    app
}

/// **The live picture at the floor** (16b cut I.4; 16c N10): the replay's
/// layout — the same six panes in the same columns — drawn from the pushed
/// picture: `Watching 7 — 1:30 (live)` over the units as a replay draws them
/// (RB, with no cell, not drawn), the four status sentences each whole in the
/// Left column, `Nothing yet.` for damage and weapons, and every unit's row
/// with its object number (a staff surface) and its words, one line each (a
/// realistic row's width is pinned in `tui::replay`'s own tests). **The legend is
/// the live table's: `u` and `p` alone** — `,` `.` and `x` are omitted,
/// because nothing steps a live picture and only the server closes it. The
/// golden is the review.
#[test]
fn i4_the_live_picture_paints_at_the_floor() {
    let mut app = watch_app();
    assert_eq!(
        app.active_screen(),
        COCKPIT_SCREEN,
        "the picture opened screen 0"
    );
    let (w, h) = (120u16, 34u16);
    let text = frame_text(&mut app, w, h);
    assert_eq!(text.lines().count(), usize::from(h));
    for line in text.lines() {
        assert!(
            line.chars().count() <= usize::from(w),
            "row too wide: {line:?}"
        );
    }
    for title in ["status", "damage", "weapons", "contacts", "notices"] {
        assert!(
            text.contains(&format!("\u{250c}{title}")),
            "{title} is painted:\n{text}"
        );
    }
    assert!(text.contains("Watching 7 \u{2014} 1:30 (live)"), "{text}");
    for sentence in [
        "Match 7  engaged  floor 5000",
        "Live, at most once a second.",
        "In flight: 2",
        "@arena/watch off stops it.",
    ] {
        assert!(
            left_holds(&text, sentence),
            "{sentence:?} is not whole in the Left column:\n{text}"
        );
    }
    assert!(left_holds(&text, "Nothing yet."), "{text}");
    for row in [
        "RA   red   (6, 4) 090\u{b0} #301 machine",
        "RB   red   off the window #303 pilot starting",
        "BA   blue  (20, 14) 270\u{b0} #302 pilot held",
        "BB   blue  (14, 7) 225\u{b0} #304 pilot wreck",
    ] {
        assert!(
            right_holds(&text, row, w),
            "{row:?} is not whole in the Right column:\n{text}"
        );
    }
    // The units as a replay draws them: RB has no cell and is not drawn.
    assert!(text.contains("o RA  * BA  % BB"), "{text}");
    assert!(!text.contains("o RB"), "{text}");
    let legend = text.lines().last().unwrap_or_default();
    assert_eq!(
        legend, "u focus \u{b7} p pull \u{b7} Enter type \u{b7} ? help",
        "the live table's legend"
    );
    check("tui_watch_120x34.txt", &text);
}
