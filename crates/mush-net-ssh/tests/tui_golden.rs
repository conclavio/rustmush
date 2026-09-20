//! Golden frames for the modal TUI (interface plan I2): the scene screen
//! with fixture data at the three budget sizes — 80×24 (scrollback, input
//! and the chrome), 120×34 (the whole scene screen), 160×44 (the HUD case).
//!
//! The frames live under `tests/golden/`; `UPDATE_GOLDEN=1 cargo test -p
//! mush-net-ssh --test tui_golden` rewrites them, and the diff is the
//! review.

use std::path::PathBuf;

use mush_ansi::AnsiTheme;
use mush_net_ssh::tui::panels::PanelStore;
use mush_net_ssh::tui::render::{RenderCtx, render_cockpit_map_rows};
use mush_net_ssh::tui::screen::COCKPIT_SCREEN;
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
    let text = frame_text(&mut app, 80, 24);
    for line in text.lines() {
        assert!(line.chars().count() <= 80, "row too wide: {line:?}");
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
    check("tui_vitals_two_feeds_80x24.txt", &text);
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
// `arena_map.json` + `cockpit_frame.json` at exactly 78 by 20 equals the twenty
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
    });
    app.apply_event(&arena_panel("arena.map", ARENA_MAP));
    app.apply_event(&arena_panel("arena.cockpit", COCKPIT_FRAME));
    app
}

/// **The parity test** (doc 16 §5): the TUI's composition of the two committed
/// payloads at exactly 78 by 20 IS the grid a line client prints.
///
/// If this ever fails, the first differing cell is a defect in one of the three
/// compositors (or in the fixture), not a golden to re-bless.
#[test]
fn the_cockpit_map_composes_the_committed_grid() {
    let store = store_with_the_ground();
    let theme = AnsiTheme::default();
    // 78 by 22 is the PANE; the renderer keeps two rows for its heading and
    // its marker legend, so the window it carves is the fixtures' 78 by 20.
    let ctx = map_ctx(&theme, &store, (78, 22));
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
    // X4 plots at (50, 33), off a twenty-row window: not drawn, and not in the
    // marker legend either — it is still in the contacts pane.
    let legend = rows.rows.last().map(|row| row.text()).unwrap_or_default();
    assert_eq!(legend, "x BA  + RB  ? X3", "{legend:?}");
    assert!(!legend.contains("X4"));
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

/// The whole screen at the two budget sizes, on a server that publishes the
/// arena feeds. The FIRST cockpit frame of a match opens the screen by itself,
/// which is what the `active_screen` assertion pins.
#[test]
fn the_cockpit_screen_paints_at_both_budget_sizes() {
    let mut app = cockpit_app();
    assert_eq!(
        app.active_screen(),
        COCKPIT_SCREEN,
        "the first frame of a match opens the screen"
    );
    for (w, h) in [(120u16, 34u16), (80, 24)] {
        let text = frame_text(&mut app, w, h);
        assert_eq!(text.lines().count(), usize::from(h));
        for line in text.lines() {
            assert!(
                line.chars().count() <= usize::from(w),
                "{w}x{h}: row too wide: {line:?}"
            );
        }
        check(&format!("tui_cockpit_{w}x{h}.txt"), &text);
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
        "tempo engaged",
        "Lock BA  430 m  62%",
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
