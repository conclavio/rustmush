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
//! **Doc 20 cut SM.2** installs the lane's Ceres and Vesta tables at boot and at apply
//! ([`install_lane_geology`](crate::surface::install_lane_geology), over
//! [`lane_geologies`](crate::surface::lane_geologies) of the lane's `BodyDef`s) and adds the
//! sites and their block shifts (`crate::port::site`); `@geology <site> <addr>` moves to SM.4,
//! with the off-loop site driver, footing from composite cells and the instruments; SM.6
//! reserves from the geology table; SM.7 mining.
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
//! The slot ([`GeologyState`](crate::surface::GeologyState)) is RAM only and never persisted: the key, its status, the bodies'
//! tables and the site cache. Both installers write it through `World::module_state_mut` from
//! inside this crate (the `mush_tactics::ground::configure` pattern), so the state-slot owner
//! census needs no row.

pub mod act;
pub mod commands;
pub mod excursion;
pub mod geology;
pub mod ground;
pub mod key;
pub mod reading;
pub mod tick;

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{CanonicalId, GeologyMac, World};

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
    MAX_PROBE_CU, MAX_PROBE_S, Refusal, RigAt, SURFACE_ACT_ACTION, SURFACE_ACT_OUTCOME,
    SURFACE_DONE_ACTION, SURFACE_DONE_OUTCOME, SURFACE_KEY_PREFIX, Where, act_key, done_key,
};
pub use excursion::{
    Act, ActKind, ActParams, ActResult, ActView, CapRefused, Charge, ChargeAfter, DUMP_S,
    EXCURSION_FAMILY, EXCURSION_SCHEMA, Excursion, ExcursionCounts, ExcursionId, FrozenLeg,
    IC_HOUR, LAUNCH_S, LegWaypoint, MAX_DUMP_GOODS, MAX_EXCURSIONS_PER_OPERATOR,
    MAX_EXCURSIONS_PER_SITE, MAX_EXCURSIONS_WORLD, MAX_LEG_WAYPOINTS, MAX_TV_LOG_BYTES,
    MAX_TV_LOG_LINES, MAX_TV_REC_BYTES, Outcome, PAD_CENTRE_CM, RECOVER_S, RefusedCode,
    ReserveShort, RigKey, RigRecord, TV_REC_PREFIX, TV_REC_SCHEMA, TvLogLine, TvPose, TvRec,
    TvRecState, TvState, cap_refusal, cells_apart, charge_at, needs_battery, ramp_pose,
    reserve_check, reserved_total, settle_free, site_cell, tv_rec_key, tv_rec_key_owner,
};
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
}

impl Default for GeologyState {
    fn default() -> Self {
        GeologyState {
            boot: None,
            status: GeologyStatus::Disabled(GeologyDisabled::NotBooted),
            mac: None,
            tables: BTreeMap::new(),
            sites: SiteGroundCache::default(),
        }
    }
}

impl GeologyState {
    /// The site ground cache.
    #[must_use]
    pub fn sites(&self) -> &SiteGroundCache {
        &self.sites
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
/// lane's `BodyDef` tables, at boot and at apply) and by tests. A body already installed is
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
            crate::pack::PackEntry::Body(def) => {
                let (Some(radius), Some(table)) = (def.radius_km, def.geology.as_ref()) else {
                    return None;
                };
                Some(
                    BodyGeology::new(id.clone(), radius, table.clone())
                        .map_err(|error| format!("{id}: {error}")),
                )
            }
            _ => None,
        })
        .collect()
}

/// **Install what [`lane_geologies`] built** (doc 20 cut SM.2, step two) through
/// [`install_body_geology`]: an `Err` is warned and skipped, and **a body whose installed
/// table is already equal is skipped**, so its site cache survives each of the six site
/// applies and every boot. RAM only; nothing is written. Returns how many it installed.
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
