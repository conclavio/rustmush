//! Magic & Resonance — sheet-tracked, **pool-rolled** (the classic MU\* model,
//! not per-spell automation).
//!
//! - `+magic` / `+resonance` — show the caster's/technomancer's magical status.
//! - `+spells` / `+spell <name>` — list / learn spells.
//! - `+cast <spell> [force <f>] [threshold <n>]` — roll the spellcasting pool
//!   (Magic + Spellcasting, Limit = Force) then the **Drain** resistance roll
//!   (Willpower + the tradition's Drain attribute), applying the drain to the
//!   Condition Monitor (Stun, or Physical when overcasting Force > Magic).
//! - `+powers` — list adept Power Points / powers.
//! - `+complexforms` / `+thread <form> [level <l>]` — a technomancer threads a
//!   complex form (Resonance + Software, Limit = Level) then resists **Fading**
//!   (Willpower + Resonance).
//!
//! Drain / Fading Value uses the transparent generic default `max(2,
//! ceil(Force/2))` (no per-spell Drain codes in this milestone — a documented
//! standard reading; a gamemaster adjusts with the threshold/force args).

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, Registrar, match_object, perm};
use mush_ruleset::SheetDoc;

use crate::Sr5Cmd;
use crate::condition::{self, Damage};
use crate::data;
use crate::roll::roll_labeled;
use crate::schema::{MAGIC, RESONANCE};

/// Awakened types that can cast spells.
///
/// Note: an Aspected Magician can only cast if aspected to Sorcery — one
/// aspected to Conjuring or Enchanting cannot cast spells at all. SR5 aspect
/// sub-types are not modelled on the sheet (there is no `aspect` field), so
/// `+cast` cannot gate them precisely; it optimistically permits an Aspected
/// Magician to cast. A gamemaster enforces the aspect restriction narratively.
fn is_caster(aw: &str) -> bool {
    matches!(aw, "magician" | "mystic_adept" | "aspected_magician")
}

/// The generic Drain / Fading Value for a Force / Level: `max(2, ceil(F/2))`.
fn drain_value(force: i64) -> i64 {
    crate::derive::ceil_div(force, 2).max(2)
}

/// Parse `<name> [force <f>] [threshold <n>] [level <l>]`.
struct CastArgs {
    name: String,
    force: Option<i64>,
    threshold: Option<u8>,
}

fn parse_cast_args(raw: &str) -> Result<CastArgs, CmdError> {
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    let is_kw = |t: &str| {
        matches!(
            t.to_ascii_lowercase().as_str(),
            "force" | "level" | "threshold" | "vs"
        )
    };
    let mut name = Vec::new();
    let mut i = 0;
    while i < tokens.len() && !is_kw(tokens[i]) {
        name.push(tokens[i]);
        i += 1;
    }
    let mut out = CastArgs {
        name: name.join(" "),
        force: None,
        threshold: None,
    };
    while i < tokens.len() {
        let kw = tokens[i].to_ascii_lowercase();
        i += 1;
        let v = tokens
            .get(i)
            .ok_or_else(|| CmdError::msg(format!("{kw} needs a number.")))?;
        i += 1;
        match kw.as_str() {
            "force" | "level" => {
                out.force = Some(
                    v.parse::<i64>()
                        .map_err(|_| CmdError::msg(format!("Bad force/level: {v:?}")))?,
                );
            }
            "threshold" | "vs" => {
                out.threshold = Some(
                    v.parse::<u8>()
                        .map_err(|_| CmdError::msg(format!("Bad threshold: {v:?}")))?,
                );
            }
            _ => {}
        }
    }
    if out.name.trim().is_empty() {
        return Err(CmdError::msg("Cast what? +cast <spell> [force <f>]."));
    }
    Ok(out)
}

/// `+cast <spell> [force <f>] [threshold <n>]`.
fn cast_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let who = ctx.actor;
    cmd.require_legacy_condition_authority(ctx, who)?;
    let args = parse_cast_args(ctx.args.raw())?;
    let doc = cmd
        .read(ctx.w, who)
        .ok_or_else(|| CmdError::msg("You have no sheet."))?;
    let aw = doc.text("awakened_type").unwrap_or("mundane").to_string();
    if !is_caster(&aw) {
        return Err(CmdError::msg("You are not a spellcaster."));
    }
    let magic = doc.number(MAGIC).unwrap_or(0);
    if magic < 1 {
        return Err(CmdError::msg("Your Magic rating is 0."));
    }
    // The spell must be known.
    let known = doc
        .list("spells")
        .map(|l| {
            l.iter()
                .filter_map(serde_json::Value::as_str)
                .any(|s| s.eq_ignore_ascii_case(&args.name))
        })
        .unwrap_or(false);
    if !known {
        return Err(CmdError::msg(format!(
            "You don't know the spell {:?}. Learn it with +spell first.",
            args.name
        )));
    }
    let force = args.force.unwrap_or(magic).clamp(1, magic * 2);
    let threshold = args.threshold.unwrap_or(2);
    let tradition = doc.text("tradition").unwrap_or("hermetic").to_string();
    let drain_attr = data::tradition(&tradition)
        .map(|t| t.drain_attribute)
        .unwrap_or("logic");

    // 1) Spellcasting: Magic + Spellcasting, Limit = Force.
    let spell_out = roll_labeled(
        cmd,
        ctx,
        who,
        "magic+spellcasting",
        &format!("Cast {} (Force {force})", args.name),
        threshold,
        Some(force.clamp(0, i64::from(u8::MAX)) as u8),
        0,
        true,
    )?;

    // 2) Drain resistance: Willpower + drain attribute.
    let dv = drain_value(force);
    let drain_out = roll_labeled(
        cmd,
        ctx,
        who,
        &format!("willpower+{drain_attr}"),
        &format!("Drain (DV {dv})"),
        0,
        None,
        0,
        true,
    )?;
    let drain_taken = (dv - i64::from(drain_out.hits())).max(0);
    let kind = if force > magic {
        Damage::Physical
    } else {
        Damage::Stun
    };

    // Apply the drain to the Condition Monitor.
    let mut doc = cmd
        .read(ctx.w, who)
        .ok_or_else(|| CmdError::msg("You have no sheet."))?;
    if drain_taken > 0 {
        condition::apply(&mut doc, kind, drain_taken as u32);
        cmd.store()
            .put_in(who, &doc, who, ctx)
            .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    }
    let _ = spell_out;
    ctx.out.sys_to_actor(
        who,
        "sr",
        if drain_taken > 0 {
            format!(
                "Drain: DV {dv} − {} hits = {drain_taken} {} damage taken.",
                drain_out.hits(),
                kind.word()
            )
        } else {
            format!("Drain: DV {dv} fully resisted ({} hits).", drain_out.hits())
        },
    );
    Ok(())
}

/// `+thread <form> [level <l>]` — a technomancer threads a complex form.
fn thread_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let who = ctx.actor;
    cmd.require_legacy_condition_authority(ctx, who)?;
    let args = parse_cast_args(ctx.args.raw())?;
    let doc = cmd
        .read(ctx.w, who)
        .ok_or_else(|| CmdError::msg("You have no sheet."))?;
    if doc.text("awakened_type") != Some("technomancer") {
        return Err(CmdError::msg("You are not a technomancer."));
    }
    let resonance = doc.number(RESONANCE).unwrap_or(0);
    if resonance < 1 {
        return Err(CmdError::msg("Your Resonance rating is 0."));
    }
    let known = doc
        .list("complex_forms")
        .map(|l| {
            l.iter()
                .filter_map(serde_json::Value::as_str)
                .any(|s| s.eq_ignore_ascii_case(&args.name))
        })
        .unwrap_or(false);
    if !known {
        return Err(CmdError::msg(format!(
            "You don't have the complex form {:?}.",
            args.name
        )));
    }
    let level = args.force.unwrap_or(resonance).clamp(1, resonance * 2);
    let threshold = args.threshold.unwrap_or(2);

    // Threading: Resonance + Software, Limit = Level.
    let _thread_out = roll_labeled(
        cmd,
        ctx,
        who,
        "resonance+software",
        &format!("Thread {} (Level {level})", args.name),
        threshold,
        Some(level.clamp(0, i64::from(u8::MAX)) as u8),
        0,
        true,
    )?;

    // Fading: Willpower + Resonance.
    let fv = drain_value(level);
    let fade_out = roll_labeled(
        cmd,
        ctx,
        who,
        "willpower+resonance",
        &format!("Fading (FV {fv})"),
        0,
        None,
        0,
        true,
    )?;
    let fade_taken = (fv - i64::from(fade_out.hits())).max(0);
    let kind = if level > resonance {
        Damage::Physical
    } else {
        Damage::Stun
    };
    let mut doc = cmd
        .read(ctx.w, who)
        .ok_or_else(|| CmdError::msg("You have no sheet."))?;
    if fade_taken > 0 {
        condition::apply(&mut doc, kind, fade_taken as u32);
        cmd.store()
            .put_in(who, &doc, who, ctx)
            .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    }
    ctx.out.sys_to_actor(
        who,
        "sr",
        if fade_taken > 0 {
            format!(
                "Fading: FV {fv} − {} hits = {fade_taken} {} damage taken.",
                fade_out.hits(),
                kind.word()
            )
        } else {
            format!("Fading: FV {fv} fully resisted ({} hits).", fade_out.hits())
        },
    );
    Ok(())
}

/// `+spell <name>` — learn a spell (during chargen, or by staff).
fn spell_learn_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let who = ctx.actor;
    let name = ctx.args.raw().trim().to_string();
    if name.is_empty() {
        return spells_show(cmd, ctx);
    }
    let mut doc = cmd.read_or_default(ctx.w, who);
    // Only editable while unapproved (chargen) or by staff.
    if doc.approved.is_approved() && !perm::has_role(ctx.w, who, mush_core::Role::Royalty) {
        return Err(CmdError::msg(
            "Your sheet is approved; learn new spells with +karma/spend spells:<name>=1.",
        ));
    }
    let mut list: Vec<serde_json::Value> = doc.list("spells").cloned().unwrap_or_default();
    if list
        .iter()
        .filter_map(serde_json::Value::as_str)
        .any(|s| s.eq_ignore_ascii_case(&name))
    {
        return Err(CmdError::msg("You already know that spell."));
    }
    list.push(serde_json::json!(name));
    doc.set("spells", serde_json::Value::Array(list));
    cmd.store()
        .put_in(who, &doc, who, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out.info(who, format!("Learned the spell {name:?}."));
    Ok(())
}

/// `+spells` — list known spells (own, or a target's for staff/examine).
fn spells_show(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let target = view_target(ctx)?;
    let doc = cmd
        .read(ctx.w, target)
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    let name = name_of(ctx, target);
    let spells: Vec<String> = doc
        .list("spells")
        .map(|l| {
            l.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if spells.is_empty() {
        ctx.out.info(ctx.actor, format!("{name} knows no spells."));
    } else {
        ctx.out.sys_to_actor(
            ctx.actor,
            "sr",
            format!("{name}'s spells: {}", spells.join(", ")),
        );
    }
    Ok(())
}

/// `+powers` — list adept powers + Power Points.
fn powers_show(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let target = view_target(ctx)?;
    let doc = cmd
        .read(ctx.w, target)
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    let name = name_of(ctx, target);
    let pp = doc.number("power_points").unwrap_or(0);
    ctx.out
        .sys_to_actor(ctx.actor, "sr", format!("{name}: Power Points {pp}."));
    if let Some(m) = doc.map("adept_powers")
        && !m.is_empty()
    {
        for (k, v) in m {
            ctx.out
                .sys_to_actor(ctx.actor, "sr", format!("  {k}: level {v}"));
        }
    } else {
        ctx.out.info(ctx.actor, "No adept powers recorded.");
    }
    Ok(())
}

/// `+complexforms` — list complex forms.
fn complexforms_show(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let target = view_target(ctx)?;
    let doc = cmd
        .read(ctx.w, target)
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    let name = name_of(ctx, target);
    let forms: Vec<String> = doc
        .list("complex_forms")
        .map(|l| {
            l.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if forms.is_empty() {
        ctx.out
            .info(ctx.actor, format!("{name} has no complex forms."));
    } else {
        ctx.out.sys_to_actor(
            ctx.actor,
            "sr",
            format!("{name}'s complex forms: {}", forms.join(", ")),
        );
    }
    Ok(())
}

/// `+magic` / `+resonance` — show magical/resonance status.
fn magic_show(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let target = view_target(ctx)?;
    let doc = cmd
        .read(ctx.w, target)
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    let name = name_of(ctx, target);
    let aw = doc.text("awakened_type").unwrap_or("mundane");
    let magic = doc.number(MAGIC).unwrap_or(0);
    let resonance = doc.number(RESONANCE).unwrap_or(0);
    let essence = crate::derive::essence_points(doc.number(crate::schema::ESSENCE).unwrap_or(600));
    let tradition = doc.text("tradition").unwrap_or("none");
    ctx.out.sys_to_actor(
        ctx.actor,
        "sr",
        format!(
            "{name}: {aw}; Magic {magic}, Resonance {resonance}, Essence {essence:.2}, Tradition {tradition}."
        ),
    );
    Ok(())
}

fn view_target(ctx: &Ctx<'_>) -> Result<ObjId, CmdError> {
    let raw = ctx.args.raw().trim();
    if raw.is_empty() {
        return Ok(ctx.actor);
    }
    let t = match_object(ctx.w, ctx.actor, raw)?;
    if t != ctx.actor && !perm::can_examine(ctx.w, ctx.actor, t) {
        return Err(CmdError::msg("You can't examine that character."));
    }
    Ok(t)
}

fn name_of(ctx: &Ctx<'_>, who: ObjId) -> String {
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

/// Whether `_doc` is worth noting has magic (kept for future gating).
#[allow(dead_code)]
fn has_magic(doc: &SheetDoc) -> bool {
    doc.number(MAGIC).unwrap_or(0) > 0 || doc.number(RESONANCE).unwrap_or(0) > 0
}

/// Register the magic command set.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    cmd: Arc<Sr5Cmd>,
) -> Result<(), mush_core::ModError> {
    macro_rules! simple {
        ($name:literal, $help:literal, $f:ident) => {{
            let cmd = Arc::clone(&cmd);
            reg.command(
                CommandSpec {
                    name: $name.into(),
                    shape: ArgShape::Raw,
                    help: $help.into(),
                    ..CommandSpec::default()
                },
                Arc::new(move |ctx: &mut Ctx<'_>| $f(&cmd, ctx)),
            )?;
        }};
    }
    simple!(
        "+cast",
        "+cast <spell> [force <f>] [threshold <n>]  --  Cast a known spell: roll \
         Magic + Spellcasting (Limit = Force), then resist Drain (Willpower + your \
         tradition's Drain attribute). Overcasting (Force > Magic) makes the Drain \
         Physical.",
        cast_cmd
    );
    simple!(
        "+thread",
        "+thread <form> [level <l>]  --  Thread a complex form (technomancer): roll \
         Resonance + Software (Limit = Level), then resist Fading (Willpower + \
         Resonance).",
        thread_cmd
    );
    simple!(
        "+spell",
        "+spell [<name>]  --  Learn a spell (chargen/staff), or list your spells.",
        spell_learn_cmd
    );
    simple!(
        "+spells",
        "+spells [<target>]  --  List known spells.",
        spells_show
    );
    simple!(
        "+powers",
        "+powers [<target>]  --  List adept Power Points and powers.",
        powers_show
    );
    simple!(
        "+complexforms",
        "+complexforms [<target>]  --  List complex forms.",
        complexforms_show
    );
    simple!(
        "+magic",
        "+magic [<target>]  --  Show magical status (Magic/Resonance/Essence/Tradition).",
        magic_show
    );
    simple!(
        "+resonance",
        "+resonance [<target>]  --  Show Resonance/technomancer status.",
        magic_show
    );
    Ok(())
}
