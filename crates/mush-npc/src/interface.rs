//! S6 (doc 14 §4.6; plan §3.7): the interface-ability side of an NPC turn —
//! the providers a profile names, resolved into the turn's tool set, invoked
//! on re-entry as the **asker**, audited, and the `help` provider the NPC
//! module fronts itself (Sharon Norbury's `help.topic`).

use std::sync::Arc;

use mush_core::{
    AbilityCall, Ctx, InterfaceAbility, InterfaceAbilityProvider, InterfaceError, InterfaceReply,
    ObjId, Registry,
};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::profile::AgentProfile;
use crate::snapshot::InterfaceToolSpec;

/// The most interface abilities one turn offers the model.
pub const MAX_INTERFACE_TOOLS: usize = 32;

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

/// The `help` provider: `help.topic(name)` reads one topic verbatim, as
/// `help <name>` prints it for anyone.
#[derive(Clone, Copy, Debug, Default)]
pub struct HelpInterface;

/// The `help` provider's id.
pub const HELP_INTERFACE: &str = "help";

impl InterfaceAbilityProvider for HelpInterface {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        vec![InterfaceAbility {
            name: SmolStr::new_static("help.topic"),
            description: "Read one help topic, verbatim, as `help <name>` prints it.".to_string(),
            params_schema: json!({
                "type": "object",
                "properties": { "name": { "type": "string", "description": "the topic" } },
                "required": ["name"]
            }),
            command: SmolStr::new_static("help"),
        }]
    }

    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        _call: AbilityCall,
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
        match ctx.reg.help_topic(&topic) {
            Some(t) => Ok(InterfaceReply::text(format!("{}\n{}", t.title, t.body))),
            None => Err(InterfaceError::Refused(format!(
                "There is no help topic called {topic:?}; `help` lists them."
            ))),
        }
    }
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
