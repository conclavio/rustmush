//! Serde structs for the `[llm]` section of `mushd.toml`, plus env-var key
//! resolution.
//!
//! **Security:** API keys are never inlined here. Each provider names an env
//! var (`api_key_env`); the key is read from the environment at service build
//! time and never logged, never Debug-printed, never placed in a prompt.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::budget::BudgetLimit;
use crate::types::{Effort, ReasoningConfig};

/// The whole `[llm]` config block.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct LlmConfig {
    /// Global kill switch. `false` disables every LLM feature (the M7 default
    /// in `mushd.toml.example`).
    pub enabled: bool,
    /// Provider endpoints, keyed by provider id (`"anthropic"`, `"openrouter"`).
    pub providers: HashMap<String, ProviderConfig>,
    /// Feature routes, keyed by route name (`"worldgen_room"`, …).
    pub routes: HashMap<String, RouteConfig>,
    /// Layered daily budgets.
    pub budgets: BudgetConfig,
    /// Concurrency + QPS limits.
    pub concurrency: ConcurrencyConfig,
    /// Retry schedule.
    pub retry: RetryConfig,
    /// Per-provider circuit breaker.
    pub circuit_breaker: CircuitBreakerConfig,
}

/// One provider endpoint.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProviderConfig {
    /// Wire protocol: `"anthropic"` or `"openai"` (OpenRouter/Ollama/vLLM).
    pub kind: ProviderKind,
    /// Base URL (e.g. `https://api.anthropic.com` or
    /// `https://openrouter.ai/api/v1`).
    pub base_url: String,
    /// Name of the env var holding the API key. The key is resolved at boot;
    /// a missing var registers the provider as disabled.
    pub api_key_env: Option<String>,
    /// QPS ceiling, requests per minute (per-provider governor cell).
    #[serde(default = "default_rpm")]
    pub requests_per_minute: u32,
    /// `HTTP-Referer` header (OpenRouter attribution).
    pub referer: Option<String>,
    /// `X-Title` header (OpenRouter attribution).
    pub title: Option<String>,
    /// `anthropic-version` header (Anthropic only).
    pub anthropic_version: Option<String>,
    /// Estimated price per million **input** tokens, USD (Anthropic cost
    /// estimation — OpenRouter reports real cost instead).
    pub price_input_per_mtok: Option<f64>,
    /// Estimated price per million **output** tokens, USD.
    pub price_output_per_mtok: Option<f64>,
    /// Cap override: does the loaded model support tool calling?
    #[serde(default = "default_true")]
    pub tool_calling: bool,
    /// Cap override: forced tool choice supported?
    #[serde(default = "default_true")]
    pub forced_tool_choice: bool,
    /// Cap override: native structured output supported?
    #[serde(default)]
    pub native_structured_output: bool,
}

/// Which wire adapter serves a provider.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Anthropic Messages API.
    Anthropic,
    /// OpenAI-compatible chat completions (OpenRouter, Ollama, vLLM).
    Openai,
}

/// One feature route: the model + call shape a feature maps to. Routing to a
/// provider is by the model-id prefix (`anthropic/…` → the anthropic provider,
/// stripped; anything else → the openrouter provider, verbatim), so there is no
/// explicit provider field here.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RouteConfig {
    /// Full model id, e.g. `"anthropic/claude-opus-5"` or `"z-ai/glm-5.3"`.
    pub model: String,
    /// Output-token ceiling (reasoning headroom included).
    pub max_tokens: u32,
    /// Per-call timeout, seconds.
    #[serde(default = "default_timeout_s")]
    pub timeout_s: u64,
    /// Reasoning effort: `"low"`/`"medium"`/`"high"`/`"off"` (mutually exclusive
    /// with `reasoning_max_tokens`).
    pub reasoning: Option<String>,
    /// Reasoning-token ceiling (overrides `reasoning`).
    pub reasoning_max_tokens: Option<u32>,
}

impl RouteConfig {
    /// The route's per-call timeout.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_s)
    }

    /// Resolve the neutral [`ReasoningConfig`], if configured.
    #[must_use]
    pub fn reasoning_config(&self) -> Option<ReasoningConfig> {
        if let Some(n) = self.reasoning_max_tokens {
            return Some(ReasoningConfig::MaxTokens(n));
        }
        match self.reasoning.as_deref()? {
            "low" => Some(ReasoningConfig::Effort(Effort::Low)),
            "medium" => Some(ReasoningConfig::Effort(Effort::Medium)),
            "high" => Some(ReasoningConfig::Effort(Effort::High)),
            "off" | "none" | "disabled" | "false" => Some(ReasoningConfig::Disabled),
            _ => None,
        }
    }
}

/// Layered daily budgets. Any layer may cap tokens, cost, or both; a call is
/// refused pre-flight if any applicable layer would be exceeded.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct BudgetConfig {
    /// Server-wide daily ceiling.
    pub global: Option<BudgetLimit>,
    /// Per-feature ceilings, keyed by route name.
    pub per_feature: HashMap<String, BudgetLimit>,
    /// Default per-actor (builder) daily quota.
    pub per_actor_default: Option<BudgetLimit>,
    /// Default per-NPC daily quota.
    pub per_npc_default: Option<BudgetLimit>,
}

/// Concurrency + QPS.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ConcurrencyConfig {
    /// Max in-flight `Interactive` calls (dialogue, single-room worldgen).
    pub max_interactive: usize,
    /// Max in-flight `Background` calls (batch gen, summarization).
    pub max_background: usize,
}

impl Default for ConcurrencyConfig {
    fn default() -> ConcurrencyConfig {
        ConcurrencyConfig {
            max_interactive: 8,
            max_background: 3,
        }
    }
}

/// Exponential-backoff retry schedule (design 03 §3.5).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct RetryConfig {
    /// Base delay before the first retry.
    pub min_delay_ms: u64,
    /// Maximum delay cap.
    pub max_delay_ms: u64,
    /// Multiplier between retries.
    pub factor: f32,
    /// Maximum retries (total attempts = this + 1).
    pub max_retries: usize,
    /// Add jitter to each delay.
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> RetryConfig {
        RetryConfig {
            min_delay_ms: 500,
            max_delay_ms: 30_000,
            factor: 2.0,
            max_retries: 3,
            jitter: true,
        }
    }
}

/// Per-provider circuit breaker.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct CircuitBreakerConfig {
    /// Consecutive transport failures that open the breaker.
    pub consecutive_failures: u32,
    /// How long the breaker stays open (fail-fast) once tripped.
    pub open_secs: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            consecutive_failures: 5,
            open_secs: 60,
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_rpm() -> u32 {
    60
}
fn default_timeout_s() -> u64 {
    60
}

/// Resolve an API key from its env var. Returns `None` (never an error, never a
/// log line containing the value) when the var is unset or empty.
#[must_use]
pub fn resolve_key(env_name: &str) -> Option<String> {
    match std::env::var(env_name) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}
