//! The stored sheet document ([`SheetDoc`]) and its approval state, plus
//! ergonomic typed getters/setters over the JSON `data`.
//!
//! One `SheetDoc` is a single versioned JSON blob per (object, ruleset),
//! stored as `AttrValue::Json` under `sheet:<ruleset>` (treaty ruling 7).
//! Keeping the whole sheet in one document keeps reads/writes atomic, makes
//! migration a pure `Value -> Value` function, and serializes trivially to
//! the web panel.

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::RulesetId;
use crate::schema::SheetSchema;

/// A jobs-queue id (a chargen/XP approval ticket). Kept as a bare `u64` so
/// this crate stays free of a `mush-jobs` dependency; the jobs module uses
/// the same width.
pub type JobId = u64;

/// Where a sheet sits in the approval workflow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    /// Freshly created / in chargen; not yet submitted.
    Unapproved,
    /// Submitted; a chargen approval job is open.
    Pending(JobId),
    /// Approved by staff.
    Approved {
        /// The approving staffer.
        by: ObjId,
        /// IC/OOC timestamp of approval (unix seconds).
        at: i64,
    },
}

impl ApprovalState {
    /// A short human label ("unapproved" / "pending #12" / "approved").
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            ApprovalState::Unapproved => "unapproved".to_string(),
            ApprovalState::Pending(job) => format!("pending (job #{job})"),
            ApprovalState::Approved { .. } => "approved".to_string(),
        }
    }

    /// Whether the sheet is approved.
    #[must_use]
    pub fn is_approved(&self) -> bool {
        matches!(self, ApprovalState::Approved { .. })
    }
}

/// A character sheet: versioned JSON data plus its approval state, tagged
/// with the ruleset that owns it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SheetDoc {
    /// The owning ruleset id.
    pub ruleset: RulesetId,
    /// The schema version `data` conforms to.
    pub schema_version: u32,
    /// The sheet data (a JSON object keyed by field id), validated against
    /// the [`SheetSchema`] on write.
    pub data: serde_json::Value,
    /// Chargen approval state.
    pub approved: ApprovalState,
}

impl SheetDoc {
    /// A fresh, unapproved sheet filled from the schema's field defaults.
    #[must_use]
    pub fn new_default(schema: &SheetSchema) -> SheetDoc {
        SheetDoc {
            ruleset: schema.ruleset.clone(),
            schema_version: schema.version,
            data: schema.default_data(),
            approved: ApprovalState::Unapproved,
        }
    }

    /// The `data` object, if it is an object (it always is for a valid
    /// sheet).
    #[must_use]
    pub fn as_object(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.data.as_object()
    }

    /// A raw field value.
    #[must_use]
    pub fn get(&self, field: &str) -> Option<&serde_json::Value> {
        self.data.get(field)
    }

    /// Set a raw field value (no validation — validate via
    /// [`SheetStore::put`](crate::store::SheetStore::put) before persisting).
    /// Creates the `data` object if the doc was somehow scalar.
    pub fn set(&mut self, field: &str, value: serde_json::Value) {
        if !self.data.is_object() {
            self.data = serde_json::Value::Object(serde_json::Map::new());
        }
        if let Some(obj) = self.data.as_object_mut() {
            obj.insert(field.to_string(), value);
        }
    }

    // ------------------------------------------------------------------
    // Typed convenience getters
    // ------------------------------------------------------------------

    /// A numeric field's value (works for Dots and Number).
    #[must_use]
    pub fn number(&self, field: &str) -> Option<i64> {
        self.data.get(field).and_then(serde_json::Value::as_i64)
    }

    /// A dots field's value as `u8` (saturating; negatives clamp to 0).
    #[must_use]
    pub fn dots(&self, field: &str) -> Option<u8> {
        self.number(field)
            .map(|n| n.clamp(0, i64::from(u8::MAX)) as u8)
    }

    /// A text/enum field's value.
    #[must_use]
    pub fn text(&self, field: &str) -> Option<&str> {
        self.data.get(field).and_then(serde_json::Value::as_str)
    }

    /// A track field's marks (`None` = empty slot, `Some(mark)` = a mark
    /// string), or `None` if the field is missing/not an array.
    #[must_use]
    pub fn track(&self, field: &str) -> Option<Vec<Option<SmolStr>>> {
        let arr = self.data.get(field)?.as_array()?;
        Some(arr.iter().map(|v| v.as_str().map(SmolStr::new)).collect())
    }

    /// A map field's entries.
    #[must_use]
    pub fn map(&self, field: &str) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.data.get(field).and_then(serde_json::Value::as_object)
    }

    /// A list field's elements.
    #[must_use]
    pub fn list(&self, field: &str) -> Option<&Vec<serde_json::Value>> {
        self.data.get(field).and_then(serde_json::Value::as_array)
    }

    // ------------------------------------------------------------------
    // Typed convenience setters (raw — validate on put)
    // ------------------------------------------------------------------

    /// Set a numeric field.
    pub fn set_number(&mut self, field: &str, value: i64) {
        self.set(field, serde_json::json!(value));
    }

    /// Set a text/enum field.
    pub fn set_text(&mut self, field: &str, value: impl Into<String>) {
        self.set(field, serde_json::json!(value.into()));
    }

    /// Set a whole track from marks.
    pub fn set_track(&mut self, field: &str, marks: &[Option<SmolStr>]) {
        let arr: Vec<serde_json::Value> = marks
            .iter()
            .map(|m| {
                m.as_ref()
                    .map_or(serde_json::Value::Null, |s| serde_json::json!(s))
            })
            .collect();
        self.set(field, serde_json::Value::Array(arr));
    }

    /// Set one entry of a map field (creating the map if absent).
    pub fn map_set(&mut self, field: &str, key: &str, value: serde_json::Value) {
        let entry = match self.data.get_mut(field) {
            Some(v) if v.is_object() => v,
            _ => {
                self.set(field, serde_json::json!({}));
                self.data.get_mut(field).expect("just inserted")
            }
        };
        if let Some(obj) = entry.as_object_mut() {
            obj.insert(key.to_string(), value);
        }
    }

    /// Remove one entry of a map field. Returns whether it was present.
    pub fn map_remove(&mut self, field: &str, key: &str) -> bool {
        self.data
            .get_mut(field)
            .and_then(serde_json::Value::as_object_mut)
            .is_some_and(|obj| obj.remove(key).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FieldDef, FieldType, SectionLayout, SheetSchemaBuilder, SheetSection};

    fn schema() -> SheetSchema {
        SheetSchemaBuilder::new("toy", 1)
            .section(
                SheetSection::new("a", "A", SectionLayout::List)
                    .field(FieldDef::dots("strength", "Strength", 1, 5))
                    .field(FieldDef::text("concept", "Concept", 40))
                    .field(FieldDef::map(
                        "backgrounds",
                        "Backgrounds",
                        FieldType::Dots { min: 0, max: 5 },
                    )),
            )
            .unwrap()
            .build()
    }

    #[test]
    fn typed_accessors_round_trip() {
        let mut doc = SheetDoc::new_default(&schema());
        assert_eq!(doc.dots("strength"), Some(1));
        assert_eq!(doc.text("concept"), Some(""));

        doc.set_number("strength", 4);
        doc.set_text("concept", "brooding detective");
        doc.map_set("backgrounds", "Resources", serde_json::json!(3));

        assert_eq!(doc.dots("strength"), Some(4));
        assert_eq!(doc.text("concept"), Some("brooding detective"));
        assert_eq!(
            doc.map("backgrounds").unwrap()["Resources"],
            serde_json::json!(3)
        );
        assert!(doc.map_remove("backgrounds", "Resources"));
        assert!(!doc.map_remove("backgrounds", "Resources"));
    }

    #[test]
    fn track_round_trip() {
        let mut doc = SheetDoc::new_default(&schema());
        doc.set_track(
            "wounds",
            &[
                Some(SmolStr::new("bashing")),
                None,
                Some(SmolStr::new("lethal")),
            ],
        );
        let marks = doc.track("wounds").unwrap();
        assert_eq!(marks[0].as_deref(), Some("bashing"));
        assert_eq!(marks[1], None);
        assert_eq!(marks[2].as_deref(), Some("lethal"));
    }

    #[test]
    fn approval_label() {
        assert_eq!(ApprovalState::Unapproved.label(), "unapproved");
        assert_eq!(ApprovalState::Pending(12).label(), "pending (job #12)");
        assert!(
            ApprovalState::Approved {
                by: ObjId(1),
                at: 0
            }
            .is_approved()
        );
    }
}
