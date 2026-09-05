//! **Pack identity, and everything cut 1 builds on top of it** (10b §3.1, B1/B4).
//!
//! # The P lens: can two byte streams produce one digest, or one stream two?
//!
//! Neither, and the reason is that this module **defines no digest**. [`pack_digest`]
//! is a thin wrapper over [`mush_core::DefinitionDigest::for_canonical_value_with_limit`]
//! — SHA-256 over `canonical_json_bytes_with_limit` with the domain **length-prefixed**
//! into the hash, which is exactly what 10b §9 q3 asked for and already existed. There
//! is no second hash and no second canonical encoder anywhere in this crate.
//!
//! **What the digest covers, exactly.** [`RulesPack::canonical_value`] builds
//!
//! ```text
//! { "manifest": { "id", "version", "kind", "requires" }, "entries": { id -> entry } }
//! ```
//!
//! and nothing else. Three consequences, each of them a test:
//!
//! - **The manifest's own `digest` field is not in it.** A self-referential hash is
//!   unverifiable, so the field a pack *declares* is compared against a digest taken
//!   over the pack **without** it (10b §3.1's review item).
//! - **`version` is inside the hashed value; the encoding version is not.** The
//!   encoding/domain version rides the domain tag [`PACK_DIGEST_DOMAIN`], which
//!   `hash_parts` length-prefixes, so a pack digest and a same-bytes digest under any
//!   other domain differ by construction (doc 09 §2.6).
//! - **Formatting is not content.** The bytes hashed are the *canonical* encoding of
//!   the parsed pack, so reordering the entries or the keys of an entry changes
//!   nothing, and whitespace inside a *value* changes everything. That pairing is the
//!   only reading of 10b §3.1's digest tests that is self-consistent: hashing raw file
//!   bytes would make a reordered table change the digest, which the same list forbids.
//!
//! A pack whose declared digest was computed over non-canonical bytes therefore fails
//! **at load**, not at first use ([`load_pack`]), because the comparison happens before
//! the pack is admitted to the store at all.
//!
//! # Strict, bounded, deterministic
//!
//! Every wire type here is `deny_unknown_fields` and re-enters through its own
//! constructor (10b §4 rule 1), so a cap is never advisory on load. Entries live in a
//! [`BTreeMap`], so iteration order is the id order and two loads of one file are
//! byte-identical in memory. [`MAX_PACK_ENTRIES`], [`MAX_REQUIRES`], [`MAX_PACK_BYTES`]
//! and [`MAX_SEED_OBJECTS`] are all re-checked by the wire.
//!
//! # No filesystem
//!
//! [`load_pack`] is `bytes -> Result<RulesPack, PackError>` and touches no file. The
//! **host** reads the bytes (`mush_server::config_ext::tacsim_config`) so an unreadable,
//! oversized or mis-digested pack fails the boot naming the file before any module
//! runs; every P-lens test here is a unit test over a string.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, DefinitionDigest};
use serde::{Deserialize, Serialize};

/// The domain tag every TacSim pack digest is taken under.
///
/// `hash_parts` length-prefixes it, so this value *is* the encoding version: change
/// the pack encoding and this string changes with it, and every previously published
/// digest stops matching loudly rather than silently colliding.
pub const PACK_DIGEST_DOMAIN: &str = "tacsim.pack.v1";

/// The most entries one pack may declare (10b §7).
pub const MAX_PACK_ENTRIES: usize = 512;

/// The most objects one campaign-seed pack may instantiate.
///
/// **Deliberately below [`MAX_PACK_ENTRIES`]** (independent verify P5). Every seeded
/// object is a [`PackEntry::Location`], so the seeded count is bounded above by the
/// entry count: at 10b §7's original 2 048 against 512 the entry cap always fired
/// first, [`PackError::TooManySeedObjects`] and `SeedError::TooManyObjects` were both
/// dead, and the latch's own re-check was four times looser than anything a pack
/// could produce. 256 against 512 says what the programme meant — *a seed pack
/// instantiates a subset of its entries* — and makes the cap the plan and the latch
/// both name the one that actually fires.
pub const MAX_SEED_OBJECTS: usize = 256;

/// The most bytes one pack's canonical encoding may occupy.
///
/// Also the limit handed to [`mush_core::DefinitionDigest::for_canonical_value_with_limit`],
/// so an oversized pack is refused *by the encoder* rather than hashed. 512 KiB is
/// two orders of magnitude past a [`MAX_PACK_ENTRIES`]-entry pack, and is deliberately
/// the same figure `ruleset-shadowrun`'s `economy::catalog::MAX_PACK_BYTES` uses for
/// the same job (E6c).
pub const MAX_PACK_BYTES: usize = 512 * 1_024;

/// The most dependencies one pack may declare.
///
/// Bounded because `requires` is walked once per pack at load and a pack that named
/// a thousand dependencies would make boot time a function of authored data.
pub const MAX_REQUIRES: usize = 16;

/// The most bytes a human-facing label inside a pack entry may occupy.
pub const MAX_LABEL_BYTES: usize = 120;

/// The most input or output lines one recipe may declare.
pub const MAX_RECIPE_LINES: usize = 16;

/// The most units one recipe line may declare (independent verify P3).
///
/// A **magnitude** bound, beside the count bounds. Cut 1 authors the numbers and cuts
/// 2, 4 and 6 compute quantity, cost and duration from them, so the bound belongs on
/// the wire rather than at the first arithmetic: `MAX_RECIPE_LINES * MAX_RECIPE_UNITS`
/// is 1.6e10, four orders of magnitude inside `i64`, so no later cost or balance term
/// built from one recipe can overflow. Zero is refused on both sides — a line that
/// consumes nothing or produces nothing is an authoring mistake, not a free lunch.
pub const MAX_RECIPE_UNITS: u64 = 1_000_000_000;

/// The most propellant one route may declare (independent verify P3).
///
/// **Zero is legal and deliberately so**: a ballistic transfer or a towed leg needs no
/// propellant of its own, and refusing `0` would make that unauthorable. The cap bounds
/// the top so a `u64::MAX` plan cannot overflow a later cost term.
pub const MAX_ROUTE_PROPELLANT: u64 = 1_000_000_000;

/// The longest one route leg may take, in unladen seconds (independent verify P3).
///
/// A corridor leg is days, not decades; `u32::MAX` is 136 years. One year, times
/// `mush_sim::MAX_LEGS`, is 1e9 seconds of route, which `mush_sim::route_duration`
/// evaluates without leaving `i64` at any laden ratio a [`DriveProfile`] can express.
///
/// [`DriveProfile`]: mush_sim::DriveProfile
pub const MAX_LEG_SECS: u32 = 31_536_000;

/// Which of Q6A's two kinds a pack is.
///
/// The distinction is load-bearing: *a rules pack is re-pinned and diffed, while
/// re-running a seed pack would duplicate a mine.*
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackKind {
    /// Immutable, digest-referenced rules: commodity families, recipes, route tables.
    RulesDefinition,
    /// Mutable world state instantiated **once**: corridor locations and their kin.
    CampaignSeed,
}

impl PackKind {
    /// The word this kind renders as.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PackKind::RulesDefinition => "rules",
            PackKind::CampaignSeed => "seed",
        }
    }
}

/// One `(pack, version)` dependency a manifest declares.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackRequirement {
    /// The pack depended on.
    pub id: CanonicalId,
    /// The exact version depended on — never a range (Q18A: versions coexist, and a
    /// consumer stays pinned until an explicit migration).
    pub version: u32,
    /// The **content** the dependency must have (independent verify P8).
    ///
    /// A pack is *"immutable and referenced by digest"*, and until this field existed
    /// the `requires` edge was the one reference in the system that was not: it named a
    /// version, so an in-place re-signed edit of a dependency propagated invisibly
    /// through a dependent pack whose own digest never moved. Declaring it makes
    /// [`check_requires`] compare content, and the boot refusal names both digests.
    ///
    /// `Option` now, required at the next pack encoding version (which rides
    /// [`PACK_DIGEST_DOMAIN`]). It enters the hashed value **only when it is `Some`**,
    /// so adding the field moved no already-published digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<DefinitionDigest>,
}

/// A pack's identity and provenance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(into = "PackManifestWire")]
pub struct PackManifest {
    id: CanonicalId,
    version: u32,
    kind: PackKind,
    digest: DefinitionDigest,
    requires: Vec<PackRequirement>,
}

/// The strict wire form of [`PackManifest`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackManifestWire {
    /// The pack id.
    pub id: CanonicalId,
    /// The pack version; `1` upwards, and versions coexist.
    pub version: u32,
    /// Rules or seed.
    pub kind: PackKind,
    /// The declared digest, 64 lowercase hex digits — **not** part of what is hashed.
    pub digest: String,
    /// The exact `(pack, version)` pairs this pack needs.
    #[serde(default)]
    pub requires: Vec<PackRequirement>,
}

impl PackManifest {
    /// The one constructor, and therefore the one place the invariants live.
    ///
    /// # Errors
    ///
    /// [`PackError::ZeroVersion`] for version `0` (versions start at one, so `0` can
    /// mean "unset" nowhere), [`PackError::TooManyRequires`] past [`MAX_REQUIRES`],
    /// [`PackError::SelfRequire`] for a pack that depends on itself, and
    /// [`PackError::DuplicateRequire`] for a pack named twice.
    pub fn new(
        id: CanonicalId,
        version: u32,
        kind: PackKind,
        digest: DefinitionDigest,
        requires: Vec<PackRequirement>,
    ) -> Result<PackManifest, PackError> {
        if version == 0 {
            return Err(PackError::ZeroVersion);
        }
        if requires.len() > MAX_REQUIRES {
            return Err(PackError::TooManyRequires {
                actual: requires.len(),
                max: MAX_REQUIRES,
            });
        }
        let mut seen: BTreeSet<&CanonicalId> = BTreeSet::new();
        for requirement in &requires {
            if requirement.id == id {
                return Err(PackError::SelfRequire { id: id.to_string() });
            }
            if !seen.insert(&requirement.id) {
                return Err(PackError::DuplicateRequire {
                    id: requirement.id.to_string(),
                });
            }
        }
        Ok(PackManifest {
            id,
            version,
            kind,
            digest,
            requires,
        })
    }

    /// The pack id.
    #[must_use]
    pub fn id(&self) -> &CanonicalId {
        &self.id
    }

    /// The pack version.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Rules or seed.
    #[must_use]
    pub fn kind(&self) -> PackKind {
        self.kind
    }

    /// The declared digest.
    #[must_use]
    pub fn digest(&self) -> DefinitionDigest {
        self.digest
    }

    /// The exact `(pack, version)` pairs this pack needs.
    #[must_use]
    pub fn requires(&self) -> &[PackRequirement] {
        &self.requires
    }

    /// The part of the manifest that is **hashed** — everything but `digest`.
    ///
    /// This function is the whole of the "a self-referential hash is unverifiable"
    /// argument: there is no other path from a manifest into the hashed bytes.
    fn hashed_value(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id.as_str(),
            "version": self.version,
            "kind": match self.kind {
                PackKind::RulesDefinition => "rules_definition",
                PackKind::CampaignSeed => "campaign_seed",
            },
            "requires": self
                .requires
                .iter()
                .map(|r| {
                    // The declared dependency digest is inside the hash when it is
                    // declared and absent from it when it is not, so adding the field
                    // (P8) moved no already-published digest while a pack that *does*
                    // bind a dependency's content cannot have that binding edited
                    // without its own digest moving.
                    match &r.digest {
                        Some(digest) => serde_json::json!({
                            "id": r.id.as_str(),
                            "version": r.version,
                            "digest": digest.to_hex(),
                        }),
                        None => serde_json::json!({ "id": r.id.as_str(), "version": r.version }),
                    }
                })
                .collect::<Vec<_>>(),
        })
    }
}

impl From<PackManifest> for PackManifestWire {
    fn from(value: PackManifest) -> PackManifestWire {
        PackManifestWire {
            id: value.id,
            version: value.version,
            kind: value.kind,
            digest: value.digest.to_hex(),
            requires: value.requires,
        }
    }
}

impl TryFrom<PackManifestWire> for PackManifest {
    type Error = PackError;

    fn try_from(value: PackManifestWire) -> Result<PackManifest, PackError> {
        let digest = DefinitionDigest::from_hex(&value.digest).map_err(|error| {
            PackError::BadDigestText {
                detail: error.to_string(),
            }
        })?;
        PackManifest::new(value.id, value.version, value.kind, digest, value.requires)
    }
}

impl<'de> Deserialize<'de> for PackManifest {
    fn deserialize<D>(deserializer: D) -> Result<PackManifest, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PackManifestWire::deserialize(deserializer)?;
        PackManifest::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// One line of a recipe: how much of a commodity family.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeLine {
    /// The commodity family.
    pub commodity: CanonicalId,
    /// How many units. **A definition, never a stock number** — cut 1 holds no
    /// quantity of anything (10b §9 q4: *there is no interim*), and nothing consumes
    /// matter until `mush-inventory` exists.
    pub units: u64,
}

/// What a pack entry declares.
///
/// Adjacently tagged rather than internally tagged, because `deny_unknown_fields`
/// works with adjacent tagging and is the whole point: **an unknown key is an error,
/// never ignored**.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "def",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PackEntry {
    /// One of S4A's ten commodity families.
    Commodity(CommodityDef),
    /// One recipe. Cut 1 authors it and runs it nowhere; cut 4 is production.
    Recipe(RecipeDef),
    /// One route definition, with its endpoints named **symbolically** — the seed
    /// resolves them to the objects it creates, because a pack is authored long
    /// before any object exists.
    Route(RouteEntry),
    /// One place a campaign-seed pack instantiates.
    Location(LocationDef),
}

impl PackEntry {
    /// The word this entry renders as.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            PackEntry::Commodity(_) => "commodity",
            PackEntry::Recipe(_) => "recipe",
            PackEntry::Route(_) => "route",
            PackEntry::Location(_) => "location",
        }
    }

    /// Whether this entry instantiates a world object when a seed pack applies.
    #[must_use]
    pub fn seeds_an_object(&self) -> bool {
        matches!(self, PackEntry::Location(_))
    }

    /// Re-run the entry's own bounds. Called by [`RulesPack`]'s wire for every entry,
    /// so a stored or authored pack cannot smuggle an oversized label past the caps.
    fn check(&self) -> Result<(), PackError> {
        let too_long = |label: &str| label.len() > MAX_LABEL_BYTES;
        match self {
            PackEntry::Commodity(def) => {
                if too_long(&def.label) {
                    return Err(PackError::LabelTooLong {
                        max: MAX_LABEL_BYTES,
                    });
                }
            }
            PackEntry::Location(def) => {
                if too_long(&def.label) {
                    return Err(PackError::LabelTooLong {
                        max: MAX_LABEL_BYTES,
                    });
                }
            }
            PackEntry::Recipe(def) => {
                if too_long(&def.label) {
                    return Err(PackError::LabelTooLong {
                        max: MAX_LABEL_BYTES,
                    });
                }
                if def.inputs.len() > MAX_RECIPE_LINES || def.outputs.len() > MAX_RECIPE_LINES {
                    return Err(PackError::TooManyRecipeLines {
                        max: MAX_RECIPE_LINES,
                    });
                }
                if def.inputs.is_empty() || def.outputs.is_empty() {
                    return Err(PackError::EmptyRecipe);
                }
                // Magnitudes, beside the counts (P3): these are the authored numbers
                // cuts 2/4/6 compute quantity and cost from, so a zero-input recipe or
                // a `u64::MAX` output is refused ON LOAD rather than at first
                // arithmetic.
                for (side, lines) in [("input", &def.inputs), ("output", &def.outputs)] {
                    for line in lines {
                        if line.units == 0 {
                            return Err(PackError::ZeroRecipeUnits {
                                commodity: line.commodity.to_string(),
                                side,
                            });
                        }
                        if line.units > MAX_RECIPE_UNITS {
                            return Err(PackError::RecipeUnitsTooLarge {
                                commodity: line.commodity.to_string(),
                                actual: line.units,
                                max: MAX_RECIPE_UNITS,
                            });
                        }
                    }
                }
            }
            PackEntry::Route(entry) => {
                if entry.legs.is_empty() {
                    return Err(PackError::EmptyRoute);
                }
                if entry.legs.len() > mush_sim::MAX_LEGS {
                    return Err(PackError::TooManyLegs {
                        actual: entry.legs.len(),
                        max: mush_sim::MAX_LEGS,
                    });
                }
                if entry.legs.iter().any(|leg| leg.base_s == 0) {
                    return Err(PackError::ZeroLeg);
                }
                // Magnitudes (P3). A leg is bounded above so a route cannot author a
                // 136-year transit, and the propellant plan is bounded so a later cost
                // term cannot overflow; `propellant == 0` is legal on purpose.
                if let Some(leg) = entry.legs.iter().find(|leg| leg.base_s > MAX_LEG_SECS) {
                    return Err(PackError::LegTooLong {
                        actual: leg.base_s,
                        max: MAX_LEG_SECS,
                    });
                }
                if entry.propellant > MAX_ROUTE_PROPELLANT {
                    return Err(PackError::PropellantTooLarge {
                        actual: entry.propellant,
                        max: MAX_ROUTE_PROPELLANT,
                    });
                }
                if entry.from == entry.to {
                    return Err(PackError::DegenerateRoute);
                }
                if entry.legs.last().map(|leg| &leg.to) != Some(&entry.to) {
                    return Err(PackError::RouteEndpointMismatch);
                }
            }
        }
        Ok(())
    }
}

/// A commodity family (S4A's ten; 10b §9 q10's ids).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommodityDef {
    /// The human-facing name.
    pub label: String,
    /// Whether the corridor fixture actually uses this family in cut 1.
    ///
    /// All ten ids are authored so cut 2 inherits the list from data rather than
    /// code; `metals` and `structural_alloys` are the two cut 1 puts to work.
    #[serde(default)]
    pub in_use: bool,
}

/// A recipe. Authored in cut 1, consumed by cut 4.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeDef {
    /// The human-facing name.
    pub label: String,
    /// What it consumes.
    pub inputs: Vec<RecipeLine>,
    /// What it produces.
    pub outputs: Vec<RecipeLine>,
}

/// A place a campaign-seed pack instantiates.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationDef {
    /// The object's name in the world.
    pub label: String,
    /// What kind of place it is — prose for cut 1, a real classification later.
    pub role: CanonicalId,
}

/// One leg of a pack-authored route: symbolic destination plus unladen time.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackLeg {
    /// The **pack-local** id of the place this leg reaches.
    pub to: CanonicalId,
    /// Unladen seconds at nominal thrust.
    pub base_s: u32,
}

/// A route definition as a pack authors it.
///
/// The materialized [`mush_sim::RouteDef`] needs `ObjId` endpoints, which do not
/// exist until the seed runs — so the pack names places by their pack-local ids and
/// [`crate::seed`] resolves them against what it created.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteEntry {
    /// The pack-local id of the origin.
    pub from: CanonicalId,
    /// The pack-local id of the destination.
    pub to: CanonicalId,
    /// The ordered legs.
    pub legs: Vec<PackLeg>,
    /// Which drive class may fly it.
    pub drive_class: CanonicalId,
    /// Propellant the plan consumes.
    pub propellant: u64,
    /// The declared cadence the body-pair alignment band is measured on.
    ///
    /// `None` through cut 1: no campaign cadence is declared for the corridor yet, and
    /// a sentinel id would be exactly the fail-open shape this programme avoids. Cut 6
    /// binds it, and `mush_sim::route_duration` never reads it — an alignment band is a
    /// departure window, not a duration term.
    #[serde(default)]
    pub alignment_band: Option<u64>,
}

/// A loaded, digest-verified pack.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(into = "RulesPackWire")]
pub struct RulesPack {
    manifest: PackManifest,
    entries: BTreeMap<CanonicalId, PackEntry>,
}

/// The strict wire form of [`RulesPack`] — and the on-disk pack format.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesPackWire {
    /// Identity and provenance.
    pub manifest: PackManifestWire,
    /// The entries, keyed by their pack-local id.
    ///
    /// Decoded through [`deserialize_unique_entries`], so a **repeated** id is an
    /// error rather than last-write-wins (independent verify P6).
    #[serde(deserialize_with = "deserialize_unique_entries")]
    pub entries: BTreeMap<CanonicalId, PackEntry>,
}

/// Decode the entry table, refusing a **duplicate id** and refusing past
/// [`MAX_PACK_ENTRIES`] as it goes (independent verify P6).
///
/// `serde_json` resolves a repeated map key by last-write-wins with no error, unlike
/// the `deny_unknown_fields` struct fields around it. Because the digest is taken over
/// the **parsed** map, a self-consistent file whose authored bytes said one thing and
/// whose loaded semantics said another passed every gate: what runs is what is hashed,
/// so nothing was mis-bound, but a reviewer, a diff or a CI check reading the file
/// could not tell which of two same-named entries the world actually got — in a
/// subsystem whose whole premise is that the file is the authored source of truth.
/// The bound is applied while decoding rather than after, so an absurd file is refused
/// without being materialized first.
fn deserialize_unique_entries<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<CanonicalId, PackEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Visitor;

    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<CanonicalId, PackEntry>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a table of pack entries keyed by their pack-local id")
        }

        fn visit_map<A>(self, mut map: A) -> Result<BTreeMap<CanonicalId, PackEntry>, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut entries: BTreeMap<CanonicalId, PackEntry> = BTreeMap::new();
            while let Some((id, entry)) = map.next_entry::<CanonicalId, PackEntry>()? {
                if entries.contains_key(&id) {
                    return Err(serde::de::Error::custom(PackError::DuplicateEntry {
                        id: id.to_string(),
                    }));
                }
                if entries.len() >= MAX_PACK_ENTRIES {
                    return Err(serde::de::Error::custom(PackError::TooManyEntries {
                        actual: entries.len() + 1,
                        max: MAX_PACK_ENTRIES,
                    }));
                }
                entries.insert(id, entry);
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_map(Visitor)
}

impl RulesPack {
    /// The one constructor, and therefore the one place the invariants live.
    ///
    /// It does **not** verify the declared digest: [`load_pack`] does that, because
    /// verification is a property of the bytes a pack arrived as, not of a value that
    /// was built in memory.
    ///
    /// # Errors
    ///
    /// [`PackError::TooManyEntries`] past [`MAX_PACK_ENTRIES`],
    /// [`PackError::TooManySeedObjects`] past [`MAX_SEED_OBJECTS`],
    /// [`PackError::WrongKindEntry`] for **a location or a route in a rules pack, and
    /// a commodity or recipe in a seed pack**, [`PackError::UnknownRouteEndpoint`] for
    /// a route naming a place the pack does not declare, and whatever
    /// [`PackEntry::check`] refuses.
    ///
    /// The rule reads that way round because a route's endpoints must be places its
    /// **own** pack declares (deviation (d)) and a rules pack may declare no place: a
    /// route in a rules pack is therefore unauthorable, and it is named here as the
    /// kind rule it is rather than surfacing as the endpoint symptom. The shipped
    /// corridor fixture is the other half — a route inside a `campaign_seed` manifest
    /// is exactly what a seed pack is for.
    pub fn new(
        manifest: PackManifest,
        entries: BTreeMap<CanonicalId, PackEntry>,
    ) -> Result<RulesPack, PackError> {
        if entries.len() > MAX_PACK_ENTRIES {
            return Err(PackError::TooManyEntries {
                actual: entries.len(),
                max: MAX_PACK_ENTRIES,
            });
        }
        let seeded = entries.values().filter(|e| e.seeds_an_object()).count();
        if seeded > MAX_SEED_OBJECTS {
            return Err(PackError::TooManySeedObjects {
                actual: seeded,
                max: MAX_SEED_OBJECTS,
            });
        }
        for (id, entry) in &entries {
            entry.check()?;
            // A rules pack instantiates nothing and a seed pack defines no rules.
            // Q6A's whole distinction is that one is re-pinned and diffed while
            // re-running the other would duplicate a mine, so an entry on the wrong
            // side of it is refused rather than quietly ignored.
            let misplaced = matches!(
                (manifest.kind, entry),
                (
                    PackKind::RulesDefinition,
                    PackEntry::Location(_) | PackEntry::Route(_)
                ) | (
                    PackKind::CampaignSeed,
                    PackEntry::Commodity(_) | PackEntry::Recipe(_)
                )
            );
            if misplaced {
                return Err(PackError::WrongKindEntry {
                    id: id.to_string(),
                    entry: entry.label(),
                    kind: manifest.kind.label(),
                });
            }
        }
        // A route's symbolic endpoints have to name places this same pack declares:
        // resolving across packs would make a route's meaning depend on load order.
        for (id, entry) in &entries {
            if let PackEntry::Route(route) = entry {
                let mut named: Vec<&CanonicalId> = vec![&route.from, &route.to];
                named.extend(route.legs.iter().map(|leg| &leg.to));
                for place in named {
                    let declared = matches!(entries.get(place), Some(PackEntry::Location(_)));
                    if !declared {
                        return Err(PackError::UnknownRouteEndpoint {
                            route: id.to_string(),
                            place: place.to_string(),
                        });
                    }
                }
            }
        }
        Ok(RulesPack { manifest, entries })
    }

    /// Identity and provenance.
    #[must_use]
    pub fn manifest(&self) -> &PackManifest {
        &self.manifest
    }

    /// The entries, in id order — deterministic on every iteration.
    #[must_use]
    pub fn entries(&self) -> &BTreeMap<CanonicalId, PackEntry> {
        &self.entries
    }

    /// One entry.
    #[must_use]
    pub fn entry(&self, id: &CanonicalId) -> Option<&PackEntry> {
        self.entries.get(id)
    }

    /// **Exactly the bytes the digest covers** — the manifest without its `digest`
    /// field, plus every entry.
    ///
    /// # Errors
    ///
    /// [`PackError::Encoding`] if an entry will not encode as JSON, which a
    /// `serde_json`-round-tripped entry cannot do.
    pub fn canonical_value(&self) -> Result<serde_json::Value, PackError> {
        let entries = serde_json::to_value(&self.entries).map_err(|error| PackError::Encoding {
            detail: error.to_string(),
        })?;
        Ok(serde_json::json!({
            "manifest": self.manifest.hashed_value(),
            "entries": entries,
        }))
    }

    /// The digest this pack's content actually has.
    ///
    /// # Errors
    ///
    /// [`PackError::Encoding`] and [`PackError::Oversized`].
    pub fn computed_digest(&self) -> Result<DefinitionDigest, PackError> {
        pack_digest(&self.canonical_value()?)
    }
}

impl From<RulesPack> for RulesPackWire {
    fn from(value: RulesPack) -> RulesPackWire {
        RulesPackWire {
            manifest: PackManifestWire::from(value.manifest),
            entries: value.entries,
        }
    }
}

impl TryFrom<RulesPackWire> for RulesPack {
    type Error = PackError;

    fn try_from(value: RulesPackWire) -> Result<RulesPack, PackError> {
        // The entry cap is re-checked before the manifest is built, so an absurd
        // stored pack is refused rather than truncated (10b §3.1's review item).
        if value.entries.len() > MAX_PACK_ENTRIES {
            return Err(PackError::TooManyEntries {
                actual: value.entries.len(),
                max: MAX_PACK_ENTRIES,
            });
        }
        let manifest = PackManifest::try_from(value.manifest)?;
        RulesPack::new(manifest, value.entries)
    }
}

impl<'de> Deserialize<'de> for RulesPack {
    fn deserialize<D>(deserializer: D) -> Result<RulesPack, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RulesPackWire::deserialize(deserializer)?;
        RulesPack::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// **The one digest function in the crate** (B1, 10b §9 q3).
///
/// A thin wrapper over [`mush_core::DefinitionDigest::for_canonical_value_with_limit`]
/// under [`PACK_DIGEST_DOMAIN`] and [`MAX_PACK_BYTES`]. It defines no hash and no
/// canonical encoder of its own, so a TacSim pack's identity is the same discipline
/// every `DefinitionRef` on this tree already uses.
///
/// # Errors
///
/// [`PackError::Oversized`] when the canonical encoding is past [`MAX_PACK_BYTES`],
/// and [`PackError::Encoding`] for anything else the shared encoder refuses.
pub fn pack_digest(canonical: &serde_json::Value) -> Result<DefinitionDigest, PackError> {
    DefinitionDigest::for_canonical_value_with_limit(PACK_DIGEST_DOMAIN, canonical, MAX_PACK_BYTES)
        .map_err(|error| {
            let detail = error.to_string();
            if detail.contains("limit") || detail.contains("byte") {
                PackError::Oversized {
                    max: MAX_PACK_BYTES,
                    detail,
                }
            } else {
                PackError::Encoding { detail }
            }
        })
}

/// **Parse and verify one pack from its bytes.** No filesystem, no world, no clock.
///
/// The order is deliberate: parse strictly, then compute the digest over the
/// canonical encoding, then compare it with what the manifest declared. A pack whose
/// declared digest was computed over non-canonical bytes therefore fails **here**,
/// before it is admitted anywhere, rather than at first use.
///
/// # Errors
///
/// [`PackError::Parse`] for anything the strict wire refuses (an unknown key
/// included), and [`PackError::DigestMismatch`] naming both digests.
pub fn load_pack(bytes: &str) -> Result<RulesPack, PackError> {
    if bytes.len() > MAX_PACK_BYTES {
        return Err(PackError::Oversized {
            max: MAX_PACK_BYTES,
            detail: format!("{} bytes of source", bytes.len()),
        });
    }
    let pack: RulesPack = serde_json::from_str(bytes).map_err(|error| PackError::Parse {
        detail: error.to_string(),
    })?;
    let computed = pack.computed_digest()?;
    let declared = pack.manifest.digest();
    if computed != declared {
        return Err(PackError::DigestMismatch {
            declared: declared.to_hex(),
            computed: computed.to_hex(),
        });
    }
    Ok(pack)
}

/// Resolve a pack's `requires` against everything loaded, failing **closed**.
///
/// # Errors
///
/// [`PackError::MissingRequire`], [`PackError::RequireVersionMismatch`] and
/// [`PackError::RequireDigestMismatch`]; the caller names the file (the host does, at
/// boot).
pub fn check_requires(
    pack: &RulesPack,
    loaded: &BTreeMap<(CanonicalId, u32), RulesPack>,
) -> Result<(), PackError> {
    for requirement in pack.manifest.requires() {
        let key = (requirement.id.clone(), requirement.version);
        if let Some(dependency) = loaded.get(&key) {
            // A dependency edge names a **content**, not a version (P8): a version
            // that was re-signed in place under other bytes is refused here, naming
            // both digests, rather than propagating invisibly through a dependent
            // pack whose own digest never moved.
            if let Some(declared) = requirement.digest {
                let actual = dependency.manifest.digest();
                if declared != actual {
                    return Err(PackError::RequireDigestMismatch {
                        id: requirement.id.to_string(),
                        version: requirement.version,
                        declared: declared.to_hex(),
                        actual: actual.to_hex(),
                    });
                }
            }
            continue;
        }
        // A version of the right pack that is not the one asked for is a *mismatch*,
        // which is a different and more useful message than "missing": it is the
        // difference between "you forgot a file" and "you upgraded underneath me".
        let present: Vec<u32> = loaded
            .keys()
            .filter(|(id, _)| *id == requirement.id)
            .map(|(_, version)| *version)
            .collect();
        if present.is_empty() {
            return Err(PackError::MissingRequire {
                id: requirement.id.to_string(),
                version: requirement.version,
            });
        }
        return Err(PackError::RequireVersionMismatch {
            id: requirement.id.to_string(),
            wanted: requirement.version,
            present: present
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    Ok(())
}

/// Why a pack refuses.
///
/// Every variant is a refusal at **load**; nothing here is deferred to first use.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PackError {
    /// The bytes are not a pack.
    #[error("not a pack: {detail}")]
    Parse {
        /// What the strict decoder said.
        detail: String,
    },
    /// The declared digest is not 64 lowercase hex digits.
    #[error("the declared digest is unreadable: {detail}")]
    BadDigestText {
        /// What the digest parser said.
        detail: String,
    },
    /// The declared digest is not the digest of the content.
    #[error(
        "the declared digest {declared} is not this pack's digest {computed}; the declaration was \
         computed over different or non-canonical bytes"
    )]
    DigestMismatch {
        /// What the manifest claimed.
        declared: String,
        /// What the content actually hashes to.
        computed: String,
    },
    /// Version zero.
    #[error("a pack version starts at one")]
    ZeroVersion,
    /// Past [`MAX_REQUIRES`].
    #[error("a pack declares {actual} dependencies; the maximum is {max}")]
    TooManyRequires {
        /// How many were declared.
        actual: usize,
        /// [`MAX_REQUIRES`].
        max: usize,
    },
    /// A pack that depends on itself.
    #[error("pack {id} declares itself as a dependency")]
    SelfRequire {
        /// The pack.
        id: String,
    },
    /// One dependency named twice.
    #[error("pack {id} is declared as a dependency more than once")]
    DuplicateRequire {
        /// The dependency.
        id: String,
    },
    /// A dependency nothing supplies.
    #[error("requires {id}@{version}, which this server does not load")]
    MissingRequire {
        /// The dependency.
        id: String,
        /// The version wanted.
        version: u32,
    },
    /// A dependency present at a different version.
    #[error("requires {id}@{wanted}, but this server loads {id}@{present}")]
    RequireVersionMismatch {
        /// The dependency.
        id: String,
        /// The version wanted.
        wanted: u32,
        /// The versions actually loaded.
        present: String,
    },
    /// A dependency present at the right version but with **other content**.
    #[error(
        "requires {id}@{version} at digest {declared}, but this server loads that version at \
         {actual}; a pack is referenced by content, not by name"
    )]
    RequireDigestMismatch {
        /// The dependency.
        id: String,
        /// The version wanted.
        version: u32,
        /// The digest the dependent pack declared.
        declared: String,
        /// The digest the loaded dependency actually has.
        actual: String,
    },
    /// Past [`MAX_PACK_ENTRIES`].
    #[error("a pack declares {actual} entries; the maximum is {max}")]
    TooManyEntries {
        /// How many were declared.
        actual: usize,
        /// [`MAX_PACK_ENTRIES`].
        max: usize,
    },
    /// Past [`MAX_SEED_OBJECTS`].
    #[error("a seed pack would instantiate {actual} objects; the maximum is {max}")]
    TooManySeedObjects {
        /// How many were declared.
        actual: usize,
        /// [`MAX_SEED_OBJECTS`].
        max: usize,
    },
    /// A label past [`MAX_LABEL_BYTES`].
    #[error("a pack entry's label is longer than {max} bytes")]
    LabelTooLong {
        /// [`MAX_LABEL_BYTES`].
        max: usize,
    },
    /// A recipe past [`MAX_RECIPE_LINES`].
    #[error("a recipe declares more than {max} input or output lines")]
    TooManyRecipeLines {
        /// [`MAX_RECIPE_LINES`].
        max: usize,
    },
    /// A recipe with no inputs or no outputs.
    #[error("a recipe needs at least one input and one output")]
    EmptyRecipe,
    /// A recipe line that moves no matter at all.
    #[error("a recipe's {side} line for {commodity} declares zero units")]
    ZeroRecipeUnits {
        /// The commodity family the line named.
        commodity: String,
        /// `input` or `output`.
        side: &'static str,
    },
    /// A recipe line past [`MAX_RECIPE_UNITS`].
    #[error("a recipe line for {commodity} declares {actual} units; the maximum is {max}")]
    RecipeUnitsTooLarge {
        /// The commodity family the line named.
        commodity: String,
        /// How many it declared.
        actual: u64,
        /// [`MAX_RECIPE_UNITS`].
        max: u64,
    },
    /// A leg past [`MAX_LEG_SECS`].
    #[error("a route leg declares {actual} seconds; the maximum is {max}")]
    LegTooLong {
        /// How long it declared.
        actual: u32,
        /// [`MAX_LEG_SECS`].
        max: u32,
    },
    /// A propellant plan past [`MAX_ROUTE_PROPELLANT`].
    #[error("a route declares {actual} propellant; the maximum is {max}")]
    PropellantTooLarge {
        /// How much it declared.
        actual: u64,
        /// [`MAX_ROUTE_PROPELLANT`].
        max: u64,
    },
    /// One entry id declared twice in one pack file.
    #[error("pack entry {id} is declared more than once")]
    DuplicateEntry {
        /// The repeated id.
        id: String,
    },
    /// A route with no legs.
    #[error("a route needs at least one leg")]
    EmptyRoute,
    /// Past `mush_sim::MAX_LEGS`.
    #[error("a route declares {actual} legs; the maximum is {max}")]
    TooManyLegs {
        /// How many were declared.
        actual: usize,
        /// `mush_sim::MAX_LEGS`.
        max: usize,
    },
    /// A leg that takes no time.
    #[error("a route leg needs a positive traversal time")]
    ZeroLeg,
    /// Origin and destination are the same place.
    #[error("a route's origin and destination are the same place")]
    DegenerateRoute,
    /// The legs do not arrive where the route says it goes.
    #[error("a route's last leg does not reach its declared destination")]
    RouteEndpointMismatch,
    /// A route naming a place its own pack does not declare.
    #[error("route {route} names the place {place}, which its pack does not declare")]
    UnknownRouteEndpoint {
        /// The route.
        route: String,
        /// The place it named.
        place: String,
    },
    /// An entry on the wrong side of Q6A's rules/seed distinction.
    #[error("entry {id} is a {entry}, which a {kind} pack may not declare")]
    WrongKindEntry {
        /// The entry.
        id: String,
        /// What it is.
        entry: &'static str,
        /// Which kind of pack tried to declare it.
        kind: &'static str,
    },
    /// Past [`MAX_PACK_BYTES`].
    #[error("a pack is larger than this server will hash ({max} bytes): {detail}")]
    Oversized {
        /// [`MAX_PACK_BYTES`].
        max: usize,
        /// What the encoder said.
        detail: String,
    },
    /// A seed latch whose declared count and placement map disagree.
    #[error("a seed latch declares {declared} objects but names {placed}")]
    SeedCountMismatch {
        /// What the record declared.
        declared: u32,
        /// What it actually named.
        placed: usize,
    },
    /// The shared canonical encoder refused.
    #[error("a pack could not be canonically encoded: {detail}")]
    Encoding {
        /// What the encoder said.
        detail: String,
    },
}
