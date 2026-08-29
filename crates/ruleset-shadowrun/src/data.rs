//! SR5 data tables — the config-adjustable game-content seam (M10b).
//!
//! Everything a house rule or a splat sub-module might tune lives here as
//! *data*, never baked into logic: the metatype attribute-range table, the
//! full active-skill list grouped by category, the skill groups, the
//! **priority** chargen table (A–E allotments for Metatype / Attributes /
//! Magic-Resonance / Skills / Resources), the Karma advancement costs, the
//! traditions (+ their Drain attribute), and small canonical Spell / Adept
//! Power / Complex Form lists.
//!
//! The big canonical **content** tables (metatypes, skills, groups,
//! traditions, spells) are `const` — a house rule that rewrites the SR5 skill
//! list is unusual, but they can be swapped wholesale by editing these
//! consts. The tunable **numbers** (priority allotments, Karma multipliers,
//! chargen Karma) additionally ride an [`Sr5Config`] (serde + `Default`,
//! mirroring `ruleset-wod`'s `WodConfig`) so the driver / a house rule builds
//! a modified one and hands it to the ruleset.
//!
//! Verified against the SR5 core rulebook priority/metatype/skill tables; the
//! Magic priority is condensed to the standard Magician-path numbers plus the
//! Adept / Aspected / Technomancer variant ratings (documented per field).
//!
//! ## What the phase-2 engine reads here
//!
//! - chargen priority allotments: [`Sr5Config::attribute_points`],
//!   [`Sr5Config::skill_points`], [`Sr5Config::resource_nuyen`],
//!   [`MAGIC_PRIORITY`], [`METATYPE_PRIORITY`];
//! - metatype attribute min/max + racial qualities: [`METATYPES`] /
//!   [`metatype`];
//! - the active-skill catalogue + linked attributes: [`ACTIVE_SKILLS`],
//!   [`SKILL_GROUPS`], [`KNOWLEDGE_CATEGORIES`];
//! - Karma costs: [`Sr5Config::karma`] + [`KarmaCosts::raise_cost`];
//! - traditions + Drain attribute: [`TRADITIONS`] / [`tradition`];
//! - starting content lists: [`SPELLS`], [`ADEPT_POWERS`], [`COMPLEX_FORMS`].

use serde::{Deserialize, Serialize};

// ----------------------------------------------------------------------
// Attributes
// ----------------------------------------------------------------------

/// The eight core attributes: `(id, label, category)`. Category is the SR5
/// attribute grouping the priority Attribute-point pool spends over
/// (`"physical"` = Body/Agility/Reaction/Strength, `"mental"` =
/// Charisma/Intuition/Logic/Willpower).
pub const CORE_ATTRS: &[(&str, &str, &str)] = &[
    ("body", "Body", "physical"),
    ("agility", "Agility", "physical"),
    ("reaction", "Reaction", "physical"),
    ("strength", "Strength", "physical"),
    ("charisma", "Charisma", "mental"),
    ("intuition", "Intuition", "mental"),
    ("logic", "Logic", "mental"),
    ("willpower", "Willpower", "mental"),
];

/// The special attributes (bought with Metatype-priority special points, not
/// the Attribute pool). Essence is not "bought"; it starts at 6.0.
pub const SPECIAL_ATTRS: &[(&str, &str)] = &[
    ("edge", "Edge"),
    ("magic", "Magic"),
    ("resonance", "Resonance"),
];

/// The absolute schema ceiling for a core attribute (a loose safety envelope;
/// the real per-metatype natural max and the augmented cap of natural-max + 4
/// are enforced from [`METATYPES`] by chargen). Troll natural Body/Strength
/// max 10 + 4 augmentation = 14 is the widest real value; 15 clears it.
pub const ATTR_SCHEMA_MAX: u8 = 15;

/// The augmentation cap: augmented attributes may exceed the metatype natural
/// maximum by this much (SR5: natural max + 4).
pub const AUGMENTED_CAP_BONUS: u8 = 4;

// ----------------------------------------------------------------------
// Metatypes
// ----------------------------------------------------------------------

/// A metatype's per-attribute natural ranges and racial qualities (SR5 core).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metatype {
    /// The metatype id (the `metatype` enum value, e.g. `"troll"`).
    pub id: &'static str,
    /// Human label ("Troll").
    pub name: &'static str,
    /// `(min, max)` natural range per core attribute, in [`CORE_ATTRS`] order.
    pub attr_ranges: [(u8, u8); 8],
    /// Edge `(min, max)`.
    pub edge_range: (u8, u8),
    /// Racial qualities granted for free at chargen (data, not automated).
    pub qualities: &'static [&'static str],
}

impl Metatype {
    /// The `(min, max)` natural range for a core attribute id, or Edge.
    #[must_use]
    pub fn attr_range(&self, attr_id: &str) -> Option<(u8, u8)> {
        if attr_id.eq_ignore_ascii_case("edge") {
            return Some(self.edge_range);
        }
        CORE_ATTRS
            .iter()
            .position(|(id, _, _)| id.eq_ignore_ascii_case(attr_id))
            .map(|i| self.attr_ranges[i])
    }
}

/// The five core metatypes with their SR5 natural attribute ranges and racial
/// qualities. Order of `attr_ranges`: Body, Agility, Reaction, Strength,
/// Charisma, Intuition, Logic, Willpower.
pub const METATYPES: &[Metatype] = &[
    Metatype {
        id: "human",
        name: "Human",
        attr_ranges: [
            (1, 6),
            (1, 6),
            (1, 6),
            (1, 6),
            (1, 6),
            (1, 6),
            (1, 6),
            (1, 6),
        ],
        edge_range: (2, 7),
        qualities: &[],
    },
    Metatype {
        id: "elf",
        name: "Elf",
        attr_ranges: [
            (1, 6),
            (2, 7),
            (1, 6),
            (1, 6),
            (3, 8),
            (1, 6),
            (1, 6),
            (1, 6),
        ],
        edge_range: (1, 6),
        qualities: &["Low-Light Vision"],
    },
    Metatype {
        id: "dwarf",
        name: "Dwarf",
        attr_ranges: [
            (3, 8),
            (1, 6),
            (1, 5),
            (3, 8),
            (1, 6),
            (1, 6),
            (1, 6),
            (2, 7),
        ],
        edge_range: (1, 6),
        qualities: &[
            "Thermographic Vision",
            "+2 dice for Body resistance tests vs pathogens/toxins",
            "+20% lifestyle cost",
        ],
    },
    Metatype {
        id: "ork",
        name: "Ork",
        attr_ranges: [
            (4, 9),
            (1, 6),
            (1, 6),
            (3, 8),
            (1, 5),
            (1, 6),
            (1, 5),
            (1, 6),
        ],
        edge_range: (1, 6),
        qualities: &["Low-Light Vision"],
    },
    Metatype {
        id: "troll",
        name: "Troll",
        attr_ranges: [
            (5, 10),
            (1, 5),
            (1, 6),
            (5, 10),
            (1, 4),
            (1, 5),
            (1, 5),
            (1, 6),
        ],
        edge_range: (1, 6),
        qualities: &["Thermographic Vision", "+1 Reach", "Dermal Armor (+1)"],
    },
];

/// The metatype with this id (case-insensitive).
#[must_use]
pub fn metatype(id: &str) -> Option<&'static Metatype> {
    METATYPES.iter().find(|m| m.id.eq_ignore_ascii_case(id))
}

/// Every metatype id, in table order (for the `metatype` sheet enum).
#[must_use]
pub fn metatype_ids() -> Vec<&'static str> {
    METATYPES.iter().map(|m| m.id).collect()
}

// ----------------------------------------------------------------------
// Awakened types / lifestyles (sheet enum option sources)
// ----------------------------------------------------------------------

/// The awakened / emergent types: `(id, label)`.
pub const AWAKENED_TYPES: &[(&str, &str)] = &[
    ("mundane", "Mundane"),
    ("magician", "Magician"),
    ("adept", "Adept"),
    ("mystic_adept", "Mystic Adept"),
    ("aspected_magician", "Aspected Magician"),
    ("technomancer", "Technomancer"),
];

/// The lifestyle tiers: `(id, label)`.
pub const LIFESTYLES: &[(&str, &str)] = &[
    ("street", "Street"),
    ("squatter", "Squatter"),
    ("low", "Low"),
    ("middle", "Middle"),
    ("high", "High"),
    ("luxury", "Luxury"),
];

// ----------------------------------------------------------------------
// Active skills + skill groups
// ----------------------------------------------------------------------

/// One active skill: id, label, linked attribute id, and category. `defaults`
/// records whether the skill may be defaulted (rolled untrained at −1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveSkill {
    /// Skill id (the `active_skills` map key).
    pub id: &'static str,
    /// Human label.
    pub name: &'static str,
    /// The linked attribute id (default pool = attribute + rating).
    pub attribute: &'static str,
    /// Category: combat / physical / social / technical / vehicle / magical /
    /// resonance.
    pub category: &'static str,
    /// Whether the skill can be used untrained (defaulted at −1).
    pub defaults: bool,
}

const fn sk(
    id: &'static str,
    name: &'static str,
    attribute: &'static str,
    category: &'static str,
    defaults: bool,
) -> ActiveSkill {
    ActiveSkill {
        id,
        name,
        attribute,
        category,
        defaults,
    }
}

/// The SR5 active-skill catalogue, grouped by category (verified against the
/// SR5 core skill list).
pub const ACTIVE_SKILLS: &[ActiveSkill] = &[
    // Combat
    sk("archery", "Archery", "agility", "combat", true),
    sk("automatics", "Automatics", "agility", "combat", true),
    sk("blades", "Blades", "agility", "combat", true),
    sk("clubs", "Clubs", "agility", "combat", true),
    sk(
        "exotic_melee",
        "Exotic Melee Weapon",
        "agility",
        "combat",
        false,
    ),
    sk(
        "exotic_ranged",
        "Exotic Ranged Weapon",
        "agility",
        "combat",
        false,
    ),
    sk("heavy_weapons", "Heavy Weapons", "agility", "combat", true),
    sk("longarms", "Longarms", "agility", "combat", true),
    sk("pistols", "Pistols", "agility", "combat", true),
    sk("throwing", "Throwing Weapons", "agility", "combat", true),
    sk("unarmed", "Unarmed Combat", "agility", "combat", true),
    // Physical
    sk("disguise", "Disguise", "intuition", "physical", true),
    sk("diving", "Diving", "body", "physical", true),
    sk(
        "escape_artist",
        "Escape Artist",
        "agility",
        "physical",
        true,
    ),
    sk("free_fall", "Free-Fall", "body", "physical", true),
    sk("gymnastics", "Gymnastics", "agility", "physical", true),
    sk("palming", "Palming", "agility", "physical", false),
    sk("perception", "Perception", "intuition", "physical", true),
    sk("running", "Running", "strength", "physical", true),
    sk("sneaking", "Sneaking", "agility", "physical", true),
    sk("survival", "Survival", "willpower", "physical", true),
    sk("swimming", "Swimming", "strength", "physical", true),
    sk("tracking", "Tracking", "intuition", "physical", true),
    // Social
    sk("con", "Con", "charisma", "social", true),
    sk("etiquette", "Etiquette", "charisma", "social", true),
    sk("impersonation", "Impersonation", "charisma", "social", true),
    sk("instruction", "Instruction", "charisma", "social", true),
    sk("intimidation", "Intimidation", "charisma", "social", true),
    sk("leadership", "Leadership", "charisma", "social", true),
    sk("negotiation", "Negotiation", "charisma", "social", true),
    sk("performance", "Performance", "charisma", "social", true),
    // Technical
    sk(
        "aeronautics_mechanic",
        "Aeronautics Mechanic",
        "logic",
        "technical",
        false,
    ),
    sk(
        "animal_handling",
        "Animal Handling",
        "charisma",
        "technical",
        true,
    ),
    sk("armorer", "Armorer", "logic", "technical", true),
    sk("artisan", "Artisan", "intuition", "technical", false),
    sk(
        "automotive_mechanic",
        "Automotive Mechanic",
        "logic",
        "technical",
        false,
    ),
    sk(
        "biotechnology",
        "Biotechnology",
        "logic",
        "technical",
        false,
    ),
    sk("chemistry", "Chemistry", "logic", "technical", false),
    sk("computer", "Computer", "logic", "technical", true),
    sk("cybercombat", "Cybercombat", "logic", "technical", true),
    sk(
        "cybertechnology",
        "Cybertechnology",
        "logic",
        "technical",
        false,
    ),
    sk("demolitions", "Demolitions", "logic", "technical", true),
    sk(
        "electronic_warfare",
        "Electronic Warfare",
        "intuition",
        "technical",
        false,
    ),
    sk("first_aid", "First Aid", "logic", "technical", true),
    sk("forgery", "Forgery", "logic", "technical", true),
    sk("hacking", "Hacking", "logic", "technical", true),
    sk("hardware", "Hardware", "logic", "technical", false),
    sk(
        "industrial_mechanic",
        "Industrial Mechanic",
        "logic",
        "technical",
        false,
    ),
    sk("locksmith", "Locksmith", "agility", "technical", false),
    sk("medicine", "Medicine", "logic", "technical", false),
    sk(
        "nautical_mechanic",
        "Nautical Mechanic",
        "logic",
        "technical",
        false,
    ),
    sk("navigation", "Navigation", "intuition", "technical", true),
    sk("software", "Software", "logic", "technical", false),
    // Vehicle
    sk("gunnery", "Gunnery", "agility", "vehicle", true),
    sk(
        "pilot_aerospace",
        "Pilot Aerospace",
        "reaction",
        "vehicle",
        false,
    ),
    sk(
        "pilot_aircraft",
        "Pilot Aircraft",
        "reaction",
        "vehicle",
        false,
    ),
    sk(
        "pilot_ground",
        "Pilot Ground Craft",
        "reaction",
        "vehicle",
        true,
    ),
    sk("pilot_walker", "Pilot Walker", "reaction", "vehicle", false),
    sk(
        "pilot_watercraft",
        "Pilot Watercraft",
        "reaction",
        "vehicle",
        true,
    ),
    // Magical
    sk("alchemy", "Alchemy", "magic", "magical", false),
    sk("arcana", "Arcana", "logic", "magical", false),
    sk("artificing", "Artificing", "magic", "magical", false),
    sk("assensing", "Assensing", "intuition", "magical", false),
    sk(
        "astral_combat",
        "Astral Combat",
        "willpower",
        "magical",
        false,
    ),
    sk("banishing", "Banishing", "magic", "magical", false),
    sk("binding", "Binding", "magic", "magical", false),
    sk(
        "counterspelling",
        "Counterspelling",
        "magic",
        "magical",
        false,
    ),
    sk("disenchanting", "Disenchanting", "magic", "magical", false),
    sk(
        "ritual_spellcasting",
        "Ritual Spellcasting",
        "magic",
        "magical",
        false,
    ),
    sk("spellcasting", "Spellcasting", "magic", "magical", false),
    sk("summoning", "Summoning", "magic", "magical", false),
    // Resonance
    sk("compiling", "Compiling", "resonance", "resonance", false),
    sk(
        "decompiling",
        "Decompiling",
        "resonance",
        "resonance",
        false,
    ),
    sk(
        "registering",
        "Registering",
        "resonance",
        "resonance",
        false,
    ),
];

/// The active skill with this id (case-insensitive).
#[must_use]
pub fn active_skill(id: &str) -> Option<&'static ActiveSkill> {
    ACTIVE_SKILLS.iter().find(|s| s.id.eq_ignore_ascii_case(id))
}

/// A skill group: a rating buys every constituent active skill at that rating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkillGroup {
    /// Group id (the `skill_groups` map key).
    pub id: &'static str,
    /// Human label.
    pub name: &'static str,
    /// The active-skill ids the group covers.
    pub skills: &'static [&'static str],
}

/// The SR5 skill groups. Every member id is a real [`ACTIVE_SKILLS`] entry
/// (asserted in tests).
pub const SKILL_GROUPS: &[SkillGroup] = &[
    SkillGroup {
        id: "acting",
        name: "Acting",
        skills: &["con", "impersonation", "performance"],
    },
    SkillGroup {
        id: "athletics",
        name: "Athletics",
        skills: &["gymnastics", "running", "swimming"],
    },
    SkillGroup {
        id: "biotech",
        name: "Biotech",
        skills: &["biotechnology", "cybertechnology", "first_aid", "medicine"],
    },
    SkillGroup {
        id: "close_combat",
        name: "Close Combat",
        skills: &["blades", "clubs", "unarmed"],
    },
    SkillGroup {
        id: "conjuring",
        name: "Conjuring",
        skills: &["banishing", "binding", "summoning"],
    },
    SkillGroup {
        id: "cracking",
        name: "Cracking",
        skills: &["cybercombat", "electronic_warfare", "hacking"],
    },
    SkillGroup {
        id: "electronics",
        name: "Electronics",
        skills: &["computer", "hardware", "software"],
    },
    SkillGroup {
        id: "enchanting",
        name: "Enchanting",
        skills: &["alchemy", "artificing", "disenchanting"],
    },
    SkillGroup {
        id: "engineering",
        name: "Engineering",
        skills: &[
            "aeronautics_mechanic",
            "automotive_mechanic",
            "industrial_mechanic",
            "nautical_mechanic",
        ],
    },
    SkillGroup {
        id: "firearms",
        name: "Firearms",
        skills: &["automatics", "longarms", "pistols"],
    },
    SkillGroup {
        id: "influence",
        name: "Influence",
        skills: &["etiquette", "leadership", "negotiation"],
    },
    SkillGroup {
        id: "outdoors",
        name: "Outdoors",
        skills: &["navigation", "survival", "tracking"],
    },
    SkillGroup {
        id: "sorcery",
        name: "Sorcery",
        skills: &["counterspelling", "ritual_spellcasting", "spellcasting"],
    },
    SkillGroup {
        id: "stealth",
        name: "Stealth",
        skills: &["disguise", "palming", "sneaking"],
    },
    SkillGroup {
        id: "tasking",
        name: "Tasking",
        skills: &["compiling", "decompiling", "registering"],
    },
];

/// The skill group with this id (case-insensitive).
#[must_use]
pub fn skill_group(id: &str) -> Option<&'static SkillGroup> {
    SKILL_GROUPS.iter().find(|g| g.id.eq_ignore_ascii_case(id))
}

/// Knowledge-skill categories: `(id, name, linked_attribute)`. Knowledge and
/// Language skills use a separate point pool (Logic + Intuition) at chargen;
/// they are stored open-ended in the `knowledge_skills` / `language_skills`
/// sheet maps. This is the *shape* — individual knowledges are free text.
pub const KNOWLEDGE_CATEGORIES: &[(&str, &str, &str)] = &[
    ("academic", "Academic", "logic"),
    ("professional", "Professional", "logic"),
    ("street", "Street", "intuition"),
    ("interests", "Interests", "intuition"),
];

// ----------------------------------------------------------------------
// Traditions
// ----------------------------------------------------------------------

/// A magical tradition and its Drain resistance attribute. Drain is resisted
/// with Willpower + this attribute (Fading, for technomancers, is always
/// Willpower + Resonance and needs no tradition).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tradition {
    /// Tradition id (the `tradition` enum value).
    pub id: &'static str,
    /// Human label.
    pub name: &'static str,
    /// The Drain resistance attribute id (paired with Willpower).
    pub drain_attribute: &'static str,
}

/// Canonical traditions with their Drain attributes (SR5 core Hermetic +
/// Shamanic, plus common Street Grimoire traditions). Extendable — this is the
/// config seam for house traditions.
pub const TRADITIONS: &[Tradition] = &[
    Tradition {
        id: "hermetic",
        name: "Hermetic",
        drain_attribute: "logic",
    },
    Tradition {
        id: "shamanic",
        name: "Shamanic",
        drain_attribute: "charisma",
    },
    Tradition {
        id: "buddhist",
        name: "Path of the Wheel (Buddhist)",
        drain_attribute: "intuition",
    },
    Tradition {
        id: "christian_theurgy",
        name: "Christian Theurgy",
        drain_attribute: "intuition",
    },
    Tradition {
        id: "chaos",
        name: "Chaos Magic",
        drain_attribute: "logic",
    },
    Tradition {
        id: "wuxing",
        name: "Wuxing",
        drain_attribute: "intuition",
    },
    Tradition {
        id: "black_magic",
        name: "Black Magic",
        drain_attribute: "charisma",
    },
    Tradition {
        id: "sioux",
        name: "Sioux (Wild)",
        drain_attribute: "charisma",
    },
];

/// The tradition with this id (case-insensitive).
#[must_use]
pub fn tradition(id: &str) -> Option<&'static Tradition> {
    TRADITIONS.iter().find(|t| t.id.eq_ignore_ascii_case(id))
}

/// Tradition ids plus a leading `"none"` (for the `tradition` sheet enum).
#[must_use]
pub fn tradition_options() -> Vec<&'static str> {
    let mut v = vec!["none"];
    v.extend(TRADITIONS.iter().map(|t| t.id));
    v
}

// ----------------------------------------------------------------------
// Spells / adept powers / complex forms (small canonical lists)
// ----------------------------------------------------------------------

/// A spell name + category (sheet-tracked, pool-ROLLED — casting rolls
/// Magic + Spellcasting; no per-spell automation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spell {
    /// Spell name.
    pub name: &'static str,
    /// Category: combat / detection / health / illusion / manipulation.
    pub category: &'static str,
}

/// A small canonical spell list spanning every category.
pub const SPELLS: &[Spell] = &[
    Spell {
        name: "Manabolt",
        category: "combat",
    },
    Spell {
        name: "Stunbolt",
        category: "combat",
    },
    Spell {
        name: "Fireball",
        category: "combat",
    },
    Spell {
        name: "Lightning Bolt",
        category: "combat",
    },
    Spell {
        name: "Powerbolt",
        category: "combat",
    },
    Spell {
        name: "Analyze Truth",
        category: "detection",
    },
    Spell {
        name: "Clairvoyance",
        category: "detection",
    },
    Spell {
        name: "Detect Life",
        category: "detection",
    },
    Spell {
        name: "Mind Probe",
        category: "detection",
    },
    Spell {
        name: "Heal",
        category: "health",
    },
    Spell {
        name: "Increase Reflexes",
        category: "health",
    },
    Spell {
        name: "Stabilize",
        category: "health",
    },
    Spell {
        name: "Antidote",
        category: "health",
    },
    Spell {
        name: "Invisibility",
        category: "illusion",
    },
    Spell {
        name: "Improved Invisibility",
        category: "illusion",
    },
    Spell {
        name: "Physical Mask",
        category: "illusion",
    },
    Spell {
        name: "Trid Phantasm",
        category: "illusion",
    },
    Spell {
        name: "Levitate",
        category: "manipulation",
    },
    Spell {
        name: "Magic Fingers",
        category: "manipulation",
    },
    Spell {
        name: "Armor",
        category: "manipulation",
    },
    Spell {
        name: "Physical Barrier",
        category: "manipulation",
    },
    Spell {
        name: "Control Thoughts",
        category: "manipulation",
    },
];

/// An adept power name, its Power Point cost (in hundredths of a PP, so a
/// 0.25-PP power is `25`), and its maximum level (`1` = not levelled).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdeptPower {
    /// Power name.
    pub name: &'static str,
    /// Cost per level in hundredths of a Power Point.
    pub cost_pp_hundredths: u16,
    /// Maximum level.
    pub max_level: u8,
}

/// A small canonical adept-power list (Power Points = Magic rating).
pub const ADEPT_POWERS: &[AdeptPower] = &[
    AdeptPower {
        name: "Improved Reflexes",
        cost_pp_hundredths: 150,
        max_level: 3,
    },
    AdeptPower {
        name: "Improved Ability (skill)",
        cost_pp_hundredths: 50,
        max_level: 3,
    },
    AdeptPower {
        name: "Attribute Boost",
        cost_pp_hundredths: 25,
        max_level: 3,
    },
    AdeptPower {
        name: "Critical Strike",
        cost_pp_hundredths: 25,
        max_level: 3,
    },
    AdeptPower {
        name: "Killing Hands",
        cost_pp_hundredths: 50,
        max_level: 1,
    },
    AdeptPower {
        name: "Combat Sense",
        cost_pp_hundredths: 50,
        max_level: 3,
    },
    AdeptPower {
        name: "Mystic Armor",
        cost_pp_hundredths: 50,
        max_level: 3,
    },
    AdeptPower {
        name: "Danger Sense",
        cost_pp_hundredths: 25,
        max_level: 3,
    },
    AdeptPower {
        name: "Enhanced Perception",
        cost_pp_hundredths: 25,
        max_level: 3,
    },
    AdeptPower {
        name: "Traceless Walk",
        cost_pp_hundredths: 100,
        max_level: 1,
    },
    AdeptPower {
        name: "Great Leap",
        cost_pp_hundredths: 50,
        max_level: 3,
    },
    AdeptPower {
        name: "Wall Running",
        cost_pp_hundredths: 50,
        max_level: 1,
    },
];

/// A small canonical complex-form list (technomancers; Fading = Willpower +
/// Resonance).
pub const COMPLEX_FORMS: &[&str] = &[
    "Cleaner",
    "Editor",
    "Pulse Storm",
    "Puppeteer",
    "Resonance Spike",
    "Static Bomb",
    "Static Veil",
    "Tattletale",
    "Transcendent Grid",
    "Infusion",
    "Diffusion",
];

// ----------------------------------------------------------------------
// Priority chargen table (A–E)
// ----------------------------------------------------------------------

/// A chargen priority level. Each of A–E is assigned once across the five
/// categories (Metatype / Attributes / Magic-Resonance / Skills / Resources).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Priority {
    /// Priority A (highest allotment).
    A,
    /// Priority B.
    B,
    /// Priority C.
    C,
    /// Priority D.
    D,
    /// Priority E (lowest allotment).
    E,
}

impl Priority {
    /// Every priority, highest first.
    pub const ALL: [Priority; 5] = [
        Priority::A,
        Priority::B,
        Priority::C,
        Priority::D,
        Priority::E,
    ];

    /// The 0-based index (`A` → 0 … `E` → 4).
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Priority::A => 0,
            Priority::B => 1,
            Priority::C => 2,
            Priority::D => 3,
            Priority::E => 4,
        }
    }

    /// The uppercase letter.
    #[must_use]
    pub fn as_char(self) -> char {
        ['A', 'B', 'C', 'D', 'E'][self.index()]
    }

    /// Parse a priority letter (case-insensitive).
    #[must_use]
    pub fn from_char(c: char) -> Option<Priority> {
        match c.to_ascii_uppercase() {
            'A' => Some(Priority::A),
            'B' => Some(Priority::B),
            'C' => Some(Priority::C),
            'D' => Some(Priority::D),
            'E' => Some(Priority::E),
            _ => None,
        }
    }
}

/// The Metatype-priority special-attribute allotment per metatype, by priority
/// (SR5 core). The number is the special-attribute points (spent on Edge, or
/// Magic/Resonance when not taken from the Magic column). All five metatypes
/// are selectable at every priority in SR5; lower priorities simply grant
/// fewer special points. Rows are indexed by [`Priority::index`]; columns are
/// `[human, elf, dwarf, ork, troll]`.
pub const METATYPE_PRIORITY: [[u8; 5]; 5] = [
    [9, 8, 7, 7, 5], // A
    [7, 6, 4, 4, 0], // B
    [5, 3, 1, 0, 0], // C
    [3, 0, 0, 0, 0], // D
    [1, 0, 0, 0, 0], // E
];

/// The metatype column order of [`METATYPE_PRIORITY`].
pub const METATYPE_PRIORITY_ORDER: [&str; 5] = ["human", "elf", "dwarf", "ork", "troll"];

/// Special-attribute points for `metatype_id` at `priority`.
#[must_use]
pub fn metatype_special_points(priority: Priority, metatype_id: &str) -> Option<u8> {
    let col = METATYPE_PRIORITY_ORDER
        .iter()
        .position(|m| m.eq_ignore_ascii_case(metatype_id))?;
    Some(METATYPE_PRIORITY[priority.index()][col])
}

/// The Magic/Resonance priority row: the standard SR5 allotments, condensed.
/// The Magician path is the headline (`magician_magic` + `free_spells` +
/// `free_magic_skills` at `free_magic_skill_rating`); the Adept, Aspected
/// Magician, and Technomancer variant ratings are carried alongside. Priority
/// E is mundane (all zero).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MagicPriorityRow {
    /// The priority this row is for.
    pub priority: Priority,
    /// Magician / Mystic Adept starting Magic (0 = path unavailable here).
    pub magician_magic: u8,
    /// Free spells (Magician / Mystic Adept).
    pub free_spells: u8,
    /// Rating of the free Magical skills granted.
    pub free_magic_skill_rating: u8,
    /// Count of free Magical skills granted at `free_magic_skill_rating`.
    pub free_magic_skill_count: u8,
    /// Adept starting Magic (0 = unavailable).
    pub adept_magic: u8,
    /// Aspected Magician starting Magic (0 = unavailable).
    pub aspected_magic: u8,
    /// Technomancer starting Resonance (0 = unavailable).
    pub technomancer_resonance: u8,
    /// Free complex forms (Technomancer).
    pub complex_forms: u8,
}

/// The Magic/Resonance priority table (indexed by [`Priority::index`]).
///
/// Standard SR5 numbers: Magician/Mystic Adept exist at A/B/C; Adept and
/// Aspected at B/C/D; Technomancer at A/B/C; E is mundane.
pub const MAGIC_PRIORITY: [MagicPriorityRow; 5] = [
    MagicPriorityRow {
        priority: Priority::A,
        magician_magic: 6,
        free_spells: 10,
        free_magic_skill_rating: 5,
        free_magic_skill_count: 2,
        // Adept is NOT an available talent at priority A in SR5 core (Adept
        // starts at B). Verified against Chummer's priorities.xml: the A talent
        // set is Magician / Mystic Adept / Technomancer only.
        adept_magic: 0,
        aspected_magic: 0,
        technomancer_resonance: 6,
        complex_forms: 5,
    },
    MagicPriorityRow {
        priority: Priority::B,
        magician_magic: 6,
        free_spells: 7,
        free_magic_skill_rating: 4,
        free_magic_skill_count: 2,
        adept_magic: 6,
        aspected_magic: 5,
        technomancer_resonance: 4,
        complex_forms: 2,
    },
    MagicPriorityRow {
        // Priority C core RAW (verified against Chummer's priorities.xml):
        // Magician/Mystic Adept Magic 3 + 5 spells (no free Magical skills),
        // Adept Magic 4, Aspected Magician Magic 3, Technomancer Resonance 3.
        // (The "Magic 5" Aspected-like talents at C are Forbidden Arcana splat
        // options — Enchanter/Explorer — not core, so Aspected core C = 3.)
        priority: Priority::C,
        magician_magic: 3,
        free_spells: 5,
        free_magic_skill_rating: 0,
        free_magic_skill_count: 0,
        adept_magic: 4,
        aspected_magic: 3,
        technomancer_resonance: 3,
        complex_forms: 1,
    },
    MagicPriorityRow {
        priority: Priority::D,
        magician_magic: 0,
        free_spells: 0,
        free_magic_skill_rating: 0,
        free_magic_skill_count: 0,
        adept_magic: 2,
        aspected_magic: 2,
        technomancer_resonance: 0,
        complex_forms: 0,
    },
    MagicPriorityRow {
        priority: Priority::E,
        magician_magic: 0,
        free_spells: 0,
        free_magic_skill_rating: 0,
        free_magic_skill_count: 0,
        adept_magic: 0,
        aspected_magic: 0,
        technomancer_resonance: 0,
        complex_forms: 0,
    },
];

/// The Magic priority row for `priority`.
#[must_use]
pub fn magic_priority(priority: Priority) -> MagicPriorityRow {
    MAGIC_PRIORITY[priority.index()]
}

// ----------------------------------------------------------------------
// Karma advancement costs
// ----------------------------------------------------------------------

/// Karma advancement multipliers (SR5 core p.104). The cost to raise a rated
/// trait by one step is `new_rating × mult`; totals sum the steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KarmaCosts {
    /// Attribute: new rating × 5.
    pub attribute_mult: u32,
    /// Special attribute (Edge / Magic / Resonance): new rating × 5.
    pub special_attribute_mult: u32,
    /// Active skill: new rating × 2.
    pub active_skill_mult: u32,
    /// Skill group: new rating × 5.
    pub skill_group_mult: u32,
    /// Knowledge / Language skill: new rating × 1.
    pub knowledge_skill_mult: u32,
    /// New active-skill specialization: flat 7.
    pub specialization: u32,
    /// Learn a spell: flat 5.
    pub spell: u32,
    /// Learn a complex form: flat 4.
    pub complex_form: u32,
    /// Learn a ritual: flat 5.
    pub ritual: u32,
    /// Initiation / Submersion base: `base + grade × grade_mult`.
    pub initiation_base: u32,
    /// Initiation / Submersion per-grade multiplier.
    pub initiation_grade_mult: u32,
}

impl Default for KarmaCosts {
    fn default() -> KarmaCosts {
        KarmaCosts {
            attribute_mult: 5,
            special_attribute_mult: 5,
            active_skill_mult: 2,
            skill_group_mult: 5,
            knowledge_skill_mult: 1,
            specialization: 7,
            spell: 5,
            complex_form: 4,
            ritual: 5,
            initiation_base: 10,
            initiation_grade_mult: 3,
        }
    }
}

impl KarmaCosts {
    /// Karma to raise a `new rating × mult` trait from `from` to `to` (`0` if
    /// `to <= from`): sums `r × mult` for each new rating `r` in `from+1..=to`.
    #[must_use]
    pub fn raise_cost(mult: u32, from: u32, to: u32) -> u32 {
        if to <= from {
            return 0;
        }
        (from + 1..=to).map(|r| r * mult).sum()
    }

    /// Karma to raise an attribute from `from` to `to`.
    #[must_use]
    pub fn attribute_cost(&self, from: u32, to: u32) -> u32 {
        KarmaCosts::raise_cost(self.attribute_mult, from, to)
    }

    /// Karma to raise an active skill from `from` to `to`.
    #[must_use]
    pub fn active_skill_cost(&self, from: u32, to: u32) -> u32 {
        KarmaCosts::raise_cost(self.active_skill_mult, from, to)
    }

    /// Karma to raise a skill group from `from` to `to`.
    #[must_use]
    pub fn skill_group_cost(&self, from: u32, to: u32) -> u32 {
        KarmaCosts::raise_cost(self.skill_group_mult, from, to)
    }

    /// Karma to initiate/submerge to `grade` from `grade - 1`.
    #[must_use]
    pub fn initiation_cost(&self, grade: u32) -> u32 {
        self.initiation_base + grade * self.initiation_grade_mult
    }
}

// ----------------------------------------------------------------------
// The tunable config bundle
// ----------------------------------------------------------------------

/// The tunable SR5 chargen/advancement numbers — the config seam a house rule
/// or the driver adjusts (the ruleset carries one, mirroring `WodConfig`).
/// Defaults are the SR5 core priority/karma values exactly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sr5Config {
    /// Attribute-priority points, indexed by [`Priority::index`]
    /// (A=24 … E=12).
    pub attribute_points: [u16; 5],
    /// Skill-priority `(skill_points, skill_group_points)`, by priority
    /// (A=(46,10) … E=(18,0)).
    pub skill_points: [(u16, u16); 5],
    /// Resource-priority nuyen, by priority (A=450000 … E=6000).
    pub resource_nuyen: [u32; 5],
    /// Starting Karma at chargen (SR5: 25).
    pub chargen_karma: u32,
    /// Karma advancement costs.
    pub karma: KarmaCosts,
}

impl Default for Sr5Config {
    fn default() -> Sr5Config {
        Sr5Config {
            attribute_points: [24, 20, 16, 14, 12],
            skill_points: [(46, 10), (36, 5), (28, 2), (22, 0), (18, 0)],
            resource_nuyen: [450_000, 275_000, 140_000, 50_000, 6_000],
            chargen_karma: 25,
            karma: KarmaCosts::default(),
        }
    }
}

impl Sr5Config {
    /// Attribute points at `priority`.
    #[must_use]
    pub fn attribute_points(&self, priority: Priority) -> u16 {
        self.attribute_points[priority.index()]
    }

    /// `(skill_points, skill_group_points)` at `priority`.
    #[must_use]
    pub fn skill_points(&self, priority: Priority) -> (u16, u16) {
        self.skill_points[priority.index()]
    }

    /// Resource nuyen at `priority`.
    #[must_use]
    pub fn resource_nuyen(&self, priority: Priority) -> u32 {
        self.resource_nuyen[priority.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn metatypes_cover_the_five_and_ranges_are_sane() {
        assert_eq!(METATYPES.len(), 5);
        for m in METATYPES {
            // Every range min <= max, and the 8 core attrs are present.
            assert_eq!(m.attr_ranges.len(), 8);
            for (lo, hi) in m.attr_ranges {
                assert!(lo >= 1 && lo <= hi && hi <= ATTR_SCHEMA_MAX);
            }
            let (elo, ehi) = m.edge_range;
            assert!(elo >= 1 && elo <= ehi);
        }
        // Troll Body 5..10, Strength 5..10 (the metatype signature).
        let troll = metatype("troll").unwrap();
        assert_eq!(troll.attr_range("body"), Some((5, 10)));
        assert_eq!(troll.attr_range("strength"), Some((5, 10)));
        // Human Edge 2..7.
        assert_eq!(metatype("human").unwrap().edge_range, (2, 7));
    }

    #[test]
    fn skill_group_members_are_all_real_active_skills() {
        for g in SKILL_GROUPS {
            for s in g.skills {
                assert!(
                    active_skill(s).is_some(),
                    "group {:?} references unknown skill {s:?}",
                    g.id
                );
            }
        }
    }

    #[test]
    fn active_skill_ids_are_unique() {
        let mut seen = HashSet::new();
        for s in ACTIVE_SKILLS {
            assert!(seen.insert(s.id), "duplicate active skill id {:?}", s.id);
            // Linked attribute is a real core/special attribute.
            let known = CORE_ATTRS.iter().any(|(id, _, _)| *id == s.attribute)
                || SPECIAL_ATTRS.iter().any(|(id, _)| *id == s.attribute);
            assert!(
                known,
                "skill {:?} links unknown attribute {:?}",
                s.id, s.attribute
            );
        }
    }

    #[test]
    fn priority_defaults_match_sr5_core() {
        let c = Sr5Config::default();
        assert_eq!(c.attribute_points(Priority::A), 24);
        assert_eq!(c.attribute_points(Priority::E), 12);
        assert_eq!(c.skill_points(Priority::A), (46, 10));
        assert_eq!(c.skill_points(Priority::C), (28, 2));
        assert_eq!(c.resource_nuyen(Priority::A), 450_000);
        assert_eq!(c.resource_nuyen(Priority::E), 6_000);
        assert_eq!(c.chargen_karma, 25);
        // Metatype special points: A/Troll = 5, C/Ork = 0.
        assert_eq!(metatype_special_points(Priority::A, "troll"), Some(5));
        assert_eq!(metatype_special_points(Priority::C, "ork"), Some(0));
        // Magic A magician = 6, E = mundane.
        assert_eq!(magic_priority(Priority::A).magician_magic, 6);
        assert_eq!(magic_priority(Priority::E).magician_magic, 0);
    }

    #[test]
    fn magic_priority_c_and_a_match_sr5_core() {
        // Verified against Chummer's priorities.xml (SR5 core talents).
        let a = magic_priority(Priority::A);
        // Adept is not available at priority A (starts at B).
        assert_eq!(a.adept_magic, 0);
        assert_eq!(a.magician_magic, 6);
        assert_eq!(a.technomancer_resonance, 6);
        let c = magic_priority(Priority::C);
        assert_eq!(c.magician_magic, 3, "C Magician = Magic 3");
        assert_eq!(c.free_spells, 5);
        assert_eq!(c.adept_magic, 4, "C Adept = Magic 4 (out-magics the mage)");
        assert_eq!(c.aspected_magic, 3, "C Aspected core = Magic 3");
        assert_eq!(c.technomancer_resonance, 3, "C Technomancer = Resonance 3");
        // The well-known SR5 quirk: at C, the Adept (4) out-rates the full
        // Magician (3).
        assert!(c.adept_magic > c.magician_magic);
    }

    #[test]
    fn karma_costs_follow_new_rating_times_mult() {
        let k = KarmaCosts::default();
        // Attribute 3→4 costs 4×5 = 20; 1→3 costs 2×5 + 3×5 = 25.
        assert_eq!(k.attribute_cost(3, 4), 20);
        assert_eq!(k.attribute_cost(1, 3), 25);
        // Active skill 0→2 costs 1×2 + 2×2 = 6.
        assert_eq!(k.active_skill_cost(0, 2), 6);
        // Skill group 2→3 costs 3×5 = 15.
        assert_eq!(k.skill_group_cost(2, 3), 15);
        // Initiation to grade 1 = 10 + 1×3 = 13.
        assert_eq!(k.initiation_cost(1), 13);
        // Flat costs.
        assert_eq!(k.specialization, 7);
        assert_eq!(k.spell, 5);
    }

    #[test]
    fn tradition_drain_attributes() {
        assert_eq!(tradition("hermetic").unwrap().drain_attribute, "logic");
        assert_eq!(tradition("shamanic").unwrap().drain_attribute, "charisma");
        assert!(tradition_options().contains(&"none"));
    }
}
