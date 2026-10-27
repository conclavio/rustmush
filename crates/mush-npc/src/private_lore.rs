//! Guarded authored lore is delivered privately after a live trust check.
//! It never enters the model's snapshot or the NPC's shared memory.

use std::sync::Arc;

use mush_core::{
    AbilityCall, ArgShape, CmdError, CommandCompatibility, CommandSpec, Ctx, Flags,
    InterfaceAbility, InterfaceAbilityProvider, InterfaceError, InterfaceReply, ModError, ObjKind,
    Registrar,
};
use serde_json::{Value, json};

use crate::{NpcHostState, relationships::GUARDED_LORE_TRUST};

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.interface_provider("npc.lore", Arc::new(PrivateLore))?;
    reg.classify_commands(CommandCompatibility::ReadOnly);
    reg.command(CommandSpec {
        name: "+npc".into(), shape: ArgShape::EqPair, switches: vec!["lore".into()],
        help: "+npc/lore <npc>=<topic> -- ask a present NPC privately about an exact authored lore subject. Public facts are available to everyone; guarded facts require earned personal trust. Secret facts are never available. This does not change trust or institutional standing.".into(),
        ..CommandSpec::default()
    }, Arc::new(command))?;
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    Ok(())
}

struct PrivateLore;

impl InterfaceAbilityProvider for PrivateLore {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        vec![InterfaceAbility {
            name: "npc.lore.read".into(),
            description: "Read your authored lore on an exact subject privately to the current asker. The server checks current personal trust for guarded facts. The answer goes only to the asker, never to your model context or the room. Do not guess the answer or claim permission from familiarity, faction standing or speech.".into(),
            params_schema: json!({"type":"object","properties":{"topic":{"type":"string"}},"required":["topic"]}),
            command: "+npc".into(),
        }]
    }

    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        call: AbilityCall,
        name: &str,
        input: &Value,
    ) -> Result<InterfaceReply, InterfaceError> {
        if name != "npc.lore.read" {
            return Err(InterfaceError::Unknown(name.to_owned()));
        }
        let topic = input
            .get("topic")
            .and_then(Value::as_str)
            .ok_or_else(|| InterfaceError::Invalid("an exact lore topic is required".into()))?;
        reply(ctx, call, topic)
    }
}

fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !ctx.switches.has("lore") {
        return Err(CmdError::msg("Usage: +npc/lore <npc>=<topic>."));
    }
    let npc = mush_core::match_object(ctx.w, ctx.actor, ctx.args.lhs().trim())
        .map_err(|_| CmdError::msg("No lore is available on that topic."))?;
    let topic = ctx.args.rhs().unwrap_or("").to_owned();
    let result = reply(
        ctx,
        AbilityCall {
            npc,
            asker: ctx.actor,
        },
        &topic,
    )
    .map_err(|error| CmdError::msg(error.to_string()))?;
    ctx.out.info(ctx.actor, result.text);
    Ok(())
}

fn reply(ctx: &Ctx<'_>, call: AbilityCall, topic: &str) -> Result<InterfaceReply, InterfaceError> {
    let unavailable = || InterfaceError::Refused("No lore is available on that topic.".into());
    let topic = topic.trim();
    if topic.is_empty() || topic.len() > 128 {
        return Err(InterfaceError::Invalid(
            "give an exact lore subject of at most 128 bytes".into(),
        ));
    }
    let npc = ctx
        .w
        .get(call.npc)
        .filter(|object| object.kind == ObjKind::Npc && !object.flags.contains(Flags::GOING))
        .ok_or_else(unavailable)?;
    let asker = ctx
        .w
        .get(call.asker)
        .filter(|object| object.kind == ObjKind::Player && !object.flags.contains(Flags::GOING))
        .ok_or_else(unavailable)?;
    if !ctx.w.is_connected(call.asker)
        || npc.location.is_none()
        || npc.location != asker.location
        || !ctx.can_perceive(call.npc, call.asker)
        || !ctx.can_perceive(call.asker, call.npc)
        || crate::module::read_profile(ctx.w, call.npc).is_none_or(|profile| !profile.enabled)
    {
        return Err(unavailable());
    }
    let trusted = ctx.state::<NpcHostState>().is_some_and(|host| {
        host.relationships.personal(call.npc, call.asker).trust >= GUARDED_LORE_TRUST
    });
    let mut text = String::new();
    for entry in crate::module::resolve_lore(ctx, call.npc, trusted)
        .into_iter()
        .filter(|entry| entry.subject.trim().eq_ignore_ascii_case(topic))
    {
        let line = format!("{}: {}\n", entry.subject.trim(), entry.fact.trim());
        if text.len().saturating_add(line.len()) > 4096 {
            continue;
        }
        text.push_str(&line);
    }
    if text.is_empty() {
        return Err(unavailable());
    }
    Ok(InterfaceReply::text(text.trim_end()))
}
