//! [`NpcAction`] — the **typed action allowlist** that is the real security
//! boundary (design 03 §7 rule 1). A fully jailbroken NPC can only ever emit
//! one of these; there is deliberately **no** action that runs a raw command
//! line. Every action re-enters the world task as a typed command and is
//! re-validated there.
//!
//! This module also owns:
//! - the [`ToolDef`]s exposed to the model ([`action_tools`]);
//! - lenient parsing of a completion into a bounded action list, over **both**
//!   the tool-calling path and a JSON-array fallback for models whose
//!   tool-calling is weak ([`parse_actions`]); and
//! - output hygiene ([`sanitize_line`]): length caps + control/ANSI stripping,
//!   so an NPC cannot forge a system line or inject terminal escapes.

use mush_llm::{CompletionResponse, ContentBlock, ToolChoice, ToolDef};
use serde::{Deserialize, Serialize};

/// Max characters of NPC `say`/`pose` text (design 03 §6.4). Longer output is
/// truncated on a word boundary.
pub const SAY_CAP: usize = 600;

/// Max actions applied from one turn (design 03 §6.4: "prefer one or two
/// actions"). Extra actions the model emits are dropped.
pub const MAX_ACTIONS: usize = 4;

/// Max tool round-trips in one agent turn (design 03 §6.4).
pub const MAX_ROUND_TRIPS: usize = 3;

/// The name of the tool that carries a batch of typed actions (the JSON
/// fallback also targets this shape).
pub const SUBMIT_TOOL: &str = "act";

/// One safe, allowlisted NPC action. **Adding a variant is a security
/// decision**: review every new action against "what if the model is fully
/// adversarial?" (design 03 §7 rule 1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum NpcAction {
    /// Speak (rendered through the normal `say` formatter — the NPC's true
    /// name is prefixed; it cannot forge another speaker).
    Say {
        /// What to say.
        text: String,
    },
    /// Emote (rendered through the normal `pose` formatter).
    Pose {
        /// The action text.
        text: String,
    },
    /// Move through a named exit. Re-validated in the world task exactly like
    /// a player's move — a locked / vanished exit simply fails.
    Move {
        /// The exit name/direction.
        exit: String,
    },
    /// Look at a present target. Resolved off-loop from the scene snapshot and
    /// fed back as a tool result; applies no world mutation.
    LookAt {
        /// The target name.
        target: String,
    },
    /// Record a durable memory keyed by `subject` (design 03 §6.5 layer 3).
    Remember {
        /// The memory subject (usually a character name).
        subject: String,
        /// The note to remember.
        note: String,
    },
    /// Invoke a ruleset ability (e.g. `wod_roll`) — validated + executed by
    /// [`Ruleset::invoke_ability`](mush_ruleset::Ruleset::invoke_ability).
    UseAbility {
        /// The ability name.
        name: String,
        /// The ability input (JSON).
        #[serde(default)]
        input: serde_json::Value,
    },
    /// Deliberately do nothing (audited, never shown in game).
    DoNothing {
        /// Why — for the audit trail only.
        #[serde(default)]
        reason: String,
    },
}

impl NpcAction {
    /// Whether this action is resolved off-loop from the scene snapshot and
    /// returns a real tool result that may justify a further round-trip. Only
    /// `LookAt` qualifies: `UseAbility` executes **on the world loop** during
    /// re-entry (it needs the sheet + RNG + outbox), so off-loop it is
    /// fire-and-forget.
    #[must_use]
    pub fn returns_result(&self) -> bool {
        matches!(self, NpcAction::LookAt { .. })
    }

    /// The tool name this action corresponds to.
    #[must_use]
    pub fn tool_name(&self) -> &'static str {
        match self {
            NpcAction::Say { .. } => "say",
            NpcAction::Pose { .. } => "pose",
            NpcAction::Move { .. } => "move",
            NpcAction::LookAt { .. } => "look_at",
            NpcAction::Remember { .. } => "remember",
            NpcAction::UseAbility { .. } => "use_ability",
            NpcAction::DoNothing { .. } => "do_nothing",
        }
    }
}

/// Strip control characters and ANSI escapes, collapse to a single line, and
/// cap length on a word boundary. Applied to every player-visible NPC text so
/// the model cannot inject terminal escapes, forge a multi-line system block,
/// or run away at length (design 03 §6.4 / §7 rule 4).
#[must_use]
pub fn sanitize_line(text: &str, cap: usize) -> String {
    let mut out = String::with_capacity(text.len().min(cap));
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        // Drop ESC-introduced ANSI/OSC sequences wholesale.
        if c == '\u{1b}' {
            // Skip until a plausible terminator (a letter or BEL), bounded by
            // the remaining input.
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() || n == '\u{7}' {
                    break;
                }
            }
            continue;
        }
        // Newlines/tabs → a single space (one-line speech); other control
        // chars are dropped.
        if c == '\n' || c == '\r' || c == '\t' {
            if !out.ends_with(' ') {
                out.push(' ');
            }
            continue;
        }
        if c.is_control() {
            continue;
        }
        // Unicode bidi/format controls are category Cf, not `is_control()`, so
        // they slip past the check above. Drop them: a jailbroken NPC could
        // otherwise emit U+202E (RLO) or zero-width joiners to visually reorder
        // or obscure its say/pose line on a terminal (Trojan-Source-style).
        if is_bidi_or_format(c) {
            continue;
        }
        out.push(c);
    }
    let out = out.trim().to_string();
    truncate_on_boundary(&out, cap)
}

/// Whether `c` is a Unicode bidirectional-override / isolate / zero-width
/// format control that must be stripped from player-visible text.
fn is_bidi_or_format(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}'   // zero-width space/NJ/J, LRM, RLM
        | '\u{202A}'..='\u{202E}' // LRE, RLE, PDF, LRO, RLO (bidi overrides)
        | '\u{2060}'..='\u{2064}' // word joiner + invisible math operators
        | '\u{2066}'..='\u{2069}' // LRI, RLI, FSI, PDI (bidi isolates)
        | '\u{061C}'              // Arabic letter mark
        | '\u{FEFF}'              // zero-width no-break space / BOM
    )
}

/// Truncate `s` to at most `cap` chars, preferring the last word boundary and
/// appending an ellipsis when it actually cut something.
fn truncate_on_boundary(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap {
        return s.to_string();
    }
    // The byte offset of the cap-th char (hard cut point).
    let hard = s.char_indices().nth(cap).map_or(s.len(), |(i, _)| i);
    // Back off to the last space so we don't cut mid-word.
    let cut = s[..hard]
        .rfind(' ')
        .filter(|&p| p > hard / 2)
        .unwrap_or(hard);
    format!("{}…", s[..cut].trim_end())
}

/// The action tools exposed to the model (the allowlist as `ToolDef`s). Ruleset
/// ability tools are appended by the caller from
/// [`AgentCapabilityProfile::tools`](mush_ruleset::AgentCapabilityProfile).
#[must_use]
pub fn action_tools() -> Vec<ToolDef> {
    let str_prop = |desc: &str| serde_json::json!({ "type": "string", "description": desc });
    vec![
        tool(
            "say",
            "Speak aloud to everyone in the room, in character.",
            serde_json::json!({
                "type": "object", "additionalProperties": false,
                "required": ["text"],
                "properties": { "text": str_prop("what you say, in character") }
            }),
        ),
        tool(
            "pose",
            "Perform a visible action/emote (your name is shown before it).",
            serde_json::json!({
                "type": "object", "additionalProperties": false,
                "required": ["text"],
                "properties": { "text": str_prop("the action, e.g. 'wipes down the bar.'") }
            }),
        ),
        tool(
            "move",
            "Leave through one of the room's existing exits.",
            serde_json::json!({
                "type": "object", "additionalProperties": false,
                "required": ["exit"],
                "properties": { "exit": str_prop("the exit name or direction") }
            }),
        ),
        tool(
            "look_at",
            "Look closely at someone or something present; returns what you see.",
            serde_json::json!({
                "type": "object", "additionalProperties": false,
                "required": ["target"],
                "properties": { "target": str_prop("who or what to look at") }
            }),
        ),
        tool(
            "remember",
            "Store a durable memory about a person or topic for later.",
            serde_json::json!({
                "type": "object", "additionalProperties": false,
                "required": ["subject", "note"],
                "properties": {
                    "subject": str_prop("who/what this is about (usually a name)"),
                    "note": str_prop("the fact to remember")
                }
            }),
        ),
        tool(
            "do_nothing",
            "Choose not to act this turn. A valid, common choice.",
            serde_json::json!({
                "type": "object", "additionalProperties": false,
                "properties": { "reason": str_prop("why (not shown to players)") }
            }),
        ),
    ]
}

fn tool(name: &str, description: &str, schema: serde_json::Value) -> ToolDef {
    ToolDef {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: schema,
        strict: false,
    }
}

/// How the model is asked to choose tools for an agent turn. `Auto` lets it
/// pick `do_nothing` freely.
#[must_use]
pub fn tool_choice() -> ToolChoice {
    ToolChoice::Auto
}

/// Parse a completion into a bounded, sanitized list of allowlisted actions.
///
/// Robustness (design 03 §5.4 mirror): tool-calling support varies across
/// providers, so this accepts **both** shapes and never fails hard —
/// unrecognized tool names and malformed inputs are skipped, not executed:
///
/// 1. **Tool calls** — one action per `tool_use` block whose name is on the
///    allowlist (or the `act` batch tool carrying an `actions` array).
/// 2. **JSON fallback** — when there are no usable tool calls, the assistant
///    text is scanned for a JSON array of `{action, …}` objects, or a single
///    such object.
///
/// Returns `(actions, needs_repair)`: `needs_repair` is `true` when the
/// response carried neither a usable tool call nor parseable JSON (the caller
/// issues one repair round-trip, mirroring worldgen).
#[must_use]
pub fn parse_actions(resp: &CompletionResponse) -> (Vec<NpcAction>, bool) {
    // 1. Tool-calling path.
    let mut actions = Vec::new();
    let mut saw_tool = false;
    for block in &resp.content {
        if let ContentBlock::ToolUse { name, input, .. } = block {
            saw_tool = true;
            collect_tool_call(name, input, &mut actions);
        }
    }
    if !actions.is_empty() {
        return (finish(actions), false);
    }

    // 2. JSON-array / object fallback from the text body.
    let text = resp.text();
    if let Some(parsed) = parse_json_actions(&text)
        && !parsed.is_empty()
    {
        return (finish(parsed), false);
    }

    // Nothing usable. If the model at least *tried* a tool but every call was
    // unrecognized, treat as a (silent) no-op rather than a repair loop; only a
    // genuinely shapeless answer asks for repair.
    if saw_tool {
        (Vec::new(), false)
    } else {
        (Vec::new(), true)
    }
}

/// The built-in action tool names (everything else is a ruleset ability tool).
const BUILTIN_TOOLS: &[&str] = &["say", "pose", "move", "look_at", "remember", "do_nothing"];

/// Translate one tool call into zero or more actions. The `act` batch tool
/// carries an `actions` array; a built-in tool name is its typed action; any
/// **other** name is a ruleset ability, mapped to [`NpcAction::UseAbility`]
/// (which [`Ruleset::invoke_ability`](mush_ruleset::Ruleset::invoke_ability)
/// then validates — that validation, not this parser, is the allowlist for
/// abilities). A fully-adversarial model inventing a tool name therefore lands
/// on `UseAbility{name: <invented>}`, which the ruleset rejects; it can never
/// become a command.
fn collect_tool_call(name: &str, input: &serde_json::Value, out: &mut Vec<NpcAction>) {
    if name == SUBMIT_TOOL {
        if let Some(arr) = input.get("actions").and_then(|v| v.as_array()) {
            for v in arr {
                if let Some(a) = action_from_value(v) {
                    out.push(a);
                }
            }
        }
        return;
    }
    if !BUILTIN_TOOLS.contains(&name) {
        // A ruleset ability tool (or an invented name): route to UseAbility.
        out.push(NpcAction::UseAbility {
            name: name.to_string(),
            input: input.clone(),
        });
        return;
    }
    // Re-tag the tool input with its tool name so the shared value parser can
    // build the tagged enum.
    let mut tagged = input.clone();
    if let serde_json::Value::Object(map) = &mut tagged {
        map.insert(
            "action".to_string(),
            serde_json::Value::String(name.to_string()),
        );
    } else {
        return;
    }
    if let Some(a) = action_from_value(&tagged) {
        out.push(a);
    }
}

/// Parse a JSON actions payload from free text: a top-level array of action
/// objects, or a single action object, tolerating prose/code fences around it.
fn parse_json_actions(text: &str) -> Option<Vec<NpcAction>> {
    // Try a balanced top-level array first.
    if let Some(arr) = extract_balanced(text, b'[', b']')
        && let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(arr)
    {
        let actions: Vec<NpcAction> = items.iter().filter_map(action_from_value).collect();
        return Some(actions);
    }
    // Else a single object.
    for obj in all_json_objects(text) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(obj)
            && let Some(a) = action_from_value(&v)
        {
            return Some(vec![a]);
        }
    }
    None
}

/// Build one action from a JSON value, accepting both the internally-tagged
/// shape (`{"action":"say","text":…}`) and a couple of lenient synonyms.
fn action_from_value(v: &serde_json::Value) -> Option<NpcAction> {
    let obj = v.as_object()?;
    // Accept "action" or "type" or "tool" as the discriminator.
    let tag = obj
        .get("action")
        .or_else(|| obj.get("type"))
        .or_else(|| obj.get("tool"))
        .and_then(|t| t.as_str())?
        .to_ascii_lowercase();
    let s = |k: &str| {
        obj.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    let action = match tag.as_str() {
        "say" => NpcAction::Say { text: s("text") },
        "pose" | "emote" => NpcAction::Pose { text: s("text") },
        "move" | "go" => NpcAction::Move {
            exit: first_nonempty(obj, &["exit", "direction", "target"]),
        },
        "look_at" | "look" | "examine" => NpcAction::LookAt {
            target: first_nonempty(obj, &["target", "name", "what"]),
        },
        "remember" => NpcAction::Remember {
            subject: s("subject"),
            note: s("note"),
        },
        "use_ability" | "ability" | "roll" => NpcAction::UseAbility {
            name: first_nonempty(obj, &["name", "ability"]),
            input: obj.get("input").cloned().unwrap_or_else(|| {
                // Fold any non-control keys into the input so a flat call like
                // {"action":"use_ability","name":"wod_roll","pool":"dex+brawl"}
                // still reaches the ruleset.
                let mut m = serde_json::Map::new();
                for (k, val) in obj {
                    if !matches!(k.as_str(), "action" | "type" | "tool" | "name" | "ability") {
                        m.insert(k.clone(), val.clone());
                    }
                }
                serde_json::Value::Object(m)
            }),
        },
        "do_nothing" | "nothing" | "wait" | "pass" => NpcAction::DoNothing {
            reason: s("reason"),
        },
        _ => return None,
    };
    Some(action)
}

fn first_nonempty(obj: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> String {
    for k in keys {
        if let Some(s) = obj.get(*k).and_then(|v| v.as_str())
            && !s.trim().is_empty()
        {
            return s.to_string();
        }
    }
    String::new()
}

/// The actions carried by one tool-use block (the turn loop uses this to keep
/// the block's tool_use id for the tool_result). A built-in tool yields its
/// typed action; any other name yields a single [`NpcAction::UseAbility`].
#[must_use]
pub fn actions_from_tool(name: &str, input: &serde_json::Value) -> Vec<NpcAction> {
    let mut out = Vec::new();
    collect_tool_call(name, input, &mut out);
    out
}

/// Sanitize one action's text fields; return `None` if it collapses to empty.
#[must_use]
pub fn clean_action(a: NpcAction) -> Option<NpcAction> {
    let cleaned = match a {
        NpcAction::Say { text } => {
            let t = sanitize_line(&text, SAY_CAP);
            if t.is_empty() {
                return None;
            }
            NpcAction::Say { text: t }
        }
        NpcAction::Pose { text } => {
            let t = sanitize_line(&text, SAY_CAP);
            if t.is_empty() {
                return None;
            }
            NpcAction::Pose { text: t }
        }
        NpcAction::Move { exit } => {
            let e = sanitize_line(&exit, 64);
            if e.is_empty() {
                return None;
            }
            NpcAction::Move { exit: e }
        }
        NpcAction::LookAt { target } => {
            let t = sanitize_line(&target, 64);
            if t.is_empty() {
                return None;
            }
            NpcAction::LookAt { target: t }
        }
        NpcAction::Remember { subject, note } => {
            let subj = sanitize_line(&subject, 64);
            let note = sanitize_line(&note, SAY_CAP);
            if subj.is_empty() || note.is_empty() {
                return None;
            }
            NpcAction::Remember {
                subject: subj,
                note,
            }
        }
        NpcAction::UseAbility { name, input } => {
            let n = sanitize_line(&name, 64);
            if n.is_empty() {
                return None;
            }
            NpcAction::UseAbility { name: n, input }
        }
        NpcAction::DoNothing { reason } => NpcAction::DoNothing {
            reason: sanitize_line(&reason, 200),
        },
    };
    Some(cleaned)
}

/// Bound the action list, drop empties, and sanitize text fields.
fn finish(actions: Vec<NpcAction>) -> Vec<NpcAction> {
    let mut out = Vec::new();
    for a in actions {
        if out.len() >= MAX_ACTIONS {
            break;
        }
        if let Some(cleaned) = clean_action(a) {
            out.push(cleaned);
        }
    }
    out
}

// -- small balanced-scan helpers (mirror worldgen's robustness) --------------

/// The first balanced `open`..`close` span in `text` (respecting strings), or
/// `None`.
fn extract_balanced(text: &str, open: u8, close: u8) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|&b| b == open)?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
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
            b if b == open => depth += 1,
            b if b == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=j]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Every balanced top-level `{…}` object in `text`, in order.
fn all_json_objects(text: &str) -> Vec<&str> {
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
            None => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_llm::StopReason;

    fn resp_with(content: Vec<ContentBlock>) -> CompletionResponse {
        CompletionResponse {
            content,
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
            model: "mock".into(),
        }
    }

    #[test]
    fn sanitize_strips_ansi_and_control_and_caps() {
        let dirty = "hel\u{1b}[31mlo\u{7}\nworld\u{0}!";
        let clean = sanitize_line(dirty, SAY_CAP);
        assert_eq!(clean, "hello world!");
        // Length cap on a word boundary with an ellipsis.
        let long = "word ".repeat(300);
        let capped = sanitize_line(&long, 20);
        assert!(capped.chars().count() <= 21, "{capped:?}");
        assert!(capped.ends_with('…'));
    }

    #[test]
    fn sanitize_strips_unicode_bidi_and_zero_width() {
        // U+202E RLO, U+2066 LRI, U+200B ZWSP, U+FEFF BOM must not survive into
        // player-visible text (Trojan-Source-style reordering / obfuscation).
        let dirty = "sa\u{202e}fe\u{2066}\u{200b}r\u{feff}!";
        let clean = sanitize_line(dirty, SAY_CAP);
        assert_eq!(clean, "safer!");
        assert!(!clean.chars().any(|c| c == '\u{202e}' || c == '\u{200b}'));
    }

    #[test]
    fn tool_calls_map_to_allowlisted_actions() {
        let resp = resp_with(vec![
            ContentBlock::ToolUse {
                id: "1".into(),
                name: "say".into(),
                input: serde_json::json!({ "text": "Welcome, friend." }),
            },
            ContentBlock::ToolUse {
                id: "2".into(),
                name: "pose".into(),
                input: serde_json::json!({ "text": "polishes a glass." }),
            },
        ]);
        let (actions, repair) = parse_actions(&resp);
        assert!(!repair);
        assert_eq!(
            actions,
            vec![
                NpcAction::Say {
                    text: "Welcome, friend.".into()
                },
                NpcAction::Pose {
                    text: "polishes a glass.".into()
                },
            ]
        );
    }

    #[test]
    fn json_fallback_array_parses() {
        let resp = resp_with(vec![ContentBlock::text(
            "Sure, here you go:\n[{\"action\":\"say\",\"text\":\"Hi\"},\
             {\"action\":\"do_nothing\"}]\nthanks",
        )]);
        let (actions, repair) = parse_actions(&resp);
        assert!(!repair);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0], NpcAction::Say { text: "Hi".into() });
        assert!(matches!(actions[1], NpcAction::DoNothing { .. }));
    }

    #[test]
    fn json_fallback_single_object_and_synonyms() {
        let resp = resp_with(vec![ContentBlock::text(
            "{\"type\":\"emote\",\"text\":\"nods slowly.\"}",
        )]);
        let (actions, _) = parse_actions(&resp);
        assert_eq!(
            actions,
            vec![NpcAction::Pose {
                text: "nods slowly.".into()
            }]
        );
    }

    #[test]
    fn shapeless_answer_requests_repair() {
        let resp = resp_with(vec![ContentBlock::text("I'm not sure what to do here.")]);
        let (actions, repair) = parse_actions(&resp);
        assert!(actions.is_empty());
        assert!(repair);
    }

    #[test]
    fn invented_tool_becomes_inert_use_ability_never_a_command() {
        // A jailbreak that invents a dangerous tool name cannot escape the
        // allowlist: it maps to UseAbility{name: "run_command"}, which the
        // ruleset's invoke_ability rejects. There is no path to a command line.
        let resp = resp_with(vec![ContentBlock::ToolUse {
            id: "1".into(),
            name: "run_command".into(),
            input: serde_json::json!({ "line": "@destroy #5" }),
        }]);
        let (actions, repair) = parse_actions(&resp);
        assert!(!repair);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], NpcAction::UseAbility { name, .. } if name == "run_command"));
        // Crucially, it did NOT become a Say/Pose/Move carrying the command.
        assert!(!matches!(
            &actions[0],
            NpcAction::Say { .. } | NpcAction::Move { .. }
        ));
    }

    #[test]
    fn action_list_is_bounded() {
        let many: Vec<serde_json::Value> = (0..10)
            .map(|i| serde_json::json!({ "action": "say", "text": format!("line {i}") }))
            .collect();
        let resp = resp_with(vec![ContentBlock::text(
            serde_json::to_string(&many).unwrap(),
        )]);
        let (actions, _) = parse_actions(&resp);
        assert_eq!(actions.len(), MAX_ACTIONS);
    }

    #[test]
    fn ability_tool_call_maps_to_use_ability() {
        // An exposed ruleset ability tool (wod_roll) arrives under its own
        // name and becomes UseAbility with the input carried verbatim.
        let resp = resp_with(vec![ContentBlock::ToolUse {
            id: "1".into(),
            name: "wod_roll".into(),
            input: serde_json::json!({ "pool": "dexterity+brawl", "difficulty": 6 }),
        }]);
        let (actions, _) = parse_actions(&resp);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            NpcAction::UseAbility { name, input } => {
                assert_eq!(name, "wod_roll");
                assert_eq!(input.get("pool").unwrap(), "dexterity+brawl");
                assert_eq!(input.get("difficulty").unwrap(), 6);
            }
            other => panic!("expected use_ability, got {other:?}"),
        }
    }

    #[test]
    fn use_ability_folds_flat_keys_via_json_path() {
        let resp = resp_with(vec![ContentBlock::text(
            "{\"action\":\"use_ability\",\"name\":\"wod_roll\",\"pool\":\"dexterity+brawl\",\"difficulty\":6}",
        )]);
        let (actions, _) = parse_actions(&resp);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            NpcAction::UseAbility { name, input } => {
                assert_eq!(name, "wod_roll");
                assert_eq!(input.get("pool").unwrap(), "dexterity+brawl");
                assert_eq!(input.get("difficulty").unwrap(), 6);
            }
            other => panic!("expected use_ability, got {other:?}"),
        }
    }
}
