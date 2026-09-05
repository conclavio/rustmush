//! The declarative, data-driven sheet schema (design doc 04 §3.3).
//!
//! One [`SheetSchema`] is the single source of truth that drives the `+sheet`
//! terminal render, the web sheet panel, `@sheet/set` validation, chargen,
//! and LLM sheet serialization. It is pure data: sections of typed fields,
//! plus a tiny arithmetic AST ([`DerivedExpr`]) for derived track sizes.
//!
//! ## Composition — the splat seam (crucial for M6b)
//!
//! Schemas **compose**: a base oWoD sheet plus template extensions. A V20
//! sub-module adds a "Disciplines" [`Map`](FieldType::Map) section, a Blood
//! Pool [`Track`](FieldType::Track), and a Generation background onto the
//! base sheet **without editing the base**, via [`SheetSchemaBuilder`]:
//!
//! ```
//! use mush_ruleset::schema::{SheetSchemaBuilder, SheetSection, SectionLayout, FieldDef, FieldType};
//!
//! // The base ruleset ships this.
//! let base = SheetSchemaBuilder::new("wod", 1)
//!     .section(SheetSection::new("attributes", "Attributes", SectionLayout::grid(3, 3))
//!         .field(FieldDef::dots("strength", "Strength", 1, 5).category("physical")))
//!     .unwrap()
//!     .build();
//!
//! // A splat sub-module extends it — base is untouched.
//! let vampire = SheetSchemaBuilder::from_schema(&base)
//!     .set_version(2)
//!     .section(SheetSection::new("disciplines", "Disciplines", SectionLayout::Map)
//!         .field(FieldDef::map("disciplines", "Disciplines", FieldType::Dots { min: 0, max: 5 })))
//!     .unwrap()
//!     .build();
//!
//! assert!(vampire.section("attributes").is_some()); // inherited
//! assert!(vampire.section("disciplines").is_some()); // added
//! assert_eq!(vampire.version, 2);
//! ```
//!
//! Field ids are **globally unique** across the whole sheet (so
//! [`DerivedExpr`] refs and the typed getters resolve unambiguously); the
//! builder rejects collisions at build time.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::RulesetId;

/// The declarative description of a ruleset's character sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SheetSchema {
    /// The owning ruleset id.
    pub ruleset: RulesetId,
    /// The schema version (bumped when fields are added/reshaped; drives
    /// [`migrate_sheet`](crate::Ruleset::migrate_sheet)).
    pub version: u32,
    /// Ordered sheet sections.
    pub sections: Vec<SheetSection>,
}

impl SheetSchema {
    /// The section with this id (case-insensitive), if any.
    #[must_use]
    pub fn section(&self, id: &str) -> Option<&SheetSection> {
        self.sections.iter().find(|s| s.id.eq_ignore_ascii_case(id))
    }

    /// The field with this id (case-insensitive), searched across every
    /// section. Field ids are unique sheet-wide.
    #[must_use]
    pub fn field(&self, id: &str) -> Option<&FieldDef> {
        self.sections
            .iter()
            .flat_map(|s| &s.fields)
            .find(|f| f.id.eq_ignore_ascii_case(id))
    }

    /// The section containing the field with this id, if any.
    #[must_use]
    pub fn section_of_field(&self, field_id: &str) -> Option<&SheetSection> {
        self.sections
            .iter()
            .find(|s| s.fields.iter().any(|f| f.id.eq_ignore_ascii_case(field_id)))
    }

    /// Iterate every `(section, field)` pair in declaration order.
    pub fn fields(&self) -> impl Iterator<Item = (&SheetSection, &FieldDef)> {
        self.sections
            .iter()
            .flat_map(|s| s.fields.iter().map(move |f| (s, f)))
    }

    /// Build the default sheet data document from every field's `default`,
    /// filling derived-size tracks with the right number of empty slots.
    ///
    /// Two passes: scalars first (so derived track sizes can read them),
    /// then tracks.
    #[must_use]
    pub fn default_data(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        // Pass 1: everything non-Track.
        for (_, f) in self.fields() {
            if !matches!(f.ty, FieldType::Track { .. }) {
                map.insert(f.id.to_string(), f.default.clone());
            }
        }
        // Pass 2: tracks, sized against the just-populated scalars.
        let data = serde_json::Value::Object(map.clone());
        for (_, f) in self.fields() {
            if let FieldType::Track { size, .. } = &f.ty {
                let value = if f.default.is_array() {
                    f.default.clone()
                } else {
                    let n = size.resolve(&data);
                    serde_json::Value::Array(vec![serde_json::Value::Null; n])
                };
                map.insert(f.id.to_string(), value);
            }
        }
        serde_json::Value::Object(map)
    }

    /// A JSON description of the sheet for the web client — literally this
    /// schema serialized. Being the single declarative source, the schema
    /// **is** the "JSON-Schema-ish" description the sheet panel renders
    /// from; the framework never invents a second wire shape.
    #[must_use]
    pub fn describe(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("SheetSchema serializes")
    }
}

/// One section of the sheet: a labelled group of fields with a layout hint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SheetSection {
    /// Section id (unique per schema, case-insensitive).
    pub id: SmolStr,
    /// Human label ("Attributes", "Health").
    pub label: String,
    /// Web/terminal layout hint.
    pub layout: SectionLayout,
    /// The section's fields, in display order.
    pub fields: Vec<FieldDef>,
}

impl SheetSection {
    /// A fresh, empty section.
    #[must_use]
    pub fn new(id: &str, label: impl Into<String>, layout: SectionLayout) -> SheetSection {
        SheetSection {
            id: SmolStr::new(id),
            label: label.into(),
            layout,
            fields: Vec::new(),
        }
    }

    /// Builder-style: append a field.
    #[must_use]
    pub fn field(mut self, field: FieldDef) -> SheetSection {
        self.fields.push(field);
        self
    }

    /// Builder-style: append several fields.
    #[must_use]
    pub fn fields_iter(mut self, fields: impl IntoIterator<Item = FieldDef>) -> SheetSection {
        self.fields.extend(fields);
        self
    }
}

/// How a section is laid out (a rendering hint; the framework does not
/// enforce it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionLayout {
    /// A grid, e.g. the 3×3 attribute block.
    Grid {
        /// Row count.
        rows: u8,
        /// Column count.
        cols: u8,
    },
    /// A simple vertical list.
    List,
    /// Damage/resource tracks.
    Tracks,
    /// A name→value map (backgrounds, disciplines, merits).
    Map,
}

impl SectionLayout {
    /// Shorthand for [`SectionLayout::Grid`].
    #[must_use]
    pub fn grid(rows: u8, cols: u8) -> SectionLayout {
        SectionLayout::Grid { rows, cols }
    }
}

/// One field on the sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldDef {
    /// Field id (unique sheet-wide, case-insensitive).
    pub id: SmolStr,
    /// Human label ("Strength", "Brawl").
    pub label: String,
    /// The field's type + constraints.
    pub ty: FieldType,
    /// Optional grouping tag ("physical"/"social"/"mental",
    /// "talents"/"skills"/"knowledges") — used by chargen budgets and the
    /// renderer, never by validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<SmolStr>,
    /// The value a fresh sheet starts with.
    pub default: serde_json::Value,
    /// If set, players cannot self-edit this field (staff/`@sheet/set` only)
    /// and it is hidden from non-controllers on `+sheet`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub staff_only: bool,
    /// When set, this field is a **vital** — one of the numbers a status
    /// strip or HUD shows — with the given placement (interface plan I1,
    /// D-I-2). Only `Track`, `Number` and `Dots` fields can be vitals; the
    /// generic publisher ([`crate::vitals::from_sheet`]) turns them into
    /// wire `Vital`s without the client knowing the ruleset. Declared once
    /// here, rendered everywhere, like the rest of the schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vital: Option<mush_proto::VitalRole>,
}

impl FieldDef {
    /// A dots field (`min..=max`), defaulting to `min`.
    #[must_use]
    pub fn dots(id: &str, label: impl Into<String>, min: u8, max: u8) -> FieldDef {
        FieldDef::new(
            id,
            label,
            FieldType::Dots { min, max },
            serde_json::json!(min),
        )
    }

    /// A bounded integer field, defaulting to `min.clamp(0, ..)` → actually
    /// to `min` (the safe in-range default).
    #[must_use]
    pub fn number(id: &str, label: impl Into<String>, min: i64, max: i64) -> FieldDef {
        FieldDef::new(
            id,
            label,
            FieldType::Number { min, max },
            serde_json::json!(min),
        )
    }

    /// A free-text field, defaulting to `""`.
    #[must_use]
    pub fn text(id: &str, label: impl Into<String>, max_len: u32) -> FieldDef {
        FieldDef::new(
            id,
            label,
            FieldType::Text { max_len },
            serde_json::json!(""),
        )
    }

    /// An enum field, defaulting to the first option (or `""` if empty).
    #[must_use]
    pub fn enumeration(id: &str, label: impl Into<String>, options: Vec<SmolStr>) -> FieldDef {
        let default = options
            .first()
            .map_or_else(|| serde_json::json!(""), |o| serde_json::json!(o));
        FieldDef::new(id, label, FieldType::Enum { options }, default)
    }

    /// A damage/resource track, defaulting to empty slots (built by
    /// [`SheetSchema::default_data`]).
    #[must_use]
    pub fn track(
        id: &str,
        label: impl Into<String>,
        size: TrackSize,
        levels: Vec<DamageLevelDef>,
    ) -> FieldDef {
        FieldDef::new(
            id,
            label,
            FieldType::Track { size, levels },
            serde_json::Value::Null,
        )
    }

    /// A homogeneous list field, defaulting to `[]`.
    #[must_use]
    pub fn list(id: &str, label: impl Into<String>, of: FieldType) -> FieldDef {
        FieldDef::new(
            id,
            label,
            FieldType::List { of: Box::new(of) },
            serde_json::json!([]),
        )
    }

    /// A name→value map field, defaulting to `{}`.
    #[must_use]
    pub fn map(id: &str, label: impl Into<String>, value: FieldType) -> FieldDef {
        FieldDef::new(
            id,
            label,
            FieldType::Map {
                value: Box::new(value),
            },
            serde_json::json!({}),
        )
    }

    /// The general constructor (category `None`, not staff-only).
    #[must_use]
    pub fn new(
        id: &str,
        label: impl Into<String>,
        ty: FieldType,
        default: serde_json::Value,
    ) -> FieldDef {
        FieldDef {
            id: SmolStr::new(id),
            label: label.into(),
            ty,
            category: None,
            default,
            staff_only: false,
            vital: None,
        }
    }

    /// Builder-style: set the grouping category.
    #[must_use]
    pub fn category(mut self, category: &str) -> FieldDef {
        self.category = Some(SmolStr::new(category));
        self
    }

    /// Builder-style: declare the field a vital with the given placement
    /// (see [`FieldDef::vital`]).
    #[must_use]
    pub fn vital(mut self, role: mush_proto::VitalRole) -> FieldDef {
        self.vital = Some(role);
        self
    }

    /// Builder-style: mark the field staff-only.
    #[must_use]
    pub fn staff_only(mut self) -> FieldDef {
        self.staff_only = true;
        self
    }

    /// Builder-style: override the default value.
    #[must_use]
    pub fn with_default(mut self, default: serde_json::Value) -> FieldDef {
        self.default = default;
        self
    }
}

/// The type and constraints of a field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FieldType {
    /// A dot rating `min..=max` (attributes, abilities, backgrounds).
    Dots {
        /// Minimum dots.
        min: u8,
        /// Maximum dots.
        max: u8,
    },
    /// A bounded integer (willpower pool, quintessence, xp).
    Number {
        /// Minimum value.
        min: i64,
        /// Maximum value.
        max: i64,
    },
    /// Free text (concept, nature, notes).
    Text {
        /// Maximum length in characters.
        max_len: u32,
    },
    /// One of a fixed set (template: mortal/vampire/…).
    Enum {
        /// The allowed values.
        options: Vec<SmolStr>,
    },
    /// A fixed- or derived-size track of damage/resource slots (health,
    /// blood pool). Each slot is `null` (empty) or a mark string
    /// ("bashing"/"lethal"/"aggravated", or a resource token).
    Track {
        /// The number of slots.
        size: TrackSize,
        /// Per-level metadata (label + wound penalty), longest-track-first.
        levels: Vec<DamageLevelDef>,
    },
    /// A homogeneous list (specialties, merits-as-text).
    List {
        /// The element type.
        of: Box<FieldType>,
    },
    /// A name→value map (backgrounds, disciplines, merits→dots).
    Map {
        /// The value type of each entry.
        value: Box<FieldType>,
    },
}

impl FieldType {
    /// A short type name for error messages.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            FieldType::Dots { .. } => "dots",
            FieldType::Number { .. } => "number",
            FieldType::Text { .. } => "text",
            FieldType::Enum { .. } => "enum",
            FieldType::Track { .. } => "track",
            FieldType::List { .. } => "list",
            FieldType::Map { .. } => "map",
        }
    }
}

/// One level of a damage/resource track: a label and the dice-pool penalty
/// incurred when it is the highest filled level (design crib: Bruised −0 …
/// Crippled −5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DamageLevelDef {
    /// Level id ("bruised", "hurt", "incapacitated").
    pub id: SmolStr,
    /// Human label ("Bruised").
    pub label: String,
    /// Wound penalty applied to pools while this is the deepest filled level
    /// (usually `<= 0`).
    pub penalty: i8,
}

impl DamageLevelDef {
    /// A level definition.
    #[must_use]
    pub fn new(id: &str, label: impl Into<String>, penalty: i8) -> DamageLevelDef {
        DamageLevelDef {
            id: SmolStr::new(id),
            label: label.into(),
            penalty,
        }
    }
}

/// A track's size: fixed, or derived from other numeric fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackSize {
    /// A constant number of slots (oWoD health = 7).
    Fixed(u8),
    /// Computed from sibling fields (`stamina + size`, `stamina + 3`).
    Derived(DerivedExpr),
}

impl TrackSize {
    /// Resolve the concrete slot count against a sheet-data document
    /// (missing numeric refs read as 0; the result is clamped to `>= 0`).
    #[must_use]
    pub fn resolve(&self, data: &serde_json::Value) -> usize {
        match self {
            TrackSize::Fixed(n) => *n as usize,
            TrackSize::Derived(expr) => expr.eval_data(data).max(0) as usize,
        }
    }
}

/// A tiny arithmetic AST over sheet-field references — "enough for derived
/// stats without inventing softcode" (design doc 04 §3.3). Evaluated by this
/// crate; deliberately not Turing-complete.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivedExpr {
    /// A reference to another numeric field's value (by field id).
    Field(SmolStr),
    /// A literal integer.
    Const(i64),
    /// Sum of two sub-expressions.
    Add(Box<DerivedExpr>, Box<DerivedExpr>),
    /// Difference of two sub-expressions.
    Sub(Box<DerivedExpr>, Box<DerivedExpr>),
    /// Product of two sub-expressions.
    Mul(Box<DerivedExpr>, Box<DerivedExpr>),
}

impl DerivedExpr {
    /// A field reference.
    #[must_use]
    pub fn field(id: &str) -> DerivedExpr {
        DerivedExpr::Field(SmolStr::new(id))
    }

    /// A constant.
    #[must_use]
    pub fn constant(n: i64) -> DerivedExpr {
        DerivedExpr::Const(n)
    }

    /// `self + rhs` (builder combinator, not the `std::ops` trait).
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, rhs: DerivedExpr) -> DerivedExpr {
        DerivedExpr::Add(Box::new(self), Box::new(rhs))
    }

    /// `self - rhs` (builder combinator, not the `std::ops` trait).
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn sub(self, rhs: DerivedExpr) -> DerivedExpr {
        DerivedExpr::Sub(Box::new(self), Box::new(rhs))
    }

    /// `self * rhs` (builder combinator, not the `std::ops` trait).
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn mul(self, rhs: DerivedExpr) -> DerivedExpr {
        DerivedExpr::Mul(Box::new(self), Box::new(rhs))
    }

    /// Evaluate against a lookup function (field id → value; unknown fields
    /// yield 0).
    #[must_use]
    pub fn eval(&self, get: &dyn Fn(&str) -> i64) -> i64 {
        match self {
            DerivedExpr::Field(id) => get(id),
            DerivedExpr::Const(n) => *n,
            DerivedExpr::Add(a, b) => a.eval(get).saturating_add(b.eval(get)),
            DerivedExpr::Sub(a, b) => a.eval(get).saturating_sub(b.eval(get)),
            DerivedExpr::Mul(a, b) => a.eval(get).saturating_mul(b.eval(get)),
        }
    }

    /// Evaluate against a sheet-data document (field values read as `i64`;
    /// non-numeric or missing fields read as 0).
    #[must_use]
    pub fn eval_data(&self, data: &serde_json::Value) -> i64 {
        self.eval(&|id| {
            data.get(id)
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0)
        })
    }

    /// Every field id referenced by this expression (for build-time
    /// validation that derived refs point at real fields).
    #[must_use]
    pub fn referenced_fields(&self) -> Vec<SmolStr> {
        let mut out = Vec::new();
        self.collect_refs(&mut out);
        out
    }

    fn collect_refs(&self, out: &mut Vec<SmolStr>) {
        match self {
            DerivedExpr::Field(id) => out.push(id.clone()),
            DerivedExpr::Const(_) => {}
            DerivedExpr::Add(a, b) | DerivedExpr::Sub(a, b) | DerivedExpr::Mul(a, b) => {
                a.collect_refs(out);
                b.collect_refs(out);
            }
        }
    }
}

/// A schema composition/build error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaError {
    /// A section id was added twice (use [`SheetSchemaBuilder::extend_section`]
    /// to add fields to an existing section).
    #[error("duplicate section id {0:?}")]
    DuplicateSection(SmolStr),
    /// [`SheetSchemaBuilder::extend_section`] named a section that does not exist.
    #[error("no such section {0:?} to extend")]
    NoSuchSection(SmolStr),
    /// A field id collided with one already in the schema (ids are unique
    /// sheet-wide).
    #[error("duplicate field id {0:?} (field ids are unique across the whole sheet)")]
    DuplicateField(SmolStr),
    /// A derived track size referenced a field the schema does not define.
    #[error("field {field:?} derives its size from unknown field {missing:?}")]
    UnknownDerivedRef {
        /// The track field.
        field: SmolStr,
        /// The dangling reference.
        missing: SmolStr,
    },
}

/// The composable builder for [`SheetSchema`] — the splat seam (see the
/// module docs). Enforces unique section ids and sheet-wide-unique field
/// ids, so base + template extensions merge without collisions.
pub struct SheetSchemaBuilder {
    ruleset: RulesetId,
    version: u32,
    sections: Vec<SheetSection>,
    /// lowercased section id → index in `sections`.
    section_idx: HashMap<String, usize>,
    /// lowercased field ids seen so far (sheet-wide uniqueness).
    field_ids: HashSet<String>,
}

impl SheetSchemaBuilder {
    /// A fresh builder for `ruleset` at `version`.
    #[must_use]
    pub fn new(ruleset: &str, version: u32) -> SheetSchemaBuilder {
        SheetSchemaBuilder {
            ruleset: SmolStr::new(ruleset),
            version,
            sections: Vec::new(),
            section_idx: HashMap::new(),
            field_ids: HashSet::new(),
        }
    }

    /// Seed a builder from an existing schema (the template-extension entry
    /// point: `from_schema(base).section(...)`). The base is cloned; the
    /// original is untouched.
    #[must_use]
    pub fn from_schema(base: &SheetSchema) -> SheetSchemaBuilder {
        let mut b = SheetSchemaBuilder::new(&base.ruleset, base.version);
        for section in &base.sections {
            for f in &section.fields {
                b.field_ids.insert(f.id.to_lowercase());
            }
            b.section_idx
                .insert(section.id.to_lowercase(), b.sections.len());
            b.sections.push(section.clone());
        }
        b
    }

    /// Override the composed version (e.g. a splat bumps it).
    #[must_use]
    pub fn set_version(mut self, version: u32) -> SheetSchemaBuilder {
        self.version = version;
        self
    }

    /// Add a whole new section. Errors on a duplicate section id or a field
    /// id already used anywhere in the schema.
    pub fn section(mut self, section: SheetSection) -> Result<SheetSchemaBuilder, SchemaError> {
        self.try_section(section)?;
        Ok(self)
    }

    /// Fallible in-place variant of [`SheetSchemaBuilder::section`].
    pub fn try_section(&mut self, section: SheetSection) -> Result<(), SchemaError> {
        let key = section.id.to_lowercase();
        if self.section_idx.contains_key(&key) {
            return Err(SchemaError::DuplicateSection(section.id.clone()));
        }
        for f in &section.fields {
            let fk = f.id.to_lowercase();
            if !self.field_ids.insert(fk) {
                return Err(SchemaError::DuplicateField(f.id.clone()));
            }
        }
        self.section_idx.insert(key, self.sections.len());
        self.sections.push(section);
        Ok(())
    }

    /// Append fields to an existing section (adding oWoD abilities, a splat's
    /// extra backgrounds). Errors if the section is unknown or a field id
    /// collides.
    pub fn extend_section(
        mut self,
        section_id: &str,
        fields: impl IntoIterator<Item = FieldDef>,
    ) -> Result<SheetSchemaBuilder, SchemaError> {
        self.try_extend_section(section_id, fields)?;
        Ok(self)
    }

    /// Fallible in-place variant of [`SheetSchemaBuilder::extend_section`].
    pub fn try_extend_section(
        &mut self,
        section_id: &str,
        fields: impl IntoIterator<Item = FieldDef>,
    ) -> Result<(), SchemaError> {
        let idx = *self
            .section_idx
            .get(&section_id.to_lowercase())
            .ok_or_else(|| SchemaError::NoSuchSection(SmolStr::new(section_id)))?;
        for f in fields {
            let fk = f.id.to_lowercase();
            if !self.field_ids.insert(fk) {
                return Err(SchemaError::DuplicateField(f.id.clone()));
            }
            self.sections[idx].fields.push(f);
        }
        Ok(())
    }

    /// Finish, validating that every derived track size references a real
    /// field.
    pub fn try_build(self) -> Result<SheetSchema, SchemaError> {
        for section in &self.sections {
            for f in &section.fields {
                if let FieldType::Track {
                    size: TrackSize::Derived(expr),
                    ..
                } = &f.ty
                {
                    for r in expr.referenced_fields() {
                        if !self.field_ids.contains(&r.to_lowercase()) {
                            return Err(SchemaError::UnknownDerivedRef {
                                field: f.id.clone(),
                                missing: r,
                            });
                        }
                    }
                }
            }
        }
        Ok(SheetSchema {
            ruleset: self.ruleset,
            version: self.version,
            sections: self.sections,
        })
    }

    /// Finish, panicking on a build error — for schemas built from trusted
    /// source constants (a ruleset's own base sheet), where a collision is a
    /// programming error, not runtime data.
    ///
    /// # Panics
    ///
    /// Panics if [`SheetSchemaBuilder::try_build`] would error.
    #[must_use]
    pub fn build(self) -> SheetSchema {
        self.try_build().expect("sheet schema must build")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> SheetSchema {
        SheetSchemaBuilder::new("toy", 1)
            .section(
                SheetSection::new("attributes", "Attributes", SectionLayout::grid(3, 3))
                    .field(FieldDef::dots("strength", "Strength", 1, 5).category("physical"))
                    .field(FieldDef::dots("stamina", "Stamina", 1, 5).category("physical")),
            )
            .unwrap()
            .section(
                SheetSection::new("health", "Health", SectionLayout::Tracks).field(
                    FieldDef::track(
                        "health",
                        "Health",
                        TrackSize::Derived(
                            DerivedExpr::field("stamina").add(DerivedExpr::constant(5)),
                        ),
                        vec![DamageLevelDef::new("bruised", "Bruised", 0)],
                    ),
                ),
            )
            .unwrap()
            .build()
    }

    #[test]
    fn compose_appends_without_touching_base() {
        let b = base();
        let extended = SheetSchemaBuilder::from_schema(&b)
            .set_version(2)
            .section(
                SheetSection::new("disciplines", "Disciplines", SectionLayout::Map).field(
                    FieldDef::map(
                        "disciplines",
                        "Disciplines",
                        FieldType::Dots { min: 0, max: 5 },
                    ),
                ),
            )
            .unwrap()
            .extend_section(
                "attributes",
                [FieldDef::dots("dexterity", "Dexterity", 1, 5)],
            )
            .unwrap()
            .build();

        // Base is untouched.
        assert_eq!(b.version, 1);
        assert!(b.section("disciplines").is_none());
        assert_eq!(b.section("attributes").unwrap().fields.len(), 2);

        // Extension has both.
        assert_eq!(extended.version, 2);
        assert!(extended.section("disciplines").is_some());
        assert_eq!(extended.section("attributes").unwrap().fields.len(), 3);
        // Ordering preserved: dexterity appended last.
        assert_eq!(
            extended.section("attributes").unwrap().fields[2].id,
            "dexterity"
        );
    }

    #[test]
    fn duplicate_section_and_field_rejected() {
        let dup_section = SheetSchemaBuilder::new("toy", 1)
            .section(SheetSection::new("a", "A", SectionLayout::List))
            .unwrap()
            .section(SheetSection::new("A", "A2", SectionLayout::List));
        assert!(matches!(dup_section, Err(SchemaError::DuplicateSection(_))));

        let dup_field = SheetSchemaBuilder::new("toy", 1)
            .section(
                SheetSection::new("a", "A", SectionLayout::List)
                    .field(FieldDef::dots("x", "X", 1, 5)),
            )
            .unwrap()
            .section(
                SheetSection::new("b", "B", SectionLayout::List)
                    .field(FieldDef::dots("X", "X2", 1, 5)),
            );
        assert!(matches!(dup_field, Err(SchemaError::DuplicateField(_))));
    }

    #[test]
    fn extend_unknown_section_errors() {
        let r = SheetSchemaBuilder::new("toy", 1)
            .extend_section("nope", [FieldDef::dots("z", "Z", 1, 5)]);
        assert!(matches!(r, Err(SchemaError::NoSuchSection(_))));
    }

    #[test]
    fn unknown_derived_ref_rejected() {
        let r = SheetSchemaBuilder::new("toy", 1)
            .section(
                SheetSection::new("h", "H", SectionLayout::Tracks).field(FieldDef::track(
                    "hp",
                    "HP",
                    TrackSize::Derived(DerivedExpr::field("ghost")),
                    vec![],
                )),
            )
            .unwrap()
            .try_build();
        assert!(matches!(r, Err(SchemaError::UnknownDerivedRef { .. })));
    }

    #[test]
    fn default_data_sizes_derived_tracks() {
        let b = base();
        let data = b.default_data();
        // stamina defaults to 1, so health = 1 + 5 = 6 empty slots.
        assert_eq!(data["strength"], serde_json::json!(1));
        assert_eq!(data["health"].as_array().unwrap().len(), 6);
        assert!(
            data["health"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| v.is_null())
        );
    }

    #[test]
    fn derived_expr_eval() {
        let e = DerivedExpr::field("a")
            .add(DerivedExpr::field("b"))
            .mul(DerivedExpr::constant(2));
        let data = serde_json::json!({ "a": 3, "b": 4 });
        assert_eq!(e.eval_data(&data), 14);
        // Missing ref reads as 0.
        assert_eq!(DerivedExpr::field("missing").eval_data(&data), 0);
    }
}
