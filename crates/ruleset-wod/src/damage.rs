//! Health-track commands: `+harm`, `+heal` (staff / ruleset), and `+health`.
//!
//! `+harm <target>=<n><type>` and `+heal <target>=<n><type>` mark and clear the
//! seven-level Health track per the crib's marking / upgrade / rollover rules
//! (see [`crate::health`]); wound penalties recompute automatically and
//! incapacitation / death transitions are emitted to the room.

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, Registrar, Role, match_object};
use mush_proto::{NarrativeKind, OutputEvent, RichText};

use crate::WodCmd;
use crate::health::{Damage, HealthTrack, parse_damage_spec};
use crate::schema::HEALTH_FIELD;

/// `+harm <target>=<n><type>` — apply damage.
fn harm_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (target, spec) = parse_target_spec(ctx)?;
    let mut doc = cmd
        .read(ctx.w, target)
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;

    let mut track = HealthTrack::from_marks(&doc.track(HEALTH_FIELD).unwrap_or_default());
    let was_incap = track.is_incapacitated();
    let was_dead = track.all_aggravated();
    track.apply(spec.kind, spec.count);
    let marks = track.to_marks();
    doc.set_track(HEALTH_FIELD, &marks);
    cmd.store_for_doc(&doc)
        .put_in(target, &doc, ctx.actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;

    let name = name_of(ctx, target);
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!(
            "{name} takes {} {} damage. {}",
            spec.count,
            damage_word(spec.kind),
            track_summary(&track),
        ),
    );
    // Transition events to the room.
    if track.all_aggravated() && !was_dead {
        room_emit(ctx, target, &format!("{name} has been slain."));
    } else if track.is_incapacitated() && !was_incap {
        room_emit(ctx, target, &format!("{name} collapses, incapacitated."));
    }
    Ok(())
}

/// `+heal <target>=<n><type>` — clear marks of a type.
fn heal_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (target, spec) = parse_target_spec(ctx)?;
    let mut doc = cmd
        .read(ctx.w, target)
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    let mut track = HealthTrack::from_marks(&doc.track(HEALTH_FIELD).unwrap_or_default());
    let removed = track.heal(spec.kind, spec.count);
    doc.set_track(HEALTH_FIELD, &track.to_marks());
    cmd.store_for_doc(&doc)
        .put_in(target, &doc, ctx.actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    let name = name_of(ctx, target);
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!(
            "Healed {removed} {} from {name}. {}",
            damage_word(spec.kind),
            track_summary(&track),
        ),
    );
    Ok(())
}

/// `+health [<target>]` — show a track.
fn health_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim();
    let target = if raw.is_empty() {
        ctx.actor
    } else {
        let t = match_object(ctx.w, ctx.actor, raw)?;
        if t != ctx.actor && !mush_core::perm::can_examine(ctx.w, ctx.actor, t) {
            return Err(CmdError::msg("You can't examine that character."));
        }
        t
    };
    let doc = cmd
        .read(ctx.w, target)
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    let track = HealthTrack::from_marks(&doc.track(HEALTH_FIELD).unwrap_or_default());
    let name = name_of(ctx, target);
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!("{name}: {}", track_summary(&track)),
    );
    Ok(())
}

/// Parse `<target>=<n><type>` from an `EqPair` command.
fn parse_target_spec(ctx: &Ctx<'_>) -> Result<(ObjId, crate::health::DamageSpec), CmdError> {
    let target_spec = ctx.args.lhs().trim();
    if target_spec.is_empty() {
        return Err(CmdError::msg(
            "Usage: <cmd> <target>=<n><type> (e.g. Bob=3L).",
        ));
    }
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg("Usage: <cmd> <target>=<n><type> (e.g. Bob=3L)."))?;
    let target = match_object(ctx.w, ctx.actor, target_spec)?;
    let spec = parse_damage_spec(rhs)
        .ok_or_else(|| CmdError::msg("Damage must be <n><type>, e.g. 3L, 2 bashing, agg."))?;
    Ok((target, spec))
}

/// A compact track render: `[LB.....] (Hurt, wound -1)`.
fn track_summary(track: &HealthTrack) -> String {
    let mut boxes = String::from("[");
    for slot in track.to_marks() {
        boxes.push(match slot.as_deref().and_then(Damage::parse) {
            Some(d) => d.letter(),
            None => '.',
        });
    }
    boxes.push(']');
    let levels = crate::schema::health_levels();
    let filled = track.filled();
    let status = if track.is_incapacitated() {
        " (Incapacitated)".to_string()
    } else if filled == 0 {
        " (unhurt)".to_string()
    } else {
        let idx = (filled - 1).min(levels.len() - 1);
        format!(" ({}, wound {:+})", levels[idx].label, levels[idx].penalty)
    };
    format!("{boxes}{status}")
}

fn room_emit(ctx: &mut Ctx<'_>, who: ObjId, line: &str) {
    if let Some(loc) = ctx.w.get(who).and_then(|o| o.location) {
        ctx.out.to_room(
            loc,
            Vec::new(),
            OutputEvent::Narrative {
                kind: NarrativeKind::Emit,
                actor: Some(mush_core::obj_ref(ctx.w, who)),
                text: RichText::plain(line),
            },
        );
    }
}

fn damage_word(kind: Damage) -> &'static str {
    match kind {
        Damage::Bashing => "bashing",
        Damage::Lethal => "lethal",
        Damage::Aggravated => "aggravated",
    }
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

/// Register `+harm`, `+heal`, `+health`.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    cmd: Arc<WodCmd>,
) -> Result<(), mush_core::ModError> {
    reg.command(
        CommandSpec {
            name: "+harm".into(),
            shape: ArgShape::EqPair,
            min_role: Role::Royalty,
            help: "+harm <target>=<n><type>  --  Apply damage. Type is b/l/a \
                   (bashing/lethal/aggravated), e.g. +harm Bob=3L. Bashing fills \
                   the track and rolls over into lethal when full."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| harm_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+heal".into(),
            shape: ArgShape::EqPair,
            min_role: Role::Royalty,
            help: "+heal <target>=<n><type>  --  Clear damage marks of a type, e.g. \
                   +heal Bob=2b."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| heal_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+health".into(),
            shape: ArgShape::Raw,
            help: "+health [<target>]  --  Show a Health track (your own, or a \
                   character you can examine)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| health_cmd(&cmd, ctx))
        },
    )?;
    Ok(())
}
