//! # mush-llm — LLM transport + policy layer
//!
//! A thin, boring transport to two wire protocols (Anthropic Messages API and
//! OpenAI-compatible chat completions) under one policy front door,
//! [`LlmService`]. Design reference: `docs/design/03-llm-integration.md`
//! §§2–4, adapted to the validated OpenRouter/Anthropic provider facts.
//!
//! ## Shape
//!
//! - [`types`] — provider-neutral request/response types (Anthropic-shaped
//!   content blocks; the OpenAI adapter down-converts).
//! - [`provider`] — the [`LlmProvider`](provider::LlmProvider) trait,
//!   [`ProviderCaps`](provider::ProviderCaps), and the [`LlmError`] taxonomy.
//! - [`anthropic`] / [`openai`] — the two hand-rolled reqwest adapters
//!   (feature-gated `anthropic` / `openai`, both default-on).
//! - [`service`] — [`LlmService`](service::LlmService): route by model-id
//!   prefix, layered daily budgets, priority concurrency + per-provider QPS,
//!   retry with a circuit breaker, and an audit row per call.
//! - [`budget`] / [`audit`] / [`config`] — the ledger, audit sink, and TOML
//!   config structs.
//! - [`mock`] — a network-free [`MockProvider`](mock::MockProvider) for tests
//!   (feature `mock`, default-on).
//!
//! ## Invariants
//!
//! - **Never blocks the world loop.** All calls are `async` and live on
//!   detached tasks; game code snapshots context, calls, and re-enters as a
//!   typed command (treaty ruling 3). This crate holds no world reference.
//! - **Every call is metered, attributed, gated.** Kill switch → budget
//!   pre-check → settle → audit, with no path around it.
//! - **Secrets never leak.** API keys resolve from env vars at boot; they are
//!   never inlined in config, never logged, never placed in a prompt, and
//!   error/log strings are scrubbed.

pub mod audit;
pub mod budget;
pub mod config;
pub mod dashboard;
pub mod provider;
pub mod service;
pub mod types;

#[cfg(any(feature = "anthropic", feature = "openai"))]
mod http;

#[cfg(feature = "anthropic")]
pub mod anthropic;
#[cfg(feature = "openai")]
pub mod openai;

#[cfg(feature = "mock")]
pub mod mock;

// -- curated re-exports (the surface worldgen/npc import) ------------------

pub use audit::{AuditRecord, AuditSink, DaySlice, FeatureTotals, Outcome};
pub use budget::{BudgetLedger, BudgetLimit, Counters, ScopeLimits};
pub use config::{
    BudgetConfig, CircuitBreakerConfig, ConcurrencyConfig, LlmConfig, ProviderConfig, ProviderKind,
    RetryConfig, RouteConfig,
};
pub use dashboard::{BudgetStatus, DashboardSnapshot, RecentCall, SpendRow};
pub use provider::{BudgetScope, LlmError, LlmProvider, ProviderCaps, ProviderId};
pub use service::{
    CallCtx, KillSwitch, LlmFeature, LlmService, LlmServiceBuilder, NpcTier, Priority,
    ServiceError, derive_route, user_request,
};
pub use types::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentBlock, Effort, LlmStream,
    ReasoningConfig, Role, Sampling, StopReason, StreamEvent, ToolChoice, ToolDef, Usage,
};

#[cfg(feature = "anthropic")]
pub use anthropic::AnthropicProvider;
#[cfg(feature = "mock")]
pub use mock::MockProvider;
#[cfg(feature = "openai")]
pub use openai::OpenAiProvider;
