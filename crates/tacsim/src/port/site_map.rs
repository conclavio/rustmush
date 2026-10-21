//! **The site map** (doc 20 cut SM.4b; §1 beats 4 and 7, §5, §10, D-20-13; the SM.4b brief's
//! lead rulings 6 and 7, its 3.9 and 3.10 and its reviewer resolutions R5 and R6): the
//! `site.map` pulled grid feed and `+tv/map` — one frame, three renders.
//!
//! The frame is a **grid** frame on the `atlas.map` shape, as `sky.chart`'s is
//! (`crate::port::chart_panel`), so the one grid decoder of each client reads it: line mode's
//! `+tv/map` prints it through its own printer (`site_map_lines`, which reads the rows through
//! `mush_ansi::window_rows`, the TUI's decoder — the legend line cannot go through
//! `mush_ansi::legend_line`, which takes one character a glyph), and the TUI's screen `q` and the
//! web's `FeedScreen` (SM.9's) draw the same `rows`. Two views:
//!
//! - **the drive view** — 28 × 28 cells of 10 m
//!   ([`DRIVE_VIEW_CELLS`](crate::port::site_map::DRIVE_VIEW_CELLS)) around the addressed rig,
//!   `cell_columns 2` (56 columns), the corner clamped to the site;
//! - **the site view** — 32 × 32 cells of 80 m
//!   ([`SITE_VIEW_CELLS`](crate::port::site_map::SITE_VIEW_CELLS)), the whole site,
//!   `cell_columns 1` in the TUI and the web (line mode prints every glyph twice); a cell shows the
//!   best of its four pockets.
//!
//! **The glyphs, top layer first**: the viewer's rigs `@` (and each one's slot digit east of it
//! when two or more stand on the ground); other rigs `o`; `H` the hull on the pad; the viewer's
//! OWN R2 readings in the family shown — `3`/`2`/`1` for rich/fair/lean, `+` where only another
//! family reads, `.` sounded and barren; the public pits `v` worked and `V` exhausted (derived
//! from the pocket cursor, never a cell edit, §3.4); a held block's edge `#` (a stub: claims are
//! SM.10's); each block's letter in its north-west corner cell; `O` a crater; the terrain —
//! regolith `,`, rock `^`, ice `=`, any other passable class `_`, an impassable cell a space.
//!
//! **The observer rule** (§5; lead ruling 6): every ore glyph and every reading is the viewer's
//! own chart row, rendered from the keyed geology under the row's rule at frame time; a second
//! principal sees terrain, units, the hull and the pits and nothing of anyone's chart. Other rigs
//! carry no name and no object number (`other-{k}`); a loud sounding of theirs inside the IC hour
//! marks their row of the vector block and nothing more (§5's transient mark; it reads the rig's
//! `last` only, so the sounder's next act clears it — the brief's decision P15).
//!
//! **Pulled, never pushed**: a `PanelSubscribe` on the feed answers one keyframe for the asker
//! (params `{ "view": "drive"|"site", "rig": 1..=4, "family": "<word>" }`); a client asks again
//! when it wants a newer one, interpolating each rig along its frozen leg in between.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{CanonicalId, CmdError, Ctx, HookReadContext, ObjId, QueryHandler, World};
use mush_inventory::record::EndpointRef;
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::port::chart_panel::run_length;
use crate::port::families::PortHandles;
use crate::port_tick::PoolReadError;
use crate::surface::address::{self, Octant};
use crate::surface::chart::{self, ChartKey, ChartRecord, SitePeek};
use crate::surface::cursor::PocketCursor;
use crate::surface::excursion::{
    LAUNCH_S, LegWaypoint, RigKey, RigRecord, cells_apart, charge_shown, site_cell,
};
use crate::surface::geology::{FieldGround, OreFamily, Word, by_family};
use crate::surface::reading::{FieldReading, Footing, READING_RULE_VERSION, read_r1, read_r2};

/// The feed (doc 20 §10). Registered by the tacsim layer whether or not it is enabled, as every
/// tacsim feed is: a disabled layer answers with the reason.
pub const SITE_MAP_FEED: &str = "site.map";
/// The frame's schema version.
pub const SCHEMA_VERSION: u32 = 1;
/// The drive view's side, 10 m cells (`cell_columns 2`: 56 printed columns).
pub const DRIVE_VIEW_CELLS: usize = 28;
/// The site view's side, 80 m cells (8 × 8 site cells).
pub const SITE_VIEW_CELLS: usize = 32;
/// A site-view cell's edge, centimetres.
pub const SITE_VIEW_CELL_CM: u32 = 8_000;
/// Site cells along a site-view cell's side.
const SITE_VIEW_SPAN: i64 = 8;
/// The most rows of the readings list.
pub const MAX_READING_ROWS: usize = 12;
/// The widest line `+tv/map` prints (the 120-column floor: the last two columns stay free).
pub const MAX_LINE_COLUMNS: usize = 118;
/// **Doc 20 §10's legend, verbatim**: every grid print carries it.
pub const SITE_LEGEND: &str = "@ you  H hull  o rig  , regolith  ^ rock  = ice  O crater  1–3 ore (t a cycle)  + other family  . barren  v/V pit  # claim";
/// The drive view's title: `{n}`, `{site}`, `{family}`.
pub const MAP_TITLE_DRIVE: &str = "Rig {n} on {site}: {family} shown";
/// The site view's title: `{site}`, `{family}`.
pub const MAP_TITLE_SITE: &str = "{site}: {family} shown";
/// No reading of the family in view: `{family}`.
pub const MAP_NO_READINGS: &str = "You have no readings of {family} in view.";
/// The pad's centre cell (the hull's `H`).
const PAD_CELL: (i64, i64) = (128, 128);

/// **Which view** (doc 20 §10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapView {
    /// The drive view around a rig: the slot given, else the lowest slot on the ground, else the
    /// ramp's cell for a rig in the bay.
    Drive {
        /// The slot named, if any.
        slot: Option<u8>,
    },
    /// The whole site.
    Site,
}

/// **The typed reads a frame needs** (doc 20 cut SM.4b), from a command (`Ctx`) or a feed query
/// (`HookReadContext`) — both through tacsim's own module-scoped typed-state capability. **Doc 20
/// cut SM.7** (the brief's 3.3, reviewer F20) adds the two reads the mining plan needs —
/// industry's deposits and the claims over one, read only (SM.6's D10 tripwire holds tacsim to
/// reads) — and an impl over the engine's own snapshot for the start's and the finish's offers
/// (`EngineReads`): one trait, never a second reader.
pub(crate) trait SurfaceReads {
    /// A rig's `tacsim.excursion` record.
    fn rig(&self, tv: ObjId) -> Result<Option<RigRecord>, String>;
    /// A principal's chart row for a site.
    fn chart(&self, principal: ObjId, site: &CanonicalId) -> Result<Option<ChartRecord>, String>;
    /// A pocket cursor (`tacsim.pockets`) — the public pits' one source.
    fn cursor(&self, deposit: mush_industry::OreDepositId) -> Result<Option<PocketCursor>, String>;
    /// One pool's balance.
    fn pool(&self, endpoint: EndpointRef, good: &CanonicalId) -> Result<u64, PoolReadError>;
    /// **An industry deposit** (doc 20 cut SM.7), `None` when it was never opened.
    fn deposit(
        &self,
        id: mush_industry::OreDepositId,
    ) -> Result<Option<mush_industry::OreDeposit>, String>;
    /// **Every claim filed over a deposit** (doc 20 cut SM.7): the subject's index, then each
    /// record (industry's `claims_over` shape, no transaction).
    fn claims(
        &self,
        deposit: mush_industry::OreDepositId,
    ) -> Result<Vec<mush_industry::ClaimRecord>, String>;
}

/// The handles, or why not.
fn handles_of(w: &World) -> Result<&PortHandles, String> {
    w.module_state::<PortHandles>()
        .ok_or_else(|| "the port handles are not composed".to_owned())
}

/// Industry's families, or why not (doc 20 cut SM.7: read only).
fn industry_of(w: &World) -> Result<mush_industry::IndustryFamilies, String> {
    w.module_state::<mush_industry::IndustryHandles>()
        .map(|handles| handles.0.clone())
        .ok_or_else(|| "the industry families are not composed".to_owned())
}

/// The claims over `deposit` through a family reader (`claims_over`'s shape: the subject's index,
/// then each record; an id the index names whose record is gone is skipped).
fn claims_through<E: std::fmt::Display>(
    deposit: mush_industry::OreDepositId,
    index: &dyn Fn(&SmolStr) -> Result<Option<mush_industry::ClaimIndex>, E>,
    claim: &dyn Fn(&mush_industry::ClaimId) -> Result<Option<mush_industry::ClaimRecord>, E>,
) -> Result<Vec<mush_industry::ClaimRecord>, String> {
    let token =
        mush_industry::claim_subject_token(mush_industry::ClaimSubject::Deposit { deposit });
    let Some(found) = index(&token).map_err(|error| error.to_string())? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(found.claims().len());
    for id in found.claims() {
        if let Some(record) = claim(id).map_err(|error| error.to_string())? {
            out.push(record);
        }
    }
    Ok(out)
}

/// **The engine's own snapshot, as [`SurfaceReads`]** (doc 20 cut SM.7; the brief's 3.3): the
/// start's and the finish's offers hold the engine, not a `Ctx`, and read through it — the same
/// families, the same pool rule ([`crate::port_tick::read_pool`]'s: absent is 0, a refusal never
/// 0), never a second reader.
pub(crate) struct EngineReads<'e>(pub &'e mush_core::Engine);

impl SurfaceReads for EngineReads<'_> {
    fn rig(&self, tv: ObjId) -> Result<Option<RigRecord>, String> {
        let handles = handles_of(&self.0.world)?;
        self.0
            .action_state()
            .get(&handles.families.excursions, &RigKey(tv))
            .map(|found| found.cloned().filter(|record| record.tv == tv))
            .map_err(|error| error.to_string())
    }

    fn chart(&self, principal: ObjId, site: &CanonicalId) -> Result<Option<ChartRecord>, String> {
        let handles = handles_of(&self.0.world)?;
        let key = ChartKey {
            principal,
            site: site.clone(),
        };
        self.0
            .action_state()
            .get(&handles.families.charts, &key)
            .map(|found| found.cloned().filter(|row| row.key() == key))
            .map_err(|error| error.to_string())
    }

    fn cursor(&self, deposit: mush_industry::OreDepositId) -> Result<Option<PocketCursor>, String> {
        let handles = handles_of(&self.0.world)?;
        self.0
            .action_state()
            .get(&handles.families.pockets, &deposit)
            .map(|found| found.cloned().filter(|row| row.deposit == deposit))
            .map_err(|error| error.to_string())
    }

    fn pool(&self, endpoint: EndpointRef, good: &CanonicalId) -> Result<u64, PoolReadError> {
        let Some(inventory) = self
            .0
            .world
            .module_state::<PortHandles>()
            .and_then(|handles| handles.inventory.as_ref())
        else {
            return Err(PoolReadError::Unwired);
        };
        let key = mush_inventory::pool_key(endpoint, good)
            .map_err(|error| PoolReadError::Key(error.to_string()))?;
        match self.0.action_state().get(&inventory.pools, &key) {
            Ok(None) => Ok(0),
            Ok(Some(scalar)) => u64::try_from(scalar.current()).map_err(|_| PoolReadError::Corrupt),
            Err(error) => Err(PoolReadError::Unreadable(error.to_string())),
        }
    }

    fn deposit(
        &self,
        id: mush_industry::OreDepositId,
    ) -> Result<Option<mush_industry::OreDeposit>, String> {
        let families = industry_of(&self.0.world)?;
        self.0
            .action_state()
            .get(&families.deposits, &id)
            .map(|found| found.cloned())
            .map_err(|error| error.to_string())
    }

    fn claims(
        &self,
        deposit: mush_industry::OreDepositId,
    ) -> Result<Vec<mush_industry::ClaimRecord>, String> {
        let families = industry_of(&self.0.world)?;
        let state = self.0.action_state();
        claims_through(
            deposit,
            &|token| state.get(&families.claim_index, token).map(|f| f.cloned()),
            &|id| state.get(&families.claims, id).map(|f| f.cloned()),
        )
    }
}

impl SurfaceReads for Ctx<'_> {
    fn rig(&self, tv: ObjId) -> Result<Option<RigRecord>, String> {
        let handles = handles_of(self.w)?;
        self.action_state(&handles.families.excursions, &RigKey(tv))
            .map(|found| found.cloned())
            .map_err(|error| error.to_string())
    }

    fn chart(&self, principal: ObjId, site: &CanonicalId) -> Result<Option<ChartRecord>, String> {
        let handles = handles_of(self.w)?;
        let key = ChartKey {
            principal,
            site: site.clone(),
        };
        self.action_state(&handles.families.charts, &key)
            .map(|found| found.cloned().filter(|row| row.key() == key))
            .map_err(|error| error.to_string())
    }

    fn cursor(&self, deposit: mush_industry::OreDepositId) -> Result<Option<PocketCursor>, String> {
        let handles = handles_of(self.w)?;
        self.action_state(&handles.families.pockets, &deposit)
            .map(|found| found.cloned().filter(|row| row.deposit == deposit))
            .map_err(|error| error.to_string())
    }

    fn pool(&self, endpoint: EndpointRef, good: &CanonicalId) -> Result<u64, PoolReadError> {
        crate::port_tick::read_pool(self, endpoint, good)
    }

    fn deposit(
        &self,
        id: mush_industry::OreDepositId,
    ) -> Result<Option<mush_industry::OreDeposit>, String> {
        let families = industry_of(self.w)?;
        self.action_state(&families.deposits, &id)
            .map(|found| found.cloned())
            .map_err(|error| error.to_string())
    }

    fn claims(
        &self,
        deposit: mush_industry::OreDepositId,
    ) -> Result<Vec<mush_industry::ClaimRecord>, String> {
        let families = industry_of(self.w)?;
        claims_through(
            deposit,
            &|token| {
                self.action_state(&families.claim_index, token)
                    .map(|f| f.cloned())
            },
            &|id| self.action_state(&families.claims, id).map(|f| f.cloned()),
        )
    }
}

impl SurfaceReads for HookReadContext<'_> {
    fn rig(&self, tv: ObjId) -> Result<Option<RigRecord>, String> {
        let handles = handles_of(self.world())?;
        self.action_state(&handles.families.excursions, &RigKey(tv))
            .map(|found| found.cloned())
            .map_err(|error| error.to_string())
    }

    fn chart(&self, principal: ObjId, site: &CanonicalId) -> Result<Option<ChartRecord>, String> {
        let handles = handles_of(self.world())?;
        let key = ChartKey {
            principal,
            site: site.clone(),
        };
        self.action_state(&handles.families.charts, &key)
            .map(|found| found.cloned().filter(|row| row.key() == key))
            .map_err(|error| error.to_string())
    }

    fn cursor(&self, deposit: mush_industry::OreDepositId) -> Result<Option<PocketCursor>, String> {
        let handles = handles_of(self.world())?;
        self.action_state(&handles.families.pockets, &deposit)
            .map(|found| found.cloned().filter(|row| row.deposit == deposit))
            .map_err(|error| error.to_string())
    }

    fn pool(&self, endpoint: EndpointRef, good: &CanonicalId) -> Result<u64, PoolReadError> {
        crate::port_tick::read_pool_in(self, endpoint, good)
    }

    fn deposit(
        &self,
        id: mush_industry::OreDepositId,
    ) -> Result<Option<mush_industry::OreDeposit>, String> {
        let families = industry_of(self.world())?;
        self.action_state(&families.deposits, &id)
            .map(|found| found.cloned())
            .map_err(|error| error.to_string())
    }

    fn claims(
        &self,
        deposit: mush_industry::OreDepositId,
    ) -> Result<Vec<mush_industry::ClaimRecord>, String> {
        let families = industry_of(self.world())?;
        claims_through(
            deposit,
            &|token| {
                self.action_state(&families.claim_index, token)
                    .map(|f| f.cloned())
            },
            &|id| self.action_state(&families.claims, id).map(|f| f.cloned()),
        )
    }
}

/// **Why a frame cannot be drawn** (doc 20 cut SM.4b): the line the command prints (and the
/// feed's reason frame shows as its title), and — for a cold site — the site and ramp cell the
/// command asks to be warmed (RAM only; the query never wants).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MapRefused {
    /// The line.
    pub line: String,
    /// A cold site's entry and ramp cell.
    pub want: Option<(CanonicalId, (i64, i64))>,
}

impl MapRefused {
    fn line(line: impl Into<String>) -> MapRefused {
        MapRefused {
            line: line.into(),
            want: None,
        }
    }
}

/// Register the feed and its query (doc 20 cut SM.4b): the handle is dropped, as `sky.chart`'s
/// is — nothing pushes a site map.
///
/// # Errors
///
/// The registrar's, for a feed or query name already taken.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    drop(reg.panel_feed(SITE_MAP_FEED)?);
    reg.query_handler(SITE_MAP_FEED, Arc::new(SiteMapQuery))?;
    Ok(())
}

/// **`site.map`'s handler**: the asker's frame read through the typed snapshot, or a frame with
/// no grid and the reason as its title (the line `+tv/map` would print).
struct SiteMapQuery;

/// The reason frame.
fn reason_frame(why: &str) -> Value {
    json!({
        "schema": { "id": SITE_MAP_FEED, "shape": "grid", "version": SCHEMA_VERSION },
        "window": 0,
        "title": why,
        "width": 0,
        "height": 0,
        "rows": [],
        "legend": [],
        "units": [],
        "you": null,
    })
}

/// The feed's params: the view, the rig and the family word.
fn params_view(params: &Value) -> Result<(MapView, Option<OreFamily>), String> {
    let slot = params
        .get("rig")
        .and_then(Value::as_u64)
        .and_then(|n| u8::try_from(n).ok())
        .filter(|n| (1..=4).contains(n));
    let view = match params.get("view").and_then(Value::as_str) {
        Some("site") => MapView::Site,
        Some("drive") | None => MapView::Drive { slot },
        Some(_) => MapView::Drive { slot },
    };
    let family = match params.get("family").and_then(Value::as_str) {
        None => None,
        Some(word) => Some(
            crate::surface::geology::family_of_word(word)
                .ok_or_else(|| crate::port::tv::MAP_FAMILY_UNKNOWN.replace("{word}", word))?,
        ),
    };
    Ok((view, family))
}

impl QueryHandler for SiteMapQuery {
    fn query(&self, _w: &World, _actor: ObjId, _params: &Value) -> Result<OutputEvent, RichText> {
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(SITE_MAP_FEED),
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
        let data = match params_view(params) {
            Ok((view, family)) => site_map_frame(context.world(), context, actor, view, family)
                .unwrap_or_else(|refused| reason_frame(&refused.line)),
            Err(line) => reason_frame(&line),
        };
        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static(SITE_MAP_FEED),
            kind: PanelFrameKind::Keyframe,
            data,
        })
    }
}

// ---- where things stand ------------------------------------------------------------------------

/// **A position along a frozen leg at `t` seconds from its start** (integer linear
/// interpolation, §8: "positions between are pure interpolation"): the first waypoint before the
/// leg, the last after it.
#[must_use]
pub fn leg_position(waypoints: &[LegWaypoint], t: i64) -> Option<(i64, i64)> {
    let first = waypoints.first()?;
    if t <= i64::from(first.at_s) {
        return Some((first.x_cm, first.y_cm));
    }
    for pair in waypoints.windows(2) {
        let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let (ta, tb) = (i64::from(a.at_s), i64::from(b.at_s));
        if t <= tb {
            if tb <= ta {
                return Some((b.x_cm, b.y_cm));
            }
            let lerp = |p: i64, q: i64| -> i64 {
                let span = i128::from(tb - ta);
                let along = i128::from(t - ta);
                let delta = i128::from(q) - i128::from(p);
                i64::try_from(i128::from(p) + delta * along / span).unwrap_or(q)
            };
            return Some((lerp(a.x_cm, b.x_cm), lerp(a.y_cm, b.y_cm)));
        }
    }
    waypoints.last().map(|last| (last.x_cm, last.y_cm))
}

/// Where a rig stands for the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// On the site's ground at this site cell (interpolated along its leg while one moves it).
    Ground((i64, i64)),
    /// In its hull's bay.
    Bay,
    /// Anywhere else.
    Away,
}

/// **A rig's place at `now` on the site `entry`** (whose passive domain is `domain`): along its
/// act's frozen leg once the leg has begun (a drive from the bay is in the bay until its launch
/// half ends) — **only for an act on this very site** (the review's correctness/F1: a rig
/// driving on another site's ground is never drawn here, its leg being in that site's cells) —
/// else its placement, which must be on this site's domain or in a bay.
fn place_of(
    w: &World,
    record: Option<&RigRecord>,
    tv: ObjId,
    (entry, domain): (&CanonicalId, mush_sim::DomainId),
    hull: Option<ObjId>,
    now: i64,
) -> Place {
    if let Some(act) = record.and_then(|record| record.act.as_ref())
        && &act.site == entry
        && let Some((waypoints, _)) = act.params.leg()
    {
        let launch = if act.params.launches() {
            i64::from(LAUNCH_S)
        } else {
            0
        };
        let t = now.saturating_sub(act.started_ic.saturating_add(launch));
        if t >= 0
            && let Some((x, y)) = leg_position(waypoints, t)
        {
            return Place::Ground(site_cell(x, y));
        }
    }
    match crate::surface::act::where_is(w, tv) {
        crate::surface::act::Where::Ground {
            domain: on,
            x_cm,
            y_cm,
            ..
        } if on == domain => Place::Ground(site_cell(x_cm, y_cm)),
        crate::surface::act::Where::Bay { hull: aboard } if hull.is_none_or(|h| h == aboard) => {
            Place::Bay
        }
        crate::surface::act::Where::Ground { .. }
        | crate::surface::act::Where::Bay { .. }
        | crate::surface::act::Where::Elsewhere => Place::Away,
    }
}

// ---- the glyphs ----------------------------------------------------------------------------------

/// **A terrain glyph** (doc 20 §10's legend): regolith `,`, rock `^`, ice `=`, any other class the
/// tracked locomotion passes `_`, an impassable one a space.
fn terrain_glyph(class: mush_atlas::TerrainClass) -> char {
    match Footing::of(class) {
        Some(Footing::Regolith) => ',',
        Some(Footing::Rock) => '^',
        Some(Footing::Ice) => '=',
        None if mush_atlas::physics::LocomotionTable::shipped().passable(
            class.label(),
            crate::surface::drive::DRIVE_LOCOMOTION,
            &[],
        ) =>
        {
            '_'
        }
        None => ' ',
    }
}

/// **A reading's glyph** in the family shown (lead ruling 6): `3`/`2`/`1` for rich/fair/lean,
/// `+` where only another family reads, `.` sounded and barren. The digit is the class ordinal —
/// the drill row's `yield_t` (a unit test holds the legend to it).
fn reading_glyph(classes: &[Word; 4], family: OreFamily) -> char {
    match by_family(*classes, family) {
        Word::Rich => '3',
        Word::Fair => '2',
        Word::Lean => '1',
        Word::Barren if classes.iter().any(|class| *class != Word::Barren) => '+',
        Word::Barren => '.',
    }
}

/// A reading glyph's rank (the site view shows the best of its four pockets).
fn rank(glyph: char) -> u8 {
    match glyph {
        '3' => 5,
        '2' => 4,
        '1' => 3,
        '+' => 2,
        '.' => 1,
        _ => 0,
    }
}

/// **The public pits** (doc 20 §3.4; the lead's note 0b): for every (block, family) of the site
/// the deposit's pocket cursor (`tacsim.pockets`, keyed by the deposit id the site clerk derives
/// from `crate::surface::opening::receipt`) — a row with `drawn_t ≥ 1` is a worked pocket `v`, and
/// `V` once its `drawn_t` has reached the pocket's tonnes of that family in the keyed site
/// geology (computed here, server-side); a pocket with several rows is `V` only when every one is
/// exhausted. **The reads**: the cursor rows (public by doc 20 §3.4: "the public pits show") and
/// the site table's tonnes for the exhausted test; never the deposit's pool or its reserve
/// (industry's, staff-only), never any chart. A pit reveals only that the pocket was worked; it
/// never shows a class.
fn pit_layer(
    reads: &dyn SurfaceReads,
    entry: &CanonicalId,
    ground: &crate::surface::SiteGround,
) -> BTreeMap<(i64, i64), char> {
    let mut pits: BTreeMap<(i64, i64), char> = BTreeMap::new();
    for letter in crate::port::def::BLOCK_LETTERS {
        for family in OreFamily::ALL {
            let receipt = crate::surface::opening::receipt(entry, letter, family);
            let Ok(deposit) = mush_industry::derive_system_deposit_id(&receipt) else {
                continue;
            };
            let Ok(Some(cursor)) = reads.cursor(deposit) else {
                continue;
            };
            for row in cursor.rows.iter().filter(|row| row.drawn_t >= 1) {
                let Some(index) = usize::from(row.pocket).checked_sub(1) else {
                    continue;
                };
                let Some(pocket) = address::pocket_of_block_index(letter, index) else {
                    continue;
                };
                let exhausted = row_exhausted(ground, pocket, family, row.drawn_t);
                let folded = fold_pit(pits.get(&pocket).copied(), exhausted);
                if let Some(glyph) = folded {
                    pits.insert(pocket, glyph);
                }
            }
        }
    }
    pits
}

/// Whether a drawn cursor row has reached the pocket's tonnes of `family` in the keyed site table.
fn row_exhausted(
    ground: &crate::surface::SiteGround,
    pocket: (i64, i64),
    family: OreFamily,
    drawn_t: u32,
) -> bool {
    let tonnes = ground
        .geology
        .pocket(pocket.0, pocket.1)
        .map_or(0, |cell| by_family(cell.tonnes, family));
    drawn_t >= u32::from(tonnes)
}

/// **The pit rule's fold over one pocket's drawn rows** (doc 20 §3.4; the SM.7 brief's 3.3 item
/// 8b): the first drawn row gives `v`, or `V` when it is exhausted; any row not exhausted makes the
/// pocket `v` — so a pocket is `V` only when every drawn row is. The one rule the map's layer and
/// [`pit_at`] fold through, so the map and the mining preview never disagree.
fn fold_pit(seen: Option<char>, exhausted: bool) -> Option<char> {
    match seen {
        None => Some(if exhausted { 'V' } else { 'v' }),
        Some(_) if !exhausted => Some('v'),
        Some(glyph) => Some(glyph),
    }
}

/// **One pocket's public pit** (doc 20 §6 "or a public pit"; cut SM.7, the brief's 3.3 item 8b,
/// R16): `v` worked, `V` exhausted, `None` never drawn — the four families' cursors of the pocket's
/// block, their row for this pocket, folded by [`pit_layer`]'s own rule ([`fold_pit`]). Public, the
/// same for every asker; a pocket barren in every family has no cursor row and so never a pit.
pub(crate) fn pit_at(
    reads: &dyn SurfaceReads,
    entry: &CanonicalId,
    ground: &crate::surface::SiteGround,
    pocket: (i64, i64),
) -> Option<char> {
    let letter = address::block_letter(pocket.0, pocket.1)?;
    let number = address::pocket_number(pocket.0, pocket.1)?;
    let mut pit: Option<char> = None;
    for family in OreFamily::ALL {
        let receipt = crate::surface::opening::receipt(entry, letter, family);
        let Ok(deposit) = mush_industry::derive_system_deposit_id(&receipt) else {
            continue;
        };
        let Ok(Some(cursor)) = reads.cursor(deposit) else {
            continue;
        };
        if let Some(row) = cursor.row(number).filter(|row| row.drawn_t >= 1) {
            pit = fold_pit(pit, row_exhausted(ground, pocket, family, row.drawn_t));
        }
    }
    pit
}

/// **A held block's edge `#`** — a stub: claims are SM.10's (no claim is filed before it).
fn claim_edges() -> impl Iterator<Item = ((i64, i64), char)> {
    std::iter::empty()
}

/// One of the viewer's rigs on the map: its slot, its frame cell and the frame cell east of it
/// (where its slot digit stands; `None` past the frame's edge).
type MineOnMap = (u8, (usize, usize), Option<(usize, usize)>);

/// **The layers above the terrain**, in frame cells, bottom first (lead ruling 6's order, read
/// upward): the blocks' letters, a held block's edge, the public pits, the viewer's own readings
/// (one glyph a cell, already the best of the cell's pockets), the hull, other rigs and the
/// viewer's rigs — each with the cell east of it, where its slot digit stands when two or more of
/// the viewer's rigs are on the ground.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Overlays {
    letters: Vec<((usize, usize), char)>,
    claims: Vec<((usize, usize), char)>,
    pits: Vec<((usize, usize), char)>,
    readings: BTreeMap<(usize, usize), char>,
    hull: Option<(usize, usize)>,
    others: Vec<(usize, usize)>,
    mine: Vec<MineOnMap>,
}

/// **The best reading glyph of each frame cell** (the site view shows the best of a cell's four
/// pockets, [`rank`]).
fn best_readings(
    readings: impl Iterator<Item = ((usize, usize), char)>,
) -> BTreeMap<(usize, usize), char> {
    let mut best: BTreeMap<(usize, usize), char> = BTreeMap::new();
    for (at, glyph) in readings {
        let slot = best.entry(at).or_insert(glyph);
        if rank(glyph) > rank(*slot) {
            *slot = glyph;
        }
    }
    best
}

/// **Paint the layers over the terrain** (pure): each later layer over the earlier — the letters,
/// the edges, the pits, the readings, `H`, every other rig's `o`, the viewer's slot digits (two or
/// more on the ground) and last each of the viewer's rigs' `@`.
fn compose(grid: &mut [Vec<char>], layers: &Overlays) {
    let mut put = |(fx, fy): (usize, usize), glyph: char| {
        if let Some(cell) = grid.get_mut(fy).and_then(|row| row.get_mut(fx)) {
            *cell = glyph;
        }
    };
    for (at, glyph) in layers
        .letters
        .iter()
        .chain(layers.claims.iter())
        .chain(layers.pits.iter())
    {
        put(*at, *glyph);
    }
    for (at, glyph) in &layers.readings {
        put(*at, *glyph);
    }
    if let Some(at) = layers.hull {
        put(at, 'H');
    }
    for at in &layers.others {
        put(*at, 'o');
    }
    if layers.mine.len() >= 2 {
        for (slot, _, east) in &layers.mine {
            if let (Some(at), Some(digit)) = (east, char::from_digit(u32::from(*slot), 10)) {
                put(*at, digit);
            }
        }
    }
    for (_, at, _) in &layers.mine {
        put(*at, '@');
    }
}

/// **The units overlay** (R5, binding): a unit is listed only where the composed `grid` already
/// holds its glyph at its cell — so an `o` under an `@` or under a slot digit is not listed —
/// the others first (`other-{k}`, `k` from 1 in the order listed, never an object number), then
/// the viewer's rigs in slot order (`rig-{n}`, `you` for the addressed one). Every decoder's
/// overlay then reproduces the rows exactly.
fn list_units(grid: &[Vec<char>], layers: &Overlays, addressed: Option<u8>) -> Vec<Value> {
    let glyph_at = |(fx, fy): (usize, usize)| grid.get(fy).and_then(|row| row.get(fx)).copied();
    let mut units: Vec<Value> = Vec::new();
    let mut k = 0_usize;
    for at in &layers.others {
        if glyph_at(*at) == Some('o') {
            k += 1;
            units.push(json!({
                "id": format!("other-{k}"), "name": "rig", "cell": [at.0, at.1],
                "glyph": "o", "you": false,
            }));
        }
    }
    let mut mine = layers.mine.clone();
    mine.sort_by_key(|(slot, _, _)| *slot);
    for (slot, at, _) in &mine {
        if glyph_at(*at) == Some('@') {
            units.push(json!({
                "id": format!("rig-{slot}"),
                "name": format!("Rig {slot}"),
                "cell": [at.0, at.1],
                "glyph": "@",
                "you": Some(*slot) == addressed,
            }));
        }
    }
    units
}

/// One pocket the viewer has sounded: its pocket, its address cell and its R2 classes.
struct Sounded {
    pocket: (i64, i64),
    classes: [Word; 4],
}

/// One field the viewer has scanned: its field, its readings and its footing.
struct Scanned {
    field: (i64, i64),
    readings: [FieldReading; 4],
    footing: Footing,
}

/// **The viewer's own readings** (§5): every pocket their row has sounded and every field it has
/// scanned, rendered from the keyed geology under the row's rule — a row under another rule
/// renders nothing. Each field's ground is read once. **The same gate and the same reads as the
/// render door** (`chart::pocket_reading`, `chart::field_reading`; the review's correctness/F6):
/// a pocket only where `chart::row_reads_pocket` holds, a field only at the footing
/// `chart::row_reads_field` gives, each through `read_r2`/`read_r1` over `chart::field_ground_at`
/// — so the map never shows a reading the door would not.
fn viewer_readings(
    site: &chart::SiteContext,
    row: Option<&ChartRecord>,
) -> (Vec<Sounded>, Vec<Scanned>) {
    let Some(row) = row.filter(|row| row.rule == READING_RULE_VERSION) else {
        return (Vec::new(), Vec::new());
    };
    let mut grounds: BTreeMap<(i64, i64), Option<FieldGround>> = BTreeMap::new();
    let mut ground_of = |field: (i64, i64)| -> Option<FieldGround> {
        grounds
            .entry(field)
            .or_insert_with(|| chart::field_ground_at(site, field).ok())
            .clone()
    };
    let mut sounded = Vec::new();
    for (letter, block) in &row.blocks {
        let Some(letter) = letter.chars().next() else {
            continue;
        };
        let Some(bytes) = block.bytes() else {
            continue;
        };
        for index in 0..bytes.len() {
            let Some(pocket) = address::pocket_of_block_index(letter, index) else {
                continue;
            };
            if !chart::row_reads_pocket(Some(row), pocket) {
                continue;
            }
            let Some(field) = address::field_of_pocket(pocket) else {
                continue;
            };
            let Some(cell) = ground_of(field).and_then(|ground| {
                ground.pocket(
                    pocket
                        .0
                        .rem_euclid(crate::surface::geology::POCKETS_PER_FIELD_SIDE),
                    pocket
                        .1
                        .rem_euclid(crate::surface::geology::POCKETS_PER_FIELD_SIDE),
                )
            }) else {
                continue;
            };
            sounded.push(Sounded {
                pocket,
                classes: read_r2(&cell),
            });
        }
    }
    let mut scanned = Vec::new();
    for index in row.fields.keys() {
        let Some(field) = address::field_of_index(*index) else {
            continue;
        };
        let Some(footing) = chart::row_reads_field(Some(row), field) else {
            continue;
        };
        let Some(ground) = ground_of(field) else {
            continue;
        };
        scanned.push(Scanned {
            field,
            readings: read_r1(&site.mac, &site.body, &ground, footing),
            footing,
        });
    }
    (sounded, scanned)
}

/// A pocket's address cell (`(4px + 2, 4py + 2)`, the brief's decision P9).
fn pocket_cell((px, py): (i64, i64)) -> (i64, i64) {
    (
        px.saturating_mul(address::CELLS_PER_POCKET)
            .saturating_add(2),
        py.saturating_mul(address::CELLS_PER_POCKET)
            .saturating_add(2),
    )
}

/// A field's middle cell.
fn field_cell((fx, fy): (i64, i64)) -> (i64, i64) {
    (
        fx.saturating_mul(address::CELLS_PER_FIELD)
            .saturating_add(16),
        fy.saturating_mul(address::CELLS_PER_FIELD)
            .saturating_add(16),
    )
}

/// A field's reading restricted to one family, when it reads ore of it (never from ice).
fn field_family_text(scanned: &Scanned, family: OreFamily) -> Option<String> {
    if scanned.footing == Footing::Ice {
        return None;
    }
    let reading = scanned.readings.get(usize::from(family.code()))?;
    if *reading == FieldReading::Exact(Word::Barren) {
        return None;
    }
    let name = address::field_name(scanned.field.0, scanned.field.1)?;
    Some(match reading.pockets() {
        Some((lo, hi)) => format!(
            "{name} {} {} ({lo}–{hi} pockets)",
            reading.words(),
            family.label()
        ),
        None => format!("{name} {} {}", reading.words(), family.label()),
    })
}

/// `{m} m {dir}`, or `here`.
fn bearing_words(from: (i64, i64), to: (i64, i64)) -> (u64, String) {
    let m = cells_apart(from, to).saturating_mul(10);
    let dir = Octant::of(to.0.saturating_sub(from.0), to.1.saturating_sub(from.1))
        .map_or_else(String::new, |octant| octant.abbrev().to_owned());
    (m, dir)
}

/// Wrap `line` at [`MAX_LINE_COLUMNS`] on spaces, continuation lines indented two spaces.
fn wrap(line: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in line.split(' ') {
        let len = current.chars().count();
        let add = word.chars().count();
        if len > 0 && len + 1 + add > MAX_LINE_COLUMNS {
            out.push(std::mem::take(&mut current));
            current.push_str("  ");
            current.push_str(word);
        } else {
            if len > 0 {
                current.push(' ');
            }
            current.push_str(word);
        }
    }
    if !current.is_empty() || out.is_empty() {
        out.push(current);
    }
    out
}

/// One rig of the viewer's, as the frame reads it.
struct ViewerRig {
    rig: crate::surface::act::RigAt,
    record: Option<RigRecord>,
    place: Place,
}

/// **The `site.map` frame for `actor`** (doc 20 cut SM.4b; the module doc): the site is the
/// actor's helm hull's site berth; the grid, the legend, the units overlay (R5: window-relative
/// cells, a unit listed only where `rows` holds its glyph, others first) and the `site` block.
///
/// # Errors
///
/// [`MapRefused`]: not settled on a site (`MAP_NOT_SITE`), the addressed rig neither on the
/// ground nor in the bay (`MAP_NO_RIG`; with no slot named and none of the actor's rigs there,
/// `MAP_NO_RIGS`), the instruments dark (`INSTRUMENTS_DARK`), or the site's
/// ground still being mapped (`GROUND_COLD`, with the site to warm).
#[allow(clippy::too_many_lines)] // one frame: the window, the layers, the block
pub(crate) fn site_map_frame(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    view: MapView,
    family: Option<OreFamily>,
) -> Result<Value, MapRefused> {
    use crate::port::tv;
    let not_site = || MapRefused::line(tv::MAP_NOT_SITE);
    let helm = crate::port::coop::helm_of(w, actor).ok_or_else(not_site)?;
    let hull_view = crate::port::ship::hull_view(w, helm.hull).ok_or_else(not_site)?;
    let berth = crate::surface::act::site_berth(w, &hull_view).map_err(|_| not_site())?;
    let entry = berth.entry.clone();
    let ramp_cell = site_cell(berth.ramp.x_cm, berth.ramp.y_cm);
    let site =
        chart::site_context(w, &entry).map_err(|_| MapRefused::line(tv::INSTRUMENTS_DARK))?;
    let mut peeks = crate::surface::Peeks::default();
    let ground = match chart::peek_site(w, &entry, &mut peeks) {
        SitePeek::Warm(ground) => ground,
        SitePeek::Cold => {
            return Err(MapRefused {
                line: tv::GROUND_COLD.replace("{site}", &berth.name),
                want: Some((entry, ramp_cell)),
            });
        }
        SitePeek::Dark => return Err(MapRefused::line(tv::INSTRUMENTS_DARK)),
    };
    let surface =
        chart::ground_surface(&ground).ok_or_else(|| MapRefused::line(tv::INSTRUMENTS_DARK))?;
    let now = w.world_ic_now().at;
    // The viewer's rigs, by slot.
    let rigs: Vec<ViewerRig> = crate::surface::act::operator_rigs(w, actor)
        .into_iter()
        .map(|rig| {
            let record = reads.rig(rig.tv).ok().flatten();
            let place = place_of(
                w,
                record.as_ref(),
                rig.tv,
                (&entry, berth.domain),
                Some(rig.hull),
                now,
            );
            ViewerRig { rig, record, place }
        })
        .collect();
    let on_ground: Vec<&ViewerRig> = rigs
        .iter()
        .filter(|rig| matches!(rig.place, Place::Ground(_)))
        .collect();
    // The addressed rig and the window.
    let addressed: Option<&ViewerRig> =
        match view {
            MapView::Drive { slot: Some(n) } => {
                let found = rigs
                    .iter()
                    .find(|rig| rig.rig.slot == n && rig.place != Place::Away);
                Some(found.ok_or_else(|| {
                    MapRefused::line(
                        tv::MAP_NO_RIG
                            .replace("{n}", &n.to_string())
                            .replace("{site}", &berth.name),
                    )
                })?)
            }
            MapView::Drive { slot: None } => {
                let found = on_ground
                    .first()
                    .copied()
                    .or_else(|| rigs.iter().find(|rig| rig.place == Place::Bay));
                Some(found.ok_or_else(|| {
                    MapRefused::line(tv::MAP_NO_RIGS.replace("{site}", &berth.name))
                })?)
            }
            MapView::Site => on_ground.first().copied(),
        };
    let addressed_cell = addressed.and_then(|rig| match rig.place {
        Place::Ground(cell) => Some(cell),
        Place::Bay | Place::Away => None,
    });
    let site_side = i64::from(crate::surface::ground::SITE_CELLS);
    let (span, side, cell_cm, columns) = match view {
        MapView::Drive { .. } => (
            1_i64,
            DRIVE_VIEW_CELLS,
            u32::from(crate::surface::ground::SITE_CELL_CM),
            2,
        ),
        MapView::Site => (SITE_VIEW_SPAN, SITE_VIEW_CELLS, SITE_VIEW_CELL_CM, 1),
    };
    let side_i = i64::try_from(side).unwrap_or(0);
    let (x0, y0) = match view {
        MapView::Drive { .. } => {
            let centre = addressed_cell.unwrap_or(ramp_cell);
            let clamp = |c: i64| (c - side_i / 2).clamp(0, site_side - side_i);
            (clamp(centre.0), clamp(centre.1))
        }
        MapView::Site => (0, 0),
    };
    // Frame cell of a site cell, if in the window.
    let frame_cell = |cell: (i64, i64)| -> Option<(usize, usize)> {
        let fx = (cell.0 - x0).div_euclid(span);
        let fy = (cell.1 - y0).div_euclid(span);
        if cell.0 < x0 || cell.1 < y0 || fx >= side_i || fy >= side_i {
            return None;
        }
        Some((usize::try_from(fx).ok()?, usize::try_from(fy).ok()?))
    };
    // The viewer's own readings, the family shown, and its counts.
    let row = reads.chart(actor, &entry).ok().flatten();
    let (sounded, scanned) = viewer_readings(&site, row.as_ref());
    let in_view_pocket = |s: &&Sounded| frame_cell(pocket_cell(s.pocket)).is_some();
    let in_view_field = |s: &&Scanned| frame_cell(field_cell(s.field)).is_some();
    let family = family.unwrap_or_else(|| {
        let mut best = (0_usize, OreFamily::Metals);
        for candidate in OreFamily::ALL {
            let count = sounded
                .iter()
                .filter(in_view_pocket)
                .filter(|s| by_family(s.classes, candidate) != Word::Barren)
                .count()
                + scanned
                    .iter()
                    .filter(in_view_field)
                    .filter(|s| field_family_text(s, candidate).is_some())
                    .count();
            if count > best.0 {
                best = (count, candidate);
            }
        }
        best.1
    });
    // ---- the grid, bottom layer first ----
    let mut grid: Vec<Vec<char>> = vec![vec![' '; side]; side];
    let put = |grid: &mut Vec<Vec<char>>, (fx, fy): (usize, usize), glyph: char| {
        if let Some(cell) = grid.get_mut(fy).and_then(|row| row.get_mut(fx)) {
            *cell = glyph;
        }
    };
    let crater: BTreeSet<(i64, i64)> = surface
        .features
        .iter()
        .filter(|feature| feature.kind == mush_atlas::record::FeatureKind::Crater)
        .flat_map(|feature| feature.cells.iter())
        .map(|(x, y)| (i64::from(*x), i64::from(*y)))
        .collect();
    let class_at = |x: i64, y: i64| {
        address::composite_cell((x, y))
            .and_then(|(cx, cy)| surface.cells.get(cx, cy))
            .map(|cell| cell.class)
    };
    for fy in 0..side_i {
        for fx in 0..side_i {
            let (sx, sy) = (x0 + fx * span, y0 + fy * span);
            let glyph = if span == 1 {
                if crater.contains(&(sx, sy)) {
                    'O'
                } else {
                    class_at(sx, sy).map_or(' ', terrain_glyph)
                }
            } else {
                // The site view: the commonest footing of the 64 cells, ties to the sharper; a
                // cell at least half crater is `O`.
                let mut counts = [0_u32; 3];
                let mut craters = 0_u32;
                let mut passable = false;
                for dy in 0..span {
                    for dx in 0..span {
                        let (cx, cy) = (sx + dx, sy + dy);
                        if crater.contains(&(cx, cy)) {
                            craters += 1;
                        }
                        match class_at(cx, cy).map(terrain_glyph) {
                            Some('^') => counts[0] += 1,
                            Some(',') => counts[1] += 1,
                            Some('=') => counts[2] += 1,
                            Some('_') => passable = true,
                            _ => {}
                        }
                    }
                }
                let cells = u32::try_from(span * span).unwrap_or(64);
                if craters * 2 >= cells {
                    'O'
                } else {
                    let best = counts.iter().copied().max().unwrap_or(0);
                    if best == 0 {
                        if passable { '_' } else { ' ' }
                    } else if counts[0] == best {
                        '^'
                    } else if counts[1] == best {
                        ','
                    } else {
                        '='
                    }
                }
            };
            if let (Ok(ux), Ok(uy)) = (usize::try_from(fx), usize::try_from(fy)) {
                put(&mut grid, (ux, uy), glyph);
            }
        }
    }
    // ---- the layers above the terrain, in frame cells ----
    let block_side = address::CELLS_PER_BLOCK;
    let mut layers = Overlays::default();
    for by in 0..crate::surface::ground::SITE_BLOCKS {
        for bx in 0..crate::surface::ground::SITE_BLOCKS {
            let Some(letter) = address::letter_of_block(bx, by) else {
                continue;
            };
            let corner = (bx * block_side, by * block_side + block_side - 1);
            if let Some(at) = frame_cell(corner) {
                layers.letters.push((at, letter));
            }
        }
    }
    layers.claims = claim_edges()
        .filter_map(|(cell, glyph)| frame_cell(cell).map(|at| (at, glyph)))
        .collect();
    layers.pits = pit_layer(reads, &entry, &ground)
        .into_iter()
        .filter_map(|(pocket, glyph)| frame_cell(pocket_cell(pocket)).map(|at| (at, glyph)))
        .collect();
    layers.readings = best_readings(sounded.iter().filter_map(|s| {
        frame_cell(pocket_cell(s.pocket)).map(|at| (at, reading_glyph(&s.classes, family)))
    }));
    let hull_here = crate::port::coop::programme_port(w).is_some_and(|port| {
        port.map.hulls.iter().any(|row| {
            matches!(
                crate::port::ship::berth_of(w, ObjId(row.hull)),
                crate::port::ship::Berth::Docked { map, .. } if map.entry == entry
            )
        })
    });
    layers.hull = if hull_here {
        frame_cell(PAD_CELL)
    } else {
        None
    };
    let mine: BTreeSet<ObjId> = rigs.iter().map(|rig| rig.rig.tv).collect();
    let others: Vec<(ObjId, Option<RigRecord>, (i64, i64))> = crate::surface::act::world_rigs(w)
        .into_iter()
        .filter(|tv| !mine.contains(tv))
        .filter_map(|tv| {
            let record = reads.rig(tv).ok().flatten();
            match place_of(w, record.as_ref(), tv, (&entry, berth.domain), None, now) {
                Place::Ground(cell) => Some((tv, record, cell)),
                Place::Bay | Place::Away => None,
            }
        })
        .collect();
    layers.others = others
        .iter()
        .filter_map(|(_, _, cell)| frame_cell(*cell))
        .collect();
    layers.mine = on_ground
        .iter()
        .filter_map(|rig| match rig.place {
            Place::Ground(cell) => frame_cell(cell).map(|at| {
                (
                    rig.rig.slot,
                    at,
                    frame_cell((cell.0.saturating_add(span), cell.1)),
                )
            }),
            Place::Bay | Place::Away => None,
        })
        .collect();
    compose(&mut grid, &layers);
    // ---- the units overlay (R5): listed only where `rows` holds the glyph ----
    let addressed_slot = addressed.map(|rig| rig.rig.slot);
    let units = list_units(&grid, &layers, addressed_slot);
    let you = addressed_cell.and_then(frame_cell);
    let rows: Vec<Value> = grid.iter().map(|row| run_length(row)).collect();
    // ---- the vector block ----
    let reference = match view {
        MapView::Drive { .. } => addressed_cell.unwrap_or(ramp_cell),
        MapView::Site => PAD_CELL,
    };
    let mut readings: Vec<(u64, String, Value)> = Vec::new();
    for s in sounded.iter().filter(in_view_pocket) {
        let class = by_family(s.classes, family);
        if class == Word::Barren {
            continue;
        }
        let Some(addr) = address::pocket_address(s.pocket.0, s.pocket.1) else {
            continue;
        };
        let text = format!("{addr} {} {}", class.name(), family.label());
        let cell = pocket_cell(s.pocket);
        let (m, dir) = bearing_words(reference, cell);
        readings.push((
            m,
            text.clone(),
            json!({ "text": text, "cell": [cell.0, cell.1], "m": m, "dir": dir }),
        ));
    }
    for s in scanned.iter().filter(in_view_field) {
        let Some(text) = field_family_text(s, family) else {
            continue;
        };
        let cell = field_cell(s.field);
        let (m, dir) = bearing_words(reference, cell);
        readings.push((
            m,
            text.clone(),
            json!({ "text": text, "cell": [cell.0, cell.1], "m": m, "dir": dir }),
        ));
    }
    readings.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    readings.truncate(MAX_READING_ROWS);
    let battery_of = |tv: ObjId| {
        w.module_state::<PortHandles>()
            .and_then(|handles| handles.surface_tools.clone())
            .and_then(|tools| {
                crate::surface::act::rig_battery_id(w, tv)
                    .and_then(|id| crate::surface::act::battery_of(&tools, &id).cloned())
            })
    };
    let pool_unit = tv::pool_unit(w);
    let read = |endpoint: EndpointRef, good: &CanonicalId| reads.pool(endpoint, good);
    let ramp = berth.ramp_label.clone();
    let mut rig_rows: Vec<Value> = Vec::new();
    let mut rig_lines: Vec<String> = Vec::new();
    for rig in &rigs {
        let battery = battery_of(rig.rig.tv);
        let charge = match (&battery, &rig.record) {
            (Some(battery), record) => {
                let record = record
                    .clone()
                    .unwrap_or_else(|| RigRecord::fresh(rig.rig.tv, battery));
                Some(crate::surface::act::percent(
                    charge_shown(&record, now, battery),
                    battery,
                ))
            }
            (None, _) => None,
        };
        let hopper = tv::hopper_load_with(w, rig.rig.tv, pool_unit.as_ref(), &read)
            .map_or_else(|_| "?".to_owned(), |(kg, cap)| tv::hopper_text(kg, cap));
        let act = rig.record.as_ref().and_then(|record| record.act.as_ref());
        let cell = match rig.place {
            Place::Ground(cell) => Some(cell),
            Place::Bay | Place::Away => None,
        };
        let leg = act.and_then(|act| {
            act.params.leg().map(|(waypoints, _)| {
                let launch = if act.params.launches() {
                    i64::from(LAUNCH_S)
                } else {
                    0
                };
                json!({
                    "started_ic": act.started_ic.saturating_add(launch),
                    "waypoints": waypoints,
                })
            })
        });
        rig_rows.push(json!({
            "slot": rig.rig.slot,
            "mark": rig.rig.mark,
            "cell": cell.map(|(x, y)| json!([x, y])),
            "charge_pct": charge,
            "hopper": hopper,
            "act": act.map(|act| json!({ "kind": act.kind.word(), "due_ic": act.due_ic })),
            "leg": leg,
        }));
        let doing = act.map_or_else(String::new, |act| {
            format!(
                "; {}, there {}",
                crate::surface::act::doing_words(act.kind, &act.params, &ramp),
                tv::due_words(w, act.due_ic)
            )
        });
        let place = match (rig.place, cell) {
            (_, Some((x, y))) => format!("({x}, {y})"),
            (Place::Bay, None) => "in the bay".to_owned(),
            (_, None) => "away".to_owned(),
        };
        rig_lines.push(format!(
            "Rig {}: {place}, charge {} %, hopper {hopper}{doing}.",
            rig.rig.slot,
            charge.map_or_else(|| "?".to_owned(), |c| c.to_string())
        ));
    }
    // Other rigs: where, and a loud sounding inside the IC hour (§5's transient mark) — one
    // made on THIS site (the review's correctness/F1: a sounding heard on another site's ground
    // is no mark here).
    let mut other_rows: Vec<Value> = Vec::new();
    let mut other_words: Vec<String> = Vec::new();
    for (_, record, cell) in &others {
        let sounding = record
            .as_ref()
            .and_then(|record| record.last.as_ref())
            .is_some_and(|last| {
                last.site == entry
                    && last.result == crate::surface::excursion::ActResult::Done
                    && matches!(
                        last.span,
                        Some(crate::surface::excursion::ReadSpan::Sound { quiet: false, .. })
                    )
                    && last.at_ic >= now.saturating_sub(crate::surface::excursion::IC_HOUR)
            });
        let (m, dir) = bearing_words(reference, *cell);
        other_rows
            .push(json!({ "cell": [cell.0, cell.1], "m": m, "dir": dir, "sounding": sounding }));
        other_words.push(format!(
            "{m} m {dir}{}",
            if sounding { " (a sounding)" } else { "" }
        ));
    }
    let hold_room = crate::port::ship::hold_unreserved_kg(
        w,
        &hull_view,
        &crate::port::tv::live_on_hull(w, helm.hull)
            .iter()
            .map(|lease| lease.tv)
            .collect::<Vec<_>>(),
        &read,
        &|tv| reads.rig(tv),
    );
    let mut lines: Vec<String> = Vec::new();
    if readings.is_empty() {
        lines.push(MAP_NO_READINGS.replace("{family}", family.label()));
    } else {
        let rows: Vec<String> = readings
            .iter()
            .map(|(m, text, value)| {
                let dir = value.get("dir").and_then(Value::as_str).unwrap_or("");
                if *m == 0 {
                    format!("{text} here")
                } else {
                    format!("{text} {m} m {dir}")
                }
            })
            .collect();
        lines.extend(wrap(&format!(
            "Readings, {}: {}",
            family.label(),
            rows.join("; ")
        )));
    }
    for line in rig_lines {
        lines.extend(wrap(&line));
    }
    if !other_words.is_empty() {
        lines.extend(wrap(&format!("Other rigs: {}", other_words.join("; "))));
    }
    if let Some(kg) = hold_room {
        lines.push(format!("Hold: {} t of room unreserved.", kg / 1_000));
    }
    let title = match (view, addressed) {
        (MapView::Drive { .. }, Some(rig)) => MAP_TITLE_DRIVE
            .replace("{n}", &rig.rig.slot.to_string())
            .replace("{site}", &berth.name)
            .replace("{family}", family.label()),
        _ => MAP_TITLE_SITE
            .replace("{site}", &berth.name)
            .replace("{family}", family.label()),
    };
    let footer = match view {
        MapView::Drive { .. } => {
            let at = match (addressed, addressed_cell) {
                (Some(rig), Some((x, y))) => format!("rig {} at ({x}, {y})", rig.rig.slot),
                (Some(rig), None) => format!("rig {} in the bay", rig.rig.slot),
                (None, _) => String::new(),
            };
            format!(
                "x {x0}..{}  y {y0}..{}  of 256x256  {at}  (10 m a cell, north up)",
                x0 + side_i - 1,
                y0 + side_i - 1
            )
        }
        MapView::Site => "the whole site, 80 m a cell, north up".to_owned(),
    };
    let legend = json!([
        ["@", "you"],
        ["H", "hull"],
        ["o", "rig"],
        [",", "regolith"],
        ["^", "rock"],
        ["=", "ice"],
        ["O", "crater"],
        ["3", "rich, 3 t a cycle"],
        ["2", "fair, 2 t a cycle"],
        ["1", "lean, 1 t a cycle"],
        ["+", "other family"],
        [".", "barren"],
    ]);
    let (origin_cell, extent) = match view {
        MapView::Drive { .. } => (json!([x0, y0]), json!([site_side, site_side])),
        MapView::Site => (json!([0, 0]), json!([side, side])),
    };
    let mut frame = json!({
        "schema": { "id": SITE_MAP_FEED, "shape": "grid", "version": SCHEMA_VERSION },
        "window": 0,
        "revision": 0,
        "tick": 0,
        "title": title,
        "surface": "ground",
        "floors": [],
        "origin": [x0.saturating_mul(1_000), y0.saturating_mul(1_000)],
        "cell_cm": cell_cm,
        "cell_columns": columns,
        "width": side,
        "height": side,
        "rows": rows,
        "legend": legend,
        "units": units,
        "you": you.map(|(x, y)| json!([x, y])),
        "origin_cell": origin_cell,
        "extent": extent,
        "site": {
            "view": match view { MapView::Drive { .. } => "drive", MapView::Site => "site" },
            "site": berth.name,
            "family": family.label(),
            "legend_line": SITE_LEGEND,
            "readings": readings.into_iter().map(|(_, _, value)| value).collect::<Vec<_>>(),
            "rigs": rig_rows,
            "others": other_rows,
            "hold": hold_room.map(|kg| json!({ "room_t": kg / 1_000 })),
            "footer": footer,
            "lines": lines,
        },
    });
    let print = site_map_lines(&frame);
    if let Some(block) = frame.get_mut("site").and_then(Value::as_object_mut) {
        block.insert("print".to_owned(), json!(print));
    }
    Ok(frame)
}

/// **The legend as printed** (doc 20 cut SM.4b): [`SITE_LEGEND`] verbatim — but doc 20 §10's
/// legend is 122 columns, past its own 118-column cap, so it is broken at the last entry
/// separator (two spaces) that keeps the first line within [`MAX_LINE_COLUMNS`], the rest on a
/// second line led by that separator: the two lines joined are the legend, byte for byte.
#[must_use]
pub(crate) fn legend_lines() -> Vec<String> {
    if SITE_LEGEND.chars().count() <= MAX_LINE_COLUMNS {
        return vec![SITE_LEGEND.to_owned()];
    }
    let cut = SITE_LEGEND
        .match_indices("  ")
        .map(|(at, _)| at)
        .filter(|at| {
            SITE_LEGEND
                .get(..*at)
                .is_some_and(|head| head.chars().count() <= MAX_LINE_COLUMNS)
        })
        .last();
    match cut.and_then(|at| Some((SITE_LEGEND.get(..at)?, SITE_LEGEND.get(at..)?))) {
        Some((head, tail)) => vec![head.to_owned(), tail.to_owned()],
        None => vec![SITE_LEGEND.to_owned()],
    }
}

/// **`+tv/map`'s print of a frame** (doc 20 cut SM.4b; lead ruling 7): the title; doc 20's legend
/// line verbatim ([`legend_lines`]); the grid rows north up, each glyph printed `max(cell_columns, 2)` times, read
/// through **`mush_ansi::window_rows`** — the TUI's own decoder — so the two cannot disagree; the
/// footer; then the vector block's lines. A frame with no grid prints its title alone (the
/// reason). Every line at most [`MAX_LINE_COLUMNS`].
#[must_use]
pub(crate) fn site_map_lines(frame: &Value) -> Vec<String> {
    let title = frame
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let num = |key: &str| {
        frame
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0)
    };
    let (width, height) = (num("width"), num("height"));
    if width == 0 || height == 0 {
        return vec![title];
    }
    let per_cell = num("cell_columns").max(2);
    let empty = Vec::new();
    let raw = frame
        .get("rows")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let grid = mush_ansi::panel::window_rows(raw, width, height, 0, 0, width, height);
    let mut lines = vec![title];
    lines.extend(legend_lines());
    for row in grid.iter().rev() {
        lines.push(
            row.iter()
                .flat_map(|glyph| std::iter::repeat_n(*glyph, per_cell))
                .collect(),
        );
    }
    let block = frame.get("site");
    if let Some(footer) = block.and_then(|b| b.get("footer")).and_then(Value::as_str) {
        lines.push(footer.to_owned());
    }
    if let Some(vector) = block.and_then(|b| b.get("lines")).and_then(Value::as_array) {
        lines.extend(vector.iter().filter_map(Value::as_str).map(str::to_owned));
    }
    lines
}

/// **`+tv[/<n>]/map [site] [family]`** (doc 20 §10; cut SM.4b, lead ruling 7): a pure read — no
/// write, no submission — of the actor's own frame through the command's typed reads, each line
/// said to the actor (the `+chart/map` way). A cold site is asked for (RAM only) and answers
/// `GROUND_COLD`; an unknown family `MAP_FAMILY_UNKNOWN`.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
pub(crate) fn map_cmd(
    ctx: &mut Ctx<'_>,
    slot: Option<u8>,
    site: bool,
    family: Option<&str>,
) -> Result<(), CmdError> {
    let family = match family {
        None => None,
        Some(word) => Some(
            crate::surface::geology::family_of_word(word).ok_or_else(|| {
                CmdError::msg(crate::port::tv::MAP_FAMILY_UNKNOWN.replace("{word}", word))
            })?,
        ),
    };
    let view = if site {
        MapView::Site
    } else {
        MapView::Drive { slot }
    };
    let actor = ctx.actor;
    let framed = {
        let reads: &Ctx<'_> = ctx;
        site_map_frame(reads.w, reads, actor, view, family)
    };
    let frame = match framed {
        Ok(frame) => frame,
        Err(refused) => {
            if let Some((entry, ramp)) = &refused.want {
                crate::surface::want_home(ctx.w, entry, *ramp);
            }
            return Err(CmdError::msg(refused.line));
        }
    };
    for line in site_map_lines(&frame) {
        ctx.out.sys_to_actor(actor, "tacsim", line);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The glyph layers stack in order** (lead ruling 6): a reading over a pit over a block
    /// letter over the terrain; the rank of the reading glyphs; the family words; the legend
    /// line verbatim and within the 118-column print.
    #[test]
    fn sm4b_glyph_layers_stack_in_order() {
        use mush_atlas::TerrainClass;
        assert_eq!(terrain_glyph(TerrainClass::Regolith), ',');
        assert_eq!(terrain_glyph(TerrainClass::Rock), '^');
        assert_eq!(terrain_glyph(TerrainClass::Ice), '=');
        assert_eq!(terrain_glyph(TerrainClass::Paved), '_');
        assert_eq!(terrain_glyph(TerrainClass::Wall), ' ');
        let classes = [Word::Fair, Word::Barren, Word::Rich, Word::Barren];
        assert_eq!(reading_glyph(&classes, OreFamily::Metals), '2');
        assert_eq!(reading_glyph(&classes, OreFamily::CarbonFeedstock), '3');
        assert_eq!(reading_glyph(&classes, OreFamily::Volatiles), '+');
        assert_eq!(reading_glyph(&[Word::Barren; 4], OreFamily::Metals), '.');
        assert_eq!(
            reading_glyph(
                &[Word::Lean, Word::Barren, Word::Barren, Word::Barren],
                OreFamily::Metals
            ),
            '1'
        );
        for (a, b) in [('3', '2'), ('2', '1'), ('1', '+'), ('+', '.'), ('.', 'v')] {
            assert!(rank(a) > rank(b), "{a} over {b}");
        }
        // Doc 20 §10's legend is 122 columns: printed verbatim over two lines within 118.
        let printed = legend_lines();
        assert_eq!(printed.len(), 2);
        assert!(
            printed
                .iter()
                .all(|l| l.chars().count() <= MAX_LINE_COLUMNS)
        );
        assert_eq!(
            printed.concat(),
            SITE_LEGEND,
            "the two lines are the legend, verbatim"
        );
        assert_eq!(printed.get(1).map(String::as_str), Some("  # claim"));
        assert_eq!(
            SITE_LEGEND,
            "@ you  H hull  o rig  , regolith  ^ rock  = ice  O crater  1–3 ore (t a cycle)  + other family  . barren  v/V pit  # claim"
        );
        assert_eq!(claim_edges().count(), 0, "claims are SM.10's");
        // The digits are the drill row's yield (the legend's "t a cycle").
        let drill = crate::definitions::tools::tools_table()
            .unwrap_or_else(|e| panic!("{e}"))
            .values()
            .find_map(crate::definitions::tools::ToolDef::drill)
            .copied()
            .unwrap_or_else(|| panic!("a drill row"));
        assert_eq!(
            (drill.yield_t.lean, drill.yield_t.fair, drill.yield_t.rich),
            (1, 2, 3)
        );
        // Wrapping keeps every line within the floor.
        let long = format!(
            "Readings, metals: {}",
            ["C117 fair metals 120 m NE"; 12].join("; ")
        );
        let wrapped = wrap(&long);
        assert!(wrapped.len() > 1);
        assert!(
            wrapped
                .iter()
                .all(|l| l.chars().count() <= MAX_LINE_COLUMNS)
        );
        assert!(wrapped.iter().skip(1).all(|l| l.starts_with("  ")));
        // A synthetic frame: every layer stacked on one cell, each layer alone on its own, so
        // the cell keeps the top layer and the others show each layer over the terrain.
        let mut grid = vec![vec![','; 9]; 2];
        let stack = (0, 0);
        let layers = Overlays {
            letters: vec![(stack, 'C'), ((1, 0), 'C')],
            claims: vec![(stack, '#'), ((2, 0), '#')],
            pits: vec![(stack, 'v'), ((3, 0), 'V')],
            readings: [(stack, '2'), ((4, 0), '.')].into_iter().collect(),
            hull: Some(stack),
            others: vec![stack, (6, 0)],
            mine: vec![(1, stack, Some((0, 1))), (2, (7, 0), Some((8, 0)))],
        };
        compose(&mut grid, &layers);
        let row = |y: usize| grid.get(y).map(|r| r.iter().collect::<String>());
        assert_eq!(row(0).as_deref(), Some("@C#V.,o@2"));
        assert_eq!(
            row(1).as_deref(),
            Some("1,,,,,,,,"),
            "two rigs on the ground: each slot digit at its own cell"
        );
        // Each layer peeled off the stacked cell uncovers the next one down.
        let peel = [
            ('o', "mine"),
            ('H', "others"),
            ('2', "hull"),
            ('v', "readings"),
            ('#', "pits"),
            ('C', "claims"),
            (',', "letters"),
        ];
        let mut layers = layers;
        layers.mine.retain(|(_, at, _)| *at != stack);
        for (under, peeled) in peel {
            let mut grid = vec![vec![','; 9]; 2];
            compose(&mut grid, &layers);
            assert_eq!(
                grid.first().and_then(|r| r.first()).copied(),
                Some(under),
                "with {peeled} peeled"
            );
            match peeled {
                "mine" => layers.others.retain(|at| *at != stack),
                "others" => layers.hull = None,
                "hull" => {
                    layers.readings.remove(&stack);
                }
                "readings" => layers.pits.retain(|(at, _)| *at != stack),
                "pits" => layers.claims.retain(|(at, _)| *at != stack),
                "claims" => layers.letters.retain(|(at, _)| *at != stack),
                _ => {}
            }
        }
        // One rig on the ground: no slot digit.
        let mut grid = vec![vec![','; 3]; 1];
        let lone = Overlays {
            mine: vec![(1, (0, 0), Some((1, 0)))],
            ..Overlays::default()
        };
        compose(&mut grid, &lone);
        assert_eq!(grid.concat().iter().collect::<String>(), "@,,");
    }

    /// **The units overlay reproduces the rows** (R5, binding): an `o` sharing an 80 m cell
    /// with the viewer's `@`, and an `o` standing where a two-rig viewer's slot digit is
    /// painted, are not listed; the listed ones are exactly the glyphs the rows hold, so
    /// `mush_ansi::panel::expand_rows` (rows plus units) equals the bare rows.
    #[test]
    fn sm4b_the_units_overlay_reproduces_the_rows() {
        let mut grid = vec![vec![','; 6]; 3];
        let layers = Overlays {
            others: vec![(0, 0), (2, 0), (4, 2)],
            mine: vec![(2, (1, 0), Some((2, 0))), (1, (0, 0), Some((1, 0)))],
            ..Overlays::default()
        };
        compose(&mut grid, &layers);
        let units = list_units(&grid, &layers, Some(2));
        let ids: Vec<&str> = units
            .iter()
            .filter_map(|u| u.get("id").and_then(Value::as_str))
            .collect();
        // Rig 1's digit lands under rig 2's `@` (east of (0,0)); rig 2's digit covers the `o`
        // at (2,0); the `o` at (0,0) is under rig 1's `@`. Only the far `o` is listed.
        assert_eq!(ids, ["other-1", "rig-1", "rig-2"]);
        let you: Vec<bool> = units
            .iter()
            .filter_map(|u| u.get("you").and_then(Value::as_bool))
            .collect();
        assert_eq!(you, [false, false, true]);
        assert_eq!(
            units.first().and_then(|u| u.get("cell")),
            Some(&json!([4, 2]))
        );
        let rows: Vec<Value> = grid.iter().map(|row| run_length(row)).collect();
        let frame = json!({ "width": 6, "height": 3, "rows": rows, "units": units });
        let bare = mush_ansi::panel::window_rows(
            frame
                .get("rows")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice),
            6,
            3,
            0,
            0,
            6,
            3,
        );
        assert_eq!(mush_ansi::panel::expand_rows(&frame), bare);
        assert_eq!(bare, grid);
        assert_eq!(
            grid.first()
                .map(|r| r.iter().collect::<String>())
                .as_deref(),
            Some("@@2,,,")
        );
    }

    /// **A leg interpolates between its waypoints** (§8): the start before it, the end after
    /// it, integer linear in between.
    #[test]
    fn sm4b_a_leg_interpolates() {
        let leg = [
            LegWaypoint {
                x_cm: 0,
                y_cm: 0,
                at_s: 0,
            },
            LegWaypoint {
                x_cm: 10_000,
                y_cm: 0,
                at_s: 100,
            },
            LegWaypoint {
                x_cm: 10_000,
                y_cm: 20_000,
                at_s: 300,
            },
        ];
        assert_eq!(leg_position(&leg, -5), Some((0, 0)));
        assert_eq!(leg_position(&leg, 50), Some((5_000, 0)));
        assert_eq!(leg_position(&leg, 100), Some((10_000, 0)));
        assert_eq!(leg_position(&leg, 200), Some((10_000, 10_000)));
        assert_eq!(leg_position(&leg, 9_999), Some((10_000, 20_000)));
        assert_eq!(leg_position(&[], 5), None);
    }

    /// **The site view takes the best of four pockets** (lead ruling 6): ranks pick the best
    /// glyph a cell's pockets give; a pocket and its address cell agree.
    #[test]
    fn sm4b_the_site_view_takes_the_best_of_four_pockets() {
        let best = best_readings(
            [
                ((0, 0), '.'),
                ((0, 0), '+'),
                ((0, 0), '1'),
                ((0, 0), '.'),
                ((1, 0), '.'),
                ((1, 0), '3'),
                ((1, 0), '2'),
            ]
            .into_iter(),
        );
        assert_eq!(best.get(&(0, 0)), Some(&'1'));
        assert_eq!(best.get(&(1, 0)), Some(&'3'));
        assert_eq!(best.len(), 2);
        // Pockets (0..2, 0..2) are site cells 2..=6, all in the first 80 m cell.
        for px in 0..2 {
            for py in 0..2 {
                let (x, y) = pocket_cell((px, py));
                assert_eq!((x / SITE_VIEW_SPAN, y / SITE_VIEW_SPAN), (0, 0));
            }
        }
        assert_eq!(pocket_cell((36, 56)), (146, 226), "C117's address cell");
        assert_eq!(field_cell((4, 7)), (144, 240), "C-NW's middle");
    }

    /// **Family words parse** (the brief's 3.9): every alias, case-insensitive; anything else
    /// does not.
    #[test]
    fn sm4b_family_words_parse() {
        use crate::surface::geology::family_of_word;
        for (word, family) in [
            ("metal", OreFamily::Metals),
            ("Metals", OreFamily::Metals),
            ("metal_ore", OreFamily::Metals),
            ("ice", OreFamily::Volatiles),
            ("volatiles", OreFamily::Volatiles),
            ("ice_ore", OreFamily::Volatiles),
            ("carbon", OreFamily::CarbonFeedstock),
            ("carbon_ore", OreFamily::CarbonFeedstock),
            ("strategic", OreFamily::StrategicElements),
            ("STRATEGIC_ORE", OreFamily::StrategicElements),
        ] {
            assert_eq!(family_of_word(word), Some(family), "{word}");
        }
        for bad in ["quartz", "", "ore", "metalz"] {
            assert_eq!(family_of_word(bad), None, "{bad}");
        }
    }
}
