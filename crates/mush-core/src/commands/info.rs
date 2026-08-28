//! Info commands: `WHO` (connected players with real idle/connect times
//! from the world's session stats) and `help`/`+help` (rendered from every
//! registered [`CommandSpec`]'s real help text). The same WHO view also
//! backs the `"who"` query handler (web WHO-rail hydration, M4).

use std::sync::Arc;

use mush_proto::{OutputEvent, RichText, WhoEntry};

use crate::cmd::{ArgShape, CmdError, CommandSpec, Ctx, Registry, RegistryError};
use crate::flags::{Flags, Powers, Role};
use crate::module::{QueryHandler, Registrar};
use crate::obj::ObjId;
use crate::perm;
use crate::world::World;

use super::attr_text;

/// The WHO listing as seen by `viewer`: connected actors with real
/// idle/connect times from the engine-maintained session stats. Staff
/// (Wizard+ or `SEE_HIDDEN`) also see locations and DARK players. Shared by
/// the `WHO` command and the `"who"` query handler — one view, two
/// transports.
#[must_use]
pub fn who_entries(w: &World, viewer: ObjId) -> Vec<WhoEntry> {
    let staff =
        perm::has_role(w, viewer, Role::Wizard) || perm::has_power(w, viewer, Powers::SEE_HIDDEN);
    let mut entries = Vec::new();
    for id in w.connected() {
        let Some(o) = w.get(id) else { continue };
        if o.flags.contains(Flags::DARK) && !staff && id != viewer {
            continue;
        }
        let location = if staff {
            o.location.and_then(|l| w.get(l)).map(|r| r.name.clone())
        } else {
            None
        };
        // Idle/connect times come from the engine-maintained session stats
        // (clamped to 0 against clock skew); actors marked connected by
        // other means (tests, NPCs) report zeros.
        let now = w.now();
        let (idle_s, conn_s) = w.session_stat(id).map_or((0, 0), |s| {
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
            doing: attr_text(w, id, "doing"),
        });
    }
    entries
}

fn who_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let entries = who_entries(ctx.w, ctx.actor);
    ctx.out.to_actor(ctx.actor, OutputEvent::Who(entries));
    Ok(())
}

/// The `"who"` query handler (web `Req { q: Who }` hydration): the same
/// listing the `WHO` command produces, wrapped by the engine in a `Reply`.
struct WhoQuery;

impl QueryHandler for WhoQuery {
    fn query(
        &self,
        w: &World,
        actor: ObjId,
        _params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        Ok(OutputEvent::Who(who_entries(w, actor)))
    }
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

/// Register `WHO` and `help` (plus the core `"who"` query handler).
pub(super) fn register(reg: &mut Registry) -> Result<(), RegistryError> {
    // Core query handlers register under the reserved module identity
    // "core". This runs before any game module registers, so a collision is
    // a programming error, not a runtime condition.
    Registrar::new(reg, "core")
        .query_handler("who", Arc::new(WhoQuery))
        .expect("core query handler registration must not collide");
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
