//! **The ground chart** (doc 20 cut SM.9; §1 beat 2, §3.1's "a 56 × 28 ground chart", §5's R0,
//! §10, D-20-8, D-20-13; the SM.9 brief's lead rulings 1, 5 and 7): the `body.ground` pulled
//! grid feed and `+chart/ground` — one frame, three renders.
//!
//! **What it shows**: the Registry's survey (R0) of one body, the same for everyone — every
//! province of the body's 56 × 28 lattice washed by its word in one ore family (`.` barren, `-`
//! lean, `+` fair, `*` rich; [`WASH`](crate::port::ground_chart::WASH)), the lane's ports and
//! mining sites on it marked by letter — and, in the list form, each site's survey words, its
//! tenure, the passage there now for the viewer's hull at its load, and **the viewer's own best
//! reading** of it. The wash is a pure keyed function of the province (one HMAC a province
//! through `crate::surface::geology::province_word`), so two viewers' grids are identical; the
//! per-viewer keys are `you`, each site's `here`, `passage`, `best`, `dark` and `best_line` (the
//! list's own sentence for the readings, which a screen shows instead of writing one), and
//! `best` reads only the viewer's own chart row through the door's own gates
//! (`crate::port::site_map::best_reading`). The site the hull stands at carries no passage: its
//! line says where you are and nothing more.
//!
//! **One frame, three renders** (lead ruling 1): the frame is a grid on the `atlas.map` shape
//! (`crate::port::chart_panel`'s pattern) at `cell_columns 1`, rows run-length coded, **row 0
//! the south edge** as every grid frame's (`mush_ansi::panel`'s contract; the lattice's own
//! province row, so a province `(c, r)` is frame cell `(c, r)` and nothing is flipped); line
//! mode's `+chart/ground map` prints its rows through `mush_ansi::panel::window_rows`, north up,
//! one character a cell, and the TUI's quarry screen and the web's draw the same rows through
//! the same decoder. **Every glyph comes from the frame**: no renderer computes one.
//!
//! **No warm site, no want** (lead ruling 3; decision D14): the best reading is computed from
//! `crate::surface::chart::site_context` (a pure keyed context) and the viewer's row, so it
//! prints after any restart whether or not a hull is docked at the site; nothing here builds a
//! site's ground, wants one, writes or submits.
//!
//! **The passage at load** (decision D5): the command and the feed both plan through
//! `crate::port::chart::vantage_reading` — the viewer's docked charter at its load, else the
//! port's programme hull empty — so a viewer reads one frame on every surface.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{CanonicalId, CmdError, Ctx, HookReadContext, ObjId, QueryHandler, World};
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::pack::{LocationDef, PackEntry, RulesPack};
use crate::port::chart::{CHART_NOWHERE, Vantage, hours_text, plan_at};
use crate::port::chart_panel::run_length;
use crate::port::site_map::{
    BestReading, MAX_LINE_COLUMNS, SurfaceReads, best_reading, family_param, family_params,
};
use crate::store::TacsimStore;
use crate::surface::geology::{OreFamily, PROVINCE_COLUMNS, PROVINCE_ROWS, Word, province_word};

/// The feed (doc 20 §10). Registered by the tacsim layer whether or not it is enabled.
pub const BODY_GROUND_FEED: &str = "body.ground";
/// The frame's schema version.
pub const SCHEMA_VERSION: u32 = 1;
/// **The wash glyphs**, by the province's word in the family shown.
pub const WASH: [(Word, char); 4] = [
    (Word::Barren, '.'),
    (Word::Lean, '-'),
    (Word::Fair, '+'),
    (Word::Rich, '*'),
];
/// The map's title: `{body}`, `{family}`.
pub const GROUND_TITLE: &str = "{body}: the Registry's survey of {family}";
/// The list's head: `{body}`.
pub const GROUND_HEAD: &str =
    "{body}: the Registry's survey, the same for everyone. The sites, by letter:";
/// A port's line: `{glyph}`, `{port}`, `{lat}`, `{lon}`.
pub const GROUND_PORT_LINE: &str = "  {glyph} {port} ({lat} {lon}).";
/// A site's line: `{letter}`, `{site}`, `{lat}`, `{lon}`, `{here}`, `{passage}`.
pub const GROUND_SITE_LINE: &str = "  {letter} {site} ({lat} {lon}){here}: {passage}.";
/// A site's line with no vantage to plan from: `{letter}`, `{site}`, `{lat}`, `{lon}`,
/// `{here}`.
pub const GROUND_SITE_LINE_UNPLANNED: &str = "  {letter} {site} ({lat} {lon}){here}.";
/// `{here}` on the site the viewer's hull is docked at.
pub const GROUND_HERE: &str = ", where you are";
/// The passage planned at the charter's load: `{hours}`, `{burn}`.
pub const GROUND_PASSAGE_LOADED: &str = "passage {hours} at this load, burn {burn} t";
/// The passage planned for the port's hull empty: `{hours}`, `{burn}`.
pub const GROUND_PASSAGE_EMPTY: &str = "passage {hours} empty, burn {burn} t";
/// The vantage's port clears for no route there: `{port}`.
pub const GROUND_NO_PASSAGE: &str = "no passage from {port}";
/// The planner refused: `{error}`.
pub const GROUND_UNPLANNED: &str = "not plannable now ({error})";
/// The viewer's best reading: `{reading}`.
pub const GROUND_BEST: &str = "    Your best reading: {reading}.";
/// No reading of the site.
pub const GROUND_BEST_NONE: &str = "    Your readings: none.";
/// Every reading barren.
pub const GROUND_BEST_BARREN: &str = "    Your readings: barren wherever you have read.";
/// A row the instruments cannot render right now.
pub const GROUND_BEST_DARK: &str =
    "    Your readings: on your chart; the instruments cannot read the ground there right now.";
/// Markers hidden under another: `{glyph}`, `{list}`.
pub const GROUND_ALSO: &str = "Also at {glyph}: {list}.";
/// The map's footer.
pub const GROUND_FOOTER: &str =
    "56 by 28 provinces of 6.43° each, north up, 0° E at the left edge.";
/// No such body: `{list}`.
pub const GROUND_NO_BODY: &str =
    "No surveyed body is called that; the Registry has surveyed {list}.";
/// A lane body with no survey table: `{body}`.
pub const GROUND_UNSURVEYED: &str = "The Registry has no survey of {body}.";
/// The geology layer is not live.
pub const GROUND_DARK: &str = "The Registry's survey cannot be read right now.";
/// The verb's usage.
pub const GROUND_USAGE: &str =
    "Usage: +chart/ground [<body>]  or  +chart/ground map [<body>] [<family>]";

/// **A province's frame cell** (decision D17 as built — see the module doc): the grid shape's
/// row 0 is the south edge and so is the lattice's, so province `(c, r)` is frame cell `[c, r]`.
/// The one place the province-to-frame rule lives.
#[must_use]
pub fn frame_cell((c, r): (u32, u32)) -> [u32; 2] {
    [c, r]
}

/// The wash glyph of a word.
fn wash_glyph(word: Word) -> char {
    WASH.iter()
        .find(|(w, _)| *w == word)
        .map_or('.', |(_, glyph)| *glyph)
}

/// `"{d}.{t} N|S"` from milli-degrees, truncated to a tenth.
fn lat_text(mdeg: i32) -> String {
    let abs = mdeg.unsigned_abs();
    let hemi = if mdeg < 0 { 'S' } else { 'N' };
    format!("{}.{} {hemi}", abs / 1_000, (abs % 1_000) / 100)
}

/// `"{d}.{t} E"` from milli-degrees east, truncated to a tenth.
fn lon_text(mdeg: i32) -> String {
    let abs = mdeg.unsigned_abs();
    format!("{}.{} E", abs / 1_000, (abs % 1_000) / 100)
}

/// One marker on the chart.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Marker {
    /// Its letter.
    glyph: char,
    /// Its name.
    name: String,
    /// Its frame cell.
    cell: [u32; 2],
    /// Milli-degrees.
    lat_mdeg: i32,
    /// Milli-degrees east.
    lon_mdeg: i32,
}

/// **A port's letter** (the brief's 3.5 marker rule): the initial of its label's last word
/// (Ceres Freeport → `F`, doc 20 §3.1's "the port `F`"), else the first free letter.
fn port_letter(label: &str, taken: &mut BTreeSet<char>) -> char {
    let wanted = label
        .split_whitespace()
        .last()
        .and_then(|word| word.chars().next())
        .map(|c| c.to_ascii_uppercase());
    take_letter(wanted.into_iter(), taken)
}

/// **A site's letter**: the initial of its label's first word, else of its next word, and so
/// on, else the first free letter `A`–`Z`.
fn site_letter(label: &str, taken: &mut BTreeSet<char>) -> char {
    let initials: Vec<char> = label
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    take_letter(initials.into_iter(), taken)
}

/// The first of `wanted` not yet taken that is an ASCII capital, else the first free capital
/// (`?` past all twenty-six).
fn take_letter(wanted: impl Iterator<Item = char>, taken: &mut BTreeSet<char>) -> char {
    let pick = wanted
        .filter(char::is_ascii_uppercase)
        .find(|c| !taken.contains(c))
        .or_else(|| ('A'..='Z').find(|c| !taken.contains(c)))
        .unwrap_or('?');
    taken.insert(pick);
    pick
}

/// One lane site on the body, with what its rows need.
struct SiteMark {
    marker: Marker,
    entry: CanonicalId,
    venue: CanonicalId,
    location: CanonicalId,
    site: crate::port::def::SiteDef,
}

/// **The markers of `body`** (pure over the pack and the sites; the brief's 3.5): the ports
/// (`role` `port`, with coordinates) lettered first, then the sites in entry-id order.
fn markers(
    pack: &RulesPack,
    body: &CanonicalId,
    geology: &crate::surface::geology::BodyGeology,
    sites: &[(CanonicalId, &crate::port::def::PortDef)],
) -> (Vec<Marker>, Vec<SiteMark>) {
    let mut taken: BTreeSet<char> = BTreeSet::new();
    let cell_of = |location: &LocationDef| -> Option<[u32; 2]> {
        crate::port::site::site_province(location, geology)
            .ok()
            .map(frame_cell)
    };
    let mut ports: Vec<Marker> = Vec::new();
    for entry in pack.entries().values() {
        let PackEntry::Location(location) = entry else {
            continue;
        };
        if location.role.as_str() != "port" || location.body.as_ref() != Some(body) {
            continue;
        }
        let (Some(lat), Some(lon), Some(cell)) =
            (location.lat_mdeg, location.lon_mdeg, cell_of(location))
        else {
            continue;
        };
        ports.push(Marker {
            glyph: port_letter(&location.label, &mut taken),
            name: location.label.clone(),
            cell,
            lat_mdeg: lat,
            lon_mdeg: lon,
        });
    }
    let mut marks: Vec<SiteMark> = Vec::new();
    let mut ordered: Vec<&(CanonicalId, &crate::port::def::PortDef)> = sites.iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));
    for (entry, def) in ordered {
        let Some(site) = def.site.as_ref() else {
            continue;
        };
        let Some(PackEntry::Location(location)) = pack.entry(&def.location) else {
            continue;
        };
        if location.body.as_ref() != Some(body) {
            continue;
        }
        let (Some(lat), Some(lon), Some(cell)) =
            (location.lat_mdeg, location.lon_mdeg, cell_of(location))
        else {
            continue;
        };
        let name = crate::port::coop::port_name(def);
        marks.push(SiteMark {
            marker: Marker {
                glyph: site_letter(&name, &mut taken),
                name,
                cell,
                lat_mdeg: lat,
                lon_mdeg: lon,
            },
            entry: entry.clone(),
            venue: def.venue.clone(),
            location: def.location.clone(),
            site: site.clone(),
        });
    }
    (ports, marks)
}

/// **Paint the markers** (pure): the sites in entry order, then the ports — a port keeps a
/// shared cell — over the wash; each marker whose letter the grid does not show at its cell is
/// hidden, listed by the glyph it is under.
fn paint(
    grid: &mut [Vec<char>],
    ports: &[Marker],
    sites: &[&Marker],
) -> (BTreeSet<char>, BTreeMap<char, Vec<String>>) {
    let at = |cell: [u32; 2]| {
        (
            usize::try_from(cell[0]).unwrap_or(usize::MAX),
            usize::try_from(cell[1]).unwrap_or(usize::MAX),
        )
    };
    for marker in sites.iter().copied().chain(ports.iter()) {
        let (x, y) = at(marker.cell);
        if let Some(slot) = grid.get_mut(y).and_then(|row| row.get_mut(x)) {
            *slot = marker.glyph;
        }
    }
    let mut drawn: BTreeSet<char> = BTreeSet::new();
    let mut hidden: BTreeMap<char, Vec<String>> = BTreeMap::new();
    for marker in sites.iter().copied().chain(ports.iter()) {
        let (x, y) = at(marker.cell);
        match grid.get(y).and_then(|row| row.get(x)).copied() {
            Some(shown) if shown == marker.glyph => {
                drawn.insert(marker.glyph);
            }
            Some(shown) => hidden
                .entry(shown)
                .or_default()
                .push(format!("{} {}", marker.glyph, marker.name)),
            None => {}
        }
    }
    (drawn, hidden)
}

/// **Why a ground frame cannot be drawn**, or the frame.
///
/// # Errors
///
/// The line the command prints and the feed's reason frame titles: no such body
/// (`GROUND_NO_BODY`), a lane body with no survey (`GROUND_UNSURVEYED`), the layer not live
/// (`GROUND_DARK`), nowhere to read from (`CHART_NOWHERE`), or a chart row that will not read.
#[allow(clippy::too_many_lines)] // one frame: the body, the wash, the markers, the rows
pub(crate) fn ground_frame(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    body: Option<&str>,
    family: Option<OreFamily>,
    vantage: Option<&Vantage<'_>>,
) -> Result<Value, String> {
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| crate::port::chart::CHART_NO_LANE.to_owned())?;
    let (pack, ports_of) = crate::port::readiness::report_ports(store, w);
    let pack = pack.ok_or_else(|| crate::port::chart::CHART_NO_LANE.to_owned())?;
    let installed = crate::surface::installed_bodies(w);
    let helm = crate::port::coop::helm_of(w, actor);
    let docked = helm
        .as_ref()
        .and_then(|helm| match crate::port::ship::berth_of(w, helm.hull) {
            crate::port::ship::Berth::Docked { def, .. } => Some(def),
            crate::port::ship::Berth::UnderWay | crate::port::ship::Berth::Elsewhere => None,
        });
    let body_of_place = |place: &CanonicalId| match pack.entry(place) {
        Some(PackEntry::Location(location)) => location.body.clone(),
        _ => None,
    };
    // The body: the word, else the helm hull's docked body, else the vantage's.
    let body: CanonicalId = match body {
        Some(word) => {
            let wanted = word.trim().to_lowercase();
            let lane_body = pack.entries().iter().find_map(|(id, entry)| match entry {
                PackEntry::Body(_) if id.as_str() == wanted => Some(id.clone()),
                _ => None,
            });
            match lane_body {
                Some(id) if installed.contains(&id) => id,
                Some(id) => {
                    return Err(
                        GROUND_UNSURVEYED.replace("{body}", &crate::port::site::body_name(&id))
                    );
                }
                None => {
                    let mut list: Vec<String> = installed.iter().map(ToString::to_string).collect();
                    list.sort();
                    return Err(GROUND_NO_BODY.replace("{list}", &list.join(", ")));
                }
            }
        }
        None => docked
            .and_then(|def| body_of_place(&def.location))
            .or_else(|| vantage.map(|v| v.body.clone()))
            .ok_or_else(|| CHART_NOWHERE.to_owned())?,
    };
    let Some(geology) = crate::surface::body_geology(w, &body) else {
        return Err(GROUND_UNSURVEYED.replace("{body}", &crate::port::site::body_name(&body)));
    };
    let (mac, _) = crate::surface::live(w).map_err(|_| GROUND_DARK.to_owned())?;
    let family = family.unwrap_or(OreFamily::Metals);
    let name = crate::port::site::body_name(&body);
    // ---- the wash, row 0 the south edge ----
    let columns = usize::try_from(PROVINCE_COLUMNS).unwrap_or(56);
    let rows_n = usize::try_from(PROVINCE_ROWS).unwrap_or(28);
    let mut grid: Vec<Vec<char>> = Vec::with_capacity(rows_n);
    for r in 0..PROVINCE_ROWS {
        let mut row: Vec<char> = Vec::with_capacity(columns);
        for c in 0..PROVINCE_COLUMNS {
            let word = province_word(&mac, &geology, (c, r), family)
                .map_err(|_| GROUND_DARK.to_owned())?;
            row.push(wash_glyph(word));
        }
        grid.push(row);
    }
    // ---- the markers ----
    let sites_on: Vec<(CanonicalId, &crate::port::def::PortDef)> = ports_of
        .iter()
        .filter(|(_, def)| def.site.is_some())
        .map(|(entry, def)| (entry.clone(), *def))
        .collect();
    let (ports, sites) = markers(pack, &body, &geology, &sites_on);
    let site_markers: Vec<&Marker> = sites.iter().map(|s| &s.marker).collect();
    let (drawn, hidden) = paint(&mut grid, &ports, &site_markers);
    let rows: Vec<Value> = grid.iter().map(|row| run_length(row)).collect();
    // ---- where the viewer is ----
    let standing = {
        let here = w.get(actor).and_then(|o| o.location).map(|room| room.0);
        crate::port::voyage::lane_ports(w)
            .into_iter()
            .find(|(_, map)| {
                map.is_some_and(|map| {
                    here.is_some_and(|room| {
                        map.host == Some(room) || map.presentation == Some(room)
                    })
                })
            })
            .map(|(def, _)| def)
    };
    let cell_on_body = |def: &crate::port::def::PortDef| -> Option<[u32; 2]> {
        let Some(PackEntry::Location(location)) = pack.entry(&def.location) else {
            return None;
        };
        if location.body.as_ref() != Some(&body) {
            return None;
        }
        crate::port::site::site_province(location, &geology)
            .ok()
            .map(frame_cell)
    };
    let you = docked
        .and_then(cell_on_body)
        .or_else(|| standing.and_then(cell_on_body));
    // ---- per site ----
    let now = w.world_ic_now().at;
    let dests = vantage.map(|v| crate::port::voyage::destinations(w, v.def));
    let mut site_rows: Vec<(char, Value, Vec<String>)> = Vec::new();
    for mark in &sites {
        let here = docked.is_some_and(|def| def.location == mark.location);
        // The site the hull stands at has no passage to itself: its line says where you are
        // and nothing more (`GROUND_SITE_LINE_UNPLANNED`'s form with `GROUND_HERE`), never "no
        // passage from" the very place it names.
        let passage: Option<String> = match (vantage, &dests) {
            _ if here => None,
            (Some(v), Some(dests)) => Some(match dests.iter().find(|d| d.venue == mark.venue) {
                None => GROUND_NO_PASSAGE.replace("{port}", &crate::port::coop::port_name(v.def)),
                Some(dest) => match plan_at(v, dest, v.payload_kg, now) {
                    Ok(plan) => {
                        let form = if v.loaded {
                            GROUND_PASSAGE_LOADED
                        } else {
                            GROUND_PASSAGE_EMPTY
                        };
                        form.replace("{hours}", &hours_text(w, plan.total_s))
                            .replace("{burn}", &plan.burn_t.to_string())
                    }
                    Err(error) => GROUND_UNPLANNED.replace("{error}", &error.to_string()),
                },
            }),
            _ => None,
        };
        let survey = crate::port::site::survey_words(w, pack, &mark.location);
        // The site's tenure from its block records (doc 20 cut SM.10, lead note 0a).
        let tenure =
            crate::port::claims::tenure_parts_now(w, reads, pack, &mark.entry, &mark.site, now);
        let row = reads.chart(actor, &mark.entry)?;
        let (best, dark) = match crate::surface::chart::site_context(w, &mark.entry) {
            Ok(site) => (
                match best_reading(&site, row.as_ref()) {
                    BestReading::None => None,
                    BestReading::Barren => Some("barren".to_owned()),
                    BestReading::Pocket(text) | BestReading::Field(text) => Some(text),
                },
                false,
            ),
            Err(_) => (None, row.is_some()),
        };
        let lat = lat_text(mark.marker.lat_mdeg);
        let lon = lon_text(mark.marker.lon_mdeg);
        let head = match &passage {
            Some(passage) => GROUND_SITE_LINE.replace("{passage}", passage),
            None => GROUND_SITE_LINE_UNPLANNED.to_owned(),
        }
        .replace("{letter}", &mark.marker.glyph.to_string())
        .replace("{site}", &mark.marker.name)
        .replace("{lat}", &lat)
        .replace("{lon}", &lon)
        .replace("{here}", if here { GROUND_HERE } else { "" });
        let mut lines = vec![head];
        lines.push(survey.as_ref().map_or_else(
            || crate::port::site::SURVEY_UNAVAILABLE.to_owned(),
            |words| crate::port::site::SURVEY_LINE.replace("{words}", words),
        ));
        lines.push(crate::port::site::TENURE_LINE.replace("{parts}", &tenure));
        let best_line = match (&best, dark) {
            (_, true) => GROUND_BEST_DARK.to_owned(),
            (Some(text), false) if text == "barren" => GROUND_BEST_BARREN.to_owned(),
            (Some(text), false) => GROUND_BEST.replace("{reading}", text),
            (None, false) => GROUND_BEST_NONE.to_owned(),
        };
        lines.push(best_line.clone());
        site_rows.push((
            mark.marker.glyph,
            json!({
                "letter": mark.marker.glyph.to_string(),
                "site": mark.marker.name,
                "entry": mark.entry.to_string(),
                "venue": mark.venue.to_string(),
                "cell": mark.marker.cell,
                "drawn": drawn.contains(&mark.marker.glyph),
                "lat_mdeg": mark.marker.lat_mdeg,
                "lon_mdeg": mark.marker.lon_mdeg,
                "survey": survey,
                "tenure": tenure,
                "passage": passage,
                "here": here,
                "best": best,
                "dark": dark,
                // The list's own words for the viewer's readings there, unindented — so a
                // screen shows the server's sentence (none, barren, dark or the best), never
                // one of its own (lead ruling 1).
                "best_line": best_line.trim_start(),
            }),
            lines,
        ));
    }
    site_rows.sort_by_key(|(letter, _, _)| *letter);
    // ---- the legend: the wash, then the ports, then the sites by letter ----
    let mut legend: Vec<Value> = vec![
        json!([".", "barren"]),
        json!(["-", "lean"]),
        json!(["+", "fair"]),
        json!(["*", "rich"]),
    ];
    for port in &ports {
        legend.push(json!([port.glyph.to_string(), port.name]));
    }
    for (letter, row, _) in &site_rows {
        legend.push(json!([
            letter.to_string(),
            row.get("site").cloned().unwrap_or(Value::Null)
        ]));
    }
    let also: Vec<String> = hidden.values().flatten().cloned().collect();
    let also_lines: Vec<String> = hidden
        .iter()
        .map(|(glyph, list)| {
            GROUND_ALSO
                .replace("{glyph}", &glyph.to_string())
                .replace("{list}", &list.join(", "))
        })
        .collect();
    // ---- the list form ----
    let mut lines: Vec<String> = wrap_kept(&GROUND_HEAD.replace("{body}", &name));
    for port in &ports {
        lines.extend(wrap_kept(
            &GROUND_PORT_LINE
                .replace("{glyph}", &port.glyph.to_string())
                .replace("{port}", &port.name)
                .replace("{lat}", &lat_text(port.lat_mdeg))
                .replace("{lon}", &lon_text(port.lon_mdeg)),
        ));
    }
    for (_, _, site_lines) in &site_rows {
        for line in site_lines {
            lines.extend(wrap_kept(line));
        }
    }
    let title = GROUND_TITLE
        .replace("{body}", &name)
        .replace("{family}", family.label());
    let mut frame = json!({
        "schema": { "id": BODY_GROUND_FEED, "shape": "grid", "version": SCHEMA_VERSION },
        "window": 0,
        "revision": 0,
        "tick": 0,
        "title": title,
        "surface": "ground",
        "floors": [],
        "origin": [0, 0],
        "cell_cm": 0,
        "cell_columns": 1,
        "width": PROVINCE_COLUMNS,
        "height": PROVINCE_ROWS,
        "rows": rows,
        "legend": legend,
        "units": [],
        "you": you,
        "ground": {
            "body": body.to_string(),
            "name": name,
            "family": family.label(),
            "family_word": family_param(family),
            "families": family_params(),
            "ports": ports.iter().map(|port| json!({
                "glyph": port.glyph.to_string(),
                "port": port.name,
                "cell": port.cell,
                "drawn": drawn.contains(&port.glyph),
                "lat_mdeg": port.lat_mdeg,
                "lon_mdeg": port.lon_mdeg,
            })).collect::<Vec<_>>(),
            "sites": site_rows.iter().map(|(_, row, _)| row.clone()).collect::<Vec<_>>(),
            "also": also,
            "footer": GROUND_FOOTER,
            "lines": lines,
        },
    });
    let mut print = vec![
        frame
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    ];
    print.extend(legend_print(frame.get("legend")));
    print.extend(grid_print(&frame));
    print.push(GROUND_FOOTER.to_owned());
    print.extend(also_lines);
    if let Some(block) = frame.get_mut("ground").and_then(Value::as_object_mut) {
        block.insert("print".to_owned(), json!(print));
    }
    Ok(frame)
}

/// **Wrap one list line at [`MAX_LINE_COLUMNS`], keeping its indent**: the site map's
/// `site_map::wrap` reads a line with no indent, and the list's survey and tenure lines are indented
/// four spaces (`SURVEY_LINE`), so the indent is kept and a continuation is indented two more.
fn wrap_kept(line: &str) -> Vec<String> {
    let body = line.trim_start_matches(' ');
    let indent = line
        .get(..line.len().saturating_sub(body.len()))
        .unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    let mut current = indent.to_owned();
    let mut words = 0_usize;
    for word in body.split(' ') {
        let len = current.chars().count();
        if words > 0 && len + 1 + word.chars().count() > MAX_LINE_COLUMNS {
            out.push(std::mem::take(&mut current));
            current = format!("{indent}  {word}");
            words = 1;
        } else {
            if words > 0 {
                current.push(' ');
            }
            current.push_str(word);
            words += 1;
        }
    }
    out.push(current);
    out
}

/// **The legend as printed** (no second string: built from the frame's pairs): `"legend: "`
/// then `"{glyph} {label}"` entries joined by two spaces, broken at entry boundaries into lines
/// of at most [`MAX_LINE_COLUMNS`].
fn legend_print(legend: Option<&Value>) -> Vec<String> {
    let entries: Vec<String> = legend
        .and_then(Value::as_array)
        .map(|pairs| {
            pairs
                .iter()
                .filter_map(|pair| {
                    Some(format!(
                        "{} {}",
                        pair.get(0)?.as_str()?,
                        pair.get(1)?.as_str()?
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut current = "legend:".to_owned();
    for entry in entries {
        let sep = if current == "legend:" { " " } else { "  " };
        if current.chars().count() + sep.len() + entry.chars().count() > MAX_LINE_COLUMNS
            && current != "legend:"
        {
            lines.push(std::mem::take(&mut current));
            current.push_str("  ");
            current.push_str(&entry);
        } else {
            current.push_str(sep);
            current.push_str(&entry);
        }
    }
    lines.push(current);
    lines
}

/// **The grid as printed**: the frame's rows through `mush_ansi::panel::window_rows` — the TUI's
/// own decoder — north up (the rows run from the south edge, so they print in reverse), one
/// character a cell.
fn grid_print(frame: &Value) -> Vec<String> {
    let num = |key: &str| {
        frame
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0)
    };
    let (width, height) = (num("width"), num("height"));
    let empty = Vec::new();
    let raw = frame
        .get("rows")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    mush_ansi::panel::window_rows(raw, width, height, 0, 0, width, height)
        .iter()
        .rev()
        .map(|row| row.iter().collect())
        .collect()
}

/// Register the feed and its query (doc 20 cut SM.9): the handle is dropped, as `sky.chart`'s
/// and `site.map`'s are — nothing pushes a ground chart.
///
/// # Errors
///
/// The registrar's, for a feed or query name already taken.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    drop(reg.panel_feed(BODY_GROUND_FEED)?);
    reg.query_handler(BODY_GROUND_FEED, Arc::new(GroundQuery))?;
    Ok(())
}

/// **`body.ground`'s handler**: the asker's frame read through the typed snapshot, or a frame
/// with no grid and the reason as its title (the line `+chart/ground` would print).
struct GroundQuery;

/// The reason frame.
fn reason_frame(why: &str) -> Value {
    json!({
        "schema": { "id": BODY_GROUND_FEED, "shape": "grid", "version": SCHEMA_VERSION },
        "window": 0,
        "title": why,
        "width": 0,
        "height": 0,
        "cell_cm": 0,
        "cell_columns": 1,
        "rows": [],
        "legend": [],
        "units": [],
        "you": null,
    })
}

impl QueryHandler for GroundQuery {
    fn query(&self, _w: &World, _actor: ObjId, _params: &Value) -> Result<OutputEvent, RichText> {
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(BODY_GROUND_FEED),
            kind: PanelFrameKind::Keyframe,
            data: reason_frame(crate::port::tv::TV_UNWIRED),
        })
    }

    fn query_with_context(
        &self,
        context: &HookReadContext<'_>,
        actor: ObjId,
        params: &Value,
    ) -> Result<OutputEvent, RichText> {
        let w = context.world();
        let body = params.get("body").and_then(Value::as_str);
        let family = match params.get("family").and_then(Value::as_str) {
            None => Ok(None),
            Some(word) => crate::surface::geology::family_of_word(word)
                .map(Some)
                .ok_or_else(|| crate::port::tv::MAP_FAMILY_UNKNOWN.replace("{word}", word)),
        };
        let data = match family {
            Ok(family) => {
                let vantage = crate::port::chart::vantage_reading(w, context, actor).ok();
                ground_frame(w, context, actor, body, family, vantage.as_ref())
                    .unwrap_or_else(|line| reason_frame(&line))
            }
            Err(line) => reason_frame(&line),
        };
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(BODY_GROUND_FEED),
            kind: PanelFrameKind::Keyframe,
            data,
        })
    }
}

/// **The parsed `+chart/ground` words**: the map form, the body word, the family.
#[derive(Clone, Debug, PartialEq, Eq)]
struct GroundArgs {
    map: bool,
    body: Option<String>,
    family: Option<OreFamily>,
}

/// **Parse `+chart/ground [<body>]` and `+chart/ground map [<body>] [<family>]`** (pure): a
/// leading `map` selects the map form; then an optional body and, in the map form only, an
/// optional family word (a word `family_of_word` accepts is the family, any other the body);
/// anything else is the usage.
fn parse_ground(raw: &str) -> Option<GroundArgs> {
    let mut words: Vec<&str> = raw.split_whitespace().collect();
    let map = words
        .first()
        .is_some_and(|first| first.eq_ignore_ascii_case("map"));
    if map {
        words.remove(0);
    }
    let mut args = GroundArgs {
        map,
        body: None,
        family: None,
    };
    if words.len() > if map { 2 } else { 1 } {
        return None;
    }
    for word in words {
        match crate::surface::geology::family_of_word(word) {
            Some(family) if map && args.family.is_none() => args.family = Some(family),
            Some(_) => return None,
            None if args.body.is_none() => args.body = Some(word.to_owned()),
            None => return None,
        }
    }
    Some(args)
}

/// **`+chart/ground [<body>]` and `+chart/ground map [<body>] [<family>]`** (doc 20 cut SM.9;
/// §1 beat 2, §3.1): a pure read — no write, no want, no submission — of the actor's own ground
/// frame, built with `crate::port::chart::vantage_reading` (the function the feed calls), each
/// line said to the actor (the `+chart/map` way): the list form prints `ground.lines`, the map
/// form `ground.print`.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
pub(crate) fn ground_cmd(ctx: &mut Ctx<'_>, raw: &str) -> Result<(), CmdError> {
    let args = parse_ground(raw).ok_or_else(|| CmdError::msg(GROUND_USAGE))?;
    let actor = ctx.actor;
    let frame = {
        let reads: &Ctx<'_> = ctx;
        let vantage = crate::port::chart::vantage_reading(reads.w, reads, actor).ok();
        ground_frame(
            reads.w,
            reads,
            actor,
            args.body.as_deref(),
            args.family,
            vantage.as_ref(),
        )
        .map_err(CmdError::msg)?
    };
    let key = if args.map { "print" } else { "lines" };
    let lines: Vec<String> = frame
        .get("ground")
        .and_then(|block| block.get(key))
        .and_then(Value::as_array)
        .map(|lines| {
            lines
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    for line in lines {
        ctx.out.sys_to_actor(actor, "tacsim", line);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The frame's rows run from the south edge** (decision D17 as built): a province's frame
    /// cell is its own `(c, r)`, so a marker at a positive latitude lands in a row `≥ 14` and one
    /// at a negative latitude in a row `< 14` — the grid shape's convention, which every
    /// renderer prints north up.
    #[test]
    fn sm9_frame_rows_run_from_the_south_edge() {
        let lattice = crate::surface::geology::Lattice::of_radius(470);
        // A pocket's province by its latitude's sign: the lattice's row 0 is the south edge.
        let rows = lattice.rows;
        let above = lattice
            .province_of_pocket(0, rows / 4)
            .map(frame_cell)
            .unwrap_or_else(|e| panic!("{e}"));
        let below = lattice
            .province_of_pocket(0, -rows / 4)
            .map(frame_cell)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            above[1] >= 14,
            "a northern cell is in a northern row: {above:?}"
        );
        assert!(
            below[1] < 14,
            "a southern cell is in a southern row: {below:?}"
        );
        assert_eq!(frame_cell((37, 17)), [37, 17]);
        assert_eq!(lat_text(19_700), "19.7 N");
        assert_eq!(lat_text(-10_550), "10.5 S");
        assert_eq!(lon_text(240_490), "240.4 E");
    }

    /// **The markers are lettered deterministically** (the brief's 3.5): a port by its label's
    /// last word, each site by its first word's initial, else the next word's, else the first
    /// free letter; a port keeps a shared cell and the hidden markers are listed by the glyph
    /// over them.
    #[test]
    fn sm9_markers_are_lettered_deterministically() {
        let mut taken = BTreeSet::new();
        assert_eq!(port_letter("Ceres Freeport", &mut taken), 'F');
        let names = [
            "Ahuna Flank",
            "Freeport Commons",
            "Juling Shadow",
            "Kerwan Floor",
            "Occator Rim",
        ];
        let letters: Vec<char> = names
            .iter()
            .map(|name| site_letter(name, &mut taken))
            .collect();
        assert_eq!(letters, ['A', 'C', 'J', 'K', 'O']);
        let mut vesta = BTreeSet::new();
        assert_eq!(site_letter("Rheasilvia Commons", &mut vesta), 'R');
        // Past every initial: the first free letter.
        let mut full: BTreeSet<char> = ['A', 'B', 'C'].into_iter().collect();
        assert_eq!(site_letter("Alpha Beta Charlie", &mut full), 'D');
        // A port keeps a shared cell; the sites under it are listed by its glyph.
        let marker = |glyph: char, name: &str, cell: [u32; 2]| Marker {
            glyph,
            name: name.to_owned(),
            cell,
            lat_mdeg: 0,
            lon_mdeg: 0,
        };
        let mut grid = vec![vec!['.'; 4]; 2];
        let ports = [marker('F', "Ceres Freeport", [1, 1])];
        let a = marker('A', "Ahuna Flank", [3, 0]);
        let c = marker('C', "Freeport Commons", [1, 1]);
        let o = marker('O', "Occator Rim", [1, 1]);
        let (drawn, hidden) = paint(&mut grid, &ports, &[&a, &c, &o]);
        assert_eq!(drawn, ['A', 'F'].into_iter().collect());
        assert_eq!(
            hidden.get(&'F'),
            Some(&vec![
                "C Freeport Commons".to_owned(),
                "O Occator Rim".to_owned()
            ])
        );
        assert_eq!(
            grid.iter()
                .map(|r| r.iter().collect::<String>())
                .collect::<Vec<_>>(),
            ["...A", ".F.."]
        );
    }

    /// **The words parse** (the brief's 3.5's verb): a leading `map`, an optional body, a family
    /// only in the map form, nothing past two words.
    #[test]
    fn sm9_the_ground_words_parse() {
        let args = |map: bool, body: Option<&str>, family: Option<OreFamily>| GroundArgs {
            map,
            body: body.map(str::to_owned),
            family,
        };
        assert_eq!(parse_ground(""), Some(args(false, None, None)));
        assert_eq!(
            parse_ground("ceres"),
            Some(args(false, Some("ceres"), None))
        );
        assert_eq!(parse_ground("map"), Some(args(true, None, None)));
        assert_eq!(
            parse_ground("map ceres ice"),
            Some(args(true, Some("ceres"), Some(OreFamily::Volatiles)))
        );
        assert_eq!(
            parse_ground("MAP metals vesta"),
            Some(args(true, Some("vesta"), Some(OreFamily::Metals)))
        );
        assert_eq!(
            parse_ground("ceres metals"),
            None,
            "a family in the list form"
        );
        assert_eq!(parse_ground("ceres extra words"), None);
        assert_eq!(parse_ground("map ceres vesta"), None, "two bodies");
        assert_eq!(parse_ground("map ice metals"), None, "two families");
        assert_eq!(parse_ground("metals"), None);
    }

    /// A list line keeps its indent and wraps within the floor.
    #[test]
    fn sm9_a_list_line_keeps_its_indent() {
        assert_eq!(wrap_kept("    Tenure: 16 open."), ["    Tenure: 16 open."]);
        assert_eq!(wrap_kept("  F Ceres Freeport."), ["  F Ceres Freeport."]);
        let long = format!("    Registry survey: {}.", ["metals fair"; 20].join(", "));
        let wrapped = wrap_kept(&long);
        assert!(wrapped.len() > 1);
        assert!(
            wrapped
                .iter()
                .all(|l| l.chars().count() <= MAX_LINE_COLUMNS)
        );
        assert!(wrapped[0].starts_with("    Registry survey: "));
        assert!(wrapped.iter().skip(1).all(|l| l.starts_with("      ")));
        assert_eq!(
            wrapped.join(" ").split_whitespace().collect::<Vec<_>>(),
            long.split_whitespace().collect::<Vec<_>>()
        );
    }

    /// The legend prints within the floor, from the pairs, whole entries a line.
    #[test]
    fn sm9_the_ground_legend_prints_from_the_pairs() {
        let pairs = json!([
            [".", "barren"],
            ["-", "lean"],
            ["+", "fair"],
            ["*", "rich"],
            ["F", "Ceres Freeport"]
        ]);
        assert_eq!(
            legend_print(Some(&pairs)),
            ["legend: . barren  - lean  + fair  * rich  F Ceres Freeport"]
        );
        let long: Vec<Value> = (0..20)
            .map(|i| {
                json!([
                    format!("{}", char::from(b'A' + i)),
                    "a fairly long site name here"
                ])
            })
            .collect();
        let printed = legend_print(Some(&Value::Array(long)));
        assert!(printed.len() > 1);
        assert!(
            printed
                .iter()
                .all(|l| l.chars().count() <= MAX_LINE_COLUMNS)
        );
        assert!(printed.iter().skip(1).all(|l| l.starts_with("  ")));
    }
}
