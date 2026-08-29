//! Provider-neutral request/response types, shaped after the Anthropic
//! Messages API (content blocks, tool_use/tool_result — the richer of the two
//! wire protocols). The OpenAI/OpenRouter adapter down-converts these; the
//! Anthropic adapter maps them almost 1:1.
//!
//! These types are transport-only: they carry no game state and no policy.
//! Routing, budgets, audit, and kill switches live in [`crate::service`].

use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::provider::LlmError;

/// Message author. `system` is a top-level [`CompletionRequest`] field, not a
/// role, matching the Anthropic shape.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// A player / caller turn.
    User,
    /// A model turn.
    Assistant,
}

/// One content block within a [`ChatMessage`].
///
/// `ToolResult` blocks are carried inside a `User` message (Anthropic shape);
/// the OpenAI adapter splits them out into `role:"tool"` messages.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ContentBlock {
    /// Free text.
    Text {
        /// The text.
        text: String,
    },
    /// A model-requested tool invocation (may appear multiple times per turn).
    ToolUse {
        /// Provider-assigned call id (correlates the matching `ToolResult`).
        id: String,
        /// The tool name.
        name: String,
        /// The tool input (already parsed from JSON — never a raw string).
        input: serde_json::Value,
    },
    /// The caller's answer to a prior `ToolUse`.
    ToolResult {
        /// The `ToolUse::id` this answers.
        tool_use_id: String,
        /// The result payload (rendered to text for the wire).
        content: String,
        /// Whether the tool call failed (drives self-correction).
        is_error: bool,
    },
}

impl ContentBlock {
    /// Shorthand text block.
    pub fn text(t: impl Into<String>) -> ContentBlock {
        ContentBlock::Text { text: t.into() }
    }
}

/// One conversation turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Who authored the turn.
    pub role: Role,
    /// The turn's content blocks.
    pub content: Vec<ContentBlock>,
}

impl ChatMessage {
    /// A user turn carrying a single text block.
    pub fn user(text: impl Into<String>) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: vec![ContentBlock::text(text)],
        }
    }

    /// An assistant turn carrying a single text block.
    pub fn assistant(text: impl Into<String>) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: vec![ContentBlock::text(text)],
        }
    }
}

/// A tool the model may call. `input_schema` is JSON Schema (worldgen sets
/// `additionalProperties:false` + `required`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    /// Tool name (matched in `ToolUse::name`).
    pub name: String,
    /// What the tool does (guides the model).
    pub description: String,
    /// JSON Schema for the tool input.
    pub input_schema: serde_json::Value,
    /// Request strict schema adherence where the provider/model supports it
    /// (Anthropic `strict`, OpenAI `function.strict`).
    pub strict: bool,
}

/// How the model is allowed to (or forced to) use tools.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolChoice {
    /// The model decides.
    Auto,
    /// The model must call some tool.
    Required,
    /// The model must call this specific tool (worldgen's structured-output
    /// mechanism).
    Tool(String),
    /// Tools are disabled for this call.
    None,
}

/// Sampling hints. Current Anthropic flagship models reject `temperature`/
/// `top_p` (HTTP 400), so the Anthropic adapter drops them for those models
/// (capability-gated); OpenAI-compatible adapters pass them through.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sampling {
    /// Sampling temperature.
    pub temperature: Option<f32>,
    /// Nucleus sampling top-p.
    pub top_p: Option<f32>,
}

/// Reasoning-effort control. OpenRouter reasoning models burn output budget on
/// hidden reasoning tokens, so this is load-bearing: `max_tokens` must have
/// headroom for reasoning **plus** the visible answer.
///
/// - [`ReasoningConfig::Effort`] → OpenRouter `{"reasoning":{"effort":…}}`;
///   Anthropic `{"output_config":{"effort":…}}`.
/// - [`ReasoningConfig::Disabled`] → OpenRouter `{"reasoning":{"enabled":false}}`.
/// - [`ReasoningConfig::MaxTokens`] → OpenRouter `{"reasoning":{"max_tokens":N}}`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReasoningConfig {
    /// A qualitative effort level.
    Effort(Effort),
    /// No reasoning (fast path).
    Disabled,
    /// A hard reasoning-token ceiling.
    MaxTokens(u32),
}

/// Reasoning effort levels.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    /// Latency-sensitive (NPC dialogue default).
    Low,
    /// Balanced.
    Medium,
    /// Quality-sensitive (worldgen default).
    High,
}

impl Effort {
    /// The wire string (`"low"`/`"medium"`/`"high"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
        }
    }
}

/// A neutral completion request. The service overrides `model`, `max_tokens`,
/// and `reasoning` from the feature route before it reaches an adapter; callers
/// own `system`, `messages`, `tools`, `tool_choice`, `sampling`, and
/// `stop_sequences`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// Model id (route-supplied; the Anthropic adapter receives it already
    /// stripped of the `anthropic/` prefix).
    pub model: String,
    /// Optional system prompt (the stable, cache-friendly prefix).
    pub system: Option<String>,
    /// The conversation.
    pub messages: Vec<ChatMessage>,
    /// Available tools.
    pub tools: Vec<ToolDef>,
    /// Tool-use policy.
    pub tool_choice: ToolChoice,
    /// Output-token ceiling (must include reasoning headroom).
    pub max_tokens: u32,
    /// Sampling hints.
    pub sampling: Sampling,
    /// Stop sequences.
    pub stop_sequences: Vec<String>,
    /// Reasoning control (route-supplied); `None` leaves it to the provider
    /// default.
    pub reasoning: Option<ReasoningConfig>,
}

impl CompletionRequest {
    /// A minimal single-user-turn request (tests, smoke calls). The service
    /// fills in `model`/`max_tokens`/`reasoning` from the route.
    #[must_use]
    pub fn simple(user: impl Into<String>) -> CompletionRequest {
        CompletionRequest {
            model: String::new(),
            system: None,
            messages: vec![ChatMessage::user(user)],
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            max_tokens: 1024,
            sampling: Sampling::default(),
            stop_sequences: Vec::new(),
            reasoning: None,
        }
    }
}

/// Why generation stopped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    /// Natural end of the assistant turn.
    EndTurn,
    /// Hit `max_tokens`.
    MaxTokens,
    /// Stopped to emit tool calls.
    ToolUse,
    /// Hit a stop sequence.
    StopSequence,
    /// The model refused (Anthropic `stop_reason:"refusal"`, OpenAI
    /// `content_filter`). The service maps this to [`LlmError::Refused`].
    Refusal,
    /// Anything else (provider-specific), preserved verbatim.
    Other(String),
}

/// Token/cost accounting for one call.
///
/// `cost_usd` is authoritative for OpenRouter (`usage.cost`, real money) and an
/// estimate for Anthropic (tokens × configured per-MTok price); `None` when no
/// price is configured. `reasoning_tokens` is OpenRouter's
/// `completion_tokens_details.reasoning_tokens` — the hidden budget these
/// models consume, surfaced for cost tuning.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// Prompt/input tokens.
    pub input_tokens: u64,
    /// Completion/output tokens (includes reasoning tokens).
    pub output_tokens: u64,
    /// Cached input tokens read (prompt-cache hit).
    pub cache_read_tokens: u64,
    /// Hidden reasoning tokens (OpenRouter reasoning models); `0` when absent.
    pub reasoning_tokens: u64,
    /// Cost in USD — authoritative (OpenRouter) or estimated (Anthropic).
    pub cost_usd: Option<f64>,
}

/// A completed generation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// Content blocks (may contain multiple `ToolUse` blocks).
    pub content: Vec<ContentBlock>,
    /// Why it stopped.
    pub stop_reason: StopReason,
    /// Accounting.
    pub usage: Usage,
    /// The model that actually served the request (echoed by the provider).
    pub model: String,
}

impl CompletionResponse {
    /// Concatenated text of all `Text` blocks (for summaries / simple callers).
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = String::new();
        for b in &self.content {
            if let ContentBlock::Text { text } = b {
                out.push_str(text);
            }
        }
        out
    }

    /// The first `ToolUse` block, if any (worldgen's forced-tool path).
    #[must_use]
    pub fn first_tool_use(&self) -> Option<(&str, &serde_json::Value)> {
        self.content.iter().find_map(|b| match b {
            ContentBlock::ToolUse { name, input, .. } => Some((name.as_str(), input)),
            _ => None,
        })
    }
}

/// A streaming event. Streaming is defined from day one (NPC dialogue in M8
/// and worldgen preview want it); worldgen itself is non-streaming.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamEvent {
    /// A chunk of assistant text.
    TextDelta(String),
    /// A tool call began.
    ToolUseStart {
        /// The call id.
        id: String,
        /// The tool name.
        name: String,
    },
    /// A chunk of a tool call's JSON input.
    ToolInputDelta {
        /// The call id.
        id: String,
        /// A raw JSON fragment (concatenate across deltas, then parse).
        json_fragment: String,
    },
    /// The final assembled response, carrying authoritative usage.
    Done(CompletionResponse),
}

/// A boxed stream of [`StreamEvent`]s (or an [`LlmError`] mid-stream).
pub type LlmStream = BoxStream<'static, Result<StreamEvent, LlmError>>;
