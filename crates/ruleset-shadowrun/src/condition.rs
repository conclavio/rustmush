//! The SR5 Condition Monitors: the two damage tracks (Physical + Stun), the
//! marking / overflow rules from the crib, and the `+damage` / `+heal` /
//! `+condition` command surface.
//!
//! Rules (authoritative for this milestone):
//!
//! - **Physical** track = `8 + ceil(BOD/2)` boxes; **Stun** = `8 + ceil(WIL/2)`.
//!   Box counts are derived and stored by [`crate::derive::apply_derived`]; the
//!   tracks are resized there, so this module never changes their length — it
//!   only fills / clears boxes.
//! - **Stun overflow**: Stun damage past the Stun track spills, box-for-box,
//!   onto the Physical track (SR5: excess Stun becomes Physical).
//! - **Physical overflow**: Physical damage past the Physical track fills the
//!   [`PHYSICAL_OVERFLOW`](crate::schema::PHYSICAL_OVERFLOW) counter (capped at
//!   Body); overflow beyond Body is lethal.
//! - The **wound modifier** (−1 per three boxes on *either* track) is recomputed
//!   from the filled boxes by [`crate::derive::wound_modifier`] and auto-applied
//!   to dice pools by `+roll`.

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, Registrar, Role, match_object, perm};
use mush_ruleset::SheetDoc;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::Sr5Cmd;
use crate::schema::{
    CONDITION_PHYSICAL, CONDITION_STUN, PHYS_CM_MAX, PHYSICAL_OVERFLOW, STUN_CM_MAX,
};

/// A Condition-Monitor damage type.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Damage {
    /// Physical damage (guns, blades, falling).
    Physical,
    /// Stun damage (fists, tasers, drain).
    Stun,
}

impl Damage {
    /// The stored mark string.
    #[must_use]
    pub fn mark(self) -> &'static str {
        match self {
            Damage::Physical => "physical",
            Damage::Stun => "stun",
        }
    }

    /// The human word.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Damage::Physical => "Physical",
            Damage::Stun => "Stun",
        }
    }

    /// Parse a type token (`p`/`s`/`physical`/`stun`, case-insensitive).
    #[must_use]
    pub fn parse(s: &str) -> Option<Damage> {
        match s.trim().to_ascii_lowercase().as_str() {
            "p" | "phys" | "physical" => Some(Damage::Physical),
            "s" | "stun" => Some(Damage::Stun),
            _ => None,
        }
    }
}

/// A parsed `<count><type>` amount, e.g. `3P`, `2 stun`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DamageSpec {
    /// How many boxes.
    pub count: u32,
    /// The damage type.
    pub kind: Damage,
}

/// Parse `<count><type>` / `<count> <type>` (count defaults to 1), e.g.
/// `"3P"`, `"2 stun"`, `"physical"`.
#[must_use]
pub fn parse_damage_spec(raw: &str) -> Option<DamageSpec> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let digits: String = raw.chars().take_while(char::is_ascii_digit).collect();
    let rest = raw[digits.len()..].trim();
    let count = if digits.is_empty() {
        1
    } else {
        digits.parse::<u32>().ok()?
    };
    let kind = Damage::parse(rest)?;
    Some(DamageSpec { count, kind })
}

/// Filled boxes on a track field.
pub(crate) fn filled(doc: &SheetDoc, field: &str) -> usize {
    doc.track(field)
        .map(|m| m.iter().filter(|b| b.is_some()).count())
        .unwrap_or(0)
}

/// The box count of a track field — its actual length. [`crate::derive::apply_derived`]
/// always sizes each Condition-Monitor track to its derived box-count field, so
/// the track length *is* the authoritative box count (the earlier "clamp to the
/// stored max field" was a no-op — `len.min(max.max(len)) == len`).
fn boxes(doc: &SheetDoc, field: &str) -> usize {
    doc.track(field).map(|m| m.len()).unwrap_or(0)
}

/// Rewrite a track so its first `fill` boxes are marked `mark` and the rest are
/// empty, preserving the track's length.
pub(crate) fn set_filled(doc: &mut SheetDoc, field: &str, fill: usize, mark: &str) {
    let len = doc.track(field).map(|m| m.len()).unwrap_or(0);
    let fill = fill.min(len);
    let marks: Vec<Option<SmolStr>> = (0..len)
        .map(|i| (i < fill).then(|| SmolStr::new(mark)))
        .collect();
    doc.set_track(field, &marks);
}

/// The result of applying damage, for the room transition messages.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DamageReport {
    /// Physical boxes filled after.
    pub physical_filled: usize,
    /// Stun boxes filled after.
    pub stun_filled: usize,
    /// Physical overflow boxes after.
    pub overflow: usize,
    /// The character just filled their Stun track (knocked out).
    pub newly_stunned: bool,
    /// The character just filled their Physical track (down / dying).
    pub newly_down: bool,
    /// The character's overflow exceeded Body (dead).
    pub dead: bool,
}

/// Apply `count` boxes of `kind` to `doc`'s Condition Monitors, following the
/// SR5 overflow rules. Returns a [`DamageReport`]. Pure over the sheet — the
/// caller persists.
pub fn apply(doc: &mut SheetDoc, kind: Damage, count: u32) -> DamageReport {
    let body = doc.number("body").unwrap_or(1).max(1) as usize;
    let was_stun_full = {
        let s = boxes(doc, CONDITION_STUN);
        s > 0 && filled(doc, CONDITION_STUN) >= s
    };
    let was_phys_full = {
        let p = boxes(doc, CONDITION_PHYSICAL);
        p > 0 && filled(doc, CONDITION_PHYSICAL) >= p
    };

    // Clamp an absurd count to what can possibly matter (both tracks + overflow).
    let cap = (boxes(doc, CONDITION_PHYSICAL) + boxes(doc, CONDITION_STUN) + body + 1) as u32;
    let mut remaining = count.min(cap);

    let mut dead = false;
    if matches!(kind, Damage::Stun) {
        let smax = boxes(doc, CONDITION_STUN);
        let scur = filled(doc, CONDITION_STUN);
        let room = smax.saturating_sub(scur);
        let take = (remaining as usize).min(room);
        set_filled(doc, CONDITION_STUN, scur + take, Damage::Stun.mark());
        remaining -= take as u32;
        // Excess Stun spills to Physical, box-for-box.
        if remaining > 0 {
            dead |= apply_physical(doc, remaining, body);
        }
    } else {
        dead |= apply_physical(doc, remaining, body);
    }

    let physical_filled = filled(doc, CONDITION_PHYSICAL);
    let stun_filled = filled(doc, CONDITION_STUN);
    let overflow = doc.number(PHYSICAL_OVERFLOW).unwrap_or(0).max(0) as usize;
    let pmax = boxes(doc, CONDITION_PHYSICAL);
    let smax = boxes(doc, CONDITION_STUN);
    DamageReport {
        physical_filled,
        stun_filled,
        overflow,
        newly_stunned: !was_stun_full && smax > 0 && stun_filled >= smax,
        newly_down: !was_phys_full && pmax > 0 && physical_filled >= pmax,
        dead,
    }
}

/// Fill the Physical track then the overflow counter. Returns whether the
/// character died (overflow exceeded Body).
fn apply_physical(doc: &mut SheetDoc, count: u32, body: usize) -> bool {
    let pmax = boxes(doc, CONDITION_PHYSICAL);
    let pcur = filled(doc, CONDITION_PHYSICAL);
    let room = pmax.saturating_sub(pcur);
    let take = (count as usize).min(room);
    set_filled(
        doc,
        CONDITION_PHYSICAL,
        pcur + take,
        Damage::Physical.mark(),
    );
    let excess = count as usize - take;
    if excess > 0 {
        let over = doc.number(PHYSICAL_OVERFLOW).unwrap_or(0).max(0) as usize + excess;
        let capped = over.min(body);
        doc.set_number(PHYSICAL_OVERFLOW, capped as i64);
        return over > body;
    }
    false
}

/// Heal `count` boxes of `kind` (overflow first for Physical). Returns how many
/// boxes cleared.
pub fn heal(doc: &mut SheetDoc, kind: Damage, count: u32) -> u32 {
    let mut count = count as usize;
    let mut cleared = 0usize;
    if matches!(kind, Damage::Physical) {
        // Clear overflow first, then track boxes.
        let over = doc.number(PHYSICAL_OVERFLOW).unwrap_or(0).max(0) as usize;
        let take = over.min(count);
        if take > 0 {
            doc.set_number(PHYSICAL_OVERFLOW, (over - take) as i64);
            count -= take;
            cleared += take;
        }
        let cur = filled(doc, CONDITION_PHYSICAL);
        let take = cur.min(count);
        set_filled(doc, CONDITION_PHYSICAL, cur - take, Damage::Physical.mark());
        cleared += take;
    } else {
        let cur = filled(doc, CONDITION_STUN);
        let take = cur.min(count);
        set_filled(doc, CONDITION_STUN, cur - take, Damage::Stun.mark());
        cleared += take;
    }
    cleared as u32
}

/// A compact one-track render: `Physical [XX.......] 2/9`.
fn track_line(doc: &SheetDoc, field: &str, max_field: &str, label: &str) -> String {
    let marks = doc.track(field).unwrap_or_default();
    let mut boxes = String::from("[");
    for m in &marks {
        boxes.push(if m.is_some() { 'X' } else { '.' });
    }
    boxes.push(']');
    let max = doc.number(max_field).unwrap_or(marks.len() as i64);
    let f = marks.iter().filter(|m| m.is_some()).count();
    format!("{label:<9} {boxes} {f}/{max}")
}

/// The two-track condition summary plus overflow and the current wound modifier.
#[must_use]
pub fn summary(doc: &SheetDoc) -> Vec<String> {
    let mut out = vec![
        track_line(doc, CONDITION_PHYSICAL, PHYS_CM_MAX, "Physical"),
        track_line(doc, CONDITION_STUN, STUN_CM_MAX, "Stun"),
    ];
    let over = doc.number(PHYSICAL_OVERFLOW).unwrap_or(0);
    let wound = crate::derive::wound_modifier(doc);
    let mut status = format!("Wound modifier {wound:+}");
    if over > 0 {
        let body = doc.number("body").unwrap_or(1);
        status.push_str(&format!("; overflow {over}/{body}"));
    }
    out.push(status);
    out
}

// ----------------------------------------------------------------------
// Commands
// ----------------------------------------------------------------------

/// `+damage <target>=<n>P|<n>S` — apply Condition-Monitor damage (staff).
fn damage_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (target, spec) = parse_target_spec(ctx)?;
    let _ = cmd;
    Err(CmdError::msg(format!(
        "Condition damage is authoritative. Use +act condition-damage on {target} with \
         {{\"amount\":{},\"kind\":\"{}\"}}.",
        spec.count,
        spec.kind.word().to_ascii_lowercase()
    )))
}

/// `+heal <target>=<n>P|<n>S` — clear Condition-Monitor boxes (staff).
fn heal_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (target, spec) = parse_target_spec(ctx)?;
    let _ = cmd;
    Err(CmdError::msg(format!(
        "Condition healing is authoritative. Use +act condition-heal on {target} with \
         {{\"amount\":{},\"kind\":\"{}\"}}.",
        spec.count,
        spec.kind.word().to_ascii_lowercase()
    )))
}

/// `+condition [<target>]` / `+cm [<target>]` — show both tracks.
fn condition_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim();
    let target = if raw.is_empty() {
        ctx.actor
    } else {
        let t = match_object(ctx.w, ctx.actor, raw)?;
        if t != ctx.actor && !perm::can_examine(ctx.w, ctx.actor, t) {
            return Err(CmdError::msg("You can't examine that character."));
        }
        t
    };
    let doc = cmd
        .read_ctx(ctx, target)?
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    let name = name_of(ctx, target);
    ctx.out
        .sys_to_actor(ctx.actor, "sr", format!("{name} — Condition Monitors:"));
    for line in summary(&doc) {
        ctx.out.sys_to_actor(ctx.actor, "sr", format!("  {line}"));
    }
    Ok(())
}

fn parse_target_spec(ctx: &Ctx<'_>) -> Result<(ObjId, DamageSpec), CmdError> {
    let target_spec = ctx.args.lhs().trim();
    if target_spec.is_empty() {
        return Err(CmdError::msg(
            "Usage: <cmd> <target>=<n>P|<n>S (e.g. Razor=3P).",
        ));
    }
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg("Usage: <cmd> <target>=<n>P|<n>S (e.g. Razor=3P)."))?;
    let target = match_object(ctx.w, ctx.actor, target_spec)?;
    let spec = parse_damage_spec(rhs)
        .ok_or_else(|| CmdError::msg("Damage must be <n>P or <n>S, e.g. 3P, 2S, 4 stun."))?;
    Ok((target, spec))
}

fn name_of(ctx: &Ctx<'_>, who: ObjId) -> String {
    ctx.w.get(who).map_or_else(
        || who.to_string(),
        |o| mush_core::output::primary_name(&o.name).to_string(),
    )
}

/// Register `+damage`, `+heal`, `+condition`, `+cm`.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    cmd: Arc<Sr5Cmd>,
) -> Result<(), mush_core::ModError> {
    reg.command(
        CommandSpec {
            name: "+damage".into(),
            shape: ArgShape::EqPair,
            min_role: Role::Royalty,
            help: "+damage <target>=<n>P|<n>S  --  Apply Condition-Monitor damage. \
                   P is Physical, S is Stun (e.g. +damage Razor=3P). Stun that \
                   overflows the Stun track becomes Physical; Physical past the \
                   track is lethal overflow (capped at Body)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| damage_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+heal".into(),
            shape: ArgShape::EqPair,
            min_role: Role::Royalty,
            help: "+heal <target>=<n>P|<n>S  --  Clear Condition-Monitor boxes, e.g. \
                   +heal Razor=2S. Physical healing clears lethal overflow first."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| heal_cmd(&cmd, ctx))
        },
    )?;
    let view = {
        let cmd = Arc::clone(&cmd);
        move |ctx: &mut Ctx<'_>| condition_cmd(&cmd, ctx)
    };
    reg.command(
        CommandSpec {
            name: "+condition".into(),
            aliases: vec!["+cm".into()],
            shape: ArgShape::Raw,
            help: "+condition [<target>]  (alias +cm)  --  Show both Condition \
                   Monitors (Physical + Stun), overflow, and the current wound \
                   modifier."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| view(ctx)),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::apply_derived;
    use crate::schema::build_schema;

    fn runner() -> SheetDoc {
        let schema = build_schema("shadowrun");
        let mut doc = SheetDoc::new_default(&schema);
        doc.set_number("body", 6); // 8 + 3 = 11 physical boxes
        doc.set_number("willpower", 4); // 8 + 2 = 10 stun boxes
        apply_derived(&mut doc);
        doc
    }

    #[test]
    fn parse_specs() {
        assert_eq!(parse_damage_spec("3P").unwrap().kind, Damage::Physical);
        assert_eq!(parse_damage_spec("2 stun").unwrap().count, 2);
        assert_eq!(parse_damage_spec("s").unwrap().kind, Damage::Stun);
        assert!(parse_damage_spec("nonsense").is_none());
    }

    #[test]
    fn physical_fills_then_overflows_at_body() {
        let mut doc = runner();
        let r = apply(&mut doc, Damage::Physical, 11);
        assert_eq!(r.physical_filled, 11);
        assert!(r.newly_down);
        assert!(!r.dead);
        // 6 more (Body 6) → overflow 6, still alive.
        let r = apply(&mut doc, Damage::Physical, 6);
        assert_eq!(r.overflow, 6);
        assert!(!r.dead);
        // one past Body → dead.
        let r = apply(&mut doc, Damage::Physical, 1);
        assert!(r.dead);
    }

    #[test]
    fn stun_overflow_becomes_physical() {
        let mut doc = runner();
        // 10 stun boxes; 12 stun → fills 10 stun, 2 spill to physical.
        let r = apply(&mut doc, Damage::Stun, 12);
        assert_eq!(r.stun_filled, 10);
        assert_eq!(r.physical_filled, 2);
        assert!(r.newly_stunned);
    }

    #[test]
    fn wound_modifier_tracks_filled_boxes() {
        let mut doc = runner();
        apply(&mut doc, Damage::Physical, 3);
        apply(&mut doc, Damage::Stun, 6);
        // −1 (3 physical) −2 (6 stun) = −3.
        assert_eq!(crate::derive::wound_modifier(&doc), -3);
    }

    #[test]
    fn heal_clears_boxes_and_overflow() {
        let mut doc = runner();
        apply(&mut doc, Damage::Physical, 13); // 11 boxes + 2 overflow
        assert_eq!(doc.number(PHYSICAL_OVERFLOW), Some(2));
        // Heal 3 physical: overflow 2 clears first, then 1 box.
        let cleared = heal(&mut doc, Damage::Physical, 3);
        assert_eq!(cleared, 3);
        assert_eq!(doc.number(PHYSICAL_OVERFLOW), Some(0));
        assert_eq!(filled(&doc, CONDITION_PHYSICAL), 10);
    }
}
