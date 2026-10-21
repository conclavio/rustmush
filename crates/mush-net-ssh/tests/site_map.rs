//! **The site map's grid through the TUI's decoder** (doc 20 cut SM.4b; lead ruling 7, V5's
//! TUI half): the two `site.map` frames `tacsim`'s `tests/driving.rs` blessed for the web client
//! (`web/src/lib/fixtures/site/site_map.json`, the viewer's own; `site_map_other.json`, a second
//! principal's on the same pad) decode through `mush_ansi::window_rows` — the decoder the TUI's
//! grid panes draw with — and `mush_ansi::expand_rows` (the units overlaid) to exactly the grid
//! rows each frame's own `site.print` (the line-mode `+tv/map site`) prints, de-doubled. The
//! second principal's rows hold no ore glyph. Fixture only: no tacsim dependency (the screen `q`
//! that will draw it is SM.9's).

use serde_json::Value;

/// The viewer's own site view (`TACSIM_BLESS=1 cargo test -p tacsim --test driving
/// sm4b_the_site_map_fixtures_are_blessed`).
const MINE: &str = include_str!("../../../web/src/lib/fixtures/site/site_map.json");
/// A second principal's site view on the same pad, at the same instant.
const OTHER: &str = include_str!("../../../web/src/lib/fixtures/site/site_map_other.json");

/// The ore glyphs: a digit for a reading, `+` another family, `.` sounded barren.
const ORE_GLYPHS: [char; 5] = ['1', '2', '3', '+', '.'];

/// A frame's numeric key.
fn num(frame: &Value, key: &str) -> usize {
    frame
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .expect("a size")
}

/// The print's grid rows, north up, each glyph printed twice — de-doubled and turned south up.
fn printed_rows(frame: &Value) -> Vec<String> {
    let height = num(frame, "height");
    let print: Vec<&str> = frame["site"]["print"]
        .as_array()
        .expect("a print")
        .iter()
        .map(|line| line.as_str().expect("a line"))
        .collect();
    // The title and the legend's two lines, then the grid.
    let mut rows: Vec<String> = print
        .iter()
        .skip(3)
        .take(height)
        .map(|line| {
            let glyphs: Vec<char> = line.chars().collect();
            assert!(
                glyphs.chunks(2).all(|pair| pair.first() == pair.get(1)),
                "every glyph printed twice: {line}"
            );
            glyphs.iter().step_by(2).collect()
        })
        .collect();
    rows.reverse();
    rows
}

/// **The TUI's decoder draws the line print** (doc 20 cut SM.4b; lead ruling 7): for both
/// blessed frames, `window_rows` over the whole grid and `expand_rows` (the units overlaid) give
/// exactly the rows `+tv/map site` printed; the legend line is doc 20's, verbatim; the second
/// principal's grid holds no ore glyph, the viewer's does.
#[test]
fn sm4b_the_tui_decoder_draws_the_line_print() {
    for (name, text) in [("mine", MINE), ("other", OTHER)] {
        let frame: Value = serde_json::from_str(text).expect("the fixture is JSON");
        assert_eq!(frame["schema"]["id"], "site.map", "{name}");
        assert!(mush_ansi::panel::is_grid(&frame), "{name}");
        let (width, height) = (num(&frame, "width"), num(&frame, "height"));
        let rows = frame["rows"].as_array().expect("rows");
        let decoded: Vec<String> =
            mush_ansi::panel::window_rows(rows, width, height, 0, 0, width, height)
                .into_iter()
                .map(|row| row.into_iter().collect())
                .collect();
        let overlaid: Vec<String> = mush_ansi::panel::expand_rows(&frame)
            .into_iter()
            .map(|row| row.into_iter().collect())
            .collect();
        let printed = printed_rows(&frame);
        assert_eq!(decoded, printed, "{name}: the TUI's decoder");
        assert_eq!(overlaid, printed, "{name}: the units reproduce the rows");
        let legend = frame["site"]["legend_line"].as_str().expect("a legend");
        assert_eq!(
            legend,
            "@ you  H hull  o rig  , regolith  ^ rock  = ice  O crater  1–3 ore (t a cycle)  + other family  . barren  v/V pit  # claim"
        );
        let has_ore = printed
            .iter()
            .any(|row| row.chars().any(|c| ORE_GLYPHS.contains(&c)));
        match name {
            "other" => assert!(!has_ore, "a second principal sees no ore glyph"),
            _ => assert!(has_ore, "the viewer sees their own readings"),
        }
    }
}
