//! **The sky chart panel** (doc 19 §5, cut A.3b): the `sky.chart` feed — one
//! frame, three renders.
//!
//! The frame is a **grid** frame (`schema.shape == "grid"`, the shape `atlas.map`
//! and the arena map already wear) with a `chart` block beside the grid:
//!
//! - the **grid** is the server's flat render of the sky — the Sun at the centre,
//!   every body's orbit as a ring at a square-root radial scale, the bodies as
//!   letters, the listed routes as dotted lines, the vantage port as the
//!   viewer's `you` cell — which the TUI shows in its chart pane and `+chart/map`
//!   prints in line mode through the one grid printer (`mush_ansi::grid_lines`,
//!   doc 06 §10's text fallback), so a line player and a TUI player see the same
//!   characters;
//! - the **`chart`** block is the vector data the web client draws its own
//!   picture from — each body's heliocentric position, distance, longitude,
//!   latitude, period and parent; the routes' legs and passages now; the window
//!   over the next IC year — and the **lines** `+chart` would print, so a renderer
//!   with no composer of its own prints the server's text.
//!
//! Pulled, never pushed: a `PanelSubscribe` on the feed answers one keyframe for
//! the asker's vantage (doc 19 §5: the docked charter's port, else the port whose
//! room they stand in, else the first lane port; the panel plans the port's
//! programme hull **empty** — `+chart` from a helm plans the charter's load). The
//! sky moves by the IC day, so a client asks again when it wants a newer one.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{CanonicalId, ObjId, QueryHandler, World};
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::ephemeris::AU_KM;
use crate::port::chart::{
    Vantage, WINDOW_DAYS, body_of, direct_at, plan_at, sky_lines, vantage_for_world,
    window_lines_for,
};
use crate::port::coop::port_name;
use crate::port::voyage::destinations;
use crate::sky::epoch_date;

/// The feed. Registered by the tacsim layer whether or not it is enabled, as
/// every tacsim feed is: a disabled layer answers with the reason.
pub const SKY_CHART_FEED: &str = "sky.chart";

/// The frame's schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// The flat render's size: 60 columns by 28 rows, so a full ring fits the
/// 120-column floor's panes and line mode's 78 columns with room for the
/// legend, two columns to a row for a terminal cell's aspect.
pub const GRID_WIDTH: usize = 60;
/// See [`GRID_WIDTH`].
pub const GRID_HEIGHT: usize = 28;

/// The glyphs the flat render draws with.
pub const SUN_GLYPH: char = '*';
/// See [`SUN_GLYPH`].
pub const ORBIT_GLYPH: char = '.';
/// See [`SUN_GLYPH`].
pub const ROUTE_GLYPH: char = ':';

/// The letters the lane's bodies draw as; any other body takes the first
/// letter of its id that no body has taken.
const LETTERS: &[(&str, char)] = &[
    ("mercury", 'm'),
    ("venus", 'v'),
    ("earth", 'E'),
    ("mars", 'M'),
    ("jupiter", 'J'),
    ("saturn", 'S'),
    ("ceres", 'C'),
    ("pallas", 'P'),
    ("juno", 'j'),
    ("vesta", 'V'),
    ("hygiea", 'H'),
    ("kirkwood", 'K'),
    ("luna", 'L'),
    ("phobos", 'p'),
    ("deimos", 'd'),
    ("titan", 'T'),
];

/// Register the feed and its query (doc 19 cut A.3b): the handle is dropped,
/// as the arena map's is — nothing pushes a chart.
///
/// # Errors
///
/// The registrar's, for a feed or query name already taken.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    drop(reg.panel_feed(SKY_CHART_FEED)?);
    reg.query_handler(SKY_CHART_FEED, Arc::new(ChartQuery))?;
    Ok(())
}

/// **`sky.chart`'s handler**: the asker's chart, or a frame with no grid and the
/// reason as its title (no lane, nowhere the lane knows), which the grid printer
/// prints as that one line and the web shows as its sentence.
struct ChartQuery;

impl ChartQuery {
    /// The frame, or the reason frame; `reads` reads the sites' block records for their tenure
    /// lines (doc 20 cut SM.10, lead note 0a), the authored tenure without.
    fn answer(
        w: &World,
        reads: Option<&dyn crate::port::site_map::SurfaceReads>,
        actor: ObjId,
    ) -> OutputEvent {
        let data = chart_frame_reading(w, reads, actor).unwrap_or_else(|why| {
            json!({
                "schema": { "id": SKY_CHART_FEED, "shape": "grid", "version": SCHEMA_VERSION },
                "window": 0,
                "title": why,
                "width": 0,
                "height": 0,
                "rows": [],
                "legend": [],
                "units": [],
                "you": null,
            })
        });
        OutputEvent::Panel {
            feed: SmolStr::new_static(SKY_CHART_FEED),
            kind: PanelFrameKind::Keyframe,
            data,
        }
    }
}

impl QueryHandler for ChartQuery {
    fn query(&self, w: &World, actor: ObjId, _params: &Value) -> Result<OutputEvent, RichText> {
        Ok(Self::answer(w, None, actor))
    }

    fn query_with_context(
        &self,
        context: &mush_core::HookReadContext<'_>,
        actor: ObjId,
        _params: &Value,
    ) -> Result<OutputEvent, RichText> {
        Ok(Self::answer(context.world(), Some(context), actor))
    }
}

/// One body as the chart carries it.
#[derive(Clone, Debug)]
pub struct ChartBody {
    pub id: CanonicalId,
    pub glyph: char,
    pub parent: Option<CanonicalId>,
    pub r_au: f64,
    pub lon_deg: f64,
    pub lat_deg: f64,
    pub x_au: f64,
    pub y_au: f64,
    pub z_au: f64,
    pub period_days: Option<f64>,
    pub here: bool,
    pub from_here_km: Option<u64>,
    pub lag_s: Option<u64>,
}

/// The flat render: `height` rows of `width` glyphs, row 0 the SOUTH edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raster {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<Vec<char>>,
    /// Where each body landed, by id.
    pub cells_of: Vec<(CanonicalId, (usize, usize))>,
}

/// The radial scale's knee, AU: inside it the picture is near linear, outside
/// it logarithmic (Mercury's 0.39 AU is just past it).
pub const R0_AU: f64 = 0.3;

/// **The radial scale** (doc 19 §5): a body sits at `ln(1 + r / R0) / ln(1 +
/// au_max / R0)` of the picture's radius — logarithmic past [`R0_AU`] — so
/// Mercury and Saturn share a picture with Earth, Mars and the Belt still a row
/// apart on a 28-row grid (a square root packed the six inner orbits into seven
/// rows); clamped to the edge. The web's `radialFraction` is the same curve.
#[must_use]
pub fn radial_fraction(r_au: f64, au_max: f64) -> f64 {
    if au_max.is_nan() || r_au.is_nan() || au_max <= 0.0 || r_au <= 0.0 {
        return 0.0;
    }
    ((1.0 + r_au / R0_AU).ln() / (1.0 + au_max / R0_AU).ln()).min(1.0)
}

/// The cell a heliocentric distance and longitude land in, on a `width` by
/// `height` grid with the Sun at the centre, two columns to a row, ecliptic
/// north up (row 0 is the south edge, so a northern body has the larger row).
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn cell_of(
    r_au: f64,
    lon_deg: f64,
    au_max: f64,
    width: usize,
    height: usize,
) -> (usize, usize) {
    let (w, h) = (width.max(1) as f64, height.max(1) as f64);
    let (cx, cy) = ((w - 1.0) / 2.0, (h - 1.0) / 2.0);
    let radius_rows = (cy * 0.96).min((cx / 2.0) * 0.96);
    let f = radial_fraction(r_au, au_max);
    let theta = lon_deg.to_radians();
    let x = (cx + 2.0 * radius_rows * f * theta.cos()).round();
    let y = (cy + radius_rows * f * theta.sin()).round();
    (
        (x.max(0.0) as usize).min(width.saturating_sub(1)),
        (y.max(0.0) as usize).min(height.saturating_sub(1)),
    )
}

/// **The flat render** (pure): rings for every body about the Sun, the routes'
/// legs as dotted lines body to body, then the letters, then the Sun; a moon
/// shares its parent's cell and draws no letter of its own.
#[must_use]
pub fn rasterize(
    bodies: &[ChartBody],
    legs: &[(CanonicalId, CanonicalId)],
    au_max: f64,
    width: usize,
    height: usize,
) -> Raster {
    let mut cells = vec![vec![' '; width]; height];
    let put = |cells: &mut Vec<Vec<char>>, x: usize, y: usize, glyph: char, over: bool| {
        if let Some(cell) = cells.get_mut(y).and_then(|row| row.get_mut(x))
            && (over || *cell == ' ')
        {
            *cell = glyph;
        }
    };
    // The rings, one per body about the Sun, dotted every five degrees so two
    // orbits a row apart stay two rings and not one blot.
    for body in bodies.iter().filter(|b| b.parent.is_none()) {
        for deg in (0..360).step_by(5) {
            let (x, y) = cell_of(body.r_au, f64::from(deg), au_max, width, height);
            put(&mut cells, x, y, ORBIT_GLYPH, false);
        }
    }
    let cells_of: Vec<(CanonicalId, (usize, usize))> = bodies
        .iter()
        .map(|b| {
            (
                b.id.clone(),
                cell_of(b.r_au, b.lon_deg, au_max, width, height),
            )
        })
        .collect();
    let at = |id: &CanonicalId| cells_of.iter().find(|(b, _)| b == id).map(|(_, c)| *c);
    // The routes' legs, body to body.
    for (from, to) in legs {
        if let (Some(a), Some(b)) = (at(from), at(to)) {
            for (x, y) in line_cells(a, b) {
                put(&mut cells, x, y, ROUTE_GLYPH, true);
            }
        }
    }
    // The letters, planets over their rings and routes; a moon draws nothing.
    // Nearest the Sun first and the first letter on a cell keeps it (Kirkwood
    // trails Ceres by 2°, one cell at this scale), except the body the chart is
    // read from, drawn last over everything: the viewer's own place is never
    // hidden under a neighbour.
    for body in bodies.iter().filter(|b| b.parent.is_none() && !b.here) {
        if let Some((x, y)) = at(&body.id) {
            let free = cells
                .get(y)
                .and_then(|row| row.get(x))
                .is_some_and(|c| *c == ' ' || *c == ORBIT_GLYPH || *c == ROUTE_GLYPH);
            if free {
                put(&mut cells, x, y, body.glyph, true);
            }
        }
    }
    for body in bodies.iter().filter(|b| b.here) {
        let anchor = body
            .parent
            .as_ref()
            .and_then(|parent| bodies.iter().find(|b| &b.id == parent))
            .unwrap_or(body);
        if let Some((x, y)) = at(&anchor.id) {
            put(&mut cells, x, y, anchor.glyph, true);
        }
    }
    let (sx, sy) = cell_of(0.0, 0.0, au_max, width, height);
    put(&mut cells, sx, sy, SUN_GLYPH, true);
    Raster {
        width,
        height,
        cells,
        cells_of,
    }
}

/// The cells of a straight line between two cells (Bresenham), both ends in.
#[must_use]
#[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
pub fn line_cells(a: (usize, usize), b: (usize, usize)) -> Vec<(usize, usize)> {
    let (mut x0, mut y0) = (a.0 as i64, a.1 as i64);
    let (x1, y1) = (b.0 as i64, b.1 as i64);
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut out = Vec::new();
    for _ in 0..4096 {
        out.push((x0 as usize, y0 as usize));
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
    out
}

/// The grid's rows as strings, row 0 the south edge (the grid shape's `rows`
/// run-length pairs expanded; a renderer's `expand_rows` in miniature).
#[must_use]
pub fn expand_rows(frame: &Value) -> Vec<String> {
    let width = frame.get("width").and_then(Value::as_u64).unwrap_or(0) as usize;
    frame
        .get("rows")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    let mut out = String::new();
                    for run in row.as_array().into_iter().flatten() {
                        let glyph = run
                            .get(0)
                            .and_then(Value::as_str)
                            .and_then(|g| g.chars().next())
                            .unwrap_or(' ');
                        let n = run.get(1).and_then(Value::as_u64).unwrap_or(1) as usize;
                        out.extend(std::iter::repeat_n(glyph, n));
                    }
                    while out.chars().count() < width {
                        out.push(' ');
                    }
                    out
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A row of glyphs as the grid shape's `[glyph, run]` pairs.
#[must_use]
pub fn run_length(row: &[char]) -> Value {
    let mut runs: Vec<Value> = Vec::new();
    let mut i = 0;
    while i < row.len() {
        let glyph = row[i];
        let mut n = 1;
        while i + n < row.len() && row[i + n] == glyph {
            n += 1;
        }
        runs.push(json!([glyph.to_string(), n]));
        i += n;
    }
    Value::Array(runs)
}

/// The letter a body draws as.
fn glyph_for(id: &CanonicalId, taken: &mut BTreeSet<char>) -> char {
    if let Some((_, glyph)) = LETTERS.iter().find(|(name, _)| *name == id.as_str())
        && !taken.contains(glyph)
    {
        taken.insert(*glyph);
        return *glyph;
    }
    for candidate in id
        .as_str()
        .chars()
        .filter(char::is_ascii_alphabetic)
        .flat_map(|c| [c.to_ascii_uppercase(), c.to_ascii_lowercase()])
    {
        if !taken.contains(&candidate) {
            taken.insert(candidate);
            return candidate;
        }
    }
    '?'
}

/// **The chart frame for `actor`** (the module doc): the grid, the `chart`
/// block and the lines — a site's tenure line here the authored one; the `sky.chart` feed and
/// `+chart/map` read the sites' block records (doc 20 cut SM.10).
///
/// # Errors
///
/// The vantage's refusal: no lane, or nowhere the lane knows.
pub fn chart_frame(w: &World, actor: ObjId) -> Result<Value, String> {
    chart_frame_reading(w, None, actor)
}

/// **[`chart_frame`] with the sites' block records read** through `reads` (doc 20 cut SM.10,
/// lead note 0a: a site's tenure line counts its live claims): the `sky.chart` feed's own
/// answer and `+chart/map`'s; `None` prints the authored tenure, as [`chart_frame`] does.
///
/// # Errors
///
/// The vantage's refusal: no lane, or nowhere the lane knows.
#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
pub(crate) fn chart_frame_reading(
    w: &World,
    reads: Option<&dyn crate::port::site_map::SurfaceReads>,
    actor: ObjId,
) -> Result<Value, String> {
    let v: Vantage<'_> = vantage_for_world(w, actor)?;
    let now = w.world_ic_now().at;
    let rate = v.sky.ephemeris().rate_permille();
    // The bodies, nearest the Sun first.
    let mut taken: BTreeSet<char> = BTreeSet::from([SUN_GLYPH, ORBIT_GLYPH, ROUTE_GLYPH, '@']);
    let mut bodies: Vec<ChartBody> = v
        .sky
        .ephemeris()
        .bodies()
        .filter_map(|(id, _)| {
            let (r_km, lon, lat) = v.sky.heliocentric(id, now)?;
            let at = v.sky.position_km(id, now)?;
            let here = id == &v.body;
            let (from_here_km, lag_s) = if here {
                (None, None)
            } else {
                (
                    v.sky.distance_km(&v.body, now, id, now),
                    v.sky.light_lag_s(&v.body, now, id, now),
                )
            };
            Some(ChartBody {
                id: id.clone(),
                glyph: ' ',
                parent: v.sky.parent_of(id).cloned(),
                r_au: r_km / AU_KM as f64,
                lon_deg: lon,
                lat_deg: lat,
                x_au: at.x as f64 / AU_KM as f64,
                y_au: at.y as f64 / AU_KM as f64,
                z_au: at.z as f64 / AU_KM as f64,
                period_days: v.sky.period_days(id, now),
                here,
                from_here_km,
                lag_s,
            })
        })
        .collect();
    bodies.sort_by(|a, b| {
        a.r_au
            .partial_cmp(&b.r_au)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for body in &mut bodies {
        body.glyph = glyph_for(&body.id, &mut taken);
    }
    let au_max = bodies.iter().map(|b| b.r_au).fold(1.0_f64, f64::max) * 1.02;
    // The destinations: their bodies, legs and passages now.
    let dests = destinations(w, v.def);
    let empty_kg = u64::from(v.spec.allowance_kg) * u64::from(v.occupants.max(1));
    let mut routes: Vec<Value> = Vec::new();
    let mut legs: Vec<(CanonicalId, CanonicalId)> = Vec::new();
    let mut windows: Vec<Value> = Vec::new();
    for dest in &dests {
        let Some(to) = v.pack.entries().iter().find_map(|(_, entry)| match entry {
            crate::pack::PackEntry::Port(port) if port.venue == dest.venue => {
                body_of(v.pack, &port.location)
            }
            _ => None,
        }) else {
            continue;
        };
        let loaded = plan_at(&v, dest, v.payload_kg, now).ok();
        let bare = plan_at(&v, dest, empty_kg, now).ok();
        // Doc 20 cut SM.2: no direct chord on the vantage's own body (`sky_lines`' rule).
        let direct = (to != v.body)
            .then(|| direct_at(&v, &to, v.payload_kg, now))
            .flatten();
        let mut route_legs: Vec<Value> = Vec::new();
        if let Some(plan) = loaded.as_ref().or(bare.as_ref()) {
            let mut from = v.body.clone();
            for leg in &plan.legs {
                route_legs.push(json!({ "from": from.to_string(), "to": leg.body.to_string() }));
                legs.push((from.clone(), leg.body.clone()));
                from = leg.body.clone();
            }
        }
        let passage = |plan: &crate::port::families::PassagePlan| json!({ "total_s": plan.total_s, "burn_t": plan.burn_t, "crew_days": plan.crew_days });
        routes.push(json!({
            "to": dest.port,
            "venue": dest.venue.to_string(),
            "body": to.to_string(),
            "legs": route_legs,
            "loaded": loaded.as_ref().map(passage),
            "empty": bare.as_ref().map(passage),
            "direct": direct.map(|(km, secs, burn)| json!({ "km": km, "total_s": secs, "burn_t": burn })),
        }));
        // The window: every IC day of the next year, when the sky moves.
        if let Some(today) = loaded.as_ref() {
            let mut window = json!({ "to": dest.port, "now": passage(today) });
            if rate != 0 {
                let mut days: Vec<(i64, i64, u64)> = Vec::new();
                for offset in 0..=WINDOW_DAYS {
                    let depart = now.saturating_add(offset.saturating_mul(86_400));
                    if let Ok(plan) = plan_at(&v, dest, v.payload_kg, depart) {
                        days.push((offset, plan.total_s, plan.burn_t));
                    }
                }
                let day = |d: &(i64, i64, u64)| json!({ "offset_days": d.0, "total_s": d.1, "burn_t": d.2 });
                if let Some(fastest) = days.iter().min_by_key(|d| (d.1, d.0)) {
                    let cheapest = days
                        .iter()
                        .min_by_key(|d| (d.2, d.1, d.0))
                        .unwrap_or(fastest);
                    let worst = days.iter().max_by_key(|d| (d.1, d.2)).unwrap_or(fastest);
                    window["fastest"] = day(fastest);
                    window["cheapest"] = day(cheapest);
                    window["worst"] = day(worst);
                }
            }
            windows.push(window);
        }
    }
    // The flat render.
    let raster = rasterize(&bodies, &legs, au_max, GRID_WIDTH, GRID_HEIGHT);
    let you = raster
        .cells_of
        .iter()
        .find(|(id, _)| id == &v.body)
        .map(|(_, c)| *c);
    let rows: Vec<Value> = raster.cells.iter().map(|row| run_length(row)).collect();
    let mut legend: Vec<Value> = vec![json!([SUN_GLYPH.to_string(), "the Sun"])];
    for body in bodies.iter().filter(|b| b.parent.is_none()) {
        let moons: Vec<String> = bodies
            .iter()
            .filter(|m| m.parent.as_ref() == Some(&body.id))
            .map(|m| m.id.to_string())
            .collect();
        let mut label = body.id.to_string();
        if body.here
            || moons
                .iter()
                .any(|m| bodies.iter().any(|b| b.here && b.id.as_str() == m))
        {
            label.push_str(" (here)");
        }
        if !moons.is_empty() {
            label.push_str(&format!(" with {}", moons.join(", ")));
        }
        legend.push(json!([body.glyph.to_string(), label]));
    }
    legend.push(json!([ORBIT_GLYPH.to_string(), "an orbit"]));
    if !legs.is_empty() {
        legend.push(json!([ROUTE_GLYPH.to_string(), "the listed routes"]));
    }
    let motion = if rate == 0 {
        "constant".to_owned()
    } else {
        format!("moving, rate {rate}")
    };
    let title = format!(
        "The sky on {} ({}; {}), from {} on {}",
        epoch_date(now),
        v.sky.model(),
        motion,
        port_name(v.def),
        v.body
    );
    let body_rows: Vec<Value> = bodies
        .iter()
        .map(|b| {
            let cell = raster
                .cells_of
                .iter()
                .find(|(id, _)| id == &b.id)
                .map_or((0, 0), |(_, c)| *c);
            json!({
                "id": b.id.to_string(),
                "label": b.id.to_string(),
                "glyph": b.glyph.to_string(),
                "parent": b.parent.as_ref().map(ToString::to_string),
                "r_au": b.r_au,
                "lon_deg": b.lon_deg,
                "lat_deg": b.lat_deg,
                "x_au": b.x_au,
                "y_au": b.y_au,
                "z_au": b.z_au,
                "period_days": b.period_days,
                "here": b.here,
                "from_here_au": b.from_here_km.map(|km| km as f64 / AU_KM as f64),
                "lag_s": b.lag_s,
                "cell": [cell.0, cell.1],
            })
        })
        .collect();
    // The lines `+chart` prints, for a renderer that composes nothing itself.
    let sky = sky_lines(w, reads, &v, now);
    let window_text: Vec<String> = dests
        .iter()
        .flat_map(|dest| window_lines_for(w, &v, dest, now).unwrap_or_default())
        .collect();
    Ok(json!({
        "schema": { "id": SKY_CHART_FEED, "shape": "grid", "version": SCHEMA_VERSION },
        "window": 0,
        "revision": 0,
        "tick": 0,
        "title": title,
        "surface": "ecliptic",
        "floors": [],
        "origin": [0, 0],
        "cell_cm": 0,
        "cell_columns": 1,
        "width": raster.width,
        "height": raster.height,
        "rows": rows,
        "legend": legend,
        "units": you.map_or_else(Vec::new, |(x, y)| vec![json!({
            "id": "here", "name": port_name(v.def), "cell": [x, y], "glyph": "@", "you": true
        })]),
        "you": you.map(|(x, y)| json!([x, y])),
        "chart": {
            "date": epoch_date(now),
            "ic_now": now,
            "model": v.sky.model(),
            "rate_permille": rate,
            "vantage": {
                "port": port_name(v.def),
                "body": v.body.to_string(),
                "hull": v.hull,
                "payload_kg": v.payload_kg,
                "occupants": v.occupants,
            },
            "scale": { "kind": "log", "au_max": au_max, "r0_au": R0_AU },
            "bodies": body_rows,
            "routes": routes,
            "window": windows,
            "lines": { "sky": sky, "window": window_text },
        },
    }))
}

/// **`+chart/map`'s lines** (doc 19 §5): the frame's title, the legend, the
/// flat render north up (the whole grid, not line mode's viewport) and a scale
/// line — the TUI's chart pane flattened for a line player.
///
/// # Errors
///
/// The vantage's refusal.
pub fn flat_lines(w: &World, actor: ObjId) -> Result<Vec<String>, String> {
    flat_lines_reading(w, None, actor)
}

/// **[`flat_lines`] with the sites' block records read** through `reads` (doc 20 cut SM.10,
/// lead note 0a): `+chart/map`'s own lines.
///
/// # Errors
///
/// The vantage's refusal.
pub(crate) fn flat_lines_reading(
    w: &World,
    reads: Option<&dyn crate::port::site_map::SurfaceReads>,
    actor: ObjId,
) -> Result<Vec<String>, String> {
    let frame = chart_frame_reading(w, reads, actor)?;
    let mut lines = vec![
        frame
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("The sky")
            .to_owned(),
    ];
    let legend: Vec<String> = frame
        .get("legend")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| {
                    let pair = e.as_array()?;
                    Some(format!(
                        "{} {}",
                        pair.first()?.as_str()?,
                        pair.get(1)?.as_str()?
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    lines.push(legend.join("   "));
    // North up; the blank rows at the top and bottom edges are not printed (a
    // line of nothing says nothing, and the output path drops an empty line).
    let rows: Vec<String> = expand_rows(&frame)
        .iter()
        .rev()
        .map(|row| row.trim_end().to_owned())
        .collect();
    let first = rows.iter().position(|r| !r.is_empty()).unwrap_or(0);
    let last = rows
        .iter()
        .rposition(|r| !r.is_empty())
        .map_or(0, |i| i + 1);
    lines.extend(rows[first..last].iter().cloned());
    let au_max = frame
        .pointer("/chart/scale/au_max")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    lines.push(format!(
        "scale: logarithmic past {R0_AU} AU, the edge at {au_max:.1} AU; north up; two columns to a row."
    ));
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(id: &str, r_au: f64, lon_deg: f64, parent: Option<&str>) -> ChartBody {
        ChartBody {
            id: CanonicalId::new(id).expect("id"),
            glyph: id.chars().next().expect("a letter").to_ascii_uppercase(),
            parent: parent.map(|p| CanonicalId::new(p).expect("id")),
            r_au,
            lon_deg,
            lat_deg: 0.0,
            x_au: r_au * lon_deg.to_radians().cos(),
            y_au: r_au * lon_deg.to_radians().sin(),
            z_au: 0.0,
            period_days: None,
            here: false,
            from_here_km: None,
            lag_s: None,
        }
    }

    #[test]
    fn a3b_the_radial_scale_is_logarithmic_and_clamps() {
        assert!((radial_fraction(9.6, 9.6) - 1.0).abs() < 1e-12);
        // ln(1 + 2.4 / 0.3) / ln(1 + 9.6 / 0.3) = ln 9 / ln 33.
        assert!((radial_fraction(2.4, 9.6) - (9.0_f64.ln() / 33.0_f64.ln())).abs() < 1e-12);
        assert!(radial_fraction(1.0, 9.6) > radial_fraction(0.39, 9.6));
        assert_eq!(radial_fraction(0.0, 9.6), 0.0);
        assert_eq!(radial_fraction(100.0, 9.6), 1.0);
        let sun = cell_of(0.0, 0.0, 9.6, 60, 28);
        assert_eq!(sun, (30, 14), "the Sun at the centre (rounded up)");
        let east = cell_of(9.6, 0.0, 9.6, 60, 28);
        assert_eq!(east.1, 14, "longitude 0 on the centre row");
        assert!(east.0 > 50, "and near the right edge: {east:?}");
        let north = cell_of(9.6, 90.0, 9.6, 60, 28);
        assert!(north.1 > 25, "longitude 90 near the top row: {north:?}");
        assert_eq!(north.0, 30, "and on the centre column");
    }

    #[test]
    fn a3b_the_raster_draws_rings_routes_letters_and_the_sun_and_no_moon() {
        let bodies = vec![
            body("mars", 1.5, 90.0, None),
            body("phobos", 1.5, 90.0, Some("mars")),
            body("ceres", 2.8, 0.0, None),
        ];
        let legs = vec![(
            CanonicalId::new("ceres").expect("id"),
            CanonicalId::new("mars").expect("id"),
        )];
        let raster = rasterize(&bodies, &legs, 3.0, 60, 28);
        assert_eq!(raster.cells.len(), 28);
        assert!(raster.cells.iter().all(|row| row.len() == 60));
        let text: String = raster
            .cells
            .iter()
            .rev()
            .map(|r| r.iter().collect::<String>() + "\n")
            .collect();
        assert!(
            text.contains('M') && text.contains('C') && text.contains('*'),
            "{text}"
        );
        assert!(
            !text.contains('P'),
            "a moon draws no letter of its own:\n{text}"
        );
        // The body the chart is read from is drawn last, over a neighbour on its
        // cell (Kirkwood on Ceres's); a later neighbour never covers an earlier.
        let mut here = body("ceres", 2.8, 0.0, None);
        here.here = true;
        let trailing = body("kirkwood", 2.8, 0.0, None);
        let shared = rasterize(&[here, trailing], &[], 3.0, 60, 28);
        let shared_text: String = shared
            .cells
            .iter()
            .rev()
            .map(|r| r.iter().collect::<String>() + "\n")
            .collect();
        assert!(
            shared_text.contains('C') && !shared_text.contains('K'),
            "{shared_text}"
        );
        assert!(
            text.contains(ORBIT_GLYPH) && text.contains(ROUTE_GLYPH),
            "{text}"
        );
        let mars = raster
            .cells_of
            .iter()
            .find(|(id, _)| id.as_str() == "mars")
            .map(|(_, c)| *c);
        let phobos = raster
            .cells_of
            .iter()
            .find(|(id, _)| id.as_str() == "phobos")
            .map(|(_, c)| *c);
        assert_eq!(mars, phobos, "a moon shares its parent's cell");
        // Rows run-length encode and decode to the same glyphs.
        let encoded = run_length(&raster.cells[14]);
        let runs = encoded.as_array().expect("runs");
        let total: u64 = runs.iter().map(|r| r[1].as_u64().expect("n")).sum();
        assert_eq!(total, 60);
    }

    #[test]
    fn a3b_a_line_joins_its_ends() {
        let cells = line_cells((0, 0), (5, 2));
        assert_eq!(cells.first(), Some(&(0, 0)));
        assert_eq!(cells.last(), Some(&(5, 2)));
        assert_eq!(cells.len(), 6);
        assert_eq!(line_cells((3, 3), (3, 3)), vec![(3, 3)]);
    }

    #[test]
    fn a3b_letters_are_the_lane_table_or_the_first_free_letter() {
        let mut taken = BTreeSet::new();
        assert_eq!(
            glyph_for(&CanonicalId::new("mars").expect("id"), &mut taken),
            'M'
        );
        assert_eq!(
            glyph_for(&CanonicalId::new("mercury").expect("id"), &mut taken),
            'm'
        );
        assert_eq!(
            glyph_for(&CanonicalId::new("marduk").expect("id"), &mut taken),
            'A'
        );
        assert_eq!(
            glyph_for(&CanonicalId::new("mars").expect("id"), &mut taken),
            'a',
            "taken: the next free letter"
        );
    }
}
