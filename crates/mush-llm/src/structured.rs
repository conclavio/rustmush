//! One structured call: a forced tool with a typed, validated result, a
//! JSON-in-text fallback for providers that cannot force a tool, and exactly one
//! repair round trip when the first answer does not fit the schema.
//!
//! Hoisted from `mush-worldgen`'s room and brief generation (16b cut R-a, R1) so
//! every structured request in the tree runs the same protocol: the room and
//! brief passes call this with their own tool, prompts, validator and repair
//! hint, and keep their own outcome types and user-facing messages.
//! [`StructuredError`] carries what a caller needs to phrase those messages and
//! to classify the failure (`is_retryable`, `is_budget`), and every outcome
//! carries the prompt hash so audit rows and drafts can be correlated.
//!
//! What this is not: the bounded agentic tool loop ([`crate::agent`]) with its
//! own repair flag; that path is `run_plan`'s and stays separate.

use serde::de::DeserializeOwned;

use crate::provider::LlmError;
use crate::service::{CallCtx, LlmFeature, LlmService, Priority};
use crate::types::{
    ChatMessage, CompletionRequest, CompletionResponse, Sampling, ToolChoice, ToolDef, Usage,
};

/// Everything one structured call needs. The tool's `name` is the forced tool
/// name; `repair_hint` is the schema sentence spliced into the repair turn.
#[derive(Clone, Debug)]
pub struct StructuredSpec {
    pub feature: LlmFeature,
    pub priority: Priority,
    pub actor: Option<mush_core::ObjId>,
    pub system: String,
    pub user: String,
    pub tool: ToolDef,
    /// A placeholder the route overrides, exactly as the callers set it before.
    pub max_tokens: u32,
    pub repair_hint: String,
}

/// A structured call that produced a valid `T`.
#[derive(Clone, Debug)]
pub struct StructuredOk<T> {
    pub value: T,
    pub model: String,
    pub prompt_hash: String,
    pub usage: Usage,
}

/// Why a structured call produced nothing usable. Every variant carries the
/// prompt hash so the caller can correlate it with the audit row.
#[derive(Debug)]
pub enum StructuredError {
    /// Both attempts produced unusable output; `last` is the final parse error.
    Malformed { last: String, prompt_hash: String },
    /// The provider refused, optionally with its category.
    Refused {
        category: Option<String>,
        prompt_hash: String,
    },
    /// A kill switch (global, per-feature or staff) is engaged.
    Disabled { prompt_hash: String },
    /// Any other provider error, unchanged.
    Call { err: LlmError, prompt_hash: String },
}

impl StructuredError {
    /// The call failed on the budget, which callers surface separately from a
    /// transient failure.
    #[must_use]
    pub fn is_budget(&self) -> bool {
        matches!(
            self,
            StructuredError::Call {
                err: LlmError::BudgetExceeded { .. },
                ..
            }
        )
    }

    /// Whether trying again could succeed: a double-malformed answer can, a
    /// refusal or a kill switch cannot, and a provider error follows its own
    /// `is_retryable` plus the budget case (the callers' classification).
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            StructuredError::Malformed { .. } => true,
            StructuredError::Refused { .. } | StructuredError::Disabled { .. } => false,
            StructuredError::Call { err, .. } => err.is_retryable() || self.is_budget(),
        }
    }

    #[must_use]
    pub fn prompt_hash(&self) -> &str {
        match self {
            StructuredError::Malformed { prompt_hash, .. }
            | StructuredError::Refused { prompt_hash, .. }
            | StructuredError::Disabled { prompt_hash }
            | StructuredError::Call { prompt_hash, .. } => prompt_hash,
        }
    }
}

/// Run one structured call: the original attempt plus at most one repair.
///
/// The forced tool is used when the route's provider can force one; otherwise
/// the tool is offered and the answer's text is scanned for the **last**
/// balanced JSON object that parses and validates (a reasoning model may emit
/// a throwaway object in a visible preamble before its real answer).
///
/// # Errors
///
/// [`StructuredError`] for a double-malformed answer, a refusal, a kill switch
/// or any other provider error, each with the prompt hash.
pub async fn call_structured<T, F>(
    llm: &LlmService,
    spec: &StructuredSpec,
    validate: F,
) -> Result<StructuredOk<T>, StructuredError>
where
    T: DeserializeOwned,
    F: Fn(T) -> Result<T, String>,
{
    let caps = llm.caps_for(spec.feature).unwrap_or_default();
    let prompt_hash = crate::audit::prompt_sha256(&format!("{}\n\n{}", spec.system, spec.user));

    let force_tool = caps.tool_calling && caps.forced_tool_choice;
    let mut req = CompletionRequest {
        model: String::new(),
        system: Some(spec.system.clone()),
        messages: vec![ChatMessage::user(spec.user.clone())],
        tools: vec![spec.tool.clone()],
        tool_choice: if force_tool {
            ToolChoice::Tool(spec.tool.name.clone())
        } else {
            ToolChoice::Auto
        },
        // The route overrides max_tokens / reasoning; these are placeholders.
        max_tokens: spec.max_tokens,
        sampling: Sampling::default(),
        stop_sequences: Vec::new(),
        reasoning: None,
    };
    let mut ctx = CallCtx::new(spec.feature, spec.priority);
    if let Some(actor) = spec.actor {
        ctx = ctx.with_actor(actor);
    }

    // At most two attempts: the original + one repair.
    let mut last_parse_err = String::new();
    for attempt in 0..2 {
        match llm.call(ctx, req.clone()).await {
            Ok(resp) => match extract::<T, F>(&resp, &spec.tool.name, &validate) {
                Ok(value) => {
                    return Ok(StructuredOk {
                        value,
                        model: resp.model.clone(),
                        prompt_hash,
                        usage: resp.usage,
                    });
                }
                Err(parse_err) => {
                    last_parse_err = parse_err;
                    if attempt == 0 {
                        append_repair(&mut req, &resp, &last_parse_err, spec);
                        continue;
                    }
                }
            },
            // The adapter itself flagged unparseable tool JSON — same repair.
            Err(LlmError::MalformedOutput(m)) => {
                last_parse_err = m;
                if attempt == 0 {
                    append_repair_text(&mut req, &last_parse_err, spec);
                    continue;
                }
            }
            Err(LlmError::Refused { category }) => {
                return Err(StructuredError::Refused {
                    category,
                    prompt_hash,
                });
            }
            Err(LlmError::Disabled) => {
                return Err(StructuredError::Disabled { prompt_hash });
            }
            Err(err) => {
                return Err(StructuredError::Call { err, prompt_hash });
            }
        }
    }

    Err(StructuredError::Malformed {
        last: last_parse_err,
        prompt_hash,
    })
}

/// Extract the typed value from a response: prefer the forced tool call; fall
/// back to the last balanced JSON object in the text that parses and validates.
///
/// # Errors
///
/// The parse or validation error of the last candidate, or a description of
/// why there was no candidate at all.
pub fn extract<T, F>(resp: &CompletionResponse, tool_name: &str, validate: &F) -> Result<T, String>
where
    T: DeserializeOwned,
    F: Fn(T) -> Result<T, String>,
{
    // 1. Forced/native tool call.
    if let Some((name, input)) = resp.first_tool_use() {
        if name != tool_name {
            return Err(format!("unexpected tool {name:?}; expected {tool_name:?}"));
        }
        let value: T = serde_json::from_value(input.clone())
            .map_err(|e| format!("tool input did not match the schema: {e}"))?;
        return validate(value);
    }
    // 2. JSON object(s) in the text body (the caps-without-forced-tool
    //    fallback path). Scan every balanced object and take the LAST one that
    //    parses and validates rather than the first `{`.
    let text = resp.text();
    let blocks = all_json_objects(&text);
    if blocks.is_empty() {
        return Err("response contained no tool call and no JSON object".to_string());
    }
    let mut last_err = String::new();
    for block in blocks.iter().rev() {
        match serde_json::from_str::<T>(block) {
            Ok(value) => match validate(value) {
                Ok(v) => return Ok(v),
                Err(e) => last_err = e,
            },
            Err(e) => last_err = format!("embedded JSON did not parse: {e}"),
        }
    }
    Err(last_err)
}

/// Append a repair turn: echo the model's raw answer, then instruct it to
/// call the tool again with valid input.
fn append_repair(
    req: &mut CompletionRequest,
    resp: &CompletionResponse,
    err: &str,
    spec: &StructuredSpec,
) {
    let echoed = if let Some((_, input)) = resp.first_tool_use() {
        input.to_string()
    } else {
        resp.text()
    };
    req.messages.push(ChatMessage::assistant(echoed));
    append_repair_text(req, err, spec);
}

/// Append only the correction instruction (used when the adapter rejected
/// the tool JSON before we saw a response object).
fn append_repair_text(req: &mut CompletionRequest, err: &str, spec: &StructuredSpec) {
    req.messages.push(ChatMessage::user(format!(
        "That was not valid: {err}. Call the {tool} tool again with input that matches the \
         schema exactly. {hint} If you cannot call a tool, reply with ONLY the JSON object, \
         nothing else.",
        tool = spec.tool.name,
        hint = spec.repair_hint,
    )));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentBlock, StopReason};

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Draft {
        desc: String,
    }

    fn validate(d: Draft) -> Result<Draft, String> {
        if d.desc.trim().is_empty() {
            return Err("`desc` was empty".to_string());
        }
        Ok(d)
    }

    fn text_response(text: &str) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            model: "test/mock".into(),
        }
    }

    #[test]
    fn all_json_objects_finds_each_in_order() {
        let text = "pre {\"a\":1} mid {\"b\":{\"c\":2}} post";
        assert_eq!(
            all_json_objects(text),
            vec!["{\"a\":1}", "{\"b\":{\"c\":2}}"]
        );
        assert_eq!(extract_json_object(text), Some("{\"a\":1}"));
    }

    #[test]
    fn all_json_objects_ignores_braces_inside_strings() {
        let text = r#"{"desc":"a } inside"} tail"#;
        assert_eq!(all_json_objects(text), vec![r#"{"desc":"a } inside"}"#]);
    }

    #[test]
    fn all_json_objects_stops_on_unbalanced_tail() {
        let text = "{\"a\":1} {\"b\":";
        assert_eq!(all_json_objects(text), vec!["{\"a\":1}"]);
    }

    #[test]
    fn extract_prefers_the_last_valid_object() {
        let resp = text_response(
            "Thinking: {\"desc\":\"\"} and {\"nope\":true} then {\"desc\":\"the real room\"}",
        );
        let draft: Draft = extract(&resp, "submit_room", &validate).expect("a draft");
        assert_eq!(draft.desc, "the real room");
    }

    #[test]
    fn extract_refuses_the_wrong_tool_and_reports_the_last_error() {
        let resp = CompletionResponse {
            content: vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "other".into(),
                input: serde_json::json!({"desc": "x"}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage::default(),
            model: "test/mock".into(),
        };
        let err = extract::<Draft, _>(&resp, "submit_room", &validate).unwrap_err();
        assert!(err.contains("unexpected tool"), "{err}");

        let resp = text_response("no json here");
        let err = extract::<Draft, _>(&resp, "submit_room", &validate).unwrap_err();
        assert_eq!(err, "response contained no tool call and no JSON object");
    }

    #[test]
    fn errors_classify_like_the_callers_did() {
        let hash = "h".to_string();
        let malformed = StructuredError::Malformed {
            last: "x".into(),
            prompt_hash: hash.clone(),
        };
        assert!(malformed.is_retryable() && !malformed.is_budget());
        let refused = StructuredError::Refused {
            category: None,
            prompt_hash: hash.clone(),
        };
        assert!(!refused.is_retryable());
        let disabled = StructuredError::Disabled {
            prompt_hash: hash.clone(),
        };
        assert!(!disabled.is_retryable());
        let budget = StructuredError::Call {
            err: LlmError::BudgetExceeded {
                scope: crate::provider::BudgetScope::Global,
            },
            prompt_hash: hash.clone(),
        };
        assert!(budget.is_retryable() && budget.is_budget());
        let timeout = StructuredError::Call {
            err: LlmError::Timeout,
            prompt_hash: hash,
        };
        assert!(timeout.is_retryable() && !timeout.is_budget());
        assert_eq!(timeout.prompt_hash(), "h");
    }
}
