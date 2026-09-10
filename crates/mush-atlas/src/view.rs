//! The `+atlas` text views (doc 14 §6): a neighborhood as a map at five to ten
//! metres a character and a list; a region as its neighborhoods' footprints and its
//! corridors. Plain text, north up, at most eighty columns; a room the viewer may
//! not see is absent, not blanked.

use mush_core::{Flags, ObjId, ObjKind, World};

use crate::geom::{PointCm, RectCm, Shape, cm};
use crate::module::AtlasState;
use crate::raster::Grid;
use crate::record::{
    Access, ConnectionRec, Endpoint, NeighborhoodRec, PortalRec, RegionRec, RoomGeo, SiteRec,
    SurfaceRef,
};

/// The widest map, in characters.
pub const MAP_COLUMNS: u16 = 76;

/// The tallest map, in rows.
pub const MAP_ROWS: u16 = 40;

/// The scales a neighborhood map may use, centimetres per character, finest first.
pub const NEIGHBORHOOD_SCALES_CM: [u16; 5] = [500, 1000, 2000, 2500, 5000];

/// The scales a region map may use.
pub const REGION_SCALES_CM: [u16; 5] = [1000, 2500, 5000, 10_000, 25_000];

/// The most rooms a map letters before it runs out of glyphs.
pub const MAX_LETTERED: usize = 62;

/// The letter of the `i`th room on a map.
#[must_use]
pub fn letter(i: usize) -> char {
    const GLYPHS: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    GLYPHS.get(i).map_or('*', |b| char::from(*b))
}

/// Whether `viewer` may see `room` on a map: public rooms, any room a staffer asks
/// about, and the room the viewer is standing in.
#[must_use]
pub fn may_see(geo: &RoomGeo, w: &World, viewer: ObjId, staff: bool) -> bool {
    if staff || geo.access == Access::Public {
        return true;
    }
    w.get(viewer).and_then(|o| o.location) == Some(geo.room)
}

/// A character canvas.
struct Canvas {
    grid: Grid,
    rows: Vec<Vec<char>>,
}

impl Canvas {
    fn new(grid: Grid) -> Canvas {
        Canvas {
            rows: vec![vec![' '; usize::from(grid.width)]; usize::from(grid.height)],
            grid,
        }
    }

    fn put(&mut self, x: u16, y: u16, glyph: char) {
        // North up: row 0 is the top, the highest y.
        let row = usize::from(self.grid.height.saturating_sub(1).saturating_sub(y));
        if let Some(r) = self.rows.get_mut(row)
            && let Some(c) = r.get_mut(usize::from(x))
        {
            *c = glyph;
        }
    }

    fn fill(&mut self, shape: &Shape, glyph: char) {
        for (x, y) in self.grid.cells_in(shape) {
            self.put(x, y, glyph);
        }
    }

    fn ring(&mut self, shape: &Shape, glyph: char) {
        let ring = shape.ring();
        let mut points: Vec<PointCm> = ring.points().to_vec();
        if let Some(first) = points.first().copied() {
            points.push(first);
        }
        for (x, y) in self.grid.cells_along(&points) {
            self.put(x, y, glyph);
        }
    }

    fn line(&mut self, points: &[PointCm], glyph: char) {
        for (x, y) in self.grid.cells_along(points) {
            self.put(x, y, glyph);
        }
    }

    fn mark(&mut self, p: PointCm, glyph: char) {
        if let Some((x, y)) = self.grid.cell_of(p) {
            self.put(x, y, glyph);
        }
    }

    fn lines(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|r| {
                let s: String = r.iter().collect();
                format!(" {}", s.trim_end())
            })
            .collect()
    }
}

/// The finest scale from `scales` that fits `bbox` on the canvas.
fn scale_for(bbox: RectCm, scales: &[u16]) -> Option<Grid> {
    scales.iter().copied().find_map(|s| {
        Grid::over(bbox, s, u32::from(MAP_COLUMNS) * u32::from(MAP_ROWS))
            .filter(|g| g.width <= MAP_COLUMNS && g.height <= MAP_ROWS)
    })
}

fn clip(line: String) -> String {
    if line.chars().count() <= 80 {
        return line;
    }
    let mut out: String = line.chars().take(79).collect();
    out.push('…');
    out
}

pub(crate) fn occupants(w: &World, room: ObjId, viewer: ObjId) -> Vec<String> {
    let Some(obj) = w.get(room) else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    for id in &obj.contents {
        let Some(o) = w.get(*id) else {
            continue;
        };
        if !o.kind.is_actor() || o.flags.contains(Flags::DARK) || o.flags.contains(Flags::GOING) {
            continue;
        }
        if o.kind == ObjKind::Player && !w.is_connected(*id) {
            continue;
        }
        let name = mush_core::output::display_name(w, *id);
        names.push(if *id == viewer {
            format!("{name} (you)")
        } else {
            name
        });
        if names.len() >= 8 {
            names.push("…".to_string());
            break;
        }
    }
    names
}

fn scale_note(grid: &Grid) -> String {
    let m = f64::from(grid.resolution_cm) / 100.0;
    if (m - m.round()).abs() < f64::EPSILON {
        format!("{} m per character", m.round())
    } else {
        format!("{m:.1} m per character")
    }
}

/// Render `nb` for `viewer`.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn neighborhood(
    state: &AtlasState,
    w: &World,
    viewer: ObjId,
    nb: &NeighborhoodRec,
    staff: bool,
) -> Vec<String> {
    let r = &state.records;
    let locality = r
        .localities
        .get(&nb.locality)
        .map_or("?", |l| l.name.as_str());
    let region = r.regions.get(&nb.region).map_or("?", |g| g.name.as_str());
    let mut out: Vec<String> = Vec::new();
    out.push(clip(format!(
        "{} — {}, {} ({})",
        nb.name, locality, region, nb.setting
    )));
    let here = w.get(viewer).and_then(|o| o.location);
    let mut rooms: Vec<&RoomGeo> = r
        .rooms_in(&nb.key)
        .filter(|g| may_see(g, w, viewer, staff))
        .collect();
    rooms.sort_by_key(|g| g.room);
    if rooms.is_empty() {
        out.push(" Nothing of it is on your map.".to_string());
        return out;
    }
    let mut lettered: Vec<(char, &RoomGeo)> = Vec::new();
    for (i, geo) in rooms.iter().enumerate() {
        lettered.push((letter(i), geo));
    }
    // Surfaces present, ground first.
    let mut surfaces: Vec<SurfaceRef> = Vec::new();
    for (_, geo) in &lettered {
        if !surfaces.contains(&geo.surface) {
            surfaces.push(geo.surface.clone());
        }
    }
    surfaces.sort();
    for surface in &surfaces {
        let on_surface: Vec<(char, &RoomGeo)> = lettered
            .iter()
            .filter(|(_, g)| &g.surface == surface)
            .map(|(c, g)| (*c, *g))
            .collect();
        let (title, bbox, walls): (String, Option<RectCm>, Vec<Shape>) = match surface {
            SurfaceRef::Ground => {
                let bbox = nb.footprint.to_cm().ok().map(|p| p.bbox());
                let walls: Vec<Shape> = r
                    .structures
                    .values()
                    .filter(|s| {
                        r.sites
                            .get(&s.site)
                            .is_some_and(|site| site.neighborhoods.contains(&nb.key))
                    })
                    .filter_map(|s| s.footprint.to_cm().ok().map(Shape::Polygon))
                    .collect();
                ("Ground".to_string(), bbox, walls)
            }
            SurfaceRef::Floor { structure, index } => {
                let s = r.structures.get(structure);
                let site = s
                    .and_then(|s| r.sites.get(&s.site))
                    .map_or("?", |s| s.name.as_str());
                let floor = s
                    .and_then(|s| s.floor(*index))
                    .and_then(|f| f.name.clone())
                    .map_or_else(
                        || format!("floor {index}"),
                        |n| format!("floor {index}, {n}"),
                    );
                let bbox = s.and_then(|s| s.footprint.to_cm().ok()).map(|p| p.bbox());
                let walls: Vec<Shape> = s
                    .and_then(|s| s.footprint.to_cm().ok().map(Shape::Polygon))
                    .into_iter()
                    .collect();
                (format!("{site}, {floor}"), bbox, walls)
            }
        };
        let Some(bbox) = bbox else {
            continue;
        };
        let Some(grid) = scale_for(bbox, &NEIGHBORHOOD_SCALES_CM) else {
            out.push(clip(format!(" {title}: too large to draw.")));
            continue;
        };
        out.push(clip(format!(" {title} ({})", scale_note(&grid))));
        let mut canvas = Canvas::new(grid);
        for wall in &walls {
            canvas.ring(wall, '#');
        }
        for (glyph, geo) in &on_surface {
            if let Ok(shape) = geo.footprint.shape() {
                canvas.fill(&shape, *glyph);
            }
        }
        let visible_rooms: Vec<ObjId> = on_surface.iter().map(|(_, g)| g.room).collect();
        for portal in r.portals.values() {
            if portal.visibility == crate::record::Visibility::Hidden && !staff {
                continue;
            }
            let touches =
                |e: &Endpoint| matches!(e, Endpoint::Room(room) if visible_rooms.contains(room));
            if !(touches(&portal.from) || touches(&portal.to)) {
                continue;
            }
            if let Some(p) = portal_cm(portal) {
                canvas.mark(p, '+');
            }
        }
        if let Some(room) = here
            && let Some((_, geo)) = on_surface.iter().find(|(_, g)| g.room == room)
            && let Ok(shape) = geo.footprint.shape()
        {
            canvas.mark(shape.centre(), '@');
        }
        out.extend(canvas.lines());
        for (glyph, geo) in &on_surface {
            let name = mush_core::output::display_name(w, geo.room);
            let who = occupants(w, geo.room, viewer);
            let mut line = format!(" {glyph} {name}");
            if geo.access == Access::Restricted {
                line.push_str(" [restricted]");
            }
            if !who.is_empty() {
                line.push_str(" — ");
                line.push_str(&who.join(", "));
            }
            out.push(clip(line));
        }
    }
    out.push(" legend: # wall  + door or way  @ you".to_string());
    let hooks: Vec<String> = r
        .hooks
        .values()
        .filter(|h| h.key.pack == nb.key.pack)
        .map(|h| h.name.clone())
        .collect();
    if !hooks.is_empty() {
        out.push(clip(format!(" Going on: {}", hooks.join("; "))));
    }
    let connections: Vec<String> = r
        .connections_in(&nb.region)
        .filter(|c| c.visibility == crate::record::Visibility::Public || staff)
        .filter(|c| {
            c.endpoints.0.neighborhood.as_ref() == Some(&nb.key.pack)
                || c.endpoints.1.neighborhood.as_ref() == Some(&nb.key.pack)
        })
        .map(|c| connection_line(state, w, c))
        .collect();
    if !connections.is_empty() {
        out.push(" Ways out:".to_string());
        for c in connections {
            out.push(clip(format!("  {c}")));
        }
    }
    out
}

fn portal_cm(portal: &PortalRec) -> Option<PointCm> {
    let p = portal.point();
    Some(PointCm {
        x: cm(p.0).ok()?,
        y: cm(p.1).ok()?,
    })
}

fn neighborhood_name(state: &AtlasState, pack: &mush_core::CanonicalId) -> String {
    state
        .records
        .neighborhoods
        .values()
        .find(|n| &n.key.pack == pack)
        .map_or_else(|| pack.to_string(), |n| n.name.clone())
}

/// What an endpoint is called: its neighborhood, or — an orbital or system
/// site's room (S9.1) — the room itself.
fn endpoint_name(state: &AtlasState, w: &World, ap: &crate::record::AccessPoint) -> String {
    match (&ap.neighborhood, ap.room) {
        (Some(nb), _) => neighborhood_name(state, nb),
        (None, Some(room)) => mush_core::output::display_name(w, room),
        (None, None) => "nowhere".to_string(),
    }
}

fn connection_line(state: &AtlasState, w: &World, c: &ConnectionRec) -> String {
    let from = endpoint_name(state, w, &c.endpoints.0);
    let to = endpoint_name(state, w, &c.endpoints.1);
    let arrow = match c.direction {
        crate::record::Direction::Both => "<->",
        crate::record::Direction::OneWay => "->",
    };
    let via: Vec<String> = c
        .legs
        .iter()
        .filter_map(|l| {
            state
                .records
                .checkpoints
                .get(&l.to)
                .map(|cp| cp.name.clone())
        })
        .collect();
    let mut line = format!("{} ({}): {from} {arrow} {to}", c.name, c.modes.join(", "));
    if !via.is_empty() {
        line.push_str(&format!(" via {}", via.join(", ")));
    }
    if c.status == crate::record::ConnectionStatus::Inactive {
        line.push_str(" [not running]");
    }
    line
}

/// Render `region` for `viewer`: neighborhoods and corridors.
#[must_use]
pub fn region(
    state: &AtlasState,
    w: &World,
    viewer: ObjId,
    region: &RegionRec,
    staff: bool,
) -> Vec<String> {
    let r = &state.records;
    let mut out: Vec<String> = Vec::new();
    let kind = match &region.kind {
        crate::record::RegionKind::Surface { body } => format!("surface of {body}"),
        crate::record::RegionKind::Orbital { body } => format!("orbit of {body}"),
        crate::record::RegionKind::System => "system space".to_string(),
        crate::record::RegionKind::Interstellar => "interstellar space".to_string(),
    };
    out.push(clip(format!("{} — {kind}", region.name)));
    let Ok(bbox) = region.extent.to_cm() else {
        return out;
    };
    let mut nbs: Vec<&NeighborhoodRec> = r.neighborhoods_in(&region.key).collect();
    nbs.sort_by_key(|n| n.key.clone());
    let here = w
        .get(viewer)
        .and_then(|o| o.location)
        .and_then(|room| r.neighborhood_of_room(room))
        .map(|n| n.key.clone());
    match scale_for(bbox, &REGION_SCALES_CM) {
        Some(grid) => {
            out.push(clip(format!(" ({})", scale_note(&grid))));
            let mut canvas = Canvas::new(grid);
            for c in r.connections_in(&region.key) {
                if c.visibility == crate::record::Visibility::Hidden && !staff {
                    continue;
                }
                let points: Vec<PointCm> = c
                    .corridor
                    .iter()
                    .filter_map(|p| {
                        Some(PointCm {
                            x: cm(p.0).ok()?,
                            y: cm(p.1).ok()?,
                        })
                    })
                    .collect();
                canvas.line(&points, '=');
            }
            for cp in r.checkpoints.values().filter(|c| c.region == region.key) {
                if let (Ok(x), Ok(y)) = (cm(cp.point.0), cm(cp.point.1)) {
                    canvas.mark(PointCm { x, y }, 'o');
                }
            }
            for (i, nb) in nbs.iter().enumerate() {
                if let Ok(poly) = nb.footprint.to_cm() {
                    canvas.fill(&Shape::Polygon(poly), letter(i));
                }
            }
            if let Some(here) = &here
                && let Some(nb) = nbs.iter().find(|n| &n.key == here)
                && let Ok(poly) = nb.footprint.to_cm()
            {
                canvas.mark(poly.centre(), '@');
            }
            out.extend(canvas.lines());
        }
        None => out.push(" Too large to draw.".to_string()),
    }
    for (i, nb) in nbs.iter().enumerate() {
        let rooms = r
            .rooms_in(&nb.key)
            .filter(|g| may_see(g, w, viewer, staff))
            .count();
        let mut line = format!(
            " {} {} ({}) — {rooms} rooms",
            letter(i),
            nb.name,
            nb.setting
        );
        if Some(&nb.key) == here.as_ref() {
            line.push_str(" (you are here)");
        }
        out.push(clip(line));
    }
    // S9.1: sites anchored by the region alone — an orbital yard, a transfer
    // node — listed under it, since no neighborhood lists them.
    let mut orbital: Vec<&SiteRec> = r
        .sites
        .values()
        .filter(|s| s.neighborhoods.is_empty() && s.region.as_ref() == Some(&region.key))
        .collect();
    orbital.sort_by_key(|s| s.key.clone());
    for site in orbital {
        out.push(clip(format!(" * {} ({})", site.name, site.kind)));
    }
    let connections: Vec<String> = r
        .connections_in(&region.key)
        .filter(|c| c.visibility == crate::record::Visibility::Public || staff)
        .map(|c| connection_line(state, w, c))
        .collect();
    if !connections.is_empty() {
        out.push(" legend: = corridor  o checkpoint  @ you".to_string());
        for c in connections {
            out.push(clip(format!(" = {c}")));
        }
    }
    out
}
