//! The provider trait, capability descriptor, error taxonomy, and the
//! provider-id/ budget-scope newtypes shared across the crate.

use std::fmt;
use std::time::Duration;

use smol_str::SmolStr;

use crate::types::{CompletionRequest, CompletionResponse, LlmStream};

/// A registered provider's stable id (`"anthropic"`, `"openrouter"`,
/// `"local-ollama"`). Derived from a route's model-id prefix by
/// [`crate::service`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProviderId(pub SmolStr);

impl ProviderId {
    /// Construct from any string-like.
    pub fn new(s: impl Into<SmolStr>) -> ProviderId {
        ProviderId(s.into())
    }

    /// The id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ProviderId {
    fn from(s: &str) -> ProviderId {
        ProviderId(SmolStr::new(s))
    }
}

/// Which budget ceiling a [`LlmError::BudgetExceeded`] tripped.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BudgetScope {
    /// The server-wide daily budget.
    Global,
    /// A per-feature daily budget.
    Feature,
    /// A per-actor (builder) daily quota — the `BUILDER_LLM` spend gate.
    Actor,
    /// A per-NPC daily budget.
    Npc,
}

impl fmt::Display for BudgetScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BudgetScope::Global => "global",
            BudgetScope::Feature => "feature",
            BudgetScope::Actor => "actor",
            BudgetScope::Npc => "npc",
        };
        f.write_str(s)
    }
}

/// Everything that can go wrong across the provider + policy layers.
#[derive(Clone, Debug, thiserror::Error)]
pub enum LlmError {
    /// HTTP 429 — rate limited. Carries `Retry-After` when the provider sent it.
    #[error("rate limited")]
    RateLimited {
        /// Suggested wait before retrying.
        retry_after: Option<Duration>,
    },
    /// HTTP 503/529 — the provider is overloaded (retryable).
    #[error("provider overloaded")]
    Overloaded,
    /// HTTP 401/403 — bad or missing API key (never retryable).
    #[error("authentication failed")]
    Auth,
    /// HTTP 400/422 — the request was rejected (never retryable). The string is
    /// scrubbed of any secret material before construction.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// The call exceeded its timeout (retryable).
    #[error("request timed out")]
    Timeout,
    /// Transport failure — DNS, connection, TLS, truncated body (retryable).
    #[error("network error: {0}")]
    Network(String),
    /// The model refused (Anthropic `stop_reason:"refusal"`, HTTP 200; OpenAI
    /// `content_filter`). Callers fall back (NPC canned line, worldgen reword).
    #[error("model refused")]
    Refused {
        /// Provider-supplied category, if any.
        category: Option<String>,
    },
    /// Tool JSON didn't parse or failed schema validation (triggers the
    /// worldgen repair loop).
    #[error("malformed output: {0}")]
    MalformedOutput(String),
    /// A budget ceiling was hit pre-call.
    #[error("budget exceeded ({scope})")]
    BudgetExceeded {
        /// Which ceiling.
        scope: BudgetScope,
    },
    /// A kill switch (global, per-feature, or per-NPC) is engaged.
    #[error("llm disabled")]
    Disabled,
}

impl LlmError {
    /// Whether the retry loop should retry this error (design 03 §3.5).
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            LlmError::RateLimited { .. }
                | LlmError::Overloaded
                | LlmError::Timeout
                | LlmError::Network(_)
        )
    }

    /// Whether this counts as a provider **transport** failure for the circuit
    /// breaker. A refusal / invalid-request / malformed-output is the provider
    /// answering fine — it must not trip the breaker; an unreachable or
    /// overloaded box must.
    #[must_use]
    pub fn is_transport_failure(&self) -> bool {
        matches!(
            self,
            LlmError::RateLimited { .. }
                | LlmError::Overloaded
                | LlmError::Timeout
                | LlmError::Network(_)
                | LlmError::Auth
        )
    }

    /// The `Retry-After` hint, when present.
    #[must_use]
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            LlmError::RateLimited { retry_after } => *retry_after,
            _ => None,
        }
    }
}

/// What a provider/model can do. Tool/JSON support varies per OpenAI-compatible
/// deployment, so those come from config; the Anthropic adapter hardcodes its
/// (stable) capabilities.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ProviderCaps {
    /// Supports tool calling at all.
    pub tool_calling: bool,
    /// Supports forcing a specific tool (worldgen structured output needs it).
    pub forced_tool_choice: bool,
    /// Supports native structured output (Anthropic output format / OpenAI
    /// json_schema response_format).
    pub native_structured_output: bool,
    /// Accepts `temperature`/`top_p` (false for current Anthropic flagship
    /// models).
    pub sampling_params: bool,
    /// Reports token usage on the streaming path (OpenAI needs
    /// `stream_options.include_usage`, which the adapter sends).
    pub streaming_usage: bool,
    /// Supports a reasoning-effort control.
    pub reasoning: bool,
}

impl Default for ProviderCaps {
    fn default() -> ProviderCaps {
        ProviderCaps {
            tool_calling: true,
            forced_tool_choice: true,
            native_structured_output: false,
            sampling_params: true,
            streaming_usage: true,
            reasoning: true,
        }
    }
}

/// A thin, boring transport to one LLM endpoint. All policy (routing, budgets,
/// retries, audit, kill switches) lives above this in [`crate::service`]; a
/// provider only speaks its wire protocol.
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    /// This provider's registry id.
    fn id(&self) -> &ProviderId;

    /// What this provider/model supports.
    fn caps(&self) -> ProviderCaps;

    /// One non-streaming completion, bounded by `timeout`.
    async fn complete(
        &self,
        req: &CompletionRequest,
        timeout: Duration,
    ) -> Result<CompletionResponse, LlmError>;

    /// One streaming completion, bounded by `timeout`. The final
    /// [`StreamEvent::Done`](crate::types::StreamEvent::Done) carries
    /// authoritative usage.
    async fn stream(
        &self,
        req: &CompletionRequest,
        timeout: Duration,
    ) -> Result<LlmStream, LlmError>;
}
