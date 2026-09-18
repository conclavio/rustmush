//! [`LlmService`] — the single front door game code uses to reach an LLM
//! (design 03 §4.2). Everything else in this crate is plumbing beneath it.
//!
//! `call()` pipeline: kill-switch check → route lookup (feature → model +
//! max_tokens + reasoning + timeout) → provider derivation by model-id prefix →
//! budget pre-check → priority semaphore + per-provider QPS → retry (backon
//! schedule, honoring `Retry-After`) around a per-provider circuit breaker →
//! budget settle with authoritative usage → audit row.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::StreamExt;
use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};
use mush_core::ObjId;
use tokio::sync::Semaphore;

use crate::audit::{AuditRecord, AuditSink, Outcome, now_unix, prompt_sha256, summarize};
use crate::budget::{BudgetLedger, ScopeLimits};
use crate::config::{
    BudgetConfig, CircuitBreakerConfig, ConcurrencyConfig, LlmConfig, ProviderKind, RetryConfig,
    RouteConfig, resolve_key,
};
use crate::provider::{LlmError, LlmProvider, ProviderId};
use crate::types::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentBlock, LlmStream, Role, StreamEvent,
};

/// NPC importance tier — selects the dialogue route + budget defaults (M8).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NpcTier {
    /// A named, plot-relevant NPC (strong model).
    Major,
    /// A recurring background NPC (cheap model).
    Minor,
    /// A throwaway NPC (cheapest / local model).
    Extra,
}

/// A metered LLM feature. The service maps each to a config route.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LlmFeature {
    /// NPC dialogue at a given tier.
    NpcDialogue(NpcTier),
    /// NPC memory summarization.
    NpcSummarize,
    /// H2a: one durable NPC decision — the provider chooses exactly one
    /// projected offer by tool call (route `npc_decision`).
    NpcDecision,
    /// Single-room worldgen draft.
    WorldgenRoom,
    /// Whole-building worldgen brief.
    WorldgenBrief,
    /// The building agent's planning run (S3, 14-spatial-implementation-plan
    /// §3.4): a bounded tool loop that turns a brief into a validating content
    /// pack (route `worldgen_plan`, a strong model).
    WorldgenPlan,
    /// Ambient flavor text.
    Ambience,
    /// The WoD Storyteller adjudication pass (M13): judge a freeform action
    /// proposal and return a structured ruling (strong model).
    StAdjudicate,
    /// The WoD Storyteller interpretation pass (M13): narrate the engine's
    /// logged dice outcome in-fiction (cheaper model).
    StInterpret,
    /// **16b cut N.2**: one squad's standing order in the combat arena (route
    /// `tacsim_deliberate`). One structured call, one forced tool whose arguments
    /// are the order, at `Priority::Background` — a squad with no order fights on
    /// its reflex doctrine, so this never competes with a player for the
    /// interactive semaphore.
    TacsimDeliberate,
    /// A staff smoke/test call.
    StaffTool,
}

impl LlmFeature {
    /// The config route name this feature maps to (also the per-feature budget
    /// key and the audit `feature` field).
    #[must_use]
    pub fn route_key(&self) -> &'static str {
        match self {
            LlmFeature::NpcDialogue(NpcTier::Major) => "npc_dialogue_major",
            LlmFeature::NpcDialogue(NpcTier::Minor) => "npc_dialogue_minor",
            LlmFeature::NpcDialogue(NpcTier::Extra) => "npc_dialogue_extra",
            LlmFeature::NpcSummarize => "npc_summarize",
            LlmFeature::NpcDecision => "npc_decision",
            LlmFeature::WorldgenRoom => "worldgen_room",
            LlmFeature::WorldgenBrief => "worldgen_brief",
            LlmFeature::WorldgenPlan => "worldgen_plan",
            LlmFeature::Ambience => "ambience",
            LlmFeature::StAdjudicate => "st_adjudicate",
            LlmFeature::StInterpret => "st_interpret",
            LlmFeature::TacsimDeliberate => "tacsim_deliberate",
            LlmFeature::StaffTool => "staff_tool",
        }
    }
}

/// Call priority — selects the concurrency semaphore.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Priority {
    /// Latency-sensitive (dialogue, single-room worldgen).
    Interactive,
    /// Bulk work (batch gen, summarization, ambience).
    Background,
}

/// Per-call context: what is being called for, by whom, at what priority.
#[derive(Copy, Clone, Debug)]
pub struct CallCtx {
    /// The metered feature.
    pub feature: LlmFeature,
    /// The triggering actor (builder/player) for budget + audit attribution.
    pub actor: Option<ObjId>,
    /// The NPC, for per-NPC budget + audit (agent turns).
    pub npc: Option<ObjId>,
    /// A per-NPC daily budget override (the NPC's `AgentProfile.daily_tokens`).
    /// When set, it **replaces** the config `per_npc_default` ceiling for this
    /// NPC — staff use it to cap a runaway agent or raise a plot-critical one.
    /// Only meaningful together with [`CallCtx::npc`].
    pub npc_limit: Option<crate::budget::BudgetLimit>,
    /// Priority.
    pub priority: Priority,
}

impl CallCtx {
    /// A minimal context (no actor/npc) at the given feature + priority.
    #[must_use]
    pub fn new(feature: LlmFeature, priority: Priority) -> CallCtx {
        CallCtx {
            feature,
            actor: None,
            npc: None,
            npc_limit: None,
            priority,
        }
    }

    /// Set the triggering actor.
    #[must_use]
    pub fn with_actor(mut self, actor: ObjId) -> CallCtx {
        self.actor = Some(actor);
        self
    }

    /// Set the NPC.
    #[must_use]
    pub fn with_npc(mut self, npc: ObjId) -> CallCtx {
        self.npc = Some(npc);
        self
    }

    /// Set the per-NPC daily budget override (from the NPC's profile). Replaces
    /// the config `per_npc_default` ceiling for this call's NPC scope.
    #[must_use]
    pub fn with_npc_limit(mut self, limit: crate::budget::BudgetLimit) -> CallCtx {
        self.npc_limit = Some(limit);
        self
    }
}

/// Failure building an [`LlmService`] from config.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    /// A configuration problem (missing feature, bad provider kind, …).
    #[error("llm config: {0}")]
    Config(String),
}

/// The global + per-feature + per-NPC kill switches (design 03 §4.3),
/// all checked pre-call. `global` is set from config `enabled = false`.
#[derive(Debug, Default)]
pub struct KillSwitch {
    global: AtomicBool,
    features: Mutex<std::collections::HashSet<String>>,
    npcs: Mutex<std::collections::HashSet<ObjId>>,
}

impl KillSwitch {
    /// A switch with the given initial global-disabled state.
    #[must_use]
    pub fn new(global_disabled: bool) -> KillSwitch {
        KillSwitch {
            global: AtomicBool::new(global_disabled),
            features: Mutex::new(std::collections::HashSet::new()),
            npcs: Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// Whether a call for `route_key`/`npc` is currently disabled.
    #[must_use]
    pub fn is_disabled(&self, route_key: &str, npc: Option<ObjId>) -> bool {
        if self.global.load(Ordering::Relaxed) {
            return true;
        }
        if self.features.lock().expect("kill lock").contains(route_key) {
            return true;
        }
        if let Some(n) = npc
            && self.npcs.lock().expect("kill lock").contains(&n)
        {
            return true;
        }
        false
    }

    /// Engage / lift the global kill switch (`@llm/disable`).
    pub fn set_global(&self, disabled: bool) {
        self.global.store(disabled, Ordering::Relaxed);
    }

    /// Whether the global switch is engaged.
    #[must_use]
    pub fn global_disabled(&self) -> bool {
        self.global.load(Ordering::Relaxed)
    }

    /// Disable one feature route.
    pub fn disable_feature(&self, route_key: &str) {
        self.features
            .lock()
            .expect("kill lock")
            .insert(route_key.to_string());
    }

    /// Re-enable one feature route.
    pub fn enable_feature(&self, route_key: &str) {
        self.features.lock().expect("kill lock").remove(route_key);
    }

    /// Disable one NPC (`@llm/kill <npc>`).
    pub fn disable_npc(&self, npc: ObjId) {
        self.npcs.lock().expect("kill lock").insert(npc);
    }

    /// Re-enable one NPC.
    pub fn enable_npc(&self, npc: ObjId) {
        self.npcs.lock().expect("kill lock").remove(&npc);
    }
}

/// A trivial per-provider circuit breaker: N consecutive transport failures
/// open it for a fixed window, during which calls fail fast with `Overloaded`.
#[derive(Debug, Default)]
pub struct CircuitBreaker {
    failures: AtomicU32,
    open_until: Mutex<Option<Instant>>,
}

impl CircuitBreaker {
    /// Whether the breaker is currently open (auto-clears when the window
    /// expires).
    #[must_use]
    pub fn is_open(&self) -> bool {
        let mut g = self.open_until.lock().expect("breaker lock");
        match *g {
            Some(t) if Instant::now() < t => true,
            Some(_) => {
                *g = None;
                false
            }
            None => false,
        }
    }

    fn record_success(&self) {
        self.failures.store(0, Ordering::Relaxed);
    }

    fn record_failure(&self, cfg: &CircuitBreakerConfig) {
        let n = self.failures.fetch_add(1, Ordering::Relaxed) + 1;
        if n >= cfg.consecutive_failures {
            *self.open_until.lock().expect("breaker lock") =
                Some(Instant::now() + Duration::from_secs(cfg.open_secs));
            self.failures.store(0, Ordering::Relaxed);
        }
    }
}

/// The LLM service.
pub struct LlmService {
    providers: HashMap<ProviderId, Arc<dyn LlmProvider>>,
    routes: HashMap<String, RouteConfig>,
    budgets: BudgetConfig,
    ledger: Arc<BudgetLedger>,
    audit: Arc<AuditSink>,
    kill: KillSwitch,
    sem_interactive: Arc<Semaphore>,
    sem_background: Arc<Semaphore>,
    admit_interactive: Arc<Semaphore>,
    admit_background: Arc<Semaphore>,
    limiters: HashMap<ProviderId, Arc<DefaultDirectRateLimiter>>,
    breakers: HashMap<ProviderId, CircuitBreaker>,
    retry: RetryConfig,
    breaker_cfg: CircuitBreakerConfig,
}

impl std::fmt::Debug for LlmService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LlmService(providers={:?}, routes={}, global_disabled={})",
            self.providers.keys().collect::<Vec<_>>(),
            self.routes.len(),
            self.kill.global_disabled()
        )
    }
}

impl LlmService {
    /// A builder (used by tests to inject a [`crate::mock::MockProvider`], and
    /// internally by [`LlmService::from_config`]).
    #[must_use]
    pub fn builder() -> LlmServiceBuilder {
        LlmServiceBuilder::default()
    }

    /// Build a service from parsed `[llm]` config, resolving API keys from
    /// their env vars. A provider whose key is unset is skipped (routes to it
    /// then fail fast with a clear error); this never logs a key.
    pub fn from_config(cfg: &LlmConfig) -> Result<LlmService, ServiceError> {
        let mut b = LlmService::builder()
            .global_enabled(cfg.enabled)
            .budgets(cfg.budgets.clone())
            .concurrency(cfg.concurrency.clone())
            .retry(cfg.retry.clone())
            .breaker(cfg.circuit_breaker.clone());

        for (name, route) in &cfg.routes {
            b = b.route(name, route.clone());
        }

        for (id, pcfg) in &cfg.providers {
            let pid = ProviderId::from(id.as_str());
            let key = pcfg.api_key_env.as_deref().and_then(resolve_key);
            match pcfg.kind {
                ProviderKind::Anthropic => {
                    #[cfg(feature = "anthropic")]
                    {
                        match key {
                            Some(key) => {
                                let p = crate::anthropic::AnthropicProvider::new(
                                    pid.clone(),
                                    pcfg,
                                    key,
                                )
                                .map_err(|e| ServiceError::Config(e.to_string()))?;
                                b = b.provider_with_rpm(
                                    pid.clone(),
                                    Arc::new(p),
                                    pcfg.requests_per_minute,
                                );
                            }
                            None => tracing::warn!(
                                provider = %pid,
                                "anthropic provider has no API key (env {:?} unset); routes to it will fail",
                                pcfg.api_key_env
                            ),
                        }
                    }
                    #[cfg(not(feature = "anthropic"))]
                    {
                        let _ = key;
                        return Err(ServiceError::Config(format!(
                            "provider {pid}: anthropic feature not compiled in"
                        )));
                    }
                }
                ProviderKind::Openai => {
                    #[cfg(feature = "openai")]
                    {
                        // A NAMED but unset key is a misconfiguration: skip +
                        // warn so a route to it fails fast with a clear error
                        // (mirrors the Anthropic arm and the documented
                        // contract). Only a provider with NO api_key_env is a
                        // legitimate keyless deployment (local Ollama/vLLM).
                        if pcfg.api_key_env.is_some() && key.is_none() {
                            tracing::warn!(
                                provider = %pid,
                                "openai provider has no API key (env {:?} unset); routes to it will fail",
                                pcfg.api_key_env
                            );
                        } else {
                            let p = crate::openai::OpenAiProvider::new(pid.clone(), pcfg, key)
                                .map_err(|e| ServiceError::Config(e.to_string()))?;
                            b = b.provider_with_rpm(
                                pid.clone(),
                                Arc::new(p),
                                pcfg.requests_per_minute,
                            );
                        }
                    }
                    #[cfg(not(feature = "openai"))]
                    {
                        let _ = key;
                        return Err(ServiceError::Config(format!(
                            "provider {pid}: openai feature not compiled in"
                        )));
                    }
                }
            }
        }

        Ok(b.build())
    }

    /// The budget ledger (staff `@llm/budget`, persistence seam).
    #[must_use]
    pub fn ledger(&self) -> &Arc<BudgetLedger> {
        &self.ledger
    }

    /// The audit log (staff `@llm/log`, persistence seam).
    #[must_use]
    pub fn audit_log(&self) -> &Arc<AuditSink> {
        &self.audit
    }

    /// The kill switches (staff `@llm/disable`, `@llm/kill`).
    #[must_use]
    pub fn kill_switch(&self) -> &KillSwitch {
        &self.kill
    }

    /// The safe cost/usage aggregate for the LLM dashboard (M10c): today's
    /// budget-vs-cap by scope, top spenders, a best-effort 7-day series, and
    /// a recent-calls tail. See [`crate::dashboard`] for the field-by-field
    /// contract; this is the one place with both the ledger/audit AND the
    /// configured routes + kill switch needed to assemble it.
    #[must_use]
    pub fn dashboard(&self) -> crate::dashboard::DashboardSnapshot {
        crate::dashboard::build(
            &self.ledger,
            &self.audit,
            &self.budgets,
            self.routes.keys().cloned(),
            self.kill.global_disabled(),
        )
    }

    /// A registered provider's capabilities (worldgen checks
    /// `forced_tool_choice` / `native_structured_output` before choosing its
    /// structured-output strategy). Resolves the provider the way `call` does.
    #[must_use]
    pub fn caps_for(&self, feature: LlmFeature) -> Option<crate::provider::ProviderCaps> {
        let route = self.routes.get(feature.route_key())?;
        let (pid, _) = derive_route(&route.model);
        self.providers.get(&pid).map(|p| p.caps())
    }

    /// The single non-streaming entry point.
    pub async fn call(
        &self,
        ctx: CallCtx,
        req: CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let start = Instant::now();
        let route_key = ctx.feature.route_key();

        // 1. Kill switch.
        if self.kill.is_disabled(route_key, ctx.npc) {
            self.write_audit(&ctx, "-", "-", &req, Outcome::Disabled, None, start);
            return Err(LlmError::Disabled);
        }

        // 2. Route → model/provider.
        let Some(route) = self.routes.get(route_key) else {
            let e =
                LlmError::InvalidRequest(format!("no route configured for feature {route_key}"));
            self.write_audit(&ctx, "-", "-", &req, Outcome::Error, None, start);
            return Err(e);
        };
        let (provider_id, model) = derive_route(&route.model);
        let Some(provider) = self.providers.get(&provider_id) else {
            let e = LlmError::InvalidRequest(format!(
                "no provider {provider_id} registered for feature {route_key}"
            ));
            self.write_audit(
                &ctx,
                provider_id.as_str(),
                &model,
                &req,
                Outcome::Error,
                None,
                start,
            );
            return Err(e);
        };

        // Effective request: route supplies model/max_tokens/reasoning.
        let mut eff = req;
        eff.model = model.clone();
        eff.max_tokens = route.max_tokens;
        eff.reasoning = route.reasoning_config();
        let timeout = route.timeout();

        // 3. Budget pre-check (estimated input + max_tokens).
        let limits = self.scope_limits(&ctx, route_key);
        let estimate = estimate_input_tokens(&eff) + u64::from(eff.max_tokens);
        if let Err(scope) = self.ledger.pre_check(&limits, estimate) {
            self.write_audit(
                &ctx,
                provider_id.as_str(),
                &model,
                &eff,
                Outcome::Budget,
                None,
                start,
            );
            return Err(LlmError::BudgetExceeded { scope });
        }

        // 4. Bounded admission, then priority semaphore. Tokio semaphores have
        // an unbounded waiter list; the outer permit caps active + queued calls
        // before this future is allowed to wait on the in-flight permit.
        let admission = match ctx.priority {
            Priority::Interactive => Arc::clone(&self.admit_interactive),
            Priority::Background => Arc::clone(&self.admit_background),
        };
        let _admission = match admission.try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.write_audit(
                    &ctx,
                    provider_id.as_str(),
                    &model,
                    &eff,
                    Outcome::Error,
                    None,
                    start,
                );
                return Err(LlmError::Overloaded);
            }
        };
        let sem = match ctx.priority {
            Priority::Interactive => &self.sem_interactive,
            Priority::Background => &self.sem_background,
        };
        let _permit = sem.acquire().await.expect("semaphore open");

        // 5. Per-provider QPS.
        if let Some(lim) = self.limiters.get(&provider_id) {
            lim.until_ready().await;
        }

        // 6. Circuit breaker + retry.
        let breaker = self.breakers.get(&provider_id);
        if let Some(b) = breaker
            && b.is_open()
        {
            self.write_audit(
                &ctx,
                provider_id.as_str(),
                &model,
                &eff,
                Outcome::Error,
                None,
                start,
            );
            return Err(LlmError::Overloaded);
        }

        let result = self
            .retrying_complete(provider.as_ref(), &eff, timeout)
            .await;

        if let Some(b) = breaker {
            match &result {
                Err(e) if e.is_transport_failure() => b.record_failure(&self.breaker_cfg),
                _ => b.record_success(),
            }
        }

        // 7. Settle + audit.
        match result {
            Ok(resp) => {
                self.ledger.settle(&limits, &resp.usage);
                self.write_audit(
                    &ctx,
                    provider_id.as_str(),
                    &model,
                    &eff,
                    Outcome::Ok,
                    Some(&resp),
                    start,
                );
                Ok(resp)
            }
            Err(e) => {
                let outcome = if matches!(e, LlmError::Refused { .. }) {
                    Outcome::Refused
                } else {
                    Outcome::Error
                };
                self.write_audit(
                    &ctx,
                    provider_id.as_str(),
                    &model,
                    &eff,
                    outcome,
                    None,
                    start,
                );
                Err(e)
            }
        }
    }

    /// The streaming entry point. Runs the same pre-flight (kill switch, route,
    /// budget pre-check, semaphore, QPS, breaker) and settles the budget +
    /// writes the audit row when the terminal
    /// [`StreamEvent::Done`](crate::types::StreamEvent::Done) arrives. The
    /// concurrency permit is held for the stream's lifetime.
    ///
    /// **WS17-S0 ruling: this entry point stays.** It has exactly one caller in
    /// the tree — its own test, `tests/service.rs`
    /// `streaming_settles_budget_and_emits_done` — so deleting it was on the
    /// table. The ruling is KEEP, because the settle-on-`Done` contract above is
    /// only half-built: a consumer that drops the stream before `Done` arrives
    /// settles no budget and writes no audit row at all, and removing the entry
    /// point would retire that hole rather than close it. So the Drop-settle
    /// guard is still owed here — an `Outcome::Abandoned` row plus a settle of
    /// what actually streamed, fired from `SettleState`'s `Drop` when `settled`
    /// is still false — and the first real streaming caller must not be the
    /// thing that discovers the gap.
    pub async fn call_streaming(
        &self,
        ctx: CallCtx,
        req: CompletionRequest,
    ) -> Result<LlmStream, LlmError> {
        let start = Instant::now();
        let route_key = ctx.feature.route_key();

        if self.kill.is_disabled(route_key, ctx.npc) {
            self.write_audit(&ctx, "-", "-", &req, Outcome::Disabled, None, start);
            return Err(LlmError::Disabled);
        }
        let Some(route) = self.routes.get(route_key) else {
            self.write_audit(&ctx, "-", "-", &req, Outcome::Error, None, start);
            return Err(LlmError::InvalidRequest(format!(
                "no route configured for feature {route_key}"
            )));
        };
        let (provider_id, model) = derive_route(&route.model);
        let Some(provider) = self.providers.get(&provider_id) else {
            self.write_audit(
                &ctx,
                provider_id.as_str(),
                &model,
                &req,
                Outcome::Error,
                None,
                start,
            );
            return Err(LlmError::InvalidRequest(format!(
                "no provider {provider_id} registered for feature {route_key}"
            )));
        };

        let mut eff = req;
        eff.model = model.clone();
        eff.max_tokens = route.max_tokens;
        eff.reasoning = route.reasoning_config();
        let timeout = route.timeout();

        let limits = self.scope_limits(&ctx, route_key);
        let estimate = estimate_input_tokens(&eff) + u64::from(eff.max_tokens);
        if let Err(scope) = self.ledger.pre_check(&limits, estimate) {
            self.write_audit(
                &ctx,
                provider_id.as_str(),
                &model,
                &eff,
                Outcome::Budget,
                None,
                start,
            );
            return Err(LlmError::BudgetExceeded { scope });
        }

        let admission = match ctx.priority {
            Priority::Interactive => Arc::clone(&self.admit_interactive),
            Priority::Background => Arc::clone(&self.admit_background),
        };
        let admission_permit = match admission.try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.write_audit(
                    &ctx,
                    provider_id.as_str(),
                    &model,
                    &eff,
                    Outcome::Error,
                    None,
                    start,
                );
                return Err(LlmError::Overloaded);
            }
        };
        let sem = match ctx.priority {
            Priority::Interactive => Arc::clone(&self.sem_interactive),
            Priority::Background => Arc::clone(&self.sem_background),
        };
        let permit = sem.acquire_owned().await.expect("semaphore open");

        if let Some(lim) = self.limiters.get(&provider_id) {
            lim.until_ready().await;
        }
        if let Some(b) = self.breakers.get(&provider_id)
            && b.is_open()
        {
            self.write_audit(
                &ctx,
                provider_id.as_str(),
                &model,
                &eff,
                Outcome::Error,
                None,
                start,
            );
            return Err(LlmError::Overloaded);
        }

        let inner = provider.stream(&eff, timeout).await;
        let inner = match inner {
            Ok(s) => s,
            Err(e) => {
                if let Some(b) = self.breakers.get(&provider_id)
                    && e.is_transport_failure()
                {
                    b.record_failure(&self.breaker_cfg);
                }
                let outcome = if matches!(e, LlmError::Refused { .. }) {
                    Outcome::Refused
                } else {
                    Outcome::Error
                };
                self.write_audit(
                    &ctx,
                    provider_id.as_str(),
                    &model,
                    &eff,
                    outcome,
                    None,
                    start,
                );
                return Err(e);
            }
        };

        // Wrap the inner stream to settle budget + audit on Done, holding the
        // permit for the stream's lifetime.
        let state = SettleState {
            inner,
            permit,
            _admission_permit: admission_permit,
            ledger: Arc::clone(&self.ledger),
            audit: Arc::clone(&self.audit),
            limits,
            settled: false,
            meta: AuditMeta {
                feature: route_key.to_string(),
                provider: provider_id.as_str().to_string(),
                model,
                actor: ctx.actor,
                npc: ctx.npc,
                prompt_sha256: prompt_sha256(&assemble_prompt_text(&eff)),
                prompt_summary: summarize(&last_user_text(&eff)),
                start,
            },
        };
        Ok(futures::stream::unfold(state, settle_step).boxed())
    }

    // -- internals ---------------------------------------------------------

    async fn retrying_complete(
        &self,
        provider: &dyn LlmProvider,
        req: &CompletionRequest,
        timeout: Duration,
    ) -> Result<CompletionResponse, LlmError> {
        use backon::{BackoffBuilder, ExponentialBuilder};

        let mut builder = ExponentialBuilder::default()
            .with_min_delay(Duration::from_millis(self.retry.min_delay_ms))
            .with_max_delay(Duration::from_millis(self.retry.max_delay_ms))
            .with_factor(self.retry.factor)
            .with_max_times(self.retry.max_retries);
        if self.retry.jitter {
            builder = builder.with_jitter();
        }
        let mut schedule = builder.build();

        loop {
            match provider.complete(req, timeout).await {
                Ok(r) => return Ok(r),
                Err(e) if e.is_retryable() => match schedule.next() {
                    Some(base) => {
                        let delay = e.retry_after().map_or(base, |ra| ra.max(base));
                        tracing::debug!(delay_ms = delay.as_millis() as u64, error = %e, "llm retry");
                        tokio::time::sleep(delay).await;
                    }
                    None => return Err(e),
                },
                Err(e) => return Err(e),
            }
        }
    }

    fn scope_limits(&self, ctx: &CallCtx, route_key: &str) -> ScopeLimits {
        // The per-NPC ceiling: the call's own override (the NPC's profile
        // `daily_tokens`) wins over the config `per_npc_default` — this is what
        // makes `@agent/set <npc>/daily_tokens=N` a live containment lever.
        let npc_limit = ctx.npc_limit.or(self.budgets.per_npc_default);
        // Record a counter at EVERY applicable scope, capped or not (M10c). An
        // uncapped scope carries `BudgetLimit::default()` — both axes `None`, so
        // it never trips `pre_check` — but `settle` still accumulates its spend.
        // Without this, `ledger.feature_totals()`/`top_actors()`/`top_npcs()`
        // only ever saw *capped* scopes, so the cost dashboard's per-feature
        // rows and top-spender lists silently omitted every feature/builder/NPC
        // that spent under no configured cap (e.g. `worldgen_brief`, `ambience`,
        // and — under a global-only cap config — every builder and NPC).
        ScopeLimits {
            global: Some(self.budgets.global.unwrap_or_default()),
            feature: Some((
                route_key.to_string(),
                self.budgets
                    .per_feature
                    .get(route_key)
                    .copied()
                    .unwrap_or_default(),
            )),
            actor: ctx
                .actor
                .map(|a| (a, self.budgets.per_actor_default.unwrap_or_default())),
            npc: ctx.npc.map(|n| (n, npc_limit.unwrap_or_default())),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn write_audit(
        &self,
        ctx: &CallCtx,
        provider: &str,
        model: &str,
        req: &CompletionRequest,
        outcome: Outcome,
        resp: Option<&CompletionResponse>,
        start: Instant,
    ) {
        let usage = resp.map(|r| r.usage).unwrap_or_default();
        let rec = AuditRecord {
            ts: now_unix(),
            feature: ctx.feature.route_key().to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            actor: ctx.actor,
            npc: ctx.npc,
            prompt_sha256: prompt_sha256(&assemble_prompt_text(req)),
            prompt_summary: summarize(&last_user_text(req)),
            response_summary: resp.map(|r| summarize(&r.text())).unwrap_or_default(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cost_usd: usage.cost_usd,
            latency_ms: start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            outcome,
        };
        self.audit.record(rec);
    }
}

// -- streaming settle adapter ---------------------------------------------

struct AuditMeta {
    feature: String,
    provider: String,
    model: String,
    actor: Option<ObjId>,
    npc: Option<ObjId>,
    prompt_sha256: String,
    prompt_summary: String,
    start: Instant,
}

struct SettleState {
    inner: LlmStream,
    permit: tokio::sync::OwnedSemaphorePermit,
    _admission_permit: tokio::sync::OwnedSemaphorePermit,
    ledger: Arc<BudgetLedger>,
    audit: Arc<AuditSink>,
    limits: ScopeLimits,
    settled: bool,
    meta: AuditMeta,
}

async fn settle_step(
    mut state: SettleState,
) -> Option<(Result<StreamEvent, LlmError>, SettleState)> {
    match state.inner.next().await {
        Some(Ok(StreamEvent::Done(resp))) => {
            if !state.settled {
                state.settled = true;
                state.ledger.settle(&state.limits, &resp.usage);
                let m = &state.meta;
                state.audit.record(AuditRecord {
                    ts: now_unix(),
                    feature: m.feature.clone(),
                    provider: m.provider.clone(),
                    model: m.model.clone(),
                    actor: m.actor,
                    npc: m.npc,
                    prompt_sha256: m.prompt_sha256.clone(),
                    prompt_summary: m.prompt_summary.clone(),
                    response_summary: summarize(&resp.text()),
                    input_tokens: resp.usage.input_tokens,
                    output_tokens: resp.usage.output_tokens,
                    cost_usd: resp.usage.cost_usd,
                    latency_ms: m.start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                    outcome: Outcome::Ok,
                });
            }
            Some((Ok(StreamEvent::Done(resp)), state))
        }
        Some(Ok(ev)) => Some((Ok(ev), state)),
        Some(Err(e)) => {
            if !state.settled {
                state.settled = true;
                let m = &state.meta;
                let outcome = if matches!(e, LlmError::Refused { .. }) {
                    Outcome::Refused
                } else {
                    Outcome::Error
                };
                state.audit.record(AuditRecord {
                    ts: now_unix(),
                    feature: m.feature.clone(),
                    provider: m.provider.clone(),
                    model: m.model.clone(),
                    actor: m.actor,
                    npc: m.npc,
                    prompt_sha256: m.prompt_sha256.clone(),
                    prompt_summary: m.prompt_summary.clone(),
                    response_summary: String::new(),
                    input_tokens: 0,
                    output_tokens: 0,
                    cost_usd: None,
                    latency_ms: m.start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                    outcome,
                });
            }
            Some((Err(e), state))
        }
        None => {
            // Stream ended; drop the permit by dropping state.
            drop(state.permit);
            None
        }
    }
}

// -- free helpers ----------------------------------------------------------

/// Derive `(provider id, model)` from a route model id: an `anthropic/` prefix
/// routes to the anthropic provider (prefix stripped); anything else routes to
/// the openrouter provider verbatim.
#[must_use]
pub fn derive_route(model: &str) -> (ProviderId, String) {
    match model.strip_prefix("anthropic/") {
        Some(stripped) => (ProviderId::from("anthropic"), stripped.to_string()),
        None => (ProviderId::from("openrouter"), model.to_string()),
    }
}

/// Conservative input-token estimate (chars/4 over prompt text) for the budget
/// pre-check. Authoritative counts come from the response `usage`.
fn estimate_input_tokens(req: &CompletionRequest) -> u64 {
    let mut chars = req.system.as_ref().map_or(0, String::len);
    for m in &req.messages {
        for b in &m.content {
            match b {
                ContentBlock::Text { text } => chars += text.len(),
                ContentBlock::ToolResult { content, .. } => chars += content.len(),
                ContentBlock::ToolUse { input, .. } => chars += input.to_string().len(),
            }
        }
    }
    for t in &req.tools {
        chars += t.description.len() + t.input_schema.to_string().len();
    }
    (chars as u64) / 4
}

/// A stable text rendering of the whole prompt, for the audit SHA-256.
fn assemble_prompt_text(req: &CompletionRequest) -> String {
    let mut s = String::new();
    if let Some(sys) = &req.system {
        s.push_str(sys);
        s.push('\n');
    }
    for m in &req.messages {
        for b in &m.content {
            if let ContentBlock::Text { text } = b {
                s.push_str(text);
                s.push('\n');
            }
        }
    }
    s
}

/// The last user turn's text (audit prompt summary).
fn last_user_text(req: &CompletionRequest) -> String {
    req.messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| {
            m.content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

// -- builder ---------------------------------------------------------------

/// Builder for [`LlmService`]. Tests inject providers directly; `from_config`
/// uses it internally.
#[derive(Default)]
pub struct LlmServiceBuilder {
    providers: HashMap<ProviderId, Arc<dyn LlmProvider>>,
    limiter_rpm: HashMap<ProviderId, u32>,
    routes: HashMap<String, RouteConfig>,
    budgets: BudgetConfig,
    concurrency: Option<ConcurrencyConfig>,
    retry: Option<RetryConfig>,
    breaker: Option<CircuitBreakerConfig>,
    ledger: Option<Arc<BudgetLedger>>,
    audit: Option<Arc<AuditSink>>,
    global_enabled: Option<bool>,
    build_limiters: bool,
}

impl LlmServiceBuilder {
    /// Register a provider (no QPS limiter — tests want none).
    #[must_use]
    pub fn provider(
        mut self,
        id: impl Into<ProviderId>,
        provider: Arc<dyn LlmProvider>,
    ) -> LlmServiceBuilder {
        self.providers.insert(id.into(), provider);
        self
    }

    /// Register a provider with a per-minute QPS limit; enables limiters.
    #[must_use]
    pub fn provider_with_rpm(
        mut self,
        id: impl Into<ProviderId>,
        provider: Arc<dyn LlmProvider>,
        rpm: u32,
    ) -> LlmServiceBuilder {
        let id = id.into();
        self.limiter_rpm.insert(id.clone(), rpm);
        self.providers.insert(id, provider);
        self.build_limiters = true;
        self
    }

    /// Add a feature route.
    #[must_use]
    pub fn route(mut self, name: impl Into<String>, route: RouteConfig) -> LlmServiceBuilder {
        self.routes.insert(name.into(), route);
        self
    }

    /// Set the budgets.
    #[must_use]
    pub fn budgets(mut self, budgets: BudgetConfig) -> LlmServiceBuilder {
        self.budgets = budgets;
        self
    }

    /// Set the concurrency limits.
    #[must_use]
    pub fn concurrency(mut self, c: ConcurrencyConfig) -> LlmServiceBuilder {
        self.concurrency = Some(c);
        self
    }

    /// Set the retry schedule.
    #[must_use]
    pub fn retry(mut self, r: RetryConfig) -> LlmServiceBuilder {
        self.retry = Some(r);
        self
    }

    /// Set the circuit-breaker config.
    #[must_use]
    pub fn breaker(mut self, c: CircuitBreakerConfig) -> LlmServiceBuilder {
        self.breaker = Some(c);
        self
    }

    /// Provide a shared budget ledger (e.g. one with an injected day source).
    #[must_use]
    pub fn ledger(mut self, ledger: Arc<BudgetLedger>) -> LlmServiceBuilder {
        self.ledger = Some(ledger);
        self
    }

    /// Provide a shared audit sink.
    #[must_use]
    pub fn audit(mut self, audit: Arc<AuditSink>) -> LlmServiceBuilder {
        self.audit = Some(audit);
        self
    }

    /// Set the global-enabled flag (config `enabled`).
    #[must_use]
    pub fn global_enabled(mut self, enabled: bool) -> LlmServiceBuilder {
        self.global_enabled = Some(enabled);
        self
    }

    /// Build the service.
    #[must_use]
    pub fn build(self) -> LlmService {
        let concurrency = self.concurrency.unwrap_or_default();
        let max_interactive = active_capacity(concurrency.max_interactive);
        let max_background = active_capacity(concurrency.max_background);
        let breaker_cfg = self.breaker.unwrap_or_default();
        let breakers = self
            .providers
            .keys()
            .map(|id| (id.clone(), CircuitBreaker::default()))
            .collect();
        let limiters = if self.build_limiters {
            self.limiter_rpm
                .iter()
                .map(|(id, rpm)| {
                    let q = Quota::per_minute(NonZeroU32::new((*rpm).max(1)).expect("nonzero"));
                    (id.clone(), Arc::new(RateLimiter::direct(q)))
                })
                .collect()
        } else {
            HashMap::new()
        };
        LlmService {
            providers: self.providers,
            routes: self.routes,
            budgets: self.budgets,
            ledger: self.ledger.unwrap_or_else(|| Arc::new(BudgetLedger::new())),
            audit: self.audit.unwrap_or_else(|| Arc::new(AuditSink::default())),
            kill: KillSwitch::new(!self.global_enabled.unwrap_or(true)),
            sem_interactive: Arc::new(Semaphore::new(max_interactive)),
            sem_background: Arc::new(Semaphore::new(max_background)),
            admit_interactive: Arc::new(Semaphore::new(admission_capacity(
                max_interactive,
                concurrency.max_interactive_queued,
            ))),
            admit_background: Arc::new(Semaphore::new(admission_capacity(
                max_background,
                concurrency.max_background_queued,
            ))),
            limiters,
            breakers,
            retry: self.retry.unwrap_or_default(),
            breaker_cfg,
        }
    }
}

fn active_capacity(configured: usize) -> usize {
    configured.clamp(1, Semaphore::MAX_PERMITS)
}

fn admission_capacity(active: usize, queued: usize) -> usize {
    active.saturating_add(queued).min(Semaphore::MAX_PERMITS)
}

/// Convenience: a single-user-turn request whose model/max_tokens/reasoning the
/// route will override.
#[must_use]
pub fn user_request(text: impl Into<String>) -> CompletionRequest {
    CompletionRequest {
        model: String::new(),
        system: None,
        messages: vec![ChatMessage::user(text)],
        tools: Vec::new(),
        tool_choice: crate::types::ToolChoice::Auto,
        max_tokens: 1024,
        sampling: crate::types::Sampling::default(),
        stop_sequences: Vec::new(),
        reasoning: None,
    }
}
