//! **The sky chart screen** (doc 19 cut A.3b; doc 19 §5): the TUI's lettered
//! screen `c`, gated on the `sky.chart` feed, drawing the server's flat render
//! and its tables from the frame the Rust side blessed for the web client
//! (`web/src/lib/fixtures/chart/sky_chart.json`), so the three renders — line
//! mode's `+chart/map`, this screen and the web's picture — read one frame.

use mush_net_ssh::tui::screen::{CHART_SCREEN, COCKPIT_SCREEN, builtin_screens, validate_screens};
use mush_net_ssh::tui::{TuiApp, TuiManifest, frame_text};
use mush_proto::{ClientCaps, ClientKind, ColorDepth, OutputEvent, PanelFrameKind};

/// The frame `tests/chart.rs` blessed (`TACSIM_BLESS=1 cargo test -p tacsim --test chart`).
const SKY_CHART_JSON: &str = include_str!("../../../web/src/lib/fixtures/chart/sky_chart.json");

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

fn app_with(panels: &[&str]) -> TuiApp {
    let mut app = TuiApp::new(caps());
    app.set_clock(|| 14 * 3600 + 30 * 60);
    app.set_manifest(TuiManifest {
        panels: panels.iter().map(|p| (*p).to_owned()).collect(),
        character: Some("Nick".into()),
        room: None,
    });
    app
}

fn chart_frame() -> OutputEvent {
    OutputEvent::Panel {
        feed: "sky.chart".into(),
        kind: PanelFrameKind::Keyframe,
        data: serde_json::from_str(SKY_CHART_JSON).expect("the fixture is JSON"),
    }
}

/// **The screen validates and sits on the letter `c`** (doc 19 cut A.3b): the
/// eleven built-ins pass the probe; the chart's id is the letter's byte; a
/// digit screen's id is still a digit.
#[test]
fn a3b_the_chart_screen_is_the_eleventh_on_the_letter_c() {
    let screens = builtin_screens();
    validate_screens(&screens).expect("the built-ins validate");
    assert_eq!(screens.len(), 11);
    let chart = screens
        .iter()
        .find(|s| s.id == CHART_SCREEN)
        .expect("the chart screen");
    assert_eq!(chart.name, "chart");
    assert_eq!(CHART_SCREEN.0, b'c');
    assert_eq!(chart.gate, Some("sky.chart"));
    assert_eq!(chart.on_enter.len(), 1);
    assert_eq!(chart.on_enter[0].0, "sky.chart");
    assert_eq!(COCKPIT_SCREEN.0, 0);
}

/// **Alt+c opens the chart where the server publishes the feed, and nowhere
/// else** (doc 19 cut A.3b): on a server with `sky.chart`, Alt+c switches and
/// the entry asks for the frame; the frame applied, the pane draws the title,
/// the Sun, the vantage port's letter and the legend, and the tables pane the
/// server's sky lines; on a server without the feed Alt+c is inert and the
/// scene screen stays.
#[test]
fn a3b_alt_c_opens_the_chart_and_draws_the_blessed_frame() {
    let mut app = app_with(&["sky.chart", "mail"]);
    // ESC then the letter in one burst is Alt+c.
    let r = app.feed_input(b"\x1bc");
    assert!(
        r.queries.iter().any(|(name, _)| name == "sky.chart"),
        "entering the chart asks for the frame: {:?}",
        r.queries
    );
    app.apply_event(&chart_frame());
    let frame = frame_text(&mut app, 120, 34);
    assert!(frame.contains("The sky on "), "{frame}");
    assert!(frame.contains('*'), "the Sun:\n{frame}");
    assert!(frame.contains("ceres (here)"), "the legend:\n{frame}");
    // The tables pane prints the server's lines, the window first (doc 19 cut A.8:
    // with two destinations from Ceres the window fills the pane before the sky's
    // own heading, so the pane is held to its first heading and its first
    // destination, Aitken Highport by venue order).
    assert!(frame.contains("the window"), "the tables pane:\n{frame}");
    let fixture: serde_json::Value =
        serde_json::from_str(SKY_CHART_JSON).expect("the fixture is JSON");
    let first = fixture["chart"]["routes"][0]["to"]
        .as_str()
        .expect("the fixture lists a destination");
    assert!(
        frame.contains(first),
        "a destination's line ({first}):\n{frame}"
    );
    assert!(
        frame.contains(" c chart"),
        "the tab strip names the letter:\n{frame}"
    );

    let mut bare = app_with(&["mail"]);
    let before = frame_text(&mut bare, 120, 34);
    let r = bare.feed_input(b"\x1bc");
    assert!(r.queries.is_empty(), "{:?}", r.queries);
    let after = frame_text(&mut bare, 120, 34);
    assert_eq!(before, after, "Alt+c is inert without the feed");
    assert!(!after.contains(" c chart"), "{after}");
}
