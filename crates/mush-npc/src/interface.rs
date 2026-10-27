//! S6 (doc 14 §4.6; plan §3.7): the interface-ability side of an NPC turn —
//! the providers a profile names, resolved into the turn's tool set, invoked
//! on re-entry as the **asker**, audited, and the `help` provider the NPC
//! module fronts itself (Sharon Norbury's `help.topic`).

use std::sync::Arc;

use mush_core::{
    AbilityCall, Ctx, InterfaceAbility, InterfaceAbilityProvider, InterfaceError, InterfaceReply,
    ObjId, Registry, World,
};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::profile::AgentProfile;
use crate::snapshot::{CommandGuidance, InterfaceToolSpec};

/// The most interface abilities one turn offers the model.
pub const MAX_INTERFACE_TOOLS: usize = 32;

/// Maximum public role text retained from one provider.
pub const MAX_ROLE_CONTEXT_PROVIDER_BYTES: usize = 1024;
/// Maximum combined public role text, including provider labels.
pub const MAX_ROLE_CONTEXT_BYTES: usize = 2048;
/// Shared framing for dialogue and scene projections.
pub const ROLE_CONTEXT_GUIDANCE: &str = "Current public facts about your own role, supplied by the game. Use them for conversation; they grant no permission to act or to claim a proposed action has completed. Do not invent missing terms or private facts.";

/// Only the profile's named providers may supply role context. Keep the same
/// bounded, public projection for dialogue and scene decisions.
#[must_use]
pub fn role_context_for(
    reg: &Registry,
    world: &World,
    profile: &AgentProfile,
    npc: ObjId,
) -> String {
    let mut context = String::new();
    let mut seen = Vec::new();
    for id in profile
        .interfaces
        .iter()
        .take(crate::profile::MAX_PROFILE_INTERFACES)
    {
        if seen.contains(id) || id.len() > crate::profile::MAX_PROFILE_INTERFACE_BYTES {
            continue;
        }
        seen.push(id.clone());
        let Some(text) = reg
            .interface_provider(id)
            .and_then(|provider| provider.npc_context(world, npc))
        else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let heading = format!("{id}: ");
        let remaining = MAX_ROLE_CONTEXT_BYTES.saturating_sub(context.len() + heading.len() + 1);
        if remaining == 0 {
            break;
        }
        let maximum = remaining.min(MAX_ROLE_CONTEXT_PROVIDER_BYTES);
        let mut boundary = text.len().min(maximum);
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        if boundary == 0 {
            continue;
        }
        context.push_str(&heading);
        context.push_str(&text[..boundary]);
        context.push('\n');
    }
    context
}

/// The most authored command references included in one turn.
pub const MAX_COMMAND_GUIDANCE: usize = 8;
/// Maximum combined command-name and text bytes in one complete help card.
pub const MAX_COMMAND_GUIDANCE_BYTES: usize = 8 * 1024;
/// Maximum combined command-name and text bytes across all help cards.
pub const MAX_COMMAND_GUIDANCE_TOTAL_BYTES: usize = 16 * 1024;

/// Current help for the profile's authored command references, filtered as the
/// asker and restricted to public commands. These cards describe commands; they
/// grant no ability to execute them. Keep complete entries rather than truncating
/// conditions or syntax, and never carry staff help into shared NPC memory.
#[must_use]
pub fn command_guidance_for(
    reg: &Registry,
    world: &World,
    profile: &AgentProfile,
    asker: Option<ObjId>,
) -> Vec<CommandGuidance> {
    let Some(asker) = asker else {
        return Vec::new();
    };
    let mut cards: Vec<CommandGuidance> = Vec::new();
    let mut used = 0_usize;
    for name in &profile.command_refs {
        if cards.len() >= MAX_COMMAND_GUIDANCE {
            break;
        }
        let Some(entry) = mush_core::commands::info::help_entry(reg, world, asker, name) else {
            continue;
        };
        let Some(command) = entry.command.filter(|_| entry.publicly_visible) else {
            continue;
        };
        if !crate::profile::valid_command_ref(&command)
            || cards.iter().any(|card| card.command == command.as_str())
        {
            continue;
        }
        let text = format!("{}\n{}", entry.header, entry.body);
        let bytes = command.len().saturating_add(text.len());
        if bytes > MAX_COMMAND_GUIDANCE_BYTES
            || used.saturating_add(bytes) > MAX_COMMAND_GUIDANCE_TOTAL_BYTES
        {
            continue;
        }
        used += bytes;
        cards.push(CommandGuidance {
            command: command.to_string(),
            text,
        });
    }
    cards
}

/// The interface abilities `profile` fronts, from the providers the registry
/// holds (an id with no provider is skipped, never an error: content may name
/// a system the server does not compose).
#[must_use]
pub fn tools_for(reg: &Registry, profile: &AgentProfile) -> Vec<InterfaceToolSpec> {
    let mut out = Vec::new();
    for id in &profile.interfaces {
        let Some(provider) = reg.interface_provider(id) else {
            continue;
        };
        for ability in provider.abilities() {
            if out.len() >= MAX_INTERFACE_TOOLS {
                return out;
            }
            out.push(InterfaceToolSpec {
                provider: id.clone(),
                ability,
            });
        }
    }
    out
}

/// The provider fronting `ability` for `profile`, with its definition.
#[must_use]
pub fn provider_for(
    reg: &Registry,
    profile: &AgentProfile,
    ability: &str,
) -> Option<(SmolStr, Arc<dyn InterfaceAbilityProvider>, InterfaceAbility)> {
    for id in &profile.interfaces {
        let Some(provider) = reg.interface_provider(id) else {
            continue;
        };
        if let Some(def) = provider.abilities().into_iter().find(|a| a.name == ability) {
            return Some((id.clone(), provider, def));
        }
    }
    None
}

/// The commands the NPC's providers stand in for, in order, unique.
#[must_use]
pub fn commands_for(reg: &Registry, profile: &AgentProfile) -> Vec<SmolStr> {
    let mut out: Vec<SmolStr> = Vec::new();
    for spec in tools_for(reg, profile) {
        if !out.contains(&spec.ability.command) {
            out.push(spec.ability.command);
        }
    }
    out
}

/// A fallback line that names the command doing the same thing (rule 4),
/// when the line does not already.
#[must_use]
pub fn with_command_hint(line: &str, commands: &[SmolStr]) -> String {
    if commands.is_empty() || commands.iter().any(|c| line.contains(c.as_str())) {
        return line.to_string();
    }
    let named: Vec<&str> = commands.iter().take(2).map(SmolStr::as_str).collect();
    format!("{line} (Or just type {}.)", named.join(" or "))
}

/// The `help` provider: `help.topic(name)` reads current command help or one
/// prose topic as the asker's own `help <name>` would.
#[derive(Clone, Copy, Debug, Default)]
pub struct HelpInterface;

/// The `help` provider's id.
pub const HELP_INTERFACE: &str = "help";

impl InterfaceAbilityProvider for HelpInterface {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        vec![InterfaceAbility {
            name: SmolStr::new_static("help.topic"),
            description: "Read current command help (including aliases) or a prose topic, verbatim, as the asker's `help <name>` prints it. This reads instructions; it does not execute the command.".to_string(),
            params_schema: json!({
                "type": "object",
                "properties": { "name": { "type": "string", "description": "an exact command name, alias or prose topic" } },
                "required": ["name"]
            }),
            command: SmolStr::new_static("help"),
        }]
    }

    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        call: AbilityCall,
        name: &str,
        input: &Value,
    ) -> Result<InterfaceReply, InterfaceError> {
        if name != "help.topic" {
            return Err(InterfaceError::Unknown(name.to_string()));
        }
        let topic = input
            .get("name")
            .and_then(Value::as_str)
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| InterfaceError::Invalid("a topic name".to_string()))?;
        help_reply(ctx.reg, ctx.w, call.asker, &topic)
    }
}

fn help_reply(
    reg: &Registry,
    world: &World,
    asker: ObjId,
    topic: &str,
) -> Result<InterfaceReply, InterfaceError> {
    let entry =
        mush_core::commands::info::help_entry(reg, world, asker, topic).ok_or_else(|| {
            InterfaceError::Refused(format!(
                "There is no help topic called {topic:?}; `help` lists them."
            ))
        })?;
    let text = format!("{}\n{}", entry.header, entry.body);
    // Commands are checked under the asker's current permissions. Keep every
    // command reply private even if its help is ordinarily public, so staff
    // permissions can never turn this provider into a room disclosure path.
    Ok(if entry.command.is_some() {
        InterfaceReply::text(text)
    } else {
        InterfaceReply::public(text)
    })
}

/// Abbreviate an ability's input for the audit row.
#[must_use]
pub fn abbreviate(input: &Value) -> String {
    let mut s = input.to_string();
    if s.len() > 200 {
        let mut cut = 200;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
    s
}

/// The asker an ability runs as, or none for an ambient turn.
#[must_use]
pub fn asker_of(asker: Option<ObjId>) -> Option<ObjId> {
    asker
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{CommandSpec, GOD, LIMBO, ObjKind, ReplyAudience, Role};

    fn setup() -> (World, Registry, ObjId) {
        let mut world = World::new_minimal();
        let player = world
            .create(ObjKind::Player, "Captain", GOD, Some(LIMBO))
            .unwrap();
        (world, Registry::new(), player)
    }

    fn register(reg: &mut Registry, spec: CommandSpec) {
        reg.register(spec, Arc::new(|_ctx: &mut Ctx<'_>| Ok(())))
            .unwrap();
    }

    #[test]
    fn command_help_is_private_and_aliases_share_the_canonical_help() {
        let (world, mut reg, player) = setup();
        register(
            &mut reg,
            CommandSpec {
                name: "+coop".into(),
                aliases: vec!["+cooperative".into()],
                switches: vec!["enrol".into()],
                help: "+coop/enrol <name> signs a charter at the desk.".into(),
                ..CommandSpec::default()
            },
        );
        reg.add_help_topic("+cooperative", "An older prose topic", "This must not win.");
        reg.add_help_topic("corridor", "The corridor", "These roads are public.");
        let canonical = help_reply(&reg, &world, player, "+coop").unwrap();
        let alias = help_reply(&reg, &world, player, "+cooperative").unwrap();
        assert_eq!(canonical, alias);
        assert_eq!(canonical.audience, ReplyAudience::Asker);
        assert!(canonical.text.contains("/enrol <name>"));
        assert!(!canonical.text.contains("must not win"));
        let prose = help_reply(&reg, &world, player, "corridor").unwrap();
        assert_eq!(prose.audience, ReplyAudience::Room);
        assert_eq!(prose.text, "The corridor\nThese roads are public.");
    }

    #[test]
    fn hidden_help_uses_the_unknown_reply_and_staff_help_stays_private() {
        let (world, mut reg, player) = setup();
        register(
            &mut reg,
            CommandSpec {
                name: "+staff".into(),
                min_role: Role::Wizard,
                help: "Sensitive operator instructions.".into(),
                ..CommandSpec::default()
            },
        );
        for name in ["+staff", "+unknown"] {
            assert_eq!(
                help_reply(&reg, &world, player, name),
                Err(InterfaceError::Refused(format!(
                    "There is no help topic called {name:?}; `help` lists them."
                )))
            );
        }
        let staff = help_reply(&reg, &world, GOD, "+staff").unwrap();
        assert_eq!(staff.audience, ReplyAudience::Asker);
        assert!(staff.text.contains("Sensitive operator instructions."));
    }

    #[test]
    fn guidance_only_contains_public_current_commands_not_topics_or_authority() {
        let (mut world, mut reg, player) = setup();
        register(
            &mut reg,
            CommandSpec {
                name: "+coop".into(),
                aliases: vec!["+cooperative".into()],
                help: "Current registered terms.".into(),
                ..CommandSpec::default()
            },
        );
        for spec in [
            CommandSpec {
                name: "+staff".into(),
                min_role: Role::Wizard,
                ..CommandSpec::default()
            },
            CommandSpec {
                name: "+power".into(),
                power: Some(mush_core::Powers::SIM_ADMIN),
                ..CommandSpec::default()
            },
            CommandSpec {
                name: "+privilege".into(),
                privilege: Some(mush_core::Privileges::DISCORD_IDENTITY),
                ..CommandSpec::default()
            },
            CommandSpec {
                name: "+locked".into(),
                lock: Some(mush_core::LockExpr::Is(GOD)),
                ..CommandSpec::default()
            },
        ] {
            register(&mut reg, spec);
        }
        reg.add_help_topic("topic", "Public prose", "Not a command.");
        let profile = AgentProfile {
            command_refs: [
                "+cooperative",
                "+coop",
                "+staff",
                "+power",
                "+privilege",
                "+locked",
                "topic",
                "+missing",
            ]
            .into_iter()
            .map(SmolStr::new)
            .collect(),
            ..AgentProfile::default()
        };
        world.get_mut(player).unwrap().powers = mush_core::Powers::SIM_ADMIN;
        world.get_mut(player).unwrap().privileges = mush_core::Privileges::DISCORD_IDENTITY;
        for asker in [player, GOD] {
            let cards = command_guidance_for(&reg, &world, &profile, Some(asker));
            assert_eq!(cards.len(), 1);
            assert_eq!(cards[0].command, "+coop");
            assert!(cards[0].text.ends_with("Current registered terms."));
        }
        assert!(command_guidance_for(&reg, &world, &profile, None).is_empty());
    }

    #[test]
    fn guidance_keeps_complete_utf8_cards_within_count_and_byte_limits() {
        let (world, mut reg, player) = setup();
        let mut refs = Vec::new();
        for n in 0..12 {
            let name = format!("+guide{n}");
            register(
                &mut reg,
                CommandSpec {
                    name: name.clone().into(),
                    help: "A complete command description.".into(),
                    ..CommandSpec::default()
                },
            );
            refs.push(name.into());
        }
        let mut profile = AgentProfile {
            command_refs: refs,
            ..AgentProfile::default()
        };
        assert_eq!(
            command_guidance_for(&reg, &world, &profile, Some(player)).len(),
            MAX_COMMAND_GUIDANCE
        );

        profile.command_refs.clear();
        for (name, body) in [
            ("+oversized", "界".repeat(MAX_COMMAND_GUIDANCE_BYTES)),
            ("+largea", "界".repeat(2_600)),
            ("+largeb", "界".repeat(2_600)),
            ("+largec", "界".repeat(2_600)),
            ("+small", "The complete final instruction.".to_owned()),
        ] {
            register(
                &mut reg,
                CommandSpec {
                    name: name.into(),
                    help: body,
                    ..CommandSpec::default()
                },
            );
            profile.command_refs.push(name.into());
        }
        let cards = command_guidance_for(&reg, &world, &profile, Some(player));
        assert_eq!(
            cards
                .iter()
                .map(|card| card.command.as_str())
                .collect::<Vec<_>>(),
            vec!["+largea", "+largeb", "+small"]
        );
        assert!(
            cards
                .iter()
                .all(|card| card.command.len() + card.text.len() <= MAX_COMMAND_GUIDANCE_BYTES)
        );
        assert!(
            cards
                .iter()
                .map(|card| card.command.len() + card.text.len())
                .sum::<usize>()
                <= MAX_COMMAND_GUIDANCE_TOTAL_BYTES
        );
        for card in cards {
            let entry =
                mush_core::commands::info::help_entry(&reg, &world, player, &card.command).unwrap();
            assert_eq!(card.text, format!("{}\n{}", entry.header, entry.body));
        }
    }
}
