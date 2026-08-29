//! # Ghoul (V20 mortal-plus) — a vampire-adjacent phase-2 splat
//!
//! A third worked implementation of the [`Splat`] contract (Vampire in
//! [`crate::splats::vampire`] is the reference). A ghoul is a **mortal** who
//! drinks a vampire's vitae: the base oWoD sheet (Attributes / Abilities /
//! Backgrounds / Willpower / Health / Virtues / Merits-Flaws) plus a thin
//! slice of Kindred machinery — Disciplines and a Blood Pool — bolted on.
//! Ghouls are **sheet-tracked, pool-rolled** exactly like every other splat:
//! Disciplines are a name→dots map rolled as the pool the Discipline names,
//! Blood Pool is a splat pool with spend/gain/show commands — no bespoke
//! per-power automation.
//!
//! ## Reuse of [`vampire`]
//!
//! This module deliberately does **not** duplicate Kindred data:
//!
//! - [`vampire::DISCIPLINES`] is the same canonical Discipline list (reused
//!   directly — no ghoul-local copy).
//! - [`vampire::CLANS`] / [`vampire::clan_def`] / [`vampire::ClanDef`] back
//!   the ghoul's `domitor_clan` field (which clan's Disciplines are "in-set"
//!   for this ghoul) — the same 13-clans-plus-Caitiff table a vampire's own
//!   `clan` field uses. Caitiff (empty Discipline list) doubles as "no
//!   domitor clan picked yet ⇒ no in-set Discipline beyond Potence", so it is
//!   a sane default with no extra sentinel value needed.
//! - Blood Pool reuses the "Blood Pool" *concept* structurally (spend/gain,
//!   clamped to a maximum, `+stat`-locked, derived at chargen finalize) but
//!   **not** vampire's generation-scaled table: a ghoul's Blood Pool is
//!   always max 10 and does not regenerate on its own (fed only by drinking
//!   — see [`GHOUL_MAX_BLOOD`]).
//!
//! What this module adds on top of the base oWoD sheet:
//!
//! - a **Disciplines** map (name→0..5): every ghoul starts with **Potence 1**
//!   (an unconditional free grant, [`Ghoul::finalize_starting`]) plus a tiny
//!   1-dot chargen pool for a domitor-clan Discipline
//!   ([`Ghoul::chargen_contribution`], capped at chargen via
//!   [`Ghoul::chargen_max`]); Disciplines in the domitor's clan (or Potence,
//!   always) are cheaper XP than out-of-set ones ([`discipline_raise_cost`]);
//! - a **Blood Pool** (max 10, starts partial) with `+blood`
//!   show/spend/gain/heal (a ghoul spends vitae to mend bashing/lethal, V20);
//! - a **Domitor** reference (free text) + **domitor's clan**
//!   ([`vampire::CLANS`]) + a **Blood Bond level** (0..3; 3 = full bond),
//!   shown with `+domitor` / `+bond`;
//! - Virtues → **Humanity** exactly like a mortal (Conscience + Self-Control;
//!   a ghoul does not walk a Path of Enlightenment) and Willpower from
//!   Courage (the base chargen finalize already derives this from the
//!   config's default `StartingWillpower::FromVirtue("courage")` — no
//!   ghoul-specific override needed);
//! - commands `+disc`/`+disciplines`/`+disc/roll`, `+blood`, `+domitor`,
//!   `+bond`, all self-gating on the actor being a ghoul.
//!
//! ## Command-name collision — flagged for the integration agent
//!
//! `+disc`, `+disciplines`, and `+blood` are registered here under the
//! **same names** [`vampire::register_commands`] already registers. Per the
//! milestone brief's concurrency contract this module must not edit
//! `vampire.rs`, so — in isolation ([`Dispatch::build`] with only
//! `[MortalSplat, Ghoul]`, exactly what this file's tests do) — that is fine.
//! But once both splats are enabled together in the same [`Registrar`] (the
//! live game, once `DEFAULT_TEMPLATES` includes both `"vampire"` and
//! `"ghoul"`), registering the same command name twice is a hard boot error
//! (`ModError::Registry`, "a command name or alias collided with an existing
//! registration").
//!
//! **Recommended fix (the integration agent):** drop this module's `+disc` /
//! `+disciplines` / `+blood` registrations (delete their three
//! `reg.command(...)` calls in [`register_commands`] below, and the
//! `disc_cmd` / `disc_roll` / `blood_cmd` / `blood_gain` / `blood_spend` /
//! `set_blood` functions they call) and instead widen `vampire.rs` to serve
//! both templates:
//!
//! 1. `vampire::require_kindred`'s gate `is_template(cmd, ctx, "vampire")` →
//!    `is_template(cmd, ctx, "vampire") || is_template(cmd, ctx, "ghoul")`.
//! 2. `vampire::disc_cmd`'s in-clan tag and clan-weakness line call
//!    `vampire::is_in_clan`/`vampire::clan_of`, which read the `clan` field —
//!    absent on a ghoul's sheet (a ghoul's analogous field is
//!    `domitor_clan`). Either teach `is_in_clan`/`clan_of` to fall back to
//!    `domitor_clan` when `clan` is unset, or branch on template inside
//!    `disc_cmd`. [`is_in_domitor_set`] / [`domitor_clan_of`] below are the
//!    ready-made ghoul-side equivalents to call into.
//! 3. `vampire::blood_cmd` reads `generation` for the max/per-turn table
//!    (absent on a ghoul); it needs a `generation.is_none()` (or
//!    template-gated) branch using the flat [`GHOUL_MAX_BLOOD`] instead of
//!    [`vampire::generation_traits`].
//! 4. Make `vampire::disc_cmd`, `vampire::blood_cmd`, and
//!    `vampire::require_kindred` `pub(crate)` (currently private `fn`s) so
//!    they can be reused/extended, or fold the above directly into them.
//!
//! **Fallback (if the integration agent prefers not to touch
//! `vampire.rs`):** rename this module's registrations to ghoul-specific
//! aliases (e.g. `+gdisc`, `+gblood`) — the module doc note above spells out
//! exactly what changes either way. `+domitor` and `+bond` are ghoul-only and
//! never collide.

use std::sync::Arc;

use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ModError, Registrar, Role, match_object, perm,
};
use mush_ruleset::SheetDoc;
use mush_ruleset::schema::{
    FieldDef, FieldType, SchemaError, SectionLayout, SheetSchemaBuilder, SheetSection,
};
use smol_str::SmolStr;

use crate::WodCmd;
use crate::config::WodConfig;
use crate::schema::Template;
use crate::splats::vampire;
use crate::splats::{ChargenContribution, Splat, SplatPool, SplatPoolSource, is_template};

/// A ghoul's maximum Blood Pool — flat, unlike a vampire's generation-scaled
/// table (a ghoul's generation is irrelevant; it is always 10).
pub const GHOUL_MAX_BLOOD: i32 = 10;

/// A freshly-ghouled character's starting Blood Pool: partial, not full — a
/// ghoul is fed just enough vitae to survive the Embrace-adjacent process,
/// not gorged (a house default; tune via `@sheet/set` per-character if a
/// story wants otherwise).
const GHOUL_STARTING_BLOOD: i32 = 3;

/// The domitor's clan definition for a ghoul's sheet, or `None` if
/// `domitor_clan` is unset/unknown (reuses [`vampire::clan_def`] against the
/// ghoul's own field).
#[must_use]
pub fn domitor_clan_of(doc: &SheetDoc) -> Option<&'static vampire::ClanDef> {
    doc.text("domitor_clan").and_then(vampire::clan_def)
}

/// Whether `discipline` is "in-set" for a ghoul: Potence always is (every
/// ghoul's universal Discipline), or it is one of the domitor clan's three
/// (Caitiff: none beyond Potence).
#[must_use]
pub fn is_in_domitor_set(doc: &SheetDoc, discipline: &str) -> bool {
    discipline.eq_ignore_ascii_case("Potence")
        || domitor_clan_of(doc).is_some_and(|c| {
            c.disciplines
                .iter()
                .any(|d| d.eq_ignore_ascii_case(discipline))
        })
}

/// The XP cost to raise a ghoul's Discipline `from`→`to`: a new Discipline
/// (0→1) costs a flat 15 (steeper than a vampire's 10 — a ghoul has no
/// vampiric blood potency to draw on); further dots cost current × 6
/// (in-set: Potence or the domitor's clan) or × 9 (out-of-set) — pricier than
/// [`vampire::discipline_raise_cost`]'s ×5/×7, mirroring its shape.
#[must_use]
pub fn discipline_raise_cost(doc: &SheetDoc, discipline: &str, from: i32, to: i32) -> i32 {
    let mult = if is_in_domitor_set(doc, discipline) {
        6
    } else {
        9
    };
    (from.max(0)..to)
        .map(|r| if r <= 0 { 15 } else { r * mult })
        .sum()
}

/// Total dots recorded for one Discipline on a sheet.
fn discipline_dots(doc: &SheetDoc, name: &str) -> i32 {
    doc.map("disciplines")
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .and_then(|(_, v)| v.as_i64())
        })
        .unwrap_or(0) as i32
}

// ----------------------------------------------------------------------
// The Ghoul splat
// ----------------------------------------------------------------------

/// The V20 ghoul splat.
#[derive(Debug, Default, Clone, Copy)]
pub struct Ghoul;

/// This splat behind an `Arc<dyn Splat>`, for the resolver arm the
/// integration agent adds (`"ghoul" => Some(ghoul::template())`).
#[must_use]
pub(crate) fn template() -> Arc<dyn Splat> {
    Arc::new(Ghoul)
}

impl Template for Ghoul {
    fn id(&self) -> &'static str {
        "ghoul"
    }

    fn display_name(&self) -> &str {
        "Ghoul"
    }

    fn extend_schema(&self, b: SheetSchemaBuilder) -> Result<SheetSchemaBuilder, SchemaError> {
        // Reuse vampire's clan table for the domitor-clan enum options —
        // no ghoul-local copy of the 13-clans-plus-Caitiff list.
        let clan_ids: Vec<SmolStr> = vampire::CLANS.iter().map(|c| SmolStr::new(c.id)).collect();
        let ghoul = SheetSection::new("ghoul", "Ghoul", SectionLayout::List)
            .field(
                FieldDef::enumeration("domitor_clan", "Domitor's Clan", clan_ids)
                    .with_default(serde_json::json!("caitiff")),
            )
            .field(FieldDef::text("domitor", "Domitor", 60))
            .field(FieldDef::number("bond_level", "Blood Bond", 0, 3))
            .field(FieldDef::number(
                "bloodpool",
                "Blood Pool",
                0,
                i64::from(GHOUL_MAX_BLOOD),
            ))
            .field(
                FieldDef::number("humanity", "Humanity", 0, 10).with_default(serde_json::json!(7)),
            );
        let b = b.section(ghoul)?;
        // Disciplines map — same field id/shape as vampire's, on this
        // template's own composed schema (no cross-template collision).
        let disc = SheetSection::new("disciplines", "Disciplines", SectionLayout::Map).field(
            FieldDef::map(
                "disciplines",
                "Disciplines",
                FieldType::Dots { min: 0, max: 5 },
            ),
        );
        b.section(disc)
    }
}

impl Splat for Ghoul {
    fn register_commands(&self, reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
        register_commands(reg, cmd)
    }

    fn chargen_contribution(&self, _config: &WodConfig, _doc: &SheetDoc) -> ChargenContribution {
        ChargenContribution {
            pools: vec![SplatPool {
                id: SmolStr::new("disciplines"),
                label: "Disciplines".to_string(),
                // A tiny 1-dot pool for a domitor-clan (or Potence) pick —
                // on top of, not instead of, the unconditional free Potence 1
                // grant at finalize below.
                total: 1,
                source: SplatPoolSource::MapDots(SmolStr::new("disciplines")),
                freebie_cost: 8,
            }],
            freebie_exempt: Vec::new(),
        }
    }

    fn finalize_starting(&self, _config: &WodConfig, doc: &mut SheetDoc) {
        // Potence 1 is an unconditional grant, independent of (and additive
        // with) whatever the player spent their 1-dot chargen pool on —
        // floor, never lower an already-higher value (e.g. a player who
        // spent their pool dot on Potence itself).
        if discipline_dots(doc, "Potence") < 1 {
            doc.map_set("disciplines", "Potence", serde_json::json!(1));
        }
        // Blood Pool starts partial (see GHOUL_STARTING_BLOOD).
        doc.set_number("bloodpool", i64::from(GHOUL_STARTING_BLOOD));
        // Humanity = Conscience + Self-Control, exactly like a mortal (a
        // ghoul does not follow a Path of Enlightenment).
        let humanity = (doc.number("conscience").unwrap_or(1)
            + doc.number("self_control").unwrap_or(1))
        .clamp(0, 10);
        doc.set_number("humanity", humanity);
        // Willpower already = Courage (config baseline) from the base
        // finalize — no ghoul-specific baseline needed.
    }

    fn chargen_locked_fields(&self) -> &'static [&'static str] {
        // Derived at finalize (bloodpool, humanity) or staff/story-adjusted
        // post-chargen (bond_level — a blood bond deepens through play, not
        // a chargen purchase).
        &["bloodpool", "humanity", "bond_level"]
    }

    fn chargen_max(&self, field: &str) -> Option<i32> {
        // A starting ghoul's Discipline picks cap low, per the crib — even
        // freebie overflow cannot buy a 2nd chargen dot; raise further with
        // XP after approval.
        field.eq_ignore_ascii_case("disciplines").then_some(1)
    }

    fn wizard_splat_fields(&self) -> &'static [&'static str] {
        // bond_level is chargen-locked (excluded here on purpose — the
        // wizard sets fields through apply_stat, which would refuse it).
        &["domitor_clan", "domitor"]
    }

    fn wizard_pool_field(&self) -> Option<&'static str> {
        Some("disciplines")
    }

    fn wizard_set_pool(
        &self,
        cmd: &WodCmd,
        ctx: &mut Ctx<'_>,
        name: &str,
        rating: i32,
    ) -> Result<(), CmdError> {
        // Not chargen-locked, so the ordinary +stat map path handles it; the
        // chargen_max(1) cap above still applies.
        crate::chargen_cmds::apply_stat(cmd, ctx, "disciplines", Some(name), &rating.to_string())
    }

    fn xp_max(&self, field: &str, key: Option<&str>) -> Option<i32> {
        (field.eq_ignore_ascii_case("disciplines") && key.is_some()).then_some(5)
    }

    fn xp_cost(
        &self,
        _config: &WodConfig,
        doc: &SheetDoc,
        _field: &str,
        key: Option<&str>,
        from: i32,
        to: i32,
    ) -> i32 {
        discipline_raise_cost(doc, key.unwrap_or(""), from, to)
    }
}

// ----------------------------------------------------------------------
// Commands (all self-gate on the actor being a ghoul — see the module doc's
// command-name-collision note for the +disc/+disciplines/+blood caveat)
// ----------------------------------------------------------------------

/// Refuse politely if the actor is not a ghoul.
fn require_ghoul(cmd: &WodCmd, ctx: &Ctx<'_>) -> Result<SheetDoc, CmdError> {
    if !is_template(cmd, ctx, "ghoul") {
        return Err(CmdError::msg("You are not a ghoul."));
    }
    cmd.read(ctx.w, ctx.actor)
        .ok_or_else(|| CmdError::msg("You have no sheet."))
}

/// `+disc` / `+disciplines` / `+disc/roll <discipline>+<trait> [vs <diff>]`.
///
/// `pub(crate)` because [`vampire::disc_cmd`](crate::splats::vampire) — which
/// owns the `+disc`/`+disciplines` registration for the whole game — delegates
/// here when the actor is a ghoul (see this module's command-collision note and
/// the integration reconciliation in `vampire.rs`).
pub(crate) fn disc_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let doc = require_ghoul(cmd, ctx)?;
    if ctx.switches.has("roll") {
        return disc_roll(cmd, ctx, &doc);
    }
    let name = name_of(ctx, ctx.actor);
    ctx.out
        .sys_to_actor(ctx.actor, "wod", format!("{name}'s Disciplines:"));
    let mut any = false;
    if let Some(m) = doc.map("disciplines") {
        let mut entries: Vec<(&String, i64)> = m
            .iter()
            .filter_map(|(k, v)| v.as_i64().map(|n| (k, n)))
            .filter(|(_, n)| *n > 0)
            .collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (k, n) in entries {
            any = true;
            let tag = if is_in_domitor_set(&doc, k) {
                " (in-set)"
            } else {
                ""
            };
            ctx.out
                .sys_to_actor(ctx.actor, "wod", format!("  {k}: {n}{tag}"));
        }
    }
    if !any {
        ctx.out.sys_to_actor(ctx.actor, "wod", "  (none)");
    }
    let clan_line = domitor_clan_of(&doc).map_or_else(
        || "Domitor's clan: unset (only Potence is in-set).".to_string(),
        |c| {
            let extra = if c.disciplines.is_empty() {
                "none beyond Potence".to_string()
            } else {
                c.disciplines.join(", ")
            };
            format!("Domitor's clan: {} (in-set: Potence, {extra}).", c.label)
        },
    );
    ctx.out.sys_to_actor(ctx.actor, "wod", clan_line);
    Ok(())
}

/// `+disc/roll <discipline>+<trait> [vs <diff>]` — build and roll the pool.
fn disc_roll(cmd: &WodCmd, ctx: &mut Ctx<'_>, doc: &SheetDoc) -> Result<(), CmdError> {
    let (expr, diff) = split_vs(ctx.args.raw());
    if expr.trim().is_empty() {
        return Err(CmdError::msg(
            "Usage: +disc/roll <discipline>+<attribute-or-ability> [vs <diff>]",
        ));
    }
    if !expr_names_a_discipline(doc, &expr) {
        return Err(CmdError::msg(
            "Name a Discipline you have, e.g. +disc/roll potence+strength vs 6.",
        ));
    }
    crate::roll::roll_expr(
        cmd,
        ctx,
        ctx.actor,
        &expr,
        diff.unwrap_or(6),
        true,
        "discipline",
    )?;
    Ok(())
}

/// Whether `expr` names at least one Discipline the sheet actually has dots
/// in (the gate `+disc/roll` enforces before building the pool from the
/// ghoul's own sheet).
fn expr_names_a_discipline(doc: &SheetDoc, expr: &str) -> bool {
    doc.map("disciplines").is_some_and(|m| {
        m.iter().any(|(k, v)| {
            v.as_i64().unwrap_or(0) > 0
                && expr.to_ascii_lowercase().contains(&k.to_ascii_lowercase())
        })
    })
}

/// The result of a `+blood/gain <n>`: `(new_total, spilled)`, clamped to
/// [`GHOUL_MAX_BLOOD`].
fn blood_gain(cur: i32, n: i32) -> (i32, i32) {
    let next = (cur + n).min(GHOUL_MAX_BLOOD);
    (next, n - (next - cur))
}

/// The result of a `+blood/spend <n>`, refusing an overspend.
fn blood_spend(cur: i32, n: i32) -> Result<i32, String> {
    if n > cur {
        return Err(format!("You have only {cur} blood; you cannot spend {n}."));
    }
    Ok(cur - n)
}

/// `+blood` / `+blood/spend <n>` / `+blood/gain <n>` / `+blood/heal [<n>]`.
///
/// `pub(crate)` because [`vampire::blood_cmd`](crate::splats::vampire) — which
/// owns the `+blood` registration for the whole game — delegates here when the
/// actor is a ghoul (a ghoul's flat 10-point pool, fed only by drinking, is
/// intentionally simpler than the vampire generation table). The vampire
/// registration's `buff` switch has no ghoul meaning and falls through to the
/// bare show.
pub(crate) fn blood_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let mut doc = require_ghoul(cmd, ctx)?;
    let cur = doc.number("bloodpool").unwrap_or(0) as i32;

    if ctx.switches.has("heal") {
        return blood_heal(cmd, ctx, doc, cur);
    }
    if ctx.switches.has("spend") {
        let n = arg_count(ctx, 0)?;
        if n < 1 {
            return Err(CmdError::msg("Spend how much blood?"));
        }
        let next = blood_spend(cur, n).map_err(CmdError::msg)?;
        set_blood(cmd, ctx, &mut doc, next)?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!("You spend {n} blood. Blood {next}/{GHOUL_MAX_BLOOD}."),
        );
        return Ok(());
    }
    if ctx.switches.has("gain") {
        let n = arg_count(ctx, 0)?;
        if n < 1 {
            return Err(CmdError::msg("Gain how much blood?"));
        }
        let (next, spilled) = blood_gain(cur, n);
        set_blood(cmd, ctx, &mut doc, next)?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!(
                "You drink from your domitor, gaining {} blood ({spilled} spilled). Blood {next}/{GHOUL_MAX_BLOOD}.",
                next - cur
            ),
        );
        return Ok(());
    }
    // Bare: show.
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!(
            "Blood Pool: {cur}/{GHOUL_MAX_BLOOD} (refilled only by drinking your domitor's vitae)."
        ),
    );
    Ok(())
}

/// `+blood/heal [<n>]` — spend blood to mend bashing/lethal levels (1:1,
/// bashing first; aggravated cannot be mended this way). A ghoul heals with
/// vitae exactly as a vampire does (V20), differing only in the flat
/// [`GHOUL_MAX_BLOOD`] pool. The vampire `+blood` registration declares the
/// `heal` switch, so this branch is reached when a ghoul types `+blood/heal`.
fn blood_heal(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    mut doc: SheetDoc,
    cur: i32,
) -> Result<(), CmdError> {
    let want = arg_count(ctx, 1)?;
    if want < 1 {
        return Err(CmdError::msg("Heal how many levels?"));
    }
    let budget = want.min(cur).max(0) as u32;
    let mut track =
        crate::health::HealthTrack::from_marks(&doc.track("health").unwrap_or_default());
    let mut healed = track.heal(crate::health::Damage::Bashing, budget);
    healed += track.heal(crate::health::Damage::Lethal, budget - healed);
    if healed == 0 {
        return Err(CmdError::msg(
            "You have no bashing or lethal damage to mend (aggravated needs more than blood).",
        ));
    }
    doc.set_track("health", &track.to_marks());
    set_blood(cmd, ctx, &mut doc, cur - healed as i32)?;
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!(
            "You channel {healed} blood, mending {healed} health level(s). Blood {}/{GHOUL_MAX_BLOOD}.",
            cur - healed as i32,
        ),
    );
    Ok(())
}

/// Validate + persist a new Blood Pool value.
fn set_blood(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    doc: &mut SheetDoc,
    value: i32,
) -> Result<(), CmdError> {
    let store = cmd.store_for_doc(doc);
    store
        .set_field(doc, "bloodpool", serde_json::json!(value.max(0)))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    store
        .put_by(ctx.w, ctx.actor, doc, ctx.actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    Ok(())
}

/// `+domitor` — show the ghoul's domitor and their clan.
fn domitor_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let doc = require_ghoul(cmd, ctx)?;
    let domitor = doc
        .text("domitor")
        .filter(|s| !s.is_empty())
        .unwrap_or("(unset)");
    let clan = domitor_clan_of(&doc).map_or_else(|| "Caitiff".to_string(), |c| c.label.to_string());
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!("Domitor: {domitor} (clan {clan})."),
    );
    Ok(())
}

/// `+bond` (show) / `+bond/set <target>=<0..3>` (staff adjust).
fn bond_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("set") {
        return bond_set(cmd, ctx);
    }
    let doc = require_ghoul(cmd, ctx)?;
    let level = doc.number("bond_level").unwrap_or(0);
    let desc = match level {
        0 => "unbound",
        1 => "first tie: a growing fondness for your domitor",
        2 => "second tie: a powerful loyalty",
        3 => "full blood bond: you love your domitor above all others",
        _ => "unknown",
    };
    ctx.out
        .sys_to_actor(ctx.actor, "wod", format!("Blood Bond: {level}/3 ({desc})."));
    Ok(())
}

/// `+bond/set <target>=<0..3>` — staff adjust a ghoul's Blood Bond level.
fn bond_set(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !perm::has_role(ctx.w, ctx.actor, Role::Royalty) {
        return Err(CmdError::msg("Permission denied."));
    }
    let target_spec = ctx.args.lhs().trim();
    let n: i64 = ctx
        .args
        .rhs()
        .and_then(|r| r.trim().parse().ok())
        .filter(|n| (0..=3).contains(n))
        .ok_or_else(|| CmdError::msg("Usage: +bond/set <target>=<0..3>"))?;
    let target = match_object(ctx.w, ctx.actor, target_spec)?;
    let mut doc = cmd
        .read(ctx.w, target)
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    if doc.text("template") != Some("ghoul") {
        return Err(CmdError::msg("That character is not a ghoul."));
    }
    let store = cmd.store_for_doc(&doc);
    store
        .set_field(&mut doc, "bond_level", serde_json::json!(n))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    store
        .put_by(ctx.w, target, &doc, ctx.actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!("Set {}'s Blood Bond to {n}.", name_of(ctx, target)),
    );
    Ok(())
}

// ----------------------------------------------------------------------
// helpers
// ----------------------------------------------------------------------

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

fn name_of(ctx: &Ctx<'_>, who: mush_core::ObjId) -> String {
    ctx.w.get(who).map_or_else(
        || who.to_string(),
        |o| mush_core::output::primary_name(&o.name).to_string(),
    )
}

fn fmt_errs(errs: &[mush_ruleset::SheetError]) -> String {
    errs.iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Register the ghoul command set.
///
/// **`+disc` / `+disciplines` / `+blood` are deliberately NOT registered
/// here.** [`vampire::register_commands`](crate::splats::vampire) owns those
/// names for the whole game; registering them a second time would hard-collide
/// (`ModError`) at boot once both `"vampire"` and `"ghoul"` are enabled (they
/// are, in [`DEFAULT_TEMPLATES`](crate::splats::DEFAULT_TEMPLATES)). The
/// integration reconciliation instead makes vampire's `disc_cmd`/`blood_cmd`
/// delegate to [`disc_cmd`]/[`blood_cmd`] here when the actor is a ghoul (both
/// made `pub(crate)` for that). Only the ghoul-unique `+domitor` / `+bond` are
/// registered below.
pub(crate) fn register_commands(reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+domitor".into(),
            shape: ArgShape::Raw,
            help: "+domitor  --  Show your domitor and their clan.".into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| domitor_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+bond".into(),
            switches: vec!["set".into()],
            shape: ArgShape::EqPair,
            help: "+bond  --  Show your Blood Bond level (0-3; 3 = full bond).\n\
                   +bond/set <target>=<0..3>  --  Adjust a ghoul's Blood Bond (staff)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| bond_cmd(&cmd, ctx))
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chargen::{Pools, finalize_sheet, issues};
    use crate::splats::{Dispatch, MortalSplat};

    /// Build a dispatch over `[mortal, ghoul]` without leaning on the
    /// resolver arm (the integration agent wires that up separately).
    fn dispatch() -> Dispatch {
        let splats: Vec<Arc<dyn Splat>> = vec![Arc::new(MortalSplat), Arc::new(Ghoul)];
        Dispatch::build("wod", splats)
    }

    fn ghoul_doc() -> SheetDoc {
        let d = dispatch();
        let mut doc = SheetDoc::new_default(&d.schemas.schema_for_id("ghoul"));
        doc.set_text("template", "ghoul");
        doc
    }

    #[test]
    fn ghoul_schema_has_disciplines_and_domitor_not_spheres_or_gifts() {
        let d = dispatch();
        let ghoul = d.schemas.schema_for_id("ghoul");
        let mortal = d.schemas.schema_for_id("mortal");

        // Base is shared.
        assert!(ghoul.field("strength").is_some());
        assert!(ghoul.field("courage").is_some());
        // Ghoul-only fields present.
        assert!(ghoul.field("disciplines").is_some());
        assert!(ghoul.field("bloodpool").is_some());
        assert!(ghoul.field("domitor_clan").is_some());
        assert!(ghoul.field("domitor").is_some());
        assert!(ghoul.field("bond_level").is_some());
        assert!(ghoul.field("humanity").is_some());
        // Not a full vampire (no clan/generation/weakness/path) and not a
        // mage or werewolf.
        assert!(ghoul.field("clan").is_none());
        assert!(ghoul.field("generation").is_none());
        assert!(ghoul.field("weakness").is_none());
        assert!(ghoul.field("spheres").is_none());
        assert!(ghoul.field("arete").is_none());
        assert!(ghoul.field("gifts").is_none());
        assert!(ghoul.field("rage").is_none());
        // Mortal carries none of the ghoul's additions.
        assert!(mortal.field("disciplines").is_none());
        assert!(mortal.field("bloodpool").is_none());

        assert!(mush_ruleset::validate(&ghoul, &ghoul.default_data()).is_ok());
    }

    #[test]
    fn potence_one_is_granted_free_at_finalize() {
        let mut doc = ghoul_doc();
        Ghoul.finalize_starting(&WodConfig::default(), &mut doc);
        assert_eq!(
            doc.map("disciplines").unwrap()["Potence"],
            serde_json::json!(1)
        );
    }

    #[test]
    fn finalize_never_lowers_an_already_higher_potence() {
        let mut doc = ghoul_doc();
        doc.map_set("disciplines", "Potence", serde_json::json!(2));
        Ghoul.finalize_starting(&WodConfig::default(), &mut doc);
        assert_eq!(
            doc.map("disciplines").unwrap()["Potence"],
            serde_json::json!(2),
            "the free grant floors at 1, it never lowers a higher value"
        );
    }

    #[test]
    fn finalize_sets_partial_blood_and_humanity_from_virtues() {
        let mut doc = ghoul_doc();
        doc.set_number("conscience", 3);
        doc.set_number("self_control", 2);
        Ghoul.finalize_starting(&WodConfig::default(), &mut doc);
        assert_eq!(
            doc.number("bloodpool"),
            Some(i64::from(GHOUL_STARTING_BLOOD))
        );
        assert_eq!(
            doc.number("humanity"),
            Some(5),
            "Conscience 3 + Self-Control 2"
        );
    }

    #[test]
    fn locked_fields_cover_derived_traits() {
        for f in ["bloodpool", "humanity", "bond_level"] {
            assert!(
                Ghoul.chargen_locked_fields().contains(&f),
                "{f} must be locked"
            );
        }
        assert!(!Ghoul.chargen_locked_fields().contains(&"disciplines"));
    }

    #[test]
    fn discipline_chargen_pick_caps_at_one_dot() {
        assert_eq!(Ghoul.chargen_max("disciplines"), Some(1));
        assert_eq!(Ghoul.chargen_max("Disciplines"), Some(1));
        assert_eq!(Ghoul.chargen_max("strength"), None);
    }

    #[test]
    fn domitor_clan_disciplines_are_in_set_potence_always_is() {
        let mut doc = ghoul_doc();
        doc.set_text("domitor_clan", "gangrel"); // Animalism, Fortitude, Protean
        assert!(is_in_domitor_set(&doc, "Fortitude"));
        assert!(is_in_domitor_set(&doc, "Potence"), "Potence is universal");
        assert!(!is_in_domitor_set(&doc, "Dominate"));
        // Caitiff (the default, and the "unset" sentinel): only Potence.
        doc.set_text("domitor_clan", "caitiff");
        assert!(is_in_domitor_set(&doc, "Potence"));
        assert!(!is_in_domitor_set(&doc, "Fortitude"));
    }

    #[test]
    fn in_set_discipline_xp_is_cheaper_than_out_of_set() {
        let mut doc = ghoul_doc();
        doc.set_text("domitor_clan", "gangrel");
        // Fortitude is in-clan for Gangrel (×6); Dominate is not (×9).
        assert_eq!(discipline_raise_cost(&doc, "Fortitude", 1, 2), 6);
        assert_eq!(discipline_raise_cost(&doc, "Dominate", 1, 2), 9);
        // A new Discipline (0→1) is a flat 15 either way, steeper than a
        // vampire's flat 10 (a ghoul has no vampiric blood potency).
        assert_eq!(discipline_raise_cost(&doc, "Dominate", 0, 1), 15);
        // Potence itself is always in-set, regardless of domitor clan.
        assert_eq!(discipline_raise_cost(&doc, "Potence", 1, 2), 6);
    }

    #[test]
    fn xp_claims_disciplines_only() {
        assert_eq!(Ghoul.xp_max("disciplines", Some("Potence")), Some(5));
        assert_eq!(
            Ghoul.xp_max("disciplines", None),
            None,
            "the whole map is not a target"
        );
        assert_eq!(
            Ghoul.xp_max("bloodpool", None),
            None,
            "base/derived, not XP-buyable"
        );
    }

    #[test]
    fn disc_roll_pool_resolves_against_the_ghouls_own_sheet() {
        // The mechanism +disc/roll relies on: crate::dice::resolve_pool picks
        // up the "disciplines" map exactly like a Backgrounds map, so a pool
        // expression naming a Discipline resolves against THIS ghoul's own
        // dots, not any other character's.
        let d = dispatch();
        let schema = d.schemas.schema_for_id("ghoul");
        let mut doc = SheetDoc::new_default(&schema);
        doc.set_text("template", "ghoul");
        doc.set_number("dexterity", 3);
        doc.map_set("disciplines", "Potence", serde_json::json!(2));

        assert!(expr_names_a_discipline(&doc, "potence+dexterity"));
        let pool = crate::dice::resolve_pool(&schema, &doc, "potence+dexterity").unwrap();
        assert_eq!(pool.total, 5, "Potence 2 + Dexterity 3");

        // A Discipline the ghoul does not have is refused by the command's
        // gate before a roll is ever attempted.
        assert!(!expr_names_a_discipline(&doc, "dominate+wits"));
    }

    #[test]
    fn blood_spend_and_gain_are_bounded() {
        assert_eq!(
            blood_gain(5, 100),
            (GHOUL_MAX_BLOOD, 5 + 100 - GHOUL_MAX_BLOOD)
        );
        assert_eq!(blood_gain(0, 3), (3, 0));
        assert_eq!(blood_spend(7, 3), Ok(4));
        assert!(blood_spend(2, 5).is_err());
        assert_eq!(blood_spend(10, 10), Ok(0));
    }

    #[test]
    fn full_ghoul_chargen_balances_and_finalizes() {
        let cfg = WodConfig::default();
        let d = dispatch();
        let schema = d.schemas.schema_for_id("ghoul");
        let mut doc = SheetDoc::new_default(&schema);
        doc.set_text("template", "ghoul");
        doc.set_number("conscience", 3);
        doc.set_number("self_control", 3);
        doc.set_number("courage", 3);
        // Spend the 1-dot Discipline pool on Potence (in-set, always).
        doc.map_set("disciplines", "Potence", serde_json::json!(1));

        // Zero out the base attribute/ability/background pools (matching
        // crate::chargen's own test pattern) to isolate the ghoul-specific
        // budget line.
        let base = Pools {
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
        let pools = base.with_splat(&cfg, &Ghoul, &doc);
        assert!(
            issues(&cfg, &doc, &pools).is_empty(),
            "the 1-dot Discipline pool is spent exactly on Potence"
        );

        finalize_sheet(&cfg, &mut doc, &pools, &Ghoul).unwrap();
        assert_eq!(
            doc.number("bloodpool"),
            Some(i64::from(GHOUL_STARTING_BLOOD))
        );
        assert_eq!(
            doc.number("humanity"),
            Some(6),
            "Conscience 3 + Self-Control 3"
        );
        assert_eq!(doc.number("willpower"), Some(3), "= Courage");
        assert_eq!(
            doc.map("disciplines").unwrap()["Potence"],
            serde_json::json!(1)
        );
        assert!(
            mush_ruleset::validate(&schema, &doc.data).is_ok(),
            "approvable sheet"
        );
    }
}
