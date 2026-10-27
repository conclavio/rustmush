//! Private NPC queries recheck the live asker and audience at delivery.
//! Answers never enter the model's snapshot or the NPC's shared memory.

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
    reg.interface_provider("npc.relationship", Arc::new(PrivateRelationship))?;
    reg.classify_commands(CommandCompatibility::ReadOnly);
    reg.command(CommandSpec {
        name: "+npc".into(), shape: ArgShape::EqPair, switches: vec!["lore".into(), "relationship".into()],
        help: "+npc/lore <npc>=<topic> -- ask a present NPC privately about an exact authored lore subject. Public facts are available to everyone; guarded facts require earned personal trust. Secret facts are never available.\n+npc/relationship <npc> -- privately read this NPC's familiarity, trust and disposition toward you, plus your standing with its public affiliation. Only your own relationship is available. These reads do not change trust or institutional standing.".into(),
        ..CommandSpec::default()
    }, Arc::new(command))?;
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    Ok(())
}

struct PrivateLore;

struct PrivateRelationship;

impl InterfaceAbilityProvider for PrivateRelationship {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        vec![InterfaceAbility {
            name: "npc.relationship.read".into(),
            description: "Privately read your current familiarity, trust and disposition toward the current asker, separately from their public standing with your openly represented faction. No parameters or third-party queries. The answer goes only to the asker, never your model context or the room; do not guess ratings, reasons, or private records.".into(),
            params_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
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
        if name != "npc.relationship.read" {
            return Err(InterfaceError::Unknown(name.to_owned()));
        }
        if input.as_object().is_none_or(|object| !object.is_empty()) {
            return Err(InterfaceError::Invalid(
                "relationship queries take no parameters".into(),
            ));
        }
        relationship_reply(ctx, call)
    }
}

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
    let relationship = ctx.switches.has("relationship");
    if relationship == ctx.switches.has("lore") || (relationship && ctx.args.rhs().is_some()) {
        return Err(CmdError::msg(
            "Usage: +npc/lore <npc>=<topic> or +npc/relationship <npc>.",
        ));
    }
    let unavailable = if relationship {
        "No relationship is available here."
    } else {
        "No lore is available on that topic."
    };
    let npc = mush_core::match_object(ctx.w, ctx.actor, ctx.args.lhs().trim())
        .map_err(|_| CmdError::msg(unavailable))?;
    let topic = ctx.args.rhs().unwrap_or("").to_owned();
    let call = AbilityCall {
        npc,
        asker: ctx.actor,
    };
    let result = if relationship {
        relationship_reply(ctx, call)
    } else {
        reply(ctx, call, &topic)
    }
    .map_err(|error| CmdError::msg(error.to_string()))?;
    ctx.out.info(ctx.actor, result.text);
    Ok(())
}

fn live_audience(ctx: &Ctx<'_>, call: AbilityCall) -> bool {
    let Some(npc) = ctx
        .w
        .get(call.npc)
        .filter(|object| object.kind == ObjKind::Npc && !object.flags.contains(Flags::GOING))
    else {
        return false;
    };
    let Some(asker) = ctx
        .w
        .get(call.asker)
        .filter(|object| object.kind == ObjKind::Player && !object.flags.contains(Flags::GOING))
    else {
        return false;
    };
    ctx.w.is_connected(call.asker)
        && npc.location.is_some()
        && npc.location == asker.location
        && ctx.can_perceive(call.npc, call.asker)
        && ctx.can_perceive(call.asker, call.npc)
        && crate::module::read_profile(ctx.w, call.npc).is_some_and(|profile| profile.enabled)
}

fn relationship_reply(ctx: &Ctx<'_>, call: AbilityCall) -> Result<InterfaceReply, InterfaceError> {
    let unavailable = || InterfaceError::Refused("No relationship is available here.".into());
    if !live_audience(ctx, call) {
        return Err(unavailable());
    }
    let host = ctx.state::<NpcHostState>().ok_or_else(unavailable)?;
    let personal = host.relationships.personal(call.npc, call.asker);
    let familiarity = match host.relationships.familiarity(call.npc, call.asker) {
        crate::Familiarity::Stranger => "stranger",
        crate::Familiarity::Acquaintance => "acquaintance",
        crate::Familiarity::Known => "known",
    };
    let mut text = format!(
        "Your relationship with this NPC:\nFamiliarity: {familiarity}.\nTrust: {}.\nDisposition: {}.",
        personal.trust_band(),
        personal.disposition_band(),
    );
    if let Some(standing) = crate::public_faction_for(ctx.w, call.npc)
        .and_then(|faction| mush_faction::public_standing(ctx.w, faction, call.asker))
    {
        let name: String = standing.name.chars().take(128).collect();
        text.push_str(&format!(
            "\nPublic standing with {name}: {}. Use +standing for details.",
            standing.band
        ));
    }
    text.push_str(
        "\nPersonal feelings and public reputation are separate; neither grants permissions.",
    );
    Ok(InterfaceReply::text(text))
}

fn reply(ctx: &Ctx<'_>, call: AbilityCall, topic: &str) -> Result<InterfaceReply, InterfaceError> {
    let unavailable = || InterfaceError::Refused("No lore is available on that topic.".into());
    let topic = topic.trim();
    if topic.is_empty() || topic.len() > 128 {
        return Err(InterfaceError::Invalid(
            "give an exact lore subject of at most 128 bytes".into(),
        ));
    }
    if !live_audience(ctx, call) {
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
