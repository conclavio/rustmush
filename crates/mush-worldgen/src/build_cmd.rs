//! `+build` (S3, doc 14 §4.4–§4.5): the player's side of the building agent —
//! place a request, read a proposal, answer it, steer it, build it, withdraw it,
//! and see where one stands.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx};
use mush_proto::{OutputEvent, RichText};

use crate::data::NS;
use crate::proposal;

/// Register `+build`.
pub(crate) fn register_build(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+build".into(),
            shape: ArgShape::Raw,
            switches: vec![
                "place".into(),
                "show".into(),
                "answer".into(),
                "feedback".into(),
                "go".into(),
                "withdraw".into(),
                "status".into(),
            ],
            help: "+build/place <kind> [<where>]=<prose>   ask for a site (a park, a shop, a house…) in the\n\
                   \x20                                  neighborhood you stand in, or one you name; the\n\
                   \x20                                  building agent plans it and asks what it must\n\
                   +build/show [<id>]                 a proposal: its pack, findings and questions\n\
                   +build/answer <id>/<n>=<answer>    answer question n; the planning goes on when\n\
                   \x20                                  every question has one\n\
                   +build/feedback <id>=<text>        steer another round of planning\n\
                   +build/go <id>                     build it: a player's site stands at once, pending\n\
                   \x20                                  review; staff's applies live\n\
                   +build/withdraw <id>               drop a proposal (and the request behind it)\n\
                   +build/status                      your proposals and your standing: request slots,\n\
                   \x20                                  strikes, what stands\n\
                   Nothing is built until you say go. What you may build, and how much at once, is the\n\
                   neighborhood's policy and your standing."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_build),
    )
}

fn say(ctx: &mut Ctx<'_>, lines: Vec<String>) {
    let text = lines.join("\n");
    ctx.out
        .to_actor(ctx.actor, OutputEvent::Sys(RichText::tag(NS, text)));
}

fn cmd_build(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("place") {
        build_place(ctx)
    } else if ctx.switches.has("show") {
        build_show(ctx)
    } else if ctx.switches.has("answer") {
        build_answer(ctx)
    } else if ctx.switches.has("feedback") {
        build_feedback(ctx)
    } else if ctx.switches.has("go") {
        build_go(ctx)
    } else if ctx.switches.has("withdraw") {
        build_withdraw(ctx)
    } else {
        build_status(ctx)
    }
}

fn parse_id(word: &str) -> Result<u64, CmdError> {
    word.trim()
        .trim_start_matches('#')
        .parse()
        .map_err(|_| CmdError::msg("Which proposal? Give its number (see +build/status)."))
}

fn build_place(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_string();
    let (lhs, prose) = raw
        .split_once('=')
        .ok_or_else(|| CmdError::msg("Usage: +build/place <kind> [<where>]=<prose>"))?;
    let mut words = lhs.trim().splitn(2, char::is_whitespace);
    let kind = words.next().unwrap_or("").trim();
    if kind.is_empty() {
        return Err(CmdError::msg(
            "Usage: +build/place <kind> [<where>]=<prose> — a kind like park, shop or house.",
        ));
    }
    let place = words.next().map(str::trim).filter(|p| !p.is_empty());
    let actor = ctx.actor;
    let line = proposal::place(ctx, actor, kind, place, prose).map_err(CmdError::msg)?;
    say(ctx, vec![line]);
    Ok(())
}

fn build_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_string();
    let actor = ctx.actor;
    let rec = if raw.is_empty() {
        let st = ctx
            .state::<crate::data::WorldgenState>()
            .ok_or_else(|| CmdError::msg("The worldgen module is not composed on this server."))?;
        st.proposals
            .values()
            .filter(|p| p.author == actor)
            .max_by_key(|p| (p.status.open(), p.updated))
            .cloned()
            .ok_or_else(|| {
                CmdError::msg("No proposals of yours. +build/place <kind>=<prose> starts one.")
            })?
    } else {
        proposal::get(ctx, parse_id(&raw)?).map_err(CmdError::msg)?
    };
    if rec.author != actor && !mush_atlas::is_staff(ctx.w, actor) {
        return Err(CmdError::msg(format!("Proposal {} is not yours.", rec.id)));
    }
    let lines = proposal::show_lines(ctx, &rec);
    say(ctx, lines);
    Ok(())
}

fn build_answer(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_string();
    let (lhs, text) = raw
        .split_once('=')
        .ok_or_else(|| CmdError::msg("Usage: +build/answer <id>/<n>=<answer>"))?;
    let (id, n) = lhs
        .split_once('/')
        .ok_or_else(|| CmdError::msg("Usage: +build/answer <id>/<n>=<answer>"))?;
    let id = parse_id(id)?;
    let n: u32 = n
        .trim()
        .parse()
        .map_err(|_| CmdError::msg("Which question? Its number, from +build/show."))?;
    let actor = ctx.actor;
    let line = proposal::answer(ctx, id, actor, n, text).map_err(CmdError::msg)?;
    say(ctx, vec![line]);
    Ok(())
}

fn build_feedback(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_string();
    let (id, text) = raw
        .split_once('=')
        .ok_or_else(|| CmdError::msg("Usage: +build/feedback <id>=<text>"))?;
    let id = parse_id(id)?;
    let actor = ctx.actor;
    let line = proposal::feedback(ctx, id, actor, text).map_err(CmdError::msg)?;
    say(ctx, vec![line]);
    Ok(())
}

fn build_go(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_id(ctx.args.raw())?;
    let actor = ctx.actor;
    let line = proposal::go(ctx, id, actor).map_err(CmdError::msg)?;
    say(ctx, vec![line]);
    Ok(())
}

fn build_withdraw(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_id(ctx.args.raw())?;
    let actor = ctx.actor;
    let line = proposal::withdraw(ctx, id, actor).map_err(CmdError::msg)?;
    say(ctx, vec![line]);
    Ok(())
}

fn build_status(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let lines = proposal::status_lines(ctx, actor);
    say(ctx, lines);
    Ok(())
}
