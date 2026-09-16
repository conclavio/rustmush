//! NPC grounding: the sheet as a capability profile, and the one ability.
//!
//! R7 deletes `wod_roll` and `sr_roll`; [`CHECK_ABILITY`] replaces both. An NPC
//! asks for a `check` by rating, difficulty and modifier; the **engine** rolls
//! it, on the world task, against the NPC's own sheet, after the model's turn
//! has already ended (design 03 §6.4). The model never computes an outcome and
//! never reaches another object's sheet.
//!
//! ```text
//! { "rating": "gunnery", "difficulty": 7, "modifier": -1 }
//!   -> 3d6 = 11, -1 => 10 vs 7  ->  ok
//! ```
//!
//! [`Ruleset::projected_ability`](mush_ruleset::Ruleset::projected_ability) is
//! deliberately left at its `None` default: `check` maps to no registered kernel
//! action yet, so it stays on the legacy invoke path — one residual row, which
//! `crates/mush-contract-tests/tests/npc_ability_residual.rs` can only watch
//! shrink.

use std::sync::Arc;

use mush_core::{ObjId, Outbox, World};
use mush_proto::{NarrativeKind, OutputEvent};
use mush_ruleset::schema::SheetSchema;
use mush_ruleset::{
    AbilityError, AbilityOutcome, AgentCapabilityProfile, AgentToolSpec, RollSide, SheetDoc,
    SheetStore, roll_narration,
};
use smol_str::SmolStr;

use crate::sheet::roll::{dice_text, roll_expression};
use crate::sheet::schema::{FIELD_CALLSIGN, FIELD_CONCEPT, RATINGS, is_rating};
use crate::sheet::{NS, PilotProfile, PilotSheetState};

/// The one ability name an NPC may request.
pub const CHECK_ABILITY: &str = "check";

/// The difficulty a `check` uses when the model does not name one.
pub const DEFAULT_DIFFICULTY: u8 = 7;
/// The highest difficulty a request may name; anything above clamps here.
pub const MAX_DIFFICULTY: u8 = 30;
/// The largest modifier magnitude a request may name; anything beyond clamps.
pub const MODIFIER_BOUND: i8 = 10;

/// The validated input of a [`CHECK_ABILITY`] request.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CheckInput {
    /// Which rating to throw: `gunnery`, `piloting` or `sensors`.
    pub rating: String,
    /// The target the total must reach.
    #[serde(default = "default_difficulty")]
    pub difficulty: u8,
    /// A situational modifier added to the total.
    #[serde(default)]
    pub modifier: i8,
}

fn default_difficulty() -> u8 {
    DEFAULT_DIFFICULTY
}

/// The tool manifest entry the LLM layer exposes for [`CHECK_ABILITY`].
#[must_use]
pub fn check_tool_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: SmolStr::new(CHECK_ABILITY),
        description: format!(
            "Throw one of your pilot ratings against a difficulty. The engine rolls \
             <rating>d6, adds the modifier, and compares the total. Default difficulty \
             {DEFAULT_DIFFICULTY}."
        ),
        params_schema: serde_json::json!({
            "type": "object",
            "required": ["rating"],
            "properties": {
                "rating": { "type": "string", "enum": RATINGS },
                "difficulty": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_DIFFICULTY,
                    "default": DEFAULT_DIFFICULTY,
                },
                "modifier": {
                    "type": "integer",
                    "minimum": -MODIFIER_BOUND,
                    "maximum": MODIFIER_BOUND,
                    "default": 0,
                },
            },
        }),
    }
}

/// Ground one NPC's sheet for the agent host: a prose line, the three ratings,
/// capability tags, and the one tool.
///
/// The concept is the character's own free text and is carried in `summary` and
/// `stats`, where it is prose the model reads — never in a `tag`, which is a
/// machine-matched key the game's own code compares against.
#[must_use]
pub fn agent_profile(sheet: &SheetDoc) -> AgentCapabilityProfile {
    let profile = PilotProfile::from_sheet(sheet);
    let callsign = sheet.text(FIELD_CALLSIGN).unwrap_or("").trim();
    let concept = sheet.text(FIELD_CONCEPT).unwrap_or("").trim();

    let mut summary = if callsign.is_empty() {
        "A pilot.".to_string()
    } else {
        format!("A pilot, callsign {callsign}.")
    };
    summary.push_str(&format!(
        " Gunnery {}, piloting {}, sensors {}.",
        profile.gunnery, profile.piloting, profile.sensors
    ));
    if !concept.is_empty() {
        summary.push(' ');
        summary.push_str(concept);
    }

    let tags: Vec<SmolStr> = RATINGS
        .iter()
        .filter(|rating| profile.rating(rating).is_some_and(|dots| dots >= 3))
        .map(|rating| SmolStr::new(format!("rating:{rating}")))
        .collect();

    AgentCapabilityProfile {
        summary,
        stats: serde_json::json!({
            "callsign": callsign,
            "concept": concept,
            "ratings": {
                "gunnery": profile.gunnery,
                "piloting": profile.piloting,
                "sensors": profile.sensors,
            },
        }),
        tags,
        tools: vec![check_tool_spec()],
    }
}

/// Invoke `name` for `npc`. Exactly one name resolves.
///
/// # Errors
///
/// [`AbilityError::Unknown`] for any name but [`CHECK_ABILITY`];
/// [`AbilityError::Invalid`] for input that does not decode, a `rating` that is
/// not one of the three, or a world with no sheet module loaded.
pub fn invoke(
    schema: &Arc<SheetSchema>,
    w: &mut World,
    out: &mut Outbox,
    npc: ObjId,
    name: &str,
    input: serde_json::Value,
) -> Result<AbilityOutcome, AbilityError> {
    if name != CHECK_ABILITY {
        return Err(AbilityError::Unknown(name.to_string()));
    }
    let request: CheckInput =
        serde_json::from_value(input).map_err(|error| AbilityError::Invalid(error.to_string()))?;
    if !is_rating(&request.rating) {
        return Err(AbilityError::Invalid(format!(
            "unknown rating {:?}; expected one of {RATINGS:?}",
            request.rating
        )));
    }
    // An NPC with no sheet is a green pilot rather than an error: the arena
    // cannot stop a pulse to ask staff for a sheet.
    let profile = SheetStore::new(Arc::clone(schema))
        .get(w, npc)
        .as_ref()
        .map_or_else(PilotProfile::default, PilotProfile::from_sheet);
    let dots = profile.rating(&request.rating).unwrap_or(1);
    let difficulty = i64::from(request.difficulty.clamp(1, MAX_DIFFICULTY));
    let modifier = i64::from(request.modifier.clamp(-MODIFIER_BOUND, MODIFIER_BOUND));
    let expr = format!("{dots}d6");

    let state = w
        .module_state_mut::<PilotSheetState>()
        .ok_or_else(|| AbilityError::Invalid("the pilot sheet is not loaded".to_string()))?;
    let outcome = roll_expression(state.rng(), &expr).map_err(AbilityError::Invalid)?;

    let total = outcome.total.saturating_add(modifier);
    let ok = total >= difficulty;
    let dice = dice_text(&outcome);
    let verdict = if ok { "success" } else { "failure" };
    let summary = format!(
        "{} check: {expr} [{dice}]{} = {total} vs {difficulty} — {verdict}",
        request.rating,
        if modifier == 0 {
            String::new()
        } else {
            format!(" {modifier:+}")
        }
    );

    announce(
        w,
        out,
        npc,
        &format!("{} ({expr})", request.rating),
        &dice,
        &total.to_string(),
        &format!("vs {difficulty}"),
    );

    Ok(AbilityOutcome {
        ok,
        summary,
        data: serde_json::json!({
            "rating": request.rating,
            "dots": dots,
            "dice": outcome.dice,
            "modifier": modifier,
            "total": total,
            "difficulty": difficulty,
            "success": ok,
        }),
    })
}

/// Show the NPC's check in its room, in the same shape a player's `+roll`
/// takes, so an onlooker cannot tell a resolved ability from a rolled one.
fn announce(
    w: &World,
    out: &mut Outbox,
    npc: ObjId,
    label: &str,
    dice: &str,
    result: &str,
    note: &str,
) {
    let name = w.get(npc).map_or_else(
        || npc.to_string(),
        |o| mush_core::output::primary_name(&o.name).to_string(),
    );
    let narration = roll_narration(
        &RollSide {
            name: &name,
            expr: label,
            dice,
            result,
        },
        &[note.to_string()],
    );
    match w.get(npc).and_then(|o| o.location) {
        Some(room) => out.to_room(
            room,
            Vec::new(),
            OutputEvent::Narrative {
                kind: NarrativeKind::Emit,
                actor: Some(mush_core::obj_ref(w, npc)),
                text: narration,
            },
        ),
        None => out.sys_to_actor(npc, NS, format!("{name} {label} {dice} = {result} {note}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheet::schema::{FIELD_GUNNERY, sheet_schema};

    fn sheet() -> SheetDoc {
        let mut doc = SheetDoc::new_default(&sheet_schema());
        doc.set(FIELD_CALLSIGN, serde_json::json!("Ladder"));
        doc.set(FIELD_CONCEPT, serde_json::json!("Ex-Exchange courier"));
        doc.set(FIELD_GUNNERY, serde_json::json!(4));
        doc
    }

    #[test]
    fn the_profile_reads_the_ratings_and_the_prose() {
        let profile = agent_profile(&sheet());
        assert!(profile.summary.contains("Ladder"), "{}", profile.summary);
        assert!(profile.summary.contains("Gunnery 4"), "{}", profile.summary);
        assert!(profile.summary.contains("Ex-Exchange courier"));
        assert_eq!(
            profile.stats.pointer("/ratings/gunnery"),
            Some(&serde_json::json!(4))
        );
        assert_eq!(profile.tags, vec![SmolStr::new("rating:gunnery")]);
        assert_eq!(profile.tools.len(), 1);
        assert_eq!(profile.tools[0].name.as_str(), CHECK_ABILITY);
    }

    #[test]
    fn a_blank_sheet_grounds_without_a_callsign() {
        let profile = agent_profile(&SheetDoc::new_default(&sheet_schema()));
        assert!(
            profile.summary.starts_with("A pilot."),
            "{}",
            profile.summary
        );
        assert!(profile.tags.is_empty(), "no rating is notable at 1");
    }

    #[test]
    fn the_input_defaults_the_difficulty_and_the_modifier() {
        let request: CheckInput =
            serde_json::from_value(serde_json::json!({ "rating": "piloting" }))
                .expect("a bare rating decodes");
        assert_eq!(request.difficulty, DEFAULT_DIFFICULTY);
        assert_eq!(request.modifier, 0);
    }
}
