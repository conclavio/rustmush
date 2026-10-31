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
/// **The pushed status feed** (doc 20 cut SM.9; §13's SM.9 row, item 3.4, decision D13): one
/// keyframe to a rig's operator each time its `tacsim:tv:<tv>` projection changes — a start's
/// first fire, a due step, a tell — so the quarry screens pull `site.map` again. Shape `status`
/// (not `grid`), so a line-mode connection prints nothing; push only, no query handler.
pub const TV_STATUS_FEED: &str = "tv.status";
/// **The run clause the screens show under a drilling rig's line** (doc 20 cut SM.9; the
/// brief's 3.3 item 7): `{addr}`, `{ore}`, `{k}`, `{cycles}`, `{when}`, `{stop}` — the same words
/// as `+tv` status's `RUN_LINE`, read from the operator's own projection, never the record's run.
/// It is never appended to `site.lines` or `site.print` (`+tv/map` is unchanged).
pub const MAP_RUN_CLAUSE: &str =
    "drilling {addr} for {ore}, cycle {k} of {cycles}, this cycle done {when}{stop}";
/// **The clause under a coring rig** (doc 20 cut SM.9; a blind core): `{addr}`, `{ore}`,
/// `{when}`.
pub const MAP_CORE_CLAUSE: &str = "coring {addr} for {ore}, done {when}";
/// **The most log lines the frame carries** (doc 20 cut SM.9; §10's Right pane), newest last.
pub const MAP_LOG_LINES: usize = 6;
/// A site cell's edge, centimetres (the drive view's cell; `site.grid.cell_cm`).
const SITE_CELL_EDGE_CM: i64 = 1_000;

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
    /// **One pool's availability** (doc 20 cut SM.11, the reviewer's R1): its balance less every
    /// live hold — [`crate::port_tick::read_available`]'s rule (absent is 0; a scalar without its
    /// book, or a book without its scalar, is `Corrupt`; a refusal is never 0).
    fn available(&self, endpoint: EndpointRef, good: &CanonicalId) -> Result<u64, PoolReadError>;
    /// **An endpoint's inventory marker ring** (doc 20 cut SM.11, the reviewer's R1): `None` when
    /// nothing was ever marked there — what a queued unload's start offer, the surface tick and
    /// the plan's `Markers` wait read through `crate::surface::act::queued_ring_full`.
    fn marker_ring(
        &self,
        endpoint: EndpointRef,
    ) -> Result<Option<mush_inventory::MarkerRing>, String>;
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
    /// **A block's public tenure** (doc 20 cut SM.10, D17): its `tacsim.claims` record, `None`
    /// when nothing was ever staked, seeded or papered there.
    fn block_tenure(
        &self,
        key: &crate::surface::claims::BlockKey,
    ) -> Result<Option<crate::surface::claims::BlockTenure>, String>;
    /// **A block place's industry tenure** (doc 20 cut SM.10): the epoch, its start and the
    /// cooling rows, read only through the delegation (`None` = never moved: fresh).
    fn tenure(&self, place: ObjId) -> Result<Option<mush_industry::TenureRecord>, String>;
    /// **Every chart row of one site, bounded** (doc 20 cut SM.10, D12): a paged scan of
    /// `tacsim.chart` keeping the rows whose site is `site`, stopping once `max` records were
    /// **read**; `true` when it stopped short of the family's end.
    fn site_charts(
        &self,
        site: &CanonicalId,
        max: usize,
    ) -> Result<(Vec<ChartRecord>, bool), String>;
    /// **A site's record** (doc 32 cut CF.1): its `tacsim.field` record, `None` when nothing was
    /// ever left there and no field opened; a record whose key disagrees with its value reads
    /// `None`.
    fn site_record(
        &self,
        site: &CanonicalId,
    ) -> Result<Option<crate::surface::site_record::SiteRecord>, String>;
}

/// The handles, or why not.
fn handles_of(w: &World) -> Result<&PortHandles, String> {
    w.module_state::<PortHandles>()
        .ok_or_else(|| "the port handles are not composed".to_owned())
}

/// The inventory's families, or why not.
fn inventory_of(w: &World) -> Result<&mush_inventory::InventoryFamilies, PoolReadError> {
    w.module_state::<PortHandles>()
        .and_then(|handles| handles.inventory.as_ref())
        .ok_or(PoolReadError::Unwired)
}

/// **A pool's availability from its two reads** (doc 20 cut SM.11, the reviewer's R1):
/// [`crate::port_tick::read_available`]'s rule over any reader's scalar and book.
fn available_of<E: std::fmt::Display>(
    scalar: Result<Option<&mush_core::BoundedScalar>, E>,
    book: Result<Option<&mush_core::ReservationBook>, E>,
) -> Result<u64, PoolReadError> {
    let scalar = scalar.map_err(|error| PoolReadError::Unreadable(error.to_string()))?;
    let book = book.map_err(|error| PoolReadError::Unreadable(error.to_string()))?;
    match (scalar, book) {
        (None, None) => Ok(0),
        (Some(scalar), Some(book)) => book
            .checked_available(*scalar)
            .map_err(|_| PoolReadError::Corrupt),
        (None, Some(_)) | (Some(_), None) => Err(PoolReadError::Corrupt),
    }
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

/// One page of `tacsim.chart` from `(after, limit)`: the records and the cursor past them.
type ChartPage<'a> =
    dyn FnMut(Option<&str>, usize) -> Result<(Vec<ChartRecord>, Option<SmolStr>), String> + 'a;

/// **The bounded site-chart scan** (doc 20 cut SM.10, D12): pages of `tacsim.chart` through
/// `page(after, limit)` until the family ends or `max` records were read, keeping the rows of
/// `site`. `true` when it stopped at `max` with more of the family left.
fn scan_site_charts(
    site: &CanonicalId,
    max: usize,
    page: &mut ChartPage<'_>,
) -> Result<(Vec<ChartRecord>, bool), String> {
    /// One page's records.
    const PAGE: usize = 64;
    let mut out = Vec::new();
    let mut read = 0_usize;
    let mut after: Option<SmolStr> = None;
    loop {
        let left = max.saturating_sub(read);
        if left == 0 {
            return Ok((out, after.is_some()));
        }
        let (rows, next) = page(after.as_deref(), left.min(PAGE))?;
        if rows.is_empty() {
            return Ok((out, false));
        }
        read = read.saturating_add(rows.len());
        out.extend(rows.into_iter().filter(|row| &row.site == site));
        match next {
            None => return Ok((out, false)),
            Some(next) => after = Some(next),
        }
    }
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

    fn available(&self, endpoint: EndpointRef, good: &CanonicalId) -> Result<u64, PoolReadError> {
        let inventory = inventory_of(&self.0.world)?;
        let key = mush_inventory::pool_key(endpoint, good)
            .map_err(|error| PoolReadError::Key(error.to_string()))?;
        let state = self.0.action_state();
        available_of(
            state.get(&inventory.pools, &key),
            state.get(&inventory.books, &key),
        )
    }

    fn marker_ring(
        &self,
        endpoint: EndpointRef,
    ) -> Result<Option<mush_inventory::MarkerRing>, String> {
        let inventory = inventory_of(&self.0.world).map_err(|error| format!("{error:?}"))?;
        self.0
            .action_state()
            .get(&inventory.marker_ring, &endpoint)
            .map(|found| found.cloned())
            .map_err(|error| error.to_string())
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

    fn block_tenure(
        &self,
        key: &crate::surface::claims::BlockKey,
    ) -> Result<Option<crate::surface::claims::BlockTenure>, String> {
        let handles = handles_of(&self.0.world)?;
        self.0
            .action_state()
            .get(&handles.families.claims, key)
            .map(|found| found.cloned().filter(|row| row.key().as_ref() == Some(key)))
            .map_err(|error| error.to_string())
    }

    fn tenure(&self, place: ObjId) -> Result<Option<mush_industry::TenureRecord>, String> {
        let families = industry_of(&self.0.world)?;
        self.0
            .action_state()
            .get(&families.tenure, &place)
            .map(|found| found.cloned().filter(|row| row.place() == place))
            .map_err(|error| error.to_string())
    }

    fn site_charts(
        &self,
        site: &CanonicalId,
        max: usize,
    ) -> Result<(Vec<ChartRecord>, bool), String> {
        let handles = handles_of(&self.0.world)?;
        scan_site_charts(site, max, &mut |after, limit| {
            let page = self
                .0
                .action_state()
                .scan_family(&handles.families.charts, after, limit)
                .map_err(|error| error.to_string())?;
            Ok((
                page.entries()
                    .iter()
                    .map(|entry| entry.value().clone())
                    .collect(),
                page.next_after().cloned(),
            ))
        })
    }

    fn site_record(
        &self,
        site: &CanonicalId,
    ) -> Result<Option<crate::surface::site_record::SiteRecord>, String> {
        let handles = handles_of(&self.0.world)?;
        let key = crate::surface::site_record::SiteKey(site.clone());
        self.0
            .action_state()
            .get(&handles.families.sites, &key)
            .map(|found| found.cloned().filter(|record| record.key() == key))
            .map_err(|error| error.to_string())
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

    fn available(&self, endpoint: EndpointRef, good: &CanonicalId) -> Result<u64, PoolReadError> {
        crate::port_tick::read_available(self, endpoint, good)
    }

    fn marker_ring(
        &self,
        endpoint: EndpointRef,
    ) -> Result<Option<mush_inventory::MarkerRing>, String> {
        let inventory = inventory_of(self.w).map_err(|error| format!("{error:?}"))?;
        self.action_state(&inventory.marker_ring, &endpoint)
            .map(|found| found.cloned())
            .map_err(|error| error.to_string())
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

    fn block_tenure(
        &self,
        key: &crate::surface::claims::BlockKey,
    ) -> Result<Option<crate::surface::claims::BlockTenure>, String> {
        let handles = handles_of(self.w)?;
        self.action_state(&handles.families.claims, key)
            .map(|found| found.cloned().filter(|row| row.key().as_ref() == Some(key)))
            .map_err(|error| error.to_string())
    }

    fn tenure(&self, place: ObjId) -> Result<Option<mush_industry::TenureRecord>, String> {
        let families = industry_of(self.w)?;
        self.action_state(&families.tenure, &place)
            .map(|found| found.cloned().filter(|row| row.place() == place))
            .map_err(|error| error.to_string())
    }

    fn site_charts(
        &self,
        site: &CanonicalId,
        max: usize,
    ) -> Result<(Vec<ChartRecord>, bool), String> {
        let handles = handles_of(self.w)?;
        scan_site_charts(site, max, &mut |after, limit| {
            let page = self
                .scan_action_state(&handles.families.charts, after, limit)
                .map_err(|error| error.to_string())?;
            Ok((
                page.entries()
                    .iter()
                    .map(|entry| entry.value().clone())
                    .collect(),
                page.next_after().cloned(),
            ))
        })
    }

    fn site_record(
        &self,
        site: &CanonicalId,
    ) -> Result<Option<crate::surface::site_record::SiteRecord>, String> {
        let handles = handles_of(self.w)?;
        let key = crate::surface::site_record::SiteKey(site.clone());
        self.action_state(&handles.families.sites, &key)
            .map(|found| found.cloned().filter(|record| record.key() == key))
            .map_err(|error| error.to_string())
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

    fn available(&self, endpoint: EndpointRef, good: &CanonicalId) -> Result<u64, PoolReadError> {
        let inventory = inventory_of(self.world())?;
        let key = mush_inventory::pool_key(endpoint, good)
            .map_err(|error| PoolReadError::Key(error.to_string()))?;
        available_of(
            self.action_state(&inventory.pools, &key),
            self.action_state(&inventory.books, &key),
        )
    }

    fn marker_ring(
        &self,
        endpoint: EndpointRef,
    ) -> Result<Option<mush_inventory::MarkerRing>, String> {
        let inventory = inventory_of(self.world()).map_err(|error| format!("{error:?}"))?;
        self.action_state(&inventory.marker_ring, &endpoint)
            .map(|found| found.cloned())
            .map_err(|error| error.to_string())
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

    fn block_tenure(
        &self,
        key: &crate::surface::claims::BlockKey,
    ) -> Result<Option<crate::surface::claims::BlockTenure>, String> {
        let handles = handles_of(self.world())?;
        self.action_state(&handles.families.claims, key)
            .map(|found| found.cloned().filter(|row| row.key().as_ref() == Some(key)))
            .map_err(|error| error.to_string())
    }

    fn tenure(&self, place: ObjId) -> Result<Option<mush_industry::TenureRecord>, String> {
        let families = industry_of(self.world())?;
        self.action_state(&families.tenure, &place)
            .map(|found| found.cloned().filter(|row| row.place() == place))
            .map_err(|error| error.to_string())
    }

    fn site_charts(
        &self,
        site: &CanonicalId,
        max: usize,
    ) -> Result<(Vec<ChartRecord>, bool), String> {
        let handles = handles_of(self.world())?;
        scan_site_charts(site, max, &mut |after, limit| {
            let page = self
                .scan_action_state(&handles.families.charts, after, limit)
                .map_err(|error| error.to_string())?;
            Ok((
                page.entries()
                    .iter()
                    .map(|entry| entry.value().clone())
                    .collect(),
                page.next_after().cloned(),
            ))
        })
    }

    fn site_record(
        &self,
        site: &CanonicalId,
    ) -> Result<Option<crate::surface::site_record::SiteRecord>, String> {
        let handles = handles_of(self.world())?;
        let key = crate::surface::site_record::SiteKey(site.clone());
        self.action_state(&handles.families.sites, &key)
            .map(|found| found.cloned().filter(|record| record.key() == key))
            .map_err(|error| error.to_string())
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
    // Doc 20 cut SM.9 (the brief's 3.4): push only — the surface tick pushes it through
    // [`push_status`]; a subscribe to it is answered by the gateway's "subscribed" ack.
    drop(reg.panel_feed(TV_STATUS_FEED)?);
    Ok(())
}

/// **One `tv.status` keyframe to the projection's operator** (doc 20 cut SM.9; the brief's 3.4,
/// decision D13): `{ schema: { id: "tv.status", shape: "status", version: 1 }, rig: <the
/// projection> }` — the operator's own `tacsim:tv:<tv>`, whose log holds only their own lines of
/// this excursion (`crate::surface::tick`'s `projection_for`, D16) and which carries no truth
/// (`RunView`, never `Run`). Called by the tick's `write_projection` only when the store wrote a
/// changed record; a projection no act wrote (`operator: None`) pushes nothing. No submission,
/// no write: one event.
///
/// The frame is a **whole-state keyframe** and carries the `retain: "latest"` marker
/// ([`RETAIN_LATEST`](crate::match_::cockpit::RETAIN_LATEST), 16b cut Q.3): the session keeps
/// only the newest `tv.status` in its feed's slot beside the resume ring, so a working pilot's
/// few hundred projection changes an hour never evict the room text, pages and channel lines a
/// reconnect replays, and a resume replays at most one re-pull cue.
pub(crate) fn push_status(ctx: &mut Ctx<'_>, rec: &crate::surface::excursion::TvRec) {
    let Some(operator) = rec.operator else {
        return;
    };
    ctx.out.push(
        mush_core::Audience::Actor(operator),
        OutputEvent::Panel {
            feed: SmolStr::new_static(TV_STATUS_FEED),
            kind: PanelFrameKind::Keyframe,
            data: json!({
                "schema": {
                    "id": TV_STATUS_FEED,
                    "shape": "status",
                    "version": 1,
                    "retain": crate::match_::cockpit::RETAIN_LATEST,
                },
                "rig": rec,
            }),
        },
    );
}

/// **The query word of a family** (doc 20 cut SM.9; the brief's 3.3 item 3): `metals`,
/// `volatiles`, `carbon`, `strategic` — each one `crate::surface::geology::family_of_word`
/// accepts, so a client names a layer the server takes without knowing the aliases.
#[must_use]
pub(crate) fn family_param(family: OreFamily) -> &'static str {
    match family {
        OreFamily::Metals => "metals",
        OreFamily::Volatiles => "volatiles",
        OreFamily::CarbonFeedstock => "carbon",
        OreFamily::StrategicElements => "strategic",
    }
}

/// The four family words in [`OreFamily::ALL`] order (a frame's `families`).
pub(crate) fn family_params() -> Vec<&'static str> {
    OreFamily::ALL.into_iter().map(family_param).collect()
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

/// **A rig's cell on the site `entry`'s ground at `now`**, or `None` when it is not on that ground
/// (doc 32 cut CF.1: [`place_of`]'s rule, for the presence picture the surface tick publishes).
pub(crate) fn ground_cell(
    w: &World,
    record: Option<&RigRecord>,
    tv: ObjId,
    (entry, domain): (&CanonicalId, mush_sim::DomainId),
    now: i64,
) -> Option<(i64, i64)> {
    match place_of(w, record, tv, (entry, domain), None, now) {
        Place::Ground(cell) => Some(cell),
        Place::Bay | Place::Away => None,
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

/// **A claimed block's edge `#`** (doc 20 cut SM.10; the brief's 3.8 item 22): every perimeter
/// site cell of each block of `letters`, less the block's letter corner (its north-west cell,
/// where the letter stands), as `'#'`. Pure: the caller maps the cells into the frame and keeps
/// the letters on top.
fn claim_edges(letters: &[char]) -> Vec<((i64, i64), char)> {
    let side = address::CELLS_PER_BLOCK;
    let mut out = Vec::new();
    for letter in letters {
        let Some((bx, by)) = address::block_of_letter(*letter) else {
            continue;
        };
        let (x0, y0) = (bx * side, by * side);
        let (x1, y1) = (x0 + side - 1, y0 + side - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let edge = x == x0 || x == x1 || y == y0 || y == y1;
                if edge && (x, y) != (x0, y1) {
                    out.push(((x, y), '#'));
                }
            }
        }
    }
    out
}

/// **One claimed block on the map** (doc 20 cut SM.10): its letter, whether an org holds it, the
/// holder's name and whether it is the viewer's. Public facts only (D13).
#[derive(Clone, Debug, PartialEq, Eq)]
struct ClaimMark {
    letter: char,
    held: bool,
    holder: String,
    yours: bool,
}

/// **The site's live claims, A–P, from the block records** (D17): a block whose record carries a
/// live claim of any holder. An unreadable record draws nothing.
fn claimed_blocks(
    w: &World,
    reads: &dyn SurfaceReads,
    entry: &CanonicalId,
    viewer: ObjId,
    now: i64,
) -> Vec<ClaimMark> {
    crate::port::def::BLOCK_LETTERS
        .into_iter()
        .filter_map(|letter| {
            let claim = reads
                .block_tenure(&crate::surface::claims::BlockKey {
                    site: entry.clone(),
                    letter,
                })
                .ok()
                .flatten()?
                .claim
                .filter(|claim| claim.live(now))?;
            Some(ClaimMark {
                letter,
                held: !claim.player_held(),
                holder: crate::port::claims::holder_name(w, &claim.holder),
                yours: claim.holder == mush_org::PrincipalId::Object(viewer),
            })
        })
        .collect()
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

/// **The marks the screens style** (doc 20 cut SM.9; the brief's 3.3 item 5, decision D6):
/// `{ letters: [[fx, fy, "A"], …], hull: [fx, fy] | null, slots: [[fx, fy, n], …] }` — the frame
/// cells where the composed `grid` shows a block letter, the hull's `H` or a rig's slot digit,
/// each listed **only where the grid holds that very glyph at that cell** (R5's rule, as
/// [`list_units`] does). This resolves doc 20 §10's collisions for the screens (letters `H`/`O`
/// against the hull and the crater; slot digits against ore digits) without changing a glyph:
/// line mode keeps doc 20's glyphs, collisions and all.
fn frame_marks(grid: &[Vec<char>], layers: &Overlays) -> Value {
    let glyph_at = |(fx, fy): (usize, usize)| grid.get(fy).and_then(|row| row.get(fx)).copied();
    // The cells each layer above a mark covers: a mark is the cell's top layer or no mark at all
    // (a block letter `H` under the hull is the hull's cell; an `O` letter over a crater is the
    // letter's).
    let rigs: BTreeSet<(usize, usize)> = layers.mine.iter().map(|(_, at, _)| *at).collect();
    let digits: BTreeSet<(usize, usize)> = if layers.mine.len() >= 2 {
        layers
            .mine
            .iter()
            .filter_map(|(_, _, east)| *east)
            .collect()
    } else {
        BTreeSet::new()
    };
    let others: BTreeSet<(usize, usize)> = layers.others.iter().copied().collect();
    let over_hull =
        |at: &(usize, usize)| others.contains(at) || digits.contains(at) || rigs.contains(at);
    let over_letter = |at: &(usize, usize)| {
        layers.claims.iter().any(|(c, _)| c == at)
            || layers.pits.iter().any(|(c, _)| c == at)
            || layers.readings.contains_key(at)
            || layers.hull.as_ref() == Some(at)
            || over_hull(at)
    };
    let letters: Vec<Value> = layers
        .letters
        .iter()
        .filter(|(at, letter)| !over_letter(at) && glyph_at(*at) == Some(*letter))
        .map(|(at, letter)| json!([at.0, at.1, letter.to_string()]))
        .collect();
    let hull = layers
        .hull
        .filter(|at| !over_hull(at) && glyph_at(*at) == Some('H'))
        .map(|at| json!([at.0, at.1]));
    let slots: Vec<Value> = if layers.mine.len() >= 2 {
        layers
            .mine
            .iter()
            .filter_map(|(slot, _, east)| {
                let east = (*east)?;
                let digit = char::from_digit(u32::from(*slot), 10)?;
                (!rigs.contains(&east) && glyph_at(east) == Some(digit))
                    .then(|| json!([east.0, east.1, slot]))
            })
            .collect()
    } else {
        Vec::new()
    };
    json!({ "letters": letters, "hull": hull, "slots": slots })
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
pub(crate) struct Sounded {
    /// The site-local pocket.
    pub(crate) pocket: (i64, i64),
    /// Its R2 classes, one per family.
    pub(crate) classes: [Word; 4],
}

/// One field the viewer has scanned: its field, its readings and its footing.
pub(crate) struct Scanned {
    /// The site-local field.
    pub(crate) field: (i64, i64),
    /// Its R1 readings, one per family.
    pub(crate) readings: [FieldReading; 4],
    /// The footing it was read from.
    pub(crate) footing: Footing,
}

/// **The viewer's own readings** (§5): every pocket their row has sounded and every field it has
/// scanned, rendered from the keyed geology under the row's rule — a row under another rule
/// renders nothing. Each field's ground is read once. **The same gate and the same reads as the
/// render door** (`chart::pocket_reading`, `chart::field_reading`; the review's correctness/F6):
/// a pocket only where `chart::row_reads_pocket` holds, a field only at the footing
/// `chart::row_reads_field` gives, each through `read_r2`/`read_r1` over `chart::field_ground_at`
/// — so the map never shows a reading the door would not.
pub(crate) fn viewer_readings(
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
pub(crate) fn field_family_text(scanned: &Scanned, family: OreFamily) -> Option<String> {
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

/// **The viewer's best reading of one site** (doc 20 cut SM.9; §1 beat 2, §3.1's ground chart),
/// from their own row only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BestReading {
    /// No row, an empty row, a row under another reading rule, or only ice scans (no return).
    None,
    /// Every pocket and field read is barren.
    Barren,
    /// `"{addr} {word} {family label}"`, e.g. "C117 rich metals" — the richest sounded pocket.
    Pocket(String),
    /// A field's family text, e.g. "C-NW fair–rich metals (5–16 pockets)" — only when no
    /// ore-bearing pocket is sounded.
    Field(String),
}

/// **The viewer's best reading of `site`** (doc 20 cut SM.9; the brief's 3.2, decision D14):
/// pure over the row and the keyed ground — [`viewer_readings`]'s very gates and reads, so the
/// ground chart never says a reading the door would not, and no warm site is wanted.
/// Deterministic order: the **sounded pockets** first — the highest word (rich > fair > lean),
/// then the family in [`OreFamily::ALL`] order, then the lowest block letter, then the lowest
/// pocket number; only when no sounded pocket reads ore, the **scanned fields** through
/// [`field_family_text`] — the reading's upper word, then an exact reading over a band at the
/// same word, then the family order, then the field index. A row whose every read is barren is
/// [`BestReading::Barren`]; ice scans read nothing (no return) and count as no read.
#[must_use]
pub(crate) fn best_reading(site: &chart::SiteContext, row: Option<&ChartRecord>) -> BestReading {
    let (sounded, scanned) = viewer_readings(site, row);
    best_of(&sounded, &scanned)
}

/// [`best_reading`]'s order over rendered readings (pure; unit-tested on hand-built rows).
fn best_of(sounded: &[Sounded], scanned: &[Scanned]) -> BestReading {
    // (word, family rank (lower first), letter, number) — the richest wins.
    let mut pocket: Option<(Word, usize, char, u16, String)> = None;
    for s in sounded {
        let (Some(letter), Some(number), Some(addr)) = (
            address::block_letter(s.pocket.0, s.pocket.1),
            address::pocket_number(s.pocket.0, s.pocket.1),
            address::pocket_address(s.pocket.0, s.pocket.1),
        ) else {
            continue;
        };
        for (rank, family) in OreFamily::ALL.into_iter().enumerate() {
            let word = by_family(s.classes, family);
            if word == Word::Barren {
                continue;
            }
            let better = pocket.as_ref().is_none_or(|(w, r, l, n, _)| {
                (
                    word,
                    std::cmp::Reverse(rank),
                    std::cmp::Reverse(letter),
                    std::cmp::Reverse(number),
                ) > (
                    *w,
                    std::cmp::Reverse(*r),
                    std::cmp::Reverse(*l),
                    std::cmp::Reverse(*n),
                )
            });
            if better {
                let text = format!("{addr} {} {}", word.name(), family.label());
                pocket = Some((word, rank, letter, number, text));
            }
        }
    }
    if let Some((_, _, _, _, text)) = pocket {
        return BestReading::Pocket(text);
    }
    // (upper word, exact, family rank, field index) — the richest, exact first.
    let mut field: Option<(Word, bool, usize, u8, String)> = None;
    let mut read_any = !sounded.is_empty();
    for s in scanned {
        if s.footing == Footing::Ice {
            continue;
        }
        read_any = true;
        let index = address::field_index(s.field.0, s.field.1).unwrap_or(u8::MAX);
        for (rank, family) in OreFamily::ALL.into_iter().enumerate() {
            let Some(text) = field_family_text(s, family) else {
                continue;
            };
            let (upper, exact) = match s.readings.get(rank) {
                Some(FieldReading::Exact(word)) => (*word, true),
                Some(FieldReading::Band(_, high)) => (*high, false),
                Some(FieldReading::NoReturn) | None => continue,
            };
            let better = field.as_ref().is_none_or(|(w, e, r, i, _)| {
                (
                    upper,
                    exact,
                    std::cmp::Reverse(rank),
                    std::cmp::Reverse(index),
                ) > (*w, *e, std::cmp::Reverse(*r), std::cmp::Reverse(*i))
            });
            if better {
                field = Some((upper, exact, rank, index, text));
            }
        }
    }
    match field {
        Some((_, _, _, _, text)) => BestReading::Field(text),
        None if read_any => BestReading::Barren,
        None => BestReading::None,
    }
}

/// **A readings row's words** (doc 20 cut SM.4b's join, factored by cut SM.9 so the frame's
/// `readings[].line` and the `Readings, …:` line can never differ): `"{text} here"` at 0 m,
/// else `"{text} {m} m {dir}"`.
fn reading_line(text: &str, m: u64, dir: &str) -> String {
    if m == 0 {
        format!("{text} here")
    } else {
        format!("{text} {m} m {dir}")
    }
}

/// `{m} m {dir}`, or `here`.
fn bearing_words(from: (i64, i64), to: (i64, i64)) -> (u64, String) {
    let m = cells_apart(from, to).saturating_mul(10);
    let dir = Octant::of(to.0.saturating_sub(from.0), to.1.saturating_sub(from.1))
        .map_or_else(String::new, |octant| octant.abbrev().to_owned());
    (m, dir)
}

/// Wrap `line` at [`MAX_LINE_COLUMNS`] on spaces, continuation lines indented two spaces.
pub(crate) fn wrap(line: &str) -> Vec<String> {
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
    // Doc 20 cut SM.12b: the hull the viewer operates — the helm's, else a dispatcher's yard
    // hull (`yard::operating_hull`), so staff possessing the dispatcher read its rigs' frame.
    let helm_hull = crate::port::yard::operating_hull(w, actor).ok_or_else(not_site)?;
    let hull_view = crate::port::ship::hull_view(w, helm_hull).ok_or_else(not_site)?;
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
    // Doc 20 cut SM.10 (the brief's 3.8 item 22): a live claim's block edge, the same for every
    // viewer, never over a block's letter.
    let claimed = claimed_blocks(w, reads, &entry, actor, now);
    let claimed_letters: Vec<char> = claimed.iter().map(|mark| mark.letter).collect();
    let claim_cells: BTreeSet<(usize, usize)> = claim_edges(&claimed_letters)
        .into_iter()
        .filter_map(|(cell, _)| frame_cell(cell))
        .filter(|at| !layers.letters.iter().any(|(letter_at, _)| letter_at == at))
        .collect();
    layers.claims = claim_cells.into_iter().map(|at| (at, '#')).collect();
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
    // Doc 20 cut SM.9 (the brief's 0z ruling 1): the glyphs beneath the viewer's own rigs —
    // the grid composed without their `@` and slot digits — so a client that places a moving
    // rig along its leg restores what it leaves behind (`site.under`).
    let mut bare = grid.clone();
    compose(
        &mut bare,
        &Overlays {
            mine: Vec::new(),
            ..layers.clone()
        },
    );
    compose(&mut grid, &layers);
    let glyph_at = |grid: &[Vec<char>], (fx, fy): (usize, usize)| {
        grid.get(fy).and_then(|row| row.get(fx)).copied()
    };
    let mut under_cells: BTreeSet<(usize, usize)> = BTreeSet::new();
    for (_, at, east) in &layers.mine {
        under_cells.insert(*at);
        if layers.mine.len() >= 2
            && let Some(east) = east
        {
            under_cells.insert(*east);
        }
    }
    let under: Vec<Value> = under_cells
        .iter()
        .filter_map(|at| glyph_at(&bare, *at).map(|glyph| json!([at.0, at.1, glyph.to_string()])))
        .collect();
    let marks = frame_marks(&grid, &layers);
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
        // Doc 20 cut SM.9 (the brief's 3.3 item 6): the row's target, taken here (never parsed
        // out of `text`), and its words as the readings line joins them.
        let line = reading_line(&text, m, &dir);
        readings.push((
            m,
            text.clone(),
            json!({
                "text": text, "cell": [cell.0, cell.1], "m": m, "dir": dir,
                "target": addr, "line": line,
            }),
        ));
    }
    for s in scanned.iter().filter(in_view_field) {
        let Some(text) = field_family_text(s, family) else {
            continue;
        };
        let cell = field_cell(s.field);
        let (m, dir) = bearing_words(reference, cell);
        let target = address::field_name(s.field.0, s.field.1).unwrap_or_default();
        let line = reading_line(&text, m, &dir);
        readings.push((
            m,
            text.clone(),
            json!({
                "text": text, "cell": [cell.0, cell.1], "m": m, "dir": dir,
                "target": target, "line": line,
            }),
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
    let mut moving = false;
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
        // Only an act on this very site carries its leg here — [`place_of`]'s own rule (a leg on
        // another site's ground is in that site's cells), so a client that places the viewer's
        // rigs along their legs (the SM.9 lead's 0z ruling 1) never draws one this map does not.
        let leg = act.filter(|act| act.site == entry).and_then(|act| {
            act.params.leg().map(|(waypoints, _)| {
                moving = true;
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
        let rig_line = format!(
            "Rig {}: {place}, charge {} %, hopper {hopper}{doing}.",
            rig.rig.slot,
            charge.map_or_else(|| "?".to_owned(), |c| c.to_string())
        );
        // Doc 20 cut SM.9 (the brief's 3.3 item 7, lead ruling 4): the run as the operator's
        // own projection shows it — never `RigRecord.run`, which holds the pocket's truth.
        let run = projected_run(w, actor, rig.rig.tv);
        // Doc 20 cut SM.8's queue (its lead note): a rig in the bay with an unload queued says
        // so, in `+tv` status's own words, from the record — never "idle".
        let queued = rig
            .record
            .as_ref()
            .and_then(|record| record.queued.as_ref())
            .map(|_| tv::QUEUED_LINE.trim());
        // Doc 20 cut SM.11 (lead ruling 10): the viewer's own plan as `PlanView` shows it — step
        // words, the step, the deadline and the end's word, never a class or a tonnage.
        let plan = rig
            .record
            .as_ref()
            .and_then(|record| record.plan.as_ref())
            .filter(|plan| plan.author == actor)
            .map_or(Value::Null, plan_value);
        rig_rows.push(json!({
            "slot": rig.rig.slot,
            "mark": rig.rig.mark,
            "cell": cell.map(|(x, y)| json!([x, y])),
            "charge_pct": charge,
            "hopper": hopper,
            "act": act.map(|act| json!({ "kind": act.kind.word(), "due_ic": act.due_ic })),
            "leg": leg,
            "line": rig_line,
            "run": run,
            "queued": queued,
            "plan": plan,
        }));
        rig_lines.push(rig_line);
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
    // Doc 32 cut CF.1 (Q-CF-15): the presence picture, seen from the viewer's nearest rig on the
    // ground, else from the pad.
    let observers: Vec<(u8, (i64, i64))> = on_ground
        .iter()
        .filter_map(|rig| match rig.place {
            Place::Ground(cell) => Some((rig.rig.slot, cell)),
            Place::Bay | Place::Away => None,
        })
        .collect();
    // Rendered from the frame's own `others` (the tick's placement rule at the frame's instant), so
    // the bands never lag the grid; the tick publishes the same rule as its book for CF.2.
    let flights: Vec<crate::surface::presence::Flight> = others
        .iter()
        .map(|(tv, _, cell)| crate::surface::presence::Flight {
            tv: *tv,
            cell: *cell,
            class: crate::surface::presence::hull_class(w, *tv),
        })
        .collect();
    let seen = crate::surface::presence::seen_from(&flights, &mine, &observers, PAD_CELL);
    let presence_line = crate::surface::presence::presence_line(&seen);
    // Doc 32 cut CF.1 (Q-CF-30, Q-CF-28): the field open on the site, named; and what the record
    // holds within sight of the viewer's own rigs on the ground — never the whole record.
    let site_record = reads.site_record(&entry).ok().flatten();
    let field_line = site_record
        .as_ref()
        .and_then(crate::surface::site_record::SiteRecord::open_field)
        .map(|field| {
            crate::surface::site_record::MAP_FIELD_OPEN
                .replace("{ordinal}", &field.ordinal.to_string())
                .replace("{site}", &berth.name)
        });
    let ground_seen: Vec<(u64, String, Value)> = site_record
        .as_ref()
        .map(|record| ground_in_sight(record, &observers))
        .unwrap_or_default();
    let ground_line = (!ground_seen.is_empty()).then(|| {
        format!(
            "On the ground: {}.",
            ground_seen
                .iter()
                .map(|(_, words, _)| words.clone())
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    let hold_room = crate::port::ship::hold_unreserved_kg(
        w,
        &hull_view,
        &crate::port::tv::live_on_hull(w, helm_hull)
            .iter()
            .map(|lease| lease.tv)
            .collect::<Vec<_>>(),
        &read,
        &|tv| reads.rig(tv),
    );
    // The two sentences the screens' Right pane shows whole (the lead's pre-commit round; the
    // judge's correctness/F10): made once here, pushed into `lines` as before, and carried
    // unwrapped as `site.no_readings` / `site.other_rigs` so no client picks them out of
    // `lines` by their opening words.
    let no_readings = readings
        .is_empty()
        .then(|| MAP_NO_READINGS.replace("{family}", family.label()));
    let other_rigs =
        (!other_words.is_empty()).then(|| format!("Other rigs: {}", other_words.join("; ")));
    let mut lines: Vec<String> = Vec::new();
    if let Some(line) = &no_readings {
        lines.push(line.clone());
    } else {
        let rows: Vec<String> = readings
            .iter()
            .map(|(m, text, value)| {
                let dir = value.get("dir").and_then(Value::as_str).unwrap_or("");
                reading_line(text, *m, dir)
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
    if let Some(line) = &other_rigs {
        lines.extend(wrap(line));
    }
    for line in [&presence_line, &ground_line, &field_line]
        .into_iter()
        .flatten()
    {
        lines.extend(wrap(line));
    }
    let hold_line = hold_room.map(|kg| format!("Hold: {} t of room unreserved.", kg / 1_000));
    if let Some(line) = &hold_line {
        lines.push(line.clone());
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
        // Doc 20 cut SM.9 (the brief's 3.3 item 1, lead ruling 2): the pairs name every glyph
        // `SITE_LEGEND` names, so the screens show the legend from the pairs alone.
        ["v", "pit, worked"],
        ["V", "pit, worked out"],
        ["#", "claim"],
    ]);
    // Doc 20 cut SM.9 (the brief's 3.3 item 9, D16): the addressed rig's own projection's log,
    // only this operator's and this excursion's lines.
    let log = addressed
        .map(|rig| viewer_log(w, actor, rig.rig.tv, rig.record.as_ref()))
        .unwrap_or_default();
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
            "hold": hold_room.map(|kg| json!({ "room_t": kg / 1_000, "line": hold_line })),
            "footer": footer,
            "lines": lines,
            // Doc 20 cut SM.9 (the brief's 3.3; vector keys only — no glyph, no line moves).
            "addressed": addressed_slot,
            "family_word": family_param(family),
            "families": family_params(),
            // Frame cell (fx, fy) spans the site cells from (x0 + fx·span, y0 + fy·span), `span`
            // on a side; fy runs north from the window's south edge, as the rows do. `cell_cm` is
            // a site cell's edge, for a leg's centimetres.
            "grid": { "x0": x0, "y0": y0, "span": span, "cell_cm": SITE_CELL_EDGE_CM },
            "marks": marks,
            "under": under,
            // The lead's 0z ruling 1: the frame's instant and the world's rate (IC seconds a
            // real second), so a client places the viewer's own rigs along their legs between
            // pulls; at the frame's own instant that is the cell the rows already show. Only
            // while one of the viewer's rigs has a leg (`null` otherwise), so a frame with
            // nothing moving stays byte-identical as the clock runs (D12's witness).
            "clock": moving.then(|| json!({ "now_ic": now, "rate": w.world_timeline().current().rate })),
            "log": log,
            // The pre-commit round (F10): the no-readings sentence (`MAP_NO_READINGS`, `null`
            // with readings in view) and the other-rigs sentence (`null` with none in view),
            // each whole and unwrapped — `lines` holds the same words wrapped for line mode.
            "no_readings": no_readings,
            "other_rigs": other_rigs,
        },
    });
    let print = site_map_lines(&frame);
    if let Some(block) = frame.get_mut("site").and_then(Value::as_object_mut) {
        block.insert("print".to_owned(), json!(print));
        // Doc 32 cut CF.1: the presence picture, the ground in sight and the open field, each
        // omitted when empty, so a frame with none of them is byte-identical to SM.10's.
        if !seen.is_empty() {
            block.insert(
                "presence".to_owned(),
                Value::Array(
                    seen.iter()
                        .map(crate::surface::presence::Seen::value)
                        .collect(),
                ),
            );
            block.insert("presence_line".to_owned(), json!(presence_line));
        }
        if !ground_seen.is_empty() {
            block.insert(
                "ground".to_owned(),
                Value::Array(ground_seen.into_iter().map(|(_, _, row)| row).collect()),
            );
            block.insert("ground_line".to_owned(), json!(ground_line));
        }
        if let Some(line) = &field_line {
            block.insert("field".to_owned(), json!(line));
        }
        // Doc 20 cut SM.10 (D13): the live claims, omitted when there are none, so a frame with
        // no claim is byte-identical to SM.9's.
        if !claimed.is_empty() {
            let claims: Vec<Value> = claimed
                .iter()
                .map(|mark| {
                    json!({
                        "letter": mark.letter.to_string(),
                        "state": if mark.held { "held" } else { "claimed" },
                        "holder": mark.holder,
                        "yours": mark.yours,
                    })
                })
                .collect();
            block.insert("claims".to_owned(), Value::Array(claims));
        }
    }
    Ok(frame)
}

/// **What the site record holds within sight of the viewer's rigs on the ground** (doc 32 cut
/// CF.1; Q-CF-28: what a party sees, never the whole record): each left unit and object within
/// [`crate::surface::presence::SIGHT_CELLS`] of the nearest observer, as `(metres, words, row)`,
/// nearest first. A viewer with no rig on the ground sees none of it.
fn ground_in_sight(
    record: &crate::surface::site_record::SiteRecord,
    observers: &[(u8, (i64, i64))],
) -> Vec<(u64, String, Value)> {
    use crate::surface::site_record::{GroundKind, object_words};
    let near = |cell: (i64, i64)| {
        observers
            .iter()
            .map(|(_, at)| (cells_apart(*at, cell), *at))
            .min()
            .filter(|(cells, _)| *cells <= crate::surface::presence::SIGHT_CELLS)
    };
    let mut out: Vec<(u64, String, Value)> = Vec::new();
    for unit in &record.units {
        let cell = unit.pose.cell();
        if let Some((_, at)) = near(cell) {
            let (m, dir) = bearing_words(at, cell);
            let what = format!("a {}", unit.condition.word());
            out.push((
                m,
                reading_line(&what, m, &dir),
                json!({ "what": unit.condition.word(), "m": m, "dir": dir }),
            ));
        }
    }
    for object in &record.objects {
        let cell = object.pose.cell();
        if let Some((_, at)) = near(cell) {
            let (m, dir) = bearing_words(at, cell);
            let what = match &object.kind {
                GroundKind::Remains { .. } => "remains".to_owned(),
                GroundKind::Cargo { .. } => object_words(&object.kind),
            };
            out.push((
                m,
                reading_line(&what, m, &dir),
                json!({ "what": object.kind.word(), "m": m, "dir": dir }),
            ));
        }
    }
    out.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    out
}

/// **The run clause of `tv`'s run** from its operator's own projection (doc 20 cut SM.9; the
/// brief's 3.3 item 7, lead ruling 4): `tacsim:tv:<tv>`'s `run`, only when the projection is
/// `actor`'s — `{ addr, ore, k, cycles, due_ic, stop, blind, text }` — else `null`. Never the
/// authoritative `RigRecord.run` (the pocket's class and tonnes).
fn projected_run(w: &World, actor: ObjId, tv: ObjId) -> Value {
    let Some(crate::surface::excursion::TvRecState::Present(rec)) = w
        .module_state::<crate::store::TacsimStore>()
        .and_then(|store| store.tv_rec(tv))
    else {
        return Value::Null;
    };
    projection_run(rec, actor).map_or(Value::Null, |run| run_value(w, run))
}

/// A run view's vector form and its clause ([`MAP_RUN_CLAUSE`], or [`MAP_CORE_CLAUSE`] for a
/// blind core).
fn run_value(w: &World, run: &crate::surface::excursion::RunView) -> Value {
    use crate::port::tv;
    let ore = run
        .good
        .as_ref()
        .map_or_else(String::new, |good| tv::ore_words(good.as_str()));
    let text = run_clause(run, &tv::due_words(w, run.due_ic));
    json!({
        "addr": run.addr,
        "ore": ore,
        "k": run.k,
        "cycles": run.cycles,
        "due_ic": run.due_ic,
        "stop": run.stop,
        "blind": run.blind,
        "text": text,
    })
}

/// **A run view's clause** (pure): [`MAP_CORE_CLAUSE`] for a blind core, else
/// [`MAP_RUN_CLAUSE`] with `RUN_STOP_CLAUSE` once a stop was asked; `{when}` as given.
fn run_clause(run: &crate::surface::excursion::RunView, when: &str) -> String {
    use crate::port::tv;
    let ore = run
        .good
        .as_ref()
        .map_or_else(String::new, |good| tv::ore_words(good.as_str()));
    if run.blind {
        MAP_CORE_CLAUSE
            .replace("{addr}", &run.addr)
            .replace("{ore}", &ore)
            .replace("{when}", when)
    } else {
        MAP_RUN_CLAUSE
            .replace("{addr}", &run.addr)
            .replace("{ore}", &ore)
            .replace("{k}", &run.k.to_string())
            .replace("{cycles}", &run.cycles.to_string())
            .replace("{when}", when)
            .replace("{stop}", if run.stop { tv::RUN_STOP_CLAUSE } else { "" })
    }
}

/// **A plan's vector form** (doc 20 cut SM.11; lead ruling 10): its [`PlanView`] keys and the rig
/// line's clause (`text`) — `Plan {no}: step {k} of {steps}, {words}` live, `Plan {no} ended:
/// {end}` ended.
///
/// [`PlanView`]: crate::surface::excursion::PlanView
fn plan_value(plan: &crate::surface::plan::FillPlan) -> Value {
    let view = crate::surface::plan::plan_view(plan);
    let text = match (view.state.as_str(), view.end.as_deref()) {
        ("ended", end) => format!("Plan {} ended: {}", view.no, end.unwrap_or_default()),
        (_, _) => format!(
            "Plan {}: step {} of {}, {}",
            view.no,
            view.step.min(view.steps),
            view.steps,
            view.next
                .first()
                .map_or(super::tv::STATE_PARKING, String::as_str)
        ),
    };
    json!({
        "no": view.no,
        "state": view.state,
        "step": view.step,
        "steps": view.steps,
        "next": view.next,
        "deadline_ic": view.deadline_ic,
        "end": view.end,
        "text": text,
    })
}

/// **Whether the public pit shows a pocket worked out in one family** (doc 20 cut SM.11; the
/// brief's 3.4 step 5): the family's cursor row for the pocket drawn to its tonnes — [`pit_at`]'s
/// own exhausted rule ([`row_exhausted`]), one family. Public, the same for every asker.
pub(crate) fn pit_exhausted(
    reads: &dyn SurfaceReads,
    entry: &CanonicalId,
    ground: &crate::surface::SiteGround,
    pocket: (i64, i64),
    family: OreFamily,
) -> bool {
    let (Some(letter), Some(number)) = (
        address::block_letter(pocket.0, pocket.1),
        address::pocket_number(pocket.0, pocket.1),
    ) else {
        return false;
    };
    let receipt = crate::surface::opening::receipt(entry, letter, family);
    let Ok(deposit) = mush_industry::derive_system_deposit_id(&receipt) else {
        return false;
    };
    let Ok(Some(cursor)) = reads.cursor(deposit) else {
        return false;
    };
    cursor
        .row(number)
        .is_some_and(|row| row.drawn_t >= 1 && row_exhausted(ground, pocket, family, row.drawn_t))
}

/// **The run a projection shows its viewer** (pure; lead ruling 4): `rec.run` only when the
/// projection is `actor`'s.
fn projection_run(
    rec: &crate::surface::excursion::TvRec,
    actor: ObjId,
) -> Option<&crate::surface::excursion::RunView> {
    match (&rec.run, rec.operator) {
        (Some(run), Some(operator)) if operator == actor => Some(run),
        _ => None,
    }
}

/// **The log lines a viewer may see of their rig** (doc 20 cut SM.9; the brief's 3.3 item 9,
/// D16): the projection's `log`, newest last, at most [`MAP_LOG_LINES`] — only when the
/// projection is `actor`'s, its excursion is the rig's open one, and only the lines stamped at or
/// after that excursion's launch; else none.
fn viewer_log(w: &World, actor: ObjId, tv: ObjId, record: Option<&RigRecord>) -> Vec<Value> {
    let Some(open) = record.and_then(|record| record.excursion.as_ref()) else {
        return Vec::new();
    };
    let Some(crate::surface::excursion::TvRecState::Present(rec)) = w
        .module_state::<crate::store::TacsimStore>()
        .and_then(|store| store.tv_rec(tv))
    else {
        return Vec::new();
    };
    log_for(rec, actor, open.id)
}

/// [`viewer_log`]'s rule over one projection (pure).
fn log_for(
    rec: &crate::surface::excursion::TvRec,
    actor: ObjId,
    open: crate::surface::excursion::ExcursionId,
) -> Vec<Value> {
    if rec.operator != Some(actor) || rec.excursion != Some(open) {
        return Vec::new();
    }
    let lines: Vec<&crate::surface::excursion::TvLogLine> = rec
        .log
        .iter()
        .filter(|line| line.at_ic >= open.launch_ic)
        .collect();
    let skip = lines.len().saturating_sub(MAP_LOG_LINES);
    lines
        .into_iter()
        .skip(skip)
        .map(|line| json!({ "at_ic": line.at_ic, "text": line.text }))
        .collect()
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
        // Doc 20 cut SM.10: the edges of a claimed block — its perimeter less its letter's corner.
        assert!(claim_edges(&[]).is_empty(), "no claim, no edge");
        let a = claim_edges(&['A']);
        assert_eq!(
            a.len(),
            64 * 4 - 4 - 1,
            "the perimeter less the letter's corner"
        );
        assert!(a.iter().all(|(_, glyph)| *glyph == '#'));
        assert!(
            !a.iter().any(|(cell, _)| *cell == (0, 255)),
            "the letter's corner stays"
        );
        assert!(
            a.iter()
                .all(|((x, y), _)| *x == 0 || *x == 63 || *y == 192 || *y == 255)
        );
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

    /// **The marks list only glyphs the grid shows** (doc 20 cut SM.9; the brief's 3.3 item 5,
    /// decision D6): a block letter `H` under the hull is no letter mark (the grid shows the
    /// hull's `H`, which is the hull mark); a letter under a reading is none; a slot digit under
    /// the other rig's `@` is not listed; one rig alone has no slot marks.
    #[test]
    fn sm9_marks_list_only_glyphs_the_grid_shows() {
        let mut grid = vec![vec![','; 8]; 2];
        let layers = Overlays {
            letters: vec![((0, 0), 'H'), ((2, 0), 'C'), ((4, 0), 'D')],
            readings: [((4, 0), '2')].into_iter().collect(),
            hull: Some((0, 0)),
            // Rig 1 at (5, 0) with its digit at (6, 0); rig 2 at (6, 0) — over rig 1's digit —
            // with its digit at (7, 0).
            mine: vec![(1, (5, 0), Some((6, 0))), (2, (6, 0), Some((7, 0)))],
            ..Overlays::default()
        };
        compose(&mut grid, &layers);
        assert_eq!(
            grid.first()
                .map(|r| r.iter().collect::<String>())
                .as_deref(),
            Some("H,C,2@@2")
        );
        let marks = frame_marks(&grid, &layers);
        assert_eq!(marks["letters"], json!([[2, 0, "C"]]), "only the C shows");
        assert_eq!(marks["hull"], json!([0, 0]));
        assert_eq!(
            marks["slots"],
            json!([[7, 0, 2]]),
            "rig 1's digit is under rig 2's @"
        );
        // One rig on the ground: no digits, no slot marks.
        let mut lone_grid = vec![vec![','; 3]; 1];
        let lone = Overlays {
            mine: vec![(1, (0, 0), Some((1, 0)))],
            ..Overlays::default()
        };
        compose(&mut lone_grid, &lone);
        assert_eq!(frame_marks(&lone_grid, &lone)["slots"], json!([]));
        assert_eq!(frame_marks(&lone_grid, &lone)["hull"], Value::Null);
    }

    /// A run view as the projection carries it (doc 20 cut SM.9).
    fn run_view(blind: bool, stop: bool) -> crate::surface::excursion::RunView {
        crate::surface::excursion::RunView {
            ordinal: 4,
            k: 2,
            cycles: 4,
            due_ic: 9_000,
            addr: "C117".to_owned(),
            good: CanonicalId::new("metal_ore").ok(),
            stop,
            blind,
        }
    }

    /// **The run text reads the projection** (doc 20 cut SM.9; the brief's 3.3 item 7, lead
    /// ruling 4): the operator's own projection's run, in `MAP_RUN_CLAUSE`'s words (the stop
    /// clause once asked), a blind core in `MAP_CORE_CLAUSE`'s; another operator's projection
    /// gives no run.
    #[test]
    fn sm9_the_run_text_reads_the_projection() {
        let mut rec = crate::surface::excursion::TvRec::new(ObjId(900), 1);
        rec.operator = Some(ObjId(41));
        rec.run = Some(Box::new(run_view(false, false)));
        let run = projection_run(&rec, ObjId(41)).unwrap_or_else(|| panic!("the operator's run"));
        assert_eq!(
            run_clause(run, "soon"),
            "drilling C117 for metal ore, cycle 2 of 4, this cycle done soon"
        );
        assert_eq!(
            run_clause(&run_view(false, true), "soon"),
            "drilling C117 for metal ore, cycle 2 of 4, this cycle done soon; it stops after this cycle"
        );
        assert_eq!(
            run_clause(&run_view(true, false), "soon"),
            "coring C117 for metal ore, done soon"
        );
        assert!(
            projection_run(&rec, ObjId(42)).is_none(),
            "another's projection"
        );
        rec.operator = None;
        assert!(projection_run(&rec, ObjId(41)).is_none());
        // The view carries no truth: its wire keys are the operator's own choice.
        let wire = serde_json::to_value(run_view(false, true)).unwrap_or_else(|e| panic!("{e}"));
        let keys: Vec<&str> = wire
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        assert_eq!(
            keys,
            ["addr", "cycles", "due_ic", "good", "k", "ordinal", "stop"]
        );
    }

    /// **The family words round-trip** (doc 20 cut SM.9; the brief's 3.3 item 3): every
    /// parameter word is one `family_of_word` accepts, in `OreFamily::ALL` order.
    #[test]
    fn sm9_family_params_round_trip() {
        for family in OreFamily::ALL {
            assert_eq!(
                crate::surface::geology::family_of_word(family_param(family)),
                Some(family)
            );
        }
        assert_eq!(
            family_params(),
            ["metals", "volatiles", "carbon", "strategic"]
        );
    }

    /// **The log is the viewer's excursion's only** (doc 20 cut SM.9; the brief's 3.3 item 9,
    /// D16): another operator's projection, or another excursion's, gives no line; a line
    /// stamped before the excursion's launch is dropped; at most `MAP_LOG_LINES`, newest last.
    #[test]
    fn sm9_the_log_is_the_viewers_excursions_only() {
        use crate::surface::excursion::{ExcursionId, TvRec};
        let open = ExcursionId {
            tv: ObjId(900),
            launch_ic: 1_000,
        };
        let mut rec = TvRec::new(ObjId(900), 1);
        rec.operator = Some(ObjId(41));
        rec.excursion = Some(open);
        rec.push_log(900, "before the launch");
        for i in 0..8 {
            rec.push_log(1_000 + i, &format!("line {i}"));
        }
        let texts = |lines: Vec<Value>| -> Vec<String> {
            lines
                .iter()
                .filter_map(|l| l.get("text").and_then(Value::as_str).map(str::to_owned))
                .collect()
        };
        assert_eq!(
            texts(log_for(&rec, ObjId(41), open)),
            ["line 2", "line 3", "line 4", "line 5", "line 6", "line 7"]
        );
        assert!(
            log_for(&rec, ObjId(42), open).is_empty(),
            "another operator's"
        );
        let later = ExcursionId {
            tv: ObjId(900),
            launch_ic: 5_000,
        };
        assert!(
            log_for(&rec, ObjId(41), later).is_empty(),
            "another excursion's"
        );
        let mut short = TvRec::new(ObjId(900), 1);
        short.operator = Some(ObjId(41));
        short.excursion = Some(open);
        short.push_log(999, "before");
        short.push_log(1_000, "at the launch");
        assert_eq!(texts(log_for(&short, ObjId(41), open)), ["at the launch"]);
    }

    /// **A pit worked out in one family reads `V` whatever else it holds** (doc 20 cut SM.9;
    /// the brief's 3.10b, decision D9): the pit rule folds only the DRAWN rows, so a pocket
    /// exhausted in one family and never drawn in another is `V` on every map — an undrawn family
    /// never shows, because a `v` would tell a rival that a second family is there; a drawn row
    /// not exhausted keeps it `v`.
    #[test]
    fn sm9_a_pit_worked_out_in_one_family_reads_v_whatever_else_it_holds() {
        // One drawn row, exhausted: `V` (the other family has no drawn row, so no fold).
        assert_eq!(fold_pit(None, true), Some('V'));
        // Two drawn rows: `V` only when both are exhausted.
        assert_eq!(fold_pit(fold_pit(None, true), true), Some('V'));
        assert_eq!(fold_pit(fold_pit(None, true), false), Some('v'));
        assert_eq!(fold_pit(fold_pit(None, false), true), Some('v'));
        assert_eq!(fold_pit(None, false), Some('v'));
    }

    /// **The best reading orders pockets before fields** (doc 20 cut SM.9; the brief's 3.2):
    /// the richest sounded pocket wins (then the family order, the block letter, the pocket
    /// number); a field speaks only when no sounded pocket reads ore — the upper word first, an
    /// exact reading over a band; a row read and all barren is `Barren`; ice scans and nothing
    /// read is `None`.
    #[test]
    fn sm9_best_reading_orders_pockets_before_fields() {
        let w = |classes: [Word; 4]| classes;
        let pocket = |p: (i64, i64), classes: [Word; 4]| Sounded { pocket: p, classes };
        let field = |f: (i64, i64), readings: [FieldReading; 4], footing: Footing| Scanned {
            field: f,
            readings,
            footing,
        };
        let barren = [Word::Barren; 4];
        let exact = |word: Word| FieldReading::Exact(word);
        let nothing = [exact(Word::Barren); 4];
        // C117 (36, 56) fair metals; C118 (37, 56) rich volatiles; a fair carbon one beside.
        let c117 = pocket(
            (36, 56),
            w([Word::Fair, Word::Barren, Word::Barren, Word::Barren]),
        );
        let c118 = pocket(
            (37, 56),
            w([Word::Barren, Word::Rich, Word::Barren, Word::Barren]),
        );
        let fields = [field(
            (4, 7),
            [
                FieldReading::Band(Word::Fair, Word::Rich),
                exact(Word::Barren),
                exact(Word::Barren),
                exact(Word::Barren),
            ],
            Footing::Regolith,
        )];
        assert_eq!(
            best_of(&[c117, c118], &fields),
            BestReading::Pocket("C118 rich volatiles".to_owned()),
            "the richest pocket, before any field"
        );
        // Equal words: the earlier family, then the lower pocket number.
        let a = pocket(
            (36, 56),
            w([Word::Fair, Word::Fair, Word::Barren, Word::Barren]),
        );
        let b = pocket(
            (37, 56),
            w([Word::Fair, Word::Barren, Word::Barren, Word::Barren]),
        );
        assert_eq!(
            best_of(&[b, a], &[]),
            BestReading::Pocket("C117 fair metals".to_owned())
        );
        // No ore-bearing pocket: the field speaks; an exact rich over a fair–rich band.
        let dud = pocket((36, 56), barren);
        let rock = field(
            (5, 7),
            [
                exact(Word::Barren),
                exact(Word::Rich),
                exact(Word::Barren),
                exact(Word::Barren),
            ],
            Footing::Rock,
        );
        let best = best_of(
            &[dud],
            &[
                field(
                    (4, 7),
                    [
                        FieldReading::Band(Word::Fair, Word::Rich),
                        exact(Word::Barren),
                        exact(Word::Barren),
                        exact(Word::Barren),
                    ],
                    Footing::Regolith,
                ),
                rock,
            ],
        );
        match best {
            BestReading::Field(text) => assert!(text.contains("rich volatiles"), "{text}"),
            other => panic!("a field: {other:?}"),
        }
        // Read, all barren: Barren. Ice only, or nothing: None.
        assert_eq!(
            best_of(
                &[pocket((36, 56), barren)],
                &[field((4, 7), nothing, Footing::Rock)]
            ),
            BestReading::Barren
        );
        assert_eq!(
            best_of(
                &[],
                &[field((4, 7), [FieldReading::NoReturn; 4], Footing::Ice)]
            ),
            BestReading::None
        );
        assert_eq!(best_of(&[], &[]), BestReading::None);
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
