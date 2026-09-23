//! The live weather state: per-zone [`WeatherCell`]s, the deterministic
//! [`WxRng`], and their persistence.
//!
//! [`WeatherState`] lives in a module state slot on the `World` (in-RAM
//! working state — not persisted as a slot). Every cell mutation mirrors to
//! a `mod:weather:cell:{zone}` document via [`ModuleOp`], so persistence is
//! write-through with command/tick atomicity; [`mush_core::GameModule::on_world_loaded`]
//! rebuilds the cells from those documents at boot. The RNG is **not**
//! persisted — it reseeds at boot, which is fine: cells carry the durable
//! state and the machine only needs *some* deterministic stream from there.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, ModuleOp, ModuleWrite, WxSnapshot};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::climate::WxStateDef;
use crate::flavor::FlavorStore;

/// The weather module's persistence namespace (`mod:weather:*`).
pub const NS: &str = "weather";

/// Wind scale (`0..=100`) → metres per second for [`WxSnapshot`]. A gale
/// (`100`) maps to ~30 m/s. Documented so ruleset consumers know the unit.
pub const WIND_MS_PER_UNIT: f32 = 0.30;

/// One opted-in place's live weather (design 04 §10.1; S2b). Keyed by the atlas
/// place's id — a neighborhood's or a region's pack id — and the whole struct is
/// the persisted `cell:<place id>` document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherCell {
    /// The atlas place this cell belongs to: a neighborhood or a region, by pack id.
    pub place: CanonicalId,
    /// The climate id driving this cell (a key into the [`crate::ClimateSet`]).
    pub climate: SmolStr,
    /// The current weather state id (a key into the climate's states).
    pub state: SmolStr,
    /// IC timestamp (game-clock seconds) the current state was entered.
    #[serde(default)]
    pub since: i64,
    /// Current temperature, °C (rolled within the state's band on entry).
    #[serde(default)]
    pub temp: i16,
    /// Staff pin: when set, the transition machine is paused for this zone
    /// (`@weather <zone>=<state>`); `/release` clears it.
    #[serde(default)]
    pub pinned: bool,
}

impl WeatherCell {
    /// A fresh cell for `place` in `climate`, starting in `state`.
    #[must_use]
    pub fn new(
        place: CanonicalId,
        climate: impl Into<SmolStr>,
        state: impl Into<SmolStr>,
    ) -> WeatherCell {
        WeatherCell {
            place,
            climate: climate.into(),
            state: state.into(),
            since: 0,
            temp: 0,
            pinned: false,
        }
    }

    /// Build the reserved [`WxSnapshot`] (mush-core's `WeatherQuery` payload)
    /// from this cell and its resolved [`WxStateDef`].
    #[must_use]
    pub fn snapshot(&self, def: &WxStateDef) -> WxSnapshot {
        WxSnapshot {
            state: self.state.clone(),
            visibility: f32::from(def.visibility) / 100.0,
            precipitation: f32::from(def.precip) / 100.0,
            wind: f32::from(def.wind) * WIND_MS_PER_UNIT,
            temperature: f32::from(self.temp),
        }
    }
}

/// A tiny xorshift64\* PRNG — the deterministic randomness behind transition
/// rolls, temperature, and line selection. Held in [`WeatherState`] so tests
/// inject a known seed and assert exact outcomes (there is no RNG in
/// mush-core; the design calls for a counter/hash here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WxRng {
    state: u64,
}

impl WxRng {
    /// A generator seeded from `seed`. The seed is run through SplitMix64
    /// first, so adjacent seeds (42 vs 43) still diverge and no seed maps to
    /// the xorshift dead state.
    #[must_use]
    pub fn seeded(seed: u64) -> WxRng {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        WxRng { state: z | 1 }
    }

    /// Advance and return the next 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// The next 32-bit value (high bits of [`WxRng::next_u64`]).
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// A value in `0..n` (`0` when `n == 0`).
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 { 0 } else { self.next_u32() % n }
    }

    /// A `bool` true with probability `permille / 1000`.
    pub fn chance_permille(&mut self, permille: u32) -> bool {
        self.below(1000) < permille
    }
}

impl Default for WxRng {
    /// A fixed non-zero default seed (golden-ratio constant).
    fn default() -> WxRng {
        WxRng::seeded(0x9E37_79B9_7F4A_7C15)
    }
}

/// The live weather state, held in a module state slot on the `World`.
///
/// **In-RAM working state**: state slots are not persisted, so the durable
/// truth is the `cell:{zone}` document set. `rng`/`tick_count` are
/// deterministic scratch that reseed at boot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeatherState {
    /// One cell per opted-in place, keyed by the place's pack id.
    pub cells: BTreeMap<CanonicalId, WeatherCell>,
    /// The deterministic randomness stream (injectable in tests). Defaults
    /// to [`WxRng::default`]'s fixed seed.
    pub rng: WxRng,
    /// Coarse ticks fired since boot (diagnostics / seed mixing).
    pub tick_count: u64,
    /// The M10 LLM flavor top-up pool: staff-reviewable generated line
    /// variants, mirrored to `flavor:*` documents. Only **approved** lines
    /// enter the live selection pool; pending lines are never shown. Empty
    /// (and inert) when the feature is unused — the hand-written lines carry
    /// the weather unchanged (M5 behavior).
    pub flavor: FlavorStore,
}

impl WeatherState {
    /// The cell for `place`, if the place opted in.
    #[must_use]
    pub fn cell(&self, place: &CanonicalId) -> Option<&WeatherCell> {
        self.cells.get(place)
    }
}

// ----------------------------------------------------------------------
// Persistence keys + op helpers
// ----------------------------------------------------------------------

/// The kv key of a place's cell document.
#[must_use]
pub fn cell_key(place: &CanonicalId) -> SmolStr {
    SmolStr::new(format!("cell:{place}"))
}

/// A `JsonSet` op mirroring one cell (cells are plain data; serialization
/// cannot fail).
#[must_use]
pub fn persist_cell(cell: &WeatherCell) -> ModuleOp {
    let value = serde_json::to_value(cell).expect("weather cell serializes");
    ModuleOp {
        ns: SmolStr::new_static(NS),
        op: ModuleWrite::JsonSet {
            key: cell_key(&cell.place),
            value,
        },
    }
}

/// A `Del` op removing a place's cell document (opt-out).
#[must_use]
pub fn delete_cell(place: &CanonicalId) -> ModuleOp {
    ModuleOp {
        ns: SmolStr::new_static(NS),
        op: ModuleWrite::Del {
            key: cell_key(place),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_from_seed() {
        let mut a = WxRng::seeded(42);
        let mut b = WxRng::seeded(42);
        let seq_a: Vec<u32> = (0..8).map(|_| a.next_u32()).collect();
        let seq_b: Vec<u32> = (0..8).map(|_| b.next_u32()).collect();
        assert_eq!(seq_a, seq_b);
        // Different seeds diverge.
        let mut c = WxRng::seeded(43);
        let seq_c: Vec<u32> = (0..8).map(|_| c.next_u32()).collect();
        assert_ne!(seq_a, seq_c);
    }

    #[test]
    fn below_is_bounded() {
        let mut r = WxRng::seeded(7);
        for _ in 0..1000 {
            assert!(r.below(6) < 6);
        }
        assert_eq!(r.below(0), 0);
    }

    #[test]
    fn snapshot_maps_bands_to_physical_units() {
        let def = WxStateDef {
            id: "rain".into(),
            visibility: 60,
            precip: 80,
            wind: 50,
            temp_band: (2, 18),
        };
        let place = CanonicalId::new("neighborhood.downtown").expect("id");
        let mut cell = WeatherCell::new(place, "temperate-coastal", "rain");
        cell.temp = 11;
        let snap = cell.snapshot(&def);
        assert_eq!(snap.state, "rain");
        assert!((snap.visibility - 0.60).abs() < 1e-6);
        assert!((snap.precipitation - 0.80).abs() < 1e-6);
        assert!((snap.wind - 15.0).abs() < 1e-6);
        assert!((snap.temperature - 11.0).abs() < 1e-6);
    }

    #[test]
    fn cell_key_uses_the_place_id() {
        let place = CanonicalId::new("region.limbo").expect("id");
        assert_eq!(cell_key(&place), "cell:region.limbo");
    }
}
