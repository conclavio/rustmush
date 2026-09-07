//! Sheet-data validation against a [`SheetSchema`].
//!
//! [`validate`] checks a sheet's `data` document field-by-field: types, dot
//! and number ranges, enum membership, text length, track slot counts
//! (resolving derived sizes), and — recursively — list elements and map
//! values. It collects **every** problem into a `Vec<SheetError>` rather than
//! failing on the first, so `@sheet/set` and chargen can report all of them
//! at once. Unknown top-level keys are flagged too (typo-catching).

use serde::Serialize;
use smol_str::SmolStr;

use crate::schema::{FieldType, SheetSchema, TrackSize};

/// One validation problem. `field` is a path (`"strength"`,
/// `"disciplines/Dominate"`, `"specialties[2]"`).
///
/// `Serialize`-only: the `WrongType` variant carries `&'static str` shape
/// names, which cannot be `Deserialize`d — errors travel outward (logs, web
/// error payloads), never back in.
#[derive(Debug, Clone, PartialEq, Serialize, thiserror::Error)]
pub enum SheetError {
    /// A `data` key that no schema field claims.
    #[error("{field}: unknown field (not in the schema)")]
    UnknownField {
        /// The offending path.
        field: String,
    },
    /// A value of the wrong JSON shape for its field type.
    #[error("{field}: expected {expected}, got {got}")]
    WrongType {
        /// The field path.
        field: String,
        /// The expected shape ("integer", "string", "array", "object").
        expected: &'static str,
        /// The actual JSON shape.
        got: &'static str,
    },
    /// A dots/number value outside its range.
    #[error("{field}: {got} is out of range {min}..={max}")]
    OutOfRange {
        /// The field path.
        field: String,
        /// Minimum allowed.
        min: i64,
        /// Maximum allowed.
        max: i64,
        /// The value seen.
        got: i64,
    },
    /// A text value longer than the field's `max_len`.
    #[error("{field}: text too long ({got} > {max_len} chars)")]
    TooLong {
        /// The field path.
        field: String,
        /// The maximum length.
        max_len: u32,
        /// The length seen.
        got: u32,
    },
    /// An enum value not in the option set.
    #[error("{field}: {got:?} is not one of {options:?}")]
    NotAnOption {
        /// The field path.
        field: String,
        /// The value seen.
        got: String,
        /// The allowed options.
        options: Vec<SmolStr>,
    },
    /// A track whose slot count does not match its (possibly derived) size.
    #[error("{field}: track has {got} slots, expected {expected}")]
    TrackSlots {
        /// The field path.
        field: String,
        /// The resolved slot count.
        expected: usize,
        /// The slot count seen.
        got: usize,
    },
    /// A track slot that is neither empty (`null`) nor a mark string.
    #[error("{field}: slot {index} is not a valid mark (null or string)")]
    BadMark {
        /// The field path.
        field: String,
        /// The bad slot index.
        index: usize,
    },
    /// A whole document larger than
    /// [`MAX_SHEET_DOC_BYTES`](crate::store::MAX_SHEET_DOC_BYTES) once
    /// serialized (interface plan I13): refused before the write, so no sheet
    /// keyframe or delta can ever exceed the client-side frame bound.
    #[error("sheet document is {bytes} bytes; the maximum is {max}")]
    TooLarge {
        /// The serialized size seen.
        bytes: usize,
        /// The cap.
        max: usize,
    },
    /// A structural/migration error carrying a free-form message.
    #[error("{0}")]
    Msg(String),
}

impl SheetError {
    /// Free-form error shorthand.
    #[must_use]
    pub fn msg(m: impl Into<String>) -> SheetError {
        SheetError::Msg(m.into())
    }
}

/// The JSON shape name for error messages.
fn shape(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Validate `data` against `schema`. Returns every problem found (empty
/// `Err` vec is impossible — an empty problem list is `Ok`).
///
/// # Errors
///
/// Returns the collected [`SheetError`]s if any field is malformed, or a
/// single [`SheetError::WrongType`] if `data` is not a JSON object.
pub fn validate(schema: &SheetSchema, data: &serde_json::Value) -> Result<(), Vec<SheetError>> {
    let mut errors = Vec::new();
    let Some(obj) = data.as_object() else {
        return Err(vec![SheetError::WrongType {
            field: "<sheet>".to_string(),
            expected: "object",
            got: shape(data),
        }]);
    };

    // Unknown top-level keys.
    for key in obj.keys() {
        if schema.field(key).is_none() {
            errors.push(SheetError::UnknownField { field: key.clone() });
        }
    }

    // Present fields (missing fields are allowed — they read as defaults).
    for (_, f) in schema.fields() {
        let Some(value) = obj.get(f.id.as_str()) else {
            continue;
        };
        match &f.ty {
            FieldType::Track { size, .. } => {
                validate_track(&f.id, size, data, value, &mut errors);
            }
            other => validate_value(f.id.as_str(), other, value, &mut errors),
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Validate a track: the array length must equal the resolved size and each
/// slot must be `null` or a string.
fn validate_track(
    field: &str,
    size: &TrackSize,
    whole_data: &serde_json::Value,
    value: &serde_json::Value,
    errors: &mut Vec<SheetError>,
) {
    let Some(arr) = value.as_array() else {
        errors.push(SheetError::WrongType {
            field: field.to_string(),
            expected: "array",
            got: shape(value),
        });
        return;
    };
    let expected = size.resolve(whole_data);
    if arr.len() != expected {
        errors.push(SheetError::TrackSlots {
            field: field.to_string(),
            expected,
            got: arr.len(),
        });
    }
    for (i, slot) in arr.iter().enumerate() {
        if !(slot.is_null() || slot.is_string()) {
            errors.push(SheetError::BadMark {
                field: field.to_string(),
                index: i,
            });
        }
    }
}

/// Validate a non-track value against `ty` (recurses into list/map).
fn validate_value(
    path: &str,
    ty: &FieldType,
    value: &serde_json::Value,
    errors: &mut Vec<SheetError>,
) {
    match ty {
        FieldType::Dots { min, max } => {
            check_int(path, i64::from(*min), i64::from(*max), value, errors);
        }
        FieldType::Number { min, max } => {
            check_int(path, *min, *max, value, errors);
        }
        FieldType::Text { max_len } => match value.as_str() {
            Some(s) => {
                let len = s.chars().count();
                if len > *max_len as usize {
                    errors.push(SheetError::TooLong {
                        field: path.to_string(),
                        max_len: *max_len,
                        got: u32::try_from(len).unwrap_or(u32::MAX),
                    });
                }
            }
            None => wrong_type(path, "string", value, errors),
        },
        FieldType::Enum { options } => match value.as_str() {
            Some(s) => {
                if !options.iter().any(|o| o == s) {
                    errors.push(SheetError::NotAnOption {
                        field: path.to_string(),
                        got: s.to_string(),
                        options: options.clone(),
                    });
                }
            }
            None => wrong_type(path, "string", value, errors),
        },
        FieldType::List { of } => match value.as_array() {
            Some(arr) => {
                for (i, el) in arr.iter().enumerate() {
                    validate_value(&format!("{path}[{i}]"), of, el, errors);
                }
            }
            None => wrong_type(path, "array", value, errors),
        },
        FieldType::Map { value: vty } => match value.as_object() {
            Some(obj) => {
                for (k, v) in obj {
                    validate_value(&format!("{path}/{k}"), vty, v, errors);
                }
            }
            None => wrong_type(path, "object", value, errors),
        },
        // Nested tracks are not supported (tracks live at top level, where
        // their derived size can see sibling fields); treat as a plain array
        // of marks with no size check.
        FieldType::Track { .. } => match value.as_array() {
            Some(arr) => {
                for (i, slot) in arr.iter().enumerate() {
                    if !(slot.is_null() || slot.is_string()) {
                        errors.push(SheetError::BadMark {
                            field: path.to_string(),
                            index: i,
                        });
                    }
                }
            }
            None => wrong_type(path, "array", value, errors),
        },
    }
}

fn check_int(
    path: &str,
    min: i64,
    max: i64,
    value: &serde_json::Value,
    errors: &mut Vec<SheetError>,
) {
    match value.as_i64() {
        Some(n) => {
            if n < min || n > max {
                errors.push(SheetError::OutOfRange {
                    field: path.to_string(),
                    min,
                    max,
                    got: n,
                });
            }
        }
        None => wrong_type(path, "integer", value, errors),
    }
}

fn wrong_type(
    path: &str,
    expected: &'static str,
    value: &serde_json::Value,
    errors: &mut Vec<SheetError>,
) {
    errors.push(SheetError::WrongType {
        field: path.to_string(),
        expected,
        got: shape(value),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{
        DamageLevelDef, DerivedExpr, FieldDef, FieldType, SectionLayout, SheetSchemaBuilder,
        SheetSection,
    };

    fn schema() -> SheetSchema {
        SheetSchemaBuilder::new("toy", 1)
            .section(
                SheetSection::new("core", "Core", SectionLayout::List)
                    .field(FieldDef::dots("strength", "Strength", 1, 5))
                    .field(FieldDef::number("willpower", "Willpower", 1, 10))
                    .field(FieldDef::text("concept", "Concept", 8))
                    .field(FieldDef::enumeration(
                        "template",
                        "Template",
                        vec!["mortal".into(), "vampire".into()],
                    ))
                    .field(FieldDef::map(
                        "backgrounds",
                        "Backgrounds",
                        FieldType::Dots { min: 0, max: 5 },
                    ))
                    .field(FieldDef::list(
                        "specialties",
                        "Specialties",
                        FieldType::Text { max_len: 20 },
                    )),
            )
            .unwrap()
            .section(
                SheetSection::new("health", "Health", SectionLayout::Tracks).field(
                    FieldDef::track(
                        "health",
                        "Health",
                        TrackSize::Derived(
                            DerivedExpr::field("strength").add(DerivedExpr::constant(2)),
                        ),
                        vec![DamageLevelDef::new("bruised", "Bruised", 0)],
                    ),
                ),
            )
            .unwrap()
            .build()
    }

    #[test]
    fn default_data_is_valid() {
        let s = schema();
        assert!(validate(&s, &s.default_data()).is_ok());
    }

    #[test]
    fn dots_out_of_range() {
        let s = schema();
        let mut data = s.default_data();
        data["strength"] = serde_json::json!(9);
        let errs = validate(&s, &data).unwrap_err();
        assert!(matches!(errs[0], SheetError::OutOfRange { got: 9, .. }));
    }

    #[test]
    fn enum_membership_and_text_len() {
        let s = schema();
        let mut data = s.default_data();
        data["template"] = serde_json::json!("werewolf");
        data["concept"] = serde_json::json!("way too long a concept");
        let errs = validate(&s, &data).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, SheetError::NotAnOption { .. }))
        );
        assert!(errs.iter().any(|e| matches!(e, SheetError::TooLong { .. })));
    }

    #[test]
    fn track_slot_count_follows_derived_size() {
        let s = schema();
        let mut data = s.default_data();
        // strength defaults to 1 → health size 3. Make strength 4 → size 6,
        // but leave the 3-slot array: mismatch.
        data["strength"] = serde_json::json!(4);
        let errs = validate(&s, &data).unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            SheetError::TrackSlots {
                expected: 6,
                got: 3,
                ..
            }
        )));
    }

    #[test]
    fn nested_map_and_list_validated() {
        let s = schema();
        let mut data = s.default_data();
        data["backgrounds"] = serde_json::json!({ "Resources": 9 }); // > 5
        data["specialties"] =
            serde_json::json!(["ok", "this specialty text is definitely too long"]);
        let errs = validate(&s, &data).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, SheetError::OutOfRange { .. }))
        );
        assert!(errs.iter().any(|e| matches!(e, SheetError::TooLong { .. })));
    }

    #[test]
    fn unknown_field_flagged() {
        let s = schema();
        let mut data = s.default_data();
        data["bogus"] = serde_json::json!(1);
        let errs = validate(&s, &data).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e, SheetError::UnknownField { .. }))
        );
    }

    #[test]
    fn wrong_type_reported() {
        let s = schema();
        let mut data = s.default_data();
        data["strength"] = serde_json::json!("three");
        let errs = validate(&s, &data).unwrap_err();
        assert!(matches!(
            errs[0],
            SheetError::WrongType {
                expected: "integer",
                ..
            }
        ));
    }
}
