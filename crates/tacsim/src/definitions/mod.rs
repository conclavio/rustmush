//! **The game's typed definitions** (16b cut C.2, C1; doc 16 §4 "Content
//! packs", §7 "The game crate"; D-16-13).
//!
//! Six new [`crate::pack::PackEntry`] kinds — `unit`, `weapon`, `ammunition`,
//! `sensor`, `resolution`, `arena` — decoded strictly (`deny_unknown_fields`,
//! integers only: centimetres, millidegrees, milliseconds, cm/s, per-mille),
//! bounded on load, and registered at composition through the C1 registry as
//! [`mush_core::DefinitionDescriptor`]s with exact [`mush_core::DefinitionRef`]s.
//!
//! **17b cut M3.1** adds a seventh content kind, `core`
//! ([`crate::definitions::core::CoreDef`]; doc 17 §9, D-UM-33): a Compact
//! class and a governed bit, minted before the units whose `core` components
//! name it, and fitted by `bind` as a component, a `core_class` cell and a
//! `tacsim.core` capability under the sixth minted schema,
//! `tacsim.core.profile`.
//!
//! # Three things happen in one place, on purpose
//!
//! [`crate::definitions::Catalog::build`] decodes the entries, checks every reference among them,
//! and mints every definition reference. It is one function because the three
//! have to agree: the `mush_tactics::UnitCatalog` the composition root hands
//! `TacticsModule` and the descriptors the module hands `Registrar::definition`
//! must carry **the same** `DefinitionRef` values, and the only way to be sure
//! of that without a second lane is for one function to produce both. The refs
//! come out of a scratch [`mush_core::DefinitionRegistryBuilder`] stamped with the module's
//! own owner id, and a definition's digest is a pure function of
//! `(owner, kind, id, version, payload, references)`, so the scratch registry's
//! answer *is* the real registry's answer.
//!
//! # An entry id is a definition id
//!
//! The pack-local entry id and the registry's `DefinitionId` are the same
//! string, so `@unit/create #3=tacsim.unit.line_tank` names the entry a reviewer
//! reads in the file. A definition's **version** is its pack's version, which is
//! what makes "a changed definition is a new version" true by the pack ledger's
//! own rule rather than by a second version field nobody would bump: a pack
//! re-signed in place under one version already refuses the boot
//! (`crate::boot_reconcile`).
//!
//! Across the loaded packs an id of these six kinds is **unique**. Two packs
//! that declare one are refused naming both, rather than one silently shadowing
//! the other or both registering and leaving `@unit/create`'s "current" lookup
//! ambiguous.

pub mod ammunition;
pub mod arena;
pub mod content;
pub mod core;
pub mod goods;
pub mod logistics;
pub mod resolution;
pub mod resources;
pub mod sensor;
pub mod tools;
pub mod unit;
pub mod weapon;

use std::collections::BTreeMap;

use mush_core::CanonicalId;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionDigest, DefinitionRef, DefinitionRegistryBuilder,
    DefinitionValidationContext, OwnerId,
};
use mush_tactics::{MobilityVocabulary, UnitCatalog, UnitCatalogBuilder};

use crate::config::LoadedPack;
use crate::pack::PackEntry;

pub use ammunition::AmmunitionDef;
pub use arena::{ArenaBuildError, ArenaDef, arena_map_within_bound, from_pack};
pub use core::CoreDef;
pub use goods::{Consumable, GoodClass, GoodDef, GoodsTable, MeasureDef};
pub use logistics::{
    BayDef, BaySlotDef, BerthsSlotDef, DriveDef, DriveLaw, EgressDef, InteriorDef, InteriorExitDef,
    InteriorRoomDef, InteriorRoomKind, LogisticsDef, StowageDef,
};
pub use resolution::ResolutionDef;
pub use resources::ResourceTable;
pub use sensor::SensorDef;
pub use tools::{ToolDef, ToolRole};
pub use unit::{UnitBinding, UnitClass, UnitSpec, to_tactical_unit, tracked_rules};
pub use weapon::WeaponDef;

/// The most bytes a human-facing label inside a definition may occupy — the
/// pack format's own bound, restated so a definition entry is held to it too.
pub const MAX_LABEL_BYTES: usize = crate::pack::MAX_LABEL_BYTES;

/// The most bytes one `note` may occupy. A note explains a number to the next
/// author; it is not documentation, and a pack is not a wiki.
pub const MAX_NOTE_BYTES: usize = 1_024;

/// The most other definitions one may name. `MechanicsLimits` has its own,
/// larger bound; this is the authoring one, so a refusal names the entry rather
/// than the registry.
pub const MAX_ENTRY_REFERENCES: usize = 32;

/// The definition kinds this crate registers, in dependency order.
///
/// `schema` and `component` are M32's vocabulary (`mush_tactics`'
/// `validate_spec_definition_kinds` fixes them), and the other six are doc 16
/// §4's content kinds. Every one needs exactly one owner-side validator, which
/// is why the list is a constant rather than six literals.
///
/// **17b cut M3.1** adds the ninth, `core` (doc 17 §9, D-UM-33), after
/// `sensor` and before `resolution`: a core is minted before the components
/// that name it, as a sensor is.
pub const KINDS: [&str; 9] = [
    "schema",
    "ammunition",
    "weapon",
    "sensor",
    "core",
    "resolution",
    "component",
    "unit",
    "arena",
];

/// The kinds a TacSim pack may carry (the other two are minted, never
/// authored): doc 16 §4's six and, since **17b cut M3.1**, the seventh,
/// `core` (doc 17 §9, D-UM-33).
pub const CONTENT_KINDS: [&str; 7] = [
    "unit",
    "weapon",
    "ammunition",
    "sensor",
    "core",
    "resolution",
    "arena",
];

/// Why a definition refused.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DefinitionError {
    /// A value or a shape the entry may not carry.
    #[error("{entry}: {detail}")]
    Value {
        /// The entry that refused.
        entry: String,
        /// What is wrong.
        detail: String,
    },
    /// An entry names something no loaded pack declares.
    #[error("{entry}: names {kind} `{id}`, which no loaded pack declares")]
    Reference {
        /// The entry that names it.
        entry: String,
        /// What kind of thing it named.
        kind: String,
        /// The id it named.
        id: String,
    },
    /// Two packs declare one entry id of a definition kind.
    #[error("{entry}: declared by {first} and by {second}; a definition id names one definition")]
    Duplicate {
        /// The entry id.
        entry: String,
        /// The first pack.
        first: String,
        /// The second.
        second: String,
    },
}

impl DefinitionError {
    /// A value refusal.
    pub(crate) fn value(entry: &str, detail: impl Into<String>) -> DefinitionError {
        DefinitionError::Value {
            entry: entry.to_string(),
            detail: detail.into(),
        }
    }

    /// A dangling reference.
    pub(crate) fn reference(entry: &str, kind: &str, id: &str) -> DefinitionError {
        DefinitionError::Reference {
            entry: entry.to_string(),
            kind: kind.to_string(),
            id: id.to_string(),
        }
    }
}

/// A label's bound.
pub(crate) fn check_label(entry: &str, label: &str) -> Result<(), DefinitionError> {
    if label.is_empty() || label.len() > MAX_LABEL_BYTES {
        return Err(DefinitionError::value(
            entry,
            format!(
                "a label is 1 to {MAX_LABEL_BYTES} bytes; {} is not",
                label.len()
            ),
        ));
    }
    Ok(())
}

/// A note's bound.
pub(crate) fn check_note(entry: &str, note: &str) -> Result<(), DefinitionError> {
    if note.len() > MAX_NOTE_BYTES {
        return Err(DefinitionError::value(
            entry,
            format!(
                "a note is at most {MAX_NOTE_BYTES} bytes; {} is not",
                note.len()
            ),
        ));
    }
    Ok(())
}

/// One decoded entry with the pack it came from and the version that pack
/// stamps on it.
#[derive(Clone, Debug)]
struct Sourced<T> {
    pack: CanonicalId,
    version: u32,
    def: T,
}

/// **The game's definitions, decoded, checked and bound** (16b cut C.2).
///
/// Everything the composition root, the module and the arena pack's applier
/// need, produced once so none of them can hold a different answer.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    /// The descriptors, in dependency order: register them in this order.
    pub descriptors: Vec<DefinitionDescriptor>,
    /// The `mush-tactics` catalog, which is what `@unit/create` resolves a bare
    /// schema id through.
    pub units: UnitCatalog,
    /// Each unit entry's binding, by entry id — the applier's way to a
    /// `TacticalUnit` without re-deriving anything.
    pub bindings: BTreeMap<CanonicalId, UnitBinding>,
    /// The authored unit specs, by entry id.
    pub specs: BTreeMap<CanonicalId, UnitSpec>,
    /// The arenas, by entry id.
    pub arenas: BTreeMap<CanonicalId, ArenaDef>,
    /// The weapons, by entry id.
    pub weapons: BTreeMap<CanonicalId, WeaponDef>,
    /// The ammunition, by entry id.
    pub ammunition: BTreeMap<CanonicalId, AmmunitionDef>,
    /// The sensors, by entry id.
    pub sensors: BTreeMap<CanonicalId, SensorDef>,
    /// **The cores, by entry id** (17b cut M3.1; doc 17 §9, D-UM-33).
    pub cores: BTreeMap<CanonicalId, CoreDef>,
    /// The resolution tables, by entry id.
    pub resolutions: BTreeMap<CanonicalId, ResolutionDef>,
    /// Every minted reference, by entry id — the exact identity a persisted
    /// unit and a result record point at.
    pub references: BTreeMap<CanonicalId, DefinitionRef>,
    /// **The drive table** (doc 18 cut IC2; Q-18-5 binding, D-18-45): drive
    /// class → the one law it flies under, indexed over the fielded unit
    /// definitions that declare `logistics`. Read through
    /// [`Catalog::drive_for_class`].
    pub drives: BTreeMap<CanonicalId, DriveLaw>,
}

impl Catalog {
    /// **Decode, check and bind every definition in `packs`.**
    ///
    /// # Errors
    ///
    /// [`DefinitionError`] naming the entry: a bound, a dangling reference among
    /// the loaded packs, or one id declared by two packs.
    pub fn build(packs: &[LoadedPack], owner: &str) -> Result<Catalog, DefinitionError> {
        let owner = OwnerId::new(owner)
            .map_err(|error| DefinitionError::value("tacsim", error.to_string()))?;
        let mut ammunition: BTreeMap<CanonicalId, Sourced<AmmunitionDef>> = BTreeMap::new();
        let mut weapons: BTreeMap<CanonicalId, Sourced<WeaponDef>> = BTreeMap::new();
        let mut sensors: BTreeMap<CanonicalId, Sourced<SensorDef>> = BTreeMap::new();
        let mut cores: BTreeMap<CanonicalId, Sourced<CoreDef>> = BTreeMap::new();
        let mut resolutions: BTreeMap<CanonicalId, Sourced<ResolutionDef>> = BTreeMap::new();
        let mut units: BTreeMap<CanonicalId, Sourced<UnitSpec>> = BTreeMap::new();
        let mut arenas: BTreeMap<CanonicalId, Sourced<ArenaDef>> = BTreeMap::new();

        // (1) Collect, refusing a duplicate id across packs and re-running each
        // entry's own bounds. `PackEntry::check` already ran at load; running it
        // again here costs nothing and makes this function total over any pack
        // set a caller assembles.
        for loaded in packs {
            let pack = loaded.pack.manifest().id().clone();
            let version = loaded.pack.manifest().version();
            for (id, entry) in loaded.pack.entries() {
                let name = id.as_str();
                match entry {
                    PackEntry::Ammunition(def) => {
                        def.check(name)?;
                        insert(&mut ammunition, id, &pack, version, def.clone())?;
                    }
                    PackEntry::Weapon(def) => {
                        def.check(name)?;
                        insert(&mut weapons, id, &pack, version, def.clone())?;
                    }
                    PackEntry::Sensor(def) => {
                        def.check(name)?;
                        insert(&mut sensors, id, &pack, version, def.clone())?;
                    }
                    // 17b cut M3.1: a core, checked and refused a duplicate as
                    // every other kind is.
                    PackEntry::Core(def) => {
                        def.check(name)?;
                        insert(&mut cores, id, &pack, version, def.clone())?;
                    }
                    PackEntry::Resolution(def) => {
                        def.check(name)?;
                        insert(&mut resolutions, id, &pack, version, def.clone())?;
                    }
                    PackEntry::Unit(def) => {
                        def.check(name)?;
                        insert(&mut units, id, &pack, version, def.clone())?;
                    }
                    PackEntry::Arena(def) => {
                        def.check(name)?;
                        insert(&mut arenas, id, &pack, version, def.clone())?;
                    }
                    _ => {}
                }
            }
        }

        // (2) Every reference among them, at load (doc 16 §4: *every definition
        // … validates its references*). The arena's atlas records are the one
        // reference that cannot be checked here and is checked at apply instead.
        for (id, sourced) in &weapons {
            if !ammunition.contains_key(&sourced.def.ammunition) {
                return Err(DefinitionError::reference(
                    id.as_str(),
                    "ammunition",
                    sourced.def.ammunition.as_str(),
                ));
            }
        }
        for (id, sourced) in &units {
            for weapon in sourced.def.weapons() {
                let Some(def) = weapons.get(weapon) else {
                    return Err(DefinitionError::reference(
                        id.as_str(),
                        "weapon",
                        weapon.as_str(),
                    ));
                };
                // **16b cut X.1** (doc 16 §9 item 1): a unit that mounts a weapon
                // which makes heat has to say where the heat goes. The two are
                // authored in different files and neither can check the other
                // alone, so the cross-reference is checked here with the rest of
                // them — a laser bolted to a tank with no heat cell would
                // otherwise run a whole match shedding its heat into nothing.
                if def.def.heat_per_shot > 0 && sourced.def.heat_max == 0 {
                    return Err(DefinitionError::value(
                        id.as_str(),
                        format!(
                            "mounts `{weapon}`, which makes heat, so it declares heat_max \
                             and heat_dissipation_per_s"
                        ),
                    ));
                }
            }
            for sensor in sourced.def.sensors() {
                if !sensors.contains_key(sensor) {
                    return Err(DefinitionError::reference(
                        id.as_str(),
                        "sensor",
                        sensor.as_str(),
                    ));
                }
            }
            // 17b cut M3.1: a unit's core names a core some loaded pack
            // declares.
            for core in sourced.def.cores() {
                if !cores.contains_key(core) {
                    return Err(DefinitionError::reference(
                        id.as_str(),
                        "core",
                        core.as_str(),
                    ));
                }
            }
        }
        for (id, sourced) in &arenas {
            for tank in &sourced.def.tanks {
                if !units.contains_key(&tank.unit) {
                    return Err(DefinitionError::reference(
                        id.as_str(),
                        "unit",
                        tank.unit.as_str(),
                    ));
                }
            }
        }

        // (2b) Doc 18 cut IC2 (Q-18-5, D-18-45): one drive class, one law. The
        // drive table is the index over the fielded definitions, walked in id
        // order so the refusal names the same pair on every boot; a second unit
        // flying a class under the same law is accepted.
        let drives = drive_table(&units)?;

        // (3) Mint, in dependency order.
        let mut scratch = DefinitionRegistryBuilder::new();
        let mut minter = Minter {
            scratch: &mut scratch,
            owner: &owner,
        };
        let mut out = Catalog {
            drives,
            ..Catalog::default()
        };
        let mut schemas: BTreeMap<&'static str, DefinitionRef> = BTreeMap::new();
        for (id, payload) in schema_payloads() {
            let descriptor = unit::descriptor("schema", id, 1, payload, Vec::new())
                .map_err(|error| DefinitionError::value(id, error))?;
            let reference = unit::register(&mut minter, descriptor.clone(), id)?;
            out.descriptors.push(descriptor);
            schemas.insert(id, reference);
        }
        for (id, sourced) in &ammunition {
            let payload = serde_json::to_value(&sourced.def)
                .map_err(|error| DefinitionError::value(id.as_str(), error.to_string()))?;
            out.mint(
                &mut minter,
                "ammunition",
                id,
                sourced.version,
                payload,
                Vec::new(),
            )?;
            out.ammunition.insert(id.clone(), sourced.def.clone());
        }
        for (id, sourced) in &weapons {
            let reference = out
                .references
                .get(&sourced.def.ammunition)
                .cloned()
                .ok_or_else(|| {
                    DefinitionError::reference(
                        id.as_str(),
                        "ammunition",
                        sourced.def.ammunition.as_str(),
                    )
                })?;
            let payload = serde_json::to_value(&sourced.def)
                .map_err(|error| DefinitionError::value(id.as_str(), error.to_string()))?;
            out.mint(
                &mut minter,
                "weapon",
                id,
                sourced.version,
                payload,
                vec![reference],
            )?;
            out.weapons.insert(id.clone(), sourced.def.clone());
        }
        for (id, sourced) in &sensors {
            let payload = serde_json::to_value(&sourced.def)
                .map_err(|error| DefinitionError::value(id.as_str(), error.to_string()))?;
            out.mint(
                &mut minter,
                "sensor",
                id,
                sourced.version,
                payload,
                Vec::new(),
            )?;
            out.sensors.insert(id.clone(), sourced.def.clone());
        }
        // 17b cut M3.1: the cores, minted as `core` definitions before the
        // units whose core components name them.
        for (id, sourced) in &cores {
            let payload = serde_json::to_value(&sourced.def)
                .map_err(|error| DefinitionError::value(id.as_str(), error.to_string()))?;
            out.mint(
                &mut minter,
                "core",
                id,
                sourced.version,
                payload,
                Vec::new(),
            )?;
            out.cores.insert(id.clone(), sourced.def.clone());
        }
        for (id, sourced) in &resolutions {
            let payload = serde_json::to_value(&sourced.def)
                .map_err(|error| DefinitionError::value(id.as_str(), error.to_string()))?;
            out.mint(
                &mut minter,
                "resolution",
                id,
                sourced.version,
                payload,
                Vec::new(),
            )?;
            out.resolutions.insert(id.clone(), sourced.def.clone());
        }
        let weapon_defs: BTreeMap<CanonicalId, WeaponDef> = out.weapons.clone();
        let ammunition_defs: BTreeMap<CanonicalId, AmmunitionDef> = out.ammunition.clone();
        let sensor_defs: BTreeMap<CanonicalId, SensorDef> = out.sensors.clone();
        let core_defs: BTreeMap<CanonicalId, CoreDef> = out.cores.clone();
        // Every reference minted so far: the ammunition, the weapons, the
        // sensors, the cores (17b cut M3.1) and the resolution tables. A unit's binding looks its weapon
        // and sensor rows up in it by id, and the two lookups share one map
        // because an id is unique across the kinds by construction.
        let minted = out.references.clone();
        let vocabulary = MobilityVocabulary::base().freeze();
        let mut catalog = UnitCatalogBuilder::new();
        for (id, sourced) in &units {
            let binding = unit::bind(
                &mut minter,
                id,
                sourced.version,
                &sourced.def,
                unit::UnitContext {
                    ammunition: &ammunition_defs,
                    weapons: &weapon_defs,
                    sensors: &sensor_defs,
                    weapon_refs: &minted,
                    sensor_refs: &minted,
                    cores: &core_defs,
                    core_refs: &minted,
                    schemas: &schemas,
                },
            )?;
            out.references
                .insert(id.clone(), binding.spec.definition.clone());
            out.descriptors.extend(binding.descriptors.iter().cloned());
            catalog
                .add(binding.spec.clone(), &vocabulary)
                .map_err(|error| DefinitionError::value(id.as_str(), error.to_string()))?;
            out.bindings.insert(id.clone(), binding);
            out.specs.insert(id.clone(), sourced.def.clone());
        }
        out.units = catalog.freeze();
        for (id, sourced) in &arenas {
            let mut references: Vec<DefinitionRef> = Vec::new();
            for tank in &sourced.def.tanks {
                let reference = out.references.get(&tank.unit).cloned().ok_or_else(|| {
                    DefinitionError::reference(id.as_str(), "unit", tank.unit.as_str())
                })?;
                if !references.contains(&reference) {
                    references.push(reference);
                }
            }
            let payload = serde_json::to_value(&sourced.def)
                .map_err(|error| DefinitionError::value(id.as_str(), error.to_string()))?;
            out.mint(
                &mut minter,
                "arena",
                id,
                sourced.version,
                payload,
                references,
            )?;
            out.arenas.insert(id.clone(), sourced.def.clone());
        }
        Ok(out)
    }

    /// **The law a drive class flies under** (doc 18 cut IC2; Q-18-5, D-18-45):
    /// the drive table is the index over the fielded definitions, so a class no
    /// loaded hull declares answers `None`.
    #[must_use]
    pub fn drive_for_class(&self, class: &CanonicalId) -> Option<&DriveLaw> {
        self.drives.get(class)
    }

    /// **The STOCK game's definitions: [`content::SOURCES`], plus whatever
    /// `[tacsim].pack_paths` added** (16b cut C.2).
    ///
    /// Two weapons, one sensor, one unit schema and one arena — doc 16 §9's own
    /// measure of what an extension adds, and what every cut-X acceptance
    /// compares its own item against — and, since **17b cut M3.1**, the three
    /// cores of `tacsim.pack.cores`, which every chassis from then on fits one
    /// of and so must find in any game it loads into (no cut-X count of
    /// weapons, sensors or unit schemas moves). **16b cut Q.8** moved the three production
    /// callers to [`Catalog::build_fielded`] and left this one exactly as it
    /// was, so those acceptances keep meaning what they said. Nothing in
    /// production calls it.
    ///
    /// An operator pack that carries a `(id, version)` the shipped content
    /// already declares is **skipped** when it is byte for byte that same file
    /// and **refused** when it is not; see `Catalog::build_from` for why.
    ///
    /// # Errors
    ///
    /// As [`Catalog::build`], plus a shipped pack that does not verify — a
    /// build error, since `crates/tacsim/tests/content.rs` pins it.
    pub fn build_game(operator: &[LoadedPack], owner: &str) -> Result<Catalog, DefinitionError> {
        Catalog::build_from(content::sources(), operator, owner)
    }

    /// **The game as it is fielded: the stock content, doc 16 §9's five
    /// extension items, and whatever `[tacsim].pack_paths` added** (16b cut
    /// Q.8).
    ///
    /// The three production sites call this and not [`Catalog::build_game`]:
    /// `crate::builtin_definitions` (the `@unit/create` catalog the composition
    /// root freezes), `crate::TacsimRuleset::register` (the C1 descriptors) and
    /// `crate::store::TacsimStore::install_packs` (the catalog `SIM UNIT`, the
    /// arena applier and the stock-tank reset all read). So the two answers
    /// cannot disagree about a laser tank any more than they could about the
    /// line tank, which is the whole reason `build` is one function.
    ///
    /// [`Catalog::build_game`] stays exactly what it was — **the stock game** —
    /// because that is what doc 16 §9's rule measures an extension against and
    /// what seven cut-X acceptances assert about. Nothing calls it in
    /// production.
    ///
    /// # Errors
    ///
    /// As [`Catalog::build_game`].
    pub fn build_fielded(operator: &[LoadedPack], owner: &str) -> Result<Catalog, DefinitionError> {
        Catalog::build_from(content::fielded_sources(), operator, owner)
    }

    /// The body both of the above share: load `base` as the compiled-in content,
    /// then append the operator's packs, **skipping** one that is the very same
    /// file and **refusing** one that is not.
    ///
    /// C.2's rule was "skip a `(id, version)` the base already carries", and it
    /// justified the skip by saying [`crate::verify_sources`] had already
    /// refused a different file under a shipped name. It has not:
    /// `verify_sources` compares the **configured** sources against each other
    /// and never reads the compiled-in bytes at all, so an operator pack under a
    /// shipped name and version was accepted, ledgered — and then silently
    /// dropped here, with the shipped pack running in its place. Q.8 made that
    /// matter, because the five extension files moved into the base and an
    /// operator may well already list one.
    ///
    /// So the dedupe compares the **declared digest**, which [`crate::pack::load_pack`]
    /// has already checked against the content on both sides:
    ///
    /// - same digest — it *is* the shipped file, arriving twice because it is on
    ///   `[tacsim].pack_paths` as well as compiled in. Skipped, exactly as
    ///   before, so that operator keeps booting and their pack ledger row is
    ///   unchanged;
    /// - different digest — an edited pack wearing a shipped pack's name and
    ///   version. **Refused**, naming the file, because the alternative is an
    ///   operator's deliberate edit having no effect and no message. A pack that
    ///   means to replace shipped content says so with a new version.
    fn build_from(
        base: Vec<crate::config::PackSource>,
        operator: &[LoadedPack],
        owner: &str,
    ) -> Result<Catalog, DefinitionError> {
        let mut packs: Vec<LoadedPack> = Vec::new();
        for source in base {
            let pack = crate::pack::load_pack(&source.bytes)
                .map_err(|error| DefinitionError::value(&source.name, error.to_string()))?;
            packs.push(LoadedPack {
                source: source.name,
                pack,
            });
        }
        let shipped: Vec<((CanonicalId, u32), DefinitionDigest)> = packs
            .iter()
            .map(|loaded| {
                let manifest = loaded.pack.manifest();
                (
                    (manifest.id().clone(), manifest.version()),
                    manifest.digest(),
                )
            })
            .collect();
        for loaded in operator {
            let manifest = loaded.pack.manifest();
            let key = (manifest.id().clone(), manifest.version());
            if let Some((_, digest)) = shipped.iter().find(|(shipped, _)| *shipped == key) {
                // The very same file, arriving through both routes. Skipped, as
                // it always was.
                if *digest == manifest.digest() {
                    continue;
                }
                return Err(DefinitionError::value(
                    &loaded.source,
                    format!(
                        "declares {}@{}, which this server ships under a different digest ({} \
                         against the shipped {}); publish it under a version of its own rather \
                         than a shipped pack's name and version",
                        key.0,
                        key.1,
                        manifest.digest().to_hex(),
                        digest.to_hex(),
                    ),
                ));
            }
            packs.push(loaded.clone());
        }
        Catalog::build(&packs, owner)
    }

    fn mint(
        &mut self,
        minter: &mut Minter<'_>,
        kind: &str,
        id: &CanonicalId,
        version: u32,
        payload: serde_json::Value,
        references: Vec<DefinitionRef>,
    ) -> Result<(), DefinitionError> {
        let name = id.as_str();
        // Every content definition is its key's current version: one id names
        // one definition across the loaded packs (see this module's header), so
        // there is never a second candidate for the registry to choose between.
        let descriptor = unit::descriptor(kind, name, version, payload, references)
            .map_err(|error| DefinitionError::value(name, error))?
            .as_current();
        let reference = unit::register(minter, descriptor.clone(), name)?;
        self.descriptors.push(descriptor);
        self.references.insert(id.clone(), reference);
        Ok(())
    }
}

/// The scratch registry a definition's exact reference is computed in, and the
/// owner it is stamped with — the pair every mint needs and neither half of
/// which means anything alone.
/// Public because [`unit::bind`] takes one and is part of this module's
/// surface; its two fields are the crate's, and nothing outside constructs one.
pub struct Minter<'a> {
    pub(crate) scratch: &'a mut DefinitionRegistryBuilder,
    pub(crate) owner: &'a OwnerId,
}

/// The `schema` definitions the unit binding needs, with payloads that say what
/// each opaque blob carries. They are minted rather than authored: a schema is
/// the *shape* of a rules value, which is code's, not content's.
///
/// **16b cut X.2** adds the fourth, for the legged mode. It is minted whether or
/// not a loaded unit walks — the set is the code's and not the content's — and
/// it moves no other definition: a `DefinitionRef` digest is a function of its
/// own `(owner, kind, id, version, payload, references)` and of nothing the
/// registry happens to hold beside it.
///
/// **16b cut AE.1** adds the fifth, for the wheeled mode, by the same rule and
/// with the same consequence: no other definition moves.
///
/// **17b cut M3.1** adds the sixth, [`unit::CORE_RULES_SCHEMA`], last, by the
/// same rule: `tacsim.power.states`' minted reference is the bytes it was.
fn schema_payloads() -> Vec<(&'static str, serde_json::Value)> {
    let mode_rules = serde_json::json!({
        "carries": [
            "max_speed_cms", "accel_cms2", "brake_cms2",
            "turn_rate_mdeg_s", "reverse_factor_pct"
        ],
        "read_by": "tacsim"
    });
    vec![
        (
            unit::RELAY_RULES_SCHEMA,
            serde_json::json!({"carries":["radius_km","room"],"read_by":"tacsim"}),
        ),
        (
            unit::UNIT_RULES_SCHEMA,
            serde_json::json!({
                "carries": [
                    "facets", "silhouettes", "sensor_height_cm",
                    "consequences", "components", "mobility"
                ],
                "read_by": "tacsim"
            }),
        ),
        (unit::TRACKED_RULES_SCHEMA, mode_rules.clone()),
        (unit::LEGGED_RULES_SCHEMA, mode_rules.clone()),
        (
            unit::POWER_RULES_SCHEMA,
            serde_json::json!({ "carries": ["states", "state"], "read_by": "tacsim" }),
        ),
        // 16b cut AE.1: the fifth, for the wheeled mode — added, as X.2 added
        // the legged one, and nothing above it touched.
        (unit::WHEELED_RULES_SCHEMA, mode_rules),
        // 17b cut M3.1: the sixth, for the core's capability — the fitting's
        // record (`core::profile_payload`), appended last by AE.1's rule.
        (
            unit::CORE_RULES_SCHEMA,
            serde_json::json!({
                "carries": [
                    "class", "governed", "think_ms", "ops", "rows", "odds_rows",
                    "slots", "memory_keys", "line_to", "profiles"
                ],
                "read_by": "tacsim"
            }),
        ),
    ]
}

/// **The drive table** (doc 18 cut IC2; D-18-45): every unit with `logistics`
/// and a drive, in id order, its drive class → its law, refusing a class already
/// flown under a different law and naming both units.
fn drive_table(
    units: &BTreeMap<CanonicalId, Sourced<UnitSpec>>,
) -> Result<BTreeMap<CanonicalId, DriveLaw>, DefinitionError> {
    let mut drives: BTreeMap<CanonicalId, (DriveLaw, &CanonicalId)> = BTreeMap::new();
    for (id, sourced) in units {
        let Some(logistics) = &sourced.def.logistics else {
            continue;
        };
        // Doc 20 cut SM.3a: "one class, one law" ignores a drive-less
        // (stowage-only) unit — the terrain vehicle flies no law at all.
        let Some(drive) = &logistics.drive else {
            continue;
        };
        match drives.get(&drive.class) {
            Some((law, _)) if *law == drive.law => {}
            Some((law, first)) => {
                return Err(DefinitionError::value(
                    id.as_str(),
                    format!(
                        "drive class `{}` is already flown under {law} by `{first}`; one class, one law",
                        drive.class
                    ),
                ));
            }
            None => {
                drives.insert(drive.class.clone(), (drive.law, id));
            }
        }
    }
    Ok(drives
        .into_iter()
        .map(|(class, (law, _))| (class, law))
        .collect())
}

/// Insert one entry, refusing an id two packs both declare.
fn insert<T>(
    into: &mut BTreeMap<CanonicalId, Sourced<T>>,
    id: &CanonicalId,
    pack: &CanonicalId,
    version: u32,
    def: T,
) -> Result<(), DefinitionError> {
    if let Some(existing) = into.get(id) {
        return Err(DefinitionError::Duplicate {
            entry: id.to_string(),
            first: format!("{}@{}", existing.pack, existing.version),
            second: format!("{pack}@{version}"),
        });
    }
    into.insert(
        id.clone(),
        Sourced {
            pack: pack.clone(),
            version,
            def,
        },
    );
    Ok(())
}

/// **The one validator per kind** (doc 16 §4: *every definition is registered
/// through the C1 registry with an exact `DefinitionRef`*).
///
/// `Registrar::definition` admits nothing whose kind has no owner-side
/// validator, and this is that validator: the frozen registry's entry must be
/// byte-identical to the descriptor this crate built, references and current
/// flag included. A definition that reached the registry by any other route,
/// or a payload that drifted between building the catalog and registering it,
/// fails composition rather than being discovered at the first `@unit/create`.
pub fn exact_validator(
    descriptors: Vec<DefinitionDescriptor>,
) -> impl Fn(&DefinitionValidationContext<'_>) -> Result<(), String> + Send + Sync + 'static {
    let expected: BTreeMap<(String, u32), DefinitionDescriptor> = descriptors
        .into_iter()
        .map(|descriptor| {
            (
                (descriptor.id().as_str().to_string(), descriptor.version()),
                descriptor,
            )
        })
        .collect();
    move |context: &DefinitionValidationContext<'_>| {
        let entry = context.definition();
        let key = (
            entry.reference().key().id.as_str().to_string(),
            entry.reference().version(),
        );
        let Some(descriptor) = expected.get(&key) else {
            return Err("unknown definition id or version for the tacsim content packs".into());
        };
        let mut references = descriptor.references().to_vec();
        references.sort();
        if entry.payload() != descriptor.payload()
            || entry.references() != references
            || entry.is_current() != descriptor.is_current()
        {
            return Err("definition does not match its typed tacsim descriptor".into());
        }
        Ok(())
    }
}
