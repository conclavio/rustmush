//! Panel text fallbacks (doc 14 §6, plan §3.5; doc 06 §10's "panel→line ASCII
//! renderer"): a panel frame that declares a **shape** renders in line mode
//! from its data alone, with no per-feed code on the gateway's side of the
//! IPC boundary. The first shape is `grid` — the `atlas.map` frame: one glyph
//! per cell, a legend, the viewer centred, one surface at a time, fitting
//! the 120-column floor — at most 118 columns by 32 rows since doc 20 cut
//! SM.4b (eighty by twenty-four before) — with the rest scrolled (P10: the
//! text is the acceptance test of every map the server emits).
//!
//! The grid frame: `schema.shape == "grid"`, `title`, `surface`, `floors`,
//! `width`, `height`, `rows` (row `y` from the south edge up, each a list of
//! `[glyph, run]` pairs), `legend` (`[glyph, label]` pairs), `units`
//! (`{ id, name, cell: [x, y], glyph, you }`), `you` (`[x, y]` or null) and,
//! on a delta, `note`.

use mush_proto::PanelFrameKind;
use serde_json::Value;

/// The widest grid line mode prints (doc 20 cut SM.4b, under the 120-column floor: the
/// hundred-and-nineteenth and twentieth columns stay free; it was 78 under the old
/// eighty-column assumption).
pub const GRID_MAX_COLUMNS: usize = 118;

/// The tallest grid line mode prints at once (doc 20 cut SM.4b: 32, so the whole 32-row site
/// map prints in one go; it was 20).
pub const GRID_MAX_ROWS: usize = 32;

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

/// **How many printed columns each cell of `data` is drawn as** (16b cut B.4,
/// `B-24`): the frame's `cell_columns`, clamped to `1..=4`, and `1` when it is
/// absent — which is every frame but the arena window. A terminal cell is about
/// twice as tall as it is wide, so the arena window asks for two and its ground
/// prints square.
fn cell_columns(data: &Value) -> usize {
    match num(data, "cell_columns") {
        0 => 1,
        n => n.clamp(1, 4),
    }
}

/// The lines a grid frame prints: a title, the legend, then the viewport's rows
/// north-up (at most `columns` PRINTED columns by `rows_max` rows; line mode
/// passes [`GRID_MAX_COLUMNS`] by [`GRID_MAX_ROWS`] — 118 by 32 since doc 20 cut SM.4b — a
/// scrolling pane the whole grid), and a footer naming the viewport's extent. A delta prints
/// its `note` alone, or nothing.
///
/// **16b cut B.4** (`B-24`, and `T-60`'s footer slip) made two keys count:
///
/// * `cell_columns` (a whole number, `1` when absent, clamped to `1..=4`): every glyph of every grid row is printed
///   that many times, and the viewport is `columns / cell_columns` cells wide so
///   the printed rows stay inside the `columns` budget;
/// * `origin_cell` (`[x, y]`, the frame's corner in a larger grid) with `extent`
///   (`[W, H]`, that grid's size; the frame's own `width`/`height` when absent):
///   the footer names that larger grid's cells —
///   `x a..b  y c..d  of WxH  you at (x, y)  (N m a cell, north up)` — rather
///   than the frame's own. The arena window carries both; a frame without
///   `origin_cell` (the atlas map) prints exactly the footer it always did.
///
/// The legend line is unchanged, and the grid rows stay one glyph per cell in
/// [`expand_rows`] and [`window_rows`]: the doubling is this printer's alone, so
/// a compositor that draws one glyph per cell composes the same rows.
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
    let per_cell = cell_columns(data);
    let you = data.get("you").and_then(cell_pair);
    let (x0, y0, w, h) = viewport(width, height, you, columns / per_cell, rows_max);
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
            .map(|r| {
                r.iter()
                    .skip(x0)
                    .take(w)
                    .flat_map(|glyph| std::iter::repeat_n(*glyph, per_cell))
                    .collect()
            })
            .unwrap_or_default();
        lines.push(row);
    }
    match data.get("origin_cell").and_then(cell_pair) {
        // 16b cut B.4: the window of a larger grid names that grid's cells.
        Some((ox, oy)) => {
            let (whole_w, whole_h) = data
                .get("extent")
                .and_then(cell_pair)
                .unwrap_or((width, height));
            lines.push(format!(
                "x {}..{}  y {}..{}  of {whole_w}x{whole_h}{}  ({} m a cell, north up)",
                ox + x0,
                ox + x0 + w - 1,
                oy + y0,
                oy + y0 + h - 1,
                you.map_or(String::new(), |(x, y)| format!(
                    "  you at ({}, {})",
                    ox + x,
                    oy + y
                )),
                cell_cm / 100
            ));
        }
        None => lines.push(format!(
            "x {x0}..{}  y {y0}..{}  of {width}x{height}{}",
            x0 + w - 1,
            y0 + h - 1,
            you.map_or(String::new(), |(x, y)| format!("  you at ({x}, {y})"))
        )),
    }
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
        // Line mode's viewport: 118 by 32 (doc 20 cut SM.4b; 78 by 20 before), clamped at the
        // east edge of the 200 by 100 grid (only x is clamped).
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
                .is_some_and(|l| l.starts_with("x 82..199  y 64..95")),
            "{:?}",
            lines.last()
        );
    }

    /// **16b cut B.4** (`B-24`, the `T-60` footer slip): a window that asks for
    /// two columns a cell prints each glyph twice — 39 cells are 78 printed
    /// columns, so the viewport is half the column budget in cells — and a window
    /// that names its corner in a larger grid (`origin_cell`, `extent`) gets a
    /// footer in that grid's cells. The atlas frame, which carries neither key,
    /// prints byte for byte what it did (the first test above is that pin).
    #[test]
    fn a_grid_with_two_columns_a_cell_prints_each_glyph_twice_and_an_arena_footer() {
        let rows: Vec<Value> = (0..20)
            .map(|y| {
                let glyph = if y % 2 == 0 { "." } else { "^" };
                json!([[glyph, 20], ["#", 19]])
            })
            .collect();
        let window = json!({
            "schema": { "id": "arena.cockpit", "version": 1, "shape": "grid" },
            "title": "Arena window", "surface": "proving_ground",
            "cell_cm": 1000, "width": 39, "height": 20,
            "rows": rows,
            "legend": [[".", "open"], ["#", "wall"], ["@", "you"]],
            "units": [{ "id": "#9", "cell": [19, 10], "glyph": "@", "you": true }],
            "you": [19, 10],
            "cell_columns": 2,
            "origin_cell": [100, 50],
            "extent": [256, 192],
        });
        let lines = grid_lines(
            &window,
            PanelFrameKind::Keyframe,
            GRID_MAX_COLUMNS,
            GRID_MAX_ROWS,
        );
        assert_eq!(lines.len(), 2 + 20 + 1);
        let grid: Vec<&String> = lines.iter().skip(2).take(20).collect();
        for row in &grid {
            assert_eq!(row.chars().count(), 78, "39 cells, two columns each");
            let glyphs: Vec<char> = row.chars().collect();
            assert!(
                glyphs.chunks(2).all(|pair| pair.first() == pair.get(1)),
                "every glyph printed twice: {row}"
            );
        }
        // North up: the top printed row is y 19 (odd, `^`), the bottom y 0.
        assert!(grid.first().is_some_and(|row| row.starts_with("^^^^")));
        assert!(grid.last().is_some_and(|row| row.starts_with("....")));
        // The `@` at cell 19 is printed at columns 38 and 39 of row y 10.
        assert_eq!(
            grid.get(9).and_then(|row| row.get(38..40)),
            Some("@@"),
            "{:?}",
            grid.get(9)
        );
        assert_eq!(
            lines.last().map(String::as_str),
            Some("x 100..138  y 50..69  of 256x192  you at (119, 60)  (10 m a cell, north up)")
        );

        // A narrower terminal: 40 printed columns is 20 cells, still doubled.
        let narrow = grid_lines(&window, PanelFrameKind::Keyframe, 40, GRID_MAX_ROWS);
        assert!(
            narrow
                .iter()
                .skip(2)
                .take(20)
                .all(|row| row.chars().count() == 40)
        );

        // Without `extent` the footer falls back to the frame's own size.
        let mut no_extent = window.clone();
        if let Some(object) = no_extent.as_object_mut() {
            object.remove("extent");
        }
        assert!(
            grid_lines(&no_extent, PanelFrameKind::Keyframe, 78, 20)
                .last()
                .is_some_and(|l| l.contains("  of 39x20  ")),
        );

        // And the same frame without the three keys is today's frame: one
        // column a cell and the window's own footer.
        let mut plain = window;
        if let Some(object) = plain.as_object_mut() {
            object.remove("cell_columns");
            object.remove("origin_cell");
            object.remove("extent");
        }
        let lines = grid_lines(&plain, PanelFrameKind::Keyframe, 78, 20);
        assert!(
            lines
                .iter()
                .skip(2)
                .take(20)
                .all(|row| row.chars().count() == 39)
        );
        assert_eq!(
            lines.last().map(String::as_str),
            Some("x 0..38  y 0..19  of 39x20  you at (19, 10)")
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
