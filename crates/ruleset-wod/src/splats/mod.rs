//! # The M6b splat seam — per-template dispatch and the [`Splat`] contract
//!
//! M6a folded a single template into one sheet schema. M6b has **three** splats
//! (Vampire / Mage / Werewolf) and a character is exactly **one** of them, so
//! the model is *dispatch*, not union:
//!
//! - Each character's sheet carries a `template` enum field
//!   (`"mortal"`/`"vampire"`/`"mage"`/`"werewolf"`).
//! - The **active schema** for a character is the base oWoD sheet plus *that
//!   template's* sections only. A vampire has Disciplines + Blood Pool +
//!   Generation; a mage has Spheres + Arete; never both. The ruleset builds one
//!   schema per enabled template at boot ([`TemplateSchemas`]).
//! - `+sheet` render, [`SheetStore`](mush_ruleset::SheetStore) validation,
//!   chargen, `+roll` field resolution and the web sheet query all select the
//!   schema by the character's `template` field (see [`WodCmd`](crate::WodCmd)'s
//!   dispatch helpers).
//! - Splat commands (`+disc`/`+blood`/`+spheres`/`+rage`/…) are always
//!   registered (once, at boot, by their splat) and **self-gate**: each checks
//!   the actor's template and refuses politely ("You are not Kindred.") when it
//!   does not apply.
//!
//! ## The contract for phase-2 splats (Mage, Werewolf)
//!
//! A splat is a `Splat` implementor added under `src/splats/<name>.rs`, then
//! wired into [`splat_by_id`] and the [`DEFAULT_TEMPLATES`] list. Vampire
//! ([`vampire`](crate::splats::vampire)) is the worked reference. To add a
//! splat, implement:
//!
//! - [`Splat::id`] / [`Splat::display_name`] — the `template` enum id + label.
//! - [`Splat::extend_schema`] (from [`schema::Template`](crate::schema::Template),
//!   the M6a seam) — add your sections/fields onto the base builder, **without
//!   touching the base**. Field ids are unique sheet-wide.
//! - [`Splat::register_commands`] — register your splat commands (each must
//!   self-gate on the actor's template via [`is_template`]).
//! - [`Splat::chargen_contribution`] — the extra chargen pools your splat adds
//!   (e.g. Vampire: 3 Discipline dots, 7 Virtue dots) plus any base
//!   [`TraitKind`]s your pools already cover (so base freebie accounting does
//!   not double-charge them). Returns [`ChargenContribution::default`] to add
//!   nothing (Mortal).
//! - [`Splat::finalize_starting`] — derive starting values at chargen finalize
//!   (Willpower, power pools, morality). Runs *after* the base finalize.
//! - [`Splat::chargen_locked_fields`] — fields `+stat` must not set directly
//!   because your splat derives them (Vampire: `bloodpool`, `humanity`,
//!   `weakness`).
//! - [`Splat::on_field_changed`] — recompute derived fields after a `+stat` /
//!   `@sheet/set` (Vampire: refresh the clan weakness when `clan` changes).
//! - [`Splat::xp_max`] / [`Splat::xp_cost`] — claim + cost your splat traits for
//!   `+xp/spend` (Vampire: Disciplines, in-clan vs out-of-clan). Return `None`
//!   to let the base XP rules handle a field.
//!
//! Everything else (Attributes, Abilities, Backgrounds, Willpower, Health,
//! Virtues, `+roll`, `+harm`) is base machinery every splat inherits for free.

pub mod changeling;
pub mod ghoul;
pub mod kinfolk;
pub mod mage;
pub mod vampire;
pub mod werewolf;

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::{CmdError, Ctx, ModError, Registrar};
use mush_ruleset::schema::{SchemaError, SheetSchema, SheetSchemaBuilder};
use mush_ruleset::{SheetDoc, SheetStore};
use smol_str::SmolStr;

use crate::WodCmd;
use crate::config::{TraitKind, WodConfig};
use crate::schema::{Template, base_builder};

/// The templates a default full game enables, in display order. The first is
/// the fallback for a sheet with no (or an unknown) `template` field. M6b
/// landed Vampire/Mage/Werewolf; M10a adds the three mortal-plus / secondary
/// templates Changeling / Ghoul / Kinfolk.
///
/// **Parent-template invariant (M10a):** `ghoul`'s `+disc`/`+blood` commands
/// are registered by (and delegate into) `vampire`, and `kinfolk`'s
/// `+gnosis`/`+gift` are registered by `werewolf` — so a live game that enables
/// `ghoul` must also enable `vampire`, and one that enables `kinfolk` must also
/// enable `werewolf`, or those splats ship without their power commands (no
/// boot error, just a missing surface). `DEFAULT_TEMPLATES` satisfies this by
/// listing both parents. A future configurable-subset feature must preserve the
/// pairing (or move the registrations). Isolated `with_templates`/`with_splats`
/// test sets deliberately exercise a child without its parent, so this is a
/// documented convention, not a runtime assertion.
pub const DEFAULT_TEMPLATES: &[&str] = &[
    "mortal",
    "vampire",
    "mage",
    "werewolf",
    "changeling",
    "ghoul",
    "kinfolk",
];

/// The extended template contract (M6b) — a supertrait of the M6a
/// [`schema::Template`](crate::schema::Template) schema seam that adds chargen,
/// commands, and XP behaviour. See the module docs for the phase-2 checklist.
///
/// Held behind `Arc<dyn Splat>` in the [`SplatRegistry`] and dispatched by the
/// character's `template` field; a splat never inspects another's sheet.
pub(crate) trait Splat: Template {
    /// Register this splat's commands (always active; each handler must
    /// self-gate on the actor's template with [`is_template`] and refuse
    /// politely otherwise). Called once at boot. The default registers nothing
    /// (Mortal / schema-only splats).
    ///
    /// # Errors
    ///
    /// Propagates a [`ModError`] from a colliding command name.
    fn register_commands(&self, reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
        let _ = (reg, cmd);
        Ok(())
    }

    /// The extra chargen pools + freebie rules this splat adds for the given
    /// in-progress `doc` (some pools depend on sheet choices, e.g. a vampire's
    /// Virtue pool follows their Path). Default: nothing (Mortal).
    fn chargen_contribution(&self, config: &WodConfig, doc: &SheetDoc) -> ChargenContribution {
        let _ = (config, doc);
        ChargenContribution::default()
    }

    /// Derive this splat's starting values at chargen finalize — runs *after*
    /// the base finalize (which has already set Willpower from the config
    /// baseline and filled the temporary pool). Default: no-op.
    ///
    /// Vampire keeps the base Willpower (= Courage, the config default) and
    /// fills Blood Pool / Humanity / weakness here. **Phase-2 note:** Mage
    /// (Willpower 5) and Werewolf (Willpower by Auspice) have a *different*
    /// Willpower baseline. The config's `starting_willpower` is currently one
    /// global value; a mixed game therefore needs a per-splat baseline. Set the
    /// derived permanent Willpower here (overriding the base value), and be
    /// aware the freebie accounting still costs Willpower above the *config*
    /// baseline — if that matters for your splat, raise the base config or
    /// extend `chargen::freebie_spent` with a per-splat baseline.
    fn finalize_starting(&self, config: &WodConfig, doc: &mut SheetDoc) {
        let _ = (config, doc);
    }

    /// Fields `+stat` must refuse during chargen because this splat derives
    /// them (Vampire: `bloodpool`, `humanity`, `weakness`). Default: none.
    fn chargen_locked_fields(&self) -> &'static [&'static str] {
        &[]
    }

    /// A hard ceiling this splat imposes on a scalar field **during chargen**,
    /// distinct from the schema max (M20 caps a starting mage's Arete at 3).
    /// `+stat` refuses a higher value. Return `None` (the default) for no extra
    /// chargen cap.
    fn chargen_max(&self, field: &str) -> Option<i32> {
        let _ = field;
        None
    }

    /// An extra, sheet-dependent ceiling on an `+xp/spend` target beyond the
    /// flat [`Splat::xp_max`] (Mage: a Sphere may not exceed the current Arete).
    /// Combined with `xp_max` by taking the lower of the two. Default: none.
    fn xp_target_cap(&self, doc: &SheetDoc, field: &str, key: Option<&str>) -> Option<i32> {
        let _ = (doc, field, key);
        None
    }

    /// Recompute derived fields after `field` was set via `+stat` /
    /// `@sheet/set` (Vampire refreshes the clan weakness when `clan` changes).
    /// Default: no-op.
    fn on_field_changed(&self, doc: &mut SheetDoc, field: &str) {
        let _ = (doc, field);
    }

    /// The max rating of a splat-owned `+xp/spend` target, or `None` if this
    /// field is not one of the splat's traits (base XP rules then apply).
    fn xp_max(&self, field: &str, key: Option<&str>) -> Option<i32> {
        let _ = (field, key);
        None
    }

    /// The sheet fields the chargen **wizard** prompts for in its splat step,
    /// each set through the normal [`+stat`/`apply_stat`](crate::chargen_cmds)
    /// path (so chargen caps apply): Vampire `["clan", "path"]`, Mage
    /// `["affiliation", "arete"]`, Werewolf `["breed", "auspice", "tribe"]`.
    /// Empty (the default, Mortal) means the wizard skips the splat step.
    fn wizard_splat_fields(&self) -> &'static [&'static str] {
        &[]
    }

    /// The name of this splat's chargen-allocatable **power-pool** map field
    /// (Vampire `"disciplines"`, Mage `"spheres"`), or `None` (Werewolf /
    /// Mortal — no player-allocated pool). The chargen wizard uses this to
    /// route `<pool> <name>=<dots>` input to [`Splat::wizard_set_pool`].
    fn wizard_pool_field(&self) -> Option<&'static str> {
        None
    }

    /// Set one entry of the wizard power pool (a Discipline / Sphere) to
    /// `rating` dots during chargen, enforcing this splat's own rules (Mage
    /// caps a Sphere at the current Arete). Called by the chargen wizard only
    /// when [`Splat::wizard_pool_field`] is `Some`. Persists; emits no output.
    /// Default: refuse (no pool).
    fn wizard_set_pool(
        &self,
        cmd: &WodCmd,
        ctx: &mut Ctx<'_>,
        name: &str,
        rating: i32,
    ) -> Result<(), CmdError> {
        let _ = (cmd, ctx, name, rating);
        Err(CmdError::msg(
            "This template has no chargen pool to allocate.",
        ))
    }

    /// The XP cost to raise a splat-owned trait `from`→`to` on `doc` (Vampire:
    /// in-clan vs out-of-clan Discipline multipliers). Only called when
    /// [`Splat::xp_max`] claimed the field. Default: unreachable no-op.
    fn xp_cost(
        &self,
        config: &WodConfig,
        doc: &SheetDoc,
        field: &str,
        key: Option<&str>,
        from: i32,
        to: i32,
    ) -> i32 {
        let _ = (config, doc, field, key, from, to);
        0
    }
}

/// Whether `ctx`'s actor is currently the given template. The self-gate every
/// splat command uses; reads the actor's sheet `template` field (absent =
/// mortal).
#[must_use]
pub(crate) fn is_template(cmd: &WodCmd, ctx: &Ctx<'_>, template: &str) -> bool {
    cmd.read(ctx.w, ctx.actor)
        .and_then(|d| d.text("template").map(str::to_string))
        .as_deref()
        .unwrap_or("mortal")
        == template
}

// ----------------------------------------------------------------------
// Chargen contribution (consumed by `crate::chargen`)
// ----------------------------------------------------------------------

/// A splat's addition to chargen point accounting: extra pools plus the base
/// trait kinds those pools already cover.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChargenContribution {
    /// Extra priority-style pools the splat grants (Vampire: Disciplines 3,
    /// Virtues 7). Each is spent fully like an attribute pool; overflow beyond
    /// the pool costs freebies.
    pub pools: Vec<SplatPool>,
    /// Base [`TraitKind`]s whose dots one of `pools` already accounts for, so
    /// base freebie counting must skip them (Vampire: [`TraitKind::Virtue`]).
    pub freebie_exempt: Vec<TraitKind>,
}

/// One splat chargen pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplatPool {
    /// Pool id (also the ledger name shown in `+chargen/status`).
    pub id: SmolStr,
    /// Human label ("Disciplines", "Virtues").
    pub label: String,
    /// Dots granted by the pool.
    pub total: i32,
    /// Where the "spent" count is read from the sheet.
    pub source: SplatPoolSource,
    /// Freebie cost per dot spent beyond `total`.
    pub freebie_cost: i32,
}

/// How a [`SplatPool`]'s spent-dots are counted off the sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SplatPoolSource {
    /// Sum of `(value - 1).max(0)` over these scalar dot fields (Virtues:
    /// dots above the free 1 each).
    ScalarDotsAboveOne(Vec<SmolStr>),
    /// Sum of every value in this dots-map field (Disciplines).
    MapDots(SmolStr),
}

impl SplatPool {
    /// The dots spent against this pool on `doc`.
    #[must_use]
    pub fn spent(&self, doc: &SheetDoc) -> i32 {
        match &self.source {
            SplatPoolSource::ScalarDotsAboveOne(fields) => fields
                .iter()
                .map(|f| (doc.number(f).unwrap_or(1) as i32 - 1).max(0))
                .sum(),
            SplatPoolSource::MapDots(field) => doc
                .map(field)
                .map(|m| {
                    m.values()
                        .filter_map(serde_json::Value::as_i64)
                        .sum::<i64>() as i32
                })
                .unwrap_or(0),
        }
    }
}

// ----------------------------------------------------------------------
// Resolver + registries
// ----------------------------------------------------------------------

/// Resolve `enabled` template ids to their [`Splat`] implementations, skipping
/// any id with no implementation yet (phase-2 ids before they land).
#[must_use]
pub(crate) fn all_splats(enabled: &[&str]) -> Vec<Arc<dyn Splat>> {
    enabled.iter().filter_map(|id| splat_by_id(id)).collect()
}

/// The single match point from a template id to its [`Splat`]. Each splat
/// module exposes its instance behind an `Arc<dyn Splat>` (vampire directly,
/// the rest via a `template()` constructor).
fn splat_by_id(id: &str) -> Option<Arc<dyn Splat>> {
    match id {
        "mortal" => Some(Arc::new(MortalSplat)),
        "vampire" => Some(Arc::new(vampire::Vampire)),
        "mage" => Some(mage::template()),
        "werewolf" => Some(werewolf::template()),
        "changeling" => Some(changeling::template()),
        "ghoul" => Some(ghoul::template()),
        "kinfolk" => Some(kinfolk::template()),
        _ => None,
    }
}

/// Wrap schema-only [`Template`]s (e.g. a test splat) as [`Splat`]s with
/// default (no-op) chargen/command behaviour — the bridge for
/// [`WodRuleset::with_templates`](crate::WodRuleset::with_templates).
#[must_use]
pub(crate) fn schema_only_splats(templates: Vec<Box<dyn Template>>) -> Vec<Arc<dyn Splat>> {
    templates
        .into_iter()
        .map(|t| Arc::new(SchemaOnlySplat(t)) as Arc<dyn Splat>)
        .collect()
}

/// The base template — adds no sections and no splat behaviour. Occupies the
/// `mortal` slot every game ships with.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct MortalSplat;

impl Template for MortalSplat {
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

impl Splat for MortalSplat {}

/// Adapter: a schema-only [`Template`] promoted to a [`Splat`] (no chargen
/// pools, no commands). Its `id`/`display_name`/`extend_schema` delegate.
struct SchemaOnlySplat(Box<dyn Template>);

impl Template for SchemaOnlySplat {
    fn id(&self) -> &'static str {
        self.0.id()
    }
    fn display_name(&self) -> &str {
        self.0.display_name()
    }
    fn extend_schema(&self, b: SheetSchemaBuilder) -> Result<SheetSchemaBuilder, SchemaError> {
        self.0.extend_schema(b)
    }
}

impl Splat for SchemaOnlySplat {}

/// The per-template composed schemas + selection by the sheet's `template`
/// field. Built once at boot; the dispatch backbone of M6b.
pub(crate) struct TemplateSchemas {
    /// Every enabled template id (the `template` enum options), in order.
    ids: Vec<SmolStr>,
    /// template id -> composed schema (base + that template only).
    by_id: HashMap<SmolStr, Arc<SheetSchema>>,
    /// The fallback template id (first enabled).
    default_id: SmolStr,
}

impl TemplateSchemas {
    /// Build one schema per splat: base (enum = all ids) + that splat's
    /// sections.
    fn build(ruleset: &str, splats: &[Arc<dyn Splat>]) -> TemplateSchemas {
        let ids: Vec<SmolStr> = splats.iter().map(|s| SmolStr::new(s.id())).collect();
        let mut by_id = HashMap::new();
        for s in splats {
            let builder = base_builder(ruleset, &ids);
            let schema = s.extend_schema(builder).unwrap_or_else(|e| {
                panic!("template {:?} failed to extend the sheet: {e}", s.id())
            });
            by_id.insert(SmolStr::new(s.id()), Arc::new(schema.build()));
        }
        let default_id = ids
            .first()
            .cloned()
            .unwrap_or_else(|| SmolStr::new("mortal"));
        TemplateSchemas {
            ids,
            by_id,
            default_id,
        }
    }

    /// Every enabled template id (the `template` enum options).
    #[must_use]
    pub fn ids(&self) -> &[SmolStr] {
        &self.ids
    }

    /// The schema for template `id`, falling back to the default template.
    #[must_use]
    pub fn schema_for_id(&self, id: &str) -> Arc<SheetSchema> {
        self.by_id
            .get(id)
            .or_else(|| self.by_id.get(self.default_id.as_str()))
            .cloned()
            .expect("default template schema present")
    }

    /// The active schema for `doc`, selected by its `template` field.
    #[must_use]
    pub fn schema_for_doc(&self, doc: &SheetDoc) -> Arc<SheetSchema> {
        let id = doc.text("template").unwrap_or(&self.default_id);
        self.schema_for_id(id)
    }

    /// The default (fallback) template's schema — the one a target with no
    /// sheet, and reads that never validate, use.
    #[must_use]
    pub fn default_schema(&self) -> Arc<SheetSchema> {
        self.schema_for_id(&self.default_id)
    }

    /// The default template's schema by reference (for the framework's
    /// single-schema `Ruleset::sheet_schema` hook).
    #[must_use]
    pub fn default_schema_ref(&self) -> &SheetSchema {
        self.by_id
            .get(self.default_id.as_str())
            .expect("default template schema present")
    }
}

/// The enabled splats, dispatched by template id.
pub(crate) struct SplatRegistry {
    by_id: HashMap<SmolStr, Arc<dyn Splat>>,
    order: Vec<SmolStr>,
    default_id: SmolStr,
}

impl SplatRegistry {
    fn build(splats: Vec<Arc<dyn Splat>>) -> SplatRegistry {
        let order: Vec<SmolStr> = splats.iter().map(|s| SmolStr::new(s.id())).collect();
        let default_id = order
            .first()
            .cloned()
            .unwrap_or_else(|| SmolStr::new("mortal"));
        let by_id = splats
            .into_iter()
            .map(|s| (SmolStr::new(s.id()), s))
            .collect();
        SplatRegistry {
            by_id,
            order,
            default_id,
        }
    }

    /// The splat for template `id`, falling back to the default template.
    #[must_use]
    pub fn get(&self, id: &str) -> Arc<dyn Splat> {
        self.by_id
            .get(id)
            .or_else(|| self.by_id.get(self.default_id.as_str()))
            .cloned()
            .expect("default splat present")
    }

    /// The splat active for `doc`, selected by its `template` field.
    #[must_use]
    pub fn for_doc(&self, doc: &SheetDoc) -> Arc<dyn Splat> {
        self.get(doc.text("template").unwrap_or(&self.default_id))
    }

    /// Every enabled splat, in declaration order (for boot-time command
    /// registration).
    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Splat>> {
        self.order.iter().map(move |id| &self.by_id[id])
    }
}

/// Both dispatch tables built from one splat set (schemas share the enum id
/// list), plus the splat set for command registration.
pub(crate) struct Dispatch {
    /// Per-template composed schemas.
    pub schemas: Arc<TemplateSchemas>,
    /// The splat behaviours.
    pub splats: Arc<SplatRegistry>,
}

impl Dispatch {
    /// Build the dispatch tables for `ruleset` from `splats`.
    #[must_use]
    pub fn build(ruleset: &str, splats: Vec<Arc<dyn Splat>>) -> Dispatch {
        let schemas = Arc::new(TemplateSchemas::build(ruleset, &splats));
        let registry = Arc::new(SplatRegistry::build(splats));
        Dispatch {
            schemas,
            splats: registry,
        }
    }

    /// Read `obj`'s sheet schema-agnostically (reads never validate).
    #[must_use]
    pub fn read(&self, w: &mush_core::World, obj: mush_core::ObjId) -> Option<SheetDoc> {
        SheetStore::new(self.schemas.default_schema()).get(w, obj)
    }

    /// A validating store over a sheet's active (template-selected) schema.
    #[must_use]
    pub fn store_for_doc(&self, doc: &SheetDoc) -> SheetStore {
        SheetStore::new(self.schemas.schema_for_doc(doc))
    }
}

// ----------------------------------------------------------------------
// Re-templating a sheet
// ----------------------------------------------------------------------

/// Re-shape `doc` for a new template: keep every base/shared value the target
/// schema still defines, drop the old splat's fields (they would fail
/// validation as unknown), fill the new splat's fields with defaults, and stamp
/// the `template` field. Used when a character picks (or is set to) a template.
pub(crate) fn retemplate(doc: &mut SheetDoc, target: &SheetSchema, new_template: &str) {
    let mut data = target.default_data();
    if let (Some(dst), Some(src)) = (data.as_object_mut(), doc.data.as_object()) {
        for (k, v) in src {
            if target.field(k).is_some() {
                dst.insert(k.clone(), v.clone());
            }
        }
        dst.insert("template".to_string(), serde_json::json!(new_template));
    }
    doc.data = data;
    doc.schema_version = target.version;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_template_schemas_carry_only_their_splat() {
        let d = Dispatch::build("wod", all_splats(&["mortal", "vampire"]));
        let mortal = d.schemas.schema_for_id("mortal");
        let vampire = d.schemas.schema_for_id("vampire");

        // Base is shared.
        assert!(mortal.field("strength").is_some());
        assert!(vampire.field("strength").is_some());

        // Vampire-only sections/fields are absent from the mortal schema.
        assert!(vampire.field("disciplines").is_some());
        assert!(vampire.field("bloodpool").is_some());
        assert!(vampire.field("generation").is_some());
        assert!(mortal.field("disciplines").is_none());
        assert!(mortal.field("bloodpool").is_none());
        assert!(mortal.field("generation").is_none());

        // Both list every enabled template in the `template` enum.
        for schema in [&mortal, &vampire] {
            let opts = match &schema.field("template").unwrap().ty {
                mush_ruleset::schema::FieldType::Enum { options } => options.clone(),
                _ => panic!("template is not an enum"),
            };
            assert!(opts.iter().any(|o| o == "mortal"));
            assert!(opts.iter().any(|o| o == "vampire"));
        }

        // Default docs of each schema validate.
        assert!(mush_ruleset::validate(&mortal, &mortal.default_data()).is_ok());
        assert!(mush_ruleset::validate(&vampire, &vampire.default_data()).is_ok());
    }

    #[test]
    fn schema_selected_by_template_field() {
        let d = Dispatch::build("wod", all_splats(&["mortal", "vampire"]));
        let mut doc = SheetDoc::new_default(&d.schemas.default_schema());
        // No template field set → default (mortal).
        assert!(
            d.schemas
                .schema_for_doc(&doc)
                .field("disciplines")
                .is_none()
        );
        doc.set_text("template", "vampire");
        assert!(
            d.schemas
                .schema_for_doc(&doc)
                .field("disciplines")
                .is_some()
        );
    }

    #[test]
    fn retemplate_keeps_base_drops_old_splat() {
        let d = Dispatch::build("wod", all_splats(&["mortal", "vampire"]));
        let mut doc = SheetDoc::new_default(&d.schemas.default_schema());
        doc.set_number("strength", 4);
        // Mortal → vampire: strength kept, vampire fields appear.
        let vamp = d.schemas.schema_for_id("vampire");
        retemplate(&mut doc, &vamp, "vampire");
        assert_eq!(doc.number("strength"), Some(4));
        assert_eq!(doc.text("template"), Some("vampire"));
        assert!(doc.get("bloodpool").is_some());
        assert!(mush_ruleset::validate(&vamp, &doc.data).is_ok());
        // Vampire → mortal: discipline data dropped so it revalidates.
        doc.map_set("disciplines", "Celerity", serde_json::json!(2));
        let mortal = d.schemas.schema_for_id("mortal");
        retemplate(&mut doc, &mortal, "mortal");
        assert!(doc.get("disciplines").is_none());
        assert_eq!(doc.number("strength"), Some(4));
        assert!(mush_ruleset::validate(&mortal, &doc.data).is_ok());
    }
}
