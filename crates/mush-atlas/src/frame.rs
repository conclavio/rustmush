//! The `atlas.map` panel feed (doc 14 §6; plan §3.5): a window as a frame of
//! glyph cells, its legend and the units on it, **built per viewer** — the
//! rooms they may not see are void, the units in those rooms absent (V) — and
//! emitted as a keyframe when a composite lands or a viewer asks (`+map`), and
//! as a delta when someone moves on it. The same frame drives line mode (the
//! text renderer in `mush-ansi`), the TUI pane and the web canvas; the text is
//! the acceptance test (P10).

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{Audience, Ctx, ObjId, PanelFeedHandle, World};
use mush_proto::{AtlasMapDelta, AtlasMapFrame, MapSchema, MapUnit};
use mush_sim::{DomainId, Pose};

use crate::module::{AtlasState, is_staff};
use crate::record::{RoomGeo, SurfaceRef};
use crate::terrain::TerrainClass;
use crate::view::may_see;
use crate::window::{Composite, WindowEntry};

/// The feed id.
pub const MAP_FEED: &str = "atlas.map";

/// The frame version.
pub const MAP_SCHEMA_VERSION: u32 = 1;

/// The most units one frame lists.
pub const MAX_FRAME_UNITS: usize = 256;

/// The feed's handle, held in a state slot so the window code can publish.
#[derive(Clone, Default)]
pub struct MapFeed(pub Option<PanelFeedHandle>);

impl std::fmt::Debug for MapFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapFeed")
            .field("registered", &self.0.is_some())
            .finish()
    }
}

fn schema() -> MapSchema {
    MapSchema {
        id: MAP_FEED.to_string(),
        version: MAP_SCHEMA_VERSION,
        shape: "grid".to_string(),
    }
}

/// A surface's label: `ground` or `floor <n>`.
#[must_use]
pub fn surface_label(surface: &SurfaceRef) -> String {
    match surface {
        SurfaceRef::Ground => "ground".to_string(),
        SurfaceRef::Floor { index, .. } => format!("floor {index}"),
    }
}

/// The surface, cell and elevation gap a pose stands at.
fn locate(composite: &Composite, pose: &Pose) -> Option<(usize, u32, u32)> {
    let p = crate::geom::PointCm {
        x: pose.x_cm,
        y: pose.y_cm,
    };
    let z_cm = i64::from(pose.z_cm);
    composite
        .surfaces
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let (x, y) = s.cell_of(p)?;
            let cell = s.cells.get(x, y)?;
            (cell.class != TerrainClass::Void).then_some((
                i,
                u32::from(x),
                u32::from(y),
                (i64::from(cell.elev_cm) - z_cm).abs(),
            ))
        })
        .min_by_key(|(_, _, _, dz)| *dz)
        .map(|(i, x, y, _)| (i, x, y))
}

/// Which surface `viewer` looks at: the floor they chose, else the one they
/// stand on, else their room's, else the first.
fn surface_index(w: &World, st: &AtlasState, entry: &WindowEntry, viewer: ObjId) -> usize {
    if let Some(i) = st.windows.viewer_floor.get(&viewer)
        && *i < entry.frame.surfaces.len()
    {
        return *i;
    }
    if let Some(composite) = &entry.composite
        && let Some((_, pose)) = mush_sim::pose_of(w, viewer)
        && let Some((i, _, _)) = locate(composite, &pose)
    {
        return i;
    }
    if let Some(room) = w.get(viewer).and_then(|o| o.location)
        && let Some(geo) = st.records.roomgeo.get(&room)
        && let Some(i) = entry.frame.surfaces.iter().position(|s| *s == geo.surface)
    {
        return i;
    }
    0
}

/// The rooms `viewer` may see on this window.
///
/// **16b cut S.2** (doc 16 §3.6 "World-side exclusion"): none at all while a
/// simulation authority holds custody of a unit mustered in this window's domain.
/// The exclusion is here, and not only in [`units_for`], because the window's rooms
/// are what [`crate::window::summary_for`] names and what `frame_for` draws terrain
/// for — an arena's ground is not somewhere the world may look, for any viewer
/// including staff.
pub(crate) fn visible_rooms(
    w: &World,
    st: &AtlasState,
    entry: &WindowEntry,
    viewer: ObjId,
) -> BTreeSet<ObjId> {
    if mush_sim::deployed::excluded(w, mush_sim::deployed::Shown::Domain(DomainId(entry.domain))) {
        return BTreeSet::new();
    }
    let staff = is_staff(w, viewer);
    let here = w.get(viewer).and_then(|o| o.location);
    entry
        .rooms
        .iter()
        .copied()
        .filter(|room| {
            Some(*room) == here
                || st
                    .records
                    .roomgeo
                    .get(room)
                    .is_some_and(|geo: &RoomGeo| may_see(geo, w, viewer, staff))
        })
        .collect()
}

/// The units `viewer` sees on `surface`: everyone placed there whose room the
/// viewer may see, the viewer first.
///
/// **16b cut S.2** (doc 16 §3.6 "World-side exclusion"): nobody at all while a
/// simulation authority holds custody of a unit mustered in `domain`. A deployed
/// unit is already absent because [`mush_sim::participants`] projects it away, and
/// this is the *domain's* half of the same rule — the whole window goes, so a
/// teammate or a bystander standing on an arena's ground is no more visible than
/// the tanks are.
fn units_for(
    w: &World,
    composite: &Composite,
    surface: usize,
    domain: DomainId,
    viewer: ObjId,
    visible: &BTreeSet<ObjId>,
) -> (Vec<MapUnit>, Option<(u32, u32)>) {
    let mut units = Vec::new();
    let mut you = None;
    if mush_sim::deployed::excluded(w, mush_sim::deployed::Shown::Domain(domain)) {
        return (units, you);
    }
    let mut placed = mush_sim::participants(w, domain);
    placed.sort_by_key(|(o, _)| (*o != viewer, o.0));
    for (obj, pose) in placed.into_iter().take(MAX_FRAME_UNITS) {
        let Some((si, x, y)) = locate(composite, &pose) else {
            continue;
        };
        if si != surface {
            continue;
        }
        let Some(o) = w.get(obj) else {
            continue;
        };
        if o.flags.contains(mush_core::Flags::GOING) {
            continue;
        }
        let room = o.location;
        let seen = obj == viewer || room.is_none_or(|r| visible.contains(&r));
        if !seen {
            continue;
        }
        let is_you = obj == viewer;
        if is_you {
            you = Some((x, y));
        }
        units.push(MapUnit {
            id: obj.to_string(),
            name: mush_core::output::display_name(w, obj),
            cell: (x, y),
            glyph: if is_you { "@" } else { "o" }.to_string(),
            you: is_you,
        });
    }
    (units, you)
}

fn run_length(glyphs: &[char]) -> Vec<(String, u32)> {
    let mut runs: Vec<(String, u32)> = Vec::new();
    for &g in glyphs {
        match runs.last_mut() {
            Some((last, n)) if last.starts_with(g) => *n += 1,
            _ => runs.push((g.to_string(), 1)),
        }
    }
    runs
}

/// The frame `viewer` sees of `domain`'s window, or none when the window is
/// not open or not drawn.
///
/// **16b cut S.2** (doc 16 §3.6 "World-side exclusion"): also none while a
/// simulation authority holds custody of a unit mustered in `domain` — the arena's
/// domain is excluded from *the atlas `+map` frame*, and `+map` renders a closed
/// window exactly as it renders a domain that has none, which is the answer with
/// the least in it.
#[must_use]
pub fn frame_for(
    w: &World,
    st: &AtlasState,
    viewer: ObjId,
    domain: DomainId,
) -> Option<AtlasMapFrame> {
    if mush_sim::deployed::excluded(w, mush_sim::deployed::Shown::Domain(domain)) {
        return None;
    }
    let entry = st.windows.open.get(&domain.0)?;
    let composite = entry.composite.as_ref()?;
    let si = surface_index(w, st, entry, viewer);
    let surface = composite.surfaces.get(si)?;
    let visible = visible_rooms(w, st, entry, viewer);
    // A floor the viewer may see no room of is not theirs at all: its walls and
    // its margins go with its rooms (V). The ground's open cells always show.
    let floor_shown = matches!(surface.surface, SurfaceRef::Ground)
        || surface.rooms.iter().any(|r| visible.contains(r));
    let mut rows = Vec::with_capacity(usize::from(surface.grid.height));
    let mut classes: BTreeMap<u8, TerrainClass> = BTreeMap::new();
    for y in 0..surface.grid.height {
        let mut glyphs: Vec<char> = Vec::with_capacity(usize::from(surface.grid.width));
        for x in 0..surface.grid.width {
            let Some(cell) = surface.cells.get(x, y) else {
                glyphs.push(' ');
                continue;
            };
            let room = surface.room_at(x, y);
            let shown = floor_shown && room.is_none_or(|r| visible.contains(&r));
            if !shown || cell.class == TerrainClass::Void {
                glyphs.push(' ');
                continue;
            }
            classes.insert(cell.class.code(), cell.class);
            glyphs.push(cell.class.glyph());
        }
        rows.push(run_length(&glyphs));
    }
    let mut legend: Vec<(String, String)> = classes
        .values()
        .map(|c| (c.glyph().to_string(), c.label().to_string()))
        .collect();
    legend.push(("@".to_string(), "you".to_string()));
    legend.push(("o".to_string(), "other".to_string()));
    let (units, you) = units_for(w, composite, si, domain, viewer, &visible);
    let here = w.get(viewer).and_then(|o| o.location).map_or_else(
        || "the map".to_string(),
        |r| mush_core::output::display_name(w, r),
    );
    Some(AtlasMapFrame {
        schema: schema(),
        window: domain.0,
        revision: entry.revision,
        tick: 0,
        title: here,
        surface: surface_label(&surface.surface),
        floors: entry.frame.surfaces.iter().map(surface_label).collect(),
        origin: (surface.grid.origin.x, surface.grid.origin.y),
        cell_cm: u32::from(surface.grid.resolution_cm),
        width: u32::from(surface.grid.width),
        height: u32::from(surface.grid.height),
        rows,
        legend,
        units,
        you,
    })
}

/// The delta `viewer` sees: the units on their surface now, and `note`.
///
/// **16b cut S.2**: none while an arena holds the domain, for [`frame_for`]'s
/// reason — a delta is the frame's own update channel and must not outlive it.
#[must_use]
pub fn delta_for(
    w: &World,
    st: &AtlasState,
    viewer: ObjId,
    domain: DomainId,
    note: Option<String>,
) -> Option<AtlasMapDelta> {
    if mush_sim::deployed::excluded(w, mush_sim::deployed::Shown::Domain(domain)) {
        return None;
    }
    let entry = st.windows.open.get(&domain.0)?;
    let composite = entry.composite.as_ref()?;
    let si = surface_index(w, st, entry, viewer);
    let visible = visible_rooms(w, st, entry, viewer);
    let (units, you) = units_for(w, composite, si, domain, viewer, &visible);
    Some(AtlasMapDelta {
        schema: schema(),
        window: domain.0,
        revision: entry.revision,
        units,
        you,
        note,
    })
}

/// The frame that says the map is gone (a window closed): no cells.
#[must_use]
pub fn empty_frame(domain: DomainId) -> AtlasMapFrame {
    AtlasMapFrame {
        schema: schema(),
        window: domain.0,
        revision: 0,
        tick: 0,
        title: String::new(),
        surface: String::new(),
        floors: Vec::new(),
        origin: (0, 0),
        cell_cm: 0,
        width: 0,
        height: 0,
        rows: Vec::new(),
        legend: Vec::new(),
        units: Vec::new(),
        you: None,
    }
}

fn feed(ctx: &Ctx<'_>) -> Option<PanelFeedHandle> {
    ctx.state::<MapFeed>().and_then(|f| f.0.clone())
}

/// Everyone a window's frames go to: its participants, and a staff opener.
fn audience(w: &World, entry: &WindowEntry, domain: DomainId) -> Vec<ObjId> {
    let mut who: Vec<ObjId> = mush_sim::participants(w, domain)
        .into_iter()
        .map(|(o, _)| o)
        .collect();
    if let mush_sim::WindowOpener::Staff { by } = entry.opened_by
        && !who.contains(&by)
    {
        who.push(by);
    }
    who.retain(|o| w.get(*o).is_some());
    who
}

/// Send `viewer` their keyframe of `domain`.
pub fn send_keyframe(ctx: &mut Ctx<'_>, viewer: ObjId, domain: DomainId) -> bool {
    let frame = ctx
        .state::<AtlasState>()
        .and_then(|st| frame_for(ctx.w, st, viewer, domain));
    let Some(frame) = frame else {
        return false;
    };
    send_frame(ctx, viewer, &frame)
}

/// Send a frame already built for `viewer` (S8.3: `+map` prints its text from
/// the same frame it sends).
pub fn send_frame(ctx: &mut Ctx<'_>, viewer: ObjId, frame: &AtlasMapFrame) -> bool {
    let Some(feed) = feed(ctx) else {
        return false;
    };
    match serde_json::to_value(frame) {
        Ok(data) => {
            feed.keyframe_to(ctx, Audience::Actor(viewer), data);
            true
        }
        Err(error) => {
            tracing::error!(%error, "atlas: a map frame would not encode");
            false
        }
    }
}

/// A keyframe to everyone on `domain`'s window (a composite landed).
pub fn publish_keyframes(ctx: &mut Ctx<'_>, domain: DomainId) {
    let who = {
        let Some(st) = ctx.state::<AtlasState>() else {
            return;
        };
        let Some(entry) = st.windows.open.get(&domain.0) else {
            return;
        };
        audience(ctx.w, entry, domain)
    };
    for viewer in who {
        send_keyframe(ctx, viewer, domain);
    }
}

/// A delta to everyone on `domain`'s window: the units as each sees them, and
/// `note` for line mode.
pub fn publish_delta(ctx: &mut Ctx<'_>, domain: DomainId, note: &str) {
    let Some(feed) = feed(ctx) else {
        return;
    };
    let who = {
        let Some(st) = ctx.state::<AtlasState>() else {
            return;
        };
        let Some(entry) = st.windows.open.get(&domain.0) else {
            return;
        };
        audience(ctx.w, entry, domain)
    };
    for viewer in who {
        let delta = ctx.state::<AtlasState>().and_then(|st| {
            delta_for(
                ctx.w,
                st,
                viewer,
                domain,
                (!note.is_empty()).then(|| note.to_string()),
            )
        });
        if let Some(delta) = delta
            && let Ok(data) = serde_json::to_value(&delta)
        {
            feed.delta_to(ctx, Audience::Actor(viewer), data);
        }
    }
}

/// The empty keyframe to `who`: their map is gone.
pub fn publish_closed(ctx: &mut Ctx<'_>, domain: DomainId, who: &[ObjId]) {
    let Some(feed) = feed(ctx) else {
        return;
    };
    let Ok(data) = serde_json::to_value(empty_frame(domain)) else {
        return;
    };
    for &viewer in who {
        if ctx.w.get(viewer).is_some() {
            feed.keyframe_to(ctx, Audience::Actor(viewer), data.clone());
        }
    }
}
