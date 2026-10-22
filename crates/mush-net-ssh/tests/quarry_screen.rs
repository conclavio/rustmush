//! **The quarry screen** (doc 20 cut SM.9; §10, D-20-13): the TUI's lettered screen `q`, gated
//! on the `site.map` feed, drawing the rigs, the map — the drive view, the whole site, the
//! body's ground chart — and the readings and the log from the frames the Rust side blessed for
//! the web client (`web/src/lib/fixtures/site/{site_map,site_map_other,body_ground}.json`), so
//! line mode's `+tv/map site` and `+chart/ground map`, this screen and the web's picture read
//! one frame. Its keys prefill `+tv/<n> …` and send nothing; a pushed `tv.status` keyframe
//! re-asks the map; a moving rig is placed along its leg by the same rule as the server's.

use mush_ansi::AnsiTheme;
use mush_net_ssh::tui::panels::QuarryMode;
use mush_net_ssh::tui::render::{
    PaneRows, RenderCtx, quarry_grid, quarry_leg_position, render_quarry_map_rows,
    render_quarry_rigs_rows, render_quarry_side_rows,
};
use mush_net_ssh::tui::screen::{
    FLOOR_COLS, KeyAction, LocalAction, PaneId, PaneSlot, QUARRY_SCREEN, builtin_screens,
    validate_screens,
};
use mush_net_ssh::tui::{TuiApp, TuiManifest, frame_text};
use mush_proto::{ClientCaps, ClientKind, ColorDepth, OutputEvent, PanelFrameKind};
use serde_json::{Value, json};

/// The viewer's own site view (`TACSIM_BLESS=1 cargo test -p tacsim --test driving
/// sm4b_the_site_map_fixtures_are_blessed`).
const SITE_MAP: &str = include_str!("../../../web/src/lib/fixtures/site/site_map.json");
/// A second principal's site view on the same pad.
const SITE_MAP_OTHER: &str = include_str!("../../../web/src/lib/fixtures/site/site_map_other.json");
/// The Ceres ground chart (`TACSIM_BLESS=1 cargo test -p tacsim --test quarry
/// sm9_the_ground_fixture_is_blessed`).
const BODY_GROUND: &str = include_str!("../../../web/src/lib/fixtures/site/body_ground.json");

/// The clock every test reads (Unix seconds).
const NOW: i64 = 14 * 3600 + 30 * 60;

fn caps(height: u16) -> ClientCaps {
    ClientCaps {
        kind: ClientKind::Ssh,
        color: ColorDepth::Ansi16,
        width: 120,
        height,
        utf8: true,
        structured: false,
    }
}

fn app_with(panels: &[&str]) -> TuiApp {
    let mut app = TuiApp::new(caps(56));
    app.set_clock(|| NOW);
    app.set_manifest(TuiManifest {
        panels: panels.iter().map(|p| (*p).to_owned()).collect(),
        character: Some("Nick".into()),
        room: None,
    });
    app
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).expect("the fixture is JSON")
}

fn keyframe(feed: &str, data: Value) -> OutputEvent {
    OutputEvent::Panel {
        feed: feed.into(),
        kind: PanelFrameKind::Keyframe,
        data,
    }
}

/// A manifest with both quarry feeds and the push feed.
const BOTH: [&str; 4] = ["site.map", "body.ground", "tv.status", "mail"];

/// The asks of one name in a result's queries.
fn asks<'a>(queries: &'a [(String, Value)], name: &str) -> Vec<&'a Value> {
    queries
        .iter()
        .filter(|(n, _)| n == name)
        .map(|(_, args)| args)
        .collect()
}

/// **The twelfth screen sits on the letter `q`** (doc 20 cut SM.9): twelve built-ins validate;
/// the quarry is gated on `site.map`; its three panes — rigs Left 32, map Main 58, readings
/// Right 28 — fit the 120-column floor; its eight bindings in order, none a `Run`.
#[test]
fn sm9_the_quarry_is_the_twelfth_screen_on_the_letter_q() {
    let screens = builtin_screens();
    validate_screens(&screens).expect("the built-ins validate");
    assert_eq!(screens.len(), 12);
    assert_eq!(QUARRY_SCREEN.0, b'q');
    let quarry = screens
        .iter()
        .find(|s| s.id == QUARRY_SCREEN)
        .expect("the quarry screen");
    assert_eq!(quarry.name, "quarry");
    assert_eq!(quarry.gate, Some("site.map"));
    let pane = |id: &'static str| quarry.pane(PaneId(id)).expect("a pane");
    assert_eq!(
        (pane("rigs").slot, pane("rigs").min_w),
        (PaneSlot::Left, 32)
    );
    assert_eq!((pane("map").slot, pane("map").min_w), (PaneSlot::Main, 58));
    assert_eq!(
        (pane("readings").slot, pane("readings").min_w),
        (PaneSlot::Right, 28)
    );
    assert!(pane("rigs").min_w + pane("map").min_w + pane("readings").min_w <= FLOOR_COLS);
    let keys: Vec<(char, &str, KeyAction)> = quarry
        .keys
        .iter()
        .map(|b| (b.key, b.label, b.action))
        .collect();
    assert_eq!(
        keys,
        [
            ('s', "scan", KeyAction::PrefillRig("scan")),
            ('d', "drive", KeyAction::PrefillRig("drive ")),
            ('x', "sound", KeyAction::PrefillRig("sound")),
            ('m', "mine", KeyAction::PrefillRig("mine")),
            ('t', "rig", KeyAction::Local(LocalAction::QuarryRig)),
            ('z', "zoom", KeyAction::Local(LocalAction::QuarryZoom)),
            ('l', "layer", KeyAction::Local(LocalAction::QuarryLayer)),
            ('r', "refresh", KeyAction::Local(LocalAction::Refresh)),
        ]
    );
    assert!(
        quarry
            .keys
            .iter()
            .all(|b| !matches!(b.action, KeyAction::Run(_))),
        "no key on the quarry sends anything"
    );
    // After the chart, before the cockpit.
    let order: Vec<u8> = screens.iter().map(|s| s.id.0).collect();
    let at = |id: u8| order.iter().position(|o| *o == id).expect("listed");
    assert!(at(b'c') < at(b'q') && at(b'q') < at(0));
}

/// **Alt+q asks for both frames, and no key submits** (lead ruling 6): on a manifest with
/// `site.map` and `body.ground`, Alt+q asks `site.map {view: drive}` and `body.ground {}`; with
/// the blessed frame held, `s` `d` `x` `m` open the box with `+tv/1 scan`, `+tv/1 drive `,
/// `+tv/1 sound`, `+tv/1 mine` (the frame's addressed rig) and submit nothing; with no frame,
/// `+tv scan`; `t`, `z`, `l` and `r` submit nothing and re-ask with their arguments; Alt+q is
/// inert on a manifest without `site.map`.
#[test]
fn sm9_alt_q_asks_for_both_frames_and_no_key_submits() {
    let mut empty = app_with(&BOTH);
    let r = empty.feed_input(b"\x1bq");
    assert_eq!(empty.active_screen(), QUARRY_SCREEN);
    assert_eq!(asks(&r.queries, "site.map"), [&json!({ "view": "drive" })]);
    assert_eq!(asks(&r.queries, "body.ground"), [&json!({})]);
    // No frame held: the prefill names no rig.
    let r = empty.feed_input(b"s");
    assert!(r.submit.is_empty());
    assert_eq!(empty.input_text(), "+tv scan");

    let mut app = app_with(&BOTH);
    let _ = app.feed_input(b"\x1bq");
    let _ = app.apply_event(&keyframe("site.map", json_of(SITE_MAP)));
    for (key, line) in [
        (b"s", "+tv/1 scan"),
        (b"d", "+tv/1 drive "),
        (b"x", "+tv/1 sound"),
        (b"m", "+tv/1 mine"),
    ] {
        let r = app.feed_input(key);
        assert!(r.submit.is_empty(), "{line}: {:?}", r.submit);
        assert_eq!(app.input_text(), line);
        let r = app.feed_input(b"\x1b");
        assert!(r.submit.is_empty());
    }
    // `z`: the site view; `z` again: the ground view (the site view pulled beside it).
    let r = app.feed_input(b"z");
    assert!(r.submit.is_empty());
    assert_eq!(asks(&r.queries, "site.map"), [&json!({ "view": "site" })]);
    assert_eq!(app.panels().quarry().mode, QuarryMode::Site);
    let r = app.feed_input(b"z");
    assert_eq!(app.panels().quarry().mode, QuarryMode::Ground);
    assert_eq!(asks(&r.queries, "site.map"), [&json!({ "view": "site" })]);
    let _ = app.feed_input(b"z");
    assert_eq!(app.panels().quarry().mode, QuarryMode::Drive);
    // `l`: the family after the frame's (carbon) in its `families`.
    let r = app.feed_input(b"l");
    assert!(r.submit.is_empty());
    assert_eq!(
        asks(&r.queries, "site.map"),
        [&json!({ "view": "drive", "family": "strategic" })]
    );
    assert_eq!(
        asks(&r.queries, "body.ground"),
        [&json!({ "family": "strategic" })]
    );
    // `t`: the next rig of the frame's (it lists one: rig 1 again).
    let r = app.feed_input(b"t");
    assert!(r.submit.is_empty());
    assert_eq!(app.panels().quarry().rig, Some(1));
    assert_eq!(
        asks(&r.queries, "site.map"),
        [&json!({ "view": "drive", "rig": 1, "family": "strategic" })]
    );
    let r = app.feed_input(b"r");
    assert!(r.submit.is_empty());
    assert_eq!(asks(&r.queries, "site.map").len(), 1);

    let mut bare = app_with(&["mail"]);
    let before = frame_text(&mut bare, 120, 34);
    let r = bare.feed_input(b"\x1bq");
    assert!(r.queries.is_empty(), "{:?}", r.queries);
    assert_ne!(bare.active_screen(), QUARRY_SCREEN);
    assert_eq!(before, frame_text(&mut bare, 120, 34));
}

/// The Main pane's rows for `app`'s store, through the renderer, at the 120-column floor's
/// Main inner width (58) and a tall pane.
fn map_rows(app: &TuiApp, data: &Value) -> Vec<String> {
    pane_rows(app, data, (58, 48), render_quarry_map_rows)
}

/// One quarry pane's rows for `app`'s store through `render`, in a pane of `size`.
fn pane_rows(
    app: &TuiApp,
    data: &Value,
    size: (u16, u16),
    render: fn(&Value, &RenderCtx<'_>) -> PaneRows,
) -> Vec<String> {
    let theme = AnsiTheme::default();
    let ctx = RenderCtx {
        theme: &theme,
        depth: ColorDepth::Ansi16,
        utf8: true,
        store: app.panels(),
        selected_channel: None,
        pane: Some(size),
        picture: None,
    };
    render(data, &ctx)
        .rows
        .iter()
        .map(mush_net_ssh::tui::render::PaneRow::text)
        .collect()
}

/// A frame's strings at `pointer`'s array, each `key` of its objects (or the strings themselves).
fn texts(frame: &Value, pointer: &str, key: Option<&str>) -> Vec<String> {
    frame
        .pointer(pointer)
        .and_then(Value::as_array)
        .expect("an array")
        .iter()
        .filter_map(|item| match key {
            Some(key) => item.get(key).and_then(Value::as_str),
            None => item.as_str(),
        })
        .map(str::to_owned)
        .collect()
}

/// **The rigs and readings panes read the frame's own words** (doc 20 §10's Left and Right
/// panes; lead ruling 1 — no renderer writes a sentence the frame does not carry): over the
/// blessed `site_map.json` the Left pane is each `site.rigs[].line` — the addressed rig led `> `
/// — then the hold's line; the Right pane is `readings, {site.family}`, each
/// `site.readings[].line`, then `log` and each `site.log[].text`. On a variant frame a drilling
/// rig's `run.text` and a queued unload's `queued` stand under its line, indented four spaces,
/// the selected rig follows `t`, the frame's own `site.no_readings` and `site.other_rigs`
/// sentences are the ones shown, and the ground view's Right pane is the ground frame's
/// `ground.lines`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm9_the_rigs_and_readings_panes_read_the_frames_words() {
    let frame = json_of(SITE_MAP);
    let mut app = app_with(&BOTH);
    let _ = app.feed_input(b"\x1bq");
    let _ = app.apply_event(&keyframe("site.map", frame.clone()));
    // The Left pane: the rigs' lines, the addressed one selected, then the hold.
    let left = pane_rows(&app, &frame, (32, 20), render_quarry_rigs_rows);
    let lines = texts(&frame, "/site/rigs", Some("line"));
    assert_eq!(lines.len(), 1, "the fixture lists one rig");
    let hold = frame["site"]["hold"]["line"].as_str().expect("a hold line");
    assert_eq!(left, [format!("> {}", lines[0]), hold.to_owned()]);
    // The Right pane: the readings in the family shown, then the log.
    let right = pane_rows(&app, &frame, (28, 40), render_quarry_side_rows);
    let family = frame["site"]["family"].as_str().expect("a family");
    let mut expected = vec![format!("readings, {family}")];
    expected.extend(texts(&frame, "/site/readings", Some("line")));
    expected.push("log".to_owned());
    expected.extend(texts(&frame, "/site/log", Some("text")));
    assert!(!frame["site"]["log"].as_array().expect("a log").is_empty());
    assert_eq!(right, expected);

    // A variant: two rigs, rig 2 drilling and rig 1 with an unload queued, no readings, another
    // rig in view.
    let mut variant = frame.clone();
    let mut second = variant["site"]["rigs"][0].clone();
    second["slot"] = json!(2);
    second["line"] = json!("Rig 2: (150, 178), charge 80 %, hopper 3 t of 4.");
    second["run"] = json!({
        "addr": "G71", "ore": "metal ore", "k": 1, "cycles": 4, "due_ic": 12_000,
        "text": "drilling G71 for metal ore, cycle 1 of 4, this cycle done in 5 min",
    });
    variant["site"]["rigs"][0]["line"] = json!("Rig 1: in the bay, charge 95 %, hopper 4 t of 4.");
    variant["site"]["rigs"][0]["queued"] = json!("Unloads its hopper into the hold next.");
    variant["site"]["rigs"]
        .as_array_mut()
        .expect("rigs")
        .push(second);
    variant["site"]["readings"] = json!([]);
    variant["site"]["lines"] = json!([
        "You have no readings of metal ore in view.",
        "Rig 1: in the bay, charge 95 %, hopper 4 t of 4.",
        "Rig 2: (150, 178), charge 80 %, hopper 3 t of 4.",
        "Other rigs: 300 m NE (a sounding)",
        "Hold: 86 t of room unreserved.",
    ]);
    variant["site"]["no_readings"] = json!("You have no readings of metal ore in view.");
    variant["site"]["other_rigs"] = json!("Other rigs: 300 m NE (a sounding)");
    variant["site"]["log"] = json!([]);
    let _ = app.apply_event(&keyframe("site.map", variant.clone()));
    let left = pane_rows(&app, &variant, (32, 20), render_quarry_rigs_rows);
    assert_eq!(
        left,
        [
            "> Rig 1: in the bay, charge 95 %, hopper 4 t of 4.",
            "    Unloads its hopper into the hold next.",
            "  Rig 2: (150, 178), charge 80 %, hopper 3 t of 4.",
            "    drilling G71 for metal ore, cycle 1 of 4, this cycle done in 5 min",
            hold,
        ]
    );
    // `t` selects the next rig: the `>` follows it.
    let _ = app.feed_input(b"t");
    assert_eq!(app.panels().quarry().rig, Some(2));
    let left = pane_rows(&app, &variant, (32, 20), render_quarry_rigs_rows);
    assert_eq!(
        left[0],
        "  Rig 1: in the bay, charge 95 %, hopper 4 t of 4."
    );
    assert_eq!(
        left[2],
        "> Rig 2: (150, 178), charge 80 %, hopper 3 t of 4."
    );
    let right = pane_rows(&app, &variant, (28, 40), render_quarry_side_rows);
    assert_eq!(
        right,
        [
            format!("readings, {family}").as_str(),
            "You have no readings of metal ore in view.",
            "Other rigs: 300 m NE (a sounding)",
            "log",
        ]
    );
    // The ground view's Right pane: the ground frame's list.
    let ground = json_of(BODY_GROUND);
    let _ = app.apply_event(&keyframe("body.ground", ground.clone()));
    let _ = app.feed_input(b"z");
    let _ = app.feed_input(b"z");
    assert_eq!(app.panels().quarry().mode, QuarryMode::Ground);
    let right = pane_rows(&app, &variant, (28, 80), render_quarry_side_rows);
    assert_eq!(right, texts(&ground, "/ground/lines", None));
}

/// **The side pane reads its two sentences from their keys, whole** (the pre-commit round; the
/// judge's correctness/F10): a frame whose `site.no_readings` is worded unlike any line of
/// `site.lines` shows that sentence; an other-rigs sentence `site.lines` wraps over two lines is
/// one row of the pane — the whole of `site.other_rigs` — and the drawn screen at 120 columns
/// shows its last words (the continuation `site.lines` would have lost); with neither key the
/// pane shows neither sentence, whatever `site.lines` says.
#[test]
fn sm9_the_side_pane_reads_its_sentences_from_their_keys() {
    let other = "Other rigs: 300 m NE (a sounding); 460 m N; 520 m NNE; 610 m E; 700 m SE; 820 m S; \
                 940 m SW; 1060 m W; 1180 m NW; 1300 m WNW";
    let mut frame = json_of(SITE_MAP);
    frame["site"]["readings"] = json!([]);
    frame["site"]["no_readings"] = json!("Nothing of carbon read in view.");
    frame["site"]["other_rigs"] = json!(other);
    // `site.lines` as line mode wraps them — and, to prove the pane never reads it, worded
    // otherwise.
    frame["site"]["lines"] = json!([
        "You have no readings of carbon feedstock in view.",
        "Rig 1: (146, 178), charge 95 %, hopper empty.",
        "Other rigs: 300 m NE (a sounding); 460 m N; 520 m NNE; 610 m E; 700 m SE; 820 m S;",
        "  940 m SW; 1060 m W; 1180 m NW; 1300 m WNW",
        "Hold: 86 t of room unreserved.",
    ]);
    frame["site"]["log"] = json!([]);
    let mut app = app_with(&BOTH);
    let _ = app.feed_input(b"\x1bq");
    let _ = app.apply_event(&keyframe("site.map", frame.clone()));
    let family = frame["site"]["family"].as_str().expect("a family");
    let right = pane_rows(&app, &frame, (28, 40), render_quarry_side_rows);
    assert_eq!(
        right,
        [
            format!("readings, {family}").as_str(),
            "Nothing of carbon read in view.",
            other,
            "log",
        ]
    );
    let screen = frame_text(&mut app, 120, 56);
    for word in ["1180", "WNW"] {
        assert!(screen.contains(word), "{word}\n{screen}");
    }
    // Neither key: neither sentence, though `site.lines` still holds both.
    let mut bare = frame.clone();
    bare["site"]["no_readings"] = Value::Null;
    bare["site"]["other_rigs"] = Value::Null;
    let right = pane_rows(&app, &bare, (28, 40), render_quarry_side_rows);
    assert_eq!(right, [format!("readings, {family}").as_str(), "log"]);
}

/// **A rig past the window's edge lends no slot digit** (the server's compose counts only the
/// viewer's rigs inside the window): with rig 1 standing inside a 4 × 1 drive window and rig 2
/// on the ground far outside it — its leg in flight, so the frame carries a clock — the placed
/// grid at the frame's own instant is the frame's rows exactly: no `1` east of rig 1, and no
/// `@` for rig 2. Once rig 2's leg brings it inside, both stand and rig 1's digit follows.
#[test]
fn sm9_a_rig_outside_the_window_lends_no_slot_digit() {
    let frame = json!({
        "width": 4, "height": 1,
        "rows": [[[",", 1], ["@", 1], [".", 1], ["^", 1]]],
        "site": {
            "grid": { "x0": 10, "y0": 20, "span": 1, "cell_cm": 1_000 },
            "under": [[1, 0, ","]],
            "rigs": [
                { "slot": 1, "cell": [11, 20], "leg": null },
                {
                    "slot": 2, "cell": [40, 20],
                    "leg": {
                        "started_ic": 500,
                        "waypoints": [
                            { "x_cm": 40_500, "y_cm": 20_500, "at_s": 0 },
                            { "x_cm": 13_500, "y_cm": 20_500, "at_s": 270 },
                        ],
                    },
                },
            ],
            "clock": { "now_ic": 500, "rate": 1 },
        },
    });
    let text = |grid: Vec<Vec<char>>| -> String { grid.concat().into_iter().collect() };
    assert_eq!(
        text(quarry_grid(&frame, Some(500))),
        ",@.^",
        "the frame's own rows: rig 2 is not on this map"
    );
    assert_eq!(
        text(quarry_grid(&frame, Some(770))),
        ",@1@",
        "rig 2 arrived inside: both stand, rig 1's digit east of it, rig 2's past the edge"
    );
}

/// The print's grid rows of a frame (`print` lines after the title and the legend).
fn print_rows(print: &[Value], skip: usize, height: usize, columns: usize) -> Vec<String> {
    print
        .iter()
        .skip(skip)
        .take(height)
        .map(|line| {
            let line = line.as_str().expect("a line");
            line.chars().step_by(columns).collect()
        })
        .collect()
}

/// **The blessed frames draw byte-identically at 120 columns** (§13's acceptance): in the
/// ground view the Main pane's grid rows are `body_ground.json`'s `ground.print` grid rows, byte
/// for byte (and the decoder's own rows, north up); in the site view the two `site.map`
/// fixtures' Main rows are their `site.print` rows de-doubled to the frame's `cell_columns`; the
/// legend rows are built from the pairs; and the whole screen at 120 × 56 shows the grid rows.
#[test]
fn sm9_the_blessed_frames_draw_byte_identically_at_120_columns() {
    let ground = json_of(BODY_GROUND);
    let mut app = app_with(&BOTH);
    let _ = app.feed_input(b"\x1bq");
    let _ = app.apply_event(&keyframe("body.ground", ground.clone()));
    let _ = app.feed_input(b"z");
    let _ = app.feed_input(b"z");
    assert_eq!(app.panels().quarry().mode, QuarryMode::Ground);
    let rows = map_rows(&app, &Value::Null);
    let print = ground["ground"]["print"].as_array().expect("a print");
    let legend_lines = print
        .iter()
        .skip(1)
        .take_while(|l| {
            l.as_str()
                .is_some_and(|l| l.starts_with("legend:") || l.starts_with("  "))
        })
        .count();
    let expected = print_rows(print, 1 + legend_lines, 28, 1);
    assert_eq!(rows[1..29].to_vec(), expected, "the ground chart's rows");
    let decoded: Vec<String> = mush_ansi::panel::expand_rows(&ground)
        .into_iter()
        .rev()
        .map(|row| row.into_iter().collect())
        .collect();
    assert_eq!(decoded, expected, "the decoder's rows, north up");
    assert!(rows.iter().skip(29).all(|row| !row.starts_with("legend:")));
    assert!(
        rows.iter()
            .skip(29)
            .any(|row| row.contains("F Ceres Freeport")),
        "the legend from the pairs: {rows:?}"
    );
    let screen = frame_text(&mut app, 120, 56);
    for row in &expected {
        assert!(screen.contains(row.as_str()), "{row}\n{screen}");
    }
    // The site view of both blessed frames: the print de-doubled.
    for text in [SITE_MAP, SITE_MAP_OTHER] {
        let frame = json_of(text);
        let mut app = app_with(&BOTH);
        let _ = app.feed_input(b"\x1bq");
        let _ = app.feed_input(b"z");
        let _ = app.apply_event(&keyframe("site.map", frame.clone()));
        let rows = map_rows(&app, &frame);
        let height = usize::try_from(frame["height"].as_u64().expect("h")).expect("fits");
        let print = frame["site"]["print"].as_array().expect("a print");
        // The title and the legend's two lines, then the grid, each glyph printed twice.
        let expected = print_rows(print, 3, height, 2);
        assert_eq!(rows[1..=height].to_vec(), expected, "{}", frame["title"]);
        let legend: Vec<&String> = rows.iter().skip(1 + height).collect();
        assert!(legend.iter().any(|row| row.contains("V pit, worked out")));
    }
}

/// **A site frame that drops the picked rig clears the pick** (the pre-commit round; the
/// judge's correctness/F7): with two rigs listed and `t` on rig 2, a frame still listing rig 2
/// keeps the pick; a frame listing rig 1 alone (rig 2 returned) clears it, so the next ask —
/// `r`, or a pushed `tv.status` — carries no rig, and `t` steps on again from the frame's
/// addressed rig (the first listed slot here); a reason frame (no rigs: the server's
/// `MAP_NO_RIG`) clears a pick as well.
#[test]
fn sm9_a_frame_that_drops_the_picked_rig_clears_the_pick() {
    let frame = json_of(SITE_MAP);
    let mut two = frame.clone();
    let mut second = two["site"]["rigs"][0].clone();
    second["slot"] = json!(2);
    two["site"]["rigs"]
        .as_array_mut()
        .expect("rigs")
        .push(second);
    let mut app = app_with(&BOTH);
    let _ = app.feed_input(b"\x1bq");
    let _ = app.apply_event(&keyframe("site.map", two.clone()));
    let _ = app.feed_input(b"t");
    assert_eq!(
        app.panels().quarry().rig,
        Some(2),
        "after the addressed rig 1"
    );
    let _ = app.apply_event(&keyframe("site.map", two.clone()));
    assert_eq!(
        app.panels().quarry().rig,
        Some(2),
        "still listed: the pick stands"
    );
    // Rig 2 returned: the frame lists rig 1 alone.
    let _ = app.apply_event(&keyframe("site.map", frame.clone()));
    assert_eq!(app.panels().quarry().rig, None);
    let r = app.feed_input(b"r");
    assert_eq!(asks(&r.queries, "site.map"), [&json!({ "view": "drive" })]);
    let status = keyframe(
        "tv.status",
        json!({ "schema": { "id": "tv.status", "shape": "status", "version": 1 }, "rig": {} }),
    );
    let asked = app.apply_event(&status);
    assert_eq!(asks(&asked, "site.map"), [&json!({ "view": "drive" })]);
    // `t` steps on from the addressed rig 1, the first listed slot.
    let _ = app.apply_event(&keyframe("site.map", two.clone()));
    let _ = app.feed_input(b"t");
    assert_eq!(app.panels().quarry().rig, Some(2));
    // A reason frame lists no rigs: the pick goes too.
    let reason = json!({
        "schema": { "id": "site.map", "shape": "grid", "version": 1 },
        "window": 0,
        "title": "Rig 2 is not on Ahuna Mons's ground or in the bay.",
        "width": 0, "height": 0, "rows": [], "legend": [], "units": [], "you": null,
    });
    let _ = app.apply_event(&keyframe("site.map", reason));
    assert_eq!(app.panels().quarry().rig, None);
    let r = app.feed_input(b"r");
    assert_eq!(asks(&r.queries, "site.map"), [&json!({ "view": "drive" })]);
}

/// **A `tv.status` keyframe re-pulls the quarry** (decision D13; the lead's 0z ruling 1): on
/// `q` it asks `site.map` (with the screen's arguments) and nothing else; on the scene screen it
/// asks nothing; and the four-second tick never asks for the site map — not with a leg in
/// flight (the TUI places the rig on its own paints), not with a run, not off the screen.
#[test]
fn sm9_a_status_keyframe_re_pulls_the_quarry() {
    let status = keyframe(
        "tv.status",
        json!({ "schema": { "id": "tv.status", "shape": "status", "version": 1 }, "rig": {} }),
    );
    let mut app = app_with(&BOTH);
    let _ = app.feed_input(b"\x1bq");
    let asked = app.apply_event(&status);
    assert_eq!(asks(&asked, "site.map"), [&json!({ "view": "drive" })]);
    assert!(asks(&asked, "body.ground").is_empty());
    // A frame with a leg in flight: the timer asks nothing.
    let mut moving = json_of(SITE_MAP);
    moving["site"]["rigs"][0]["leg"] = json!({
        "started_ic": 1_000,
        "waypoints": [{ "x_cm": 0, "y_cm": 0, "at_s": 0 }, { "x_cm": 9_000, "y_cm": 0, "at_s": 90 }],
    });
    moving["site"]["clock"] = json!({ "now_ic": 1_010, "rate": 1 });
    let _ = app.apply_event(&keyframe("site.map", moving));
    for later in [NOW, NOW + 4, NOW + 8, NOW + 60] {
        assert!(
            asks(&app.heal_queries(later), "site.map").is_empty(),
            "no timed pull at {later}"
        );
    }
    let mut scene = app_with(&BOTH);
    assert!(
        scene.apply_event(&status).is_empty(),
        "off the quarry: nothing"
    );
}

/// **The map pane draws without a site frame** (the brief's F8): with no `site.map` held and
/// a `body.ground` frame held, the ground view draws the ground grid (the `pane_rows` arm); the
/// drive view says `(no map yet: r asks the server)`, not the loading spinner.
#[test]
fn sm9_the_map_pane_draws_without_a_site_frame() {
    let mut app = app_with(&BOTH);
    let _ = app.feed_input(b"\x1bq");
    let screen = frame_text(&mut app, 120, 56);
    assert!(
        screen.contains("(no map yet: r asks the server)"),
        "{screen}"
    );
    assert!(!screen.contains("loading"), "{screen}");
    let _ = app.apply_event(&keyframe("body.ground", json_of(BODY_GROUND)));
    let _ = app.feed_input(b"z");
    let _ = app.feed_input(b"z");
    let screen = frame_text(&mut app, 120, 56);
    assert!(
        screen.contains("Ceres: the Registry's survey of metals"),
        "{screen}"
    );
}

/// **`body.ground` is asked only where it is published**: Alt+q on a manifest with `site.map`
/// and without `body.ground` asks `site.map` alone.
#[test]
fn sm9_body_ground_is_asked_only_when_published() {
    let mut app = app_with(&["site.map", "mail"]);
    let r = app.feed_input(b"\x1bq");
    assert_eq!(app.active_screen(), QUARRY_SCREEN);
    assert_eq!(asks(&r.queries, "site.map").len(), 1);
    assert!(
        asks(&r.queries, "body.ground").is_empty(),
        "{:?}",
        r.queries
    );
}

/// **A moving rig is placed along its leg** (the lead's 0z ruling 1): the TUI's leg rule gives
/// the cells `tacsim::port::site_map::leg_position`'s own unit test pins (the same inputs, the
/// same answers); at a frame's own instant the placed grid is the frame's rows exactly; later,
/// the `@` stands where the leg says and the cell it left shows the glyph the frame listed
/// under it.
#[test]
fn sm9_a_moving_rig_is_placed_along_its_leg() {
    let leg = json!([
        { "x_cm": 0, "y_cm": 0, "at_s": 0 },
        { "x_cm": 10_000, "y_cm": 0, "at_s": 100 },
        { "x_cm": 10_000, "y_cm": 20_000, "at_s": 300 },
    ]);
    let points = leg.as_array().expect("waypoints");
    assert_eq!(quarry_leg_position(points, -5), Some((0, 0)));
    assert_eq!(quarry_leg_position(points, 50), Some((5_000, 0)));
    assert_eq!(quarry_leg_position(points, 100), Some((10_000, 0)));
    assert_eq!(quarry_leg_position(points, 200), Some((10_000, 10_000)));
    assert_eq!(quarry_leg_position(points, 9_999), Some((10_000, 20_000)));
    assert_eq!(quarry_leg_position(&[], 5), None);
    // A 4 × 1 drive window from site cell (10, 20): the rig at (10, 20), moving east 10 m a
    // 10 s; the frame was taken at its start.
    let frame = json!({
        "width": 4, "height": 1,
        "rows": [[["@", 1], [",", 2], ["^", 1]]],
        "site": {
            "grid": { "x0": 10, "y0": 20, "span": 1, "cell_cm": 1_000 },
            "under": [[0, 0, "."]],
            "rigs": [{
                "slot": 1, "cell": [10, 20],
                "leg": {
                    "started_ic": 500,
                    "waypoints": [
                        { "x_cm": 10_500, "y_cm": 20_500, "at_s": 0 },
                        { "x_cm": 13_500, "y_cm": 20_500, "at_s": 30 },
                    ],
                },
            }],
            "clock": { "now_ic": 500, "rate": 1 },
        },
    });
    let text = |grid: Vec<Vec<char>>| -> String { grid.concat().into_iter().collect() };
    assert_eq!(
        text(quarry_grid(&frame, Some(500))),
        "@,,^",
        "the frame's own rows"
    );
    assert_eq!(text(quarry_grid(&frame, None)), "@,,^");
    assert_eq!(
        text(quarry_grid(&frame, Some(520))),
        ".,@^",
        "20 s on: two cells east"
    );
    assert_eq!(
        text(quarry_grid(&frame, Some(900))),
        ".,,@",
        "past the leg: its end"
    );
    assert_eq!(
        mush_net_ssh::tui::render::quarry_ic_now(&frame, NOW, NOW + 7),
        Some(507)
    );
}
