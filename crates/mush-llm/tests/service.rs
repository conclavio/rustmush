//! MockProvider-driven `LlmService` tests: routing by model-id prefix,
//! feature→route override, budget pre-check + settle, kill switch, per-actor
//! quota, retry, circuit breaker, and refusal handling. No network.

#![cfg(feature = "mock")]

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::stream;
use mush_core::ObjId;
use mush_llm::budget::BudgetLimit;
use mush_llm::config::{
    BudgetConfig, CircuitBreakerConfig, ConcurrencyConfig, RetryConfig, RouteConfig,
};
use mush_llm::mock::{self, MockProvider};
use mush_llm::provider::{LlmError, LlmProvider, ProviderCaps, ProviderId};
use mush_llm::service::{CallCtx, LlmFeature, LlmService, NpcTier, Priority};
use mush_llm::types::{CompletionRequest, CompletionResponse, LlmStream, StreamEvent};
use tokio::sync::Notify;

fn route(model: &str, max_tokens: u32) -> RouteConfig {
    RouteConfig {
        model: model.to_string(),
        max_tokens,
        timeout_s: 30,
        reasoning: None,
        reasoning_max_tokens: None,
    }
}

fn fast_retry(max_retries: usize) -> RetryConfig {
    RetryConfig {
        min_delay_ms: 1,
        max_delay_ms: 2,
        factor: 2.0,
        max_retries,
        jitter: false,
    }
}

fn ctx(feature: LlmFeature) -> CallCtx {
    CallCtx::new(feature, Priority::Interactive)
}

fn req() -> CompletionRequest {
    CompletionRequest::simple("hello there")
}

struct BlockingProvider {
    id: ProviderId,
    entered: Notify,
    release: Notify,
}

impl BlockingProvider {
    fn new() -> Self {
        Self {
            id: ProviderId::new("openrouter"),
            entered: Notify::new(),
            release: Notify::new(),
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for BlockingProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn caps(&self) -> ProviderCaps {
        ProviderCaps::default()
    }

    async fn complete(
        &self,
        _req: &CompletionRequest,
        _timeout: Duration,
    ) -> Result<CompletionResponse, LlmError> {
        self.entered.notify_waiters();
        self.release.notified().await;
        Ok(mock::ok_response("ok", mock::usage(1, 1, None)))
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
        _timeout: Duration,
    ) -> Result<LlmStream, LlmError> {
        Ok(stream::empty::<Result<StreamEvent, LlmError>>().boxed())
    }
}

#[tokio::test]
async fn concurrency_admission_refuses_instead_of_growing_unbounded_waiters() {
    let provider = Arc::new(BlockingProvider::new());
    let svc = Arc::new(
        LlmService::builder()
            .global_enabled(true)
            .provider("openrouter", Arc::clone(&provider) as Arc<dyn LlmProvider>)
            .route("worldgen_room", route("z-ai/glm-5.3", 64))
            .concurrency(ConcurrencyConfig {
                max_interactive: 1,
                max_interactive_queued: 0,
                max_background: 1,
                max_background_queued: 0,
            })
            .build(),
    );

    let entered = provider.entered.notified();
    let first = tokio::spawn({
        let svc = Arc::clone(&svc);
        async move { svc.call(ctx(LlmFeature::WorldgenRoom), req()).await }
    });
    entered.await;

    let second = svc
        .call(ctx(LlmFeature::WorldgenRoom), req())
        .await
        .expect_err("active-plus-queued capacity is exhausted");
    assert!(matches!(second, LlmError::Overloaded));

    provider.release.notify_one();
    first.await.unwrap().unwrap();
}

#[tokio::test]
async fn routes_by_model_id_prefix() {
    let anthropic = Arc::new(MockProvider::new("anthropic"));
    let openrouter = Arc::new(MockProvider::new("openrouter"));
    anthropic.push_ok("from anthropic", mock::usage(5, 5, None));
    openrouter.push_ok("from openrouter", mock::usage(5, 5, None));

    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("anthropic", anthropic.clone() as Arc<dyn LlmProvider>)
        .provider("openrouter", openrouter.clone() as Arc<dyn LlmProvider>)
        // StaffTool → anthropic (prefix), WorldgenRoom → openrouter (verbatim).
        .route("staff_tool", route("anthropic/claude-opus-5", 64))
        .route("worldgen_room", route("z-ai/glm-5.3", 64))
        .build();

    svc.call(ctx(LlmFeature::StaffTool), req()).await.unwrap();
    assert_eq!(
        anthropic.call_count(),
        1,
        "anthropic prefix routed to anthropic"
    );
    assert_eq!(openrouter.call_count(), 0);

    svc.call(ctx(LlmFeature::WorldgenRoom), req())
        .await
        .unwrap();
    assert_eq!(
        openrouter.call_count(),
        1,
        "non-prefixed routed to openrouter"
    );

    // Prefix stripped for anthropic; verbatim for openrouter.
    assert_eq!(anthropic.last_request().unwrap().model, "claude-opus-5");
    assert_eq!(openrouter.last_request().unwrap().model, "z-ai/glm-5.3");
}

#[tokio::test]
async fn route_overrides_model_max_tokens_and_reasoning() {
    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.push_ok("ok", mock::usage(1, 1, None));
    let mut r = route("deepseek/deepseek-v4-flash", 777);
    r.reasoning = Some("high".to_string());

    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_room", r)
        .build();

    // Caller-supplied model/max_tokens are ignored; the route wins.
    let mut caller = req();
    caller.model = "ignored".to_string();
    caller.max_tokens = 1;
    svc.call(ctx(LlmFeature::WorldgenRoom), caller)
        .await
        .unwrap();

    let sent = provider.last_request().unwrap();
    assert_eq!(sent.model, "deepseek/deepseek-v4-flash");
    assert_eq!(sent.max_tokens, 777);
    assert_eq!(
        sent.reasoning,
        Some(mush_llm::types::ReasoningConfig::Effort(
            mush_llm::types::Effort::High
        ))
    );
}

#[tokio::test]
async fn budget_pre_check_refuses_then_settles_with_cost() {
    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.push_ok("draft", mock::usage(10, 20, Some(0.0005)));

    let budgets = BudgetConfig {
        global: Some(BudgetLimit::tokens(1_000_000)),
        ..Default::default()
    };
    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_room", route("z-ai/glm-5.3", 100))
        .budgets(budgets)
        .build();

    svc.call(ctx(LlmFeature::WorldgenRoom), req())
        .await
        .unwrap();

    // Authoritative usage settled into the global scope (cost preferred).
    let g = svc.ledger().global();
    assert_eq!(g.input_tokens, 10);
    assert_eq!(g.output_tokens, 20);
    assert!((g.cost_usd - 0.0005).abs() < 1e-12);
    assert_eq!(provider.call_count(), 1);
}

#[tokio::test]
async fn budget_pre_check_refuses_over_ceiling() {
    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.push_ok("should not run", mock::usage(1, 1, None));

    // Ceiling 50 tokens; estimate = chars/4 + max_tokens(100) > 50.
    let budgets = BudgetConfig {
        global: Some(BudgetLimit::tokens(50)),
        ..Default::default()
    };
    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_room", route("z-ai/glm-5.3", 100))
        .budgets(budgets)
        .build();

    let err = svc
        .call(ctx(LlmFeature::WorldgenRoom), req())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        LlmError::BudgetExceeded {
            scope: mush_llm::provider::BudgetScope::Global
        }
    ));
    assert_eq!(
        provider.call_count(),
        0,
        "refused before the provider is touched"
    );
}

#[tokio::test]
async fn kill_switch_blocks_calls() {
    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.set_default(Ok(mock::ok_response("ok", mock::usage(1, 1, None))));

    // Global disabled via config (`enabled = false`).
    let svc = LlmService::builder()
        .global_enabled(false)
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_room", route("z-ai/glm-5.3", 64))
        .build();
    assert!(matches!(
        svc.call(ctx(LlmFeature::WorldgenRoom), req()).await,
        Err(LlmError::Disabled)
    ));
    assert_eq!(provider.call_count(), 0);

    // Lift global, then disable just the feature.
    svc.kill_switch().set_global(false);
    svc.kill_switch().disable_feature("worldgen_room");
    assert!(matches!(
        svc.call(ctx(LlmFeature::WorldgenRoom), req()).await,
        Err(LlmError::Disabled)
    ));
    // A different feature route still works.
    svc.call(ctx(LlmFeature::StaffTool), req()).await.err(); // no route → error, but not Disabled
}

/// MW.17b: a call whose caller knows its NPC is parked is refused as a kill
/// switch refuses it, before any provider is asked, and the refusal is audited
/// against that NPC. The same NPC, not parked, is served.
#[tokio::test]
async fn a_parked_npcs_call_is_refused_as_disabled_and_audited_against_it() {
    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.set_default(Ok(mock::ok_response("ok", mock::usage(3, 1, None))));
    let svc = LlmService::builder()
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("npc_decision", route("z-ai/glm-5.3", 64))
        .build();
    let npc = ObjId(12);
    let call = ctx(LlmFeature::NpcDecision).with_npc(npc);

    assert!(matches!(
        svc.call(call.with_parked(true), req()).await,
        Err(LlmError::Disabled)
    ));
    assert_eq!(
        provider.call_count(),
        0,
        "a parked call reached the provider"
    );
    assert_eq!(svc.ledger().npc(npc).total_tokens(), 0);
    let rows = svc.audit_log().recent(8);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].npc, Some(npc));
    assert_eq!(rows[0].outcome, mush_llm::Outcome::Disabled);

    svc.call(call, req())
        .await
        .expect("the same NPC, not parked");
    assert_eq!(provider.call_count(), 1);
    assert_eq!(svc.ledger().npc(npc).total_tokens(), 4);
}

#[tokio::test]
async fn per_actor_quota_enforced() {
    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.set_default(Ok(mock::ok_response("ok", mock::usage(100, 100, None))));

    let budgets = BudgetConfig {
        per_actor_default: Some(BudgetLimit::tokens(300)),
        ..Default::default()
    };
    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_room", route("z-ai/glm-5.3", 100))
        .budgets(budgets)
        .build();

    let builder = ObjId(42);
    let c = ctx(LlmFeature::WorldgenRoom).with_actor(builder);

    // First call: 200 tokens settled, under 300.
    svc.call(c, req()).await.unwrap();
    assert_eq!(svc.ledger().actor(builder).input_tokens, 100);

    // Second call: 200 used + estimate(≈100) > 300 → refused for the actor.
    let err = svc.call(c, req()).await.unwrap_err();
    assert!(matches!(
        err,
        LlmError::BudgetExceeded {
            scope: mush_llm::provider::BudgetScope::Actor
        }
    ));
    assert_eq!(
        provider.call_count(),
        1,
        "the second call never reached the provider"
    );
}

#[tokio::test]
async fn per_npc_limit_override_beats_the_default() {
    // A generous per-NPC default; the call's own override (an NPC's
    // `daily_tokens`) is the live ceiling — it both lowers (this test) and, for
    // a plot-critical NPC, could raise the default.
    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.set_default(Ok(mock::ok_response("ok", mock::usage(10, 10, None))));

    let budgets = BudgetConfig {
        per_npc_default: Some(BudgetLimit::tokens(1_000_000)),
        ..Default::default()
    };
    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("npc_dialogue_minor", route("z-ai/glm-5.3", 100))
        .budgets(budgets)
        .build();

    let chatty = ObjId(7);
    // An override far below any real turn's estimate trips the NPC scope,
    // even though the config default is generous.
    let capped = ctx(LlmFeature::NpcDialogue(NpcTier::Minor))
        .with_npc(chatty)
        .with_npc_limit(BudgetLimit::tokens(1));
    let err = svc.call(capped, req()).await.unwrap_err();
    assert!(
        matches!(
            err,
            LlmError::BudgetExceeded {
                scope: mush_llm::provider::BudgetScope::Npc
            }
        ),
        "the per-NPC override tripped the NPC budget scope: {err:?}"
    );
    assert_eq!(provider.call_count(), 0, "refused before the provider");

    // Another NPC with no override rides the generous default and succeeds.
    let other = ctx(LlmFeature::NpcDialogue(NpcTier::Minor)).with_npc(ObjId(8));
    svc.call(other, req()).await.unwrap();
    assert_eq!(provider.call_count(), 1);
}

#[tokio::test]
async fn retries_rate_limited_then_succeeds() {
    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.push_error(LlmError::RateLimited { retry_after: None });
    provider.push_error(LlmError::Overloaded);
    provider.push_ok("finally", mock::usage(3, 3, None));

    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_room", route("z-ai/glm-5.3", 64))
        .retry(fast_retry(4))
        .build();

    let resp = svc
        .call(ctx(LlmFeature::WorldgenRoom), req())
        .await
        .unwrap();
    assert_eq!(resp.text(), "finally");
    assert_eq!(provider.call_count(), 3, "two retries then success");
}

#[tokio::test]
async fn retry_gives_up_on_non_retryable() {
    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.push_error(LlmError::InvalidRequest("bad".to_string()));

    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_room", route("z-ai/glm-5.3", 64))
        .retry(fast_retry(4))
        .build();

    assert!(matches!(
        svc.call(ctx(LlmFeature::WorldgenRoom), req()).await,
        Err(LlmError::InvalidRequest(_))
    ));
    assert_eq!(provider.call_count(), 1, "non-retryable is not retried");
}

#[tokio::test]
async fn circuit_breaker_opens_after_consecutive_failures() {
    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.set_default(Err(LlmError::Network("down".to_string())));

    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_room", route("z-ai/glm-5.3", 64))
        .retry(fast_retry(0)) // one attempt per call
        .breaker(CircuitBreakerConfig {
            consecutive_failures: 2,
            open_secs: 60,
        })
        .build();

    // Two hard failures open the breaker.
    assert!(matches!(
        svc.call(ctx(LlmFeature::WorldgenRoom), req()).await,
        Err(LlmError::Network(_))
    ));
    assert!(matches!(
        svc.call(ctx(LlmFeature::WorldgenRoom), req()).await,
        Err(LlmError::Network(_))
    ));
    // Third call fails fast without touching the provider.
    assert!(matches!(
        svc.call(ctx(LlmFeature::WorldgenRoom), req()).await,
        Err(LlmError::Overloaded)
    ));
    assert_eq!(
        provider.call_count(),
        2,
        "breaker short-circuited the third call"
    );
}

#[tokio::test]
async fn refusal_maps_to_error_and_records_audit() {
    let provider = Arc::new(MockProvider::new("anthropic"));
    provider.push_error(LlmError::Refused {
        category: Some("safety".to_string()),
    });

    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("anthropic", provider.clone() as Arc<dyn LlmProvider>)
        .route("npc_dialogue_major", route("anthropic/claude-opus-5", 64))
        .build();

    let err = svc
        .call(ctx(LlmFeature::NpcDialogue(NpcTier::Major)), req())
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Refused { .. }));

    let last = svc.audit_log().recent(1).pop().unwrap();
    assert_eq!(last.outcome, mush_llm::audit::Outcome::Refused);
    assert_eq!(last.feature, "npc_dialogue_major");
    assert_eq!(last.provider, "anthropic");
}

#[tokio::test]
async fn per_feature_budget_ceiling_enforced() {
    use std::collections::HashMap;

    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.set_default(Ok(mock::ok_response("ok", mock::usage(100, 100, None))));

    // Only the worldgen_room feature is capped (the example config's shape).
    let mut per_feature = HashMap::new();
    per_feature.insert("worldgen_room".to_string(), BudgetLimit::tokens(300));
    let budgets = BudgetConfig {
        per_feature,
        ..Default::default()
    };
    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_room", route("z-ai/glm-5.3", 100))
        .route("worldgen_brief", route("z-ai/glm-5.3-flash", 100))
        .budgets(budgets)
        .build();

    // First worldgen_room call: 200 tokens settled into the feature scope.
    svc.call(ctx(LlmFeature::WorldgenRoom), req())
        .await
        .unwrap();
    assert_eq!(svc.ledger().feature("worldgen_room").input_tokens, 100);

    // Second: 200 used + estimate > 300 → refused for the FEATURE scope.
    let err = svc
        .call(ctx(LlmFeature::WorldgenRoom), req())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        LlmError::BudgetExceeded {
            scope: mush_llm::provider::BudgetScope::Feature
        }
    ));
    assert_eq!(
        provider.call_count(),
        1,
        "capped feature stops at the ceiling"
    );

    // A DIFFERENT, uncapped feature route is unaffected.
    svc.call(ctx(LlmFeature::WorldgenBrief), req())
        .await
        .unwrap();
    assert_eq!(provider.call_count(), 2, "uncapped feature still runs");
}

#[tokio::test]
async fn streaming_settles_budget_and_emits_done() {
    use futures::StreamExt;

    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.push_ok("streamed text", mock::usage(4, 8, Some(0.00002)));

    let budgets = BudgetConfig {
        global: Some(BudgetLimit::tokens(1_000_000)),
        ..Default::default()
    };
    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_room", route("z-ai/glm-5.3", 64))
        .budgets(budgets)
        .build();

    let mut stream = svc
        .call_streaming(ctx(LlmFeature::WorldgenRoom), req())
        .await
        .unwrap();

    let mut saw_text = false;
    let mut saw_done = false;
    while let Some(ev) = stream.next().await {
        match ev.unwrap() {
            mush_llm::types::StreamEvent::TextDelta(_) => saw_text = true,
            mush_llm::types::StreamEvent::Done(_) => saw_done = true,
            _ => {}
        }
    }
    assert!(saw_text && saw_done);

    let g = svc.ledger().global();
    assert_eq!(g.output_tokens, 8);
    assert!((g.cost_usd - 0.00002).abs() < 1e-12);
}

/// M10c regression: the cost dashboard must attribute spend for features,
/// builders, and NPCs that have NO configured per-scope cap. A "global-only"
/// budget (a reasonable "cap the whole server" setup) once produced an empty
/// "top builders" / "top NPCs" list and omitted every uncapped feature row,
/// because the ledger only recorded a scope counter when that scope was
/// capped. `scope_limits` now records every applicable scope regardless of
/// cap, so the always-recorded audit and the ledger agree.
#[tokio::test]
async fn dashboard_attributes_uncapped_feature_and_spenders() {
    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.set_default(Ok(mock::ok_response(
        "ok",
        mock::usage(100, 50, Some(0.02)),
    )));

    // Global-only cap: NO per_feature, per_actor_default, or per_npc_default.
    let budgets = BudgetConfig {
        global: Some(BudgetLimit::tokens(10_000_000)),
        ..Default::default()
    };
    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_brief", route("z-ai/glm-5.3", 64))
        .route("ambience", route("z-ai/glm-5.3-flash", 64))
        .route("npc_dialogue_major", route("z-ai/glm-5.3", 64))
        .budgets(budgets)
        .build();

    let builder = ObjId(7);
    let npc = ObjId(99);

    // An uncapped feature, attributed to a builder.
    svc.call(ctx(LlmFeature::WorldgenBrief).with_actor(builder), req())
        .await
        .unwrap();
    // An uncapped NPC dialogue turn.
    svc.call(
        CallCtx::new(
            LlmFeature::NpcDialogue(NpcTier::Major),
            Priority::Background,
        )
        .with_npc(npc),
        req(),
    )
    .await
    .unwrap();

    let dash = svc.dashboard();

    // The uncapped-but-spent feature has a budget-vs-cap row (no cap, real spend).
    let brief = dash
        .budgets
        .iter()
        .find(|b| b.scope == "feature:worldgen_brief")
        .expect("uncapped-but-spent feature must have a dashboard row");
    assert_eq!(brief.tokens_cap, None, "the feature is uncapped");
    assert_eq!(brief.input_tokens, 100);

    // The builder appears in the top-spender list despite no per-actor cap.
    assert!(
        dash.top_actors.iter().any(|r| r.obj == builder),
        "an uncapped builder's spend must still be attributed: {:?}",
        dash.top_actors
    );
    // The NPC appears in the top-NPC list despite no per-NPC cap.
    assert!(
        dash.top_npcs.iter().any(|r| r.obj == npc),
        "an uncapped NPC's spend must still be attributed: {:?}",
        dash.top_npcs
    );
}

#[tokio::test]
async fn p2_core_11_concurrent_burst_cannot_bypass_token_or_cost_caps() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::Notify;

    struct BarrierProvider {
        calls: AtomicUsize,
        release: Arc<Notify>,
        entered: Arc<Notify>,
        usage: mush_llm::Usage,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl LlmProvider for BarrierProvider {
        fn id(&self) -> &mush_llm::ProviderId {
            static ID: std::sync::OnceLock<mush_llm::ProviderId> = std::sync::OnceLock::new();
            ID.get_or_init(|| mush_llm::ProviderId::from("openrouter"))
        }
        fn caps(&self) -> mush_llm::ProviderCaps {
            mush_llm::ProviderCaps::default()
        }
        async fn complete(
            &self,
            _req: &mush_llm::CompletionRequest,
            _timeout: Duration,
        ) -> Result<mush_llm::CompletionResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_waiters();
            self.release.notified().await;
            if self.fail {
                return Err(LlmError::Timeout);
            }
            Ok(mock::ok_response("ok", self.usage))
        }
        async fn stream(
            &self,
            req: &mush_llm::CompletionRequest,
            timeout: Duration,
        ) -> Result<mush_llm::LlmStream, LlmError> {
            let resp = self.complete(req, timeout).await?;
            Ok(Box::pin(futures::stream::iter(vec![Ok(
                mush_llm::StreamEvent::Done(resp),
            )])))
        }
    }

    // 1. Token ceiling: 20 concurrent calls with max_tokens = 100 each against
    //    a daily cap of 250 tokens -> at most 2 calls may enter the provider!
    let release = Arc::new(Notify::new());
    let entered = Arc::new(Notify::new());
    let provider = Arc::new(BarrierProvider {
        calls: AtomicUsize::new(0),
        release: Arc::clone(&release),
        entered: Arc::clone(&entered),
        usage: mock::usage(20, 80, Some(0.01)),
        fail: false,
    });
    let svc = Arc::new(
        LlmService::builder()
            .global_enabled(true)
            .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
            .route("worldgen_room", route("z-ai/glm-5.3", 100))
            .concurrency(mush_llm::ConcurrencyConfig {
                max_interactive: 8,
                max_interactive_queued: 32,
                max_background: 3,
                max_background_queued: 12,
            })
            .budgets(BudgetConfig {
                global: Some(BudgetLimit::tokens(250)),
                ..Default::default()
            })
            .build(),
    );

    let mut handles = Vec::new();
    for _ in 0..20 {
        let s = Arc::clone(&svc);
        handles.push(tokio::spawn(async move {
            s.call(ctx(LlmFeature::WorldgenRoom), req()).await
        }));
    }

    // Wait until the admitted calls enter the provider, then release all.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        provider.calls.load(Ordering::SeqCst) <= 2,
        "at most 2 concurrent calls may reserve against a 250-token cap, saw {}",
        provider.calls.load(Ordering::SeqCst)
    );
    release.notify_waiters();

    let mut ok_count = 0usize;
    let mut budget_err_count = 0usize;
    for h in handles {
        match h.await.unwrap() {
            Ok(_) => ok_count += 1,
            Err(LlmError::BudgetExceeded { .. }) => budget_err_count += 1,
            Err(other) => panic!("unexpected error: {other}"),
        }
    }
    assert_eq!(ok_count, 2);
    assert_eq!(budget_err_count, 18);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert_eq!(svc.ledger().global().total_tokens(), 200);

    // 2. Cost ceiling with provider pricing: 10 concurrent calls each costing
    //    worst-case $0.10 (1000 max_tokens at $100/MTok) against a $0.25 cap
    //    admits at most 2 calls!
    let release2 = Arc::new(Notify::new());
    let entered2 = Arc::new(Notify::new());
    let provider2 = Arc::new(BarrierProvider {
        calls: AtomicUsize::new(0),
        release: Arc::clone(&release2),
        entered: Arc::clone(&entered2),
        usage: mock::usage(10, 1000, Some(0.10)),
        fail: false,
    });
    let svc2 = Arc::new(
        LlmService::builder()
            .global_enabled(true)
            .provider("openrouter", provider2.clone() as Arc<dyn LlmProvider>)
            .provider_pricing("openrouter", 0.0, 100.0)
            .route("worldgen_room", route("z-ai/glm-5.3", 1000))
            .budgets(BudgetConfig {
                global: Some(BudgetLimit {
                    tokens: None,
                    cost_usd: Some(0.25),
                }),
                ..Default::default()
            })
            .build(),
    );
    let mut handles2 = Vec::new();
    for _ in 0..10 {
        let s = Arc::clone(&svc2);
        handles2.push(tokio::spawn(async move {
            s.call(ctx(LlmFeature::WorldgenRoom), req()).await
        }));
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(provider2.calls.load(Ordering::SeqCst) <= 2);
    release2.notify_waiters();
    let mut ok2 = 0usize;
    for h in handles2 {
        if h.await.unwrap().is_ok() {
            ok2 += 1;
        }
    }
    assert_eq!(ok2, 2);
    assert_eq!(provider2.calls.load(Ordering::SeqCst), 2);

    // 3. Failed call releases its RAII reservation so a subsequent call passes.
    let provider3 = Arc::new(MockProvider::new("openrouter"));
    provider3.push_error(LlmError::Timeout);
    provider3.push_ok("recovered", mock::usage(10, 20, Some(0.01)));
    let svc3 = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider3.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_room", route("z-ai/glm-5.3", 100))
        .retry(mush_llm::RetryConfig {
            max_retries: 0,
            ..Default::default()
        })
        .budgets(BudgetConfig {
            global: Some(BudgetLimit::tokens(150)),
            ..Default::default()
        })
        .build();
    assert!(
        svc3.call(ctx(LlmFeature::WorldgenRoom), req())
            .await
            .is_err()
    );
    assert!(
        svc3.call(ctx(LlmFeature::WorldgenRoom), req())
            .await
            .is_ok(),
        "reservation must release on error"
    );
}

#[tokio::test]
async fn p2_core_11_queued_calls_recheck_kill_switch_after_semaphore() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::Notify;

    struct HoldingProvider {
        calls: AtomicUsize,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl LlmProvider for HoldingProvider {
        fn id(&self) -> &mush_llm::ProviderId {
            static ID: std::sync::OnceLock<mush_llm::ProviderId> = std::sync::OnceLock::new();
            ID.get_or_init(|| mush_llm::ProviderId::from("openrouter"))
        }
        fn caps(&self) -> mush_llm::ProviderCaps {
            mush_llm::ProviderCaps::default()
        }
        async fn complete(
            &self,
            _req: &mush_llm::CompletionRequest,
            _timeout: Duration,
        ) -> Result<mush_llm::CompletionResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            self.release.notified().await;
            Ok(mock::ok_response("ok", mock::usage(5, 5, Some(0.001))))
        }
        async fn stream(
            &self,
            req: &mush_llm::CompletionRequest,
            timeout: Duration,
        ) -> Result<mush_llm::LlmStream, LlmError> {
            let resp = self.complete(req, timeout).await?;
            Ok(Box::pin(futures::stream::iter(vec![Ok(
                mush_llm::StreamEvent::Done(resp),
            )])))
        }
    }

    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let provider = Arc::new(HoldingProvider {
        calls: AtomicUsize::new(0),
        entered: Arc::clone(&entered),
        release: Arc::clone(&release),
    });
    let svc = Arc::new(
        LlmService::builder()
            .global_enabled(true)
            .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
            .route("worldgen_room", route("z-ai/glm-5.3", 64))
            .concurrency(mush_llm::ConcurrencyConfig {
                max_interactive: 1,
                max_interactive_queued: 8,
                max_background: 1,
                max_background_queued: 2,
            })
            .build(),
    );

    // First call acquires the sole interactive slot and pauses inside provider.
    let s0 = Arc::clone(&svc);
    let first = tokio::spawn(async move { s0.call(ctx(LlmFeature::WorldgenRoom), req()).await });
    entered.notified().await;

    // Queue 3 calls and 1 streaming call behind the semaphore while enabled.
    let mut queued = Vec::new();
    for _ in 0..3 {
        let s = Arc::clone(&svc);
        queued.push(tokio::spawn(async move {
            s.call(ctx(LlmFeature::WorldgenRoom), req())
                .await
                .map(|_| ())
        }));
    }
    let s_stream = Arc::clone(&svc);
    queued.push(tokio::spawn(async move {
        s_stream
            .call_streaming(ctx(LlmFeature::WorldgenRoom), req())
            .await
            .map(|_| ())
    }));
    tokio::time::sleep(Duration::from_millis(40)).await;

    // Trip the global kill switch while all 4 calls are queued behind `sem`.
    svc.kill_switch().set_global(true);
    release.notify_waiters();

    assert!(first.await.unwrap().is_ok());
    for h in queued {
        assert!(
            matches!(h.await.unwrap(), Err(LlmError::Disabled)),
            "queued callers must re-check kill switch after semaphore wakeup"
        );
    }
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "no queued call may dispatch HTTP after kill switch is tripped"
    );
}

#[tokio::test]
async fn p2_core_11_nan_inf_and_negative_cost_usd_rejected_and_never_poison_caps() {
    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.push_ok("nan", mock::usage(10, 10, Some(f64::NAN)));
    provider.push_ok("inf", mock::usage(10, 10, Some(f64::INFINITY)));
    provider.push_ok("neg", mock::usage(10, 10, Some(-5.0)));
    provider.push_ok("real", mock::usage(10, 10, Some(1.0)));
    provider.push_ok("blocked", mock::usage(10, 10, Some(0.1)));

    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_room", route("z-ai/glm-5.3", 64))
        .budgets(BudgetConfig {
            global: Some(BudgetLimit {
                tokens: None,
                cost_usd: Some(0.50),
            }),
            ..Default::default()
        })
        .build();

    // NaN, +Inf, and negative cost_usd are rejected as MalformedOutput while
    // never poisoning the ledger's cost_usd accumulator.
    for _ in 0..3 {
        let err = svc
            .call(ctx(LlmFeature::WorldgenRoom), req())
            .await
            .unwrap_err();
        assert!(
            matches!(err, LlmError::MalformedOutput(_)),
            "invalid cost_usd must fail with MalformedOutput, got {err:?}"
        );
        let g = svc.ledger().global();
        assert!(g.cost_usd.is_finite() && g.cost_usd == 0.0);
    }
    // Tokens from those 3 responses were still metered (60 tokens total).
    assert_eq!(svc.ledger().global().total_tokens(), 60);

    // Real cost settles cleanly and enforces the $0.50 cost cap on the next call.
    svc.call(ctx(LlmFeature::WorldgenRoom), req())
        .await
        .unwrap();
    let err = svc
        .call(ctx(LlmFeature::WorldgenRoom), req())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        LlmError::BudgetExceeded {
            scope: mush_llm::provider::BudgetScope::Global
        }
    ));

    // Config validation rejects negative and NaN cost ceilings and pricing.
    let mut bad_cfg = mush_llm::LlmConfig::default();
    bad_cfg.budgets.global = Some(BudgetLimit {
        tokens: None,
        cost_usd: Some(f64::NAN),
    });
    assert!(LlmService::from_config(&bad_cfg).is_err());
    bad_cfg.budgets.global = Some(BudgetLimit {
        tokens: None,
        cost_usd: Some(-1.0),
    });
    assert!(LlmService::from_config(&bad_cfg).is_err());
}

#[tokio::test]
async fn p2_core_10_retry_after_clamped_to_max_delay_and_call_deadline() {
    use std::time::Instant;

    // 1. Huge Retry-After (24h) is clamped to retry.max_delay_ms (5ms) and retries promptly.
    let provider = Arc::new(MockProvider::new("openrouter"));
    provider.push_error(LlmError::RateLimited {
        retry_after: Some(Duration::from_secs(86_400)),
    });
    provider.push_ok("recovered", mock::usage(2, 2, None));

    let svc = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_room", route("z-ai/glm-5.3", 64))
        .retry(RetryConfig {
            min_delay_ms: 1,
            max_delay_ms: 5,
            factor: 2.0,
            max_retries: 2,
            jitter: false,
        })
        .build();

    let t0 = Instant::now();
    let resp = svc
        .call(ctx(LlmFeature::WorldgenRoom), req())
        .await
        .expect("should clamp Retry-After and succeed on retry");
    assert_eq!(resp.text(), "recovered");
    assert_eq!(provider.call_count(), 2);
    assert!(
        t0.elapsed() < Duration::from_secs(1),
        "Retry-After was not clamped: elapsed {:?}",
        t0.elapsed()
    );

    // 2. When clamped delay exceeds remaining call deadline (timeout_s = 1, max_delay_ms = 30s),
    //    the retry aborts immediately without sleeping past the deadline.
    let provider2 = Arc::new(MockProvider::new("openrouter"));
    provider2.push_error(LlmError::RateLimited {
        retry_after: Some(Duration::from_secs(60)),
    });
    provider2.push_ok("unreachable", mock::usage(1, 1, None));

    let mut short_route = route("z-ai/glm-5.3", 64);
    short_route.timeout_s = 1;
    let svc2 = LlmService::builder()
        .global_enabled(true)
        .provider("openrouter", provider2.clone() as Arc<dyn LlmProvider>)
        .route("worldgen_room", short_route)
        .retry(RetryConfig {
            min_delay_ms: 10,
            max_delay_ms: 30_000,
            factor: 2.0,
            max_retries: 3,
            jitter: false,
        })
        .build();

    let t1 = Instant::now();
    let err = svc2
        .call(ctx(LlmFeature::WorldgenRoom), req())
        .await
        .expect_err("retry delay >= remaining deadline must fail fast");
    assert!(matches!(err, LlmError::RateLimited { .. }), "got {err:?}");
    assert_eq!(provider2.call_count(), 1);
    assert!(
        t1.elapsed() < Duration::from_millis(500),
        "exceeded deadline budget: elapsed {:?}",
        t1.elapsed()
    );
}

#[tokio::test]
async fn p2_core_10_backoff_releases_concurrency_permit() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RateLimitFirstProvider {
        calls: AtomicUsize,
        first_failed: Notify,
        second_finished: Notify,
    }

    #[async_trait::async_trait]
    impl LlmProvider for RateLimitFirstProvider {
        fn id(&self) -> &ProviderId {
            static ID: std::sync::OnceLock<ProviderId> = std::sync::OnceLock::new();
            ID.get_or_init(|| ProviderId::from("openrouter"))
        }
        fn caps(&self) -> ProviderCaps {
            ProviderCaps::default()
        }
        async fn complete(
            &self,
            req: &CompletionRequest,
            _timeout: Duration,
        ) -> Result<CompletionResponse, LlmError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            let is_first = format!("{:?}", req.messages).contains("call-a");
            if is_first && n == 0 {
                self.first_failed.notify_waiters();
                return Err(LlmError::RateLimited {
                    retry_after: Some(Duration::from_millis(250)),
                });
            }
            if !is_first {
                self.second_finished.notify_waiters();
            }
            Ok(mock::ok_response("ok", mock::usage(1, 1, None)))
        }
        async fn stream(
            &self,
            _req: &CompletionRequest,
            _timeout: Duration,
        ) -> Result<LlmStream, LlmError> {
            Ok(stream::empty::<Result<StreamEvent, LlmError>>().boxed())
        }
    }

    let provider = Arc::new(RateLimitFirstProvider {
        calls: AtomicUsize::new(0),
        first_failed: Notify::new(),
        second_finished: Notify::new(),
    });
    let svc = Arc::new(
        LlmService::builder()
            .global_enabled(true)
            .provider("openrouter", provider.clone() as Arc<dyn LlmProvider>)
            .route("worldgen_room", route("z-ai/glm-5.3", 64))
            .concurrency(ConcurrencyConfig {
                max_interactive: 1,
                max_interactive_queued: 4,
                max_background: 1,
                max_background_queued: 4,
            })
            .retry(RetryConfig {
                min_delay_ms: 250,
                max_delay_ms: 250,
                factor: 1.0,
                max_retries: 2,
                jitter: false,
            })
            .build(),
    );

    let failed_notified = provider.first_failed.notified();
    let call_a = tokio::spawn({
        let svc = Arc::clone(&svc);
        async move {
            svc.call(
                ctx(LlmFeature::WorldgenRoom),
                CompletionRequest::simple("call-a"),
            )
            .await
        }
    });
    failed_notified.await;
    // Yield briefly so call_a drops its semaphore permit and enters sleep.
    tokio::time::sleep(Duration::from_millis(20)).await;

    // With max_interactive = 1, call_b can only finish inside <150ms if call_a
    // released the active concurrency permit during its 250ms backoff sleep!
    let t0 = std::time::Instant::now();
    svc.call(
        ctx(LlmFeature::WorldgenRoom),
        CompletionRequest::simple("call-b"),
    )
    .await
    .expect("call-b should acquire released permit during call-a backoff");
    assert!(
        t0.elapsed() < Duration::from_millis(150),
        "call-b was blocked by call-a backoff sleep ({:?})",
        t0.elapsed()
    );

    call_a
        .await
        .expect("task join")
        .expect("call-a succeeds on retry");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn p2_core_10_retry_rechecks_kill_switch_circuit_breaker_and_qps() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    // 1. Kill switch engaged during backoff sleep aborts retry with Disabled.
    struct SlowFailProvider {
        calls: AtomicUsize,
        entered: Notify,
    }
    #[async_trait::async_trait]
    impl LlmProvider for SlowFailProvider {
        fn id(&self) -> &ProviderId {
            static ID: std::sync::OnceLock<ProviderId> = std::sync::OnceLock::new();
            ID.get_or_init(|| ProviderId::from("openrouter"))
        }
        fn caps(&self) -> ProviderCaps {
            ProviderCaps::default()
        }
        async fn complete(
            &self,
            _req: &CompletionRequest,
            _timeout: Duration,
        ) -> Result<CompletionResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_waiters();
            Err(LlmError::RateLimited {
                retry_after: Some(Duration::from_millis(120)),
            })
        }
        async fn stream(
            &self,
            _req: &CompletionRequest,
            _timeout: Duration,
        ) -> Result<LlmStream, LlmError> {
            Ok(stream::empty::<Result<StreamEvent, LlmError>>().boxed())
        }
    }

    let kill_provider = Arc::new(SlowFailProvider {
        calls: AtomicUsize::new(0),
        entered: Notify::new(),
    });
    let svc_kill = Arc::new(
        LlmService::builder()
            .global_enabled(true)
            .provider("openrouter", kill_provider.clone() as Arc<dyn LlmProvider>)
            .route("worldgen_room", route("z-ai/glm-5.3", 64))
            .retry(RetryConfig {
                min_delay_ms: 120,
                max_delay_ms: 120,
                factor: 1.0,
                max_retries: 3,
                jitter: false,
            })
            .build(),
    );
    let entered = kill_provider.entered.notified();
    let h = tokio::spawn({
        let svc = Arc::clone(&svc_kill);
        async move { svc.call(ctx(LlmFeature::WorldgenRoom), req()).await }
    });
    entered.await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    svc_kill.kill_switch().set_global(true);
    let err = h
        .await
        .expect("join")
        .expect_err("must stop on kill switch");
    assert!(matches!(err, LlmError::Disabled), "got {err:?}");
    assert_eq!(kill_provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        svc_kill.audit_log().recent(1)[0].outcome,
        mush_llm::Outcome::Disabled
    );

    // 2. Circuit breaker trips mid-retry and short-circuits remaining attempts.
    let breaker_provider = Arc::new(MockProvider::new("openrouter"));
    breaker_provider.set_default(Err(LlmError::Overloaded));
    let svc_breaker = LlmService::builder()
        .global_enabled(true)
        .provider(
            "openrouter",
            breaker_provider.clone() as Arc<dyn LlmProvider>,
        )
        .route("worldgen_room", route("z-ai/glm-5.3", 64))
        .retry(fast_retry(5))
        .breaker(CircuitBreakerConfig {
            consecutive_failures: 2,
            open_secs: 60,
        })
        .build();
    let err = svc_breaker
        .call(ctx(LlmFeature::WorldgenRoom), req())
        .await
        .expect_err("breaker should open mid-retry");
    assert!(matches!(err, LlmError::Overloaded), "got {err:?}");
    assert_eq!(
        breaker_provider.call_count(),
        2,
        "breaker must stop retries once consecutive_failures is reached"
    );

    // 3. Per-provider QPS limiter is re-checked before every retry attempt
    //    (and bounded by the call deadline so a 1-rpm limiter times out instead
    //    of dispatching an unthrottled retry).
    let qps_provider = Arc::new(MockProvider::new("openrouter"));
    qps_provider.push_error(LlmError::Overloaded);
    qps_provider.push_ok("should be throttled", mock::usage(1, 1, None));
    let mut short_route = route("z-ai/glm-5.3", 64);
    short_route.timeout_s = 1;
    let svc_qps = LlmService::builder()
        .global_enabled(true)
        .provider_with_rpm(
            "openrouter",
            qps_provider.clone() as Arc<dyn LlmProvider>,
            1,
        )
        .route("worldgen_room", short_route)
        .retry(fast_retry(2))
        .build();
    let err = svc_qps
        .call(ctx(LlmFeature::WorldgenRoom), req())
        .await
        .expect_err("retry must obey QPS limiter and deadline");
    assert!(matches!(err, LlmError::Timeout), "got {err:?}");
    assert_eq!(
        qps_provider.call_count(),
        1,
        "retry must not bypass the QPS limiter"
    );
}
