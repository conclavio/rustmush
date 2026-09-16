//! The off-loop room generator: prompt assembly, the forced-tool call, and
//! the lenient-parse + one-shot repair loop (design 03 §§5.2, 5.4).
//!
//! This is the **only** async code in the crate. It holds no world
//! reference: it consumes a [`GenSnapshot`] (owned data snapshotted
//! synchronously by the `@gen` handler) and produces a [`GenOutcome`]
//! (serialized back into the world as a typed completion). `mushd`'s world
//! task calls [`run_worldgen_async`] with its `LlmService` handle when it
//! drains the `Effect::ModuleAsync` the handler deferred.

use mush_llm::{
    LlmFeature, LlmService, Priority, StructuredError, StructuredSpec, ToolDef, call_structured,
};

use crate::data::{
    BriefRoomFact, BriefSnapshot, BuildingBriefContext, BuildingBriefPayload, DraftPayload,
    GenOutcome, GenSnapshot, RoomConcept, WorldgenReq,
};
use mush_atlas::theme::Theme;

/// The forced tool name (the portable structured-output mechanism).
const TOOL_NAME: &str = "submit_room";

/// The forced tool name for the building-brief pass.
const BRIEF_TOOL_NAME: &str = "submit_building_brief";

/// Run one worldgen request end-to-end and return the serialized
/// [`GenOutcome`]. This is the entry point the driver invokes off the world
/// loop: `payload` is a serialized [`GenSnapshot`]; the returned string is a
/// serialized [`GenOutcome`], carried back as the
/// [`WorldCommand::ModuleAsyncDone`](mush_core::engine::WorldCommand::ModuleAsyncDone)
/// payload.
pub async fn run_worldgen_async(llm: &LlmService, payload: &str) -> String {
    // Batch work is wrapped in a `WorldgenReq` envelope (a `"wg"` tag); a
    // single-room `@gen` still sends a bare `GenSnapshot`, which has no tag and
    // falls through to the historical path — so the M7 wire format is unchanged.
    let outcome = if let Ok(req) = serde_json::from_str::<WorldgenReq>(payload) {
        match req {
            WorldgenReq::Brief(snap) => generate_brief(llm, &snap).await,
            WorldgenReq::BuildingRoom {
                snap,
                building,
                brief,
            } => generate_room(llm, &snap, Some(building), Some(&brief)).await,
            WorldgenReq::Plan(snap) => GenOutcome::Plan {
                outcome: crate::plan::run_plan(llm, &snap).await,
            },
        }
    } else {
        match serde_json::from_str::<GenSnapshot>(payload) {
            Ok(snap) => generate_room(llm, &snap, None, None).await,
            Err(e) => {
                // We serialized this ourselves; a decode failure is a bug. There
                // is no room id to attribute it to, so report against #0.
                tracing::error!(error = %e, "worldgen: undecodable snapshot payload");
                GenOutcome::Err {
                    room: mush_core::ObjId(0),
                    builder: mush_core::ObjId(0),
                    message: format!("internal error decoding worldgen request: {e}"),
                    retryable: false,
                    building: None,
                    budget: false,
                }
            }
        }
    };
    serde_json::to_string(&outcome).unwrap_or_else(|e| {
        // Serializing a GenOutcome cannot realistically fail; keep the world
        // re-entry robust regardless.
        tracing::error!(error = %e, "worldgen: unserializable outcome");
        String::from(
            r#"{"status":"err","room":0,"builder":0,"message":"internal error","retryable":false}"#,
        )
    })
}

/// The single-room generation with a one-shot malformed-output repair.
/// `building`/`brief` are `Some` for a batch room (the brief context is
/// woven into the prompt and the outcome carries the batch id back).
async fn generate_room(
    llm: &LlmService,
    snap: &GenSnapshot,
    building: Option<mush_core::ObjId>,
    brief: Option<&BuildingBriefContext>,
) -> GenOutcome {
    let spec = StructuredSpec {
        feature: LlmFeature::WorldgenRoom,
        // A batch room is bulk work (background semaphore); a single-room `@gen`
        // is interactive (the builder is waiting on it).
        priority: if building.is_some() {
            Priority::Background
        } else {
            Priority::Interactive
        },
        actor: Some(snap.builder),
        system: system_prompt(),
        user: user_prompt_inner(snap, brief),
        tool: submit_room_tool(),
        max_tokens: 2048,
        repair_hint: "`desc` is required and must be a non-empty string.".to_string(),
    };
    match call_structured::<DraftPayload, _>(llm, &spec, validate).await {
        Ok(ok) => GenOutcome::Ok {
            room: snap.room,
            builder: snap.builder,
            draft: ok.value,
            prose: snap.prose.clone(),
            prompt_hash: ok.prompt_hash,
            model: ok.model,
            building,
        },
        Err(e) => {
            let (message, retryable, budget) = match &e {
                StructuredError::Malformed { last, .. } => (
                    format!("The model returned malformed output twice ({last}). Try @gen/retry."),
                    true,
                    false,
                ),
                StructuredError::Refused { category, .. } => {
                    let cat = category
                        .as_ref()
                        .map(|c| format!(" ({c})"))
                        .unwrap_or_default();
                    (
                        format!(
                            "The model declined to describe this room{cat}. Try rephrasing \
                             with @gen/retry <guidance>."
                        ),
                        false,
                        false,
                    )
                }
                // A kill switch (global `[llm].enabled = false`, per-feature, or
                // the staff `@llm/disable`) is engaged: report it plainly rather
                // than as a generic failure. A keyless clone hits this on every `@gen`.
                StructuredError::Disabled { .. } => {
                    ("LLM features are disabled.".to_string(), false, false)
                }
                StructuredError::Call { err, .. } => (
                    format!("Generation failed: {err}."),
                    e.is_retryable(),
                    e.is_budget(),
                ),
            };
            GenOutcome::Err {
                room: snap.room,
                builder: snap.builder,
                message,
                retryable,
                building,
                budget,
            }
        }
    }
}

fn validate(draft: DraftPayload) -> Result<DraftPayload, String> {
    if draft.desc.trim().is_empty() {
        return Err("`desc` was empty".to_string());
    }
    Ok(draft)
}

// The JSON scanners moved to `mush-llm` with the structured call (16b cut R-a,
// R1); they stay reachable at their old paths.
pub use mush_llm::{all_json_objects, extract_json_object};

/// The stable, cache-friendly system prompt: the worldgen role + the output
/// contract. Theme, neighbors, facts, and the builder's prose all live in
/// the user turn (design 03 §5.2).
#[must_use]
pub fn system_prompt() -> String {
    format!(
        "You are a MUSH worldbuilding assistant. You write vivid, concise, sensory room \
         descriptions for a text-based game, in the established house style, consistent \
         with neighboring rooms.\n\n\
         Rules:\n\
         - Describe ONLY the room the builder names. Do not invent exits, rooms, objects, \
         or NPCs that are not stated in the facts.\n\
         - You may propose descriptions for the exits that already exist (listed in the \
         facts); never invent new exits or destinations.\n\
         - Honor the theme's style, vocabulary, banned words, POV, and length target.\n\
         - Return your answer by calling the `{TOOL_NAME}` tool with structured fields. If \
         tool calling is unavailable, reply with ONLY a JSON object of the same shape \
         (`desc` is required; `short_desc`, `details`, `exits` are optional). No prose \
         outside the JSON."
    )
}

/// Assemble the user turn: theme block, neighbor descs, room facts, and the
/// builder's prose (plus retry guidance / the rejected prior draft).
#[must_use]
pub fn user_prompt(snap: &GenSnapshot) -> String {
    user_prompt_inner(snap, None)
}

/// The user-turn assembly, with an optional building-brief block prepended
/// (present for a batch room — the shared palette + overall concept; the
/// room's own concept travels as its prose).
#[must_use]
pub fn user_prompt_inner(snap: &GenSnapshot, brief: Option<&BuildingBriefContext>) -> String {
    let mut s = String::new();
    s.push_str(&theme_block(&snap.theme));
    s.push('\n');

    if let Some(b) = brief
        && (!b.palette.trim().is_empty() || !b.building_concept.trim().is_empty())
    {
        s.push_str("== BUILDING BRIEF (this room is one room of a larger building) ==\n");
        if !b.building_concept.trim().is_empty() {
            s.push_str(&format!(
                "Overall building: {}\n",
                b.building_concept.trim()
            ));
        }
        if !b.palette.trim().is_empty() {
            s.push_str(&format!(
                "Shared palette (materials/motifs/light/sound to echo): {}\n",
                b.palette.trim()
            ));
        }
        s.push_str(
            "Keep this room consistent with the shared palette and its drafted neighbors.\n\n",
        );
    }

    s.push_str("== ROOM FACTS ==\n");
    s.push_str(&format!("Room name: {}\n", snap.facts.name));
    if snap.facts.exits.is_empty() {
        s.push_str("Exits: (none)\n");
    } else {
        s.push_str("Exits (describe only these; do not invent others):\n");
        for e in &snap.facts.exits {
            let aliases = if e.aliases.is_empty() {
                String::new()
            } else {
                format!(" [aliases: {}]", e.aliases.join(", "))
            };
            let dest = e
                .destination
                .as_deref()
                .map(|d| format!(" -> {d}"))
                .unwrap_or_else(|| " -> (unlinked)".to_string());
            s.push_str(&format!("  - {}{aliases}{dest}\n", e.name));
        }
    }
    s.push('\n');

    if !snap.neighbors.is_empty() {
        s.push_str("== NEIGHBORING ROOMS (for consistency; do not copy) ==\n");
        for n in &snap.neighbors {
            s.push_str(&format!("[{}] {}:\n{}\n\n", n.direction, n.name, n.desc));
        }
    }

    s.push_str("== BUILDER'S BRIEF ==\n");
    s.push_str(snap.prose.trim());
    s.push('\n');

    if let Some(g) = &snap.guidance
        && !g.trim().is_empty()
    {
        s.push_str("\n== ADDITIONAL GUIDANCE ==\n");
        s.push_str(g.trim());
        s.push('\n');
    }

    if let Some(prev) = &snap.prior_draft {
        s.push_str("\n== AVOID THIS (previously rejected draft) ==\n");
        s.push_str(prev.desc.trim());
        s.push_str("\nProduce a meaningfully different description.\n");
    }

    s
}

fn theme_block(theme: &Theme) -> String {
    let mut s = String::from("== THEME ==\n");
    if theme.is_empty() {
        s.push_str("(no theme configured; use a neutral, consistent style)\n");
        return s;
    }
    if !theme.name.is_empty() {
        s.push_str(&format!("Name: {}\n", theme.name));
    }
    if !theme.style_guide.is_empty() {
        s.push_str(&format!("Style: {}\n", theme.style_guide));
    }
    if !theme.pov_rules.is_empty() {
        s.push_str(&format!("POV/tense: {}\n", theme.pov_rules));
    }
    if !theme.vocabulary.is_empty() {
        s.push_str(&format!(
            "Favor these words/motifs: {}\n",
            theme.vocabulary.join(", ")
        ));
    }
    if !theme.banned.is_empty() {
        s.push_str(&format!("Never use: {}\n", theme.banned.join(", ")));
    }
    s.push_str(&format!(
        "Length: {} (about {} words).\n",
        theme.desc_length.as_str(),
        theme.desc_length.token_target()
    ));
    for (i, ex) in theme.exemplars.iter().enumerate() {
        s.push_str(&format!("Exemplar {}:\n{}\n", i + 1, ex));
    }
    s
}

/// The `submit_room` tool schema (design 03 §5.2).
#[must_use]
pub fn submit_room_tool() -> ToolDef {
    ToolDef {
        name: TOOL_NAME.to_string(),
        description: "Submit the finished room description as structured data.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["desc"],
            "properties": {
                "desc": { "type": "string" },
                "short_desc": { "type": "string", "description": "one line, for maps/glances" },
                "details": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "keyword": { "type": "string" },
                            "desc": { "type": "string" }
                        }
                    }
                },
                "exits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "direction": { "type": "string" },
                            "name": { "type": "string" },
                            "desc": { "type": "string" },
                            "transit_msg": { "type": "string" }
                        }
                    }
                }
            }
        }),
        strict: true,
    }
}

// ----------------------------------------------------------------------
// The building-brief pass (design 03 §5.3, stage 1)
// ----------------------------------------------------------------------

/// Run the brief pass end-to-end: one strong-model call that walks the
/// building's room graph and emits a shared palette + a one-line concept per
/// room. Same one-shot malformed-output repair as the room path. The brief is
/// itself a reviewable draft — cheap to steer before N room calls.
async fn generate_brief(llm: &LlmService, snap: &BriefSnapshot) -> GenOutcome {
    let spec = StructuredSpec {
        feature: LlmFeature::WorldgenBrief,
        priority: Priority::Interactive,
        actor: Some(snap.builder),
        system: brief_system_prompt(),
        user: brief_user_prompt(snap),
        tool: submit_building_brief_tool(),
        max_tokens: 4096,
        repair_hint: "`palette` is required and must be a non-empty string, and `rooms` must \
                      carry one concept per listed room."
            .to_string(),
    };
    match call_structured::<BuildingBriefPayload, _>(llm, &spec, validate_brief).await {
        Ok(ok) => GenOutcome::Brief {
            entrance: snap.entrance,
            builder: snap.builder,
            brief: ok.value,
            prose: snap.prose.clone(),
            prompt_hash: ok.prompt_hash,
            model: ok.model,
        },
        Err(e) => {
            let (message, retryable) = match &e {
                StructuredError::Malformed { last, .. } => (
                    format!("The model returned a malformed brief twice ({last}). Try @gen/retry."),
                    true,
                ),
                StructuredError::Refused { category, .. } => {
                    let cat = category
                        .as_ref()
                        .map(|c| format!(" ({c})"))
                        .unwrap_or_default();
                    (
                        format!(
                            "The model declined to write a brief for this building{cat}. Try \
                             rephrasing with @gen/retry <guidance>."
                        ),
                        false,
                    )
                }
                StructuredError::Disabled { .. } => ("LLM features are disabled.".to_string(), false),
                // The budget case folds into `retryable`, exactly as before.
                StructuredError::Call { err, .. } => {
                    (format!("Brief generation failed: {err}."), e.is_retryable())
                }
            };
            GenOutcome::BriefErr {
                entrance: snap.entrance,
                builder: snap.builder,
                message,
                retryable,
            }
        }
    }
}

fn validate_brief(brief: BuildingBriefPayload) -> Result<BuildingBriefPayload, String> {
    if brief.palette.trim().is_empty() && brief.concept.trim().is_empty() {
        return Err("brief had neither a `palette` nor a `concept`".to_string());
    }
    Ok(brief)
}

/// The stable, cache-friendly brief system prompt.
#[must_use]
pub fn brief_system_prompt() -> String {
    format!(
        "You are a MUSH worldbuilding assistant preparing a BUILDING BRIEF: a short shared \
         style guide for a whole building before its rooms are written individually.\n\n\
         You are given the building's rooms and how they connect (its interior map). Produce, \
         by calling the `{BRIEF_TOOL_NAME}` tool:\n\
         - a one-line overall concept for the building,\n\
         - a shared palette: the materials, motifs, light, and sound that recur across its \
         rooms (so the per-room descriptions read as one coherent place),\n\
         - one short concept line per room (what makes THAT room distinct), keyed by the \
         room's index as listed.\n\n\
         Rules:\n\
         - Use ONLY the rooms and connections given. Do not invent rooms, exits, or \
         destinations.\n\
         - Honor the theme's style, vocabulary, banned words, and POV.\n\
         - Keep each room concept to a single line; the full descriptions come later.\n\
         - If tool calling is unavailable, reply with ONLY a JSON object of the same shape \
         (`palette` and `rooms` required). No prose outside the JSON."
    )
}

/// Assemble the brief user turn: theme block, the graph walk (rooms + their
/// interior exits), the builder's prose, plus retry guidance / prior brief.
#[must_use]
pub fn brief_user_prompt(snap: &BriefSnapshot) -> String {
    let mut s = String::new();
    s.push_str(&theme_block(&snap.theme));
    s.push('\n');

    s.push_str(&format!(
        "== BUILDING MAP ({} rooms, walked from the entrance) ==\n",
        snap.rooms.len()
    ));
    for (i, r) in snap.rooms.iter().enumerate() {
        let marker = if i == 0 { " (entrance)" } else { "" };
        s.push_str(&format!("Room {}: {}{marker}\n", i + 1, r.name));
        if r.exits.is_empty() {
            s.push_str("  interior exits: (none)\n");
        } else {
            for e in &r.exits {
                s.push_str(&format!("  exit: {e}\n"));
            }
        }
    }
    s.push('\n');

    s.push_str("== BUILDER'S BRIEF ==\n");
    s.push_str(snap.prose.trim());
    s.push('\n');

    if let Some(g) = &snap.guidance
        && !g.trim().is_empty()
    {
        s.push_str("\n== ADDITIONAL GUIDANCE ==\n");
        s.push_str(g.trim());
        s.push('\n');
    }

    if let Some(prev) = &snap.prior_brief {
        s.push_str("\n== AVOID THIS (previously rejected brief) ==\n");
        if !prev.concept.trim().is_empty() {
            s.push_str(&format!("Concept: {}\n", prev.concept.trim()));
        }
        if !prev.palette.trim().is_empty() {
            s.push_str(&format!("Palette: {}\n", prev.palette.trim()));
        }
        s.push_str("Produce a meaningfully different brief.\n");
    }

    s
}

/// The `submit_building_brief` tool schema.
#[must_use]
pub fn submit_building_brief_tool() -> ToolDef {
    ToolDef {
        name: BRIEF_TOOL_NAME.to_string(),
        description: "Submit the building brief: shared palette + a one-line concept per room."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["palette", "rooms"],
            "properties": {
                "concept": { "type": "string", "description": "one-line overall building concept" },
                "palette": { "type": "string", "description": "shared materials/motifs/light/sound" },
                "rooms": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "index": { "type": "integer", "description": "1-based room index as listed" },
                            "name": { "type": "string" },
                            "concept": { "type": "string", "description": "one-line concept for this room" }
                        }
                    }
                }
            }
        }),
        strict: true,
    }
}

/// Map a completed [`BuildingBriefPayload`]'s per-room concepts back onto the
/// batch's ordered rooms: by 1-based `index` first, else by case-insensitive
/// `name`. Returns `(room-fact-index, concept)` pairs the completion applies.
/// Kept here (beside the prompt that assigns the indices) so the mapping and
/// the numbering never drift.
#[must_use]
pub fn map_brief_concepts(rooms: &[BriefRoomFact], brief: &BuildingBriefPayload) -> Vec<String> {
    let mut concepts = vec![String::new(); rooms.len()];
    for rc in &brief.rooms {
        if let Some(slot) = resolve_concept_slot(rooms, rc) {
            concepts[slot] = rc.concept.trim().to_string();
        }
    }
    concepts
}

fn resolve_concept_slot(rooms: &[BriefRoomFact], rc: &RoomConcept) -> Option<usize> {
    // 1-based index into the listed rooms.
    if rc.index >= 1 && (rc.index as usize) <= rooms.len() {
        return Some(rc.index as usize - 1);
    }
    // Fall back to a name match.
    let name = rc.name.trim();
    if !name.is_empty() {
        return rooms.iter().position(|r| r.name.eq_ignore_ascii_case(name));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_llm::CompletionResponse;

    #[test]
    fn all_json_objects_finds_each_in_order() {
        let text = "prefix {\"a\":1} middle {\"b\":{\"c\":2}} suffix";
        let objs = all_json_objects(text);
        assert_eq!(objs, vec!["{\"a\":1}", "{\"b\":{\"c\":2}}"]);
        // extract_json_object keeps its first-object contract.
        assert_eq!(extract_json_object(text), Some("{\"a\":1}"));
    }

    #[test]
    fn all_json_objects_ignores_braces_inside_strings() {
        let text = "{\"desc\":\"a room with a } brace and { another\"}";
        let objs = all_json_objects(text);
        assert_eq!(objs, vec![text]);
    }

    #[test]
    fn all_json_objects_stops_on_unbalanced_tail() {
        // A complete object, then an unterminated one: only the complete one.
        let text = "{\"desc\":\"ok\"} then {\"desc\": \"never closes";
        assert_eq!(all_json_objects(text), vec!["{\"desc\":\"ok\"}"]);
    }

    #[test]
    fn extract_draft_prefers_last_valid_object() {
        // A preamble object then the real answer: the last valid draft wins.
        let resp = CompletionResponse {
            content: vec![mush_llm::ContentBlock::text(
                "think: {\"desc\":\"preamble\"} final: {\"desc\":\"the real room\"}",
            )],
            stop_reason: mush_llm::StopReason::EndTurn,
            usage: Default::default(),
            model: "m".into(),
        };
        let draft: DraftPayload =
            mush_llm::structured::extract(&resp, TOOL_NAME, &validate).expect("a draft");
        assert_eq!(draft.desc, "the real room");
    }
}
