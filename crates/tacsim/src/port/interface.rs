//! Read-only Co-op abilities for an NPC acting as the speaking player's clerk.

use mush_core::{
    AbilityCall, Ctx, InterfaceAbility, InterfaceAbilityProvider, InterfaceError, InterfaceReply,
};
use serde_json::{Value, json};

/// The provider id named in a clerk's NPC profile.
pub const COOP_INTERFACE: &str = "coop";

/// The same personalized reads as `+coop` and `+coop/statement`.
#[derive(Clone, Copy, Debug, Default)]
pub struct CoopInterface;

impl InterfaceAbilityProvider for CoopInterface {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        [
            (
                "coop.terms",
                "+coop",
                "Read the Co-op's programme terms, available hulls and the asker's current charter status. This does not enrol or buy anything.",
            ),
            (
                "coop.statement",
                "+coop/statement",
                "Read the asker's own charter statement, frozen buyout terms, trading counters and available cash. This does not pay or buy anything.",
            ),
        ]
        .into_iter()
        .map(|(name, command, description)| InterfaceAbility {
            name: name.into(),
            command: command.into(),
            description: description.to_owned(),
            params_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        })
        .collect()
    }

    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        call: AbilityCall,
        name: &str,
        input: &Value,
    ) -> Result<InterfaceReply, InterfaceError> {
        if !matches!(name, "coop.terms" | "coop.statement") {
            return Err(InterfaceError::Unknown(name.to_owned()));
        }
        if !input.as_object().is_some_and(serde_json::Map::is_empty) {
            return Err(InterfaceError::Invalid(
                "Co-op reads take an empty object; they always read the asker's own books.".into(),
            ));
        }
        // The speaking player is authoritative even if a caller supplies a
        // context whose actor is the NPC or another character. These projections
        // cannot dispatch commands, emit output or mutate world state.
        let previous_actor = ctx.actor;
        ctx.actor = call.asker;
        let result = match name {
            "coop.terms" => super::coop::terms_lines(ctx),
            _ => super::coop::statement_lines(ctx),
        };
        ctx.actor = previous_actor;
        result
            .map(|lines| InterfaceReply::lines(&lines))
            .map_err(|error| InterfaceError::Refused(error.to_string()))
    }
}
