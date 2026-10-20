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
//! integers and names. It is the shape of **any** unit, not only the tank the
//! module is named for — 16b cut X.2's biped is six other sections and a `legged`
//! mode through the very same decoder.
//!
//! [`bind`] is the one function that turns the second into
//! the first, minting every reference through a scratch
//! [`mush_core::DefinitionRegistryBuilder`] with the same owner the module registers under,
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
use std::sync::LazyLock;

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
use crate::definitions::sensor::{
    MAX_ATTENUATION_COST, MAX_JAM_RADIUS_CELLS, MAX_SIGNATURE_PERMILLE, SensorDef, ThermalSignature,
};
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

/// The `schema` definition a **legged** mode's opaque rules blob is bound to
/// (16b cut X.2; doc 16 §9 item 2: *a `legged` mobility mode*).
///
/// A second id for the same five numbers, and deliberately: a schema is what a
/// blob *is*, and a walking machine's kinematics are not a tracked hull's however
/// alike the two tables look. [`tracked_rules`] decodes either into one
/// [`TrackedRules`], which is where the likeness belongs.
pub const LEGGED_RULES_SCHEMA: &str = "tacsim.mobility.legged";

/// The `schema` definition a **wheeled** mode's opaque rules blob is bound to
/// (16b cut AE.1): the same five numbers again, under a third id, for the
/// reason [`LEGGED_RULES_SCHEMA`] gives.
pub const WHEELED_RULES_SCHEMA: &str = "tacsim.mobility.wheeled";

/// The `schema` definition the power capability's rules blob is bound to.
pub const POWER_RULES_SCHEMA: &str = "tacsim.power.states";

/// **The capability a core grants** (17b cut M3.1; doc 17 §9, D-UM-33): its
/// key's source is the core's component, and its rules payload is the
/// fitting's record ([`crate::definitions::core::profile_payload`]). What
/// [`crate::arena::core::read_core`] looks for: a hull without it is Class 0.
pub const CORE_CAPABILITY: &str = "tacsim.core";

/// **The cell on a core's component that holds its class now** (17b cut
/// M3.1; D-UM-33, D-UM-34): `0` to `3`, born at the fitted class's ordinal —
/// the live class [`crate::arena::core::read_core`] reads, and what 17b cut
/// M3.4's `degrade_core` lowers.
pub const CORE_CLASS_CELL: &str = "core_class";

/// **The `schema` definition the core capability's rules blob is bound to**
/// (17b cut M3.1): the sixth minted schema, appended last
/// (`crate::definitions`' `schema_payloads`), so no schema minted before it
/// moves.
pub const CORE_RULES_SCHEMA: &str = "tacsim.core.profile";

/// The locomotion id doc 16 §3.3's mode rules belong to — the atlas's own
/// (`mush_atlas::physics::BASE_LOCOMOTIONS`), so the arena's move costs and the
/// unit's mobility name the same thing.
pub const TRACKED: &str = "tracked";

/// The locomotion id a legged mode belongs to (16b cut X.2) — the atlas's own,
/// beside [`TRACKED`] in `mush_atlas::physics::BASE_LOCOMOTIONS`, so a biped and
/// a tank name their ground the same way.
pub const LEGGED: &str = "legged";

/// The locomotion id a wheeled mode belongs to (16b cut AE.1) — the atlas's
/// own, beside [`TRACKED`] and [`LEGGED`] in
/// `mush_atlas::physics::BASE_LOCOMOTIONS`, so a car prices its ground off the
/// same table row the atlas does.
pub const WHEELED: &str = "wheeled";

/// The medium a tracked mode moves in.
pub const GROUND: &str = "ground";

/// The mobility mode id the stock tank carries.
pub const TRACKED_MODE: &str = "ground.tracked";

/// The mobility mode id a biped carries (16b cut X.2).
pub const LEGGED_MODE: &str = "ground.legged";

/// The mobility mode id a wheeled car carries (16b cut AE.1).
pub const WHEELED_MODE: &str = "ground.wheeled";

/// The armour cell every facet carries.
pub const ARMOR_CELL: &str = "armor";

/// The cell every section carries under its armour.
pub const INTERNALS_CELL: &str = "internals";

/// **[`ARMOR_CELL`] as a [`CellId`], built once for the process** (16c
/// T-110a).
///
/// `CellId::new` runs `mush_tactics`' `canon` — a trim, an ASCII lowercase and
/// a fresh `String` — on every call (`canon` itself 2.1–3.6 per cent of the
/// step at the hash v2 tree, over every caller), and the arena was running it
/// on a constant: twice per pilot frame and per mind view in
/// `Arena::frame_sections`, once or twice a round in `Arena::plan_hit` and
/// `Arena::struck_maxima`, and a reading in `Condition::of`. The id is now
/// built on the first read and shared from then on.
///
/// `None` only if `canon` ever refused the constant, which it does not
/// (`t110a_the_once_built_ids_are_the_constants`); the readers keep the
/// refusal they had, so a `None` here is exactly the `Err` the per-call build
/// would have answered, and no reader's behaviour moves.
#[must_use]
pub fn armor_cell() -> Option<&'static CellId> {
    static ARMOR: LazyLock<Option<CellId>> = LazyLock::new(|| CellId::new(ARMOR_CELL).ok());
    ARMOR.as_ref()
}

/// **[`INTERNALS_CELL`] as a [`CellId`], built once for the process** (16c
/// T-110a) — [`armor_cell`]'s twin, for the same readers and on the same
/// terms.
#[must_use]
pub fn internals_cell() -> Option<&'static CellId> {
    static INTERNALS: LazyLock<Option<CellId>> = LazyLock::new(|| CellId::new(INTERNALS_CELL).ok());
    INTERNALS.as_ref()
}

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

/// **What a thermal sight makes of a powered, stopped target** (16b cut X.3;
/// doc 16 §9 item 3), per mille, on the *sensor's* component.
///
/// Laid down only when the mount authors a
/// [`crate::definitions::sensor::ThermalSignature`], so a unit that carries the
/// stock sight — every shipped unit — carries no such cell and is byte-identical
/// to what it was. The cell's `max` is the factor of a target that hides nothing,
/// so a later cut can *degrade* a sight's thermal channel the way a hit degrades
/// its [`CONDITION_CELL`], with no new seam.
pub const THERMAL_POWERED_CELL: &str = "thermal_powered";

/// **What it makes of a target whose power is not online** (16b cut X.3), beside
/// [`THERMAL_POWERED_CELL`] on the same component.
///
/// The arena reads both at check-out exactly as it reads the armour, the magazine
/// and [`HEAT_PER_SHOT_CELL`]: the alternative is a new field on the check-out's
/// sensor row, and that row is a struct seven test files build literally.
pub const THERMAL_COLD_CELL: &str = "thermal_cold";

/// **How far a jamming pod's bubble reaches**, in cells, on the *pod's* component
/// (16b cut X.4; doc 16 §9 item 4).
///
/// Laid down only for a [`ComponentRole::Ecm`] component, whose sensor row
/// carries a [`crate::definitions::sensor::JammingDef`] — so every unit the game
/// ships carries no such cell and is byte-identical to what it was. The three jam
/// cells take the route 16b cut X.1's `heat_per_shot` and cut X.3's thermal band
/// take, and for the same reason: the arena's check-out rows
/// ([`crate::arena::state::CustodyUnit`], `SensorProfile`) are structs a dozen
/// test files build as exhaustive literals, and the numbers ride on the unit
/// rather than across a widened seam.
///
/// In **cells** and not centimetres, because cells are what content authors and
/// what [`crate::definitions::sensor::SensorDef::max_range_cells`] already is; the
/// arena multiplies by the window's own `cell_cm` once, at check-out.
pub const JAM_RADIUS_CELL: &str = "jam_radius_cells";

/// **What the bubble takes out of an observer's attenuation budget** (16b cut
/// X.4), beside [`JAM_RADIUS_CELL`] on the same component.
///
/// The cell's `max` is the whole budget, so a later cut can *degrade* a pod the
/// way a hit degrades a sight's [`CONDITION_CELL`], with no new seam.
pub const JAM_ATTENUATION_CELL: &str = "jam_attenuation_cost";

/// **What the bubble leaves of a target's signature factor**, per mille (16b cut
/// X.4), beside the other two on the same component.
pub const JAM_SIGNATURE_CELL: &str = "jam_signature_permille";

/// **What a filter buys back of an observer's attenuation budget**, on the atlas's
/// 0-to-100 scale, on the *filter's* component (16b cut X.5; doc 16 §9 item 5).
///
/// Laid down only for a [`ComponentRole::Eccm`] component, whose sensor row carries
/// an [`crate::definitions::sensor::EccmDef`] — so every unit the game ships carries
/// no such cell and is byte-identical to what it was. The two ECCM cells take the
/// route cut X.1's `heat_per_shot`, cut X.3's thermal band and cut X.4's three jam
/// cells take, for the reason [`JAM_RADIUS_CELL`] gives.
///
/// The cell's `max` is the whole of the budget a bubble can eat, so a later cut can
/// *degrade* a filter the way a hit degrades a sight's [`CONDITION_CELL`], with no
/// new seam.
pub const ECCM_ATTENUATION_CELL: &str = "eccm_attenuation_recovery";

/// **What it adds back to a target's signature factor**, per mille (16b cut X.5),
/// beside [`ECCM_ATTENUATION_CELL`] on the same component.
pub const ECCM_SIGNATURE_CELL: &str = "eccm_signature_recovery_permille";

/// The condition a section, and a component, is born in.
pub const INTACT: &str = "intact";

/// The ceiling of a `ready_at_ms` cell: an hour of arena time — four times the
/// stock `match.max_duration_ms` (fifteen minutes since 16b cut AP.2) and the
/// whole of the hour it was before — so at the stock cap a cycle can never be
/// set past the match that contains it. `MatchConfig::validate` puts a floor
/// on the cap and no ceiling, so an operator who configures a match past the
/// hour is past this bound too: not enforced, a 16c debt.
pub const MAX_READY_AT_MS: i64 = 3_600_000;

/// The ceiling of a sensor's `condition` cell: a percentage.
pub const MAX_CONDITION: i64 = 100;

/// The most armour or internals one cell may hold.
pub const MAX_CELL_VALUE: u32 = 100_000;

/// The most sections one unit may declare here. M32's own cap is far higher;
/// this is the authoring bound, because every section is a row of the facet
/// table and a weight in every band.
///
/// **16b cut Q.2 tightened it from 32 to 16**, and named the reason: a gunner's
/// reading of a target ([`crate::arena::condition::Condition`]) is two bits a
/// section packed into one `u32`, so sixteen is what a contact can carry without
/// an allocation per contact per sensor pass. The clamp in `Condition::of` is
/// defensive; **this** is the rule, so no unit an arena can hold ever reaches it.
/// Nothing the game ships comes near either bound — the stock tank declares five
/// and the extension biped seven.
pub const MAX_SECTIONS: usize = crate::arena::condition::MAX_SCANNED_SECTIONS;

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
    /// **The unit cannot move once every one of `sections` is destroyed** (16b
    /// cut X.2; doc 16 §9 item 2: *`slow` for one leg destroyed, `disable_motion`
    /// for both*).
    ///
    /// The one consequence whose effect is not a function of the section it is
    /// authored on alone, and the reason it is a kind of its own rather than a
    /// rule about the word `leg` in [`crate::arena::damage`]: a biped's legs are
    /// two sections that are lethal together and survivable apart, and the pair is
    /// the content's statement about its own anatomy. Nothing about it is legs —
    /// a four-legged walker names four sections, a unit with two engines names
    /// two, and the vocabulary stays closed either way.
    ///
    /// It fires from the section that carries it, at the moment that section's
    /// internals reach zero, and does nothing until every named section is
    /// [`crate::arena::damage::DESTROYED`]; authoring it on **both** legs is what
    /// makes the order they are lost in immaterial.
    DisableMotionWhenAllOf {
        /// The sections that must all be destroyed, named in this unit's own
        /// section table.
        sections: Vec<CanonicalId>,
    },
    /// **The hull's core loses one class** (17b cut M3.4; doc 17 §9,
    /// D-UM-34): `{"kind": "degrade_core"}`. The engine lowers the core
    /// component's `core_class` cell by one, never under its floor; the
    /// shipped chassis author it from 17b cut M3.6 (the heavy's turret).
    DegradeCore,
    /// **The hull's core is destroyed** (17b cut M3.4; doc 17 §9, D-UM-34):
    /// `{"kind": "destroy_core"}`. The engine revokes the `tacsim.core`
    /// capability and a mind on the hull drops to the ROM; the shipped
    /// chassis author it from 17b cut M3.6 (the line tanks' and the scout's
    /// turret).
    DestroyCore,
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
            Consequence::DisableMotionWhenAllOf { .. } => "disable_motion_when_all_of",
            Consequence::DegradeCore => "degrade_core",
            Consequence::DestroyCore => "destroy_core",
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
        /// **What this sight makes of a target's heat** (16b cut X.3; doc 16 §9
        /// item 3), or `None` for a sight with no thermal channel — every stock
        /// mount, which is why a `line_tank` bound after this cut is the bytes
        /// it was before it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thermal: Option<ThermalSignature>,
    },
    /// **A jamming pod** (16b cut X.4; doc 16 §9 item 4): a `condition` cell and
    /// the three jam cells, from the `jamming` block of the sensor row it names.
    ///
    /// A role of its own rather than a second [`ComponentRole::Sensor`], because a
    /// pod is not something the unit *sees with*: `sensor_profile` reads a unit's
    /// first sensor mount, and a pod mounted as a sight would either shadow the
    /// sight or depend on which was authored first — which is exactly the *silent
    /// `max`* [`crate::match_::sensor_profile`]'s own note refuses. It
    /// names a sensor row all the same, because doc 16 §3.6 puts `jamming` on that
    /// row and this cut does not move it.
    Ecm {
        /// The sensor row whose `jamming` block this pod radiates — an entry id,
        /// checked at load, and refused at bind time if the row carries no
        /// `jamming`.
        sensor: CanonicalId,
    },
    /// **A counter-jamming filter** (16b cut X.5; doc 16 §9 item 5): a `condition`
    /// cell and the two ECCM cells, from the `emission` block of the sensor row it
    /// names.
    ///
    /// A role of its own rather than a second [`ComponentRole::Sensor`], for the
    /// reason [`ComponentRole::Ecm`] is one: a filter is not something the unit
    /// *sees with* — it is fitted to the sight the unit already has — and a second
    /// sensor mount would shadow that sight or not, depending on which was authored
    /// first. It names a sensor row all the same, because doc 16 §3.6 puts
    /// `emission` on that row and this cut does not move it.
    Eccm {
        /// The sensor row whose `emission` block this filter applies — an entry id,
        /// checked at load, and refused at bind time if the row carries no filter
        /// block.
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
    /// **A core** (17b cut M3.1; doc 17 §9, D-UM-33, D-UM-34): the hardware a
    /// mind thinks on, of the class the [`crate::definitions::core::CoreDef`]
    /// it names declares. Bound as a component carrying one
    /// [`CORE_CLASS_CELL`] and a [`CORE_CAPABILITY`] sourced on it, beside
    /// [`ComponentRole::Power`], the other role that grants a capability. A
    /// unit carries at most one ([`UnitSpec::check`]); a unit with none is
    /// Class 0, the ROM alone.
    Core {
        /// The core definition it is — an entry id, checked at load.
        core: CanonicalId,
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

/// One mobility mode's rules as authored (doc 16 §3.3 "Mobility mode rules").
///
/// The five numbers doc 16 names, and the shape of **every** mode's blob: the
/// `legged` mode 16b cut X.2 adds is the same five, under its own
/// [`LEGGED_RULES_SCHEMA`] and its own locomotion. The name is the tracked
/// mode's because the tracked mode is the one doc 16 §3.3 authors.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackedDef {
    /// The forward ceiling, cm/s. **Nought is legal** and [`UnitSpec::check`]
    /// puts no floor under it: a hull that stands where it is put and turns in
    /// place (16b cut AA.1's immobile reading) — `tacsim.unit.target_mk1`, the
    /// range's target (16b cut O.3), is the one the game ships.
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

/// **A legged mode's rules as authored** (16b cut X.2; doc 16 §9 item 2), the
/// same five numbers under their own mode.
pub type LeggedDef = TrackedDef;

/// **A wheeled mode's rules as authored** (16b cut AE.1), the same five numbers
/// under their own mode.
pub type WheeledDef = TrackedDef;

/// **How a unit gets about** (doc 16 §3.3 "Mobility mode rules"), one entry per
/// declared mode.
///
/// **16b cut X.2** made both fields optional and added `legged`: a biped has no
/// tracks and a tank has no legs, so neither can be the field every unit fills.
/// Each is `skip_serializing_if`, so `line_tank`'s `{"tracked": {…}}` serialises
/// exactly as it did before the second mode existed — which is what keeps every
/// shipped `DefinitionRef` and every persisted unit pointing where it was, with
/// no migration (16b §8 cut X).
///
/// **16b cut AE.1** added `wheeled` the same way, between the two: a third
/// `skip_serializing_if`, so every pack that authors no wheels serialises its
/// `game_rules` blob, and therefore every existing `DefinitionRef`, byte for
/// byte as before.
///
/// [`UnitSpec::check`] refuses a unit that declares none, which is why there
/// is no [`Default`]: a `MobilityDef` with every field empty is not a unit that
/// stands still, it is a unit the loader will not have.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MobilityDef {
    /// The tracked mode, for a unit that has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracked: Option<TrackedDef>,
    /// **The wheeled mode** (16b cut AE.1), for a unit that drives on wheels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wheeled: Option<WheeledDef>,
    /// **The legged mode** (16b cut X.2), for a unit that walks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legged: Option<LeggedDef>,
}

/// One declared mode, as [`bind`] and [`tracked_rules`] read it: everything
/// needed to mint a [`MobilityMode`], and the decoded five.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModeDef {
    /// The `mush-tactics` mode id ([`TRACKED_MODE`], [`WHEELED_MODE`] or
    /// [`LEGGED_MODE`]).
    pub mode: &'static str,
    /// The atlas locomotion ([`TRACKED`], [`WHEELED`] or [`LEGGED`]).
    pub locomotion: &'static str,
    /// The `schema` its opaque blob is bound to.
    pub schema: &'static str,
    /// The five numbers.
    pub rules: TrackedRules,
}

impl MobilityDef {
    /// **Every mode this unit declares**, tracked first (16b cut X.2), then
    /// wheeled (16b cut AE.1), then legged.
    ///
    /// The order is the authoring order of the struct and it is load-bearing
    /// twice: the first mode is the [`MobilityProfile`]'s default, and it is what
    /// [`tracked_rules`] answers with. A unit that declared both would drive as a
    /// tank and be *able* to walk, which is the only reading of "default" the
    /// arena has a use for — doc 16 §3.4 integrates one mode.
    #[must_use]
    pub fn modes(&self) -> Vec<ModeDef> {
        let mut out: Vec<ModeDef> = Vec::new();
        if let Some(tracked) = self.tracked {
            out.push(ModeDef {
                mode: TRACKED_MODE,
                locomotion: TRACKED,
                schema: TRACKED_RULES_SCHEMA,
                rules: rules_of(tracked),
            });
        }
        // 16b cut AE.1: wheels between the two, the order
        // `crate::arena::slope::driving_mode`'s ladder climbs.
        if let Some(wheeled) = self.wheeled {
            out.push(ModeDef {
                mode: WHEELED_MODE,
                locomotion: WHEELED,
                schema: WHEELED_RULES_SCHEMA,
                rules: rules_of(wheeled),
            });
        }
        if let Some(legged) = self.legged {
            out.push(ModeDef {
                mode: LEGGED_MODE,
                locomotion: LEGGED,
                schema: LEGGED_RULES_SCHEMA,
                rules: rules_of(legged),
            });
        }
        out
    }
}

/// One authored mode's five numbers as the arena's own [`TrackedRules`].
const fn rules_of(def: TrackedDef) -> TrackedRules {
    TrackedRules {
        max_speed_cms: def.max_speed_cms,
        accel_cms2: def.accel_cms2,
        brake_cms2: def.brake_cms2,
        turn_rate_mdeg_s: def.turn_rate_mdeg_s,
        reverse_factor_pct: if def.reverse_factor_pct > 100 {
            100
        } else {
            def.reverse_factor_pct
        },
    }
}

/// **What kind of machine a unit is, for W2's cap** (16b cut AE.3; doc 16 §2
/// W2: *no drone swarms; `arena.max_units_by_class`; class counts on the map
/// obey the caps*).
///
/// A closed word, as the draw labels and the consequences are: the arena and the
/// lobby count units by it, and a word they cannot count would be a cap nobody
/// enforces. It is **declared** on a unit definition (`"class": "drone"`) or
/// **derived** when absent — a unit whose driving mode walks is a
/// [`UnitClass::Walker`], anything else [`UnitClass::Armour`] — so every unit
/// authored before this cut keeps its class without a word written and without
/// its `DefinitionRef` moving (`game_rules` emits `class` only when declared).
///
/// The derivation is [`UnitSpec::class_of`] on the authored spec and
/// [`UnitClass::read`] on the bound M32 unit; the two agree for every unit the
/// loader admits, because the blob carries the declared word and the profile
/// carries the modes in the ladder's order.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitClass {
    /// A tank, a car, a gun on tracks or wheels — every machine that is neither.
    Armour,
    /// A machine whose driving mode walks (the bipeds).
    Walker,
    /// A small unmanned platform: a sight and no gun (`drone_mk1`). W2's word,
    /// capped at one a side by default.
    Drone,
    /// **A cargo hull** (doc 18 cut IC2; doc 18 §4.2 schema addition (1),
    /// D-18-29): the Dray. Excluded from arena class caps; never fielded —
    /// [`UnitClass::fieldable`] is false, so the lobby never offers one,
    /// `Arena::new` refuses one at the custody seam and the class-cap config
    /// refuses the word. [`UnitClass::read`] still knows it.
    Hauler,
    /// **A terrain vehicle** (doc 20 cut SM.3a; doc 20 §4.1, D-20-6): the
    /// prospector rig a Dray-P carries in its bay. Excluded from arena class
    /// caps and never fielded in phase 1, exactly as [`UnitClass::Hauler`];
    /// the promotion cut (doc 20 §17) flips [`UnitClass::fieldable`] for it.
    Vehicle,
}

impl UnitClass {
    /// Every class, in the enum's (and the `BTreeMap`'s) order.
    ///
    /// Doc 18 cut IC2 appends [`UnitClass::Hauler`] and doc 20 cut SM.3a
    /// [`UnitClass::Vehicle`], so [`UnitClass::parse`] and [`UnitClass::read`]
    /// know the words; the arena's doors ask [`UnitClass::fieldable`].
    pub const ALL: [UnitClass; 5] = [
        UnitClass::Armour,
        UnitClass::Walker,
        UnitClass::Drone,
        UnitClass::Hauler,
        UnitClass::Vehicle,
    ];

    /// The class word, as content and the operator's config write it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            UnitClass::Armour => "armour",
            UnitClass::Walker => "walker",
            UnitClass::Drone => "drone",
            UnitClass::Hauler => "hauler",
            UnitClass::Vehicle => "vehicle",
        }
    }

    /// **Whether an arena may field this class** (doc 18 cut IC2; doc 18 §4.2
    /// schema addition (1), D-18-29): false for [`UnitClass::Hauler`] and, since
    /// doc 20 cut SM.3a (D-20-6), [`UnitClass::Vehicle`] — excluded from arena
    /// class caps; never fielded in phase 1.
    #[must_use]
    pub const fn fieldable(self) -> bool {
        match self {
            UnitClass::Armour | UnitClass::Walker | UnitClass::Drone => true,
            UnitClass::Hauler | UnitClass::Vehicle => false,
        }
    }

    /// The class a word names, or `None` for a word that names none (the
    /// config layer's refusal; the loader refuses through `serde`).
    #[must_use]
    pub fn parse(word: &str) -> Option<UnitClass> {
        UnitClass::ALL
            .into_iter()
            .find(|class| class.as_str() == word)
    }

    /// **The noun a player reads**, singular when `count == 1` and plural
    /// otherwise — the lobby's refusals (`A side fields at most 1 drone; …`,
    /// `This server fields no drones; …`).
    #[must_use]
    pub const fn noun(self, count: u32) -> &'static str {
        match (self, count == 1) {
            (UnitClass::Armour, true) => "armoured machine",
            (UnitClass::Armour, false) => "armoured machines",
            (UnitClass::Walker, true) => "walker",
            (UnitClass::Walker, false) => "walkers",
            (UnitClass::Drone, true) => "drone",
            (UnitClass::Drone, false) => "drones",
            (UnitClass::Hauler, true) => "hauler",
            (UnitClass::Hauler, false) => "haulers",
            (UnitClass::Vehicle, true) => "terrain vehicle",
            (UnitClass::Vehicle, false) => "terrain vehicles",
        }
    }

    /// **The class of a bound M32 unit**, read at check-out (16b cut AE.3).
    ///
    /// The unit-level rules blob ([`TacticalUnit::rules`], the opaque payload
    /// this crate alone decodes) read for a `"class"` word through
    /// [`UnitClass::parse`]; when it carries none, [`UnitClass::Walker`] for a
    /// unit whose driving mode is legged (the row
    /// [`crate::arena::CostRow::read`] answers — the ladder, else the profile's
    /// first mode) and [`UnitClass::Armour`] for anything else. So an engine
    /// fixture with no blob word and no profile is `Armour`, and is never
    /// counted against the drone's cap.
    #[must_use]
    pub fn read(unit: &TacticalUnit) -> UnitClass {
        if let Some(class) = unit
            .rules()
            .data()
            .get("class")
            .and_then(serde_json::Value::as_str)
            .and_then(UnitClass::parse)
        {
            return class;
        }
        match crate::arena::CostRow::read(unit) {
            crate::arena::CostRow::Legged => UnitClass::Walker,
            crate::arena::CostRow::Tracked | crate::arena::CostRow::Wheeled => UnitClass::Armour,
        }
    }
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
    /// **The declared class** (16b cut AE.3), or `None` to derive it
    /// ([`UnitSpec::class_of`]).
    ///
    /// Absent is the default and keeps every unit authored before this cut
    /// byte-identical: nothing is serialised and `game_rules` writes no
    /// `class` into the blob, so no `DefinitionRef` moves. A word the enum does
    /// not name (`"flier"`) is refused at decode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<UnitClass>,
    /// **The hull's logistics** (doc 18 cut IC2; doc 18 §4.2 schema additions
    /// (2), D-18-29): dry mass, the stowage compartments as measure maps, the
    /// berths, the payload rail and the drive. Read by tacsim's port code only;
    /// `bind()` ignores it, so declaring it moves no `DefinitionRef`, and absent
    /// it serialises nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logistics: Option<crate::definitions::LogisticsDef>,
    /// **The hull's walkable interior** (doc 18 cut IC2; doc 18 §4.2 schema
    /// additions (2); 18a §3; lead ruling R6): the rooms, exits and gauges the
    /// hull template instantiates. Requires [`UnitSpec::logistics`]. Read by
    /// tacsim's port code only; `bind()` ignores it, so no `DefinitionRef`
    /// moves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interior: Option<crate::definitions::InteriorDef>,
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
    /// **This unit's class** (16b cut AE.3): the declared
    /// [`UnitSpec::class`], else [`UnitClass::Walker`] when its driving mode —
    /// [`MobilityDef::modes`]' first, which is the ladder's order: tracked,
    /// wheeled, legged — is legged, else [`UnitClass::Armour`]. The lobby reads
    /// it off the catalog; [`UnitClass::read`] is the same answer off the bound
    /// unit.
    #[must_use]
    pub fn class_of(&self) -> UnitClass {
        if let Some(class) = self.class {
            return class;
        }
        match self.mobility.modes().first() {
            Some(mode) if mode.locomotion == LEGGED => UnitClass::Walker,
            _ => UnitClass::Armour,
        }
    }

    /// Every weapon definition this unit names, in component order.
    pub fn weapons(&self) -> impl Iterator<Item = &CanonicalId> {
        self.components.iter().filter_map(|c| match &c.role {
            ComponentRole::Weapon { weapon, .. } => Some(weapon),
            _ => None,
        })
    }

    /// Every sensor definition this unit names, in component order — the sights it
    /// sees with, (16b cut X.4) the rows its jamming pods radiate and (16b cut X.5)
    /// the rows its filters apply, because what this iterator is for is the
    /// reference check at load and a pod's or a filter's row is a reference like any
    /// other.
    pub fn sensors(&self) -> impl Iterator<Item = &CanonicalId> {
        self.components.iter().filter_map(|c| match &c.role {
            ComponentRole::Sensor { sensor, .. }
            | ComponentRole::Ecm { sensor }
            | ComponentRole::Eccm { sensor } => Some(sensor),
            _ => None,
        })
    }

    /// **Every core definition this unit names**, in component order (17b cut
    /// M3.1) — at most one for a unit [`UnitSpec::check`] admits; what the
    /// reference check at load walks.
    pub fn cores(&self) -> impl Iterator<Item = &CanonicalId> {
        self.components.iter().filter_map(|c| match &c.role {
            ComponentRole::Core { core } => Some(core),
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
        if self.sections.is_empty() {
            return Err(DefinitionError::value(
                entry,
                format!("a unit declares 1 to {MAX_SECTIONS} sections; 0 is not"),
            ));
        }
        // **16b cut Q.2**: the ceiling has a sentence of its own, because it is
        // the rule behind [`crate::arena::condition::Condition`]'s packing rather
        // than a bound on authoring taste — see [`MAX_SECTIONS`].
        if self.sections.len() > MAX_SECTIONS {
            return Err(DefinitionError::value(
                entry,
                format!(
                    "a unit has at most {MAX_SECTIONS} sections; {} is not",
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
                    // **16b cut X.2**: the pair is only a statement about this
                    // unit's own anatomy, so every section it names is one this
                    // unit declares — an empty list would fire on the first
                    // section lost, which is `disable_motion` spelled the long
                    // way and never what an author meant.
                    Consequence::DisableMotionWhenAllOf { sections: named } => {
                        if named.is_empty() || named.len() > MAX_SECTIONS {
                            return Err(DefinitionError::value(
                                entry,
                                format!(
                                    "section `{}`'s disable_motion_when_all_of names 1 to \
                                     {MAX_SECTIONS} sections; {} is not",
                                    section.id,
                                    named.len()
                                ),
                            ));
                        }
                        for other in named {
                            if self.section(other).is_none() {
                                return Err(DefinitionError::value(
                                    entry,
                                    format!(
                                        "section `{}`'s disable_motion_when_all_of names \
                                         section `{other}`, which this unit does not declare",
                                        section.id
                                    ),
                                ));
                            }
                        }
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
            // **16b cut X.3**: the thermal term's own bounds, re-run where every
            // other component bound is re-run.
            if let ComponentRole::Sensor {
                thermal: Some(thermal),
                ..
            } = &component.role
            {
                thermal.check(entry, component.id.as_str())?;
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
        // **17b cut M3.1** (D-UM-33): one core a hull. Two would be two classes
        // and two `tacsim.core` capabilities, and "the hull's class" would
        // depend on which `read_core` found first.
        let cores = self.cores().count();
        if cores > 1 {
            return Err(DefinitionError::value(
                entry,
                format!("a unit carries at most one core; {cores} are declared"),
            ));
        }
        self.check_bands(entry)?;
        self.check_heat(entry)?;
        self.check_mobility(entry)?;
        self.check_logistics(entry)?;
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

    /// **The logistics and the interior** (doc 18 cut IC2; doc 18 §4.2; lead
    /// ruling R15): a hauler declares its logistics (and, since doc 20 cut
    /// SM.3a, so does a vehicle), an interior requires them, and each block
    /// re-runs its own bounds.
    fn check_logistics(&self, entry: &str) -> Result<(), DefinitionError> {
        match (&self.logistics, &self.interior) {
            (Some(logistics), interior) => {
                logistics.check(entry)?;
                if let Some(interior) = interior {
                    interior.check(entry, logistics)?;
                }
            }
            (None, Some(_)) => {
                return Err(DefinitionError::value(
                    entry,
                    "an interior requires logistics",
                ));
            }
            (None, None) => {}
        }
        if self.class == Some(UnitClass::Hauler) && self.logistics.is_none() {
            return Err(DefinitionError::value(
                entry,
                "a hauler declares its logistics",
            ));
        }
        // Doc 20 cut SM.3a (§4.1): a terrain vehicle carries a hopper, so it
        // declares its (stowage-only) logistics too.
        if self.class == Some(UnitClass::Vehicle) && self.logistics.is_none() {
            return Err(DefinitionError::value(
                entry,
                "a vehicle declares its logistics",
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

    /// **At least one mobility mode** (16b cut X.2).
    ///
    /// Both fields of [`MobilityDef`] became optional when the legged mode
    /// arrived, and `"mobility": {}` is the one thing that combination makes
    /// newly expressible: a unit that declares no way of moving at all. It is
    /// refused here rather than in [`bind`], because a `MobilityProfile` needs a
    /// default mode and "the spec named none" is an authoring mistake, not a
    /// binding failure.
    fn check_mobility(&self, entry: &str) -> Result<(), DefinitionError> {
        if self.mobility.modes().is_empty() {
            return Err(DefinitionError::value(
                entry,
                "a unit declares at least one mobility mode (`tracked`, `wheeled` or `legged`)",
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

/// **The mode rules a unit drives under, read from the authored spec** (doc 16
/// §3.3).
///
/// The twin of [`crate::match_::tracked_rules`], which reads the same five
/// numbers out of an instantiated unit's opaque [`ModeRules`]. [`bind`] is what
/// makes them agree: it writes exactly these five keys into every mode's blob, so
/// for every unit made from a spec the two functions return the same value. A
/// test pins that.
///
/// **16b cut X.2**: the mode is [`MobilityDef::modes`]' first — `tracked` for
/// every unit that has tracks, `legged` for a biped — and both functions take it
/// the same way round, which is the whole of "motion reads either". The name is
/// the one doc 16 §3.3 gave the only mode there was; [`TrackedRules`] is the
/// shape of *a* mode's rules and not a statement about tracks.
///
/// A spec with no mode at all is refused at load ([`UnitSpec::check`]) and
/// answers all-zero here, which is a unit that cannot move.
#[must_use]
pub fn tracked_rules(spec: &UnitSpec) -> TrackedRules {
    spec.mobility
        .modes()
        .first()
        .map_or_else(TrackedRules::default, |mode| mode.rules)
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
    /// **Every core entry, by id** (17b cut M3.1): a core component's class
    /// and governed bit are its definition's.
    pub cores: &'a BTreeMap<CanonicalId, crate::definitions::core::CoreDef>,
    /// The already-minted reference of every core entry (17b cut M3.1).
    pub core_refs: &'a BTreeMap<CanonicalId, DefinitionRef>,
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
                ComponentRole::Sensor { sensor, thermal } => {
                    if !cx.sensors.contains_key(sensor) {
                        return Err(DefinitionError::reference(name, "sensor", sensor.as_str()));
                    }
                    let sensor_ref = cx.sensor_refs.get(sensor).ok_or_else(|| {
                        DefinitionError::reference(name, "sensor", sensor.as_str())
                    })?;
                    let mut payload =
                        serde_json::json!({ "role": "sensor", "sensor": sensor.as_str() });
                    let mut cells = vec![CellSpec {
                        id: tactics_id!(name, "cell", CONDITION_CELL, CellId)?,
                        min: 0,
                        max: MAX_CONDITION,
                        init: MAX_CONDITION,
                    }];
                    // **16b cut X.3**: the two thermal factors, and only for a
                    // mount that authors them — so every stock sight's component
                    // is the definition, the cells and the digest it was.
                    if let Some(thermal) = thermal {
                        if let Some(object) = payload.as_object_mut() {
                            object.insert(
                                "thermal".to_owned(),
                                serde_json::json!({
                                    "powered_permille": thermal.powered_permille,
                                    "cold_permille": thermal.cold_permille,
                                }),
                            );
                        }
                        for (cell, permille) in [
                            (THERMAL_POWERED_CELL, thermal.powered_permille),
                            (THERMAL_COLD_CELL, thermal.cold_permille),
                        ] {
                            cells.push(CellSpec {
                                id: tactics_id!(name, "cell", cell, CellId)?,
                                min: 0,
                                max: i64::from(MAX_SIGNATURE_PERMILLE),
                                init: i64::from(permille),
                            });
                        }
                    }
                    (payload, vec![sensor_ref.clone()], cells)
                }
                // **16b cut X.4**: the jamming pod (doc 16 §9 item 4). Its three
                // numbers are the sensor row's `jamming` block, laid down as cells
                // so the arena reads them off the unit that crosses the custody
                // seam anyway.
                ComponentRole::Ecm { sensor } => {
                    let def = cx.sensors.get(sensor).ok_or_else(|| {
                        DefinitionError::reference(name, "sensor", sensor.as_str())
                    })?;
                    let sensor_ref = cx.sensor_refs.get(sensor).ok_or_else(|| {
                        DefinitionError::reference(name, "sensor", sensor.as_str())
                    })?;
                    // A pod that radiates a row with no `jamming` block jams
                    // nothing, and would do it silently for a whole match. The two
                    // are authored in different files and neither can check the
                    // other alone, so the cross-reference is checked here — the
                    // same place cut X.1 checks that a laser's tank declares a heat
                    // ceiling.
                    let jamming = def.jamming.as_ref().ok_or_else(|| {
                        DefinitionError::value(
                            name,
                            format!(
                                "component `{}` is an `ecm` pod radiating `{}`, which declares \
                                 no `jamming`",
                                component.id, sensor
                            ),
                        )
                    })?;
                    let cells = vec![
                        CellSpec {
                            id: tactics_id!(name, "cell", CONDITION_CELL, CellId)?,
                            min: 0,
                            max: MAX_CONDITION,
                            init: MAX_CONDITION,
                        },
                        CellSpec {
                            id: tactics_id!(name, "cell", JAM_RADIUS_CELL, CellId)?,
                            min: 0,
                            max: i64::from(MAX_JAM_RADIUS_CELLS),
                            init: i64::from(jamming.radius_cells),
                        },
                        CellSpec {
                            id: tactics_id!(name, "cell", JAM_ATTENUATION_CELL, CellId)?,
                            min: 0,
                            max: i64::from(MAX_ATTENUATION_COST),
                            init: i64::from(jamming.attenuation_cost),
                        },
                        CellSpec {
                            id: tactics_id!(name, "cell", JAM_SIGNATURE_CELL, CellId)?,
                            min: 0,
                            max: i64::from(MAX_SIGNATURE_PERMILLE),
                            init: i64::from(jamming.signature_permille),
                        },
                    ];
                    // The block itself is deliberately **not** copied into the
                    // payload the way cut X.3's thermal band is. X.3's band is
                    // authored on the mount, so the mount is the only place it
                    // exists; a pod's three numbers are the *sensor row's*, and
                    // that row is `sensor_ref` — a reference with its own digest
                    // in it. Change the block and the row's digest moves, so this
                    // component's does, so the unit's does. Copying the numbers in
                    // as well would hash them twice and give two places to
                    // disagree.
                    (
                        serde_json::json!({ "role": "ecm", "sensor": sensor.as_str() }),
                        vec![sensor_ref.clone()],
                        cells,
                    )
                }
                // **16b cut X.5**: the counter-jamming filter (doc 16 §9 item 5).
                // Its two numbers are the sensor row's `emission` block, laid down
                // as cells for the reason the pod's three are, and beside the
                // `condition` cell that says whether the filter is still there —
                // which is what the arena reads to decide that it counts.
                ComponentRole::Eccm { sensor } => {
                    let def = cx.sensors.get(sensor).ok_or_else(|| {
                        DefinitionError::reference(name, "sensor", sensor.as_str())
                    })?;
                    let sensor_ref = cx.sensor_refs.get(sensor).ok_or_else(|| {
                        DefinitionError::reference(name, "sensor", sensor.as_str())
                    })?;
                    // A filter fitted to a row with no `emission` block recovers
                    // nothing, and would do it silently for a whole match — the
                    // same failure cut X.4's pod has, checked in the same place and
                    // for the same reason: the two are authored in different files
                    // and neither can check the other alone.
                    let eccm = def.eccm().ok_or_else(|| {
                        DefinitionError::value(
                            name,
                            format!(
                                "component `{}` is an `eccm` filter fitted to `{}`, which \
                                 declares no `emission`",
                                component.id, sensor
                            ),
                        )
                    })?;
                    let cells = vec![
                        CellSpec {
                            id: tactics_id!(name, "cell", CONDITION_CELL, CellId)?,
                            min: 0,
                            max: MAX_CONDITION,
                            init: MAX_CONDITION,
                        },
                        CellSpec {
                            id: tactics_id!(name, "cell", ECCM_ATTENUATION_CELL, CellId)?,
                            min: 0,
                            max: i64::from(MAX_ATTENUATION_COST),
                            init: i64::from(eccm.attenuation_recovery),
                        },
                        CellSpec {
                            id: tactics_id!(name, "cell", ECCM_SIGNATURE_CELL, CellId)?,
                            min: 0,
                            max: i64::from(MAX_SIGNATURE_PERMILLE),
                            init: i64::from(eccm.signature_recovery_permille),
                        },
                    ];
                    // The block is **not** copied into the payload, for the reason
                    // cut X.4's is not: a filter's two numbers are the *sensor
                    // row's*, and that row is `sensor_ref` — a reference carrying
                    // its own digest. Change the block and the row's digest moves,
                    // so this component's does, so the unit's does.
                    (
                        serde_json::json!({ "role": "eccm", "sensor": sensor.as_str() }),
                        vec![sensor_ref.clone()],
                        cells,
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
                // **17b cut M3.1** (doc 17 §9, D-UM-33): the core. One cell —
                // the class NOW, born at the fitted class's ordinal, what
                // `degrade_core` will lower — and the `tacsim.core` capability
                // sourced on this component, whose payload is the fitting's
                // record: the class, the governed bit and that class's row of
                // the compiled-in `ClassTable::STOCK`, through the ONE helper
                // every test hull uses too (`core::profile_payload`). The class
                // a mind thinks at is read live off the cell
                // (`crate::arena::core::read_core`), never off the payload.
                ComponentRole::Core { core } => {
                    let def = cx
                        .cores
                        .get(core)
                        .ok_or_else(|| DefinitionError::reference(name, "core", core.as_str()))?;
                    let core_ref = cx
                        .core_refs
                        .get(core)
                        .ok_or_else(|| DefinitionError::reference(name, "core", core.as_str()))?;
                    let schema = schema_ref(name, cx.schemas, CORE_RULES_SCHEMA)?;
                    let source =
                        tactics_id!(name, "component", component.id.as_str(), ComponentId)?;
                    let unbuilt = || {
                        DefinitionError::value(
                            name,
                            format!("component `{}`'s core will not bind", component.id),
                        )
                    };
                    capabilities.push(
                        crate::definitions::core::core_capability(
                            &source,
                            def.class,
                            def.governed,
                            schema,
                        )
                        .ok_or_else(unbuilt)?,
                    );
                    (
                        serde_json::json!({ "role": "core", "core": core.as_str() }),
                        vec![core_ref.clone()],
                        vec![crate::definitions::core::core_cell(def.class).ok_or_else(unbuilt)?],
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

    // (3) The mobility modes — one per mode the spec declares, in the spec's own
    // order (**16b cut X.2**; before it there was only ever `tracked`, and a unit
    // that declares only `tracked` binds to exactly the profile it bound to then).
    let mut modes: Vec<MobilityMode> = Vec::new();
    let mut default_mode: Option<MobilityModeId> = None;
    for declared in spec.mobility.modes() {
        let rules = declared.rules;
        let id = tactics_id!(name, "mobility mode", declared.mode, MobilityModeId)?;
        let mode = MobilityMode::new(
            id.clone(),
            [tactics_id!(name, "medium", GROUND, MediumId)?],
            tactics_id!(name, "locomotion", declared.locomotion, LocomotionId)?,
            ModeRules::new(
                schema_ref(name, cx.schemas, declared.schema)?,
                serde_json::json!({
                    "max_speed_cms": rules.max_speed_cms,
                    "accel_cms2": rules.accel_cms2,
                    "brake_cms2": rules.brake_cms2,
                    "turn_rate_mdeg_s": rules.turn_rate_mdeg_s,
                    "reverse_factor_pct": rules.reverse_factor_pct,
                }),
            )
            .map_err(|error| DefinitionError::value(name, error.to_string()))?,
        )
        .with_speeds(rules.max_speed_cms, 0);
        if default_mode.is_none() {
            default_mode = Some(id);
        }
        modes.push(mode);
    }
    let default_mode = default_mode.ok_or_else(|| {
        DefinitionError::value(
            name,
            "a unit declares at least one mobility mode (`tracked`, `wheeled` or `legged`)",
        )
    })?;
    let profile = MobilityProfile::new(modes, default_mode, &MobilityVocabulary::base().freeze())
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
///
/// **16b cut AE.3**: `class` is written only when the spec declares one, so a
/// unit authored before the class existed binds to the same blob — and the same
/// `DefinitionRef` — as it did; [`UnitClass::read`] derives the class of such a
/// unit from its profile instead.
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
    let mut rules = serde_json::json!({
        "facets": spec.facets,
        "silhouettes": spec.silhouettes,
        "sensor_height_cm": spec.sensor_height_cm,
        "consequences": consequences,
        "components": spec.components,
        "mobility": spec.mobility,
    });
    if let (Some(class), Some(map)) = (spec.class, rules.as_object_mut()) {
        map.insert("class".to_owned(), serde_json::json!(class.as_str()));
    }
    rules
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
