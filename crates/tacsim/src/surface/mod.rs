//! **Surface mining's ground** (doc 20, the quarry loop; this module is cut SM.1: §3.2, §5,
//! D-20-2, D-20-4, Q-20-10, Q-20-12, §16).
//!
//! The world gets a ground to mine before it gets a site, a vehicle or a player verb:
//!
//! - [`geology`] — the **keyed geology function** (D-20-4): HMAC-SHA256 under the fourth durable
//!   key family over a body-fixed pocket lattice, generated top-down (province → field →
//!   pocket) so every reading nests; one word set (barren, lean, fair, rich); integers only, no
//!   RNG;
//! - [`reading`] — **the reading rule, version 1** (§5): R0, R1 (sharpened by footing) and R2
//!   as pure quantisations of that function;
//! - [`key`] — the key's **boot, its fingerprint pin** (`tacsim:geology`) and the RAM-only
//!   disabled state every later consumer refuses on ([`live`]); the readiness report's
//!   `geology:` line (reported, never gated);
//! - [`ground`] — a site's **geology table** and terrain composite, held in an in-RAM LRU of
//!   sixteen, with a synchronous build door;
//! - [`commands`] — `@geology`, staff truth over the body-fixed lattice.
//!
//! What later cuts add: SM.2 the Ceres and Vesta tables as lane data (replacing
//! [`install_body_geology`]'s test-only caller), sites, their block shifts and `@geology <site>
//! <addr>`; SM.4 the off-loop site driver, footing from composite cells and the instruments; SM.6
//! reserves from the geology table; SM.7 mining.
//!
//! The slot ([`GeologyState`]) is RAM only and never persisted: the key, its status, the bodies'
//! tables and the site cache. Both installers write it through `World::module_state_mut` from
//! inside this crate (the `mush_tactics::ground::configure` pattern), so the state-slot owner
//! census needs no row.

pub mod commands;
pub mod geology;
pub mod ground;
pub mod key;
pub mod reading;

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

/// A body's ground for this world. SM.1's only source (tests); SM.2 replaces the caller with
/// the lane's `BodyDef` tables. A body already installed is replaced, and **its cached sites
/// are dropped**: the site cache's key (`ground::site_revision`) carries the captured input and
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
