//! **The stock tank, authored** (16b cut C.2, C1; doc 16 §3.3 "Units", §4
//! "Content packs"; D-16-6).
//!
//! # Two `UnitSpec`s, and why
//!
//! `mush_tactics::UnitSpec` is M32's *instantiation* shape: sections, facets,
//! cells, components, capabilities and a mobility profile, every external
//! reference already an exact [`DefinitionRef`] with a digest in it. A digest
//! cannot be authored — it is computed when the definition registers — so a
//! content file cannot be one.
//!
//! [`UnitSpec`] here is the *authored* shape: the table of doc 16 §3.3 as
//! integers and names. [`bind`] is the one function that turns the second into
//! the first, minting every reference through a scratch
//! [`DefinitionRegistryBuilder`] with the same owner the module registers under,
//! so the refs it hands back are **the same values** `Registrar::definition`
//! will produce. That is what lets the composition root build a
//! `mush_tactics::UnitCatalog` and the module register the descriptors without
//! either being able to disagree with the other: both are `bind`'s output.
//!
//! # What M32 holds and what it carries
//!
//! M32 is the unit's truth (D-16-6) and interprets **nothing** game-specific.
//! So the anatomy — sections, facets, the armour and internals cells, the four
//! components, the tracked mobility mode — becomes real M32 structure, and
//! everything the arena reads and M32 must not (the facet-attribution table, the
//! silhouettes, `sensor_height_cm`, which weapon each component fires, each
//! section's consequences) rides in the unit's opaque `RulesValue` under
//! [`UNIT_RULES_SCHEMA`]. `tacsim` is the only layer that decodes it, exactly as
//! it is the only layer that decodes [`mush_tactics::ModeRules`].

use std::collections::BTreeMap;

use mush_core::CanonicalId;
use mush_core::mechanics::{DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef};
use mush_core::{ObjId, RulesValue};
use mush_tactics::{
    CapabilityId, CapabilityInit, CapabilityKey, CellId, CellSpec, ComponentId, ComponentSpec,
    ConditionId, FacetId, FacetSpec, InstallPoint, LocomotionId, MediumId, MobilityMode,
    MobilityModeId, MobilityProfile, MobilityVocabulary, ModeRules, SectionId, SectionSpec,
    TacticalUnit, UnitSpecBody,
};
use serde::{Deserialize, Serialize};

use crate::arena::state::{Side, TrackedRules};
use crate::definitions::ammunition::AmmunitionDef;
use crate::definitions::sensor::SensorDef;
use crate::definitions::weapon::WeaponDef;
use crate::definitions::{DefinitionError, MAX_ENTRY_REFERENCES};

/// A `mush-tactics` id from an authored word, or a refusal naming the field.
///
/// A macro rather than a function because every one of `mush-tactics`' seven id
/// newtypes has its own `new(impl AsRef<str>)`, with no trait in common, and
/// passing `FacetId::new` as a higher-ranked function value does not type-check
/// (its `AsRef` parameter binds one lifetime, not all of them).
macro_rules! tactics_id {
    ($entry:expr, $field:expr, $word:expr, $ty:ty) => {
        <$ty>::new($word).map_err(|error| {
            DefinitionError::value($entry, format!("{} `{}`: {error}", $field, $word))
        })
    };
}

/// The `schema` definition the unit's opaque rules blob is bound to.
pub const UNIT_RULES_SCHEMA: &str = "tacsim.unit.rules";

/// The `schema` definition a tracked mode's opaque rules blob is bound to.
pub const TRACKED_RULES_SCHEMA: &str = "tacsim.mobility.tracked";

/// The `schema` definition the power capability's rules blob is bound to.
pub const POWER_RULES_SCHEMA: &str = "tacsim.power.states";

/// The locomotion id doc 16 §3.3's mode rules belong to — the atlas's own
/// (`mush_atlas::physics::BASE_LOCOMOTIONS`), so the arena's move costs and the
/// unit's mobility name the same thing.
pub const TRACKED: &str = "tracked";

/// The medium a tracked mode moves in.
pub const GROUND: &str = "ground";

/// The mobility mode id the stock tank carries.
pub const TRACKED_MODE: &str = "ground.tracked";

/// The armour cell every facet carries.
pub const ARMOR_CELL: &str = "armor";

/// The cell every section carries under its armour.
pub const INTERNALS_CELL: &str = "internals";

/// A firing component's magazine cell.
pub const AMMO_CELL: &str = "ammo";

/// A firing component's cycle cell (doc 16 §3.3 "Components").
pub const READY_AT_CELL: &str = "ready_at_ms";

/// A sensor component's condition cell.
pub const CONDITION_CELL: &str = "condition";

/// **The heat a unit is carrying** (16b cut X.1; doc 16 §9 item 1: *a `heat` cell
/// and dissipation appear on the unit*).
///
/// A section cell on the unit's body, laid down **only** when the spec declares a
/// [`UnitSpec::heat_max`], so a unit whose content predates the laser — every
/// stock unit — carries no such cell and is byte-identical to what it was. It
/// starts empty and its `max` **is** the ceiling `FIRE` refuses at, which is why
/// there is no second number to keep in step with it.
pub const HEAT_CELL: &str = "heat";

/// **What the unit sheds per second** (16b cut X.1), beside [`HEAT_CELL`] on the
/// same section.
///
/// A rating, carried as a cell rather than as a constant so that the one place a
/// unit's heat numbers live is the unit — and so a later extension can damage a
/// unit's heat sinks the way a hit damages its sight, with no new seam. Nothing
/// writes it today.
pub const HEAT_SINK_CELL: &str = "heat_sink";

/// **What one shot of a weapon adds** (16b cut X.1), a cell on the *weapon's*
/// component, laid down only when its weapon authors a non-zero
/// `heat_per_shot`.
///
/// The arena reads it at check-out exactly as it reads the armour and the
/// magazine: the alternative is a new field on the check-out's weapon row, and
/// the row is a struct six test files build literally.
pub const HEAT_PER_SHOT_CELL: &str = "heat_per_shot";

/// The condition a section, and a component, is born in.
pub const INTACT: &str = "intact";

/// The ceiling of a `ready_at_ms` cell: an hour of arena time, which is
/// `match.max_duration_ms` (16b §9), so a cycle can never be set past the match
/// that contains it.
pub const MAX_READY_AT_MS: i64 = 3_600_000;

/// The ceiling of a sensor's `condition` cell: a percentage.
pub const MAX_CONDITION: i64 = 100;

/// The most armour or internals one cell may hold.
pub const MAX_CELL_VALUE: u32 = 100_000;

/// The most sections one unit may declare here. M32's own cap is far higher;
/// this is the authoring bound, because every section is a row of the facet
/// table and a weight in every band.
pub const MAX_SECTIONS: usize = 32;

/// The most facets one section may declare.
pub const MAX_FACETS: usize = 8;

/// The most components one unit may declare.
pub const MAX_COMPONENTS: usize = 32;

/// The most consequences one section may declare.
pub const MAX_CONSEQUENCES: usize = 8;

/// The tallest a sensor may sit, centimetres (doc 16 §3.2 "Line of sight").
pub const MAX_SENSOR_HEIGHT_CM: u32 = 10_000;

/// The widest or tallest a silhouette may present, centimetres.
pub const MAX_SILHOUETTE_CM: u32 = 100_000;

// ---------------------------------------------------------------------------
// The authored shape
// ---------------------------------------------------------------------------

/// What happens when a section's `internals` reach zero (doc 16 §3.3, §7 "What
/// stays Rust": the consequence kinds are a closed vocabulary and config picks
/// from it by name).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Consequence {
    /// The unit becomes a wreck (doc 16 §3.3 "A wreck").
    DestroyUnit,
    /// Every weapon mounted on `section` is removed.
    RemoveWeapons {
        /// The section whose weapons go.
        section: CanonicalId,
    },
    /// The unit's speed ceiling falls to `percent` of its rules value.
    Slow {
        /// The surviving percentage, 1 to 100.
        percent: u8,
    },
    /// The unit can no longer turn toward `side`.
    NoTurnToward {
        /// Left or right.
        side: Side,
    },
    /// The unit cannot move at all.
    DisableMotion,
}

impl Consequence {
    /// The word this consequence renders as.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Consequence::DestroyUnit => "destroy_unit",
            Consequence::RemoveWeapons { .. } => "remove_weapons",
            Consequence::Slow { .. } => "slow",
            Consequence::NoTurnToward { .. } => "no_turn_toward",
            Consequence::DisableMotion => "disable_motion",
        }
    }
}

/// One armour facet of a section (doc 16 §3.3's *Facets (armor cells)* column).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FacetArmor {
    /// The facet: `front`, `left`, `right`, `rear`, `top`.
    pub id: CanonicalId,
    /// The facet's `armor` cell, full and at its maximum when the unit is made.
    pub armor: u32,
}

/// Where a section with no facets of its own is struck (doc 16 §3.3: the engine
/// has *no facets; struck through `hull.rear`*).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StruckThrough {
    /// The section whose armour a shot must pass.
    pub section: CanonicalId,
    /// The facet of it.
    pub facet: CanonicalId,
}

/// One section of a unit (doc 16 §3.3's table row).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectionDef {
    /// The section: `hull`, `turret`, `left_track`, `right_track`, `engine`.
    pub id: CanonicalId,
    /// Its armour facets; empty for a section struck through another's.
    #[serde(default)]
    pub facets: Vec<FacetArmor>,
    /// Its `internals` cell, full when the unit is made.
    pub internals: u32,
    /// What happens at zero internals, in order.
    pub consequences: Vec<Consequence>,
    /// Where this section is struck when it has no facets of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub struck_through: Option<StruckThrough>,
}

/// What a component *is* (doc 16 §3.3 "Components"). A closed kind over open
/// ids, exactly as `CellAddr` is: the arena knows these three roles and content
/// picks from them by name.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComponentRole {
    /// A weapon: an `ammo` cell sized from its ammunition's `count_per_load`
    /// and a `ready_at_ms` cell.
    Weapon {
        /// The weapon definition it fires — an entry id, checked at load.
        weapon: CanonicalId,
        /// Whether it is slaved to another weapon's aim (the coaxial machine
        /// gun of doc 16 §3.3).
        #[serde(default)]
        coaxial: bool,
    },
    /// A sensor: a `condition` cell.
    Sensor {
        /// The sensor definition it reads through — an entry id, checked at
        /// load.
        sensor: CanonicalId,
    },
    /// The power plant: a capability with a named state set (doc 16 §3.3
    /// "Startup").
    Power {
        /// The capability id, `tacsim.power`.
        capability: CanonicalId,
        /// Its states, in order: `off`, `starting`, `online`.
        states: Vec<CanonicalId>,
        /// The state a freshly made unit is in.
        initial: CanonicalId,
    },
}

/// One component (doc 16 §3.3 "Components").
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentDef {
    /// The component: `cannon`, `machine_gun`, `visual_sensor`, `power`.
    pub id: CanonicalId,
    /// The section it is installed in.
    pub section: CanonicalId,
    /// What it is.
    pub role: ComponentRole,
}

/// One row of the facet-attribution table (doc 16 §3.3 "Facet attribution",
/// W12): *a table in the `UnitSpec` maps each band to the sections it can strike
/// with weights. No geometry is computed.*
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FacetBand {
    /// The band an impact's relative bearing selects.
    pub band: CanonicalId,
    /// The sections it can strike, and how often.
    pub sections: Vec<FacetWeight>,
}

/// One section's share of a band.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FacetWeight {
    /// The section.
    pub section: CanonicalId,
    /// Its weight in this band; the band's weights need not sum to anything.
    pub weight: u16,
}

/// What a band presents (doc 16 §3.3 "Silhouette", §3.5 step 1).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Silhouette {
    /// The band.
    pub band: CanonicalId,
    /// Presented width, centimetres.
    pub width_cm: u32,
    /// Presented height, centimetres.
    pub height_cm: u32,
}

/// The `tracked` mode's rules as authored (doc 16 §3.3 "Mobility mode rules").
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackedDef {
    /// The forward ceiling, cm/s.
    pub max_speed_cms: u32,
    /// Acceleration, cm/s².
    pub accel_cms2: u32,
    /// Braking, cm/s².
    pub brake_cms2: u32,
    /// Turn rate, millidegrees per second.
    pub turn_rate_mdeg_s: u32,
    /// The reverse ceiling as a percentage of `max_speed_cms`.
    pub reverse_factor_pct: u8,
}

/// A unit's mobility. One mode today; doc 16 §9's extensions add their own.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MobilityDef {
    /// The tracked mode.
    pub tracked: TrackedDef,
}

/// One unit schema (doc 16 §3.3's `line_tank`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitSpec {
    /// The human-facing name.
    pub label: String,
    /// The sections, their facets, their internals and their consequences.
    pub sections: Vec<SectionDef>,
    /// The components.
    pub components: Vec<ComponentDef>,
    /// The mobility mode rules.
    pub mobility: MobilityDef,
    /// The facet-attribution table.
    pub facets: Vec<FacetBand>,
    /// What each band presents.
    pub silhouettes: Vec<Silhouette>,
    /// How high the unit's eye sits (doc 16 §3.2 "Line of sight"; 150 by
    /// default in the prose, authored here).
    pub sensor_height_cm: u32,
    /// **The heat ceiling** (16b cut X.1; doc 16 §9 item 1), or `0` for a unit
    /// that carries no heat at all.
    ///
    /// Zero is the default and the whole of what keeps `line_tank` unchanged:
    /// nothing is serialised, no cell is laid down, and the arena's per-substep
    /// dissipation skips the unit on an `Option` test. A unit that declares it
    /// gets a [`HEAT_CELL`] whose `max` is this number, and `FIRE` refuses at it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub heat_max: u32,
    /// **What it sheds per second of arena time** (16b cut X.1), or `0`.
    ///
    /// Declared together with [`UnitSpec::heat_max`] or not at all: a unit that
    /// can heat and never cool would lock its own weapons out for the match, and
    /// a unit that cools and cannot heat is a number nothing reads.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub heat_dissipation_per_s: u32,
    /// Why these numbers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Whether an authored number is absent — a `serde` predicate, so a spec that
/// declares no heat serialises exactly as one written before heat existed.
fn is_zero(value: &u32) -> bool {
    *value == 0
}

impl UnitSpec {
    /// Every weapon definition this unit names, in component order.
    pub fn weapons(&self) -> impl Iterator<Item = &CanonicalId> {
        self.components.iter().filter_map(|c| match &c.role {
            ComponentRole::Weapon { weapon, .. } => Some(weapon),
            _ => None,
        })
    }

    /// Every sensor definition this unit names, in component order.
    pub fn sensors(&self) -> impl Iterator<Item = &CanonicalId> {
        self.components.iter().filter_map(|c| match &c.role {
            ComponentRole::Sensor { sensor } => Some(sensor),
            _ => None,
        })
    }

    /// The section named `id`, when there is one.
    #[must_use]
    pub fn section(&self, id: &CanonicalId) -> Option<&SectionDef> {
        self.sections.iter().find(|s| &s.id == id)
    }

    /// **Which section carries the heat cells** (16b cut X.1): the body — the
    /// section doc 16 §3.3 calls `hull` — and the first authored section for a
    /// unit whose content calls its body something else.
    ///
    /// It is the same rule [`crate::arena::damage`]'s `wreck_ops` follows for the
    /// condition that marks a wreck, and for the same reason: one section of a
    /// unit is its body, and content that does not use the word still has one.
    /// The arena never re-derives it — it **finds** the section that carries the
    /// cell — so the rule lives here alone.
    #[must_use]
    pub fn heat_section(&self) -> Option<&SectionDef> {
        if self.heat_max == 0 {
            return None;
        }
        CanonicalId::new(crate::arena::damage::HULL_SECTION)
            .ok()
            .and_then(|hull| self.section(&hull))
            .or_else(|| self.sections.first())
    }

    /// Re-run every bound and every internal reference.
    ///
    /// # Errors
    ///
    /// [`DefinitionError::Value`] naming the entry and what is wrong.
    pub fn check(&self, entry: &str) -> Result<(), DefinitionError> {
        crate::definitions::check_label(entry, &self.label)?;
        crate::definitions::check_note(entry, &self.note)?;
        if self.sections.is_empty() || self.sections.len() > MAX_SECTIONS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "a unit declares 1 to {MAX_SECTIONS} sections; {} is not",
                    self.sections.len()
                ),
            ));
        }
        if self.components.len() > MAX_COMPONENTS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "a unit declares at most {MAX_COMPONENTS} components; {} is not",
                    self.components.len()
                ),
            ));
        }
        let mut seen: Vec<&CanonicalId> = Vec::new();
        for section in &self.sections {
            if seen.contains(&&section.id) {
                return Err(DefinitionError::value(
                    entry,
                    format!("section `{}` is declared twice", section.id),
                ));
            }
            seen.push(&section.id);
            if section.facets.len() > MAX_FACETS {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "section `{}` declares at most {MAX_FACETS} facets; {} is not",
                        section.id,
                        section.facets.len()
                    ),
                ));
            }
            if section.internals == 0 || section.internals > MAX_CELL_VALUE {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "section `{}` internals is 1 to {MAX_CELL_VALUE}; {} is not",
                        section.id, section.internals
                    ),
                ));
            }
            let mut facets: Vec<&CanonicalId> = Vec::new();
            for facet in &section.facets {
                if facets.contains(&&facet.id) {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "section `{}` declares facet `{}` twice",
                            section.id, facet.id
                        ),
                    ));
                }
                facets.push(&facet.id);
                if facet.armor == 0 || facet.armor > MAX_CELL_VALUE {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "facet `{}.{}` armor is 1 to {MAX_CELL_VALUE}; {} is not",
                            section.id, facet.id, facet.armor
                        ),
                    ));
                }
            }
            if section.consequences.is_empty() || section.consequences.len() > MAX_CONSEQUENCES {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "section `{}` declares 1 to {MAX_CONSEQUENCES} consequences; {} is not",
                        section.id,
                        section.consequences.len()
                    ),
                ));
            }
            for consequence in &section.consequences {
                match consequence {
                    Consequence::RemoveWeapons { section: named } => {
                        if self.section(named).is_none() {
                            return Err(DefinitionError::value(
                                entry,
                                format!(
                                    "section `{}`'s remove_weapons names section `{named}`, \
                                     which this unit does not declare",
                                    section.id
                                ),
                            ));
                        }
                    }
                    Consequence::Slow { percent } if *percent == 0 || *percent > 100 => {
                        return Err(DefinitionError::value(
                            entry,
                            format!(
                                "section `{}`'s slow(%) is 1 to 100; {percent} is not",
                                section.id
                            ),
                        ));
                    }
                    _ => {}
                }
            }
            if section.facets.is_empty() && section.struck_through.is_none() {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "section `{}` has no facets, so it must say where it is struck \
                         (`struck_through`)",
                        section.id
                    ),
                ));
            }
        }
        for section in &self.sections {
            let Some(through) = &section.struck_through else {
                continue;
            };
            let host = self.section(&through.section).ok_or_else(|| {
                DefinitionError::value(
                    entry,
                    format!(
                        "section `{}` is struck through `{}`, which this unit does not declare",
                        section.id, through.section
                    ),
                )
            })?;
            if !host.facets.iter().any(|f| f.id == through.facet) {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "section `{}` is struck through `{}.{}`, which is not one of that \
                         section's facets",
                        section.id, through.section, through.facet
                    ),
                ));
            }
        }
        let mut components: Vec<&CanonicalId> = Vec::new();
        for component in &self.components {
            if components.contains(&&component.id) {
                return Err(DefinitionError::value(
                    entry,
                    format!("component `{}` is declared twice", component.id),
                ));
            }
            components.push(&component.id);
            if self.section(&component.section).is_none() {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "component `{}` is installed in section `{}`, which this unit does \
                         not declare",
                        component.id, component.section
                    ),
                ));
            }
            if let ComponentRole::Power {
                states, initial, ..
            } = &component.role
            {
                if states.is_empty() || states.len() > MAX_CONSEQUENCES {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "component `{}` declares 1 to {MAX_CONSEQUENCES} power states; \
                             {} is not",
                            component.id,
                            states.len()
                        ),
                    ));
                }
                if !states.contains(initial) {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "component `{}`'s initial state `{initial}` is not one of its \
                             declared states",
                            component.id
                        ),
                    ));
                }
            }
        }
        self.check_bands(entry)?;
        self.check_heat(entry)?;
        if self.sensor_height_cm == 0 || self.sensor_height_cm > MAX_SENSOR_HEIGHT_CM {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "sensor_height_cm is 1 to {MAX_SENSOR_HEIGHT_CM}; {} is not",
                    self.sensor_height_cm
                ),
            ));
        }
        Ok(())
    }

    /// **The two heat numbers** (16b cut X.1): both or neither, and each inside
    /// the cell bound its cell is built with.
    ///
    /// Both-or-neither is the interesting half. A unit with a ceiling and no
    /// dissipation heats to it and refuses `FIRE` for the rest of the match with
    /// nothing a pilot can do; a unit with dissipation and no ceiling carries a
    /// rate nothing reads, because the cells are laid down off the ceiling. Both
    /// are authoring mistakes and both are refused at load rather than discovered
    /// in a match.
    fn check_heat(&self, entry: &str) -> Result<(), DefinitionError> {
        if (self.heat_max == 0) != (self.heat_dissipation_per_s == 0) {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "a unit declares heat_max and heat_dissipation_per_s together or not at \
                     all; {} and {} do not",
                    self.heat_max, self.heat_dissipation_per_s
                ),
            ));
        }
        if self.heat_max > MAX_CELL_VALUE || self.heat_dissipation_per_s > MAX_CELL_VALUE {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "heat_max and heat_dissipation_per_s are at most {MAX_CELL_VALUE}; \
                     {} and {} are not",
                    self.heat_max, self.heat_dissipation_per_s
                ),
            ));
        }
        Ok(())
    }

    /// The facet table and the silhouettes: every band names sections this unit
    /// has, every band presents something, and the two tables name the same set
    /// of bands.
    fn check_bands(&self, entry: &str) -> Result<(), DefinitionError> {
        if self.facets.is_empty() || self.facets.len() > MAX_FACETS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "a unit's facet table is 1 to {MAX_FACETS} bands; {} is not",
                    self.facets.len()
                ),
            ));
        }
        let mut bands: Vec<&CanonicalId> = Vec::new();
        for band in &self.facets {
            if bands.contains(&&band.band) {
                return Err(DefinitionError::value(
                    entry,
                    format!("facet band `{}` is declared twice", band.band),
                ));
            }
            bands.push(&band.band);
            if band.sections.is_empty() {
                return Err(DefinitionError::value(
                    entry,
                    format!("facet band `{}` strikes nothing", band.band),
                ));
            }
            let mut total = 0_u32;
            for weight in &band.sections {
                if self.section(&weight.section).is_none() {
                    return Err(DefinitionError::value(
                        entry,
                        format!(
                            "facet band `{}` names section `{}`, which this unit does not \
                             declare",
                            band.band, weight.section
                        ),
                    ));
                }
                total = total.saturating_add(u32::from(weight.weight));
            }
            if total == 0 {
                return Err(DefinitionError::value(
                    entry,
                    format!("facet band `{}`'s weights are all zero", band.band),
                ));
            }
        }
        if self.silhouettes.len() != self.facets.len() {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "a unit presents one silhouette per facet band; {} bands and {} \
                     silhouettes do not match",
                    self.facets.len(),
                    self.silhouettes.len()
                ),
            ));
        }
        for silhouette in &self.silhouettes {
            if !bands.contains(&&silhouette.band) {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "silhouette `{}` names a band the facet table does not declare",
                        silhouette.band
                    ),
                ));
            }
            if silhouette.width_cm == 0
                || silhouette.height_cm == 0
                || silhouette.width_cm > MAX_SILHOUETTE_CM
                || silhouette.height_cm > MAX_SILHOUETTE_CM
            {
                return Err(DefinitionError::value(
                    entry,
                    format!(
                        "silhouette `{}` presents 1 to {MAX_SILHOUETTE_CM} cm each way; \
                         {}×{} does not",
                        silhouette.band, silhouette.width_cm, silhouette.height_cm
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// **The `tracked` mode's rules, read from the authored spec** (doc 16 §3.3).
///
/// The twin of [`crate::match_::tracked_rules`], which reads the same five
/// numbers out of an instantiated unit's opaque [`ModeRules`]. [`bind`] is what
/// makes them agree: it writes exactly these five keys into the blob, so for
/// every unit made from a spec the two functions return the same value. A test
/// pins that.
#[must_use]
pub fn tracked_rules(spec: &UnitSpec) -> TrackedRules {
    TrackedRules {
        max_speed_cms: spec.mobility.tracked.max_speed_cms,
        accel_cms2: spec.mobility.tracked.accel_cms2,
        brake_cms2: spec.mobility.tracked.brake_cms2,
        turn_rate_mdeg_s: spec.mobility.tracked.turn_rate_mdeg_s,
        reverse_factor_pct: spec.mobility.tracked.reverse_factor_pct.min(100),
    }
}

// ---------------------------------------------------------------------------
// Binding: the authored shape to M32's
// ---------------------------------------------------------------------------

/// What binding one unit produces: the M32 spec, and every descriptor whose
/// registration is what makes the spec's references exact.
///
/// The descriptors are in dependency order — schemas, then components, then the
/// unit itself — which is the order a registrar must take them in for the
/// registry's own dependency check to pass at freeze.
#[derive(Clone, Debug)]
pub struct UnitBinding {
    /// The instantiation shape, every reference exact.
    pub spec: mush_tactics::UnitSpec,
    /// The descriptors, in dependency order. The last is the unit's, marked
    /// current, which is what `@unit/create <obj>=<id>` resolves through.
    pub descriptors: Vec<DefinitionDescriptor>,
}

/// What [`bind`] needs from the rest of the pack.
#[derive(Clone, Copy, Debug)]
pub struct UnitContext<'a> {
    /// Every ammunition entry, by id — a weapon component's `ammo` cell is
    /// sized from the ammunition its weapon names.
    pub ammunition: &'a BTreeMap<CanonicalId, AmmunitionDef>,
    /// Every weapon entry, by id.
    pub weapons: &'a BTreeMap<CanonicalId, WeaponDef>,
    /// Every sensor entry, by id.
    pub sensors: &'a BTreeMap<CanonicalId, SensorDef>,
    /// The already-minted reference of every weapon entry.
    pub weapon_refs: &'a BTreeMap<CanonicalId, DefinitionRef>,
    /// The already-minted reference of every sensor entry.
    pub sensor_refs: &'a BTreeMap<CanonicalId, DefinitionRef>,
    /// The already-minted `schema` references, by id.
    pub schemas: &'a BTreeMap<&'static str, DefinitionRef>,
}

/// **Turn the authored spec into M32's, minting every reference it needs.**
///
/// `builder` is a scratch registry with the module's own owner id: registering
/// a descriptor in it computes exactly the [`DefinitionRef`]
/// `Registrar::definition` would, because a definition's digest is a function of
/// `(owner, kind, id, version, payload, references)` and nothing else. The
/// caller registers [`UnitBinding::descriptors`] for real; the refs are the same
/// values either way, which is the whole reason this is one function.
///
/// # Errors
///
/// [`DefinitionError`] for a spec that names something the pack does not carry,
/// a malformed id, or a registry bound.
pub fn bind(
    minter: &mut crate::definitions::Minter<'_>,
    entry: &CanonicalId,
    version: u32,
    spec: &UnitSpec,
    cx: UnitContext<'_>,
) -> Result<UnitBinding, DefinitionError> {
    let name = entry.as_str();
    let mut descriptors: Vec<DefinitionDescriptor> = Vec::new();

    // (1) The sections, their facets and their cells.
    //
    // **16b cut X.1**: the body section also carries the two heat cells, and only
    // when the spec declares a ceiling — which is why a `line_tank` bound after
    // this cut is byte-identical to one bound before it, down to the digest its
    // persisted units point at (no migration; 16b §8 cut X).
    let heated = spec.heat_section().map(|section| section.id.clone());
    let mut sections: Vec<SectionSpec> = Vec::new();
    for section in &spec.sections {
        let mut facets: Vec<FacetSpec> = Vec::new();
        for facet in &section.facets {
            facets.push(FacetSpec {
                id: tactics_id!(name, "facet", facet.id.as_str(), FacetId)?,
                cells: vec![CellSpec {
                    id: tactics_id!(name, "cell", ARMOR_CELL, CellId)?,
                    min: 0,
                    max: i64::from(facet.armor),
                    init: i64::from(facet.armor),
                }],
            });
        }
        let mut cells = vec![CellSpec {
            id: tactics_id!(name, "cell", INTERNALS_CELL, CellId)?,
            min: 0,
            max: i64::from(section.internals),
            init: i64::from(section.internals),
        }];
        if heated.as_ref() == Some(&section.id) {
            // The ceiling **is** the cell's maximum, so there is no second number
            // to keep in step with it and no plan this crate builds can ask M32
            // for a value outside the cell (doc 16 §3.5's refuse-never-clamp, read
            // for heat).
            cells.push(CellSpec {
                id: tactics_id!(name, "cell", HEAT_CELL, CellId)?,
                min: 0,
                max: i64::from(spec.heat_max),
                init: 0,
            });
            cells.push(CellSpec {
                id: tactics_id!(name, "cell", HEAT_SINK_CELL, CellId)?,
                min: 0,
                max: i64::from(spec.heat_dissipation_per_s),
                init: i64::from(spec.heat_dissipation_per_s),
            });
        }
        sections.push(SectionSpec {
            id: tactics_id!(name, "section", section.id.as_str(), SectionId)?,
            condition: tactics_id!(name, "condition", INTACT, ConditionId)?,
            facets,
            slots: Vec::new(),
            cells,
        });
    }

    // (2) The components. Each gets a `component` definition of its own, whose
    // references are the weapon or sensor row it names: that is what makes a
    // persisted unit point at the exact weapon it carries.
    let mut components: Vec<ComponentSpec> = Vec::new();
    let mut capabilities: Vec<CapabilityInit> = Vec::new();
    for component in &spec.components {
        let component_id = format!("{name}.{}", component.id);
        let (payload, references, cells) =
            match &component.role {
                ComponentRole::Weapon { weapon, coaxial } => {
                    let def = cx.weapons.get(weapon).ok_or_else(|| {
                        DefinitionError::reference(name, "weapon", weapon.as_str())
                    })?;
                    let rounds = cx
                        .ammunition
                        .get(&def.ammunition)
                        .ok_or_else(|| {
                            DefinitionError::reference(name, "ammunition", def.ammunition.as_str())
                        })?
                        .count_per_load;
                    let weapon_ref = cx.weapon_refs.get(weapon).ok_or_else(|| {
                        DefinitionError::reference(name, "weapon", weapon.as_str())
                    })?;
                    let mut cells = vec![
                        CellSpec {
                            id: tactics_id!(name, "cell", AMMO_CELL, CellId)?,
                            min: 0,
                            max: i64::from(rounds),
                            init: i64::from(rounds),
                        },
                        CellSpec {
                            id: tactics_id!(name, "cell", READY_AT_CELL, CellId)?,
                            min: 0,
                            max: MAX_READY_AT_MS,
                            init: 0,
                        },
                    ];
                    // **16b cut X.1**: what one shot of this weapon adds, and only
                    // for a weapon that authors any — so both stock weapons'
                    // components are what they were.
                    if def.heat_per_shot > 0 {
                        cells.push(CellSpec {
                            id: tactics_id!(name, "cell", HEAT_PER_SHOT_CELL, CellId)?,
                            min: 0,
                            max: i64::from(def.heat_per_shot),
                            init: i64::from(def.heat_per_shot),
                        });
                    }
                    (
                        serde_json::json!({
                            "role": "weapon",
                            "weapon": weapon.as_str(),
                            "coaxial": coaxial,
                        }),
                        vec![weapon_ref.clone()],
                        cells,
                    )
                }
                ComponentRole::Sensor { sensor } => {
                    if !cx.sensors.contains_key(sensor) {
                        return Err(DefinitionError::reference(name, "sensor", sensor.as_str()));
                    }
                    let sensor_ref = cx.sensor_refs.get(sensor).ok_or_else(|| {
                        DefinitionError::reference(name, "sensor", sensor.as_str())
                    })?;
                    (
                        serde_json::json!({ "role": "sensor", "sensor": sensor.as_str() }),
                        vec![sensor_ref.clone()],
                        vec![CellSpec {
                            id: tactics_id!(name, "cell", CONDITION_CELL, CellId)?,
                            min: 0,
                            max: MAX_CONDITION,
                            init: MAX_CONDITION,
                        }],
                    )
                }
                ComponentRole::Power {
                    capability,
                    states,
                    initial,
                } => {
                    let schema = schema_ref(name, cx.schemas, POWER_RULES_SCHEMA)?;
                    let words: Vec<&str> = states.iter().map(CanonicalId::as_str).collect();
                    capabilities.push(CapabilityInit {
                        key: CapabilityKey {
                            id: tactics_id!(name, "capability", capability.as_str(), CapabilityId)?,
                            source: Some(tactics_id!(
                                name,
                                "component",
                                component.id.as_str(),
                                ComponentId
                            )?),
                        },
                        rules: RulesValue::new(
                            schema,
                            serde_json::json!({ "states": words, "state": initial.as_str() }),
                        )
                        .map_err(|error| DefinitionError::value(name, error.to_string()))?,
                    });
                    (
                        serde_json::json!({
                            "role": "power",
                            "capability": capability.as_str(),
                            "states": states.iter().map(CanonicalId::as_str).collect::<Vec<_>>(),
                            "initial": initial.as_str(),
                        }),
                        Vec::new(),
                        Vec::new(),
                    )
                }
            };
        let descriptor = descriptor(
            "component",
            &component_id,
            version,
            payload,
            references.clone(),
        )
        .map_err(|error| DefinitionError::value(name, error))?;
        let reference = register(minter, descriptor.clone(), name)?;
        descriptors.push(descriptor);
        components.push(ComponentSpec {
            id: tactics_id!(name, "component", component.id.as_str(), ComponentId)?,
            definition: reference,
            install: InstallPoint::Section(tactics_id!(
                name,
                "section",
                component.section.as_str(),
                SectionId
            )?),
            condition: tactics_id!(name, "condition", INTACT, ConditionId)?,
            cells,
            asset: None,
        });
    }

    // (3) The one mobility mode.
    let tracked = tracked_rules(spec);
    let mode = MobilityMode::new(
        tactics_id!(name, "mobility mode", TRACKED_MODE, MobilityModeId)?,
        [tactics_id!(name, "medium", GROUND, MediumId)?],
        tactics_id!(name, "locomotion", TRACKED, LocomotionId)?,
        ModeRules::new(
            schema_ref(name, cx.schemas, TRACKED_RULES_SCHEMA)?,
            serde_json::json!({
                "max_speed_cms": tracked.max_speed_cms,
                "accel_cms2": tracked.accel_cms2,
                "brake_cms2": tracked.brake_cms2,
                "turn_rate_mdeg_s": tracked.turn_rate_mdeg_s,
                "reverse_factor_pct": tracked.reverse_factor_pct,
            }),
        )
        .map_err(|error| DefinitionError::value(name, error.to_string()))?,
    )
    .with_speeds(tracked.max_speed_cms, 0);
    let profile = MobilityProfile::new(
        vec![mode],
        tactics_id!(name, "mobility mode", TRACKED_MODE, MobilityModeId)?,
        &MobilityVocabulary::base().freeze(),
    )
    .map_err(|error| DefinitionError::value(name, error.to_string()))?;

    // (4) The game's own half, carried opaque (doc 16 §3.3: M32 interprets
    // nothing game-specific).
    let rules = RulesValue::new(
        schema_ref(name, cx.schemas, UNIT_RULES_SCHEMA)?,
        game_rules(spec),
    )
    .map_err(|error| DefinitionError::value(name, error.to_string()))?;

    let body = UnitSpecBody {
        sections,
        unit_cells: Vec::new(),
        components,
        capabilities,
        mobility: Some(profile),
        rules,
    };
    let unit_descriptor = body
        .descriptor(
            DefinitionKind::new("unit")
                .map_err(|error| DefinitionError::value(name, error.to_string()))?,
            DefinitionId::new(name)
                .map_err(|error| DefinitionError::value(name, error.to_string()))?,
            version,
        )
        .map_err(|error| DefinitionError::value(name, error.to_string()))?
        // `@unit/create` resolves a bare schema id through the registry's own
        // current selection, so the shipped version must say it is the one.
        .as_current();
    let reference = register(minter, unit_descriptor.clone(), name)?;
    descriptors.push(unit_descriptor);
    Ok(UnitBinding {
        spec: body.with_definition(reference),
        descriptors,
    })
}

/// **Instantiate `object` as this unit** (doc 16 §3.3; the arena pack's applier
/// and cut K's reset are the callers).
///
/// `binding` is [`bind`]'s output for `spec`, which is what carries the exact
/// registry identity a [`TacticalUnit`] must point at; the instantiation itself
/// is M32's own [`TacticalUnit::from_spec`] and nothing here reimplements it.
///
/// # Errors
///
/// [`DefinitionError::Value`] naming the spec and M32's refusal.
pub fn to_tactical_unit(
    spec: &UnitSpec,
    binding: &UnitBinding,
    object: ObjId,
) -> Result<TacticalUnit, DefinitionError> {
    TacticalUnit::from_spec(&binding.spec, object)
        .map_err(|error| DefinitionError::value(&spec.label, error.to_string()))
}

/// The opaque blob M32 carries and only this crate reads.
fn game_rules(spec: &UnitSpec) -> serde_json::Value {
    let consequences: Vec<serde_json::Value> = spec
        .sections
        .iter()
        .map(|section| {
            serde_json::json!({
                "section": section.id.as_str(),
                "on_zero_internals": section.consequences,
                "struck_through": section.struck_through,
            })
        })
        .collect();
    serde_json::json!({
        "facets": spec.facets,
        "silhouettes": spec.silhouettes,
        "sensor_height_cm": spec.sensor_height_cm,
        "consequences": consequences,
        "components": spec.components,
        "mobility": spec.mobility,
    })
}

/// One `schema` reference the binding needs, or a refusal naming it.
fn schema_ref(
    entry: &str,
    schemas: &BTreeMap<&'static str, DefinitionRef>,
    id: &'static str,
) -> Result<DefinitionRef, DefinitionError> {
    schemas
        .get(id)
        .cloned()
        .ok_or_else(|| DefinitionError::reference(entry, "schema", id))
}

/// A descriptor from its parts.
pub(crate) fn descriptor(
    kind: &str,
    id: &str,
    version: u32,
    payload: serde_json::Value,
    references: Vec<DefinitionRef>,
) -> Result<DefinitionDescriptor, String> {
    if references.len() > MAX_ENTRY_REFERENCES {
        return Err(format!(
            "a definition names at most {MAX_ENTRY_REFERENCES} others; {} does not",
            references.len()
        ));
    }
    let kind = DefinitionKind::new(kind).map_err(|error| error.to_string())?;
    let id = DefinitionId::new(id).map_err(|error| error.to_string())?;
    DefinitionDescriptor::new(kind, id, version, payload)
        .map(|descriptor| descriptor.with_references(references))
        .map_err(|error| error.to_string())
}

/// Register one descriptor in the scratch registry, which is how its exact
/// reference is computed.
pub(crate) fn register(
    minter: &mut crate::definitions::Minter<'_>,
    descriptor: DefinitionDescriptor,
    entry: &str,
) -> Result<DefinitionRef, DefinitionError> {
    let owner = minter.owner.clone();
    minter
        .scratch
        .register(owner, descriptor)
        .map_err(|error| DefinitionError::value(entry, error.to_string()))
}
