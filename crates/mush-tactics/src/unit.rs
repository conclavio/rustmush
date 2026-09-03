//! # TacticalUnit — the composed structural model + damage pipeline (design 06 §3.3)
//!
//! `mush-tactics` understands that a unit has **sections, facets, installed
//! components, bounded scalar cells, and capabilities** — as generic,
//! individually addressable collections. It does NOT understand "right leg",
//! "rear torso armor", "autocannon", or "ECM": concrete anatomy and every
//! formula belong to the ruleset. Nothing below `mush-tactics` (esp.
//! `mush-sim`) knows sections or equipment at all — a `TacticalUnit` is ONE
//! placed object in the placement graph; its sections/facets/cells/components
//! form a SECOND tree that lives entirely inside `mush-tactics`.
//!
//! ## The through-line: closed verbs, open nouns
//! The engine's mutation/validation ALPHABET ([`UnitMutationOp`]) is a closed,
//! typed set; every referent it names ([`SectionId`], [`CellId`],
//! [`CapabilityId`], …) is an open, ruleset-registered id. A consequence outside
//! the alphabet is the signal to grow it deliberately, never to teach the engine
//! a formula.
//!
//! ## The damage pipeline (the [`mush_sim`] `Transition` analogue)
//! The RULESET authors a flat [`UnitMutationPlan`]; [`apply`] VALIDATES it (every
//! referent resolves, every scalar lands in-bounds, no overflow) and APPLIES it
//! ATOMICALLY on a scratch clone — any failure drops the clone and leaves the
//! caller's unit untouched (refusal = zero mutation). Unlike `mush-sim`'s
//! self-authored plans, a `UnitMutationPlan` is untrusted-for-bugs: a buggy
//! ruleset plan yields a clean typed [`UnitError`], never a panic or partial
//! write. The plan is FLAT — the ruleset expands any cascade ("a lost side torso
//! kills its components") into explicit ops; the engine never traverses the tree
//! to infer consequences.
//!
//! ## What the engine does NOT do (design 06 §8)
//! The engine standardizes the CONTAINER — a bounded integer at an address, an
//! opaque condition, an opaque capability bit — never the SEMANTICS of heat,
//! armor, or "destroyed". Section adjacency is not walked by any engine
//! algorithm (M32 chose opaque addressing over a validated section tree); damage
//! never edits [`MobilityProfile`] topology (that would entangle its
//! `begin_switch`/`settle` invariants) — total mobility loss is a
//! `RevokeCapability(tactics.mobility)`, degradation is ruleset cells the opaque
//! kinematics read.
//!
//! ## Scope (M32) — a PURE LIBRARY
//! Types + construction ([`TacticalUnit::from_spec`]) + the damage pipeline, all
//! pure (`now: i64`). Per-object persistence, a module, boot revalidation, an
//! `ObjDestroyed` scrub, a staff command, and panels land in M33 (the tactics
//! module that first attaches this + M31 mobility to real objects). The
//! action-provider protocol (weapons) waits on the F3 control bundle;
//! install/remove structural verbs grow with the M33/economy cut.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionLocator, DefinitionRegistry,
    DefinitionRegistryBuilder, MAX_CASCADE_OPERATIONS, ObjId,
};
pub use mush_core::{DefinitionRef, RulesValue};
use serde::de::{Error as _, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use smol_str::SmolStr;

use crate::mobility::{MobilityMode, MobilityProfile, ModeState};

/// Max canonical length of any tactical id.
pub const TACTICAL_ID_MAX: usize = 64;
/// Maximum exact unit definitions in one frozen tactical catalog.
pub const MAX_UNIT_CATALOG_DEFINITIONS: usize = 4_096;
/// Maximum encoded bytes retained by one tactical catalog.
pub const MAX_UNIT_CATALOG_BYTES: usize = 32 * 1_048_576;
/// Maximum encoded bytes in one unit-definition body.
pub const MAX_UNIT_SPEC_BYTES: usize = 1_048_576;
/// Maximum structural sections in one unit definition or instance.
pub const MAX_UNIT_SECTIONS: usize = 256;
/// Maximum facets across all sections in one unit definition or instance.
pub const MAX_UNIT_FACETS: usize = 1_024;
/// Maximum install slots across all sections in one unit definition or instance.
pub const MAX_UNIT_SLOTS: usize = 2_048;
/// Maximum scalar cells across all scopes in one unit definition or instance.
pub const MAX_UNIT_CELLS: usize = 8_192;
/// Maximum installed components in one unit definition or instance.
pub const MAX_UNIT_COMPONENTS: usize = 4_096;
/// Maximum capability entries in one unit definition or instance.
pub const MAX_UNIT_CAPABILITIES: usize = 4_096;
/// Maximum mobility modes in one unit definition or instance.
pub const MAX_UNIT_MOBILITY_MODES: usize = 128;
/// Maximum operations in one ruleset-authored mutation plan. This is exactly
/// K2's whole-cascade operation ceiling: one tactical plan can consume no more
/// than the complete action budget that contains it.
pub const MAX_UNIT_MUTATION_OPERATIONS: usize = MAX_CASCADE_OPERATIONS as usize;

/// Canonicalize trusted tactical-id input: trim, ASCII-lowercase, require a
/// non-empty run of `[a-z0-9_.]` with no leading/trailing dot and no empty segment. Both bare
/// (`head`, `armor`) and namespaced (`tacsim.rear`) forms are accepted; the
/// canonical form guarantees one logical id always serializes to identical
/// bytes, so equal units hash equally.
fn canon(s: &str) -> Result<SmolStr, UnitError> {
    let t = s.trim().to_ascii_lowercase();
    if t.is_empty() {
        return Err(UnitError::EmptyId);
    }
    if t.len() > TACTICAL_ID_MAX {
        return Err(UnitError::IdTooLong(t));
    }
    if !t
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
    {
        return Err(UnitError::MalformedId(t));
    }
    if t.starts_with('.') || t.ends_with('.') || t.split('.').any(str::is_empty) {
        return Err(UnitError::MalformedId(t));
    }
    Ok(SmolStr::new(t))
}

/// Parse persisted/wire input without silently changing its identity. Authoring
/// paths may use [`canon`], but durable data must already be canonical so two
/// distinct byte strings can never collapse to one map key during deserialize.
fn parse_canonical(s: &str) -> Result<SmolStr, UnitError> {
    let canonical = canon(s)?;
    if canonical.as_str() != s {
        return Err(UnitError::NonCanonicalId {
            canonical: canonical.to_string(),
        });
    }
    Ok(canonical)
}

/// Define a canonical, `SmolStr`-backed tactical id newtype. Trusted authoring
/// construction normalizes; persisted/wire deserialization is strict.
macro_rules! tactical_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "SmolStr", into = "SmolStr")]
        pub struct $name(SmolStr);
        impl $name {
            /// Validate and canonicalize trusted authoring input (see the
            /// module's `canon`). Persisted/wire deserialization is stricter and
            /// rejects text that is not already canonical.
            ///
            /// # Errors
            /// [`UnitError::EmptyId`] / [`UnitError::IdTooLong`] /
            /// [`UnitError::MalformedId`].
            pub fn new(s: impl AsRef<str>) -> Result<$name, UnitError> {
                Ok($name(canon(s.as_ref())?))
            }
            /// The canonical id as a string slice.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<SmolStr> for $name {
            type Error = UnitError;
            fn try_from(s: SmolStr) -> Result<$name, UnitError> {
                Ok($name(parse_canonical(s.as_str())?))
            }
        }
        impl From<$name> for SmolStr {
            fn from(v: $name) -> SmolStr {
                v.0
            }
        }
        impl std::str::FromStr for $name {
            type Err = UnitError;
            fn from_str(s: &str) -> Result<$name, UnitError> {
                $name::new(s)
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.0)
            }
        }
    };
}

tactical_id! {
    /// A structural section of a unit (`head`, `center_torso`, `hull`, `turret`).
    /// Opaque to the engine; the ruleset supplies the anatomy.
    SectionId
}
tactical_id! {
    /// An exposed surface of a section, struck from a direction (`front`,
    /// `rear`, `top`). The ruleset owns which facet an attack reaches.
    FacetId
}
tactical_id! {
    /// An internal equipment install point within a section (NOT a `mush-sim`
    /// `MountId` — that is an EXTERNAL placement-graph attachment; this is a
    /// section-tree address invisible below `mush-tactics`).
    InstallSlotId
}
tactical_id! {
    /// A bounded scalar cell's id within its host (`armor`, `structure`, `heat`,
    /// `ammo`, `fuel`). The ruleset names it and owns its meaning/regen.
    CellId
}
tactical_id! {
    /// A unit-scoped installed-component instance id.
    ComponentId
}
tactical_id! {
    /// A registered capability interface id (`tactics.mobility`,
    /// `tactics.action.attack`, `tactics.observation.sensor`). An interface id,
    /// never a promise the engine understands its formula.
    CapabilityId
}
tactical_id! {
    /// An opaque condition label the engine stores and moves but NEVER
    /// interprets (a closed Ok/Damaged/Destroyed enum would teach the engine a
    /// damage ladder — the ruleset owns that).
    ConditionId
}

/// A bounded integer scalar — the unified primitive behind located tracks
/// (armor/structure) and pooled resources (heat/ammo/fuel). `min`/`max` are
/// immutable structure; only `current` is mutated, and only within `[min,max]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScalarCell {
    /// Inclusive lower bound.
    pub min: i64,
    /// Inclusive upper bound (`>= min`).
    pub max: i64,
    /// Current value, always in `[min, max]`.
    pub current: i64,
}

impl ScalarCell {
    fn check(&self) -> Result<(), UnitError> {
        if self.min > self.max || self.current < self.min || self.current > self.max {
            return Err(UnitError::CellBounds {
                min: self.min,
                max: self.max,
                current: self.current,
            });
        }
        Ok(())
    }
}

/// A fully-qualified address of a [`ScalarCell`] — a CLOSED location kind over
/// OPEN ids. Facet/component cells are section- or component-qualified so the
/// address is globally unique within a unit.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CellAddr {
    /// A unit-level pool.
    Unit {
        /// The cell id.
        cell: CellId,
    },
    /// A section-level cell.
    Section {
        /// The section.
        section: SectionId,
        /// The cell id.
        cell: CellId,
    },
    /// A facet-level cell (armor on an exposed surface).
    Facet {
        /// The section.
        section: SectionId,
        /// The facet within that section.
        facet: FacetId,
        /// The cell id.
        cell: CellId,
    },
    /// A component-level cell (a weapon's ammo pool).
    Component {
        /// The component.
        component: ComponentId,
        /// The cell id.
        cell: CellId,
    },
}

/// Where an installed component sits — an opaque section-tree address (M32 does
/// NOT validate section topology; addresses only resolve to declared sections
/// and their declared slots).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum InstallPoint {
    /// Installed at unit level.
    Unit,
    /// Installed directly in a section.
    Section(SectionId),
    /// Installed in a named slot of a section.
    Slot {
        /// The section.
        section: SectionId,
        /// The install slot.
        slot: InstallSlotId,
    },
}

/// The key of a capability entry: the interface id plus the component that
/// provides it (if any), so "component destroyed → its capability revoked" is a
/// legible cause/effect pair.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityKey {
    /// The capability interface id.
    pub id: CapabilityId,
    /// The providing component, or `None` for an intrinsic unit capability.
    pub source: Option<ComponentId>,
}

/// A section's mutable condition plus its declared (immutable) facet and slot
/// addresses — the instance is self-describing so references resolve without a
/// registry. No parent link: M32 does not model or walk section topology.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectionState {
    /// The section's opaque condition.
    pub condition: ConditionId,
    /// The facet ids this section exposes (for facet-cell address resolution).
    pub facets: BTreeSet<FacetId>,
    /// The install-slot ids this section offers (for component-install resolution).
    pub slots: BTreeSet<InstallSlotId>,
}

/// An installed component instance. The engine tracks installation + opaque
/// condition (persistence, refits, legal actions, presentation, ownership); the
/// ruleset defines what it IS and does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledComponent {
    /// The component definition (provenance).
    pub definition: DefinitionRef,
    /// Where it is installed (an opaque section-tree address).
    pub install: InstallPoint,
    /// Its opaque condition.
    pub condition: ConditionId,
    /// An optional detachable, ownable economic asset this component IS (major
    /// tradeable kit); minor internal parts leave this `None`.
    ///
    /// E3c gave the pointer a reader (`mush-economy` titles objects, and a title
    /// record names an `ObjId`), so it is no longer inert and no longer allowed
    /// to outlive its referent: [`UnitMutationOp::SetComponentAsset`] is the one
    /// way to change it, and the module nulls it on `ObjDestroyed` and at boot
    /// (`module.rs`). `mush-tactics` still takes **no** dependency on
    /// `mush-economy` — a dangling `ObjId` is a tactics-side integrity problem,
    /// and ObjIds are never reused, so the scrub cannot mis-target.
    #[serde(default)]
    pub asset: Option<ObjId>,
}

// ----------------------------------------------------------------------
// The unit instance
// ----------------------------------------------------------------------

/// A composed tactical unit. Fields are private; build via
/// [`from_spec`](Self::from_spec) or `serde` (both enforce the intrinsic
/// invariants). A fresh unit has `revision == 0`; each APPLIED plan bumps it by 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "UnitRepr", into = "UnitRepr")]
pub struct TacticalUnit {
    object: ObjId,
    schema: DefinitionRef,
    sections: BTreeMap<SectionId, SectionState>,
    cells: BTreeMap<CellAddr, ScalarCell>,
    components: BTreeMap<ComponentId, InstalledComponent>,
    capabilities: BTreeMap<CapabilityKey, RulesValue>,
    mobility: Option<MobilityProfile>,
    revision: u64,
    rules: RulesValue,
}

/// The serialized shape, so `serde` re-checks the intrinsic invariants
/// (non-empty sections, cell bounds, every intra-instance reference resolves) on
/// load — a structurally broken document fails to deserialize (the M31
/// `ProfileRepr` precedent). `cells`/`capabilities` are `Vec` pairs, not maps,
/// because their keys are enums/structs (not JSON-string map keys).
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnitRepr {
    object: ObjId,
    schema: DefinitionRef,
    sections: BTreeMap<SectionId, SectionState>,
    cells: Vec<(CellAddr, ScalarCell)>,
    components: BTreeMap<ComponentId, InstalledComponent>,
    capabilities: Vec<(CapabilityKey, RulesValue)>,
    mobility: Option<MobilityProfile>,
    revision: u64,
    rules: RulesValue,
}

impl From<TacticalUnit> for UnitRepr {
    fn from(u: TacticalUnit) -> UnitRepr {
        UnitRepr {
            object: u.object,
            schema: u.schema,
            sections: u.sections,
            cells: u.cells.into_iter().collect(),
            components: u.components,
            capabilities: u.capabilities.into_iter().collect(),
            mobility: u.mobility,
            revision: u.revision,
            rules: u.rules,
        }
    }
}
impl TryFrom<UnitRepr> for TacticalUnit {
    type Error = UnitError;
    fn try_from(r: UnitRepr) -> Result<TacticalUnit, UnitError> {
        let mut cells = BTreeMap::new();
        for (addr, cell) in r.cells {
            if cells.insert(addr.clone(), cell).is_some() {
                return Err(UnitError::DuplicateCell(addr));
            }
        }
        let mut capabilities = BTreeMap::new();
        for (key, rules) in r.capabilities {
            if capabilities.insert(key.clone(), rules).is_some() {
                return Err(UnitError::DuplicateCapability(key));
            }
        }
        let u = TacticalUnit {
            object: r.object,
            schema: r.schema,
            sections: r.sections,
            cells,
            components: r.components,
            capabilities,
            mobility: r.mobility,
            revision: r.revision,
            rules: r.rules,
        };
        u.validate()?;
        Ok(u)
    }
}

// ----------------------------------------------------------------------
// Strict legacy-v1 persistence adapter
// ----------------------------------------------------------------------

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyDefinitionRefV1 {
    id: SmolStr,
    version: u32,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyRulesValueV1 {
    schema: SmolStr,
    version: u32,
    #[serde(default)]
    data: serde_json::Value,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyInstalledComponentV1 {
    definition: LegacyDefinitionRefV1,
    install: InstallPoint,
    condition: ConditionId,
    #[serde(default)]
    asset: Option<ObjId>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyMobilityModeV1 {
    id: crate::mobility::MobilityModeId,
    mediums: BTreeSet<crate::mobility::MediumId>,
    locomotion: crate::mobility::LocomotionId,
    #[serde(default)]
    tags: BTreeSet<crate::mobility::MobilityTag>,
    rules: LegacyRulesValueV1,
    #[serde(default)]
    nominal_speed_cms: u32,
    #[serde(default)]
    transition_s: u32,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyMobilityProfileV1 {
    modes: Vec<LegacyMobilityModeV1>,
    mode: ModeState,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyUnitReprV1 {
    object: ObjId,
    schema: LegacyDefinitionRefV1,
    sections: BTreeMap<SectionId, SectionState>,
    cells: Vec<(CellAddr, ScalarCell)>,
    components: BTreeMap<ComponentId, LegacyInstalledComponentV1>,
    capabilities: Vec<(CapabilityKey, LegacyRulesValueV1)>,
    mobility: Option<LegacyMobilityProfileV1>,
    revision: u64,
    rules: LegacyRulesValueV1,
}

/// Pure result of decoding the original M32/M33 unit wire format. This adapter
/// deliberately returns only the typed unit body: the owning persistence module
/// must place it in that module's current versioned record envelope before an
/// acknowledgement-gated migration may author bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyUnitV1Decoded {
    /// Current, exact-definition in-memory representation.
    pub unit: TacticalUnit,
}

/// Why an old tactical-unit document cannot be mapped exactly through the
/// selected catalog. Errors never infer an owner from a dotted legacy id.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LegacyUnitV1Error {
    /// The old document did not have the exact known v1 shape.
    #[error("malformed legacy tactical-unit v1 document: {0}")]
    Malformed(String),
    /// No explicit deployment alias maps the legacy root id and exact version
    /// to one exact catalog definition.
    #[error("legacy unit definition is unavailable in the catalog")]
    RootDefinitionUnavailable,
    /// A legacy nested id/version differs from the reference supplied by the
    /// selected unit definition.
    #[error("legacy definition reference does not match catalog field {field}")]
    DefinitionMismatch {
        /// Stable field category, never untrusted input.
        field: &'static str,
    },
    /// An installed component is not declared by the selected definition.
    #[error("legacy component {0} has no mapping in the selected unit definition")]
    ComponentUnavailable(ComponentId),
    /// A capability is not declared by the selected definition.
    #[error("legacy capability {0:?} has no mapping in the selected unit definition")]
    CapabilityUnavailable(CapabilityKey),
    /// Legacy and selected definitions disagree about whether mobility exists.
    #[error("legacy mobility does not match the selected unit definition")]
    MobilityMismatch,
    /// A legacy mobility mode is absent or structurally different in the
    /// selected definition.
    #[error("legacy mobility mode {0} does not match the selected unit definition")]
    MobilityModeMismatch(crate::mobility::MobilityModeId),
    /// Shared rules-value size or mobility intrinsic validation failed.
    #[error("legacy rules payload is invalid at {field}: {reason}")]
    InvalidRulesValue {
        /// Stable field category.
        field: &'static str,
        /// Bounded engine-generated explanation.
        reason: String,
    },
    /// The mapped unit violates current intrinsic invariants.
    #[error("mapped legacy unit is invalid: {0}")]
    InvalidUnit(#[source] Box<UnitError>),
}

impl From<UnitError> for LegacyUnitV1Error {
    fn from(error: UnitError) -> Self {
        LegacyUnitV1Error::InvalidUnit(Box::new(error))
    }
}

fn legacy_ref_matches(legacy: &LegacyDefinitionRefV1, current: &DefinitionRef) -> bool {
    legacy.id == current.key().id.as_str() && legacy.version == current.version()
}

fn map_legacy_rules(
    legacy: LegacyRulesValueV1,
    current: &DefinitionRef,
    field: &'static str,
) -> Result<RulesValue, LegacyUnitV1Error> {
    if legacy.schema != current.key().id.as_str() || legacy.version != current.version() {
        return Err(LegacyUnitV1Error::DefinitionMismatch { field });
    }
    RulesValue::new(current.clone(), legacy.data).map_err(|error| {
        LegacyUnitV1Error::InvalidRulesValue {
            field,
            reason: error.to_string(),
        }
    })
}

fn map_legacy_mobility(
    legacy: Option<LegacyMobilityProfileV1>,
    current: Option<&MobilityProfile>,
) -> Result<Option<MobilityProfile>, LegacyUnitV1Error> {
    let (legacy, current) = match (legacy, current) {
        (None, None) => return Ok(None),
        (Some(legacy), Some(current)) => (legacy, current),
        _ => return Err(LegacyUnitV1Error::MobilityMismatch),
    };
    if legacy.modes.len() != current.modes().len() {
        return Err(LegacyUnitV1Error::MobilityMismatch);
    }
    let mut modes = Vec::with_capacity(legacy.modes.len());
    for legacy_mode in legacy.modes {
        let Some(current_mode) = current.mode_by_id(&legacy_mode.id) else {
            return Err(LegacyUnitV1Error::MobilityModeMismatch(legacy_mode.id));
        };
        if legacy_mode.mediums != current_mode.mediums
            || legacy_mode.locomotion != current_mode.locomotion
            || legacy_mode.tags != current_mode.tags
            || legacy_mode.nominal_speed_cms != current_mode.nominal_speed_cms
            || legacy_mode.transition_s != current_mode.transition_s
        {
            return Err(LegacyUnitV1Error::MobilityModeMismatch(legacy_mode.id));
        }
        let rules = map_legacy_rules(
            legacy_mode.rules,
            current_mode.rules.schema(),
            "mobility mode rules",
        )?;
        modes.push(MobilityMode {
            id: legacy_mode.id,
            mediums: legacy_mode.mediums,
            locomotion: legacy_mode.locomotion,
            tags: legacy_mode.tags,
            rules,
            nominal_speed_cms: legacy_mode.nominal_speed_cms,
            transition_s: legacy_mode.transition_s,
        });
    }
    MobilityProfile::from_parts(modes, legacy.mode)
        .map(Some)
        .map_err(|error| LegacyUnitV1Error::InvalidRulesValue {
            field: "mobility profile",
            reason: error.to_string(),
        })
}

/// Strictly decode the original tactical-unit persistence shape against one
/// frozen catalog. The legacy root string is resolved only through an explicit
/// [`UnitCatalogBuilder::legacy_v1_alias`] configured by deployment wiring;
/// nested strings are then matched only to exact references on that selected
/// spec. Dotted strings are never split to manufacture owner or kind.
///
/// # Errors
/// Returns [`LegacyUnitV1Error`] when the v1 shape is malformed, the root is
/// absent from the explicit alias table, a nested legacy reference cannot be mapped through that
/// root definition, or the resulting current unit is invalid.
pub fn decode_legacy_unit_v1(
    value: &serde_json::Value,
    catalog: &UnitCatalog,
) -> Result<LegacyUnitV1Decoded, LegacyUnitV1Error> {
    let legacy: LegacyUnitReprV1 = serde_json::from_value(value.clone())
        .map_err(|error| LegacyUnitV1Error::Malformed(error.to_string()))?;
    let legacy_root_id = DefinitionId::parse_canonical(legacy.schema.id.as_str())
        .map_err(|error| LegacyUnitV1Error::Malformed(error.to_string()))?;
    let root = catalog
        .legacy_v1_root(&legacy_root_id, legacy.schema.version)
        .ok_or(LegacyUnitV1Error::RootDefinitionUnavailable)?;

    let mut components = BTreeMap::new();
    for (id, legacy_component) in legacy.components {
        let current_component = root
            .components
            .iter()
            .find(|component| component.id == id)
            .ok_or_else(|| LegacyUnitV1Error::ComponentUnavailable(id.clone()))?;
        if !legacy_ref_matches(&legacy_component.definition, &current_component.definition) {
            return Err(LegacyUnitV1Error::DefinitionMismatch {
                field: "component definition",
            });
        }
        components.insert(
            id,
            InstalledComponent {
                definition: current_component.definition.clone(),
                install: legacy_component.install,
                condition: legacy_component.condition,
                asset: legacy_component.asset,
            },
        );
    }

    let mut capabilities = Vec::with_capacity(legacy.capabilities.len());
    for (key, legacy_rules) in legacy.capabilities {
        let current_capability = root
            .capabilities
            .iter()
            .find(|capability| capability.key == key)
            .ok_or_else(|| LegacyUnitV1Error::CapabilityUnavailable(key.clone()))?;
        capabilities.push((
            key,
            map_legacy_rules(
                legacy_rules,
                current_capability.rules.schema(),
                "capability rules",
            )?,
        ));
    }

    let rules = map_legacy_rules(legacy.rules, root.rules.schema(), "unit rules")?;
    let mobility = map_legacy_mobility(legacy.mobility, root.mobility.as_ref())?;
    let unit = TacticalUnit::try_from(UnitRepr {
        object: legacy.object,
        schema: root.definition.clone(),
        sections: legacy.sections,
        cells: legacy.cells,
        components,
        capabilities,
        mobility,
        revision: legacy.revision,
        rules,
    })?;
    root.validate_instance(&unit)?;
    Ok(LegacyUnitV1Decoded { unit })
}

impl TacticalUnit {
    /// Instantiate a unit from a ruleset schema and its object id. Cells start at
    /// their spec `init`; sections/components at their initial condition;
    /// `revision` at 0.
    ///
    /// # Errors
    /// A [`UnitError`] for a duplicate id, a bad cell spec (`min>max` or `init`
    /// out of range), an unresolved reference, or an empty unit.
    pub fn from_spec(spec: &UnitSpec, object: ObjId) -> Result<TacticalUnit, UnitError> {
        validate_spec_definition_kinds(spec)?;
        validate_spec_definition_owners(spec)?;
        validate_spec_bounds(spec)?;
        let mut sections = BTreeMap::new();
        let mut cells: BTreeMap<CellAddr, ScalarCell> = BTreeMap::new();

        for s in &spec.sections {
            let mut facets = BTreeSet::new();
            for f in &s.facets {
                if !facets.insert(f.id.clone()) {
                    return Err(UnitError::DuplicateFacet {
                        section: s.id.clone(),
                        facet: f.id.clone(),
                    });
                }
            }
            let mut slots = BTreeSet::new();
            for sl in &s.slots {
                if !slots.insert(sl.clone()) {
                    return Err(UnitError::DuplicateSlot {
                        section: s.id.clone(),
                        slot: sl.clone(),
                    });
                }
            }
            if sections
                .insert(
                    s.id.clone(),
                    SectionState {
                        condition: s.condition.clone(),
                        facets,
                        slots,
                    },
                )
                .is_some()
            {
                return Err(UnitError::DuplicateSection(s.id.clone()));
            }
            for c in &s.cells {
                insert_cell(
                    &mut cells,
                    CellAddr::Section {
                        section: s.id.clone(),
                        cell: c.id.clone(),
                    },
                    c,
                )?;
            }
            for f in &s.facets {
                for c in &f.cells {
                    let addr = CellAddr::Facet {
                        section: s.id.clone(),
                        facet: f.id.clone(),
                        cell: c.id.clone(),
                    };
                    insert_cell(&mut cells, addr, c)?;
                }
            }
        }
        for c in &spec.unit_cells {
            insert_cell(&mut cells, CellAddr::Unit { cell: c.id.clone() }, c)?;
        }
        let mut components = BTreeMap::new();
        for comp in &spec.components {
            if components
                .insert(
                    comp.id.clone(),
                    InstalledComponent {
                        definition: comp.definition.clone(),
                        install: comp.install.clone(),
                        condition: comp.condition.clone(),
                        asset: comp.asset,
                    },
                )
                .is_some()
            {
                return Err(UnitError::DuplicateComponent(comp.id.clone()));
            }
            for c in &comp.cells {
                insert_cell(
                    &mut cells,
                    CellAddr::Component {
                        component: comp.id.clone(),
                        cell: c.id.clone(),
                    },
                    c,
                )?;
            }
        }
        let mut capabilities = BTreeMap::new();
        for cap in &spec.capabilities {
            if capabilities
                .insert(cap.key.clone(), cap.rules.clone())
                .is_some()
            {
                return Err(UnitError::DuplicateCapability(cap.key.clone()));
            }
        }

        let unit = TacticalUnit {
            object,
            schema: spec.definition.clone(),
            sections,
            cells,
            components,
            capabilities,
            mobility: spec.mobility.clone(),
            revision: 0,
            rules: spec.rules.clone(),
        };
        unit.validate()?;
        Ok(unit)
    }

    /// Validate the intrinsic invariants: at least one section, every cell in
    /// bounds, and every intra-instance reference (facet/component/slot in a cell
    /// address, a component install point, a capability source) resolves. Runs on
    /// construction AND deserialize; needs no registry.
    fn validate(&self) -> Result<(), UnitError> {
        validate_unit_definition_kinds(self)?;
        validate_unit_definition_owners(self)?;
        validate_unit_bounds(self)?;
        if self.sections.is_empty() {
            return Err(UnitError::EmptyUnit);
        }
        for (addr, cell) in &self.cells {
            cell.check()?;
            self.resolve_cell_addr(addr)?;
        }
        for (id, comp) in &self.components {
            self.resolve_install(&comp.install)
                .map_err(|_| UnitError::UnresolvedInstall(id.clone()))?;
        }
        for key in self.capabilities.keys() {
            if let Some(src) = &key.source
                && !self.components.contains_key(src)
            {
                return Err(UnitError::UnresolvedCapabilitySource(src.clone()));
            }
        }
        Ok(())
    }

    fn resolve_cell_addr(&self, addr: &CellAddr) -> Result<(), UnitError> {
        match addr {
            CellAddr::Unit { .. } => Ok(()),
            CellAddr::Section { section, .. } => self.require_section(section).map(|_| ()),
            CellAddr::Facet { section, facet, .. } => {
                let s = self.require_section(section)?;
                if s.facets.contains(facet) {
                    Ok(())
                } else {
                    Err(UnitError::UnknownFacet {
                        section: section.clone(),
                        facet: facet.clone(),
                    })
                }
            }
            CellAddr::Component { component, .. } => {
                if self.components.contains_key(component) {
                    Ok(())
                } else {
                    Err(UnitError::UnknownComponent(component.clone()))
                }
            }
        }
    }

    fn resolve_install(&self, install: &InstallPoint) -> Result<(), UnitError> {
        match install {
            InstallPoint::Unit => Ok(()),
            InstallPoint::Section(section) => self.require_section(section).map(|_| ()),
            InstallPoint::Slot { section, slot } => {
                let s = self.require_section(section)?;
                if s.slots.contains(slot) {
                    Ok(())
                } else {
                    Err(UnitError::UnknownSlot {
                        section: section.clone(),
                        slot: slot.clone(),
                    })
                }
            }
        }
    }

    fn require_section(&self, section: &SectionId) -> Result<&SectionState, UnitError> {
        self.sections
            .get(section)
            .ok_or_else(|| UnitError::UnknownSection(section.clone()))
    }

    // --- accessors ---

    /// The unit's object id.
    #[must_use]
    pub fn object(&self) -> ObjId {
        self.object
    }
    /// The schema/provenance reference.
    #[must_use]
    pub fn schema(&self) -> &DefinitionRef {
        &self.schema
    }
    /// Every exact definition required to interpret this instance, including
    /// its root unit definition. The ordered result is deduplicated for direct
    /// registry validation during boot.
    #[must_use]
    pub fn definition_references(&self) -> Vec<DefinitionRef> {
        let mut references = BTreeSet::new();
        references.insert(self.schema.clone());
        references.insert(self.rules.schema().clone());
        references.extend(
            self.components
                .values()
                .map(|component| component.definition.clone()),
        );
        references.extend(
            self.capabilities
                .values()
                .map(|rules| rules.schema().clone()),
        );
        if let Some(profile) = &self.mobility {
            references.extend(
                profile
                    .modes()
                    .iter()
                    .map(|mode| mode.rules.schema().clone()),
            );
        }
        references.into_iter().collect()
    }
    /// The applied-plan count (0 for a fresh unit).
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// A cell by address.
    #[must_use]
    pub fn cell(&self, addr: &CellAddr) -> Option<&ScalarCell> {
        self.cells.get(addr)
    }
    /// A section's state.
    #[must_use]
    pub fn section(&self, id: &SectionId) -> Option<&SectionState> {
        self.sections.get(id)
    }
    /// A component by id.
    #[must_use]
    pub fn component(&self, id: &ComponentId) -> Option<&InstalledComponent> {
        self.components.get(id)
    }
    /// The typed mobility profile (the ONE capability whose structure the engine
    /// understands). `tactics.mobility` in the capability set is the opaque
    /// "movement functional now?" bit consumers read; this field is the modes.
    #[must_use]
    pub fn mobility(&self) -> Option<&MobilityProfile> {
        self.mobility.as_ref()
    }
    /// The opaque payload of a capability, if present.
    #[must_use]
    pub fn capability(&self, key: &CapabilityKey) -> Option<&RulesValue> {
        self.capabilities.get(key)
    }
    /// Whether the unit currently has the capability `id` from ANY source.
    #[must_use]
    pub fn has_capability(&self, id: &CapabilityId) -> bool {
        self.capabilities.keys().any(|k| &k.id == id)
    }
    /// All capability keys (ordered).
    pub fn capabilities(&self) -> impl Iterator<Item = &CapabilityKey> {
        self.capabilities.keys()
    }
    /// All sections (ordered by id) — for inspection/rendering.
    pub fn sections(&self) -> impl Iterator<Item = (&SectionId, &SectionState)> {
        self.sections.iter()
    }
    /// All cells (ordered by address) — for inspection/rendering.
    pub fn cells(&self) -> impl Iterator<Item = (&CellAddr, &ScalarCell)> {
        self.cells.iter()
    }
    /// All installed components (ordered by id) — for inspection/rendering.
    pub fn components(&self) -> impl Iterator<Item = (&ComponentId, &InstalledComponent)> {
        self.components.iter()
    }

    /// Normalize a due mobility transition (M31 `settle`) — call before ANY
    /// whole-unit serialize so a due-but-unsettled `Switching` never persists.
    pub fn settle_mobility(&mut self, now: i64) {
        if let Some(m) = &mut self.mobility {
            m.settle(now);
        }
    }

    // --- the damage pipeline (private per-op applier used by `apply`) ---

    fn apply_op(&mut self, op: &UnitMutationOp) -> Result<JournalEntry, UnitError> {
        match op {
            UnitMutationOp::AdjustCell { addr, delta } => {
                let cell = self
                    .cells
                    .get_mut(addr)
                    .ok_or_else(|| UnitError::UnknownCell(addr.clone()))?;
                let from = cell.current;
                let to = from.checked_add(*delta).ok_or(UnitError::Overflow)?;
                if to < cell.min || to > cell.max {
                    return Err(UnitError::OutOfBounds {
                        addr: addr.clone(),
                        value: to,
                        min: cell.min,
                        max: cell.max,
                    });
                }
                cell.current = to;
                Ok(JournalEntry::Cell {
                    addr: addr.clone(),
                    from,
                    to,
                })
            }
            UnitMutationOp::SetCell { addr, value } => {
                let cell = self
                    .cells
                    .get_mut(addr)
                    .ok_or_else(|| UnitError::UnknownCell(addr.clone()))?;
                let from = cell.current;
                if *value < cell.min || *value > cell.max {
                    return Err(UnitError::OutOfBounds {
                        addr: addr.clone(),
                        value: *value,
                        min: cell.min,
                        max: cell.max,
                    });
                }
                cell.current = *value;
                Ok(JournalEntry::Cell {
                    addr: addr.clone(),
                    from,
                    to: *value,
                })
            }
            UnitMutationOp::SetSectionCondition { section, condition } => {
                let s = self
                    .sections
                    .get_mut(section)
                    .ok_or_else(|| UnitError::UnknownSection(section.clone()))?;
                let from = std::mem::replace(&mut s.condition, condition.clone());
                Ok(JournalEntry::SectionCondition {
                    section: section.clone(),
                    from,
                    to: condition.clone(),
                })
            }
            UnitMutationOp::SetComponentCondition {
                component,
                condition,
            } => {
                let c = self
                    .components
                    .get_mut(component)
                    .ok_or_else(|| UnitError::UnknownComponent(component.clone()))?;
                let from = std::mem::replace(&mut c.condition, condition.clone());
                Ok(JournalEntry::ComponentCondition {
                    component: component.clone(),
                    from,
                    to: condition.clone(),
                })
            }
            UnitMutationOp::SetComponentAsset { component, asset } => {
                let c = self
                    .components
                    .get_mut(component)
                    .ok_or_else(|| UnitError::UnknownComponent(component.clone()))?;
                let from = std::mem::replace(&mut c.asset, *asset);
                Ok(JournalEntry::ComponentAsset { component: component.clone(), from, to: *asset })
            }
            UnitMutationOp::GrantCapability { key, rules } => {
                // Grant is the one op that introduces a fresh referent, so it
                // must re-check the source resolves — otherwise apply could bless
                // a unit that its own validate()/deserialize would reject.
                if let Some(src) = &key.source
                    && !self.components.contains_key(src)
                {
                    return Err(UnitError::UnresolvedCapabilitySource(src.clone()));
                }
                check_definition_kind(rules.schema(), "capability rules", "schema")?;
                check_definition_owner(&self.schema, rules.schema(), "capability rules")?;
                let replaced = self
                    .capabilities
                    .insert(key.clone(), rules.clone())
                    .is_some();
                Ok(JournalEntry::CapabilityGranted {
                    key: key.clone(),
                    replaced,
                })
            }
            UnitMutationOp::RevokeCapability { key } => {
                let was_present = self.capabilities.remove(key).is_some();
                Ok(JournalEntry::CapabilityRevoked {
                    key: key.clone(),
                    was_present,
                })
            }
        }
    }
}

/// Insert a cell built from a spec, checking bounds and address uniqueness.
fn insert_cell(
    cells: &mut BTreeMap<CellAddr, ScalarCell>,
    addr: CellAddr,
    spec: &CellSpec,
) -> Result<(), UnitError> {
    let cell = ScalarCell {
        min: spec.min,
        max: spec.max,
        current: spec.init,
    };
    cell.check()?;
    if cells.insert(addr.clone(), cell).is_some() {
        return Err(UnitError::DuplicateCell(addr));
    }
    Ok(())
}

// ----------------------------------------------------------------------
// The damage pipeline
// ----------------------------------------------------------------------

/// A closed, typed mutation over open ids — the engine's ENTIRE damage
/// vocabulary. A consequence outside this set is the signal to grow it
/// deliberately (e.g. structural install/remove verbs arrive with the M33/
/// economy cut), never to teach the engine a formula.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UnitMutationOp {
    /// Add `delta` (may be negative) to a cell; refuses on overflow or if the
    /// result leaves `[min,max]`.
    AdjustCell {
        /// The cell.
        addr: CellAddr,
        /// The signed change.
        delta: i64,
    },
    /// Set a cell to an absolute in-bounds value.
    SetCell {
        /// The cell.
        addr: CellAddr,
        /// The new value (must be in `[min,max]`).
        value: i64,
    },
    /// Set a section's opaque condition.
    SetSectionCondition {
        /// The section.
        section: SectionId,
        /// The new condition.
        condition: ConditionId,
    },
    /// Set a component's opaque condition.
    SetComponentCondition {
        /// The component.
        component: ComponentId,
        /// The new condition.
        condition: ConditionId,
    },
    /// Point a component at an economic asset object, or clear the pointer
    /// (E3c). Idempotent; an unknown component is refused like every other
    /// unresolved referent. This is the ONLY way
    /// [`InstalledComponent::asset`] changes after `from_spec`, so a scrub is a
    /// real revision bump a queued order that captured `(subject, gen)` can see,
    /// not a silent doc edit.
    SetComponentAsset {
        /// The component.
        component: ComponentId,
        /// The asset object, or `None` to clear.
        asset: Option<ObjId>,
    },
    /// Grant (or overwrite) a capability with an opaque payload (idempotent).
    GrantCapability {
        /// The capability key.
        key: CapabilityKey,
        /// The opaque payload.
        rules: RulesValue,
    },
    /// Revoke a capability (idempotent — revoking an absent one is a no-op).
    RevokeCapability {
        /// The capability key.
        key: CapabilityKey,
    },
}

/// A flat, ruleset-authored mutation plan. The ruleset has already expanded any
/// cascade into explicit ops; the engine applies them in order, atomically.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnitMutationPlan {
    /// The ops, applied in this order.
    ops: Vec<UnitMutationOp>,
    /// If `Some(n)`, the plan is refused unless `unit.revision == n` (the F3
    /// lease-revision guard; optional in M32 where plan+apply run atomically).
    expected_revision: Option<u64>,
    /// An optional semantic tag recorded in the journal.
    cause: Option<SmolStr>,
}

impl UnitMutationPlan {
    /// Construct a bounded mutation plan.
    ///
    /// # Errors
    /// [`UnitError::LimitExceeded`] if `ops` exceeds
    /// [`MAX_UNIT_MUTATION_OPERATIONS`].
    pub fn new(
        ops: Vec<UnitMutationOp>,
        expected_revision: Option<u64>,
        cause: Option<SmolStr>,
    ) -> Result<Self, UnitError> {
        check_mutation_operation_limit(ops.len())?;
        Ok(Self {
            ops,
            expected_revision,
            cause,
        })
    }

    /// The ordered operations in this plan.
    #[must_use]
    pub fn ops(&self) -> &[UnitMutationOp] {
        &self.ops
    }

    /// The optimistic revision guard, if present.
    #[must_use]
    pub const fn expected_revision(&self) -> Option<u64> {
        self.expected_revision
    }

    /// Return this bounded plan with a different optimistic revision guard.
    #[must_use]
    pub fn with_expected_revision(mut self, expected_revision: Option<u64>) -> Self {
        self.expected_revision = expected_revision;
        self
    }

    /// The optional semantic journal tag.
    #[must_use]
    pub fn cause(&self) -> Option<&str> {
        self.cause.as_deref()
    }
}

/// Serde adapter that enforces the operation ceiling while decoding the
/// sequence. Its allocation hint is checked before `Vec::with_capacity`, and
/// the unknown-length path refuses the first operation over the ceiling rather
/// than collecting the complete hostile sequence.
struct BoundedMutationOps(Vec<UnitMutationOp>);

impl<'de> Deserialize<'de> for BoundedMutationOps {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BoundedMutationOpsVisitor;

        impl<'de> Visitor<'de> for BoundedMutationOpsVisitor {
            type Value = BoundedMutationOps;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    formatter,
                    "at most {MAX_UNIT_MUTATION_OPERATIONS} tactical mutation operations"
                )
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let hint = seq.size_hint();
                if let Some(count) = hint {
                    check_mutation_operation_limit(count).map_err(A::Error::custom)?;
                }
                let mut ops = Vec::with_capacity(hint.unwrap_or(0));
                while let Some(op) = seq.next_element()? {
                    check_mutation_operation_limit(ops.len() + 1).map_err(A::Error::custom)?;
                    ops.push(op);
                }
                Ok(BoundedMutationOps(ops))
            }
        }

        deserializer.deserialize_seq(BoundedMutationOpsVisitor)
    }
}

#[derive(Deserialize)]
struct UnitMutationPlanRepr {
    ops: BoundedMutationOps,
    #[serde(default)]
    expected_revision: Option<u64>,
    #[serde(default)]
    cause: Option<SmolStr>,
}

impl<'de> Deserialize<'de> for UnitMutationPlan {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let repr = UnitMutationPlanRepr::deserialize(deserializer)?;
        Self::new(repr.ops.0, repr.expected_revision, repr.cause).map_err(D::Error::custom)
    }
}

/// The observer-facing record of an APPLIED plan (its real job is the M33
/// epistemic/panel layer). It is NOT the replay substrate — replay re-derives
/// state from a base unit plus the plan sequence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Journal {
    /// The unit's object.
    pub object: ObjId,
    /// Revision before applying.
    pub from_revision: u64,
    /// Revision after applying.
    pub to_revision: u64,
    /// The `now` passed to `apply` (unix seconds).
    pub now: i64,
    /// The plan's semantic tag, if any.
    pub cause: Option<SmolStr>,
    /// One entry per applied op, in order.
    pub entries: Vec<JournalEntry>,
}

/// One applied effect (the ACTUAL delta, e.g. the real capability-set change).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum JournalEntry {
    /// A cell changed value.
    Cell {
        /// The cell.
        addr: CellAddr,
        /// Value before.
        from: i64,
        /// Value after.
        to: i64,
    },
    /// A section's condition changed.
    SectionCondition {
        /// The section.
        section: SectionId,
        /// Before.
        from: ConditionId,
        /// After.
        to: ConditionId,
    },
    /// A component's condition changed.
    ComponentCondition {
        /// The component.
        component: ComponentId,
        /// Before.
        from: ConditionId,
        /// After.
        to: ConditionId,
    },
    /// A component's economic-asset pointer changed (E3c).
    ComponentAsset {
        /// The component.
        component: ComponentId,
        /// Before.
        from: Option<ObjId>,
        /// After.
        to: Option<ObjId>,
    },
    /// A capability was granted (`replaced` = an entry already existed).
    CapabilityGranted {
        /// The key.
        key: CapabilityKey,
        /// Whether it replaced an existing entry.
        replaced: bool,
    },
    /// A capability revoke was applied (`was_present` = it actually existed).
    CapabilityRevoked {
        /// The key.
        key: CapabilityKey,
        /// Whether it was present.
        was_present: bool,
    },
}

/// Validate and atomically apply a ruleset-authored plan to `unit`, returning the
/// mutated unit (with `revision` bumped by 1) and the journal. The op sequence is
/// folded on a scratch CLONE, re-checking bounds and references after each step;
/// ANY failure returns [`Err`] and the caller's `unit` is untouched (refusal =
/// zero mutation). Ops apply in authored order, so two adjusts on one cell
/// compose sequentially.
///
/// # Errors
/// A [`UnitError`] on a stale `expected_revision`, an unresolved referent, an
/// unauthorized cross-owner capability grant, an out-of-bounds/overflowing
/// scalar, or a revision overflow — with no mutation.
pub fn apply(
    unit: &TacticalUnit,
    plan: &UnitMutationPlan,
    now: i64,
) -> Result<(TacticalUnit, Journal), UnitError> {
    // Defense in depth for any in-crate construction path: reject before the
    // scratch clone, journal allocation, or operation iteration.
    check_mutation_operation_limit(plan.ops.len())?;
    if let Some(expected) = plan.expected_revision
        && unit.revision != expected
    {
        return Err(UnitError::StaleRevision {
            expected,
            actual: unit.revision,
        });
    }
    let mut post = unit.clone();
    let mut entries = Vec::with_capacity(plan.ops.len());
    for op in &plan.ops {
        entries.push(post.apply_op(op)?);
    }
    let to = unit
        .revision
        .checked_add(1)
        .ok_or(UnitError::RevisionOverflow)?;
    post.revision = to;
    let journal = Journal {
        object: unit.object,
        from_revision: unit.revision,
        to_revision: to,
        now,
        cause: plan.cause.clone(),
        entries,
    };
    Ok((post, journal))
}

fn check_mutation_operation_limit(count: usize) -> Result<(), UnitError> {
    check_limit(
        "unit mutation plan operations",
        count,
        MAX_UNIT_MUTATION_OPERATIONS,
    )
}

fn check_limit(collection: &'static str, count: usize, max: usize) -> Result<(), UnitError> {
    if count > max {
        Err(UnitError::LimitExceeded {
            collection,
            count,
            max,
        })
    } else {
        Ok(())
    }
}

fn check_definition_kind(
    reference: &DefinitionRef,
    field: &'static str,
    expected: &'static str,
) -> Result<(), UnitError> {
    if reference.key().kind.as_str() == expected {
        Ok(())
    } else {
        Err(UnitError::DefinitionKindMismatch {
            field,
            expected,
            found: reference.key().kind.clone(),
        })
    }
}

fn check_definition_owner(
    root: &DefinitionRef,
    dependency: &DefinitionRef,
    field: &'static str,
) -> Result<(), UnitError> {
    if dependency.key().owner == root.key().owner {
        Ok(())
    } else {
        Err(UnitError::CrossOwnerDefinitionDependency {
            field,
            root: Box::new(root.clone()),
            dependency: Box::new(dependency.clone()),
        })
    }
}

fn validate_body_definition_kinds(body: &UnitSpecBody) -> Result<(), UnitError> {
    check_definition_kind(body.rules.schema(), "unit rules", "schema")?;
    for component in &body.components {
        check_definition_kind(&component.definition, "component definition", "component")?;
    }
    for capability in &body.capabilities {
        check_definition_kind(capability.rules.schema(), "capability rules", "schema")?;
    }
    if let Some(profile) = &body.mobility {
        for mode in profile.modes() {
            check_definition_kind(mode.rules.schema(), "mobility mode rules", "schema")?;
        }
    }
    Ok(())
}

fn validate_spec_definition_kinds(spec: &UnitSpec) -> Result<(), UnitError> {
    check_definition_kind(&spec.definition, "unit definition", "unit")?;
    check_definition_kind(spec.rules.schema(), "unit rules", "schema")?;
    for component in &spec.components {
        check_definition_kind(&component.definition, "component definition", "component")?;
    }
    for capability in &spec.capabilities {
        check_definition_kind(capability.rules.schema(), "capability rules", "schema")?;
    }
    if let Some(profile) = &spec.mobility {
        for mode in profile.modes() {
            check_definition_kind(mode.rules.schema(), "mobility mode rules", "schema")?;
        }
    }
    Ok(())
}

/// Interim fail-closed composition rule: a unit definition may only embed
/// definitions owned by its root owner. The K1 bridge registry describes an
/// interaction between owners, but `UnitSpecBody` does not yet carry an exact
/// bridge/interaction binding for nested structural composition. Admitting a
/// cross-owner reference here would therefore bypass the bridge contract.
///
/// A future bridged-composition format can replace this guard once the body
/// records (and boot validates) the exact directional bridge and interaction.
fn validate_spec_definition_owners(spec: &UnitSpec) -> Result<(), UnitError> {
    check_definition_owner(&spec.definition, spec.rules.schema(), "unit rules")?;
    for component in &spec.components {
        check_definition_owner(
            &spec.definition,
            &component.definition,
            "component definition",
        )?;
    }
    for capability in &spec.capabilities {
        check_definition_owner(
            &spec.definition,
            capability.rules.schema(),
            "capability rules",
        )?;
    }
    if let Some(profile) = &spec.mobility {
        for mode in profile.modes() {
            check_definition_owner(&spec.definition, mode.rules.schema(), "mobility mode rules")?;
        }
    }
    Ok(())
}

fn validate_unit_definition_kinds(unit: &TacticalUnit) -> Result<(), UnitError> {
    check_definition_kind(&unit.schema, "unit definition", "unit")?;
    check_definition_kind(unit.rules.schema(), "unit rules", "schema")?;
    for component in unit.components.values() {
        check_definition_kind(&component.definition, "component definition", "component")?;
    }
    for rules in unit.capabilities.values() {
        check_definition_kind(rules.schema(), "capability rules", "schema")?;
    }
    if let Some(profile) = &unit.mobility {
        for mode in profile.modes() {
            check_definition_kind(mode.rules.schema(), "mobility mode rules", "schema")?;
        }
    }
    Ok(())
}

fn validate_unit_definition_owners(unit: &TacticalUnit) -> Result<(), UnitError> {
    check_definition_owner(&unit.schema, unit.rules.schema(), "unit rules")?;
    for component in unit.components.values() {
        check_definition_owner(&unit.schema, &component.definition, "component definition")?;
    }
    for rules in unit.capabilities.values() {
        check_definition_owner(&unit.schema, rules.schema(), "capability rules")?;
    }
    if let Some(profile) = &unit.mobility {
        for mode in profile.modes() {
            check_definition_owner(&unit.schema, mode.rules.schema(), "mobility mode rules")?;
        }
    }
    Ok(())
}

fn validate_spec_bounds(spec: &UnitSpec) -> Result<(), UnitError> {
    check_limit("sections", spec.sections.len(), MAX_UNIT_SECTIONS)?;
    check_limit(
        "facets",
        spec.sections
            .iter()
            .fold(0_usize, |n, section| n.saturating_add(section.facets.len())),
        MAX_UNIT_FACETS,
    )?;
    check_limit(
        "install slots",
        spec.sections
            .iter()
            .fold(0_usize, |n, section| n.saturating_add(section.slots.len())),
        MAX_UNIT_SLOTS,
    )?;
    let section_cells = spec.sections.iter().fold(0_usize, |n, section| {
        let facet_cells = section
            .facets
            .iter()
            .fold(0_usize, |m, facet| m.saturating_add(facet.cells.len()));
        n.saturating_add(section.cells.len())
            .saturating_add(facet_cells)
    });
    let component_cells = spec.components.iter().fold(0_usize, |n, component| {
        n.saturating_add(component.cells.len())
    });
    check_limit(
        "scalar cells",
        spec.unit_cells
            .len()
            .saturating_add(section_cells)
            .saturating_add(component_cells),
        MAX_UNIT_CELLS,
    )?;
    check_limit("components", spec.components.len(), MAX_UNIT_COMPONENTS)?;
    check_limit(
        "capabilities",
        spec.capabilities.len(),
        MAX_UNIT_CAPABILITIES,
    )?;
    check_limit(
        "mobility modes",
        spec.mobility
            .as_ref()
            .map_or(0, |profile| profile.modes().len()),
        MAX_UNIT_MOBILITY_MODES,
    )
}

fn validate_unit_bounds(unit: &TacticalUnit) -> Result<(), UnitError> {
    check_limit("sections", unit.sections.len(), MAX_UNIT_SECTIONS)?;
    check_limit(
        "facets",
        unit.sections
            .values()
            .fold(0_usize, |n, section| n.saturating_add(section.facets.len())),
        MAX_UNIT_FACETS,
    )?;
    check_limit(
        "install slots",
        unit.sections
            .values()
            .fold(0_usize, |n, section| n.saturating_add(section.slots.len())),
        MAX_UNIT_SLOTS,
    )?;
    check_limit("scalar cells", unit.cells.len(), MAX_UNIT_CELLS)?;
    check_limit("components", unit.components.len(), MAX_UNIT_COMPONENTS)?;
    check_limit(
        "capabilities",
        unit.capabilities.len(),
        MAX_UNIT_CAPABILITIES,
    )?;
    check_limit(
        "mobility modes",
        unit.mobility
            .as_ref()
            .map_or(0, |profile| profile.modes().len()),
        MAX_UNIT_MOBILITY_MODES,
    )
}

// ----------------------------------------------------------------------
// Ruleset-authored schema (the input to `from_spec`)
// ----------------------------------------------------------------------

/// A bounded-cell definition: id, immutable bounds, and starting value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellSpec {
    /// The cell id (unique within its host).
    pub id: CellId,
    /// Inclusive lower bound.
    pub min: i64,
    /// Inclusive upper bound.
    pub max: i64,
    /// Starting value (must be in `[min,max]`).
    pub init: i64,
}

/// A facet definition (an exposed surface with its own cells, e.g. `rear.armor`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FacetSpec {
    /// The facet id (unique within its section).
    pub id: FacetId,
    /// Cells on this facet.
    #[serde(default)]
    pub cells: Vec<CellSpec>,
}

/// A section definition: condition, facets, install slots, and section-level cells.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionSpec {
    /// The section id (unique within the unit).
    pub id: SectionId,
    /// Initial condition.
    pub condition: ConditionId,
    /// Facets (exposed surfaces).
    #[serde(default)]
    pub facets: Vec<FacetSpec>,
    /// Internal install slots.
    #[serde(default)]
    pub slots: Vec<InstallSlotId>,
    /// Section-level cells (e.g. `structure`).
    #[serde(default)]
    pub cells: Vec<CellSpec>,
}

/// A component definition to install at build time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentSpec {
    /// The component instance id (unique within the unit).
    pub id: ComponentId,
    /// The component definition (provenance).
    pub definition: DefinitionRef,
    /// Where it installs.
    pub install: InstallPoint,
    /// Initial condition.
    pub condition: ConditionId,
    /// Component-level cells (e.g. an ammo pool).
    #[serde(default)]
    pub cells: Vec<CellSpec>,
    /// An optional economic asset this component IS.
    #[serde(default)]
    pub asset: Option<ObjId>,
}

/// An initial capability entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityInit {
    /// The key.
    pub key: CapabilityKey,
    /// The opaque payload.
    pub rules: RulesValue,
}

/// The ruleset-authored anatomy schema a [`TacticalUnit`] is built from. At least
/// one section is required (a coarse unit is one "chassis" section plus
/// unit-level pools).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnitSpec {
    /// Schema id + version (provenance).
    pub definition: DefinitionRef,
    /// The sections (>= 1).
    pub sections: Vec<SectionSpec>,
    /// Unit-level pools.
    #[serde(default)]
    pub unit_cells: Vec<CellSpec>,
    /// Components installed at build time.
    #[serde(default)]
    pub components: Vec<ComponentSpec>,
    /// Initial capabilities.
    #[serde(default)]
    pub capabilities: Vec<CapabilityInit>,
    /// The typed mobility profile, if this unit moves.
    #[serde(default)]
    pub mobility: Option<MobilityProfile>,
    /// The one unit-level opaque rules blob.
    pub rules: RulesValue,
}

/// The canonical payload of a unit definition. It deliberately excludes the
/// definition's own [`DefinitionRef`], so registering the descriptor cannot
/// create a recursive self-digest. Consequently a standalone body has no root
/// owner against which to authorize its dependencies; [`UnitSpec`] validation
/// and [`UnitCatalogBuilder::add`] apply the interim same-owner rule after
/// [`with_definition`](Self::with_definition) supplies that identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnitSpecBody {
    /// The sections (>= 1).
    pub sections: Vec<SectionSpec>,
    /// Unit-level pools.
    #[serde(default)]
    pub unit_cells: Vec<CellSpec>,
    /// Components installed at build time.
    #[serde(default)]
    pub components: Vec<ComponentSpec>,
    /// Initial capabilities.
    #[serde(default)]
    pub capabilities: Vec<CapabilityInit>,
    /// The typed mobility profile, if this unit moves.
    #[serde(default)]
    pub mobility: Option<MobilityProfile>,
    /// The one unit-level opaque rules blob.
    pub rules: RulesValue,
}

impl UnitSpecBody {
    /// Wrap this registered body with its exact identity.
    #[must_use]
    pub fn with_definition(self, definition: DefinitionRef) -> UnitSpec {
        UnitSpec {
            definition,
            sections: self.sections,
            unit_cells: self.unit_cells,
            components: self.components,
            capabilities: self.capabilities,
            mobility: self.mobility,
            rules: self.rules,
        }
    }

    /// Every exact external definition used by this body. The root definition
    /// is absent by construction. Results are sorted and deduplicated so the
    /// descriptor digest is independent of authoring traversal order.
    #[must_use]
    pub fn definition_references(&self) -> Vec<DefinitionRef> {
        let mut references = BTreeSet::new();
        references.insert(self.rules.schema().clone());
        for component in &self.components {
            references.insert(component.definition.clone());
        }
        for capability in &self.capabilities {
            references.insert(capability.rules.schema().clone());
        }
        if let Some(profile) = &self.mobility {
            for mode in profile.modes() {
                references.insert(mode.rules.schema().clone());
            }
        }
        references.into_iter().collect()
    }

    /// Build the owner-free registration descriptor for this body. The
    /// definition registry stamps the owner and computes the exact digest.
    ///
    /// # Errors
    /// Returns [`UnitError::DefinitionDescriptor`] when the version is zero or
    /// the typed body cannot be serialized.
    pub fn descriptor(
        &self,
        kind: DefinitionKind,
        id: DefinitionId,
        version: u32,
    ) -> Result<DefinitionDescriptor, UnitError> {
        if kind.as_str() != "unit" {
            return Err(UnitError::DefinitionKindMismatch {
                field: "unit definition",
                expected: "unit",
                found: kind,
            });
        }
        validate_body_definition_kinds(self)?;
        DefinitionDescriptor::from_serializable(kind, id, version, self)
            .map(|descriptor| descriptor.with_references(self.definition_references()))
            .map_err(|error| UnitError::DefinitionDescriptor(error.to_string()))
    }
}

impl UnitSpec {
    /// Copy the non-self-referential definition payload.
    #[must_use]
    pub fn body(&self) -> UnitSpecBody {
        UnitSpecBody {
            sections: self.sections.clone(),
            unit_cells: self.unit_cells.clone(),
            components: self.components.clone(),
            capabilities: self.capabilities.clone(),
            mobility: self.mobility.clone(),
            rules: self.rules.clone(),
        }
    }

    /// Recreate the descriptor whose registration must produce this spec's
    /// declared exact identity.
    ///
    /// # Errors
    /// Returns a typed definition kind/ownership error or
    /// [`UnitError::DefinitionDescriptor`] if descriptor construction fails.
    pub fn descriptor(&self) -> Result<DefinitionDescriptor, UnitError> {
        validate_spec_definition_kinds(self)?;
        validate_spec_definition_owners(self)?;
        self.body().descriptor(
            self.definition.key().kind.clone(),
            self.definition.key().id.clone(),
            self.definition.version(),
        )
    }

    /// Reconcile a decoded instance against this exact definition's immutable
    /// shape. This is deliberately stronger than [`TacticalUnit`]'s intrinsic
    /// validation: serde can prove that an instance is internally coherent,
    /// but only its selected `UnitSpec` can prove that persisted bytes did not
    /// invent a different anatomy while retaining the same root reference.
    ///
    /// Mutable instance state is intentionally ignored: object/revision,
    /// section and component conditions, scalar current values, mobility mode
    /// state, capability presence/data, economic asset pointers, and opaque
    /// rules data. Definition-bearing fields remain exact: the root, component
    /// definitions and installs, unit/mobility/baseline-capability rules schemas,
    /// plus all section/facet/slot/cell/component/mobility topology and cell
    /// bounds.
    ///
    /// # Errors
    /// Returns the first intrinsic [`UnitError`] or a stable
    /// [`UnitError::DefinitionShapeMismatch`] category.
    pub fn validate_instance(&self, unit: &TacticalUnit) -> Result<(), UnitError> {
        // Recheck both inputs so this method is safe independently of catalog
        // construction or serde call sites.
        unit.validate()?;
        let baseline = TacticalUnit::from_spec(self, unit.object)?;
        let mismatch = |field| UnitError::DefinitionShapeMismatch { field };

        if unit.schema != self.definition {
            return Err(mismatch("unit definition"));
        }

        if unit.sections.len() != baseline.sections.len() {
            return Err(mismatch("section topology"));
        }
        for (id, expected) in &baseline.sections {
            let Some(actual) = unit.sections.get(id) else {
                return Err(mismatch("section topology"));
            };
            if actual.facets != expected.facets {
                return Err(mismatch("facet topology"));
            }
            if actual.slots != expected.slots {
                return Err(mismatch("install-slot topology"));
            }
        }

        if unit.cells.len() != baseline.cells.len() {
            return Err(mismatch("cell topology"));
        }
        for (address, expected) in &baseline.cells {
            let Some(actual) = unit.cells.get(address) else {
                return Err(mismatch("cell topology"));
            };
            if actual.min != expected.min || actual.max != expected.max {
                return Err(mismatch("cell bounds"));
            }
        }

        if unit.components.len() != baseline.components.len() {
            return Err(mismatch("component topology"));
        }
        for (id, expected) in &baseline.components {
            let Some(actual) = unit.components.get(id) else {
                return Err(mismatch("component topology"));
            };
            if actual.definition != expected.definition {
                return Err(mismatch("component definition"));
            }
            if actual.install != expected.install {
                return Err(mismatch("component install"));
            }
        }

        if unit.rules.schema() != baseline.rules.schema() {
            return Err(mismatch("unit rules schema"));
        }
        // Capability entries are mutable (grant/revoke). If a definition's
        // baseline key is present, however, its schema cannot be silently
        // reinterpreted. Newly granted keys are exact-registry checked by boot.
        for (key, expected) in &baseline.capabilities {
            if let Some(actual) = unit.capabilities.get(key)
                && actual.schema() != expected.schema()
            {
                return Err(mismatch("capability rules schema"));
            }
        }

        match (&unit.mobility, &baseline.mobility) {
            (None, None) => {}
            (Some(actual), Some(expected)) => {
                if actual.modes().len() != expected.modes().len() {
                    return Err(mismatch("mobility mode topology"));
                }
                for (actual_mode, expected_mode) in actual.modes().iter().zip(expected.modes()) {
                    if actual_mode.id != expected_mode.id {
                        return Err(mismatch("mobility mode topology"));
                    }
                    if actual_mode.mediums != expected_mode.mediums
                        || actual_mode.locomotion != expected_mode.locomotion
                        || actual_mode.tags != expected_mode.tags
                        || actual_mode.nominal_speed_cms != expected_mode.nominal_speed_cms
                        || actual_mode.transition_s != expected_mode.transition_s
                    {
                        return Err(mismatch("mobility mode structure"));
                    }
                    if actual_mode.rules.schema() != expected_mode.rules.schema() {
                        return Err(mismatch("mobility rules schema"));
                    }
                }
            }
            _ => return Err(mismatch("mobility profile topology")),
        }

        Ok(())
    }
}

/// A ruleset-registered catalog of `UnitSpec`s by exact definition reference,
/// composed at module construction (builder → freeze, like M31's
/// `MobilityVocabulary`). The engine ships the MECHANISM empty; a ruleset (or
/// dev/test wiring) seeds the specs — no concrete spec is baked into the engine
/// crate.
#[derive(Clone, Debug, Default)]
pub struct UnitCatalog {
    specs: BTreeMap<DefinitionRef, UnitSpec>,
    /// Explicit context for ownerless legacy-v1 root ids. A deployment must
    /// opt each old id/version into exactly one exact modern definition; merely
    /// having a similarly named spec in the catalog never selects it.
    legacy_v1_aliases: BTreeMap<(DefinitionId, u32), DefinitionRef>,
    encoded_bytes: usize,
}

impl UnitCatalog {
    /// The one spec whose owner-local id is `def_id`. This compatibility lookup
    /// deliberately returns `None` when several exact versions are present;
    /// callers must then select an exact reference rather than guessing latest.
    #[must_use]
    pub fn get(&self, def_id: &str) -> Option<&UnitSpec> {
        let mut matches = self
            .specs
            .values()
            .filter(|spec| spec.definition.key().id.as_str() == def_id);
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }
    /// The spec registered under `definition`, including its digest, if any.
    #[must_use]
    pub fn get_exact(&self, definition: &DefinitionRef) -> Option<&UnitSpec> {
        self.specs.get(definition)
    }
    /// Resolve one ownerless legacy-v1 root id through deployment-supplied
    /// context. This never searches specs or infers an owner/kind.
    #[must_use]
    pub fn legacy_v1_root(&self, id: &DefinitionId, version: u32) -> Option<&UnitSpec> {
        self.legacy_v1_aliases
            .get(&(id.clone(), version))
            .and_then(|definition| self.specs.get(definition))
    }
    /// Ordered deployment context captured by the legacy migration contract.
    /// Exact targets bind the mapping to each selected spec body/digest.
    pub fn legacy_v1_aliases(&self) -> impl Iterator<Item = (&DefinitionId, u32, &DefinitionRef)> {
        self.legacy_v1_aliases
            .iter()
            .map(|((id, version), target)| (id, *version, target))
    }
    /// Select the one catalog spec whose exact reference is the frozen shared
    /// registry's explicit current definition for its key. This never treats
    /// the greatest numeric version as current, and returns `None` if `def_id`
    /// is absent or ambiguous across owners/kinds.
    #[must_use]
    pub fn get_current(&self, def_id: &str, registry: &DefinitionRegistry) -> Option<&UnitSpec> {
        let mut matches = self.specs.values().filter(|spec| {
            spec.definition.key().id.as_str() == def_id
                && registry.current(spec.definition.key()) == Some(&spec.definition)
        });
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }
    /// All registered specs in exact-reference order.
    pub fn specs(&self) -> impl Iterator<Item = &UnitSpec> {
        self.specs.values()
    }
    /// The registered exact definition references (ordered).
    pub fn definitions(&self) -> impl Iterator<Item = &DefinitionRef> {
        self.specs.keys()
    }
    /// The registered owner-local definition ids (ordered by exact reference;
    /// an id appears once per registered version).
    pub fn ids(&self) -> impl Iterator<Item = &DefinitionId> {
        self.specs.keys().map(|definition| &definition.key().id)
    }
    /// Whether the catalog holds no specs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }
    /// Number of exact unit definitions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }
    /// Encoded bytes retained across all definition bodies.
    #[must_use]
    pub const fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }
}

/// Builder for a [`UnitCatalog`]: each added spec's mobility modes are validated
/// against a [`MobilityRegistry`](crate::mobility::MobilityRegistry) before the
/// catalog is frozen (closing the gap that `from_spec` does not registry-check
/// an embedded profile).
#[derive(Clone, Debug, Default)]
pub struct UnitCatalogBuilder {
    specs: BTreeMap<DefinitionRef, UnitSpec>,
    locators: BTreeMap<DefinitionLocator, DefinitionRef>,
    legacy_v1_aliases: BTreeMap<(DefinitionId, u32), DefinitionRef>,
    encoded_bytes: usize,
}

impl UnitCatalogBuilder {
    /// A fresh, empty builder.
    #[must_use]
    pub fn new() -> UnitCatalogBuilder {
        UnitCatalogBuilder::default()
    }
    /// Register one exact `spec`. Intrinsic `UnitSpec` validation uses the same
    /// construction path as runtime unit creation; each mobility mode is also
    /// checked against `mobility_reg`. Finally, the body descriptor is
    /// independently registered in a scratch shared registry and must reproduce
    /// the exact declared reference (including digest).
    ///
    /// # Errors
    /// A [`UnitError`] for a duplicate/conflicting exact definition, invalid
    /// structure or mobility, identity mismatch, or catalog size limit.
    pub fn add(
        &mut self,
        spec: UnitSpec,
        mobility_reg: &crate::mobility::MobilityRegistry,
    ) -> Result<&mut UnitCatalogBuilder, UnitError> {
        let definition = spec.definition.clone();
        let locator = definition.locator();
        if let Some(existing) = self.locators.get(&locator) {
            return if existing == &definition {
                Err(UnitError::DuplicateDefinition(definition))
            } else {
                Err(UnitError::DefinitionDigestConflict {
                    locator,
                    registered: Box::new(existing.clone()),
                    attempted: Box::new(definition),
                })
            };
        }
        if self.specs.len() >= MAX_UNIT_CATALOG_DEFINITIONS {
            return Err(UnitError::LimitExceeded {
                collection: "unit catalog definitions",
                count: self.specs.len().saturating_add(1),
                max: MAX_UNIT_CATALOG_DEFINITIONS,
            });
        }

        // Catalog registration is itself an authorization boundary. Do not
        // rely solely on construction below to reject another owner's nested
        // definition when no exact bridge binding can be presented.
        validate_spec_definition_owners(&spec)?;

        // Full intrinsic validation. Using the real construction path prevents
        // catalog acceptance and later `@unit/create` from disagreeing.
        let _ = TacticalUnit::from_spec(&spec, ObjId(0))?;
        if let Some(m) = &spec.mobility {
            for mode in m.modes() {
                mobility_reg.validate_mode(mode)?;
            }
        }

        let body = spec.body();
        let encoded_bytes = serde_json::to_vec(&body)
            .map_err(|error| UnitError::DefinitionDescriptor(error.to_string()))?
            .len();
        if encoded_bytes > MAX_UNIT_SPEC_BYTES {
            return Err(UnitError::LimitExceeded {
                collection: "unit definition bytes",
                count: encoded_bytes,
                max: MAX_UNIT_SPEC_BYTES,
            });
        }
        let next_bytes =
            self.encoded_bytes
                .checked_add(encoded_bytes)
                .ok_or(UnitError::LimitExceeded {
                    collection: "unit catalog bytes",
                    count: usize::MAX,
                    max: MAX_UNIT_CATALOG_BYTES,
                })?;
        if next_bytes > MAX_UNIT_CATALOG_BYTES {
            return Err(UnitError::LimitExceeded {
                collection: "unit catalog bytes",
                count: next_bytes,
                max: MAX_UNIT_CATALOG_BYTES,
            });
        }

        // Recompute exact identity from the non-self-referential body. The
        // scratch builder need not freeze (the deployment-wide registry does
        // that reference-availability check); registration still applies the
        // canonical payload/reference digest and all per-definition bounds.
        let descriptor = spec.descriptor()?;
        let mut definitions = DefinitionRegistryBuilder::new();
        let computed = definitions
            .register(definition.key().owner.clone(), descriptor)
            .map_err(|error| UnitError::DefinitionDescriptor(error.to_string()))?;
        if computed != definition {
            return Err(UnitError::DefinitionIdentityMismatch {
                declared: Box::new(definition),
                computed: Box::new(computed),
            });
        }

        self.encoded_bytes = next_bytes;
        self.locators.insert(locator, computed.clone());
        self.specs.insert(computed, spec);
        Ok(self)
    }
    /// Explicitly map an ownerless legacy-v1 root id/version to one exact unit
    /// definition already added to this builder. This deployment context is
    /// required because old records carried no owner, kind, or digest; catalog
    /// contents alone are never sufficient authority to invent those fields.
    ///
    /// # Errors
    /// Returns [`UnitError::LegacyAliasTargetUnavailable`] if `target` is not
    /// already present, [`UnitError::DuplicateLegacyAlias`] if that old identity
    /// was configured twice, or a limit/kind error for invalid input.
    pub fn legacy_v1_alias(
        &mut self,
        legacy_id: DefinitionId,
        legacy_version: u32,
        target: DefinitionRef,
    ) -> Result<&mut UnitCatalogBuilder, UnitError> {
        if legacy_version == 0 {
            return Err(UnitError::LegacyAliasVersionZero);
        }
        check_definition_kind(&target, "legacy alias target", "unit")?;
        if !self.specs.contains_key(&target) {
            return Err(UnitError::LegacyAliasTargetUnavailable(Box::new(target)));
        }
        let key = (legacy_id, legacy_version);
        if self.legacy_v1_aliases.contains_key(&key) {
            return Err(UnitError::DuplicateLegacyAlias {
                id: key.0,
                version: key.1,
            });
        }
        if self.legacy_v1_aliases.len() >= MAX_UNIT_CATALOG_DEFINITIONS {
            return Err(UnitError::LimitExceeded {
                collection: "legacy unit aliases",
                count: self.legacy_v1_aliases.len().saturating_add(1),
                max: MAX_UNIT_CATALOG_DEFINITIONS,
            });
        }
        self.legacy_v1_aliases.insert(key, target);
        Ok(self)
    }
    /// Freeze into an immutable, runtime-shareable catalog.
    #[must_use]
    pub fn freeze(self) -> UnitCatalog {
        UnitCatalog {
            specs: self.specs,
            legacy_v1_aliases: self.legacy_v1_aliases,
            encoded_bytes: self.encoded_bytes,
        }
    }
}

// ----------------------------------------------------------------------

/// Why a tactical-unit construction or mutation was refused. Carries the
/// offending referent; `Display` suits a command layer (mirrors `SpatialError`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnitError {
    /// An id was empty after trimming.
    #[error("tactical id is empty")]
    EmptyId,
    /// An id exceeded [`TACTICAL_ID_MAX`].
    #[error("tactical id {0:?} is too long (max {TACTICAL_ID_MAX})")]
    IdTooLong(String),
    /// An id had bad characters or dot segments.
    #[error(
        "tactical id {0:?} must be lowercase [a-z0-9_.] with no leading/trailing/empty dot segment"
    )]
    MalformedId(String),
    /// Persisted or wire input was valid after normalization, but was not
    /// already in its one canonical byte representation.
    #[error("tactical id is not canonical; expected {canonical:?}")]
    NonCanonicalId {
        /// Its bounded canonical representation, useful for an authoring diagnostic.
        canonical: String,
    },
    /// A unit had no sections.
    #[error("a tactical unit must have at least one section")]
    EmptyUnit,
    /// A catalog spec's mobility mode referenced an undeclared atom.
    #[error("mobility: {0}")]
    Mobility(#[from] crate::mobility::MobilityError),
    /// A bounded unit collection or encoded catalog exceeded its hard limit.
    #[error("{collection} has size {count}; maximum is {max}")]
    LimitExceeded {
        /// Which bounded collection was refused.
        collection: &'static str,
        /// Attempted count or byte size.
        count: usize,
        /// Hard maximum.
        max: usize,
    },
    /// A shared definition descriptor could not be encoded or registered.
    #[error("unit definition descriptor: {0}")]
    DefinitionDescriptor(String),
    /// A definition reference occupied the wrong typed registry namespace.
    #[error("{field} has kind {found}; expected {expected}")]
    DefinitionKindMismatch {
        /// Stable location of the bad reference.
        field: &'static str,
        /// Required shared definition kind.
        expected: &'static str,
        /// Actual shared definition kind.
        found: DefinitionKind,
    },
    /// A unit body attempted to compose another owner's definition without an
    /// exact bridge/interaction binding. K1's current `UnitSpecBody` format has
    /// no field in which to prove that authorization, so this fails closed.
    #[error(
        "{field} {dependency} is not owned by unit root {root}; cross-owner unit composition requires an explicit bridge-bound format"
    )]
    CrossOwnerDefinitionDependency {
        /// Stable location of the unauthorized dependency.
        field: &'static str,
        /// The exact root unit definition establishing the allowed owner.
        root: Box<DefinitionRef>,
        /// The exact nested definition owned by somebody else.
        dependency: Box<DefinitionRef>,
    },
    /// The same exact unit definition was added more than once.
    #[error("unit definition {0} was added more than once")]
    DuplicateDefinition(DefinitionRef),
    /// One key/version pair was presented with two different digests.
    #[error("unit definition {locator:?} changes exact identity from {registered} to {attempted}")]
    DefinitionDigestConflict {
        /// The conflicting key and version.
        locator: DefinitionLocator,
        /// The first exact identity.
        registered: Box<DefinitionRef>,
        /// The conflicting identity.
        attempted: Box<DefinitionRef>,
    },
    /// A spec's declared identity does not match its canonical body/references.
    #[error("unit definition declares {declared}, but its body computes {computed}")]
    DefinitionIdentityMismatch {
        /// Identity supplied on the spec.
        declared: Box<DefinitionRef>,
        /// Identity derived from canonical descriptor registration.
        computed: Box<DefinitionRef>,
    },
    /// Legacy schema versions are positive; zero cannot name an old contract.
    #[error("legacy unit alias version must be positive")]
    LegacyAliasVersionZero,
    /// An alias attempted to target a definition not retained by this builder.
    #[error("legacy unit alias target {0} is not present in this catalog")]
    LegacyAliasTargetUnavailable(Box<DefinitionRef>),
    /// A deployment configured one ownerless old root identity more than once.
    #[error("legacy unit alias {id}@{version} was configured more than once")]
    DuplicateLegacyAlias {
        /// Ownerless id carried by the old record.
        id: DefinitionId,
        /// Old record's exact version.
        version: u32,
    },
    /// An instance's immutable structure disagrees with its exact unit
    /// definition. The field is an engine-authored stable category, never raw
    /// persisted input.
    #[error("tactical unit structure does not match its definition at {field}")]
    DefinitionShapeMismatch {
        /// Stable structural category that disagreed.
        field: &'static str,
    },
    /// Two sections shared an id.
    #[error("duplicate section {0}")]
    DuplicateSection(SectionId),
    /// Two facets in one section shared an id.
    #[error("duplicate facet {facet} in section {section}")]
    DuplicateFacet {
        /// The section.
        section: SectionId,
        /// The facet.
        facet: FacetId,
    },
    /// Two install slots in one section shared an id.
    #[error("duplicate slot {slot} in section {section}")]
    DuplicateSlot {
        /// The section.
        section: SectionId,
        /// The slot.
        slot: InstallSlotId,
    },
    /// Two components shared an id.
    #[error("duplicate component {0}")]
    DuplicateComponent(ComponentId),
    /// Two cells resolved to the same address.
    #[error("duplicate cell address {0:?}")]
    DuplicateCell(CellAddr),
    /// Two capabilities shared a key.
    #[error("duplicate capability {0:?}")]
    DuplicateCapability(CapabilityKey),
    /// A cell spec/instance violated `min <= current <= max`.
    #[error("cell bounds invalid: min {min}, max {max}, current {current}")]
    CellBounds {
        /// Lower bound.
        min: i64,
        /// Upper bound.
        max: i64,
        /// Current value.
        current: i64,
    },
    /// A reference named a section that does not exist.
    #[error("unknown section {0}")]
    UnknownSection(SectionId),
    /// A reference named a facet not on its section.
    #[error("unknown facet {facet} on section {section}")]
    UnknownFacet {
        /// The section.
        section: SectionId,
        /// The facet.
        facet: FacetId,
    },
    /// A reference named an install slot not on its section.
    #[error("unknown slot {slot} on section {section}")]
    UnknownSlot {
        /// The section.
        section: SectionId,
        /// The slot.
        slot: InstallSlotId,
    },
    /// A reference named a component that does not exist.
    #[error("unknown component {0}")]
    UnknownComponent(ComponentId),
    /// A mutation named a cell address that does not exist.
    #[error("unknown cell address {0:?}")]
    UnknownCell(CellAddr),
    /// A component's install point did not resolve.
    #[error("component {0} has an unresolved install point")]
    UnresolvedInstall(ComponentId),
    /// A capability named a source component that does not exist.
    #[error("capability source component {0} does not exist")]
    UnresolvedCapabilitySource(ComponentId),
    /// A scalar mutation would leave the cell's bounds.
    #[error("cell {addr:?} value {value} out of bounds [{min},{max}]")]
    OutOfBounds {
        /// The cell.
        addr: CellAddr,
        /// The rejected value.
        value: i64,
        /// Lower bound.
        min: i64,
        /// Upper bound.
        max: i64,
    },
    /// A scalar delta overflowed `i64`.
    #[error("scalar mutation overflowed")]
    Overflow,
    /// The unit revision overflowed `u64`.
    #[error("unit revision overflowed")]
    RevisionOverflow,
    /// The plan's `expected_revision` did not match the unit.
    #[error("stale plan: expected revision {expected}, unit is at {actual}")]
    StaleRevision {
        /// The plan's expectation.
        expected: u64,
        /// The unit's actual revision.
        actual: u64,
    },
}
