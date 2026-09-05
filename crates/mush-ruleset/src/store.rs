//! [`SheetStore`] — a validating, auditing facade over `World` attribute
//! access (treaty ruling 7: one storage, one API).
//!
//! Sheets live as `AttrValue::Json` under the attribute `sheet:<ruleset>`.
//! Reads deserialize (and optionally migrate) the document; writes validate
//! the data against the [`SheetSchema`] and go through `World::set_attr`,
//! which stamps the object modified so the write-through persistence layer
//! picks it up. No sheet write ever bypasses validation.
//!
//! The store also offers the typed field/track helpers rulesets lean on:
//! validated single-field sets, and the generic wound-penalty read over a
//! damage track.

use std::sync::Arc;

use mush_core::{Attr, Ctx, ObjId, World, WorldEvent};

use crate::doc::SheetDoc;
use crate::schema::{FieldType, SheetSchema};
use crate::validate::{SheetError, validate};

/// The attribute namespace every sheet lives under (`sheet:<ruleset>` →
/// namespace `sheet`). A ruleset claims this namespace so the core `@set`
/// backdoor cannot let players hand-edit sheets; the store itself is the
/// sanctioned writer and does not go through that gate.
pub const SHEET_ATTR_NS: &str = "sheet";

/// A lazy sheet migration: `(from_version, data) -> data` at the current
/// schema version. Boxed so this crate needs no dependency on the concrete
/// ruleset; a ruleset supplies one that delegates to
/// [`Ruleset::migrate_sheet`](crate::Ruleset::migrate_sheet).
pub type MigrateFn =
    Arc<dyn Fn(u32, serde_json::Value) -> Result<serde_json::Value, SheetError> + Send + Sync>;

/// The attribute name a ruleset's sheets are stored under.
#[must_use]
pub fn sheet_attr_name(ruleset: &str) -> String {
    format!("{SHEET_ATTR_NS}:{ruleset}")
}

/// A validating facade over sheet storage for one ruleset. Cheap to
/// construct per call; holds an `Arc<SheetSchema>` and an optional migrator.
#[derive(Clone)]
pub struct SheetStore {
    schema: Arc<SheetSchema>,
    migrate: Option<MigrateFn>,
}

impl SheetStore {
    /// A store over `schema` with no migration (versions are assumed
    /// current).
    #[must_use]
    pub fn new(schema: Arc<SheetSchema>) -> SheetStore {
        SheetStore {
            schema,
            migrate: None,
        }
    }

    /// A store that lazily upgrades documents behind the schema version via
    /// `migrate`.
    #[must_use]
    pub fn with_migration(schema: Arc<SheetSchema>, migrate: MigrateFn) -> SheetStore {
        SheetStore {
            schema,
            migrate: Some(migrate),
        }
    }

    /// The schema this store validates against.
    #[must_use]
    pub fn schema(&self) -> &SheetSchema {
        &self.schema
    }

    /// The attribute name sheets are stored under (`sheet:<ruleset>`).
    #[must_use]
    pub fn attr_name(&self) -> String {
        sheet_attr_name(&self.schema.ruleset)
    }

    /// Read a sheet, applying lazy migration in memory (does **not** write
    /// the upgrade back — use [`SheetStore::get_and_upgrade`] for that).
    /// Returns `Ok(None)` when the object has no sheet.
    ///
    /// # Errors
    ///
    /// Returns a [`SheetError`] if the stored value cannot be decoded as a
    /// [`SheetDoc`] or migration fails.
    pub fn try_get(&self, w: &World, obj: ObjId) -> Result<Option<SheetDoc>, SheetError> {
        let name = self.attr_name();
        let Some(attr) = w.attr(obj, &name) else {
            return Ok(None);
        };
        let Some(json) = attr.as_json() else {
            return Err(SheetError::msg(format!(
                "{name} on #{} is not a JSON sheet document",
                obj.0
            )));
        };
        let mut doc: SheetDoc = serde_json::from_value(json.clone())
            .map_err(|e| SheetError::msg(format!("corrupt sheet on #{}: {e}", obj.0)))?;
        self.migrate_in_place(&mut doc)?;
        Ok(Some(doc))
    }

    /// [`SheetStore::try_get`] swallowing errors to `None` (logging them) —
    /// the ergonomic read path for display commands.
    #[must_use]
    pub fn get(&self, w: &World, obj: ObjId) -> Option<SheetDoc> {
        match self.try_get(w, obj) {
            Ok(doc) => doc,
            Err(e) => {
                tracing::error!(obj = obj.0, error = %e, "sheet read failed");
                None
            }
        }
    }

    /// The sheet, or a fresh default one if the object has none.
    #[must_use]
    pub fn get_or_default(&self, w: &World, obj: ObjId) -> SheetDoc {
        self.get(w, obj)
            .unwrap_or_else(|| SheetDoc::new_default(&self.schema))
    }

    /// Read, and if migration changed the document, persist the upgrade back
    /// (audited to `by`). Returns the current doc.
    ///
    /// # Errors
    ///
    /// Propagates decode/migration/validation errors.
    pub fn get_and_upgrade(
        &self,
        w: &mut World,
        obj: ObjId,
        by: ObjId,
    ) -> Result<Option<SheetDoc>, Vec<SheetError>> {
        let name = self.attr_name();
        let before_version = match w.attr(obj, &name).and_then(Attr::as_json) {
            Some(v) => v.get("schema_version").and_then(serde_json::Value::as_u64),
            None => return Ok(None),
        };
        let doc = self.try_get(w, obj).map_err(|e| vec![e])?;
        let Some(doc) = doc else { return Ok(None) };
        if before_version != Some(u64::from(doc.schema_version)) {
            self.put_by(w, obj, &doc, by)?;
        }
        Ok(Some(doc))
    }

    /// Validate `doc.data` against the schema and write it (audited to the
    /// object itself). Marks the object dirty for persistence.
    ///
    /// # Errors
    ///
    /// Returns the collected [`SheetError`]s if validation fails; nothing is
    /// written in that case.
    pub fn put(&self, w: &mut World, obj: ObjId, doc: &SheetDoc) -> Result<(), Vec<SheetError>> {
        self.put_by(w, obj, doc, obj)
    }

    /// [`SheetStore::put`] recording `by` as the attribute's audit owner
    /// (the staffer running `@sheet/set`, or the player in chargen).
    ///
    /// # Errors
    ///
    /// Returns the collected [`SheetError`]s if validation fails.
    pub fn put_by(
        &self,
        w: &mut World,
        obj: ObjId,
        doc: &SheetDoc,
        by: ObjId,
    ) -> Result<(), Vec<SheetError>> {
        validate(&self.schema, &doc.data)?;
        let value = serde_json::to_value(doc)
            .map_err(|e| vec![SheetError::msg(format!("sheet serialize: {e}"))])?;
        w.set_attr(obj, &self.attr_name(), Attr::json(value, by))
            .map_err(|e| vec![SheetError::msg(e.to_string())])?;
        Ok(())
    }

    /// [`SheetStore::put_by`] from inside a command or observer: the same
    /// validated write, then a [`WorldEvent::AttrSet`] for the sheet
    /// attribute so subscribers run — the `<ruleset>.vitals` and
    /// `<ruleset>.sheet` republishers above all (interface plan I1). `put_by`
    /// itself cannot emit (it has no `Ctx`), which is why every command-side
    /// write goes through here and a probe keeps it that way.
    ///
    /// `ctx` comes **last** so a caller can pass `ctx.actor` as `by` (the
    /// arguments before it are evaluated before `ctx` is reborrowed).
    ///
    /// # Errors
    ///
    /// Returns the collected [`SheetError`]s if validation fails; nothing is
    /// written and no event fires in that case.
    pub fn put_in(
        &self,
        obj: ObjId,
        doc: &SheetDoc,
        by: ObjId,
        ctx: &mut Ctx<'_>,
    ) -> Result<(), Vec<SheetError>> {
        self.put_by(ctx.w, obj, doc, by)?;
        ctx.emit_world_event(&WorldEvent::AttrSet {
            obj,
            name: self.attr_name(),
            setter: by,
        });
        Ok(())
    }

    /// Delete a sheet outright. Returns whether one was present.
    pub fn remove(&self, w: &mut World, obj: ObjId) -> bool {
        matches!(w.remove_attr(obj, &self.attr_name()), Ok(Some(_)))
    }

    // ------------------------------------------------------------------
    // Typed single-field helpers (validate just the touched field)
    // ------------------------------------------------------------------

    /// Set one scalar field on an in-memory doc, validating that value
    /// against its schema type. Does not touch storage — `put` when done.
    ///
    /// # Errors
    ///
    /// Returns a [`SheetError`] if the field is unknown or the value is
    /// invalid for its type.
    pub fn set_field(
        &self,
        doc: &mut SheetDoc,
        field: &str,
        value: serde_json::Value,
    ) -> Result<(), SheetError> {
        let f = self
            .schema
            .field(field)
            .ok_or_else(|| SheetError::UnknownField {
                field: field.to_string(),
            })?;
        // Validate the single value by round-tripping through the full
        // validator on a scratch doc (cheap; reuses one code path).
        let mut scratch = serde_json::Map::new();
        scratch.insert(f.id.to_string(), value.clone());
        // Track fields need sibling numeric fields for size resolution.
        if matches!(f.ty, FieldType::Track { .. })
            && let Some(obj) = doc.as_object()
        {
            for (k, v) in obj {
                scratch.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        let id = f.id.clone();
        validate(&self.schema, &serde_json::Value::Object(scratch))
            .map_err(|mut es| es.pop().unwrap_or_else(|| SheetError::msg("invalid field")))?;
        doc.set(id.as_str(), value);
        Ok(())
    }

    /// Set one entry of a map field, validating the value against the map's
    /// element type.
    ///
    /// # Errors
    ///
    /// Returns a [`SheetError`] if the field is not a map or the value is
    /// invalid.
    pub fn set_map_entry(
        &self,
        doc: &mut SheetDoc,
        field: &str,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), SheetError> {
        let f = self
            .schema
            .field(field)
            .ok_or_else(|| SheetError::UnknownField {
                field: field.to_string(),
            })?;
        let FieldType::Map { .. } = &f.ty else {
            return Err(SheetError::WrongType {
                field: field.to_string(),
                expected: "map",
                got: f.ty.name(),
            });
        };
        // Build the candidate map and validate the whole field.
        let mut candidate = doc.map(field).cloned().unwrap_or_default();
        candidate.insert(key.to_string(), value);
        self.set_field(doc, field, serde_json::Value::Object(candidate))
    }

    // ------------------------------------------------------------------
    // Track helpers (generic over any damage/resource track)
    // ------------------------------------------------------------------

    /// The resolved slot count of a track field (`0` if it is not a track).
    #[must_use]
    pub fn track_size(&self, doc: &SheetDoc, field: &str) -> usize {
        match self.schema.field(field).map(|f| &f.ty) {
            Some(FieldType::Track { size, .. }) => size.resolve(&doc.data),
            _ => 0,
        }
    }

    /// Apply the migrator (if any) to bring `doc` up to the schema version,
    /// in place. A no-op when the versions already match or no migrator is
    /// installed (the doc is left as-is, best-effort).
    ///
    /// When a migrator runs, the result is **validated** before it is handed
    /// back: the write path validates every `put`, but the read path
    /// (`+roll`, `+sheet`, `agent_profile`, …) consumes migrated data
    /// directly, so a buggy migrator or a tightened bound applied to an older
    /// stored value must not silently feed out-of-schema values (e.g. a
    /// `strength: 99` becoming a 99-die pool) to readers. A validation
    /// failure quarantines the document — [`SheetStore::get`] logs it and
    /// reads `None` rather than serving the bad data.
    fn migrate_in_place(&self, doc: &mut SheetDoc) -> Result<(), SheetError> {
        if doc.schema_version == self.schema.version {
            return Ok(());
        }
        if let Some(migrate) = &self.migrate {
            let data = std::mem::take(&mut doc.data);
            doc.data = migrate(doc.schema_version, data)?;
            doc.schema_version = self.schema.version;
            if let Err(errs) = validate(&self.schema, &doc.data) {
                let detail = errs
                    .into_iter()
                    .next()
                    .map_or_else(String::new, |e| e.to_string());
                return Err(SheetError::msg(format!(
                    "migrated sheet failed schema validation: {detail}"
                )));
            }
        }
        Ok(())
    }

    /// The wound penalty currently in effect from a damage track: the
    /// `penalty` of the deepest filled level, mapping filled-slot count onto
    /// the track's [`DamageLevelDef`](crate::schema::DamageLevelDef)s in
    /// order. Returns `0` for no damage, an empty level list, or a non-track
    /// field.
    #[must_use]
    pub fn wound_penalty(&self, doc: &SheetDoc, field: &str) -> i8 {
        let Some(crate::schema::FieldType::Track { levels, .. }) =
            self.schema.field(field).map(|f| &f.ty)
        else {
            return 0;
        };
        let Some(marks) = doc.track(field) else {
            return 0;
        };
        let filled = marks.iter().filter(|m| m.is_some()).count();
        if filled == 0 || levels.is_empty() {
            return 0;
        }
        // The deepest filled level: slot `filled - 1` maps to a level; clamp
        // to the last defined level for tracks longer than their level list.
        let idx = (filled - 1).min(levels.len() - 1);
        levels[idx].penalty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{GOD, LIMBO, ObjKind};
    use smol_str::SmolStr;

    use crate::doc::SheetDoc;
    use crate::schema::{
        DamageLevelDef, FieldDef, FieldType, SectionLayout, SheetSchemaBuilder, SheetSection,
        TrackSize,
    };

    fn schema() -> Arc<SheetSchema> {
        Arc::new(
            SheetSchemaBuilder::new("toy", 1)
                .section(
                    SheetSection::new("core", "Core", SectionLayout::List)
                        .field(FieldDef::dots("stamina", "Stamina", 1, 5))
                        .field(FieldDef::map(
                            "backgrounds",
                            "Backgrounds",
                            FieldType::Dots { min: 0, max: 5 },
                        )),
                )
                .unwrap()
                .section(
                    SheetSection::new("health", "Health", SectionLayout::Tracks).field(
                        FieldDef::track(
                            "health",
                            "Health",
                            TrackSize::Fixed(7),
                            vec![
                                DamageLevelDef::new("bruised", "Bruised", 0),
                                DamageLevelDef::new("hurt", "Hurt", -1),
                                DamageLevelDef::new("injured", "Injured", -1),
                                DamageLevelDef::new("wounded", "Wounded", -2),
                                DamageLevelDef::new("mauled", "Mauled", -2),
                                DamageLevelDef::new("crippled", "Crippled", -5),
                                DamageLevelDef::new("incap", "Incapacitated", -127),
                            ],
                        ),
                    ),
                )
                .unwrap()
                .build(),
        )
    }

    fn world() -> (World, ObjId) {
        let mut w = World::new_minimal();
        let bob = w.create(ObjKind::Player, "Bob", GOD, Some(LIMBO)).unwrap();
        (w, bob)
    }

    #[test]
    fn put_get_round_trip_identical() {
        let store = SheetStore::new(schema());
        let (mut w, bob) = world();
        let mut doc = SheetDoc::new_default(store.schema());
        doc.set_number("stamina", 3);
        store.put(&mut w, bob, &doc).unwrap();

        let read = store.get(&w, bob).unwrap();
        assert_eq!(read, doc);
        // The write marked the object dirty for persistence.
        assert!(w.take_dirty().contains(&bob));
    }

    #[test]
    fn put_rejects_invalid_without_writing() {
        let store = SheetStore::new(schema());
        let (mut w, bob) = world();
        let mut doc = SheetDoc::new_default(store.schema());
        doc.set_number("stamina", 99); // out of range
        let err = store.put(&mut w, bob, &doc).unwrap_err();
        assert!(!err.is_empty());
        // Nothing was written.
        assert!(store.get(&w, bob).is_none());
    }

    #[test]
    fn set_field_validates() {
        let store = SheetStore::new(schema());
        let mut doc = SheetDoc::new_default(store.schema());
        assert!(
            store
                .set_field(&mut doc, "stamina", serde_json::json!(4))
                .is_ok()
        );
        assert_eq!(doc.dots("stamina"), Some(4));
        assert!(
            store
                .set_field(&mut doc, "stamina", serde_json::json!(6))
                .is_err()
        );
        assert!(
            store
                .set_field(&mut doc, "ghost", serde_json::json!(1))
                .is_err()
        );
    }

    #[test]
    fn set_map_entry_validates_element() {
        let store = SheetStore::new(schema());
        let mut doc = SheetDoc::new_default(store.schema());
        assert!(
            store
                .set_map_entry(&mut doc, "backgrounds", "Resources", serde_json::json!(3))
                .is_ok()
        );
        assert_eq!(
            doc.map("backgrounds").unwrap()["Resources"],
            serde_json::json!(3)
        );
        assert!(
            store
                .set_map_entry(&mut doc, "backgrounds", "Resources", serde_json::json!(9))
                .is_err()
        );
    }

    #[test]
    fn wound_penalty_reads_deepest_level() {
        let store = SheetStore::new(schema());
        let mut doc = SheetDoc::new_default(store.schema());
        assert_eq!(store.wound_penalty(&doc, "health"), 0);
        // Fill 4 slots → level index 3 = Wounded (-2).
        doc.set_track(
            "health",
            &[
                Some(SmolStr::new("bashing")),
                Some(SmolStr::new("bashing")),
                Some(SmolStr::new("lethal")),
                Some(SmolStr::new("lethal")),
                None,
                None,
                None,
            ],
        );
        assert_eq!(store.wound_penalty(&doc, "health"), -2);
    }

    #[test]
    fn migration_upgrades_on_read() {
        // v2 schema renamed nothing but bumped the version; the migrator
        // just stamps the version and fills a new field.
        let schema_v2 = Arc::new(
            SheetSchemaBuilder::new("toy", 2)
                .section(
                    SheetSection::new("core", "Core", SectionLayout::List)
                        .field(FieldDef::dots("stamina", "Stamina", 1, 5))
                        .field(FieldDef::map(
                            "backgrounds",
                            "Backgrounds",
                            FieldType::Dots { min: 0, max: 5 },
                        ))
                        .field(FieldDef::number("willpower", "Willpower", 1, 10)),
                )
                .unwrap()
                .section(
                    SheetSection::new("health", "Health", SectionLayout::Tracks).field(
                        FieldDef::track("health", "Health", TrackSize::Fixed(7), vec![]),
                    ),
                )
                .unwrap()
                .build(),
        );
        let migrate: MigrateFn = Arc::new(|_from, mut data: serde_json::Value| {
            if let Some(obj) = data.as_object_mut() {
                obj.entry("willpower").or_insert(serde_json::json!(1));
            }
            Ok(data)
        });
        let store = SheetStore::with_migration(schema_v2.clone(), migrate);

        // Write a v1 doc directly (no willpower field).
        let (mut w, bob) = world();
        let v1 = SheetDoc {
            ruleset: "toy".into(),
            schema_version: 1,
            data: serde_json::json!({
                "stamina": 2,
                "backgrounds": {},
                "health": [null, null, null, null, null, null, null]
            }),
            approved: crate::doc::ApprovalState::Unapproved,
        };
        w.set_attr(
            bob,
            &store.attr_name(),
            Attr::json(serde_json::to_value(&v1).unwrap(), bob),
        )
        .unwrap();

        let read = store.try_get(&w, bob).unwrap().unwrap();
        assert_eq!(read.schema_version, 2);
        assert_eq!(read.number("willpower"), Some(1));

        // get_and_upgrade persists the bumped version.
        let _ = w.take_dirty();
        let upgraded = store.get_and_upgrade(&mut w, bob, GOD).unwrap().unwrap();
        assert_eq!(upgraded.schema_version, 2);
        assert!(w.take_dirty().contains(&bob));
    }

    #[test]
    fn migration_producing_invalid_data_is_quarantined_on_read() {
        // A buggy migrator that yields an out-of-schema value must not feed
        // that value to readers: try_get errors and get quarantines to None,
        // rather than serving e.g. a 99-dot stat as a 99-die pool.
        let schema_v2 = Arc::new(
            SheetSchemaBuilder::new("toy", 2)
                .section(
                    SheetSection::new("core", "Core", SectionLayout::List)
                        .field(FieldDef::dots("stamina", "Stamina", 1, 5))
                        .field(FieldDef::map(
                            "backgrounds",
                            "Backgrounds",
                            FieldType::Dots { min: 0, max: 5 },
                        )),
                )
                .unwrap()
                .section(
                    SheetSection::new("health", "Health", SectionLayout::Tracks).field(
                        FieldDef::track("health", "Health", TrackSize::Fixed(7), vec![]),
                    ),
                )
                .unwrap()
                .build(),
        );
        // Migrator corrupts stamina out of its 1..=5 range.
        let migrate: MigrateFn = Arc::new(|_from, mut data: serde_json::Value| {
            if let Some(obj) = data.as_object_mut() {
                obj.insert("stamina".to_string(), serde_json::json!(99));
            }
            Ok(data)
        });
        let store = SheetStore::with_migration(schema_v2, migrate);

        let (mut w, bob) = world();
        let v1 = SheetDoc {
            ruleset: "toy".into(),
            schema_version: 1,
            data: serde_json::json!({
                "stamina": 2,
                "backgrounds": {},
                "health": [null, null, null, null, null, null, null]
            }),
            approved: crate::doc::ApprovalState::Unapproved,
        };
        w.set_attr(
            bob,
            &store.attr_name(),
            Attr::json(serde_json::to_value(&v1).unwrap(), bob),
        )
        .unwrap();

        assert!(
            store.try_get(&w, bob).is_err(),
            "invalid migration must error"
        );
        assert!(
            store.get(&w, bob).is_none(),
            "invalid migration must quarantine to None"
        );
    }
}
