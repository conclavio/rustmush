//! # Mage: The Ascension 20th Anniversary (M20) — the phase-2 splat
//!
//! A second worked implementation of the [`Splat`] contract (see the module
//! docs in [`crate::splats`]), mirroring [`Vampire`](crate::splats::vampire)
//! structurally so the integration merge is trivial. The Awakened are
//! **sheet-tracked, pool-rolled**: Spheres are a name→dots map, Arete is the
//! casting pool rolled through [`roll_expr`](crate::roll::roll_expr), and
//! Quintessence / Paradox are splat pools with spend/gain commands — no bespoke
//! per-effect automation.
//!
//! What this module adds on top of the base oWoD sheet:
//!
//! - an **Affiliation** ([`TRADITIONS`], the nine Traditions plus Technocracy /
//!   Orphan) driving a derived **affinity Sphere** ([`affinity_for`]);
//! - **Arete** (0..10 dots — the magic rating and the `+cast` pool), starting
//!   at 1 and capped at 3 at chargen;
//! - the nine **Spheres** ([`SPHERES`]) as a name→0..5 map, each rating capped
//!   by Arete, with in-affinity vs out-of-affinity XP costs;
//! - **Quintessence** (Avatar-rated) and **Paradox** pools with `+quint` /
//!   `+paradox` commands;
//! - a free-form **Resonance** map (flavour);
//! - commands `+sphere`/`+spheres`, `+arete`, `+cast`, `+quint`, `+paradox`,
//!   `+resonance`, all self-gating on the actor being Awakened.

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ModError, Registrar, Role, perm};
use mush_ruleset::SheetDoc;
use mush_ruleset::schema::{
    FieldDef, FieldType, SchemaError, SectionLayout, SheetSchemaBuilder, SheetSection,
};
use smol_str::SmolStr;

use crate::WodCmd;
use crate::config::{TraitKind, WodConfig};
use crate::schema::Template;
use crate::splats::{ChargenContribution, Splat, SplatPool, SplatPoolSource, is_template};
use crate::state::WodState;

/// The nine Spheres (canonical spelling; the map may hold only these).
pub const SPHERES: &[&str] = &[
    "Correspondence",
    "Entropy",
    "Forces",
    "Life",
    "Matter",
    "Mind",
    "Prime",
    "Spirit",
    "Time",
];

/// The canonical Sphere name for a token (case-insensitive), or `None`.
#[must_use]
pub fn canonical_sphere(token: &str) -> Option<&'static str> {
    SPHERES
        .iter()
        .copied()
        .find(|s| s.eq_ignore_ascii_case(token))
}

/// One affiliation's crib entry: id, label, and the affinity Sphere its mages
/// favour (empty for the "other" affiliations, whose affinity varies).
pub struct TraditionDef {
    /// The `affiliation` enum id (snake_case).
    pub id: &'static str,
    /// Human label.
    pub label: &'static str,
    /// The affinity Sphere (canonical name), or `""` when it varies.
    pub affinity: &'static str,
}

/// The nine Traditions plus Technocracy / Orphan as "other" affiliations. The
/// affinity table is the sane per-Tradition default (Order of Hermes → Forces
/// by convention); Technocracy / Orphan leave the affinity to the player/ST.
pub const TRADITIONS: &[TraditionDef] = &[
    TraditionDef {
        id: "akashic_brotherhood",
        label: "Akashic Brotherhood",
        affinity: "Mind",
    },
    TraditionDef {
        id: "celestial_chorus",
        label: "Celestial Chorus",
        affinity: "Prime",
    },
    TraditionDef {
        id: "cult_of_ecstasy",
        label: "Cult of Ecstasy",
        affinity: "Time",
    },
    TraditionDef {
        id: "dreamspeakers",
        label: "Dreamspeakers",
        affinity: "Spirit",
    },
    TraditionDef {
        id: "euthanatos",
        label: "Euthanatos",
        affinity: "Entropy",
    },
    TraditionDef {
        id: "order_of_hermes",
        label: "Order of Hermes",
        affinity: "Forces",
    },
    TraditionDef {
        id: "sons_of_ether",
        label: "Sons of Ether",
        affinity: "Matter",
    },
    TraditionDef {
        id: "verbena",
        label: "Verbena",
        affinity: "Life",
    },
    TraditionDef {
        id: "virtual_adepts",
        label: "Virtual Adepts",
        affinity: "Correspondence",
    },
    TraditionDef {
        id: "technocracy",
        label: "Technocracy",
        affinity: "",
    },
    TraditionDef {
        id: "orphan",
        label: "Orphan",
        affinity: "",
    },
];

/// The affiliation definition for an id (case-insensitive).
#[must_use]
pub fn affiliation_def(id: &str) -> Option<&'static TraditionDef> {
    TRADITIONS.iter().find(|t| t.id.eq_ignore_ascii_case(id))
}

/// The affinity Sphere for an affiliation id (`""` when it varies / unknown).
#[must_use]
pub fn affinity_for(affiliation: &str) -> &'static str {
    affiliation_def(affiliation).map_or("", |t| t.affinity)
}

/// The affiliation enum options (the ids).
fn affiliation_ids() -> Vec<SmolStr> {
    TRADITIONS.iter().map(|t| SmolStr::new(t.id)).collect()
}

/// Whether `sphere` is the sheet's affinity Sphere (case-insensitive).
#[must_use]
pub fn is_affinity(doc: &SheetDoc, sphere: &str) -> bool {
    doc.text("affinity")
        .is_some_and(|a| !a.is_empty() && a.eq_ignore_ascii_case(sphere))
}

/// Total dots of a named background on a sheet.
fn background_dots(doc: &SheetDoc, name: &str) -> i32 {
    doc.map("backgrounds")
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .and_then(|(_, v)| v.as_i64())
        })
        .unwrap_or(0) as i32
}

/// The XP cost to raise a Sphere `from`→`to`: a new Sphere (0→1) costs 10;
/// further dots cost current × 7 (affinity) or × 8 (other) — the M20 in-/
/// out-of-affinity split, mirroring the Discipline in-clan rule.
#[must_use]
pub fn sphere_raise_cost(doc: &SheetDoc, sphere: &str, from: i32, to: i32) -> i32 {
    let mult = if is_affinity(doc, sphere) { 7 } else { 8 };
    (from.max(0)..to)
        .map(|r| if r <= 0 { 10 } else { r * mult })
        .sum()
}

/// The XP cost to raise Arete `from`→`to`: current × 8 per dot (M20).
#[must_use]
pub fn arete_raise_cost(from: i32, to: i32) -> i32 {
    (from.max(1)..to).map(|r| r * 8).sum()
}

// ----------------------------------------------------------------------
// The Mage splat
// ----------------------------------------------------------------------

/// The M20 splat.
#[derive(Debug, Default, Clone, Copy)]
pub struct Mage;

/// This splat behind an `Arc<dyn Splat>`, for the resolver arm the integration
/// agent adds (`"mage" => Some(mage::template())`).
#[must_use]
pub(crate) fn template() -> Arc<dyn Splat> {
    Arc::new(Mage)
}

impl Template for Mage {
    fn id(&self) -> &'static str {
        "mage"
    }

    fn display_name(&self) -> &str {
        "Mage"
    }

    fn extend_schema(&self, b: SheetSchemaBuilder) -> Result<SheetSchemaBuilder, SchemaError> {
        // Arcane scalars.
        let arcane = SheetSection::new("arcane", "Arcane", SectionLayout::List)
            .field(
                FieldDef::enumeration("affiliation", "Affiliation", affiliation_ids())
                    .with_default(serde_json::json!("akashic_brotherhood")),
            )
            .field(
                FieldDef::text("affinity", "Affinity Sphere", 20)
                    .with_default(serde_json::json!("Mind")),
            )
            // Arete is Dots so `+cast`/`+roll arete` resolves it as a pool.
            .field(FieldDef::dots("arete", "Arete", 0, 10).with_default(serde_json::json!(1)))
            .field(FieldDef::number("quintessence", "Quintessence", 0, 100))
            .field(FieldDef::number("paradox", "Paradox", 0, 100));
        let b = b.section(arcane)?;
        // Spheres map (name→0..5).
        let spheres = SheetSection::new("spheres", "Spheres", SectionLayout::Map).field(
            FieldDef::map("spheres", "Spheres", FieldType::Dots { min: 0, max: 5 }),
        );
        let b = b.section(spheres)?;
        // Resonance map (free-form flavour traits).
        let resonance = SheetSection::new("resonance", "Resonance", SectionLayout::Map).field(
            FieldDef::map("resonance", "Resonance", FieldType::Dots { min: 0, max: 5 }),
        );
        b.section(resonance)
    }
}

impl Splat for Mage {
    fn register_commands(&self, reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
        register_commands(reg, cmd)
    }

    fn chargen_contribution(&self, _config: &WodConfig, _doc: &SheetDoc) -> ChargenContribution {
        ChargenContribution {
            pools: vec![
                SplatPool {
                    id: SmolStr::new("spheres"),
                    label: "Spheres".to_string(),
                    total: 5,
                    source: SplatPoolSource::MapDots(SmolStr::new("spheres")),
                    freebie_cost: 7,
                },
                // Arete above the free 1st dot: no free pool, 4 freebies each.
                SplatPool {
                    id: SmolStr::new("arete"),
                    label: "Arete".to_string(),
                    total: 0,
                    source: SplatPoolSource::ScalarDotsAboveOne(vec![SmolStr::new("arete")]),
                    freebie_cost: 4,
                },
            ],
            // Willpower is derived to a fixed 5 (not the config's Courage
            // baseline), so exempt it from the base Willpower freebie line —
            // its five free dots must not be charged. (Requires the matching
            // guard in `chargen::freebie_spent`.)
            freebie_exempt: vec![TraitKind::Willpower],
        }
    }

    fn finalize_starting(&self, _config: &WodConfig, doc: &mut SheetDoc) {
        // Arete floors at 1.
        let arete = doc.number("arete").unwrap_or(1).max(1);
        doc.set_number("arete", arete);

        // Willpower for mages is a flat 5 (overrides the base Courage baseline).
        let wp = doc.number("willpower").unwrap_or(1).max(5);
        doc.set_number("willpower", wp);
        doc.set_number("willpower_temp", wp);

        // Quintessence starts at the Avatar Background rating.
        let avatar = background_dots(doc, "Avatar");
        doc.set_number("quintessence", i64::from(avatar));

        // Affinity Sphere from the affiliation.
        let affinity = affinity_for(doc.text("affiliation").unwrap_or(""));
        doc.set_text("affinity", affinity);
    }

    fn chargen_locked_fields(&self) -> &'static [&'static str] {
        // Spheres are set (Arete-capped) with `+sphere`; the rest are derived.
        &[
            "spheres",
            "willpower",
            "quintessence",
            "paradox",
            "affinity",
        ]
    }

    fn chargen_max(&self, field: &str) -> Option<i32> {
        // M20 caps a starting mage's Arete at 3.
        field.eq_ignore_ascii_case("arete").then_some(3)
    }

    fn wizard_splat_fields(&self) -> &'static [&'static str] {
        // Affiliation (derives the affinity Sphere) and Arete (chargen-capped
        // at 3); affinity is derived, Spheres are the wizard pool below.
        &["affiliation", "arete"]
    }

    fn wizard_pool_field(&self) -> Option<&'static str> {
        Some("spheres")
    }

    fn wizard_set_pool(
        &self,
        cmd: &WodCmd,
        ctx: &mut Ctx<'_>,
        name: &str,
        rating: i32,
    ) -> Result<(), CmdError> {
        // Spheres are Arete-capped and chargen-locked from +stat, so route
        // through the same validated setter `+sphere` uses.
        apply_sphere(cmd, ctx, name, i64::from(rating)).map(|_| ())
    }

    fn xp_target_cap(&self, doc: &SheetDoc, field: &str, _key: Option<&str>) -> Option<i32> {
        // A Sphere may never exceed the caster's current Arete — the same rule
        // `+sphere` enforces, applied to the `+xp/spend` path so a spend cannot
        // sneak a Sphere above Arete on approval.
        field
            .eq_ignore_ascii_case("spheres")
            .then(|| doc.number("arete").unwrap_or(1) as i32)
    }

    fn on_field_changed(&self, doc: &mut SheetDoc, field: &str) {
        if field.eq_ignore_ascii_case("affiliation") {
            let affinity = affinity_for(doc.text("affiliation").unwrap_or(""));
            doc.set_text("affinity", affinity);
        }
    }

    fn xp_max(&self, field: &str, key: Option<&str>) -> Option<i32> {
        if field.eq_ignore_ascii_case("spheres") && key.is_some() {
            Some(5)
        } else if field.eq_ignore_ascii_case("arete") {
            Some(10)
        } else {
            None
        }
    }

    fn xp_cost(
        &self,
        _config: &WodConfig,
        doc: &SheetDoc,
        field: &str,
        key: Option<&str>,
        from: i32,
        to: i32,
    ) -> i32 {
        if field.eq_ignore_ascii_case("arete") {
            arete_raise_cost(from, to)
        } else {
            sphere_raise_cost(doc, key.unwrap_or(""), from, to)
        }
    }
}

// ----------------------------------------------------------------------
// Commands (all self-gate on the actor being Awakened)
// ----------------------------------------------------------------------

/// Refuse politely if the actor is not a mage.
fn require_mage(cmd: &WodCmd, ctx: &Ctx<'_>) -> Result<SheetDoc, CmdError> {
    if !is_template(cmd, ctx, "mage") {
        return Err(CmdError::msg("You are not Awakened."));
    }
    cmd.read(ctx.w, ctx.actor)
        .ok_or_else(|| CmdError::msg("You have no sheet."))
}

/// `+sphere` / `+spheres` (list) or `+sphere <name>=<0..5>` (rate, Arete-capped).
fn sphere_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().trim().to_string();
    if lhs.is_empty() {
        let doc = require_mage(cmd, ctx)?;
        return sphere_list(ctx, &doc);
    }
    let rating: i64 = ctx
        .args
        .rhs()
        .and_then(|r| r.trim().parse().ok())
        .ok_or_else(|| CmdError::msg("Usage: +sphere <name>=<0..5>"))?;
    let (sphere, aff) = apply_sphere(cmd, ctx, &lhs, rating)?;
    ctx.out
        .sys_to_actor(ctx.actor, "wod", format!("{sphere} set to {rating}{aff}."));
    Ok(())
}

/// Set one Sphere to `rating` for the acting mage, enforcing the Arete cap and
/// the chargen-or-staff gate — the core of `+sphere`, shared with the chargen
/// wizard. Returns the canonical Sphere name and an `" (affinity)"` suffix (or
/// `""`). Persists; emits no output.
pub(crate) fn apply_sphere(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    sphere_input: &str,
    rating: i64,
) -> Result<(&'static str, &'static str), CmdError> {
    let mut doc = require_mage(cmd, ctx)?;
    let sphere = canonical_sphere(sphere_input).ok_or_else(|| {
        CmdError::msg(format!(
            "Unknown Sphere {sphere_input:?}. The nine are: {}.",
            SPHERES.join(", ")
        ))
    })?;
    if !(0..=5).contains(&rating) {
        return Err(CmdError::msg("A Sphere is rated 0 to 5."));
    }
    let arete = doc.number("arete").unwrap_or(1);
    if rating > arete {
        return Err(CmdError::msg(format!(
            "A Sphere cannot exceed your Arete ({arete}). Raise Arete first."
        )));
    }
    // Setting a Sphere directly is a chargen (or staff) action. Once the sheet
    // is approved, Spheres rise only through the costed `+xp/spend` path — the
    // Arete-cap check above still runs first so an over-Arete set is reported as
    // such regardless of chargen state.
    let in_chargen = ctx
        .state::<WodState>()
        .is_some_and(|st| st.in_chargen(ctx.actor));
    if !in_chargen && !perm::has_role(ctx.w, ctx.actor, Role::Royalty) {
        return Err(CmdError::msg(
            "Your sheet is set; raise Spheres with +xp/spend spheres:<name>=<n>.",
        ));
    }
    let store = cmd.store_for_doc(&doc);
    store
        .set_map_entry(&mut doc, "spheres", sphere, serde_json::json!(rating))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    store
        .put_by(ctx.w, ctx.actor, &doc, ctx.actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    let aff = if is_affinity(&doc, sphere) {
        " (affinity)"
    } else {
        ""
    };
    Ok((sphere, aff))
}

/// List the nine Spheres and the current Arete.
fn sphere_list(ctx: &mut Ctx<'_>, doc: &SheetDoc) -> Result<(), CmdError> {
    let arete = doc.number("arete").unwrap_or(1);
    ctx.out
        .sys_to_actor(ctx.actor, "wod", format!("Spheres (Arete {arete}):"));
    let map = doc.map("spheres");
    for s in SPHERES {
        let n = map
            .and_then(|m| m.get(*s))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let aff = if is_affinity(doc, s) {
            " (affinity)"
        } else {
            ""
        };
        ctx.out
            .sys_to_actor(ctx.actor, "wod", format!("  {s}: {n}{aff}"));
    }
    Ok(())
}

/// `+arete` — show the magic rating / casting pool.
fn arete_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let doc = require_mage(cmd, ctx)?;
    let arete = doc.number("arete").unwrap_or(1);
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!("Arete {arete}: your Awakened potency and the pool you roll to cast (+cast)."),
    );
    Ok(())
}

/// `+cast <description> [vs <diff>]` (`/vulgar` flags Paradox risk) — roll Arete.
fn cast_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let _doc = require_mage(cmd, ctx)?;
    let (desc, diff) = split_vs(ctx.args.raw());
    let vulgar = ctx.switches.has("vulgar");
    let label = if vulgar {
        "vulgar magick"
    } else {
        "coincidental magick"
    };
    // Casting is an action, so the wound penalty applies to the Arete pool
    // (matching Discipline and Gift rolls).
    let out = crate::roll::roll_expr(cmd, ctx, ctx.actor, "arete", diff.unwrap_or(6), true, label)?;
    let verdict = if out.is_botch() {
        "The Tapestry recoils — a Paradox backlash (the Storyteller adjudicates).".to_string()
    } else if out.successes() > 0 {
        format!(
            "The working takes shape with {} success(es) of magnitude.",
            out.successes()
        )
    } else {
        "The magick fails to gather.".to_string()
    };
    if !desc.trim().is_empty() {
        ctx.out
            .sys_to_actor(ctx.actor, "wod", format!("Effect: {}", desc.trim()));
    }
    ctx.out.sys_to_actor(ctx.actor, "wod", verdict);
    if vulgar {
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            "Vulgar magick invites Paradox; the Storyteller may assign it (see +paradox/gain).",
        );
    }
    Ok(())
}

/// `+quint` / `+quint/gain <n>` / `+quint/spend <n>` — Quintessence (Avatar-capped).
fn quint_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let mut doc = require_mage(cmd, ctx)?;
    let cur = doc.number("quintessence").unwrap_or(0) as i32;
    let cap = background_dots(&doc, "Avatar");

    if ctx.switches.has("gain") {
        let n = arg_count(ctx, 0)?;
        if n < 1 {
            return Err(CmdError::msg("Gain how much Quintessence?"));
        }
        let next = (cur + n).min(cap);
        let gained = (next - cur).max(0);
        set_quint(cmd, ctx, &mut doc, next)?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!("You channel {gained} Quintessence. Quintessence {next}/{cap} (Avatar cap)."),
        );
        return Ok(());
    }
    if ctx.switches.has("spend") {
        let n = arg_count(ctx, 0)?;
        if n < 1 {
            return Err(CmdError::msg("Spend how much Quintessence?"));
        }
        if n > cur {
            return Err(CmdError::msg(format!(
                "You have only {cur} Quintessence; you cannot spend {n}."
            )));
        }
        let next = cur - n;
        set_quint(cmd, ctx, &mut doc, next)?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!("You spend {n} Quintessence. Quintessence {next}/{cap} (Avatar cap)."),
        );
        return Ok(());
    }
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!("Quintessence: {cur} (Avatar cap {cap})."),
    );
    Ok(())
}

/// `+paradox` / `+paradox/gain <n>` / `+paradox/spend [<n>]` (burn off).
fn paradox_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let mut doc = require_mage(cmd, ctx)?;
    let cur = doc.number("paradox").unwrap_or(0) as i32;

    if ctx.switches.has("gain") {
        let n = arg_count(ctx, 0)?;
        if n < 1 {
            return Err(CmdError::msg("Gain how much Paradox?"));
        }
        let next = (cur + n).min(100);
        set_paradox(cmd, ctx, &mut doc, next)?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!(
                "Paradox rises by {}. Paradox now {next}. (Vulgar magick and misused Quintessence feed it.)",
                next - cur
            ),
        );
        return Ok(());
    }
    if ctx.switches.has("spend") || ctx.switches.has("burn") {
        // Default: burn off everything.
        let n = arg_count(ctx, cur)?;
        if n < 1 {
            return Err(CmdError::msg("Burn off how much Paradox?"));
        }
        let next = (cur - n).max(0);
        set_paradox(cmd, ctx, &mut doc, next)?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!(
                "You work off {} Paradox through quiet and rest. Paradox now {next}.",
                cur - next
            ),
        );
        return Ok(());
    }
    ctx.out
        .sys_to_actor(ctx.actor, "wod", format!("Paradox: {cur}."));
    Ok(())
}

/// `+resonance` (list) or `+resonance <trait>=<0..5>` (set; 0 clears).
fn resonance_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let mut doc = require_mage(cmd, ctx)?;
    let lhs = ctx.args.lhs().trim().to_string();
    if lhs.is_empty() {
        ctx.out.sys_to_actor(ctx.actor, "wod", "Resonance:");
        let mut any = false;
        if let Some(m) = doc.map("resonance") {
            let mut entries: Vec<(&String, i64)> = m
                .iter()
                .filter_map(|(k, v)| v.as_i64().map(|n| (k, n)))
                .filter(|(_, n)| *n > 0)
                .collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            for (k, n) in entries {
                any = true;
                ctx.out
                    .sys_to_actor(ctx.actor, "wod", format!("  {k}: {n}"));
            }
        }
        if !any {
            ctx.out.sys_to_actor(ctx.actor, "wod", "  (none)");
        }
        return Ok(());
    }
    let rating: i64 = ctx
        .args
        .rhs()
        .and_then(|r| r.trim().parse().ok())
        .ok_or_else(|| CmdError::msg("Usage: +resonance <trait>=<0..5>"))?;
    if !(0..=5).contains(&rating) {
        return Err(CmdError::msg("Resonance is rated 0 to 5."));
    }
    let store = cmd.store_for_doc(&doc);
    if rating == 0 {
        doc.map_remove("resonance", &lhs);
        store
            .put_by(ctx.w, ctx.actor, &doc, ctx.actor)
            .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
        ctx.out
            .sys_to_actor(ctx.actor, "wod", format!("Resonance {lhs:?} cleared."));
        return Ok(());
    }
    store
        .set_map_entry(&mut doc, "resonance", &lhs, serde_json::json!(rating))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    store
        .put_by(ctx.w, ctx.actor, &doc, ctx.actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!("Resonance {lhs:?} set to {rating}."),
    );
    Ok(())
}

// ----------------------------------------------------------------------
// helpers
// ----------------------------------------------------------------------

/// Validate + persist a new Quintessence value.
fn set_quint(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    doc: &mut SheetDoc,
    value: i32,
) -> Result<(), CmdError> {
    let store = cmd.store_for_doc(doc);
    store
        .set_field(doc, "quintessence", serde_json::json!(value.max(0)))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    store
        .put_by(ctx.w, ctx.actor, doc, ctx.actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    Ok(())
}

/// Validate + persist a new Paradox value.
fn set_paradox(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    doc: &mut SheetDoc,
    value: i32,
) -> Result<(), CmdError> {
    let store = cmd.store_for_doc(doc);
    store
        .set_field(doc, "paradox", serde_json::json!(value.max(0)))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    store
        .put_by(ctx.w, ctx.actor, doc, ctx.actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    Ok(())
}

/// Parse a bare integer argument (falling back to `default` when empty).
fn arg_count(ctx: &Ctx<'_>, default: i32) -> Result<i32, CmdError> {
    let raw = ctx.args.raw().trim();
    if raw.is_empty() {
        return Ok(default);
    }
    raw.split_whitespace()
        .next()
        .unwrap_or("")
        .parse::<i32>()
        .map_err(|_| CmdError::msg(format!("Not a number: {raw:?}")))
}

/// Split a raw arg into `(expr, Some(diff))` on a trailing `vs <n>` / `diff <n>`.
fn split_vs(raw: &str) -> (String, Option<u8>) {
    let lower = raw.to_ascii_lowercase();
    for kw in [" vs ", " diff "] {
        if let Some(idx) = lower.rfind(kw) {
            let (head, tail) = raw.split_at(idx);
            let num = tail[kw.len()..].trim();
            if let Ok(n) = num.parse::<u8>() {
                return (head.trim().to_string(), Some(n));
            }
        }
    }
    (raw.trim().to_string(), None)
}

fn fmt_errs(errs: &[mush_ruleset::SheetError]) -> String {
    errs.iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Register the Awakened command set.
pub(crate) fn register_commands(reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
    let sphere_help = "+sphere  --  List your nine Spheres and Arete.\n\
        +sphere <name>=<0..5>  --  Set a Sphere (capped by your Arete; raise Arete first \
        to go higher). You are not Awakened? The command refuses.";
    for name in ["+sphere", "+spheres"] {
        reg.command(
            CommandSpec {
                name: name.into(),
                shape: ArgShape::EqPair,
                help: sphere_help.into(),
                ..CommandSpec::default()
            },
            {
                let cmd = Arc::clone(&cmd);
                Arc::new(move |ctx: &mut Ctx<'_>| sphere_cmd(&cmd, ctx))
            },
        )?;
    }
    reg.command(
        CommandSpec {
            name: "+arete".into(),
            shape: ArgShape::Raw,
            help: "+arete  --  Show your Arete (magic rating and casting pool).".into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| arete_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+cast".into(),
            switches: vec!["vulgar".into()],
            shape: ArgShape::Raw,
            help: "+cast <description> [vs <diff>]  --  Roll your Arete to work an effect \
                   (successes = magnitude; the effect is ST/player adjudicated).\n\
                   +cast/vulgar <description> [vs <diff>]  --  Flag vulgar magick (Paradox risk)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| cast_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+quint".into(),
            switches: vec!["gain".into(), "spend".into()],
            shape: ArgShape::Raw,
            help: "+quint  --  Show your Quintessence.\n\
                   +quint/gain <n>  --  Channel Quintessence (up to your Avatar rating).\n\
                   +quint/spend <n>  --  Spend Quintessence (powering effects, ST-adjudicated)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| quint_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+paradox".into(),
            switches: vec!["gain".into(), "spend".into(), "burn".into()],
            shape: ArgShape::Raw,
            help: "+paradox  --  Show your Paradox.\n\
                   +paradox/gain <n>  --  Accrue Paradox (vulgar magick; ST-assigned).\n\
                   +paradox/spend [<n>]  --  Work off Paradox through quiet/rest (default: all)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| paradox_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+resonance".into(),
            shape: ArgShape::EqPair,
            help: "+resonance  --  List your Resonance traits.\n\
                   +resonance <trait>=<0..5>  --  Set a Resonance trait (0 clears it)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| resonance_cmd(&cmd, ctx))
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::splats::{Dispatch, MortalSplat};

    /// Build a dispatch over `[mortal, mage]` without leaning on the resolver
    /// arm (which the integration agent wires up separately).
    fn dispatch() -> Dispatch {
        let splats: Vec<Arc<dyn Splat>> = vec![Arc::new(MortalSplat), Arc::new(Mage)];
        Dispatch::build("wod", splats)
    }

    fn mage_doc() -> SheetDoc {
        let d = dispatch();
        let mut doc = SheetDoc::new_default(&d.schemas.schema_for_id("mage"));
        doc.set_text("template", "mage");
        doc
    }

    #[test]
    fn mage_schema_has_arcane_not_kindred() {
        let d = dispatch();
        let mage = d.schemas.schema_for_id("mage");
        let mortal = d.schemas.schema_for_id("mortal");

        // Base is shared.
        assert!(mage.field("strength").is_some());
        // Mage sections/fields present.
        assert!(mage.field("arete").is_some());
        assert!(mage.field("spheres").is_some());
        assert!(mage.field("quintessence").is_some());
        assert!(mage.field("paradox").is_some());
        assert!(mage.field("affiliation").is_some());
        // No vampire fields on a mage, and no mage fields on a mortal.
        assert!(mage.field("disciplines").is_none());
        assert!(mage.field("bloodpool").is_none());
        assert!(mage.field("generation").is_none());
        assert!(mortal.field("spheres").is_none());
        assert!(mortal.field("arete").is_none());

        // Default docs validate.
        assert!(mush_ruleset::validate(&mage, &mage.default_data()).is_ok());
    }

    #[test]
    fn affinity_table_and_ids() {
        assert_eq!(affinity_for("order_of_hermes"), "Forces");
        assert_eq!(affinity_for("verbena"), "Life");
        assert_eq!(affinity_for("virtual_adepts"), "Correspondence");
        assert_eq!(affinity_for("technocracy"), "");
        assert_eq!(affinity_for("nope"), "");
        // Eleven affiliations; the nine Traditions plus two "other".
        assert_eq!(TRADITIONS.len(), 11);
        assert_eq!(SPHERES.len(), 9);
        assert_eq!(canonical_sphere("forces"), Some("Forces"));
        assert_eq!(canonical_sphere("Prime"), Some("Prime"));
        assert_eq!(canonical_sphere("qi"), None);
    }

    #[test]
    fn affiliation_change_derives_affinity() {
        let mut doc = mage_doc();
        doc.set_text("affiliation", "order_of_hermes");
        Mage.on_field_changed(&mut doc, "affiliation");
        assert_eq!(doc.text("affinity"), Some("Forces"));
        assert!(is_affinity(&doc, "forces"));
        assert!(!is_affinity(&doc, "mind"));
    }

    #[test]
    fn finalize_sets_willpower_quintessence_and_affinity() {
        let mut doc = mage_doc();
        doc.set_text("affiliation", "verbena");
        doc.map_set("backgrounds", "Avatar", serde_json::json!(3));
        Mage.finalize_starting(&WodConfig::default(), &mut doc);
        assert_eq!(
            doc.number("willpower"),
            Some(5),
            "mage Willpower is a flat 5"
        );
        assert_eq!(doc.number("willpower_temp"), Some(5));
        assert_eq!(doc.number("quintessence"), Some(3), "Quintessence = Avatar");
        assert_eq!(doc.text("affinity"), Some("Life"));
        assert_eq!(doc.number("arete"), Some(1), "Arete floors at 1");
    }

    #[test]
    fn sphere_xp_affinity_is_cheaper_than_other() {
        let mut doc = mage_doc();
        doc.set_text("affiliation", "order_of_hermes");
        doc.set_text("affinity", "Forces");
        // Forces is affinity (×7); Mind is not (×8). Raise 2→3.
        assert_eq!(sphere_raise_cost(&doc, "Forces", 2, 3), 14); // 2×7
        assert_eq!(sphere_raise_cost(&doc, "Mind", 2, 3), 16); // 2×8
        // A new Sphere (0→1) is a flat 10 either way.
        assert_eq!(sphere_raise_cost(&doc, "Mind", 0, 1), 10);
        // Arete is current × 8.
        assert_eq!(arete_raise_cost(2, 3), 16);
        assert_eq!(arete_raise_cost(1, 3), 8 + 16);
    }

    #[test]
    fn xp_claims_spheres_and_arete_only() {
        assert_eq!(Mage.xp_max("spheres", Some("Forces")), Some(5));
        assert_eq!(
            Mage.xp_max("spheres", None),
            None,
            "the whole map is not a target"
        );
        assert_eq!(Mage.xp_max("arete", None), Some(10));
        assert_eq!(
            Mage.xp_max("willpower", None),
            None,
            "base handles Willpower"
        );
    }

    #[test]
    fn chargen_contribution_pools_and_exemption() {
        let c = Mage.chargen_contribution(&WodConfig::default(), &mage_doc());
        let spheres = c.pools.iter().find(|p| p.id == "spheres").unwrap();
        assert_eq!(spheres.total, 5);
        assert_eq!(spheres.freebie_cost, 7);
        let arete = c.pools.iter().find(|p| p.id == "arete").unwrap();
        assert_eq!(arete.total, 0);
        assert_eq!(arete.freebie_cost, 4);
        // Spent counts: 5 sphere dots, and Arete above the free 1.
        let mut doc = mage_doc();
        doc.map_set("spheres", "Forces", serde_json::json!(2));
        doc.map_set("spheres", "Life", serde_json::json!(3));
        doc.set_number("arete", 3);
        assert_eq!(spheres.spent(&doc), 5);
        assert_eq!(arete.spent(&doc), 2);
        // Willpower is exempt from the base freebie line.
        assert!(c.freebie_exempt.contains(&TraitKind::Willpower));
    }

    #[test]
    fn arete_is_chargen_capped_at_three() {
        // M20 caps a starting mage's Arete at 3 (raised further with XP).
        assert_eq!(Mage.chargen_max("arete"), Some(3));
        assert_eq!(Mage.chargen_max("Arete"), Some(3));
        assert_eq!(Mage.chargen_max("strength"), None);
    }

    #[test]
    fn sphere_xp_target_is_capped_at_arete() {
        // A Sphere may not be raised above the current Arete through +xp/spend
        // (the same rule +sphere enforces), so the effective XP max tracks Arete.
        let mut doc = mage_doc();
        doc.set_number("arete", 2);
        assert_eq!(Mage.xp_target_cap(&doc, "spheres", Some("Forces")), Some(2));
        doc.set_number("arete", 4);
        assert_eq!(Mage.xp_target_cap(&doc, "spheres", Some("Life")), Some(4));
        // Arete itself (and anything else) carries no extra Sphere cap.
        assert_eq!(Mage.xp_target_cap(&doc, "arete", None), None);
    }

    #[test]
    fn locked_fields_cover_derived_traits() {
        let locked = Mage.chargen_locked_fields();
        for f in [
            "spheres",
            "willpower",
            "quintessence",
            "paradox",
            "affinity",
        ] {
            assert!(locked.contains(&f), "{f} must be chargen-locked");
        }
    }
}
