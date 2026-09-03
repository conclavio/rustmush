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

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::mobility::MobilityProfile;

/// Max canonical length of any tactical id.
pub const TACTICAL_ID_MAX: usize = 64;

/// Canonicalize a tactical id: trim, ASCII-lowercase, require a non-empty run of
/// `[a-z0-9_.]` with no leading/trailing dot and no empty segment. Both bare
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
    if !t.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.') {
        return Err(UnitError::MalformedId(t));
    }
    if t.starts_with('.') || t.ends_with('.') || t.split('.').any(str::is_empty) {
        return Err(UnitError::MalformedId(t));
    }
    Ok(SmolStr::new(t))
}

/// Define a canonical, `SmolStr`-backed tactical id newtype (construction, incl.
/// via `serde`, always canonicalizes).
macro_rules! tactical_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "SmolStr", into = "SmolStr")]
        pub struct $name(SmolStr);
        impl $name {
            /// Validate and canonicalize (see the module's `canon`).
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
                $name::new(s)
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

/// A versioned reference to the ruleset schema a unit was built from — provenance
/// used only by the deferred M33 boot pass for drift detection and migration,
/// never the source of structure at read time (the instance is self-describing).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionRef {
    /// The ruleset schema id.
    pub id: SmolStr,
    /// The schema version.
    pub version: u32,
}

impl DefinitionRef {
    /// A definition reference with a canonicalized id.
    ///
    /// # Errors
    /// A malformed id (see `canon`).
    pub fn new(id: impl AsRef<str>, version: u32) -> Result<DefinitionRef, UnitError> {
        Ok(DefinitionRef { id: canon(id.as_ref())?, version })
    }
}

/// Opaque, schema-tagged ruleset data (kinematics, geometry, capability payload,
/// …) the engine carries and never interprets — the `mush-sim` `RulesValue`
/// analogue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RulesValue {
    /// The ruleset schema id that interprets `data`.
    pub schema: SmolStr,
    /// Schema version.
    pub version: u32,
    /// The opaque payload.
    #[serde(default)]
    pub data: serde_json::Value,
}

impl RulesValue {
    /// A schema-tagged value.
    #[must_use]
    pub fn new(schema: impl Into<SmolStr>, version: u32, data: serde_json::Value) -> RulesValue {
        RulesValue { schema: schema.into(), version, data }
    }
    /// An empty (`{}`) v1 value under `schema`.
    #[must_use]
    pub fn empty(schema: impl Into<SmolStr>) -> RulesValue {
        RulesValue::new(schema, 1, serde_json::Value::Object(serde_json::Map::new()))
    }
}

/// A bounded integer scalar — the unified primitive behind located tracks
/// (armor/structure) and pooled resources (heat/ammo/fuel). `min`/`max` are
/// immutable structure; only `current` is mutated, and only within `[min,max]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
            return Err(UnitError::CellBounds { min: self.min, max: self.max, current: self.current });
        }
        Ok(())
    }
}

/// A fully-qualified address of a [`ScalarCell`] — a CLOSED location kind over
/// OPEN ids. Facet/component cells are section- or component-qualified so the
/// address is globally unique within a unit.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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

impl TacticalUnit {
    /// Instantiate a unit from a ruleset schema and its object id. Cells start at
    /// their spec `init`; sections/components at their initial condition;
    /// `revision` at 0.
    ///
    /// # Errors
    /// A [`UnitError`] for a duplicate id, a bad cell spec (`min>max` or `init`
    /// out of range), an unresolved reference, or an empty unit.
    pub fn from_spec(spec: &UnitSpec, object: ObjId) -> Result<TacticalUnit, UnitError> {
        let mut sections = BTreeMap::new();
        let mut cells: BTreeMap<CellAddr, ScalarCell> = BTreeMap::new();

        for s in &spec.sections {
            let mut facets = BTreeSet::new();
            for f in &s.facets {
                if !facets.insert(f.id.clone()) {
                    return Err(UnitError::DuplicateFacet { section: s.id.clone(), facet: f.id.clone() });
                }
            }
            let mut slots = BTreeSet::new();
            for sl in &s.slots {
                if !slots.insert(sl.clone()) {
                    return Err(UnitError::DuplicateSlot { section: s.id.clone(), slot: sl.clone() });
                }
            }
            if sections.insert(s.id.clone(), SectionState { condition: s.condition.clone(), facets, slots }).is_some() {
                return Err(UnitError::DuplicateSection(s.id.clone()));
            }
            for c in &s.cells {
                insert_cell(&mut cells, CellAddr::Section { section: s.id.clone(), cell: c.id.clone() }, c)?;
            }
            for f in &s.facets {
                for c in &f.cells {
                    let addr = CellAddr::Facet { section: s.id.clone(), facet: f.id.clone(), cell: c.id.clone() };
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
                insert_cell(&mut cells, CellAddr::Component { component: comp.id.clone(), cell: c.id.clone() }, c)?;
            }
        }
        let mut capabilities = BTreeMap::new();
        for cap in &spec.capabilities {
            if capabilities.insert(cap.key.clone(), cap.rules.clone()).is_some() {
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
        if self.sections.is_empty() {
            return Err(UnitError::EmptyUnit);
        }
        for (addr, cell) in &self.cells {
            cell.check()?;
            self.resolve_cell_addr(addr)?;
        }
        for (id, comp) in &self.components {
            self.resolve_install(&comp.install).map_err(|_| UnitError::UnresolvedInstall(id.clone()))?;
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
                    Err(UnitError::UnknownFacet { section: section.clone(), facet: facet.clone() })
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
                    Err(UnitError::UnknownSlot { section: section.clone(), slot: slot.clone() })
                }
            }
        }
    }

    fn require_section(&self, section: &SectionId) -> Result<&SectionState, UnitError> {
        self.sections.get(section).ok_or_else(|| UnitError::UnknownSection(section.clone()))
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
                let cell = self.cells.get_mut(addr).ok_or_else(|| UnitError::UnknownCell(addr.clone()))?;
                let from = cell.current;
                let to = from.checked_add(*delta).ok_or(UnitError::Overflow)?;
                if to < cell.min || to > cell.max {
                    return Err(UnitError::OutOfBounds { addr: addr.clone(), value: to, min: cell.min, max: cell.max });
                }
                cell.current = to;
                Ok(JournalEntry::Cell { addr: addr.clone(), from, to })
            }
            UnitMutationOp::SetCell { addr, value } => {
                let cell = self.cells.get_mut(addr).ok_or_else(|| UnitError::UnknownCell(addr.clone()))?;
                let from = cell.current;
                if *value < cell.min || *value > cell.max {
                    return Err(UnitError::OutOfBounds { addr: addr.clone(), value: *value, min: cell.min, max: cell.max });
                }
                cell.current = *value;
                Ok(JournalEntry::Cell { addr: addr.clone(), from, to: *value })
            }
            UnitMutationOp::SetSectionCondition { section, condition } => {
                let s = self.sections.get_mut(section).ok_or_else(|| UnitError::UnknownSection(section.clone()))?;
                let from = std::mem::replace(&mut s.condition, condition.clone());
                Ok(JournalEntry::SectionCondition { section: section.clone(), from, to: condition.clone() })
            }
            UnitMutationOp::SetComponentCondition { component, condition } => {
                let c = self.components.get_mut(component).ok_or_else(|| UnitError::UnknownComponent(component.clone()))?;
                let from = std::mem::replace(&mut c.condition, condition.clone());
                Ok(JournalEntry::ComponentCondition { component: component.clone(), from, to: condition.clone() })
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
                let replaced = self.capabilities.insert(key.clone(), rules.clone()).is_some();
                Ok(JournalEntry::CapabilityGranted { key: key.clone(), replaced })
            }
            UnitMutationOp::RevokeCapability { key } => {
                let was_present = self.capabilities.remove(key).is_some();
                Ok(JournalEntry::CapabilityRevoked { key: key.clone(), was_present })
            }
        }
    }
}

/// Insert a cell built from a spec, checking bounds and address uniqueness.
fn insert_cell(cells: &mut BTreeMap<CellAddr, ScalarCell>, addr: CellAddr, spec: &CellSpec) -> Result<(), UnitError> {
    let cell = ScalarCell { min: spec.min, max: spec.max, current: spec.init };
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnitMutationPlan {
    /// The ops, applied in this order.
    pub ops: Vec<UnitMutationOp>,
    /// If `Some(n)`, the plan is refused unless `unit.revision == n` (the F3
    /// lease-revision guard; optional in M32 where plan+apply run atomically).
    #[serde(default)]
    pub expected_revision: Option<u64>,
    /// An optional semantic tag recorded in the journal.
    #[serde(default)]
    pub cause: Option<SmolStr>,
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
/// out-of-bounds/overflowing scalar, or a revision overflow — with no mutation.
pub fn apply(unit: &TacticalUnit, plan: &UnitMutationPlan, now: i64) -> Result<(TacticalUnit, Journal), UnitError> {
    if let Some(expected) = plan.expected_revision
        && unit.revision != expected
    {
        return Err(UnitError::StaleRevision { expected, actual: unit.revision });
    }
    let mut post = unit.clone();
    let mut entries = Vec::with_capacity(plan.ops.len());
    for op in &plan.ops {
        entries.push(post.apply_op(op)?);
    }
    let to = unit.revision.checked_add(1).ok_or(UnitError::RevisionOverflow)?;
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

/// A ruleset-registered catalog of `UnitSpec`s by definition id, composed at
/// module construction (builder → freeze, like M31's `MobilityVocabulary`). The
/// engine ships the MECHANISM empty; a ruleset (or dev/test wiring) seeds the
/// specs — no concrete spec is baked into the engine crate.
#[derive(Clone, Debug, Default)]
pub struct UnitCatalog {
    specs: BTreeMap<SmolStr, UnitSpec>,
}

impl UnitCatalog {
    /// The spec registered under `def_id`, if any.
    #[must_use]
    pub fn get(&self, def_id: &str) -> Option<&UnitSpec> {
        self.specs.get(def_id)
    }
    /// The registered definition ids (ordered).
    pub fn ids(&self) -> impl Iterator<Item = &SmolStr> {
        self.specs.keys()
    }
    /// Whether the catalog holds no specs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }
}

/// Builder for a [`UnitCatalog`]: each added spec's mobility modes are validated
/// against a [`MobilityRegistry`](crate::mobility::MobilityRegistry) before the
/// catalog is frozen (closing the gap that `from_spec` does not registry-check
/// an embedded profile).
#[derive(Clone, Debug, Default)]
pub struct UnitCatalogBuilder {
    specs: BTreeMap<SmolStr, UnitSpec>,
}

impl UnitCatalogBuilder {
    /// A fresh, empty builder.
    #[must_use]
    pub fn new() -> UnitCatalogBuilder {
        UnitCatalogBuilder::default()
    }
    /// Register `spec`, validating each of its mobility modes against
    /// `mobility_reg` (a mode referencing an undeclared atom is refused here, at
    /// wiring time, rather than silently at create).
    ///
    /// # Errors
    /// [`UnitError::Mobility`] if a mobility mode references an undeclared atom.
    pub fn add(
        &mut self,
        spec: UnitSpec,
        mobility_reg: &crate::mobility::MobilityRegistry,
    ) -> Result<&mut UnitCatalogBuilder, UnitError> {
        if let Some(m) = &spec.mobility {
            for mode in m.modes() {
                mobility_reg.validate_mode(mode)?;
            }
        }
        self.specs.insert(spec.definition.id.clone(), spec);
        Ok(self)
    }
    /// Freeze into an immutable, runtime-shareable catalog.
    #[must_use]
    pub fn freeze(self) -> UnitCatalog {
        UnitCatalog { specs: self.specs }
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
    #[error("tactical id {0:?} must be lowercase [a-z0-9_.] with no leading/trailing/empty dot segment")]
    MalformedId(String),
    /// A unit had no sections.
    #[error("a tactical unit must have at least one section")]
    EmptyUnit,
    /// A catalog spec's mobility mode referenced an undeclared atom.
    #[error("mobility: {0}")]
    Mobility(#[from] crate::mobility::MobilityError),
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
