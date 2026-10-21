//! **Surface mining's ground** (doc 20, the quarry loop; this module is cut SM.1: §3.2, §5,
//! D-20-2, D-20-4, Q-20-10, Q-20-12, §16).
//!
//! The world gets a ground to mine before it gets a site, a vehicle or a player verb:
//!
//! - [`geology`](crate::surface::geology) — the **keyed geology function** (D-20-4): HMAC-SHA256 under the fourth durable
//!   key family over a body-fixed pocket lattice, generated top-down (province → field →
//!   pocket) so every reading nests; one word set (barren, lean, fair, rich); integers only, no
//!   RNG;
//! - [`reading`](crate::surface::reading) — **the reading rule, version 1** (§5): R0, R1 (sharpened by footing) and R2
//!   as pure quantisations of that function;
//! - [`key`](crate::surface::key) — the key's **boot, its fingerprint pin** (`tacsim:geology`) and the RAM-only
//!   disabled state every later consumer refuses on ([`live`](crate::surface::key::live)); the readiness report's
//!   `geology:` line (reported, never gated);
//! - [`ground`](crate::surface::ground) — a site's **geology table** and terrain composite, held in an in-RAM LRU of
//!   sixteen, with a synchronous build door;
//! - [`commands`](crate::surface::commands) — `@geology`, staff truth over the body-fixed lattice.
//!
//! **Doc 20 cut SM.2** installs the lane's Ceres and Vesta tables at boot
//! ([`install_lane_geology`](crate::surface::install_lane_geology), over
//! [`lane_geologies`](crate::surface::lane_geologies) of the lane's `BodyDef`s) and, since cut
//! SM.5 (lead ruling 9), wherever a body entry is applied (the definition applier, over
//! [`body_geology_of`](crate::surface::body_geology_of)), whatever else the version changes;
//! SM.2 adds the
//! sites and their block shifts (`crate::port::site`); `@geology <site> <addr>` moves to SM.4b
//! with the address parse; SM.6 reserves from the geology table; SM.7 mining.
//!
//! **Doc 20 cut SM.3b** gives the terrain vehicle its clock:
//!
//! - [`excursion`](crate::surface::excursion) — the typed `tacsim.excursion` family (one
//!   [`RigRecord`](crate::surface::excursion::RigRecord) per rig: the act in flight, the
//!   charge it reserved, the open excursion, the lazy charge) and the normal-lane projection
//!   `tacsim:tv:<tv>` ([`TvRec`](crate::surface::excursion::TvRec)), with the pure charge,
//!   reserve, cap and ramp arithmetic;
//! - [`act`](crate::surface::act) — the acts' world plans and the two authoritative
//!   providers, `tacsim_surface_act` (every act's start) and `tacsim_surface_done` (every
//!   act's finish);
//! - [`tick`](crate::surface::tick) — `tacsim.surface`, the Coarse 1 s pull tick that writes
//!   each act's placement and projection at its due instant and submits its finish as the
//!   operator.
//!
//! **Doc 20 cut SM.4a** gives the operator the knowing half (§5, D-20-8):
//!
//! - [`address`](crate::surface::address) — the site's names (block letters, pocket addresses,
//!   field names) and the instruments' exact per-axis reach;
//! - [`chart`](crate::surface::chart) — the `tacsim.chart` family of coverage and provenance
//!   per (character, site), the pure rules a scan and a sounding share, the render re-derived
//!   from the keyed geology through reading rule v1, the coverage over the composite, the
//!   non-building peek and `+tv/forget`'s own action;
//! - the scan and the sounding are two new kinds on SM.3b's one start and one finish
//!   (`crate::surface::act`), and the tick tells their readings and the site-wide heard line.
//!
//! **The site ground is warmed off the act's path** (lead ruling 6, D11): the command, the
//! start's offer and `+tv` status only peek the cache
//! ([`ground::site_ground_cached`](crate::surface::ground::site_ground_cached)); a cold peek
//! answers "still being mapped" and asks for the site (`want_site_ground`), and so do a
//! hull's landing on a site's pad and the boot (every site with a programme hull docked); the
//! `tacsim.surface` tick builds at most one wanted site a fire (`warm_one`). With the layer
//! dark nothing is wanted or warmed; a site whose build fails is remembered for the boot and
//! reads dark, never "still being mapped" (D25). A true off-loop runner is SM.12's residual.
//!
//! The slot ([`GeologyState`](crate::surface::GeologyState)) is RAM only and never persisted: the key, its status, the bodies'
//! tables, the site cache and the warm requests. Every writer reaches it through
//! `World::module_state_mut` from inside this crate (the `mush_tactics::ground::configure`
//! pattern), so the state-slot owner census needs no row — save the once-a-boot dark set, which
//! a read-only peek also marks through its own lock (`fail_site`; cut SM.4a's pre-commit round,
//! item 5).

pub mod act;
pub mod address;
pub mod chart;
pub mod commands;
pub mod excursion;
pub mod geology;
pub mod ground;
pub mod key;
pub mod reading;
pub mod tick;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::{CanonicalId, Ctx, GeologyMac, World};

pub use geology::{
    BodyGeology, FieldGround, GeologyError, GeologyTable, GeologyTableWire, Lattice, OreFamily,
    PocketRow, ProvinceGround, Word,
};
pub use ground::{SiteGeology, SiteGeologySpec, SiteGround, SiteGroundCache, SiteGroundKey};
pub use key::{
    GEOLOGY_PIN_KEY, GeologyBoot, GeologyDisabled, GeologyPinRecord, GeologyStatus, live,
};
pub use reading::{FieldReading, Footing, READING_RULE_VERSION};

pub use act::{
    Refusal, RigAt, SURFACE_ACT_ACTION, SURFACE_ACT_OUTCOME, SURFACE_DONE_ACTION,
    SURFACE_DONE_OUTCOME, SURFACE_KEY_PREFIX, Where, act_key, done_key,
};
pub use address::Octant;
pub use chart::{
    CHART_FAMILY, CHART_FORGET_ACTION, CHART_FORGET_OUTCOME, CHART_SCHEMA, ChartKey,
    ChartProvenance, ChartRecord, FieldRow, HEARD_EVERY_IC, Instrument,
    MAX_CHART_FIELD_ROWS_PER_BODY, MAX_CHART_POCKET_BLOCKS_PER_BODY, MAX_CHART_PROVENANCE,
    MAX_CHART_RECORD_BYTES, MAX_CHART_SITES_PER_BODY, PIT_SEEN, PocketBlock, TIER_MASK, TIER_R2,
    Via, better_footing, body_admits, cell_tier, field_reading, field_row_text, footing_at,
    forget_row, heard_line_due, pocket_reading, pocket_row_text, repeat_refused, scan_adds,
    seen_fields, sound_adds, write_scan, write_sound,
};
pub use excursion::{
    Act, ActKind, ActParams, ActResult, ActView, CapRefused, Charge, ChargeAfter, DUMP_S,
    EXCURSION_FAMILY, EXCURSION_SCHEMA, Excursion, ExcursionCounts, ExcursionId, FrozenLeg,
    IC_HOUR, LAUNCH_S, LegWaypoint, MAX_DUMP_GOODS, MAX_EXCURSIONS_PER_OPERATOR,
    MAX_EXCURSIONS_PER_SITE, MAX_EXCURSIONS_WORLD, MAX_LEG_WAYPOINTS, MAX_READ_RADIUS_CELLS,
    MAX_SCAN_FIELDS, MAX_TV_LOG_BYTES, MAX_TV_LOG_LINES, MAX_TV_REC_BYTES, Outcome, PAD_CENTRE_CM,
    RECOVER_S, ReadSpan, RefusedCode, ReserveShort, RigKey, RigRecord, TV_REC_PREFIX,
    TV_REC_SCHEMA, TvLogLine, TvPose, TvRec, TvRecState, TvState, cap_refusal, cells_apart,
    charge_at, charge_for_act, charge_shown, needs_battery, ramp_pose, reserve_check,
    reserved_total, settle_free, settle_spent_out_of_bay, site_cell, tv_rec_key, tv_rec_key_owner,
};
pub use ground::site_ground_cached;
pub use tick::{
    DONE_BACKOFF_FIRES, MAX_DONE_ATTEMPTS, MAX_EXCURSIONS_VISITED_PER_FIRE,
    MAX_EXCURSIONS_WORKED_PER_FIRE, MAX_RIG_RECORDS, SCAN_PAGE, SURFACE_TICK, SURFACE_TICK_PERIOD,
    SurfaceFireStats, SurfaceTickState, last_surface_fire,
};

/// The most body tables one world holds.
pub const MAX_BODY_GEOLOGIES: usize = 32;

/// RAM only, never persisted: the handed-in key until boot, the status decided at boot, the
/// keyed HMAC state while enabled, the bodies' tables and the site cache.
#[derive(Clone, Debug)]
pub struct GeologyState {
    /// What the host handed in; taken by the boot pass.
    boot: Option<GeologyBoot>,
    /// Decided once, by the boot pass.
    status: GeologyStatus,
    /// The keyed state while enabled.
    mac: Option<GeologyMac>,
    /// Each body's ground.
    tables: BTreeMap<CanonicalId, Arc<BodyGeology>>,
    /// The site ground LRU.
    sites: SiteGroundCache,
    /// **Doc 20 cut SM.4a** (lead ruling 6): the site entries asked for a warm — at landing, at
    /// boot and by a cold peek — popped one a fire by [`warm_one`]. Bounded by the lane's sites.
    warm: BTreeSet<CanonicalId>,
    /// **Doc 20 cut SM.4a** (D25): the sites whose build failed this boot — never wanted or
    /// warned again, and read dark (never "still being mapped") until the next boot. Since the
    /// cut's pre-commit round (item 5) a peek whose **capture** fails joins it too, from a
    /// `&World` reader ([`fail_site`]), so a dark site logs once a boot, never once a peek.
    warm_failed: FailedSites,
}

/// **The once-a-boot dark set** (doc 20 cut SM.4a, D25; the pre-commit round's item 5): behind
/// a lock so the read-only peek (`chart::peek_site`, which the start's offer projection reaches
/// through `&World`) can record a capture failure. Only the world thread takes it, so it is
/// never contended; a poisoned lock is read through (nothing in it can be half-written). Its
/// `Clone` copies the set.
#[derive(Debug, Default)]
struct FailedSites(std::sync::Mutex<BTreeSet<CanonicalId>>);

impl FailedSites {
    /// The set, locked.
    fn set(&self) -> std::sync::MutexGuard<'_, BTreeSet<CanonicalId>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Clone for FailedSites {
    fn clone(&self) -> Self {
        FailedSites(std::sync::Mutex::new(self.set().clone()))
    }
}

impl Default for GeologyState {
    fn default() -> Self {
        GeologyState {
            boot: None,
            status: GeologyStatus::Disabled(GeologyDisabled::NotBooted),
            mac: None,
            tables: BTreeMap::new(),
            sites: SiteGroundCache::default(),
            warm: BTreeSet::new(),
            warm_failed: FailedSites::default(),
        }
    }
}

impl GeologyState {
    /// The site ground cache.
    #[must_use]
    pub fn sites(&self) -> &SiteGroundCache {
        &self.sites
    }

    /// The site entries waiting for a warm (doc 20 cut SM.4a), in id order.
    #[must_use]
    pub fn wanted(&self) -> Vec<CanonicalId> {
        self.warm.iter().cloned().collect()
    }

    /// The site entries whose build failed this boot (doc 20 cut SM.4a, D25), in id order.
    #[must_use]
    pub fn failed(&self) -> Vec<CanonicalId> {
        self.warm_failed.set().iter().cloned().collect()
    }
}

/// **Ask for a site's ground to be warmed** (doc 20 cut SM.4a; lead ruling 6, D25): RAM only,
/// a no-op unless the layer is live, the entry is a lane site whose body has a table
/// (`chart::site_context`), and its build has not failed this boot — so an unkeyed world never
/// wants anything. The `tacsim.surface` tick builds it ([`warm_one`]), never an act's path.
pub(crate) fn want_site_ground(w: &mut World, entry: &CanonicalId) {
    if chart::site_context(w, entry).is_err() || warm_failed(w, entry) {
        return;
    }
    if let Some(state) = w.module_state_mut::<GeologyState>() {
        state.warm.insert(entry.clone());
    }
}

/// Whether `entry`'s build failed this boot (D25).
#[must_use]
pub(crate) fn warm_failed(w: &World, entry: &CanonicalId) -> bool {
    w.module_state::<GeologyState>()
        .is_some_and(|state| state.warm_failed.set().contains(entry))
}

/// **Mark `entry` dark for the boot** (D25; doc 20 cut SM.4a's pre-commit round, item 5) from a
/// read-only path: true when it was not already, so the caller warns exactly once a boot. A dark
/// site is never wanted again ([`want_site_ground`]), skipped by [`warm_one`] if it was wanted
/// already, and read `Dark` by every later peek before any capture.
pub(crate) fn fail_site(w: &World, entry: &CanonicalId) -> bool {
    w.module_state::<GeologyState>()
        .is_some_and(|state| state.warm_failed.set().insert(entry.clone()))
}

/// **Warm one wanted site** (doc 20 cut SM.4a; lead ruling 6): called at the top of each
/// `tacsim.surface` fire. Returns at once with the layer dark; else pops the first wanted entry
/// (id order) and builds its ground through [`ground::site_ground_sync`] — one build a fire,
/// on the tick, never on an act's path — `tracing::debug!`ing its milliseconds. A failed build
/// `tracing::warn!`s once, joins `warm_failed` and is dropped: it is never wanted or warned
/// again this boot (D25).
pub(crate) fn warm_one(ctx: &mut Ctx<'_>) {
    if live(ctx.w).is_err() {
        return;
    }
    let Some(entry) = ctx
        .w
        .module_state_mut::<GeologyState>()
        .and_then(|state| state.warm.pop_first())
    else {
        return;
    };
    // A site a peek has marked dark since it was wanted (the pre-commit round's item 5) is
    // not built, and so not warned a second time.
    if warm_failed(ctx.w, &entry) {
        return;
    }
    let started = std::time::Instant::now();
    let built = chart::site_context(ctx.w, &entry)
        .map(|site| (site.frame.clone(), site.spec.clone()))
        .map_err(|dark| dark.reason())
        .and_then(|(frame, spec)| ground::site_ground_sync(ctx.w, &frame, &spec));
    match built {
        Ok(_) => tracing::debug!(
            site = %entry,
            ms = started.elapsed().as_millis(),
            "tacsim: a site's ground was warmed"
        ),
        Err(error) => {
            tracing::warn!(
                site = %entry,
                %error,
                "tacsim: a site's ground does not build; it reads dark until the next boot"
            );
            let _ = fail_site(ctx.w, &entry);
        }
    }
}

/// **The composition root's door** (the SM.1 brief's lead ruling 3): the host hands the booted
/// key in after composition and before the module boot pass — `mush_server::boot::build_engine`,
/// and a test's own engine. Returns false (and stores nothing) when no tacsim slot is composed,
/// and — with a `tracing::warn!` — once the boot pass has run (the status is no longer
/// `NotBooted`): a key handed in after boot would sit beside a status nothing recomputes, so it
/// is refused, never silently ignored. A second call before boot replaces the first.
pub fn install_geology(w: &mut World, boot: GeologyBoot) -> bool {
    let Some(state) = w.module_state_mut::<GeologyState>() else {
        return false;
    };
    if state.status != GeologyStatus::Disabled(GeologyDisabled::NotBooted) {
        tracing::warn!(
            "tacsim: a geology key was handed in after the boot pass decided the layer's status; \
             refused"
        );
        return false;
    }
    state.boot = Some(boot);
    true
}

/// A body's ground for this world. Called by [`install_lane_geology`] (doc 20 cut SM.2: the
/// lane's `BodyDef` tables, at boot and, since cut SM.5, wherever a body entry is applied) and
/// by tests. A body already installed is
/// replaced, and **its cached sites are dropped**: the site cache's key (`ground::site_revision`) carries the captured input and
/// the spec, not the body's table or radius, so this writer — the only one — keeps a replaced
/// table from serving the old ground.
///
/// # Errors
///
/// No geology slot is composed, or past [`MAX_BODY_GEOLOGIES`] bodies.
pub fn install_body_geology(w: &mut World, geology: BodyGeology) -> Result<(), String> {
    let state = w
        .module_state_mut::<GeologyState>()
        .ok_or_else(|| "no geology state is composed".to_owned())?;
    if !state.tables.contains_key(&geology.body) && state.tables.len() >= MAX_BODY_GEOLOGIES {
        return Err(format!(
            "a world holds at most {MAX_BODY_GEOLOGIES} body geology tables"
        ));
    }
    let _ = state.sites.forget_body(&geology.body);
    state.tables.insert(geology.body.clone(), Arc::new(geology));
    Ok(())
}

/// A body's ground, if its table is installed.
#[must_use]
pub fn body_geology(w: &World, body: &CanonicalId) -> Option<Arc<BodyGeology>> {
    w.module_state::<GeologyState>()?.tables.get(body).cloned()
}

/// **The lane's body tables** (doc 20 cut SM.2; the SM.2 brief's lead ruling 6, step one):
/// every body of `pack` that authors both `radius_km` and `geology`, built as a
/// [`BodyGeology`], in id order. Pure, so it runs while the store that holds `pack` is
/// borrowed; [`install_lane_geology`] installs the result after the borrow ends.
#[must_use]
pub fn lane_geologies(pack: &crate::pack::RulesPack) -> Vec<Result<BodyGeology, String>> {
    pack.entries()
        .iter()
        .filter_map(|(id, entry)| match entry {
            crate::pack::PackEntry::Body(def) => body_geology_of(id, def),
            _ => None,
        })
        .collect()
}

/// **One body entry's table** (doc 20 cut SM.2; cut SM.5's lead ruling 9): `None` when
/// the body authors no `radius_km` or no `geology`, else the built [`BodyGeology`] (or
/// why it does not build). [`lane_geologies`] at boot and the definition applier
/// (`crate::content`, wherever a body entry is created or updated) both read it.
#[must_use]
pub fn body_geology_of(
    id: &CanonicalId,
    def: &crate::pack::BodyDef,
) -> Option<Result<BodyGeology, String>> {
    let (Some(radius), Some(table)) = (def.radius_km, def.geology.as_ref()) else {
        return None;
    };
    Some(
        BodyGeology::new(id.clone(), radius, table.clone())
            .map_err(|error| format!("{id}: {error}")),
    )
}

/// **Install what [`lane_geologies`] built** (doc 20 cut SM.2, step two) through
/// [`install_body_geology`]: an `Err` is warned and skipped, and **a body whose installed
/// table is already equal is skipped**, so its site cache survives every re-applied body
/// entry and every boot. RAM only; nothing is written. Returns how many it installed.
pub fn install_lane_geology(w: &mut World, bodies: Vec<Result<BodyGeology, String>>) -> usize {
    let mut installed = 0_usize;
    for body in bodies {
        let body = match body {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(%error, "tacsim: a lane body's geology table does not build; skipped");
                continue;
            }
        };
        if body_geology(w, &body.body).is_some_and(|standing| *standing == body) {
            continue;
        }
        let id = body.body.clone();
        match install_body_geology(w, body) {
            Ok(()) => installed = installed.saturating_add(1),
            Err(error) => {
                tracing::warn!(body = %id, %error, "tacsim: a lane body's geology table was not installed");
            }
        }
    }
    installed
}

/// The bodies with a table installed, in id order (for readiness and tests).
#[must_use]
pub fn installed_bodies(w: &World) -> Vec<CanonicalId> {
    w.module_state::<GeologyState>()
        .map(|state| state.tables.keys().cloned().collect())
        .unwrap_or_default()
}
