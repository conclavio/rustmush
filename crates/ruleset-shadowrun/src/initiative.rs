//! `+init [/roll]` — Initiative.
//!
//! Initiative Score = (Reaction + Intuition) + the Initiative dice roll (base
//! `1D6`, more from 'ware/magic — the stored `initiative_dice` count). `+init`
//! shows the derived base and dice; `+init/roll` rolls the dice and announces
//! the combat turn's Initiative Score to the room.

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, Registrar, match_object, perm};
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use rand::RngCore;

use crate::Sr5Cmd;
use crate::schema::{INITIATIVE_BASE, INITIATIVE_DICE};
use crate::state::Sr5State;

/// `+init` / `+init/roll`.
fn init_cmd(cmd: &Sr5Cmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim();
    let who = if raw.is_empty() {
        ctx.actor
    } else {
        let t = match_object(ctx.w, ctx.actor, raw)?;
        if t != ctx.actor && !perm::can_examine(ctx.w, ctx.actor, t) {
            return Err(CmdError::msg("You can't examine that character."));
        }
        t
    };
    let doc = cmd
        .read(ctx.w, who)
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    let base = doc.number(INITIATIVE_BASE).unwrap_or_else(|| {
        crate::derive::initiative_base(
            doc.number("reaction").unwrap_or(1),
            doc.number("intuition").unwrap_or(1),
        )
    });
    let dice = doc.number(INITIATIVE_DICE).unwrap_or(1).clamp(1, 5) as usize;

    if ctx.switches.has("roll") {
        let rolled = {
            let st = ctx
                .state_mut::<Sr5State>()
                .ok_or_else(|| CmdError::msg("The Shadowrun ruleset is not loaded."))?;
            (0..dice)
                .map(|_| (st.rng().next_u32() % 6) as i64 + 1)
                .collect::<Vec<_>>()
        };
        let sum: i64 = rolled.iter().sum();
        let score = base + sum;
        let faces = rolled
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        let name = name_of(ctx, who);
        let line = format!("{name} rolls Initiative: {base} + {dice}D6 [{faces}] = {score}.");
        emit_room(ctx, who, &line);
    } else {
        let name = name_of(ctx, who);
        ctx.out.sys_to_actor(
            ctx.actor,
            "sr",
            format!("{name}: Initiative {base} + {dice}D6 (Reaction + Intuition + initiative dice). +init/roll to roll."),
        );
    }
    Ok(())
}

fn emit_room(ctx: &mut Ctx<'_>, who: ObjId, line: &str) {
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
    } else {
        ctx.out.sys_to_actor(who, "sr", line);
    }
}

fn name_of(ctx: &Ctx<'_>, who: ObjId) -> String {
    ctx.w.get(who).map_or_else(
        || who.to_string(),
        |o| mush_core::output::primary_name(&o.name).to_string(),
    )
}

/// Register `+init`.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    cmd: Arc<Sr5Cmd>,
) -> Result<(), mush_core::ModError> {
    reg.command(
        CommandSpec {
            name: "+init".into(),
            switches: vec!["roll".into()],
            shape: ArgShape::Raw,
            help: "+init [<target>]  --  Show Initiative (Reaction + Intuition + \
                   initiative dice).\n\
                   +init/roll  --  Roll your Initiative dice and announce the \
                   Initiative Score to the room."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| init_cmd(&cmd, ctx))
        },
    )
}
