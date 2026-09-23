//! The one game sheet's declarative schema (doc 16 §7; 16a Q35 A).
//!
//! Three sections and eight fields, and that is the whole character sheet the
//! 2350 game has:
//!
//! | Section | Fields |
//! |---|---|
//! | `identity` | `name`, `callsign`, `concept` — free text |
//! | `ratings` | `gunnery`, `piloting`, `sensors` — dots 1–5, the [`PilotProfile`](super::PilotProfile) seam |
//! | `standing` | `certifications` (map of text), `reputation` (−5..=5) — staff-only |
//!
//! **What is deliberately absent.** No health track, no money, no experience,
//! and no vitals: doc 16 §3.3 puts unit truth in M32's `TacticalUnit` cells, so
//! a health track on the character sheet would be a second damage authority for
//! the same hit, and doc 16 §3.5's resolution reads exactly one number off the
//! sheet — the relevant rating — through [`PilotProfile`](super::PilotProfile).
//! Money is the ledger's (docs 08/08a) and experience waits for a loop that
//! spends it (16a Q35 A: *nothing else until the economy loop needs it*).
//! [`register_vitals`](mush_ruleset::register_vitals) is therefore **not**
//! called, and the crate registers no `<ruleset>.vitals` feed.
//!
//! # The schema is a literal, not a builder call
//!
//! [`SheetSchemaBuilder`](mush_ruleset::SheetSchemaBuilder) is fallible (a
//! duplicate section or field id), and this crate's root denies
//! `clippy::unwrap_used` and `clippy::panic` — so a builder here would make
//! [`sheet_schema`] fallible and every constructor above it fallible with it,
//! for an error no input can produce. [`SheetSchema`]'s fields are public, so
//! the schema is written as the literal it is and the uniqueness the builder
//! would have checked is checked by the `the_builder_accepts_this_schema` unit test instead — at test time, over the same data.

use mush_ruleset::schema::{FieldDef, FieldType, SectionLayout, SheetSchema, SheetSection};
use smol_str::SmolStr;

/// The sheet's ruleset id: the `sheet:tacsim` attribute and the `tacsim.sheet`
/// panel feed.
///
/// This is **not** the sheet module's [`GameModule::name`](mush_core::GameModule::name)
/// — see [`super::NS`] for why the two differ and why nothing downstream minds.
pub const RULESET: &str = "tacsim";

/// The schema version stored on every document.
pub const SCHEMA_VERSION: u32 = 1;

/// The `identity` section id.
pub const SECTION_IDENTITY: &str = "identity";
/// The `ratings` section id.
pub const SECTION_RATINGS: &str = "ratings";
/// The `standing` section id.
pub const SECTION_STANDING: &str = "standing";

/// The character's full name.
pub const FIELD_NAME: &str = "name";
/// The pilot's callsign — what the NPC layer's capability profile names them
/// by. Nothing in `arena/` or `match_/` reads it (16b cut F.4).
pub const FIELD_CALLSIGN: &str = "callsign";
/// One line of concept.
pub const FIELD_CONCEPT: &str = "concept";

/// Gunnery: doc 16 §3.5's `resolution.error_terms` reads this one.
pub const FIELD_GUNNERY: &str = "gunnery";
/// Piloting.
pub const FIELD_PILOTING: &str = "piloting";
/// Sensors.
pub const FIELD_SENSORS: &str = "sensors";

/// Staff-only: named certifications the character holds, each with a grade.
pub const FIELD_CERTIFICATIONS: &str = "certifications";
/// Staff-only: standing with the Concordat, −5 to 5.
pub const FIELD_REPUTATION: &str = "reputation";

/// The three pilot ratings, in sheet order. Every rating surface — the chargen
/// cost rule, [`PilotProfile`](super::PilotProfile), the `check` ability's
/// `rating` argument — enumerates exactly this.
pub const RATINGS: [&str; 3] = [FIELD_GUNNERY, FIELD_PILOTING, FIELD_SENSORS];

/// The lowest a rating can be. Doc 16 §3.5: *every stock rating is 1*.
pub const RATING_MIN: u8 = 1;
/// The highest a rating can be, at chargen and after.
pub const RATING_MAX: u8 = 5;

/// The longest a `name` may be.
pub const NAME_MAX_LEN: u32 = 64;
/// The longest a `callsign` may be.
pub const CALLSIGN_MAX_LEN: u32 = 24;
/// The longest a `concept` may be.
pub const CONCEPT_MAX_LEN: u32 = 120;

/// The lowest `reputation`.
pub const REPUTATION_MIN: i64 = -5;
/// The highest `reputation`.
pub const REPUTATION_MAX: i64 = 5;

/// The game's sheet schema — `("tacsim", 1)`.
#[must_use]
pub fn sheet_schema() -> SheetSchema {
    SheetSchema {
        ruleset: SmolStr::new(RULESET),
        version: SCHEMA_VERSION,
        sections: vec![identity_section(), ratings_section(), standing_section()],
    }
}

/// `identity` — the character-first facts of doc 10 §17–§19, kept to the three
/// a cockpit and a match record actually print.
fn identity_section() -> SheetSection {
    SheetSection::new(SECTION_IDENTITY, "Identity", SectionLayout::List)
        .field(FieldDef::text(FIELD_NAME, "Name", NAME_MAX_LEN))
        .field(FieldDef::text(FIELD_CALLSIGN, "Callsign", CALLSIGN_MAX_LEN))
        .field(FieldDef::text(FIELD_CONCEPT, "Concept", CONCEPT_MAX_LEN))
}

/// `ratings` — the three dots the resolution mechanic reads, every one of them
/// starting at [`RATING_MIN`] so a sheet that never went through chargen still
/// answers doc 16 §3.5's "every stock rating is 1".
fn ratings_section() -> SheetSection {
    let rating = |id: &str, label: &str| {
        FieldDef::dots(id, label, RATING_MIN, RATING_MAX)
            .category(SECTION_RATINGS)
            .with_default(serde_json::json!(RATING_MIN))
    };
    SheetSection::new(SECTION_RATINGS, "Pilot ratings", SectionLayout::grid(1, 3))
        .field(rating(FIELD_GUNNERY, "Gunnery"))
        .field(rating(FIELD_PILOTING, "Piloting"))
        .field(rating(FIELD_SENSORS, "Sensors"))
}

/// `standing` — staff-written only: a player cannot grant themselves a
/// certification or a reputation, and neither is a chargen budget line.
fn standing_section() -> SheetSection {
    SheetSection::new(SECTION_STANDING, "Standing", SectionLayout::Map)
        .field(
            FieldDef::map(
                FIELD_CERTIFICATIONS,
                "Certifications",
                FieldType::Text { max_len: 32 },
            )
            .staff_only(),
        )
        .field(
            FieldDef::number(
                FIELD_REPUTATION,
                "Reputation",
                REPUTATION_MIN,
                REPUTATION_MAX,
            )
            .with_default(serde_json::json!(0))
            .staff_only(),
        )
}

/// Whether `field` is one of the three pilot ratings (case-insensitively).
#[must_use]
pub fn is_rating(field: &str) -> bool {
    RATINGS.iter().any(|r| r.eq_ignore_ascii_case(field))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_ruleset::SheetSchemaBuilder;

    /// The uniqueness the literal skips the builder for: every section id and
    /// every field id is distinct, checked over the same data the game ships.
    #[test]
    fn the_builder_accepts_this_schema() {
        let mut builder = SheetSchemaBuilder::new(RULESET, SCHEMA_VERSION);
        for section in sheet_schema().sections {
            builder
                .try_section(section)
                .expect("the shipped sections have unique ids and unique fields");
        }
        assert_eq!(builder.build(), sheet_schema());
    }

    #[test]
    fn a_default_document_has_every_rating_at_one() {
        let data = sheet_schema().default_data();
        for rating in RATINGS {
            assert_eq!(
                data.get(rating).and_then(serde_json::Value::as_i64),
                Some(i64::from(RATING_MIN)),
                "{rating} defaults to {RATING_MIN}"
            );
        }
        assert_eq!(data.get(FIELD_CALLSIGN).and_then(|v| v.as_str()), Some(""));
        assert_eq!(
            data.get(FIELD_REPUTATION)
                .and_then(serde_json::Value::as_i64),
            Some(0)
        );
    }

    /// No vitals anywhere: `register_vitals` is not called, and no field claims
    /// a [`VitalRole`](mush_proto::VitalRole) that a publisher could pick up.
    #[test]
    fn the_sheet_declares_no_vital_and_no_track() {
        for (_, field) in sheet_schema().fields() {
            assert!(field.vital.is_none(), "{} claims a vital role", field.id);
            assert!(
                !matches!(field.ty, FieldType::Track { .. }),
                "{} is a track; this sheet has no health",
                field.id
            );
        }
    }

    #[test]
    fn standing_is_staff_only_and_ratings_are_not() {
        let schema = sheet_schema();
        for id in [FIELD_CERTIFICATIONS, FIELD_REPUTATION] {
            assert!(
                schema.field(id).is_some_and(|f| f.staff_only),
                "{id} is staff-only"
            );
        }
        for id in RATINGS {
            assert!(schema.field(id).is_some_and(|f| !f.staff_only));
            assert!(is_rating(id));
        }
        assert!(!is_rating(FIELD_REPUTATION));
    }
}
