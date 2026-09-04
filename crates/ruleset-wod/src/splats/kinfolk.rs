//! # Kinfolk (W20) — the Garou's mortal relatives
//!
//! A **mortal-plus** template built on top of the base oWoD sheet, reusing
//! [`crate::splats::werewolf`]'s data tables and mechanics rather than
//! duplicating them (see the module docs in [`crate::splats`] for the general
//! contract; this module mirrors Vampire/Werewolf's structure).
//!
//! Kinfolk are ordinary humans (or wolves) who carry the Garou blood without
//! ever Changing: no Rage, no Forms, no Renown/Rank — just a Tribe, an
//! optional trickle of Gnosis, a handful of level-1 Gifts, and an inborn
//! immunity to the Delirium. What this module adds on top of the base sheet:
//!
//! - a **Tribe** ([`werewolf::TRIBES`], reused directly — Kinfolk are raised
//!   in a tribe's household even though they never Change);
//! - **Gnosis** (permanent + temporary pool, split exactly like Werewolf's),
//!   defaulting to 0: most Kinfolk have none. It is **bought as the "Gnosis"
//!   entry in the base Backgrounds map** — mirroring how Vampire's Generation
//!   background derives `generation` at [`finalize_starting`](Kinfolk::finalize_starting)
//!   — then clamped to the 0..=3 a Kinfolk may start with; the ordinary
//!   Background freebie price (1/dot beyond the flat pool) is what a fresh
//!   Kinfolk pays for it at chargen. Post-chargen, Gnosis becomes a normal
//!   XP-raisable splat scalar (`+xp/spend gnosis=<n>`, cap 3, ×2/step — see
//!   [`gnosis_raise_cost`]), exactly as Werewolf's own Gnosis is;
//! - a **Gifts** map (name → level, capped at 1 by the field type: Kinfolk
//!   never know more than level-1 Gifts) restricted in spirit to the eligible
//!   set [`eligible_gifts`] returns — the Homid breed's three plus the
//!   character's Tribe's three, straight off [`werewolf::BREEDS`] /
//!   [`werewolf::TRIBES`] (W20 also has a separate "Kinfolk Gifts" list not
//!   reproduced here — see the Deviations note below);
//! - a **Pure Breed** background (0..5, an ordinary Backgrounds map entry —
//!   no new field, exactly like Gnosis above);
//! - a fixed **Delirium immunity** flag (`delirium_immune`, always 1 —
//!   Kinfolk do not suffer the Curse on seeing Crinos);
//! - **no** Rage, Renown, Rank, or Forms — Kinfolk are not Garou;
//! - Willpower and the Virtues are untouched base Mortal mechanics (Courage-
//!   derived), since Kinfolk are still, fundamentally, human;
//! - a `+tribe` command (self-gating on the actor being Kinfolk) showing
//!   Tribe / Gnosis / Pure Breed / Delirium immunity / the eligible Gift list.
//!
//! ## Deviations / integration notes
//!
//! - **`+gnosis` and `+gift`/`+gifts`/`+gift/roll` are *not* registered by
//!   this module.** [`werewolf::register_commands`] already owns those names;
//!   registering them again here would collide (`ModError`) the moment both
//!   splats are enabled together. Their handlers are schema-generic (they read
//!   `gnosis`/`gnosis_temp`/`gifts` off whatever doc is passed), and the gate at
//!   the top of `gnosis_cmd` and `gift_cmd` in `werewolf.rs` is
//!   `require_garou_or_kin`, which admits `is_template(cmd, ctx, "kinfolk")` —
//!   so a Kinfolk uses them today (the integration this note once flagged as a
//!   TODO is done; look-back fold T5). `rage_cmd`, `renown_cmd`, `shift_cmd` and
//!   `rank_cmd` stay on the strict Garou-only gate (Kinfolk have none of those).
//!   The pool-building and eligibility logic ([`eligible_gifts`], [`has_gift`])
//!   lives and is tested here.
//! - `werewolf::pool_raise_cost` (the Rage/Gnosis ×mult XP formula) is
//!   private to `werewolf.rs`, so [`gnosis_raise_cost`] below is a small
//!   local copy of the same one-line formula rather than a cross-module
//!   call. Widening it to `pub(crate)` would let this module (and Ghoul, if
//!   it ever prices a pool the same way) call the original instead.
//! - The eligible-Gift set (`+ homid breed gifts ∪ own-tribe gifts`) is a
//!   deliberate simplification: W20's canonical Kinfolk have their own
//!   separate short "Kinfolk Gifts" list distinct from the Garou breed/tribe
//!   tables. Reusing `werewolf`'s existing tables (per this milestone's
//!   reuse-not-duplicate brief) instead of authoring a new data table gets
//!   the "breed=homid + tribe" half of the crib note; a "+ Kinfolk-specific"
//!   list would be new data, out of scope here.

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ModError, Registrar};
use mush_ruleset::SheetDoc;
use mush_ruleset::schema::{
    FieldDef, FieldType, SchemaError, SectionLayout, SheetSchemaBuilder, SheetSection,
};
use smol_str::SmolStr;

use crate::WodCmd;
use crate::config::WodConfig;
use crate::schema::Template;
use crate::splats::werewolf;
use crate::splats::{Splat, is_template};

// ----------------------------------------------------------------------
// Tribe (reused from werewolf) + Gift eligibility
// ----------------------------------------------------------------------

/// The tribe enum options, off [`werewolf::TRIBES`] (that module's own
/// `tribe_ids` is private, so this is a small local rebuild of the same list
/// — the data itself, `TRIBES`/`tribe_def`/`tribe_of`, is reused directly).
fn tribe_ids() -> Vec<SmolStr> {
    werewolf::TRIBES
        .iter()
        .map(|t| SmolStr::new(t.id))
        .collect()
}

/// The level-1 Gifts a Kinfolk with Gnosis may plausibly learn: the Homid
/// breed's three plus their own Tribe's three (deduplicated), straight off
/// [`werewolf::BREEDS`] / [`werewolf::TRIBES`]. Empty if no tribe is chosen
/// yet. See the module docs' Deviations note for what this does *not* cover.
#[must_use]
pub fn eligible_gifts(doc: &SheetDoc) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    if let Some(homid) = werewolf::breed_def("homid") {
        out.extend_from_slice(homid.gifts);
    }
    if let Some(tribe) = werewolf::tribe_of(doc) {
        for g in tribe.gifts {
            if !out.contains(g) {
                out.push(g);
            }
        }
    }
    out
}

/// Whether the sheet knows a Gift by name (case-insensitive, level > 0) —
/// the same check [`werewolf`]'s own `has_gift` makes (private there, so a
/// local copy; trivial enough that duplicating it costs nothing).
fn has_gift(doc: &SheetDoc, gift: &str) -> bool {
    doc.map("gifts").is_some_and(|m| {
        m.iter()
            .any(|(k, v)| k.eq_ignore_ascii_case(gift) && v.as_i64().unwrap_or(0) > 0)
    })
}

/// Total dots of a named background on a sheet (Gnosis / Pure Breed both live
/// here — no dedicated scalar field, exactly how Vampire's Generation
/// background works).
fn background_dots(doc: &SheetDoc, name: &str) -> i32 {
    doc.map("backgrounds")
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .and_then(|(_, v)| v.as_i64())
        })
        .unwrap_or(0) as i32
}

/// The XP cost to raise Gnosis `from`→`to` at ×2 per step (current rating ×
/// mult; a first dot from 0 is ×1 of the multiplier) — the same formula
/// [`werewolf`] prices its own Gnosis with, reproduced locally because that
/// helper is private there. See the module docs' Deviations note.
#[must_use]
fn gnosis_raise_cost(from: i32, to: i32) -> i32 {
    (from.max(0)..to).map(|r| r.max(1) * 2).sum()
}

// ----------------------------------------------------------------------
// The Kinfolk splat
// ----------------------------------------------------------------------

/// The W20 Kinfolk splat.
#[derive(Debug, Default, Clone, Copy)]
pub struct Kinfolk;

impl Template for Kinfolk {
    fn id(&self) -> &'static str {
        "kinfolk"
    }

    fn display_name(&self) -> &str {
        "Kinfolk"
    }

    fn extend_schema(&self, b: SheetSchemaBuilder) -> Result<SheetSchemaBuilder, SchemaError> {
        let kin = SheetSection::new("kinfolk", "Kinfolk", SectionLayout::List)
            .field(FieldDef::enumeration("tribe", "Tribe", tribe_ids()))
            .field(FieldDef::number("gnosis", "Gnosis", 0, 10))
            .field(FieldDef::number("gnosis_temp", "Gnosis (temporary)", 0, 10))
            .field(
                FieldDef::number("delirium_immune", "Delirium Immune", 0, 1)
                    .with_default(serde_json::json!(1)),
            );
        let b = b.section(kin)?;
        // Gifts map (name → level, capped at 1 — Kinfolk never know a Gift
        // above level 1).
        let gifts = SheetSection::new("gifts", "Gifts", SectionLayout::Map).field(FieldDef::map(
            "gifts",
            "Gifts",
            FieldType::Dots { min: 0, max: 1 },
        ));
        b.section(gifts)
    }
}

impl Splat for Kinfolk {
    fn register_commands(&self, reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
        register_commands(reg, cmd)
    }

    // No chargen priority pools: Gnosis and Pure Breed ride the base
    // Backgrounds pool like any other named background (see the module
    // docs), so the default (empty) contribution is exactly right — a
    // Kinfolk's budget balances byte-identically to a Mortal's plus whatever
    // Background dots they spend.

    fn finalize_starting(&self, _config: &WodConfig, doc: &mut SheetDoc) {
        // Gnosis: the "Gnosis" background, clamped to the 0..=3 a Kinfolk may
        // start with (a player who dumps more than 3 dots there just wastes
        // the overflow, exactly like Vampire's Generation background).
        let gnosis = background_dots(doc, "Gnosis").clamp(0, 3);
        doc.set_number("gnosis", i64::from(gnosis));
        doc.set_number("gnosis_temp", i64::from(gnosis));
        // Always immune to the Delirium.
        doc.set_number("delirium_immune", 1);
        // Willpower stays the base Courage-derived value from the base
        // finalize — Kinfolk are mortals, not Garou with a tribal fixed WP.
    }

    fn chargen_locked_fields(&self) -> &'static [&'static str] {
        // Derived at finalize from the Gnosis background; +stat must not set
        // these directly (mirrors Vampire's locked `generation`/`bloodpool`
        // and Werewolf's locked `rage`/`gnosis`).
        //
        // `gifts` is locked too: Kinfolk grant no free Gifts at chargen and add
        // no Gift pool, so an unlocked `gifts` map would let a chargen `+stat
        // gifts:X=1` write arbitrarily many level-1 Gifts that no budget ledger
        // charges (`freebie_spent` never scans the map) — a point-economy
        // bypass. Locked here (exactly as Werewolf locks `gifts`), a Kinfolk's
        // Gifts are staff-granted or XP-bought post-approval only.
        &["gnosis", "gnosis_temp", "delirium_immune", "gifts"]
    }

    fn wizard_splat_fields(&self) -> &'static [&'static str] {
        // Gnosis / Pure Breed are ordinary Backgrounds entries, handled by
        // the wizard's existing Backgrounds step; Tribe is the one splat
        // scalar.
        &["tribe"]
    }

    fn xp_max(&self, field: &str, key: Option<&str>) -> Option<i32> {
        match field {
            "gnosis" if key.is_none() => Some(3),
            "gifts" if key.is_some() => Some(1),
            _ => None,
        }
    }

    fn xp_cost(
        &self,
        _config: &WodConfig,
        _doc: &SheetDoc,
        field: &str,
        _key: Option<&str>,
        from: i32,
        to: i32,
    ) -> i32 {
        match field {
            "gnosis" => gnosis_raise_cost(from, to),
            "gifts" => werewolf::gift_raise_cost(from, to),
            _ => 0,
        }
    }
}

// ----------------------------------------------------------------------
// Commands (self-gate on the actor being Kinfolk)
// ----------------------------------------------------------------------

/// Refuse politely if the actor is not Kinfolk.
fn require_kinfolk(cmd: &WodCmd, ctx: &Ctx<'_>) -> Result<SheetDoc, CmdError> {
    if !is_template(cmd, ctx, "kinfolk") {
        return Err(CmdError::msg("You are not Kinfolk."));
    }
    cmd.read(ctx.w, ctx.actor)
        .ok_or_else(|| CmdError::msg("You have no sheet."))
}

/// `+tribe` — show Tribe, Gnosis, Pure Breed, Delirium immunity, and (with
/// Gnosis) which Gifts are open to learn.
fn tribe_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let doc = require_kinfolk(cmd, ctx)?;
    match werewolf::tribe_of(&doc) {
        Some(t) => {
            ctx.out
                .sys_to_actor(ctx.actor, "wod", format!("Tribe: {} — {}", t.label, t.ban))
        }
        None => ctx
            .out
            .sys_to_actor(ctx.actor, "wod", "Tribe: (none chosen)"),
    }
    let perm = doc.number("gnosis").unwrap_or(0);
    let temp = doc.number("gnosis_temp").unwrap_or(0);
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!("Gnosis: {temp}/{perm} (temporary/permanent)."),
    );
    let pure_breed = background_dots(&doc, "Pure Breed");
    ctx.out
        .sys_to_actor(ctx.actor, "wod", format!("Pure Breed: {pure_breed}."));
    ctx.out
        .sys_to_actor(ctx.actor, "wod", "Immune to the Delirium.");
    if perm > 0 {
        let eligible = eligible_gifts(&doc);
        if eligible.is_empty() {
            ctx.out.sys_to_actor(
                ctx.actor,
                "wod",
                "Choose a Tribe to see which Gifts you may learn.",
            );
        } else {
            let (known, unlearned): (Vec<&str>, Vec<&str>) =
                eligible.iter().copied().partition(|g| has_gift(&doc, g));
            if !known.is_empty() {
                ctx.out.sys_to_actor(
                    ctx.actor,
                    "wod",
                    format!("Gifts you know: {}.", known.join(", ")),
                );
            }
            if !unlearned.is_empty() {
                ctx.out.sys_to_actor(
                    ctx.actor,
                    "wod",
                    format!("Gifts you may yet learn: {}.", unlearned.join(", ")),
                );
            }
        }
    } else {
        ctx.out
            .sys_to_actor(ctx.actor, "wod", "No Gnosis: you cannot yet learn a Gift.");
    }
    Ok(())
}

/// Register the Kinfolk command set. **Deliberately does not register
/// `+gnosis` / `+gift` / `+gifts`** — see the module docs' Deviations note.
pub(crate) fn register_commands(reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+tribe".into(),
            shape: ArgShape::Raw,
            help: "+tribe  --  Show your Tribe, Gnosis, Pure Breed, Delirium immunity, and \
                   (with Gnosis) the Gifts open to you."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| tribe_cmd(&cmd, ctx))
        },
    )?;
    Ok(())
}

/// The Kinfolk splat behind an `Arc<dyn Splat>` (the resolver entry point).
#[must_use]
pub(crate) fn template() -> Arc<dyn Splat> {
    Arc::new(Kinfolk)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::splats::{Dispatch, MortalSplat};

    /// A dispatch over mortal + kinfolk built directly (no resolver arm
    /// needed — mirrors werewolf.rs's own isolated test setup).
    fn dispatch() -> Dispatch {
        let splats: Vec<Arc<dyn Splat>> = vec![Arc::new(MortalSplat), Arc::new(Kinfolk)];
        Dispatch::build("wod", splats)
    }

    fn kinfolk_doc() -> SheetDoc {
        let d = dispatch();
        let mut doc = SheetDoc::new_default(&d.schemas.schema_for_id("kinfolk"));
        doc.set_text("template", "kinfolk");
        doc
    }

    #[test]
    fn schema_composes_kinfolk_traits_and_not_other_splats() {
        let d = dispatch();
        let k = d.schemas.schema_for_id("kinfolk");
        for id in ["tribe", "gnosis", "gnosis_temp", "delirium_immune", "gifts"] {
            assert!(k.field(id).is_some(), "kinfolk schema missing {id}");
        }
        // No Rage, no Renown/Rank, no Forms, no Disciplines/Spheres/Arts —
        // Kinfolk are not Garou and not any other splat.
        for id in [
            "rage",
            "rage_temp",
            "rank",
            "glory_perm",
            "honor_perm",
            "wisdom_perm",
            "current_form",
            "disciplines",
            "bloodpool",
            "spheres",
            "arete",
        ] {
            assert!(
                k.field(id).is_none(),
                "kinfolk schema should not carry {id}"
            );
        }
        // A mortal carries none of the Kinfolk fields either.
        let m = d.schemas.schema_for_id("mortal");
        for id in ["tribe", "gnosis", "gifts"] {
            assert!(m.field(id).is_none());
        }
        assert!(mush_ruleset::validate(&k, &k.default_data()).is_ok());
        assert!(mush_ruleset::validate(&m, &m.default_data()).is_ok());
    }

    #[test]
    fn gnosis_defaults_zero_and_is_buyable_to_three_via_a_background() {
        let cfg = WodConfig::default();
        // No Gnosis background at all → still 0 after finalize.
        let mut doc = kinfolk_doc();
        Kinfolk.finalize_starting(&cfg, &mut doc);
        assert_eq!(doc.number("gnosis"), Some(0));
        assert_eq!(doc.number("gnosis_temp"), Some(0));

        // 2 dots in the Gnosis background → Gnosis 2, pool starts full.
        let mut doc = kinfolk_doc();
        doc.map_set("backgrounds", "Gnosis", serde_json::json!(2));
        Kinfolk.finalize_starting(&cfg, &mut doc);
        assert_eq!(doc.number("gnosis"), Some(2));
        assert_eq!(doc.number("gnosis_temp"), Some(2));

        // 5 dots (the background's own max) clamps the derived Gnosis to 3.
        let mut doc = kinfolk_doc();
        doc.map_set("backgrounds", "Gnosis", serde_json::json!(5));
        Kinfolk.finalize_starting(&cfg, &mut doc);
        assert_eq!(
            doc.number("gnosis"),
            Some(3),
            "Gnosis caps at 3 for Kinfolk"
        );
        assert_eq!(doc.number("gnosis_temp"), Some(3));
    }

    #[test]
    fn delirium_immune_defaults_true_and_is_chargen_locked() {
        let d = dispatch();
        let k = d.schemas.schema_for_id("kinfolk");
        let doc = SheetDoc::new_default(&k);
        assert_eq!(doc.number("delirium_immune"), Some(1));
        let locked = Kinfolk.chargen_locked_fields();
        assert!(locked.contains(&"delirium_immune"));
        assert!(locked.contains(&"gnosis"));
        assert!(locked.contains(&"gnosis_temp"));
        // Gifts are locked at chargen (no free Gifts, no Gift pool): a chargen
        // +stat gifts:X=1 must be refused so it cannot bypass the budget.
        assert!(locked.contains(&"gifts"));
        // Tribe itself stays player-settable.
        assert!(!locked.contains(&"tribe"));
    }

    #[test]
    fn eligible_gifts_combine_homid_breed_and_own_tribe() {
        let mut doc = kinfolk_doc();
        doc.set_text("tribe", "black_furies");
        let names = eligible_gifts(&doc);
        // Homid breed gifts.
        assert!(names.contains(&"Persuasion"));
        assert!(names.contains(&"Smell of Man"));
        // Black Furies tribe gifts.
        assert!(names.contains(&"Breath of the Wyld"));
        assert!(names.contains(&"Sense Wyrm"));
        // No tribe chosen yet → only the (always-present) Homid set.
        let mut none_doc = kinfolk_doc();
        none_doc.set_text("tribe", "");
        let names = eligible_gifts(&none_doc);
        assert!(names.contains(&"Persuasion"));
        assert!(!names.contains(&"Breath of the Wyld"));
    }

    #[test]
    fn gift_roll_pool_resolves_from_the_kinfolk_sheet() {
        let d = dispatch();
        let schema = d.schemas.schema_for_id("kinfolk");
        let mut doc = kinfolk_doc();
        doc.set_number("dexterity", 3);
        doc.set_number("alertness", 2);
        doc.map_set("gifts", "Persuasion", serde_json::json!(1));

        assert!(has_gift(&doc, "persuasion"), "case-insensitive lookup");
        assert!(!has_gift(&doc, "razor claws"), "not a known gift");

        // The same pool-resolution path +gift/roll (and +roll) uses, applied
        // to a real Kinfolk sheet: dexterity(3) + alertness(2) = 5.
        let resolved = crate::dice::resolve_pool(&schema, &doc, "dexterity+alertness").unwrap();
        assert_eq!(resolved.total, 5);
        assert_eq!(resolved.trait_fields, vec!["dexterity", "alertness"]);
    }

    #[test]
    fn xp_costs_and_caps_claim_only_gnosis_and_gifts() {
        let cfg = WodConfig::default();
        let doc = kinfolk_doc();
        assert_eq!(Kinfolk.xp_max("gnosis", None), Some(3));
        assert_eq!(Kinfolk.xp_max("gifts", Some("Persuasion")), Some(1));
        // Not Kinfolk-owned traits: base rules (or nothing) apply.
        assert_eq!(Kinfolk.xp_max("rage", None), None);
        assert_eq!(Kinfolk.xp_max("strength", None), None);
        // Gnosis ×2: 0→1 = 2; 1→2 = 2×2 = 4 (total 0→2 = 6).
        assert_eq!(Kinfolk.xp_cost(&cfg, &doc, "gnosis", None, 0, 1), 2);
        assert_eq!(Kinfolk.xp_cost(&cfg, &doc, "gnosis", None, 0, 2), 4);
        assert_eq!(Kinfolk.xp_cost(&cfg, &doc, "gnosis", None, 2, 3), 4);
        // Gifts reuse werewolf's exact formula (a new Gift = 3).
        assert_eq!(
            Kinfolk.xp_cost(&cfg, &doc, "gifts", Some("Persuasion"), 0, 1),
            werewolf::gift_raise_cost(0, 1)
        );
        assert_eq!(Kinfolk.xp_cost(&cfg, &doc, "gifts", Some("x"), 0, 1), 3);
    }

    #[test]
    fn full_kinfolk_chargen_finalizes_and_approves() {
        let cfg = WodConfig::default();
        // A trivially-balanced budget (mirrors crate::chargen's own zero-pool
        // tests): nothing owed beyond whatever Background dots are spent,
        // which 15 freebies easily covers at the base 1/dot Background price.
        let pools = crate::chargen::Pools {
            attr: vec![
                (SmolStr::new("physical"), 0),
                (SmolStr::new("social"), 0),
                (SmolStr::new("mental"), 0),
            ],
            abil: vec![
                (SmolStr::new("talents"), 0),
                (SmolStr::new("skills"), 0),
                (SmolStr::new("knowledges"), 0),
            ],
            backgrounds: 0,
            freebies: 15,
            splat: Vec::new(),
            freebie_exempt: Vec::new(),
        };
        let mut doc = kinfolk_doc();
        doc.set_text("tribe", "fianna");
        doc.set_number("courage", 3);
        doc.map_set("backgrounds", "Gnosis", serde_json::json!(2));
        doc.map_set("backgrounds", "Pure Breed", serde_json::json!(1));

        // finalize_sheet is exactly what +chargen/submit calls, and what the
        // wod-chargen job handler re-validates on approval; a clean Ok(())
        // here is the "-> approve" half of the flow at the unit level.
        crate::chargen::finalize_sheet(&cfg, &mut doc, &pools, &Kinfolk).unwrap();

        assert_eq!(doc.number("gnosis"), Some(2));
        assert_eq!(doc.number("gnosis_temp"), Some(2));
        assert_eq!(doc.number("delirium_immune"), Some(1));
        assert_eq!(
            doc.number("willpower"),
            Some(3),
            "Willpower = Courage (base)"
        );
        assert_eq!(doc.number("willpower_temp"), Some(3));

        let d = dispatch();
        let schema = d.schemas.schema_for_id("kinfolk");
        assert!(mush_ruleset::validate(&schema, &doc.data).is_ok());

        // The same budget re-validates cleanly a second time (what the
        // approval job handler does).
        assert!(crate::chargen::issues(&cfg, &doc, &pools).is_empty());
    }
}
