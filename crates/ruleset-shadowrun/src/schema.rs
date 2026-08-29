//! The SR5 character-sheet [`SheetSchema`] — one sheet, no splats.
//!
//! Built from the composable `mush-ruleset` schema primitives per the M10b
//! crib. SR5 has no template/splat seam (unlike oWoD), so this is a single
//! flat schema: eight core Attributes plus the Edge/Essence/Magic/Resonance
//! specials, the metatype enum, the active/knowledge/language skill maps and
//! skill-group map, the two derived Condition Monitor **Track**s, the derived
//! Limits / Initiative, the Magic/Resonance section, gear/augmentations with
//! Essence accounting, contacts, nuyen, karma, and qualities.
//!
//! ## Derived values that [`DerivedExpr`] cannot express
//!
//! [`DerivedExpr`] is an integer AST over `+`/`-`/`*` only — no division, no
//! `ceil`. SR5's derived stats need `ceil(x/2)` and `ceil(x/3)`:
//!
//! - **Condition Monitors**: Physical = `8 + ceil(BOD/2)` boxes, Stun =
//!   `8 + ceil(WIL/2)`.
//! - **Limits**: Physical = `ceil((STR·2 + BOD + REA)/3)`, Mental =
//!   `ceil((LOG·2 + INT + WIL)/3)`, Social = `ceil((CHA·2 + WIL + ⌊ESS⌉)/3)`.
//!
//! The chosen pattern (per the assignment's guidance): compute these in a
//! **finalize step** ([`crate::derive::apply_derived`]) and store the results
//! in ordinary **Number fields** ([`PHYS_CM_MAX`], [`STUN_CM_MAX`],
//! [`PHYS_LIMIT`], …). The two CM Track sizes are then
//! `TrackSize::Derived(DerivedExpr::field(PHYS_CM_MAX))` — a plain field
//! reference, which `DerivedExpr` *can* express. The `ceil`/division math
//! lives in `derive.rs`, not in the schema; the box-count fields carry the
//! answer. So the sheet always validates (a Track's length always equals its
//! referenced box-count field), and the engine calls `apply_derived` after
//! any attribute/essence change to keep the stored derived fields fresh.
//!
//! ## Essence encoding
//!
//! There is no float field type, so **Essence is stored in hundredths of a
//! point**: [`ESSENCE`] `= 600` is Essence 6.00, `= 545` is 5.45. Augmentation
//! Essence costs (`augmentations` map values) are likewise in hundredths.
//! [`crate::derive`] exposes `essence_points` / `essence_rounded` helpers.

use mush_ruleset::DerivedExpr;
use mush_ruleset::schema::{
    DamageLevelDef, FieldDef, FieldType, SectionLayout, SheetSchema, SheetSchemaBuilder,
    SheetSection, TrackSize,
};
use smol_str::SmolStr;

use crate::data;

/// The current SR5 sheet-schema version.
pub const SCHEMA_VERSION: u32 = 1;

// --- Load-bearing field ids the engine + derive helpers reference ----------

/// Edge special-attribute rating.
pub const EDGE: &str = "edge";
/// Current spendable Edge this session (refreshes to [`EDGE`]).
pub const EDGE_CURRENT: &str = "edge_current";
/// Essence, stored in hundredths of a point (600 = 6.00).
pub const ESSENCE: &str = "essence";
/// Magic rating (0 unless Awakened).
pub const MAGIC: &str = "magic";
/// Resonance rating (0 unless a technomancer).
pub const RESONANCE: &str = "resonance";

/// Physical Limit (derived, engine-maintained).
pub const PHYS_LIMIT: &str = "physical_limit";
/// Mental Limit (derived).
pub const MENTAL_LIMIT: &str = "mental_limit";
/// Social Limit (derived).
pub const SOCIAL_LIMIT: &str = "social_limit";

/// Initiative base = Reaction + Intuition (derived).
pub const INITIATIVE_BASE: &str = "initiative_base";
/// Initiative dice (1D6 base; more from 'ware/magic).
pub const INITIATIVE_DICE: &str = "initiative_dice";

/// Physical Condition Monitor box count (derived: `8 + ceil(BOD/2)`).
pub const PHYS_CM_MAX: &str = "phys_cm_max";
/// Stun Condition Monitor box count (derived: `8 + ceil(WIL/2)`).
pub const STUN_CM_MAX: &str = "stun_cm_max";
/// The Physical Condition Monitor damage track.
pub const CONDITION_PHYSICAL: &str = "condition_physical";
/// The Stun Condition Monitor damage track.
pub const CONDITION_STUN: &str = "condition_stun";
/// Physical damage overflow past the track (lethal; capped at Body).
pub const PHYSICAL_OVERFLOW: &str = "physical_overflow";

/// The active-skills map (`name -> rating`).
pub const ACTIVE_SKILLS_FIELD: &str = "active_skills";
/// The active-skill specializations map (`name -> [specialization]`).
pub const SKILL_SPECIALIZATIONS_FIELD: &str = "skill_specializations";
/// The skill-groups map (`name -> rating`).
pub const SKILL_GROUPS_FIELD: &str = "skill_groups";
/// The knowledge-skills map (`name -> rating`).
pub const KNOWLEDGE_SKILLS_FIELD: &str = "knowledge_skills";
/// The language-skills map (`name -> rating`).
pub const LANGUAGE_SKILLS_FIELD: &str = "language_skills";

/// The augmentations map (`name -> essence cost in hundredths`).
pub const AUGMENTATIONS_FIELD: &str = "augmentations";

/// The staff-only record of the chargen priority assignment (category ->
/// letter), for reboot-safe budget re-validation on approval.
pub const CHARGEN_PRIORITIES_FIELD: &str = "chargen_priorities";

/// The maximum Condition-Monitor box index the wound-penalty ladder covers.
/// Larger than any reachable CM (max ≈ `8 + ceil(15/2) = 16`); the store's
/// generic wound-penalty helper clamps a shorter track to the last level.
const CM_LADDER_LEN: usize = 24;

/// The SR5 Condition-Monitor wound-penalty ladder: filling `n` boxes imposes a
/// `−⌊n/3⌋` dice-pool modifier (SR5: −1 per three boxes of damage). Mapped
/// one level per box so `SheetStore::wound_penalty` reads the right per-track
/// modifier for free; [`crate::derive::wound_modifier`] sums both tracks.
#[must_use]
pub fn condition_levels() -> Vec<DamageLevelDef> {
    (1..=CM_LADDER_LEN)
        .map(|n| {
            let penalty = -((n / 3) as i8);
            DamageLevelDef::new(&format!("box{n}"), format!("Box {n} ({penalty})"), penalty)
        })
        .collect()
}

/// Build the SR5 sheet schema (`ruleset` = `"shadowrun"`).
#[must_use]
pub fn build_schema(ruleset: &str) -> SheetSchema {
    let metatype_ids: Vec<SmolStr> = data::metatype_ids().into_iter().map(SmolStr::new).collect();
    let awakened_ids: Vec<SmolStr> = data::AWAKENED_TYPES
        .iter()
        .map(|(id, _)| SmolStr::new(id))
        .collect();
    let tradition_ids: Vec<SmolStr> = data::tradition_options()
        .into_iter()
        .map(SmolStr::new)
        .collect();
    let lifestyle_ids: Vec<SmolStr> = data::LIFESTYLES
        .iter()
        .map(|(id, _)| SmolStr::new(id))
        .collect();

    let attr_max = data::ATTR_SCHEMA_MAX;

    // --- Bio -----------------------------------------------------------
    let bio = SheetSection::new("bio", "Bio", SectionLayout::List)
        .field(FieldDef::enumeration("metatype", "Metatype", metatype_ids))
        .field(FieldDef::text("concept", "Concept", 80))
        .field(FieldDef::text("native_language", "Native Language", 40))
        .field(FieldDef::enumeration(
            "lifestyle",
            "Lifestyle",
            lifestyle_ids,
        ));

    // --- Attributes (Physical + Mental) --------------------------------
    let mut attributes = SheetSection::new("attributes", "Attributes", SectionLayout::grid(4, 2));
    for (id, label, cat) in data::CORE_ATTRS {
        attributes = attributes.field(FieldDef::dots(id, *label, 1, attr_max).category(cat));
    }

    // --- Special attributes --------------------------------------------
    let special = SheetSection::new("special", "Special Attributes", SectionLayout::List)
        .field(FieldDef::dots(EDGE, "Edge", 1, 8).category("special"))
        .field(
            FieldDef::number(EDGE_CURRENT, "Edge (available)", 0, 8)
                .with_default(serde_json::json!(1)),
        )
        .field(
            FieldDef::number(ESSENCE, "Essence (×100)", 0, 600)
                .with_default(serde_json::json!(600)),
        )
        .field(FieldDef::number(MAGIC, "Magic", 0, 12).category("special"))
        .field(FieldDef::number(RESONANCE, "Resonance", 0, 12).category("special"));

    // --- Derived limits ------------------------------------------------
    let limits = SheetSection::new("limits", "Limits", SectionLayout::List)
        .field(
            FieldDef::number(PHYS_LIMIT, "Physical Limit", 0, 20)
                .with_default(serde_json::json!(2)),
        )
        .field(
            FieldDef::number(MENTAL_LIMIT, "Mental Limit", 0, 20)
                .with_default(serde_json::json!(2)),
        )
        .field(
            FieldDef::number(SOCIAL_LIMIT, "Social Limit", 0, 20)
                .with_default(serde_json::json!(3)),
        );

    // --- Initiative ----------------------------------------------------
    let initiative = SheetSection::new("initiative", "Initiative", SectionLayout::List)
        .field(
            FieldDef::number(INITIATIVE_BASE, "Initiative (base)", 0, 40)
                .with_default(serde_json::json!(2)),
        )
        .field(
            FieldDef::number(INITIATIVE_DICE, "Initiative Dice", 1, 6)
                .with_default(serde_json::json!(1)),
        );

    // --- Skills --------------------------------------------------------
    let skills = SheetSection::new("skills", "Active Skills", SectionLayout::Map)
        .field(FieldDef::map(
            ACTIVE_SKILLS_FIELD,
            "Active Skills",
            FieldType::Number { min: 0, max: 12 },
        ))
        .field(FieldDef::map(
            SKILL_SPECIALIZATIONS_FIELD,
            "Specializations",
            FieldType::List {
                of: Box::new(FieldType::Text { max_len: 40 }),
            },
        ))
        .field(FieldDef::map(
            SKILL_GROUPS_FIELD,
            "Skill Groups",
            FieldType::Number { min: 0, max: 12 },
        ));

    let knowledge = SheetSection::new("knowledge", "Knowledge & Language", SectionLayout::Map)
        .field(FieldDef::map(
            KNOWLEDGE_SKILLS_FIELD,
            "Knowledge Skills",
            FieldType::Number { min: 0, max: 12 },
        ))
        .field(FieldDef::map(
            LANGUAGE_SKILLS_FIELD,
            "Language Skills",
            FieldType::Number { min: 0, max: 12 },
        ));

    // --- Magic / Resonance ---------------------------------------------
    let magic_section = SheetSection::new("magic", "Magic & Resonance", SectionLayout::List)
        .field(FieldDef::enumeration(
            "awakened_type",
            "Awakened Type",
            awakened_ids,
        ))
        .field(FieldDef::enumeration(
            "tradition",
            "Tradition",
            tradition_ids,
        ))
        .field(FieldDef::number("initiate_grade", "Initiate Grade", 0, 12))
        .field(FieldDef::number(
            "submersion_grade",
            "Submersion Grade",
            0,
            12,
        ))
        .field(FieldDef::number("power_points", "Power Points (×1)", 0, 12))
        .field(FieldDef::list(
            "spells",
            "Spells",
            FieldType::Text { max_len: 60 },
        ))
        .field(FieldDef::list(
            "rituals",
            "Rituals",
            FieldType::Text { max_len: 60 },
        ))
        .field(FieldDef::map(
            "adept_powers",
            "Adept Powers",
            FieldType::Number { min: 0, max: 12 },
        ))
        .field(FieldDef::list(
            "complex_forms",
            "Complex Forms",
            FieldType::Text { max_len: 60 },
        ));

    // --- Gear & augmentations (Essence accounting) ---------------------
    let gear = SheetSection::new("gear", "Gear & Augmentations", SectionLayout::Map)
        .field(FieldDef::map(
            AUGMENTATIONS_FIELD,
            "Augmentations (Essence ×100)",
            FieldType::Number { min: 0, max: 600 },
        ))
        .field(FieldDef::map(
            "weapons",
            "Weapons (Accuracy)",
            FieldType::Number { min: 0, max: 12 },
        ))
        .field(FieldDef::map(
            "armor",
            "Armor (Rating)",
            FieldType::Number { min: 0, max: 40 },
        ))
        .field(FieldDef::list(
            "gear",
            "Gear",
            FieldType::Text { max_len: 80 },
        ));

    // --- Economy / social ----------------------------------------------
    let economy = SheetSection::new("economy", "Nuyen, Contacts & Karma", SectionLayout::Map)
        .field(FieldDef::number("nuyen", "Nuyen", 0, 50_000_000))
        .field(FieldDef::number("karma", "Karma (available)", 0, 1_000_000))
        .field(FieldDef::number(
            "karma_total",
            "Karma (total earned)",
            0,
            1_000_000,
        ))
        .field(FieldDef::list(
            "karma_log",
            "Karma History",
            FieldType::Text { max_len: 120 },
        ))
        .field(FieldDef::map(
            "contacts_connection",
            "Contact Connection",
            FieldType::Number { min: 1, max: 12 },
        ))
        .field(FieldDef::map(
            "contacts_loyalty",
            "Contact Loyalty",
            FieldType::Number { min: 1, max: 6 },
        ))
        .field(FieldDef::map(
            "positive_qualities",
            "Positive Qualities (Karma)",
            FieldType::Number { min: 0, max: 40 },
        ))
        .field(FieldDef::map(
            "negative_qualities",
            "Negative Qualities (Karma)",
            FieldType::Number { min: 0, max: 40 },
        ));

    // --- Chargen bookkeeping (staff-only) ------------------------------
    let chargen = SheetSection::new("chargen", "Chargen", SectionLayout::Map).field(
        FieldDef::map(
            CHARGEN_PRIORITIES_FIELD,
            "Priority assignment",
            FieldType::Text { max_len: 1 },
        )
        .staff_only(),
    );

    // --- Condition monitors --------------------------------------------
    let condition = SheetSection::new("condition", "Condition Monitors", SectionLayout::Tracks)
        .field(
            FieldDef::number(PHYS_CM_MAX, "Physical CM boxes", 8, 20)
                .with_default(serde_json::json!(9)),
        )
        .field(
            FieldDef::number(STUN_CM_MAX, "Stun CM boxes", 8, 20)
                .with_default(serde_json::json!(9)),
        )
        .field(FieldDef::number(
            PHYSICAL_OVERFLOW,
            "Physical Overflow",
            0,
            20,
        ))
        .field(FieldDef::track(
            CONDITION_PHYSICAL,
            "Physical Damage",
            TrackSize::Derived(DerivedExpr::field(PHYS_CM_MAX)),
            condition_levels(),
        ))
        .field(FieldDef::track(
            CONDITION_STUN,
            "Stun Damage",
            TrackSize::Derived(DerivedExpr::field(STUN_CM_MAX)),
            condition_levels(),
        ));

    SheetSchemaBuilder::new(ruleset, SCHEMA_VERSION)
        .section(bio)
        .and_then(|b| b.section(attributes))
        .and_then(|b| b.section(special))
        .and_then(|b| b.section(limits))
        .and_then(|b| b.section(initiative))
        .and_then(|b| b.section(skills))
        .and_then(|b| b.section(knowledge))
        .and_then(|b| b.section(magic_section))
        .and_then(|b| b.section(gear))
        .and_then(|b| b.section(economy))
        .and_then(|b| b.section(chargen))
        .and_then(|b| b.section(condition))
        .expect("SR5 sheet schema must build")
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_ruleset::{SheetDoc, validate};

    #[test]
    fn default_sheet_is_valid() {
        let schema = build_schema("shadowrun");
        assert!(validate(&schema, &schema.default_data()).is_ok());
        let doc = SheetDoc::new_default(&schema);
        // Attributes default to 1; specials as documented.
        assert_eq!(doc.dots("body"), Some(1));
        assert_eq!(doc.number(ESSENCE), Some(600));
        assert_eq!(doc.dots(EDGE), Some(1));
        // Both CM tracks default to 9 boxes (BOD/WIL = 1 → 8 + ceil(1/2) = 9).
        assert_eq!(doc.track(CONDITION_PHYSICAL).unwrap().len(), 9);
        assert_eq!(doc.track(CONDITION_STUN).unwrap().len(), 9);
    }

    #[test]
    fn every_core_attribute_and_derived_field_present() {
        let schema = build_schema("shadowrun");
        for (id, _, _) in data::CORE_ATTRS {
            assert!(schema.field(id).is_some(), "missing attribute {id}");
        }
        for id in [
            PHYS_LIMIT,
            MENTAL_LIMIT,
            SOCIAL_LIMIT,
            INITIATIVE_BASE,
            PHYS_CM_MAX,
            STUN_CM_MAX,
            ACTIVE_SKILLS_FIELD,
            SKILL_GROUPS_FIELD,
            AUGMENTATIONS_FIELD,
        ] {
            assert!(schema.field(id).is_some(), "missing derived/field {id}");
        }
    }

    #[test]
    fn condition_ladder_is_minus_one_per_three_boxes() {
        let levels = condition_levels();
        assert_eq!(levels[0].penalty, 0); // box 1
        assert_eq!(levels[1].penalty, 0); // box 2
        assert_eq!(levels[2].penalty, -1); // box 3
        assert_eq!(levels[5].penalty, -2); // box 6
        assert_eq!(levels[8].penalty, -3); // box 9
    }

    #[test]
    fn chargen_priorities_is_staff_only() {
        let schema = build_schema("shadowrun");
        assert!(schema.field(CHARGEN_PRIORITIES_FIELD).unwrap().staff_only);
    }
}
