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
    CallCtx, ChatMessage, CompletionRequest, CompletionResponse, LlmError, LlmFeature, LlmService,
    Priority, Sampling, ToolChoice, ToolDef,
};
use sha2::{Digest, Sha256};

use crate::data::{
    BriefRoomFact, BriefSnapshot, BuildingBriefContext, BuildingBriefPayload, DraftPayload,
    GenOutcome, GenSnapshot, RoomConcept, WorldgenReq,
};
use crate::theme::Theme;

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
    let caps = llm.caps_for(LlmFeature::WorldgenRoom).unwrap_or_default();
    let system = system_prompt();
    let user = user_prompt_inner(snap, brief);
    let prompt_hash = sha256_hex(&format!("{system}\n\n{user}"));
    // A batch room is bulk work (background semaphore); a single-room `@gen` is
    // interactive (the builder is waiting on it).
    let priority = if building.is_some() {
        Priority::Background
    } else {
        Priority::Interactive
    };

    let force_tool = caps.tool_calling && caps.forced_tool_choice;
    let mut req = CompletionRequest {
        model: String::new(),
        system: Some(system),
        messages: vec![ChatMessage::user(user)],
        tools: vec![submit_room_tool()],
        tool_choice: if force_tool {
            ToolChoice::Tool(TOOL_NAME.to_string())
        } else {
            ToolChoice::Auto
        },
        // The route overrides max_tokens / reasoning; these are placeholders.
        max_tokens: 2048,
        sampling: Sampling::default(),
        stop_sequences: Vec::new(),
        reasoning: None,
    };
    let ctx = CallCtx::new(LlmFeature::WorldgenRoom, priority).with_actor(snap.builder);

    let fail = |message: String, retryable: bool, budget: bool| GenOutcome::Err {
        room: snap.room,
        builder: snap.builder,
        message,
        retryable,
        building,
        budget,
    };

    // At most two attempts: the original + one repair.
    let mut last_parse_err = String::new();
    for attempt in 0..2 {
        match llm.call(ctx, req.clone()).await {
            Ok(resp) => match extract_draft(&resp) {
                Ok(draft) => {
                    return GenOutcome::Ok {
                        room: snap.room,
                        builder: snap.builder,
                        draft,
                        prose: snap.prose.clone(),
                        prompt_hash,
                        model: resp.model.clone(),
                        building,
                    };
                }
                Err(parse_err) => {
                    last_parse_err = parse_err;
                    if attempt == 0 {
                        append_repair(&mut req, &resp, &last_parse_err);
                        continue;
                    }
                }
            },
            // The adapter itself flagged unparseable tool JSON — same repair.
            Err(LlmError::MalformedOutput(m)) => {
                last_parse_err = m;
                if attempt == 0 {
                    append_repair_text(&mut req, &last_parse_err);
                    continue;
                }
            }
            Err(LlmError::Refused { category }) => {
                let cat = category.map(|c| format!(" ({c})")).unwrap_or_default();
                return fail(
                    format!(
                        "The model declined to describe this room{cat}. Try rephrasing \
                         with @gen/retry <guidance>."
                    ),
                    false,
                    false,
                );
            }
            // A kill switch (global `[llm].enabled = false`, per-feature, or the
            // staff `@llm/disable`) is engaged: report it plainly rather than as
            // a generic failure. A keyless clone hits this on every `@gen`.
            Err(LlmError::Disabled) => {
                return fail("LLM features are disabled.".to_string(), false, false);
            }
            Err(e) => {
                let budget = matches!(e, LlmError::BudgetExceeded { .. });
                return fail(
                    format!("Generation failed: {e}."),
                    e.is_retryable() || budget,
                    budget,
                );
            }
        }
    }

    fail(
        format!("The model returned malformed output twice ({last_parse_err}). Try @gen/retry."),
        true,
        false,
    )
}

/// Append a repair turn: echo the model's raw answer, then instruct it to
/// call the tool again with valid input.
fn append_repair(req: &mut CompletionRequest, resp: &CompletionResponse, err: &str) {
    let echoed = if let Some((_, input)) = resp.first_tool_use() {
        input.to_string()
    } else {
        resp.text()
    };
    req.messages.push(ChatMessage::assistant(echoed));
    append_repair_text(req, err);
}

/// Append only the correction instruction (used when the adapter rejected
/// the tool JSON before we saw a response object).
fn append_repair_text(req: &mut CompletionRequest, err: &str) {
    req.messages.push(ChatMessage::user(format!(
        "That was not valid: {err}. Call the {TOOL_NAME} tool again with input that \
         matches the schema exactly. `desc` is required and must be a non-empty string. \
         If you cannot call a tool, reply with ONLY the JSON object, nothing else."
    )));
}

/// Extract the draft from a response: prefer the forced tool call; fall back
/// to a JSON object embedded in the text. Validates that `desc` is non-empty.
fn extract_draft(resp: &CompletionResponse) -> Result<DraftPayload, String> {
    // 1. Forced/native tool call.
    if let Some((name, input)) = resp.first_tool_use() {
        if name != TOOL_NAME {
            return Err(format!("unexpected tool {name:?}; expected {TOOL_NAME:?}"));
        }
        let draft: DraftPayload = serde_json::from_value(input.clone())
            .map_err(|e| format!("tool input did not match the schema: {e}"))?;
        return validate(draft);
    }
    // 2. JSON object(s) in the text body (the caps-without-forced-tool
    //    fallback path). A reasoning model may emit a throwaway JSON-looking
    //    object in a visible preamble before its real answer, so scan every
    //    balanced object and take the LAST one that parses as a valid draft
    //    rather than the first `{`.
    let text = resp.text();
    let blocks = all_json_objects(&text);
    if blocks.is_empty() {
        return Err("response contained no tool call and no JSON object".to_string());
    }
    let mut last_err = String::new();
    for block in blocks.iter().rev() {
        match serde_json::from_str::<DraftPayload>(block) {
            Ok(draft) => match validate(draft) {
                Ok(d) => return Ok(d),
                Err(e) => last_err = e,
            },
            Err(e) => last_err = format!("embedded JSON did not parse: {e}"),
        }
    }
    Err(last_err)
}

fn validate(draft: DraftPayload) -> Result<DraftPayload, String> {
    if draft.desc.trim().is_empty() {
        return Err("`desc` was empty".to_string());
    }
    Ok(draft)
}

/// Find the first balanced top-level JSON object in `text` (tolerating
/// prose or code fences around it). Respects strings and escapes so a `}`
/// inside a string value does not close the object early.
#[must_use]
pub fn extract_json_object(text: &str) -> Option<&str> {
    all_json_objects(text).into_iter().next()
}

/// Every balanced top-level JSON object in `text`, in order of appearance
/// (tolerating prose or code fences between them). Respects strings and
/// escapes so a `}` inside a string value does not close an object early, and
/// so a brace inside one object's string is not mistaken for a new top-level
/// object.
#[must_use]
pub fn all_json_objects(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let start = i;
        let mut depth = 0i32;
        let mut in_str = false;
        let mut escaped = false;
        let mut end = None;
        for (j, &c) in bytes.iter().enumerate().skip(start) {
            if in_str {
                if escaped {
                    escaped = false;
                } else if c == b'\\' {
                    escaped = true;
                } else if c == b'"' {
                    in_str = false;
                }
                continue;
            }
            match c {
                b'"' => in_str = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(j);
                        break;
                    }
                }
                _ => {}
            }
        }
        match end {
            Some(j) => {
                out.push(&text[start..=j]);
                i = j + 1;
            }
            // Unbalanced from here on: no further top-level object can close.
            None => break,
        }
    }
    out
}

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

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

// ----------------------------------------------------------------------
// The building-brief pass (design 03 §5.3, stage 1)
// ----------------------------------------------------------------------

/// Run the brief pass end-to-end: one strong-model call that walks the
/// building's room graph and emits a shared palette + a one-line concept per
/// room. Same one-shot malformed-output repair as the room path. The brief is
/// itself a reviewable draft — cheap to steer before N room calls.
async fn generate_brief(llm: &LlmService, snap: &BriefSnapshot) -> GenOutcome {
    let caps = llm.caps_for(LlmFeature::WorldgenBrief).unwrap_or_default();
    let system = brief_system_prompt();
    let user = brief_user_prompt(snap);
    let prompt_hash = sha256_hex(&format!("{system}\n\n{user}"));

    let force_tool = caps.tool_calling && caps.forced_tool_choice;
    let mut req = CompletionRequest {
        model: String::new(),
        system: Some(system),
        messages: vec![ChatMessage::user(user)],
        tools: vec![submit_building_brief_tool()],
        tool_choice: if force_tool {
            ToolChoice::Tool(BRIEF_TOOL_NAME.to_string())
        } else {
            ToolChoice::Auto
        },
        max_tokens: 4096,
        sampling: Sampling::default(),
        stop_sequences: Vec::new(),
        reasoning: None,
    };
    let ctx =
        CallCtx::new(LlmFeature::WorldgenBrief, Priority::Interactive).with_actor(snap.builder);

    let fail = |message: String, retryable: bool| GenOutcome::BriefErr {
        entrance: snap.entrance,
        builder: snap.builder,
        message,
        retryable,
    };

    let mut last_parse_err = String::new();
    for attempt in 0..2 {
        match llm.call(ctx, req.clone()).await {
            Ok(resp) => match extract_brief(&resp) {
                Ok(brief) => {
                    return GenOutcome::Brief {
                        entrance: snap.entrance,
                        builder: snap.builder,
                        brief,
                        prose: snap.prose.clone(),
                        prompt_hash,
                        model: resp.model.clone(),
                    };
                }
                Err(parse_err) => {
                    last_parse_err = parse_err;
                    if attempt == 0 {
                        append_brief_repair(&mut req, &resp, &last_parse_err);
                        continue;
                    }
                }
            },
            Err(LlmError::MalformedOutput(m)) => {
                last_parse_err = m;
                if attempt == 0 {
                    append_repair_text_brief(&mut req, &last_parse_err);
                    continue;
                }
            }
            Err(LlmError::Refused { category }) => {
                let cat = category.map(|c| format!(" ({c})")).unwrap_or_default();
                return fail(
                    format!(
                        "The model declined to write a brief for this building{cat}. Try \
                         rephrasing with @gen/retry <guidance>."
                    ),
                    false,
                );
            }
            Err(LlmError::Disabled) => {
                return fail("LLM features are disabled.".to_string(), false);
            }
            Err(e) => {
                let budget = matches!(e, LlmError::BudgetExceeded { .. });
                return fail(
                    format!("Brief generation failed: {e}."),
                    e.is_retryable() || budget,
                );
            }
        }
    }

    fail(
        format!("The model returned a malformed brief twice ({last_parse_err}). Try @gen/retry."),
        true,
    )
}

fn append_brief_repair(req: &mut CompletionRequest, resp: &CompletionResponse, err: &str) {
    let echoed = if let Some((_, input)) = resp.first_tool_use() {
        input.to_string()
    } else {
        resp.text()
    };
    req.messages.push(ChatMessage::assistant(echoed));
    append_repair_text_brief(req, err);
}

fn append_repair_text_brief(req: &mut CompletionRequest, err: &str) {
    req.messages.push(ChatMessage::user(format!(
        "That was not valid: {err}. Call the {BRIEF_TOOL_NAME} tool again with input that \
         matches the schema exactly. `palette` is required and must be a non-empty string, and \
         `rooms` must carry one concept per listed room. If you cannot call a tool, reply with \
         ONLY the JSON object, nothing else."
    )));
}

/// Extract the brief from a response: prefer the forced tool call; fall back
/// to the last balanced JSON object in the text.
fn extract_brief(resp: &CompletionResponse) -> Result<BuildingBriefPayload, String> {
    if let Some((name, input)) = resp.first_tool_use() {
        if name != BRIEF_TOOL_NAME {
            return Err(format!(
                "unexpected tool {name:?}; expected {BRIEF_TOOL_NAME:?}"
            ));
        }
        let brief: BuildingBriefPayload = serde_json::from_value(input.clone())
            .map_err(|e| format!("tool input did not match the schema: {e}"))?;
        return validate_brief(brief);
    }
    let text = resp.text();
    let blocks = all_json_objects(&text);
    if blocks.is_empty() {
        return Err("response contained no tool call and no JSON object".to_string());
    }
    let mut last_err = String::new();
    for block in blocks.iter().rev() {
        match serde_json::from_str::<BuildingBriefPayload>(block) {
            Ok(brief) => match validate_brief(brief) {
                Ok(b) => return Ok(b),
                Err(e) => last_err = e,
            },
            Err(e) => last_err = format!("embedded JSON did not parse: {e}"),
        }
    }
    Err(last_err)
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
        let draft = extract_draft(&resp).expect("a draft");
        assert_eq!(draft.desc, "the real room");
    }
}
