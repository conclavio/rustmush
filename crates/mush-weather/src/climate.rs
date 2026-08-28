//! Climate definitions: the deterministic data behind the weather machine
//! (design 04 §10.1). A [`ClimateDef`] is a set of weather [`WxStateDef`]s,
//! a per-season transition matrix, time-of-day modifiers, and hand-written
//! line pools. Climates are shipped as TOML data files in `climates/*.toml`
//! and embedded at compile time via [`ClimateSet::builtin`].
//!
//! ## The LLM top-up seam (design 04 §10.4)
//!
//! States and transitions are **never** LLM-driven. Flavor text is: every
//! `lines.*` pool is a plain `Vec<String>` keyed by state / season /
//! time-of-day. M10's batch generator appends variants to these pools (per
//! `(climate, state, season, tod)` bucket) with no change to the machine.
//! [`LinePools`] is that structure; nothing here calls an LLM.

use std::collections::BTreeMap;

use mush_core::{Season, TimeOfDay};
use serde::Deserialize;
use smol_str::SmolStr;

/// One weather state's physical band (design 04 §10.1). `visibility`,
/// `precip`, and `wind` are `0..=100` scales; `temp_band` is `[low, high]`
/// in °C. The state `id` doubles as its web/ANSI icon hint.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct WxStateDef {
    /// State id: `"clear"`, `"overcast"`, `"rain"`, `"storm"`, `"snow"`,
    /// `"fog"` (also the panel/`Weather` icon hint).
    pub id: SmolStr,
    /// Visibility, `0` (whiteout) ..= `100` (perfectly clear).
    pub visibility: u8,
    /// Precipitation intensity, `0..=100`.
    pub precip: u8,
    /// Wind strength, `0..=100`.
    pub wind: u8,
    /// Temperature band `[low, high]` in °C the state's temperature is
    /// rolled within.
    pub temp_band: (i16, i16),
}

/// One transition matrix row set for a season: `from-state → [(to-state,
/// weight)]`. Weights are relative; the machine normalizes by their sum.
pub type TransitionMatrix = BTreeMap<SmolStr, Vec<(SmolStr, u32)>>;

/// Hand-written flavor pools (the LLM top-up seam — see the module docs).
/// Every field is a plain string pool; selection is a pure RNG pick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinePools {
    /// Generic "entering `<to>`" lines, keyed by destination state id.
    pub transition: BTreeMap<SmolStr, Vec<String>>,
    /// Specific `"<from>-><to>"` lines, preferred over [`LinePools::transition`].
    pub transition_pair: BTreeMap<SmolStr, Vec<String>>,
    /// Idle ambience while in `<state>`, keyed by state id.
    pub ambient: BTreeMap<SmolStr, Vec<String>>,
    /// Generic mood lines merged into the ambient pick, keyed by season name.
    pub ambient_season: BTreeMap<SmolStr, Vec<String>>,
    /// Generic mood lines merged into the ambient pick, keyed by tod name.
    pub ambient_tod: BTreeMap<SmolStr, Vec<String>>,
}

/// A fully-parsed, validated climate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClimateDef {
    /// Stable id, e.g. `"temperate-coastal"`.
    pub id: SmolStr,
    /// Human-facing name, e.g. `"Temperate Coastal"`.
    pub name: String,
    /// The weather states, in file order.
    pub states: Vec<WxStateDef>,
    /// Per-season transition matrices, keyed by season name
    /// (`"winter"`..`"autumn"`).
    pub seasonal: BTreeMap<SmolStr, TransitionMatrix>,
    /// Per-time-of-day weight modifiers: `tod name → (to-state → percent)`,
    /// where `100` is no change.
    pub tod: BTreeMap<SmolStr, BTreeMap<SmolStr, u32>>,
    /// Flavor line pools.
    pub lines: LinePools,
}

impl ClimateDef {
    /// The state def with this id.
    #[must_use]
    pub fn state(&self, id: &str) -> Option<&WxStateDef> {
        self.states.iter().find(|s| s.id == id)
    }

    /// The state id every fresh cell of this climate starts in: `"clear"`
    /// if present, else the first defined state.
    #[must_use]
    pub fn default_state(&self) -> SmolStr {
        self.state("clear")
            .map(|s| s.id.clone())
            .unwrap_or_else(|| self.states[0].id.clone())
    }
}

/// The season's line/matrix key (matches [`Season`]'s `Display`).
#[must_use]
pub fn season_key(s: Season) -> &'static str {
    match s {
        Season::Winter => "winter",
        Season::Spring => "spring",
        Season::Summer => "summer",
        Season::Autumn => "autumn",
    }
}

/// The time-of-day's modifier/line key (matches [`TimeOfDay`]'s `Display`).
#[must_use]
pub fn tod_key(t: TimeOfDay) -> &'static str {
    match t {
        TimeOfDay::Night => "night",
        TimeOfDay::Dawn => "dawn",
        TimeOfDay::Morning => "morning",
        TimeOfDay::Afternoon => "afternoon",
        TimeOfDay::Dusk => "dusk",
        TimeOfDay::Evening => "evening",
    }
}

// ----------------------------------------------------------------------
// Raw TOML shapes (deserialized, then validated into `ClimateDef`)
// ----------------------------------------------------------------------

#[derive(Deserialize)]
struct RawClimate {
    id: String,
    #[serde(default)]
    name: String,
    states: Vec<WxStateDef>,
    #[serde(default)]
    seasonal: BTreeMap<String, BTreeMap<String, BTreeMap<String, u32>>>,
    #[serde(default)]
    tod: BTreeMap<String, BTreeMap<String, u32>>,
    #[serde(default)]
    lines: RawLines,
}

#[derive(Default, Deserialize)]
struct RawLines {
    #[serde(default)]
    transition: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    transition_pair: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    ambient: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    ambient_season: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    ambient_tod: BTreeMap<String, Vec<String>>,
}

/// The four seasons every climate must define a transition matrix for.
const REQUIRED_SEASONS: [&str; 4] = ["winter", "spring", "summer", "autumn"];

impl ClimateDef {
    /// Parse and validate one climate from TOML text. `src` is only for
    /// error context (a file path or logical name).
    pub fn parse(src: &str, toml_text: &str) -> Result<ClimateDef, String> {
        let raw: RawClimate =
            toml::from_str(toml_text).map_err(|e| format!("{src}: parse error: {e}"))?;
        if raw.id.trim().is_empty() {
            return Err(format!("{src}: climate id is empty"));
        }
        if raw.states.is_empty() {
            return Err(format!("{src}: climate {:?} has no states", raw.id));
        }
        // Unique state ids.
        let mut known: Vec<SmolStr> = Vec::new();
        for s in &raw.states {
            if known.iter().any(|k| k == &s.id) {
                return Err(format!(
                    "{src}: climate {:?} has duplicate state {:?}",
                    raw.id, s.id
                ));
            }
            if s.temp_band.0 > s.temp_band.1 {
                return Err(format!(
                    "{src}: climate {:?} state {:?} has inverted temp_band {:?}",
                    raw.id, s.id, s.temp_band
                ));
            }
            known.push(s.id.clone());
        }
        let is_known = |id: &str| known.iter().any(|k| k == id);

        // Every required season present; every row references known states.
        let mut seasonal: BTreeMap<SmolStr, TransitionMatrix> = BTreeMap::new();
        for season in REQUIRED_SEASONS {
            let Some(rows) = raw.seasonal.get(season) else {
                return Err(format!(
                    "{src}: climate {:?} is missing the {season:?} transition matrix",
                    raw.id
                ));
            };
            let mut matrix: TransitionMatrix = BTreeMap::new();
            for (from, targets) in rows {
                if !is_known(from) {
                    return Err(format!(
                        "{src}: climate {:?} season {season:?} row {from:?} is not a state",
                        raw.id
                    ));
                }
                let mut weighted: Vec<(SmolStr, u32)> = Vec::new();
                for (to, w) in targets {
                    if !is_known(to) {
                        return Err(format!(
                            "{src}: climate {:?} season {season:?} row {from:?} → {to:?} \
                             is not a state",
                            raw.id
                        ));
                    }
                    if *w > 0 {
                        weighted.push((SmolStr::new(to), *w));
                    }
                }
                if weighted.is_empty() {
                    return Err(format!(
                        "{src}: climate {:?} season {season:?} row {from:?} has zero total weight",
                        raw.id
                    ));
                }
                matrix.insert(SmolStr::new(from), weighted);
            }
            seasonal.insert(SmolStr::new(season), matrix);
        }

        let smol_map = |m: &BTreeMap<String, u32>| -> BTreeMap<SmolStr, u32> {
            m.iter().map(|(k, v)| (SmolStr::new(k), *v)).collect()
        };
        let tod: BTreeMap<SmolStr, BTreeMap<SmolStr, u32>> = raw
            .tod
            .iter()
            .map(|(k, v)| (SmolStr::new(k), smol_map(v)))
            .collect();

        let smol_pool = |m: &BTreeMap<String, Vec<String>>| -> BTreeMap<SmolStr, Vec<String>> {
            m.iter()
                .map(|(k, v)| (SmolStr::new(k), v.clone()))
                .collect()
        };
        let lines = LinePools {
            transition: smol_pool(&raw.lines.transition),
            transition_pair: smol_pool(&raw.lines.transition_pair),
            ambient: smol_pool(&raw.lines.ambient),
            ambient_season: smol_pool(&raw.lines.ambient_season),
            ambient_tod: smol_pool(&raw.lines.ambient_tod),
        };

        let name = if raw.name.trim().is_empty() {
            raw.id.clone()
        } else {
            raw.name.clone()
        };
        Ok(ClimateDef {
            id: SmolStr::new(&raw.id),
            name,
            states: raw.states,
            seasonal,
            tod,
            lines,
        })
    }
}

// ----------------------------------------------------------------------
// The climate registry
// ----------------------------------------------------------------------

/// One `(source-name, TOML)` pair per shipped climate, embedded at compile
/// time. Adding a climate is: drop a `climates/*.toml` file and add a line
/// here.
const BUILTIN: &[(&str, &str)] = &[
    (
        "temperate-coastal",
        include_str!("../climates/temperate-coastal.toml"),
    ),
    ("desert", include_str!("../climates/desert.toml")),
    ("arctic", include_str!("../climates/arctic.toml")),
];

/// The loaded set of climates, keyed by id. Immutable after construction and
/// shared (`Arc`) across the tick, hooks, commands, and the query provider.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClimateSet {
    climates: BTreeMap<SmolStr, ClimateDef>,
}

impl ClimateSet {
    /// Load and validate every shipped climate. Errors name the offending
    /// file — a failure here is a bug in the crate's embedded data, caught
    /// by [`ClimateSet::builtin`]'s test.
    pub fn builtin() -> Result<ClimateSet, String> {
        let mut climates = BTreeMap::new();
        for (src, text) in BUILTIN {
            let def = ClimateDef::parse(src, text)?;
            if climates.contains_key(&def.id) {
                return Err(format!("duplicate climate id {:?}", def.id));
            }
            climates.insert(def.id.clone(), def);
        }
        Ok(ClimateSet { climates })
    }

    /// A set built from already-parsed climates (tests).
    #[must_use]
    pub fn from_defs(defs: impl IntoIterator<Item = ClimateDef>) -> ClimateSet {
        ClimateSet {
            climates: defs.into_iter().map(|d| (d.id.clone(), d)).collect(),
        }
    }

    /// The climate with this id (case-sensitive).
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&ClimateDef> {
        self.climates.get(id)
    }

    /// Whether a climate with this id is loaded.
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.climates.contains_key(id)
    }

    /// Every climate id, sorted.
    #[must_use]
    pub fn ids(&self) -> Vec<SmolStr> {
        self.climates.keys().cloned().collect()
    }

    /// Number of loaded climates.
    #[must_use]
    pub fn len(&self) -> usize {
        self.climates.len()
    }

    /// Whether no climates are loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.climates.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_climates_load_and_validate() {
        let set = ClimateSet::builtin().expect("shipped climates must parse");
        assert_eq!(set.len(), 3);
        for id in ["temperate-coastal", "desert", "arctic"] {
            let c = set.get(id).unwrap_or_else(|| panic!("missing {id}"));
            // All four seasons defined.
            for season in REQUIRED_SEASONS {
                assert!(c.seasonal.contains_key(season), "{id} missing {season}");
            }
            // The default state exists.
            assert!(c.state(&c.default_state()).is_some());
        }
    }

    #[test]
    fn parse_rejects_unknown_transition_target() {
        let bad = r#"
id = "bad"
[[states]]
id = "clear"
visibility = 100
precip = 0
wind = 10
temp_band = [0, 20]
[seasonal.winter]
clear = { nonesuch = 100 }
[seasonal.spring]
clear = { clear = 100 }
[seasonal.summer]
clear = { clear = 100 }
[seasonal.autumn]
clear = { clear = 100 }
"#;
        let err = ClimateDef::parse("bad", bad).unwrap_err();
        assert!(err.contains("is not a state"), "{err}");
    }

    #[test]
    fn parse_rejects_missing_season() {
        let bad = r#"
id = "bad"
[[states]]
id = "clear"
visibility = 100
precip = 0
wind = 10
temp_band = [0, 20]
[seasonal.winter]
clear = { clear = 100 }
"#;
        let err = ClimateDef::parse("bad", bad).unwrap_err();
        assert!(err.contains("missing the"), "{err}");
    }

    #[test]
    fn default_state_prefers_clear() {
        let set = ClimateSet::builtin().unwrap();
        assert_eq!(
            set.get("temperate-coastal").unwrap().default_state(),
            "clear"
        );
    }
}
