//! Info commands: `WHO` (connected players with real idle/connect times
//! from the world's session stats) and `help`/`+help` (rendered from every
//! registered [`CommandSpec`]'s real help text).

use std::sync::Arc;

use mush_proto::{OutputEvent, WhoEntry};

use crate::cmd::{ArgShape, CmdError, CommandSpec, Ctx, Registry, RegistryError};
use crate::flags::{Flags, Powers, Role};
use crate::perm;

use super::attr_text;

fn who_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let staff = perm::has_role(ctx.w, ctx.actor, Role::Wizard)
        || perm::has_power(ctx.w, ctx.actor, Powers::SEE_HIDDEN);
    let mut entries = Vec::new();
    for id in ctx.w.connected() {
        let Some(o) = ctx.w.get(id) else { continue };
        if o.flags.contains(Flags::DARK) && !staff && id != ctx.actor {
            continue;
        }
        let location = if staff {
            o.location
                .and_then(|l| ctx.w.get(l))
                .map(|r| r.name.clone())
        } else {
            None
        };
        // Idle/connect times come from the engine-maintained session stats
        // (clamped to 0 against clock skew); actors marked connected by
        // other means (tests, NPCs) report zeros.
        let now = ctx.w.now();
        let (idle_s, conn_s) = ctx.w.session_stat(id).map_or((0, 0), |s| {
            (
                u64::try_from(now - s.last_input).unwrap_or(0),
                u64::try_from(now - s.connect_time).unwrap_or(0),
            )
        });
        entries.push(WhoEntry {
            name: o.name.clone(),
            alias: o.aliases.first().map(ToString::to_string),
            idle_s,
            conn_s,
            location,
            doing: attr_text(ctx.w, id, "doing"),
        });
    }
    ctx.out.to_actor(ctx.actor, OutputEvent::Who(entries));
    Ok(())
}

fn help_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let topic = ctx.args.raw().trim().to_string();
    if topic.is_empty() {
        ctx.out
            .info(ctx.actor, "Commands (type \"help <command>\" for details):");
        let mut names: Vec<&str> = ctx.reg.specs().map(|s| s.name.as_str()).collect();
        names.sort_unstable();
        ctx.out.info(ctx.actor, names.join("  "));
        let topics: Vec<&str> = ctx.reg.help_topic_names().collect();
        if !topics.is_empty() {
            ctx.out
                .info(ctx.actor, "Topics (type \"help <topic>\" to read):");
            ctx.out.info(ctx.actor, topics.join("  "));
        }
        return Ok(());
    }
    // Precedence: an exact command-name (or alias) match always wins over a
    // prose topic of the same name; topics fill in everything else.
    let Some(spec) = ctx.reg.spec_of(&topic) else {
        if let Some(t) = ctx.reg.help_topic(&topic) {
            let (title, body) = (t.title.clone(), t.body.clone());
            ctx.out.info(ctx.actor, title);
            ctx.out.info(ctx.actor, body);
            return Ok(());
        }
        return Err(CmdError::msg(format!("No help available for \"{topic}\".")));
    };
    let mut header = spec.name.to_string();
    if !spec.aliases.is_empty() {
        let aliases: Vec<&str> = spec.aliases.iter().map(smol_str::SmolStr::as_str).collect();
        header.push_str(&format!("  (aliases: {})", aliases.join(", ")));
    }
    if !spec.switches.is_empty() {
        let switches: Vec<&str> = spec
            .switches
            .iter()
            .map(smol_str::SmolStr::as_str)
            .collect();
        header.push_str(&format!("  [switches: /{}]", switches.join(" /")));
    }
    ctx.out.info(ctx.actor, header);
    ctx.out.info(ctx.actor, spec.help.clone());
    Ok(())
}

/// Register `WHO` and `help`.
pub(super) fn register(reg: &mut Registry) -> Result<(), RegistryError> {
    reg.register(
        CommandSpec {
            name: "WHO".into(),
            shape: ArgShape::None,
            help: "WHO  --  List connected players with their idle and connected \
                   times. Staff also see locations; DARK players are hidden from \
                   non-staff."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(who_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "help".into(),
            aliases: vec!["+help".into()],
            help: "help [<command>|<topic>]  --  With no argument, list every \
                   command and every prose topic. With a command name (exact, \
                   including its @ or + prefix), show its usage, aliases, and \
                   switches; otherwise serve the matching prose topic (e.g. \
                   \"help getting-started\"). An exact command name wins over a \
                   topic of the same name. +help is the same command."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(help_cmd),
    )?;
    Ok(())
}
