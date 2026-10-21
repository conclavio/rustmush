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
//! **Doc 20 cut SM.6** makes a site's ore conserved industry quantity before anyone mines it
//! (§3.4, D-20-5):
//!
//! - [`opening`](crate::surface::opening) — the site's opening plan, one deposit per (block,
//!   family) with ore, its gross from the keyed site geology and the worked share per pocket,
//!   and the RAM slot the Site readiness line reads; the site clerk acts on it on the port
//!   tick (`crate::port::site::site_step`);
//! - [`cursor`](crate::surface::cursor) — `tacsim.pockets`, the pocket cursor sidecar per
//!   deposit, what industry's sweep reconciles each pool against and the lock's one home;
//! - [`tenure`](crate::surface::tenure) — §9's numbers, which SM.10 reads.
//!
//! **Doc 20 cut SM.4b** gives the operator the moving half (§1 beats 7 and 9, §8, §10, D-20-13):
//!
//! - [`drive`](crate::surface::drive) — the pure planner: the targets, ONE A* a command over the
//!   site's composite at a whole site's budget, the frozen leg (an integer Douglas–Peucker at a
//!   display tolerance of 1–8 cells, at most 32 waypoints, time and charge the exact path's), the
//!   rates (the unit's tracked ceiling, the battery's per-cell draw: a regolith kilometre is 500
//!   IC s and 2,250 cu) and the **home field** — every cell's exact least cost back to beside the
//!   ramp, built once per (site terrain, ramp cell) on the surface tick, so every act from the
//!   ground reserves its real trip home and a drive to a stand with no way home is refused;
//! - a drive is a fourth kind on SM.3b's one start and one finish (`act` word `drive`), its leg
//!   in the act's params, projected into `tacsim:tv:<tv>` for interpolation and placed at its
//!   end at the due instant; a recover from farther than beside the ramp drives home first, in
//!   the same one act, never held to the 5 % floor;
//! - the home fields are cached beside the site cache (`want_home`, `peek_home`, one
//!   `Peeks` bundle per call) and warmed **whatever the geology layer's state**: a dark site's
//!   home is built from its keyless terrain (`ground::site_terrain`), so a rig's way home never
//!   depends on the geology key;
//! - `@geology <site> <addr>` (`commands`), and the map (`crate::port::site_map`).
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
pub mod cursor;
pub mod drive;
pub mod excursion;
pub mod geology;
pub mod ground;
pub mod key;
pub mod opening;
pub mod reading;
pub mod tenure;
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
    IC_HOUR, LAUNCH_S, LegWaypoint, MAX_DRIVE_TARGET_BYTES, MAX_DUMP_GOODS,
    MAX_EXCURSIONS_PER_OPERATOR, MAX_EXCURSIONS_PER_SITE, MAX_EXCURSIONS_WORLD, MAX_LEG_WAYPOINTS,
    MAX_READ_RADIUS_CELLS, MAX_SCAN_FIELDS, MAX_TV_LOG_BYTES, MAX_TV_LOG_LINES, MAX_TV_REC_BYTES,
    Outcome, PAD_CENTRE_CM, RECOVER_S, ReadSpan, RefusedCode, ReserveShort, RigKey, RigRecord,
    TV_REC_PREFIX, TV_REC_SCHEMA, TvLogLine, TvPose, TvRec, TvRecState, TvState, cap_refusal,
    cell_pose, cells_apart, charge_at, charge_for_act, charge_shown, leg_affordable, needs_battery,
    ramp_pose, reserve_check, reserved_total, settle_free, settle_spent_out_of_bay, site_cell,
    tv_rec_key, tv_rec_key_owner,
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
    /// **Doc 20 cut SM.4b** (lead ruling 4; the brief's decision P1, R3): the built home fields,
    /// one per (site entry, ramp cell), at most [`MAX_HOME_FIELDS`] (the least recently built
    /// dropped past it). RAM only.
    homes: BTreeMap<(CanonicalId, (i64, i64)), HomeEntry>,
    /// The home fields asked for — at landing, at boot and by a cold peek — worked by
    /// [`warm_one`] **whatever the layer's state** (R3), one job a fire.
    homes_wanted: BTreeSet<(CanonicalId, (i64, i64))>,
    /// The sites whose home field could not be built this boot (no terrain, or no field over
    /// it): never wanted or warned again, and read [`HomePeek::Dark`] (D25's rule).
    homes_failed: FailedSites,
    /// The home cache's clock (the least recently built is dropped first).
    homes_clock: u64,
}

/// **The most home fields one world holds** (doc 20 cut SM.4b): twice the site cache's sixteen
/// — a field is about 330 KiB (a cost and a step for each of 65,536 cells).
pub const MAX_HOME_FIELDS: usize = 2 * ground::SITE_CACHE_CAPACITY;

/// **One built home field** (doc 20 cut SM.4b): `built_over` is the warm site ground it was built
/// from (its composite) — `None` when it was built from keyless terrain because the site read
/// dark (R3) — and the clock tick it was built at.
#[derive(Clone, Debug)]
struct HomeEntry {
    built_over: Option<std::sync::Weak<SiteGround>>,
    field: Arc<drive::HomeField>,
    built: u64,
}

/// **What a peek at a site's home field found** (doc 20 cut SM.4b; R3).
#[derive(Clone, Debug)]
pub(crate) enum HomePeek {
    /// The field cannot be built this boot: its terrain failed to capture or the field did not
    /// form (`homes_failed`). Staff's remedy.
    Dark,
    /// Not built yet (or built over a ground the cache has since replaced): wanted.
    Cold,
    /// The field.
    Warm(Arc<drive::HomeField>),
}

/// **One call's peeks** (doc 20 cut SM.4b): every site's and every home's peek at most once a
/// call — the command, the start's offer projection, `+tv` status and the map each carry one.
#[derive(Clone, Debug, Default)]
pub(crate) struct Peeks {
    /// Site entry → its peek (`chart::peek_site`).
    pub sites: BTreeMap<CanonicalId, chart::SitePeek>,
    /// (Site entry, ramp cell) → its home's peek ([`peek_home`]).
    pub homes: BTreeMap<(CanonicalId, (i64, i64)), HomePeek>,
}

/// Whether `entry` is a lane site (keyless: `chart::lane_sites`).
fn is_lane_site(w: &World, entry: &CanonicalId) -> bool {
    chart::lane_sites(w).iter().any(|site| &site.entry == entry)
}

/// **Ask for a site's home field** (doc 20 cut SM.4b; lead ruling 4, R3): RAM only. Wants the home
/// for `(entry, ramp)` whenever `entry` is a lane site and its home has not failed this boot —
/// **whatever the geology layer's state**, so a far rig's way home is built at the first fire
/// after a keyless restart — and wants the site's ground too (a no-op when dark).
pub(crate) fn want_home(w: &mut World, entry: &CanonicalId, ramp: (i64, i64)) {
    want_site_ground(w, entry);
    if !is_lane_site(w, entry) {
        return;
    }
    if let Some(state) = w.module_state_mut::<GeologyState>()
        && !state.homes_failed.set().contains(entry)
    {
        state.homes_wanted.insert((entry.clone(), ramp));
    }
}

/// **The home of a hull settled on a site's pad** (doc 20 cut SM.4b): its site entry and its
/// ramp cell, through `act::site_berth`; `None` when it is not settled on a site.
pub(crate) fn hull_home(w: &World, hull: mush_core::ObjId) -> Option<(CanonicalId, (i64, i64))> {
    let view = crate::port::ship::hull_view(w, hull)?;
    let berth = act::site_berth(w, &view).ok()?;
    Some((
        berth.entry.clone(),
        excursion::site_cell(berth.ramp.x_cm, berth.ramp.y_cm),
    ))
}

/// **What a site's own peek read, for the home rule** ([`home_read`]): warm, cold or dark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SiteSeen {
    /// The site's ground is warm.
    Warm,
    /// Not built yet.
    Cold,
    /// The geology layer is dark for this site.
    Dark,
}

/// **The home field the cache holds, for the home rule** ([`home_read`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeldHome {
    /// None held.
    None,
    /// Built over the very ground the site's peek found warm.
    OverThisGround,
    /// Built over a ground the site cache has since replaced or dropped.
    OverOtherGround,
    /// Built from keyless terrain because the site read dark (R3).
    Keyless,
}

/// **What a home peek reads, for the home rule** ([`home_read`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HomeRead {
    /// The held field.
    Held,
    /// [`HomePeek::Cold`].
    Cold,
    /// [`HomePeek::Dark`].
    Dark,
}

/// **The home rule, pure** (doc 20 cut SM.4b; R3; the review's correctness/F4): a warm site reads
/// the field built over that very ground, a dark site the keyless one; anything else is cold —
/// **unless the home failed this boot**, which is never retried (D25's rule: `warm_one_home`
/// skips it and [`want_home`] never wants it again), so it reads dark rather than "still being
/// mapped" for the rest of the boot, whatever the site's own peek.
fn home_read(site: SiteSeen, held: HeldHome, failed: bool) -> HomeRead {
    let read = match (site, held) {
        (SiteSeen::Warm, HeldHome::OverThisGround) | (SiteSeen::Dark, HeldHome::Keyless) => {
            HomeRead::Held
        }
        (SiteSeen::Warm | SiteSeen::Cold | SiteSeen::Dark, _) => HomeRead::Cold,
    };
    match read {
        HomeRead::Cold if failed => HomeRead::Dark,
        other => other,
    }
}

/// **Peek a site's home field, never building it** (doc 20 cut SM.4b; R3), at most once per call
/// (`peeks`), by [`home_read`]: the site's own peek first — warm, the home built over that very
/// ground (`built_over` upgrades to it), else `Cold`; cold, `Cold`; **dark, the home built
/// keyless** (the site read dark when it was built), else `Cold` while its terrain may still be
/// captured — and `Dark` for a home that failed this boot, whatever the site's peek. A keyless
/// home stands for the boot: a terrain edit on a lane site during a dark boot is seen at the next
/// boot (the brief's decision P14's residual).
pub(crate) fn peek_home(
    w: &World,
    entry: &CanonicalId,
    ramp: (i64, i64),
    peeks: &mut Peeks,
) -> HomePeek {
    let key = (entry.clone(), ramp);
    if let Some(found) = peeks.homes.get(&key) {
        return found.clone();
    }
    let state = w.module_state::<GeologyState>();
    let failed = state.is_some_and(|state| state.homes_failed.set().contains(entry));
    let held = state.and_then(|state| state.homes.get(&key));
    let (site, over) = match chart::peek_site(w, entry, peeks) {
        chart::SitePeek::Warm(ground) => {
            let over = held.map(|home| {
                home.built_over
                    .as_ref()
                    .and_then(std::sync::Weak::upgrade)
                    .is_some_and(|over| Arc::ptr_eq(&over, &ground))
            });
            (SiteSeen::Warm, over)
        }
        chart::SitePeek::Cold => (SiteSeen::Cold, None),
        chart::SitePeek::Dark => (SiteSeen::Dark, None),
    };
    let shape = match (held, over) {
        (None, _) => HeldHome::None,
        (Some(home), _) if home.built_over.is_none() => HeldHome::Keyless,
        (Some(_), Some(true)) => HeldHome::OverThisGround,
        (Some(_), Some(false) | None) => HeldHome::OverOtherGround,
    };
    let found = match (home_read(site, shape, failed), held) {
        (HomeRead::Held, Some(home)) => HomePeek::Warm(Arc::clone(&home.field)),
        (HomeRead::Held | HomeRead::Cold, _) => HomePeek::Cold,
        (HomeRead::Dark, _) => HomePeek::Dark,
    };
    peeks.homes.insert(key, found.clone());
    found
}

/// Insert a built home, dropping the least recently built past [`MAX_HOME_FIELDS`].
fn insert_home(
    state: &mut GeologyState,
    key: (CanonicalId, (i64, i64)),
    built_over: Option<std::sync::Weak<SiteGround>>,
    field: drive::HomeField,
) {
    state.homes_clock = state.homes_clock.saturating_add(1);
    let built = state.homes_clock;
    state.homes.insert(
        key,
        HomeEntry {
            built_over,
            field: Arc::new(field),
            built,
        },
    );
    while state.homes.len() > MAX_HOME_FIELDS {
        let Some(oldest) = state
            .homes
            .iter()
            .min_by_key(|(_, home)| home.built)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        state.homes.remove(&oldest);
    }
}

/// **Work one wanted home field** (doc 20 cut SM.4b; [`warm_one`]'s second job): the first
/// wanted home that has not failed and does not already peek warm ([`peek_home`]); the site's peek
/// warm → the field over that ground's composite (`built_over` it); cold → the site wanted again
/// and the home kept wanted, nothing else this fire; dark → the field over the keyless terrain
/// (`ground::site_terrain`, R3). Each build is timed (`tracing::debug!`); a build that fails
/// warns once and joins `homes_failed` (never wanted or warned again this boot).
fn warm_one_home(ctx: &mut Ctx<'_>) {
    // The first wanted home that needs a build. A failed one is skipped (D25), and — the
    // review's correctness/F3 — so is one whose field over the current ground (or, dark, its
    // keyless field) still stands: every drive's arrival wants its home again, and that costs
    // no build ("built once per (site ground, ramp cell)", the brief's decision P1). Each turn
    // pops one, so the loop ends within the wanted set.
    let mut peeks = Peeks::default();
    let (entry, ramp) = loop {
        let Some((entry, ramp)) = ctx
            .w
            .module_state_mut::<GeologyState>()
            .and_then(|state| state.homes_wanted.pop_first())
        else {
            return;
        };
        let failed = ctx
            .w
            .module_state::<GeologyState>()
            .is_some_and(|state| state.homes_failed.set().contains(&entry));
        if failed {
            continue;
        }
        if matches!(
            peek_home(ctx.w, &entry, ramp, &mut peeks),
            HomePeek::Warm(_)
        ) {
            continue;
        }
        break (entry, ramp);
    };
    let started = std::time::Instant::now();
    let table = mush_atlas::physics::LocomotionTable::shipped();
    let built = match chart::peek_site(ctx.w, &entry, &mut peeks) {
        chart::SitePeek::Warm(ground) => chart::ground_surface(&ground)
            .and_then(|surface| drive::HomeField::build(surface, table, ramp))
            .map(|field| (Some(Arc::downgrade(&ground)), field))
            .ok_or_else(|| "no home field forms over the site's ground".to_owned()),
        chart::SitePeek::Cold => {
            want_site_ground(ctx.w, &entry);
            if let Some(state) = ctx.w.module_state_mut::<GeologyState>() {
                state.homes_wanted.insert((entry, ramp));
            }
            return;
        }
        chart::SitePeek::Dark => ground::site_terrain(ctx.w, &entry).and_then(|terrain| {
            terrain
                .surface(&mush_atlas::SurfaceRef::Ground)
                .and_then(|surface| drive::HomeField::build(surface, table, ramp))
                .map(|field| (None, field))
                .ok_or_else(|| "no home field forms over the site's terrain".to_owned())
        }),
    };
    match built {
        Ok((built_over, field)) => {
            let keyless = built_over.is_none();
            if let Some(state) = ctx.w.module_state_mut::<GeologyState>() {
                insert_home(state, (entry.clone(), ramp), built_over, field);
            }
            tracing::debug!(
                site = %entry,
                keyless,
                ms = started.elapsed().as_millis(),
                "tacsim: a site's home field was warmed"
            );
        }
        Err(error) => {
            tracing::warn!(
                site = %entry,
                %error,
                "tacsim: a site's way home cannot be mapped; its rigs read dark until the next boot"
            );
            if let Some(state) = ctx.w.module_state::<GeologyState>() {
                state.homes_failed.set().insert(entry);
            }
        }
    }
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
            homes: BTreeMap::new(),
            homes_wanted: BTreeSet::new(),
            homes_failed: FailedSites::default(),
            homes_clock: 0,
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

    /// **The built home fields** (doc 20 cut SM.4b): `(site entry, ramp cell, keyless)` in key
    /// order — `keyless` when the field was built from the terrain alone because the site read
    /// dark (R3). For tests and status renders.
    #[must_use]
    pub fn homes(&self) -> Vec<(CanonicalId, (i64, i64), bool)> {
        self.homes
            .iter()
            .map(|((entry, ramp), home)| (entry.clone(), *ramp, home.built_over.is_none()))
            .collect()
    }

    /// **A built home field** (doc 20 cut SM.4b), whatever ground it was built over.
    #[must_use]
    pub fn home_field(
        &self,
        entry: &CanonicalId,
        ramp: (i64, i64),
    ) -> Option<Arc<drive::HomeField>> {
        self.homes
            .get(&(entry.clone(), ramp))
            .map(|home| Arc::clone(&home.field))
    }

    /// The home fields waiting for a warm (doc 20 cut SM.4b), in key order.
    #[must_use]
    pub fn homes_wanted(&self) -> Vec<(CanonicalId, (i64, i64))> {
        self.homes_wanted.iter().cloned().collect()
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

/// **Warm one wanted site, else one wanted home** (doc 20 cut SM.4a; lead ruling 6; since cut
/// SM.4b the home fields too): called at the top of each `tacsim.surface` fire, one job a fire,
/// on the tick, never on an act's path. With the layer live a wanted **site** goes first: the
/// first wanted entry (id order) is built through [`ground::site_ground_sync`],
/// `tracing::debug!`ing its milliseconds; a failed build `tracing::warn!`s once, joins
/// `warm_failed` and is dropped — never wanted or warned again this boot (D25). With none wanted,
/// **or with the layer dark** (R3: a rig's way home never depends on the geology key), one wanted
/// **home field** is worked instead (`warm_one_home`).
pub(crate) fn warm_one(ctx: &mut Ctx<'_>) {
    if live(ctx.w).is_err() {
        warm_one_home(ctx);
        return;
    }
    let Some(entry) = ctx
        .w
        .module_state_mut::<GeologyState>()
        .and_then(|state| state.warm.pop_first())
    else {
        warm_one_home(ctx);
        return;
    };
    // A site a peek has marked dark since it was wanted (the pre-commit round's item 5) is
    // not built, and so not warned a second time.
    if warm_failed(ctx.w, &entry) {
        return;
    }
    // Doc 20 cut SM.4b: a site wanted again while already warm (a cold home's command, a drive's
    // arrival) costs no build, so the fire goes on to a wanted home — a site re-wanted every
    // fire must never starve the home fields.
    // The cache is read directly, never through `chart::peek_site`: a capture that refuses
    // here is the build's to warn (once, below).
    let already_warm = chart::site_context(ctx.w, &entry).is_ok_and(|site| {
        matches!(
            ground::site_ground_cached(ctx.w, &site.frame, &site.spec),
            Ok(Some(_))
        )
    });
    if already_warm {
        warm_one_home(ctx);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// **The home rule** (doc 20 cut SM.4b; R3; the review's correctness/F4): a warm site reads
    /// only the field built over that very ground, a dark site only the keyless one, and every
    /// other combination is cold — save a home that failed this boot, which is never retried and
    /// so reads dark whatever the site's peek (it never reads "still being mapped" forever).
    #[test]
    fn sm4b_a_failed_home_reads_dark_and_a_held_one_by_its_ground() {
        use HeldHome as H;
        use HomeRead as R;
        use SiteSeen as S;
        let held = [H::None, H::OverThisGround, H::OverOtherGround, H::Keyless];
        for site in [S::Warm, S::Cold, S::Dark] {
            for shape in held {
                let expected = match (site, shape) {
                    (S::Warm, H::OverThisGround) | (S::Dark, H::Keyless) => R::Held,
                    _ => R::Cold,
                };
                assert_eq!(
                    home_read(site, shape, false),
                    expected,
                    "{site:?} {shape:?}"
                );
                let failed = if expected == R::Held {
                    R::Held
                } else {
                    R::Dark
                };
                assert_eq!(
                    home_read(site, shape, true),
                    failed,
                    "failed: {site:?} {shape:?}"
                );
            }
        }
        // The case the review found: the site warm, its home failed and none held — dark, never
        // a cold "still being mapped" for the rest of the boot.
        assert_eq!(home_read(S::Warm, H::None, true), R::Dark);
    }
}
