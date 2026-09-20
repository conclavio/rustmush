//! Panel text fallbacks (doc 14 §6, plan §3.5; doc 06 §10's "panel→line ASCII
//! renderer"): a panel frame that declares a **shape** renders in line mode
//! from its data alone, with no per-feed code on the gateway's side of the
//! IPC boundary. The first shape is `grid` — the `atlas.map` frame: one glyph
//! per cell, a legend, the viewer centred, one surface at a time, fitting
//! eighty by twenty-four with the rest scrolled (P10: the text is the
//! acceptance test of every map the server emits).
//!
//! The grid frame: `schema.shape == "grid"`, `title`, `surface`, `floors`,
//! `width`, `height`, `rows` (row `y` from the south edge up, each a list of
//! `[glyph, run]` pairs), `legend` (`[glyph, label]` pairs), `units`
//! (`{ id, name, cell: [x, y], glyph, you }`), `you` (`[x, y]` or null) and,
//! on a delta, `note`.

use mush_proto::PanelFrameKind;
use serde_json::Value;

/// The widest grid line mode prints (the eightieth column stays free).
pub const GRID_MAX_COLUMNS: usize = 78;

/// The tallest grid line mode prints at once.
pub const GRID_MAX_ROWS: usize = 20;

/// Whether `data` is a frame in the grid shape.
#[must_use]
pub fn is_grid(data: &Value) -> bool {
    data.get("schema")
        .and_then(|s| s.get("shape"))
        .and_then(Value::as_str)
        == Some("grid")
}

fn num(data: &Value, key: &str) -> usize {
    data.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0)
}

fn text<'a>(data: &'a Value, key: &str) -> &'a str {
    data.get(key).and_then(Value::as_str).unwrap_or("")
}

fn cell_pair(v: &Value) -> Option<(usize, usize)> {
    let arr = v.as_array()?;
    let x = arr.first()?.as_u64()?;
    let y = arr.get(1)?.as_u64()?;
    Some((usize::try_from(x).ok()?, usize::try_from(y).ok()?))
}

fn glyph_of(v: &Value) -> char {
    v.as_str().and_then(|s| s.chars().next()).unwrap_or('?')
}

/// **The glyph rows of one viewport of a run-length `rows` array** (16b cut
/// Q.4), row `0` the viewport's SOUTH row, each exactly `w` glyphs.
///
/// `rows` is the frame's own `[[glyph, run], …]` array and `width`/`height` are
/// the whole grid's; only the rows `y0..y0 + h` are expanded, and only the
/// columns `x0..x0 + w` of each are kept. That is the difference that matters
/// to a panel: [`expand_rows`] materialises the whole grid — 256 by 256 for a
/// shipped arena — on every redraw, and a pane that shows seventy-eight by
/// twenty cells of it has no business paying for the other sixty-one thousand.
///
/// Out-of-range rows and columns are spaces, so the answer is always `h` rows
/// of exactly `w` glyphs: a window clamped at an edge, or a short row, pads
/// rather than running short. Units are NOT overlaid — a compositor that reads
/// its markers from another frame (the TUI's cockpit map reads them from
/// `arena.cockpit`) draws them itself.
#[must_use]
pub fn window_rows(
    rows: &[Value],
    width: usize,
    height: usize,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
) -> Vec<Vec<char>> {
    let mut out: Vec<Vec<char>> = Vec::with_capacity(h);
    let x_end = x0.saturating_add(w);
    for y in y0..y0.saturating_add(h) {
        let mut row: Vec<char> = vec![' '; w];
        if y < height
            && let Some(runs) = rows.get(y).and_then(Value::as_array)
        {
            // The cursor is the absolute column the next run starts at; a run
            // past `width` is clipped exactly as `expand_rows` clips it, and a
            // malformed entry is skipped without moving the cursor.
            let mut col = 0usize;
            for run in runs {
                if col >= width {
                    break;
                }
                let Some(pair) = run.as_array() else {
                    continue;
                };
                let glyph = pair.first().map_or(' ', glyph_of);
                let n = pair
                    .get(1)
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                    .unwrap_or(1);
                let end = col.saturating_add(n).min(width);
                for column in col.max(x0)..end.min(x_end) {
                    if let Some(cell) = row.get_mut(column - x0) {
                        *cell = glyph;
                    }
                }
                col = end;
            }
        }
        out.push(row);
    }
    out
}

/// The grid's rows as strings, row `0` the south edge, each exactly `width`
/// glyphs (runs past the width are clipped, short rows padded with spaces),
/// with the units overlaid.
#[must_use]
pub fn expand_rows(data: &Value) -> Vec<Vec<char>> {
    let width = num(data, "width");
    let height = num(data, "height");
    let empty = Vec::new();
    let raw_rows = data.get("rows").and_then(Value::as_array).unwrap_or(&empty);
    let mut rows = window_rows(raw_rows, width, height, 0, 0, width, height);
    if let Some(units) = data.get("units").and_then(Value::as_array) {
        for unit in units {
            let Some((x, y)) = unit.get("cell").and_then(cell_pair) else {
                continue;
            };
            let glyph = unit.get("glyph").map_or('o', glyph_of);
            if let Some(cell) = rows.get_mut(y).and_then(|r| r.get_mut(x)) {
                *cell = glyph;
            }
        }
    }
    rows
}

/// The viewport: `(x0, y0, w, h)` of at most `max_w` by `max_h` cells, centred
/// on `you` (else the grid's centre) and clamped to the grid.
#[must_use]
pub fn viewport(
    width: usize,
    height: usize,
    you: Option<(usize, usize)>,
    max_w: usize,
    max_h: usize,
) -> (usize, usize, usize, usize) {
    let w = width.min(max_w.max(1));
    let h = height.min(max_h.max(1));
    let (cx, cy) = you.unwrap_or((width / 2, height / 2));
    let x0 = cx.saturating_sub(w / 2).min(width.saturating_sub(w));
    let y0 = cy.saturating_sub(h / 2).min(height.saturating_sub(h));
    (x0, y0, w, h)
}

/// The legend line: the frame's legend, then the unit glyphs.
#[must_use]
pub fn legend_line(data: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(legend) = data.get("legend").and_then(Value::as_array) {
        for entry in legend {
            let Some(pair) = entry.as_array() else {
                continue;
            };
            let glyph = pair.first().map_or(' ', glyph_of);
            let label = pair.get(1).and_then(Value::as_str).unwrap_or("");
            parts.push(format!("{glyph} {label}"));
        }
    }
    format!("legend: {}", parts.join("  "))
}

/// The lines a grid frame prints: a title, the legend, then the viewport's rows
/// north-up (at most `columns` by `rows_max` cells; line mode passes
/// [`GRID_MAX_COLUMNS`] by [`GRID_MAX_ROWS`], a scrolling pane the whole grid),
/// and a footer naming the viewport's extent. A delta prints its `note` alone,
/// or nothing.
#[must_use]
pub fn grid_lines(
    data: &Value,
    kind: PanelFrameKind,
    columns: usize,
    rows_max: usize,
) -> Vec<String> {
    if kind == PanelFrameKind::Delta {
        let note = text(data, "note");
        return if note.is_empty() {
            Vec::new()
        } else {
            vec![note.to_string()]
        };
    }
    let width = num(data, "width");
    let height = num(data, "height");
    if width == 0 || height == 0 {
        return vec!["(an empty map)".to_string()];
    }
    let columns = columns.clamp(8, 512);
    let rows_max = rows_max.clamp(3, 512);
    let you = data.get("you").and_then(cell_pair);
    let (x0, y0, w, h) = viewport(width, height, you, columns, rows_max);
    let grid = expand_rows(data);
    let mut lines = Vec::with_capacity(h + 3);
    let title = text(data, "title");
    let surface = text(data, "surface");
    let floors: Vec<&str> = data
        .get("floors")
        .and_then(Value::as_array)
        .map(|f| f.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let cell_cm = num(data, "cell_cm");
    let mut head = if title.is_empty() {
        "Map".to_string()
    } else {
        title.to_string()
    };
    if !surface.is_empty() {
        head.push_str(&format!(" ({surface}"));
        if cell_cm > 0 {
            head.push_str(&format!(", {cell_cm} cm cells"));
        }
        head.push(')');
    }
    if floors.len() > 1 {
        head.push_str(&format!("  floors: {}", floors.join(", ")));
    }
    lines.push(head);
    lines.push(legend_line(data));
    for y in (y0..y0 + h).rev() {
        let row: String = grid
            .get(y)
            .map(|r| r.iter().skip(x0).take(w).collect())
            .unwrap_or_default();
        lines.push(row);
    }
    lines.push(format!(
        "x {x0}..{}  y {y0}..{}  of {width}x{height}{}",
        x0 + w - 1,
        y0 + h - 1,
        you.map_or(String::new(), |(x, y)| format!("  you at ({x}, {y})"))
    ));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frame() -> Value {
        json!({
            "schema": { "id": "atlas.map", "version": 1, "shape": "grid" },
            "window": 1, "revision": 1, "tick": 0,
            "title": "Coffee Shop", "surface": "floor 0", "floors": ["ground", "floor 0"],
            "origin": [31200, 24000], "cell_cm": 100,
            "width": 5, "height": 3,
            "rows": [
                [["#", 5]],
                [["#", 1], ["_", 3], ["#", 1]],
                [["#", 2], ["+", 1], ["#", 2]]
            ],
            "legend": [["#", "wall"], ["_", "floor"], ["+", "door"], ["@", "you"], ["o", "other"]],
            "units": [
                { "id": "#5", "name": "Rae", "cell": [1, 1], "glyph": "@", "you": true },
                { "id": "#6", "name": "Kit", "cell": [3, 1], "glyph": "o", "you": false }
            ],
            "you": [1, 1]
        })
    }

    #[test]
    fn a_keyframe_prints_north_up_with_the_units_overlaid() {
        let lines = grid_lines(&frame(), PanelFrameKind::Keyframe, 80, 24);
        assert_eq!(
            lines,
            vec![
                "Coffee Shop (floor 0, 100 cm cells)  floors: ground, floor 0".to_string(),
                "legend: # wall  _ floor  + door  @ you  o other".to_string(),
                "##+##".to_string(),
                "#@_o#".to_string(),
                "#####".to_string(),
                "x 0..4  y 0..2  of 5x3  you at (1, 1)".to_string(),
            ]
        );
    }

    #[test]
    fn a_wide_grid_is_windowed_on_the_viewer() {
        let mut f = frame();
        f["width"] = json!(200);
        f["height"] = json!(100);
        f["rows"] = json!((0..100).map(|_| json!([[".", 200]])).collect::<Vec<_>>());
        f["you"] = json!([150, 80]);
        f["units"] =
            json!([{ "id": "#5", "name": "Rae", "cell": [150, 80], "glyph": "@", "you": true }]);
        // Line mode's viewport: seventy-eight by twenty.
        let lines = grid_lines(
            &f,
            PanelFrameKind::Keyframe,
            GRID_MAX_COLUMNS,
            GRID_MAX_ROWS,
        );
        assert_eq!(lines.len(), 2 + GRID_MAX_ROWS + 1);
        assert!(
            lines
                .iter()
                .skip(2)
                .take(GRID_MAX_ROWS)
                .all(|l| l.chars().count() == GRID_MAX_COLUMNS)
        );
        assert!(
            lines.iter().any(|l| l.contains('@')),
            "the viewer is in the viewport"
        );
        assert!(
            lines
                .last()
                .is_some_and(|l| l.starts_with("x 111..188  y 70..89")),
            "{:?}",
            lines.last()
        );
    }

    #[test]
    fn a_delta_prints_its_note_or_nothing() {
        let d = json!({ "schema": { "shape": "grid" }, "note": "Kit moves north." });
        assert_eq!(
            grid_lines(&d, PanelFrameKind::Delta, 80, 24),
            vec!["Kit moves north.".to_string()]
        );
        let d = json!({ "schema": { "shape": "grid" }, "units": [] });
        assert!(grid_lines(&d, PanelFrameKind::Delta, 80, 24).is_empty());
    }

    #[test]
    fn a_short_row_is_padded_and_a_long_run_is_clipped() {
        let f = json!({
            "schema": { "shape": "grid" }, "width": 3, "height": 2,
            "rows": [[["#", 9]], [["_", 1]]],
            "legend": [], "units": [], "you": null
        });
        let rows = expand_rows(&f);
        assert_eq!(rows, vec![vec!['#', '#', '#'], vec!['_', ' ', ' ']]);
        assert!(is_grid(&f));
        assert!(!is_grid(&json!({ "schema": { "shape": "list" } })));
    }

    /// **16b cut Q.4**: one window of a grid is the matching slice of the whole
    /// one, and the whole one is now built out of it — so a panel that draws
    /// seventy-eight by twenty cells of a 256-by-256 arena and a line client
    /// that draws all of it cannot disagree about a single glyph.
    ///
    /// Three shapes: a window in the interior, a window clamped at each of the
    /// four edges, and rows shorter than the declared width (the pad).
    #[test]
    fn a_window_is_the_matching_slice_of_the_whole_grid() {
        // A 40-by-12 grid with a distinct pattern per row, no units: the
        // comparison is against `expand_rows`' terrain, which units overlay.
        let rows: Vec<Value> = (0..12)
            .map(|y| {
                let glyph = char::from(b'a' + u8::try_from(y % 26).unwrap_or(0));
                json!([[glyph.to_string(), 7], ["#", 1], [".", 40]])
            })
            .collect();
        let grid = json!({
            "schema": { "shape": "grid" }, "width": 40, "height": 12,
            "rows": rows.clone(), "legend": [], "units": [], "you": null
        });
        let whole = expand_rows(&grid);
        let slice = |x0: usize, y0: usize, w: usize, h: usize| -> Vec<Vec<char>> {
            (y0..y0 + h)
                .map(|y| {
                    whole
                        .get(y)
                        .map(|row| row.iter().skip(x0).take(w).copied().collect())
                        .unwrap_or_default()
                })
                .collect()
        };
        for (x0, y0, w, h) in [
            (5usize, 3usize, 9usize, 4usize),
            (0, 0, 40, 12),
            (0, 0, 6, 2),
            (34, 8, 6, 4),
            (0, 8, 6, 4),
            (34, 0, 6, 4),
        ] {
            assert_eq!(
                window_rows(&rows, 40, 12, x0, y0, w, h),
                slice(x0, y0, w, h),
                "window {x0},{y0} {w}x{h}"
            );
        }
        // Past the grid's own edge: spaces, never a short row.
        let over = window_rows(&rows, 40, 12, 38, 10, 5, 4);
        assert_eq!(over.len(), 4);
        for row in &over {
            assert_eq!(row.len(), 5);
        }
        assert_eq!(over.get(2).map(|r| r.as_slice()), Some([' '; 5].as_slice()));
        assert_eq!(
            over.first().and_then(|r| r.get(2..)),
            Some([' ', ' ', ' '].as_slice()),
            "the columns past the width pad too"
        );

        // A row shorter than the declared width pads; a run longer than it is
        // clipped; and both agree with `expand_rows`.
        let short = json!({
            "schema": { "shape": "grid" }, "width": 6, "height": 3,
            "rows": [[["#", 99]], [["_", 2]], []],
            "legend": [], "units": [], "you": null
        });
        let short_rows = short
            .get("rows")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            window_rows(&short_rows, 6, 3, 0, 0, 6, 3),
            expand_rows(&short)
        );
        assert_eq!(
            window_rows(&short_rows, 6, 3, 1, 1, 3, 2),
            vec![vec!['_', ' ', ' '], vec![' ', ' ', ' ']]
        );
    }
}
