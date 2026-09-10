//! S6 (doc 14 §4.6): the set designer's interface — `build.propose`,
//! `build.status` and `build.answer` over the authoring agent. The
//! conversation *is* the proposal record's question loop; the proposal is the
//! asker's, its run is charged to the asker's actor budget under
//! `worldgen_plan`, and the NPC never applies anything (`+build/go` is the
//! asker's own typed command).

use mush_core::{
    AbilityCall, Ctx, InterfaceAbility, InterfaceAbilityProvider, InterfaceError, InterfaceReply,
};
use serde_json::{Value, json};
use smol_str::SmolStr;

use mush_atlas::AtlasState;
use mush_atlas::hydrate::{ProposeRequest, ProposeScope, ProposeSource};

/// The provider id.
pub const BUILD_INTERFACE: &str = "build";

/// The set designer.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuildInterface;

impl InterfaceAbilityProvider for BuildInterface {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        vec![
            InterfaceAbility {
                name: SmolStr::new_static("build.propose"),
                description: "Turn the asker's brief for a place into a proposal the building agent plans (in the asker's name, on the asker's budget); nothing is built until the asker types +build/go."
                    .to_string(),
                params_schema: json!({
                    "type": "object",
                    "properties": { "brief": { "type": "string", "description": "what the place is for and what it looks like" } },
                    "required": ["brief"]
                }),
                command: SmolStr::new_static("+build"),
            },
            InterfaceAbility {
                name: SmolStr::new_static("build.status"),
                description: "The asker's own proposals and where each stands, with any open questions."
                    .to_string(),
                params_schema: json!({ "type": "object", "properties": {} }),
                command: SmolStr::new_static("+build"),
            },
            InterfaceAbility {
                name: SmolStr::new_static("build.answer"),
                description: "Answer one of the agent's open questions on the asker's proposal."
                    .to_string(),
                params_schema: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "the proposal" },
                        "n": { "type": "integer", "description": "the question number" },
                        "text": { "type": "string" }
                    },
                    "required": ["id", "n", "text"]
                }),
                command: SmolStr::new_static("+build"),
            },
        ]
    }

    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        call: AbilityCall,
        name: &str,
        input: &Value,
    ) -> Result<InterfaceReply, InterfaceError> {
        match name {
            "build.propose" => {
                let brief = input
                    .get("brief")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|b| !b.is_empty())
                    .ok_or_else(|| InterfaceError::Invalid("a brief".to_string()))?;
                // The asker's own neighborhood is the scope, else the world.
                let here = ctx.w.get(call.asker).and_then(|o| o.location);
                let scope = here
                    .and_then(|room| {
                        ctx.state::<AtlasState>().and_then(|st| {
                            st.records
                                .neighborhood_of_room(room)
                                .map(|nb| nb.key.clone())
                        })
                    })
                    .map_or(ProposeScope::World, ProposeScope::Neighborhood);
                let line = crate::proposal::propose_hook(
                    ctx,
                    ProposeRequest {
                        author: call.asker,
                        scope,
                        source: ProposeSource::Prose(brief.to_string()),
                    },
                )
                .map_err(InterfaceError::Refused)?;
                Ok(InterfaceReply::text(line))
            }
            "build.status" => {
                let lines = crate::proposal::status_lines(ctx, call.asker);
                if lines.is_empty() {
                    Ok(InterfaceReply::text("You have no proposals on the desk."))
                } else {
                    Ok(InterfaceReply::lines(&lines))
                }
            }
            "build.answer" => {
                let id = input
                    .get("id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| InterfaceError::Invalid("a proposal id".to_string()))?;
                let n = input
                    .get("n")
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| InterfaceError::Invalid("a question number".to_string()))?;
                let text = input
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .ok_or_else(|| InterfaceError::Invalid("an answer".to_string()))?;
                let line = crate::proposal::answer(ctx, id, call.asker, n, text)
                    .map_err(InterfaceError::Refused)?;
                Ok(InterfaceReply::text(line))
            }
            other => Err(InterfaceError::Unknown(other.to_string())),
        }
    }
}
