//! **The shipped mounts' numbers, read off the content** (doc 49 cut CC.0, Q-49-16).
//!
//! A lens that asserts a magazine reads it here, not as a literal, so a retune of a
//! mount or round pack moves only the content pins (the digests, the harness
//! baseline and the corpus pins), never these stories. What each story asserts
//! about the rows is unchanged: a fresh unit reads a full load, a shot spends what
//! its burst spends, and so on. No lens asserts a cycle's length (each reads
//! `cycling (` and nothing after it), so the helper carries none.
//!
//! The numbers come from [`tacsim::builtin_definitions`], the catalog the
//! production composition binds `SIM UNIT` and `@unit/create` through, so the
//! story's server and these helpers read one content.

use std::sync::OnceLock;

use mush_core::CanonicalId;
use tacsim::definitions::{Catalog, unit::ComponentRole};

/// The shipped catalog, bound once a process.
fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        tacsim::builtin_definitions()
            .unwrap_or_else(|error| panic!("the shipped content binds: {error}"))
    })
}

/// One mount on a shipped unit, as its packs author it.
#[derive(Clone, Copy, Debug)]
pub struct Mount {
    /// Its round's `count_per_load`: what a fresh unit carries.
    pub magazine: u32,
    /// Its `burst_rounds`: what one pull spends.
    pub burst: u32,
}

impl Mount {
    /// `N of N round(s)`: the full load a fresh unit's `WEAPONSTATUS` row reads.
    pub fn full(self) -> String {
        self.left(0)
    }

    /// `M of N round(s)` once `spent` rounds have gone.
    pub fn left(self, spent: u32) -> String {
        let left = self
            .magazine
            .checked_sub(spent)
            .unwrap_or_else(|| panic!("{spent} round(s) spent from a load of {}", self.magazine));
        format!("{left} of {} round(s)", self.magazine)
    }
}

/// The mount `component` on the shipped unit `unit` (`line_tank`, `scout_mk1`,
/// `heavy_mk1`, ...): its burst and its round's load.
///
/// # Panics
///
/// When the unit is not fielded, or carries no such mount: a story's premise.
pub fn mount(unit: &str, component: &str) -> Mount {
    let catalog = catalog();
    let key = CanonicalId::new(format!("tacsim.unit.{unit}"))
        .unwrap_or_else(|_| panic!("tacsim.unit.{unit} is an id"));
    let spec = catalog
        .specs
        .get(&key)
        .unwrap_or_else(|| panic!("{key} is fielded"));
    let weapon = spec
        .components
        .iter()
        .find_map(|slot| match &slot.role {
            ComponentRole::Weapon { weapon, .. } if slot.id.as_str() == component => Some(weapon),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{key} carries no mount {component}"));
    let def = catalog
        .weapons
        .get(weapon)
        .unwrap_or_else(|| panic!("{weapon} is in the catalog"));
    let round = catalog
        .ammunition
        .get(&def.ammunition)
        .unwrap_or_else(|| panic!("{} is in the catalog", def.ammunition));
    Mount {
        magazine: round.count_per_load,
        burst: def.burst_rounds,
    }
}
