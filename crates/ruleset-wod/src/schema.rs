//! The base oWoD20 (Mortal) sheet schema and the **template seam** M6b splats
//! bolt onto.
//!
//! The base sheet is the classic Storyteller-System character sheet from the
//! milestone crib: nine Attributes (Physical/Social/Mental), thirty Abilities
//! split Talents/Skills/Knowledges (each carrying free-text specialties), an
//! open Backgrounds map, Willpower (permanent + temporary), the fixed 7-level
//! Health track (Bruised → Incapacitated with the exact penalties, marked
//! Bashing/Lethal/Aggravated), the three Virtues, Merits/Flaws, Experience,
//! and Nature/Demeanor/Concept.
//!
//! ## The template seam (M6b)
//!
//! A [`Template`] extends the base schema **without editing it**, exactly the
//! `SheetSchemaBuilder::from_schema` composition the framework proves. The base
//! `"mortal"` template ([`Mortal`]) adds nothing — it exists to occupy the
//! template slot and to prove the fold is a no-op. A splat sub-module (Vampire
//! in M6b) implements [`Template`] to add a Disciplines map, a Blood Pool
//! track, a Generation background, and its chargen contributions; the ruleset
//! folds every registered template's [`Template::extend_schema`] over the base
//! builder and lists their ids as the sheet's `template` enum options.

use mush_ruleset::schema::{
    DamageLevelDef, FieldDef, FieldType, SchemaError, SectionLayout, SheetSchema,
    SheetSchemaBuilder, SheetSection, TrackSize,
};
use smol_str::SmolStr;

/// The current base-schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// The nine attributes: `(id, label, category)`.
pub const ATTRIBUTES: &[(&str, &str, &str)] = &[
    ("strength", "Strength", "physical"),
    ("dexterity", "Dexterity", "physical"),
    ("stamina", "Stamina", "physical"),
    ("charisma", "Charisma", "social"),
    ("manipulation", "Manipulation", "social"),
    ("appearance", "Appearance", "social"),
    ("perception", "Perception", "mental"),
    ("intelligence", "Intelligence", "mental"),
    ("wits", "Wits", "mental"),
];

/// The ten Talents: `(id, label)`.
pub const TALENTS: &[(&str, &str)] = &[
    ("alertness", "Alertness"),
    ("athletics", "Athletics"),
    ("awareness", "Awareness"),
    ("brawl", "Brawl"),
    ("empathy", "Empathy"),
    ("expression", "Expression"),
    ("intimidation", "Intimidation"),
    ("leadership", "Leadership"),
    ("streetwise", "Streetwise"),
    ("subterfuge", "Subterfuge"),
];

/// The ten Skills: `(id, label)`.
pub const SKILLS: &[(&str, &str)] = &[
    ("animal_ken", "Animal Ken"),
    ("crafts", "Crafts"),
    ("drive", "Drive"),
    ("etiquette", "Etiquette"),
    ("firearms", "Firearms"),
    ("larceny", "Larceny"),
    ("melee", "Melee"),
    ("performance", "Performance"),
    ("stealth", "Stealth"),
    ("survival", "Survival"),
];

/// The ten Knowledges: `(id, label)`.
pub const KNOWLEDGES: &[(&str, &str)] = &[
    ("academics", "Academics"),
    ("computer", "Computer"),
    ("finance", "Finance"),
    ("investigation", "Investigation"),
    ("law", "Law"),
    ("medicine", "Medicine"),
    ("occult", "Occult"),
    ("politics", "Politics"),
    ("science", "Science"),
    ("technology", "Technology"),
];

/// The three Virtues: `(id, label)`.
pub const VIRTUES: &[(&str, &str)] = &[
    ("conscience", "Conscience"),
    ("self_control", "Self-Control"),
    ("courage", "Courage"),
];

/// The sheet field id of the Health track.
pub const HEALTH_FIELD: &str = "health";

/// The seven fixed Health levels with their dice-pool wound penalties (crib).
/// The Incapacitated level's nominal penalty mirrors Crippled's; genuine
/// incapacitation is detected as a fully-filled track, not from this number.
#[must_use]
pub fn health_levels() -> Vec<DamageLevelDef> {
    vec![
        DamageLevelDef::new("bruised", "Bruised", 0),
        DamageLevelDef::new("hurt", "Hurt", -1),
        DamageLevelDef::new("injured", "Injured", -1),
        DamageLevelDef::new("wounded", "Wounded", -2),
        DamageLevelDef::new("mauled", "Mauled", -2),
        DamageLevelDef::new("crippled", "Crippled", -5),
        DamageLevelDef::new("incapacitated", "Incapacitated", -5),
    ]
}

/// A base-sheet template: the seam a splat sub-module extends (M6b). Object
/// safe so the ruleset can hold `Box<dyn Template>`s.
pub trait Template: Send + Sync {
    /// The template id (the `template` enum value, e.g. `"mortal"`, `"vampire"`).
    fn id(&self) -> &'static str;

    /// The human name ("Mortal", "Vampire").
    fn display_name(&self) -> &str;

    /// Extend the base builder with this template's sections/fields, leaving
    /// the base untouched. The base [`Mortal`] returns `b` unchanged.
    ///
    /// # Errors
    ///
    /// Returns a [`SchemaError`] if a section/field id collides with the base
    /// or another template.
    fn extend_schema(&self, b: SheetSchemaBuilder) -> Result<SheetSchemaBuilder, SchemaError>;
}

/// The base template — adds nothing. It proves the fold is a clean no-op and
/// occupies the `mortal` template slot every game ships with.
#[derive(Debug, Default, Clone, Copy)]
pub struct Mortal;

impl Template for Mortal {
    fn id(&self) -> &'static str {
        "mortal"
    }

    fn display_name(&self) -> &str {
        "Mortal"
    }

    fn extend_schema(&self, b: SheetSchemaBuilder) -> Result<SheetSchemaBuilder, SchemaError> {
        Ok(b)
    }
}

/// Build the base oWoD builder (before any template extension). The `template`
/// enum lists `template_ids`; the caller folds each template's
/// [`Template::extend_schema`] over the result and calls `build`.
#[must_use]
pub fn base_builder(ruleset: &str, template_ids: &[SmolStr]) -> SheetSchemaBuilder {
    let ability_dot =
        |id: &str, label: &str, group: &str| FieldDef::dots(id, label, 0, 5).category(group);

    let mut builder = SheetSchemaBuilder::new(ruleset, SCHEMA_VERSION);

    // Attributes (3×3 grid, everyone starts at 1).
    let mut attrs = SheetSection::new("attributes", "Attributes", SectionLayout::grid(3, 3));
    for (id, label, cat) in ATTRIBUTES {
        attrs = attrs.field(FieldDef::dots(id, *label, 1, 5).category(cat));
    }
    builder = builder.section(attrs).expect("attributes section");

    // Abilities: three groups, each its own section, category = group.
    let mut talents = SheetSection::new("talents", "Talents", SectionLayout::List);
    for (id, label) in TALENTS {
        talents = talents.field(ability_dot(id, label, "talents"));
    }
    let mut skills = SheetSection::new("skills", "Skills", SectionLayout::List);
    for (id, label) in SKILLS {
        skills = skills.field(ability_dot(id, label, "skills"));
    }
    let mut knowledges = SheetSection::new("knowledges", "Knowledges", SectionLayout::List);
    for (id, label) in KNOWLEDGES {
        knowledges = knowledges.field(ability_dot(id, label, "knowledges"));
    }
    builder = builder
        .section(talents)
        .and_then(|b| b.section(skills))
        .and_then(|b| b.section(knowledges))
        .expect("ability sections");

    // Specialties: one map of ability-id → list of free-text specialties.
    let specialties =
        SheetSection::new("specialties", "Specialties", SectionLayout::Map).field(FieldDef::map(
            "specialties",
            "Specialties",
            FieldType::List {
                of: Box::new(FieldType::Text { max_len: 40 }),
            },
        ));
    builder = builder.section(specialties).expect("specialties section");

    // Backgrounds: an open name → dots map.
    let backgrounds =
        SheetSection::new("backgrounds", "Backgrounds", SectionLayout::Map).field(FieldDef::map(
            "backgrounds",
            "Backgrounds",
            FieldType::Dots { min: 0, max: 5 },
        ));
    builder = builder.section(backgrounds).expect("backgrounds section");

    // Virtues.
    let mut virtues = SheetSection::new("virtues", "Virtues", SectionLayout::List);
    for (id, label) in VIRTUES {
        virtues = virtues.field(FieldDef::dots(id, *label, 1, 5).category("virtue"));
    }
    builder = builder.section(virtues).expect("virtues section");

    // Willpower (permanent + temporary pool) and Experience.
    let advantages = SheetSection::new("advantages", "Advantages", SectionLayout::List)
        .field(FieldDef::number("willpower", "Willpower", 1, 10))
        .field(FieldDef::number(
            "willpower_temp",
            "Willpower (temporary)",
            0,
            10,
        ))
        .field(FieldDef::number(
            "experience",
            "Experience (current)",
            0,
            1_000_000,
        ))
        .field(FieldDef::number(
            "total_xp",
            "Experience (earned)",
            0,
            1_000_000,
        ))
        .field(FieldDef::list(
            "xp_log",
            "Experience log",
            FieldType::Text { max_len: 120 },
        ));
    builder = builder.section(advantages).expect("advantages section");

    // Merits & Flaws (name → points).
    let meritsflaws = SheetSection::new("meritsflaws", "Merits & Flaws", SectionLayout::Map)
        .field(FieldDef::map(
            "merits",
            "Merits",
            FieldType::Number { min: 0, max: 7 },
        ))
        .field(FieldDef::map(
            "flaws",
            "Flaws",
            FieldType::Number { min: 0, max: 7 },
        ));
    builder = builder.section(meritsflaws).expect("meritsflaws section");

    // Bio: template + concept + nature/demeanor archetypes.
    let bio = SheetSection::new("bio", "Bio", SectionLayout::List)
        .field(FieldDef::enumeration(
            "template",
            "Template",
            template_ids.to_vec(),
        ))
        .field(FieldDef::text("concept", "Concept", 60))
        .field(FieldDef::text("nature", "Nature", 40))
        .field(FieldDef::text("demeanor", "Demeanor", 40));
    builder = builder.section(bio).expect("bio section");

    // Chargen bookkeeping: the staff-only record of the priority assignment
    // the sheet was built against (reboot-safe budget basis + approval
    // re-validation). Keys: physical/social/mental, talents/skills/knowledges
    // → assigned tier dots.
    let chargen = SheetSection::new("chargen", "Chargen", SectionLayout::Map).field(
        FieldDef::map(
            crate::chargen::CHARGEN_POOLS_FIELD,
            "Priority pools",
            FieldType::Number { min: 0, max: 40 },
        )
        .staff_only(),
    );
    builder = builder.section(chargen).expect("chargen section");

    // Health track: fixed 7 levels.
    let health = SheetSection::new("health", "Health", SectionLayout::Tracks).field(
        FieldDef::track(HEALTH_FIELD, "Health", TrackSize::Fixed(7), health_levels()),
    );
    builder.section(health).expect("health section")
}

/// Fold `templates` over the base builder and build the composed schema. The
/// `template` enum is seeded with every template id.
///
/// This folds **all** templates into one sheet — the M6a single-schema shape,
/// still used to assert the composition merges cleanly. The live multi-template
/// dispatch (M6b) instead builds one schema *per* template with
/// [`build_template_schema`], so a character's active sheet carries only their
/// own splat's sections.
///
/// # Panics
///
/// Panics if a template's extension collides with the base or another template
/// — a programming error, surfaced at construction.
#[must_use]
pub fn build_schema(ruleset: &str, templates: &[Box<dyn Template>]) -> SheetSchema {
    let ids: Vec<SmolStr> = templates.iter().map(|t| SmolStr::new(t.id())).collect();
    let mut builder = base_builder(ruleset, &ids);
    for t in templates {
        builder = t
            .extend_schema(builder)
            .unwrap_or_else(|e| panic!("template {:?} failed to extend the sheet: {e}", t.id()));
    }
    builder.build()
}

/// Build **one** template's composed schema: the base oWoD sheet (whose
/// `template` enum lists every id in `all_ids`, so a character may switch
/// templates) plus **only** `template`'s own sections.
///
/// This is the per-character "active schema" the M6b multi-template dispatch
/// selects by the sheet's `template` field: a vampire's schema carries
/// Disciplines / Blood Pool / Generation and *not* a mage's Spheres, and a
/// mortal's carries neither. The ruleset holds a map `template-id -> schema`
/// built once at boot with this.
///
/// # Panics
///
/// Panics if the template's extension collides with the base — a programming
/// error surfaced at construction.
#[must_use]
pub fn build_template_schema(
    ruleset: &str,
    all_ids: &[SmolStr],
    template: &dyn Template,
) -> SheetSchema {
    let builder = base_builder(ruleset, all_ids);
    template
        .extend_schema(builder)
        .unwrap_or_else(|e| {
            panic!(
                "template {:?} failed to extend the sheet: {e}",
                template.id()
            )
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_ruleset::{SheetDoc, validate};

    #[test]
    fn base_schema_default_is_valid() {
        let schema = build_schema("wod", &[Box::new(Mortal)]);
        assert!(validate(&schema, &schema.default_data()).is_ok());
        // Every attribute starts at 1, every ability at 0.
        let doc = SheetDoc::new_default(&schema);
        assert_eq!(doc.dots("strength"), Some(1));
        assert_eq!(doc.dots("brawl"), Some(0));
        // 9 attributes, 30 abilities present.
        for (id, _, _) in ATTRIBUTES {
            assert!(schema.field(id).is_some(), "missing {id}");
        }
        assert!(schema.field("template").is_some());
    }

    #[test]
    fn health_track_has_seven_levels_with_penalties() {
        let schema = build_schema("wod", &[Box::new(Mortal)]);
        let doc = SheetDoc::new_default(&schema);
        assert_eq!(doc.track(HEALTH_FIELD).unwrap().len(), 7);
        let levels = health_levels();
        assert_eq!(levels.len(), 7);
        assert_eq!(levels[0].penalty, 0);
        assert_eq!(levels[5].penalty, -5);
    }
}
